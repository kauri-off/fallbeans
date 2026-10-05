//! A lobby and the games played from it (port of `server/rooms/room.ts`): players and bots, the host,
//! rounds, scores, chat, dev commands. Time is in server ticks; the arena runs on the room's game clock.
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::log::{info, warn};
use fb_arena::{Arena, ArenaEvent, ArenaKind, DevPlace, FallBehaviour, PawnStatus, Recording, RoundStats};
use fb_maps::director::{self, plan_game, valid_playlist};
use fb_proto::*;
use fb_shared::game::{GameMeta, Genre};
use fb_shared::input::InputFrame;
use fb_shared::outfit::bot_outfit;
use fb_shared::rng::{Rng, shuffle};
use fb_shared::rules::{RoundView, is_round_over, score_round};
use fb_shared::text::{sanitize_chat, sanitize_name};
use fb_shared::*;
use fb_sim::map::json;
use fb_sim::math::V3;

use super::awards::{GameStats, compute_awards};
use super::clock::GameClock;
use super::players::{Player, bot_name};
use super::{ConnId, Inputs, NoInputs, Out, random_u32, ticks};

/// Control messages per second from one player.
const MSG_RATE: u32 = 60;
/// Shortest time between two chat lines of one player (s).
const CHAT_GAP_S: f64 = 0.7;
/// Ticks the arena may fall behind before it skips ahead instead of catching up.
const MAX_CATCHUP: i64 = 60;

#[derive(Clone, Debug)]
pub struct Practice {
    pub game: String,
    pub bots: usize,
}

#[derive(Clone, Debug)]
pub struct RoomOptions {
    pub min_players: usize,
    pub max_players: usize,
    pub practice: Option<Practice>,
    pub seed: Option<u32>,
    pub intro_ticks: u32,
    /// Accept dev commands (server `--dev`).
    pub dev: bool,
    /// Beans falling out of a survival round are out (off with `fb_server --respawn`: stress runs keep
    /// everybody in play).
    pub eliminate: bool,
    /// The room's code and name in the room list (empty for rooms that are not listed).
    pub id: String,
    pub title: String,
    /// Identity of the player who created the room: the host whenever they are in it.
    pub owner: Option<String>,
    /// PIN of a private room.
    pub pin: Option<String>,
    /// Kept when nobody is in it (the dev server's room).
    pub permanent: bool,
    /// Who opened it, as limits per address count (`auth::address_key`; None: this machine or nobody).
    pub creator: Option<String>,
}

impl Default for RoomOptions {
    fn default() -> Self {
        Self {
            min_players: 2,
            max_players: MAX_PLAYERS,
            practice: None,
            seed: None,
            intro_ticks: ticks(INTRO_S) as u32,
            dev: false,
            eliminate: true,
            id: String::new(),
            title: String::new(),
            owner: None,
            pin: None,
            permanent: false,
            creator: None,
        }
    }
}

/// One game: a planned series of rounds everyone plays, scored by points.
#[derive(Clone, Debug)]
pub struct Session {
    pub plan: Vec<&'static str>,
    /// Rounds started so far.
    pub index: usize,
    /// Players the game started with (for solo detection).
    pub started: usize,
}

#[derive(Clone, Debug)]
pub struct RoundInfo {
    pub game: &'static GameMeta,
    pub over: bool,
    pub index: u32,
    pub total: u32,
}

/// The next round, drawn when the last one ended and built on another thread while the results show.
struct Upcoming {
    game: &'static str,
    seed: u32,
    start: i64,
    participants: Vec<Pid>,
    dev_seed: Option<u32>,
    arena: std::thread::JoinHandle<Arena>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    NextRound,
    BackToLobby,
}

/// Who enters a room.
#[derive(Clone, Debug, Default)]
pub struct Who {
    pub uid: String,
    pub name: String,
    pub color: Option<u8>,
    pub outfit: Option<Outfit>,
}

pub fn make_pin() -> String {
    format!("{:04}", random_u32() % 10u32.pow(ROOM_PIN_DIGITS as u32))
}

pub struct Room {
    pub id: String,
    pub title: String,
    /// Identity of the room's creator (None: nobody's, the dev room).
    pub owner: Option<String>,
    /// PIN of a private room; None when anyone may enter.
    pub pin: Option<String>,
    /// Identities the room let in: they come back without the PIN.
    pub admitted: BTreeSet<String>,
    /// In the order they came.
    pub players: Vec<Player>,
    /// The acting host: the owner whenever they are in the room; while they are away (or after they handed
    /// the role over) another connected player.
    pub host: Option<Pid>,
    pub phase: Phase,
    pub session: Option<Session>,
    pub round: Option<RoundInfo>,
    pub arena: Arena,
    /// Changes with every new arena (clients drop what belongs to an older one).
    pub arena_id: u32,
    /// Game tick of the arena's tick 0.
    zero: f64,
    pub playlist: Playlist,
    /// The host wants the empty places of the lobby filled with bots.
    pub fill: bool,
    /// Game time: timers and arenas run on it (dev commands slow, pause or warp it).
    pub clock: GameClock,
    pub max: usize,
    next_id: Pid,
    arena_seq: u32,
    timer: Option<(f64, Timer)>,
    rng: Rng,
    /// Dev: seed of the next round's map.
    next_seed: Option<u32>,
    upcoming: Option<Upcoming>,
    /// Dev: the last rounds as played (newest last).
    pub replays: VecDeque<Recording>,
    /// Map events of this arena that someone arriving later must hear of.
    history: Vec<MapEventMsg>,
    /// What to send.
    pub out: Vec<Out>,
    /// Something the room list shows changed (players, host, phase, access).
    pub changed: bool,
    opts: RoomOptions,
}

