//! The Esc menu: the room (players, access, the host's game setup), the player's name
//! and look, the options, dev tools; and who has the mouse: the game (captured) or the interface.
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::input_focus::InputFocus;
use bevy::picking::events::{Pointer, Press};
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused};
use fb_arena::ArenaKind;
use fb_maps::director::ROUND_COUNTS;
use fb_proto::{ClientMsg, DevCmd, Goto, Lobby, Mode, Phase, Pid, Playlist};
use fb_shared::COLORS;
use fb_shared::outfit::{GLASSES, HATS, Hat, TINT_LIST, Tint};
use lightyear::prelude::client::Client;
use lightyear::prelude::*;

use super::home::{name_row, practice_list, tabs};
use super::*;
use crate::game::Gate;
use crate::session::Session;
use crate::settings::{Me, Player};

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
                (head, top, body, settings, dev),
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

#[derive(Component)]
struct MenuHead;
#[derive(Component)]
struct MenuTop;
#[derive(Component)]
struct MenuNameRow;
#[derive(Component)]
struct MenuBody;
#[derive(Component)]
struct MenuSettings;
#[derive(Component)]
struct MenuDev;

fn build_menu(mut commands: Commands, layers: Query<(Entity, &Layer)>, f: Res<Fonts>, me: Me) {
    let f = &*f;
    let Some((e, _)) = layers.iter().find(|(_, l)| **l == Layer::Menu) else {
        return;
    };
    commands.entity(e).with_children(|l| {
        // A column beside the player panel, over the game in full view.
        l.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: rem(17.5),
                top: rem(0.75),
                bottom: rem(0.75),
                width: rem(28.0),
                max_width: percent(60),
                align_items: AlignItems::FlexStart,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|w| {
            w.spawn((
                Node {
                    width: percent(100),
                    max_height: percent(100),
                    flex_direction: FlexDirection::Column,
                    row_gap: rem(0.75),
                    padding: UiRect::axes(rem(1.0), rem(0.875)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(rem(1.125)),
                    ..default()
                },
                glass(),
            ))
            .with_children(|m| {
                m.spawn((MenuHead, Section::default(), Node::default()));
                m.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: rem(0.625),
                        overflow: Overflow::scroll_y(),
                        flex_shrink: 1.0,
                        ..default()
                    },
                    bevy::ui_widgets::ScrollArea,
                ))
                .with_children(|b| {
                    let col = || Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: rem(0.625),
                        ..default()
                    };
                    b.spawn((MenuTop, Section::default(), col()));
                    b.spawn((MenuNameRow, col())).with_children(|n| {
                        name_row(n, f, Field::MenuName, &me.name());
                    });
                    b.spawn((MenuBody, Section::default(), col()));
                    b.spawn((MenuSettings, Section::default(), col()));
                    b.spawn((MenuDev, Section::default(), col()));
                });
                button(
                    m,
                    f,
                    &format!("{} {}", text::RESUME, text::RESUME_HINT),
                    Look::Primary,
                    Action::Resume,
                );
            });
        });
    });
}

