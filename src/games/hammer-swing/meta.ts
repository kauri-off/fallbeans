import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'hammer-swing',
  title: 'Молоты и качели',
  genre: 'race',
  rules: 'race',
  desc: 'Уворачивайтесь от маятников, удержитесь на качелях и пробегите против ленты!',
  goal: 'Добегите до финиша',
  duration: 180,
});
