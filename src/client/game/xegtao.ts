import * as THREE from 'three';
import type { FullScreenQuad } from 'three/addons/postprocessing/Pass.js';

/**
 * Ground-truth ambient occlusion after Intel's XeGTAO (github.com/GameTechDev/XeGTAO), on WebGL2,
 * at half resolution:
 *   1. prefilter: the scene's depth → linear view depth, and a hierarchy of 5 levels filtered the
 *      XeGTAO way (far samples read coarse levels: far fewer cache misses for wide radii);
 *   2. main pass: horizon search in a few screen-space slices (normals rebuilt in place from depth),
 *      noise from a Hilbert curve + R2 sequence (fixed: see PostPipeline.renderAo), and edges
 *      packed next to the result for the denoiser;
 *   3. edge-aware 3×3 denoise, three passes (the noise is not averaged over frames).
 * The composite pass then upsamples it depth-aware (see postfx.ts) and applies it.
 */

export interface AoQuality {
  slices: number;
  steps: number;
  denoise: number;
}

export const AO_QUALITY = {
  medium: { slices: 2, steps: 2, denoise: 3 },
  high: { slices: 3, steps: 3, denoise: 3 },
} satisfies Record<string, AoQuality>;

/** Depth hierarchy levels (XE_GTAO_DEPTH_MIP_LEVELS). */
const MIPS = 5;
/** Effect radius (m), before XeGTAO's radius multiplier. */
const RADIUS = 0.45;
const RADIUS_MULTIPLIER = 1.457;
const FALLOFF_RANGE = 0.615;
const SAMPLE_DISTRIBUTION_POWER = 2;
const FINAL_VALUE_POWER = 2.2;
const DEPTH_MIP_SAMPLING_OFFSET = 3.3;
/** Visibility is stored divided by this (room above 1 before the denoiser). */
const OCCLUSION_TERM_SCALE = 1.5;
const DENOISE_BLUR_BETA = 1.2;

const VERT = 'void main(){ gl_Position = vec4(position.xy, 0.0, 1.0); }';

const common = (m: THREE.ShaderMaterial) => {
  m.depthTest = false;
  m.depthWrite = false;
  return m;
};

/** Level 0: one full-resolution depth per half-resolution texel, as positive view depth. */
const PREFILTER0 = /* glsl */ `
uniform sampler2D tDepth;
uniform ivec2 uSrcMax;
uniform vec2 uCam;
void main() {
  float d = texelFetch(tDepth, min(ivec2(gl_FragCoord.xy) * 2, uSrcMax), 0).x;
  gl_FragColor = vec4(uCam.x * uCam.y / (uCam.y - d * (uCam.y - uCam.x)), 0.0, 0.0, 1.0);
}`;

/** Levels 1…4: XeGTAO_DepthMIPFilter over the 2×2 texels below. */
const PREFILTER_N = /* glsl */ `
uniform sampler2D tSrc;
uniform ivec2 uSrcMax;
uniform vec2 uFalloff;
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy) * 2;
  float d0 = texelFetch(tSrc, min(p, uSrcMax), 0).x;
  float d1 = texelFetch(tSrc, min(p + ivec2(1, 0), uSrcMax), 0).x;
  float d2 = texelFetch(tSrc, min(p + ivec2(0, 1), uSrcMax), 0).x;
  float d3 = texelFetch(tSrc, min(p + ivec2(1, 1), uSrcMax), 0).x;
  float mx = max(max(d0, d1), max(d2, d3));
  vec4 w = clamp((mx - vec4(d0, d1, d2, d3)) * uFalloff.x + uFalloff.y, 0.0, 1.0);
  gl_FragColor = vec4(dot(w, vec4(d0, d1, d2, d3)) / (w.x + w.y + w.z + w.w), 0.0, 0.0, 1.0);
}`;

