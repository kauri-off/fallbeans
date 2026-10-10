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

use bevy::picking::hover::Hovered;

use super::home::{name_row, practice_fold, tabs};
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
                    outfit.run_if(resource_changed::<Player>.or_else(resource_changed::<Folds>)),
                    hover_rows,
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
    Body,
    Settings,
    Dev,
}

/// The menu's card, sized by what it shows.
#[derive(Component)]
struct MenuCard;

/// The way back from practice and its line; the room's strip with its name, access and way out.
#[derive(Component)]
struct PracticeTop;
#[derive(Component)]
struct PracticeLine;
#[derive(Component)]
struct RoomTop;
#[derive(Component)]
struct RoomTitle;
#[derive(Component)]
struct PrivateNote;
#[derive(Component)]
struct PinLine;

/// The lobby's columns (who is here, the setup), and the card of a game under way.
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

/// A row lit while hovered.
#[derive(Component)]
struct HoverRow;

fn build_menu(
    mut commands: Commands,
    layers: Res<Layers>,
    f: Res<Fonts>,
    me: Me,
    options: Options,
    folds: Res<Folds>,
    part: Res<Section>,
) {
    let f = &*f;
    let e = layers[Layer::Menu];
    let col = |gap: f32| Node {
        flex_direction: FlexDirection::Column,
        row_gap: px(gap),
        ..default()
    };
    commands.entity(e).with_children(|l| {
        // (The veil lets clicks through: a click on the game closes the menu.)
        l.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            BackgroundColor(DIM),
            Pickable::IGNORE,
        ));
        l.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexStart,
                padding: UiRect::top(px(22)),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|w| {
            w.spawn((
                MenuCard,
                Node {
                    width: px(1300),
                    max_width: percent(96),
                    height: percent(95),
                    flex_direction: FlexDirection::Column,
                    border_radius: BorderRadius::all(px(26)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                panel_raised(),
            ))
            .with_children(|m| {
                m.spawn(Node {
                    align_items: AlignItems::Center,
                    column_gap: px(20),
                    padding: UiRect::new(px(26), px(26), px(18), px(0)),
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_children(|h| {
                    tabs(
                        h,
                        f,
                        &[
                            (text::TAB_GAME, Action::MenuTab(MenuTab::Game)),
                            (text::TAB_SETTINGS, Action::MenuTab(MenuTab::Settings)),
                            (text::TAB_DEV, Action::MenuTab(MenuTab::Dev)),
                        ],
                    );
                    spacer(h);
                    resume(h, f);
                });
                let tab = || motion::Reveal::new(motion::Motion::slide(0.0, 12.0));
                m.spawn((Part::Top, col(0.0), tab())).with_children(|t| {
                    t.spawn((
                        PracticeTop,
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: px(20),
                            padding: UiRect::new(px(30), px(30), px(24), px(30)),
                            ..default()
                        },
                    ))
                    .with_children(|p| {
                        row(p, false, |r| {
                            let here = r.target_entity();
                            r.commands().entity(here).insert(Node {
                                column_gap: px(18),
                                align_items: AlignItems::Center,
                                ..default()
                            });
                            r.spawn((
                                Node {
                                    width: px(64),
                                    height: px(64),
                                    border_radius: BorderRadius::all(px(18)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    flex_shrink: 0.0,
                                    ..default()
                                },
                                BackgroundColor(genre_tones(fb_shared::game::Genre::Race).1),
                            ))
                            .with_children(|b| {
                                let e = bean(b, me.color().map_or(APRICOT, suit), 40.0);
                                b.commands().entity(e).insert(super::home::OwnBean);
                            });
                            let t = rich(r, f, "", 18.0, INK);
                            r.commands().entity(t).insert((
                                PracticeLine,
                                Node {
                                    flex_shrink: 1.0,
                                    ..default()
                                },
                            ));
                        });
                        row(p, false, |r| {
                            button(r, f, text::PRACTICE_BACK, Look::Primary, Action::EndPractice);
                        });
                    });
                    t.spawn((
                        RoomTop,
                        Node {
                            align_items: AlignItems::Center,
                            column_gap: px(22),
                            margin: UiRect::new(px(26), px(26), px(14), px(0)),
                            padding: UiRect::new(px(18), px(12), px(10), px(10)),
                            border_radius: BorderRadius::all(px(18)),
                            ..default()
                        },
                        BackgroundColor(CARD2),
                    ))
                    .with_children(|r| {
                        r.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            flex_shrink: 1.0,
                            min_width: px(0),
                            ..default()
                        })
                        .with_children(|t| {
                            caption(t, f, text::SECTION_ROOM);
                            let title = rich_in(t, f, "", 20.0, INK, true);
                            t.commands().entity(title).insert(RoomTitle);
                        });
                        r.spawn((
                            Node {
                                width: px(1),
                                align_self: AlignSelf::Stretch,
                                flex_shrink: 0.0,
                                ..default()
                            },
                            BackgroundColor(LINE),
                        ));
                        r.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: px(4),
                            flex_shrink: 1.0,
                            ..default()
                        })
                        .with_children(|a| {
                            button(
                                a,
                                f,
                                text::PRIVATE_ROOM_PIN,
                                Look::Toggle(false),
                                Action::Send(ClientMsg::Access { private: true }),
                            );
                            let pin = rich(a, f, "", 14.0, MUTED);
                            a.commands().entity(pin).insert((
                                PinLine,
                                Node {
                                    padding: UiRect::left(px(60)),
                                    ..default()
                                },
                            ));
                            let note = rich(a, f, "", 14.0, MUTED);
                            a.commands().entity(note).insert(PrivateNote);
                        });
                        spacer(r);
                        button(r, f, text::LEAVE_ROOM, Look::Danger, Action::LeaveRoom);
                    });
                });
                m.spawn((
                    Part::Body,
                    Node {
                        flex_grow: 1.0,
                        min_height: px(0),
                        column_gap: px(18),
                        padding: UiRect::new(px(26), px(26), px(16), px(24)),
                        ..default()
                    },
                    tab(),
                ))
                .with_children(|b| {
                    b.spawn(Node {
                        flex_basis: px(450),
                        flex_shrink: 1.0,
                        min_width: px(300),
                        flex_direction: FlexDirection::Column,
                        min_height: px(0),
                        ..default()
                    })
                    .with_children(|c| {
                        let g = group(c, |g| {
                            subheading(g, f, text::SECTION_BEAN);
                            name_row(g, f, Field::MenuName, &me.name(), me.color().map_or(APRICOT, suit));
                            g.spawn((Swatches, col(8.0))).with_children(|s| {
                                caption(s, f, text::COLOR);
                                s.spawn(Node {
                                    flex_wrap: FlexWrap::Wrap,
                                    column_gap: px(8),
                                    row_gap: px(8),
                                    align_items: AlignItems::Center,
                                    ..default()
                                })
                                .with_children(|r| {
                                    for i in 0..COLORS.len() as u8 {
                                        swatch(r, suit(i), false, Action::Color(i), true, 30.0);
                                    }
                                });
                            });
                            g.spawn((
                                OutfitBox,
                                Node {
                                    flex_direction: FlexDirection::Column,
                                    flex_grow: 1.0,
                                    min_height: px(0),
                                    row_gap: px(12),
                                    overflow: Overflow::scroll_y(),
                                    ..default()
                                },
                                bevy::ui_widgets::ScrollArea,
                            ));
                        });
                        c.commands().entity(g).insert(Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: px(12),
                            flex_grow: 1.0,
                            min_height: px(0),
                            padding: UiRect::axes(px(18), px(16)),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(px(18)),
                            ..default()
                        });
                    });
                    b.spawn((
                        LobbyPart,
                        Node {
                            flex_grow: 1.0,
                            flex_basis: px(0),
                            min_width: px(538),
                            column_gap: px(18),
                            ..default()
                        },
                    ))
                    .with_children(|l| {
                        l.spawn((
                            Node {
                                flex_basis: px(340),
                                flex_shrink: 1.0,
                                min_width: px(240),
                                flex_direction: FlexDirection::Column,
                                row_gap: px(12),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            bevy::ui_widgets::ScrollArea,
                        ))
                        .with_children(|c| {
                            c.spawn((
                                PlayersBox,
                                Node {
                                    flex_grow: 1.0,
                                    flex_shrink: 0.0,
                                    flex_direction: FlexDirection::Column,
                                    ..default()
                                },
                            ));
                            group(c, |g| practice_fold(g, f, &folds));
                        });
                        l.spawn((
                            Node {
                                flex_grow: 1.0,
                                flex_basis: px(0),
                                min_width: px(280),
                                flex_direction: FlexDirection::Column,
                                row_gap: px(12),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            bevy::ui_widgets::ScrollArea,
                        ))
                        .with_children(|c| {
                            c.spawn((
                                HostBox,
                                Node {
                                    flex_grow: 1.0,
                                    flex_shrink: 0.0,
                                    flex_direction: FlexDirection::Column,
                                    ..default()
                                },
                            ));
                        });
                    });
                    b.spawn((
                        PhaseBox,
                        Node {
                            flex_grow: 1.0,
                            flex_basis: px(0),
                            min_width: px(0),
                            flex_direction: FlexDirection::Column,
                            ..default()
                        },
                    ));
                });
                m.spawn((
                    Part::Settings,
                    Node {
                        flex_grow: 1.0,
                        min_height: px(0),
                        padding: UiRect::new(px(0), px(0), px(14), px(10)),
                        ..default()
                    },
                    tab(),
                ))
                .with_children(|s| settings_tab(s, f, &options, *part));
                m.spawn((
                    Part::Dev,
                    Node {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::axes(px(30), px(16)),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    bevy::ui_widgets::ScrollArea,
                    tab(),
                ))
                .with_children(|d| {
                    d.spawn((DevBox, col(0.0)));
                });
            });
        });
    });
}

