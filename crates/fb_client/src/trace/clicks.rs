//! `--trace clicks`: every left click traced through the window, picking, the button widget and the interface's
//! actions, with a verdict a few frames after the release: `#<click> f<frame> …` lines.
use std::collections::BTreeMap;
use std::fmt;

use bevy::diagnostic::FrameCount;
use bevy::ecs::entity::EntityHashMap;
use bevy::ecs::message::MessageCursor;
use bevy::ecs::world::DeferredWorld;
use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::picking::backend::HitData;
use bevy::picking::events::{Cancel, Click, DragEnd, DragStart, Press, Release};
use bevy::picking::hover::{HoverMap, Hovered, PreviousHoverMap, generate_hovermap};
use bevy::picking::pointer::{PointerAction, PointerId, PointerInput};
use bevy::picking::{PickingSystems, events::pointer_events};
use bevy::prelude::*;
use bevy::ui::{InteractionDisabled, Pressed, UiGlobalTransform};
use bevy::ui_widgets::Activate;
use bevy::window::{CursorOptions, PrimaryWindow};
use fb_net::trace::TraceFile;

use crate::ui::{Act, Layer, Ui, UiAction};

/// Frames after the release the verdict waits for (the action reaches its system a frame or two later).
const SETTLE: u32 = 4;

pub fn add(app: &mut App, out: TraceFile) {
    app.insert_resource(Trace {
        out,
        n: 0,
        cur: None,
        churn: BTreeMap::new(),
        churn_since: 0.0,
        menu: None,
    });
    app.add_systems(
        PreUpdate,
        raw_input
            .in_set(PickingSystems::Hover)
            .after(generate_hovermap)
            .before(pointer_events),
    );
    app.add_systems(Update, (actions, menu_state, verdict, churn).chain());
    app.add_observer(on_press)
        .add_observer(on_release)
        .add_observer(on_click)
        .add_observer(on_cancel)
        .add_observer(on_drag_start)
        .add_observer(on_drag_end)
        .add_observer(on_pressed_add)
        .add_observer(on_pressed_remove)
        .add_observer(on_activate)
        .add_observer(on_button_gone);
}

/// One left click, from the window's press to the verdict.
#[derive(Default)]
struct Attempt {
    n: u32,
    down: u32,
    up: Option<u32>,
    pick_down: bool,
    pick_up: bool,
    hit_down: Vec<String>,
    button: Option<(Entity, String)>,
    disabled: bool,
    gone: Option<(u32, String)>,
    unpressed: Option<u32>,
    released_on: Vec<String>,
    clicked: Vec<String>,
    cancelled: bool,
    activated: Vec<String>,
    actions: Vec<String>,
}

#[derive(Resource)]
struct Trace {
    out: TraceFile,
    n: u32,
    cur: Option<Attempt>,
    /// Buttons despawned this second, by the section they were under.
    churn: BTreeMap<String, u32>,
    churn_since: f32,
    menu: Option<bool>,
}

fn frame(w: &World) -> u32 {
    w.get_resource::<FrameCount>().map_or(0, |f| f.0)
}

