import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'plate-drop',
  title: 'Падающие плиты',
  genre: 'final',
  rules: 'lastStanding',
  desc: 'Плиты арены обрушиваются одна за другой, а над ними крутятся балки. Кто останется — тот и чемпион!',
  goal: 'Останьтесь последним',
  duration: 300,
});
