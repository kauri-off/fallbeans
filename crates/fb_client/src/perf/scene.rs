//! The scene's load: entities, meshes, triangles, lights, materials, textures, memory.
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bevy::camera::visibility::ViewVisibility;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice};
use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use wgpu_types::Backend;

use super::RenderShared;
use crate::render::surface::SurfaceMaterial;

const MB: f32 = 1024.0 * 1024.0;
/// Seconds between GPU allocator reports (DX12 only; the report walks every allocation).
const GPU_REPORT_S: f32 = 5.0;
/// A CPU sample older than this is no fair "since the last look" (the counts were not shown meanwhile).
const CPU_GAP: Duration = Duration::from_secs(2);

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
    /// What wgpu allocated on the GPU, MB: DX12 only (the other backends have no allocator report).
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

/// Process stats are read only while shown or recorded, not by Bevy's plugin, which polls every CPU at 5 Hz.
#[derive(Resource)]
pub struct Probe {
    sys: System,
    pid: Pid,
    /// When, and the process's CPU time then (ms).
    cpu: Option<(Instant, u64)>,
    /// When (real time, s), and what the report said.
    gpu: Option<(f32, Option<f32>)>,
}

impl Default for Probe {
    fn default() -> Probe {
        Probe {
            sys: System::new(),
            pid: Pid::from_u32(std::process::id()),
            cpu: None,
            gpu: None,
        }
    }
}

impl Probe {
    /// The process's resident memory (MB) and its CPU since the last look (% of one core; None the first time).
    fn process(&mut self) -> (Option<f32>, Option<f32>) {
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::Some(&[self.pid]), false, kind);
        let Some(p) = self.sys.process(self.pid) else {
            return (None, None);
        };
        let (now, ms) = (Instant::now(), p.accumulated_cpu_time());
        let cpu = self
            .cpu
            .replace((now, ms))
            .filter(|(at, _)| now.duration_since(*at) < CPU_GAP)
            .and_then(|(at, was)| {
                let wall = now.duration_since(at).as_secs_f64() * 1000.0;
                (wall > 0.0).then(|| (ms.saturating_sub(was) as f64 / wall * 100.0) as f32)
            });
        (Some(p.memory() as f32 / MB), cpu)
    }
}

/// The counts as of now (`now`: real time, s).
pub fn count(world: &mut World, now: f32) -> Scene {
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
    s.mesh_mb = world
        .get_resource::<RenderShared>()
        .map_or(0, |r| r.0.mesh_bytes.load(Ordering::Relaxed)) as f32
        / MB;
    let dx12 = world
        .get_resource::<RenderAdapterInfo>()
        .is_some_and(|i| i.0.backend == Backend::Dx12);
    let report_due = dx12
        && world
            .get_resource::<Probe>()
            .is_some_and(|p| p.gpu.is_none_or(|(at, _)| now - at >= GPU_REPORT_S));
    let report = report_due.then(|| {
        world
            .get_resource::<RenderDevice>()
            .and_then(|d| d.wgpu_device().generate_allocator_report())
            .map(|r| r.total_allocated_bytes as f32 / MB)
    });
    if let Some(mut probe) = world.get_resource_mut::<Probe>() {
        if let Some(mb) = report {
            probe.gpu = Some((now, mb));
        }
        (s.ram_mb, s.cpu) = probe.process();
        s.gpu_mb = probe.gpu.and_then(|(_, mb)| mb);
    }
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
