//! The warm-up behind the loading screen (`ui/loading.rs`). At the start every model is loaded (and kept), every
//! surface's detail texture, every model's levels of detail and every look's ambient light are made; then every
//! map is built as a round of it would be (the same `Map`, drawn by the same systems: props, decor, specials,
//! bonuses, sky, motes), one after another with its clock run through the round, beside beans in every hat and
//! glasses and a sample of every kind of material, until the pipeline cache has nothing left to compile; at the
//! end, until the maps' baked occlusion (`ao.rs`) is made. Nothing compiles or is made later, in play. Bevy
//! specializes pipelines only for what a view sees: everything is in every view meanwhile (`NoFrustumCulling`),
//! drawn under the opaque loading screen.
//!
//! A change of the graphics settings changes the pipelines: at the room list the maps are warmed again (fewer
//! frames each), in a room what is on screen is (and the maps later, at the room list).
//!
//! No pipeline cache is kept on disk: Bevy 0.19 creates its pipelines with `cache: None` (`PipelineCache`), so
//! wgpu's is out of reach; the driver's own shader cache still works.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::asset::RecursiveDependencyLoadState;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::system::SystemParam;
use bevy::gltf::{GltfMaterialName, GltfMesh};
use bevy::prelude::*;
use bevy::render::render_resource::{CachedPipelineState, Face, PipelineCache};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};
use fb_arena::ArenaKind;
use fb_shared::DT;
use fb_shared::outfit::{GLASSES, Glasses, HATS, Hat};
use fb_sim::looks::{LOOKS, Look, Pattern};
use fb_sim::map::MapDef;
use lightyear::prelude::LocalTimeline;

use super::EnvLights;
use super::ao::BakedAo;
use super::lod::ModelLods;
use super::quality::{Preset, Quality};
use super::surface::{Kind, Paint, Spec, SurfaceMaterial, Surfaces};
use super::upscale::{Upscaler, Upscaling};
use crate::face::{FaceKit, FaceSource};
use crate::game::{Generations, Map};
use crate::outfit::{Wardrobe, make_glasses, make_hat};
use crate::session::Session;
use crate::settings::Graphics;
use crate::ui::text;
use crate::view::MapRoot;

/// Frames a map's clock is run through its round (movers, specials and bonuses in every state they show), and
/// then frames in a row with nothing compiling, loading or appearing before the next map; after a change of
/// the graphics, fewer.
const SCRUB_FRAMES: u32 = 8;
const QUIET_FRAMES: u32 = 6;
const SCRUB_FRAMES_AGAIN: u32 = 4;
const QUIET_FRAMES_AGAIN: u32 = 3;
/// A map that does not settle in this long is left as it is (s of real time).
const MAP_MAX_S: f32 = 20.0;
/// The models and textures, at most (s).
const ASSETS_MAX_S: f32 = 60.0;
/// What is on screen in a room, at most (s).
const SETTLE_MAX_S: f32 = 10.0;
/// The maps' baked occlusion still being made after the last map, waited for at most (s).
const AO_MAX_S: f32 = 15.0;
/// The share of the bar the models and textures take in the first pass.
const ASSETS_SHARE: f32 = 0.15;

/// Pipelines the render world's cache still waits for, and those it has (written there each frame, read here).
#[derive(Resource, Clone, Default)]
pub struct Compiling {
    waiting: Arc<AtomicUsize>,
    made: Arc<AtomicUsize>,
}

impl Compiling {
    /// Pipelines compiling as of the render world's last frame.
    pub fn now(&self) -> usize {
        self.waiting.load(Ordering::Relaxed)
    }

    /// Pipelines made so far.
    pub fn made(&self) -> usize {
        self.made.load(Ordering::Relaxed)
    }
}

/// Every model, loaded at the start and kept: a map never waits for one, and their meshes and materials keep
/// their ids (the levels of detail and the props' materials made for them stay good).
#[derive(Resource, Default)]
pub struct Models(Vec<Handle<Gltf>>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Idle,
    /// Models, textures, levels of detail, ambient light (the first pass only).
    Assets,
    /// Every map in turn (at the room list).
    Maps,
    /// What is on screen now (in a room), until it is compiled.
    Settle,
}

