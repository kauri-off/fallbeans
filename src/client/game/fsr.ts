import * as THREE from 'three';
import type { FullScreenQuad } from 'three/addons/postprocessing/Pass.js';

/**
 * AMD FidelityFX Super Resolution 1 (github.com/GPUOpen-Effects/FidelityFX-FSR), on WebGL2: the
 * scene is drawn (and anti-aliased) at a lower resolution, then EASU (edge-adaptive spatial
 * upsampling, 12 taps, a Lanczos-like kernel stretched along local edges, deringed) brings it to the
 * display resolution and RCAS (robust contrast-adaptive sharpening) restores the detail.
 * Both passes read display-referred colour (after tone mapping, as FSR expects).
 */

/** Render scale per axis for each FSR mode (AMD's presets). */
export const FSR_SCALE = {
  off: 1,
  ultra: 1 / 1.3,
  quality: 1 / 1.5,
  balanced: 1 / 1.7,
  performance: 1 / 2,
} as const;

export type Upscale = keyof typeof FSR_SCALE;

/** RCAS sharpness in stops (0 = sharpest); AMD's default is 0.2. */
const SHARPNESS = 0.2;

const VERT = 'void main(){ gl_Position = vec4(position.xy, 0.0, 1.0); }';

const EASU = /* glsl */ `
uniform sampler2D tInput;
uniform vec2 uInSize;
uniform vec2 uOutSize;
uniform ivec2 uMax;

vec3 tap(ivec2 p) { return texelFetch(tInput, clamp(p, ivec2(0), uMax), 0).rgb; }
// Luma times 2.
float luma(vec3 c) { return c.g + 0.5 * (c.r + c.b); }

void setF(inout vec2 dir, inout float len, float w, float lA, float lB, float lC, float lD, float lE) {
  float lenX = max(abs(lD - lC), abs(lC - lB));
  float dirX = lD - lB;
  dir.x += dirX * w;
  lenX = clamp(abs(dirX) / max(lenX, 1e-5), 0.0, 1.0);
  len += lenX * lenX * w;
  float lenY = max(abs(lE - lC), abs(lC - lA));
  float dirY = lE - lA;
  dir.y += dirY * w;
  lenY = clamp(abs(dirY) / max(lenY, 1e-5), 0.0, 1.0);
  len += lenY * lenY * w;
}

void tapF(inout vec3 aC, inout float aW, vec2 off, vec2 dir, vec2 len, float lob, float clp, vec3 c) {
  vec2 v = vec2(off.x * dir.x + off.y * dir.y, off.x * -dir.y + off.y * dir.x) * len;
  float d2 = min(dot(v, v), clp);
  float wB = 0.4 * d2 - 1.0;
  float wA = lob * d2 - 1.0;
  wB *= wB;
  wA *= wA;
  wB = 1.5625 * wB - 0.5625;
  float w = wB * wA;
  aW += w;
  aC += c * w;
}

void main() {
  // Position in the input (texel centres at integers), split into the texel and the fraction.
  vec2 pp = gl_FragCoord.xy * (uInSize / uOutSize) - 0.5;
  vec2 fp = floor(pp);
  pp -= fp;
  ivec2 ip = ivec2(fp);
  //    b c
  //  e f g h
  //  i j k l
  //    n o
  vec3 bC = tap(ip + ivec2(0, -1));
  vec3 cC = tap(ip + ivec2(1, -1));
  vec3 eC = tap(ip + ivec2(-1, 0));
  vec3 fC = tap(ip);
  vec3 gC = tap(ip + ivec2(1, 0));
  vec3 hC = tap(ip + ivec2(2, 0));
  vec3 iC = tap(ip + ivec2(-1, 1));
  vec3 jC = tap(ip + ivec2(0, 1));
  vec3 kC = tap(ip + ivec2(1, 1));
  vec3 lC = tap(ip + ivec2(2, 1));
  vec3 nC = tap(ip + ivec2(0, 2));
  vec3 oC = tap(ip + ivec2(1, 2));
  float bL = luma(bC), cL = luma(cC), eL = luma(eC), fL = luma(fC), gL = luma(gC), hL = luma(hC);
  float iL = luma(iC), jL = luma(jC), kL = luma(kC), lL = luma(lC), nL = luma(nC), oL = luma(oC);

  // Direction and length of the local edge, bilinearly accumulated over the 4 nearest texels.
  vec2 dir = vec2(0.0);
  float len = 0.0;
  setF(dir, len, (1.0 - pp.x) * (1.0 - pp.y), bL, eL, fL, gL, jL);
  setF(dir, len, pp.x * (1.0 - pp.y), cL, fL, gL, hL, kL);
  setF(dir, len, (1.0 - pp.x) * pp.y, fL, iL, jL, kL, nL);
  setF(dir, len, pp.x * pp.y, gL, jL, kL, lL, oL);
  vec2 dir2 = dir * dir;
  float dirR = dir2.x + dir2.y;
  bool zro = dirR < 1.0 / 32768.0;
  dirR = zro ? 1.0 : inversesqrt(dirR);
  dir.x = zro ? 1.0 : dir.x;
  dir *= dirR;
  len = len * 0.5;
  len *= len;
  float stretch = dot(dir, dir) / max(abs(dir.x), abs(dir.y));
  vec2 len2 = vec2(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
  float lob = 0.5 - 0.29 * len;
  float clp = 1.0 / lob;

  vec3 aC = vec3(0.0);
  float aW = 0.0;
  tapF(aC, aW, vec2(0.0, -1.0) - pp, dir, len2, lob, clp, bC);
  tapF(aC, aW, vec2(1.0, -1.0) - pp, dir, len2, lob, clp, cC);
  tapF(aC, aW, vec2(-1.0, 1.0) - pp, dir, len2, lob, clp, iC);
  tapF(aC, aW, vec2(0.0, 1.0) - pp, dir, len2, lob, clp, jC);
  tapF(aC, aW, vec2(0.0, 0.0) - pp, dir, len2, lob, clp, fC);
  tapF(aC, aW, vec2(-1.0, 0.0) - pp, dir, len2, lob, clp, eC);
  tapF(aC, aW, vec2(1.0, 1.0) - pp, dir, len2, lob, clp, kC);
  tapF(aC, aW, vec2(2.0, 1.0) - pp, dir, len2, lob, clp, lC);
  tapF(aC, aW, vec2(2.0, 0.0) - pp, dir, len2, lob, clp, hC);
  tapF(aC, aW, vec2(1.0, 0.0) - pp, dir, len2, lob, clp, gC);
  tapF(aC, aW, vec2(1.0, 2.0) - pp, dir, len2, lob, clp, oC);
  tapF(aC, aW, vec2(0.0, 2.0) - pp, dir, len2, lob, clp, nC);
  // Dering: within the 4 nearest texels.
  vec3 mn = min(min(fC, gC), min(jC, kC));
  vec3 mx = max(max(fC, gC), max(jC, kC));
  gl_FragColor = vec4(min(mx, max(mn, aC / aW)), 1.0);
}`;

