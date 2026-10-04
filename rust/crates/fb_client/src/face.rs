//! The bean's mouth and brows (port of `face.ts`). The mouth is drawn into a texture per expression and
//! shown on a thin patch laid over the front of the model (visor and body), so nothing sticks out of it
//! or cuts into it; the brows are small solid strokes lying on the visor above the eyes, which squint,
//! widen and roll with them. The eyes and tears are animated in `bean.rs`/`beans.rs`.
use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::gltf::GltfMaterialName;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::bean::Expr;
use crate::beans::Rig;

/// Patch bounds on the face (model space, metres).
const X0: f32 = -0.25;
const X1: f32 = 0.25;
const Y0: f32 = 0.94;
const Y1: f32 = 1.42;
const TEX: usize = 512;
const GRID: usize = 28;

const EYE_Y: f32 = 1.23;
const EYE_HALF: f32 = 0.1;
const BROW_X: f32 = 0.112;
const BROW_GAP: f32 = 0.023;
const BROW_MIN_Y: f32 = 1.335;
const BROW_MAX_Y: f32 = 1.37;
/// How far a brow floats off the visor (m).
const BROW_LIFT: f32 = 0.011;

const EXPRS: [Expr; 10] = [
    Expr::Smile,
    Expr::Grin,
    Expr::Laugh,
    Expr::Surprised,
    Expr::Scared,
    Expr::Sad,
    Expr::Cry,
    Expr::Dizzy,
    Expr::Strain,
    Expr::Determined,
];

// ---------------------------------------------------------------- a small canvas

type Rgba = [f32; 4];

fn rgba(hex: &str, a: f32) -> Rgba {
    let c = crate::view::hex(hex).to_srgba();
    [c.red, c.green, c.blue, a]
}

