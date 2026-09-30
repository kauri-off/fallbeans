import * as THREE from 'three';
import { clone } from './assets';

/**
 * The bean's face: the mouth is drawn into a texture on a thin patch that follows the visor (so
 * nothing sticks out of it or cuts into it), one texture per expression; the brows are small solid
 * strokes lying on the visor just above the modelled eyes, which squint, widen and roll along with
 * it; tears run while crying.
 */

export type Expr = 'smile' | 'grin' | 'laugh' | 'surprised' | 'scared' | 'sad' | 'cry' | 'dizzy' | 'strain' | 'determined';

/** Patch bounds on the face (model space, metres). */
const X0 = -0.25;
const X1 = 0.25;
const Y0 = 0.94;
const Y1 = 1.42;
const TEX = 512;
const GRID = 28;

/** Eye opening and pupil size per expression (wide eyes stop short of the brows). */
const EYES: Record<Expr, { open: number; pupil: number }> = {
  smile: { open: 1, pupil: 1 },
  grin: { open: 0.85, pupil: 1 },
  laugh: { open: 0.22, pupil: 1 },
  surprised: { open: 1.06, pupil: 0.8 },
  scared: { open: 1.06, pupil: 0.72 },
  sad: { open: 0.72, pupil: 1.05 },
  cry: { open: 0.35, pupil: 1 },
  dizzy: { open: 0.9, pupil: 0.85 },
  strain: { open: 0.5, pupil: 1 },
  determined: { open: 0.7, pupil: 1 },
};

/**
 * Brows per expression: lift (m) and tilt (rad; > 0 raises the inner ends: worried, < 0 lowers them:
 * cross).
 */
const BROWS: Record<Expr, { lift: number; tilt: number }> = {
  smile: { lift: 0, tilt: 0 },
  grin: { lift: 0.006, tilt: 0.05 },
  laugh: { lift: 0.01, tilt: 0.1 },
  surprised: { lift: 0.012, tilt: 0.12 },
  scared: { lift: 0.008, tilt: 0.36 },
  sad: { lift: 0, tilt: 0.34 },
  cry: { lift: 0, tilt: 0.42 },
  dizzy: { lift: 0.004, tilt: 0.16 },
  strain: { lift: -0.004, tilt: -0.32 },
  determined: { lift: -0.003, tilt: -0.26 },
};
/** Eye centre height and half height (model space), and where a brow sits: its x and the gap above the eye. */
const EYE_Y = 1.23;
const EYE_HALF = 0.1;
const BROW_X = 0.112;
const BROW_GAP = 0.023;
const BROW_MIN_Y = 1.335;
const BROW_MAX_Y = 1.37;
/** How far a brow floats off the visor (m). */
const BROW_LIFT = 0.011;

let patch: THREE.BufferGeometry | null = null;
let browGeo: THREE.BufferGeometry | null = null;
let browMat: THREE.Material | null = null;
let surface: Surface | null = null;
const mats = new Map<Expr, THREE.MeshStandardMaterial>();
let tearGeo: THREE.BufferGeometry | null = null;
let tearMat: THREE.Material | null = null;

/** A grid laid over the front of the model (visor and body), a hair above the surface. */
function buildPatch(): THREE.BufferGeometry {
  const model = clone('bean');
  model.updateMatrixWorld(true);
  const targets: THREE.Object3D[] = [];
  model.traverse((o) => {
    if (o instanceof THREE.Mesh && (o.name === 'Visor' || o.name === 'BeanBody')) targets.push(o);
  });
  const ray = new THREE.Raycaster();
  const pos: number[] = [];
  const uv: number[] = [];
  const idx: number[] = [];
  const dir = new THREE.Vector3(0, 0, -1);
  for (let j = 0; j < GRID; j++)
    for (let i = 0; i < GRID; i++) {
      const u = i / (GRID - 1);
      const v = j / (GRID - 1);
      const x = X0 + (X1 - X0) * u;
      const y = Y0 + (Y1 - Y0) * v;
      ray.set(new THREE.Vector3(x, y, 2), dir);
      const hit = ray.intersectObjects(targets, false)[0];
      // (A miss cannot happen inside the body's outline; fall back to the capsule just in case.)
      const z = hit ? hit.point.z : Math.sqrt(Math.max(0, 0.25 - x * x));
      pos.push(x, y, z + 0.004);
      uv.push(u, v);
    }
  for (let j = 0; j < GRID - 1; j++)
    for (let i = 0; i < GRID - 1; i++) {
      const a = j * GRID + i;
      idx.push(a, a + 1, a + GRID, a + 1, a + GRID + 1, a + GRID);
    }
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
  g.setAttribute('uv', new THREE.Float32BufferAttribute(uv, 2));
  g.setIndex(idx);
  g.computeVertexNormals();
  g.computeBoundingSphere();
  return g;
}

