//! Drawing: the map from its `SceneDesc` (primitives, the static ones merged by cell and material; glTF models
//! and specials), beans, bonuses, camera.
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use bevy::camera::primitives::MeshAabb;
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy::world_serialization::WorldAssetRoot;
use fb_arena::ArenaKind;
use fb_net::*;
use fb_shared::Rgb;
use fb_sim::V3;
use fb_sim::looks::{Pattern, ResolvedLook};
use fb_sim::math::Affine;
use fb_sim::nodes::Nodes;
use fb_sim::physics::Power;
use fb_sim::scene::Palette;
use fb_sim::scene::{LookOut, PrimKind, SceneItem};
use lightyear::prelude::*;

use crate::beans;
use crate::camera::{PITCH_MAX, PITCH_MIN};
use crate::game::{CameraAngles, Gate, Map};
use crate::render::ao::{self, AoKit, BakedAo, Solids};
use crate::render::meshes::{self, BandPad, Candidate, LodBand};
use crate::render::props::Prop;
use crate::render::quality::Quality;
use crate::render::surface::{Kind, Paint, Spec, SurfaceMaterial, Surfaces};
use crate::settings::Controls;
use crate::specials::{MapPrim, SpecialCache, SpecialRoot, pose_specials};

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(1.0, 0.85, 0.95)));
        app.add_systems(Startup, setup_camera);
        app.add_systems(
            PostUpdate,
            (
                spawn_map,
                pose_map,
                pose_specials,
                beans::spawn_beans,
                beans::place_beans,
                beans::dress_beans,
                beans::animate_beans,
                place_bonuses,
            )
                .chain()
                .before(TransformSystems::Propagate),
        );
        app.init_resource::<Spectate>();
        app.init_resource::<SpecialCache>();
        app.init_resource::<Drawn>();
        app.add_systems(Update, (spectate, mouse_look).chain());
    }
}

/// A power's colour (bonus bubbles, the aura).
pub const fn power_color(p: Power) -> Color {
    match p {
        Power::Giant => Color::srgb_u8(0xff, 0x6f, 0x91),
        Power::Jump => Color::srgb_u8(0x58, 0xd6, 0x8d),
        Power::Speed => Color::srgb_u8(0xff, 0xd2, 0x3f),
    }
}

/// A primitive's mesh at one level of detail (a child of its `MapPrim`).
#[derive(Component)]
pub struct PrimLevel;

/// Entities of the drawn map (despawned with it), each following one node.
#[derive(Component)]
pub struct MapPiece {
    pub node: u32,
}

#[derive(Component)]
pub struct MapRoot(pub u32);

#[derive(Component)]
struct BonusView(u32);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum BonusPart {
    Bubble,
    Card,
    Ring,
}

#[derive(Component)]
pub struct MainCamera;

/// Who the camera follows while the player has no bean in a round (finished, out, or not taking part).
#[derive(Resource, Default)]
pub struct Spectate {
    /// None: the overview of the map.
    pub target: Option<u32>,
    /// The player picked the target; until then (or until it leaves) the first bean in play is followed.
    manual: bool,
    generation: u32,
}

