import * as THREE from 'three';
import type { Builder, ModelName } from '../../sim/builder';
import type { LookId, PalKey, ResolvedLook } from '../../sim/looks';
import { clone } from './assets';
import { applySurface, surfaceForModelMaterial } from './materials';
import { patternMaterial, plainMaterial } from './view';

/**
 * Themed scenery around a map (client only): set pieces that belong to the round's look — castle
 * towers, factory gears, snowmen, planets, circus tents, neon rings, lighthouses, cacti, palms,
 * volcanoes, golden crowns, lollipops… — standing on floating islands or floating by themselves,
 * always well clear of anything solid; and the land far below (fields, sea, snow, lava…).
 */

export interface Box {
  min: THREE.Vector3;
  max: THREE.Vector3;
}
/** Is a volume (horizontal radius r, half height h) at p too close to the course? (scenery.ts) */
export type Blocked = (p: THREE.Vector3, r: number, h: number, margin: number) => boolean;

type Tick = (t: number) => void;
const UP = new THREE.Vector3(0, 1, 0);
interface Made {
  obj: THREE.Object3D;
  tick?: Tick;
}

interface Piece {
  /** Footprint radius and height at scale 1. */
  r: number;
  h: number;
  /** Stands on a floating island (else floats by itself). */
  island: boolean;
  weight?: number;
  make(k: Kit): Made;
}

type Shape = 'box' | 'sphere' | 'cyl' | 'cone' | 'cone4' | 'torus' | 'ring' | 'octa' | 'rock' | 'dome' | 'taper';
type V3 = readonly [number, number, number];

/** Shared bits for building pieces: unit shapes, materials, the look's colours, randomness. */
class Kit {
  private readonly geos = new Map<Shape, THREE.BufferGeometry>();
  private readonly glows = new Map<string, THREE.Material>();
  constructor(
    readonly b: Builder,
    readonly look: ResolvedLook,
    readonly rnd: () => number,
  ) {}

  private geo(s: Shape): THREE.BufferGeometry {
    let g = this.geos.get(s);
    if (!g) {
      g =
        s === 'box'
          ? new THREE.BoxGeometry(1, 1, 1)
          : s === 'sphere'
            ? new THREE.SphereGeometry(1, 20, 14)
            : s === 'cyl'
              ? new THREE.CylinderGeometry(1, 1, 1, 20)
              : s === 'taper'
                ? new THREE.CylinderGeometry(0.62, 1, 1, 20)
                : s === 'cone'
                  ? new THREE.ConeGeometry(1, 1, 20)
                  : s === 'cone4'
                    ? new THREE.ConeGeometry(1, 1, 4)
                    : s === 'torus'
                      ? new THREE.TorusGeometry(1, 0.3, 12, 36)
                      : s === 'ring'
                        ? new THREE.TorusGeometry(1, 0.07, 8, 56)
                        : s === 'octa'
                          ? new THREE.OctahedronGeometry(1, 0)
                          : s === 'rock'
                            ? new THREE.DodecahedronGeometry(1, 0)
                            : new THREE.SphereGeometry(1, 20, 10, 0, Math.PI * 2, 0, Math.PI / 2);
      this.b.view!.own(g);
      this.geos.set(s, g);
    }
    return g;
  }

  /** A part: unit `shape` scaled to `size`, at `at` in `parent` (rotated by `rot`). */
  part(parent: THREE.Object3D, shape: Shape, mat: THREE.Material, at: V3, size: V3, rot?: V3): THREE.Mesh {
    const m = new THREE.Mesh(this.geo(shape), mat);
    m.position.set(...at);
    m.scale.set(...size);
    if (rot) m.rotation.set(...rot);
    m.castShadow = false;
    m.receiveShadow = true;
    parent.add(m);
    return m;
  }

  col(k: PalKey, tone: 0 | 1 = 0) {
    return this.look.palette[k][tone];
  }
  plain(c: string, surface: Parameters<typeof plainMaterial>[2] = 'plastic', opts: THREE.MeshStandardMaterialParameters = {}) {
    return plainMaterial(c, opts, surface);
  }
  stripes(
    c1: string,
    c2: string,
    freq = 1.2,
    dir: [number, number] = [0, 1],
    kind: 'stripes' | 'checker' | 'dots' | 'waves' | 'chevron' = 'stripes',
  ) {
    return patternMaterial(c1, c2, freq, dir, 0, 'plastic', kind);
  }
  /** Unlit, glowing (not dimmed by the light, no fog). */
  glow(c: string, opacity = 1): THREE.Material {
    const key = `${c}|${opacity}`;
    let m = this.glows.get(key);
    if (!m) {
      m = this.b.view!.own(
        new THREE.MeshBasicMaterial({ color: c, toneMapped: false, transparent: opacity < 1, opacity, fog: false }),
      );
      this.glows.set(key, m);
    }
    return m;
  }
  pick<T>(list: readonly T[]): T {
    return list[Math.floor(this.rnd() * list.length)]!;
  }
  /** One of the look's bright palette colours. */
  bright(): string {
    return this.col(this.pick(['pink', 'yellow', 'blue', 'green', 'orange', 'purple', 'teal', 'red'] as const));
  }
  model(name: ModelName, parent: THREE.Object3D, at: V3, scale = 1, tint?: string) {
    const m = clone(name);
    m.position.set(...at);
    m.scale.setScalar(scale);
    m.rotation.y = this.rnd() * 6.3;
    // (Only a flag's cloth takes the tint.)
    if (tint) recolor(this.b, m, (n) => (n === 'Flag' ? tint : null));
    parent.add(m);
    return m;
  }
}

const recolored = new WeakMap<object, Map<string, THREE.Material>>();

/** Gives a model's materials new colours (copies, owned by the map). */
function recolor(
  b: Builder,
  root: THREE.Object3D,
  color: (name: string) => string | null,
  emissive?: (name: string) => string | null,
) {
  // Copies are shared across the build, so identical pieces stay batchable (statics.ts).
  let done = recolored.get(b.view!);
  if (!done) recolored.set(b.view!, (done = new Map()));
  root.traverse((o) => {
    if (!(o instanceof THREE.Mesh) || !(o.material instanceof THREE.MeshStandardMaterial)) return;
    const src = o.material;
    const c = color(src.name);
    const e = emissive?.(src.name);
    if (!c && !e) return;
    const key = `${src.uuid}|${c}|${e}`;
    let m = done.get(key);
    if (!m) {
      const copy = src.clone();
      if (c) copy.color.set(c);
      if (e) copy.emissive.set(e);
      // (A copy loses the surface detail: apply it again.)
      delete copy.userData.detail;
      applySurface(copy, surfaceForModelMaterial(copy.name));
      m = b.view!.own(copy);
      done.set(key, m);
    }
    o.material = m;
  });
}

