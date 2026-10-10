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

/// Text in the display face (Unbounded), for titles and big numbers.
pub fn big(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, size: f32, color: Color) -> Entity {
    let font = TextFont {
        font: f.display.clone().into(),
        font_size: FontSize::Px(size),
        ..default()
    };
    p.spawn((Text::default(), font.clone(), TextColor(color), Rich(s.into())))
        .with_children(|t| spans(t, f, s, &font, TextColor(color), None))
        .id()
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
    rich(p, f, s, 14.5, INK)
}

pub fn muted(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich(p, f, s, 13.0, MUTED)
}

pub fn heading(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich_in(p, f, s, 15.0, INK, true)
}

/// A small capital label over a part of a panel.
pub fn caption(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich_in(p, f, &s.to_uppercase(), 11.0, FAINT, true)
}

pub fn logo(p: &mut ChildSpawnerCommands, f: &Fonts, size: f32) -> Entity {
    p.spawn((
        Text::new(text::LOGO),
        TextFont {
            font: f.display.clone().into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(BLUE),
        TextShadow {
            offset: Vec2::new(size / 18.0, size / 14.0),
            color: BLUE_DEEP,
        },
    ))
    .id()
}

pub fn button(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, look: Look, act: Action) -> Entity {
    button_if(p, f, s, look, act, true)
}

pub fn button_if(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, look: Look, act: Action, enabled: bool) -> Entity {
    let (bg, ink) = look.fill();
    // (Labels of switches wrap; other buttons keep their size.)
    let shrink = if matches!(look, Look::Check(_)) { 1.0 } else { 0.0 };
    let face = (
        Node {
            flex_grow: 1.0,
            flex_shrink: shrink,
            padding: look.padding(),
            border: look.border(),
            border_radius: BorderRadius::all(look.radius()),
            justify_content: match look {
                Look::Check(_) | Look::Fold => JustifyContent::SpaceBetween,
                _ => JustifyContent::Center,
            },
            align_items: AlignItems::Center,
            column_gap: rem(0.625),
            ..default()
        },
        BackgroundColor(bg),
        look.rim(),
    );
    button_shell(p, look, act, enabled, shrink, face, |b| {
        // (Picking hits text by its runs: a run without IGNORE would take the release from the button.)
        if !s.is_empty() {
            let t = rich_with(b, f, s, look.size(), ink, look.strong(), Some(Pickable::IGNORE));
            if matches!(look, Look::Check(_)) {
                b.commands().entity(t).insert(Node {
                    flex_shrink: 1.0,
                    ..default()
                });
            }
        }
        match look {
            Look::Check(on) => switch(b, on),
            Look::Fold => {
                b.spawn((
                    FoldChevron,
                    Text::new("›"),
                    TextFont {
                        font: f.strong.clone().into(),
                        font_size: FontSize::Px(20.0),
                        ..default()
                    },
                    TextColor(MUTED),
                    Pickable::IGNORE,
                ));
            }
            _ => {}
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
            width: rem(2.5),
            height: rem(1.375),
            padding: UiRect::all(px(3)),
            border: UiRect::all(px(1)),
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
        switch_track(on),
        Pickable::IGNORE,
    ))
    .with_children(|c| {
        c.spawn((
            CheckDot,
            Node {
                width: rem(0.9375),
                height: rem(0.9375),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(if on { ON_FILL } else { SURFACE }),
            BoxShadow::new(SHADOW.with_alpha(0.4), px(0), px(1), px(0), px(3)),
            Pickable::IGNORE,
        ));
    });
}

fn switch_track(on: bool) -> (BackgroundColor, BorderColor) {
    if on {
        (BackgroundColor(BLUE), BorderColor::all(BLUE))
    } else {
        (BackgroundColor(ink_wash(0.12)), BorderColor::all(Color::NONE))
    }
}

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

/// A map's tile: its genre over its title, with the genre's colour along the top.
pub fn tile(p: &mut ChildSpawnerCommands, f: &Fonts, title: &str, sub: &str, color: Color, act: Action) -> Entity {
    let look = Look::Tile(color);
    let face = (
        Node {
            width: rem(9.25),
            flex_direction: FlexDirection::Column,
            row_gap: rem(0.125),
            padding: look.padding(),
            border: look.border(),
            border_radius: BorderRadius::all(look.radius()),
            ..default()
        },
        BackgroundColor(look.fill().0),
        look.rim(),
    );
    button_shell(p, look, act, true, 0.0, face, |b| {
        rich_with(b, f, &sub.to_uppercase(), 10.0, MUTED, true, Some(Pickable::IGNORE));
        rich_with(b, f, title, 14.0, INK, true, Some(Pickable::IGNORE));
    })
}

/// What of a button moves when it is hovered or pressed. The button itself stays put: pressed at its edge,
/// it would otherwise slip from under the pointer, and the release would miss it.
#[derive(Component)]
pub(super) struct Face(pub(super) Entity);

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
        if let Some(g) = look.gradient(!enabled) {
            fe.insert(g);
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

type Track = (
    &'static mut Node,
    &'static mut BorderColor,
    &'static mut BackgroundColor,
);

/// A button whose `Look` was set anew is redrawn: its rim, shadow, gradient, ink and switch.
pub(super) fn restyle(
    buttons: Query<(Ref<Look>, &Face), Changed<Look>>,
    mut faces: Query<(&mut Node, &mut BorderColor), Without<CheckBox>>,
    children: Query<&Children>,
    mut tracks: Query<Track, With<CheckBox>>,
    mut knobs: Query<&mut BackgroundColor, (With<CheckDot>, Without<CheckBox>)>,
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
        match look.gradient(false) {
            Some(g) => commands.entity(face.0).insert(g),
            None => commands.entity(face.0).remove::<BackgroundGradient>(),
        };
        let ink = look.fill().1;
        for c in children.iter_descendants(face.0) {
            if let Ok(mut t) = inks.get_mut(c) {
                t.set_if_neq(TextColor(ink));
            }
            if let Look::Check(on) = look {
                if let Ok((mut node, mut rim, mut fill)) = tracks.get_mut(c) {
                    let (bg, border) = switch_track(on);
                    *rim = border;
                    fill.set_if_neq(bg);
                    let j = if on {
                        JustifyContent::FlexEnd
                    } else {
                        JustifyContent::FlexStart
                    };
                    if node.justify_content != j {
                        node.justify_content = j;
                    }
                }
                if let Ok(mut k) = knobs.get_mut(c) {
                    k.set_if_neq(BackgroundColor(if on { ON_FILL } else { SURFACE }));
                }
            }
        }
    }
}

const FIELD_RIM: Color = ink_wash(0.16);

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
            padding: UiRect::axes(rem(0.875), rem(0.625)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(0.875)),
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
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(INK),
        TextCursorStyle {
            color: BLUE,
            ..default()
        },
        BackgroundColor(SURFACE),
        BorderColor::all(FIELD_RIM),
    ));
    if let Some(filter) = filter {
        e.insert(EditableTextFilter::new(filter));
    }
    e.id()
}

