//! Runs the audits and prints what they found:
//!   fb_audit [map…] [--quick] [--only a,b] [--skip a,b] [--seed n] [--metrics] [--notes] [--json]
//! The report is also saved to .reports/audit-latest.json. Exit code 1 when there are errors.
use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use fb_audit::{RunOpts, audit_names, format_report, run_audits};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let valued = ["--only", "--skip", "--seed"];
    let opt = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let list = |n: &str| -> Vec<String> {
        opt(n)
            .map(|s| s.split(',').filter(|x| !x.is_empty()).map(String::from).collect())
            .unwrap_or_default()
    };
    let flag = |n: &str| args.iter().any(|a| a == n);
    let maps: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|&(i, a)| !a.starts_with("--") && (i == 0 || !valued.contains(&args[i - 1].as_str())))
        .map(|(_, a)| a.clone())
        .collect();
    let (only, skip) = (list("--only"), list("--skip"));
    let names = audit_names();
    let bad: Vec<&String> = only
        .iter()
        .chain(&skip)
        .filter(|a| !names.contains(&a.as_str()))
        .collect();
    if !bad.is_empty() {
        let bad: Vec<&str> = bad.iter().map(|s| s.as_str()).collect();
        eprintln!("unknown audits: {} (known: {})", bad.join(", "), names.join(", "));
        return ExitCode::from(2);
    }
    let seed = match opt("--seed").map(|s| s.parse::<u32>()) {
        None => None,
        Some(Ok(s)) => Some(s),
        Some(Err(_)) => {
            eprintln!("--seed takes a number");
            return ExitCode::from(2);
        }
    };
    let json = flag("--json");
    let progress = !json && std::io::stderr().is_terminal();
    let on_result = |r: &fb_audit::AuditResult| {
        let mut e = std::io::stderr();
        let _ = write!(e, "\r\x1b[K  {} / {} {} ms", r.map, r.audit, r.ms);
        let _ = e.flush();
    };
    let opts = RunOpts {
        maps,
        only,
        skip,
        quick: flag("--quick"),
        seed,
    };
    let report = run_audits(
        &opts,
        progress.then_some(&on_result as &(dyn Fn(&fb_audit::AuditResult) + Sync)),
    );
    if progress {
        eprint!("\r\x1b[K");
    }
    let text = serde_json::to_string_pretty(&report).expect("the report serializes");
    if std::fs::create_dir_all(".reports").is_ok() {
        let _ = std::fs::write(".reports/audit-latest.json", &text);
    }
    if json {
        println!("{text}");
    } else {
        println!("{}", format_report(&report, flag("--metrics"), flag("--notes")));
    }
    if report.summary.errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
