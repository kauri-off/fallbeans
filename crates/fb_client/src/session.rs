//! The client's side of the control protocol: the hello, where the player is (room list, a room, its lobby
//! and arena), and what the interface shows of it (results, chat, the feed). `--start` plays a game by
//! itself as the room's host (stress runs, `xtask dev`).
use std::collections::BTreeMap;

use bevy::prelude::*;
use fb_arena::ArenaKind;
use fb_net::*;
use fb_proto::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;

use crate::game::Cue;
use crate::net::Conn;
use crate::opts::Opts;
use crate::settings::Me;

#[derive(Resource, Default)]
pub struct Session {
    pub dev: bool,
    /// Player id in the room, and the room's id.
    pub me: Option<Pid>,
    pub room: Option<String>,
    pub lobby: Option<Lobby>,
    pub arena: Option<ArenaInfo>,
    /// Points of the current arena (the map's own scoring: stars, tails).
    pub scores: BTreeMap<Pid, f64>,
    /// The server sent the client away (another window) or it is out of date: no more reconnecting.
    pub refused: bool,
    /// The last game's standings (the podium's poses).
    pub standings: Vec<Standing>,
    /// The host already asked to start the game in this lobby.
    started: bool,
    /// The room list while in no room (None until the server sent it), and the room this player created.
    pub rooms: Option<Vec<RoomInfo>>,
    pub mine: Option<String>,
    /// Why entering or creating a room failed (`DenyReason::Pin`: that room asks for its PIN).
    pub denied: Option<Denied>,
    /// Why the server sent the client away (another window, out of date).
    pub reject: Option<String>,
    /// This room is a practice round of one map, and the room to go back to after it.
    pub practice: bool,
    pub back_to: Option<String>,
    /// Counts entries into a room (not resumes): the menu opens on each.
    pub entries: u32,
    pub results: Option<Results>,
    /// Shown beside the podium.
    pub game_end: Option<GameEnd>,
    pub chat: Vec<ChatLine>,
    pub feed: Vec<FeedEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Denied {
    pub room: Option<String>,
    pub reason: DenyReason,
    pub msg: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Results {
    pub game: String,
    pub index: u32,
    pub total: u32,
    pub rows: Vec<fb_shared::rules::RoundRow>,
    pub practice: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameEnd {
    pub standings: Vec<Standing>,
    pub awards: Vec<Award>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatLine {
    pub n: u32,
    pub id: Pid,
    pub name: String,
    pub text: String,
    /// Real time it came (seconds since start).
    pub at: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Feed {
    Ko {
        victim: Pid,
        by: Option<Pid>,
        cause: String,
        out: bool,
        shortcut: bool,
    },
    Note(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedEntry {
    pub n: u32,
    pub at: f32,
    pub what: Feed,
}

/// Lines of the feed and the chat kept.
const FEED_MAX: usize = 6;
const CHAT_MAX_LINES: usize = 50;
/// Seconds a line of the feed stays.
pub const FEED_SECS: f32 = 6.0;

impl Session {
    pub fn player(&self, id: Pid) -> Option<&LobbyPlayer> {
        self.lobby.as_ref()?.players.iter().find(|p| p.id == id)
    }

    pub fn name_of(&self, id: Pid) -> String {
        self.player(id).map_or_else(|| format!("#{id}"), |p| p.name.clone())
    }

    pub fn host(&self) -> bool {
        self.me.is_some() && self.lobby.as_ref().is_some_and(|l| l.host == self.me)
    }

    pub fn push_feed(&mut self, now: f32, what: Feed) {
        let n = self.feed.last().map_or(0, |f| f.n + 1);
        self.feed.retain(|f| now - f.at < FEED_SECS);
        if self.feed.len() >= FEED_MAX {
            self.feed.remove(0);
        }
        self.feed.push(FeedEntry { n, at: now, what });
    }

    pub fn note(&mut self, now: f32, text: String) {
        self.push_feed(now, Feed::Note(text));
    }

    fn leave_room(&mut self) {
        self.me = None;
        self.room = None;
        self.lobby = None;
        self.arena = None;
        self.practice = false;
        self.results = None;
        self.game_end = None;
        self.chat.clear();
        self.feed.clear();
    }
}

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>();
        app.add_observer(say_hello);
        app.add_systems(Update, (receive, start_game).chain());
    }
}

fn say_hello(
    _: On<Add, Connected>,
    me: Me,
    session: Res<Session>,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
) {
    let opts = &me.opts;
    let hello = Hello {
        name: me.name(),
        // Back into the room after a reconnect.
        room: session
            .room
            .clone()
            .filter(|r| !r.is_empty())
            .or_else(|| opts.room.clone()),
        pin: opts.pin.clone(),
        practice: opts.practice.clone(),
        color: me.color(),
        outfit: Some(me.player.outfit()),
    };
    for mut s in &mut senders {
        s.send::<ControlChannel>(ClientMsg::Hello(hello.clone()));
    }
}

pub fn send(senders: &mut Query<&mut MessageSender<ClientMsg>, With<Client>>, msg: ClientMsg) {
    for mut s in senders {
        s.send::<ControlChannel>(msg.clone());
    }
}

fn receive(
    mut receivers: Query<&mut MessageReceiver<ServerMsg>, With<Client>>,
    mut session: ResMut<Session>,
    conn: Option<Res<Conn>>,
    mut commands: Commands,
    mut cues: MessageWriter<Cue>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs();
    for mut r in &mut receivers {
        for msg in r.receive() {
            match msg {
                ServerMsg::Ready { dev } => {
                    session.dev = dev;
                }
                ServerMsg::Reject { reason, msg } => {
                    warn!("refused ({reason:?}): {msg}");
                    session.refused = true;
                    session.reject = Some(msg);
                    if let Some(entity) = conn.as_ref().and_then(|c| c.entity) {
                        crate::net::hang_up(&mut commands, entity);
                    }
                }
                ServerMsg::Rooms { rooms, mine } => {
                    session.me = None;
                    session.room = None;
                    let list: Vec<String> = rooms
                        .iter()
                        .map(|r| format!("{} «{}» {}+{}/{}", r.id, r.title, r.players, r.bots, r.max))
                        .collect();
                    debug!("rooms: [{}] mine {mine:?}", list.join(", "));
                    session.rooms = Some(rooms);
                    session.mine = mine;
                }
                ServerMsg::Denied { room, reason, msg } => {
                    warn!("room {room:?} denied ({reason:?}): {msg}");
                    // The room from a link or the one we were in is not to be had: the room list it is.
                    if session.arena.is_some() {
                        session.leave_room();
                    }
                    session.denied = Some(Denied { room, reason, msg });
                }
                ServerMsg::Home { msg } => {
                    session.leave_room();
                    session.denied = if msg.is_empty() {
                        None
                    } else {
                        Some(Denied {
                            room: None,
                            reason: DenyReason::Gone,
                            msg,
                        })
                    };
                }
                ServerMsg::Welcome {
                    id,
                    room,
                    resumed,
                    practice,
                    ..
                } => {
                    info!(
                        "in room {room:?} as player {id}{}",
                        if resumed { " (resumed)" } else { "" }
                    );
                    session.me = Some(id);
                    session.room = Some(room);
                    session.practice = practice;
                    session.denied = None;
                    if !resumed {
                        session.chat.clear();
                        session.entries += 1;
                    }
                }
                ServerMsg::Lobby(l) => {
                    if l.phase == Phase::Lobby && session.lobby.as_ref().is_none_or(|o| o.phase != Phase::Lobby) {
                        session.started = false;
                    }
                    if l.phase != Phase::Results {
                        session.results = None;
                    }
                    if l.phase != Phase::Podium {
                        session.game_end = None;
                    }
                    session.lobby = Some(l);
                }
                ServerMsg::Arena(a) => {
                    info!(
                        "arena {}: {} ({:?}{}), {} players",
                        a.id,
                        a.game,
                        a.kind,
                        if a.total > 0 {
                            format!(" {}/{}", a.index, a.total)
                        } else {
                            String::new()
                        },
                        a.participants.len()
                    );
                    session.scores = a.scores.iter().copied().collect();
                    if a.kind != ArenaKind::Lobby {
                        session.results = None;
                    }
                    session.feed.clear();
                    session.arena = Some(a);
                }
                ServerMsg::RoundEnd {
                    game,
                    index,
                    total,
                    rows,
                    practice,
                } => {
                    let line: Vec<String> = rows.iter().map(|r| format!("#{} {:+}", r.id, r.delta)).collect();
                    info!("round {index} ({game}) over: {}", line.join(" "));
                    session.results = Some(Results {
                        game,
                        index,
                        total,
                        rows,
                        practice,
                    });
                    cues.write(Cue::Results);
                }
                ServerMsg::GameEnd { standings, awards } => {
                    let s: Vec<String> = standings.iter().map(|s| format!("{} {}", s.name, s.total)).collect();
                    info!("game over: {}", s.join(", "));
                    session.standings = standings.clone();
                    session.game_end = Some(GameEnd { standings, awards });
                    session.results = None;
                }
                ServerMsg::Chat { id, name, text } => {
                    info!("chat {name}: {text}");
                    let n = session.chat.last().map_or(0, |l| l.n + 1);
                    if session.chat.len() >= CHAT_MAX_LINES {
                        session.chat.remove(0);
                    }
                    session.chat.push(ChatLine {
                        n,
                        id,
                        name,
                        text,
                        at: now,
                    });
                    if Some(id) != session.me {
                        cues.write(Cue::Sfx("click"));
                    }
                }
                ServerMsg::DevAck { ok, msg, .. } => {
                    info!("dev: {} {msg}", if ok { "ok" } else { "failed" });
                    let mark = if ok { "" } else { "✖ " };
                    session.note(now, format!("🛠 {mark}{msg}"));
                }
                ServerMsg::Clock { rate } => info!("game time ×{rate}"),
                ServerMsg::Scores(s) => {
                    // In the lobby a score is the bell on the tower, rung once more (bots ring it silently).
                    let lobby = session.arena.as_ref().is_some_and(|a| a.kind == ArenaKind::Lobby);
                    for (id, v) in s {
                        let was = session.scores.insert(id, v).unwrap_or(0.0);
                        let bot = session
                            .lobby
                            .as_ref()
                            .and_then(|l| l.players.iter().find(|p| p.id == id))
                            .is_none_or(|p| p.bot);
                        if lobby && v > was && !bot {
                            cues.write(Cue::Bell(id));
                            let mine = Some(id) == session.me;
                            let line = crate::ui::text::bell(mine, &session.name_of(id), v as u32);
                            session.note(now, line);
                        }
                    }
                }
                ServerMsg::Emote { id, e } => {
                    cues.write(Cue::Emote { id, e });
                }
                ServerMsg::Left(_) => {}
            }
        }
    }
}

/// `--start <maps>`: as the host, plays a game of those maps in turn once enough players are in.
fn start_game(
    opts: Res<Opts>,
    mut session: ResMut<Session>,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
) {
    if opts.start.is_empty() {
        return;
    }
    let Some(lobby) = &session.lobby else { return };
    if session.started || lobby.phase != Phase::Lobby || lobby.host != session.me {
        return;
    }
    let want = opts.start_players.unwrap_or(lobby.min as usize).max(lobby.min as usize);
    let n = lobby.players.len();
    if n < want {
        return;
    }
    session.started = true;
    info!("starting {} with {n} players", opts.start.join(","));
    if opts.fill {
        send(&mut senders, ClientMsg::Fill(true));
    }
    send(
        &mut senders,
        ClientMsg::Playlist(Playlist {
            mode: Mode::Custom,
            games: opts
                .start
                .iter()
                .cycle()
                .take(opts.start_rounds as usize)
                .cloned()
                .collect(),
            rounds: opts.start_rounds,
        }),
    );
    send(&mut senders, ClientMsg::Start);
}
