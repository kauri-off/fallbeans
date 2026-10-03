//! The quick audits, as `bun run check` runs them: 0 errors and 0 warnings.
use fb_audit::{RunOpts, Severity, format_report, run_audits};

#[test]
fn quick_audits_are_clean() {
    let r = run_audits(
        &RunOpts {
            quick: true,
            ..Default::default()
        },
        None,
    );
    let bad = r
        .results
        .iter()
        .flat_map(|x| &x.findings)
        .filter(|f| f.severity != Severity::Info)
        .count();
    assert_eq!(bad, 0, "\n{}", format_report(&r, false, false));
}
