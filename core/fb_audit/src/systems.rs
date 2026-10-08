//! Audits of the systems, not of one map: physics feel, input handling, game rules and planning.
use std::collections::{BTreeMap, BTreeSet};

use fb_arena::{Arena, ArenaKind, Stepper, tick_plain};
use fb_maps::director::{Mode, Playlist, ROUND_COUNTS, game, plan_game};
use fb_shared::game::Genre;
use fb_shared::input::{BTN_JUMP, InputFrame};
use fb_shared::m::MinMax;
use fb_shared::rng::{Rng, shuffle};
use fb_shared::rules::{RoundStats, RoundView, TOP_POINTS};
use fb_shared::{DT, MAX_PLAYERS, m};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::collider::ColliderOpts;
use fb_sim::map::NoLogic;
use fb_sim::math::V3;
use fb_sim::physics::{Body, BodyInput, BodyState, RUN_SPEED, StepEvents};
use fb_sim::scene::pal;
use fb_sim::world::World;

use crate::{Audit, Ctx, Out, Run, r3};

pub const AUDITS: [Audit; 3] = [
    Audit {
        name: "physics",
        run: Run::Global(physics),
        timed: false,
    },
    Audit {
        name: "input",
        run: Run::Global(input),
        timed: false,
    },
    Audit {
        name: "rules",
        run: Run::Global(rules),
        timed: false,
    },
];

// ------------------------------------------------------------------ physics feel

const LEDGES: [f64; 10] = [0.2, 0.35, 0.5, 0.65, 0.8, 1.0, 1.3, 1.6, 2.0, 2.4];

/// A flat test floor with ledges of increasing height along −x, and an ice patch.
fn test_world() -> World {
    let mut b = Builder::new(1, false);
    let o = PrimOpts::default;
    b.box_(0.0, -1.0, 0.0, 400.0, 2.0, 400.0, pal::BLUE, o());
    for (i, h) in LEDGES.into_iter().enumerate() {
        b.box_(-100.0 - i as f64 * 20.0, h / 2.0, 0.0, 6.0, h, 6.0, pal::BLUE, o());
    }
    let ice = PrimOpts {
        col: ColliderOpts {
            slip: 1.0,
            ..Default::default()
        },
        ..Default::default()
    };
    b.box_(100.0, 0.1, 0.0, 40.0, 0.2, 40.0, pal::BLUE, ice);
    b.world.finalize(0.0, &NoLogic);
    b.world
}

/// Steps a lone bean `ticks` times (sim time k·DT); stops early when `each` says so. Returns the ticks run.
fn run(
    world: &mut World,
    body: &mut Body,
    ticks: i64,
    mut input: impl FnMut(i64, &Body) -> BodyInput,
    mut each: impl FnMut(i64, &Body, &World) -> bool,
) -> i64 {
    let mut ev = StepEvents::default();
    for k in 1..=ticks {
        let i = input(k, body);
        let mut one = [Stepper {
            id: 1,
            body: &mut *body,
            ev: &mut ev,
            input: i,
        }];
        tick_plain(world, k as f64 * DT, &mut one, &[]);
        if each(k, body, world) {
            return k;
        }
    }
    ticks
}

const IDLE: BodyInput = BodyInput {
    mx: 0.0,
    mz: 0.0,
    jump: false,
    dive: false,
};
const FWD: BodyInput = BodyInput { mz: 1.0, ..IDLE };

fn speed(b: &Body) -> f64 {
    m::hypot(b.vel.x, b.vel.z)
}

fn idle(_: i64, _: &Body) -> BodyInput {
    IDLE
}

fn fwd(_: i64, _: &Body) -> BodyInput {
    FWD
}

fn never(_: i64, _: &Body, _: &World) -> bool {
    false
}

