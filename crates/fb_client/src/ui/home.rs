//! The first screen: the player's servers, then on one of them (port of `home/Home.tsx`) the player's name,
//! the rooms to enter, a room of one's own, practice; the options; and the screens of the connection
//! (connecting, updating, refused).
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use fb_proto::{ClientMsg, DenyReason, Phase, RoomInfo};
use fb_shared::{NAME_MAX, ROOM_PIN_DIGITS, ROOM_TITLE_MAX};

use super::*;
use crate::net::Conn;
use crate::opts::Opts;
use crate::servers::{Servers, States, Status, Target};
use crate::session::{Denied, Session};
use crate::settings::{Me, Player};
use crate::update::{State as UpdateState, Update};

pub struct HomePlugin;

impl Plugin for HomePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build_home.after(super::setup));
        app.add_systems(
            Update,
            (
                layers,
                (
                    head, update_box, servers, server_add, rooms, pin, create, practice, settings, banner,
                ),
                enter_submits,
                (home_actions, server_actions),
            )
                .chain(),
        );
    }
}

#[derive(Component)]
struct HomeHead;
#[derive(Component)]
struct UpdateBox;
#[derive(Component)]
struct ServersBox;
#[derive(Component)]
struct ServerAddBox;
#[derive(Component)]
struct RoomsTabBox;
#[derive(Component)]
struct SettingsBox;
#[derive(Component)]
struct RoomsList;
#[derive(Component)]
struct ServersList;
#[derive(Component)]
struct PinBox;
#[derive(Component)]
struct CreateBox;
#[derive(Component)]
struct PracticeBox;
#[derive(Component)]
struct BannerBox;

fn build_home(mut commands: Commands, layers: Query<(Entity, &Layer)>, f: Res<Fonts>, me: Me) {
    let f = &*f;
    for (e, layer) in &layers {
        match layer {
            Layer::Home => {
                commands.entity(e).with_children(|l| {
                    l.spawn(Node {
                        width: percent(100),
                        height: percent(100),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        padding: UiRect::all(rem(1.0)),
                        ..default()
                    })
                    .with_children(|c| {
                        c.spawn((
                            Node {
                                width: rem(36.0),
                                max_width: percent(100),
                                max_height: percent(100),
                                flex_direction: FlexDirection::Column,
                                row_gap: rem(0.75),
                                padding: UiRect::all(rem(1.25)),
                                border: UiRect::all(px(1)),
                                border_radius: BorderRadius::all(rem(1.125)),
                                ..default()
                            },
                            glass(),
                        ))
                        .with_children(|panel| {
                            panel.spawn((HomeHead, Section::default(), Node::default()));
                            panel.spawn((
                                UpdateBox,
                                Section::default(),
                                Node {
                                    flex_direction: FlexDirection::Column,
                                    row_gap: rem(0.5),
                                    ..default()
                                },
                            ));
                            panel
                                .spawn((
                                    Node {
                                        flex_direction: FlexDirection::Column,
                                        overflow: Overflow::scroll_y(),
                                        flex_shrink: 1.0,
                                        ..default()
                                    },
                                    bevy::ui_widgets::ScrollArea,
                                ))
                                .with_children(|body| {
                                    body.spawn((
                                        ServersBox,
                                        Node {
                                            flex_direction: FlexDirection::Column,
                                            row_gap: rem(0.75),
                                            ..default()
                                        },
                                    ))
                                    .with_children(|t| {
                                        t.spawn((
                                            ServersList,
                                            Section::default(),
                                            Node {
                                                flex_direction: FlexDirection::Column,
                                                row_gap: rem(0.625),
                                                ..default()
                                            },
                                        ));
                                        t.spawn((
                                            ServerAddBox,
                                            Section::default(),
                                            Node {
                                                flex_direction: FlexDirection::Column,
                                                row_gap: rem(0.5),
                                                ..default()
                                            },
                                        ));
                                    });
                                    body.spawn((
                                        RoomsTabBox,
                                        Node {
                                            flex_direction: FlexDirection::Column,
                                            row_gap: rem(0.75),
                                            ..default()
                                        },
                                    ))
                                    .with_children(|t| {
                                        name_row(t, f, Field::Name, &me.name());
                                        for marker in 0..4 {
                                            let mut s = t.spawn((
                                                Section::default(),
                                                Node {
                                                    flex_direction: FlexDirection::Column,
                                                    row_gap: rem(0.625),
                                                    ..default()
                                                },
                                            ));
                                            match marker {
                                                0 => s.insert(RoomsList),
                                                1 => s.insert(PinBox),
                                                2 => s.insert(CreateBox),
                                                _ => s.insert(PracticeBox),
                                            };
                                        }
                                    });
                                    body.spawn((
                                        SettingsBox,
                                        Section::default(),
                                        Node {
                                            flex_direction: FlexDirection::Column,
                                            ..default()
                                        },
                                    ));
                                });
                        });
                    });
                });
            }
            Layer::Banner => {
                commands.entity(e).insert((BannerBox, Section::default()));
            }
            _ => {}
        }
    }
}