pub fn setup_camera(mut commands: Commands, offscreen: Option<Res<crate::Offscreen>>) {
    let mut cam = commands.spawn((
        Camera3d::default(),
        MainCamera,
        // (Other beans' sounds are placed against it: `audio.rs`.)
        SpatialListener::new(4.0),
        Transform::from_xyz(0.0, 14.0, 22.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));
    if let Some(o) = offscreen {
        cam.insert((bevy::camera::RenderTarget::Image(o.0.clone().into()), IsDefaultUiCamera));
    }
}

pub fn color(c: Rgb) -> Color {
    let [r, g, b] = c.bytes();
    Color::srgba_u8(r, g, b, c.alpha)
}

fn mat4(m: &Affine) -> Mat4 {
    Mat4::from(bevy::math::Affine3A::from_cols_array(
        &m.to_cols_array().map(|v| v as f32),
    ))
}

/// The world bounds (min, max) of a box of half size `half` under `m`.
fn world_bounds(m: &Mat4, half: Vec3) -> (Vec3, Vec3) {
    let mut lo = Vec3::INFINITY;
    let mut hi = Vec3::NEG_INFINITY;
    for i in 0..8u32 {
        let side = |bit: u32, h: f32| if i & bit == 0 { -h } else { h };
        let p = m.transform_point3(Vec3::new(side(1, half.x), side(2, half.y), side(4, half.z)));
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo, hi)
}

/// What `spawn_map` drew of the map shown: the static primitives merged by cell, material and levels of
/// detail, so that `pose_map` can take a group apart should one of its nodes move after all.
#[derive(Resource, Default)]
pub struct Drawn {
    generation: Option<u32>,
    root: Option<Entity>,
    /// By scene item: a primitive's paint, material and lift step.
    prims: Vec<Option<(Spec, Handle<SurfaceMaterial>, u32)>>,
    /// By scene item: the merged group it is drawn in.
    group_of: Vec<Option<usize>>,
    /// Each group's entities (one per level of detail) and items.
    groups: Vec<(Vec<Entity>, Vec<usize>)>,
    /// By node: a primitive on it is drawn merged.
    merged: Vec<bool>,
    /// The shared meshes of the primitives drawn by themselves.
    meshes: HashMap<LevelKey, Handle<Mesh>>,
}

/// A primitive's mesh at a level: identical primitives share it.
type LevelKey = (PrimKind, [u64; 3], u32);

fn level_key(kind: PrimKind, dims: [f64; 3], band: usize) -> LevelKey {
    (kind, dims.map(f64::to_bits), meshes::level_id(kind, dims, band))
}

/// A primitive `spawn_map` may merge: its levels' world matrix (the lift in it) and its world bounds; and whether
/// it moves and shades what is near it (`ao::Movers`).
struct Placed {
    item: usize,
    kind: PrimKind,
    dims: [f64; 3],
    world: Mat4,
    lo: Vec3,
    hi: Vec3,
    mover: bool,
}

/// Nodes that never move, turn or vanish (`fixed`), and of them those never tinted either (`still`): no moving
/// collider is on them, and neither the movers nor the looks change them or a node above them at any time of the
/// round tried (`PROBES`). The world is not touched: the movers pose a copy. (What the trial misses, `pose_map`
/// still catches.)
fn still_nodes(map: &Map) -> (Vec<bool>, Vec<bool>) {
    let base = &map.render;
    let n = base.0.len();
    let mut moves = vec![false; n];
    let mut tinted = vec![false; n];
    // Only what primitives and models hang from is compared.
    let mut watched = vec![false; n];
    for item in &map.scene.items {
        let (SceneItem::Prim { node, .. } | SceneItem::Model { node, .. }) = item else {
            continue;
        };
        let mut at = Some(*node);
        while let Some(i) = at {
            match watched.get_mut(i as usize) {
                Some(w) if !*w => *w = true,
                _ => break,
            }
            at = base.0[i as usize].parent;
        }
    }
    let watch: Vec<usize> = (0..n).filter(|&i| watched[i]).collect();
    for &c in &map.world.dynamic {
        if let Some(col) = map.world.colliders.get(c as usize)
            && let Some(m) = moves.get_mut(col.node as usize)
        {
            *m = true;
        }
    }
    let looks: Vec<_> = map
        .scene
        .items
        .iter()
        .filter_map(|item| match item {
            SceneItem::Special { look: Some(l), .. } => Some(l),
            _ => None,
        })
        .collect();
    let mut probe = base.clone();
    let mut out = LookOut::default();
    for t in PROBES
        .iter()
        .flat_map(|&(from, to, step)| (0..((to - from) / step) as usize).map(move |k| from + k as f64 * step))
    {
        map.world.pose_locals(t, &mut probe, &*map.spec.logic);
        for &i in &watch {
            let (a, b) = (&probe.0[i], &base.0[i]);
            if a.pos != b.pos || a.rot != b.rot || a.scale != b.scale || a.visible != b.visible {
                moves[i] = true;
            }
        }
        for l in &looks {
            out.pieces.clear();
            out.tints.clear();
            l.run(&*map.spec.logic, &map.world, t, &mut out);
            for tint in &out.tints {
                if let Some(m) = tinted.get_mut(tint.node as usize) {
                    *m = true;
                }
            }
        }
    }
    // Hidden ones stay out too, and what hangs from a moving (tinted) node moves (is tinted); parents come before
    // children.
    for (i, node) in base.0.iter().enumerate() {
        let up = |of: &[bool]| node.parent.is_some_and(|p| of.get(p as usize) == Some(&true));
        let (moved, tint) = (up(&moves[..]), up(&tinted[..]));
        moves[i] |= !node.visible || moved;
        tinted[i] |= tint;
    }
    let still = moves.iter().zip(&tinted).map(|(m, t)| !m && !t).collect();
    (still, moves.into_iter().map(|m| !m).collect())
}

/// The times of the round `still_nodes` tries (from, to, step; s): finely through the start, coarser after.
const PROBES: [(f64, f64, f64); 2] = [(-10.0, 60.0, 0.25), (60.0, 300.0, 1.0)];

/// What the merged groups' meshes are baked again with, their ambient occlusion (`ao.rs`): the map's solids, the
/// solid of each scene item, the bakes.
struct Shading<'a> {
    solids: &'a Arc<Solids>,
    solid_of: &'a [Option<u32>],
    baked: &'a mut BakedAo,
}

