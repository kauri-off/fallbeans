//! Transform propagation on one thread instead of Bevy's on all of the compute pool: the scene's hierarchies
//! are small, and the parallel one waits for every pool thread it hands work to (spinning) — a frame of a
//! loaded machine waits for the slowest of a dozen threads to be scheduled.
use bevy::ecs::schedule::ScheduleCleanupPolicy;
use bevy::prelude::*;
use bevy::transform::components::TransformTreeChanged;
use bevy::transform::systems::{
    StaticTransformOptimizations, mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms,
};

pub struct TransformsPlugin;

impl Plugin for TransformsPlugin {
    fn build(&self, app: &mut App) {
        if app
            .remove_systems_in_set(
                PostUpdate,
                propagate_parent_transforms,
                ScheduleCleanupPolicy::RemoveSystemsOnly,
            )
            .is_err()
        {
            warn!("transforms: Bevy's propagation not found, kept");
            return;
        }
        app.add_systems(
            PostUpdate,
            propagate
                .in_set(TransformSystems::Propagate)
                .after(mark_dirty_trees)
                .before(sync_simple_transforms),
        );
    }
}

type Roots<'w, 's> = Query<
    'w,
    's,
    (
        Ref<'static, Transform>,
        &'static mut GlobalTransform,
        &'static Children,
        Ref<'static, TransformTreeChanged>,
    ),
    Without<ChildOf>,
>;

type Nodes<'w, 's> = Query<
    'w,
    's,
    (
        Ref<'static, Transform>,
        &'static mut GlobalTransform,
        Ref<'static, TransformTreeChanged>,
        Option<&'static Children>,
    ),
    With<ChildOf>,
>;

/// As Bevy's `propagate_parent_transforms`: a root whose tree changed sets its children; below them a subtree is
/// skipped when neither it changed nor its parent's global transform did.
fn propagate(
    mut roots: Roots,
    mut nodes: Nodes,
    statics: Res<StaticTransformOptimizations>,
    mut stack: Local<Vec<(Entity, GlobalTransform, bool)>>,
) {
    let skip = statics.is_enabled();
    for (tf, mut global, children, tree) in &mut roots {
        if skip && !tree.is_changed() {
            continue;
        }
        let at = GlobalTransform::from(*tf);
        *global = at;
        stack.extend(children.iter().map(|c| (c, at, true)));
        while let Some((e, parent, moved)) = stack.pop() {
            let Ok((tf, mut global, tree, children)) = nodes.get_mut(e) else {
                continue;
            };
            if skip && !moved && !tree.is_changed() {
                continue;
            }
            let at = parent.mul_transform(*tf);
            let changed = global.set_if_neq(at);
            if let Some(children) = children {
                stack.extend(children.iter().map(|c| (c, at, changed)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::TaskPoolPlugin;
    use bevy::transform::TransformPlugin;

    fn app(serial: bool) -> App {
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), TransformPlugin));
        if serial {
            app.add_plugins(TransformsPlugin);
        }
        app
    }

    /// A few trees, then moves at every depth, a reparenting and a despawn: the same global transforms as
    /// Bevy's propagation, frame by frame.
    #[test]
    fn matches_bevy() {
        let mut apps = [app(false), app(true)];
        let mut ids: Vec<Vec<Entity>> = vec![Vec::new(), Vec::new()];
        for (app, ids) in apps.iter_mut().zip(&mut ids) {
            let w = app.world_mut();
            for r in 0..3 {
                let root = w.spawn(Transform::from_xyz(r as f32, 0.0, 0.0)).id();
                ids.push(root);
                let mut parent = root;
                for d in 0..4 {
                    let a = w
                        .spawn((
                            Transform::from_xyz(0.0, 1.0, d as f32)
                                .with_rotation(Quat::from_rotation_y(0.3 * d as f32)),
                            ChildOf(parent),
                        ))
                        .id();
                    let b = w.spawn((Transform::from_scale(Vec3::splat(1.5)), ChildOf(parent))).id();
                    ids.extend([a, b]);
                    parent = a;
                }
            }
        }
        apps[1].update();
        let names: Vec<String> = apps[1]
            .get_schedule(PostUpdate)
            .and_then(|s| s.systems().ok())
            .map(|s| s.map(|(_, s)| s.name().to_string()).collect())
            .unwrap_or_default();
        assert!(names.iter().any(|n| n.ends_with("transforms::propagate")), "{names:?}");
        assert!(
            !names.iter().any(|n| n.contains("propagate_parent_transforms")),
            "{names:?}"
        );
        apps[0].update();
        let n = ids[0].len();
        for frame in 0..8 {
            for (app, ids) in apps.iter_mut().zip(&ids) {
                let w = app.world_mut();
                match frame {
                    1 => w.entity_mut(ids[3]).get_mut::<Transform>().unwrap().translation.x += 2.0,
                    2 => w.entity_mut(ids[0]).get_mut::<Transform>().unwrap().rotate_z(0.5),
                    3 => {
                        w.entity_mut(ids[n - 1]).get_mut::<Transform>().unwrap().scale = Vec3::splat(0.5);
                        w.entity_mut(ids[n / 2]).get_mut::<Transform>().unwrap().translation.y -= 1.0;
                    }
                    4 => {
                        let to = ids[1];
                        w.entity_mut(ids[n - 3]).insert(ChildOf(to));
                    }
                    5 => {
                        w.entity_mut(ids[n / 2 + 1]).despawn();
                    }
                    _ => {}
                }
                app.update();
            }
            for (i, (a, b)) in ids[0].iter().zip(&ids[1]).enumerate() {
                let get = |app: &App, e: Entity| app.world().get::<GlobalTransform>(e).map(|g| g.affine());
                let (ga, gb) = (get(&apps[0], *a), get(&apps[1], *b));
                assert!(
                    match (ga, gb) {
                        (Some(x), Some(y)) => x.abs_diff_eq(y, 1e-6),
                        (x, y) => x.is_none() && y.is_none(),
                    },
                    "frame {frame}, entity {i}: {ga:?} vs {gb:?}"
                );
            }
        }
    }
}
