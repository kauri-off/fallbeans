//! Plates drop two or three at a time (a quick shake, then gone), over a big arena; the barriers turning
//! over it knock over whoever they catch and pass over them. The centre never drops.
use std::sync::Arc;

use fb_shared::rng::shuffle;
use fb_sim::bots::{ArenaOpts, arena_brain};
use fb_sim::builder::Builder;
use fb_sim::collider::ColId;
use fb_sim::m;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::{NodeId, ROOT};
use fb_sim::props::{SpinUp, arm_contact_eta};
use fb_sim::scene::{Palette, Tint, pal};

use crate::util::{dynamic, o};

pub struct PlateDrop;

static META: GameMeta = GameMeta {
    finale: true,
    ..GameMeta::new(
        "plate-drop",
        "Падающие плиты",
        Genre::Survival,
        "Плиты обрушиваются по две-три сразу и быстро, а над ними крутятся балки-шлагбаумы, сбивающие с ног. Продержитесь дольше всех!",
        "Продержитесь дольше всех",
        100.0,
    )
};

const PLATE: f64 = 3.6;
const GAP: f64 = 0.25;
const N: i32 = 9;
/// Warning (shaking, reddening) before a plate drops, and how fast it falls away.
const WARN: f64 = 1.1;
const DROP: f64 = 48.0;

struct Plate {
    x: f64,
    z: f64,
    fall_at: f64,
    node: NodeId,
    col: ColId,
    base: V3,
}

