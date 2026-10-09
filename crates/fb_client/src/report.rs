//! F8: what the player saw going wrong, in a file for whoever fixes it: the connection, the corrections of the
//! last minute, the own bean's last seconds tick by tick and the log's last lines.
use std::fmt::Write as _;
use std::io::Write as _;

use bevy::prelude::*;
use fb_net::BodyFull;
use lightyear::prelude::*;

use crate::game::Map;
use crate::logs::{self, Logs};
use crate::net::Conn;
use crate::session::{FeedLog, Session};
use crate::stats::NetNow;
use crate::ui::text;
use crate::watch::{Corrections, Recent};

/// The own bean's ticks in a report (10 s).
const TICKS: usize = 10 * fb_shared::TICK_RATE as usize;
const LOG_LINES: usize = 300;

pub struct ReportPlugin;

impl Plugin for ReportPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, report);
    }
}

fn report(
    keys: Res<ButtonInput<KeyCode>>,
    logs: Res<Logs>,
    real: Res<Time<Real>>,
    time: Res<Time>,
    net: Res<NetNow>,
    recent: Res<Recent>,
    corrections: Res<Corrections>,
    conn: Option<Res<Conn>>,
    map: Option<Res<Map>>,
    timeline: Res<LocalTimeline>,
    own: Query<&BodyFull, With<Predicted>>,
    session: Res<Session>,
    mut feed: ResMut<FeedLog>,
) {
    if !keys.just_pressed(KeyCode::F8) {
        return;
    }
    let now = real.elapsed_secs();
    warn!("F8: the player marks a problem | {} | {}", net.transport, net.line(now));
    let Some(dir) = &logs.0 else { return };
    let Some((mut file, path)) = logs::create(dir, "report", "txt") else {
        warn!("F8 report: cannot create a file in {}", dir.display());
        feed.note(time.elapsed_secs(), text::REPORT_FAILED.to_string());
        return;
    };
    let mut s = String::new();
    let _ = writeln!(s, "Fall Beans {} report (F8)", fb_net::build());
    let _ = writeln!(s, "time: {} UTC", logs::stamp(logs::now_secs()));
    let _ = writeln!(s, "os: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    if let Some(log) = logs::file() {
        let _ = writeln!(s, "log: {}", log.display());
    }
    let _ = writeln!(s, "server: {}", conn.as_ref().map_or("—", |c| c.http.as_str()));
    let _ = writeln!(s, "net: {} | {}", net.transport, net.line(now));
    let _ = writeln!(
        s,
        "room: {} as {} (practice: {})",
        session.room.as_deref().unwrap_or("—"),
        session.me.map_or("—".into(), |m| m.to_string()),
        session.practice
    );
    if let Some(m) = &map {
        let tick = timeline.tick().0;
        let _ = writeln!(
            s,
            "arena {}: {} ({:?}) seed {} | tick {tick} t {:.2} s",
            m.round.arena,
            m.round.map,
            m.round.kind,
            m.round.seed,
            m.time(f64::from(tick))
        );
    }
    if let Ok(full) = own.single() {
        let b = &full.body;
        let _ = writeln!(
            s,
            "bean: pos {:.2} {:.2} {:.2} vel {:.2} {:.2} {:.2} {:?} grounded {} teleports {}",
            b.pos.x, b.pos.y, b.pos.z, b.vel.x, b.vel.y, b.vel.z, b.state, b.grounded, full.teleports
        );
    }
    let _ = writeln!(s, "\ncorrections, last minute (s ago, tick, m, x y z, ticks replayed):");
    for c in &corrections.list {
        let _ = writeln!(
            s,
            "{:6.1} {} {:.2} {:+.2} {:+.2} {:+.2} {}",
            now - c.at,
            c.tick,
            c.by.length(),
            c.by.x,
            c.by.y,
            c.by.z,
            c.replayed
        );
    }
    let _ = writeln!(
        s,
        "\nown bean, last {} s (tick arena mx mz buttons x y z, then the correction after it):",
        TICKS / fb_shared::TICK_RATE as usize
    );
    for r in recent.0.iter().skip(recent.0.len().saturating_sub(TICKS)) {
        let _ = write!(
            s,
            "{} {} {} {} {} {:.3} {:.3} {:.3}",
            r.tick, r.arena, r.frame.mx, r.frame.mz, r.frame.buttons, r.pos.x, r.pos.y, r.pos.z
        );
        let _ = if r.corrected > 0.0 {
            writeln!(s, " * {:.2}", r.corrected)
        } else {
            writeln!(s)
        };
    }
    let _ = writeln!(s, "\nlog (last {LOG_LINES} lines):");
    for l in fb_net::logbook::tail(LOG_LINES) {
        let _ = writeln!(s, "{}", crate::crash::line(&l));
    }
    // (Sent to whoever fixes it: without the player's home folder, the user name in it.)
    match file.write_all(crate::crash::redact(&s).as_bytes()) {
        Ok(()) => {
            info!("F8 report: {}", path.display());
            feed.note(time.elapsed_secs(), text::report_saved(&path.display().to_string()));
        }
        Err(e) => {
            warn!("F8 report: {}: {e}", path.display());
            feed.note(time.elapsed_secs(), text::REPORT_FAILED.to_string());
        }
    }
}
