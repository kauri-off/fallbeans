import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'jump-club',
  title: 'Прыг-клуб',
  genre: 'survival',
  rules: 'survival',
  desc: 'Перепрыгивайте нижнюю балку и не попадите под верхнюю. Со временем они ускоряются!',
  goal: 'Не упадите',
  duration: 75,
});
