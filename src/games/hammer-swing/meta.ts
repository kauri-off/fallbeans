import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'hammer-swing',
  title: 'Молоты и качели',
  genre: 'race',
  desc: 'Развилка: мост под молотами или коридор с толкателями. Дальше — качели, мостики-перевёртыши, ленты, перчатки и молоты в случайном порядке и со своими таймингами.',
  goal: 'Доберитесь до финиша',
  duration: 150,
});
