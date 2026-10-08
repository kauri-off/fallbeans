//! A lobby and the games played from it: players and bots, the host,
//! rounds, scores, chat, dev commands. Time is in server ticks; the arena runs on the room's game clock.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Once, OnceLock};
use std::thread::JoinHandle;

use bevy::log::{info, warn};
use fb_arena::{Arena, ArenaEvent, ArenaKind, DevPlace, FallBehaviour, PawnStatus, PreparedNav, Recording, RoundStats};
use fb_maps::director::{self, plan_game, valid_playlist};
use fb_maps::lobby::Lobby as LobbyMap;
use fb_maps::podium::Podium as PodiumMap;
use fb_proto::*;
use fb_shared::game::{GameMeta, Genre};
use fb_shared::input::InputFrame;
use fb_shared::outfit::bot_outfit;
use fb_shared::rng::{Rng, shuffle};
use fb_shared::rules::RoundView;
use fb_shared::text::{sanitize_chat, sanitize_person_name};
use fb_shared::*;
use fb_sim::map::MapDef;
use fb_sim::math::V3;
use serde_json::json;

use super::awards::{GameStats, compute_awards};
use super::clock::GameClock;
use super::players::{Kind, Player, bot_name};
use super::{ConnId, Inputs, NoInputs, Out, Uid, ticks};
use crate::auth::{AddrKey, random_bytes};

/// Control messages per second from one player.
const MSG_RATE: u32 = 60;
/// Shortest time between two chat lines of one player (s).
const CHAT_GAP_S: f64 = 0.7;
/// Ticks the arena may fall behind before it skips ahead instead of catching up.
const MAX_CATCHUP: i64 = 60;
/// Shortest time between two starts by the host (s): building a round's arena costs the shared tick up to ≈6 ms.
const START_GAP_S: f64 = 1.0;
/// Name, colour and outfit changes go out in a lobby at most this often (s): each one is a lobby to everybody.
const PROFILE_EVERY_S: f64 = 0.5;
/// A player whose connection dropped in the lobby keeps their place (and crowns) this long (s).
const LOBBY_GRACE_S: f64 = 10.0;
/// The longest dev warp (s): it simulates every tick at once, and the tick is every room's.
const WARP_MAX_S: f64 = 30.0;

/// A game's map, looked up once by its id.
#[derive(Clone, Copy)]
pub struct Game(&'static dyn MapDef);

impl Game {
    /// The game (not the lobby or the podium) of this id.
    pub fn by_id(id: &str) -> Option<Self> {
        fb_maps::GAMES.iter().copied().find(|m| m.meta().id == id).map(Self)
    }

    pub fn meta(self) -> &'static GameMeta {
        self.0.meta()
    }

    pub fn id(self) -> &'static str {
        self.meta().id
    }

    pub fn map(self) -> &'static dyn MapDef {
        self.0
    }

    /// The rounds of a game for so many players (`director::plan_game`).
    fn plan(players: u32, playlist: &Playlist, rng: &mut Rng) -> Vec<Self> {
        plan_game(players, playlist, rng)
            .into_iter()
            .filter_map(Self::by_id)
            .collect()
    }
}

impl PartialEq for Game {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl core::fmt::Debug for Game {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.id())
    }
}

#[derive(Clone, Debug)]
pub struct Practice {
    pub game: Game,
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
    pub owner: Option<Uid>,
    /// PIN of a private room.
    pub pin: Option<String>,
    /// Kept when nobody is in it (the dev server's room).
    pub permanent: bool,
    /// Who opened it, as limits per address count (`auth::address_key`; None: this machine or nobody).
    pub creator: Option<AddrKey>,
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
    pub plan: Vec<Game>,
    /// Rounds started so far.
    pub index: usize,
    /// Players the game started with (bots too; the debug API shows it).
    pub started: usize,
}