/// A merged group of static primitives: one mesh per level of detail around the group's centre, its
/// distances padded by how far its pieces lie from the centre (`BandPad`) and taken at its biggest piece's
/// size (`meshes::level_class` keeps the sizes close), each baked again with its occlusion in the background.
/// None: a piece that does not merge.
#[allow(clippy::too_many_arguments)]
fn spawn_group(
    commands: &mut Commands,
    assets: &mut Assets<Mesh>,
    cpu: &mut HashMap<LevelKey, Arc<Mesh>>,
    group: &[&Placed],
    mat: &Handle<SurfaceMaterial>,
    root: Entity,
    lod_k: f32,
    shading: &mut Shading,
) -> Option<Vec<Entity>> {
    let first = group.first()?;
    let (lo, hi) = group.iter().fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(lo, hi), p| {
        (lo.min(p.lo), hi.max(p.hi))
    });
    let origin = (lo + hi) / 2.0;
    let pad = group
        .iter()
        .map(|p| p.world.w_axis.truncate().distance(origin))
        .fold(0.0, f32::max);
    let r = group.iter().map(|p| meshes::radius(p.kind, p.dims)).fold(0.0, f32::max);
    let levels = meshes::prim_levels(first.kind, first.dims);
    let mut made = Vec::with_capacity(levels.len());
    for band in &levels {
        for p in group {
            cpu.entry(level_key(p.kind, p.dims, band.first))
                .or_insert_with(|| Arc::new(meshes::prim(p.kind, p.dims, band.first)));
        }
        let pieces: Vec<(&Mesh, Mat4)> = group
            .iter()
            .filter_map(|p| cpu.get(&level_key(p.kind, p.dims, band.first)).map(|m| (&**m, p.world)))
            .collect();
        let mesh = meshes::merge(&pieces, origin, true)?;
        let pieces = group
            .iter()
            .filter_map(|p| {
                let key = level_key(p.kind, p.dims, band.first);
                Some(ao::Piece {
                    mesh: cpu.get(&key)?.clone(),
                    world: p.world,
                    own: shading.solid_of.get(p.item).copied().flatten(),
                    key: ao::piece_key(shading.solids, key, &p.world, band.first),
                })
            })
            .collect();
        let bake = ao::Bake {
            pieces,
            origin,
            band: band.first,
        };
        made.push((mesh, bake));
    }
    let single = levels.len() == 1;
    let mut out = Vec::with_capacity(made.len());
    for (band, (mesh, bake)) in levels.into_iter().zip(made) {
        let aabb = mesh.compute_aabb();
        let mesh = assets.add(mesh);
        shading.baked.start(mesh.id(), bake, shading.solids.clone());
        let mut e = commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(origin),
            Visibility::default(),
            ChildOf(root),
        ));
        if let Some(aabb) = aabb {
            e.insert(aabb);
        }
        if !single {
            let band = LodBand { r, ..band };
            e.insert((band, BandPad(pad), band.range_padded(lod_k, pad)));
        }
        out.push(e.id());
    }
    Some(out)
}

/// A primitive drawn by itself: its levels of detail are children (shown by the camera's distance) of an entity
/// that follows its node (returned).
fn spawn_prim(
    commands: &mut Commands,
    drawn: &mut Drawn,
    assets: &mut Assets<Mesh>,
    map: &Map,
    item: usize,
    lod_k: f32,
    shown: bool,
) -> Option<Entity> {
    let (Some(root), Some(SceneItem::Prim { node, kind, dims, .. }), Some(Some((spec, mat, lift)))) =
        (drawn.root, map.scene.items.get(item), drawn.prims.get(item))
    else {
        return None;
    };
    let piece = commands
        .spawn((
            MapPrim(spec.clone(), mat.clone()),
            MapPiece { node: *node },
            Transform::from_matrix(mat4(&map.render.get(*node).world)),
            if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
            ChildOf(root),
        ))
        .id();
    let lift = Transform::from_xyz(0.0, *lift as f32 * meshes::LIFT, 0.0);
    let levels = meshes::prim_levels(*kind, *dims);
    let single = levels.len() == 1;
    for band in levels {
        let mesh = drawn
            .meshes
            .entry(level_key(*kind, *dims, band.first))
            .or_insert_with(|| assets.add(meshes::prim(*kind, *dims, band.first)))
            .clone();
        let mut child = commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mat.clone()),
            PrimLevel,
            lift,
            ChildOf(piece),
        ));
        if !single {
            child.insert((band, band.range(lod_k)));
        }
    }
    Some(piece)
}

