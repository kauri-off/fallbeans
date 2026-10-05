//! Drums: a staircase of drums surging and easing (or a launch pad to a walkway over them), drums with
//! pegs, rollers, logs rolling you sideways and changing direction, in an order from the seed.
use fb_sim::bots::{BotView, SharedTest, Waypoint};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::collider::ColliderOpts;
use fb_sim::course::{
    CourseOpts, SegOut, Segment, edge_jump, hammer_bridges, pick_sections, race_course, rotor_decks, trampoline_gap,
    with_rests,
};
use fb_sim::m::{self, MinMaxJs};
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::scene::{Palette, pal};

use crate::util::{deco, o};

pub struct DrumRoll;

static META: GameMeta = GameMeta::new(
    "drum-roll",
    "Барабаны",
    Genre::Race,
    "Лестница из барабанов, которые то разгоняются, то замедляются (или батут на мостик над ними), барабаны с шипами, катки, брёвна, которые катят вбок и меняют направление. Порядок каждый раз свой!",
    "Доберитесь до финиша",
    140.0,
);

/// A drum along x (rolling you forwards or back) or along z (a log rolling you sideways), turned by
/// `angle(t)`. Pegs (boxes on the surface) knock over whoever they catch.
fn drum(
    b: &mut Builder,
    x: f64,
    top: f64,
    z: f64,
    r: f64,
    len: f64,
    angle: impl Fn(f64) -> f64 + Send + Sync + 'static,
    p: Palette,
    along_z: bool,
    pegs: u32,
) {
    let axis = b.anchor(x, top - r, z, ROOT);
    if along_z {
        b.world.nodes.get_mut(axis).rot.y = m::PI / 2.0;
    }
    let opts = PrimOpts {
        parent: Some(axis),
        dynamic: true,
        rot: Some(V3::new(0.0, 0.0, m::PI / 2.0)),
        seg: Some(36),
        col: ColliderOpts {
            tag: Some("drum"),
            ..Default::default()
        },
        ..Default::default()
    };
    let d = b.cyl(0.0, 0.0, 0.0, r, len, p, opts).node;
    for k in 0..8 {
        let a = (k as f64 / 8.0) * m::PI * 2.0;
        let stripe = PrimOpts {
            parent: Some(d),
            ..deco()
        };
        b.box_(
            m::cos(a) * r,
            0.0,
            m::sin(a) * r,
            0.16,
            len - 0.2,
            0.32,
            pal::hex("#ffffff"),
            stripe,
        );
    }
    // Pegs: bars across the drum that come round and sweep the top.
    for k in 0..pegs {
        let a = (k as f64 / pegs.max(1) as f64) * m::PI * 2.0 + 0.4;
        let opts = PrimOpts {
            parent: Some(d),
            dynamic: true,
            rot: Some(V3::new(0.0, a, 0.0)),
            col: ColliderOpts {
                hit: 0.9,
                tag: Some("peg"),
                ..Default::default()
            },
            ..Default::default()
        };
        b.box_(
            m::cos(a) * (r + 0.2),
            0.0,
            m::sin(a) * (r + 0.2),
            0.4,
            len - 1.2,
            0.4,
            pal::RED,
            opts,
        );
    }
    b.mover(move |t, ctx| ctx.node(d).rot = V3::new(angle(t), 0.0, m::PI / 2.0));
}

/// Spin that speeds up and slows down (and may reverse): rate base + amp·sin(w t + ph); its angle.
#[derive(Clone, Copy)]
struct Pulse {
    base: f64,
    amp: f64,
    w: f64,
    ph: f64,
}

impl Pulse {
    fn angle(self, t: f64) -> f64 {
        let tt = t.max_js(0.0);
        self.base * tt - (self.amp / self.w) * (m::cos(tt * self.w + self.ph) - m::cos(self.ph))
    }
}

const PALS: [Palette; 4] = [pal::ORANGE, pal::TEAL, pal::PINK, pal::GREEN];

