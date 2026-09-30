import './page.css';

/**
 * /fallbeans/debug/: the server's view of the game — rooms and beans, simulation cost by section,
 * client error reports, log lines, a bean's recent path, and the audits. Reads api/debug/*.
 */

const API = '../api/debug/';
const TABS = [
  ['overview', 'Обзор'],
  ['errors', 'Ошибки клиентов'],
  ['logs', 'Журнал'],
  ['trace', 'Трасса'],
  ['audit', 'Аудит'],
] as const;
type Tab = (typeof TABS)[number][0];

const view = document.getElementById('view')!;
const tabsEl = document.getElementById('tabs')!;
const auto = document.getElementById('auto') as HTMLInputElement;
let tab: Tab = (location.hash.slice(1) as Tab) || 'overview';
let timer: ReturnType<typeof setTimeout> | null = null;

const esc = (v: unknown) =>
  String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
const num = (v: unknown, d = 1) => (typeof v === 'number' ? v.toFixed(d) : esc(v));

async function get<T>(path: string): Promise<T> {
  const r = await fetch(API + path, { cache: 'no-store' });
  if (r.status === 403)
    throw new Error('Нет доступа: откройте с компьютера сервера (dev) или войдите по ключу: api/debug/login?key=…');
  if (!r.ok) throw new Error(`${r.status} ${await r.text()}`);
  return r.json() as Promise<T>;
}

function table(rows: Record<string, unknown>[], cols?: string[]): string {
  if (!rows.length) return '<p class="muted">пусто</p>';
  const keys = cols ?? Object.keys(rows[0]!);
  const cell = (v: unknown) =>
    typeof v === 'object' && v !== null
      ? `<code>${esc(JSON.stringify(v))}</code>`
      : typeof v === 'number'
        ? num(v, Number.isInteger(v) ? 0 : 2)
        : esc(v);
  return `<table><thead><tr>${keys.map((k) => `<th>${esc(k)}</th>`).join('')}</tr></thead><tbody>${rows
    .map((r) => `<tr>${keys.map((k) => `<td>${cell(r[k])}</td>`).join('')}</tr>`)
    .join('')}</tbody></table>`;
}

// ------------------------------------------------------------------ tabs

interface State {
  server: Record<string, unknown> & { health: Record<string, number> | null };
  rooms: (Record<string, unknown> & {
    room: number;
    phase: string;
    players: Record<string, unknown>[];
    arena: Record<string, unknown> & { pawns: Record<string, unknown>[]; journal?: unknown };
    perf: Record<string, unknown> & {
      profile: { frames: number; sections: Record<string, { perFrame: number; share: number; max: number }> };
    };
  })[];
}

async function overview() {
  const s = await get<State>('state');
  const h = s.server.health;
  const head = `<section class="cards">
    <div class="card"><b>Сборка</b>${esc(s.server.build)} · протокол ${esc(s.server.protocol)}${s.server.dev ? ' · <span class="tag">dev</span>' : ''}</div>
    <div class="card"><b>Работает</b>${Math.round(Number(s.server.uptime) / 60)} мин · Bun ${esc(s.server.bun)}</div>
    <div class="card"><b>Память</b>${h ? `${h.rssMB} МБ RSS · куча ${h.heapMB} МБ` : '—'}</div>
    <div class="card"><b>CPU</b>${h ? `${h.cpu}% ядра · задержка цикла ${h.lagMs} мс` : '—'}</div>
    <div class="card"><b>Ошибки клиентов</b>${esc(s.server.reports)} · предупреждений ${esc(s.server.warnings)}</div>
  </section>`;
  const rooms = s.rooms
    .map((r) => {
      const a = r.arena;
      const prof = Object.entries(r.perf.profile.sections).map(([k, v]) => ({
        section: k,
        'мс/тик': v.perFrame,
        'доля %': v.share,
        'макс мс': v.max,
      }));
      return `<section>
        <h2>Комната ${r.room}${r.practice ? ' (тренировка)' : ` «${esc(r.title)}» [${esc(r.id)}]${r.private ? ' 🔒' : ''}`}: ${esc(r.phase)} · ${esc(a.game)} (${esc(a.kind)}) · тик ${esc(a.tick)} · t ${num(a.t)} с${
          Number(r.rate) !== 1 ? ` · время ×${esc(r.rate)}` : ''
        }</h2>
        <p class="muted">Нагрузка: ${esc((r.perf as Record<string, unknown>).load)}% ядра · ${esc((r.perf as Record<string, unknown>).msPerTick)} мс/тик · худшее обновление ${esc(
          (r.perf as Record<string, unknown>).worstUpdateMs,
        )} мс · seed ${esc(a.seed)}</p>
        <div class="grid2">
          <div><h3>Игроки (хост #${esc(r.host)})</h3>${table(r.players, ['id', 'name', 'bot', 'owner', 'connected', 'via', 'rtt', 'score', 'spectator'])}</div>
          <div><h3>Где тратится тик</h3>${table(prof)}</div>
        </div>
        <h3>Бобы</h3>${table(a.pawns, ['id', 'bot', 'status', 'pos', 'speed', 'state', 'grounded', 'progress', 'grabbing', 'lagTicks', 'queued', 'rejected'])}
      </section>`;
    })
    .join('');
  view.innerHTML = head + rooms;
}

