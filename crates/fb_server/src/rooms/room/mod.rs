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
use super::{ConnId, Inputs, NoInputs, Out, Uid, secs, ticks};
use crate::auth::{AddrKey, random_bytes};

mod connections;
mod dev;
mod flow;
mod sim;

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
    participants: Vec<PlayerId>,
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
    pub host: Option<PlayerId>,
    /// The owner handed the host role over (`Host`): coming back they do not take it back.
    handed_over: bool,
    pub stage: Stage,
    pub arena: Arena,
    /// Changes with every new arena (clients drop what belongs to an older one).
    pub arena_id: u32,
    /// `--trace-hits`: the arenas' lines (`Arena::hit_log`), prefixed with the arena, for the server to take.
    pub hits: Option<Vec<String>>,
    /// Game tick of the arena's tick 0.
    zero: f64,
    pub playlist: Playlist,
    /// The host wants the empty places of the lobby filled with bots.
    pub fill: bool,
    /// Game time: timers and arenas run on it (dev commands slow, pause or warp it).
    pub clock: GameClock,
    pub max: usize,
    next_id: PlayerId,
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
fn lobby_arena(ids: &[PlayerId]) -> Arena {
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
            hits: None,
            zero,
            playlist: Playlist::default(),
            fill: false,
            clock,
            max: max_players.min(MAX_PLAYERS),
            next_id: PlayerId(1),
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

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// The player with this identity, if they are in the room (connected or not).
    pub fn player_of(&self, uid: &Uid) -> Option<&Player> {
        self.players.iter().find(|p| p.uid() == Some(uid))
    }

    pub fn set_rtt(&mut self, id: PlayerId, rtt: u32) {
        if let Some(p) = self.player_mut(id) {
            p.rtt = rtt;
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

    fn send_to(&mut self, id: PlayerId, msg: ServerMsg) {
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
