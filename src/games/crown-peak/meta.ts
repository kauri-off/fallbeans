import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'crown-peak',
  title: 'Гора короны',
  genre: 'race',
  desc: 'Две дороги наверх: склон с шарами или скользящие ступени. Дальше — мосты под молотами, батуты и вертушки. Кто первым коснётся короны, тот и победил!',
  goal: 'Доберитесь до короны',
  duration: 120,
  finale: true,
});
