//! What the server signs with its secret: player ids, netcode connect tokens and the debug cookie.
//! It also rate limits guessing (room PINs, debug key). A new secret resets all of them.
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::rooms::Uid;

type HmacSha256 = Hmac<Sha256>;

pub const DEBUG_COOKIE: &str = "fb_debug";
pub const DEBUG_TTL_S: u64 = 7 * 24 * 3600;

pub struct Auth {
    secret: Vec<u8>,
}

/// The server's secret: FB_SECRET (64 hex characters), else a new one per run (players get new identities
/// when the server restarts).
pub fn secret() -> Vec<u8> {
    if let Ok(hex) = std::env::var("FB_SECRET") {
        let bytes: Option<Vec<u8>> = (0..hex.len())
            .step_by(2)
            .map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()))
            .collect();
        match bytes {
            Some(b) if b.len() == 32 => return b,
            _ => bevy::log::warn!("FB_SECRET is not 64 hex characters: using a random secret"),
        }
    }
    random_bytes::<32>().to_vec()
}

#[expect(clippy::expect_used, reason = "no system randomness: no secrets, no server")]
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("system randomness");
    b
}

impl Auth {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            secret: secret.to_vec(),
        }
    }

    #[expect(clippy::expect_used, reason = "HMAC takes any key length")]
    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.secret).expect("any key length")
    }

    fn sign(&self, payload: &str) -> String {
        let mut m = self.mac();
        m.update(payload.as_bytes());
        B64.encode(m.finalize().into_bytes())
    }

    fn verify(&self, payload: &str, sig: &str) -> bool {
        let Ok(sig) = B64.decode(sig) else { return false };
        let mut m = self.mac();
        m.update(payload.as_bytes());
        m.verify_slice(&sig).is_ok()
    }

    /// The key netcode encrypts connect tokens with.
    pub fn netcode_key(&self) -> [u8; 32] {
        let mut m = self.mac();
        m.update(b"netcode key");
        m.finalize().into_bytes().into()
    }

    /// A new player: the id the server knows them by, and the token their client keeps.
    pub fn issue_identity(&self) -> (Uid, String) {
        let uid = B64.encode(random_bytes::<12>());
        let payload = format!("u1.{uid}");
        let token = format!("{payload}.{}", self.sign(&payload));
        (uid.into(), token)
    }

    /// The player id of an identity token, or None when it is not one of ours.
    pub fn identity(&self, token: &str) -> Option<Uid> {
        let mut parts = token.split('.');
        let (Some("u1"), Some(uid), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return None;
        };
        (!uid.is_empty() && self.verify(&format!("u1.{uid}"), sig)).then(|| uid.into())
    }

    /// A short tag of the debug key, signed into its cookies: a new key and the old cookies stop working.
    fn key_tag(&self, key: &str) -> String {
        let mut m = self.mac();
        m.update(b"debug key:");
        m.update(key.as_bytes());
        let tag = m.finalize().into_bytes();
        B64.encode(tag.get(..9).unwrap_or_default())
    }

    /// Debug API access (production) with debug key `key`: a week. `now` in seconds since the epoch.
    pub fn issue_debug_cookie(&self, now: u64, key: &str) -> String {
        let payload = format!("d2.{now}.{}", self.key_tag(key));
        format!("{payload}.{}", self.sign(&payload))
    }

    /// Whether `value` is a cookie of ours for the debug key `key`, and not too old.
    pub fn valid_debug_cookie(&self, value: &str, now: u64, key: &str) -> bool {
        let mut parts = value.split('.');
        let (Some("d2"), Some(at), Some(tag), Some(sig), None) =
            (parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return false;
        };
        let Ok(at) = at.parse::<u64>() else { return false };
        if at > now + 60 || now.saturating_sub(at) > DEBUG_TTL_S || tag != self.key_tag(key) {
            return false;
        }
        self.verify(&format!("d2.{at}.{tag}"), sig)
    }
}

/// Wrong guesses a minute from one guesser (`guesser`) before it is locked out.
const GUESSES_PER_IP: usize = 5;
/// Wrong guesses a minute at one target after which only guessers without a recent miss may try.
pub const GUESSES_PER_TARGET: usize = 30;
/// The first lockout of a guesser; each one in a row doubles it, up to LOCKOUT_MAX_MS.
const LOCKOUT_MS: u64 = 60_000;
const LOCKOUT_MAX_MS: u64 = 3_600_000;

