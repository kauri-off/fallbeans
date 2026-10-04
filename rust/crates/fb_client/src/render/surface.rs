//! Surfaces (port of `materials.ts`): every map material gets fine detail from a procedural, tileable
//! texture per kind of surface (normal from a height map in RG, the height in B, a roughness mask in A),
//! mapped triplanar in the object's own space (the models have no UVs), so detail keeps its size on any
//! primitive and stays glued to moving parts. Palette materials also carry the look's pattern
//! (stripes, checker, dots, chevron, waves) in its two tones.
use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat,
};
use bevy::shader::ShaderRef;
use fb_sim::looks::Pattern;

pub type SurfaceMaterial = ExtendedMaterial<StandardMaterial, Surface>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Plastic,
    Padded,
    Rubber,
    Metal,
    Fabric,
    Ice,
    Cloud,
    Gold,
    Wood,
    Glossy,
    Tile,
    Leaf,
    Grass,
    Rock,
    Cloth,
    Glass,
    Carpet,
}

impl Kind {
    pub fn of(name: &str) -> Option<Kind> {
        Some(match name {
            "plastic" => Kind::Plastic,
            "padded" => Kind::Padded,
            "rubber" => Kind::Rubber,
            "metal" => Kind::Metal,
            "fabric" => Kind::Fabric,
            "ice" => Kind::Ice,
            "cloud" => Kind::Cloud,
            "gold" => Kind::Gold,
            "wood" => Kind::Wood,
            "glossy" => Kind::Glossy,
            "tile" => Kind::Tile,
            "leaf" => Kind::Leaf,
            "grass" => Kind::Grass,
            "rock" => Kind::Rock,
            "cloth" => Kind::Cloth,
            "glass" => Kind::Glass,
            "carpet" => Kind::Carpet,
            _ => return None,
        })
    }

    /// Surface of a model's material, by the material's name in the asset pack.
    pub fn of_model(material: &str) -> Option<Kind> {
        Some(match material {
            // The beans are smooth and solid, shoes included.
            "Body" | "Belly" | "Blush" | "Shoe" => return None,
            "Bumper" | "Glove" => Kind::Rubber,
            "Metal" => Kind::Metal,
            "Gold" => Kind::Gold,
            "Gem" | "Eye" | "Sclera" | "Glint" | "Visor" | "Star" => Kind::Glossy,
            "Cloud" => Kind::Cloud,
            "Door" | "Trunk" => Kind::Wood,
            "Top" | "Side" => Kind::Tile,
            "Leaves" | "Pine" => Kind::Leaf,
            "Grass" => Kind::Grass,
            "Rock" => Kind::Rock,
            "Flag" => Kind::Cloth,
            _ => Kind::Plastic,
        })
    }
}

/// How a kind of surface looks.
struct Def {
    /// Texture tiles per metre.
    scale: f32,
    /// Normal perturbation strength.
    normal: f32,
    /// Roughness modulation (± fraction) by the mask.
    rough_var: f32,
    /// Darkening of the height map's low points.
    cavity: f32,
    roughness: Option<f32>,
    metalness: Option<f32>,
    /// Whitening where the roughness mask is high (frost on ice).
    frost: f32,
}

fn def(k: Kind) -> Def {
    let d = |scale, normal, rough_var, cavity| Def {
        scale,
        normal,
        rough_var,
        cavity,
        roughness: None,
        metalness: None,
        frost: 0.0,
    };
    match k {
        Kind::Plastic => d(0.5, 0.2, 0.25, 0.03),
        Kind::Padded => Def {
            roughness: Some(0.62),
            ..d(0.25, 0.9, 0.25, 0.14)
        },
        Kind::Rubber => Def {
            roughness: Some(0.72),
            ..d(1.2, 0.6, 0.2, 0.1)
        },
        Kind::Metal => Def {
            roughness: Some(0.4),
            metalness: Some(0.6),
            ..d(0.8, 0.3, 0.45, 0.04)
        },
        Kind::Fabric => d(2.2, 0.55, 0.18, 0.12),
        Kind::Ice => Def {
            roughness: Some(0.35),
            frost: 0.35,
            ..d(0.3, 0.7, 0.8, 0.14)
        },
        Kind::Cloud => Def {
            roughness: Some(1.0),
            ..d(0.35, 1.0, 0.0, 0.18)
        },
        Kind::Gold => Def {
            roughness: Some(0.3),
            metalness: Some(1.0),
            ..d(1.0, 0.04, 0.15, 0.0)
        },
        Kind::Wood => Def {
            roughness: Some(0.55),
            ..d(0.35, 0.18, 0.35, 0.1)
        },
        Kind::Glossy => d(1.0, 0.08, 0.5, 0.0),
        Kind::Tile => Def {
            roughness: Some(0.38),
            ..d(0.7, 0.3, 0.3, 0.08)
        },
        Kind::Leaf => Def {
            roughness: Some(0.62),
            ..d(1.3, 0.7, 0.3, 0.2)
        },
        Kind::Grass => Def {
            roughness: Some(0.85),
            ..d(1.6, 0.55, 0.25, 0.16)
        },
        Kind::Rock => Def {
            roughness: Some(0.9),
            ..d(0.45, 0.9, 0.2, 0.22)
        },
        Kind::Cloth => Def {
            roughness: Some(0.8),
            ..d(3.0, 0.3, 0.15, 0.08)
        },
        Kind::Glass => Def {
            roughness: Some(0.06),
            ..d(0.5, 0.06, 1.0, 0.0)
        },
        Kind::Carpet => Def {
            roughness: Some(0.95),
            ..d(2.4, 0.45, 0.2, 0.14)
        },
    }
}

