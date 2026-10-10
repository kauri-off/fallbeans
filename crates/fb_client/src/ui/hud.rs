//! The in-game HUD: standings, feed, intro and countdown, timer, round results and the game's summary.
use std::time::Duration;

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::time::common_conditions::on_real_timer;
use fb_arena::{ArenaKind, client_hud};
use fb_net::*;
use fb_proto::PlayerId;
use fb_shared::DT;
use fb_shared::game::Genre;
use lightyear::prelude::*;

use super::motion::{Motion, Reveal};
use super::*;
use crate::game::Map;
use crate::net::Conn;
use crate::opts::Transport;
use crate::session::{FEED_SECS, Feed, FeedEntry, FeedLog, Outcome, Session};
use crate::view::{FrameClock, Spectate};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hud>();
        app.init_resource::<RoundTime>();
        app.add_systems(Startup, build_hud.after(super::setup));
        app.add_systems(
            Update,
            (
                hud_state,
                (
                    root.run_if(resource_changed::<Hud>),
                    panel.run_if(resource_changed::<Hud>.or_else(resource_changed::<Session>)),
                    net_line.run_if(on_real_timer(Duration::from_millis(250))),
                    // (Lines gone before the log is looked at: neither takes away what the other did.)
                    (expire, feed.run_if(resource_changed::<FeedLog>)).chain(),
                    feed_box.run_if(resource_changed::<Outcome>),
                    intro.run_if(resource_changed::<Session>),
                    clock,
                    results.run_if(resource_changed::<Outcome>.or_else(resource_changed::<Session>)),
                    summary.run_if(resource_changed::<Outcome>.or_else(resource_changed::<Session>)),
                    status.run_if(resource_changed::<Hud>.or_else(resource_changed::<Outcome>)),
                    prompt.run_if(resource_changed::<Hud>.or_else(resource_changed::<Ui>)),
                ),
            )
                .chain(),
        );
    }
}

/// A player in the round: how they are doing, and their finishing place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: PlayerId,
    pub part: Part,
    pub place: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Part {
    #[default]
    Play,
    Finished,
    Out,
    Spectating,
}

/// What the HUD shows of the round, set when it changes.
#[derive(Resource, Default, PartialEq)]
pub struct Hud {
    pub on: bool,
    pub kind: Option<ArenaKind>,
    pub status: Part,
    pub place: usize,
    pub spectating: Option<String>,
    pub map_text: Option<String>,
    pub bonus: Option<String>,
    pub roster: Vec<Entry>,
}

/// The round's clock, every frame.
#[derive(Resource, Default)]
pub struct RoundTime {
    /// Round time, seconds (negative during the intro).
    pub t: f64,
    pub time_left: f64,
    /// The round's length, seconds.
    pub duration: f64,
    /// Seconds until the next scene starts on its own.
    pub next_in: Option<f64>,
}

#[derive(Component)]
struct HudRoot;
#[derive(Component)]
struct PanelBox;
#[derive(Component)]
struct NetLine;
#[derive(Component)]
struct FeedBox;
/// A line of the feed: its entry's number and time.
#[derive(Component)]
struct FeedRow(u32, f32);
#[derive(Component)]
struct IntroBox;
#[derive(Component)]
struct IntroLeft;
#[derive(Component)]
struct TimerBox;
#[derive(Component)]
struct TimerText;
/// The lit part of the bar under the timer: the time left.
#[derive(Component)]
struct TimerFill;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Pill {
    MapText,
    Bonus,
}
#[derive(Component)]
struct GoText;
#[derive(Component)]
struct ResultsBox;
#[derive(Component)]
struct SummaryBox;
/// The time to the next scene, after its label.
#[derive(Component)]
struct NextLine(&'static str);
#[derive(Component)]
struct StatusBox;
#[derive(Component)]
struct PromptBox;
#[derive(Component)]
struct CountText;

/// A node placed across the screen with its content centred (`left: 50%; translateX(-50%)` in CSS).
fn centred(top: Option<Val>, bottom: Option<Val>) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: px(0),
        right: px(0),
        top: top.unwrap_or(Val::Auto),
        bottom: bottom.unwrap_or(Val::Auto),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: rem(0.5),
        ..default()
    }
}

