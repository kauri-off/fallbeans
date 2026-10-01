//! Bytes and packets each side puts on the wire (after Lightyear's own framing, before IO), so stress
//! runs can hold the traffic budget.
use bevy::prelude::*;
use lightyear::prelude::*;

#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct NetStats {
    pub bytes_out: u64,
    pub packets_out: u64,
}

pub struct NetStatsPlugin;

impl Plugin for NetStatsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetStats>();
        // After netcode has framed and encrypted what the transport queued, before the IO takes it.
        app.add_systems(
            PostUpdate,
            count_sent.after(ConnectionSystems::Send).before(LinkSystems::Send),
        );
    }
}

/// `LinkSender` has no public iterator: each queued payload is popped, counted and pushed back in order.
fn count_sent(mut links: Query<&mut Link>, mut stats: ResMut<NetStats>) {
    for mut link in &mut links {
        let send = &mut link.bypass_change_detection().send;
        for _ in 0..send.len() {
            let Some(p) = send.pop() else { break };
            stats.bytes_out += p.len() as u64;
            stats.packets_out += 1;
            send.push(p);
        }
    }
}
