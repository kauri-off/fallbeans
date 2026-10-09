//! Glass bridges with one safe pane per row. A fake pane is not solid: whoever steps on it falls straight
//! through (it shatters for everybody to see). Panes that held light up green for everybody. Gloves punch
//! across some rows of the later bridges.
use fb_shared::{rgb, rgba};
use fb_sim::bots::{BotInput, BotView, HumanOpts, Note, Waypoint, humanize, init_bot, steer};
use fb_sim::builder::Builder;
use fb_sim::collider::{ColId, ColliderOpts, Shape};
use fb_sim::course::{
    CourseOpts, Section, SegOut, Segment, glove_alley, moving_platforms, pick_sections, pistons, race_course,
    rotor_decks, tipping_bridge, with_rests,
};
use fb_sim::looks::LookId;
use fb_sim::m::MinMax;
use fb_sim::map::{Cx, GameMeta, Genre, Hook, MapCtx, MapDef, MapId, MapSfx, MapSpec, SegEvent, Steer};
use fb_sim::math::{V3, v3};
use fb_sim::nodes::ROOT;
use fb_sim::physics::{Body, StepEvents, Touch};
use fb_sim::props::{Glove, GloveOpts, glove_puncher};
use fb_sim::scene::Surface;
use fb_sim::scene::{Finish, Form, LookOut, Part, Piece, pal};
use fb_sim::world::{MoveCtx, World};

use crate::util::{deco, o};

pub struct HiddenBridge;

