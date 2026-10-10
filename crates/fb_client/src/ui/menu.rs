//! The Esc menu: the room (players, access, the host's game setup), the player's name
//! and look, the options, dev tools; and who has the mouse: the game (captured) or the interface.
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::ecs::system::SystemParam;
use bevy::input_focus::InputFocus;
use bevy::picking::events::{Pointer, Press};
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::InteractionDisabled;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused};
use fb_arena::ArenaKind;
use fb_maps::director::ROUND_COUNTS;
use fb_proto::{ClientMsg, DevCmd, Goto, Lobby, Mode, Phase, PlayerId, Playlist};
use fb_shared::COLORS;
use fb_shared::outfit::{GLASSES, HATS, Hat, Tint};
use lightyear::prelude::client::Client;
use lightyear::prelude::*;

use super::home::{head, name_row, practice_list};
use super::*;
use crate::game::{Buttons, Gate};
use crate::session::Session;
use crate::settings::{Me, Player, Profile};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<FieldClick>();
        app.add_observer(on_press);
        app.add_systems(Startup, build_menu.after(super::setup));
        app.add_systems(
            Update,
            (
                (adopt_name, menu_flow, super::rebind, sync_names, gate).chain(),
                (
                    parts.run_if(state_changed::<MenuTab>.or_else(resource_changed::<Session>)),
                    top.run_if(resource_changed::<Session>),
                    (swatches, players, host_setup, phase_line).run_if(resource_changed::<Session>),
                    outfit.run_if(resource_changed::<Player>),
                    dev,
                ),
                menu_actions,
            )
                .chain(),
        );
    }
}

/// A press on the game field: on nothing, or on the HUD and name tags over it.
#[derive(Message)]
pub(super) struct FieldClick;

/// Presses bubble from what they hit up to the window: one that started on a panel is not the field's.
fn on_press(
    ev: On<Pointer<Press>>,
    windows: Query<(), With<Window>>,
    parents: Query<&ChildOf>,
    layers: Query<&Layer>,
    mut out: MessageWriter<FieldClick>,
) {
    if !windows.contains(ev.entity) {
        return;
    }
    let hit = ev.original_event_target();
    let on_panel = core::iter::once(hit)
        .chain(parents.iter_ancestors(hit))
        .any(|e| layers.get(e).is_ok_and(|l| !matches!(l, Layer::Hud | Layer::Tags)));
    if !on_panel {
        out.write(FieldClick);
    }
}

/// The menu's parts, each shown on its tab.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Part {
    Top,
    Name,
    Body,
    Settings,
    Dev,
}

/// The way back from practice, and the room's name, way out and access.
#[derive(Component)]
struct PracticeTop;
#[derive(Component)]
struct RoomTop;
#[derive(Component)]
struct RoomTitle;
#[derive(Component)]
struct PrivateNote;

/// The lobby's part of the body (the colours, who is here, the setup, practice), and the line of a game under way.
#[derive(Component)]
struct LobbyPart;
#[derive(Component)]
struct Swatches;
#[derive(Component)]
struct OutfitBox;
#[derive(Component)]
struct PlayersBox;
#[derive(Component)]
struct HostBox;
#[derive(Component)]
struct PhaseBox;
#[derive(Component)]
struct DevBox;

