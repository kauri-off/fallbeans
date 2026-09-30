import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'plate-drop',
  title: 'Падающие плиты',
  genre: 'survival',
  desc: 'Плиты обрушиваются по две-три сразу и быстро, а над ними крутятся балки-шлагбаумы, сбивающие с ног. Продержитесь дольше всех!',
  goal: 'Продержитесь дольше всех',
  duration: 100,
  finale: true,
});
