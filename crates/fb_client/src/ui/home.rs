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
                    update_box.run_if(resource_exists_and_changed::<Update>.or_else(run_once)),
                    crash_box.run_if(resource_changed::<LastCrash>),
                    own_beans.run_if(resource_changed::<Player>.or_else(resource_changed::<Session>)),
                    servers.run_if(resource_changed::<Servers>.or_else(resource_changed::<States>)),
                    server_add.run_if(resource_changed::<Servers>.or_else(resource_changed::<Form>)),
                    rooms.run_if(
                        resource_changed::<RoomList>
                            .or_else(resource_changed::<Session>)
                            .or_else(resource_changed::<Servers>)
                            .or_else(resource_changed::<States>),
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
struct CrashBox;
#[derive(Component)]
struct ServersBox;
#[derive(Component)]
struct ServersList;
#[derive(Component)]
struct ServerBad;
#[derive(Component)]
struct RoomsBox;
#[derive(Component)]
struct RoomsTop;
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
        row_gap: px(gap),
        ..default()
    }
}

/// A column that scrolls when what it holds is higher than the screen.
fn scroll_column(gap: f32) -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(gap),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(0),
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            ..default()
        },
        bevy::ui_widgets::ScrollArea,
    )
}

/// The right-hand column of the home screens.
fn aside() -> Node {
    Node {
        width: px(440),
        max_width: percent(40),
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        row_gap: px(14),
        ..default()
    }
}

