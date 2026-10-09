//! Little bursts where things happen, from a pool of slots drawn by `vfx.wgsl` on the GPU's clock (a uniform
//! written per burst, nothing per frame): dust where a bean lands or throws itself on its belly (and behind it
//! as it slides on), confetti where one finishes or rings the lobby's bell, sparkles where one takes a bonus,
//! a ring where a bumper throws one off (`props.rs`); and the twinkle about the finish's stars. Cheap enough for
//! every preset. Also the glow of a bean's aura (`GlowMaterial`). Both mark the upscalers' reactive mask
//! (`reactive.rs`): they have no motion vectors.
use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendState, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use fb_net::{Anim, BeanId, BodyFull, RemotePose};
use fb_sim::looks::Look;
use lightyear::prelude::Predicted;

use super::reactive::{ADD, MARK};
use super::warmup::Warmup;
use crate::beans::BeanView;
use crate::game::{Cue, Map};

/// Particles in a burst (quads of one mesh).
const COUNT: usize = 16;
/// Bursts at once at most: the oldest goes for a new one.
const SLOTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Puffs rolling out low round the feet, swelling and fading.
    Dust,
    /// Cards in every colour thrown up, fluttering down.
    Confetti,
    /// Stars bursting out round, glowing.
    Sparkle,
    /// Glowing puffs racing out flat in a ring.
    Ring,
    /// Points on a slowly turning shell, each flaring now and then: for ever (a slot never plays it).
    Twinkle,
}

impl Kind {
    /// Its number in `vfx.wgsl`.
    fn code(self) -> f32 {
        match self {
            Kind::Dust => 0.0,
            Kind::Confetti => 1.0,
            Kind::Sparkle => 2.0,
            Kind::Ring => 3.0,
            Kind::Twinkle => 4.0,
        }
    }

    /// How long it plays (s).
    fn duration(self) -> f32 {
        match self {
            Kind::Dust => 0.6,
            Kind::Confetti => 2.2,
            Kind::Sparkle => 0.7,
            Kind::Ring => 0.45,
            Kind::Twinkle => 0.0,
        }
    }
}

/// A burst to play: where, in what colour (confetti has its own), how big (1: a bean's).
#[derive(Message, Clone, Copy, Debug)]
pub struct Burst {
    pub kind: Kind,
    pub at: Vec3,
    pub color: LinearRgba,
    pub size: f32,
}

#[derive(Clone, Copy, Default, ShaderType)]
pub struct VfxUniform {
    /// rgb: colour (linear); a: 1 (0: nothing drawn).
    pub color: Vec4,
    /// x: kind (`Kind::code`), y: start (s, on the shaders' clock), z: duration (s), w: unused.
    pub params: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(VfxKey)]
pub struct VfxMaterial {
    #[uniform(0)]
    pub u: VfxUniform,
}

/// The glowing kinds add their light; the others cover what is behind.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct VfxKey {
    glow: bool,
}

impl From<&VfxMaterial> for VfxKey {
    fn from(m: &VfxMaterial) -> Self {
        Self {
            glow: m.u.params.x >= Kind::Sparkle.code() - 0.5,
        }
    }
}

impl Material for VfxMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://fb_client/render/vfx.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/vfx.wgsl".into()
    }

    // (Drawn with the see-through things; the blend is `specialize`'s.)
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_UV_1.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![layout];
        descriptor.primitive.cull_mode = None;
        if let Some(d) = &mut descriptor.depth_stencil {
            d.depth_write_enabled = Some(false);
        }
        // Premultiplied over what is behind, or light added; the alpha (coverage, or the glow's strength) marks
        // the mask either way.
        let color = if key.bind_group_data.glow {
            ADD
        } else {
            BlendState::PREMULTIPLIED_ALPHA_BLENDING.color
        };
        super::reactive::blend(descriptor, color, MARK);
        Ok(())
    }
}

