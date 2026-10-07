//! The player's interface on bevy_ui: the room list, the Esc menu, the HUD, the chat
//! and the name tags. A panel is rebuilt whole when what it shows changes (`Section`); text fields live
//! outside the rebuilt parts, so typing survives. Every button carries an `Action`; one observer turns
//! presses into `UiAction` messages for the module that owns them.
pub mod text;

mod chat;
mod home;
mod hud;
mod menu;
mod tags;

use core::hash::{Hash, Hasher};
use std::collections::BTreeSet;

use bevy::asset::AssetId;
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::input_focus::InputFocus;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle, TextEdit};
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::{Activate, Slider, SliderRange, SliderStep, SliderThumb, SliderValue, ValueChange};
use bevy::window::PrimaryWindow;
use fb_proto::{ClientMsg, Outfit};

use crate::settings::Display;

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

pub fn hex(c: &str) -> Color {
    crate::view::hex(c)
}

/// A suit colour (index into `COLORS`) as a swatch; the rainbow shows its middle.
pub fn suit(i: u8) -> Color {
    match fb_shared::COLORS.get(i as usize) {
        Some(&fb_shared::RAINBOW) | None => Color::srgb(1.0, 0.69, 0.25),
        Some(c) => hex(c),
    }
}

#[derive(Resource, Clone)]
pub struct Fonts {
    pub bold: Handle<Font>,
    pub black: Handle<Font>,
    pub emoji: Handle<Font>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum HomeTab {
    #[default]
    Rooms,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MenuTab {
    #[default]
    Game,
    Settings,
    Dev,
}

/// What the interface is doing.
#[derive(Resource, Default)]
pub struct Ui {
    /// The Esc menu is open: the mouse is free and the game takes no input.
    pub menu: bool,
    pub menu_tab: MenuTab,
    pub home_tab: HomeTab,
    /// The chat line takes the keyboard.
    pub chat: bool,
    /// In play with the mouse free (focus lost): the field asks for a click.
    pub need_click: bool,
    /// F3: the network and performance overlay.
    pub debug: bool,
    /// Folded parts that are open.
    pub open: BTreeSet<&'static str>,
    /// The create-room form's "private" box.
    pub create_private: bool,
    /// The address typed to add a server is not one.
    pub server_bad: bool,
    /// The last device used was a gamepad (the hints follow it).
    pub pad: bool,
    /// Waiting for the key to bind to this action.
    pub rebinding: Option<crate::keys::Bind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    InvertMouse,
    InvertStick,
    Shake,
    ShowFps,
    Shadows,
    Ao,
    Aa,
    Grade,
    Motes,
    Vsync,
}

/// A graphics choice of several.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GfxPick {
    Preset(&'static str),
    Upscale(&'static str),
    Fps(u32),
    Backend(&'static str),
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
    Fold(&'static str),
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
    Rebind(crate::keys::Bind),
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
}

/// A part of a screen rebuilt when what it shows changes: `key` is a hash of that.
#[derive(Component, Default)]
pub struct Section {
    key: Option<u64>,
}

pub fn key_of(v: &impl core::fmt::Debug) -> u64 {
    // (Sections ask every frame: the Debug text goes straight into the hasher, without a String.)
    struct Feed<'a>(&'a mut std::hash::DefaultHasher);
    impl core::fmt::Write for Feed<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            self.0.write(s.as_bytes());
            Ok(())
        }
    }
    let mut h = std::hash::DefaultHasher::new();
    let _ = core::fmt::Write::write_fmt(&mut Feed(&mut h), format_args!("{v:?}"));
    h.finish()
}

impl Section {
    /// True (and remembers it) when the section must be rebuilt for `key`.
    pub fn stale(&mut self, key: u64) -> bool {
        if self.key == Some(key) {
            return false;
        }
        self.key = Some(key);
        true
    }
}

/// Replaces a section's content.
pub fn rebuild(commands: &mut Commands, e: Entity, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    commands.entity(e).despawn_children().with_children(f);
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

/// Text that may hold emoji, as spans of the two fonts.
pub fn rich(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, size: f32, color: Color) -> Entity {
    rich_in(p, f, s, size, color, false)
}

pub fn rich_in(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, size: f32, color: Color, black: bool) -> Entity {
    rich_with(p, f, s, size, color, black, ())
}

/// Text whose root and every run carry `extra`.
fn rich_with(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    s: &str,
    size: f32,
    color: Color,
    black: bool,
    extra: impl Bundle + Clone,
) -> Entity {
    let font = if black { f.black.clone() } else { f.bold.clone() };
    let mut e = p.spawn((
        Text::default(),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        extra.clone(),
    ));
    let id = e.id();
    e.with_children(|t| {
        for (run, emoji) in runs(s) {
            t.spawn((
                TextSpan::new(run),
                TextFont {
                    font: if emoji { f.emoji.clone() } else { font.clone() }.into(),
                    font_size: FontSize::Px(size),
                    ..default()
                },
                TextColor(color),
                extra.clone(),
            ));
        }
    });
    id
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
            border: UiRect::all(px(1)),
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
        BorderColor::all(if matches!(look, Look::Check(_) | Look::Fold | Look::Tab(_)) {
            Color::NONE
        } else {
            RIM
        }),
    );
    let shadow = (!matches!(look, Look::Check(_) | Look::Fold | Look::Tab(false)))
        .then(|| BoxShadow::new(SHADOW.with_alpha(0.12), px(0), px(2), px(0), px(6)));
    button_shell(p, look, act, enabled, shrink, face, shadow, |b| {
        if let Look::Check(on) = look {
            b.spawn((
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
                if on {
                    c.spawn((
                        Node {
                            width: rem(0.5),
                            height: rem(0.5),
                            border_radius: BorderRadius::all(px(2)),
                            ..default()
                        },
                        BackgroundColor(Color::WHITE),
                        Pickable::IGNORE,
                    ));
                }
            });
        }
        // (Picking hits text by its runs: a run without IGNORE would take the release from the button.)
        if !s.is_empty() {
            rich_with(b, f, s, look.size(), ink, false, Pickable::IGNORE);
        }
    })
}

/// A colour swatch button.
pub fn swatch(p: &mut ChildSpawnerCommands, color: Color, on: bool, act: Action, enabled: bool, size: f32) -> Entity {
    let face = (
        Node {
            width: rem(size),
            height: rem(size),
            border: UiRect::all(px(if on { 3 } else { 2 })),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(color),
        BorderColor::all(if on { INK } else { Color::WHITE }),
    );
    let shadow = Some(BoxShadow::new(SHADOW.with_alpha(0.2), px(0), px(1), px(0), px(4)));
    button_shell(p, Look::Swatch(color, on), act, enabled, 0.0, face, shadow, |_| {})
}

/// What of a button moves when it is hovered or pressed. The button itself stays put: pressed at its edge,
/// it would otherwise slip from under the pointer, and the release would miss it.
#[derive(Component)]
struct Face(Entity);

#[expect(clippy::too_many_arguments)]
fn button_shell(
    p: &mut ChildSpawnerCommands,
    look: Look,
    act: Action,
    enabled: bool,
    shrink: f32,
    face: impl Bundle,
    shadow: Option<BoxShadow>,
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
        if let Some(shadow) = shadow {
            fe.insert(shadow);
        }
        fe.with_children(inside);
        face_e = fe.id();
    });
    e.insert(Face(face_e));
    e.id()
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

/// A folded part: its title toggles it.
pub fn fold(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    ui: &Ui,
    key: &'static str,
    title: &str,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) {
    let open = ui.open.contains(key);
    let mark = if open { "− " } else { "+ " };
    stack(p, |c| {
        button(c, f, &format!("{mark}{title}"), Look::Fold, Action::Fold(key));
        if open {
            stack(c, body);
        }
    });
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
        app.add_message::<UiAction>();
        app.add_observer(on_activate);
        app.add_observer(on_slide);
        app.add_systems(Startup, setup);
        app.add_systems(
            Update,
            (
                scale_ui,
                style_buttons,
                place_thumbs,
                knob_values,
                show_placeholders,
                last_device,
                ui_actions,
            ),
        );
        // (After the sections redrawn this frame, before the next frame's keys.)
        app.add_systems(PostUpdate, keep_focus);
        app.add_plugins((
            home::HomePlugin,
            menu::MenuPlugin,
            hud::HudPlugin,
            chat::ChatPlugin,
            tags::TagsPlugin,
        ));
    }
}

fn setup(mut commands: Commands) {
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
                r.spawn((
                    layer,
                    Node {
                        position_type: PositionType::Absolute,
                        width: percent(100),
                        height: percent(100),
                        display: bevy::ui::Display::None,
                        ..default()
                    },
                    ZIndex(z),
                    Pickable::IGNORE,
                ));
            }
        });
}

