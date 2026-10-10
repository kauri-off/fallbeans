//! The loading screen: over everything while the warm-up runs (`render/warmup.rs`), at the start and after a
//! change of the graphics. Opaque: what is built behind it is not seen, and it takes the clicks meant for it.
use bevy::input_focus::InputFocus;
use bevy::prelude::*;

use super::*;
use crate::render::warmup::Warmup;

/// The screen's backdrop: the night behind the panels.
const BACKDROP: Color = PLANE;

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build_screen);
        app.add_systems(Update, update_screen);
    }
}

#[derive(Component)]
struct Curtain;

#[derive(Component)]
struct Bar;

#[derive(Component)]
struct StepLine;

fn build_screen(mut commands: Commands, f: Res<Fonts>, warm: Option<Res<Warmup>>) {
    let f = &*f;
    let on = warm.is_some_and(|w| w.busy());
    commands
        .spawn((
            Curtain,
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
            // (Also warms the gradient pipeline the interface draws with.)
            BackgroundGradient::from(RadialGradient::new(
                UiPosition::TOP,
                RadialGradientShape::FarthestCorner,
                vec![BLUE_SOFT.into(), BLUE_SOFT.with_alpha(0.3).into(), BACKDROP.into()],
            )),
            // (Above every layer, the F4 overlay too.)
            GlobalZIndex(1000),
        ))
        .with_children(|s| {
            logo(s, f, 80.0);
            row(s, false, |r| {
                spinner(r, 1.25, BLUE);
                rich_in(r, f, text::LOADING, 18.0, INK, true);
            });
            s.spawn((
                Node {
                    width: rem(24.0),
                    max_width: percent(80),
                    height: rem(0.375),
                    border_radius: BorderRadius::MAX,
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(ink_wash(0.08)),
            ))
            .with_children(|b| {
                b.spawn((
                    Bar,
                    Node {
                        width: percent(0),
                        height: percent(100),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(BLUE),
                    BoxShadow::new(BLUE.with_alpha(0.5), px(0), px(0), px(0), px(10)),
                ));
            });
            let step = muted(s, f, "");
            s.commands().entity(step).insert(StepLine);
        });
}

/// Up while the warm-up runs, its bar and what it is doing; no text field has the keyboard meanwhile.
fn update_screen(
    warm: Option<Res<Warmup>>,
    mut screen: Query<&mut Node, (With<Curtain>, Without<Bar>)>,
    mut bar: Query<&mut Node, (With<Bar>, Without<Curtain>)>,
    mut line: Query<&mut Rich, With<StepLine>>,
    mut focus: ResMut<InputFocus>,
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
    for mut t in &mut line {
        t.set(&w.step());
    }
}