impl Session {
    /// Planned games that need more players than there are now are skipped (one of two left: no tail tag).
    fn skip_unfit(&mut self, players: usize) -> Vec<&'static str> {
        let mut skipped = Vec::new();
        while let Some(&g) = self.plan.get(self.index) {
            if director::fits(g.meta(), players as u32) {
                break;
            }
            skipped.push(g.id());
            self.index += 1;
        }
        skipped
    }

    /// The game of the next round, its number and the number of rounds (None: the game is over). Practice
    /// plays its one game over and over.
    fn next(&self, practice: bool) -> Option<(Game, u32, u32)> {
        if practice {
            return Some((*self.plan.first()?, 1, 1));
        }
        let game = *self.plan.get(self.index)?;
        Some((game, self.index as u32 + 1, self.plan.len() as u32))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RoundInfo {
    pub game: Game,
    pub index: u32,
    pub total: u32,
}

/// Where the room is: its lobby, or a game.
#[derive(Clone, Debug)]
pub enum Stage {
    Lobby,
    Game { session: Session, step: Step },
}

#[derive(Clone, Copy, Debug)]
pub enum Step {
    Round(RoundInfo),
    /// The round's results show until game tick `next` (the next round, or the podium).
    Results {
        round: RoundInfo,
        next: f64,
    },
    /// The game is over: back to the lobby at game tick `lobby`.
    Podium {
        lobby: f64,
    },
}

/// The next round, drawn when the last one ended and built on another thread while the results show.
struct Upcoming {
    game: Game,
    seed: u32,
    start: i64,
    participants: Vec<Pid>,
    dev_seed: Option<u32>,
    arena: std::thread::JoinHandle<Arena>,
}

/// Who enters a room.
#[derive(Clone, Debug)]
pub struct Who {
    pub uid: Uid,
    pub name: String,
    pub color: Option<u8>,
    pub outfit: Option<Outfit>,
}

/// What a dev command answers to question `q`: what it did, or why it could not.
struct DevReply {
    q: Option<u32>,
    result: Result<String, String>,
}

impl From<DevReply> for ServerMsg {
    fn from(r: DevReply) -> Self {
        ServerMsg::DevAck {
            q: r.q,
            result: r.result,
        }
    }
}

pub fn make_pin() -> String {
    let n = u32::from_le_bytes(random_bytes());
    format!("{:04}", n % 10u32.pow(ROOM_PIN_DIGITS as u32))
}

pub struct Room {
    pub id: String,
    pub title: String,
    /// Identity of the room's creator (None: nobody's, the dev room).
    pub owner: Option<Uid>,
    /// PIN of a private room; None when anyone may enter.
    pub pin: Option<String>,
    /// Kept when nobody is in it (the dev server's room).
    permanent: bool,
    /// Who opened it, as limits per address count (`auth::address_key`; None: this machine or nobody).
    creator: Option<AddrKey>,
    min_players: usize,
    practice: Option<Practice>,
    intro_ticks: u32,
    dev: bool,
    eliminate: bool,
    /// Identities the room let in: they come back without the PIN.
    pub admitted: BTreeSet<Uid>,
    /// In the order they came.
    pub players: Vec<Player>,
    /// The acting host: the owner whenever they are in the room; while they are away (or after they handed
    /// the role over) another connected player.
    pub host: Option<Pid>,
    /// The owner handed the host role over (`Host`): coming back they do not take it back.
    handed_over: bool,
    pub stage: Stage,
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
    /// Real tick of the host's last start.
    started_at: Option<u64>,
    rng: Rng,
    /// Dev: seed of the next round's map.
    next_seed: Option<u32>,
    upcoming: Option<Upcoming>,
    /// The bots' grid of a round built on the tick, being built on another thread (`nav_ahead`), and the
    /// arena it is for.
    nav_job: Option<(u32, JoinHandle<Option<PreparedNav>>)>,
    /// Dev: the last rounds as played (newest last).
    pub replays: VecDeque<Recording>,
    /// Map events of this arena that someone arriving later must hear of.
    history: Vec<MapEventMsg>,
    /// What to send.
    pub out: Vec<Out>,
    /// Something the room list shows changed (players, host, phase, access).
    pub changed: bool,
    /// A name, colour or outfit changed since the last lobby; and the real tick that change last went out.
    lobby_due: bool,
    profile_sent: u64,
    /// Real tick of the last input from a person that was not idle (practice rooms close without one).
    active_at: u64,
    /// Server tick since which nobody is in the room (it closes after ROOM_EMPTY_S, `Hub::sweep`).
    pub empty_since: Option<u64>,
}

/// The bots' grid of the lobby, which is the same map with the same seed in every room: built once, off the
/// tick (`prebuild_lobby_nav`), and copied into each lobby.
static LOBBY_NAV: OnceLock<PreparedNav> = OnceLock::new();

/// Builds the lobby's grid on a thread of its own at the server's start (a lobby made before it is done builds
/// its own on first need, as without it).
pub fn prebuild_lobby_nav() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let built = std::thread::Builder::new().name("lobby nav".into()).spawn(|| {
            let (mut arena, _) = Arena::new(&LobbyMap, ArenaKind::Lobby, 1, -1, &[], false);
            arena.prepare_nav();
            if let Some(pre) = arena.prepared_nav() {
                let _ = LOBBY_NAV.set(pre.clone());
            }
        });
        if let Err(e) = built {
            warn!("no thread for the lobby's bot grid: {e}");
        }
    });
}