const MAIN = /* glsl */ `
uniform sampler2D tD0;
uniform sampler2D tD1;
uniform sampler2D tD2;
uniform sampler2D tD3;
uniform sampler2D tD4;
uniform vec2 uPixel;
uniform ivec2 uMax;
uniform vec2 uNdcMul;
uniform vec2 uNdcAdd;
uniform vec2 uNdcMulPix;
uniform float uFrame;
const float PI = 3.1415926535;
const float PI_HALF = 1.5707963268;

float depthAt(vec2 uv, float mip) {
  if (mip < 0.5) return textureLod(tD0, uv, 0.0).x;
  if (mip < 1.5) return textureLod(tD1, uv, 0.0).x;
  if (mip < 2.5) return textureLod(tD2, uv, 0.0).x;
  if (mip < 3.5) return textureLod(tD3, uv, 0.0).x;
  return textureLod(tD4, uv, 0.0).x;
}

vec3 viewPos(vec2 uv, float z) {
  return vec3((uNdcMul * uv + uNdcAdd) * z, z);
}

float fastAcos(float inX) {
  float x = min(abs(inX), 1.0);
  float res = (-0.156583 * x + PI_HALF) * sqrt(1.0 - x);
  return inX >= 0.0 ? res : PI - res;
}

// Edges (left, right, top, bottom) from depth differences, slope-adjusted.
vec4 edgesOf(float c, float l, float r, float t, float b) {
  vec4 e = vec4(l, r, t, b) - c;
  float slopeLR = (e.y - e.x) * 0.5;
  float slopeTB = (e.w - e.z) * 0.5;
  vec4 adj = e + vec4(slopeLR, -slopeLR, slopeTB, -slopeTB);
  e = min(abs(e), abs(adj));
  return clamp(1.25 - e / (c * 0.011), 0.0, 1.0);
}

// XE_HILBERT_LEVEL 6: a 64×64 Hilbert curve.
uint hilbert(uint x, uint y) {
  uint index = 0u;
  for (uint level = 32u; level > 0u; level /= 2u) {
    uint rx = (x & level) > 0u ? 1u : 0u;
    uint ry = (y & level) > 0u ? 1u : 0u;
    index += level * level * ((3u * rx) ^ ry);
    if (ry == 0u) {
      if (rx == 1u) { x = 63u - x; y = 63u - y; }
      uint t = x; x = y; y = t;
    }
  }
  return index;
}

void main() {
  ivec2 pix = ivec2(gl_FragCoord.xy);
  vec2 uv = gl_FragCoord.xy * uPixel;
  float z = texelFetch(tD0, pix, 0).x;
  float zl = texelFetch(tD0, max(pix - ivec2(1, 0), ivec2(0)), 0).x;
  float zr = texelFetch(tD0, min(pix + ivec2(1, 0), uMax), 0).x;
  float zt = texelFetch(tD0, min(pix + ivec2(0, 1), uMax), 0).x;
  float zb = texelFetch(tD0, max(pix - ivec2(0, 1), ivec2(0)), 0).x;
  vec4 edges = edgesOf(z, zl, zr, zt, zb);
  // XeGTAO_PackEdges: 2 bits per edge.
  vec4 q = floor(clamp(edges, 0.0, 1.0) * 2.9 + 0.5);
  float packed = dot(q, vec4(64.0, 16.0, 4.0, 1.0)) / 255.0;

  float radius = ${(RADIUS * RADIUS_MULTIPLIER).toFixed(5)};
  // Screen-space radius (pixels): too small to matter (far away, the sky) → unoccluded.
  float screenRadius = radius / (z * uNdcMulPix.x);
  if (screenRadius < 1.3) {
    gl_FragColor = vec4(1.0 / ${OCCLUSION_TERM_SCALE.toFixed(1)}, packed, 0.0, 1.0);
    return;
  }

  // Normal rebuilt in place (XeGTAO_CalculateNormal).
  vec3 C = viewPos(uv, z);
  vec3 L = normalize(viewPos(uv - vec2(uPixel.x, 0.0), zl) - C);
  vec3 R = normalize(viewPos(uv + vec2(uPixel.x, 0.0), zr) - C);
  vec3 T = normalize(viewPos(uv + vec2(0.0, uPixel.y), zt) - C);
  vec3 B = normalize(viewPos(uv - vec2(0.0, uPixel.y), zb) - C);
  vec4 acc = clamp(vec4(edges.x * edges.z, edges.z * edges.y, edges.y * edges.w, edges.w * edges.x) + 0.01, 0.0, 1.0);
  vec3 normal = normalize(acc.x * cross(L, T) + acc.y * cross(T, R) + acc.z * cross(R, B) + acc.w * cross(B, L));

  vec3 center = viewPos(uv, z * 0.99999);
  vec3 viewVec = normalize(-center);

  // Spatio-temporal noise: Hilbert index driving the R2 sequence, shifted every frame.
  uint index = hilbert(uint(pix.x) & 63u, uint(pix.y) & 63u) + 288u * uint(uFrame);
  vec2 noise = fract(0.5 + float(index) * vec2(0.75487766624669276, 0.56984029099805327));

  float falloffRange = ${FALLOFF_RANGE.toFixed(3)} * radius;
  float falloffFrom = radius * (1.0 - ${FALLOFF_RANGE.toFixed(3)});
  float falloffMul = -1.0 / falloffRange;
  float falloffAdd = falloffFrom / falloffRange + 1.0;

  float visibility = clamp((10.0 - screenRadius) / 100.0, 0.0, 1.0) * 0.5;
  float minS = 1.3 / screenRadius;

  for (int slice = 0; slice < SLICES; slice++) {
    float sliceK = (float(slice) + noise.x) / float(SLICES);
    float phi = sliceK * PI;
    float cosPhi = cos(phi);
    float sinPhi = sin(phi);
    vec2 omega = vec2(cosPhi, sinPhi) * screenRadius;
    vec3 dirVec = vec3(cosPhi, sinPhi, 0.0);
    vec3 orthoDir = dirVec - dot(dirVec, viewVec) * viewVec;
    vec3 axis = normalize(cross(orthoDir, viewVec));
    vec3 projN = normal - axis * dot(normal, axis);
    float signNorm = sign(dot(orthoDir, projN));
    float projNLen = length(projN);
    float cosNorm = clamp(dot(projN, viewVec) / max(projNLen, 1e-6), 0.0, 1.0);
    float n = signNorm * fastAcos(cosNorm);
    float lowCos0 = cos(n + PI_HALF);
    float lowCos1 = cos(n - PI_HALF);
    float hc0 = lowCos0;
    float hc1 = lowCos1;
    for (int st = 0; st < STEPS; st++) {
      float stepNoise = fract(noise.y + float(slice + st * STEPS) * 0.6180339887498948);
      float s = (float(st) + stepNoise) / float(STEPS);
      s = pow(s, ${SAMPLE_DISTRIBUTION_POWER.toFixed(1)}) + minS;
      vec2 offset = s * omega;
      float mip = clamp(floor(log2(length(offset)) - ${DEPTH_MIP_SAMPLING_OFFSET.toFixed(2)} + 0.5), 0.0, ${(MIPS - 1).toFixed(1)});
      offset = floor(offset + 0.5) * uPixel;
      vec2 uv0 = uv + offset;
      vec2 uv1 = uv - offset;
      vec3 d0 = viewPos(uv0, depthAt(uv0, mip)) - center;
      vec3 d1 = viewPos(uv1, depthAt(uv1, mip)) - center;
      float l0 = length(d0);
      float l1 = length(d1);
      float w0 = clamp(l0 * falloffMul + falloffAdd, 0.0, 1.0);
      float w1 = clamp(l1 * falloffMul + falloffAdd, 0.0, 1.0);
      float shc0 = mix(lowCos0, dot(d0 / max(l0, 1e-6), viewVec), w0);
      float shc1 = mix(lowCos1, dot(d1 / max(l1, 1e-6), viewVec), w1);
      hc0 = max(hc0, shc0);
      hc1 = max(hc1, shc1);
    }
    projNLen = mix(projNLen, 1.0, 0.05);
    float h0 = -fastAcos(hc1);
    float h1 = fastAcos(hc0);
    float sinN = sin(n);
    float iarc0 = (cosNorm + 2.0 * h0 * sinN - cos(2.0 * h0 - n)) / 4.0;
    float iarc1 = (cosNorm + 2.0 * h1 * sinN - cos(2.0 * h1 - n)) / 4.0;
    visibility += projNLen * (iarc0 + iarc1);
  }
  visibility /= float(SLICES);
  visibility = max(0.03, pow(max(visibility, 0.0), ${FINAL_VALUE_POWER.toFixed(1)}));
  gl_FragColor = vec4(clamp(visibility / ${OCCLUSION_TERM_SCALE.toFixed(1)}, 0.0, 1.0), packed, 0.0, 1.0);
}`;