/// A zig-zag staircase of drums rolling back at you, their speed surging and easing, or a launch pad to
/// a narrow walkway over them.
fn drum_stairs() -> Segment {
    Box::new(|s| {
        let y = s.y;
        s.b.box_(0.0, y - 1.0, s.z + 2.0, 16.0, 2.0, 4.0, pal::PURPLE, o());
        let r = 1.4;
        let stairs: Vec<(f64, f64, f64, Pulse)> = (0..6)
            .map(|i| {
                let x = if i % 2 == 1 { 3.0 } else { -3.0 };
                let base = -(1.1 + s.rng() * 0.3);
                let amp = 0.45 + s.rng() * 0.35;
                let w = 0.7 + s.rng() * 0.5;
                let ph = s.rng() * 6.0;
                (
                    x,
                    s.z + 5.8 + i as f64 * 4.8,
                    y + 0.3 + i as f64 * 0.5,
                    Pulse { base, amp, w, ph },
                )
            })
            .collect();
        for (i, &(x, z, top, spin)) in stairs.iter().enumerate() {
            drum(s.b, x, top, z, r, 5.0, move |t| spin.angle(t), PALS[i % 4], false, 0);
        }
        let (_, last_z, last_top, _) = stairs[5];
        let end_z = last_z + r + 0.05;
        let end_y = last_top - 0.45;
        s.b.box_(0.0, end_y - 1.0, end_z + 3.0, 23.0, 2.0, 6.0, pal::PURPLE, o());
        // The walkway: launch pads at the sides, up to a narrow beam over the drums.
        for sx in [-1.0, 1.0] {
            s.b.box_(sx * 9.5, y - 1.0, s.z + 3.5, 3.5, 2.0, 3.0, pal::YELLOW, o());
            s.b.pad(sx * 9.5, y, s.z + 3.4, 1.1, 17.0, None);
        }
        let beam_y = y + 3.5;
        let beam_z0 = s.z + 5.5;
        s.b.box_(
            9.5,
            beam_y,
            (beam_z0 + end_z) / 2.0,
            2.2,
            1.0,
            end_z - beam_z0,
            pal::PINK,
            o(),
        );
        s.b.bonus(9.5, beam_y + 0.5, (beam_z0 + end_z) / 2.0);

        let z0 = s.z;
        let stairs_route = || {
            let mut route = vec![Waypoint::w(0.0, z0 + 1.5, 1.0)];
            for &(x, z, _, _) in &stairs {
                route.push(
                    Waypoint::w(if x > 0.0 { 1.3 } else { -1.3 }, z, 0.15)
                        .speed(0.9)
                        .jump_when(move |bot| bot.body.pos.z > z - 5.6 && bot.body.pos.z < z - 4.4),
                );
            }
            route.push(
                Waypoint::w(0.0, end_z + 1.5, 0.5)
                    .jump_when(move |bot| bot.body.pos.z > last_z + 0.3 && bot.body.pos.z < end_z),
            );
            route.push(Waypoint::w(0.0, end_z + 3.0, 1.0));
            route
        };
        let beam_route = vec![
            Waypoint::w(6.5, z0 + 2.5, 0.0),
            Waypoint::w(9.5, z0 + 3.4, 0.0),
            Waypoint::w(9.5, beam_z0 + 2.0, 0.0),
            Waypoint::w(9.5, end_z - 0.5, 0.0),
            Waypoint::w(5.0, end_z + 2.0, 0.3),
            Waypoint::w(0.0, end_z + 3.0, 1.0),
        ];
        SegOut {
            z: end_z + 6.0,
            y: end_y,
            routes: vec![stairs_route(), stairs_route(), beam_route],
            // Only over this section: the course ORs every section's zone over the whole map.
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < end_z && p.y > beam_y + 3.0)),
            checkpoint: Some((end_z + 0.5, V3::new(0.0, end_y + 0.1, end_z + 3.0))),
        }
    })
}

/// Logs rolling sideways, each on its own rhythm, reversing now and then; run along them and hop the gaps.
fn log_run(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let r = 1.8;
        let l = 9.0;
        let mut zz = s.z + 2.0;
        s.b.box_(0.0, y - 1.0, s.z + 1.0, 12.0, 2.0, 2.0, pal::PURPLE, o());
        let mut route = vec![Waypoint::w(0.0, s.z + 1.0, 1.0)];
        let mut edge = s.z + 2.0;
        for i in 0..n {
            let c = zz + 1.4 + l / 2.0;
            let dir = if s.rng() < 0.5 { -1.0 } else { 1.0 };
            // Some logs reverse (the rate swings through zero), some just surge.
            let reverse = s.rng() < 0.4;
            let sp = if reverse {
                let amp = 1.4 + s.rng() * 0.4;
                let w = 0.5 + s.rng() * 0.3;
                let ph = s.rng() * 6.0;
                Pulse { base: 0.0, amp, w, ph }
            } else {
                let base = dir * (1.0 + s.rng() * 0.5);
                let ph = s.rng() * 6.0;
                Pulse {
                    base,
                    amp: 0.5,
                    w: 0.9,
                    ph,
                }
            };
            drum(
                s.b,
                0.0,
                y,
                c,
                r,
                l,
                move |t| sp.angle(t),
                PALS[(i as usize + 2) % 4],
                true,
                0,
            );
            let e = edge;
            route.push(
                Waypoint::w(0.0, c - l / 2.0 + 1.2, 0.1)
                    .jump_when(move |bot| bot.body.pos.z > e - 1.5 && bot.body.pos.z < e + 0.2),
            );
            route.push(Waypoint::w(0.0, c + l / 2.0 - 2.2, 0.0));
            edge = c + l / 2.0;
            zz = c + l / 2.0;
        }
        s.b.bonus(0.0, y + 0.1, s.z + 2.0 + 1.4 + l + 1.4 + l / 2.0);
        zz += 1.4;
        s.b.box_(0.0, y - 1.0, zz + 3.0, 14.0, 2.0, 6.0, pal::PURPLE, o());
        let e = edge;
        route.push(
            Waypoint::w(0.0, zz + 3.0, 1.0).jump_when(move |bot| bot.body.pos.z > e - 1.5 && bot.body.pos.z < e + 0.2),
        );
        SegOut {
            z: zz + 6.0,
            y,
            routes: vec![route],
            checkpoint: Some((zz + 0.5, V3::new(0.0, y + 0.1, zz + 3.0))),
            ..Default::default()
        }
    })
}

