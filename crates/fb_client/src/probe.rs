//! Whether UDP works again, asked while playing over WebSocket: a netcode connection of its own, in an app of
//! its own (the game's world must not see a second client), that has to come up and hold.
use core::time::Duration;
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use fb_proto::SessionRequest;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use lightyear::connection::client::{Connect, Connected, Disconnect, Disconnected};
use lightyear::netcode::NetcodeClient;
use lightyear::netcode::auth::Authentication;
use lightyear::netcode::client_plugin::{NetcodeClientPlugin, NetcodeConfig};
use lightyear::prelude::{Link, LinkConditionerConfig, RecvLinkConditioner, UdpIo};
use lightyear_udp::UdpPlugin;

use crate::net::{UDP_TRY_S, local_addr_for, request, token_of};

/// How long the probe's connection has to hold. Well under the server's hello timeout (5 s,
/// `fb_server::rooms::hub`): the probe says no hello, and the server would warn about it every minute.
const HOLD_S: f32 = 3.0;

/// On a thread: a UDP connect token from the HTTP API at `url`, then the connection; the answer is whether it
/// came up and held.
pub fn start(url: String, req: SessionRequest, conditioner: Option<LinkConditionerConfig>) -> Receiver<bool> {
    let (tx, rx) = channel();
    let _ = std::thread::Builder::new().name("fb-probe".into()).spawn(move || {
        let _ = tx.send(run(&url, &req, conditioner));
    });
    rx
}

fn run(url: &str, req: &SessionRequest, conditioner: Option<LinkConditionerConfig>) -> bool {
    let Some(token) = request(url, req).ok().as_ref().and_then(token_of) else {
        return false;
    };
    let Ok(netcode) = NetcodeClient::new(Authentication::Token(token), NetcodeConfig::default()) else {
        return false;
    };
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.build().disable::<ScheduleRunnerPlugin>(),
        UdpPlugin,
        NetcodeClientPlugin,
    ));
    app.finish();
    app.cleanup();
    let local = local_addr_for(netcode.inner.server_addr());
    let link = app
        .world_mut()
        .spawn((
            Link::default().with_conditioner(conditioner.map(RecvLinkConditioner::new)),
            local,
            UdpIo::default(),
            netcode,
        ))
        .id();
    app.world_mut().trigger(Connect { entity: link });
    let start = Instant::now();
    let mut up: Option<Instant> = None;
    let held = loop {
        app.update();
        // The server's messages, handed back by netcode for a transport this app does not have.
        if let Some(mut l) = app.world_mut().get_mut::<Link>(link) {
            l.recv.drain().for_each(drop);
        }
        let w = app.world();
        let s = start.elapsed().as_secs_f32();
        if w.get::<Connected>(link).is_some() {
            if up.get_or_insert_with(Instant::now).elapsed().as_secs_f32() >= HOLD_S {
                break true;
            }
        } else if up.is_some() || (s > 0.2 && w.get::<Disconnected>(link).is_some()) || s > UDP_TRY_S {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if held {
        app.world_mut().trigger(Disconnect { entity: link });
        // Sends the disconnect packets: the server lets the connection go at once.
        app.update();
    }
    held
}