/** XeGTAO_Denoise for one pixel: edge-aware 3×3 blur (edges kept in G for the next pass). */
const DENOISE = /* glsl */ `
uniform sampler2D tSrc;
uniform ivec2 uMax;
uniform float uBlur;
uniform float uFinal;
vec4 unpack(float v) {
  float p = floor(v * 255.0 + 0.5);
  return clamp(vec4(floor(p / 64.0), mod(floor(p / 16.0), 4.0), mod(floor(p / 4.0), 4.0), mod(p, 4.0)) / 3.0, 0.0, 1.0);
}
vec2 at(ivec2 p) { return texelFetch(tSrc, clamp(p, ivec2(0), uMax), 0).xy; }
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  vec2 c = at(p);
  vec2 l = at(p + ivec2(-1, 0));
  vec2 r = at(p + ivec2(1, 0));
  vec2 t = at(p + ivec2(0, 1));
  vec2 b = at(p + ivec2(0, -1));
  vec4 eC = unpack(c.y);
  vec4 eL = unpack(l.y);
  vec4 eR = unpack(r.y);
  vec4 eT = unpack(t.y);
  vec4 eB = unpack(b.y);
  // Edges made symmetric with the neighbours' own.
  eC *= vec4(eL.y, eR.x, eT.w, eB.z);
  // A little leaking where there are 3 or 4 edges (less aliasing).
  float edginess = (clamp(4.0 - 2.5 - dot(eC, vec4(1.0)), 0.0, 1.0) / (4.0 - 2.5)) * 0.5;
  eC = clamp(eC + edginess, 0.0, 1.0);
  const float diag = 0.85 * 0.5;
  float wTL = diag * (eC.x * eL.z + eC.z * eT.x);
  float wTR = diag * (eC.z * eT.y + eC.y * eR.z);
  float wBL = diag * (eC.w * eB.x + eC.x * eL.w);
  float wBR = diag * (eC.y * eR.w + eC.w * eB.y);
  float sumW = uBlur;
  float sum = c.x * sumW;
  sum += l.x * eC.x; sumW += eC.x;
  sum += r.x * eC.y; sumW += eC.y;
  sum += t.x * eC.z; sumW += eC.z;
  sum += b.x * eC.w; sumW += eC.w;
  sum += at(p + ivec2(-1, 1)).x * wTL; sumW += wTL;
  sum += at(p + ivec2(1, 1)).x * wTR; sumW += wTR;
  sum += at(p + ivec2(-1, -1)).x * wBL; sumW += wBL;
  sum += at(p + ivec2(1, -1)).x * wBR; sumW += wBR;
  float ao = sum / sumW;
  if (uFinal > 0.5) ao = clamp(ao * ${OCCLUSION_TERM_SCALE.toFixed(1)}, 0.0, 1.0);
  gl_FragColor = vec4(ao, c.y, 0.0, 1.0);
}`;