fn raw_input(
    world: &mut World,
    mut buttons: Local<MessageCursor<MouseButtonInput>>,
    mut pointer: Local<MessageCursor<PointerInput>>,
) {
    let f = frame(world);
    let presses: Vec<MouseButtonInput> = buttons
        .read(world.resource::<Messages<MouseButtonInput>>())
        .filter(|b| b.button == MouseButton::Left)
        .copied()
        .collect();
    let inputs: Vec<PointerInput> = pointer
        .read(world.resource::<Messages<PointerInput>>())
        .filter(|p| p.pointer_id == PointerId::Mouse)
        .cloned()
        .collect();
    if presses.is_empty() && inputs.is_empty() {
        return;
    }
    let mut q = world.query_filtered::<(&Window, Option<&CursorOptions>), With<PrimaryWindow>>();
    let (cursor, state) = q
        .single(world)
        .map_or((None, String::from("no primary window")), |(w, c)| {
            (
                w.cursor_position(),
                format!(
                    "focused={} grab={:?} visible={}",
                    w.focused,
                    c.map(|c| c.grab_mode),
                    c.is_none_or(|c| c.visible)
                ),
            )
        });
    let at = cursor.map_or("outside".into(), |c| format!("({:.1}, {:.1})", c.x, c.y));
    let dt = world.resource::<Time<Real>>().delta_secs() * 1000.0;
    for b in presses {
        let mut trace = world.resource_mut::<Trace>();
        match b.state {
            ButtonState::Pressed => {
                if let Some(a) = trace.cur.take() {
                    report(&mut trace.out, a, "next press came first");
                }
                trace.n += 1;
                let n = trace.n;
                trace.cur = Some(Attempt {
                    n,
                    down: f,
                    ..default()
                });
                trace.say(f, format_args!("window: down at {at}, frame {dt:.1} ms, {state}"));
            }
            ButtonState::Released => {
                trace.say(f, format_args!("window: up at {at}, frame {dt:.1} ms, {state}"));
                if let Some(a) = &mut trace.cur {
                    a.up = Some(f);
                }
            }
        }
    }
    for p in inputs {
        let pos = p.location.position;
        match p.action {
            PointerAction::Press(PointerButton::Primary) => {
                let hits = hits(world, world.resource::<HoverMap>().get(&PointerId::Mouse));
                let mut trace = world.resource_mut::<Trace>();
                trace.say(
                    f,
                    format_args!(
                        "picking: press at ({:.1}, {:.1}); under it now: {}",
                        pos.x,
                        pos.y,
                        list(&hits)
                    ),
                );
                if let Some(a) = &mut trace.cur {
                    a.pick_down = true;
                    a.hit_down = hits;
                }
            }
            PointerAction::Release(PointerButton::Primary) => {
                let before = hits(world, world.resource::<PreviousHoverMap>().get(&PointerId::Mouse));
                let now = hits(world, world.resource::<HoverMap>().get(&PointerId::Mouse));
                let mut trace = world.resource_mut::<Trace>();
                trace.say(
                    f,
                    format_args!(
                        "picking: release at ({:.1}, {:.1}); hovered last frame (gets Click/Release): {}; now: {}",
                        pos.x,
                        pos.y,
                        list(&before),
                        list(&now)
                    ),
                );
                if let Some(a) = &mut trace.cur {
                    a.pick_up = true;
                }
            }
            _ => {}
        }
    }
}

fn hits(w: &World, map: Option<&EntityHashMap<HitData>>) -> Vec<String> {
    let Some(map) = map else { return Vec::new() };
    let mut v: Vec<_> = map.iter().collect();
    v.sort_by(|a, b| a.1.depth.total_cmp(&b.1.depth));
    v.into_iter()
        .map(|(e, h)| format!("{} depth {:.1}", describe(w, *e), h.depth))
        .collect()
}

fn list(v: &[String]) -> String {
    if v.is_empty() { "nothing".into() } else { v.join(" | ") }
}

/// An entity as the log shows it: its text, the button it is (or is in), its layer, state and rect.
fn describe(w: &World, e: Entity) -> String {
    if w.get_entity(e).is_err() {
        return format!("{e} (despawned)");
    }
    let mut s = format!("{e}");
    let mut button = None;
    let mut layer = None;
    let mut cur = Some(e);
    while let Some(c) = cur {
        if button.is_none()
            && let Some(a) = w.get::<Act>(c)
        {
            button = Some((c, format!("{:?}", a.0)));
        }
        if layer.is_none() {
            layer = w.get::<Layer>(c).copied();
        }
        cur = w.get::<ChildOf>(c).map(ChildOf::parent);
    }
    match &button {
        Some((b, a)) if *b == e => s += &format!(" button {a}"),
        Some((b, a)) => s += &format!(" in button {b} {a}"),
        None => s += &format!(" {}", own_types(w, e)),
    }
    if let Some(t) = text_under(w, button.as_ref().map_or(e, |b| b.0)) {
        s += &format!(" «{t}»");
    }
    if w.get::<Pressed>(e).is_some() {
        s += " PRESSED";
    }
    if w.get::<InteractionDisabled>(e).is_some() {
        s += " DISABLED";
    }
    if w.get::<Window>(e).is_some() {
        s += " (the window: no interface here)";
    }
    if let Some(r) = rect(w, e) {
        s += &format!(
            " rect ({:.0}, {:.0})–({:.0}, {:.0})",
            r.min.x, r.min.y, r.max.x, r.max.y
        );
    }
    if let Some(l) = layer {
        s += &format!(" [{l:?}]");
    }
    s
}

