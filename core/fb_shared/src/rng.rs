/// mulberry32: a small seeded generator, the same sequence on every platform.
#[derive(Clone, Debug)]
pub struct Rng {
    a: u32,
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self { a: seed }
    }

    /// Where the generator is (state hashes compare it).
    pub fn state(&self) -> u32 {
        self.a
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b_79f5);
        let a = self.a;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    /// Uniform in 0..len (0 for len 0).
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "unit() is in [0, 1): the floor is in 0..len"
    )]
    pub fn index(&mut self, len: usize) -> usize {
        (self.unit() * len as f64).floor() as usize
    }

    /// One element, drawn uniformly. Panics on an empty slice.
    pub fn pick<'a, T>(&mut self, a: &'a [T]) -> &'a T {
        &a[self.index(a.len())]
    }
}

/// Fisher–Yates, drawing from the end down.
pub fn shuffle<T>(a: &mut [T], rng: &mut Rng) {
    for i in (1..a.len()).rev() {
        let j = rng.index(i + 1);
        a.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mulberry32_sequence() {
        let mut r = Rng::new(12345);
        let got: Vec<f64> = (0..4).map(|_| r.unit()).collect();
        assert_eq!(
            got,
            [
                0.9797282677609473,
                0.3067522644996643,
                0.484205421525985,
                0.817934412509203
            ]
        );
    }
}
