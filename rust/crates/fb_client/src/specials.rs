//! Drawing the specials of a map (portal rings, hex tiles, glass panes…): the pieces their looks place
//! every frame, and the map's primitives they tint. Lit pieces are drawn on surfaces, as the map.
use std::collections::{BTreeMap, HashMap};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use bevy::world_serialization::WorldAssetRoot;
use fb_sim::scene::{Finish, Form, LookOut, Part, SceneItem, Tint};
use lightyear::prelude::*;

use crate::game::Map;
use crate::render::surface::{Kind, Spec, SurfaceMaterial, Surfaces};
use crate::view::{frame_tick, hex};

/// Tone and opacity are drawn in steps of 1/STEPS (one material per step).
const STEPS: f32 = 16.0;

/// A special of the map (its scene item), with the entities of its pieces by part.
#[derive(Component)]
pub struct SpecialRoot {
    pub item: usize,
    pub slots: Vec<Vec<Entity>>,
}

#[derive(Component)]
pub struct SpecialPiece;

/// A map primitive, with what it is painted with (specials may tint it).
#[derive(Component)]
pub struct MapPrim(pub Spec);

#[derive(Resource, Default)]
pub struct SpecialCache {
    generation: u32,
    meshes: HashMap<(usize, usize), Handle<Mesh>>,
    mats: HashMap<(usize, usize, i8, i8), PieceMat>,
    pictures: HashMap<(usize, usize), Handle<Image>>,
    tints: HashMap<(String, &'static str, i8), Handle<SurfaceMaterial>>,
    out: LookOut,
}

#[derive(Clone, PartialEq)]
enum PieceMat {
    Plain(Handle<StandardMaterial>),
    Surface(Handle<SurfaceMaterial>),
}

/// A cylinder as three.js builds it (the first segment faces +z): hexagonal tiles line up.
pub fn cyl_mesh(r: f32, h: f32, seg: u32) -> Mesh {
    Cylinder::new(r, h)
        .mesh()
        .resolution(seg)
        .build()
        .rotated_by(Quat::from_rotation_y(-core::f32::consts::FRAC_PI_2))
}

fn form_mesh(form: Form) -> Option<Mesh> {
    let f = |v: f64| v as f32;
    Some(match form {
        Form::Box([x, y, z]) => Cuboid::new(f(x), f(y), f(z)).into(),
        Form::Cyl([r, h, seg]) => cyl_mesh(f(r), f(h), seg as u32),
        Form::Sphere(r) => Sphere::new(f(r)).mesh().uv(32, 18),
        Form::Torus(major, minor) => Torus {
            minor_radius: f(minor),
            major_radius: f(major),
        }
        .mesh()
        .build()
        .rotated_by(Quat::from_rotation_x(core::f32::consts::FRAC_PI_2)),
        Form::Ring(inner, outer) => Annulus::new(f(inner), f(outer)).mesh().resolution(32).build(),
        Form::Sash(r) => CircularSector::new(f(r), core::f32::consts::FRAC_PI_2)
            .mesh()
            .resolution(32)
            .build()
            .rotated_by(Quat::from_rotation_z(core::f32::consts::FRAC_PI_2))
            .translated_by(Vec3::X * f(r)),
        Form::Swirl(r) | Form::Rings(r) => Circle::new(f(r)).mesh().resolution(48).build(),
        Form::Arrow => crate::render::portal::arrow(),
        Form::Plane(w, h) | Form::Label(w, h, _) => Rectangle::new(f(w), f(h)).into(),
        Form::Model(_) => return None,
    })
}

fn flat(form: Form) -> bool {
    matches!(
        form,
        Form::Ring(..)
            | Form::Sash(_)
            | Form::Swirl(_)
            | Form::Rings(_)
            | Form::Arrow
            | Form::Plane(..)
            | Form::Label(..)
    )
}

/// Whether a part is lit and drawn on a surface (portal pictures, flat colours and light are not).
fn lit(part: &Part) -> bool {
    !matches!(part.finish, Finish::Flat | Finish::Light) && !matches!(part.form, Form::Swirl(_) | Form::Rings(_))
}

fn step(v: f64) -> i8 {
    (v as f32 * STEPS).round().clamp(-STEPS, STEPS) as i8
}

fn part_material(part: &Part, tone: i8, alpha: i8) -> StandardMaterial {
    let [a, b] = part.colors.map(|c| LinearRgba::from(hex(c)));
    let k = tone as f32 / STEPS;
    let mut c = if k >= 0.0 {
        a.mix(&b, k)
    } else {
        let d = 1.0 + k;
        LinearRgba::new(a.red * d, a.green * d, a.blue * d, a.alpha)
    };
    c.alpha *= alpha as f32 / STEPS;
    let mut m = StandardMaterial {
        base_color: c.into(),
        perceptual_roughness: 0.6,
        alpha_mode: if c.alpha < 0.999 {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
        ..default()
    };
    match part.finish {
        Finish::Matte => m.perceptual_roughness = 0.8,
        Finish::Glossy => m.perceptual_roughness = 0.35,
        Finish::Metal => {
            m.metallic = 0.8;
            m.perceptual_roughness = 0.3;
        }
        Finish::Glass => {
            m.perceptual_roughness = 0.08;
            m.reflectance = 0.8;
        }
        Finish::Glow => m.emissive = c * 1.5,
        Finish::Flat => m.unlit = true,
        Finish::Light => {
            m.unlit = true;
            m.alpha_mode = AlphaMode::Add;
        }
    }
    if flat(part.form) || part.finish == Finish::Flat {
        m.cull_mode = None::<Face>;
        m.double_sided = true;
    }
    m
}

type Pieces<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Visibility,
        Option<&'static mut MeshMaterial3d<StandardMaterial>>,
        Option<&'static mut MeshMaterial3d<SurfaceMaterial>>,
    ),
    (With<SpecialPiece>, Without<SpecialRoot>),
>;

#[allow(clippy::too_many_arguments)]
pub fn pose_specials(
    mut commands: Commands,
    map: Option<Res<Map>>,
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    assets: Res<AssetServer>,
    mut cache: ResMut<SpecialCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut roots: Query<(Entity, &mut SpecialRoot)>,
    mut pieces: Pieces,
    prims: Query<(&crate::view::MapPiece, &MapPrim, &Children), Without<SpecialPiece>>,
    mut levels: Query<&mut MeshMaterial3d<SurfaceMaterial>, With<crate::view::PrimLevel>>,
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut surface_mats: ResMut<Assets<SurfaceMaterial>>,
) {
    let Some(map) = map else { return };
    let cache = &mut *cache;
    if cache.generation != map.generation {
        cache.generation = map.generation;
        cache.meshes.clear();
        cache.mats.clear();
        cache.pictures.clear();
        cache.tints.clear();
    }
    let t = map.time(frame_tick(&timeline, &fixed) - 1.0);
    let mut tints: BTreeMap<u32, Tint> = BTreeMap::new();
    for (root_e, mut root) in &mut roots {
        let Some(SceneItem::Special {
            parts,
            pieces: still,
            look,
            ..
        }) = map.scene.items.get(root.item)
        else {
            continue;
        };
        let out = &mut cache.out;
        out.pieces.clear();
        out.tints.clear();
        match look {
            Some(l) => (l.0)(&map.world, t, out),
            None => out.pieces.extend_from_slice(still),
        }
        for tint in out.tints.drain(..) {
            tints.insert(tint.node, tint);
        }
        root.slots.resize_with(parts.len(), Vec::new);
        let mut used = vec![0usize; parts.len()];
        for p in &out.pieces {
            let pi = p.part as usize;
            let Some(part) = parts.get(pi) else { continue };
            let tf = Transform {
                translation: p.pos.as_vec3(),
                rotation: Quat::from_euler(EulerRot::XYZ, p.rot.x as f32, p.rot.y as f32, p.rot.z as f32),
                scale: p.scale.max(0.0) as f32 * p.axes.as_vec3(),
            };
            let vis = if p.scale > 0.0 {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            let mat = match part.form {
                Form::Model(_) => None,
                _ => {
                    let key = (root.item, pi, step(p.tone), step(p.alpha.clamp(0.0, 1.0)));
                    Some(
                        cache
                            .mats
                            .entry(key)
                            .or_insert_with(|| {
                                let mut m = part_material(part, key.2, key.3);
                                let a = m.base_color.alpha();
                                match part.form {
                                    // A board with its emoji drawn on (the board's colour in the picture).
                                    Form::Label(_, _, text) if !text.is_empty() => {
                                        m.base_color_texture =
                                            Some(images.add(crate::render::emoji::board(text, m.base_color)));
                                        m.base_color = Color::WHITE.with_alpha(a);
                                    }
                                    // A portal's picture; its tone brightens it (the flash of a trip).
                                    Form::Swirl(_) | Form::Rings(_) => {
                                        let tex = cache
                                            .pictures
                                            .entry((root.item, pi))
                                            .or_insert_with(|| {
                                                let c = hex(part.colors[0]);
                                                images.add(match part.form {
                                                    Form::Swirl(_) => crate::render::portal::swirl(c),
                                                    _ => crate::render::portal::rings(c),
                                                })
                                            })
                                            .clone();
                                        m.base_color_texture = Some(tex);
                                        let k = 1.0 + 1.5 * key.2.max(0) as f32 / STEPS;
                                        m.base_color = LinearRgba::new(k, k, k, a).into();
                                    }
                                    _ => {}
                                }
                                if lit(part) {
                                    let kind = Some(part.surface.and_then(Kind::of).unwrap_or(Kind::Plastic));
                                    PieceMat::Surface(surfaces.material_from(
                                        m,
                                        kind,
                                        None,
                                        false,
                                        None,
                                        &mut images,
                                        &mut surface_mats,
                                    ))
                                } else {
                                    PieceMat::Plain(materials.add(m))
                                }
                            })
                            .clone(),
                    )
                }
            };
            let n = used[pi];
            used[pi] += 1;
            if let Some(&e) = root.slots[pi].get(n) {
                if let Ok((mut cur, mut v, plain, surface)) = pieces.get_mut(e) {
                    if *cur != tf {
                        *cur = tf;
                    }
                    v.set_if_neq(vis);
                    match (mat, plain, surface) {
                        (Some(PieceMat::Plain(h)), Some(mut m), _) if m.0 != h => m.0 = h,
                        (Some(PieceMat::Surface(h)), _, Some(mut m)) if m.0 != h => m.0 = h,
                        _ => {}
                    }
                }
                continue;
            }
            let mut e = commands.spawn((SpecialPiece, tf, vis, ChildOf(root_e)));
            match (part.form, mat) {
                (Form::Model(name), _) => {
                    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/{name}.glb")));
                    e.insert(WorldAssetRoot(scene));
                }
                (form, Some(mat)) => {
                    let mesh = cache
                        .meshes
                        .entry((root.item, pi))
                        .or_insert_with(|| meshes.add(form_mesh(form).unwrap_or_else(|| Cuboid::default().into())))
                        .clone();
                    match mat {
                        PieceMat::Plain(h) => e.insert((Mesh3d(mesh), MeshMaterial3d(h))),
                        PieceMat::Surface(h) => e.insert((Mesh3d(mesh), MeshMaterial3d(h))),
                    };
                    if matches!(part.finish, Finish::Flat | Finish::Light) {
                        e.insert(NotShadowCaster);
                    }
                }
                _ => {}
            }
            root.slots[pi].push(e.id());
        }
        // Pieces not placed this frame are hidden.
        for (pi, slots) in root.slots.iter().enumerate() {
            for &e in &slots[used[pi]..] {
                if let Ok((_, mut v, _, _)) = pieces.get_mut(e) {
                    v.set_if_neq(Visibility::Hidden);
                }
            }
        }
    }
    if tints.is_empty() {
        return;
    }
    for (piece, prim, children) in &prims {
        let Some(tint) = tints.get(&piece.node) else { continue };
        let k = step(tint.k);
        let h = cache
            .tints
            .entry((format!("{:?}", prim.0), tint.to, k))
            .or_insert_with(|| {
                let to = LinearRgba::from(hex(tint.to));
                let f = k as f32 / STEPS;
                let mut spec = prim.0.clone();
                match &mut spec.paint {
                    Some(p) => {
                        p.c1 = p.c1.mix(&to, f);
                        p.c2 = p.c2.mix(&to, f);
                    }
                    None => spec.color = spec.color.mix(&to, f),
                }
                surfaces.material(&spec, &mut images, &mut surface_mats)
            })
            .clone();
        for c in children {
            if let Ok(mut mat) = levels.get_mut(*c)
                && mat.0 != h
            {
                mat.0 = h.clone();
            }
        }
    }
}
