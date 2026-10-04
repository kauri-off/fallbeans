//! Levels of detail of the models (port of `lod.ts`): each model mesh gets simplified versions M1…M6
//! (meshoptimizer, the same targets as TS), made once per mesh, and every placed mesh is drawn at the
//! level its size on screen calls for, cross-faded by Bevy's visibility ranges.
use std::collections::HashMap;

use bevy::camera::primitives::MeshAabb;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;

use super::meshes::{LEVELS, range_for};

/// Target share of the triangles kept at M0…M6.
const KEEP: [f32; LEVELS] = [1.0, 0.62, 0.4, 0.24, 0.13, 0.07, 0.035];

/// Simplified levels of a mesh (None: too small to bother, or not simplifiable).
fn simplify(mesh: &Mesh) -> Option<Vec<Mesh>> {
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let indices: Vec<u32> = mesh.indices()?.iter().map(|i| i as u32).collect();
    if indices.len() < 240 {
        return None;
    }
    let bytes: Vec<u8> = pos
        .iter()
        .flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    let adapter = meshopt::VertexDataAdapter::new(&bytes, 12, 0).ok()?;
    let mut out = Vec::new();
    let mut prev = indices.clone();
    for (l, keep) in KEEP.iter().enumerate().skip(1) {
        let target = ((indices.len() as f32 * keep / 3.0) as usize * 3).max(36);
        let mut res = meshopt::simplify(
            &prev,
            &adapter,
            target,
            0.035 * l as f32,
            meshopt::SimplifyOptions::Prune,
            None,
        );
        // Some shapes stop short of the target (seams, small parts): the coarsest levels may be sloppy.
        if l >= LEVELS - 2 && res.len() as f32 > target as f32 * 1.6 {
            res = meshopt::simplify_sloppy(&prev, &adapter, target, 0.08, None);
        }
        if res.is_empty() {
            res = prev.clone();
        }
        let mut m = mesh.clone();
        m.insert_indices(Indices::U32(res.clone()));
        out.push(m);
        prev = res;
    }
    Some(out)
}

/// The levels made so far, by the full mesh.
#[derive(Resource, Default)]
pub struct ModelLods {
    levels: HashMap<AssetId<Mesh>, Option<Vec<Handle<Mesh>>>>,
}

impl ModelLods {
    /// Levels 1…6 of a mesh (made on first use).
    pub fn of(&mut self, mesh: &Handle<Mesh>, meshes: &mut Assets<Mesh>) -> Option<Vec<Handle<Mesh>>> {
        if let Some(l) = self.levels.get(&mesh.id()) {
            return l.clone();
        }
        let made = meshes
            .get(mesh)
            .and_then(simplify)
            .map(|ms| ms.into_iter().map(|m| meshes.add(m)).collect::<Vec<_>>());
        self.levels.insert(mesh.id(), made.clone());
        made
    }
}

/// Spawns the levels of a placed model mesh as its siblings, each shown at its distances.
pub fn add_levels<M: Material>(
    commands: &mut Commands,
    e: Entity,
    mesh: &Handle<Mesh>,
    material: &Handle<M>,
    tf: Transform,
    parent: Entity,
    scale: f32,
    lods: &mut ModelLods,
    meshes: &mut Assets<Mesh>,
    k: f32,
) {
    let Some(levels) = lods.of(mesh, meshes) else { return };
    let Some(aabb) = meshes.get(mesh).and_then(|m| m.compute_aabb()) else {
        return;
    };
    let radius = Vec3::from(aabb.half_extents).length() * scale;
    commands.entity(e).insert(range_for(radius, k, 0));
    for (i, h) in levels.into_iter().enumerate() {
        commands.spawn((
            Mesh3d(h),
            MeshMaterial3d(material.clone()),
            tf,
            Visibility::default(),
            range_for(radius, k, i + 1),
            ChildOf(parent),
        ));
    }
}
