import { useState } from 'preact/hooks';
import type { Profiler } from '../debug/profiler';

type Result = Awaited<ReturnType<Profiler['run']>>;

/** Dev tab: runs the profiler (about 20 s with experiments) and shows where frame time goes. */
export function ProfilerPanel() {
  const [busy, setBusy] = useState(false);
  const [res, setRes] = useState<Result | null>(null);
  const [err, setErr] = useState('');
  const run = async (ablate: boolean) => {
    const p = window.__fallbeans?.profile;
    if (!p) return;
    setBusy(true);
    setErr('');
    try {
      setRes(await p.run({ ablate }));
    } catch (e) {
      setErr(String(e));
    }
    setBusy(false);
  };
  const gpu = res?.passes.gpu;
  const cpu = res ? Object.entries(res.passes.cpu.sections) : [];
  return (
    <div class="stack prof">
      <div class="row wrap">
        <button type="button" class="btn chip" disabled={busy} onClick={() => run(false)}>
          {busy ? 'Замер…' : 'Профайлер: быстро'}
        </button>
        <button
          type="button"
          class="btn chip"
          disabled={busy}
          onClick={() => run(true)}
          title="Выключает по очереди тени, проходы, объекты (~20 с)"
        >
          + эксперименты
        </button>
      </div>
      {err && <p class="muted">✖ {err}</p>}
      {res && (
        <>
          <p class="muted">
            {res.device.renderer} · {res.size.join('×')} · {res.quality} · {res.map}
            {res.passes.throttled && ' · вкладка тормозилась: цифры неточные'}
          </p>
          <table class="prof-table">
            <tbody>
              <tr>
                <th colspan={3}>
                  GPU по проходам{' '}
                  {gpu
                    ? `(сумма ${gpu.total} мс, весь кадр ${res.passes.frameGpu} мс${res.passes.inflated ? ': смотрите на доли' : ''})`
                    : '— нет таймеров GPU'}
                </th>
              </tr>
              {gpu?.labels.map((l) => (
                <tr key={l.label}>
                  <td>{l.label}</td>
                  <td>{l.ms} мс</td>
                  <td>{l.share}%</td>
                </tr>
              ))}
              <tr>
                <th colspan={3}>CPU по частям кадра</th>
              </tr>
              {cpu.map(([k, v]) => (
                <tr key={k}>
                  <td>{k}</td>
                  <td>{v.perFrame} мс</td>
                  <td>{v.share}%</td>
                </tr>
              ))}
              <tr>
                <th colspan={3}>Сцена: вызовы · тени · треугольники</th>
              </tr>
              {Object.entries(res.scene.categories).map(([k, v]) => (
                <tr key={k}>
                  <td>{k}</td>
                  <td>
                    {v.draws} · {v.shadowDraws}
                  </td>
                  <td>{Math.round(v.triangles / 1000)}k</td>
                </tr>
              ))}
              {res.ablation && (
                <>
                  <tr>
                    <th colspan={3}>
                      Без функции: экономия GPU · CPU (база {res.ablation.baseline.gpu ?? res.ablation.baseline.frameMs} мс, шум ±
                      {res.ablation.baseline.noise ?? '?'})
                    </th>
                  </tr>
                  {res.ablation.rows.map((r) => (
                    <tr key={r.name} title={r.what} class={r.withinNoise ? 'dim' : ''}>
                      <td>{r.name}</td>
                      <td>{r.savedGpu ?? '—'} мс</td>
                      <td>{r.savedCpu} мс</td>
                    </tr>
                  ))}
                  {res.ablation.warnings.map((w) => (
                    <tr key={w}>
                      <td colspan={3} class="muted">
                        ⚠ {w}
                      </td>
                    </tr>
                  ))}
                </>
              )}
            </tbody>
          </table>
        </>
      )}
    </div>
  );
}
