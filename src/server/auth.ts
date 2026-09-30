import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto';

export const COOKIE = 'fb_auth';
const COOKIE_TTL_S = 365 * 24 * 3600;
const TICKET_TTL_MS = 120_000;
export const DEBUG_COOKIE = 'fb_debug';
const DEBUG_TTL_S = 7 * 24 * 3600;

/**
 * PIN access. A correct PIN earns a signed cookie (one year, this browser); the page trades the
 * cookie for short-lived tickets to open game connections (WebTransport sends no cookies).
 * Changing FB_SECRET signs everyone out.
 */
export class Auth {
  private readonly perIp = new Map<string, number[]>();
  private global: number[] = [];

  constructor(
    private readonly secret: Buffer,
    private readonly checkPin: (pin: string) => Promise<boolean>,
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

  /** Rate limit: 5 attempts per minute per address, 30 per minute in total. */
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

  async login(pin: string, ip: string): Promise<'ok' | 'bad' | 'limited'> {
    if (!this.allowAttempt(ip)) return 'limited';
    if (!/^\d{4,12}$/.test(pin)) return 'bad';
    return (await this.checkPin(pin)) ? 'ok' : 'bad';
  }

  issueCookie(): string {
    const payload = `c1.${Math.floor(this.wallClock() / 1000)}`;
    return `${payload}.${this.sign(payload)}`;
  }

  validCookie(value: string | undefined): boolean {
    if (!value) return false;
    const parts = value.split('.');
    if (parts.length !== 3 || parts[0] !== 'c1') return false;
    const issued = Number(parts[1]);
    if (!Number.isInteger(issued)) return false;
    const age = this.wallClock() / 1000 - issued;
    if (age < -60 || age > COOKIE_TTL_S) return false;
    return this.verify(`${parts[0]}.${parts[1]}`, parts[2]!);
  }

  /** Debug page access (production): a week, on top of the PIN cookie. */
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

export function cookieHeader(value: string, secure: boolean, maxAge = COOKIE_TTL_S): string {
  return `${COOKIE}=${value}; Path=/fallbeans; Max-Age=${maxAge}; HttpOnly; SameSite=Strict${secure ? '; Secure' : ''}`;
}

export function debugCookieHeader(value: string, secure: boolean): string {
  return `${DEBUG_COOKIE}=${value}; Path=/fallbeans; Max-Age=${DEBUG_TTL_S}; HttpOnly; SameSite=Strict${secure ? '; Secure' : ''}`;
}

/** Constant-time comparison of a debug key with the configured one. */
export function sameKey(given: string, want: string): boolean {
  const a = createHmac('sha256', 'fb-debug').update(given).digest();
  const b = createHmac('sha256', 'fb-debug').update(want).digest();
  return timingSafeEqual(a, b);
}

/** PIN hash for FB_PIN_HASH_B64 (argon2id). */
export function hashPin(pin: string): Promise<string> {
  return Bun.password.hash(pin, { algorithm: 'argon2id', memoryCost: 19456, timeCost: 2 });
}
