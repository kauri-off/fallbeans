//! The bevy_ui interface: screens are built once and updated in place; each button's `Action` becomes a `UiAction`.
pub mod text;

mod chat;
mod home;
mod hud;
mod loading;
mod menu;
pub mod motion;
mod settings;
mod tags;
mod widgets;

pub use settings::*;
pub use widgets::*;

#[cfg(test)]
pub use home::Form;

use std::collections::BTreeSet;

use bevy::asset::AssetId;
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::ecs::system::SystemParam;
use bevy::input_focus::InputFocus;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle, TextEdit};
use bevy::ui::{InteractionDisabled, Pressed, UiSystems};
use bevy::ui_widgets::{Activate, Slider, SliderRange, SliderStep, SliderThumb, SliderValue, ValueChange};
use bevy::window::PrimaryWindow;
use fb_proto::MapId;
use fb_proto::{ClientMsg, Outfit};

use crate::keys::Bind;
use crate::render::upscale::Upscaler;
use crate::session::{RoomList, Session};
use crate::settings::{Bindings, Controls, Display, Graphics, Sound};
use crate::view::color;

const BODY: &[u8] = include_bytes!("../../../../assets/fonts/Manrope-SemiBold.ttf");
const STRONG: &[u8] = include_bytes!("../../../../assets/fonts/Manrope-ExtraBold.ttf");
const DISPLAY: &[u8] = include_bytes!("../../../../assets/fonts/Unbounded-Bold.ttf");
pub const EMOJI: &[u8] = include_bytes!("../../../../assets/fonts/NotoColorEmoji-subset.ttf");

// The dataviz skill's reference palette, light mode (its `palette.md`): chrome and ink, categorical slots,
// the blue ramp and the status colours. Text wears the inks; a colour beside it carries the meaning.
pub const INK: Color = Color::srgb(0.043, 0.043, 0.043);
pub const MUTED: Color = Color::srgb(0.3216, 0.3176, 0.3059);
pub const FAINT: Color = Color::srgb(0.537, 0.529, 0.506);
/// Text on the saturated fills (blue, green, critical).
pub const ON_FILL: Color = Color::WHITE;
/// Categorical slot 1 and blue ramp 450: the interface's one accent.
pub const BLUE: Color = Color::srgb(0.165, 0.471, 0.839);
/// Blue ramp 400 and 500: the gradient of the main buttons.
pub const BLUE_400: Color = Color::srgb(0.224, 0.529, 0.898);
pub const BLUE_500: Color = Color::srgb(0.145, 0.416, 0.749);
/// Blue ramp 550 and 100.
pub const BLUE_DEEP: Color = Color::srgb(0.11, 0.361, 0.671);
pub const BLUE_SOFT: Color = Color::srgb(0.804, 0.886, 0.984);
/// Categorical slot 6, and status good.
pub const GREEN: Color = Color::srgb(0.0, 0.514, 0.0);
pub const GREEN_DEEP: Color = Color::srgb(0.047, 0.639, 0.047);
/// Status: good text, serious, warning, critical.
pub const GOOD: Color = Color::srgb(0.0, 0.388, 0.0);
pub const SERIOUS: Color = Color::srgb(0.925, 0.514, 0.353);
pub const WARNING: Color = Color::srgb(0.98, 0.698, 0.098);
pub const CRITICAL: Color = Color::srgb(0.816, 0.231, 0.231);
/// The medals: categorical yellow, the baseline grey, categorical orange.
pub const GOLD: Color = Color::srgb(0.929, 0.631, 0.0);
pub const SILVER: Color = Color::srgb(0.765, 0.761, 0.718);
pub const BRONZE: Color = Color::srgb(0.922, 0.408, 0.204);
/// The page plane, the chart surface (panels) and the neutral (cards in them).
pub const PLANE: Color = Color::srgb(0.976, 0.976, 0.969);
/// The chart surface, opaque: buttons and fields.
pub const SURFACE: Color = Color::srgb(0.988, 0.988, 0.984);
pub const PANEL: Color = Color::srgba(0.988, 0.988, 0.984, 0.94);
pub const NEUTRAL: Color = Color::srgb(0.941, 0.937, 0.925);
/// The hairline ring.
pub const RIM: Color = ink_wash(0.1);
pub const GROUP: Color = Color::srgba(0.941, 0.937, 0.925, 0.7);
pub const SHADOW: Color = Color::srgba(0.043, 0.043, 0.043, 0.2);