/// What the pipelines depend on: the preset, the upscaler (a temporal one adds the motion vector prepass and its
/// own pipelines, made with its context) and the switches that add passes or change them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Key {
    preset: Preset,
    upscaler: Upscaler,
    shadows: bool,
    aa: bool,
    grade: bool,
    motes: bool,
    upscaled: bool,
}

impl Key {
    fn of(g: &Graphics, up: &Upscaling) -> Key {
        Key {
            preset: g.preset,
            upscaler: up.active,
            shadows: g.shadows,
            aa: g.aa,
            grade: g.grade,
            motes: g.motes,
            upscaled: g.upscale.scale() < 1.0,
        }
    }
}

/// The map on screen: its generation, frames since it came, when it came, and the round's length.
struct Shown {
    generation: u32,
    frames: u32,
    since: f32,
    duration: f64,
}

/// The warm-up's state; the loading screen is up while it is `busy`.
#[derive(Resource)]
pub struct Warmup {
    on: bool,
    stage: Stage,
    /// The pass at the start (models and all, more frames per map).
    first: bool,
    /// What the pass running warms for; what the maps, and what anything, were last warmed for.
    key: Option<Key>,
    maps_for: Option<Key>,
    settled_for: Option<Key>,
    /// When the pass and its stage began (s of real time; None: not yet).
    began: Option<f32>,
    since: f32,
    plan: Vec<(&'static dyn MapDef, ArenaKind, &'static Look)>,
    next: usize,
    shown: Option<Shown>,
    /// Frames in a row with nothing compiling, loading or appearing; the meshes counted the frame before.
    quiet: u32,
    meshes: usize,
    /// The beans and material samples (one root), once spawned for the stage.
    own: Option<Entity>,
    face: bool,
    lods: bool,
    progress: f32,
    /// Maps built, and those of them that did not settle in time.
    built: u32,
    late: u32,
    /// When the last map was done with (s of real time): the baked occlusion is waited for from then.
    tail: Option<f32>,
}

impl Warmup {
    fn new(on: bool) -> Self {
        Self {
            on,
            stage: if on { Stage::Assets } else { Stage::Idle },
            first: true,
            key: None,
            maps_for: None,
            settled_for: None,
            began: None,
            since: 0.0,
            plan: Vec::new(),
            next: 0,
            shown: None,
            quiet: 0,
            meshes: 0,
            own: None,
            face: false,
            lods: false,
            progress: 0.0,
            built: 0,
            late: 0,
            tail: None,
        }
    }

