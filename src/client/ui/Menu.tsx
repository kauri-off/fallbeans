import { useState } from 'preact/hooks';
import { GAMES, getGame } from '../../games';
import { BASE_PATH, COLORS, MAX_PLAYERS } from '../../shared/consts';
import { GENRE_LABEL } from '../../shared/game';
import { type Playlist, ROUND_COUNTS } from '../../shared/protocol';
import type { Game } from '../game/game';
import type { Quality } from '../game/renderer';
import { settings, updateSettings } from '../settings';
import { arenaInfo, devMode, lobby, menuTab, myId, practiceGame } from '../state';
import { DevTab } from './DevTab';
import { plural } from './labels';

const MODES: { id: Playlist['mode']; label: string }[] = [
  { id: 'mix', label: 'Микс' },
  { id: 'races', label: 'Гонки' },
  { id: 'survival', label: 'Выживание' },
  { id: 'custom', label: 'Свой список' },
];

const QUALITIES: { id: Quality; label: string }[] = [
  { id: 'medium', label: 'Среднее' },
  { id: 'high', label: 'Высокое' },
  { id: 'ultra', label: 'Ультра' },
];

/** The Esc menu: lobby and game setup (host), current game, and options. */
export function Menu({ game }: { game: Game }) {
  const tab = menuTab.value;
  return (
    <div class="menu-wrap">
      <div class="menu glass">
        <div class="row between">
          <h1 class="logo small">Fall Beans</h1>
          <div class="tabs">
            <button type="button" class={`tab${tab === 'game' ? ' on' : ''}`} onClick={() => (menuTab.value = 'game')}>
              Игра
            </button>
            <button type="button" class={`tab${tab === 'settings' ? ' on' : ''}`} onClick={() => (menuTab.value = 'settings')}>
              Настройки
            </button>
            {devMode.value && (
              <button type="button" class={`tab${tab === 'dev' ? ' on' : ''}`} onClick={() => (menuTab.value = 'dev')}>
                Dev
              </button>
            )}
          </div>
        </div>
        <div class="menu-body">
          {tab === 'game' ? <GameTab game={game} /> : tab === 'dev' && devMode.value ? <DevTab game={game} /> : <SettingsTab />}
        </div>
        <button type="button" class="btn primary" onClick={() => game.resume()}>
          Продолжить <span class="muted-inv">· Esc или щелчок по полю</span>
        </button>
      </div>
    </div>
  );
}

