//! Game flow: arenas, rounds and the session from the lobby to the podium.
use super::*;

impl Room {
    fn new_arena_id(&mut self) -> u32 {
        self.arena_seq = self.arena_seq % 65535 + 1;
        self.arena_seq
    }

    fn make_lobby_arena(&mut self) -> (Arena, f64) {
        let zero = self.now().floor();
        let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        let mut arena = lobby_arena(&ids);
        for p in &self.players {
            arena.add_pawn(p.id, p.is_bot());
        }
        (arena, zero)
    }

    fn set_arena(&mut self, (mut arena, zero): (Arena, f64)) {
        if let Some(rec) = self.arena.take_recording() {
            self.replays.push_back(rec);
            if self.replays.len() > 5 {
                self.replays.pop_front();
            }
        }
        // To the new spawn as a teleport: views snap instead of gliding across the map.
        for p in &mut arena.pawns {
            if let Some(old) = self.arena.pawn(p.id) {
                p.teleports = old.teleports + 1;
            }
        }
        self.arena = arena;
        self.arena_id = self.new_arena_id();
        self.zero = zero;
        self.history.clear();
        self.broadcast(ServerMsg::Arena(self.arena_info(false)));
    }

    /// Players who take part in the next round: bots and connected humans.
    pub(super) fn roster(&self) -> Vec<PlayerId> {
        self.players.iter().filter(|p| p.present()).map(|p| p.id).collect()
    }

    pub(super) fn start_game(&mut self) {
        let mut ids = self.roster();
        ids.truncate(self.max);
        for p in &mut self.players {
            p.score = 0;
            p.stats = GameStats::default();
            p.spectator = false;
        }
        let plan = match &self.practice {
            Some(pr) => vec![pr.game],
            None => Game::plan(ids.len() as u32, &self.playlist, &mut self.rng),
        };
        info!(room = %self.id, players = ids.len(), plan = ?plan, "game started");
        self.next_round(Session {
            plan,
            index: 0,
            started: ids.len(),
        });
    }

    /// Skips the planned games that no longer fit so many players (practice plays its game whatever comes).
    fn skip_unfit(&self, session: &mut Session, players: usize) {
        if self.practice() {
            return;
        }
        let skipped = session.skip_unfit(players);
        if !skipped.is_empty() {
            info!(room = %self.id, players, skipped = ?skipped, "games that no longer fit skipped");
        }
    }

    /// Arena tick a round is made at (the intro runs before tick 0).
    fn round_start(&self) -> i64 {
        let zero = self.now().floor() + f64::from(self.intro_ticks);
        (self.now() - zero).floor() as i64 - 1
    }

    /// The map's seed and the spawn order.
    fn draw_round(&mut self, mut ids: Vec<PlayerId>) -> (u32, Vec<PlayerId>) {
        let seed = self.next_seed.unwrap_or_else(|| (self.rng.unit() * 1e9).floor() as u32);
        // A dev seed fixes the spawn order too (screenshots, repeatable tests).
        match self.next_seed.take() {
            Some(s) => shuffle(&mut ids, &mut Rng::new(s ^ 0x5eed)),
            None => shuffle(&mut ids, &mut self.rng),
        }
        (seed, ids)
    }

    #[cfg(test)]
    pub fn round_prepared(&self) -> bool {
        self.upcoming.is_some()
    }

    /// Draws the next round now and builds its arena (and the bots' grid) on another thread: off the tick.
    fn prepare_round(&mut self) {
        let players = self.roster().len();
        let Stage::Game { mut session, step } = core::mem::replace(&mut self.stage, Stage::Lobby) else {
            return;
        };
        self.skip_unfit(&mut session, players);
        let next = session.next(self.practice());
        self.stage = Stage::Game { session, step };
        let Some((game, _, _)) = next else { return };
        let dev_seed = self.next_seed;
        let (seed, participants) = self.draw_round(self.roster());
        let bots = participants
            .iter()
            .any(|&id| self.player(id).is_some_and(Player::is_bot));
        let (map, start, ids) = (game.map(), self.round_start(), participants.clone());
        let built = std::thread::Builder::new().name("round".into()).spawn(move || {
            let (mut arena, _) = Arena::new(map, ArenaKind::Round, seed, start, &ids, false);
            if bots {
                arena.prepare_nav();
            }
            arena
        });
        match built {
            Ok(arena) => {
                self.upcoming = Some(Upcoming {
                    game,
                    seed,
                    start,
                    participants,
                    dev_seed,
                    arena,
                });
            }
            Err(e) => {
                warn!(room = %self.id, "no thread for the next round: {e}");
                self.next_seed = dev_seed;
            }
        }
    }

