//! **The seeded CI driver window** — feed generated cases to the TLP + NoREC oracles
//! and report every verdict, fail-closed. Bead `sq-3dyje.9` (epic
//! `sq-3dyje`, design record `research/testing-strategy-assessment-2026-07.md` §7.3).
//!
//! This module is the missing plumbing between the oracles and CI: the
//! `metamorph-driver` binary (`src/bin/metamorph-driver.rs`) is a thin wrapper over
//! [`run_window`], and `.github/workflows/metamorph.yml` runs it nightly over an
//! advancing seed window plus a fixed smoke window (mirroring `differential.yml`'s
//! repro + auto-file pattern).
//!
//! # Verdict accounting (never silently skipped)
//!
//! Every seed in the window is checked by **both** single-engine oracles
//! ([`crate::tlp`], [`crate::norec`]) against the in-process sparq engine, and every
//! verdict is counted in the [`WindowReport`]:
//!
//! * [`Verdict::Pass`] — the relation held; counted, nothing printed.
//! * [`Verdict::Violation`] — a wrong-result signal; a `VIOLATION seed=N` repro line
//!   is printed and the window fails.
//! * [`Verdict::EngineFailure`] — a query did not evaluate. The generator emits only
//!   valid SPARQL inside the oracles' scope preconditions, so an engine failure on a
//!   generated case is itself a finding (an engine-error bug or a harness bug), never
//!   an excuse: an `ENGINE-FAILURE seed=N` line is printed and the window **fails**
//!   (fail-closed — a lane that skips errors proves nothing).
//!
//! The first non-Pass verdict is echoed at the end of the log as a complete
//! deterministic repro block (`FIRST FAILING CASE:` — seed, oracle, replay command,
//! all queries, the N-Triples graph), which `scripts/ci-file-metamorph-failure.py`
//! parses into the auto-filed GitHub issue + P1 bug bead.
//!
//! # Non-vacuity anchor
//!
//! [`check_seed`] takes an `inject_filter_drops_row` switch that wraps the engine in
//! the [`FilterDropsRow`] wrong-result mutant. The self-tests assert a window over the
//! mutant **fails with both oracles reporting violations** — a driver that cannot flag
//! the crate's own seeded bug would make the whole lane vacuous. The binary exposes
//! the switch as `--inject-filter-drops-row` so the same red-path can be demonstrated
//! end-to-end in CI or locally (never enabled in the standing workflow).

use std::io::{self, Write};

use crate::engine::{FilterDropsRow, InProcessSparq, SparqlEngine};
use crate::generate::{generate_case, GeneratedCase};
use crate::norec::{check_norec, norec_queries};
use crate::tlp::{check_tlp, tlp_queries};
use crate::verdict::Verdict;

/// The engine name used for every verdict this driver produces.
const ENGINE_NAME: &str = "sparq";

/// Both oracle verdicts for one generated seed, with the case kept for repro output.
#[derive(Debug, Clone)]
pub struct SeedOutcome {
    /// The generated case (seed + data + pattern + predicate).
    pub case: GeneratedCase,
    /// The TLP verdict for this case.
    pub tlp: Verdict,
    /// The NoREC verdict for this case.
    pub norec: Verdict,
}

impl SeedOutcome {
    /// True iff both oracles passed.
    pub fn is_pass(&self) -> bool {
        self.tlp.is_pass() && self.norec.is_pass()
    }
}

/// Aggregate verdict counts for one seed window. Every generated case lands in
/// exactly one of pass / violation / engine-failure **per oracle** — nothing is
/// skipped or silently dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowReport {
    /// First seed of the window.
    pub start: u64,
    /// Number of seeds checked (equals the requested count).
    pub checked: u64,
    /// Seeds where both oracles passed.
    pub pass: u64,
    /// TLP wrong-result violations.
    pub tlp_violations: u64,
    /// NoREC wrong-result violations.
    pub norec_violations: u64,
    /// Verdicts (across both oracles) where a query failed to evaluate.
    pub engine_failures: u64,
}

impl WindowReport {
    /// True iff the window is green: at least one seed was checked and **every**
    /// verdict was a pass. Fail-closed: an empty window (`count == 0`) is NOT a
    /// pass — a misconfigured lane that checks nothing must go red, not green.
    pub fn all_pass(&self) -> bool {
        self.checked > 0 && self.pass == self.checked
    }

