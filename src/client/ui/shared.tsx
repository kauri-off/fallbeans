import { useState } from 'preact/hooks';
import { GAMES } from '../../games';
import { BASE_PATH, NAME_MAX } from '../../shared/consts';
import { GENRE_LABEL } from '../../shared/game';
import type { Game } from '../game/game';
import type { Effects, Quality, Upscale } from '../game/renderer';
import { settings, updateSettings } from '../settings';
import { room } from '../state';

const QUALITIES: { id: Quality; label: string }[] = [
  { id: 'medium', label: 'Среднее' },
  { id: 'high', label: 'Высокое' },
];

const UPSCALES: { id: Upscale; label: string; title: string }[] = [
  { id: 'off', label: 'Выкл', title: 'Полное разрешение' },
  { id: 'ultra', label: 'Ультра', title: 'Рендер в 77% разрешения' },
  { id: 'quality', label: 'Качество', title: 'Рендер в 67% разрешения' },
  { id: 'balanced', label: 'Баланс', title: 'Рендер в 59% разрешения' },
  { id: 'performance', label: 'Скорость', title: 'Рендер в 50% разрешения' },
];

/** Graphics features that can be switched off one by one (to see what each costs). */
const FEATURES: { id: keyof Effects; label: string }[] = [
  { id: 'shadows', label: 'Тени' },
  { id: 'ao', label: 'Фоновое затенение (AO, только «Высокое»)' },
  { id: 'grade', label: 'Цветокоррекция и виньетка' },
  { id: 'msaa', label: 'Сглаживание MSAA' },
  { id: 'smaa', label: 'Сглаживание SMAA' },
  { id: 'temporal', label: 'Временное сглаживание (TAA)' },
  { id: 'motes', label: 'Частицы в воздухе' },
];

/** The player's name: kept in the browser and told to the server (at the room list or in a room). */
export function NameForm({ game }: { game: Game }) {
  const [name, setName] = useState(settings.value.name);
  return (
    <form
      class="row"
      onSubmit={(e) => {
        e.preventDefault();
        const n = name.trim();
        if (!n) return;
        updateSettings({ name: n });
        game.net.send({ t: 'name', name: n });
      }}
    >
      <input
        class="input"
        maxLength={NAME_MAX}
        value={name}
        placeholder="Ваше имя"
        onInput={(e) => setName(e.currentTarget.value)}
      />
      <button type="submit" class="btn">
        Сохранить
      </button>
    </form>
  );
}

/** One map against bots, alone: a link that reloads the page into a practice round. */
export function PracticeList() {
  const from = room.value?.id;
  return (
    <details class="practice">
      <summary>Тренировка одной карты с ботами</summary>
      <div class="row wrap">
        {GAMES.map((g) => (
          <a key={g.id} class="btn chip" href={`${BASE_PATH}?practice=${g.id}${from ? `&from=${from}` : ''}`} title={g.desc}>
            {g.title} · {GENRE_LABEL[g.genre]}
          </a>
        ))}
      </div>
    </details>
  );
}

/** Options kept in this browser: mouse, view, sound, graphics. */
export function SettingsTab() {
  const s = settings.value;
  const range = (
    label: string,
    key: 'sensitivity' | 'fov' | 'volume',
    min: number,
    max: number,
    step: number,
    fmt: (v: number) => string,
  ) => (
    <label class="field">
      <span>
        {label}: <b>{fmt(s[key])}</b>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={s[key]}
        onInput={(e) => updateSettings({ [key]: Number(e.currentTarget.value) })}
      />
    </label>
  );
  return (
    <div class="stack">
      {range('Чувствительность мыши', 'sensitivity', 0.2, 3, 0.05, (v) => v.toFixed(2))}
      <label class="row">
        <input type="checkbox" checked={s.invertY} onChange={(e) => updateSettings({ invertY: e.currentTarget.checked })} />
        Инвертировать мышь по вертикали
      </label>
      {range('Угол обзора', 'fov', 55, 100, 1, (v) => `${v}°`)}
      {range('Громкость звуков', 'volume', 0, 1, 0.05, (v) => `${Math.round(v * 100)} %`)}
      <div class="field">
        <span>Качество графики:</span>
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
      <div class="field">
        <span>Масштабирование AMD FSR:</span>
        <div class="row wrap">
          {UPSCALES.map((u) => (
            <button
              type="button"
              key={u.id}
              title={u.title}
              class={`btn chip${s.upscale === u.id ? ' on' : ''}`}
              onClick={() => updateSettings({ upscale: u.id })}
            >
              {u.label}
            </button>
          ))}
        </div>
      </div>
      <details class="practice">
        <summary>Графические эффекты (для проверки производительности)</summary>
        <div class="stack">
          {FEATURES.map((f) => (
            <label key={f.id} class="row">
              <input
                type="checkbox"
                checked={s.gfx[f.id]}
                onChange={(e) => updateSettings({ gfx: { ...s.gfx, [f.id]: e.currentTarget.checked } })}
              />
              {f.label}
            </label>
          ))}
        </div>
      </details>
      <label class="row">
        <input type="checkbox" checked={s.showFps} onChange={(e) => updateSettings({ showFps: e.currentTarget.checked })} />
        Показывать частоту кадров
      </label>
      <div class="field">
        <span>Меню (Esc):</span>
        <div class="row">
          {(
            [
              ['side', 'Сбоку, игра видна'],
              ['center', 'По центру, в две колонки'],
            ] as const
          ).map(([id, label]) => (
            <button
              type="button"
              key={id}
              class={`btn chip${s.menuLayout === id ? ' on' : ''}`}
              onClick={() => updateSettings({ menuLayout: id })}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
