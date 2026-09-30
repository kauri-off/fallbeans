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

  it('does not pass through a thin wall at any speed (collision substeps)', () => {
    const b = new Builder(1, null);
    b.box(3, 0, 0, 0.1, 10, 10);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    // 1.25 m a tick, from where whole ticks would land at x = 2.40 and 3.65: either side of the wall.
    body.reset(new THREE.Vector3(-0.05, 1, 0));
    body.vel.set(150, 0, 0);
    const idle = { mx: 0, mz: 0, jump: false, dive: false };
    for (let i = 0; i < 12; i++) body.step(1 / 120, idle, b.world, i / 120);
    expect(body.pos.x).toBeLessThan(3);
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

  it('falls straight through a fake pane, keeping its speed and reporting the touch', () => {
    const b = new Builder(1, null);
    let touched = 0;
    b.box(0, -0.15, 0, 3, 0.3, 3, undefined, { trigger: true, onTouch: () => touched++ });
    b.world.finalize(0);
    const body = new PlayerBody(1);
    body.reset(new THREE.Vector3(0, 2, 0));
    body.vel.set(0, -6, 4);
    run(body, b.world, 0, 40, { ...idle, mz: 1, jump: true });
    expect(touched).toBeGreaterThan(0);
    expect(body.jumped).toBe(false);
    expect(body.pos.y).toBeLessThan(-0.5);
    expect(body.vel.y).toBeLessThan(-6);
    expect(body.vel.z).toBeGreaterThan(3);
  });

  /** Deepest overlap of the body's spheres with sweeping arms right now. */
  const armOverlap = (body: PlayerBody, world: Builder['world']) => {
    const c = new THREE.Vector3();
    const hit = { local: new THREE.Vector3(), point: new THREE.Vector3(), normal: new THREE.Vector3(), depth: 0 };
    let worst = 0;
    for (const col of world.colliders)
      for (const i of [0, 1]) if (col.sweep && col.contact(body.sphere(i, c), 0.5, hit)) worst = Math.max(worst, hit.depth);
    return worst;
  };

  it('a sweeping arm knocks a bean over, and never passes through it', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 40, 2, 40);
    b.rotor(0, 0.6, 0, 8, 1, (t) => t * 1.2);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    // A quarter turn ahead of the arm (which starts along +x and turns towards −z).
    body.reset(new THREE.Vector3(0.5, 0.02, -5));
    let knocked = false;
    let maxDrag = 0;
    let overlap = 0;
    const start = body.pos.clone();
    for (let i = 1; i <= 360; i++) {
      run(body, b.world, (i - 1) / 120, 1);
      if (body.knocked) knocked = true;
      maxDrag = Math.max(maxDrag, body.pos.distanceTo(start));
      overlap = Math.max(overlap, armOverlap(body, b.world));
    }
    expect(knocked).toBe(true);
    // Shoved a few metres, not carried round with the arm.
    expect(maxDrag).toBeLessThan(8);
    expect(overlap).toBeLessThan(0.35);
  });

  it('a sweeping arm tosses a bean lying in its way up and over itself', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 40, 2, 40);
    b.rotor(0, 0.6, 0, 8, 1, (t) => t * 1.2);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    body.reset(new THREE.Vector3(0.5, 0.02, -5));
    body.knock(0.2, 0, 0, 3);
    let top = 0;
    let overlap = 0;
    let maxDrag = 0;
    const start = body.pos.clone();
    for (let i = 1; i <= 240; i++) {
      run(body, b.world, (i - 1) / 120, 1);
      if (i > 30) overlap = Math.max(overlap, armOverlap(body, b.world));
      top = Math.max(top, body.pos.y);
      maxDrag = Math.max(maxDrag, Math.hypot(body.pos.x - start.x, body.pos.z - start.z));
    }
    expect(top).toBeGreaterThan(0.7);
    expect(overlap).toBeLessThan(0.35);
    expect(maxDrag).toBeLessThan(6);
  });

  it('a sweeping arm is no ride: a bean on top of it drops off behind it', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 40, 2, 40);
    b.rotor(0, 0.6, 0, 8, 1, (t) => t * 1.2);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    // On top of the arm (along +x at t = 0; its top is at 0.96).
    body.reset(new THREE.Vector3(5, 1, 0));
    run(body, b.world, 0, 120);
    // Carried along, it would be 6 m round the circle by now.
    expect(Math.hypot(body.pos.x - 5, body.pos.z)).toBeLessThan(1.5);
    expect(body.pos.y).toBeLessThan(0.1);
  });

  it('running into the back of an arm that is moving away does not knock the bean over', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 40, 2, 40);
    b.rotor(0, 0.6, 0, 8, 1, (t) => t * 0.5);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    // Behind the arm (it turns towards −z), running after it.
    body.reset(new THREE.Vector3(3, 0.02, 1.5));
    let knocked = false;
    for (let i = 1; i <= 96; i++) {
      run(body, b.world, (i - 1) / 120, 1, { ...idle, mz: -1 });
      if (body.knocked) knocked = true;
    }
    expect(knocked).toBe(false);
    expect(body.state).toBe('normal');
  });

  it('a dive lays the body along its flight, and into a wall it stays out of it', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 20, 2, 40);
    b.box(0, 2, 6, 20, 4, 1);
    b.world.finalize(0);
    const body = new PlayerBody(1);
    body.reset(new THREE.Vector3(0, 0.02, 0));
    run(body, b.world, 0, 20, { ...idle, mz: 1 });
    run(body, b.world, 20 / 120, 1, { ...idle, mz: 1, dive: true });
    run(body, b.world, 21 / 120, 12, { ...idle, mz: 1 });
    expect(body.state).toBe('dive');
    expect(body.tilt).toBeGreaterThan(0.8);
    const head = new THREE.Vector3();
    let deepest = -9;
    for (let i = 0; i < 120; i++) {
      run(body, b.world, (33 + i) / 120, 1, { ...idle, mz: 1 });
      deepest = Math.max(deepest, body.sphere(1, head).z + 0.5 - 5.5, body.pos.z + 0.5 - 5.5);
    }
    expect(deepest).toBeLessThan(0.02);
  });

  const ledgeCourse = (height: number) => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 20, 2, 40);
    b.box(0, height / 2, 7, 20, height, 6);
    b.world.finalize(0);
    return b;
  };
  /** Runs at the block and jumps a little before it. */
  const jumpAt = (body: PlayerBody, world: Builder['world'], hold = { ...idle, mz: 1 }, phases: boolean[] = []) => {
    body.reset(new THREE.Vector3(0, 0.02, 0));
    let t = 0;
    const step = (n: number, input: typeof idle) => {
      run(body, world, t, n, input);
      t += n / 120;
    };
    step(24, { ...idle, mz: 1 });
    step(1, { ...idle, mz: 1, jump: true });
    let climbed = false;
    for (let i = 0; i < 360; i++) {
      step(1, hold);
      if (body.state === 'climb') {
        climbed = true;
        phases.push(body.climbingOver);
      } else if (climbed) break;
    }
    // Settle where it got to.
    step(30, idle);
    return climbed;
  };

  it('catches a ledge out of reach of a jump and climbs onto it', () => {
    const b = ledgeCourse(2.6);
    const body = new PlayerBody(1);
    const phases: boolean[] = [];
    const climbed = jumpAt(body, b.world, undefined, phases);
    expect(climbed).toBe(true);
    // Pulling up, then over the edge: one change of phase (what others see as the pose).
    expect(phases[0]).toBe(false);
    expect(phases.at(-1)).toBe(true);
    expect(phases.filter((p, i) => i > 0 && p !== phases[i - 1]).length).toBe(1);
    expect(body.state).toBe('normal');
    expect(body.pos.y).toBeCloseTo(2.6, 1);
    expect(body.pos.z).toBeGreaterThan(4.3);
  });

  it('does not catch a ledge that is too high, or when not pushing towards it', () => {
    const high = ledgeCourse(4.2);
    const a = new PlayerBody(1);
    expect(jumpAt(a, high.world)).toBe(false);
    expect(a.pos.y).toBeLessThan(0.1);
    const low = ledgeCourse(2.6);
    const c = new PlayerBody(1);
    expect(jumpAt(c, low.world, idle)).toBe(false);
  });

  it('carries on climbing from a full state exactly', () => {
    const b = ledgeCourse(2.6);
    const a = new PlayerBody(1);
    a.reset(new THREE.Vector3(0, 0.02, 0));
    let t = 0;
    const go = { ...idle, mz: 1 };
    for (let i = 0; i < 300 && a.state !== 'climb'; i++, t += 1 / 120) run(a, b.world, t, 1, { ...go, jump: i === 24 });
    expect(a.state).toBe('climb');
    const c = new PlayerBody(1);
    c.fromFull(a.toFull(), b.world);
    for (let i = 0; i < 120; i++, t += 1 / 120) {
      run(a, b.world, t, 1, go);
      run(c, b.world, t, 1, go);
    }
    expect(c.pos.distanceTo(a.pos)).toBeLessThan(1e-6);
  });

  it('portals send a bean out of the other end, facing its way', () => {
    const b = new Builder(1, null);
    b.box(0, -1, 0, 60, 2, 60);
    b.portal({ x: 0, y: 0, z: 5, yaw: Math.PI }, { x: 20, y: 0, z: 0, yaw: Math.PI / 2 });
    b.world.finalize(0);
    const body = new PlayerBody(1);
    body.reset(new THREE.Vector3(0, 0.02, 0));
    // In, half a second inside (out of play), then out at the other end.
    let out = false;
    for (let i = 0; i < 240 && !out; i++) {
      run(body, b.world, i / 120, 1, { ...idle, mz: 1 });
      out = body.portalOut;
    }
    expect(out).toBe(true);
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
