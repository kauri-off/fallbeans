//! Tiles drop half a second after somebody stands on them, three floors one under the other: the last
//! bean standing wins.
use std::collections::BTreeMap;

use fb_shared::rgb;
use fb_sim::bots::{BotInput, BotView, HumanOpts, LandCheck, Note, humanize, init_bot, unstick};
use fb_sim::builder::Builder;
use fb_sim::collider::{ColId, ColliderOpts, Shape};
use fb_sim::looks::LookId;
use fb_sim::m::{self, MinMax};
use fb_sim::map::{Cx, GameMeta, Genre, Hook, MapCtx, MapDef, MapEvent, MapLogic, MapSpec};
use fb_sim::math::V3;
use fb_sim::nodes::ROOT;
use fb_sim::physics::{Body, StepEvents, Touch};
use fb_sim::scene::Surface;
use fb_sim::scene::{Finish, Form, LookOut, Palette, Part, Piece, pal};
use fb_sim::world::{MoveCtx, World};

pub struct HexAGone;

static META: GameMeta = GameMeta {
    finale: true,
    ..GameMeta::new(
        "hex-a-gone",
        "Хекс-а-гон",
        Genre::Survival,
        "Плитки исчезают у вас из-под ног, а этажей всего три. Чем дольше продержитесь, тем больше очков!",
        "Продержитесь дольше всех",
        110.0,
    )
};

const SIZE: f64 = 1.5;
const RINGS: i64 = 8;
const THICK: f64 = 0.5;
const FLOORS: [f64; 3] = [0.0, -10.0, -20.0];
const FALL_DELAY: f64 = 0.42;
/// Floor colours, top to bottom (repainted by the round's look).
const FLOOR_PALS: [Palette; 3] = [pal::PINK, pal::BLUE, pal::YELLOW];

struct Tiles {
    /// In build order (ascending).
    cols: Vec<ColId>,
    /// (floor, q, r) → tile.
    index: BTreeMap<(usize, i64, i64), usize>,
}

impl Tiles {
    /// The tile under (x, z) on a floor, if any (axial rounding).
    fn at(&self, x: f64, z: f64, floor: usize) -> Option<usize> {
        let sq3 = m::sqrt(3.0);
        let qf = ((sq3 / 3.0) * x - z / 3.0) / SIZE;
        let rf = ((2.0 / 3.0) * z) / SIZE;
        let sf = -qf - rf;
        let mut q = qf.round();
        let mut r = rf.round();
        let s = sf.round();
        let dq = (q - qf).abs();
        let dr = (r - rf).abs();
        let ds = (s - sf).abs();
        if dq > dr && dq > ds {
            q = -r - s;
        } else if dr > ds {
            r = -q - s;
        }
        self.index.get(&(floor, q as i64, r as i64)).copied()
    }
}

fn intact(falls: &[Option<f64>], tile: Option<usize>, t: f64) -> bool {
    tile.is_some_and(|i| falls[i].is_none_or(|at| at > t + 0.2))
}

fn floor_of(y: f64) -> usize {
    let mut best = 0;
    for (i, fy) in FLOORS.iter().enumerate() {
        if (y - fy).abs() < (y - FLOORS[best]).abs() {
            best = i;
        }
    }
    best
}

/// The floors' tiles and when they drop.
struct Floors {
    tiles: Tiles,
    /// When each tile drops (None: still there), by tile index.
    falls: Vec<Option<f64>>,
    heading: Note<f64>,
    /// Client: the tiles' special, and where each tile is (floor, x, y, z).
    look: Option<(Hook, Vec<(u8, f64, f64, f64)>)>,
}

impl MapLogic for Floors {
    fn event(&mut self, _: &mut Cx, ev: &MapEvent) {
        let &MapEvent::Tile { i, at } = ev else { return };
        if let Some(f) = self.falls.get_mut(i as usize) {
            *f = Some(at);
        }
    }

    fn touch(&mut self, cx: &mut Cx, _: &mut Body, _: &mut StepEvents, tc: Touch) {
        let Ok(i) = self.tiles.cols.binary_search(&tc.col) else {
            return;
        };
        // Nothing falls before the start.
        if self.falls[i].is_some() || cx.t < 0.0 {
            return;
        }
        // (A client waits for the server's event, well within FALL_DELAY: a tile only its prediction stood on
        // would drop for it alone, for good.)
        if cx.server {
            let at = cx.t + FALL_DELAY;
            self.emit(cx, MapEvent::Tile { i: i as u32, at });
        }
    }

    fn pose(&self, t: f64, ctx: &mut MoveCtx) {
        for (i, &c) in self.tiles.cols.iter().enumerate() {
            let on = self.falls[i].is_none_or(|at| t < at);
            ctx.set_enabled(c, on);
        }
    }

    fn look(&self, hook: Hook, _: &World, t: f64, out: &mut LookOut) {
        let Some((_, spots)) = self.look.as_ref().filter(|l| l.0 == hook) else {
            return;
        };
        for (i, &(floor, x, y, z)) in spots.iter().enumerate() {
            let mut y = y - THICK / 2.0;
            let mut p = Piece::at(floor, x, y, z);
            if let Some(at) = self.falls[i] {
                let left = at - t;
                if left > 0.0 {
                    p.tone = 1.0 - left / FALL_DELAY;
                    y -= p.tone * 0.08;
                } else {
                    y -= 14.0 * left * left;
                    p.tone = -0.3;
                    if -left > 1.2 {
                        p.scale = 0.0;
                    }
                }
            }
            p.pos.y = y;
            out.pieces.push(p);
        }
    }

    fn bots(&self) -> bool {
        true
    }