/// Measures how the bean handles (accelerate, stop, turn, jump, dive, climb, ice) and flags big changes.
pub(crate) fn physics(_: &Ctx, out: &mut Out) {
    let mut world = test_world();
    let w = &mut world;
    let fresh = |w: &mut World, x: f64, z: f64| {
        let mut b = Body::new(1);
        b.reset(V3::new(x, 0.02, z), 0.0);
        run(w, &mut b, 30, idle, never);
        b
    };
    // Acceleration to 90% of running speed.
    let mut b = fresh(w, 0.0, 0.0);
    let accel = run(w, &mut b, 240, fwd, |_, b, _| speed(b) >= RUN_SPEED * 0.9) as f64 * DT;
    // Stopping from full speed.
    run(w, &mut b, 60, fwd, never);
    let stop = run(w, &mut b, 240, idle, |_, b, _| speed(b) < 0.5) as f64 * DT;
    // Turning around at full speed.
    run(w, &mut b, 60, fwd, never);
    let back = BodyInput { mz: -1.0, ..IDLE };
    let turn = run(w, &mut b, 240, |_, _| back, |_, b, _| b.vel.z < -RUN_SPEED * 0.9) as f64 * DT;
    // Jump: apex and air time.
    let mut b = fresh(w, 0.0, 0.0);
    let y0 = b.pos.y;
    let mut apex: f64 = 0.0;
    let mut left = false;
    let jump_once = |k: i64, _: &Body| BodyInput { jump: k == 1, ..IDLE };
    let air = run(w, &mut b, 240, jump_once, |_, b, _| {
        apex = apex.at_least(b.pos.y - y0);
        left |= !b.grounded;
        left && b.grounded
    }) as f64
        * DT;
    // Running jump distance.
    let mut b = fresh(w, 0.0, 0.0);
    run(w, &mut b, 90, fwd, never);
    let jz = b.pos.z;
    let mut jumped = false;
    let run_jump = |k: i64, _: &Body| BodyInput { jump: k == 1, ..FWD };
    run(w, &mut b, 240, run_jump, |_, b, _| {
        jumped |= !b.grounded;
        jumped && b.grounded
    });
    let jump_dist = b.pos.z - jz;
    // Dive from a run: distance until back on the feet.
    let mut b = fresh(w, 0.0, 0.0);
    run(w, &mut b, 90, fwd, never);
    let dz0 = b.pos.z;
    let dive_once = |k: i64, _: &Body| BodyInput { dive: k == 1, ..FWD };
    let dive_time = run(w, &mut b, 480, dive_once, |k, b, _| {
        k > 5 && b.state == BodyState::Normal
    }) as f64
        * DT;
    let dive_dist = b.pos.z - dz0;
    // Climbing: the highest ledge walked onto, and jumped onto.
    let mut walk: f64 = 0.0;
    let mut jump_up: f64 = 0.0;
    for (i, h) in LEDGES.into_iter().enumerate() {
        let x = -100.0 - i as f64 * 20.0;
        for with_jump in [false, true] {
            let mut lb = Body::new(1);
            lb.reset(V3::new(x, 0.02, -8.0), 0.0);
            run(w, &mut lb, 20, idle, never);
            // On top at some point while over the ledge (the bean runs on past it).
            let mut top = -1.0f64;
            let climb = |_: i64, b: &Body| BodyInput {
                jump: with_jump && b.grounded && b.pos.z > -4.6 && b.pos.z < -3.2,
                ..FWD
            };
            run(w, &mut lb, 180, climb, |_, b, _| {
                if b.pos.z.abs() < 2.5 && b.grounded {
                    top = top.at_least(b.pos.y);
                }
                false
            });
            let on = top > h - 0.2;
            if on && !with_jump {
                walk = walk.at_least(h);
            }
            if on && with_jump {
                jump_up = jump_up.at_least(h);
            }
        }
    }
    // Ice: run onto it at full speed, let go, and see how much speed is left half a second later.
    let mut b = fresh(w, 100.0, -34.0);
    run(w, &mut b, 360, fwd, |_, b, w| {
        b.pos.z > -15.0 && b.ground_col.is_some_and(|c| w.col(c).opts.slip == 1.0)
    });
    let ice_v0 = speed(&b);
    run(w, &mut b, 60, idle, never);
    let ice_keep = speed(&b) / ice_v0.at_least(0.01);

    // Expected ranges: outside them the controls feel different from what the maps were built for.
    let measured = [
        ("accel", r3(accel), 0.05, 0.3),
        ("stop", r3(stop), 0.02, 0.3),
        ("turn", r3(turn), 0.05, 0.45),
        ("jump_apex", r3(apex), 1.4, 2.6),
        ("air_time", r3(air), 0.55, 1.0),
        ("jump_dist", r3(jump_dist), 4.0, 9.0),
        ("dive_dist", r3(dive_dist), 3.0, 9.0),
        ("dive_recover", r3(dive_time), 0.3, 1.6),
        ("step_walk", walk, 0.2, 0.65),
        ("step_jump", jump_up, 1.0, 2.4),
        ("ice_keep", r3(ice_keep), 0.3, 1.0),
    ];
    for (k, v, _, _) in measured {
        out.metric(k, v);
    }
    for (k, v, lo, hi) in measured {
        if v < lo || v > hi {
            out.warn(format!("{k} = {v} is outside the expected {lo}…{hi}"));
        }
    }
}

// ------------------------------------------------------------------ input handling

/// What of input handling the core owns: frames clamped as the server takes them, and a press acted on
/// in the tick it is for. (The wire format and the room's input buffer are tested in fb_net/fb_server.)
fn input(ctx: &Ctx, out: &mut Out) {
    let mut rng = Rng::new(ctx.seed);
    let n = if ctx.quick { 2000 } else { 20000 };
    let mut bad = 0;
    for _ in 0..n {
        let f = InputFrame {
            mx: (rng.unit() * 256.0 - 128.0).floor() as i8,
            mz: (rng.unit() * 256.0 - 128.0).floor() as i8,
            buttons: (rng.unit() * 256.0).floor() as u8,
        };
        let c = f.clamped();
        let l2 = i32::from(c.mx).pow(2) + i32::from(c.mz).pow(2);
        if c.clamped() != c || l2 > 127 * 127 || c.buttons > 7 {
            bad += 1;
        }
    }
    if bad > 0 {
        out.error(format!(
            "{bad} of {n} input frames were not clamped to unit length and 3 buttons"
        ));
    }
    let diag = InputFrame::from_stick(1.0, 1.0, 0);
    if m::hypot(f64::from(diag.mx), f64::from(diag.mz)) > 127.5 {
        out.error("diagonal input is not clamped to unit length");
    }

    // The arena acts on a press in the tick it is for.
    let map = fb_maps::by_id("jump-club").unwrap_or(fb_maps::GAMES[0]);
    let (mut a, _) = Arena::new(map, ArenaKind::Lobby, 1, 0, &[1], false);
    a.add_pawn(1, false);
    for k in 1..=120 {
        a.step(k, |_| InputFrame::IDLE);
    }
    let jump = InputFrame {
        buttons: BTN_JUMP,
        ..InputFrame::IDLE
    };
    a.step(121, |_| jump);
    if !a.pawn(1).is_some_and(|p| p.body.vel.y > 0.0) {
        out.error("a jump pressed for tick k did not start in tick k");
    }
    out.metric("fuzz_frames", n);
}

