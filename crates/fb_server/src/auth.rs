//! What the server signs with its secret (port of `server/auth.ts`). The game is open to everyone:
//!   identities     a random player id the client keeps, so a player is one person across restarts
//!                  (one room at a time, their own room stays theirs);
//!   connect tokens netcode's, encrypted with a key derived from the secret, the player id inside;
//!   debug cookie   access to the production debug API, earned with the debug key.
//! It also rate limits guessing (PINs of private rooms, the debug key). A new secret resets all of them.
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv6Addr};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, Mac};
use sha2::Sha256;

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
    pub fn issue_identity(&self) -> (String, String) {
        let uid = B64.encode(random_bytes::<12>());
        let payload = format!("u1.{uid}");
        let token = format!("{payload}.{}", self.sign(&payload));
        (uid, token)
    }

    /// The player id of an identity token, or None when it is not one of ours.
    pub fn identity(&self, token: &str) -> Option<String> {
        let mut parts = token.split('.');
        let (Some("u1"), Some(uid), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return None;
        };
        (!uid.is_empty() && self.verify(&format!("u1.{uid}"), sig)).then(|| uid.to_string())
    }

    /// Debug API access (production): a week. `now` in seconds since the epoch.
    pub fn issue_debug_cookie(&self, now: u64) -> String {
        let payload = format!("d1.{now}");
        format!("{payload}.{}", self.sign(&payload))
    }

    pub fn valid_debug_cookie(&self, value: &str, now: u64) -> bool {
        let mut parts = value.split('.');
        let (Some("d1"), Some(at), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return false;
        };
        let Ok(at) = at.parse::<u64>() else { return false };
        if at > now + 60 || now.saturating_sub(at) > DEBUG_TTL_S {
            return false;
        }
        self.verify(&format!("d1.{at}"), sig)
    }
}

/// Rate limit for guesses: 5 attempts per minute per address, 30 per minute in total.
#[derive(Default)]
pub struct Limiter {
    per_ip: BTreeMap<String, Vec<u64>>,
    global: Vec<u64>,
}

/// Who a guess is counted against: the address, or for IPv6 its /64 (one subscriber's network has
/// billions of addresses).
fn guesser(ip: &str) -> String {
    match ip.parse::<Ipv6Addr>() {
        Ok(v6) if v6.to_ipv4_mapped().is_none() => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
        _ => ip.to_string(),
    }
}

/// Who a limit per address counts against (`guesser`); None for this machine and unknown addresses, which
/// no such limit applies to (dev servers, stress runs: every client is 127.0.0.1).
pub fn address_key(ip: &str) -> Option<String> {
    let parsed: IpAddr = ip.parse().ok()?;
    let parsed = parsed.to_canonical();
    (!parsed.is_loopback() && !parsed.is_unspecified()).then(|| guesser(ip))
}

/// So many requests a minute per address (`address_key`).
pub struct Budget {
    per_minute: usize,
    per_ip: BTreeMap<String, Vec<u64>>,
}

impl Budget {
    pub fn new(per_minute: usize) -> Self {
        Self {
            per_minute,
            per_ip: BTreeMap::new(),
        }
    }

