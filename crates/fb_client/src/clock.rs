//! How far ahead of the server the client's clock runs. The aim rises at once but falls slowly, so a jerky path does
//! not make the clock jump back and forth.
use bevy::prelude::*;
use fb_net::TICK;
use lightyear::core::time::TickDelta;
use lightyear::prelude::client::{InputDelayConfig, InputTimelineConfig, RemoteTimeline};
use lightyear::prelude::*;
use lightyear_sync::timeline::sync::SyncTargetTimeline;

use crate::game::Linked;
use crate::net::Conn;
use crate::opts::{Opts, Transport};

/// Lightyear's jitter multiple (`SyncConfig::jitter_multiple`), kept for the held aim.
const JITTER_K: f32 = 4.0;
/// How fast the held aim comes down once the path is calm again, ticks a second.
const FALL_PER_S: f32 = 1.0;
/// Extra ticks of input lead over WebSocket: TCP delivers in bursts after any stall.
const WS_MARGIN: f32 = 2.0;
/// Errors up to this are corrected by pacing the clock; larger ones jump it (Lightyear: 10).
const MAX_ERROR: f32 = 24.0;
/// Pacing at full error: up to ±2 × (factor − 1) of normal speed (Lightyear: 1.05).
const SPEEDUP: f32 = 1.1;
/// Changes of the margin smaller than this leave the config alone.
const STEP: f32 = 0.1;
/// A frame this long (s) stalls the clocks: its burst skews the estimate, so the offset may not raise the aim.
const STALL: f32 = 0.1;
const STALL_QUIET: f32 = 5.0;
/// Most ticks an input may lead the server's tick: Lightyear drops inputs more than 64 ticks ahead.
const MAX_AHEAD: f32 = (fb_net::INPUT_RING - 4) as f32;
/// Above this margin a late press can be lost: the server's input buffer is a ring of 64 ticks ending at
/// the newest input, and it must still hold the ticks up to LATE_TICKS behind the server's.
const LATE_LEAD: f32 = (fb_net::INPUT_RING - 1 - fb_net::LATE_TICKS) as f32;
/// How often the warning about a margin above LATE_LEAD may repeat, s.
const WARN_EVERY_S: f32 = 30.0;

/// The held aim on the remote timeline, ticks; offsets are from the connection's first estimate (f32 precision).
#[derive(Resource, Default)]
pub struct Lead {
    pub held: Option<f32>,
    base: Option<TickDelta>,
    /// The margin the config carries now (fixed part plus what the held aim adds).
    pub margin: f32,
    /// Real time until which the offset does not raise the held aim (after a long frame), and the offset
    /// from before that frame.
    quiet_until: f32,
    quiet_offset: f32,
    last_offset: Option<f32>,
    /// When the margin was last said to be above LATE_LEAD.
    warned: Option<f32>,
}

pub struct ClockPlugin;

impl Plugin for ClockPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Lead>();
        app.add_systems(Startup, configure);
        app.add_systems(Update, steer);
    }
}

fn insert(commands: &mut Commands, opts: &Opts, margin: f32) {
    let sync = SyncConfig {
        jitter_multiple: 0,
        jitter_margin: margin,
        max_error_margin: opts.sync_max_error.unwrap_or(MAX_ERROR),
        speedup_factor: SPEEDUP,
        ..default()
    };
    commands.insert_resource(
        InputTimelineConfig::default()
            .with_sync_config(sync)
            .with_input_delay(InputDelayConfig::no_input_delay()),
    );
}

fn configure(mut commands: Commands, opts: Res<Opts>, mut lead: ResMut<Lead>) {
    // Until the first estimate: Lightyear's own aim, a jitter part included.
    lead.margin = opts.input_margin + JITTER_K;
    insert(&mut commands, &opts, lead.margin);
}

fn steer(
    mut commands: Commands,
    opts: Res<Opts>,
    time: Res<Time<Real>>,
    conn: Option<Res<Conn>>,
    links: Query<(&PingManager, &RemoteTimeline), Linked>,
    mut lead: ResMut<Lead>,
) {
    let Ok((ping, remote)) = links.single() else {
        (lead.held, lead.base, lead.last_offset) = (None, None, None);
        return;
    };
    if ping.latency_samples_recv() < 3 || !remote.is_initialized() {
        return;
    }
    let now = time.elapsed_secs();
    let tick = TICK.as_secs_f32();
    let offset = remote.current_estimate() - remote.now();
    let offset = (offset - *lead.base.get_or_insert(offset)).to_f32();
    if time.delta_secs() > STALL {
        // (The offset of this frame may have jumped already: the one before it.)
        let before = lead.last_offset.unwrap_or(offset);
        lead.quiet_offset = if now < lead.quiet_until {
            lead.quiet_offset.min(before)
        } else {
            before
        };
        lead.quiet_until = now + STALL_QUIET;
    }
    lead.last_offset = Some(offset);
    let half = ping.rtt().as_secs_f32() / 2.0 / tick;
    let jitter = JITTER_K * ping.jitter().as_secs_f32() / tick;
    let want = offset + half + jitter;
    let held = match lead.held {
        Some(h) if h > want => (h - FALL_PER_S * time.delta_secs()).max(want),
        // After a long frame only RTT and jitter raise it.
        Some(h) if now < lead.quiet_until => h.max(offset.min(lead.quiet_offset) + half + jitter),
        _ => want,
    };
    lead.held = Some(held);
    let ws = conn.is_some_and(|c| c.transport == Transport::Ws);
    // Lightyear adds its estimate (offset + RTT/2) itself.
    // (Held through a stall while the offset jumps, it could go below zero: Lightyear panics on that.)
    let max_error = opts.sync_max_error.unwrap_or(MAX_ERROR);
    let want_margin = opts.input_margin + if ws { WS_MARGIN } else { 0.0 } + held - offset - half;
    let margin = want_margin.min(MAX_AHEAD - max_error).max(0.0);
    if margin > LATE_LEAD && lead.warned.is_none_or(|t| now - t >= WARN_EVERY_S) {
        lead.warned = Some(now);
        warn!(
            "input lead {margin:.1} ticks (wanted {want_margin:.1}; RTT {:.0} ms, jitter {:.0} ms): above {LATE_LEAD} a late press may be lost",
            ping.rtt().as_secs_f32() * 1000.0,
            ping.jitter().as_secs_f32() * 1000.0
        );
    }
    if (margin - lead.margin).abs() >= STEP {
        lead.margin = margin;
        insert(&mut commands, &opts, margin);
    }
}
