import { getGame } from '../../games';
import { DEV_ROOM_ID, MAX_PRACTICE_ROOMS, MAX_ROOMS, ROOM_EMPTY_MS } from '../../shared/consts';
import { type Hello, type RoomInfo, type ServerMsg, sanitizeName, sanitizeTitle } from '../../shared/protocol';
import { sameKey } from '../auth';
import type { Logger } from '../log';
import type { Conn } from '../net/conn';
import { makePin, Room, type RoomOptions } from './room';

/** A connected player: at the room list (`room` null) or in a room. */
export interface Member {
  readonly conn: Conn;
  /** Identity (see Auth.issueIdentity): the same person on every connection they make. */
  readonly uid: string;
  name: string;
  room: Room | null;
  /** Player id in `room`. */
  id: number;
  /** The connection no longer stands for the player: closed, or replaced by a newer one. */
  gone: boolean;
}

/** What every room of the server is made with. */
export type SharedRoomOptions = Pick<
  RoomOptions,
  'minPlayers' | 'maxPlayers' | 'seed' | 'introMs' | 'clock' | 'autoTick' | 'dev'
>;

export interface HubOptions {
  room: SharedRoomOptions;
  log: Logger;
  /** Rate limit for guessing the PIN of a private room (by address). */
  allowGuess(ip: string): boolean;
}

/** Room codes avoid look-alike characters (no i, l, o, 0, 1). */
const ID_CHARS = 'abcdefghjkmnpqrstuvwxyz23456789';
const ID_LENGTH = 5;

/**
 * The rooms of the server and who is where. Anyone may open one room of their own (they are its
 * host whenever they are in it) and be in one room at a time; a private room asks newcomers for
 * its PIN. Rooms close when everybody has left. Practice rooms are separate: one player, not listed.
 */
export class Hub {
  readonly rooms = new Map<string, Room>();
  readonly practice = new Set<Room>();
  /** The live connection of each player (at the room list or in a room; practice is not counted). */
  private readonly members = new Map<string, Member>();
  /** Rooms nobody is in, and since when (they close after ROOM_EMPTY_MS). */
  private readonly emptySince = new Map<Room, number>();
  /** The room list as last sent, to skip changes that do not show in it. */
  private sent = '';
  private readonly sweeper: ReturnType<typeof setInterval> | null;
  private readonly log: Logger;
  private readonly clock: () => number;

  constructor(private readonly opts: HubOptions) {
    this.log = opts.log;
    this.clock = opts.room.clock ?? (() => performance.now());
    // Dev server: a room that is always there, for the tools (probe, benchmarks, e2e) to meet in.
    if (opts.room.dev) this.open(DEV_ROOM_ID, { title: 'Dev', permanent: true });
    this.sweeper = opts.room.autoTick === false ? null : setInterval(() => this.sweep(), 5000);
    this.sweeper?.unref?.();
  }

  /** Every room being simulated: listed ones first, then practice. */
  get all(): Room[] {
    return [...this.rooms.values(), ...this.practice];
  }

  /** Server clock for connections that are in no room. */
  now() {
    return this.clock();
  }

  // ------------------------------------------------------------------ connections

  /**
   * A connection said a valid hello. It takes the place of any earlier connection of the same
   * player, then goes where the hello says: a practice round, a room (asked for, or the one the
   * player is still in), or the room list. Null when the connection was refused.
   */
  enter(conn: Conn, uid: string, h: Hello): Member | null {
    const m: Member = { conn, uid, name: sanitizeName(h.name), room: null, id: -1, gone: false };
    if (h.practice) return this.enterPractice(m, h.practice) ? m : null;
    const old = this.members.get(uid);
    const target = h.room ?? old?.room?.id ?? this.roomOf(uid)?.id;
    if (old) this.evict(old, target);
    this.members.set(uid, m);
    if (!target || !this.join(m, target, h.pin)) this.sendDirectory(m);
    return m;
  }

