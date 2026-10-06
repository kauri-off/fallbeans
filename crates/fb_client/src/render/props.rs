//! The map's models: their materials get surfaces by material name (a flag's
//! pennant and a mushroom's cap take the map's tint), and the props move: pennants sway, fans
//! spin, stars twirl and bob over the finish, mushroom caps squash like jelly.
use std::collections::HashMap;

use bevy::gltf::GltfMaterialName;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::lod::{ModelLods, add_levels};
use super::surface::{Kind, SurfaceMaterial, Surfaces};
use crate::game::Map;
use crate::view::frame_tick;

/// A map model: what it is, its tint, and the phase of its motion (from where it stands).
#[derive(Component)]
pub struct Prop {
    pub name: &'static str,
    pub tint: Option<&'static str>,
    pub phase: f32,
    /// Materials repainted by name: colour, and how much of it glows (clouds of a tinted sky).
    pub paint: Vec<(&'static str, String, f32)>,
    /// A special's piece: its special moves it, and it gets no level-of-detail copies.
    pub special: bool,
    /// Scenery or a special's piece: its own transform is left alone (stars and mushrooms of the map
    /// bob and squash; the scenery's are placed and scaled by `decor.rs`).
    still: bool,
}

impl Prop {
    pub fn new(name: &'static str, tint: Option<&'static str>, x: f64, z: f64) -> Self {
        // (x and z of the placement: the node's own position.)
        let phase = if name == "mushroom" {
            ((x * 3.1 + z * 1.3) % core::f64::consts::TAU) as f32
        } else {
            ((x * 12.9898 + z * 78.233) % core::f64::consts::TAU) as f32
        };
        Self {
            name,
            tint,
            phase,
            paint: Vec::new(),
            special: false,
            still: false,
        }
    }

    pub fn special(name: &'static str) -> Self {
        Self {
            special: true,
            ..Self::painted(name, Vec::new())
        }
    }

    /// A model of the scenery: no motion of its own, materials repainted by name.
    pub fn painted(name: &'static str, paint: Vec<(&'static str, String, f32)>) -> Self {
        Self {
            name,
            tint: None,
            phase: 0.0,
            paint,
            special: false,
            still: true,
        }
    }
}

/// A part of a prop that moves, with its pose as modelled.
#[derive(Component)]
struct Moving {
    prop: Entity,
    rest: Transform,
}

/// A mesh `dress` is done with: a model's, now on its surface, or one of no model (beans, bonuses…), which
/// is not looked at again.
#[derive(Component)]
struct Dressed;

pub struct PropsPlugin;

impl Plugin for PropsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModelLods>();
        app.add_systems(Update, (dress, find_moving, animate).chain());
        app.add_systems(Update, (super::lod::finish_levels, super::meshes::refresh_bands));
    }
}

fn prop_of(mut e: Entity, parents: &Query<&ChildOf>, props: &Query<&Prop>) -> Option<Entity> {
    for _ in 0..12 {
        if props.contains(e) {
            return Some(e);
        }
        e = parents.get(e).ok()?.parent();
    }
    None
}

