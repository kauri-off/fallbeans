use crate::m;
use crate::math::{M4, V3, add_scaled, div_s, dot, len, len_sq};
use crate::nodes::{NodeId, Nodes};

pub type ColId = u32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Box { hx: f64, hy: f64, hz: f64 },
    Cyl { r: f64, hh: f64 },
    Sphere { r: f64 },
}

#[derive(Clone, Debug, Default)]
pub struct ColliderOpts {
    pub is_static: bool,
    pub bounce: f64,
    pub hit: f64,
    pub pad: f64,
    pub conveyor: Option<V3>,
    /// Hazard name for knockout credit ("hammer", "rotor", …).
    pub tag: Option<&'static str>,
    /// Ice: 0 normal grip, 1 almost none.
    pub slip: f64,
    /// Touches are reported to the map (the TS `onTouch` callback).
    pub on_touch: bool,
    pub on_ground: bool,
    pub nav_skip: bool,
    pub sinks: bool,
    /// A sweeping arm: always knocks over, and passes over beans that are down.
    pub sweep: bool,
    pub trigger: bool,
    pub ladder: bool,
    pub launch: Option<V3>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Contact {
    pub local: V3,
    pub point: V3,
    pub normal: V3,
    pub depth: f64,
}

#[derive(Clone, Debug)]
pub struct Collider {
    pub node: NodeId,
    pub shape: Shape,
    pub enabled: bool,
    pub index: ColId,
    pub is_static: bool,
    pub bounce: f64,
    pub hit: f64,
    pub pad: f64,
    pub conveyor: Option<V3>,
    pub tag: Option<&'static str>,
    pub slip: f64,
    pub on_touch: bool,
    pub on_ground: bool,
    pub nav_skip: bool,
    pub sinks: bool,
    pub sweep: bool,
    pub trigger: bool,
    pub ladder: bool,
    pub launch: Option<V3>,
    pub cur: M4,
    pub prev: M4,
    pub inv: M4,
    pub center: V3,
    pub radius: f64,
    synced: bool,
}

impl Collider {
    pub fn new(node: NodeId, shape: Shape, o: ColliderOpts) -> Self {
        let radius = match shape {
            Shape::Box { hx, hy, hz } => m::hypot3(hx, hy, hz),
            Shape::Cyl { r, hh } => m::hypot(r, hh),
            Shape::Sphere { r } => r,
        };
        Self {
            node,
            shape,
            enabled: true,
            index: u32::MAX,
            is_static: o.is_static,
            bounce: o.bounce,
            hit: o.hit,
            pad: o.pad,
            conveyor: o.conveyor,
            tag: o.tag,
            slip: o.slip,
            on_touch: o.on_touch,
            on_ground: o.on_ground,
            nav_skip: o.nav_skip,
            sinks: o.sinks,
            sweep: o.sweep,
            trigger: o.trigger || o.ladder,
            ladder: o.ladder,
            launch: o.launch,
            cur: M4::IDENTITY,
            prev: M4::IDENTITY,
            inv: M4::IDENTITY,
            center: V3::ZERO,
            radius,
            synced: false,
        }
    }

    /// Reads the node's world matrix (its chain must be up to date); the previous one is kept.
    pub fn sync(&mut self, nodes: &Nodes) {
        if self.is_static && self.synced {
            self.prev = self.cur;
            return;
        }
        let w = nodes.get(self.node).world;
        self.prev = if self.synced { self.cur } else { w };
        self.cur = w;
        self.inv = w.invert();
        self.center = w.position();
        self.synced = true;
    }

    /// Half extents of the world-space AABB in x and z (for the static grid).
    pub fn extent_xz(&self) -> (f64, f64) {
        let e = &self.cur.0;
        let (hx, hy, hz) = match self.shape {
            Shape::Box { hx, hy, hz } => (hx, hy, hz),
            Shape::Cyl { r, hh } => (r, hh, r),
            Shape::Sphere { r } => (r, r, r),
        };
        (
            e[0].abs() * hx + e[4].abs() * hy + e[8].abs() * hz,
            e[2].abs() * hx + e[6].abs() * hy + e[10].abs() * hz,
        )
    }

