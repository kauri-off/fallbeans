//! Port of `tests/room.test.ts` (rooms, the hub, dev tools) plus the hub's rules for rooms and PINs.
use std::collections::BTreeMap;

use fb_arena::{ArenaKind, PawnStatus};
use fb_proto::*;
use fb_shared::input::{BTN_DIVE, BTN_GRAB, BTN_JUMP, InputFrame};
use fb_shared::rng::Rng;
use fb_sim::math::V3;

use super::hub::Hub;
use super::room::{Practice, Room, RoomOptions, Who};
use super::{ConnId, Inputs, Out, ticks};

/// What every client holds down, and one-tick presses on top.
#[derive(Default)]
struct Script {
    held: BTreeMap<Pid, InputFrame>,
    press: BTreeMap<Pid, u8>,
}

impl Inputs for Script {
    fn frame(&mut self, id: Pid, _: ConnId, _: u32) -> InputFrame {
        let f = self.held.get(&id).copied().unwrap_or(InputFrame::IDLE);
        InputFrame {
            buttons: f.buttons | self.press.remove(&id).unwrap_or(0),
            ..f
        }
    }
}

/// A room or a hub on a test clock, with each connection's mail.
struct Bench {
    room: Room,
    real: u64,
    inputs: Script,
    mail: BTreeMap<ConnId, Vec<Out>>,
    next_conn: ConnId,
}

fn opts() -> RoomOptions {
    RoomOptions {
        min_players: 2,
        seed: Some(7),
        intro_ticks: ticks(1.0) as u32,
        ..Default::default()
    }
}

#[derive(Clone, Copy)]
struct Client {
    conn: ConnId,
    id: Pid,
}

impl Bench {
    fn new(o: RoomOptions) -> Self {
        Self {
            room: Room::new(o, 1000),
            real: 1000,
            inputs: Script::default(),
            mail: BTreeMap::new(),
            next_conn: 0,
        }
    }

    fn pump(&mut self) {
        for o in self.room.out.drain(..) {
            let c = match &o {
                Out::Msg(c, _) | Out::Event(c, _) | Out::Close(c) => *c,
            };
            self.mail.entry(c).or_default().push(o);
        }
    }

    /// Enters the room as `uid` (the same uid again: the same player on a new connection).
    fn hello(&mut self, name: &str, uid: &str) -> Client {
        self.hello_with(Who {
            uid: uid.into(),
            name: name.into(),
            ..Default::default()
        })
    }

    fn hello_with(&mut self, who: Who) -> Client {
        self.next_conn += 1;
        let conn = self.next_conn;
        let id = self.room.join(conn, who).expect("room has space");
        self.pump();
        Client { conn, id }
    }

    fn ctl(&mut self, c: Client, m: ClientMsg) {
        self.room.control(c.id, c.conn, &m);
        self.pump();
    }

    fn msgs(&self, c: Client) -> impl Iterator<Item = &ServerMsg> {
        self.mail.get(&c.conn).into_iter().flatten().filter_map(|o| match o {
            Out::Msg(_, m) => Some(m),
            _ => None,
        })
    }

    fn events(&self, c: Client) -> impl Iterator<Item = &MapEventKind> {
        self.mail.get(&c.conn).into_iter().flatten().filter_map(|o| match o {
            Out::Event(_, e) => Some(&e.ev),
            _ => None,
        })
    }

    fn lobby(&self, c: Client) -> Lobby {
        self.msgs(c)
            .filter_map(|m| {
                if let ServerMsg::Lobby(l) = m {
                    Some(l.clone())
                } else {
                    None
                }
            })
            .last()
            .expect("a lobby message")
    }

