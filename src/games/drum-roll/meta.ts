import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'drum-roll',
  title: 'Барабаны',
  genre: 'race',
  rules: 'race',
  desc: 'Перебегайте крутящиеся барабаны, отталкивайтесь от батутов и не попадите под лопасти!',
  goal: 'Добегите до финиша',
  duration: 180,
  finishZ: 128,
});