    /// The one-line machine-readable summary the workflow log ends with (parsed by
    /// `scripts/ci-file-metamorph-failure.py`).
    pub fn summary_line(&self) -> String {
        format!(
            "metamorph seeds {}..{}: checked={} pass={} tlp-violation={} norec-violation={} engine-failure={}",
            self.start,
            self.start + self.checked,
            self.checked,
            self.pass,
            self.tlp_violations,
            self.norec_violations,
            self.engine_failures
        )
    }
}

/// Run both oracles on the case generated from `seed`.
///
/// With `inject_filter_drops_row` set, the engine is wrapped in the
/// [`FilterDropsRow`] wrong-result mutant (the non-vacuity red-path — see the module
/// docs); the standing CI lane always passes `false`.
///
/// A data-load failure is reported as an engine failure on **both** oracles (the
/// verdicts are counted, never skipped).
pub fn check_seed(seed: u64, inject_filter_drops_row: bool) -> SeedOutcome {
    let case = generate_case(seed);
    match InProcessSparq::from_ntriples(ENGINE_NAME, &case.data_ntriples) {
        Err(failure) => SeedOutcome {
            case,
            tlp: Verdict::EngineFailure(failure.clone()),
            norec: Verdict::EngineFailure(failure),
        },
        Ok(engine) => {
            if inject_filter_drops_row {
                let mutant = FilterDropsRow::new(engine);
                run_oracles(&mutant, case)
            } else {
                run_oracles(&engine, case)
            }
        }
    }
}

fn run_oracles(engine: &dyn SparqlEngine, case: GeneratedCase) -> SeedOutcome {
    let tlp = check_tlp(engine, &case.pattern, &case.predicate);
    let norec = check_norec(engine, &case.pattern, &case.predicate);
    SeedOutcome { case, tlp, norec }
}

/// One oracle verdict rendered as a log line (`None` for a pass — passes are counted
/// in the summary, not printed per seed).
fn verdict_line(oracle: &str, seed: u64, verdict: &Verdict) -> Option<String> {
    match verdict {
        Verdict::Pass { .. } => None,
        Verdict::Violation(v) => Some(format!("VIOLATION seed={seed} oracle={oracle}: {v}")),
        Verdict::EngineFailure(f) => {
            Some(format!("ENGINE-FAILURE seed={seed} oracle={oracle}: {f}"))
        }
    }
}

/// Render the complete deterministic repro block for the first failing seed:
/// replayable command, verdict detail, every oracle query, and the graph.
fn first_failing_case_block(outcome: &SeedOutcome) -> String {
    let seed = outcome.case.seed;
    let mut block = String::from("FIRST FAILING CASE:\n");
    block.push_str(&format!("seed={seed}\n"));
    block.push_str(&format!(
        "replay: cargo run -p sparq-metamorph --bin metamorph-driver -- {seed} 1\n"
    ));
    for (oracle, verdict) in [("tlp", &outcome.tlp), ("norec", &outcome.norec)] {
        if let Some(line) = verdict_line(oracle, seed, verdict) {
            block.push_str(&line);
            block.push('\n');
        }
    }
    let tlp = tlp_queries(&outcome.case.pattern, &outcome.case.predicate);
    let norec = norec_queries(&outcome.case.pattern, &outcome.case.predicate);
    block.push_str("--- queries ---\n");
    for query in [
        &tlp.base,
        &tlp.branch_true,
        &tlp.branch_false,
        &tlp.branch_error,
        &norec.optimized,
        &norec.rewritten,
    ] {
        block.push_str(query);
        block.push('\n');
    }
    block.push_str("--- graph ---\n");
    block.push_str(&outcome.case.data_ntriples);
    block
}