    fn arena_msg(&self, c: Client) -> Option<ArenaInfo> {
        self.msgs(c)
            .filter_map(|m| {
                if let ServerMsg::Arena(a) = m {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .last()
    }

    fn tick(&mut self) {
        self.real += 1;
        self.room.update(self.real, &mut self.inputs);
        self.pump();
    }

    fn advance(&mut self, s: f64) {
        for _ in 0..ticks(s) {
            self.tick();
        }
    }

    /// Runs until `cond` holds (at most `s` seconds), calling `each` every 50 ms.
    fn until(&mut self, s: f64, mut cond: impl FnMut(&Self) -> bool, mut each: impl FnMut(&mut Self)) {
        let mut t = 0.0;
        while t < s && !cond(self) {
            each(self);
            self.advance(0.05);
            t += 0.05;
        }
        assert!(cond(self), "condition not met within {s} s");
    }

    fn hold(&mut self, c: Client, mz: i8, buttons: u8) {
        self.inputs.held.insert(c.id, InputFrame { mx: 0, mz, buttons });
    }

    fn body(&mut self, id: Pid) -> &mut fb_sim::physics::Body {
        &mut self
            .room
            .arena
            .pawns
            .iter_mut()
            .find(|p| p.id == id)
            .expect("pawn")
            .body
    }
}

#[test]
fn assigns_host_and_colors_and_puts_players_into_the_lobby_world() {
    let mut t = Bench::new(opts());
    let a = t.hello("Аня", "ua");
    let b = t.hello("Боря", "ub");
    let lobby = t.lobby(b);
    assert_eq!(lobby.host, Some(a.id));
    assert_ne!(lobby.players[0].color, lobby.players[1].color);
    assert_eq!(t.arena_msg(b).map(|a| a.kind), Some(ArenaKind::Lobby));
    t.advance(0.2);
    let ids: Vec<Pid> = t.room.arena.pawns.iter().map(|p| p.id).collect();
    assert_eq!(ids, [a.id, b.id]);
}

#[test]
fn keeps_the_wished_colour_when_free_and_passes_outfits_on() {
    let mut t = Bench::new(opts());
    let who = |uid: &str| Who {
        uid: uid.into(),
        name: uid.into(),
        color: Some(4),
        outfit: None,
    };
    let a = t.hello_with(who("wa"));
    let b = t.hello_with(who("wb"));
    let colors: BTreeMap<Pid, u8> = t.lobby(b).players.iter().map(|p| (p.id, p.color)).collect();
    assert_eq!(colors[&a.id], 4);
    assert_ne!(colors[&b.id], 4);
    let o = Outfit {
        hat: fb_shared::outfit::Hat::Tophat,
        hat_color: Some(fb_shared::outfit::Tint::Red),
        glasses: fb_shared::outfit::Glasses::Shades,
        belly: None,
        shoes: Some(fb_shared::outfit::Tint::Black),
    };
    t.ctl(a, ClientMsg::Outfit(o));
    assert_eq!(t.lobby(b).players.iter().find(|p| p.id == a.id).unwrap().outfit, o);
}

#[test]
fn moves_players_only_through_their_inputs() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    let b = t.hello("B", "ub");
    t.advance(0.5);
    // On open floor between the plaza and the playground.
    t.body(a.id).reset(V3::new(-3.0, 0.05, -12.0), 0.0);
    t.body(b.id).reset(V3::new(3.0, 0.05, -12.5), 0.0);
    t.advance(0.3);
    let (start_a, start_b) = (t.body(a.id).pos, t.body(b.id).pos);
    t.hold(a, 127, 0);
    t.advance(0.3);
    assert!(t.body(a.id).pos.distance(start_a) > 1.0);
    assert!(t.body(b.id).pos.distance(start_b) < 0.2);
}

#[test]
fn only_the_host_starts_and_only_with_enough_players() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    t.ctl(a, ClientMsg::Start);
    assert_eq!(t.room.phase, Phase::Lobby);
    let b = t.hello("B", "ub");
    t.ctl(b, ClientMsg::Start);
    assert_eq!(t.room.phase, Phase::Lobby);
    t.ctl(a, ClientMsg::Start);
    assert_eq!(t.room.phase, Phase::Round);
    let mut parts = t.arena_msg(b).unwrap().participants;
    parts.sort_unstable();
    assert_eq!(parts, [a.id, b.id]);
}

#[test]
fn runs_a_game_of_points_to_a_podium_freezing_beans_before_each_start() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    let b = t.hello("B", "ub");
    t.room.playlist = Playlist {
        mode: Mode::Custom,
        games: vec!["jump-club".into(), "hex-a-gone".into()],
        rounds: 5,
    };
    t.ctl(a, ClientMsg::Start);
    assert_eq!(t.arena_msg(a).unwrap().game, "jump-club");
    // During the intro inputs move nobody.
    t.advance(0.2);
    let start = t.body(b.id).pos;
    t.hold(b, 127, BTN_JUMP);
    t.advance(0.5);
    assert!(t.body(b.id).pos.distance(start) < 0.05);
    // B walks off the edge; the server notices, not the client.
    t.hold(b, 127, 0);
    t.advance(0.4);
    t.until(5.0, |t| t.room.arena.out.contains(&b.id), |_| {});
    assert!(
        t.events(a)
            .any(|e| matches!(e, MapEventKind::Ko { id, out: true, .. } if *id == b.id))
    );
    t.until(5.0, |t| t.room.phase == Phase::Results, |_| {});
    let rows = t
        .msgs(a)
        .find_map(|m| {
            if let ServerMsg::RoundEnd { rows, .. } = m {
                Some(rows.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(rows.iter().find(|r| r.id == a.id).unwrap().points, 10);
    assert_eq!(rows.iter().find(|r| r.id == b.id).unwrap().points, 0);
    t.inputs.held.clear();
    t.until(10.0, |t| t.room.arena.map.meta().id == "hex-a-gone", |_| {});
    t.advance(1.1);
    t.hold(b, 127, 0);
    t.until(
        20.0,
        |t| t.room.phase == Phase::Podium,
        |t| {
            t.inputs.press.insert(a.id, BTN_JUMP);
        },
    );
    let end = t
        .msgs(a)
        .find_map(|m| {
            if let ServerMsg::GameEnd { standings, .. } = m {
                Some(standings.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!((end[0].id, end[0].total), (a.id, 20));
    assert_eq!(t.room.arena.kind, ArenaKind::Podium);
    t.advance(21.0);
    assert_eq!(t.room.phase, Phase::Lobby);
    assert_eq!(t.lobby(a).players.iter().find(|p| p.id == a.id).unwrap().crowns, 1);
}

#[test]
fn bots_count_as_players_and_are_simulated_on_the_server() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    t.ctl(a, ClientMsg::AddBot);
    let bot = t.lobby(a).players.iter().find(|p| p.bot).unwrap().id;
    t.room.playlist = Playlist {
        mode: Mode::Custom,
        games: vec!["door-dash".into()],
        rounds: 5,
    };
    t.ctl(a, ClientMsg::Start);
    assert_eq!(t.room.phase, Phase::Round);
    t.advance(6.0);
    assert!(t.room.arena.pawn(bot).unwrap().progress > 5.0);
}

#[test]
fn keeps_disconnected_players_during_a_game_and_resumes_by_identity() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    let b = t.hello("B", "b");
    t.ctl(a, ClientMsg::Start);
    t.room.leave(b.id, b.conn);
    t.pump();
    assert!(t.room.player(b.id).is_some());
    assert_eq!(t.room.host, Some(a.id));
    let b2 = t.hello("B", "b");
    assert_eq!(b2.id, b.id);
    assert!(
        t.msgs(b2)
            .any(|m| matches!(m, ServerMsg::Welcome { id, resumed: true, .. } if *id == b.id))
    );
    assert!(t.arena_msg(b2).unwrap().late);
}

#[test]
fn drops_disconnected_players_after_the_grace_period() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    let b = t.hello("B", "ub");
    t.ctl(a, ClientMsg::Start);
    t.room.leave(b.id, b.conn);
    t.advance(31.0);
    assert!(t.room.player(b.id).is_none());
}

#[test]
fn transfers_host_when_the_host_leaves_the_lobby() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    let b = t.hello("B", "ub");
    t.ctl(a, ClientMsg::AddBot);
    t.room.leave(a.id, a.conn);
    t.pump();
    assert_eq!(t.lobby(b).host, Some(b.id));
}

#[test]
fn lets_the_host_hand_the_role_to_another_connected_player() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    let b = t.hello("B", "ub");
    t.ctl(a, ClientMsg::AddBot);
    let bot = t.lobby(a).players.iter().find(|p| p.bot).unwrap().id;
    // Not to a bot, and nobody but the host can do it.
    t.ctl(a, ClientMsg::Host(bot));
    t.ctl(b, ClientMsg::Host(b.id));
    assert_eq!(t.lobby(b).host, Some(a.id));
    t.ctl(a, ClientMsg::Host(b.id));
    assert_eq!(t.lobby(a).host, Some(b.id));
    t.ctl(a, ClientMsg::Start);
    assert_eq!(t.room.phase, Phase::Lobby);
}

#[test]
fn tail_tag_steals_only_by_grabbing_within_reach() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    let b = t.hello("B", "ub");
    t.room.playlist = Playlist {
        mode: Mode::Custom,
        games: vec!["tail-tag".into()],
        rounds: 5,
    };
    t.ctl(a, ClientMsg::Start);
    t.advance(1.1);
    // Put them face to face; whoever has no tail grabs.
    t.body(a.id).pos = V3::new(0.0, 1.6, 0.0);
    t.body(b.id).pos = V3::new(0.0, 1.6, 1.0);
    t.body(a.id).yaw = 0.0;
    t.body(b.id).yaw = core::f64::consts::PI;
    let tails = |t: &Bench| {
        t.events(a)
            .filter(|e| matches!(e, MapEventKind::Map { name, .. } if name == "tails"))
            .count()
    };
    let before = tails(&t);
    t.hold(a, 0, BTN_GRAB);
    t.hold(b, 0, BTN_GRAB);
    t.advance(0.06);
    assert!(tails(&t) > before);
}

#[test]
fn practice_rooms_start_immediately_with_bots_and_loop() {
    let mut t = Bench::new(RoomOptions {
        min_players: 1,
        practice: Some(Practice {
            game: "jump-club".into(),
            bots: 2,
        }),
        intro_ticks: 12,
        ..opts()
    });
    let a = t.hello("A", "ua");
    assert_eq!(t.room.phase, Phase::Round);
    assert_eq!(t.arena_msg(a).unwrap().participants.len(), 3);
    // Stop right at the first round's end: rounds can be short (everyone falls).
    let ended = |t: &Bench| {
        t.msgs(a)
            .any(|m| matches!(m, ServerMsg::RoundEnd { practice: true, .. }))
    };
    t.until(80.0, ended, |_| {});
    let arenas = |t: &Bench| t.msgs(a).filter(|m| matches!(m, ServerMsg::Arena(_))).count();
    let n = arenas(&t);
    t.advance(4.0);
    assert!(arenas(&t) > n);
}

#[test]
fn ignores_dev_commands_unless_the_server_runs_with_dev() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    t.ctl(
        a,
        ClientMsg::Dev {
            q: Some(1),
            cmd: DevCmd::SkipIntro,
        },
    );
    assert!(t.msgs(a).any(|m| matches!(
        m,
        ServerMsg::DevAck {
            q: Some(1),
            ok: false,
            ..
        }
    )));
}

#[test]
fn replays_a_recorded_round_to_exactly_the_same_state() {
    let mut t = Bench::new(RoomOptions {
        min_players: 1,
        seed: Some(3),
        dev: true,
        ..opts()
    });
    let a = t.hello("A", "ua");
    let b = t.hello("B", "ub");
    let cmd = |t: &mut Bench, c: Client, cmd: DevCmd| t.ctl(c, ClientMsg::Dev { q: None, cmd });
    cmd(
        &mut t,
        a,
        DevCmd::Start {
            games: vec!["hammer-swing".into()],
            rounds: None,
            bots: Some(2),
        },
    );
    cmd(&mut t, a, DevCmd::SkipIntro);
    let mut rng = Rng::new(5);
    for i in 0..160 {
        for c in [a, b] {
            // Inputs change, or (sometimes) do not arrive and the last is held.
            if rng.next() < 0.15 {
                continue;
            }
            let buttons = if rng.next() < 0.05 {
                BTN_JUMP
            } else if rng.next() < 0.03 {
                BTN_DIVE
            } else if rng.next() < 0.1 {
                BTN_GRAB
            } else {
                0
            };
            let mx = (rng.next() * 254.0 - 127.0).round() as i8;
            t.inputs.held.insert(c.id, InputFrame { mx, mz: 100, buttons });
        }
        match i {
            40 => cmd(
                &mut t,
                a,
                DevCmd::Goto {
                    id: None,
                    to: Goto::Checkpoint(1),
                },
            ),
            60 => cmd(
                &mut t,
                a,
                DevCmd::Knock {
                    id: Some(b.id),
                    v: [2.0, 5.0, -3.0],
                },
            ),
            80 => cmd(&mut t, b, DevCmd::Bot { n: None, near: true }),
            100 => cmd(
                &mut t,
                a,
                DevCmd::Grab {
                    actor: None,
                    target: b.id,
                    s: Some(1.0),
                },
            ),
            120 => cmd(&mut t, b, DevCmd::Bots { on: false }),
            _ => {}
        }
        t.advance(0.05);
    }
    cmd(&mut t, a, DevCmd::Lobby);
    let rec = t.room.debug_replay(Some(0)).expect("a recording");
    assert_eq!(rec.game, "hammer-swing");
    let ops: Vec<&str> = rec.ops.iter().map(|o| o.1.as_str()).collect();
    for op in ["teleport", "knock", "late", "grab", "bots"] {
        assert!(ops.contains(&op), "{op} in {ops:?}");
    }
    let r = fb_arena::replay(&rec, |_| false).unwrap();
    assert!(r.ticks > 900);
    assert!(r.matches);
}

#[test]
fn builds_the_next_round_ahead_and_it_replays_the_same() {
    let mut t = Bench::new(RoomOptions {
        min_players: 1,
        seed: Some(4),
        dev: true,
        ..opts()
    });
    let a = t.hello("A", "ua");
    let cmd = |t: &mut Bench, cmd: DevCmd| t.ctl(a, ClientMsg::Dev { q: None, cmd });
    cmd(
        &mut t,
        DevCmd::Start {
            games: vec!["jump-club".into(), "door-dash".into()],
            rounds: Some(2),
            bots: Some(3),
        },
    );
    cmd(&mut t, DevCmd::EndRound);
    assert_eq!(t.room.phase, Phase::Results);
    assert!(t.room.round_prepared());
    t.until(15.0, |t| t.room.arena.map.meta().id == "door-dash", |_| {});
    assert!(!t.room.round_prepared());
    t.inputs.held.insert(
        a.id,
        InputFrame {
            mx: 0,
            mz: 127,
            buttons: 0,
        },
    );
    t.advance(8.0);
    cmd(&mut t, DevCmd::Lobby);
    let rec = t.room.debug_replay(Some(0)).expect("a recording");
    assert_eq!(rec.game, "door-dash");
    let r = fb_arena::replay(&rec, |_| false).unwrap();
    assert!(r.ticks > 900);
    assert!(r.matches);
}

#[test]
fn a_bot_gives_up_its_place_and_status_follows_the_arena() {
    let mut t = Bench::new(RoomOptions {
        max_players: 2,
        ..opts()
    });
    let a = t.hello("A", "ua");
    t.ctl(a, ClientMsg::AddBot);
    let b = t.hello("B", "ub");
    assert!(t.room.players.iter().all(|p| !p.bot));
    assert_eq!(t.room.arena.pawn(b.id).map(|p| p.status), Some(PawnStatus::Play));
}

#[test]
fn forgets_the_debug_trace_of_whoever_leaves_the_arena() {
    let mut t = Bench::new(opts());
    let a = t.hello("A", "ua");
    t.ctl(a, ClientMsg::AddBot);
    let bot = t.lobby(a).players.iter().find(|p| p.bot).unwrap().id;
    t.advance(0.5);
    assert!(t.room.arena.trace.contains_key(&bot));
    t.ctl(a, ClientMsg::RemoveBot(bot));
    assert!(!t.room.arena.trace.contains_key(&bot));
    assert!(t.room.arena.trace.contains_key(&a.id));
}

// ------------------------------------------------------------------ hub

struct HubBench {
    hub: Hub,
    real: u64,
    mail: BTreeMap<ConnId, Vec<Out>>,
    next_conn: ConnId,
}

impl HubBench {
    fn new(dev: bool) -> Self {
        let base = RoomOptions { dev, ..opts() };
        Self {
            hub: Hub::new(base, 1000),
            real: 1000,
            mail: BTreeMap::new(),
            next_conn: 0,
        }
    }