/// The game's own components on `e`, or the last of Bevy's.
fn own_types(w: &World, e: Entity) -> String {
    let Ok(infos) = w.inspect_entity(e) else {
        return String::new();
    };
    let names: Vec<String> = infos.map(|i| i.name().to_string()).collect();
    let own: Vec<&str> = names
        .iter()
        .filter(|n| n.starts_with("fb_client::"))
        .map(|n| n.rsplit("::").next().unwrap_or(n))
        .collect();
    if own.is_empty() {
        let kinds = ["Text", "TextSpan", "ImageNode", "Node", "Camera", "Mesh3d", "Window"];
        let found = kinds
            .into_iter()
            .find(|k| names.iter().any(|n| n.rsplit("::").next() == Some(*k)));
        found.unwrap_or("?").to_string()
    } else {
        own.join("+")
    }
}

/// The first text in `e` or under it, shortened.
fn text_under(w: &World, e: Entity) -> Option<String> {
    let mut out = String::new();
    let mut stack = vec![e];
    while let Some(c) = stack.pop() {
        if let Some(t) = w.get::<TextSpan>(c) {
            out += &t.0;
        }
        if let Some(t) = w.get::<Text>(c) {
            out += &t.0;
        }
        if out.chars().count() > 40 {
            break;
        }
        if let Some(ch) = w.get::<Children>(c) {
            stack.extend(ch.iter().rev());
        }
    }
    let out: String = out.chars().take(40).collect();
    (!out.trim().is_empty()).then_some(out)
}

fn rect(w: &World, e: Entity) -> Option<Rect> {
    let n = w.get::<ComputedNode>(e)?;
    let t = w.get::<UiGlobalTransform>(e)?;
    let mut windows = w.try_query_filtered::<&Window, With<PrimaryWindow>>()?;
    let k = windows.single(w).map_or(1.0, |w| 1.0 / w.scale_factor());
    Some(Rect::from_center_size(t.translation * k, n.size() * k))
}

fn mouse<E: core::fmt::Debug + Clone + Reflect>(ev: &On<Pointer<E>>) -> bool {
    ev.pointer_id == PointerId::Mouse && ev.entity == ev.original_event_target()
}

fn at<E: core::fmt::Debug + Clone + Reflect>(ev: &On<Pointer<E>>) -> String {
    let p = ev.pointer_location.position;
    format!("({:.1}, {:.1})", p.x, p.y)
}

fn n_of(t: &Trace) -> u32 {
    t.cur.as_ref().map_or(0, |a| a.n)
}

impl Trace {
    /// A line of the click under way at frame `f`.
    fn say(&mut self, f: u32, l: fmt::Arguments) {
        let n = n_of(self);
        self.out.line(format_args!("#{n} f{f} {l}"));
    }
}

fn on_press(ev: On<Pointer<Press>>, mut w: DeferredWorld) {
    if !mouse(&ev) || ev.event.button != PointerButton::Primary {
        return;
    }
    let d = describe(&w, ev.entity);
    let f = frame(&w);
    w.resource_mut::<Trace>()
        .say(f, format_args!("Press → {d} at {}", at(&ev)));
}