/** An island in the look's colours. */
export function island(b: Builder, look: ResolvedLook): THREE.Object3D {
  const g = clone('island');
  const i = look.island;
  if (look.id !== 'classic' && look.id !== 'meadow')
    recolor(b, g, (n) => (n === 'Grass' ? i.grass : n === 'Rock' ? i.rock : n === 'Leaves' ? i.leaves : null));
  return g;
}

/** The clouds' material in the look's tint (null: as modelled). */
export function cloudTint(b: Builder, look: ResolvedLook, base: THREE.Material): THREE.Material {
  const c = look.sky.cloud;
  if (c.toLowerCase() === '#ffffff' || !(base instanceof THREE.MeshStandardMaterial)) return base;
  const m = base.clone();
  m.color.set(c);
  m.emissive.set(c).multiplyScalar(0.5);
  delete m.userData.detail;
  applySurface(m, 'cloud');
  return b.view!.own(m);
}

// ------------------------------------------------------------------ pieces

const flora = (k: Kit, parent: THREE.Object3D) => {
  const n = 2 + Math.floor(k.rnd() * 2);
  for (let i = 0; i < n; i++) {
    const a = k.rnd() * 6.3;
    const r = i ? 1.2 + k.rnd() * 1.2 : 0;
    k.model(
      k.pick(['tree', 'pine', 'tree', 'mushroom'] as const),
      parent,
      [Math.cos(a) * r, 0, Math.sin(a) * r],
      0.55 + k.rnd() * 0.3,
    );
  }
};

const grove: Piece = {
  r: 3,
  h: 5,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    flora(k, g);
    return { obj: g };
  },
};

const flowers: Piece = {
  r: 2.6,
  h: 1.6,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const stem = k.plain('#4fae4a', 'leaf');
    const heart = k.plain(k.col('yellow'), 'plastic');
    for (let i = 0; i < 9; i++) {
      const a = k.rnd() * 6.3;
      const r = 0.4 + k.rnd() * 2.2;
      const h = 0.6 + k.rnd() * 0.9;
      const x = Math.cos(a) * r;
      const z = Math.sin(a) * r;
      k.part(g, 'cyl', stem, [x, h / 2, z], [0.05, h, 0.05]);
      const head = k.plain(k.bright(), 'fabric');
      for (let p = 0; p < 5; p++) {
        const pa = (p / 5) * Math.PI * 2;
        k.part(g, 'sphere', head, [x + Math.cos(pa) * 0.2, h, z + Math.sin(pa) * 0.2], [0.18, 0.07, 0.18]);
      }
      k.part(g, 'sphere', heart, [x, h + 0.03, z], [0.11, 0.08, 0.11]);
    }
    return { obj: g };
  },
};

const windmill: Piece = {
  r: 2.2,
  h: 8,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    k.part(g, 'taper', k.plain(k.col('white'), 'wood'), [0, 2.6, 0], [1, 5.2, 1]);
    k.part(g, 'cone', k.plain(k.col('red'), 'wood'), [0, 5.9, 0], [1.1, 1.6, 1.1]);
    const hub = new THREE.Group();
    hub.position.set(0, 4.6, 0.75);
    g.add(hub);
    const sail = k.stripes(k.col('white'), k.col('pink'), 2.2, [1, 0]);
    for (let i = 0; i < 4; i++) {
      const arm = new THREE.Group();
      arm.rotation.z = (i * Math.PI) / 2;
      hub.add(arm);
      k.part(arm, 'box', sail, [0, 1.7, 0], [0.55, 3, 0.06]);
    }
    k.part(hub, 'sphere', k.plain(k.col('yellow')), [0, 0, 0.05], [0.25, 0.25, 0.25]);
    const sp = 0.6 + k.rnd() * 0.6;
    return { obj: g, tick: (t) => (hub.rotation.z = t * sp) };
  },
};

const tower: Piece = {
  r: 2,
  h: 11,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const stone = k.stripes('#d8d2c6', '#bdb5a8', 1.6, [0, 1], 'checker');
    k.part(g, 'cyl', stone, [0, 3.5, 0], [1.4, 7, 1.4]);
    for (let i = 0; i < 8; i++) {
      const a = (i / 8) * Math.PI * 2;
      k.part(g, 'box', stone, [Math.cos(a) * 1.35, 7.25, Math.sin(a) * 1.35], [0.45, 0.5, 0.45], [0, -a, 0]);
    }
    const roof = k.pick(['red', 'blue', 'purple'] as const);
    k.part(g, 'cone', k.stripes(k.col(roof), k.col(roof, 1), 2, [0, 1]), [0, 8.8, 0], [1.65, 2.6, 1.65]);
    k.part(g, 'box', k.plain('#3a3048'), [0, 4.2, 1.38], [0.35, 0.7, 0.1]);
    k.model('flag', g, [0, 9.9, 0], 0.45, k.col('yellow'));
    return { obj: g };
  },
};

const keep: Piece = {
  r: 3.2,
  h: 8,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const stone = k.stripes('#d8d2c6', '#c4bcae', 1.4, [1, 1], 'checker');
    k.part(g, 'box', stone, [0, 2.4, 0], [4, 4.8, 4]);
    const roof = k.plain(k.col('blue'), 'plastic');
    for (const sx of [-1, 1])
      for (const sz of [-1, 1]) {
        k.part(g, 'cyl', stone, [sx * 2, 3, sz * 2], [0.7, 6, 0.7]);
        k.part(g, 'cone', roof, [sx * 2, 6.8, sz * 2], [0.85, 1.6, 0.85]);
      }
    const banner = k.stripes(k.col('red'), k.col('yellow'), 1.8, [1, 0], 'chevron');
    k.part(g, 'box', banner, [0, 3.2, 2.03], [1.3, 2.4, 0.05]);
    k.part(g, 'box', k.plain('#3a3048'), [0, 0.8, 2.02], [1, 1.6, 0.05]);
    return { obj: g };
  },
};

