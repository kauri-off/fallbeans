import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'hidden-bridge',
  title: 'Невидимый мост',
  genre: 'race',
  desc: 'Часть плиток моста — обманки. Шагайте осторожно и следите, где провалились другие!',
  goal: 'Найдите путь и добегите до финиша',
  duration: 100,
});