    pub(super) fn next_round(&mut self, mut session: Session) {
        let ids = self.roster();
        if ids.is_empty() {
            return self.back_to_lobby();
        }
        self.skip_unfit(&mut session, ids.len());
        let Some((game, index, total)) = session.next(self.practice()) else {
            return self.end_game(session);
        };
        if !self.practice() {
            session.index += 1;
        }
        let round = RoundInfo { game, index, total };
        self.stage = Stage::Game {
            session,
            step: Step::Round(round),
        };
        let zero = self.now().floor() + f64::from(self.intro_ticks);
        let start = self.round_start();
        // The round drawn ahead, if the same players are here for it.
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        let (ready, stale): (Option<Upcoming>, Option<Upcoming>) = match self.upcoming.take() {
            Some(u) => {
                let mut theirs = u.participants.clone();
                theirs.sort_unstable();
                if u.game == game && u.start == start && theirs == sorted {
                    (Some(u), None)
                } else {
                    (None, Some(u))
                }
            }
            None => (None, None),
        };
        if let Some(u) = stale {
            self.next_seed = u.dev_seed;
        }
        let (seed, participants, built) = match ready {
            Some(u) => (u.seed, u.participants, u.arena.join().ok()),
            None => {
                let (seed, participants) = self.draw_round(ids);
                (seed, participants, None)
            }
        };
        for p in &mut self.players {
            p.spectator = !participants.contains(&p.id);
        }
        let on_tick = built.is_none();
        let mut arena =
            built.unwrap_or_else(|| Arena::new(game.map(), ArenaKind::Round, seed, start, &participants, false).0);
        if !self.eliminate && arena.fall == FallBehaviour::Out {
            arena.fall = FallBehaviour::Spawn;
        }
        if self.dev() {
            arena.record();
        }
        for (i, id) in participants.iter().enumerate() {
            let bot = self.player(*id).is_some_and(Player::is_bot);
            arena.add_pawn_at(*id, bot, Some(i));
        }
        info!(room = %self.id, game = game.id(), seed, players = participants.len(), "round");
        let bots = participants
            .iter()
            .any(|&id| self.player(id).is_some_and(Player::is_bot));
        self.set_arena((arena, zero));
        if on_tick && bots {
            self.nav_ahead(game, seed, start, participants);
        }
        self.send_lobby();
    }

    /// A round's arena built on the tick (none drawn ahead for these players): its bots' grid is built from a
    /// twin of it on another thread meanwhile and handed over when done (`take_nav`), before the intro ends.
    fn nav_ahead(&mut self, game: Game, seed: u32, start: i64, ids: Vec<PlayerId>) {
        let map = game.map();
        let built = std::thread::Builder::new().name("round nav".into()).spawn(move || {
            let (mut twin, _) = Arena::new(map, ArenaKind::Round, seed, start, &ids, false);
            twin.prepare_nav();
            twin.prepared_nav().cloned()
        });
        match built {
            Ok(job) => self.nav_job = Some((self.arena_id, job)),
            Err(e) => warn!(room = %self.id, "no thread for the bots' grid: {e}"),
        }
    }

    /// The grid `nav_ahead` built, to its arena if that is still the one played.
    pub(super) fn take_nav(&mut self) {
        if !self.nav_job.as_ref().is_some_and(|(_, job)| job.is_finished()) {
            return;
        }
        let Some((id, job)) = self.nav_job.take() else { return };
        if let Ok(Some(pre)) = job.join()
            && id == self.arena_id
        {
            self.arena.give_nav(pre);
        }
    }

    fn time_up(&self) -> bool {
        self.now() > self.zero + ticks(self.arena.map.meta().duration) as f64
    }

    /// The round as the rules see it.
    fn with_view<R>(&mut self, f: impl FnOnce(&RoundView, &mut Rng) -> R) -> Option<R> {
        let game = self.round()?.game;
        let time_up = self.time_up();
        let bots: BTreeSet<PlayerId> = self.players.iter().filter(|p| p.is_bot()).map(|p| p.id).collect();
        let players = &self.players;
        let arena = &self.arena;
        let connected = |id: PlayerId| players.iter().any(|p| p.id == id);
        let progress = |id: PlayerId| arena.pawn(id).map_or(f64::NEG_INFINITY, |p| p.progress);
        let view = RoundView {
            genre: game.meta().genre,
            participants: &arena.participants,
            connected: &connected,
            finished: &arena.finished,
            out: &arena.out,
            scores: &arena.scores,
            progress: &progress,
            time_up,
            bots: Some(&bots),
        };
        Some(f(&view, &mut self.rng))
    }

    pub(super) fn check_round(&mut self) {
        if !self.round_live() || self.arena.kind != ArenaKind::Round {
            return;
        }
        if self.with_view(|v, _| v.is_round_over()) == Some(true) {
            self.end_round();
        }
    }

