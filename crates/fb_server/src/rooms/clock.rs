//! A room's game time (port of `server/rooms/clock.ts`), in ticks: the server's tick, unless a dev
//! command slowed, paused or warped it. Timers and arenas follow game time.

#[derive(Clone, Debug)]
pub struct GameClock {
    /// Game ticks per server tick (dev: slow motion, 0 = paused).
    pub rate: f64,
    time_base: f64,
    real_base: f64,
    /// The server tick as last told.
    real: u64,
}

impl GameClock {
    pub fn new(real: u64) -> Self {
        Self {
            rate: 1.0,
            time_base: real as f64,
            real_base: real as f64,
            real,
        }
    }

    pub fn real(&self) -> u64 {
        self.real
    }

    pub fn set_real(&mut self, real: u64) {
        self.real = real;
    }

    /// Game time (ticks; fractional only after a dev change of rate).
    pub fn now(&self) -> f64 {
        if !self.shifted() {
            return self.real as f64;
        }
        self.time_base + (self.real as f64 - self.real_base) * self.rate
    }

    /// Game time no longer is the server's tick.
    pub fn shifted(&self) -> bool {
        self.rate != 1.0 || self.time_base != self.real_base
    }

    /// Game minus server time: the server tick of game tick `g` is `g - offset()` (exact while the rate is 1).
    pub fn offset(&self) -> f64 {
        self.now() - self.real as f64
    }

    /// Game time runs `k` times as fast from now on; returns the game time of the change.
    pub fn set_rate(&mut self, k: f64) -> f64 {
        let now = self.rebase();
        self.rate = k;
        now
    }

    /// Starts counting from the current game time (before a rate change or a jump).
    pub fn rebase(&mut self) -> f64 {
        let now = self.now();
        self.time_base = now;
        self.real_base = self.real as f64;
        now
    }

    /// Jumps game time forward.
    pub fn skip(&mut self, ticks: f64) {
        self.time_base += ticks;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_server_until_shifted() {
        let mut c = GameClock::new(100);
        c.set_real(160);
        assert_eq!(c.now(), 160.0);
        assert!(!c.shifted());
        c.set_rate(0.5);
        c.set_real(200);
        assert_eq!(c.now(), 180.0);
        c.set_rate(1.0);
        c.rebase();
        c.skip(30.0);
        c.set_real(210);
        assert_eq!(c.now(), 220.0);
        assert_eq!(c.offset(), 10.0);
    }
}
