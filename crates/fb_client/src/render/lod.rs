//! Levels of detail of the models: a model mesh with enough triangles gets simplified versions
//! (meshoptimizer, made once per mesh in a background task, each with only the vertices it uses), and every
//! placed mesh is drawn at the level its size on screen calls for.
use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::MeshAabb;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};

use super::meshes::{BANDS, LodBand, lod_k};
use super::surface::SurfaceMaterial;

/// A mesh with fewer triangles is drawn as it is at every distance (so are all of today's models: an extra
/// entity per level costs more than the triangles it saves).
const MIN_TRIS: usize = 2000;
/// The levels: the bands each spans and the share of the triangles it keeps.
const LEVELS: [(usize, usize, f32); 3] = [(0, 1, 1.0), (2, 3, 0.4), (4, BANDS - 1, 0.13)];

fn pick<T: Copy>(values: &[T], order: &[u32]) -> Option<Vec<T>> {
    order.iter().map(|&i| values.get(i as usize).copied()).collect()
}

/// The triangles `indices` of a mesh as a mesh of the vertices they use alone, for the GPU only (nothing
/// reads a level back).
fn compact(mesh: &Mesh, vertices: usize, indices: &[u32]) -> Option<Mesh> {
    use VertexAttributeValues as V;
    let mut remap = vec![u32::MAX; vertices];
    let mut order: Vec<u32> = Vec::new();
    let mut idx = Vec::with_capacity(indices.len());
    for &i in indices {
        let r = remap.get_mut(i as usize)?;
        if *r == u32::MAX {
            *r = order.len() as u32;
            order.push(i);
        }
        idx.push(*r);
    }
    let mut out = Mesh::new(mesh.primitive_topology(), RenderAssetUsages::RENDER_WORLD);
    for (attribute, values) in mesh.try_attributes().ok()? {
        let values = match values {
            V::Float32(v) => V::Float32(pick(v, &order)?),
            V::Float32x2(v) => V::Float32x2(pick(v, &order)?),
            V::Float32x3(v) => V::Float32x3(pick(v, &order)?),
            V::Float32x4(v) => V::Float32x4(pick(v, &order)?),
            V::Uint16x4(v) => V::Uint16x4(pick(v, &order)?),
            V::Unorm16x2(v) => V::Unorm16x2(pick(v, &order)?),
            V::Unorm8x4(v) => V::Unorm8x4(pick(v, &order)?),
            // (No model has others: such a mesh keeps its one level.)
            _ => return None,
        };
        out.insert_attribute(*attribute, values);
    }
    out.insert_indices(Indices::U32(idx));
    Some(out)
}

/// The levels after the first of a mesh (None: too few triangles, or not simplifiable).
fn simplify(mesh: &Mesh) -> Option<Vec<Mesh>> {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return None;
    }
    let Ok(VertexAttributeValues::Float32x3(pos)) = mesh.try_attribute(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let indices: Vec<u32> = mesh.try_indices().ok()?.iter().map(|i| i as u32).collect();
    let full = indices.len();
    if full < MIN_TRIS * 3 {
        return None;
    }
    let bytes: Vec<u8> = pos
        .iter()
        .flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes()))
        .collect();
    let adapter = meshopt::VertexDataAdapter::new(&bytes, 12, 0).ok()?;
    let mut out = Vec::new();
    let mut prev = indices;
    for (l, &(band, _, keep)) in LEVELS.iter().enumerate().skip(1) {
        let target = ((full as f32 * keep / 3.0) as usize * 3).max(36);
        let mut res = meshopt::simplify(
            &prev,
            &adapter,
            target,
            0.035 * band as f32,
            meshopt::SimplifyOptions::Prune,
            None,
        );
        // Some shapes stop short of the target (seams, small parts): the coarsest level may be sloppy.
        if l == LEVELS.len() - 1 && res.len() as f32 > target as f32 * 1.6 {
            res = meshopt::simplify_sloppy(&prev, &adapter, target, 0.08, None);
        }
        if res.is_empty() {
            res = prev.clone();
        }
        out.push(compact(mesh, pos.len(), &res)?);
        prev = res;
    }
    Some(out)
}

/// A placed model mesh, for its levels.
struct Placed {
    e: Entity,
    parent: Entity,
    tf: Transform,
    material: Handle<SurfaceMaterial>,
    /// Bounding radius (scaled).
    r: f32,
    shadow: bool,
}

/// The levels made so far, by the full mesh, and those being made.
#[derive(Resource, Default)]
pub struct ModelLods {
    /// Levels after the first (None: the mesh is drawn as it is).
    levels: HashMap<AssetId<Mesh>, Option<Vec<Handle<Mesh>>>>,
    /// Meshes being simplified, with the placements waiting for their levels.
    making: HashMap<AssetId<Mesh>, (Task<Option<Vec<Mesh>>>, Vec<Placed>)>,
}

