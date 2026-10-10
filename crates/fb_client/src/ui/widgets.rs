//! The building blocks of every screen: rich text, buttons, switches, fields, sliders, cards, folds and badges.
use super::*;

/// Splits text into runs of the game's font and of the emoji font (Manrope has no emoji or symbols).
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

/// A title or a big number, in the bold face.
pub fn big(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, size: f32, color: Color) -> Entity {
    rich_in(p, f, s, size, color, true)
}

/// Text whose root and every run carry `pick`.
pub(super) fn rich_with(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    s: &str,
    size: f32,
    color: Color,
    black: bool,
    pick: Option<Pickable>,
) -> Entity {
    let font = TextFont {
        font: if black { f.strong.clone() } else { f.body.clone() }.into(),
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

type RichText = (
    Entity,
    Ref<'static, Rich>,
    &'static TextFont,
    &'static TextColor,
    Option<&'static Pickable>,
);

/// One text of parts in their own weight and ink (bold names in a line): it wraps as a whole.
pub fn line_of(p: &mut ChildSpawnerCommands, f: &Fonts, parts: &[(&str, bool, Color)], size: f32) -> Entity {
    p.spawn((Text::default(), TextLayout::default(), Pickable::IGNORE))
        .with_children(|t| {
            for &(s, strong, ink) in parts {
                let font = TextFont {
                    font: if strong { f.strong.clone() } else { f.body.clone() }.into(),
                    font_size: FontSize::Px(size),
                    ..default()
                };
                spans(t, f, s, &font, TextColor(ink), Some(Pickable::IGNORE));
            }
        })
        .id()
}

/// A `Rich` text set to something else gets its spans anew.
pub(super) fn respan(texts: Query<RichText, Changed<Rich>>, f: Res<Fonts>, mut commands: Commands) {
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

/// The `Rich` texts, and the nodes they are inside of.
#[derive(SystemParam)]
pub struct Labels<'w, 's> {
    pub children: Query<'w, 's, &'static Children>,
    pub texts: Query<'w, 's, &'static mut Rich>,
}

impl Labels<'_, '_> {
    /// Sets the first `Rich` text inside `e` (a button's label).
    pub fn set(&mut self, e: Entity, s: &str) {
        if let Some(c) = self.children.iter_descendants(e).find(|c| self.texts.contains(*c))
            && let Ok(mut t) = self.texts.get_mut(c)
        {
            t.set(s);
        }
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
    rich(p, f, s, 16.0, INK)
}

pub fn muted(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich(p, f, s, 14.0, FAINT)
}

/// A card's heading.
pub fn heading(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich_in(p, f, s, 21.0, INK, true)
}

/// A part's heading inside a card.
pub fn subheading(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich_in(p, f, s, 17.0, INK, true)
}

/// A small label over a part of a card.
pub fn caption(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich_in(p, f, s, 13.0, FAINT, true)
}

/// A bean: the game's hero as an icon, its eyes and smile on a rounded body `h` high (px).
pub fn bean(p: &mut ChildSpawnerCommands, c: Color, h: f32) -> Entity {
    let w = (h * 0.76).round();
    let k = w / 38.0;
    let eye = |x: f32| Node {
        position_type: PositionType::Absolute,
        left: px((x - 2.4) * k),
        top: px(13.6 * k),
        width: px(4.8 * k),
        height: px(4.8 * k),
        border_radius: BorderRadius::MAX,
        ..default()
    };
    p.spawn((
        Node {
            width: px(w),
            height: px(h),
            border: UiRect::all(px((1.5 * k).max(1.0))),
            border_radius: BorderRadius::all(px(w / 2.0)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(c),
        BorderColor::all(Color::srgba(0.235, 0.157, 0.078, 0.1)),
        Pickable::IGNORE,
    ))
    .with_children(|b| {
        for x in [14.0, 24.0] {
            b.spawn((eye(x), BackgroundColor(INK), Pickable::IGNORE));
        }
        b.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(15.5 * k),
                top: px(22.0 * k),
                width: px(7.0 * k),
                height: px((1.8 * k).max(1.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(INK.with_alpha(0.7)),
            Pickable::IGNORE,
        ));
    })
    .id()
}

/// The game's name beside a bean.
pub fn logo(p: &mut ChildSpawnerCommands, f: &Fonts, size: f32) -> Entity {
    p.spawn(Node {
        column_gap: px(size * 0.45),
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(|l| {
        bean(l, APRICOT, size * 1.6);
        rich_in(l, f, text::LOGO, size, INK, true);
    })
    .id()
}

pub fn button(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, look: Look, act: Action) -> Entity {
    button_if(p, f, s, look, act, true)
}

pub fn button_if(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, look: Look, act: Action, enabled: bool) -> Entity {
    let (bg, ink) = if enabled { look.fill() } else { look.off() };
    // (Labels of switches wrap; other buttons keep their size.)
    let row_ = matches!(look, Look::Check(_) | Look::Toggle(_));
    let shrink = if row_ { 1.0 } else { 0.0 };
    let face = (
        Node {
            flex_grow: 1.0,
            flex_shrink: shrink,
            padding: look.padding(),
            border: look.border(),
            border_radius: BorderRadius::all(look.radius()),
            justify_content: match look {
                Look::Check(_) => JustifyContent::SpaceBetween,
                Look::Toggle(_) | Look::Nav(_) => JustifyContent::FlexStart,
                _ => JustifyContent::Center,
            },
            align_items: AlignItems::Center,
            column_gap: px(if row_ { 14 } else { 8 }),
            ..default()
        },
        BackgroundColor(bg),
        look.rim(),
    );
    button_shell(p, look, act, enabled, shrink, face, |b| {
        if let Look::Toggle(on) = look {
            switch(b, on);
        }
        // (Picking hits text by its runs: a run without IGNORE would take the release from the button.)
        if !s.is_empty() {
            let t = rich_with(b, f, s, look.size(), ink, look.strong(), Some(Pickable::IGNORE));
            if row_ {
                b.commands().entity(t).insert(Node {
                    flex_shrink: 1.0,
                    ..default()
                });
            }
        }
        if let Look::Check(on) = look {
            switch(b, on);
        }
    })
}

/// A switch's track, and the knob that slides along it.
#[derive(Component)]
pub(super) struct CheckBox;
#[derive(Component)]
pub(super) struct CheckDot;

fn switch(b: &mut ChildSpawnerCommands, on: bool) {
    b.spawn((
        CheckBox,
        Node {
            width: px(46),
            height: px(28),
            padding: UiRect::all(px(3)),
            border_radius: BorderRadius::MAX,
            justify_content: if on {
                JustifyContent::FlexEnd
            } else {
                JustifyContent::FlexStart
            },
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(switch_track(on)),
        Pickable::IGNORE,
    ))
    .with_children(|c| {
        c.spawn((
            CheckDot,
            Node {
                width: px(22),
                height: px(22),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(ON_FILL),
            BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.18), px(0), px(1), px(0), px(3)),
            Pickable::IGNORE,
        ));
    });
}

fn switch_track(on: bool) -> Color {
    if on { TEAL } else { hex(0xD6CDBF) }
}

/// A colour swatch button (`size` in px).
pub fn swatch(p: &mut ChildSpawnerCommands, color: Color, on: bool, act: Action, enabled: bool, size: f32) -> Entity {
    let look = Look::Swatch(color, on);
    let face = (
        Node {
            width: px(size),
            height: px(size),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(color),
        BorderColor::all(ink_wash(0.08)),
        look.outline(),
    );
    button_shell(p, look, act, enabled, 0.0, face, |_| {})
}

/// The "as designed" swatch: no colour of its own, a dashed ring in it.
pub fn swatch_none(p: &mut ChildSpawnerCommands, on: bool, act: Action, size: f32) -> Entity {
    let look = Look::Swatch(CARD, on);
    let face = (
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
        look.outline(),
    );
    button_shell(p, look, act, true, 0.0, face, |b| {
        b.spawn((
            Node {
                width: px(size - 16.0),
                height: px(size - 16.0),
                border: UiRect::all(px(1.5)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BorderColor::all(GHOST),
            Pickable::IGNORE,
        ));
    })
}

/// A map's tile: its title over its genre, in the genre's colours.
pub fn tile(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    title: &str,
    genre: fb_shared::game::Genre,
    act: Action,
) -> Entity {
    let look = Look::Tile(genre);
    let face = (
        Node {
            width: percent(100),
            min_height: px(58),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::SpaceBetween,
            row_gap: px(4),
            padding: look.padding(),
            border_radius: BorderRadius::all(look.radius()),
            ..default()
        },
        BackgroundColor(look.fill().0),
    );
    let e = button_shell(p, look, act, true, 0.0, face, |b| {
        rich_with(b, f, title, 14.0, INK, true, Some(Pickable::IGNORE));
        rich_with(
            b,
            f,
            text::genre(genre),
            12.0,
            genre_tones(genre).2,
            true,
            Some(Pickable::IGNORE),
        );
    });
    p.commands().entity(e).insert(Node {
        flex_shrink: 0.0,
        ..default()
    });
    e
}

/// What of a button moves when it is hovered or pressed. The button itself stays put: pressed at its edge,
/// it would otherwise slip from under the pointer, and the release would miss it.
#[derive(Component)]
pub(super) struct Face(pub(super) Entity);

pub(super) fn button_shell(
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

type Track = (&'static mut Node, &'static mut BackgroundColor);
type Ringed = (&'static mut Node, &'static mut BorderColor, &'static mut Outline);
type Rimmed = (&'static mut Node, &'static mut BorderColor);
type Restyled = (Ref<'static, Look>, &'static Face, Has<InteractionDisabled>);

/// A button whose `Look` was set anew is redrawn: its rim, ring, shadow, ink and switch.
pub(super) fn restyle(
    buttons: Query<Restyled, Changed<Look>>,
    mut faces: Query<Ringed, Without<CheckBox>>,
    mut rims: Query<Rimmed, (Without<CheckBox>, Without<Outline>)>,
    children: Query<&Children>,
    mut tracks: Query<Track, With<CheckBox>>,
    mut inks: Query<&mut TextColor>,
    mut commands: Commands,
) {
    for (look, face, disabled) in &buttons {
        if look.is_added() {
            continue;
        }
        let look = *look;
        if let Ok((_, _, mut ring)) = faces.get_mut(face.0) {
            ring.set_if_neq(look.outline());
        } else if let Ok((mut node, mut rim)) = rims.get_mut(face.0) {
            if node.border != look.border() {
                node.border = look.border();
            }
            rim.set_if_neq(look.rim());
        }
        match look.shadow() {
            Some(s) => commands.entity(face.0).insert(s),
            None => commands.entity(face.0).remove::<BoxShadow>(),
        };
        let ink = if disabled { look.off().1 } else { look.fill().1 };
        for c in children.iter_descendants(face.0) {
            if let Ok(mut t) = inks.get_mut(c) {
                t.set_if_neq(TextColor(ink));
            }
            if let Look::Check(on) | Look::Toggle(on) = look
                && let Ok((mut node, mut fill)) = tracks.get_mut(c)
            {
                fill.set_if_neq(BackgroundColor(switch_track(on)));
                let j = if on {
                    JustifyContent::FlexEnd
                } else {
                    JustifyContent::FlexStart
                };
                if node.justify_content != j {
                    node.justify_content = j;
                }
            }
        }
    }
}

const FIELD_RIM: Color = LINE;
const FIELD_FOCUS: Color = hex(0x9DBDB4);

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
            padding: UiRect::axes(px(14), px(11)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
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
            font: f.body.clone().into(),
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(INK),
        TextCursorStyle {
            color: TEAL,
            ..default()
        },
        BackgroundColor(Color::WHITE),
        BorderColor::all(FIELD_RIM),
        Outline::new(px(0), px(0), Color::NONE),
    ));
    if let Some(filter) = filter {
        e.insert(EditableTextFilter::new(filter));
    }
    e.id()
}

/// The field with the keyboard is ringed in the accent.
pub(super) fn focus_ring(
    focus: Res<InputFocus>,
    mut fields: Query<(Entity, &mut BorderColor, &mut Outline), With<Field>>,
) {
    for (e, mut rim, mut ring) in &mut fields {
        let on = focus.get() == Some(e);
        rim.set_if_neq(BorderColor::all(if on { FIELD_FOCUS } else { FIELD_RIM }));
        ring.set_if_neq(if on {
            Outline::new(px(3), px(0), TEAL_SOFT)
        } else {
            Outline::new(px(0), px(0), Color::NONE)
        });
    }
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
                left: px(15),
                top: px(11),
                ..default()
            },
            Pickable::IGNORE,
            Text::new(hint),
            TextFont {
                font: f.body.clone().into(),
                font_size: FontSize::Px(16.0),
                ..default()
            },
            TextColor(hex(0x8A8075)),
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

/// The lit part of a slider's rail, up to its thumb.
#[derive(Component)]
pub struct SliderFill;

/// A setting's row with a slider: its label, the rail lit up to the thumb, and the value.
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
    let at = percent(range.thumb_position(value) * 100.0);
    const THUMB: f32 = 22.0;
    setting_row(p, |r| {
        r.spawn(Node {
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_width: px(0),
            ..default()
        })
        .with_children(|l| {
            label(l, f, label_s);
        });
        r.spawn((
            knob,
            Slider::default(),
            SliderValue(value),
            range,
            SliderStep(step),
            Hovered::default(),
            Node {
                width: px(300),
                max_width: percent(45),
                height: px(28),
                justify_content: JustifyContent::Center,
                flex_direction: FlexDirection::Column,
                flex_shrink: 1.0,
                ..default()
            },
        ))
        .with_children(|s| {
            // (The rail runs between the thumb's centres at both ends, as the slider maps the pointer.)
            s.spawn(Node {
                margin: UiRect::axes(px(THUMB / 2.0), px(0)),
                height: px(THUMB),
                justify_content: JustifyContent::Center,
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|rail| {
                rail.spawn((
                    Node {
                        height: px(6),
                        border_radius: BorderRadius::MAX,
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(WELL),
                ))
                .with_children(|t| {
                    t.spawn((
                        SliderFill,
                        Node {
                            width: at,
                            height: percent(100),
                            ..default()
                        },
                        BackgroundColor(TEAL),
                    ));
                });
                rail.spawn((
                    SliderThumb,
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(THUMB),
                        height: px(THUMB),
                        left: at,
                        border: UiRect::all(px(2)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    UiTransform::from_translation(Val2::percent(-50.0, 0.0)),
                    BackgroundColor(Color::WHITE),
                    BorderColor::all(TEAL),
                    BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.22), px(0), px(1), px(0), px(4)),
                ));
            });
        });
        r.spawn(Node {
            width: px(52),
            justify_content: JustifyContent::FlexEnd,
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|v| {
            v.spawn((
                KnobValue(knob),
                Text::new(knob_text(knob, value)),
                TextFont {
                    font: f.strong.clone().into(),
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(MUTED),
            ));
        });
    });
}

/// A row of the settings: what it sets, and how, over a rule.
pub fn setting_row(p: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn((
        Node {
            align_items: AlignItems::Center,
            column_gap: px(20),
            min_height: px(56),
            border: UiRect::bottom(px(1)),
            flex_shrink: 0.0,
            ..default()
        },
        BorderColor::all(HAIR),
    ))
    .with_children(f)
    .id()
}

/// The value shown beside a slider.
#[derive(Component)]
pub struct KnobValue(pub Knob);

pub fn knob_text(knob: Knob, v: f32) -> String {
    match knob {
        Knob::MouseSens | Knob::StickSens | Knob::UiScale => format!("{v:.2}").replace('.', ","),
        Knob::Fov => format!("{v:.0}"),
        Knob::Volume => format!("{:.0}%", v * 100.0),
    }
}

/// A card: the light rounded surface every part of the interface sits on.
pub fn card<'a>(p: &'a mut ChildSpawnerCommands, node: Node) -> EntityCommands<'a> {
    p.spawn((
        Node {
            border_radius: BorderRadius::all(px(20)),
            ..node
        },
        BackgroundColor(CARD),
        BoxShadow::new(SHADOW.with_alpha(0.1), px(0), px(2), px(0), px(10)),
    ))
}

/// A card of controls under its heading.
pub fn section(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    title: &str,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    card(
        p,
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(12),
            padding: UiRect::axes(px(24), px(22)),
            flex_shrink: 0.0,
            ..default()
        },
    )
    .with_children(|c| {
        heading(c, f, title);
        body(c);
    })
    .id()
}

/// A part of a card, inside a thin rim.
pub fn group(p: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(12),
            padding: UiRect::axes(px(18), px(16)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(18)),
            ..default()
        },
        BorderColor::all(hex(0xECE4D8)),
    ))
    .with_children(f)
    .id()
}

pub fn row(p: &mut ChildSpawnerCommands, wrap: bool, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: if wrap { FlexWrap::Wrap } else { FlexWrap::NoWrap },
        column_gap: px(8),
        row_gap: px(8),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(f)
    .id()
}

pub fn stack(p: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: px(10),
        ..default()
    })
    .with_children(f)
    .id()
}

/// Room to the right: what follows goes to the row's end.
pub fn spacer(p: &mut ChildSpawnerCommands) {
    p.spawn(Node {
        flex_grow: 1.0,
        ..default()
    });
}

/// A colour dot (a player's suit).
pub fn dot(p: &mut ChildSpawnerCommands, color: Color, size: f32) -> Entity {
    p.spawn((
        Node {
            width: rem(size),
            height: rem(size),
            border_radius: BorderRadius::MAX,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(color),
    ))
    .id()
}

/// A player's round avatar: their suit colour with their initials.
pub fn avatar(p: &mut ChildSpawnerCommands, f: &Fonts, name: &str, color: Color, size: f32) -> Entity {
    p.spawn((
        Node {
            width: px(size),
            height: px(size),
            border_radius: BorderRadius::MAX,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(color),
    ))
    .with_children(|a| {
        rich_in(a, f, &initials(name), size * 0.34, INK, true);
    })
    .id()
}

/// «Mr_Bean» → «MB», «Тыковка» → «ТЫ».
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '_' || c == '.')
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .collect();
    let s: String = if words.len() > 1 {
        words
            .iter()
            .take(2)
            .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
            .collect()
    } else {
        name.chars().filter(|c| c.is_alphanumeric()).take(2).collect()
    };
    if s.is_empty() { "—".into() } else { s.to_uppercase() }
}

/// A small pill of colour with a word in it.
pub fn badge(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, fill: Color, ink: Color) -> Entity {
    p.spawn((
        Node {
            column_gap: px(6),
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(10), px(3)),
            border_radius: BorderRadius::MAX,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(fill),
    ))
    .with_children(|b| {
        rich_in(b, f, s, 13.0, ink, true);
    })
    .id()
}

/// A genre's tag: its name or the round, on its soft colour.
pub fn genre_tag(p: &mut ChildSpawnerCommands, f: &Fonts, g: Option<fb_shared::game::Genre>, s: &str) -> Entity {
    let (fill, ink) = g.map_or((CARD2, MUTED), |g| {
        let t = genre_tones(g);
        (t.1, t.2)
    });
    p.spawn((
        Node {
            padding: UiRect::axes(px(9), px(3)),
            border_radius: BorderRadius::all(px(8)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(fill),
    ))
    .with_children(|b| {
        rich_in(b, f, s, 13.0, ink, true);
    })
    .id()
}

/// A key as it is printed on a keyboard.
pub fn keycap(p: &mut ChildSpawnerCommands, f: &Fonts, key: &str) -> Entity {
    p.spawn((
        Node {
            min_width: px(30),
            height: px(30),
            padding: UiRect::axes(px(8), px(0)),
            border: UiRect::new(px(1), px(1), px(1), px(2)),
            border_radius: BorderRadius::all(px(9)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(CARD),
        BorderColor::all(LINE),
    ))
    .with_children(|k| {
        rich_in(k, f, key, 14.0, INK, true);
    })
    .id()
}

/// How far a bar is filled (0–1), in `color`.
pub fn meter(p: &mut ChildSpawnerCommands, frac: f32, color: Color, width: Val) -> Entity {
    p.spawn((
        Node {
            width,
            height: px(6),
            border_radius: BorderRadius::MAX,
            overflow: Overflow::clip(),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(WELL),
    ))
    .with_children(|m| {
        m.spawn((
            Node {
                width: percent(frac.clamp(0.0, 1.0) * 100.0),
                height: percent(100),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(color),
        ));
    })
    .id()
}

/// A ring turning while something is on its way (`size` in px).
pub fn spinner(p: &mut ChildSpawnerCommands, size: f32, color: Color) -> Entity {
    p.spawn((
        Node {
            width: px(size),
            height: px(size),
            border: UiRect::all(px((size / 8.0).clamp(2.0, 3.5))),
            border_radius: BorderRadius::MAX,
            flex_shrink: 0.0,
            ..default()
        },
        BorderColor {
            top: color,
            ..BorderColor::all(TEAL_SOFT)
        },
        super::motion::Spin,
    ))
    .id()
}

/// A folded part: its head (a title, a note, the arrow) toggles it; the body is built folded or not and shown when
/// open (`sync_folds`).
pub fn fold(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    folds: &Folds,
    key: Fold,
    title: &str,
    note: &str,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) {
    let open = folds.open(key);
    let look = Look::Fold;
    let face = (
        Node {
            flex_grow: 1.0,
            padding: look.padding(),
            align_items: AlignItems::Center,
            column_gap: px(10),
            ..default()
        },
        BackgroundColor(Color::NONE),
    );
    let head = button_shell(p, look, Action::Fold(key), true, 0.0, face, |b| {
        let t = rich_with(b, f, title, 17.0, INK, true, Some(Pickable::IGNORE));
        b.commands().entity(t).insert(Node {
            flex_grow: 1.0,
            ..default()
        });
        if !note.is_empty() {
            let n = rich_with(b, f, note, 14.0, FAINT, false, Some(Pickable::IGNORE));
            b.commands().entity(n).insert((
                TextLayout::no_wrap(),
                Node {
                    flex_shrink: 0.0,
                    ..default()
                },
            ));
        }
        b.spawn((
            FoldChevron,
            Text::new("›"),
            TextFont {
                font: f.strong.clone().into(),
                font_size: FontSize::Px(20.0),
                ..default()
            },
            TextColor(FAINT),
            UiTransform::from_rotation(chevron(open)),
            Pickable::IGNORE,
        ));
    });
    p.commands().entity(head).insert(Node {
        flex_shrink: 0.0,
        ..default()
    });
    p.spawn((
        FoldBody(key),
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            margin: UiRect::top(px(12)),
            display: display(open),
            ..default()
        },
        super::motion::Reveal::new(super::motion::Motion::slide(0.0, -8.0)),
    ))
    .with_children(body);
}

fn chevron(open: bool) -> Rot2 {
    Rot2::degrees(if open { -90.0 } else { 90.0 })
}

/// The arrow at the end of a fold's head: down while folded, up while open.
#[derive(Component)]
pub(super) struct FoldChevron;

#[derive(Component)]
pub(super) struct FoldBody(Fold);

pub(super) fn sync_folds(
    folds: Res<Folds>,
    mut bodies: Query<(&FoldBody, &mut Node)>,
    heads: Query<(&Act, &Face)>,
    children: Query<&Children>,
    mut chevrons: Query<&mut UiTransform, With<FoldChevron>>,
) {
    for (b, mut node) in &mut bodies {
        show(&mut node, folds.open(b.0));
    }
    for (act, face) in &heads {
        let Action::Fold(k) = act.0 else { continue };
        let turn = chevron(folds.open(k));
        for c in children.iter_descendants(face.0) {
            if let Ok(mut t) = chevrons.get_mut(c)
                && t.rotation != turn
            {
                t.rotation = turn;
            }
        }
    }
}

/// A part of the settings, shown while it is the one picked.
#[derive(Component)]
pub(super) struct SectionBody(pub(super) Section);

pub(super) fn sync_sections(
    part: Res<Section>,
    mut bodies: Query<(&SectionBody, &mut Node)>,
    mut navs: Query<(&Act, &mut Look)>,
) {
    for (b, mut node) in &mut bodies {
        show(&mut node, b.0 == *part);
    }
    for (act, mut look) in &mut navs {
        if let Action::Section(s) = act.0 {
            look.set_if_neq(Look::Nav(s == *part));
        }
    }
}

/// A card over the game: light, a little see-through, softly shadowed.
pub fn panel() -> (BackgroundColor, BoxShadow) {
    (
        BackgroundColor(PANEL),
        BoxShadow::new(SHADOW.with_alpha(0.1), px(0), px(2), px(0), px(10)),
    )
}

/// A big card over the game (a dialog, a board).
pub fn panel_raised() -> (BackgroundColor, BoxShadow) {
    (
        BackgroundColor(CARD),
        BoxShadow::new(SHADOW, px(0), px(6), px(0), px(24)),
    )
}
