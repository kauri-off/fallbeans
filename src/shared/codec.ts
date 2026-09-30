/**
 * Binary packets for the hot path. Little-endian, fixed layouts, validated on decode.
 *
 * Input (client → server), 8 + 3n bytes:
 *   u8 type=1 · u16 arena · i32 firstTick · u8 n · n × (i8 mx · i8 mz · u8 buttons)
 * Snapshot (server → client):
 *   u8 type=2 · u16 arena · i32 tick · u8 flags
 *   [flags & 1: i32 ackTick · own body state (FULL_BYTES) · i16 grabbed id (−1 none)]
 *   u8 count · count × (u16 id · f32 x · f32 y · f32 z · u16 yaw · u8 anim · u8 flags · u8 tilt · u8 tiltDir · u16 grab+1)
 */

export const PKT_INPUT = 1;
export const PKT_SNAPSHOT = 2;

export const BTN = { jump: 1, dive: 2, grab: 4 } as const;
export const MAX_INPUT_FRAMES = 32;

/** One simulation tick of player input. mx/mz are world-space, quantized to −127…127. */
export interface InputFrame {
  mx: number;
  mz: number;
  buttons: number;
}

export interface InputPacket {
  arena: number;
  firstTick: number;
  frames: InputFrame[];
}

export const quantizeAxis = (v: number) => clampAxis(v * 127);
export const axisValue = (q: number) => q / 127;
const clampAxis = (q: number) => Math.max(-127, Math.min(127, Math.round(q) || 0));

export function encodeInput(p: InputPacket): Uint8Array<ArrayBuffer> {
  const n = Math.min(p.frames.length, MAX_INPUT_FRAMES);
  const buf = new ArrayBuffer(8 + 3 * n);
  const v = new DataView(buf);
  v.setUint8(0, PKT_INPUT);
  v.setUint16(1, p.arena, true);
  v.setInt32(3, p.firstTick, true);
  v.setUint8(7, n);
  for (let i = 0; i < n; i++) {
    const f = p.frames[i]!;
    v.setInt8(8 + i * 3, clampAxis(f.mx));
    v.setInt8(9 + i * 3, clampAxis(f.mz));
    v.setUint8(10 + i * 3, f.buttons & 7);
  }
  return new Uint8Array(buf);
}

export function decodeInput(data: Uint8Array): InputPacket | null {
  if (data.byteLength < 8) return null;
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  if (v.getUint8(0) !== PKT_INPUT) return null;
  const n = v.getUint8(7);
  if (n < 1 || n > MAX_INPUT_FRAMES || data.byteLength !== 8 + 3 * n) return null;
  const frames: InputFrame[] = [];
  for (let i = 0; i < n; i++) {
    const mx = Math.max(-127, v.getInt8(8 + i * 3));
    const mz = Math.max(-127, v.getInt8(9 + i * 3));
    frames.push({ mx, mz, buttons: v.getUint8(10 + i * 3) & 7 });
  }
  return { arena: v.getUint16(1, true), firstTick: v.getInt32(3, true), frames };
}

/** Everything client prediction needs to continue simulating its own body from a server state. */
export interface BodyFullState {
  px: number;
  py: number;
  pz: number;
  vx: number;
  vy: number;
  vz: number;
  yaw: number;
  state: number;
  stateT: number;
  grounded: boolean;
  coyote: number;
  jumpBuf: number;
  slowUntil: number;
  slowK: number;
  groundCol: number;
  landImpact: number;
  tilt: number;
  tiltDir: number;
  /** Bonus in effect (physics POWER) and the sim time it wears off. */
  power: number;
  powerUntil: number;
  /** A teleport (respawn): the client snaps instead of smoothing. */
  teleport: boolean;
}

export const FULL_BYTES = 8 * 7 + 1 + 4 + 1 + 4 + 4 + 4 + 2 + 4 + 4 + 4 + 4 + 1 + 4;

