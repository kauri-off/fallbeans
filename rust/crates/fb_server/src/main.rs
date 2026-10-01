//! Authoritative server (phase 0): one room playing jump-club rounds over UDP and WebSocket.
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use fb_arena::{Arena, MapEvent};
use fb_net::*;
use fb_shared::input::{BTN_DIVE, BTN_JUMP, InputFrame};
use fb_shared::{DT, INPUT_HOLD, RESULTS_S};
use lightyear::connection::client::Connected;
use lightyear::input::native::prelude::ActionState;
use lightyear::netcode::NetcodeServer;
use lightyear::prelude::input::InputBuffer;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use lightyear_replication::send::ReplicationMode;

#[derive(Resource, Clone)]
struct Opts {
    udp_port: u16,
    ws_port: u16,
    seed: Option<u32>,
    intro: f64,
    map: String,
    conditioner: Option<LinkConditionerConfig>,
}

fn parse_opts() -> Opts {
    let mut o = Opts {
        udp_port: UDP_PORT,
        ws_port: WS_PORT,
        seed: None,
        intro: 3.0,
        map: "jump-club".into(),
        conditioner: None,
    };
    let (mut lag, mut jitter, mut loss) = (0u64, 0u64, 0f32);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--udp-port" => o.udp_port = v.parse().expect("--udp-port"),
            "--ws-port" => o.ws_port = v.parse().expect("--ws-port"),
            "--seed" => o.seed = Some(v.parse().expect("--seed")),
            "--intro" => o.intro = v.parse().expect("--intro"),
            "--map" => o.map = v,
            "--lag" => lag = v.parse().expect("--lag ms (one way)"),
            "--jitter" => jitter = v.parse().expect("--jitter ms"),
            "--loss" => loss = v.parse().expect("--loss 0..1"),
            other => panic!("unknown flag {other}"),
        }
        i += 2;
    }
    if lag > 0 || jitter > 0 || loss > 0.0 {
        o.conditioner = Some(
            LinkConditionerConfig::default()
                .with_incoming_latency(Duration::from_millis(lag))
                .with_incoming_jitter(Duration::from_millis(jitter))
                .with_fixed_loss(loss),
        );
    }
    o
}

/// The room's arena and its round entity.
#[derive(Resource)]
struct Room {
    arena: Arena,
    round: Round,
    round_entity: Entity,
    /// Events of this round so far (sent to whoever joins late).
    events: Vec<MapEventMsg>,
    teleports: Vec<(u32, u32)>,
}

#[derive(Component)]
struct Pawn;

fn main() {
    let opts = parse_opts();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1))),
        LogPlugin {
            filter: "wgpu=error,bevy_ecs=warn,lightyear=warn,aeronet=warn".into(),
            ..default()
        },
        bevy::state::app::StatesPlugin,
        bevy::diagnostic::DiagnosticsPlugin,
    ));
    app.add_plugins(ServerPlugins { tick_duration: TICK });
    app.add_plugins(ProtocolPlugin);
    app.insert_resource(ReplicationMetadata::new(SEND_INTERVAL));
    app.insert_resource(opts);
    app.add_systems(Startup, (start_servers, start_room).chain());
    app.add_observer(on_link);
    app.add_observer(on_connected);
    app.add_observer(on_pawn_removed);
    app.add_systems(FixedUpdate, tick_room);
    app.run();
}

fn start_servers(mut commands: Commands, opts: Res<Opts>) {
    let netcode = || {
        NetcodeServer::new(NetcodeConfig {
            protocol_id: PROTOCOL_ID,
            private_key: DEV_KEY,
            ..default()
        })
    };
    let cond = || opts.conditioner.clone().map(RecvLinkConditioner::new);
    let udp_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), opts.udp_port);
    let udp = commands
        .spawn((Name::new("udp"), Server::new(cond()), netcode(), LocalAddr(udp_addr), ServerUdpIo::default()))
        .id();
    let ws_addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), opts.ws_port);
    let ws_config = lightyear::websocket::server::ServerConfig::builder()
        .with_bind_address(ws_addr)
        .with_no_encryption();
    let ws = commands
        .spawn((
            Name::new("ws"),
            Server::new(cond()),
            netcode(),
            LocalAddr(ws_addr),
            WebSocketServerIo { config: ws_config },
        ))
        .id();
    commands.trigger(Start { entity: udp });
    commands.trigger(Start { entity: ws });
    info!("listening: udp {udp_addr}, ws {ws_addr}, tick {} Hz", 1.0 / DT);
}

