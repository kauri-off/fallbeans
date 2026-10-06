//! The map-building API (`b.cyl`, `b.rotor`, …). The server builds without a scene: only nodes,
//! colliders and movers; the client also gets a `SceneDesc` to draw.
use std::sync::Arc;

use crate::bots::Note;
use crate::collider::{ColId, Collider, ColliderOpts, Shape};
use crate::m::{self, MinMax};
use crate::map::{Cx, MapEvent, MapOut, Touches};
use crate::math::V3;
use crate::nodes::{NodeId, ROOT};
use crate::physics::{Body, PORTAL_T, StepEvents, Touch};
use crate::scene::{
    Finish, Form, Look, LookOut, Palette, Part, Piece, PrimKind, SceneDesc, SceneItem, SceneryRequest, pal,
};
use crate::world::{MoveCtx, PORTAL_CLOSED, PortalPair, St, World};
use fb_shared::rng::Rng;

pub const MODEL_NAMES: [&str; 19] = [
    "bean", "crown", "hub", "arm", "hammer", "hex", "door", "finish", "bumper", "cloud", "tree", "pine", "flag",
    "cone", "star", "island", "mushroom", "glove", "fan",
];

#[derive(Clone, Debug, Default)]
pub struct PrimOpts {
    pub col: ColliderOpts,
    pub freq: Option<f64>,
    pub rot: Option<V3>,
    pub parent: Option<NodeId>,
    pub no_collide: bool,
    /// The primitive moves (a mover changes it): its collider is re-read every tick.
    pub dynamic: bool,
    pub seg: Option<u32>,
    /// Surface finish (default: padded for big floors, rubber for balls, plastic otherwise).
    pub surface: Option<&'static str>,
    /// Pattern of a two-colour palette (default: the map's style).
    pub pattern: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct Prim {
    pub node: NodeId,
    pub col: Option<ColId>,
}

impl Prim {
    /// The collider (the primitive was built solid).
    pub fn col(&self) -> ColId {
        self.col.expect("a primitive without a collider")
    }
}

/// A decorative model's placement (`b.prop`).
#[derive(Clone, Copy, Debug, Default)]
pub struct PropOpts {
    pub yaw: f64,
    pub scale: Option<f64>,
    pub tint: Option<&'static str>,
}

/// One end of a pair of portals: a ring standing at (x, y, z), facing `yaw`.
#[derive(Clone, Copy, Debug)]
pub struct PortalEnd {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f64,
}

pub type OpenFn = Arc<dyn Fn(f64) -> bool + Send + Sync>;

#[derive(Clone, Default)]
pub struct PortalOpts {
    /// Only the first end takes beans in; the second only lets them out.
    pub one_way: bool,
    /// Beans come out with at least this speed (m/s; default 6), thrown up at `lift` m/s when given.
    pub speed: Option<f64>,
    pub lift: Option<f64>,
    /// How long (s) the ends stay shut after a trip (default PORTAL_CLOSED).
    pub closed: Option<f64>,
    /// Open only while this says so (a pure function of sim time): shut in between.
    pub open: Option<OpenFn>,
}

/// Shut right now (after a trip, or out of its rhythm)? (Not in the tick it closed: prediction may
/// replay that tick, and must go through again.)
pub fn portal_shut(pair: &PortalPair, open: Option<&OpenFn>, t: f64) -> bool {
    (t > pair.at && t < pair.closed_until) || open.is_some_and(|o| !o(t))
}

pub struct Builder {
    pub world: World,
    pub rng: Rng,
    pub seed: u32,
    pub scene: Option<SceneDesc>,
    /// Candidate spots for bonuses (bonus.rs picks a few per round).
    pub bonus_spots: Vec<V3>,
    /// Touch handlers of colliders (moved into the map's spec after the build).
    pub touches: Touches,
    /// Look of the map (client only): the pattern its palette materials use by default.
    pub pattern: &'static str,
    /// Notes in bots' memory made so far.
    notes: u32,
}

impl Builder {
    pub fn new(seed: u32, with_scene: bool) -> Self {
        Self {
            world: World::default(),
            rng: Rng::new(seed),
            seed,
            scene: with_scene.then(SceneDesc::default),
            bonus_spots: Vec::new(),
            touches: Touches::default(),
            pattern: "stripes",
            notes: 0,
        }
    }