/// Rate limit for wrong guesses (room PINs, debug key): 5 a minute per guesser, then doubling lockouts up to an hour.
/// A target takes 30 misses a minute, so spread-out guessers cannot lock out the right PIN.
#[derive(Default)]
pub struct Limiter {
    /// None: guessers whose address is unknown, together.
    per_ip: BTreeMap<Option<AddrKey>, Guesser>,
    per_target: BTreeMap<String, Vec<u64>>,
}

#[derive(Default)]
struct Guesser {
    /// Misses within the last minute (since the last lockout).
    misses: Vec<u64>,
    locked_until: u64,
    /// Lockouts in a row, and when the last miss was.
    strikes: u32,
    last_miss: u64,
}

/// Who a guess or a limit per address is counted against: the IPv4 address, or for IPv6 its /64 (one
/// subscriber's network has billions of addresses).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AddrKey {
    V4(Ipv4Addr),
    V6Net([u16; 4]),
}

impl AddrKey {
    pub fn of(ip: IpAddr) -> Self {
        match ip.to_canonical() {
            IpAddr::V4(v4) => Self::V4(v4),
            IpAddr::V6(v6) => {
                let s = v6.segments();
                Self::V6Net([s[0], s[1], s[2], s[3]])
            }
        }
    }
}

/// Who a limit per address counts against (`AddrKey`); None for this machine, which no such limit applies to
/// (dev servers, stress runs: every client is 127.0.0.1).
pub fn address_key(ip: IpAddr) -> Option<AddrKey> {
    let ip = ip.to_canonical();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| AddrKey::of(ip))
}

/// So many requests a minute per address (`address_key`).
pub struct Budget {
    per_minute: usize,
    per_ip: BTreeMap<AddrKey, Vec<u64>>,
}

impl Budget {
    pub fn new(per_minute: usize) -> Self {
        Self {
            per_minute,
            per_ip: BTreeMap::new(),
        }
    }

    /// `now` in milliseconds (any monotonic origin).
    pub fn allow(&mut self, ip: IpAddr, now: u64) -> bool {
        let Some(key) = address_key(ip) else { return true };
        // (Within the last minute: `t > now - 60 s` would drop what came at 0 during the first minute.)
        let fresh = |t: &u64| now.saturating_sub(*t) < 60_000;
        let mine = self.per_ip.entry(key).or_default();
        mine.retain(fresh);
        if mine.len() >= self.per_minute {
            return false;
        }
        mine.push(now);
        if self.per_ip.len() > 10_000 {
            self.per_ip.retain(|_, v| v.last().is_some_and(fresh));
        }
        true
    }
}

impl Limiter {
    /// Whether `ip` may guess at `target` now; `now` in milliseconds (any monotonic origin).
    pub fn allow(&mut self, ip: Option<IpAddr>, target: &str, now: u64) -> bool {
        let fresh = |t: &u64| now.saturating_sub(*t) < 60_000;
        let (locked, recent) = self.per_ip.get_mut(&ip.map(AddrKey::of)).map_or((false, false), |g| {
            g.misses.retain(fresh);
            (now < g.locked_until, !g.misses.is_empty())
        });
        if locked {
            return false;
        }
        let busy = self.per_target.get_mut(target).is_some_and(|v| {
            v.retain(fresh);
            v.len() >= GUESSES_PER_TARGET
        });
        !(busy && recent)
    }

    /// Counts a wrong guess.
    pub fn failed(&mut self, ip: Option<IpAddr>, target: &str, now: u64) {
        let g = self.per_ip.entry(ip.map(AddrKey::of)).or_default();
        if now.saturating_sub(g.last_miss) > LOCKOUT_MAX_MS {
            g.strikes = 0;
        }
        g.last_miss = now;
        g.misses.retain(|t| now.saturating_sub(*t) < 60_000);
        g.misses.push(now);
        if g.misses.len() >= GUESSES_PER_IP {
            g.locked_until = now + (LOCKOUT_MS << g.strikes.min(6)).min(LOCKOUT_MAX_MS);
            g.strikes += 1;
            g.misses.clear();
        }
        self.per_target.entry(target.to_string()).or_default().push(now);
        if self.per_ip.len() > 10_000 {
            self.per_ip
                .retain(|_, g| now < g.locked_until || now.saturating_sub(g.last_miss) < LOCKOUT_MAX_MS);
        }
        if self.per_target.len() > 10_000 {
            self.per_target
                .retain(|_, v| v.last().is_some_and(|t| now.saturating_sub(*t) < 60_000));
        }
    }

