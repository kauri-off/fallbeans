//! The last log lines in memory, for `/api/debug/logs` (a tracing layer next to Bevy's console output).
use std::collections::VecDeque;
use std::fmt::Write;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use bevy::log::BoxedLayer;
use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::{Event, Level, Subscriber};
use bevy::log::tracing_subscriber::Layer;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::prelude::App;
use serde::Serialize;

const LINES: usize = 2000;

#[derive(Serialize, Clone, Debug)]
pub struct LogLine {
    /// Milliseconds since the epoch.
    pub at: u64,
    pub level: &'static str,
    pub target: String,
    pub msg: String,
}

fn book() -> &'static Mutex<VecDeque<LogLine>> {
    static BOOK: OnceLock<Mutex<VecDeque<LogLine>>> = OnceLock::new();
    BOOK.get_or_init(|| Mutex::new(VecDeque::with_capacity(LINES)))
}

/// The newest `n` lines at `level` (None: every level), oldest first.
pub fn recent(level: Option<&str>, n: usize) -> Vec<LogLine> {
    let book = book().lock().unwrap_or_else(|e| e.into_inner());
    let mut lines: Vec<LogLine> = book
        .iter()
        .rev()
        .filter(|l| level.is_none_or(|w| l.level.eq_ignore_ascii_case(w)))
        .take(n)
        .cloned()
        .collect();
    lines.reverse();
    lines
}

pub fn warnings() -> usize {
    let book = book().lock().unwrap_or_else(|e| e.into_inner());
    book.iter().filter(|l| l.level == "WARN").count()
}

/// For `LogPlugin::custom_layer`.
pub fn layer(_: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(Logbook))
}

struct Logbook;

/// The message, the other fields, and the target of a line from the `log` crate.
#[derive(Default)]
struct Fields(String, String, Option<String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        match field.name() {
            "message" => drop(write!(self.0, "{value:?}")),
            n if n.starts_with("log.") => {}
            n => drop(write!(self.1, " {n}={value:?}")),
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.0.push_str(value),
            "log.target" => self.2 = Some(value.into()),
            n if n.starts_with("log.") => {}
            n => drop(write!(self.1, " {n}={value}")),
        }
    }
}

impl<S: Subscriber> Layer<S> for Logbook {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() > Level::INFO {
            return;
        }
        let mut f = Fields::default();
        event.record(&mut f);
        let at = SystemTime::UNIX_EPOCH.elapsed().unwrap_or_default().as_millis() as u64;
        let line = LogLine {
            at,
            level: meta.level().as_str(),
            target: f.2.unwrap_or_else(|| meta.target().to_string()),
            msg: f.0 + &f.1,
        };
        let mut book = book().lock().unwrap_or_else(|e| e.into_inner());
        if book.len() == LINES {
            book.pop_front();
        }
        book.push_back(line);
    }
}
