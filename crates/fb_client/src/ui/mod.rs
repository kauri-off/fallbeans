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
pub const EMOJI: &[u8] = include_bytes!("../../../../assets/fonts/NotoColorEmoji-subset.ttf");

const fn hex(rgb: u32) -> Color {
    Color::srgb(
        ((rgb >> 16) & 0xff) as f32 / 255.0,
        ((rgb >> 8) & 0xff) as f32 / 255.0,
        (rgb & 0xff) as f32 / 255.0,
    )
}

// The soft-toy palette (`mock/c.html`): warm paper and cards, brown inks, one teal accent, apricot for "yours".
pub const INK: Color = hex(0x2F2B27);
pub const MUTED: Color = hex(0x5C544B);
pub const FAINT: Color = hex(0x71675C);
/// Placeholders and the text of what cannot be pressed.
pub const GHOST: Color = hex(0x958B80);
pub const ON_FILL: Color = Color::WHITE;
pub const PAPER: Color = hex(0xECE6DC);
pub const CARD: Color = hex(0xFBF8F3);
pub const CARD2: Color = hex(0xF3EEE6);
pub const WELL: Color = hex(0xEAE3D8);
pub const LINE: Color = hex(0xE0D7CA);
/// The rule between rows of a card.
pub const HAIR: Color = hex(0xEEE7DC);
pub const TEAL: Color = hex(0x3F6F66);
pub const TEAL_DEEP: Color = hex(0x335C54);
pub const TEAL_SOFT: Color = hex(0xDDE8E4);
pub const TEAL_INK: Color = hex(0x244A43);
pub const APRICOT: Color = hex(0xE2A07C);
pub const APRICOT_SOFT: Color = hex(0xF7E6DA);
pub const APRICOT_INK: Color = hex(0x8E4D2A);
pub const CRITICAL: Color = hex(0xAE4438);
pub const CRITICAL_SOFT: Color = hex(0xF4E0DB);
pub const GOOD: Color = hex(0x3B7350);
pub const GOOD_SOFT: Color = hex(0xE1EDE3);
pub const GOLD: Color = hex(0xD9B65E);
pub const SILVER: Color = hex(0xC2BEB5);
pub const BRONZE: Color = hex(0xC99470);
/// Cards over the game, a little see-through.
pub const PANEL: Color = Color::srgba(0.984, 0.973, 0.953, 0.96);
pub const SHADOW: Color = Color::srgba(0.275, 0.204, 0.133, 0.14);
/// The veil over the game under a card.
pub const DIM: Color = Color::srgba(0.188, 0.157, 0.125, 0.3);

/// Primary ink, faint: washes on the light surfaces.
pub const fn ink_wash(a: f32) -> Color {
    Color::srgba(0.184, 0.169, 0.153, a)
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

/// A game's genre: its colour, the soft fill of its tags and tiles, and the ink on that fill.
pub fn genre_tones(g: fb_shared::game::Genre) -> (Color, Color, Color) {
    use fb_shared::game::Genre;
    match g {
        Genre::Race => (hex(0x7591B0), hex(0xDFE6EE), hex(0x3C5A7A)),
        Genre::Survival => (hex(0xC4A15C), hex(0xF1E7D1), hex(0x6A511E)),
        Genre::Points => (hex(0x8BAD88), hex(0xE0EBDD), hex(0x3D633F)),
    }
}

#[derive(Resource, Clone)]
pub struct Fonts {
    /// Manrope SemiBold: running text.
    pub body: Handle<Font>,
    /// Manrope ExtraBold: names, headings, titles and numbers.
    pub strong: Handle<Font>,
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

/// A part of the settings, picked in their side bar (by name in BRP's `fb/ui`).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Section {
    #[default]
    Controls,
    Screen,
    Gfx,
    Keys,
    Problems,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Controls,
        Section::Screen,
        Section::Gfx,
        Section::Keys,
        Section::Problems,
    ];
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
    Section(Section),
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
    /// A light button with a rim.
    Plain,
    Primary,
    /// The big main action of a card.
    Go,
    /// Apricot: back to what is the player's own.
    Warm,
    Danger,
    Chip(bool),
    /// A chip in a long list of them.
    SmallChip(bool),
    Tab(bool),
    /// An entry of the settings' side bar.
    Nav(bool),
    Tiny,
    /// A bare icon («×»).
    Icon,
    /// A setting's row: its label, the switch at the end.
    Check(bool),
    /// A switch before its label.
    Toggle(bool),
    /// The head of a folded part.
    Fold,
    /// A colour swatch (selected or not).
    Swatch(Color, bool),
    /// A map's tile, in its genre's colours.
    Tile(fb_shared::game::Genre),
}

