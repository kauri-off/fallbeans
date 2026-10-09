//! Dev commands.
use core::fmt;

use super::*;

/// Why a dev command did not run.
#[derive(Clone, Debug, PartialEq)]
pub enum DevError {
    Off,
    NotHost,
    BadWarp,
    NoRound,
    NotGames(Vec<MapId>),
    NotPaused,
    NoBean(PlayerId),
    NoCheckpoint(u32),
    NoFinish,
    Full,
    NoGrab,
}

impl fmt::Display for DevError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DevError::Off => f.write_str("dev commands are off"),
            DevError::NotHost => f.write_str("dev commands are the host's"),
            DevError::BadWarp => f.write_str("warp by some seconds"),
            DevError::NoRound => f.write_str("no round running"),
            DevError::NotGames(ids) => {
                let ids: Vec<&str> = ids.iter().map(|m| m.as_str()).collect();
                write!(f, "not games: {}", ids.join(", "))
            }
            DevError::NotPaused => f.write_str("pause first (rate 0)"),
            DevError::NoBean(id) => write!(f, "no bean #{id} in this arena"),
            DevError::NoCheckpoint(i) => write!(f, "this map has no checkpoint {i}"),
            DevError::NoFinish => f.write_str("this map has no finish"),
            DevError::Full => f.write_str("room is full"),
            DevError::NoGrab => f.write_str("no such beans in play"),
        }
    }
}