/// A merged group with a node that moved, appeared or vanished after all (the trial of `still_nodes` missed
/// it) is drawn piece by piece from now on.
fn split_moved(
    commands: &mut Commands,
    drawn: &mut Drawn,
    assets: &mut Assets<Mesh>,
    map: &Map,
    posed: &Posed,
    lod_k: f32,
) {
    let changed = |i: usize| posed.moved.get(i) == Some(&true) || posed.flipped.get(i) == Some(&true);
    if !drawn.merged.iter().enumerate().any(|(i, &m)| m && changed(i)) {
        return;
    }
    let node_of = |item: usize| match map.scene.items.get(item) {
        Some(SceneItem::Prim { node, .. }) => Some(*node as usize),
        _ => None,
    };
    let mut hit: Vec<usize> = Vec::new();
    for (item, g) in drawn.group_of.iter().enumerate() {
        if let Some(g) = *g
            && node_of(item).is_some_and(changed)
            && !hit.contains(&g)
        {
            hit.push(g);
        }
    }
    for g in hit {
        let Some((entities, items)) = drawn.groups.get_mut(g).map(core::mem::take) else {
            continue;
        };
        warn!("map: a merged group of {} primitives moved, drawn apart", items.len());
        for e in entities {
            commands.entity(e).despawn();
        }
        for item in items {
            drawn.group_of[item] = None;
            let shown = node_of(item).is_none_or(|n| posed.shown.get(n) != Some(&false));
            spawn_prim(commands, drawn, assets, map, item, lod_k, shown);
        }
    }
    drawn.merged.fill(false);
    for (item, g) in drawn.group_of.iter().enumerate() {
        if g.is_some()
            && let Some(n) = node_of(item)
            && let Some(m) = drawn.merged.get_mut(n)
        {
            *m = true;
        }
    }
}

