//! The building blocks of every screen: rich text, buttons, fields, sliders, groups and folds.
use super::*;

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
        TextColor(ACCENT),
        TextShadow {
            offset: Vec2::new(0.0, size / 10.0),
            color: VIOLET,
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
        Look::Tab(_) | Look::Chip(_) | Look::Tiny | Look::TinyDanger => px(f32::MAX),
        _ => rem(0.75),
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
                BorderColor::all(if on { ACCENT } else { MUTED }),
                BackgroundColor(if on { ACCENT } else { Color::NONE }),
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
                    BackgroundColor(DARK),
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
pub(super) struct CheckBox;
#[derive(Component)]
pub(super) struct CheckDot;

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
pub(super) fn restyle(
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
                    *rim = BorderColor::all(if on { ACCENT } else { MUTED });
                    fill.set_if_neq(BackgroundColor(if on { ACCENT } else { Color::NONE }));
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
            color: ACCENT,
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.28)),
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.14)),
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
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.12)),
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
                    BackgroundColor(ACCENT),
                    BorderColor::all(DARK),
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
            padding: UiRect::all(rem(0.875)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(rem(1.0)),
            ..default()
        },
        BackgroundColor(GROUP),
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.06)),
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
        Color::srgba(0.0, 0.0, 0.0, 0.4)
    } else {
        Color::srgba(1.0, 1.0, 1.0, 0.45)
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
pub(super) struct FoldTitle(&'static str);

#[derive(Component)]
pub(super) struct FoldBody(Fold);

fn fold_title(title: &str, open: bool) -> String {
    format!("{} {title}", if open { "−" } else { "+" })
}

pub(super) fn sync_folds(
    folds: Res<Folds>,
    mut bodies: Query<(&FoldBody, &mut Node)>,
    titles: Query<(Entity, &Act, &FoldTitle)>,
    mut labels: Labels,
) {
    for (b, mut node) in &mut bodies {
        show(&mut node, folds.open(b.0));
    }
    for (e, act, title) in &titles {
        if let Action::Fold(k) = act.0 {
            labels.set(e, &fold_title(title.0, folds.open(k)));
        }
    }
}

/// A dark glass panel.
pub fn glass() -> (BackgroundColor, BorderColor, BoxShadow) {
    (
        BackgroundColor(PANEL),
        BorderColor::all(RIM),
        BoxShadow::new(SHADOW.with_alpha(0.45), px(0), rem(0.75), px(0), rem(2.0)),
    )
}

/// A glass panel with a coloured bar down its left side.
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

/// A small capital label over a part of a panel.
pub fn caption(p: &mut ChildSpawnerCommands, f: &Fonts, s: &str) -> Entity {
    rich_in(p, f, &s.to_uppercase(), 11.0, MUTED, true)
}