/// The player's name: kept in the settings and told to the server (at the room list or in a room).
pub fn name_row(p: &mut ChildSpawnerCommands, f: &Fonts, which: Field, name: &str) {
    row(p, false, |r| {
        field_with_hint(r, f, which, name, text::NAME_PLACEHOLDER, NAME_MAX, None);
        button(r, f, text::SAVE, Look::Plain, Action::SaveName(which));
    });
}

/// Where the player is decides what is on screen: the room list, a room (HUD, chat, menu), or a banner.
fn layers(
    mut q: Query<(&Layer, &mut Node)>,
    session: Res<Session>,
    conn: Option<Res<Conn>>,
    target: Res<Target>,
    ui: Res<Ui>,
    mut rooms_box: Query<&mut Node, (With<RoomsTabBox>, Without<Layer>)>,
    mut settings_box: Query<&mut Node, (With<SettingsBox>, Without<Layer>, Without<RoomsTabBox>)>,
    mut servers_box: Query<
        &mut Node,
        (
            With<ServersBox>,
            Without<Layer>,
            Without<RoomsTabBox>,
            Without<SettingsBox>,
        ),
    >,
) {
    let online = conn.as_ref().is_some_and(|c| c.connected);
    let in_room = session.room.is_some() && session.arena.is_some();
    let picking = target.0.is_none();
    let home = picking || (online && session.room.is_none() && session.rooms.is_some() && !session.refused);
    for (layer, mut node) in &mut q {
        let on = match layer {
            Layer::Home => home,
            Layer::Banner => true,
            Layer::Hud | Layer::Tags => in_room,
            Layer::Chat => in_room && !session.practice,
            Layer::Menu => in_room && ui.menu,
        };
        show(&mut node, on);
    }
    for mut n in &mut rooms_box {
        show(&mut n, ui.home_tab == HomeTab::Rooms && !picking);
    }
    for mut n in &mut servers_box {
        show(&mut n, ui.home_tab == HomeTab::Rooms && picking);
    }
    for mut n in &mut settings_box {
        show(&mut n, ui.home_tab == HomeTab::Settings);
    }
}

pub fn tabs(p: &mut ChildSpawnerCommands, f: &Fonts, items: &[(&str, Action, bool)]) {
    p.spawn((
        Node {
            column_gap: rem(0.25),
            padding: UiRect::all(rem(0.2)),
            border_radius: BorderRadius::all(rem(0.875)),
            ..default()
        },
        BackgroundColor(GROUP),
    ))
    .with_children(|t| {
        for (s, a, on) in items {
            button(t, f, s, Look::Tab(*on), a.clone());
        }
    });
}

fn head(
    mut q: Query<(Entity, &mut Section), With<HomeHead>>,
    ui: Res<Ui>,
    target: Res<Target>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let picking = target.0.is_none();
    if !sec.stale(key_of(&(ui.home_tab, picking))) {
        return;
    }
    let f = &*f;
    let tab = ui.home_tab;
    let first = if picking { text::TAB_SERVERS } else { text::TAB_ROOMS };
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
            tabs(
                r,
                f,
                &[
                    (first, Action::HomeTab(HomeTab::Rooms), tab == HomeTab::Rooms),
                    (
                        text::TAB_SETTINGS,
                        Action::HomeTab(HomeTab::Settings),
                        tab == HomeTab::Settings,
                    ),
                ],
            );
        });
    });
}

