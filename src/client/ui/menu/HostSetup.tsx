import { GAMES } from '../../../games';
import { type Playlist, ROUND_COUNTS } from '../../../shared/protocol';
import type { Game } from '../../game/game';
import { lobby } from '../../state';
import { plural } from '../labels';

const MODES: { id: Playlist['mode']; label: string }[] = [
  { id: 'mix', label: 'Микс' },
  { id: 'races', label: 'Гонки' },
  { id: 'survival', label: 'Выживание' },
  { id: 'custom', label: 'Свой список' },
];

/** The host's part of the lobby: which maps, how many rounds, bots, and the start button. */
export function HostSetup({ game }: { game: Game }) {
  const l = lobby.value!;
  const pl = l.playlist;
  const send = game.net.send.bind(game.net);
  const setPl = (patch: Partial<Playlist>) => send({ t: 'playlist', pl: { ...pl, ...patch } });
  const ready = l.players.filter((p) => p.connected || p.bot).length;
  const enough = ready >= l.min;
  return (
    <div class="group stack">
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
      <label class="row" title="Боты занимают все свободные места и уступают их приходящим игрокам">
        <input type="checkbox" checked={l.fill} onChange={(e) => send({ t: 'fill', on: e.currentTarget.checked })} />
        Заполнять свободные места ботами
      </label>
      <div class="row">
        <button type="button" class="btn" disabled={l.fill || l.players.length >= l.max} onClick={() => send({ t: 'addBot' })}>
          + Добавить бота
        </button>
        <button type="button" class="btn go-btn grow" disabled={!enough} onClick={() => send({ t: 'start' })}>
          {enough ? 'Начать игру' : `Нужно хотя бы ${plural(l.min, 'игрок', 'игрока', 'игроков')}`}
        </button>
      </div>
    </div>
  );
}