/// Big text in the display face, with a coloured shadow under it.
fn shadowed(size: f32, color: Color, shadow: Color, f: &Fonts) -> impl Bundle {
    (
        Text::new(""),
        TextFont {
            font: f.display.clone().into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        TextShadow {
            offset: Vec2::new(size / 28.0, size / 16.0),
            color: shadow,
        },
    )
}

/// The colour of a place: gold, silver and bronze, then none.
fn medal(place: usize) -> Option<Color> {
    match place {
        1 => Some(GOLD),
        2 => Some(SILVER),
        3 => Some(BRONZE),
        _ => None,
    }
}

/// A place in a round badge, lit in its medal's colour.
fn place_badge(p: &mut ChildSpawnerCommands, f: &Fonts, place: usize, size: f32, lit: bool) {
    let m = medal(place).filter(|_| lit);
    p.spawn((
        Node {
            width: rem(size),
            height: rem(size),
            border_radius: BorderRadius::MAX,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(m.unwrap_or(ink_wash(0.07))),
    ))
    .with_children(|b| {
        rich_in(
            b,
            f,
            &place.to_string(),
            size * 7.0,
            if m.is_some() { INK } else { MUTED },
            true,
        );
    });
}

fn build_hud(mut commands: Commands, layers: Res<Layers>, f: Res<Fonts>) {
    let f = &*f;
    let e = layers[Layer::Hud];
    commands.entity(e).with_children(|l| {
        l.spawn((
            HudRoot,
            Node {
                width: percent(100),
                height: percent(100),
                display: display(false),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|h| {
            // Top left: the standings.
            h.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: rem(1.0),
                    left: rem(1.0),
                    width: rem(17.5),
                    max_height: percent(96),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::new(rem(0.5), rem(0.5), rem(0.75), rem(0.625)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(rem(1.375)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                glass(),
            ))
            .with_children(|p| {
                p.spawn((
                    PanelBox,
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: px(2),
                        ..default()
                    },
                ));
                p.spawn((
                    NetLine,
                    Node {
                        margin: UiRect::top(rem(0.5)),
                        padding: UiRect::new(rem(0.5), px(0), rem(0.5), px(0)),
                        border: UiRect::top(px(1)),
                        ..default()
                    },
                    BorderColor::all(RIM),
                    Text::new(""),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                    TextColor(FAINT),
                ));
            });
            // Top right: the feed.
            h.spawn((
                FeedBox,
                Node {
                    position_type: PositionType::Absolute,
                    top: rem(1.0),
                    right: rem(1.0),
                    max_width: percent(40),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexEnd,
                    row_gap: rem(0.375),
                    ..default()
                },
            ));
            // Top middle: the timer over its bar, the map's line and the bonus in effect.
            h.spawn(centred(Some(rem(1.0)), None)).with_children(|t| {
                t.spawn((
                    TimerBox,
                    Node {
                        min_width: rem(9.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: rem(0.375),
                        padding: UiRect::new(rem(1.25), rem(1.25), rem(0.375), rem(0.625)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(rem(1.125)),
                        display: display(false),
                        ..default()
                    },
                    glass(),
                    Reveal::new(Motion::slide(0.0, -24.0)),
                ))
                .with_children(|b| {
                    b.spawn((TimerText, shadowed(26.0, INK, Color::NONE, f)));
                    b.spawn((
                        Node {
                            width: percent(100),
                            height: rem(0.25),
                            border_radius: BorderRadius::MAX,
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(ink_wash(0.1)),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            TimerFill,
                            Node {
                                width: percent(100),
                                height: percent(100),
                                border_radius: BorderRadius::MAX,
                                ..default()
                            },
                            BackgroundColor(BLUE),
                        ));
                    });
                });
                for pill in [Pill::MapText, Pill::Bonus] {
                    let bonus = pill == Pill::Bonus;
                    t.spawn((
                        pill,
                        Node {
                            padding: UiRect::axes(rem(1.0), rem(0.4375)),
                            border: UiRect::all(px(if bonus { 2 } else { 1 })),
                            border_radius: BorderRadius::MAX,
                            display: display(false),
                            ..default()
                        },
                        BackgroundColor(PANEL),
                        BorderColor::all(if bonus { BLUE } else { RIM }),
                        BoxShadow::new(
                            if bonus {
                                BLUE.with_alpha(0.35)
                            } else {
                                SHADOW.with_alpha(0.4)
                            },
                            px(0),
                            px(4),
                            px(0),
                            px(16),
                        ),
                        Reveal::new(Motion::pop(0.7)),
                    ))
                    .with_children(|p| {
                        rich_in(p, f, "", 15.0, INK, true);
                    });
                }
            });
            h.spawn((ResultsBox, centred(Some(rem(1.0)), None)));
            h.spawn((
                SummaryBox,
                Node {
                    position_type: PositionType::Absolute,
                    top: rem(1.0),
                    right: rem(1.0),
                    width: rem(24.0),
                    max_width: percent(45),
                    max_height: percent(94),
                    ..default()
                },
            ));
            // The middle: 3-2-1 and «ВПЕРЁД!».
            h.spawn(centred(Some(percent(26)), None)).with_children(|c| {
                c.spawn((CountText, shadowed(160.0, INK, Color::WHITE, f)));
                c.spawn((GoText, shadowed(108.0, BLUE, Color::WHITE, f)));
            });
            h.spawn((IntroBox, centred(None, Some(rem(5.0)))));
            h.spawn((StatusBox, centred(None, Some(rem(4.5)))));
            h.spawn(centred(Some(percent(56)), None)).with_children(|c| {
                c.spawn((
                    PromptBox,
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: rem(0.375),
                        padding: UiRect::axes(rem(2.0), rem(1.125)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(rem(1.375)),
                        display: display(false),
                        ..default()
                    },
                    glass(),
                    Reveal::new(Motion::pop(0.85)),
                ))
                .with_children(|c| {
                    rich_in(c, f, text::CLICK_FIELD, 18.0, INK, true);
                    rich(c, f, text::OR_ESC_MENU, 12.5, MUTED);
                });
            });
        });
    });
}

fn hud_state(
    mut hud: ResMut<Hud>,
    mut time: ResMut<RoundTime>,
    map: Option<ResMut<Map>>,
    session: Res<Session>,
    spectate: Option<Res<Spectate>>,
    own: Query<&BodyFull, With<Predicted>>,
    clock: FrameClock,
) {
    let Some(mut map) = map.filter(|_| session.room.is_some()) else {
        hud.set_if_neq(Hud::default());
        return;
    };
    let map = &mut *map;
    let kind = map.round.kind;
    let tick = clock.tick();
    let t = map.time(tick);
    let me = session.me;
    let roster = map
        .info
        .participants
        .iter()
        .map(|id| {
            let place = map.info.finished.iter().position(|f| f == id);
            let part = if place.is_some() {
                Part::Finished
            } else if map.info.out.contains(id) {
                Part::Out
            } else {
                Part::Play
            };
            Entry {
                id: *id,
                part,
                place: place.map(|p| p + 1),
            }
        })
        .collect();
    let body = own.single().ok().map(|f| &f.body);
    let place = me
        .and_then(|me| map.info.finished.iter().position(|f| *f == me))
        .map_or(0, |p| p + 1);
    let status = if body.is_some() {
        Part::Play
    } else if place > 0 {
        Part::Finished
    } else if me.is_some_and(|me| map.info.out.contains(&me)) {
        Part::Out
    } else {
        Part::Spectating
    };
    let bonus = body.and_then(|b| {
        let left = (b.power_until - f64::from(clock.timeline.tick().0) * DT + map.round.zero_tick as f64 * DT).ceil();
        let (icon, title) = text::bonus(b.power?);
        (left > 0.0).then(|| format!("{icon} {title} · {left:.0} с"))
    });
    let map_text = if kind == ArenaKind::Round && t >= 0.0 {
        let mut scores = session.scores.clone();
        client_hud(&mut map.world, &map.spec, &mut scores, me)
    } else {
        None
    };
    let duration = fb_maps::by_id(map.round.map).meta().duration;
    let next_in = session
        .lobby
        .as_ref()
        .and_then(|l| l.next)
        .map(|n| ((f64::from(n) - f64::from(clock.timeline.tick().0)) * DT).max(0.0));
    let spectating = spectate
        .and_then(|s| s.target)
        .filter(|_| status != Part::Play)
        .map(|id| session.name_of(id));
    hud.set_if_neq(Hud {
        on: true,
        kind: Some(kind),
        status,
        place,
        spectating,
        map_text,
        bonus,
        roster,
    });
    *time = RoundTime {
        t,
        time_left: (duration - t).max(0.0),
        duration,
        next_in,
    };
}

fn root(hud: Res<Hud>, mut q: Single<&mut Node, With<HudRoot>>) {
    show(&mut q, hud.on);
}

fn name_tag(p: &mut ChildSpawnerCommands, f: &Fonts, session: &Session, id: PlayerId, size: f32) {
    let (name, color) = match session.player(id) {
        Some(pl) => (pl.name.clone(), Some(pl.color)),
        None => (format!("#{id}"), None),
    };
    p.spawn(Node {
        column_gap: rem(0.375),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|r| {
        dot(r, color.map_or(MUTED, suit), 0.625);
        rich_in(r, f, &name, size, INK, true);
    });
}

/// A genre's tag: its colour as a dot beside the ink (text wears the inks, the dot carries the genre).
fn genre_pill(p: &mut ChildSpawnerCommands, f: &Fonts, g: Genre, s: &str) {
    p.spawn((
        Node {
            column_gap: rem(0.3125),
            align_items: AlignItems::Center,
            padding: UiRect::new(rem(0.375), rem(0.5), px(2), px(2)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::MAX,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(SURFACE),
        BorderColor::all(RIM),
    ))
    .with_children(|t| {
        dot(t, genre_color(g), 0.5);
        rich_in(t, f, s, 11.0, INK, true);
    });
}

/// Top left: the round, the players in their order with their status, score and ping.
fn panel(
    q: Single<Entity, With<PanelBox>>,
    hud: Res<Hud>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Some(info) = session.arena.as_ref().filter(|_| hud.on) else {
        return;
    };
    let round = info.kind == ArenaKind::Round;
    let f = &*f;
    let def = fb_maps::by_id(info.game).meta();
    let mut players = session.lobby.as_ref().map_or_else(Vec::new, |l| l.players.clone());
    if info.kind == ArenaKind::Lobby {
        players.sort_by_key(|p| (core::cmp::Reverse(p.crowns), p.id));
    } else {
        players.sort_by_key(|p| (core::cmp::Reverse(p.score), p.id));
    }
    let host = session.lobby.as_ref().and_then(|l| l.host);
    let genre_points = def.genre == Genre::Points;
    rebuild(&mut commands, *q, |p| {
        p.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: rem(0.25),
            padding: UiRect::new(rem(0.5), rem(0.5), px(0), rem(0.5)),
            ..default()
        })
        .with_children(|h| {
            if round {
                row(h, false, |r| {
                    let s = if info.practice {
                        text::PRACTICE_TITLE.to_string()
                    } else {
                        format!("{}/{}", info.index, info.total)
                    };
                    genre_pill(r, f, def.genre, &s);
                    caption(r, f, text::genre(def.genre));
                });
                big(h, f, def.title, 15.0, INK);
            } else {
                let s = if info.kind == ArenaKind::Podium {
                    text::GAME_SUMMARY.to_string()
                } else {
                    session
                        .lobby
                        .as_ref()
                        .map(|l| l.room.title.clone())
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| text::LOBBY.into())
                };
                if info.kind == ArenaKind::Lobby {
                    caption(h, f, text::LOBBY);
                }
                big(h, f, &s, 15.0, INK);
            }
        });
        for (rank, pl) in players.iter().enumerate() {
            let st = hud.roster.iter().find(|r| r.id == pl.id);
            let icon = if !round {
                String::new()
            } else {
                match st {
                    None => "👁".into(),
                    Some(Entry {
                        part: Part::Finished,
                        place,
                        ..
                    }) => format!("🏁{}", place.unwrap_or(0)),
                    Some(Entry { part: Part::Out, .. }) => "✖".into(),
                    Some(_) => String::new(),
                }
            };
            let dim = st.is_some_and(|s| s.part == Part::Out) || !pl.connected;
            let me = session.me == Some(pl.id);
            p.spawn((
                Node {
                    column_gap: rem(0.5),
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(rem(0.375), rem(0.25)),
                    border: UiRect::left(px(3)),
                    border_radius: BorderRadius::all(rem(0.625)),
                    ..default()
                },
                BackgroundColor(if me { ink_wash(0.08) } else { Color::NONE }),
                BorderColor {
                    left: suit(pl.color).with_alpha(if dim { 0.35 } else { 1.0 }),
                    ..BorderColor::all(Color::NONE)
                },
            ))
            .with_children(|r| {
                place_badge(r, f, rank + 1, 1.25, info.kind != ArenaKind::Lobby);
                r.spawn(Node {
                    flex_grow: 1.0,
                    min_width: px(0),
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|t| {
                    let ink = if dim {
                        FAINT
                    } else if me {
                        BLUE
                    } else {
                        INK
                    };
                    rich_in(t, f, &pl.name, 13.0, ink, true);
                });
                if Some(pl.id) == host {
                    rich(r, f, "⭐", 11.0, INK);
                }
                if info.kind == ArenaKind::Lobby && pl.crowns > 0 {
                    rich_in(r, f, &format!("👑{}", pl.crowns), 11.0, INK, true);
                }
                let bells = session.scores.get(&pl.id).copied().unwrap_or(0);
                if info.kind == ArenaKind::Lobby && bells > 0 {
                    rich(r, f, &format!("🔔{bells}"), 11.0, INK);
                }
                if !icon.is_empty() {
                    rich(r, f, &icon, 11.0, INK);
                }
                if round && genre_points {
                    rich_in(r, f, &bells.to_string(), 12.0, GOOD, true);
                }
                if info.kind != ArenaKind::Lobby {
                    rich_in(r, f, &pl.score.to_string(), 13.5, INK, true);
                }
                let ping = if pl.bot {
                    text::BOT.to_string()
                } else if pl.connected {
                    pl.ping.to_string()
                } else {
                    "—".into()
                };
                rich(r, f, &ping, 10.5, FAINT);
            });
        }
    });
}

/// Under the panel: transport and ping (and frames a second); after a fallback, the VPN hint.
fn net_line(
    mut q: Query<&mut Text, With<NetLine>>,
    hud: Res<Hud>,
    conn: Option<Res<Conn>>,
    links: Query<&Link>,
    display: Res<crate::settings::Display>,
    diag: Res<DiagnosticsStore>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs();
    if !hud.on {
        return;
    }
    let Ok(mut t) = q.single_mut() else { return };
    let Some(conn) = conn else { return };
    let rtt = conn
        .entity
        .and_then(|e| links.get(e).ok())
        .map_or(0.0, |l| l.stats.rtt.as_secs_f32() * 1000.0);
    let transport = if conn.transport == Transport::Ws {
        "WebSocket"
    } else {
        "UDP"
    };
    let mut s = format!("{transport} · {}", text::ping(rtt.round() as u32));
    if display.show_fps {
        let fps = diag
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|d| d.smoothed())
            .unwrap_or(0.0);
        s += &format!(" · {fps:.0} к/с");
    }
    if let Some(hint) = crate::diag::vpn_hint(&conn, now) {
        s += "\n";
        s += hint;
    }
    if t.0 != s {
        t.0 = s;
    }
}

/// Top right: who fell and why; finishes, bonuses and other notes. New entries are added, gone ones taken away.
fn feed(
    q: Single<Entity, With<FeedBox>>,
    log: Res<FeedLog>,
    rows: Query<(Entity, &FeedRow)>,
    session: Res<Session>,
    time: Res<Time<Real>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    let now = time.elapsed_secs();
    let row_of = |x: &FeedEntry| (x.n, x.at.to_bits());
    let mut have = Vec::new();
    for (e, r) in &rows {
        if log.0.iter().any(|x| row_of(x) == (r.0, r.1.to_bits())) {
            have.push((r.0, r.1.to_bits()));
        } else {
            commands.entity(e).despawn();
        }
    }
    commands.entity(*q).with_children(|p| {
        // (Not those `expire` took away already.)
        for x in log
            .0
            .iter()
            .filter(|x| now - x.at < FEED_SECS && !have.contains(&row_of(x)))
        {
            feed_row(p, f, &session, x);
        }
    });
}

fn feed_row(p: &mut ChildSpawnerCommands, f: &Fonts, session: &Session, x: &FeedEntry) {
    let bar = match &x.what {
        Feed::Ko { out: true, .. } => CRITICAL,
        Feed::Ko { .. } => SERIOUS,
        Feed::Note(_) => BLUE,
    };
    p.spawn((
        FeedRow(x.n, x.at),
        Node {
            column_gap: rem(0.5),
            align_items: AlignItems::Center,
            padding: UiRect::new(rem(0.75), rem(0.875), rem(0.375), rem(0.375)),
            border: UiRect::new(px(3), px(1), px(1), px(1)),
            border_radius: BorderRadius::all(rem(0.75)),
            // (A note may carry a long path: an F8 report's.)
            max_width: rem(30.0),
            ..default()
        },
        glass_bar(bar),
        Motion::slide(56.0, 0.0),
    ))
    .with_children(|r| match &x.what {
        Feed::Note(s) => {
            let t = rich(r, f, s, 13.0, INK);
            wrap_anywhere(r, t);
        }
        Feed::Ko {
            victim,
            by,
            cause,
            out,
            shortcut,
        } => {
            let (icon, why) = text::cause(*cause);
            if let Some(by) = by.filter(|b| b != victim) {
                name_tag(r, f, session, by, 13.0);
            }
            rich(r, f, icon, 16.0, INK);
            name_tag(r, f, session, *victim, 13.0);
            let ink = if *out { CRITICAL } else { MUTED };
            rich(r, f, &text::ko_line(why, *out, *shortcut), 12.0, ink);
        }
    });
}

/// The feed gives way to the game's summary.
fn feed_box(outcome: Res<Outcome>, mut q: Single<&mut Node, With<FeedBox>>) {
    show(&mut q, outcome.game_end.is_none());
}

/// A line of the feed goes after a while.
fn expire(rows: Query<(Entity, &FeedRow)>, time: Res<Time<Real>>, mut commands: Commands) {
    let now = time.elapsed_secs();
    for (e, r) in &rows {
        if now - r.1 >= FEED_SECS {
            commands.entity(e).despawn();
        }
    }
}

/// Before the start: the game, its goal, and the time to the start (the camera flies over the course).
fn intro(
    q: Single<Entity, With<IntroBox>>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut shown: Local<Option<u32>>,
    mut commands: Commands,
) {
    let info = session.arena.as_ref().filter(|a| a.kind == ArenaKind::Round);
    if *shown == info.map(|i| i.id) {
        return;
    }
    *shown = info.map(|i| i.id);
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        let Some(info) = info else { return };
        let m = fb_maps::by_id(info.game).meta();
        let g = genre_color(m.genre);
        p.spawn((
            Node {
                width: rem(38.0),
                max_width: percent(70),
                flex_direction: FlexDirection::Column,
                row_gap: rem(0.5),
                padding: UiRect::new(rem(1.5), rem(1.5), rem(1.25), rem(1.375)),
                border: UiRect::new(px(4), px(1), px(1), px(1)),
                border_radius: BorderRadius::all(rem(1.5)),
                ..default()
            },
            glass_bar(g),
            Motion::slide(0.0, 40.0).lasting(0.45),
        ))
        .with_children(|c| {
            row(c, false, |r| {
                genre_pill(r, f, m.genre, text::genre(m.genre));
                if !info.practice {
                    caption(r, f, &text::round_of(info.index, info.total));
                }
                spacer(r);
                let left = rich_in(r, f, "", 14.0, BLUE, true);
                r.commands().entity(left).insert(IntroLeft);
            });
            big(c, f, m.title, 36.0, INK);
            rich_in(c, f, &format!("🎯 {}.", m.goal), 17.0, INK, true);
            muted(c, f, m.desc);
        });
    });
}

/// The HUD's state, the round's time, and the outcome (shown between rounds).
#[derive(SystemParam)]
struct RoundView<'w> {
    hud: Res<'w, Hud>,
    time: Res<'w, RoundTime>,
    outcome: Res<'w, Outcome>,
}

impl RoundView<'_> {
    /// Between rounds: the results or the game's end are up.
    fn between(&self) -> bool {
        self.outcome.results.is_some() || self.outcome.game_end.is_some()
    }
}

