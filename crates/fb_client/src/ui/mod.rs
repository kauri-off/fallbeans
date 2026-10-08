//! The player's interface on bevy_ui: the room list, the Esc menu, the HUD, the chat
//! and the name tags. Where the player is picks the screen (`Screen`, its tabs `HomeTab` and `MenuTab`). Each
//! screen is built once; what changes is set in place (`Rich` texts, a button's `Look`, folds shown or not), and
//! only lists are rebuilt, when the resource they show changes. Every button carries an `Action`; one observer
//! turns presses into `UiAction` messages for the module that owns them.
pub mod text;

mod chat;
mod home;
mod hud;
mod loading;
mod menu;
mod tags;

#[cfg(test)]
pub use home::Form;

use std::collections::BTreeSet;

use bevy::asset::AssetId;
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::input_focus::InputFocus;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle, TextEdit};
use bevy::ui::{InteractionDisabled, Pressed, UiSystems};
use bevy::ui_widgets::{Activate, Slider, SliderRange, SliderStep, SliderThumb, SliderValue, ValueChange};
use bevy::window::PrimaryWindow;
use fb_proto::{ClientMsg, Outfit};

use crate::keys::Bind;
use crate::render::upscale::Upscaler;
use crate::session::{RoomList, Session};
use crate::settings::{Bindings, Controls, Display, Graphics, Sound};
use crate::view::color;

const BOLD: &[u8] = include_bytes!("../../../../assets/fonts/Nunito-Bold.ttf");
const BLACK: &[u8] = include_bytes!("../../../../assets/fonts/Nunito-Black.ttf");
pub const EMOJI: &[u8] = include_bytes!("../../../../assets/fonts/NotoColorEmoji-subset.ttf");

pub const INK: Color = Color::srgb(0.169, 0.102, 0.361);
pub const MUTED: Color = Color::srgb(0.329, 0.271, 0.498);
pub const LOGO_PINK: Color = Color::srgb(0.816, 0.165, 0.451);
pub const PINK: Color = Color::srgb(0.831, 0.2, 0.478);
pub const GREEN: Color = Color::srgb(0.094, 0.525, 0.275);
pub const GREEN_INK: Color = Color::srgb(0.043, 0.373, 0.176);
pub const BLUE: Color = Color::srgb(0.094, 0.439, 0.733);
pub const RED: Color = Color::srgb(0.816, 0.161, 0.271);
pub const RED_INK: Color = Color::srgb(0.659, 0.082, 0.184);
pub const YELLOW: Color = Color::srgb(1.0, 0.824, 0.247);
pub const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.84);
pub const RIM: Color = Color::srgba(1.0, 1.0, 1.0, 0.8);
pub const GROUP: Color = Color::srgba(0.169, 0.102, 0.361, 0.06);
pub const SHADOW: Color = Color::srgba(0.169, 0.102, 0.361, 0.22);

/// Sizes are in rem: 16 px at the base scale, which follows the window (`UiScale`, `scale_ui`).
pub fn rem(x: f32) -> Val {
    px(x * 16.0)
}

/// A suit colour (index into `COLORS`) as a swatch; the rainbow shows its middle.
pub fn suit(i: u8) -> Color {
    match fb_shared::COLORS.get(i as usize) {
        Some(fb_shared::Suit::Color(c)) => color(*c),
        Some(fb_shared::Suit::Rainbow) | None => Color::srgb(1.0, 0.69, 0.25),
    }
}

#[derive(Resource, Clone)]
pub struct Fonts {
    pub bold: Handle<Font>,
    pub black: Handle<Font>,
    pub emoji: Handle<Font>,
}

/// Which screen is up: the player's servers, a server's rooms, a room; or none, under a banner (connecting,
/// refused, between the room list and a room's arena).
#[derive(States, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Screen {
    #[default]
    Servers,
    Rooms,
    Room,
    Away,
}

#[derive(SubStates, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[source(Screen = Screen::Servers | Screen::Rooms)]
pub enum HomeTab {
    #[default]
    Main,
    Settings,
}

#[derive(SubStates, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[source(Screen = Screen::Room)]
pub enum MenuTab {
    #[default]
    Game,
    Settings,
    Dev,
}

/// Who has the keyboard and the mouse.
#[derive(Resource, Clone, Default, PartialEq)]
pub struct Ui {
    /// The Esc menu is open: the mouse is free and the game takes no input.
    pub menu: bool,
    /// The chat line takes the keyboard.
    pub chat: bool,
    /// In play with the mouse free (focus lost): the field asks for a click.
    pub need_click: bool,
    /// The last device used was a gamepad (the hints follow it).
    pub pad: bool,
    /// Waiting for the key to bind to this action.
    pub rebinding: Option<Bind>,
}

/// A folded part of a screen (by name in BRP's `fb/ui`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fold {
    Gfx,
    Keys,
    Problems,
    Practice,
    Outfit,
    DevMaps,
}

/// The folded parts that are open.
#[derive(Resource, Default)]
pub struct Folds(BTreeSet<Fold>);

impl Folds {
    /// Opens a folded part, or folds it.
    pub fn toggle(&mut self, f: Fold) {
        if !self.0.remove(&f) {
            self.0.insert(f);
        }
    }

    pub fn open(&self, f: Fold) -> bool {
        self.0.contains(&f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    InvertMouse,
    InvertStick,
    Shake,
    ShowFps,
    Fullscreen,
    Vsync,
}

/// A graphics choice of several.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GfxPick {
    Preset(crate::render::quality::Preset),
    Fps(u32),
    Backend(Option<crate::opts::Backend>),
    Upscaler(Option<crate::render::upscale::Upscaler>),
    Upscale(crate::render::quality::Upscale),
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    MouseSens,
    StickSens,
    Fov,
    Volume,
    UiScale,
}

#[derive(Clone, Debug)]
pub enum Action {
    Send(ClientMsg),
    HomeTab(HomeTab),
    MenuTab(MenuTab),
    /// Opens or folds a folded part.
    Fold(Fold),
    SaveName(Field),
    Join(String),
    SubmitPin(String),
    CancelPin,
    CreatePrivate,
    CreateRoom,
    Practice(&'static str),
    EndPractice,
    Update,
    ReleasePage,
    AddServer,
    RemoveServer(String),
    Connect(String),
    LeaveServer,
    LeaveRoom,
    Resume,
    Quit,
    Color(u8),
    Wear(Outfit),
    RandomOutfit,
    Set(Toggle),
    Rebind(Bind),
    ResetKeys,
    Gfx(GfxPick),
    OpenLogs,
}

#[derive(Component, Clone, Debug)]
pub struct Act(pub Action);

#[derive(Message, Clone, Debug)]
pub struct UiAction(pub Action);

/// How a button looks.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum Look {
    Plain,
    Primary,
    Go,
    Danger,
    Chip(bool),
    Tab(bool),
    Tiny,
    TinyDanger,
    /// A check box with its label.
    Check(bool),
    /// The title of a folded part.
    Fold,
    /// A colour swatch (selected or not).
    Swatch(Color, bool),
}

impl Look {
    fn fill(self) -> (Color, Color) {
        match self {
            Look::Plain | Look::Tiny | Look::Chip(false) => (Color::srgba(1.0, 1.0, 1.0, 0.7), INK),
            Look::Tab(false) => (Color::NONE, INK),
            Look::Tab(true) => (Color::srgba(1.0, 1.0, 1.0, 0.92), INK),
            Look::Primary => (PINK, Color::WHITE),
            Look::Go => (GREEN, Color::WHITE),
            Look::Danger | Look::TinyDanger => (RED, Color::WHITE),
            Look::Chip(true) => (BLUE, Color::WHITE),
            Look::Check(_) => (Color::NONE, INK),
            Look::Fold => (Color::NONE, MUTED),
            Look::Swatch(c, _) => (c, INK),
        }
    }