fn spawn_map(
    mut commands: Commands,
    map: Option<Res<Map>>,
    roots: Query<(Entity, &MapRoot)>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut surfaces: ResMut<Surfaces>,
    mut images: ResMut<Assets<Image>>,
    mut surface_mats: ResMut<Assets<SurfaceMaterial>>,
    quality: Option<Res<Quality>>,
    display: Res<crate::settings::Display>,
    mut drawn: ResMut<Drawn>,
    mut ao_kit: AoKit,
) {
    let Some(map) = map else { return };
    if roots.iter().any(|(_, r)| r.0 == map.generation) {
        return;
    }
    for (e, _) in &roots {
        commands.entity(e).despawn();
    }
    let root = commands
        .spawn((MapRoot(map.generation), Transform::default(), Visibility::default()))
        .id();
    let lod_k = meshes::lod_k(display.fov, quality.as_ref().map(|q| q.preset));
    let items = map.scene.items.len();
    let drawn = &mut *drawn;
    *drawn = Drawn {
        generation: Some(map.generation),
        root: Some(root),
        prims: vec![None; items],
        group_of: vec![None; items],
        merged: vec![false; map.render.0.len()],
        ..default()
    };
    let (still, fixed) = still_nodes(&map);
    // What hides the sky from the rest of the map (`ao.rs`): what stands still and is not see-through, the
    // primitives as themselves (by scene item), the models as their parts; and what moves, as capsules.
    let mut solids: Vec<ao::Solid> = Vec::new();
    let mut solid_of: Vec<Option<u32>> = vec![None; items];
    let mut movers: Vec<(Entity, ao::Capsule)> = Vec::new();
    let mut capsules: Vec<ao::Capsule> = Vec::new();
    // Primitives that may merge, with the candidates `meshes::groups` weighs.
    let mut placed: Vec<Placed> = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut material_ids: HashMap<AssetId<SurfaceMaterial>, usize> = HashMap::new();
    // Primitives that touch are lifted apart (by their transform: identical ones still share a mesh).
    let bounds: Vec<(Vec3, Vec3)> = map
        .scene
        .items
        .iter()
        .filter_map(|item| match item {
            SceneItem::Prim { node, kind, dims, .. } => Some(world_bounds(
                &mat4(&map.render.get(*node).world),
                meshes::half_extents(*kind, *dims),
            )),
            _ => None,
        })
        .collect();
    let mut lifts = meshes::lifts(&bounds).into_iter();
    let mut prim_bounds = bounds.iter();
    for (i, item) in map.scene.items.iter().enumerate() {
        let (node, child) = match item {
            SceneItem::Prim {
                node,
                kind,
                dims,
                pal,
                freq,
                surface,
                pattern,
            } => {
                let lift = lifts.next().unwrap_or(0);
                let (lo, hi) = prim_bounds.next().copied().unwrap_or_default();
                let spec = prim_spec(&map.look, *kind, *dims, *pal, *freq, *surface, *pattern);
                let mat = surfaces.material(&spec, &mut images, &mut surface_mats);
                let world = mat4(&map.render.get(*node).world)
                    * Mat4::from_translation(Vec3::new(0.0, lift as f32 * meshes::LIFT, 0.0));
                let opaque = spec.alpha == AlphaMode::Opaque;
                let fixed = fixed.get(*node as usize) == Some(&true);
                if fixed
                    && opaque
                    && let Some(s) = ao::Solid::new(ao::prim_shape(*kind, *dims), &world, 1.0)
                {
                    solid_of[i] = Some(solids.len() as u32);
                    solids.push(s);
                }
                let n = material_ids.len();
                candidates.push(Candidate {
                    at: (lo + hi) / 2.0,
                    material: *material_ids.entry(mat.id()).or_insert(n),
                    class: meshes::level_class(&meshes::prim_levels(*kind, *dims)),
                    // (Nothing a look tints, nothing see-through: a merged mesh is sorted as one.)
                    still: still.get(*node as usize) == Some(&true)
                        && spec.alpha == AlphaMode::Opaque
                        && meshes::frame(&world).is_some(),
                });
                placed.push(Placed {
                    item: i,
                    kind: *kind,
                    dims: *dims,
                    world,
                    lo,
                    hi,
                    mover: !fixed && opaque,
                });
                drawn.prims[i] = Some((spec, mat, lift));
                // (Drawn below: merged, or by itself.)
                continue;
            }
            SceneItem::Model { node, name, tint } => {
                let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/{name}.glb")));
                let n = map.render.get(*node);
                let piece = commands.spawn_empty().id();
                let prop = commands
                    .spawn((
                        WorldAssetRoot(scene),
                        Prop::new(*name, *tint, n.pos.x, n.pos.z),
                        Transform::default(),
                        Visibility::default(),
                        ChildOf(piece),
                    ))
                    .id();
                // Standing still, it shades the map, and the ground shades its foot; moving, what it passes.
                let world = mat4(&n.world);
                let parts = ao_kit.model(*name, &assets, &meshes);
                if fixed.get(*node as usize) == Some(&true) {
                    ao::model_solids(&parts, &world, &mut solids);
                    if ao::GROUNDED.contains(name) {
                        commands.entity(prop).insert(ao::Grounded(world.w_axis.y));
                    }
                } else {
                    ao::model_capsules(&parts, &mut capsules);
                    movers.extend(capsules.drain(..).map(|c| (piece, c)));
                }
                (*node, piece)
            }
            SceneItem::Special { node, .. } => {
                let root = SpecialRoot {
                    item: i,
                    slots: Vec::new(),
                };
                (*node, commands.spawn(root).id())
            }
        };
        commands.entity(child).insert((
            MapPiece { node },
            Transform::from_matrix(mat4(&map.render.get(node).world)),
            Visibility::default(),
            ChildOf(root),
        ));
    }
    // Static primitives merge by cell, material and levels of detail (one by itself too: its mesh is its own, to
    // bake its occlusion into); the rest are drawn by themselves.
    let solids = Arc::new(ao::Solids::new(solids));
    let mut shading = Shading {
        solids: &solids,
        solid_of: &solid_of,
        baked: &mut ao_kit.baked,
    };
    let mut cpu: HashMap<LevelKey, Arc<Mesh>> = HashMap::new();
    let mut alone = vec![true; placed.len()];
    for group in meshes::groups_of(&candidates, 1) {
        let members: Vec<&Placed> = group.iter().filter_map(|&c| placed.get(c)).collect();
        let Some(Some((_, mat, _))) = members.first().and_then(|p| drawn.prims.get(p.item)) else {
            continue;
        };
        let Some(entities) = spawn_group(
            &mut commands,
            &mut meshes,
            &mut cpu,
            &members,
            mat,
            root,
            lod_k,
            &mut shading,
        ) else {
            continue;
        };
        let g = drawn.groups.len();
        for p in &members {
            drawn.group_of[p.item] = Some(g);
            if let Some(SceneItem::Prim { node, .. }) = map.scene.items.get(p.item)
                && let Some(m) = drawn.merged.get_mut(*node as usize)
            {
                *m = true;
            }
        }
        drawn.groups.push((entities, members.iter().map(|p| p.item).collect()));
        for &c in &group {
            alone[c] = false;
        }
    }
    for (p, _) in placed.iter().zip(&alone).filter(|(_, a)| **a) {
        let piece = spawn_prim(&mut commands, drawn, &mut meshes, &map, p.item, lod_k, true);
        if let Some(piece) = piece.filter(|_| p.mover) {
            ao::prim_capsules(p.kind, p.dims, &mut capsules);
            movers.extend(capsules.drain(..).map(|c| (piece, c)));
        }
    }
    let merged: usize = drawn.groups.iter().map(|g| g.1.len()).sum();
    info!(
        "map drawn: {} primitives, {merged} of them merged into {} groups; {} moving occluders",
        placed.len(),
        drawn.groups.len(),
        movers.len()
    );
    ao_kit.movers.set(movers);
    if map.bonuses.list.is_empty() {
        return;
    }
    // Bonuses share their meshes, and their materials by kind (the card's picture too: it is the kind's).
    let shapes = [
        meshes.add(Sphere::new(0.62).mesh().uv(32, 20)),
        meshes.add(Circle::new(0.42).mesh().resolution(32)),
        meshes.add(Annulus::new(0.75, 1.0).mesh().resolution(40)),
    ];
    let mut looks: HashMap<Power, [Handle<StandardMaterial>; 3]> = HashMap::new();
    for b in &map.bonuses.list {
        let [bubble, card, ring] = looks
            .entry(b.kind)
            .or_insert_with(|| {
                let color = power_color(b.kind);
                let (icon, _) = crate::ui::text::bonus(b.kind);
                [
                    materials.add(StandardMaterial {
                        base_color: color.with_alpha(0.45),
                        emissive: color.to_linear() * 0.45,
                        perceptual_roughness: 0.15,
                        alpha_mode: AlphaMode::Blend,
                        ..default()
                    }),
                    materials.add(StandardMaterial {
                        base_color_texture: Some(images.add(crate::render::emoji::board(icon, color))),
                        unlit: true,
                        cull_mode: None,
                        double_sided: true,
                        ..default()
                    }),
                    materials.add(StandardMaterial {
                        base_color: color.with_alpha(0.6),
                        unlit: true,
                        alpha_mode: AlphaMode::Blend,
                        cull_mode: None,
                        double_sided: true,
                        ..default()
                    }),
                ]
            })
            .clone();
        commands
            .spawn((
                BonusView(b.i),
                Transform::from_translation(b.pos.as_vec3()),
                Visibility::Hidden,
                ChildOf(root),
            ))
            .with_children(|g| {
                g.spawn((
                    BonusPart::Bubble,
                    Mesh3d(shapes[0].clone()),
                    MeshMaterial3d(bubble),
                    Transform::from_xyz(0.0, 1.05, 0.0),
                ));
                g.spawn((
                    BonusPart::Card,
                    Mesh3d(shapes[1].clone()),
                    MeshMaterial3d(card),
                    Transform::from_xyz(0.0, 1.05, 0.0),
                ));
                g.spawn((
                    BonusPart::Ring,
                    Mesh3d(shapes[2].clone()),
                    MeshMaterial3d(ring),
                    Transform::from_xyz(0.0, 0.04, 0.0).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
                    bevy::light::NotShadowCaster,
                ));
            });
    }
}

