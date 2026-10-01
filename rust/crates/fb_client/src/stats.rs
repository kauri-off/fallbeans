//! One `stats:` line a second in the log, windowed or headless: what stress runs and humans compare.
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use fb_net::NetStats;
use lightyear::prelude::*;

use crate::game::{Map, Stats};
use crate::net::Conn;

pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, log_stats.run_if(on_timer(core::time::Duration::from_secs(1))));
    }
}

fn log_stats(
    conn: Option<Res<Conn>>,
    map: Option<Res<Map>>,
    stats: Res<Stats>,
    net: Res<NetStats>,
    metrics: Option<Res<PredictionMetrics>>,
    links: Query<&Link>,
    timeline: Res<LocalTimeline>,
    others: Query<(), With<Interpolated>>,
    mut last_bytes: Local<u64>,
) {
    let link = links.iter().next();
    let rtt = link.map_or(0.0, |l| l.stats.rtt.as_secs_f64() * 1000.0);
    let jitter = link.map_or(0.0, |l| l.stats.jitter.as_secs_f64() * 1000.0);
    let (rollbacks, rb_ticks) = metrics.map_or((0, 0), |m| (m.rollbacks, m.rollback_ticks));
    let t = map.as_ref().map_or(0.0, |m| m.time(timeline.tick().0 as f64));
    let out = net.bytes_out - *last_bytes;
    *last_bytes = net.bytes_out;
    info!(
        "stats: {} | rtt {rtt:.0} ms jitter {jitter:.0} ms | rollbacks {rollbacks} ({rb_ticks} ticks) | predicted {} | others {} | events {} | out {out} B/s | round {} t {t:.1}{}",
        conn.map_or("not connected".into(), |c| format!(
            "{:?} {}",
            c.transport,
            if c.connected { "connected" } else { "connecting" }
        )),
        stats.ticks,
        others.iter().count(),
        stats.map_events,
        map.as_ref().map_or(0, |m| m.round.number),
        if stats.hash_mismatch {
            " | MAP HASH MISMATCH"
        } else {
            ""
        },
    );
}
