//! The map-building API (`b.cyl`, `b.rotor`, …). The server builds without a scene: only nodes,
//! colliders and movers; the client also gets a `SceneDesc` to draw.
use std::ops::Range;
use std::sync::Arc;

use fb_shared::cause::Hazard;
use fb_shared::rng::Rng;
use fb_shared::{NEVER, Rgb, rgb};

use crate::bots::Note;
use crate::collider::{ColId, ColliderOpts, Shape};
use crate::looks::Pattern;
use crate::m::{self, MinMax};
use crate::map::{Cx, Hook, MapEvent, MapOut};
use crate::math::{V3, v3};
use crate::nodes::{NodeId, ROOT};
use crate::physics::{Body, PORTAL_T, StepEvents};
use crate::scene::Surface;
use crate::scene::{
    Finish, Form, Look, LookOut, Model, Palette, Part, Piece, PrimKind, SceneDesc, SceneItem, SceneryRequest, pal,
};
use crate::world::{MoveCtx, PORTAL_CLOSED, PortalGate, PortalPair, World};

#[derive(Clone, Copy, Debug)]
pub struct PrimOpts {
    pub col: ColliderOpts,
    pub freq: Option<f64>,
    pub rot: Option<V3>,
    pub parent: Option<NodeId>,
    pub no_collide: bool,
    /// The primitive moves (a mover changes it): its collider is re-read every tick.
    pub dynamic: bool,
    /// Sides of a cylinder.
    pub seg: u32,
    /// Surface finish (default: padded for big floors, rubber for balls, plastic otherwise).
    pub surface: Option<Surface>,
    /// Pattern of a two-colour palette (default: the map's style).
    pub pattern: Option<Pattern>,
}

impl Default for PrimOpts {
    fn default() -> Self {
        Self {
            col: ColliderOpts::default(),
            freq: None,
            rot: None,
            parent: None,
            no_collide: false,
            dynamic: false,
            seg: 48,
            surface: None,
            pattern: None,
        }
    }
}

/// A slab `thick` deep and `width` wide at `x`, rising from (z0, y0) to (z1, y1) along its top.
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    pub x: f64,
    pub z0: f64,
    pub y0: f64,
    pub z1: f64,
    pub y1: f64,
    pub width: f64,
    pub thick: f64,
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
#[derive(Clone, Copy, Debug)]
pub struct PropOpts {
    pub yaw: f64,
    pub scale: f64,
    pub tint: Option<Rgb>,
}

impl Default for PropOpts {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            scale: 1.0,
            tint: None,
        }
    }
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

#[derive(Clone)]
pub struct PortalOpts {
    /// Only the first end takes beans in; the second only lets them out.
    pub one_way: bool,
    /// Beans come out with at least this speed (m/s), thrown up at `lift` m/s when given.
    pub speed: f64,
    pub lift: Option<f64>,
    /// How long (s) the ends stay shut after a trip.
    pub closed: f64,
    /// Open only while this says so (a pure function of sim time): shut in between.
    pub open: Option<OpenFn>,
}

impl Default for PortalOpts {
    fn default() -> Self {
        Self {
            one_way: false,
            speed: 6.0,
            lift: None,
            closed: PORTAL_CLOSED,
            open: None,
        }
    }
}

/// Shut right now (after a trip, or out of its rhythm)? (Not in the tick it closed: prediction may
/// replay that tick, and must go through again.)
pub fn portal_shut(pair: &PortalPair, open: Option<&OpenFn>, t: f64) -> bool {
    (t > pair.at && t < pair.closed_until) || open.is_some_and(|o| !o(t))
}

