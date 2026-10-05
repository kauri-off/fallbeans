//! The fork: a narrow bridge under swinging hammers (short, but you must time it) or a walled zig-zag
//! with pushers (safe, but longer); then seesaws, tipping bridges, belts, gloves and hammers in an order
//! and with timings from the seed.
use fb_sim::bots::Waypoint;
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::collider::ColliderOpts;
use fb_sim::course::{
    CourseOpts, SegOut, Segment, conveyor, glove_alley, hammer_bridges, moving_platforms, pick_sections, race_course,
    rotor_decks, seesaws, tipping_bridge, with_rests,
};
use fb_sim::m;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::scene::pal;

use crate::util::o;

pub struct HammerSwing;

static META: GameMeta = GameMeta::new(
    "hammer-swing",
    "Молоты и качели",
    Genre::Race,
    "Развилка: мост под молотами или коридор с толкателями. Дальше — качели, мостики-перевёртыши, ленты, перчатки и молоты в случайном порядке и со своими таймингами.",
    "Доберитесь до финиша",
    150.0,
);

fn fork() -> Segment {
    Box::new(|s| {
        let y = s.y;
        let z0 = s.z;
        let len = 30.0;
        let z1 = z0 + len;
        let bx = -5.0;
        s.b.box_(bx, y - 1.0, z0 + len / 2.0, 3.2, 2.0, len, pal::BLUE, o());
        let hammers: Vec<(f64, f64, f64)> = [5.0, 12.0, 19.0, 26.0]
            .iter()
            .map(|dz| {
                let w = 1.8 + s.rng() * 0.8;
                let ph = s.rng() * m::TAU;
                (z0 + dz, w, ph)
            })
            .collect();
        for &(z, w, ph) in &hammers {
            s.b.hammer(bx, y + 7.4, z, w, ph, 1.12, true);
        }
        let head_x = move |w: f64, ph: f64, t: f64| bx + 6.0 * m::sin(m::sin(t * w + ph) * 1.12);

        s.b.box_(5.0, y - 1.0, z0 + len / 2.0, 5.0, 2.0, len, pal::GREEN, o());
        for sx in [2.1, 7.9] {
            s.b.box_(sx, y + 1.2, z0 + len / 2.0, 0.8, 2.4, len, pal::PINK, o());
        }
        let zig: Vec<(f64, f64, f64)> = [5.0, 11.5, 18.0, 24.5]
            .iter()
            .enumerate()
            .map(|(k, dz)| {
                let odd = k % 2 == 1;
                (z0 + dz, if odd { 4.5 } else { 2.5 }, if odd { 7.5 } else { 5.5 })
            })
            .collect();
        for &(z, x0, x1) in &zig {
            s.b.box_((x0 + x1) / 2.0, y + 1.2, z, x1 - x0, 2.4, 0.8, pal::PURPLE, o());
        }
        let pushers: Vec<(f64, f64, f64)> = zig
            .iter()
            .map(|&(z, _, _)| {
                let w = 1.4 + s.rng() * 0.7;
                let ph = s.rng() * 6.0;
                (z + 3.2, w, ph)
            })
            .collect();
        let pusher_x = |w: f64, ph: f64, t: f64| 5.0 + m::sin(t * w + ph) * 1.6;
        for &(z, w, ph) in &pushers {
            let opts = PrimOpts {
                dynamic: true,
                col: ColliderOpts {
                    hit: 0.7,
                    tag: Some("pusher"),
                    ..Default::default()
                },
                ..Default::default()
            };
            let node = s.b.box_(5.0, y + 0.8, z, 1.4, 1.6, 1.4, pal::ORANGE, opts).node;
            s.b.mover(move |t, ctx| ctx.node(node).pos.x = pusher_x(w, ph, t));
        }
        s.b.bonus(bx, y, z0 + 15.5);
        s.b.box_(0.0, y - 1.0, z1 + 4.0, 18.0, 2.0, 8.0, pal::PURPLE, o());

        let mut bridge = vec![Waypoint::spread(bx, z0 + 0.5, 0.0)];
        for &(z, w, ph) in &hammers {
            bridge.push(Waypoint::spread(bx, z - 2.6, 0.0));
            bridge.push(Waypoint::spread(bx, z + 2.0, 0.0).wait(move |bot| {
                [0.0, 0.2, 0.4, 0.6, 0.8]
                    .iter()
                    .all(|dt| (head_x(w, ph, bot.t + dt) - bx).abs() > 2.6)
            }));
        }
        let mut walls = vec![Waypoint::spread(5.0, z0 + 0.5, 0.2)];
        for (k, &(z, x0, _)) in zig.iter().enumerate() {
            let gx = if x0 > 3.0 { 3.4 } else { 6.6 };
            let (pz, w, ph) = pushers[k];
            walls.push(Waypoint::spread(gx, z - 1.4, 0.0));
            walls.push(Waypoint::spread(gx, z + 1.4, 0.0));
            walls.push(Waypoint::spread(gx, pz + 1.4, 0.0).wait(move |bot| {
                (pusher_x(w, ph, bot.t + 0.35) - gx).abs() > 1.6 && (pusher_x(w, ph, bot.t) - gx).abs() > 1.6
            }));
        }
        for r in [&mut bridge, &mut walls] {
            r.push(Waypoint::spread(0.0, z1 + 4.0, 1.0));
        }
        SegOut {
            z: z1 + 8.0,
            y,
            routes: vec![bridge, walls],
            // On top of the zig-zag walls or the hammer frames.
            forbidden: Some(Box::new(move |p| p.z > z0 && p.z < z1 && p.y > y + 1.9)),
            checkpoint: Some((z1 + 0.5, V3::new(0.0, y + 0.1, z1 + 4.0))),
        }
    })
}

impl MapDef for HammerSwing {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["factory", "desert", "lava"]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![
            seesaws(3),
            conveyor(30.0),
            hammer_bridges(3),
            tipping_bridge(6),
            glove_alley(4),
            moving_platforms(4),
        ];
        let middle = pick_sections(&mut b.rng, pool, 4);
        let mut sections = vec![fork()];
        sections.extend(middle);
        sections.push(rotor_decks(1));
        let opts = CourseOpts {
            sections: with_rests(sections, 7.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
