//! FNV-1a over quantized numbers: fingerprints of simulation state that two runs or two builds compare.
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub struct Fnv(u64);

/// How many things were hashed, and their hash: `n:hash` in hex.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    pub n: u32,
    pub h: u64,
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{:016x}", self.n, self.h)
    }
}

/// A hash of a whole simulation state: 16 hex digits (and so in JSON).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateHash(pub u64);

impl fmt::Display for StateHash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl Serialize for StateHash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for StateHash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        u64::from_str_radix(&s, 16).map(Self).map_err(serde::de::Error::custom)
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    /// Mixes in `v` rounded to a multiple of `1 / scale`. NaN mixes in as a value no number rounds to.
    pub fn mix(&mut self, v: f64, scale: f64) {
        // (`as` turns NaN into 0; a rounded f64 is never i64::MIN + 1: near 2⁶³ they are 1024 apart.)
        let q = if v.is_nan() {
            i64::MIN + 1
        } else {
            (v * scale).round() as i64
        };
        self.bytes(&q.to_le_bytes());
    }

    /// Mixes in `v` exactly, by its bits.
    pub fn bits(&mut self, v: f64) {
        self.int(v.to_bits());
    }

    pub fn int(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn bytes(&mut self, b: &[u8]) {
        for &b in b {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixed(v: f64) -> u64 {
        let mut h = Fnv::default();
        h.mix(v, 1e5);
        h.finish()
    }

    #[test]
    fn nan_is_not_zero() {
        assert_ne!(mixed(f64::NAN), mixed(0.0));
        assert_ne!(mixed(f64::NAN), mixed(f64::NEG_INFINITY));
        assert_eq!(mixed(1.0), mixed(1.0 + 1e-9));
    }
}
