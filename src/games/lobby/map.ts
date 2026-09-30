import * as THREE from 'three';
import { defineGame } from '../../shared/game';
import { arenaBrain } from '../../sim/bots';
import { type Builder, type ModelName, PAL, type Palette } from '../../sim/builder';
import { defineMap } from '../../sim/map';

/** Not a game: the playground players run around in between shows. */
export const LOBBY_META = defineGame({
  id: 'lobby',
  title: 'Лобби',
  genre: 'points',
  desc: '',
  goal: '',
  duration: 1e6,
});

/**
 * Layout (metres; y up, the plaza in the middle, every zone clear of the others):
 *   plaza        r < 7.5 around the fountain: the spawn ring, nothing that moves
 *   north        the bell tower: a spiral of pillars up to the top, an icy slide down to the east
 *   east         trampolines, and a high platform with a portal back down
 *   south-east   the ice rink with bumpers
 *   south-west   blocks to climb, the other end of the portal
 *   west         the spinner: a raised disc swept by a slow rotor
 *   north-west   a launch pad
 * Planters with trees ring the edge wherever no zone reaches it.
 */
const FLOOR_R = 24;
const TOWER = { x: 0, z: 16, top: 6, half: 2 };
/** Ringing the bell: standing up here after having been down on the floor since the last ring. */
const BELL = { x: TOWER.x, z: TOWER.z, r: 1.7, y: TOWER.top - 0.3 };
/** Height of the bell's beam over the tower top (the bell's rim is 1.25 m lower: 2.75 m up). */
const BELL_HANG = 4;
const TRAMPS = [
  { x: 12.5, z: 0 },
  { x: 16.5, z: -3 },
  { x: 16.5, z: 3.5 },
];
const PLATFORM = { x: 21, z: 0.25, top: 6 };
const SPINNER = { x: -14.5, z: 0, r: 6 };
const RINK = { x: 11, z: -12.5, r: 5.5 };
const BLOCKS = { x: -13, z: -12, step: 2.6 };
const PAD = { x: -10, z: 10 };
const PORTAL_B = { x: -6.5, z: -10 };

/** Circles the edge planters keep away from. */
const ZONES = [
  { x: PLATFORM.x, z: PLATFORM.z, r: 3.2 },
  { x: RINK.x, z: RINK.z, r: RINK.r },
  { x: BLOCKS.x, z: BLOCKS.z, r: 5.3 },
  { x: SPINNER.x, z: SPINNER.z, r: SPINNER.r },
  // The foot of the slide, with its flags.
  { x: TOWER.x + 12, z: TOWER.z, r: 3 },
];

/** A slab from (x0, y0) down to (x1, y1) along x, its top surface on that line. */
function slide(b: Builder, x0: number, y0: number, x1: number, y1: number, z: number, width: number, material?: THREE.Material) {
  const ang = Math.atan2(y1 - y0, x1 - x0);
  const len = Math.hypot(x1 - x0, y1 - y0);
  const thick = 0.5;
  return b.box((x0 + x1) / 2, (y0 + y1) / 2 - thick / 2 / Math.cos(ang), z, len, thick, width, PAL.teal, {
    material,
    slip: 1,
    rot: [0, 0, ang],
  });
}

/** A painted path on the floor from (x0, z0) to (x1, z1) (no collision). */
function path(b: Builder, x0: number, z0: number, x1: number, z1: number, pal: Palette) {
  const len = Math.hypot(x1 - x0, z1 - z0);
  b.box((x0 + x1) / 2, 0.03, (z0 + z1) / 2, 2.2, 0.06, len, pal, {
    noCollide: true,
    castShadow: false,
    pattern: 'chevron',
    rot: [0, Math.atan2(x1 - x0, z1 - z0), 0],
  });
}

/**
 * A decorative model that is also solid (props alone are pictures: beans would run through them).
 * The colliders follow the models' shapes (public/models, see `bun run assets`); mushroom caps bounce.
 */
