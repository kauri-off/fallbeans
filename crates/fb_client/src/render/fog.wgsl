// Haze (`fog.rs`): the slices across the view, far to near, each adding the air's scattered sky and sun light and
// hiding what is behind it, composited in one full-screen pass up to the depth of what is drawn.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_pbr::mesh_view_bindings::{view, lights, globals}
#import bevy_pbr::mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
#import bevy_pbr::shadows::get_cascade_index
#import bevy_pbr::shadow_sampling::sample_shadow_map_hardware

struct Haze {
    // rgb: the sky's light in the haze; a: density at height 0 (1/m).
    haze: vec4<f32>,
    // rgb: tint of the sunlight scattered; a: how much of it.
    sun: vec4<f32>,
    // x: thinning going up (1/m), y: the most it thickens below, z: forward scattering (g), w: drift (m/s).
    shape: vec4<f32>,
}

@group(1) @binding(0) var<uniform> m: Haze;
@group(1) @binding(1) var depth: texture_depth_2d;
// The noise's lattice values (`lattice` in `fog.rs`), repeating.
@group(1) @binding(2) var lattice: texture_3d<f32>;
@group(1) @binding(3) var lattice_sampler: sampler;

// Slices from `NEAR` to `REACH` metres, closer together near the eye (where the haze in front of the beans and
// the course is seen through the fewest).
const SLICES: i32 = 24;
const NEAR: f32 = 1.0;
const REACH: f32 = 90.0;

// Where the wind blows (world x, z): `WIND` in `cloth.rs`.
const WIND: vec2<f32> = vec2(0.9439, 0.3303);

// `LATTICE` in `fog.rs`.
const LATTICE: i32 = 32;

// The distance of slice bound `j`, growing with the square.
fn bound(j: i32) -> f32 {
    let f = f32(j) / f32(SLICES);
    return NEAR + (REACH - NEAR) * f * f;
}

// Smooth value noise, 0…1: one filtered lookup between a lattice cell's corners, at the smoothed weights.
fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return textureSampleLevel(lattice, lattice_sampler, (i + u + 0.5) / f32(LATTICE), 0.0).r;
}

// Henyey-Greenstein times 4π: 1 for light scattered alike every way.
fn phase(cos_t: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cos_t;
    return (1.0 - g * g) / (d * sqrt(d));
}

// The sun's shadow along the ray: in a cascade (an orthographic projection) the point at distance d is at
// `a + b d`, made again only when the slices reach the next cascade in.
struct Shadow {
    cascade: u32,
    a: vec3<f32>,
    b: vec3<f32>,
}

// How much of the sun reaches the ray at distance `d` (1 past the shadow map's reach): one hardware-filtered lookup.
fn sunlit(s: ptr<function, Shadow>, origin: vec3<f32>, step: vec3<f32>, d: f32) -> f32 {
    let light = &lights.directional_lights[0];
    if (((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u) {
        return 1.0;
    }
    let cascade = get_cascade_index(0u, -d);
    if (cascade >= (*light).num_cascades) {
        return 1.0;
    }
    if (cascade != (*s).cascade) {
        let c = (*light).cascades[cascade].clip_from_world;
        (*s).cascade = cascade;
        (*s).a = (c * vec4(origin + (*light).shadow_depth_bias * (*light).direction_to_light, 1.0)).xyz;
        (*s).b = (c * vec4(step, 0.0)).xyz;
    }
    let ndc = (*s).a + (*s).b * d;
    if (any(ndc.xy < vec2(-1.0)) || ndc.z < 0.0 || any(ndc > vec3(1.0))) {
        return 1.0;
    }
    let uv = ndc.xy * vec2(0.5, -0.5) + vec2(0.5);
    return sample_shadow_map_hardware(uv, ndc.z, i32((*light).depth_texture_base_index + cascade));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return haze_at(in.uv, textureLoad(depth, vec2<i32>(in.position.xy), 0));
}

// The haze through `uv` of the view, up to depth `z` (reversed infinite depth: near / distance; the sky at 0).
fn haze_at(uv: vec2<f32>, z: f32) -> vec4<f32> {
    if (m.haze.a <= 0.0) {
        return vec4(0.0);
    }
    let scene = select(3.4e38, view.clip_from_view[3][2] / z, z > 0.0);
    // The ray through the pixel, one metre deep into the view (with the projection's jitter).
    let ndc = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let p = view.clip_from_view;
    let dir = vec3((ndc.x + p[2][0]) / p[0][0], (ndc.y + p[2][1]) / p[1][1], -1.0);
    let origin = view.world_position;
    let step = (view.world_from_view * vec4(dir, 0.0)).xyz;
    let drift = vec3(WIND.x, 0.02, WIND.y) * (globals.time * m.shape.w);
    // The sunlight scattered towards the eye, alike along the ray but for the shadows.
    var sun = vec3(0.0);
    let lit = lights.n_directional_lights > 0u;
    if (lit) {
        let s = &lights.directional_lights[0];
        let cos_t = dot(normalize(step), (*s).direction_to_light);
        sun = (*s).color.rgb * view.exposure * m.sun.rgb * (m.sun.a * phase(cos_t, m.shape.z));
    }
    var shadow = Shadow(0xffffffffu, vec3(0.0), vec3(0.0));
    var acc = vec4(0.0);
    for (var j = SLICES - 1; j >= 0; j--) {
        let near = bound(j);
        let far = bound(j + 1);
        let d = (near + far) * 0.5;
        if (d > scene) {
            continue;
        }
        let world = origin + step * d;
        // Thinner going up, thicker below (up to the most), in wisps drifting with the wind.
        let wisps = noise((world - drift) * 0.045);
        let density = m.haze.a * min(exp(-world.y * m.shape.x), m.shape.y) * (0.45 + 1.1 * wisps);
        let alpha = 1.0 - exp(-density * (far - near));
        var light = m.haze.rgb;
        if (lit) {
            light += sun * sunlit(&shadow, origin, step, d);
        }
        acc = vec4(light * alpha, alpha) + acc * (1.0 - alpha);
    }
    return acc;
}
