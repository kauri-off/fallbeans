use core::any::Any;
use core::marker::PhantomData;

use fb_shared::DT;
use fb_shared::hash::{Fingerprint, Fnv};

use crate::collider::{ColId, Collider, ColliderOpts, Shape};
use crate::nodes::{NodeId, Nodes};
use crate::physics::PORTAL_T;

/// State a map keeps besides geometry (tiles that fell, doors that broke): movers read it, map logic
/// and bot brains get at it through its handle.
pub type StateBox = Box<dyn Any + Send + Sync>;

/// Handle to a map state of type S in its world.
pub struct St<S>(usize, PhantomData<fn() -> S>);

impl<S> Clone for St<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for St<S> {}

/// How long (s) both ends of a portal stay shut after the traveller came out.
pub const PORTAL_CLOSED: f64 = 1.0;

/// The last trip through a pair of portals (the same on the server and on clients).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortalPair {
    /// Sim time somebody went in, and at which end (0 or 1).
    pub at: f64,
    pub from: u32,
    /// Both ends are shut until then.
    pub closed_until: f64,
    /// How long (s) the ends stay shut after a traveller came out.
    pub close_for: f64,
}

/// What a mover may change: transforms of nodes and whether colliders are solid. It reads the map's
/// state and its portals.
pub struct MoveCtx<'a> {
    pub nodes: &'a mut Nodes,
    colliders: Option<&'a mut [Collider]>,
    states: &'a [StateBox],
    portals: &'a [PortalPair],
}

impl<'a> MoveCtx<'a> {
    pub fn node(&mut self, id: NodeId) -> &mut crate::nodes::Node {
        self.nodes.get_mut(id)
    }

    pub fn set_enabled(&mut self, col: ColId, on: bool) {
        if let Some(c) = self.colliders.as_deref_mut() {
            c[col as usize].enabled = on;
        }
    }

    pub fn st<S: 'static>(&self, h: St<S>) -> &'a S {
        state(self.states, h)
    }

    pub fn portal(&self, pair: usize) -> &'a PortalPair {
        &self.portals[pair]
    }
}

fn state<S: 'static>(states: &[StateBox], h: St<S>) -> &S {
    states[h.0].downcast_ref().expect("map state of another type")
}

/// Pure function of sim time: positions the moving parts of a map. Runs on server and client.
pub type Mover = Box<dyn Fn(f64, &mut MoveCtx) + Send + Sync>;

/// Uniform x/z grid of static colliders: a dense block of cells over their extent.
#[derive(Default)]
pub struct ColliderGrid {
    cell: f64,
    /// Every collider inserted, in order, with its cells [x0, x1, z0, z1] (inclusive).
    placed: Vec<(ColId, [i64; 4])>,
    /// The block (from `build`): cells x0.. and z0.., nx × nz of them; cell (ix, iz) is number
    /// c = (ix − x0)·nz + (iz − z0), its colliders `items[start[c]..start[c + 1]]` in insertion order.
    x0: i64,
    z0: i64,
    nx: i64,
    nz: i64,
    start: Vec<u32>,
    items: Vec<GridItem>,
}

#[derive(Clone, Copy, Default)]
struct GridItem {
    col: ColId,
    /// The collider's first cell: its lowest x and z.
    x0: i64,
    z0: i64,
}

impl ColliderGrid {
    fn new(cell: f64) -> Self {
        Self {
            cell,
            ..Default::default()
        }
    }

    /// Cells [x0, x1, z0, z1] that a span covers. (`as` saturates: a huge or infinite position gives
    /// cells outside the block, never an overflow.)
    fn cells(&self, x0: f64, x1: f64, z0: f64, z1: f64) -> [i64; 4] {
        [
            (x0 / self.cell).floor() as i64,
            (x1 / self.cell).floor() as i64,
            (z0 / self.cell).floor() as i64,
            (z1 / self.cell).floor() as i64,
        ]
    }

    /// Adds a collider; `build` lays it out.
    fn insert(&mut self, col: &Collider) {
        let (ex, ez) = col.extent_xz();
        let c = col.center;
        let cells = self.cells(c.x - ex, c.x + ex, c.z - ez, c.z + ez);
        self.placed.push((col.index, cells));
    }

