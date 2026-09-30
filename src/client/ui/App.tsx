import { useEffect } from 'preact/hooks';
import type { Game } from '../game/game';
import { arenaInfo, conn, debugOverlay, loadProgress, menuOpen, practiceGame, roomList, uiHidden } from '../state';
import { DebugOverlay } from './DebugOverlay';
import { Home } from './home/Home';
import { Chat } from './hud/Chat';
import { Hud } from './hud/Hud';
import { Tags } from './hud/Tags';
import { Menu } from './menu/Menu';

/** Loading → the room list (home) → a room: HUD, chat and the Esc menu over the 3D scene. */
export function App({ game }: { game: Game }) {
  const c = conn.value;
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.code !== 'F3') return;
      e.preventDefault();
      debugOverlay.value = !debugOverlay.value;
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);
  const overlay = debugOverlay.value && game && <DebugOverlay game={game} />;
  if (uiHidden.value) return overlay || null;
  if (c.status === 'loading')
    return (
      <div class="screen">
        <div class="glass card center">
          <h1 class="logo">Fall Beans</h1>
          <div class="bar">
            <div style={{ width: `${Math.round(loadProgress.value * 100)}%` }} />
          </div>
          <p class="muted">Загрузка…</p>
        </div>
      </div>
    );
  if (c.status === 'rejected')
    return (
      <div class="screen">
        <div class="glass card center">
          <h1 class="logo small">Fall Beans</h1>
          <p>{c.message}</p>
          <button type="button" class="btn primary" onClick={() => location.reload()}>
            Обновить страницу
          </button>
        </div>
      </div>
    );
  const info = arenaInfo.value;
  // In no room (and not in practice): the room list, as soon as the server has sent it.
  const home = !info && !practiceGame.value && roomList.value !== null;
  return (
    <>
      {home && <Home game={game} />}
      {(c.status === 'connecting' || c.status === 'reconnecting' || (!info && !home)) && (
        <div class="banner glass">{c.status === 'reconnecting' ? 'Связь потеряна — переподключаемся…' : 'Подключение…'}</div>
      )}
      {info && <Tags game={game} />}
      {info && <Hud />}
      {info && !info.practice && <Chat game={game} />}
      {info && menuOpen.value && <Menu game={game} />}
      {overlay}
    </>
  );
}