impl ModelLods {
    /// Meshes still being simplified.
    pub fn making(&self) -> usize {
        self.making.len()
    }

    /// Starts a model mesh's levels before anything places it (the warm-up: every model at the start).
    pub fn prepare(&mut self, mesh: &Handle<Mesh>, meshes: &Assets<Mesh>) {
        let id = mesh.id();
        if self.levels.contains_key(&id) || self.making.contains_key(&id) {
            return;
        }
        let Some(m) = meshes.get(mesh) else { return };
        if m.try_indices_option()
            .ok()
            .flatten()
            .is_none_or(|i| i.len() < MIN_TRIS * 3)
        {
            self.levels.insert(id, None);
            return;
        }
        let m = m.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move { simplify(&m) });
        self.making.insert(id, (task, Vec::new()));
    }
}

/// Gives a placed model mesh (entity `e`, a child of `parent`) its levels of detail: siblings shown at their
/// distances, now or once they are made. `scale`: of the mesh and all above it.
pub fn add_levels(
    commands: &mut Commands,
    e: Entity,
    mesh: &Handle<Mesh>,
    material: &Handle<SurfaceMaterial>,
    tf: Transform,
    parent: Entity,
    scale: f32,
    shadow: bool,
    lods: &mut ModelLods,
    meshes: &Assets<Mesh>,
    k: f32,
) {
    let id = mesh.id();
    if matches!(lods.levels.get(&id), Some(None)) {
        return;
    }
    let Some(m) = meshes.get(mesh) else { return };
    let Some(aabb) = m.compute_aabb() else { return };
    let placed = Placed {
        e,
        parent,
        tf,
        material: material.clone(),
        r: Vec3::from(aabb.half_extents).length() * scale,
        shadow,
    };
    if let Some(Some(levels)) = lods.levels.get(&id) {
        place(commands, placed, levels.clone(), k);
        return;
    }
    if let Some((_, waiting)) = lods.making.get_mut(&id) {
        waiting.push(placed);
        return;
    }
    // (A model unloaded between maps comes back as new meshes: the levels of the old ones go.)
    lods.levels.retain(|id, _| meshes.contains(*id));
    if m.try_indices_option()
        .ok()
        .flatten()
        .is_none_or(|i| i.len() < MIN_TRIS * 3)
    {
        lods.levels.insert(id, None);
        return;
    }
    let m = m.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move { simplify(&m) });
    lods.making.insert(id, (task, vec![placed]));
}

/// Levels made in the background go to the placements waiting for them.
pub fn finish_levels(
    mut commands: Commands,
    mut lods: ResMut<ModelLods>,
    mut meshes: ResMut<Assets<Mesh>>,
    display: Res<crate::settings::Display>,
    quality: Option<Res<super::quality::Quality>>,
) {
    if lods.making.is_empty() {
        return;
    }
    let k = lod_k(display.fov, quality.map(|q| q.preset));
    let ModelLods { levels, making } = &mut *lods;
    making.retain(|id, (task, waiting)| {
        let Some(made) = check_ready(task) else { return true };
        let made = made.map(|ms| ms.into_iter().map(|m| meshes.add(m)).collect::<Vec<_>>());
        if let Some(hs) = &made {
            for p in waiting.drain(..) {
                place(&mut commands, p, hs.clone(), k);
            }
        }
        levels.insert(*id, made);
        false
    });
}

/// The full mesh shows over the nearest bands, its levels as its siblings further out (unless the map that
/// placed it is gone by then).
fn place(commands: &mut Commands, p: Placed, levels: Vec<Handle<Mesh>>, k: f32) {
    commands.entity(p.e).queue_silenced(move |mut full: EntityWorldMut| {
        let first = LodBand {
            r: p.r,
            first: LEVELS[0].0,
            last: LEVELS[0].1,
        };
        full.insert((first, first.range(k)));
        full.world_scope(|w| {
            if w.get_entity(p.parent).is_err() {
                return;
            }
            for (h, &(a, b, _)) in levels.into_iter().zip(&LEVELS[1..]) {
                let band = LodBand {
                    r: p.r,
                    first: a,
                    last: b,
                };
                let mut level = w.spawn((
                    Mesh3d(h),
                    MeshMaterial3d(p.material.clone()),
                    p.tf,
                    Visibility::default(),
                    band,
                    band.range(k),
                    ChildOf(p.parent),
                ));
                if !p.shadow {
                    level.insert(NotShadowCaster);
                }
            }
        });
    });
}