    /// A new note for the map's bots to keep in their memory.
    pub fn note<T>(&mut self) -> Note<T> {
        self.notes += 1;
        Note::new(self.notes)
    }

    pub fn server(&self) -> bool {
        self.scene.is_none()
    }

    /// Collision-relevant motion: a pure function of time, run on server and client.
    pub fn mover(&mut self, f: impl Fn(f64, &mut MoveCtx) + Send + Sync + 'static) {
        self.world.movers.push(Box::new(f));
    }

    pub fn anchor(&mut self, x: f64, y: f64, z: f64, parent: NodeId) -> NodeId {
        self.world.nodes.add(parent, V3::new(x, y, z))
    }

    pub fn model(&mut self, name: &'static str, parent: NodeId) -> NodeId {
        self.model_tinted(name, parent, None)
    }

    fn model_tinted(&mut self, name: &'static str, parent: NodeId, tint: Option<&'static str>) -> NodeId {
        let node = self.world.nodes.add(parent, V3::ZERO);
        if let Some(s) = &mut self.scene {
            s.items.push(SceneItem::Model { node, name, tint });
        }
        node
    }

    /// Something only the client draws, still: pieces of its parts at a node.
    pub fn special(&mut self, node: NodeId, kind: &'static str, parts: &[Part], pieces: Vec<Piece>) {
        if let Some(s) = &mut self.scene {
            s.items.push(SceneItem::Special {
                node,
                kind,
                parts: parts.to_vec(),
                pieces,
                look: None,
            });
        }
    }

    /// Something only the client draws, moving: `look` places its pieces every frame.
    pub fn special_look(
        &mut self,
        node: NodeId,
        kind: &'static str,
        parts: &[Part],
        look: impl Fn(&World, f64, &mut LookOut) + Send + Sync + 'static,
    ) {
        if let Some(s) = &mut self.scene {
            s.items.push(SceneItem::Special {
                node,
                kind,
                parts: parts.to_vec(),
                pieces: Vec::new(),
                look: Some(Look(Arc::new(look))),
            });
        }
    }

    pub fn collider(&mut self, node: NodeId, shape: Shape, o: ColliderOpts) -> ColId {
        self.world.add(Collider::new(node, shape, o))
    }

    /// State the map keeps (movers, logic and bot brains reach it through the handle).
    pub fn state<S: core::any::Any + Send + Sync>(&mut self, s: S) -> St<S> {
        self.world.add_state(s)
    }

    /// `col.onTouch`: called whenever a body touches the collider (once per contact, inside its step).
    pub fn on_touch(
        &mut self,
        col: ColId,
        f: impl FnMut(&mut Cx, &mut Body, &mut StepEvents, Touch) + Send + Sync + 'static,
    ) {
        self.world.colliders[col as usize].on_touch = true;
        self.touches.touch.insert(col, Box::new(f));
    }

    /// `col.onGround`: called on every step a body stands on the collider.
    pub fn on_ground(
        &mut self,
        col: ColId,
        f: impl FnMut(&mut Cx, &mut Body, &mut StepEvents, Touch) + Send + Sync + 'static,
    ) {
        self.world.colliders[col as usize].on_ground = true;
        self.touches.ground.insert(col, Box::new(f));
    }