/// Model meshes as they appear: the standard material from the glTF becomes a surface material. A model whose
/// root casts no shadow (scenery) gives that to each of its meshes: the component is not inherited.
#[allow(clippy::type_complexity)]
fn dress(
    mut commands: Commands,
    meshes: Query<
        (
            Entity,
            &MeshMaterial3d<StandardMaterial>,
            Option<&GltfMaterialName>,
            Option<&Name>,
        ),
        Without<Dressed>,
    >,
    parents: Query<&ChildOf>,
    props: Query<&Prop>,
    shadowless: Query<(), (With<Prop>, With<NotShadowCaster>)>,
    standard: Res<Assets<StandardMaterial>>,
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
    mut done: Local<HashMap<(AssetId<StandardMaterial>, String), Handle<SurfaceMaterial>>>,
    lod: (
        ResMut<ModelLods>,
        Res<Assets<Mesh>>,
        Res<crate::settings::Display>,
        Option<Res<super::quality::Quality>>,
    ),
    shapes: Query<(&Mesh3d, &Transform, &ChildOf)>,
    transforms: Query<&Transform>,
) {
    let (mut lods, mesh_assets, display, quality) = lod;
    let k = super::meshes::lod_k(display.fov, quality.map(|q| q.preset));
    if !meshes.is_empty() {
        // (Materials of a model unloaded between maps come back under new ids: forget the old ones.)
        done.retain(|(id, _), _| standard.contains(*id));
    }
    for (e, mat, mat_name, name) in &meshes {
        let Some(p) = prop_of(e, &parents, &props) else {
            commands.entity(e).try_insert(Dressed);
            continue;
        };
        let Some(base) = standard.get(&mat.0) else { continue };
        let prop = props.get(p).ok();
        let part = name.map(|n| n.as_str()).unwrap_or("");
        let mat_name = mat_name.map(|n| n.0.as_str()).unwrap_or("");
        let tinted = part.starts_with("Pennant") || part.starts_with("MushCap");
        let tint = prop.and_then(|p| p.tint).filter(|_| tinted);
        let paint = prop.and_then(|p| p.paint.iter().find(|(n, _, _)| *n == mat_name));
        let key = format!("{tint:?} {paint:?}");
        let h = done
            .entry((mat.0.id(), key))
            .or_insert_with(|| {
                let mut base = base.clone();
                let kind = match tint {
                    Some(_) if part.starts_with("Pennant") => Some(Kind::Cloth),
                    Some(_) => Some(Kind::Glossy),
                    None => Kind::of_model(mat_name),
                };
                if let Some(t) = tint {
                    base.base_color = crate::view::hex(t);
                    base.base_color_texture = None;
                }
                if let Some((_, c, glow)) = paint {
                    let c = crate::view::hex(c);
                    base.base_color = c;
                    base.base_color_texture = None;
                    if *glow > 0.0 {
                        base.emissive = c.to_linear() * *glow;
                    }
                }
                if let Some(k) = kind
                    && !matches!(k, Kind::Metal | Kind::Gold)
                    && base.metallic > 0.0
                {
                    base.metallic = 0.0;
                }
                surfaces.material_from(base, kind, None, mat_name == "Glint", None, &mut images, &mut materials)
            })
            .clone();
        let shadow = !shadowless.contains(p);
        let mut dressed = commands.entity(e);
        dressed
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((MeshMaterial3d(h.clone()), Dressed));
        if !shadow {
            dressed.insert(NotShadowCaster);
        }
        // Levels of detail for what stands still (moving parts would leave their copies behind).
        let moving = part.starts_with("Pennant") || part.starts_with("FanBlades") || prop.is_some_and(|p| p.special);
        if !moving && let Ok((mesh, tf, parent)) = shapes.get(e) {
            // (The levels switch by the size on screen: the scales of the mesh and all above it count.
            // The global transform is not propagated yet for a scene just spawned.)
            let mut scale = tf.scale.abs().max_element();
            let mut up = Some(parent.parent());
            while let Some(a) = up {
                if let Ok(t) = transforms.get(a) {
                    scale *= t.scale.abs().max_element();
                }
                up = parents.get(a).ok().map(ChildOf::parent);
            }
            add_levels(
                &mut commands,
                e,
                &mesh.0,
                &h,
                *tf,
                parent.parent(),
                scale,
                shadow,
                &mut lods,
                &mesh_assets,
                k,
            );
        }
    }
}

/// The parts that move (by node name), once the model's scene is in.
fn find_moving(
    mut commands: Commands,
    named: Query<(Entity, &Name, &Transform), (Added<Name>, Without<Moving>)>,
    parents: Query<&ChildOf>,
    props: Query<&Prop>,
) {
    for (e, name, tf) in &named {
        if !matches!(name.as_str(), "Pennant" | "FanBlades") {
            continue;
        }
        if let Some(p) = prop_of(e, &parents, &props) {
            commands.entity(e).insert(Moving { prop: p, rest: *tf });
        }
    }
}

fn animate(
    map: Option<Res<Map>>,
    timeline: Res<lightyear::prelude::LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    mut props: Query<(&Prop, &mut Transform), Without<Moving>>,
    mut parts: Query<(&Moving, &Name, &mut Transform), Without<Prop>>,
) {
    let Some(map) = map else { return };
    let t = map.time(frame_tick(&timeline, &fixed)) as f32;
    for (p, mut tf) in &mut props {
        if p.still {
            continue;
        }
        let ph = p.phase;
        match p.name {
            "star" => {
                tf.rotation = Quat::from_rotation_y(t * 1.4 + ph);
                tf.translation.y = (t * 1.8 + ph).sin() * 0.2;
            }
            "mushroom" => {
                let k = (t * 5.0 + ph).sin().max(0.0).powi(6);
                tf.scale = Vec3::new(1.0 + k * 0.05, 1.0 - k * 0.06, 1.0 + k * 0.05);
            }
            _ => {}
        }
    }
    for (m, name, mut tf) in &mut parts {
        let Ok((p, prop_tf)) = props.get(m.prop) else { continue };
        let ph = p.phase;
        match name.as_str() {
            "Pennant" => {
                // One wind over the map: gusts travel across it, so neighbouring flags swing together a moment
                // apart (each on a phase of its own looked like flags in different weathers); a flag's own
                // phase only adds its flutter.
                let gust = prop_tf.translation.x * 0.3 + prop_tf.translation.z * 0.18;
                let a = (t * 2.2 - gust).sin() * 0.2
                    + (t * 0.7 - gust * 0.4).sin() * 0.08
                    + (t * 6.3 - gust * 2.0 + ph).sin() * 0.045;
                tf.rotation = m.rest.rotation * Quat::from_rotation_y(a);
            }
            "FanBlades" => {
                tf.rotation = m.rest.rotation * Quat::from_rotation_z(t * 6.0 + ph);
            }
            _ => {}
        }
    }
}