fn on_release(ev: On<Pointer<Release>>, mut w: DeferredWorld) {
    if !mouse(&ev) || ev.event.button != PointerButton::Primary {
        return;
    }
    let d = describe(&w, ev.entity);
    let f = frame(&w);
    let mut t = w.resource_mut::<Trace>();
    t.say(f, format_args!("Release → {d}"));
    if let Some(a) = &mut t.cur {
        a.released_on.push(d);
    }
}

fn on_click(ev: On<Pointer<Click>>, mut w: DeferredWorld) {
    if !mouse(&ev) || ev.event.button != PointerButton::Primary {
        return;
    }
    let d = describe(&w, ev.entity);
    let f = frame(&w);
    let mut t = w.resource_mut::<Trace>();
    t.say(
        f,
        format_args!("Click → {d} (held {:.0} ms)", ev.event.duration.as_secs_f32() * 1000.0),
    );
    if let Some(a) = &mut t.cur {
        a.clicked.push(d);
    }
}

fn on_cancel(ev: On<Pointer<Cancel>>, mut w: DeferredWorld) {
    if !mouse(&ev) {
        return;
    }
    let d = describe(&w, ev.entity);
    let f = frame(&w);
    let mut t = w.resource_mut::<Trace>();
    t.say(f, format_args!("Cancel → {d}"));
    if let Some(a) = &mut t.cur {
        a.cancelled = true;
    }
}

fn on_drag_start(ev: On<Pointer<DragStart>>, mut w: DeferredWorld) {
    if mouse(&ev) && ev.event.button == PointerButton::Primary {
        let (d, f) = (describe(&w, ev.entity), frame(&w));
        w.resource_mut::<Trace>()
            .say(f, format_args!("DragStart → {d} at {}", at(&ev)));
    }
}

fn on_drag_end(ev: On<Pointer<DragEnd>>, mut w: DeferredWorld) {
    if mouse(&ev) && ev.event.button == PointerButton::Primary {
        let (d, f) = (describe(&w, ev.entity), frame(&w));
        w.resource_mut::<Trace>()
            .say(f, format_args!("DragEnd → {d} (moved {:?})", ev.event.distance));
    }
}

fn on_pressed_add(ev: On<Add, Pressed>, mut w: DeferredWorld) {
    let d = describe(&w, ev.entity);
    let disabled = w.get::<InteractionDisabled>(ev.entity).is_some();
    let f = frame(&w);
    let mut t = w.resource_mut::<Trace>();
    t.say(f, format_args!("+Pressed {d}"));
    if let Some(a) = &mut t.cur {
        a.button = Some((ev.entity, d));
        a.disabled = disabled;
    }
}

fn on_pressed_remove(ev: On<Remove, Pressed>, mut w: DeferredWorld) {
    let d = describe(&w, ev.entity);
    let f = frame(&w);
    let mut t = w.resource_mut::<Trace>();
    t.say(f, format_args!("-Pressed {d}"));
    if let Some(a) = &mut t.cur
        && a.button.as_ref().is_some_and(|b| b.0 == ev.entity)
    {
        a.unpressed = Some(f);
    }
}

fn on_activate(ev: On<Activate>, mut w: DeferredWorld) {
    let d = describe(&w, ev.entity);
    let f = frame(&w);
    let mut t = w.resource_mut::<Trace>();
    t.say(f, format_args!("Activate {d}"));
    if let Some(a) = &mut t.cur {
        a.activated.push(d);
    }
}

/// A button despawned: logged when it matters to the click under way, counted always.
fn on_button_gone(ev: On<Despawn, Act>, mut w: DeferredWorld) {
    let e = ev.entity;
    let d = describe(&w, e);
    let section = w.get::<ChildOf>(e).map_or_else(
        || "root".into(),
        |p| format!("{} {}", p.parent(), own_types(&w, p.parent())),
    );
    let hovered = w.get::<Hovered>(e).is_some_and(|h| h.0);
    let pressed = w.get::<Pressed>(e).is_some();
    let f = frame(&w);
    let mut t = w.resource_mut::<Trace>();
    *t.churn.entry(section.clone()).or_default() += 1;
    let ours = t
        .cur
        .as_ref()
        .is_some_and(|a| a.button.as_ref().is_some_and(|b| b.0 == e));
    let held = t.cur.as_ref().is_some_and(|a| a.up.is_none());
    if ours || pressed || hovered || held {
        t.say(
            f,
            format_args!("despawned {d} (hovered={hovered}) from section {section}"),
        );
    }
    if ours && let Some(a) = &mut t.cur {
        a.gone = Some((f, section));
    }
}