const RCAS = /* glsl */ `
uniform sampler2D tInput;
uniform ivec2 uMax;
uniform float uSharp;
#define RCAS_LIMIT (0.25 - (1.0 / 16.0))
vec3 tap(ivec2 p) { return texelFetch(tInput, clamp(p, ivec2(0), uMax), 0).rgb; }
void main() {
  //    b
  //  d e f
  //    h
  ivec2 sp = ivec2(gl_FragCoord.xy);
  vec3 b = tap(sp + ivec2(0, -1));
  vec3 d = tap(sp + ivec2(-1, 0));
  vec3 e = tap(sp);
  vec3 f = tap(sp + ivec2(1, 0));
  vec3 h = tap(sp + ivec2(0, 1));
  float bL = b.b * 0.5 + (b.r * 0.5 + b.g);
  float dL = d.b * 0.5 + (d.r * 0.5 + d.g);
  float eL = e.b * 0.5 + (e.r * 0.5 + e.g);
  float fL = f.b * 0.5 + (f.r * 0.5 + f.g);
  float hL = h.b * 0.5 + (h.r * 0.5 + h.g);
  // Noise detection: less sharpening on noise.
  float nz = 0.25 * (bL + dL + fL + hL) - eL;
  float range = max(max(max(bL, dL), max(eL, fL)), hL) - min(min(min(bL, dL), min(eL, fL)), hL);
  nz = clamp(abs(nz) / max(range, 1e-5), 0.0, 1.0);
  nz = -0.5 * nz + 1.0;
  vec3 mn4 = min(min(b, d), min(f, h));
  vec3 mx4 = max(max(b, d), max(f, h));
  // Limiters: the most sharpening that keeps the result within the ring's range.
  vec3 hitMin = min(mn4, e) / max(4.0 * mx4, vec3(1e-5));
  vec3 hitMax = (1.0 - max(mx4, e)) / min(4.0 * mn4 - 4.0, vec3(-1e-5));
  vec3 lobeRGB = max(-hitMin, hitMax);
  float lobe = max(-RCAS_LIMIT, min(max(lobeRGB.r, max(lobeRGB.g, lobeRGB.b)), 0.0)) * uSharp;
  lobe *= nz;
  vec3 c = (lobe * (b + d + f + h) + e) / (4.0 * lobe + 1.0);
  gl_FragColor = vec4(clamp(c, 0.0, 1.0), 1.0);
}`;

