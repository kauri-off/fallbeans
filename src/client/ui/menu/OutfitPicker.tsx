import { COLORS } from '../../../shared/consts';
import { DEFAULT_OUTFIT, GLASSES, HATS, type Outfit, TINTS } from '../../../shared/outfit';
import type { Game } from '../../game/game';
import { settings, updateSettings } from '../../settings';
import { lobby, myId } from '../../state';
import { colorBg, GLASSES_LABEL, HAT_LABEL } from '../labels';

type TintKey = 'hatColor' | 'belly' | 'shoes';

const pick = <T,>(a: readonly T[]) => a[Math.floor(Math.random() * a.length)]!;

/** The player's look: suit colour (lobby only, one per bean), hat, glasses and colours of the hat, belly and shoes. */
export function OutfitPicker({ game }: { game: Game }) {
  const l = lobby.value;
  const me = myId.value;
  if (!l) return null;
  const mine = l.players.find((p) => p.id === me);
  const outfit = settings.value.outfit;
  const wear = (patch: Partial<Outfit>) => {
    const o = { ...outfit, ...patch };
    updateSettings({ outfit: o });
    game.net.send({ t: 'outfit', o });
  };
  const tints = (key: TintKey, title: string) => (
    <div class="field">
      <span class="small muted">{title}</span>
      <div class="swatches small">
        <button
          type="button"
          class={`swatch auto${outfit[key] === '' ? ' on' : ''}`}
          title="Как задумано"
          onClick={() => wear({ [key]: '' })}
        />
        {TINTS.map((c) => (
          <button
            type="button"
            key={c}
            class={`swatch${outfit[key] === c ? ' on' : ''}`}
            style={{ background: c }}
            onClick={() => wear({ [key]: c })}
          />
        ))}
      </div>
    </div>
  );

  return (
    <>
      {l.phase === 'lobby' && (
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
                onClick={() => {
                  updateSettings({ color: c });
                  game.net.send({ t: 'color', c });
                }}
              />
            );
          })}
        </div>
      )}
      <details class="outfit">
        <summary>Шапка, очки и цвета</summary>
        <div class="stack">
          <div class="row wrap">
            {HATS.map((h) => (
              <button type="button" key={h} class={`btn chip${outfit.hat === h ? ' on' : ''}`} onClick={() => wear({ hat: h })}>
                {HAT_LABEL[h]}
              </button>
            ))}
          </div>
          {outfit.hat !== 'none' && tints('hatColor', 'Цвет шапки')}
          <div class="row wrap">
            {GLASSES.map((g) => (
              <button
                type="button"
                key={g}
                class={`btn chip${outfit.glasses === g ? ' on' : ''}`}
                onClick={() => wear({ glasses: g })}
              >
                {GLASSES_LABEL[g]}
              </button>
            ))}
          </div>
          {tints('belly', 'Цвет живота')}
          {tints('shoes', 'Цвет ботинок')}
          <div class="row">
            <button
              type="button"
              class="btn tiny"
              onClick={() =>
                wear({
                  hat: pick(HATS),
                  hatColor: pick(['', ...TINTS]),
                  glasses: pick(GLASSES),
                  belly: pick(['', ...TINTS]),
                  shoes: pick(['', ...TINTS]),
                })
              }
            >
              🎲 Случайно
            </button>
            <button type="button" class="btn tiny" onClick={() => wear(DEFAULT_OUTFIT)}>
              Сбросить
            </button>
          </div>
        </div>
      </details>
    </>
  );
}
