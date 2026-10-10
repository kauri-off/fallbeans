//! The settings tab: controls, sound, display and graphics, and key rebinding.
use bevy::ecs::system::SystemParam;

use super::*;

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
pub(super) enum GfxLine {
    Adapter,
    Upscaler,
    Note,
}

/// The keys of an action (keycaps, redrawn when they change), its row and its button, and the line asking for one
/// while it is rebound.
#[derive(Component)]
pub(super) struct BindKeys(Bind);
#[derive(Component)]
pub(super) struct BindRow(Bind);
#[derive(Component)]
pub(super) struct BindWait(Bind);

/// The options tab, at the room list and in the menu: a side bar of its parts and the part picked, built once; its
/// controls follow the options (`sync_settings`, `sync_sliders`).
pub fn settings_tab(p: &mut ChildSpawnerCommands, f: &Fonts, o: &Options, section: Section) {
    p.spawn(Node {
        flex_grow: 1.0,
        min_height: px(0),
        ..default()
    })
    .with_children(|t| {
        t.spawn((
            Node {
                width: px(250),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                padding: UiRect::all(px(10)),
                border: UiRect::right(px(1)),
                ..default()
            },
            BorderColor::all(LINE),
        ))
        .with_children(|nav| {
            for s in Section::ALL {
                button(nav, f, text::section(s), Look::Nav(s == section), Action::Section(s));
            }
            spacer(nav);
            nav.spawn(Node {
                padding: UiRect::axes(px(14), px(12)),
                ..default()
            })
            .with_children(|foot| {
                rich(foot, f, text::AUTOSAVED, 13.0, FAINT);
            });
        });
        t.spawn((
            Node {
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_width: px(0),
                max_width: px(1000),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(px(34), px(22)),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            bevy::ui_widgets::ScrollArea,
        ))
        .with_children(|body| {
            for s in Section::ALL {
                body.spawn((
                    SectionBody(s),
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: px(6),
                        flex_shrink: 0.0,
                        display: display(s == section),
                        ..default()
                    },
                    motion::Reveal::new(motion::Motion::slide(0.0, 8.0)),
                ))
                .with_children(|c| {
                    c.spawn(Node {
                        margin: UiRect::bottom(px(8)),
                        ..default()
                    })
                    .with_children(|h| {
                        heading(h, f, text::section(s));
                    });
                    section_body(c, f, o, s);
                });
            }
        });
    });
}

fn section_body(c: &mut ChildSpawnerCommands, f: &Fonts, o: &Options, s: Section) {
    let check = |c: &mut ChildSpawnerCommands, s: &str, t: Toggle| {
        button(c, f, s, Look::Check(o.toggle(t)), Action::Set(t));
    };
    let knob = |c: &mut ChildSpawnerCommands, k: Knob, s: &str, range: (f32, f32), step: f32| {
        slider(c, f, k, s, o.knob(k), range, step);
    };
    match s {
        Section::Controls => {
            knob(c, Knob::MouseSens, text::MOUSE_SENS, crate::settings::SENS_RANGE, 0.05);
            check(c, text::INVERT_MOUSE, Toggle::InvertMouse);
            knob(c, Knob::StickSens, text::STICK_SENS, crate::settings::SENS_RANGE, 0.05);
            check(c, text::INVERT_STICK, Toggle::InvertStick);
            check(c, text::SHAKE, Toggle::Shake);
        }
        Section::Screen => {
            knob(c, Knob::Fov, text::FOV, crate::settings::FOV_RANGE, 1.0);
            knob(c, Knob::Volume, text::VOLUME, (0.0, 1.0), 0.05);
            knob(c, Knob::UiScale, text::UI_SCALE, crate::settings::UI_SCALE_RANGE, 0.05);
            check(c, text::SHOW_FPS, Toggle::ShowFps);
            check(c, text::FULLSCREEN, Toggle::Fullscreen);
        }
        Section::Gfx => graphics(c, f, o),
        Section::Keys => {
            for b in crate::keys::BINDS {
                keys_row(c, f, o, b);
            }
            c.spawn(Node {
                align_items: AlignItems::Center,
                column_gap: px(16),
                margin: UiRect::top(px(14)),
                ..default()
            })
            .with_children(|r| {
                let note = muted(r, f, text::KEYS_NOTE);
                r.commands().entity(note).insert(Node {
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    ..default()
                });
                button(r, f, text::RESET_KEYS, Look::Plain, Action::ResetKeys);
            });
        }
        Section::Problems => {
            c.spawn(Node {
                max_width: px(620),
                flex_direction: FlexDirection::Column,
                row_gap: px(18),
                padding: UiRect::top(px(8)),
                ..default()
            })
            .with_children(|k| {
                label(k, f, text::PROBLEMS_NOTE);
                row(k, false, |r| {
                    button(r, f, text::OPEN_LOGS, Look::Plain, Action::OpenLogs);
                });
            });
        }
    }
}

