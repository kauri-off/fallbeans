//! Cloth: the flags' pennants wave in the wind. A surface material (`surface.rs`, the same fragment
//! stage) with a vertex stage that bends the pennant by a wave running from the pole to the tip, the
//! normals with it; the prepass does the same, so the shadows wave too. The wind is one for the whole
//! world: gusts roll across it along `WIND` (`gust`), and the trees sway with them (`props.rs`).
//!
//! The cloth itself is made here (`Cut::mesh`) in place of flag.glb's pennant: one fine, two-sided sheet
//! (the material lights its back as its front) cut as a pennant or a swallowtail, sewn into a sleeve round
//! the pole. The wave starts flat at the sleeve's seam, so nothing of it ever reaches the pole.
use std::f32::consts::TAU;

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoAutoAabb;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

use super::surface::{Kind, Spec, SurfaceMaterial, SurfaceUniform, Surfaces};
use crate::view::MainCamera;

pub type ClothMaterial = ExtendedMaterial<StandardMaterial, Cloth>;

/// Where the wind blows (world x, z; `WIND` in `cloth.wgsl`).
pub const WIND: Vec2 = Vec2::new(0.9439, 0.3303);

/// The wind's strength at a place (0.2…1): gusts rolling across the world along `WIND`, on the
/// renderer's clock (`Time::elapsed_secs_wrapped`; `gust` in `cloth.wgsl`).
pub fn gust(t: f32, p: Vec2) -> f32 {
    let d = p.dot(WIND);
    0.6 + 0.25 * (t * 0.9 - d * 0.11).sin() + 0.15 * (t * 2.3 - d * 0.31 + 1.7).sin()
}

// flag.glb (model units): the pole stands round the y axis, 0.07 in radius, up to its ball (0.16 in radius)
// at y = 4.45. The cloth is in the frame of the model's pennant, which turns a little about the pole axis
// (`props::animate`): the sleeve turns round the pole with it.
const POLE_R: f32 = 0.07;
/// The sleeve: its radius round the pole, and the seam where the cloth leaves it (local x).
const SLEEVE_R: f32 = 0.088;
const ROOT_X: f32 = 0.15;
/// The cloth: its middle and half its height at the sleeve, its length from the seam to the far end, and how
/// far the far end hangs below the middle.
const MID_Y: f32 = 3.46;
const HALF_H: f32 = 0.64;
const LENGTH: f32 = 1.95;
const DROOP: f32 = 0.12;
/// The pennant's blunt tip (half its height); the swallowtail's notch (a share of the length).
const TIP: f32 = 0.03;
const NOTCH: f32 = 0.3;
/// Swing across the cloth at the far end, in full wind (model units), and the flutter's share of it.
const SWING: f32 = 0.2;
const FLUTTER: f32 = 0.18;
/// Quads along the cloth; the rows across it (bottom −1 … top 1; the outer ones are the hem); segments round
/// the sleeve.
const ALONG: usize = 32;
const ROWS: [f32; 11] = [-1.0, -0.92, -0.72, -0.48, -0.24, 0.0, 0.24, 0.48, 0.72, 0.92, 1.0];
const ROUND: usize = 16;

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct ClothUniform {
    /// The seam along the length (local x), 1 / length, swing at the far end, waves along the length.
    pub shape: Vec4,
    /// Wave speed (rad/s), flutter share, phase per unit of height (rad), unused.
    pub motion: Vec4,
}

const PENNANT: ClothUniform = ClothUniform {
    shape: Vec4::new(ROOT_X, 1.0 / LENGTH, SWING, 1.3),
    motion: Vec4::new(7.0, FLUTTER, 0.9, 0.0),
};

/// The pennant's bounds with its swing (the mesh's own are flat: culled while still in view).
pub fn pennant_bounds() -> (Aabb, NoAutoAabb) {
    let z = SWING * (1.0 + FLUTTER) + 0.1;
    (
        Aabb::from_min_max(
            Vec3::new(-SLEEVE_R - 0.02, MID_Y - HALF_H - DROOP - 0.05, -z),
            Vec3::new(ROOT_X + LENGTH + 0.05, MID_Y + HALF_H + 0.05, z),
        ),
        NoAutoAabb,
    )
}

/// How a flag's cloth is cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// A long triangle with gently bowed edges and a blunt tip.
    Pennant,
    /// A banner tapering a little, its end cut in a V.
    Swallowtail,
}

