//! Boards with an emoji: the board's colour, a white frame and the
//! emoji drawn from the game's colour emoji font (its bitmaps, through swash as Bevy's text does).
use std::sync::{Mutex, PoisonError};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use swash::FontRef;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};

use crate::ui::EMOJI;

const SIZE: usize = 256;

/// Straight-alpha RGBA of the emoji `text` (its first glyph) at about `px` pixels, or None.
fn glyph(text: &str, px: f32) -> Option<(usize, usize, Vec<u8>)> {
    let font = FontRef::from_index(EMOJI, 0)?;
    let ch = text.chars().find(|c| *c != '\u{fe0f}')?;
    let id = font.charmap().map(ch);
    if id == 0 {
        return None;
    }
    let mut ctx = ScaleContext::new();
    let mut scaler = ctx.builder(font).size(px).build();
    let img = Render::new(&[
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::ColorOutline(0),
        Source::Outline,
    ])
    .render(&mut scaler, id)?;
    let (w, h) = (img.placement.width as usize, img.placement.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let data = match img.content {
        swash::scale::image::Content::Color => img.data,
        _ => img.data.iter().flat_map(|a| [40, 40, 40, *a]).collect(),
    };
    Some((w, h, data))
}

/// Boards drawn so far (≈350 KB each with mips), by text and colour: the bonus cards and signs every round
/// shows again (the warm-up's maps draw them all first). At most `KEPT`; the rest are drawn each time.
static BOARDS: Mutex<Vec<(String, [u8; 4], Image)>> = Mutex::new(Vec::new());
const KEPT: usize = 24;

/// A board: `bg` filled, a white frame, the emoji in the middle (62% of the board).
pub fn board(text: &str, bg: Color) -> Image {
    let key = bg.to_srgba().to_u8_array();
    let mut kept = BOARDS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((.., image)) = kept.iter().find(|(t, k, _)| t == text && *k == key) {
        return image.clone();
    }
    let image = draw(text, bg);
    if kept.len() < KEPT {
        kept.push((text.to_string(), key, image.clone()));
    }
    image
}

fn draw(text: &str, bg: Color) -> Image {
    let bg = bg.to_srgba();
    let mut px = vec![[bg.red, bg.green, bg.blue, 1.0f32]; SIZE * SIZE];
    // The frame: 4% wide, 4% in from the edge.
    let (a, b) = ((SIZE as f32 * 0.04) as usize, (SIZE as f32 * 0.08) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = x.min(y).min(SIZE - 1 - x).min(SIZE - 1 - y);
            if d >= a && d < b {
                let p = &mut px[y * SIZE + x];
                for (c, w) in p.iter_mut().zip([1.0, 1.0, 1.0]) {
                    *c = *c * 0.15 + w * 0.85;
                }
            }
        }
    }
    let want = SIZE as f32 * 0.62;
    if let Some((w, h, data)) = glyph(text, want) {
        // Scaled to fit (bitmap fonts come at their own size), centred a little below the middle.
        let k = want / w.max(h) as f32;
        let (ow, oh) = ((w as f32 * k) as usize, (h as f32 * k) as usize);
        let (x0, y0) = (
            (SIZE as f32 / 2.0 - ow as f32 / 2.0) as i64,
            (SIZE as f32 * 0.54 - oh as f32 / 2.0) as i64,
        );
        for y in 0..oh {
            for x in 0..ow {
                let (sx, sy) = (((x as f32 + 0.5) / k) as usize, ((y as f32 + 0.5) / k) as usize);
                let s = &data[(sy.min(h - 1) * w + sx.min(w - 1)) * 4..][..4];
                let alpha = s[3] as f32 / 255.0;
                let (tx, ty) = (x0 + x as i64, y0 + y as i64);
                if alpha <= 0.0 || tx < 0 || ty < 0 || tx >= SIZE as i64 || ty >= SIZE as i64 {
                    continue;
                }
                let p = &mut px[ty as usize * SIZE + tx as usize];
                for c in 0..3 {
                    p[c] = p[c] * (1.0 - alpha) + s[c] as f32 / 255.0 * alpha;
                }
            }
        }
    }
    let data = px
        .iter()
        .flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect();
    let mut image = Image::new(
        Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // (Mips: the frame and the emoji shimmer at a distance without them.)
    super::surface::add_mips(&mut image);
    super::surface::filtered(&mut image, 8);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_has_pixels() {
        let (w, h, data) = glyph("⭐", 160.0).expect("star in the emoji font");
        assert!(w > 16 && h > 16);
        assert!(data.chunks(4).filter(|p| p[3] > 200).count() > 100);
        let img = board("🏆", Color::srgb(0.2, 0.4, 0.8));
        // Every mip level down to 1×1.
        assert_eq!(img.texture_descriptor.mip_level_count, SIZE.trailing_zeros() + 1);
        let total: usize = (0..=SIZE.trailing_zeros()).map(|l| (SIZE >> l).pow(2) * 4).sum();
        assert_eq!(img.data.unwrap().len(), total);
    }
}