    /// `now` in milliseconds (any monotonic origin).
    pub fn allow(&mut self, ip: &str, now: u64) -> bool {
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
    /// `now` in milliseconds (any monotonic origin).
    pub fn allow(&mut self, ip: &str, now: u64) -> bool {
        let fresh = |t: &u64| now.saturating_sub(*t) < 60_000;
        let mine = self.per_ip.entry(guesser(ip)).or_default();
        mine.retain(fresh);
        self.global.retain(fresh);
        if mine.len() >= 5 || self.global.len() >= 30 {
            return false;
        }
        mine.push(now);
        self.global.push(now);
        if self.per_ip.len() > 10_000 {
            self.per_ip.clear();
        }
        true
    }
}

/// Constant-time comparison of a secret (the debug key, a room's PIN) with the expected one.
pub fn same_key(given: &str, want: &str) -> bool {
    let digest = |s: &str| {
        let mut m = HmacSha256::new_from_slice(b"fb-debug").expect("any key length");
        m.update(s.as_bytes());
        m.finalize()
    };
    // `CtOutput` compares in constant time.
    digest(given) == digest(want)
}

/// A player id and the address their client asked for the token from (behind nginx: `X-Real-IP`) into a
/// connect token's user data: `uid`, NUL, the address. The token is sealed with the server's key, so the
/// address can be trusted where the link's own cannot (a WebSocket through nginx comes from 127.0.0.1).
pub fn to_user_data(uid: &str, ip: Option<IpAddr>) -> [u8; 256] {
    let mut data = [0u8; 256];
    let ip = ip.map(|ip| ip.to_string()).unwrap_or_default();
    let bytes = uid.bytes().chain((!ip.is_empty()).then_some(0)).chain(ip.bytes());
    for (d, b) in data.iter_mut().zip(bytes) {
        *d = b;
    }
    data
}

/// The player id (empty when there is none) and the address of `to_user_data`.
pub fn from_user_data(data: &[u8]) -> (String, Option<IpAddr>) {
    let mut parts = data.split(|&b| b == 0);
    let uid = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
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

    #[test]
    fn rate_limits_guessing() {
        let mut l = Limiter::default();
        let mut now = 1_000_000;
        for _ in 0..5 {
            assert!(l.allow("a", now));
        }
        assert!(!l.allow("a", now));
        assert!(l.allow("b", now));
        now += 61_000;
        assert!(l.allow("a", now));
        // One IPv6 network is one guesser, whichever of its addresses it uses.
        for i in 0..5 {
            assert!(l.allow(&format!("2001:db8:1:2::{i}"), now));
        }
        assert!(!l.allow("2001:db8:1:2:ffff::9", now));
        assert!(l.allow("2001:db8:1:3::1", now));
        assert_eq!(guesser("::ffff:10.0.0.1"), "::ffff:10.0.0.1");
    }

    #[test]
    fn budgets_per_address_spare_this_machine() {
        let mut b = Budget::new(3);
        for _ in 0..3 {
            assert!(b.allow("203.0.113.5", 0));
        }
        assert!(!b.allow("203.0.113.5", 1));
        assert!(b.allow("203.0.113.6", 1));
        assert!(b.allow("203.0.113.5", 61_000));
        for _ in 0..10 {
            assert!(b.allow("127.0.0.1", 2));
            assert!(b.allow("::1", 2));
            assert!(b.allow("?", 2));
        }
        assert_eq!(address_key("2001:db8:1:2::7").as_deref(), Some("2001:db8:1:2::/64"));
        assert_eq!(address_key("::ffff:127.0.0.1"), None);
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
        let cookie = auth.issue_debug_cookie(now);
        assert!(auth.valid_debug_cookie(&cookie, now));
        assert!(!auth.valid_debug_cookie(&format!("{cookie}x"), now));
        assert!(!other.valid_debug_cookie(&cookie, now));
        assert!(auth.valid_debug_cookie(&cookie, now + 6 * 24 * 3600));
        assert!(!auth.valid_debug_cookie(&cookie, now + 8 * 24 * 3600));
        assert_eq!(auth.identity(&token), Some(uid));
    }

    #[test]
    fn compares_keys_and_reads_cookies() {
        assert!(same_key("1234", "1234"));
        assert!(!same_key("1234", "1235"));
        assert_eq!(from_user_data(&to_user_data("abc", None)), ("abc".into(), None));
        let ip: IpAddr = "2001:db8::7".parse().unwrap();
        assert_eq!(from_user_data(&to_user_data("abc", Some(ip))), ("abc".into(), Some(ip)));
        assert_eq!(from_user_data(&[0; 256]), (String::new(), None));
        assert_ne!(Auth::new(&[1; 32]).netcode_key(), Auth::new(&[2; 32]).netcode_key());
        assert_eq!(
            read_cookie(Some("a=1; fb_debug=x.y.z; b=2"), DEBUG_COOKIE),
            Some("x.y.z")
        );
        assert_eq!(read_cookie(None, DEBUG_COOKIE), None);
    }
}
