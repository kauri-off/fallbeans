//! Haze around the camera on High: scattered sky and sun light, sunlight shafts, low mist, wisps with the wind.
//! 24 slices across the view (`fog.wgsl`), composited in one full-screen pass after the see-through things (over
//! them, as if drawn last among them) up to the depth of what is drawn; kept out of the reactive mask, where it
//! would shimmer.
use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::{MainPassResolutionOverride, Viewport};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::main_transparent_pass_3d;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::pbr::{
    MeshPipelineSystems, MeshPipelineViewLayoutKey, MeshPipelineViewLayouts, MeshViewBindGroup, ViewKeyCache,
};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{sampler, texture_3d, texture_depth_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, ViewDepthTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use fb_sim::looks::Look;

use super::quality::{Preset, Quality};
use crate::game::Map;
use crate::view::MainCamera;

/// The side of the noise's lattice texture (`fog.wgsl`): the wisps repeat every `LATTICE` / 0.045 m.
const LATTICE: i32 = 32;

/// `fog.wgsl`'s lattice hash, 0…1.
fn hash(x: i32, y: i32, z: i32) -> f32 {
    let h = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663) ^ (z as u32).wrapping_mul(83492791);
    let mut h = (h ^ (h >> 16)).wrapping_mul(2246822519);
    h ^= h >> 13;
    (h & 0xffffff) as f32 / 16777216.0
}

/// The lattice: hash(i) at texel i mod `LATTICE`, i from −`LATTICE` / 2 on (the same noise as the hash's around
/// the origin), x fastest.
fn lattice() -> Vec<u8> {
    let half = LATTICE / 2;
    let at = |t: i32| if t < half { t } else { t - LATTICE };
    let mut v = Vec::with_capacity((LATTICE * LATTICE * LATTICE) as usize);
    for z in 0..LATTICE {
        for y in 0..LATTICE {
            for x in 0..LATTICE {
                v.push((hash(at(x), at(y), at(z)) * 255.0).round() as u8);
            }
        }
    }
    v
}

/// On the main camera on High: its haze.
#[derive(Component, Clone, Copy, Default, ShaderType, ExtractComponent)]
pub struct Haze {
    /// rgb: the sky's light scattered in the haze (linear); a: its density at height 0 (1/m; 0: none).
    pub haze: Vec4,
    /// rgb: the tint of the sunlight scattered in it; a: how much of it.
    pub sun: Vec4,
    /// x: how fast it thins going up (1/m), y: how many times thicker it gets below at most, z: how much of the
    /// sunlight goes on forward (Henyey-Greenstein g: the glow towards the sun), w: drift (m/s).
    pub shape: Vec4,
}

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "fog.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<Haze>::default(),
            UniformComponentPlugin::<Haze>::default(),
        ));
        app.add_systems(Update, apply);
        let Some(r) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        r.init_resource::<SpecializedRenderPipelines<HazePipeline>>()
            .add_systems(RenderStartup, init_pipeline.after(MeshPipelineSystems))
            .add_systems(Render, specialize.in_set(RenderSystems::Prepare))
            .add_systems(
                Core3d,
                haze.after(main_transparent_pass_3d).in_set(Core3dSystems::MainPass),
            );
    }
}

/// The haze of a look: the colour its distance fog fades to, as thick as that fog is near (it begins
/// nearer in thicker looks), the sunlight tinted half way to it.
pub fn haze_of(l: &Look) -> Haze {
    let fog = super::linear(l.fog.color);
    let tint = LinearRgba::WHITE.mix(&fog, 0.5);
    let density = 0.25 / (l.fog.near as f32).max(30.0);
    Haze {
        haze: (fog.to_vec3() * 0.8).extend(density),
        sun: tint.to_vec3().extend(0.16),
        shape: Vec4::new(0.025, 3.0, 0.45, 1.2),
    }
}

/// The look's distance fog; without the haze (Low) it starts nearer, for the depth the haze would give.
fn falloff(l: &Look, haze: bool) -> FogFalloff {
    let (near, far) = (l.fog.near as f32, l.fog.far as f32);
    let (start, end) = if haze { (near, far) } else { (near * 0.55, far * 0.9) };
    FogFalloff::Linear { start, end }
}

/// On High only, in the round's look; while there is no map it stays on, as thin as nothing (its pipeline
/// compiles with the warm-up's maps all the same). The distance fog as the haze is there or not.
fn apply(
    map: Option<Res<Map>>,
    quality: Option<Res<Quality>>,
    mut camera: Query<(Entity, &mut Camera3d, &mut DistanceFog), With<MainCamera>>,
    mut commands: Commands,
    mut last: Local<Option<(Option<u32>, bool, Entity)>>,
) {
    let Ok((cam, mut c3d, mut fog)) = camera.single_mut() else {
        return;
    };
    let on = quality.is_some_and(|q| q.preset != Preset::Low);
    let now = (map.as_ref().map(|m| m.generation), on, cam);
    if *last == Some(now) {
        return;
    }
    *last = Some(now);
    // (The haze pass reads the depth of what is drawn.)
    let usages = TextureUsages::from(c3d.depth_texture_usages);
    if !usages.contains(TextureUsages::TEXTURE_BINDING) {
        c3d.depth_texture_usages = (usages | TextureUsages::TEXTURE_BINDING).into();
    }
    if let Some(map) = &map {
        fog.falloff = falloff(map.look.look, on);
    }
    if on {
        commands
            .entity(cam)
            .insert(map.as_ref().map_or_else(Haze::default, |m| haze_of(m.look.look)));
    } else {
        commands.entity(cam).remove::<Haze>();
    }
}