    fn prim(
        &mut self,
        kind: PrimKind,
        dims: [f64; 3],
        p: Palette,
        x: f64,
        y: f64,
        z: f64,
        o: &PrimOpts,
        freq: Option<f64>,
    ) -> NodeId {
        let node = self.world.nodes.add(o.parent.unwrap_or(ROOT), V3::new(x, y, z));
        if let Some(r) = o.rot {
            self.world.nodes.get_mut(node).rot = r;
        }
        if let Some(s) = &mut self.scene {
            s.items.push(SceneItem::Prim {
                node,
                kind,
                dims,
                pal: p,
                freq: o.freq.or(freq),
                surface: o.surface,
                pattern: o.pattern,
            });
        }
        node
    }

    fn result(&mut self, node: NodeId, shape: Shape, o: PrimOpts) -> Prim {
        if o.no_collide {
            return Prim { node, col: None };
        }
        let col = self.collider(
            node,
            shape,
            ColliderOpts {
                is_static: !o.dynamic,
                ..o.col
            },
        );
        Prim { node, col: Some(col) }
    }

    pub fn box_(&mut self, x: f64, y: f64, z: f64, sx: f64, sy: f64, sz: f64, p: Palette, o: PrimOpts) -> Prim {
        let node = self.prim(PrimKind::Box, [sx, sy, sz], p, x, y, z, &o, None);
        self.result(
            node,
            Shape::Box {
                hx: sx / 2.0,
                hy: sy / 2.0,
                hz: sz / 2.0,
            },
            o,
        )
    }

    pub fn cyl(&mut self, x: f64, y: f64, z: f64, r: f64, h: f64, p: Palette, o: PrimOpts) -> Prim {
        let seg = o.seg.unwrap_or(48) as f64;
        let node = self.prim(PrimKind::Cyl, [r, h, seg], p, x, y, z, &o, None);
        self.result(node, Shape::Cyl { r, hh: h / 2.0 }, o)
    }

    pub fn sphere(&mut self, x: f64, y: f64, z: f64, r: f64, p: Palette, o: PrimOpts) -> Prim {
        let node = self.prim(PrimKind::Sphere, [r, 0.0, 0.0], p, x, y, z, &o, Some(0.6));
        self.result(node, Shape::Sphere { r }, o)
    }

    pub fn ramp(
        &mut self,
        x: f64,
        z0: f64,
        y0: f64,
        z1: f64,
        y1: f64,
        width: f64,
        p: Palette,
        thick: f64,
        o: PrimOpts,
    ) -> Prim {
        let ang = m::atan2(y1 - y0, z1 - z0);
        let len = m::hypot(z1 - z0, y1 - y0);
        self.box_(
            x,
            (y0 + y1) / 2.0 - thick / 2.0 / m::cos(ang),
            (z0 + z1) / 2.0,
            width,
            thick,
            len,
            p,
            PrimOpts {
                rot: Some(V3::new(-ang, 0.0, 0.0)),
                ..o
            },
        )
    }

    pub fn rails(&mut self, z0: f64, z1: f64, half_width: f64, y: f64, p: Palette) {
        let len = (z1 - z0).abs();
        for s in [-1.0, 1.0] {
            self.box_(
                s * (half_width + 0.4),
                y + 0.6,
                (z0 + z1) / 2.0,
                0.8,
                1.2,
                len,
                p,
                PrimOpts::default(),
            );
        }
    }

    pub fn hub(&mut self, x: f64, y: f64, z: f64, scale: f64) {
        let h = self.model("hub", ROOT);
        let n = self.world.nodes.get_mut(h);
        n.pos = V3::new(x, y, z);
        n.scale = V3::splat(scale);
        let a = self.anchor(x, y + 1.6 * scale, z, ROOT);
        self.collider(
            a,
            Shape::Cyl {
                r: 1.1 * scale,
                hh: 1.6 * scale,
            },
            ColliderOpts {
                is_static: true,
                ..Default::default()
            },
        );
    }