    /// The loading screen is up.
    pub fn busy(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// How far the pass is, 0…1.
    pub fn progress(&self) -> f32 {
        self.progress.clamp(0.0, 1.0)
    }

    /// What it is doing, for the player.
    pub fn step(&self) -> String {
        match self.stage {
            Stage::Idle => String::new(),
            Stage::Assets => text::LOADING_ASSETS.into(),
            Stage::Maps if self.plan.is_empty() => text::LOADING_SHADERS.into(),
            Stage::Maps => text::loading_maps(self.next.min(self.plan.len()), self.plan.len()),
            Stage::Settle => text::LOADING_SETTINGS.into(),
        }
    }

    fn start(&mut self, stage: Stage, key: Key, now: f32) {
        self.stage = stage;
        self.key = Some(key);
        self.began = Some(now);
        self.since = now;
        self.plan = if stage == Stage::Maps { plan() } else { Vec::new() };
        self.next = 0;
        self.shown = None;
        self.quiet = 0;
        self.progress = if self.first { ASSETS_SHARE } else { 0.0 };
        self.built = 0;
        self.late = 0;
        self.tail = None;
    }
}

pub struct WarmupPlugin {
    pub on: bool,
}

impl Plugin for WarmupPlugin {
    fn build(&self, app: &mut App) {
        let compiling = Compiling::default();
        app.insert_resource(compiling.clone());
        app.insert_resource(Warmup::new(self.on));
        app.init_resource::<Models>();
        app.add_observer(dress_bean);
        app.add_systems(
            Update,
            (watch, (in_every_view, drive).run_if(|w: Res<Warmup>| w.busy())).chain(),
        );
        if let Some(r) = app.get_sub_app_mut(RenderApp) {
            r.insert_resource(compiling);
            r.add_systems(Render, count_compiling.in_set(RenderSystems::Cleanup));
        }
    }
}

fn count_compiling(cache: Res<PipelineCache>, compiling: Res<Compiling>) {
    compiling
        .waiting
        .store(cache.waiting_pipelines().count(), Ordering::Relaxed);
    let made = cache
        .pipelines()
        .filter(|p| matches!(p.state, CachedPipelineState::Ok(_)))
        .count();
    compiling.made.store(made, Ordering::Relaxed);
}

/// Every map in its own look, then each look not shown yet on a map that has it, the lobby last (the room list
/// is lit as it is afterwards).
fn plan() -> Vec<(&'static dyn MapDef, ArenaKind, &'static Look)> {
    let classic = fb_sim::looks::classic().look;
    let kind = |id: &str| match id {
        "lobby" => ArenaKind::Lobby,
        "podium" => ArenaKind::Podium,
        _ => ArenaKind::Round,
    };
    let own_look = |d: &dyn MapDef| d.looks().first().map_or(classic, |id| id.look());
    let mut out: Vec<(&'static dyn MapDef, ArenaKind, &'static Look)> = Vec::new();
    let mut lobby = None;
    for &d in fb_maps::MAPS {
        let id = d.meta().id;
        if id == "lobby" {
            lobby = Some(d);
            continue;
        }
        out.push((d, kind(id), own_look(d)));
    }
    for l in LOOKS {
        if out.iter().any(|(_, _, shown)| shown.id == l.id) {
            continue;
        }
        if let Some(&d) = fb_maps::MAPS.iter().find(|d| d.looks().contains(&l.id)) {
            out.push((d, kind(d.meta().id), l));
        }
    }
    if let Some(d) = lobby {
        out.push((d, ArenaKind::Lobby, own_look(d)));
    }
    out
}

/// A pass begins when the graphics the pipelines were warmed for change (not while a sweep switches them).
fn watch(
    mut w: ResMut<Warmup>,
    g: Res<Graphics>,
    quality: Option<Res<Quality>>,
    session: Res<Session>,
    perf: Option<Res<crate::perf::Perf>>,
    time: Res<Time<Real>>,
    up: Res<Upscaling>,
) {
    if !w.on || w.busy() || quality.is_none() || perf.is_some_and(|p| p.busy) {
        return;
    }
    let key = Key::of(&g, &up);
    let now = time.elapsed_secs();
    if session.room.is_none() && w.maps_for != Some(key) {
        info!("warm-up: the maps again, for {key:?}");
        w.start(Stage::Maps, key, now);
    } else if w.settled_for != Some(key) {
        info!("warm-up: the scene on screen, for {key:?}");
        w.start(Stage::Settle, key, now);
    }
}

/// What the warm-up gave `NoFrustumCulling` (taken away again at its end).
#[derive(Component)]
struct Culled;

/// Every mesh is in every view, the shadows' too, wherever it is: Bevy specializes pipelines for what a view sees.
fn in_every_view(mut commands: Commands, meshes: Query<Entity, (With<Mesh3d>, Without<NoFrustumCulling>)>) {
    for e in &meshes {
        commands.entity(e).try_insert((NoFrustumCulling, Culled));
    }
}

/// What the warm-up makes and spawns with.
#[derive(SystemParam)]
struct Kit<'w> {
    assets: Res<'w, AssetServer>,
    models: ResMut<'w, Models>,
    gltfs: Res<'w, Assets<Gltf>>,
    gltf_meshes: Res<'w, Assets<GltfMesh>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    standard: ResMut<'w, Assets<StandardMaterial>>,
    images: ResMut<'w, Assets<Image>>,
    surfaces: ResMut<'w, Surfaces>,
    surface_mats: ResMut<'w, Assets<SurfaceMaterial>>,
    lods: ResMut<'w, ModelLods>,
    env: ResMut<'w, EnvLights>,
    ao: Res<'w, BakedAo>,
}

