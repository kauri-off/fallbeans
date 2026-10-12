// Bevy's bloom filters (Call of Duty: Advanced Warfare's): a 13-tap downsample, Karis-averaged on the first one
// against fireflies, and a 3x3 tent upsample.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

fn karis_average(c: vec3<f32>) -> f32 {
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)) / 4.0;
    return 1.0 / (1.0 + luma);
}

fn taps(uv: vec2<f32>, first: bool) -> vec3<f32> {
    let a = textureSample(input_texture, s, uv, vec2<i32>(-2, 2)).rgb;
    let b = textureSample(input_texture, s, uv, vec2<i32>(0, 2)).rgb;
    let c = textureSample(input_texture, s, uv, vec2<i32>(2, 2)).rgb;
    let d = textureSample(input_texture, s, uv, vec2<i32>(-2, 0)).rgb;
    let e = textureSample(input_texture, s, uv).rgb;
    let f = textureSample(input_texture, s, uv, vec2<i32>(2, 0)).rgb;
    let g = textureSample(input_texture, s, uv, vec2<i32>(-2, -2)).rgb;
    let h = textureSample(input_texture, s, uv, vec2<i32>(0, -2)).rgb;
    let i = textureSample(input_texture, s, uv, vec2<i32>(2, -2)).rgb;
    let j = textureSample(input_texture, s, uv, vec2<i32>(-1, 1)).rgb;
    let k = textureSample(input_texture, s, uv, vec2<i32>(1, 1)).rgb;
    let l = textureSample(input_texture, s, uv, vec2<i32>(-1, -1)).rgb;
    let m = textureSample(input_texture, s, uv, vec2<i32>(1, -1)).rgb;
    if first {
        var g0 = (a + b + d + e) * (0.125 / 4.0);
        var g1 = (b + c + e + f) * (0.125 / 4.0);
        var g2 = (d + e + g + h) * (0.125 / 4.0);
        var g3 = (e + f + h + i) * (0.125 / 4.0);
        var g4 = (j + k + l + m) * (0.5 / 4.0);
        g0 *= karis_average(g0);
        g1 *= karis_average(g1);
        g2 *= karis_average(g2);
        g3 *= karis_average(g3);
        g4 *= karis_average(g4);
        return g0 + g1 + g2 + g3 + g4;
    }
    return (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (e + j + k + l + m) * 0.125;
}

@fragment
fn downsample_first(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // (Kept off zero, which would spread as black boxes, and off infinity.)
    return vec4<f32>(clamp(taps(in.uv, true), vec3<f32>(0.0001), vec3<f32>(3.4e37)), 1.0);
}

@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(taps(in.uv, false), 1.0);
}

@fragment
fn upsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let px = 1.0 / vec2<f32>(textureDimensions(input_texture));
    let uv = in.uv;
    let a = textureSample(input_texture, s, uv + vec2<f32>(-px.x, px.y)).rgb;
    let b = textureSample(input_texture, s, uv + vec2<f32>(0.0, px.y)).rgb;
    let c = textureSample(input_texture, s, uv + vec2<f32>(px.x, px.y)).rgb;
    let d = textureSample(input_texture, s, uv + vec2<f32>(-px.x, 0.0)).rgb;
    let e = textureSample(input_texture, s, uv).rgb;
    let f = textureSample(input_texture, s, uv + vec2<f32>(px.x, 0.0)).rgb;
    let g = textureSample(input_texture, s, uv + vec2<f32>(-px.x, -px.y)).rgb;
    let h = textureSample(input_texture, s, uv + vec2<f32>(0.0, -px.y)).rgb;
    let i = textureSample(input_texture, s, uv + vec2<f32>(px.x, -px.y)).rgb;
    return vec4<f32>(e * 0.25 + (b + d + f + h) * 0.125 + (a + c + g + i) * 0.0625, 1.0);
}
