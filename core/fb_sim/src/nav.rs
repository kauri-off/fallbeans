//! Navigation for bots: a grid over the static part of a map with up to a few
//! walkable layers per cell, linked by walking, stepping, jumping up, dropping down and jumping gaps.
//! A* finds routes on it; moving parts are left to the map's bot logic.
use std::sync::Mutex;

use crate::collider::{ColId, Collider, Shape};
use crate::m::{self, MinMax};
use crate::math::V3;
use crate::world::World;

pub const NAV_CELL: f64 = 0.5;
const LAYERS: usize = 4;
const CLEAR: f64 = 1.7;
const STEP: f64 = 0.55;
const JUMP_UP: f64 = 1.85;
const DROP: f64 = 6.0;
/// Longest gap a running jump clears (m).
const GAP: f64 = 3.2;
const DOWN: V3 = V3::new(0.0, -1.0, 0.0);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Jump on the way to this point (a climb or a gap).
    pub jump: bool,
}

#[derive(Default)]
pub struct PathOpts<'a> {
    /// Extra cost of standing at (x, z) (e.g. other beans in the way).
    pub cost: Option<&'a dyn Fn(f64, f64) -> f64>,
    /// Goal radius (m), default 0.8.
    pub radius: Option<f64>,
    pub max_nodes: Option<usize>,
}

#[derive(Clone, Copy)]
struct Span {
    lo: f64,
    hi: f64,
    ny: f64,
    col: ColId,
}

/// Where a downward ray enters and leaves a collider (world y), with the entry normal's y.
fn ray_down(c: &Collider, x: f64, z: f64, top: f64) -> Option<Span> {
    let o = c.inv.transform_point3(V3::new(x, top, z));
    let d = c.inv.transform_vector3(DOWN).normalize_or_zero();
    let mut t0 = f64::NEG_INFINITY;
    let mut t1 = f64::INFINITY;
    let mut n;
    match c.shape {
        Shape::Box { hx, hy, hz } => {
            let half = [hx, hy, hz];
            let oo = [o.x, o.y, o.z];
            let dd = [d.x, d.y, d.z];
            let mut axis = -1i32;
            let mut sign = 0.0;
            for i in 0..3 {
                let (di, oi, h) = (dd[i], oo[i], half[i]);
                if di.abs() < 1e-9 {
                    if oi < -h || oi > h {
                        return None;
                    }
                    continue;
                }
                let mut a = (-h - oi) / di;
                let mut b = (h - oi) / di;
                let mut sg = -1.0;
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                    sg = 1.0;
                }
                if a > t0 {
                    t0 = a;
                    axis = i as i32;
                    sign = sg;
                }
                t1 = t1.at_most(b);
                if t0 > t1 {
                    return None;
                }
            }
            n = match axis {
                0 => V3::new(sign, 0.0, 0.0),
                1 => V3::new(0.0, sign, 0.0),
                2 => V3::new(0.0, 0.0, sign),
                _ => return None,
            };
        }
        Shape::Cyl { r, hh } => {
            let mut cap_a = f64::NEG_INFINITY;
            let mut cap_b = f64::INFINITY;
            if d.y.abs() < 1e-9 {
                if o.y.abs() > hh {
                    return None;
                }
            } else {
                cap_a = (-hh - o.y) / d.y;
                cap_b = (hh - o.y) / d.y;
                if cap_a > cap_b {
                    std::mem::swap(&mut cap_a, &mut cap_b);
                }
            }
            let a = d.x * d.x + d.z * d.z;
            let b = 2.0 * (o.x * d.x + o.z * d.z);
            let cc = o.x * o.x + o.z * o.z - r * r;
            let mut side_a = f64::NEG_INFINITY;
            let mut side_b = f64::INFINITY;
            if a < 1e-9 {
                if cc > 0.0 {
                    return None;
                }
            } else {
                let disc = b * b - 4.0 * a * cc;
                if disc < 0.0 {
                    return None;
                }
                let q = m::sqrt(disc);
                side_a = (-b - q) / (2.0 * a);
                side_b = (-b + q) / (2.0 * a);
            }
            t0 = cap_a.at_least(side_a);
            t1 = cap_b.at_most(side_b);
            if t0 > t1 {
                return None;
            }
            n = if cap_a >= side_a {
                let s = -m::sign(d.y);
                V3::new(0.0, if s == 0.0 || s.is_nan() { 1.0 } else { s }, 0.0)
            } else {
                V3::new(o.x + d.x * t0, 0.0, o.z + d.z * t0).normalize_or_zero()
            };
        }
        Shape::Sphere { r } => {
            let b = o.dot(d);
            let cc = o.length_squared() - r * r;
            let disc = b * b - cc;
            if disc < 0.0 {
                return None;
            }
            let q = m::sqrt(disc);
            t0 = -b - q;
            t1 = -b + q;
            n = V3::new(d.x * t0 + o.x, d.y * t0 + o.y, d.z * t0 + o.z).normalize_or_zero();
        }
    }
    if t1 < 0.0 {
        return None;
    }
    n = c.cur.transform_vector3(n).normalize_or_zero();
    Some(Span {
        hi: top - t0.at_least(0.0),
        lo: top - t1,
        ny: n.y,
        col: c.index,
    })
}