fn capture(cursor: &mut CursorOptions, on: bool) {
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

/// Who has the mouse, and when the menu opens and closes: it opens on entering a room and
/// with Esc; a round's start, Esc again or a click on the field close it and capture the mouse.
pub(super) fn menu_flow(
    mut ui: ResMut<Ui>,
    session: Res<Session>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut clicks: MessageReader<FieldClick>,
    mut focus_events: MessageReader<WindowFocused>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut seen: Local<(u32, Option<u32>)>,
    fields: Query<(), With<EditableText>>,
    focus: Res<InputFocus>,
) {
    let Ok(mut cursor) = cursor.single_mut() else { return };
    let in_room = session.room.is_some() && session.arena.is_some();
    let clicked = clicks.read().count() > 0;
    let lost_focus = focus_events.read().any(|e| !e.focused);
    if !in_room {
        ui.menu = false;
        ui.need_click = false;
        capture(&mut cursor, false);
        *seen = (session.entries, None);
        return;
    }
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
    let typing = focus.get().is_some_and(|e| fields.contains(e));
    let esc = keys.just_pressed(KeyCode::Escape) && !ui.chat && ui.rebinding.is_none() && !(typing && !ui.menu);
    // (A pad keeps reporting while another window has the focus.)
    let start = window_focused(&windows) && pads.iter().any(|p| p.just_pressed(GamepadButton::Start));
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
}

/// A player without a name of their own keeps the one the room gave them.
fn adopt_name(session: Res<Session>, mut player: ResMut<Player>, opts: Res<crate::opts::Opts>, mut commands: Commands) {
    if !player.name.is_empty() || !opts.name.is_empty() || !session.is_changed() {
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
        if matches!(f, Field::MenuName | Field::Name) && Some(e) != focused && t.value().to_string() != name {
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

fn head(
    mut q: Query<(Entity, &mut Section), With<MenuHead>>,
    ui: Res<Ui>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    if !sec.stale(key_of(&(ui.menu_tab, session.dev))) {
        return;
    }
    let f = &*f;
    let tab = ui.menu_tab;
    let dev = session.dev;
    rebuild(&mut commands, e, |p| {
        p.spawn(Node {
            width: percent(100),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            flex_wrap: FlexWrap::Wrap,
            row_gap: rem(0.5),
            ..default()
        })
        .with_children(|r| {
            logo(r, f, 26.0);
            let mut items = vec![
                (text::TAB_GAME, Action::MenuTab(MenuTab::Game), tab == MenuTab::Game),
                (
                    text::TAB_SETTINGS,
                    Action::MenuTab(MenuTab::Settings),
                    tab == MenuTab::Settings,
                ),
            ];
            if dev {
                items.push((text::TAB_DEV, Action::MenuTab(MenuTab::Dev), tab == MenuTab::Dev));
            }
            tabs(r, f, &items);
        });
    });
}

/// The room's name and the way out; for the host, private or public and the PIN. In practice: the way back.
fn top(
    mut q: Query<(Entity, &mut Section, &mut Node), With<MenuTop>>,
    mut name_row: Query<&mut Node, (With<MenuNameRow>, Without<MenuTop>)>,
    ui: Res<Ui>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec, mut node)) = q.single_mut() else {
        return;
    };
    let on = ui.menu_tab == MenuTab::Game;
    show(&mut node, on);
    for mut n in &mut name_row {
        show(&mut n, on && !session.practice);
    }
    let room = session.lobby.as_ref().map(|l| (&l.room, l.host, &l.pin));
    let key = key_of(&(
        room,
        session.me,
        session.practice,
        &session.back_to,
        &session.arena.as_ref().map(|a| &a.game),
    ));
    if !sec.stale(key) {
        return;
    }
    let f = &*f;
    let host = session.host();
    let lobby = session.lobby.clone();
    rebuild(&mut commands, e, |p| {
        if session.practice {
            let game = session.arena.as_ref().map(|a| a.game.clone()).unwrap_or_default();
            let title = fb_maps::by_id(&game).map_or(game.clone(), |d| d.meta().title.to_string());
            label(p, f, &text::practice_now(&title));
            let back = if session.back_to.is_some() {
                text::BACK_TO_ROOM
            } else {
                text::PRACTICE_BACK
            };
            button(p, f, back, Look::Plain, Action::EndPractice);
            return;
        }
        let Some(l) = lobby else { return };
        group(p, |g| {
            row(g, false, |r| {
                let lock = if l.room.private { "🔒 " } else { "" };
                r.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
                })
                .with_children(|t| {
                    heading(t, f, &format!("{lock}{}", l.room.title));
                });
                button(r, f, text::LEAVE_ROOM, Look::TinyDanger, Action::LeaveRoom);
            });
            if host {
                let s = match &l.pin {
                    Some(pin) => text::pin_line(pin),
                    None => format!("{} {}", text::PRIVATE_ROOM, text::PIN_FOR_ENTRY),
                };
                button(
                    g,
                    f,
                    &s,
                    Look::Check(l.room.private),
                    Action::Send(ClientMsg::Access {
                        private: !l.room.private,
                    }),
                );
            } else if l.room.private {
                muted(g, f, text::PRIVATE_NOTE);
            }
        });
    });
}

#[derive(Debug, PartialEq)]
struct BodyKey<'a> {
    lobby: Option<&'a Lobby>,
    me: Option<Pid>,
    open: &'a std::collections::BTreeSet<&'static str>,
    outfit: fb_proto::Outfit,
    arena: Option<(u32, ArenaKind, u32, u32)>,
    practice: bool,
}