/// Straight-alpha sRGB pixels, blended as a 2D canvas does.
struct Canvas {
    px: Vec<Rgba>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            px: vec![[0.0; 4]; TEX * TEX],
        }
    }

    fn blend(&mut self, i: usize, c: Rgba, cov: f32) {
        let a = c[3] * cov;
        if a <= 0.0 {
            return;
        }
        let d = self.px[i];
        let out_a = a + d[3] * (1.0 - a);
        let mut o = [0.0; 4];
        for k in 0..3 {
            o[k] = (c[k] * a + d[k] * d[3] * (1.0 - a)) / out_a.max(1e-6);
        }
        o[3] = out_a;
        self.px[i] = o;
    }

    /// Coverage of a closed polygon (non-zero winding), 4 sub-rows per pixel, exact across a row.
    fn coverage(poly: &[Vec2]) -> Vec<f32> {
        let mut cov = vec![0f32; TEX * TEX];
        let (mut y0, mut y1) = (f32::MAX, f32::MIN);
        for p in poly {
            y0 = y0.min(p.y);
            y1 = y1.max(p.y);
        }
        let r0 = (y0.floor().max(0.0)) as usize;
        let r1 = (y1.ceil().min(TEX as f32)) as usize;
        const SUB: usize = 4;
        let mut xs: Vec<(f32, i32)> = Vec::new();
        for row in r0..r1 {
            for s in 0..SUB {
                let y = row as f32 + (s as f32 + 0.5) / SUB as f32;
                xs.clear();
                for i in 0..poly.len() {
                    let a = poly[i];
                    let b = poly[(i + 1) % poly.len()];
                    if (a.y <= y) != (b.y <= y) {
                        let x = a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                        xs.push((x, if b.y > a.y { 1 } else { -1 }));
                    }
                }
                xs.sort_by(|p, q| p.0.total_cmp(&q.0));
                let mut wind = 0;
                let mut start = 0.0;
                for (x, dir) in &xs {
                    let was = wind;
                    wind += dir;
                    if was == 0 && wind != 0 {
                        start = *x;
                    } else if was != 0 && wind == 0 {
                        span(&mut cov[row * TEX..(row + 1) * TEX], start, *x, 1.0 / SUB as f32);
                    }
                }
            }
        }
        cov
    }

    fn fill(&mut self, poly: &[Vec2], paint: impl Fn(usize, usize) -> Rgba, clip: Option<&[f32]>) {
        let cov = Self::coverage(poly);
        for (i, c) in cov.iter().enumerate() {
            let c = c.min(1.0) * clip.map_or(1.0, |m| m[i].min(1.0));
            if c > 0.0 {
                self.blend(i, paint(i % TEX, i / TEX), c);
            }
        }
    }

    /// A polyline of `width` with round caps and joins.
    fn stroke(&mut self, pts: &[Vec2], closed: bool, width: f32, color: Rgba) {
        let mut cov = vec![0f32; TEX * TEX];
        let r = width / 2.0;
        let n = if closed { pts.len() } else { pts.len() - 1 };
        for i in 0..n {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            let lo = a.min(b) - Vec2::splat(r + 1.0);
            let hi = a.max(b) + Vec2::splat(r + 1.0);
            for y in (lo.y.max(0.0) as usize)..(hi.y.min(TEX as f32 - 1.0) as usize + 1) {
                for x in (lo.x.max(0.0) as usize)..(hi.x.min(TEX as f32 - 1.0) as usize + 1) {
                    let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let ab = b - a;
                    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                    let d = (a + ab * t).distance(p);
                    let c = (r - d + 0.5).clamp(0.0, 1.0);
                    let k = y * TEX + x;
                    cov[k] = cov[k].max(c);
                }
            }
        }
        for (i, c) in cov.iter().enumerate() {
            if *c > 0.0 {
                self.blend(i, color, *c);
            }
        }
    }

    fn image(&self) -> Image {
        let mut data = Vec::with_capacity(TEX * TEX * 4);
        for p in &self.px {
            for c in p {
                data.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        Image::new(
            Extent3d {
                width: TEX as u32,
                height: TEX as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        )
    }
}

/// Adds `amount` of coverage to the pixels of a row between x0 and x1 (fractional ends).
fn span(row: &mut [f32], x0: f32, x1: f32, amount: f32) {
    let (x0, x1) = (x0.max(0.0), x1.min(TEX as f32));
    if x1 <= x0 {
        return;
    }
    let (i0, i1) = (x0.floor() as usize, x1.floor() as usize);
    if i0 == i1 {
        row[i0.min(TEX - 1)] += (x1 - x0) * amount;
        return;
    }
    row[i0] += (i0 as f32 + 1.0 - x0) * amount;
    for v in &mut row[i0 + 1..i1.min(TEX)] {
        *v += amount;
    }
    if i1 < TEX {
        row[i1] += (x1 - i1 as f32) * amount;
    }
}

/// A path as points: segments of quadratic and cubic curves flattened.
struct Path {
    pts: Vec<Vec2>,
}

impl Path {
    fn at(p: Vec2) -> Self {
        Self { pts: vec![p] }
    }

    fn last(&self) -> Vec2 {
        *self.pts.last().unwrap()
    }

    fn line(mut self, p: Vec2) -> Self {
        self.pts.push(p);
        self
    }

    fn quad(mut self, c: Vec2, p: Vec2) -> Self {
        let a = self.last();
        for k in 1..=24 {
            let t = k as f32 / 24.0;
            self.pts
                .push(a * (1.0 - t).powi(2) + c * 2.0 * t * (1.0 - t) + p * t * t);
        }
        self
    }

    fn cubic(mut self, c1: Vec2, c2: Vec2, p: Vec2) -> Self {
        let a = self.last();
        for k in 1..=32 {
            let t = k as f32 / 32.0;
            let u = 1.0 - t;
            self.pts
                .push(a * u * u * u + c1 * 3.0 * u * u * t + c2 * 3.0 * u * t * t + p * t * t * t);
        }
        self
    }
}

fn ellipse(c: Vec2, rx: f32, ry: f32) -> Vec<Vec2> {
    (0..64)
        .map(|k| {
            let a = k as f32 / 64.0 * core::f32::consts::TAU;
            c + Vec2::new(a.cos() * rx, a.sin() * ry)
        })
        .collect()
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Vec<Vec2> {
    vec![
        Vec2::new(x, y),
        Vec2::new(x + w, y),
        Vec2::new(x + w, y + h),
        Vec2::new(x, y + h),
    ]
}

fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<Vec2> {
    let mut out = Vec::new();
    let corners = [
        (Vec2::new(x + w - r, y + r), -0.25f32),
        (Vec2::new(x + w - r, y + h - r), 0.0),
        (Vec2::new(x + r, y + h - r), 0.25),
        (Vec2::new(x + r, y + r), 0.5),
    ];
    for (c, start) in corners {
        for k in 0..=8 {
            let a = (start + k as f32 / 32.0) * core::f32::consts::TAU;
            out.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    }
    out
}

/// Canvas position of a point on the face (model metres).
fn px(x: f32) -> f32 {
    (x - X0) / (X1 - X0) * TEX as f32
}
fn py(y: f32) -> f32 {
    (Y1 - y) / (Y1 - Y0) * TEX as f32
}
fn m(d: f32) -> f32 {
    d / (X1 - X0) * TEX as f32
}

const INK: &str = "#3a0f22";
const TONGUE: &str = "#ff6f8f";

/// The mouth of an expression, as `face.ts` draws it on its canvas.
fn draw(e: Expr) -> Image {
    let mut g = Canvas::new();
    let ink = rgba(INK, 1.0);
    let cx = px(0.0);
    let my = py(1.095);
    let open_mouth = |g: &mut Canvas, w: f32, h: f32, down: bool, top: f32| {
        let hw = m(w) / 2.0;
        let hh = m(h);
        let y0 = my - if down { -hh * 0.35 } else { hh * 0.35 } - top;
        let s = if down { -1.0 } else { 1.0 };
        let shape = Path::at(Vec2::new(cx - hw, y0))
            .quad(Vec2::new(cx, y0 + s * hh * 0.18), Vec2::new(cx + hw, y0))
            .cubic(
                Vec2::new(cx + hw * 0.9, y0 + s * hh * 1.25),
                Vec2::new(cx - hw * 0.9, y0 + s * hh * 1.25),
                Vec2::new(cx - hw, y0),
            )
            .pts;
        g.fill(&shape, |_, _| ink, None);
        let clip = Canvas::coverage(&shape);
        // Tongue at the bottom, a strip of teeth at the top.
        let ty = if down { y0 - hh * 0.05 } else { y0 + hh * 0.95 };
        g.fill(
            &ellipse(Vec2::new(cx, ty), hw * 0.55, hh * 0.45),
            |_, _| rgba(TONGUE, 1.0),
            Some(&clip),
        );
        if !down {
            g.fill(
                &rect(cx - hw, y0 - hh * 0.2, hw * 2.0, hh * 0.3),
                |_, _| [1.0; 4],
                Some(&clip),
            );
        }
        g.stroke(&shape, true, m(0.008), ink);
    };
    let arc = |g: &mut Canvas, w: f32, bend: f32, width: f32, y: f32| {
        let p = Path::at(Vec2::new(px(-w / 2.0), py(y)))
            .quad(Vec2::new(cx, py(y - bend)), Vec2::new(px(w / 2.0), py(y)))
            .pts;
        g.stroke(&p, false, m(width), ink);
    };
    match e {
        Expr::Smile => open_mouth(&mut g, 0.135, 0.062, false, 0.0),
        Expr::Grin => open_mouth(&mut g, 0.15, 0.075, false, 0.0),
        Expr::Laugh => open_mouth(&mut g, 0.16, 0.1, false, m(0.01)),
        Expr::Surprised => g.fill(&ellipse(Vec2::new(cx, py(1.085)), m(0.03), m(0.036)), |_, _| ink, None),
        Expr::Scared => {
            g.fill(&ellipse(Vec2::new(cx, py(1.08)), m(0.04), m(0.052)), |_, _| ink, None);
            g.fill(&rect(cx - m(0.03), py(1.12), m(0.06), m(0.012)), |_, _| [1.0; 4], None);
        }
        Expr::Sad => arc(&mut g, 0.1, -0.03, 0.013, 1.095),
        Expr::Cry => {
            for s in [-1.0f32, 1.0] {
                let x = s * 0.115;
                let tear = Path::at(Vec2::new(px(x - 0.018), py(1.16)))
                    .cubic(
                        Vec2::new(px(x - 0.03), py(1.06)),
                        Vec2::new(px(x + 0.005), py(1.02)),
                        Vec2::new(px(x - 0.01), py(0.96)),
                    )
                    .line(Vec2::new(px(x + 0.022), py(0.96)))
                    .cubic(
                        Vec2::new(px(x + 0.03), py(1.03)),
                        Vec2::new(px(x + 0.02), py(1.08)),
                        Vec2::new(px(x + 0.018), py(1.16)),
                    )
                    .pts;
                let (t0, t1) = (py(1.16), py(0.96));
                g.fill(
                    &tear,
                    |_, y| {
                        let f = ((y as f32 - t0) / (t1 - t0)).clamp(0.0, 1.0);
                        [140.0 / 255.0, 210.0 / 255.0, 1.0, 0.95 + (0.15 - 0.95) * f]
                    },
                    None,
                );
            }
            open_mouth(&mut g, 0.12, 0.06, true, 0.0);
        }
        Expr::Dizzy => {
            let pts: Vec<Vec2> = (0..=24)
                .map(|k| {
                    let f = k as f32 / 24.0;
                    let x = -0.055 + f * 0.11;
                    let y = 1.09 + (f * core::f32::consts::PI * 4.0).sin() * 0.012;
                    Vec2::new(px(x), py(y))
                })
                .collect();
            g.stroke(&pts, false, m(0.012), ink);
        }
        Expr::Strain => {
            // Gritted teeth.
            let (w, h) = (m(0.11), m(0.04));
            let box_ = round_rect(cx - w / 2.0, my - h / 2.0, w, h, h * 0.45);
            g.fill(&box_, |_, _| [1.0; 4], None);
            g.stroke(&box_, true, m(0.009), ink);
            g.stroke(
                &[Vec2::new(cx - w / 2.0, my), Vec2::new(cx + w / 2.0, my)],
                false,
                m(0.005),
                ink,
            );
            for k in 1..5 {
                let x = cx - w / 2.0 + w * k as f32 / 5.0;
                g.stroke(
                    &[Vec2::new(x, my - h / 2.0), Vec2::new(x, my + h / 2.0)],
                    false,
                    m(0.005),
                    ink,
                );
            }
        }
        Expr::Determined => arc(&mut g, 0.07, -0.008, 0.012, 1.095),
    }
    g.image()
}

// ---------------------------------------------------------------- the face's shape

/// Triangles of the visor and the body in model space.
struct Front {
    tris: Vec<[Vec3; 3]>,
}

impl Front {
    /// Where a ray from (x, y, 2) along −z first hits the front, and the normal there.
    fn hit(&self, x: f32, y: f32) -> Option<(f32, Vec3)> {
        let o = Vec3::new(x, y, 2.0);
        let d = Vec3::NEG_Z;
        let mut best: Option<(f32, Vec3)> = None;
        for [a, b, c] in &self.tris {
            let e1 = *b - *a;
            let e2 = *c - *a;
            let p = d.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-9 {
                continue;
            }
            let inv = 1.0 / det;
            let s = o - *a;
            let u = s.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = d.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(q) * inv;
            if t > 0.0 && best.is_none_or(|(bt, _)| t < bt) {
                let mut n = e1.cross(e2).normalize_or(Vec3::Z);
                if n.z < 0.0 {
                    n = -n;
                }
                best = Some((t, n));
            }
        }
        best.map(|(t, n)| (o.z - t, n))
    }

    fn z(&self, x: f32, y: f32) -> (f32, Vec3) {
        self.hit(x, y).unwrap_or(((0.25 - x * x).max(0.0).sqrt(), Vec3::Z))
    }
}

/// The front of the head where the brows go: its depth and normal on a grid (bilinear between).
struct Surface {
    z: Vec<f32>,
    n: Vec<Vec3>,
}

const S_X0: f32 = -0.22;
const S_Y0: f32 = 1.28;
const S_STEP: f32 = 0.005;
const S_NX: usize = 89;
const S_NY: usize = 27;

impl Surface {
    fn build(front: &Front) -> Self {
        let mut z = Vec::with_capacity(S_NX * S_NY);
        let mut n = Vec::with_capacity(S_NX * S_NY);
        for j in 0..S_NY {
            for i in 0..S_NX {
                let (zz, nn) = front.z(S_X0 + i as f32 * S_STEP, S_Y0 + j as f32 * S_STEP);
                z.push(zz);
                n.push(nn);
            }
        }
        Self { z, n }
    }

    fn at(&self, x: f32, y: f32) -> (Vec3, Vec3) {
        let fx = ((x - S_X0) / S_STEP).clamp(0.0, S_NX as f32 - 1.001);
        let fy = ((y - S_Y0) / S_STEP).clamp(0.0, S_NY as f32 - 1.001);
        let (i, j) = (fx.floor() as usize, fy.floor() as usize);
        let (u, v) = (fx - i as f32, fy - j as f32);
        let w = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
        let ks = [
            j * S_NX + i,
            j * S_NX + i + 1,
            (j + 1) * S_NX + i,
            (j + 1) * S_NX + i + 1,
        ];
        let mut z = 0.0;
        let mut n = Vec3::ZERO;
        for q in 0..4 {
            z += self.z[ks[q]] * w[q];
            n += self.n[ks[q]] * w[q];
        }
        (Vec3::new(x, y, z), n.normalize_or(Vec3::Z))
    }
}

/// A grid laid over the front of the model, a hair above it.
fn patch(front: &Front) -> Mesh {
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    for j in 0..GRID {
        for i in 0..GRID {
            let u = i as f32 / (GRID - 1) as f32;
            let v = j as f32 / (GRID - 1) as f32;
            let x = X0 + (X1 - X0) * u;
            let y = Y0 + (Y1 - Y0) * v;
            let (z, _) = front.z(x, y);
            pos.push([x, y, z + 0.004]);
            uv.push([u, 1.0 - v]);
        }
    }
    let mut idx = Vec::new();
    for j in 0..GRID - 1 {
        for i in 0..GRID - 1 {
            let a = (j * GRID + i) as u32;
            let g = GRID as u32;
            idx.extend_from_slice(&[a, a + 1, a + g, a + 1, a + g + 1, a + g]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
        .with_computed_smooth_normals()
}

/// A brow: a flattened, arched stroke along x, bent to follow the curve of the visor.
fn brow() -> Mesh {
    let mut mesh = Capsule3d::new(0.0145, 0.076).mesh().latitudes(8).longitudes(12).build();
    mesh.rotate_by(Quat::from_rotation_z(core::f32::consts::FRAC_PI_2));
    if let Some(VertexAttributeValues::Float32x3(p)) = mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
        for v in p.iter_mut() {
            let x = v[0];
            let e = (x.abs() / 0.05).min(1.0);
            // Thinner towards the ends, arched, flat against the face and curved round it.
            v[1] = v[1] * (1.0 - 0.35 * e * e) - x * x * 2.2;
            v[2] = v[2] * 0.5 - x * x * 1.65;
        }
    }
    mesh.compute_smooth_normals();
    mesh
}

// ---------------------------------------------------------------- systems

/// What every face shares: the patch, the brow, the surface, a material per expression.
#[derive(Resource)]
struct FaceKit {
    patch: Handle<Mesh>,
    brow: Handle<Mesh>,
    brow_mat: Handle<StandardMaterial>,
    surface: Surface,
    mouths: HashMap<Expr, Handle<StandardMaterial>>,
}

/// A bean's mouth patch and brows, and the expression shown.
#[derive(Component)]
struct FaceParts {
    patch: Entity,
    brows: [Entity; 2],
    shown: Expr,
}

pub struct FacePlugin;

impl Plugin for FacePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (make_kit, attach, animate).chain());
    }
}

/// The face's shape from the first bean model whose meshes are in.
#[allow(clippy::too_many_arguments)]
fn make_kit(
    mut commands: Commands,
    kit: Option<Res<FaceKit>>,
    rigs: Query<&Rig>,
    children: Query<&Children>,
    parts: Query<(&GltfMaterialName, &Mesh3d)>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if kit.is_some() {
        return;
    }
    let Some(model) = rigs.iter().find(|r| r.ready()).map(|r| r.model) else {
        return;
    };
    let mut tris = Vec::new();
    for e in children.iter_descendants(model) {
        let Ok((mat, mesh)) = parts.get(e) else { continue };
        if !matches!(mat.0.as_str(), "Visor" | "Body") {
            continue;
        }
        // (Limbs wear the body's material too: only the body itself.)
        let mut affine = Mat4::IDENTITY;
        let mut on_limb = false;
        let mut at = e;
        while at != model {
            if let Ok(n) = names.get(at)
                && matches!(n.as_str(), "ArmL" | "ArmR" | "LegL" | "LegR" | "HandL" | "HandR")
            {
                on_limb = true;
            }
            if let Ok(tf) = transforms.get(at) {
                affine = tf.to_matrix() * affine;
            }
            let Ok(p) = parents.get(at) else { break };
            at = p.parent();
        }
        if on_limb {
            continue;
        }
        let Some(m) = meshes.get(&mesh.0) else { return };
        let Some(VertexAttributeValues::Float32x3(pos)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
            continue;
        };
        let p: Vec<Vec3> = pos.iter().map(|v| affine.transform_point3(Vec3::from(*v))).collect();
        match m.indices() {
            Some(ix) => {
                let ix: Vec<usize> = ix.iter().collect();
                for t in ix.as_chunks::<3>().0 {
                    tris.push([p[t[0]], p[t[1]], p[t[2]]]);
                }
            }
            None => {
                for t in p.as_chunks::<3>().0 {
                    tris.push(*t);
                }
            }
        }
    }
    if tris.is_empty() {
        return;
    }
    let front = Front { tris };
    let mouths = EXPRS
        .iter()
        .map(|e| {
            let tex = images.add(draw(*e));
            let mat = materials.add(StandardMaterial {
                base_color_texture: Some(tex),
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 0.5,
                depth_bias: 4.0,
                ..default()
            });
            (*e, mat)
        })
        .collect();
    commands.insert_resource(FaceKit {
        patch: meshes.add(patch(&front)),
        brow: meshes.add(brow()),
        brow_mat: materials.add(StandardMaterial {
            base_color: crate::view::hex(INK),
            perceptual_roughness: 0.6,
            ..default()
        }),
        surface: Surface::build(&front),
        mouths,
    });
}

fn attach(mut commands: Commands, kit: Option<Res<FaceKit>>, rigs: Query<(Entity, &Rig), Without<FaceParts>>) {
    let Some(kit) = kit else { return };
    for (e, rig) in &rigs {
        if !rig.ready() {
            continue;
        }
        let patch = commands
            .spawn((
                Mesh3d(kit.patch.clone()),
                MeshMaterial3d(kit.mouths[&Expr::Smile].clone()),
                Transform::default(),
                NotShadowCaster,
                ChildOf(rig.model),
            ))
            .id();
        let brows = [0, 1].map(|_| {
            commands
                .spawn((
                    Mesh3d(kit.brow.clone()),
                    MeshMaterial3d(kit.brow_mat.clone()),
                    Transform::default(),
                    NotShadowCaster,
                    ChildOf(rig.model),
                ))
                .id()
        });
        commands.entity(e).insert(FaceParts {
            patch,
            brows,
            shown: Expr::Smile,
        });
    }
}

fn animate(
    kit: Option<Res<FaceKit>>,
    mut faces: Query<(&mut FaceParts, &crate::bean::BeanAnim)>,
    mut mats: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut tfs: Query<&mut Transform>,
) {
    let Some(kit) = kit else { return };
    for (mut face, anim) in &mut faces {
        let out = &anim.out;
        if out.expr != face.shown {
            face.shown = out.expr;
            if let Ok(mut m) = mats.get_mut(face.patch) {
                m.0 = kit.mouths[&out.expr].clone();
            }
        }
        // Each brow on the visor above its eye, turned by the tilt within the surface.
        let y = (EYE_Y + EYE_HALF * out.wide + BROW_GAP + out.brow_lift).clamp(BROW_MIN_Y, BROW_MAX_Y);
        for (i, b) in face.brows.iter().enumerate() {
            let side = if i == 1 { 1.0 } else { -1.0 };
            let (p, n) = kit.surface.at(side * BROW_X, y);
            // Along the surface, level; then raised at the inner end by the tilt.
            let mut t = (Vec3::X - n * n.x).normalize();
            let bb = n.cross(t);
            let a = -side * out.brow_tilt;
            t = t * a.cos() + bb * a.sin();
            let bb = n.cross(t);
            let m = Mat3::from_cols(t, bb, n);
            if let Ok(mut tf) = tfs.get_mut(*b) {
                *tf = Transform {
                    translation: p + n * BROW_LIFT,
                    rotation: Quat::from_mat3(&m),
                    scale: Vec3::ONE,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cargo test -p fb_client face -- --ignored`: the mouths as PNG files in /tmp, to look at.
    #[test]
    #[ignore]
    fn mouths_to_files() {
        for e in EXPRS {
            let img = draw(e).try_into_dynamic().unwrap();
            img.save(format!("/tmp/mouth-{e:?}.png")).unwrap();
        }
    }

    #[test]
    fn mouths_draw() {
        for e in EXPRS {
            let img = draw(e);
            let data = img.data.unwrap();
            let opaque = data.chunks(4).filter(|p| p[3] > 128).count();
            assert!(opaque > 100, "{e:?}: {opaque} pixels");
        }
    }
}