/// What a primitive is painted with: the palette as the
/// round's look repaints it with the look's pattern, or a plain colour; and its surface (padded for big
/// floors, rubber for balls, plastic otherwise).
pub fn prim_spec(
    look: &ResolvedLook,
    kind: PrimKind,
    dims: [f64; 3],
    pal: Palette,
    freq: Option<f64>,
    surface: Option<fb_sim::scene::Surface>,
    pattern: Option<Pattern>,
) -> Spec {
    let [a, b, c] = dims;
    let big = match kind {
        PrimKind::Box => a * c >= 30.0 && a.min(c) >= 3.0 && b <= 3.0,
        PrimKind::Cyl => a >= 4.0 && b <= 3.0,
        PrimKind::Sphere => false,
    };
    let fallback = match kind {
        PrimKind::Sphere => Kind::Rubber,
        _ if big => Kind::Padded,
        _ => Kind::Plastic,
    };
    let kind = surface.map_or(fallback, Kind::from);
    // Ice keeps its piece's colour, lighter and bluer: clear ice over it.
    let tone = |c: Rgb| {
        let c = color(c).to_linear();
        if kind == Kind::Ice {
            LinearRgba::new(c.red * 0.6 + 0.2, c.green * 0.6 + 0.3, c.blue * 0.6 + 0.4, c.alpha)
        } else {
            c
        }
    };
    if pal[0] == pal[1] {
        return Spec::plain(tone(pal[0]), Some(kind));
    }
    let tones = look.repaint(pal).unwrap_or(pal);
    Spec {
        paint: Some(Paint {
            c1: tone(tones[0]),
            c2: tone(tones[1]),
            freq: freq.unwrap_or(0.25) as f32,
            dir: Vec2::ONE,
            speed: 0.0,
            kind: pattern.unwrap_or(look.pattern),
        }),
        ..Spec::plain(LinearRgba::WHITE, Some(kind))
    }
}

