//! What the player (and a log) needs to tell a bad network from a blocked one: transport, RTT, jitter,
//! the share of packets lost on the way to the server, why the game went over WebSocket and what the last
//! UDP check found; and, after a fallback, how to let the game through a VPN over UDP.
use std::collections::VecDeque;

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_transport::plugin::{PacketAcked, PacketLost};

use crate::net::{Conn, Fallback};

/// Seconds the loss is measured over.
const WINDOW_S: u64 = 10;
/// Seconds the full VPN hint stays up after a fallback.
pub const HINT_S: f32 = 25.0;

/// Packets acknowledged and lost, a bucket a second (by the second of real time it is for: seconds with no
/// packet have none).
#[derive(Resource, Default)]
pub struct NetDiag {
    buckets: VecDeque<(u64, u32, u32)>,
}

impl NetDiag {
    /// Share of the packets of the last `WINDOW_S` seconds (`now`: real time) whose acknowledgement never
    /// came, or came too late (None: too few).
    pub fn loss(&self, now: f32) -> Option<f32> {
        let s = now as u64;
        let (acked, lost) = self
            .buckets
            .iter()
            .filter(|(at, ..)| at + WINDOW_S > s)
            .fold((0, 0), |(a, l), (_, ba, bl)| (a + ba, l + bl));
        (acked + lost >= 30).then(|| lost as f32 / (acked + lost) as f32)
    }

    /// This second's (acked, lost).
    fn bucket(&mut self, now: f32) -> (&mut u32, &mut u32) {
        let s = now as u64;
        while self.buckets.front().is_some_and(|(at, ..)| at + WINDOW_S <= s) {
            self.buckets.pop_front();
        }
        if self.buckets.back().is_none_or(|(at, ..)| *at != s) {
            self.buckets.push_back((s, 0, 0));
        }
        match self.buckets.back_mut() {
            Some((_, acked, lost)) => (acked, lost),
            None => unreachable!("a bucket was just pushed"),
        }
    }
}

pub struct DiagPlugin;

impl Plugin for DiagPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetDiag>();
        app.add_observer(|_: On<PacketAcked>, time: Res<Time<Real>>, mut d: ResMut<NetDiag>| {
            *d.bucket(time.elapsed_secs()).0 += 1;
        });
        app.add_observer(|_: On<PacketLost>, time: Res<Time<Real>>, mut d: ResMut<NetDiag>| {
            *d.bucket(time.elapsed_secs()).1 += 1;
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
    if let Some(loss) = diag.loss(now) {
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
    use crate::ui::text;
    let (why, at) = conn.fallback?;
    if now - at > HINT_S {
        return Some(text::VPN_SHORT);
    }
    Some(match why {
        Fallback::Silent => text::VPN_SILENT,
        Fallback::Lost => text::VPN_LOST,
    })
}