#[derive(Clone, Copy, Default, ShaderType)]
pub struct GlowUniform {
    /// rgb: colour (linear); a: strength (the light added is the colour times it).
    pub color: Vec4,
}

/// Unlit light added over the scene in one colour: a bean's aura.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct GlowMaterial {
    #[uniform(0)]
    pub u: GlowUniform,
}

impl GlowMaterial {
    pub fn new(color: Color) -> Self {
        Self {
            u: GlowUniform {
                color: color.to_linear().to_vec4(),
            },
        }
    }
}

impl Material for GlowMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fb_client/render/glow.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        super::reactive::blend(descriptor, ADD, MARK);
        Ok(())
    }
}

/// A slot of the pool.
#[derive(Component)]
struct Slot;

/// The slots (entity, material, until when it plays: s of the shaders' clock), the next to use, and what the
/// stars' twinkles are drawn with.
#[derive(Resource)]
pub struct Pool {
    slots: Vec<(Entity, Handle<VfxMaterial>, f32)>,
    next: usize,
    mesh: Handle<Mesh>,
    twinkle: Handle<VfxMaterial>,
}

impl Pool {
    /// A twinkle round a star (a child of it), about a unit of its own size out.
    pub fn twinkle(&self) -> impl Bundle {
        (
            Mesh3d(self.mesh.clone()),
            MeshMaterial3d(self.twinkle.clone()),
            Transform::from_scale(Vec3::splat(1.2)),
            NotShadowCaster,
            NotShadowReceiver,
        )
    }
}

/// A quad per particle: a direction of its own (a unit vector) in the position, the corner in the first UV,
/// a seed and its share of the count in the second (any randomness will do: visual only).
fn mesh() -> Mesh {
    let mut x = 0x6c07_8965u32;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x as f32 / u32::MAX as f32
    };
    let (mut pos, mut corner, mut seed, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..COUNT {
        // (Even over the sphere: an even height and an even turn.)
        let y = rnd() * 2.0 - 1.0;
        let a = rnd() * core::f32::consts::TAU;
        let r = (1.0 - y * y).max(0.0).sqrt();
        let d = [r * a.cos(), y, r * a.sin()];
        let s = rnd();
        for c in [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            pos.push(d);
            corner.push(c);
            seed.push([s, i as f32 / COUNT as f32]);
        }
        let b = (i * 4) as u32;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, corner)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, seed)
        .with_inserted_indices(Indices::U32(idx))
}

pub struct VfxPlugin;

impl Plugin for VfxPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "vfx.wgsl");
        bevy::asset::embedded_asset!(app, "glow.wgsl");
        app.add_plugins((
            MaterialPlugin::<VfxMaterial>::default(),
            MaterialPlugin::<GlowMaterial>::default(),
        ));
        app.add_message::<Burst>();
        app.add_systems(Startup, setup);
        app.add_systems(Update, ((bean_dust, cue_bursts), play, rest).chain());
    }
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<VfxMaterial>>) {
    let mesh = meshes.add(mesh());
    // (Drawn from the start, played out: their pipelines compile with the warm-up, `rest` hides them after. Half
    // are glowing kinds, half not: the two blends.)
    let slots = (0..SLOTS)
        .map(|i| {
            let kind = if i % 2 == 0 { Kind::Dust } else { Kind::Sparkle };
            let m = mats.add(VfxMaterial {
                u: VfxUniform {
                    color: Vec4::ZERO,
                    params: Vec4::new(kind.code(), 0.0, 0.0, 0.0),
                },
            });
            let e = commands
                .spawn((
                    Slot,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(m.clone()),
                    Transform::default(),
                    Visibility::default(),
                    NoFrustumCulling,
                    NotShadowCaster,
                    NotShadowReceiver,
                ))
                .id();
            (e, m, 0.0)
        })
        .collect();
    let gold = LinearRgba::rgb(1.0, 0.8, 0.35);
    let twinkle = mats.add(VfxMaterial {
        u: VfxUniform {
            color: gold.to_vec3().extend(1.0),
            params: Vec4::new(Kind::Twinkle.code(), 0.0, 0.0, 0.0),
        },
    });
    commands.insert_resource(Pool {
        slots,
        next: 0,
        mesh,
        twinkle,
    });
}