type Only<T, A, B> = (With<T>, Without<A>, Without<B>);
/// The nodes of `T`, apart from those of the three other kinds `clock` sets.
type NodeOf<T, A, B, C> = (With<T>, Without<A>, Without<B>, Without<C>);

type PillNode = (Entity, &'static Pill, &'static mut Node);

/// The intro, the timer and its bar, and the «ВПЕРЁД!» and 3-2-1 texts.
#[derive(SystemParam)]
struct Countdown<'w, 's> {
    intro: Single<'w, 's, &'static mut Node, NodeOf<IntroBox, Pill, TimerBox, TimerFill>>,
    timer_box: Single<'w, 's, (Entity, &'static mut Node), NodeOf<TimerBox, IntroBox, Pill, TimerFill>>,
    fill: Single<'w, 's, &'static mut Node, NodeOf<TimerFill, IntroBox, Pill, TimerBox>>,
    timer: Single<'w, 's, (&'static mut Text, &'static mut TextColor), Only<TimerText, GoText, CountText>>,
    go: Single<'w, 's, (Entity, &'static mut Text), Only<GoText, TimerText, CountText>>,
    count: Single<'w, 's, (Entity, &'static mut Text, &'static mut TextColor), Only<CountText, TimerText, GoText>>,
}

/// Every frame: the intro and its time to the start, the timer and its bar, the map's line, the bonus in effect,
/// «ВПЕРЁД!», 3-2-1, and the time to the next scene.
fn clock(
    round: RoundView,
    mut countdown: Countdown,
    mut pills: Query<PillNode, NodeOf<Pill, IntroBox, TimerBox, TimerFill>>,
    mut leaves: Query<(&mut Rich, Option<&IntroLeft>, Option<&NextLine>)>,
    children: Query<&Children>,
    mut ticked: Local<i64>,
    mut commands: Commands,
) {
    let between = round.between();
    let in_round = round.hud.on && round.hud.kind == Some(ArenaKind::Round) && !between;
    let before = in_round && round.time.t < 0.0;
    let on = in_round && round.time.t >= 0.0;
    show(&mut countdown.intro, before);
    let (box_e, ref mut box_node) = *countdown.timer_box;
    show(box_node, on);
    let left = round.time.time_left;
    let width = percent((left / round.time.duration.max(1.0) * 100.0) as f32);
    if countdown.fill.width != width {
        countdown.fill.width = width;
    }
    let (ref mut t, ref mut c) = *countdown.timer;
    let s = if on { text::fmt_time(left) } else { String::new() };
    if t.0 != s {
        t.0 = s;
    }
    let hurry = on && left < 10.0;
    c.set_if_neq(TextColor(if hurry { CRITICAL } else { INK }));
    // (Each of the last ten seconds beats.)
    let sec = left.ceil() as i64;
    if hurry && sec != *ticked && sec > 0 {
        commands.entity(box_e).insert(Motion::pop(1.12).lasting(0.3));
    }
    *ticked = sec;
    let (go_e, ref mut go) = *countdown.go;
    let s = if on && round.time.t < 1.2 { text::GO } else { "" };
    if go.0 != s {
        if !s.is_empty() {
            commands.entity(go_e).insert(Motion::pop(0.3).lasting(0.5));
        }
        go.0 = s.into();
    }
    let n = (-round.time.t).ceil() as i64;
    let s = if round.hud.on && round.hud.kind == Some(ArenaKind::Round) && (1..=3).contains(&n) {
        n.to_string()
    } else {
        String::new()
    };
    let (count_e, ref mut count, ref mut ink) = *countdown.count;
    if count.0 != s {
        if !s.is_empty() {
            commands.entity(count_e).insert(Motion::pop(1.9).lasting(0.45));
        }
        count.0 = s;
    }
    ink.set_if_neq(TextColor(match n {
        3 => CRITICAL,
        2 => WARNING,
        _ => GREEN_DEEP,
    }));
    for (e, pill, mut node) in &mut pills {
        let s = match pill {
            _ if !on => None,
            Pill::MapText => round.hud.map_text.as_deref(),
            Pill::Bonus => round.hud.bonus.as_deref(),
        };
        show(&mut node, s.is_some());
        if let Some(s) = s
            && let Some(c) = children.iter_descendants(e).find(|c| leaves.contains(*c))
            && let Ok((mut t, ..)) = leaves.get_mut(c)
        {
            t.set(s);
        }
    }
    for (mut t, intro, next) in &mut leaves {
        if intro.is_some() && before {
            t.set(&text::in_secs(text::TO_START, -round.time.t));
        }
        if let Some(n) = next {
            t.set(&round.time.next_in.map(|s| text::in_secs(n.0, s)).unwrap_or_default());
        }
    }
}

fn next_line(p: &mut ChildSpawnerCommands, f: &Fonts, label_s: &'static str) {
    let e = rich_in(p, f, "", 13.0, BLUE, true);
    p.commands().entity(e).insert(NextLine(label_s));
}

/// A board's head, lit by a gradient: its caption, its title, and the time to the next scene.
fn board_head(p: &mut ChildSpawnerCommands, f: &Fonts, cap: &str, title: &str, next: &'static str) {
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: rem(0.25),
            padding: UiRect::new(rem(1.375), rem(1.375), rem(1.125), rem(1.0)),
            ..default()
        },
        BackgroundGradient::from(LinearGradient::to_right(vec![
            BLUE_SOFT.into(),
            BLUE_SOFT.with_alpha(0.0).into(),
        ])),
    ))
    .with_children(|h| {
        rich_in(h, f, &cap.to_uppercase(), 11.0, MUTED, true);
        big(h, f, title, 24.0, INK);
        next_line(h, f, next);
    });
}