const rt = (w: number, h: number, format: THREE.PixelFormat, type: THREE.TextureDataType) =>
  new THREE.WebGLRenderTarget(w, h, {
    format,
    type,
    minFilter: THREE.NearestFilter,
    magFilter: THREE.NearestFilter,
    generateMipmaps: false,
    depthBuffer: false,
  });

export class XeGTAO {
  /** Linear view depth at the AO resolution (level 0 of the hierarchy): the upsampler compares with it. */
  get depth(): THREE.Texture {
    return this.mips[0]!.texture;
  }
  /** The occlusion (R: visibility 0…1) after render(). */
  texture: THREE.Texture | null = null;
  /** AO resolution. */
  readonly size = new THREE.Vector2();
  /** Render resolution (of the scene's depth). */
  private readonly src = new THREE.Vector2();

  private mips: THREE.WebGLRenderTarget[] = [];
  private ping: THREE.WebGLRenderTarget | null = null;
  private pong: THREE.WebGLRenderTarget | null = null;
  private readonly prefilter0 = common(
    new THREE.ShaderMaterial({
      uniforms: { tDepth: { value: null }, uSrcMax: { value: new THREE.Vector2() }, uCam: { value: new THREE.Vector2() } },
      vertexShader: VERT,
      fragmentShader: PREFILTER0,
    }),
  );
  private readonly prefilterN = common(
    new THREE.ShaderMaterial({
      uniforms: { tSrc: { value: null }, uSrcMax: { value: new THREE.Vector2() }, uFalloff: { value: new THREE.Vector2() } },
      vertexShader: VERT,
      fragmentShader: PREFILTER_N,
    }),
  );
  private main: THREE.ShaderMaterial;
  private readonly denoise = common(
    new THREE.ShaderMaterial({
      uniforms: { tSrc: { value: null }, uMax: { value: new THREE.Vector2() }, uBlur: { value: 1 }, uFinal: { value: 0 } },
      vertexShader: VERT,
      fragmentShader: DENOISE,
    }),
  );