/// A lobby's arena, with the bots' grid from the start if it is built.
fn lobby_arena(ids: &[Pid]) -> Arena {
    let (mut arena, _) = Arena::new(&LobbyMap, ArenaKind::Lobby, 1, -1, ids, false);
    if let Some(pre) = LOBBY_NAV.get() {
        arena.give_nav(pre.clone());
    }
    arena
}

impl Room {
    pub fn new(opts: RoomOptions, real: u64) -> Self {
        let RoomOptions {
            min_players,
            max_players,
            practice,
            seed,
            intro_ticks,
            dev,
            eliminate,
            id,
            title,
            owner,
            pin,
            permanent,
            creator,
        } = opts;
        let clock = GameClock::new(real);
        let zero = clock.now().floor();
        let arena = lobby_arena(&[]);
        Self {
            id,
            title,
            owner,
            pin,
            permanent,
            creator,
            min_players,
            practice,
            intro_ticks,
            dev,
            eliminate,
            admitted: BTreeSet::new(),
            players: Vec::new(),
            host: None,
            handed_over: false,
            stage: Stage::Lobby,
            arena,
            arena_id: 1,
            zero,
            playlist: Playlist::default(),
            fill: false,
            clock,
            max: max_players.min(MAX_PLAYERS),
            next_id: 1,
            arena_seq: 1,
            started_at: None,
            rng: Rng::new(seed.unwrap_or_else(|| u32::from_le_bytes(random_bytes()))),
            next_seed: None,
            upcoming: None,
            nav_job: None,
            replays: VecDeque::new(),
            history: Vec::new(),
            out: Vec::new(),
            changed: false,
            lobby_due: false,
            profile_sent: 0,
            active_at: real,
            empty_since: None,
        }
    }

    /// Real tick of the last input from a person that was not idle (or of the room's opening).
    pub fn active_at(&self) -> u64 {
        self.active_at
    }

    pub fn practice(&self) -> bool {
        self.practice.is_some()
    }

    /// Nobody is in the room (not even someone who may still reconnect).
    pub fn empty(&self) -> bool {
        self.players.iter().all(Player::is_bot)
    }

    pub fn permanent(&self) -> bool {
        self.permanent
    }

    /// Who opened the room, as limits per address count it.
    pub fn creator(&self) -> Option<AddrKey> {
        self.creator
    }

    pub fn dev(&self) -> bool {
        self.dev
    }