/** The front of the head where the brows go: height and normal of the visor (or body) by x and y. */
interface Surface {
  x0: number;
  y0: number;
  step: number;
  nx: number;
  ny: number;
  z: Float32Array;
  n: Float32Array;
}

function buildSurface(): Surface {
  const model = clone('bean');
  model.updateMatrixWorld(true);
  const targets: THREE.Object3D[] = [];
  model.traverse((o) => {
    if (o instanceof THREE.Mesh && (o.name === 'Visor' || o.name === 'BeanBody')) targets.push(o);
  });
  const S: Surface = { x0: -0.22, y0: 1.28, step: 0.005, nx: 89, ny: 27, z: new Float32Array(0), n: new Float32Array(0) };
  S.z = new Float32Array(S.nx * S.ny);
  S.n = new Float32Array(S.nx * S.ny * 3);
  const ray = new THREE.Raycaster();
  const dir = new THREE.Vector3(0, 0, -1);
  const nm = new THREE.Vector3();
  for (let j = 0; j < S.ny; j++)
    for (let i = 0; i < S.nx; i++) {
      const k = j * S.nx + i;
      const x = S.x0 + i * S.step;
      const y = S.y0 + j * S.step;
      ray.set(new THREE.Vector3(x, y, 2), dir);
      const hit = ray.intersectObjects(targets, false)[0];
      S.z[k] = hit ? hit.point.z : Math.sqrt(Math.max(0, 0.25 - x * x));
      if (hit?.face) nm.copy(hit.face.normal).transformDirection(hit.object.matrixWorld);
      else nm.set(0, 0, 1);
      S.n.set([nm.x, nm.y, nm.z], k * 3);
    }
  return S;
}

/** Point on the head surface at (x, y) and its normal (bilinear). */
function onSurface(S: Surface, x: number, y: number, p: THREE.Vector3, n: THREE.Vector3) {
  const fx = THREE.MathUtils.clamp((x - S.x0) / S.step, 0, S.nx - 1.001);
  const fy = THREE.MathUtils.clamp((y - S.y0) / S.step, 0, S.ny - 1.001);
  const i = Math.floor(fx);
  const j = Math.floor(fy);
  const u = fx - i;
  const v = fy - j;
  const w = [(1 - u) * (1 - v), u * (1 - v), (1 - u) * v, u * v];
  const ks = [j * S.nx + i, j * S.nx + i + 1, (j + 1) * S.nx + i, (j + 1) * S.nx + i + 1];
  let z = 0;
  n.set(0, 0, 0);
  for (let q = 0; q < 4; q++) {
    const k = ks[q]!;
    z += S.z[k]! * w[q]!;
    n.x += S.n[k * 3]! * w[q]!;
    n.y += S.n[k * 3 + 1]! * w[q]!;
    n.z += S.n[k * 3 + 2]! * w[q]!;
  }
  p.set(x, y, z);
  n.normalize();
}