fn build_menu(mut commands: Commands, layers: Res<Layers>, f: Res<Fonts>, me: Me, options: Options, folds: Res<Folds>) {
    let f = &*f;
    let e = layers[Layer::Menu];
    let col = |gap: f32| Node {
        flex_direction: FlexDirection::Column,
        row_gap: rem(gap),
        ..default()
    };
    commands.entity(e).with_children(|l| {
        // A sheet docked on the right, over the game in full view.
        l.spawn((
            Node {
                position_type: PositionType::Absolute,
                right: rem(1.0),
                top: rem(1.0),
                bottom: rem(1.0),
                width: rem(29.0),
                max_width: percent(60),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|w| {
            w.spawn((
                Node {
                    width: percent(100),
                    height: percent(100),
                    flex_direction: FlexDirection::Column,
                    row_gap: rem(0.875),
                    padding: UiRect::all(rem(1.25)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(rem(1.5)),
                    ..default()
                },
                glass(),
            ))
            .with_children(|m| {
                head(
                    m,
                    f,
                    true,
                    &[
                        (text::TAB_GAME, Action::MenuTab(MenuTab::Game)),
                        (text::TAB_SETTINGS, Action::MenuTab(MenuTab::Settings)),
                        (text::TAB_DEV, Action::MenuTab(MenuTab::Dev)),
                    ],
                );
                m.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: rem(0.625),
                        overflow: Overflow::scroll_y(),
                        flex_grow: 1.0,
                        flex_shrink: 1.0,
                        ..default()
                    },
                    bevy::ui_widgets::ScrollArea,
                ))
                .with_children(|b| {
                    let tab = || motion::Reveal::new(motion::Motion::slide(0.0, 12.0));
                    b.spawn((Part::Top, col(0.625), tab())).with_children(|t| {
                        t.spawn((PracticeTop, col(0.0))).with_children(|p| {
                            group(p, |g| {
                                // (The first text in it: `top` writes the map's line there.)
                                rich_in(g, f, "", 15.0, INK, true);
                                button(g, f, text::PRACTICE_BACK, Look::Plain, Action::EndPractice);
                            });
                        });
                        t.spawn((RoomTop, col(0.0))).with_children(|r| {
                            section(r, f, text::SECTION_ROOM, |g| {
                                row(g, false, |r| {
                                    r.spawn(Node {
                                        flex_grow: 1.0,
                                        flex_shrink: 1.0,
                                        min_width: px(0),
                                        ..default()
                                    })
                                    .with_children(|t| {
                                        let title = big(t, f, "", 19.0, INK);
                                        t.commands().entity(title).insert(RoomTitle);
                                    });
                                    button(r, f, text::LEAVE_ROOM, Look::TinyDanger, Action::LeaveRoom);
                                });
                                button(
                                    g,
                                    f,
                                    text::PRIVATE_ROOM,
                                    Look::Check(false),
                                    Action::Send(ClientMsg::Access { private: true }),
                                );
                                let note = muted(g, f, text::PRIVATE_NOTE);
                                g.commands().entity(note).insert(PrivateNote);
                            });
                        });
                    });
                    b.spawn((Part::Name, col(0.625), tab())).with_children(|n| {
                        section(n, f, text::SECTION_BEAN, |g| {
                            name_row(g, f, Field::MenuName, &me.name());
                            g.spawn((
                                Swatches,
                                Node {
                                    flex_wrap: FlexWrap::Wrap,
                                    column_gap: rem(0.5),
                                    row_gap: rem(0.5),
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                            ))
                            .with_children(|r| {
                                for i in 0..COLORS.len() as u8 {
                                    swatch(r, suit(i), false, Action::Color(i), true, 1.75);
                                }
                            });
                            g.spawn((OutfitBox, col(0.625)));
                        });
                    });
                    b.spawn((Part::Body, col(1.0), tab())).with_children(|p| {
                        p.spawn((PhaseBox, col(0.625)));
                        p.spawn((LobbyPart, col(1.0))).with_children(|l| {
                            l.spawn((PlayersBox, col(0.375)));
                            l.spawn((HostBox, col(0.625)));
                            practice_list(l, f, &folds);
                        });
                    });
                    b.spawn((Part::Settings, col(0.625), tab()))
                        .with_children(|s| settings_tab(s, f, &options, &folds));
                    b.spawn((Part::Dev, col(0.625), tab())).with_children(|d| {
                        d.spawn((DevBox, col(0.625)));
                    });
                });
                m.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: rem(0.375),
                        padding: UiRect::top(rem(0.875)),
                        border: UiRect::top(px(1)),
                        ..default()
                    },
                    BorderColor::all(RIM),
                ))
                .with_children(|r| {
                    button(r, f, text::RESUME, Look::Primary, Action::Resume);
                    r.spawn(Node {
                        justify_content: JustifyContent::Center,
                        ..default()
                    })
                    .with_children(|h| {
                        rich(h, f, text::RESUME_HINT.trim_start_matches("· "), 12.0, FAINT);
                    });
                });
            });
        });
    });
}

fn capture(cursor: &mut Mut<CursorOptions>, on: bool) {
    let mode = if !on {
        CursorGrabMode::None
    } else if cfg!(target_os = "windows") {
        // Locked (pointer constraints) where the platform has it; Windows only confines.
        CursorGrabMode::Confined
    } else {
        CursorGrabMode::Locked
    };
    if cursor.grab_mode != mode {
        cursor.grab_mode = mode;
        cursor.visible = !on;
    }
}

