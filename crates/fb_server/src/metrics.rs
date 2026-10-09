//! What stress runs measure on the server, logged as one `metrics:` line every `--metrics-every` s (with
//! nobody playing, only the first): the rooms' tick cost (all rooms), the longest frame (a stalled server
//! reads inputs late), ticks run without a player's input in time, traffic out, process CPU (share of one
//! core over the window) and resident memory.
use std::time::Instant;

use bevy::prelude::*;
use fb_net::NetStats;
use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::http::HttpShared;

/// One metrics period, for `/api/debug/health`.
#[derive(Serialize, Clone, Debug)]
pub struct Sample {
    /// Unix time, s.
    at: u64,
    players: usize,
    bots: usize,
    rooms: usize,
    tick_us: TickUs,
    frame_max_ms: f64,
    input_missed: u32,
    out_bps: f64,
    packets_per_s: f64,
    cpu: f64,
    mem_mb: f64,
}

#[derive(Serialize, Clone, Debug)]
struct TickUs {
    mean: f64,
    p50: u32,
    p99: u32,
    max: u32,
}
use crate::opts::Opts;
use crate::play::{InputState, Pawn, RoomTick, Rooms};

pub struct MetricsPlugin;

impl Plugin for MetricsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TickTimes>();
        app.insert_resource(Usage {
            sys: System::new(),
            pid: Pid::from_u32(std::process::id()),
            cpu_ms: None,
        });
        // (The tick's time is all between these two: a FixedUpdate system not ordered against `RoomTick` may
        // fall inside, and counts then, or outside.)
        app.add_systems(FixedUpdate, (start_tick.before(RoomTick), end_tick.after(RoomTick)));
        app.add_systems(Update, (track_frames, report, exit_after).chain());
    }
}

#[derive(Resource, Default)]
struct TickTimes {
    started: Option<Instant>,
    /// Microseconds of each room tick since the last report.
    us: Vec<u32>,
    last_report: f64,
    last_bytes: u64,
    last_packets: u64,
    frame_max: f64,
    /// The last line had nobody playing: the next empty ones are not logged.
    idle: bool,
}

#[derive(Resource)]
struct Usage {
    sys: System,
    pid: Pid,
    /// The process's CPU time at the last call, ms (None before the first).
    cpu_ms: Option<u64>,
}

impl Usage {
    /// Percent of one core since the last call (NaN the first time: that would be the start-up's) and the
    /// resident set in MB.
    fn sample(&mut self, span: f64) -> (f64, f64) {
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::Some(&[self.pid]), false, kind);
        let Some(p) = self.sys.process(self.pid) else {
            return (f64::NAN, f64::NAN);
        };
        let ms = p.accumulated_cpu_time();
        let cpu = self
            .cpu_ms
            .replace(ms)
            .map_or(f64::NAN, |was| ms.saturating_sub(was) as f64 / 10.0 / span);
        (cpu, p.memory() as f64 / 1048576.0)
    }
}

fn track_frames(time: Res<Time<Real>>, mut t: ResMut<TickTimes>) {
    t.frame_max = t.frame_max.max(time.delta_secs_f64());
}

fn start_tick(mut t: ResMut<TickTimes>) {
    t.started = Some(Instant::now());
}

fn end_tick(mut t: ResMut<TickTimes>) {
    if let Some(s) = t.started.take() {
        t.us.push(u32::try_from(s.elapsed().as_micros()).unwrap_or(u32::MAX));
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "an index below len"
)]
fn report(
    opts: Res<Opts>,
    time: Res<Time<Real>>,
    mut t: ResMut<TickTimes>,
    stats: Res<NetStats>,
    mut usage: ResMut<Usage>,
    mut pawns: Query<&mut InputState, With<Pawn>>,
    rooms: Option<Res<Rooms>>,
    shared: Res<HttpShared>,
) {
    let now = time.elapsed_secs_f64();
    let span = now - t.last_report;
    if opts.metrics_every <= 0.0 || opts.metrics_every.is_nan() {
        // Off: nothing reads the samples, which would otherwise pile up (120 a second, for good).
        t.us.clear();
        for mut st in &mut pawns {
            st.missed = 0;
        }
        return;
    }
    if span < opts.metrics_every {
        return;
    }
    t.last_report = now;
    let mut us = std::mem::take(&mut t.us);
    us.sort_unstable();
    let pct = |p: f64| {
        us.get(((us.len() as f64 - 1.0) * p).round() as usize)
            .copied()
            .unwrap_or(0)
    };
    let mean = us.iter().map(|&u| f64::from(u)).sum::<f64>() / us.len().max(1) as f64;
    let bytes = (stats.bytes_out - t.last_bytes) as f64 / span;
    let packets = (stats.packets_out - t.last_packets) as f64 / span;
    (t.last_bytes, t.last_packets) = (stats.bytes_out, stats.packets_out);
    let (mut players, mut bots, mut open) = (0, 0, 0);
    for room in rooms.iter().flat_map(|r| r.hub.rooms.values()) {
        open += 1;
        players += room.players.iter().filter(|p| p.conn().is_some()).count();
        bots += room.players.iter().filter(|p| p.is_bot()).count();
    }
    let (mut missed, mut missed_max) = (0, 0);
    for mut st in &mut pawns {
        missed += st.missed;
        missed_max = missed_max.max(core::mem::take(&mut st.missed));
    }
    let frame_max = core::mem::take(&mut t.frame_max) * 1000.0;
    let (cpu, mem) = usage.sample(span);
    shared.0.sample(Sample {
        at: std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .unwrap_or_default()
            .as_secs(),
        players,
        bots,
        rooms: open,
        tick_us: TickUs {
            mean: mean.round(),
            p50: pct(0.5),
            p99: pct(0.99),
            max: us.last().copied().unwrap_or(0),
        },
        frame_max_ms: frame_max.round(),
        input_missed: missed,
        out_bps: bytes.round(),
        packets_per_s: packets.round(),
        cpu: (cpu * 10.0).round() / 10.0,
        mem_mb: mem.round(),
    });
    let idle = players == 0;
    if core::mem::replace(&mut t.idle, idle) && idle {
        return;
    }
    info!(
        "metrics: players {players} bots {bots} rooms {open} | tick µs mean {mean:.0} p50 {} p99 {} max {} ({} ticks) | frame max {frame_max:.0} ms | input missed {missed} ticks (worst player {missed_max}) | out {:.0} B/s ({:.0} per player), {packets:.0} packets/s | cpu {}% mem {:.0} MB",
        pct(0.5),
        pct(0.99),
        us.last().copied().unwrap_or(0),
        us.len(),
        bytes,
        bytes / players.max(1) as f64,
        if cpu.is_finite() {
            format!("{cpu:.1}")
        } else {
            "—".into()
        },
        mem,
    );
}

fn exit_after(opts: Res<Opts>, time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>) {
    if opts.exit_after.is_some_and(|s| time.elapsed_secs_f64() >= s) {
        exit.write(AppExit::Success);
    }
}
