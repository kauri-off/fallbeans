//! Walls rush across the platform, fast, each one different: pieces of every width, low ones to jump,
//! high bars to run under, windows to jump through, full blocks, and gaps. Find your way through (or
//! over) in time, or get swept off.
use std::sync::Arc;

use fb_shared::cause::Hazard;
use fb_sim::bots::{HumanOpts, Note, humanize, init_bot, steer};
use fb_sim::builder::{Builder, PrimOpts};
use fb_sim::collider::{ColId, ColliderOpts};
use fb_sim::looks::LookId;
use fb_sim::m::MinMax;
use fb_sim::map::{Brain, GameMeta, Genre, MapCtx, MapDef, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::{NodeId, ROOT};
use fb_sim::scene::{Palette, pal};

use crate::util::{deco, freq, o};

pub struct WallRush;

static META: GameMeta = GameMeta::new(
    "wall-rush",
    "Стенобой",
    Genre::Survival,
    "На платформу несутся стены — быстро и каждая по-своему: сплошные блоки, низкие стенки, балки сверху, окна и проёмы разной ширины. Найдите путь или перепрыгните — иначе снесёт!",
    "Не упадите",
    80.0,
);

/// What a stretch of wall is: solid, a gap, low (jump it), a bar overhead (run under), a window (jump through).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Solid,
    Gap,
    Low,
    Bar,
    Window,
}

const W: f64 = 20.0;
const START_Z: f64 = -58.0;
const END_Z: f64 = 16.0;
/// Jumpable heights: a jump clears about 1.9 m.
const LOW_H: f64 = 1.0;
const BAR_Y: f64 = 1.85;

#[derive(Clone, Copy, Debug)]
struct Piece {
    kind: Kind,
    x0: f64,
    x1: f64,
}

struct Wall {
    at: f64,
    speed: f64,
    pieces: Vec<Piece>,
    group: NodeId,
    cols: Vec<ColId>,
}

impl Wall {
    fn z(&self, t: f64) -> f64 {
        START_Z + (t - self.at) * self.speed
    }
}