/// After a round: points won and lost, on a board at the top.
fn results(
    q: Single<Entity, With<ResultsBox>>,
    outcome: Res<Outcome>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        let Some(r) = &outcome.results else { return };
        let title = fb_maps::by_id(r.game).meta().title;
        let next = if r.practice {
            text::AGAIN
        } else if r.index < r.total {
            text::NEXT_ROUND
        } else {
            text::GAME_RESULTS_IN
        };
        p.spawn((
            Node {
                width: rem(36.0),
                max_width: percent(70),
                flex_direction: FlexDirection::Column,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(rem(1.5)),
                overflow: Overflow::clip(),
                ..default()
            },
            glass(),
            Motion::slide(0.0, -32.0).lasting(0.4),
        ))
        .with_children(|c| {
            let cap = if r.practice {
                text::PRACTICE_SMALL.to_string()
            } else {
                text::results_of(r.index, r.total)
            };
            board_head(c, f, &cap, title, next);
            c.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                padding: UiRect::all(rem(0.625)),
                ..default()
            })
            .with_children(|t| {
                for (i, row_) in r.rows.iter().enumerate() {
                    let me = session.me == Some(row_.id);
                    let e = table_row(t, me, |t| {
                        place_badge(t, f, row_.place, 1.5, true);
                        t.spawn(Node {
                            flex_grow: 1.0,
                            min_width: px(0),
                            flex_direction: FlexDirection::Column,
                            overflow: Overflow::clip(),
                            ..default()
                        })
                        .with_children(|x| {
                            name_tag(x, f, &session, row_.id, 14.0);
                            rich(x, f, &text::round_note(row_.note), 11.5, MUTED);
                        });
                        cell(t, 2.25, |x| {
                            rich_in(x, f, &format!("+{}", row_.points), 13.0, GOOD, true);
                        });
                        cell(t, 2.0, |x| {
                            if row_.penalty > 0 {
                                rich_in(x, f, &format!("−{}", row_.penalty), 13.0, CRITICAL, true);
                            }
                        });
                        cell(t, 2.75, |x| {
                            let (fill, ink) = match row_.delta {
                                d if d > 0 => (GREEN_DEEP.with_alpha(0.14), GOOD),
                                d if d < 0 => (CRITICAL.with_alpha(0.12), CRITICAL),
                                _ => (ink_wash(0.07), MUTED),
                            };
                            badge(x, f, &text::signed(row_.delta), fill, ink);
                        });
                        cell(t, 2.5, |x| {
                            rich_in(x, f, &row_.total.to_string(), 16.0, INK, true);
                        });
                    });
                    t.commands()
                        .entity(e)
                        .insert(Motion::slide(-24.0, 0.0).after(0.15 + i as f32 * 0.05));
                }
            });
        });
    });
}

