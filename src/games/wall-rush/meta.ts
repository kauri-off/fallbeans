import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'wall-rush',
  title: 'Стенобой',
  genre: 'survival',
  desc: 'На платформу несутся стены — быстро и каждая по-своему: сплошные блоки, низкие стенки, балки сверху, окна и проёмы разной ширины. Найдите путь или перепрыгните — иначе снесёт!',
  goal: 'Не упадите',
  duration: 80,
});
