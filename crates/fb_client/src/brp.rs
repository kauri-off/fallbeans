//! `--brp`: Bevy Remote Protocol with the game's own `fb/*` methods.
use bevy::prelude::*;
use bevy::remote::http::RemoteHttpPlugin;
use bevy::remote::{BrpError, BrpResult, RemotePlugin, error_codes};
use fb_net::*;
use fb_proto::{ClientMsg, DevCmd};
use fb_shared::input::{BTN_DIVE, BTN_GRAB, BTN_JUMP, InputFrame};
use lightyear::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::game::{Map, ProbeInput, Stats};
use crate::net::Conn;
use crate::session::{Session, send};
use crate::ui::{Fold, Folds, HomeTab, MenuTab, Ui};

pub struct BrpPlugin {
    pub port: u16,
}

impl Plugin for BrpPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            RemotePlugin::default()
                .with_method_main("fb/state", state)
                .with_method_main("fb/send", send_msg)
                .with_method_main("fb/dev", dev)
                .with_method_main("fb/input", input)
                .with_method_main("fb/shot", shot)
                .with_method_main("fb/ui", ui_state)
                .with_method_main("fb/camera", camera),
            RemoteHttpPlugin::default().with_port(self.port),
        ));
        info!("BRP on 127.0.0.1:{}", self.port);
    }
}