    /// Wrong guesses at `target` within the last minute.
    pub fn misses_at(&mut self, target: &str, now: u64) -> usize {
        self.per_target.get_mut(target).map_or(0, |v| {
            v.retain(|t| now.saturating_sub(*t) < 60_000);
            v.len()
        })
    }

    /// The target changed (a room's new PIN): its misses no longer count.
    pub fn forget_target(&mut self, target: &str) {
        self.per_target.remove(target);
    }
}

/// Constant-time comparison of a secret (the debug key, a room's PIN) with the expected one.
#[expect(clippy::expect_used, reason = "HMAC takes any key length")]
pub fn same_key(given: &str, want: &str) -> bool {
    let digest = |s: &str| {
        let mut m = HmacSha256::new_from_slice(b"fb-debug").expect("any key length");
        m.update(s.as_bytes());
        m.finalize()
    };
    // `CtOutput` compares in constant time.
    digest(given) == digest(want)
}

/// Player id and the address the token was asked at, sealed into the token's user data (`uid`, NUL, address).
/// Trusted where the link's address is not: a proxied WebSocket comes from 127.0.0.1.
pub fn to_user_data(uid: &Uid, ip: Option<IpAddr>) -> [u8; 256] {
    let mut data = [0u8; 256];
    let ip = ip.map(|ip| ip.to_string()).unwrap_or_default();
    let bytes = uid
        .as_str()
        .bytes()
        .chain((!ip.is_empty()).then_some(0))
        .chain(ip.bytes());
    for (d, b) in data.iter_mut().zip(bytes) {
        *d = b;
    }
    data
}

/// The player id and the address of `to_user_data`.
pub fn from_user_data(data: &[u8]) -> (Option<Uid>, Option<IpAddr>) {
    let mut parts = data.split(|&b| b == 0);
    let uid = parts
        .next()
        .filter(|b| !b.is_empty())
        .map(|b| String::from_utf8_lossy(b).into_owned().into());
    let ip = parts
        .next()
        .and_then(|b| core::str::from_utf8(b).ok())
        .and_then(|s| s.parse().ok());
    (uid, ip)
}

