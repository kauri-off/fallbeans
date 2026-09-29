import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'door-dash',
  title: 'Дверной переполох',
  genre: 'race',
  rules: 'race',
  desc: 'Ломайте фальшивые двери, перепрыгивайте вертушки и не упадите с движущихся платформ!',
  goal: 'Добегите до финиша',
  duration: 180,
});
