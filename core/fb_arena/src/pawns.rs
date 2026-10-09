//! A bean's tick: its bot's thinking, grabs and holds, and the map's rules.
use super::*;

impl Arena {
    pub(crate) fn think(&mut self, i: usize, events: &mut Vec<ArenaEvent>) {
        let t = self.time();
        let Some(bot) = self.pawns[i].bot.as_mut() else { return };
        bot.input = BotInput::default();
        if !self.spec.logic.bots() || !self.bots_on {
            return;
        }
        if self.nav.is_none() && (t >= 0.0 || self.kind != ArenaKind::Round) {
            // Built ahead during the intro: the same grid if the static world is the same.
            let pre = self.nav_pre.take();
            self.nav = Some(match pre {
                Some((nav, key)) if key == NavGrid::key(&self.world) => nav,
                _ => self.build_nav(),
            });
        }
        let Arena {
            pawns,
            world,
            nav,
            spec,
            scores,
            bonuses,
            views: others,
            spots,
            ..
        } = &mut *self;
        others.clear();
        others.extend(
            pawns
                .iter()
                .enumerate()
                .filter(|(j, o)| *j != i && o.status == PawnStatus::Play)
                .map(|(_, o)| {
                    let ob = &o.body;
                    let dive = ob.state == BodyState::Dive
                        || (ob.state == BodyState::Slide && m::hypot(ob.vel.x, ob.vel.z) > 6.0);
                    OtherView {
                        id: o.id,
                        pos: ob.pos,
                        vel: ob.vel,
                        down: ob.down(),
                        dive,
                        reach: o.reaching,
                    }
                }),
        );
        spots.clear();
        spots.extend(bonuses.available(t).map(|b| b.pos));
        let (others, bonuses) = (others.as_slice(), spots.as_slice());
        let p = &mut pawns[i];
        let Some(st) = p.bot.as_mut() else {
            return;
        };
        let mut out = BotInput::default();
        let mut view = BotView {
            id: p.id,
            body: &p.body,
            t,
            rng: &mut st.rng,
            mem: &mut st.mem,
            plan: &mut st.plan,
            others,
            nav: nav.as_ref().map(|g| Nav::new(g, world)),
            bonuses,
            world,
            scores,
        };
        spec.logic.bot(&mut view, &mut out);
        smooth_stick(&mut st.mem, &mut out);
        st.input = out;
        if out.emote != 0 {
            events.push(ArenaEvent::Emote { id: p.id, e: out.emote });
        }
    }