impl Kit<'_> {
    /// Models whose files and everything in them are in (or failed: nothing more will come).
    fn models_in(&self) -> usize {
        self.models
            .0
            .iter()
            .filter(|h| {
                matches!(
                    self.assets.recursive_dependency_load_state(*h),
                    RecursiveDependencyLoadState::Loaded | RecursiveDependencyLoadState::Failed(_)
                )
            })
            .count()
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut warm: ResMut<Warmup>,
    time: Res<Time<Real>>,
    mut kit: Kit,
    mut map: Option<ResMut<Map>>,
    session: Res<Session>,
    timeline: Res<LocalTimeline>,
    mut generations: ResMut<Generations>,
    compiling: Res<Compiling>,
    drawn: Query<(), With<Mesh3d>>,
    roots: Query<Entity, With<MapRoot>>,
    culled: Query<Entity, With<Culled>>,
    faces: Query<Entity, (With<WarmBean>, With<FaceSource>)>,
    face_kit: Option<Res<FaceKit>>,
    g: Res<Graphics>,
    up: Res<Upscaling>,
) {
    let w = &mut *warm;
    let now = time.elapsed_secs();
    let began = *w.began.get_or_insert(now);
    if w.stage == Stage::Assets {
        if kit.models.0.is_empty() {
            kit.models.0 = fb_sim::scene::Model::ALL
                .into_iter()
                .map(|n| kit.assets.load(format!("models/{n}.glb")))
                .collect();
            let looks = kit.env.make_all(&mut kit.images);
            info!(
                "warm-up: {} models loading, ambient light of {looks} looks made",
                kit.models.0.len()
            );
            w.since = now;
        }
        let all = kit.models.0.len();
        let models_in = kit.models_in();
        if models_in == all && !w.lods {
            w.lods = true;
            let Kit {
                models,
                gltfs,
                gltf_meshes,
                meshes,
                lods,
                ..
            } = &mut kit;
            for gltf in models.0.iter().filter_map(|h| gltfs.get(h)) {
                for m in gltf.meshes.iter().filter_map(|h| gltf_meshes.get(h)) {
                    for p in &m.primitives {
                        lods.prepare(&p.mesh, meshes);
                    }
                }
            }
        }
        let made = w.lods && kit.lods.making() == 0 && kit.surfaces.pending() == 0;
        w.progress = ASSETS_SHARE * (models_in as f32 + f32::from(u8::from(made))) / (all as f32 + 1.0);
        if !made && now - w.since < ASSETS_MAX_S {
            return;
        }
        if !made {
            warn!(
                "warm-up: models, textures or levels of detail not ready after {ASSETS_MAX_S} s ({models_in} of {all} models)"
            );
        }
        let key = Key::of(&g, &up);
        w.start(Stage::Maps, key, now);
        w.began = Some(began);
        return;
    }
    if w.own.is_none() {
        w.own = Some(spawn_own(&mut commands, &mut kit));
        w.face = false;
        w.meshes = 0;
    }
    // The face kit is made from a bean model: once it is, one of the beans wears it.
    if !w.face
        && let Some(face_kit) = &face_kit
        && let Some(bean) = faces.iter().next()
    {
        for (mesh, mat) in face_kit.sample() {
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                Transform::default(),
                bevy::light::NotShadowCaster,
                ChildOf(bean),
            ));
        }
        w.face = true;
    }
    let count = drawn.iter().count();
    let still = count == w.meshes;
    w.meshes = count;
    let idle = still && compiling.now() == 0 && kit.surfaces.pending() == 0 && kit.lods.making() == 0;
    let (scrub, quiet) = if w.first {
        (SCRUB_FRAMES, QUIET_FRAMES)
    } else {
        (SCRUB_FRAMES_AGAIN, QUIET_FRAMES_AGAIN)
    };
    if w.stage == Stage::Settle {
        w.quiet = if idle { w.quiet + 1 } else { 0 };
        w.progress = (now - w.since) / SETTLE_MAX_S;
        if w.quiet >= quiet || now - w.since > SETTLE_MAX_S {
            finish(
                &mut commands,
                w,
                map.as_deref(),
                &roots,
                &culled,
                &compiling,
                kit.images.len(),
                now,
                true,
            );
        }
        return;
    }
    // Maps.
    if w.shown.is_none() {
        // (A room entered meanwhile: its map comes, there is no more room for the warm-up's.)
        let Some(&(def, kind, look)) = w.plan.get(w.next).filter(|_| session.room.is_none()) else {
            let done = session.room.is_none();
            // (The maps' occlusion is baked in the background: a round should find its map's ready.)
            let since = *w.tail.get_or_insert(now);
            if done && kit.ao.baking() > 0 && now - since < AO_MAX_S {
                return;
            }
            finish(
                &mut commands,
                w,
                map.as_deref(),
                &roots,
                &culled,
                &compiling,
                kit.images.len(),
                now,
                done,
            );
            return;
        };
        let generation = generations.next();
        let pattern = look.patterns.first().copied().unwrap_or(Pattern::Stripes);
        let look = fb_sim::looks::resolve(look, 0.0, pattern);
        commands.insert_resource(Map::warmup(def, kind, look, generation, i64::from(timeline.tick().0)));
        w.shown = Some(Shown {
            generation,
            frames: 0,
            since: now,
            duration: def.meta().duration.clamp(10.0, 240.0),
        });
        w.next += 1;
        w.quiet = 0;
        return;
    }
    let Some(shown) = w.shown.as_mut() else { return };
    // Someone else's map now (a room entered meanwhile): the pass ends without it.
    let Some(m) = map.as_mut().filter(|m| m.warmup && m.generation == shown.generation) else {
        finish(
            &mut commands,
            w,
            map.as_deref(),
            &roots,
            &culled,
            &compiling,
            kit.images.len(),
            now,
            false,
        );
        return;
    };
    shown.frames += 1;
    if shown.frames <= scrub {
        let t = shown.duration * f64::from(shown.frames) / f64::from(scrub);
        m.round.zero_tick = i64::from(timeline.tick().0) - (t / DT).round() as i64;
    }
    w.quiet = if idle && shown.frames > scrub { w.quiet + 1 } else { 0 };
    let late = now - shown.since > MAP_MAX_S;
    if w.quiet >= quiet || late {
        if late {
            w.late += 1;
            warn!(
                "warm-up: {} still compiling after {MAP_MAX_S} s, left as it is",
                m.round.map
            );
        }
        w.built += 1;
        w.shown = None;
    }
    let share = if w.first { ASSETS_SHARE } else { 0.0 };
    w.progress = share + (1.0 - share) * w.built as f32 / w.plan.len().max(1) as f32;
}

