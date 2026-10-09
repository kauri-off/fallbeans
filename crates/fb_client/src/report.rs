//! F8: what the player saw going wrong, in a file for whoever fixes it: the connection, the corrections of the
//! last minute, the own bean's last seconds tick by tick and the log's last lines.
use std::fmt::Write as _;
use std::io::Write as _;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fb_net::BodyFull;
use lightyear::prelude::*;

use crate::game::MapNow;
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

/// What a report is written from.
#[derive(SystemParam)]
struct Sources<'w, 's> {
    logs: Res<'w, Logs>,
    net: Res<'w, NetNow>,
    recent: Res<'w, Recent>,
    corrections: Res<'w, Corrections>,
    conn: Option<Res<'w, Conn>>,
    now: MapNow<'w>,
    own: Query<'w, 's, &'static BodyFull, With<Predicted>>,
    session: Res<'w, Session>,
}

fn report(
    keys: Res<ButtonInput<KeyCode>>,
    real: Res<Time<Real>>,
    time: Res<Time>,
    src: Sources,
    mut feed: ResMut<FeedLog>,
) {
    if !keys.just_pressed(KeyCode::F8) {
        return;
    }
    let now = real.elapsed_secs();
    warn!(
        "F8: the player marks a problem | {} | {}",
        src.net.transport,
        src.net.line(now)
    );
    let Some(dir) = &src.logs.0 else { return };
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
    let _ = writeln!(s, "server: {}", src.conn.as_ref().map_or("—", |c| c.http.as_str()));
    let _ = writeln!(s, "net: {} | {}", src.net.transport, src.net.line(now));
    let _ = writeln!(
        s,
        "room: {} as {} (practice: {})",
        src.session.room.as_deref().unwrap_or("—"),
        src.session.me.map_or("—".into(), |m| m.to_string()),
        src.session.practice
    );
    if let Some(m) = &src.now.map {
        let tick = src.now.timeline.tick().0;
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
    if let Ok(full) = src.own.single() {
        let b = &full.body;
        let _ = writeln!(
            s,
            "bean: pos {:.2} {:.2} {:.2} vel {:.2} {:.2} {:.2} {:?} grounded {} teleports {}",
            b.pos.x, b.pos.y, b.pos.z, b.vel.x, b.vel.y, b.vel.z, b.state, b.grounded, full.teleports
        );
    }
    let _ = writeln!(s, "\ncorrections, last minute (s ago, tick, m, x y z, ticks replayed):");
    for c in &src.corrections.list {
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
    for r in src.recent.0.iter().skip(src.recent.0.len().saturating_sub(TICKS)) {
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