    pub fn contact(&self, center: V3, r: f64, out: &mut Contact) -> bool {
        let reach = self.radius + r;
        if crate::math::dist_sq(center, self.center) > reach * reach {
            return false;
        }
        let l = self.inv.apply_point(center);
        let mut q;
        let n;
        let depth;
        match self.shape {
            Shape::Box { hx, hy, hz } => {
                q = V3::new(m::clamp(l.x, -hx, hx), m::clamp(l.y, -hy, hy), m::clamp(l.z, -hz, hz));
                let d0 = l - q;
                let d = len(d0);
                if d > 1e-6 {
                    if d >= r {
                        return false;
                    }
                    n = div_s(d0, d);
                    depth = r - d;
                } else {
                    let dx = hx - l.x.abs();
                    let dy = hy - l.y.abs();
                    let dz = hz - l.z.abs();
                    let mut nn = V3::ZERO;
                    if dy <= dx && dy <= dz {
                        nn.y = sign_or_one(l.y);
                        q.y = nn.y * hy;
                        depth = dy + r;
                    } else if dx <= dz {
                        nn.x = sign_or_one(l.x);
                        q.x = nn.x * hx;
                        depth = dx + r;
                    } else {
                        nn.z = sign_or_one(l.z);
                        q.z = nn.z * hz;
                        depth = dz + r;
                    }
                    n = nn;
                }
            }
            Shape::Cyl { r: cr, hh } => {
                let rl = m::hypot(l.x, l.z);
                let in_r = rl <= cr;
                let in_y = l.y.abs() <= hh;
                if in_r && in_y {
                    let side = cr - rl;
                    let top = hh - l.y;
                    let bot = hh + l.y;
                    q = l;
                    if top <= side && top <= bot {
                        n = V3::new(0.0, 1.0, 0.0);
                        q.y = hh;
                        depth = top + r;
                    } else if bot <= side {
                        n = V3::new(0.0, -1.0, 0.0);
                        q.y = -hh;
                        depth = bot + r;
                    } else {
                        n = if rl < 1e-6 { V3::new(1.0, 0.0, 0.0) } else { V3::new(l.x / rl, 0.0, l.z / rl) };
                        q = V3::new(n.x * cr, l.y, n.z * cr);
                        depth = side + r;
                    }
                } else {
                    let k = if in_r { 1.0 } else { cr / rl };
                    q = V3::new(l.x * k, m::clamp(l.y, -hh, hh), l.z * k);
                    let d0 = l - q;
                    let d = len(d0);
                    if d >= r || d < 1e-9 {
                        return false;
                    }
                    n = div_s(d0, d);
                    depth = r - d;
                }
            }
            Shape::Sphere { r: sr } => {
                let d = len(l);
                if d >= sr + r {
                    return false;
                }
                n = if d < 1e-6 { V3::new(0.0, 1.0, 0.0) } else { div_s(l, d) };
                q = V3::new(n.x * sr, n.y * sr, n.z * sr);
                depth = sr + r - d;
            }
        }
        out.local = q;
        out.point = self.cur.apply_point(q);
        out.normal = self.cur.transform_dir(n);
        out.depth = depth;
        true
    }

    /// Distance along a ray (unit `dir`) to where it enters the shape, or −1; `normal` gets the world normal.
    pub fn raycast(&self, origin: V3, dir: V3, max_t: f64, normal: &mut V3) -> f64 {
        let o = self.inv.apply_point(origin);
        let d = self.inv.transform_dir(dir);
        let t;
        let mut nl = V3::ZERO;
        match self.shape {
            Shape::Box { hx, hy, hz } => {
                let h = [hx, hy, hz];
                let oa = [o.x, o.y, o.z];
                let da = [d.x, d.y, d.z];
                let mut t0 = f64::NEG_INFINITY;
                let mut t1 = f64::INFINITY;
                let mut axis: i32 = -1;
                for a in 0..3 {
                    let (ha, o1, d1) = (h[a], oa[a], da[a]);
                    if d1.abs() < 1e-9 {
                        if o1.abs() > ha {
                            return -1.0;
                        }
                        continue;
                    }
                    let mut ta = (-ha - o1) / d1;
                    let mut tb = (ha - o1) / d1;
                    if ta > tb {
                        core::mem::swap(&mut ta, &mut tb);
                    }
                    if ta > t0 {
                        t0 = ta;
                        axis = a as i32;
                    }
                    t1 = t1.min(tb);
                    if t0 > t1 {
                        return -1.0;
                    }
                }
                if axis < 0 || t0 < 0.0 || t0 > max_t {
                    return -1.0;
                }
                t = t0;
                let s = -m::sign(da[axis as usize]);
                match axis {
                    0 => nl.x = s,
                    1 => nl.y = s,
                    _ => nl.z = s,
                }
            }
            Shape::Cyl { r, hh } => {
                let mut best = f64::INFINITY;
                if m::hypot(o.x, o.z) <= r && o.y.abs() <= hh {
                    return -1.0;
                }
                if d.y.abs() > 1e-9 {
                    for cy in [hh, -hh] {
                        let tc = (cy - o.y) / d.y;
                        if tc < 0.0 || tc >= best {
                            continue;
                        }
                        if m::hypot(o.x + d.x * tc, o.z + d.z * tc) <= r {
                            best = tc;
                            nl = V3::new(0.0, m::sign(cy), 0.0);
                        }
                    }
                }
                let a = d.x * d.x + d.z * d.z;
                if a > 1e-12 {
                    let b = o.x * d.x + o.z * d.z;
                    let c = o.x * o.x + o.z * o.z - r * r;
                    let disc = b * b - a * c;
                    if disc >= 0.0 {
                        let ts = (-b - m::sqrt(disc)) / a;
                        if ts >= 0.0 && ts < best && (o.y + d.y * ts).abs() <= hh {
                            best = ts;
                            nl = V3::new((o.x + d.x * ts) / r, 0.0, (o.z + d.z * ts) / r);
                        }
                    }
                }
                if best > max_t {
                    return -1.0;
                }
                t = best;
            }
            Shape::Sphere { r } => {
                let b = dot(o, d);
                let c = len_sq(o) - r * r;
                if c <= 0.0 {
                    return -1.0;
                }
                let disc = b * b - c;
                if disc < 0.0 {
                    return -1.0;
                }
                let tt = -b - m::sqrt(disc);
                if tt < 0.0 || tt > max_t {
                    return -1.0;
                }
                t = tt;
                nl = div_s(add_scaled(o, d, t), r);
            }
        }
        *normal = self.cur.transform_dir(nl);
        t
    }

    pub fn surface_velocity(&self, local: V3, dt: f64) -> V3 {
        let p = self.cur.apply_point(local);
        let w = self.prev.apply_point(local);
        div_s(p - w, dt.max(1e-4))
    }
}

#[inline]
fn sign_or_one(x: f64) -> f64 {
    let s = m::sign(x);
    if s == 0.0 || s.is_nan() { 1.0 } else { s }
}