impl Look {
    /// The look it shares its colours with.
    fn base(self) -> Look {
        match self {
            Look::SmallChip(on) => Look::Chip(on),
            l => l,
        }
    }

    fn fill(self) -> (Color, Color) {
        match self.base() {
            Look::Plain | Look::Tiny => (CARD2, INK),
            Look::Primary | Look::Go => (TEAL, ON_FILL),
            Look::Warm => (APRICOT, hex(0x3E2414)),
            Look::Danger => (CRITICAL_SOFT, CRITICAL),
            Look::Chip(false) => (CARD, INK),
            Look::Chip(true) | Look::Nav(true) => (TEAL_SOFT, TEAL_INK),
            Look::Tab(false) | Look::Nav(false) => (Color::NONE, MUTED),
            Look::Tab(true) => (CARD, INK),
            Look::Icon => (Color::NONE, FAINT),
            Look::Check(_) | Look::Toggle(_) | Look::Fold => (Color::NONE, INK),
            Look::Swatch(c, _) => (c, INK),
            Look::Tile(g) => (genre_tones(g).1, INK),
            Look::SmallChip(_) => unreachable!("a small chip looks like a chip"),
        }
    }

    /// What it looks like when it cannot be pressed.
    fn off(self) -> (Color, Color) {
        match self.base() {
            Look::Chip(_) => (CARD2, GHOST),
            Look::Swatch(c, _) => (c.with_alpha(0.35), INK),
            Look::Check(_) | Look::Toggle(_) | Look::Fold | Look::Tab(_) | Look::Nav(_) | Look::Icon => {
                (Color::NONE, GHOST)
            }
            _ => (WELL, FAINT),
        }
    }

    fn hover(self) -> Color {
        match self.base() {
            Look::Primary | Look::Go => TEAL_DEEP,
            Look::Warm => hex(0xD99470),
            Look::Danger => hex(0xEDD3CD),
            Look::Plain | Look::Tiny => WELL,
            Look::Chip(false) | Look::Icon | Look::Nav(false) => CARD2,
            Look::Check(_) | Look::Toggle(_) | Look::Fold | Look::Tab(false) | Look::Swatch(..) | Look::Nav(true) => {
                self.fill().0
            }
            Look::Chip(true) | Look::Tab(true) | Look::Tile(_) | Look::SmallChip(_) => self.fill().0.darker(0.025),
        }
    }

    fn size(self) -> f32 {
        match self {
            Look::Tiny | Look::Tile(_) | Look::SmallChip(_) => 14.0,
            Look::Chip(_) | Look::Toggle(_) => 15.0,
            Look::Go => 18.0,
            _ => 16.0,
        }
    }

    fn strong(self) -> bool {
        !matches!(
            self.base(),
            Look::Chip(false) | Look::Check(_) | Look::Toggle(_) | Look::Icon
        )
    }

    fn padding(self) -> UiRect {
        match self {
            Look::Tiny => UiRect::axes(px(11), px(5)),
            Look::Chip(_) => UiRect::axes(px(14), px(7)),
            Look::SmallChip(_) => UiRect::axes(px(10), px(4)),
            Look::Tab(_) => UiRect::axes(px(20), px(9)),
            Look::Nav(_) => UiRect::axes(px(16), px(12)),
            Look::Icon => UiRect::axes(px(12), px(6)),
            Look::Tile(_) => UiRect::axes(px(12), px(10)),
            Look::Check(_) => UiRect::axes(px(0), px(14)),
            Look::Toggle(_) | Look::Swatch(..) => UiRect::ZERO,
            Look::Fold => UiRect::axes(px(0), px(4)),
            Look::Go => UiRect::axes(px(28), px(15)),
            _ => UiRect::axes(px(20), px(11)),
        }
    }