/// Shows a layer or hides it.
pub fn show(node: &mut Node, on: bool) {
    let d = if on {
        bevy::ui::Display::Flex
    } else {
        bevy::ui::Display::None
    };
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
            let empty = t.value().to_string().is_empty();
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
    mut controls: ResMut<crate::settings::Controls>,
    mut display: ResMut<Display>,
    mut binds: ResMut<crate::settings::Bindings>,
    mut gfx: ResMut<crate::settings::Graphics>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    for UiAction(a) in actions.read() {
        match a {
            Action::HomeTab(t) => ui.home_tab = *t,
            Action::MenuTab(t) => ui.menu_tab = *t,
            Action::Fold(k) => {
                if !ui.open.remove(k) {
                    ui.open.insert(k);
                }
            }
            Action::Set(t) => {
                match t {
                    Toggle::InvertMouse => controls.invert_mouse_y ^= true,
                    Toggle::InvertStick => controls.invert_stick_y ^= true,
                    Toggle::Shake => controls.camera_shake ^= true,
                    Toggle::ShowFps => display.show_fps ^= true,
                    Toggle::Shadows => gfx.shadows ^= true,
                    Toggle::Ao => gfx.ao ^= true,
                    Toggle::Aa => gfx.aa ^= true,
                    Toggle::Grade => gfx.grade ^= true,
                    Toggle::Motes => gfx.motes ^= true,
                    Toggle::Vsync => gfx.vsync ^= true,
                }
                crate::settings::save_soon(&mut commands);
            }
            Action::Quit => {
                exit.write(AppExit::Success);
            }
            Action::Gfx(pick) => {
                match *pick {
                    GfxPick::Preset(p) => gfx.preset = p.into(),
                    GfxPick::Upscale(u) => gfx.upscale = u.into(),
                    GfxPick::Fps(n) => gfx.fps_limit = n,
                    GfxPick::Backend(b) => gfx.backend = b.into(),
                }
                crate::settings::save_soon(&mut commands);
            }
            Action::Rebind(b) => ui.rebinding = Some(*b),
            Action::ResetKeys => {
                *binds = crate::settings::Bindings::default();
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
    pub controls: Res<'w, crate::settings::Controls>,
    pub sound: Res<'w, crate::settings::Sound>,
    pub display: Res<'w, Display>,
    pub binds: Res<'w, crate::settings::Bindings>,
    pub gfx: Res<'w, crate::settings::Graphics>,
    pub quality: Option<Res<'w, crate::render::quality::Quality>>,
}

impl Options<'_> {
    /// What the settings tab must be rebuilt for (not the sliders: they move themselves).
    pub fn key(&self, ui: &Ui) -> u64 {
        let c = &*self.controls;
        key_of(&(
            c.invert_mouse_y,
            c.invert_stick_y,
            c.camera_shake,
            self.display.show_fps,
            *self.binds == crate::settings::Bindings::default(),
            (
                &self.binds.forward,
                &self.binds.back,
                &self.binds.left,
                &self.binds.right,
            ),
            (&self.binds.jump, &self.binds.dive, &self.binds.grab),
            ui.rebinding,
            ui.open.contains("keys"),
            &*self.gfx,
            ui.open.contains("gfx"),
            self.quality.as_ref().map(|q| (q.tier, q.dropped)),
        ))
    }
}

/// The options tab, at the room list and in the menu.
pub fn settings_tab(p: &mut ChildSpawnerCommands, f: &Fonts, o: &Options, ui: &Ui) {
    let (controls, sound, display) = (&*o.controls, &*o.sound, &*o.display);
    stack(p, |c| {
        slider(
            c,
            f,
            Knob::MouseSens,
            text::MOUSE_SENS,
            controls.mouse_sensitivity,
            (0.2, 3.0),
            0.05,
        );
        button(
            c,
            f,
            text::INVERT_MOUSE,
            Look::Check(controls.invert_mouse_y),
            Action::Set(Toggle::InvertMouse),
        );
        slider(
            c,
            f,
            Knob::StickSens,
            text::STICK_SENS,
            controls.stick_sensitivity,
            (0.2, 3.0),
            0.05,
        );
        button(
            c,
            f,
            text::INVERT_STICK,
            Look::Check(controls.invert_stick_y),
            Action::Set(Toggle::InvertStick),
        );
        button(
            c,
            f,
            text::SHAKE,
            Look::Check(controls.camera_shake),
            Action::Set(Toggle::Shake),
        );
        slider(c, f, Knob::Fov, text::FOV, display.fov, (55.0, 100.0), 1.0);
        slider(c, f, Knob::Volume, text::VOLUME, sound.volume, (0.0, 1.0), 0.05);
        slider(c, f, Knob::UiScale, text::UI_SCALE, display.ui_scale, (0.75, 1.5), 0.05);
        button(
            c,
            f,
            text::SHOW_FPS,
            Look::Check(display.show_fps),
            Action::Set(Toggle::ShowFps),
        );
        fold(c, f, ui, "gfx", text::GRAPHICS, |k| graphics(k, f, o));
        fold(c, f, ui, "keys", text::KEYS, |k| {
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
                        if ui.rebinding == Some(b) {
                            rich(n, f, text::PRESS_KEY, 14.0, PINK);
                        } else {
                            heading(n, f, &crate::keys::labels(&o.binds.keys(b)));
                        }
                    });
                    button(r, f, text::CHANGE, Look::Tiny, Action::Rebind(b));
                });
            }
            muted(k, f, text::KEYS_NOTE);
            button(k, f, text::RESET_KEYS, Look::Tiny, Action::ResetKeys);
        });
        fold(c, f, ui, "problems", text::PROBLEMS, |k| {
            muted(k, f, text::PROBLEMS_NOTE);
            button(k, f, text::OPEN_LOGS, Look::Tiny, Action::OpenLogs);
        });
    });
}

