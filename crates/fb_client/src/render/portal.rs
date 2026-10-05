//! Portal pictures: the discs
//! carry a spiral (the way in) or rings flowing out (a one-way exit) in the portal's colour.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const SIZE: usize = 256;

/// A canvas radial gradient from radius 4 to the rim, with its stops.
fn gradient(r: f32, stops: [(f32, [f32; 3]); 3]) -> [f32; 3] {
    let f = ((r - 4.0) / (SIZE as f32 / 2.0 - 4.0)).clamp(0.0, 1.0);
    let i = if f < stops[1].0 { 0 } else { 1 };
    let (a, b) = (stops[i], stops[i + 1]);
    let k = ((f - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
    std::array::from_fn(|c| a.1[c] + (b.1[c] - a.1[c]) * k)
}

fn rgb(c: Color) -> [f32; 3] {
    let s = c.to_srgba();
    [s.red, s.green, s.blue]
}

const DEEP: [f32; 3] = [0x1a as f32 / 255.0, 0x0f as f32 / 255.0, 0x3d as f32 / 255.0];

/// The disc filled by `fill`, with white strokes over it (coverage per pixel from `stroke`, 0…1).
fn picture(fill: impl Fn(f32) -> [f32; 3], stroke: impl Fn(f32, f32) -> f32, alpha: f32) -> Image {
    let half = SIZE as f32 / 2.0;
    let mut data = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
            let r = px.hypot(py);
            let edge = (half - r + 0.5).clamp(0.0, 1.0);
            let s = stroke(px, py) * alpha;
            let c = fill(r).map(|v| v * (1.0 - s) + s);
            data.extend(c.map(|v| (v * 255.0).round() as u8));
            data.push((edge * 255.0).round() as u8);
        }
    }
    Image::new(
        Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn segment_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// White at the centre through the colour to deep violet, four white spiral arms.
pub fn swirl(color: Color) -> Image {
    let c = rgb(color);
    let arms: Vec<[Vec2; 61]> = (0..4)
        .map(|arm| {
            std::array::from_fn(|k| {
                let f = k as f32 / 60.0;
                let a = arm as f32 * core::f32::consts::FRAC_PI_2 + f * core::f32::consts::PI * 2.2;
                let r = f * SIZE as f32 * 0.48;
                Vec2::new(a.cos() * r, a.sin() * r)
            })
        })
        .collect();
    picture(
        |r| gradient(r, [(0.0, [1.0; 3]), (0.35, c), (1.0, DEEP)]),
        |x, y| {
            let p = Vec2::new(x, y);
            let d = arms
                .iter()
                .flat_map(|arm| arm.windows(2).map(|w| segment_dist(p, w[0], w[1])))
                .fold(f32::MAX, f32::min);
            (3.0 - d + 0.5).clamp(0.0, 1.0)
        },
        0.55,
    )
}

/// Deep violet at the centre through the colour to white at the rim, three white rings.
pub fn rings(color: Color) -> Image {
    let c = rgb(color);
    picture(
        |r| gradient(r, [(0.0, DEEP), (0.6, c), (1.0, [1.0; 3])]),
        |x, y| {
            let r = x.hypot(y);
            let d = [0.14, 0.28, 0.42]
                .map(|k| (r - k * SIZE as f32).abs())
                .into_iter()
                .fold(f32::MAX, f32::min);
            (3.5 - d + 0.5).clamp(0.0, 1.0)
        },
        0.6,
    )
}

/// The exit's arrow in the x/y plane, its tip at +y.
pub fn arrow() -> Mesh {
    let v: [[f32; 2]; 7] = [
        [0.0, 0.75],
        [-0.6, 0.0],
        [0.6, 0.0],
        [-0.22, 0.0],
        [0.22, 0.0],
        [-0.22, -0.6],
        [0.22, -0.6],
    ];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, v.map(|[x, y]| [x, y, 0.0]).to_vec())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 7])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, v.map(|[x, y]| [x + 0.5, 0.5 - y]).to_vec())
        .with_inserted_indices(Indices::U16(vec![0, 1, 2, 3, 5, 6, 3, 6, 4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fill_the_disc() {
        for img in [swirl(Color::srgb(0.6, 0.3, 1.0)), rings(Color::srgb(0.2, 0.8, 1.0))] {
            let d = img.data.unwrap();
            let at = |x: usize, y: usize| &d[(y * SIZE + x) * 4..][..4];
            assert_eq!(at(0, 0)[3], 0);
            assert_eq!(at(SIZE / 2, SIZE / 2)[3], 255);
        }
    }
}
