// AMD FidelityFX Super Resolution 1, EASU (port of fsr.ts; github.com/GPUOpen-Effects/FidelityFX-FSR, MIT):
// edge-adaptive spatial upsampling, 12 taps, a Lanczos-like kernel stretched along local edges, deringed.
// The scene was drawn into the top-left `in_size` of the source; the output fills the whole target.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Easu {
    in_size: vec2<f32>,
    out_size: vec2<f32>,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> u: Easu;

fn tap(p: vec2<i32>) -> vec3<f32> {
    let mx = vec2<i32>(u.in_size) - 1;
    return textureLoad(src, clamp(p, vec2(0), mx), 0).rgb;
}

// Luma times 2.
fn luma(c: vec3<f32>) -> f32 {
    return c.g + 0.5 * (c.r + c.b);
}

struct Acc {
    dir: vec2<f32>,
    len: f32,
}

fn set_f(a: Acc, w: f32, la: f32, lb: f32, lc: f32, ld: f32, le: f32) -> Acc {
    var o = a;
    var len_x = max(abs(ld - lc), abs(lc - lb));
    let dir_x = ld - lb;
    o.dir.x += dir_x * w;
    len_x = clamp(abs(dir_x) / max(len_x, 1e-5), 0.0, 1.0);
    o.len += len_x * len_x * w;
    var len_y = max(abs(le - lc), abs(lc - la));
    let dir_y = le - la;
    o.dir.y += dir_y * w;
    len_y = clamp(abs(dir_y) / max(len_y, 1e-5), 0.0, 1.0);
    o.len += len_y * len_y * w;
    return o;
}

fn weight(off: vec2<f32>, dir: vec2<f32>, len: vec2<f32>, lob: f32, clp: f32) -> f32 {
    let v = vec2(off.x * dir.x + off.y * dir.y, off.x * -dir.y + off.y * dir.x) * len;
    let d2 = min(dot(v, v), clp);
    var wb = 0.4 * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb *= wb;
    wa *= wa;
    wb = 1.5625 * wb - 0.5625;
    return wb * wa;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // Position in the input (texel centres at integers), split into the texel and the fraction.
    var pp = in.position.xy * (u.in_size / u.out_size) - 0.5;
    let fp = floor(pp);
    pp -= fp;
    let ip = vec2<i32>(fp);
    //    b c
    //  e f g h
    //  i j k l
    //    n o
    let b = tap(ip + vec2(0, -1));
    let c = tap(ip + vec2(1, -1));
    let e = tap(ip + vec2(-1, 0));
    let f = tap(ip);
    let g = tap(ip + vec2(1, 0));
    let h = tap(ip + vec2(2, 0));
    let i = tap(ip + vec2(-1, 1));
    let j = tap(ip + vec2(0, 1));
    let k = tap(ip + vec2(1, 1));
    let l = tap(ip + vec2(2, 1));
    let n = tap(ip + vec2(0, 2));
    let o = tap(ip + vec2(1, 2));
    let bl = luma(b);
    let cl = luma(c);
    let el = luma(e);
    let fl = luma(f);
    let gl = luma(g);
    let hl = luma(h);
    let il = luma(i);
    let jl = luma(j);
    let kl = luma(k);
    let ll = luma(l);
    let nl = luma(n);
    let ol = luma(o);

    // Direction and length of the local edge, bilinearly accumulated over the 4 nearest texels.
    var a = Acc(vec2(0.0), 0.0);
    a = set_f(a, (1.0 - pp.x) * (1.0 - pp.y), bl, el, fl, gl, jl);
    a = set_f(a, pp.x * (1.0 - pp.y), cl, fl, gl, hl, kl);
    a = set_f(a, (1.0 - pp.x) * pp.y, fl, il, jl, kl, nl);
    a = set_f(a, pp.x * pp.y, gl, jl, kl, ll, ol);
    var dir = a.dir;
    let dir2 = dir * dir;
    var dir_r = dir2.x + dir2.y;
    let zro = dir_r < 1.0 / 32768.0;
    dir_r = select(inverseSqrt(dir_r), 1.0, zro);
    dir.x = select(dir.x, 1.0, zro);
    dir *= dir_r;
    var len = a.len * 0.5;
    len *= len;
    let stretch = dot(dir, dir) / max(abs(dir.x), abs(dir.y));
    let len2 = vec2(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    let lob = 0.5 - 0.29 * len;
    let clp = 1.0 / lob;

    var ac = vec3(0.0);
    var aw = 0.0;
    let offs = array<vec2<f32>, 12>(
        vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(0.0, 1.0), vec2(0.0, 0.0), vec2(-1.0, 0.0),
        vec2(1.0, 1.0), vec2(2.0, 1.0), vec2(2.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 2.0), vec2(0.0, 2.0),
    );
    let cols = array<vec3<f32>, 12>(b, c, i, j, f, e, k, l, h, g, o, n);
    for (var t = 0; t < 12; t++) {
        let w = weight(offs[t] - pp, dir, len2, lob, clp);
        aw += w;
        ac += cols[t] * w;
    }
    // Dering: within the 4 nearest texels.
    let mn = min(min(f, g), min(j, k));
    let mx = max(max(f, g), max(j, k));
    return vec4(min(mx, max(mn, ac / aw)), 1.0);
}
