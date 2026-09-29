export type Sfx =
  | 'jump'
  | 'dive'
  | 'hit'
  | 'boing'
  | 'break'
  | 'count'
  | 'go'
  | 'qualify'
  | 'out'
  | 'win'
  | 'click'
  | 'steal'
  | 'warn';

let actx: AudioContext | null = null;
let master: GainNode | null = null;
let volume = 0.8;

export function setVolume(v: number) {
  volume = v;
  if (master) master.gain.value = v;
}

export function sfx(type: Sfx) {
  if (volume <= 0) return;
  try {
    actx ??= new AudioContext();
    if (!master) {
      master = actx.createGain();
      master.gain.value = volume;
      master.connect(actx.destination);
    }
    if (actx.state === 'suspended') void actx.resume();
    const ctx = actx;
    const out = master;
    const t0 = ctx.currentTime;
    const tone = (f1: number, f2: number, dur: number, wave: OscillatorType = 'sine', vol = 0.12, delay = 0) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.type = wave;
      o.frequency.setValueAtTime(f1, t0 + delay);
      o.frequency.exponentialRampToValueAtTime(f2, t0 + delay + dur);
      g.gain.setValueAtTime(vol, t0 + delay);
      g.gain.exponentialRampToValueAtTime(0.001, t0 + delay + dur);
      o.connect(g).connect(out);
      o.start(t0 + delay);
      o.stop(t0 + delay + dur + 0.02);
    };
    const seq = (fs: number[], dur: number, wave: OscillatorType, vol: number, gap: number, bend = 1) => {
      fs.forEach((f, i) => {
        tone(f, f * bend, dur, wave, vol, i * gap);
      });
    };
    switch (type) {
      case 'jump':
        return tone(330, 620, 0.14, 'triangle', 0.1);
      case 'dive':
        return tone(500, 180, 0.2, 'triangle', 0.1);
      case 'hit':
        return tone(180, 60, 0.25, 'sawtooth', 0.09);
      case 'boing':
        return tone(200, 700, 0.22, 'sine', 0.14);
      case 'break':
        return tone(140, 50, 0.3, 'square', 0.08);
      case 'count':
        return tone(520, 520, 0.18, 'square', 0.07);
      case 'go':
        return tone(880, 880, 0.45, 'square', 0.08);
      case 'click':
        return tone(700, 900, 0.06, 'triangle', 0.06);
      case 'warn':
        return tone(660, 440, 0.25, 'square', 0.05);
      case 'steal':
        return seq([660, 990], 0.1, 'triangle', 0.1, 0.07);
      case 'qualify':
        return seq([523, 659, 784, 1046], 0.18, 'triangle', 0.12, 0.09);
      case 'out':
        return seq([440, 330, 220], 0.22, 'sawtooth', 0.07, 0.14, 0.95);
      case 'win':
        return seq([523, 659, 784, 1046, 784, 1046], 0.25, 'triangle', 0.12, 0.12);
    }
  } catch {}
}