/// A body ran into a portal's gate: in it goes unless the portal is shut. False: `col` is no gate.
pub fn enter_gate(cx: &mut Cx, col: ColId, body: &mut Body, ev: &mut StepEvents) -> bool {
    let t = cx.world.t;
    let Some(g) = cx.world.gates.get(&col) else {
        return false;
    };
    let (k, end) = (g.pair, g.end);
    if body.in_portal() || portal_shut(&cx.world.portals[k], g.open.as_ref(), t) {
        return true;
    }
    body.enter_portal(ev, g.to, g.yaw, g.speed, g.lift);
    // Clients hear of it (it shuts, they show the light); not kept for late joiners. A client's prediction
    // does not shut it: only the server's event does.
    if cx.server {
        cx.world.portal_used(k, end, t);
        cx.out.push(MapOut::Event {
            ev: MapEvent::Portal {
                pair: u32::try_from(k).expect("fewer than 2³² portals"),
                from: end,
                t,
            },
            keep: false,
        });
    }
    true
}

pub struct Builder {
    pub world: World,
    pub rng: Rng,
    pub seed: u32,
    pub scene: Option<SceneDesc>,
    /// Candidate spots for bonuses (bonus.rs picks a few per round).
    pub bonus_spots: Vec<V3>,
    /// Notes in bots' memory made so far.
    notes: u32,
    /// Hooks made so far.
    hooks: u32,
}

impl Builder {
    pub fn new(seed: u32, with_scene: bool) -> Self {
        Self {
            world: World::default(),
            rng: Rng::new(seed),
            seed,
            scene: with_scene.then(SceneDesc::default),
            bonus_spots: Vec::new(),
            notes: 0,
            hooks: 0,
        }
    }

    /// A new note for the map's bots to keep in their memory.
    pub fn note<T>(&mut self) -> Note<T> {
        self.notes += 1;
        Note::new(self.notes)
    }

    /// A new hook for the map's logic (a look it draws, a waypoint it steers bots by).
    pub fn hook(&mut self) -> Hook {
        self.hooks += 1;
        Hook(self.hooks)
    }

    pub fn server(&self) -> bool {
        self.scene.is_none()
    }

    /// Collision-relevant motion: a pure function of time, run on server and client.
    pub fn mover(&mut self, f: impl Fn(f64, &mut MoveCtx) + Send + Sync + 'static) {
        self.world.movers.push(Box::new(f));
    }

    pub fn anchor(&mut self, at: V3, parent: NodeId) -> NodeId {
        self.world.nodes.add(parent, at)
    }

    pub fn model(&mut self, name: Model, parent: NodeId) -> NodeId {
        self.model_tinted(name, parent, None)
    }