const banners: Piece = {
  r: 2.2,
  h: 6,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const pole = k.plain('#d8c090', 'gold', { metalness: 0.7, roughness: 0.3 });
    const ph = k.rnd() * 6;
    const cloths: THREE.Object3D[] = [];
    for (let i = 0; i < 3; i++) {
      const x = (i - 1) * 1.6;
      k.part(g, 'cyl', pole, [x, 2.8, 0], [0.07, 5.6, 0.07]);
      k.part(g, 'sphere', pole, [x, 5.7, 0], [0.16, 0.16, 0.16]);
      const c = k.bright();
      const cloth = new THREE.Group();
      cloth.position.set(x, 5.3, 0.1);
      g.add(cloth);
      k.part(cloth, 'box', k.stripes(c, k.col('white'), 1.6, [0, 1], 'chevron'), [0, -1.3, 0], [0.9, 2.6, 0.04]);
      cloths.push(cloth);
    }
    return {
      obj: g,
      tick: (t) => {
        cloths.forEach((c, i) => {
          c.rotation.x = Math.sin(t * 1.3 + ph + i) * 0.08;
        });
      },
    };
  },
};

const gear: Piece = {
  r: 3.4,
  h: 7,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const wheel = new THREE.Group();
    wheel.position.y = 3.5;
    g.add(wheel);
    const metal = k.plain(k.pick([k.col('orange'), k.col('yellow'), '#9aa3b0']), 'metal', { metalness: 0.6, roughness: 0.35 });
    k.part(wheel, 'cyl', metal, [0, 0, 0], [2.6, 0.6, 2.6], [Math.PI / 2, 0, 0]);
    const teeth = 12;
    for (let i = 0; i < teeth; i++) {
      const a = (i / teeth) * Math.PI * 2;
      k.part(wheel, 'box', metal, [Math.cos(a) * 2.85, Math.sin(a) * 2.85, 0], [0.7, 0.7, 0.6], [0, 0, a]);
    }
    k.part(wheel, 'cyl', k.plain('#4a4f5a', 'metal'), [0, 0, 0], [0.6, 0.9, 0.6], [Math.PI / 2, 0, 0]);
    for (let i = 0; i < 4; i++)
      k.part(
        wheel,
        'cyl',
        k.plain('#3a3f4a', 'metal'),
        [Math.cos(i * 1.57) * 1.5, Math.sin(i * 1.57) * 1.5, 0],
        [0.35, 0.8, 0.35],
        [Math.PI / 2, 0, 0],
      );
    g.rotation.y = k.rnd() * 6.3;
    const sp = (k.rnd() < 0.5 ? -1 : 1) * (0.3 + k.rnd() * 0.4);
    return { obj: g, tick: (t) => (wheel.rotation.z = t * sp) };
  },
};

const chimney: Piece = {
  r: 1.6,
  h: 13,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    k.part(g, 'taper', k.stripes(k.col('red'), k.col('white'), 0.45, [0, 1]), [0, 5, 0], [0.9, 10, 0.9]);
    k.part(g, 'cyl', k.plain('#4a4f5a', 'metal'), [0, 10.1, 0], [0.62, 0.4, 0.62]);
    const smoke = k.plain('#e8e4de', 'cloud', { transparent: true, opacity: 0.75 });
    const puffs = [0, 1, 2, 3].map(() => k.part(g, 'sphere', smoke, [0, 10.5, 0], [0.6, 0.6, 0.6]));
    const ph = k.rnd() * 4;
    return {
      obj: g,
      tick: (t) =>
        puffs.forEach((p, i) => {
          const f = (t * 0.25 + ph + i / puffs.length) % 1;
          p.position.set(f * 1.6, 10.5 + f * 5, f * 0.6);
          p.scale.setScalar(0.5 + f * 1.4);
          p.visible = f < 0.95;
        }),
    };
  },
};

const tank: Piece = {
  r: 2.6,
  h: 5,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const metal = k.plain('#aeb6c2', 'metal', { metalness: 0.6, roughness: 0.3 });
    k.part(g, 'cyl', k.stripes(k.col('yellow'), '#3a3f4a', 1.4, [1, 1], 'chevron'), [0, 0.4, 0], [2, 0.8, 2]);
    k.part(g, 'cyl', metal, [0, 2.2, 0], [1.9, 2.8, 1.9]);
    k.part(g, 'dome', metal, [0, 3.6, 0], [1.9, 0.9, 1.9]);
    const pipe = k.plain(k.col('teal'), 'metal');
    k.part(g, 'cyl', pipe, [2.2, 2.6, 0], [0.25, 2.2, 0.25], [0, 0, Math.PI / 2]);
    k.part(g, 'cyl', pipe, [3.2, 1.6, 0], [0.25, 2.2, 0.25]);
    return { obj: g };
  },
};

const snowPine: Piece = {
  r: 2,
  h: 5,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const snow = k.plain('#ffffff', 'cloth');
    for (let i = 0; i < 2; i++) {
      const a = k.rnd() * 6.3;
      const r = i ? 1.4 : 0;
      const s = 0.7 + k.rnd() * 0.4;
      const x = Math.cos(a) * r;
      const z = Math.sin(a) * r;
      k.model('pine', g, [x, 0, z], s);
      k.part(g, 'cone', snow, [x, 4.25 * s, z], [0.5 * s, 0.7 * s, 0.5 * s]);
      k.part(g, 'cone', snow, [x, 3.35 * s, z], [0.85 * s, 0.35 * s, 0.85 * s]);
      k.part(g, 'cone', snow, [x, 2.45 * s, z], [1.2 * s, 0.35 * s, 1.2 * s]);
    }
    return { obj: g };
  },
};

const snowman: Piece = {
  r: 1.3,
  h: 3.6,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const snow = k.plain('#ffffff', 'cloth');
    k.part(g, 'sphere', snow, [0, 0.9, 0], [1, 0.95, 1]);
    k.part(g, 'sphere', snow, [0, 2.2, 0], [0.72, 0.7, 0.72]);
    k.part(g, 'sphere', snow, [0, 3.15, 0], [0.52, 0.5, 0.52]);
    const coal = k.plain('#2a2a33');
    for (const sx of [-1, 1]) k.part(g, 'sphere', coal, [sx * 0.18, 3.28, 0.44], [0.06, 0.06, 0.06]);
    for (let i = 0; i < 3; i++) k.part(g, 'sphere', coal, [0, 2.5 - i * 0.3, 0.68], [0.07, 0.07, 0.07]);
    k.part(g, 'cone', k.plain('#ff8a3d'), [0, 3.15, 0.62], [0.08, 0.45, 0.08], [Math.PI / 2, 0, 0]);
    k.part(g, 'cyl', coal, [0, 3.62, 0], [0.55, 0.06, 0.55]);
    k.part(g, 'cyl', coal, [0, 3.9, 0], [0.36, 0.55, 0.36]);
    k.part(g, 'torus', k.plain(k.col('red'), 'fabric'), [0, 2.72, 0], [0.5, 0.5, 0.35], [Math.PI / 2, 0, 0]);
    g.rotation.y = k.rnd() * 6.3;
    return { obj: g };
  },
};

