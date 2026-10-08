//! The camera: a third-person orbit that pulls in
//! instead of going through walls (the arm stops at `fb_sim` colliders), shakes from hits and hard
//! landings, flies over the course during a round's intro and circles the podium.
use bevy::prelude::*;
use fb_arena::ArenaKind;
use fb_net::*;
use fb_shared::INTRO_S;
use fb_sim::collider::{ColId, Contact};
use fb_sim::math::V3;
use fb_sim::world::World;
use lightyear::prelude::*;

use crate::beans::BeanView;
use crate::game::{CameraAngles, Cue, Map};
use crate::session::Session;
use crate::settings::{Controls, Display};
use crate::view::{MainCamera, Spectate, frame_tick};

/// The final pose follows its target this fast (1/s): barely a frame of lag, but one uneven frame (a
/// prediction correction, mouse input bunched into a frame) does not show as a jolt.
const POSE_RATE: f32 = 45.0;
const DISTANCE: f32 = 8.5;
/// The point looked at is this far above the bean's feet.
const LOOK_UP: f32 = 1.4;
/// The intro fly-over ends this long before the start; then the camera sits behind the bean.
const INTRO_HANDOVER: f32 = 1.4;
/// The intro shot's last seconds glide into the follow camera's pose behind the player's bean.
const INTRO_BLEND: f32 = 2.0;
pub const PITCH_MIN: f32 = -0.55;
pub const PITCH_MAX: f32 = 1.25;
const START_PITCH: f32 = 0.32;

/// A fixed camera (debug, screenshots: `fb/camera`): eye and the point looked at.
#[derive(Resource)]
pub struct CameraOverride {
    pub eye: Vec3,
    pub look: Vec3,
}

#[derive(Resource)]
pub struct Rig {
    smooth_target: Vec3,
    arm: f32,
    follow: Follow,
    /// Colliders near the arm (kept from frame to frame).
    near: Vec<ColId>,
    /// Shake energy (0…1), decays; the offset grows with its square.
    trauma: f32,
    shake_t: f32,
    pose_pos: Vec3,
    pose_look: Vec3,
    handover: f32,
    /// The camera as last placed (scripted shots move it from there).
    pos: Vec3,
    look: Vec3,
    generation: Option<u32>,
    teleports: u32,
    watching: Option<u32>,
    cut_arena: Option<u32>,
    /// The yaw a round starts with: the angles stay there through the intro, whatever the mouse does.
    start_yaw: f32,
}

impl Default for Rig {
    fn default() -> Self {
        Self {
            smooth_target: Vec3::ZERO,
            arm: DISTANCE,
            follow: Follow::Snap,
            near: Vec::new(),
            trauma: 0.0,
            shake_t: 0.0,
            pose_pos: Vec3::ZERO,
            pose_look: Vec3::ZERO,
            handover: 0.0,
            pos: Vec3::new(0.0, 14.0, 22.0),
            look: Vec3::new(0.0, 2.0, 0.0),
            generation: None,
            teleports: 0,
            watching: None,
            cut_arena: None,
            start_yaw: 0.0,
        }
    }
}

/// How the follow camera goes on from the last frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Follow {
    /// Jump straight to the follow pose (a new map, a respawn, another bean to watch).
    Snap,
    /// The last frame was a scripted shot: ease in from it.
    FromShot,
    Following,
}

/// The direction from the looked-at point to the eye.
fn back(cam: &CameraAngles) -> Vec3 {
    let cp = cam.pitch.cos();
    Vec3::new(-cam.yaw.sin() * cp, cam.pitch.sin(), -cam.yaw.cos() * cp)
}

