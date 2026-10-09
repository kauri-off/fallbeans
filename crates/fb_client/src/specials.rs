//! Drawing the specials of a map (portal rings, hex tiles, glass panes…): the pieces their looks place
//! every frame, and the map's primitives they tint. Lit pieces are drawn on surfaces, as the map.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use bevy::world_serialization::WorldAssetRoot;
use fb_shared::Rgb;
use fb_sim::looks::ResolvedLook;
use fb_sim::scene::{Finish, Form, LookOut, Part, SceneItem, Tint};
use lightyear::prelude::*;

use crate::game::Map;
use crate::render::meshes::{soft_box, soft_cylinder};
use crate::render::portal::PortalMaterial;
use crate::render::surface::{Kind, Spec, SurfaceMaterial, Surfaces};
use crate::view::{color, frame_tick};

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

/// A map primitive, with what it is painted with and its untinted material (specials may tint it).
#[derive(Component)]
pub struct MapPrim(pub Spec, pub Handle<SurfaceMaterial>);

#[derive(Resource, Default)]
pub struct SpecialCache {
    generation: u32,
    meshes: HashMap<(usize, usize), Handle<Mesh>>,
    /// `look_of` ids by item and part: identical parts share materials (one draw).
    looks: HashMap<(usize, usize), u32>,
    look_ids: HashMap<String, u32>,
    /// By look, tone step, opacity step.
    mats: HashMap<(u32, i8, i8), PieceMat>,
    /// Tinted materials by the primitive's own material (identical primitives share it), colour, step.
    tints: HashMap<(AssetId<SurfaceMaterial>, Rgb, i8), Handle<SurfaceMaterial>>,
    /// Nodes tinted the frame before: one the looks no longer tint gets its own material back.
    tinted: BTreeSet<u32>,
    out: LookOut,
}

#[derive(Clone, PartialEq)]
enum PieceMat {
    Plain(Handle<StandardMaterial>),
    Surface(Handle<SurfaceMaterial>),
    Portal(Handle<PortalMaterial>),
}

/// Segments per rounded quarter of the specials' soft boxes and cylinders: they have no levels of detail, and
/// some come by the hundred (hexagonal tiles).
const SOFT_ARCS: u32 = 2;