async function errors() {
  const { reports } = await get<{ reports: Record<string, unknown>[] }>('errors');
  view.innerHTML = reports.length
    ? reports
        .map(
          (r) =>
            `<details class="report"><summary><span class="tag">${esc(r.kind)}</span> ${esc(r.msg)} <span class="muted">×${esc(r.count)} · ${esc(
              r.ts,
            )} · ${esc(r.build)}</span></summary><pre>${esc(r.stack)}</pre><code>${esc(JSON.stringify(r.ctx))}</code><p class="muted">${esc(r.ua)} · ${esc(r.path)}</p></details>`,
        )
        .join('')
    : '<p class="muted">Ошибок не было.</p>';
}

async function logs() {
  const { lines } = await get<{ lines: { ts: string; level: string; msg: string; data?: unknown }[] }>('logs?n=300');
  view.innerHTML = `<pre class="log">${lines
    .slice()
    .reverse()
    .map(
      (l) =>
        `<span class="${l.level}">${esc(l.ts.slice(11, 19))} ${l.level === 'warn' ? '⚠' : '·'} ${esc(l.msg)}</span> ${esc(l.data ? JSON.stringify(l.data) : '')}`,
    )
    .join('\n')}</pre>`;
}

let traceRoom = 0;
let traceId = '';
async function trace() {
  const t = await get<{
    game: string;
    t: number;
    journal: { t: number; what: string; id?: number; data?: unknown }[];
    trace: Record<
      string,
      { t: number; pos: number[]; vel: number[]; state: string; grounded: boolean; input: number[]; hazard: string | null }[]
    >;
  }>(`trace?room=${traceRoom}&s=20${traceId ? `&id=${traceId}` : ''}`);
  const ids = Object.keys(t.trace);
  const all = Object.values(t.trace).flat();
  // Top view (x, z) of every path, and height over time.
  const xs = all.map((e) => e.pos[0]!);
  const zs = all.map((e) => e.pos[2]!);
  const [x0, x1, z0, z1] = [Math.min(...xs, -5), Math.max(...xs, 5), Math.min(...zs, -5), Math.max(...zs, 5)];
  const W = 460;
  const H = 300;
  const sx = (x: number) => ((x - x0) / (x1 - x0 || 1)) * (W - 20) + 10;
  const sz = (z: number) => H - 10 - ((z - z0) / (z1 - z0 || 1)) * (H - 20);
  const colors = ['#ff5fa2', '#3fa9ff', '#ffd23f', '#4fdc6a', '#a66bff', '#ff8a3d', '#39e0d0', '#888'];
  const paths = ids
    .map((id, i) => {
      const pts = t.trace[id]!.map((e) => `${sx(e.pos[0]!).toFixed(1)},${sz(e.pos[2]!).toFixed(1)}`).join(' ');
      return `<polyline points="${pts}" fill="none" stroke="${colors[i % colors.length]}" stroke-width="1.5"><title>#${esc(id)}</title></polyline>`;
    })
    .join('');
  const legend = ids.map((id, i) => `<span style="color:${colors[i % colors.length]}">■ #${esc(id)}</span>`).join(' ');
  const one = traceId ? (t.trace[traceId] ?? []) : [];
  view.innerHTML = `<div class="row">
      <label>Комната № <input id="troom" type="number" min="0" value="${traceRoom}" /></label>
      <label>Боб <input id="tid" placeholder="все" value="${esc(traceId)}" /></label>
      <span class="muted">${esc(t.game)} · t ${num(t.t)} с · последние 20 с</span>
    </div>
    <div class="grid2"><div><svg viewBox="0 0 ${W} ${H}" class="plot">${paths}</svg><p>${legend}</p></div>
    <div><h3>События</h3>${table(t.journal.slice().reverse().slice(0, 60) as unknown as Record<string, unknown>[], ['t', 'what', 'id', 'data'])}</div></div>
    ${one.length ? `<h3>#${esc(traceId)} по шагам</h3>${table(one.slice().reverse() as unknown as Record<string, unknown>[], ['t', 'pos', 'vel', 'state', 'grounded', 'input', 'hazard'])}` : ''}`;
  (document.getElementById('troom') as HTMLInputElement).onchange = (e) => {
    traceRoom = Number((e.target as HTMLInputElement).value) || 0;
    void render();
  };
  (document.getElementById('tid') as HTMLInputElement).onchange = (e) => {
    traceId = (e.target as HTMLInputElement).value.trim();
    void render();
  };
}

