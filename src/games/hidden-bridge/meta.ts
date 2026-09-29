import { z } from 'zod';
import { defineGame, relayOnce } from '../../shared/game';

export default defineGame({
  id: 'hidden-bridge',
  title: 'Невидимый мост',
  genre: 'race',
  rules: 'race',
  desc: 'Половина плиток моста — обманки. Шагайте осторожно и запоминайте, куда провалились другие!',
  goal: 'Найдите путь и добегите до финиша',
  duration: 180,
  finishZ: 96,
  events: { tile: z.object({ i: z.number().int().min(0).max(255) }) },
  server: { init: () => ({ seen: new Set<number>() }), on: { tile: relayOnce(350) } },
});