/// Mouse capture: the menu opens in a room on Esc and closes on a round's start or a click on the field.
#[derive(SystemParam)]
pub(super) struct MainWindow<'w, 's> {
    cursor: Query<'w, 's, &'static mut CursorOptions, With<PrimaryWindow>>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    focus_events: MessageReader<'w, 's, WindowFocused>,
}

/// Whether a text field has the keyboard.
#[derive(SystemParam)]
pub(super) struct Typing<'w, 's> {
    focus: Res<'w, InputFocus>,
    fields: Query<'w, 's, (), With<EditableText>>,
}

impl Typing<'_, '_> {
    pub(super) fn typing(&self) -> bool {
        self.focus.get().is_some_and(|e| self.fields.contains(e))
    }
}

pub(super) fn menu_flow(
    mut res: ResMut<Ui>,
    session: Res<Session>,
    buttons: Buttons,
    mut window: MainWindow,
    typing: Typing,
    mut clicks: MessageReader<FieldClick>,
    mut seen: Local<(u32, Option<u32>)>,
) {
    let Ok(mut cursor) = window.cursor.single_mut() else {
        return;
    };
    // (Worked out on a copy: `Ui` reads as changed only when it is.)
    let mut ui = res.clone();
    let in_room = session.room.is_some() && session.arena.is_some();
    let clicked = clicks.read().count() > 0;
    let lost_focus = window.focus_events.read().any(|e| !e.focused);
    if in_room {
        let captured = cursor.grab_mode != CursorGrabMode::None;
        let was_menu = ui.menu;
        if seen.0 != session.entries {
            seen.0 = session.entries;
            ui.menu = true;
        }
        let arena = session.arena.as_ref().map(|a| a.id);
        if seen.1 != arena {
            seen.1 = arena;
            if let Some(a) = &session.arena
                && a.kind != ArenaKind::Lobby
            {
                ui.menu = false;
                if a.kind == ArenaKind::Round && !a.late {
                    capture(&mut cursor, true);
                }
            }
        }
        let typing = typing.typing();
        let esc =
            buttons.keys.just_pressed(KeyCode::Escape) && !ui.chat && ui.rebinding.is_none() && !(typing && !ui.menu);
        // (A pad keeps reporting while another window has the focus.)
        let start =
            window_focused(&window.windows) && buttons.pads.iter().any(|p| p.just_pressed(GamepadButton::Start));
        if esc || start {
            if ui.menu {
                ui.menu = false;
                capture(&mut cursor, true);
            } else {
                ui.menu = true;
            }
        } else if clicked && !ui.chat {
            ui.menu = false;
            capture(&mut cursor, true);
        }
        if lost_focus && captured {
            capture(&mut cursor, false);
            if !ui.menu {
                ui.need_click = true;
            }
        }
        if ui.menu {
            capture(&mut cursor, false);
            ui.need_click = false;
        }
        if cursor.grab_mode != CursorGrabMode::None {
            ui.need_click = false;
        } else if !ui.menu && !ui.chat && was_menu {
            ui.need_click = true;
        }
    } else {
        ui.menu = false;
        ui.need_click = false;
        capture(&mut cursor, false);
        *seen = (session.entries, None);
    }
    res.set_if_neq(ui);
}

/// A player without a name of their own keeps the one the room gave them.
fn adopt_name(session: Res<Session>, mut player: ResMut<Player>, opts: Res<crate::opts::Opts>, mut commands: Commands) {
    if !player.name.is_empty() || opts.name.is_some() || !session.is_changed() {
        return;
    }
    if let Some(p) = session.me.and_then(|me| session.player(me)) {
        player.name = p.name.clone();
        crate::settings::save_soon(&mut commands);
    }
}

/// The name fields show the name in use each time the menu opens and whenever it changes.
fn sync_names(
    ui: Res<Ui>,
    mut fields: Query<(Entity, &Field, &mut EditableText)>,
    focus: Res<InputFocus>,
    me: Me,
    mut last: Local<(String, bool)>,
) {
    let name = me.name();
    let opened = ui.menu && !last.1;
    last.1 = ui.menu;
    if !opened && last.0 == name {
        return;
    }
    last.0 = name.clone();
    let focused = focus.get();
    for (e, f, mut t) in &mut fields {
        if matches!(f, Field::MenuName | Field::Name) && Some(e) != focused && t.value() != name.as_str() {
            set_field_text(&mut t, &name);
        }
    }
}

