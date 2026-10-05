//! FNV-1a over quantized numbers: fingerprints of simulation state that two runs or two builds compare.
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    /// Mixes in `v` rounded to a multiple of `1 / scale`.
    pub fn mix(&mut self, v: f64, scale: f64) {
        for b in ((v * scale).round() as i64).to_le_bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}