    fn border(self) -> UiRect {
        match self.base() {
            Look::Plain | Look::Tiny | Look::Chip(_) => UiRect::all(px(1)),
            Look::Check(_) => UiRect::bottom(px(1)),
            _ => UiRect::ZERO,
        }
    }

    fn rim(self) -> BorderColor {
        match self.base() {
            Look::Chip(true) => BorderColor::all(hex(0xB9D0C8)),
            Look::Plain | Look::Tiny | Look::Chip(false) => BorderColor::all(LINE),
            Look::Check(_) => BorderColor::all(HAIR),
            _ => BorderColor::all(Color::NONE),
        }
    }

    fn radius(self) -> Val {
        match self.base() {
            Look::Chip(_) | Look::Swatch(..) => px(f32::MAX),
            Look::Tiny => px(10),
            Look::Tab(_) | Look::Nav(_) | Look::Icon => px(12),
            Look::Go => px(16),
            Look::Check(_) | Look::Toggle(_) | Look::Fold => px(0),
            _ => px(14),
        }
    }

    fn shadow(self) -> Option<BoxShadow> {
        match self.base() {
            Look::Primary | Look::Go | Look::Warm => Some(BoxShadow::new(
                Color::srgba(0.157, 0.235, 0.216, 0.18),
                px(0),
                px(1),
                px(0),
                px(2),
            )),
            Look::Tab(true) => Some(BoxShadow::new(SHADOW, px(0), px(1), px(0), px(3))),
            _ => None,
        }
    }

    /// The ring round the swatch picked.
    fn outline(self) -> Outline {
        match self {
            Look::Swatch(_, true) => Outline::new(px(2), px(2), INK),
            _ => Outline::new(px(0), px(0), Color::NONE),
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

/// Text fields, found by what they hold (by name in BRP's `fb/field`).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
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
            emoji: fonts.add(Font::from_bytes(EMOJI.to_vec())),
        };
        app.insert_resource(f);
        app.init_resource::<Ui>();
        app.init_resource::<Folds>();
        app.init_resource::<Section>();
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
                    sync_sections.run_if(resource_changed::<Section>.or_else(any_match_filter::<Added<SectionBody>>)),
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
                    Layer::Menu => l.insert(motion::Reveal::new(motion::Motion::slide(0.0, 24.0))),
                    Layer::Home => l.insert(motion::Reveal::new(motion::Motion::slide(0.0, 16.0).lasting(0.35))),
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
fn scale_ui(
    windows: Query<&Window, With<PrimaryWindow>>,
    offscreen: Option<Res<crate::Offscreen>>,
    display: Res<Display>,
    mut scale: ResMut<UiScale>,
) {
    let image = offscreen.map_or((1600.0, 900.0), |o| (o.1.x, o.1.y));
    let (w, h) = windows.single().map_or(image, |w| (w.width(), w.height()));
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
    mut faces: Query<(&mut BackgroundColor, &mut UiTransform)>,
) {
    for (look, hovered, pressed, disabled, face) in &q {
        let Ok((mut bg, mut tf)) = faces.get_mut(face.0) else {
            continue;
        };
        let hover = hovered.get() && !disabled;
        let c = if disabled {
            look.off().0
        } else if hover {
            look.hover()
        } else {
            look.fill().0
        };
        bg.set_if_neq(BackgroundColor(c));
        let still = disabled || matches!(look, Look::Check(_) | Look::Toggle(_) | Look::Fold | Look::Nav(_));
        let k = if still {
            1.0
        } else if pressed {
            0.97
        } else if matches!(look, Look::Swatch(..)) && hover {
            1.1
        } else {
            1.0
        };
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
    section: ResMut<'w, Section>,
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
            Action::Section(k) => *tabs.section = *k,
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
