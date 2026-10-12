//! What Bevy runs every frame for nothing here: the render world's systems of features the game never uses, and
//! light clustering (no point or spot lights).
use bevy::core_pipeline::schedule::{Core2d, Core3d};
use bevy::ecs::intern::Interned;
use bevy::ecs::schedule::{ScheduleCleanupPolicy, ScheduleLabel};
use bevy::ecs::system::ScheduleSystem;
use bevy::light::cluster::GlobalClusterSettings;
use bevy::prelude::*;
use bevy::render::{ExtractSchedule, Render, RenderApp};

/// Parts of the names of the systems taken out: atmosphere, screen-space reflections, volumetric fog, transmission,
/// order-independent transparency, deferred shading, depth of field, motion blur, forward decals (clustered ones
/// stay: they make a buffer of the mesh view's bindings), contact shadows, generated environment maps, scattering
/// media, SSAO and TAA (`quality.rs` adds neither), Bevy's bloom (`bloom.rs` instead), occlusion culling's depth
/// pyramid, and 2D.
const UNUSED: &[&str] = &[
    "bevy_pbr::atmosphere::",
    "GpuAtmosphere",
    "bevy_pbr::ssr::",
    "bevy_pbr::volumetric_fog::",
    // (Not its phases' extraction: a camera without a transmissive phase gets no meshes queued, and one activated
    // after the strip, the look's stage, would never get it.)
    "bevy_pbr::transmission::node::",
    "bevy_pbr::transmission::texture::",
    "Transmissive3d",
    "bevy_core_pipeline::oit::",
    "Deferred",
    "bevy_pbr::deferred::",
    "bevy_core_pipeline::deferred::",
    "bevy_post_process::dof::",
    "DepthOfField",
    "bevy_post_process::motion_blur::",
    "MotionBlur",
    "ForwardDecal",
    "bevy_pbr::contact_shadows::",
    "bevy_pbr::light_probe::generate::",
    "ScatteringMedium",
    "bevy_pbr::medium::",
    "bevy_pbr::ssao::",
    "bevy_anti_alias::taa::",
    "bevy_post_process::bloom::",
    "BloomUniforms",
    "mip_generation::experimental::depth::",
    "core_2d::",
    "mesh2d",
    "Material2d",
    "queue_sprites",
    "prepare_sprite_",
    "tilemap",
    "text2d",
];

pub struct StripPlugin;

impl Plugin for StripPlugin {
    fn build(&self, app: &mut App) {
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            // (The render world's `First` runs before its `Render`, which is then not running.)
            r.add_systems(First, strip.run_if(run_once));
        }
        app.add_systems(PostStartup, cpu_clustering);
    }
}

fn strip(world: &mut World) {
    let labels: [Interned<dyn ScheduleLabel>; 4] = [
        Render.intern(),
        ExtractSchedule.intern(),
        Core3d.intern(),
        Core2d.intern(),
    ];
    let mut total = 0;
    for label in labels {
        let _ = world.try_schedule_scope(label, |world, schedule| {
            // (A schedule that has run holds its systems in its executable, one yet to run in its graph.)
            let unused = |s: &ScheduleSystem| {
                let name = s.name().to_string();
                UNUSED
                    .iter()
                    .any(|u| name.contains(u))
                    .then(|| s.default_system_sets().into_iter().next())
                    .flatten()
            };
            let mut sets: Vec<_> = schedule
                .graph()
                .systems
                .iter()
                .filter_map(|(_, s, _)| unused(s))
                .collect();
            if let Ok(run) = schedule.systems() {
                sets.extend(run.filter_map(|(_, s)| unused(s)));
            }
            // (What the extraction left for the render world, as `apply_extract_commands` would apply it.)
            schedule.apply_deferred(world);
            for set in sets {
                total += schedule
                    .remove_systems_in_set(set, world, ScheduleCleanupPolicy::RemoveSystemsOnly)
                    .unwrap_or(0);
            }
        });
    }
    info!("render: {total} systems of unused features taken out");
}

/// The cameras cluster nothing (`ClusterConfig::None`); GPU clustering would still run its passes, and Bevy 0.19's
/// makes a zero-sized texture for a view without clusters.
fn cpu_clustering(settings: Option<ResMut<GlobalClusterSettings>>) {
    if let Some(mut s) = settings {
        s.gpu_clustering = None;
    }
}
