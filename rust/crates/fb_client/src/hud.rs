//! A debug overlay: what phase 0 measures (RTT, rollbacks, transport, map hash, fps).
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use fb_net::*;
use lightyear::prelude::*;

use crate::game::{Map, Stats};
use crate::net::Conn;

#[derive(Component)]
struct HudText;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        app.add_systems(Startup, setup);
        app.add_systems(Update, update.run_if(on_timer(core::time::Duration::from_millis(250))));
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        HudText,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            left: px(10),
            top: px(8),
            ..default()
        },
    ));
}

/// One line per second in the log too, so headless runs can be measured.
#[derive(Default)]
struct LogEvery(f32);

fn update(
    mut text: Query<&mut Text, With<HudText>>,
    conn: Option<Res<Conn>>,
    map: Option<Res<Map>>,
    stats: Res<Stats>,
    metrics: Option<Res<PredictionMetrics>>,
    pings: Query<&PingManager>,
    diag: Res<DiagnosticsStore>,
    timeline: Res<LocalTimeline>,
    own: Query<&BodyFull, With<Predicted>>,
    others: Query<(), With<Interpolated>>,
    time: Res<Time>,
    mut last: Local<LogEvery>,
) {
    let Ok(mut text) = text.single_mut() else { return };
    let fps = diag
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let rtt = pings.iter().next().map_or(0.0, |p| p.rtt().as_secs_f32() * 1000.0);
    let (rollbacks, rb_ticks) = metrics.map_or((0, 0), |m| (m.rollbacks, m.rollback_ticks));
    let conn_s = conn.map_or("—".to_string(), |c| {
        format!("{:?} {} id {}", c.transport, if c.connected { "connected" } else { "connecting…" }, c.id)
    });
    let (round_s, t) = map.as_ref().map_or(("no round".to_string(), 0.0), |m| {
        let ok = if m.static_hash == m.round.static_hash { "ok" } else { "MISMATCH" };
        (
            format!("{} seed {} round {} | map hash {} {ok}", m.round.map, m.round.seed, m.round.number, m.static_hash),
            m.time(timeline.tick().0 as f64),
        )
    });
    let body = own.single().ok().map(|f| &f.body);
    let body_s = body.map_or("no bean yet".to_string(), |b| {
        format!(
            "pos {:.2} {:.2} {:.2} | {:?}{}",
            b.pos.x,
            b.pos.y,
            b.pos.z,
            b.state,
            if b.power != 0 { format!(" | bonus {}", b.power) } else { String::new() }
        )
    });
    text.0 = format!(
        "{conn_s} | rtt {rtt:.0} ms | {fps:.0} fps\n{round_s}\nt {t:.2} s | tick {} | others {}\n{body_s}\nrollbacks {rollbacks} ({rb_ticks} ticks) | map events {}",
        timeline.tick().0,
        others.iter().count(),
        stats.bonus_events,
    );
    if time.elapsed_secs() - last.0 >= 1.0 {
        last.0 = time.elapsed_secs();
        info!(
            "stats: rtt {rtt:.0} ms, fps {fps:.0}, rollbacks {rollbacks}, rollback ticks {rb_ticks}, others {}, events {}, t {t:.1}",
            others.iter().count(),
            stats.bonus_events
        );
    }
}