fn table_row(p: &mut ChildSpawnerCommands, me: bool, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn((
        Node {
            column_gap: rem(0.625),
            align_items: AlignItems::Center,
            padding: UiRect::axes(rem(0.625), rem(0.375)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(0.75)),
            ..default()
        },
        BackgroundColor(if me { BLUE.with_alpha(0.08) } else { ink_wash(0.03) }),
        BorderColor::all(if me { BLUE.with_alpha(0.45) } else { Color::NONE }),
    ))
    .with_children(f)
    .id()
}

fn cell(p: &mut ChildSpawnerCommands, w: f32, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        width: rem(w),
        justify_content: JustifyContent::FlexEnd,
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(f);
}

/// A step of the podium: the player over a block as high as their place deserves.
fn podium_step(p: &mut ChildSpawnerCommands, f: &Fonts, s: &fb_proto::Standing) {
    let (high, size) = match s.place {
        1 => (5.5, 3.25),
        2 => (4.0, 2.75),
        _ => (3.0, 2.5),
    };
    let m = medal(s.place as usize).unwrap_or(MUTED);
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: rem(0.375),
            width: rem(6.5),
            ..default()
        },
        Motion::slide(0.0, 48.0).after(match s.place {
            1 => 0.45,
            2 => 0.25,
            _ => 0.1,
        }),
    ))
    .with_children(|c| {
        avatar(c, f, &s.name, suit(s.color), size);
        rich_in(c, f, &s.name, 13.0, INK, true);
        rich_in(c, f, &s.total.to_string(), 15.0, INK, true);
        c.spawn((
            Node {
                width: percent(100),
                height: rem(high),
                justify_content: JustifyContent::Center,
                padding: UiRect::top(rem(0.5)),
                border_radius: BorderRadius::new(rem(0.75), rem(0.75), px(0), px(0)),
                ..default()
            },
            BackgroundGradient::from(LinearGradient::to_bottom(vec![
                m.with_alpha(0.95).into(),
                m.with_alpha(0.25).into(),
            ])),
        ))
        .with_children(|b| {
            big(b, f, &s.place.to_string(), 26.0, INK);
        });
    });
}

