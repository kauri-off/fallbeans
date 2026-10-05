//! What the log says about the connection: a `net:` summary a minute while connected, and a line as soon as
//! something goes wrong (loss, a slow round trip, a long frame, a clock jump). `NetNow` is the same for other
//! lines (a correction, an F8 report). `stats:` (every second headless) is what stress runs compare.
use bevy::prelude::*;
use fb_net::NetStats;
use lightyear::prelude::*;

use crate::game::{Map, Stats};
use crate::net::Conn;
use crate::opts::Opts;
use crate::watch::Corrections;

const SUMMARY_S: f32 = 60.0;
/// A frame this long is logged (ms), then no other for `FRAME_QUIET_S` (they are only counted).
const LONG_FRAME_MS: f64 = 250.0;
const FRAME_QUIET_S: f32 = 30.0;
/// Loss over the last 10 s that is logged, and below which it is over.
const LOSS_BAD: f32 = 0.05;
const LOSS_OK: f32 = 0.02;
/// The same for the round trip, ms.
const RTT_BAD: f64 = 250.0;
const RTT_OK: f64 = 150.0;
/// How long a hiccup stays in `NetNow::line`, s.
const RECENT_S: f32 = 5.0;

pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetNow>();
        app.init_resource::<Window>();
        app.init_resource::<Alarms>();
        app.init_resource::<StatsLine>();
        app.add_observer(on_shift);
        app.add_systems(Update, (watch, summary, log_stats).chain());
    }
}

/// The connection as of this frame, and its last hiccups (real time, s).
#[derive(Resource, Default)]
pub struct NetNow {
    pub connected: bool,
    pub transport: String,
    pub rtt: f64,
    pub jitter: f64,
    pub loss: Option<f32>,
    pub lead: Option<f32>,
    /// The last clock jump (Lightyear's resync when its lead is off by more than
    /// `SyncConfig::max_error_margin`: inputs already sent move to other ticks): when, by how many ticks.
    pub shift: Option<(f32, i32)>,
    /// The last frame over `LONG_FRAME_MS` (a stall of this machine: its inputs reach the server late):
    /// when, how long in ms.
    pub long_frame: Option<(f32, f64)>,
}

impl NetNow {
    /// `rtt 72 ms jitter 4 ms loss 3.7% lead 4.9`, and the hiccups of the last seconds.
    pub fn line(&self, now: f32) -> String {
        let mut s = format!(
            "rtt {:.0} ms jitter {:.0} ms loss {} lead {}",
            self.rtt,
            self.jitter,
            pct(self.loss),
            self.lead.map_or("—".into(), |l| format!("{l:.1}"))
        );
        if let Some((t, d)) = self.shift.filter(|(t, _)| now - t < RECENT_S) {
            s += &format!(" | clock jump {d:+} ticks {:.1} s ago", now - t);
        }
        if let Some((t, ms)) = self.long_frame.filter(|(t, _)| now - t < RECENT_S) {
            s += &format!(" | frame of {ms:.0} ms {:.1} s ago", now - t);
        }
        s
    }
}

fn pct(loss: Option<f32>) -> String {
    loss.map_or("—".into(), |l| format!("{:.1}%", l * 100.0))
}

/// Since the last `net:` line (or the connection).
#[derive(Resource, Default)]
struct Window {
    since: f32,
    rtt_sum: f64,
    rtt_max: f64,
    samples: u32,
    frame_max: f64,
    shifts: u32,
    /// `PredictionMetrics` at the start.
    rollbacks: (u32, u32),
}

/// What has been said about hiccups that last.
#[derive(Resource, Default)]
struct Alarms {
    long_quiet_until: f32,
    long_hushed: u32,
    loss_bad: bool,
    rtt_bad: bool,
    /// The clock was set since the connection came up.
    synced: bool,
}

/// Since the last `stats:` line.
#[derive(Resource, Default)]
struct StatsLine {
    at: f32,
    frame_max: f64,
    shifts: Vec<i32>,
}

fn on_shift(
    trigger: On<LocalTimelineShift>,
    time: Res<Time<Real>>,
    mut net: ResMut<NetNow>,
    mut w: ResMut<Window>,
    mut alarms: ResMut<Alarms>,
    mut line: ResMut<StatsLine>,
) {
    let now = time.elapsed_secs();
    line.shifts.push(trigger.delta);
    // (The first one of a connection sets the clock.)
    if !core::mem::replace(&mut alarms.synced, true) {
        info!("clock set: {:+} ticks", trigger.delta);
        return;
    }
    w.shifts += 1;
    warn!(
        "clock jumped {:+} ticks: inputs already sent moved to other ticks | {}",
        trigger.delta,
        net.line(now)
    );
    net.shift = Some((now, trigger.delta));
}