/// Primary ink, faint: washes and rules on the light surfaces.
pub const fn ink_wash(a: f32) -> Color {
    Color::srgba(0.043, 0.043, 0.043, a)
}

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

/// A game's genre as a colour: categorical slots 1-3, the ones validated all-pairs (`validate_palette.js`).
pub fn genre_color(g: fb_shared::game::Genre) -> Color {
    use fb_shared::game::Genre;
    match g {
        Genre::Race => Color::srgb(0.165, 0.471, 0.839),
        Genre::Survival => Color::srgb(0.922, 0.408, 0.204),
        Genre::Points => Color::srgb(0.106, 0.686, 0.478),
    }
}

#[derive(Resource, Clone)]
pub struct Fonts {
    /// Manrope SemiBold: running text.
    pub body: Handle<Font>,
    /// Manrope ExtraBold: names, headings, numbers in tables.
    pub strong: Handle<Font>,
    /// Unbounded: the logo, titles, the timer and the countdown.
    pub display: Handle<Font>,
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
    Practice(MapId),
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
    /// A switch with its label.
    Check(bool),
    /// The head of a folded part.
    Fold,
    /// A colour swatch (selected or not).
    Swatch(Color, bool),
    /// A map's tile, with its genre's colour.
    Tile(Color),
}

impl Look {
    fn fill(self) -> (Color, Color) {
        match self {
            Look::Plain | Look::Tiny | Look::Chip(false) | Look::Tile(_) => (SURFACE, INK),
            Look::Tab(false) => (Color::NONE, MUTED),
            Look::Tab(true) => (SURFACE, INK),
            Look::Primary => (BLUE, ON_FILL),
            Look::Go => (GREEN, ON_FILL),
            Look::Danger => (CRITICAL, ON_FILL),
            Look::TinyDanger => (CRITICAL.with_alpha(0.1), CRITICAL),
            Look::Chip(true) => (BLUE, ON_FILL),
            Look::Check(_) => (Color::NONE, INK),
            Look::Fold => (Color::NONE, INK),
            Look::Swatch(c, _) => (c, INK),
        }
    }

    /// The big buttons are lit by a gradient across them.
    fn gradient(self, dim: bool) -> Option<BackgroundGradient> {
        let (a, b) = match self {
            Look::Primary => (BLUE_400, BLUE_500),
            Look::Go => (GREEN_DEEP, GREEN),
            _ => return None,
        };
        let k = if dim { 0.4 } else { 1.0 };
        Some(BackgroundGradient::from(LinearGradient::to_right(vec![
            a.with_alpha(k).into(),
            b.with_alpha(k).into(),
        ])))
    }

    /// Hovered, the face is lit from within rather than recoloured.
    fn flat(self) -> bool {
        matches!(self, Look::Check(_) | Look::Fold | Look::Tab(false))
    }

    fn size(self) -> f32 {
        match self {
            Look::Tiny | Look::TinyDanger => 12.0,
            Look::Chip(_) | Look::Tab(_) | Look::Tile(_) => 13.5,
            Look::Check(_) => 14.5,
            Look::Fold => 15.0,
            Look::Primary | Look::Go | Look::Danger => 15.5,
            _ => 14.5,
        }
    }

    fn strong(self) -> bool {
        matches!(
            self,
            Look::Primary | Look::Go | Look::Danger | Look::Tab(_) | Look::Fold | Look::Chip(true) | Look::Tile(_)
        )
    }

    fn padding(self) -> UiRect {
        match self {
            Look::Tiny | Look::TinyDanger => UiRect::axes(rem(0.625), rem(0.25)),
            Look::Chip(_) => UiRect::axes(rem(0.875), rem(0.375)),
            Look::Tab(_) => UiRect::axes(rem(1.0), rem(0.5)),
            Look::Tile(_) => UiRect::new(rem(0.75), rem(0.75), rem(0.625), rem(0.625)),
            Look::Check(_) => UiRect::axes(px(0), rem(0.25)),
            Look::Fold => UiRect::axes(rem(0.875), rem(0.75)),
            Look::Swatch(..) => UiRect::ZERO,
            Look::Primary | Look::Go | Look::Danger => UiRect::axes(rem(1.375), rem(0.75)),
            _ => UiRect::axes(rem(1.125), rem(0.625)),
        }
    }

