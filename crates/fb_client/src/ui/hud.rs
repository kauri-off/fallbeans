//! The in-game HUD: players, feed, intro and countdown, timer, round results and summary.
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
                    keys,
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
struct TimerText;
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
struct KeysBox;
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
        row_gap: rem(0.375),
        ..default()
    }
}

fn shadowed(size: f32, color: Color, font: &Handle<Font>) -> impl Bundle {
    (
        Text::new(""),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        TextShadow {
            offset: Vec2::new(0.0, size / 16.0),
            color: INK.with_alpha(0.85),
        },
    )
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
            h.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: rem(0.75),
                    left: rem(0.75),
                    width: rem(16.0),
                    max_height: percent(96),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::new(rem(0.75), rem(0.75), rem(0.625), rem(0.5)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(rem(1.125)),
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
                        margin: UiRect::top(rem(0.375)),
                        ..default()
                    },
                    Text::new(""),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                    TextColor(MUTED),
                ));
            });
            h.spawn((
                FeedBox,
                Node {
                    position_type: PositionType::Absolute,
                    top: rem(0.75),
                    right: rem(0.75),
                    max_width: percent(46),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexEnd,
                    row_gap: rem(0.375),
                    ..default()
                },
            ));
            h.spawn(centred(Some(rem(0.75)), None)).with_children(|t| {
                let full = || Node {
                    width: percent(100),
                    justify_content: JustifyContent::Center,
                    ..default()
                };
                t.spawn((IntroBox, full()));
                t.spawn((TimerText, shadowed(34.0, Color::WHITE, &f.black)));
                for pill in [Pill::MapText, Pill::Bonus] {
                    t.spawn((
                        pill,
                        Node {
                            padding: UiRect::axes(rem(1.0), rem(0.375)),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::MAX,
                            display: display(false),
                            ..default()
                        },
                        glass(),
                    ))
                    .with_children(|p| {
                        rich_in(p, f, "", 16.0, INK, pill == Pill::Bonus);
                    });
                }
                t.spawn((GoText, shadowed(72.0, YELLOW, &f.black)));
                t.spawn((ResultsBox, full()));
            });
            h.spawn((
                SummaryBox,
                Node {
                    position_type: PositionType::Absolute,
                    top: rem(0.75),
                    right: rem(0.75),
                    width: rem(20.0),
                    ..default()
                },
            ));
            h.spawn(centred(Some(percent(30)), None)).with_children(|c| {
                c.spawn((CountText, shadowed(120.0, Color::WHITE, &f.black)));
            });
            h.spawn((StatusBox, centred(None, Some(rem(3.5)))));
            h.spawn(centred(None, Some(rem(0.75)))).with_children(|k| {
                k.spawn((
                    KeysBox,
                    Node {
                        padding: UiRect::axes(rem(0.9), rem(0.4)),
                        border_radius: BorderRadius::MAX,
                        max_width: percent(96),
                        display: display(false),
                        ..default()
                    },
                    BackgroundColor(INK.with_alpha(0.75)),
                ))
                .with_children(|k| {
                    rich(k, f, "", 13.0, Color::WHITE);
                });
            });
            h.spawn(centred(Some(percent(58)), None)).with_children(|c| {
                c.spawn((
                    PromptBox,
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(rem(1.4), rem(0.8)),
                        border_radius: BorderRadius::all(rem(1.0)),
                        display: display(false),
                        ..default()
                    },
                    BackgroundColor(INK.with_alpha(0.78)),
                ))
                .with_children(|c| {
                    rich_in(c, f, text::CLICK_FIELD, 18.0, Color::WHITE, true);
                    rich(c, f, text::OR_ESC_MENU, 12.0, Color::WHITE);
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
        next_in,
    };
}

fn root(hud: Res<Hud>, mut q: Single<&mut Node, With<HudRoot>>) {
    show(&mut q, hud.on);
}

fn name_tag(p: &mut ChildSpawnerCommands, f: &Fonts, session: &Session, id: PlayerId, size: f32) {
    let me = session.me == Some(id);
    let (name, color) = match session.player(id) {
        Some(pl) => (pl.name.clone(), Some(pl.color)),
        None => (format!("#{id}"), None),
    };
    row(p, false, |r| {
        dot(r, color.map_or(Color::srgb(0.8, 0.8, 0.8), suit), 0.6);
        rich_in(r, f, &name, size, INK, me);
    });
}

fn genre_color(g: Genre) -> Color {
    match g {
        Genre::Race => Color::srgb(0.086, 0.451, 0.769),
        Genre::Survival => Color::srgb(0.722, 0.306, 0.047),
        Genre::Points => Color::srgb(0.075, 0.478, 0.243),
    }
}

fn genre_pill(p: &mut ChildSpawnerCommands, f: &Fonts, g: Genre, s: &str) {
    p.spawn((
        Node {
            padding: UiRect::axes(rem(0.5625), px(2)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(genre_color(g)),
    ))
    .with_children(|c| {
        rich(c, f, s, 12.0, Color::WHITE);
    });
}

/// Top left: the round, the players with their status, score and ping.
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
        row(p, false, |r| {
            if round {
                let s = if info.practice {
                    text::PRACTICE_TITLE.to_string()
                } else {
                    format!("{}/{}", info.index, info.total)
                };
                genre_pill(r, f, def.genre, &s);
                rich_in(r, f, def.title, 14.0, INK, true);
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
                rich_in(r, f, &s, 14.0, INK, true);
            }
        });
        for pl in &players {
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
            p.spawn(Node {
                column_gap: rem(0.375),
                align_items: AlignItems::Center,
                padding: UiRect::axes(rem(0.25), px(2)),
                ..default()
            })
            .with_children(|r| {
                r.spawn(Node {
                    flex_grow: 1.0,
                    min_width: px(0),
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|n| {
                    dot(n, suit(pl.color), 0.6);
                    n.spawn(Node {
                        margin: UiRect::left(rem(0.3)),
                        ..default()
                    })
                    .with_children(|t| {
                        let ink = if dim { MUTED } else { INK };
                        rich_in(t, f, &pl.name, 13.0, ink, session.me == Some(pl.id));
                    });
                });
                if Some(pl.id) == host {
                    rich(r, f, "⭐", 12.0, INK);
                }
                if info.kind == ArenaKind::Lobby && pl.crowns > 0 {
                    rich(r, f, &format!("👑{}", pl.crowns), 12.0, INK);
                }
                let bells = session.scores.get(&pl.id).copied().unwrap_or(0);
                if info.kind == ArenaKind::Lobby && bells > 0 {
                    rich(r, f, &format!("🔔{bells}"), 12.0, INK);
                }
                if !icon.is_empty() {
                    rich(r, f, &icon, 12.0, INK);
                }
                if round && genre_points {
                    rich(r, f, &bells.to_string(), 12.0, GREEN_INK);
                }
                if info.kind != ArenaKind::Lobby {
                    rich_in(r, f, &pl.score.to_string(), 13.0, INK, true);
                }
                let ping = if pl.bot {
                    text::BOT.to_string()
                } else if pl.connected {
                    pl.ping.to_string()
                } else {
                    "—".into()
                };
                rich(r, f, &ping, 11.0, MUTED);
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
    p.spawn((
        FeedRow(x.n, x.at),
        Node {
            column_gap: rem(0.375),
            align_items: AlignItems::Center,
            padding: UiRect::axes(rem(0.75), rem(0.3125)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(0.75)),
            // (A note may carry a long path: an F8 report's.)
            max_width: rem(30.0),
            ..default()
        },
        glass(),
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
            rich(r, f, icon, 15.0, INK);
            name_tag(r, f, session, *victim, 13.0);
            let ink = if *out { RED_INK } else { MUTED };
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
        p.spawn((
            Node {
                width: rem(32.5),
                max_width: percent(60),
                flex_direction: FlexDirection::Column,
                row_gap: rem(0.25),
                padding: UiRect::axes(rem(1.0), rem(0.625)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(rem(1.125)),
                ..default()
            },
            glass(),
        ))
        .with_children(|c| {
            row(c, false, |r| {
                genre_pill(r, f, m.genre, text::genre(m.genre));
                if !info.practice {
                    muted(r, f, &text::round_of(info.index, info.total));
                }
                r.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
                });
                let left = rich(r, f, "", 13.0, PINK);
                r.commands().entity(left).insert(IntroLeft);
            });
            rich_in(c, f, m.title, 24.0, PINK, true);
            label(c, f, &format!("🎯 {}.", m.goal));
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

/// The intro box, and the timer, «ВПЕРЁД!» and 3-2-1 texts.
#[derive(SystemParam)]
struct Countdown<'w, 's> {
    intro: Single<'w, 's, &'static mut Node, (With<IntroBox>, Without<Pill>)>,
    timer: Single<'w, 's, (&'static mut Text, &'static mut TextColor), Only<TimerText, GoText, CountText>>,
    go: Single<'w, 's, &'static mut Text, Only<GoText, TimerText, CountText>>,
    count: Single<'w, 's, &'static mut Text, Only<CountText, TimerText, GoText>>,
}

/// Every frame: the intro and its time to the start, the timer, the map's line, the bonus in effect, «ВПЕРЁД!»,
/// 3-2-1, and the time to the next scene.
fn clock(
    round: RoundView,
    mut countdown: Countdown,
    mut pills: Query<(Entity, &Pill, &mut Node), Without<IntroBox>>,
    mut leaves: Query<(&mut Rich, Option<&IntroLeft>, Option<&NextLine>)>,
    children: Query<&Children>,
) {
    let between = round.between();
    let in_round = round.hud.on && round.hud.kind == Some(ArenaKind::Round) && !between;
    let before = in_round && round.time.t < 0.0;
    let on = in_round && round.time.t >= 0.0;
    show(&mut countdown.intro, before);
    let (ref mut t, ref mut c) = *countdown.timer;
    let s = if on {
        text::fmt_time(round.time.time_left)
    } else {
        String::new()
    };
    if t.0 != s {
        t.0 = s;
    }
    c.set_if_neq(TextColor(if round.time.time_left < 10.0 {
        YELLOW
    } else {
        Color::WHITE
    }));
    let s = if on && round.time.t < 1.2 { text::GO } else { "" };
    if countdown.go.0 != s {
        countdown.go.0 = s.into();
    }
    let left = (-round.time.t).ceil() as i64;
    let s = if round.hud.on && round.hud.kind == Some(ArenaKind::Round) && (1..=3).contains(&left) {
        left.to_string()
    } else {
        String::new()
    };
    if countdown.count.0 != s {
        countdown.count.0 = s;
    }
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
    let e = rich_in(p, f, "", 13.0, PINK, true);
    p.commands().entity(e).insert(NextLine(label_s));
}

/// After a round: points won and lost, compact, at the top.
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
                width: rem(31.25),
                max_width: percent(60),
                flex_direction: FlexDirection::Column,
                row_gap: rem(0.2),
                padding: UiRect::axes(rem(0.875), rem(0.625)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(rem(1.125)),
                ..default()
            },
            glass(),
        ))
        .with_children(|c| {
            c.spawn(Node {
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Baseline,
                ..default()
            })
            .with_children(|h| {
                heading(h, f, title);
                let s = if r.practice {
                    text::PRACTICE_SMALL.to_string()
                } else {
                    text::results_of(r.index, r.total)
                };
                muted(h, f, &s);
            });
            c.spawn(Node {
                justify_content: JustifyContent::Center,
                ..default()
            })
            .with_children(|n| next_line(n, f, next));
            for row_ in &r.rows {
                let me = session.me == Some(row_.id);
                table_row(c, me, |t| {
                    cell(t, 1.375, |x| {
                        rich_in(x, f, &row_.place.to_string(), 13.0, INK, true);
                    });
                    t.spawn(Node {
                        flex_grow: 1.0,
                        min_width: px(0),
                        overflow: Overflow::clip(),
                        ..default()
                    })
                    .with_children(|x| name_tag(x, f, &session, row_.id, 13.0));
                    rich(t, f, &text::round_note(row_.note), 12.0, MUTED);
                    cell(t, 2.0, |x| {
                        rich(x, f, &format!("+{}", row_.points), 13.0, GREEN_INK);
                    });
                    cell(t, 1.625, |x| {
                        if row_.penalty > 0 {
                            rich(x, f, &format!("−{}", row_.penalty), 13.0, RED_INK);
                        }
                    });
                    cell(t, 1.875, |x| {
                        let ink = match row_.delta {
                            d if d > 0 => GREEN_INK,
                            d if d < 0 => RED_INK,
                            _ => INK,
                        };
                        rich_in(x, f, &text::signed(row_.delta), 13.0, ink, true);
                    });
                    cell(t, 2.125, |x| {
                        rich_in(x, f, &row_.total.to_string(), 13.0, INK, true);
                    });
                });
            }
        });
    });
}

fn table_row(p: &mut ChildSpawnerCommands, me: bool, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn((
        Node {
            column_gap: rem(0.375),
            align_items: AlignItems::Center,
            padding: UiRect::axes(rem(0.25), px(2)),
            border_radius: BorderRadius::all(rem(0.375)),
            ..default()
        },
        BackgroundColor(if me {
            Color::srgba(1.0, 1.0, 1.0, 0.6)
        } else {
            Color::NONE
        }),
    ))
    .with_children(f);
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

/// The end of the game: the final table and the titles, beside the podium.
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
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: rem(0.5),
                padding: UiRect::axes(rem(0.875), rem(0.75)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(rem(1.125)),
                ..default()
            },
            glass(),
        ))
        .with_children(|c| {
            rich(c, f, "👑", 40.0, INK);
            let title = match g.standings.first() {
                Some(w) if session.me == Some(w.id) => text::YOU_WON.to_string(),
                Some(w) => text::wins(&w.name),
                None => text::GAME_OVER.to_string(),
            };
            rich_in(c, f, &title, 22.0, PINK, true);
            c.spawn(Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|t| {
                for s in &g.standings {
                    let me = session.me == Some(s.id);
                    table_row(t, me, |r| {
                        let place = match s.place {
                            1 => "🥇".to_string(),
                            2 => "🥈".to_string(),
                            3 => "🥉".to_string(),
                            n => n.to_string(),
                        };
                        cell(r, 1.6, |x| {
                            rich_in(x, f, &place, 13.0, INK, true);
                        });
                        r.spawn(Node {
                            flex_grow: 1.0,
                            column_gap: rem(0.3),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|x| {
                            dot(x, suit(s.color), 0.6);
                            rich_in(x, f, &s.name, 13.0, INK, me);
                        });
                        rich(r, f, &format!("🏆{}", s.wins), 12.0, MUTED);
                        cell(r, 2.125, |x| {
                            rich_in(x, f, &s.total.to_string(), 13.0, INK, true);
                        });
                    });
                }
            });
            for a in &g.awards {
                c.spawn((
                    Node {
                        width: percent(100),
                        column_gap: rem(0.5),
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(rem(0.5), rem(0.3125)),
                        border_radius: BorderRadius::all(rem(0.75)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.45)),
                ))
                .with_children(|r| {
                    let (icon, title, what) = text::award(a);
                    rich(r, f, icon, 22.0, INK);
                    stack(r, |s| {
                        heading(s, f, title);
                        row(s, true, |x| {
                            name_tag(x, f, &session, a.id, 12.0);
                            muted(x, f, &what);
                        });
                    });
                });
            }
            next_line(c, f, text::BACK_TO_LOBBY);
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
        let (line, ink) = match hud.status {
            Part::Finished => (text::finished(hud.place), GREEN_INK),
            Part::Out => (text::YOU_ARE_OUT.to_string(), RED_INK),
            _ => (text::SPECTATOR.to_string(), INK),
        };
        p.spawn((
            Node {
                column_gap: rem(0.625),
                align_items: AlignItems::Center,
                padding: UiRect::axes(rem(1.125), rem(0.5)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(rem(1.125)),
                ..default()
            },
            glass(),
        ))
        .with_children(|r| {
            rich_in(
                r,
                f,
                &format!("{line} · {}", text::camera_on(hud.spectating.as_deref())),
                17.0,
                ink,
                true,
            );
            muted(r, f, text::SPECTATE_HINT);
        });
    });
}

/// The controls line at the bottom: in the first seconds of a round, and in the lobby with the menu shut.
fn keys(
    round: RoundView,
    ui: Res<Ui>,
    session: Res<Session>,
    binds: Res<crate::settings::Bindings>,
    mut q: Single<(Entity, &mut Node), With<KeysBox>>,
    mut labels: Labels,
) {
    let between = round.between();
    let info = session.arena.as_ref();
    let line = match round.hud.kind {
        Some(ArenaKind::Round)
            if round.hud.on && !between && round.hud.status == Part::Play && (0.0..10.0).contains(&round.time.t) =>
        {
            let practice = info.is_some_and(|i| i.practice);
            let grab = info.is_some_and(|i| fb_maps::by_id(i.game).meta().grab);
            let g = if grab {
                "захват хвоста"
            } else {
                "захват"
            };
            Some(text::keys(
                ui.pad,
                &binds,
                g,
                !practice,
                &text::menu_lead(ui.pad, false),
            ))
        }
        Some(ArenaKind::Lobby) if round.hud.on && !ui.menu => Some(text::keys(
            ui.pad,
            &binds,
            "захват",
            true,
            &text::menu_lead(ui.pad, session.host()),
        )),
        _ => None,
    };
    let (e, ref mut node) = *q;
    show(node, line.is_some());
    if let Some(line) = line {
        labels.set(e, &line);
    }
}

/// In play but the mouse is free: ask for a click.
fn prompt(ui: Res<Ui>, hud: Res<Hud>, mut q: Single<&mut Node, With<PromptBox>>) {
    show(&mut q, hud.on && ui.need_click && !ui.menu);
}