impl Cut {
    const ALL: [Cut; 2] = [Cut::Pennant, Cut::Swallowtail];

    /// Half the cloth's height at `s` along it (0 at the seam, 1 at the far end).
    fn half_height(self, s: f32) -> f32 {
        match self {
            Cut::Pennant => TIP + (HALF_H - TIP) * (1.0 - s) * (1.0 + 0.3 * s),
            Cut::Swallowtail => HALF_H * (1.0 - 0.15 * s),
        }
    }

    /// How far along the cloth the row at `v` (−1 … 1 across it) reaches.
    fn reach(self, v: f32) -> f32 {
        match self {
            Cut::Pennant => 1.0,
            Cut::Swallowtail => 1.0 - NOTCH * (1.0 - v.abs()),
        }
    }

    /// The cloth: a sheet flat in the xy plane facing +z (the vertex stage bends it, the material is two-sided),
    /// sewn into its sleeve; the hems and the seam a little darker (vertex colours).
    fn geometry(self) -> Geometry {
        let mut g = Geometry::default();
        let cols = ALONG + 1;
        for &v in &ROWS {
            for i in 0..cols {
                let u = i as f32 / ALONG as f32;
                let s = u * self.reach(v);
                let y = MID_Y - DROOP * s * s + v * self.half_height(s);
                let hem: f32 = if v.abs() > 0.99 { 0.84 } else { 1.0 };
                let seam: f32 = if i == 0 {
                    0.86
                } else if i == ALONG {
                    0.9
                } else {
                    1.0
                };
                let p = Vec3::new(ROOT_X + LENGTH * s, y, 0.0);
                g.vertex(p, Vec3::Z, Vec2::new(u, (1.0 - v) * 0.5), hem.min(seam));
            }
        }
        for j in 0..ROWS.len() - 1 {
            for i in 0..ALONG {
                let a = (j * cols + i) as u16;
                let b = a + 1;
                let c = a + cols as u16;
                let d = c + 1;
                // (Counter-clockwise seen from +z: the front is the normal's side.)
                g.idx.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
        sleeve(&mut g);
        g
    }

    pub fn mesh(self) -> Mesh {
        let g = self.geometry();
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, g.pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, g.nrm)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, g.uv)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, g.col)
            .with_inserted_indices(Indices::U16(g.idx))
    }
}

/// A mesh's data as it is made.
#[derive(Default)]
struct Geometry {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    col: Vec<[f32; 4]>,
    idx: Vec<u16>,
}

impl Geometry {
    fn vertex(&mut self, p: Vec3, n: Vec3, uv: Vec2, shade: f32) {
        self.pos.push(p.to_array());
        self.nrm.push(n.to_array());
        self.uv.push(uv.to_array());
        self.col.push([shade, shade, shade, 1.0]);
    }
}

/// The sleeve round the pole, as tall as the cloth at the seam: a tube shaped like a drop, its point the seam,
/// closed at both ends by flat rings in to just off the pole. It never moves with the wave (it is short of
/// the seam), only with the pennant's turn about the pole.
fn sleeve(g: &mut Geometry) {
    let (y0, y1) = (MID_Y - HALF_H, MID_Y + HALF_H);
    // Round the pole from the seam and back to it (x, z), counter-clockwise seen from above, with the outward
    // normals: the drop's straight sides touch the circle where its normal is theirs.
    let t0 = (SLEEVE_R / ROOT_X).acos();
    let seam = Vec2::new(ROOT_X, 0.0);
    let mut ring = vec![(seam, Vec2::from_angle(t0))];
    for k in 0..=ROUND {
        let d = Vec2::from_angle(t0 + (TAU - 2.0 * t0) * k as f32 / ROUND as f32);
        ring.push((d * SLEEVE_R, d));
    }
    ring.push((seam, Vec2::from_angle(-t0)));
    let at = |q: Vec2, y: f32| Vec3::new(q.x, y, q.y);
    let last = ring.len() - 1;
    // The side: bottom and top of each step round.
    let base = g.pos.len() as u16;
    for (k, &(q, n)) in ring.iter().enumerate() {
        let shade = if k == 0 || k == last { 0.78 } else { 0.9 };
        let n = Vec3::new(n.x, 0.0, n.y);
        let u = k as f32 / last as f32;
        g.vertex(at(q, y0), n, Vec2::new(u, 1.0), shade);
        g.vertex(at(q, y1), n, Vec2::new(u, 0.0), shade);
    }
    for k in 0..last as u16 {
        let (a, c) = (base + 2 * k, base + 2 * k + 1);
        let (b, d) = (a + 2, c + 2);
        g.idx.extend_from_slice(&[a, c, b, b, c, d]);
    }
    // The ends.
    for (y, up) in [(y1, true), (y0, false)] {
        let n = if up { Vec3::Y } else { Vec3::NEG_Y };
        let base = g.pos.len() as u16;
        for &(q, _) in &ring {
            g.vertex(at(q, y), n, Vec2::ZERO, 0.8);
            g.vertex(at(q.normalize() * (POLE_R + 0.004), y), n, Vec2::ZERO, 0.7);
        }
        for k in 0..last as u16 {
            let (o, i) = (base + 2 * k, base + 2 * k + 1);
            let (o2, i2) = (o + 2, i + 2);
            if up {
                g.idx.extend_from_slice(&[o, i, o2, o2, i, i2]);
            } else {
                g.idx.extend_from_slice(&[o, o2, i, o2, i2, i]);
            }
        }
    }
}