// ------------------------------------------------------------------ game rules and planning

fn rules(ctx: &Ctx, out: &mut Out) {
    let mut rng = Rng::new(ctx.seed);
    // Planning: every player count, mode and length gives a valid game.
    let mut plans = 0;
    for players in 1..=MAX_PLAYERS as u32 {
        for (mode, name) in [(Mode::Mix, "mix"), (Mode::Races, "races"), (Mode::Survival, "survival")] {
            for rounds in ROUND_COUNTS {
                let pl = Playlist {
                    mode,
                    games: Vec::new(),
                    rounds,
                };
                let plan = plan_game(players, &pl, &mut rng);
                plans += 1;
                if plan.len() != rounds as usize {
                    out.error(format!(
                        "{name}, {rounds} rounds, {players} players: planned {} rounds",
                        plan.len()
                    ));
                }
                for id in &plan {
                    match game(id) {
                        None => {
                            out.error(format!("planned an unknown game {id}"));
                        }
                        Some(g) => {
                            let need = g.min_players;
                            if need > players {
                                out.error(format!("planned {id} for {players} players (needs {need})"));
                            }
                            if mode == Mode::Races && g.genre != Genre::Race {
                                out.error(format!("races mode planned {id} ({})", g.genre.id()));
                            }
                            if mode == Mode::Survival && g.genre != Genre::Survival {
                                out.error(format!("survival mode planned {id} ({})", g.genre.id()));
                            }
                        }
                    }
                }
                let uniq: BTreeSet<&str> = plan.iter().copied().collect();
                if uniq.len() < plan.len().min(5) {
                    out.warn(format!("{name} {rounds} rounds repeats a game: {}", plan.join(", ")));
                }
            }
        }
    }
    out.metric("plans", plans);

    // Scoring: points within 0…10, totals never negative, better placement never scores less.
    let genres = [Genre::Race, Genre::Survival, Genre::Points];
    let total = if ctx.quick { 300 } else { 3000 };
    for i in 0..total {
        let n = 1 + (rng.unit() * 8.0).floor() as u32;
        let ids: Vec<u32> = (1..=n).collect();
        let genre = genres[i % 3];
        let mut shuffled = ids.clone();
        shuffle(&mut shuffled, &mut rng);
        let cut = rng.index(n as usize + 1);
        let scores: BTreeMap<u32, i64> = ids.iter().map(|&id| (id, (rng.unit() * 5.0).floor() as i64)).collect();
        let finished = if genre == Genre::Race { &shuffled[..cut] } else { &[] };
        let outs = if genre == Genre::Survival {
            &shuffled[..cut]
        } else {
            &[]
        };
        let view = RoundView {
            genre,
            participants: &ids,
            connected: &|_| true,
            finished,
            out: outs,
            scores: &scores,
            progress: &|id| id as f64 * 3.0,
            time_up: true,
            bots: None,
        };
        let stats: BTreeMap<u32, RoundStats> = ids
            .iter()
            .map(|&id| {
                let falls = (rng.unit() * 6.0).floor() as u32;
                let shortcuts = (rng.unit() * 2.0).floor() as u32;
                (
                    id,
                    RoundStats {
                        falls,
                        shortcuts,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let totals: BTreeMap<u32, i64> = ids.iter().map(|&id| (id, (rng.unit() * 30.0).floor() as i64)).collect();
        let rows = view.score_round(&stats, &totals, None);
        for r in &rows {
            if r.points < 0 || r.points > TOP_POINTS {
                out.error(format!("round scored {} placement points", r.points));
            }
            if r.total < 0 {
                out.error(format!("a total went negative ({})", r.total));
            }
            if r.place < 1 || r.place > n as usize {
                out.error(format!("place {} of {n}", r.place));
            }
        }
        let mut by_place: Vec<_> = rows.iter().collect();
        by_place.sort_by_key(|r| r.place);
        if let Some(w) = by_place.windows(2).find(|w| w[1].points > w[0].points) {
            out.error(format!(
                "place {} got more points than place {}",
                w[1].place, w[0].place
            ));
        }
    }
    out.metric("scored_rounds", total);
}
