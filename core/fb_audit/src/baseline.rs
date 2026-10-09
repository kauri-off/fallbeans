//! How the game feels, in numbers, against the recorded baseline (`baseline.json`): how the bean handles, and
//! how bots do on every map averaged over seeds. A change to the simulation that moves one of them beyond its
//! tolerance changes the feel. `fb_audit --baseline` compares, `fb_audit --bless-baseline` records.
use std::collections::BTreeMap;

use fb_shared::game::Genre;
use fb_shared::m::{self, MinMax};
use fb_sim::map::MapDef;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::harness::median;
use crate::maps::{SeedRun, play_seed};
use crate::{Ctx, Metric, Out, r3};

pub const SEEDS: [u32; 16] = [11, 23, 37, 51, 77, 91, 113, 131, 149, 167, 181, 199, 211, 229, 241, 257];

/// Bots in a round of the balance runs.
const BEANS: f64 = 8.0;

/// A measured value and how far it may move.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub value: f64,
    pub tol: f64,
}

pub type Baseline = BTreeMap<String, Entry>;

pub fn path() -> String {
    format!("{}/baseline.json", env!("CARGO_MANIFEST_DIR"))
}

/// Per-seed values of one metric; the tolerance is the larger of the floors and four standard errors of
/// the mean (a fresh set of runs of an unchanged game differs from the recorded one by √2 standard errors).
fn spread(values: &[f64], rel: f64, abs: f64) -> Option<Entry> {
    if values.is_empty() {
        return None;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let se = if values.len() > 1 {
        m::sqrt(values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.0) / n)
    } else {
        0.0
    };
    Some(Entry {
        value: r3(mean),
        tol: r3((rel * mean.abs()).at_least(abs).at_least(4.0 * se)),
    })
}

fn physics(into: &mut Baseline) {
    let mut out = Out::default();
    crate::systems::physics(&Ctx { quick: false, seed: 0 }, &mut out);
    for (k, v) in out.metrics {
        if let Metric::Num(v) = v {
            let tol = r3((0.02 * v.abs()).at_least(0.005));
            into.insert(format!("physics/{k}"), Entry { value: v, tol });
        }
    }
}

fn balance(map: &'static dyn MapDef, into: &mut Baseline) {
    let meta = map.meta();
    let runs: Vec<SeedRun> = SEEDS.par_iter().map(|&s| play_seed(map, s)).collect();
    let id = meta.id;
    let mut put = |k: &str, values: Vec<f64>, rel: f64, abs: f64| {
        if let Some(e) = spread(&values, rel, abs) {
            into.insert(format!("balance/{id}/{k}"), e);
        }
    };
    // (A fall in a survival round is an elimination, not a fall: always 0 there.)
    if meta.genre != Genre::Survival {
        put(
            "falls_per_bot_min",
            runs.iter()
                .map(|r| r.falls.len() as f64 / r.bot_seconds * 60.0)
                .collect(),
            0.15,
            0.2,
        );
    }
    match meta.genre {
        Genre::Race => {
            put(
                "finish_rate",
                runs.iter().map(|r| r.finish_times.len() as f64 / BEANS).collect(),
                0.0,
                0.05,
            );
            put(
                "finish_p50",
                runs.iter()
                    .filter(|r| !r.finish_times.is_empty())
                    .map(|r| median(&r.finish_times))
                    .collect(),
                0.05,
                0.5,
            );
        }
        // (A mean over the 8 bots: the first elimination alone varies too much from seed to seed.)
        Genre::Survival => {
            put(
                "survival",
                runs.iter().map(|r| r.survival(meta.duration)).collect(),
                0.03,
                1.0,
            );
        }
        Genre::Points => {
            put("top_score", runs.iter().map(|r| r.top_score).collect(), 0.1, 1.0);
        }
    }
}

pub fn measure() -> Baseline {
    let mut b = Baseline::new();
    physics(&mut b);
    for &map in fb_maps::GAMES {
        balance(map, &mut b);
    }
    b
}

/// One line per value; problems are values beyond the recorded tolerance, or missing on either side.
pub fn compare(got: &Baseline, want: &Baseline) -> (Vec<String>, Vec<String>) {
    let mut lines = Vec::new();
    let mut problems = Vec::new();
    for (k, w) in want {
        match got.get(k) {
            None => problems.push(format!("{k}: not measured any more")),
            Some(g) => {
                let d = g.value - w.value;
                let line = format!("{k}: {} (was {} ± {}, {:+})", g.value, w.value, w.tol, r3(d));
                if d.abs() > w.tol {
                    problems.push(line.clone());
                }
                lines.push(line);
            }
        }
    }
    for k in got.keys().filter(|k| !want.contains_key(*k)) {
        problems.push(format!("{k}: new, not in the baseline"));
    }
    (lines, problems)
}
