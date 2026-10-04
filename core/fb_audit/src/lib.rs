//! Audits (port of `src/audit`): the game's content and systems checked without a client. Maps (rules,
//! spawns, clipping, reachability, balance), physics feel, determinism, input handling and budgets.
//! Each audit returns findings (problems, by severity) and metrics (numbers worth tracking). Run them
//! with `cargo xtask audit`, or the quick subset from tests.
use std::panic::{AssertUnwindSafe, catch_unwind};

use fb_shared::m;
use fb_sim::map::MapDef;
use fb_sim::math::V3;
use rayon::prelude::*;
use serde::{Serialize, Serializer};
use serde_json::Value;

pub mod harness;
pub mod maps;
pub mod systems;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warn,
    Info,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub msg: String,
    /// World position the finding is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 3]>,
    /// Sim time (s).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Finding {
    pub fn at(&mut self, p: V3) -> &mut Self {
        self.at = Some(v3(p));
        self
    }

    pub fn t(&mut self, t: f64) -> &mut Self {
        self.t = Some(t);
        self
    }

    pub fn data(&mut self, d: Value) -> &mut Self {
        self.data = Some(d);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Metric {
    Num(f64),
    Str(String),
    Bool(bool),
}

impl std::fmt::Display for Metric {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Metric::Num(v) => write!(f, "{v}"),
            Metric::Str(s) => f.write_str(s),
            Metric::Bool(b) => write!(f, "{b}"),
        }
    }
}

macro_rules! metric_from_num {
    ($($t:ty),*) => {
        $(impl From<$t> for Metric {
            fn from(v: $t) -> Self {
                Metric::Num(v as f64)
            }
        })*
    };
}
metric_from_num!(f64, i32, u32, i64, u64, usize);

impl From<bool> for Metric {
    fn from(v: bool) -> Self {
        Metric::Bool(v)
    }
}

impl From<String> for Metric {
    fn from(v: String) -> Self {
        Metric::Str(v)
    }
}

impl From<&str> for Metric {
    fn from(v: &str) -> Self {
        Metric::Str(v.to_string())
    }
}

/// What an audit reports into.
#[derive(Default)]
pub struct Out {
    pub findings: Vec<Finding>,
    pub metrics: Vec<(String, Metric)>,
}

impl Out {
    pub fn add(&mut self, severity: Severity, msg: impl Into<String>) -> &mut Finding {
        self.findings.push(Finding {
            severity,
            msg: msg.into(),
            at: None,
            t: None,
            data: None,
        });
        self.findings.last_mut().unwrap()
    }

    pub fn error(&mut self, msg: impl Into<String>) -> &mut Finding {
        self.add(Severity::Error, msg)
    }

    pub fn warn(&mut self, msg: impl Into<String>) -> &mut Finding {
        self.add(Severity::Warn, msg)
    }

    pub fn info(&mut self, msg: impl Into<String>) -> &mut Finding {
        self.add(Severity::Info, msg)
    }

