import { useState } from 'preact/hooks';
import { FINALS, GAMES } from '../../games';
import { BASE_PATH, COLORS, MAX_PLAYERS } from '../../shared/consts';
import { GENRE_LABEL } from '../../shared/game';
import type { Playlist } from '../../shared/protocol';
import type { Game } from '../game/game';
import { settings, updateSettings } from '../settings';
import { lobby, myId, panelOpen, practiceGame, settingsOpen } from '../state';

const MODES: { id: Playlist['mode']; label: string }[] = [
  { id: 'mix', label: 'Микс' },
  { id: 'races', label: 'Гонки' },
  { id: 'survival', label: 'Выживание' },
  { id: 'custom', label: 'Свой список' },
];

export function Lobby({ game }: { game: Game }) {
  const l = lobby.value;
  const me = myId.value;
  const [name, setName] = useState(settings.value.name);
  if (!l) return null;
  const isHost = l.host === me;
  const mine = l.players.find((p) => p.id === me);
  const send = game.net.send.bind(game.net);
  const pl = l.playlist;
  const setPl = (patch: Partial<Playlist>) => send({ t: 'playlist', pl: { ...pl, ...patch } });
  const enough = l.players.length >= l.min;

  if (practiceGame.value)
    return (
      <div class="hint-bottom">
        Тренировка: {GAMES.find((g) => g.id === practiceGame.value)?.title ?? practiceGame.value} ·{' '}
        <a href={BASE_PATH}>вернуться в лобби</a>
      </div>
    );

  if (!panelOpen.value)
    return (
      <div class="hint-bottom">
        <button type="button" class="btn small" onClick={() => (panelOpen.value = true)}>
          Открыть лобби
        </button>
        <span>Бегайте по площадке: WASD, мышь, пробел. Esc — меню.</span>
      </div>
    );

  return (
    <div class="lobby">
      <div class="card lobby-main">
        <div class="row between">
          <h1 class="logo small">Fall Beans</h1>
          <button type="button" class="btn icon" title="Настройки" onClick={() => (settingsOpen.value = true)}>
            ⚙
          </button>
        </div>

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
          <input
            class="input"
            maxLength={16}
            value={name}
            placeholder="Ваше имя"
            onInput={(e) => setName(e.currentTarget.value)}
          />
          <button type="submit" class="btn">
            Сохранить
          </button>
        </form>

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

        <h3>
          Игроки {l.players.length}/{MAX_PLAYERS}
        </h3>
        <ul class="players">
          {l.players.map((p) => (
            <li key={p.id} class={p.connected ? '' : 'muted'}>
              <span class="dot" style={{ background: p.color }} />
              <span class="grow">
                {p.name}
                {p.id === me && ' (вы)'}
              </span>
              {p.id === l.host && <span title="Хост">⭐</span>}
              {p.crowns > 0 && <span title="Короны">👑{p.crowns}</span>}
              {p.bot ? (
                isHost && (
                  <button type="button" class="btn tiny" onClick={() => send({ t: 'removeBot', id: p.id })}>
                    ✕
                  </button>
                )
              ) : (
                <span class="ping">{p.connected ? `${p.ping} мс` : 'нет связи'}</span>
              )}
            </li>
          ))}
        </ul>

        {isHost ? (
          <div class="host">
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
            {pl.mode === 'custom' && (
              <div class="row wrap">
                {GAMES.filter((g) => g.genre !== 'final').map((g) => {
                  const on = pl.games.includes(g.id);
                  return (
                    <button
                      type="button"
                      key={g.id}
                      class={`btn chip${on ? ' on' : ''}`}
                      title={g.desc}
                      onClick={() => setPl({ games: on ? pl.games.filter((x) => x !== g.id) : [...pl.games, g.id].slice(0, 8) })}
                    >
                      {g.title}
                    </button>
                  );
                })}
              </div>
            )}
            <label class="row">
              <span>Финал:</span>
              <select class="input" value={pl.final} onChange={(e) => setPl({ final: e.currentTarget.value })}>
                <option value="random">Случайный</option>
                {FINALS.map((g) => (
                  <option key={g.id} value={g.id}>
                    {g.title}
                  </option>
                ))}
              </select>
            </label>
            <div class="row">
              <button type="button" class="btn" disabled={l.players.length >= l.max} onClick={() => send({ t: 'addBot' })}>
                + Бот
              </button>
              <button type="button" class="btn primary grow" disabled={!enough} onClick={() => send({ t: 'start' })}>
                {enough ? 'Начать шоу' : `Нужно игроков: ${l.min}`}
              </button>
            </div>
          </div>
        ) : (
          <p class="muted">Шоу запускает хост ⭐</p>
        )}

        <div class="row between">
          <button
            type="button"
            class="btn"
            onClick={() => {
              panelOpen.value = false;
              game.capture();
            }}
          >
            Побегать
          </button>
          <details class="practice">
            <summary>Тренировка</summary>
            <div class="row wrap">
              {GAMES.map((g) => (
                <a key={g.id} class="btn chip" href={`${BASE_PATH}?practice=${g.id}`} title={g.desc}>
                  {g.title} · {GENRE_LABEL[g.genre]}
                </a>
              ))}
            </div>
          </details>
        </div>
      </div>
    </div>
  );
}