    fn size(self) -> f32 {
        match self {
            Look::Tiny | Look::TinyDanger => 12.0,
            Look::Chip(_) | Look::Check(_) | Look::Fold => 13.5,
            _ => 16.0,
        }
    }

    fn padding(self) -> UiRect {
        match self {
            Look::Tiny | Look::TinyDanger => UiRect::axes(rem(0.5), px(2)),
            Look::Chip(_) => UiRect::axes(rem(0.6875), rem(0.3125)),
            Look::Tab(_) => UiRect::axes(rem(0.75), rem(0.375)),
            Look::Check(_) | Look::Fold => UiRect::axes(px(0), px(2)),
            Look::Swatch(..) => UiRect::ZERO,
            _ => UiRect::axes(rem(1.0), rem(0.5625)),
        }
    }

    fn border(self) -> UiRect {
        match self {
            Look::Swatch(_, on) => UiRect::all(px(if on { 3 } else { 2 })),
            _ => UiRect::all(px(1)),
        }
    }

    fn rim(self) -> BorderColor {
        BorderColor::all(match self {
            Look::Check(_) | Look::Fold | Look::Tab(_) => Color::NONE,
            Look::Swatch(_, true) => INK,
            Look::Swatch(_, false) => Color::WHITE,
            _ => RIM,
        })
    }

    fn shadow(self) -> Option<BoxShadow> {
        match self {
            Look::Check(_) | Look::Fold | Look::Tab(false) => None,
            Look::Swatch(..) => Some(BoxShadow::new(SHADOW.with_alpha(0.2), px(0), px(1), px(0), px(4))),
            _ => Some(BoxShadow::new(SHADOW.with_alpha(0.12), px(0), px(2), px(0), px(6))),
        }
    }
}

/// Replaces an entity's content (a list rebuilt when the resource it shows changes).
pub fn rebuild(commands: &mut Commands, e: Entity, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    commands.entity(e).despawn_children().with_children(f);
}

pub fn display(on: bool) -> bevy::ui::Display {
    if on {
        bevy::ui::Display::Flex
    } else {
        bevy::ui::Display::None
    }
}

/// Text fields, found by what they hold.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Server,
    Name,
    MenuName,
    RoomTitle,
    Pin,
    Chat,
}

/// Splits text into runs of the game's font and of the emoji font (Nunito has no emoji or symbols).
pub fn runs(s: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for c in s.chars() {
        if c == '\u{fe0f}' {
            continue;
        }
        let o = c as u32;
        let emoji = o >= 0x2190 && o != 0x2212 && o != 0x20ac && o != 0x2116 && o != 0x2122;
        match out.last_mut() {
            Some((run, e)) if *e == emoji => run.push(c),
            _ => out.push((c.to_string(), emoji)),
        }
    }
    out
}

/// What a text that may hold emoji says: set it, and its spans of the two fonts follow (`respan`).
#[derive(Component, Clone, PartialEq, Eq, Debug)]
pub struct Rich(pub String);

impl Rich {
    pub fn set(&mut self, s: &str) {
        if self.0 != s {
            s.clone_into(&mut self.0);
        }
    }
}

/// Text that may hold emoji, as spans of the two fonts.
pub fn rich(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, size: f32, color: Color) -> Entity {
    rich_in(p, f, s, size, color, false)
}

pub fn rich_in(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, size: f32, color: Color, black: bool) -> Entity {
    rich_with(p, f, s, size, color, black, None)
}

/// Text whose root and every run carry `pick`.
fn rich_with(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    s: &str,
    size: f32,
    color: Color,
    black: bool,
    pick: Option<Pickable>,
) -> Entity {
    let font = TextFont {
        font: if black { f.black.clone() } else { f.bold.clone() }.into(),
        font_size: FontSize::Px(size),
        ..default()
    };
    let mut e = p.spawn((Text::default(), font.clone(), TextColor(color), Rich(s.into())));
    if let Some(pick) = pick {
        e.insert(pick);
    }
    e.with_children(|t| spans(t, f, s, &font, TextColor(color), pick));
    e.id()
}

fn spans(t: &mut ChildSpawnerCommands, f: &Fonts, s: &str, font: &TextFont, color: TextColor, pick: Option<Pickable>) {
    for (run, emoji) in runs(s) {
        let font = if emoji {
            TextFont {
                font: f.emoji.clone().into(),
                ..font.clone()
            }
        } else {
            font.clone()
        };
        let mut span = t.spawn((TextSpan::new(run), font, color));
        if let Some(pick) = pick {
            span.insert(pick);
        }
    }
}

/// A `Rich` text set to something else gets its spans anew.
fn respan(
    texts: Query<(Entity, Ref<Rich>, &TextFont, &TextColor, Option<&Pickable>), Changed<Rich>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    for (e, rich, font, color, pick) in &texts {
        if rich.is_added() {
            continue;
        }
        commands
            .entity(e)
            .despawn_children()
            .with_children(|t| spans(t, &f, &rich.0, font, *color, pick.copied()));
    }
}

/// Sets the first `Rich` text inside `e` (a button's label).
pub fn relabel(e: Entity, s: &str, children: &Query<&Children>, texts: &mut Query<&mut Rich>) {
    if let Some(c) = children.iter_descendants(e).find(|c| texts.contains(*c))
        && let Ok(mut t) = texts.get_mut(c)
    {
        t.set(s);
    }
}

