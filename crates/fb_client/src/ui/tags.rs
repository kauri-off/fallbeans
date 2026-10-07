//! Names over the beans: interface nodes placed every frame where the bean's head
//! is on screen, crisp and untouched by fog and the scene's lighting. Maps may add a badge («⭐ 5»).
//! A nearer tag keeps its place, a farther one that would cover it moves up; tags of far beans and of
//! beans behind the course (a wall, the floor over one that fell) fade out.
use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use fb_net::*;
use fb_proto::Pid;
use fb_sim::collider::ColId;
use fb_sim::math::V3;
use fb_sim::world::World;
use lightyear::prelude::*;

use super::*;
use crate::beans::BeanView;
use crate::game::Map;
use crate::session::Session;
use crate::view::MainCamera;

/// No tag past this distance (m); they fade out from `FADE_FROM`.
const FAR: f32 = 60.0;
const FADE_FROM: f32 = 40.0;
/// Text sizes (of the nearest tag's): a tag is laid out at its size rather than scaled, so the text stays sharp.
const STEPS: [f32; 4] = [0.7, 0.8, 0.9, 1.0];
/// Between stacked tags, and from the screen's edges (px).
const GAP: f32 = 2.0;
const MARGIN: f32 = 4.0;

pub struct TagsPlugin;

impl Plugin for TagsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            place_tags
                .after(TransformSystems::Propagate)
                .after(VisibilitySystems::VisibilityPropagate),
        );
    }
}

#[derive(Component)]
struct Tag {
    id: Pid,
    /// What it shows: name, colour and badge (rebuilt when that changes).
    key: u64,
    color: u8,
    /// Its size, an index into `STEPS`.
    step: usize,
    /// How far above the head it is drawn (px), easing towards where it does not cover a nearer tag.
    lift: f32,
    /// Opacity, easing towards the wanted one; and the opacity its colours were last set to.
    alpha: f32,
    painted: f32,
    /// The course is between the camera and the bean (looked again every few frames).
    blocked: bool,
}

struct Seen {
    id: Pid,
    /// The head on screen (px of the interface), the tag's bottom middle.
    at: Vec2,
    d: f32,
    name: String,
    key: u64,
    color: u8,
    /// The points looked at for walls: the head and the middle of the body.
    aims: [Vec3; 2],
}

