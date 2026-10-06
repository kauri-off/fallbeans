//! The scene's load: entities, meshes, triangles, lights, materials, textures, memory.
use bevy::camera::visibility::ViewVisibility;
use bevy::diagnostic::{DiagnosticPath, DiagnosticsStore, SystemInformationDiagnosticsPlugin};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::diagnostic::MeshAllocatorDiagnosticPlugin;
use bevy::render::renderer::RenderDevice;
use serde::Serialize;

use crate::render::surface::SurfaceMaterial;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Scene {
    pub entities: u32,
    pub meshes: u32,
    pub visible: u32,
    /// Of the visible ones, those that also go into the shadow map.
    pub casters: u32,
    pub tris: u64,
    pub tris_visible: u64,
    pub lights: u32,
    pub materials: u32,
    pub textures: u32,
    /// Textures' size on the GPU (mip levels included), MB.
    pub texture_mb: f32,
    /// Vertex and index buffers, MB.
    pub mesh_mb: f32,
    /// What wgpu allocated on the GPU (Vulkan, DX12), MB.
    pub gpu_mb: Option<f32>,
    pub ui_nodes: u32,
    pub ram_mb: Option<f32>,
    /// The process's CPU, % of one core.
    pub cpu: Option<f32>,
}

fn triangles(mesh: &Mesh) -> u64 {
    let n = match mesh.try_indices_option() {
        Ok(Some(Indices::U16(i))) => i.len(),
        Ok(Some(Indices::U32(i))) => i.len(),
        Ok(None) => mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).map_or(0, |a| a.len()),
        Err(_) => 0,
    } as u64;
    match mesh.primitive_topology() {
        PrimitiveTopology::TriangleList => n / 3,
        PrimitiveTopology::TriangleStrip => n.saturating_sub(2),
        _ => 0,
    }
}

fn image_bytes(image: &Image) -> u64 {
    let d = &image.texture_descriptor;
    let (bw, bh) = d.format.block_dimensions();
    let block = u64::from(d.format.block_copy_size(None).unwrap_or(4));
    let (mut w, mut h) = (d.size.width, d.size.height);
    let mut sum = 0;
    for _ in 0..d.mip_level_count.max(1) {
        sum += u64::from(w.div_ceil(bw)) * u64::from(h.div_ceil(bh)) * block;
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    sum * u64::from(d.size.depth_or_array_layers.max(1))
}

pub fn count(world: &mut World) -> Scene {
    const MB: f32 = 1024.0 * 1024.0;
    let mut s = Scene {
        entities: world.entities().count_spawned(),
        ..default()
    };
    let mut q = world.query::<(&Mesh3d, &ViewVisibility, Has<NotShadowCaster>)>();
    let meshes = world.resource::<Assets<Mesh>>();
    for (mesh, seen, no_shadow) in q.iter(world) {
        let tris = meshes.get(&mesh.0).map_or(0, triangles);
        s.meshes += 1;
        s.tris += tris;
        if seen.get() {
            s.visible += 1;
            s.tris_visible += tris;
            s.casters += u32::from(!no_shadow);
        }
    }
    let mut lights = world.query_filtered::<(), Or<(With<DirectionalLight>, With<PointLight>, With<SpotLight>)>>();
    s.lights = lights.iter(world).count() as u32;
    let mut nodes = world.query_filtered::<(), With<Node>>();
    s.ui_nodes = nodes.iter(world).count() as u32;
    s.materials = (world.resource::<Assets<StandardMaterial>>().len()
        + world.get_resource::<Assets<SurfaceMaterial>>().map_or(0, |a| a.len())) as u32;
    let images = world.resource::<Assets<Image>>();
    s.textures = images.len() as u32;
    s.texture_mb = images.iter().map(|(_, i)| image_bytes(i)).sum::<u64>() as f32 / MB;
    let store = world.resource::<DiagnosticsStore>();
    let value = |p: &DiagnosticPath| store.get(p).and_then(|d| d.value()).map(|v| v as f32);
    let (mem, cpu) = (
        SystemInformationDiagnosticsPlugin::PROCESS_MEM_USAGE,
        SystemInformationDiagnosticsPlugin::PROCESS_CPU_USAGE,
    );
    s.mesh_mb = value(MeshAllocatorDiagnosticPlugin::slabs_size_diagnostic_path()).unwrap_or(0.0) / MB;
    s.ram_mb = value(&mem).map(|gib| gib * 1024.0);
    s.cpu = value(&cpu);
    s.gpu_mb = world
        .get_resource::<RenderDevice>()
        .and_then(|d| d.wgpu_device().generate_allocator_report())
        .map(|r| r.total_allocated_bytes as f32 / MB);
    s
}

impl Scene {
    pub fn line(&self) -> String {
        let opt = |v: Option<f32>, unit: &str| v.map_or("—".into(), |v| format!("{v:.0} {unit}"));
        format!(
            "entities {} | meshes {} visible {} (shadow {}) | tris {} visible {}\nlights {} | materials {} | textures {} ({:.0} MB) | mesh buffers {:.0} MB | GPU alloc {} | UI nodes {}\nprocess: RAM {} | CPU {}",
            self.entities,
            self.meshes,
            self.visible,
            self.casters,
            big(self.tris),
            big(self.tris_visible),
            self.lights,
            self.materials,
            self.textures,
            self.texture_mb,
            self.mesh_mb,
            opt(self.gpu_mb, "MB"),
            self.ui_nodes,
            opt(self.ram_mb, "MB"),
            opt(self.cpu, "%"),
        )
    }
}

/// `1.2M`, `34k`.
pub fn big(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.2}M", n as f64 / 1e6),
    }
}
