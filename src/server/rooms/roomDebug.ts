import type { Room } from './room';

const r3 = (v: number) => Math.round(v * 1000) / 1000;

/** Everything about a room for the debug page and API (plain JSON). */
export function roomState(room: Room) {
  const a = room.arena;
  const now = room.now();
  return {
    id: room.id,
    title: room.title,
    private: !!room.pin,
    phase: room.phase,
    practice: room.practice,
    host: room.host,
    fill: room.fill,
    rate: room.clock.rate,
    timerIn: room.timerIn,
    session: room.session,
    round: room.round && { game: room.round.game.id, index: room.round.index, total: room.round.total, over: room.round.over },
    playlist: room.playlist,
    players: [...room.players.values()].map((p) => ({
      id: p.id,
      name: p.name,
      bot: p.bot,
      owner: !p.bot && p.uid === room.owner,
      connected: p.bot || !!p.conn,
      via: p.conn?.kind ?? null,
      rtt: Math.round(p.rtt),
      score: p.score,
      crowns: p.crowns,
      spectator: p.spectator,
      stats: p.stats,
    })),
    arena: {
      id: a.id,
      kind: a.kind,
      game: a.module.meta.id,
      seed: a.seed,
      tick: a.tick,
      t: r3(a.time),
      startsIn: Math.round(a.startAt - now),
      endsIn: Math.round(a.endAt - now),
      frozen: a.frozen,
      botsOn: a.botsOn,
      finished: a.finished,
      out: a.out,
      scores: [...a.scores],
      events: a.events.length,
      pawns: [...a.pawns.values()].map((p) => ({
        id: p.id,
        bot: !!p.bot,
        status: p.status,
        pos: [r3(p.body.pos.x), r3(p.body.pos.y), r3(p.body.pos.z)],
        speed: r3(Math.hypot(p.body.vel.x, p.body.vel.z)),
        state: p.body.state,
        grounded: p.body.grounded,
        progress: r3(p.progress),
        checkpoint: p.checkpoint ? p.checkpoint.z : null,
        grabbing: p.grabbing,
        ack: p.ack,
        lagTicks: p.bot ? 0 : a.tick - p.ack,
        queued: p.inputs.size,
        rejected: p.rejected,
        stats: p.stats,
      })),
    },
    perf: { ...room.meter.summary(), profile: room.prof.report() },
  };
}

/** Recent history of one bean (or of every bean) and the arena journal. */
export function roomTrace(room: Room, id?: number, seconds = 10) {
  const a = room.arena;
  const from = a.time - seconds;
  const pick = (list: readonly { t: number }[]) => list.filter((e) => e.t >= from);
  const trace =
    id !== undefined ? { [id]: pick(a.trace.get(id) ?? []) } : Object.fromEntries([...a.trace].map(([k, v]) => [k, pick(v)]));
  return { game: a.module.meta.id, t: a.time, journal: a.journal.filter((e) => e.t >= from), trace };
}