/// A newer release, the download, or this build's version.
fn update_box(
    mut q: Query<(Entity, &mut Section), With<UpdateBox>>,
    update: Option<Res<Update>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let state = update.as_ref().map_or(UpdateState::Idle, |u| u.state.clone());
    // (A download is redrawn when its percentage changes, not with every chunk that comes in.)
    let key = match &state {
        UpdateState::Downloading(got, total) => key_of(&("downloading", (got * 100).checked_div(*total))),
        s => key_of(s),
    };
    if !sec.stale(key) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| match &state {
        UpdateState::Idle => {
            muted(p, f, &text::version(&fb_net::build()));
        }
        UpdateState::Available(r) => {
            group(p, |g| {
                row(g, false, |row_| {
                    heading(row_, f, &text::update_out(&r.version));
                    if r.installable() {
                        button(row_, f, text::UPDATE, Look::Go, Action::Update);
                    } else {
                        button(row_, f, text::DOWNLOAD, Look::Go, Action::ReleasePage);
                    }
                });
            });
        }
        UpdateState::Downloading(got, total) => {
            label(p, f, &text::downloading(*got, *total));
        }
        UpdateState::Failed(err) => {
            group(p, |g| {
                rich(g, f, &text::update_failed(err), 13.0, RED_INK);
                button(g, f, text::RELEASE_PAGE, Look::Plain, Action::ReleasePage);
            });
        }
        UpdateState::Restarting => {
            label(p, f, text::RESTARTING);
        }
    });
}

/// The player's servers, each with what it said last.
fn servers(
    mut q: Query<(Entity, &mut Section), With<ServersList>>,
    list: Res<Servers>,
    states: Res<States>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let rows: Vec<(String, Status)> = list
        .ordered()
        .into_iter()
        .map(|a| (a.clone(), states.get(&a)))
        .collect();
    if !sec.stale(key_of(&rows)) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| {
        heading(p, f, text::SERVERS);
        if rows.is_empty() {
            muted(p, f, text::NO_SERVERS);
        }
        for (addr, status) in &rows {
            server_row(p, f, addr, status);
        }
    });
}

fn server_row(p: &mut ChildSpawnerCommands, f: &Fonts, addr: &str, status: &Status) {
    let (title, line, can) = match status {
        Status::Checking => (addr.to_string(), text::CHECKING.to_string(), true),
        Status::Down => (addr.to_string(), text::SERVER_DOWN.to_string(), true),
        Status::Up(i) => {
            let title = i.name.clone().unwrap_or_else(|| addr.to_string());
            let same = i.protocol == fb_shared::PROTOCOL_VERSION;
            let line = if !same {
                format!("{} ({})", text::SERVER_OTHER_VERSION, i.build)
            } else if i.updating {
                text::SERVER_UPDATING.to_string()
            } else {
                text::server_line(i.players, i.rooms)
            };
            (title, line, same)
        }
    };
    let up = matches!(status, Status::Up(_));
    p.spawn((
        Node {
            column_gap: rem(0.5),
            align_items: AlignItems::Center,
            padding: UiRect::axes(rem(0.75), rem(0.5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(0.875)),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
        BorderColor::all(RIM),
    ))
    .with_children(|row_| {
        dot(row_, if up { GREEN } else { MUTED.with_alpha(0.5) }, 10.0);
        row_.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            ..default()
        })
        .with_children(|c| {
            heading(c, f, &title);
            let addr_line = if title == addr {
                line
            } else {
                format!("{addr} · {line}")
            };
            muted(c, f, &addr_line);
        });
        button_if(
            row_,
            f,
            text::ENTER,
            Look::Primary,
            Action::Connect(addr.to_string()),
            can,
        );
        // ("×": "✕" is in neither of the game's fonts.)
        button(row_, f, "×", Look::TinyDanger, Action::RemoveServer(addr.to_string()));
    });
}