fn arena_map(id: &str) -> &'static dyn fb_sim::map::MapDef {
    fb_maps::by_id(id).unwrap_or_else(|| panic!("map {id} is registered"))
}

impl Room {
    pub fn new(opts: RoomOptions, real: u64) -> Self {
        if let Some(p) = &opts.practice {
            assert!(director::game(&p.game).is_some(), "unknown game {}", p.game);
        }
        let clock = GameClock::new(real);
        let zero = clock.now().floor();
        let (arena, _) = Arena::new(arena_map("lobby"), ArenaKind::Lobby, 1, -1, &[], false);
        Self {
            id: opts.id.clone(),
            title: opts.title.clone(),
            owner: opts.owner.clone(),
            pin: opts.pin.clone(),
            admitted: BTreeSet::new(),
            players: Vec::new(),
            host: None,
            phase: Phase::Lobby,
            session: None,
            round: None,
            arena,
            arena_id: 1,
            zero,
            playlist: Playlist::default(),
            fill: false,
            clock,
            max: opts.max_players.min(MAX_PLAYERS),
            next_id: 1,
            arena_seq: 1,
            timer: None,
            rng: Rng::new(opts.seed.unwrap_or_else(random_u32)),
            next_seed: None,
            upcoming: None,
            replays: VecDeque::new(),
            history: Vec::new(),
            out: Vec::new(),
            changed: false,
            opts,
        }
    }

    pub fn practice(&self) -> bool {
        self.opts.practice.is_some()
    }

    /// Nobody is in the room (not even someone who may still reconnect).
    pub fn empty(&self) -> bool {
        !self.players.iter().any(|p| !p.bot)
    }

    pub fn permanent(&self) -> bool {
        self.opts.permanent
    }

    /// Who opened the room, as limits per address count it.
    pub fn creator(&self) -> Option<&str> {
        self.opts.creator.as_deref()
    }

    pub fn dev(&self) -> bool {
        self.opts.dev
    }

    /// Game time (ticks).
    pub fn now(&self) -> f64 {
        self.clock.now()
    }

    /// Server tick of the arena's tick 0 (what clients map their timeline with).
    pub fn zero_tick(&self) -> i64 {
        (self.zero - self.clock.offset()).round() as i64
    }

    pub fn player(&self, id: Pid) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    fn player_mut(&mut self, id: Pid) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// The player with this identity, if they are in the room (connected or not).
    pub fn player_of(&self, uid: &str) -> Option<&Player> {
        self.players.iter().find(|p| !p.bot && p.uid == uid)
    }

    pub fn set_rtt(&mut self, id: Pid, rtt: u32) {
        if let Some(p) = self.player_mut(id) {
            p.rtt = rtt;
        }
    }

    // ------------------------------------------------------------------ connections

    /// A player enters: someone new, or (same identity) the one who is already here, on a new connection.
    /// Returns the player id, or None when the room is full.
    pub fn join(&mut self, conn: ConnId, who: Who) -> Option<Pid> {
        let resumed = (!who.uid.is_empty())
            .then(|| self.player_of(&who.uid).map(|p| p.id))
            .flatten();
        if let Some(id) = resumed {
            let p = self.player_mut(id)?;
            let old = p.conn.replace(conn);
            p.disconnected_at = 0;
            if let Some(o) = who.outfit {
                p.outfit = o;
            }
            let name = p.name.clone();
            if let Some(old) = old.filter(|&c| c != conn) {
                self.out.push(Out::Close(old));
            }
            info!(room = %self.id, id, name, "player resumed");
            self.seat_host(id);
            self.welcome(id, true);
            return Some(id);
        }
        if self.players.len() >= self.max {
            // A bot gives up its place to a person.
            let bot = self.spare_bot()?;
            self.drop_player(bot);
        }
        let id = self.next_id;
        self.next_id += 1;
        let name = Some(sanitize_name(&who.name))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Боб {id}"));
        let color = self.free_color(who.color);
        let mut p = Player::new(id, name, color, who.outfit.unwrap_or_default());
        p.uid = who.uid.clone();
        p.bot = false;
        p.conn = Some(conn);
        p.spectator = self.phase != Phase::Lobby;
        info!(room = %self.id, id, name = p.name, practice = self.practice(), "player joined");
        self.players.push(p);
        if !who.uid.is_empty() {
            self.admitted.insert(who.uid);
        }
        self.seat_host(id);
        if self.arena.kind == ArenaKind::Lobby {
            self.arena.add_pawn(id, false);
        }
        self.sync_bots();
        self.welcome(id, false);
        if let Some(pr) = self.opts.practice.clone()
            && self.session.is_none()
        {
            let min = director::game(&pr.game).and_then(|g| g.min_players).unwrap_or(1) as usize;
            let need = pr.bots.max(min.saturating_sub(1));
            for _ in 0..need {
                if self.players.len() >= self.max {
                    break;
                }
                self.add_bot(false);
            }
            self.start_game();
        }
        self.check_round();
        Some(id)
    }