/// The text `t` (just spawned in `p`) breaks inside a word that does not fit on a line, a path or a URL,
/// instead of running out of its panel.
pub fn wrap_anywhere(p: &mut ChildSpawnerCommands, t: Entity) {
    p.commands()
        .entity(t)
        .insert(TextLayout::linebreak(bevy::text::LineBreak::WordOrCharacter));
}

pub fn label(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich(p, f, s, 15.0, INK)
}

pub fn muted(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich(p, f, s, 13.0, MUTED)
}

pub fn heading(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich_in(p, f, s, 15.0, INK, true)
}

pub fn logo(p: &mut ChildSpawnerCommands, f: &Fonts, size: f32) -> Entity {
    p.spawn((
        Text::new(text::LOGO),
        TextFont {
            font: f.black.clone().into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(LOGO_PINK),
        TextShadow {
            offset: Vec2::new(0.0, 2.5),
            color: Color::WHITE,
        },
    ))
    .id()
}

pub fn button(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, look: Look, act: Action) -> Entity {
    button_if(p, f, s, look, act, true)
}

pub fn button_if(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, look: Look, act: Action, enabled: bool) -> Entity {
    let (bg, ink) = look.fill();
    let radius = match look {
        Look::Tab(_) => rem(0.75),
        Look::Chip(_) | Look::Tiny | Look::TinyDanger => rem(0.6),
        _ => rem(0.875),
    };
    // (Labels of check boxes wrap; other buttons keep their size.)
    let shrink = if matches!(look, Look::Check(_)) { 1.0 } else { 0.0 };
    let face = (
        Node {
            flex_grow: 1.0,
            flex_shrink: shrink,
            padding: look.padding(),
            border: look.border(),
            border_radius: BorderRadius::all(radius),
            justify_content: if matches!(look, Look::Check(_) | Look::Fold) {
                JustifyContent::FlexStart
            } else {
                JustifyContent::Center
            },
            align_items: AlignItems::Center,
            column_gap: rem(0.4),
            ..default()
        },
        BackgroundColor(bg),
        look.rim(),
    );
    button_shell(p, look, act, enabled, shrink, face, |b| {
        if let Look::Check(on) = look {
            b.spawn((
                CheckBox,
                Node {
                    width: rem(1.1),
                    height: rem(1.1),
                    border: UiRect::all(px(2)),
                    border_radius: BorderRadius::all(rem(0.3)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    flex_shrink: 0.0,
                    ..default()
                },
                BorderColor::all(if on { BLUE } else { MUTED }),
                BackgroundColor(if on { BLUE } else { Color::WHITE }),
                Pickable::IGNORE,
            ))
            .with_children(|c| {
                c.spawn((
                    CheckDot,
                    Node {
                        width: rem(0.5),
                        height: rem(0.5),
                        border_radius: BorderRadius::all(px(2)),
                        ..default()
                    },
                    BackgroundColor(Color::WHITE),
                    if on { Visibility::Inherited } else { Visibility::Hidden },
                    Pickable::IGNORE,
                ));
            });
        }
        // (Picking hits text by its runs: a run without IGNORE would take the release from the button.)
        if !s.is_empty() {
            rich_with(b, f, s, look.size(), ink, false, Some(Pickable::IGNORE));
        }
    })
}

/// A check box's square, and the mark in it.
#[derive(Component)]
struct CheckBox;
#[derive(Component)]
struct CheckDot;

/// A colour swatch button.
pub fn swatch(p: &mut ChildSpawnerCommands, color: Color, on: bool, act: Action, enabled: bool, size: f32) -> Entity {
    let look = Look::Swatch(color, on);
    let face = (
        Node {
            width: rem(size),
            height: rem(size),
            border: look.border(),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(color),
        look.rim(),
    );
    button_shell(p, look, act, enabled, 0.0, face, |_| {})
}

/// What of a button moves when it is hovered or pressed. The button itself stays put: pressed at its edge,
/// it would otherwise slip from under the pointer, and the release would miss it.
#[derive(Component)]
struct Face(Entity);

fn button_shell(
    p: &mut ChildSpawnerCommands,
    look: Look,
    act: Action,
    enabled: bool,
    shrink: f32,
    face: impl Bundle,
    inside: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    let mut e = p.spawn((
        bevy::ui_widgets::Button,
        Hovered::default(),
        look,
        Act(act),
        Node {
            flex_shrink: shrink,
            ..default()
        },
    ));
    if !enabled {
        e.insert(InteractionDisabled);
    }
    let mut face_e = Entity::PLACEHOLDER;
    e.with_children(|b| {
        let mut fe = b.spawn((face, UiTransform::default(), Pickable::IGNORE));
        if let Some(shadow) = look.shadow() {
            fe.insert(shadow);
        }
        fe.with_children(inside);
        face_e = fe.id();
    });
    e.insert(Face(face_e));
    e.id()
}

/// Makes a button pressable or not.
pub fn enable(commands: &mut Commands, e: Entity, was: bool, on: bool) {
    if was != on {
        if on {
            commands.entity(e).remove::<InteractionDisabled>();
        } else {
            commands.entity(e).insert(InteractionDisabled);
        }
    }
}

/// A button whose `Look` was set anew is redrawn: its rim, shadow, ink and check box.
fn restyle(
    buttons: Query<(Ref<Look>, &Face), Changed<Look>>,
    mut faces: Query<(&mut Node, &mut BorderColor), Without<CheckBox>>,
    children: Query<&Children>,
    mut boxes: Query<(&mut BorderColor, &mut BackgroundColor), With<CheckBox>>,
    mut dots: Query<&mut Visibility, With<CheckDot>>,
    mut inks: Query<&mut TextColor>,
    mut commands: Commands,
) {
    for (look, face) in &buttons {
        if look.is_added() {
            continue;
        }
        let look = *look;
        if let Ok((mut node, mut rim)) = faces.get_mut(face.0) {
            if node.border != look.border() {
                node.border = look.border();
            }
            *rim = look.rim();
        }
        match look.shadow() {
            Some(s) => commands.entity(face.0).insert(s),
            None => commands.entity(face.0).remove::<BoxShadow>(),
        };
        let ink = look.fill().1;
        for c in children.iter_descendants(face.0) {
            if let Ok(mut t) = inks.get_mut(c) {
                t.set_if_neq(TextColor(ink));
            }
            if let Look::Check(on) = look {
                if let Ok((mut rim, mut fill)) = boxes.get_mut(c) {
                    *rim = BorderColor::all(if on { BLUE } else { MUTED });
                    fill.set_if_neq(BackgroundColor(if on { BLUE } else { Color::WHITE }));
                }
                if let Ok(mut v) = dots.get_mut(c) {
                    v.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
                }
            }
        }
    }
}

/// A text field (one line). `filter` keeps only the characters it allows.
pub fn field(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    which: Field,
    initial: &str,
    max: usize,
    filter: Option<fn(char) -> bool>,
) -> Entity {
    let mut e = p.spawn((
        which,
        Node {
            flex_grow: 1.0,
            min_width: rem(4.0),
            padding: UiRect::axes(rem(0.75), rem(0.5)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(0.75)),
            overflow: Overflow::clip(),
            ..default()
        },
        EditableText {
            max_characters: Some(max),
            allow_newlines: false,
            ..EditableText::new(initial)
        },
        TextLayout::no_wrap(),
        TextFont {
            font: f.bold.clone().into(),
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(INK),
        TextCursorStyle {
            color: INK,
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.92)),
        BorderColor::all(Color::srgba(0.169, 0.102, 0.361, 0.25)),
    ));
    if let Some(filter) = filter {
        e.insert(EditableTextFilter::new(filter));
    }
    e.id()
}

/// The placeholder shown over an empty field (`Placeholder(field)` next to it).
#[derive(Component)]
pub struct Placeholder(pub Entity);

pub fn field_with_hint(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    which: Field,
    initial: &str,
    hint: &str,
    max: usize,
    filter: Option<fn(char) -> bool>,
) -> Entity {
    let mut field_e = Entity::PLACEHOLDER;
    p.spawn(Node {
        flex_grow: 1.0,
        min_width: rem(4.0),
        ..default()
    })
    .with_children(|w| {
        field_e = field(w, f, which, initial, max, filter);
        w.spawn((
            Placeholder(field_e),
            Node {
                position_type: PositionType::Absolute,
                left: rem(0.8),
                top: rem(0.5),
                ..default()
            },
            Pickable::IGNORE,
            Text::new(hint),
            TextFont {
                font: f.bold.clone().into(),
                font_size: FontSize::Px(15.0),
                ..default()
            },
            TextColor(MUTED.with_alpha(0.6)),
        ));
    });
    field_e
}

/// Replaces a field's text; `EditableText::clear` alone leaves the cursor past the end of the empty text.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place that clears, with the cursor reset after it"
)]
pub fn set_field_text(t: &mut EditableText, s: &str) {
    t.clear();
    t.queue_edit(TextEdit::SelectAll);
    if !s.is_empty() {
        t.queue_edit(TextEdit::Insert(s.into()));
    }
}

pub fn field_text(fields: &Query<(&Field, &EditableText)>, which: Field) -> String {
    fields
        .iter()
        .find(|(f, _)| **f == which)
        .map(|(_, t)| t.value().to_string())
        .unwrap_or_default()
}

/// A slider of a setting, with its label and value.
pub fn slider(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    knob: Knob,
    label_s: &str,
    value: f32,
    range: (f32, f32),
    step: f32,
) {
    let range = SliderRange::new(range.0, range.1);
    let thumb_left = percent(range.thumb_position(value) * 100.0);
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: rem(0.3),
        ..default()
    })
    .with_children(|c| {
        row(c, false, |r| {
            label(r, f, &format!("{label_s}:"));
            r.spawn((
                KnobValue(knob),
                Text::new(knob_text(knob, value)),
                TextFont {
                    font: f.black.clone().into(),
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(INK),
            ));
        });
        c.spawn((
            knob,
            Slider::default(),
            SliderValue(value),
            range,
            SliderStep(step),
            Hovered::default(),
            Node {
                height: rem(1.1),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Stretch,
                flex_direction: FlexDirection::Column,
                ..default()
            },
        ))
        .with_children(|s| {
            s.spawn((
                Node {
                    height: rem(0.4),
                    border_radius: BorderRadius::all(rem(0.2)),
                    ..default()
                },
                BackgroundColor(INK.with_alpha(0.2)),
            ));
            s.spawn(Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: rem(1.1),
                top: px(0),
                bottom: px(0),
                ..default()
            })
            .with_children(|t| {
                t.spawn((
                    SliderThumb,
                    Node {
                        position_type: PositionType::Absolute,
                        width: rem(1.1),
                        height: rem(1.1),
                        left: thumb_left,
                        border: UiRect::all(px(2)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(BLUE),
                    BorderColor::all(Color::WHITE),
                ));
            });
        });
    });
}

/// The value shown beside a slider's label.
#[derive(Component)]
pub struct KnobValue(pub Knob);

pub fn knob_text(knob: Knob, v: f32) -> String {
    match knob {
        Knob::MouseSens | Knob::StickSens => format!("{v:.2}"),
        Knob::Fov => format!("{v:.0}°"),
        Knob::Volume | Knob::UiScale => format!("{:.0} %", v * 100.0),
    }
}

/// A box of related controls.
pub fn group(p: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: rem(0.625),
            padding: UiRect::all(rem(0.75)),
            border_radius: BorderRadius::all(rem(0.875)),
            ..default()
        },
        BackgroundColor(GROUP),
    ))
    .with_children(f)
    .id()
}