static META: GameMeta = GameMeta::new(
    MapId::HiddenBridge,
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

/// How the client draws a bridge: its height, its panes (x, z) and how shards fly.
struct GlassLook {
    hook: Hook,
    y: f64,
    panes: Vec<(f64, f64)>,
    spread: Vec<[f64; 4]>,
}

/// A glass bridge: its panes, what happened to them, and how bots cross it.
struct GlassBridge {
    seg: u32,
    /// By collider (ascending).
    tiles: Vec<Tile>,
    panes: Vec<Pane>,
    /// Tiles row by row.
    rows: Vec<Vec<usize>>,
    end: f64,
    /// Gloves punching across rows: (row, glove, its rest x).
    gloves: Vec<(usize, Glove, f64)>,
    /// The waypoint where bots cross.
    hook: Hook,
    k_col: Note<usize>,
    k_row: Note<usize>,
    k_wait: Note<f64>,
    look: Option<GlassLook>,
}

impl GlassBridge {
    fn break_tile(&mut self, cx: &mut Cx, real: bool, i: usize, at: f64) {
        let pane = &mut self.panes[i];
        if real || pane.fall_at.is_some() {
            return;
        }
        pane.fall_at = Some(at);
        cx.sfx(MapSfx::Break);
    }

    fn touched(&mut self, cx: &mut Cx, i: usize) {
        let pane = self.panes[i];
        if self.tiles[i].real {
            if !pane.trusted && cx.server {
                self.emit(
                    cx,
                    self.seg,
                    SegEvent::Safe(u32::try_from(i).expect("fewer than 2³² panes")),
                );
            }
            return;
        }
        if pane.fall_at.is_some() || cx.t < 0.0 {
            return;
        }
        // Server only: clients break it on the server's event (a break they predicted could not be taken back
        // when the server disagrees).
        if cx.server {
            let now = cx.t;
            let i = u32::try_from(i).expect("fewer than 2³² panes");
            self.emit(cx, self.seg, SegEvent::Fall { i, at: now });
        }
    }
}

impl Section for GlassBridge {
    fn event(&mut self, cx: &mut Cx, ev: &SegEvent) {
        let real = |i: u32| self.tiles.get(i as usize).map(|t| t.real);
        match *ev {
            SegEvent::Safe(i) if real(i) == Some(true) => {
                self.panes[i as usize].trusted = true;
            }
            SegEvent::Fall { i, at } if real(i) == Some(false) => {
                let i = i as usize;
                // (Heard again, e.g. by a late joiner: the server's time, which the collider follows.)
                if self.panes[i].fall_at.is_none() {
                    self.break_tile(cx, false, i, at);
                } else {
                    self.panes[i].fall_at = Some(at);
                }
            }
            _ => {}
        }
    }

    fn touch(&mut self, cx: &mut Cx, _: &mut Body, _: &mut StepEvents, tc: Touch) {
        let Ok(i) = self.tiles.binary_search_by_key(&tc.col, |t| t.collider) else {
            return;
        };
        // Brushing a fake pane's side from the next pane does not break it.
        if tc.normal.is_none_or(|n| n.y > 0.3) {
            self.touched(cx, i);
        }
    }

    fn pose(&self, t: f64, ctx: &mut MoveCtx) {
        for (tile, pane) in self.tiles.iter().zip(&self.panes) {
            ctx.set_enabled(tile.collider, pane.fall_at.is_none_or(|at| t < at));
        }
    }

    fn look(&self, hook: Hook, _: &World, t: f64, out: &mut LookOut) {
        let Some(GlassLook {
            hook: _,
            y,
            panes,
            spread,
        }) = self.look.as_ref().filter(|l| l.hook == hook)
        else {
            return;
        };
        let py = *y - THICK / 2.0;
        let e = PANE / 2.0 - 0.06;
        let bar = THICK / 2.0 + 0.02;
        for (&(x, z), pane) in panes.iter().zip(&self.panes) {
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
            for &[ox, oz, spin, up] in spread {
                let p = Piece::at(
                    3,
                    x + ox * (1.0 + f * 1.5),
                    py + up * f - 14.0 * f * f,
                    z + oz * (1.0 + f * 1.5),
                );
                out.pieces.push(p.rot(f * spin, f * spin * 0.7, f * spin * 0.3));
            }
        }
    }

    /// Like a player: pick a pane (a green one if any), hesitate before a guess, step straight across.
    fn steer(&self, hook: Hook, bot: &mut BotView, out: &mut BotInput) -> Steer {
        if hook != self.hook {
            return Steer::Follow;
        }
        let (tiles, rows_of, panes, end) = (&self.tiles, &self.rows, &self.panes, self.end);
        init_bot(bot);
        let p = bot.body.pos;
        if p.z > end + 0.4 {
            return Steer::Follow;
        }
        let Some(ri) = rows_of
            .iter()
            .position(|r| r.first().map_or(0.0, |&i| tiles[i].z) > p.z + 0.5)
        else {
            // Off the last pane straight ahead (a diagonal step could land on a fake one beside it).
            let x = if p.z < end + 0.5 {
                p.x
            } else {
                bot.mem.traits().off * 3.0
            };
            steer(bot, x, end + 3.0, out, bot.mem.traits().spd);
            return Steer::Drove;
        };
        let row = &rows_of[ri];
        let col = bot.mem.get(self.k_col);
        let chosen = row.iter().copied().find(|&i| Some(tiles[i].col) == col);
        // Only what anyone can see: panes that broke, panes that held somebody (green).
        let lit = |i: usize| panes[i].trusted;
        let turned_green = chosen.is_some_and(|i| !lit(i)) && row.iter().any(|&i| lit(i));
        if bot.mem.get(self.k_row) != Some(ri) || chosen.is_none_or(|i| panes[i].fall_at.is_some()) || turned_green {
            bot.mem.set(self.k_row, ri);
            // Standing on the row before, the bot is on its safe pane: the next one is within a column of
            // it. Anywhere else (the platform before the bridge, back after a fall) any pane may be it.
            let under = ri.checked_sub(1).and_then(|r| {
                rows_of[r].iter().copied().find(|&i| {
                    (tiles[i].x - p.x).abs() <= PANE / 2.0 + 0.2 && (tiles[i].z - p.z).abs() <= PANE / 2.0 + 0.2
                })
            });
            let cur = under.map(|i| tiles[i].col);
            let open: Vec<usize> = row.iter().copied().filter(|&i| panes[i].fall_at.is_none()).collect();
            let near: Vec<usize> = open
                .iter()
                .copied()
                .filter(|&i| cur.is_none_or(|c| tiles[i].col.abs_diff(c) <= 1))
                .collect();
            let choices = if near.is_empty() { open } else { near };
            let known = choices.iter().copied().find(|&i| lit(i));
            // Nobody has stood on this row yet: a guess, after a look round (the careful ones look
            // longer, in case somebody else tries first).
            let pick = known.or_else(|| choices.get(bot.rng.index(choices.len())).copied());
            bot.mem
                .set(self.k_col, pick.map_or(cur.unwrap_or(row.len() / 2), |i| tiles[i].col));
            let wait = bot.t
                + if known.is_some() {
                    0.05
                } else {
                    0.3 + bot.rng.unit() * 0.6 + bot.mem.traits().skill * 0.4
                };
            bot.mem.set(self.k_wait, wait);
        }
        let col = bot.mem.get(self.k_col);
        let target = tiles[row
            .iter()
            .copied()
            .find(|&i| Some(tiles[i].col) == col)
            .unwrap_or(row[0])];
        let standing = bot.body.grounded && bot.body.ground_col.is_some();
        if bot.t < bot.mem.get(self.k_wait).unwrap_or(0.0) && standing {
            out.mx = 0.0;
            out.mz = 0.0;
            return Steer::Drove;
        }
        // A glove row ahead: step onto it only when the glove has pulled back for a while.
        if let Some((_, g, rest)) = self.gloves.iter().find(|g| g.0 == ri)
            && standing
            && [0.0, 0.3, 0.6, 0.9]
                .iter()
                .any(|dt| (g.x_at(bot.t + dt) - rest).abs() > 1.5)
        {
            out.mx = 0.0;
            out.mz = 0.0;
            return Steer::Drove;
        }
        // Line up first, then step straight across: cutting the corner would cross a broken pane's hole.
        let on = tiles.iter().find(|t| Some(t.collider) == bot.body.ground_col);
        let exit_x = on.map_or(target.x, |on| (on.x - 0.95).at_least((on.x + 0.95).at_most(target.x)));
        let exit_z = on.map_or(target.z - PANE / 2.0 - 0.4, |on| on.z + PANE / 2.0) - 0.45;
        let aligned = (p.x - exit_x).abs() < 0.35 || p.z > exit_z + 0.2;
        if standing && !aligned && p.z < target.z - PANE / 2.0 {
            steer(bot, exit_x, exit_z.at_most(p.z.at_least(exit_z - 1.0)), out, 0.7);
        } else {
            steer(bot, target.x, target.z + 0.3, out, bot.mem.traits().spd);
        }
        let opts = HumanOpts {
            precise: true,
            ..Default::default()
        };
        humanize(bot, out, &opts);
        Steer::Drove
    }
}

#[expect(clippy::cast_possible_truncation, reason = "a column below cols")]
fn glass_bridge(rows: usize, cols: usize, glove_rows: &'static [usize]) -> Segment {
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
                let at = s.b.anchor(v3(x, y - THICK / 2.0, zz), ROOT);
                let collider = s.b.collider(
                    at,
                    Shape::Box {
                        hx: PANE / 2.0,
                        hy: THICK / 2.0,
                        hz: PANE / 2.0,
                    },
                    // Kept out of the bots' grid, the real ones too (it would tell them apart): bots go
                    // by what anyone can see.
                    ColliderOpts {
                        is_static: true,
                        trigger: !real,
                        nav_skip: true,
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
        for tile in &tiles {
            s.b.watch_ground(tile.collider);
            // (Brushing a fake pane's side from the next pane does not break it: `Section::touch`.)
            s.b.watch_touch(tile.collider);
        }
        let end = z0 + (rows as f64 - 1.0) * dz + PANE / 2.0;
        s.b.box_(v3(0.0, y - 1.0, s.z + 0.65), v3(16.0, 2.0, 1.3), pal::PURPLE, o());
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
        let mut look = None;
        if !s.b.server() {
            let parts = [
                Part::toned(
                    Form::Box([PANE, THICK, PANE]),
                    rgba(0xbfe9ffa6),
                    rgba(0x8ceaa2c0),
                    Finish::Glass,
                )
                .on(Surface::Glass),
                Part::new(Form::Box([PANE, 0.14, 0.12]), rgb(0x9aa3c7), Finish::Metal).on(Surface::Metal),
                Part::new(Form::Box([0.12, 0.14, PANE]), rgb(0x9aa3c7), Finish::Metal).on(Surface::Metal),
                Part::new(Form::Box([0.7, 0.12, 0.7]), rgba(0xd9f3ffb0), Finish::Glass).on(Surface::Glass),
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
            let hook = s.b.special_hook(ROOT, "glass-bridge", &parts);
            look = Some(GlassLook { hook, y, panes, spread });
            // Girders along both sides (scenery, well outside the panes).
            for sx in [-1.0, 1.0] {
                let l = end - z0 + PANE + 1.2;
                s.b.box_(
                    v3(sx * half, y - 1.1, (z0 - PANE / 2.0 + end) / 2.0),
                    v3(0.5, 0.7, l),
                    pal::solid(rgb(0x7d86ad)),
                    deco(),
                );
            }
        }
        s.b.box_(v3(0.0, y - 1.0, end + 0.1 + 3.0), v3(16.0, 2.0, 6.0), pal::PURPLE, o());

        let (k_col, k_row, k_wait): (Note<usize>, Note<usize>, Note<f64>) = (s.b.note(), s.b.note(), s.b.note());
        let hook = s.b.hook();
        let seg = s.seg();
        let panes = vec![Pane::default(); tiles.len()];
        s.rules(GlassBridge {
            seg,
            tiles,
            panes,
            rows: rows_of,
            end,
            gloves,
            hook,
            k_col,
            k_row,
            k_wait,
            look,
        });
        SegOut {
            z: end + 6.1,
            y,
            routes: vec![vec![
                Waypoint::spread(0.0, end + 1.5, 0.5).hook(hook),
                Waypoint::spread(0.0, end + 3.4, 1.0),
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

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Starlight, LookId::Neon, LookId::Ocean]
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
        let mut rows = |lo: usize| lo + b.rng.index(3);
        let (r0, r1, r2) = (rows(8), rows(6), rows(5));
        let sections = vec![
            glass_bridge(r0, 5, &[]),
            mids.next().unwrap(),
            glass_bridge(r1, 4, &[2, 5]),
            mids.next().unwrap(),
            glass_bridge(r2, 3, &[1, 3]),
        ];
        let opts = CourseOpts {
            sections: with_rests(sections, 6.0),
            ..Default::default()
        };
        race_course(b, ctx, opts)
    }
}