function GameTab({ game }: { game: Game }) {
  const l = lobby.value;
  const me = myId.value;
  const [name, setName] = useState(settings.value.name);
  if (!l) return null;
  const isHost = l.host === me;
  const mine = l.players.find((p) => p.id === me);
  const send = game.net.send.bind(game.net);
  const inLobby = l.phase === 'lobby';
  const info = arenaInfo.value;

  if (practiceGame.value)
    return (
      <div class="stack">
        <p>
          Тренировка: <b>«{getGame(practiceGame.value)?.title ?? practiceGame.value}»</b>. Раунд повторяется с ботами, пока вы не
          вернётесь в лобби.
        </p>
        <a class="btn" href={BASE_PATH}>
          ← Вернуться в лобби
        </a>
      </div>
    );

  return (
    <div class="stack">
      <form
        class="row"
        onSubmit={(e) => {
          e.preventDefault();
          const n = name.trim();
          if (!n) return;
          updateSettings({ name: n });
          send({ t: 'name', name: n });
        }}
      >
        <input class="input" maxLength={16} value={name} placeholder="Ваше имя" onInput={(e) => setName(e.currentTarget.value)} />
        <button type="submit" class="btn">
          Сохранить
        </button>
      </form>
      {inLobby && (
        <div class="swatches">
          {COLORS.map((c) => {
            const taken = l.players.some((p) => p.color === c && p.id !== me);
            return (
              <button
                type="button"
                key={c}
                class={`swatch${mine?.color === c ? ' on' : ''}`}
                style={{ background: c }}
                disabled={taken}
                title={taken ? 'Цвет занят' : 'Выбрать цвет'}
                onClick={() => send({ t: 'color', c })}
              />
            );
          })}
        </div>
      )}

      {inLobby ? (
        <>
          <h3>
            Игроки: {l.players.length} из {MAX_PLAYERS}
          </h3>
          <ul class="players">
            {l.players.map((p) => (
              <li key={p.id} class={p.connected ? '' : 'dim'}>
                <i class="dot" style={{ background: p.color }} />
                <span class="grow">
                  {p.name}
                  {p.id === me && ' (вы)'}
                </span>
                {p.id === l.host && <span title="Хост">⭐</span>}
                {p.crowns > 0 && <span title="Победы">👑{p.crowns}</span>}
                {p.bot ? (
                  isHost && (
                    <button type="button" class="btn tiny" title="Убрать бота" onClick={() => send({ t: 'removeBot', id: p.id })}>
                      ✕
                    </button>
                  )
                ) : (
                  <span class="ping">{p.connected ? `${p.ping} мс` : 'нет связи'}</span>
                )}
              </li>
            ))}
          </ul>
          {isHost ? <HostSetup game={game} /> : <p class="muted">Игру запускает хост ⭐</p>}
          <details class="practice">
            <summary>Тренировка одной карты с ботами</summary>
            <div class="row wrap">
              {GAMES.map((g) => (
                <a key={g.id} class="btn chip" href={`${BASE_PATH}?practice=${g.id}`} title={g.desc}>
                  {g.title} · {GENRE_LABEL[g.genre]}
                </a>
              ))}
            </div>
          </details>
        </>
      ) : (
        <div class="stack">
          <p>
            {info?.kind === 'round'
              ? `Идёт раунд ${info.index} из ${info.total}: «${getGame(info.game)?.title ?? ''}»`
              : l.phase === 'podium'
                ? 'Игра окончена — награждение.'
                : 'Итоги раунда.'}
          </p>
          {isHost && (
            <button type="button" class="btn danger" onClick={() => send({ t: 'abort' })}>
              Прервать игру
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function HostSetup({ game }: { game: Game }) {
  const l = lobby.value!;
  const pl = l.playlist;
  const send = game.net.send.bind(game.net);
  const setPl = (patch: Partial<Playlist>) => send({ t: 'playlist', pl: { ...pl, ...patch } });
  const humans = l.players.filter((p) => p.connected || p.bot).length;
  const enough = humans >= l.min;
  return (
    <div class="host stack">
      <div class="row wrap">
        {MODES.map((m) => (
          <button
            type="button"
            key={m.id}
            class={`btn chip${pl.mode === m.id ? ' on' : ''}`}
            onClick={() => setPl({ mode: m.id })}
          >
            {m.label}
          </button>
        ))}
      </div>
      {pl.mode === 'custom' ? (
        <div class="row wrap">
          {GAMES.map((g) => {
            const i = pl.games.indexOf(g.id);
            return (
              <button
                type="button"
                key={g.id}
                class={`btn chip${i >= 0 ? ' on' : ''}`}
                title={g.desc}
                onClick={() => setPl({ games: i >= 0 ? pl.games.filter((x) => x !== g.id) : [...pl.games, g.id].slice(0, 12) })}
              >
                {i >= 0 && `${i + 1}. `}
                {g.title}
              </button>
            );
          })}
        </div>
      ) : (
        <div class="row">
          <span>Раундов:</span>
          {ROUND_COUNTS.map((n) => (
            <button type="button" key={n} class={`btn chip${pl.rounds === n ? ' on' : ''}`} onClick={() => setPl({ rounds: n })}>
              {n}
            </button>
          ))}
        </div>
      )}
      <div class="row">
        <button type="button" class="btn" disabled={l.players.length >= l.max} onClick={() => send({ t: 'addBot' })}>
          + Добавить бота
        </button>
        <button type="button" class="btn go-btn grow" disabled={!enough} onClick={() => send({ t: 'start' })}>
          {enough ? 'Начать игру' : `Нужно хотя бы ${plural(l.min, 'игрок', 'игрока', 'игроков')}`}
        </button>
      </div>
    </div>
  );
}

function SettingsTab() {
  const s = settings.value;
  const range = (
    label: string,
    key: 'sensitivity' | 'fov' | 'volume',
    min: number,
    max: number,
    step: number,
    fmt: (v: number) => string,
  ) => (
    <label class="field">
      <span>
        {label}: <b>{fmt(s[key])}</b>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={s[key]}
        onInput={(e) => updateSettings({ [key]: Number(e.currentTarget.value) })}
      />
    </label>
  );
  return (
    <div class="stack">
      {range('Чувствительность мыши', 'sensitivity', 0.2, 3, 0.05, (v) => v.toFixed(2))}
      <label class="row">
        <input type="checkbox" checked={s.invertY} onChange={(e) => updateSettings({ invertY: e.currentTarget.checked })} />
        Инвертировать мышь по вертикали
      </label>
      {range('Угол обзора', 'fov', 55, 100, 1, (v) => `${v}°`)}
      {range('Громкость звуков', 'volume', 0, 1, 0.05, (v) => `${Math.round(v * 100)} %`)}
      <div class="field">
        <span>Качество графики:</span>
        <div class="row">
          {QUALITIES.map((q) => (
            <button
              type="button"
              key={q.id}
              class={`btn chip${s.quality === q.id ? ' on' : ''}`}
              onClick={() => updateSettings({ quality: q.id })}
            >
              {q.label}
            </button>
          ))}
        </div>
      </div>
      <label class="row">
        <input type="checkbox" checked={s.showFps} onChange={(e) => updateSettings({ showFps: e.currentTarget.checked })} />
        Показывать частоту кадров
      </label>
      <button
        type="button"
        class="btn danger"
        onClick={async () => {
          await fetch(`${BASE_PATH}api/logout`, { method: 'POST' }).catch(() => {});
          location.replace(`${BASE_PATH}pin/index.html`);
        }}
      >
        Выйти и сбросить PIN-код
      </button>
    </div>
  );
}
