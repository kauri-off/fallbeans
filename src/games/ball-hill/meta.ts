import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'ball-hill',
  title: 'Скользкий склон',
  genre: 'race',
  rules: 'race',
  desc: 'Взбегите в гору, пока сверху катятся огромные шары. Прячьтесь за бортиками и прыгайте!',
  goal: 'Добегите до финиша',
  duration: 180,
  finishZ: 150,
});
