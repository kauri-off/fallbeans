//! Surfaces: every map material gets fine detail from a procedural, tileable
//! texture per kind of surface (normal from a height map in RG, the height in B, a roughness mask in A),
//! mapped triplanar in the object's own space (the models have no UVs), so detail keeps its size on any
//! primitive and stays glued to moving parts. Palette materials also carry the look's pattern
//! (stripes, checker, dots, chevron, waves) in its two tones.
use std::collections::HashMap;
use std::sync::Arc;

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
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on};
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
    const ALL: [Kind; 17] = [
        Kind::Plastic,
        Kind::Padded,
        Kind::Rubber,
        Kind::Metal,
        Kind::Fabric,
        Kind::Ice,
        Kind::Cloud,
        Kind::Gold,
        Kind::Wood,
        Kind::Glossy,
        Kind::Tile,
        Kind::Leaf,
        Kind::Grass,
        Kind::Rock,
        Kind::Cloth,
        Kind::Glass,
        Kind::Carpet,
    ];

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

// ---------------------------------------------------------------- the detail textures

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
    let mut image = Image::new(
        Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    // (The tiles repeat: the mips need no edge handling.)
    add_mips(&mut image);
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

/// Trilinear sampling with anisotropic filtering (`anisotropy`× at most; wgpu wants every filter linear for
/// it): pictures made on the CPU that are seen at an angle (portals, emoji boards, mouths).
pub fn filtered(image: &mut Image, anisotropy: u16) {
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: anisotropy,
        ..default()
    });
}

/// The models' textures (their baked AO) come from the glTF without mips: Bevy makes none for PNG or JPEG,
/// and crevices and seams sparkle at a distance. The chain is made here once each is loaded; `AO_MARGIN`
/// (`blender/export.py`) pads the UV islands by 4 px, so the mips are used down to the 8th of the size only
/// (`lod_max_clamp`), past which the islands would bleed into each other.
fn mip_model_textures(
    mut events: MessageReader<AssetEvent<Image>>,
    server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    for e in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id }) = e else {
            continue;
        };
        let from_model = server
            .get_path(*id)
            .is_some_and(|p| p.path().extension().is_some_and(|x| x == "glb"));
        if !from_model {
            continue;
        }
        let Some(mut image) = images.get_mut(*id) else { continue };
        let format = image.texture_descriptor.format;
        if image.texture_descriptor.mip_level_count != 1
            || !matches!(format, TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb)
        {
            continue;
        }
        add_mips(&mut image);
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            lod_max_clamp: 3.0,
            anisotropy_clamp: 4,
            ..default()
        });
    }
}

