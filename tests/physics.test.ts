import * as THREE from 'three';
import { describe, expect, it } from 'vitest';
import { mulberry32 } from '../src/shared/rng';
import { Builder } from '../src/sim/builder';
import { type Collider, GIANT_MASS, GIANT_SIZE, PlayerBody, POWER } from '../src/sim/physics';

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

describe('course mechanics', () => {
  const idle = { mx: 0, mz: 0, jump: false, dive: false };
  const run = (body: PlayerBody, world: Builder['world'], t0: number, n: number, input = idle) => {
    for (let i = 1; i <= n; i++) {
      const t = t0 + i / 120;
      body.clearEvents();
      body.beforeWorldUpdate();
      world.setTime(t);
      body.afterWorldUpdate();
      body.step(1 / 120, input, world, t);
    }
  };

  it('allows no jump (and no dive boost) off crumbling ground', () => {
    const b = new Builder(1, null);
    b.box(0, -0.5, 0, 6, 1, 6, undefined, { crumbly: true });
    b.box(20, -0.5, 0, 6, 1, 6);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    body.reset(new THREE.Vector3(0, 0.02, 0));
    run(body, b.world, 0, 30);
    run(body, b.world, 0.25, 1, { ...idle, jump: true });
    expect(body.jumped).toBe(false);
    run(body, b.world, 0.26, 1, { ...idle, mz: 1, dive: true });
    expect(body.vel.y).toBeLessThanOrEqual(0);
    // On solid ground it works as always.
    body.reset(new THREE.Vector3(20, 0.02, 0));
    run(body, b.world, 1, 30);
    run(body, b.world, 1.25, 1, { ...idle, jump: true });
    expect(body.jumped).toBe(true);
  });

  it('a sweeping arm always knocks a bean over, then passes over it', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 40, 2, 40);
    b.rotor(0, 0.6, 0, 8, 1, (t) => t * 1.2);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    // A quarter turn ahead of the arm (which starts along +x and turns towards −z).
    body.reset(new THREE.Vector3(0.5, 0.02, -5));
    let knocked = false;
    let maxDrag = 0;
    const start = body.pos.clone();
    for (let i = 1; i <= 360; i++) {
      run(body, b.world, (i - 1) / 120, 1);
      if (body.knocked) knocked = true;
      maxDrag = Math.max(maxDrag, body.pos.distanceTo(start));
    }
    expect(knocked).toBe(true);
    // Shoved a few metres, not carried round with the arm.
    expect(maxDrag).toBeLessThan(8);
  });

  it('portals send a bean out of the other end, facing its way', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 60, 2, 60);
    b.portal({ x: 0, y: 0, z: 5, yaw: Math.PI }, { x: 20, y: 0, z: 0, yaw: Math.PI / 2 });
    b.world.finalize(0);
    const body = new PlayerBody(1);
    body.reset(new THREE.Vector3(0, 0.02, 0));
    let warped = false;
    for (let i = 0; i < 120 && !warped; i++) {
      run(body, b.world, i / 120, 1, { ...idle, mz: 1 });
      warped = body.warped;
    }
    expect(warped).toBe(true);
    expect(body.pos.x).toBeGreaterThan(21);
    expect(body.vel.x).toBeGreaterThan(5);
  });

  it('a giant is bigger, heavier and shrugs off knocks', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 20, 2, 20);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    body.reset(new THREE.Vector3(0, 0.02, 0));
    body.givePower(POWER.giant, 0);
    run(body, b.world, 0, 10);
    expect(body.size).toBe(GIANT_SIZE);
    expect(body.mass).toBe(GIANT_MASS);
    body.knock(10, 0, 5, 1);
    expect(body.state).not.toBe('tumble');
    expect(body.vel.x).toBeLessThan(4);
    // It wears off.
    run(body, b.world, 0.1, 120 * 10);
    expect(body.size).toBe(1);
  });
});