    /// The connection of player `id` is gone: out of the lobby at once, out of a game after the grace period.
    pub fn leave(&mut self, id: Pid, conn: ConnId) {
        let real = self.clock.real();
        let Some(p) = self.player_mut(id).filter(|p| p.conn == Some(conn)) else {
            return;
        };
        p.conn = None;
        p.disconnected_at = real.max(1);
        info!(room = %self.id, id, "player disconnected");
        if self.phase == Phase::Lobby && !self.practice() {
            self.remove_player(id);
            return;
        }
        self.update_host();
        self.send_lobby();
    }

    /// Player `id` leaves for good (back to the room list, or into another room).
    pub fn quit(&mut self, id: Pid) {
        let Some(p) = self.player_mut(id).filter(|p| !p.bot) else {
            return;
        };
        p.conn = None;
        info!(room = %self.id, id, "player left");
        self.remove_player(id);
    }

    pub fn control(&mut self, id: Pid, conn: ConnId, m: &ClientMsg) {
        let real = self.clock.real();
        let Some(p) = self.player_mut(id).filter(|p| p.conn == Some(conn)) else {
            return;
        };
        if real.saturating_sub(p.msg_window) > ticks(1.0) {
            p.msg_window = real;
            p.msg_count = 0;
        }
        p.msg_count += 1;
        let count = p.msg_count;
        if count == MSG_RATE + 1 {
            warn!(room = %self.id, id, "rate limited");
        }
        if count > MSG_RATE {
            return;
        }
        let host = self.host == Some(id);
        let lobby = self.phase == Phase::Lobby;
        match m {
            // Handled before a message reaches the room (see Hub).
            ClientMsg::Hello(_) | ClientMsg::Create { .. } | ClientMsg::Join { .. } | ClientMsg::Leave => {}
            ClientMsg::Name(name) => {
                let name = sanitize_name(name);
                if let Some(p) = self.player_mut(id).filter(|_| !name.is_empty()) {
                    p.name = name;
                    self.send_lobby();
                }
            }
            ClientMsg::Color(c) => {
                if lobby && !self.players.iter().any(|o| o.id != id && o.color == *c) {
                    if let Some(p) = self.player_mut(id) {
                        p.color = *c;
                    }
                    self.send_lobby();
                }
            }
            ClientMsg::Outfit(o) => {
                if let Some(p) = self.player_mut(id).filter(|p| p.outfit != *o) {
                    p.outfit = *o;
                    self.send_lobby();
                }
            }
            ClientMsg::Start => {
                if host && lobby && self.roster().len() >= self.opts.min_players {
                    self.start_game();
                }
            }
            ClientMsg::Abort => {
                if host && !lobby && !self.practice() {
                    self.back_to_lobby();
                }
            }
            ClientMsg::Playlist(pl) => {
                if host && lobby {
                    self.playlist = valid_playlist(pl);
                    self.send_lobby();
                }
            }
            ClientMsg::AddBot => {
                if host && lobby && self.players.len() < self.max {
                    self.add_bot(false);
                    self.send_lobby();
                }
            }
            ClientMsg::RemoveBot(b) => {
                if host && lobby && self.player(*b).is_some_and(|b| b.bot) {
                    self.remove_player(*b);
                }
            }
            ClientMsg::Fill(on) => {
                if host && lobby && !self.practice() {
                    self.fill = *on;
                    // Off: the bots that only filled places go; the ones the host added stay.
                    if !on {
                        let autos: Vec<Pid> = self.players.iter().filter(|b| b.auto).map(|b| b.id).collect();
                        for b in autos {
                            self.drop_player(b);
                        }
                    }
                    self.sync_bots();
                    self.send_lobby();
                }
            }
            ClientMsg::Access { private } => {
                if host && !self.practice() && self.pin.is_some() != *private {
                    self.pin = private.then(make_pin);
                    // A new PIN: whoever is here stays welcome, those who left need it.
                    self.admitted = self.players.iter().filter(|h| !h.bot).map(|h| h.uid.clone()).collect();
                    info!(room = %self.id, by = id, private, "room access changed");
                    self.send_lobby();
                }
            }
            ClientMsg::Host(to) => {
                // The host hands the role over to another connected player (any phase).
                if host && *to != id && self.player(*to).is_some_and(|t| !t.bot && t.conn.is_some()) {
                    self.host = Some(*to);
                    self.send_lobby();
                }
            }
            ClientMsg::Emote(e) => {
                if self.arena.pawn(id).is_some_and(|p| p.status == PawnStatus::Play) {
                    self.broadcast(ServerMsg::Emote { id, e: *e });
                }
            }
            ClientMsg::Chat(text) => {
                let text = sanitize_chat(text);
                let Some(p) = self.player_mut(id) else { return };
                if text.is_empty() || p.chat_at.is_some_and(|at| real.saturating_sub(at) < ticks(CHAT_GAP_S)) {
                    return;
                }
                p.chat_at = Some(real);
                let name = p.name.clone();
                self.broadcast(ServerMsg::Chat { id, name, text });
            }
            ClientMsg::Dev { q, cmd } => {
                if !self.dev() {
                    self.send_to(
                        id,
                        ServerMsg::DevAck {
                            q: *q,
                            ok: false,
                            msg: "dev commands are off".into(),
                        },
                    );
                    return;
                }
                let (ok, msg) = match self.dev_command(id, cmd) {
                    Ok(m) => (true, m),
                    Err(m) => (false, m),
                };
                info!(room = %self.id, id, cmd = cmd.name(), ok, msg, "dev");
                self.arena.note(
                    "dev",
                    Some(id),
                    Some(json!({ "cmd": cmd.name(), "ok": ok, "msg": msg })),
                );
                self.send_to(id, ServerMsg::DevAck { q: *q, ok, msg });
            }
        }
    }

