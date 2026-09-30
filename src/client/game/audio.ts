/** Simple synthesized sound effects. The AudioContext is only created after a user gesture. */
export type Sfx =
  | 'jump'
  | 'dive'
  | 'hit'
  | 'tackle'
  | 'grab'
  | 'boing'
  | 'break'
  | 'count'
  | 'go'
  | 'qualify'
  | 'finish'
  | 'results'
  | 'out'
  | 'fall'
  | 'win'
  | 'click'
  | 'steal'
  | 'pickup'
  | 'warn';

let actx: AudioContext | null = null;
let master: GainNode | null = null;
let volume = 0.8;
let unlocked = false;

export function setVolume(v: number) {
  volume = v;
  if (master) master.gain.value = v;
}

export function sfx(type: Sfx, gain = 1) {
  if (!unlocked || !actx || !master || volume <= 0) return;
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
    g.gain.setValueAtTime(vol * gain, t0 + delay);
    g.gain.exponentialRampToValueAtTime(0.001, t0 + delay + dur);
    o.connect(g).connect(out);
    o.start(t0 + delay);
    o.stop(t0 + delay + dur + 0.02);
  };
  switch (type) {
    case 'jump':
      return tone(330, 620, 0.14, 'triangle', 0.1);
    case 'dive':
      return tone(500, 180, 0.2, 'triangle', 0.1);
    case 'hit':
    case 'tackle':
    case 'fall':
      return tone(180, 60, 0.25, 'sawtooth', 0.09);
    case 'grab':
      return tone(260, 200, 0.1, 'triangle', 0.08);
    case 'boing':
    case 'steal':
      return tone(200, 700, 0.22, 'sine', 0.14);
    case 'break':
      return tone(140, 50, 0.3, 'square', 0.08);
    case 'pickup':
      tone(988, 988, 0.07, 'triangle', 0.1);
      return tone(1319, 1319, 0.16, 'triangle', 0.1, 0.07);
    case 'count':
    case 'warn':
      return tone(520, 520, 0.18, 'square', 0.07);
    case 'go':
      return tone(880, 880, 0.45, 'square', 0.08);
    case 'click':
      return tone(700, 900, 0.05, 'triangle', 0.05);
    case 'qualify':
    case 'finish':
    case 'results':
      for (const [i, f] of [523, 659, 784, 1046].entries()) tone(f, f, 0.18, 'triangle', 0.12, i * 0.09);
      return;
    case 'out':
      for (const [i, f] of [440, 330, 220].entries()) tone(f, f * 0.95, 0.22, 'sawtooth', 0.07, i * 0.14);
      return;
    case 'win':
      for (const [i, f] of [523, 659, 784, 1046, 784, 1046].entries()) tone(f, f, 0.25, 'triangle', 0.12, i * 0.12);
      return;
  }
}

// Browsers only allow audio after a gesture: create the context on the first one.
const unlock = () => {
  if (unlocked) return;
  try {
    actx = new AudioContext();
    master = actx.createGain();
    master.gain.value = volume;
    master.connect(actx.destination);
    unlocked = true;
  } catch {
    return;
  }
  for (const ev of ['pointerdown', 'keydown'] as const) window.removeEventListener(ev, unlock, true);
};
for (const ev of ['pointerdown', 'keydown'] as const) window.addEventListener(ev, unlock, true);
