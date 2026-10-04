//! Messages between client and server besides replication and input (port of `shared/protocol.ts`):
//! the room list, rooms and their lobby, game flow, chat, dev commands, map events. Everything a client
//! sends passes `ClientMsg::check` (the bounds zod checked in TS) before the server looks at it.
use fb_shared::game::ArenaKind;
use fb_shared::rules::RoundRow;
use fb_shared::{CHAT_MAX, COLORS, EMOTES, ROOM_PIN_DIGITS};
use serde::{Deserialize, Serialize};

pub use fb_maps::director::{Mode, Playlist};
pub use fb_shared::outfit::Outfit;

/// A player's id in a room (small, sequential; not the network id).
pub type Pid = u32;

/// Room ids: short codes, as in a link to the room.
pub fn valid_room_id(id: &str) -> bool {
    (2..=8).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

pub fn valid_pin(pin: &str) -> bool {
    pin.len() == ROOM_PIN_DIGITS && pin.bytes().all(|b| b.is_ascii_digit())
}

/// `POST /fallbeans/api/session`: what a client asks for before each connection attempt.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SessionRequest {
    /// The identity token of an earlier session; without a valid one the server makes a new player.
    pub identity: Option<String>,
    pub protocol: u32,
    /// The transport the connection will go over: "udp" or "ws".
    pub transport: String,
}

/// The player's identity to keep and a connect token (base64) for one attempt; none while updating or on another protocol.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SessionReply {
    pub protocol: u32,
    pub build: String,
    pub updating: bool,
    pub identity: String,
    pub token: Option<String>,
    /// Where the WebSocket fallback is (`ws://host:port`, or `wss://domain/fallbeans/ws` behind a proxy).
    #[serde(default)]
    pub ws_url: Option<String>,
}

/// The first message of a connection.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Hello {
    pub name: String,
    /// Go straight into this room (a link to it, a restart, a reconnect).
    pub room: Option<String>,
    pub pin: Option<String>,
    /// A practice round of this game, alone with bots.
    pub practice: Option<String>,
    /// The suit colour picked last time (taken if nobody in the room has it) and the outfit.
    pub color: Option<u8>,
    pub outfit: Option<Outfit>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goto {
    Spawn,
    Finish,
    Checkpoint(u32),
}

