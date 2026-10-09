//! Messages between client and server besides replication and input:
//! the room list, rooms and their lobby, game flow, chat, dev commands, map events. Everything a client
//! sends passes `ClientMsg::check` (bounds and shapes) before the server looks at it.
pub use fb_shared::cause::{Cause, Hazard};
use fb_shared::game::ArenaKind;
pub use fb_shared::game::MapId;
use fb_shared::rules::RoundRow;
use fb_shared::{CHAT_MAX, COLORS, EMOTES, ROOM_PIN_DIGITS};
pub use fb_sim::map::MapEvent;
use serde::{Deserialize, Serialize};

pub use fb_maps::director::{Mode, Playlist};
pub use fb_shared::PlayerId;
pub use fb_shared::outfit::Outfit;

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
    pub transport: TransportKind,
}

/// `GET /fallbeans/health`: a server up, its protocol and how busy it is.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Health {
    pub ok: bool,
    /// What the server's owner called it.
    #[serde(default)]
    pub name: Option<String>,
    /// `PROTOCOL_VERSION`.
    pub version: u32,
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub rooms: u64,
    #[serde(default)]
    pub players: u64,
    #[serde(default)]
    pub practice: u64,
}

/// The transport a connection goes over.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    #[default]
    Udp,
    Ws,
}

/// The player's identity to keep and a connect token (base64) for one attempt; none on another protocol.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SessionReply {
    pub protocol: u32,
    pub build: String,
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
    pub practice: Option<MapId>,
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
        games: Vec<MapId>,
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
        id: Option<PlayerId>,
        p: [f64; 3],
        yaw: Option<f64>,
    },
    /// Teleport to the spawn, a checkpoint or just before the finish.
    Goto {
        id: Option<PlayerId>,
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
        id: Option<PlayerId>,
    },
    /// Knock a bean over with this velocity.
    Knock {
        id: Option<PlayerId>,
        v: [f64; 3],
    },
    /// Make `actor` hold `target` for `s` seconds (as if the grab button were held).
    Grab {
        actor: Option<PlayerId>,
        target: PlayerId,
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
    RemoveBot(PlayerId),
    /// The host makes another player the host.
    Host(PlayerId),
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

// (No upper bound: a room's ids start at 1 and only grow, past 65535 after heavy bot churn.)
fn check_pid(id: PlayerId) -> Result<(), &'static str> {
    ensure(id.0 != 0, "player id")
}

fn check_games(games: &[MapId]) -> Result<(), &'static str> {
    ensure(games.len() <= 12, "games")
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

impl DevCmd {
    pub fn check(&self) -> Result<(), &'static str> {
        let opt_pid = |id: &Option<PlayerId>| id.map_or(Ok(()), check_pid);
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
            DevCmd::SkipIntro => "skip_intro",
            DevCmd::Warp { .. } => "warp",
            DevCmd::EndRound => "end_round",
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
    /// None for a practice room (not in the list).
    pub id: Option<String>,
    pub title: String,
    pub private: bool,
}

