import type { Game } from '../../game/game';
import { settings, updateSettings } from '../../settings';
import { devMode, menuTab } from '../../state';
import { SettingsTab } from '../shared';
import { DevTab } from './DevTab';
import { RoomTab } from './RoomTab';

/**
 * The Esc menu: the room (players, access, game setup for the host), options, dev tools. Two
 * layouts, the player's choice (kept in the browser): a column beside the player panel with the
 * game in full view, or a wide two-column panel in the middle over the dimmed game.
 */
export function Menu({ game }: { game: Game }) {
  const tab = menuTab.value;
  const layout = settings.value.menuLayout;
  const center = layout === 'center';
  return (
    <div
      class={`menu-wrap ${layout}`}
      onClick={(e) => {
        if (center && e.target === e.currentTarget) game.resume();
      }}
    >
      <div class="menu glass">
        <div class="row between wrap">
          <h1 class="logo small">Fall Beans</h1>
          <LayoutToggle />
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
          {tab === 'game' ? <RoomTab game={game} /> : tab === 'dev' && devMode.value ? <DevTab game={game} /> : <SettingsTab />}
        </div>
        <button type="button" class="btn primary" onClick={() => game.resume()}>
          Продолжить <span class="muted-inv">· Esc или щелчок {center ? 'вне меню' : 'по полю'}</span>
        </button>
      </div>
    </div>
  );
}

/** Switches the menu between its two layouts. */
export function LayoutToggle({ labels = false }: { labels?: boolean }) {
  const layout = settings.value.menuLayout;
  return (
    <div class="tabs" title="Вид меню">
      {(
        [
          ['side', '◧', 'Сбоку'],
          ['center', '▣', 'По центру'],
        ] as const
      ).map(([id, icon, label]) => (
        <button
          type="button"
          key={id}
          class={`tab${layout === id ? ' on' : ''}`}
          title={`Меню ${label.toLowerCase()}`}
          onClick={() => updateSettings({ menuLayout: id })}
        >
          {icon}
          {labels && ` ${label}`}
        </button>
      ))}
    </div>
  );
}
