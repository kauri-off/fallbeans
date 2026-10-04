//! Not a game: the stage at the end of a game, players standing on podiums by final place.
use fb_shared::rng::Rng;
use fb_sim::builder::{Builder, PrimOpts, PropOpts};
use fb_sim::m;
use fb_sim::map::{GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::scene::{Finish, Form, Part, Piece, pal};

use crate::util::{deco, o};

pub struct Podium;

pub static META: GameMeta = GameMeta::new("podium", "Итоги", Genre::Points, "", "", 1e6);

/// Podium x and top height for each final place (1st in the middle, then alternating sides).
pub const PODIUM_SLOTS: [(f64, f64); 8] = [
    (0.0, 3.0),
    (-3.0, 2.2),
    (3.0, 1.6),
    (-6.0, 0.8),
    (6.0, 0.8),
    (-8.6, 0.4),
    (8.6, 0.4),
    (0.0, 0.4),
];

impl MapDef for Podium {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn build(&self, b: &mut Builder, _ctx: &MapCtx) -> MapSpec {
        let floor = PrimOpts {
            freq: Some(0.3),
            ..Default::default()
        };
        b.cyl(0.0, -1.0, 0.0, 16.0, 2.0, pal::PURPLE, floor);
        b.cyl(0.0, 0.03, 0.0, 16.05, 0.1, pal::YELLOW, deco());
        let pals = [
            pal::YELLOW,
            pal::WHITE,
            pal::ORANGE,
            pal::BLUE,
            pal::BLUE,
            pal::TEAL,
            pal::TEAL,
            pal::PINK,
        ];
        let spawns = PODIUM_SLOTS
            .iter()
            .enumerate()
            .map(|(i, &(x, h))| {
                // The 8th place stands in front, on the floor.
                let z = if i == 7 { 4.0 } else { 0.0 };
                if i < 7 {
                    b.box_(x, h / 2.0, z, 2.6, h, 2.6, pals[i], o());
                }
                V3::new(x, h + 0.05, z)
            })
            .collect();
        if !b.server() {
            // Medals on the podium fronts, and confetti over them.
            let medals = [("🥇", "#ffd84a"), ("🥈", "#f4f1ff"), ("🥉", "#ff9f4a")];
            for (i, (medal, color)) in medals.into_iter().enumerate() {
                let (x, h) = PODIUM_SLOTS[i];
                let at = b.anchor(x, h / 2.0, 1.32, ROOT);
                let face = [Part::new(Form::Label(1.4, 1.4, medal), color, Finish::Matte)];
                b.special(at, "medal", &face, vec![Piece::at(0, 0.0, 0.0, 0.0)]);
            }
            confetti(b);
        }
        // Stage dressing behind the podiums: fans, flags and stars.
        for sx in [-1.0, 1.0] {
            let fan = PropOpts {
                yaw: -sx * 0.5,
                scale: Some(1.3),
                tint: None,
            };
            b.prop("fan", sx * 11.5, 0.0, -5.5, fan);
            let flag = PropOpts {
                tint: Some(if sx < 0.0 { "#ff5fa2" } else { "#3fa9ff" }),
                yaw: if sx < 0.0 { m::PI } else { 0.0 },
                scale: None,
            };
            b.prop("flag", sx * 13.5, 0.0, -1.0, flag);
            let star = PropOpts {
                scale: Some(1.2),
                ..Default::default()
            };
            b.prop("star", sx * 4.5, 7.2, -2.0, star);
        }
        let star = PropOpts {
            scale: Some(1.8),
            ..Default::default()
        };
        b.prop("star", 0.0, 8.4, -2.5, star);
        b.clouds(0.0, 0.0, 45.0);
        MapSpec {
            spawns,
            kill_y: -15.0,
            view: Some(V3::new(0.0, 2.4, 0.0)),
            ..Default::default()
        }
    }
}

/// Small tumbling pieces falling on a loop over the podiums (looks only: a fixed seed of its own).
fn confetti(b: &mut Builder) {
    const COLORS: [&str; 7] = [
        "#ff5fa2", "#3fa9ff", "#ffd23f", "#4fdc6a", "#a66bff", "#ff8a3d", "#ffffff",
    ];
    let parts = COLORS.map(|c| Part::new(Form::Plane(0.16, 0.26), c, Finish::Flat));
    let mut rng = Rng::new(7);
    let seeds: Vec<[f64; 5]> = (0..260)
        .map(|_| {
            let x = (rng.next() - 0.5) * 22.0;
            let z = (rng.next() - 0.5) * 10.0;
            [x, z, 1.2 + rng.next() * 1.4, rng.next() * 20.0, 2.0 + rng.next() * 6.0]
        })
        .collect();
    b.special_look(ROOT, "confetti", &parts, move |_, t, out| {
        for (i, &[x, z, speed, phase, spin]) in seeds.iter().enumerate() {
            let y = 14.0 - ((t * speed + phase) % 16.0);
            let p = Piece::at(
                (i % COLORS.len()) as u8,
                x + m::sin(t * 1.3 + phase) * 0.6,
                y,
                z + m::cos(t + phase) * 0.4,
            );
            out.pieces.push(p.rot(t * spin, t * spin * 0.7, phase));
        }
    });
}
