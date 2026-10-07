//! The loading screen: over everything while the warm-up runs (`render/warmup.rs`), at the start and after a
//! change of the graphics. Opaque: what is built behind it is not seen, and it takes the clicks meant for it.
use bevy::input_focus::InputFocus;
use bevy::prelude::*;

use super::*;
use crate::render::warmup::Warmup;

/// The screen's backdrop: the game's pink sky.
const BACKDROP: Color = Color::srgb(1.0, 0.85, 0.95);

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build_screen);
        app.add_systems(Update, update_screen);
    }
}

#[derive(Component)]
struct Screen;

#[derive(Component)]
struct Bar;

#[derive(Component)]
struct StepLine;

fn build_screen(mut commands: Commands, f: Res<Fonts>, warm: Option<Res<Warmup>>) {
    let f = &*f;
    let on = warm.is_some_and(|w| w.busy());
    commands
        .spawn((
            Screen,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: rem(1.25),
                display: if on {
                    bevy::ui::Display::Flex
                } else {
                    bevy::ui::Display::None
                },
                ..default()
            },
            BackgroundColor(BACKDROP),
            // (Above every layer, the F4 overlay too.)
            GlobalZIndex(1000),
        ))
        .with_children(|s| {
            logo(s, f, 64.0);
            rich_in(s, f, text::LOADING, 24.0, INK, true);
            s.spawn((
                Node {
                    width: rem(22.0),
                    max_width: percent(80),
                    height: rem(0.75),
                    border_radius: BorderRadius::all(rem(0.375)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(GROUP),
            ))
            .with_children(|b| {
                b.spawn((
                    Bar,
                    Node {
                        width: percent(0),
                        height: percent(100),
                        border_radius: BorderRadius::all(rem(0.375)),
                        ..default()
                    },
                    BackgroundColor(PINK),
                ));
            });
            s.spawn((StepLine, Section::default(), Node::default()));
        });
}

/// Up while the warm-up runs, its bar and what it is doing; no text field has the keyboard meanwhile.
fn update_screen(
    warm: Option<Res<Warmup>>,
    mut screen: Query<&mut Node, (With<Screen>, Without<Bar>)>,
    mut bar: Query<&mut Node, (With<Bar>, Without<Screen>)>,
    mut line: Query<(Entity, &mut Section), With<StepLine>>,
    mut focus: ResMut<InputFocus>,
    f: Res<Fonts>,
    mut commands: Commands,
) {
    let busy = warm.as_ref().is_some_and(|w| w.busy());
    for mut n in &mut screen {
        super::show(&mut n, busy);
    }
    let Some(w) = warm.filter(|_| busy) else { return };
    if focus.get().is_some() {
        focus.clear();
    }
    let width = percent((w.progress() * 100.0).round());
    for mut n in &mut bar {
        if n.width != width {
            n.width = width;
        }
    }
    let step = w.step();
    let f = &*f;
    for (e, mut sec) in &mut line {
        if sec.stale(key_of(&step)) {
            rebuild(&mut commands, e, |p| {
                muted(p, f, &step);
            });
        }
    }
}