/// The frame's sim time: the predicted tick plus how far into the next one the frame is.
pub fn frame_tick(timeline: &LocalTimeline, fixed: &Time<Fixed>) -> f64 {
    timeline.tick().0 as f64 + fixed.overstep_fraction() as f64
}

/// What `pose_map` saw of each node the frame before, so that only what moves is posed again.
#[derive(Default)]
struct Posed {
    generation: Option<u32>,
    /// Position, rotation, scale.
    locals: Vec<(V3, V3, V3)>,
    shown: Vec<bool>,
    /// The world matrix changed this frame (the node or one above it moved).
    moved: Vec<bool>,
    /// Turned shown or hidden this frame.
    flipped: Vec<bool>,
}

impl Posed {
    /// Recomputes the world matrices of the nodes that moved or sit under one that did (all of them: `all`).
    fn update(&mut self, nodes: &mut Nodes, all: bool) {
        let n = nodes.0.len();
        if self.locals.len() != n {
            self.locals.resize(n, (V3::ZERO, V3::ZERO, V3::ZERO));
            self.shown.resize(n, false);
            self.moved.resize(n, false);
            self.flipped.resize(n, false);
        }
        // (Parents come before their children.)
        for i in 0..n {
            let node = &nodes.0[i];
            let local = (node.pos, node.rot, node.scale);
            let parent = node.parent.map(|p| p as usize);
            let moved = all || self.locals[i] != local || parent.is_some_and(|p| self.moved[p]);
            let shown = node.visible && parent.is_none_or(|p| self.shown[p]);
            if moved {
                nodes.update_one(i as u32);
            }
            self.flipped[i] = all || shown != self.shown[i];
            self.locals[i] = local;
            self.shown[i] = shown;
            self.moved[i] = moved;
        }
    }
}

