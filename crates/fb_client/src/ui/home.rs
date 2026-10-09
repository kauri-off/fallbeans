//! The first screen: the player's servers, then on one of them the player's name, the rooms to enter, a
//! room of one's own, practice; the options; and the screens of the connection (connecting, refused).
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use fb_proto::{ClientMsg, DenyReason, Phase, RoomInfo};
use fb_shared::{NAME_MAX, ROOM_PIN_DIGITS, ROOM_TITLE_MAX};

use super::*;
use crate::crash::LastCrash;
use crate::net::Conn;
use crate::opts::Opts;
use crate::servers::{Servers, States, Status, Target};
use crate::session::{Denied, RoomList, RoomView, Session};
use crate::settings::{Identities, Me, Player};
use crate::update::{State as UpdateState, Update};

pub struct HomePlugin;

impl Plugin for HomePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Form>();
        app.add_systems(Startup, build_home.after(super::setup));
        app.add_systems(
            Update,
            (
                (
                    layers.run_if(
                        state_changed::<Screen>
                            .or_else(state_changed::<HomeTab>)
                            .or_else(resource_changed::<Ui>)
                            .or_else(resource_changed::<Session>),
                    ),
                    tabs_follow.run_if(
                        state_changed::<Screen>
                            .or_else(state_changed::<HomeTab>)
                            .or_else(state_changed::<MenuTab>),
                    ),
                    update_box.run_if(resource_exists_and_changed::<Update>.or_else(resource_changed::<LastCrash>)),
                    servers.run_if(resource_changed::<Servers>.or_else(resource_changed::<States>)),
                    server_add.run_if(resource_changed::<Servers>.or_else(resource_changed::<Form>)),
                    rooms.run_if(
                        resource_changed::<RoomList>
                            .or_else(resource_changed::<Session>)
                            .or_else(resource_changed::<Servers>),
                    ),
                    pin.run_if(resource_changed::<Session>),
                    own_room.run_if(resource_changed::<RoomList>),
                    private_box.run_if(resource_changed::<Form>),
                    title_hint.run_if(resource_changed::<Player>.or_else(resource_changed::<Opts>)),
                    banner,
                ),
                enter_submits,
                (home_actions, server_actions),
            )
                .chain(),
        );
    }
}

/// The room list's forms: the private box of a room to create, and an address that is not one.
#[derive(Resource, Default)]
pub struct Form {
    pub private: bool,
    pub server_bad: bool,
}

#[derive(Component)]
struct UpdateBox;
#[derive(Component)]
struct ServersBox;
#[derive(Component)]
struct ServersList;
#[derive(Component)]
struct ServerBad;
#[derive(Component)]
struct RoomsBox;
#[derive(Component)]
struct RoomsList;
#[derive(Component)]
struct PinBox;
#[derive(Component)]
struct OwnRoom;
#[derive(Component)]
struct NewRoom;
#[derive(Component)]
struct SettingsBox;
#[derive(Component)]
struct BannerBox;

fn column(gap: f32) -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        row_gap: rem(gap),
        ..default()
    }
}