impl MapDef for WallRush {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Desert, LookId::Factory, LookId::Castle]
    }

    fn build(&self, b: &mut Builder, _ctx: &MapCtx) -> MapSpec {
        let home_note: Note<f64> = b.note();
        let wall_note: Note<usize> = b.note();
        let gap_note: Note<usize> = b.note();
        b.box_(0.0, -1.0, 0.0, W, 2.0, 16.0, pal::BLUE, freq(0.3));
        b.box_(0.0, 0.02, -7.6, W, 0.05, 0.6, pal::RED, deco());
        b.box_(0.0, 0.02, 7.6, W, 0.05, 0.6, pal::RED, deco());
        for sx in [-1.0, 1.0] {
            b.box_(sx * (W / 2.0 + 0.4), 0.6, 0.0, 0.8, 1.2, 16.0, pal::PINK, o());
        }
        for k in 0..4 {
            b.bonus(-6.0 + k as f64 * 4.0, 0.0, -3.0 + (k % 2) as f64 * 6.0);
        }

        let mut walls: Vec<Wall> = Vec::new();
        let pals: [Palette; 5] = [pal::ORANGE, pal::PURPLE, pal::GREEN, pal::PINK, pal::TEAL];
        let mut at = 0.6;
        let mut k = 0;
        while k < 90 && at < META.duration {
            // Split the width into 3–6 pieces of random widths (at least 2.4 m, room for a bean).
            let n = 3 + b.rng.index(4);
            let mut cuts: Vec<f64> = (0..n - 1).map(|_| b.rng.unit()).collect();
            cuts.sort_by(f64::total_cmp);
            let mut edges = vec![0.0];
            edges.extend(cuts);
            edges.push(1.0);
            let edges: Vec<f64> = edges.iter().map(|f| -W / 2.0 + f * W).collect();
            let mut pieces: Vec<Piece> = Vec::new();
            for i in 0..n {
                let (x0, x1) = (edges[i], edges[i + 1]);
                if x1 - x0 < 1.2
                    && let Some(last) = pieces.last_mut()
                {
                    last.x1 = x1;
                    continue;
                }
                pieces.push(Piece {
                    kind: Kind::Solid,
                    x0,
                    x1,
                });
            }
            // Ways through: early walls have two, later ones one (sometimes two); what kind, by chance.
            let ways = if k < 4 || b.rng.unit() < 0.3 { 2 } else { 1 };
            let kinds: &[Kind] = if k < 2 {
                &[Kind::Gap]
            } else {
                &[Kind::Gap, Kind::Low, Kind::Bar, Kind::Window, Kind::Low, Kind::Bar]
            };
            let mut wide: Vec<usize> = (0..pieces.len())
                .filter(|&i| pieces[i].x1 - pieces[i].x0 >= 2.4)
                .collect();
            let mut w = 0;
            while w < ways && !wide.is_empty() {
                let p = wide.remove(b.rng.index(wide.len()));
                pieces[p].kind = kinds[b.rng.index(kinds.len())];
                w += 1;
            }
            if !pieces.iter().any(|p| p.kind != Kind::Solid) {
                pieces[0].kind = Kind::Gap;
            }
            // Three times the old pace: from 11 m/s up to about 19.
            let speed = 11.0 + 8f64.at_most(k as f64 * 0.22) + b.rng.unit() * 1.5;
            let group = b.anchor(0.0, 0.0, START_Z, ROOT);
            b.world.nodes.get_mut(group).visible = false;
            let p = pals[k % pals.len()];
            let thick = 0.8 + b.rng.unit() * 1.2;
            let mut cols = Vec::new();
            for pc in &pieces {
                let x = (pc.x0 + pc.x1) / 2.0;
                let w = pc.x1 - pc.x0;
                let mut add = |b: &mut Builder, y: f64, h: f64, c: Palette| {
                    let opts = PrimOpts {
                        parent: Some(group),
                        dynamic: true,
                        col: ColliderOpts {
                            tag: Some(Hazard::Wall),
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    cols.push(b.box_(x, y + h / 2.0, 0.0, w, h, thick, c, opts).col());
                };
                match pc.kind {
                    Kind::Solid => add(b, 0.0, 4.2, p),
                    Kind::Low => add(b, 0.0, LOW_H, pal::YELLOW),
                    Kind::Bar => add(b, BAR_Y, 4.2 - BAR_Y, p),
                    Kind::Window => {
                        // A hole to jump through: sill at 0.9 m, lintel at 3 m.
                        add(b, 0.0, 0.9, pal::YELLOW);
                        add(b, 3.0, 1.2, p);
                    }
                    Kind::Gap => {}
                }
            }
            walls.push(Wall {
                at,
                speed,
                pieces,
                group,
                cols,
            });
            at += 1.5f64.at_least(3.4 - k as f64 * 0.07) + b.rng.unit() * 0.6;
            k += 1;
        }
        let walls = Arc::new(walls);
        let ws = walls.clone();
        b.mover(move |t, ctx| {
            for w in ws.iter() {
                let z = w.z(t);
                let on = t >= w.at && z < END_Z;
                let n = ctx.node(w.group);
                n.visible = on;
                n.pos.z = if on { z } else { START_Z };
                for &c in &w.cols {
                    ctx.set_enabled(c, on);
                }
            }
        });
        b.clouds(0.0, -20.0, 50.0);

        let spawns = (0..8).map(|i| V3::new(-7.0 + i as f64 * 2.0, 0.1, 2.0)).collect();
        MapSpec {
            spawns,
            kill_y: -8.0,
            view: Some(V3::new(0.0, 2.0, -6.0)),
            logic: Box::new(Brain(Box::new(move |bot, out| {
                init_bot(bot);
                let p = bot.body.pos;
                let t = bot.t;
                if bot.mem.get(home_note).is_none() {
                    let home = 1.0 + bot.rng.unit() * 4.0;
                    bot.mem.set(home_note, home);
                }
                let home = bot.mem.get(home_note).unwrap_or(2.0);
                // The next wall that has not passed us yet (and is on its way).
                let Some(wi) = walls.iter().position(|w| t >= w.at - 0.5 && w.z(t) < p.z + 0.8) else {
                    steer(bot, p.x, home, out, bot.mem.traits().spd);
                    humanize(bot, out, &HumanOpts::default());
                    return;
                };
                let next = &walls[wi];
                if bot.mem.get(wall_note) != Some(wi) {
                    bot.mem.set(wall_note, wi);
                    let options: Vec<usize> = (0..next.pieces.len())
                        .filter(|&i| next.pieces[i].kind != Kind::Solid)
                        .collect();
                    let mid = |i: usize| (next.pieces[i].x0 + next.pieces[i].x1) / 2.0;
                    let skill = bot.rng.unit();
                    let choice = if skill < 0.85 {
                        let mut a = options[0];
                        for &o in &options[1..] {
                            if (mid(o) - p.x).abs() < (mid(a) - p.x).abs() {
                                a = o;
                            }
                        }
                        Some(a)
                    } else {
                        options.first().copied()
                    };
                    bot.mem.set(gap_note, choice.unwrap_or(0));
                }
                let seg = bot.mem.get(gap_note).unwrap_or(0);
                let pc = next.pieces.get(seg).unwrap_or(&next.pieces[0]);
                let tx = (pc.x0 + 0.7).at_least((pc.x1 - 0.7).at_most((pc.x0 + pc.x1) / 2.0));
                let eta = (p.z - next.z(t)) / next.speed;
                steer(bot, tx, (-5f64).at_least(5f64.at_most(home)), out, 1.0);
                // Over a low wall or through a window: jump just before it arrives.
                let inside = p.x > pc.x0 + 0.4 && p.x < pc.x1 - 0.4;
                if matches!(pc.kind, Kind::Low | Kind::Window)
                    && inside
                    && bot.body.grounded
                    && eta < 0.2 + bot.mem.traits().react * 0.3
                    && eta > 0.08
                {
                    out.jump = true;
                }
                let opts = HumanOpts {
                    precise: eta < 2.0,
                    fun: eta > 3.0,
                    ..Default::default()
                };
                humanize(bot, out, &opts);
            }))),
            ..Default::default()
        }
    }
}
