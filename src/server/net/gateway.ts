import { PROTOCOL_VERSION } from '../../shared/consts';
import { type ClientMsg, ClientMsgSchema, type Hello, MAX_CONTROL_BYTES } from '../../shared/protocol';
import type { Auth } from '../auth';
import type { Logger } from '../log';
import { Hub, type Member, type SharedRoomOptions } from '../rooms/hub';
import type { Room } from '../rooms/room';
import type { Conn } from './conn';

const HELLO_TIMEOUT_MS = 5000;
/** Control messages per second from a connection at the room list (rooms have their own limit). */
const LIST_RATE = 20;

export interface ConnSession {
  control(raw: string): void;
  datagram(data: Uint8Array): void;
  close(): void;
}

/**
 * Entry point for every connection: checks the hello (version, ticket, identity) and then passes
 * messages on to the hub (room list: create, join, leave) or to the room the player is in.
 */
export class Gateway {
  readonly hub: Hub;
  /** Every open connection (at the room list, in a room, or before its hello). */
  private readonly conns = new Set<Conn>();
  private _updating = false;

  constructor(
    private readonly auth: Auth,
    private readonly log: Logger,
    private readonly roomOpts: SharedRoomOptions,
  ) {
    this.hub = new Hub({ room: roomOpts, log, allowGuess: (ip) => auth.allowAttempt(ip) });
  }

  get rooms(): Room[] {
    return this.hub.all;
  }

  /** The game is being updated (see Config.maintenanceFile). */
  get updating() {
    return this._updating;
  }

  /** Into or out of the update: going in, every connection is told and closed. */
  setUpdating(on: boolean) {
    if (on === this._updating) return;
    this._updating = on;
    this.log.info(on ? 'updating: disconnecting everybody' : 'update over: open again', { connections: this.conns.size });
    if (on) for (const c of [...this.conns]) this.sendOff(c);
  }

  private sendOff(conn: Conn) {
    try {
      conn.send({ t: 'updating' });
    } catch {}
    conn.close('updating');
  }

  open(conn: Conn): ConnSession {
    if (this._updating) {
      this.sendOff(conn);
      return { control() {}, datagram() {}, close() {} };
    }
    this.conns.add(conn);
    let member: Member | null = null;
    let closed = false;
    let windowAt = 0;
    let count = 0;
    const timeout = setTimeout(() => {
      if (!member && !closed) {
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
        this.log.warn('bad message', { ip: conn.ip, id: member?.id ?? null, issue: r.error.issues[0]?.message });
        return null;
      }
      return r.data;
    };
    const allow = () => {
      const now = this.hub.now();
      if (now - windowAt > 1000) {
        windowAt = now;
        count = 0;
      }
      return ++count <= LIST_RATE;
    };

    return {
      control: (raw) => {
        const m = parse(raw);
        if (!m || closed) return;
        if (!member) {
          if (m.t === 'ping') conn.send({ t: 'pong', c: m.c, s: this.hub.now() });
          else if (m.t === 'hello') {
            member = this.hello(conn, m);
            if (member) clearTimeout(timeout);
          }
          return;
        }
        if (member.gone) return;
        // (The name, colour and outfit travel with the player from room to room.)
        if (m.t === 'name') this.hub.rename(member, m.name);
        if (m.t === 'color') member.color = m.c;
        if (m.t === 'outfit') member.outfit = m.o;
        if (member.room) {
          if (m.t === 'leave') this.hub.leave(member);
          else member.room.control(member.id, conn, m);
          return;
        }
        // At the room list.
        if (!allow()) return;
        if (m.t === 'ping') conn.send({ t: 'pong', c: m.c, s: this.hub.now() });
        else if (m.t === 'create') this.hub.create(member, m.title, m.private);
        else if (m.t === 'join') this.hub.join(member, m.room, m.pin);
      },
      datagram: (data) => {
        if (member?.room) member.room.datagram(member.id, conn, data);
      },
      close: () => {
        closed = true;
        this.conns.delete(conn);
        clearTimeout(timeout);
        if (member) this.hub.drop(member);
      },
    };
  }

  /** Checks a hello; an accepted connection learns its identity token and enters the hub. */
  private hello(conn: Conn, m: Hello): Member | null {
    if (m.v !== PROTOCOL_VERSION) {
      conn.send({ t: 'reject', reason: 'version', msg: 'Версия игры устарела — обновите страницу' });
      conn.close('version');
      return null;
    }
    if (!this.auth.validTicket(m.ticket)) {
      conn.send({ t: 'reject', reason: 'auth', msg: 'Сессия устарела — обновите страницу' });
      conn.close('auth');
      return null;
    }
    // A player is whoever holds the identity token; a browser without one gets a new identity.
    const known = this.auth.identity(m.token);
    const who = known && m.token ? { uid: known, token: m.token } : this.auth.issueIdentity();
    conn.send({ t: 'ready', token: who.token, dev: !!this.roomOpts.dev });
    return this.hub.enter(conn, who.uid, m);
  }

  dispose() {
    this.hub.dispose();
  }
}
