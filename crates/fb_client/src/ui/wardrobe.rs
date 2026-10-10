//! The player's look in the room menu: the bean itself (`preview.rs`), a list for the hat and for the glasses, a
//! palette for each colour; what the pointer rests on in a list or palette is tried on the bean at once.
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::picking::events::{Drag, Pointer};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::{InteractionDisabled, UiGlobalTransform, UiSystems};
use fb_proto::Phase;
use fb_shared::outfit::{GLASSES, HATS, Hat, Outfit, Tint};
use fb_shared::{COLORS, Suit};

use super::*;
use crate::preview::{Showcase, Stage, Turntable};
use crate::session::Session;
use crate::settings::{Me, Player};

/// What a list or palette picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Suit,
    Hat,
    HatColor,
    Glasses,
    Belly,
    Shoes,
}

/// The list or palette open.
#[derive(Resource, Default)]
pub struct Picking(pub Option<Pick>);

/// The card of the bean and its look: lists and palettes open beside it, leaving the bean in sight.
#[derive(Component)]
struct WardrobeCard;
/// Where the bean is shown.
#[derive(Component)]
struct StageView;
/// The look's controls under it.
#[derive(Component)]
struct WardrobeBox;
/// The open list or palette, over everything.
#[derive(Component)]
struct PickerPop;
/// A control that opens a list or palette.
#[derive(Component)]
struct Anchor(Pick);
/// A choice tried on while hovered.
#[derive(Component)]
struct TryOn;

pub struct WardrobePlugin;

impl Plugin for WardrobePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Picking>();
        app.add_systems(Startup, build_pop.after(super::setup));
        let changed = || {
            resource_changed::<Player>
                .or_else(resource_changed::<Session>)
                .or_else(resource_changed::<Picking>)
        };
        app.add_systems(
            Update,
            (
                (picks, close_outside, esc_closes.before(super::menu::menu_flow)).chain(),
                attach_stage,
                (controls.run_if(changed()), open_looks, pop.run_if(changed())).chain(),
                showcase,
            )
                .chain(),
        );
        app.add_systems(PostUpdate, (place_pop, fit_stage).after(UiSystems::Layout));
    }
}

/// The suit's index into `COLORS`: the room's, or the one asked for.
fn own_suit(me: &Me, session: &Session) -> u8 {
    session
        .me
        .and_then(|id| session.player(id))
        .map(|p| p.color)
        .or_else(|| me.color())
        .unwrap_or(0)
}

/// The suit can be changed in the lobby only, to a colour no one else wears.
fn suit_free(session: &Session, i: u8) -> bool {
    session
        .lobby
        .as_ref()
        .is_some_and(|l| l.phase == Phase::Lobby && !l.players.iter().any(|p| p.color == i && Some(p.id) != session.me))
}

fn can_paint(session: &Session) -> bool {
    session.lobby.as_ref().is_none_or(|l| l.phase == Phase::Lobby)
}