#[derive(Default)]
struct Heap {
    ids: Vec<usize>,
    keys: Vec<f64>,
}

impl Heap {
    fn clear(&mut self) {
        self.ids.clear();
        self.keys.clear();
    }

    fn push(&mut self, id: usize, key: f64) {
        let mut i = self.ids.len();
        self.ids.push(id);
        self.keys.push(key);
        while i > 0 {
            let p = (i - 1) >> 1;
            if self.keys[p] <= key {
                break;
            }
            self.ids[i] = self.ids[p];
            self.keys[i] = self.keys[p];
            i = p;
        }
        self.ids[i] = id;
        self.keys[i] = key;
    }

    fn pop(&mut self) -> usize {
        let top = self.ids[0];
        let last_id = self.ids.pop().unwrap();
        let last_key = self.keys.pop().unwrap();
        let n = self.ids.len();
        if n > 0 {
            let mut i = 0;
            loop {
                let l = i * 2 + 1;
                if l >= n {
                    break;
                }
                let r = l + 1;
                let c = if r < n && self.keys[r] < self.keys[l] { r } else { l };
                if self.keys[c] >= last_key {
                    break;
                }
                self.ids[i] = self.ids[c];
                self.keys[i] = self.keys[c];
                i = c;
            }
            self.ids[i] = last_id;
            self.keys[i] = last_key;
        }
        top
    }
}

const DIRS: [(i64, i64); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

/// A* scratch, reused between searches.
#[derive(Default)]
struct Search {
    g: Vec<f64>,
    from: Vec<i64>,
    how: Vec<u8>,
    seen: Vec<u32>,
    generation: u32,
    heap: Heap,
}

pub struct NavGrid {
    pub x0: f64,
    pub z0: f64,
    pub nx: usize,
    pub nz: usize,
    /// Surface height per node (cell · LAYERS + layer), NaN where there is none.
    ys: Vec<f64>,
    /// Collider carrying the node (it may disappear: hex tiles, fake bridge tiles).
    cols: Vec<i64>,
    /// Extra cost of a node: walls and edges close by.
    pen: Vec<f64>,
    search: Mutex<Search>,
}

/// The grid together with the world whose colliders it checks (they may be switched off).
#[derive(Clone, Copy)]
pub struct Nav<'a> {
    pub grid: &'a NavGrid,
    pub world: &'a World,
}

impl NavGrid {
    fn new(x0: f64, z0: f64, nx: usize, nz: usize) -> Self {
        let n = nx * nz * LAYERS;
        Self {
            x0,
            z0,
            nx,
            nz,
            ys: vec![f64::NAN; n],
            cols: vec![-1; n],
            pen: vec![0.0; n],
            search: Mutex::new(Search {
                g: vec![0.0; n],
                from: vec![0; n],
                how: vec![0; n],
                seen: vec![0; n],
                generation: 0,
                heap: Heap::default(),
            }),
        }
    }