    fn border(self) -> UiRect {
        match self {
            Look::Swatch(_, on) => UiRect::all(px(if on { 3 } else { 2 })),
            Look::Tile(_) => UiRect::new(px(1), px(1), px(3), px(1)),
            _ => UiRect::all(px(1)),
        }
    }

    fn rim(self) -> BorderColor {
        match self {
            Look::Check(_) | Look::Fold | Look::Tab(false) => BorderColor::all(Color::NONE),
            Look::Tab(true) => BorderColor::all(RIM),
            Look::Swatch(_, true) => BorderColor::all(INK),
            Look::Swatch(c, false) => BorderColor::all(dot_rim(c)),
            Look::Tile(c) => BorderColor {
                top: c,
                ..BorderColor::all(RIM)
            },
            Look::Plain | Look::Tiny | Look::Chip(false) => BorderColor::all(RIM),
            _ => BorderColor::all(Color::NONE),
        }
    }

    fn radius(self) -> Val {
        match self {
            Look::Tab(_) | Look::Chip(_) | Look::Tiny | Look::TinyDanger | Look::Swatch(..) => px(f32::MAX),
            Look::Fold => rem(1.0),
            Look::Check(_) => px(0),
            _ => rem(0.875),
        }
    }

    fn shadow(self) -> Option<BoxShadow> {
        match self {
            Look::Swatch(..) => Some(BoxShadow::new(SHADOW, px(0), px(1), px(0), px(4))),
            Look::Primary => Some(BoxShadow::new(BLUE.with_alpha(0.3), px(0), px(4), px(0), px(14))),
            Look::Go => Some(BoxShadow::new(GREEN.with_alpha(0.25), px(0), px(4), px(0), px(14))),
            Look::Danger => Some(BoxShadow::new(CRITICAL.with_alpha(0.25), px(0), px(4), px(0), px(14))),
            Look::Chip(true) => Some(BoxShadow::new(BLUE.with_alpha(0.25), px(0), px(2), px(0), px(8))),
            Look::Tab(true) | Look::Plain | Look::Tiny | Look::Chip(false) | Look::Tile(_) => {
                Some(BoxShadow::new(SHADOW.with_alpha(0.08), px(0), px(1), px(0), px(3)))
            }
            _ => None,
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
        let _ = fonts.insert(AssetId::default(), Font::from_bytes(BODY.to_vec()));
        let f = Fonts {
            body: Handle::default(),
            strong: fonts.add(Font::from_bytes(STRONG.to_vec())),
            display: fonts.add(Font::from_bytes(DISPLAY.to_vec())),
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
                focus_ring.run_if(resource_changed::<InputFocus>),
                (
                    ui_actions,
                    sync_folds.run_if(resource_changed::<Folds>.or_else(any_match_filter::<Added<FoldChevron>>)),
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
            motion::MotionPlugin,
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
                let mut l = r.spawn((
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
                ));
                match layer {
                    Layer::Menu => l.insert(motion::Reveal::new(motion::Motion::slide(56.0, 0.0))),
                    Layer::Home => l.insert(motion::Reveal::new(motion::Motion::slide(-40.0, 0.0).lasting(0.4))),
                    _ => &mut l,
                };
                layers[layer as usize] = l.id();
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
    mut thumbs: Query<&mut Node, (With<SliderThumb>, Without<SliderFill>)>,
    mut fills: Query<&mut Node, (With<SliderFill>, Without<SliderThumb>)>,
) {
    for (e, v, range) in &sliders {
        let at = percent(range.thumb_position(v.0) * 100.0);
        for c in children.iter_descendants(e) {
            if let Ok(mut n) = thumbs.get_mut(c) {
                n.left = at;
            }
            if let Ok(mut n) = fills.get_mut(c) {
                n.width = at;
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

type ButtonState = (
    &'static Look,
    &'static Hovered,
    Has<Pressed>,
    Has<InteractionDisabled>,
    &'static Face,
);

fn style_buttons(
    q: Query<ButtonState, With<bevy::ui_widgets::Button>>,
    mut faces: Query<(&mut BackgroundColor, &mut UiTransform, Option<&mut BackgroundGradient>)>,
) {
    for (look, hovered, pressed, disabled, face) in &q {
        let Ok((mut bg, mut tf, gradient)) = faces.get_mut(face.0) else {
            continue;
        };
        let hover = hovered.get() && !disabled;
        let (base, _) = look.fill();
        let c = if look.flat() {
            if hover { INK.with_alpha(0.06) } else { Color::NONE }
        } else if disabled {
            base.with_alpha(base.alpha() * 0.4)
        } else if look.gradient(false).is_some() {
            Color::NONE
        } else if hover {
            // (A light face darkens a little, a saturated one lightens.)
            if base.luminance() > 0.6 {
                base.darker(0.04)
            } else {
                base.lighter(0.05)
            }
        } else {
            base
        };
        bg.set_if_neq(BackgroundColor(c));
        if let (Some(mut g), Some(want)) = (gradient, look.gradient(disabled)) {
            g.set_if_neq(want);
        }
        let still = disabled || matches!(look, Look::Check(_) | Look::Fold | Look::Swatch(..));
        let (y, k) = if still {
            (0.0, 1.0)
        } else if pressed {
            (1.0, 0.97)
        } else if hover {
            (-2.0, 1.0)
        } else {
            (0.0, 1.0)
        };
        let k = if matches!(look, Look::Swatch(..)) && hover {
            1.12
        } else {
            k
        };
        let next = Val2::px(0.0, y);
        if tf.translation != next {
            tf.translation = next;
        }
        if tf.scale != Vec2::splat(k) {
            tf.scale = Vec2::splat(k);
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

/// The settings the interface toggles.
#[derive(SystemParam)]
struct SettingsMut<'w> {
    controls: ResMut<'w, Controls>,
    display: ResMut<'w, Display>,
    binds: ResMut<'w, Bindings>,
    gfx: ResMut<'w, Graphics>,
}

/// Which tabs and folds are open.
#[derive(SystemParam)]
struct Tabs<'w> {
    folds: ResMut<'w, Folds>,
    home_tab: ResMut<'w, NextState<HomeTab>>,
    menu_tab: ResMut<'w, NextState<MenuTab>>,
}

/// Actions every screen shares: tabs, folds, settings toggles, quitting.
fn ui_actions(
    mut actions: MessageReader<UiAction>,
    mut ui: ResMut<Ui>,
    mut tabs: Tabs,
    mut settings: SettingsMut,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    for UiAction(a) in actions.read() {
        match a {
            Action::HomeTab(t) => tabs.home_tab.set(*t),
            Action::MenuTab(t) => tabs.menu_tab.set(*t),
            Action::Fold(k) => tabs.folds.toggle(*k),
            Action::Set(t) => {
                match t {
                    Toggle::InvertMouse => settings.controls.invert_mouse_y ^= true,
                    Toggle::InvertStick => settings.controls.invert_stick_y ^= true,
                    Toggle::Shake => settings.controls.camera_shake ^= true,
                    Toggle::ShowFps => settings.display.show_fps ^= true,
                    Toggle::Fullscreen => settings.display.fullscreen ^= true,
                    Toggle::Vsync => settings.gfx.vsync ^= true,
                }
                crate::settings::save_soon(&mut commands);
            }
            Action::Quit => {
                exit.write(AppExit::Success);
            }
            Action::Gfx(pick) => {
                match *pick {
                    GfxPick::Preset(p) => settings.gfx.preset = p,
                    GfxPick::Fps(n) => settings.gfx.fps_limit = n,
                    GfxPick::Backend(b) => settings.gfx.backend = crate::backend::BackendSetting(b),
                    GfxPick::Upscaler(u) => settings.gfx.upscaler = crate::render::upscale::UpscalerSetting(u),
                    GfxPick::Upscale(m) => settings.gfx.upscale = m,
                }
                crate::settings::save_soon(&mut commands);
            }
            Action::Rebind(b) => ui.rebinding = Some(*b),
            Action::ResetKeys => {
                *settings.binds = Bindings::default();
                ui.rebinding = None;
                crate::settings::save_soon(&mut commands);
            }
            _ => {}
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
