//! What goes over the wire, byte for byte: a sample of every message and replicated component (postcard,
//! as Lightyear and Replicon send them) against `wire/v<WIRE_VERSION>.txt`. A difference is a wire change
//! that old clients do not understand: bump `WIRE_VERSION` (`fb_shared::consts`), and the next run writes
//! the new version's file (delete the old one). Also catches changes that come with a dependency (serde,
//! postcard) or from a type deep in the simulation (`MapEvent`, `Outfit`).
//! Rewrite the current version's file after an intended change with
//! `FB_BLESS=1 cargo test -p fb_net --test wire`.
use bevy::math::{Vec2, Vec3};
use bevy_replicon::postcard;
use fb_net::{
    Anim, BeanColor, BodyFull, ClientMsg, FbInput, Hold, MapEventKind, MapEventMsg, PlayerId, RemotePose, Round,
    ServerMsg,
};
use fb_proto::{
    ArenaInfo, Award, DenyReason, DevCmd, Goto, Hello, Lobby, LobbyPlayer, MapEvent, Mode, Outfit, Phase, Playlist,
    RejectReason, RoomInfo, RoomRef, SessionReply, SessionRequest, Standing,
};
use fb_shared::WIRE_VERSION;
use fb_shared::game::{ArenaKind, FallBehaviour};
use fb_shared::outfit::{Glasses, Hat, Tint};
use fb_shared::rules::RoundRow;
use fb_sim::map::SegEvent;
use fb_sim::math::V3;
use fb_sim::physics::{Body, BodyState, power};
use serde::Serialize;

fn hex<T: Serialize>(v: &T) -> String {
    let mut buf = vec![0u8; 64 * 1024];
    let bytes = postcard::to_slice(v, &mut buf).expect("serializes");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The variant's name: a new one does not compile until it gets a sample here.
fn client_kind(m: &ClientMsg) -> &'static str {
    match m {
        ClientMsg::Hello(_) => "Hello",
        ClientMsg::Name(_) => "Name",
        ClientMsg::Create { .. } => "Create",
        ClientMsg::Join { .. } => "Join",
        ClientMsg::Leave => "Leave",
        ClientMsg::Color(_) => "Color",
        ClientMsg::Outfit(_) => "Outfit",
        ClientMsg::Start => "Start",
        ClientMsg::Abort => "Abort",
        ClientMsg::Playlist(_) => "Playlist",
        ClientMsg::AddBot => "AddBot",
        ClientMsg::RemoveBot(_) => "RemoveBot",
        ClientMsg::Host(_) => "Host",
        ClientMsg::Access { .. } => "Access",
        ClientMsg::Fill(_) => "Fill",
        ClientMsg::Emote(_) => "Emote",
        ClientMsg::Chat(_) => "Chat",
        ClientMsg::Dev { cmd, .. } => cmd.name(),
    }
}

fn server_kind(m: &ServerMsg) -> &'static str {
    match m {
        ServerMsg::Ready { .. } => "Ready",
        ServerMsg::Reject { .. } => "Reject",
        ServerMsg::Rooms { .. } => "Rooms",
        ServerMsg::Denied { .. } => "Denied",
        ServerMsg::Home { .. } => "Home",
        ServerMsg::Welcome { .. } => "Welcome",
        ServerMsg::Lobby(_) => "Lobby",
        ServerMsg::Arena(_) => "Arena",
        ServerMsg::RoundEnd { .. } => "RoundEnd",
        ServerMsg::GameEnd { .. } => "GameEnd",
        ServerMsg::Scores(_) => "Scores",
        ServerMsg::Emote { .. } => "Emote",
        ServerMsg::Chat { .. } => "Chat",
        ServerMsg::Left(_) => "Left",
        ServerMsg::DevAck { .. } => "DevAck",
        ServerMsg::Clock { .. } => "Clock",
    }
}

