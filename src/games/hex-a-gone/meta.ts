import { z } from 'zod';
import { defineGame, relayOnce } from '../../shared/game';

export default defineGame({
  id: 'hex-a-gone',
  title: 'Хекс-а-гон',
  genre: 'final',
  rules: 'lastStanding',
  desc: 'Плитки исчезают под ногами. Три этажа. Кто продержится последним — забирает корону!',
  goal: 'Останьтесь последним',
  duration: 400,
  events: { tile: z.object({ i: z.number().int().min(0).max(1023) }) },
  server: { init: () => ({ seen: new Set<number>() }), on: { tile: relayOnce(450) } },
});
