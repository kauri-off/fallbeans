import { getGame } from '../games';
import { MAX_PRACTICE_ROOMS, PROTOCOL_VERSION } from '../shared/consts';
import { type ClientMsg, ClientMsgSchema, MAX_CONTROL_BYTES } from '../shared/protocol';
import type { Auth } from './auth';
import { type Conn, type Logger, Room, type RoomOptions } from './room';

const HELLO_TIMEOUT_MS = 5000;

export interface ConnSession {
  control(raw: string): void;
  datagram(data: Uint8Array): void;
  close(): void;
}

/**
 * Entry point for every connection: waits for an authenticated hello, then routes the connection
 * to the main room or to a private practice room.
 */
export class Gateway {
  readonly main: Room;
  readonly practice = new Set<Room>();

  constructor(
    private readonly auth: Auth,
    private readonly log: Logger,
    private readonly roomOpts: Omit<RoomOptions, 'practice' | 'log'>,
  ) {
    this.main = new Room({ ...roomOpts, log });
  }

  get rooms(): Room[] {
    return [this.main, ...this.practice];
  }

  open(conn: Conn): ConnSession {
    let room: Room | null = null;
    let id: number | null = null;
    let closed = false;
    const timeout = setTimeout(() => {
      if (id === null && !closed) {
        this.log.warn('no hello', { ip: conn.ip });
        conn.close('timeout');
      }
    }, HELLO_TIMEOUT_MS);

    const parse = (raw: string): ClientMsg | null => {
      if (raw.length > MAX_CONTROL_BYTES) return null;
      let data: unknown;
      try {
        data = JSON.parse(raw);
      } catch {
        return null;
      }
      const r = ClientMsgSchema.safeParse(data);
      if (!r.success) {
        this.log.warn('bad message', { ip: conn.ip, id, issue: r.error.issues[0]?.message });
        return null;
      }
      return r.data;
    };

    return {
      control: (raw) => {
        const m = parse(raw);
        if (!m) return;
        if (room && id !== null) {
          room.control(id, conn, m);
          return;
        }
        if (m.t === 'ping') {
          conn.send({ t: 'pong', c: m.c, s: this.main.now() });
          return;
        }
        if (m.t !== 'hello') return;
        if (m.v !== PROTOCOL_VERSION) {
          conn.send({ t: 'reject', reason: 'version', msg: 'Версия игры устарела — обновите страницу' });
          conn.close('version');
          return;
        }
        if (!this.auth.validTicket(m.ticket)) {
          conn.send({ t: 'reject', reason: 'auth', msg: 'Нужен PIN-код — обновите страницу' });
          conn.close('auth');
          return;
        }
        let target = this.main;
        if (m.practice) {
          const game = getGame(m.practice);
          if (!game) {
            conn.send({ t: 'reject', reason: 'bad', msg: 'Нет такой карты' });
            conn.close('bad');
            return;
          }
          if (this.practice.size >= MAX_PRACTICE_ROOMS) {
            conn.send({ t: 'reject', reason: 'busy', msg: 'Сейчас слишком много тренировок — попробуйте позже' });
            conn.close('busy');
            return;
          }
          target = new Room({ ...this.roomOpts, minPlayers: 1, practice: { game: game.id, bots: 3 }, log: this.log });
          this.practice.add(target);
        }
        const joined = target.join(conn, m);
        if (joined === null) {
          this.cleanup(target);
          return;
        }
        room = target;
        id = joined;
        clearTimeout(timeout);
      },
      datagram: (data) => {
        if (room && id !== null) room.datagram(id, conn, data);
      },
      close: () => {
        closed = true;
        clearTimeout(timeout);
        if (room && id !== null) room.leave(id, conn);
        if (room) this.cleanup(room);
      },
    };
  }

  private cleanup(room: Room) {
    if (room === this.main || !this.practice.has(room)) return;
    // A practice room lives only while its player is connected.
    room.dispose();
    this.practice.delete(room);
  }

  dispose() {
    for (const r of this.rooms) r.dispose();
  }
}