  /** The connection closed. */
  drop(m: Member) {
    if (m.gone) return;
    m.gone = true;
    if (this.members.get(m.uid) === m) this.members.delete(m.uid);
    const room = m.room;
    m.room = null;
    if (!room) return;
    room.leave(m.id, m.conn);
    // A practice room lives only while its player is connected.
    if (this.practice.delete(room)) room.dispose();
    else this.vacated(room);
  }

  rename(m: Member, name: string) {
    m.name = sanitizeName(name) || m.name;
  }

  // ------------------------------------------------------------------ rooms

  /** Opens the player's own room and enters it; if they already have one, they return to it. */
  create(m: Member, title: string, isPrivate: boolean): boolean {
    if (m.room || m.gone) return false;
    const mine = this.owned(m.uid);
    if (mine) return this.join(m, mine.id);
    if (this.rooms.size >= MAX_ROOMS)
      return this.deny(m, null, 'limit', 'Сейчас открыто слишком много комнат — зайдите в одну из них или попробуйте позже');
    const id = this.newId();
    this.open(id, {
      title: sanitizeTitle(title) || (m.name ? `Комната ${m.name}` : `Комната ${id.toUpperCase()}`),
      owner: m.uid,
      pin: isPrivate ? makePin() : null,
    });
    this.log.info('room opened', { room: id, private: isPrivate, rooms: this.rooms.size });
    return this.join(m, id);
  }

  /** Enters a room from the room list. On failure the player is told why and stays where they are. */
  join(m: Member, id: string, pin?: string): boolean {
    if (m.room || m.gone) return false;
    const room = this.rooms.get(id);
    if (!room) return this.deny(m, id, 'gone', 'Этой комнаты больше нет');
    // The owner and those the room let in before come back without the PIN.
    const known = room.owner === m.uid || room.admitted.has(m.uid) || !!room.playerOf(m.uid);
    if (room.pin && !known) {
      if (!pin) return this.deny(m, id, 'pin', '');
      if (!this.opts.allowGuess(m.conn.ip)) return this.deny(m, id, 'limit', 'Слишком много попыток — подождите минуту');
      if (!sameKey(pin, room.pin)) {
        this.log.warn('wrong room pin', { room: id, ip: m.conn.ip });
        return this.deny(m, id, 'pin', 'Неверный PIN-код');
      }
    }
    // (Set first: the room's own messages must not be followed by a room list for this player.)
    m.room = room;
    const pid = room.join(m.conn, { uid: m.uid, name: m.name });
    if (pid === null) {
      m.room = null;
      return this.deny(m, id, 'full', 'В комнате нет свободных мест');
    }
    m.id = pid;
    this.emptySince.delete(room);
    // One room at a time: a room still keeping this player's place (a game they dropped out of) lets go.
    for (const other of [...this.rooms.values()]) {
      const p = other === room ? undefined : other.playerOf(m.uid);
      if (!p) continue;
      other.quit(p.id);
      this.vacated(other, true);
    }
    this.changed();
    return true;
  }

  /** Back to the room list. */
  leave(m: Member) {
    const room = m.room;
    if (!room || m.gone || this.practice.has(room)) return;
    room.quit(m.id);
    m.room = null;
    this.send(m.conn, { t: 'home', msg: '' });
    this.vacated(room, true);
    this.sendDirectory(m);
  }

  /** Closes rooms that have stood empty for a while. */
  sweep(now = this.clock()) {
    for (const room of [...this.rooms.values()]) {
      if (room.permanent) continue;
      if (!room.empty) {
        this.emptySince.delete(room);
        continue;
      }
      const since = this.emptySince.get(room);
      if (since === undefined) this.emptySince.set(room, now);
      else if (now - since >= ROOM_EMPTY_MS) this.close(room);
    }
  }

  dispose() {
    if (this.sweeper) clearInterval(this.sweeper);
    for (const r of this.all) r.dispose();
  }

  // ------------------------------------------------------------------ internals