/// Each burst takes the next slot (the oldest), placed and scaled where it plays.
fn play(
    mut bursts: MessageReader<Burst>,
    time: Res<Time>,
    pool: Option<ResMut<Pool>>,
    mut slots: Query<(&mut Transform, &mut Visibility), With<Slot>>,
    mut mats: ResMut<Assets<VfxMaterial>>,
) {
    let Some(mut pool) = pool else {
        bursts.clear();
        return;
    };
    let now = time.elapsed_secs_wrapped();
    for b in bursts.read() {
        let i = pool.next % pool.slots.len().max(1);
        pool.next = i + 1;
        let Some((e, h, until)) = pool.slots.get_mut(i) else {
            continue;
        };
        *until = now + b.kind.duration();
        if let Ok((mut tf, mut vis)) = slots.get_mut(*e) {
            *tf = Transform::from_translation(b.at).with_scale(Vec3::splat(b.size.max(0.01)));
            vis.set_if_neq(Visibility::Inherited);
        }
        if let Some(mut m) = mats.get_mut(&*h) {
            m.u = VfxUniform {
                color: b.color.to_vec3().extend(1.0),
                params: Vec4::new(b.kind.code(), now, b.kind.duration(), 0.0),
            };
        }
    }
}

/// Slots played out are not drawn (but all are while the warm-up runs: their pipeline compiles then).
fn rest(
    time: Res<Time>,
    warm: Option<Res<Warmup>>,
    pool: Option<Res<Pool>>,
    mut slots: Query<&mut Visibility, With<Slot>>,
) {
    let (Some(pool), false) = (pool, warm.is_some_and(|w| w.busy())) else {
        return;
    };
    let now = time.elapsed_secs_wrapped();
    for (e, _, until) in &pool.slots {
        // (Over by the clock, or by a whole wrap of it.)
        let over = now > *until || *until - now > 60.0;
        if over && let Ok(mut vis) = slots.get_mut(*e) {
            vis.set_if_neq(Visibility::Hidden);
        }
    }
}

/// The colour of dust in a look: light, a little of its haze in it, dimmer under a weaker sun.
fn dust_color(l: &Look) -> LinearRgba {
    let k = (l.sun.intensity as f32 / 2.2).clamp(0.4, 1.1) * 0.85;
    LinearRgba::WHITE.mix(&super::linear(l.fog.color), 0.35) * k
}

/// A bean's feet: since when it has been in the air (or diving), whether it slid last frame, when it last
/// left a puff sliding (s of real time).
#[derive(Default)]
struct Feet {
    air_since: Option<f32>,
    sliding: bool,
    puffed: f32,
}

/// A bean as drawn, and what it is doing (its own body, or the others' pose).
type DrawnBean = (
    Entity,
    &'static GlobalTransform,
    &'static InheritedVisibility,
    Option<&'static BodyFull>,
    Option<&'static RemotePose>,
    Has<Predicted>,
);