pub fn row(p: &mut ChildSpawnerCommands, wrap: bool, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: if wrap { FlexWrap::Wrap } else { FlexWrap::NoWrap },
        column_gap: rem(0.5),
        row_gap: rem(0.4),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(f)
    .id()
}

pub fn stack(p: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: rem(0.625),
        ..default()
    })
    .with_children(f)
    .id()
}

/// A colour dot (a player's suit).
pub fn dot(p: &mut ChildSpawnerCommands, color: Color, size: f32) -> Entity {
    p.spawn((
        Node {
            width: rem(size),
            height: rem(size),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::MAX,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(color),
        BorderColor::all(dot_rim(color)),
    ))
    .id()
}

/// A dot's rim: white, or a faint ink line round a colour too pale to show on the white panels (a white suit).
pub fn dot_rim(c: Color) -> Color {
    let rim = if c.luminance() > 0.8 {
        Color::srgba(0.169, 0.102, 0.361, 0.35)
    } else {
        Color::WHITE
    };
    rim.with_alpha(rim.alpha() * c.alpha())
}

/// A folded part: its title toggles it; the body is built folded or not and shown when open (`sync_folds`).
pub fn fold(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    folds: &Folds,
    key: Fold,
    title: &'static str,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) {
    let open = folds.open(key);
    stack(p, |c| {
        let b = button(c, f, &fold_title(title, open), Look::Fold, Action::Fold(key));
        c.commands().entity(b).insert(FoldTitle(title));
        c.spawn((
            FoldBody(key),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: rem(0.625),
                display: display(open),
                ..default()
            },
        ))
        .with_children(body);
    });
}

