import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'cliff-climb',
  title: 'Скалолазы',
  genre: 'race',
  desc: 'Всё выше и выше! Запрыгивайте на уступы и цепляйтесь за край, лезьте по лестницам (прыжок — соскочить) и не попадитесь под маятник. Наверху ждёт финиш.',
  goal: 'Заберитесь на вершину',
  duration: 160,
});
