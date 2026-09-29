import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'hex-a-gone',
  title: 'Хекс-а-гон',
  genre: 'survival',
  desc: 'Плитки исчезают у вас из-под ног, а этажей всего три. Чем дольше продержитесь, тем больше очков!',
  goal: 'Продержитесь дольше всех',
  duration: 110,
  finale: true,
});
