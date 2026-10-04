//! Glass bridges with one safe pane per row. A fake pane is not solid: whoever steps on it falls straight
//! through (it shatters for everybody to see). Panes that held light up green for everybody. Gloves punch
//! across some rows of the later bridges.
use std::sync::Arc;

use fb_sim::bots::{BotInput, BotView, HumanOpts, Waypoint, humanize, init_bot, steer};
use fb_sim::builder::Builder;
use fb_sim::collider::{ColId, ColliderOpts, Shape};
use fb_sim::course::{
    CourseOpts, SegOut, Segment, glove_alley, moving_platforms, pick_sections, pistons, race_course, rotor_decks,
    seg_emit, tipping_bridge, with_rests,
};
use fb_sim::map::{Cx, GameMeta, Genre, MapCtx, MapDef, MapSpec, json};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::props::{Glove, GloveOpts, glove_puncher};
use fb_sim::scene::{Finish, Form, Part, Piece, pal};
use fb_sim::world::St;

use crate::util::{deco, o};

pub struct HiddenBridge;

static META: GameMeta = GameMeta::new(
    "hidden-bridge",
    "Невидимый мост",
    Genre::Race,
    "Три стеклянных моста: ложная панель лопается от первого касания, и оттолкнуться от неё уже не выйдет. Смотрите, где упали другие, и идите по зелёным! Дальше мосты уже и с перчатками.",
    "Найдите путь и добегите до финиша",
    150.0,
);

const PANE: f64 = 2.6;
const THICK: f64 = 0.3;
const SHARDS: usize = 9;

/// A pane as built.
#[derive(Clone, Copy, Debug)]
struct Tile {
    col: usize,
    x: f64,
    z: f64,
    real: bool,
    collider: ColId,
}

/// What happened to the panes: trusted (held somebody), fell (when).
#[derive(Clone, Copy, Debug, Default)]
struct Pane {
    trusted: bool,
    fall_at: Option<f64>,
}

fn break_tile(cx: &mut Cx, st: St<Vec<Pane>>, real: bool, i: usize, at: f64) {
    let pane = &mut cx.world.st_mut(st)[i];
    if real || pane.fall_at.is_some() {
        return;
    }
    pane.fall_at = Some(at);
    cx.sfx("break");
}

