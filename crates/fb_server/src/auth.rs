//! What the server signs with its secret (port of `server/auth.ts`). The game is open to everyone:
//!   identities     a random player id the client keeps, so a player is one person across restarts
//!                  (one room at a time, their own room stays theirs);
//!   connect tokens netcode's, encrypted with a key derived from the secret, the player id inside;
//!   debug cookie   access to the production debug API, earned with the debug key.
//! It also rate limits guessing (PINs of private rooms, the debug key). A new secret resets all of them.
use std::collections::BTreeMap;

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

impl Limiter {
    /// `now` in milliseconds (any monotonic origin).
    pub fn allow(&mut self, ip: &str, now: u64) -> bool {
        let cut = now.saturating_sub(60_000);
        let mine = self.per_ip.entry(ip.to_string()).or_default();
        mine.retain(|&t| t > cut);
        self.global.retain(|&t| t > cut);
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

/// A player id into a connect token's user data, and back (empty when there is none).
pub fn uid_to_user_data(uid: &str) -> [u8; 256] {
    let mut data = [0u8; 256];
    let n = uid.len().min(data.len());
    data[..n].copy_from_slice(&uid.as_bytes()[..n]);
    data
}

pub fn uid_from_user_data(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).into_owned()
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
        assert_eq!(uid_from_user_data(&uid_to_user_data("abc")), "abc");
        assert_eq!(uid_from_user_data(&[0; 256]), "");
        assert_ne!(Auth::new(&[1; 32]).netcode_key(), Auth::new(&[2; 32]).netcode_key());
        assert_eq!(
            read_cookie(Some("a=1; fb_debug=x.y.z; b=2"), DEBUG_COOKIE),
            Some("x.y.z")
        );
        assert_eq!(read_cookie(None, DEBUG_COOKIE), None);
    }
}
