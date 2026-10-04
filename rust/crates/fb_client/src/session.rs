//! The client's side of the control protocol (no UI yet, Phase 5): the hello, where the player is (room list,
//! a room, its lobby and arena), and the log of what the server says. `--start` plays a game by itself as
//! the room's host (stress runs, `xtask dev`).
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
    opts: Res<Opts>,
    session: Res<Session>,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
) {
    let hello = Hello {
        name: opts.name.clone(),
        // Back into the room after a reconnect.
        room: session
            .room
            .clone()
            .filter(|r| !r.is_empty())
            .or_else(|| opts.room.clone()),
        pin: opts.pin.clone(),
        practice: opts.practice.clone(),
        color: opts.color,
        outfit: None,
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
) {
    for mut r in &mut receivers {
        for msg in r.receive() {
            match msg {
                ServerMsg::Ready { dev } => session.dev = dev,
                ServerMsg::Reject { reason, msg } => {
                    warn!("refused ({reason:?}): {msg}");
                    session.refused = true;
                    if let Some(entity) = conn.as_ref().and_then(|c| c.entity) {
                        commands.trigger(Disconnect { entity });
                    }
                }
                // Back once the new version is up (the session API says when).
                ServerMsg::Updating => {
                    warn!("the game is being updated");
                    if let Some(entity) = conn.as_ref().and_then(|c| c.entity) {
                        commands.trigger(Disconnect { entity });
                    }
                }
                ServerMsg::Rooms { rooms, mine } => {
                    session.me = None;
                    session.room = None;
                    let list: Vec<String> = rooms
                        .iter()
                        .map(|r| format!("{} «{}» {}+{}/{}", r.id, r.title, r.players, r.bots, r.max))
                        .collect();
                    info!("rooms: [{}] mine {mine:?}", list.join(", "));
                }
                ServerMsg::Denied { room, reason, msg } => warn!("room {room:?} denied ({reason:?}): {msg}"),
                ServerMsg::Home { .. } => {
                    session.me = None;
                    session.room = None;
                    session.lobby = None;
                    session.arena = None;
                }
                ServerMsg::Welcome { id, room, resumed, .. } => {
                    info!(
                        "in room {room:?} as player {id}{}",
                        if resumed { " (resumed)" } else { "" }
                    );
                    session.me = Some(id);
                    session.room = Some(room);
                }
                ServerMsg::Lobby(l) => {
                    if l.phase == Phase::Lobby && session.lobby.as_ref().is_none_or(|o| o.phase != Phase::Lobby) {
                        session.started = false;
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
                    session.arena = Some(a);
                }
                ServerMsg::RoundEnd { game, index, rows, .. } => {
                    let rows: Vec<String> = rows.iter().map(|r| format!("#{} {:+}", r.id, r.delta)).collect();
                    info!("round {index} ({game}) over: {}", rows.join(" "));
                    cues.write(Cue::Results);
                }
                ServerMsg::GameEnd { standings, .. } => {
                    let s: Vec<String> = standings.iter().map(|s| format!("{} {}", s.name, s.total)).collect();
                    info!("game over: {}", s.join(", "));
                    session.standings = standings;
                }
                ServerMsg::Chat { name, text, .. } => info!("chat {name}: {text}"),
                ServerMsg::DevAck { ok, msg, .. } => info!("dev: {} {msg}", if ok { "ok" } else { "failed" }),
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

/// `--start <map>`: as the host, plays a game of that map (rounds of it) once enough players are in.
fn start_game(
    opts: Res<Opts>,
    mut session: ResMut<Session>,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
) {
    let Some(map) = &opts.start else { return };
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
    info!("starting {map} with {n} players");
    if opts.fill {
        send(&mut senders, ClientMsg::Fill(true));
    }
    send(
        &mut senders,
        ClientMsg::Playlist(Playlist {
            mode: Mode::Custom,
            games: vec![map.clone(); opts.start_rounds as usize],
            rounds: opts.start_rounds,
        }),
    );
    send(&mut senders, ClientMsg::Start);
}
