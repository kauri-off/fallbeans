//! Haze around the camera on High: scattered sky and sun light, sunlight shafts, low mist, wisps with the wind.
//! 24 slices across the view (`fog.wgsl`), composited in one full-screen pass after the see-through things (over
//! them, as if drawn last among them) up to the depth of what is drawn; kept out of the reactive mask, where it
//! would shimmer. On an integrated GPU at half the resolution (12.9 → 2.9 ms on an Intel UHD 620), scaled up by
//! depth (`fog_up.wgsl`).
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
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_3d, texture_depth_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ExtractedView, ViewDepthTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use fb_sim::looks::Look;

use super::quality::{Preset, Quality, Tier};
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

/// On the main camera on an integrated GPU (or a software one): the haze at half the resolution.
#[derive(Component, Clone, Copy, ExtractComponent)]
pub struct HalfHaze;

/// The half-resolution haze's format.
const LOW_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "fog.wgsl");
        embedded_asset!(app, "fog_up.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<Haze>::default(),
            ExtractComponentPlugin::<HalfHaze>::default(),
            UniformComponentPlugin::<Haze>::default(),
        ));
        app.add_systems(Update, apply);
        let Some(r) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        r.init_resource::<SpecializedRenderPipelines<HazePipeline>>()
            .add_systems(RenderStartup, init_pipeline.after(MeshPipelineSystems))
            .add_systems(Render, specialize.in_set(RenderSystems::Prepare))
            .add_systems(Render, prepare_half.in_set(RenderSystems::PrepareResources))
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

/// What the haze was last made for: the map's generation, on, at half the resolution, the camera.
type Applied = (Option<u32>, bool, bool, Entity);

/// On High only, in the round's look (at half the resolution below a discrete GPU); while there is no map it stays
/// on, as thin as nothing (its pipeline compiles with the warm-up's maps all the same). The distance fog as the
/// haze is there or not.
fn apply(
    map: Option<Res<Map>>,
    quality: Option<Res<Quality>>,
    mut camera: Query<(Entity, &mut Camera3d, &mut DistanceFog), With<MainCamera>>,
    mut commands: Commands,
    mut last: Local<Option<Applied>>,
) {
    let Ok((cam, mut c3d, mut fog)) = camera.single_mut() else {
        return;
    };
    let on = quality.as_ref().is_some_and(|q| q.preset != Preset::Low);
    let half = quality.as_ref().is_some_and(|q| q.tier < Tier::T2);
    let now = (map.as_ref().map(|m| m.generation), on, half, cam);
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
    if half {
        commands.entity(cam).insert(HalfHaze);
    } else {
        commands.entity(cam).remove::<HalfHaze>();
    }
}

#[derive(Resource)]
struct HazePipeline {
    views: MeshPipelineViewLayouts,
    layout: BindGroupLayoutDescriptor,
    lattice: TextureView,
    sampler: Sampler,
    up_layout: BindGroupLayoutDescriptor,
    vertex: VertexState,
    shader: Handle<Shader>,
    up_shader: Handle<Shader>,
}

/// The pass a pipeline is for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    /// Over the view at its resolution.
    Full,
    /// At half of it, into the view's `HalfTarget`.
    Low,
    /// That scaled up over the view.
    Up,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct HazeKey {
    view: MeshPipelineViewLayoutKey,
    format: TextureFormat,
    kind: Kind,
}

impl SpecializedRenderPipeline for HazePipeline {
    type Key = HazeKey;