const crystals = (glowing: boolean): Piece => ({
  r: 1.8,
  h: 4,
  island: false,
  make: (k) => {
    const g = new THREE.Group();
    const spin = new THREE.Group();
    spin.position.y = 2;
    g.add(spin);
    const n = 3 + Math.floor(k.rnd() * 3);
    for (let i = 0; i < n; i++) {
      const c = glowing ? k.bright() : '#cdeeff';
      const mat = glowing
        ? k.glow(c, 0.9)
        : k.plain(c, 'ice', { roughness: 0.35, emissive: new THREE.Color('#9fd8ff'), emissiveIntensity: 0.25 });
      const a = (i / n) * Math.PI * 2;
      const r = i ? 0.8 : 0;
      const s = i ? 0.35 + k.rnd() * 0.25 : 0.55;
      k.part(spin, 'octa', mat, [Math.cos(a) * r, 0, Math.sin(a) * r], [s, s * 2.6, s], [0, a, (k.rnd() - 0.5) * 0.5]);
    }
    const sp = 0.2 + k.rnd() * 0.3;
    const ph = k.rnd() * 6;
    return {
      obj: g,
      tick: (t) => {
        spin.rotation.y = t * sp;
        spin.position.y = 2 + Math.sin(t * 0.8 + ph) * 0.4;
      },
    };
  },
});

const planet: Piece = {
  r: 5,
  h: 6,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const body = new THREE.Group();
    body.position.y = 3;
    g.add(body);
    const c = k.bright();
    k.part(body, 'sphere', k.stripes(c, k.col('white'), 0.9, [0, 1], 'waves'), [0, 0, 0], [2.4, 2.4, 2.4]);
    const tilt = new THREE.Group();
    tilt.rotation.set(0.5, 0, 0.3);
    body.add(tilt);
    k.part(tilt, 'ring', k.glow(k.bright(), 0.85), [0, 0, 0], [3.8, 3.8, 1], [Math.PI / 2, 0, 0]);
    k.part(tilt, 'ring', k.glow(k.col('white'), 0.6), [0, 0, 0], [4.4, 4.4, 1], [Math.PI / 2, 0, 0]);
    const sp = 0.1 + k.rnd() * 0.15;
    return { obj: g, tick: (t) => (body.rotation.y = t * sp) };
  },
};

const orbs: Piece = {
  r: 3,
  h: 5,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const list = Array.from({ length: 3 + Math.floor(k.rnd() * 3) }, () => {
      const c = k.bright();
      const o = new THREE.Group();
      k.part(o, 'sphere', k.glow(c), [0, 0, 0], [0.45, 0.45, 0.45]);
      k.part(o, 'sphere', k.glow(c, 0.25), [0, 0, 0], [0.8, 0.8, 0.8]);
      g.add(o);
      return { o, x: (k.rnd() - 0.5) * 4, z: (k.rnd() - 0.5) * 4, y: 1 + k.rnd() * 3, ph: k.rnd() * 6 };
    });
    return {
      obj: g,
      tick: (t) => {
        for (const s of list) s.o.position.set(s.x + Math.sin(t * 0.4 + s.ph) * 0.4, s.y + Math.sin(t * 0.9 + s.ph) * 0.5, s.z);
      },
    };
  },
};

const tent: Piece = {
  r: 3.3,
  h: 6.5,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const [a, b] = k.pick([
      ['red', 'white'],
      ['blue', 'yellow'],
      ['pink', 'white'],
      ['purple', 'yellow'],
    ] as const);
    const cloth = patternMaterial(k.col(a), k.col(b), 2.4, [1, 0], 0, 'cloth', 'stripes');
    k.part(g, 'cyl', cloth, [0, 1.3, 0], [2.8, 2.6, 2.8]);
    k.part(g, 'cone', cloth, [0, 3.9, 0], [3.1, 2.6, 3.1]);
    k.part(g, 'box', k.plain('#3a2040'), [0, 0.9, 2.72], [1.2, 1.8, 0.2]);
    k.part(g, 'cyl', k.plain(k.col('yellow'), 'gold'), [0, 5.4, 0], [0.06, 0.8, 0.06]);
    k.model('flag', g, [0, 5.2, 0], 0.35, k.col('yellow'));
    for (let i = 0; i < 12; i++) {
      const aa = (i / 12) * Math.PI * 2;
      k.part(
        g,
        'sphere',
        k.plain(i % 2 ? k.col(a) : k.col(b)),
        [Math.cos(aa) * 2.95, 2.62, Math.sin(aa) * 2.95],
        [0.22, 0.22, 0.22],
      );
    }
    return { obj: g };
  },
};

const balloonBunch: Piece = {
  r: 2,
  h: 6,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const sway = new THREE.Group();
    g.add(sway);
    const string = k.plain('#ffffff', 'fabric');
    for (let i = 0; i < 7; i++) {
      const a = (i / 7) * Math.PI * 2;
      const r = 0.6 + k.rnd() * 0.6;
      const x = Math.cos(a) * r;
      const z = Math.sin(a) * r;
      const y = 3.6 + k.rnd() * 1.4;
      k.part(sway, 'sphere', k.plain(k.bright(), 'rubber', { roughness: 0.25 }), [x, y, z], [0.55, 0.68, 0.55]);
      const len = Math.hypot(x, y, z);
      const s = k.part(sway, 'cyl', string, [x / 2, y / 2, z / 2], [0.015, len, 0.015]);
      s.quaternion.setFromUnitVectors(UP, new THREE.Vector3(x, y, z).normalize());
    }
    const ph = k.rnd() * 6;
    return {
      obj: g,
      tick: (t) => {
        sway.rotation.set(Math.sin(t * 0.7 + ph) * 0.08, t * 0.1, Math.cos(t * 0.6 + ph) * 0.08);
      },
    };
  },
};