    // ------------------------------------------------------------------ dev commands

    /// Runs a dev command for player `by`: a short result, or why it cannot.
    pub fn dev_command(&mut self, by: Pid, cmd: &DevCmd) -> Result<String, String> {
        let target = |a: &Arena, id: Option<Pid>| {
            let tid = id.unwrap_or(by);
            a.pawn(tid)
                .map(|_| tid)
                .ok_or_else(|| format!("no bean #{tid} in this arena"))
        };
        match cmd {
            DevCmd::SkipIntro => {
                let left = self.zero - self.now();
                if self.arena.kind != ArenaKind::Round || left <= 0.0 {
                    return Ok("already started".into());
                }
                self.warp(left.ceil() + 1.0);
                Ok(format!("skipped {:.0} ms", left * 1000.0 / TICK_RATE as f64))
            }
            DevCmd::Warp { s } => {
                self.warp((s * TICK_RATE as f64).round());
                Ok(format!("warped {s} s"))
            }
            DevCmd::EndRound => {
                if self.phase != Phase::Round || self.round.as_ref().is_none_or(|r| r.over) {
                    return Err("no round running".into());
                }
                self.end_round();
                Ok("round ended".into())
            }
            DevCmd::Start { games, rounds, bots } => {
                let bad: Vec<&str> = games
                    .iter()
                    .filter(|g| director::game(g).is_none())
                    .map(String::as_str)
                    .collect();
                if !bad.is_empty() {
                    return Err(format!("unknown games: {}", bad.join(", ")));
                }
                if self.session.is_some() {
                    self.back_to_lobby();
                }
                // `bots`: exactly that many (the ones left from before are replaced).
                if bots.is_some() {
                    self.fill = false;
                    let old: Vec<Pid> = self.players.iter().filter(|p| p.bot).map(|p| p.id).collect();
                    for b in old {
                        self.remove_player(b);
                    }
                }
                let humans = self.players.iter().filter(|p| !p.bot).count();
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
                        games: games.clone(),
                        rounds: rounds.unwrap_or(games.len() as u32),
                    }
                };
                self.start_game();
                let plan = self.session.as_ref().map(|s| s.plan.join(", ")).unwrap_or_default();
                Ok(format!("started: {plan}"))
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
                    return Err("pause first (rate 0)".into());
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
                        Goto::Checkpoint(i) => format!("this map has no checkpoint {i}"),
                        _ => "this map has no finish".into(),
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
                    return Err("room is full".into());
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
                self.arena.dev_knock(id, *v);
                Ok(format!("#{id} knocked"))
            }
            DevCmd::Grab { actor, target: t, s } => {
                let actor = target(&self.arena, *actor)?;
                let t = target(&self.arena, Some(*t))?;
                self.arena.dev_grab(actor, t, s.unwrap_or(3.0)).map_err(String::from)?;
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

    fn welcome(&mut self, id: Pid, resumed: bool) {
        self.send_to(
            id,
            ServerMsg::Welcome {
                id,
                room: self.id.clone(),
                solo: self.opts.min_players <= 1,
                practice: self.practice(),
                resumed,
            },
        );
        if self.clock.shifted() {
            self.send_to(id, ServerMsg::Clock { rate: self.clock.rate });
        }
        self.send_lobby();
        self.send_to(id, ServerMsg::Arena(self.arena_info(true)));
        if let Some(conn) = self.player(id).and_then(|p| p.conn) {
            for ev in &self.history {
                let ev = MapEventMsg {
                    history: true,
                    ..ev.clone()
                };
                self.out.push(Out::Event(conn, ev));
            }
        }
    }

    // ------------------------------------------------------------------ players, host, bots

    fn free_color(&self, wish: Option<u8>) -> u8 {
        let used = |c: u8| self.players.iter().any(|p| p.color == c);
        if let Some(w) = wish.filter(|&w| (w as usize) < COLORS.len() && !used(w)) {
            return w;
        }
        (0..COLORS.len() as u8).find(|&c| !used(c)).unwrap_or(0)
    }

    fn add_bot(&mut self, auto: bool) -> Pid {
        let id = self.next_id;
        self.next_id += 1;
        let name = bot_name(self.players.iter().map(|p| p.name.as_str()), id);
        let mut p = Player::new(id, name, self.free_color(None), bot_outfit(id));
        p.auto = auto;
        self.players.push(p);
        if self.arena.kind == ArenaKind::Lobby {
            self.arena.add_pawn(id, true);
        }
        id
    }

    /// With "fill with bots" on, the lobby is kept full while someone is there to play with them.
    fn sync_bots(&mut self) {
        if !self.fill || self.phase != Phase::Lobby || self.practice() {
            return;
        }
        if !self.players.iter().any(|p| !p.bot && p.conn.is_some()) {
            return;
        }
        while self.players.len() < self.max {
            self.add_bot(true);
        }
    }

    /// The bot that gives up its place to a person: in a round, one that no longer plays if there is one.
    fn spare_bot(&self) -> Option<Pid> {
        let bots: Vec<Pid> = self.players.iter().rev().filter(|p| p.bot).map(|p| p.id).collect();
        bots.iter()
            .copied()
            .find(|&b| self.arena.pawn(b).is_none_or(|p| p.status != PawnStatus::Play))
            .or(bots.first().copied())
    }

    /// `id` just connected: the room's owner takes the host role back, anyone else may fill a vacancy.
    fn seat_host(&mut self, id: Pid) {
        let uid = self.player(id).map(|p| p.uid.as_str());
        if self.owner.is_some() && uid == self.owner.as_deref() {
            self.host = Some(id);
        } else {
            self.update_host();
        }
    }

    /// The host must be connected: the owner if they are here, else whoever has been here longest.
    fn update_host(&mut self) {
        let cur = self.host.and_then(|h| self.player(h));
        if cur.is_some_and(|c| c.conn.is_some()) {
            return;
        }
        let cur = cur.map(|c| c.id);
        let humans: Vec<&Player> = self.players.iter().filter(|p| !p.bot).collect();
        let next = humans
            .iter()
            .find(|p| p.conn.is_some() && Some(&p.uid) == self.owner.as_ref())
            .or_else(|| humans.iter().find(|p| p.conn.is_some()));
        self.host = next.map(|p| p.id).or(cur).or(humans.first().map(|p| p.id));
    }

    /// Takes a player out of the room and its arena (what that means for the room: `remove_player`).
    fn drop_player(&mut self, id: Pid) -> bool {
        let Some(i) = self.players.iter().position(|p| p.id == id) else {
            return false;
        };
        self.players.remove(i);
        self.arena.remove_pawn(id);
        self.broadcast(ServerMsg::Left(id));
        true
    }

    fn remove_player(&mut self, id: Pid) {
        let bot = self.player(id).is_some_and(|p| p.bot);
        if !self.drop_player(id) {
            return;
        }
        if !bot && self.empty() {
            // The last person left: the bots go too, and the room waits in its lobby.
            self.players.clear();
            self.back_to_lobby();
            return;
        }
        self.update_host();
        self.sync_bots();
        self.send_lobby();
        self.check_round();
    }

    fn later(&mut self, s: f64, t: Timer) {
        self.timer = Some((self.now() + ticks(s) as f64, t));
    }

    // ------------------------------------------------------------------ game flow

    fn new_arena_id(&mut self) -> u32 {
        self.arena_seq = self.arena_seq % 65535 + 1;
        self.arena_seq
    }

    fn make_lobby_arena(&mut self) -> (Arena, f64) {
        let zero = self.now().floor();
        let ids: Vec<Pid> = self.players.iter().map(|p| p.id).collect();
        let (mut arena, _) = Arena::new(arena_map("lobby"), ArenaKind::Lobby, 1, -1, &ids, false);
        for p in &self.players {
            arena.add_pawn(p.id, p.bot);
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
    fn roster(&self) -> Vec<Pid> {
        self.players
            .iter()
            .filter(|p| p.bot || p.conn.is_some())
            .map(|p| p.id)
            .collect()
    }

    fn start_game(&mut self) {
        let mut ids = self.roster();
        ids.truncate(self.max);
        for p in &mut self.players {
            p.score = 0;
            p.stats = GameStats::default();
            p.spectator = false;
        }
        let plan = match &self.opts.practice {
            Some(pr) => vec![director::game(&pr.game).map_or("jump-club", |g| g.id)],
            None => plan_game(ids.len() as u32, &self.playlist, &mut self.rng),
        };
        info!(room = %self.id, players = ids.len(), plan = ?plan, "game started");
        self.session = Some(Session {
            plan,
            index: 0,
            started: ids.len(),
        });
        self.next_round();
    }

    /// The game of the next round, its number and the number of rounds (None: the game is over).
    fn next_game(&self) -> Option<(&'static GameMeta, u32, u32)> {
        let session = self.session.as_ref()?;
        if self.practice() {
            return Some((director::game(session.plan.first()?)?, 1, 1));
        }
        let game = director::game(session.plan.get(session.index)?)?;
        Some((game, session.index as u32 + 1, session.plan.len() as u32))
    }

    /// Arena tick a round is made at (the intro runs before tick 0).
    fn round_start(&self) -> i64 {
        let zero = self.now().floor() + f64::from(self.opts.intro_ticks);
        (self.now() - zero).floor() as i64 - 1
    }

    /// The map's seed and the spawn order.
    fn draw_round(&mut self, mut ids: Vec<Pid>) -> (u32, Vec<Pid>) {
        let seed = self.next_seed.unwrap_or_else(|| (self.rng.next() * 1e9).floor() as u32);
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
        let Some((game, _, _)) = self.next_game() else { return };
        let dev_seed = self.next_seed;
        let (seed, participants) = self.draw_round(self.roster());
        let bots = participants.iter().any(|&id| self.player(id).is_some_and(|p| p.bot));
        let (map, start, ids) = (arena_map(game.id), self.round_start(), participants.clone());
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
                    game: game.id,
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

    fn next_round(&mut self) {
        self.timer = None;
        if self.session.is_none() {
            return;
        }
        let ids = self.roster();
        if ids.is_empty() {
            return self.back_to_lobby();
        }
        let Some((game, index, total)) = self.next_game() else {
            return self.end_game();
        };
        if !self.practice()
            && let Some(session) = self.session.as_mut()
        {
            session.index += 1;
        }
        self.phase = Phase::Round;
        let zero = self.now().floor() + f64::from(self.opts.intro_ticks);
        let start = self.round_start();
        // The round drawn ahead, if the same players are here for it.
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        let (ready, stale): (Option<Upcoming>, Option<Upcoming>) = match self.upcoming.take() {
            Some(u) => {
                let mut theirs = u.participants.clone();
                theirs.sort_unstable();
                if u.game == game.id && u.start == start && theirs == sorted {
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
        self.round = Some(RoundInfo {
            game,
            over: false,
            index,
            total,
        });
        let mut arena = built
            .unwrap_or_else(|| Arena::new(arena_map(game.id), ArenaKind::Round, seed, start, &participants, false).0);
        if !self.opts.eliminate && arena.fall == FallBehaviour::Out {
            arena.fall = FallBehaviour::Spawn;
        }
        if self.dev() {
            arena.record();
        }
        for (i, id) in participants.iter().enumerate() {
            let bot = self.player(*id).is_some_and(|p| p.bot);
            arena.add_pawn_at(*id, bot, Some(i));
        }
        info!(room = %self.id, game = game.id, seed, players = participants.len(), "round");
        self.set_arena((arena, zero));
        self.send_lobby();
    }

    fn time_up(&self) -> bool {
        self.now() > self.zero + ticks(self.arena.map.meta().duration) as f64
    }

    /// The round as the rules see it.
    fn with_view<R>(&mut self, f: impl FnOnce(&RoundView, &mut Rng) -> R) -> Option<R> {
        let game = self.round.as_ref()?.game;
        let time_up = self.time_up();
        let solo = self.session.as_ref().map_or(1, |s| s.started) <= 1;
        let bots: BTreeSet<Pid> = self.players.iter().filter(|p| p.bot).map(|p| p.id).collect();
        let players = &self.players;
        let arena = &self.arena;
        let connected = |id: Pid| players.iter().any(|p| p.id == id);
        let progress = |id: Pid| arena.pawn(id).map_or(f64::NEG_INFINITY, |p| p.progress);
        let view = RoundView {
            genre: game.genre,
            participants: &arena.participants,
            connected: &connected,
            finished: &arena.finished,
            out: &arena.out,
            scores: &arena.scores,
            progress: &progress,
            time_up,
            solo,
            bots: Some(&bots),
        };
        Some(f(&view, &mut self.rng))
    }

    fn check_round(&mut self) {
        let live = self.round.as_ref().is_some_and(|r| !r.over);
        if !live || self.phase != Phase::Round || self.arena.kind != ArenaKind::Round {
            return;
        }
        if self.with_view(|v, _| is_round_over(v)) == Some(true) {
            self.end_round();
        }
    }

    fn end_round(&mut self) {
        let Some(r) = &mut self.round else { return };
        r.over = true;
        let (game, index, total) = (r.game, r.index, r.total);
        self.arena.frozen = true;
        let stats: BTreeMap<Pid, RoundStats> = self.arena.pawns.iter().map(|p| (p.id, p.stats)).collect();
        let totals: BTreeMap<Pid, i64> = self.players.iter().map(|p| (p.id, p.score)).collect();
        let bots: BTreeSet<Pid> = self.players.iter().filter(|p| p.bot).map(|p| p.id).collect();
        let rows = self
            .with_view(|v, rng| score_round(v, &stats, &totals, &bots, Some(rng)))
            .unwrap_or_default();
        let n = rows.len();
        let round_secs = self.arena.time().max(0.0);
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
            if game.genre == Genre::Race {
                g.race_ranks.push(if n > 1 {
                    (row.place - 1) as f64 / (n - 1) as f64
                } else {
                    0.0
                });
            }
            if game.genre == Genre::Survival {
                g.survived += s.out_at.unwrap_or(round_secs);
            }
        }
        let deltas: Vec<(Pid, i64)> = rows.iter().map(|r| (r.id, r.delta)).collect();
        info!(room = %self.id, game = game.id, rows = ?deltas, "round over");
        self.phase = Phase::Results;
        self.broadcast(ServerMsg::RoundEnd {
            game: game.id.into(),
            index,
            total,
            rows,
            practice: self.practice(),
        });
        let wait = if self.practice() { PRACTICE_RESULTS_S } else { RESULTS_S };
        self.later(wait, Timer::NextRound);
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

    fn end_game(&mut self) {
        self.timer = None;
        if let Some(r) = &mut self.round {
            r.over = true;
        }
        let standings = self.standings();
        let Some(winner) = standings.first().map(|s| s.id) else {
            return self.back_to_lobby();
        };
        self.phase = Phase::Podium;
        if let Some(p) = self.player_mut(winner) {
            p.crowns += 1;
        }
        let awards = compute_awards(&self.players.iter().map(|p| (p.id, &p.stats)).collect::<Vec<_>>());
        let totals: Vec<(Pid, i64)> = standings.iter().map(|s| (s.id, s.total)).collect();
        info!(room = %self.id, winner, standings = ?totals, "game over");
        let order: Vec<Pid> = standings.iter().map(|s| s.id).collect();
        let zero = self.now().floor();
        let (mut arena, _) = Arena::new(arena_map("podium"), ArenaKind::Podium, 1, -1, &order, false);
        for (i, id) in order.iter().enumerate() {
            let bot = self.player(*id).is_some_and(|p| p.bot);
            arena.add_pawn_at(*id, bot, Some(i));
        }
        self.set_arena((arena, zero));
        self.broadcast(ServerMsg::GameEnd { standings, awards });
        self.later(PODIUM_S, Timer::BackToLobby);
        self.send_lobby();
    }

    fn back_to_lobby(&mut self) {
        self.timer = None;
        self.phase = Phase::Lobby;
        self.session = None;
        self.round = None;
        if !self.practice() {
            let gone: Vec<Pid> = self
                .players
                .iter()
                .filter(|p| !p.bot && p.conn.is_none())
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

    // ------------------------------------------------------------------ simulation

    /// Advances timers and the simulation to server tick `real`.
    pub fn update(&mut self, real: u64, inputs: &mut dyn Inputs) {
        self.clock.set_real(real);
        let grace = ticks(RECONNECT_GRACE_S);
        let timed_out: Vec<Pid> = self
            .players
            .iter()
            .filter(|p| !p.bot && p.conn.is_none() && p.disconnected_at > 0)
            .filter(|p| real.saturating_sub(p.disconnected_at) > grace)
            .map(|p| p.id)
            .collect();
        for id in timed_out {
            info!(room = %self.id, id, "player timed out");
            self.remove_player(id);
        }
        self.tick_room(inputs, false);
    }

    fn tick_room(&mut self, inputs: &mut dyn Inputs, all: bool) {
        if let Some((at, t)) = self.timer
            && self.now() >= at
        {
            self.timer = None;
            match t {
                Timer::NextRound => self.next_round(),
                Timer::BackToLobby => self.back_to_lobby(),
            }
        }
        let target = (self.now() - self.zero).floor() as i64;
        if !all && target - self.arena.tick > MAX_CATCHUP {
            warn!(room = %self.id, behind = target - self.arena.tick, "arena fell behind");
            self.arena.tick = target - MAX_CATCHUP;
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
        let mut frames: Vec<(Pid, InputFrame)> = self
            .players
            .iter()
            .filter(|p| !p.bot)
            .filter_map(|p| Some((p.id, inputs.frame(p.id, p.conn?, tick).clamped())))
            .collect();
        frames.sort_unstable_by_key(|f| f.0);
        let events = self.arena.step(k, |id| {
            frames
                .binary_search_by_key(&id, |f| f.0)
                .map_or(InputFrame::IDLE, |i| frames[i].1)
        });
        let live = self.phase == Phase::Round && self.round.as_ref().is_some_and(|r| !r.over);
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
                        cause: ko.cause.into(),
                        shortcut: ko.shortcut,
                    };
                    self.map_event(tick, ev, false);
                    if ko.out {
                        self.check_round();
                    }
                }
                ArenaEvent::Emote { id, e } => self.broadcast(ServerMsg::Emote { id, e: e as u8 }),
                ArenaEvent::Event { name, data, keep } => {
                    let ev = MapEventKind::Map {
                        name,
                        data: data.to_string(),
                    };
                    self.map_event(tick, ev, keep);
                }
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
        for p in &self.players {
            if let Some(c) = p.conn {
                self.out.push(Out::Event(c, msg.clone()));
            }
        }
        if keep {
            self.history.push(msg);
        }
    }

    /// Seconds of game time until the room's timer (next round, back to the lobby) goes off.
    pub fn timer_in(&self) -> Option<f64> {
        self.timer.map(|(at, _)| (at - self.now()) / TICK_RATE as f64)
    }

    /// Seconds of game time until the round's time is up (rounds with a time limit).
    pub fn ends_in(&self) -> Option<f64> {
        let d = self.arena.map.meta().duration;
        (self.arena.kind == ArenaKind::Round && d > 0.0)
            .then(|| (self.zero + ticks(d) as f64 - self.now()) / TICK_RATE as f64)
    }

    /// A recorded round: the last finished ones (0 newest), or (`None`) the one running now.
    pub fn debug_replay(&self, i: Option<usize>) -> Option<Recording> {
        match i {
            None => self.arena.take_recording(),
            Some(i) => self.replays.iter().rev().nth(i).cloned(),
        }
    }

    // ------------------------------------------------------------------ messages

    pub fn arena_info(&self, late: bool) -> ArenaInfo {
        let a = &self.arena;
        let round = self.round.as_ref().filter(|_| a.kind == ArenaKind::Round);
        ArenaInfo {
            id: self.arena_id,
            kind: a.kind,
            game: a.map.meta().id.into(),
            participants: a.participants.clone(),
            index: round.map_or(0, |r| r.index),
            total: round.map_or(0, |r| r.total),
            practice: self.practice(),
            late,
            finished: a.finished.clone(),
            out: a.out.clone(),
            scores: a.scores.iter().map(|(&k, &v)| (k, v)).collect(),
        }
    }

    /// The room's line in the room list.
    pub fn info(&self) -> RoomInfo {
        let humans = self.players.iter().filter(|p| !p.bot).count();
        RoomInfo {
            id: self.id.clone(),
            title: self.title.clone(),
            private: self.pin.is_some(),
            host: self
                .host
                .and_then(|h| self.player(h))
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            players: humans as u32,
            bots: (self.players.len() - humans) as u32,
            max: self.max as u32,
            phase: self.phase,
        }
    }

    pub fn lobby_msg(&self) -> Lobby {
        Lobby {
            room: RoomRef {
                id: self.id.clone(),
                title: self.title.clone(),
                private: self.pin.is_some(),
            },
            phase: self.phase,
            host: self.host,
            min: self.opts.min_players as u32,
            max: self.max as u32,
            players: self
                .players
                .iter()
                .map(|p| LobbyPlayer {
                    id: p.id,
                    name: p.name.clone(),
                    color: p.color,
                    outfit: p.outfit,
                    score: p.score,
                    crowns: p.crowns,
                    spectator: p.spectator,
                    bot: p.bot,
                    connected: p.bot || p.conn.is_some(),
                    ping: p.rtt,
                })
                .collect(),
            playlist: self.playlist.clone(),
            fill: self.fill,
            pin: None,
            next: self
                .timer
                .map(|(at, _)| (at - self.clock.offset()).round().max(0.0) as u32),
        }
    }

    fn send_lobby(&mut self) {
        let msg = self.lobby_msg();
        // Only the host is told the PIN: the others ask them for it.
        for p in &self.players {
            let Some(c) = p.conn else { continue };
            let mut m = msg.clone();
            if Some(p.id) == self.host {
                m.pin.clone_from(&self.pin);
            }
            self.out.push(Out::Msg(c, ServerMsg::Lobby(m)));
        }
        self.changed = true;
    }

    fn send_to(&mut self, id: Pid, msg: ServerMsg) {
        if let Some(c) = self.player(id).and_then(|p| p.conn) {
            self.out.push(Out::Msg(c, msg));
        }
    }

    fn broadcast(&mut self, msg: ServerMsg) {
        for p in &self.players {
            if let Some(c) = p.conn {
                self.out.push(Out::Msg(c, msg.clone()));
            }
        }
    }
}
