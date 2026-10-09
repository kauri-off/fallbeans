//! Dev tools (server --dev only) and replayed operations.
use super::*;

impl Arena {
    /// Adds a pawn to a running arena (a bot joining mid-round), at `at` or the next spawn.
    pub fn add_late_pawn(&mut self, id: u32, bot: bool, at: Option<V3>) {
        if self.index(id).is_some() {
            return;
        }
        self.op(Op::Late { id, bot, at });
        // The op adds it again on replay: keep it out of the list of pawns added at the start.
        let n = self.recording.as_ref().map_or(0, |r| r.pawns.len());
        if !self.participants.contains(&id) {
            self.participants.push(id);
        }
        self.add_pawn(id, bot);
        if let Some(r) = &mut self.recording
            && r.pawns.len() > n
        {
            r.pawns.pop();
        }
        if let Some(p) = at {
            self.place(id, p, None);
        }
    }

    fn place(&mut self, id: u32, pos: V3, yaw: Option<f64>) -> bool {
        let Some(i) = self.index(id) else { return false };
        let p = &mut self.pawns[i];
        if p.status != PawnStatus::Play {
            return false;
        }
        let yaw = yaw.unwrap_or(p.body.yaw);
        p.body.reset(pos, yaw);
        p.teleports += 1;
        p.forbidden_for = 0.0;
        true
    }

    pub fn dev_teleport(&mut self, id: u32, pos: V3, yaw: Option<f64>) -> bool {
        self.op(Op::Teleport { id, pos, yaw });
        self.place(id, pos, yaw)
    }

    /// Where `goto` sends a bean: its spawn, a checkpoint, or 3 m before the finish line.
    pub fn dev_place(&self, id: u32, to: DevPlace) -> Option<V3> {
        let p = self.pawn(id)?;
        match to {
            DevPlace::Spawn => Some(p.spawn),
            DevPlace::Finish => self.spec.finish.map(|f| V3::new(0.0, f.y + 1.5, f.z - 3.0)),
            DevPlace::Checkpoint(i) => self.spec.checkpoints.get(i).map(|c| V3::new(c.p.x, c.p.y + 0.5, c.p.z)),
        }
    }

    pub fn dev_knock(&mut self, id: u32, v: V3) -> bool {
        self.op(Op::Knock { id, v });
        let t = self.time();
        let Some(i) = self.index(id) else { return false };
        let p = &mut self.pawns[i];
        if p.status != PawnStatus::Play {
            return false;
        }
        p.body.knock(&mut p.ev, v.x, v.z, v.y, 1.0, false);
        p.last_hit = Some(Hit {
            by: None,
            cause: Cause::Dev,
            t,
        });
        true
    }

    /// Drops a bean below the kill height: it falls by the map's rules on the next tick.
    pub fn dev_kill(&mut self, id: u32) -> bool {
        self.op(Op::Kill(id));
        let kill_y = self.spec.kill_y;
        let Some(i) = self.index(id) else { return false };
        let p = &mut self.pawns[i];
        if p.status != PawnStatus::Play {
            return false;
        }
        p.body.pos.y = kill_y - 1.0;
        true
    }

    pub fn dev_grab(&mut self, actor: u32, target: u32, seconds: f64) -> Result<(), &'static str> {
        self.op(Op::Grab { actor, target, seconds });
        let (Some(a), Some(o)) = (self.index(actor), self.index(target)) else {
            return Err("no such beans in play");
        };
        if a == o || self.pawns[a].status != PawnStatus::Play || self.pawns[o].status != PawnStatus::Play {
            return Err("no such beans in play");
        }
        let t = self.time();
        let p = &mut self.pawns[a];
        p.force_grab_until = t + seconds;
        p.grabbing = Some(target);
        p.hold_since = t;
        self.pawns[o].struggle = 0;
        Ok(())
    }

    /// Replays: applies a recorded operation.
    pub fn apply_op(&mut self, op: &Op) {
        match *op {
            Op::Remove(id) => self.remove_pawn(id),
            Op::Late { id, bot, at } => self.add_late_pawn(id, bot, at),
            Op::Teleport { id, pos, yaw } => {
                self.dev_teleport(id, pos, yaw);
            }
            Op::Knock { id, v } => {
                self.dev_knock(id, v);
            }
            Op::Kill(id) => {
                self.dev_kill(id);
            }
            Op::Grab { actor, target, seconds } => {
                let _ = self.dev_grab(actor, target, seconds);
            }
            Op::Bots(on) => self.set_bots_on(on),
            Op::Freeze => self.freeze(),
            Op::View { id, ticks } => self.set_view(id, ticks),
            Op::Skip(k) => self.skip_to(k),
        }
    }
}

/// Where a dev `goto` sends a bean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevPlace {
    Spawn,
    Finish,
    Checkpoint(usize),
}