    pub fn rotor(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        len: f64,
        count: u32,
        angle: impl Fn(f64) -> f64 + Send + Sync + 'static,
        hit: f64,
    ) -> NodeId {
        let rotor = self.anchor(x, y, z, ROOT);
        for k in 0..count {
            let pivot = self.world.nodes.add(rotor, V3::ZERO);
            self.world.nodes.get_mut(pivot).rot.y = (k as f64 / count as f64) * m::PI * 2.0;
            let arm = self.model("arm", pivot);
            self.world.nodes.get_mut(arm).scale = V3::new(len, 1.0, 1.0);
            let a = self.anchor(len / 2.0 + 0.3, 0.0, 0.0, pivot);
            self.collider(
                a,
                Shape::Box {
                    hx: len / 2.0 - 0.3,
                    hy: 0.36,
                    hz: 0.36,
                },
                ColliderOpts {
                    hit,
                    tag: Some("rotor"),
                    sweep: true,
                    ..Default::default()
                },
            );
        }
        self.mover(move |t, ctx| ctx.node(rotor).rot.y = angle(t));
        rotor
    }

    pub fn bumper(&mut self, x: f64, y: f64, z: f64, s: f64, power: f64) -> NodeId {
        let b = self.model("bumper", ROOT);
        let n = self.world.nodes.get_mut(b);
        n.pos = V3::new(x, y, z);
        n.scale = V3::splat(s);
        let a = self.anchor(x, y + 0.95 * s, z, ROOT);
        self.collider(
            a,
            Shape::Cyl {
                r: 0.9 * s,
                hh: 0.9 * s,
            },
            ColliderOpts {
                is_static: true,
                bounce: power,
                tag: Some("bumper"),
                ..Default::default()
            },
        );
        b
    }

    pub fn hammer(&mut self, x: f64, y: f64, z: f64, speed: f64, phase: f64, amp: f64, with_frame: bool) -> NodeId {
        if with_frame {
            for sx in [-4.4, 4.4] {
                for sz in [-1.5, 1.5] {
                    self.box_(x + sx, y - 3.6, z + sz, 0.8, 8.4, 0.8, pal::PURPLE, PrimOpts::default());
                }
            }
            for sx in [-4.4, 4.4] {
                self.box_(x + sx, y + 0.8, z, 0.8, 0.8, 3.8, pal::PURPLE, PrimOpts::default());
            }
            self.box_(x, y + 0.8, z, 9.6, 0.8, 1.2, pal::PURPLE, PrimOpts::default());
        }
        let h = self.model("hammer", ROOT);
        self.world.nodes.get_mut(h).pos = V3::new(x, y, z);
        let a = self.anchor(0.0, -6.0, 0.0, h);
        self.collider(
            a,
            Shape::Box {
                hx: 1.35,
                hy: 0.95,
                hz: 0.95,
            },
            ColliderOpts {
                hit: 0.9,
                tag: Some("hammer"),
                ..Default::default()
            },
        );
        self.mover(move |t, ctx| ctx.node(h).rot.z = m::sin(t * speed + phase) * amp);
        h
    }

    /// A launch pad: throws beans up (power, m/s), and along `launch` (horizontal m/s) when given.
    pub fn pad(&mut self, x: f64, y: f64, z: f64, r: f64, power: f64, launch: Option<(f64, f64)>) -> Prim {
        self.cyl(
            x,
            y - 0.26,
            z,
            r + 0.2,
            0.6,
            ["#5a3fb8", "#5a3fb8"],
            PrimOpts::default(),
        );
        self.cyl(
            x,
            y + 0.07,
            z,
            r,
            0.2,
            if launch.is_some() { pal::ORANGE } else { pal::TEAL },
            PrimOpts {
                col: ColliderOpts {
                    pad: power,
                    launch: launch.map(|(lx, lz)| V3::new(lx, 0.0, lz)),
                    ..Default::default()
                },
                freq: Some(1.2),
                ..Default::default()
            },
        )
    }

