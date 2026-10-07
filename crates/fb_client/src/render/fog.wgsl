// Haze around the camera (`fog.rs`): slices across the view, far to near in one draw, each depth-tested
// against the scene. A slice adds what its stretch of air scatters towards the eye (the sky's light, and the
// sun's where the shadow map lets it through) and hides as much of what is behind it.
#import bevy_pbr::mesh_view_bindings::{view, lights, globals}
#import bevy_pbr::mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
#import bevy_pbr::shadows::{get_cascade_index, world_to_directional_light_local}
#import bevy_pbr::shadow_sampling::sample_shadow_map_hardware

struct Haze {
    // rgb: the sky's light in the haze; a: density at height 0 (1/m).
    haze: vec4<f32>,
    // rgb: tint of the sunlight scattered; a: how much of it.
    sun: vec4<f32>,
    // x: thinning going up (1/m), y: the most it thickens below, z: forward scattering (g), w: drift (m/s).
    shape: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: Haze;

// Where the wind blows (world x, z): `WIND` in `cloth.rs`.
const WIND: vec2<f32> = vec2(0.9439, 0.3303);

struct Vertex {
    // x, y: the corner of the view (−1…1); z: the slice's distance (m).
    @location(0) position: vec3<f32>,
    // x: the slice's thickness (m).
    @location(2) thickness: vec2<f32>,
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) view_pos: vec3<f32>,
    @location(1) thickness: f32,
}

@vertex
fn vertex(v: Vertex) -> Out {
    let d = v.position.z;
    // Through the corner of the view at that distance, whatever the projection's aspect and field of view.
    let p = vec3(
        v.position.x * d / view.clip_from_view[0][0],
        v.position.y * d / view.clip_from_view[1][1],
        -d,
    );
    var out: Out;
    out.clip = view.clip_from_view * vec4(p, 1.0);
    out.view_pos = p;
    out.thickness = v.thickness.x;
    return out;
}

// 0…1 for a lattice point (integer maths: no sine losing its precision far from the origin).
fn hash(p: vec3<f32>) -> f32 {
    let q = vec3<u32>(vec3<i32>(p));
    var h = (q.x * 73856093u) ^ (q.y * 19349663u) ^ (q.z * 83492791u);
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = h ^ (h >> 13u);
    return f32(h & 0xffffffu) / 16777216.0;
}

// Smooth value noise, 0…1.
fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x00 = mix(hash(i), hash(i + vec3(1.0, 0.0, 0.0)), u.x);
    let x10 = mix(hash(i + vec3(0.0, 1.0, 0.0)), hash(i + vec3(1.0, 1.0, 0.0)), u.x);
    let x01 = mix(hash(i + vec3(0.0, 0.0, 1.0)), hash(i + vec3(1.0, 0.0, 1.0)), u.x);
    let x11 = mix(hash(i + vec3(0.0, 1.0, 1.0)), hash(i + vec3(1.0, 1.0, 1.0)), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

// Henyey-Greenstein times 4π: 1 for light scattered alike every way.
fn phase(cos_t: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cos_t;
    return (1.0 - g * g) / (d * sqrt(d));
}

// How much of the sun reaches a point (1 past the shadow map's reach): one hardware-filtered lookup.
fn sunlit(world: vec3<f32>, view_z: f32) -> f32 {
    let light = &lights.directional_lights[0];
    if (((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u) {
        return 1.0;
    }
    let cascade = get_cascade_index(0u, view_z);
    if (cascade >= (*light).num_cascades) {
        return 1.0;
    }
    let at = world + (*light).shadow_depth_bias * (*light).direction_to_light;
    let local = world_to_directional_light_local(0u, cascade, vec4(at, 1.0));
    if (local.w == 0.0) {
        return 1.0;
    }
    return sample_shadow_map_hardware(local.xy, local.z, i32((*light).depth_texture_base_index + cascade));
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    if (m.haze.a <= 0.0) {
        return vec4(0.0);
    }
    let world = (view.world_from_view * vec4(in.view_pos, 1.0)).xyz;
    let t = globals.time;
    // Thinner going up, thicker below (up to the most), in wisps drifting with the wind.
    let drift = vec3(WIND.x, 0.02, WIND.y) * (t * m.shape.w);
    let wisps = noise((world - drift) * 0.045);
    let density = m.haze.a * min(exp(-world.y * m.shape.x), m.shape.y) * (0.45 + 1.1 * wisps);
    let alpha = 1.0 - exp(-density * in.thickness);
    var light = m.haze.rgb;
    if (lights.n_directional_lights > 0u) {
        let sun = &lights.directional_lights[0];
        let ray = normalize(world - view.world_position);
        let lit = phase(dot(ray, (*sun).direction_to_light), m.shape.z) * sunlit(world, in.view_pos.z);
        light += (*sun).color.rgb * view.exposure * m.sun.rgb * (m.sun.a * lit);
    }
    return vec4(light * alpha, alpha);
}