#[derive(Resource)]
struct HazePipeline {
    views: MeshPipelineViewLayouts,
    layout: BindGroupLayoutDescriptor,
    lattice: TextureView,
    sampler: Sampler,
    vertex: VertexState,
    shader: Handle<Shader>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct HazeKey {
    view: MeshPipelineViewLayoutKey,
    format: TextureFormat,
}

impl SpecializedRenderPipeline for HazePipeline {
    type Key = HazeKey;

    fn specialize(&self, key: HazeKey) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("haze".into()),
            layout: vec![self.views.get_view_layout(key.view).main_layout, self.layout.clone()],
            vertex: self.vertex.clone(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                targets: vec![Some(ColorTargetState {
                    format: key.format,
                    blend: Some(BlendState {
                        color: BlendState::PREMULTIPLIED_ALPHA_BLENDING.color,
                        alpha: super::reactive::KEEP,
                    }),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

fn init_pipeline(
    mut commands: Commands,
    views: Res<MeshPipelineViewLayouts>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "haze_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<Haze>(true),
                texture_depth_2d(),
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let side = LATTICE as u32;
    let texture = device.create_texture_with_data(
        &queue,
        &TextureDescriptor {
            label: Some("haze_lattice"),
            size: Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: side,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D3,
            format: TextureFormat::R8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &lattice(),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("haze_lattice_sampler"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(HazePipeline {
        views: views.clone(),
        layout,
        lattice: texture.create_view(&TextureViewDescriptor::default()),
        sampler,
        vertex: fullscreen.to_vertex_state(),
        shader: load_embedded_asset!(assets.as_ref(), "fog.wgsl"),
    });
}

#[derive(Component)]
struct HazePipelineId(CachedRenderPipelineId);

/// Each hazy view's pipeline, for its view bindings (as its meshes have them) and target.
fn specialize(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<HazePipeline>>,
    pipeline: Res<HazePipeline>,
    keys: Res<ViewKeyCache>,
    views: Query<(Entity, &ExtractedView), With<Haze>>,
) {
    for (e, view) in &views {
        let Some(key) = keys.get(&view.retained_view_entity) else {
            continue;
        };
        let key = HazeKey {
            view: (*key).into(),
            format: view.target_format,
        };
        let id = pipelines.specialize(&cache, &pipeline, key);
        commands.entity(e).insert(HazePipelineId(id));
    }
}

/// What the haze pass reads of the view.
type HazeView = (
    &'static ExtractedCamera,
    &'static ViewTarget,
    &'static ViewDepthTexture,
    &'static MeshViewBindGroup,
    &'static HazePipelineId,
    &'static DynamicUniformIndex<Haze>,
    Option<&'static MainPassResolutionOverride>,
);

/// The haze over the main pass's corner of the colour target.
fn haze(
    view: ViewQuery<HazeView, With<Haze>>,
    pipeline: Res<HazePipeline>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<Haze>>,
    mut ctx: RenderContext,
) {
    let (camera, target, depth, view_bind_group, id, index, size) = view.into_inner();
    let (Some(p), Some(u)) = (cache.get_render_pipeline(id.0), uniforms.uniforms().binding()) else {
        return;
    };
    let bind_group = ctx.render_device().create_bind_group(
        "haze_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((u, depth.view(), &pipeline.lattice, &pipeline.sampler)),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("haze"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let span = diagnostics.pass_span(&mut pass, "haze");
    if let Some(viewport) = Viewport::from_viewport_and_override(camera.viewport.as_ref(), size) {
        pass.set_camera_viewport(&viewport);
    }
    pass.set_render_pipeline(p);
    pass.set_bind_group(0, &view_bind_group.main, &view_bind_group.main_offsets);
    pass.set_bind_group(1, &bind_group, &[index.index()]);
    pass.draw(0..3, 0..1);
    span.end(&mut pass);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lattice_is_the_hash_around_the_origin() {
        let v = lattice();
        let n = LATTICE as usize;
        let texel = |x: i32, y: i32, z: i32| {
            let w = |i: i32| i.rem_euclid(LATTICE) as usize;
            f32::from(v[w(x) + n * (w(y) + n * w(z))]) / 255.0
        };
        for (x, y, z) in [(0, 0, 0), (-16, 5, 15), (7, -1, -9), (15, 15, -16)] {
            assert!((texel(x, y, z) - hash(x, y, z)).abs() <= 0.5 / 255.0, "{x} {y} {z}");
        }
        assert!(include_str!("fog.wgsl").contains(&format!("const LATTICE: i32 = {LATTICE};")));
    }

    #[test]
    fn every_look_has_some_haze() {
        for l in fb_sim::looks::LOOKS {
            let h = haze_of(l);
            assert!(h.haze.w > 0.0 && h.haze.w < 0.01, "{:?}: {}", l.id, h.haze.w);
            // A hundred metres of it at height 0 hide less than half of what is behind.
            assert!((-h.haze.w * 100.0).exp() > 0.5, "{:?}", l.id);
        }
    }
}