fn build_home(
    mut commands: Commands,
    layers: Res<Layers>,
    f: Res<Fonts>,
    me: Me,
    options: Options,
    folds: Res<Folds>,
    part: Res<Section>,
) {
    let f = &*f;
    commands.entity(layers[Layer::Banner]).insert(BannerBox);
    commands.entity(layers[Layer::Home]).with_children(|l| {
        l.spawn((
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(PAPER),
        ))
        .with_children(|c| {
            c.spawn(Node {
                height: px(84),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                column_gap: px(40),
                padding: UiRect::axes(px(56), px(0)),
                ..default()
            })
            .with_children(|h| {
                logo(h, f, 22.0);
                tabs(
                    h,
                    f,
                    &[
                        (text::TAB_SERVERS, Action::HomeTab(HomeTab::Main)),
                        (text::TAB_SETTINGS, Action::HomeTab(HomeTab::Settings)),
                    ],
                );
            });
            c.spawn(Node {
                flex_grow: 1.0,
                min_height: px(0),
                padding: UiRect::new(px(56), px(56), px(4), px(40)),
                ..default()
            })
            .with_children(|main| {
                let tab = || motion::Reveal::new(motion::Motion::slide(0.0, 12.0));
                main.spawn((
                    ServersBox,
                    Node {
                        flex_grow: 1.0,
                        column_gap: px(28),
                        min_height: px(0),
                        ..default()
                    },
                    tab(),
                ))
                .with_children(|t| {
                    t.spawn(scroll_column(14.0)).with_children(|left| {
                        left.spawn((CrashBox, column(0.0)));
                        left.spawn(Node {
                            padding: UiRect::new(px(4), px(4), px(6), px(2)),
                            ..default()
                        })
                        .with_children(|h| {
                            rich_in(h, f, text::SERVERS, 30.0, INK, true);
                        });
                        left.spawn((ServersList, column(14.0)));
                    });
                    t.spawn(aside()).with_children(|side| {
                        side.spawn((UpdateBox, column(0.0)));
                        section(side, f, text::ADD_SERVER, |p| {
                            row(p, false, |r| {
                                field_with_hint(r, f, Field::Server, "", text::SERVER_PLACEHOLDER, 120, None);
                                button(r, f, text::ADD, Look::Primary, Action::AddServer);
                            });
                            let bad = rich(p, f, text::SERVER_BAD, 14.0, CRITICAL);
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
                });
                main.spawn((
                    RoomsBox,
                    Node {
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(14),
                        min_height: px(0),
                        ..default()
                    },
                    tab(),
                ))
                .with_children(|t| {
                    t.spawn((RoomsTop, column(14.0)));
                    t.spawn(Node {
                        flex_grow: 1.0,
                        column_gap: px(28),
                        min_height: px(0),
                        ..default()
                    })
                    .with_children(|cols| {
                        cols.spawn(scroll_column(14.0)).with_children(|left| {
                            left.spawn((RoomsList, column(14.0)));
                            spacer(left);
                            practice_list(left, f, &folds);
                        });
                        cols.spawn(aside()).with_children(|side| {
                            section(side, f, text::OWN_ROOM, |g| {
                                g.spawn((OwnRoom, column(14.0)));
                                g.spawn((NewRoom, column(14.0))).with_children(|n| {
                                    field_with_hint(
                                        n,
                                        f,
                                        Field::RoomTitle,
                                        "",
                                        &text::room_title_placeholder(&me.name()),
                                        ROOM_TITLE_MAX,
                                        None,
                                    );
                                    button(n, f, text::PRIVATE_CREATE, Look::Toggle(false), Action::CreatePrivate);
                                    button(n, f, text::CREATE_ROOM, Look::Primary, Action::CreateRoom);
                                });
                            });
                            section(side, f, text::SECTION_NAME, |n| {
                                name_row(n, f, Field::Name, &me.name(), me.color().map_or(APRICOT, suit));
                            });
                        });
                    });
                    t.spawn((
                        PinBox,
                        Node {
                            position_type: PositionType::Absolute,
                            width: percent(100),
                            height: percent(100),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                });
                main.spawn((
                    SettingsBox,
                    Node {
                        flex_grow: 1.0,
                        min_height: px(0),
                        ..default()
                    },
                    tab(),
                ))
                .with_children(|t| {
                    card(
                        t,
                        Node {
                            flex_grow: 1.0,
                            padding: UiRect::axes(px(0), px(10)),
                            min_height: px(0),
                            ..default()
                        },
                    )
                    .with_children(|c| settings_tab(c, f, &options, *part));
                });
            });
        });
    });
}

/// Tabs as one segmented bar.
pub fn tabs(p: &mut ChildSpawnerCommands, f: &Fonts, items: &[(&str, Action)]) {
    p.spawn((
        Node {
            column_gap: px(4),
            padding: UiRect::all(px(4)),
            border_radius: BorderRadius::all(px(16)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(WELL),
    ))
    .with_children(|t| {
        for (i, (s, a)) in items.iter().enumerate() {
            button(t, f, s, Look::Tab(i == 0), a.clone());
        }
    });
}

/// The player's name: kept in the settings and told to the server (at the room list or in a room).
pub fn name_row(p: &mut ChildSpawnerCommands, f: &Fonts, which: Field, name: &str, color: Color) {
    p.spawn(Node {
        column_gap: px(8),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|r| {
        // (The menu shows the bean itself under the name.)
        if which != Field::MenuName {
            let b = bean(r, color, 40.0);
            r.commands().entity(b).insert(OwnBean);
            r.spawn(Node {
                width: px(6),
                ..default()
            });
        }
        field_with_hint(r, f, which, name, text::NAME_PLACEHOLDER, NAME_MAX, None);
        button(r, f, text::SAVE, Look::Plain, Action::SaveName(which));
    });
}

/// A bean in the player's own colour.
#[derive(Component)]
pub(super) struct OwnBean;

/// The player's beans follow their colour: the room's, or the one they asked for.
fn own_beans(me: Me, session: Res<Session>, mut beans: Query<&mut BackgroundColor, With<OwnBean>>) {
    let c = own_color(&me, &session);
    for mut b in &mut beans {
        b.set_if_neq(BackgroundColor(c));
    }
}

/// The player's suit colour.
pub(super) fn own_color(me: &Me, session: &Session) -> Color {
    session
        .me
        .and_then(|id| session.player(id))
        .map(|p| p.color)
        .or_else(|| me.color())
        .map_or(APRICOT, suit)
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
            Layer::Hud => room && !ui.menu,
            Layer::Tags => room,
            Layer::Chat => room && !session.practice && !ui.menu,
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
    if *shown == next {
        return;
    }
    *shown = next;
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        card(
            p,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
                padding: UiRect::axes(px(20), px(16)),
                ..default()
            },
        )
        .with_children(|c| update_state(c, f, &view, can));
    });
}

/// The game crashed last time: what to do, and where the report is.
fn crash_box(q: Single<Entity, With<CrashBox>>, crash: Res<LastCrash>, f: Res<Fonts>, mut commands: Commands) {
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        let Some(c) = &crash.0 else { return };
        let report = c.report.display().to_string();
        let msg = if c.gpu_lost {
            text::gpu_lost(&report)
        } else {
            text::crashed(&report)
        };
        p.spawn((
            Node {
                column_gap: px(16),
                align_items: AlignItems::FlexStart,
                padding: UiRect::axes(px(20), px(18)),
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            BackgroundColor(APRICOT_SOFT),
            motion::Motion::pop(0.95),
        ))
        .with_children(|n| {
            n.spawn((
                Node {
                    width: px(32),
                    height: px(32),
                    border_radius: BorderRadius::MAX,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(hex(0xEDC9B2)),
            ))
            .with_children(|i| {
                rich_in(i, f, "!", 16.0, APRICOT_INK, true);
            });
            n.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_width: px(0),
                ..default()
            })
            .with_children(|c| {
                let t = rich(c, f, &msg, 15.0, hex(0x4A3426));
                wrap_anywhere(c, t);
                row(c, false, |r| {
                    button(r, f, text::OPEN_LOGS, Look::Plain, Action::OpenLogs);
                });
            });
        });
    });
}

/// `can`: the release can be installed here, so a failed try may be tried again.
fn update_state(p: &mut ChildSpawnerCommands, f: &Fonts, view: &UpdateView, can: bool) {
    let line = |p: &mut ChildSpawnerCommands, s: &str, ink: Color| {
        let t = rich_in(p, f, s, 16.0, ink, true);
        p.commands().entity(t).insert(Node {
            flex_grow: 1.0,
            flex_shrink: 1.0,
            ..default()
        });
        wrap_anywhere(p, t);
    };
    match view {
        UpdateView::State(UpdateState::Idle) => {
            muted(p, f, &text::version(&fb_net::build()));
        }
        UpdateView::State(UpdateState::Available(r)) => {
            row(p, false, |row_| {
                line(row_, &text::update_out(&r.version), INK);
                if r.installable() {
                    button(row_, f, text::UPDATE, Look::Primary, Action::Update);
                } else {
                    button(row_, f, text::DOWNLOAD, Look::Primary, Action::ReleasePage);
                }
            });
        }
        UpdateView::Downloading(pct) | UpdateView::State(UpdateState::Downloading(pct, _)) => {
            line(p, &text::downloading(*pct, 100), INK);
            meter(p, *pct as f32 / 100.0, TEAL, percent(100));
        }
        UpdateView::State(UpdateState::Failed(err)) => {
            line(p, &text::update_failed(err), CRITICAL);
            row(p, false, |r| {
                if can {
                    button(r, f, text::RETRY, Look::Primary, Action::Update);
                }
                button(r, f, text::RELEASE_PAGE, Look::Plain, Action::ReleasePage);
            });
        }
        UpdateView::State(UpdateState::Restarting) => {
            row(p, false, |r| {
                spinner(r, 18.0, TEAL);
                line(r, text::RESTARTING, INK);
            });
        }
    }
}

/// A card with nothing in it yet: an icon in a circle over a line.
fn empty_card(p: &mut ChildSpawnerCommands, f: &Fonts, icon: Option<&str>, s: &str) {
    card(
        p,
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(18),
            padding: UiRect::all(px(40)),
            min_height: px(260),
            ..default()
        },
    )
    .with_children(|c| {
        match icon {
            Some(i) => {
                c.spawn((
                    Node {
                        width: px(84),
                        height: px(84),
                        border_radius: BorderRadius::MAX,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(CARD2),
                ))
                .with_children(|b| {
                    rich(b, f, i, 36.0, FAINT);
                });
            }
            None => {
                spinner(c, 30.0, TEAL);
            }
        }
        c.spawn(Node {
            max_width: px(420),
            ..default()
        })
        .with_children(|t| {
            let e = rich(t, f, s, 17.0, MUTED);
            t.commands().entity(e).insert(TextLayout::justify(Justify::Center));
        });
    });
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
        if list.list.is_empty() {
            empty_card(p, f, Some("🌐"), text::NO_SERVERS);
        }
        for (i, addr) in list.ordered().iter().enumerate() {
            let e = server_row(p, f, addr, &states.get(addr));
            p.commands()
                .entity(e)
                .insert(motion::Motion::slide(0.0, 10.0).after(i as f32 * 0.04));
        }
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
    card(
        p,
        Node {
            column_gap: px(18),
            align_items: AlignItems::Center,
            padding: UiRect::new(px(22), px(18), px(18), px(18)),
            flex_shrink: 0.0,
            ..default()
        },
    )
    .with_children(|row_| {
        row_.spawn((
            Node {
                width: px(14),
                height: px(14),
                border: UiRect::all(px(2)),
                border_radius: BorderRadius::MAX,
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(if up { TEAL } else { Color::NONE }),
            BorderColor::all(if up { TEAL } else { hex(0xB8AEA1) }),
            Outline::new(px(if up { 4 } else { 0 }), px(0), TEAL_SOFT),
        ));
        row_.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(0),
            ..default()
        })
        .with_children(|c| {
            row(c, true, |r| {
                let here = r.target_entity();
                r.commands().entity(here).insert(Node {
                    column_gap: px(12),
                    row_gap: px(6),
                    flex_wrap: FlexWrap::Wrap,
                    align_items: AlignItems::Center,
                    ..default()
                });
                rich_in(r, f, &title, 18.0, INK, true);
                match status {
                    Status::Checking => {
                        r.spawn((
                            Node {
                                column_gap: px(6),
                                align_items: AlignItems::Center,
                                padding: UiRect::axes(px(10), px(3)),
                                border_radius: BorderRadius::MAX,
                                ..default()
                            },
                            BackgroundColor(CARD2),
                        ))
                        .with_children(|b| {
                            spinner(b, 12.0, TEAL);
                            rich_in(b, f, text::CHECKING, 13.0, MUTED, true);
                        });
                    }
                    Status::Down => {
                        badge(r, f, text::SERVER_DOWN, CRITICAL_SOFT, CRITICAL);
                    }
                    Status::Up(_) if can => {
                        badge(r, f, text::ONLINE, TEAL_SOFT, hex(0x2C5850));
                    }
                    Status::Up(_) => {
                        let (_, fill, ink) = genre_tones(fb_shared::game::Genre::Survival);
                        badge(r, f, text::SERVER_OTHER_VERSION, fill, ink);
                    }
                }
            });
            let more = match (&line, title == addr) {
                (Some(l), true) => Some(l.clone()),
                (Some(l), false) => Some(format!("{l} · {addr}")),
                (None, false) => Some(addr.to_string()),
                (None, true) => None,
            };
            if let Some(m) = more {
                muted(c, f, &m);
            }
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
        button(row_, f, "×", Look::Icon, Action::RemoveServer(addr.to_string()));
    })
    .id()
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

/// The way back to the servers, the server's address, a refusal; and the rooms.
/// The server played on, as the room list shows it.
#[derive(SystemParam)]
struct ServerView<'w> {
    conn: Option<Res<'w, Conn>>,
    servers: Res<'w, Servers>,
    states: Res<'w, States>,
}

fn rooms(
    top: Single<Entity, With<RoomsTop>>,
    q: Single<Entity, (With<RoomsList>, Without<RoomsTop>)>,
    session: Res<Session>,
    list: Res<RoomList>,
    view: ServerView,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    let ServerView { conn, servers, states } = view;
    let alert = session
        .denied
        .as_ref()
        .filter(|d| d.reason != DenyReason::Pin)
        .and_then(|d| d.msg.as_deref());
    let server = conn.map(|c| servers.last.clone().unwrap_or_else(|| c.http.clone()));
    rebuild(&mut commands, *top, |p| {
        row(p, false, |r| {
            let here = r.target_entity();
            r.commands().entity(here).insert(Node {
                column_gap: px(14),
                align_items: AlignItems::Center,
                ..default()
            });
            button(r, f, text::TO_SERVERS, Look::Plain, Action::LeaveServer);
            if let Some(s) = &server {
                let line = match states.get(s) {
                    Status::Up(i) if i.name.as_ref().is_some_and(|n| n != s) => {
                        format!("{} · {s}", i.name.unwrap_or_default())
                    }
                    _ => s.clone(),
                };
                rich(r, f, &line, 16.0, FAINT);
            }
        });
        if let Some(msg) = alert {
            p.spawn((
                Node {
                    padding: UiRect::axes(px(16), px(12)),
                    border_radius: BorderRadius::all(px(14)),
                    ..default()
                },
                BackgroundColor(CRITICAL_SOFT),
                motion::Motion::pop(0.95),
            ))
            .with_children(|a| {
                rich_in(a, f, msg, 15.0, hex(0x7E2E25), true);
            });
        }
    });
    rebuild(&mut commands, *q, |p| {
        p.spawn(Node {
            padding: UiRect::axes(px(4), px(0)),
            ..default()
        })
        .with_children(|t| {
            heading(t, f, &text::rooms_count(list.rooms.as_ref().map_or(0, Vec::len)));
        });
        match &list.rooms {
            None => empty_card(p, f, None, text::LOADING_ROOMS),
            Some(l) if l.is_empty() => empty_card(p, f, Some("🚪"), text::NO_ROOMS),
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
    let mut c = card(
        p,
        Node {
            column_gap: px(16),
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(16), px(14)),
            border: UiRect::all(px(if mine { 2 } else { 0 })),
            flex_shrink: 0.0,
            ..default()
        },
    );
    c.insert(BorderColor::all(APRICOT));
    c.with_children(|row_| {
        match r.host.as_deref() {
            Some(h) => avatar(row_, f, h, host_color(h), 44.0),
            None => avatar(row_, f, "—", WELL, 44.0),
        };
        row_.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(2),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(0),
            ..default()
        })
        .with_children(|c| {
            let lock = if r.private { "🔒 " } else { "" };
            rich_in(c, f, &format!("{lock}{}", r.title), 17.0, INK, true);
            row(c, true, |x| {
                let here = x.target_entity();
                x.commands().entity(here).insert(Node {
                    column_gap: px(0),
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                });
                muted(
                    x,
                    f,
                    &text::room_line(r.host.as_deref(), r.phase != Phase::Lobby, false),
                );
                if mine {
                    rich_in(x, f, &format!(" · {}", text::YOUR_ROOM), 14.0, APRICOT_INK, true);
                }
            });
        });
        row_.spawn(Node {
            width: px(120),
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|c| {
            row(c, false, |x| {
                let here = x.target_entity();
                x.commands().entity(here).insert(Node {
                    column_gap: px(6),
                    align_items: AlignItems::Baseline,
                    ..default()
                });
                rich_in(x, f, &format!("{}/{}", r.players, r.max), 15.0, INK, true);
                if r.bots > 0 {
                    rich(x, f, &format!("+{}🤖", r.bots), 14.0, FAINT);
                }
            });
            let frac = r.players as f32 / r.max.max(1) as f32;
            meter(c, frac, if full { CRITICAL } else { TEAL }, percent(100));
        });
        let s = if full { text::NO_PLACES } else { text::ENTER };
        let b = button_if(row_, f, s, Look::Primary, Action::Join(r.id.clone()), !full);
        row_.commands().entity(b).insert(Node {
            width: px(112),
            flex_shrink: 0.0,
            ..default()
        });
    });
    c.id()
}

/// A host's avatar colour, the same for the same name.
fn host_color(name: &str) -> Color {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    suit((h % fb_shared::COLORS.len() as u32) as u8)
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
        dialog(p, |g| {
            let t = rich_in(g, f, &format!("{} «{title}»", text::PIN_LOCKED), 21.0, INK, true);
            g.commands().entity(t).insert(Node {
                margin: UiRect::bottom(px(20)),
                ..default()
            });
            row(g, false, |r| {
                let here = r.target_entity();
                r.commands().entity(here).insert(Node {
                    column_gap: px(10),
                    align_items: AlignItems::Center,
                    ..default()
                });
                r.spawn(Node {
                    width: px(170),
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_children(|w| {
                    field_with_hint(
                        w,
                        f,
                        Field::Pin,
                        "",
                        text::PIN_PLACEHOLDER,
                        ROOM_PIN_DIGITS,
                        Some(|c: char| c.is_ascii_digit()),
                    );
                });
                button(r, f, text::ENTER, Look::Primary, Action::SubmitPin(room.clone()));
                button(r, f, text::CANCEL, Look::Plain, Action::CancelPin);
            });
            let note = match &d.msg {
                None => muted(g, f, text::PIN_ASK),
                Some(m) => rich_in(g, f, m, 14.0, CRITICAL, true),
            };
            g.commands().entity(note).insert(Node {
                margin: UiRect::top(px(14)),
                ..default()
            });
        });
    });
}

/// A card in the middle of the screen over a veil.
fn dialog(p: &mut ChildSpawnerCommands, inner: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn((
        Node {
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.188, 0.157, 0.125, 0.38)),
    ))
    .with_children(|c| {
        c.spawn((
            Node {
                width: px(520),
                max_width: percent(90),
                flex_direction: FlexDirection::Column,
                padding: UiRect::new(px(32), px(32), px(30), px(28)),
                border_radius: BorderRadius::all(px(24)),
                ..default()
            },
            panel_raised(),
            motion::Motion::pop(0.9),
        ))
        .with_children(inner);
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
            button(g, f, text::BACK_TO_OWN, Look::Warm, Action::Join(id.clone()));
            muted(g, f, text::OWN_ROOM_NOTE);
        }
    });
}

fn private_box(form: Res<Form>, mut checks: Query<(&Act, &mut Look)>) {
    for (act, mut look) in &mut checks {
        if let Action::CreatePrivate = act.0 {
            look.set_if_neq(Look::Toggle(form.private));
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

/// One map against bots, alone: a tile for each, in a card that folds.
pub fn practice_list(p: &mut ChildSpawnerCommands, f: &Fonts, folds: &Folds) {
    card(
        p,
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::axes(px(18), px(16)),
            flex_shrink: 0.0,
            ..default()
        },
    )
    .with_children(|c| practice_fold(c, f, folds));
}

/// The practice tiles, as many to a row as fit.
pub fn practice_fold(p: &mut ChildSpawnerCommands, f: &Fonts, folds: &Folds) {
    let note = text::maps_count(fb_maps::GAMES.len());
    fold(p, f, folds, Fold::Practice, text::PRACTICE, &note, |c| {
        c.spawn(Node {
            display: bevy::ui::Display::Grid,
            grid_template_columns: vec![RepeatedGridTrack::minmax(
                GridTrackRepetition::AutoFill,
                MinTrackSizingFunction::Px(130.0),
                MaxTrackSizingFunction::Fraction(1.0),
            )],
            column_gap: px(8),
            row_gap: px(8),
            ..default()
        })
        .with_children(|r| {
            for g in fb_maps::GAMES {
                let m = g.meta();
                tile(r, f, m.title, m.genre, Action::Practice(m.id));
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
    /// Connecting or reconnecting: the line, what to, and whether the game goes on behind.
    Wait(&'static str, Option<String>, bool),
}

/// What the banner tells of.
#[derive(SystemParam)]
struct BannerFacts<'w> {
    session: Res<'w, Session>,
    list: Res<'w, RoomList>,
    conn: Option<Res<'w, Conn>>,
    target: Res<'w, Target>,
    update: Option<Res<'w, Update>>,
    servers: Res<'w, Servers>,
    states: Res<'w, States>,
}

impl BannerFacts<'_> {
    /// The server being reached: its name and address.
    fn server(&self) -> Option<String> {
        let addr = self.servers.last.clone()?;
        Some(match self.states.get(&addr) {
            Status::Up(i) => match i.name.as_ref().filter(|n| **n != addr) {
                Some(n) => format!("{n} · {addr}"),
                None => addr,
            },
            _ => addr,
        })
    }
}

/// Connecting, reconnecting, the game being updated, or refused: a card in the middle.
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
    let in_room = facts.session.room.is_some() && facts.session.arena.is_some();
    let show = if facts.target.0.is_none() {
        Show::None
    } else if let Some(msg) = &facts.session.reject {
        Show::Card(Some(msg.clone()), true)
    } else if facts.session.refused {
        Show::Outdated(offer)
    } else if !online && ever {
        let room = facts
            .session
            .lobby
            .as_ref()
            .map(|l| l.room.title.clone())
            .filter(|t| !t.is_empty());
        Show::Wait(text::RECONNECTING, room.or_else(|| facts.server()), in_room)
    } else if !online || (facts.session.room.is_none() && facts.list.rooms.is_none()) {
        Show::Wait(text::CONNECTING, facts.server(), in_room)
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
        Show::Outdated(offer) => away(p, false, |card| {
            logo(card, f, 26.0);
            rich_in(card, f, text::OUTDATED, 19.0, INK, true);
            if !matches!(offer, Offer::Page) {
                card.spawn((
                    Node {
                        width: percent(100),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(10),
                        padding: UiRect::axes(px(18), px(16)),
                        border_radius: BorderRadius::all(px(16)),
                        ..default()
                    },
                    BackgroundColor(CARD2),
                ))
                .with_children(|u| {
                    let view = match &offer {
                        Offer::Downloading(pct) => UpdateView::Downloading(*pct),
                        Offer::Failed(e, _) => UpdateView::State(UpdateState::Failed(e.clone())),
                        Offer::Restarting => UpdateView::State(UpdateState::Restarting),
                        Offer::Install | Offer::Page => UpdateView::State(UpdateState::Idle),
                    };
                    if matches!(offer, Offer::Install) {
                        row(u, false, |r| {
                            rich_in(r, f, text::UPDATE_READY, 16.0, INK, true);
                        });
                    } else {
                        update_state(u, f, &view, matches!(offer, Offer::Failed(_, true)));
                    }
                });
            }
            row(card, false, |r| {
                // (No «Скачать» while the update is on its way: a second copy from the page would race it.)
                match &offer {
                    Offer::Install => {
                        button(r, f, text::UPDATE, Look::Primary, Action::Update);
                    }
                    Offer::Page => {
                        button(r, f, text::DOWNLOAD, Look::Primary, Action::ReleasePage);
                    }
                    Offer::Failed(_, false) => {
                        button(r, f, text::DOWNLOAD, Look::Primary, Action::ReleasePage);
                    }
                    Offer::Failed(_, true) | Offer::Downloading(_) | Offer::Restarting => {}
                }
                button(r, f, text::TO_SERVERS, Look::Plain, Action::LeaveServer);
            });
        }),
        Show::Wait(s, what, in_room) => away(p, in_room, |card| {
            spinner(card, 30.0, TEAL);
            rich_in(card, f, s, 20.0, INK, true);
            if let Some(w) = what {
                muted(card, f, &w);
            }
            let b = button(card, f, text::CANCEL, Look::Plain, Action::LeaveServer);
            card.commands().entity(b).insert(Node {
                margin: UiRect::top(px(6)),
                ..default()
            });
        }),
        Show::Card(more, quit) => away(p, false, |card| {
            logo(card, f, 26.0);
            if let Some(m) = more {
                let t = rich_in(card, f, &m, 19.0, INK, true);
                card.commands().entity(t).insert((
                    Node {
                        max_width: px(460),
                        ..default()
                    },
                    TextLayout::justify(Justify::Center),
                ));
            }
            // (Always a way back: a server being updated may take long or not come back.)
            row(card, false, |r| {
                let here = r.target_entity();
                r.commands().entity(here).insert(Node {
                    column_gap: px(10),
                    margin: UiRect::top(px(6)),
                    ..default()
                });
                button(r, f, text::TO_SERVERS, Look::Plain, Action::LeaveServer);
                if quit {
                    button(r, f, text::QUIT, Look::Primary, Action::Quit);
                }
            });
        }),
    });
}

/// A card in the middle of the screen: over the paper, or over the game veiled (`over_game`).
fn away(p: &mut ChildSpawnerCommands, over_game: bool, inner: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn((
        Node {
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(if over_game { DIM } else { PAPER }),
    ))
    .with_children(|c| {
        c.spawn((
            Node {
                width: px(600),
                max_width: percent(90),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(18),
                padding: UiRect::new(px(44), px(44), px(40), px(36)),
                border_radius: BorderRadius::all(px(26)),
                ..default()
            },
            panel_raised(),
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