/// The map's movers pose the nodes for the frame; the pieces on nodes that moved, appeared or vanished follow.
fn pose_map(
    mut commands: Commands,
    map: Option<ResMut<Map>>,
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    mut posed: Local<Posed>,
    mut drawn: ResMut<Drawn>,
    mut meshes: ResMut<Assets<Mesh>>,
    quality: Option<Res<Quality>>,
    display: Res<crate::settings::Display>,
    mut pieces: Query<(Ref<MapPiece>, &mut Transform, &mut Visibility)>,
) {
    let Some(mut map) = map else { return };
    let t = map.time(frame_tick(&timeline, &fixed) - 1.0);
    // (A map built again after none starts from generation 0 again.)
    let all = map.is_added() || posed.generation != Some(map.generation);
    let map = &mut *map;
    map.world.pose_locals(t, &mut map.render, &*map.spec.logic);
    posed.generation = Some(map.generation);
    posed.update(&mut map.render, all);
    if !all && drawn.generation == Some(map.generation) {
        let lod_k = meshes::lod_k(display.fov, quality.as_ref().map(|q| q.preset));
        split_moved(&mut commands, &mut drawn, &mut meshes, map, &posed, lod_k);
    }
    for (piece, mut tf, mut vis) in &mut pieces {
        let n = piece.node as usize;
        let fresh = piece.is_added();
        if fresh || posed.moved.get(n) == Some(&true) {
            let next = Transform::from_matrix(mat4(&map.render.get(piece.node).world));
            if *tf != next {
                *tf = next;
            }
        }
        if fresh || posed.flipped.get(n) == Some(&true) {
            let shown = if posed.shown.get(n).copied().unwrap_or(true) {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            vis.set_if_neq(shown);
        }
    }
}

/// Pops in, bobs and turns; taken: swells and vanishes.
fn place_bonuses(
    map: Option<Res<Map>>,
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
    mut views: Query<(&BonusView, &Children, &mut Visibility)>,
    mut parts: Query<(&BonusPart, &mut Transform)>,
) {
    let Some(map) = map else { return };
    let t = map.time(frame_tick(&timeline, &fixed));
    for (v, children, mut vis) in &mut views {
        let Some(b) = map.bonuses.list.get(v.0 as usize) else {
            continue;
        };
        let shown = t >= b.appear_at && b.taken.is_none_or(|(_, at)| t < at + 0.35);
        vis.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !shown {
            continue;
        }
        let grow = ((t - b.appear_at) / 0.5 + if b.appear_at <= 0.0 { 1.0 } else { 0.0 }).min(1.0);
        let gone = b.taken.map_or(0.0, |(_, at)| (t - at) / 0.35);
        let s = (grow * (1.0 + gone * 0.8) * (1.0 - gone)).max(0.01) as f32;
        let i = v.0 as f64;
        let bob = ((t * 2.4 + i).sin() * 0.15) as f32;
        for &c in children {
            let Ok((part, mut tf)) = parts.get_mut(c) else {
                continue;
            };
            match part {
                BonusPart::Ring => tf.scale = Vec3::splat(1.0 + ((t * 3.0 + i).sin() * 0.08) as f32),
                BonusPart::Bubble | BonusPart::Card => {
                    tf.scale = Vec3::splat(s);
                    tf.translation.y = 1.05 + bob;
                    if *part == BonusPart::Card {
                        tf.rotation = Quat::from_rotation_y((t * 1.8) as f32);
                    }
                }
            }
        }
    }
}

const PREV_KEYS: [KeyCode; 3] = [KeyCode::ArrowLeft, KeyCode::KeyA, KeyCode::KeyQ];
const NEXT_KEYS: [KeyCode; 3] = [KeyCode::ArrowRight, KeyCode::KeyD, KeyCode::KeyE];

/// Without a bean of one's own in a round: follow someone still in play, or look over the map. The keys
/// (and, with the mouse captured, its buttons) go through the beans and the overview.
fn spectate(
    map: Option<Res<Map>>,
    own: Query<(), (With<Predicted>, With<PlayerId>)>,
    beans: Query<&PlayerId, (With<Interpolated>, Without<Predicted>)>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pads: Query<&Gamepad>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    mut spec: ResMut<Spectate>,
    gate: Res<Gate>,
) {
    let Some(map) = map else { return };
    if spec.generation != map.generation {
        *spec = Spectate {
            generation: map.generation,
            ..default()
        };
    }
    if !own.is_empty() || map.round.kind != ArenaKind::Round {
        spec.target = None;
        return;
    }
    let mut ids: Vec<u32> = beans.iter().map(|p| p.0).filter(|id| !map.gone(*id)).collect();
    ids.sort_unstable();
    ids.dedup();
    if spec.target.is_some_and(|t| !ids.contains(&t)) {
        spec.manual = false;
    }
    // (Checked before `mouse_look`: the click that captures the mouse does not switch.)
    let captured = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    let pad = |b: [GamepadButton; 2]| pads.iter().any(|p| p.any_just_pressed(b));
    let dir = if !gate.play {
        0
    } else if keys.any_just_pressed(PREV_KEYS)
        || (captured && mouse.just_pressed(MouseButton::Right))
        || pad([GamepadButton::DPadLeft, GamepadButton::LeftTrigger])
    {
        -1
    } else if keys.any_just_pressed(NEXT_KEYS)
        || (captured && mouse.just_pressed(MouseButton::Left))
        || pad([GamepadButton::DPadRight, GamepadButton::RightTrigger])
    {
        1
    } else {
        0
    };
    if dir != 0 {
        let all: Vec<Option<u32>> = core::iter::once(None).chain(ids.iter().copied().map(Some)).collect();
        let i = all.iter().position(|t| *t == spec.target).unwrap_or(0) as i32;
        spec.target = all[(i + dir).rem_euclid(all.len() as i32) as usize];
        spec.manual = true;
    } else if !spec.manual {
        spec.target = ids.first().copied();
    }
}

/// The pad's right stick turns the camera (radians a second at full tilt).
const PAD_YAW: f32 = 3.2;
const PAD_PITCH: f32 = 2.2;

fn mouse_look(
    mut motion: MessageReader<MouseMotion>,
    pads: Query<&Gamepad>,
    time: Res<Time<Real>>,
    mut cam: ResMut<CameraAngles>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    controls: Res<Controls>,
    gate: Res<Gate>,
) {
    let dt = time.delta_secs().min(0.1);
    if gate.play {
        let k = controls.stick_sensitivity;
        let inv = if controls.invert_stick_y { -1.0 } else { 1.0 };
        for pad in &pads {
            let dz = |v: f32| if v.abs() < 0.15 { 0.0 } else { v };
            let s = pad.right_stick();
            cam.yaw -= dz(s.x) * PAD_YAW * k * dt;
            cam.pitch = (cam.pitch - dz(s.y) * PAD_PITCH * k * inv * dt).clamp(PITCH_MIN, PITCH_MAX);
        }
    }
    let captured = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if !captured {
        motion.clear();
        return;
    }
    let k = controls.mouse_sensitivity;
    let inv = if controls.invert_mouse_y { -1.0 } else { 1.0 };
    for m in motion.read() {
        cam.yaw -= m.delta.x * 0.004 * k;
        cam.pitch = (cam.pitch + m.delta.y * 0.003 * k * inv).clamp(PITCH_MIN, PITCH_MAX);
    }
}
