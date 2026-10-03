//! `cargo xtask audit`: the audits (`fb_audit`), and with `--vs-ts` the same audits of the TS version
//! (computed with this libm, see `golden`) compared result by result.
use std::collections::BTreeMap;
use std::process::Command;

use clap::Args;
use serde_json::Value;

use crate::{build_golden_libm, bun, cargo, run};

#[derive(Args, Clone, Debug)]
pub struct AuditArgs {
    /// Run the TS audits too (needs bun and the wasm32 target) and compare: everything but wall times
    /// must be equal.
    #[arg(long)]
    vs_ts: bool,
    /// Passed on to the audits: [map…] [--quick] [--only a,b] [--skip a,b] [--seed n] [--metrics]
    /// [--notes] [--json].
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// Metrics that measure the machine, not the game.
const TIMING: [&str; 4] = ["ms", "msPerTick", "buildMs", "msPerTick8Bots"];

pub fn audit(a: &AuditArgs) -> bool {
    if !a.vs_ts {
        return run(cargo().args(["run", "--release", "-p", "fb_audit", "--"]).args(&a.args));
    }
    if !build_golden_libm() {
        return false;
    }
    let args: Vec<&str> = a.args.iter().map(String::as_str).filter(|x| *x != "--json").collect();
    let mut rs = cargo();
    rs.args(["run", "-q", "--release", "-p", "fb_audit", "--"])
        .args(&args)
        .arg("--json");
    let mut ts_args = vec!["--preload", "./scripts/golden-math.ts", "scripts/audit.ts"];
    ts_args.extend(&args);
    ts_args.push("--json");
    let (Some(rs), Some(ts)) = (report(rs, "Rust"), report(bun(&ts_args), "TS")) else {
        return false;
    };
    compare(&ts, &rs)
}

fn report(mut c: Command, who: &str) -> Option<Value> {
    eprintln!("$ {c:?}");
    let out = c.output().ok()?;
    match serde_json::from_slice(&out.stdout) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("{who} audit: no report ({e})\n{}", String::from_utf8_lossy(&out.stderr));
            None
        }
    }
}

fn results(r: &Value) -> BTreeMap<(String, String), &Value> {
    r["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let s = |k: &str| x[k].as_str().unwrap_or_default().to_string();
            ((s("map"), s("audit")), x)
        })
        .collect()
}

/// Numbers as numbers (Rust writes 9.0 where JS writes 9), everything else as is.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

fn compare(ts: &Value, rs: &Value) -> bool {
    let (ts, rs) = (results(ts), results(rs));
    let mut diffs = 0;
    for key in ts.keys().chain(rs.keys().filter(|k| !ts.contains_key(*k))) {
        let (map, audit) = key;
        // The input audit was rewritten for Rust (no TS codec in the core).
        if audit == "input" {
            continue;
        }
        let (Some(t), Some(r)) = (ts.get(key), rs.get(key)) else {
            println!(
                "{map} / {audit}: only in {}",
                if ts.contains_key(key) { "TS" } else { "Rust" }
            );
            diffs += 1;
            continue;
        };
        let metrics = |v: &Value| -> Value {
            let mut m = v["metrics"].as_object().cloned().unwrap_or_default();
            m.retain(|k, _| !TIMING.contains(&k.as_str()));
            Value::Object(m)
        };
        let (mt, mr) = (metrics(t), metrics(r));
        if !same(&mt, &mr) {
            println!("{map} / {audit}: metrics differ\n  TS   {mt}\n  Rust {mr}");
            diffs += 1;
        }
        if !same(&t["findings"], &r["findings"]) {
            println!(
                "{map} / {audit}: findings differ\n  TS   {}\n  Rust {}",
                t["findings"], r["findings"]
            );
            diffs += 1;
        }
    }
    println!("{} results compared, {diffs} differ", ts.len().max(rs.len()));
    diffs == 0
}