/// The field to add a server (rebuilt, so emptied, when the list changes).
fn server_add(
    mut q: Query<(Entity, &mut Section), With<ServerAddBox>>,
    list: Res<Servers>,
    ui: Res<Ui>,
    fields: Query<(&Field, &EditableText)>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    if !sec.stale(key_of(&(&list.list, ui.server_bad))) {
        return;
    }
    let f = &*f;
    let typed = if ui.server_bad {
        field_text(&fields, Field::Server)
    } else {
        String::new()
    };
    rebuild(&mut commands, e, |p| {
        row(p, false, |r| {
            field_with_hint(r, f, Field::Server, &typed, text::SERVER_PLACEHOLDER, 120, None);
            button(r, f, text::ADD, Look::Go, Action::AddServer);
        });
        if ui.server_bad {
            rich(p, f, text::SERVER_BAD, 13.0, RED_INK);
        }
        muted(p, f, text::SERVER_HINT);
    });
}

fn rooms(
    mut q: Query<(Entity, &mut Section), With<RoomsList>>,
    session: Res<Session>,
    conn: Option<Res<Conn>>,
    servers: Res<Servers>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let gone = session.denied.as_ref().filter(|d| d.reason != DenyReason::Pin);
    let server = conn.map(|c| {
        if servers.last.is_empty() {
            c.http.clone()
        } else {
            servers.last.clone()
        }
    });
    if !sec.stale(key_of(&(&session.rooms, &session.mine, gone, &server))) {
        return;
    }
    let f = &*f;
    let list = session.rooms.clone();
    let mine = session.mine.clone();
    let alert = gone.map(|d| d.msg.clone()).filter(|m| !m.is_empty());
    rebuild(&mut commands, e, |p| {
        row(p, false, |r| {
            button(r, f, text::TO_SERVERS, Look::Tiny, Action::LeaveServer);
            if let Some(s) = &server {
                muted(r, f, s);
            }
        });
        if let Some(msg) = alert {
            rich(p, f, &msg, 14.0, RED_INK);
        }
        heading(p, f, &text::rooms_count(list.as_ref().map_or(0, Vec::len)));
        match &list {
            None => {
                muted(p, f, text::LOADING_ROOMS);
            }
            Some(l) if l.is_empty() => {
                muted(p, f, text::NO_ROOMS);
            }
            Some(l) => {
                for r in l {
                    room_row(p, f, r, mine.as_deref() == Some(r.id.as_str()));
                }
            }
        }
    });
}

fn room_row(p: &mut ChildSpawnerCommands, f: &Fonts, r: &RoomInfo, mine: bool) {
    let full = r.players >= r.max;
    p.spawn((
        Node {
            column_gap: rem(0.5),
            align_items: AlignItems::Center,
            padding: UiRect::axes(rem(0.75), rem(0.5)),
            border: UiRect::all(px(if mine { 2 } else { 1 })),
            border_radius: BorderRadius::all(rem(0.875)),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
        BorderColor::all(if mine { PINK } else { RIM }),
    ))
    .with_children(|row_| {
        row_.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            ..default()
        })
        .with_children(|c| {
            let lock = if r.private { "🔒 " } else { "" };
            heading(c, f, &format!("{lock}{}", r.title));
            muted(c, f, &text::room_line(&r.host, r.phase != Phase::Lobby, mine));
        });
        let bots = if r.bots > 0 {
            format!(" +{} 🤖", r.bots)
        } else {
            String::new()
        };
        label(row_, f, &format!("{}/{}{bots}", r.players, r.max));
        let s = if full { text::NO_PLACES } else { text::ENTER };
        button_if(row_, f, s, Look::Primary, Action::Join(r.id.clone()), !full);
    });
}

