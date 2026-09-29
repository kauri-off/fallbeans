import { BASE_PATH } from '../../shared/consts';
import type { Game } from '../game/game';
import type { Quality } from '../game/renderer';
import { settings, updateSettings } from '../settings';
import { lobby, myId, settingsOpen } from '../state';

export function PauseMenu({ game }: { game: Game }) {
  const isHost = lobby.value?.host === myId.value;
  return (
    <div class="screen dim">
      <div class="card center menu">
        <h2>Пауза</h2>
        <p class="muted">Игра продолжается — щёлкните, чтобы вернуться</p>
        <button type="button" class="btn primary" onClick={() => game.capture()}>
          Продолжить
        </button>
        <button type="button" class="btn" onClick={() => (settingsOpen.value = true)}>
          Настройки
        </button>
        {isHost && lobby.value?.phase !== 'lobby' && (
          <button type="button" class="btn danger" onClick={() => game.net.send({ t: 'abort' })}>
            Прервать шоу (хост)
          </button>
        )}
      </div>
    </div>
  );
}

const QUALITIES: { id: Quality; label: string }[] = [
  { id: 'medium', label: 'Среднее' },
  { id: 'high', label: 'Высокое' },
  { id: 'ultra', label: 'Ультра' },
];

export function SettingsModal() {
  const s = settings.value;
  return (
    <div class="screen dim">
      <div class="card settings">
        <h2>Настройки</h2>
        <label>
          Чувствительность мыши: {s.sensitivity.toFixed(2)}
          <input
            type="range"
            min={0.2}
            max={3}
            step={0.05}
            value={s.sensitivity}
            onInput={(e) => updateSettings({ sensitivity: Number(e.currentTarget.value) })}
          />
        </label>
        <label class="row">
          <input type="checkbox" checked={s.invertY} onChange={(e) => updateSettings({ invertY: e.currentTarget.checked })} />
          Инвертировать ось Y
        </label>
        <label>
          Поле зрения: {s.fov}°
          <input
            type="range"
            min={55}
            max={100}
            step={1}
            value={s.fov}
            onInput={(e) => updateSettings({ fov: Number(e.currentTarget.value) })}
          />
        </label>
        <label>
          Громкость: {Math.round(s.volume * 100)}%
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={s.volume}
            onInput={(e) => updateSettings({ volume: Number(e.currentTarget.value) })}
          />
        </label>
        <div>
          Графика:
          <div class="row">
            {QUALITIES.map((q) => (
              <button
                type="button"
                key={q.id}
                class={`btn chip${s.quality === q.id ? ' on' : ''}`}
                onClick={() => updateSettings({ quality: q.id })}
              >
                {q.label}
              </button>
            ))}
          </div>
        </div>
        <label class="row">
          <input type="checkbox" checked={s.showFps} onChange={(e) => updateSettings({ showFps: e.currentTarget.checked })} />
          Показывать FPS
        </label>
        <div class="row between">
          <button
            type="button"
            class="btn danger"
            onClick={async () => {
              await fetch(`${BASE_PATH}api/logout`, { method: 'POST' }).catch(() => {});
              location.replace(`${BASE_PATH}pin/index.html`);
            }}
          >
            Выйти (забыть PIN)
          </button>
          <button type="button" class="btn primary" onClick={() => (settingsOpen.value = false)}>
            Готово
          </button>
        </div>
      </div>
    </div>
  );
}
