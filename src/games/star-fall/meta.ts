import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'star-fall',
  title: 'Звездопад',
  genre: 'points',
  desc: 'Звёзды сыплются на арену: собирайте! На башне и островах — крупные. Сбили вас — все звёзды достаются обидчику, упали сами — сгорают. Хват (Q / ПКМ) выхватывает звезду.',
  goal: 'Соберите больше всех звёзд',
  duration: 100,
  grab: true,
});
