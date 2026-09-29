import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'wall-rush',
  title: 'Стенобой',
  genre: 'survival',
  desc: 'На платформу надвигаются стены с проёмами. Ищите проход или перепрыгивайте низкие стенки — иначе вас столкнут!',
  goal: 'Не упадите',
  duration: 80,
});
