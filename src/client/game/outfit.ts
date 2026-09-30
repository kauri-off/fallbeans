import * as THREE from 'three';
import type { Glasses, Hat } from '../../shared/outfit';
import { applySurface } from './materials';

// Model space: head sphere r 0.5 around (0, 1.1, 0), face along +z, eyes at x ±0.115, y 1.23.

/** Tipped back a little so that the front of a hat clears the visor and brows. */
const HAT_Y = 1.42;
const HAT_TILT = -0.1;

const mats = new Map<string, THREE.MeshStandardMaterial>();
function mat(
  color: string,
  o: { rough?: number; metal?: number; glow?: number; both?: boolean } = {},
): THREE.MeshStandardMaterial {
  const key = `${color}|${o.rough ?? 0.6}|${o.metal ?? 0}|${o.glow ?? 0}|${!!o.both}`;
  let m = mats.get(key);
  if (!m) {
    m = new THREE.MeshStandardMaterial({ color, roughness: o.rough ?? 0.6, metalness: o.metal ?? 0 });
    if (o.glow) m.emissive.set(color).multiplyScalar(o.glow);
    if (o.both) m.side = THREE.DoubleSide;
    applySurface(m, null);
    mats.set(key, m);
  }
  return m;
}

/** A second colour that stands out against `c` (bands, trims). */
function contrast(c: string) {
  const l = new THREE.Color(c).getHSL({ h: 0, s: 0, l: 0 }).l;
  return l < 0.35 ? '#ff3b3b' : '#2b2b33';
}

function part(g: THREE.BufferGeometry, m: THREE.Material, x = 0, y = 0, z = 0, parent?: THREE.Object3D): THREE.Mesh {
  const o = new THREE.Mesh(g, m);
  o.position.set(x, y, z);
  o.castShadow = true;
  parent?.add(o);
  return o;
}

const dome = (r: number, h: number) => {
  const g = new THREE.SphereGeometry(r, 28, 12, 0, Math.PI * 2, 0, Math.PI / 2);
  g.scale(1, h / r, 1);
  return g;
};
const ball = (r: number) => new THREE.SphereGeometry(r, 16, 12);

export interface Accessory {
  root: THREE.Object3D;
  /** How far above its usual place a winner's crown goes (on top of the hat). */
  crownLift: number;
  update?(t: number, speed: number): void;
}