    pub(super) fn end_round(&mut self) {
        let wait = if self.practice() { PRACTICE_RESULTS_S } else { RESULTS_S };
        let next = self.after(wait);
        let Stage::Game { step, .. } = &mut self.stage else {
            return;
        };
        let Step::Round(round) = *step else { return };
        *step = Step::Results { round, next };
        let RoundInfo { game, index, total } = round;
        self.arena.freeze();
        let stats: BTreeMap<PlayerId, RoundStats> = self.arena.pawns.iter().map(|p| (p.id, p.stats)).collect();
        let totals: BTreeMap<PlayerId, i64> = self.players.iter().map(|p| (p.id, p.score)).collect();
        let rows = self
            .with_view(|v, rng| v.score_round(&stats, &totals, Some(rng)))
            .unwrap_or_default();
        let n = rows.len();
        let round_secs = self.arena.time().max(0.0);
        let genre = game.meta().genre;
        for row in &rows {
            let Some(p) = self.players.iter_mut().find(|p| p.id == row.id) else {
                continue;
            };
            p.score = row.total;
            let Some(s) = stats.get(&row.id) else { continue };
            let g = &mut p.stats;
            g.falls += s.falls;
            g.kos += s.kos;
            g.grabs += s.grabs;
            g.tackles += s.tackles;
            g.shortcuts += s.shortcuts;
            if row.place == 1 && n > 1 {
                g.wins += 1;
            }
            if genre == Genre::Race {
                g.race_ranks.push(if n > 1 {
                    (row.place - 1) as f64 / (n - 1) as f64
                } else {
                    0.0
                });
            }
            if genre == Genre::Survival {
                g.survived += s.out_at.unwrap_or(round_secs);
            }
        }
        let deltas: Vec<(PlayerId, i64)> = rows.iter().map(|r| (r.id, r.delta)).collect();
        info!(room = %self.id, game = game.id(), rows = ?deltas, "round over");
        self.broadcast(ServerMsg::RoundEnd {
            game: game.id().into(),
            index,
            total,
            rows,
            practice: self.practice(),
        });
        self.prepare_round();
        self.send_lobby();
    }

    /// Final standings: points, then round wins, then fewer falls.
    fn standings(&self) -> Vec<Standing> {
        let mut list: Vec<&Player> = self.players.iter().collect();
        list.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.stats.wins.cmp(&a.stats.wins))
                .then(a.stats.falls.cmp(&b.stats.falls))
                .then(a.id.cmp(&b.id))
        });
        list.iter()
            .enumerate()
            .map(|(i, p)| Standing {
                id: p.id,
                name: p.name.clone(),
                color: p.color,
                place: i as u32 + 1,
                total: p.score,
                wins: p.stats.wins,
                falls: p.stats.falls,
            })
            .collect()
    }

    fn end_game(&mut self, session: Session) {
        let standings = self.standings();
        let Some(winner) = standings.first().map(|s| s.id) else {
            return self.back_to_lobby();
        };
        self.stage = Stage::Game {
            session,
            step: Step::Podium {
                lobby: self.after(PODIUM_S),
            },
        };
        if let Some(p) = self.player_mut(winner) {
            p.crowns += 1;
        }
        let awards = compute_awards(&self.players.iter().map(|p| (p.id, &p.stats)).collect::<Vec<_>>());
        let totals: Vec<(PlayerId, i64)> = standings.iter().map(|s| (s.id, s.total)).collect();
        info!(room = %self.id, %winner, standings = ?totals, "game over");
        let order: Vec<PlayerId> = standings.iter().map(|s| s.id).collect();
        let zero = self.now().floor();
        let (mut arena, _) = Arena::new(&PodiumMap, ArenaKind::Podium, 1, -1, &order, false);
        for (i, id) in order.iter().enumerate() {
            let bot = self.player(*id).is_some_and(Player::is_bot);
            arena.add_pawn_at(*id, bot, Some(i));
        }
        self.set_arena((arena, zero));
        self.broadcast(ServerMsg::GameEnd { standings, awards });
        self.send_lobby();
    }

    pub(super) fn back_to_lobby(&mut self) {
        self.stage = Stage::Lobby;
        if !self.practice() {
            let gone: Vec<PlayerId> = self
                .players
                .iter()
                .filter(|p| !p.is_bot() && p.conn().is_none())
                .map(|p| p.id)
                .collect();
            for id in gone {
                self.players.retain(|p| p.id != id);
                self.broadcast(ServerMsg::Left(id));
            }
        }
        for p in &mut self.players {
            p.spectator = false;
        }
        self.update_host();
        if self.arena.kind != ArenaKind::Lobby || self.arena.pawns.len() != self.players.len() {
            let lobby = self.make_lobby_arena();
            self.set_arena(lobby);
        }
        self.sync_bots();
        self.send_lobby();
    }
}
