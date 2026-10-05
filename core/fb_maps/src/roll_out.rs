//! Three huge drums with missing slats turn under you: run against the turn and hop the holes. Fall in
//! through a hole and you land inside the drum: keep running on the inside (the holes come round down
//! there too). Only falling out of a drum ends your round.
use std::collections::BTreeSet;

use fb_sim::bots::{HumanOpts, Note, humanize, init_bot};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::m::{self, MinMax};
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::scene::{Finish, Form, Part, Piece, pal};

use crate::util::deco;

pub struct RollOut;

static META: GameMeta = GameMeta::new(
    "roll-out",
    "Перекати-поле",
    Genre::Survival,
    "Огромные барабаны с дырами вращаются под ногами. Бегите против вращения и перепрыгивайте провалы!",
    "Не упадите",
    80.0,
);

const R: f64 = 7.0;
const N: u32 = 20;
const CY: f64 = -R;
const STEP: f64 = (2.0 * m::PI) / N as f64;

struct Ring {
    z: f64,
    dir: f64,
    /// Runs of missing slats as angular intervals (from, to) in slat units.
    holes: Vec<(f64, f64)>,
}

impl Ring {
    fn angle(&self, t: f64) -> f64 {
        if t <= 0.0 {
            0.0
        } else {
            self.dir * (0.35 * t + 0.002 * t * t)
        }
    }
}

