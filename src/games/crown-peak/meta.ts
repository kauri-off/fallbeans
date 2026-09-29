import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'crown-peak',
  title: 'Гора короны',
  genre: 'final',
  rules: 'raceFinal',
  desc: 'Карабкайтесь на вершину сквозь шары, молоты и вертушки. Первый, кто схватит корону, — победитель!',
  goal: 'Схватите корону первым',
  duration: 240,
});