    /// Builds the grid from the world's static, solid colliders as they are now.
    pub fn build(world: &World, forbidden: Option<&dyn Fn(V3) -> bool>) -> NavGrid {
        let solid = |c: &Collider| c.is_static && c.enabled && !c.nav_skip && !c.trigger;
        let mut x0 = f64::INFINITY;
        let mut x1 = f64::NEG_INFINITY;
        let mut z0 = f64::INFINITY;
        let mut z1 = f64::NEG_INFINITY;
        let mut top = f64::NEG_INFINITY;
        let mut any = false;
        for c in world.colliders.iter().filter(|c| solid(c)) {
            any = true;
            let (ex, ez) = c.extent_xz();
            x0 = x0.at_most(c.center.x - ex);
            x1 = x1.at_least(c.center.x + ex);
            z0 = z0.at_most(c.center.z - ez);
            z1 = z1.at_least(c.center.z + ez);
            top = top.at_least(c.center.y + c.radius);
        }
        if !any {
            x0 = -1.0;
            z0 = -1.0;
            x1 = 1.0;
            z1 = 1.0;
            top = 1.0;
        }
        let nx = (((x1 - x0) / NAV_CELL).ceil() + 2.0).at_least(1.0) as usize;
        let nz = (((z1 - z0) / NAV_CELL).ceil() + 2.0).at_least(1.0) as usize;
        let mut nav = NavGrid::new(x0 - NAV_CELL, z0 - NAV_CELL, nx, nz);
        let mut spans: Vec<Vec<Span>> = Vec::with_capacity(nx * nz);
        let mut near = Vec::new();
        for iz in 0..nz {
            for ix in 0..nx {
                let x = nav.cx(ix);
                let z = nav.cz(iz);
                let mut list: Vec<Span> = Vec::new();
                world.query(x, z, NAV_CELL * 0.5, &mut near);
                for &ci in &near {
                    let c = world.col(ci);
                    if !solid(c) {
                        continue;
                    }
                    if let Some(sp) = ray_down(c, x, z, top + 5.0) {
                        list.push(sp);
                    }
                }
                // (`hi` is never NaN for a finite top; were it, it would compare equal.)
                list.sort_by(|a, b| b.hi.partial_cmp(&a.hi).unwrap_or(core::cmp::Ordering::Equal));
                let cell = iz * nx + ix;
                let mut layer = 0;
                for (si, s) in list.iter().enumerate() {
                    if layer >= LAYERS {
                        break;
                    }
                    // Too steep, bouncy or hazardous to stand on.
                    let col = world.col(s.col);
                    if s.ny < 0.6 || col.bounce != 0.0 || col.hit != 0.0 {
                        continue;
                    }
                    let y = s.hi;
                    // Buried inside, or no head room under something else.
                    if list
                        .iter()
                        .enumerate()
                        .any(|(oi, o)| oi != si && o.lo < y + CLEAR && o.hi > y + 0.05)
                    {
                        continue;
                    }
                    let prev = if layer > 0 {
                        nav.ys[cell * LAYERS + layer - 1]
                    } else {
                        f64::NAN
                    };
                    if !prev.is_nan() && (prev - y).abs() < 0.3 {
                        continue;
                    }
                    if forbidden.is_some_and(|f| f(V3::new(x, y + 0.05, z))) {
                        continue;
                    }
                    nav.ys[cell * LAYERS + layer] = y;
                    nav.cols[cell * LAYERS + layer] = s.col as i64;
                    layer += 1;
                }
                spans.push(list);
            }
        }
        // Penalties: walls within a bean's radius, and edges (a drop right next to it).
        for iz in 0..nz {
            for ix in 0..nx {
                let cell = iz * nx + ix;
                for l in 0..LAYERS {
                    let id = cell * LAYERS + l;
                    let y = nav.ys[id];
                    if y.is_nan() {
                        break;
                    }
                    let mut wall = 0;
                    let mut edge = 0;
                    for (dx, dz) in DIRS {
                        let jx = ix as i64 + dx;
                        let jz = iz as i64 + dz;
                        if jx < 0 || jz < 0 || jx >= nx as i64 || jz >= nz as i64 {
                            edge += 1;
                            continue;
                        }
                        let other = jz as usize * nx + jx as usize;
                        if spans[other].iter().any(|s| s.lo < y + 1.4 && s.hi > y + 0.3) {
                            wall += 1;
                        }
                        if nav.layer_near(world, other as i64, y, STEP) < 0 {
                            edge += 1;
                        }
                    }
                    let p =
                        (if wall > 0 { 1.5 } else { 0.0 }) + (if edge > 0 { 1.0 + edge as f64 * 0.25 } else { 0.0 });
                    nav.pen[id] = p;
                }
            }
        }
        // Second ring of edge caution: one cell further in.
        let pen2 = nav.pen.clone();
        for iz in 1..nz.saturating_sub(1) {
            for ix in 1..nx.saturating_sub(1) {
                for l in 0..LAYERS {
                    let id = (iz * nx + ix) * LAYERS + l;
                    let y = nav.ys[id];
                    if y.is_nan() || pen2[id] > 0.0 {
                        continue;
                    }
                    for (dx, dz) in DIRS {
                        let cell = (iz as i64 + dz) * nx as i64 + ix as i64 + dx;
                        let o = nav.layer_near(world, cell, y, STEP);
                        if o >= 0 && pen2[o as usize] >= 1.0 {
                            nav.pen[id] = 0.4;
                            break;
                        }
                    }
                }
            }
        }
        nav
    }