  constructor(private quality: AoQuality) {
    this.main = this.mainMaterial();
    // Mip filter weights (XeGTAO_DepthMIPFilter: 0.75 × the effect radius).
    const r = 0.75 * RADIUS * RADIUS_MULTIPLIER;
    const range = FALLOFF_RANGE * r;
    const from = r * (1 - FALLOFF_RANGE);
    this.prefilterN.uniforms.uFalloff!.value.set(-1 / range, from / range + 1);
  }

  private mainMaterial() {
    const u: Record<string, THREE.IUniform> = {
      uPixel: { value: new THREE.Vector2() },
      uMax: { value: new THREE.Vector2() },
      uNdcMul: { value: new THREE.Vector2() },
      uNdcAdd: { value: new THREE.Vector2() },
      uNdcMulPix: { value: new THREE.Vector2() },
      uFrame: { value: 0 },
    };
    for (let i = 0; i < MIPS; i++) u[`tD${i}`] = { value: null };
    return common(
      new THREE.ShaderMaterial({
        defines: { SLICES: this.quality.slices, STEPS: this.quality.steps },
        uniforms: u,
        vertexShader: VERT,
        fragmentShader: MAIN,
      }),
    );
  }

  setQuality(q: AoQuality) {
    if (q.slices === this.quality.slices && q.steps === this.quality.steps && q.denoise === this.quality.denoise) return;
    this.quality = q;
    this.main.dispose();
    this.main = this.mainMaterial();
  }

