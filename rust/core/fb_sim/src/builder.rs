//! The map-building API (`b.cyl`, `b.rotor`, …). The server builds without a scene: only nodes,
//! colliders and movers; the client also gets a `SceneDesc` to draw.
use crate::collider::{ColId, Collider, ColliderOpts, Shape};
use crate::m;
use crate::math::V3;
use crate::nodes::{NodeId, ROOT};
use crate::scene::{Palette, PrimKind, SceneDesc, SceneItem, SceneryRequest, pal};
use crate::world::{MoveCtx, World};
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
}

#[derive(Clone, Copy, Debug)]
pub struct Prim {
    pub node: NodeId,
    pub col: Option<ColId>,
}

pub struct Builder {
    pub world: World,
    pub rng: Rng,
    pub seed: u32,
    pub scene: Option<SceneDesc>,
    /// Candidate spots for bonuses (bonus.rs picks a few per round).
    pub bonus_spots: Vec<V3>,
}

impl Builder {
    pub fn new(seed: u32, with_scene: bool) -> Self {
        Self {
            world: World::default(),
            rng: Rng::new(seed),
            seed,
            scene: with_scene.then(SceneDesc::default),
            bonus_spots: Vec::new(),
        }
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
        let node = self.world.nodes.add(parent, V3::ZERO);
        if let Some(s) = &mut self.scene {
            s.items.push(SceneItem::Model { node, name });
        }
        node
    }

    pub fn collider(&mut self, node: NodeId, shape: Shape, o: ColliderOpts) -> ColId {
        self.world.add(Collider::new(node, shape, o))
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
                freq: o.freq.or(freq).unwrap_or(1.0),
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
    #[allow(clippy::approx_constant, reason = "6.283 as in TS, not TAU")]
    pub fn mushroom(&mut self, x: f64, y: f64, z: f64, scale: f64, power: f64) -> ColId {
        let mush = self.model("mushroom", ROOT);
        let n = self.world.nodes.get_mut(mush);
        n.pos = V3::new(x, y, z);
        n.scale = V3::splat(scale);
        n.rot.y = (x * 1.7 + z * 0.9) % 6.283;
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
    pub fn ladder(&mut self, x: f64, y0: f64, z: f64, y1: f64, yaw: f64) {
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
        let wood = ["#ffb347", "#ffb347"];
        let deco = PrimOpts {
            parent: Some(holder),
            no_collide: true,
            ..Default::default()
        };
        for sx in [-0.45, 0.45] {
            self.box_(sx, (h + 0.7) / 2.0, 0.14, 0.11, h + 0.7, 0.11, wood, deco.clone());
        }
        let rungs = m::round_js(h / 0.38).max(2.0) as u32;
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
        if let Some(s) = &mut self.scene {
            s.scenery.push(SceneryRequest {
                cx,
                cz,
                spread,
                clouds: 26,
                y_min: -30.0,
                y_max: -4.0,
            });
        }
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
