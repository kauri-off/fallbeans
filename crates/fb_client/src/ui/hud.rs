//! The HUD over the game: the players' panel, the feed, the intro and countdown,
//! the timer and the map's line, the round's results, the game's summary, the status of a player out of
//! play, the controls line and the prompt to click back in.
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use fb_arena::{ArenaKind, client_hud};
use fb_net::*;
use fb_proto::Pid;
use fb_shared::DT;
use fb_shared::game::Genre;
use lightyear::prelude::*;

use super::*;
use crate::game::Map;
use crate::net::Conn;
use crate::opts::Transport;
use crate::session::{FEED_SECS, Feed, Session};
use crate::view::{Spectate, frame_tick};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hud>();
        app.add_systems(Startup, build_hud.after(super::setup));
        app.add_systems(
            Update,
            (
                hud_state,
                (
                    panel, net_line, feed, intro, top, results, summary, status, keys, prompt, count,
                ),
            )
                .chain(),
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Part {
    #[default]
    Play,
    Finished,
    Out,
    Spectating,
}

/// What the HUD shows, worked out once a frame.
#[derive(Resource, Default)]
pub struct Hud {
    pub on: bool,
    pub kind: Option<ArenaKind>,
    /// Round time, seconds (negative during the intro).
    pub t: f64,
    pub time_left: f64,
    /// Seconds until the next scene starts on its own.
    pub next_in: Option<f64>,
    pub status: Part,
    pub place: usize,
    pub spectating: Option<String>,
    pub map_text: Option<String>,
    pub bonus: Option<String>,
    pub roster: Vec<(Pid, Part, usize)>,
    pub fps: f64,
}

#[derive(Component)]
struct PanelBox;
#[derive(Component)]
struct NetLine;
#[derive(Component)]
struct FeedBox;
#[derive(Component)]
struct IntroBox;
#[derive(Component)]
struct TopBox;
#[derive(Component)]
struct TimerText;
#[derive(Component)]
struct Pill(u8);
#[derive(Component)]
struct GoText;
#[derive(Component)]
struct ResultsBox;
#[derive(Component)]
struct SummaryBox;
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

fn build_hud(mut commands: Commands, layers: Query<(Entity, &Layer)>, f: Res<Fonts>) {
    let Some((e, _)) = layers.iter().find(|(_, l)| **l == Layer::Hud) else {
        return;
    };
    commands.entity(e).with_children(|h| {
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
                Section::default(),
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
            Section::default(),
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
        h.spawn((TopBox, centred(Some(rem(0.75)), None))).with_children(|t| {
            let full = || Node {
                width: percent(100),
                justify_content: JustifyContent::Center,
                ..default()
            };
            t.spawn((IntroBox, Section::default(), full()));
            t.spawn((TimerText, shadowed(34.0, Color::WHITE, &f.black)));
            for i in 0..2 {
                t.spawn((
                    Pill(i),
                    Section::default(),
                    Node {
                        padding: UiRect::axes(rem(1.0), rem(0.375)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::MAX,
                        display: bevy::ui::Display::None,
                        ..default()
                    },
                    glass(),
                ));
            }
            t.spawn((GoText, shadowed(72.0, YELLOW, &f.black)));
            t.spawn((ResultsBox, Section::default(), full()));
        });
        h.spawn((
            SummaryBox,
            Section::default(),
            Node {
                position_type: PositionType::Absolute,
                top: rem(0.75),
                right: rem(0.75),
                width: rem(20.0),
                ..default()
            },
        ));
        h.spawn((centred(Some(percent(30)), None),)).with_children(|c| {
            c.spawn((CountText, shadowed(120.0, Color::WHITE, &f.black)));
        });
        h.spawn((StatusBox, Section::default(), centred(None, Some(rem(3.5)))));
        h.spawn((KeysBox, Section::default(), centred(None, Some(rem(0.75)))));
        h.spawn((PromptBox, Section::default(), centred(Some(percent(58)), None)));
    });
}

fn hud_state(
    mut hud: ResMut<Hud>,
    map: Option<ResMut<Map>>,
    session: Res<Session>,
    spectate: Option<Res<Spectate>>,
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    own: Query<&BodyFull, With<Predicted>>,
    diag: Res<DiagnosticsStore>,
) {
    let Some(mut map) = map.filter(|_| session.room.is_some()) else {
        if hud.on {
            *hud = Hud::default();
        }
        return;
    };
    let map = &mut *map;
    let kind = map.round.kind;
    let tick = frame_tick(&timeline, &fixed);
    let t = map.time(tick);
    let me = session.me;
    let mut roster = Vec::new();
    for id in &map.info.participants {
        let place = map.info.finished.iter().position(|f| f == id);
        let st = if place.is_some() {
            Part::Finished
        } else if map.info.out.contains(id) {
            Part::Out
        } else {
            Part::Play
        };
        roster.push((*id, st, place.map_or(0, |p| p + 1)));
    }
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
    let bonus = body.filter(|b| b.power != 0).and_then(|b| {
        let left = (b.power_until - timeline.tick().0 as f64 * DT + map.round.zero_tick as f64 * DT).ceil();
        let (icon, title) = text::bonus(b.power);
        (left > 0.0).then(|| format!("{icon} {title} · {left:.0} с"))
    });
    let map_text = if kind == ArenaKind::Round && t >= 0.0 {
        let mut scores = session.scores.clone();
        client_hud(&mut map.world, &map.spec, &mut scores, me)
    } else {
        None
    };
    let duration = fb_maps::by_id(&map.round.map).map_or(0.0, |d| d.meta().duration);
    let next_in = session
        .lobby
        .as_ref()
        .and_then(|l| l.next)
        .map(|n| ((n as f64 - timeline.tick().0 as f64) * DT).max(0.0));
    let spectating = spectate
        .and_then(|s| s.target)
        .filter(|_| status != Part::Play)
        .map(|id| session.name_of(id));
    *hud = Hud {
        on: true,
        kind: Some(kind),
        t,
        time_left: (duration - t).max(0.0),
        next_in,
        status,
        place,
        spectating,
        map_text,
        bonus,
        roster,
        fps: diag
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|d| d.smoothed())
            .unwrap_or(0.0),
    };
}

fn name_tag(p: &mut ChildSpawnerCommands, f: &Fonts, session: &Session, id: Pid, size: f32) {
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
    mut q: Query<(Entity, &mut Section), With<PanelBox>>,
    hud: Res<Hud>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let Some(info) = session.arena.as_ref().filter(|_| hud.on) else {
        return;
    };
    let round = info.kind == ArenaKind::Round;
    let points: Vec<(Pid, i64)> = session.scores.iter().map(|(id, v)| (*id, *v as i64)).collect();
    let key = key_of(&(&session.lobby, info.id, info.kind, &hud.roster, &points, session.me));
    if !sec.stale(key) {
        return;
    }
    let f = &*f;
    let def = fb_maps::by_id(&info.game).map(|d| d.meta());
    let mut players = session.lobby.as_ref().map_or_else(Vec::new, |l| l.players.clone());
    if info.kind == ArenaKind::Lobby {
        players.sort_by_key(|p| (core::cmp::Reverse(p.crowns), p.id));
    } else {
        players.sort_by_key(|p| (core::cmp::Reverse(p.score), p.id));
    }
    let host = session.lobby.as_ref().and_then(|l| l.host);
    let genre_points = def.is_some_and(|m| m.genre == Genre::Points);
    let info = info.clone();
    rebuild(&mut commands, e, |p| {
        row(p, false, |r| match (round, def) {
            (true, Some(m)) => {
                let s = if info.practice {
                    text::PRACTICE_TITLE.to_string()
                } else {
                    format!("{}/{}", info.index, info.total)
                };
                genre_pill(r, f, m.genre, &s);
                rich_in(r, f, m.title, 14.0, INK, true);
            }
            _ => {
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
            let st = hud.roster.iter().find(|r| r.0 == pl.id);
            let icon = if !round {
                String::new()
            } else {
                match st {
                    None => "👁".into(),
                    Some((_, Part::Finished, place)) => format!("🏁{place}"),
                    Some((_, Part::Out, _)) => "✖".into(),
                    Some(_) => String::new(),
                }
            };
            let dim = st.is_some_and(|s| s.1 == Part::Out) || !pl.connected;
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
                let bells = points.iter().find(|s| s.0 == pl.id).map_or(0, |s| s.1);
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
    time: Res<Time<Real>>,
    mut every: Local<f32>,
) {
    let now = time.elapsed_secs();
    if now - *every < 0.25 || !hud.on {
        return;
    }
    *every = now;
    let Ok(mut t) = q.single_mut() else { return };
    let Some(conn) = conn else { return };
    let rtt = conn
        .entity
        .and_then(|e| links.get(e).ok())
        .map_or(0.0, |l| l.stats.rtt.as_secs_f32() * 1000.0);
    let transport = if conn.transport == Transport::Ws { "TCP" } else { "UDP" };
    let mut s = format!("{transport} · {}", text::ping(rtt.round() as u32));
    if display.show_fps {
        s += &format!(" · {:.0} к/с", hud.fps);
    }
    if let Some(hint) = crate::diag::vpn_hint(&conn, now) {
        s += "\n";
        s += hint;
    }
    if t.0 != s {
        t.0 = s;
    }
}

/// Top right: who fell and why; finishes, bonuses and other notes.
fn feed(
    mut q: Query<(Entity, &mut Section), With<FeedBox>>,
    session: Res<Session>,
    hud: Res<Hud>,
    time: Res<Time<Real>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let now = time.elapsed_secs();
    let live: Vec<_> = session
        .feed
        .iter()
        .filter(|x| hud.on && now - x.at < FEED_SECS && session.game_end.is_none())
        .collect();
    let ns: Vec<u32> = live.iter().map(|x| x.n).collect();
    if !sec.stale(key_of(&ns)) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| {
        for x in live {
            p.spawn((
                Node {
                    column_gap: rem(0.375),
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(rem(0.75), rem(0.3125)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(rem(0.75)),
                    ..default()
                },
                glass(),
            ))
            .with_children(|r| match &x.what {
                Feed::Note(s) => {
                    rich(r, f, s, 13.0, INK);
                }
                Feed::Ko {
                    victim,
                    by,
                    cause,
                    out,
                    shortcut,
                } => {
                    let (icon, why) = text::cause(cause);
                    if let Some(by) = by.filter(|b| b != victim) {
                        name_tag(r, f, &session, by, 13.0);
                    }
                    rich(r, f, icon, 15.0, INK);
                    name_tag(r, f, &session, *victim, 13.0);
                    let ink = if *out { RED_INK } else { MUTED };
                    rich(r, f, &text::ko_line(&why, *out, *shortcut), 12.0, ink);
                }
            });
        }
    });
}

/// Before the start: the game, its goal, and the time to the start (the camera flies over the course).
fn intro(
    mut q: Query<(Entity, &mut Section), With<IntroBox>>,
    hud: Res<Hud>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let between = session.results.is_some() || session.game_end.is_some();
    let on = hud.on && hud.kind == Some(ArenaKind::Round) && hud.t < 0.0 && !between;
    let info = session.arena.as_ref().filter(|_| on);
    let left = (-hud.t).ceil() as i64;
    if !sec.stale(key_of(&(info.map(|i| i.id), on.then_some(left)))) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| {
        let Some(info) = info else { return };
        let Some(m) = fb_maps::by_id(&info.game).map(|d| d.meta()) else {
            return;
        };
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
                rich(r, f, &text::in_secs(text::TO_START, -hud.t), 13.0, PINK);
            });
            rich_in(c, f, m.title, 24.0, PINK, true);
            label(c, f, &format!("🎯 {}.", m.goal));
            muted(c, f, m.desc);
        });
    });
}

/// After the start: the timer, the map's line, the bonus in effect, «ВПЕРЁД!».
fn top(
    hud: Res<Hud>,
    session: Res<Session>,
    mut timer: Query<(&mut Text, &mut TextColor), (With<TimerText>, Without<GoText>)>,
    mut go: Query<&mut Text, (With<GoText>, Without<TimerText>)>,
    mut pills: Query<(Entity, &Pill, &mut Section, &mut Node)>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let between = session.results.is_some() || session.game_end.is_some();
    let on = hud.on && hud.kind == Some(ArenaKind::Round) && hud.t >= 0.0 && !between;
    if let Ok((mut t, mut c)) = timer.single_mut() {
        let s = if on {
            text::fmt_time(hud.time_left)
        } else {
            String::new()
        };
        if t.0 != s {
            t.0 = s;
        }
        let color = if hud.time_left < 10.0 { YELLOW } else { Color::WHITE };
        c.set_if_neq(TextColor(color));
    }
    if let Ok(mut t) = go.single_mut() {
        let s = if on && hud.t < 1.2 { text::GO } else { "" };
        if t.0 != s {
            t.0 = s.into();
        }
    }
    let f = &*f;
    for (e, pill, mut sec, mut node) in &mut pills {
        let s = if !on {
            None
        } else if pill.0 == 0 {
            hud.map_text.clone()
        } else {
            hud.bonus.clone()
        };
        show(&mut node, s.is_some());
        if sec.stale(key_of(&s)) {
            rebuild(&mut commands, e, |p| {
                if let Some(s) = s {
                    rich_in(p, f, &s, 16.0, INK, pill.0 == 1);
                }
            });
        }
    }
}

/// 3, 2, 1 before the start.
fn count(hud: Res<Hud>, mut q: Query<&mut Text, With<CountText>>) {
    let Ok(mut t) = q.single_mut() else { return };
    let left = (-hud.t).ceil() as i64;
    let on = hud.on && hud.kind == Some(ArenaKind::Round) && (1..=3).contains(&left);
    let s = if on { left.to_string() } else { String::new() };
    if t.0 != s {
        t.0 = s;
    }
}

fn next_line(p: &mut ChildSpawnerCommands, f: &Fonts, label_s: &str, s: Option<f64>) {
    if let Some(s) = s {
        rich_in(p, f, &text::in_secs(label_s, s), 13.0, PINK, true);
    }
}

/// After a round: points won and lost, compact, at the top.
fn results(
    mut q: Query<(Entity, &mut Section), With<ResultsBox>>,
    hud: Res<Hud>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let r = session.results.as_ref().filter(|_| hud.on);
    let next = hud.next_in.map(|s| s.ceil() as i64);
    if !sec.stale(key_of(&(r, next, &session.lobby.as_ref().map(|l| &l.players)))) {
        return;
    }
    let f = &*f;
    let r = r.cloned();
    rebuild(&mut commands, e, |p| {
        let Some(r) = r else { return };
        let title = fb_maps::by_id(&r.game).map_or(r.game.clone(), |d| d.meta().title.to_string());
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
                heading(h, f, &title);
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
            .with_children(|n| next_line(n, f, next, hud.next_in));
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
                    rich(t, f, &row_.note, 12.0, MUTED);
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
    mut q: Query<(Entity, &mut Section), With<SummaryBox>>,
    hud: Res<Hud>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let g = session.game_end.as_ref().filter(|_| hud.on);
    let next = hud.next_in.map(|s| s.ceil() as i64);
    if !sec.stale(key_of(&(g, next))) {
        return;
    }
    let f = &*f;
    let g = g.cloned();
    rebuild(&mut commands, e, |p| {
        let Some(g) = g else { return };
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
                    rich(r, f, &a.icon, 22.0, INK);
                    stack(r, |s| {
                        heading(s, f, &a.title);
                        row(s, true, |x| {
                            name_tag(x, f, &session, a.id, 12.0);
                            muted(x, f, &a.text);
                        });
                    });
                });
            }
            next_line(c, f, text::BACK_TO_LOBBY, hud.next_in);
        });
    });
}

