//! Lists sorted by dragging: the row taken follows the pointer, the others make way, and it settles at its place.
use bevy::picking::events::{Drag, DragEnd, DragStart, Pointer};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::ui::UiSystems;

use super::Act;
use super::motion::{Motion, MotionSystems};

pub struct SortPlugin;

impl Plugin for SortPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sorting>();
        app.add_message::<Moved>();
        app.add_systems(
            PostUpdate,
            (adopt, follow).chain().before(MotionSystems).before(UiSystems::Prepare),
        );
    }
}

/// The lists that can be sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Servers,
    Rounds,
}

/// A row of a sorted list, by its place (the list's rows are siblings).
#[derive(Component, Clone, Copy, Debug)]
pub struct Sortable {
    pub list: List,
    pub index: usize,
}

/// A row was dropped at another place.
#[derive(Message, Clone, Copy, Debug)]
pub struct Moved {
    pub list: List,
    pub from: usize,
    pub to: usize,
}

/// How long a dropped row waits for its list to be redrawn before it goes back (s).
const WAIT_S: f32 = 1.5;
/// How fast the rows ease to where they go (1/s).
const EASE: f32 = 18.0;
const LIFT: f32 = 1.025;

struct Slot {
    row: Entity,
    top: f32,
    h: f32,
    off: f32,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Dragging,
    /// Dropped at `to`: waits for the list redrawn in its new order.
    Dropped {
        to: usize,
        at: f32,
    },
    /// Let go where it was taken, or never redrawn: the rows go back.
    Back,
}

/// The drag under way, or the rows still easing after it.
#[derive(Resource, Default)]
pub struct Sorting {
    state: State,
    list: Option<List>,
    from: usize,
    dy: f32,
    slots: Vec<Slot>,
    look: Option<(BackgroundColor, Option<BoxShadow>)>,
}

impl Sorting {
    /// A row of `list` is held: its list is not redrawn under the pointer.
    pub fn holds(&self, list: List) -> bool {
        matches!(self.state, State::Dragging) && self.list == Some(list)
    }

    fn to(&self) -> usize {
        let Some(me) = self.slots.get(self.from) else {
            return self.from;
        };
        let mid = me.top + me.h / 2.0 + self.dy;
        self.slots
            .iter()
            .enumerate()
            .filter(|&(k, s)| k != self.from && s.top + s.h / 2.0 < mid)
            .count()
    }

    /// Where each row goes, from where it is laid out.
    fn target(&self, k: usize, to: usize) -> f32 {
        let me = &self.slots[self.from];
        let step = me.h + self.gap();
        if k == self.from {
            return match self.state {
                State::Dragging => self.dy,
                State::Dropped { .. } => self.top_at(to) - me.top,
                _ => 0.0,
            };
        }
        if matches!(self.state, State::Back) {
            0.0
        } else if self.from < k && k <= to {
            -step
        } else if to <= k && k < self.from {
            step
        } else {
            0.0
        }
    }

    fn gap(&self) -> f32 {
        match self.slots.as_slice() {
            [a, b, ..] => (b.top - a.top - a.h).max(0.0),
            _ => 0.0,
        }
    }

    /// The top of the taken row once at place `to`.
    fn top_at(&self, to: usize) -> f32 {
        let me = &self.slots[self.from];
        let s = &self.slots[to];
        if to <= self.from { s.top } else { s.top + s.h - me.h }
    }
}

/// Makes `e` a row of `list` at place `index`, taken by a drag anywhere but on its buttons.
pub fn sortable(commands: &mut Commands, e: Entity, list: List, index: usize) {
    commands
        .entity(e)
        .insert(Sortable { list, index })
        .observe(take)
        .observe(drag)
        .observe(drop);
}

fn take(
    ev: On<Pointer<DragStart>>,
    tree: Query<(Option<&ChildOf>, Has<Act>)>,
    children: Query<&Children>,
    nodes: Query<(&Sortable, &ComputedNode, &UiGlobalTransform, &UiTransform)>,
    mut looks: Query<(&mut BackgroundColor, Option<&BoxShadow>, &mut ZIndex)>,
    mut sorting: ResMut<Sorting>,
    mut commands: Commands,
) {
    let row = ev.entity;
    // (Not while a row dropped waits for its list redrawn: the places are about to change.)
    if ev.button != PointerButton::Primary || matches!(sorting.state, State::Dragging | State::Dropped { .. }) {
        return;
    }
    let (Ok((me, ..)), Ok((Some(parent), _))) = (nodes.get(row), tree.get(row)) else {
        return;
    };
    // (A drag that starts on a button would press it when let go over it, the row having followed.)
    let mut e = ev.original_event_target();
    while e != row {
        match tree.get(e) {
            Ok((_, true)) => return,
            Ok((Some(up), false)) => e = up.parent(),
            _ => break,
        }
    }
    let mut slots = Vec::new();
    for &e in children.get(parent.parent()).into_iter().flatten() {
        let Ok((s, node, gt, tf)) = nodes.get(e) else { continue };
        if s.list != me.list {
            continue;
        }
        let k = node.inverse_scale_factor;
        let shown = match tf.translation.y {
            Val::Px(y) => y,
            _ => 0.0,
        };
        let h = node.size.y * k;
        slots.push(Slot {
            row: e,
            top: gt.translation.y * k - h / 2.0 - shown,
            h,
            off: shown,
        });
    }
    let Some(from) = slots.iter().position(|s| s.row == row) else {
        return;
    };
    if slots.len() < 2 {
        return;
    }
    for s in &slots {
        commands.entity(s.row).remove::<Motion>();
    }
    let look = looks.get_mut(row).ok().map(|(mut bg, shadow, mut z)| {
        let was = (*bg, shadow.cloned());
        if bg.0.alpha() < 0.5 {
            *bg = BackgroundColor(super::CARD);
        }
        *z = ZIndex(1);
        was
    });
    commands.entity(row).insert(BoxShadow::new(
        super::SHADOW.with_alpha(0.28),
        px(0),
        px(8),
        px(0),
        px(22),
    ));
    *sorting = Sorting {
        state: State::Dragging,
        list: Some(me.list),
        from,
        dy: 0.0,
        slots,
        look,
    };
}