/// A private room asked for its PIN: only its host knows it.
fn pin(
    mut q: Query<(Entity, &mut Section), With<PinBox>>,
    session: Res<Session>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let asks = session.denied.as_ref().filter(|d| d.reason == DenyReason::Pin);
    if !sec.stale(key_of(&asks)) {
        return;
    }
    let f = &*f;
    let asks: Option<Denied> = asks.cloned();
    rebuild(&mut commands, e, |p| {
        let Some(d) = asks else { return };
        let Some(room) = d.room.clone() else { return };
        let title = session
            .rooms
            .as_ref()
            .and_then(|l| l.iter().find(|r| r.id == room))
            .map_or_else(|| room.to_uppercase(), |r| r.title.clone());
        group(p, |g| {
            heading(g, f, &format!("{} «{title}»", text::PIN_LOCKED));
            row(g, false, |r| {
                field_with_hint(
                    r,
                    f,
                    Field::Pin,
                    "",
                    text::PIN_PLACEHOLDER,
                    ROOM_PIN_DIGITS,
                    Some(|c: char| c.is_ascii_digit()),
                );
                button(r, f, text::ENTER, Look::Primary, Action::SubmitPin(room.clone()));
                button(r, f, text::CANCEL, Look::Plain, Action::CancelPin);
            });
            if d.msg.is_empty() {
                muted(g, f, text::PIN_ASK);
            } else {
                rich(g, f, &d.msg, 13.0, RED_INK);
            }
        });
    });
}

/// Everyone may keep one room of their own: they are its host whenever they are in it.
fn create(
    mut q: Query<(Entity, &mut Section), With<CreateBox>>,
    session: Res<Session>,
    ui: Res<Ui>,
    me: Me,
    fields: Query<(&Field, &EditableText)>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let name = me.name();
    if !sec.stale(key_of(&(&session.mine, ui.create_private, &name))) {
        return;
    }
    let f = &*f;
    let mine = session.mine.clone();
    let title = field_text(&fields, Field::RoomTitle);
    let private = ui.create_private;
    rebuild(&mut commands, e, |p| {
        group(p, |g| {
            if let Some(id) = mine {
                button(g, f, text::BACK_TO_OWN, Look::Go, Action::Join(id));
                muted(g, f, text::OWN_ROOM_NOTE);
                return;
            }
            heading(g, f, text::OWN_ROOM);
            field_with_hint(
                g,
                f,
                Field::RoomTitle,
                &title,
                &text::room_title_placeholder(&name),
                ROOM_TITLE_MAX,
                None,
            );
            button(g, f, text::PRIVATE_CREATE, Look::Check(private), Action::CreatePrivate);
            button(g, f, text::CREATE_ROOM, Look::Go, Action::CreateRoom);
        });
    });
}

/// One map against bots, alone.
pub fn practice_list(p: &mut ChildSpawnerCommands, f: &Fonts, ui: &Ui) {
    fold(p, f, ui, "practice", text::PRACTICE, |c| {
        row(c, true, |r| {
            for g in fb_maps::GAMES {
                let m = g.meta();
                let s = format!("{} · {}", m.title, text::genre(m.genre));
                button(r, f, &s, Look::Chip(false), Action::Practice(m.id));
            }
        });
    });
}

fn practice(
    mut q: Query<(Entity, &mut Section), With<PracticeBox>>,
    ui: Res<Ui>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    if !sec.stale(key_of(&ui.open.contains("practice"))) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| practice_list(p, f, &ui));
}

fn settings(
    mut q: Query<(Entity, &mut Section), With<SettingsBox>>,
    options: Options,
    ui: Res<Ui>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    if !sec.stale(options.key(&ui)) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| settings_tab(p, f, &options, &ui));
}