// ---------------------------------------------------------------- the detail textures (as TS)

fn hash(x: i64, y: i64, seed: i64) -> f32 {
    let h = (x * 374_761_393 + y * 668_265_263 + seed * 2_147_483_647) as i32;
    let h = (h ^ ((h as u32) >> 13) as i32).wrapping_mul(1_274_126_177);
    ((h ^ ((h as u32) >> 16) as i32) as u32) as f32 / 4_294_967_296.0
}

fn wrap(i: i64, period: i64) -> i64 {
    ((i % period) + period) % period
}

/// Periodic value noise: `period` cells across the unit square.
fn vnoise(u: f32, v: f32, period: i64, seed: i64) -> f32 {
    let x = u * period as f32;
    let y = v * period as f32;
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let (xi, yi) = (xi as i64, yi as i64);
    let w = |i| wrap(i, period);
    let a = hash(w(xi), w(yi), seed);
    let b = hash(w(xi + 1), w(yi), seed);
    let c = hash(w(xi), w(yi + 1), seed);
    let d = hash(w(xi + 1), w(yi + 1), seed);
    a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy
}

fn fbm(u: f32, v: f32, period: i64, octaves: u32, seed: i64) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 0.5, 0.0);
    for o in 0..octaves {
        sum += vnoise(u, v, period << o, seed + o as i64 * 17) * amp;
        norm += amp;
        amp *= 0.5;
    }
    sum / norm
}

