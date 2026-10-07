// Surfaces: triplanar detail in the object's own space (normal, cavity shading,
// roughness, frost) and the look's two-tone pattern, on top of the standard PBR material.
// (The forward pass only: the prepasses and shadows use the standard material's shaders.)
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    mesh_functions,
    mesh_view_bindings::globals,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither;
#endif

#ifdef BINDLESS
#import bevy_pbr::mesh_bindings::mesh
#import bevy_render::bindless::{bindless_samplers_filtering, bindless_textures_2d}
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

#ifdef BINDLESS
// Bindless indices 50…52 (`Surface` in surface.rs): its data, the detail texture and its sampler.
struct SurfaceIndices {
    material: u32,
    detail_texture: u32,
    detail_sampler: u32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage> surface_indices: array<SurfaceIndices>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage> surface_array: array<Surface>;
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(50) var<uniform> surface_uniform: Surface;
@group(#{MATERIAL_BIND_GROUP}) @binding(51) var detail_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(52) var detail_smp: sampler;
#endif

#ifdef OBJECT_FRAME
// A vector turned by a unit quaternion (x, y, z, w).
fn turned(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}
#endif

// One projection of the detail texture.
fn sample_detail(slot: u32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    return textureSampleGrad(
        bindless_textures_2d[surface_indices[slot].detail_texture],
        bindless_samplers_filtering[surface_indices[slot].detail_sampler],
        uv,
        gx,
        gy,
    );
#else
    return textureSampleGrad(detail_tex, detail_smp, uv, gx, gy);
#endif
}

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var in = vertex_output;
#ifdef OBJECT_FRAME
    // Merged static pieces (`meshes::merge`): each vertex carries its piece's frame, the rotation in the colour
    // and the position in it in the UVs. The colour is no tint.
    let piece_rot = normalize(in.color);
    let piece_pos = vec3(in.uv_b, in.uv.x);
    in.color = vec4(1.0);
#endif
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef BINDLESS
    let slot = mesh[in.instance_index].material_and_lightmap_bind_group_slot & 0xffffu;
    let surface = surface_array[surface_indices[slot].material];
#else
    let slot = 0u;
    let surface = surface_uniform;
#endif

    var color = pbr_input.material.base_color;
    let kind = i32(round(surface.extra.x));
    // (A kind with no normal, roughness, cavity or frost would sample the flat texture for nothing.)
    let sampled = surface.extra.z < 0.5
        && any(vec4(surface.detail.y, surface.detail.z, surface.detail.w, surface.extra.y) != vec4(0.0));
    // (Without the detail texture: what the flat texture holds, no change.)
    var height = 1.0;
    var rough = 0.5;
    // (The branches are the material's: uniform over a triangle, as the derivatives in them want.)
    if sampled || kind >= 0 {
#ifdef OBJECT_FRAME
        let ax = turned(piece_rot, vec3(1.0, 0.0, 0.0));
        let ay = turned(piece_rot, vec3(0.0, 1.0, 0.0));
        let az = turned(piece_rot, vec3(0.0, 0.0, 1.0));
        let p = piece_pos;
#else
        // The object's frame: its axes (scale removed) and origin.
        let m = mesh_functions::get_world_from_local(in.instance_index);
        let ax = normalize(m[0].xyz);
        let ay = normalize(m[1].xyz);
        let az = normalize(m[2].xyz);
        let rel = in.world_position.xyz - m[3].xyz;
        // The position in the object's space at world scale.
        let p = vec3(dot(rel, ax), dot(rel, ay), dot(rel, az));
#endif
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
                tx = sample_detail(slot, dp.zy, gx.zy, gy.zy);
            }
            if w.y > 0.0 {
                ty = sample_detail(slot, dp.xz, gx.xz, gy.xz);
            }
            if w.z > 0.0 {
                tz = sample_detail(slot, dp.xy, gx.xy, gy.xy);
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
            // The drift repeats a whole number of times (even: the checker's period is 2) over Bevy's hour of
            // `globals.time`, which wraps at 3600 s: no jump when it does.
            q.x += 2.0 * fract(globals.time / 3600.0 * round(surface.pattern.w * 1800.0));
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

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