fn event_kind(e: &MapEventKind) -> &'static str {
    match e {
        MapEventKind::Bonus { .. } => "Bonus",
        MapEventKind::Finish { .. } => "Finish",
        MapEventKind::Ko { .. } => "Ko",
        MapEventKind::Map(MapEvent::Portal { .. }) => "Portal",
        MapEventKind::Map(MapEvent::Seg { ev, .. }) => match ev {
            SegEvent::Button { .. } => "Seg.Button",
            SegEvent::Door(_) => "Seg.Door",
            SegEvent::Safe(_) => "Seg.Safe",
            SegEvent::Fall { .. } => "Seg.Fall",
        },
        MapEventKind::Map(MapEvent::Tile { .. }) => "Tile",
        MapEventKind::Map(MapEvent::Star { .. }) => "Star",
        MapEventKind::Map(MapEvent::Drop { .. }) => "Drop",
        MapEventKind::Map(MapEvent::Snatch { .. }) => "Snatch",
        MapEventKind::Map(MapEvent::Tails { .. }) => "Tails",
    }
}

fn outfit() -> Outfit {
    Outfit {
        hat: Hat::Viking,
        hat_color: Some(Tint::Teal),
        glasses: Glasses::Hearts,
        belly: None,
        shoes: Some(Tint::Black),
    }
}

fn playlist() -> Playlist {
    Playlist {
        mode: Mode::Custom,
        games: vec!["door-dash".into(), "jump-club".into()],
        rounds: 3,
    }
}

fn body() -> BodyFull {
    let mut body = Body::new(3);
    body.pos = V3::new(12.345678901, -3.25, 7.000001);
    body.vel = V3::new(-8.5, 10.5, 0.125);
    body.yaw = 2.9;
    body.grounded = true;
    body.ground_col = 17;
    body.state = BodyState::Climb;
    body.state_t = 0.4;
    body.coyote = 0.05;
    body.jump_buf = 0.1;
    body.slow_until = 3.5;
    body.slow_k = 0.6;
    body.land_impact = 0.75;
    body.tilt = 1.4;
    body.tilt_dir = -0.7;
    body.power = power::GIANT;
    body.power_until = 61.25;
    body.climb_to = V3::new(1.0, 2.5, -4.0);
    BodyFull {
        body,
        teleports: 300,
        checkpoint: Some(2),
        spawn: 5,
    }
}

fn client_msgs() -> Vec<ClientMsg> {
    let dev = |cmd| ClientMsg::Dev { q: Some(7), cmd };
    vec![
        ClientMsg::Hello(Hello {
            name: "Аня".into(),
            room: Some("k7qxm".into()),
            pin: Some("0042".into()),
            practice: Some("door-dash".into()),
            color: Some(4),
            outfit: Some(outfit()),
        }),
        ClientMsg::Name("Боб".into()),
        ClientMsg::Create {
            title: "Наша комната".into(),
            private: true,
        },
        ClientMsg::Join {
            room: "k7qxm".into(),
            pin: Some("1234".into()),
        },
        ClientMsg::Leave,
        ClientMsg::Color(12),
        ClientMsg::Outfit(outfit()),
        ClientMsg::Start,
        ClientMsg::Abort,
        ClientMsg::Playlist(playlist()),
        ClientMsg::AddBot,
        ClientMsg::RemoveBot(9),
        ClientMsg::Host(2),
        ClientMsg::Access { private: false },
        ClientMsg::Fill(true),
        ClientMsg::Emote(3),
        ClientMsg::Chat("привет 👋".into()),
        dev(DevCmd::SkipIntro),
        dev(DevCmd::Warp { s: 12.5 }),
        dev(DevCmd::EndRound),
        dev(DevCmd::Start {
            games: vec!["ball-hill".into()],
            rounds: Some(2),
            bots: Some(3),
        }),
        dev(DevCmd::Lobby),
        dev(DevCmd::Rate { k: 0.25 }),
        dev(DevCmd::Step { ticks: 60 }),
        dev(DevCmd::Teleport {
            id: Some(2),
            p: [1.5, -2.0, 300.25],
            yaw: Some(3.0),
        }),
        dev(DevCmd::Goto {
            id: None,
            to: Goto::Checkpoint(3),
        }),
        dev(DevCmd::Bot { n: Some(2), near: true }),
        dev(DevCmd::Bots { on: false }),
        dev(DevCmd::Kill { id: Some(5) }),
        dev(DevCmd::Knock {
            id: None,
            v: [4.0, 8.0, -1.0],
        }),
        dev(DevCmd::Grab {
            actor: Some(1),
            target: 2,
            s: Some(1.5),
        }),
        dev(DevCmd::Seed { seed: 777 }),
    ]
}

