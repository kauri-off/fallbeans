import { describe, expect, it } from 'vitest';
import { Auth, readCookie } from '../src/server/auth';

const secret = Buffer.alloc(32, 7);

describe('auth', () => {
  it('accepts the right PIN and rate limits guessing', async () => {
    let now = 1_000_000;
    const auth = new Auth(
      secret,
      async (p) => p === '5050',
      () => now,
    );
    expect(await auth.login('5050', 'a')).toBe('ok');
    for (let i = 0; i < 4; i++) expect(await auth.login('1111', 'a')).toBe('bad');
    expect(await auth.login('5050', 'a')).toBe('limited');
    expect(await auth.login('5050', 'b')).toBe('ok');
    now += 61_000;
    expect(await auth.login('5050', 'a')).toBe('ok');
    expect(await auth.login('abc', 'c')).toBe('bad');
  });

  it('signs cookies and tickets and expires them', () => {
    let now = 1_700_000_000_000;
    const auth = new Auth(
      secret,
      async () => true,
      () => now,
    );
    const cookie = auth.issueCookie();
    expect(auth.validCookie(cookie)).toBe(true);
    expect(auth.validCookie(`${cookie}x`)).toBe(false);
    expect(
      new Auth(
        Buffer.alloc(32, 8),
        async () => true,
        () => now,
      ).validCookie(cookie),
    ).toBe(false);
    const ticket = auth.issueTicket();
    expect(auth.validTicket(ticket)).toBe(true);
    now += 121_000;
    expect(auth.validTicket(ticket)).toBe(false);
    expect(auth.validCookie(cookie)).toBe(true);
    now += 366 * 24 * 3600 * 1000;
    expect(auth.validCookie(cookie)).toBe(false);
  });

  it('reads cookies', () => {
    expect(readCookie('a=1; fb_auth=x.y.z; b=2', 'fb_auth')).toBe('x.y.z');
    expect(readCookie(null, 'fb_auth')).toBeUndefined();
  });
});