/// Drive the TLP + NoREC oracles over seeds `start .. start + count`, streaming a
/// repro line per non-Pass verdict to `out`, then the `FIRST FAILING CASE:` block
/// (if any) and the one-line summary. Returns the [`WindowReport`]; the caller
/// (the `metamorph-driver` binary) turns [`WindowReport::all_pass`] into the exit
/// code. Deterministic: same window + same switch, byte-identical output, forever.
pub fn run_window(
    start: u64,
    count: u64,
    inject_filter_drops_row: bool,
    out: &mut dyn Write,
) -> io::Result<WindowReport> {
    let mut report = WindowReport {
        start,
        ..WindowReport::default()
    };
    let mut first_failure: Option<String> = None;
    for seed in start..start + count {
        let outcome = check_seed(seed, inject_filter_drops_row);
        report.checked += 1;
        if outcome.is_pass() {
            report.pass += 1;
        } else if first_failure.is_none() {
            first_failure = Some(first_failing_case_block(&outcome));
        }
        for (oracle, verdict) in [("tlp", &outcome.tlp), ("norec", &outcome.norec)] {
            match verdict {
                Verdict::Pass { .. } => {}
                Verdict::Violation(_) => {
                    match oracle {
                        "tlp" => report.tlp_violations += 1,
                        _ => report.norec_violations += 1,
                    }
                    writeln!(out, "{}", verdict_line(oracle, seed, verdict).unwrap())?;
                }
                Verdict::EngineFailure(_) => {
                    report.engine_failures += 1;
                    writeln!(out, "{}", verdict_line(oracle, seed, verdict).unwrap())?;
                }
            }
        }
    }
    if let Some(block) = &first_failure {
        writeln!(out, "\n{block}")?;
    }
    writeln!(out, "{}", report.summary_line())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_seed_passes_both_oracles_on_the_real_engine() {
        let outcome = check_seed(0, false);
        assert!(outcome.tlp.is_pass(), "TLP on seed 0: {:?}", outcome.tlp);
        assert!(outcome.norec.is_pass(), "NoREC on seed 0: {:?}", outcome.norec);
        assert!(outcome.is_pass());
        assert_eq!(outcome.case.seed, 0);
    }

    #[test]
    fn check_seed_with_injected_mutant_is_flagged() {
        // The non-vacuity anchor: the FilterDropsRow mutant loses a row from every
        // FILTER query, so TLP must flag it on any seed whose base is non-empty
        // (the generator guarantees >= 4 base rows).
        let outcome = check_seed(0, true);
        assert!(
            outcome.tlp.is_violation(),
            "TLP must flag the injected mutant: {:?}",
            outcome.tlp
        );
        assert!(!outcome.is_pass());
    }

    #[test]
    fn window_report_all_pass_is_fail_closed_on_empty_and_on_any_non_pass() {
        assert!(!WindowReport::default().all_pass(), "empty window must not pass");
        let green = WindowReport {
            start: 0,
            checked: 5,
            pass: 5,
            ..WindowReport::default()
        };
        assert!(green.all_pass());
        let red = WindowReport {
            start: 0,
            checked: 5,
            pass: 4,
            engine_failures: 1,
            ..WindowReport::default()
        };
        assert!(!red.all_pass());
    }

    #[test]
    fn summary_line_carries_every_count() {
        let report = WindowReport {
            start: 10,
            checked: 5,
            pass: 3,
            tlp_violations: 2,
            norec_violations: 1,
            engine_failures: 1,
        };
        assert_eq!(
            report.summary_line(),
            "metamorph seeds 10..15: checked=5 pass=3 tlp-violation=2 norec-violation=1 engine-failure=1"
        );
    }

    #[test]
    fn run_window_green_path_prints_only_the_summary() {
        let mut out = Vec::new();
        let report = run_window(0, 5, false, &mut out).unwrap();
        assert!(report.all_pass(), "seeds 0..5 must pass on main: {report:?}");
        assert_eq!(report.checked, 5);
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("VIOLATION"), "green window prints no repro: {text}");
        assert!(!text.contains("FIRST FAILING CASE"), "{text}");
        assert!(text.trim_end().ends_with(&report.summary_line()));
    }

    #[test]
    fn run_window_red_path_prints_repro_and_fails() {
        // Injected FilterDropsRow over a window: both oracles must report
        // violations somewhere in the window, the log must carry per-seed repro
        // lines plus the FIRST FAILING CASE block with replay + queries + graph.
        let mut out = Vec::new();
        let report = run_window(0, 25, true, &mut out).unwrap();
        assert!(!report.all_pass());
        assert!(report.tlp_violations > 0, "{report:?}");
        assert!(report.norec_violations > 0, "{report:?}");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("VIOLATION seed=0 oracle=tlp"), "{text}");
        assert!(text.contains("FIRST FAILING CASE:"), "{text}");
        assert!(text.contains("replay: cargo run -p sparq-metamorph --bin metamorph-driver -- 0 1"));
        assert!(text.contains("--- queries ---") && text.contains("--- graph ---"));
        assert!(text.contains("<http://example.org/s0>"), "graph inline: {text}");
        assert!(text.contains(&report.summary_line()));
    }

    #[test]
    fn run_window_is_deterministic() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        let ra = run_window(100, 10, true, &mut a).unwrap();
        let rb = run_window(100, 10, true, &mut b).unwrap();
        assert_eq!(ra, rb);
        assert_eq!(a, b, "same window, byte-identical log");
    }

    #[test]
    fn run_window_empty_window_is_red() {
        let mut out = Vec::new();
        let report = run_window(0, 0, false, &mut out).unwrap();
        assert!(!report.all_pass(), "count=0 must fail closed");
    }
}
