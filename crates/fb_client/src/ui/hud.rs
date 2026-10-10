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
                    panel_rows.run_if(resource_changed::<Hud>.or_else(resource_changed::<Session>)),
                    net_line.run_if(on_real_timer(Duration::from_millis(250))),
                    // (Lines gone before the log is looked at: neither takes away what the other did.)
                    (expire, feed.run_if(resource_changed::<FeedLog>)).chain(),
                    feed_box.run_if(resource_changed::<Outcome>),
                    intro.run_if(resource_changed::<Session>),
                    clock,
                    results.run_if(resource_changed::<Outcome>.or_else(resource_changed::<Session>)),
                    summary.run_if(resource_changed::<Outcome>.or_else(resource_changed::<Session>)),
                    status.run_if(resource_changed::<Hud>.or_else(resource_changed::<Outcome>)),
                    prompt.run_if(
                        resource_changed::<Hud>
                            .or_else(resource_changed::<Ui>)
                            .or_else(resource_changed::<Session>),
                    ),
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
struct StandingsCol;
#[derive(Component)]
struct PanelBox;
#[derive(Component)]
struct NetLine;
/// The advice after a fallback to WebSocket: its head and the rest.
#[derive(Component)]
struct VpnBox;
#[derive(Component)]
struct VpnHead;
#[derive(Component)]
struct VpnMore;
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
struct GoBox;
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
struct LobbyHint;
#[derive(Component)]
struct CountBox;
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
        row_gap: px(8),
        ..default()
    }
}