fn keys_row(c: &mut ChildSpawnerCommands, f: &Fonts, o: &Options, b: Bind) {
    c.spawn((
        BindRow(b),
        Node {
            align_items: AlignItems::Center,
            column_gap: px(10),
            min_height: px(50),
            padding: UiRect::axes(px(12), px(0)),
            margin: UiRect::axes(px(-12), px(0)),
            border: UiRect::bottom(px(1)),
            border_radius: BorderRadius::all(px(12)),
            ..default()
        },
        BorderColor::all(HAIR),
        BackgroundColor(Color::NONE),
    ))
    .with_children(|r| {
        r.spawn(Node {
            width: px(120),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|n| {
            rich_in(n, f, text::bind(b), 16.0, INK, true);
        });
        r.spawn((
            BindKeys(b),
            Node {
                flex_grow: 1.0,
                min_width: px(0),
                column_gap: px(6),
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|n| {
            for k in crate::keys::each_label(o.binds.keys(b)) {
                keycap(n, f, k);
            }
        });
        let wait = rich_in(r, f, text::PRESS_KEY, 16.0, APRICOT_INK, true);
        r.commands().entity(wait).insert((
            BindWait(b),
            Node {
                flex_grow: 1.0,
                display: display(false),
                ..default()
            },
            motion::Reveal::new(motion::Motion::pop(0.9)),
        ));
        button(r, f, text::CHANGE, Look::Tiny, Action::Rebind(b));
    });
}

fn graphics(p: &mut ChildSpawnerCommands, f: &Fonts, o: &Options) {
    use crate::opts::Backend;
    use crate::render::quality::{Preset, Upscale};
    let chips = |r: &mut ChildSpawnerCommands, items: &mut dyn Iterator<Item = (&str, GfxPick)>| {
        row(r, true, |r| {
            for (s, pick) in items {
                let (on, ok) = o.picked(pick);
                button_if(r, f, s, Look::Chip(on), Action::Gfx(pick), ok);
            }
        });
    };
    let block = |p: &mut ChildSpawnerCommands, title: &str, items: &mut dyn Iterator<Item = (&str, GfxPick)>| {
        setting_row(p, |r| {
            let t = label(r, f, title);
            r.commands().entity(t).insert(Node {
                flex_grow: 1.0,
                flex_shrink: 1.0,
                ..default()
            });
            chips(r, items);
        });
    };
    let line = |p: &mut ChildSpawnerCommands, l: GfxLine, size: f32, ink: Color, strong: bool| {
        let s = o.line(l);
        let e = rich_in(p, f, s.as_deref().unwrap_or_default(), size, ink, strong);
        p.commands().entity(e).insert((
            l,
            Node {
                display: display(s.is_some()),
                ..default()
            },
        ));
        e
    };
    let presets = [Preset::Low, Preset::High].map(|p| (text::preset(p), GfxPick::Preset(p)));
    block(p, text::PRESET, &mut presets.into_iter());
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::axes(px(0), px(12)),
            border: UiRect::bottom(px(1)),
            ..default()
        },
        BorderColor::all(HAIR),
    ))
    .with_children(|b| {
        b.spawn(Node {
            align_items: AlignItems::Center,
            column_gap: px(16),
            ..default()
        })
        .with_children(|r| {
            let t = label(r, f, text::UPSCALER);
            r.commands().entity(t).insert(Node {
                flex_grow: 1.0,
                flex_shrink: 1.0,
                ..default()
            });
            let upscalers = core::iter::once((text::UPSCALER_AUTO, GfxPick::Upscaler(None)))
                .chain(Upscaler::ALL.map(|u| (text::upscaler(u), GfxPick::Upscaler(Some(u)))));
            chips(r, &mut upscalers.into_iter());
        });
        b.spawn(Node {
            align_items: AlignItems::Baseline,
            column_gap: px(18),
            margin: UiRect::top(px(10)),
            flex_wrap: FlexWrap::Wrap,
            ..default()
        })
        .with_children(|r| {
            line(r, GfxLine::Upscaler, 14.0, INK, true);
            spacer(r);
            line(r, GfxLine::Adapter, 14.0, FAINT, false);
        });
        let note = line(b, GfxLine::Note, 13.5, FAINT, false);
        b.commands().entity(note).insert(Node {
            margin: UiRect::top(px(4)),
            display: display(o.line(GfxLine::Note).is_some()),
            ..default()
        });
    });
    let modes = Upscale::PICKS.map(|m| (text::upscale(m), GfxPick::Upscale(m)));
    block(p, text::UPSCALE, &mut modes.into_iter());
    button(
        p,
        f,
        text::VSYNC,
        Look::Check(o.toggle(Toggle::Vsync)),
        Action::Set(Toggle::Vsync),
    );
    let fps =
        [(0, text::NO_LIMIT), (30, "30"), (60, "60"), (120, "120"), (144, "144")].map(|(n, s)| (s, GfxPick::Fps(n)));
    block(p, text::FPS_LIMIT, &mut fps.into_iter());
    let backends = [
        (text::BACKEND_AUTO, None),
        ("DirectX 12", Some(Backend::Dx12)),
        ("Vulkan", Some(Backend::Vulkan)),
    ]
    .into_iter()
    .filter(|(_, b)| b.is_none_or(Backend::available))
    .map(|(s, b)| (s, GfxPick::Backend(b)));
    block(p, text::BACKEND, &mut backends.into_iter());
}