/// The field with the keyboard is ringed in the accent.
pub(super) fn focus_ring(focus: Res<InputFocus>, mut fields: Query<(Entity, &mut BorderColor), With<Field>>) {
    for (e, mut rim) in &mut fields {
        let want = BorderColor::all(if focus.get() == Some(e) { BLUE } else { FIELD_RIM });
        if *rim != want {
            *rim = want;
        }
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
                left: rem(0.9375),
                top: rem(0.625),
                ..default()
            },
            Pickable::IGNORE,
            Text::new(hint),
            TextFont {
                font: f.body.clone().into(),
                font_size: FontSize::Px(15.0),
                ..default()
            },
            TextColor(FAINT),
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

/// A slider of a setting: its label and value over a rail lit up to the thumb.
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
    const THUMB: f32 = 1.125;
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: rem(0.375),
        ..default()
    })
    .with_children(|c| {
        c.spawn(Node {
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|r| {
            label(r, f, label_s);
            r.spawn((
                Node {
                    padding: UiRect::axes(rem(0.5), px(1)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(ink_wash(0.07)),
            ))
            .with_children(|v| {
                v.spawn((
                    KnobValue(knob),
                    Text::new(knob_text(knob, value)),
                    TextFont {
                        font: f.strong.clone().into(),
                        font_size: FontSize::Px(12.5),
                        ..default()
                    },
                    TextColor(INK),
                ));
            });
        });
        c.spawn((
            knob,
            Slider::default(),
            SliderValue(value),
            range,
            SliderStep(step),
            Hovered::default(),
            Node {
                height: rem(THUMB),
                justify_content: JustifyContent::Center,
                flex_direction: FlexDirection::Column,
                ..default()
            },
        ))
        .with_children(|s| {
            // (The rail runs between the thumb's centres at both ends, as the slider maps the pointer.)
            s.spawn(Node {
                margin: UiRect::axes(rem(THUMB / 2.0), px(0)),
                height: rem(THUMB),
                justify_content: JustifyContent::Center,
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|rail| {
                rail.spawn((
                    Node {
                        height: rem(0.375),
                        border_radius: BorderRadius::MAX,
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(ink_wash(0.1)),
                ))
                .with_children(|t| {
                    t.spawn((
                        SliderFill,
                        Node {
                            width: at,
                            height: percent(100),
                            ..default()
                        },
                        BackgroundColor(BLUE),
                    ));
                });
                rail.spawn((
                    SliderThumb,
                    Node {
                        position_type: PositionType::Absolute,
                        width: rem(THUMB),
                        height: rem(THUMB),
                        left: at,
                        border: UiRect::all(px(3)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    UiTransform::from_translation(Val2::percent(-50.0, 0.0)),
                    BackgroundColor(SURFACE),
                    BorderColor::all(BLUE),
                    BoxShadow::new(SHADOW.with_alpha(0.2), px(0), px(1), px(0), px(4)),
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

/// A card of related controls.
pub fn group(p: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: rem(0.75),
            padding: UiRect::all(rem(1.0)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(1.125)),
            ..default()
        },
        BackgroundColor(GROUP),
        BorderColor::all(ink_wash(0.06)),
    ))
    .with_children(f)
    .id()
}

/// A card under its caption.
pub fn section(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    title: &str,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: rem(0.5),
        ..default()
    })
    .with_children(|c| {
        c.spawn(Node {
            padding: UiRect::left(rem(0.25)),
            ..default()
        })
        .with_children(|t| {
            caption(t, f, title);
        });
        group(c, body);
    })
    .id()
}

pub fn row(p: &mut ChildSpawnerCommands, wrap: bool, f: impl FnOnce(&mut ChildSpawnerCommands)) -> Entity {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: if wrap { FlexWrap::Wrap } else { FlexWrap::NoWrap },
        column_gap: rem(0.5),
        row_gap: rem(0.5),
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

/// A dot's rim: a light line round a dark colour, a dark one round a colour too pale to show on its own.
pub fn dot_rim(c: Color) -> Color {
    let rim = if c.luminance() > 0.8 {
        ink_wash(0.35)
    } else {
        Color::WHITE
    };
    rim.with_alpha(rim.alpha() * c.alpha())
}

/// Ink that reads on a fill of colour `c`.
pub fn ink_on(c: Color) -> Color {
    if c.luminance() > 0.45 { INK } else { ON_FILL }
}

/// A player's round avatar: their suit colour with their initial.
pub fn avatar(p: &mut ChildSpawnerCommands, f: &Fonts, name: &str, color: Color, size: f32) -> Entity {
    // (The last word's: "Бот Кекс" is К, not Б like every other bot.)
    let word = name.split_whitespace().last().unwrap_or(name);
    let initial: String = word
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".into(), |c| c.to_uppercase().collect());
    p.spawn((
        Node {
            width: rem(size),
            height: rem(size),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundGradient::from(LinearGradient::to_bottom(vec![
            color.lighter(0.08).into(),
            color.darker(0.12).into(),
        ])),
        BorderColor::all(Color::WHITE),
        BoxShadow::new(SHADOW.with_alpha(0.15), px(0), px(1), px(0), px(3)),
    ))
    .with_children(|a| {
        rich_in(a, f, &initial, size * 7.5, ink_on(color), true);
    })
    .id()
}

/// A small pill of colour with a word in it.
pub fn badge(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str, fill: Color, ink: Color) -> Entity {
    p.spawn((
        Node {
            padding: UiRect::axes(rem(0.5), px(2)),
            border_radius: BorderRadius::MAX,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(fill),
    ))
    .with_children(|b| {
        rich_in(b, f, s, 11.0, ink, true);
    })
    .id()
}

/// A key as it is printed on a keyboard; the entity of its text.
pub fn keycap(p: &mut ChildSpawnerCommands, f: &Fonts, key: &str) -> Entity {
    let mut t = Entity::PLACEHOLDER;
    p.spawn((
        Node {
            min_width: rem(1.625),
            padding: UiRect::new(rem(0.4375), rem(0.4375), px(1), px(2)),
            border: UiRect::new(px(1), px(1), px(1), px(3)),
            border_radius: BorderRadius::all(rem(0.375)),
            justify_content: JustifyContent::Center,
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(SURFACE),
        BorderColor::all(ink_wash(0.22)),
    ))
    .with_children(|k| {
        t = rich_in(k, f, key, 11.5, INK, true);
    });
    t
}

/// How far a bar is filled (0–1), in `color`.
pub fn meter(p: &mut ChildSpawnerCommands, frac: f32, color: Color, width: Val) -> Entity {
    p.spawn((
        Node {
            width,
            height: rem(0.3125),
            border_radius: BorderRadius::MAX,
            overflow: Overflow::clip(),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(ink_wash(0.1)),
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

/// A ring turning while something is on its way.
pub fn spinner(p: &mut ChildSpawnerCommands, size: f32, color: Color) -> Entity {
    p.spawn((
        Node {
            width: rem(size),
            height: rem(size),
            border: UiRect::all(px(3)),
            border_radius: BorderRadius::MAX,
            flex_shrink: 0.0,
            ..default()
        },
        BorderColor {
            top: color,
            right: color.with_alpha(0.5),
            bottom: color.with_alpha(0.15),
            left: color.with_alpha(0.15),
        },
        super::motion::Spin,
    ))
    .id()
}

/// A folded part: a card whose head toggles it; the body is built folded or not and shown when open
/// (`sync_folds`).
pub fn fold(
    p: &mut ChildSpawnerCommands,
    f: &Fonts,
    folds: &Folds,
    key: Fold,
    title: &'static str,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) {
    let open = folds.open(key);
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(1.0)),
            ..default()
        },
        BackgroundColor(GROUP),
        BorderColor::all(ink_wash(0.06)),
    ))
    .with_children(|c| {
        button(c, f, title, Look::Fold, Action::Fold(key));
        c.spawn((
            FoldBody(key),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: rem(0.75),
                padding: UiRect::new(rem(0.875), rem(0.875), rem(0.125), rem(0.875)),
                display: display(open),
                ..default()
            },
            super::motion::Reveal::new(super::motion::Motion::slide(0.0, -8.0)),
        ))
        .with_children(body);
    });
}

/// The arrow at the end of a fold's head: turned down while it is open.
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
        let turn = Rot2::degrees(if folds.open(k) { 90.0 } else { 0.0 });
        for c in children.iter_descendants(face.0) {
            if let Ok(mut t) = chevrons.get_mut(c)
                && t.rotation != turn
            {
                t.rotation = turn;
            }
        }
    }
}

/// A dark glass panel.
pub fn glass() -> (BackgroundColor, BorderColor, BoxShadow) {
    (
        BackgroundColor(PANEL),
        BorderColor::all(RIM),
        BoxShadow::new(SHADOW.with_alpha(0.5), px(0), rem(0.75), px(0), rem(2.25)),
    )
}

/// A glass panel with a coloured bar down its left side (its node's left border).
pub fn glass_bar(c: Color) -> (BackgroundColor, BorderColor, BoxShadow) {
    let (bg, _, shadow) = glass();
    (
        bg,
        BorderColor {
            left: c,
            ..BorderColor::all(RIM)
        },
        shadow,
    )
}
