import { useState } from 'preact/hooks';
import { ROOM_PIN_DIGITS, ROOM_TITLE_MAX } from '../../../shared/consts';
import type { RoomInfo } from '../../../shared/protocol';
import type { Game } from '../../game/game';
import { settings } from '../../settings';
import { denied, ownRoom, roomList } from '../../state';
import { NameForm, PracticeList, SettingsTab } from '../shared';

/** The first screen: the player's name, the rooms to enter, a room of one's own, and the options. */
export function Home({ game }: { game: Game }) {
  const [tab, setTab] = useState<'rooms' | 'settings'>('rooms');
  return (
    <div class="screen">
      <div class="home glass">
        <div class="row between wrap">
          <h1 class="logo small">Fall Beans</h1>
          <div class="tabs">
            <button type="button" class={`tab${tab === 'rooms' ? ' on' : ''}`} onClick={() => setTab('rooms')}>
              Комнаты
            </button>
            <button type="button" class={`tab${tab === 'settings' ? ' on' : ''}`} onClick={() => setTab('settings')}>
              Настройки
            </button>
          </div>
        </div>
        <div class="menu-body">{tab === 'rooms' ? <RoomsTab game={game} /> : <SettingsTab />}</div>
      </div>
    </div>
  );
}

function RoomsTab({ game }: { game: Game }) {
  const list = roomList.value;
  const no = denied.value;
  // The room that answered "PIN, please" (from the list, or from a link to a room that is not listed).
  const asks = no?.reason === 'pin' ? no.room : null;
  return (
    <div class="stack">
      <NameForm game={game} />
      {no?.msg && no.reason !== 'pin' && <p class="alert">{no.msg}</p>}
      <h3>Комнаты{list?.length ? `: ${list.length}` : ''}</h3>
      {!list ? (
        <p class="muted">Загружаем список…</p>
      ) : list.length === 0 ? (
        <p class="muted">Пока нет ни одной комнаты — создайте свою.</p>
      ) : (
        <ul class="rooms">
          {list.map((r) => (
            <RoomRow key={r.id} game={game} r={r} />
          ))}
        </ul>
      )}
      {asks && !list?.some((r) => r.id === asks) && (
        <div class="group stack">
          <b>🔒 Комната закрыта PIN-кодом</b>
          <PinForm game={game} id={asks} />
        </div>
      )}
      <CreateRoom game={game} />
      <PracticeList />
    </div>
  );
}

function RoomRow({ game, r }: { game: Game; r: RoomInfo }) {
  const mine = r.id === ownRoom.value;
  const no = denied.value;
  const full = r.players >= r.max;
  return (
    <li class={mine ? 'mine' : ''}>
      <div class="row">
        <div class="grow room-name">
          <b>
            {r.private && <span title="Приватная: вход по PIN-коду">🔒 </span>}
            {r.title}
          </b>
          <span class="muted small">
            хост ⭐ {r.host || '—'} · {r.phase === 'lobby' ? 'в лобби' : 'идёт игра'}
            {mine && ' · ваша комната'}
          </span>
        </div>
        <span class="room-count" title={r.bots ? `Игроков: ${r.players}, ботов: ${r.bots}` : `Игроков: ${r.players}`}>
          {r.players}/{r.max}
          {r.bots > 0 && <span class="muted"> +{r.bots} 🤖</span>}
        </span>
        <button type="button" class="btn primary" disabled={full} onClick={() => game.joinRoom(r.id)}>
          {full ? 'Мест нет' : 'Войти'}
        </button>
      </div>
      {no?.reason === 'pin' && no.room === r.id && <PinForm game={game} id={r.id} />}
    </li>
  );
}

/** A private room asked for its PIN: only its host knows it. */
function PinForm({ game, id }: { game: Game; id: string }) {
  const [pin, setPin] = useState('');
  const ready = pin.length === ROOM_PIN_DIGITS;
  return (
    <form
      class="stack"
      onSubmit={(e) => {
        e.preventDefault();
        if (ready) game.joinRoom(id, pin);
      }}
    >
      <div class="row">
        <input
          class="input pin-input"
          autoFocus
          inputMode="numeric"
          autocomplete="off"
          maxLength={ROOM_PIN_DIGITS}
          placeholder="PIN-код"
          value={pin}
          onInput={(e) => setPin(e.currentTarget.value.replace(/\D/g, ''))}
        />
        <button type="submit" class="btn primary" disabled={!ready}>
          Войти
        </button>
        <button type="button" class="btn" onClick={() => (denied.value = null)}>
          Отмена
        </button>
      </div>
      <span class={denied.value?.msg ? 'alert' : 'muted small'}>{denied.value?.msg || 'PIN-код спросите у хоста комнаты.'}</span>
    </form>
  );
}

/** Everyone may keep one room of their own: they are its host whenever they are in it. */
function CreateRoom({ game }: { game: Game }) {
  const mine = ownRoom.value;
  const [title, setTitle] = useState('');
  const [isPrivate, setPrivate] = useState(false);
  if (mine)
    return (
      <div class="group stack">
        <button type="button" class="btn go-btn" onClick={() => game.joinRoom(mine)}>
          Вернуться в свою комнату
        </button>
        <p class="muted small">
          Своя комната у каждого одна. Пока вас нет, хостом в ней кто-то из оставшихся; вернётесь — роль снова ваша.
        </p>
      </div>
    );
  const name = settings.value.name;
  return (
    <form
      class="group stack"
      onSubmit={(e) => {
        e.preventDefault();
        game.createRoom(title, isPrivate);
      }}
    >
      <h3>Своя комната</h3>
      <input
        class="input"
        maxLength={ROOM_TITLE_MAX}
        placeholder={name ? `Комната ${name}` : 'Название комнаты'}
        value={title}
        onInput={(e) => setTitle(e.currentTarget.value)}
      />
      <label class="row">
        <input type="checkbox" checked={isPrivate} onChange={(e) => setPrivate(e.currentTarget.checked)} />
        Приватная: вход по PIN-коду (вы увидите его в меню комнаты)
      </label>
      <button type="submit" class="btn go-btn">
        Создать комнату
      </button>
    </form>
  );
}