/** A brow: a flattened, arched stroke along x, bent to follow the curve of the visor. */
function buildBrow(): THREE.BufferGeometry {
  const g = new THREE.CapsuleGeometry(0.0145, 0.076, 4, 12);
  g.rotateZ(Math.PI / 2);
  const pos = g.attributes.position!;
  for (let i = 0; i < pos.count; i++) {
    const x = pos.getX(i);
    const e = Math.min(1, Math.abs(x) / 0.05);
    // Thinner towards the ends, arched, flat against the face and curved round it.
    pos.setY(i, pos.getY(i) * (1 - 0.35 * e * e) - x * x * 2.2);
    pos.setZ(i, pos.getZ(i) * 0.5 - x * x * 1.65);
  }
  g.computeVertexNormals();
  return g;
}

/** Canvas position of a point on the face (model metres). */
const px = (x: number) => ((x - X0) / (X1 - X0)) * TEX;
const py = (y: number) => ((Y1 - y) / (Y1 - Y0)) * TEX;
const m = (d: number) => (d / (X1 - X0)) * TEX;

const INK = '#3a0f22';
const TONGUE = '#ff6f8f';

function drawExpr(e: Expr): THREE.CanvasTexture {
  const c = document.createElement('canvas');
  c.width = c.height = TEX;
  const g = c.getContext('2d')!;
  g.lineCap = 'round';
  g.lineJoin = 'round';
  const cx = px(0);
  const my = py(1.095);

  /** An open, D-shaped mouth (w, h in metres) with teeth and a tongue; `down` flips it (a sad mouth). */
  const openMouth = (w: number, h: number, down = false, top = 0) => {
    const hw = m(w) / 2;
    const hh = m(h);
    const y0 = my - (down ? -hh * 0.35 : hh * 0.35) - top;
    const shape = () => {
      g.beginPath();
      if (!down) {
        g.moveTo(cx - hw, y0);
        g.quadraticCurveTo(cx, y0 + hh * 0.18, cx + hw, y0);
        g.bezierCurveTo(cx + hw * 0.9, y0 + hh * 1.25, cx - hw * 0.9, y0 + hh * 1.25, cx - hw, y0);
      } else {
        g.moveTo(cx - hw, y0);
        g.quadraticCurveTo(cx, y0 - hh * 0.18, cx + hw, y0);
        g.bezierCurveTo(cx + hw * 0.9, y0 - hh * 1.25, cx - hw * 0.9, y0 - hh * 1.25, cx - hw, y0);
      }
      g.closePath();
    };
    g.save();
    shape();
    g.fillStyle = INK;
    g.fill();
    g.clip();
    // Tongue at the bottom, a strip of teeth at the top.
    g.fillStyle = TONGUE;
    g.beginPath();
    g.ellipse(cx, down ? y0 - hh * 0.05 : y0 + hh * 0.95, hw * 0.55, hh * 0.45, 0, 0, Math.PI * 2);
    g.fill();
    if (!down) {
      g.fillStyle = '#ffffff';
      g.fillRect(cx - hw, y0 - hh * 0.2, hw * 2, hh * 0.3);
    }
    g.restore();
    shape();
    g.strokeStyle = INK;
    g.lineWidth = m(0.008);
    g.stroke();
  };
  const arc = (w: number, bend: number, width = 0.013, y = 1.095) => {
    g.strokeStyle = INK;
    g.lineWidth = m(width);
    g.beginPath();
    g.moveTo(px(-w / 2), py(y));
    g.quadraticCurveTo(cx, py(y - bend), px(w / 2), py(y));
    g.stroke();
  };
  const tears = () => {
    for (const s of [-1, 1]) {
      const x = s * 0.115;
      const grad = g.createLinearGradient(0, py(1.16), 0, py(0.96));
      grad.addColorStop(0, 'rgba(140, 210, 255, 0.95)');
      grad.addColorStop(1, 'rgba(140, 210, 255, 0.15)');
      g.fillStyle = grad;
      g.beginPath();
      g.moveTo(px(x - 0.018), py(1.16));
      g.bezierCurveTo(px(x - 0.03), py(1.06), px(x + 0.005), py(1.02), px(x - 0.01), py(0.96));
      g.lineTo(px(x + 0.022), py(0.96));
      g.bezierCurveTo(px(x + 0.03), py(1.03), px(x + 0.02), py(1.08), px(x + 0.018), py(1.16));
      g.closePath();
      g.fill();
    }
  };

  switch (e) {
    case 'smile':
      openMouth(0.135, 0.062);
      break;
    case 'grin':
      openMouth(0.15, 0.075);
      break;
    case 'laugh':
      openMouth(0.16, 0.1, false, m(0.01));
      break;
    case 'surprised':
      g.fillStyle = INK;
      g.beginPath();
      g.ellipse(cx, py(1.085), m(0.03), m(0.036), 0, 0, Math.PI * 2);
      g.fill();
      break;
    case 'scared':
      g.fillStyle = INK;
      g.beginPath();
      g.ellipse(cx, py(1.08), m(0.04), m(0.052), 0, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = '#ffffff';
      g.fillRect(cx - m(0.03), py(1.12), m(0.06), m(0.012));
      break;
    case 'sad':
      arc(0.1, -0.03);
      break;
    case 'cry':
      tears();
      openMouth(0.12, 0.06, true);
      break;
    case 'dizzy': {
      g.strokeStyle = INK;
      g.lineWidth = m(0.012);
      g.beginPath();
      for (let k = 0; k <= 24; k++) {
        const f = k / 24;
        const x = -0.055 + f * 0.11;
        const y = 1.09 + Math.sin(f * Math.PI * 4) * 0.012;
        if (k) g.lineTo(px(x), py(y));
        else g.moveTo(px(x), py(y));
      }
      g.stroke();
      break;
    }
    case 'strain': {
      // Gritted teeth.
      const w = m(0.11);
      const h = m(0.04);
      g.fillStyle = '#ffffff';
      g.strokeStyle = INK;
      g.lineWidth = m(0.009);
      g.beginPath();
      g.roundRect(cx - w / 2, my - h / 2, w, h, h * 0.45);
      g.fill();
      g.stroke();
      g.lineWidth = m(0.005);
      g.beginPath();
      g.moveTo(cx - w / 2, my);
      g.lineTo(cx + w / 2, my);
      for (let k = 1; k < 5; k++) {
        g.moveTo(cx - w / 2 + (w * k) / 5, my - h / 2);
        g.lineTo(cx - w / 2 + (w * k) / 5, my + h / 2);
      }
      g.stroke();
      break;
    }
    case 'determined':
      arc(0.07, -0.008, 0.012);
      break;
  }
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = 4;
  return tex;
}

function material(e: Expr): THREE.MeshStandardMaterial {
  let mat = mats.get(e);
  if (!mat) {
    mat = new THREE.MeshStandardMaterial({
      map: drawExpr(e),
      transparent: true,
      depthWrite: false,
      polygonOffset: true,
      polygonOffsetFactor: -2,
      polygonOffsetUnits: -2,
      roughness: 0.5,
    });
    mats.set(e, mat);
  }
  return mat;
}

const _bp = new THREE.Vector3();
const _bn = new THREE.Vector3();
const _bt = new THREE.Vector3();
const _bb = new THREE.Vector3();
const _bm = new THREE.Matrix4();

export class Face {
  private readonly mesh: THREE.Mesh;
  private readonly brows: THREE.Mesh[] = [];
  private browLift = 0;
  private browTilt = 0;
  /** Eye opening without blinks (the brows follow it, not every blink). */
  private wide = 1;
  private readonly eyes: THREE.Object3D[] = [];
  private readonly pupils: THREE.Object3D[] = [];
  private readonly pupilBase: THREE.Vector3[] = [];
  private readonly tears: THREE.Mesh[] = [];
  private open = 1;
  private pupil = 1;
  expr: Expr = 'smile';

  constructor(model: THREE.Object3D) {
    patch ??= buildPatch();
    this.mesh = new THREE.Mesh(patch, material('smile'));
    this.mesh.renderOrder = 2;
    this.mesh.userData.noLod = true;
    this.mesh.name = 'Face';
    model.add(this.mesh);
    for (const n of ['EyeL', 'EyeR']) {
      const e = model.getObjectByName(n);
      if (!e) continue;
      this.eyes.push(e);
      const p = e.getObjectByName(n === 'EyeL' ? 'PupilL' : 'PupilR');
      if (p) {
        this.pupils.push(p);
        this.pupilBase.push(p.position.clone());
      }
    }
    surface ??= buildSurface();
    browGeo ??= buildBrow();
    browMat ??= new THREE.MeshStandardMaterial({ color: INK, roughness: 0.6 });
    for (let k = 0; k < 2; k++) {
      const b = new THREE.Mesh(browGeo, browMat);
      b.name = k ? 'BrowR' : 'BrowL';
      b.userData.noLod = true;
      b.matrixAutoUpdate = false;
      model.add(b);
      this.brows.push(b);
    }
    tearGeo ??= new THREE.SphereGeometry(0.022, 10, 8).scale(1, 1.4, 0.7);
    tearMat ??= new THREE.MeshStandardMaterial({ color: '#9fdcff', roughness: 0.05, transparent: true, opacity: 0.85 });
    for (let k = 0; k < 4; k++) {
      const t = new THREE.Mesh(tearGeo, tearMat);
      t.visible = false;
      t.userData.noLod = true;
      model.add(t);
      this.tears.push(t);
    }
  }

  /** Each brow on the visor above its eye, turned by the tilt within the surface. */
  private placeBrows() {
    const y = THREE.MathUtils.clamp(EYE_Y + EYE_HALF * this.wide + BROW_GAP + this.browLift, BROW_MIN_Y, BROW_MAX_Y);
    this.brows.forEach((b, i) => {
      const side = i ? 1 : -1;
      onSurface(surface!, side * BROW_X, y, _bp, _bn);
      // Along the surface, level; then raised at the inner end (towards the middle) by the tilt.
      _bt.set(1, 0, 0).addScaledVector(_bn, -_bn.x).normalize();
      _bb.crossVectors(_bn, _bt);
      const a = -side * this.browTilt;
      _bt.multiplyScalar(Math.cos(a)).addScaledVector(_bb, Math.sin(a));
      _bb.crossVectors(_bn, _bt);
      _bm.makeBasis(_bt, _bb, _bn).setPosition(_bp.addScaledVector(_bn, BROW_LIFT));
      b.matrix.copy(_bm);
      b.matrixWorldNeedsUpdate = true;
    });
  }

  set(e: Expr) {
    if (e === this.expr) return;
    this.expr = e;
    this.mesh.material = material(e);
  }

  /** Eyes and tears for this frame; `blink` (0 closed … 1 open) and `squeeze` (tumbling) multiply the opening. */
  update(dt: number, t: number, blink: number, squeeze = 1) {
    const want = EYES[this.expr];
    const k = Math.min(1, dt * 18);
    this.open += (want.open * blink * squeeze - this.open) * k;
    this.pupil += (want.pupil - this.pupil) * k;
    for (const e of this.eyes) e.scale.y = this.open;
    this.wide += (want.open * squeeze - this.wide) * k;
    const bw = BROWS[this.expr];
    this.browLift += (bw.lift - this.browLift) * k;
    this.browTilt += (bw.tilt - this.browTilt) * k;
    this.placeBrows();
    this.pupils.forEach((p, i) => {
      p.scale.setScalar(this.pupil);
      const base = this.pupilBase[i]!;
      if (this.expr === 'dizzy') {
        // Eyes rolling in circles (in opposite directions).
        const a = t * 9 * (i ? -1 : 1);
        p.position.set(base.x + Math.cos(a) * 0.018, base.y + Math.sin(a) * 0.022, base.z);
      } else p.position.copy(base);
    });
    const crying = this.expr === 'cry';
    this.tears.forEach((d, i) => {
      d.visible = crying;
      if (!crying) return;
      // Drops run down from under each eye and drip off.
      const side = i % 2 ? 1 : -1;
      const f = (t * 1.3 + i * 0.37) % 1;
      d.position.set(side * (0.12 + f * 0.02), 1.15 - f * 0.25, 0.52 - f * 0.05);
      d.scale.setScalar(0.6 + Math.sin(f * Math.PI) * 0.5);
    });
  }
}
