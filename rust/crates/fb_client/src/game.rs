//! The client's copy of the round: the same map built from the seed, own-bean prediction, map events.
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use fb_arena::{Stepper, build_map, tick_bodies};
use fb_net::*;
use fb_shared::DT;
use fb_shared::input::{BTN_DIVE, BTN_GRAB, BTN_JUMP, InputFrame};
use fb_sim::bonus::{BonusTaken, Bonuses};
use fb_sim::map::{Genre, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::Nodes;
use fb_sim::physics::{BodyInput, OtherBody, StepEvents};
use fb_sim::scene::SceneDesc;
use fb_sim::world::World;
use lightyear::input::native::prelude::{ActionState, InputMarker};
use lightyear::prelude::client::input::InputSystems;
use lightyear::prelude::*;

use crate::opts::Opts;

/// The map of the current round as this client built it.
#[derive(Resource)]
pub struct Map {
    pub round: Round,
    pub world: World,
    pub spec: MapSpec,
    pub scene: SceneDesc,
    pub bonuses: Bonuses,
    /// Nodes posed for drawing (at the frame's time, not the tick's).
    pub render: Nodes,
    pub static_hash: String,
    /// Map events waiting for their tick on the interpolated timeline.
    pub pending: Vec<MapEventMsg>,
    pub generation: u32,
}

impl Map {
    pub fn time(&self, tick: f64) -> f64 {
        (tick - self.round.zero_tick as f64) * DT
    }
}

/// Camera yaw (radians; forward is (sin, cos) on x/z) and pitch.
#[derive(Resource)]
pub struct CameraAngles {
    pub yaw: f32,
    pub pitch: f32,
}

/// The own bean's position before the last tick (frames are drawn between ticks).
#[derive(Component, Default)]
pub struct PrevPos(pub V3);

#[derive(Component, Default)]
pub struct OwnEvents(pub StepEvents);

#[derive(Resource, Default)]
pub struct Stats {
    pub ticks: u64,
    pub bonus_events: u32,
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraAngles { yaw: 0.0, pitch: 0.38 });
        app.init_resource::<Stats>();
        app.add_systems(PreUpdate, build_round);
        app.add_systems(FixedPreUpdate, write_input.in_set(InputSystems::WriteClientInputs));
        app.add_systems(FixedUpdate, predict);
        app.add_systems(Update, (receive_map_events, apply_map_events, mouse_look));
        app.add_observer(on_controlled);
    }
}

fn build_round(mut commands: Commands, rounds: Query<&Round>, map: Option<Res<Map>>) {
    let Some(round) = rounds.iter().next() else { return };
    if map.as_ref().is_some_and(|m| m.round == *round) {
        return;
    }
    let Some(def) = fb_maps::by_id(&round.map) else {
        error!("unknown map {}", round.map);
        return;
    };
    let (mut b, spec) = build_map(def, round.seed, true);
    let meta = def.meta();
    let bonuses = Bonuses::new(&b.bonus_spots, round.seed, meta.genre != Genre::Race, meta.duration);
    b.world.finalize(-1e3);
    let static_hash = b.world.hash(true);
    info!("round {}: {} seed {} (static {static_hash})", round.number, round.map, round.seed);
    let render = b.world.nodes.clone();
    commands.insert_resource(Map {
        round: round.clone(),
        world: b.world,
        spec,
        scene: b.scene.unwrap_or_default(),
        bonuses,
        render,
        static_hash,
        pending: Vec::new(),
        generation: map.map_or(0, |m| m.generation + 1),
    });
}

fn on_controlled(trigger: On<Add, Controlled>, mut commands: Commands, pawns: Query<(), With<PlayerId>>) {
    if pawns.get(trigger.entity).is_ok() {
        commands
            .entity(trigger.entity)
            .insert((InputMarker::<FbInput>::default(), PrevPos::default(), OwnEvents::default()));
    }
}

/// Stick relative to the camera, quantized as the server takes it.
fn write_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cam: Res<CameraAngles>,
    opts: Res<Opts>,
    timeline: Res<LocalTimeline>,
    map: Option<Res<Map>>,
    mut q: Query<(&mut ActionState<FbInput>, Option<&BodyFull>), With<InputMarker<FbInput>>>,
) {
    let Ok((mut state, own)) = q.single_mut() else { return };
    let (mut f, mut r) = (0.0f64, 0.0f64);
    let mut buttons = 0;
    if opts.autopilot {
        // A slow circle with a hop now and then: enough to exercise prediction without a player.
        let k = timeline.tick().0;
        f = 1.0;
        r = 0.6 * ((k / 240) % 2) as f64 * 2.0 - 0.6;
        if k % 97 < 2 {
            buttons |= BTN_JUMP;
        }
        // A bonus lying about: run for it (exercises the map event path).
        if let (Some(map), Some(own)) = (map.as_ref(), own) {
            let t = map.time(k as f64);
            let p = own.body.pos;
            if let Some(b) = map.bonuses.available(t).next() {
                let (dx, dz) = (b.pos.x - p.x, b.pos.z - p.z);
                let l = (dx * dx + dz * dz).sqrt().max(1e-6);
                state.0 = InputFrame::from_stick(dx / l, dz / l, buttons).into();
                return;
            }
        }
    } else {
        let p = |k: &[KeyCode]| k.iter().any(|k| keys.pressed(*k));
        if p(&[KeyCode::KeyW, KeyCode::ArrowUp]) {
            f += 1.0;
        }
        if p(&[KeyCode::KeyS, KeyCode::ArrowDown]) {
            f -= 1.0;
        }
        if p(&[KeyCode::KeyD, KeyCode::ArrowRight]) {
            r += 1.0;
        }
        if p(&[KeyCode::KeyA, KeyCode::ArrowLeft]) {
            r -= 1.0;
        }
        if p(&[KeyCode::Space]) {
            buttons |= BTN_JUMP;
        }
        if p(&[KeyCode::KeyE, KeyCode::ShiftLeft, KeyCode::ControlLeft]) || mouse.pressed(MouseButton::Left) {
            buttons |= BTN_DIVE;
        }
        if p(&[KeyCode::KeyQ]) || mouse.pressed(MouseButton::Right) {
            buttons |= BTN_GRAB;
        }
    }
    let (fx, fz) = (cam.yaw.sin() as f64, cam.yaw.cos() as f64);
    let (mut mx, mut mz) = (f * fx - r * fz, f * fz + r * fx);
    let l = (mx * mx + mz * mz).sqrt();
    if l > 1.0 {
        mx /= l;
        mz /= l;
    }
    state.0 = InputFrame::from_stick(mx, mz, buttons).into();
}