/// The game's window has the focus (no window: a run without one, always).
fn window_focused(windows: &Query<&Window, With<PrimaryWindow>>) -> bool {
    windows.single().ok().is_none_or(|w| w.focused)
}

/// The game gets the keys, mouse and pads only while nothing else takes them: a menu, the chat, a text field,
/// another window (pads and mouse buttons keep reporting while one has the focus).
fn gate(
    ui: Res<Ui>,
    session: Res<Session>,
    focus: Res<InputFocus>,
    fields: Query<(), With<EditableText>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut gate: ResMut<Gate>,
    warm: Option<Res<crate::render::warmup::Warmup>>,
) {
    // (Nor under the loading screen.)
    let play = session.room.is_some()
        && !ui.menu
        && !ui.chat
        && !typing(&focus, &fields)
        && window_focused(&windows)
        && !warm.is_some_and(|w| w.busy());
    if gate.play != play {
        gate.play = play;
    }
}

/// Each part on its tab; the dev tab only on a dev server.
fn parts(
    tab: Option<Res<State<MenuTab>>>,
    session: Res<Session>,
    mut parts: Query<(&Part, &mut Node)>,
    mut tabs: Query<(&Act, &mut Node), Without<Part>>,
) {
    let tab = tab.map(|t| *t.get());
    for (part, mut node) in &mut parts {
        let on = match part {
            Part::Top => tab == Some(MenuTab::Game),
            Part::Name | Part::Body => tab == Some(MenuTab::Game) && !session.practice,
            Part::Settings => tab == Some(MenuTab::Settings),
            Part::Dev => tab == Some(MenuTab::Dev) && session.dev,
        };
        show(&mut node, on);
    }
    for (act, mut node) in &mut tabs {
        if let Action::MenuTab(MenuTab::Dev) = act.0 {
            show(&mut node, session.dev);
        }
    }
}

type Tops = (Without<PracticeTop>, Without<RoomTop>, Without<PrivateNote>);
type PracticeOnly = (With<PracticeTop>, Without<RoomTop>);
type NoteOnly = (With<PrivateNote>, Without<PracticeTop>, Without<RoomTop>);