    /// Grabbing holds on (pulling the other bean along) until released, broken free or timed out.
    pub(crate) fn interact(&mut self, i: usize, active: &[usize], t: f64, events: &mut Vec<ArenaEvent>) {
        let round = self.kind == ArenaKind::Round;
        let f = self.pawns[i].frame;
        let frame_of = |pawns: &[Pawn], j: usize| active.contains(&j).then(|| pawns[j].frame);
        let p = &mut self.pawns[i];
        let wants = ((f.buttons & BTN_GRAB) != 0 || t < p.force_grab_until) && p.body.state == BodyState::Normal;
        p.reaching = wants && p.grabbing.is_none();
        if let Some(gid) = p.grabbing {
            let oi = self.index(gid);
            let p = &self.pawns[i];
            let (dist, escaped, playing) = match oi {
                Some(oi) => {
                    let o = &self.pawns[oi];
                    let ob = &o.body;
                    (
                        dist_xz(ob.pos, p.body.pos),
                        o.struggle >= STRUGGLE || ob.state == BodyState::Dive || ob.down() || ob.in_portal(),
                        o.status == PawnStatus::Play,
                    )
                }
                None => (99.0, false, false),
            };
            let timed_out = t - p.hold_since > HOLD_MAX;
            match oi {
                Some(oi) if wants && playing && !escaped && !timed_out && dist <= HOLD_BREAK => {
                    let of = frame_of(&self.pawns, oi);
                    self.hold(i, oi, dist, t, of);
                }
                _ => {
                    let p = &mut self.pawns[i];
                    p.grabbing = None;
                    p.grab_ready_at = t + if escaped || timed_out { GRAB_COOLDOWN } else { 0.3 };
                    if let Some(oi) = oi {
                        self.pawns[oi].struggle = 0;
                    }
                }
            }
        } else if wants && t >= p.grab_ready_at {
            let b = &self.pawns[i].body;
            let fx = m::sin(b.yaw);
            let fz = m::cos(b.yaw);
            let mut best = f64::INFINITY;
            let mut target = None;
            for &oj in active {
                let ob = &self.pawns[oj].body;
                if oj == i || ob.down() || ob.in_portal() {
                    continue;
                }
                let dx = ob.pos.x - b.pos.x;
                let dz = ob.pos.z - b.pos.z;
                let d = m::hypot(dx, dz);
                // Reach grows with the size of either bean (giants have long arms, and are big targets).
                let size = b.size.at_least(ob.size);
                if d > GRAB_REACH * size || (ob.pos.y - b.pos.y).abs() > 1.6 * size {
                    continue;
                }
                // Anything in front, or right beside (turning to it): forgiving, as grabbing should feel.
                let facing = if d > 0.3 { (dx * fx + dz * fz) / d } else { 1.0 };
                if facing < (if d < 1.5 { -0.35 } else { 0.1 }) {
                    continue;
                }
                // Prefer what is ahead over what is merely close.
                let score = d - facing * 0.6;
                if score >= best {
                    continue;
                }
                best = score;
                target = Some(oj);
            }
            if let Some(oj) = target {
                let (pid, tid) = (self.pawns[i].id, self.pawns[oj].id);
                self.note("grab", Some(pid), Some(json!({ "target": tid })));
                let p = &mut self.pawns[i];
                p.grabbing = Some(tid);
                p.hold_since = t;
                self.pawns[oj].struggle = 0;
                if round {
                    self.pawns[i].stats.grabs += 1;
                }
                self.with_cx(t, |logic, cx| logic.grab(cx, pid, tid));
                self.flush(events);
                self.pawns[i].reaching = false;
                let (b, ob) = (&self.pawns[i].body, &self.pawns[oj].body);
                let dist = dist_xz(ob.pos, b.pos);
                let of = frame_of(&self.pawns, oj);
                self.hold(i, oj, dist, t, of);
            }
        }
    }

    /// One tick of holding: both slow down, the held bean is pulled back within reach; jumping
    /// struggles free.
    fn hold(&mut self, i: usize, oi: usize, dist: f64, t: f64, of: Option<InputFrame>) {
        let (p, o) = pair(&mut self.pawns, i, oi);
        let b = &mut p.body;
        let ob = &mut o.body;
        // A giant held by a normal bean is barely slowed (and not dragged much).
        let heavy = ob.mass() / b.mass();
        ob.slow_until = t + 0.15;
        ob.slow_k = if heavy > 1.0 { 0.85 } else { 0.5 };
        b.slow_until = t + 0.15;
        b.slow_k = 0.7;
        o.last_hit = Some(Hit {
            by: Some(p.id),
            cause: Cause::Grab,
            t,
        });
        if of.is_some_and(|f| f.buttons & BTN_JUMP != 0) {
            o.struggle += 1;
        }
        if dist > HOLD_LEN {
            let dx = (ob.pos.x - b.pos.x) / dist;
            let dz = (ob.pos.z - b.pos.z) / dist;
            // Spring back towards the grabber; the held bean cannot outrun the hand.
            let away = ob.vel.x * dx + ob.vel.z * dz;
            if away > 0.0 {
                ob.vel.x -= (dx * away * 0.6) / heavy;
                ob.vel.z -= (dz * away * 0.6) / heavy;
            }
            let k = ((dist - HOLD_LEN) * 0.5).at_most(1.0) / heavy;
            ob.pos.x -= dx * k * 0.1;
            ob.pos.z -= dz * k * 0.1;
        }
        // Face what we hold.
        b.yaw = m::atan2(ob.pos.x - b.pos.x, ob.pos.z - b.pos.z);
    }

