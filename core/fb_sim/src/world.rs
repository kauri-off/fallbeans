use core::any::Any;
use core::marker::PhantomData;
use std::collections::BTreeMap;

use crate::collider::{ColId, Collider, Shape};
use crate::m;
use crate::nodes::{NodeId, Nodes};
use crate::physics::PORTAL_T;
use fb_shared::DT;

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

impl MoveCtx<'_> {
    pub fn node(&mut self, id: NodeId) -> &mut crate::nodes::Node {
        self.nodes.get_mut(id)
    }

    pub fn set_enabled(&mut self, col: ColId, on: bool) {
        if let Some(c) = self.colliders.as_deref_mut() {
            c[col as usize].enabled = on;
        }
    }

    pub fn st<S: 'static>(&self, h: St<S>) -> &S {
        state(self.states, h)
    }

    pub fn portal(&self, pair: usize) -> &PortalPair {
        &self.portals[pair]
    }
}

fn state<S: 'static>(states: &[StateBox], h: St<S>) -> &S {
    states[h.0].downcast_ref().expect("map state of another type")
}

/// Pure function of sim time: positions the moving parts of a map. Runs on server and client.
pub type Mover = Box<dyn Fn(f64, &mut MoveCtx) + Send + Sync>;

/// Uniform x/z grid of static colliders.
#[derive(Default)]
pub struct ColliderGrid {
    cell: f64,
    cells: BTreeMap<i64, Vec<ColId>>,
}

impl ColliderGrid {
    fn key(ix: i64, iz: i64) -> i64 {
        (ix + 32768) * 65536 + (iz + 32768)
    }

    fn insert(&mut self, col: &Collider) {
        let (ex, ez) = col.extent_xz();
        let c = col.center;
        let x0 = ((c.x - ex) / self.cell).floor() as i64;
        let x1 = ((c.x + ex) / self.cell).floor() as i64;
        let z0 = ((c.z - ez) / self.cell).floor() as i64;
        let z1 = ((c.z + ez) / self.cell).floor() as i64;
        for ix in x0..=x1 {
            for iz in z0..=z1 {
                self.cells.entry(Self::key(ix, iz)).or_default().push(col.index);
            }
        }
    }

    fn query(&self, x: f64, z: f64, r: f64, out: &mut Vec<ColId>) {
        let x0 = ((x - r) / self.cell).floor() as i64;
        let x1 = ((x + r) / self.cell).floor() as i64;
        let z0 = ((z - r) / self.cell).floor() as i64;
        let z1 = ((z + r) / self.cell).floor() as i64;
        for ix in x0..=x1 {
            for iz in z0..=z1 {
                if let Some(list) = self.cells.get(&Self::key(ix, iz)) {
                    for &c in list {
                        if !out.contains(&c) {
                            out.push(c);
                        }
                    }
                }
            }
        }
    }

    pub fn size(&self) -> usize {
        self.cells.len()
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
            grid: ColliderGrid {
                cell: 4.0,
                cells: BTreeMap::new(),
            },
            t: f64::NEG_INFINITY,
            states: Vec::new(),
            portals: Vec::new(),
            dyn_nodes: Vec::new(),
            finalized: false,
        }
    }
}

impl World {
    pub fn add(&mut self, mut col: Collider) -> ColId {
        assert!(!self.finalized, "world is finalized");
        let id = self.colliders.len() as ColId;
        col.index = id;
        self.colliders.push(col);
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
            if c.is_static {
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

    /// Moves to time t as if ticking there: the previous matrices are those of t − DT, whatever the
    /// world showed before (client prediction replays ticks out of order).
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
        let mut ctx = MoveCtx {
            nodes,
            colliders: None,
            states: &self.states,
            portals: &self.portals,
        };
        for mv in &self.movers {
            mv(t, &mut ctx);
        }
        nodes.update_all();
    }

    /// Fingerprint of the collision geometry at the current time (`World.hash` in TS).
    pub fn hash(&self, static_only: bool) -> String {
        let mut h: u32 = 2_166_136_261;
        let mut mix = |v: f64| {
            let x = m::to_i32(m::round_js(v * 1e3));
            h = (h ^ x as u32).wrapping_mul(16_777_619);
        };
        let mut n = 0;
        for c in &self.colliders {
            if static_only && !c.is_static {
                continue;
            }
            n += 1;
            for &e in &c.cur.0 {
                mix(e);
            }
            match c.shape {
                Shape::Box { hx, hy, hz } => mix(hx + hy * 3.0 + hz * 7.0),
                Shape::Cyl { r, hh } => mix(r + hh * 3.0),
                Shape::Sphere { r } => mix(r),
            }
            mix(if c.enabled { 1.0 } else { 0.0 });
            mix(c.hit + c.bounce * 3.0 + c.pad * 7.0 + c.slip * 11.0);
        }
        format!("{n}:{h:08x}")
    }

    /// Colliders near (x, z): static ones from the grid, then moving ones within reach.
    pub fn query(&self, x: f64, z: f64, r: f64, out: &mut Vec<ColId>) {
        out.clear();
        self.grid.query(x, z, r, out);
        for &i in &self.dynamic {
            let c = &self.colliders[i as usize];
            let dx = c.center.x - x;
            let dz = c.center.z - z;
            let reach = c.radius + r;
            if dx * dx + dz * dz <= reach * reach && !out.contains(&i) {
                out.push(i);
            }
        }
    }

    #[inline]
    pub fn col(&self, i: ColId) -> &Collider {
        &self.colliders[i as usize]
    }
}