fn mouse_look(
    mut motion: MessageReader<MouseMotion>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cam: ResMut<CameraAngles>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut cursor) = cursor.single_mut() else { return };
    if mouse.just_pressed(MouseButton::Left) && cursor.grab_mode == CursorGrabMode::None {
        cursor.grab_mode = if cfg!(target_os = "windows") { CursorGrabMode::Confined } else { CursorGrabMode::Locked };
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
    if cursor.grab_mode == CursorGrabMode::None {
        motion.clear();
        return;
    }
    for m in motion.read() {
        cam.yaw -= m.delta.x * 0.004;
        cam.pitch = (cam.pitch + m.delta.y * 0.003).clamp(-0.2, 1.2);
    }
}

/// Own bean: the same tick as the server's, against the others as drawn.
fn predict(
    timeline: Res<LocalTimeline>,
    map: Option<ResMut<Map>>,
    mut stats: ResMut<Stats>,
    mut own: Query<(&PlayerId, &mut BodyFull, &ActionState<FbInput>, &mut OwnEvents, &mut PrevPos), With<Predicted>>,
    others: Query<(&PlayerId, &RemotePose), (With<Interpolated>, Without<Predicted>)>,
) {
    let Some(mut map) = map else { return };
    let Ok((id, mut full, state, mut ev, mut prev)) = own.single_mut() else { return };
    let k = map.round.arena_tick(timeline.tick());
    let t = k as f64 * DT;
    let input: BodyInput = if t >= 0.0 { InputFrame::from(state.0).into() } else { BodyInput::default() };
    let extra: Vec<OtherBody> = others
        .iter()
        .filter(|(_, p)| p.state != fb_sim::physics::BodyState::Portal as u8)
        .map(|(o, p)| OtherBody {
            id: o.0,
            x: p.pos.x as f64,
            y: p.pos.y as f64,
            z: p.pos.z as f64,
            vx: p.vel.x as f64,
            vz: p.vel.y as f64,
            touching: false,
            size: p.size as f64,
        })
        .collect();
    let full = &mut *full;
    prev.0 = full.body.pos;
    let mut steppers = [Stepper {
        id: id.0,
        body: &mut full.body,
        ev: &mut ev.0,
        input,
    }];
    tick_bodies(&mut map.world, t, &mut steppers, &extra);
    stats.ticks += 1;
    if std::env::var_os("FB_TRACE").is_some() {
        let f = InputFrame::from(state.0);
        let b = &full.body;
        eprintln!("C {} {} {} {} {:.6} {:.6} {:.6}", timeline.tick().0, f.mx, f.mz, f.buttons, b.pos.x, b.pos.y, b.pos.z);
    }
}

fn receive_map_events(mut receivers: Query<&mut MessageReceiver<MapEventMsg>>, map: Option<ResMut<Map>>, mut stats: ResMut<Stats>) {
    let Some(mut map) = map else {
        for mut r in &mut receivers {
            r.receive().for_each(drop);
        }
        return;
    };
    for mut r in &mut receivers {
        for msg in r.receive() {
            if msg.round == map.round.number {
                map.pending.push(msg);
                stats.bonus_events += 1;
            }
        }
    }
}

/// Events about the own bean apply at once (it is drawn ahead); the rest when the others are drawn at their tick.
fn apply_map_events(map: Option<ResMut<Map>>, interp: Option<Res<InterpolationTimeline>>, own: Query<&PlayerId, With<Predicted>>) {
    let Some(mut map) = map else { return };
    let me = own.single().ok().map(|p| p.0);
    let now = interp.map(|t| t.tick().0);
    let map = &mut *map;
    map.pending.retain(|msg| {
        let MapEventKind::Bonus { i, id, at } = msg.ev;
        let due = Some(id) == me || now.is_none_or(|n| n >= msg.tick);
        if due {
            map.bonuses.on_event(BonusTaken { i, id, at });
        }
        !due
    });
}