    pub fn cx(&self, ix: usize) -> f64 {
        self.x0 + (ix as f64 + 0.5) * NAV_CELL
    }

    pub fn cz(&self, iz: usize) -> f64 {
        self.z0 + (iz as f64 + 0.5) * NAV_CELL
    }

    fn cell_of(&self, x: f64, z: f64) -> i64 {
        let ix = ((x - self.x0) / NAV_CELL).floor();
        let iz = ((z - self.z0) / NAV_CELL).floor();
        // (Written so that NaN is off the grid too.)
        if !(ix >= 0.0 && iz >= 0.0 && ix < self.nx as f64 && iz < self.nz as f64) {
            return -1;
        }
        iz as i64 * self.nx as i64 + ix as i64
    }

    /// Node of `cell` whose surface is within `tol` of y (the closest), or −1.
    fn layer_near(&self, world: &World, cell: i64, y: f64, tol: f64) -> i64 {
        if cell < 0 || cell >= (self.nx * self.nz) as i64 {
            return -1;
        }
        let mut best = -1;
        let mut bd = tol;
        for l in 0..LAYERS {
            let id = cell as usize * LAYERS + l;
            let ly = self.ys[id];
            if ly.is_nan() {
                break;
            }
            let d = (ly - y).abs();
            if d <= bd && self.alive(world, id) {
                bd = d;
                best = id as i64;
            }
        }
        best
    }

    fn alive(&self, world: &World, id: usize) -> bool {
        let c = self.cols[id];
        c >= 0 && world.colliders.get(c as usize).is_some_and(|c| c.enabled)
    }

    /// Highest node of a cell at or a little above y (what a body at y stands on).
    fn layer_below(&self, world: &World, cell: i64, y: f64) -> i64 {
        if cell < 0 {
            return -1;
        }
        for l in 0..LAYERS {
            let id = cell as usize * LAYERS + l;
            let ly = self.ys[id];
            if ly.is_nan() {
                break;
            }
            if ly <= y + 0.6 && self.alive(world, id) {
                return id as i64;
            }
        }
        -1
    }

    fn point(&self, id: usize, jump: bool) -> NavPoint {
        let cell = id / LAYERS;
        NavPoint {
            x: self.cx(cell % self.nx),
            y: self.ys[id],
            z: self.cz(cell / self.nx),
            jump,
        }
    }
}

impl<'a> Nav<'a> {
    pub fn new(grid: &'a NavGrid, world: &'a World) -> Self {
        Self { grid, world }
    }

    /// Ground height under (x, z) within `tol` of height y, or None.
    pub fn ground_at(&self, x: f64, z: f64, y: f64, tol: f64) -> Option<f64> {
        let g = self.grid;
        let id = g.layer_near(self.world, g.cell_of(x, z), y, tol);
        (id >= 0).then(|| g.ys[id as usize])
    }

    /// Is (x, z, y) on walkable ground and not right at an edge?
    pub fn safe(&self, x: f64, z: f64, y: f64) -> bool {
        let g = self.grid;
        let id = g.layer_near(self.world, g.cell_of(x, z), y, 0.8);
        id >= 0 && g.pen[id as usize] < 1.0
    }

    /// The ground a body falling at (x, z) from height y lands on: its height and whether it is safe.
    pub fn floor_below(&self, x: f64, z: f64, y: f64) -> Option<(f64, bool)> {
        let g = self.grid;
        let id = g.layer_below(self.world, g.cell_of(x, z), y);
        (id >= 0).then(|| (g.ys[id as usize], g.pen[id as usize] < 1.0))
    }

