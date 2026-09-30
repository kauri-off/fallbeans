import { useState } from 'preact/hooks';
import { getGame } from '../../../games';
import { BASE_PATH, COLORS } from '../../../shared/consts';
import type { Game } from '../../game/game';
import { arenaInfo, lobby, myId, practiceGame } from '../../state';
import { colorBg } from '../labels';
import { NameForm, PracticeList } from '../shared';
import { HostSetup } from './HostSetup';

/** The room: its name and access, the player's name and colour, who is here, and the game setup (host). */
export function RoomTab({ game }: { game: Game }) {
  const l = lobby.value;
  const me = myId.value;
  if (!l) return null;
  const isHost = l.host === me;
  const mine = l.players.find((p) => p.id === me);
  const send = game.net.send.bind(game.net);
  const inLobby = l.phase === 'lobby';
  const info = arenaInfo.value;

  if (practiceGame.value) {
    // Practice was started from a room: the way back leads there.
    const from = new URLSearchParams(location.search).get('from');
    return (
      <div class="stack">
        <p>
          Тренировка: <b>«{getGame(practiceGame.value)?.title ?? practiceGame.value}»</b>. Раунд повторяется с ботами, пока вы не
          вернётесь.
        </p>
        <a class="btn" href={from ? `${BASE_PATH}?room=${encodeURIComponent(from)}` : BASE_PATH}>
          ← {from ? 'Вернуться в комнату' : 'К списку комнат'}
        </a>
      </div>
    );
  }

  const profile = (
    <div class="group stack">
      <NameForm game={game} />
      {inLobby && (
        <div class="swatches">
          {COLORS.map((c) => {
            const taken = l.players.some((p) => p.color === c && p.id !== me);
            return (
              <button
                type="button"
                key={c}
                class={`swatch${mine?.color === c ? ' on' : ''}`}
                style={{ background: colorBg(c) }}
                disabled={taken}
                title={taken ? 'Цвет занят' : 'Выбрать цвет'}
                onClick={() => send({ t: 'color', c })}
              />
            );
          })}
        </div>
      )}
    </div>
  );

  if (!inLobby)
    return (
      <div class="stack">
        <RoomHead game={game} />
        {profile}
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
    );

  return (
    <div class="room-grid">
      <div class="stack">
        <h3>
          Игроки: {l.players.length} из {l.max}
        </h3>
        <ul class="players">
          {l.players.map((p) => (
            <li key={p.id} class={p.connected ? '' : 'dim'}>
              <i class="dot" style={{ background: colorBg(p.color) }} />
              <span class="grow">
                {p.name}
                {p.id === me && ' (вы)'}
              </span>
              {p.id === l.host && <span title="Хост">⭐</span>}
              {p.crowns > 0 && <span title="Победы">👑{p.crowns}</span>}
              {p.bot ? (
                // (With "fill with bots" on, a bot taken out would be replaced at once.)
                isHost &&
                !l.fill && (
                  <button type="button" class="btn tiny" title="Убрать бота" onClick={() => send({ t: 'removeBot', id: p.id })}>
                    ✕
                  </button>
                )
              ) : (
                <>
                  {isHost && p.id !== me && p.connected && (
                    <button
                      type="button"
                      class="btn tiny"
                      title="Передать роль хоста этому игроку"
                      onClick={() => send({ t: 'host', id: p.id })}
                    >
                      Отдать хоста
                    </button>
                  )}
                  <span class="ping">{p.connected ? `${p.ping} мс` : 'нет связи'}</span>
                </>
              )}
            </li>
          ))}
        </ul>
      </div>
      <div class="stack">
        <RoomHead game={game} />
        {profile}
        {isHost ? <HostSetup game={game} /> : <p class="muted">Игру запускает хост ⭐</p>}
        <PracticeList />
      </div>
    </div>
  );
}

/** The room's name, a link to invite with, the way out; for the host: private or public, and the PIN. */
function RoomHead({ game }: { game: Game }) {
  const l = lobby.value!;
  const isHost = l.host === myId.value;
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard
      ?.writeText(location.href)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };
  return (
    <div class="group stack">
      <div class="row">
        <b class="grow room-title" title={`Комната ${l.room.id.toUpperCase()}`}>
          {l.room.private && '🔒 '}
          {l.room.title}
        </b>
        <button type="button" class="btn tiny" title="Скопировать ссылку на комнату" onClick={copy}>
          {copied ? 'Скопировано' : '🔗 Ссылка'}
        </button>
        <button type="button" class="btn tiny danger" onClick={() => game.leaveRoom()}>
          Выйти из комнаты
        </button>
      </div>
      {isHost ? (
        <label class="row">
          <input
            type="checkbox"
            checked={l.room.private}
            onChange={(e) => game.net.send({ t: 'access', private: e.currentTarget.checked })}
          />
          <span>
            Приватная комната
            {l.pin ? (
              <>
                {' '}
                — PIN-код для входа: <b class="pin">{l.pin}</b>
              </>
            ) : (
              <span class="muted"> (вход по PIN-коду)</span>
            )}
          </span>
        </label>
      ) : (
        l.room.private && <span class="muted small">Приватная комната: PIN-код для друзей знает хост ⭐</span>
      )}
    </div>
  );
}
