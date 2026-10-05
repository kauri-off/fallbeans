//! The own bean tick by tick: corrections (the server had it elsewhere, and a rollback moved it: the bean
//! snaps back) and its last seconds, for an F8 report.
use std::collections::VecDeque;

use bevy::prelude::*;
use fb_net::{BodyFull, FbInput};
use fb_shared::TICK_RATE;
use fb_shared::input::InputFrame;
use fb_sim::math::{V3, len};
use lightyear::input::native::prelude::ActionState;
use lightyear::prelude::*;

use crate::game::Map;
use crate::stats::NetNow;

/// Ticks of the own bean kept for a report (20 s).
const KEEP_TICKS: usize = 20 * TICK_RATE as usize;
/// Corrections kept for a report, s.
const KEEP_S: f32 = 60.0;
/// A correction this long (m) is counted; this long, logged.
const NOTICE_M: f64 = 0.05;
const LOG_M: f64 = 0.5;
/// Longer, or with a respawn, it was a teleport the client did not predict (a respawn, a portal).
const TELEPORT_M: f64 = 15.0;
/// Logged corrections closer than this (s) are one run of rubber-banding: its first `RUN_LINES` are logged,
/// then a summary.
const RUN_GAP_S: f32 = 3.0;
const RUN_LINES: u32 = 3;

/// The own bean at the end of a predicted tick.
pub struct Rec {
    pub tick: u32,
    pub arena: u32,
    pub frame: InputFrame,
    pub pos: V3,
    teleports: u32,
    /// How far the next tick found it moved by a rollback (m).
    pub corrected: f64,
}

#[derive(Resource, Default)]
pub struct Recent(pub VecDeque<Rec>);

pub struct Correction {
    /// Real time, s.
    pub at: f32,
    pub tick: u32,
    pub by: V3,
    /// Ticks replayed by the rollbacks since the tick before.
    pub replayed: u32,
}

#[derive(Resource, Default)]
pub struct Corrections {
    pub list: VecDeque<Correction>,
    /// Since the last `net:` line: how many, the longest (m).
    pub window: (u32, f64),
    run: Option<Run>,
    /// `PredictionMetrics::rollback_ticks` at the last tick recorded.
    replayed: u32,
}

#[derive(Default)]
struct Run {
    start: f32,
    last: f32,
    n: u32,
    logged: u32,
    max: f64,
    total: f64,
}

pub struct WatchPlugin;

impl Plugin for WatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Recent>();
        app.init_resource::<Corrections>();
        app.add_systems(FixedPreUpdate, check.run_if(not(resource_exists::<Rollback>)));
        app.add_systems(FixedPostUpdate, record.run_if(not(resource_exists::<Rollback>)));
        app.add_systems(Update, end_run);
    }
}

fn record(
    own: Query<(&BodyFull, &ActionState<FbInput>), With<Predicted>>,
    map: Option<Res<Map>>,
    timeline: Res<LocalTimeline>,
    metrics: Option<Res<PredictionMetrics>>,
    mut recent: ResMut<Recent>,
    mut c: ResMut<Corrections>,
) {
    let (Some(map), Ok((full, state))) = (map, own.single()) else {
        return;
    };
    if recent.0.len() == KEEP_TICKS {
        recent.0.pop_front();
    }
    recent.0.push_back(Rec {
        tick: timeline.tick().0,
        arena: map.round.arena,
        frame: InputFrame::from(state.0),
        pos: full.body.pos,
        teleports: full.teleports,
        corrected: 0.0,
    });
    c.replayed = metrics.map_or(0, |m| m.rollback_ticks);
}

/// Before a tick: where the bean is against where the last one left it (only a rollback moves it between).
fn check(
    own: Query<&BodyFull, With<Predicted>>,
    map: Option<Res<Map>>,
    timeline: Res<LocalTimeline>,
    time: Res<Time<Real>>,
    metrics: Option<Res<PredictionMetrics>>,
    net: Res<NetNow>,
    mut recent: ResMut<Recent>,
    mut c: ResMut<Corrections>,
) {
    let (Some(map), Ok(full)) = (map, own.single()) else {
        return;
    };
    let tick = timeline.tick().0;
    let Some(last) = recent.0.back_mut() else { return };
    if last.tick.wrapping_add(1) != tick || last.arena != map.round.arena {
        return;
    }
    let by = full.body.pos - last.pos;
    let d = len(by);
    if d < NOTICE_M {
        return;
    }
    last.corrected = d;
    let now = time.elapsed_secs();
    if last.teleports != full.teleports || d >= TELEPORT_M {
        let p = full.body.pos;
        warn!(
            "the server put the bean elsewhere: {d:.1} m to {:.1} {:.1} {:.1} at tick {tick} (respawns {} → {}) | {}",
            p.x,
            p.y,
            p.z,
            last.teleports,
            full.teleports,
            net.line(now)
        );
        return;
    }
    let replayed = metrics.map_or(0, |m| m.rollback_ticks).wrapping_sub(c.replayed);
    c.window = (c.window.0 + 1, c.window.1.max(d));
    while c.list.front().is_some_and(|x| now - x.at > KEEP_S) {
        c.list.pop_front();
    }
    c.list.push_back(Correction {
        at: now,
        tick,
        by,
        replayed,
    });
    if d < LOG_M {
        return;
    }
    if c.run.as_ref().is_some_and(|r| now - r.last > RUN_GAP_S) {
        end(&mut c.run);
    }
    let run = c.run.get_or_insert_with(|| Run {
        start: now,
        ..default()
    });
    run.n += 1;
    run.last = now;
    run.max = run.max.max(d);
    run.total += d;
    if run.logged < RUN_LINES {
        run.logged += 1;
        warn!(
            "correction {d:.2} m (x {:+.2} y {:+.2} z {:+.2}) at tick {tick}, {replayed} ticks replayed | {}",
            by.x,
            by.y,
            by.z,
            net.line(now)
        );
    }
}

fn end_run(time: Res<Time<Real>>, mut c: ResMut<Corrections>) {
    if c.run.as_ref().is_some_and(|r| time.elapsed_secs() - r.last > RUN_GAP_S) {
        end(&mut c.run);
    }
}

fn end(run: &mut Option<Run>) {
    let Some(r) = run.take() else { return };
    if r.n > r.logged {
        warn!(
            "rubber-banding: {} corrections over {:.1} s, the longest {:.2} m, {:.1} m in all",
            r.n,
            r.last - r.start,
            r.max,
            r.total
        );
    }
}
