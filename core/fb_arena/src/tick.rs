//! Stepping bodies through a tick, shared by the server's arena, client prediction and bare steps.
use super::*;

/// One body for `tick_bodies`.
pub struct Stepper<'a> {
    pub id: u32,
    pub body: &'a mut Body,
    pub ev: &'a mut StepEvents,
    pub input: BodyInput,
}

/// A navigation grid built ahead, and the static world it was built for (`NavGrid::key`).
pub type PreparedNav = (NavGrid, Fingerprint);

/// The map code a tick of bodies runs (`tick_bodies`): its logic's moving parts and touch handlers, and portals.
pub struct MapRun<'a> {
    pub logic: &'a mut dyn MapLogic,
    pub server: bool,
    /// Events the logic emits are applied at once (the server's arena; not client prediction or a bare step).
    pub apply: bool,
    /// Client: the local player.
    pub me: Option<u32>,
    pub scores: &'a mut BTreeMap<u32, i64>,
    pub out: &'a mut Vec<MapOut>,
}

impl MapRun<'_> {
    fn touch(&mut self, world: &mut World, t: f64, body: &mut Body, ev: &mut StepEvents, tc: Touch) {
        let mut none = NoBodies;
        let mut cx = Cx::new(
            self.server,
            self.apply,
            t,
            self.me,
            world,
            &mut none,
            self.scores,
            self.out,
        );
        if !enter_gate(&mut cx, tc.col, body, ev) {
            self.logic.touch(&mut cx, body, ev, tc);
        }
    }
}

/// A server's map event on a client's map: a portal trip shuts its pair, the rest go to the logic (replies in `out`).
pub fn client_event(
    world: &mut World,
    spec: &mut MapSpec,
    scores: &mut BTreeMap<u32, i64>,
    me: Option<u32>,
    ev: &MapEvent,
    out: &mut Vec<MapOut>,
) {
    if let &MapEvent::Portal { pair, from, t } = ev {
        if (pair as usize) < world.portals.len() {
            world.portal_used(pair as usize, from, t);
        }
        return;
    }
    let mut none = NoBodies;
    let mut cx = Cx::new(false, false, world.t, me, world, &mut none, scores, out);
    spec.logic.event(&mut cx, ev);
}

/// A client's map once built: what the map sets up on a client (decorations).
pub fn client_start(
    world: &mut World,
    spec: &mut MapSpec,
    scores: &mut BTreeMap<u32, i64>,
    me: Option<u32>,
    out: &mut Vec<MapOut>,
) {
    let mut none = NoBodies;
    let mut cx = Cx::new(false, false, world.t, me, world, &mut none, scores, out);
    spec.logic.start(&mut cx);
}

/// The map's line of HUD text for the local player.
pub fn client_hud(
    world: &mut World,
    spec: &MapSpec,
    scores: &mut BTreeMap<u32, i64>,
    me: Option<u32>,
) -> Option<String> {
    let mut out = Vec::new();
    let mut none = NoBodies;
    let cx = Cx::new(false, false, world.t, me, world, &mut none, scores, &mut out);
    spec.logic.hud(&cx)
}

/// Buffers `tick_bodies` reuses from tick to tick (none of it is state).
#[derive(Default)]
pub struct TickScratch {
    carries: Vec<Carry>,
    step: StepScratch,
}

/// One tick of bodies in a world: the shared core of the server arena and client prediction.
/// `extra` are bodies that are not stepped here (on a client: the others, as drawn).
pub fn tick_bodies(world: &mut World, t: f64, bodies: &mut [Stepper], extra: &[OtherBody], map: &mut MapRun) {
    tick_bodies_with(&mut TickScratch::default(), world, t, bodies, extra, &|_, _| None, map);
}

/// `tick_bodies` in a world without map logic (portals still work), as the server would step it.
pub fn tick_plain(world: &mut World, t: f64, bodies: &mut [Stepper], extra: &[OtherBody]) {
    let (mut scores, mut out) = (BTreeMap::new(), Vec::new());
    let mut map = MapRun {
        logic: &mut NoLogic,
        server: true,
        apply: false,
        me: None,
        scores: &mut scores,
        out: &mut out,
    };
    tick_bodies(world, t, bodies, extra, &mut map);
}

