import type { Game } from '../../game/game';
import { devMode, menuTab } from '../../state';
import { SettingsTab } from '../shared';
import { DevTab } from './DevTab';
import { RoomTab } from './RoomTab';

/** The Esc menu: the room (players, access, game setup for the host), options, dev tools. */
export function Menu({ game }: { game: Game }) {
  const tab = menuTab.value;
  return (
    <div class="menu-wrap">
      <div class="menu glass">
        <div class="row between wrap">
          <h1 class="logo small">Fall Beans</h1>
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
          Продолжить <span class="muted-inv">· Esc или щелчок по полю</span>
        </button>
      </div>
    </div>
  );
}
