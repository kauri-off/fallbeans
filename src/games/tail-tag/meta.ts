import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'tail-tag',
  title: 'Хвостики',
  genre: 'points',
  desc: 'У половины игроков есть хвосты, и с хвостом бежится медленнее. Хватайте (Q / ПКМ) чужой хвост и не отдавайте свой: упавший отдаёт хвост тому, кто столкнул, или ближайшему. Батуты, портал и платформы помогут уйти.',
  goal: 'Держите хвост как можно дольше',
  duration: 75,
  minPlayers: 2,
  grab: true,
});