    pub fn metric(&mut self, name: &str, v: impl Into<Metric>) {
        let v = v.into();
        match self.metrics.iter_mut().find(|(k, _)| k == name) {
            Some(m) => m.1 = v,
            None => self.metrics.push((name.to_string(), v)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// Quick mode: fewer seeds, shorter runs, coarser sampling (tests).
    pub quick: bool,
    pub seed: u32,
}

#[derive(Clone, Copy)]
pub enum Run {
    /// Runs once per map.
    Map(fn(&'static dyn MapDef, &Ctx, &mut Out)),
    Global(fn(&Ctx, &mut Out)),
}

#[derive(Clone, Copy)]
pub struct Audit {
    pub name: &'static str,
    pub run: Run,
    /// Measures wall time: run alone, after the others, so that they do not compete for the CPU.
    pub timed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditResult {
    pub audit: &'static str,
    /// Map id, or '*' for audits that are not about one map.
    pub map: String,
    pub ms: u64,
    pub findings: Vec<Finding>,
    #[serde(serialize_with = "as_map")]
    pub metrics: Vec<(String, Metric)>,
}

fn as_map<S: Serializer>(v: &[(String, Metric)], s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(v.iter().map(|(k, m)| (k, m)))
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub audits: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub build: String,
    pub quick: bool,
    pub ms: u64,
    pub summary: Summary,
    pub results: Vec<AuditResult>,
}

/// Audits that are slow (bots play whole rounds): only in full runs unless asked for by name.
const SLOW: [&str; 1] = ["balance"];

pub fn all_audits() -> Vec<Audit> {
    systems::AUDITS.iter().chain(maps::AUDITS.iter()).copied().collect()
}

pub fn audit_names() -> Vec<&'static str> {
    all_audits().iter().map(|a| a.name).collect()
}

#[derive(Clone, Debug, Default)]
pub struct RunOpts {
    pub maps: Vec<String>,
    pub only: Vec<String>,
    pub skip: Vec<String>,
    pub quick: bool,
    pub seed: Option<u32>,
}

fn panic_text(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| e.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

fn run_one(a: &Audit, map: Option<&'static dyn MapDef>, ctx: &Ctx) -> AuditResult {
    let mut out = Out::default();
    let clock = clock::Clock::start();
    let res = catch_unwind(AssertUnwindSafe(|| match (a.run, map) {
        (Run::Map(f), Some(m)) => f(m, ctx, &mut out),
        (Run::Global(f), _) => f(ctx, &mut out),
        (Run::Map(_), None) => unreachable!("a map audit without a map"),
    }));
    if let Err(e) = res {
        out.error(format!("audit crashed: {}", panic_text(&*e)));
    }
    AuditResult {
        audit: a.name,
        map: map.map_or("*".into(), |m| m.meta().id.to_string()),
        ms: m::round_js(clock.ms()) as u64,
        findings: out.findings,
        metrics: out.metrics,
    }
}

/// Runs the audits (map audits for every game, or the ones in `o.maps`) on all cores; the results come
/// in a fixed order: global audits, then map by map.
pub fn run_audits(o: &RunOpts, on_result: Option<&(dyn Fn(&AuditResult) + Sync)>) -> Report {
    let ctx = Ctx {
        quick: o.quick,
        seed: o.seed.unwrap_or(11),
    };
    let picked = |a: &Audit| {
        let on = if o.only.is_empty() {
            !(o.quick && SLOW.contains(&a.name))
        } else {
            o.only.iter().any(|n| n == a.name)
        };
        on && !o.skip.iter().any(|n| n == a.name)
    };
    let clock = clock::Clock::start();
    let mut results = Vec::new();
    for id in &o.maps {
        if fb_maps::GAMES.iter().all(|m| m.meta().id != id) {
            results.push(AuditResult {
                audit: "maps",
                map: id.clone(),
                ms: 0,
                findings: vec![Finding {
                    severity: Severity::Error,
                    msg: format!("unknown map {id}"),
                    at: None,
                    t: None,
                    data: None,
                }],
                metrics: Vec::new(),
            });
        }
    }
    let maps: Vec<&'static dyn MapDef> = fb_maps::GAMES
        .iter()
        .copied()
        .filter(|m| o.maps.is_empty() || o.maps.iter().any(|id| id == m.meta().id))
        .collect();
    let mut jobs: Vec<(Audit, Option<&'static dyn MapDef>)> = Vec::new();
    for a in systems::AUDITS.iter().filter(|a| picked(a)) {
        jobs.push((*a, None));
    }
    for &map in &maps {
        for a in maps::AUDITS.iter().filter(|a| picked(a)) {
            jobs.push((*a, Some(map)));
        }
    }
    let run = |(a, map): &(Audit, Option<&'static dyn MapDef>)| {
        let r = run_one(a, *map, &ctx);
        if let Some(f) = on_result {
            f(&r);
        }
        r
    };
    let mut done: Vec<Option<AuditResult>> = jobs
        .par_iter()
        .map(|j| if j.0.timed { None } else { Some(run(j)) })
        .collect();
    for (slot, j) in done.iter_mut().zip(&jobs) {
        if slot.is_none() {
            *slot = Some(run(j));
        }
    }
    results.extend(done.into_iter().flatten());
    let count = |s: Severity| {
        results
            .iter()
            .flat_map(|r| &r.findings)
            .filter(|f| f.severity == s)
            .count()
    };
    let summary = Summary {
        errors: count(Severity::Error),
        warnings: count(Severity::Warn),
        infos: count(Severity::Info),
        audits: results.len(),
    };
    Report {
        build: std::env::var("FB_BUILD").unwrap_or_else(|_| "local".into()),
        quick: o.quick,
        ms: m::round_js(clock.ms()) as u64,
        summary,
        results,
    }
}

/// Plain-text report: one line per finding, grouped by map and audit, plus key metrics.
pub fn format_report(r: &Report, metrics: bool, infos: bool) -> String {
    let mut lines = Vec::new();
    for res in &r.results {
        let shown: Vec<&Finding> = res
            .findings
            .iter()
            .filter(|f| infos || f.severity != Severity::Info)
            .collect();
        if shown.is_empty() && !(metrics && !res.metrics.is_empty()) {
            continue;
        }
        let map = if res.map == "*" { "[global]" } else { &res.map };
        lines.push(format!("{map} / {} ({} ms)", res.audit, res.ms));
        for f in shown {
            let icon = match f.severity {
                Severity::Error => '✖',
                Severity::Warn => '⚠',
                Severity::Info => '·',
            };
            let mut where_ = Vec::new();
            if let Some(t) = f.t {
                where_.push(format!("t={t}"));
            }
            if let Some([x, y, z]) = f.at {
                where_.push(format!("at {x} {y} {z}"));
            }
            let where_ = if where_.is_empty() {
                String::new()
            } else {
                format!("  [{}]", where_.join(" "))
            };
            lines.push(format!("  {icon} {}{where_}", f.msg));
        }
        if metrics && !res.metrics.is_empty() {
            let m: Vec<String> = res.metrics.iter().map(|(k, v)| format!("{k}={v}")).collect();
            lines.push(format!("    {}", m.join("  ")));
        }
    }
    let s = &r.summary;
    lines.push(format!(
        "{} errors, {} warnings, {} notes · {} audits in {:.1} s{}",
        s.errors,
        s.warnings,
        s.infos,
        s.audits,
        r.ms as f64 / 1000.0,
        if r.quick { " (quick)" } else { "" }
    ));
    lines.join("\n")
}

pub fn r3(v: f64) -> f64 {
    m::round_js(v * 1000.0) / 1000.0
}

pub fn r1(v: f64) -> f64 {
    m::round_js(v * 10.0) / 10.0
}

pub fn v3(v: V3) -> [f64; 3] {
    [r3(v.x), r3(v.y), r3(v.z)]
}

/// Wall time for the audits' own measurements (never read by the simulation).
pub mod clock {
    #![allow(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "audits time themselves; the simulation never sees it"
    )]

    pub struct Clock(std::time::Instant);

    impl Clock {
        pub fn start() -> Self {
            Self(std::time::Instant::now())
        }

        pub fn ms(&self) -> f64 {
            self.0.elapsed().as_secs_f64() * 1000.0
        }
    }
}