fn server_msgs() -> Vec<ServerMsg> {
    let room = RoomInfo {
        id: "k7qxm".into(),
        title: "Наша комната".into(),
        private: true,
        host: "Аня".into(),
        players: 3,
        bots: 2,
        max: 8,
        phase: Phase::Round,
    };
    let player = LobbyPlayer {
        id: 2,
        name: "Боб".into(),
        color: 1,
        outfit: outfit(),
        score: -3,
        crowns: 1,
        spectator: false,
        bot: true,
        connected: true,
        ping: 48,
    };
    vec![
        ServerMsg::Ready { dev: true },
        ServerMsg::Reject {
            reason: RejectReason::Moved,
            msg: "открыто в другом окне".into(),
        },
        ServerMsg::Rooms {
            rooms: vec![room],
            mine: Some("k7qxm".into()),
        },
        ServerMsg::Denied {
            room: Some("k7qxm".into()),
            reason: DenyReason::Pin,
            msg: String::new(),
        },
        ServerMsg::Home {
            msg: "комната закрыта".into(),
        },
        ServerMsg::Welcome {
            id: 2,
            room: "k7qxm".into(),
            solo: false,
            practice: false,
            resumed: true,
        },
        ServerMsg::Lobby(Lobby {
            room: RoomRef {
                id: "k7qxm".into(),
                title: "Наша комната".into(),
                private: true,
            },
            phase: Phase::Results,
            host: Some(1),
            min: 2,
            max: 8,
            players: vec![player],
            playlist: playlist(),
            fill: true,
            pin: Some("0042".into()),
            next: Some(123_456),
        }),
        ServerMsg::Arena(ArenaInfo {
            id: 41,
            kind: ArenaKind::Round,
            game: "hammer-swing".into(),
            participants: vec![1, 2, 3],
            index: 2,
            total: 3,
            practice: false,
            late: true,
            finished: vec![3],
            out: vec![2],
            scores: vec![(1, 4.5)],
        }),
        ServerMsg::RoundEnd {
            game: "hammer-swing".into(),
            index: 2,
            total: 3,
            rows: vec![RoundRow {
                id: 1,
                place: 1,
                points: 10,
                penalty: 2,
                delta: 8,
                total: 18,
                ok: true,
                note: "срезал путь".into(),
                falls: 3,
            }],
            practice: false,
        },
        ServerMsg::GameEnd {
            standings: vec![Standing {
                id: 1,
                name: "Аня".into(),
                color: 0,
                place: 1,
                total: 25,
                wins: 2,
                falls: 4,
            }],
            awards: vec![Award {
                key: "falls".into(),
                title: "Неваляшка".into(),
                icon: "🤸".into(),
                id: 2,
                text: "7 падений".into(),
            }],
        },
        ServerMsg::Scores(vec![(1, 3.0), (2, -0.5)]),
        ServerMsg::Emote { id: 2, e: 4 },
        ServerMsg::Chat {
            id: 1,
            name: "Аня".into(),
            text: "гг".into(),
        },
        ServerMsg::Left(3),
        ServerMsg::DevAck {
            q: Some(7),
            ok: false,
            msg: "no such bean".into(),
        },
        ServerMsg::Clock { rate: 0.25 },
    ]
}

