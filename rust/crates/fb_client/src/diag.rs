//! What the player (and a log) needs to tell a bad network from a blocked one: transport, RTT, jitter,
//! the share of packets lost on the way to the server, why the game went over WebSocket and what the last
//! UDP check found; and, after a fallback, how to let the game through a VPN over UDP (`plan.md` §5).
use std::collections::VecDeque;

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_transport::plugin::{PacketAcked, PacketLost};

use crate::net::{Conn, Fallback};

/// Seconds the loss is measured over.
const WINDOW_S: usize = 10;
/// Seconds the full VPN hint stays up after a fallback.
pub const HINT_S: f32 = 25.0;

/// Packets sent and acknowledged or lost, a bucket a second.
#[derive(Resource, Default)]
pub struct NetDiag {
    buckets: VecDeque<(u32, u32)>,
    second: u64,
}

impl NetDiag {
    /// Share of the packets sent in the last seconds whose acknowledgement never came, or came too late
    /// (None: too few).
    pub fn loss(&self) -> Option<f32> {
        let (acked, lost) = self.buckets.iter().fold((0, 0), |(a, l), (ba, bl)| (a + ba, l + bl));
        (acked + lost >= 30).then(|| lost as f32 / (acked + lost) as f32)
    }

    fn bucket(&mut self, now: f32) -> &mut (u32, u32) {
        let s = now as u64;
        if s != self.second || self.buckets.is_empty() {
            self.second = s;
            self.buckets.push_back((0, 0));
            if self.buckets.len() > WINDOW_S {
                self.buckets.pop_front();
            }
        }
        self.buckets.back_mut().unwrap()
    }
}

pub struct DiagPlugin;

impl Plugin for DiagPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetDiag>();
        app.add_observer(|_: On<PacketAcked>, time: Res<Time<Real>>, mut d: ResMut<NetDiag>| {
            d.bucket(time.elapsed_secs()).0 += 1;
        });
        app.add_observer(|_: On<PacketLost>, time: Res<Time<Real>>, mut d: ResMut<NetDiag>| {
            d.bucket(time.elapsed_secs()).1 += 1;
        });
        app.add_observer(|_: On<Add, Connected>, mut d: ResMut<NetDiag>| d.buckets.clear());
    }
}

/// One line: transport, RTT, jitter, loss, and why WebSocket.
pub fn summary(conn: Option<&Conn>, link: Option<&Link>, diag: &NetDiag, now: f32) -> String {
    let Some(conn) = conn else {
        return "net: —".into();
    };
    let state = if conn.connected { "" } else { " (connecting)" };
    let mut s = format!("net: {:?}{state}", conn.transport);
    if let Some(l) = link.filter(|_| conn.connected) {
        s += &format!(
            " | rtt {:.0} ms jitter {:.0} ms",
            l.stats.rtt.as_secs_f32() * 1000.0,
            l.stats.jitter.as_secs_f32() * 1000.0
        );
    }
    if let Some(loss) = diag.loss() {
        s += &format!(" | loss {:.1}%", loss * 100.0);
    }
    match conn.fallback {
        Some((Fallback::Silent, _)) => s += " | UDP: no answer",
        Some((Fallback::Lost, _)) => s += " | UDP: lost mid-game",
        None => {}
    }
    if let Some((at, works)) = conn.udp_check {
        s += &format!(
            " | UDP check {:.0} s ago: {}",
            now - at,
            if works { "works" } else { "fails" }
        );
    }
    s
}

/// For the player, after `auto` went over WebSocket: what that means and how to let UDP through a VPN.
pub fn vpn_hint(conn: &Conn, now: f32) -> Option<&'static str> {
    let (why, at) = conn.fallback?;
    if now - at > HINT_S {
        return Some("Игра идёт через WebSocket: UDP до сервера не доходит");
    }
    Some(match why {
        Fallback::Silent => {
            "Игра идёт через WebSocket: UDP до сервера не доходит, задержка может быть больше.\n\
             Если включён VPN, пустите адрес сервера мимо него (direct):\n\
             v2rayN — правило «IP сервера: direct» выше правила, блокирующего udp443;\n\
             sing-box, Hiddify, NekoBox — правило ip_cidr или domain сервера.\n\
             Для игры лучше протокол с родным UDP: Hysteria2 или TUIC."
        }
        Fallback::Lost => {
            "UDP пропал посреди игры (переподключился VPN или поменялись его правила): игра идёт через WebSocket.\n\
             Раз в минуту игра проверяет UDP и вернётся на него между раундами.\n\
             Чтобы UDP не пропадал, пустите адрес сервера мимо VPN (direct)."
        }
    })
}
