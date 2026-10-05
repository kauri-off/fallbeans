//! Names over the beans: interface nodes placed every frame where the bean's head
//! is on screen, crisp and untouched by fog and the scene's lighting. Maps may add a badge («⭐ 5»).
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
        app.add_systems(PostUpdate, place_tags.after(TransformSystems::Propagate));
    }
}

#[derive(Component)]
struct Tag {
    id: Pid,
    /// What it shows: name, colour and badge (rebuilt when that changes).
    key: u64,
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
        &mut UiTransform,
        &mut Visibility,
        &mut ZIndex,
    )>,
    session: Res<Session>,
    map: Option<Res<Map>>,
    scale: Res<UiScale>,
    f: Res<Fonts>,
) {
    let Some((layer, _)) = layers.iter().find(|(_, l)| **l == Layer::Tags) else {
        return;
    };
    let Ok((cam, cam_tf)) = camera.single() else { return };
    let mut seen: Vec<Pid> = Vec::new();
    let f = &*f;
    for (id, tf, vis, pose) in &beans {
        if !vis.get() || Some(id.0) == session.me {
            continue;
        }
        let head = tf.translation() + Vec3::Y * (0.65 + 1.6 * pose.size);
        let d = head.distance(cam_tf.translation());
        let Ok(at) = cam.world_to_viewport(cam_tf, head) else {
            continue;
        };
        let Some(view) = cam.logical_viewport_size() else {
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
        seen.push(id.0);
        let s = (14.0 / d).clamp(0.55, 1.0);
        let left = px(at.x / scale.0);
        let top = px(at.y / scale.0);
        let z = ZIndex(1000 - d.round() as i32);
        if let Some((e, mut tag, mut node, mut ut, mut v, mut zi)) = tags.iter_mut().find(|t| t.1.id == id.0) {
            node.left = left;
            node.top = top;
            ut.scale = Vec2::splat(s);
            v.set_if_neq(Visibility::Inherited);
            zi.set_if_neq(z);
            if tag.key != key {
                tag.key = key;
                rebuild(&mut commands, e, |t| tag_body(t, f, &name, p.color));
            }
            continue;
        }
        commands.entity(layer).with_children(|l| {
            l.spawn((
                Tag { id: id.0, key },
                Node {
                    position_type: PositionType::Absolute,
                    left,
                    top,
                    column_gap: rem(0.3125),
                    align_items: AlignItems::Center,
                    padding: UiRect::new(rem(0.4375), rem(0.625), rem(0.1875), rem(0.1875)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                UiTransform {
                    translation: Val2::percent(-50.0, -100.0),
                    scale: Vec2::splat(s),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.82)),
                BorderColor::all(RIM),
                z,
                Pickable::IGNORE,
            ))
            .with_children(|t| tag_body(t, f, &name, p.color));
        });
    }
    for (e, tag, _, _, mut v, _) in &mut tags {
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