/// Connecting, reconnecting, the game being updated, or refused: a card in the middle or a banner on top.
fn banner(
    mut q: Query<(Entity, &mut Section), With<BannerBox>>,
    session: Res<Session>,
    conn: Option<Res<Conn>>,
    target: Res<Target>,
    update: Option<Res<Update>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let Ok((e, mut sec)) = q.single_mut() else { return };
    let online = conn.as_ref().is_some_and(|c| c.connected);
    let newer = update.as_ref().and_then(|u| match &u.state {
        UpdateState::Available(r) => Some(r.installable()),
        _ => None,
    });
    let ever = conn.as_ref().is_some_and(|c| c.ever);
    #[derive(Debug, PartialEq)]
    enum Show {
        None,
        Card(&'static str, Option<String>, bool),
        Outdated(Option<bool>),
        Line(&'static str),
    }
    let show = if target.0.is_none() {
        Show::None
    } else if let Some(msg) = &session.reject {
        Show::Card(text::LOGO, Some(msg.clone()), true)
    } else if session.refused {
        Show::Outdated(newer)
    } else if session.updating {
        Show::Card(text::UPDATING, Some(text::UPDATING_MORE.into()), false)
    } else if !online && ever {
        Show::Line(text::RECONNECTING)
    } else if !online || (session.room.is_none() && session.rooms.is_none()) {
        Show::Line(text::CONNECTING)
    } else {
        Show::None
    };
    if !sec.stale(key_of(&show)) {
        return;
    }
    let f = &*f;
    rebuild(&mut commands, e, |p| match show {
        Show::None => {}
        Show::Outdated(newer) => card(p, |card| {
            logo(card, f, 30.0);
            label(card, f, text::OUTDATED);
            row(card, false, |r| {
                match newer {
                    Some(true) => button(r, f, text::UPDATE, Look::Go, Action::Update),
                    _ => button(r, f, text::DOWNLOAD, Look::Go, Action::ReleasePage),
                };
                button(r, f, text::TO_SERVERS, Look::Plain, Action::LeaveServer);
            });
        }),
        Show::Line(s) => {
            p.spawn(Node {
                width: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexStart,
                padding: UiRect::top(rem(1.0)),
                ..default()
            })
            .with_children(|c| {
                c.spawn((
                    Node {
                        padding: UiRect::axes(rem(1.25), rem(0.6)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(rem(1.0)),
                        ..default()
                    },
                    glass(),
                ))
                .with_children(|b| {
                    row(b, false, |r| {
                        heading(r, f, s);
                        button(r, f, text::CANCEL, Look::Tiny, Action::LeaveServer);
                    });
                });
            });
        }
        Show::Card(title, more, quit) => card(p, |card| {
            logo(card, f, 30.0);
            if title != text::LOGO {
                heading(card, f, title);
            }
            if let Some(m) = more {
                label(card, f, &m);
            }
            // (Always a way back: a server being updated may take long or not come back.)
            row(card, false, |r| {
                button(r, f, text::TO_SERVERS, Look::Plain, Action::LeaveServer);
                if quit {
                    button(r, f, text::QUIT, Look::Primary, Action::Quit);
                }
            });
        }),
    });
}

/// A card in the middle of a dimmed screen.
fn card(p: &mut ChildSpawnerCommands, inner: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn((
        Node {
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.169, 0.102, 0.361, 0.25)),
    ))
    .with_children(|c| {
        c.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: rem(0.75),
                padding: UiRect::axes(rem(1.6), rem(1.4)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(rem(1.125)),
                max_width: rem(30.0),
                ..default()
            },
            glass(),
        ))
        .with_children(inner);
    });
}

/// Enter in a field does what its button does.
fn enter_submits(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    fields: Query<&Field>,
    session: Res<Session>,
    mut out: MessageWriter<UiAction>,
) {
    if !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        return;
    }
    let Some(field) = focus.get().and_then(|e| fields.get(e).ok()) else {
        return;
    };
    let a = match field {
        Field::Server => Action::AddServer,
        Field::Name | Field::MenuName => Action::SaveName(*field),
        Field::RoomTitle => Action::CreateRoom,
        Field::Pin => match session.denied.as_ref().and_then(|d| d.room.clone()) {
            Some(room) => Action::SubmitPin(room),
            None => return,
        },
        Field::Chat => return,
    };
    out.write(UiAction(a));
}