const ferris: Piece = {
  r: 4.4,
  h: 10,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const metal = k.plain(k.col('white'), 'metal', { metalness: 0.4 });
    for (const sz of [-0.6, 0.6])
      for (const sx of [-1, 1]) k.part(g, 'box', metal, [sx * 1.4, 2.6, sz], [0.25, 5.6, 0.25], [0, 0, sx * -0.25]);
    const wheel = new THREE.Group();
    wheel.position.y = 5.2;
    g.add(wheel);
    k.part(wheel, 'ring', k.plain(k.col('pink'), 'metal'), [0, 0, 0], [3.8, 3.8, 3.8]);
    k.part(wheel, 'ring', k.plain(k.col('pink'), 'metal'), [0, 0, 0], [1.2, 1.2, 1.2]);
    const cabins: THREE.Object3D[] = [];
    const n = 8;
    for (let i = 0; i < n; i++) {
      const a = (i / n) * Math.PI * 2;
      k.part(wheel, 'box', metal, [Math.cos(a) * 1.9, Math.sin(a) * 1.9, 0], [3.8, 0.1, 0.1], [0, 0, a]);
      const cab = new THREE.Group();
      cab.position.set(Math.cos(a) * 3.8, Math.sin(a) * 3.8, 0);
      wheel.add(cab);
      k.part(cab, 'box', k.plain(k.bright(), 'plastic'), [0, -0.45, 0], [0.7, 0.6, 0.7]);
      k.part(cab, 'box', metal, [0, -0.05, 0], [0.8, 0.08, 0.8]);
      cabins.push(cab);
    }
    const sp = 0.15 + k.rnd() * 0.1;
    return {
      obj: g,
      tick: (t) => {
        wheel.rotation.z = t * sp;
        for (const c of cabins) c.rotation.z = -t * sp;
      },
    };
  },
};

const neonRings: Piece = {
  r: 3.2,
  h: 7,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const rings = [0, 1, 2].map((i) => {
      const r = k.part(g, 'ring', k.glow(k.bright()), [0, 3.5, 0], [2.8 - i * 0.6, 2.8 - i * 0.6, 2.8 - i * 0.6]);
      return r;
    });
    k.part(g, 'sphere', k.glow(k.col('white')), [0, 3.5, 0], [0.4, 0.4, 0.4]);
    const sp = 0.4 + k.rnd() * 0.5;
    return {
      obj: g,
      tick: (t) =>
        rings.forEach((r, i) => {
          r.rotation.set(t * sp * (i + 1) * 0.6, t * sp * (1.4 - i * 0.3), i);
        }),
    };
  },
};

const pylon: Piece = {
  r: 1.5,
  h: 10,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const dark = k.plain('#1d1438', 'metal', { metalness: 0.5, roughness: 0.3 });
    k.part(g, 'box', dark, [0, 4, 0], [1.1, 8, 1.1]);
    const c = k.bright();
    for (const sx of [-1, 1]) for (const sz of [-1, 1]) k.part(g, 'box', k.glow(c), [sx * 0.57, 4, sz * 0.57], [0.08, 8, 0.08]);
    for (let i = 0; i < 4; i++) k.part(g, 'box', k.glow(c, 0.8), [0, 1 + i * 2, 0], [1.16, 0.08, 1.16]);
    const top = k.part(g, 'octa', k.glow(k.bright()), [0, 9.3, 0], [0.7, 0.9, 0.7]);
    const ph = k.rnd() * 6;
    return {
      obj: g,
      tick: (t) => {
        top.rotation.y = t * 1.2;
        top.position.y = 9.3 + Math.sin(t * 1.5 + ph) * 0.25;
      },
    };
  },
};

const lighthouse: Piece = {
  r: 1.8,
  h: 10,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    k.part(g, 'taper', k.stripes(k.col('red'), '#ffffff', 0.55, [0, 1]), [0, 3.5, 0], [1.2, 7, 1.2]);
    k.part(g, 'cyl', k.plain('#3a3f4a', 'metal'), [0, 7.1, 0], [1.05, 0.2, 1.05]);
    k.part(g, 'cyl', k.plain('#ffffff', 'glass', { transparent: true, opacity: 0.5 }), [0, 7.7, 0], [0.6, 1, 0.6]);
    const lamp = k.part(g, 'sphere', k.glow('#fff2a0'), [0, 7.7, 0], [0.35, 0.35, 0.35]);
    k.part(g, 'cone', k.plain(k.col('red')), [0, 8.6, 0], [0.8, 0.9, 0.8]);
    const beam = new THREE.Group();
    beam.position.set(0, 7.7, 0);
    g.add(beam);
    k.part(beam, 'cone', k.glow('#fff6c0', 0.18), [0, 0, 4], [0.9, 8, 0.9], [-Math.PI / 2, 0, 0]);
    return {
      obj: g,
      tick: (t) => {
        beam.rotation.y = t * 0.8;
        lamp.scale.setScalar(0.33 + Math.sin(t * 4) * 0.04);
      },
    };
  },
};

const palm: Piece = {
  r: 2.8,
  h: 6,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const bark = k.stripes('#b88a5a', '#9a6f45', 3, [0, 1]);
    const lean = (k.rnd() - 0.5) * 0.5;
    let x = 0;
    let y = 0;
    for (let i = 0; i < 7; i++) {
      x += lean * 0.3 * (i / 3);
      k.part(g, 'cyl', bark, [x, y + 0.4, 0], [0.26 - i * 0.015, 0.82, 0.26 - i * 0.015], [0, 0, -lean * 0.3 * (i / 3)]);
      y += 0.78;
    }
    const leaf = k.plain(k.look.id === 'jungle' ? '#2fae4a' : '#4fcf5a', 'leaf');
    for (let i = 0; i < 7; i++) {
      const a = (i / 7) * Math.PI * 2;
      const l = new THREE.Group();
      l.position.set(x, y, 0);
      l.rotation.set(0, a, 0);
      g.add(l);
      k.part(l, 'sphere', leaf, [1.3, -0.35, 0], [1.4, 0.08, 0.35], [0, 0, -0.45]);
    }
    for (let i = 0; i < 3; i++)
      k.part(g, 'sphere', k.plain('#7a5030'), [x + Math.cos(i * 2.1) * 0.3, y - 0.3, Math.sin(i * 2.1) * 0.3], [0.2, 0.2, 0.2]);
    return { obj: g };
  },
};