export interface RemoteState {
  id: number;
  x: number;
  y: number;
  z: number;
  yaw: number;
  anim: number;
  flags: number;
  /** Tumbling: tip-over angle (0…π/2) and its direction. */
  tilt: number;
  tiltDir: number;
  /** Id of the bean this one holds, or −1. */
  grab: number;
}

export const REMOTE_BYTES = 22;
/** Remote flags: holding someone, reaching out (grab held, nobody in hand); bits 2–3: the bonus in effect. */
export const REMOTE_FLAG = { grab: 1, reach: 2 } as const;
export const POWER_SHIFT = 2;

export interface Snapshot {
  arena: number;
  tick: number;
  own: { ack: number; s: BodyFullState; grab: number } | null;
  bodies: RemoteState[];
}

const TAU = Math.PI * 2;
const yawToU16 = (y: number) => Math.round((((y % TAU) + TAU) % TAU) * (65535 / TAU)) & 0xffff;
const u16ToYaw = (q: number) => (q * TAU) / 65535;
const tiltToU8 = (t: number) => Math.max(0, Math.min(255, Math.round((t / (Math.PI / 2)) * 255)));
const u8ToTilt = (q: number) => (q / 255) * (Math.PI / 2);
const dirToU8 = (y: number) => Math.round((((y % TAU) + TAU) % TAU) * (255 / TAU)) & 0xff;
const u8ToDir = (q: number) => (q * TAU) / 255;

export function encodeSnapshot(s: Snapshot): Uint8Array<ArrayBuffer> {
  const size = 8 + (s.own ? 4 + FULL_BYTES + 2 : 0) + 1 + s.bodies.length * REMOTE_BYTES;
  const buf = new ArrayBuffer(size);
  const v = new DataView(buf);
  let o = 0;
  v.setUint8(o, PKT_SNAPSHOT);
  v.setUint16(o + 1, s.arena, true);
  v.setInt32(o + 3, s.tick, true);
  v.setUint8(o + 7, s.own ? 1 : 0);
  o = 8;
  if (s.own) {
    const f = s.own.s;
    v.setInt32(o, s.own.ack, true);
    o += 4;
    for (const x of [f.px, f.py, f.pz, f.vx, f.vy, f.vz, f.yaw]) {
      v.setFloat64(o, x, true);
      o += 8;
    }
    v.setUint8(o, f.state);
    v.setFloat32(o + 1, f.stateT, true);
    v.setUint8(o + 5, (f.grounded ? 1 : 0) | (f.teleport ? 2 : 0));
    v.setFloat32(o + 6, f.coyote, true);
    v.setFloat32(o + 10, f.jumpBuf, true);
    v.setFloat32(o + 14, f.slowUntil, true);
    v.setInt16(o + 18, f.groundCol, true);
    v.setFloat32(o + 20, f.landImpact, true);
    v.setFloat32(o + 24, f.slowK, true);
    v.setFloat32(o + 28, f.tilt, true);
    v.setFloat32(o + 32, f.tiltDir, true);
    v.setUint8(o + 36, f.power);
    v.setFloat32(o + 37, f.powerUntil, true);
    v.setInt16(o + 41, s.own.grab, true);
    o += 43;
  }
  v.setUint8(o, s.bodies.length);
  o += 1;
  for (const b of s.bodies) {
    v.setUint16(o, b.id, true);
    v.setFloat32(o + 2, b.x, true);
    v.setFloat32(o + 6, b.y, true);
    v.setFloat32(o + 10, b.z, true);
    v.setUint16(o + 14, yawToU16(b.yaw), true);
    v.setUint8(o + 16, b.anim);
    v.setUint8(o + 17, b.flags);
    v.setUint8(o + 18, tiltToU8(b.tilt));
    v.setUint8(o + 19, dirToU8(b.tiltDir));
    v.setUint16(o + 20, b.grab + 1, true);
    o += REMOTE_BYTES;
  }
  return new Uint8Array(buf);
}

