//! What stress runs measure on the server, logged as one `metrics:` line every `--metrics-every` s:
//! room tick cost, the longest frame (a stalled server reads inputs late), ticks run without a player's
//! input in time, traffic out, process CPU and memory.
use std::time::Instant;

use bevy::diagnostic::{DiagnosticsStore, SystemInformationDiagnosticsPlugin};
use bevy::prelude::*;
use fb_net::NetStats;

use crate::opts::Opts;
use crate::room::{InputState, Pawn, RoomTick};

pub struct MetricsPlugin;

impl Plugin for MetricsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TickTimes>();
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
}

fn track_frames(time: Res<Time<Real>>, mut t: ResMut<TickTimes>) {
    t.frame_max = t.frame_max.max(time.delta_secs_f64());
}

fn start_tick(mut t: ResMut<TickTimes>) {
    t.started = Some(Instant::now());
}

fn end_tick(mut t: ResMut<TickTimes>) {
    if let Some(s) = t.started.take() {
        t.us.push(s.elapsed().as_micros().min(u32::MAX as u128) as u32);
    }
}

fn report(
    opts: Res<Opts>,
    time: Res<Time<Real>>,
    mut t: ResMut<TickTimes>,
    stats: Res<NetStats>,
    diag: Res<DiagnosticsStore>,
    mut pawns: Query<&mut InputState, With<Pawn>>,
) {
    let now = time.elapsed_secs_f64();
    let span = now - t.last_report;
    if opts.metrics_every <= 0.0 || span < opts.metrics_every {
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
    let mean = us.iter().map(|&u| u as f64).sum::<f64>() / us.len().max(1) as f64;
    let bytes = (stats.bytes_out - t.last_bytes) as f64 / span;
    let packets = (stats.packets_out - t.last_packets) as f64 / span;
    (t.last_bytes, t.last_packets) = (stats.bytes_out, stats.packets_out);
    let players = pawns.iter().count();
    let (mut missed, mut missed_max) = (0, 0);
    for mut st in &mut pawns {
        missed += st.missed;
        missed_max = missed_max.max(core::mem::take(&mut st.missed));
    }
    let frame_max = core::mem::take(&mut t.frame_max) * 1000.0;
    let value = |p| diag.get(p).and_then(|d| d.smoothed()).unwrap_or(f64::NAN);
    info!(
        "metrics: players {players} | tick µs mean {mean:.0} p50 {} p99 {} max {} ({} ticks) | frame max {frame_max:.0} ms | input missed {missed} ticks (worst player {missed_max}) | out {:.0} B/s ({:.0} per player), {packets:.0} packets/s | cpu {:.1}% mem {:.0} MB",
        pct(0.5),
        pct(0.99),
        us.last().copied().unwrap_or(0),
        us.len(),
        bytes,
        bytes / players.max(1) as f64,
        value(&SystemInformationDiagnosticsPlugin::PROCESS_CPU_USAGE),
        value(&SystemInformationDiagnosticsPlugin::PROCESS_MEM_USAGE) * 1024.0,
    );
}

fn exit_after(opts: Res<Opts>, time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>) {
    if opts.exit_after.is_some_and(|s| time.elapsed_secs_f64() >= s) {
        exit.write(AppExit::Success);
    }
}