    pub fn phase(&self) -> Phase {
        match &self.stage {
            Stage::Lobby => Phase::Lobby,
            Stage::Game { step, .. } => match step {
                Step::Round(_) => Phase::Round,
                Step::Results { .. } => Phase::Results,
                Step::Podium { .. } => Phase::Podium,
            },
        }
    }

    pub fn session(&self) -> Option<&Session> {
        match &self.stage {
            Stage::Game { session, .. } => Some(session),
            Stage::Lobby => None,
        }
    }

    /// The round being played or whose results show.
    pub fn round(&self) -> Option<&RoundInfo> {
        match &self.stage {
            Stage::Game {
                step: Step::Round(round) | Step::Results { round, .. },
                ..
            } => Some(round),
            _ => None,
        }
    }

    /// A round is being played (not over).
    pub fn round_live(&self) -> bool {
        matches!(
            self.stage,
            Stage::Game {
                step: Step::Round(_),
                ..
            }
        )
    }

    /// Game tick the room moves on at (results: the next round; podium: the lobby).
    fn timer(&self) -> Option<f64> {
        match self.stage {
            Stage::Game {
                step: Step::Results { next: at, .. } | Step::Podium { lobby: at },
                ..
            } => Some(at),
            _ => None,
        }
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
    pub fn player_of(&self, uid: &Uid) -> Option<&Player> {
        self.players.iter().find(|p| p.uid() == Some(uid))
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
        if let Some(id) = self.player_of(&who.uid).map(|p| p.id) {
            let p = self.player_mut(id)?;
            let Kind::Human {
                conn: c,
                disconnected_at,
                ..
            } = &mut p.kind
            else {
                return None;
            };
            let old = c.replace(conn);
            *disconnected_at = None;
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
        let name = Some(sanitize_person_name(&who.name))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Боб {id}"));
        let name = self.unique_name(id, name);
        let color = self.free_color(who.color);
        let outfit = who.outfit.unwrap_or_default();
        let mut p = Player::human(id, name, color, outfit, who.uid.clone(), conn);
        p.spectator = !matches!(self.stage, Stage::Lobby);
        self.active_at = self.clock.real();
        info!(room = %self.id, id, name = p.name, practice = self.practice(), "player joined");
        self.players.push(p);
        self.admitted.insert(who.uid);
        self.seat_host(id);
        if self.arena.kind == ArenaKind::Lobby {
            self.arena.add_pawn(id, false);
        }
        self.sync_bots();
        self.welcome(id, false);
        if let Some(pr) = self.practice.clone()
            && matches!(self.stage, Stage::Lobby)
        {
            let min = pr.game.meta().min_players as usize;
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

    /// The connection of player `id` is gone: they keep their place a while to come back to (LOBBY_GRACE_S in
    /// the lobby, RECONNECT_GRACE_S in a game; `update`).
    pub fn leave(&mut self, id: Pid, conn: ConnId) {
        let real = self.clock.real();
        let Some(p) = self.player_mut(id).filter(|p| p.conn() == Some(conn)) else {
            return;
        };
        if let Kind::Human {
            conn, disconnected_at, ..
        } = &mut p.kind
        {
            *conn = None;
            *disconnected_at = Some(real);
        }
        info!(room = %self.id, id, "player disconnected");
        self.update_host();
        self.send_lobby();
    }

    /// Player `id` leaves for good (back to the room list, or into another room).
    pub fn quit(&mut self, id: Pid) {
        let Some(p) = self.player_mut(id) else { return };
        let Kind::Human { conn, .. } = &mut p.kind else {
            return;
        };
        *conn = None;
        info!(room = %self.id, id, "player left");
        self.remove_player(id);
    }

    pub fn control(&mut self, id: Pid, conn: ConnId, m: &ClientMsg) {
        let real = self.clock.real();
        let Some(p) = self.player_mut(id).filter(|p| p.conn() == Some(conn)) else {
            return;
        };
        let count = p.msgs.hit(real);
        // (Once a second at most while it lasts, and less often the longer it goes on.)
        let hushed = (count == MSG_RATE + 1)
            .then(|| p.rate_log.hit(super::secs(real)))
            .flatten();
        if let Some(hushed) = hushed {
            warn!(room = %self.id, id, hushed, "rate limited");
        }
        if count > MSG_RATE {
            return;
        }
        let host = self.host == Some(id);
        let lobby = matches!(self.stage, Stage::Lobby);
        match m {
            // Handled before a message reaches the room (see Hub).
            ClientMsg::Hello(_) | ClientMsg::Create { .. } | ClientMsg::Join { .. } | ClientMsg::Leave => {}
            // Name, colour and outfit change at once and go out together, at most every PROFILE_EVERY_S (`update`).
            ClientMsg::Name(name) => {
                let name = sanitize_person_name(name);
                if !name.is_empty() {
                    let name = self.unique_name(id, name);
                    if let Some(p) = self.player_mut(id).filter(|p| p.name != name) {
                        p.name = name;
                        self.lobby_due = true;
                    }
                }
            }
            ClientMsg::Color(c) => {
                if lobby
                    && !self.players.iter().any(|o| o.id != id && o.color == *c)
                    && let Some(p) = self.player_mut(id).filter(|p| p.color != *c)
                {
                    p.color = *c;
                    self.lobby_due = true;
                }
            }
            ClientMsg::Outfit(o) => {
                if let Some(p) = self.player_mut(id).filter(|p| p.outfit != *o) {
                    p.outfit = *o;
                    self.lobby_due = true;
                }
            }
            ClientMsg::Start => {
                let early = self
                    .started_at
                    .is_some_and(|at| real.saturating_sub(at) < ticks(START_GAP_S));
                if host && lobby && !early && self.roster().len() >= self.min_players {
                    self.started_at = Some(real);
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
                if host && lobby && self.player(*b).is_some_and(Player::is_bot) {
                    self.remove_player(*b);
                }
            }
            ClientMsg::Fill(on) => {
                if host && lobby && !self.practice() {
                    self.fill = *on;
                    // Off: the bots that only filled places go; the ones the host added stay.
                    if !on {
                        let autos: Vec<Pid> = self.players.iter().filter(|b| b.auto()).map(|b| b.id).collect();
                        for b in autos {
                            self.drop_player(b);
                        }
                    }
                    self.sync_bots();
                    self.send_lobby();
                }
            }
            ClientMsg::Access { private } => {
                // Not in a room nobody owns (the dev room, `--open-rooms`): its stand-in host could lock it and
                // leave, and nobody would know the PIN until a restart.
                if host && !self.practice() && self.owner.is_some() && self.pin.is_some() != *private {
                    self.pin = private.then(make_pin);
                    // A new PIN: whoever is here stays welcome, those who left need it.
                    self.admitted = self.players.iter().filter_map(|h| h.uid().cloned()).collect();
                    info!(room = %self.id, by = id, private, "room access changed");
                    self.send_lobby();
                }
            }
            ClientMsg::Host(to) => {
                // The host hands the role over to another connected player (any phase).
                if host && *to != id && self.player(*to).is_some_and(|t| t.conn().is_some()) {
                    // Given away by the owner, it stays given when they reconnect; given back, it is theirs again.
                    if self.is_owner(id) {
                        self.handed_over = true;
                    }
                    if self.is_owner(*to) {
                        self.handed_over = false;
                    }
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
                let result = if !self.dev() {
                    Err("dev commands are off".into())
                } else if !host {
                    Err("dev commands are the host's".into())
                } else {
                    let result = self.dev_command(id, cmd);
                    let ok = result.is_ok();
                    let (Ok(msg) | Err(msg)) = &result;
                    info!(room = %self.id, id, cmd = cmd.name(), ok, msg, "dev");
                    self.arena.note(
                        "dev",
                        Some(id),
                        Some(json!({ "cmd": cmd.name(), "ok": ok, "msg": msg })),
                    );
                    result
                };
                self.send_to(id, DevReply { q: *q, result }.into());
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
                if !s.is_finite() || *s <= 0.0 {
                    return Err("warp by some seconds".into());
                }
                let capped = s.min(WARP_MAX_S);
                self.warp((capped * TICK_RATE as f64).round());
                Ok(if capped < *s {
                    format!("warped {capped} s (at most {WARP_MAX_S} s at once)")
                } else {
                    format!("warped {capped} s")
                })
            }
            DevCmd::EndRound => {
                if !self.round_live() {
                    return Err("no round running".into());
                }
                self.end_round();
                Ok("round ended".into())
            }
            DevCmd::Start { games, rounds, bots } => {
                let bad: Vec<&str> = games
                    .iter()
                    .filter(|g| Game::by_id(g).is_none())
                    .map(String::as_str)
                    .collect();
                if !bad.is_empty() {
                    return Err(format!("unknown games: {}", bad.join(", ")));
                }
                if !matches!(self.stage, Stage::Lobby) {
                    self.back_to_lobby();
                }
                // `bots`: exactly that many (the ones left from before are replaced).
                if bots.is_some() {
                    self.fill = false;
                    let old: Vec<Pid> = self.players.iter().filter(|p| p.is_bot()).map(|p| p.id).collect();
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
                        games: games.clone(),
                        rounds: rounds.unwrap_or(games.len() as u32),
                    }
                };
                self.start_game();
                let plan: Vec<&str> = self
                    .session()
                    .map_or(Vec::new(), |s| s.plan.iter().map(|g| g.id()).collect());
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
                self.arena.dev_knock(id, V3::from_array(*v));
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
                solo: self.min_players <= 1,
                practice: self.practice(),
                resumed,
            },
        );
        if self.clock.shifted() {
            self.send_to(id, ServerMsg::Clock { rate: self.clock.rate });
        }
        self.send_lobby();
        self.send_to(id, ServerMsg::Arena(self.arena_info(true)));
        if let Some(conn) = self.player(id).and_then(Player::conn) {
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
        let p = Player::bot(id, name, self.free_color(None), bot_outfit(id), auto);
        self.players.push(p);
        if self.arena.kind == ArenaKind::Lobby {
            self.arena.add_pawn(id, true);
        }
        id
    }

    /// With "fill with bots" on, the lobby is kept full while someone is there to play with them.
    fn sync_bots(&mut self) {
        if !self.fill || !matches!(self.stage, Stage::Lobby) || self.practice() {
            return;
        }
        if !self.players.iter().any(|p| p.conn().is_some()) {
            return;
        }
        while self.players.len() < self.max {
            self.add_bot(true);
        }
    }

    /// The bot that gives up its place to a person: in a round, one that no longer plays if there is one.
    fn spare_bot(&self) -> Option<Pid> {
        let bots: Vec<Pid> = self.players.iter().rev().filter(|p| p.is_bot()).map(|p| p.id).collect();
        bots.iter()
            .copied()
            .find(|&b| self.arena.pawn(b).is_none_or(|p| p.status != PawnStatus::Play))
            .or(bots.first().copied())
    }

    /// `id` just connected: the room's owner takes the host role back (unless they gave it away), anyone else
    /// may fill a vacancy.
    fn seat_host(&mut self, id: Pid) {
        if self.is_owner(id) && !self.handed_over {
            self.host = Some(id);
        } else {
            self.update_host();
        }
    }

    /// Player `id` is the person who owns the room.
    fn is_owner(&self, id: Pid) -> bool {
        self.owner.is_some() && self.player(id).and_then(Player::uid) == self.owner.as_ref()
    }

    /// The host must be connected: the owner if they are here, else whoever has been here longest.
    fn update_host(&mut self) {
        let cur = self.host.and_then(|h| self.player(h));
        if cur.is_some_and(|c| c.conn().is_some()) {
            return;
        }
        let cur = cur.map(|c| c.id);
        let humans: Vec<&Player> = self.players.iter().filter(|p| !p.is_bot()).collect();
        let next = humans
            .iter()
            .find(|p| p.conn().is_some() && p.uid() == self.owner.as_ref())
            .or_else(|| humans.iter().find(|p| p.conn().is_some()));
        self.host = next.map(|p| p.id).or(cur).or(humans.first().map(|p| p.id));
        // The role is back with the owner: a handover before this one no longer counts.
        if self.host.is_some_and(|h| self.is_owner(h)) {
            self.handed_over = false;
        }
    }

    /// `name`, or with a number after it when someone else in the room is already called so.
    fn unique_name(&self, id: Pid, name: String) -> String {
        let taken = |n: &str| {
            let n = n.to_lowercase();
            self.players.iter().any(|p| p.id != id && p.name.to_lowercase() == n)
        };
        if !taken(&name) {
            return name;
        }
        for k in 2..=MAX_PLAYERS + 1 {
            let suffix = format!(" {k}");
            let keep = NAME_MAX.saturating_sub(suffix.chars().count());
            let base: String = name.chars().take(keep).collect();
            let candidate = format!("{}{suffix}", base.trim_end());
            if !taken(&candidate) {
                return candidate;
            }
        }
        name
    }

    /// Someone kept guessing the PIN (`Hub::join`): a new one, which the host sees in the lobby. Those the room let
    /// in before still come back without it.
    pub fn new_pin(&mut self) {
        if self.pin.is_none() {
            return;
        }
        self.pin = Some(make_pin());
        warn!(room = %self.id, "too many wrong PINs: the room has a new one");
        if let Some(h) = self.host {
            let text = "Кто-то подбирал PIN-код комнаты — он сменился, новый видно в лобби".into();
            self.send_to(h, ServerMsg::Notice(text));
        }
        self.send_lobby();
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
        let bot = self.player(id).is_some_and(Player::is_bot);
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

    /// Game tick `s` seconds from now.
    fn after(&self, s: f64) -> f64 {
        self.now() + ticks(s) as f64
    }

    // ------------------------------------------------------------------ game flow

    fn new_arena_id(&mut self) -> u32 {
        self.arena_seq = self.arena_seq % 65535 + 1;
        self.arena_seq
    }

    fn make_lobby_arena(&mut self) -> (Arena, f64) {
        let zero = self.now().floor();
        let ids: Vec<Pid> = self.players.iter().map(|p| p.id).collect();
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
    fn roster(&self) -> Vec<Pid> {
        self.players.iter().filter(|p| p.present()).map(|p| p.id).collect()
    }

    fn start_game(&mut self) {
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
    fn draw_round(&mut self, mut ids: Vec<Pid>) -> (u32, Vec<Pid>) {
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

    fn next_round(&mut self, mut session: Session) {
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
    fn nav_ahead(&mut self, game: Game, seed: u32, start: i64, ids: Vec<Pid>) {
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
    fn take_nav(&mut self) {
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
        let bots: BTreeSet<Pid> = self.players.iter().filter(|p| p.is_bot()).map(|p| p.id).collect();
        let players = &self.players;
        let arena = &self.arena;
        let connected = |id: Pid| players.iter().any(|p| p.id == id);
        let progress = |id: Pid| arena.pawn(id).map_or(f64::NEG_INFINITY, |p| p.progress);
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

    fn check_round(&mut self) {
        if !self.round_live() || self.arena.kind != ArenaKind::Round {
            return;
        }
        if self.with_view(|v, _| v.is_round_over()) == Some(true) {
            self.end_round();
        }
    }

    fn end_round(&mut self) {
        let wait = if self.practice() { PRACTICE_RESULTS_S } else { RESULTS_S };
        let next = self.after(wait);
        let Stage::Game { step, .. } = &mut self.stage else {
            return;
        };
        let Step::Round(round) = *step else { return };
        *step = Step::Results { round, next };
        let RoundInfo { game, index, total } = round;
        self.arena.freeze();
        let stats: BTreeMap<Pid, RoundStats> = self.arena.pawns.iter().map(|p| (p.id, p.stats)).collect();
        let totals: BTreeMap<Pid, i64> = self.players.iter().map(|p| (p.id, p.score)).collect();
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
        let deltas: Vec<(Pid, i64)> = rows.iter().map(|r| (r.id, r.delta)).collect();
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
        let totals: Vec<(Pid, i64)> = standings.iter().map(|s| (s.id, s.total)).collect();
        info!(room = %self.id, winner, standings = ?totals, "game over");
        let order: Vec<Pid> = standings.iter().map(|s| s.id).collect();
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

    fn back_to_lobby(&mut self) {
        self.stage = Stage::Lobby;
        if !self.practice() {
            let gone: Vec<Pid> = self
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

    // ------------------------------------------------------------------ simulation

    /// Advances timers and the simulation to server tick `real`.
    pub fn update(&mut self, real: u64, inputs: &mut dyn Inputs) {
        self.clock.set_real(real);
        let grace = ticks(if matches!(self.stage, Stage::Lobby) {
            LOBBY_GRACE_S
        } else {
            RECONNECT_GRACE_S
        });
        let timed_out: Vec<Pid> = self
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
            info!(room = %self.id, id, "player timed out");
            self.remove_player(id);
        }
        self.tick_room(inputs, false);
        if self.lobby_due && real.saturating_sub(self.profile_sent) >= ticks(PROFILE_EVERY_S) {
            self.profile_sent = real;
            self.send_lobby();
        }
    }

    fn tick_room(&mut self, inputs: &mut dyn Inputs, all: bool) {
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

    /// Seconds of game time until the room's timer (next round, back to the lobby) goes off.
    pub fn timer_in(&self) -> Option<f64> {
        self.timer().map(|at| (at - self.now()) / TICK_RATE as f64)
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
        let round = self.round().filter(|_| a.kind == ArenaKind::Round);
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
        let humans = self.players.iter().filter(|p| !p.is_bot()).count();
        RoomInfo {
            id: self.id.clone(),
            title: self.title.clone(),
            private: self.pin.is_some(),
            host: self.host.and_then(|h| self.player(h)).map(|p| p.name.clone()),
            players: humans as u32,
            bots: (self.players.len() - humans) as u32,
            max: self.max as u32,
            phase: self.phase(),
        }
    }

    pub fn lobby_msg(&self) -> Lobby {
        Lobby {
            room: RoomRef {
                id: (!self.practice()).then(|| self.id.clone()),
                title: self.title.clone(),
                private: self.pin.is_some(),
            },
            phase: self.phase(),
            host: self.host,
            min: self.min_players as u32,
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
                    bot: p.is_bot(),
                    connected: p.present(),
                    ping: p.rtt,
                })
                .collect(),
            playlist: self.playlist.clone(),
            fill: self.fill,
            pin: None,
            next: self
                .timer()
                .map(|at| (at - self.clock.offset()).round().max(0.0) as u32),
        }
    }

    fn send_lobby(&mut self) {
        let msg = self.lobby_msg();
        // Only the host is told the PIN: the others ask them for it.
        for p in &self.players {
            let Some(c) = p.conn() else { continue };
            let mut m = msg.clone();
            if Some(p.id) == self.host {
                m.pin.clone_from(&self.pin);
            }
            self.out.push(Out::Msg(c, ServerMsg::Lobby(m)));
        }
        self.changed = true;
        self.lobby_due = false;
    }

    fn send_to(&mut self, id: Pid, msg: ServerMsg) {
        if let Some(c) = self.player(id).and_then(Player::conn) {
            self.out.push(Out::Msg(c, msg));
        }
    }

    fn broadcast(&mut self, msg: ServerMsg) {
        for c in self.players.iter().filter_map(Player::conn) {
            self.out.push(Out::Msg(c, msg.clone()));
        }
    }
}