    fn bot(&self, bot: &mut BotView, out: &mut BotInput) {
        init_bot(bot);
        let tiles = &self.tiles;
        let p = bot.body.pos;
        let floor = floor_of(p.y);
        let speed = bot.mem.traits().spd;
        let fl = &self.falls;
        // Keep moving (tiles drop half a second after being touched) towards intact ground, preferring
        // directions with more intact tiles ahead and staying away from the rim.
        let heading = match bot.mem.get(self.heading) {
            Some(h) => h,
            None => bot.rng.unit() * m::PI * 2.0,
        };
        let t = bot.t;
        let mut best = heading;
        let mut best_score = -1e9;
        for k in 0..12 {
            let a = heading + (k as f64 / 12.0) * m::PI * 2.0;
            let mut score = 0.0;
            let mut d = 1.4;
            while d <= 5.6 {
                let tile = tiles.at(p.x + m::sin(a) * d, p.z + m::cos(a) * d, floor);
                if !intact(fl, tile, t + d / 8.0) {
                    score -= if d < 2.0 { 12.0 } else { 6.0 / d };
                    break;
                }
                score += 1.0;
                d += 1.4;
            }
            score -= m::atan2(m::sin(a - heading), m::cos(a - heading)).abs() * 0.6;
            let ex = p.x + m::sin(a) * 4.0;
            let ez = p.z + m::cos(a) * 4.0;
            score -= 0f64.at_least(m::hypot(ex, ez) - SIZE * 1.5 * (RINGS - 1) as f64) * 2.0;
            score += (bot.rng.unit() - 0.5) * 0.4;
            if score > best_score {
                best_score = score;
                best = a;
            }
        }
        bot.mem.set(self.heading, best);
        out.mx = m::sin(best) * speed * 0.7;
        out.mz = m::cos(best) * speed * 0.7;
        // Hop along: tiles only drop where we land.
        if bot.body.grounded && t > 0.0 && bot.rng.unit() < 0.8 {
            out.jump = true;
        }
        // A gap right ahead: jump it (if there is ground beyond).
        let ahead = tiles.at(p.x + m::sin(best) * 1.5, p.z + m::cos(best) * 1.5, floor);
        let beyond = tiles.at(p.x + m::sin(best) * 3.2, p.z + m::cos(best) * 3.2, floor);
        if bot.body.grounded && !intact(fl, ahead, t + 0.1) && intact(fl, beyond, t + 0.4) {
            out.jump = true;
        }
        // Rescue dives judged by the tiles still there (the navigation grid still has the dropped ones).
        let ok = |x: f64, z: f64, at: f64| intact(fl, tiles.at(x, z, floor), at);
        let land = LandCheck {
            y: FLOORS[floor],
            ok: &ok,
        };
        let opts = HumanOpts {
            precise: true,
            land: Some(&land),
            ..Default::default()
        };
        humanize(bot, out, &opts);
        unstick(bot, out);
    }
}

impl MapDef for HexAGone {
    fn meta(&self) -> &'static GameMeta {
        &META
    }

    fn looks(&self) -> &'static [LookId] {
        &[LookId::Lava, LookId::Neon, LookId::Starlight]
    }

    fn build(&self, b: &mut Builder, _ctx: &MapCtx) -> MapSpec {
        let heading: Note<f64> = b.note();
        let sq3 = m::sqrt(3.0);
        let mut spots: Vec<(u8, f64, f64, f64)> = Vec::new();
        let mut tiles = Tiles {
            cols: Vec::new(),
            index: BTreeMap::new(),
        };
        for (floor, &y) in FLOORS.iter().enumerate() {
            for q in -RINGS..=RINGS {
                for r in -RINGS..=RINGS {
                    if (q + r).abs() > RINGS {
                        continue;
                    }
                    let x = SIZE * sq3 * (q as f64 + r as f64 / 2.0);
                    let z = SIZE * 1.5 * r as f64;
                    let i = tiles.cols.len();
                    spots.push((floor as u8, x, y, z));
                    let at = b.anchor(x, y - THICK / 2.0, z, ROOT);
                    let col = b.collider(
                        at,
                        Shape::Cyl {
                            r: SIZE * 0.92,
                            hh: THICK / 2.0,
                        },
                        ColliderOpts {
                            is_static: true,
                            ..Default::default()
                        },
                    );
                    b.watch_ground(col);
                    tiles.cols.push(col);
                    tiles.index.insert((floor, q, r), i);
                }
            }
        }
        for &fy in &FLOORS[..2] {
            for k in 0..3 {
                let a = (k as f64 / 3.0) * m::PI * 2.0 + fy;
                b.bonus(m::cos(a) * 7.0, fy, m::sin(a) * 7.0);
            }
        }
        let look = (!b.server()).then(|| {
            let parts = FLOOR_PALS.map(|p| {
                Part::toned(
                    Form::Cyl([SIZE * 0.97, THICK, 6.0]),
                    p[0],
                    rgb(0xffffff),
                    Finish::Glossy,
                )
                .on(Surface::Tile)
                .painted(p)
            });
            (b.special_hook(ROOT, "hex-tiles", &parts), spots)
        });
        b.clouds_with(0.0, 0.0, 55.0, 30, -45.0, -26.0);

        let spawns = b.ring_spawns(8, 6.0, 0.1, m::PI / 8.0);
        let falls = vec![None; tiles.cols.len()];
        MapSpec {
            spawns,
            kill_y: -30.0,
            face_center: true,
            view: Some(V3::new(0.0, -6.0, 0.0)),
            logic: Box::new(Floors {
                tiles,
                falls,
                heading,
                look,
            }),
            ..Default::default()
        }
    }
}