const beach: Piece = {
  r: 2.8,
  h: 3.5,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const c = k.bright();
    k.part(g, 'cyl', k.plain('#ffffff'), [0, 1.3, 0], [0.05, 2.6, 0.05]);
    k.part(g, 'cone', patternMaterial(c, '#ffffff', 3, [1, 0], 0, 'cloth', 'stripes'), [0, 2.75, 0], [1.6, 0.6, 1.6]);
    k.part(g, 'box', k.stripes(k.bright(), '#ffffff', 2, [1, 0]), [0.6, 0.03, 1.2], [1, 0.04, 1.9]);
    k.part(
      g,
      'sphere',
      patternMaterial(k.bright(), '#ffffff', 2.2, [1, 0], 0, 'rubber', 'stripes'),
      [-1.3, 0.35, 0.8],
      [0.35, 0.35, 0.35],
    );
    g.rotation.y = k.rnd() * 6.3;
    return { obj: g };
  },
};

const cactus: Piece = {
  r: 1.5,
  h: 5,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const green = k.stripes('#5a9a4a', '#6aae56', 5, [1, 0]);
    const h = 3.2 + k.rnd() * 1.5;
    k.part(g, 'cyl', green, [0, h / 2, 0], [0.45, h, 0.45]);
    k.part(g, 'sphere', green, [0, h, 0], [0.45, 0.45, 0.45]);
    for (const side of [-1, 1]) {
      const ay = 1.2 + k.rnd() * 1.2;
      const up = 0.8 + k.rnd() * 0.8;
      k.part(g, 'cyl', green, [side * 0.6, ay, 0], [0.26, 0.7, 0.26], [0, 0, Math.PI / 2]);
      k.part(g, 'cyl', green, [side * 0.9, ay + up / 2, 0], [0.26, up, 0.26]);
      k.part(g, 'sphere', green, [side * 0.9, ay + up, 0], [0.26, 0.26, 0.26]);
    }
    k.part(g, 'sphere', k.plain(k.col('pink'), 'fabric'), [0, h + 0.4, 0], [0.2, 0.15, 0.2]);
    g.rotation.y = k.rnd() * 6.3;
    return { obj: g };
  },
};

const mesa: Piece = {
  r: 4.4,
  h: 7,
  island: false,
  make: (k) => {
    const g = new THREE.Group();
    const layers = ['#c8764a', '#e0a070', '#b8603a', '#e8b888', '#a8503a'];
    let y = 0;
    for (let i = 0; i < 5; i++) {
      const r = 4 - i * 0.35 - k.rnd() * 0.2;
      const h = 0.8 + k.rnd() * 0.6;
      k.part(g, 'cyl', k.plain(layers[i]!, 'rock'), [0, y - h / 2, 0], [r, h, r]);
      y -= h;
    }
    k.part(g, 'cone', k.plain('#a8503a', 'rock'), [0, y - 1.2, 0], [3, 2.4, 3], [Math.PI, 0, 0]);
    const top = new THREE.Group();
    g.add(top);
    k.model('flag', top, [0, 0, 0], 0.5, k.col('red'));
    g.position.y = 0;
    return { obj: g };
  },
};

const pyramid: Piece = {
  r: 3.3,
  h: 4,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    k.part(g, 'cone4', k.stripes('#f0d090', '#e0bc78', 1.6, [0, 1]), [0, 1.8, 0], [3, 3.6, 3], [0, Math.PI / 4, 0]);
    k.part(
      g,
      'cone4',
      k.plain(k.col('yellow'), 'gold', { metalness: 0.7, roughness: 0.3 }),
      [0, 3.35, 0],
      [0.5, 0.6, 0.5],
      [0, Math.PI / 4, 0],
    );
    return { obj: g };
  },
};

const bigPlant: Piece = {
  r: 2.8,
  h: 3.5,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const leaf = k.stripes('#2f9f3f', '#48b858', 2, [1, 0], 'stripes');
    for (let i = 0; i < 7; i++) {
      const l = new THREE.Group();
      l.rotation.y = (i / 7) * Math.PI * 2 + k.rnd() * 0.3;
      g.add(l);
      k.part(l, 'sphere', leaf, [1.1, 0.9, 0], [1.3, 0.1, 0.45], [0, 0, 0.5 + k.rnd() * 0.3]);
    }
    const c = k.bright();
    const petal = k.plain(c, 'fabric');
    for (let p = 0; p < 6; p++) {
      const a = (p / 6) * Math.PI * 2;
      k.part(g, 'sphere', petal, [Math.cos(a) * 0.45, 2.2, Math.sin(a) * 0.45], [0.42, 0.1, 0.25], [0, -a, 0.25]);
    }
    k.part(g, 'sphere', k.plain(k.col('yellow')), [0, 2.25, 0], [0.22, 0.18, 0.22]);
    k.part(g, 'cyl', k.plain('#3f8f3a', 'leaf'), [0, 1.1, 0], [0.08, 2.2, 0.08]);
    return { obj: g };
  },
};

const volcano: Piece = {
  r: 4.4,
  h: 7,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const rock = k.plain('#3a2a2a', 'rock');
    k.part(g, 'taper', rock, [0, 1.6, 0], [4, 3.2, 4]);
    k.part(g, 'cone', rock, [0, -1.5, 0], [4, 3, 4], [Math.PI, 0, 0]);
    k.part(g, 'cyl', k.glow('#ff7a2a'), [0, 3.22, 0], [2.3, 0.05, 2.3]);
    const lava = k.glow('#ffb030', 0.9);
    for (let i = 0; i < 3; i++) {
      const a = k.rnd() * 6.3;
      k.part(g, 'box', lava, [Math.cos(a) * 2.95, 1.6, Math.sin(a) * 2.95], [0.25, 3, 0.05], [0.3, -a + Math.PI / 2, 0]);
    }
    const smoke = k.plain('#5a4a4a', 'cloud', { transparent: true, opacity: 0.6 });
    const puffs = [0, 1, 2, 3].map(() => k.part(g, 'sphere', smoke, [0, 3.4, 0], [1, 1, 1]));
    const ph = k.rnd() * 4;
    return {
      obj: g,
      tick: (t) =>
        puffs.forEach((p, i) => {
          const f = (t * 0.18 + ph + i / puffs.length) % 1;
          p.position.set(f * 2, 3.4 + f * 7, -f);
          p.scale.setScalar(0.8 + f * 2.2);
          p.visible = f < 0.95;
        }),
    };
  },
};

