//! Names over the beans: interface nodes placed every frame where the bean's head
//! is on screen, crisp and untouched by fog and the scene's lighting. Maps may add a badge («⭐ 5»).
use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use fb_net::*;
use fb_proto::Pid;
use lightyear::prelude::*;

use super::*;
use crate::beans::BeanView;
use crate::game::Map;
use crate::session::Session;
use crate::view::MainCamera;

pub struct TagsPlugin;

impl Plugin for TagsPlugin {
    fn build(&self, app: &mut App) {
        // Where the beans and the camera are this frame (both are placed before it), and before the layout:
        // placed after it, a tag showed where its bean was a frame before.
        app.add_systems(
            PostUpdate,
            place_tags
                .after(crate::camera::place_camera)
                .before(bevy::ui::UiSystems::Layout),
        );
    }
}

/// A tag's anchor at the bean's head: moved by its `UiTransform` alone, which the layout does not lay out
/// again (a `Node` moved every frame would).
#[derive(Component)]
struct Tag {
    id: Pid,
    /// What it shows: name, colour and badge (rebuilt when that changes).
    key: u64,
    /// The pill: centred above the anchor, scaled with the distance.
    body: Entity,
}

#[derive(Component)]
struct TagBody;

fn place_tags(
    mut commands: Commands,
    layers: Query<(Entity, &Layer)>,
    beans: Query<(&PlayerId, &Transform, &Visibility, &RemotePose), (With<BeanView>, With<Interpolated>)>,
    camera: Query<(&Camera, &Transform), With<MainCamera>>,
    mut tags: Query<(Entity, &mut Tag, &mut UiTransform, &mut Visibility, &mut ZIndex), Without<BeanView>>,
    mut bodies: Query<&mut UiTransform, (With<TagBody>, Without<Tag>)>,
    session: Res<Session>,
    map: Option<Res<Map>>,
    scale: Res<UiScale>,
    f: Res<Fonts>,
) {
    let Some((layer, _)) = layers.iter().find(|(_, l)| **l == Layer::Tags) else {
        return;
    };
    // (Beans and the camera have no parent: their `Transform` is where they are, and this frame's.)
    let Ok((cam, cam_tf)) = camera.single() else { return };
    let cam_at = GlobalTransform::from(*cam_tf);
    let Some(view) = cam.logical_viewport_size() else {
        return;
    };
    let by_id: BTreeMap<Pid, Entity> = tags.iter().map(|(e, t, ..)| (t.id, e)).collect();
    let mut seen: BTreeSet<Pid> = BTreeSet::new();
    let f = &*f;
    for (id, tf, vis, pose) in &beans {
        if *vis == Visibility::Hidden || Some(id.0) == session.me {
            continue;
        }
        let head = tf.translation + Vec3::Y * (0.65 + 1.6 * pose.size);
        let d = head.distance(cam_tf.translation);
        let Ok(at) = cam.world_to_viewport(&cam_at, head) else {
            continue;
        };
        if at.x < -50.0 || at.y < -50.0 || at.x > view.x + 50.0 || at.y > view.y + 50.0 || d > 60.0 {
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
        let key = key_of(&(&name, p.color));
        seen.insert(id.0);
        let s = Vec2::splat((14.0 / d).clamp(0.55, 1.0));
        let to = Val2::px(at.x / scale.0, at.y / scale.0);
        let z = ZIndex(1000 - d.round() as i32);
        if let Some((_, mut tag, mut ut, mut v, mut zi)) = by_id.get(&id.0).and_then(|e| tags.get_mut(*e).ok()) {
            if ut.translation != to {
                ut.translation = to;
            }
            v.set_if_neq(Visibility::Inherited);
            zi.set_if_neq(z);
            if let Ok(mut b) = bodies.get_mut(tag.body)
                && b.scale != s
            {
                b.scale = s;
            }
            if tag.key != key {
                tag.key = key;
                rebuild(&mut commands, tag.body, |t| tag_body(t, f, &name, p.color));
            }
            continue;
        }
        let anchor = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    ..default()
                },
                UiTransform::from_translation(to),
                z,
                Pickable::IGNORE,
                ChildOf(layer),
            ))
            .id();
        let body = commands
            .spawn((
                TagBody,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    column_gap: rem(0.3125),
                    align_items: AlignItems::Center,
                    padding: UiRect::new(rem(0.4375), rem(0.625), rem(0.1875), rem(0.1875)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                UiTransform {
                    translation: Val2::percent(-50.0, -100.0),
                    scale: s,
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.82)),
                BorderColor::all(RIM),
                Pickable::IGNORE,
                ChildOf(anchor),
            ))
            .with_children(|t| tag_body(t, f, &name, p.color))
            .id();
        commands.entity(anchor).insert(Tag { id: id.0, key, body });
    }
    for (e, tag, _, mut v, _) in &mut tags {
        if seen.contains(&tag.id) {
            continue;
        }
        if map.is_none() || session.player(tag.id).is_none() {
            commands.entity(e).despawn();
        } else {
            v.set_if_neq(Visibility::Hidden);
        }
    }
}

fn tag_body(p: &mut ChildSpawnerCommands, f: &Fonts, name: &str, color: u8) {
    dot(p, suit(color), 0.5625);
    rich_in(p, f, name, 15.0, INK, true);
}