#[derive(Component)]
struct FoldTitle(&'static str);

#[derive(Component)]
struct FoldBody(Fold);

fn fold_title(title: &str, open: bool) -> String {
    format!("{} {title}", if open { "−" } else { "+" })
}

fn sync_folds(
    folds: Res<Folds>,
    mut bodies: Query<(&FoldBody, &mut Node)>,
    titles: Query<(Entity, &Act, &FoldTitle)>,
    children: Query<&Children>,
    mut texts: Query<&mut Rich>,
) {
    for (b, mut node) in &mut bodies {
        show(&mut node, folds.open(b.0));
    }
    for (e, act, title) in &titles {
        if let Action::Fold(k) = act.0 {
            relabel(e, &fold_title(title.0, folds.open(k)), &children, &mut texts);
        }
    }
}

/// A glass panel (`.glass`).
pub fn glass() -> (BackgroundColor, BorderColor, BoxShadow) {
    (
        BackgroundColor(PANEL),
        BorderColor::all(RIM),
        BoxShadow::new(SHADOW.with_alpha(0.18), px(0), rem(0.6), px(0), rem(1.8)),
    )
}

#[derive(Component)]
pub struct UiRoot;

/// Screens and layers under the root, shown or hidden by state.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Home,
    Banner,
    Hud,
    Menu,
    Chat,
    Tags,
}

/// Every layer's entity.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Layers([Entity; 6]);

impl core::ops::Index<Layer> for Layers {
    type Output = Entity;

    fn index(&self, l: Layer) -> &Entity {
        &self.0[l as usize]
    }
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        let mut fonts = app.world_mut().resource_mut::<Assets<Font>>();
        // Every text without a font of its own (the debug overlay too) gets the game's, with Cyrillic.
        let _ = fonts.insert(AssetId::default(), Font::from_bytes(BOLD.to_vec()));
        let f = Fonts {
            bold: Handle::default(),
            black: fonts.add(Font::from_bytes(BLACK.to_vec())),
            emoji: fonts.add(Font::from_bytes(EMOJI.to_vec())),
        };
        app.insert_resource(f);
        app.init_resource::<Ui>();
        app.init_resource::<Folds>();
        app.init_state::<Screen>();
        app.add_sub_state::<HomeTab>();
        app.add_sub_state::<MenuTab>();
        app.add_message::<UiAction>();
        app.add_observer(on_activate);
        app.add_observer(on_slide);
        app.add_systems(Startup, setup);
        // (Before the frame's state transitions: the screen changes in the frame after what picks it.)
        app.add_systems(PreUpdate, screen);
        app.add_systems(
            Update,
            (
                scale_ui,
                style_buttons,
                place_thumbs,
                knob_values,
                show_placeholders,
                last_device,
                (
                    ui_actions,
                    sync_folds.run_if(resource_changed::<Folds>),
                    sync_settings.run_if(
                        resource_changed::<Controls>
                            .or_else(resource_changed::<Display>)
                            .or_else(resource_changed::<Graphics>)
                            .or_else(resource_changed::<Bindings>)
                            .or_else(resource_changed::<Ui>)
                            .or_else(resource_exists_and_changed::<crate::render::quality::Quality>)
                            .or_else(resource_exists_and_changed::<crate::render::upscale::Upscaling>),
                    ),
                    sync_sliders.run_if(
                        resource_changed::<Controls>
                            .or_else(resource_changed::<Display>)
                            .or_else(resource_changed::<Sound>),
                    ),
                )
                    .chain(),
            ),
        );
        // (Restyled, then spanned anew with the new ink, before the layout; focus kept after the lists redrawn this
        // frame, before the next frame's keys.)
        app.add_systems(
            PostUpdate,
            ((restyle, respan).chain().before(UiSystems::Prepare), keep_focus),
        );
        app.add_plugins((
            home::HomePlugin,
            menu::MenuPlugin,
            hud::HudPlugin,
            chat::ChatPlugin,
            tags::TagsPlugin,
            loading::LoadingPlugin,
        ));
    }
}

