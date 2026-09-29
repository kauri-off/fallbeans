import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'jump-club',
  title: 'Прыг-клуб',
  genre: 'survival',
  desc: 'Перепрыгивайте нижнюю балку и не попадайтесь под верхнюю. Со временем обе крутятся всё быстрее!',
  goal: 'Не упадите',
  duration: 75,
});