/// The pass is over (`done`: all of it; else a room's map came in the middle): what it spawned goes, and what it
/// made for nothing else.
#[allow(clippy::too_many_arguments)]
fn finish(
    commands: &mut Commands,
    w: &mut Warmup,
    map: Option<&Map>,
    roots: &Query<Entity, With<MapRoot>>,
    culled: &Query<Entity, With<Culled>>,
    compiling: &Compiling,
    textures: usize,
    now: f32,
    done: bool,
) {
    if let Some(own) = w.own.take() {
        commands.entity(own).try_despawn();
    }
    // (Its last map, unless a room's has taken its place.)
    if map.is_none_or(|m| m.warmup) {
        if map.is_some() {
            commands.remove_resource::<Map>();
        }
        if w.stage == Stage::Maps {
            for e in roots {
                commands.entity(e).try_despawn();
            }
        }
    }
    for e in culled {
        commands.entity(e).try_remove::<(NoFrustumCulling, Culled)>();
    }
    let secs = now - w.began.unwrap_or(now);
    let maps = if w.stage == Stage::Maps {
        format!(", {} maps", w.built)
    } else {
        String::new()
    };
    let late = if w.late > 0 {
        format!(" ({} still compiling)", w.late)
    } else {
        String::new()
    };
    let why = if done { "" } else { ", cut short by a room" };
    info!(
        "warm-up: {} pipelines, {textures} textures in {secs:.1} s{maps}{late}{why}",
        compiling.made()
    );
    if done {
        if w.stage == Stage::Maps {
            w.maps_for = w.key;
        }
        w.settled_for = w.key;
    }
    w.stage = Stage::Idle;
    w.first = false;
    w.shown = None;
    w.plan.clear();
    w.progress = 1.0;
}

/// A bean of the warm-up: what it wears once its model is in (`dress_bean`).
#[derive(Component)]
struct WarmBean {
    hat: Hat,
    glasses: Glasses,
    crown: bool,
}