fn setup(mut commands: Commands) {
    let mut layers = [Entity::PLACEHOLDER; 6];
    commands
        .spawn((
            UiRoot,
            Node {
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|r| {
            for (layer, z) in [
                (Layer::Tags, 0),
                (Layer::Hud, 1),
                (Layer::Chat, 2),
                (Layer::Menu, 3),
                (Layer::Home, 4),
                (Layer::Banner, 5),
            ] {
                layers[layer as usize] = r
                    .spawn((
                        layer,
                        Node {
                            position_type: PositionType::Absolute,
                            width: percent(100),
                            height: percent(100),
                            display: display(layer == Layer::Banner),
                            ..default()
                        },
                        ZIndex(z),
                        Pickable::IGNORE,
                    ))
                    .id();
            }
        });
    commands.insert_resource(Layers(layers));
}

/// Where the player is picks the screen.
fn screen(
    session: Res<Session>,
    list: Res<RoomList>,
    conn: Option<Res<crate::net::Conn>>,
    target: Res<crate::servers::Target>,
    now: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
) {
    let online = conn.is_some_and(|c| c.connected);
    let s = if target.0.is_none() {
        Screen::Servers
    } else if session.room.is_some() && session.arena.is_some() {
        Screen::Room
    } else if online && session.room.is_none() && list.rooms.is_some() && !session.refused {
        Screen::Rooms
    } else {
        Screen::Away
    };
    if *now.get() != s {
        next.set(s);
    }
}

/// Shows a layer or hides it.
pub fn show(node: &mut Mut<Node>, on: bool) {
    let d = display(on);
    if node.display != d {
        node.display = d;
    }
}

/// The interface follows the window: one rem is 12–20 px, growing with its width and height, times the
/// player's own scale.
fn scale_ui(windows: Query<&Window, With<PrimaryWindow>>, display: Res<Display>, mut scale: ResMut<UiScale>) {
    let (w, h) = windows.single().map_or((1600.0, 900.0), |w| (w.width(), w.height()));
    let root = (0.0042 * w + 0.0055 * h + 4.5).clamp(12.0, 20.0);
    let s = root / 16.0 * display.ui_scale.clamp(0.75, 1.5);
    if (scale.0 - s).abs() > 1e-3 {
        scale.0 = s;
    }
}

fn on_activate(ev: On<Activate>, acts: Query<&Act>, mut out: MessageWriter<UiAction>) {
    if let Ok(a) = acts.get(ev.entity) {
        out.write(UiAction(a.0.clone()));
    }
}

fn on_slide(
    ev: On<ValueChange<f32>>,
    knobs: Query<&Knob>,
    mut commands: Commands,
    mut controls: ResMut<crate::settings::Controls>,
    mut sound: ResMut<crate::settings::Sound>,
    mut display: ResMut<Display>,
) {
    let Ok(knob) = knobs.get(ev.source) else { return };
    let v = ev.value;
    match knob {
        Knob::MouseSens => controls.mouse_sensitivity = v,
        Knob::StickSens => controls.stick_sensitivity = v,
        Knob::Fov => display.fov = v,
        Knob::Volume => sound.volume = v,
        Knob::UiScale => {
            // (Applied when let go: the slider would run away under the pointer.)
            if !ev.is_final {
                commands.entity(ev.source).insert(SliderValue(v));
                return;
            }
            display.ui_scale = v;
        }
    }
    commands.entity(ev.source).insert(SliderValue(v));
    crate::settings::save_soon(&mut commands);
}

fn place_thumbs(
    sliders: Query<(Entity, &SliderValue, &SliderRange), Changed<SliderValue>>,
    children: Query<&Children>,
    mut thumbs: Query<&mut Node, With<SliderThumb>>,
) {
    for (e, v, range) in &sliders {
        for c in children.iter_descendants(e) {
            if let Ok(mut n) = thumbs.get_mut(c) {
                n.left = percent(range.thumb_position(v.0) * 100.0);
            }
        }
    }
}

fn knob_values(
    sliders: Query<(&Knob, &SliderValue), Changed<SliderValue>>,
    mut values: Query<(&KnobValue, &mut Text)>,
) {
    for (knob, v) in &sliders {
        for (k, mut t) in &mut values {
            if k.0 == *knob {
                let s = knob_text(*knob, v.0);
                if t.0 != s {
                    t.0 = s;
                }
            }
        }
    }
}

fn style_buttons(
    q: Query<(&Look, &Hovered, Has<Pressed>, Has<InteractionDisabled>, &Face), With<bevy::ui_widgets::Button>>,
    mut faces: Query<(&mut BackgroundColor, &mut UiTransform)>,
) {
    for (look, hovered, pressed, disabled, face) in &q {
        let Ok((mut bg, mut tf)) = faces.get_mut(face.0) else {
            continue;
        };
        let (base, _) = look.fill();
        let c = if disabled {
            base.with_alpha(base.alpha() * 0.45)
        } else if matches!(look, Look::Check(_) | Look::Fold | Look::Tab(false)) {
            if hovered.get() {
                INK.with_alpha(0.06)
            } else {
                Color::NONE
            }
        } else if hovered.get() {
            base.lighter(0.06).with_alpha((base.alpha() + 0.15).min(1.0))
        } else {
            base
        };
        bg.set_if_neq(BackgroundColor(c));
        let y = if disabled || matches!(look, Look::Check(_) | Look::Fold) {
            0.0
        } else if pressed {
            1.0
        } else if hovered.get() {
            -1.0
        } else {
            0.0
        };
        let next = Val2::px(0.0, y);
        if tf.translation != next {
            tf.translation = next;
        }
    }
}

fn show_placeholders(
    hints: Query<(&Placeholder, &mut Visibility)>,
    fields: Query<&EditableText, Changed<EditableText>>,
) {
    for (p, mut vis) in hints {
        if let Ok(t) = fields.get(p.0) {
            let empty = t.value().chars().next().is_none();
            vis.set_if_neq(if empty {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }
}

/// Keyboard or mouse against a gamepad: the hints show the controls of the one used last.
fn last_device(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pads: Query<&Gamepad>,
    mut ui: ResMut<Ui>,
) {
    let pad = pads
        .iter()
        .any(|p| p.get_just_pressed().next().is_some() || p.left_stick().length() > 0.5);
    let desk = keys.get_just_pressed().next().is_some() || mouse.get_just_pressed().next().is_some();
    if pad && !ui.pad {
        ui.pad = true;
    } else if desk && ui.pad {
        ui.pad = false;
    }
}

/// Actions every screen shares: tabs, folds, settings toggles, quitting.
fn ui_actions(
    mut actions: MessageReader<UiAction>,
    mut ui: ResMut<Ui>,
    mut folds: ResMut<Folds>,
    mut home_tab: ResMut<NextState<HomeTab>>,
    mut menu_tab: ResMut<NextState<MenuTab>>,
    mut controls: ResMut<Controls>,
    mut display: ResMut<Display>,
    mut binds: ResMut<Bindings>,
    mut gfx: ResMut<Graphics>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    for UiAction(a) in actions.read() {
        match a {
            Action::HomeTab(t) => home_tab.set(*t),
            Action::MenuTab(t) => menu_tab.set(*t),
            Action::Fold(k) => folds.toggle(*k),
            Action::Set(t) => {
                match t {
                    Toggle::InvertMouse => controls.invert_mouse_y ^= true,
                    Toggle::InvertStick => controls.invert_stick_y ^= true,
                    Toggle::Shake => controls.camera_shake ^= true,
                    Toggle::ShowFps => display.show_fps ^= true,
                    Toggle::Fullscreen => display.fullscreen ^= true,
                    Toggle::Vsync => gfx.vsync ^= true,
                }
                crate::settings::save_soon(&mut commands);
            }
            Action::Quit => {
                exit.write(AppExit::Success);
            }
            Action::Gfx(pick) => {
                match *pick {
                    GfxPick::Preset(p) => gfx.preset = p,
                    GfxPick::Fps(n) => gfx.fps_limit = n,
                    GfxPick::Backend(b) => gfx.backend = crate::backend::BackendSetting(b),
                    GfxPick::Upscaler(u) => gfx.upscaler = crate::render::upscale::UpscalerSetting(u),
                    GfxPick::Upscale(m) => gfx.upscale = m,
                }
                crate::settings::save_soon(&mut commands);
            }
            Action::Rebind(b) => ui.rebinding = Some(*b),
            Action::ResetKeys => {
                *binds = Bindings::default();
                ui.rebinding = None;
                crate::settings::save_soon(&mut commands);
            }
            _ => {}
        }
    }
}

/// The options kept on this machine.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Options<'w> {
    pub controls: Res<'w, Controls>,
    pub sound: Res<'w, Sound>,
    pub display: Res<'w, Display>,
    pub binds: Res<'w, Bindings>,
    pub gfx: Res<'w, Graphics>,
    pub quality: Option<Res<'w, crate::render::quality::Quality>>,
    pub upscaling: Option<Res<'w, crate::render::upscale::Upscaling>>,
}

impl Options<'_> {
    fn toggle(&self, t: Toggle) -> bool {
        match t {
            Toggle::InvertMouse => self.controls.invert_mouse_y,
            Toggle::InvertStick => self.controls.invert_stick_y,
            Toggle::Shake => self.controls.camera_shake,
            Toggle::ShowFps => self.display.show_fps,
            Toggle::Fullscreen => self.display.fullscreen,
            Toggle::Vsync => self.gfx.vsync,
        }
    }

    fn knob(&self, k: Knob) -> f32 {
        match k {
            Knob::MouseSens => self.controls.mouse_sensitivity,
            Knob::StickSens => self.controls.stick_sensitivity,
            Knob::Fov => self.display.fov,
            Knob::Volume => self.sound.volume,
            Knob::UiScale => self.display.ui_scale,
        }
    }

    fn upscaling(&self) -> crate::render::upscale::Upscaling {
        self.upscaling.as_deref().copied().unwrap_or_default()
    }

    /// A graphics choice is the one made, and whether this machine can make it. (Only what this system runs: a
    /// saved backend or upscaler it cannot is "auto".)
    fn picked(&self, pick: GfxPick) -> (bool, bool) {
        let g = &*self.gfx;
        let offer = self.upscaling().offer;
        let upscaler = g.upscaler.0.filter(|u| offer.has(*u));
        match pick {
            GfxPick::Preset(p) => (g.preset == p, true),
            GfxPick::Fps(n) => (g.fps_limit == n, true),
            GfxPick::Backend(b) => (g.backend.runnable() == b, true),
            GfxPick::Upscaler(None) => (upscaler.is_none(), true),
            GfxPick::Upscaler(Some(u)) => (upscaler == Some(u), offer.has(u)),
            GfxPick::Upscale(m) => (g.upscale == m, true),
        }
    }

    fn line(&self, l: GfxLine) -> Option<String> {
        let up = self.upscaling();
        match l {
            GfxLine::Adapter => self
                .quality
                .as_ref()
                .map(|q| text::adapter(&q.adapter, &format!("{:?}", q.tier))),
            GfxLine::Upscaler => {
                let failed = (up.active != up.chosen).then_some(up.chosen.name());
                Some(text::upscaler_now(up.active.name(), failed))
            }
            GfxLine::Note => {
                (!Upscaler::ALL.into_iter().all(|u| up.offer.has(u))).then(|| text::UPSCALER_NOTE.to_string())
            }
        }
    }
}

/// Lines of the graphics part that follow what the machine has.
#[derive(Component, Clone, Copy)]
enum GfxLine {
    Adapter,
    Upscaler,
    Note,
}

/// The keys of an action, and the line asking for one while it is rebound.
#[derive(Component)]
struct BindKeys(Bind);
#[derive(Component)]
struct BindWait(Bind);

/// The options tab, at the room list and in the menu: built once, its controls follow the options
/// (`sync_settings`, `sync_sliders`).
pub fn settings_tab(p: &mut ChildSpawnerCommands, f: &Fonts, o: &Options, folds: &Folds) {
    let check = |c: &mut ChildSpawnerCommands, s: &str, t: Toggle| {
        button(c, f, s, Look::Check(o.toggle(t)), Action::Set(t));
    };
    let knob = |c: &mut ChildSpawnerCommands, k: Knob, s: &str, range: (f32, f32), step: f32| {
        slider(c, f, k, s, o.knob(k), range, step);
    };
    stack(p, |c| {
        knob(c, Knob::MouseSens, text::MOUSE_SENS, crate::settings::SENS_RANGE, 0.05);
        check(c, text::INVERT_MOUSE, Toggle::InvertMouse);
        knob(c, Knob::StickSens, text::STICK_SENS, crate::settings::SENS_RANGE, 0.05);
        check(c, text::INVERT_STICK, Toggle::InvertStick);
        check(c, text::SHAKE, Toggle::Shake);
        knob(c, Knob::Fov, text::FOV, crate::settings::FOV_RANGE, 1.0);
        knob(c, Knob::Volume, text::VOLUME, (0.0, 1.0), 0.05);
        knob(c, Knob::UiScale, text::UI_SCALE, crate::settings::UI_SCALE_RANGE, 0.05);
        check(c, text::SHOW_FPS, Toggle::ShowFps);
        check(c, text::FULLSCREEN, Toggle::Fullscreen);
        fold(c, f, folds, Fold::Gfx, text::GRAPHICS, |k| graphics(k, f, o));
        fold(c, f, folds, Fold::Keys, text::KEYS, |k| {
            for b in crate::keys::BINDS {
                row(k, false, |r| {
                    r.spawn(Node {
                        width: rem(6.0),
                        flex_shrink: 0.0,
                        ..default()
                    })
                    .with_children(|n| {
                        label(n, f, text::bind(b));
                    });
                    r.spawn(Node {
                        flex_grow: 1.0,
                        min_width: px(0),
                        ..default()
                    })
                    .with_children(|n| {
                        let keys = heading(n, f, &crate::keys::labels(o.binds.keys(b)));
                        n.commands().entity(keys).insert(BindKeys(b));
                        let wait = rich(n, f, text::PRESS_KEY, 14.0, PINK);
                        n.commands().entity(wait).insert((
                            BindWait(b),
                            Node {
                                display: display(false),
                                ..default()
                            },
                        ));
                    });
                    button(r, f, text::CHANGE, Look::Tiny, Action::Rebind(b));
                });
            }
            muted(k, f, text::KEYS_NOTE);
            button(k, f, text::RESET_KEYS, Look::Tiny, Action::ResetKeys);
        });
        fold(c, f, folds, Fold::Problems, text::PROBLEMS, |k| {
            muted(k, f, text::PROBLEMS_NOTE);
            button(k, f, text::OPEN_LOGS, Look::Tiny, Action::OpenLogs);
        });
    });
}

fn graphics(p: &mut ChildSpawnerCommands, f: &Fonts, o: &Options) {
    use crate::opts::Backend;
    use crate::render::quality::{Preset, Upscale};
    let chips = |p: &mut ChildSpawnerCommands, title: &str, items: &mut dyn Iterator<Item = (&str, GfxPick)>| {
        stack(p, |c| {
            muted(c, f, title);
            row(c, true, |r| {
                for (s, pick) in items {
                    let (on, ok) = o.picked(pick);
                    button_if(r, f, s, Look::Chip(on), Action::Gfx(pick), ok);
                }
            });
        });
    };
    let line = |p: &mut ChildSpawnerCommands, l: GfxLine| {
        let s = o.line(l);
        let e = muted(p, f, s.as_deref().unwrap_or_default());
        p.commands().entity(e).insert((
            l,
            Node {
                display: display(s.is_some()),
                ..default()
            },
        ));
    };
    let presets = [Preset::Low, Preset::High].map(|p| (text::preset(p), GfxPick::Preset(p)));
    chips(p, text::PRESET, &mut presets.into_iter());
    line(p, GfxLine::Adapter);
    let upscalers = core::iter::once((text::UPSCALER_AUTO, GfxPick::Upscaler(None)))
        .chain(Upscaler::ALL.map(|u| (text::upscaler(u), GfxPick::Upscaler(Some(u)))));
    chips(p, text::UPSCALER, &mut upscalers.into_iter());
    let modes = Upscale::PICKS.map(|m| (text::upscale(m), GfxPick::Upscale(m)));
    chips(p, text::UPSCALE, &mut modes.into_iter());
    line(p, GfxLine::Upscaler);
    line(p, GfxLine::Note);
    button(
        p,
        f,
        text::VSYNC,
        Look::Check(o.toggle(Toggle::Vsync)),
        Action::Set(Toggle::Vsync),
    );
    let fps =
        [(0, text::NO_LIMIT), (30, "30"), (60, "60"), (120, "120"), (144, "144")].map(|(n, s)| (s, GfxPick::Fps(n)));
    chips(p, text::FPS_LIMIT, &mut fps.into_iter());
    let backends = [
        (text::BACKEND_AUTO, None),
        ("DirectX 12", Some(Backend::Dx12)),
        ("Vulkan", Some(Backend::Vulkan)),
    ]
    .into_iter()
    .filter(|(_, b)| b.is_none_or(Backend::available))
    .map(|(s, b)| (s, GfxPick::Backend(b)));
    chips(p, text::BACKEND, &mut backends.into_iter());
}

/// The options tabs follow the options: check boxes, choices, the machine's lines and the keys.
fn sync_settings(
    o: Options,
    ui: Res<Ui>,
    mut buttons: Query<(Entity, &Act, &mut Look, Has<InteractionDisabled>)>,
    mut lines: Query<(&GfxLine, &mut Rich, &mut Node)>,
    mut keys: Query<(&BindKeys, &mut Rich), Without<GfxLine>>,
    mut waits: Query<(&BindWait, &mut Node), Without<GfxLine>>,
    mut commands: Commands,
) {
    for (e, act, mut look, disabled) in &mut buttons {
        match act.0 {
            Action::Set(t) => {
                look.set_if_neq(Look::Check(o.toggle(t)));
            }
            Action::Gfx(pick) => {
                let (on, ok) = o.picked(pick);
                look.set_if_neq(Look::Chip(on));
                enable(&mut commands, e, !disabled, ok);
            }
            _ => {}
        }
    }
    for (l, mut t, mut node) in &mut lines {
        let s = o.line(*l);
        show(&mut node, s.is_some());
        t.set(s.as_deref().unwrap_or_default());
    }
    for (k, mut t) in &mut keys {
        t.set(&crate::keys::labels(o.binds.keys(k.0)));
    }
    for (w, mut node) in &mut waits {
        show(&mut node, ui.rebinding == Some(w.0));
    }
}

/// Sliders follow their options (set at the other tab, or anew).
fn sync_sliders(o: Options, sliders: Query<(Entity, &Knob, &SliderValue)>, mut commands: Commands) {
    for (e, k, v) in &sliders {
        let want = o.knob(*k);
        if v.0 != want {
            commands.entity(e).insert(SliderValue(want));
        }
    }
}

/// Rebinding: the next key pressed becomes the action's (Esc lets it be). It ends when the keys are no
/// longer on screen (the menu closed as a round started, another tab): a key pressed in play is not a pick.
pub fn rebind(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui: ResMut<Ui>,
    folds: Res<Folds>,
    home: Option<Res<State<HomeTab>>>,
    menu: Option<Res<State<MenuTab>>>,
    mut binds: ResMut<Bindings>,
    mut commands: Commands,
) {
    let Some(b) = ui.rebinding else { return };
    let tab = home.is_some_and(|t| *t.get() == HomeTab::Settings)
        || (ui.menu && menu.is_some_and(|t| *t.get() == MenuTab::Settings));
    if !(tab && folds.open(Fold::Keys)) {
        ui.rebinding = None;
        return;
    }
    for k in keys.get_just_pressed() {
        if *k == KeyCode::Escape {
            ui.rebinding = None;
            return;
        }
        if crate::keys::bindable(*k) {
            binds.bind(b, *k);
            ui.rebinding = None;
            crate::settings::save_soon(&mut commands);
            return;
        }
    }
}

/// A field redrawn while it had the keyboard (a wrong address, a wrong PIN) keeps it in its new self; a hidden
/// one (the menu closed by a round's start, the room list left with Enter) gives it back to the game.
fn keep_focus(
    mut focus: ResMut<InputFocus>,
    fields: Query<(Entity, &Field)>,
    nodes: Query<(&Node, Option<&ChildOf>)>,
    mut had: Local<Option<(Entity, Field)>>,
) {
    if let Some((old, kind)) = *had
        && focus.get().is_none_or(|e| e == old)
        && !fields.contains(old)
        && let Some((e, _)) = fields.iter().find(|(e, f)| **f == kind && !hidden(*e, &nodes))
    {
        focus.set(e, bevy::input_focus::FocusCause::Navigated);
    }
    if focus.get().is_some_and(|e| fields.contains(e) && hidden(e, &nodes)) {
        focus.clear();
    }
    *had = focus.get().and_then(|e| fields.get(e).ok()).map(|(e, f)| (e, *f));
}

/// The node or one of its ancestors is not displayed.
fn hidden(mut e: Entity, nodes: &Query<(&Node, Option<&ChildOf>)>) -> bool {
    while let Ok((node, parent)) = nodes.get(e) {
        if node.display == bevy::ui::Display::None {
            return true;
        }
        let Some(p) = parent else { break };
        e = p.parent();
    }
    false
}

/// The keyboard belongs to a text field.
pub fn typing(focus: &InputFocus, fields: &Query<(), With<EditableText>>) -> bool {
    focus.get().is_some_and(|e| fields.contains(e))
}