/// Back to the game: Esc, or a click here.
fn resume(p: &mut ChildSpawnerCommands, f: &Fonts) {
    let look = Look::Icon;
    let face = (
        Node {
            column_gap: px(8),
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(10), px(4)),
            border_radius: BorderRadius::all(px(12)),
            ..default()
        },
        BackgroundColor(Color::NONE),
    );
    button_shell(p, look, Action::Resume, true, 0.0, face, |b| {
        let k = keycap(b, f, "Esc");
        b.commands().entity(k).insert(Pickable::IGNORE);
        rich_with(b, f, text::RESUME_HINT, 14.0, FAINT, false, Some(Pickable::IGNORE));
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

/// Each part on its tab; the dev tab only on a dev server. The card is as high as the screen, but for practice's
/// and the dev tools' few lines.
fn parts(
    tab: Option<Res<State<MenuTab>>>,
    session: Res<Session>,
    mut parts: Query<(&Part, &mut Node)>,
    mut tabs: Query<(&Act, &mut Node), Without<Part>>,
    mut card: Single<&mut Node, CardOnly>,
) {
    let tab = tab.map(|t| *t.get());
    for (part, mut node) in &mut parts {
        let on = match part {
            Part::Top => tab == Some(MenuTab::Game),
            Part::Body => tab == Some(MenuTab::Game) && !session.practice,
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
    let small = tab == Some(MenuTab::Game) && session.practice;
    let short = small || tab == Some(MenuTab::Dev);
    let (w, h) = (
        px(if small { 760 } else { 1300 }),
        if short { Val::Auto } else { percent(95) },
    );
    if card.width != w {
        card.width = w;
    }
    if card.height != h {
        card.height = h;
        card.max_height = percent(95);
    }
    let top = if small { px(120) } else { px(0) };
    if card.margin.top != top {
        card.margin.top = top;
    }
}

type CardOnly = (With<MenuCard>, Without<Part>, Without<Act>);
type Tops = (
    Without<PracticeTop>,
    Without<RoomTop>,
    Without<PrivateNote>,
    Without<PinLine>,
);
type PracticeOnly = (With<PracticeTop>, Without<RoomTop>);
type NoteOnly = (
    With<PrivateNote>,
    Without<PracticeTop>,
    Without<RoomTop>,
    Without<PinLine>,
);
type PinOnly = (
    With<PinLine>,
    Without<PracticeTop>,
    Without<RoomTop>,
    Without<PrivateNote>,
);

/// The menu's top: practice's, the room's with its title, the access and the PIN, and their buttons.
#[derive(SystemParam)]
struct Top<'w, 's> {
    practice: Single<'w, 's, &'static mut Node, PracticeOnly>,
    line: Single<'w, 's, Entity, With<PracticeLine>>,
    room: Single<'w, 's, &'static mut Node, (With<RoomTop>, Without<PracticeTop>)>,
    title: Single<'w, 's, Entity, With<RoomTitle>>,
    note: Single<'w, 's, (Entity, &'static mut Node), NoteOnly>,
    pin: Single<'w, 's, (Entity, &'static mut Node), PinOnly>,
    buttons: Query<'w, 's, (Entity, &'static mut Act, &'static mut Look, &'static mut Node), Tops>,
}

/// The room's name and the way out; for the host, private or public and the PIN. In practice: the way back.
fn top(session: Res<Session>, mut top: Top, mut labels: Labels) {
    show(&mut top.practice, session.practice);
    show(&mut top.room, !session.practice && session.lobby.is_some());
    if session.practice {
        let title = session
            .arena
            .as_ref()
            .map_or("", |a| fb_maps::by_id(a.game).meta().title);
        if let Ok(mut t) = labels.texts.get_mut(*top.line) {
            t.set(&text::practice_now(title));
        }
    }
    let host = session.host();
    let private = session.lobby.as_ref().is_some_and(|l| l.room.private);
    if let Some(l) = &session.lobby {
        let lock = if private { "🔒 " } else { "" };
        if let Ok(mut t) = labels.texts.get_mut(*top.title) {
            t.set(&format!("{lock}{}", l.room.title));
        }
    }
    let pin = session
        .lobby
        .as_ref()
        .and_then(|l| l.pin.as_ref())
        .filter(|_| host && private);
    let (pin_e, ref mut pin_node) = *top.pin;
    show(pin_node, pin.is_some());
    if let Some(p) = pin
        && let Ok(mut t) = labels.texts.get_mut(pin_e)
    {
        t.set(&text::pin_line(p));
    }
    let (note_e, ref mut note_node) = *top.note;
    show(note_node, !host);
    if !host && let Ok(mut t) = labels.texts.get_mut(note_e) {
        let host_name = session
            .lobby
            .as_ref()
            .and_then(|l| l.host)
            .map(|id| session.name_of(id));
        let s = if private {
            text::PRIVATE_NOTE.to_string()
        } else {
            text::hosted_by(host_name.as_deref())
        };
        t.set(&s);
    }
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
                look.set_if_neq(Look::Toggle(private));
                if !matches!(act.0, Action::Send(ClientMsg::Access { private: p }) if p != private) {
                    act.0 = Action::Send(ClientMsg::Access { private: !private });
                }
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
        let g = group(p, |g| {
            subheading(g, f, &text::players_of(l.players.len(), l.max));
            let m = meter(g, l.players.len() as f32 / l.max.max(1) as f32, TEAL, percent(100));
            g.commands().entity(m).insert(Node {
                width: percent(100),
                height: px(6),
                margin: UiRect::bottom(px(6)),
                border_radius: BorderRadius::MAX,
                overflow: Overflow::clip(),
                flex_shrink: 0.0,
                ..default()
            });
            for pl in &l.players {
                player_row(g, f, l, pl, me, host);
            }
        });
        p.commands().entity(g).insert(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            flex_grow: 1.0,
            padding: UiRect::axes(px(18), px(16)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(18)),
            ..default()
        });
    });
}

/// A game under way: which round, or the results or podium; the host may abort it.
fn phase_line(
    session: Res<Session>,
    q: Single<Entity, With<PhaseBox>>,
    f: Res<Fonts>,
    mut shown: Local<Option<(String, bool, bool)>>,
    mut commands: Commands,
) {
    let Some(l) = session.lobby.as_ref().filter(|l| l.phase != Phase::Lobby) else {
        return;
    };
    let round = session.arena.as_ref().filter(|a| a.kind == ArenaKind::Round);
    let line = match round {
        Some(a) => {
            let title = fb_maps::by_id(a.game).meta().title;
            text::round_now(a.index, a.total, title)
        }
        _ if l.phase == Phase::Podium => text::PODIUM_NOW.into(),
        _ => text::RESULTS_NOW.into(),
    };
    let next = Some((line, session.host(), round.is_some()));
    if *shown == next {
        return;
    }
    *shown = next;
    let Some((line, host, round)) = &*shown else { return };
    let f = &*f;
    rebuild(&mut commands, *q, |p| {
        let g = group(p, |g| {
            badge(g, f, text::IN_GAME, APRICOT_SOFT, APRICOT_INK);
            rich_in(g, f, line, 24.0, INK, true);
            if *round {
                rich(g, f, text::MENU_NO_PAUSE, 16.0, FAINT);
            }
            if *host {
                let b = button(g, f, text::ABORT, Look::Danger, Action::Send(ClientMsg::Abort));
                g.commands().entity(b).insert(Node {
                    margin: UiRect::top(px(12)),
                    ..default()
                });
            }
        });
        p.commands().entity(g).insert(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(14),
            flex_grow: 1.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::FlexStart,
            padding: UiRect::axes(px(34), px(30)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(18)),
            ..default()
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
        HoverRow,
        Hovered::default(),
        Node {
            column_gap: px(10),
            row_gap: px(4),
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            min_height: px(46),
            padding: UiRect::axes(px(6), px(4)),
            border_radius: BorderRadius::all(px(12)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(Color::NONE),
    ))
    .with_children(|r| {
        bean(r, suit(pl.color), 30.0);
        r.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(90),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|n| {
            row(n, false, |x| {
                let here = x.target_entity();
                x.commands().entity(here).insert(Node {
                    column_gap: px(0),
                    ..default()
                });
                let live = pl.connected || pl.bot;
                let n = rich_in(x, f, &pl.name, 15.5, if live { INK } else { GHOST }, true);
                x.commands().entity(n).insert(TextLayout::no_wrap());
                if mine {
                    rich(x, f, text::YOU, 15.5, FAINT);
                }
            });
            let (line, ink) = if pl.bot {
                (text::BOT.to_string(), FAINT)
            } else if pl.connected {
                (text::ping(pl.ping), FAINT)
            } else {
                (text::NO_LINK.into(), CRITICAL)
            };
            rich_in(n, f, &line, 13.0, ink, !pl.connected && !pl.bot);
        });
        // (Beside the name, or under it in a narrow column.)
        r.spawn(Node {
            column_gap: px(6),
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            margin: UiRect::left(Val::Auto),
            ..default()
        })
        .with_children(|a| {
            if Some(pl.id) == l.host {
                rich(a, f, "⭐", 15.0, INK);
            }
            if pl.crowns > 0 {
                badge(a, f, &format!("👑 {}", pl.crowns), CARD2, MUTED);
            }
            // (With "fill with bots" on, a bot taken out would be replaced at once.)
            if pl.bot && host && !l.fill {
                button(a, f, "×", Look::Icon, Action::Send(ClientMsg::RemoveBot(pl.id)));
            }
            if !pl.bot && host && !mine && pl.connected {
                button(a, f, text::GIVE_HOST, Look::Tiny, Action::Send(ClientMsg::Host(pl.id)));
            }
        });
    });
}

type HoveredRows = (With<HoverRow>, Changed<Hovered>);

/// A hovered row is lit.
fn hover_rows(mut rows: Query<(&Hovered, &mut BackgroundColor), HoveredRows>) {
    for (hovered, mut bg) in &mut rows {
        bg.set_if_neq(BackgroundColor(if hovered.get() { CARD2 } else { Color::NONE }));
    }
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
            let g = group(p, |g| {
                spinner(g, 30.0, TEAL);
                rich_in(g, f, text::HOST_STARTS, 17.0, INK, true);
                muted(g, f, &text::setup_line(&l.playlist));
            });
            p.commands().entity(g).insert(Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(14),
                flex_grow: 1.0,
                min_height: px(220),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                padding: UiRect::axes(px(18), px(16)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(18)),
                ..default()
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
    let g = group(p, |g| {
        subheading(g, f, text::SECTION_GAME);
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
            stack(g, |c| {
                caption(c, f, &text::custom_count(pl.games.len()));
                row(c, true, |r| {
                    let here = r.target_entity();
                    r.commands().entity(here).insert(Node {
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: px(6),
                        row_gap: px(6),
                        ..default()
                    });
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
                        button(r, f, &s, Look::SmallChip(at.is_some()), act);
                    }
                });
            });
        } else {
            row(g, true, |r| {
                let here = r.target_entity();
                r.commands().entity(here).insert(Node {
                    column_gap: px(12),
                    align_items: AlignItems::Center,
                    ..default()
                });
                rich_in(r, f, text::ROUNDS, 16.0, INK, true);
                row(r, false, |c| {
                    for n in ROUND_COUNTS {
                        button(
                            c,
                            f,
                            &n.to_string(),
                            Look::Chip(pl.rounds == n),
                            with(&|p| p.rounds = n),
                        );
                    }
                });
            });
        }
        button(
            g,
            f,
            text::FILL_BOTS,
            Look::Toggle(l.fill),
            Action::Send(ClientMsg::Fill(!l.fill)),
        );
        spacer(g);
        row(g, false, |r| {
            let here = r.target_entity();
            r.commands().entity(here).insert(Node {
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(10),
                row_gap: px(10),
                margin: UiRect::top(px(12)),
                ..default()
            });
            if !l.fill && (l.players.len() as u32) < l.max {
                button(r, f, text::ADD_BOT, Look::Plain, Action::Send(ClientMsg::AddBot));
            }
            let s = if enough {
                text::START_GAME.to_string()
            } else {
                text::need_players(l.min)
            };
            let b = button_if(r, f, &s, Look::Go, Action::Send(ClientMsg::Start), enough);
            r.commands().entity(b).insert(Node {
                flex_grow: 1.0,
                ..default()
            });
        });
    });
    p.commands().entity(g).insert(Node {
        flex_direction: FlexDirection::Column,
        row_gap: px(14),
        flex_grow: 1.0,
        padding: UiRect::axes(px(18), px(16)),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(18)),
        ..default()
    });
}

/// The player's look: a picture of the bean while folded; hat, glasses and colours of the hat, belly and shoes
/// (each choice carries the whole outfit: redrawn when it changes).
fn outfit(
    player: Res<Player>,
    folds: Res<Folds>,
    me: Me,
    session: Res<Session>,
    q: Single<Entity, With<OutfitBox>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    let outfit = player.outfit();
    let c = super::home::own_color(&me, &session);
    rebuild(&mut commands, *q, |p| outfit_picker(p, f, &folds, &outfit, c));
}

fn outfit_picker(p: &mut ChildSpawnerCommands, f: &Fonts, folds: &Folds, outfit: &fb_proto::Outfit, c: Color) {
    if !folds.open(Fold::Outfit) {
        p.spawn((
            Node {
                flex_grow: 1.0,
                min_height: px(120),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexEnd,
                padding: UiRect::bottom(px(18)),
                border_radius: BorderRadius::all(px(16)),
                ..default()
            },
            BackgroundColor(CARD2),
        ))
        .with_children(|v| {
            let b = bean(v, c, 124.0);
            v.commands().entity(b).insert(super::home::OwnBean);
        });
    }
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::top(px(10)),
            border: UiRect::top(px(1)),
            flex_shrink: 0.0,
            ..default()
        },
        BorderColor::all(HAIR),
    ))
    .with_children(|w| {
        let note = format!("{} · {}", text::hat(outfit.hat), text::glasses(outfit.glasses));
        let note = if folds.open(Fold::Outfit) { "" } else { note.as_str() };
        fold(w, f, folds, Fold::Outfit, text::OUTFIT, note, |c| {
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
    });
}

fn tints(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    title: &str,
    now: Option<Tint>,
    act: &dyn Fn(Option<Tint>) -> Action,
) {
    row(p, false, |c| {
        let here = c.target_entity();
        c.commands().entity(here).insert(Node {
            column_gap: px(10),
            align_items: AlignItems::Center,
            ..default()
        });
        c.spawn(Node {
            width: px(96),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|t| {
            caption(t, f, title);
        });
        row(c, true, |r| {
            let here = r.target_entity();
            r.commands().entity(here).insert(Node {
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(7),
                row_gap: px(7),
                flex_shrink: 1.0,
                ..default()
            });
            // "As designed": the part's own colour.
            swatch_none(r, now.is_none(), act(None), 26.0);
            for t in Tint::ALL {
                swatch(r, color(t.rgb()), now == Some(t), act(Some(t)), true, 26.0);
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
    let goto = |to: Goto| d(DevCmd::Goto { id: None, to });
    rebuild(&mut commands, *q, |p| {
        if !session.dev {
            return;
        }
        let mut times: Vec<(String, Action)> = RATES
            .iter()
            .map(|&k| {
                let s = if k == 0.0 { "⏸".to_string() } else { format!("×{k}") };
                (s, d(DevCmd::Rate { k }))
            })
            .collect();
        times.push(("+1 тик".into(), d(DevCmd::Step { ticks: 1 })));
        times.push(("+0,5 с".into(), d(DevCmd::Step { ticks: 60 })));
        dev_row(p, f, "Время:", times);
        dev_row(
            p,
            f,
            "Раунд",
            vec![
                ("Пропустить заставку".into(), d(DevCmd::SkipIntro)),
                ("+10 с".into(), d(DevCmd::Warp { s: 10.0 })),
                ("Завершить раунд".into(), d(DevCmd::EndRound)),
                ("В лобби".into(), d(DevCmd::Lobby)),
            ],
        );
        let mut tp = vec![("старт".to_string(), goto(Goto::Spawn))];
        tp.extend((0..checkpoints).map(|i| (format!("КТ {i}"), goto(Goto::Checkpoint(i as u32)))));
        if finish {
            tp.push(("финиш".into(), goto(Goto::Finish)));
        }
        dev_row(p, f, "Телепорт:", tp);
        dev_row(
            p,
            f,
            "Боты",
            vec![
                ("+ бот рядом".into(), d(DevCmd::Bot { n: None, near: true })),
                ("Заморозить ботов".into(), d(DevCmd::Bots { on: false })),
                ("Разморозить ботов".into(), d(DevCmd::Bots { on: true })),
                (
                    "Сбить меня".into(),
                    d(DevCmd::Knock {
                        id: None,
                        v: [0.0, 6.0, 8.0],
                    }),
                ),
                ("Выбыть".into(), d(DevCmd::Kill { id: None })),
            ],
        );
        p.spawn(Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::top(px(14)),
            ..default()
        })
        .with_children(|c| {
            fold(c, f, &folds, Fold::DevMaps, text::PLAY_MAP, "", |c| {
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
    });
}

/// A row of dev tools under its label.
fn dev_row(p: &mut ChildSpawnerCommands, f: &Fonts, title: &str, items: Vec<(String, Action)>) {
    p.spawn((
        Node {
            align_items: AlignItems::Center,
            flex_wrap: FlexWrap::Wrap,
            column_gap: px(8),
            row_gap: px(8),
            padding: UiRect::axes(px(0), px(10)),
            border: UiRect::bottom(px(1)),
            ..default()
        },
        BorderColor::all(HAIR),
    ))
    .with_children(|r| {
        r.spawn(Node {
            width: px(96),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|l| {
            rich_in(l, f, title, 16.0, MUTED, true);
        });
        for (s, a) in items {
            button(r, f, &s, Look::Chip(false), a);
        }
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
