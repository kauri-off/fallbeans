import { useState } from 'preact/hooks';
import { GAMES } from '../../games';
import type { DevCmd } from '../../shared/protocol';
import type { Game } from '../game/game';
import { arenaInfo, debugOverlay, myId } from '../state';
import { ProfilerPanel } from './ProfilerPanel';

const RATES = [0, 0.1, 0.25, 0.5, 1, 2, 4];

/** Dev tools (server started with --dev): time control, quick games, teleports, bots, forced hits. */
export function DevTab({ game }: { game: Game }) {
  const [map, setMap] = useState(GAMES[0]?.id ?? '');
  const [bots, setBots] = useState(3);
  const [last, setLast] = useState('');
  const [gpu, setGpu] = useState(false);
  const run = async (cmd: DevCmd) => {
    const r = await game.dev(cmd);
    setLast(`${r.ok ? '✔' : '✖'} ${r.msg}`);
  };
  const info = arenaInfo.value;
  const world = window.__fallbeans?.world();
  const rate = game.net.clock.rate;
  const nearest = () => {
    const me = myId.value;
    return window.__fallbeans
      ?.beans()
      .filter((b) => b.id !== me && b.visible && b.dist !== null)
      .sort((a, b) => a.dist! - b.dist!)[0]?.id;
  };
  const grab = (meHolds: boolean) => {
    const other = nearest();
    if (other === undefined) return setLast('✖ никого рядом');
    void run(meHolds ? { c: 'grab', target: other } : { c: 'grab', actor: other, target: myId.value });
  };
  return (
    <div class="stack dev">
      <div class="field">
        <span>Время игры: ×{rate}</span>
        <div class="row wrap">
          {RATES.map((k) => (
            <button type="button" key={k} class={`btn chip${rate === k ? ' on' : ''}`} onClick={() => run({ c: 'rate', k })}>
              {k === 0 ? '⏸' : `×${k}`}
            </button>
          ))}
          <button type="button" class="btn chip" disabled={rate !== 0} onClick={() => run({ c: 'step', ticks: 1 })}>
            +1 тик
          </button>
          <button type="button" class="btn chip" disabled={rate !== 0} onClick={() => run({ c: 'step', ticks: 60 })}>
            +0.5 с
          </button>
        </div>
      </div>
      <div class="row wrap">
        <button type="button" class="btn chip" onClick={() => run({ c: 'skipIntro' })}>
          Пропустить заставку
        </button>
        <button type="button" class="btn chip" onClick={() => run({ c: 'warp', ms: 10_000 })}>
          +10 с
        </button>
        <button type="button" class="btn chip" onClick={() => run({ c: 'endRound' })}>
          Завершить раунд
        </button>
        <button type="button" class="btn chip" onClick={() => run({ c: 'lobby' })}>
          В лобби
        </button>
      </div>
      <div class="row wrap">
        <select class="input" value={map} onChange={(e) => setMap(e.currentTarget.value)}>
          {GAMES.map((g) => (
            <option key={g.id} value={g.id}>
              {g.title} ({g.id})
            </option>
          ))}
        </select>
        <label class="row">
          ботов
          <input
            class="input narrow"
            type="number"
            min={0}
            max={7}
            value={bots}
            onInput={(e) => setBots(Number(e.currentTarget.value))}
          />
        </label>
        <button type="button" class="btn go-btn" onClick={() => run({ c: 'start', games: [map], bots })}>
          Играть карту
        </button>
      </div>
      <div class="row wrap">
        <span>Телепорт:</span>
        <button type="button" class="btn chip" onClick={() => run({ c: 'goto', to: 'spawn' })}>
          старт
        </button>
        {world?.checkpoints.map((_, i) => (
          <button type="button" key={i} class="btn chip" onClick={() => run({ c: 'goto', to: i })}>
            КТ {i}
          </button>
        ))}
        {world?.finish && (
          <button type="button" class="btn chip" onClick={() => run({ c: 'goto', to: 'finish' })}>
            финиш
          </button>
        )}
      </div>
      <div class="row wrap">
        <button type="button" class="btn chip" onClick={() => run({ c: 'bot', near: true })}>
          + бот рядом
        </button>
        <button type="button" class="btn chip" onClick={() => run({ c: 'bots', on: false })}>
          Заморозить ботов
        </button>
        <button type="button" class="btn chip" onClick={() => run({ c: 'bots', on: true })}>
          Разморозить
        </button>
      </div>
      <div class="row wrap">
        <button type="button" class="btn chip" onClick={() => run({ c: 'knock', v: [0, 6, 8] })}>
          Сбить меня
        </button>
        <button type="button" class="btn chip" onClick={() => run({ c: 'kill' })}>
          Упасть
        </button>
        <button type="button" class="btn chip" onClick={() => grab(true)}>
          Схватить ближайшего
        </button>
        <button type="button" class="btn chip" onClick={() => grab(false)}>
          Меня хватают
        </button>
      </div>
      <div class="row wrap">
        <button
          type="button"
          class={`btn chip${debugOverlay.value ? ' on' : ''}`}
          onClick={() => (debugOverlay.value = !debugOverlay.value)}
        >
          Оверлей (F3)
        </button>
        <button
          type="button"
          class={`btn chip${gpu ? ' on' : ''}`}
          onClick={() => setGpu(game.renderer.measureGpu(!gpu))}
          title="EXT_disjoint_timer_query_webgl2"
        >
          Замер GPU
        </button>
        <a class="btn chip" href="debug/" target="_blank" rel="noopener">
          Отладка сервера ↗
        </a>
      </div>
      <ProfilerPanel />
      <p class="muted">
        {info ? `${info.game} · арена ${info.id} · seed ${info.seed}` : ''} {last && <b>{last}</b>}
      </p>
    </div>
  );
}