/// Appends the mip chain to a square, power-of-two RGBA8 image made on the CPU and sets its level count:
/// 2×2 averages. sRGB images are averaged in linear light, weighted by alpha (no dark fringes where they turn
/// transparent); the rest channel by channel, as plain data (the detail textures' normals and masks).
pub fn add_mips(image: &mut Image) {
    let size = image.width() as usize;
    let srgb = image.texture_descriptor.format.is_srgb();
    if !size.is_power_of_two() || image.height() as usize != size || image.texture_descriptor.mip_level_count != 1 {
        return;
    }
    let Some(level0) = image.data.take() else { return };
    let (data, levels) = mip_chain(level0, size, srgb);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Level 0 followed by every smaller level down to 1×1, and how many levels that is.
fn mip_chain(level0: Vec<u8>, size: usize, srgb: bool) -> (Vec<u8>, u32) {
    // (Linear → sRGB through 4096 steps: finer than a byte of sRGB everywhere but in the deepest blacks.)
    const STEPS: usize = 4096;
    let (to_linear, to_srgb): (Vec<f32>, Vec<u8>) = if srgb {
        (
            (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect(),
            (0..STEPS)
                .map(|i| (linear_to_srgb(i as f32 / (STEPS - 1) as f32) * 255.0).round() as u8)
                .collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let mut data = level0.clone();
    let mut level = level0;
    let mut size = size;
    let mut levels = 1;
    while size > 1 {
        let n = size / 2;
        let mut next = Vec::with_capacity(n * n * 4);
        for y in 0..n {
            for x in 0..n {
                let quad = [
                    (2 * y * size + 2 * x) * 4,
                    (2 * y * size + 2 * x + 1) * 4,
                    ((2 * y + 1) * size + 2 * x) * 4,
                    ((2 * y + 1) * size + 2 * x + 1) * 4,
                ];
                let sum = |c: usize| quad.iter().map(|&i| level[i + c] as u32).sum::<u32>();
                if srgb {
                    let alpha = quad.map(|i| level[i + 3] as f32);
                    let total: f32 = alpha.iter().sum();
                    for c in 0..3 {
                        let lin = quad.iter().zip(alpha).fold(0.0, |acc, (&i, a)| {
                            acc + to_linear[level[i + c] as usize] * if total > 0.0 { a / total } else { 0.25 }
                        });
                        next.push(to_srgb[(lin.clamp(0.0, 1.0) * (STEPS - 1) as f32).round() as usize]);
                    }
                    next.push(((sum(3) + 2) / 4) as u8);
                } else {
                    for c in 0..4 {
                        next.push(((sum(c) + 2) / 4) as u8);
                    }
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        size = n;
        levels += 1;
    }
    (data, levels)
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
    /// Pattern kind (−1: none), frost, 1: no detail texture (`Surfaces::set_plain`).
    pub extra: Vec4,
}

/// Bindless where the standard material is (Vulkan; not DX12, whose 2048 samplers do not hold its six per slot
/// of a 2048-slot slab): an extended material is bindless only when both halves are, and only then do the map's
/// many materials share one bind group.
/// Bindless indices 50…52 (the standard material has 0…30) in their own index table at binding 100, the
/// data in an array at 101; without bindless, a uniform at 50 and the texture and sampler at 51 and 52.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
#[data(50, SurfaceUniform, binding_array(101))]
#[bindless(index_table(range(50..53), binding(100)))]
pub struct Surface {
    pub u: SurfaceUniform,
    #[texture(51)]
    #[sampler(52)]
    pub detail: Handle<Image>,
}

impl From<&Surface> for SurfaceUniform {
    fn from(s: &Surface) -> Self {
        s.u
    }
}

impl MaterialExtension for Surface {
    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/surface.wgsl".into()
    }

    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The fragment shader reads the mesh's transform (object-space mapping).
        let def = "VERTEX_OUTPUT_INSTANCE_INDEX";
        descriptor.vertex.shader_defs.push(def.into());
        // Merged static geometry has its pieces' frames in its vertices instead (`meshes::merge`).
        let framed = [
            super::meshes::ATTRIBUTE_FRAME,
            Mesh::ATTRIBUTE_COLOR,
            Mesh::ATTRIBUTE_UV_0,
            Mesh::ATTRIBUTE_UV_1,
        ]
        .into_iter()
        .all(|a| layout.0.contains(a));
        if let Some(f) = &mut descriptor.fragment {
            f.shader_defs.push(def.into());
            if framed {
                f.shader_defs.push("OBJECT_FRAME".into());
            }
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

/// What a surface material is made of (`Surfaces::material` shares one material per spec).
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

    /// The spec as a cache key: its numbers bit for bit, every field in a fixed place.
    fn key(&self) -> SpecKey {
        let mut bits = [0u32; SPEC_KEY_LEN];
        let mut n = 0;
        let mut put = |v: u32| {
            bits[n] = v;
            n += 1;
        };
        let rgba = |c: LinearRgba| [c.red, c.green, c.blue, c.alpha];
        // Colour and kind: 4 + 1.
        for v in rgba(self.color) {
            put(v.to_bits());
        }
        put(self.kind.map_or(0, |k| k as u32 + 1));
        // The paint: 1 + 13.
        match self.paint {
            Some(p) => {
                put(1);
                for v in rgba(p.c1).into_iter().chain(rgba(p.c2)) {
                    put(v.to_bits());
                }
                for v in [p.freq, p.dir.x, p.dir.y, p.speed, pattern_id(p.kind)] {
                    put(v.to_bits());
                }
            }
            None => {
                for _ in 0..14 {
                    put(0);
                }
            }
        }
        // Roughness and metalness: 2 + 2.
        for v in [self.roughness, self.metallic] {
            put(u32::from(v.is_some()));
            put(v.map_or(0, f32::to_bits));
        }
        // The alpha mode: 2.
        let (mode, cutoff) = match self.alpha {
            AlphaMode::Opaque => (0, 0.0),
            AlphaMode::Mask(c) => (1, c),
            AlphaMode::Blend => (2, 0.0),
            AlphaMode::Premultiplied => (3, 0.0),
            AlphaMode::AlphaToCoverage => (4, 0.0),
            AlphaMode::Add => (5, 0.0),
            AlphaMode::Multiply => (6, 0.0),
        };
        put(mode);
        put(f32::to_bits(cutoff));
        // Emission: 4.
        for v in rgba(self.emissive) {
            put(v.to_bits());
        }
        debug_assert_eq!(n, SPEC_KEY_LEN);
        SpecKey(bits)
    }
}

/// A `Spec` as a cache key (`Spec::key`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpecKey([u32; SPEC_KEY_LEN]);

const SPEC_KEY_LEN: usize = 29;

/// The detail textures made so far, and the materials (shared: identical ones batch into one draw).
#[derive(Resource, Default)]
pub struct Surfaces {
    textures: HashMap<Kind, Handle<Image>>,
    /// Detail textures being made off the main thread from the start (2–8 ms each).
    pending: HashMap<Kind, Task<Image>>,
    /// Materials lent the flat texture while their kind's is still being made: given it once it is in.
    waiting: Vec<(Kind, AssetId<SurfaceMaterial>)>,
    /// The shared materials by spec; the ones nothing else holds any more go (`prune`).
    materials: HashMap<SpecKey, Handle<SurfaceMaterial>>,
    /// A plain texture for materials without a surface (the shader then changes nothing).
    flat: Option<Handle<Image>>,
    /// No detail texture or its maths (the Low preset).
    plain: bool,
    /// Anisotropic filtering of the detail textures finished from now on (the preset's; 0: not set yet).
    pub anisotropy: u16,
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
    /// Detail textures still being made (the warm-up waits for them).
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Every surface material, made and to be made, with or without its detail texture.
    pub fn set_plain(&mut self, plain: bool, materials: &mut Assets<SurfaceMaterial>) {
        if self.plain == plain {
            return;
        }
        self.plain = plain;
        let z = if plain { 1.0 } else { 0.0 };
        // (Only the ones that differ: a material touched is prepared again.)
        let stale: Vec<AssetId<SurfaceMaterial>> = materials
            .iter()
            .filter(|(_, m)| m.extension.u.extra.z != z)
            .map(|(id, _)| id)
            .collect();
        for id in stale {
            if let Some(mut m) = materials.get_mut(id) {
                m.extension.u.extra.z = z;
            }
        }
    }

    /// The detail texture of a kind if it is made; if not, it is started (if it was not) and None.
    fn ready_texture(&mut self, k: Kind) -> Option<Handle<Image>> {
        if let Some(h) = self.textures.get(&k) {
            return Some(h.clone());
        }
        self.pending
            .entry(k)
            .or_insert_with(|| AsyncComputeTaskPool::get().spawn(async move { detail_image(k) }));
        None
    }

    fn flat(&mut self, images: &mut Assets<Image>) -> Handle<Image> {
        self.flat.get_or_insert_with(|| images.add(flat_image())).clone()
    }

    /// A finished detail texture: in, and given to the materials that wait for it.
    fn finish(&mut self, k: Kind, images: &mut Assets<Image>, materials: &mut Assets<SurfaceMaterial>) {
        let Some(task) = self.pending.remove(&k) else { return };
        // (Finished: no wait.)
        let mut image = block_on(task);
        if let ImageSampler::Descriptor(d) = &mut image.sampler {
            // (Set when it is made: the image lives in the render world only, it cannot be changed later.)
            d.anisotropy_clamp = if self.anisotropy == 0 { 8 } else { self.anisotropy };
        }
        let h = images.add(image);
        self.waiting.retain(|(wk, id)| {
            if *wk != k {
                return true;
            }
            if let Some(mut m) = materials.get_mut(*id) {
                m.extension.detail = h.clone();
            }
            false
        });
        self.textures.insert(k, h);
    }

    /// Forgets the shared materials only this cache still holds (their maps are gone).
    fn prune(&mut self) {
        self.materials.retain(|_, h| match h {
            Handle::Strong(a) => Arc::strong_count(a) > 1,
            Handle::Uuid(..) => true,
        });
    }

    /// A material with a base standard material to start from (a model's own); the kind's roughness
    /// replaces the base's unless `keep_roughness`.
    #[allow(clippy::too_many_arguments)]
    pub fn material_from(
        &mut self,
        base: StandardMaterial,
        kind: Option<Kind>,
        paint: Option<Paint>,
        keep_roughness: bool,
        key: Option<SpecKey>,
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
            extra: Vec4::new(-1.0, 0.0, if self.plain { 1.0 } else { 0.0 }, 0.0),
            ..default()
        };
        // (A detail texture still being made: the flat one until it is in, rather than waiting for it here.)
        let (detail, waits) = match kind.map(|k| (k, self.ready_texture(k))) {
            Some((_, Some(h))) => (h, None),
            Some((k, None)) => (self.flat(images), Some(k)),
            None => (self.flat(images), None),
        };
        if let Some(d) = &d {
            u.detail = Vec4::new(d.scale, d.normal, d.rough_var, d.cavity);
            u.extra.y = d.frost;
            if let Some(r) = d.roughness
                && !keep_roughness
            {
                base.perceptual_roughness = r;
            }
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
        if let Some(k) = waits {
            self.waiting.push((k, h.id()));
        }
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
        self.material_from(base, spec.kind, spec.paint, true, Some(key), images, materials)
    }
}

fn make_textures(mut surfaces: ResMut<Surfaces>) {
    for k in Kind::ALL {
        surfaces.ready_texture(k);
    }
}

fn take_textures(
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<SurfaceMaterial>>,
) {
    // (Not before the preset has set the filtering: a texture keeps the one it was finished with.)
    if surfaces.pending.is_empty() || surfaces.anisotropy == 0 {
        return;
    }
    let ready: Vec<Kind> = surfaces
        .pending
        .iter()
        .filter_map(|(k, task)| task.is_finished().then_some(*k))
        .collect();
    for k in ready {
        surfaces.finish(k, &mut images, &mut materials);
    }
}

/// Every few seconds: the cache lets go of the materials no map uses any more.
fn prune_materials(mut surfaces: ResMut<Surfaces>, time: Res<Time<Real>>, mut next: Local<f32>) {
    let now = time.elapsed_secs();
    if now < *next {
        return;
    }
    *next = now + 5.0;
    surfaces.prune();
}

pub struct SurfacePlugin;

impl Plugin for SurfacePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SurfaceMaterial>::default());
        app.init_resource::<Surfaces>();
        app.add_systems(Startup, make_textures);
        app.add_systems(Update, (take_textures, prune_materials, mip_model_textures));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable() {
        // Fixed values: the detail textures stay as they are.
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

    #[test]
    fn srgb_mips_average_in_linear_light_by_alpha() {
        // 2×2: opaque white, opaque black, two transparent reds.
        let level0 = vec![255, 255, 255, 255, 0, 0, 0, 255, 255, 0, 0, 0, 255, 0, 0, 0];
        let (data, levels) = mip_chain(level0.clone(), 2, true);
        assert_eq!(levels, 2);
        let top = &data[16..];
        // Half of white in linear light is sRGB 188; the transparent red does not tint it.
        assert!((187..=189).contains(&top[0]), "{top:?}");
        assert_eq!(top[0], top[1]);
        assert_eq!(top[1], top[2]);
        assert_eq!(top[3], 128);
        // As plain data: byte averages.
        let (data, _) = mip_chain(level0, 2, false);
        assert_eq!(&data[16..], &[191, 64, 64, 128]);
    }

    #[test]
    fn spec_keys_tell_specs_apart() {
        let a = Spec::plain(LinearRgba::WHITE, Some(Kind::Plastic));
        let b = Spec {
            alpha: AlphaMode::Blend,
            ..a.clone()
        };
        let c = Spec {
            paint: Some(Paint {
                c1: LinearRgba::WHITE,
                c2: LinearRgba::BLACK,
                freq: 1.0,
                dir: Vec2::X,
                speed: 0.0,
                kind: Pattern::Dots,
            }),
            ..a.clone()
        };
        assert_eq!(a.key(), a.clone().key());
        assert_ne!(a.key(), b.key());
        assert_ne!(a.key(), c.key());
        assert_ne!(a.key(), Spec::plain(LinearRgba::WHITE, None).key());
    }
}