fn build_home(mut commands: Commands, layers: Res<Layers>, f: Res<Fonts>, me: Me, options: Options, folds: Res<Folds>) {
    let f = &*f;
    commands.entity(layers[Layer::Banner]).insert(BannerBox);
    commands.entity(layers[Layer::Home]).with_children(|l| {
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
                head(
                    panel,
                    f,
                    &[
                        (text::TAB_SERVERS, Action::HomeTab(HomeTab::Main)),
                        (text::TAB_SETTINGS, Action::HomeTab(HomeTab::Settings)),
                    ],
                );
                panel.spawn((UpdateBox, column(0.5)));
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
                        body.spawn((ServersBox, column(0.75))).with_children(|t| {
                            t.spawn((ServersList, column(0.625)));
                            t.spawn(column(0.5)).with_children(|p| {
                                row(p, false, |r| {
                                    field_with_hint(r, f, Field::Server, "", text::SERVER_PLACEHOLDER, 120, None);
                                    button(r, f, text::ADD, Look::Go, Action::AddServer);
                                });
                                let bad = rich(p, f, text::SERVER_BAD, 13.0, RED_INK);
                                p.commands().entity(bad).insert((
                                    ServerBad,
                                    Node {
                                        display: display(false),
                                        ..default()
                                    },
                                ));
                                muted(p, f, text::SERVER_HINT);
                            });
                        });
                        body.spawn((RoomsBox, column(0.75))).with_children(|t| {
                            name_row(t, f, Field::Name, &me.name());
                            t.spawn((RoomsList, column(0.625)));
                            t.spawn((PinBox, column(0.625)));
                            group(t, |g| {
                                g.spawn((OwnRoom, column(0.625)));
                                g.spawn((NewRoom, column(0.625))).with_children(|n| {
                                    heading(n, f, text::OWN_ROOM);
                                    field_with_hint(
                                        n,
                                        f,
                                        Field::RoomTitle,
                                        "",
                                        &text::room_title_placeholder(&me.name()),
                                        ROOM_TITLE_MAX,
                                        None,
                                    );
                                    button(n, f, text::PRIVATE_CREATE, Look::Check(false), Action::CreatePrivate);
                                    button(n, f, text::CREATE_ROOM, Look::Go, Action::CreateRoom);
                                });
                            });
                            practice_list(t, f, &folds);
                        });
                        body.spawn((SettingsBox, column(0.0)))
                            .with_children(|t| settings_tab(t, f, &options, &folds));
                    });
            });
        });
    });
}

/// A screen's head: the logo and its tabs.
pub fn head(p: &mut ChildSpawnerCommands, f: &Fonts, items: &[(&str, Action)]) {
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
        r.spawn((
            Node {
                column_gap: rem(0.25),
                padding: UiRect::all(rem(0.2)),
                border_radius: BorderRadius::all(rem(0.875)),
                ..default()
            },
            BackgroundColor(GROUP),
        ))
        .with_children(|t| {
            for (i, (s, a)) in items.iter().enumerate() {
                button(t, f, s, Look::Tab(i == 0), a.clone());
            }
        });
    });
}

/// The player's name: kept in the settings and told to the server (at the room list or in a room).
pub fn name_row(p: &mut ChildSpawnerCommands, f: &Fonts, which: Field, name: &str) {
    row(p, false, |r| {
        field_with_hint(r, f, which, name, text::NAME_PLACEHOLDER, NAME_MAX, None);
        button(r, f, text::SAVE, Look::Plain, Action::SaveName(which));
    });
}

/// The screen decides which layers are up, and the home tab which of its parts.
fn layers(
    mut q: Query<(&Layer, &mut Node)>,
    screen: Res<State<Screen>>,
    tab: Option<Res<State<HomeTab>>>,
    session: Res<Session>,
    ui: Res<Ui>,
    mut boxes: Query<
        (&mut Node, Has<ServersBox>, Has<RoomsBox>),
        (
            Or<(With<ServersBox>, With<RoomsBox>, With<SettingsBox>)>,
            Without<Layer>,
        ),
    >,
) {
    let screen = *screen.get();
    let room = screen == Screen::Room;
    for (layer, mut node) in &mut q {
        let on = match layer {
            Layer::Home => matches!(screen, Screen::Servers | Screen::Rooms),
            Layer::Banner => true,
            Layer::Hud | Layer::Tags => room,
            Layer::Chat => room && !session.practice,
            Layer::Menu => room && ui.menu,
        };
        show(&mut node, on);
    }
    let tab = tab.map(|t| *t.get());
    for (mut node, servers, rooms) in &mut boxes {
        let on = if servers {
            tab == Some(HomeTab::Main) && screen == Screen::Servers
        } else if rooms {
            tab == Some(HomeTab::Main) && screen == Screen::Rooms
        } else {
            tab == Some(HomeTab::Settings)
        };
        show(&mut node, on);
    }
}