fn place_tags(
    mut commands: Commands,
    layers: Query<(Entity, &Layer)>,
    beans: Query<
        (&PlayerId, &GlobalTransform, &InheritedVisibility, &RemotePose),
        (With<BeanView>, With<Interpolated>),
    >,
    camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut tags: Query<(
        Entity,
        &mut Tag,
        &mut Node,
        &ComputedNode,
        &mut Visibility,
        &mut ZIndex,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
    children: Query<&Children>,
    mut dots: Query<(&mut BackgroundColor, &mut BorderColor), Without<Tag>>,
    mut texts: Query<&mut TextColor>,
    session: Res<Session>,
    map: Option<Res<Map>>,
    scale: Res<UiScale>,
    time: Res<Time<Real>>,
    f: Res<Fonts>,
    mut frame: Local<u32>,
    mut near: Local<(Vec<ColId>, Vec<ColId>)>,
) {
    let Some((layer, _)) = layers.iter().find(|(_, l)| **l == Layer::Tags) else {
        return;
    };
    let Ok((cam, cam_tf)) = camera.single() else { return };
    let Some(view) = cam.logical_viewport_size() else {
        return;
    };
    *frame = frame.wrapping_add(1);
    let dt = time.delta_secs().min(0.1);
    let eye = cam_tf.translation();
    let mut seen: Vec<Seen> = Vec::new();
    for (id, tf, vis, pose) in &beans {
        if !vis.get() || Some(id.0) == session.me {
            continue;
        }
        let feet = tf.translation();
        let head = feet + Vec3::Y * (0.65 + 1.6 * pose.size);
        let d = head.distance(eye);
        let Ok(at) = cam.world_to_viewport(cam_tf, head) else {
            continue;
        };
        if at.x < -50.0 || at.y < -50.0 || at.x > view.x + 50.0 || at.y > view.y + 50.0 || d > FAR {
            continue;
        }
        let Some(p) = session.player(id.0) else { continue };
        let badge = map
            .as_ref()
            .and_then(|m| m.deco.get(&id.0))
            .and_then(|d| d.badge.clone());
        let name = match &badge {
            Some(b) => format!("{} {b}", p.name),
            None => p.name.clone(),
        };
        seen.push(Seen {
            id: id.0,
            at: at / scale.0,
            d,
            key: key_of(&(&name, p.color)),
            name,
            color: p.color,
            aims: [head, feet + Vec3::Y * 0.9 * pose.size],
        });
    }
    seen.sort_by(|a, b| a.d.total_cmp(&b.d));

    let f = &*f;
    let (near, checked) = &mut *near;
    // Tags already up, nearest first, with how much they want to show.
    let mut shown: Vec<(Entity, usize, f32)> = Vec::new();
    for (i, s) in seen.iter().enumerate() {
        let want_step = STEPS
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - size_at(s.d)).abs().total_cmp(&(b.1 - size_at(s.d)).abs()))
            .map_or(STEPS.len() - 1, |(k, _)| k);
        let mut look = |world: &World| s.aims.iter().all(|&to| blocked(world, eye, to, near, checked));
        let Some((e, mut tag, mut node, ..)) = tags.iter_mut().find(|t| t.1.id == s.id) else {
            let blocked = map.as_ref().is_some_and(|m| look(&m.world));
            commands.entity(layer).with_children(|l| {
                l.spawn((
                    Tag {
                        id: s.id,
                        key: s.key,
                        color: s.color,
                        step: want_step,
                        lift: 0.0,
                        alpha: 0.0,
                        painted: 0.0,
                        blocked,
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(s.at.x),
                        top: px(s.at.y),
                        ..frame_node(STEPS[want_step])
                    },
                    UiTransform {
                        translation: Val2::percent(-50.0, -100.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderColor::all(Color::NONE),
                    ZIndex(depth(s.d)),
                    Visibility::Hidden,
                    Pickable::IGNORE,
                ))
                .with_children(|t| tag_body(t, f, &s.name, s.color, STEPS[want_step], 0.0));
            });
            continue;
        };
        if (*frame).wrapping_add(s.id) % 4 == 0 {
            tag.blocked = map.as_ref().is_some_and(|m| look(&m.world));
        }
        // Change size only when clearly nearer another step: no flicker between two.
        if (size_at(s.d) - STEPS[tag.step]).abs() > 0.075 && tag.step != want_step {
            tag.step = want_step;
            let k = STEPS[want_step];
            let fresh = frame_node(k);
            node.padding = fresh.padding;
            node.column_gap = fresh.column_gap;
            tag.key = 0;
        }
        if tag.key != s.key {
            tag.key = s.key;
            tag.color = s.color;
            let (k, a) = (STEPS[tag.step], tag.alpha);
            tag.painted = a;
            rebuild(&mut commands, e, |t| tag_body(t, f, &s.name, s.color, k, a));
        }
        let want = if tag.blocked {
            0.0
        } else {
            1.0 - smoothstep(s.d, FADE_FROM, FAR)
        };
        shown.push((e, i, want));
    }

    // Stack the ones showing so that none covers a nearer one.
    let slots: Vec<(Vec2, Vec2)> = shown
        .iter()
        .filter(|(.., want)| *want > 0.0)
        .map(|&(e, i, _)| {
            let size = tags
                .get(e)
                .map_or(Vec2::ZERO, |t| t.3.size() * t.3.inverse_scale_factor());
            (seen[i].at, size)
        })
        .collect();
    let mut spots = arrange(&slots, view / scale.0).into_iter();
    let ease = |k: f32| 1.0 - (-dt * k).exp();
    for &(e, i, want) in &shown {
        let s = &seen[i];
        let Ok((_, mut tag, mut node, _, mut vis, mut z, mut bg, mut rim)) = tags.get_mut(e) else {
            continue;
        };
        let spot = if want > 0.0 { spots.next() } else { None };
        let spot = spot.unwrap_or(Vec2::new(s.at.x, s.at.y - tag.lift));
        tag.lift += (s.at.y - spot.y - tag.lift) * ease(14.0);
        node.left = px(spot.x);
        node.top = px(s.at.y - tag.lift);
        z.set_if_neq(ZIndex(depth(s.d)));
        tag.alpha += (want - tag.alpha) * ease(10.0);
        if (want - tag.alpha).abs() < 0.01 {
            tag.alpha = want;
        }
        vis.set_if_neq(if tag.alpha > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let a = tag.alpha;
        // Recoloured in steps, and once more on reaching shown or gone.
        if (a - tag.painted).abs() > 0.04 || (a != tag.painted && (a == 0.0 || a == 1.0)) {
            tag.painted = a;
            *bg = BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.82 * a));
            *rim = BorderColor::all(RIM.with_alpha(0.8 * a));
            let dot = suit(tag.color).with_alpha(a);
            for c in children.iter_descendants(e) {
                if let Ok((mut fill, mut border)) = dots.get_mut(c) {
                    *fill = BackgroundColor(dot);
                    *border = BorderColor::all(dot_rim(dot));
                }
                if let Ok(mut t) = texts.get_mut(c) {
                    t.0 = INK.with_alpha(a);
                }
            }
        }
    }

    for (e, mut tag, _, _, mut v, ..) in &mut tags {
        if seen.iter().any(|s| s.id == tag.id) {
            continue;
        }
        if map.is_none() || session.player(tag.id).is_none() {
            commands.entity(e).despawn();
        } else {
            // Back into view later: fades in where it is then.
            tag.alpha = 0.0;
            v.set_if_neq(Visibility::Hidden);
        }
    }
}

