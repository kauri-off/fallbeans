// Cloth: the vertex stage of a pennant waving in the wind (`cloth.rs`). A wave runs from the pole to the
// tip along the length (local x), bending the cloth across it (local z), still at the pole and widest at
// the tip, stronger in gusts; the normals bend with it. The prepass runs the same, so shadows wave too.
#import bevy_pbr::{
    mesh_functions,
    view_transformations::position_world_to_clip,
}
#import bevy_render::globals::Globals

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::{Vertex, VertexOutput}
// (The prepass binds the globals at 1.)
@group(0) @binding(1) var<uniform> prepass_globals: Globals;
#else
#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings::globals,
}
#endif

struct Cloth {
    // Pole edge along the length (local x), 1 / length, swing at the tip, waves along the length.
    shape: vec4<f32>,
    // Wave speed (rad/s), flutter share, phase per unit of height (rad), unused.
    motion: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(53) var<uniform> cloth: Cloth;

// Where the wind blows (world x, z): `WIND` in `cloth.rs`.
const WIND: vec2<f32> = vec2(0.9439, 0.3303);
const TAU: f32 = 6.2831853;

// Time and the last frame's length (s).
fn clock() -> vec2<f32> {
#ifdef PREPASS_PIPELINE
    return vec2(prepass_globals.time, prepass_globals.delta_time);
#else
    return vec2(globals.time, globals.delta_time);
#endif
}

// The wind's strength at a place (0.2…1): `gust` in `cloth.rs`.
fn gust(t: f32, p: vec2<f32>) -> f32 {
    let d = dot(p, WIND);
    return 0.6 + 0.25 * sin(t * 0.9 - d * 0.11) + 0.15 * sin(t * 2.3 - d * 0.31 + 1.7);
}

// The offset across the cloth at a local position, and its slopes along x and y.
fn wave(p: vec3<f32>, t: f32, origin: vec3<f32>) -> vec3<f32> {
    let inv_len = cloth.shape.y;
    let s = clamp((p.x - cloth.shape.x) * inv_len, 0.0, 1.0);
    let g = cloth.shape.z * (0.45 + 0.55 * gust(t, origin.xz));
    // Swing growing from the pole to the tip, and its rate along x.
    let a = g * s * (0.4 + 0.6 * s);
    let da = g * (0.4 + 1.2 * s) * inv_len;
    let k = cloth.shape.w * TAU * inv_len;
    let ky = cloth.motion.z;
    let th = k * p.x + ky * p.y - cloth.motion.x * t + dot(origin.xz, vec2(1.7, 2.3));
    // A faster, shorter flutter on top of the main wave.
    let th2 = 2.3 * th + 1.3;
    let f = cloth.motion.y;
    let w = sin(th) + f * sin(th2);
    let dw = cos(th) + f * 2.3 * cos(th2);
    return vec3(a * w, da * w + a * dw * k, a * dw * ky);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let origin = world_from_local[3].xyz;
    let c = clock();
    let w = wave(vertex.position, c.x, origin);
    let p = vec3(vertex.position.xy, vertex.position.z + w.x);

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(p, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif

#ifdef PREPASS_PIPELINE
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    let n = vertex.normal;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vec3(n.x - w.y * n.z, n.y - w.z * n.z, n.z),
        vertex.instance_index,
    );
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        vertex.instance_index,
    );
#endif
#endif
#ifdef MOTION_VECTOR_PREPASS
    // Where the vertex was last frame, as it waved then.
    let prev_model = mesh_functions::get_previous_world_from_local(vertex.instance_index);
    let pw = wave(vertex.position, c.x - c.y, prev_model[3].xyz);
    out.previous_world_position = mesh_functions::mesh_position_local_to_world(
        prev_model,
        vec4(vertex.position.xy, vertex.position.z + pw.x, 1.0),
    );
#endif
#else
#ifdef VERTEX_NORMALS
    let n = vertex.normal;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vec3(n.x - w.y * n.z, n.y - w.z * n.z, n.z),
        vertex.instance_index,
    );
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        vertex.instance_index,
    );
#endif
#endif

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex.instance_index,
        world_from_local[3],
    );
#endif
    return out;
}