/// The menu's top: practice's, the room's with its title and the private note, and their buttons.
#[derive(SystemParam)]
struct Top<'w, 's> {
    practice: Single<'w, 's, (Entity, &'static mut Node), PracticeOnly>,
    room: Single<'w, 's, &'static mut Node, (With<RoomTop>, Without<PracticeTop>)>,
    title: Single<'w, 's, Entity, With<RoomTitle>>,
    note: Single<'w, 's, &'static mut Node, NoteOnly>,
    buttons: Query<'w, 's, (Entity, &'static mut Act, &'static mut Look, &'static mut Node), Tops>,
}

/// The room's name and the way out; for the host, private or public and the PIN. In practice: the way back.
fn top(session: Res<Session>, mut top: Top, mut labels: Labels) {
    let (practice_e, ref mut practice_node) = *top.practice;
    show(practice_node, session.practice);
    show(&mut top.room, !session.practice && session.lobby.is_some());
    if session.practice {
        let title = session
            .arena
            .as_ref()
            .map_or("", |a| fb_maps::by_id(a.game).meta().title);
        labels.set(practice_e, &text::practice_now(title));
    }
    let host = session.host();
    let private = session.lobby.as_ref().is_some_and(|l| l.room.private);
    if let Some(l) = &session.lobby {
        let lock = if private { "🔒 " } else { "" };
        if let Ok(mut t) = labels.texts.get_mut(*top.title) {
            t.set(&format!("{lock}{}", l.room.title));
        }
    }
    show(&mut top.note, !host && private);
    for (e, mut act, mut look, mut node) in &mut top.buttons {
        match act.0 {
            Action::EndPractice => {
                let back = if session.back_to.is_some() {
                    text::BACK_TO_ROOM
                } else {
                    text::PRACTICE_BACK
                };
                labels.set(e, back);
            }
            Action::Send(ClientMsg::Access { .. }) => {
                show(&mut node, host);
                look.set_if_neq(Look::Check(private));
                if !matches!(act.0, Action::Send(ClientMsg::Access { private: p }) if p != private) {
                    act.0 = Action::Send(ClientMsg::Access { private: !private });
                }
                let s = match session.lobby.as_ref().and_then(|l| l.pin.as_ref()) {
                    Some(pin) => text::pin_line(pin),
                    None => format!("{} {}", text::PRIVATE_ROOM, text::PIN_FOR_ENTRY),
                };
                labels.set(e, &s);
            }
            _ => {}
        }
    }
}

/// The suit colours (in the lobby only, one per bean): the player's marked, the others' not to be had.
fn swatches(
    session: Res<Session>,
    mut row: Single<&mut Node, With<Swatches>>,
    mut buttons: Query<(Entity, &Act, &mut Look, Has<InteractionDisabled>)>,
    mut commands: Commands,
) {
    let Some(l) = &session.lobby else { return };
    show(&mut row, l.phase == Phase::Lobby);
    let me = session.me;
    let mine = l.players.iter().find(|p| Some(p.id) == me);
    for (e, act, mut look, disabled) in &mut buttons {
        let Action::Color(i) = act.0 else { continue };
        let taken = l.players.iter().any(|p| p.color == i && Some(p.id) != me);
        look.set_if_neq(Look::Swatch(suit(i), mine.is_some_and(|m| m.color == i)));
        enable(&mut commands, e, !disabled, !taken);
    }
}

/// Who is here, in the lobby.
fn players(
    session: Res<Session>,
    q: Single<Entity, With<PlayersBox>>,
    mut lobby_part: Single<&mut Node, (With<LobbyPart>, Without<PhaseBox>)>,
    mut phase: Single<&mut Node, (With<PhaseBox>, Without<LobbyPart>)>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let lobby = session.lobby.as_ref().filter(|l| l.phase == Phase::Lobby);
    show(&mut lobby_part, lobby.is_some());
    show(&mut phase, session.lobby.is_some() && lobby.is_none());
    let Some(l) = lobby else { return };
    let f = &*f;
    let (me, host) = (session.me, session.host());
    rebuild(&mut commands, *q, |p| {
        p.spawn(Node {
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            padding: UiRect::new(rem(0.25), rem(0.25), px(0), rem(0.125)),
            ..default()
        })
        .with_children(|h| {
            caption(h, f, &text::players_of(l.players.len(), l.max));
            meter(h, l.players.len() as f32 / l.max.max(1) as f32, BLUE, rem(4.0));
        });
        for pl in &l.players {
            player_row(p, f, l, pl, me, host);
        }
    });
}

/// A game under way: which round, or the results or podium; the host may abort it.
fn phase_line(
    session: Res<Session>,
    q: Single<Entity, With<PhaseBox>>,
    f: Res<Fonts>,
    mut shown: Local<Option<(String, bool)>>,
    mut commands: Commands,
) {
    let Some(l) = session.lobby.as_ref().filter(|l| l.phase != Phase::Lobby) else {
        return;
    };
    let line = match &session.arena {
        Some(a) if a.kind == ArenaKind::Round => {
            let title = fb_maps::by_id(a.game).meta().title;
            text::round_now(a.index, a.total, title)
        }
        _ if l.phase == Phase::Podium => text::PODIUM_NOW.into(),
        _ => text::RESULTS_NOW.into(),
    };
    let next = Some((line, session.host()));
    if *shown == next {
        return;
    }
    *shown = next;
    let Some((line, host)) = &*shown else { return };
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        group(p, |g| {
            row(g, false, |r| {
                badge(r, f, text::IN_GAME, WARNING.with_alpha(0.25), INK);
                label(r, f, line);
            });
            if *host {
                button(g, f, text::ABORT, Look::Danger, Action::Send(ClientMsg::Abort));
            }
        });
    });
}

