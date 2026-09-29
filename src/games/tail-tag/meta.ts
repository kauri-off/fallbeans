import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'tail-tag',
  title: 'Хвостики',
  genre: 'points',
  rules: 'points',
  desc: 'У половины игроков есть хвосты. Хватайте (Q / ПКМ) чужой хвост и не отдавайте свой!',
  goal: 'Держите хвост как можно дольше',
  duration: 75,
  minPlayers: 2,
  grab: true,
});
