// Specks drifting in the air around the camera: each a small billboard whose
// place is a function of time and its seed, wrapped in a box that follows the camera; added light.
#import bevy_pbr::mesh_view_bindings::{view, globals}

struct Motes {
    // rgb: tint; a: on (1) or off (0).
    tint: vec4<f32>,
    // xyz: the box; w: vertical drift (m/s).
    box_rise: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: Motes;

struct Vertex {
    @location(0) position: vec3<f32>,
    // The quad's corner (−1…1).
    @location(2) corner: vec2<f32>,
    // x: the speck's seed (0…1).
    @location(3) seed: vec2<f32>,
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) alpha: f32,
}

@vertex
fn vertex(v: Vertex) -> Out {
    let seed = v.seed.x;
    let t = globals.time;
    let rise = m.box_rise.w;
    let drift = vec3(
        sin(t * 0.3 + seed * 40.0) * 0.8 + t * 0.35,
        sin(t * 0.5 + seed * 17.0) * 0.6 + t * rise * (0.7 + seed * 0.6),
        cos(t * 0.27 + seed * 23.0) * 0.8,
    );
    let b = m.box_rise.xyz;
    let c = view.world_position;
    let q = v.position + drift - c + b * 0.5;
    let p = q - b * floor(q / b) - b * 0.5 + c;
    var mv = view.view_from_world * vec4(p, 1.0);
    let d = -mv.z;
    // A constant size in the world.
    let r = 0.028 * (0.8 + seed * 1.6);
    mv = vec4(mv.xy + v.corner * r, mv.z, 1.0);
    var out: Out;
    out.clip = view.clip_from_view * mv;
    out.corner = v.corner;
    out.alpha = smoothstep(1.5, 4.0, d) * (1.0 - smoothstep(14.0, 22.0, d))
        * (0.55 + 0.45 * sin(t * (1.5 + seed * 2.0) + seed * 60.0)) * m.tint.a;
    return out;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let c = in.corner * 0.5;
    let r = dot(c, c);
    let a = smoothstep(0.25, 0.0, r) * in.alpha;
    return vec4(m.tint.rgb * a * a * 0.55, 0.0);
}