/// The tabs of both screens show the one open; the room list's first is the servers' until one is entered.
fn tabs_follow(
    screen: Res<State<Screen>>,
    home: Option<Res<State<HomeTab>>>,
    menu: Option<Res<State<MenuTab>>>,
    mut tabs: Query<(Entity, &Act, &mut Look)>,
    children: Query<&Children>,
    mut texts: Query<&mut Rich>,
) {
    for (e, act, mut look) in &mut tabs {
        let on = match act.0 {
            Action::HomeTab(t) => home.as_ref().is_some_and(|h| *h.get() == t),
            Action::MenuTab(t) => menu.as_ref().is_some_and(|m| *m.get() == t),
            _ => continue,
        };
        look.set_if_neq(Look::Tab(on));
        if let Action::HomeTab(HomeTab::Main) = act.0 {
            let s = if *screen.get() == Screen::Rooms {
                text::TAB_ROOMS
            } else {
                text::TAB_SERVERS
            };
            relabel(e, s, &children, &mut texts);
        }
    }
}

/// What the update box shows: redrawn when that changes, not with every chunk of a download.
#[derive(Clone, PartialEq)]
enum UpdateView {
    State(UpdateState),
    /// Per cent.
    Downloading(u64),
}

/// A newer release, the download, or this build's version.
fn update_box(
    q: Single<Entity, With<UpdateBox>>,
    update: Option<Res<Update>>,
    crash: Res<LastCrash>,
    f: Res<Fonts>,
    mut shown: Local<Option<(UpdateView, bool)>>,
    mut commands: Commands,
) {
    let state = update.as_ref().map_or(UpdateState::Idle, |u| u.state.clone());
    let can = update.as_ref().is_some_and(|u| u.can_install());
    let view = match state {
        UpdateState::Downloading(got, total) => UpdateView::Downloading((got * 100).checked_div(total).unwrap_or(0)),
        s => UpdateView::State(s),
    };
    let next = Some((view.clone(), can));
    if *shown == next && !crash.is_changed() {
        return;
    }
    *shown = next;
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        update_state(p, f, &view, can);
        if let Some(c) = &crash.0 {
            let report = c.report.display().to_string();
            let msg = if c.gpu_lost {
                text::gpu_lost(&report)
            } else {
                text::crashed(&report)
            };
            let t = rich(p, f, &msg, 13.0, RED_INK);
            wrap_anywhere(p, t);
            button(p, f, text::OPEN_LOGS, Look::Tiny, Action::OpenLogs);
        }
    });
}

/// `can`: the release can be installed here, so a failed try may be tried again.
fn update_state(p: &mut ChildSpawnerCommands, f: &Fonts, view: &UpdateView, can: bool) {
    match view {
        UpdateView::State(UpdateState::Idle) => {
            muted(p, f, &text::version(&fb_net::build()));
        }
        UpdateView::State(UpdateState::Available(r)) => {
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
        UpdateView::Downloading(pct) | UpdateView::State(UpdateState::Downloading(pct, _)) => {
            label(p, f, &text::downloading(*pct, 100));
        }
        UpdateView::State(UpdateState::Failed(err)) => {
            group(p, |g| {
                let t = rich(g, f, &text::update_failed(err), 13.0, RED_INK);
                wrap_anywhere(g, t);
                row(g, false, |r| {
                    if can {
                        button(r, f, text::RETRY, Look::Go, Action::Update);
                    }
                    button(r, f, text::RELEASE_PAGE, Look::Plain, Action::ReleasePage);
                });
            });
        }
        UpdateView::State(UpdateState::Restarting) => {
            label(p, f, text::RESTARTING);
        }
    }
}

/// The player's servers, each with what it said last.
fn servers(
    q: Single<Entity, With<ServersList>>,
    list: Res<Servers>,
    states: Res<States>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        heading(p, f, text::SERVERS);
        if list.list.is_empty() {
            muted(p, f, text::NO_SERVERS);
        }
        for addr in list.ordered() {
            server_row(p, f, &addr, &states.get(&addr));
        }
    });
}

