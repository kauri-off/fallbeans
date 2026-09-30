/**
 * Network stress test: a real game server (dev mode, WebSocket only) and simulated players on bad
 * connections, playing a real round.
 *   bun run stress [--clients 8] [--seconds 30] [--map door-dash] [--latency 60] [--jitter 20] [--loss 0.02]
 *
 * Each client logs in with the dev PIN, joins, and sends inputs at 60 Hz like the browser does
 * (redundant frames, a lead ahead of the server clock), through a fake link that delays, jitters
 * and drops input packets. Reported: snapshot rate and bandwidth per client, round-trip time,
 * inputs that arrived too late, and the server's own tick cost and event-loop lag while loaded.
 */
import { decodeSnapshot, encodeInput, type InputFrame } from '../src/shared/codec';
import { INPUT_EVERY, INPUT_REDUNDANCY, PROTOCOL_VERSION, TICK_MS } from '../src/shared/consts';
import type { ServerMsg } from '../src/shared/protocol';

const args = process.argv.slice(2);
const opt = (n: string, d: string) => {
  const i = args.indexOf(n);
  return i >= 0 ? (args[i + 1] ?? d) : d;
};
const CLIENTS = Math.min(8, Number(opt('--clients', '8')));
const SECONDS = Number(opt('--seconds', '30'));
const MAP = opt('--map', 'door-dash');
const LATENCY = Number(opt('--latency', '60'));
const JITTER = Number(opt('--jitter', '20'));
const LOSS = Number(opt('--loss', '0.02'));
const PORT = Number(opt('--port', '7791'));
const base = `http://127.0.0.1:${PORT}/fallbeans/`;

const server = Bun.spawn(
  [process.execPath, 'src/server/main.ts', '--dev', '--solo', '--no-wt', '--host', '127.0.0.1', '--port', String(PORT)],
  {
    stdout: 'ignore',
    stderr: 'inherit',
  },
);
const stop = () => server.kill();
process.on('exit', stop);

async function waitHealth() {
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(`${base}health`)).ok) return;
    } catch {}
    await Bun.sleep(100);
  }
  throw new Error('server did not start');
}

interface Client {
  i: number;
  ws: WebSocket;
  id: number;
  arena: { id: number; startAt: number } | null;
  offset: number;
  rtts: number[];
  snaps: number;
  bytes: number;
  lastTick: number;
  ackLag: number[];
  sent: number;
  dropped: number;
  msgs: Map<string, number>;
  dev: Map<number, (m: { ok: boolean; msg: string }) => void>;
}

const link = (fn: () => void, lossy: boolean) => {
  if (lossy && Math.random() < LOSS) return false;
  setTimeout(fn, Math.max(0, LATENCY / 2 + (Math.random() * 2 - 1) * JITTER));
  return true;
};

/** One login for everyone (the PIN endpoint allows 5 attempts a minute per address). */
let cookie = '';
async function login() {
  const auth = await fetch(`${base}api/auth`, { method: 'POST', body: JSON.stringify({ pin: '5050' }) });
  if (!auth.ok) throw new Error(`login failed: ${auth.status}`);
  cookie = (auth.headers.get('set-cookie') ?? '').split(';')[0]!;
}

async function connect(i: number): Promise<Client> {
  const session = (await (await fetch(`${base}api/session`, { headers: { cookie } })).json()) as { ticket: string };
  const ws = new WebSocket(`ws://127.0.0.1:${PORT}/fallbeans/ws`, { headers: { cookie } } as unknown as string[]);
  ws.binaryType = 'arraybuffer';
  const c: Client = {
    i,
    ws,
    id: -1,
    arena: null,
    offset: 0,
    rtts: [],
    snaps: 0,
    bytes: 0,
    lastTick: 0,
    ackLag: [],
    sent: 0,
    dropped: 0,
    msgs: new Map(),
    dev: new Map(),
  };
  const send = (m: object) => link(() => ws.readyState === 1 && ws.send(JSON.stringify(m)), false);
  // Downstream goes through the fake link too: control messages delayed, snapshots delayed or lost.
  ws.onmessage = (e) => {
    link(() => receive(e), typeof e.data !== 'string');
  };
  const receive = (e: MessageEvent) => {
    if (typeof e.data === 'string') {
      const m = JSON.parse(e.data) as ServerMsg;
      c.msgs.set(m.t, (c.msgs.get(m.t) ?? 0) + 1);
      if (m.t === 'welcome') c.id = m.id;
      if (m.t === 'arena') c.arena = { id: m.id, startAt: m.startAt };
      if (m.t === 'pong') {
        const now = performance.now();
        const rtt = now - m.c;
        c.rtts.push(rtt);
        c.offset = m.s + rtt / 2 - now;
      }
      if (m.t === 'devAck' && m.q !== null) c.dev.get(m.q)?.(m);
      // The server clock jumped (dev warp): re-sync like the browser does.
      if (m.t === 'clock') c.offset = m.s + (c.rtts.at(-1) ?? 0) / 2 - performance.now();
      return;
    }
    const data = new Uint8Array(e.data as ArrayBuffer);
    c.bytes += data.byteLength;
    const s = decodeSnapshot(data);
    if (!s || s.arena !== c.arena?.id) return;
    c.snaps++;
    c.lastTick = s.tick;
    if (s.own) c.ackLag.push(s.tick - s.own.ack);
  };
  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve();
    ws.onerror = () => reject(new Error('ws failed'));
  });
  send({ t: 'hello', v: PROTOCOL_VERSION, name: `Stress ${i}`, ticket: session.ticket });
  // Pings like the browser (every 2 s after the first few).
  let n = 0;
  setInterval(() => {
    if (++n < 6 || n % 4 === 0) send({ t: 'ping', c: performance.now() });
  }, 500);
  // Inputs: every INPUT_EVERY ticks, the last INPUT_REDUNDANCY frames, ahead by half the rtt + 30 ms.
  const history = new Map<number, InputFrame>();
  let sentUpTo = 0;
  let dir = Math.random() * Math.PI * 2;
  setInterval(() => {
    if (!c.arena || !c.rtts.length) return;
    const rtt = c.rtts.slice(-8).reduce((a, b) => a + b, 0) / Math.min(8, c.rtts.length);
    const target = Math.floor((performance.now() + c.offset + rtt / 2 + 30 - c.arena.startAt) / TICK_MS);
    if (target - sentUpTo > 200) sentUpTo = target - 1;
    for (let k = sentUpTo + 1; k <= target; k++) {
      if (Math.random() < 0.01) dir += (Math.random() - 0.5) * 2;
      history.set(k, {
        mx: Math.round(Math.sin(dir) * 127),
        mz: Math.round(Math.cos(dir) * 127),
        buttons: Math.random() < 0.01 ? 1 : 0,
      });
      history.delete(k - 64);
      if (k % INPUT_EVERY === 0) {
        const first = k - INPUT_REDUNDANCY + 1;
        const frames = Array.from({ length: INPUT_REDUNDANCY }, (_, j) => history.get(first + j) ?? { mx: 0, mz: 0, buttons: 0 });
        const pkt = encodeInput({ arena: c.arena.id, firstTick: first, frames });
        c.sent++;
        if (!link(() => ws.readyState === 1 && ws.send(pkt), true)) c.dropped++;
      }
    }
    sentUpTo = target;
  }, 8);
  return c;
}

