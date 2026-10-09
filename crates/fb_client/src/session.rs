//! The client's side of the control protocol: the hello, where the player is (`Session`: a room, its lobby
//! and arena), the room list (`RoomList`), and what the interface shows of the room (`Outcome`, `ChatLog`,
//! `FeedLog`). `--start` plays a game by itself as the room's host (stress runs, `xtask dev`).
use std::collections::{BTreeMap, VecDeque};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fb_arena::ArenaKind;
use fb_net::*;
use fb_proto::MapId;
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
    pub me: Option<PlayerId>,
    pub room: Option<String>,
    pub lobby: Option<Lobby>,
    pub arena: Option<ArenaInfo>,
    /// Points of the current arena (the map's own scoring: stars, tails).
    pub scores: BTreeMap<PlayerId, i64>,
    /// The server sent the client away (another window) or it is out of date: no more reconnecting.
    pub refused: bool,
    /// The host already asked to start the game in this lobby.
    started: bool,
    /// Why entering or creating a room failed (`DenyReason::Pin`: that room asks for its PIN).
    pub denied: Option<Denied>,
    /// Why the server sent the client away (another window, out of date).
    pub reject: Option<String>,
    /// This room is a practice round of one map, and the room to go back to after it.
    pub practice: bool,
    pub back_to: Option<String>,
    /// Counts entries into a room (not resumes): the menu opens on each.
    pub entries: u32,
}

/// The room list while in no room (None until the server sent it), and the room this player created.
#[derive(Resource, Default, PartialEq)]
pub struct RoomList {
    pub rooms: Option<Vec<RoomInfo>>,
    pub mine: Option<String>,
}

/// How the last round and game went.
#[derive(Resource, Default, PartialEq)]
pub struct Outcome {
    pub results: Option<Results>,
    /// Shown beside the podium.
    pub game_end: Option<GameEnd>,
    /// The last game's standings (the podium's poses).
    pub standings: Vec<Standing>,
}

/// The room's chat, oldest first.
#[derive(Resource, Default, PartialEq)]
pub struct ChatLog(pub VecDeque<ChatLine>);

/// The feed: who fell, finishes, notes; oldest first.
#[derive(Resource, Default, PartialEq)]
pub struct FeedLog(pub VecDeque<FeedEntry>);

