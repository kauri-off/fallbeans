//! sRGB colours, written `#rrggbb` (or `#rrggbbaa`) in files.
use core::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rgb {
    /// `0xrrggbb`.
    pub rgb: u32,
    pub alpha: u8,
}

/// `rgb(0x7ccfff)`: opaque.
pub const fn rgb(v: u32) -> Rgb {
    Rgb { rgb: v, alpha: 255 }
}

/// `rgba(0xbfe9ffa6)`.
pub const fn rgba(v: u32) -> Rgb {
    Rgb {
        rgb: v >> 8,
        alpha: v as u8,
    }
}

impl Rgb {
    pub const WHITE: Rgb = rgb(0xffffff);

    pub const fn bytes(self) -> [u8; 3] {
        [(self.rgb >> 16) as u8, (self.rgb >> 8) as u8, self.rgb as u8]
    }

    pub fn from_bytes([r, g, b]: [u8; 3]) -> Self {
        rgb(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
    }

    /// `#rrggbb` or `#rrggbbaa` (the `#` may be left out).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#').unwrap_or(s);
        let v = u32::from_str_radix(s, 16).ok()?;
        match s.len() {
            6 => Some(rgb(v)),
            8 => Some(rgba(v)),
            _ => None,
        }
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06x}", self.rgb)?;
        if self.alpha != 255 {
            write!(f, "{:02x}", self.alpha)?;
        }
        Ok(())
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("not a colour: {s}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_hex() {
        assert_eq!(rgb(0x07cfff).to_string(), "#07cfff");
        assert_eq!(rgba(0xbfe9ffa6).to_string(), "#bfe9ffa6");
        assert_eq!(Rgb::parse("#bfe9ffa6"), Some(rgba(0xbfe9ffa6)));
        assert_eq!(Rgb::parse("#7CCFFF"), Some(rgb(0x7ccfff)));
        assert_eq!(Rgb::parse("rainbow"), None);
        assert_eq!(rgb(0x123456).bytes(), [0x12, 0x34, 0x56]);
        assert_eq!(Rgb::from_bytes([0x12, 0x34, 0x56]), rgb(0x123456));
    }
}