    fn pump(&mut self) {
        for o in self.hub.take_out() {
            let c = match &o {
                Out::Msg(c, _) | Out::Event(c, _) | Out::Close(c) => *c,
            };
            self.mail.entry(c).or_default().push(o);
        }
    }

    /// A connection of a new player.
    fn open(&mut self) -> ConnId {
        let uid = format!("u{}", self.next_conn + 1);
        self.open_as(&uid)
    }

    fn open_as(&mut self, uid: &str) -> ConnId {
        self.open_from("127.0.0.1", uid)
    }

    fn open_from(&mut self, ip: &str, uid: &str) -> ConnId {
        self.next_conn += 1;
        self.hub.open(self.next_conn, ip.into(), uid.into());
        self.pump();
        self.next_conn
    }

    fn send(&mut self, c: ConnId, m: ClientMsg) {
        self.hub.message(c, m);
        self.pump();
    }

    fn hello(&mut self, h: Hello) -> ConnId {
        let c = self.open();
        self.send(c, ClientMsg::Hello(h));
        c
    }

    fn hello_as(&mut self, uid: &str, h: Hello) -> ConnId {
        let c = self.open_as(uid);
        self.send(c, ClientMsg::Hello(h));
        c
    }

    fn last(&self, c: ConnId, pick: impl Fn(&ServerMsg) -> bool) -> Option<ServerMsg> {
        self.mail.get(&c)?.iter().rev().find_map(|o| match o {
            Out::Msg(_, m) if pick(m) => Some(m.clone()),
            _ => None,
        })
    }