fn player_row(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    l: &Lobby,
    pl: &fb_proto::LobbyPlayer,
    me: Option<PlayerId>,
    host: bool,
) {
    let mine = Some(pl.id) == me;
    p.spawn((
        Node {
            column_gap: rem(0.75),
            align_items: AlignItems::Center,
            padding: UiRect::axes(rem(0.625), rem(0.5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(0.875)),
            ..default()
        },
        BackgroundColor(ink_wash(if mine { 0.08 } else { 0.035 })),
        BorderColor::all(if mine { BLUE.with_alpha(0.5) } else { Color::NONE }),
    ))
    .with_children(|r| {
        avatar(r, f, &pl.name, suit(pl.color), 2.25);
        r.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(0),
            ..default()
        })
        .with_children(|n| {
            let you = if mine { text::YOU } else { "" };
            let live = pl.connected || pl.bot;
            rich_in(
                n,
                f,
                &format!("{}{you}", pl.name),
                14.5,
                if live { INK } else { MUTED },
                true,
            );
            let line = if pl.bot {
                text::BOT.to_string()
            } else if pl.connected {
                text::ping(pl.ping)
            } else {
                text::NO_LINK.into()
            };
            rich(n, f, &line, 12.0, if live { MUTED } else { CRITICAL });
        });
        if Some(pl.id) == l.host {
            badge(r, f, "⭐", GOLD.with_alpha(0.2), INK);
        }
        if pl.crowns > 0 {
            badge(r, f, &format!("👑 {}", pl.crowns), GOLD.with_alpha(0.2), INK);
        }
        // (With "fill with bots" on, a bot taken out would be replaced at once.)
        if pl.bot && host && !l.fill {
            button(r, f, "×", Look::TinyDanger, Action::Send(ClientMsg::RemoveBot(pl.id)));
        }
        if !pl.bot && host && !mine && pl.connected {
            button(r, f, text::GIVE_HOST, Look::Tiny, Action::Send(ClientMsg::Host(pl.id)));
        }
    });
}

/// The host's setup, or the line that the host starts the game.
fn host_setup(session: Res<Session>, q: Single<Entity, With<HostBox>>, f: Res<Fonts>, mut commands: Commands) {
    let Some(l) = session.lobby.as_ref().filter(|l| l.phase == Phase::Lobby) else {
        return;
    };
    let f = &*f;
    let host = session.host();
    rebuild(&mut commands, *q, |p| {
        if host {
            setup_of(p, f, l);
        } else {
            group(p, |g| {
                row(g, false, |r| {
                    spinner(r, 1.0, BLUE);
                    label(r, f, text::HOST_STARTS);
                });
            });
        }
    });
}

/// The host's part of the lobby: which maps, how many rounds, bots, and the start button.
fn setup_of(p: &mut ChildSpawnerCommands, f: &Fonts, l: &Lobby) {
    let pl = &l.playlist;
    let ready = l.players.iter().filter(|p| p.connected || p.bot).count() as u32;
    let enough = ready >= l.min;
    let with = |patch: &dyn Fn(&mut Playlist)| {
        let mut next = pl.clone();
        patch(&mut next);
        Action::Send(ClientMsg::Playlist(next))
    };
    section(p, f, text::SECTION_GAME, |g| {
        row(g, true, |r| {
            for (m, s) in [
                (Mode::Mix, text::MODE_MIX),
                (Mode::Races, text::MODE_RACES),
                (Mode::Survival, text::MODE_SURVIVAL),
                (Mode::Custom, text::MODE_CUSTOM),
            ] {
                button(r, f, s, Look::Chip(pl.mode == m), with(&|p| p.mode = m));
            }
        });
        if pl.mode == Mode::Custom {
            row(g, true, |r| {
                for def in fb_maps::GAMES {
                    let m = def.meta();
                    let at = pl.games.iter().position(|&g| g == m.id);
                    let s = match at {
                        Some(i) => format!("{}. {}", i + 1, m.title),
                        None => m.title.to_string(),
                    };
                    let id = m.id;
                    let act = with(&|p| {
                        if let Some(i) = p.games.iter().position(|&g| g == id) {
                            p.games.remove(i);
                        } else if p.games.len() < 12 {
                            p.games.push(id);
                        }
                    });
                    button(r, f, &s, Look::Chip(at.is_some()), act);
                }
            });
        } else {
            row(g, true, |r| {
                label(r, f, text::ROUNDS);
                for n in ROUND_COUNTS {
                    button(
                        r,
                        f,
                        &n.to_string(),
                        Look::Chip(pl.rounds == n),
                        with(&|p| p.rounds = n),
                    );
                }
            });
        }
        button(
            g,
            f,
            text::FILL_BOTS,
            Look::Check(l.fill),
            Action::Send(ClientMsg::Fill(!l.fill)),
        );
        let room = !l.fill && (l.players.len() as u32) < l.max;
        row(g, false, |r| {
            button_if(r, f, text::ADD_BOT, Look::Plain, Action::Send(ClientMsg::AddBot), room);
        });
        let s = if enough {
            text::START_GAME.to_string()
        } else {
            text::need_players(l.min)
        };
        button_if(g, f, &s, Look::Go, Action::Send(ClientMsg::Start), enough);
    });
}