/// The look, who is here, the game setup (host) and practice.
fn body(
    mut q: Query<(Entity, &mut Section, &mut Node), With<MenuBody>>,
    ui: Res<Ui>,
    session: Res<Session>,
    player: Res<Player>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec, mut node)) = q.single_mut() else {
        return;
    };
    show(&mut node, ui.menu_tab == MenuTab::Game && !session.practice);
    let key = BodyKey {
        lobby: session.lobby.as_ref(),
        me: session.me,
        open: &ui.open,
        outfit: player.outfit(),
        arena: session.arena.as_ref().map(|a| (a.id, a.kind, a.index, a.total)),
        practice: session.practice,
    };
    if !sec.stale(key_of(&key)) {
        return;
    }
    let f = &*f;
    let Some(l) = session.lobby.clone() else {
        rebuild(&mut commands, e, |_| {});
        return;
    };
    let me = session.me;
    let host = session.host();
    let outfit = player.outfit();
    let arena = session.arena.clone();
    rebuild(&mut commands, e, |p| {
        outfit_picker(p, f, &ui, &l, me, &outfit);
        if l.phase != Phase::Lobby {
            let line = match &arena {
                Some(a) if a.kind == ArenaKind::Round => {
                    let title = fb_maps::by_id(&a.game).map_or("", |d| d.meta().title);
                    text::round_now(a.index, a.total, title)
                }
                _ if l.phase == Phase::Podium => text::PODIUM_NOW.into(),
                _ => text::RESULTS_NOW.into(),
            };
            label(p, f, &line);
            if host {
                button(p, f, text::ABORT, Look::Danger, Action::Send(ClientMsg::Abort));
            }
            return;
        }
        heading(p, f, &text::players_of(l.players.len(), l.max));
        stack(p, |list| {
            for pl in &l.players {
                player_row(list, f, &l, pl, me, host);
            }
        });
        if host {
            host_setup(p, f, &l);
        } else {
            muted(p, f, text::HOST_STARTS);
        }
        practice_list(p, f, &ui);
    });
}

fn player_row(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    l: &Lobby,
    pl: &fb_proto::LobbyPlayer,
    me: Option<Pid>,
    host: bool,
) {
    p.spawn((
        Node {
            column_gap: rem(0.5),
            align_items: AlignItems::Center,
            padding: UiRect::axes(rem(0.5), rem(0.3)),
            border_radius: BorderRadius::all(rem(0.625)),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, if pl.connected { 0.45 } else { 0.25 })),
    ))
    .with_children(|r| {
        dot(r, suit(pl.color), 0.8);
        r.spawn(Node {
            flex_grow: 1.0,
            ..default()
        })
        .with_children(|n| {
            let you = if Some(pl.id) == me { text::YOU } else { "" };
            label(n, f, &format!("{}{you}", pl.name));
        });
        if Some(pl.id) == l.host {
            label(r, f, "⭐");
        }
        if pl.crowns > 0 {
            label(r, f, &format!("👑{}", pl.crowns));
        }
        if pl.bot {
            // (With "fill with bots" on, a bot taken out would be replaced at once.)
            if host && !l.fill {
                button(r, f, "×", Look::Tiny, Action::Send(ClientMsg::RemoveBot(pl.id)));
            } else {
                muted(r, f, text::BOT);
            }
        } else {
            if host && Some(pl.id) != me && pl.connected {
                button(r, f, text::GIVE_HOST, Look::Tiny, Action::Send(ClientMsg::Host(pl.id)));
            }
            let ping = if pl.connected {
                text::ping(pl.ping)
            } else {
                text::NO_LINK.into()
            };
            muted(r, f, &ping);
        }
    });
}

