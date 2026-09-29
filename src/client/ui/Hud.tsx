import { getGame } from '../../games';
import { GENRE_LABEL } from '../../shared/game';
import { settings } from '../settings';
import { arenaInfo, conn, feed, hud, lobby, myId, results, scoreboard, winner } from '../state';

const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;

function Intro() {
  const info = arenaInfo.value!;
  const h = hud.value;
  const g = getGame(info.game);
  if (!g) return null;
  const left = Math.ceil(-h.t);
  const n = info.participants.length;
  return (
    <div class="intro">
      <div class="card intro-card">
        {!info.practice && (
          <div class="muted">
            Раунд {info.index} из {info.total}
          </div>
        )}
        <div class={`genre g-${g.genre}`}>{GENRE_LABEL[g.genre]}</div>
        <h1>{g.title}</h1>
        <p>{g.desc}</p>
        <p class="goal">🎯 {g.goal}</p>
        {info.eliminate > 0 && (
          <p class="muted">
            Проходят {info.qualify} из {n}
          </p>
        )}
      </div>
      {left <= 3 && left >= 1 && <div class="count">{left}</div>}
    </div>
  );
}

function Results() {
  const r = results.value!;
  const players = new Map((lobby.value?.players ?? []).map((p) => [p.id, p]));
  const g = getGame(r.game);
  return (
    <div class="screen dim">
      <div class="card results">
        <h2>{g?.title ?? r.game}: итоги</h2>
        <ol>
          {r.ranking.map((e) => {
            const p = players.get(e.id);
            return (
              <li key={e.id} class={e.ok ? 'ok' : 'bad'}>
                <span class="dot" style={{ background: p?.color ?? '#ccc' }} />
                <span class="grow">
                  {p?.name ?? `#${e.id}`}
                  {e.id === myId.value && ' (вы)'}
                </span>
                <span class="muted">{e.note}</span>
                <span>{e.ok ? '✔ прошёл' : '✖ выбыл'}</span>
              </li>
            );
          })}
        </ol>
      </div>
    </div>
  );
}

function Scoreboard() {
  const l = lobby.value;
  if (!l) return null;
  return (
    <div class="screen dim">
      <div class="card results">
        <h2>Игроки</h2>
        <ol>
          {[...l.players]
            .sort((a, b) => b.score - a.score)
            .map((p) => (
              <li key={p.id} class={p.alive ? '' : 'muted'}>
                <span class="dot" style={{ background: p.color }} />
                <span class="grow">{p.name}</span>
                <span>{p.score} очк.</span>
                <span>👑 {p.crowns}</span>
                <span class="ping">{p.bot ? 'бот' : `${p.ping} мс`}</span>
              </li>
            ))}
        </ol>
      </div>
    </div>
  );
}

export function Hud() {
  const info = arenaInfo.value!;
  const h = hud.value;
  const c = conn.value;
  const w = winner.value;
  const round = info.kind === 'round';
  return (
    <div class="hud">
      <div class="corner">
        <span class={`net ${c.transport ?? ''}`} title={c.transport === 'wt' ? 'WebTransport (UDP)' : 'WebSocket'}>
          {c.transport === 'wt' ? 'UDP' : 'TCP'} · {c.ping} мс
        </span>
        {settings.value.showFps && (
          <span>
            {' '}
            · {h.fps} FPS · {h.drawCalls} dc
          </span>
        )}
      </div>

      {round && h.t < 0 && !results.value && <Intro />}

      {round && h.t >= 0 && !results.value && (
        <div class="top">
          <div class="timer">{fmt(h.timeLeft)}</div>
          {info.eliminate > 0 && getGame(info.game)?.genre === 'race' && (
            <div class="pill">
              Финишировали {h.finished}/{info.qualify}
            </div>
          )}
          {info.eliminate > 0 && getGame(info.game)?.genre !== 'race' && (
            <div class="pill">
              Выбыло {h.out}/{info.eliminate}
            </div>
          )}
          {h.mapText && <div class="pill big">{h.mapText}</div>}
          {h.t < 1.2 && <div class="go">ВПЕРЁД!</div>}
        </div>
      )}

      {round && h.status === 'finished' && !results.value && (
        <div class="status ok">
          Финиш{h.place > 0 ? ` #${h.place}` : ''}! {h.spectating && `Смотрим: ${h.spectating}`}
        </div>
      )}
      {round && h.status === 'out' && !results.value && (
        <div class="status bad">Вы выбыли. {h.spectating && `Смотрим: ${h.spectating}`}</div>
      )}
      {round && h.status === 'spectating' && !results.value && (
        <div class="status">Вы зритель{h.spectating && ` · смотрим: ${h.spectating}`}</div>
      )}

      <div class="feed">
        {feed.value.map((f) => (
          <div key={f.id}>{f.text}</div>
        ))}
      </div>

      {round && h.status === 'play' && h.t >= 0 && (
        <div class="keys">
          WASD · мышь · Пробел прыжок · E/ЛКМ нырок{getGame(info.game)?.grab ? ' · Q/ПКМ схватить' : ''} · 1-3 эмоции · Tab игроки
        </div>
      )}

      {results.value && <Results />}
      {w && (
        <div class="screen">
          <div class="card center winner">
            <div class="crown">👑</div>
            <h1>{w.name}</h1>
            <p>{w.id === myId.value ? 'Вы победили!' : 'забирает корону!'}</p>
          </div>
        </div>
      )}
      {scoreboard.value && <Scoreboard />}
    </div>
  );
}