impl Room {
    /// Runs a dev command for player `by`: a short result, or why it cannot.
    pub fn dev_command(&mut self, by: PlayerId, cmd: &DevCmd) -> Result<String, DevError> {
        let target = |a: &Arena, id: Option<PlayerId>| {
            let tid = id.unwrap_or(by);
            a.pawn(tid).map(|_| tid).ok_or(DevError::NoBean(tid))
        };
        match cmd {
            DevCmd::SkipIntro => {
                let left = self.zero - self.now();
                if self.arena.kind != ArenaKind::Round || left <= 0.0 {
                    return Ok("already started".into());
                }
                self.warp(left.ceil() + 1.0);
                Ok(format!("skipped {:.0} ms", left * 1000.0 / f64::from(TICK_RATE)))
            }
            DevCmd::Warp { s } => {
                if !s.is_finite() || *s <= 0.0 {
                    return Err(DevError::BadWarp);
                }
                let capped = s.min(WARP_MAX_S);
                self.warp((capped * f64::from(TICK_RATE)).round());
                Ok(if capped < *s {
                    format!("warped {capped} s (at most {WARP_MAX_S} s at once)")
                } else {
                    format!("warped {capped} s")
                })
            }
            DevCmd::EndRound => {
                if !self.round_live() {
                    return Err(DevError::NoRound);
                }
                self.end_round();
                Ok("round ended".into())
            }
            DevCmd::Start { games, rounds, bots } => {
                let bad: Vec<MapId> = games.iter().copied().filter(|&g| Game::by_id(g).is_none()).collect();
                if !bad.is_empty() {
                    return Err(DevError::NotGames(bad));
                }
                if !matches!(self.stage, Stage::Lobby) {
                    self.back_to_lobby();
                }
                // `bots`: exactly that many (the ones left from before are replaced).
                if bots.is_some() {
                    self.fill = false;
                    let old: Vec<PlayerId> = self.players.iter().filter(|p| p.is_bot()).map(|p| p.id).collect();
                    for b in old {
                        self.remove_player(b);
                    }
                }
                let humans = self.players.iter().filter(|p| !p.is_bot()).count();
                let want = self.max.min(humans + bots.unwrap_or(0) as usize);
                while self.players.len() < want {
                    self.add_bot(false);
                }
                self.playlist = if games.is_empty() {
                    Playlist {
                        rounds: rounds.unwrap_or(self.playlist.rounds),
                        ..self.playlist.clone()
                    }
                } else {
                    Playlist {
                        mode: Mode::Custom,
                        games: match rounds {
                            Some(n) => games.iter().cycle().take(*n as usize).cloned().collect(),
                            None => games.clone(),
                        },
                        rounds: rounds.unwrap_or(count(games.len())),
                    }
                };
                self.start_game();
                let plan: Vec<&str> = self
                    .session()
                    .map_or(Vec::new(), |s| s.plan.iter().map(|g| g.id().as_str()).collect());
                Ok(format!("started: {}", plan.join(", ")))
            }
            DevCmd::Lobby => {
                self.back_to_lobby();
                Ok("back in the lobby".into())
            }
            DevCmd::Rate { k } => {
                self.clock.set_rate(*k);
                self.broadcast(ServerMsg::Clock { rate: *k });
                Ok(if *k == 0.0 {
                    "paused".into()
                } else {
                    format!("game time ×{k}")
                })
            }
            DevCmd::Step { ticks } => {
                if self.clock.rate != 0.0 {
                    return Err(DevError::NotPaused);
                }
                self.warp(f64::from(*ticks));
                Ok(format!("stepped {ticks} ticks (tick {})", self.arena.tick))
            }
            DevCmd::Teleport { id, p, yaw } => {
                let id = target(&self.arena, *id)?;
                self.arena.dev_teleport(id, V3::from_array(*p), *yaw);
                Ok(format!("#{id} -> {:.1} {:.1} {:.1}", p[0], p[1], p[2]))
            }
            DevCmd::Goto { id, to } => {
                let id = target(&self.arena, *id)?;
                let place = match to {
                    Goto::Spawn => DevPlace::Spawn,
                    Goto::Finish => DevPlace::Finish,
                    Goto::Checkpoint(i) => DevPlace::Checkpoint(*i as usize),
                };
                let Some(at) = self.arena.dev_place(id, place) else {
                    return Err(match to {
                        Goto::Checkpoint(i) => DevError::NoCheckpoint(*i),
                        _ => DevError::NoFinish,
                    });
                };
                self.arena.dev_teleport(id, at, None);
                Ok(format!("#{id} -> {to:?}"))
            }
            DevCmd::Bot { n, near } => {
                let me = self.arena.pawn(by).map(|p| p.body.pos);
                let mut added = Vec::new();
                for i in 0..n.unwrap_or(1) {
                    if self.players.len() >= self.max {
                        break;
                    }
                    let id = self.add_bot(false);
                    added.push(id.to_string());
                    let a = f64::from(i) * 1.3;
                    let at = me
                        .filter(|_| *near)
                        .map(|m| m + V3::new(a.cos() * 2.0, 0.3, a.sin() * 2.0));
                    if self.arena.kind != ArenaKind::Lobby {
                        self.arena.add_late_pawn(id, true, at);
                    } else if let Some(at) = at {
                        self.arena.dev_teleport(id, at, None);
                    }
                }
                if added.is_empty() {
                    return Err(DevError::Full);
                }
                self.send_lobby();
                Ok(format!("bots {}", added.join(", ")))
            }
            DevCmd::Bots { on } => {
                self.arena.set_bots_on(*on);
                Ok(if *on { "bots think" } else { "bots frozen" }.into())
            }
            DevCmd::Kill { id } => {
                let id = target(&self.arena, *id)?;
                self.arena.dev_kill(id);
                Ok(format!("#{id} dropped"))
            }
            DevCmd::Knock { id, v } => {
                let id = target(&self.arena, *id)?;
                self.arena.dev_knock(id, V3::from_array(*v));
                Ok(format!("#{id} knocked"))
            }
            DevCmd::Grab { actor, target: t, s } => {
                let actor = target(&self.arena, *actor)?;
                let t = target(&self.arena, Some(*t))?;
                if !self.arena.dev_grab(actor, t, s.unwrap_or(3.0)) {
                    return Err(DevError::NoGrab);
                }
                Ok(format!("#{actor} holds #{t}"))
            }
            DevCmd::Seed { seed } => {
                self.next_seed = Some(*seed);
                self.upcoming = None;
                Ok(format!("next round seed {seed}"))
            }
        }
    }

    /// Dev: moves game time forward by `n` ticks, simulating every one (and running due timers).
    fn warp(&mut self, n: f64) {
        self.clock.rebase();
        let mut left = n;
        while left > 0.0 {
            let step = left.min(6.0);
            self.clock.skip(step);
            left -= step;
            self.tick_room(&mut NoInputs, true);
        }
        self.broadcast(ServerMsg::Clock { rate: self.clock.rate });
    }
}
