//! Properties of rotations and collider queries, on generated shapes, poses and probes.
use fb_shared::m::{self, MinMax};
use fb_sim::collider::{Collider, ColliderOpts, Contact, Shape};
use fb_sim::math::{V3, euler_xyz};
use fb_sim::nodes::{NodeId, Nodes, ROOT};
use proptest::prelude::*;

const EPS: f64 = 1e-9;

fn v3_in(r: f64) -> impl Strategy<Value = V3> {
    (-r..r, -r..r, -r..r).prop_map(|(x, y, z)| V3::new(x, y, z))
}

fn shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        (0.1..10.0f64, 0.1..10.0f64, 0.1..10.0f64).prop_map(|(hx, hy, hz)| Shape::Box { hx, hy, hz }),
        (0.1..10.0f64, 0.1..10.0f64).prop_map(|(r, hh)| Shape::Cyl { r, hh }),
        (0.1..10.0f64).prop_map(|r| Shape::Sphere { r }),
    ]
}

/// A collider moved and turned (scale 1, so local and world distances agree).
fn placed(shape: Shape, pos: V3, rot: V3) -> Collider {
    let mut nodes = Nodes::default();
    let n: NodeId = nodes.add(ROOT, pos);
    nodes.get_mut(n).rot = rot;
    nodes.update_all();
    let mut c = Collider::new(0, n, shape, ColliderOpts::default());
    c.sync(&nodes);
    c
}

/// How far a local point lies outside the shape (negative inside).
fn outside(shape: Shape, l: V3) -> f64 {
    match shape {
        Shape::Box { hx, hy, hz } => {
            let q = V3::new(l.x.abs() - hx, l.y.abs() - hy, l.z.abs() - hz);
            let o = V3::new(q.x.at_least(0.0), q.y.at_least(0.0), q.z.at_least(0.0));
            o.length() + m::min(m::max(q.x, m::max(q.y, q.z)), 0.0)
        }
        Shape::Cyl { r, hh } => {
            let (dr, dy) = (m::hypot(l.x, l.z) - r, l.y.abs() - hh);
            m::hypot(dr.at_least(0.0), dy.at_least(0.0)) + m::min(m::max(dr, dy), 0.0)
        }
        Shape::Sphere { r } => l.length() - r,
    }
}

proptest! {
    #[test]
    fn euler_rotations_are_unit(r in v3_in(20.0)) {
        let q = euler_xyz(r);
        prop_assert!((q.length() - 1.0).abs() < 1e-12, "{q:?}");
        let v = V3::new(0.3, -1.2, 2.5);
        prop_assert!(((q * v).length() - v.length()).abs() < 1e-9);
    }

    #[test]
    fn contacts_touch_the_surface_and_push_clear(
        shape in shape(),
        pos in v3_in(20.0),
        rot in v3_in(4.0),
        probe in v3_in(8.0),
        r in 0.05..2.0f64,
    ) {
        let c = placed(shape, pos, rot);
        let center = pos + probe;
        let local = c.inv.transform_point3(center);
        let mut hit = Contact::default();
        let touching = c.contact(center, r, &mut hit);
        let gap = outside(shape, local);
        if gap >= r + 1e-6 {
            prop_assert!(!touching, "{gap} away from a {shape:?}, radius {r}: {hit:?}");
        }
        if !touching {
            return Ok(());
        }
        prop_assert!(hit.depth > 0.0 && hit.depth.is_finite(), "{hit:?}");
        prop_assert!((hit.normal.length() - 1.0).abs() < 1e-9, "{hit:?}");
        prop_assert!(outside(shape, hit.local).abs() < 1e-6, "{shape:?}: {hit:?}");
        prop_assert!(hit.point.distance(c.cur.transform_point3(hit.local)) < 1e-9);
        prop_assert!((hit.depth - (r - gap)).abs() < 1e-6, "{shape:?} gap {gap} r {r}: {hit:?}");
        let pushed = center + hit.normal * (hit.depth + 1e-6);
        let mut again = Contact::default();
        prop_assert!(
            !c.contact(pushed, r, &mut again) || again.depth < 1e-5,
            "{shape:?} pushed out by {hit:?} still {again:?}"
        );
    }

    #[test]
    fn rays_stop_on_the_surface(
        shape in shape(),
        pos in v3_in(20.0),
        rot in v3_in(4.0),
        from in v3_in(30.0),
        aim in v3_in(10.0),
        max_t in 0.0..60.0f64,
    ) {
        let c = placed(shape, pos, rot);
        let origin = pos + from;
        let dir = (pos + aim - origin).normalize_or_zero();
        prop_assume!(dir != V3::ZERO);
        let Some((t, normal)) = c.raycast(origin, dir, max_t) else {
            return Ok(());
        };
        prop_assert!((0.0..=max_t).contains(&t), "{t} of {max_t}");
        prop_assert!(outside(shape, c.inv.transform_point3(origin)) > -EPS);
        let at = c.inv.transform_point3(origin + dir * t);
        prop_assert!(outside(shape, at).abs() < 1e-6, "{shape:?} hit at {at:?}");
        prop_assert!((normal.length() - 1.0).abs() < 1e-9);
        prop_assert!(normal.dot(dir) <= EPS, "a ray enters against the normal: {normal:?} {dir:?}");
    }

    #[test]
    fn a_ray_at_the_centre_from_outside_hits(
        shape in shape(),
        pos in v3_in(20.0),
        rot in v3_in(4.0),
        from in v3_in(30.0),
    ) {
        let c = placed(shape, pos, rot);
        let origin = pos + from;
        prop_assume!(outside(shape, c.inv.transform_point3(origin)) > 1e-3);
        let dir = (pos - origin).normalize();
        prop_assert!(c.raycast(origin, dir, 100.0).is_some(), "{shape:?} from {from:?}");
    }
}
