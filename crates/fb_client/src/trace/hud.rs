//! `--trace hud`: the results and summary boards frame by frame while one is up: each rebuild and its cause (what
//! changed in the session), the board's motion and size, and the time line under its title: `f<frame> …` lines,
//! written only when something differs from the frame before.
use std::collections::BTreeMap;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::ui::UiSystems;
use fb_net::trace::TraceFile;

use crate::session::{Outcome, Session};
use crate::ui::hud::{NextLine, ResultsBox, SummaryBox};
use crate::ui::motion::Motion;

pub fn add(app: &mut App, out: TraceFile) {
    app.insert_resource(Trace {
        out,
        session: BTreeMap::new(),
        last: String::new(),
        up: false,
    });
    app.add_systems(PostUpdate, trace.after(UiSystems::Layout));
}

#[derive(Resource)]
struct Trace {
    out: TraceFile,
    /// The session's fields as last seen, by name.
    session: BTreeMap<&'static str, String>,
    /// The boards' state line of the frame before.
    last: String,
    up: bool,
}

fn fields(s: &Session) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("dev", format!("{:?}", s.dev)),
        ("me", format!("{:?}", s.me)),
        ("room", format!("{:?}", s.room)),
        ("lobby", format!("{:?}", s.lobby)),
        ("arena", format!("{:?}", s.arena)),
        ("scores", format!("{:?}", s.scores)),
        ("refused", format!("{:?}", s.refused)),
        ("denied", format!("{:?}", s.denied)),
        ("reject", format!("{:?}", s.reject)),
        ("practice", format!("{:?}", s.practice)),
        ("back_to", format!("{:?}", s.back_to)),
        ("entries", format!("{:?}", s.entries)),
    ])
}

fn short(s: &str) -> String {
    let mut out: String = s.chars().take(400).collect();
    if s.chars().count() > 400 {
        out += "…";
    }
    out
}

type Boards<'w, 's> = Query<'w, 's, (Entity, Has<ResultsBox>), Or<(With<ResultsBox>, With<SummaryBox>)>>;

#[expect(clippy::too_many_arguments, reason = "a trace reads much of the interface")]
fn trace(
    frames: Res<FrameCount>,
    time: Res<Time<Real>>,
    session: Res<Session>,
    outcome: Res<Outcome>,
    boxes: Boards,
    children: Query<&Children>,
    nodes: Query<(&ComputedNode, &UiTransform, Has<Motion>)>,
    lines: Query<(Entity, &crate::ui::Rich), With<NextLine>>,
    texts: Query<(Option<&Text>, Option<&TextSpan>)>,
    mut t: ResMut<Trace>,
) {
    let up = outcome.results.is_some() || outcome.game_end.is_some();
    let f = frames.0;
    let dt = time.delta_secs() * 1000.0;
    let now = fields(&session);
    if session.is_changed() && (up || t.up) {
        let changed: Vec<String> = now
            .iter()
            .filter(|(k, v)| t.session.get(*k) != Some(*v))
            .map(|(k, v)| format!("{k} = {}", short(v)))
            .collect();
        let what = if changed.is_empty() {
            "nothing (marked changed by a write of the same value)".to_string()
        } else {
            changed.join("; ")
        };
        t.out.line(format_args!("f{f} ({dt:.1} ms) Session changed: {what}"));
    }
    t.session = now;
    if outcome.is_changed() && (up || t.up) {
        t.out.line(format_args!(
            "f{f} ({dt:.1} ms) Outcome changed: results {}, game_end {}",
            outcome
                .results
                .as_ref()
                .map_or("none".into(), |r| format!("round {}/{}", r.index, r.total)),
            if outcome.game_end.is_some() { "some" } else { "none" },
        ));
    }
    if !up && !t.up {
        return;
    }
    t.up = up;
    let mut state = Vec::new();
    for (e, results) in &boxes {
        let name = if results { "results" } else { "summary" };
        let Some(board) = children.get(e).ok().and_then(|c| c.last().copied()) else {
            state.push(format!("{name}: empty"));
            continue;
        };
        let mut s = format!("{name}: board {board}");
        if let Ok((n, tf, motion)) = nodes.get(board) {
            let size = n.size() * n.inverse_scale_factor();
            s += &format!(
                " {:.0}×{:.0} at {:?}{}",
                size.x,
                size.y,
                tf.translation,
                if motion { " MOTION" } else { "" }
            );
        } else {
            s += " (not laid out yet)";
        }
        let moving = children
            .iter_descendants(board)
            .filter(|c| nodes.get(*c).is_ok_and(|n| n.2))
            .count();
        if moving > 0 {
            s += &format!(", {moving} children in motion");
        }
        for (l, rich) in &lines {
            if !children.iter_descendants(board).any(|c| c == l) {
                continue;
            }
            let shown: String = std::iter::once(l)
                .chain(children.iter_descendants(l))
                .filter_map(|c| texts.get(c).ok())
                .map(|(t, sp)| format!("{}{}", t.map_or("", |t| &t.0), sp.map_or("", |s| &s.0)))
                .collect();
            let size = nodes.get(l).map_or("not laid out".into(), |(n, ..)| {
                let z = n.size() * n.inverse_scale_factor();
                format!("{:.0}×{:.0}", z.x, z.y)
            });
            s += &format!(", time line {l} Rich «{}» shown «{shown}» {size}", rich.0);
        }
        state.push(s);
    }
    let state = state.join(" | ");
    if state != t.last {
        t.out.line(format_args!("f{f} ({dt:.1} ms) {state}"));
        t.last = state;
    }
}
