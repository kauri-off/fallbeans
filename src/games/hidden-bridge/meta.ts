import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'hidden-bridge',
  title: 'Невидимый мост',
  genre: 'race',
  rules: 'race',
  desc: 'Часть плиток моста — обманки. Шагайте осторожно и запоминайте, куда провалились другие!',
  goal: 'Найдите путь и добегите до финиша',
  duration: 180,
});