fn new_round(opts: &Opts, now: Tick, number: u32) -> (Arena, Round) {
    let map = fb_maps::by_id(&opts.map).unwrap_or_else(|| panic!("no map {}", opts.map));
    let seed = opts.seed.unwrap_or_else(|| {
        // Not simulation: any source of variety will do for the seed.
        let n = std::time::SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default().as_nanos() as u64;
        (n ^ (n >> 29) ^ (number as u64 * 0x9e37_79b9)) as u32
    });
    let intro_ticks = (opts.intro / DT).round() as u32;
    let zero_tick = now.0 + intro_ticks + 1;
    let (arena, _) = Arena::new(map, seed, now.0 as i64 - zero_tick as i64, false);
    info!("round {number}: {} seed {seed}, starts at tick {zero_tick}", opts.map);
    let static_hash = arena.static_hash.clone();
    (
        arena,
        Round {
            map: opts.map.clone(),
            seed,
            zero_tick,
            number,
            static_hash,
        },
    )
}

fn start_room(mut commands: Commands, opts: Res<Opts>, timeline: Res<LocalTimeline>) {
    let (arena, round) = new_round(&opts, timeline.tick(), 1);
    let round_entity = commands
        .spawn((round.clone(), Replicate::new(ReplicationMode::Target(NetworkTarget::All))))
        .id();
    commands.insert_resource(Room {
        arena,
        round,
        round_entity,
        events: Vec::new(),
        teleports: Vec::new(),
    });
}

/// A new link (not yet authenticated): it may receive replication once connected.
fn on_link(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands.entity(trigger.entity).insert((ReplicationSender, Name::new("client")));
}

fn on_connected(
    trigger: On<Add, Connected>,
    links: Query<(&RemoteId, &LinkOf), With<ClientOf>>,
    mut room: ResMut<Room>,
    mut commands: Commands,
    mut sender: ServerMultiMessageSender,
    servers: Query<&Server>,
) {
    let Ok((remote, link_of)) = links.get(trigger.entity) else { return };
    let peer = remote.0;
    let id = peer.to_bits() as u32;
    let pawn = room.arena.add_pawn(id);
    let full = BodyFull {
        body: pawn.body.clone(),
        teleports: 0,
    };
    let color = (room.arena.pawns.len() as u8 - 1) % fb_shared::BEAN_COLORS.len() as u8;
    commands.spawn((
        Pawn,
        PlayerId(id),
        BeanColor(color),
        RemotePose::of(&full),
        full,
        Replicate::new(ReplicationMode::Target(NetworkTarget::All)),
        PredictionTarget::new(ReplicationMode::Target(NetworkTarget::Single(peer))),
        InterpolationTarget::new(ReplicationMode::Target(NetworkTarget::AllExceptSingle(peer))),
        ControlledBy {
            owner: trigger.entity,
            lifetime: Default::default(),
        },
    ));
    if let Ok(server) = servers.get(link_of.server) {
        for msg in &room.events {
            let _ = sender.send::<_, MapEventsChannel>(msg, server, &NetworkTarget::Single(peer));
        }
    }
    info!("player {id} joined ({peer:?})");
}

fn on_pawn_removed(trigger: On<Remove, Pawn>, pawns: Query<&PlayerId>, room: Option<ResMut<Room>>) {
    if let (Ok(id), Some(mut room)) = (pawns.get(trigger.entity), room) {
        room.arena.remove_pawn(id.0);
        info!("player {} left", id.0);
    }
}