const mat = (fragmentShader: string, uniforms: Record<string, THREE.IUniform>) =>
  new THREE.ShaderMaterial({ uniforms, vertexShader: VERT, fragmentShader, depthTest: false, depthWrite: false });

export class Fsr {
  private target: THREE.WebGLRenderTarget | null = null;
  private readonly easu = mat(EASU, {
    tInput: { value: null },
    uInSize: { value: new THREE.Vector2() },
    uOutSize: { value: new THREE.Vector2() },
    uMax: { value: new THREE.Vector2() },
  });
  private readonly rcas = mat(RCAS, {
    tInput: { value: null },
    uMax: { value: new THREE.Vector2() },
    uSharp: { value: 2 ** -SHARPNESS },
  });

  /** Output (display) size. */
  setSize(w: number, h: number) {
    if (this.target && this.target.width === w && this.target.height === h) return;
    this.target?.dispose();
    this.target = new THREE.WebGLRenderTarget(w, h, {
      minFilter: THREE.NearestFilter,
      magFilter: THREE.NearestFilter,
      generateMipmaps: false,
      depthBuffer: false,
    });
  }

  /** Upscales `input` (its own size) to the output size, sharpened, into `output` (null: the screen). */
  render(renderer: THREE.WebGLRenderer, quad: FullScreenQuad, input: THREE.Texture, output: THREE.WebGLRenderTarget | null) {
    const t = this.target;
    if (!t) return;
    const img = input.image as { width: number; height: number };
    const e = this.easu.uniforms;
    e.tInput!.value = input;
    e.uInSize!.value.set(img.width, img.height);
    e.uOutSize!.value.set(t.width, t.height);
    e.uMax!.value.set(img.width - 1, img.height - 1);
    quad.material = this.easu;
    renderer.setRenderTarget(t);
    quad.render(renderer);
    const r = this.rcas.uniforms;
    r.tInput!.value = t.texture;
    r.uMax!.value.set(t.width - 1, t.height - 1);
    quad.material = this.rcas;
    renderer.setRenderTarget(output);
    quad.render(renderer);
  }

  dispose() {
    this.target?.dispose();
    this.target = null;
    this.easu.dispose();
    this.rcas.dispose();
  }
}
