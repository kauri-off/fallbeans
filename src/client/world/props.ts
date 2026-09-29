import * as THREE from 'three';
import { type Builder, PAL, type Palette } from './builder';

export interface BallLaneOpts {
  lanes: readonly number[];
  zTop: number;
  yTop: number;
  zBottom: number;
  yBottom: number;
  radius: number;
  speed: (t: number) => number;
  period: number;
  perLane?: number;
  pal?: Palette;
}

export function rollingBalls(b: Builder, o: BallLaneOpts) {
  const len = o.zTop - o.zBottom;
  const slope = (o.yTop - o.yBottom) / len;
  const cosA = Math.cos(Math.atan(slope));
  const perLane = o.perLane ?? 1;
  const palettes = [PAL.pink, PAL.orange, PAL.purple, PAL.red];
  o.lanes.forEach((x, li) => {
    for (let k = 0; k < perLane; k++) {
      const phase = b.rng() * o.period + (k * o.period) / perLane;
      const ball = b.sphere(x, o.yTop + o.radius, o.zTop, o.radius, o.pal ?? palettes[(li + k) % palettes.length]!, {
        dynamic: true,
        hit: 1.1,
      });
      const mesh = ball.mesh;
      mesh.visible = false;
      ball.col.enabled = false;
      let dist = 0;
      let lastCycle = -1;
      b.update((t) => {
        const tt = Math.max(0, t) + phase;
        const cycle = Math.floor(tt / o.period);
        const s = tt - cycle * o.period;
        const v = o.speed(t);
        dist = s * v;
        const alive = t > 0 && dist < len;
        if (cycle !== lastCycle) {
          lastCycle = cycle;
          ball.col.enabled = false;
        } else ball.col.enabled = alive && s > 0.1;
        mesh.visible = alive;
        if (!alive) return;
        const z = o.zTop - dist;
        const grow = Math.min(1, s / 0.3) * Math.min(1, (len - dist) / 1.5);
        mesh.scale.setScalar(Math.max(0.01, grow));
        mesh.position.set(x, o.yBottom + (z - o.zBottom) * slope + o.radius / cosA, z);
        mesh.rotation.x = -dist / o.radius;
      });
    }
  });
}

export function emojiTexture(emoji: string, bg: string, size = 256): THREE.CanvasTexture {
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const g = c.getContext('2d')!;
  g.fillStyle = bg;
  g.fillRect(0, 0, size, size);
  g.strokeStyle = 'rgba(255,255,255,0.8)';
  g.lineWidth = size * 0.04;
  g.strokeRect(size * 0.04, size * 0.04, size * 0.92, size * 0.92);
  g.font = `${Math.floor(size * 0.62)}px "Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji", sans-serif`;
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  g.fillText(emoji, size / 2, size * 0.54);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = 4;
  return tex;
}

export function yOnRamp(z: number, z0: number, y0: number, z1: number, y1: number) {
  return y0 + ((z - z0) / (z1 - z0)) * (y1 - y0);
}