/// Smoothstep of x from `min` to `max`.
pub fn smoothstep(x: f32, min: f32, max: f32) -> f32 {
    let t = ((x - min) / (max - min)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The spawn point of the local player's bean.
fn my_spawn(map: &Map, session: &Session) -> Option<V3> {
    let i = session
        .me
        .and_then(|me| map.info.participants.iter().position(|p| *p == me))
        .unwrap_or(0);
    map.spec.spawns.get(i % map.spec.spawns.len().max(1)).copied()
}

impl Rig {
    fn shake(&mut self, amount: f32) {
        self.trauma = (self.trauma + amount).min(1.0);
    }

    fn snap(&mut self) {
        self.follow = Follow::Snap;
    }

    fn apply_shake(&mut self, pos: &mut Vec3, dt: f32) {
        if self.trauma <= 0.0 {
            return;
        }
        self.shake_t += dt;
        let k = self.trauma * self.trauma * 0.35;
        let t = self.shake_t * 31.0;
        pos.x += (t * 1.1).sin() * (t * 0.37 + 1.0).sin() * k;
        pos.y += (t * 1.3 + 2.0).sin() * (t * 0.41).sin() * k;
        pos.z += (t * 0.9 + 4.0).sin() * (t * 0.53 + 3.0).sin() * k;
        self.trauma = (self.trauma - dt * 1.8).max(0.0);
    }

    /// Where the follow camera wants to be for `focus` (no smoothing, no walls): eye and looked-at point.
    fn follow_pose(cam: &CameraAngles, focus: Vec3) -> (Vec3, Vec3) {
        let look = focus + Vec3::Y * LOOK_UP;
        (look + back(cam) * DISTANCE, look)
    }

    /// The follow camera: the arm marches out from the bean and stops before the first solid collider.
    fn follow(&mut self, cam: &CameraAngles, focus: Vec3, dt: f32, world: Option<&World>) -> (Vec3, Vec3) {
        let target = focus + Vec3::Y * LOOK_UP;
        let mode = core::mem::replace(&mut self.follow, Follow::Following);
        if mode == Follow::FromShot {
            // Leaving a scripted shot: start from the camera as it is, and ease in.
            self.pose_pos = self.pos;
            self.pose_look = self.look;
            self.arm = DISTANCE;
            self.handover = 1.0;
        }
        if mode != Follow::Following {
            self.smooth_target = target;
        } else {
            self.smooth_target = self.smooth_target.lerp(target, 1.0 - (-dt * 14.0).exp());
        }
        let dir = back(cam);
        let mut want = DISTANCE;
        if let Some(world) = world {
            const STEPS: usize = 12;
            let mut hit = Contact::default();
            'arm: for i in 1..=STEPS {
                let d = DISTANCE * i as f32 / STEPS as f32;
                let p = self.smooth_target + dir * d;
                let probe = V3::new(p.x as f64, p.y as f64, p.z as f64);
                world.query(probe.x, probe.z, 0.6, &mut self.near);
                for &ci in &self.near {
                    let c = world.col(ci);
                    if c.enabled && c.opts.hit == 0.0 && !c.opts.trigger && c.contact(probe, 0.3, &mut hit) {
                        want = (d - DISTANCE / STEPS as f32).max(1.2);
                        break 'arm;
                    }
                }
            }
        }
        self.arm = if want < self.arm {
            want
        } else {
            self.arm + (want - self.arm) * (1.0 - (-dt * 4.0).exp())
        };
        let want_pos = self.smooth_target + dir * self.arm;
        if mode == Follow::Snap {
            self.pose_pos = want_pos;
            self.pose_look = self.smooth_target;
        } else {
            self.handover = (self.handover - dt).max(0.0);
            let rate = POSE_RATE + (4.0 - POSE_RATE) * self.handover;
            let k = 1.0 - (-dt * rate).exp();
            self.pose_pos = self.pose_pos.lerp(want_pos, k);
            self.pose_look = self.pose_look.lerp(self.smooth_target, k);
        }
        (self.pose_pos, self.pose_look)
    }

    /// A scripted shot: moves the camera smoothly to `eye`, looking at `look`.
    fn cinematic(&mut self, eye: Vec3, look: Vec3, dt: f32, snap: bool) -> (Vec3, Vec3) {
        let k = if snap { 1.0 } else { 1.0 - (-dt * 3.0).exp() };
        self.pos = self.pos.lerp(eye, k);
        self.look = self.look.lerp(look, k);
        // Hand over to the follow camera without a jump.
        self.follow = Follow::FromShot;
        (self.pos, self.look)
    }
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rig>();
        app.add_systems(Update, (shake, fov));
        app.add_systems(
            PostUpdate,
            place_camera
                .after(crate::beans::place_beans)
                .before(TransformSystems::Propagate),
        );
    }
}

fn shake(mut cues: MessageReader<Cue>, mut rig: ResMut<Rig>, controls: Res<Controls>) {
    for c in cues.read() {
        if !controls.camera_shake {
            continue;
        }
        match *c {
            Cue::Knocked => rig.shake(0.55),
            Cue::Hit => rig.shake(0.3),
            Cue::Bumped(b) => rig.shake((b * 0.02).min(0.25)),
            Cue::Landed(l) if l > 0.55 => rig.shake((l - 0.5) * 0.4),
            _ => {}
        }
    }
}

