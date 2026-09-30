import { RAINBOW } from '../../shared/consts';
import type { Glasses, Hat } from '../../shared/outfit';

const RAINBOW_CSS = 'linear-gradient(135deg, #ff5f5f, #ffb13f, #ffe53f, #4fdc6a, #3fa9ff, #a66bff)';

/** A bean colour as a CSS background (colour dots, swatches, name tags). */
export const colorBg = (c: string) => (c === RAINBOW ? RAINBOW_CSS : c);

/** A bean colour as text on the dark chat panel: tinted halfway to white so every suit reads (AA). */
export const colorInk = (c: string) => {
  if (c === RAINBOW) return '#ffd89f';
  if (c === '#2b2b33' || c === '#8b5a2b') return '#e0d6ff';
  const n = Number.parseInt(c.slice(1), 16);
  const tint = (v: number) => Math.round((v + 255) / 2);
  return `rgb(${tint(n >> 16)}, ${tint((n >> 8) & 255)}, ${tint(n & 255)})`;
};

/** How the kill feed shows what knocked a bean off (collider tags and server causes). */
export const CAUSES: Record<string, { icon: string; text: string }> = {
  hammer: { icon: '🔨', text: 'молот' },
  rotor: { icon: '🌀', text: 'вертушка' },
  ball: { icon: '🎳', text: 'шар' },
  bumper: { icon: '💥', text: 'отбойник' },
  wall: { icon: '🧱', text: 'стена' },
  pusher: { icon: '🧱', text: 'толкатель' },
  drum: { icon: '🥁', text: 'барабан' },
  sweeper: { icon: '🧹', text: 'метла' },
  gate: { icon: '🚪', text: 'ворота' },
  tile: { icon: '⬡', text: 'плитка ушла из-под ног' },
  fall: { icon: '🕳️', text: 'падение' },
  tackle: { icon: '🤸', text: 'сбит нырком' },
  grab: { icon: '✊', text: 'захват' },
  shortcut: { icon: '🚫', text: 'срезка пути' },
};

export const cause = (c: string) => CAUSES[c] ?? { icon: '💫', text: c };

export const fmtTime = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;

export const signed = (n: number) => (n > 0 ? `+${n}` : n < 0 ? `−${-n}` : '0');

/** "1-е", "2-е"… (for «место»). */
export const ordinal = (n: number) => `${n}-е`;

/** Seconds with a decimal comma: 12,3 с. */
export const fmtSec = (s: number) => `${s.toFixed(1).replace('.', ',')} с`;

/** Russian plural: plural(5, 'игрок', 'игрока', 'игроков') → «5 игроков». */
export function plural(n: number, one: string, few: string, many: string) {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return `${n} ${one}`;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return `${n} ${few}`;
  return `${n} ${many}`;
}

export const HAT_LABEL: Record<Hat, string> = {
  none: 'Без шапки',
  cap: '🧢 Кепка',
  beanie: '🧶 Шапка',
  party: '🥳 Колпак',
  tophat: '🎩 Цилиндр',
  cowboy: '🤠 Ковбойская',
  viking: '⚔️ Викинг',
  propeller: '🚁 Пропеллер',
  bunny: '🐰 Ушки зайки',
  cat: '🐱 Ушки кошки',
  horns: '😈 Рожки',
  halo: '😇 Нимб',
  flower: '🌸 Цветок',
  antenna: '👽 Антенны',
};

export const GLASSES_LABEL: Record<Glasses, string> = {
  none: 'Без очков',
  round: '👓 Круглые',
  shades: '🕶️ Тёмные',
  hearts: '💕 Сердечки',
  monocle: '🧐 Монокль',
  visor: '🥽 Визор',
};