    /// The node a body at p stands on (or the nearest one close by).
    fn node_at(&self, x: f64, y: f64, z: f64) -> i64 {
        let g = self.grid;
        let cell = g.cell_of(x, z);
        let mut id = g.layer_below(self.world, cell, y);
        if id >= 0 {
            return id;
        }
        // Standing on something that is not in the grid (a moving platform), or right at an edge.
        let ix = ((x - g.x0) / NAV_CELL).floor() as i64;
        let iz = ((z - g.z0) / NAV_CELL).floor() as i64;
        let mut best = -1;
        let mut bd = f64::INFINITY;
        let mut r: i64 = 1;
        while r <= 4 && best < 0 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let jx = ix + dx;
                    let jz = iz + dz;
                    if jx < 0 || jz < 0 || jx >= g.nx as i64 || jz >= g.nz as i64 {
                        continue;
                    }
                    id = g.layer_below(self.world, jz * g.nx as i64 + jx, y);
                    if id < 0 {
                        continue;
                    }
                    let dy = g.ys[id as usize] - y;
                    let d = (dx * dx + dz * dz) as f64 + dy * dy;
                    if d < bd {
                        bd = d;
                        best = id;
                    }
                }
            }
            r += 1;
        }
        best
    }

    /// A* from p to (tx, tz) (any layer; the one nearest ty when given). Returns a smoothed list of
    /// points to run through (the first one is where to head now), or None when there is no route.
    pub fn path(&self, p: V3, tx: f64, tz: f64, ty: Option<f64>, opts: &PathOpts) -> Option<Vec<NavPoint>> {
        let g = self.grid;
        let world = self.world;
        let start = self.node_at(p.x, p.y, p.z);
        if start < 0 {
            return None;
        }
        let start = start as usize;
        let radius = NAV_CELL.at_least(opts.radius.unwrap_or(0.8));
        let max_nodes = opts.max_nodes.unwrap_or(6000);
        let mut guard = g.search.lock().unwrap_or_else(|e| e.into_inner());
        let s = &mut *guard;
        s.generation = s.generation.wrapping_add(1);
        if s.generation == 0 {
            // Wrapped round: stamps of old searches would read as this one's.
            s.seen.fill(0);
            s.generation = 1;
        }
        let gen_now = s.generation;
        s.heap.clear();
        let h = |id: usize| {
            let cell = id / LAYERS;
            let x = g.cx(cell % g.nx);
            let z = g.cz(cell / g.nx);
            m::hypot(x - tx, z - tz)
        };
        let is_goal = |id: usize| {
            if h(id) > radius {
                return false;
            }
            ty.is_none_or(|ty| (g.ys[id] - ty).abs() < 1.5)
        };
        s.seen[start] = gen_now;
        s.g[start] = 0.0;
        s.from[start] = -1;
        s.how[start] = 0;
        s.heap.push(start, h(start));
        let mut goal: i64 = -1;
        let mut best = start;
        let mut best_h = h(start);
        let mut expanded = 0;
        let try_edge = |s: &mut Search, from: usize, to: i64, cost: f64, jump: bool| {
            if to < 0 {
                return;
            }
            let to = to as usize;
            let mut extra = 0.0;
            if let Some(c) = opts.cost {
                let cell = to / LAYERS;
                extra = c(g.cx(cell % g.nx), g.cz(cell / g.nx));
            }
            let gv = s.g[from] + cost + g.pen[to] * 0.6 + extra;
            if s.seen[to] == gen_now && gv >= s.g[to] {
                return;
            }
            s.seen[to] = gen_now;
            s.g[to] = gv;
            s.from[to] = from as i64;
            s.how[to] = u8::from(jump);
            s.heap.push(to, gv + h(to));
        };
        while !s.heap.ids.is_empty() && {
            let go = expanded < max_nodes;
            expanded += 1;
            go
        } {
            let id = s.heap.pop();
            if is_goal(id) {
                goal = id as i64;
                break;
            }
            let hid = h(id);
            if hid < best_h {
                best_h = hid;
                best = id;
            }
            let cell = id / LAYERS;
            let ix = (cell % g.nx) as i64;
            let iz = (cell / g.nx) as i64;
            let nx = g.nx as i64;
            let nz = g.nz as i64;
            let y = g.ys[id];
            let mut edge = false;
            for (dx, dz) in DIRS {
                let jx = ix + dx;
                let jz = iz + dz;
                if jx < 0 || jz < 0 || jx >= nx || jz >= nz {
                    continue;
                }
                let other = (jz * nx + jx) as usize;
                let diag = dx != 0 && dz != 0;
                let dist = if diag { NAV_CELL * m::SQRT2 } else { NAV_CELL };
                // No cutting corners past walls or holes.
                if diag
                    && (g.layer_near(world, iz * nx + jx, y, STEP) < 0
                        || g.layer_near(world, jz * nx + ix, y, STEP) < 0)
                {
                    continue;
                }
                let mut linked = false;
                for l in 0..LAYERS {
                    let to = other * LAYERS + l;
                    let ly = g.ys[to];
                    if ly.is_nan() {
                        break;
                    }
                    if !g.alive(world, to) {
                        continue;
                    }
                    let dh = ly - y;
                    if dh.abs() <= STEP {
                        try_edge(s, id, to as i64, dist, false);
                        linked = true;
                    } else if dh > STEP && dh <= JUMP_UP {
                        try_edge(s, id, to as i64, dist + 2.5, true);
                    } else if (-DROP..-STEP).contains(&dh) {
                        try_edge(s, id, to as i64, dist + 0.6 - dh * 0.35, false);
                    }
                }
                if !linked && !diag {
                    edge = true;
                }
            }
            // At an edge: jumps across gaps.
            if edge {
                for (dx, dz) in DIRS {
                    let step = if dx != 0 && dz != 0 { m::SQRT2 } else { 1.0 };
                    let mut k = 2;
                    while k as f64 * step * NAV_CELL <= GAP {
                        let jx = ix + dx * k;
                        let jz = iz + dz * k;
                        if jx < 0 || jz < 0 || jx >= nx || jz >= nz {
                            break;
                        }
                        let other = jz * nx + jx;
                        let land = g.layer_near(world, other, y - 0.6, 1.8);
                        if land >= 0 && g.ys[land as usize] <= y + 1.1 {
                            if k > 2 {
                                try_edge(s, id, land, k as f64 * step * NAV_CELL + 3.0, true);
                            }
                            break;
                        }
                        // Something solid in the way at jumping height: no gap jump this way.
                        if g.layer_near(world, other, y, JUMP_UP) >= 0 {
                            break;
                        }
                        k += 1;
                    }
                }
            }
        }
        let end = if goal >= 0 { goal as usize } else { best };
        if end == start && goal < 0 {
            return None;
        }
        let mut raw = Vec::new();
        let mut id = end as i64;
        while id >= 0 {
            raw.push(id as usize);
            id = s.from[id as usize];
        }
        raw.reverse();
        Some(self.smooth(&raw, &s.how))
    }

    /// String pulling: skip nodes while the straight line stays on similar, unbroken ground.
    fn smooth(&self, ids: &[usize], how: &[u8]) -> Vec<NavPoint> {
        let g = self.grid;
        let mut out = Vec::new();
        let mut i = 0;
        while i + 1 < ids.len() {
            let mut j = i + 1;
            while j + 1 < ids.len() && how[ids[j + 1]] == 0 && self.straight(ids[i], ids[j + 1]) {
                j += 1;
            }
            out.push(g.point(ids[j], how[ids[j]] == 1));
            i = j;
        }
        if out.is_empty() && !ids.is_empty() {
            out.push(g.point(ids[0], false));
        }
        out
    }

    fn straight(&self, a: usize, b: usize) -> bool {
        let g = self.grid;
        let pa = g.point(a, false);
        let pb = g.point(b, false);
        let len = m::hypot(pb.x - pa.x, pb.z - pa.z);
        let n = (len / (NAV_CELL * 0.5)).ceil() as i64;
        let mut y = pa.y;
        for k in 1..n {
            let f = k as f64 / n as f64;
            let cell = g.cell_of(pa.x + (pb.x - pa.x) * f, pa.z + (pb.z - pa.z) * f);
            let id = g.layer_near(self.world, cell, y, STEP);
            if id < 0 || g.pen[id as usize] >= 1.5 {
                return false;
            }
            y = g.ys[id as usize];
        }
        true
    }
}