  private open(id: string, o: Pick<RoomOptions, 'title' | 'owner' | 'pin' | 'permanent'>): Room {
    const { log } = this;
    const room = new Room({
      ...this.opts.room,
      ...o,
      id,
      log: { info: (msg, d) => log.info(msg, { room: id, ...d }), warn: (msg, d) => log.warn(msg, { room: id, ...d }) },
      onChange: () => this.changed(),
    });
    this.rooms.set(id, room);
    return room;
  }

  private close(room: Room) {
    this.rooms.delete(room.id);
    this.emptySince.delete(room);
    room.dispose();
    this.log.info('room closed', { room: room.id, rooms: this.rooms.size });
    this.changed();
  }

  /** Someone left `room`: if it is empty now it closes, at once or (a lost connection) after a while. */
  private vacated(room: Room, now = false) {
    if (room.permanent || !room.empty || !this.rooms.has(room.id)) return;
    if (now) this.close(room);
    else if (!this.emptySince.has(room)) this.emptySince.set(room, this.clock());
  }

  private enterPractice(m: Member, gameId: string): boolean {
    const game = getGame(gameId);
    const refuse = (reason: 'bad' | 'busy', msg: string) => {
      this.send(m.conn, { t: 'reject', reason, msg });
      m.conn.close(reason);
      return false;
    };
    if (!game) return refuse('bad', 'Нет такой карты');
    if (this.practice.size >= MAX_PRACTICE_ROOMS) return refuse('busy', 'Сейчас слишком много тренировок — попробуйте позже');
    const room = new Room({ ...this.opts.room, minPlayers: 1, practice: { game: game.id, bots: 3 }, log: this.log });
    this.practice.add(room);
    m.room = room;
    m.id = room.join(m.conn, { uid: m.uid, name: m.name }) ?? -1;
    return true;
  }

  /** A newer connection of the same player takes over: this one is told and closed. */
  private evict(old: Member, stayIn: string | undefined) {
    const room = old.room;
    old.gone = true;
    old.room = null;
    this.send(old.conn, { t: 'reject', reason: 'moved', msg: 'Игра открыта в другой вкладке или окне' });
    // Going elsewhere: out of the room. Staying: Room.join moves the player to the new connection.
    if (room && room.id !== stayIn) {
      room.quit(old.id);
      this.vacated(room, true);
    }
    old.conn.close('moved');
  }

  /** The room keeping a place for this player (they are in it, or dropped out of its game). */
  private roomOf(uid: string): Room | undefined {
    for (const r of this.rooms.values()) if (r.playerOf(uid)) return r;
    return undefined;
  }

  private owned(uid: string): Room | undefined {
    for (const r of this.rooms.values()) if (r.owner === uid) return r;
    return undefined;
  }

  private newId(): string {
    for (;;) {
      const bytes = globalThis.crypto.getRandomValues(new Uint8Array(ID_LENGTH));
      const id = [...bytes].map((b) => ID_CHARS[b % ID_CHARS.length]).join('');
      if (!this.rooms.has(id) && id !== DEV_ROOM_ID) return id;
    }
  }

  /** The room list: rooms someone is in (an empty one is about to close). */
  private directory(): RoomInfo[] {
    return [...this.rooms.values()].filter((r) => r.permanent || !r.empty).map((r) => r.info());
  }

  private sendDirectory(m: Member, rooms = this.directory()) {
    this.send(m.conn, { t: 'rooms', rooms, mine: this.owned(m.uid)?.id ?? null });
  }

  /** Tells everyone at the room list when it changed. */
  private changed() {
    const rooms = this.directory();
    const key = JSON.stringify([rooms, [...this.rooms.keys()]]);
    if (key === this.sent) return;
    this.sent = key;
    for (const m of this.members.values()) if (!m.room && !m.gone) this.sendDirectory(m, rooms);
  }

  private deny(m: Member, room: string | null, reason: 'pin' | 'full' | 'gone' | 'limit', msg: string): false {
    this.send(m.conn, { t: 'denied', room, reason, msg });
    return false;
  }

  private send(conn: Conn, msg: ServerMsg) {
    try {
      conn.send(msg);
    } catch (e) {
      this.log.warn('send failed', { ip: conn.ip, err: String(e) });
    }
  }
}
