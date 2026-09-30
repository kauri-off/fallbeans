import * as THREE from 'three';
import { clone } from './assets';

/**
 * The bean's face: mouth and brows are drawn into a texture on a thin patch that follows the visor
 * (so nothing sticks out of it or cuts into it), one texture per expression; the modelled eyes squint,
 * widen and roll along with it, and tears run while crying.
 */

export type Expr = 'smile' | 'grin' | 'laugh' | 'surprised' | 'scared' | 'sad' | 'cry' | 'dizzy' | 'strain' | 'determined';

/** Patch bounds on the face (model space, metres). */
const X0 = -0.25;
const X1 = 0.25;
const Y0 = 0.94;
const Y1 = 1.42;
const TEX = 512;
const GRID = 28;

/** Eye opening and pupil size per expression. */
const EYES: Record<Expr, { open: number; pupil: number }> = {
  smile: { open: 1, pupil: 1 },
  grin: { open: 0.85, pupil: 1 },
  laugh: { open: 0.22, pupil: 1 },
  surprised: { open: 1.12, pupil: 0.8 },
  scared: { open: 1.18, pupil: 0.6 },
  sad: { open: 0.72, pupil: 1.05 },
  cry: { open: 0.35, pupil: 1 },
  dizzy: { open: 0.9, pupil: 0.85 },
  strain: { open: 0.5, pupil: 1 },
  determined: { open: 0.7, pupil: 1 },
};

let patch: THREE.BufferGeometry | null = null;
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
  /** Brows: `tilt` > 0 raises the inner ends (worried), < 0 lowers them (cross). */
  const brows = (tilt: number, lift = 0) => {
    g.strokeStyle = INK;
    g.lineWidth = m(0.016);
    for (const s of [-1, 1]) {
      const inner = s * 0.055;
      const outer = s * 0.17;
      const y = 1.355 + lift;
      g.beginPath();
      g.moveTo(px(outer), py(y - tilt * 0.35));
      g.quadraticCurveTo(px((inner + outer) / 2), py(y + 0.018), px(inner), py(y + tilt));
      g.stroke();
    }
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
      brows(0, 0.012);
      break;
    case 'laugh':
      openMouth(0.16, 0.1, false, m(0.01));
      brows(0.004, 0.02);
      break;
    case 'surprised':
      g.fillStyle = INK;
      g.beginPath();
      g.ellipse(cx, py(1.085), m(0.03), m(0.036), 0, 0, Math.PI * 2);
      g.fill();
      brows(0.01, 0.025);
      break;
    case 'scared':
      g.fillStyle = INK;
      g.beginPath();
      g.ellipse(cx, py(1.08), m(0.04), m(0.052), 0, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = '#ffffff';
      g.fillRect(cx - m(0.03), py(1.12), m(0.06), m(0.012));
      brows(0.03, 0.02);
      break;
    case 'sad':
      arc(0.1, -0.03);
      brows(0.028);
      break;
    case 'cry':
      tears();
      openMouth(0.12, 0.06, true);
      brows(0.034);
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
      brows(0.015, 0.01);
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
      brows(-0.022);
      break;
    }
    case 'determined':
      arc(0.07, -0.008, 0.012);
      brows(-0.018);
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

export class Face {
  private readonly mesh: THREE.Mesh;
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