impl MapDef for PlateDrop {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["candy", "ocean", "circus"]
    }

    fn build(&self, b: &mut Builder, _ctx: &MapCtx) -> MapSpec {
        let mut plates: Vec<Plate> = Vec::new();
        let pals: [Palette; 4] = [pal::PURPLE, pal::BLUE, pal::PINK, pal::TEAL];
        let half = (N - 1) as f64 / 2.0;
        for i in 0..N {
            for k in 0..N {
                let x = (i as f64 - half) * (PLATE + GAP);
                let z = (k as f64 - half) * (PLATE + GAP);
                if m::hypot(x, z) > (half + 0.6) * (PLATE + GAP) {
                    continue;
                }
                if i as f64 == half && k as f64 == half {
                    continue;
                }
                let p = pals[((i + k) % 4) as usize];
                let prim = b.box_(x, -0.5, z, PLATE, 1.0, PLATE, p, dynamic());
                plates.push(Plate {
                    x,
                    z,
                    fall_at: f64::INFINITY,
                    node: prim.node,
                    col: prim.col(),
                    base: V3::new(x, -0.5, z),
                });
            }
        }
        // The centre never falls: it carries the pillar with the beams.
        b.box_(0.0, -0.5, 0.0, PLATE, 1.0, PLATE, pal::YELLOW, o());
        b.hub(0.0, 0.0, 0.0, 0.9);
        let reach = (half + 0.5) * (PLATE + GAP);
        let dir = if b.rng.next() < 0.5 { 1.0 } else { -1.0 };
        let low = SpinUp::new(-0.22, 0.8 + b.rng.next() * 0.2, 0.004);
        let low_angle = move |t: f64| dir * low.angle(t);
        let high_at = 35.0 + b.rng.next() * 15.0;
        let high_angle = move |t: f64| {
            if t <= high_at {
                return 0.0;
            }
            let d = t - high_at;
            -dir * (0.6 * d + 0.003 * (d * d))
        };
        b.rotor(0.0, 0.6, 0.0, reach, 2, low_angle, 0.7);
        b.rotor(0.0, 2.45, 0.0, reach, 1, high_angle, 0.7);

        // Deterministic drop order, in groups of two or three: identical everywhere, no events needed.
        let mut at = 8.0;
        let mut order: Vec<usize> = (0..plates.len()).collect();
        shuffle(&mut order, &mut b.rng);
        let mut k = 0;
        while k < order.len() {
            let group = if b.rng.next() < 0.5 { 2 } else { 3 };
            let mut g = 0;
            while g < group && k < order.len() {
                plates[order[k]].fall_at = at;
                g += 1;
                k += 1;
            }
            at += 2.3f64.max(5.2 - k as f64 * 0.07);
        }
        for r in [3.0, 7.0] {
            for a in 0..4 {
                let a = a as f64;
                b.bonus(m::cos(a * 1.57 + r) * r * 1.3, 0.0, m::sin(a * 1.57 + r) * r * 1.3);
            }
        }
        let plates = Arc::new(plates);
        let ps = plates.clone();
        b.mover(move |t, ctx| {
            for p in ps.iter() {
                let pos = p.base;
                let left = p.fall_at - t;
                ctx.set_enabled(p.col, left > 0.0);
                let n = ctx.node(p.node);
                n.pos = if left > WARN {
                    pos
                } else if left > 0.0 {
                    V3::new(
                        pos.x + m::sin(t * 50.0 + p.x) * 0.08 * (1.0 - left / WARN),
                        pos.y,
                        pos.z,
                    )
                } else {
                    V3::new(pos.x, pos.y - DROP * left * left, pos.z)
                };
                n.visible = left > -1.0;
            }
        });
        if !b.server() {
            // A steady one-way tint toward red instead of blinking (photosensitivity: no flashes).
            let ps = plates.clone();
            b.special_look(ROOT, "plate-tint", &[], move |_, t, out| {
                for p in ps.iter() {
                    let u = m::clamp(1.0 - (p.fall_at - t) / WARN, 0.0, 1.0);
                    let k = 0.8 * u * u * (3.0 - 2.0 * u);
                    out.tints.push(Tint {
                        node: p.node,
                        to: "#ff4a3a",
                        k,
                    });
                }
            });
        }
        b.clouds(0.0, 0.0, 55.0);

        let plate_at = {
            let plates = plates.clone();
            move |x: f64, z: f64| {
                plates
                    .iter()
                    .find(|p| (p.x - x).abs() < PLATE / 2.0 && (p.z - z).abs() < PLATE / 2.0)
                    .map(|p| p.fall_at)
            }
        };
        let plate_at2 = plate_at.clone();
        let mut opts = ArenaOpts::new(reach - 1.0);
        opts.retarget = Some(1.2);
        opts.ignore_nav = true;
        opts.floor = Some(Box::new(move |x, z, t| {
            if x.abs() < PLATE / 2.0 + 0.1 && z.abs() < PLATE / 2.0 + 0.1 {
                return true;
            }
            plate_at(x, z).is_some_and(|f| f - t > 0.3)
        }));
        opts.safe = Some(Box::new(move |x, z, t| {
            let r = m::hypot(x, z);
            if r < 2.8 || r > reach - 3.0 {
                return false;
            }
            plate_at2(x, z).is_some_and(|f| f - t > WARN + 2.0)
        }));
        opts.jump_when = Some(Box::new(move |bot| {
            let t = bot.t.max(0.0);
            if t <= 0.0 {
                return false;
            }
            let p = bot.body.pos;
            let eta = arm_contact_eta(p, low_angle(t), dir * low.omega(t), 2, 0.0, 0.0, 0.36);
            let high = if t > high_at {
                let w = -dir * (0.6 + 0.006 * (t - high_at));
                arm_contact_eta(p, high_angle(t), w, 1, 0.0, 0.0, 0.36)
            } else {
                9.0
            };
            eta > 0.1 && eta < 0.15 + bot.mem.react.unwrap_or(0.2) * 0.3 && high > 0.7
        }));
        MapSpec {
            spawns: b.ring_spawns(8, 8.0, 0.1, m::PI / 8.0),
            kill_y: -12.0,
            face_center: true,
            view: Some(V3::new(0.0, 2.0, 0.0)),
            bot: Some(arena_brain(opts)),
            ..Default::default()
        }
    }
}