fn glass_bridge(id: usize, rows: usize, cols: usize, glove_rows: &'static [usize]) -> Segment {
    Box::new(move |s| {
        let y = s.y;
        let dx = 3.0;
        let dz = 3.2;
        let z0 = s.z + 1.3 + PANE / 2.0;
        let mut tiles: Vec<Tile> = Vec::new();
        let mut rows_of: Vec<Vec<usize>> = Vec::new();
        let mut c = (s.rng() * cols as f64).floor() as i64;
        for r in 0..rows {
            if r > 0 {
                c = 0.max((cols as i64 - 1).min(c + (s.rng() * 3.0).floor() as i64 - 1));
            }
            let mut row = Vec::new();
            for k in 0..cols {
                let x = (k as f64 - (cols as f64 - 1.0) / 2.0) * dx;
                let zz = z0 + r as f64 * dz;
                let real = k as i64 == c;
                let at = s.b.anchor(x, y - THICK / 2.0, zz, ROOT);
                let collider = s.b.collider(
                    at,
                    Shape::Box {
                        hx: PANE / 2.0,
                        hy: THICK / 2.0,
                        hz: PANE / 2.0,
                    },
                    ColliderOpts {
                        is_static: true,
                        trigger: !real,
                        ..Default::default()
                    },
                );
                row.push(tiles.len());
                tiles.push(Tile {
                    col: k,
                    x,
                    z: zz,
                    real,
                    collider,
                });
            }
            rows_of.push(row);
        }
        let st: St<Vec<Pane>> = s.b.state(vec![Pane::default(); tiles.len()]);
        let (safe_key, tile_key) = (s.event("safe"), s.event("tile"));
        for (i, tile) in tiles.iter().enumerate() {
            let real = tile.real;
            let (safe_key, tile_key) = (safe_key.clone(), tile_key.clone());
            let touched = move |cx: &mut Cx| {
                let pane = cx.world.st(st)[i];
                if real {
                    if !pane.trusted && cx.server {
                        seg_emit(cx, &safe_key, json!({ "i": i }));
                    }
                    return;
                }
                if pane.fall_at.is_some() || cx.t < 0.0 {
                    return;
                }
                let now = cx.t;
                if cx.server {
                    seg_emit(cx, &tile_key, json!({ "i": i, "at": now }));
                } else {
                    break_tile(cx, st, real, i, now);
                }
            };
            let touched2 = touched.clone();
            s.b.on_ground(tile.collider, move |cx, _, _, _| touched(cx));
            // Brushing a fake pane's side from the next pane does not break it.
            s.b.on_touch(tile.collider, move |cx, _, _, t| {
                if t.normal.is_some_and(|n| n.y > 0.3) {
                    touched2(cx);
                }
            });
        }
        let reals: Vec<bool> = tiles.iter().map(|t| t.real).collect();
        let reals2 = reals.clone();
        let parse = |d: &fb_sim::map::Value| {
            d["i"]
                .as_u64()
                .filter(|&i| i <= 255)
                .map(|i| (i as usize, d["at"].as_f64()))
        };
        s.on("safe", move |cx, d| {
            if let Some((i, _)) = parse(d)
                && reals.get(i) == Some(&true)
            {
                cx.world.st_mut(st)[i].trusted = true;
            }
        });
        s.on("tile", move |cx, d| {
            let Some((i, Some(at))) = parse(d) else { return };
            if reals2.get(i) != Some(&false) {
                return;
            }
            // The server's time wins over the local guess (the collider follows it).
            if cx.world.st(st)[i].fall_at.is_none() {
                break_tile(cx, st, false, i, at);
            } else {
                cx.world.st_mut(st)[i].fall_at = Some(at);
            }
        });
        let cols_of: Vec<ColId> = tiles.iter().map(|t| t.collider).collect();
        s.b.mover(move |t, ctx| {
            for (i, &c) in cols_of.iter().enumerate() {
                let on = ctx.st(st)[i].fall_at.is_none_or(|at| t < at);
                ctx.set_enabled(c, on);
            }
        });
        let end = z0 + (rows as f64 - 1.0) * dz + PANE / 2.0;
        s.b.box_(0.0, y - 1.0, s.z + 0.65, 16.0, 2.0, 1.3, pal::PURPLE, o());
        let half = ((cols as f64 - 1.0) / 2.0) * dx + PANE / 2.0 + 1.1;
        let gloves: Vec<(usize, Glove, f64)> = glove_rows
            .iter()
            .enumerate()
            .map(|(k, &row)| {
                let side = if k % 2 == 1 { 1.0 } else { -1.0 };
                let rest = side * (half + 1.6);
                let w = 1.1 + s.rng() * 0.3;
                let ph = s.rng() * 6.0;
                let g = glove_puncher(
                    s.b,
                    GloveOpts {
                        x: rest,
                        y: y + 0.95,
                        z: z0 + row as f64 * dz,
                        side,
                        w,
                        ph,
                        reach: half + 0.6,
                        scale: 1.35,
                        post_to: Some(y - 4.8),
                    },
                );
                (row, g, rest)
            })
            .collect();
        if !s.b.server() {
            let parts = [
                Part::toned(Form::Box([PANE, THICK, PANE]), "#bfe9ffa6", "#8ceaa2c0", Finish::Glass).on("glass"),
                Part::new(Form::Box([PANE, 0.14, 0.12]), "#9aa3c7", Finish::Metal).on("metal"),
                Part::new(Form::Box([0.12, 0.14, PANE]), "#9aa3c7", Finish::Metal).on("metal"),
                Part::new(Form::Box([0.7, 0.12, 0.7]), "#d9f3ffb0", Finish::Glass).on("glass"),
            ];
            let panes: Vec<(f64, f64)> = tiles.iter().map(|t| (t.x, t.z)).collect();
            // Shards fly apart in a fixed pattern per pane.
            let spread: Vec<[f64; 4]> = (0..SHARDS)
                .map(|k| {
                    let ox = ((k % 3) as f64 - 1.0) * 0.8;
                    let oz = ((k / 3) as f64 - 1.0) * 0.8;
                    [ox, oz, 3.0 + ((k * 7) % 5) as f64, 1.0 + ((k * 3) % 4) as f64 * 0.6]
                })
                .collect();
            s.b.special_look(ROOT, "glass-bridge", &parts, move |w, t, out| {
                let state = w.st(st);
                let py = y - THICK / 2.0;
                let e = PANE / 2.0 - 0.06;
                let bar = THICK / 2.0 + 0.02;
                for (&(x, z), pane) in panes.iter().zip(state) {
                    let fell = pane.fall_at.filter(|&at| t >= at);
                    if fell.is_none() {
                        out.pieces
                            .push(Piece::at(0, x, py, z).tone(if pane.trusted { 1.0 } else { 0.0 }));
                        out.pieces.push(Piece::at(1, x, py + bar, z + e));
                        out.pieces.push(Piece::at(1, x, py + bar, z - e));
                        out.pieces.push(Piece::at(2, x + e, py + bar, z));
                        out.pieces.push(Piece::at(2, x - e, py + bar, z));
                        continue;
                    }
                    let f = t - fell.unwrap_or(t);
                    if f > 2.5 {
                        continue;
                    }
                    for &[ox, oz, spin, up] in &spread {
                        let p = Piece::at(
                            3,
                            x + ox * (1.0 + f * 1.5),
                            py + up * f - 14.0 * f * f,
                            z + oz * (1.0 + f * 1.5),
                        );
                        out.pieces.push(p.rot(f * spin, f * spin * 0.7, f * spin * 0.3));
                    }
                }
            });
            // Girders along both sides (scenery, well outside the panes).
            for sx in [-1.0, 1.0] {
                let l = end - z0 + PANE + 1.2;
                s.b.box_(
                    sx * half,
                    y - 1.1,
                    (z0 - PANE / 2.0 + end) / 2.0,
                    0.5,
                    0.7,
                    l,
                    pal::hex("#7d86ad"),
                    deco(),
                );
            }
        }
        s.b.box_(0.0, y - 1.0, end + 0.1 + 3.0, 16.0, 2.0, 6.0, pal::PURPLE, o());

        let tiles = Arc::new(tiles);
        let key = |n: &str| format!("{n}{id}");
        let (k_col, k_row, k_wait) = (key("col"), key("row"), key("wait"));
        // Like a player: pick a pane (a green one if any), hesitate before a guess, step straight across.
        let drive = move |bot: &mut BotView, out: &mut BotInput| {
            init_bot(bot);
            let p = bot.body.pos;
            if p.z > end + 0.4 {
                return false;
            }
            let panes = bot.world.st(st);
            let Some(ri) = rows_of
                .iter()
                .position(|r| r.first().map_or(0.0, |&i| tiles[i].z) > p.z + 0.5)
            else {
                // Off the last pane straight ahead (a diagonal step could land on a fake one beside it).
                let x = if p.z < end + 0.5 {
                    p.x
                } else {
                    bot.mem.off.unwrap_or(0.0) * 3.0
                };
                steer(bot, x, end + 3.0, out, bot.mem.spd.unwrap_or(1.0));
                return true;
            };
            let row = &rows_of[ri];
            let col = bot.mem.get(&k_col);
            let chosen = row.iter().copied().find(|&i| Some(tiles[i].col as f64) == col);
            if bot.mem.get(&k_row) != Some(ri as f64) || chosen.is_none_or(|i| panes[i].fall_at.is_some()) {
                bot.mem.set(&k_row, ri as f64);
                let cur = col.unwrap_or((row.len() / 2) as f64);
                let open: Vec<usize> = row
                    .iter()
                    .copied()
                    .filter(|&i| panes[i].fall_at.is_none() && (tiles[i].col as f64 - cur).abs() <= 1.0)
                    .collect();
                let choices = if !open.is_empty() {
                    open
                } else {
                    row.iter().copied().filter(|&i| panes[i].fall_at.is_none()).collect()
                };
                let known = choices.iter().copied().find(|&i| panes[i].trusted);
                let real = choices.iter().copied().find(|&i| tiles[i].real);
                // Nobody has stood on this row yet: a guess (a good eye sometimes spots the right pane).
                let eye = 0.1 + bot.mem.skill.unwrap_or(0.7) * 0.12;
                let pick = match known {
                    Some(k) => Some(k),
                    None => {
                        if real.is_some() && bot.rng.next() < eye {
                            real
                        } else {
                            choices
                                .get((bot.rng.next() * choices.len() as f64).floor() as usize)
                                .copied()
                        }
                    }
                };
                bot.mem.set(&k_col, pick.map_or(cur, |i| tiles[i].col as f64));
                let wait = bot.t
                    + if known.is_some() {
                        0.05
                    } else {
                        0.3 + bot.rng.next() * 0.6
                    };
                bot.mem.set(&k_wait, wait);
            }
            let col = bot.mem.get(&k_col);
            let target = tiles[row
                .iter()
                .copied()
                .find(|&i| Some(tiles[i].col as f64) == col)
                .unwrap_or(row[0])];
            let standing = bot.body.grounded && bot.body.ground_col >= 0;
            if bot.t < bot.mem.get(&k_wait).unwrap_or(0.0) && standing {
                out.mx = 0.0;
                out.mz = 0.0;
                return true;
            }
            // A glove row ahead: step onto it only when the glove has pulled back for a while.
            if let Some((_, g, rest)) = gloves.iter().find(|g| g.0 == ri)
                && standing
                && [0.0, 0.3, 0.6, 0.9]
                    .iter()
                    .any(|dt| (g.x_at(bot.t + dt) - rest).abs() > 1.5)
            {
                out.mx = 0.0;
                out.mz = 0.0;
                return true;
            }
            // Line up first, then step straight across: cutting the corner would cross a broken pane's hole.
            let on = tiles.iter().find(|t| t.collider as i32 == bot.body.ground_col);
            let exit_x = on.map_or(target.x, |on| (on.x - 0.95).max((on.x + 0.95).min(target.x)));
            let exit_z = on.map_or(target.z - PANE / 2.0 - 0.4, |on| on.z + PANE / 2.0) - 0.45;
            let aligned = (p.x - exit_x).abs() < 0.35 || p.z > exit_z + 0.2;
            if standing && !aligned && p.z < target.z - PANE / 2.0 {
                steer(bot, exit_x, exit_z.min(p.z.max(exit_z - 1.0)), out, 0.7);
            } else {
                steer(bot, target.x, target.z + 0.3, out, bot.mem.spd.unwrap_or(1.0));
            }
            let opts = HumanOpts {
                precise: true,
                ..Default::default()
            };
            humanize(bot, out, &opts);
            true
        };
        SegOut {
            z: end + 6.1,
            y,
            routes: vec![vec![
                Waypoint::w(0.0, end + 1.5, 0.5).drive(drive),
                Waypoint::w(0.0, end + 3.4, 1.0),
            ]],
            checkpoint: Some((end + 0.9, V3::new(0.0, y + 0.1, end + 3.4))),
            ..Default::default()
        }
    })
}

impl MapDef for HiddenBridge {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [&'static str] {
        &["starlight", "neon", "ocean"]
    }

    fn build(&self, b: &mut Builder, ctx: &MapCtx) -> MapSpec {
        let pool = vec![
            rotor_decks(1),
            moving_platforms(4),
            glove_alley(3),
            pistons(3, 14.0),
            tipping_bridge(5),
        ];
        let mut mids = pick_sections(&mut b.rng, pool, 2).into_iter();
        let mut rows = |lo: usize| lo + (b.rng.next() * 3.0).floor() as usize;
        let (r0, r1, r2) = (rows(8), rows(6), rows(5));
        let sections = vec![
            glass_bridge(0, r0, 5, &[]),
            mids.next().unwrap(),
            glass_bridge(1, r1, 4, &[2, 5]),
            mids.next().unwrap(),
            glass_bridge(2, r2, 3, &[1, 3]),
        ];
        let opts = CourseOpts {
            sections: with_rests(sections, 6.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
