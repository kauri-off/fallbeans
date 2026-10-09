//! The settings tab: controls, sound, display and graphics, and key rebinding.
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

/// The keys of an action, and the line asking for one while it is rebound.
#[derive(Component)]
pub(super) struct BindKeys(Bind);
#[derive(Component)]
pub(super) struct BindWait(Bind);

/// The options tab, at the room list and in the menu: built once, its controls follow the options
/// (`sync_settings`, `sync_sliders`).
pub fn settings_tab(p: &mut ChildSpawnerCommands, f: &Fonts, o: &Options, folds: &Folds) {
    let check = |c: &mut ChildSpawnerCommands, s: &str, t: Toggle| {
        button(c, f, s, Look::Check(o.toggle(t)), Action::Set(t));
    };
    let knob = |c: &mut ChildSpawnerCommands, k: Knob, s: &str, range: (f32, f32), step: f32| {
        slider(c, f, k, s, o.knob(k), range, step);
    };
    stack(p, |c| {
        knob(c, Knob::MouseSens, text::MOUSE_SENS, crate::settings::SENS_RANGE, 0.05);
        check(c, text::INVERT_MOUSE, Toggle::InvertMouse);
        knob(c, Knob::StickSens, text::STICK_SENS, crate::settings::SENS_RANGE, 0.05);
        check(c, text::INVERT_STICK, Toggle::InvertStick);
        check(c, text::SHAKE, Toggle::Shake);
        knob(c, Knob::Fov, text::FOV, crate::settings::FOV_RANGE, 1.0);
        knob(c, Knob::Volume, text::VOLUME, (0.0, 1.0), 0.05);
        knob(c, Knob::UiScale, text::UI_SCALE, crate::settings::UI_SCALE_RANGE, 0.05);
        check(c, text::SHOW_FPS, Toggle::ShowFps);
        check(c, text::FULLSCREEN, Toggle::Fullscreen);
        fold(c, f, folds, Fold::Gfx, text::GRAPHICS, |k| graphics(k, f, o));
        fold(c, f, folds, Fold::Keys, text::KEYS, |k| {
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
                        let keys = heading(n, f, &crate::keys::labels(o.binds.keys(b)));
                        n.commands().entity(keys).insert(BindKeys(b));
                        let wait = rich(n, f, text::PRESS_KEY, 14.0, PINK);
                        n.commands().entity(wait).insert((
                            BindWait(b),
                            Node {
                                display: display(false),
                                ..default()
                            },
                        ));
                    });
                    button(r, f, text::CHANGE, Look::Tiny, Action::Rebind(b));
                });
            }
            muted(k, f, text::KEYS_NOTE);
            button(k, f, text::RESET_KEYS, Look::Tiny, Action::ResetKeys);
        });
        fold(c, f, folds, Fold::Problems, text::PROBLEMS, |k| {
            muted(k, f, text::PROBLEMS_NOTE);
            button(k, f, text::OPEN_LOGS, Look::Tiny, Action::OpenLogs);
        });
    });
}

fn graphics(p: &mut ChildSpawnerCommands, f: &Fonts, o: &Options) {
    use crate::opts::Backend;
    use crate::render::quality::{Preset, Upscale};
    let chips = |p: &mut ChildSpawnerCommands, title: &str, items: &mut dyn Iterator<Item = (&str, GfxPick)>| {
        stack(p, |c| {
            muted(c, f, title);
            row(c, true, |r| {
                for (s, pick) in items {
                    let (on, ok) = o.picked(pick);
                    button_if(r, f, s, Look::Chip(on), Action::Gfx(pick), ok);
                }
            });
        });
    };
    let line = |p: &mut ChildSpawnerCommands, l: GfxLine| {
        let s = o.line(l);
        let e = muted(p, f, s.as_deref().unwrap_or_default());
        p.commands().entity(e).insert((
            l,
            Node {
                display: display(s.is_some()),
                ..default()
            },
        ));
    };
    let presets = [Preset::Low, Preset::High].map(|p| (text::preset(p), GfxPick::Preset(p)));
    chips(p, text::PRESET, &mut presets.into_iter());
    line(p, GfxLine::Adapter);
    let upscalers = core::iter::once((text::UPSCALER_AUTO, GfxPick::Upscaler(None)))
        .chain(Upscaler::ALL.map(|u| (text::upscaler(u), GfxPick::Upscaler(Some(u)))));
    chips(p, text::UPSCALER, &mut upscalers.into_iter());
    let modes = Upscale::PICKS.map(|m| (text::upscale(m), GfxPick::Upscale(m)));
    chips(p, text::UPSCALE, &mut modes.into_iter());
    line(p, GfxLine::Upscaler);
    line(p, GfxLine::Note);
    button(
        p,
        f,
        text::VSYNC,
        Look::Check(o.toggle(Toggle::Vsync)),
        Action::Set(Toggle::Vsync),
    );
    let fps =
        [(0, text::NO_LIMIT), (30, "30"), (60, "60"), (120, "120"), (144, "144")].map(|(n, s)| (s, GfxPick::Fps(n)));
    chips(p, text::FPS_LIMIT, &mut fps.into_iter());
    let backends = [
        (text::BACKEND_AUTO, None),
        ("DirectX 12", Some(Backend::Dx12)),
        ("Vulkan", Some(Backend::Vulkan)),
    ]
    .into_iter()
    .filter(|(_, b)| b.is_none_or(Backend::available))
    .map(|(s, b)| (s, GfxPick::Backend(b)));
    chips(p, text::BACKEND, &mut backends.into_iter());
}

/// The options tabs follow the options: check boxes, choices, the machine's lines and the keys.
pub(super) fn sync_settings(
    o: Options,
    ui: Res<Ui>,
    mut buttons: Query<(Entity, &Act, &mut Look, Has<InteractionDisabled>)>,
    mut lines: Query<(&GfxLine, &mut Rich, &mut Node)>,
    mut keys: Query<(&BindKeys, &mut Rich), Without<GfxLine>>,
    mut waits: Query<(&BindWait, &mut Node), Without<GfxLine>>,
    mut commands: Commands,
) {
    for (e, act, mut look, disabled) in &mut buttons {
        match act.0 {
            Action::Set(t) => {
                look.set_if_neq(Look::Check(o.toggle(t)));
            }
            Action::Gfx(pick) => {
                let (on, ok) = o.picked(pick);
                look.set_if_neq(Look::Chip(on));
                enable(&mut commands, e, !disabled, ok);
            }
            _ => {}
        }
    }
    for (l, mut t, mut node) in &mut lines {
        let s = o.line(*l);
        show(&mut node, s.is_some());
        t.set(s.as_deref().unwrap_or_default());
    }
    for (k, mut t) in &mut keys {
        t.set(&crate::keys::labels(o.binds.keys(k.0)));
    }
    for (w, mut node) in &mut waits {
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
    folds: Res<Folds>,
    home: Option<Res<State<HomeTab>>>,
    menu: Option<Res<State<MenuTab>>>,
    mut binds: ResMut<Bindings>,
    mut commands: Commands,
) {
    let Some(b) = ui.rebinding else { return };
    let tab = home.is_some_and(|t| *t.get() == HomeTab::Settings)
        || (ui.menu && menu.is_some_and(|t| *t.get() == MenuTab::Settings));
    if !(tab && folds.open(Fold::Keys)) {
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