/// A surface (bindings 50–53 as `Surface` without bindless, which surface.wgsl reads then) that waves (54).
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct Cloth {
    #[uniform(50)]
    pub u: SurfaceUniform,
    #[texture(51)]
    #[sampler(52)]
    pub detail: Handle<Image>,
    #[texture(53)]
    pub occluders: Handle<Image>,
    #[uniform(54)]
    pub wave: ClothUniform,
}

impl MaterialExtension for Cloth {
    fn vertex_shader() -> ShaderRef {
        "embedded://fb_client/render/cloth.wgsl".into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        "embedded://fb_client/render/cloth.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/surface.wgsl".into()
    }

    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The fragment shader reads the mesh's transform (object-space mapping), as a surface's.
        let def = "VERTEX_OUTPUT_INSTANCE_INDEX";
        descriptor.vertex.shader_defs.push(def.into());
        if let Some(f) = &mut descriptor.fragment {
            f.shader_defs.push(def.into());
        }
        Ok(())
    }
}

/// The waving twin of each surface material a pennant wears (kept in step with it by `follow`), and the cloths
/// of every cut (made at the start: `shape`).
#[derive(Resource, Default)]
pub struct Cloths {
    twins: Vec<(Handle<SurfaceMaterial>, Handle<ClothMaterial>)>,
    meshes: Vec<Handle<Mesh>>,
}

impl Cloths {
    pub fn pennant(
        &mut self,
        surface: &Handle<SurfaceMaterial>,
        surfaces: &Assets<SurfaceMaterial>,
        cloths: &mut Assets<ClothMaterial>,
    ) -> Option<Handle<ClothMaterial>> {
        if let Some((_, c)) = self.twins.iter().find(|(s, _)| s == surface) {
            return Some(c.clone());
        }
        let src = surfaces.get(surface)?;
        // Two-sided, its back lit as its front (the cloth is one sheet); no baked occlusion (the model's was
        // for its own pennant, not this cloth: the vertex colours shade the hems instead).
        let mut base = src.base.clone();
        base.double_sided = true;
        base.cull_mode = None;
        base.occlusion_texture = None;
        let c = cloths.add(ExtendedMaterial {
            base,
            extension: Cloth {
                u: src.extension.u,
                detail: src.extension.detail.clone(),
                occluders: src.extension.occluders.clone(),
                wave: PENNANT,
            },
        });
        self.twins.push((surface.clone(), c.clone()));
        Some(c)
    }

    /// The cloth of a cut (once `shape` has made them).
    pub fn mesh(&self, cut: Cut) -> Option<Handle<Mesh>> {
        self.meshes.get(cut as usize).cloned()
    }

    /// Forgets the twins of surface materials no longer made for models.
    pub fn retain(&mut self, keep: impl Fn(AssetId<SurfaceMaterial>) -> bool) {
        self.twins.retain(|(s, _)| keep(s.id()));
    }
}

fn shape(mut cloths: ResMut<Cloths>, mut meshes: ResMut<Assets<Mesh>>) {
    cloths.meshes = Cut::ALL.iter().map(|c| meshes.add(c.mesh())).collect();
}