let auditHtml = '';
function audit(): Promise<void> {
  if (!auditHtml)
    view.innerHTML = `<p>Аудиты карт и систем: правила, точки появления, пересечения, детерминизм, баланс ботов.</p>
      <div class="row"><button id="aq">Быстрый аудит</button><button id="af">Полный (с балансом, ~15 с)</button>
      <input id="amap" placeholder="карты через запятую (все)" /></div><div id="ares"></div>`;
  else view.innerHTML = auditHtml;
  const go = async (full: boolean) => {
    const res = document.getElementById('ares')!;
    res.innerHTML = '<p class="muted">Идёт аудит…</p>';
    const maps = (document.getElementById('amap') as HTMLInputElement).value.trim();
    const r = await get<{
      ms: number;
      summary: { errors: number; warnings: number; infos: number; audits: number };
      results: {
        audit: string;
        map: string;
        ms: number;
        findings: { severity: string; msg: string; at?: number[]; t?: number }[];
        metrics: Record<string, unknown>;
      }[];
    }>(`audit?${full ? 'full=1&' : ''}${maps ? `map=${encodeURIComponent(maps)}` : ''}`);
    res.innerHTML = `<p><b>${r.summary.errors}</b> ошибок · <b>${r.summary.warnings}</b> предупреждений · ${r.summary.infos} заметок · ${r.summary.audits} аудитов за ${(r.ms / 1000).toFixed(1)} с</p>
      ${r.results
        .filter((x) => x.findings.length || Object.keys(x.metrics).length)
        .map(
          (x) =>
            `<details ${x.findings.some((f) => f.severity !== 'info') ? 'open' : ''}><summary>${esc(x.map)} / ${esc(x.audit)} <span class="muted">${x.ms} мс · ${
              x.findings.length
            }</span></summary><ul>${x.findings
              .map(
                (f) =>
                  `<li class="${esc(f.severity)}">${esc(f.msg)} ${f.t !== undefined ? `<span class="muted">t=${esc(f.t)}</span>` : ''} ${f.at ? `<span class="muted">(${esc(f.at.join(' '))})</span>` : ''}</li>`,
              )
              .join('')}</ul><p class="muted">${Object.entries(x.metrics)
              .map(([k, v]) => `${esc(k)}=${esc(v)}`)
              .join(' · ')}</p></details>`,
        )
        .join('')}`;
    auditHtml = view.innerHTML;
  };
  document.getElementById('aq')!.onclick = () => void go(false);
  document.getElementById('af')!.onclick = () => void go(true);
  return Promise.resolve();
}

// ------------------------------------------------------------------ shell

const renderers: Record<Tab, () => Promise<void>> = { overview, errors, logs, trace, audit };
const refreshing: Tab[] = ['overview', 'errors', 'logs', 'trace'];

async function render() {
  if (timer) clearTimeout(timer);
  tabsEl.innerHTML = TABS.map(([id, label]) => `<a href="#${id}" class="${id === tab ? 'on' : ''}">${label}</a>`).join('');
  try {
    // Keep the page while the user reads: do not re-render a tab that has focused inputs.
    if (!(document.activeElement instanceof HTMLInputElement && document.activeElement.id !== 'auto')) await renderers[tab]();
  } catch (e) {
    view.innerHTML = `<p class="error">${esc(e instanceof Error ? e.message : e)}</p>`;
  }
  if (auto.checked && refreshing.includes(tab)) timer = setTimeout(render, 2000);
}

window.addEventListener('hashchange', () => {
  tab = (location.hash.slice(1) as Tab) || 'overview';
  void render();
});
auto.onchange = () => void render();
void render();
