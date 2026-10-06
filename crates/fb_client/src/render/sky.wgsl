// The sky: a gradient from the horizon up, the sun's glow, slowly drifting
// clouds and, at night, twinkling stars. Linear colours, as bright as a lit white surface.
// (Drawn after the opaque geometry, `SkyMaterial`: covered pixels never get here.)
#import bevy_pbr::{forward_io::VertexOutput, mesh_view_bindings::{view, globals}}

struct Sky {
    top: vec4<f32>,
    horizon: vec4<f32>,
    cloud: vec4<f32>,
    sun_color: vec4<f32>,
    // xyz: towards the sun; w: stars (0…1).
    sun_dir: vec4<f32>,
    // x: light scale (the sun's lux per unit of colour), y: clouds on (1) or off (0).
    params: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> sky: Sky;

const TAU: f32 = 6.2831853;
// Bevy's `globals.time` wraps at this: whatever moves repeats a whole number of times over it.
const HOUR: f32 = 3600.0;
// The cloud noise repeats over this many cells (x, y); the wind crosses exactly one period an hour.
const CLOUD_PERIOD = vec2<i32>(36, 18);

// PCG (Jarzynski and Olano, "Hash Functions for GPU Rendering"): exact at any cell, unlike sin().
fn pcg3(p: vec3<u32>) -> vec3<u32> {
    var v = p * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

// 0…1 from a cell.
fn h3(c: vec3<i32>) -> f32 {
    return f32(pcg3(bitcast<vec3<u32>>(c)).x >> 8u) / 16777216.0;
}

// A cell of the cloud noise, wrapped to its period.
fn h2(c: vec2<i32>) -> f32 {
    let w = ((c % CLOUD_PERIOD) + CLOUD_PERIOD) % CLOUD_PERIOD;
    return h3(vec3(w, 7));
}

fn n2(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(h2(i), h2(i + vec2(1, 0)), u.x),
        mix(h2(i + vec2(0, 1)), h2(i + vec2(1, 1)), u.x),
        u.y,
    );
}

// (Octaves at exactly twice the scale: a shift by whole periods stays whole periods in every one.)
fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var s = 0.0;
    var a = 0.5;
    for (var i = 0; i < 5; i++) {
        s += a * n2(p);
        p = p * 2.0 + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return s;
}

// sin(rate · t + phase) with the rate rounded to whole turns an hour (seamless where the time wraps).
fn wave(t: f32, rate: f32, phase: f32) -> f32 {
    let turns = round(rate * HOUR / TAU);
    return sin(TAU * fract(t / HOUR * turns) + phase);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = normalize(in.world_position.xyz - view.world_position);
    let time = globals.time;
    let h = clamp(d.y * 1.6 + 0.12, 0.0, 1.0);
    var col = mix(sky.horizon.rgb, sky.top.rgb, pow(h, 0.8));
    let sd = max(dot(d, sky.sun_dir.xyz), 0.0);
    col += sky.sun_color.rgb * (pow(sd, 700.0) * 2.0 + pow(sd, 10.0) * 0.18);
    let stars = sky.sun_dir.w;
    if stars > 0.0 && d.y > 0.0 {
        // A still field of twinkling stars (night looks), fading towards the horizon.
        let q = d * 220.0;
        let s = h3(vec3<i32>(floor(q)));
        var star = step(0.9965, s) * smoothstep(0.35, 0.05, length(fract(q) - 0.5));
        star *= 0.6 + 0.4 * wave(time, 1.0 + s * 3.0, s * 90.0);
        col += vec3(1.0, 0.96, 0.9) * star * stars * smoothstep(0.0, 0.25, d.y);
    }
    if d.y > 0.0 && sky.params.y > 0.5 {
        let uv = d.xz / (d.y + 0.18) * 1.3;
        // One noise period an hour: (36, 18) cells in 3600 s.
        let wind = vec2<f32>(CLOUD_PERIOD) * fract(time / HOUR);
        var c = fbm(uv + wind) + 0.25 * fbm(uv * 3.1 - wind * 3.0);
        c = smoothstep(0.62, 0.95, c);
        let fade = smoothstep(0.02, 0.3, d.y);
        let cc = mix(sky.cloud.rgb, sky.horizon.rgb, 0.18) + sky.sun_color.rgb * pow(sd, 4.0) * 0.25;
        col = mix(col, cc, c * fade * 0.8);
    }
    return vec4(col * sky.params.x * view.exposure, 1.0);
}
