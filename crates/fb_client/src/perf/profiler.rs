//! Built-in CPU profiler: Bevy's system and schedule spans summed per frame (feature `profiler`).
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bevy::log::BoxedLayer;
use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::span::{Attributes, Id};
use bevy::log::tracing::{Metadata, Subscriber};
use bevy::log::tracing_subscriber::Layer;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::tracing_subscriber::registry::LookupSpan;

/// Every span is summed (else only the waits).
static ON: AtomicBool = AtomicBool::new(false);
static EPOCH: OnceLock<Instant> = OnceLock::new();
static SLOTS: Mutex<BTreeMap<(Kind, String), Arc<Slot>>> = Mutex::new(BTreeMap::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    System,
    /// A system's commands applied.
    Commands,
    Schedule,
    /// The whole main world, the render world, the swapchain.
    Frame,
}

pub struct Slot {
    pub kind: Kind,
    pub name: String,
    /// Summed whether the profiler is on or not.
    pub always: bool,
    total_ns: AtomicU64,
    calls: AtomicU32,
    start_ns: AtomicU64,
}

pub fn set_on(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn is_on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The profiler was built in (else the overlay says how to get it).
pub const BUILT: bool = cfg!(feature = "profiler");

fn now_ns() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64 + 1
}

/// What each span took since the last call (ms, calls), the slots with nothing left out.
pub fn take() -> Vec<(Arc<Slot>, f32, u32)> {
    let Ok(slots) = SLOTS.lock() else { return Vec::new() };
    slots
        .values()
        .filter_map(|s| {
            let ns = s.total_ns.swap(0, Ordering::Relaxed);
            let calls = s.calls.swap(0, Ordering::Relaxed);
            (calls > 0).then(|| (s.clone(), ns as f32 / 1e6, calls))
        })
        .collect()
}

/// `crate::module::function<other::Type>` → `module::function<other::Type>`.
pub fn short(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        let parts: Vec<&str> = word.split("::").collect();
        let keep = if parts.last() == Some(&"{{closure}}") { 3 } else { 2 };
        out.push_str(&parts[parts.len().saturating_sub(keep)..].join("::"));
        word.clear();
    };
    for c in name.chars() {
        if matches!(c, '<' | '>' | ',' | ' ' | '(' | ')' | '&' | '[' | ']' | ';') {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// For `LogPlugin::custom_layer`.
pub fn layer() -> BoxedLayer {
    Box::new(Profiler)
}

struct Profiler;

#[derive(Default)]
struct Name(String);

impl Visit for Name {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "name" {
            self.0 = value.into();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        if field.name() == "name" {
            self.0 = format!("{value:?}");
        }
    }
}

fn kind_of(meta: &Metadata<'_>) -> Option<(Kind, bool)> {
    Some(match meta.name() {
        "system" => (Kind::System, false),
        "system_commands" => (Kind::Commands, false),
        "schedule" => (Kind::Schedule, false),
        "main app" | "sub app" => (Kind::Frame, false),
        "present_frames" => (Kind::Frame, true),
        _ => return None,
    })
}

fn slot(kind: Kind, name: String, always: bool) -> Option<Arc<Slot>> {
    let mut slots = SLOTS.lock().ok()?;
    let s = slots.entry((kind, name.clone())).or_insert_with(|| {
        Arc::new(Slot {
            kind,
            name,
            always,
            total_ns: AtomicU64::new(0),
            calls: AtomicU32::new(0),
            start_ns: AtomicU64::new(0),
        })
    });
    Some(s.clone())
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Profiler {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let meta = attrs.metadata();
        let Some((kind, mut always)) = kind_of(meta) else {
            return;
        };
        // (Schedule spans are made every run, a system's once: only the latter must be caught while off.)
        if matches!(kind, Kind::Schedule | Kind::Frame) && !always && !is_on() {
            return;
        }
        let mut name = Name::default();
        attrs.record(&mut name);
        let name = match kind {
            Kind::Frame if name.0.is_empty() => meta.name().to_string(),
            Kind::Frame => name.0,
            _ => short(&name.0),
        };
        always |= kind == Kind::System && name.ends_with("::prepare_windows");
        let (Some(slot), Some(span)) = (slot(kind, name, always), ctx.span(id)) else {
            return;
        };
        span.extensions_mut().insert(slot);
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let ext = span.extensions();
        if let Some(s) = ext.get::<Arc<Slot>>()
            && (s.always || is_on())
        {
            s.start_ns.store(now_ns(), Ordering::Relaxed);
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let ext = span.extensions();
        if let Some(s) = ext.get::<Arc<Slot>>() {
            let start = s.start_ns.swap(0, Ordering::Relaxed);
            if start != 0 {
                s.total_ns.fetch_add(now_ns().saturating_sub(start), Ordering::Relaxed);
                s.calls.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::short;

    #[test]
    fn short_names() {
        assert_eq!(short("fb_client::stats::watch"), "stats::watch");
        assert_eq!(
            short("bevy_render::extract_component::extract_components<fb_client::render::fsr::Fsr>"),
            "extract_component::extract_components<fsr::Fsr>"
        );
        assert_eq!(
            short("fb_client::diag::DiagPlugin::{{closure}}"),
            "diag::DiagPlugin::{{closure}}"
        );
        assert_eq!(short("Update"), "Update");
    }
}
