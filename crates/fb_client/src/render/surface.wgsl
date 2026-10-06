// Surfaces: triplanar detail in the object's own space (normal, cavity shading,
// roughness, frost) and the look's two-tone pattern, on top of the standard PBR material.
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    mesh_functions,
    mesh_view_bindings::globals,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#endif

#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither;
#endif

struct Surface {
    c1: vec4<f32>,
    c2: vec4<f32>,
    // Tiles per metre, normal strength, roughness variation, cavity.
    detail: vec4<f32>,
    // Pattern frequency, direction (x, z), speed.
    pattern: vec4<f32>,
    // Pattern kind (−1: none), frost, 1: no detail texture (the Low preset).
    extra: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> surface: Surface;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var detail_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var detail_smp: sampler;

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var in = vertex_output;
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    var color = pbr_input.material.base_color;
    let kind = i32(round(surface.extra.x));
    let sampled = surface.extra.z < 0.5;
    // (Without the detail texture: what the flat texture holds, no change.)
    var height = 1.0;
    var rough = 0.5;
    // (Both branches are uniform: the material's, as the derivatives in them want.)
    if sampled || kind >= 0 {
        // The object's frame: its axes (scale removed) and origin.
        let m = mesh_functions::get_world_from_local(in.instance_index);
        let ax = normalize(m[0].xyz);
        let ay = normalize(m[1].xyz);
        let az = normalize(m[2].xyz);
        let rel = in.world_position.xyz - m[3].xyz;
        // The position in the object's space at world scale.
        let p = vec3(dot(rel, ax), dot(rel, ay), dot(rel, az));
        if sampled {
            let nw = normalize(in.world_normal);
            let n = vec3(dot(nw, ax), dot(nw, ay), dot(nw, az));
            var w = pow(abs(n), vec3(4.0));
            w /= w.x + w.y + w.z;
            // Projections that barely show are not sampled; the rest are renormalised.
            w = max(w - 0.02, vec3(0.0));
            w /= w.x + w.y + w.z;
            let dp = p * surface.detail.x;
            let gx = dpdx(dp);
            let gy = dpdy(dp);
            var tx = vec4(0.5);
            var ty = vec4(0.5);
            var tz = vec4(0.5);
            if w.x > 0.0 {
                tx = textureSampleGrad(detail_tex, detail_smp, dp.zy, gx.zy, gy.zy);
            }
            if w.y > 0.0 {
                ty = textureSampleGrad(detail_tex, detail_smp, dp.xz, gx.xz, gy.xz);
            }
            if w.z > 0.0 {
                tz = textureSampleGrad(detail_tex, detail_smp, dp.xy, gx.xy, gy.xy);
            }
            height = tx.b * w.x + ty.b * w.y + tz.b * w.z;
            rough = tx.a * w.x + ty.a * w.y + tz.a * w.z;
            let nx = vec3(0.0, tx.y * 2.0 - 1.0, tx.x * 2.0 - 1.0);
            let ny = vec3(ty.x * 2.0 - 1.0, 0.0, ty.y * 2.0 - 1.0);
            let nz = vec3(tz.x * 2.0 - 1.0, tz.y * 2.0 - 1.0, 0.0);
            let d = (nx * w.x + ny * w.y + nz * w.z) * surface.detail.y;
            pbr_input.N = normalize(pbr_input.N + d.x * ax + d.y * ay + d.z * az);
        }
        // The pattern, anti-aliased by its own screen-space rate of change.
        if kind >= 0 {
            let dir = surface.pattern.yz;
            var q = vec2(dot(p.xz, dir), dot(p.xz, vec2(-dir.y, dir.x))) * surface.pattern.x;
            q.x += globals.time * surface.pattern.w;
            var sx = q.x;
            if kind == 3 {
                sx += abs(fract(q.y * 0.5) - 0.5) * 1.2;
            } else if kind == 4 {
                sx += sin(q.y * 1.5) * 0.3;
            }
            let fw = min(0.5, fwidth(sx));
            var stp = smoothstep(0.25 - fw, 0.25 + fw, abs(fract(sx) - 0.5));
            if kind == 1 {
                let fw2 = min(vec2(0.5), fwidth(q));
                let sq = smoothstep(vec2(0.25) - fw2, vec2(0.25) + fw2, abs(fract(q * 0.5) - 0.5));
                stp = sq.x + sq.y - 2.0 * sq.x * sq.y;
            } else if kind == 2 {
                let r = length(fract(q) - 0.5);
                let fwr = min(0.5, fwidth(r));
                stp = smoothstep(0.26 - fwr, 0.26 + fwr, r);
            }
            color = vec4(color.rgb * mix(surface.c1.rgb, surface.c2.rgb, stp), color.a);
        }
    }
    color = vec4(color.rgb * mix(1.0 - surface.detail.w, 1.0, smoothstep(0.1, 0.7, height)), color.a);
    color = vec4(mix(color.rgb, vec3(1.0), smoothstep(0.35, 1.0, rough) * surface.extra.y), color.a);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, color);
    pbr_input.material.perceptual_roughness = clamp(
        pbr_input.material.perceptual_roughness * mix(1.0 - surface.detail.z, 1.0 + surface.detail.z, rough),
        0.04,
        1.0,
    );

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
