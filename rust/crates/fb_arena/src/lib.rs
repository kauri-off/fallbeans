//! The authoritative arena (phase 0 subset of `server/rooms/arena.ts`): pawns, bonuses, falls.
use fb_shared::input::InputFrame;
use fb_shared::{DT, m};
use fb_sim::bonus::{BonusTaken, Bonuses};
use fb_sim::builder::Builder;
use fb_sim::map::{Genre, MapCtx, MapDef, MapSpec, spec_problems};
use fb_sim::math::V3;
use fb_sim::physics::{Body, BodyInput, OtherBody, StepEvents};
use fb_sim::scene::SceneDesc;
use fb_sim::world::World;

pub struct Pawn {
    pub id: u32,
    pub body: Body,
    pub ev: StepEvents,
    pub spawn: V3,
    /// Respawned this tick: clients snap instead of smoothing.
    pub teleported: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MapEvent {
    Bonus(BonusTaken),
}

pub struct Arena {
    pub map: &'static dyn MapDef,
    pub seed: u32,
    pub world: World,
    pub spec: MapSpec,
    pub bonuses: Bonuses,
    pub pawns: Vec<Pawn>,
    /// Arena tick: sim time is tick × DT, negative during the intro.
    pub tick: i64,
    pub static_hash: String,
    spawn_cursor: usize,
}

/// One body for `tick_bodies`.
pub struct Stepper<'a> {
    pub id: u32,
    pub body: &'a mut Body,
    pub ev: &'a mut StepEvents,
    pub input: BodyInput,
}

/// One tick of bodies in a world: the shared core of the server arena and client prediction.
/// `extra` are bodies that are not stepped here (on a client: the others, as drawn).
pub fn tick_bodies(world: &mut World, t: f64, bodies: &mut [Stepper], extra: &[OtherBody]) {
    let mut carries = Vec::with_capacity(bodies.len());
    for s in bodies.iter_mut() {
        *s.ev = StepEvents::default();
        carries.push(s.body.before_world_update(world));
    }
    world.goto(t);
    for (s, c) in bodies.iter_mut().zip(&carries) {
        s.body.after_world_update(c, world);
    }
    let mut others: Vec<OtherBody> = bodies
        .iter()
        .filter(|s| !s.body.in_portal())
        .map(|s| other_of(s.id, s.body))
        .chain(extra.iter().copied())
        .collect();
    let mut rest = Vec::with_capacity(others.len());
    for s in bodies.iter_mut() {
        rest.clear();
        rest.extend(others.iter().copied().filter(|o| o.id != s.id));
        s.body.step(s.ev, DT, s.input, world, t, &mut rest);
    }
    others.clear();
}

pub fn other_of(id: u32, b: &Body) -> OtherBody {
    OtherBody {
        id,
        x: b.pos.x,
        y: b.pos.y,
        z: b.pos.z,
        vx: b.vel.x,
        vz: b.vel.z,
        touching: false,
        size: b.size,
    }
}

/// Builds a map's world the way server and clients both do.
pub fn build_map(map: &'static dyn MapDef, seed: u32, with_scene: bool) -> (Builder, MapSpec) {
    let mut b = Builder::new(seed, with_scene);
    let ctx = MapCtx {
        server: !with_scene,
        seed,
        participants: &[],
    };
    let spec = map.build(&mut b, &ctx);
    let bad = spec_problems(&spec);
    assert!(bad.is_empty(), "map {}: {}", map.meta().id, bad.join(", "));
    (b, spec)
}

impl Arena {
    pub fn new(map: &'static dyn MapDef, seed: u32, tick: i64, with_scene: bool) -> (Self, Option<SceneDesc>) {
        let (mut b, spec) = build_map(map, seed, with_scene);
        let meta = map.meta();
        let bonuses = Bonuses::new(&b.bonus_spots, seed, meta.genre != Genre::Race, meta.duration);
        b.world.finalize(tick as f64 * DT);
        let static_hash = b.world.hash(true);
        (
            Self {
                map,
                seed,
                world: b.world,
                spec,
                bonuses,
                pawns: Vec::new(),
                tick,
                static_hash,
                spawn_cursor: 0,
            },
            b.scene,
        )
    }

    pub fn time(&self) -> f64 {
        self.tick as f64 * DT
    }

    fn face_yaw(&self, p: V3) -> f64 {
        if self.spec.face_center { m::atan2(-p.x, -p.z) } else { 0.0 }
    }

    pub fn add_pawn(&mut self, id: u32) -> &mut Pawn {
        let spawns = &self.spec.spawns;
        let spawn = spawns[self.spawn_cursor % spawns.len()];
        self.spawn_cursor += 1;
        let mut body = Body::new(id as i32);
        body.reset(spawn, self.face_yaw(spawn));
        let at = self.pawns.partition_point(|p| p.id < id);
        self.pawns.insert(
            at,
            Pawn {
                id,
                body,
                ev: StepEvents::default(),
                spawn,
                teleported: true,
            },
        );
        &mut self.pawns[at]
    }

    pub fn remove_pawn(&mut self, id: u32) {
        self.pawns.retain(|p| p.id != id);
    }

    pub fn pawn(&self, id: u32) -> Option<&Pawn> {
        self.pawns.iter().find(|p| p.id == id)
    }

    /// Runs tick k with each pawn's input frame (before the start nobody moves).
    pub fn step(&mut self, k: i64, frame: impl Fn(u32) -> InputFrame) -> Vec<MapEvent> {
        self.tick = k;
        let t = k as f64 * DT;
        let moving = t >= 0.0;
        let mut steppers: Vec<Stepper> = self
            .pawns
            .iter_mut()
            .map(|p| {
                p.teleported = false;
                Stepper {
                    id: p.id,
                    input: if moving { frame(p.id).into() } else { BodyInput::default() },
                    body: &mut p.body,
                    ev: &mut p.ev,
                }
            })
            .collect();
        tick_bodies(&mut self.world, t, &mut steppers, &[]);
        let mut events = Vec::new();
        if t >= 0.0 {
            let mut bodies: Vec<(u32, &mut Body)> = self.pawns.iter_mut().map(|p| (p.id, &mut p.body)).collect();
            events.extend(self.bonuses.check(t, &mut bodies).into_iter().map(MapEvent::Bonus));
        }
        for i in 0..self.pawns.len() {
            if self.pawns[i].body.pos.y < self.spec.kill_y {
                let to = self.pawns[i].spawn;
                let yaw = self.face_yaw(to);
                let p = &mut self.pawns[i];
                let jitter = ((p.id * 7919) % 100) as f64 / 100.0 - 0.5;
                p.body.reset(V3::new(to.x + jitter * 2.0, to.y + 0.5, to.z), yaw);
                p.teleported = true;
            }
        }
        events
    }

    /// Hash of every bean's state (replays and determinism checks compare it), as in TS.
    pub fn state_hash(&self) -> String {
        let mut h: u32 = 2_166_136_261;
        let mut mix = |v: f64| {
            let x = m::to_i32(m::round_js(v * 1e5)) as u32;
            h = (h ^ (x & 0xffff)).wrapping_mul(16_777_619);
            h = (h ^ (x >> 16)).wrapping_mul(16_777_619);
        };
        for p in &self.pawns {
            mix(p.id as f64);
            let b = &p.body;
            for v in [b.pos.x, b.pos.y, b.pos.z, b.vel.x, b.vel.y, b.vel.z, b.yaw, b.tilt] {
                mix(v);
            }
        }
        format!("{h:08x}")
    }
}