#[derive(Clone, Debug, PartialEq)]
pub struct Denied {
    pub room: Option<String>,
    pub reason: DenyReason,
    pub msg: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Results {
    pub game: MapId,
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
    /// None: the server.
    pub id: Option<PlayerId>,
    pub name: String,
    pub text: String,
    /// Real time it came (seconds since start).
    pub at: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Feed {
    Ko {
        victim: PlayerId,
        by: Option<PlayerId>,
        cause: Cause,
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
    pub fn player(&self, id: PlayerId) -> Option<&LobbyPlayer> {
        self.lobby.as_ref()?.players.iter().find(|p| p.id == id)
    }

    pub fn name_of(&self, id: PlayerId) -> String {
        self.player(id).map_or_else(|| format!("#{id}"), |p| p.name.clone())
    }

    pub fn host(&self) -> bool {
        self.me.is_some() && self.lobby.as_ref().is_some_and(|l| l.host == self.me)
    }

    fn leave_room(&mut self) {
        self.me = None;
        self.room = None;
        self.lobby = None;
        self.arena = None;
        self.practice = false;
    }
}

impl ChatLog {
    pub fn push(&mut self, now: f32, id: Option<PlayerId>, name: String, text: String) {
        let n = self.0.back().map_or(0, |l| l.n + 1);
        if self.0.len() >= CHAT_MAX_LINES {
            self.0.pop_front();
        }
        self.0.push_back(ChatLine {
            n,
            id,
            name,
            text,
            at: now,
        });
    }
}

impl FeedLog {
    pub fn push(&mut self, now: f32, what: Feed) {
        let n = self.0.back().map_or(0, |f| f.n + 1);
        self.0.retain(|f| now - f.at < FEED_SECS);
        if self.0.len() >= FEED_MAX {
            self.0.pop_front();
        }
        self.0.push_back(FeedEntry { n, at: now, what });
    }

    pub fn note(&mut self, now: f32, text: String) {
        self.push(now, Feed::Note(text));
    }
}

/// What the interface keeps of a room, emptied on leaving it.
#[derive(SystemParam)]
pub struct RoomView<'w> {
    pub outcome: ResMut<'w, Outcome>,
    pub chat: ResMut<'w, ChatLog>,
    pub feed: ResMut<'w, FeedLog>,
}

/// What this client knows of the rooms: its session, the room list, the room's view, and the connection.
#[derive(SystemParam)]
pub struct RoomState<'w> {
    pub session: ResMut<'w, Session>,
    pub list: ResMut<'w, RoomList>,
    pub view: RoomView<'w>,
    pub conn: Option<Res<'w, Conn>>,
}

impl RoomView<'_> {
    pub fn clear(&mut self) {
        self.outcome.set_if_neq(Outcome::default());
        self.chat.set_if_neq(ChatLog::default());
        self.feed.set_if_neq(FeedLog::default());
    }
}

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>();
        app.init_resource::<RoomList>();
        app.init_resource::<Outcome>();
        app.init_resource::<ChatLog>();
        app.init_resource::<FeedLog>();
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
        practice: opts.practice,
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
    mut state: RoomState,
    mut opts: ResMut<Opts>,
    mut commands: Commands,
    mut cues: MessageWriter<Cue>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs();
    for mut r in &mut receivers {
        for msg in r.receive() {
            match msg {
                ServerMsg::Ready { dev } => {
                    state.session.dev = dev;
                }
                ServerMsg::Reject { reason, msg } => {
                    warn!("refused ({reason:?}): {msg}");
                    // A practice the server would not give (too many on it): the next connection asks for the
                    // room the player came from, not for that practice again.
                    if opts.practice.take().is_some() {
                        opts.room = state.session.back_to.take();
                    }
                    state.session.refused = true;
                    state.session.reject = Some(msg);
                    if let Some(entity) = state.conn.as_ref().and_then(|c| c.entity) {
                        crate::net::hang_up(&mut commands, entity);
                    }
                }
                ServerMsg::Rooms { rooms, mine } => {
                    // (The list comes again and again: what is unchanged is not marked so.)
                    if state.session.me.is_some() || state.session.room.is_some() {
                        state.session.me = None;
                        state.session.room = None;
                    }
                    if bevy::log::tracing::enabled!(bevy::log::Level::DEBUG) {
                        let names: Vec<String> = rooms
                            .iter()
                            .map(|r| format!("{} «{}» {}+{}/{}", r.id, r.title, r.players, r.bots, r.max))
                            .collect();
                        debug!("rooms: [{}] mine {mine:?}", names.join(", "));
                    }
                    state.list.set_if_neq(RoomList {
                        rooms: Some(rooms),
                        mine,
                    });
                }
                ServerMsg::Denied { room, reason, msg } => {
                    warn!("room {room:?} denied ({reason:?}): {}", msg.as_deref().unwrap_or("-"));
                    // The room from a link or the one we were in is not to be had: the room list it is.
                    if state.session.arena.is_some() {
                        state.session.leave_room();
                        state.view.clear();
                    }
                    state.session.denied = Some(Denied { room, reason, msg });
                }
                ServerMsg::Home { msg } => {
                    state.session.leave_room();
                    state.view.clear();
                    state.session.denied = msg.map(|msg| Denied {
                        room: None,
                        reason: DenyReason::Gone,
                        msg: Some(msg),
                    });
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
                    state.session.me = Some(id);
                    // (Back from practice within the server's lobby grace is a resume of the room, but an entry
                    // for the player: the menu opens as on any entry.)
                    let other_room = state.session.room.as_ref() != Some(&room);
                    state.session.room = Some(room);
                    state.session.practice = practice;
                    state.session.denied = None;
                    if !resumed {
                        state.view.chat.set_if_neq(ChatLog::default());
                    }
                    if !resumed || other_room {
                        state.session.entries += 1;
                    }
                }
                ServerMsg::Lobby(l) => {
                    if l.phase == Phase::Lobby && state.session.lobby.as_ref().is_none_or(|o| o.phase != Phase::Lobby) {
                        state.session.started = false;
                    }
                    if l.phase != Phase::Results && state.view.outcome.results.is_some() {
                        state.view.outcome.results = None;
                    }
                    if l.phase != Phase::Podium && state.view.outcome.game_end.is_some() {
                        state.view.outcome.game_end = None;
                    }
                    if state.session.lobby.as_ref() != Some(&l) {
                        state.session.lobby = Some(l);
                    }
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
                    state.session.scores = a.scores.iter().copied().collect();
                    if a.kind != ArenaKind::Lobby && state.view.outcome.results.is_some() {
                        state.view.outcome.results = None;
                    }
                    state.view.feed.set_if_neq(FeedLog::default());
                    state.session.arena = Some(a);
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
                    state.view.outcome.results = Some(Results {
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
                    let o = &mut *state.view.outcome;
                    o.standings = standings.clone();
                    o.game_end = Some(GameEnd { standings, awards });
                    o.results = None;
                }
                ServerMsg::Chat { id, name, text } => {
                    // (Other people's words: not in the logs and reports a player sends.)
                    debug!("chat {name}: {text}");
                    state.view.chat.push(now, Some(id), name, text);
                    if Some(id) != state.session.me {
                        cues.write(Cue::Sfx(crate::audio::Sfx::Click));
                    }
                }
                ServerMsg::Notice(text) => {
                    info!("server: {text}");
                    state.view.chat.push(now, None, crate::ui::text::SERVER.into(), text);
                    cues.write(Cue::Sfx(crate::audio::Sfx::Click));
                }
                ServerMsg::DevAck { result, .. } => {
                    let line = match result {
                        Ok(m) => m,
                        Err(m) => format!("✖ {m}"),
                    };
                    info!("dev: {line}");
                    state.view.feed.note(now, format!("🛠 {line}"));
                }
                ServerMsg::Clock { rate } => info!("game time ×{rate}"),
                ServerMsg::Scores(s) => {
                    // In the lobby a score is the bell on the tower, rung once more (bots ring it silently).
                    let lobby = state.session.arena.as_ref().is_some_and(|a| a.kind == ArenaKind::Lobby);
                    for (id, v) in s {
                        let was = state.session.scores.insert(id, v).unwrap_or(0);
                        let bot = state.session.player(id).is_none_or(|p| p.bot);
                        if lobby && v > was && !bot {
                            cues.write(Cue::Bell(id));
                            let mine = Some(id) == state.session.me;
                            let line = crate::ui::text::bell(mine, &state.session.name_of(id), v as u32);
                            state.view.feed.note(now, line);
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
    let games: Vec<&str> = opts.start.iter().map(|m| m.as_str()).collect();
    info!("starting {} with {n} players", games.join(","));
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
                .copied()
                .collect(),
            rounds: opts.start_rounds,
        }),
    );
    send(&mut senders, ClientMsg::Start);
}