    fn closed(&self, c: ConnId) -> bool {
        self.mail
            .get(&c)
            .is_some_and(|m| m.iter().any(|o| matches!(o, Out::Close(_))))
    }

    fn advance(&mut self, s: f64) {
        for _ in 0..ticks(s) {
            self.real += 1;
            self.hub.update(self.real, &mut super::NoInputs);
        }
        self.pump();
    }
}

#[test]
fn hub_gives_identities_lists_rooms_and_opens_private_ones() {
    let mut t = HubBench::new(false);
    let c = t.hello(Hello {
        name: "ok".into(),
        ..Default::default()
    });
    assert!(t.last(c, |m| matches!(m, ServerMsg::Ready { .. })).is_some());
    assert_eq!(
        t.last(c, |m| matches!(m, ServerMsg::Rooms { .. })),
        Some(ServerMsg::Rooms {
            rooms: vec![],
            mine: None
        })
    );
    t.send(
        c,
        ClientMsg::Create {
            title: "Наша".into(),
            private: true,
        },
    );
    let Some(ServerMsg::Welcome { id, room, .. }) = t.last(c, |m| matches!(m, ServerMsg::Welcome { .. })) else {
        panic!("no welcome");
    };
    let Some(ServerMsg::Lobby(lobby)) = t.last(c, |m| matches!(m, ServerMsg::Lobby(_))) else {
        panic!("no lobby");
    };
    assert_eq!(lobby.room.title, "Наша");
    assert!(lobby.room.private);
    assert_eq!(lobby.host, Some(id));
    let pin = lobby.pin.expect("the host is told the PIN");
    assert!(valid_pin(&pin));
    assert_eq!(t.hub.listed().count(), 1);

    // Someone else at the list sees it and needs the PIN.
    let d = t.hello(Hello {
        name: "other".into(),
        ..Default::default()
    });
    let Some(ServerMsg::Rooms { rooms, .. }) = t.last(d, |m| matches!(m, ServerMsg::Rooms { .. })) else {
        panic!("no rooms");
    };
    assert_eq!(rooms.len(), 1);
    t.send(
        d,
        ClientMsg::Join {
            room: room.clone(),
            pin: None,
        },
    );
    let denied = |t: &HubBench| t.last(d, |m| matches!(m, ServerMsg::Denied { .. }));
    assert!(matches!(denied(&t), Some(ServerMsg::Denied { reason: DenyReason::Pin, msg, .. }) if msg.is_empty()));
    let wrong = if pin == "0000" { "1111" } else { "0000" };
    t.send(
        d,
        ClientMsg::Join {
            room: room.clone(),
            pin: Some(wrong.into()),
        },
    );
    assert!(matches!(denied(&t), Some(ServerMsg::Denied { reason: DenyReason::Pin, msg, .. }) if !msg.is_empty()));
    t.send(
        d,
        ClientMsg::Join {
            room: room.clone(),
            pin: Some(pin),
        },
    );
    assert!(t.last(d, |m| matches!(m, ServerMsg::Welcome { .. })).is_some());
    // Back to the list and in again: no PIN asked any more.
    t.send(d, ClientMsg::Leave);
    assert!(t.last(d, |m| matches!(m, ServerMsg::Home { .. })).is_some());
    t.mail.clear();
    t.send(d, ClientMsg::Join { room, pin: None });
    assert!(t.last(d, |m| matches!(m, ServerMsg::Welcome { .. })).is_some());

    // A practice round: its own room, not listed.
    let p = t.hello(Hello {
        name: "p".into(),
        practice: Some("hex-a-gone".into()),
        ..Default::default()
    });
    assert_eq!(t.hub.rooms.values().filter(|r| r.practice()).count(), 1);
    let Some(ServerMsg::Arena(a)) = t.last(p, |m| matches!(m, ServerMsg::Arena(_))) else {
        panic!("no arena");
    };
    assert_eq!(a.game, "hex-a-gone");
    t.hub.close(p);
    assert_eq!(t.hub.rooms.values().filter(|r| r.practice()).count(), 0);
}

#[test]
fn hub_moves_a_player_to_their_newest_connection() {
    let mut t = HubBench::new(false);
    let c1 = t.hello_as(
        "me",
        Hello {
            name: "me".into(),
            ..Default::default()
        },
    );
    t.send(
        c1,
        ClientMsg::Create {
            title: String::new(),
            private: false,
        },
    );
    let c2 = t.hello_as(
        "me",
        Hello {
            name: "me".into(),
            ..Default::default()
        },
    );
    assert!(matches!(
        t.last(c1, |m| matches!(m, ServerMsg::Reject { .. })),
        Some(ServerMsg::Reject {
            reason: RejectReason::Moved,
            ..
        })
    ));
    assert!(t.closed(c1));
    // Back in the same room, as the same player.
    assert!(
        t.last(c2, |m| matches!(m, ServerMsg::Welcome { resumed: true, .. }))
            .is_some()
    );
    t.hub.close(c1);
    t.pump();
    assert_eq!(t.hub.listed().count(), 1);
    assert_eq!(t.hub.listed().next().unwrap().1.players.len(), 1);
}

#[test]
fn hub_closes_rooms_left_empty_and_times_out_silent_connections() {
    let mut t = HubBench::new(true);
    // The dev room is always there.
    assert_eq!(t.hub.listed().count(), 1);
    let c = t.hello(Hello {
        name: "me".into(),
        ..Default::default()
    });
    t.send(
        c,
        ClientMsg::Create {
            title: String::new(),
            private: false,
        },
    );
    assert_eq!(t.hub.listed().count(), 2);
    // The connection drops: the room waits for its owner a while, then closes.
    t.hub.close(c);
    t.advance(10.0);
    assert_eq!(t.hub.listed().count(), 2);
    t.advance(31.0);
    assert_eq!(t.hub.listed().count(), 1);
    let silent = t.open();
    t.advance(6.0);
    assert!(t.closed(silent));
    // A token without an identity is turned away.
    let nobody = t.open_as("");
    t.pump();
    assert!(t.closed(nobody));
    // Being updated: everyone is told and sent off.
    let d = t.hello(Hello::default());
    t.hub.set_updating(true);
    t.pump();
    assert!(t.last(d, |m| *m == ServerMsg::Updating).is_some());
    assert!(t.closed(d));
}

#[test]
fn hub_closes_a_room_that_panics_and_carries_on() {
    let mut t = HubBench::new(false);
    let c = t.hello(Hello {
        name: "me".into(),
        ..Default::default()
    });
    t.send(
        c,
        ClientMsg::Create {
            title: String::new(),
            private: false,
        },
    );
    let key = *t.hub.listed().next().unwrap().0;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = t.hub.with_room(key, |_| -> () { panic!("boom") });
    std::panic::set_hook(hook);
    t.pump();
    assert!(r.is_none());
    assert_eq!(t.hub.listed().count(), 0);
    assert!(
        t.last(c, |m| matches!(m, ServerMsg::Home { msg } if !msg.is_empty()))
            .is_some()
    );
    // Still served: a new room opens.
    t.send(
        c,
        ClientMsg::Create {
            title: String::new(),
            private: false,
        },
    );
    assert_eq!(t.hub.listed().count(), 1);
}

#[test]
fn one_address_cannot_take_the_whole_server() {
    let mut t = HubBench::new(false);
    let hello = |t: &mut HubBench, ip: &str, uid: &str, practice: Option<&str>| {
        let c = t.open_from(ip, uid);
        t.send(
            c,
            ClientMsg::Hello(Hello {
                practice: practice.map(String::from),
                ..Default::default()
            }),
        );
        c
    };
    let create = |t: &mut HubBench, c| {
        t.send(
            c,
            ClientMsg::Create {
                title: String::new(),
                private: false,
            },
        );
        t.last(c, |m| matches!(m, ServerMsg::Welcome { .. })).is_some()
    };
    // Rooms: three per address, then "join one of them"; another address still opens its own.
    for i in 0..3 {
        let c = hello(&mut t, "203.0.113.7", &format!("r{i}"), None);
        assert!(create(&mut t, c), "room {i}");
    }
    let c = hello(&mut t, "203.0.113.7", "r3", None);
    assert!(!create(&mut t, c));
    assert!(matches!(
        t.last(c, |m| matches!(m, ServerMsg::Denied { .. })),
        Some(ServerMsg::Denied {
            reason: DenyReason::Limit,
            ..
        })
    ));
    let other = hello(&mut t, "198.51.100.2", "x", None);
    assert!(create(&mut t, other));
    // Practice: one per address.
    let p = hello(&mut t, "203.0.113.9", "p1", Some("hex-a-gone"));
    assert!(!t.closed(p));
    let p2 = hello(&mut t, "203.0.113.9", "p2", Some("hex-a-gone"));
    assert!(t.closed(p2));
    // Connections: SESSIONS_PER_ADDRESS from one IPv6 network (any of its addresses), not from this machine.
    for i in 0..super::hub::SESSIONS_PER_ADDRESS {
        let c = t.open_from(&format!("2001:db8:5:6::{i:x}"), &format!("s{i}"));
        assert!(!t.closed(c), "connection {i}");
    }
    let c = t.open_from("2001:db8:5:6::ffff", "s-last");
    assert!(t.closed(c));
    for i in 0..40 {
        let c = t.open_from("127.0.0.1", &format!("l{i}"));
        assert!(!t.closed(c), "local connection {i}");
    }
}