/// The player's look: hat, glasses and colours of the hat, belly and shoes (each choice carries the whole outfit:
/// redrawn when it changes).
fn outfit(
    player: Res<Player>,
    folds: Res<Folds>,
    q: Single<Entity, With<OutfitBox>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    let outfit = player.outfit();
    rebuild(&mut commands, *q, |p| outfit_picker(p, f, &folds, &outfit));
}

fn outfit_picker(p: &mut ChildSpawnerCommands, f: &Fonts, folds: &Folds, outfit: &fb_proto::Outfit) {
    fold(p, f, folds, Fold::Outfit, text::OUTFIT, |c| {
        let wear = |patch: &dyn Fn(&mut fb_proto::Outfit)| {
            let mut o = *outfit;
            patch(&mut o);
            Action::Wear(o)
        };
        row(c, true, |r| {
            for h in HATS {
                button(r, f, text::hat(h), Look::Chip(outfit.hat == h), wear(&|o| o.hat = h));
            }
        });
        if outfit.hat != Hat::None {
            tints(c, f, text::HAT_COLOR, outfit.hat_color, &|t| wear(&|o| o.hat_color = t));
        }
        row(c, true, |r| {
            for g in GLASSES {
                button(
                    r,
                    f,
                    text::glasses(g),
                    Look::Chip(outfit.glasses == g),
                    wear(&|o| o.glasses = g),
                );
            }
        });
        tints(c, f, text::BELLY, outfit.belly, &|t| wear(&|o| o.belly = t));
        tints(c, f, text::SHOES, outfit.shoes, &|t| wear(&|o| o.shoes = t));
        row(c, false, |r| {
            button(r, f, text::RANDOM, Look::Tiny, Action::RandomOutfit);
            button(r, f, text::RESET, Look::Tiny, Action::Wear(fb_proto::Outfit::default()));
        });
    });
}

fn tints(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    title: &str,
    now: Option<Tint>,
    act: &dyn Fn(Option<Tint>) -> Action,
) {
    stack(p, |c| {
        caption(c, f, title);
        row(c, true, |r| {
            // "As designed": the part's own colour.
            swatch(r, Color::srgb(0.85, 0.86, 0.89), now.is_none(), act(None), true, 1.375);
            for t in Tint::ALL {
                swatch(r, color(t.rgb()), now == Some(t), act(Some(t)), true, 1.375);
            }
        });
    });
}

const RATES: [f64; 7] = [0.0, 0.1, 0.25, 0.5, 1.0, 2.0, 4.0];