pub fn read_cookie<'a>(header: Option<&'a str>, name: &str) -> Option<&'a str> {
    header?.split(';').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn rate_limits_guessing() {
        let mut l = Limiter::default();
        let mut now = 1_000_000;
        let guess = |l: &mut Limiter, at: &str, target: &str, now: u64| {
            let ok = l.allow(ip(at), target, now);
            if ok {
                l.failed(ip(at), target, now);
            }
            ok
        };
        let (a, b, c) = ("192.0.2.1", "192.0.2.2", "192.0.2.3");
        for _ in 0..5 {
            assert!(guess(&mut l, a, "r1", now));
        }
        assert!(!guess(&mut l, a, "r1", now));
        assert!(!guess(&mut l, a, "r2", now));
        assert!(guess(&mut l, b, "r1", now));
        now += 61_000;
        assert!(guess(&mut l, a, "r1", now));
        // One IPv6 network is one guesser, whichever of its addresses it uses.
        for i in 0..5 {
            assert!(guess(&mut l, &format!("2001:db8:1:2::{i}"), "r1", now));
        }
        assert!(!guess(&mut l, "2001:db8:1:2:ffff::9", "r1", now));
        assert!(guess(&mut l, "2001:db8:1:3::1", "r1", now));
        assert_eq!(
            AddrKey::of("::ffff:10.0.0.1".parse().unwrap()),
            AddrKey::V4(Ipv4Addr::new(10, 0, 0, 1))
        );
        // Right guesses do not count.
        for _ in 0..10 {
            assert!(l.allow(ip(c), "r1", now));
        }
        // Many guessers at one target: past its cap only those without a recent miss may try, so the right
        // PIN still gets in; other targets are not touched.
        for i in 0..30 {
            assert!(guess(&mut l, &format!("10.0.1.{i}"), "r3", now));
        }
        assert_eq!(l.misses_at("r3", now), 30);
        assert!(!l.allow(ip("10.0.1.1"), "r3", now));
        assert!(l.allow(ip("10.0.1.1"), "r4", now));
        assert!(l.allow(ip("10.0.2.1"), "r3", now));
        l.failed(ip("10.0.2.1"), "r3", now);
        assert!(!l.allow(ip("10.0.2.1"), "r3", now));
        l.forget_target("r3");
        assert!(l.allow(ip("10.0.2.1"), "r3", now));
    }

    #[test]
    fn locks_out_for_longer_each_time() {
        let mut l = Limiter::default();
        let ip = ip("203.0.113.4");
        let mut now = 0;
        for lockout in [60_000, 120_000, 240_000] {
            for _ in 0..5 {
                assert!(l.allow(ip, "r", now));
                l.failed(ip, "r", now);
            }
            assert!(!l.allow(ip, "r", now + lockout - 1));
            now += lockout;
            assert!(l.allow(ip, "r", now));
        }
        // An hour and more without a miss: from a minute again.
        now += 2 * LOCKOUT_MAX_MS;
        for _ in 0..5 {
            l.failed(ip, "r", now);
        }
        assert!(l.allow(ip, "r", now + 60_000));
    }

    #[test]
    fn budgets_per_address_spare_this_machine() {
        let mut b = Budget::new(3);
        let at = |s: &str| -> IpAddr { s.parse().unwrap() };
        for _ in 0..3 {
            assert!(b.allow(at("203.0.113.5"), 0));
        }
        assert!(!b.allow(at("203.0.113.5"), 1));
        assert!(b.allow(at("203.0.113.6"), 1));
        assert!(b.allow(at("203.0.113.5"), 61_000));
        for _ in 0..10 {
            assert!(b.allow(at("127.0.0.1"), 2));
            assert!(b.allow(at("::1"), 2));
        }
        assert_eq!(
            address_key(at("2001:db8:1:2::7")),
            Some(AddrKey::V6Net([0x2001, 0xdb8, 1, 2]))
        );
        assert_eq!(address_key(at("::ffff:127.0.0.1")), None);
    }

    #[test]
    fn signs_identities_and_debug_cookies_and_expires_them() {
        let auth = Auth::new(&[7; 32]);
        let other = Auth::new(&[8; 32]);
        let (uid, token) = auth.issue_identity();
        assert_eq!(auth.identity(&token), Some(uid.clone()));
        assert_eq!(auth.identity(&format!("{token}x")), None);
        assert_eq!(auth.identity(""), None);
        assert_eq!(other.identity(&token), None);
        let now = 1_700_000_000;
        let cookie = auth.issue_debug_cookie(now, "key");
        assert!(auth.valid_debug_cookie(&cookie, now, "key"));
        assert!(!auth.valid_debug_cookie(&format!("{cookie}x"), now, "key"));
        assert!(!other.valid_debug_cookie(&cookie, now, "key"));
        assert!(auth.valid_debug_cookie(&cookie, now + 6 * 24 * 3600, "key"));
        assert!(!auth.valid_debug_cookie(&cookie, now + 8 * 24 * 3600, "key"));
        // A new debug key revokes the cookies of the old one.
        assert!(!auth.valid_debug_cookie(&cookie, now, "new key"));
        assert!(!auth.valid_debug_cookie("d1.1700000000.x", now, "key"));
        assert_eq!(auth.identity(&token), Some(uid));
    }

    #[test]
    fn compares_keys_and_reads_cookies() {
        assert!(same_key("1234", "1234"));
        assert!(!same_key("1234", "1235"));
        let abc = Uid::from("abc");
        assert_eq!(from_user_data(&to_user_data(&abc, None)), (Some(abc.clone()), None));
        let ip: IpAddr = "2001:db8::7".parse().unwrap();
        assert_eq!(from_user_data(&to_user_data(&abc, Some(ip))), (Some(abc), Some(ip)));
        assert_eq!(from_user_data(&[0; 256]), (None, None));
        assert_ne!(Auth::new(&[1; 32]).netcode_key(), Auth::new(&[2; 32]).netcode_key());
        assert_eq!(
            read_cookie(Some("a=1; fb_debug=x.y.z; b=2"), DEBUG_COOKIE),
            Some("x.y.z")
        );
        assert_eq!(read_cookie(None, DEBUG_COOKIE), None);
    }
}
