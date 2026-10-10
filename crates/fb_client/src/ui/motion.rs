//! Motion: nodes slide or pop into place through their `UiTransform`, eased on the real clock.
use bevy::prelude::*;
use bevy::ui::UiSystems;

pub struct MotionPlugin;

impl Plugin for MotionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, (reveal, animate, spin).chain().before(UiSystems::Prepare));
    }
}

/// How a node comes in: from an offset (px) and a scale, to where it is laid out.
#[derive(Component, Clone, Copy, Debug)]
pub struct Motion {
    pub from: Vec2,
    pub scale: f32,
    pub dur: f32,
    pub delay: f32,
    /// Overshoots before it settles (a pop).
    pub bounce: bool,
    age: f32,
}

impl Motion {
    pub const fn slide(x: f32, y: f32) -> Self {
        Self {
            from: Vec2::new(x, y),
            scale: 1.0,
            dur: 0.28,
            delay: 0.0,
            bounce: false,
            age: 0.0,
        }
    }

    pub const fn pop(scale: f32) -> Self {
        Self {
            from: Vec2::ZERO,
            scale,
            dur: 0.36,
            delay: 0.0,
            bounce: true,
            age: 0.0,
        }
    }

    pub const fn after(mut self, delay: f32) -> Self {
        self.delay = delay;
        self
    }

    pub const fn lasting(mut self, dur: f32) -> Self {
        self.dur = dur;
        self
    }

    fn at(&self) -> (Vec2, f32) {
        let t = ((self.age - self.delay) / self.dur).clamp(0.0, 1.0);
        let k = if self.bounce { back_out(t) } else { cubic_out(t) };
        (self.from * (1.0 - k), self.scale + (1.0 - self.scale) * k)
    }
}

fn cubic_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn back_out(t: f32) -> f32 {
    const C: f32 = 1.70158;
    let u = t - 1.0;
    1.0 + (C + 1.0) * u.powi(3) + C * u.powi(2)
}

/// Plays its motion each time the node is shown (its `display` turned on).
#[derive(Component, Clone, Copy, Debug)]
pub struct Reveal {
    motion: Motion,
    shown: bool,
}

impl Reveal {
    pub const fn new(motion: Motion) -> Self {
        Self { motion, shown: false }
    }
}

/// Turns for ever (a spinner).
#[derive(Component)]
pub struct Spin;

fn reveal(mut q: Query<(Entity, &Node, &mut Reveal), Changed<Node>>, mut commands: Commands) {
    for (e, node, mut r) in &mut q {
        let on = node.display != bevy::ui::Display::None;
        if on && !r.shown {
            commands.entity(e).insert(r.motion);
        }
        r.shown = on;
    }
}

fn animate(time: Res<Time<Real>>, mut q: Query<(Entity, &mut Motion, &mut UiTransform)>, mut commands: Commands) {
    // (A long frame, the first after a load, does not skip the motion.)
    let dt = time.delta_secs().min(1.0 / 30.0);
    for (e, mut m, mut tf) in &mut q {
        let (off, k) = m.at();
        tf.translation = Val2::px(off.x, off.y);
        tf.scale = Vec2::splat(k);
        m.age += dt;
        if m.age >= m.delay + m.dur {
            tf.translation = Val2::ZERO;
            tf.scale = Vec2::ONE;
            commands.entity(e).remove::<Motion>();
        }
    }
}

fn spin(time: Res<Time<Real>>, mut q: Query<&mut UiTransform, With<Spin>>) {
    let turn = Rot2::radians(time.elapsed_secs() * core::f32::consts::TAU * 1.1);
    for mut tf in &mut q {
        tf.rotation = turn;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_motion_starts_at_its_offset_and_ends_in_place() {
        let mut m = Motion::slide(40.0, 0.0);
        assert_eq!(m.at(), (Vec2::new(40.0, 0.0), 1.0));
        m.age = m.dur;
        assert_eq!(m.at(), (Vec2::ZERO, 1.0));
    }

    #[test]
    fn a_pop_overshoots_then_settles() {
        let mut m = Motion::pop(0.5);
        m.age = m.dur * 0.7;
        assert!(m.at().1 > 1.0);
        m.age = m.dur;
        assert!((m.at().1 - 1.0).abs() < 1e-6);
    }
}