    /// Lays out every collider inserted so far.
    fn build(&mut self) {
        let Some(&(_, first)) = self.placed.first() else {
            return;
        };
        let [mut x0, mut x1, mut z0, mut z1] = first;
        for &(_, [a0, a1, b0, b1]) in &self.placed {
            x0 = x0.min(a0);
            x1 = x1.max(a1);
            z0 = z0.min(b0);
            z1 = z1.max(b1);
        }
        let (nx, nz) = (x1 - x0 + 1, z1 - z0 + 1);
        let n = usize::try_from(nx.checked_mul(nz).expect("collider grid too large")).expect("collider grid too large");
        let at = |ix: i64, iz: i64| ((ix - x0) * nz + (iz - z0)) as usize;
        let mut start = vec![0u32; n + 1];
        for &(_, [a0, a1, b0, b1]) in &self.placed {
            for ix in a0..=a1 {
                for iz in b0..=b1 {
                    start[at(ix, iz) + 1] += 1;
                }
            }
        }
        let mut sum = 0;
        for s in start.iter_mut() {
            sum += *s;
            *s = sum;
        }
        let mut fill = start.clone();
        let mut items = vec![GridItem::default(); start[n] as usize];
        for &(col, [a0, a1, b0, b1]) in &self.placed {
            for ix in a0..=a1 {
                for iz in b0..=b1 {
                    let c = at(ix, iz);
                    items[fill[c] as usize] = GridItem { col, x0: a0, z0: b0 };
                    fill[c] += 1;
                }
            }
        }
        (self.x0, self.z0, self.nx, self.nz) = (x0, z0, nx, nz);
        self.start = start;
        self.items = items;
    }

    /// Colliders in the cells within r of (x, z), each once, in the order a walk over those cells (by x, then
    /// by z; in a cell, in insertion order) first meets them.
    fn query(&self, x: f64, z: f64, r: f64, out: &mut Vec<ColId>) {
        if self.items.is_empty() {
            return;
        }
        let [qx0, qx1, qz0, qz1] = self.cells(x - r, x + r, z - r, z + r);
        let (x0, x1) = (qx0.max(self.x0), qx1.min(self.x0 + self.nx - 1));
        let (z0, z1) = (qz0.max(self.z0), qz1.min(self.z0 + self.nz - 1));
        for ix in x0..=x1 {
            let row = (ix - self.x0) * self.nz - self.z0;
            for iz in z0..=z1 {
                let c = (row + iz) as usize;
                for it in &self.items[self.start[c] as usize..self.start[c + 1] as usize] {
                    // The walk meets a collider over several cells first in the lowest x, then the lowest z,
                    // of its cells within the query: it is taken there and only there.
                    if ix == it.x0.max(x0) && iz == it.z0.max(z0) {
                        out.push(it.col);
                    }
                }
            }
        }
    }

    /// Cells that hold a collider.
    pub fn size(&self) -> usize {
        self.start.windows(2).filter(|w| w[1] > w[0]).count()
    }
}

/// The collision side of a built map. Its state is a pure function of time (plus map events).
pub struct World {
    pub nodes: Nodes,
    pub colliders: Vec<Collider>,
    pub dynamic: Vec<ColId>,
    pub movers: Vec<Mover>,
    pub grid: ColliderGrid,
    pub t: f64,
    pub states: Vec<StateBox>,
    /// Portal pairs in build order (their index is how the server names them to clients).
    pub portals: Vec<PortalPair>,
    /// Nodes whose matrices a tick must refresh (ancestors of moving colliders), in order.
    dyn_nodes: Vec<NodeId>,
    finalized: bool,
}

impl Default for World {
    fn default() -> Self {
        Self {
            nodes: Nodes::default(),
            colliders: Vec::new(),
            dynamic: Vec::new(),
            movers: Vec::new(),
            grid: ColliderGrid::new(4.0),
            t: f64::NEG_INFINITY,
            states: Vec::new(),
            portals: Vec::new(),
            dyn_nodes: Vec::new(),
            finalized: false,
        }
    }
}

impl World {
    pub fn add(&mut self, node: NodeId, shape: Shape, opts: ColliderOpts) -> ColId {
        assert!(!self.finalized, "world is finalized");
        let id = self.colliders.len() as ColId;
        self.colliders.push(Collider::new(id, node, shape, opts));
        id
    }

    pub fn add_state<S: Any + Send + Sync>(&mut self, s: S) -> St<S> {
        self.states.push(Box::new(s));
        St(self.states.len() - 1, PhantomData)
    }

