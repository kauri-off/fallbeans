//! The arena's state hash, compared by replays and determinism checks.
use super::*;

impl Arena {
    /// Hash of the arena's state, by the bits (replays and determinism checks compare it): every bean (body,
    /// timers, holds, rules, bot's generator), scores, who finished and who is out, and the world. Not what
    /// the room changes from outside (`teleports`), nor the round's stats.
    pub fn state_hash(&self) -> StateHash {
        let mut h = Fnv::default();
        let opt = |v: Option<u64>| v.unwrap_or(u64::MAX);
        let mut sorted: Vec<&Pawn> = self.pawns.iter().collect();
        sorted.sort_by_key(|p| p.id);
        for p in sorted {
            h.int(u64::from(p.id));
            let b = &p.body;
            for v in [
                b.pos.x,
                b.pos.y,
                b.pos.z,
                b.vel.x,
                b.vel.y,
                b.vel.z,
                b.yaw,
                b.state_t,
                b.coyote,
                b.jump_buf,
                b.slow_until,
                b.slow_k,
                b.land_impact,
                b.tilt,
                b.tilt_dir,
                b.power_until,
                b.size,
                b.climb_to.x,
                b.climb_to.y,
                b.climb_to.z,
                p.progress,
                p.hold_since,
                p.grab_ready_at,
                p.forbidden_for,
                p.force_grab_until,
            ] {
                h.bits(v);
            }
            for v in [
                b.actor as i64 as u64,
                u64::from(b.grounded),
                b.ground_col.map_or(u64::MAX, u64::from),
                b.state as u64,
                u64::from(b.power.map_or(0, |p| p as u8)),
                p.status as u64,
                opt(p.checkpoint.map(|c| c as u64)),
                opt(p.grabbing.map(u64::from)),
                opt(b.tackled.map(u64::from)),
                u64::from(p.struggle),
                u64::from(p.reaching),
                u64::from(p.view),
                p.spawn_i as u64,
                opt(p.bot.as_ref().map(|s| u64::from(s.rng.state()))),
            ] {
                h.int(v);
            }
            match p.last_hit {
                Some(hit) => {
                    h.int(opt(hit.by.map(u64::from)));
                    h.bytes(hit.cause.name().as_bytes());
                    h.bits(hit.t);
                }
                None => h.int(u64::MAX),
            }
        }
        h.int(self.seen.len() as u64);
        for (k, list) in &self.seen {
            h.int(*k as u64);
            h.int(list.len() as u64);
            // (Not the teleports: like the pawns' own, a room may shift them; only their changes count.)
            for (id, _, down, o) in list {
                h.int(u64::from(*down));
                h.int(u64::from(*id));
                for v in [o.x, o.y, o.z, o.vx, o.vy, o.vz, o.tilt, o.tilt_dir, o.size] {
                    h.bits(v);
                }
            }
        }
        h.int(self.scores.len() as u64);
        for (&id, &v) in &self.scores {
            h.int(u64::from(id));
            h.int(v as u64);
        }
        for list in [&self.finished, &self.out] {
            h.int(list.len() as u64);
            for &id in list {
                h.int(u64::from(id));
            }
        }
        h.bytes(self.world.hash(false).to_string().as_bytes());
        StateHash(h.finish())
    }
}
