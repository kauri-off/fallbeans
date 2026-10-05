// The sky: a gradient from the horizon up, the sun's glow, slowly drifting
// clouds and, at night, twinkling stars. Linear colours, as bright as a lit white surface.
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

fn h2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

fn n2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(h2(i), h2(i + vec2(1.0, 0.0)), u.x), mix(h2(i + vec2(0.0, 1.0)), h2(i + vec2(1.0, 1.0)), u.x), u.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var s = 0.0;
    var a = 0.5;
    for (var i = 0; i < 5; i++) {
        s += a * n2(p);
        p = p * 2.03 + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return s;
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
        let cell = floor(q);
        let s = h2(cell.xy + cell.z * 17.13);
        var star = step(0.9965, s) * smoothstep(0.35, 0.05, length(fract(q) - 0.5));
        star *= 0.6 + 0.4 * sin(time * (1.0 + s * 3.0) + s * 90.0);
        col += vec3(1.0, 0.96, 0.9) * star * stars * smoothstep(0.0, 0.25, d.y);
    }
    if d.y > 0.0 && sky.params.y > 0.5 {
        let uv = d.xz / (d.y + 0.18) * 1.3;
        let wind = vec2(time * 0.010, time * 0.004);
        var c = fbm(uv + wind) + 0.25 * fbm(uv * 3.1 - wind * 2.5);
        c = smoothstep(0.62, 0.95, c);
        let fade = smoothstep(0.02, 0.3, d.y);
        let cc = mix(sky.cloud.rgb, sky.horizon.rgb, 0.18) + sky.sun_color.rgb * pow(sd, 4.0) * 0.25;
        col = mix(col, cc, c * fade * 0.8);
    }
    return vec4(col * sky.params.x * view.exposure, 1.0);
}