const rocks: Piece = {
  r: 2.8,
  h: 4,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const dark = k.plain('#2e2426', 'rock');
    const list = [0, 1, 2, 3].map((i) => {
      const o = k.part(
        g,
        'rock',
        i ? dark : k.plain('#4a2e2a', 'rock'),
        [(k.rnd() - 0.5) * 4, 1 + k.rnd() * 3, (k.rnd() - 0.5) * 4],
        [0.6 + k.rnd() * 0.9, 0.6 + k.rnd() * 0.9, 0.6 + k.rnd() * 0.9],
      );
      k.part(o, 'octa', k.glow('#ff8a3a'), [0, 0, 0], [0.3, 0.3, 0.3]);
      return { o, y: o.position.y, ph: k.rnd() * 6 };
    });
    return {
      obj: g,
      tick: (t) => {
        for (const r of list) {
          r.o.position.y = r.y + Math.sin(t * 0.5 + r.ph) * 0.3;
          r.o.rotation.y = t * 0.1 + r.ph;
        }
      },
    };
  },
};

const shards: Piece = {
  r: 2,
  h: 5,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const obsidian = k.plain('#241a2a', 'glass', { roughness: 0.15, metalness: 0.3 });
    for (let i = 0; i < 5; i++) {
      const a = k.rnd() * 6.3;
      const r = i ? 0.6 + k.rnd() * 1.2 : 0;
      const h = 1.5 + k.rnd() * 3;
      k.part(
        g,
        'cone4',
        obsidian,
        [Math.cos(a) * r, h / 2, Math.sin(a) * r],
        [0.4, h, 0.4],
        [(k.rnd() - 0.5) * 0.3, a, (k.rnd() - 0.5) * 0.3],
      );
    }
    k.part(g, 'sphere', k.glow('#ff7a2a', 0.8), [0, 0.1, 0], [1.4, 0.1, 1.4]);
    return { obj: g };
  },
};

const pillar: Piece = {
  r: 1.4,
  h: 7.5,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const gold = k.plain('#f2c14e', 'gold', { metalness: 0.8, roughness: 0.25 });
    const marble = k.stripes('#fff8ee', '#efe6f6', 2, [1, 1], 'waves');
    k.part(g, 'box', marble, [0, 0.3, 0], [1.8, 0.6, 1.8]);
    k.part(g, 'cyl', marble, [0, 3.4, 0], [0.55, 5.6, 0.55]);
    for (let i = 0; i < 3; i++) k.part(g, 'torus', gold, [0, 1 + i * 2.2, 0], [0.58, 0.58, 0.58], [Math.PI / 2, 0, 0]);
    k.part(g, 'box', marble, [0, 6.35, 0], [1.4, 0.3, 1.4]);
    const orb = k.part(g, 'sphere', gold, [0, 7, 0], [0.45, 0.45, 0.45]);
    const ph = k.rnd() * 6;
    return { obj: g, tick: (t) => (orb.position.y = 7.1 + Math.sin(t * 1.2 + ph) * 0.15) };
  },
};

const crown: Piece = {
  r: 2.8,
  h: 4,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const spin = new THREE.Group();
    spin.position.y = 2;
    spin.rotation.x = 0.15;
    g.add(spin);
    const gold = k.plain('#ffcf3f', 'gold', { metalness: 0.85, roughness: 0.2 });
    k.part(spin, 'cyl', gold, [0, 0, 0], [2, 0.7, 2]);
    k.part(spin, 'cyl', k.plain(k.col('red'), 'fabric'), [0, 0.1, 0], [1.9, 0.72, 1.9]);
    const n = 7;
    for (let i = 0; i < n; i++) {
      const a = (i / n) * Math.PI * 2;
      k.part(spin, 'cone', gold, [Math.cos(a) * 1.85, 0.95, Math.sin(a) * 1.85], [0.35, 1.2, 0.35]);
      k.part(
        spin,
        'sphere',
        k.glow(i % 2 ? k.col('blue') : k.col('pink')),
        [Math.cos(a) * 1.85, 1.6, Math.sin(a) * 1.85],
        [0.16, 0.16, 0.16],
      );
      k.part(spin, 'octa', k.glow(k.col('teal')), [Math.cos(a) * 2.02, 0, Math.sin(a) * 2.02], [0.14, 0.2, 0.14]);
    }
    const ph = k.rnd() * 6;
    return {
      obj: g,
      tick: (t) => {
        spin.rotation.y = t * 0.25;
        spin.position.y = 2 + Math.sin(t * 0.6 + ph) * 0.4;
      },
    };
  },
};

const lollipop: Piece = {
  r: 1.6,
  h: 6,
  island: true,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const h = 3.5 + k.rnd() * 1.5;
    k.part(g, 'cyl', k.plain('#ffffff'), [0, h / 2, 0], [0.09, h, 0.09]);
    const candy = patternMaterial(
      k.bright(),
      '#ffffff',
      2.6,
      [1, 1],
      0,
      'glossy',
      k.pick(['waves', 'stripes', 'chevron'] as const),
    );
    const disc = k.part(g, 'cyl', candy, [0, h + 1.2, 0], [1.3, 0.35, 1.3], [Math.PI / 2, 0, 0]);
    const ph = k.rnd() * 6;
    return { obj: g, tick: (t) => (disc.rotation.y = Math.sin(t * 0.5 + ph) * 0.4) };
  },
};

const cane: Piece = {
  r: 1.4,
  h: 6,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    const stripes = patternMaterial(k.col('red'), '#ffffff', 2.2, [1, 1.6], 0, 'glossy', 'stripes');
    k.part(g, 'cyl', stripes, [0, 2.2, 0], [0.28, 4.4, 0.28]);
    const hook = new THREE.Mesh(k.b.view!.own(new THREE.TorusGeometry(0.7, 0.28, 12, 24, Math.PI)), stripes);
    hook.position.set(0.7, 4.4, 0);
    g.add(hook);
    g.rotation.y = k.rnd() * 6.3;
    return { obj: g };
  },
};