/// Out of play in a round: finished, out, or watching, and whom the camera follows.
fn status(
    mut q: Query<(Entity, &mut Section), With<StatusBox>>,
    hud: Res<Hud>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let between = session.results.is_some() || session.game_end.is_some();
    let on = hud.on && hud.kind == Some(ArenaKind::Round) && !between && hud.status != Part::Play;
    let key = on.then(|| (hud.status, hud.place, hud.spectating.clone()));
    if !sec.stale(key_of(&key)) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| {
        let Some((st, place, who)) = key else { return };
        let (line, ink) = match st {
            Part::Finished => (text::finished(place), GREEN_INK),
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
                &format!("{line} · {}", text::camera_on(who.as_deref())),
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
    mut q: Query<(Entity, &mut Section), With<KeysBox>>,
    hud: Res<Hud>,
    ui: Res<Ui>,
    session: Res<Session>,
    binds: Res<crate::settings::Bindings>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let between = session.results.is_some() || session.game_end.is_some();
    let info = session.arena.as_ref();
    let grab = info
        .and_then(|i| fb_maps::by_id(&i.game))
        .is_some_and(|d| d.meta().grab);
    let line = match hud.kind {
        Some(ArenaKind::Round) if hud.on && !between && hud.status == Part::Play && (0.0..10.0).contains(&hud.t) => {
            let practice = info.is_some_and(|i| i.practice);
            let g = if grab {
                "схватить хвост"
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
        Some(ArenaKind::Lobby) if hud.on && !ui.menu => Some(text::keys(
            ui.pad,
            &binds,
            "захват",
            true,
            &text::menu_lead(ui.pad, session.host()),
        )),
        _ => None,
    };
    if !sec.stale(key_of(&line)) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| {
        let Some(line) = line else { return };
        p.spawn((
            Node {
                padding: UiRect::axes(rem(0.9), rem(0.4)),
                border_radius: BorderRadius::MAX,
                max_width: percent(96),
                ..default()
            },
            BackgroundColor(INK.with_alpha(0.75)),
        ))
        .with_children(|k| {
            rich(k, f, &line, 13.0, Color::WHITE);
        });
    });
}

/// In play but the mouse is free: ask for a click.
fn prompt(
    mut q: Query<(Entity, &mut Section), With<PromptBox>>,
    ui: Res<Ui>,
    hud: Res<Hud>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let on = hud.on && ui.need_click && !ui.menu;
    if !sec.stale(key_of(&on)) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| {
        if !on {
            return;
        }
        p.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(rem(1.4), rem(0.8)),
                border_radius: BorderRadius::all(rem(1.0)),
                ..default()
            },
            BackgroundColor(INK.with_alpha(0.78)),
        ))
        .with_children(|c| {
            rich_in(c, f, text::CLICK_FIELD, 18.0, Color::WHITE, true);
            rich(c, f, text::OR_ESC_MENU, 12.0, Color::WHITE);
        });
    });
}
