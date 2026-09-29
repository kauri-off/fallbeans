import { z } from 'zod';
import { defineGame, relayOnce } from '../../shared/game';

export default defineGame({
  id: 'door-dash',
  title: 'Дверной переполох',
  genre: 'race',
  rules: 'race',
  desc: 'Ломайте фальшивые двери, перепрыгивайте вертушки и не упадите с движущихся платформ!',
  goal: 'Добегите до финиша',
  duration: 180,
  finishZ: 170,
  events: { door: z.object({ i: z.number().int().min(0).max(63) }) },
  server: { init: () => ({ seen: new Set<number>() }), on: { door: relayOnce(0) } },
});
