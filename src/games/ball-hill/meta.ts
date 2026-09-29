import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'ball-hill',
  title: 'Скользкий склон',
  genre: 'race',
  desc: 'Ледяной склон: держаться можно только на ковровых дорожках-зигзагах, а сверху катятся шары. Выше — ворота со сдвигающимися проёмами.',
  goal: 'Доберитесь до финиша',
  duration: 110,
});