fn map_events() -> Vec<MapEventKind> {
    let seg = |ev| MapEventKind::Map(MapEvent::Seg { seg: 4, ev });
    vec![
        MapEventKind::Bonus { i: 3, id: 2, at: 12.25 },
        MapEventKind::Finish {
            id: 1,
            place: 2,
            time: 63.125,
        },
        MapEventKind::Ko {
            id: 2,
            out: true,
            by: Some(1),
            cause: "hammer".into(),
            shortcut: false,
        },
        MapEventKind::Map(MapEvent::Portal {
            pair: 1,
            from: 0,
            t: 20.5,
        }),
        seg(SegEvent::Button { on: true, at: 5.0 }),
        seg(SegEvent::Door(2)),
        seg(SegEvent::Safe(7)),
        seg(SegEvent::Fall { i: 3, at: 9.75 }),
        MapEventKind::Map(MapEvent::Tile { i: 120, at: 33.0 }),
        MapEventKind::Map(MapEvent::Star { k: 5, id: 3 }),
        MapEventKind::Map(MapEvent::Drop {
            from: 3,
            to: None,
            n: 2.0,
        }),
        MapEventKind::Map(MapEvent::Snatch { from: 3, to: 1 }),
        MapEventKind::Map(MapEvent::Tails { ids: vec![1, 4], by: 4 }),
    ]
}

fn lines() -> Vec<String> {
    let mut out = vec![];
    let mut put = |name: &str, bytes: String| out.push(format!("{name} {bytes}"));
    for m in client_msgs() {
        put(&format!("ClientMsg::{}", client_kind(&m)), hex(&m));
    }
    for m in server_msgs() {
        put(&format!("ServerMsg::{}", server_kind(&m)), hex(&m));
    }
    for ev in map_events() {
        let name = format!("MapEventMsg::{}", event_kind(&ev));
        let msg = MapEventMsg {
            arena: 41,
            tick: 1_234_567,
            ev,
            history: true,
        };
        put(&name, hex(&msg));
    }
    put(
        "SessionRequest",
        hex(&SessionRequest {
            identity: Some("id-token".into()),
            protocol: 21_123_456,
            transport: "ws".into(),
        }),
    );
    put(
        "SessionReply",
        hex(&SessionReply {
            protocol: 21_123_456,
            build: "0.1.0-alpha.2 (abc1234)".into(),
            identity: "id-token".into(),
            token: Some("dG9rZW4=".into()),
            ws_url: Some("wss://game.example.com/fallbeans/ws".into()),
        }),
    );
    put("PlayerId", hex(&PlayerId(70_000)));
    put("BeanColor", hex(&BeanColor(12)));
    put(
        "Round",
        hex(&Round {
            arena: 41,
            kind: ArenaKind::Round,
            map: "hidden-bridge".into(),
            seed: 123_456_789,
            zero_tick: -720,
            fall: FallBehaviour::Checkpoint,
            static_hash: "0123456789abcdef".into(),
        }),
    );
    put("BodyFull", hex(&body()));
    put(
        "RemotePose",
        hex(&RemotePose {
            pos: Vec3::new(12.34, -3.25, 7.0),
            yaw: 2.9,
            tilt: 1.4,
            tilt_dir: -0.7,
            anim: Anim::ClimbOver,
            power: power::SPEED,
            size: 1.8,
            vel: Vec2::new(-8.5, 0.125),
            teleports: 300,
        }),
    );
    put(
        "Hold",
        hex(&Hold {
            target: Some(4),
            reaching: true,
        }),
    );
    put(
        "FbInput",
        hex(&FbInput {
            mx: -127,
            mz: 90,
            buttons: 3,
        }),
    );
    out
}

#[test]
fn the_wire_is_as_recorded() {
    let text = lines().join("\n") + "\n";
    let dir = format!("{}/tests/wire", env!("CARGO_MANIFEST_DIR"));
    let path = format!("{dir}/v{WIRE_VERSION}.txt");
    let want = std::fs::read_to_string(&path).ok();
    if std::env::var("FB_BLESS").is_ok() || want.is_none() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, &text).unwrap();
        println!("wrote {path}: commit it");
        return;
    }
    let want = want.unwrap_or_default().replace("\r\n", "\n");
    for (got, want) in text.lines().zip(want.lines()) {
        assert_eq!(
            got, want,
            "a wire change: bump WIRE_VERSION (fb_shared::consts), or bless if old clients still understand it"
        );
    }
    assert_eq!(text, want, "samples added or removed: bless (FB_BLESS=1)");
}