fn server_row(p: &mut ChildSpawnerCommands, f: &Fonts, addr: &str, status: &Status) {
    let (title, line, can) = match status {
        Status::Checking => (addr.to_string(), text::CHECKING.to_string(), true),
        Status::Down => (addr.to_string(), text::SERVER_DOWN.to_string(), true),
        Status::Up(i) => {
            let title = i.name.clone().unwrap_or_else(|| addr.to_string());
            let same = i.protocol == fb_net::PROTOCOL_VERSION;
            let line = if !same {
                format!("{} ({})", text::SERVER_OTHER_VERSION, i.build)
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

/// The field to add a server is emptied when the list changes (unless the address was not one), and says so then.
fn server_add(
    list: Res<Servers>,
    form: Res<Form>,
    mut fields: Query<(&Field, &mut EditableText)>,
    mut bad: Single<&mut Node, With<ServerBad>>,
    mut had: Local<Vec<String>>,
) {
    show(&mut bad, form.server_bad);
    if *had == list.list {
        return;
    }
    had.clone_from(&list.list);
    if !form.server_bad
        && let Some((_, mut t)) = fields.iter_mut().find(|(f, _)| **f == Field::Server)
    {
        set_field_text(&mut t, "");
    }
}

fn rooms(
    q: Single<Entity, With<RoomsList>>,
    session: Res<Session>,
    list: Res<RoomList>,
    conn: Option<Res<Conn>>,
    servers: Res<Servers>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    let alert = session
        .denied
        .as_ref()
        .filter(|d| d.reason != DenyReason::Pin)
        .and_then(|d| d.msg.as_deref());
    let server = conn.map(|c| servers.last.clone().unwrap_or_else(|| c.http.clone()));
    rebuild(&mut commands, *q, |p| {
        row(p, false, |r| {
            button(r, f, text::TO_SERVERS, Look::Tiny, Action::LeaveServer);
            if let Some(s) = &server {
                muted(r, f, s);
            }
        });
        if let Some(msg) = alert {
            rich(p, f, msg, 14.0, RED_INK);
        }
        heading(p, f, &text::rooms_count(list.rooms.as_ref().map_or(0, Vec::len)));
        match &list.rooms {
            None => {
                muted(p, f, text::LOADING_ROOMS);
            }
            Some(l) if l.is_empty() => {
                muted(p, f, text::NO_ROOMS);
            }
            Some(l) => {
                for r in l {
                    room_row(p, f, r, list.mine.as_deref() == Some(r.id.as_str()));
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
            muted(c, f, &text::room_line(r.host.as_deref(), r.phase != Phase::Lobby, mine));
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

/// A private room asked for its PIN: only its host knows it. (Redrawn, so emptied, when it asks again.)
fn pin(
    q: Single<Entity, With<PinBox>>,
    session: Res<Session>,
    list: Res<RoomList>,
    f: Res<Fonts>,
    mut shown: Local<Option<Denied>>,
    mut commands: Commands,
) {
    let asks = session.denied.as_ref().filter(|d| d.reason == DenyReason::Pin);
    if shown.as_ref() == asks {
        return;
    }
    shown.clone_from(&asks.cloned());
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        let Some(d) = asks else { return };
        let Some(room) = d.room.clone() else { return };
        let title = list
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
            match &d.msg {
                None => muted(g, f, text::PIN_ASK),
                Some(m) => rich(g, f, m, 13.0, RED_INK),
            };
        });
    });
}

/// Everyone may keep one room of their own: they are its host whenever they are in it. The form to create one
/// is shown while they have none.
fn own_room(
    own: Single<Entity, With<OwnRoom>>,
    mut new: Single<&mut Node, With<NewRoom>>,
    list: Res<RoomList>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    show(&mut new, list.mine.is_none());
    let f = &*f;
    rebuild(&mut commands, *own, |g| {
        if let Some(id) = &list.mine {
            button(g, f, text::BACK_TO_OWN, Look::Go, Action::Join(id.clone()));
            muted(g, f, text::OWN_ROOM_NOTE);
        }
    });
}

fn private_box(form: Res<Form>, mut checks: Query<(&Act, &mut Look)>) {
    for (act, mut look) in &mut checks {
        if let Action::CreatePrivate = act.0 {
            look.set_if_neq(Look::Check(form.private));
        }
    }
}

/// A room created without a title is named after its host.
fn title_hint(me: Me, fields: Query<&Field>, mut hints: Query<(&Placeholder, &mut Text)>) {
    let hint = text::room_title_placeholder(&me.name());
    for (p, mut t) in &mut hints {
        if fields.get(p.0).is_ok_and(|f| *f == Field::RoomTitle) && t.0 != hint {
            t.0.clone_from(&hint);
        }
    }
}

/// One map against bots, alone.
pub fn practice_list(p: &mut ChildSpawnerCommands, f: &Fonts, folds: &Folds) {
    fold(p, f, folds, Fold::Practice, text::PRACTICE, |c| {
        row(c, true, |r| {
            for g in fb_maps::GAMES {
                let m = g.meta();
                let s = format!("{} · {}", m.title, text::genre(m.genre));
                button(r, f, &s, Look::Chip(false), Action::Practice(m.id));
            }
        });
    });
}

/// What the out-of-date card offers: the update as it goes, or the release page.
#[derive(Clone, PartialEq)]
enum Offer {
    Page,
    Install,
    /// Per cent.
    Downloading(u64),
    /// Why, and whether it may be tried again.
    Failed(String, bool),
    Restarting,
}

/// What the banner layer shows: redrawn when that changes.
#[derive(Clone, PartialEq)]
enum Show {
    None,
    Card(Option<String>, bool),
    Outdated(Offer),
    Line(&'static str),
}

/// Connecting, reconnecting, the game being updated, or refused: a card in the middle or a banner on top.
fn banner(
    q: Single<Entity, With<BannerBox>>,
    session: Res<Session>,
    list: Res<RoomList>,
    conn: Option<Res<Conn>>,
    target: Res<Target>,
    update: Option<Res<Update>>,
    f: Res<Fonts>,
    mut shown: Local<Option<Show>>,
    mut commands: Commands,
) {
    let online = conn.as_ref().is_some_and(|c| c.connected);
    let offer = match update.as_ref().map(|u| (&u.state, u.can_install())) {
        Some((UpdateState::Available(_), true)) => Offer::Install,
        Some((UpdateState::Downloading(got, total), _)) => {
            Offer::Downloading((got * 100).checked_div(*total).unwrap_or(0))
        }
        Some((UpdateState::Failed(e), can)) => Offer::Failed(e.clone(), can),
        Some((UpdateState::Restarting, _)) => Offer::Restarting,
        _ => Offer::Page,
    };
    let ever = conn.as_ref().is_some_and(|c| c.ever);
    let show = if target.0.is_none() {
        Show::None
    } else if let Some(msg) = &session.reject {
        Show::Card(Some(msg.clone()), true)
    } else if session.refused {
        Show::Outdated(offer)
    } else if !online && ever {
        Show::Line(text::RECONNECTING)
    } else if !online || (session.room.is_none() && list.rooms.is_none()) {
        Show::Line(text::CONNECTING)
    } else {
        Show::None
    };
    if shown.as_ref() == Some(&show) {
        return;
    }
    *shown = Some(show.clone());
    let e = *q;
    let f = &*f;
    rebuild(&mut commands, e, |p| match show {
        Show::None => {}
        Show::Outdated(offer) => card(p, |card| {
            logo(card, f, 30.0);
            label(card, f, text::OUTDATED);
            match &offer {
                Offer::Downloading(pct) => {
                    label(card, f, &text::downloading(*pct, 100));
                }
                Offer::Failed(e, _) => {
                    let t = rich(card, f, &text::update_failed(e), 13.0, RED_INK);
                    wrap_anywhere(card, t);
                }
                Offer::Restarting => {
                    label(card, f, text::RESTARTING);
                }
                Offer::Page | Offer::Install => {}
            }
            row(card, false, |r| {
                // (No «Скачать» while the update is on its way: a second copy from the page would race it.)
                match &offer {
                    Offer::Install => {
                        button(r, f, text::UPDATE, Look::Go, Action::Update);
                    }
                    Offer::Failed(_, true) => {
                        button(r, f, text::RETRY, Look::Go, Action::Update);
                        button(r, f, text::RELEASE_PAGE, Look::Plain, Action::ReleasePage);
                    }
                    Offer::Page | Offer::Failed(_, false) => {
                        button(r, f, text::DOWNLOAD, Look::Go, Action::ReleasePage);
                    }
                    Offer::Downloading(_) | Offer::Restarting => {}
                }
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
        Show::Card(more, quit) => card(p, |card| {
            logo(card, f, 30.0);
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
    mut form: ResMut<Form>,
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
                opts.name = None;
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
            Action::CreatePrivate => form.private ^= true,
            Action::CreateRoom => {
                let mut title = fb_shared::text::sanitize_title(&field_text(&fields, Field::RoomTitle));
                let name = opts.name.clone().unwrap_or_else(|| player.name.clone());
                if title.is_empty() && !name.is_empty() {
                    title = text::room_title_placeholder(&name);
                }
                send(
                    &mut senders,
                    ClientMsg::Create {
                        title,
                        private: form.private,
                    },
                );
            }
            Action::Practice(id) => {
                let Some(c) = conn.as_deref_mut() else { continue };
                session.back_to = session.room.clone().filter(|r| !r.is_empty());
                opts.practice = Some(*id);
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
    mut list: ResMut<RoomList>,
    mut view: RoomView,
    mut opts: ResMut<Opts>,
    mut ui: ResMut<Ui>,
    mut form: ResMut<Form>,
    ids: Res<Identities>,
    conn: Option<Res<Conn>>,
    fields: Query<(&Field, &EditableText)>,
    time: Res<Time<Real>>,
    mut commands: Commands,
) {
    let mut opened = false;
    for UiAction(a) in actions.read() {
        match a {
            Action::AddServer => {
                let addr = field_text(&fields, Field::Server);
                form.server_bad = !addr.trim().is_empty() && crate::servers::candidates(&addr).is_empty();
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
                // (A second click on the button in the frame it goes would open a second link.)
                if conn.is_some() || opened {
                    continue;
                }
                let base = match states.get(addr) {
                    Status::Up(i) => Some(i.base),
                    _ => crate::servers::candidates(addr).into_iter().next(),
                };
                let Some(base) = base else { continue };
                servers.last = Some(addr.clone());
                crate::settings::save_soon(&mut commands);
                // (This server's own: one never sees the player's identity on another.)
                let identity = opts.token.clone().or_else(|| ids.get(&base));
                *session = Session::default();
                *list = RoomList::default();
                view.clear();
                target.0 = Some(base.clone());
                crate::net::open(&mut commands, &opts, identity, base, time.elapsed_secs());
                opened = true;
            }
            Action::LeaveServer => {
                if let Some(c) = &conn {
                    crate::net::close(&mut commands, c);
                }
                target.0 = None;
                *session = Session::default();
                *list = RoomList::default();
                view.clear();
                opts.room = None;
                opts.practice = None;
                ui.menu = false;
                states.refresh();
            }
            _ => {}
        }
    }
}

use lightyear::prelude::client::Client;
use lightyear::prelude::*;
