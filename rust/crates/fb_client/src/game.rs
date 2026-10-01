//! The client's copy of the round: the same map built from the seed, own-bean prediction, map events,
//! and the input it sends.
use std::fs::File;
use std::io::{BufWriter, Write};

use bevy::prelude::*;
use fb_arena::{Stepper, build_map, tick_bodies};
use fb_net::*;
use fb_shared::DT;
use fb_shared::input::{BTN_DIVE, BTN_GRAB, BTN_JUMP, InputFrame};
use fb_sim::bonus::{BonusTaken, Bonuses};
use fb_sim::map::{Genre, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::Nodes;
use fb_sim::physics::{BodyInput, BodyState, OtherBody, StepEvents};
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
    /// Prediction steps, replays of rollbacks included.
    pub ticks: u64,
    pub map_events: u32,
    pub hash_mismatch: bool,
}

/// `--trace`: one line per predicted tick, `C tick id mx mz buttons x y z` (rollback replays repeat ticks).
#[derive(Resource)]
struct Trace(BufWriter<File>);

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraAngles { yaw: 0.0, pitch: 0.38 });
        app.init_resource::<Stats>();
        app.init_resource::<Inbox>();
        app.add_systems(Startup, open_trace);
        app.add_systems(PreUpdate, build_round);
        app.add_systems(FixedPreUpdate, write_input.in_set(InputSystems::WriteClientInputs));
        app.add_systems(FixedUpdate, predict);
        app.add_systems(Update, (receive_map_events, apply_map_events).chain());
        app.add_observer(on_controlled);
    }
}

fn open_trace(mut commands: Commands, opts: Res<Opts>) {
    if let Some(path) = &opts.trace {
        let file = File::create(path).unwrap_or_else(|e| panic!("--trace {}: {e}", path.display()));
        commands.insert_resource(Trace(BufWriter::new(file)));
    }
}

fn build_round(mut commands: Commands, rounds: Query<&Round>, map: Option<Res<Map>>, mut stats: ResMut<Stats>) {
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
    if static_hash != round.static_hash {
        // The client would predict against a different map: a bug in determinism, never expected.
        error!("map hash {static_hash} differs from the server's {}", round.static_hash);
        stats.hash_mismatch = true;
    }
    info!(
        "round {}: {} seed {} (static {static_hash})",
        round.number, round.map, round.seed
    );
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
        commands.entity(trigger.entity).insert((
            InputMarker::<FbInput>::default(),
            PrevPos::default(),
            OwnEvents::default(),
        ));
    }
}

/// A slow circle with a hop now and then, and a run for any bonus lying about: exercises prediction and
/// the map event path without a player. Returns (forward, right, buttons) or a world-space stick.
fn autopilot(k: u32, map: Option<&Map>, own: Option<(&PlayerId, &BodyFull)>) -> Result<(f64, f64, u8), InputFrame> {
    // Each player on its own rhythm, so a room of autopilots does not move in lockstep.
    let id = own.map_or(0, |(id, _)| id.0);
    let p = k + id * 53;
    let buttons = if p % (89 + id % 13) < 2 { BTN_JUMP } else { 0 };
    if let (Some(map), Some((_, own))) = (map, own) {
        let p = own.body.pos;
        if let Some(b) = map.bonuses.available(map.time(k as f64)).next() {
            let (dx, dz) = (b.pos.x - p.x, b.pos.z - p.z);
            let l = (dx * dx + dz * dz).sqrt().max(1e-6);
            return Err(InputFrame::from_stick(dx / l, dz / l, buttons));
        }
    }
    let turn = if (p / (200 + 17 * (id % 5))).is_multiple_of(2) {
        0.6
    } else {
        -0.6
    };
    Ok((1.0, turn, buttons))
}