/// The bean (turned by dragging it) and its look's controls under it.
pub(super) fn bean_card(p: &mut ChildSpawnerCommands, f: &Fonts) {
    let card = p.target_entity();
    p.commands().entity(card).insert(WardrobeCard);
    p.spawn((
        StageView,
        Node {
            flex_grow: 1.0,
            min_height: px(150),
            border_radius: BorderRadius::all(px(16)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(CARD2),
    ))
    .with_children(|v| {
        let hint = rich_in(v, f, text::TURN_HINT, 12.0, GHOST, false);
        v.commands().entity(hint).insert((
            Node {
                position_type: PositionType::Absolute,
                right: px(12),
                top: px(10),
                ..default()
            },
            Pickable::IGNORE,
        ));
    })
    .observe(
        |d: On<Pointer<Drag>>, mut table: ResMut<Turntable>, time: Res<Time<Real>>| {
            table.drag(d.delta.x, time.delta_secs());
        },
    );
    p.spawn((
        WardrobeBox,
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            flex_shrink: 0.0,
            ..default()
        },
    ));
}

/// The stage's picture, drawn at the size it is shown.
#[derive(Component)]
struct StagePicture;

/// The stage's picture fills its view.
fn attach_stage(stage: Option<Res<Stage>>, views: Query<Entity, Added<StageView>>, mut commands: Commands) {
    let Some(stage) = stage else { return };
    for e in &views {
        commands.entity(e).with_child((
            StagePicture,
            ImageNode::new(stage.image.clone()),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
        ));
    }
}

fn controls(
    player: Res<Player>,
    me: Me,
    session: Res<Session>,
    picking: Res<Picking>,
    boxes: Query<Entity, With<WardrobeBox>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    let o = player.outfit();
    let open = picking.0;
    let suit_i = own_suit(&me, &session);
    for e in &boxes {
        rebuild(&mut commands, e, |p| {
            line(p, f, text::HAT, |r| {
                select(r, f, Pick::Hat, text::hat(o.hat), open);
                if o.hat != Hat::None {
                    paint(
                        r,
                        f,
                        Pick::HatColor,
                        text::TINT,
                        o.hat_color.map(Dot::Color),
                        open,
                        true,
                    );
                }
            });
            line(p, f, text::GLASSES, |r| {
                select(r, f, Pick::Glasses, text::glasses(o.glasses), open);
            });
            line(p, f, text::COLORS, |r| {
                let here = r.target_entity();
                r.commands().entity(here).insert(Node {
                    flex_grow: 1.0,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: px(6),
                    row_gap: px(6),
                    align_items: AlignItems::Center,
                    ..default()
                });
                paint(
                    r,
                    f,
                    Pick::Suit,
                    text::SUIT,
                    Some(suit_dot(suit_i)),
                    open,
                    can_paint(&session),
                );
                paint(
                    r,
                    f,
                    Pick::Belly,
                    text::BELLY_SHORT,
                    o.belly.map(Dot::Color),
                    open,
                    true,
                );
                paint(
                    r,
                    f,
                    Pick::Shoes,
                    text::SHOES_SHORT,
                    o.shoes.map(Dot::Color),
                    open,
                    true,
                );
            });
            row(p, false, |r| {
                button(r, f, text::RANDOM, Look::Tiny, Action::RandomOutfit);
                button(r, f, text::RESET, Look::Tiny, Action::Wear(Outfit::default()));
            });
        });
    }
}

/// What a colour's dot shows.
#[derive(Clone, Copy)]
enum Dot {
    Color(Tint),
    Suit(Color),
    Rainbow,
}

fn suit_dot(i: u8) -> Dot {
    match COLORS.get(i as usize) {
        Some(Suit::Rainbow) => Dot::Rainbow,
        _ => Dot::Suit(suit(i)),
    }
}

/// The rainbow suit's colours round a circle.
fn rainbow() -> BackgroundGradient {
    let stops = (0..=6)
        .map(|i| AngularColorStop::auto(Color::hsl(i as f32 * 60.0, 0.85, 0.62)))
        .collect();
    ConicGradient::new(UiPosition::CENTER, stops).into()
}

fn tint(t: Tint) -> Color {
    color(t.rgb())
}

/// A line of controls after its label.
fn line(p: &mut ChildSpawnerCommands, f: &Fonts, title: &str, inner: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        column_gap: px(10),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|l| {
        l.spawn(Node {
            width: px(58),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|t| {
            caption(t, f, title);
        });
        l.spawn(Node {
            flex_grow: 1.0,
            min_width: px(0),
            column_gap: px(8),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(inner);
    });
}

/// A field that opens its list: what is chosen, and an arrow.
fn select(p: &mut ChildSpawnerCommands, f: &Fonts, pick: Pick, now: &str, open: Option<Pick>) {
    let look = Look::Select(open == Some(pick));
    let face = (
        Node {
            flex_grow: 1.0,
            padding: look.padding(),
            border: look.border(),
            border_radius: BorderRadius::all(look.radius()),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            column_gap: px(8),
            ..default()
        },
        BackgroundColor(look.fill().0),
        look.rim(),
    );
    let b = button_shell(p, look, Action::Pick(pick), true, 1.0, face, |b| {
        let t = rich_with(b, f, now, look.size(), INK, false, Some(Pickable::IGNORE));
        b.commands().entity(t).insert(TextLayout::no_wrap());
        b.spawn((
            Text::new("›"),
            TextFont {
                font: f.strong.clone().into(),
                font_size: FontSize::Px(18.0),
                ..default()
            },
            TextColor(FAINT),
            UiTransform::from_rotation(Rot2::degrees(90.0)),
            Pickable::IGNORE,
        ));
    });
    p.commands().entity(b).insert((
        Anchor(pick),
        Node {
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(0),
            ..default()
        },
    ));
}

/// A colour that opens its palette: a dot of it (or of none: "as designed") and its name.
fn paint(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    pick: Pick,
    title: &str,
    now: Option<Dot>,
    open: Option<Pick>,
    enabled: bool,
) {
    let look = Look::Chip(open == Some(pick));
    let (bg, ink) = if enabled { look.fill() } else { look.off() };
    let pad = if title.is_empty() { px(5) } else { px(12) };
    let face = (
        Node {
            padding: UiRect::new(px(5), pad, px(5), px(5)),
            border: look.border(),
            border_radius: BorderRadius::MAX,
            align_items: AlignItems::Center,
            column_gap: px(7),
            ..default()
        },
        BackgroundColor(bg),
        look.rim(),
    );
    let b = button_shell(p, look, Action::Pick(pick), enabled, 0.0, face, |b| {
        color_dot(b, now, 22.0);
        if !title.is_empty() {
            rich_with(b, f, title, 14.0, ink, false, Some(Pickable::IGNORE));
        }
    });
    p.commands().entity(b).insert(Anchor(pick));
}

/// A colour's dot; without one, a dashed ring (the part's own colour).
fn color_dot(p: &mut ChildSpawnerCommands, c: Option<Dot>, size: f32) {
    match c {
        Some(d) => {
            let mut e = p.spawn((
                Node {
                    width: px(size),
                    height: px(size),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BorderColor::all(ink_wash(0.12)),
                Pickable::IGNORE,
            ));
            match d {
                Dot::Color(t) => e.insert(BackgroundColor(tint(t))),
                Dot::Suit(c) => e.insert(BackgroundColor(c)),
                Dot::Rainbow => e.insert(rainbow()),
            };
        }
        None => {
            p.spawn((
                Node {
                    width: px(size),
                    height: px(size),
                    border: UiRect::all(px(1.5)),
                    border_radius: BorderRadius::MAX,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(CARD),
                BorderColor::all(LINE),
                Pickable::IGNORE,
            ))
            .with_children(|r| {
                r.spawn((
                    Node {
                        width: px(size * 0.4),
                        height: px(size * 0.4),
                        border: UiRect::all(px(1.5)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BorderColor::all(GHOST),
                    Pickable::IGNORE,
                ));
            });
        }
    }
}

/// The controls show which one has its list or palette open.
fn open_looks(picking: Res<Picking>, mut anchors: Query<(&Anchor, &mut Look)>) {
    if !picking.is_changed() {
        return;
    }
    for (a, mut look) in &mut anchors {
        let on = picking.0 == Some(a.0);
        let next = match *look {
            Look::Select(_) => Look::Select(on),
            Look::Chip(_) => Look::Chip(on),
            l => l,
        };
        look.set_if_neq(next);
    }
}

fn build_pop(mut commands: Commands, layers: Res<Layers>) {
    commands.entity(layers[Layer::Menu]).with_child((
        PickerPop,
        Hovered::default(),
        Node {
            position_type: PositionType::Absolute,
            display: display(false),
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            padding: UiRect::all(px(14)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(16)),
            ..default()
        },
        BorderColor::all(LINE),
        GlobalZIndex(10),
        Visibility::Hidden,
        panel_raised(),
    ));
}

type PopParts = (&'static mut Node, &'static mut Visibility);

fn pop(
    picking: Res<Picking>,
    player: Res<Player>,
    me: Me,
    session: Res<Session>,
    mut q: Single<(Entity, PopParts), With<PickerPop>>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let f = &*f;
    let (e, (node, vis)) = &mut *q;
    show(node, picking.0.is_some());
    vis.set_if_neq(Visibility::Hidden);
    let Some(pick) = picking.0 else {
        commands.entity(*e).despawn_children();
        return;
    };
    let o = player.outfit();
    let wear = |patch: &dyn Fn(&mut Outfit)| {
        let mut n = o;
        patch(&mut n);
        Action::Wear(n)
    };
    rebuild(&mut commands, *e, |p| match pick {
        Pick::Hat => {
            caption(p, f, text::HAT);
            list(p, |l| {
                for h in HATS {
                    item(l, f, text::hat(h), o.hat == h, wear(&|n| n.hat = h));
                }
            });
        }
        Pick::Glasses => {
            caption(p, f, text::GLASSES);
            list(p, |l| {
                for g in GLASSES {
                    item(l, f, text::glasses(g), o.glasses == g, wear(&|n| n.glasses = g));
                }
            });
        }
        Pick::Suit => {
            caption(p, f, text::SUIT_COLOR);
            let now = own_suit(&me, &session);
            palette(p, |r| {
                for i in 0..COLORS.len() as u8 {
                    let s = suit_swatch(r, i, now == i, suit_free(&session, i));
                    r.commands().entity(s).insert(TryOn);
                }
            });
        }
        Pick::HatColor | Pick::Belly | Pick::Shoes => {
            let (title, now, set): (_, _, &SetTint) = match pick {
                Pick::HatColor => (text::HAT_COLOR, o.hat_color, &|n, t| n.hat_color = t),
                Pick::Belly => (text::BELLY, o.belly, &|n, t| n.belly = t),
                _ => (text::SHOES, o.shoes, &|n, t| n.shoes = t),
            };
            caption(p, f, title);
            palette(p, |r| {
                let s = swatch_none(r, now.is_none(), wear(&|n| set(n, None)), 30.0);
                r.commands().entity(s).insert(TryOn);
                for t in Tint::ALL {
                    let s = swatch(r, tint(t), now == Some(t), wear(&|n| set(n, Some(t))), true, 30.0);
                    r.commands().entity(s).insert(TryOn);
                }
            });
        }
    });
}

/// A suit's swatch (the rainbow's in all its colours).
fn suit_swatch(p: &mut ChildSpawnerCommands, i: u8, on: bool, enabled: bool) -> Entity {
    let Dot::Rainbow = suit_dot(i) else {
        return swatch(p, suit(i), on, Action::Color(i), enabled, 30.0);
    };
    let look = Look::Swatch(suit(i), on);
    let face = (
        Node {
            width: px(30),
            height: px(30),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(suit(i)),
        rainbow(),
        BorderColor::all(ink_wash(0.08)),
        look.outline(),
    );
    button_shell(p, look, Action::Color(i), enabled, 0.0, face, |_| {})
}

/// Puts a colour on a part of an outfit.
type SetTint = dyn Fn(&mut Outfit, Option<Tint>);

/// Choices in two columns.
fn list(p: &mut ChildSpawnerCommands, inner: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        display: bevy::ui::Display::Grid,
        grid_template_columns: RepeatedGridTrack::auto(2),
        column_gap: px(4),
        row_gap: px(2),
        ..default()
    })
    .with_children(inner);
}

/// Swatches, seven to a row.
fn palette(p: &mut ChildSpawnerCommands, inner: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        display: bevy::ui::Display::Grid,
        grid_template_columns: RepeatedGridTrack::px(7, 30.0),
        column_gap: px(10),
        row_gap: px(10),
        padding: UiRect::all(px(3)),
        ..default()
    })
    .with_children(inner);
}

fn item(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, on: bool, act: Action) {
    let look = Look::Item(on);
    let face = (
        Node {
            flex_grow: 1.0,
            padding: look.padding(),
            border_radius: BorderRadius::all(look.radius()),
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(look.fill().0),
    );
    let b = button_shell(p, look, act, true, 0.0, face, |b| {
        let t = rich_with(
            b,
            f,
            s,
            look.size(),
            look.fill().1,
            look.strong(),
            Some(Pickable::IGNORE),
        );
        b.commands().entity(t).insert(TextLayout::no_wrap());
    });
    p.commands().entity(b).insert(TryOn);
}

/// The stage is drawn as big as its picture is on screen (within a texture's limits: nothing, or nonsense, before
/// the first layout).
fn fit_stage(
    stage: Option<Res<Stage>>,
    pictures: Query<&ComputedNode, (With<StagePicture>, Changed<ComputedNode>)>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(stage) = stage else { return };
    for n in &pictures {
        let size = n.size().as_uvec2().clamp(UVec2::ONE, UVec2::splat(2048));
        if images.get(&stage.image).is_some_and(|i| i.size() != size)
            && let Some(mut image) = images.get_mut(&stage.image)
        {
            image.resize(bevy::render::render_resource::Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            });
        }
    }
}

/// The list or palette opens beside the card, level with its control; without room there, under the control
/// (over it where there is no room below).
fn place_pop(
    picking: Res<Picking>,
    anchors: Query<(&Anchor, &ComputedNode, &UiGlobalTransform)>,
    card: Single<(&ComputedNode, &UiGlobalTransform), With<WardrobeCard>>,
    root: Single<&ComputedNode, With<UiRoot>>,
    mut pop: Single<(&ComputedNode, PopParts), With<PickerPop>>,
) {
    let Some(pick) = picking.0 else { return };
    let Some((_, a, at)) = anchors.iter().find(|(a, ..)| a.0 == pick) else {
        return;
    };
    let (own, (node, vis)) = &mut *pop;
    let rect = |n: &ComputedNode, t: &UiGlobalTransform| {
        let k = n.inverse_scale_factor();
        let (centre, size) = (t.translation * k, n.size() * k);
        (centre - size / 2.0, centre + size / 2.0)
    };
    let (lo, hi) = rect(a, at);
    let (_, card_hi) = rect(card.0, card.1);
    let screen = root.size() * root.inverse_scale_factor();
    let me = own.size() * own.inverse_scale_factor();
    if me.x <= 0.0 {
        return;
    }
    const GAP: f32 = 10.0;
    const EDGE: f32 = 10.0;
    let fit_y = |y: f32| y.min(screen.y - EDGE - me.y).max(EDGE);
    let (left, top) = if card_hi.x + GAP + me.x <= screen.x - EDGE {
        (card_hi.x + GAP, fit_y(lo.y - 14.0))
    } else {
        let below = hi.y + 6.0;
        let top = if below + me.y > screen.y - EDGE && lo.y - 6.0 - me.y > EDGE {
            lo.y - 6.0 - me.y
        } else {
            fit_y(below)
        };
        (lo.x.min(screen.x - EDGE - me.x).max(EDGE), top)
    };
    // (Moved: shown from the next frame, where the layout has it in place. Its height may change by a pixel's
    // rounding with where it is: a move that small is no move.)
    let near = |v: Val, x: f32| matches!(v, Val::Px(p) if (p - x).abs() < 2.0);
    if near(node.left, left) && near(node.top, top) {
        vis.set_if_neq(Visibility::Inherited);
    } else {
        node.left = px(left.round());
        node.top = px(top.round());
    }
}

/// Lists and palettes open and close with their controls; a choice closes them.
fn picks(mut actions: MessageReader<UiAction>, mut picking: ResMut<Picking>) {
    for UiAction(a) in actions.read() {
        match a {
            Action::Pick(p) => picking.0 = (picking.0 != Some(*p)).then_some(*p),
            Action::Color(_) | Action::Wear(_) | Action::RandomOutfit if picking.0.is_some() => picking.0 = None,
            _ => {}
        }
    }
}

/// A press elsewhere closes the list or palette, as does the menu closing.
fn close_outside(
    mouse: Res<ButtonInput<MouseButton>>,
    ui: Res<Ui>,
    mut picking: ResMut<Picking>,
    pop: Single<&Hovered, With<PickerPop>>,
    anchors: Query<(&Anchor, &Hovered)>,
) {
    let Some(open) = picking.0 else { return };
    let away =
        mouse.just_pressed(MouseButton::Left) && !pop.get() && !anchors.iter().any(|(a, h)| a.0 == open && h.get());
    if away || !ui.menu {
        picking.0 = None;
    }
}

/// Esc closes the open list or palette, not the menu.
fn esc_closes(ui: Res<Ui>, mut keys: ResMut<ButtonInput<KeyCode>>, mut picking: ResMut<Picking>) {
    if ui.menu && picking.0.is_some() && keys.just_pressed(KeyCode::Escape) {
        keys.clear_just_pressed(KeyCode::Escape);
        picking.0 = None;
    }
}

/// What the bean on the stage wears: the player's look, or the choice under the pointer.
fn showcase(
    ui: Res<Ui>,
    me: Me,
    session: Res<Session>,
    views: Query<Entity, With<StageView>>,
    nodes: Query<(&Node, Option<&ChildOf>)>,
    tries: Query<(&Act, &Hovered, Has<InteractionDisabled>), With<TryOn>>,
    mut shown: ResMut<Showcase>,
) {
    let on = ui.menu && views.iter().any(|e| !hidden(e, &nodes));
    let mut s = Showcase {
        on,
        color: own_suit(&me, &session),
        outfit: me.player.outfit(),
    };
    for (act, hovered, disabled) in &tries {
        if !hovered.get() || disabled {
            continue;
        }
        match &act.0 {
            Action::Color(i) => s.color = *i,
            Action::Wear(o) => s.outfit = *o,
            _ => {}
        }
    }
    shown.set_if_neq(s);
}
