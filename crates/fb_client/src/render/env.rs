//! Ambient light without a light source (port of `environment.ts`): a small cube map of the look's sky
//! and ground colours as a vertical gradient (what a hemisphere light gave) plus a soft even fill (the
//! studio room TS added), used for image-based lighting. Made on the CPU: no compute shaders (T0).
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

const SIZE: u32 = 32;

/// IEEE half from f32 (finite, non-negative values: radiance).
fn f16(x: f32) -> u16 {
    let x = x.max(0.0);
    if x >= 65504.0 {
        return 0x7bff;
    }
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = bits & 0x7f_ffff;
    if exp <= 0 {
        if exp < -10 {
            return 0;
        }
        let m = (mant | 0x80_0000) >> (1 - exp);
        return ((m + 0x1000) >> 13) as u16;
    }
    // (Added, not or-ed: a mantissa rounded up to 0x400 carries into the exponent.)
    let h = ((exp as u32) << 10) + ((mant + 0x1000) >> 13);
    h.min(0x7bff) as u16
}

/// The direction of a texel's centre on a cube face (+X, −X, +Y, −Y, +Z, −Z).
fn dir(face: usize, x: u32, y: u32, size: u32) -> [f32; 3] {
    let u = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let v = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    let d = match face {
        0 => [1.0, -v, -u],
        1 => [-1.0, -v, u],
        2 => [u, 1.0, v],
        3 => [u, -1.0, -v],
        4 => [u, -v, 1.0],
        _ => [-u, -v, -1.0],
    };
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] / l, d[1] / l, d[2] / l]
}

/// The look's ambient light as a cube map (with mip levels, for rough reflections): radiance in units
/// of the sun's colour scale. `sky`/`ground` are linear colours of the hemisphere light, `env` the
/// strength of the even fill.
pub fn cube(sky: [f32; 3], ground: [f32; 3], intensity: f32, env: f32) -> Image {
    let k = intensity / core::f32::consts::PI;
    let sky = sky.map(|c| c * k);
    let ground = ground.map(|c| c * k);
    let fill = env * 0.5;
    let at = |d: [f32; 3]| -> [f32; 3] {
        // Cosine-weighted over a hemisphere a gradient flattens to 2/3 of its slope: 1.5× steeper.
        core::array::from_fn(|i| ((sky[i] + ground[i]) * 0.5 + (sky[i] - ground[i]) * 0.75 * d[1]).max(0.0) + fill)
    };
    let levels = SIZE.trailing_zeros() + 1;
    let mut data = Vec::new();
    for face in 0..6 {
        let mut size = SIZE;
        for _ in 0..levels {
            for y in 0..size {
                for x in 0..size {
                    let c = at(dir(face, x, y, size));
                    for v in [c[0], c[1], c[2], 1.0] {
                        data.extend_from_slice(&f16(v).to_le_bytes());
                    }
                }
            }
            size = (size / 2).max(1);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        vec![0; (SIZE * SIZE * 6 * 8) as usize],
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.data_order = bevy::render::render_resource::TextureDataOrder::LayerMajor;
    image.texture_descriptor.mip_level_count = levels;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..Default::default()
    });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves() {
        assert_eq!(f16(0.0), 0);
        assert_eq!(f16(1.0), 0x3c00);
        assert_eq!(f16(0.5), 0x3800);
        assert_eq!(f16(2.0), 0x4000);
        assert_eq!(f16(1e9), 0x7bff);
        // Rounds up across a power of two (odd and even exponents).
        assert_eq!(f16(1.9999), 0x4000);
        assert_eq!(f16(3.9999), 0x4400);
    }
}
