//! Simulation: timers and arena ticks.
use super::*;

impl Room {
    /// Advances timers and the simulation to server tick `real`.
    pub fn update(&mut self, real: u64, inputs: &mut dyn Inputs) {
        self.clock.set_real(real);
        let grace = ticks(if matches!(self.stage, Stage::Lobby) {
            LOBBY_GRACE_S
        } else {
            RECONNECT_GRACE_S
        });
        let timed_out: Vec<PlayerId> = self
            .players
            .iter()
            .filter(|p| match p.kind {
                Kind::Human {
                    conn: None,
                    disconnected_at: Some(at),
                    ..
                } => real.saturating_sub(at) > grace,
                _ => false,
            })
            .map(|p| p.id)
            .collect();
        for id in timed_out {
            info!(room = %self.id, %id, "player timed out");
            self.remove_player(id);
        }
        self.tick_room(inputs, false);
        if self.lobby_due && real.saturating_sub(self.profile_sent) >= ticks(PROFILE_EVERY_S) {
            self.profile_sent = real;
            self.send_lobby();
        }
    }

    pub(super) fn tick_room(&mut self, inputs: &mut dyn Inputs, all: bool) {
        self.take_nav();
        if self.timer().is_some_and(|at| self.now() >= at) {
            match core::mem::replace(&mut self.stage, Stage::Lobby) {
                Stage::Game {
                    session,
                    step: Step::Results { .. },
                } => self.next_round(session),
                _ => self.back_to_lobby(),
            }
        }
        let target = (self.now() - self.zero).floor() as i64;
        if !all && target - self.arena.tick > MAX_CATCHUP {
            warn!(room = %self.id, behind = target - self.arena.tick, "arena fell behind");
            self.arena.skip_to(target - MAX_CATCHUP);
        }
        while self.arena.tick < target {
            let k = self.arena.tick + 1;
            self.step(k, inputs);
        }
        self.check_round();
    }

    /// Server tick of arena tick `k`.
    fn server_tick(&self, k: i64) -> u32 {
        (self.zero_tick() + k).max(0) as u32
    }

    fn step(&mut self, k: i64, inputs: &mut dyn Inputs) {
        let tick = self.server_tick(k);
        if self.hits.is_some() && self.arena.hit_log.is_none() {
            self.arena.hit_log = Some(Vec::new());
        }
        for p in &self.players {
            if let Some(conn) = p.conn() {
                let view = inputs.view(p.id, conn);
                self.arena.set_view(p.id, view);
            }
        }
        let mut frames: Vec<(PlayerId, InputFrame)> = self
            .players
            .iter()
            .filter_map(|p| Some((p.id, inputs.frame(p.id, p.conn()?, tick).clamped())))
            .collect();
        frames.sort_unstable_by_key(|f| f.0);
        if frames.iter().any(|f| f.1 != InputFrame::IDLE) {
            self.active_at = self.clock.real();
        }
        let events = self.arena.step(k, |id| {
            frames
                .binary_search_by_key(&id, |f| f.0)
                .map_or(InputFrame::IDLE, |i| frames[i].1)
        });
        if let (Some(out), Some(log)) = (&mut self.hits, &mut self.arena.hit_log) {
            let (arena, map) = (self.arena_id, self.arena.map.meta().id);
            out.extend(log.drain(..).map(|l| format!("{arena} {map} {l}")));
        }
        let live = self.round_live();
        for e in events {
            match e {
                ArenaEvent::Bonus(b) => {
                    let ev = MapEventKind::Bonus {
                        i: b.i,
                        id: b.id,
                        at: b.at,
                    };
                    self.map_event(tick, ev, true);
                }
                ArenaEvent::Finish { id, t } => {
                    if !live {
                        continue;
                    }
                    let place = self.arena.finished.len() as u32;
                    self.map_event(tick, MapEventKind::Finish { id, place, time: t }, false);
                    self.check_round();
                }
                ArenaEvent::Ko(ko) => {
                    if self.arena.kind == ArenaKind::Round && !live {
                        continue;
                    }
                    let ev = MapEventKind::Ko {
                        id: ko.id,
                        out: ko.out,
                        by: ko.by,
                        cause: ko.cause,
                        shortcut: ko.shortcut,
                    };
                    self.map_event(tick, ev, false);
                    if ko.out {
                        self.check_round();
                    }
                }
                ArenaEvent::Emote { id, e } => self.broadcast(ServerMsg::Emote { id, e: e as u8 }),
                ArenaEvent::Event { ev, keep } => self.map_event(tick, MapEventKind::Map(ev), keep),
                ArenaEvent::Score { id, v } => self.broadcast(ServerMsg::Scores(vec![(id, v)])),
            }
        }
    }

    fn map_event(&mut self, tick: u32, ev: MapEventKind, keep: bool) {
        let msg = MapEventMsg {
            arena: self.arena_id,
            tick,
            ev,
            history: false,
        };
        for c in self.players.iter().filter_map(Player::conn) {
            self.out.push(Out::Event(c, msg.clone()));
        }
        if keep {
            self.history.push(msg);
        }
    }
}