/// Dev tools (a server started with `--dev`): time, quick games, teleports, bots, forced hits.
/// (Redrawn when the map's checkpoints and finish, its teleports, change.)
fn dev(
    q: Single<Entity, With<DevBox>>,
    session: Res<Session>,
    map: Option<Res<crate::game::Map>>,
    folds: Res<Folds>,
    f: Res<Fonts>,
    mut shown: Local<Option<(bool, usize, bool)>>,
    mut commands: Commands,
) {
    let checkpoints = map.as_ref().map_or(0, |m| m.spec.checkpoints.len());
    let finish = map.as_ref().is_some_and(|m| m.spec.finish.is_some());
    let next = Some((session.dev, checkpoints, finish));
    if *shown == next {
        return;
    }
    *shown = next;
    let f = &*f;
    let d = |cmd: DevCmd| Action::Send(ClientMsg::Dev { q: None, cmd });
    rebuild(&mut commands, *q, |p| {
        if !session.dev {
            return;
        }
        row(p, true, |r| {
            label(r, f, "Время:");
            for k in RATES {
                let s = if k == 0.0 { "⏸".to_string() } else { format!("×{k}") };
                button(r, f, &s, Look::Chip(false), d(DevCmd::Rate { k }));
            }
            button(r, f, "+1 тик", Look::Chip(false), d(DevCmd::Step { ticks: 1 }));
            button(r, f, "+0,5 с", Look::Chip(false), d(DevCmd::Step { ticks: 60 }));
        });
        row(p, true, |r| {
            button(r, f, "Пропустить заставку", Look::Chip(false), d(DevCmd::SkipIntro));
            button(r, f, "+10 с", Look::Chip(false), d(DevCmd::Warp { s: 10.0 }));
            button(r, f, "Завершить раунд", Look::Chip(false), d(DevCmd::EndRound));
            button(r, f, "В лобби", Look::Chip(false), d(DevCmd::Lobby));
        });
        row(p, true, |r| {
            label(r, f, "Телепорт:");
            button(
                r,
                f,
                "старт",
                Look::Chip(false),
                d(DevCmd::Goto {
                    id: None,
                    to: Goto::Spawn,
                }),
            );
            for i in 0..checkpoints {
                let to = Goto::Checkpoint(i as u32);
                button(
                    r,
                    f,
                    &format!("КТ {i}"),
                    Look::Chip(false),
                    d(DevCmd::Goto { id: None, to }),
                );
            }
            if finish {
                button(
                    r,
                    f,
                    "финиш",
                    Look::Chip(false),
                    d(DevCmd::Goto {
                        id: None,
                        to: Goto::Finish,
                    }),
                );
            }
        });
        row(p, true, |r| {
            button(
                r,
                f,
                "+ бот рядом",
                Look::Chip(false),
                d(DevCmd::Bot { n: None, near: true }),
            );
            button(
                r,
                f,
                "Заморозить ботов",
                Look::Chip(false),
                d(DevCmd::Bots { on: false }),
            );
            button(
                r,
                f,
                "Разморозить ботов",
                Look::Chip(false),
                d(DevCmd::Bots { on: true }),
            );
            button(
                r,
                f,
                "Сбить меня",
                Look::Chip(false),
                d(DevCmd::Knock {
                    id: None,
                    v: [0.0, 6.0, 8.0],
                }),
            );
            button(r, f, "Выбыть", Look::Chip(false), d(DevCmd::Kill { id: None }));
        });
        fold(p, f, &folds, Fold::DevMaps, text::PLAY_MAP, |c| {
            row(c, true, |r| {
                for def in fb_maps::GAMES {
                    let m = def.meta();
                    let cmd = DevCmd::Start {
                        games: vec![m.id],
                        rounds: Some(1),
                        bots: Some(3),
                    };
                    button(r, f, m.title, Look::Chip(false), d(cmd));
                }
            });
        });
    });
}

fn menu_actions(
    mut actions: MessageReader<UiAction>,
    mut ui: ResMut<Ui>,
    mut profile: Profile,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    time: Res<Time<Real>>,
    mut commands: Commands,
) {
    for UiAction(a) in actions.read() {
        match a {
            Action::Resume => {
                ui.menu = false;
                if let Ok(mut c) = cursor.single_mut() {
                    capture(&mut c, true);
                }
            }
            Action::Color(c) => {
                profile.player.color = crate::settings::ColorSetting(Some(*c));
                profile.opts.color = None;
                crate::settings::save_soon(&mut commands);
                crate::session::send(&mut senders, ClientMsg::Color(*c));
            }
            Action::Wear(o) => wear(&mut profile.player, &mut senders, &mut commands, o),
            Action::RandomOutfit => {
                // Any pick will do: the clock's nanoseconds are random enough for a hat.
                let mut x = time.elapsed().as_nanos() as u64 | 1;
                let mut pick = |n: usize| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x % n as u64) as usize
                };
                let tint = |i: usize| (i > 0).then(|| Tint::ALL[i - 1]);
                let o = fb_proto::Outfit {
                    hat: HATS[pick(HATS.len())],
                    hat_color: tint(pick(Tint::ALL.len() + 1)),
                    glasses: GLASSES[pick(GLASSES.len())],
                    belly: tint(pick(Tint::ALL.len() + 1)),
                    shoes: tint(pick(Tint::ALL.len() + 1)),
                };
                wear(&mut profile.player, &mut senders, &mut commands, &o);
            }
            _ => {}
        }
    }
}

fn wear(
    player: &mut Player,
    senders: &mut Query<&mut MessageSender<ClientMsg>, With<Client>>,
    commands: &mut Commands,
    o: &fb_proto::Outfit,
) {
    player.set_outfit(o);
    crate::settings::save_soon(commands);
    crate::session::send(senders, ClientMsg::Outfit(*o));
}
