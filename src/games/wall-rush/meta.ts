import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'wall-rush',
  title: 'Стенобой',
  genre: 'survival',
  rules: 'survival',
  desc: 'На платформу надвигаются стены с проёмами. Найдите проход или перепрыгните — иначе столкнут!',
  goal: 'Не упадите',
  duration: 90,
});