export function decodeSnapshot(data: Uint8Array): Snapshot | null {
  if (data.byteLength < 9) return null;
  const v = new DataView(data.buffer, data.byteOffset, data.byteLength);
  if (v.getUint8(0) !== PKT_SNAPSHOT) return null;
  const arena = v.getUint16(1, true);
  const tick = v.getInt32(3, true);
  const hasOwn = (v.getUint8(7) & 1) === 1;
  let o = 8;
  let own: Snapshot['own'] = null;
  if (hasOwn) {
    if (data.byteLength < o + 4 + FULL_BYTES + 2 + 1) return null;
    const ack = v.getInt32(o, true);
    o += 4;
    const d: number[] = [];
    for (let i = 0; i < 7; i++) {
      d.push(v.getFloat64(o, true));
      o += 8;
    }
    const flags = v.getUint8(o + 5);
    own = {
      ack,
      s: {
        px: d[0]!,
        py: d[1]!,
        pz: d[2]!,
        vx: d[3]!,
        vy: d[4]!,
        vz: d[5]!,
        yaw: d[6]!,
        state: v.getUint8(o),
        stateT: v.getFloat32(o + 1, true),
        grounded: (flags & 1) !== 0,
        teleport: (flags & 2) !== 0,
        coyote: v.getFloat32(o + 6, true),
        jumpBuf: v.getFloat32(o + 10, true),
        slowUntil: v.getFloat32(o + 14, true),
        groundCol: v.getInt16(o + 18, true),
        landImpact: v.getFloat32(o + 20, true),
        slowK: v.getFloat32(o + 24, true),
        tilt: v.getFloat32(o + 28, true),
        tiltDir: v.getFloat32(o + 32, true),
        power: v.getUint8(o + 36),
        powerUntil: v.getFloat32(o + 37, true),
      },
      grab: v.getInt16(o + 41, true),
    };
    o += 43;
  }
  const count = v.getUint8(o);
  o += 1;
  if (data.byteLength !== o + count * REMOTE_BYTES) return null;
  const bodies: RemoteState[] = [];
  for (let i = 0; i < count; i++) {
    bodies.push({
      id: v.getUint16(o, true),
      x: v.getFloat32(o + 2, true),
      y: v.getFloat32(o + 6, true),
      z: v.getFloat32(o + 10, true),
      yaw: u16ToYaw(v.getUint16(o + 14, true)),
      anim: v.getUint8(o + 16),
      flags: v.getUint8(o + 17),
      tilt: u8ToTilt(v.getUint8(o + 18)),
      tiltDir: u8ToDir(v.getUint8(o + 19)),
      grab: v.getUint16(o + 20, true) - 1,
    });
    o += REMOTE_BYTES;
  }
  return { arena, tick, own, bodies };
}

/** Frames for the reliable WebTransport stream: u32 length + UTF-8 JSON. */
export class FrameReader {
  private buf = new Uint8Array(0);
  constructor(private readonly max: number) {}

  /** Appends a chunk and returns complete frames; throws on an oversized frame. */
  push(chunk: Uint8Array): Uint8Array[] {
    const next = new Uint8Array(this.buf.length + chunk.length);
    next.set(this.buf);
    next.set(chunk, this.buf.length);
    this.buf = next;
    const out: Uint8Array[] = [];
    for (;;) {
      if (this.buf.length < 4) break;
      const len = new DataView(this.buf.buffer, this.buf.byteOffset, 4).getUint32(0, true);
      if (len > this.max) throw new Error(`frame too large: ${len}`);
      if (this.buf.length < 4 + len) break;
      out.push(this.buf.slice(4, 4 + len));
      this.buf = this.buf.slice(4 + len);
    }
    return out;
  }
}

export function frame(payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(4 + payload.length);
  new DataView(out.buffer).setUint32(0, payload.length, true);
  out.set(payload, 4);
  return out;
}
