use core::time::Duration;

use lightyear::prelude::LinkConditionerConfig;

/// Simulated bad network on incoming packets (each side conditions what it receives, so the same
/// flags on server and client give RTT ≈ 2 × lag).
#[derive(clap::Args, Clone, Copy, Debug, Default)]
pub struct NetSim {
    /// One-way latency added to incoming packets, ms.
    #[arg(long, default_value_t = 0)]
    pub lag: u64,
    /// Random extra delay, ms.
    #[arg(long, default_value_t = 0)]
    pub jitter: u64,
    /// Share of incoming packets dropped, 0..1.
    #[arg(long, default_value_t = 0.0)]
    pub loss: f32,
}

impl NetSim {
    pub fn config(&self) -> Option<LinkConditionerConfig> {
        (self.lag > 0 || self.jitter > 0 || self.loss > 0.0).then(|| {
            LinkConditionerConfig::default()
                .with_incoming_latency(Duration::from_millis(self.lag))
                .with_incoming_jitter(Duration::from_millis(self.jitter))
                .with_fixed_loss(self.loss)
        })
    }
}