/// Periodic cellular noise: distances to the nearest two feature points (in cell units).
fn cells(u: f32, v: f32, period: i64, seed: i64) -> (f32, f32) {
    let x = u * period as f32;
    let y = v * period as f32;
    let (xi, yi) = (x.floor() as i64, y.floor() as i64);
    let (mut f1, mut f2) = (9.0f32, 9.0f32);
    for j in -1..=1 {
        for i in -1..=1 {
            let (cx, cy) = (xi + i, yi + j);
            let (wx, wy) = (wrap(cx, period), wrap(cy, period));
            let px = cx as f32 + hash(wx, wy, seed);
            let py = cy as f32 + hash(wx, wy, seed + 1);
            let d = (px - x).hypot(py - y);
            if d < f1 {
                f2 = f1;
                f1 = d;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2)
}

fn smooth(x: f32, a: f32, b: f32) -> f32 {
    if x <= a {
        return 0.0;
    }
    if x >= b {
        return 1.0;
    }
    let t = (x - a) / (b - a);
    t * t * (3.0 - 2.0 * t)
}

use core::f32::consts::PI;

/// Height and roughness mask of a kind at (u, v) in the unit square.
fn build(k: Kind, u: f32, v: f32) -> (f32, f32) {
    match k {
        Kind::Plastic => {
            let peel = fbm(u, v, 24, 2, 1);
            (0.55 + (peel - 0.5) * 0.3, 0.4 + fbm(u, v, 4, 3, 2) * 0.35)
        }
        Kind::Padded => {
            let n = 4.0;
            let (cu, cv) = ((u * n) % 1.0, (v * n) % 1.0);
            let pillow = ((PI * cu).sin() * (PI * cv).sin()).max(0.0).powf(0.35);
            let grain = fbm(u, v, 64, 2, 3);
            (
                pillow * 0.85 + grain * 0.15,
                0.4 + grain * 0.4 + fbm(u, v, 8, 2, 4) * 0.2,
            )
        }
        Kind::Rubber => {
            let (f1, _) = cells(u, v, 22, 5);
            let dot = 1.0 - smooth(f1, 0.18, 0.42);
            (
                0.35 + dot * 0.5 + fbm(u, v, 32, 2, 6) * 0.15,
                0.5 + fbm(u, v, 16, 2, 7) * 0.5,
            )
        }
        Kind::Metal => {
            let streak = vnoise(u * 0.25, v, 180, 8) * 0.6 + vnoise(u, v, 90, 9) * 0.4;
            (0.5 + (streak - 0.5) * 0.4, 0.3 + streak * 0.45)
        }
        Kind::Fabric => {
            let n = 28.0;
            let (x, y) = (u * n, v * n);
            let over = (x.floor() as i64 + y.floor() as i64) % 2 == 0;
            let t = if over {
                (PI * (y % 1.0)).sin()
            } else {
                (PI * (x % 1.0)).sin()
            };
            let fuzz = fbm(u, v, 64, 2, 11);
            (t.max(0.0).powf(0.6) * 0.8 + fuzz * 0.2, 0.55 + fuzz * 0.45)
        }
        Kind::Ice => {
            let (f1, f2) = cells(u, v, 6, 12);
            let crack = smooth(f2 - f1, 0.0, 0.05);
            let frost = smooth(fbm(u, v, 4, 4, 13), 0.5, 0.75);
            (
                0.5 + (fbm(u, v, 4, 3, 14) - 0.5) * 0.4 - (1.0 - crack) * 0.35,
                frost * 0.9 + (1.0 - crack) * 0.6,
            )
        }
        Kind::Cloud => (1.0 - (fbm(u, v, 4, 5, 15) * 2.0 - 1.0).abs(), 1.0),
        Kind::Gold => (0.5 + (fbm(u, v, 8, 3, 16) - 0.5) * 0.2, 0.4 + fbm(u, v, 6, 2, 17) * 0.3),
        Kind::Wood => {
            let warp = fbm(u, v, 4, 3, 18);
            let ring = 0.5 + 0.5 * ((u * 26.0 + warp * 6.0) * PI).sin();
            let fine = vnoise(u * 0.1, v, 120, 19);
            (ring * 0.7 + fine * 0.3, 0.4 + ring * 0.3 + fine * 0.3)
        }
        Kind::Glossy => (0.5 + (fbm(u, v, 16, 3, 20) - 0.5) * 0.2, fbm(u, v, 8, 3, 21)),
        Kind::Tile => {
            let speck = if hash((u * 180.0).floor() as i64, (v * 180.0).floor() as i64, 22) > 0.93 {
                1.0
            } else {
                0.0
            };
            (
                0.5 + (fbm(u, v, 6, 3, 23) - 0.5) * 0.4 - speck * 0.15,
                0.35 + fbm(u, v, 12, 2, 24) * 0.4 + speck * 0.25,
            )
        }
        Kind::Leaf => {
            let (f1, f2) = cells(u, v, 12, 31);
            let clump = 1.0 - smooth(f1, 0.1, 0.55);
            let vein = smooth(f2 - f1, 0.0, 0.06);
            (
                clump * 0.8 * vein + fbm(u, v, 32, 2, 32) * 0.2,
                0.4 + fbm(u, v, 16, 2, 33) * 0.6,
            )
        }
        Kind::Grass => {
            let blades = vnoise(u, v * 0.25, 160, 34) * 0.7 + vnoise(u, v, 64, 35) * 0.3;
            (blades * 0.75 + fbm(u, v, 6, 3, 36) * 0.25, 0.6 + blades * 0.4)
        }
        Kind::Rock => {
            let (f1, f2) = cells(u, v, 7, 37);
            let crack = smooth(f2 - f1, 0.0, 0.08);
            (
                (0.35 + f1 * 0.4) * crack + fbm(u, v, 16, 3, 38) * 0.25,
                0.7 + fbm(u, v, 8, 2, 39) * 0.3,
            )
        }
        Kind::Cloth => {
            let n = 64.0;
            let w = (PI * u * n).sin().abs() * 0.5 + (PI * v * n).sin().abs() * 0.5;
            (w * 0.85 + fbm(u, v, 32, 2, 40) * 0.15, 0.6 + fbm(u, v, 16, 2, 41) * 0.4)
        }
        Kind::Glass => (0.5, smooth(fbm(u, v, 4, 4, 43), 0.45, 0.8) * 0.6),
        Kind::Carpet => {
            let fuzz = fbm(u, v, 96, 2, 44);
            (fuzz * 0.8 + fbm(u, v, 8, 2, 45) * 0.2, 0.7 + fuzz * 0.3)
        }
    }
}

const SIZE: usize = 256;

/// The detail texture of a kind with its mip levels (RG normal, B height, A roughness mask).
pub fn detail_image(k: Kind) -> Image {
    let mut h = vec![0f32; SIZE * SIZE];
    let mut r = vec![0f32; SIZE * SIZE];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (hh, rr) = build(k, x as f32 / SIZE as f32, y as f32 / SIZE as f32);
            h[y * SIZE + x] = hh.clamp(0.0, 1.0);
            r[y * SIZE + x] = rr.clamp(0.0, 1.0);
        }
    }
    let at = |x: i64, y: i64| h[(wrap(y, SIZE as i64) as usize) * SIZE + wrap(x, SIZE as i64) as usize];
    // Height gradient over a texel, scaled so a full 0→1 rise across ~6 texels tilts about 45°.
    let k6 = 6.0;
    let mut level = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE as i64 {
        for x in 0..SIZE as i64 {
            let dx = (at(x + 1, y) - at(x - 1, y)) * 0.5 * k6;
            let dy = (at(x, y + 1) - at(x, y - 1)) * 0.5 * k6;
            let l = (dx * dx + dy * dy + 1.0).sqrt();
            let i = (y as usize * SIZE + x as usize) * 4;
            level[i] = ((-dx / l) * 127.5 + 127.5).round() as u8;
            level[i + 1] = ((-dy / l) * 127.5 + 127.5).round() as u8;
            level[i + 2] = (at(x, y) * 255.0).round() as u8;
            level[i + 3] = (r[y as usize * SIZE + x as usize] * 255.0).round() as u8;
        }
    }
    // Mip levels by averaging 2×2 texels (the tiles repeat: no edge handling needed).
    let mut data = level.clone();
    let mut size = SIZE;
    let mut levels = 1;
    while size > 1 {
        let n = size / 2;
        let mut next = vec![0u8; n * n * 4];
        for y in 0..n {
            for x in 0..n {
                for c in 0..4 {
                    let s = |xx: usize, yy: usize| level[(yy * size + xx) * 4 + c] as u32;
                    let v = s(2 * x, 2 * y) + s(2 * x + 1, 2 * y) + s(2 * x, 2 * y + 1) + s(2 * x + 1, 2 * y + 1);
                    next[(y * n + x) * 4 + c] = ((v + 2) / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        size = n;
        levels += 1;
    }
    let mut image = Image::new(
        Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; SIZE * SIZE * 4],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

// ---------------------------------------------------------------- the material

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct SurfaceUniform {
    pub c1: Vec4,
    pub c2: Vec4,
    /// Tiles per metre, normal strength, roughness variation, cavity.
    pub detail: Vec4,
    /// Pattern frequency, direction (x, z), speed.
    pub pattern: Vec4,
    /// Pattern kind (−1: none), frost.
    pub extra: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct Surface {
    #[uniform(100)]
    pub u: SurfaceUniform,
    #[texture(101)]
    #[sampler(102)]
    pub detail: Handle<Image>,
}

impl MaterialExtension for Surface {
    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/surface.wgsl".into()
    }

    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The fragment shader reads the mesh's transform (object-space mapping).
        let def = "VERTEX_OUTPUT_INSTANCE_INDEX";
        descriptor.vertex.shader_defs.push(def.into());
        if let Some(f) = &mut descriptor.fragment {
            f.shader_defs.push(def.into());
        }
        Ok(())
    }
}

fn pattern_id(p: Pattern) -> f32 {
    match p {
        Pattern::Stripes => 0.0,
        Pattern::Checker => 1.0,
        Pattern::Dots => 2.0,
        Pattern::Chevron => 3.0,
        Pattern::Waves => 4.0,
    }
}

/// A two-tone pattern on a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub c1: LinearRgba,
    pub c2: LinearRgba,
    pub freq: f32,
    pub dir: Vec2,
    pub speed: f32,
    pub kind: Pattern,
}

/// What a surface material is made of, as a cache key.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub color: LinearRgba,
    pub kind: Option<Kind>,
    pub paint: Option<Paint>,
    /// Keep this roughness instead of the kind's.
    pub roughness: Option<f32>,
    pub metallic: Option<f32>,
    pub alpha: AlphaMode,
    pub emissive: LinearRgba,
}

impl Spec {
    pub fn plain(color: LinearRgba, kind: Option<Kind>) -> Self {
        Self {
            color,
            kind,
            paint: None,
            roughness: None,
            metallic: None,
            alpha: AlphaMode::Opaque,
            emissive: LinearRgba::BLACK,
        }
    }

    fn key(&self) -> String {
        format!("{self:?}")
    }
}

/// The detail textures made so far, and the materials (shared: identical ones batch into one draw).
#[derive(Resource, Default)]
pub struct Surfaces {
    textures: HashMap<Kind, Handle<Image>>,
    materials: HashMap<String, Handle<SurfaceMaterial>>,
    /// A plain texture for materials without a surface (the shader then changes nothing).
    flat: Option<Handle<Image>>,
}

fn flat_image() -> Image {
    // Flat normal, mid height, mid roughness mask: no change.
    Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[128, 128, 255, 128],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

impl Surfaces {
    pub fn texture(&mut self, k: Kind, images: &mut Assets<Image>) -> Handle<Image> {
        self.textures
            .entry(k)
            .or_insert_with(|| images.add(detail_image(k)))
            .clone()
    }

    /// A material with a base standard material to start from (a model's own).
    pub fn material_from(
        &mut self,
        base: StandardMaterial,
        kind: Option<Kind>,
        paint: Option<Paint>,
        key: Option<String>,
        images: &mut Assets<Image>,
        materials: &mut Assets<SurfaceMaterial>,
    ) -> Handle<SurfaceMaterial> {
        if let Some(k) = &key
            && let Some(h) = self.materials.get(k)
        {
            return h.clone();
        }
        let mut base = base;
        let d = kind.map(def);
        let mut u = SurfaceUniform {
            extra: Vec4::new(-1.0, 0.0, 0.0, 0.0),
            ..default()
        };
        let detail = match kind {
            Some(k) => self.texture(k, images),
            None => self.flat.get_or_insert_with(|| images.add(flat_image())).clone(),
        };
        if let Some(d) = &d {
            u.detail = Vec4::new(d.scale, d.normal, d.rough_var, d.cavity);
            u.extra.y = d.frost;
            if let Some(m) = d.metalness {
                base.metallic = m;
            }
        }
        if let Some(p) = paint {
            u.c1 = p.c1.to_vec4();
            u.c2 = p.c2.to_vec4();
            let dir = p.dir.normalize_or(Vec2::X);
            u.pattern = Vec4::new(p.freq, dir.x, dir.y, p.speed);
            u.extra.x = pattern_id(p.kind);
        }
        let h = materials.add(ExtendedMaterial {
            base,
            extension: Surface { u, detail },
        });
        if let Some(k) = key {
            self.materials.insert(k, h.clone());
        }
        h
    }

    /// The shared material of a spec.
    pub fn material(
        &mut self,
        spec: &Spec,
        images: &mut Assets<Image>,
        materials: &mut Assets<SurfaceMaterial>,
    ) -> Handle<SurfaceMaterial> {
        let key = spec.key();
        if let Some(h) = self.materials.get(&key) {
            return h.clone();
        }
        let d = spec.kind.map(def);
        let base = StandardMaterial {
            base_color: if spec.paint.is_some() {
                Color::WHITE
            } else {
                spec.color.into()
            },
            perceptual_roughness: spec.roughness.or(d.as_ref().and_then(|d| d.roughness)).unwrap_or(0.5),
            metallic: spec.metallic.unwrap_or(0.0),
            alpha_mode: spec.alpha,
            emissive: spec.emissive,
            ..default()
        };
        self.material_from(base, spec.kind, spec.paint, Some(key), images, materials)
    }
}

pub struct SurfacePlugin;

impl Plugin for SurfacePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SurfaceMaterial>::default());
        app.init_resource::<Surfaces>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_ts() {
        // Values of `hash` in materials.ts.
        for (x, y, seed, want) in [
            (1, 2, 3, 0.5767206135205925),
            (255, 0, 22, 0.08044512826018035),
            (1023, 1023, 45, 0.855865815654397),
            (5, 7, 0, 0.9253859769087285),
        ] {
            assert!((hash(x, y, seed) as f64 - want).abs() < 1e-6, "{x} {y} {seed}");
        }
    }

    #[test]
    fn detail_has_every_mip() {
        let img = detail_image(Kind::Rubber);
        assert_eq!(img.texture_descriptor.mip_level_count, 9);
        let total: usize = (0..9).map(|l| (256usize >> l).pow(2) * 4).sum();
        assert_eq!(img.data.as_ref().unwrap().len(), total);
    }
}