impl MapDef for RollOut {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["ocean", "jungle", "meadow"]
    }

    fn build(&self, b: &mut Builder, _ctx: &MapCtx) -> MapSpec {
        let hop_mx: Note<f64> = b.note();
        let hop_until: Note<f64> = b.note();
        let mut rings = Vec::new();
        let mut groups = Vec::new();
        for (z, dir, p) in [(-9.0, 1.0, pal::PINK), (0.0, -1.0, pal::BLUE), (9.0, 1.0, pal::YELLOW)] {
            let group = b.anchor(0.0, CY, z, ROOT);
            let mut missing = BTreeSet::new();
            // The outer drums lose one slat more than the middle one.
            while missing.len() < if z == 0.0 { 4 } else { 5 } {
                let k = (b.rng.next() * N as f64).floor() as u32;
                if k > 2 && k < N - 2 {
                    missing.insert(k);
                }
            }
            let width = ((2.0 * m::PI * R) / N as f64) * 0.96;
            for k in 0..N {
                if missing.contains(&k) {
                    continue;
                }
                let a = (k as f64 / N as f64) * m::PI * 2.0;
                let o = PrimOpts {
                    parent: Some(group),
                    rot: Some(V3::new(0.0, 0.0, -a)),
                    dynamic: true,
                    ..Default::default()
                };
                let slat = if k % 2 == 1 { p } else { pal::WHITE };
                b.box_(
                    m::sin(a) * (R - 0.25),
                    m::cos(a) * (R - 0.25),
                    0.0,
                    width,
                    0.5,
                    8.0,
                    slat,
                    o,
                );
            }
            if !b.server() {
                let rim = [Part::new(Form::Torus(R, 0.25), "#5a3fb8", Finish::Matte).on("rubber")];
                b.special(
                    group,
                    "drum-rims",
                    &rim,
                    vec![Piece::at(0, 0.0, 0.0, -4.1), Piece::at(0, 0.0, 0.0, 4.1)],
                );
            }
            let ring = Ring {
                z,
                dir,
                holes: Vec::new(),
            };
            let angle = move |t: f64| ring.angle(t);
            b.mover(move |t, ctx| ctx.node(group).rot.z = angle(t));
            let mut holes: Vec<(f64, f64)> = Vec::new();
            for &k in &missing {
                let k = k as f64;
                match holes.last_mut() {
                    Some(last) if k - 0.5 == last.1 => last.1 = k + 0.5,
                    _ => holes.push((k - 0.5, k + 0.5)),
                }
            }
            rings.push(Ring { z, dir, holes });
            groups.push(group);
        }
        // Spokes at the rims instead of an axle through the middle (the inside is part of the course).
        if !b.server() {
            for &group in &groups {
                for dz in [-4.1, 4.1] {
                    for k in 0..3 {
                        let o = PrimOpts {
                            parent: Some(group),
                            rot: Some(V3::new(0.0, 0.0, (k as f64 / 3.0) * m::PI)),
                            ..deco()
                        };
                        b.box_(0.0, 0.0, dz * 1.06, 0.3, R * 2.0 - 0.6, 0.3, pal::hex("#5a3fb8"), o);
                    }
                }
            }
        }
        for r in &rings {
            b.bonus(0.0, 0.05, r.z + 2.5);
        }
        b.clouds_with(0.0, 0.0, 40.0, 30, -40.0, -10.0);

        let mut spawns: Vec<V3> = [-9.0, 0.0, 9.0]
            .iter()
            .flat_map(|&z| [-1.2, 1.2].map(|x| V3::new(x, 0.1, z)))
            .collect();
        spawns.push(V3::new(0.0, 0.1, -11.0));
        spawns.push(V3::new(0.0, 0.1, 11.0));

        MapSpec {
            spawns,
            kill_y: -16.0,
            view: Some(V3::new(0.0, 2.0, 0.0)),
            bot: Some(Box::new(move |bot, out| {
                init_bot(bot);
                let p = bot.body.pos;
                let mut ring = &rings[0];
                for r in &rings[1..] {
                    if (r.z - p.z).abs() < (ring.z - p.z).abs() {
                        ring = r;
                    }
                }
                // Inside the drum (fallen in): the same game on the inner surface, at the bottom.
                let inside = p.y < CY;
                let rs = if inside { R - 0.5 } else { R };
                let mirror = if inside { -1.0 } else { 1.0 };
                let t = bot.t.at_least(0.0);
                let omega = if t > 0.0 { ring.dir * (0.35 + 0.004 * t) } else { 0.0 };
                // The top of the drum carries us sideways at −ω·R; "up" is against it.
                let carry = -omega * rs * mirror;
                let up = if carry == 0.0 { 0.0 } else { -m::sign(carry) };
                let lane = ring.z + ((bot.id % 3) as f64 - 1.0) * 1.5 + bot.mem.traits.off * 0.4;
                // Holes as intervals along the surface, in metres towards "up" from the bot.
                let theta = ring.angle(t);
                let phi = m::atan2(p.x, p.y - CY);
                let mut ahead: Option<(f64, f64)> = None;
                let mut behind = f64::INFINITY;
                for &(from, to) in &ring.holes {
                    let mut a0 = from * STEP - theta - phi;
                    a0 = m::atan2(m::sin(a0), m::cos(a0));
                    let a1 = a0 + (to - from) * STEP;
                    let up_a = up * mirror;
                    let near = if up_a >= 0.0 { a0 * rs } else { -a1 * rs };
                    let far = if up_a >= 0.0 { a1 * rs } else { -a0 * rs };
                    if far > -0.2 && near > -0.4 {
                        if ahead.is_none_or(|a| near < a.0) {
                            ahead = Some((near, far));
                        }
                    } else if far <= -0.2 {
                        behind = behind.at_most(-far);
                    }
                }
                // On the ground: hold our place against the carry and drift back to the crest, but never
                // towards a hole that just went by under us.
                let mut mx = -carry / 8.5;
                let to_top = -p.x;
                let into_behind = up != 0.0 && m::sign(to_top) == -up && behind < 2.8;
                if !into_behind {
                    mx += (-0.45f64).at_least(0.45f64.at_most(to_top * 0.35));
                }
                let mz = (-1f64).at_least(1f64.at_most((lane - p.z) * 0.6));
                // A hole coming at us: hop when its near edge reaches our feet. Worse players jump a little
                // early or late.
                let slop = (1.0 - bot.mem.traits.skill) * 0.5;
                let late = (bot.rng.next() - 0.5) * slop;
                let grounded = bot.body.grounded;
                if let Some((near, far)) = ahead
                    && up != 0.0
                    && grounded
                    && near < 1.0 + late
                    && near > -0.3
                {
                    // Clear the near edge, the hole and a body length, minus what the drum brings us.
                    let span = far - near.at_least(0.0) + 1.6;
                    out.jump = true;
                    // …but never so far that it lands on the steep side of the drum.
                    let room = 0.2f64.at_least((3.2 - up * p.x * mirror) / 6.4);
                    mx = up
                        * 1f64
                            .at_most(room)
                            .at_most(0.2f64.at_least((span - carry.abs() * 0.75) / 0.75 / 8.5));
                    bot.mem.set(hop_mx, mx);
                    bot.mem.set(hop_until, bot.t + 0.7);
                } else if !grounded && bot.mem.get(hop_until).unwrap_or(-1.0) > bot.t {
                    mx = bot.mem.get(hop_mx).unwrap_or(mx);
                }
                let l = m::hypot(mx, mz);
                let l = if l == 0.0 { 1.0 } else { l };
                let k = l.at_most(1.0);
                out.mx = (mx / l) * k;
                out.mz = (mz / l) * k;
                humanize(
                    bot,
                    out,
                    &HumanOpts {
                        precise: true,
                        ..Default::default()
                    },
                );
            })),
            ..Default::default()
        }
    }
}
