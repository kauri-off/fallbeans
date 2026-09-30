import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto';

const TICKET_TTL_MS = 120_000;
export const DEBUG_COOKIE = 'fb_debug';
const DEBUG_TTL_S = 7 * 24 * 3600;

/**
 * What the server signs with FB_SECRET. The game itself is open to everyone:
 *   tickets     short-lived, fetched over HTTP to open a game connection (WebTransport sends no cookies);
 *   identities  a random player id kept by the browser, so a player is one person across tabs and reloads
 *               (one room at a time, their own room stays theirs);
 *   debug cookie  access to the production debug page, earned with FB_DEBUG_KEY.
 * It also rate limits guessing (PINs of private rooms, the debug key). Changing FB_SECRET resets all three.
 */
export class Auth {
  private readonly perIp = new Map<string, number[]>();
  private global: number[] = [];

  constructor(
    private readonly secret: Buffer,
    private readonly wallClock: () => number = Date.now,
  ) {}

  private sign(s: string) {
    return createHmac('sha256', this.secret).update(s).digest('base64url');
  }

  private verify(payload: string, sig: string) {
    const want = Buffer.from(this.sign(payload));
    const got = Buffer.from(sig);
    return want.length === got.length && timingSafeEqual(want, got);
  }

  /** Rate limit for guesses: 5 attempts per minute per address, 30 per minute in total. */
  allowAttempt(ip: string): boolean {
    const now = this.wallClock();
    const cut = now - 60_000;
    const mine = (this.perIp.get(ip) ?? []).filter((t) => t > cut);
    this.global = this.global.filter((t) => t > cut);
    if (mine.length >= 5 || this.global.length >= 30) return false;
    mine.push(now);
    this.global.push(now);
    this.perIp.set(ip, mine);
    if (this.perIp.size > 10_000) this.perIp.clear();
    return true;
  }

  /** A new player: the id the server knows them by, and the token their browser keeps. */
  issueIdentity(): { uid: string; token: string } {
    const uid = randomBytes(12).toString('base64url');
    const payload = `u1.${uid}`;
    return { uid, token: `${payload}.${this.sign(payload)}` };
  }

  /** The player id of an identity token, or null when it is not one of ours. */
  identity(token: string | undefined): string | null {
    if (!token) return null;
    const parts = token.split('.');
    if (parts.length !== 3 || parts[0] !== 'u1' || !parts[1]) return null;
    return this.verify(`${parts[0]}.${parts[1]}`, parts[2]!) ? parts[1] : null;
  }

  /** Debug page access (production): a week. */
  issueDebugCookie(): string {
    const payload = `d1.${Math.floor(this.wallClock() / 1000)}`;
    return `${payload}.${this.sign(payload)}`;
  }

  validDebugCookie(value: string | undefined): boolean {
    if (!value) return false;
    const parts = value.split('.');
    if (parts.length !== 3 || parts[0] !== 'd1') return false;
    const age = this.wallClock() / 1000 - Number(parts[1]);
    if (!Number.isFinite(age) || age < -60 || age > DEBUG_TTL_S) return false;
    return this.verify(`${parts[0]}.${parts[1]}`, parts[2]!);
  }

  issueTicket(): string {
    const payload = `t1.${this.wallClock()}.${randomBytes(9).toString('base64url')}`;
    return `${payload}.${this.sign(payload)}`;
  }

  validTicket(value: string): boolean {
    const parts = value.split('.');
    if (parts.length !== 4 || parts[0] !== 't1') return false;
    const issued = Number(parts[1]);
    const age = this.wallClock() - issued;
    if (!Number.isFinite(issued) || age < -5000 || age > TICKET_TTL_MS) return false;
    return this.verify(parts.slice(0, 3).join('.'), parts[3]!);
  }
}

export function readCookie(header: string | null, name: string): string | undefined {
  if (!header) return undefined;
  for (const part of header.split(';')) {
    const i = part.indexOf('=');
    if (i < 0) continue;
    if (part.slice(0, i).trim() === name) return part.slice(i + 1).trim();
  }
  return undefined;
}

export function debugCookieHeader(value: string, secure: boolean): string {
  return `${DEBUG_COOKIE}=${value}; Path=/fallbeans; Max-Age=${DEBUG_TTL_S}; HttpOnly; SameSite=Strict${secure ? '; Secure' : ''}`;
}

/** Constant-time comparison of a secret (the debug key, a room's PIN) with the expected one. */
export function sameKey(given: string, want: string): boolean {
  const a = createHmac('sha256', 'fb-debug').update(given).digest();
  const b = createHmac('sha256', 'fb-debug').update(want).digest();
  return timingSafeEqual(a, b);
}
