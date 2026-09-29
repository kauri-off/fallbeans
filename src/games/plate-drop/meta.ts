import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'plate-drop',
  title: 'Падающие плиты',
  genre: 'survival',
  desc: 'Плиты арены обрушиваются одна за другой, а над ними крутятся балки. Продержитесь дольше всех!',
  goal: 'Продержитесь дольше всех',
  duration: 100,
  finale: true,
});