    /// A bouncy mushroom standing at (x, y, z): its cap throws beans up at `power` m/s; the stem is solid.
    pub fn mushroom(&mut self, x: f64, y: f64, z: f64, scale: f64, power: f64, tint: Option<&'static str>) -> ColId {
        let mush = self.model_tinted("mushroom", ROOT, tint);
        let n = self.world.nodes.get_mut(mush);
        n.pos = V3::new(x, y, z);
        n.scale = V3::splat(scale);
        n.rot.y = (x * 1.7 + z * 0.9) % m::TAU;
        let stem = self.anchor(x, y + 0.6 * scale, z, ROOT);
        self.collider(
            stem,
            Shape::Cyl {
                r: 0.42 * scale,
                hh: 0.6 * scale,
            },
            ColliderOpts {
                is_static: true,
                ..Default::default()
            },
        );
        let cap = self.anchor(x, y + 1.66 * scale, z, ROOT);
        self.collider(
            cap,
            Shape::Cyl {
                r: 0.98 * scale,
                hh: 0.26 * scale,
            },
            ColliderOpts {
                is_static: true,
                pad: power,
                ..Default::default()
            },
        )
    }

    /// A ladder up a wall: its foot at (x, y0, z) on the wall's face, up to y1, the rungs facing `yaw`.
    pub fn ladder(&mut self, x: f64, y0: f64, z: f64, y1: f64, yaw: f64, color: &'static str) {
        let h = y1 - y0;
        let holder = self.anchor(x, y0, z, ROOT);
        self.world.nodes.get_mut(holder).rot.y = yaw;
        let a = self.anchor(0.0, h / 2.0, 0.45, holder);
        self.collider(
            a,
            Shape::Box {
                hx: 0.55,
                hy: h / 2.0,
                hz: 0.3,
            },
            ColliderOpts {
                is_static: true,
                ladder: true,
                nav_skip: true,
                ..Default::default()
            },
        );
        if self.server() {
            return;
        }
        let wood = pal::hex(color);
        let deco = PrimOpts {
            parent: Some(holder),
            no_collide: true,
            surface: Some("wood"),
            ..Default::default()
        };
        for sx in [-0.45, 0.45] {
            self.box_(sx, (h + 0.7) / 2.0, 0.14, 0.11, h + 0.7, 0.11, wood, deco.clone());
        }
        let rungs = (h / 0.38).round().at_least(2.0) as u32;
        for k in 1..rungs {
            self.cyl(
                0.0,
                (k as f64 / rungs as f64) * h,
                0.14,
                0.045,
                0.9,
                wood,
                PrimOpts {
                    rot: Some(V3::new(0.0, 0.0, m::PI / 2.0)),
                    seg: Some(8),
                    ..deco.clone()
                },
            );
        }
    }