const donut: Piece = {
  r: 2.2,
  h: 3,
  island: false,
  weight: 2,
  make: (k) => {
    const g = new THREE.Group();
    const spin = new THREE.Group();
    spin.position.y = 1.5;
    spin.rotation.x = 0.4 + k.rnd() * 0.5;
    g.add(spin);
    k.part(spin, 'torus', k.plain('#e8b878', 'plastic'), [0, 0, 0], [1.4, 1.4, 1.4], [Math.PI / 2, 0, 0]);
    k.part(
      spin,
      'torus',
      k.plain(k.bright(), 'glossy', { roughness: 0.3 }),
      [0, 0.14, 0],
      [1.42, 1.42, 1.2],
      [Math.PI / 2, 0, 0],
    );
    for (let i = 0; i < 9; i++) {
      const a = k.rnd() * 6.3;
      const r = 1.1 + k.rnd() * 0.6;
      k.part(spin, 'box', k.plain(k.bright()), [Math.cos(a) * r, 0.55, Math.sin(a) * r], [0.06, 0.06, 0.2], [0, k.rnd() * 3, 0]);
    }
    const ph = k.rnd() * 6;
    return {
      obj: g,
      tick: (t) => {
        spin.rotation.z = t * 0.3;
        spin.position.y = 1.5 + Math.sin(t * 0.7 + ph) * 0.3;
      },
    };
  },
};

const gumdrops: Piece = {
  r: 2.6,
  h: 1.8,
  island: true,
  make: (k) => {
    const g = new THREE.Group();
    for (let i = 0; i < 6; i++) {
      const a = k.rnd() * 6.3;
      const r = i ? 0.8 + k.rnd() * 1.6 : 0;
      const s = 0.45 + k.rnd() * 0.4;
      k.part(
        g,
        'dome',
        k.plain(k.bright(), 'glossy', { roughness: 0.2 }),
        [Math.cos(a) * r, 0, Math.sin(a) * r],
        [s, s * 1.5, s],
      );
    }
    return { obj: g };
  },
};

/** The pieces of each look (weighted). */
const SETS: Record<LookId, Piece[]> = {
  classic: [],
  meadow: [grove, flowers, windmill],
  castle: [tower, keep, banners, grove],
  factory: [gear, chimney, tank],
  snow: [snowPine, snowman, crystals(false)],
  starlight: [planet, orbs, crystals(true)],
  circus: [tent, balloonBunch, ferris, banners],
  neon: [neonRings, pylon, crystals(true)],
  ocean: [lighthouse, palm, beach],
  desert: [cactus, mesa, pyramid, palm],
  jungle: [palm, bigPlant, flowers, grove],
  lava: [volcano, rocks, shards],
  royal: [pillar, crown, banners, grove],
  candy: [lollipop, cane, donut, gumdrops],
};

/** The land far below the course, in the look's colours. */
function ground(b: Builder, look: ResolvedLook, all: Box) {
  const gr = look.ground;
  if (!gr) return;
  const mat = patternMaterial(gr.c1, gr.c2, gr.freq, [1, 0.6], gr.speed, gr.glow ? 'glossy' : 'padded', gr.kind);
  let m = mat;
  if (gr.glow && mat instanceof THREE.MeshStandardMaterial) {
    const lit = mat.clone();
    lit.emissive.set(gr.c1).multiplyScalar(0.8);
    delete lit.userData.detail;
    applySurface(lit, 'glossy', {
      pattern: { c1: gr.c1, c2: gr.c2, freq: gr.freq, dir: [1, 0.6], speed: gr.speed, kind: gr.kind },
    });
    m = b.view!.own(lit);
  }
  const plane = new THREE.Mesh(b.view!.own(new THREE.CircleGeometry(900, 72).rotateX(-Math.PI / 2)), m);
  plane.position.set((all.min.x + all.max.x) / 2, all.min.y - 70, (all.min.z + all.max.z) / 2);
  plane.receiveShadow = false;
  plane.castShadow = false;
  plane.userData.cat = 'ground';
  plane.userData.dynamic = true;
  b.group.add(plane);
}

/** Places the look's pieces round the course (and the land below). */
export function decorate(b: Builder, all: Box, blocked: Blocked, rnd: () => number) {
  const look = b.look;
  ground(b, look, all);
  const set = SETS[look.id];
  if (!set.length) return;
  const kit = new Kit(b, look, rnd);
  const total = set.reduce((s, p) => s + (p.weight ?? 1), 0);
  const choose = () => {
    let x = rnd() * total;
    for (const p of set) if ((x -= p.weight ?? 1) <= 0) return p;
    return set[0]!;
  };
  const placed: { x: number; y: number; z: number; r: number }[] = [];
  const w = all.max.x - all.min.x;
  const d = all.max.z - all.min.z;
  // More for bigger maps (long races), within a budget.
  const count = Math.round(Math.min(26, 12 + (w + d) / 14));
  for (let n = 0; n < count; n++) {
    const piece = choose();
    const s = 0.8 + rnd() * 0.6;
    const r = piece.r * s + (piece.island ? 1 : 0);
    for (let tries = 0; tries < 30; tries++) {
      const p = new THREE.Vector3(
        all.min.x - 40 + rnd() * (w + 80),
        piece.island
          ? all.min.y - 24 + rnd() * (all.max.y - all.min.y + 20)
          : all.min.y - 16 + rnd() * (all.max.y - all.min.y + 26),
        all.min.z - 40 + rnd() * (d + 80),
      );
      // (An island's rock reaches 7 m below it.)
      const mid = piece.island ? p.y + (piece.h * s - 7) / 2 : p.y + (piece.h * s) / 2;
      const half = piece.island ? (piece.h * s + 7) / 2 : (piece.h * s) / 2;
      if (blocked(new THREE.Vector3(p.x, mid, p.z), r + 2, half, 8)) continue;
      if (placed.some((q) => Math.hypot(q.x - p.x, q.z - p.z) < q.r + r + 2 && Math.abs(q.y - p.y) < 12)) continue;
      placed.push({ x: p.x, y: p.y, z: p.z, r });
      const made = piece.make(kit);
      const root = new THREE.Group();
      root.position.copy(p);
      root.userData.cat = 'decor';
      if (piece.island) {
        const isl = island(b, look);
        isl.scale.setScalar(s * 0.85);
        root.add(isl);
        made.obj.position.y = 0.4 * s * 0.85;
      }
      made.obj.scale.multiplyScalar(s);
      if (!piece.island) made.obj.rotation.y += rnd() * 6.3;
      root.add(made.obj);
      // Far enough from the course that their shadows would only cost: none.
      root.traverse((o) => {
        o.castShadow = false;
      });
      b.group.add(root);
      if (made.tick) b.anim(made.tick);
      break;
    }
  }
}