fn fov(display: Res<Display>, mut q: Query<&mut Projection, With<MainCamera>>) {
    if !display.is_changed() {
        return;
    }
    for mut p in &mut q {
        if let Projection::Perspective(pp) = &mut *p {
            pp.fov = display.fov.clamp(40.0, 110.0).to_radians();
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn place_camera(
    mut rig: ResMut<Rig>,
    mut cam: ResMut<CameraAngles>,
    map: Option<Res<Map>>,
    session: Res<Session>,
    spectate: Res<Spectate>,
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time<Real>>,
    own: Query<(&Transform, &BodyFull), (With<Predicted>, With<BeanView>, Without<MainCamera>)>,
    others: Query<(&PlayerId, &Transform), (With<Interpolated>, With<BeanView>, Without<MainCamera>)>,
    mut camera: Query<&mut Transform, With<MainCamera>>,
    fixed_cam: Option<Res<CameraOverride>>,
) {
    let Ok(mut tf) = camera.single_mut() else { return };
    if let Some(o) = fixed_cam {
        *tf = Transform::from_translation(o.eye).looking_at(o.look, Vec3::Y);
        return;
    }
    let dt = time.delta_secs().min(0.1);
    let Some(map) = map else {
        return;
    };
    let rig = &mut *rig;
    let own = own.single().ok();
    // A new arena: down the course in races, towards the middle in arenas.
    if rig.generation != Some(map.generation) {
        rig.generation = Some(map.generation);
        let spawn = my_spawn(&map, &session);
        let yaw = match spawn {
            Some(p) if map.spec.face_center && map.spec.finish.is_none() => (-p.x).atan2(-p.z) as f32,
            _ => 0.0,
        };
        cam.yaw = yaw;
        rig.start_yaw = yaw;
        cam.pitch = START_PITCH;
        rig.snap();
        rig.teleports = own.map_or(0, |(_, f)| f.teleports);
    }
    // Respawned: look the way the bean faces.
    if let Some((_, f)) = own
        && f.teleports != rig.teleports
    {
        rig.teleports = f.teleports;
        cam.yaw = f.body.yaw as f32;
        rig.start_yaw = cam.yaw;
        rig.snap();
    }
    let focus = match own {
        Some((t, _)) => {
            rig.watching = None;
            t.translation
        }
        None => {
            if rig.watching != spectate.target {
                rig.watching = spectate.target;
                // A new target: cut to it instead of sliding the camera across the map.
                rig.snap();
            }
            spectate
                .target
                .and_then(|id| others.iter().find(|(p, _)| p.0 == id))
                .map(|(_, t)| t.translation)
                .or_else(|| map.spec.view.map(|v| v.as_vec3()))
                .unwrap_or(Vec3::ZERO)
        }
    };
    let t = map.time(frame_tick(&timeline, &fixed)) as f32;
    let cut = rig.cut_arena != Some(map.round.arena);
    let late = session
        .arena
        .as_ref()
        .is_some_and(|a| a.id == map.round.arena && a.late);
    let (mut eye, look) = if map.round.kind == ArenaKind::Podium {
        rig.cut_arena = Some(map.round.arena);
        let a = (t * 0.25).sin() * 0.55;
        let eye = Vec3::new(a.sin() * 12.5, 4.6, a.cos() * 12.5);
        rig.cinematic(eye, Vec3::new(0.0, 2.6, 0.0), dt, cut)
    } else if map.round.kind == ArenaKind::Round && !late && t <= -INTRO_HANDOVER {
        rig.cut_arena = Some(map.round.arena);
        cam.yaw = rig.start_yaw;
        cam.pitch = START_PITCH;
        let (mut eye, mut look) = intro_shot(&map, &session, t);
        if let Some((tf, _)) = own {
            let h = smoothstep(t, -INTRO_HANDOVER - INTRO_BLEND, -INTRO_HANDOVER);
            if h > 0.0 {
                let (fe, fl) = Rig::follow_pose(&cam, tf.translation);
                eye = eye.lerp(fe, h);
                look = look.lerp(fl, h);
            }
        }
        rig.cinematic(eye, look, dt, cut)
    } else {
        rig.follow(&cam, focus, dt, Some(&map.world))
    };
    rig.pos = eye;
    rig.look = look;
    rig.apply_shake(&mut eye, dt);
    *tf = Transform::from_translation(eye).looking_at(look, Vec3::Y);
}

/// The intro fly-over: from above the finish back to the start line in races, a circle over arenas.
fn intro_shot(map: &Map, session: &Session, t: f32) -> (Vec3, Vec3) {
    let spec = &map.spec;
    let intro = INTRO_S as f32;
    let f = ((t + intro) / (intro - INTRO_HANDOVER)).clamp(0.0, 1.0);
    let e = f * f * (3.0 - 2.0 * f);
    let start = my_spawn(map, session).map_or(Vec3::ZERO, |p| p.as_vec3());
    let lerp = f32::lerp;
    if let Some(fin) = &spec.finish {
        let (fz, fy) = (fin.z as f32, fin.y as f32);
        let ez = lerp(fz + 14.0, start.z - 9.0, e);
        // Stay well above the course (it climbs on some maps) until the last moment.
        let along = ((ez - start.z) / (fz - start.z)).clamp(0.0, 1.0);
        let course_y = lerp(start.y, fy, along);
        let above = course_y + 5.0 + 8.0 * (1.0 - smoothstep(e, 0.75, 1.0));
        let eye = Vec3::new(lerp(10.0, 0.0, e), lerp(fy + 16.0, start.y + 5.0, e).max(above), ez);
        let look = Vec3::new(0.0, lerp(fy, start.y + 1.0, e), lerp(fz - 20.0, start.z + 6.0, e));
        (eye, look)
    } else {
        let c = spec.view.map_or(Vec3::ZERO, |v| v.as_vec3());
        let a = t * 0.35;
        let r = lerp(30.0, 20.0, e);
        let eye = Vec3::new(c.x + a.sin() * r, c.y + lerp(18.0, 11.0, e), c.z + a.cos() * r);
        (eye, c)
    }
}
