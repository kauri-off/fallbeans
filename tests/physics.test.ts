import * as THREE from 'three';
import { describe, expect, it } from 'vitest';
import { mulberry32 } from '../src/shared/rng';
import { Builder } from '../src/sim/builder';
import { type Collider, PlayerBody } from '../src/sim/physics';

describe('collider grid', () => {
  it('finds every collider a brute-force search finds', () => {
    const rng = mulberry32(5);
    const b = new Builder(1, null);
    for (let i = 0; i < 300; i++) {
      const x = (rng() - 0.5) * 120;
      const z = (rng() - 0.5) * 120;
      if (i % 3 === 0) b.cyl(x, rng() * 5, z, 0.5 + rng() * 3, 1 + rng() * 2);
      else b.box(x, rng() * 5, z, 0.5 + rng() * 10, 1, 0.5 + rng() * 10, undefined, { rot: [0, rng() * 6, rng() * 0.4] });
    }
    b.world.finalize(0);
    const out: Collider[] = [];
    for (let k = 0; k < 200; k++) {
      const x = (rng() - 0.5) * 130;
      const z = (rng() - 0.5) * 130;
      const r = 1.7;
      const got = new Set(b.world.query(x, z, r, out));
      for (const c of b.world.colliders) {
        const [ex, ez] = c.extentXZ();
        const overlaps = Math.abs(c.center.x - x) <= ex + r && Math.abs(c.center.z - z) <= ez + r;
        if (overlaps) expect(got.has(c)).toBe(true);
      }
    }
  });

  it('lands on the ground, walks, jumps and falls off edges', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 10, 2, 10);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    body.reset(new THREE.Vector3(0, 2, 0));
    const idle = { mx: 0, mz: 0, jump: false, dive: false };
    for (let i = 0; i < 120; i++) body.step(1 / 120, idle, b.world, i / 120);
    expect(body.grounded).toBe(true);
    expect(body.pos.y).toBeCloseTo(0, 2);
    body.step(1 / 120, { ...idle, jump: true }, b.world, 1);
    expect(body.jumped).toBe(true);
    for (let i = 0; i < 240; i++) body.step(1 / 120, { ...idle, mz: 1 }, b.world, 1 + i / 120);
    expect(body.pos.z).toBeGreaterThan(6);
    expect(body.pos.y).toBeLessThan(-1);
  });

  it('round-trips the full body state', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 10, 2, 10);
    b.world.finalize(0);
    const a = new PlayerBody(1);
    a.reset(new THREE.Vector3(0, 0.5, 0));
    for (let i = 0; i < 60; i++) a.step(1 / 120, { mx: 0.5, mz: 1, jump: i === 30, dive: false }, b.world, i / 120);
    const c = new PlayerBody(1);
    c.fromFull(a.toFull(), b.world);
    for (let i = 60; i < 120; i++) {
      const inp = { mx: 1, mz: 0, jump: false, dive: i === 90 };
      a.step(1 / 120, inp, b.world, i / 120);
      c.step(1 / 120, inp, b.world, i / 120);
    }
    expect(c.pos.distanceTo(a.pos)).toBeLessThan(1e-9);
  });
});
