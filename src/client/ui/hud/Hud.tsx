import { getGame } from '../../../games';
import { GENRE_LABEL } from '../../../shared/game';
import type { LobbyPlayer } from '../../../shared/protocol';
import { settings } from '../../settings';
import { arenaInfo, conn, feed, gameEnd, hud, lobby, menuOpen, myId, needClick, results } from '../../state';
import { cause, fmtTime, ordinal, signed } from '../labels';
import { Keys } from './Keys';

const playersById = () => new Map((lobby.value?.players ?? []).map((p) => [p.id, p]));

function Name({ p, id }: { p: LobbyPlayer | undefined; id: number }) {
  return (
    <span class={`who${id === myId.value ? ' me' : ''}`}>
      <i class="dot" style={{ background: p?.color ?? '#ccc' }} />
      {p?.name ?? `#${id}`}
    </span>
  );
}

/** Top left: round, players with their status, score and ping, connection. */
function Panel() {
  const info = arenaInfo.value!;
  const l = lobby.value;
  const h = hud.value;
  const c = conn.value;
  const g = getGame(info.game);
  const round = info.kind === 'round';
  const players = [...(l?.players ?? [])].sort((a, b) =>
    info.kind === 'lobby' ? b.crowns - a.crowns || a.id - b.id : b.score - a.score || a.id - b.id,
  );
  const points = g?.genre === 'points';
  return (
    <div class="panel glass">
      <div class="panel-head">
        {round && g ? (
          <>
            <span class={`genre g-${g.genre}`}>{info.practice ? 'Тренировка' : `${info.index}/${info.total}`}</span>
            <b>{g.title}</b>
          </>
        ) : (
          <b class="room-title">{info.kind === 'podium' ? 'Итоги игры' : l?.room.title || 'Лобби'}</b>
        )}
      </div>
      <ul class="roster">
        {players.map((p) => {
          const r = h.roster[p.id];
          const icon = !round ? '' : !r ? '👁' : r.status === 'finished' ? `🏁${r.place}` : r.status === 'out' ? '✖' : '';
          return (
            <li key={p.id} class={`${r?.status === 'out' ? 'dim' : ''}${p.connected ? '' : ' off'}`}>
              <Name p={p} id={p.id} />
              {p.id === l?.host && <span title="Хост">⭐</span>}
              {info.kind === 'lobby' && p.crowns > 0 && <span title="Победы">👑{p.crowns}</span>}
              <span class="st">{icon}</span>
              {round && points && <span class="rs">{h.roundScores[p.id] ?? 0}</span>}
              {info.kind !== 'lobby' && <b class="sc">{p.score}</b>}
              <span class="ping">{p.bot ? 'бот' : p.connected ? `${p.ping}` : '—'}</span>
            </li>
          );
        })}
      </ul>
      <div class="net">
        <span class={c.transport ?? ''} title={c.transport === 'wt' ? 'WebTransport (UDP)' : 'WebSocket'}>
          {c.transport === 'wt' ? 'UDP' : 'TCP'}
        </span>{' '}
        · {c.ping} мс
        {settings.value.showFps && ` · ${h.fps} к/с · ${h.drawCalls} выз. отрисовки`}
      </div>
    </div>
  );
}

/** Top right: who fell and why. */
function Feed() {
  const ps = playersById();
  return (
    <div class="feed">
      {feed.value.map((f) => {
        if (f.text)
          return (
            <div key={f.id} class="glass">
              {f.text}
            </div>
          );
        const c = cause(f.cause);
        return (
          <div key={f.id} class={`glass${f.out ? ' out' : ''}`}>
            {f.by !== null && f.by !== f.victim && (
              <>
                <Name p={ps.get(f.by)} id={f.by} />
                <span class="how" title={c.text}>
                  {c.icon}
                </span>
              </>
            )}
            {(f.by === null || f.by === f.victim) && (
              <span class="how" title={c.text}>
                {c.icon}
              </span>
            )}
            <Name p={ps.get(f.victim)} id={f.victim} />
            <span class="why">{f.shortcut ? 'срезка пути: −2' : f.out ? `выбывает (${c.text})` : c.text}</span>
          </div>
        );
      })}
    </div>
  );
}

/** Before the start: a compact line at the top (the camera flies over the course meanwhile). */
function Intro() {
  const info = arenaInfo.value!;
  const h = hud.value;
  const g = getGame(info.game);
  if (!g) return null;
  const left = Math.ceil(-h.t);
  return (
    <>
      <div class="intro glass">
        <div class="row">
          <span class={`genre g-${g.genre}`}>{GENRE_LABEL[g.genre]}</span>
          {!info.practice && (
            <span class="muted">
              Раунд {info.index} из {info.total}
            </span>
          )}
        </div>
        <h2>{g.title}</h2>
        <p>
          🎯 {g.goal}. <span class="muted">{g.desc}</span>
        </p>
      </div>
      {left <= 3 && left >= 1 && (
        <div class="count" key={left}>
          {left}
        </div>
      )}
    </>
  );
}