/// `tick_bodies` with buffers kept by the caller; `seen(a, b)`: where `a`'s player saw `b` when it acted, if not now.
pub fn tick_bodies_with(
    scratch: &mut TickScratch,
    world: &mut World,
    t: f64,
    bodies: &mut [Stepper],
    extra: &[OtherBody],
    seen: &dyn Fn(u32, u32) -> Option<OtherBody>,
    map: &mut MapRun,
) {
    // Where a body stands on a moving platform is read in the world of the previous tick. The server's
    // world is always there; a client replaying a rollback finds it at its latest prediction instead.
    // At a tick time the world goes by the tick number: (k − 1)·DT is the server's previous time, and
    // k·DT − DT may differ from it in the last bit.
    let k = (t / DT).round();
    let tick = (t.is_finite() && k * DT == t).then_some(k as i64);
    match tick {
        Some(k) => world.goto_tick(k - 1, &*map.logic),
        None => {
            if (world.t - (t - DT)).abs() > 1e-9 {
                world.goto(t - DT, &*map.logic);
            }
        }
    }
    let TickScratch { carries, step } = scratch;
    carries.clear();
    for s in bodies.iter_mut() {
        *s.ev = StepEvents::default();
        carries.push(s.body.before_world_update(world));
    }
    match tick {
        Some(k) => world.goto_tick(k, &*map.logic),
        None => world.goto(t, &*map.logic),
    }
    for (s, c) in bodies.iter_mut().zip(carries.iter()) {
        s.body.after_world_update(c, world);
    }
    for s in bodies.iter_mut() {
        s.body.step(step, s.ev, DT, s.input, world, t, &mut |w, b, e, tc| {
            map.touch(w, t, b, e, tc)
        });
    }
    for i in 0..bodies.len() {
        let (head, tail) = bodies.split_at_mut(i + 1);
        let x = &mut head[i];
        if x.body.in_portal() {
            continue;
        }
        for y in tail.iter_mut().filter(|y| !y.body.in_portal()) {
            let (x_saw, y_saw) = (seen(x.id, y.id), seen(y.id, x.id));
            beans::resolve(
                Side {
                    id: x.id,
                    body: x.body,
                    ev: x.ev,
                },
                Other::Stepped(Side {
                    id: y.id,
                    body: y.body,
                    ev: y.ev,
                }),
                x_saw.as_ref(),
                y_saw.as_ref(),
            );
        }
        for o in extra {
            let x_saw = seen(x.id, o.id);
            beans::resolve(
                Side {
                    id: x.id,
                    body: x.body,
                    ev: x.ev,
                },
                Other::Seen(o),
                x_saw.as_ref(),
                None,
            );
        }
    }
}

/// Which way a bean placed at `p` faces.
pub fn face_yaw(spec: &MapSpec, p: V3) -> f64 {
    if spec.face_center { m::atan2(-p.x, -p.z) } else { 0.0 }
}

/// Moves a standing bean's checkpoint on to the farthest one its progress has passed.
pub fn reach_checkpoint(spec: &MapSpec, body: &Body, checkpoint: &mut Option<usize>) {
    if !body.grounded {
        return;
    }
    for (ci, cp) in spec.checkpoints.iter().enumerate() {
        if body.pos.z >= cp.z && checkpoint.is_none_or(|c| cp.z > spec.checkpoints[c].z) {
            *checkpoint = Some(ci);
        }
    }
}

/// Below the kill line (or nowhere: not a number).
pub fn fell(spec: &MapSpec, pos: V3) -> bool {
    pos.y < spec.kill_y || !pos.is_finite()
}

/// Where a bean that fell (or took a shortcut) comes back: its checkpoint (races) or its spawn; None in the
/// lobby (a free spawn, which depends on where everybody stands).
pub fn respawn_point(
    spec: &MapSpec,
    kind: ArenaKind,
    fall: FallBehaviour,
    checkpoint: Option<usize>,
    spawn_i: usize,
) -> Option<V3> {
    match checkpoint {
        Some(c) if fall == FallBehaviour::Checkpoint => Some(spec.checkpoints[c].p),
        _ if kind == ArenaKind::Lobby => None,
        _ => spec.spawns.get(spawn_i).copied(),
    }
}

/// Puts a bean that fell back at `to`, a little aside by its id (beans respawning together do not stack).
pub fn respawn(spec: &MapSpec, id: u32, body: &mut Body, to: V3) {
    // In u64: client ids use all 32 bits.
    let jitter = ((id as u64 * 7919) % 100) as f64 / 100.0 - 0.5;
    body.reset(V3::new(to.x + jitter * 2.0, to.y + 0.5, to.z), face_yaw(spec, to));
}

pub fn other_of(id: u32, b: &Body) -> OtherBody {
    OtherBody {
        id,
        x: b.pos.x,
        y: b.pos.y,
        z: b.pos.z,
        vx: b.vel.x,
        vy: b.vel.y,
        vz: b.vel.z,
        tilt: b.tilt,
        tilt_dir: b.tilt_dir,
        size: b.size,
    }
}

/// Builds a map's world the way server and clients both do.
pub fn build_map(map: &'static dyn MapDef, seed: u32, with_scene: bool, participants: &[u32]) -> (Builder, MapSpec) {
    let mut b = Builder::new(seed, with_scene);
    let ctx = MapCtx {
        server: !with_scene,
        seed,
        participants,
    };
    let spec = map.build(&mut b, &ctx);
    let bad = spec_problems(&spec);
    assert!(bad.is_empty(), "map {}: {}", map.meta().id, bad.join(", "));
    (b, spec)
}