/// Development commands (only a server started with `--dev` takes them). `id` defaults to the sender.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum DevCmd {
    /// Jump to the start of the round (fast-forwards through the intro).
    SkipIntro,
    /// Simulate this much game time right away (seconds, every tick simulated).
    Warp {
        s: f64,
    },
    EndRound,
    /// Start a game now: optional list of games, rounds, and exactly how many bots (replacing any).
    Start {
        games: Vec<String>,
        rounds: Option<u32>,
        bots: Option<u32>,
    },
    Lobby,
    /// Speed of game time: 1 normal, 0.25 slow motion, 0 paused.
    Rate {
        k: f64,
    },
    /// While paused: advance this many ticks.
    Step {
        ticks: u32,
    },
    Teleport {
        id: Option<Pid>,
        p: [f64; 3],
        yaw: Option<f64>,
    },
    /// Teleport to the spawn, a checkpoint or just before the finish.
    Goto {
        id: Option<Pid>,
        to: Goto,
    },
    /// Add bots (in any phase; in a round they join it), optionally right next to the sender.
    Bot {
        n: Option<u32>,
        near: bool,
    },
    /// Freeze (false) or resume (true) bot brains.
    Bots {
        on: bool,
    },
    Kill {
        id: Option<Pid>,
    },
    /// Knock a bean over with this velocity.
    Knock {
        id: Option<Pid>,
        v: [f64; 3],
    },
    /// Make `actor` hold `target` for `s` seconds (as if the grab button were held).
    Grab {
        actor: Option<Pid>,
        target: Pid,
        s: Option<f64>,
    },
    /// Seed of the next round's map.
    Seed {
        seed: u32,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ClientMsg {
    Hello(Hello),
    Name(String),
    /// From the room list: open a room of one's own (each player has at most one) and enter it.
    Create {
        title: String,
        private: bool,
    },
    /// From the room list: enter a room; a private one wants its PIN from those it has not let in before.
    Join {
        room: String,
        pin: Option<String>,
    },
    /// Back to the room list.
    Leave,
    Color(u8),
    Outfit(Outfit),
    Start,
    Abort,
    Playlist(Playlist),
    AddBot,
    RemoveBot(Pid),
    /// The host makes another player the host.
    Host(Pid),
    /// The host makes the room private (the server picks a PIN) or public.
    Access {
        private: bool,
    },
    /// The host has the empty places of the lobby filled with bots.
    Fill(bool),
    Emote(u8),
    Chat(String),
    Dev {
        q: Option<u32>,
        cmd: DevCmd,
    },
}

fn chars_max(s: &str, n: usize) -> bool {
    s.len() <= n * 4 && s.chars().count() <= n
}

fn ensure(ok: bool, what: &'static str) -> Result<(), &'static str> {
    if ok { Ok(()) } else { Err(what) }
}

fn check_pid(id: Pid) -> Result<(), &'static str> {
    ensure(id <= 65535, "player id")
}

fn check_games(games: &[String]) -> Result<(), &'static str> {
    ensure(games.len() <= 12 && games.iter().all(|g| chars_max(g, 32)), "games")
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

impl DevCmd {
    pub fn check(&self) -> Result<(), &'static str> {
        let opt_pid = |id: &Option<Pid>| id.map_or(Ok(()), check_pid);
        match self {
            DevCmd::SkipIntro | DevCmd::EndRound | DevCmd::Lobby | DevCmd::Bots { .. } => Ok(()),
            DevCmd::Warp { s } => ensure((0.0..=180.0).contains(s), "warp"),
            DevCmd::Start { games, rounds, bots } => {
                check_games(games)?;
                ensure(rounds.is_none_or(|r| (1..=12).contains(&r)), "rounds")?;
                ensure(bots.is_none_or(|b| b <= 7), "bots")
            }
            DevCmd::Rate { k } => ensure((0.0..=8.0).contains(k), "rate"),
            DevCmd::Step { ticks } => ensure((1..=1200).contains(ticks), "step"),
            DevCmd::Teleport { id, p, yaw } => {
                opt_pid(id)?;
                ensure(finite(p) && yaw.is_none_or(f64::is_finite), "teleport")
            }
            DevCmd::Goto { id, to } => {
                opt_pid(id)?;
                ensure(!matches!(to, Goto::Checkpoint(i) if *i > 64), "checkpoint")
            }
            DevCmd::Bot { n, .. } => ensure(n.is_none_or(|n| (1..=7).contains(&n)), "bots"),
            DevCmd::Kill { id } => opt_pid(id),
            DevCmd::Knock { id, v } => {
                opt_pid(id)?;
                ensure(finite(v), "knock")
            }
            DevCmd::Grab { actor, target, s } => {
                opt_pid(actor)?;
                check_pid(*target)?;
                ensure(s.is_none_or(|s| (0.0..=10.0).contains(&s)), "grab")
            }
            DevCmd::Seed { seed } => ensure(*seed <= 1 << 31, "seed"),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            DevCmd::SkipIntro => "skipIntro",
            DevCmd::Warp { .. } => "warp",
            DevCmd::EndRound => "endRound",
            DevCmd::Start { .. } => "start",
            DevCmd::Lobby => "lobby",
            DevCmd::Rate { .. } => "rate",
            DevCmd::Step { .. } => "step",
            DevCmd::Teleport { .. } => "teleport",
            DevCmd::Goto { .. } => "goto",
            DevCmd::Bot { .. } => "bot",
            DevCmd::Bots { .. } => "bots",
            DevCmd::Kill { .. } => "kill",
            DevCmd::Knock { .. } => "knock",
            DevCmd::Grab { .. } => "grab",
            DevCmd::Seed { .. } => "seed",
        }
    }
}