function solid(
  b: Builder,
  name: ModelName,
  x: number,
  y: number,
  z: number,
  o: { yaw?: number; scale?: number; tint?: string } = {},
) {
  b.prop(name, x, y, z, o);
  const s = o.scale ?? 1;
  const cyl = (r: number, h: number, cy: number, pad?: number) =>
    b.collider(
      b.anchor(x, y + cy * s, z),
      { type: 'cyl', r: r * s, hh: (h / 2) * s },
      { isStatic: true, ...(pad ? { pad } : {}) },
    );
  const ball = (r: number, cy: number) =>
    b.collider(b.anchor(x, y + cy * s, z), { type: 'sphere', r: r * s }, { isStatic: true });
  switch (name) {
    case 'tree':
      cyl(0.34, 3.2, 1.6);
      ball(1.35, 3.7);
      break;
    case 'pine':
      cyl(0.25, 1.4, 0.7);
      cyl(1.3, 1.8, 1.8);
      cyl(1, 1.6, 2.8);
      cyl(0.68, 1.4, 3.7);
      break;
    case 'mushroom':
      cyl(0.42, 1.3, 0.65);
      cyl(1, 0.8, 1.58, 13);
      break;
    case 'cone':
      cyl(0.3, 0.88, 0.44);
      break;
    case 'flag':
      cyl(0.1, 4.6, 2.3);
      break;
    default:
      throw new Error(`no collider shape for ${name}`);
  }
}

/** A signpost with a board showing what the zone is for (solid, both). */
function sign(b: Builder, x: number, z: number, emoji: string, bg: string) {
  // Turned to the plaza, where people come from.
  const yaw = Math.atan2(-x, -z);
  b.box(x, 1.25, z, 0.22, 2.5, 0.22, '#8a6a4f', { surface: 'wood' });
  b.box(x, 3.05, z, 1.5, 1.5, 0.1, '#8a6a4f', { surface: 'wood', rot: [0, yaw, 0] });
  const v = b.view;
  if (!v) return;
  const geo = v.own(new THREE.PlaneGeometry(1.4, 1.4));
  const mat = v.own(new THREE.MeshStandardMaterial({ map: v.own(v.emojiTexture(emoji, bg)), roughness: 0.6 }));
  for (const side of [1, -1]) {
    const face = new THREE.Mesh(geo, mat);
    face.position.set(x + Math.sin(yaw) * 0.06 * side, 3.05, z + Math.cos(yaw) * 0.06 * side);
    face.rotation.y = side > 0 ? yaw : yaw + Math.PI;
    b.group.add(face);
  }
}