    /// A trampoline: a springy mat on a ring frame that throws beans up (power: m/s upwards).
    pub fn trampoline(&mut self, x: f64, y: f64, z: f64, r: f64, power: f64) -> Prim {
        let legs = 6;
        for k in 0..legs {
            let a = (k as f64 / legs as f64) * m::PI * 2.0;
            self.cyl(
                x + m::cos(a) * (r + 0.1),
                y - 0.65,
                z + m::sin(a) * (r + 0.1),
                0.09,
                1.2,
                ["#39406b", "#39406b"],
                PrimOpts {
                    no_collide: true,
                    seg: Some(10),
                    ..Default::default()
                },
            );
        }
        self.cyl(x, y - 0.03, z, r + 0.3, 0.3, pal::ORANGE, PrimOpts::default());
        self.cyl(
            x,
            y + 0.14,
            z,
            r,
            0.1,
            pal::BLUE,
            PrimOpts {
                col: ColliderOpts {
                    pad: power,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
    }

    /// A spot where a bonus may lie (on the ground at y).
    pub fn bonus(&mut self, x: f64, y: f64, z: f64) {
        self.bonus_spots.push(V3::new(x, y, z));
    }

    /// Decorative clouds around the course (client only).
    pub fn clouds(&mut self, cx: f64, cz: f64, spread: f64) {
        self.clouds_with(cx, cz, spread, 26, -30.0, -4.0);
    }

    pub fn clouds_with(&mut self, cx: f64, cz: f64, spread: f64, clouds: u32, y_min: f64, y_max: f64) {
        if let Some(s) = &mut self.scene {
            s.scenery.push(SceneryRequest {
                cx,
                cz,
                spread,
                clouds,
                y_min,
                y_max,
            });
        }
    }

    /// A decorative model (client only, no collision): trees, flags, cones, stars, fans, mushrooms.
    pub fn prop(&mut self, name: &'static str, x: f64, y: f64, z: f64, o: PropOpts) -> Option<NodeId> {
        if self.server() {
            return None;
        }
        let node = self.model_tinted(name, ROOT, o.tint);
        let n = self.world.nodes.get_mut(node);
        n.pos = V3::new(x, y, z);
        n.rot.y = o.yaw;
        n.scale = V3::splat(o.scale.unwrap_or(1.0));
        Some(node)
    }

    pub fn finish(&mut self, x: f64, y: f64, z: f64) {
        let f = self.model("finish", ROOT);
        self.world.nodes.get_mut(f).pos = V3::new(x, y, z);
        for sx in [-8.5, 8.5] {
            let a = self.anchor(x + sx, y + 3.0, z, ROOT);
            self.collider(
                a,
                Shape::Cyl { r: 0.6, hh: 3.0 },
                ColliderOpts {
                    is_static: true,
                    ..Default::default()
                },
            );
        }
        // Stars twirling over the arch; flags just past the line (finished beans have left the course).
        for sx in [-5.0, 0.0, 5.0] {
            let star_y = y + 8.2 + if sx != 0.0 { 0.0 } else { 0.6 };
            let scale = Some(if sx != 0.0 { 1.1 } else { 1.5 });
            self.prop(
                "star",
                x + sx,
                star_y,
                z,
                PropOpts {
                    scale,
                    ..Default::default()
                },
            );
        }
        for sx in [-7.0, 7.0] {
            let o = PropOpts {
                tint: Some(if sx < 0.0 { "#ffd23f" } else { "#4fdc6a" }),
                yaw: if sx < 0.0 { m::PI } else { 0.0 },
                scale: None,
            };
            self.prop("flag", x + sx, y, z + 3.0, o);
        }
    }

    /// Start pen with a gate that opens at t = 0; returns 8 spawn points.
    pub fn start_area(&mut self, z0: f64) -> Vec<V3> {
        let d = PrimOpts::default;
        self.box_(0.0, -1.0, z0, 18.0, 2.0, 14.0, pal::PURPLE, d());
        self.box_(-9.4, 0.6, z0, 0.8, 1.2, 14.0, pal::PINK, d());
        self.box_(9.4, 0.6, z0, 0.8, 1.2, 14.0, pal::PINK, d());
        self.box_(0.0, 0.6, z0 - 7.4, 19.6, 1.2, 0.8, pal::PINK, d());
        let gate = self.box_(0.0, 1.8, z0 + 7.1, 18.0, 3.6, 0.4, pal::PINK, d());
        // Flags and cones by the gate (on the rails: nothing to trip over, out of the camera's way).
        let flag = |tint, yaw| PropOpts {
            tint: Some(tint),
            yaw,
            scale: None,
        };
        self.prop("flag", -9.4, 1.2, z0 + 6.4, flag("#ff5fa2", m::PI));
        self.prop("flag", 9.4, 1.2, z0 + 6.4, flag("#3fa9ff", 0.0));
        for sx in [-1.0, 1.0] {
            let o = PropOpts {
                scale: Some(0.9),
                ..Default::default()
            };
            self.prop("cone", sx * 9.4, 1.2, z0 + 4.2, o);
        }
        let (node, col) = (gate.node, gate.col());
        self.mover(move |t, ctx| {
            ctx.set_enabled(col, t < 0.0);
            ctx.node(node).visible = t < 0.0;
        });
        (0..8).map(|i| V3::new(-7.0 + i as f64 * 2.0, 0.05, z0 - 2.0)).collect()
    }

    /// Two linked portals (rings standing up, facing yaw): running into either takes PORTAL_T, out of
    /// sight, and comes out in front of the other, facing its way, with at least 6 m/s. Each trip closes
    /// both ends (solid sashes) until a moment after the traveller is out. One-way: only `a` takes beans
    /// in. Returns the pair's index.
    pub fn portal(&mut self, a: PortalEnd, b: PortalEnd, color: &'static str, o: PortalOpts) -> usize {
        let k = self.world.portals.len();
        self.world.portals.push(PortalPair {
            at: -1e9,
            from: 0,
            closed_until: -1e9,
            close_for: o.closed.unwrap_or(PORTAL_CLOSED),
        });
        let ends = [a, b];
        for (i, e) in ends.iter().enumerate() {
            let other = ends[1 - i];
            let exit_only = o.one_way && i == 1;
            let to = V3::new(
                other.x + m::sin(other.yaw) * 1.9,
                other.y + 0.05,
                other.z + m::cos(other.yaw) * 1.9,
            );
            let ring = self.anchor(e.x, e.y, e.z, ROOT);
            self.world.nodes.get_mut(ring).rot.y = e.yaw;
            if !exit_only {
                let at = self.anchor(0.0, 1.35, 0.0, ring);
                let trigger = self.collider(
                    at,
                    Shape::Box {
                        hx: 1.05,
                        hy: 1.25,
                        hz: 0.3,
                    },
                    ColliderOpts {
                        is_static: true,
                        trigger: true,
                        nav_skip: true,
                        ..Default::default()
                    },
                );
                let (open, speed, lift, yaw) = (o.open.clone(), o.speed.unwrap_or(6.0), o.lift, other.yaw);
                self.on_touch(trigger, move |cx, body, ev, _| {
                    let t = cx.world.t;
                    if body.in_portal() || portal_shut(&cx.world.portals[k], open.as_ref(), t) {
                        return;
                    }
                    body.enter_portal(ev, to, yaw, speed, lift);
                    // Clients hear of it (it shuts, they show the light); not kept for late joiners. A
                    // client's prediction does not shut it: only the server's event does.
                    if cx.server {
                        cx.world.portal_used(k, i as u32, t);
                        cx.out.push(MapOut::Event {
                            ev: MapEvent::Portal {
                                pair: k as u32,
                                from: i as u32,
                                t,
                            },
                            keep: false,
                        });
                    }
                });
                // Shut: the sashes are a wall.
                let at = self.anchor(0.0, 1.35, 0.0, ring);
                let sash = self.collider(
                    at,
                    Shape::Box {
                        hx: 1.25,
                        hy: 1.3,
                        hz: 0.12,
                    },
                    ColliderOpts {
                        is_static: true,
                        nav_skip: true,
                        ..Default::default()
                    },
                );
                self.world.colliders[sash as usize].enabled = false;
                let open = o.open.clone();
                self.mover(move |t, ctx: &mut MoveCtx| {
                    let shut = portal_shut(ctx.portal(k), open.as_ref(), t);
                    ctx.set_enabled(sash, shut);
                });
            }
            if self.server() {
                continue;
            }
            self.portal_look(ring, k, i as u32, exit_only, color, o.open.clone());
            for sx in [-1.0, 1.0] {
                let deco = PrimOpts {
                    no_collide: true,
                    ..Default::default()
                };
                self.box_(
                    e.x + m::cos(e.yaw) * sx * 1.35,
                    e.y + 0.15,
                    e.z - m::sin(e.yaw) * sx * 1.35,
                    0.5,
                    0.3,
                    0.5,
                    pal::hex(color),
                    deco,
                );
            }
        }
        k
    }

    /// A portal end as drawn: frame, swirling disc, sashes while shut, a flash at each trip.
    fn portal_look(
        &mut self,
        ring: NodeId,
        k: usize,
        i: u32,
        exit_only: bool,
        color: &'static str,
        open: Option<OpenFn>,
    ) {
        let parts = [
            Part::new(Form::Torus(1.35, 0.18), color, Finish::Glossy).on("glossy"),
            Part::new(
                if exit_only { Form::Rings(1.2) } else { Form::Swirl(1.2) },
                color,
                Finish::Flat,
            ),
            Part::new(Form::Sash(1.22), color, Finish::Metal).on("metal"),
            Part::new(Form::Sphere(1.0), "#fff6d0", Finish::Light),
            Part::new(Form::Torus(1.35, 0.1), color, Finish::Light),
            Part::new(Form::Arrow, color, Finish::Flat),
        ];
        let kind = if exit_only { "portal-exit" } else { "portal" };
        let spin = if i == 1 { -2.2 } else { 2.2 };
        self.special_look(ring, kind, &parts, move |w, t, out| {
            let pair = &w.portals[k];
            let used = if t < pair.at || t >= pair.closed_until {
                0.0
            } else {
                ((t - pair.at) / 0.15)
                    .at_most((pair.closed_until - t) / 0.2)
                    .at_most(1.0)
            };
            let shut = if exit_only {
                0.0
            } else if open.as_ref().is_some_and(|o| !o(t)) {
                1.0
            } else {
                used
            };
            // How long ago somebody went in here, or came out here.
            let into = pair.from == i;
            let age = if into { t - pair.at } else { t - pair.at - PORTAL_T };
            let span = if into { 0.4 } else { 0.6 };
            let f = if age >= 0.0 && age < span { age / span } else { -1.0 };
            out.pieces.push(Piece::at(0, 0.0, 1.4, 0.0));
            let pulse = 1.0 + m::sin(t * 4.0 + i as f64) * 0.03;
            let disc = Piece::at(1, 0.0, 1.4, 0.0)
                .rot(0.0, 0.0, t * spin)
                .scale(pulse * (1.0 - shut * 0.6))
                .tone(if f >= 0.0 { 1.0 - f } else { 0.0 })
                .alpha(0.85);
            out.pieces.push(disc);
            if exit_only {
                // An arrow on the ground: the way out (nobody goes in here).
                out.pieces
                    .push(Piece::at(5, 0.0, 0.06, 1.3).rot(-m::PI / 2.0, 0.0, 0.0).alpha(0.85));
            }
            // Two half-disc sashes hinged at the rim, sliding shut towards the middle.
            if shut > 0.001 {
                for (x, yaw) in [(-1.22, 0.0), (1.22, m::PI)] {
                    for z in [-0.03, 0.03] {
                        out.pieces
                            .push(Piece::at(2, x, 1.4, z).rot(0.0, yaw, 0.0).axes(shut, 1.0, 1.0));
                    }
                }
            }
            if f >= 0.0 {
                let burst = 1.0 - (1.0 - f) * (1.0 - f) * (1.0 - f);
                let (scale, alpha) = if into {
                    (0.2 + 2.4 * (1.0 - f), 0.95 * (1.0 - f * f))
                } else {
                    (0.3 + 3.2 * burst, 1.0 - f)
                };
                out.pieces.push(Piece::at(3, 0.0, 1.4, 0.0).scale(scale).alpha(alpha));
                if !into {
                    out.pieces
                        .push(Piece::at(4, 0.0, 1.4, 0.0).scale(1.0 + 1.4 * burst).alpha(1.0 - f));
                }
            }
        });
    }

    pub fn ring_spawns(&self, n: u32, radius: f64, y: f64, offset: f64) -> Vec<V3> {
        (0..n)
            .map(|k| {
                let a = (k as f64 / n as f64) * m::PI * 2.0 + offset;
                V3::new(m::cos(a) * radius, y, m::sin(a) * radius)
            })
            .collect()
    }
}