impl ClientMsg {
    /// The bounds a message must keep to (a client that breaks them is buggy or hostile: the message is dropped).
    pub fn check(&self) -> Result<(), &'static str> {
        let color = |c: u8| ensure((c as usize) < COLORS.len(), "color");
        match self {
            ClientMsg::Hello(h) => {
                ensure(chars_max(&h.name, 64), "name")?;
                ensure(h.room.as_deref().is_none_or(valid_room_id), "room")?;
                ensure(h.pin.as_deref().is_none_or(valid_pin), "pin")?;
                ensure(h.practice.as_deref().is_none_or(|p| chars_max(p, 32)), "practice")?;
                h.color.map_or(Ok(()), color)
            }
            ClientMsg::Name(name) => ensure(chars_max(name, 64), "name"),
            ClientMsg::Create { title, .. } => ensure(chars_max(title, 64), "title"),
            ClientMsg::Join { room, pin } => {
                ensure(valid_room_id(room), "room")?;
                ensure(pin.as_deref().is_none_or(valid_pin), "pin")
            }
            ClientMsg::Color(c) => color(*c),
            ClientMsg::Playlist(pl) => {
                check_games(&pl.games)?;
                ensure((1..=12).contains(&pl.rounds), "rounds")
            }
            ClientMsg::RemoveBot(id) | ClientMsg::Host(id) => check_pid(*id),
            ClientMsg::Emote(e) => ensure((1..=EMOTES).contains(&u32::from(*e)), "emote"),
            ClientMsg::Chat(text) => ensure(chars_max(text, CHAT_MAX * 2), "chat"),
            ClientMsg::Dev { cmd, .. } => cmd.check(),
            ClientMsg::Leave
            | ClientMsg::Outfit(_)
            | ClientMsg::Start
            | ClientMsg::Abort
            | ClientMsg::AddBot
            | ClientMsg::Access { .. }
            | ClientMsg::Fill(_) => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Lobby,
    Round,
    Results,
    Podium,
}

/// A room as its members know it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RoomRef {
    /// Empty for a practice room (not in the list).
    pub id: String,
    pub title: String,
    pub private: bool,
}