fn keyboard(keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> (f64, f64, u8) {
    let held = |k: &[KeyCode]| k.iter().any(|k| keys.pressed(*k));
    let axis = |pos: &[KeyCode], neg: &[KeyCode]| f64::from(i8::from(held(pos)) - i8::from(held(neg)));
    let f = axis(&[KeyCode::KeyW, KeyCode::ArrowUp], &[KeyCode::KeyS, KeyCode::ArrowDown]);
    let r = axis(
        &[KeyCode::KeyD, KeyCode::ArrowRight],
        &[KeyCode::KeyA, KeyCode::ArrowLeft],
    );
    let mut buttons = 0;
    if held(&[KeyCode::Space]) {
        buttons |= BTN_JUMP;
    }
    if held(&[KeyCode::KeyE, KeyCode::ShiftLeft, KeyCode::ControlLeft]) || mouse.pressed(MouseButton::Left) {
        buttons |= BTN_DIVE;
    }
    if held(&[KeyCode::KeyQ]) || mouse.pressed(MouseButton::Right) {
        buttons |= BTN_GRAB;
    }
    (f, r, buttons)
}

/// The stick relative to the camera, quantized as the server takes it.
fn write_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    cam: Res<CameraAngles>,
    opts: Res<Opts>,
    timeline: Res<LocalTimeline>,
    map: Option<Res<Map>>,
    mut q: Query<(&mut ActionState<FbInput>, Option<(&PlayerId, &BodyFull)>), With<InputMarker<FbInput>>>,
) {
    let Ok((mut state, own)) = q.single_mut() else { return };
    let (f, r, buttons) = if opts.autopilot() {
        match autopilot(timeline.tick().0, map.as_deref(), own) {
            Ok(v) => v,
            Err(frame) => {
                state.0 = frame.into();
                return;
            }
        }
    } else if let (Some(keys), Some(mouse)) = (keys, mouse) {
        keyboard(&keys, &mouse)
    } else {
        (0.0, 0.0, 0)
    };
    let (fx, fz) = (cam.yaw.sin() as f64, cam.yaw.cos() as f64);
    let (mut mx, mut mz) = (f * fx - r * fz, f * fz + r * fx);
    let l = (mx * mx + mz * mz).sqrt();
    if l > 1.0 {
        mx /= l;
        mz /= l;
    }
    state.0 = InputFrame::from_stick(mx, mz, buttons).into();
}

/// Own bean: the same tick as the server's, against the others as drawn.
fn predict(
    timeline: Res<LocalTimeline>,
    map: Option<ResMut<Map>>,
    mut stats: ResMut<Stats>,
    mut own: Query<
        (
            &PlayerId,
            &mut BodyFull,
            &ActionState<FbInput>,
            &mut OwnEvents,
            &mut PrevPos,
        ),
        With<Predicted>,
    >,
    others: Query<(&PlayerId, &RemotePose), (With<Interpolated>, Without<Predicted>)>,
    trace: Option<ResMut<Trace>>,
) {
    let Some(mut map) = map else { return };
    let Ok((id, mut full, state, mut ev, mut prev)) = own.single_mut() else {
        return;
    };
    let k = map.round.arena_tick(timeline.tick());
    let t = k as f64 * DT;
    let frame = InputFrame::from(state.0);
    let input: BodyInput = if t >= 0.0 { frame.into() } else { BodyInput::default() };
    let extra: Vec<OtherBody> = others
        .iter()
        .filter(|(_, p)| p.state != BodyState::Portal as u8)
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
    if let Some(mut trace) = trace {
        let b = &full.body;
        let _ = writeln!(
            trace.0,
            "C {} {} {} {} {} {:.6} {:.6} {:.6}",
            timeline.tick().0,
            id.0,
            frame.mx,
            frame.mz,
            frame.buttons,
            b.pos.x,
            b.pos.y,
            b.pos.z
        );
    }
}

/// Map events as they arrive. They may come before the round they belong to is replicated (the history
/// sent on join, or a round that just started), so they wait here until that round's map is built.
#[derive(Resource, Default)]
struct Inbox(Vec<MapEventMsg>);

fn receive_map_events(
    mut receivers: Query<&mut MessageReceiver<MapEventMsg>>,
    mut inbox: ResMut<Inbox>,
    mut stats: ResMut<Stats>,
) {
    for mut r in &mut receivers {
        for msg in r.receive() {
            inbox.0.push(msg);
            stats.map_events += 1;
        }
    }
}

/// Events about the own bean apply at once (it is drawn ahead); the rest when the others are drawn at their tick.
fn apply_map_events(
    map: Option<ResMut<Map>>,
    mut inbox: ResMut<Inbox>,
    interp: Option<Res<InterpolationTimeline>>,
    own: Query<&PlayerId, With<Predicted>>,
) {
    let Some(mut map) = map else { return };
    let map = &mut *map;
    let round = map.round.number;
    // Older rounds' events are dropped, later rounds' wait for their map.
    inbox.0.retain(|msg| {
        if msg.round == round {
            map.pending.push(*msg);
        }
        msg.round > round
    });
    let me = own.single().ok().map(|p| p.0);
    let now = interp.map(|t| t.tick().0);
    map.pending.retain(|msg| {
        let MapEventKind::Bonus { i, id, at } = msg.ev;
        let due = Some(id) == me || now.is_none_or(|n| n >= msg.tick);
        if due {
            map.bonuses.on_event(BonusTaken { i, id, at });
        }
        !due
    });
}
