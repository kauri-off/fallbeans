// Surfaces: triplanar detail in the object's own space (normal, cavity shading,
// roughness, frost) and the look's two-tone pattern, on top of the standard PBR material; ambient occlusion
// (`ao.rs`): baked into merged static geometry, from the moving occluders near the camera, at the foot of what
// stands on the ground. (The forward pass only: the prepasses and shadows use the standard material's shaders.)
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
    // Pattern kind (−1: none), frost, 1: no detail texture (the Low preset), size of the kind's features
    // (log2 of texels).
    extra: vec4<f32>,
}

// The detail textures' size (`SIZE` in surface.rs) and their smallest mip level, 1×1: the tile's average.
const DETAIL_SIZE: f32 = 256.0;
const DETAIL_MEAN_LEVEL: f32 = 8.0;

// How much of the sun the baked occlusion takes too (all of the ambient light): a little, for contact, as the
// shadow maps miss the creases and reach no scenery.
const BAKED_ON_SUN: f32 = 0.35;
// How far a moving occluder's shade reaches, in its radii (`OCCLUDER_REACH` in ao.rs), and how much of the sky
// it hides at most, close up.
const OCCLUDER_REACH: f32 = 3.5;
const OCCLUDER_STRENGTH: f32 = 0.55;
// How far above the ground the shade at a model's foot reaches (m), and how dark it is at the ground.
const GROUND_REACH: f32 = 1.0;
const GROUND_SHADE: f32 = 0.45;

