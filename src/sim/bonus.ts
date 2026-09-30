import * as THREE from 'three';
import { z } from 'zod';
import { mulberry32 } from '../shared/rng';
import type { Builder } from './builder';
import { type PlayerBody, POWER } from './physics';

/**
 * Rare one-off bonuses lying on the course (a spot or two per round, picked from the map's
 * candidate spots by the seed): run through one to become a giant (bigger, four times as heavy),
 * get a mega jump or a burst of speed for a few seconds. The server decides who took it; the
 * choice of spots and kinds follows from the seed, identical on every client.
 */

export const BONUS_EVENT = '@bonus';
const BonusEvent = z.object({ i: z.number().int().min(0).max(63), id: z.number().int(), at: z.number() });

export interface BonusInfo {
  kind: number;
  icon: string;
  title: string;
  color: string;
}

export const BONUS_KINDS: Record<number, BonusInfo> = {
  [POWER.giant]: { kind: POWER.giant, icon: '🍄', title: 'Великан', color: '#ff6f91' },
  [POWER.jump]: { kind: POWER.jump, icon: '🦘', title: 'Мега-прыжок', color: '#58d68d' },
  [POWER.speed]: { kind: POWER.speed, icon: '⚡', title: 'Ускорение', color: '#ffd23f' },
};

export interface Bonus {
  i: number;
  x: number;
  y: number;
  z: number;
  kind: number;
  /** Sim time it shows up (0 on race courses). */
  appearAt: number;
  takenBy: number | null;
  takenAt: number;
}

const REACH = 1.25;

export class Bonuses {
  readonly list: Bonus[] = [];
  private readonly objs: THREE.Object3D[] = [];

  constructor(b: Builder, seed: number, o: { arena: boolean; duration: number }) {
    const spots = b.bonusSpots;
    if (!spots.length) return;
    const rng = mulberry32((seed ^ 0x5bd1e995) >>> 0);
    const roll = rng();
    // Rare: none in about one round in six, otherwise one, sometimes two.
    const count = Math.min(spots.length, roll < 0.16 ? 0 : roll < 0.7 ? 1 : 2);
    const pool = [...spots.keys()];
    const kinds = [POWER.giant, POWER.jump, POWER.speed];
    for (let k = 0; k < count; k++) {
      const pick = pool.splice(Math.floor(rng() * pool.length), 1)[0]!;
      const s = spots[pick]!;
      const kind = kinds[Math.floor(rng() * kinds.length)]!;
      const appearAt = o.arena ? 12 + rng() * Math.max(1, o.duration * 0.55 - 12) : 0;
      this.list.push({ i: k, x: s.x, y: s.y, z: s.z, kind, appearAt, takenBy: null, takenAt: 0 });
    }
    if (b.view) this.build(b);
  }

  /** Bonuses lying on the course at time t (bots look for them). */
  available(t: number): Bonus[] {
    return this.list.filter((x) => x.takenBy === null && t >= x.appearAt);
  }

  /** Server: bodies that touch a bonus take it. Calls `emit` with the event to broadcast. */
  check(t: number, bodies: Iterable<PlayerBody>, emit: (name: string, data: unknown) => void) {
    for (const x of this.list) {
      if (x.takenBy !== null || t < x.appearAt) continue;
      for (const body of bodies) {
        const dy = body.pos.y - x.y;
        if (dy < -0.8 || dy > 2) continue;
        if (Math.hypot(body.pos.x - x.x, body.pos.z - x.z) > REACH + 0.3 * body.size) continue;
        body.givePower(x.kind, t);
        emit(BONUS_EVENT, { i: x.i, id: body.actor, at: t });
        break;
      }
    }
  }

  /** Both sides: a bonus was taken (the server has already applied it to the body). */
  onEvent(data: unknown): Bonus | null {
    const d = BonusEvent.safeParse(data);
    if (!d.success) return null;
    const x = this.list[d.data.i];
    if (!x || x.takenBy !== null) return null;
    x.takenBy = d.data.id;
    x.takenAt = d.data.at;
    return x;
  }

  private build(b: Builder) {
    const v = b.view!;
    for (const x of this.list) {
      const info = BONUS_KINDS[x.kind]!;
      const g = new THREE.Group();
      g.position.set(x.x, x.y, x.z);
      g.userData.cat = 'decor';
      const bubble = new THREE.Mesh(
        v.own(new THREE.SphereGeometry(0.62, 32, 20)),
        v.own(
          new THREE.MeshStandardMaterial({
            color: info.color,
            emissive: new THREE.Color(info.color),
            emissiveIntensity: 0.45,
            transparent: true,
            opacity: 0.45,
            roughness: 0.15,
            depthWrite: false,
          }),
        ),
      );
      bubble.position.y = 1.05;
      bubble.name = 'bubble';
      const card = new THREE.Mesh(
        v.own(new THREE.CircleGeometry(0.42, 32)),
        v.own(
          new THREE.MeshBasicMaterial({
            map: v.emojiTexture(info.icon, info.color),
            side: THREE.DoubleSide,
            toneMapped: false,
          }),
        ),
      );
      card.position.y = 1.05;
      card.name = 'card';
      const ring = new THREE.Mesh(
        v.own(new THREE.RingGeometry(0.75, 1.0, 40)),
        v.own(
          new THREE.MeshBasicMaterial({
            color: info.color,
            transparent: true,
            opacity: 0.6,
            side: THREE.DoubleSide,
            depthWrite: false,
            toneMapped: false,
          }),
        ),
      );
      ring.rotation.x = -Math.PI / 2;
      ring.position.y = 0.04;
      ring.name = 'ring';
      g.add(bubble, card, ring);
      g.visible = false;
      b.group.add(g);
      this.objs[x.i] = g;
    }
    b.anim((t) => {
      for (const x of this.list) {
        const g = this.objs[x.i];
        if (!g) continue;
        const shown = t >= x.appearAt && (x.takenBy === null || t < x.takenAt + 0.35);
        g.visible = shown;
        if (!shown) continue;
        // Pops in, bobs and turns; taken: swells and vanishes.
        const grow = Math.min(1, (t - x.appearAt) / 0.5 + (x.appearAt <= 0 ? 1 : 0));
        const gone = x.takenBy === null ? 0 : (t - x.takenAt) / 0.35;
        const s = Math.max(0.01, grow * (1 + gone * 0.8) * (1 - gone));
        const bob = Math.sin(t * 2.4 + x.i) * 0.15;
        for (const c of g.children) {
          if (c.name === 'ring') {
            c.scale.setScalar(1 + Math.sin(t * 3 + x.i) * 0.08);
            continue;
          }
          c.scale.setScalar(s);
          c.position.y = 1.05 + bob;
        }
        const card = g.getObjectByName('card');
        if (card) card.rotation.y = t * 1.8;
      }
    });
  }
}