/// The size a tag this far away wants (of the nearest's).
fn size_at(d: f32) -> f32 {
    (14.0 / d).clamp(STEPS[0], 1.0)
}

/// Nearer tags in front.
fn depth(d: f32) -> i32 {
    1000 - d.round() as i32
}

fn smoothstep(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The tag's frame at size `k`.
fn frame_node(k: f32) -> Node {
    Node {
        column_gap: rem(0.3125 * k),
        align_items: AlignItems::Center,
        padding: UiRect::new(rem(0.4375 * k), rem(0.625 * k), rem(0.1875 * k), rem(0.1875 * k)),
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::MAX,
        ..default()
    }
}

fn tag_body(p: &mut ChildSpawnerCommands, f: &Fonts, name: &str, color: u8, k: f32, alpha: f32) {
    dot(p, suit(color).with_alpha(alpha), 0.5625 * k);
    rich_in(p, f, name, 15.0 * k, INK.with_alpha(alpha), true);
}

/// Where each tag goes, nearest first (each is its bottom middle and its size): inside the screen, and
/// above every nearer tag it would cover.
fn arrange(slots: &[(Vec2, Vec2)], view: Vec2) -> Vec<Vec2> {
    let mut placed: Vec<Rect> = Vec::with_capacity(slots.len());
    let mut out = Vec::with_capacity(slots.len());
    for &(at, size) in slots {
        let half = size.x / 2.0;
        let x = if view.x > size.x + 2.0 * MARGIN {
            at.x.clamp(half + MARGIN, view.x - half - MARGIN)
        } else {
            at.x
        };
        let rect = |bottom: f32| Rect::new(x - half, bottom - size.y, x + half, bottom);
        let mut bottom = at.y;
        // Each move clears one nearer tag for good (it only goes up), so this many tries are enough.
        for _ in 0..=placed.len() {
            let Some(p) = placed.iter().find(|p| !p.intersect(rect(bottom)).is_empty()) else {
                break;
            };
            bottom = p.min.y - GAP;
        }
        placed.push(rect(bottom));
        out.push(Vec2::new(x, bottom));
    }
    out
}

/// Whether the course stands between `from` and `to`: any solid collider, moving hazards aside
/// (a hammer passing by should not blink the names behind it).
fn blocked(world: &World, from: Vec3, to: Vec3, near: &mut Vec<ColId>, checked: &mut Vec<ColId>) -> bool {
    let span = to - from;
    let len = span.length();
    if len < 1.0 {
        return false;
    }
    let dir = span / len;
    let (o, d) = (
        V3::new(from.x as f64, from.y as f64, from.z as f64),
        V3::new(dir.x as f64, dir.y as f64, dir.z as f64),
    );
    const STEP: f32 = 2.0;
    checked.clear();
    let mut normal = V3::ZERO;
    let n = (len / STEP).ceil() as usize;
    for i in 0..=n {
        let p = from + dir * (i as f32 * STEP).min(len);
        world.query(p.x as f64, p.z as f64, STEP as f64, near);
        for &ci in near.iter() {
            if checked.contains(&ci) {
                continue;
            }
            checked.push(ci);
            let c = world.col(ci);
            if !c.enabled || c.trigger || c.hit > 0.0 || c.sweep {
                continue;
            }
            if c.raycast(o, d, (len - 0.3) as f64, &mut normal) >= 0.0 {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Vec2 = Vec2::new(1600.0, 900.0);
    const TAG: Vec2 = Vec2::new(120.0, 24.0);

    #[test]
    fn a_farther_tag_over_a_nearer_one_moves_up() {
        let out = arrange(&[(Vec2::new(800.0, 400.0), TAG), (Vec2::new(830.0, 410.0), TAG)], VIEW);
        assert_eq!(out[0], Vec2::new(800.0, 400.0));
        assert_eq!(out[1].x, 830.0);
        assert_eq!(out[1].y, 400.0 - TAG.y - GAP);
    }

    #[test]
    fn three_in_a_heap_stack_up() {
        let at = Vec2::new(800.0, 400.0);
        let out = arrange(&[(at, TAG), (at, TAG), (at + Vec2::new(5.0, -3.0), TAG)], VIEW);
        assert_eq!(out[1].y, 400.0 - TAG.y - GAP);
        assert_eq!(out[2].y, out[1].y - TAG.y - GAP);
    }

    #[test]
    fn tags_apart_stay_put() {
        let a = Vec2::new(300.0, 400.0);
        let b = Vec2::new(900.0, 400.0);
        assert_eq!(arrange(&[(a, TAG), (b, TAG)], VIEW), vec![a, b]);
    }

    #[test]
    fn a_tag_at_the_edge_stays_on_screen() {
        let out = arrange(&[(Vec2::new(1590.0, 400.0), TAG)], VIEW);
        assert_eq!(out[0].x, VIEW.x - TAG.x / 2.0 - MARGIN);
    }
}
