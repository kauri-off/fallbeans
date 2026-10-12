// The half-resolution haze (`fog.wgsl`'s `low`) scaled up: each pixel takes its four nearest texels, those whose
// depth is like its own the most.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var low: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.position.xy);
    let z = textureLoad(depth, px, 0);
    let last = vec2<i32>(textureDimensions(low)) - 1;
    let at = in.position.xy * 0.5 - 0.5;
    let base = vec2<i32>(floor(at));
    let f = fract(at);
    var sum = vec4(0.0);
    var total = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2(k & 1, k >> 1);
        let t = clamp(base + o, vec2(0), last);
        let zt = textureLoad(depth, t * 2, 0);
        let bilinear = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let near = abs(zt - z) / max(max(zt, z), 1e-6);
        let w = bilinear / (near + 0.01) + 1e-5;
        sum += textureLoad(low, t, 0) * w;
        total += w;
    }
    return sum / total;
}
