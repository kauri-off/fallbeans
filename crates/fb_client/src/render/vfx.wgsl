// A burst's particles (`vfx.rs`): billboards placed and coloured by the burst's age and each particle's seed;
// premultiplied, alpha is coverage.
#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::{view, globals},
}

struct Vfx {
    // rgb: colour; a: 1 (0: nothing drawn).
    color: vec4<f32>,
    // x: kind, y: start (s), z: duration (s).
    params: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: Vfx;

const TAU: f32 = 6.2831853;
// Bevy's `globals.time` wraps at this.
const HOUR: f32 = 3600.0;

// Kinds (`Kind::code`): 0 dust, 1 confetti, 2 sparkle, 3 ring, 4 twinkle (the switch below spells them out).

// Shapes: a soft round puff, a four-pointed star, a card.
const PUFF: u32 = 0u;
const STAR: u32 = 1u;
const CARD: u32 = 2u;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    // The particle's own direction (a unit vector).
    @location(0) position: vec3<f32>,
    // The quad's corner (−1…1).
    @location(2) corner: vec2<f32>,
    // x: a seed (0…1); y: its share of the count (0…1).
    @location(3) seed: vec2<f32>,
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) corner: vec2<f32>,
    // Premultiplied colour and coverage.
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) shape: u32,
}

// A bright colour round the wheel.
fn hue(h: f32) -> vec3<f32> {
    let k = abs(fract(vec3(h) + vec3(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0;
    return clamp(k, vec3(0.0), vec3(1.0));
}

@vertex
fn vertex(v: Vertex) -> Out {
    var out: Out;
    // (Nothing drawn: outside the view.)
    out.clip = vec4(2.0, 2.0, 2.0, 1.0);
    out.corner = v.corner;
    out.color = vec4(0.0);
    out.shape = PUFF;
    let kind = u32(m.params.x + 0.5);
    let s = v.seed.x;
    var age = globals.time - m.params.y;
    if (age < 0.0) {
        age += HOUR;
    }
    var f = age / max(m.params.z, 0.001);
    if (m.params.z <= 0.0) {
        // For ever: each particle flares in its own time.
        f = fract(globals.time * (0.35 + 0.3 * s) + s * 7.0);
    }
    if (m.color.a <= 0.0 || f >= 1.0) {
        return out;
    }
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let origin = world_from_local[3].xyz;
    let scale = length(world_from_local[0].xyz);
    let dir = v.position;
    // Offset from the burst's place and radius (in the burst's units), colour and coverage.
    var p = vec3(0.0);
    var r = 0.1;
    var col = m.color.rgb;
    var a = 1.0;
    var shape = PUFF;
    var spin = 0.0;
    switch kind {
        case 0u: {
            // Dust: rolling out low round the feet and slowing, rising a little, swelling, fading.
            let ang = v.seed.y * TAU + s * 0.6;
            let d = (1.0 - pow(1.0 - f, 3.0)) * (0.5 + 0.45 * s);
            p = vec3(cos(ang) * d, 0.06 + 0.3 * f * (0.5 + s), sin(ang) * d);
            r = 0.13 + 0.26 * f;
            a = (1.0 - f) * (1.0 - f) * 0.65;
        }
        case 1u: {
            // Confetti: thrown up and out, slowed by the air, falling slowly and fluttering, turning over and over.
            let k = 1.8;
            let slow = (1.0 - exp(-k * age)) / k;
            let v0 = vec3(dir.x * 2.2, 3.5 + 2.5 * s, dir.z * 2.2);
            let fall = 4.0 / k * (age - slow);
            let flutter = sin(age * 9.0 + s * 40.0) * 0.12;
            p = v0 * slow + vec3(flutter, -fall, flutter * 0.5);
            r = 0.07;
            col = hue(s + v.seed.y * 0.37);
            a = smoothstep(1.0, 0.75, f);
            shape = CARD;
            spin = age * (5.0 + 7.0 * s) + s * TAU;
        }
        case 2u: {
            // Sparkle: stars bursting out round, slowing, twinkling as they fade.
            let d = (1.0 - (1.0 - f) * (1.0 - f)) * (0.55 + 0.6 * s);
            p = normalize(dir + vec3(0.0, 0.4, 0.0)) * d;
            r = 0.13 * (1.0 - 0.5 * f);
            col = m.color.rgb * 2.5;
            a = (1.0 - f) * (0.65 + 0.35 * sin(age * 30.0 + s * 20.0));
            shape = STAR;
            spin = s * TAU;
        }
        case 3u: {
            // Ring: glowing puffs racing out flat in a ring.
            let ang = v.seed.y * TAU;
            let d = (1.0 - (1.0 - f) * (1.0 - f)) * 1.1;
            p = vec3(cos(ang) * d, 0.1 * s, sin(ang) * d);
            r = 0.12 + 0.12 * f;
            col = m.color.rgb * 1.6;
            a = (1.0 - f) * 0.8;
        }
        default: {
            // Twinkle: points on a slowly turning shell, each flaring now and then.
            let turn = globals.time * 0.4 + s;
            let c = cos(turn);
            let sn = sin(turn);
            p = vec3(dir.x * c - dir.z * sn, dir.y, dir.x * sn + dir.z * c);
            r = 0.1;
            col = m.color.rgb * 3.0;
            a = pow(sin(f * 3.14159), 6.0);
            shape = STAR;
            spin = s * TAU;
        }
    }
    let at = origin + p * scale;
    var corner = v.corner;
    if (shape == CARD) {
        corner.y *= 0.6;
    }
    let cs = cos(spin);
    let sn = sin(spin);
    corner = vec2(corner.x * cs - corner.y * sn, corner.x * sn + corner.y * cs);
    var mv = view.view_from_world * vec4(at, 1.0);
    mv = vec4(mv.xy + corner * (r * scale), mv.z, 1.0);
    out.clip = view.clip_from_view * mv;
    out.color = vec4(col * a, a);
    out.shape = shape;
    return out;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let q = in.corner;
    var k = 1.0;
    if (in.shape == STAR) {
        // A bright middle and four thin rays.
        let ax = abs(q);
        let core = max(1.0 - length(q) * 2.0, 0.0);
        let rays = max(1.0 - ax.x * 7.0, 0.0) * (1.0 - ax.y) + max(1.0 - ax.y * 7.0, 0.0) * (1.0 - ax.x);
        k = min(core + rays, 1.0);
    } else if (in.shape == PUFF) {
        k = smoothstep(1.0, 0.15, length(q));
    }
    return in.color * k;
}