/// The end of the game: the podium, the final table and the titles, beside the podium on the field.
fn summary(
    q: Single<Entity, With<SummaryBox>>,
    outcome: Res<Outcome>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        let Some(g) = &outcome.game_end else { return };
        p.spawn((
            Node {
                width: percent(100),
                max_height: percent(100),
                flex_direction: FlexDirection::Column,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(rem(1.5)),
                overflow: Overflow::clip(),
                ..default()
            },
            glass(),
            Motion::slide(48.0, 0.0).lasting(0.45),
        ))
        .with_children(|c| {
            let title = match g.standings.first() {
                Some(w) if session.me == Some(w.id) => text::YOU_WON.to_string(),
                Some(w) => text::wins(&w.name),
                None => text::GAME_OVER.to_string(),
            };
            board_head(c, f, text::GAME_SUMMARY, &format!("👑 {title}"), text::BACK_TO_LOBBY);
            c.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: rem(0.75),
                    padding: UiRect::all(rem(1.0)),
                    overflow: Overflow::scroll_y(),
                    flex_shrink: 1.0,
                    ..default()
                },
                bevy::ui_widgets::ScrollArea,
            ))
            .with_children(|b| {
                b.spawn(Node {
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::FlexEnd,
                    column_gap: rem(0.375),
                    padding: UiRect::top(rem(0.5)),
                    ..default()
                })
                .with_children(|podium| {
                    for place in [2, 1, 3] {
                        if let Some(s) = g.standings.iter().find(|s| s.place == place) {
                            podium_step(podium, f, s);
                        }
                    }
                });
                caption(b, f, text::STANDINGS);
                b.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    ..default()
                })
                .with_children(|t| {
                    for s in &g.standings {
                        let me = session.me == Some(s.id);
                        table_row(t, me, |r| {
                            place_badge(r, f, s.place as usize, 1.375, true);
                            r.spawn(Node {
                                flex_grow: 1.0,
                                min_width: px(0),
                                column_gap: rem(0.375),
                                align_items: AlignItems::Center,
                                overflow: Overflow::clip(),
                                ..default()
                            })
                            .with_children(|x| {
                                dot(x, suit(s.color), 0.625);
                                rich_in(x, f, &s.name, 13.0, INK, true);
                            });
                            rich(r, f, &format!("🏆{}", s.wins), 12.0, MUTED);
                            cell(r, 2.5, |x| {
                                rich_in(x, f, &s.total.to_string(), 15.0, INK, true);
                            });
                        });
                    }
                });
                if !g.awards.is_empty() {
                    caption(b, f, text::AWARDS);
                }
                for (i, a) in g.awards.iter().enumerate() {
                    b.spawn((
                        Node {
                            column_gap: rem(0.75),
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(rem(0.75), rem(0.5)),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(rem(0.875)),
                            ..default()
                        },
                        BackgroundColor(GROUP),
                        BorderColor::all(RIM),
                        Motion::slide(24.0, 0.0).after(0.6 + i as f32 * 0.08),
                    ))
                    .with_children(|r| {
                        let (icon, title, what) = text::award(a);
                        r.spawn((
                            Node {
                                width: rem(2.5),
                                height: rem(2.5),
                                border_radius: BorderRadius::all(rem(0.75)),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                flex_shrink: 0.0,
                                ..default()
                            },
                            BackgroundColor(GOLD.with_alpha(0.14)),
                        ))
                        .with_children(|i| {
                            rich(i, f, icon, 20.0, INK);
                        });
                        stack(r, |s| {
                            heading(s, f, title);
                            row(s, true, |x| {
                                name_tag(x, f, &session, a.id, 12.0);
                                muted(x, f, &what);
                            });
                        });
                    });
                }
            });
        });
    });
}