/// A room in the room list.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RoomInfo {
    pub id: String,
    pub title: String,
    pub private: bool,
    /// Name of the current host (None while there is none).
    pub host: Option<String>,
    /// People in the room (bots are counted apart).
    pub players: u32,
    pub bots: u32,
    pub max: u32,
    pub phase: Phase,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LobbyPlayer {
    pub id: PlayerId,
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
    pub host: Option<PlayerId>,
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
    pub game: MapId,
    pub participants: Vec<PlayerId>,
    /// Round number in the game (1-based) and the number of rounds; 0 outside rounds.
    pub index: u32,
    pub total: u32,
    pub practice: bool,
    /// Sent to someone joining an arena already running.
    pub late: bool,
    pub finished: Vec<PlayerId>,
    pub out: Vec<PlayerId>,
    pub scores: Vec<(PlayerId, i64)>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    pub id: PlayerId,
    pub name: String,
    pub color: u8,
    pub place: u32,
    pub total: i64,
    pub wins: u32,
    pub falls: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Award {
    pub kind: AwardKind,
    pub id: PlayerId,
    /// What it was won with: seconds survived, knock-offs, grabs, falls, shortcuts.
    pub value: u32,
}

/// A fun title at the end of a game.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AwardKind {
    /// Best places in races.
    Fastest,
    Survivor,
    Bully,
    Grabber,
    Clumsy,
    Sly,
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
    /// Without a message: the room asks for its PIN.
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
    /// The room list, sent while the player is in no room; `mine` is the room they created, if it still exists.
    Rooms {
        rooms: Vec<RoomInfo>,
        mine: Option<String>,
    },
    /// Creating or entering a room failed.
    Denied {
        room: Option<String>,
        reason: DenyReason,
        msg: Option<String>,
    },
    /// The player is out of the room, back at the room list (with why, when it was not their doing).
    Home {
        msg: Option<String>,
    },
    /// Entered a room (`id` is the player's id there); its lobby and arena follow.
    Welcome {
        id: PlayerId,
        room: String,
        solo: bool,
        practice: bool,
        resumed: bool,
    },
    Lobby(Lobby),
    Arena(ArenaInfo),
    RoundEnd {
        game: MapId,
        index: u32,
        total: u32,
        rows: Vec<RoundRow>,
        practice: bool,
    },
    GameEnd {
        standings: Vec<Standing>,
        awards: Vec<Award>,
    },
    Scores(Vec<(PlayerId, i64)>),
    Emote {
        id: PlayerId,
        e: u8,
    },
    Chat {
        id: PlayerId,
        name: String,
        text: String,
    },
    /// The server says something in the room's chat.
    Notice(String),
    Left(PlayerId),
    /// Reply to a dev command (`q` echoes the request's): what it did, or why it could not.
    DevAck {
        q: Option<u32>,
        result: Result<String, String>,
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
        id: PlayerId,
        at: f64,
    },
    Finish {
        id: PlayerId,
        place: u32,
        time: f64,
    },
    /// A bean fell (respawned) or was eliminated, with who or what caused it.
    Ko {
        id: PlayerId,
        out: bool,
        by: Option<PlayerId>,
        cause: Cause,
        shortcut: bool,
    },
    /// A map's own event.
    Map(MapEvent),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MapEventMsg {
    /// The arena (`Round::arena`) it belongs to.
    pub arena: u32,
    /// Lightyear tick it happened on.
    pub tick: u32,
    pub ev: MapEventKind,
    /// Sent again from the arena's history to someone who just came in: what it changes applies, its sounds
    /// and notes do not (it happened before they were there).
    pub history: bool,
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

    #[test]
    fn checks_the_other_messages() {
        let long = "ы".repeat(65);
        assert!(ClientMsg::Name("Боб".into()).check().is_ok());
        assert!(ClientMsg::Name(long.clone()).check().is_err());
        assert!(ClientMsg::Name("a".repeat(257)).check().is_err());
        let create = |title: &str| {
            ClientMsg::Create {
                title: title.into(),
                private: true,
            }
            .check()
        };
        assert!(create("Комната").is_ok());
        assert!(create(&long).is_err());
        let join = |room: &str, pin: Option<&str>| {
            ClientMsg::Join {
                room: room.into(),
                pin: pin.map(Into::into),
            }
            .check()
        };
        assert!(join("k7qxm", None).is_ok());
        assert!(join("k7qxm", Some("0042")).is_ok());
        assert!(join("k", None).is_err());
        assert!(join("k7qxm9abc", None).is_err());
        assert!(join("k7-xm", None).is_err());
        assert!(join("k7qxm", Some("004")).is_err());
        assert!(ClientMsg::Color(12).check().is_ok());
        assert!(ClientMsg::Color(13).check().is_err());
        assert!(ClientMsg::Color(255).check().is_err());
        let playlist = |games: Vec<MapId>, rounds| {
            ClientMsg::Playlist(Playlist {
                games,
                rounds,
                ..Default::default()
            })
            .check()
        };
        assert!(playlist(vec![MapId::DoorDash], 5).is_ok());
        assert!(playlist(Vec::new(), 0).is_err());
        assert!(playlist(Vec::new(), 13).is_err());
        assert!(playlist(vec![MapId::DoorDash; 13], 5).is_err());
        assert!(ClientMsg::RemoveBot(PlayerId(0)).check().is_err());
        assert!(ClientMsg::RemoveBot(PlayerId(3)).check().is_ok());
        assert!(ClientMsg::Host(PlayerId(0)).check().is_err());
        assert!(ClientMsg::Emote(6).check().is_err());
        assert!(
            ClientMsg::Hello(Hello {
                name: long,
                ..Default::default()
            })
            .check()
            .is_err()
        );
    }

    #[test]
    fn checks_dev_commands() {
        let ok = |cmd: DevCmd| cmd.check().is_ok();
        assert!(ok(DevCmd::Warp { s: 180.0 }));
        assert!(!ok(DevCmd::Warp { s: -1.0 }));
        assert!(!ok(DevCmd::Warp { s: f64::NAN }));
        assert!(ok(DevCmd::Step { ticks: 1200 }));
        assert!(!ok(DevCmd::Step { ticks: 0 }));
        assert!(!ok(DevCmd::Step { ticks: 1201 }));
        assert!(ok(DevCmd::Seed { seed: 1 << 31 }));
        assert!(!ok(DevCmd::Seed { seed: (1 << 31) + 1 }));
        let start = |games: Vec<MapId>, rounds, bots| DevCmd::Start { games, rounds, bots };
        assert!(ok(start(Vec::new(), Some(12), Some(7))));
        assert!(!ok(start(Vec::new(), Some(0), None)));
        assert!(!ok(start(Vec::new(), None, Some(8))));
        assert!(!ok(start(vec![MapId::DoorDash; 13], None, None)));
        let goto = |to| DevCmd::Goto { id: None, to };
        assert!(ok(goto(Goto::Checkpoint(64))));
        assert!(!ok(goto(Goto::Checkpoint(65))));
        assert!(!ok(DevCmd::Goto {
            id: Some(PlayerId(0)),
            to: Goto::Spawn
        }));
        let teleport = |p, yaw| DevCmd::Teleport { id: None, p, yaw };
        assert!(ok(teleport([1.0, 2.0, 3.0], Some(0.5))));
        assert!(!ok(teleport([1.0, f64::INFINITY, 3.0], None)));
        assert!(!ok(teleport([1.0, 2.0, 3.0], Some(f64::NAN))));
        let grab = |target, s| DevCmd::Grab { actor: None, target, s };
        assert!(ok(grab(PlayerId(2), Some(10.0))));
        assert!(!ok(grab(PlayerId(0), None)));
        assert!(!ok(grab(PlayerId(2), Some(10.5))));
        assert!(ok(DevCmd::Bot { n: Some(7), near: true }));
        assert!(!ok(DevCmd::Bot {
            n: Some(0),
            near: false
        }));
        assert!(!ok(DevCmd::Kill { id: Some(PlayerId(0)) }));
        assert!(!ok(DevCmd::Rate { k: -0.5 }));
    }
}
