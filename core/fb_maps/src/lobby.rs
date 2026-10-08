//! Not a game: the playground players run around in between shows.
//!
//! Layout (metres; y up, the plaza in the middle, every zone clear of the others): the plaza (r < 7.5,
//! the spawn ring round the fountain), north the bell tower (a spiral of pillars up, an icy slide down
//! to the east), east trampolines and a high platform with a portal down, south-east the ice rink,
//! south-west blocks to climb, west the spinner, north-west a launch pad; planters round the edge.
use std::collections::BTreeSet;

use fb_shared::{Rgb, rgb};
use fb_sim::bots::{ArenaOpts, arena_brain};
use fb_sim::builder::{Builder, PortalEnd, PortalOpts, PrimOpts, PropOpts};
use fb_sim::collider::{ColliderOpts, Shape};
use fb_sim::looks::Pattern;
use fb_sim::m;
use fb_sim::map::{GameMeta, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::scene::Model;
use fb_sim::scene::Surface;
use fb_sim::scene::{Finish, Form, Palette, Part, Piece, pal};

use crate::util::{deco, o};

pub struct Lobby;

pub static META: GameMeta = GameMeta::place("lobby", "Лобби");

const FLOOR_R: f64 = 24.0;
/// The bell tower: x, z, top, half width.
const TOWER: (f64, f64, f64, f64) = (0.0, 16.0, 6.0, 2.0);
/// Ringing the bell: standing up here after having been down on the floor since the last ring.
const BELL: (f64, f64, f64, f64) = (TOWER.0, TOWER.1, 1.7, TOWER.2 - 0.3);
/// Height of the bell's beam over the tower top.
const BELL_HANG: f64 = 4.0;
const TRAMPS: [(f64, f64); 3] = [(12.5, 0.0), (16.5, -3.0), (16.5, 3.5)];
const PLATFORM: (f64, f64, f64) = (21.0, 0.25, 6.0);
const SPINNER: (f64, f64, f64) = (-14.5, 0.0, 6.0);
const RINK: (f64, f64, f64) = (11.0, -12.5, 5.5);
const BLOCKS: (f64, f64, f64) = (-13.0, -12.0, 2.6);
const PAD: (f64, f64) = (-10.0, 10.0);
const PORTAL_B: (f64, f64) = (-6.5, -10.0);

/// Circles the edge planters keep away from.
const ZONES: [(f64, f64, f64); 5] = [
    (PLATFORM.0, PLATFORM.1, 3.2),
    (RINK.0, RINK.1, RINK.2),
    (BLOCKS.0, BLOCKS.1, 5.3),
    (SPINNER.0, SPINNER.1, SPINNER.2),
    // The foot of the slide, with its flags.
    (TOWER.0 + 12.0, TOWER.1, 3.0),
];

/// A slab from (x0, y0) down to (x1, y1) along x, its top surface on that line.
fn slide(b: &mut Builder, x0: f64, y0: f64, x1: f64, y1: f64, z: f64, width: f64) {
    let ang = m::atan2(y1 - y0, x1 - x0);
    let len = m::hypot(x1 - x0, y1 - y0);
    let thick = 0.5;
    let opts = PrimOpts {
        col: ColliderOpts {
            slip: 1.0,
            ..Default::default()
        },
        rot: Some(V3::new(0.0, 0.0, ang)),
        surface: Some(Surface::Ice),
        ..Default::default()
    };
    b.box_(
        (x0 + x1) / 2.0,
        (y0 + y1) / 2.0 - thick / 2.0 / m::cos(ang),
        z,
        len,
        thick,
        width,
        pal::TEAL,
        opts,
    );
}

/// A painted path on the floor from (x0, z0) to (x1, z1) (no collision).
fn path(b: &mut Builder, x0: f64, z0: f64, x1: f64, z1: f64, p: Palette) {
    let len = m::hypot(x1 - x0, z1 - z0);
    let opts = PrimOpts {
        no_collide: true,
        pattern: Some(Pattern::Chevron),
        rot: Some(V3::new(0.0, m::atan2(x1 - x0, z1 - z0), 0.0)),
        ..Default::default()
    };
    b.box_((x0 + x1) / 2.0, 0.03, (z0 + z1) / 2.0, 2.2, 0.06, len, p, opts);
}

/// The models the lobby makes solid.
#[derive(Clone, Copy)]
enum Solid {
    Tree,
    Pine,
    Mushroom,
    Cone,
    Flag,
}

impl Solid {
    fn model(self) -> Model {
        match self {
            Solid::Tree => Model::Tree,
            Solid::Pine => Model::Pine,
            Solid::Mushroom => Model::Mushroom,
            Solid::Cone => Model::Cone,
            Solid::Flag => Model::Flag,
        }
    }
}

/// A decorative model that is also solid: colliders following the model's shape; mushroom caps bounce.
fn solid(b: &mut Builder, name: Solid, x: f64, y: f64, z: f64, o: PropOpts) {
    b.prop(name.model(), x, y, z, o);
    let s = o.scale;
    let cyl = |b: &mut Builder, r: f64, h: f64, cy: f64, pad: f64| {
        let at = b.anchor(x, y + cy * s, z, ROOT);
        b.collider(
            at,
            Shape::Cyl {
                r: r * s,
                hh: (h / 2.0) * s,
            },
            ColliderOpts {
                is_static: true,
                pad,
                ..Default::default()
            },
        );
    };
    match name {
        Solid::Tree => {
            cyl(b, 0.34, 3.2, 1.6, 0.0);
            let at = b.anchor(x, y + 3.7 * s, z, ROOT);
            let ball = ColliderOpts {
                is_static: true,
                ..Default::default()
            };
            b.collider(at, Shape::Sphere { r: 1.35 * s }, ball);
        }
        Solid::Pine => {
            cyl(b, 0.25, 1.4, 0.7, 0.0);
            cyl(b, 1.3, 1.8, 1.8, 0.0);
            cyl(b, 1.0, 1.6, 2.8, 0.0);
            cyl(b, 0.68, 1.4, 3.7, 0.0);
        }
        Solid::Mushroom => {
            cyl(b, 0.42, 1.3, 0.65, 0.0);
            cyl(b, 1.0, 0.8, 1.58, 13.0);
        }
        Solid::Cone => cyl(b, 0.3, 0.88, 0.44, 0.0),
        Solid::Flag => cyl(b, 0.1, 4.6, 2.3, 0.0),
    }
}

/// A signpost with a board showing what the zone is for (solid, both).
fn sign(b: &mut Builder, x: f64, z: f64, emoji: &'static str, bg: Rgb) {
    // Turned to the plaza, where people come from.
    let yaw = m::atan2(-x, -z);
    let wood = pal::solid(rgb(0x8a6a4f));
    let post = PrimOpts {
        surface: Some(Surface::Wood),
        ..Default::default()
    };
    b.box_(x, 1.25, z, 0.22, 2.5, 0.22, wood, post.clone());
    let board = PrimOpts {
        rot: Some(V3::new(0.0, yaw, 0.0)),
        ..post
    };
    let node = b.box_(x, 3.05, z, 1.5, 1.5, 0.1, wood, board).node;
    let face = [Part::new(Form::Label(1.4, 1.4, emoji), bg, Finish::Matte)];
    let sides = vec![
        Piece::at(0, 0.0, 0.0, 0.06),
        Piece::at(0, 0.0, 0.0, -0.06).rot(0.0, m::PI, 0.0),
    ];
    b.special(node, "sign", &face, sides);
}

impl MapDef for Lobby {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        // ---------------------------------------------------------------- ground and plaza
        let floor = PrimOpts {
            freq: Some(0.3),
            ..Default::default()
        };
        b.cyl(0.0, -1.0, 0.0, FLOOR_R, 2.0, pal::BLUE, floor);
        let plaza = PrimOpts {
            pattern: Some(Pattern::Waves),
            ..deco()
        };
        b.cyl(0.0, 0.05, 0.0, 7.5, 0.1, pal::YELLOW, plaza);
        // Fountain: a basin with water.
        let tile = PrimOpts {
            surface: Some(Surface::Tile),
            ..Default::default()
        };
        b.cyl(0.0, 0.35, 0.0, 1.6, 0.7, pal::WHITE, tile);
        let water = PrimOpts {
            surface: Some(Surface::Glossy),
            ..deco()
        };
        b.cyl(0.0, 0.72, 0.0, 1.35, 0.06, pal::TEAL, water);
        // Flags on little posts round the plaza (between the paths that lead out of it).
        let flag_cols = [rgb(0xff5fa2), rgb(0x3fa9ff), rgb(0xffd23f), rgb(0x4fdc6a)];
        for (k, tint) in flag_cols.into_iter().enumerate() {
            let a = m::PI / 8.0 + (k as f64 * m::PI) / 2.0;
            let x = m::cos(a) * 7.9;
            let z = m::sin(a) * 7.9;
            let post = PrimOpts {
                seg: 16,
                ..Default::default()
            };
            b.cyl(x, 0.15, z, 0.35, 0.3, pal::PURPLE, post);
            let flag = PropOpts {
                tint: Some(tint),
                yaw: -a,
                ..Default::default()
            };
            solid(b, Solid::Flag, x, 0.3, z, flag);
        }

        // ---------------------------------------------------------------- the bell tower (north)
        let (tx, tz, top, half) = TOWER;
        let stripes = PrimOpts {
            pattern: Some(Pattern::Stripes),
            ..Default::default()
        };
        b.box_(tx, top / 2.0, tz, half * 2.0, top, half * 2.0, pal::PURPLE, stripes);
        // Pillars spiral up round the west side: 1 m higher each, the last one a jump from the top.
        let pillar_pals = [pal::GREEN, pal::YELLOW, pal::ORANGE, pal::PINK, pal::RED];
        for (k, p) in pillar_pals.into_iter().enumerate() {
            let a = -m::PI / 2.0 - k as f64 * 0.62;
            let h = k as f64 + 1.0;
            let seg = PrimOpts {
                seg: 32,
                ..Default::default()
            };
            b.cyl(tx + m::cos(a) * 5.0, h / 2.0, tz + m::sin(a) * 5.0, 1.1, h, p, seg);
        }
        // The bell hangs in a frame on top, high enough to walk under.
        let metal = PrimOpts {
            surface: Some(Surface::Metal),
            ..Default::default()
        };
        for sx in [-1.0, 1.0] {
            let y = top + BELL_HANG / 2.0;
            b.box_(tx + sx * 1.75, y, tz, 0.3, BELL_HANG, 0.3, pal::WHITE, metal.clone());
        }
        b.box_(tx, top + BELL_HANG + 0.15, tz, 3.8, 0.3, 0.3, pal::WHITE, metal);
        // (The bell swings a little: a still collider round where it hangs.)
        let at = b.anchor(tx, top + BELL_HANG - 0.85, tz, ROOT);
        let still = ColliderOpts {
            is_static: true,
            ..Default::default()
        };
        b.collider(at, Shape::Cyl { r: 0.75, hh: 0.8 }, still);
        if !b.server() {
            let bell = b.anchor(tx, top + BELL_HANG, tz, ROOT);
            let parts = [
                Part::new(Form::Cyl([0.75, 1.0, 32.0]), rgb(0xffcf3f), Finish::Metal).on(Surface::Gold),
                Part::new(Form::Sphere(0.5), rgb(0xffcf3f), Finish::Metal).on(Surface::Gold),
                Part::new(Form::Sphere(0.18), rgb(0x8a6a4f), Finish::Matte).on(Surface::Rubber),
            ];
            b.special_look(bell, "bell", &parts, |_, t, out| {
                // Swinging a little round where it hangs.
                let a = m::sin(t * 1.7) * 0.12;
                for (part, y) in [(0, -0.75), (1, -0.3), (2, -1.35)] {
                    let p = Piece::at(part, 0.0, y * m::cos(a), y * m::sin(a));
                    out.pieces.push(p.rot(a, 0.0, 0.0));
                }
            });
        }
        // The way down: an icy slide from the top to the east.
        slide(b, tx + half, top, tx + 11.5, 0.0, tz, 3.2);
        let teal_flag = |yaw| PropOpts {
            tint: Some(rgb(0x39e0d0)),
            yaw,
            ..Default::default()
        };
        solid(b, Solid::Flag, tx + 12.5, 0.0, tz - 2.2, teal_flag(m::PI / 2.0));
        solid(b, Solid::Flag, tx + 12.5, 0.0, tz + 2.2, teal_flag(m::PI / 2.0));

        // ---------------------------------------------------------------- trampolines and the high platform (east)
        for (x, z) in TRAMPS {
            b.trampoline(x, 0.0, z, 1.8, 21.0);
        }
        let (px, pz, ptop) = PLATFORM;
        b.box_(px, ptop - 0.4, pz, 4.0, 0.8, 4.5, pal::ORANGE, o());
        let flag = PropOpts {
            tint: Some(rgb(0xff8a3d)),
            yaw: -m::PI / 2.0,
            ..Default::default()
        };
        solid(b, Solid::Flag, px - 1.5, ptop, pz - 1.8, flag);
        // A portal from the platform down to the south-west, and back up.
        let (bx, bz) = PORTAL_B;
        b.portal(
            PortalEnd {
                x: px + 1.3,
                y: ptop,
                z: pz,
                yaw: -m::PI / 2.0,
            },
            PortalEnd {
                x: bx,
                y: 0.0,
                z: bz,
                yaw: m::atan2(-bx, -bz),
            },
            rgb(0xa66bff),
            PortalOpts::default(),
        );

        // ---------------------------------------------------------------- ice rink (south-east)
        let (rx, rz, rr) = RINK;
        let ice = PrimOpts {
            col: ColliderOpts {
                slip: 1.0,
                ..Default::default()
            },
            surface: Some(Surface::Ice),
            ..Default::default()
        };
        b.cyl(rx, 0.06, rz, rr, 0.12, pal::WHITE, ice);
        b.bumper(rx - 1.6, 0.12, rz - 1.4, 0.8, 11.0);
        b.bumper(rx + 2.0, 0.12, rz + 1.5, 0.8, 11.0);
        for k in 0..10 {
            let a = (k as f64 / 10.0) * m::PI * 2.0;
            let cone = PropOpts {
                scale: 0.7,
                ..Default::default()
            };
            solid(
                b,
                Solid::Cone,
                rx + m::cos(a) * (rr + 0.6),
                0.0,
                rz + m::sin(a) * (rr + 0.6),
                cone,
            );
        }

        // ---------------------------------------------------------------- blocks to climb (south-west)
        let heights = [0.6, 1.2, 1.8, 1.2, 2.4, 3.0, 1.8, 3.0, 3.8];
        let block_pals = [
            pal::GREEN,
            pal::TEAL,
            pal::BLUE,
            pal::TEAL,
            pal::PURPLE,
            pal::PINK,
            pal::BLUE,
            pal::PINK,
            pal::RED,
        ];
        let (blx, blz, step) = BLOCKS;
        for i in 0..9 {
            // Low near the plaza, higher towards the edge.
            let x = blx + (1.0 - (i % 3) as f64) * step;
            let z = blz + (1.0 - (i / 3) as f64) * step;
            let h = heights[i];
            let checker = PrimOpts {
                pattern: Some(Pattern::Checker),
                ..Default::default()
            };
            b.box_(x, h / 2.0, z, 2.2, h, 2.2, block_pals[i], checker);
        }

        // ---------------------------------------------------------------- the spinner (west)
        let (sx, sz, sr) = SPINNER;
        let stripes = PrimOpts {
            pattern: Some(Pattern::Stripes),
            ..Default::default()
        };
        b.cyl(sx, 0.25, sz, sr, 0.5, pal::PINK, stripes);
        b.hub(sx, 0.5, sz, 0.9);
        b.rotor(sx, 1.1, sz, 5.2, 2, |t| t * 0.8, 0.6);

        // ---------------------------------------------------------------- launch pad (north-west)
        let (padx, padz) = PAD;
        b.pad(padx, 0.0, padz, 1.3, 16.0, None);
        let mush = |s| PropOpts {
            scale: s,
            ..Default::default()
        };
        solid(b, Solid::Mushroom, padx - 3.0, 0.0, padz + 2.5, mush(1.3));
        solid(b, Solid::Mushroom, padx + 2.4, 0.0, padz + 3.2, mush(0.9));

        // ---------------------------------------------------------------- paths and signs from the plaza
        path(b, 0.0, 7.5, 0.0, 9.6, pal::PURPLE);
        path(b, 7.5, 0.0, 10.4, 0.0, pal::ORANGE);
        path(b, -7.5, 0.0, -8.5, 0.0, pal::PINK);
        path(b, 5.3, -5.3, 7.3, -8.2, pal::WHITE);
        path(b, -5.3, -5.3, -8.8, -8.8, pal::GREEN);
        sign(b, 2.6, 9.6, "🔔", rgb(0xa98bff));
        sign(b, 9.6, 2.6, "🤸", rgb(0xff9f4a));
        sign(b, -7.6, 3.0, "🌀", rgb(0xff8cc8));
        sign(b, 5.0, -7.6, "⛸️", rgb(0x9bdcff));
        sign(b, -8.6, -13.8, "🧗", rgb(0x6fe08a));
        sign(b, padx + 2.2, padz - 1.4, "🚀", rgb(0x39e0d0));

        // ---------------------------------------------------------------- planters round the edge
        let flora = [Solid::Tree, Solid::Pine, Solid::Tree, Solid::Mushroom, Solid::Pine];
        for k in 0..20 {
            let a = (k as f64 / 20.0) * m::PI * 2.0;
            let x = m::cos(a) * 22.8;
            let z = m::sin(a) * 22.8;
            if ZONES.iter().any(|q| m::hypot(x - q.0, z - q.1) < q.2 + 1.2) {
                continue;
            }
            let grass = PrimOpts {
                surface: Some(Surface::Grass),
                seg: 24,
                ..Default::default()
            };
            b.cyl(x, 0.4, z, 0.9, 0.8, pal::GREEN, grass);
            let o = PropOpts {
                scale: 0.8 + ((k * 37) % 5) as f64 * 0.1,
                yaw: k as f64 * 1.7,
                tint: None,
            };
            solid(b, flora[k % flora.len()], x, 0.8, z, o);
        }
        b.clouds(0.0, 0.0, 44.0);

        // Bots potter about the playground: onto the pillars, the trampolines and the pad, round the rink.
        let mut opts = ArenaOpts {
            radius: 16.0,
            ..Default::default()
        };
        opts.retarget = 3.0;
        opts.social = true;
        opts.pois = vec![(0.0, 11.0)];
        opts.pois.extend(TRAMPS);
        opts.pois.extend([
            (padx, padz),
            (rx, rz),
            (sx + 3.0, sz + 2.0),
            (blx + step, blz + step),
            (6.0, 5.0),
            (-5.0, -4.0),
        ]);
        // The bell (server): standing up there after having been down on the floor since the last ring.
        let mut armed: BTreeSet<u32> = BTreeSet::new();
        let tick = move |cx: &mut fb_sim::map::Cx, _t: f64| {
            let (bx, bz, br, by) = BELL;
            let ids = cx.bodies.ids();
            // Whoever left is forgotten.
            armed.retain(|id| ids.contains(id));
            for id in ids {
                let Some(p) = cx.bodies.get(id).map(|b| b.pos) else {
                    continue;
                };
                if p.y < 1.0 {
                    armed.insert(id);
                } else if p.y > by && m::hypot(p.x - bx, p.z - bz) < br && armed.remove(&id) {
                    let v = cx.score(id) + 1;
                    cx.set_score(id, v);
                }
            }
        };
        MapSpec {
            // A ring round the fountain, more places than players: a newcomer always finds a free one.
            spawns: b.ring_spawns(12, 5.0, 0.05, m::PI / 12.0),
            kill_y: -15.0,
            face_center: true,
            view: Some(V3::new(0.0, 2.0, 0.0)),
            tick: ctx.server.then(|| Box::new(tick) as fb_sim::map::OnTick),
            bot: Some(arena_brain(opts)),
            ..Default::default()
        }
    }
}
