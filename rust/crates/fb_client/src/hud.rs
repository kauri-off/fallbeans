//! A debug overlay (RTT, rollbacks, transport, map hash, fps) and, until the HUD of Phase 5, one line of
//! the player's status in the round.
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use fb_arena::{ArenaKind, client_hud};
use fb_net::*;
use lightyear::prelude::*;

use crate::game::{Map, Stats};
use crate::net::Conn;
use crate::session::Session;
use crate::view::Spectate;

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct StatusText;

#[derive(Component)]
struct HintText;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        app.add_systems(Startup, setup);
        app.add_systems(
            Update,
            (update, status, hint).run_if(on_timer(core::time::Duration::from_millis(250))),
        );
    }
}

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
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
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            top: px(16),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child((
            StatusText,
            Text::new(""),
            TextFont {
                // The game's font (Latin and Cyrillic): Bevy's built-in one has no Cyrillic.
                font: assets.load("fonts/Nunito-Black.ttf").into(),
                font_size: FontSize::Px(24.0),
                ..default()
            },
            TextLayout::justify(Justify::Center),
            TextColor(Color::WHITE),
            TextShadow::default(),
        ));
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            bottom: px(18),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_child((
            HintText,
            Text::new(""),
            TextFont {
                font: assets.load("fonts/Nunito-Bold.ttf").into(),
                font_size: FontSize::Px(17.0),
                ..default()
            },
            TextLayout::justify(Justify::Center),
            TextColor(Color::srgb(1.0, 0.93, 0.7)),
            TextShadow::default(),
        ));
}

fn hint(mut text: Query<&mut Text, With<HintText>>, conn: Option<Res<Conn>>, time: Res<Time<Real>>) {
    let Ok(mut text) = text.single_mut() else { return };
    let line = conn
        .and_then(|c| crate::diag::vpn_hint(&c, time.elapsed_secs()))
        .unwrap_or("");
    if text.0 != line {
        text.0 = line.into();
    }
}

fn status(
    mut text: Query<&mut Text, With<StatusText>>,
    map: Option<ResMut<Map>>,
    session: Res<Session>,
    spectate: Option<Res<Spectate>>,
    own: Query<(), (With<Predicted>, With<PlayerId>)>,
) {
    let Ok(mut text) = text.single_mut() else { return };
    let line = match map {
        Some(mut map) if map.round.kind == ArenaKind::Round => {
            let map = &mut *map;
            let me = session.me;
            let name = |id: u32| {
                session
                    .lobby
                    .as_ref()
                    .and_then(|l| l.players.iter().find(|p| p.id == id))
                    .map_or_else(|| format!("#{id}"), |p| p.name.clone())
            };
            if !own.is_empty() {
                let mut scores = session.scores.clone();
                client_hud(&mut map.world, &map.spec, &mut scores, me).unwrap_or_default()
            } else {
                let mine = me.and_then(|me| {
                    let place = map.info.finished.iter().position(|id| *id == me);
                    match place {
                        Some(i) => Some(format!("Финиш! Место: {}", i + 1)),
                        None => map.info.out.contains(&me).then(|| "Вы выбыли".to_string()),
                    }
                });
                let camera = match spectate.and_then(|s| s.target) {
                    Some(id) => format!("Камера: {}", name(id)),
                    None => "Камера: обзор арены".to_string(),
                };
                let lines: Vec<String> = mine
                    .into_iter()
                    .chain([format!("{camera} · A / D или крестовина — другой игрок")])
                    .collect();
                lines.join("\n")
            }
        }
        _ => String::new(),
    };
    if text.0 != line {
        text.0 = line;
    }
}

fn update(
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
) {
    let Ok(mut text) = text.single_mut() else { return };
    let fps = diag
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
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
        "{net_s} | {fps:.0} fps\n{round_s}\nt {t:.2} s | tick {} | others {}\n{body_s}\nrollbacks {rollbacks} ({rb_ticks} ticks) | map events {}",
        timeline.tick().0,
        others.iter().count(),
        stats.map_events,
    );
}