fn bad(message: impl Into<String>) -> BrpError {
    BrpError {
        code: error_codes::INVALID_PARAMS,
        message: message.into(),
        data: None,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> Result<T, BrpError> {
    serde_json::from_value(params.ok_or_else(|| bad("params required"))?).map_err(|e| bad(e.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn state(
    In(_): In<Option<Value>>,
    conn: Option<Res<Conn>>,
    session: Res<Session>,
    map: Option<Res<Map>>,
    stats: Res<Stats>,
    timeline: Res<LocalTimeline>,
    links: Query<&Link>,
    own: Query<(&BeanId, &BodyFull), With<Predicted>>,
    others: Query<(&BeanId, &RemotePose), (With<Interpolated>, Without<Predicted>)>,
    metrics: Option<Res<PredictionMetrics>>,
) -> BrpResult {
    let lobby = session.lobby.as_ref().map(|l| {
        json!({
            "phase": l.phase,
            "host": l.host,
            "players": l.players.iter().map(|p| json!({
                "id": p.id, "name": p.name, "bot": p.bot, "score": p.score, "spectator": p.spectator,
            })).collect::<Vec<_>>(),
        })
    });
    let arena = map.as_ref().map(|m| {
        let a = session.arena.as_ref().filter(|a| a.id == m.round.arena);
        json!({
            "id": m.round.arena,
            "map": m.round.map,
            "kind": m.round.kind,
            "seed": m.round.seed,
            "t": m.time(f64::from(timeline.tick().0)),
            "index": a.map(|a| a.index),
            "total": a.map(|a| a.total),
            "participants": m.info.participants,
            "finished": m.info.finished,
            "out": m.info.out,
            "hash_ok": m.static_hash == m.round.static_hash,
        })
    });
    let own = own.single().ok().map(|(id, f)| {
        let b = &f.body;
        json!({
            "id": id.0,
            "pos": [b.pos.x, b.pos.y, b.pos.z],
            "vel": [b.vel.x, b.vel.y, b.vel.z],
            "state": format!("{:?}", b.state),
            "grounded": b.grounded,
            "power": b.power,
        })
    });
    let mut others: Vec<Value> = others
        .iter()
        .map(|(id, p)| json!({ "id": id.0, "pos": [p.pos.x, p.pos.y, p.pos.z], "anim": format!("{:?}", p.anim) }))
        .collect();
    others.sort_by_key(|o| o["id"].as_u64());
    let status = match (&map, &own) {
        (None, _) => "none",
        (Some(_), Some(_)) => "play",
        (Some(m), None) => match session.me {
            Some(me) if m.info.finished.contains(&me) => "finished",
            Some(me) if m.info.out.contains(&me) => "out",
            _ => "spectating",
        },
    };
    Ok(json!({
        "connected": conn.as_ref().is_some_and(|c| c.connected),
        "transport": conn.as_ref().map(|c| format!("{:?}", c.transport)),
        "rtt_ms": links.iter().next().map(|l| l.stats.rtt.as_secs_f64() * 1000.0),
        "tick": timeline.tick().0,
        "me": session.me,
        "room": session.room,
        "dev": session.dev,
        "lobby": lobby,
        "arena": arena,
        "status": status,
        "own": own,
        "others": others,
        "scores": session.scores,
        "rollbacks": metrics.map(|m| m.rollbacks),
        "predicted_ticks": stats.ticks,
        "map_events": stats.map_events,
        "hash_mismatch": stats.hash_mismatch,
    }))
}

fn send_checked(senders: &mut Query<&mut MessageSender<ClientMsg>, With<Client>>, msg: ClientMsg) -> BrpResult {
    msg.check().map_err(|e| bad(format!("out of bounds: {e}")))?;
    if senders.is_empty() {
        return Err(bad("not connected"));
    }
    send(senders, msg);
    Ok(Value::Null)
}

fn send_msg(
    In(params): In<Option<Value>>,
    mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>,
) -> BrpResult {
    send_checked(&mut senders, parse(params)?)
}

fn dev(In(params): In<Option<Value>>, mut senders: Query<&mut MessageSender<ClientMsg>, With<Client>>) -> BrpResult {
    let cmd: DevCmd = parse(params)?;
    send_checked(&mut senders, ClientMsg::Dev { q: None, cmd })
}

#[derive(Deserialize)]
struct InputParams {
    #[serde(default)]
    mx: f64,
    #[serde(default)]
    mz: f64,
    #[serde(default)]
    jump: bool,
    #[serde(default)]
    dive: bool,
    #[serde(default)]
    grab: bool,
    #[serde(default)]
    secs: f64,
}

fn input(In(params): In<Option<Value>>, mut commands: Commands, time: Res<Time<Real>>) -> BrpResult {
    let p: InputParams = parse(params)?;
    if !(p.mx.is_finite() && p.mz.is_finite() && (0.0..=60.0).contains(&p.secs)) {
        return Err(bad("mx, mz finite, secs within 0..60"));
    }
    let bit = |on: bool, b: u8| if on { b } else { 0 };
    let buttons = bit(p.jump, BTN_JUMP) | bit(p.dive, BTN_DIVE) | bit(p.grab, BTN_GRAB);
    commands.insert_resource(ProbeInput {
        frame: InputFrame::from_stick(p.mx, p.mz, buttons),
        until: time.elapsed_secs_f64() + p.secs,
    });
    Ok(Value::Null)
}

#[derive(Deserialize)]
struct ShotParams {
    path: String,
}

/// A screenshot of the window to a file (written a frame or two later).
fn shot(In(params): In<Option<Value>>, mut commands: Commands, offscreen: Option<Res<crate::Offscreen>>) -> BrpResult {
    let p: ShotParams = parse(params)?;
    commands
        .spawn(crate::shot_of(offscreen.as_deref()))
        .observe(bevy::render::view::screenshot::save_to_disk(p.path));
    Ok(Value::Null)
}

#[derive(Deserialize)]
struct UiParams {
    menu: Option<bool>,
    /// "game", "settings" or "dev" (the menu's tabs); "rooms" or "home-settings" at the room list.
    tab: Option<String>,
    /// Opens or folds a folded part ("gfx", "keys", "problems", "practice", "outfit", "dev-maps").
    fold: Option<Fold>,
    debug: Option<bool>,
}

/// Drives the interface the way the player's clicks would (menu, tabs, folded parts, F3).
fn ui_state(
    In(params): In<Option<Value>>,
    ui: Option<ResMut<Ui>>,
    folds: Option<ResMut<Folds>>,
    home: Option<ResMut<NextState<HomeTab>>>,
    menu: Option<ResMut<NextState<MenuTab>>>,
    overlay: Option<ResMut<crate::hud::Overlay>>,
) -> BrpResult {
    let p: UiParams = parse(params)?;
    let (Some(mut ui), Some(mut folds), Some(mut home), Some(mut menu), Some(mut overlay)) =
        (ui, folds, home, menu, overlay)
    else {
        return Err(bad("no interface (headless)"));
    };
    if let Some(m) = p.menu {
        ui.menu = m;
    }
    match p.tab.as_deref() {
        Some("game") => menu.set(MenuTab::Game),
        Some("settings") => menu.set(MenuTab::Settings),
        Some("dev") => menu.set(MenuTab::Dev),
        Some("rooms") => home.set(HomeTab::Main),
        Some("home-settings") => home.set(HomeTab::Settings),
        Some(t) => return Err(bad(format!("no tab {t}"))),
        None => {}
    }
    if let Some(k) = p.fold {
        folds.toggle(k);
    }
    if let Some(d) = p.debug {
        overlay.0 = d;
    }
    Ok(json!({ "menu": ui.menu, "chat": ui.chat, "need_click": ui.need_click }))
}

#[derive(Deserialize)]
struct CameraParams {
    eye: [f32; 3],
    look: [f32; 3],
}

/// A fixed camera (`{"eye":[x,y,z],"look":[x,y,z]}`), or back to the game's (no params).
fn camera(In(params): In<Option<Value>>, mut commands: Commands) -> BrpResult {
    match params {
        None | Some(Value::Null) => commands.remove_resource::<crate::camera::CameraOverride>(),
        p => {
            let p: CameraParams = parse(p)?;
            commands.insert_resource(crate::camera::CameraOverride {
                eye: Vec3::from(p.eye),
                look: Vec3::from(p.look),
            });
        }
    }
    Ok(Value::Null)
}