fn watch(
    time: Res<Time<Real>>,
    conn: Option<Res<Conn>>,
    links: Query<&Link>,
    diag: Res<crate::diag::NetDiag>,
    lead: Res<crate::clock::Lead>,
    metrics: Option<Res<PredictionMetrics>>,
    mut net: ResMut<NetNow>,
    mut w: ResMut<Window>,
    mut alarms: ResMut<Alarms>,
    mut line: ResMut<StatsLine>,
) {
    let now = time.elapsed_secs();
    let link = links.iter().next();
    net.connected = conn.as_ref().is_some_and(|c| c.connected);
    net.transport = conn.as_ref().map_or("not connected".into(), |c| {
        format!(
            "{:?} {}{}",
            c.transport,
            if c.connected { "connected" } else { "connecting" },
            match c.fallback {
                Some((f, _)) => format!(" (fallback: {f:?})"),
                None => String::new(),
            }
        )
    });
    net.rtt = link.map_or(0.0, |l| l.stats.rtt.as_secs_f64() * 1000.0);
    net.jitter = link.map_or(0.0, |l| l.stats.jitter.as_secs_f64() * 1000.0);
    net.loss = diag.loss();
    net.lead = lead.held.map(|_| lead.margin);
    let frame = time.delta_secs_f64() * 1000.0;
    line.frame_max = line.frame_max.max(frame);
    if !net.connected {
        alarms.synced = false;
        let rollbacks = metrics.map_or((0, 0), |m| (m.rollbacks, m.rollback_ticks));
        *w = Window {
            since: now,
            rollbacks,
            ..default()
        };
        return;
    }
    w.rtt_sum += net.rtt;
    w.rtt_max = w.rtt_max.max(net.rtt);
    w.samples += 1;
    w.frame_max = w.frame_max.max(frame);
    if frame >= LONG_FRAME_MS {
        net.long_frame = Some((now, frame));
        if now >= alarms.long_quiet_until {
            let more = core::mem::take(&mut alarms.long_hushed);
            let more = if more > 0 {
                format!(" ({more} more since the last such line)")
            } else {
                String::new()
            };
            warn!("long frame: {frame:.0} ms{more}");
            alarms.long_quiet_until = now + FRAME_QUIET_S;
        } else {
            alarms.long_hushed += 1;
        }
    }
    if let Some(loss) = net.loss {
        if !alarms.loss_bad && loss >= LOSS_BAD {
            alarms.loss_bad = true;
            warn!("packet loss {} over 10 s | {}", pct(net.loss), net.line(now));
        } else if alarms.loss_bad && loss < LOSS_OK {
            alarms.loss_bad = false;
            info!("packet loss back to {}", pct(net.loss));
        }
    }
    if !alarms.rtt_bad && net.rtt >= RTT_BAD {
        alarms.rtt_bad = true;
        warn!("slow round trip | {}", net.line(now));
    } else if alarms.rtt_bad && net.rtt < RTT_OK {
        alarms.rtt_bad = false;
        info!("round trip back to {:.0} ms", net.rtt);
    }
}

/// One `net:` line a minute while connected.
fn summary(
    time: Res<Time<Real>>,
    net: Res<NetNow>,
    metrics: Option<Res<PredictionMetrics>>,
    mut corrections: ResMut<Corrections>,
    mut w: ResMut<Window>,
) {
    let now = time.elapsed_secs();
    if !net.connected || now - w.since < SUMMARY_S {
        return;
    }
    let rollbacks = metrics.map_or((0, 0), |m| (m.rollbacks, m.rollback_ticks));
    let (fixes, worst) = core::mem::take(&mut corrections.window);
    info!(
        "net: {} | rtt avg {:.0} max {:.0} ms jitter {:.0} ms | loss {} | frame max {:.0} ms | clock jumps {} | rollbacks {} ({} ticks) | corrections {fixes} (max {worst:.2} m)",
        net.transport,
        w.rtt_sum / f64::from(w.samples.max(1)),
        w.rtt_max,
        net.jitter,
        pct(net.loss),
        w.frame_max,
        w.shifts,
        rollbacks.0.wrapping_sub(w.rollbacks.0),
        rollbacks.1.wrapping_sub(w.rollbacks.1),
    );
    *w = Window {
        since: now,
        rollbacks,
        ..default()
    };
}

/// `stats:` every `--stats-every` s: what stress runs read and compare.
fn log_stats(
    opts: Res<Opts>,
    time: Res<Time<Real>>,
    net: Res<NetNow>,
    map: Option<Res<Map>>,
    stats: Res<Stats>,
    bytes: Res<NetStats>,
    metrics: Option<Res<PredictionMetrics>>,
    timeline: Res<LocalTimeline>,
    others: Query<(), With<Interpolated>>,
    mut line: ResMut<StatsLine>,
    mut last_bytes: Local<u64>,
) {
    let every = opts.stats_every.unwrap_or(if opts.headless { 1.0 } else { 0.0 });
    let now = time.elapsed_secs();
    if every <= 0.0 || now - line.at < every {
        return;
    }
    line.at = now;
    let (rollbacks, rb_ticks) = metrics.map_or((0, 0), |m| (m.rollbacks, m.rollback_ticks));
    let t = map.as_ref().map_or(0.0, |m| m.time(timeline.tick().0 as f64));
    let out = bytes.bytes_out - *last_bytes;
    *last_bytes = bytes.bytes_out;
    let frame_max = core::mem::take(&mut line.frame_max);
    let shifts = core::mem::take(&mut line.shifts);
    info!(
        "stats: {} | rtt {:.0} ms jitter {:.0} ms loss {} | lead {} | frame max {frame_max:.0} ms | shifts {shifts:?} | rollbacks {rollbacks} ({rb_ticks} ticks) | predicted {} | others {} | events {} | out {out} B/s | arena {} t {t:.1}{}",
        net.transport,
        net.rtt,
        net.jitter,
        pct(net.loss),
        net.lead.map_or("—".into(), |l| format!("{l:.1}")),
        stats.ticks,
        others.iter().count(),
        stats.map_events,
        map.as_ref().map_or(0, |m| m.round.arena),
        if stats.hash_mismatch {
            " | MAP HASH MISMATCH"
        } else {
            ""
        },
    );
}
