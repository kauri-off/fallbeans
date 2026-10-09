//! `--trace hits`: the own bean's notes per predicted tick, `C tick id …` (`c`: a rollback's replay), and while it
//! tackles the others as drawn; the interpolation delay (`view`) when it changes; `F` lines: the bean as drawn.
use std::collections::BTreeMap;

use bevy::prelude::*;
use fb_arena::Stepper;
use fb_net::trace::TraceFile;
use fb_shared::m;
use fb_sim::beans;
use fb_sim::math::V3;
use fb_sim::physics::OtherBody;

#[derive(Resource)]
pub struct Hits {
    out: TraceFile,
    view: Option<u32>,
    quiet: beans::NoteQuiet,
    /// The own bean as last predicted at each recent tick (pos, vel, yaw): a rollback's replay is compared with it.
    pred: BTreeMap<i64, (V3, V3, f64)>,
}

impl Hits {
    pub fn new(out: TraceFile) -> Self {
        Self {
            out,
            view: None,
            quiet: beans::NoteQuiet::default(),
            pred: BTreeMap::new(),
        }
    }

    /// One line as it is (the drawing's `F` lines, `beans::animate_beans`).
    pub fn line(&mut self, l: core::fmt::Arguments) {
        self.out.line(l);
    }

    /// The lines of predicted tick k.
    pub fn write(&mut self, k: i64, s: &Stepper, replay: bool, view: Option<u32>, extra: &[OtherBody]) {
        let (id, b, ev) = (s.id, &*s.body, &*s.ev);
        let c = if replay { 'c' } else { 'C' };
        let v3 = |x: f64, y: f64, z: f64| format!("{x:.2},{y:.2},{z:.2}");
        // A replay that ends elsewhere than the prediction it replaces: the correction the drawing jumps by.
        let now = (b.pos, b.vel, b.yaw);
        if let Some((p, v, y)) = self.pred.insert(k, now).filter(|_| replay) {
            let (dp, dv) = (b.pos - p, b.vel - v);
            let dy = (b.yaw - y).sin().atan2((b.yaw - y).cos());
            if dp.length() > 1e-3 || dv.length() > 1e-2 || dy.abs() > 1e-3 {
                self.out.line(format_args!(
                    "c {k} {id} corr dpos={} dvel={} dyaw={dy:.3}",
                    v3(dp.x, dp.y, dp.z),
                    v3(dv.x, dv.y, dv.z)
                ));
            }
        }
        while self.pred.len() > 256 {
            self.pred.pop_first();
        }
        // (It swings a tick either way as frames and ticks beat: only a real change.)
        if let Some(v) = view
            && !replay
            && self.view.is_none_or(|w| v.abs_diff(w) >= 2)
        {
            self.view = view;
            self.out.line(format_args!("{c} {k} {id} view {v}"));
        }
        for n in &ev.notes {
            if replay || self.quiet.fresh(k, id, n) {
                self.out.line(format_args!("{c} {k} {id} {n}"));
            }
        }
        if ev.knocked {
            self.out.line(format_args!(
                "{c} {k} {id} state {:?} v={}",
                b.state,
                v3(b.vel.x, b.vel.y, b.vel.z)
            ));
        }
        if beans::tackling(b) {
            self.out.line(format_args!(
                "{c} {k} {id} {:?} pos={} v={}",
                b.state,
                v3(b.pos.x, b.pos.y, b.pos.z),
                v3(b.vel.x, b.vel.y, b.vel.z)
            ));
            let cb = beans::Capsule::of_body(b);
            for o in extra.iter().filter(|o| m::hypot(o.x - b.pos.x, o.z - b.pos.z) <= 4.0) {
                self.out.line(format_args!(
                    "{c} {k} {id}   near {} drawn={} gap={:.2}",
                    o.id,
                    v3(o.x, o.y, o.z),
                    beans::gap(&cb, &beans::Capsule::of_other(o))
                ));
            }
        }
        self.out.flush();
    }
}
