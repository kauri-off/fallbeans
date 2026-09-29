import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'roll-out',
  title: 'Перекати-поле',
  genre: 'survival',
  desc: 'Огромные барабаны с дырами вращаются под ногами. Бегите против вращения и перепрыгивайте провалы!',
  goal: 'Не упадите',
  duration: 80,
});