/// Big drums rolling towards you with pegs across them: jump each peg as it comes over the top.
fn peg_drums(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let r = 2.2;
        let mut zz = s.z + 2.0;
        s.b.box_(0.0, y - 1.0, s.z + 1.0, 12.0, 2.0, 2.0, pal::PURPLE, o());
        let mut route = vec![Waypoint::w(0.0, s.z + 1.0, 1.0)];
        for i in 0..n {
            // Room for the pegs (they stand 0.4 m proud) between drums and platforms.
            let c = zz + 0.55 + r;
            let w = -(0.55 + s.rng() * 0.25);
            let ph = s.rng() * 6.0;
            let pegs = 3;
            let ang = move |t: f64| ph + t.max_js(0.0) * w;
            drum(s.b, 0.0, y + 0.2, c, r, 9.0, ang, PALS[i as usize % 4], false, pegs);
            // Pegs sit at angle β = offset + ang(t) on the drum (β = 0 on top, growing towards +z) and
            // come round to the bot at |w| rad/s: jump just before one reaches it.
            let axis_y = y + 0.2 - r;
            let margin = 0.7 / (r + 0.2);
            let jump_when: SharedTest = std::sync::Arc::new(move |bot: &mut BotView| {
                let p = bot.body.pos;
                if (p.z - c).abs() > r + 0.6 || bot.t <= 0.0 {
                    return false;
                }
                let phi = m::atan2(p.z - c, p.y - axis_y);
                for k in 0..pegs {
                    let mut d = ((k as f64 / pegs as f64) * m::PI * 2.0 + 0.4 + ang(bot.t) - phi) % (m::PI * 2.0);
                    if d < 0.0 {
                        d += m::PI * 2.0;
                    }
                    let eta = (d - margin) / -w;
                    if eta > 0.08 && eta < 0.22 {
                        return true;
                    }
                }
                false
            });
            let gap_at = c - r - 0.55;
            let edge = edge_jump(gap_at, 0.8);
            let jw = jump_when.clone();
            route.push(Waypoint::w(0.0, c - 0.5, 0.2).jump_when(move |bot| jw(bot) || edge(bot)));
            route.push(Waypoint::w(0.0, c + r - 0.3, 0.2).jump_shared(&jump_when));
            zz = c + r + 0.55;
        }
        s.b.box_(0.0, y - 1.0, zz + 3.0, 14.0, 2.0, 6.0, pal::PURPLE, o());
        route.push(Waypoint::w(0.0, zz + 3.0, 1.0).jump_when(edge_jump(zz, 1.2)));
        SegOut {
            z: zz + 6.0,
            y,
            routes: vec![route],
            checkpoint: Some((zz + 0.5, V3::new(0.0, y + 0.1, zz + 3.0))),
            ..Default::default()
        }
    })
}

/// A bridge of small rollers turning in alternating directions, with bumpers to dodge.
fn roller_bridge(n: u32) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let r = 0.55;
        let pitch = 1.3;
        s.b.box_(0.0, y - 1.0, s.z + 1.0, 10.0, 2.0, 2.0, pal::PURPLE, o());
        for i in 0..n {
            let c = s.z + 2.0 + r + i as f64 * pitch;
            let sp = (if i % 2 == 1 { 1.0 } else { -1.0 }) * (2.0 + s.rng() * 2.0);
            drum(
                s.b,
                0.0,
                y,
                c,
                r,
                7.0,
                move |t| t.max_js(0.0) * sp,
                PALS[i as usize % 4],
                false,
                0,
            );
        }
        let end = s.z + 2.0 + n as f64 * pitch + 0.2;
        for (x, f) in [(-2.0, 0.3), (2.0, 0.65)] {
            s.b.bumper(x, y + 0.2, s.z + 2.0 + n as f64 * pitch * f, 0.7, 9.0);
        }
        s.b.box_(0.0, y - 1.0, end + 3.0, 14.0, 2.0, 6.0, pal::PURPLE, o());
        SegOut {
            z: end + 6.0,
            y,
            routes: vec![vec![
                Waypoint::w(0.0, s.z + 2.0 + n as f64 * pitch * 0.5, 0.5),
                Waypoint::w(0.0, end + 3.0, 1.0),
            ]],
            checkpoint: Some((end + 0.5, V3::new(0.0, y + 0.1, end + 3.0))),
            ..Default::default()
        }
    })
}

impl MapDef for DrumRoll {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["circus", "candy", "royal"]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![
            rotor_decks(1),
            peg_drums(3),
            roller_bridge(10),
            trampoline_gap(),
            hammer_bridges(2),
        ];
        let middle = pick_sections(&mut b.rng, pool, 3);
        let mut sections = vec![drum_stairs()];
        sections.extend(middle);
        sections.push(log_run(5));
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