/// Big text in the bold face.
fn big_text(size: f32, color: Color, f: &Fonts) -> impl Bundle {
    (
        Text::new(""),
        TextFont {
            font: f.strong.clone().into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        Pickable::IGNORE,
    )
}

/// The colour of a place: gold, silver and bronze, then none.
fn medal(place: usize) -> Option<(Color, Color)> {
    match place {
        1 => Some((GOLD, hex(0x3D2E08))),
        2 => Some((SILVER, INK)),
        3 => Some((BRONZE, hex(0x3A2414))),
        _ => None,
    }
}

/// A place in a round badge (`size` px), lit in its medal's colour.
fn place_badge(p: &mut ChildSpawnerCommands, f: &Fonts, place: usize, size: f32, lit: bool) {
    let (fill, ink) = medal(place).filter(|_| lit).unwrap_or((CARD2, MUTED));
    p.spawn((
        Node {
            width: px(size),
            height: px(size),
            border_radius: BorderRadius::MAX,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(fill),
    ))
    .with_children(|b| {
        rich_in(b, f, &place.to_string(), (size * 0.5).round(), ink, true);
    });
}

/// A small card floating over the game, softly lit from below.
fn pill(radius: f32) -> impl Bundle {
    (
        Node {
            padding: UiRect::axes(px(12), px(6)),
            border_radius: BorderRadius::all(px(radius)),
            ..default()
        },
        BackgroundColor(PANEL),
        BoxShadow::new(SHADOW.with_alpha(0.1), px(0), px(1), px(0), px(4)),
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
            // Top left: the standings, the link under them and its advice.
            h.spawn((
                StandingsCol,
                Node {
                    position_type: PositionType::Absolute,
                    top: px(24),
                    left: px(24),
                    width: px(300),
                    max_height: percent(94),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexStart,
                    row_gap: px(8),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|c| {
                c.spawn((
                    PanelBox,
                    Node {
                        width: percent(100),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::new(px(12), px(12), px(12), px(8)),
                        border_radius: BorderRadius::all(px(18)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    panel(),
                ));
                c.spawn((
                    NetLine,
                    Node {
                        margin: UiRect::left(px(6)),
                        padding: UiRect::axes(px(9), px(3)),
                        border_radius: BorderRadius::all(px(8)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.984, 0.973, 0.953, 0.82)),
                    Text::new(""),
                    TextFont {
                        font: f.strong.clone().into(),
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(hex(0x3B4A4E)),
                ));
                c.spawn((
                    VpnBox,
                    Node {
                        width: percent(100),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(4),
                        padding: UiRect::axes(px(14), px(12)),
                        border_radius: BorderRadius::all(px(18)),
                        display: display(false),
                        ..default()
                    },
                    panel(),
                ))
                .with_children(|v| {
                    let t = rich_in(v, f, "", 13.5, INK, true);
                    v.commands().entity(t).insert(VpnHead);
                    let t = rich(v, f, "", 13.5, FAINT);
                    v.commands().entity(t).insert(VpnMore);
                });
            });
            // Top right: the feed.
            h.spawn((
                FeedBox,
                Node {
                    position_type: PositionType::Absolute,
                    top: px(24),
                    right: px(24),
                    width: px(420),
                    max_width: percent(36),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexEnd,
                    row_gap: px(6),
                    ..default()
                },
            ));
            // Top middle: the timer over its bar, the map's line and the bonus in effect.
            h.spawn(centred(Some(px(22)), None)).with_children(|t| {
                t.spawn((
                    TimerBox,
                    Node {
                        width: px(190),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Stretch,
                        row_gap: px(4),
                        padding: UiRect::new(px(18), px(18), px(8), px(12)),
                        border_radius: BorderRadius::all(px(18)),
                        display: display(false),
                        ..default()
                    },
                    panel(),
                    Reveal::new(Motion::slide(0.0, -24.0)),
                ))
                .with_children(|b| {
                    b.spawn(Node {
                        justify_content: JustifyContent::Center,
                        ..default()
                    })
                    .with_children(|c| {
                        c.spawn((TimerText, big_text(40.0, INK, f)));
                    });
                    b.spawn((
                        Node {
                            height: px(6),
                            border_radius: BorderRadius::MAX,
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(WELL),
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
                            BackgroundColor(TEAL),
                        ));
                    });
                });
                t.spawn(Node {
                    column_gap: px(8),
                    ..default()
                })
                .with_children(|r| {
                    for p in [Pill::MapText, Pill::Bonus] {
                        let bonus = p == Pill::Bonus;
                        r.spawn((p, pill(12.0), Reveal::new(Motion::pop(0.7))))
                            .insert(Node {
                                padding: UiRect::axes(px(12), px(6)),
                                border_radius: BorderRadius::all(px(12)),
                                display: display(false),
                                ..default()
                            })
                            .insert(BackgroundColor(if bonus { APRICOT_SOFT } else { PANEL }))
                            .with_children(|p| {
                                rich_in(p, f, "", 14.0, if bonus { hex(0x5E331B) } else { INK }, true);
                            });
                    }
                });
            });
            h.spawn((ResultsBox, centred(Some(px(50)), None), Pickable::IGNORE));
            h.spawn((
                SummaryBox,
                Node {
                    position_type: PositionType::Absolute,
                    top: px(0),
                    bottom: px(0),
                    right: px(40),
                    width: px(720),
                    max_width: percent(55),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ));
            // The middle: 3-2-1 in a disc, and «ВПЕРЁД!».
            h.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|c| {
                c.spawn((
                    CountBox,
                    Node {
                        width: px(230),
                        height: px(230),
                        border_radius: BorderRadius::MAX,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        display: display(false),
                        ..default()
                    },
                    panel_raised(),
                ))
                .with_children(|b| {
                    b.spawn((CountText, big_text(140.0, TEAL, f)));
                });
                c.spawn((
                    GoBox,
                    Node {
                        padding: UiRect::axes(px(48), px(16)),
                        border_radius: BorderRadius::all(px(36)),
                        display: display(false),
                        ..default()
                    },
                    BackgroundColor(TEAL),
                    BoxShadow::new(SHADOW, px(0), px(6), px(0), px(24)),
                ))
                .with_children(|b| {
                    b.spawn((GoText, big_text(64.0, ON_FILL, f)));
                });
            });
            h.spawn((
                IntroBox,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(90),
                    top: px(200),
                    width: px(620),
                    max_width: percent(60),
                    ..default()
                },
            ));
            h.spawn((StatusBox, centred(None, Some(px(30)))));
            h.spawn(centred(None, Some(px(30)))).with_children(|c| {
                c.spawn((
                    LobbyHint,
                    Node {
                        column_gap: px(10),
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(px(22), px(12)),
                        border_radius: BorderRadius::all(px(18)),
                        display: display(false),
                        ..default()
                    },
                    panel(),
                    Reveal::new(Motion::slide(0.0, 16.0)),
                ))
                .with_children(|r| {
                    keycap(r, f, "Esc");
                    rich_in(r, f, text::ROOM_MENU, 15.0, INK, true);
                    let t = rich(r, f, text::ENTER_CHAT, 15.0, FAINT);
                    r.commands().entity(t).insert(Node {
                        margin: UiRect::left(px(14)),
                        ..default()
                    });
                });
            });
            h.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|c| {
                c.spawn((
                    PromptBox,
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: px(6),
                        padding: UiRect::axes(px(34), px(22)),
                        border_radius: BorderRadius::all(px(18)),
                        display: display(false),
                        ..default()
                    },
                    panel(),
                    Reveal::new(Motion::pop(0.85)),
                ))
                .with_children(|c| {
                    rich_in(c, f, text::CLICK_FIELD, 22.0, INK, true);
                    rich(c, f, text::OR_ESC_MENU, 16.0, FAINT);
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

/// A player's name in bold, "Вы" for the player.
fn who(session: &Session, id: PlayerId) -> String {
    if session.me == Some(id) {
        text::YOU_SHORT.into()
    } else {
        session.name_of(id)
    }
}

/// Top left: the round, the players in their order with their status, score and ping.
fn panel_rows(
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
    let lobby = info.kind == ArenaKind::Lobby;
    let f = &*f;
    let def = fb_maps::by_id(info.game).meta();
    let mut players = session.lobby.as_ref().map_or_else(Vec::new, |l| l.players.clone());
    if lobby {
        players.sort_by_key(|p| (core::cmp::Reverse(p.crowns), p.id));
    } else {
        players.sort_by_key(|p| (core::cmp::Reverse(p.score), p.id));
    }
    let host = session.lobby.as_ref().and_then(|l| l.host);
    let genre_points = def.genre == Genre::Points;
    rebuild(&mut commands, *q, |p| {
        p.spawn((
            Node {
                column_gap: px(8),
                align_items: AlignItems::Center,
                padding: UiRect::new(px(4), px(4), px(2), px(10)),
                margin: UiRect::bottom(px(4)),
                border: UiRect::bottom(px(1)),
                overflow: Overflow::clip(),
                ..default()
            },
            BorderColor::all(HAIR),
        ))
        .with_children(|h| {
            let title = if round {
                let s = if info.practice {
                    text::PRACTICE_TITLE.to_string()
                } else {
                    format!("{}/{}", info.index, info.total)
                };
                genre_tag(h, f, Some(def.genre), &s);
                rich_in(h, f, text::genre(def.genre), 15.0, INK, true);
                def.title.to_string()
            } else if lobby {
                genre_tag(h, f, None, text::LOBBY);
                session.lobby.as_ref().map(|l| l.room.title.clone()).unwrap_or_default()
            } else {
                rich_in(h, f, text::GAME_SUMMARY, 15.0, INK, true);
                String::new()
            };
            let t = rich_in(h, f, &title, 15.0, MUTED, true);
            h.commands().entity(t).insert(TextLayout::no_wrap());
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
            let dim = (round && st.is_none_or(|s| s.part == Part::Out)) || (!pl.connected && !pl.bot);
            let me = session.me == Some(pl.id);
            p.spawn((
                Node {
                    column_gap: px(8),
                    align_items: AlignItems::Center,
                    height: px(31),
                    padding: UiRect::axes(px(4), px(0)),
                    border_radius: BorderRadius::all(px(9)),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(if me { APRICOT_SOFT } else { Color::NONE }),
            ))
            .with_children(|r| {
                if !lobby {
                    place_badge(r, f, rank + 1, 22.0, true);
                }
                r.spawn(Node {
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    min_width: px(0),
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|t| {
                    let ink = if dim {
                        GHOST
                    } else if me {
                        hex(0x5E331B)
                    } else {
                        INK
                    };
                    let star = if Some(pl.id) == host { " ⭐" } else { "" };
                    let n = rich_in(t, f, &format!("{}{star}", pl.name), 14.5, ink, me);
                    t.commands().entity(n).insert(TextLayout::no_wrap());
                });
                let cell = |r: &mut ChildSpawnerCommands, w: f32, s: &str, size: f32, ink: Color, strong: bool| {
                    r.spawn(Node {
                        width: px(w),
                        justify_content: JustifyContent::FlexEnd,
                        flex_shrink: 0.0,
                        ..default()
                    })
                    .with_children(|c| {
                        let t = rich_in(c, f, s, size, ink, strong);
                        c.commands().entity(t).insert(TextLayout::no_wrap());
                    });
                };
                let bells = session.scores.get(&pl.id).copied().unwrap_or(0);
                if lobby {
                    cell(r, 40.0, &format!("👑{}", pl.crowns), 13.0, MUTED, false);
                    cell(r, 44.0, &format!("🔔{bells}"), 13.0, MUTED, false);
                } else {
                    cell(r, 40.0, &icon, 13.0, MUTED, false);
                    if round && genre_points {
                        cell(r, 28.0, &bells.to_string(), 14.5, GOOD, true);
                    } else {
                        let ink = if dim { GHOST } else { INK };
                        cell(r, 30.0, &pl.score.to_string(), 14.5, ink, true);
                    }
                }
                let ping = if pl.bot {
                    text::BOT.to_string()
                } else if pl.connected {
                    text::ping(pl.ping)
                } else {
                    "—".into()
                };
                cell(r, 44.0, &ping, 12.5, FAINT, false);
            });
        }
    });
}

/// The advice after a fallback: its card and its two texts.
#[derive(SystemParam)]
struct VpnAdvice<'w, 's> {
    node: Single<'w, 's, &'static mut Node, With<VpnBox>>,
    lines: Query<'w, 's, (&'static mut Rich, Has<VpnHead>), VpnText>,
    net: Query<'w, 's, &'static mut Text, With<NetLine>>,
}

type VpnText = Or<(With<VpnHead>, With<VpnMore>)>;

/// Under the panel: transport and ping (and frames a second); after a fallback, the VPN hint.
fn net_line(
    mut vpn: VpnAdvice,
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
    let Ok(mut t) = vpn.net.single_mut() else { return };
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
    let hint = crate::diag::vpn_hint(&conn, now);
    show(&mut vpn.node, hint.is_some());
    if let Some(hint) = hint {
        let (head, more) = hint.split_once('\n').unwrap_or((hint, ""));
        for (mut r, is_head) in &mut vpn.lines {
            r.set(if is_head { head } else { more });
        }
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
            column_gap: px(5),
            align_items: AlignItems::Center,
            flex_wrap: FlexWrap::Wrap,
            padding: UiRect::axes(px(12), px(6)),
            border_radius: BorderRadius::all(px(12)),
            // (A note may carry a long path: an F8 report's.)
            max_width: percent(100),
            ..default()
        },
        BackgroundColor(Color::srgba(0.984, 0.973, 0.953, 0.95)),
        BoxShadow::new(SHADOW.with_alpha(0.1), px(0), px(1), px(0), px(4)),
        Motion::slide(40.0, 0.0),
    ))
    .with_children(|r| match &x.what {
        Feed::Note(s) => {
            let t = rich(r, f, s, 14.5, INK);
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
            let ink = if *out { CRITICAL } else { INK };
            let by = by.filter(|b| b != victim).map(|b| format!("{} ", who(session, b)));
            let victim = who(session, *victim);
            let why = format!(" — {}", text::ko_line(why, *out, *shortcut));
            let icon = format!("{icon} ");
            let mut parts = Vec::new();
            if let Some(b) = &by {
                parts.push((b.as_str(), true, ink));
            }
            parts.extend([
                (icon.as_str(), false, INK),
                (victim.as_str(), true, ink),
                (why.as_str(), false, ink),
            ]);
            line_of(r, f, &parts, 14.5);
        }
    });
}

/// The feed gives way to the game's summary.
fn feed_box(outcome: Res<Outcome>, mut q: Single<&mut Node, With<FeedBox>>) {
    show(&mut q, outcome.game_end.is_none() && outcome.results.is_none());
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
                width: percent(100),
                flex_direction: FlexDirection::Column,
                padding: UiRect::new(px(36), px(36), px(30), px(32)),
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            panel(),
            Motion::slide(-40.0, 0.0).lasting(0.45),
        ))
        .with_children(|c| {
            row(c, false, |r| {
                let here = r.target_entity();
                r.commands().entity(here).insert(Node {
                    column_gap: px(10),
                    align_items: AlignItems::Center,
                    ..default()
                });
                if info.practice {
                    genre_tag(r, f, Some(m.genre), text::PRACTICE_TITLE);
                } else {
                    genre_tag(r, f, Some(m.genre), text::genre(m.genre));
                    rich_in(r, f, &text::round_of(info.index, info.total), 16.0, MUTED, true);
                }
                spacer(r);
                r.spawn((
                    Node {
                        padding: UiRect::axes(px(10), px(3)),
                        border_radius: BorderRadius::MAX,
                        flex_shrink: 0.0,
                        ..default()
                    },
                    BackgroundColor(CARD2),
                ))
                .with_children(|b| {
                    let t = rich_in(b, f, "", 13.0, MUTED, true);
                    b.commands().entity(t).insert(IntroLeft);
                });
            });
            let t = big(c, f, m.title, 52.0, INK);
            c.commands().entity(t).insert(Node {
                margin: UiRect::new(px(0), px(0), px(18), px(10)),
                ..default()
            });
            rich_in(c, f, &format!("🎯 {}.", m.goal), 19.0, INK, true);
            let d = rich(c, f, m.desc, 16.0, MUTED);
            c.commands().entity(d).insert(Node {
                margin: UiRect::top(px(10)),
                ..default()
            });
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

/// The intro, the timer and its bar, and the «ВПЕРЁД!» and 3-2-1 texts in their boxes.
#[derive(SystemParam)]
struct Countdown<'w, 's> {
    intro: Single<'w, 's, &'static mut Node, NodeOf<IntroBox, Pill, TimerBox, TimerFill>>,
    timer_box: Single<'w, 's, (Entity, &'static mut Node), NodeOf<TimerBox, IntroBox, Pill, TimerFill>>,
    fill:
        Single<'w, 's, (&'static mut Node, &'static mut BackgroundColor), NodeOf<TimerFill, IntroBox, Pill, TimerBox>>,
    timer: Single<'w, 's, (&'static mut Text, &'static mut TextColor), Only<TimerText, GoText, CountText>>,
    go: Single<'w, 's, &'static mut Text, Only<GoText, TimerText, CountText>>,
    count: Single<'w, 's, &'static mut Text, Only<CountText, TimerText, GoText>>,
    boxes: Query<'w, 's, (Entity, &'static mut Node, Has<GoBox>), BoxOf>,
    standings: Single<'w, 's, &'static mut Node, StandingsOnly>,
}

type StandingsOnly = (
    With<StandingsCol>,
    Without<IntroBox>,
    Without<TimerBox>,
    Without<TimerFill>,
    Without<Pill>,
    Without<GoBox>,
    Without<CountBox>,
);

type BoxOf = (
    Or<(With<GoBox>, With<CountBox>)>,
    Without<IntroBox>,
    Without<TimerBox>,
    Without<TimerFill>,
    Without<Pill>,
);

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
    // (The intro's card gives way to the standings for 3-2-1.)
    let before = in_round && round.time.t < -3.0;
    let on = in_round && round.time.t >= 0.0;
    show(&mut countdown.intro, before);
    // (And to a round's results, which tell the same and more.)
    show(&mut countdown.standings, !before && round.outcome.results.is_none());
    let (box_e, ref mut box_node) = *countdown.timer_box;
    show(box_node, on);
    let left = round.time.time_left;
    let width = percent((left / round.time.duration.max(1.0) * 100.0) as f32);
    let hurry = on && left < 10.0;
    let (ref mut fill, ref mut bar) = *countdown.fill;
    if fill.width != width {
        fill.width = width;
    }
    bar.set_if_neq(BackgroundColor(if hurry { CRITICAL } else { TEAL }));
    let (ref mut t, ref mut c) = *countdown.timer;
    let s = if on { text::fmt_time(left) } else { String::new() };
    if t.0 != s {
        t.0 = s;
    }
    c.set_if_neq(TextColor(if hurry { CRITICAL } else { INK }));
    // (Each of the last ten seconds beats.)
    let sec = left.ceil() as i64;
    if hurry && sec != *ticked && sec > 0 {
        commands.entity(box_e).insert(Motion::pop(1.1).lasting(0.3));
    }
    *ticked = sec;
    let go_s = if on && round.time.t < 1.2 { text::GO } else { "" };
    let n = (-round.time.t).ceil() as i64;
    let count_s = if round.hud.on && round.hud.kind == Some(ArenaKind::Round) && (1..=3).contains(&n) {
        n.to_string()
    } else {
        String::new()
    };
    let go_new = countdown.go.0 != go_s;
    let count_new = countdown.count.0 != count_s;
    if go_new {
        countdown.go.0 = go_s.into();
    }
    if count_new {
        countdown.count.0.clone_from(&count_s);
    }
    for (e, mut node, go) in &mut countdown.boxes {
        let (on, new) = if go {
            (!go_s.is_empty(), go_new)
        } else {
            (!count_s.is_empty(), count_new)
        };
        show(&mut node, on);
        if on && new {
            commands.entity(e).insert(if go {
                Motion::pop(0.3).lasting(0.5)
            } else {
                Motion::pop(1.6).lasting(0.45)
            });
        }
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

/// A board's head: its caption, its title, and the time to the next scene.
fn board_head(p: &mut ChildSpawnerCommands, f: &Fonts, cap: &str, title: &str, next: &'static str) {
    caption(p, f, cap);
    // (Title and time one under the other: a title wrapped beside the time is measured a line short.)
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::FlexStart,
        row_gap: px(8),
        margin: UiRect::new(px(0), px(0), px(4), px(14)),
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(|r| {
        big(r, f, title, 30.0, INK);
        r.spawn((
            Node {
                padding: UiRect::axes(px(12), px(5)),
                border_radius: BorderRadius::MAX,
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(CARD2),
        ))
        .with_children(|b| {
            let e = rich_in(b, f, "", 14.0, MUTED, true);
            b.commands().entity(e).insert(NextLine(next));
        });
    });
}

/// A board over the game.
fn board(width: f32) -> impl Bundle {
    (
        Node {
            width: px(width),
            max_width: percent(92),
            max_height: percent(100),
            flex_direction: FlexDirection::Column,
            padding: UiRect::new(px(34), px(34), px(28), px(22)),
            border_radius: BorderRadius::all(px(20)),
            overflow: Overflow::clip(),
            ..default()
        },
        panel_raised(),
    )
}

/// After a round: points won and lost, on a board at the top over the game veiled.
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
                position_type: PositionType::Absolute,
                top: px(-50),
                left: px(0),
                width: percent(100),
                height: Val::Vh(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.188, 0.157, 0.125, 0.18)),
            Pickable::IGNORE,
        ));
        p.spawn((board(820.0), Motion::slide(0.0, -32.0).lasting(0.4)))
            .with_children(|c| {
                let cap = if r.practice {
                    text::PRACTICE_SMALL.to_string()
                } else {
                    text::results_of(r.index, r.total)
                };
                board_head(c, f, &cap, title, next);
                for (i, row_) in r.rows.iter().enumerate() {
                    let me = session.me == Some(row_.id);
                    let last = i + 1 == r.rows.len();
                    let off = session.player(row_.id).is_some_and(|p| !p.connected && !p.bot);
                    let e = table_row(c, me, last, 56.0, |t| {
                        cell(t, 40.0, false, |x| place_badge(x, f, row_.place, 26.0, true));
                        t.spawn(Node {
                            flex_grow: 1.0,
                            min_width: px(0),
                            flex_direction: FlexDirection::Column,
                            overflow: Overflow::clip(),
                            ..default()
                        })
                        .with_children(|x| {
                            row(x, false, |n| {
                                let here = n.target_entity();
                                n.commands().entity(here).insert(Node {
                                    column_gap: px(0),
                                    ..default()
                                });
                                let ink = if off { GHOST } else { INK };
                                rich_in(n, f, &session.name_of(row_.id), 16.0, ink, true);
                                if me {
                                    rich(n, f, text::YOU, 16.0, FAINT);
                                }
                            });
                            rich(
                                x,
                                f,
                                &text::round_note(row_.note),
                                13.5,
                                if off { GHOST } else { FAINT },
                            );
                        });
                        cell(t, 54.0, true, |x| {
                            if row_.points > 0 {
                                rich_in(x, f, &format!("+{}", row_.points), 16.0, GOOD, true);
                            }
                        });
                        cell(t, 54.0, true, |x| {
                            if row_.penalty > 0 {
                                rich_in(x, f, &format!("−{}", row_.penalty), 16.0, CRITICAL, true);
                            }
                        });
                        cell(t, 60.0, false, |x| {
                            let (fill, ink) = match row_.delta {
                                d if d > 0 => (GOOD_SOFT, GOOD),
                                d if d < 0 => (CRITICAL_SOFT, CRITICAL),
                                _ => (Color::NONE, GHOST),
                            };
                            x.spawn((
                                Node {
                                    padding: UiRect::axes(px(7), px(2)),
                                    border_radius: BorderRadius::all(px(8)),
                                    ..default()
                                },
                                BackgroundColor(fill),
                            ))
                            .with_children(|b| {
                                let s = if row_.delta == 0 {
                                    "—".into()
                                } else {
                                    text::signed(row_.delta)
                                };
                                rich_in(b, f, &s, 13.0, ink, true);
                            });
                        });
                        cell(t, 62.0, true, |x| {
                            rich_in(x, f, &row_.total.to_string(), 19.0, INK, true);
                        });
                    });
                    c.commands()
                        .entity(e)
                        .insert(Motion::slide(-24.0, 0.0).after(0.15 + i as f32 * 0.05));
                }
            });
    });
}

/// A row of a board's table: lit if it is the player's, over a rule unless it is the last.
fn table_row(
    p: &mut ChildSpawnerCommands,
    me: bool,
    last: bool,
    high: f32,
    f: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    p.spawn((
        Node {
            column_gap: px(6),
            align_items: AlignItems::Center,
            min_height: px(high),
            padding: UiRect::axes(px(if me { 8 } else { 0 }), px(0)),
            margin: UiRect::axes(px(if me { -8 } else { 0 }), px(0)),
            border: UiRect::bottom(px(if me || last { 0 } else { 1 })),
            border_radius: BorderRadius::all(px(if me { 12 } else { 0 })),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(if me { APRICOT_SOFT } else { Color::NONE }),
        BorderColor::all(HAIR),
    ))
    .with_children(f)
    .id()
}

/// A cell of a table `w` px wide, its content at its end or in its middle.
fn cell(p: &mut ChildSpawnerCommands, w: f32, end: bool, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        width: px(w),
        justify_content: if end {
            JustifyContent::FlexEnd
        } else {
            JustifyContent::Center
        },
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(f);
}

/// The end of the game, beside the podium on the field: who won, the final table and the titles.
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
        p.spawn((board(720.0), Motion::slide(48.0, 0.0).lasting(0.45)))
            .with_children(|c| {
                let title = match g.standings.first() {
                    Some(w) if session.me == Some(w.id) => format!("👑 {}", text::YOU_WON),
                    Some(w) => format!("👑 {}", text::wins(&w.name)),
                    None => text::GAME_OVER.to_string(),
                };
                board_head(c, f, text::GAME_SUMMARY, &title, text::BACK_TO_LOBBY);
                c.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    overflow: Overflow::scroll_y(),
                    flex_shrink: 1.0,
                    min_height: px(0),
                    ..default()
                })
                .insert(bevy::ui_widgets::ScrollArea)
                .with_children(|b| {
                    let h = subheading(b, f, text::STANDINGS);
                    b.commands().entity(h).insert(Node {
                        margin: UiRect::bottom(px(6)),
                        ..default()
                    });
                    let half = g.standings.len().div_ceil(2);
                    b.spawn(Node {
                        column_gap: px(24),
                        flex_shrink: 0.0,
                        ..default()
                    })
                    .with_children(|cols| {
                        for part in [&g.standings[..half], &g.standings[half..]] {
                            cols.spawn(Node {
                                flex_direction: FlexDirection::Column,
                                flex_grow: 1.0,
                                flex_basis: px(0),
                                min_width: px(0),
                                ..default()
                            })
                            .with_children(|t| {
                                for s in part {
                                    let me = session.me == Some(s.id);
                                    table_row(t, me, false, 40.0, |r| {
                                        let here = r.target_entity();
                                        r.commands().entity(here).insert(Node {
                                            column_gap: px(10),
                                            align_items: AlignItems::Center,
                                            min_height: px(40),
                                            padding: UiRect::axes(px(if me { 8 } else { 0 }), px(0)),
                                            margin: UiRect::axes(px(if me { -8 } else { 0 }), px(0)),
                                            border: UiRect::bottom(px(if me { 0 } else { 1 })),
                                            border_radius: BorderRadius::all(px(if me { 10 } else { 0 })),
                                            ..default()
                                        });
                                        place_badge(r, f, s.place as usize, 26.0, true);
                                        let n = rich_in(r, f, &s.name, 16.0, INK, true);
                                        r.commands().entity(n).insert((
                                            Node {
                                                flex_grow: 1.0,
                                                flex_shrink: 1.0,
                                                min_width: px(0),
                                                ..default()
                                            },
                                            TextLayout::no_wrap(),
                                        ));
                                        let w = rich(r, f, &format!("🏆{}", s.wins), 14.0, FAINT);
                                        r.commands().entity(w).insert(TextLayout::no_wrap());
                                        cell(r, 34.0, true, |x| {
                                            rich_in(x, f, &s.total.to_string(), 16.0, INK, true);
                                        });
                                    });
                                }
                            });
                        }
                    });
                    if !g.awards.is_empty() {
                        let h = subheading(b, f, text::AWARDS);
                        b.commands().entity(h).insert(Node {
                            margin: UiRect::new(px(0), px(0), px(22), px(10)),
                            ..default()
                        });
                    }
                    b.spawn(Node {
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: px(10),
                        row_gap: px(10),
                        flex_shrink: 0.0,
                        ..default()
                    })
                    .with_children(|grid| {
                        for (i, a) in g.awards.iter().enumerate() {
                            let (icon, title, what) = text::award(a);
                            grid.spawn((
                                Node {
                                    width: percent(31.5),
                                    min_width: px(170),
                                    flex_direction: FlexDirection::Column,
                                    row_gap: px(3),
                                    padding: UiRect::axes(px(14), px(12)),
                                    border_radius: BorderRadius::all(px(14)),
                                    ..default()
                                },
                                BackgroundColor(CARD2),
                                Motion::slide(24.0, 0.0).after(0.6 + i as f32 * 0.08),
                            ))
                            .with_children(|r| {
                                rich_in(r, f, &format!("{icon} {title}"), 15.0, INK, true);
                                rich_in(r, f, &session.name_of(a.id), 14.0, INK, true);
                                rich(r, f, &what, 13.0, FAINT);
                            });
                        }
                    });
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
        let (line, ink) = match hud.status {
            Part::Finished => (text::finished(hud.place), INK),
            Part::Out => (text::YOU_ARE_OUT.to_string(), CRITICAL),
            _ => (text::SPECTATOR.to_string(), INK),
        };
        p.spawn((
            Node {
                min_width: px(420),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(px(26), px(14)),
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            panel(),
            Motion::pop(0.85),
        ))
        .with_children(|r| {
            big(r, f, &line, 24.0, ink);
            r.spawn(Node {
                column_gap: px(12),
                align_items: AlignItems::Center,
                margin: UiRect::top(px(6)),
                ..default()
            })
            .with_children(|x| {
                let arrow = |x: &mut ChildSpawnerCommands, s: &str| {
                    x.spawn((
                        Node {
                            width: px(30),
                            height: px(30),
                            border_radius: BorderRadius::all(px(10)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(CARD2),
                    ))
                    .with_children(|a| {
                        rich_in(a, f, s, 18.0, MUTED, true);
                    });
                };
                arrow(x, "‹");
                rich_in(x, f, &text::camera_on(hud.spectating.as_deref()), 16.0, MUTED, true);
                arrow(x, "›");
            });
            let h = rich(r, f, text::SPECTATE_HINT, 13.0, FAINT);
            r.commands().entity(h).insert(Node {
                margin: UiRect::top(px(6)),
                ..default()
            });
        });
    });
}

/// In play but the mouse is free: ask for a click. In the lobby with the menu closed: how to open it and the chat.
fn prompt(
    ui: Res<Ui>,
    hud: Res<Hud>,
    session: Res<Session>,
    mut q: Single<&mut Node, (With<PromptBox>, Without<LobbyHint>)>,
    mut hint: Single<&mut Node, (With<LobbyHint>, Without<PromptBox>)>,
) {
    let ask = hud.on && ui.need_click && !ui.menu;
    show(&mut q, ask);
    let lobby = hud.on && hud.kind == Some(ArenaKind::Lobby) && !session.practice;
    show(&mut hint, lobby && !ui.menu && !ui.chat && !ask);
}
