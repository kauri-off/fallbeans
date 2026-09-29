import type { Game } from '../game/game';
import { arenaInfo, conn, loadProgress, lobby, paused, settingsOpen } from '../state';
import { Hud } from './Hud';
import { Lobby } from './Lobby';
import { PauseMenu, SettingsModal } from './Menus';

export function App({ game }: { game: Game }) {
  const c = conn.value;
  if (c.status === 'loading')
    return (
      <div class="screen">
        <div class="card center">
          <h1 class="logo">Fall Beans</h1>
          <div class="bar">
            <div style={{ width: `${Math.round(loadProgress.value * 100)}%` }} />
          </div>
          <p class="muted">Загрузка моделей…</p>
        </div>
      </div>
    );
  if (c.status === 'rejected')
    return (
      <div class="screen">
        <div class="card center">
          <h2>Не удалось войти</h2>
          <p>{c.message}</p>
          <button type="button" class="btn primary" onClick={() => location.reload()}>
            Обновить страницу
          </button>
        </div>
      </div>
    );
  const info = arenaInfo.value;
  const inLobby = !info || info.kind === 'lobby' || lobby.value?.phase === 'lobby';
  return (
    <>
      {(c.status === 'connecting' || c.status === 'reconnecting' || !info) && (
        <div class="banner">{c.status === 'reconnecting' ? 'Связь потеряна — переподключаемся…' : 'Подключение…'}</div>
      )}
      {info && <Hud />}
      {info && inLobby && lobby.value?.phase === 'lobby' && <Lobby game={game} />}
      {info && paused.value && !inLobby && !settingsOpen.value && <PauseMenu game={game} />}
      {settingsOpen.value && <SettingsModal />}
    </>
  );
}
