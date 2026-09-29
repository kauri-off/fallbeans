import type { Game } from '../game/game';
import { arenaInfo, conn, loadProgress, menuOpen } from '../state';
import { Hud } from './Hud';
import { Menu } from './Menu';
import { Tags } from './Tags';

export function App({ game }: { game: Game }) {
  const c = conn.value;
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
          <h2>Не удалось войти</h2>
          <p>{c.message}</p>
          <button type="button" class="btn primary" onClick={() => location.reload()}>
            Обновить страницу
          </button>
        </div>
      </div>
    );
  const info = arenaInfo.value;
  return (
    <>
      {(c.status === 'connecting' || c.status === 'reconnecting' || !info) && (
        <div class="banner glass">{c.status === 'reconnecting' ? 'Связь потеряна — переподключаемся…' : 'Подключение…'}</div>
      )}
      {info && <Tags game={game} />}
      {info && <Hud />}
      {info && menuOpen.value && <Menu game={game} />}
    </>
  );
}
