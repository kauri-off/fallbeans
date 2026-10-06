//! The GPU's passes from Bevy's render diagnostics: GPU and CPU time, triangles, fragments.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use bevy::diagnostic::{Diagnostic, DiagnosticPath, DiagnosticsStore};
use serde::Serialize;

/// A pass that has not reported for this long is no longer drawn (its last value stays in the store).
const STALE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, Serialize)]
pub struct Pass {
    pub name: String,
    pub gpu: f32,
    pub cpu: f32,
    /// Triangles out of clipping.
    pub tris: f32,
    /// Fragment shader invocations.
    pub frags: f32,
}

/// The last value, if recent and a real one (a driver can report a negative GPU time).
fn fresh(d: &Diagnostic, now: Instant) -> Option<f32> {
    let m = d.measurement()?;
    let v = m.value as f32;
    (now.saturating_duration_since(m.time) < STALE && v.is_finite() && v >= 0.0).then_some(v)
}

/// `render/<top>/…/<field>`: the top-level pass, how deep, and the field.
fn split(path: &str) -> Option<(&str, usize, &str)> {
    let rest = path.strip_prefix("render/")?;
    let (head, field) = rest.rsplit_once('/')?;
    let top = head.split('/').next()?;
    Some((top, head.matches('/').count(), field))
}

/// The top-level passes' GPU times in the store, looked up again only when it has new paths (it never loses
/// one): the frame's sum reads them without parsing every path every frame.
#[derive(Default)]
pub struct FramePaths {
    seen: usize,
    gpu: Vec<DiagnosticPath>,
}

impl FramePaths {
    /// The GPU time of the frame: the top-level passes added up.
    pub fn frame_ms(&mut self, store: &DiagnosticsStore, now: Instant) -> f32 {
        let n = store.iter().count();
        if n != self.seen {
            self.seen = n;
            self.gpu = store
                .iter()
                .filter(|d| matches!(split(d.path().as_str()), Some((_, 0, "elapsed_gpu"))))
                .map(|d| d.path().clone())
                .collect();
        }
        let mut sum = 0.0;
        let mut any = false;
        for v in self.gpu.iter().filter_map(|p| fresh(store.get(p)?, now)) {
            sum += v;
            any = true;
        }
        if any { sum } else { f32::NAN }
    }
}

/// The passes drawn now, the costliest on the GPU (else the CPU) first.
pub fn passes(store: &DiagnosticsStore, now: Instant) -> Vec<Pass> {
    let mut by: BTreeMap<&str, Pass> = BTreeMap::new();
    for d in store.iter() {
        let Some((top, depth, field)) = split(d.path().as_str()) else {
            continue;
        };
        let Some(v) = fresh(d, now) else { continue };
        let p = by.entry(top).or_insert_with(|| Pass {
            name: top.to_string(),
            ..Pass::default()
        });
        match (field, depth) {
            ("elapsed_gpu", 0) => p.gpu = v,
            ("elapsed_cpu", 0) => p.cpu = v,
            ("clipper_primitives_out", _) => p.tris += v,
            ("fragment_shader_invocations", _) => p.frags += v,
            _ => {}
        }
    }
    let mut v: Vec<Pass> = by.into_values().collect();
    v.sort_by(|a, b| b.gpu.total_cmp(&a.gpu).then(b.cpu.total_cmp(&a.cpu)));
    v
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn paths() {
        assert_eq!(split("render/shadows/elapsed_gpu"), Some(("shadows", 0, "elapsed_gpu")));
        assert_eq!(
            split("render/fxaa/fxaa/fragment_shader_invocations"),
            Some(("fxaa", 1, "fragment_shader_invocations"))
        );
        assert_eq!(split("frame_time"), None);
    }
}
