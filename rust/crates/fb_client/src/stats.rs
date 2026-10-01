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
        app.init_resource::<Hiccups>();
        app.add_observer(on_shift);
        app.add_systems(
            Update,
            (
                track_frames,
                log_stats.run_if(on_timer(core::time::Duration::from_secs(1))),
            )
                .chain(),
        );
    }
}

/// What tells a stall of this machine from one of the network: the longest frame of the second (a
/// frozen client sends nothing, and its inputs reach the server late) and the shifts of the local clock
/// (Lightyear's resync when its lead is off by more than `SyncConfig::max_error_margin`: the input
/// buffer is relabelled, and inputs already sent move to other ticks).
#[derive(Resource, Default)]
struct Hiccups {
    frame_max: f64,
    shifts: Vec<i32>,
}

fn track_frames(time: Res<Time<Real>>, mut h: ResMut<Hiccups>) {
    h.frame_max = h.frame_max.max(time.delta_secs_f64());
}

fn on_shift(trigger: On<LocalTimelineShift>, mut h: ResMut<Hiccups>) {
    h.shifts.push(trigger.delta);
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
    mut hiccups: ResMut<Hiccups>,
    mut last_bytes: Local<u64>,
) {
    let link = links.iter().next();
    let rtt = link.map_or(0.0, |l| l.stats.rtt.as_secs_f64() * 1000.0);
    let jitter = link.map_or(0.0, |l| l.stats.jitter.as_secs_f64() * 1000.0);
    let (rollbacks, rb_ticks) = metrics.map_or((0, 0), |m| (m.rollbacks, m.rollback_ticks));
    let t = map.as_ref().map_or(0.0, |m| m.time(timeline.tick().0 as f64));
    let out = net.bytes_out - *last_bytes;
    *last_bytes = net.bytes_out;
    let frame_max = core::mem::take(&mut hiccups.frame_max) * 1000.0;
    let shifts = core::mem::take(&mut hiccups.shifts);
    info!(
        "stats: {} | rtt {rtt:.0} ms jitter {jitter:.0} ms | frame max {frame_max:.0} ms | shifts {shifts:?} | rollbacks {rollbacks} ({rb_ticks} ticks) | predicted {} | others {} | events {} | out {out} B/s | round {} t {t:.1}{}",
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