/** A hat: `tint` its colour ('' its own default), `suit` the bean's suit material (for ears the colour of the bean). */
export function makeHat(kind: Hat, tint: string, suit: THREE.Material): Accessory | null {
  if (kind === 'none') return null;
  const g = new THREE.Group();
  g.position.y = HAT_Y;
  g.rotation.x = HAT_TILT;
  const main = (def: string) => mat(tint || def);
  const acc: Accessory = { root: g, crownLift: 0 };
  switch (kind) {
    case 'cap': {
      const m = main('#3fa9ff');
      part(dome(0.41, 0.225), m, 0, 0, 0, g);
      const brim = part(new THREE.CylinderGeometry(0.3, 0.3, 0.025, 28, 1, false, -Math.PI / 2, Math.PI), m, 0, 0.015, 0.3, g);
      brim.scale.z = 1.1;
      brim.rotation.x = -0.12;
      part(ball(0.035), mat(contrast(tint || '#3fa9ff')), 0, 0.22, 0, g);
      acc.crownLift = 0.16;
      break;
    }
    case 'beanie': {
      const m = main('#ff5fa2');
      part(dome(0.42, 0.3), m, 0, 0, 0, g);
      const cuff = part(new THREE.TorusGeometry(0.41, 0.06, 10, 36), m, 0, 0.03, 0, g);
      cuff.rotation.x = Math.PI / 2;
      part(ball(0.1), mat('#ffffff', { rough: 0.9 }), 0, 0.36, 0, g);
      acc.crownLift = 0.3;
      break;
    }
    case 'party': {
      const c = tint || '#ffd23f';
      part(new THREE.ConeGeometry(0.19, 0.5, 24), mat(c), 0, 0.33, 0, g);
      const band = part(new THREE.TorusGeometry(0.15, 0.03, 8, 24), mat(contrast(c)), 0, 0.22, 0, g);
      band.rotation.x = Math.PI / 2;
      part(ball(0.06), mat('#ffffff', { rough: 0.9 }), 0, 0.6, 0, g);
      g.rotation.z = 0.25;
      acc.crownLift = 0.05;
      break;
    }
    case 'tophat': {
      const c = tint || '#2b2b33';
      const m = mat(c, { rough: 0.45 });
      part(new THREE.CylinderGeometry(0.42, 0.42, 0.03, 36), m, 0, 0.03, 0, g);
      part(new THREE.CylinderGeometry(0.27, 0.29, 0.45, 32), m, 0, 0.255, 0, g);
      part(new THREE.CylinderGeometry(0.295, 0.295, 0.08, 32), mat(contrast(c)), 0, 0.1, 0, g);
      acc.crownLift = 0.42;
      break;
    }
    case 'cowboy': {
      const c = tint || '#8b5a2b';
      const m = mat(c, { rough: 0.8, both: true });
      const brim = part(
        new THREE.LatheGeometry(
          [
            [0.28, 0],
            [0.48, 0.01],
            [0.58, 0.05],
            [0.63, 0.11],
          ].map(([x, y]) => new THREE.Vector2(x, y)),
          40,
        ),
        m,
        0,
        0.02,
        0,
        g,
      );
      brim.scale.z = 0.85;
      part(
        new THREE.LatheGeometry(
          [
            [0.31, 0],
            [0.31, 0.2],
            [0.27, 0.3],
            [0.12, 0.33],
            [0, 0.29],
          ].map(([x, y]) => new THREE.Vector2(x, y)),
          32,
        ),
        m,
        0,
        0.02,
        0,
        g,
      ).scale.z = 0.9;
      part(new THREE.CylinderGeometry(0.315, 0.315, 0.06, 32, 1, true), mat(contrast(c)), 0, 0.08, 0, g).scale.z = 0.9;
      acc.crownLift = 0.28;
      break;
    }
    case 'viking': {
      const m = mat(tint || '#9ea3b0', { rough: 0.35, metal: 0.6 });
      part(dome(0.43, 0.3), m, 0, 0, 0, g);
      const rim = part(new THREE.TorusGeometry(0.42, 0.04, 8, 36), mat('#ffd23f', { rough: 0.35, metal: 0.7 }), 0, 0.01, 0, g);
      rim.rotation.x = Math.PI / 2;
      const horn = mat('#fff4d6', { rough: 0.5 });
      for (const s of [-1, 1]) {
        const h = part(new THREE.ConeGeometry(0.08, 0.34, 16), horn, s * 0.44, 0.2, 0, g);
        h.rotation.z = -s * 0.8;
      }
      acc.crownLift = 0.26;
      break;
    }
    case 'propeller': {
      const m = main('#4fdc6a');
      part(dome(0.41, 0.24), m, 0, 0, 0, g);
      part(new THREE.CylinderGeometry(0.015, 0.015, 0.12, 8), mat('#9ea3b0', { metal: 0.6, rough: 0.4 }), 0, 0.28, 0, g);
      const rotor = new THREE.Group();
      rotor.position.y = 0.34;
      g.add(rotor);
      for (const [i, c] of ['#ff3b3b', '#3fa9ff'].entries()) {
        const b = part(new THREE.BoxGeometry(0.24, 0.012, 0.07), mat(c), (i ? 1 : -1) * 0.13, 0, 0, rotor);
        b.rotation.x = (i ? 1 : -1) * 0.25;
      }
      part(ball(0.03), mat('#ffd23f'), 0, 0, 0, rotor);
      let a = 0;
      let last = 0;
      acc.update = (t, speed) => {
        a += Math.min(0.1, Math.max(0, t - last)) * (5 + speed * 3);
        last = t;
        rotor.rotation.y = a;
      };
      acc.crownLift = 0.36;
      break;
    }
    case 'bunny': {
      const m = tint ? mat(tint) : mat('#ffffff', { rough: 0.85 });
      const inner = mat('#ffb3cf', { rough: 0.85 });
      const ears: THREE.Object3D[] = [];
      for (const s of [-1, 1]) {
        const e = new THREE.Group();
        e.position.set(s * 0.13, 0.08, 0);
        e.rotation.z = -s * 0.18;
        g.add(e);
        part(ball(0.1), m, 0, 0.26, 0, e).scale.set(0.9, 3, 0.45);
        part(ball(0.1), inner, 0, 0.26, 0.03, e).scale.set(0.5, 2.3, 0.2);
        ears.push(e);
      }
      acc.update = (t, speed) => {
        const flop = Math.min(0.35, speed * 0.04);
        for (const [i, e] of ears.entries()) e.rotation.x = -flop + Math.sin(t * 3 + i) * 0.05;
      };
      break;
    }
    case 'cat': {
      const m = tint ? mat(tint) : suit;
      const inner = mat('#ffb3cf', { rough: 0.85 });
      for (const s of [-1, 1]) {
        const e = new THREE.Group();
        e.position.set(s * 0.22, 0.08, 0);
        e.rotation.z = -s * 0.45;
        g.add(e);
        part(new THREE.ConeGeometry(0.13, 0.24, 20), m, 0, 0.1, 0, e).scale.z = 0.5;
        part(new THREE.ConeGeometry(0.08, 0.16, 16), inner, 0, 0.08, 0.035, e).scale.z = 0.3;
      }
      break;
    }
    case 'horns': {
      const m = mat(tint || '#ff3b3b', { rough: 0.4 });
      for (const s of [-1, 1]) {
        const h = part(new THREE.ConeGeometry(0.065, 0.2, 16), m, s * 0.19, 0.12, 0.06, g);
        h.rotation.z = -s * 0.4;
        h.rotation.x = 0.2;
      }
      break;
    }
    case 'halo': {
      const ring = part(
        new THREE.TorusGeometry(0.24, 0.028, 10, 40),
        mat(tint || '#ffd23f', { rough: 0.3, glow: 0.8 }),
        0,
        0.33,
        0,
        g,
      );
      ring.rotation.x = Math.PI / 2;
      ring.castShadow = false;
      acc.update = (t) => {
        ring.position.y = 0.45 + Math.sin(t * 2.2) * 0.025;
      };
      break;
    }
    case 'flower': {
      const f = new THREE.Group();
      f.position.set(0.14, 0.06, 0.06);
      g.add(f);
      part(new THREE.CylinderGeometry(0.014, 0.014, 0.24, 8), mat('#4fdc6a'), 0, 0.12, 0, f);
      const head = new THREE.Group();
      head.position.y = 0.26;
      head.rotation.x = 0.35;
      f.add(head);
      part(ball(0.05), mat('#ffd23f'), 0, 0, 0.02, head);
      const petal = mat(tint || '#ff5fa2');
      for (let i = 0; i < 6; i++) {
        const a = (i / 6) * Math.PI * 2;
        part(ball(0.055), petal, Math.cos(a) * 0.08, Math.sin(a) * 0.08, 0, head).scale.set(1, 1, 0.4);
      }
      acc.update = (t, speed) => {
        f.rotation.z = Math.sin(t * 2.4) * 0.12 - Math.min(0.3, speed * 0.03);
      };
      break;
    }
    case 'antenna': {
      const stalk = mat('#2b2b33');
      const tip = mat(tint || '#ffd23f', { rough: 0.35, glow: 0.35 });
      const arms: THREE.Object3D[] = [];
      for (const s of [-1, 1]) {
        const a = new THREE.Group();
        a.position.set(s * 0.12, 0.1, 0.02);
        a.rotation.z = -s * 0.3;
        g.add(a);
        part(new THREE.CylinderGeometry(0.012, 0.012, 0.3, 6), stalk, 0, 0.15, 0, a);
        part(ball(0.055), tip, 0, 0.31, 0, a);
        arms.push(a);
      }
      acc.update = (t, speed) => {
        for (const [i, a] of arms.entries()) {
          const s = i ? 1 : -1;
          a.rotation.z = -s * 0.3 + Math.sin(t * 7 + i * 1.7) * (0.06 + Math.min(0.2, speed * 0.025));
          a.rotation.x = -Math.min(0.4, speed * 0.05);
        }
      };
      break;
    }
  }
  return acc;
}

