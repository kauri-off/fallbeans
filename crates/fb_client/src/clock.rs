//! How far ahead of the server the client's clock runs. Lightyear aims at its estimate of the server's tick
//! (the last tick heard + RTT/2) + 4 × jitter + margin, and jumps the clock (relabelling inputs already sent)
//! when it is more than `max_error_margin` off. Both the estimate and the jitter follow every burst up and
//! down within a second, so on a jerky path (TCP in a VPN) the aim swung by tens of ticks and the clock
//! jumped in pairs, back and forth. Here the aim rises at once but comes down slowly, errors short of a
//! large jump only pace the clock, and WebSocket gets a bigger margin.
use bevy::prelude::*;
use fb_net::TICK;
use lightyear::core::time::TickDelta;
use lightyear::prelude::client::{InputDelayConfig, InputTimelineConfig, RemoteTimeline};
use lightyear::prelude::*;
use lightyear_sync::timeline::sync::SyncTargetTimeline;

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

/// The aim over the remote timeline's own clock (its estimate's offset + RTT/2 + JITTER_K × jitter) as
/// held, ticks; None before the first estimate. Offsets count from the first one of the connection (the
/// remote clock starts at 0, the server's tick may be in the millions: too far out for f32).
#[derive(Resource, Default)]
pub struct Lead {
    pub held: Option<f32>,
    base: Option<TickDelta>,
    /// The margin the config carries now (fixed part plus what the held aim adds).
    pub margin: f32,
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
    links: Query<(&PingManager, &RemoteTimeline), (With<Client>, With<Connected>)>,
    mut lead: ResMut<Lead>,
) {
    let Ok((ping, remote)) = links.single() else {
        (lead.held, lead.base) = (None, None);
        return;
    };
    if ping.latency_samples_recv() < 3 || !remote.is_initialized() {
        return;
    }
    let tick = TICK.as_secs_f32();
    let offset = remote.current_estimate() - remote.now();
    let offset = (offset - *lead.base.get_or_insert(offset)).to_f32();
    let half = ping.rtt().as_secs_f32() / 2.0 / tick;
    let want = offset + half + JITTER_K * ping.jitter().as_secs_f32() / tick;
    let held = match lead.held {
        Some(h) if h > want => (h - FALL_PER_S * time.delta_secs()).max(want),
        _ => want,
    };
    lead.held = Some(held);
    let ws = conn.is_some_and(|c| c.transport == Transport::Ws);
    // Lightyear adds its estimate (offset + RTT/2) itself.
    let margin = opts.input_margin + if ws { WS_MARGIN } else { 0.0 } + held - offset - half;
    if (margin - lead.margin).abs() >= STEP {
        lead.margin = margin;
        insert(&mut commands, &opts, margin);
    }
}