fn form_mesh(form: Form) -> Option<Mesh> {
    let f = |v: f64| v as f32;
    Some(match form {
        Form::Box([x, y, z]) => soft_box(Vec3::new(f(x), f(y), f(z)), SOFT_ARCS),
        // (The first segment faces +z: hexagonal tiles line up.)
        Form::Cyl([r, h, seg]) => soft_cylinder(f(r), f(h), seg as u32, SOFT_ARCS),
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

/// Everything `part_material`, the boards and the portal discs read of a part (the round's look is the same for all).
fn look_of(part: &Part) -> String {
    let form = match part.form {
        Form::Label(_, _, text) => format!("label {text}"),
        Form::Swirl(_) => "swirl".into(),
        Form::Rings(_) => "rings".into(),
        f if flat(f) => "flat".into(),
        _ => "solid".into(),
    };
    format!(
        "{:?} {:?} {:?} {:?} {form}",
        part.colors, part.pal, part.finish, part.surface
    )
}

/// Whether a part is lit and drawn on a surface (portal discs, flat colours and light are not).
fn lit(part: &Part) -> bool {
    !matches!(part.finish, Finish::Flat | Finish::Light) && !matches!(part.form, Form::Swirl(_) | Form::Rings(_))
}

fn step(v: f64) -> i8 {
    (v as f32 * STEPS).round().clamp(-STEPS, STEPS) as i8
}

fn part_material(part: &Part, look: &ResolvedLook, tone: i8, alpha: i8) -> StandardMaterial {
    let [mut a, b] = part.colors.map(|c| LinearRgba::from(color(c)));
    if let Some([c, _]) = part.pal.and_then(|p| look.repaint(p)) {
        a = LinearRgba::from(color(c));
    }
    let k = f32::from(tone) / STEPS;
    let mut c = if k >= 0.0 {
        a.mix(&b, k)
    } else {
        let d = 1.0 + k;
        LinearRgba::new(a.red * d, a.green * d, a.blue * d, a.alpha)
    };
    c.alpha *= f32::from(alpha) / STEPS;
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

/// A portal's disc (`render::portal`), for a part that is one: its colour and kind, the flash of a trip (the
/// tone) and its opacity.
fn portal_disc(part: &Part, tone: i8, alpha: i8) -> Option<PortalMaterial> {
    let exit = match part.form {
        Form::Swirl(_) => false,
        Form::Rings(_) => true,
        _ => return None,
    };
    let c = color(part.colors[0]);
    let a = c.alpha() * f32::from(alpha) / STEPS;
    let phase = part.colors[0].bytes().into_iter().map(u32::from).sum::<u32>() % 7;
    Some(PortalMaterial::new(
        c,
        exit,
        f32::from(tone.max(0)) / STEPS,
        a,
        phase as f32,
    ))
}

type Pieces<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Visibility,
        Option<&'static mut MeshMaterial3d<StandardMaterial>>,
        Option<&'static mut MeshMaterial3d<SurfaceMaterial>>,
        Option<&'static mut MeshMaterial3d<PortalMaterial>>,
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
    (mut materials, mut portals): (ResMut<Assets<StandardMaterial>>, ResMut<Assets<PortalMaterial>>),
    mut roots: Query<(Entity, &mut SpecialRoot)>,
    mut pieces: Pieces,
    prims: Query<(&crate::view::MapPiece, &MapPrim, &Children), Without<SpecialPiece>>,
    mut levels: Query<&mut MeshMaterial3d<SurfaceMaterial>, (With<crate::view::PrimLevel>, Without<SpecialPiece>)>,
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut surface_mats: ResMut<Assets<SurfaceMaterial>>,
) {
    let Some(map) = map else { return };
    let cache = &mut *cache;
    if cache.generation != map.generation {
        cache.generation = map.generation;
        cache.meshes.clear();
        cache.looks.clear();
        cache.look_ids.clear();
        cache.mats.clear();
        cache.tints.clear();
        cache.tinted.clear();
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
            Some(l) => l.run(&*map.spec.logic, &map.world, t, out),
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
                    let n = cache.look_ids.len() as u32;
                    let ids = &mut cache.look_ids;
                    let look = *cache
                        .looks
                        .entry((root.item, pi))
                        .or_insert_with(|| *ids.entry(look_of(part)).or_insert(n));
                    let key = (look, step(p.tone), step(p.alpha.clamp(0.0, 1.0)));
                    Some(
                        cache
                            .mats
                            .entry(key)
                            .or_insert_with(|| {
                                // A portal's disc: its tone is the flash of a trip.
                                if let Some(disc) = portal_disc(part, key.1, key.2) {
                                    return PieceMat::Portal(portals.add(disc));
                                }
                                let mut m = part_material(part, &map.look, key.1, key.2);
                                // A board with its emoji drawn on (the board's colour in the picture).
                                if let Form::Label(_, _, text) = part.form
                                    && !text.is_empty()
                                {
                                    let a = m.base_color.alpha();
                                    m.base_color_texture =
                                        Some(images.add(crate::render::emoji::board(text, m.base_color)));
                                    m.base_color = Color::WHITE.with_alpha(a);
                                }
                                if lit(part) {
                                    let kind = Some(part.surface.map_or(Kind::Plastic, Kind::from));
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
                if let Ok((mut cur, mut v, plain, surface, portal)) = pieces.get_mut(e) {
                    if *cur != tf {
                        *cur = tf;
                    }
                    v.set_if_neq(vis);
                    match (mat, plain, surface, portal) {
                        (Some(PieceMat::Plain(h)), Some(mut m), _, _) if m.0 != h => m.0 = h,
                        (Some(PieceMat::Surface(h)), _, Some(mut m), _) if m.0 != h => m.0 = h,
                        (Some(PieceMat::Portal(h)), _, _, Some(mut m)) if m.0 != h => m.0 = h,
                        _ => {}
                    }
                }
                continue;
            }
            let mut e = commands.spawn((SpecialPiece, tf, vis, ChildOf(root_e)));
            match (part.form, mat) {
                (Form::Model(name), _) => {
                    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/{name}.glb")));
                    e.insert((WorldAssetRoot(scene), crate::render::props::Prop::special(name)));
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
                        PieceMat::Portal(h) => e.insert((Mesh3d(mesh), MeshMaterial3d(h))),
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
                if let Ok((_, mut v, ..)) = pieces.get_mut(e) {
                    v.set_if_neq(Visibility::Hidden);
                }
            }
        }
    }
    if tints.is_empty() && cache.tinted.is_empty() {
        return;
    }
    for (piece, prim, children) in &prims {
        let h = match tints.get(&piece.node) {
            Some(tint) => {
                let k = step(tint.k);
                cache
                    .tints
                    .entry((prim.1.id(), tint.to, k))
                    .or_insert_with(|| {
                        let to = LinearRgba::from(color(tint.to));
                        let f = f32::from(k) / STEPS;
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
                    .clone()
            }
            None if cache.tinted.contains(&piece.node) => prim.1.clone(),
            None => continue,
        };
        for c in children {
            if let Ok(mut mat) = levels.get_mut(*c)
                && mat.0 != h
            {
                mat.0 = h.clone();
            }
        }
    }
    cache.tinted.clear();
    cache.tinted.extend(tints.keys().copied());
}