/// Dust where a bean comes down after a moment in the air (more if it lands on its belly), as it throws itself
/// down on the ground, and behind it while it slides on fast; every bean, from what it is drawn doing.
fn bean_dust(
    time: Res<Time<Real>>,
    map: Option<Res<Map>>,
    beans: Query<DrawnBean, With<BeanView>>,
    mut feet: Local<HashMap<Entity, Feet>>,
    mut bursts: MessageWriter<Burst>,
) {
    let Some(map) = map else {
        feet.clear();
        return;
    };
    let now = time.elapsed_secs();
    let color = dust_color(map.look.look);
    feet.retain(|e, _| beans.contains(*e));
    for (e, at, shown, full, pose, own) in &beans {
        let (anim, speed) = match (own, full, pose) {
            (true, Some(f), _) => {
                let v = Vec2::new(f.body.vel.x as f32, f.body.vel.z as f32);
                (Anim::of(&f.body, false, false), v.length())
            }
            (_, _, Some(p)) => (p.anim, p.vel.length()),
            _ => continue,
        };
        let f = feet.entry(e).or_default();
        if !shown.get() {
            *f = Feet::default();
            continue;
        }
        let p = at.translation();
        let slide = anim == Anim::Slide;
        if matches!(anim, Anim::Air | Anim::Dive) {
            f.air_since.get_or_insert(now);
        } else if let Some(since) = f.air_since.take()
            && now - since > 0.25
        {
            let size = if slide { 1.35 } else { 1.0 };
            bursts.write(Burst {
                kind: Kind::Dust,
                at: p,
                color,
                size,
            });
            f.puffed = now;
        }
        // (Each a little behind the last, the first at once.)
        let gap = if f.sliding { 0.14 } else { 0.0 };
        if slide && speed > 3.0 && now - f.puffed > gap {
            bursts.write(Burst {
                kind: Kind::Dust,
                at: p,
                color,
                size: 0.65,
            });
            f.puffed = now;
        }
        f.sliding = slide;
    }
}

/// Confetti where a bean finishes or rings the bell, sparkles where one takes a bonus.
fn cue_bursts(
    mut cues: MessageReader<Cue>,
    beans: Query<(&BeanId, &GlobalTransform), With<BeanView>>,
    mut bursts: MessageWriter<Burst>,
) {
    for c in cues.read() {
        let (id, kind, color, up) = match *c {
            Cue::Finish(id) | Cue::Bell(id) => (id, Kind::Confetti, LinearRgba::WHITE, 0.8),
            Cue::Bonus(id) => (id, Kind::Sparkle, LinearRgba::rgb(1.0, 0.85, 0.4), 0.9),
            _ => continue,
        };
        let Some((_, at)) = beans.iter().find(|(p, _)| p.0 == id) else {
            continue;
        };
        bursts.write(Burst {
            kind,
            at: at.translation() + Vec3::Y * up,
            color,
            size: 1.0,
        });
    }
}

#[cfg(test)]
mod tests {
    use bevy::mesh::VertexAttributeValues;

    use super::*;

    #[test]
    fn particles_head_their_own_ways() {
        let m = mesh();
        let Some(VertexAttributeValues::Float32x3(pos)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("no positions");
        };
        assert_eq!(pos.len(), COUNT * 4);
        for d in pos {
            let len = Vec3::from_array(*d).length();
            assert!((len - 1.0).abs() < 1e-3, "{len}");
        }
        // Not all the same way: they spread over the sphere.
        let mean = pos.iter().map(|d| Vec3::from_array(*d)).sum::<Vec3>() / pos.len() as f32;
        assert!(mean.length() < 0.6, "{mean}");
    }

    #[test]
    fn bursts_end_but_the_twinkle() {
        for k in [Kind::Dust, Kind::Confetti, Kind::Sparkle, Kind::Ring] {
            assert!(k.duration() > 0.0 && k.duration() < 3.0, "{k:?}");
        }
        assert_eq!(Kind::Twinkle.duration(), 0.0);
    }

    #[test]
    fn glowing_kinds_add_their_light() {
        let key = |k: Kind| {
            VfxKey::from(&VfxMaterial {
                u: VfxUniform {
                    color: Vec4::ONE,
                    params: Vec4::new(k.code(), 0.0, 0.0, 0.0),
                },
            })
            .glow
        };
        assert!(!key(Kind::Dust) && !key(Kind::Confetti));
        assert!(key(Kind::Sparkle) && key(Kind::Ring) && key(Kind::Twinkle));
    }

    #[test]
    fn dust_is_light_in_every_look() {
        for l in fb_sim::looks::LOOKS {
            let c = dust_color(l);
            assert!(c.red > 0.2 && c.green > 0.2 && c.blue > 0.2, "{:?}", l.id);
        }
    }
}