/// The input for `tick` straight from the buffer (Lightyear's own copy into `ActionState` needs a single
/// started server, and this one runs two: UDP and WebSocket). TS rules for a tick without input: keep the
/// stick for a moment, never repeat a jump or dive.
fn frame_for(tick: Tick, buffer: Option<&InputBuffer<ActionState<FbInput>, FbInput>>) -> InputFrame {
    let Some(b) = buffer else { return InputFrame::IDLE };
    if let Some(s) = b.get(tick) {
        return s.0.into();
    }
    match b.get_last_with_tick() {
        Some((last, s)) if last < tick && tick.0 - last.0 <= INPUT_HOLD => {
            let f: InputFrame = s.0.into();
            InputFrame {
                buttons: f.buttons & !(BTN_JUMP | BTN_DIVE),
                ..f
            }
        }
        _ => InputFrame::IDLE,
    }
}

fn tick_room(
    timeline: Res<LocalTimeline>,
    opts: Res<Opts>,
    mut room: ResMut<Room>,
    mut pawns: Query<
        (
            &PlayerId,
            Option<&InputBuffer<ActionState<FbInput>, FbInput>>,
            &mut BodyFull,
            &mut RemotePose,
        ),
        With<Pawn>,
    >,
    mut rounds: Query<&mut Round>,
    mut sender: ServerMultiMessageSender,
    servers: Query<&Server>,
) {
    let tick = timeline.tick();
    let room = &mut *room;
    let k = room.round.arena_tick(tick);
    if k as f64 * DT > room.arena.map.meta().duration + RESULTS_S {
        let (mut arena, round) = new_round(&opts, tick, room.round.number + 1);
        for p in &room.arena.pawns {
            arena.add_pawn(p.id);
        }
        room.arena = arena;
        room.round = round.clone();
        room.events.clear();
        if let Ok(mut r) = rounds.get_mut(room.round_entity) {
            *r = round;
        }
        return;
    }
    if tick.0 % 120 == 0 {
        let ys: Vec<String> = room.arena.pawns.iter().map(|p| format!("{}: y {:.1}", p.id, p.body.pos.y)).collect();
        info!("tick {} t {:.1}: {} pawns synced of {} [{}]", tick.0, k as f64 * DT, pawns.iter().count(), room.arena.pawns.len(), ys.join(", "));
    }
    let frames: Vec<(u32, InputFrame)> = pawns
        .iter()
        .map(|(id, buffer, _, _)| (id.0, frame_for(tick, buffer).clamped()))
        .collect();
    let events = room.arena.step(k, |id| frames.iter().find(|f| f.0 == id).map_or(InputFrame::IDLE, |f| f.1));
    if std::env::var_os("FB_TRACE").is_some() {
        for (id, f) in &frames {
            if let Some(p) = room.arena.pawn(*id) {
                let b = &p.body;
                eprintln!("S {} {} {} {} {:.6} {:.6} {:.6}", tick.0, f.mx, f.mz, f.buttons, b.pos.x, b.pos.y, b.pos.z);
            }
        }
    }
    for e in events {
        let MapEvent::Bonus(b) = e;
        let msg = MapEventMsg {
            round: room.round.number,
            tick: tick.0,
            ev: MapEventKind::Bonus {
                i: b.i,
                id: b.id,
                at: b.at,
            },
        };
        room.events.push(msg);
        for server in &servers {
            let _ = sender.send::<_, MapEventsChannel>(&msg, server, &NetworkTarget::All);
        }
        info!("tick {}: player {} took bonus {}", tick.0, b.id, b.i);
    }
    for (id, _, mut full, mut pose) in &mut pawns {
        let Some(p) = room.arena.pawn(id.0) else { continue };
        let tp = room.teleports.iter_mut().find(|t| t.0 == id.0);
        let teleports = match (tp, p.teleported) {
            (Some(t), true) => {
                t.1 += 1;
                t.1
            }
            (Some(t), false) => t.1,
            (None, _) => {
                room.teleports.push((id.0, 0));
                0
            }
        };
        let next = BodyFull {
            body: p.body.clone(),
            teleports,
        };
        if *full != next {
            *full = next;
            *pose = RemotePose::of(&full);
        }
    }
}