/// Beans in every hat and glasses (one with the winner's crown), and a sample of every kind of material the game
/// draws on the meshes it draws them on: their pipelines, whatever the maps show.
fn spawn_own(commands: &mut Commands, kit: &mut Kit) -> Entity {
    let root = commands.spawn((Transform::default(), Visibility::default())).id();
    let sphere = kit.meshes.add(Sphere::new(0.5).mesh().uv(16, 10));
    let block = kit.meshes.add(super::meshes::rounded_box(Vec3::ONE, 2, 0.1));
    let mut n = 0.0;
    let mut next = || {
        n += 1.5;
        Transform::from_xyz(n, 1.0, -4.0)
    };
    // Standard materials as the beans' parts, bonuses, specials and scenery use them.
    for alpha in [
        AlphaMode::Opaque,
        AlphaMode::Blend,
        AlphaMode::Add,
        AlphaMode::Mask(0.5),
    ] {
        for unlit in [false, true] {
            for both in [false, true] {
                let m = kit.standard.add(StandardMaterial {
                    base_color: Color::srgba(1.0, 1.0, 1.0, 0.5),
                    unlit,
                    alpha_mode: alpha,
                    double_sided: both,
                    cull_mode: if both { None } else { Some(Face::Back) },
                    ..default()
                });
                commands.spawn((Mesh3d(sphere.clone()), MeshMaterial3d(m), next(), ChildOf(root)));
            }
        }
    }
    // Surfaces: plain, painted, see-through.
    for spec in [
        Spec::plain(LinearRgba::WHITE, Some(Kind::Plastic)),
        Spec {
            paint: Some(Paint {
                c1: LinearRgba::WHITE,
                c2: LinearRgba::BLACK,
                freq: 1.0,
                dir: Vec2::ONE,
                speed: 0.0,
                kind: Pattern::Stripes,
            }),
            ..Spec::plain(LinearRgba::WHITE, Some(Kind::Padded))
        },
        Spec {
            alpha: AlphaMode::Blend,
            ..Spec::plain(LinearRgba::new(1.0, 1.0, 1.0, 0.5), Some(Kind::Glass))
        },
    ] {
        let m = kit.surfaces.material(&spec, &mut kit.images, &mut kit.surface_mats);
        commands.spawn((Mesh3d(block.clone()), MeshMaterial3d(m), next(), ChildOf(root)));
    }
    let bean = kit.assets.load(GltfAssetLabel::Scene(0).from_asset("models/bean.glb"));
    for (i, &hat) in HATS.iter().enumerate() {
        let glasses = GLASSES.get(i % GLASSES.len()).copied().unwrap_or_default();
        commands.spawn((
            WarmBean {
                hat,
                glasses,
                crown: i == 1,
            },
            WorldAssetRoot(bean.clone()),
            Transform::from_xyz(i as f32 * 1.5, 0.0, 4.0),
            Visibility::default(),
            ChildOf(root),
        ));
    }
    root
}

/// A warm-up bean's model is in: the suit as `beans::dress_beans` paints it (its clearcoat above Low), the hat,
/// glasses and crown as players wear them; and the face kit may be made from it (`face.rs`).
#[allow(clippy::too_many_arguments)]
fn dress_bean(
    ready: On<WorldInstanceReady>,
    beans: Query<&WarmBean>,
    children: Query<&Children>,
    mats: Query<(&GltfMaterialName, &MeshMaterial3d<StandardMaterial>)>,
    mut commands: Commands,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut wardrobe: ResMut<Wardrobe>,
    assets: Res<AssetServer>,
    quality: Option<Res<Quality>>,
) {
    let model = ready.entity;
    let Ok(bean) = beans.get(model) else { return };
    let mut body = Vec::new();
    let mut template = None;
    for e in children.iter_descendants(model) {
        if let Ok((name, mat)) = mats.get(e)
            && name.0 == "Body"
        {
            body.push(e);
            template = Some(mat.0.clone());
        }
    }
    let coat = quality.is_none_or(|q| q.preset != Preset::Low);
    let original = template.and_then(|h| standard.get(&h));
    let suit = crate::beans::plain_part(original, original.map_or(Color::WHITE, |m| m.base_color), 0.5, coat);
    let suit = standard.add(suit);
    for e in body {
        commands.entity(e).insert(MeshMaterial3d(suit.clone()));
    }
    let mut wiggles = Vec::new();
    for (acc, shadows) in [(make_hat(bean.hat, None), true), (make_glasses(bean.glasses), false)] {
        if let Some(a) = acc {
            wardrobe.spawn(
                &mut commands,
                model,
                &a.root,
                &suit,
                shadows,
                &mut wiggles,
                &mut meshes,
                &mut standard,
            );
        }
    }
    if bean.crown {
        commands.spawn((
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/crown.glb"))),
            Transform::from_xyz(0.0, 1.465, -0.01).with_scale(Vec3::splat(0.62)),
            Visibility::default(),
            ChildOf(model),
        ));
    }
    commands.entity(model).insert(FaceSource);
}