function dev(c: Client, cmd: object): Promise<{ ok: boolean; msg: string }> {
  const q = Math.floor(Math.random() * 1e9);
  return new Promise((resolve) => {
    c.dev.set(q, resolve);
    c.ws.send(JSON.stringify({ t: 'dev', q, cmd }));
  });
}

await waitHealth();
await login();
const clients: Client[] = [];
for (let i = 0; i < CLIENTS; i++) clients.push(await connect(i));
await Bun.sleep(1500);
const host = clients[0]!;
console.log(`[stress] ${CLIENTS} clients on ws, link ${LATENCY}±${JITTER} ms, ${LOSS * 100}% input loss, map ${MAP}`);
console.log(`[stress] ${JSON.stringify(await dev(host, { c: 'start', games: [MAP], rounds: 1 }))}`);
await Bun.sleep(500);
console.log(`[stress] ${JSON.stringify(await dev(host, { c: 'skipIntro' }))}`);
// Measure from a second after the jump (the warp itself shows up as one big lag).
await Bun.sleep(1000);
for (const c of clients) {
  c.snaps = 0;
  c.bytes = 0;
  c.ackLag = [];
}
const t0 = performance.now();
const samples: { load: number; msPerTick: number; worst: number; lag: number }[] = [];
while (performance.now() - t0 < SECONDS * 1000) {
  await Bun.sleep(2000);
  const s = (await (await fetch(`${base}api/debug/state`)).json()) as {
    server: { health: { lagMs: number } | null };
    rooms: { perf: { load: number; msPerTick: number; worstUpdateMs: number } }[];
  };
  const p = s.rooms[0]!.perf;
  samples.push({ load: p.load, msPerTick: p.msPerTick, worst: p.worstUpdateMs, lag: s.server.health?.lagMs ?? 0 });
}
const secs = (performance.now() - t0) / 1000;
const state = (await (await fetch(`${base}api/debug/state`)).json()) as {
  rooms: {
    phase: string;
    arena: { pawns: { id: number; bot: boolean; lagTicks: number; rejected: number; queued: number }[] };
  }[];
};
const avg = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0);
const r1 = (v: number) => Math.round(v * 10) / 10;
console.log('\nclient  snaps/s  kbit/s down  rtt ms  ack lag ticks (avg/max)  inputs sent  dropped');
for (const c of clients)
  console.log(
    `${String(c.i).padStart(6)} ${String(r1(c.snaps / secs)).padStart(8)} ${String(r1((c.bytes * 8) / secs / 1000)).padStart(12)} ${String(r1(avg(c.rtts.slice(-10)))).padStart(7)} ${`${r1(avg(c.ackLag))}/${Math.max(0, ...c.ackLag)}`.padStart(24)} ${String(c.sent).padStart(12)} ${String(c.dropped).padStart(8)}`,
  );
const pawns = state.rooms[0]!.arena.pawns.filter((p) => !p.bot);
console.log(
  `\nserver: load ${r1(avg(samples.map((s) => s.load)))}% of a core · ${avg(samples.map((s) => s.msPerTick)).toFixed(3)} ms/tick · worst update ${r1(
    Math.max(0, ...samples.map((s) => s.worst)),
  )} ms · event-loop lag max ${Math.max(0, ...samples.map((s) => s.lag))} ms · phase ${state.rooms[0]!.phase}`,
);
console.log(`inputs rejected (too far ahead) per player: ${pawns.map((p) => p.rejected).join(' ')}`);
stop();
process.exit(0);
