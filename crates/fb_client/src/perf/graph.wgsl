// The F4 graph: a bar per frame (green ≤ 60 fps, yellow ≤ 30, red), GPU time as a cyan line.
#import bevy_ui::ui_vertex_output::UiVertexOutput

struct Graph {
    frame: array<vec4<f32>, 64>,
    gpu: array<vec4<f32>, 64>,
    // x: ms at the top, y: frames filled (the rest is empty).
    params: vec4<f32>,
}

@group(1) @binding(0) var<uniform> graph: Graph;

fn at(i: u32, gpu: bool) -> f32 {
    if gpu {
        return graph.gpu[i / 4u][i % 4u];
    }
    return graph.frame[i / 4u][i % 4u];
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let n = 256.0;
    let i = u32(clamp(in.uv.x * n, 0.0, n - 1.0));
    let top = graph.params.x;
    let px = top / max(in.size.y, 1.0);
    let y = (1.0 - in.uv.y) * top;
    var c = vec4(0.0, 0.0, 0.0, 0.5);
    if f32(i) < n - graph.params.y {
        return c;
    }
    let f = at(i, false);
    let g = at(i, true);
    if y <= f {
        if f <= 16.9 {
            c = vec4(0.25, 0.85, 0.35, 0.85);
        } else if f <= 33.4 {
            c = vec4(0.95, 0.8, 0.2, 0.9);
        } else {
            c = vec4(0.95, 0.25, 0.2, 0.95);
        }
    }
    if abs(y - 16.67) < px * 0.6 || abs(y - 6.94) < px * 0.6 {
        c = mix(c, vec4(1.0, 1.0, 1.0, 1.0), 0.35);
    }
    if g > 0.0 && abs(y - g) < px * 1.2 {
        c = vec4(0.3, 0.85, 1.0, 1.0);
    }
    return c;
}