    pub fn st<S: 'static>(&self, h: St<S>) -> &S {
        state(&self.states, h)
    }

    pub fn st_mut<S: 'static>(&mut self, h: St<S>) -> &mut S {
        self.states[h.0].downcast_mut().expect("map state of another type")
    }

    /// A trip through portal pair `pair`, in at end `from` at time t: both ends shut till it is over.
    pub fn portal_used(&mut self, pair: usize, from: u32, t: f64) {
        let Some(p) = self.portals.get_mut(pair) else { return };
        if t < p.at {
            return;
        }
        p.at = t;
        p.from = from;
        p.closed_until = t + PORTAL_T + p.close_for;
    }

    fn run_movers(&mut self, t: f64) {
        let mut ctx = MoveCtx {
            nodes: &mut self.nodes,
            colliders: Some(&mut self.colliders),
            states: &self.states,
            portals: &self.portals,
        };
        for mv in &self.movers {
            mv(t, &mut ctx);
        }
    }

    /// Call once after building: positions everything for time t and indexes static colliders.
    pub fn finalize(&mut self, t: f64) {
        self.run_movers(t);
        self.nodes.update_all();
        let mut chain = vec![false; self.nodes.0.len()];
        for i in 0..self.colliders.len() {
            self.colliders[i].sync(&self.nodes);
            let c = &self.colliders[i];
            if c.opts.is_static {
                // (A non-finite extent would make the grid loop over cells for ever.)
                let (ex, ez) = c.extent_xz();
                assert!(
                    c.center.is_finite() && ex.is_finite() && ez.is_finite(),
                    "collider {i} is not finite"
                );
                self.grid.insert(c);
            } else {
                self.dynamic.push(c.index);
                let mut n = Some(c.node);
                while let Some(id) = n {
                    chain[id as usize] = true;
                    n = self.nodes.get(id).parent;
                }
            }
        }
        self.grid.build();
        self.dyn_nodes = (0..chain.len() as NodeId).filter(|&i| chain[i as usize]).collect();
        self.t = t;
        self.finalized = true;
    }

    /// Moves the world to time t; the previous matrices become the ones of the last call.
    pub fn set_time(&mut self, t: f64) {
        self.run_movers(t);
        for &i in &self.dyn_nodes {
            self.nodes.update_one(i);
        }
        for &i in &self.dynamic {
            self.colliders[i as usize].sync(&self.nodes);
        }
        self.t = t;
    }

    /// Moves to tick k (time k·DT) as if ticking there: the previous matrices are those of tick k − 1,
    /// whatever the world showed before (client prediction replays ticks out of order). Both times come
    /// from the tick (k·DT − DT and (k − 1)·DT may differ in the last bit), so a replay is the server's tick.
    pub fn goto_tick(&mut self, k: i64) {
        let t = k as f64 * DT;
        if t == self.t {
            return;
        }
        let prev = (k - 1) as f64 * DT;
        if prev != self.t {
            self.set_time(prev);
        }
        self.set_time(t);
    }

    /// Moves to time t as if ticking there: the previous matrices are those of t − DT, whatever the
    /// world showed before. For times between ticks; at a tick, `goto_tick`.
    pub fn goto(&mut self, t: f64) {
        if t == self.t {
            return;
        }
        if (t - DT - self.t).abs() > 1e-9 {
            self.set_time(t - DT);
        }
        self.set_time(t);
    }

    /// Poses `nodes` (a copy of this world's) for drawing at time t; colliders are left alone.
    pub fn pose(&self, t: f64, nodes: &mut Nodes) {
        self.pose_locals(t, nodes);
        nodes.update_all();
    }

    /// `pose` without the world matrices: the caller refreshes those of the nodes that moved.
    pub fn pose_locals(&self, t: f64, nodes: &mut Nodes) {
        let mut ctx = MoveCtx {
            nodes,
            colliders: None,
            states: &self.states,
            portals: &self.portals,
        };
        for mv in &self.movers {
            mv(t, &mut ctx);
        }
    }

    /// Fingerprint of the collision geometry at the current time.
    pub fn hash(&self, static_only: bool) -> Fingerprint {
        let mut h = Fnv::default();
        let mut mix = |v: f64| h.mix(v, 1e3);
        let mut n = 0;
        for c in &self.colliders {
            if static_only && !c.opts.is_static {
                continue;
            }
            n += 1;
            for e in c.cur.to_cols_array() {
                mix(e);
            }
            match c.shape {
                Shape::Box { hx, hy, hz } => mix(hx + hy * 3.0 + hz * 7.0),
                Shape::Cyl { r, hh } => mix(r + hh * 3.0),
                Shape::Sphere { r } => mix(r),
            }
            mix(if c.enabled { 1.0 } else { 0.0 });
            mix(c.opts.hit + c.opts.bounce * 3.0 + c.opts.pad * 7.0 + c.opts.slip * 11.0);
        }
        Fingerprint { n, h: h.finish() }
    }

    /// Colliders near (x, z): static ones from the grid, then moving ones within reach.
    pub fn query(&self, x: f64, z: f64, r: f64, out: &mut Vec<ColId>) {
        out.clear();
        self.grid.query(x, z, r, out);
        // (Moving colliders are never in the grid, and each is listed once.)
        for &i in &self.dynamic {
            let c = &self.colliders[i as usize];
            let dx = c.center.x - x;
            let dz = c.center.z - z;
            let reach = c.radius + r;
            if dx * dx + dz * dz <= reach * reach {
                out.push(i);
            }
        }
    }

    #[inline]
    pub fn col(&self, i: ColId) -> &Collider {
        &self.colliders[i as usize]
    }
}
