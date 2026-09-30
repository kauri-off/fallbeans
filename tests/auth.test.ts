import { describe, expect, it } from 'vitest';
import { Auth, readCookie } from '../src/server/auth';

const secret = Buffer.alloc(32, 7);

describe('auth', () => {
  it('rate limits guessing', () => {
    let now = 1_000_000;
    const auth = new Auth(secret, () => now);
    for (let i = 0; i < 5; i++) expect(auth.allowAttempt('a')).toBe(true);
    expect(auth.allowAttempt('a')).toBe(false);
    expect(auth.allowAttempt('b')).toBe(true);
    now += 61_000;
    expect(auth.allowAttempt('a')).toBe(true);
  });

  it('signs identities, tickets and debug cookies, and expires them', () => {
    let now = 1_700_000_000_000;
    const auth = new Auth(secret, () => now);
    const other = new Auth(Buffer.alloc(32, 8), () => now);
    const who = auth.issueIdentity();
    expect(auth.identity(who.token)).toBe(who.uid);
    expect(auth.identity(`${who.token}x`)).toBeNull();
    expect(auth.identity(undefined)).toBeNull();
    expect(other.identity(who.token)).toBeNull();
    const cookie = auth.issueDebugCookie();
    expect(auth.validDebugCookie(cookie)).toBe(true);
    expect(auth.validDebugCookie(`${cookie}x`)).toBe(false);
    expect(other.validDebugCookie(cookie)).toBe(false);
    const ticket = auth.issueTicket();
    expect(auth.validTicket(ticket)).toBe(true);
    now += 121_000;
    expect(auth.validTicket(ticket)).toBe(false);
    expect(auth.validDebugCookie(cookie)).toBe(true);
    now += 8 * 24 * 3600 * 1000;
    expect(auth.validDebugCookie(cookie)).toBe(false);
    expect(auth.identity(who.token)).toBe(who.uid);
  });

  it('reads cookies', () => {
    expect(readCookie('a=1; fb_debug=x.y.z; b=2', 'fb_debug')).toBe('x.y.z');
    expect(readCookie(null, 'fb_debug')).toBeUndefined();
  });
});
