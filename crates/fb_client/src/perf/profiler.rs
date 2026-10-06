//! Built-in CPU profiler: Bevy's system and schedule spans summed per frame (feature `profiler`). The spans of
//! systems and schedules exist only with `--profiler` (`main.rs` filters them out otherwise); `present_frames`
//! is always there and always summed (the swapchain wait).
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

/// Every span is summed (else only `present_frames`).
static ON: AtomicBool = AtomicBool::new(false);
/// When `ON` last went on (`now_ns`): a run entered before it (or while off) is not summed.
static ON_SINCE: AtomicU64 = AtomicU64::new(0);
/// Id of the latest `present_frames` span: summed while off too.
static ALWAYS: AtomicU64 = AtomicU64::new(0);
static EPOCH: OnceLock<Instant> = OnceLock::new();
/// By kind and full name: two systems may have one short name.
static SLOTS: Mutex<BTreeMap<(Kind, String), Arc<Slot>>> = Mutex::new(BTreeMap::new());
/// `present_frames`, summed whether the profiler is on or not.
static PRESENT: OnceLock<Arc<Slot>> = OnceLock::new();

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

/// What the runs of one name added up to. Systems of the same name (a closure of one function in two
/// worlds) share it: one row, their runs added.
pub struct Slot {
    pub kind: Kind,
    /// Short (`module::function`).
    pub name: String,
    /// Summed whether the profiler is on or not.
    pub always: bool,
    total_ns: AtomicU64,
    calls: AtomicU32,
}

impl Slot {
    fn new(kind: Kind, name: String, always: bool) -> Arc<Slot> {
        Arc::new(Slot {
            kind,
            name,
            always,
            total_ns: AtomicU64::new(0),
            calls: AtomicU32::new(0),
        })
    }
}

/// What a slot took since the last call (ms, runs), if it ran.
fn drain(s: &Arc<Slot>) -> Option<(Arc<Slot>, f32, u32)> {
    let ns = s.total_ns.swap(0, Ordering::Relaxed);
    let calls = s.calls.swap(0, Ordering::Relaxed);
    (calls > 0).then(|| (s.clone(), ns as f32 / 1e6, calls))
}

/// A span's own start: two spans of one slot can run at once (on the main and the render thread).
struct Timed {
    slot: Arc<Slot>,
    start_ns: AtomicU64,
}

pub fn set_on(on: bool) {
    if !on {
        ON.store(false, Ordering::Relaxed);
        return;
    }
    if ON.load(Ordering::Relaxed) {
        return;
    }
    // (What the slots kept from the last time on is stale.)
    if let Ok(slots) = SLOTS.lock() {
        for s in slots.values() {
            s.total_ns.store(0, Ordering::Relaxed);
            s.calls.store(0, Ordering::Relaxed);
        }
    }
    ON_SINCE.store(now_ns(), Ordering::Relaxed);
    ON.store(true, Ordering::Release);
}

pub fn is_on() -> bool {
    ON.load(Ordering::Acquire)
}

/// The profiler was built in (else the overlay says how to get it).
pub const BUILT: bool = cfg!(feature = "profiler");

fn now_ns() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64 + 1
}

/// What each span took since the last call (ms, calls), the slots with nothing left out. While off only
/// `present_frames` is read.
pub fn take() -> Vec<(Arc<Slot>, f32, u32)> {
    let mut out: Vec<_> = PRESENT.get().and_then(drain).into_iter().collect();
    if is_on()
        && let Ok(slots) = SLOTS.lock()
    {
        out.extend(slots.values().filter_map(drain));
    }
    out
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

fn kind_of(meta: &Metadata<'_>) -> Option<Kind> {
    Some(match meta.name() {
        "system" => Kind::System,
        "system_commands" => Kind::Commands,
        "schedule" => Kind::Schedule,
        "main app" | "sub app" | "present_frames" => Kind::Frame,
        _ => return None,
    })
}

fn slot(kind: Kind, full: String) -> Option<Arc<Slot>> {
    let mut slots = SLOTS.lock().ok()?;
    let s = slots.entry((kind, full)).or_insert_with_key(|(kind, full)| {
        let name = if *kind == Kind::Frame {
            full.clone()
        } else {
            short(full)
        };
        Slot::new(*kind, name, false)
    });
    Some(s.clone())
}

fn passed_over(id: &Id) -> bool {
    !is_on() && ALWAYS.load(Ordering::Relaxed) != id.into_u64()
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Profiler {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let meta = attrs.metadata();
        let Some(kind) = kind_of(meta) else {
            return;
        };
        let always = meta.name() == "present_frames";
        // (Schedule spans are made every run, a system's once: only the latter must be caught while off.)
        if matches!(kind, Kind::Schedule | Kind::Frame) && !always && !is_on() {
            return;
        }
        let slot = if always {
            PRESENT
                .get_or_init(|| Slot::new(Kind::Frame, "present_frames".into(), true))
                .clone()
        } else {
            let mut name = Name::default();
            attrs.record(&mut name);
            let full = if name.0.is_empty() {
                meta.name().to_string()
            } else {
                name.0
            };
            match slot(kind, full) {
                Some(s) => s,
                None => return,
            }
        };
        let Some(span) = ctx.span(id) else { return };
        if always {
            ALWAYS.store(id.into_u64(), Ordering::Relaxed);
        }
        span.extensions_mut().insert(Timed {
            slot,
            start_ns: AtomicU64::new(0),
        });
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if passed_over(id) {
            return;
        }
        let Some(span) = ctx.span(id) else { return };
        if let Some(t) = span.extensions().get::<Timed>() {
            t.start_ns.store(now_ns(), Ordering::Relaxed);
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if passed_over(id) {
            return;
        }
        let Some(span) = ctx.span(id) else { return };
        let ext = span.extensions();
        let Some(t) = ext.get::<Timed>() else { return };
        let start = t.start_ns.swap(0, Ordering::Relaxed);
        // (Entered while off, or before the profiler last went on: not a whole run, or a stale start.)
        if start == 0 || (!t.slot.always && start < ON_SINCE.load(Ordering::Relaxed)) {
            return;
        }
        t.slot
            .total_ns
            .fetch_add(now_ns().saturating_sub(start), Ordering::Relaxed);
        t.slot.calls.fetch_add(1, Ordering::Relaxed);
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