#ifdef BINDLESS
// Bindless indices 50…53 (`Surface` in surface.rs): its data, the detail texture and its sampler, the occluders.
struct SurfaceIndices {
    material: u32,
    detail_texture: u32,
    detail_sampler: u32,
    occluders: u32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage> surface_indices: array<SurfaceIndices>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage> surface_array: array<Surface>;
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(50) var<uniform> surface_uniform: Surface;
@group(#{MATERIAL_BIND_GROUP}) @binding(51) var detail_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(52) var detail_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(53) var occluder_tex: texture_2d<f32>;
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

// The detail texture's average: what it fades to where it is too fine to show.
fn sample_mean(slot: u32) -> vec4<f32> {
#ifdef BINDLESS
    return textureSampleLevel(
        bindless_textures_2d[surface_indices[slot].detail_texture],
        bindless_samplers_filtering[surface_indices[slot].detail_sampler],
        vec2(0.5),
        DETAIL_MEAN_LEVEL,
    );
#else
    return textureSampleLevel(detail_tex, detail_smp, vec2(0.5), DETAIL_MEAN_LEVEL);
#endif
}

// A texel of the moving occluders (`ao::gather`): 0 is the anchor their positions are from and their count,
// then two per capsule: its centre and how far its shade reaches from it, half its axis and its radius.
fn occluder(slot: u32, i: i32) -> vec4<f32> {
#ifdef BINDLESS
    return textureLoad(bindless_textures_2d[surface_indices[slot].occluders], vec2(i, 0), 0);
#else
    return textureLoad(occluder_tex, vec2(i, 0), 0);
#endif
}

// The sky the moving occluders leave a point `p` with normal `n`: each capsule as the ball at its point nearest
// to `p`, by Íñigo Quílez's sphere occlusion (with his approximation where the ball dips below the horizon),
// fading to nothing at `OCCLUDER_REACH` radii.
fn moving_sky(slot: u32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let head = occluder(slot, 0);
    let count = i32(head.w + 0.5);
    let rel = p - head.xyz;
    // (None of them reaches this far from the camera.)
    if count <= 0 || dot(rel, rel) > 80.0 * 80.0 {
        return 1.0;
    }
    var left = 1.0;
    for (var i = 0; i < count; i += 1) {
        let a = occluder(slot, 1 + 2 * i);
        let c = rel - a.xyz;
        if dot(c, c) > a.w * a.w {
            continue;
        }
        let b = occluder(slot, 2 + 2 * i);
        let t = clamp(dot(c, b.xyz) / max(dot(b.xyz, b.xyz), 1e-6), -1.0, 1.0);
        let d = b.xyz * t - c;
        let l = max(length(d), 1e-3);
        let reach = b.w * OCCLUDER_REACH;
        if l < reach {
            let h = l / b.w;
            let nl = dot(n, d) / l;
            let h2 = h * h;
            var occ = max(nl, 0.0) / h2;
            if 1.0 - h2 * nl * nl > 0.0 {
                let k = clamp(0.5 * (nl * h + 1.0) / h2, 0.0, 1.0);
                occ = k * sqrt(k);
            }
            left *= 1.0 - min(occ, 1.0) * OCCLUDER_STRENGTH * (1.0 - smoothstep(0.55 * reach, reach, l));
        }
    }
    return left;
}

// The sky the ground leaves the foot of a model standing on it (its height in the mesh's tag, `ao::Grounded`; 0:
// none): darker towards the ground, less where the surface looks up.
fn ground_sky(instance: u32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let tag = mesh_functions::get_tag(instance);
    if tag == 0u {
        return 1.0;
    }
    let low = clamp(1.0 - (p.y - bitcast<f32>(tag)) / GROUND_REACH, 0.0, 1.0);
    return 1.0 - GROUND_SHADE * low * low * (1.0 - 0.8 * max(n.y, 0.0));
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
    // UV_0.y: how much of the sky the rest of the course (`ao.rs`) or of its set piece (`decor.rs`) hides from
    // the vertex.
    let piece_occ = clamp(in.uv.y, 0.0, 1.0);
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
            let dp = p * surface.detail.x;
            let gx = dpdx(dp);
            let gy = dpdy(dp);
            // How much of the detail shows: texels a pixel along the footprint's longer axis (so it goes at
            // grazing angles too) against the size of the kind's features. Finer than a few texels a pixel, the
            // tile's repeats read as a grid and its bumps as moiré (worse after the upscaler's sharpening): the
            // detail gives way to the tile's average, gone well before its features are a pixel wide.
            let texels = max(length(gx), length(gy)) * DETAIL_SIZE;
            let fade = 1.0 - smoothstep(0.5, 2.5, log2(max(texels, 1e-4)) - surface.extra.w);
            let mean = sample_mean(slot);
            height = mean.b;
            rough = mean.a;
            // (Per pixel: the samples take their gradients as given, no derivatives in here.)
            if fade > 0.0 {
                let nw = normalize(in.world_normal);
                let n = vec3(dot(nw, ax), dot(nw, ay), dot(nw, az));
                // Sharp blending: a projection is used only where it is stretched by less than ~1.7× (more drew
                // streaks along round sides); the ones that barely show are not sampled, the rest renormalised.
                var w = pow(abs(n), vec3(6.0));
                w /= w.x + w.y + w.z;
                w = max(w - 0.1, vec3(0.0));
                w /= w.x + w.y + w.z;
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
                height = mix(mean.b, tx.b * w.x + ty.b * w.y + tz.b * w.z, fade);
                rough = mix(mean.a, tx.a * w.x + ty.a * w.y + tz.a * w.z, fade);
                let nx = vec3(0.0, tx.y * 2.0 - 1.0, tx.x * 2.0 - 1.0);
                let ny = vec3(ty.x * 2.0 - 1.0, 0.0, ty.y * 2.0 - 1.0);
                let nz = vec3(tz.x * 2.0 - 1.0, tz.y * 2.0 - 1.0, 0.0);
                let d = (nx * w.x + ny * w.y + nz * w.z) * (surface.detail.y * fade);
                pbr_input.N = normalize(pbr_input.N + d.x * ax + d.y * ay + d.z * az);
            }
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
            let fws = fwidth(sx);
            let fwq = fwidth(q);
            let fw = min(0.5, fws);
            var stp = smoothstep(0.25 - fw, 0.25 + fw, abs(fract(sx) - 0.5));
            // Where a period shrinks to a couple of pixels the edges alias into moiré: the pattern goes over to
            // its average share of the second tone (half; the dots cover a fifth of each cell).
            var blur = smoothstep(0.2, 0.45, fws);
            var share = 0.5;
            if kind == 1 {
                let fw2 = min(vec2(0.5), fwq);
                let sq = smoothstep(vec2(0.25) - fw2, vec2(0.25) + fw2, abs(fract(q * 0.5) - 0.5));
                stp = sq.x + sq.y - 2.0 * sq.x * sq.y;
                // (Its squares are a whole unit wide, the stripes half one.)
                blur = smoothstep(0.2, 0.45, max(fwq.x, fwq.y) * 0.5);
            } else if kind == 2 {
                let r = length(fract(q) - 0.5);
                let fwr = min(0.5, fwidth(r));
                stp = smoothstep(0.26 - fwr, 0.26 + fwr, r);
                blur = smoothstep(0.2, 0.45, max(fwq.x, fwq.y));
                share = 1.0 - 3.14159265 * 0.26 * 0.26;
            }
            stp = mix(stp, share, blur);
            color = vec4(color.rgb * mix(surface.c1.rgb, surface.c2.rgb, stp), color.a);
        }
    }
    color = vec4(color.rgb * mix(1.0 - surface.detail.w, 1.0, smoothstep(0.1, 0.7, height)), color.a);
    color = vec4(mix(color.rgb, vec3(1.0), smoothstep(0.35, 1.0, rough) * surface.extra.y), color.a);
    // Ambient occlusion: what moves near, the ground at a model's foot, and what is baked into merged geometry
    // (that one dims the sun a little too, in the colour). By the surface's own normal, not the detail's.
    let geo_n = normalize(in.world_normal);
    let wp = in.world_position.xyz;
    var sky = moving_sky(slot, wp, geo_n) * ground_sky(in.instance_index, wp, geo_n);
#ifdef OBJECT_FRAME
    sky *= 1.0 - piece_occ;
    color = vec4(color.rgb * (1.0 - piece_occ * BAKED_ON_SUN), color.a);
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, color);
    pbr_input.material.perceptual_roughness = clamp(
        pbr_input.material.perceptual_roughness * mix(1.0 - surface.detail.z, 1.0 + surface.detail.z, rough),
        0.04,
        1.0,
    );
    // (The ambient light's diffuse part, and its reflections as Bevy takes them from SSAO.)
    pbr_input.diffuse_occlusion *= sky;
    let nv = max(dot(pbr_input.N, pbr_input.V), 1e-4);
    let roughness = pbr_input.material.perceptual_roughness * pbr_input.material.perceptual_roughness;
    pbr_input.specular_occlusion *= saturate(pow(nv + sky, exp2(-16.0 * roughness - 1.0)) - 1.0 + sky);

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
