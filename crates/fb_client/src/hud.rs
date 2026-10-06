//! The debug overlay (F3): RTT, rollbacks, transport, map hash, fps.
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use fb_net::*;
use lightyear::prelude::*;

use crate::game::{Map, Stats};
use crate::net::Conn;
use crate::ui::Ui;

#[derive(Component)]
struct HudText;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        app.add_systems(Startup, setup);
        app.add_systems(
            Update,
            (toggle, update.run_if(on_timer(core::time::Duration::from_millis(250)))),
        );
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        HudText,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow::default(),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        Node {
            position_type: PositionType::Absolute,
            left: px(10),
            bottom: px(8),
            padding: UiRect::all(px(6)),
            ..default()
        },
        GlobalZIndex(100),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
}

/// F3 shows the overlay or hides it.
fn toggle(keys: Res<ButtonInput<KeyCode>>, mut ui: ResMut<Ui>, mut q: Query<&mut Visibility, With<HudText>>) {
    if keys.just_pressed(KeyCode::F3) {
        ui.debug ^= true;
    }
    for mut v in &mut q {
        v.set_if_neq(if ui.debug {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

fn update(
    ui: Res<Ui>,
    mut text: Query<&mut Text, With<HudText>>,
    conn: Option<Res<Conn>>,
    map: Option<Res<Map>>,
    stats: Res<Stats>,
    metrics: Option<Res<PredictionMetrics>>,
    links: Query<&Link>,
    diag: Res<DiagnosticsStore>,
    timeline: Res<LocalTimeline>,
    own: Query<&BodyFull, With<Predicted>>,
    others: Query<(), With<Interpolated>>,
    net: Res<crate::diag::NetDiag>,
    time: Res<Time<Real>>,
    perf: Option<Res<crate::perf::Perf>>,
) {
    let Ok(mut text) = text.single_mut() else { return };
    if !ui.debug {
        return;
    }
    // (As F4 counts it: the frames of the last 2 s over their time. Bevy's smoothed rate, which differs, only
    // without the perf plugin.)
    let fps = perf
        .and_then(|p| p.fps(2.0))
        .map(f64::from)
        .or_else(|| diag.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()))
        .unwrap_or(0.0);
    let (rollbacks, rb_ticks) = metrics.map_or((0, 0), |m| (m.rollbacks, m.rollback_ticks));
    let link = conn.as_ref().and_then(|c| c.entity).and_then(|e| links.get(e).ok());
    let net_s = crate::diag::summary(conn.as_deref(), link, &net, time.elapsed_secs());
    let (round_s, t) = map.as_ref().map_or(("no round".to_string(), 0.0), |m| {
        let ok = if m.static_hash == m.round.static_hash {
            "ok"
        } else {
            "MISMATCH"
        };
        (
            format!(
                "{} seed {} arena {} | map hash {} {ok}",
                m.round.map, m.round.seed, m.round.arena, m.static_hash
            ),
            m.time(timeline.tick().0 as f64),
        )
    });
    let body_s = own
        .single()
        .ok()
        .map(|f| &f.body)
        .map_or("no bean yet".to_string(), |b| {
            let bonus = if b.power != 0 {
                format!(" | bonus {}", b.power)
            } else {
                String::new()
            };
            format!(
                "pos {:.2} {:.2} {:.2} | {:?}{bonus}",
                b.pos.x, b.pos.y, b.pos.z, b.state
            )
        });
    text.0 = format!(
        "{net_s} | {fps:.0} fps (2 s)\n{round_s}\nt {t:.2} s | tick {} | others {}\n{body_s}\nrollbacks {rollbacks} ({rb_ticks} ticks) | map events {}",
        timeline.tick().0,
        others.iter().count(),
        stats.map_events,
    );
}