const EYE_X = 0.12;
const EYE_Y = 1.235;
/** Just in front of the eyes (their glints reach z ≈ 0.61). */
const LENS_Z = 0.63;

function heart(): THREE.Shape {
  const s = new THREE.Shape();
  s.moveTo(0, -0.09);
  s.bezierCurveTo(-0.02, -0.06, -0.11, -0.02, -0.11, 0.035);
  s.bezierCurveTo(-0.11, 0.1, -0.03, 0.11, 0, 0.055);
  s.bezierCurveTo(0.03, 0.11, 0.11, 0.1, 0.11, 0.035);
  s.bezierCurveTo(0.11, -0.02, 0.02, -0.06, 0, -0.09);
  return s;
}

export function makeGlasses(kind: Glasses): Accessory | null {
  if (kind === 'none') return null;
  const g = new THREE.Group();
  const acc: Accessory = { root: g, crownLift: 0 };
  const frames = (m: THREE.Material, r = 0.1) => {
    for (const s of [-1, 1]) part(new THREE.TorusGeometry(r, 0.014, 8, 32), m, s * EYE_X, EYE_Y, LENS_Z, g);
    const bridge = part(new THREE.CylinderGeometry(0.01, 0.01, 2 * (EYE_X - r) + 0.02, 6), m, 0, EYE_Y + 0.02, LENS_Z, g);
    bridge.rotation.z = Math.PI / 2;
    temples(m, r);
  };
  const temples = (m: THREE.Material, r: number) => {
    for (const s of [-1, 1]) {
      const from = new THREE.Vector3(s * (EYE_X + r), EYE_Y, LENS_Z - 0.01);
      const to = new THREE.Vector3(s * 0.49, EYE_Y + 0.02, 0.02);
      const t = part(new THREE.BoxGeometry(0.014, 0.018, from.distanceTo(to)), m, 0, 0, 0, g);
      t.position.lerpVectors(from, to, 0.5);
      t.lookAt(to);
    }
  };
  switch (kind) {
    case 'round':
      frames(mat('#2b2b33', { rough: 0.3, metal: 0.4 }));
      break;
    case 'shades': {
      frames(mat('#15151c', { rough: 0.3 }), 0.105);
      const lens = mat('#1d2233', { rough: 0.08, metal: 0.5 });
      for (const s of [-1, 1]) part(new THREE.CircleGeometry(0.105, 32), lens, s * EYE_X, EYE_Y, LENS_Z, g).scale.y = 0.85;
      break;
    }
    case 'hearts': {
      const geo = new THREE.ExtrudeGeometry(heart(), { depth: 0.02, bevelEnabled: false, curveSegments: 10 });
      const m = mat('#ff5fa2', { rough: 0.2, glow: 0.2 });
      for (const s of [-1, 1]) part(geo, m, s * EYE_X, EYE_Y, LENS_Z - 0.015, g).scale.setScalar(1.05);
      const bridge = part(new THREE.CylinderGeometry(0.01, 0.01, 0.06, 6), m, 0, EYE_Y + 0.04, LENS_Z, g);
      bridge.rotation.z = Math.PI / 2;
      temples(m, 0.11);
      break;
    }
    case 'monocle': {
      const gold = mat('#ffd23f', { rough: 0.25, metal: 0.8 });
      part(new THREE.TorusGeometry(0.1, 0.016, 8, 32), gold, -EYE_X, EYE_Y, LENS_Z, g);
      const chain = part(new THREE.CylinderGeometry(0.006, 0.006, 0.34, 6), gold, -EYE_X - 0.1, EYE_Y - 0.25, LENS_Z - 0.06, g);
      chain.rotation.z = -0.35;
      chain.rotation.x = -0.25;
      break;
    }
    case 'visor': {
      const r = 0.55;
      const arc = 0.62;
      part(
        new THREE.CylinderGeometry(r, r, 0.15, 32, 1, true, -arc, arc * 2),
        mat('#39e0d0', { rough: 0.1, metal: 0.3, glow: 0.35 }),
        0,
        EYE_Y,
        LENS_Z - r,
        g,
      );
      const rim = mat('#2b2b33', { rough: 0.4 });
      for (const dy of [-1, 1])
        part(
          new THREE.CylinderGeometry(r + 0.005, r + 0.005, 0.02, 32, 1, true, -arc, arc * 2),
          rim,
          0,
          EYE_Y + dy * 0.08,
          LENS_Z - r,
          g,
        );
      temples(rim, 0.2);
      break;
    }
  }
  g.traverse((o) => {
    o.castShadow = false;
  });
  return acc;
}
