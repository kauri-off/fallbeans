import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'crown-peak',
  title: 'Гора короны',
  genre: 'race',
  desc: 'Долгий подъём: склон с шарами или скользящие ступени, мосты под молотами, перчатки и батуты, дальше — испытания в случайном порядке. Кто первым коснётся короны на вершине, тот и победил!',
  goal: 'Доберитесь до короны',
  duration: 170,
  finale: true,
});