  /** Sizes for a render resolution (the AO runs at half of it). */
  setSize(w: number, h: number) {
    const hw = Math.max(1, Math.ceil(w / 2));
    const hh = Math.max(1, Math.ceil(h / 2));
    this.src.set(w, h);
    if (this.size.x === hw && this.size.y === hh && this.ping) return;
    this.disposeTargets();
    this.size.set(hw, hh);
    for (let i = 0; i < MIPS; i++)
      this.mips.push(rt(Math.max(1, hw >> i), Math.max(1, hh >> i), THREE.RedFormat, THREE.FloatType));
    this.ping = rt(hw, hh, THREE.RGFormat, THREE.UnsignedByteType);
    this.pong = rt(hw, hh, THREE.RGFormat, THREE.UnsignedByteType);
  }

  /**
   * Occlusion for the scene's depth (full render resolution); `frame` varies the noise (temporal
   * anti-aliasing on) or stays 0.
   */
  render(
    renderer: THREE.WebGLRenderer,
    quad: FullScreenQuad,
    depth: THREE.DepthTexture,
    camera: THREE.PerspectiveCamera,
    frame: number,
  ) {
    const { ping, pong } = this;
    if (!ping || !pong) return;
    const draw = (m: THREE.ShaderMaterial, target: THREE.WebGLRenderTarget) => {
      quad.material = m;
      renderer.setRenderTarget(target);
      quad.render(renderer);
    };
    // 1. Depth hierarchy.
    const p0 = this.prefilter0.uniforms;
    p0.tDepth!.value = depth;
    p0.uSrcMax!.value.set(this.src.x - 1, this.src.y - 1);
    p0.uCam!.value.set(camera.near, camera.far);
    draw(this.prefilter0, this.mips[0]!);
    const pn = this.prefilterN.uniforms;
    for (let i = 1; i < MIPS; i++) {
      const src = this.mips[i - 1]!;
      pn.tSrc!.value = src.texture;
      pn.uSrcMax!.value.set(src.width - 1, src.height - 1);
      draw(this.prefilterN, this.mips[i]!);
    }
    // 2. Main pass. View-space position from uv: xy = (mul · uv + add) · z (y up, z forward).
    const e = camera.projectionMatrix.elements;
    const tx = 1 / e[0]!;
    const ty = 1 / e[5]!;
    const m = this.main.uniforms;
    for (let i = 0; i < MIPS; i++) m[`tD${i}`]!.value = this.mips[i]!.texture;
    m.uPixel!.value.set(1 / this.size.x, 1 / this.size.y);
    m.uMax!.value.set(this.size.x - 1, this.size.y - 1);
    m.uNdcMul!.value.set(2 * tx, 2 * ty);
    m.uNdcAdd!.value.set(-tx, -ty);
    m.uNdcMulPix!.value.set((2 * tx) / this.size.x, (2 * ty) / this.size.y);
    m.uFrame!.value = frame % 64;
    draw(this.main, ping);
    // 3. Denoise (the last pass is the final one).
    const d = this.denoise.uniforms;
    d.uMax!.value.set(this.size.x - 1, this.size.y - 1);
    let src = ping;
    let dst = pong;
    const passes = Math.max(1, this.quality.denoise);
    for (let i = 0; i < passes; i++) {
      const last = i === passes - 1;
      d.tSrc!.value = src.texture;
      d.uBlur!.value = last ? DENOISE_BLUR_BETA : DENOISE_BLUR_BETA / 5;
      d.uFinal!.value = last ? 1 : 0;
      draw(this.denoise, dst);
      [src, dst] = [dst, src];
    }
    this.texture = src.texture;
  }

  private disposeTargets() {
    for (const m of this.mips) m.dispose();
    this.mips = [];
    this.ping?.dispose();
    this.pong?.dispose();
    this.ping = this.pong = null;
    this.texture = null;
  }

  dispose() {
    this.disposeTargets();
    this.prefilter0.dispose();
    this.prefilterN.dispose();
    this.main.dispose();
    this.denoise.dispose();
  }
}