fn graphics(p: &mut ChildSpawnerCommands, f: &Fonts, o: &Options) {
    use crate::render::quality::{Preset, Tier};
    let g = &*o.gfx;
    let tier = o.quality.as_ref().map_or(Tier::T2, |q| q.tier);
    let most = match tier {
        Tier::T0 => Preset::Low,
        Tier::T1 => Preset::Medium,
        Tier::T2 => Preset::High,
    };
    let chips = |p: &mut ChildSpawnerCommands, title: &str, items: &[(&str, GfxPick, bool, bool)]| {
        stack(p, |c| {
            muted(c, f, title);
            row(c, true, |r| {
                for (s, pick, on, ok) in items {
                    button_if(r, f, s, Look::Chip(*on), Action::Gfx(*pick), *ok);
                }
            });
        });
    };
    let presets: Vec<(&str, GfxPick, bool, bool)> = [
        ("auto", text::PRESET_AUTO, None),
        ("low", text::PRESET_LOW, Some(Preset::Low)),
        ("medium", text::PRESET_MEDIUM, Some(Preset::Medium)),
        ("high", text::PRESET_HIGH, Some(Preset::High)),
    ]
    .into_iter()
    .map(|(id, s, p)| (s, GfxPick::Preset(id), g.preset == id, p.is_none_or(|p| p <= most)))
    .collect();
    chips(p, text::PRESET, &presets);
    if let Some(q) = &o.quality {
        muted(p, f, &text::adapter(&q.adapter, &format!("{:?}", q.tier)));
    }
    let ups: Vec<(&str, GfxPick, bool, bool)> = text::UPSCALES
        .iter()
        .map(|(id, s)| {
            (
                *s,
                GfxPick::Upscale(id),
                g.upscale == *id || (g.upscale.is_empty() && *id == "off"),
                true,
            )
        })
        .collect();
    chips(p, text::UPSCALE, &ups);
    button(
        p,
        f,
        text::SHADOWS,
        Look::Check(g.shadows),
        Action::Set(Toggle::Shadows),
    );
    button_if(
        p,
        f,
        text::AO,
        Look::Check(g.ao && tier == Tier::T2),
        Action::Set(Toggle::Ao),
        tier == Tier::T2,
    );
    button(p, f, text::AA, Look::Check(g.aa), Action::Set(Toggle::Aa));
    button(p, f, text::GRADE, Look::Check(g.grade), Action::Set(Toggle::Grade));
    button(p, f, text::MOTES, Look::Check(g.motes), Action::Set(Toggle::Motes));
    button(p, f, text::VSYNC, Look::Check(g.vsync), Action::Set(Toggle::Vsync));
    let fps: Vec<(&str, GfxPick, bool, bool)> =
        [(0, text::NO_LIMIT), (30, "30"), (60, "60"), (120, "120"), (144, "144")]
            .into_iter()
            .map(|(n, s)| (s, GfxPick::Fps(n), g.fps_limit == n, true))
            .collect();
    chips(p, text::FPS_LIMIT, &fps);
    let backends: Vec<(&str, GfxPick, bool, bool)> = [
        ("", text::BACKEND_AUTO, true),
        ("vulkan", "Vulkan", true),
        ("dx12", "DirectX 12", cfg!(target_os = "windows")),
        ("gl", "OpenGL", true),
    ]
    .into_iter()
    .map(|(id, s, ok)| (s, GfxPick::Backend(id), g.backend == id, ok))
    .collect();
    chips(p, text::BACKEND, &backends);
}

/// Rebinding: the next key pressed becomes the action's (Esc lets it be). It ends when the keys are no
/// longer on screen (the menu closed as a round started, another tab): a key pressed in play is not a pick.
pub fn rebind(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui: ResMut<Ui>,
    mut binds: ResMut<crate::settings::Bindings>,
    session: Res<crate::session::Session>,
    mut commands: Commands,
) {
    let Some(b) = ui.rebinding else { return };
    let in_room = session.room.is_some() && session.arena.is_some();
    let shown = ui.open.contains("keys")
        && if in_room {
            ui.menu && ui.menu_tab == MenuTab::Settings
        } else {
            ui.home_tab == HomeTab::Settings
        };
    if !shown {
        ui.rebinding = None;
        return;
    }
    for k in keys.get_just_pressed() {
        if *k == KeyCode::Escape {
            ui.rebinding = None;
            return;
        }
        if crate::keys::label(*k).is_some() {
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