fn drag(ev: On<Pointer<Drag>>, scale: Res<UiScale>, mut sorting: ResMut<Sorting>) {
    let s = sorting.bypass_change_detection();
    if !matches!(s.state, State::Dragging) || s.slots.get(s.from).is_none_or(|m| m.row != ev.entity) {
        return;
    }
    let (Some(first), Some(last)) = (s.slots.first(), s.slots.last()) else {
        return;
    };
    let me = &s.slots[s.from];
    let lo = first.top - me.top;
    let hi = last.top + last.h - me.top - me.h;
    s.dy = (ev.distance.y / scale.0.max(1e-3)).clamp(lo.min(0.0), hi.max(0.0));
}

fn drop(ev: On<Pointer<DragEnd>>, time: Res<Time<Real>>, mut sorting: ResMut<Sorting>, mut out: MessageWriter<Moved>) {
    if !matches!(sorting.state, State::Dragging) || sorting.slots.get(sorting.from).is_none_or(|m| m.row != ev.entity) {
        return;
    }
    let to = sorting.to();
    let from = sorting.from;
    let Some(list) = sorting.list else { return };
    if to == from {
        sorting.state = State::Back;
        return;
    }
    sorting.state = State::Dropped {
        to,
        at: time.elapsed_secs(),
    };
    out.write(Moved { list, from, to });
}

/// Eases every row of the list to where it goes; the row taken is put down once there.
fn follow(
    time: Res<Time<Real>>,
    mut sorting: ResMut<Sorting>,
    mut q: Query<(&mut UiTransform, &mut BackgroundColor, &mut ZIndex)>,
    mut commands: Commands,
) {
    let s = sorting.bypass_change_detection();
    if matches!(s.state, State::Idle) {
        return;
    }
    // (Its list redrawn under it some other way: nothing left to move.)
    if s.slots.get(s.from).is_none_or(|m| !q.contains(m.row)) {
        *sorting = Sorting::default();
        return;
    }
    if let State::Dropped { at, .. } = s.state
        && time.elapsed_secs() - at > WAIT_S
    {
        s.state = State::Back;
    }
    let to = match s.state {
        State::Dropped { to, .. } => to,
        _ => s.to(),
    };
    let k = 1.0 - (-EASE * time.delta_secs().min(0.1)).exp();
    let mut still = true;
    for i in 0..s.slots.len() {
        let goal = s.target(i, to);
        let held = i == s.from && matches!(s.state, State::Dragging);
        let slot = &mut s.slots[i];
        slot.off = if held { goal } else { slot.off + (goal - slot.off) * k };
        if (goal - slot.off).abs() > 0.5 {
            still = false;
        } else if !held {
            slot.off = goal;
        }
        let Ok((mut tf, mut bg, mut z)) = q.get_mut(slot.row) else {
            continue;
        };
        tf.translation = Val2::px(0.0, slot.off);
        if i != s.from {
            continue;
        }
        let lift = if held {
            LIFT
        } else {
            1.0 + (LIFT - 1.0) * (slot.off - goal).abs().min(8.0) / 8.0
        };
        tf.scale = Vec2::splat(lift);
        if !held
            && slot.off == goal
            && let Some((was, shadow)) = s.look.take()
        {
            *bg = was;
            *z = ZIndex(0);
            match shadow {
                Some(sh) => commands.entity(slot.row).insert(sh),
                None => commands.entity(slot.row).remove::<BoxShadow>(),
            };
        }
    }
    if still && matches!(s.state, State::Back) {
        *sorting = Sorting::default();
    }
}

/// The list redrawn in its new order: each row slides on from where it was seen.
fn adopt(added: Query<(Entity, &Sortable), Added<Sortable>>, mut sorting: ResMut<Sorting>, mut commands: Commands) {
    let State::Dropped { to, .. } = sorting.state else {
        return;
    };
    let from = sorting.from;
    // (How far each row, by its new place, still is from it.)
    let mut left = vec![0.0; sorting.slots.len()];
    for (k, slot) in sorting.slots.iter().enumerate() {
        let (at, goal) = if k == from {
            (to, sorting.top_at(to) - sorting.slots[from].top)
        } else if from < k && k <= to {
            (k - 1, sorting.target(k, to))
        } else if to <= k && k < from {
            (k + 1, sorting.target(k, to))
        } else {
            (k, 0.0)
        };
        left[at] = slot.off - goal;
    }
    let mut seen = false;
    for (e, s) in &added {
        if Some(s.list) != sorting.list {
            continue;
        }
        seen = true;
        let mut row = commands.entity(e);
        row.remove::<Motion>();
        if s.index == to {
            row.insert(ZIndex(1));
        }
        if let Some(&d) = left.get(s.index)
            && d.abs() > 0.5
        {
            row.insert(Motion::slide(0.0, d).lasting(0.22));
        }
    }
    if seen {
        *sorting = Sorting::default();
    }
}