/// The options tabs follow the options: check boxes, choices, the machine's lines and the keys.
/// The rows of the keys: their keycaps, and the one waiting for a key.
#[derive(SystemParam)]
pub(super) struct KeyRows<'w, 's> {
    keys: Query<'w, 's, (Entity, &'static BindKeys, &'static mut Node), Without<GfxLine>>,
    rows: Query<'w, 's, RowLook>,
    waits: Query<'w, 's, (&'static BindWait, &'static mut Node), WaitOnly>,
}

type RowLook = (&'static BindRow, &'static mut BackgroundColor, &'static mut BorderColor);
type WaitOnly = (Without<GfxLine>, Without<BindKeys>, Without<Act>);
type ButtonOf = (
    Entity,
    &'static Act,
    &'static mut Look,
    &'static mut Node,
    Has<InteractionDisabled>,
);
type LineOf = (&'static GfxLine, &'static mut Rich, &'static mut Node);

/// The settings' buttons and the graphics' lines.
#[derive(SystemParam)]
pub(super) struct SettingNodes<'w, 's> {
    buttons: Query<'w, 's, ButtonOf, Without<BindKeys>>,
    lines: Query<'w, 's, LineOf, (Without<BindKeys>, Without<Act>)>,
}

pub(super) fn sync_settings(
    o: Options,
    ui: Res<Ui>,
    f: Res<Fonts>,
    mut nodes: SettingNodes,
    mut key_rows: KeyRows,
    mut shown_keys: Local<Option<Bindings>>,
    mut commands: Commands,
) {
    let f = &*f;
    for (e, act, mut look, mut node, disabled) in &mut nodes.buttons {
        match act.0 {
            Action::Set(t) => {
                look.set_if_neq(Look::Check(o.toggle(t)));
            }
            Action::Gfx(pick) => {
                let (on, ok) = o.picked(pick);
                look.set_if_neq(Look::Chip(on));
                enable(&mut commands, e, !disabled, ok);
            }
            Action::Rebind(b) => show(&mut node, ui.rebinding != Some(b)),
            _ => {}
        }
    }
    for (l, mut t, mut node) in &mut nodes.lines {
        let s = o.line(*l);
        show(&mut node, s.is_some());
        t.set(s.as_deref().unwrap_or_default());
    }
    let redraw = shown_keys.as_ref() != Some(&*o.binds);
    for (e, k, mut node) in &mut key_rows.keys {
        show(&mut node, ui.rebinding != Some(k.0));
        if redraw {
            let labels = crate::keys::each_label(o.binds.keys(k.0));
            rebuild(&mut commands, e, |n| {
                for l in &labels {
                    keycap(n, f, l);
                }
            });
        }
    }
    *shown_keys = Some(o.binds.clone());
    for (r, mut bg, mut rim) in &mut key_rows.rows {
        let on = ui.rebinding == Some(r.0);
        bg.set_if_neq(BackgroundColor(if on { APRICOT_SOFT } else { Color::NONE }));
        rim.set_if_neq(BorderColor::all(if on { Color::NONE } else { HAIR }));
    }
    for (w, mut node) in &mut key_rows.waits {
        show(&mut node, ui.rebinding == Some(w.0));
    }
}

/// Sliders follow their options (set at the other tab, or anew).
pub(super) fn sync_sliders(o: Options, sliders: Query<(Entity, &Knob, &SliderValue)>, mut commands: Commands) {
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
    section: Res<Section>,
    home: Option<Res<State<HomeTab>>>,
    menu: Option<Res<State<MenuTab>>>,
    mut binds: ResMut<Bindings>,
    mut commands: Commands,
) {
    let Some(b) = ui.rebinding else { return };
    let tab = home.is_some_and(|t| *t.get() == HomeTab::Settings)
        || (ui.menu && menu.is_some_and(|t| *t.get() == MenuTab::Settings));
    if !(tab && *section == Section::Keys) {
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
