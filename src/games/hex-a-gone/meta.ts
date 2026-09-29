import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'hex-a-gone',
  title: 'Хекс-а-гон',
  genre: 'final',
  rules: 'lastStanding',
  desc: 'Плитки исчезают под ногами. Три этажа. Кто продержится последним — забирает корону!',
  goal: 'Останьтесь последним',
  duration: 300,
});
