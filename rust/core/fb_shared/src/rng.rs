/// mulberry32, bit for bit as in `shared/rng.ts`.
#[derive(Clone, Debug)]
pub struct Rng {
    a: u32,
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self { a: seed }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b_79f5);
        let a = self.a;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        (t ^ (t >> 14)) as f64 / 4_294_967_296.0
    }

    pub fn pick<'a, T>(&mut self, a: &'a [T]) -> &'a T {
        &a[(self.next() * a.len() as f64).floor() as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_js() {
        let mut r = Rng::new(12345);
        let got: Vec<f64> = (0..4).map(|_| r.next()).collect();
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
