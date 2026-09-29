import { describe, expect, it } from 'vitest';
import { decodeInput, decodeSnapshot, encodeInput, encodeSnapshot, FrameReader, frame, type Snapshot } from '../src/shared/codec';

describe('codec', () => {
  it('round-trips inputs and clamps values', () => {
    const pkt = {
      arena: 513,
      firstTick: -840,
      frames: [
        { mx: 127, mz: -127, buttons: 5 },
        { mx: 300, mz: 0, buttons: 255 },
      ],
    };
    const back = decodeInput(encodeInput(pkt))!;
    expect(back.arena).toBe(513);
    expect(back.firstTick).toBe(-840);
    expect(back.frames).toEqual([
      { mx: 127, mz: -127, buttons: 5 },
      { mx: 127, mz: 0, buttons: 7 },
    ]);
  });

  it('rejects malformed input packets', () => {
    expect(decodeInput(new Uint8Array([1, 0, 0]))).toBeNull();
    const ok = encodeInput({ arena: 1, firstTick: 0, frames: [{ mx: 0, mz: 0, buttons: 0 }] });
    expect(decodeInput(ok.slice(0, ok.length - 1))).toBeNull();
    const zero = new Uint8Array(ok);
    zero[7] = 0;
    expect(decodeInput(zero)).toBeNull();
  });

  it('round-trips snapshots with and without own state', () => {
    const s: Snapshot = {
      arena: 7,
      tick: 12345,
      own: {
        ack: 12340,
        s: {
          px: 1.25,
          py: -3.5,
          pz: 170.125,
          vx: 0.1,
          vy: -2,
          vz: 8.5,
          yaw: 1.5,
          state: 2,
          stateT: 0.25,
          grounded: true,
          coyote: 0.1,
          jumpBuf: 0,
          slowUntil: -1e6,
          groundCol: 42,
          landImpact: 0.5,
          slowK: 0.5,
          tilt: 1.2,
          tiltDir: -2,
          teleport: true,
        },
        grab: 9,
      },
      bodies: [{ id: 300, x: 1, y: 2, z: 3, yaw: 3, anim: 4, flags: 1, tilt: 1, tiltDir: 2, grab: -1 }],
    };
    const back = decodeSnapshot(encodeSnapshot(s))!;
    expect(back.tick).toBe(12345);
    expect(back.own?.s).toMatchObject({ px: 1.25, pz: 170.125, groundCol: 42, grounded: true, teleport: true, state: 2 });
    expect(back.bodies[0]).toMatchObject({ id: 300, x: 1, y: 2, z: 3, anim: 4, flags: 1 });
    expect(back.bodies[0]!.yaw).toBeCloseTo(3, 3);
    expect(back.bodies[0]).toMatchObject({ grab: -1 });
    expect(back.bodies[0]!.tilt).toBeCloseTo(1, 2);
    expect(back.bodies[0]!.tiltDir).toBeCloseTo(2, 1);
    expect(back.own).toMatchObject({ grab: 9, s: { slowK: 0.5 } });
    expect(back.own!.s.tilt).toBeCloseTo(1.2, 5);
    const spec = decodeSnapshot(encodeSnapshot({ ...s, own: null }))!;
    expect(spec.own).toBeNull();
    expect(spec.bodies).toHaveLength(1);
  });

  it('reassembles stream frames split across chunks', () => {
    const r = new FrameReader(100);
    const a = frame(new TextEncoder().encode('hello'));
    const b = frame(new TextEncoder().encode('world!'));
    const all = new Uint8Array([...a, ...b]);
    const out = [...r.push(all.slice(0, 3)), ...r.push(all.slice(3, 11)), ...r.push(all.slice(11))];
    expect(out.map((f) => new TextDecoder().decode(f))).toEqual(['hello', 'world!']);
    expect(() => new FrameReader(4).push(frame(new Uint8Array(10)))).toThrow();
  });
});