/// A room in the room list.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RoomInfo {
    pub id: String,
    pub title: String,
    pub private: bool,
    /// Name of the current host.
    pub host: String,
    /// People in the room (bots are counted apart).
    pub players: u32,
    pub bots: u32,
    pub max: u32,
    pub phase: Phase,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LobbyPlayer {
    pub id: Pid,
    pub name: String,
    pub color: u8,
    pub outfit: Outfit,
    /// Points in the current game.
    pub score: i64,
    /// Games won.
    pub crowns: u32,
    pub spectator: bool,
    pub bot: bool,
    pub connected: bool,
    /// Round trip in milliseconds.
    pub ping: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Lobby {
    pub room: RoomRef,
    pub phase: Phase,
    pub host: Option<Pid>,
    pub min: u32,
    pub max: u32,
    pub players: Vec<LobbyPlayer>,
    pub playlist: Playlist,
    /// Empty places are filled with bots.
    pub fill: bool,
    /// PIN of a private room: only the host is told.
    pub pin: Option<String>,
    /// Server tick when the next scene starts on its own (results → round, podium → lobby).
    pub next: Option<u32>,
}

/// What a client needs about the arena besides its map (the room's replicated `Round`): who plays,
/// where in the game it is, and how it stands for someone arriving in the middle.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ArenaInfo {
    /// Changes with every new arena (the replicated `Round` carries the same id).
    pub id: u32,
    pub kind: ArenaKind,
    pub game: String,
    pub participants: Vec<Pid>,
    /// Round number in the game (1-based) and the number of rounds; 0 outside rounds.
    pub index: u32,
    pub total: u32,
    pub practice: bool,
    /// Sent to someone joining an arena already running.
    pub late: bool,
    pub finished: Vec<Pid>,
    pub out: Vec<Pid>,
    pub scores: Vec<(Pid, f64)>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    pub id: Pid,
    pub name: String,
    pub color: u8,
    pub place: u32,
    pub total: i64,
    pub wins: u32,
    pub falls: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Award {
    pub key: String,
    pub title: String,
    pub icon: String,
    pub id: Pid,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    Auth,
    Bad,
    Busy,
    /// The player opened the game somewhere else.
    Moved,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    /// With an empty message: the room asks for its PIN.
    Pin,
    Full,
    Gone,
    Limit,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ServerMsg {
    /// The hello was accepted (the player's identity came with the connect token, see `Session`).
    Ready {
        dev: bool,
    },
    /// The connection is refused and closed.
    Reject {
        reason: RejectReason,
        msg: String,
    },
    /// The game is being updated: the connection closes.
    Updating,
    /// The room list, sent while the player is in no room; `mine` is the room they created, if it still exists.
    Rooms {
        rooms: Vec<RoomInfo>,
        mine: Option<String>,
    },
    /// Creating or entering a room failed.
    Denied {
        room: Option<String>,
        reason: DenyReason,
        msg: String,
    },
    /// The player is out of the room, back at the room list.
    Home {
        msg: String,
    },
    /// Entered a room (`id` is the player's id there); its lobby and arena follow.
    Welcome {
        id: Pid,
        room: String,
        solo: bool,
        practice: bool,
        resumed: bool,
    },
    Lobby(Lobby),
    Arena(ArenaInfo),
    RoundEnd {
        game: String,
        index: u32,
        total: u32,
        rows: Vec<RoundRow>,
        practice: bool,
    },
    GameEnd {
        standings: Vec<Standing>,
        awards: Vec<Award>,
    },
    Scores(Vec<(Pid, f64)>),
    Emote {
        id: Pid,
        e: u8,
    },
    Chat {
        id: Pid,
        name: String,
        text: String,
    },
    Left(Pid),
    /// Reply to a dev command (`q` echoes the request's).
    DevAck {
        q: Option<u32>,
        ok: bool,
        msg: String,
    },
    /// Game time changed speed (dev; prediction is off while it is not 1).
    Clock {
        rate: f64,
    },
}

/// Map events with the arena tick they happened at (clients apply them on that tick).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MapEventKind {
    Bonus {
        i: u32,
        id: Pid,
        at: f64,
    },
    Finish {
        id: Pid,
        place: u32,
        time: f64,
    },
    /// A bean fell (respawned) or was eliminated, with who or what caused it.
    Ko {
        id: Pid,
        out: bool,
        by: Option<Pid>,
        cause: String,
        shortcut: bool,
    },
    /// A map's own event; `data` is JSON (maps describe their events freely).
    Map {
        name: String,
        data: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MapEventMsg {
    /// The arena (`Round::arena`) it belongs to.
    pub arena: u32,
    /// Lightyear tick it happened on.
    pub tick: u32,
    pub ev: MapEventKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_bounds() {
        let hello = |h: Hello| ClientMsg::Hello(h).check();
        assert!(hello(Hello::default()).is_ok());
        assert!(
            hello(Hello {
                room: Some("k7qxm".into()),
                pin: Some("0042".into()),
                color: Some(12),
                ..Default::default()
            })
            .is_ok()
        );
        assert!(
            hello(Hello {
                room: Some("K7".into()),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            hello(Hello {
                pin: Some("12a4".into()),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            hello(Hello {
                color: Some(13),
                ..Default::default()
            })
            .is_err()
        );
        assert!(ClientMsg::Emote(0).check().is_err());
        assert!(ClientMsg::Emote(5).check().is_ok());
        assert!(ClientMsg::Chat("ы".repeat(CHAT_MAX * 2)).check().is_ok());
        assert!(ClientMsg::Chat("ы".repeat(CHAT_MAX * 2 + 1)).check().is_err());
        let knock = |v| ClientMsg::Dev {
            q: None,
            cmd: DevCmd::Knock { id: None, v },
        };
        assert!(knock([1.0, 2.0, 3.0]).check().is_ok());
        assert!(knock([f64::NAN, 2.0, 3.0]).check().is_err());
        assert!(
            ClientMsg::Dev {
                q: None,
                cmd: DevCmd::Rate { k: 9.0 },
            }
            .check()
            .is_err()
        );
    }
}
