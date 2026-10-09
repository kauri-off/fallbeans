// A portal's unlit disc (`portal.rs`): a vortex in, rings out; linear colours may exceed 1 for bloom.
#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::globals,
}

struct Portal {
    // rgb: the colour; a: opacity.
    color: vec4<f32>,
    // x: 0 a way in, 1 an exit; y: flash (0…1); z: phase.
    params: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: Portal;

const TAU: f32 = 6.2831853;
const SPARKS: u32 = 10u;
// Deep violet (#1a0f3d), the far end of the tunnel.
const DEEP: vec3<f32> = vec3(0.0103, 0.0048, 0.0467);
const WHITE: vec3<f32> = vec3(1.0, 1.0, 1.0);

fn hash(n: f32) -> f32 {
    return fract(sin(n * 12.9898 + 4.1414) * 43758.547);
}

// Sparks on their way through the disc: `inward` drawn into the middle along a spiral, else thrown out.
fn sparks(p: vec2<f32>, t: f32, inward: bool) -> f32 {
    var s = 0.0;
    for (var i = 0u; i < SPARKS; i += 1u) {
        let k = f32(i);
        let life = fract(t * (0.3 + 0.2 * hash(k)) + hash(k + 11.0));
        let r = select(0.1 + 0.85 * life, 0.95 * (1.0 - life), inward);
        let a = hash(k + 23.0) * TAU + select(life * 1.2, life * 3.5, inward);
        let d = length(p - r * vec2(cos(a), sin(a)));
        let size = 0.025 + 0.035 * r;
        // Fading in at the rim and out in the middle (or out at the rim, thrown).
        let fade = select(smoothstep(1.0, 0.75, r), smoothstep(0.0, 0.25, r) * smoothstep(1.0, 0.8, r), inward);
        s += smoothstep(size, 0.0, d) * fade;
    }
    return s;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.uv * 2.0 - 1.0;
    let r = length(p);
    let a = atan2(p.y, p.x);
    let flash = m.params.y;
    // The flash spins it up for a moment: set back as it strikes, its time catches up 0.6 s while it fades.
    let t = globals.time + m.params.z - flash * 0.6;
    let c = m.color.rgb;
    var col: vec3<f32>;
    if (m.params.x < 0.5) {
        // A tunnel: its depth grows towards the middle, bands sink into it.
        let depth = 0.3 / (r + 0.06);
        let bands = 0.5 + 0.5 * sin(depth * 5.0 - t * 4.0);
        let wall = smoothstep(0.08, 0.95, r);
        col = mix(DEEP + c * 0.15, c, wall) * (0.7 + 0.3 * bands);
        // Spiral arms winding in (a constant phase moves inwards with time), lighter than the colour.
        let arm = smoothstep(0.35, 1.0, sin(a * 4.0 + log(r + 0.02) * 5.0 + t * 3.0));
        col += mix(c, WHITE, 0.55) * arm * smoothstep(0.06, 0.35, r) * 0.9;
        // The white-hot core, with a halo of the colour.
        col += WHITE * exp(-r * r * 30.0) * 2.4 + c * exp(-r * r * 6.0) * 0.8;
        col += WHITE * sparks(p, t, true) * 1.6;
    } else {
        // Rings flowing out from a deep middle to a white rim.
        let rings = smoothstep(0.4, 0.95, sin(r * 16.0 - t * 4.5));
        let wall = smoothstep(0.0, 0.75, r);
        col = mix(DEEP, c, wall) + mix(c, WHITE, 0.5) * rings * smoothstep(0.1, 0.4, r) * 0.8;
        col += mix(c, WHITE, 0.6) * smoothstep(0.7, 0.98, r) * 0.6;
        col += mix(c, WHITE, 0.5) * sparks(p, t, false) * 1.4;
    }
    // A glowing rim, a slow pulse, and the flash of a trip.
    col += mix(c, WHITE, 0.4) * smoothstep(0.8, 0.97, r) * 1.4;
    col *= 1.0 + 0.12 * sin(globals.time * 3.0 + m.params.z * 5.0);
    col = col * (1.0 + 1.5 * flash) + c * flash;
    // A soft edge, and a little more solid in the bright middle.
    let edge = smoothstep(1.0, 0.96, r);
    let alpha = clamp(m.color.a + (1.0 - m.color.a) * exp(-r * r * 8.0) + flash * 0.2, 0.0, 1.0) * edge;
    return vec4(col, alpha);
}
