//! The debug overlay (F3): RTT, rollbacks, transport, map hash, fps.
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use fb_net::*;
use lightyear::prelude::*;

use crate::game::{MapNow, PredictionStats};
use crate::net::Connection;

#[derive(Component)]
struct HudText;

/// F3: the network and performance overlay is up.
#[derive(Resource, Default)]
pub struct Overlay(pub bool);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        app.init_resource::<Overlay>();
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
fn toggle(keys: Res<ButtonInput<KeyCode>>, mut on: ResMut<Overlay>, mut q: Query<&mut Visibility, With<HudText>>) {
    if keys.just_pressed(KeyCode::F3) {
        on.0 ^= true;
    }
    for mut v in &mut q {
        v.set_if_neq(if on.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// What the overlay shows.
#[derive(SystemParam)]
struct Shown<'w, 's> {
    connection: Connection<'w, 's>,
    net: Res<'w, crate::diag::NetDiag>,
    now: MapNow<'w>,
    counts: PredictionStats<'w>,
    own: Query<'w, 's, &'static BodyFull, With<Predicted>>,
    others: Query<'w, 's, (), With<Interpolated>>,
    diag: Res<'w, DiagnosticsStore>,
    perf: Option<Res<'w, crate::perf::Perf>>,
}

fn update(on: Res<Overlay>, mut text: Query<&mut Text, With<HudText>>, shown: Shown, time: Res<Time<Real>>) {
    let Ok(mut text) = text.single_mut() else { return };
    if !on.0 {
        return;
    }
    // (As F4 counts it: the frames of the last 2 s over their time. Bevy's smoothed rate, which differs, only
    // without the perf plugin.)
    let fps = shown
        .perf
        .and_then(|p| p.fps(2.0))
        .map(f64::from)
        .or_else(|| {
            shown
                .diag
                .get(&FrameTimeDiagnosticsPlugin::FPS)
                .and_then(|d| d.smoothed())
        })
        .unwrap_or(0.0);
    let (rollbacks, rb_ticks) = shown.counts.metrics.map_or((0, 0), |m| (m.rollbacks, m.rollback_ticks));
    let link = shown.connection.link();
    let net_s = crate::diag::summary(shown.connection.conn.as_deref(), link, &shown.net, time.elapsed_secs());
    let (round_s, t) = shown.now.map.as_ref().map_or(("no round".to_string(), 0.0), |m| {
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
            m.time(f64::from(shown.now.timeline.tick().0)),
        )
    });
    let body_s = shown
        .own
        .single()
        .ok()
        .map(|f| &f.body)
        .map_or("no bean yet".to_string(), |b| {
            let bonus = if let Some(p) = b.power {
                format!(" | bonus {p:?}")
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
        shown.now.timeline.tick().0,
        shown.others.iter().count(),
        shown.counts.stats.map_events,
    );
}
