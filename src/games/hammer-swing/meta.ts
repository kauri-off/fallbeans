import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'hammer-swing',
  title: 'Молоты и качели',
  genre: 'race',
  desc: 'Развилка: узкий мост под молотами или извилистый коридор с толкателями. Дальше — качели зигзагом, лента, бегущая навстречу, и вертушка перед финишем.',
  goal: 'Доберитесь до финиша',
  duration: 110,
});