/// The surfaces' switches (the Low preset's plain look) and their detail textures (a surface made before its
/// texture was ready gets it later) carried over to their twins.
fn follow(cloths: Res<Cloths>, surfaces: Res<Assets<SurfaceMaterial>>, mut mats: ResMut<Assets<ClothMaterial>>) {
    for (s, c) in &cloths.twins {
        let Some(src) = surfaces.get(s) else { continue };
        let Some(m) = mats.get(c) else { continue };
        if m.extension.u.extra == src.extension.u.extra && m.extension.detail == src.extension.detail {
            continue;
        }
        if let Some(mut m) = mats.get_mut(c) {
            m.extension.u = src.extension.u;
            m.extension.detail = src.extension.detail.clone();
        }
    }
}

/// How long the warm-up pennant stays (s of real time), as `warmup.rs`'s objects.
const KEEP_S: f32 = 4.0;

#[derive(Component)]
struct Warm;

/// A tiny pennant in front of the camera at the start: its pipelines (and its shadow's) compile before
/// the first round. (The flags' own cloth: the pipelines depend on the mesh's attributes.)
fn warm_up(
    mut commands: Commands,
    camera: Query<Entity, With<MainCamera>>,
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut surface_mats: ResMut<Assets<SurfaceMaterial>>,
    mut cloths: ResMut<Cloths>,
    mut cloth_mats: ResMut<Assets<ClothMaterial>>,
) {
    let Ok(cam) = camera.single() else { return };
    let Some(mesh) = cloths.mesh(Cut::Pennant) else {
        return;
    };
    let s = surfaces.material(
        &Spec::plain(LinearRgba::WHITE, Some(Kind::Cloth)),
        &mut images,
        &mut surface_mats,
    );
    let Some(m) = cloths.pennant(&s, &surface_mats, &mut cloth_mats) else {
        return;
    };
    commands.spawn((
        Warm,
        Mesh3d(mesh),
        MeshMaterial3d(m),
        pennant_bounds(),
        Transform::from_xyz(0.03, -0.002, -1.0).with_scale(Vec3::splat(0.0005)),
        ChildOf(cam),
    ));
}

fn cool_down(mut commands: Commands, q: Query<Entity, With<Warm>>, time: Res<Time<Real>>) {
    if time.elapsed_secs() < KEEP_S {
        return;
    }
    for e in &q {
        commands.entity(e).despawn();
    }
}

pub struct ClothPlugin;

impl Plugin for ClothPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "cloth.wgsl");
        app.add_plugins(MaterialPlugin::<ClothMaterial>::default());
        app.init_resource::<Cloths>();
        app.add_systems(Startup, (shape, warm_up).chain().after(crate::view::setup_camera));
        app.add_systems(Update, (follow, cool_down));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gusts_stay_in_range() {
        for i in 0..2000 {
            let t = i as f32 * 0.37;
            let g = gust(t, Vec2::new(i as f32 * 1.3 - 900.0, i as f32 * -0.7));
            assert!((0.19..=1.01).contains(&g), "{g}");
        }
    }

    #[test]
    fn cloth_keeps_off_the_pole_and_in_its_bounds() {
        let (aabb, _) = pennant_bounds();
        let (lo, hi) = (Vec3::from(aabb.min()), Vec3::from(aabb.max()));
        for cut in Cut::ALL {
            let g = cut.geometry();
            assert!(g.pos.len() <= usize::from(u16::MAX), "{cut:?}");
            assert!(g.idx.iter().all(|&i| usize::from(i) < g.pos.len()), "{cut:?}");
            for p in g.pos.iter().map(|p| Vec3::from(*p)) {
                assert!(Vec2::new(p.x, p.z).length() > POLE_R, "{cut:?} {p}");
                // (Below the ball.)
                assert!(p.y < 4.45 - 0.16, "{cut:?} {p}");
                assert!(p.cmpge(lo).all() && p.cmple(hi).all(), "{cut:?} {p}");
            }
            // The widest swing fits the bounds across.
            assert!(SWING * (1.0 + FLUTTER) + SLEEVE_R < hi.z, "{cut:?}");
        }
    }

    #[test]
    fn cloth_faces_its_normals() {
        // (The material is two-sided: a triangle wound against its normals would be lit from behind.)
        for cut in Cut::ALL {
            let g = cut.geometry();
            for t in g.idx.chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(g.pos[usize::from(i)]));
                let face = (b - a).cross(c - a);
                if face.length() < 1e-7 {
                    continue;
                }
                let n: Vec3 = [t[0], t[1], t[2]]
                    .iter()
                    .map(|&i| Vec3::from(g.nrm[usize::from(i)]))
                    .sum();
                assert!(face.dot(n) > 0.0, "{cut:?} {a} {b} {c}");
            }
        }
    }
}