fn home_actions(
    mut actions: MessageReader<UiAction>,
    mut session: ResMut<Session>,
    mut player: ResMut<Player>,
    mut opts: ResMut<Opts>,
    mut ui: ResMut<Ui>,
    conn: Option<ResMut<Conn>>,
    fields: Query<(&Field, &EditableText)>,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
    mut commands: Commands,
) {
    let mut conn = conn;
    for UiAction(a) in actions.read() {
        let send = |senders: &mut Query<&mut MessageSender<ClientMsg>, With<Client>>, m: ClientMsg| {
            crate::session::send(senders, m);
        };
        match a {
            Action::Send(m) => send(&mut senders, m.clone()),
            Action::SaveName(which) => {
                let n = fb_shared::text::sanitize_name(&field_text(&fields, *which));
                if n.is_empty() {
                    continue;
                }
                player.name = n.clone();
                opts.name.clear();
                crate::settings::save_soon(&mut commands);
                send(&mut senders, ClientMsg::Name(n));
            }
            Action::Join(room) => {
                session.denied = None;
                send(
                    &mut senders,
                    ClientMsg::Join {
                        room: room.clone(),
                        pin: None,
                    },
                );
            }
            Action::SubmitPin(room) => {
                let pin = field_text(&fields, Field::Pin);
                if fb_proto::valid_pin(&pin) {
                    send(
                        &mut senders,
                        ClientMsg::Join {
                            room: room.clone(),
                            pin: Some(pin),
                        },
                    );
                }
            }
            Action::CancelPin => session.denied = None,
            Action::CreatePrivate => ui.create_private ^= true,
            Action::CreateRoom => {
                let mut title = fb_shared::text::sanitize_title(&field_text(&fields, Field::RoomTitle));
                let name = if opts.name.is_empty() {
                    player.name.clone()
                } else {
                    opts.name.clone()
                };
                if title.is_empty() && !name.is_empty() {
                    title = text::room_title_placeholder(&name);
                }
                send(
                    &mut senders,
                    ClientMsg::Create {
                        title,
                        private: ui.create_private,
                    },
                );
            }
            Action::Practice(id) => {
                let Some(c) = conn.as_deref_mut() else { continue };
                session.back_to = session.room.clone().filter(|r| !r.is_empty());
                opts.practice = Some((*id).to_string());
                opts.room = None;
                session.room = None;
                ui.menu = false;
                crate::net::restart(&mut commands, c);
            }
            Action::EndPractice => {
                let Some(c) = conn.as_deref_mut() else { continue };
                opts.practice = None;
                opts.room = session.back_to.take();
                session.room = None;
                crate::net::restart(&mut commands, c);
            }
            Action::LeaveRoom => {
                ui.menu = false;
                opts.room = None;
                send(&mut senders, ClientMsg::Leave);
            }
            _ => {}
        }
    }
}

/// Adding, removing, entering and leaving servers.
fn server_actions(
    mut actions: MessageReader<UiAction>,
    mut servers: ResMut<Servers>,
    mut states: ResMut<States>,
    mut target: ResMut<Target>,
    mut session: ResMut<Session>,
    mut opts: ResMut<Opts>,
    mut ui: ResMut<Ui>,
    player: Res<Player>,
    conn: Option<Res<Conn>>,
    fields: Query<(&Field, &EditableText)>,
    time: Res<Time>,
    mut commands: Commands,
) {
    for UiAction(a) in actions.read() {
        match a {
            Action::AddServer => {
                let addr = field_text(&fields, Field::Server);
                ui.server_bad = !addr.trim().is_empty() && crate::servers::candidates(&addr).is_empty();
                if servers.add(&addr) {
                    states.refresh();
                    crate::settings::save_soon(&mut commands);
                }
            }
            Action::RemoveServer(addr) => {
                servers.remove(addr);
                crate::settings::save_soon(&mut commands);
            }
            Action::Connect(addr) => {
                let base = match states.get(addr) {
                    Status::Up(i) => Some(i.base),
                    _ => crate::servers::candidates(addr).into_iter().next(),
                };
                let Some(base) = base else { continue };
                servers.last = addr.clone();
                crate::settings::save_soon(&mut commands);
                let identity = opts
                    .token
                    .clone()
                    .or_else(|| Some(player.identity.clone()).filter(|s| !s.is_empty()));
                *session = Session::default();
                target.0 = Some(base.clone());
                crate::net::open(&mut commands, &opts, identity, base, time.elapsed_secs());
            }
            Action::LeaveServer => {
                if let Some(c) = &conn {
                    crate::net::close(&mut commands, c);
                }
                target.0 = None;
                *session = Session::default();
                opts.room = None;
                opts.practice = None;
                ui.menu = false;
                ui.home_tab = HomeTab::Rooms;
                states.refresh();
            }
            _ => {}
        }
    }
}

use lightyear::prelude::client::Client;
use lightyear::prelude::*;
