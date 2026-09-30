import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'tail-tag',
  title: 'Хвостики',
  genre: 'points',
  desc: 'У половины игроков есть хвосты — и с хвостом бежится чуть медленнее. Хватайте (Q / ПКМ) чужой хвост и не отдавайте свой! Батуты на парящие острова, портал и платформы по кругу помогут уйти.',
  goal: 'Держите хвост как можно дольше',
  duration: 75,
  minPlayers: 2,
  grab: true,
});