/// The host's part of the lobby: which maps, how many rounds, bots, and the start button.
fn host_setup(p: &mut ChildSpawnerCommands, f: &Fonts, l: &Lobby) {
    let pl = &l.playlist;
    let ready = l.players.iter().filter(|p| p.connected || p.bot).count() as u32;
    let enough = ready >= l.min;
    let with = |patch: &dyn Fn(&mut Playlist)| {
        let mut next = pl.clone();
        patch(&mut next);
        Action::Send(ClientMsg::Playlist(next))
    };
    group(p, |g| {
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
                    let at = pl.games.iter().position(|g| g == m.id);
                    let s = match at {
                        Some(i) => format!("{}. {}", i + 1, m.title),
                        None => m.title.to_string(),
                    };
                    let id = m.id.to_string();
                    let act = with(&|p| {
                        if let Some(i) = p.games.iter().position(|g| *g == id) {
                            p.games.remove(i);
                        } else if p.games.len() < 12 {
                            p.games.push(id.clone());
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
        row(g, false, |r| {
            let room = !l.fill && (l.players.len() as u32) < l.max;
            button_if(r, f, text::ADD_BOT, Look::Plain, Action::Send(ClientMsg::AddBot), room);
            r.spawn(Node {
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_width: px(0),
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|g| {
                let s = if enough {
                    text::START_GAME.to_string()
                } else {
                    text::need_players(l.min)
                };
                button_if(g, f, &s, Look::Go, Action::Send(ClientMsg::Start), enough);
            });
        });
    });
}

/// The player's look: suit colour (lobby only, one per bean), hat, glasses and colours of the hat, belly and shoes.
fn outfit_picker(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    ui: &Ui,
    l: &Lobby,
    me: Option<Pid>,
    outfit: &fb_proto::Outfit,
) {
    let mine = l.players.iter().find(|p| Some(p.id) == me);
    if l.phase == Phase::Lobby {
        row(p, true, |r| {
            for (i, _) in COLORS.iter().enumerate() {
                let i = i as u8;
                let taken = l.players.iter().any(|p| p.color == i && Some(p.id) != me);
                let on = mine.is_some_and(|m| m.color == i);
                swatch(r, suit(i), on, Action::Color(i), !taken, 1.75);
            }
        });
    }
    fold(p, f, ui, "outfit", text::OUTFIT, |c| {
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
        muted(c, f, title);
        row(c, true, |r| {
            // "As designed": the part's own colour.
            swatch(r, Color::srgb(0.85, 0.86, 0.89), now.is_none(), act(None), true, 1.375);
            for t in TINT_LIST {
                swatch(r, hex(t.hex()), now == Some(t), act(Some(t)), true, 1.375);
            }
        });
    });
}

fn settings(
    mut q: Query<(Entity, &mut Section, &mut Node), With<MenuSettings>>,
    ui: Res<Ui>,
    options: Options,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec, mut node)) = q.single_mut() else {
        return;
    };
    show(&mut node, ui.menu_tab == MenuTab::Settings);
    if !sec.stale(options.key(&ui)) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| settings_tab(p, f, &options, &ui));
}

const RATES: [f64; 7] = [0.0, 0.1, 0.25, 0.5, 1.0, 2.0, 4.0];

/// Dev tools (a server started with `--dev`): time, quick games, teleports, bots, forced hits.
fn dev(
    mut q: Query<(Entity, &mut Section, &mut Node), With<MenuDev>>,
    ui: Res<Ui>,
    session: Res<Session>,
    map: Option<Res<crate::game::Map>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec, mut node)) = q.single_mut() else {
        return;
    };
    show(&mut node, ui.menu_tab == MenuTab::Dev && session.dev);
    let checkpoints = map.as_ref().map_or(0, |m| m.spec.checkpoints.len());
    let finish = map.as_ref().is_some_and(|m| m.spec.finish.is_some());
    if !sec.stale(key_of(&(session.dev, checkpoints, finish, &ui.open))) {
        return;
    }
    let f = &*f;
    let d = |cmd: DevCmd| Action::Send(ClientMsg::Dev { q: None, cmd });
    rebuild(&mut commands, e, |p| {
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
            button(r, f, "Разморозить", Look::Chip(false), d(DevCmd::Bots { on: true }));
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
        fold(p, f, &ui, "dev-maps", "Играть карту (3 бота)", |c| {
            row(c, true, |r| {
                for def in fb_maps::GAMES {
                    let m = def.meta();
                    let cmd = DevCmd::Start {
                        games: vec![m.id.to_string()],
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
    mut player: ResMut<Player>,
    mut opts: ResMut<crate::opts::Opts>,
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
                player.color = i32::from(*c);
                opts.color = None;
                crate::settings::save_soon(&mut commands);
                crate::session::send(&mut senders, ClientMsg::Color(*c));
            }
            Action::Wear(o) => wear(&mut player, &mut senders, &mut commands, o),
            Action::RandomOutfit => {
                // Any pick will do: the clock's nanoseconds are random enough for a hat.
                let mut x = time.elapsed().as_nanos() as u64 | 1;
                let mut pick = |n: usize| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    (x % n as u64) as usize
                };
                let tint = |i: usize| (i > 0).then(|| TINT_LIST[i - 1]);
                let o = fb_proto::Outfit {
                    hat: HATS[pick(HATS.len())],
                    hat_color: tint(pick(TINT_LIST.len() + 1)),
                    glasses: GLASSES[pick(GLASSES.len())],
                    belly: tint(pick(TINT_LIST.len() + 1)),
                    shoes: tint(pick(TINT_LIST.len() + 1)),
                };
                wear(&mut player, &mut senders, &mut commands, &o);
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
