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
use crate::servers::{ServerBook, Servers, States, Status, Target};
use crate::session::{Denied, RoomList, RoomState, Session};
use crate::settings::{Me, Player, Profile};
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
        l.spawn((
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Stretch,
                column_gap: rem(2.5),
                padding: UiRect::all(rem(1.5)),
                ..default()
            },
            BackgroundGradient::from(LinearGradient::to_right(vec![
                PLANE.with_alpha(0.9).into(),
                PLANE.with_alpha(0.55).into(),
                PLANE.with_alpha(0.15).into(),
            ])),
        ))
        .with_children(|c| {
            c.spawn((
                Node {
                    width: rem(34.0),
                    max_width: percent(100),
                    max_height: percent(100),
                    flex_direction: FlexDirection::Column,
                    row_gap: rem(1.0),
                    padding: UiRect::all(rem(1.5)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(rem(1.75)),
                    ..default()
                },
                glass(),
            ))
            .with_children(|panel| {
                head(
                    panel,
                    f,
                    false,
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
                        let tab = || motion::Reveal::new(motion::Motion::slide(0.0, 12.0));
                        body.spawn((ServersBox, column(1.25), tab())).with_children(|t| {
                            t.spawn((ServersList, column(0.625)));
                            section(t, f, text::ADD_SERVER, |p| {
                                row(p, false, |r| {
                                    field_with_hint(r, f, Field::Server, "", text::SERVER_PLACEHOLDER, 120, None);
                                    button(r, f, text::ADD, Look::Go, Action::AddServer);
                                });
                                let bad = rich(p, f, text::SERVER_BAD, 13.0, CRITICAL);
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
                        body.spawn((RoomsBox, column(1.25), tab())).with_children(|t| {
                            section(t, f, text::SECTION_NAME, |n| {
                                name_row(n, f, Field::Name, &me.name());
                            });
                            t.spawn((RoomsList, column(0.625)));
                            t.spawn((PinBox, column(0.625)));
                            section(t, f, text::OWN_ROOM, |g| {
                                g.spawn((OwnRoom, column(0.625)));
                                g.spawn((NewRoom, column(0.75))).with_children(|n| {
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
                        body.spawn((SettingsBox, column(0.0), tab()))
                            .with_children(|t| settings_tab(t, f, &options, &folds));
                    });
            });
            c.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::FlexEnd,
                    align_items: AlignItems::FlexEnd,
                    row_gap: rem(0.5),
                    flex_shrink: 1.0,
                    min_width: px(0),
                    padding: UiRect::all(rem(1.0)),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|h| {
                logo(h, f, 88.0);
                rich_in(h, f, text::TAGLINE, 17.0, INK.with_alpha(0.85), true);
            });
        });
    });
}

/// A screen's head: the logo (if `with_logo`) and its tabs as one segmented bar.
pub fn head(p: &mut ChildSpawnerCommands, f: &Fonts, with_logo: bool, items: &[(&str, Action)]) {
    p.spawn(Node {
        width: percent(100),
        flex_direction: FlexDirection::Column,
        row_gap: rem(1.0),
        ..default()
    })
    .with_children(|r| {
        if with_logo {
            logo(r, f, 24.0);
        }
        r.spawn((
            Node {
                column_gap: rem(0.25),
                padding: UiRect::all(rem(0.25)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(NEUTRAL),
            BorderColor::all(RIM),
        ))
        .with_children(|t| {
            for (i, (s, a)) in items.iter().enumerate() {
                let b = button(t, f, s, Look::Tab(i == 0), a.clone());
                t.commands().entity(b).insert(Node {
                    flex_grow: 1.0,
                    ..default()
                });
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

/// The home tab's parts.
type Panels = (
    Or<(With<ServersBox>, With<RoomsBox>, With<SettingsBox>)>,
    Without<Layer>,
);

/// The screen decides which layers are up, and the home tab which of its parts.
fn layers(
    mut q: Query<(&Layer, &mut Node)>,
    screen: Res<State<Screen>>,
    tab: Option<Res<State<HomeTab>>>,
    session: Res<Session>,
    ui: Res<Ui>,
    mut boxes: Query<(&mut Node, Has<ServersBox>, Has<RoomsBox>), Panels>,
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
    mut labels: Labels,
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
            labels.set(e, s);
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
            let t = rich(p, f, &msg, 13.0, CRITICAL);
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
                let t = rich(g, f, &text::update_failed(err), 13.0, CRITICAL);
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
        p.spawn(Node {
            padding: UiRect::left(rem(0.25)),
            ..default()
        })
        .with_children(|t| {
            caption(t, f, text::SERVERS);
        });
        if list.list.is_empty() {
            group(p, |g| {
                rich(g, f, "🌐", 28.0, INK);
                muted(g, f, text::NO_SERVERS);
            });
        }
        for (i, addr) in list.ordered().iter().enumerate() {
            let e = server_row(p, f, addr, &states.get(addr));
            p.commands()
                .entity(e)
                .insert(motion::Motion::slide(0.0, 10.0).after(i as f32 * 0.04));
        }
    });
}

/// A card of a list: an icon, two lines, and what may be done on the right.
fn list_card(p: &mut ChildSpawnerCommands, lit: bool, inner: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn((
        Node {
            column_gap: rem(0.875),
            align_items: AlignItems::Center,
            padding: UiRect::axes(rem(0.875), rem(0.75)),
            border: UiRect::all(px(if lit { 2 } else { 1 })),
            border_radius: BorderRadius::all(rem(1.125)),
            ..default()
        },
        BackgroundColor(if lit { BLUE.with_alpha(0.07) } else { GROUP }),
        BorderColor::all(if lit { BLUE } else { RIM }),
    ))
    .with_children(inner)
    .id()
}

/// A round icon of a list's card, lit while what it stands for is up.
fn list_icon(p: &mut ChildSpawnerCommands, f: &Fonts, icon: &str, lit: bool) {
    p.spawn((
        Node {
            width: rem(2.75),
            height: rem(2.75),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(0.875)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(if lit { BLUE.with_alpha(0.12) } else { ink_wash(0.05) }),
        BorderColor::all(if lit { BLUE.with_alpha(0.4) } else { RIM }),
    ))
    .with_children(|i| {
        rich(i, f, icon, 18.0, INK);
    });
}

fn server_row(p: &mut ChildSpawnerCommands, f: &Fonts, addr: &str, status: &Status) -> Entity {
    let (title, line, can) = match status {
        Status::Checking | Status::Down => (addr.to_string(), None, true),
        Status::Up(i) => {
            let title = i.name.clone().unwrap_or_else(|| addr.to_string());
            let same = i.protocol == fb_net::PROTOCOL_VERSION;
            let line = if !same {
                format!("{} ({})", text::SERVER_OTHER_VERSION, i.build)
            } else {
                text::server_line(i.players, i.rooms)
            };
            (title, Some(line), same)
        }
    };
    let up = matches!(status, Status::Up(_)) && can;
    list_card(p, false, |row_| {
        list_icon(row_, f, "🖥", up);
        row_.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: rem(0.25),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(0),
            ..default()
        })
        .with_children(|c| {
            rich_in(c, f, &title, 16.0, INK, true);
            row(c, true, |r| {
                match status {
                    Status::Checking => {
                        spinner(r, 0.75, MUTED);
                        muted(r, f, text::CHECKING);
                    }
                    Status::Down => {
                        badge(r, f, text::SERVER_DOWN, CRITICAL.with_alpha(0.16), CRITICAL);
                    }
                    Status::Up(_) if can => {
                        badge(r, f, text::ONLINE, BLUE.with_alpha(0.16), BLUE);
                    }
                    Status::Up(_) => {
                        badge(r, f, text::SERVER_OTHER_VERSION, WARNING.with_alpha(0.25), INK);
                    }
                }
                let more = match (&line, title == addr) {
                    (Some(l), true) if can => Some(l.clone()),
                    (Some(l), false) if can => Some(format!("{addr} · {l}")),
                    (_, false) => Some(addr.to_string()),
                    _ => None,
                };
                if let Some(m) = more {
                    muted(r, f, &m);
                }
            });
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
    })
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
            p.spawn((
                Node {
                    padding: UiRect::axes(rem(0.875), rem(0.625)),
                    border: UiRect::new(px(3), px(1), px(1), px(1)),
                    border_radius: BorderRadius::all(rem(0.75)),
                    ..default()
                },
                BackgroundColor(CRITICAL.with_alpha(0.1)),
                BorderColor {
                    left: CRITICAL,
                    ..BorderColor::all(CRITICAL.with_alpha(0.25))
                },
                motion::Motion::pop(0.95),
            ))
            .with_children(|a| {
                rich(a, f, msg, 14.0, CRITICAL);
            });
        }
        p.spawn(Node {
            padding: UiRect::new(rem(0.25), px(0), rem(0.5), px(0)),
            ..default()
        })
        .with_children(|t| {
            caption(t, f, &text::rooms_count(list.rooms.as_ref().map_or(0, Vec::len)));
        });
        match &list.rooms {
            None => {
                row(p, false, |r| {
                    spinner(r, 1.0, BLUE);
                    muted(r, f, text::LOADING_ROOMS);
                });
            }
            Some(l) if l.is_empty() => {
                group(p, |g| {
                    rich(g, f, "🚪", 28.0, INK);
                    muted(g, f, text::NO_ROOMS);
                });
            }
            Some(l) => {
                for (i, r) in l.iter().enumerate() {
                    let e = room_row(p, f, r, list.mine.as_deref() == Some(r.id.as_str()));
                    p.commands()
                        .entity(e)
                        .insert(motion::Motion::slide(0.0, 10.0).after(i as f32 * 0.04));
                }
            }
        }
    });
}

fn room_row(p: &mut ChildSpawnerCommands, f: &Fonts, r: &RoomInfo, mine: bool) -> Entity {
    let full = r.players >= r.max;
    list_card(p, mine, |row_| {
        let host = r.host.as_deref().unwrap_or("?");
        avatar(row_, f, host, if mine { BLUE } else { BLUE_DEEP }, 2.75);
        row_.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: rem(0.25),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(0),
            ..default()
        })
        .with_children(|c| {
            let lock = if r.private { "🔒 " } else { "" };
            rich_in(c, f, &format!("{lock}{}", r.title), 16.0, INK, true);
            muted(c, f, &text::room_line(r.host.as_deref(), r.phase != Phase::Lobby, mine));
        });
        row_.spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexEnd,
            row_gap: rem(0.3125),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|c| {
            let bots = if r.bots > 0 {
                format!(" +{}🤖", r.bots)
            } else {
                String::new()
            };
            rich_in(c, f, &format!("{}/{}{bots}", r.players, r.max), 13.0, INK, true);
            let frac = r.players as f32 / r.max.max(1) as f32;
            meter(c, frac, if full { CRITICAL } else { BLUE }, rem(3.5));
        });
        let s = if full { text::NO_PLACES } else { text::ENTER };
        button_if(row_, f, s, Look::Primary, Action::Join(r.id.clone()), !full);
    })
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
                Some(m) => rich(g, f, m, 13.0, CRITICAL),
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

/// One map against bots, alone: a tile for each.
pub fn practice_list(p: &mut ChildSpawnerCommands, f: &Fonts, folds: &Folds) {
    fold(p, f, folds, Fold::Practice, text::PRACTICE, |c| {
        // (Tiles of a row as high as its highest.)
        c.spawn(Node {
            flex_wrap: FlexWrap::Wrap,
            column_gap: rem(0.5),
            row_gap: rem(0.5),
            align_items: AlignItems::Stretch,
            ..default()
        })
        .with_children(|r| {
            for g in fb_maps::GAMES {
                let m = g.meta();
                tile(
                    r,
                    f,
                    m.title,
                    text::genre(m.genre),
                    genre_color(m.genre),
                    Action::Practice(m.id),
                );
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

/// What the banner tells of.
#[derive(SystemParam)]
struct BannerFacts<'w> {
    session: Res<'w, Session>,
    list: Res<'w, RoomList>,
    conn: Option<Res<'w, Conn>>,
    target: Res<'w, Target>,
    update: Option<Res<'w, Update>>,
}

/// Connecting, reconnecting, the game being updated, or refused: a card in the middle or a banner on top.
fn banner(
    q: Single<Entity, With<BannerBox>>,
    facts: BannerFacts,
    f: Res<Fonts>,
    mut shown: Local<Option<Show>>,
    mut commands: Commands,
) {
    let online = facts.conn.as_ref().is_some_and(|c| c.connected);
    let offer = match facts.update.as_ref().map(|u| (&u.state, u.can_install())) {
        Some((UpdateState::Available(_), true)) => Offer::Install,
        Some((UpdateState::Downloading(got, total), _)) => {
            Offer::Downloading((got * 100).checked_div(*total).unwrap_or(0))
        }
        Some((UpdateState::Failed(e), can)) => Offer::Failed(e.clone(), can),
        Some((UpdateState::Restarting, _)) => Offer::Restarting,
        _ => Offer::Page,
    };
    let ever = facts.conn.as_ref().is_some_and(|c| c.ever);
    let show = if facts.target.0.is_none() {
        Show::None
    } else if let Some(msg) = &facts.session.reject {
        Show::Card(Some(msg.clone()), true)
    } else if facts.session.refused {
        Show::Outdated(offer)
    } else if !online && ever {
        Show::Line(text::RECONNECTING)
    } else if !online || (facts.session.room.is_none() && facts.list.rooms.is_none()) {
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
            logo(card, f, 36.0);
            label(card, f, text::OUTDATED);
            match &offer {
                Offer::Downloading(pct) => {
                    label(card, f, &text::downloading(*pct, 100));
                }
                Offer::Failed(e, _) => {
                    let t = rich(card, f, &text::update_failed(e), 13.0, CRITICAL);
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
                padding: UiRect::top(rem(1.25)),
                ..default()
            })
            .with_children(|c| {
                c.spawn((
                    Node {
                        padding: UiRect::new(rem(1.0), rem(0.625), rem(0.625), rem(0.625)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    glass(),
                    motion::Motion::slide(0.0, -24.0),
                ))
                .with_children(|b| {
                    row(b, false, |r| {
                        spinner(r, 1.125, BLUE);
                        heading(r, f, s);
                        button(r, f, text::CANCEL, Look::Tiny, Action::LeaveServer);
                    });
                });
            });
        }
        Show::Card(more, quit) => card(p, |card| {
            logo(card, f, 36.0);
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
        BackgroundGradient::from(RadialGradient::new(
            UiPosition::CENTER,
            RadialGradientShape::FarthestCorner,
            vec![BLUE_SOFT.with_alpha(0.4).into(), ink_wash(0.35).into()],
        )),
    ))
    .with_children(|c| {
        c.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: rem(1.0),
                padding: UiRect::axes(rem(2.25), rem(2.0)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(rem(1.75)),
                max_width: rem(32.0),
                ..default()
            },
            glass(),
            motion::Motion::pop(0.9),
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

/// The interface's state, the forms' and their fields.
#[derive(SystemParam)]
struct Forms<'w, 's> {
    ui: ResMut<'w, Ui>,
    form: ResMut<'w, Form>,
    fields: Query<'w, 's, (&'static Field, &'static EditableText)>,
}

fn home_actions(
    mut actions: MessageReader<UiAction>,
    mut session: ResMut<Session>,
    mut profile: Profile,
    mut forms: Forms,
    conn: Option<ResMut<Conn>>,
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
                let n = fb_shared::text::sanitize_name(&field_text(&forms.fields, *which));
                if n.is_empty() {
                    continue;
                }
                profile.player.name = n.clone();
                profile.opts.name = None;
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
                let pin = field_text(&forms.fields, Field::Pin);
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
            Action::CreatePrivate => forms.form.private ^= true,
            Action::CreateRoom => {
                let mut title = fb_shared::text::sanitize_title(&field_text(&forms.fields, Field::RoomTitle));
                let name = profile.opts.name.clone().unwrap_or_else(|| profile.player.name.clone());
                if title.is_empty() && !name.is_empty() {
                    title = text::room_title_placeholder(&name);
                }
                send(
                    &mut senders,
                    ClientMsg::Create {
                        title,
                        private: forms.form.private,
                    },
                );
            }
            Action::Practice(id) => {
                let Some(c) = conn.as_deref_mut() else { continue };
                session.back_to = session.room.clone().filter(|r| !r.is_empty());
                profile.opts.practice = Some(*id);
                profile.opts.room = None;
                session.room = None;
                forms.ui.menu = false;
                crate::net::restart(&mut commands, c);
            }
            Action::EndPractice => {
                let Some(c) = conn.as_deref_mut() else { continue };
                profile.opts.practice = None;
                profile.opts.room = session.back_to.take();
                session.room = None;
                crate::net::restart(&mut commands, c);
            }
            Action::LeaveRoom => {
                forms.ui.menu = false;
                profile.opts.room = None;
                send(&mut senders, ClientMsg::Leave);
            }
            _ => {}
        }
    }
}

/// Adding, removing, entering and leaving servers.
fn server_actions(
    mut actions: MessageReader<UiAction>,
    mut book: ServerBook,
    mut rooms: RoomState,
    mut forms: Forms,
    mut opts: ResMut<Opts>,
    time: Res<Time<Real>>,
    mut commands: Commands,
) {
    let mut opened = false;
    for UiAction(a) in actions.read() {
        match a {
            Action::AddServer => {
                let addr = field_text(&forms.fields, Field::Server);
                forms.form.server_bad = !addr.trim().is_empty() && crate::servers::candidates(&addr).is_empty();
                if book.servers.add(&addr) {
                    book.states.refresh();
                    crate::settings::save_soon(&mut commands);
                }
            }
            Action::RemoveServer(addr) => {
                book.servers.remove(addr);
                crate::settings::save_soon(&mut commands);
            }
            Action::Connect(addr) => {
                // (A second click on the button in the frame it goes would open a second link.)
                if rooms.conn.is_some() || opened {
                    continue;
                }
                let base = match book.states.get(addr) {
                    Status::Up(i) => Some(i.base),
                    _ => crate::servers::candidates(addr).into_iter().next(),
                };
                let Some(base) = base else { continue };
                book.servers.last = Some(addr.clone());
                crate::settings::save_soon(&mut commands);
                // (This server's own: one never sees the player's identity on another.)
                let identity = opts.token.clone().or_else(|| book.ids.get(&base));
                *rooms.session = Session::default();
                *rooms.list = RoomList::default();
                rooms.view.clear();
                book.target.0 = Some(base.clone());
                crate::net::open(&mut commands, &opts, identity, base, time.elapsed_secs());
                opened = true;
            }
            Action::LeaveServer => {
                if let Some(c) = &rooms.conn {
                    crate::net::close(&mut commands, c);
                }
                book.target.0 = None;
                *rooms.session = Session::default();
                *rooms.list = RoomList::default();
                rooms.view.clear();
                opts.room = None;
                opts.practice = None;
                forms.ui.menu = false;
                book.states.refresh();
            }
            _ => {}
        }
    }
}

use lightyear::prelude::client::Client;
use lightyear::prelude::*;
