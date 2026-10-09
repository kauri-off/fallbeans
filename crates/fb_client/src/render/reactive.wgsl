// The reactive mask (`reactive.rs`) from the main pass's alpha: 1 − alpha, a little stronger, short of all.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var color: texture_2d<f32>;

// Faint marks count a little more (a fading puff still trails); FSR's own generator stops at 0.9: some history
// stays, against the shimmer of what is wholly new each frame.
const GAIN: f32 = 1.5;
const MOST: f32 = 0.9;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let a = textureLoad(color, vec2<i32>(in.position.xy), 0).a;
    return vec4(min((1.0 - clamp(a, 0.0, 1.0)) * GAIN, MOST), 0.0, 0.0, 1.0);
}