    fn model_tinted(&mut self, name: Model, parent: NodeId, tint: Option<Rgb>) -> NodeId {
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
                look: Some(Look::Fn(Arc::new(look))),
            });
        }
    }

    /// Something only the client draws, following the map's state (`MapLogic::look`, by the hook).
    pub fn special_hook(&mut self, node: NodeId, kind: &'static str, parts: &[Part]) -> Hook {
        let hook = self.hook();
        if let Some(s) = &mut self.scene {
            s.items.push(SceneItem::Special {
                node,
                kind,
                parts: parts.to_vec(),
                pieces: Vec::new(),
                look: Some(Look::Map(hook)),
            });
        }
        hook
    }

    pub fn collider(&mut self, node: NodeId, shape: Shape, o: ColliderOpts) -> ColId {
        self.world.add(node, shape, o)
    }

    /// `MapLogic::touch` hears whenever a body touches the collider (once per contact, inside its step).
    pub fn watch_touch(&mut self, col: ColId) {
        self.world.colliders[col as usize].opts.on_touch = true;
    }

    /// `MapLogic::touch` hears on every step a body stands on the collider.
    pub fn watch_ground(&mut self, col: ColId) {
        self.world.colliders[col as usize].opts.on_ground = true;
    }

    fn prim(&mut self, kind: PrimKind, dims: [f64; 3], p: Palette, at: V3, o: &PrimOpts, freq: Option<f64>) -> NodeId {
        let node = self.world.nodes.add(o.parent.unwrap_or(ROOT), at);
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

    pub fn box_(&mut self, at: V3, size: V3, p: Palette, o: PrimOpts) -> Prim {
        let node = self.prim(PrimKind::Box, size.to_array(), p, at, &o, None);
        self.result(
            node,
            Shape::Box {
                hx: size.x / 2.0,
                hy: size.y / 2.0,
                hz: size.z / 2.0,
            },
            o,
        )
    }

    pub fn cyl(&mut self, at: V3, r: f64, h: f64, p: Palette, o: PrimOpts) -> Prim {
        let seg = f64::from(o.seg);
        let node = self.prim(PrimKind::Cyl, [r, h, seg], p, at, &o, None);
        self.result(node, Shape::Cyl { r, hh: h / 2.0 }, o)
    }

    pub fn sphere(&mut self, at: V3, r: f64, p: Palette, o: PrimOpts) -> Prim {
        let node = self.prim(PrimKind::Sphere, [r, 0.0, 0.0], p, at, &o, Some(0.6));
        self.result(node, Shape::Sphere { r }, o)
    }

    pub fn ramp(&mut self, ramp: Ramp, p: Palette, o: PrimOpts) -> Prim {
        let Ramp {
            x,
            z0,
            y0,
            z1,
            y1,
            width,
            thick,
        } = ramp;
        let ang = m::atan2(y1 - y0, z1 - z0);
        let len = m::hypot(z1 - z0, y1 - y0);
        self.box_(
            v3(x, (y0 + y1) / 2.0 - thick / 2.0 / m::cos(ang), (z0 + z1) / 2.0),
            v3(width, thick, len),
            p,
            PrimOpts {
                rot: Some(V3::new(-ang, 0.0, 0.0)),
                ..o
            },
        )
    }

    pub fn rails(&mut self, z0: f64, z1: f64, half_width: f64, y: f64, p: Palette) {
        self.fence(z0..z1, half_width, y, 1.2, false, p);
    }

    /// Side walls `h` high along `z`; `no_grab`: their top edge cannot be climbed.
    pub fn fence(&mut self, z: Range<f64>, half_width: f64, y: f64, h: f64, no_grab: bool, p: Palette) {
        let (z0, z1) = (z.start, z.end);
        let len = (z1 - z0).abs();
        for s in [-1.0, 1.0] {
            self.box_(
                v3(s * (half_width + 0.4), y + h / 2.0, (z0 + z1) / 2.0),
                v3(0.8, h, len),
                p,
                PrimOpts {
                    col: ColliderOpts {
                        no_grab,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
        }
    }

    pub fn hub(&mut self, at: V3, scale: f64) {
        let V3 { x, y, z } = at;
        let h = self.model(Model::Hub, ROOT);
        let n = self.world.nodes.get_mut(h);
        n.pos = at;
        n.scale = V3::splat(scale);
        let a = self.anchor(v3(x, y + 1.6 * scale, z), ROOT);
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
        at: V3,
        len: f64,
        count: u32,
        angle: impl Fn(f64) -> f64 + Send + Sync + 'static,
        hit: f64,
    ) -> NodeId {
        let rotor = self.anchor(at, ROOT);
        for k in 0..count {
            let pivot = self.world.nodes.add(rotor, V3::ZERO);
            self.world.nodes.get_mut(pivot).rot.y = (f64::from(k) / f64::from(count)) * m::PI * 2.0;
            let arm = self.model(Model::Arm, pivot);
            self.world.nodes.get_mut(arm).scale = V3::new(len, 1.0, 1.0);
            let a = self.anchor(v3(len / 2.0 + 0.3, 0.0, 0.0), pivot);
            self.collider(
                a,
                Shape::Box {
                    hx: len / 2.0 - 0.3,
                    hy: 0.36,
                    hz: 0.36,
                },
                ColliderOpts {
                    hit,
                    tag: Some(Hazard::Rotor),
                    sweep: true,
                    ..Default::default()
                },
            );
        }
        self.mover(move |t, ctx| ctx.node(rotor).rot.y = angle(t));
        rotor
    }

    pub fn bumper(&mut self, at: V3, s: f64, power: f64) -> NodeId {
        let V3 { x, y, z } = at;
        let b = self.model(Model::Bumper, ROOT);
        let n = self.world.nodes.get_mut(b);
        n.pos = at;
        n.scale = V3::splat(s);
        let a = self.anchor(v3(x, y + 0.95 * s, z), ROOT);
        self.collider(
            a,
            Shape::Cyl {
                r: 0.9 * s,
                hh: 0.9 * s,
            },
            ColliderOpts {
                is_static: true,
                bounce: power,
                tag: Some(Hazard::Bumper),
                ..Default::default()
            },
        );
        b
    }

    pub fn hammer(&mut self, at: V3, speed: f64, phase: f64, amp: f64, with_frame: bool) -> NodeId {
        let V3 { x, y, z } = at;
        if with_frame {
            for sx in [-4.4, 4.4] {
                for sz in [-1.5, 1.5] {
                    self.box_(
                        v3(x + sx, y - 3.6, z + sz),
                        v3(0.8, 8.4, 0.8),
                        pal::PURPLE,
                        PrimOpts::default(),
                    );
                }
            }
            for sx in [-4.4, 4.4] {
                self.box_(
                    v3(x + sx, y + 0.8, z),
                    v3(0.8, 0.8, 3.8),
                    pal::PURPLE,
                    PrimOpts::default(),
                );
            }
            self.box_(v3(x, y + 0.8, z), v3(9.6, 0.8, 1.2), pal::PURPLE, PrimOpts::default());
        }
        let h = self.model(Model::Hammer, ROOT);
        self.world.nodes.get_mut(h).pos = at;
        let a = self.anchor(v3(0.0, -6.0, 0.0), h);
        self.collider(
            a,
            Shape::Box {
                hx: 1.35,
                hy: 0.95,
                hz: 0.95,
            },
            ColliderOpts {
                hit: 0.9,
                tag: Some(Hazard::Hammer),
                ..Default::default()
            },
        );
        self.mover(move |t, ctx| ctx.node(h).rot.z = m::sin(t * speed + phase) * amp);
        h
    }

    /// A launch pad: throws beans up (power, m/s), and along `launch` (horizontal m/s) when given.
    pub fn pad(&mut self, at: V3, r: f64, power: f64, launch: Option<(f64, f64)>) -> Prim {
        let V3 { x, y, z } = at;
        self.cyl(
            v3(x, y - 0.26, z),
            r + 0.2,
            0.6,
            [rgb(0x5a3fb8), rgb(0x5a3fb8)],
            PrimOpts::default(),
        );
        self.cyl(
            v3(x, y + 0.07, z),
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
    pub fn mushroom(&mut self, at: V3, scale: f64, power: f64, tint: Option<Rgb>) -> ColId {
        let V3 { x, y, z } = at;
        let mush = self.model_tinted(Model::Mushroom, ROOT, tint);
        let n = self.world.nodes.get_mut(mush);
        n.pos = at;
        n.scale = V3::splat(scale);
        n.rot.y = (x * 1.7 + z * 0.9) % m::TAU;
        let stem = self.anchor(v3(x, y + 0.6 * scale, z), ROOT);
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
        let cap = self.anchor(v3(x, y + 1.66 * scale, z), ROOT);
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
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a ladder's rungs, at least 2"
    )]
    pub fn ladder(&mut self, x: f64, y0: f64, z: f64, y1: f64, yaw: f64, color: Rgb) {
        let h = y1 - y0;
        let holder = self.anchor(v3(x, y0, z), ROOT);
        self.world.nodes.get_mut(holder).rot.y = yaw;
        let a = self.anchor(v3(0.0, h / 2.0, 0.45), holder);
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
        let wood = pal::solid(color);
        let deco = PrimOpts {
            parent: Some(holder),
            no_collide: true,
            surface: Some(Surface::Wood),
            ..Default::default()
        };
        for sx in [-0.45, 0.45] {
            self.box_(v3(sx, (h + 0.7) / 2.0, 0.14), v3(0.11, h + 0.7, 0.11), wood, deco);
        }
        let rungs = (h / 0.38).round().at_least(2.0) as u32;
        for k in 1..rungs {
            self.cyl(
                v3(0.0, (f64::from(k) / f64::from(rungs)) * h, 0.14),
                0.045,
                0.9,
                wood,
                PrimOpts {
                    rot: Some(V3::new(0.0, 0.0, m::PI / 2.0)),
                    seg: 8,
                    ..deco
                },
            );
        }
    }

    /// A trampoline: a springy mat on a ring frame that throws beans up (power: m/s upwards).
    pub fn trampoline(&mut self, at: V3, r: f64, power: f64) -> Prim {
        let V3 { x, y, z } = at;
        let legs = 6;
        for k in 0..legs {
            let a = (f64::from(k) / f64::from(legs)) * m::PI * 2.0;
            self.cyl(
                v3(x + m::cos(a) * (r + 0.1), y - 0.65, z + m::sin(a) * (r + 0.1)),
                0.09,
                1.2,
                [rgb(0x39406b), rgb(0x39406b)],
                PrimOpts {
                    no_collide: true,
                    seg: 10,
                    ..Default::default()
                },
            );
        }
        self.cyl(v3(x, y - 0.03, z), r + 0.3, 0.3, pal::ORANGE, PrimOpts::default());
        self.cyl(
            v3(x, y + 0.14, z),
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
    pub fn bonus(&mut self, at: V3) {
        self.bonus_spots.push(at);
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
    pub fn prop(&mut self, name: Model, at: V3, o: PropOpts) -> Option<NodeId> {
        if self.server() {
            return None;
        }
        let node = self.model_tinted(name, ROOT, o.tint);
        let n = self.world.nodes.get_mut(node);
        n.pos = at;
        n.rot.y = o.yaw;
        n.scale = V3::splat(o.scale);
        Some(node)
    }

    pub fn finish(&mut self, at: V3) {
        let V3 { x, y, z } = at;
        let f = self.model(Model::Finish, ROOT);
        self.world.nodes.get_mut(f).pos = at;
        for sx in [-8.5, 8.5] {
            let a = self.anchor(v3(x + sx, y + 3.0, z), ROOT);
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
            let scale = if sx != 0.0 { 1.1 } else { 1.5 };
            self.prop(
                Model::Star,
                v3(x + sx, star_y, z),
                PropOpts {
                    scale,
                    ..Default::default()
                },
            );
        }
        for sx in [-7.0, 7.0] {
            let o = PropOpts {
                tint: Some(if sx < 0.0 { rgb(0xffd23f) } else { rgb(0x4fdc6a) }),
                yaw: if sx < 0.0 { m::PI } else { 0.0 },
                ..Default::default()
            };
            self.prop(Model::Flag, v3(x + sx, y, z + 3.0), o);
        }
    }

    /// Start pen with a gate that opens at t = 0; returns 8 spawn points.
    pub fn start_area(&mut self, z0: f64) -> Vec<V3> {
        let d = PrimOpts::default;
        self.box_(v3(0.0, -1.0, z0), v3(18.0, 2.0, 14.0), pal::PURPLE, d());
        self.box_(v3(-9.4, 0.6, z0), v3(0.8, 1.2, 14.0), pal::PINK, d());
        self.box_(v3(9.4, 0.6, z0), v3(0.8, 1.2, 14.0), pal::PINK, d());
        self.box_(v3(0.0, 0.6, z0 - 7.4), v3(19.6, 1.2, 0.8), pal::PINK, d());
        // Gone by the time bots plan (t = 0): the grid leaves it out, so the one built during the intro holds.
        let no_nav = PrimOpts {
            col: ColliderOpts {
                nav_skip: true,
                ..Default::default()
            },
            ..d()
        };
        let gate = self.box_(v3(0.0, 1.8, z0 + 7.1), v3(18.0, 3.6, 0.4), pal::PINK, no_nav);
        // Flags and cones by the gate (on the rails: nothing to trip over, out of the camera's way).
        let flag = |tint, yaw| PropOpts {
            tint: Some(tint),
            yaw,
            ..Default::default()
        };
        self.prop(Model::Flag, v3(-9.4, 1.2, z0 + 6.4), flag(rgb(0xff5fa2), m::PI));
        self.prop(Model::Flag, v3(9.4, 1.2, z0 + 6.4), flag(rgb(0x3fa9ff), 0.0));
        for sx in [-1.0, 1.0] {
            let o = PropOpts {
                scale: 0.9,
                ..Default::default()
            };
            self.prop(Model::Cone, v3(sx * 9.4, 1.2, z0 + 4.2), o);
        }
        let (node, col) = (gate.node, gate.col());
        self.mover(move |t, ctx| {
            ctx.set_enabled(col, t < 0.0);
            ctx.node(node).visible = t < 0.0;
        });
        (0..8)
            .map(|i| V3::new(-7.0 + f64::from(i) * 2.0, 0.05, z0 - 2.0))
            .collect()
    }

    /// Two linked portals (rings standing up, facing yaw): running into either takes PORTAL_T, out of
    /// sight, and comes out in front of the other, facing its way, with at least 6 m/s. Each trip closes
    /// both ends (solid sashes) until a moment after the traveller is out. One-way: only `a` takes beans
    /// in. Returns the pair's index.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "options built for the call, like the other builders'"
    )]
    pub fn portal(&mut self, a: PortalEnd, b: PortalEnd, color: Rgb, o: PortalOpts) -> usize {
        let k = self.world.portals.len();
        self.world.portals.push(PortalPair {
            at: NEVER,
            from: 0,
            closed_until: NEVER,
            close_for: o.closed,
        });
        let ends = [a, b];
        for (i, e) in ends.iter().enumerate() {
            let end = u32::from(i == 1);
            let other = ends[1 - i];
            let exit_only = o.one_way && i == 1;
            let to = V3::new(
                other.x + m::sin(other.yaw) * 1.9,
                other.y + 0.05,
                other.z + m::cos(other.yaw) * 1.9,
            );
            let ring = self.anchor(v3(e.x, e.y, e.z), ROOT);
            self.world.nodes.get_mut(ring).rot.y = e.yaw;
            if !exit_only {
                let at = self.anchor(v3(0.0, 1.35, 0.0), ring);
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
                self.world.colliders[trigger as usize].opts.on_touch = true;
                let gate = PortalGate {
                    pair: k,
                    end,
                    to,
                    yaw: other.yaw,
                    speed: o.speed,
                    lift: o.lift,
                    open: o.open.clone(),
                };
                self.world.gates.insert(trigger, gate);
                // Shut: the sashes are a wall.
                let at = self.anchor(v3(0.0, 1.35, 0.0), ring);
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
            self.portal_look(ring, k, end, exit_only, color, o.open.clone());
            for sx in [-1.0, 1.0] {
                let deco = PrimOpts {
                    no_collide: true,
                    ..Default::default()
                };
                self.box_(
                    v3(
                        e.x + m::cos(e.yaw) * sx * 1.35,
                        e.y + 0.15,
                        e.z - m::sin(e.yaw) * sx * 1.35,
                    ),
                    v3(0.5, 0.3, 0.5),
                    pal::solid(color),
                    deco,
                );
            }
        }
        k
    }

    /// A portal end as drawn: frame, swirling disc, sashes while shut, a flash at each trip.
    fn portal_look(&mut self, ring: NodeId, k: usize, i: u32, exit_only: bool, color: Rgb, open: Option<OpenFn>) {
        let parts = [
            Part::new(Form::Torus(1.35, 0.18), color, Finish::Glossy).on(Surface::Glossy),
            Part::new(
                if exit_only { Form::Rings(1.2) } else { Form::Swirl(1.2) },
                color,
                Finish::Flat,
            ),
            Part::new(Form::Sash(1.22), color, Finish::Metal).on(Surface::Metal),
            Part::new(Form::Sphere(1.0), rgb(0xfff6d0), Finish::Light),
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
            let pulse = 1.0 + m::sin(t * 4.0 + f64::from(i)) * 0.03;
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
                let a = (f64::from(k) / f64::from(n)) * m::PI * 2.0 + offset;
                V3::new(m::cos(a) * radius, y, m::sin(a) * radius)
            })
            .collect()
    }
}