/** After a round: points won and lost, compact, at the top. */
function Results() {
  const r = results.value!;
  const ps = playersById();
  const g = getGame(r.game);
  return (
    <div class="sheet glass">
      <div class="sheet-head">
        <b>{g?.title ?? r.game}</b>
        <span class="muted">{r.practice ? 'тренировка' : `итоги раунда ${r.index} из ${r.total}`}</span>
      </div>
      <table>
        <tbody>
          {r.rows.map((e) => (
            <tr key={e.id} class={e.id === myId.value ? 'me' : ''}>
              <td class="pl">{e.place}</td>
              <td class="nm">
                <Name p={ps.get(e.id)} id={e.id} />
              </td>
              <td class="muted nt">{e.afk ? 'AFK' : e.note}</td>
              <td class="pts">+{e.points}</td>
              <td class="pen">{e.penalty ? `−${e.penalty}` : ''}</td>
              <td class={`dl ${e.delta > 0 ? 'up' : e.delta < 0 ? 'down' : ''}`}>{signed(e.delta)}</td>
              <td class="tot">{e.total}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** End of the game: final table and titles, beside the podium. */
function Summary() {
  const s = gameEnd.value!;
  const ps = playersById();
  const win = s.standings[0];
  return (
    <div class="summary glass">
      <div class="crown">👑</div>
      <h2>{win ? (win.id === myId.value ? 'Вы победили!' : `${win.name} побеждает!`) : 'Игра окончена'}</h2>
      <table>
        <tbody>
          {s.standings.map((e) => (
            <tr key={e.id} class={e.id === myId.value ? 'me' : ''}>
              <td class="pl">{e.place === 1 ? '🥇' : e.place === 2 ? '🥈' : e.place === 3 ? '🥉' : e.place}</td>
              <td class="nm">
                <Name p={ps.get(e.id) ?? ({ name: e.name, color: e.color } as LobbyPlayer)} id={e.id} />
              </td>
              <td class="muted">🏆{e.wins}</td>
              <td class="tot">{e.total}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {s.awards.length > 0 && (
        <div class="awards">
          {s.awards.map((a) => (
            <div key={a.key} class="award">
              <span class="ai">{a.icon}</span>
              <div>
                <b>{a.title}</b>
                <div>
                  <Name p={ps.get(a.id)} id={a.id} /> <span class="muted">{a.text}</span>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Status() {
  const h = hud.value;
  const spect = h.spectating ? `Камера: ${h.spectating}` : 'Камера: обзор арены';
  const hint = <span class="muted small">ЛКМ/ПКМ или ←/→ — сменить игрока</span>;
  if (h.status === 'finished')
    return (
      <div class="status glass ok">
        🏁 Финиш{h.place > 0 ? `: ${ordinal(h.place)} место` : ''}! · {spect} {hint}
      </div>
    );
  if (h.status === 'out')
    return (
      <div class="status glass bad">
        Вы выбыли · {spect} {hint}
      </div>
    );
  if (h.status === 'spectating')
    return (
      <div class="status glass">
        👁 Вы зритель · {spect} {hint}
      </div>
    );
  return null;
}

export function Hud() {
  const info = arenaInfo.value!;
  const h = hud.value;
  const round = info.kind === 'round';
  const g = getGame(info.game);
  const between = !!results.value || !!gameEnd.value;
  return (
    <div class="hud">
      <Panel />
      <Feed />
      {round && h.t < 0 && !between && <Intro />}
      {round && h.t >= 0 && !between && (
        <div class="top">
          <div class={`timer${h.timeLeft < 10 ? ' hurry' : ''}`}>{fmtTime(h.timeLeft)}</div>
          {h.mapText && <div class="pill glass">{h.mapText}</div>}
          {h.bonus && <div class="pill glass bonus">{h.bonus}</div>}
          {h.t < 1.2 && <div class="go">ВПЕРЁД!</div>}
        </div>
      )}
      {round && !between && <Status />}
      {round && h.status === 'play' && h.t >= 0 && h.t < 10 && (
        <Keys
          grab={g?.grab ? 'схватить хвост' : 'захват'}
          chat={!info.practice}
          lead={
            <>
              <kbd>Esc</kbd> меню ·{' '}
            </>
          }
        />
      )}
      {info.kind === 'lobby' && !menuOpen.value && (
        <Keys
          grab="захват"
          lead={
            <>
              <kbd>Esc</kbd> меню{lobby.value?.host === myId.value ? ' и запуск игры' : ''} ·{' '}
            </>
          }
        />
      )}
      {needClick.value && !menuOpen.value && (
        <div class="prompt">
          🖱 Щёлкните по полю, чтобы вернуть управление
          <div class="small">
            или нажмите <kbd>Esc</kbd> для меню
          </div>
        </div>
      )}
      {results.value && <Results />}
      {gameEnd.value && <Summary />}
    </div>
  );
}