fn actions(mut read: MessageReader<UiAction>, frames: Res<FrameCount>, mut trace: ResMut<Trace>) {
    for UiAction(act) in read.read() {
        trace.say(frames.0, format_args!("UiAction {act:?}"));
        if let Some(a) = &mut trace.cur {
            a.actions.push(format!("{act:?}"));
        }
    }
}

fn menu_state(ui: Res<Ui>, frames: Res<FrameCount>, mut trace: ResMut<Trace>) {
    if trace.menu != Some(ui.menu) {
        if trace.menu.is_some() {
            let open = if ui.menu { "opened" } else { "closed" };
            trace.say(frames.0, format_args!("menu {open}"));
        }
        trace.menu = Some(ui.menu);
    }
}

fn verdict(frames: Res<FrameCount>, mut trace: ResMut<Trace>) {
    let done = trace
        .cur
        .as_ref()
        .and_then(|a| a.up)
        .is_some_and(|up| frames.0 >= up + SETTLE);
    if done && let Some(a) = trace.cur.take() {
        report(&mut trace.out, a, "");
    }
}

fn report(out: &mut TraceFile, a: Attempt, note: &str) {
    let n = a.n;
    let span = a.up.map_or("not released".into(), |u| format!("{} frames", u - a.down));
    let why = if !a.pick_down {
        "picking never saw the press (window input not reaching picking: focus, captured cursor, or the event was eaten)"
            .to_string()
    } else if a.hit_down.is_empty() {
        "the press hit nothing pickable".into()
    } else if a.button.is_none() {
        format!(
            "the press reached no button (no Pressed added); topmost hit: {}",
            a.hit_down[0]
        )
    } else if a.disabled {
        "the button is disabled".into()
    } else if let Some((f, section)) = &a.gone {
        format!(
            "the pressed button was despawned at f{f} ({} frames after the press) — section {section} was rebuilt under the pointer",
            f - a.down
        )
    } else if a.up.is_some() && !a.pick_up {
        "picking never saw the release".into()
    } else if a.cancelled {
        "the pointer was cancelled".into()
    } else if a.clicked.is_empty() {
        format!(
            "no Click: released over [{}], not the pressed {}",
            a.released_on.join(" | "),
            a.button.as_ref().map_or("", |b| b.1.as_str())
        )
    } else if a.activated.is_empty() {
        format!(
            "Click came but no Activate (Pressed removed at {:?}, before the Click?)",
            a.unpressed
        )
    } else if a.actions.is_empty() {
        "Activate fired but no UiAction (no Act on the button?)".into()
    } else {
        format!("OK → {}", a.actions.join(", "))
    };
    let note = if note.is_empty() {
        String::new()
    } else {
        format!(" ({note})")
    };
    out.line(format_args!("#{n} VERDICT{note}: {why}; held {span}"));
}

fn churn(time: Res<Time<Real>>, mut trace: ResMut<Trace>) {
    let now = time.elapsed_secs();
    if now - trace.churn_since < 1.0 {
        return;
    }
    trace.churn_since = now;
    if trace.churn.is_empty() {
        return;
    }
    let total: u32 = trace.churn.values().sum();
    let by: Vec<String> = trace.churn.iter().map(|(s, c)| format!("{c} under {s}")).collect();
    trace.out.line(format_args!(
        "buttons despawned in the last second: {total} ({})",
        by.join("; ")
    ));
    trace.churn.clear();
}