export default defineMap(LOBBY_META, (b, ctx) => {
  b.style.pattern = 'dots';

  // ---------------------------------------------------------------- ground and plaza
  b.cyl(0, -1, 0, FLOOR_R, 2, PAL.blue, { freq: 0.3 });
  b.cyl(0, 0.05, 0, 7.5, 0.1, PAL.yellow, { noCollide: true, castShadow: false, pattern: 'waves' });
  // Fountain: a basin with water.
  b.cyl(0, 0.35, 0, 1.6, 0.7, PAL.white, { surface: 'tile' });
  const water = b.view?.pattern('#6fd6ff', '#b8ecff', 1.6, [1, 0.4], 0.35, 'glossy', 'waves');
  b.cyl(0, 0.72, 0, 1.35, 0.06, PAL.teal, { noCollide: true, material: water, castShadow: false });
  // Flags on little posts round the plaza.
  const flagCols = ['#ff5fa2', '#3fa9ff', '#ffd23f', '#4fdc6a'];
  for (let k = 0; k < 4; k++) {
    // (Between the paths that lead out of the plaza.)
    const a = Math.PI / 8 + (k * Math.PI) / 2;
    const x = Math.cos(a) * 7.9;
    const z = Math.sin(a) * 7.9;
    b.cyl(x, 0.15, z, 0.35, 0.3, PAL.purple, { seg: 16 });
    solid(b, 'flag', x, 0.3, z, { tint: flagCols[k]!, yaw: -a });
  }

  // ---------------------------------------------------------------- the bell tower (north)
  b.box(TOWER.x, TOWER.top / 2, TOWER.z, TOWER.half * 2, TOWER.top, TOWER.half * 2, PAL.purple, { pattern: 'stripes' });
  // Pillars spiral up round the west side: 1 m higher each, the last one a jump from the top.
  const pillarPals = [PAL.green, PAL.yellow, PAL.orange, PAL.pink, PAL.red];
  for (let k = 0; k < 5; k++) {
    const a = -Math.PI / 2 - k * 0.62;
    const h = k + 1;
    b.cyl(TOWER.x + Math.cos(a) * 5, h / 2, TOWER.z + Math.sin(a) * 5, 1.1, h, pillarPals[k]!, { seg: 32 });
  }
  // The bell hangs in a frame on top (the posts stand at the edge, out of the way), high enough
  // to walk under: its rim is a jump above a bean's head.
  for (const sx of [-1, 1])
    b.box(TOWER.x + sx * 1.75, TOWER.top + BELL_HANG / 2, TOWER.z, 0.3, BELL_HANG, 0.3, PAL.white, { surface: 'metal' });
  b.box(TOWER.x, TOWER.top + BELL_HANG + 0.15, TOWER.z, 3.8, 0.3, 0.3, PAL.white, { surface: 'metal' });
  // (The bell swings a little: a still collider round where it hangs.)
  b.collider(b.anchor(TOWER.x, TOWER.top + BELL_HANG - 0.85, TOWER.z), { type: 'cyl', r: 0.75, hh: 0.8 }, { isStatic: true });
  if (b.view) {
    const bell = b.anchor(TOWER.x, TOWER.top + BELL_HANG, TOWER.z);
    const gold = b.view.plain('#ffcf3f', { roughness: 0.25, metalness: 0.8 }, 'gold');
    b.cyl(0, -0.75, 0, 0.75, 1, '#ffcf3f', { noCollide: true, material: gold, parent: bell, seg: 32 });
    b.sphere(0, -0.3, 0, 0.5, '#ffcf3f', { noCollide: true, material: gold, parent: bell });
    b.sphere(0, -1.35, 0, 0.18, '#8a6a4f', { noCollide: true, parent: bell });
    b.anim((t) => {
      bell.rotation.x = Math.sin(t * 1.7) * 0.12;
    });
  }
  // The way down: an icy slide from the top to the east.
  const ice = b.view?.plain('#d6f2ff', { roughness: 0.08, metalness: 0.05 }, 'ice');
  slide(b, TOWER.x + TOWER.half, TOWER.top, TOWER.x + 11.5, 0, TOWER.z, 3.2, ice);
  solid(b, 'flag', TOWER.x + 12.5, 0, TOWER.z - 2.2, { tint: '#39e0d0', yaw: Math.PI / 2 });
  solid(b, 'flag', TOWER.x + 12.5, 0, TOWER.z + 2.2, { tint: '#39e0d0', yaw: Math.PI / 2 });

  // ---------------------------------------------------------------- trampolines and the high platform (east)
  for (const t of TRAMPS) b.trampoline(t.x, 0, t.z, 1.8, 21);
  b.box(PLATFORM.x, PLATFORM.top - 0.4, PLATFORM.z, 4, 0.8, 4.5, PAL.orange);
  solid(b, 'flag', PLATFORM.x - 1.5, PLATFORM.top, PLATFORM.z - 1.8, { tint: '#ff8a3d', yaw: -Math.PI / 2 });
  // A portal from the platform down to the south-west, and back up.
  b.portal(
    { x: PLATFORM.x + 1.3, y: PLATFORM.top, z: PLATFORM.z, yaw: -Math.PI / 2 },
    { x: PORTAL_B.x, y: 0, z: PORTAL_B.z, yaw: Math.atan2(-PORTAL_B.x, -PORTAL_B.z) },
  );

  // ---------------------------------------------------------------- ice rink (south-east)
  b.cyl(RINK.x, 0.06, RINK.z, RINK.r, 0.12, PAL.white, { material: ice, slip: 1, surface: 'ice' });
  b.bumper(RINK.x - 1.6, 0.12, RINK.z - 1.4, 0.8, 11);
  b.bumper(RINK.x + 2, 0.12, RINK.z + 1.5, 0.8, 11);
  for (let k = 0; k < 10; k++) {
    const a = (k / 10) * Math.PI * 2;
    solid(b, 'cone', RINK.x + Math.cos(a) * (RINK.r + 0.6), 0, RINK.z + Math.sin(a) * (RINK.r + 0.6), { scale: 0.7 });
  }

  // ---------------------------------------------------------------- blocks to climb (south-west)
  const heights = [0.6, 1.2, 1.8, 1.2, 2.4, 3, 1.8, 3, 3.8];
  const blockPals = [PAL.green, PAL.teal, PAL.blue, PAL.teal, PAL.purple, PAL.pink, PAL.blue, PAL.pink, PAL.red];
  for (let i = 0; i < 9; i++) {
    // Low near the plaza, higher towards the edge.
    const x = BLOCKS.x + (1 - (i % 3)) * BLOCKS.step;
    const z = BLOCKS.z + (1 - Math.floor(i / 3)) * BLOCKS.step;
    const h = heights[i]!;
    b.box(x, h / 2, z, 2.2, h, 2.2, blockPals[i]!, { pattern: 'checker' });
  }

  // ---------------------------------------------------------------- the spinner (west)
  b.cyl(SPINNER.x, 0.25, SPINNER.z, SPINNER.r, 0.5, PAL.pink, { pattern: 'stripes' });
  b.hub(SPINNER.x, 0.5, SPINNER.z, 0.9);
  b.rotor(SPINNER.x, 1.1, SPINNER.z, 5.2, 2, (t) => t * 0.8, 0.6);

  // ---------------------------------------------------------------- launch pad (north-west)
  b.pad(PAD.x, 0, PAD.z, 1.3, 16);
  solid(b, 'mushroom', PAD.x - 3, 0, PAD.z + 2.5, { scale: 1.3 });
  solid(b, 'mushroom', PAD.x + 2.4, 0, PAD.z + 3.2, { scale: 0.9 });

  // ---------------------------------------------------------------- paths and signs from the plaza
  path(b, 0, 7.5, 0, 9.6, PAL.purple);
  path(b, 7.5, 0, 10.4, 0, PAL.orange);
  path(b, -7.5, 0, -8.5, 0, PAL.pink);
  path(b, 5.3, -5.3, 7.3, -8.2, PAL.white);
  path(b, -5.3, -5.3, -8.8, -8.8, PAL.green);
  sign(b, 2.6, 9.6, '🔔', '#a98bff');
  sign(b, 9.6, 2.6, '🤸', '#ff9f4a');
  sign(b, -7.6, 3, '🌀', '#ff8cc8');
  sign(b, 5, -7.6, '⛸️', '#9bdcff');
  sign(b, -8.6, -13.8, '🧗', '#6fe08a');
  sign(b, PAD.x + 2.2, PAD.z - 1.4, '🚀', '#39e0d0');

  // ---------------------------------------------------------------- planters round the edge
  const flora = ['tree', 'pine', 'tree', 'mushroom', 'pine'] as const;
  for (let k = 0; k < 20; k++) {
    const a = (k / 20) * Math.PI * 2;
    const x = Math.cos(a) * 22.8;
    const z = Math.sin(a) * 22.8;
    if (ZONES.some((q) => Math.hypot(x - q.x, z - q.z) < q.r + 1.2)) continue;
    b.cyl(x, 0.4, z, 0.9, 0.8, PAL.green, { surface: 'grass', seg: 24 });
    solid(b, flora[k % flora.length]!, x, 0.8, z, { scale: 0.8 + ((k * 37) % 5) * 0.1, yaw: k * 1.7 });
  }
  b.clouds(0, 0, 44);

  // ---------------------------------------------------------------- the bell (server)
  const armed = new Set<number>();
  const tick = () => {
    for (const [id, body] of ctx.bodies()) {
      const p = body.pos;
      if (p.y < 1) armed.add(id);
      else if (p.y > BELL.y && Math.hypot(p.x - BELL.x, p.z - BELL.z) < BELL.r && armed.delete(id))
        ctx.setScore(id, ctx.score(id) + 1);
    }
  };

  // Bots potter about the playground: onto the pillars, the trampolines and the pad, round the rink.
  const bot = arenaBrain({
    radius: 16,
    retarget: 3,
    social: true,
    pois: [
      { x: 0, z: 11 },
      ...TRAMPS,
      { x: PAD.x, z: PAD.z },
      { x: RINK.x, z: RINK.z },
      { x: SPINNER.x + 3, z: SPINNER.z + 2 },
      { x: BLOCKS.x + BLOCKS.step, z: BLOCKS.z + BLOCKS.step },
      { x: 6, z: 5 },
      { x: -5, z: -4 },
    ],
  });
  return {
    // A ring round the fountain, more places than players: a newcomer always finds a free one.
    spawns: b.ringSpawns(12, 5, 0.05, Math.PI / 12),
    killY: -15,
    faceCenter: true,
    view: new THREE.Vector3(0, 2, 0),
    ...(ctx.server ? { tick } : {}),
    bot,
  };
});