/// Out of play in a round: finished, out, or watching, and whom the camera follows.
fn status(
    q: Single<Entity, With<StatusBox>>,
    hud: Res<Hud>,
    outcome: Res<Outcome>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let between = outcome.results.is_some() || outcome.game_end.is_some();
    let on = hud.on && hud.kind == Some(ArenaKind::Round) && !between && hud.status != Part::Play;
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        if !on {
            return;
        }
        let (line, ink, bar) = match hud.status {
            Part::Finished => (text::finished(hud.place), GOOD, BLUE),
            Part::Out => (text::YOU_ARE_OUT.to_string(), CRITICAL, CRITICAL),
            _ => (text::SPECTATOR.to_string(), INK, BLUE),
        };
        p.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: rem(0.25),
                padding: UiRect::axes(rem(1.75), rem(0.75)),
                border: UiRect::new(px(1), px(1), px(3), px(1)),
                border_radius: BorderRadius::all(rem(1.125)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor {
                top: bar,
                ..BorderColor::all(RIM)
            },
            BoxShadow::new(bar.with_alpha(0.25), px(0), px(0), px(0), px(24)),
            Motion::pop(0.85),
        ))
        .with_children(|r| {
            big(r, f, &line, 18.0, ink);
            row(r, false, |x| {
                rich_in(x, f, &text::camera_on(hud.spectating.as_deref()), 13.0, INK, true);
                rich(x, f, "·", 13.0, FAINT);
                muted(x, f, text::SPECTATE_HINT);
            });
        });
    });
}

/// In play but the mouse is free: ask for a click.
fn prompt(ui: Res<Ui>, hud: Res<Hud>, mut q: Single<&mut Node, With<PromptBox>>) {
    show(&mut q, hud.on && ui.need_click && !ui.menu);
}