    fn specialize(&self, key: HazeKey) -> RenderPipelineDescriptor {
        let over = BlendState {
            color: BlendState::PREMULTIPLIED_ALPHA_BLENDING.color,
            alpha: super::reactive::KEEP,
        };
        let view = || vec![self.views.get_view_layout(key.view).main_layout, self.layout.clone()];
        let (layout, shader, entry, format, blend) = match key.kind {
            Kind::Full => (view(), &self.shader, "fragment", key.format, Some(over)),
            Kind::Low => (view(), &self.shader, "low", LOW_FORMAT, None),
            Kind::Up => (
                vec![self.up_layout.clone()],
                &self.up_shader,
                "fragment",
                key.format,
                Some(over),
            ),
        };
        RenderPipelineDescriptor {
            label: Some("haze".into()),
            layout,
            vertex: self.vertex.clone(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend,
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
    let up_layout = BindGroupLayoutDescriptor::new(
        "haze_up_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                texture_depth_2d(),
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
        up_layout,
        vertex: fullscreen.to_vertex_state(),
        shader: load_embedded_asset!(assets.as_ref(), "fog.wgsl"),
        up_shader: load_embedded_asset!(assets.as_ref(), "fog_up.wgsl"),
    });
}

/// A view's haze pipelines: the full one, or the half one and its scaling up.
#[derive(Component)]
enum HazePipelineId {
    Full(CachedRenderPipelineId),
    Half(CachedRenderPipelineId, CachedRenderPipelineId),
}

/// Each hazy view's pipelines, for its view bindings (as its meshes have them) and target.
fn specialize(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<HazePipeline>>,
    pipeline: Res<HazePipeline>,
    keys: Res<ViewKeyCache>,
    views: Query<(Entity, &ExtractedView, Has<HalfHaze>), With<Haze>>,
) {
    for (e, view, half) in &views {
        let Some(key) = keys.get(&view.retained_view_entity) else {
            continue;
        };
        let mut id = |kind| {
            let key = HazeKey {
                view: (*key).into(),
                format: view.target_format,
                kind,
            };
            pipelines.specialize(&cache, &pipeline, key)
        };
        let ids = if half {
            HazePipelineId::Half(id(Kind::Low), id(Kind::Up))
        } else {
            HazePipelineId::Full(id(Kind::Full))
        };
        commands.entity(e).insert(ids);
    }
}

type HalfView = (
    Entity,
    &'static ExtractedView,
    Option<&'static MainPassResolutionOverride>,
);

/// A half-resolution view's haze target, half its main pass (rounded up).
#[derive(Component)]
struct HalfTarget(CachedTexture);

fn prepare_half(
    mut commands: Commands,
    mut textures: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<HalfView, (With<Haze>, With<HalfHaze>)>,
    stale: Query<Entity, (With<HalfTarget>, Without<HalfHaze>)>,
) {
    for e in &stale {
        commands.entity(e).remove::<HalfTarget>();
    }
    for (e, view, size) in &views {
        let size = size.map_or(view.viewport.zw(), |s| s.0);
        let size = ((size + 1) / 2).max(UVec2::ONE);
        let texture = textures.get(
            &device,
            TextureDescriptor {
                label: Some("haze_half"),
                size: Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: LOW_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(e).insert(HalfTarget(texture));
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
    Option<&'static HalfTarget>,
);

/// The haze over the main pass's corner of the colour target (at half the resolution first, on a `HalfHaze` view).
fn haze(
    view: ViewQuery<HazeView, With<Haze>>,
    pipeline: Res<HazePipeline>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<Haze>>,
    mut ctx: RenderContext,
) {
    let (camera, target, depth, view_bind_group, ids, index, size, half) = view.into_inner();
    let Some(u) = uniforms.uniforms().binding() else {
        return;
    };
    let bind_group = ctx.render_device().create_bind_group(
        "haze_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((u, depth.view(), &pipeline.lattice, &pipeline.sampler)),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    // (The pass over the view: the haze, or its half-resolution one scaled up.)
    let (over, up_group) = match (ids, half) {
        (HazePipelineId::Full(full), _) => (*full, None),
        (HazePipelineId::Half(low, up), Some(half)) => {
            let Some(low) = cache.get_render_pipeline(*low) else {
                return;
            };
            let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("haze_half"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &half.0.default_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let span = diagnostics.pass_span(&mut pass, "haze_half");
            pass.set_render_pipeline(low);
            pass.set_bind_group(0, &view_bind_group.main, &view_bind_group.main_offsets);
            pass.set_bind_group(1, &bind_group, &[index.index()]);
            pass.draw(0..3, 0..1);
            span.end(&mut pass);
            drop(pass);
            let up_group = ctx.render_device().create_bind_group(
                "haze_up_bind_group",
                &cache.get_bind_group_layout(&pipeline.up_layout),
                &BindGroupEntries::sequential((&half.0.default_view, depth.view())),
            );
            (*up, Some(up_group))
        }
        (HazePipelineId::Half(..), None) => return,
    };
    let Some(over) = cache.get_render_pipeline(over) else {
        return;
    };
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
    pass.set_render_pipeline(over);
    match &up_group {
        Some(g) => pass.set_bind_group(0, g, &[]),
        None => {
            pass.set_bind_group(0, &view_bind_group.main, &view_bind_group.main_offsets);
            pass.set_bind_group(1, &bind_group, &[index.index()]);
        }
    }
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
