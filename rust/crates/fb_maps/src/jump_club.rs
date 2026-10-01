//! Jump the low bar, stay under the high one. Direction, the number of bars and how fast they speed up
//! come from the seed.
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::m;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::props::SpinUp;
use fb_sim::scene::pal;

pub struct JumpClub;

static META: GameMeta = GameMeta {
    id: "jump-club",
    title: "Прыг-клуб",
    genre: Genre::Survival,
    desc: "Перепрыгивайте нижнюю балку и не попадайтесь под верхнюю. Со временем обе крутятся всё быстрее!",
    goal: "Не упадите",
    duration: 75.0,
};

impl MapDef for JumpClub {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["neon", "starlight", "circus"]
    }

    fn build(&self, b: &mut Builder, _ctx: &MapCtx) -> MapSpec {
        let rng = &mut b.rng;
        let dir = if rng.next() < 0.5 { 1.0 } else { -1.0 };
        let low_arms = 2;
        let high_arms = if rng.next() < 0.35 { 1 } else { 2 };
        let acc = 0.009 + rng.next() * 0.005;
        // The low bar starts 10° past a spawn pair and eases in: the first bean it reaches has over a second.
        let low = SpinUp::new(-0.26, 1.1 + rng.next() * 0.15, acc);
        let hk = 0.006 + rng.next() * 0.004;
        let low_ang = move |t: f64| dir * low.angle(t);
        let high_ang = move |t: f64| dir * (m::PI / 2.0 - if t <= 0.0 { 0.0 } else { 0.7 * t + hk * t * t });

        let freq = |f: f64| PrimOpts {
            freq: Some(f),
            ..Default::default()
        };
        let deco = |f: Option<f64>| PrimOpts {
            freq: f,
            no_collide: true,
            ..Default::default()
        };
        b.cyl(0.0, -1.0, 0.0, 13.0, 2.0, pal::BLUE, freq(0.35));
        b.cyl(0.0, 0.03, 0.0, 13.05, 0.1, pal::YELLOW, deco(None));
        b.cyl(0.0, 0.06, 0.0, 11.5, 0.1, pal::BLUE, deco(Some(0.35)));
        b.hub(0.0, 0.0, 0.0, 1.2);
        b.rotor(0.0, 0.6, 0.0, 12.6, low_arms, low_ang, 0.6);
        b.rotor(0.0, 2.45, 0.0, 12.6, high_arms, high_ang, 0.6);
        for a in [0.0, 1.0, 2.0, 3.0] {
            b.bonus(m::cos(a * 1.57 + 0.8) * 8.0, 0.0, m::sin(a * 1.57 + 0.8) * 8.0);
        }
        b.clouds(0.0, 0.0, 40.0);
        let spawns = [25.0, 65.0, 115.0, 155.0, 205.0, 245.0, 295.0, 335.0]
            .map(|d: f64| {
                let a = (d * m::PI) / 180.0;
                V3::new(m::cos(a) * 6.0, 0.1, m::sin(a) * 6.0)
            })
            .to_vec();
        MapSpec {
            spawns,
            kill_y: -6.0,
            face_center: true,
            view: Some(V3::new(0.0, 3.0, 0.0)),
        }
    }
}
