import { useEffect, useState } from 'preact/hooks';
import { log } from '../debug/capture';
import type { Game } from '../game/game';
import { debugOverlay } from '../state';

/** F3: frame times, render cost, network and the local bean's state, refreshed four times a second. */
export function DebugOverlay({ game }: { game: Game }) {
  const [, redraw] = useState(0);
  useEffect(() => {
    const id = setInterval(() => redraw((n) => n + 1), 250);
    return () => clearInterval(id);
  }, []);
  const probe = window.__fallbeans;
  if (!probe) return null;
  const r = probe.render();
  const n = probe.net();
  const a = n.arena;
  const t = probe.time();
  const b = probe.body();
  const m = probe.memory();
  const errors = log.filter((l) => l.level === 'error');
  const lines: [string, string][] = [
    ['fps', `${r.fps}  frame p50 ${r.frame.p50} p95 ${r.frame.p95} max ${r.frame.max} ms`],
    ['cpu', `p50 ${r.cpu.p50} p95 ${r.cpu.p95} ms${r.gpu.n ? `  gpu p50 ${r.gpu.p50} p95 ${r.gpu.p95} ms` : ''}`],
    [
      'draw',
      `${r.calls} calls  ${Math.round(r.triangles / 1000)}k tris  ${r.programs} programs  ${r.geometries} geo  ${r.textures} tex`,
    ],
    ['view', `${r.size[0]}×${r.size[1]} @${r.pixelRatio}  ${r.quality} fsr:${r.upscale}  ${r.sceneObjects} objects`],
    [
      'net',
      `${n.transport ?? '—'}  rtt ${n.rtt}  lead ${a?.leadMs ?? '—'}  jitter ${a?.jitterMs ?? '—'}  snap age ${a?.snapshotAgeMs ?? '—'} ms`,
    ],
    ['sync', `corr ${a?.corrections ?? 0}  err ${a?.offset ?? 0}  interp ${a?.interpDelayMs ?? '—'} ms  rate ×${n.rate}`],
    ['time', `${t.arena ?? '—'} ${t.kind ?? ''}  t ${t.t ?? '—'}  tick ${t.tick ?? '—'}  left ${t.timeLeft ?? '—'} s`],
  ];
  if (b)
    lines.push(
      ['pos', `${b.pos.map((v) => v.toFixed(2)).join(' ')}  v ${b.speed.toFixed(1)}`],
      [
        'body',
        `${b.state}${b.grounded ? ' ground' : ' air'}${b.ground?.tag ? `(${b.ground.tag})` : ''}  tilt ${b.tilt.toFixed(2)}  hold ${b.holding}`,
      ],
    );
  if (m) lines.push(['heap', `${m.heapMB} / ${m.limitMB} MB`]);
  if (errors.length) lines.push(['errors', `${errors.length}: ${errors.at(-1)!.msg.slice(0, 80)}`]);
  return (
    <div class="debug-overlay" aria-hidden="true" onClick={() => (debugOverlay.value = false)}>
      <div class="debug-build">
        {__BUILD__} · {game.net.kind ?? 'offline'}
      </div>
      {lines.map(([k, v]) => (
        <div key={k}>
          <b>{k}</b> {v}
        </div>
      ))}
    </div>
  );
}