    pub(crate) fn rules(&mut self, i: usize, t: f64, events: &mut Vec<ArenaEvent>) {
        let round = self.kind == ArenaKind::Round;
        let spec = &self.spec;
        let p = &mut self.pawns[i];
        let pos = p.body.pos;
        if t >= 0.0 {
            p.progress = p.progress.at_least(pos.z);
        }
        if let Some(fin) = spec.finish
            && round
            && t >= 0.0
            && pos.z >= fin.z
            && pos.y > fin.y - 2.0
            && fin.half_width.is_none_or(|hw| pos.x.abs() <= hw)
        {
            if !self.frozen {
                p.status = PawnStatus::Finished;
                p.stats.finish_at = Some(t);
                let id = p.id;
                self.note("finish", Some(id), None);
                self.finished.push(id);
                events.push(ArenaEvent::Finish { id, t });
            }
            return;
        }
        reach_checkpoint(spec, &p.body, &mut p.checkpoint);
        // Standing where the course does not go (on frames, behind walls): back to the checkpoint,
        // fined. Only while standing there: being knocked off over a rail is a fall, not a shortcut.
        if p.body.grounded {
            p.forbidden_for = if round && t >= 0.0 && spec.logic.forbidden(pos) {
                p.forbidden_for + DT
            } else {
                0.0
            };
        }
        let shortcut = p.forbidden_for > FORBIDDEN_GRACE;
        let fell = shortcut || fell(spec, pos);
        if !fell {
            return;
        }
        p.forbidden_for = 0.0;
        let hit = p.last_hit.filter(|h| t - h.t < CREDIT_WINDOW);
        p.last_hit = None;
        let by = if shortcut { None } else { hit.and_then(|h| h.by) };
        let cause = if shortcut {
            Cause::Shortcut
        } else {
            hit.map_or(Cause::Fall, |h| h.cause)
        };
        let id = p.id;
        let counts = round && t >= 0.0 && !self.frozen;
        if counts {
            if let Some(by) = by
                && by != id
                && let Some(a) = self.index(by)
            {
                self.pawns[a].stats.kos += 1;
            }
            if shortcut {
                self.pawns[i].stats.shortcuts += 1;
            }
        }
        let ko = |out| KoInfo {
            id,
            out,
            by,
            cause,
            shortcut,
            pos,
        };
        let what = json!({ "by": by, "cause": cause.name(), "pos": [r3(pos.x), r3(pos.y), r3(pos.z)] });
        // (A fall during the intro is not out yet: back to the spawn.)
        if self.fall == FallBehaviour::Out && !shortcut && round && t >= 0.0 {
            if self.frozen {
                return;
            }
            let p = &mut self.pawns[i];
            p.status = PawnStatus::Out;
            p.stats.out_at = Some(t.at_least(0.0));
            self.note("out", Some(id), Some(what));
            self.out.push(id);
            events.push(ArenaEvent::Ko(ko(true)));
            return;
        }
        if counts && !shortcut {
            self.pawns[i].stats.falls += 1;
        }
        self.note(if shortcut { "shortcut" } else { "fall" }, Some(id), Some(what));
        if counts || self.kind == ArenaKind::Lobby {
            events.push(ArenaEvent::Ko(ko(false)));
        }
        if counts && !shortcut {
            self.with_cx(t, |logic, cx| logic.fall(cx, id, by));
            self.flush(events);
        }
        let p = &self.pawns[i];
        let to = match respawn_point(&self.spec, self.kind, self.fall, p.checkpoint, p.spawn_i) {
            Some(to) => to,
            None => {
                let s = self.free_spawn();
                self.spec.spawns[s]
            }
        };
        let p = &mut self.pawns[i];
        respawn(&self.spec, p.id, &mut p.body, to);
        p.teleports += 1;
    }
}
