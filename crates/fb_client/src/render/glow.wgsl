// Light added in one colour (`GlowMaterial` in `vfx.rs`); its strength marks the reactive mask (`reactive.rs`).
#import bevy_pbr::forward_io::VertexOutput

struct Glow {
    // rgb: colour; a: strength.
    color: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> m: Glow;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4(m.color.rgb * m.color.a, m.color.a);
}
