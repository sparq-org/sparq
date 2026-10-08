//! [OPUS-4.8] sq-ro6if (PSS #1415 / #1478) — `served-conformance-report`: emits the
//! MACHINE-READABLE served-surface SPARQL 1.1 Protocol conformance report as JSON.
//!
//! PUBLICATION ONLY. This binary runs NO new conformance tests: it re-runs the two
//! EXISTING served-surface lanes in-process against the `sparq-server` loopback
//! endpoint — the sq-jaj38 HTTP-protocol lane (GET/POST query+update, the QUERY
//! method, dataset overrides, SRJ/SRX/CSV/TSV negotiation, status codes) and the
//! sq-1uuxz Service-Description + Graph-Store-Protocol lane — and serialises their
//! per-assertion outcomes + rolled-up counts to a single JSON document. Consumers
//! (e.g. PSS, which consumes the served HTTP surface only) get a published, versioned
//! per-release record of what the WIRE contract passes, without re-running the suite.
//!
//! It reuses the SAME public functions the ratchet tests drive
//! (`sparq_conformance::http_protocol::run_protocol_suite`,
//! `sparq_conformance::sd_gsp::run_sd_gsp_suite`) and pulls each suite's ratchet
//! FLOOR from the central registry ([`sparq_conformance::scoreboard::SUITES`]) so the
//! report can never drift from the enforcer.
//!
//! ## Fail-closed discipline (the vacuous-artifact guard)
//!
//! The report is emitted from the lanes; it must FAIL LOUDLY rather than silently
//! publish an incomplete/misleading artifact. The binary writes the JSON (so the
//! artifact always records the honest state) and then exits NON-ZERO if ANY of:
//!
//! - a lane produced ZERO assertions (the loopback server did not run — a suite would be silently omitted);
//! - a lane has any genuine `Fail` outcome;
//! - a lane's pass count regressed below its registry ratchet floor;
//! - a lane is missing from `scoreboard::SUITES` (registry drift — no floor to check).
//!
//! A non-zero exit fails the CI/release step, so a bad report can never ship. Documented
//! DIVERGENCES are reported per-assertion but are NEVER summed into `pass` and never fail the
//! build — an honest gap can never inflate the conformance number.
//!
//! Provenance (commit / version / date / run source) is threaded from env by the
//! caller (CI/release), with local fall-backs so a bare `cargo run` still produces a
//! self-describing artifact.
//!
//!   cargo run --release -p sparq-conformance \
//!     --features http-protocol,federation-descriptors \
//!     --bin served-conformance-report -- --out served-conformance.json
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(all(feature = "http-protocol", feature = "federation-descriptors"))]
    {
        // --out FILE (default served-conformance.json).
        let mut out = std::path::PathBuf::from("served-conformance.json");
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--out" => {
                    out = std::path::PathBuf::from(args.next().unwrap_or_else(|| {
                        eprintln!("--out needs a path");
                        std::process::exit(2);
                    }))
                }
                other => {
                    eprintln!("unknown argument: {other}");
                    eprintln!(
                        "usage: served-conformance-report [--out FILE]  \
                         (requires --features http-protocol,federation-descriptors)"
                    );
                    return ExitCode::from(2);
                }
            }
        }
        match emit::run(&out) {
            Ok(()) => ExitCode::SUCCESS,
            Err(fatal) => {
                eprintln!(
                    "\nserved-conformance-report: FAIL-CLOSED — the report is NOT publishable:"
                );
                for line in fatal {
                    eprintln!("  - {line}");
                }
                ExitCode::from(1)
            }
        }
    }
    #[cfg(not(all(feature = "http-protocol", feature = "federation-descriptors")))]
    {
        eprintln!(
            "served-conformance-report requires BOTH the `http-protocol` and \
             `federation-descriptors` features to run the served-surface lanes.\n\
             Run: cargo run -p sparq-conformance \
             --features http-protocol,federation-descriptors \
             --bin served-conformance-report -- --out served-conformance.json"
        );
        ExitCode::from(2)
    }
}

#[cfg(all(feature = "http-protocol", feature = "federation-descriptors"))]
mod emit {
    use serde_json::{json, Value};
    use sparq_conformance::http_protocol::{self, Outcome, SuiteReport};
    use sparq_conformance::scoreboard::{Runner, Suite, SUITES};
    use sparq_conformance::sd_gsp;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// One served-surface lane to publish: the human `id`/`title` for the report and the
    /// `scoreboard::SUITES` runner `target` used to locate its registry row (label / family /
    /// ci_job / ratchet floor). Keeping the floor in the registry (never duplicated here) means
    /// the report can only ever agree with the enforcer.
    struct Lane {
        id: &'static str,
        /// The scoreboard runner `--test` target that identifies the registry row.
        target: &'static str,
    }

    const LANES: &[Lane] = &[
        Lane {
            id: "http-protocol",
            target: "http_protocol_suite",
        },
        Lane {
            id: "sd-gsp",
            target: "sd_gsp_suite",
        },
    ];

    /// The `scoreboard::SUITES` row whose feature-gated runner targets `target` (or None on
    /// registry drift — a fail-closed condition, never silently ignored).
    fn suite_for(target: &str) -> Option<&'static Suite> {
        SUITES.iter().find(
            |s| matches!(s.runner, Runner::FeatureGatedCrateTest { target: t, .. } if t == target),
        )
    }

    /// The cargo feature a feature-gated runner requires (for the report's provenance column).
    fn suite_feature(s: &Suite) -> &'static str {
        match s.runner {
            Runner::FeatureGatedCrateTest { feature, .. } => feature,
            _ => "",
        }
    }

    fn outcome_json(label: &str, o: &Outcome) -> Value {
        match o {
            Outcome::Pass => json!({ "label": label, "outcome": "pass" }),
            Outcome::Divergence(why) => {
                json!({ "label": label, "outcome": "divergence", "detail": why })
            }
            Outcome::Fail(why) => json!({ "label": label, "outcome": "fail", "detail": why }),
        }
    }

    /// Build one lane's JSON object and append any FATAL conditions to `fatal`.
    fn lane_json(lane: &Lane, report: &SuiteReport, fatal: &mut Vec<String>) -> Value {
        let pass = report.pass();
        let div = report.divergences();
        let fails = report.failures();
        let total = report.outcomes.len();

        // Vacuous-artifact guard: a lane that emitted NO assertions means the loopback server
        // never ran — the suite would be silently omitted from the published counts.
        if total == 0 {
            fatal.push(format!(
                "lane `{}` produced ZERO assertions (the loopback server did not run) — refusing \
                 to publish a report that silently omits a served suite",
                lane.id
            ));
        }
        // A genuine failure is never laundered into the artifact as a pass.
        for (label, why) in &fails {
            fatal.push(format!(
                "lane `{}` FAILED assertion: {} — {}",
                lane.id, label, why
            ));
        }

        // Floor + registry metadata from the central scoreboard (single source of truth).
        let (label, family, ci_job, floor, floor_basis, note, feature) =
            match suite_for(lane.target) {
                Some(s) => (
                    s.label,
                    s.family,
                    s.ci_job,
                    Some(s.ratchet_floor),
                    s.floor_basis,
                    s.note,
                    suite_feature(s),
                ),
                None => {
                    fatal.push(format!(
                        "registry drift: no scoreboard::SUITES row with a feature-gated runner \
                     targeting `{}` — cannot bind lane `{}` to a ratchet floor",
                        lane.target, lane.id
                    ));
                    ("", "", "", None, "", "", "")
                }
            };
        if let Some(f) = floor {
            if pass < f {
                fatal.push(format!(
                    "lane `{}` pass count {} regressed below its ratchet floor {}",
                    lane.id, pass, f
                ));
            }
        }

        let tests: Vec<Value> = report
            .outcomes
            .iter()
            .map(|(l, o)| outcome_json(l, o))
            .collect();

        json!({
            "id": lane.id,
            "label": label,
            "family": family,
            "feature": feature,
            "ci_job": ci_job,
            "floor": floor,
            "floor_basis": floor_basis,
            "note": note,
            "counts": { "pass": pass, "fail": fails.len(), "divergence": div, "total": total },
            "tests": tests,
        })
    }

    /// `s / 86400` days since the epoch → UTC calendar date (Howard Hinnant's `civil_from_days`),
    /// plus the wall-clock h:m:s from the day remainder — a dependency-free ISO-8601 UTC stamp for
    /// the local fall-back (CI/release pass an exact `SPARQ_CONFORMANCE_DATE` instead).
    fn iso_utc(secs: i64) -> String {
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
        let y = y + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }

    /// Env value or a fall-back, trimming to None on empty (so a set-but-empty CI var falls back).
    fn env_or(key: &str, fallback: &str) -> String {
        match std::env::var(key) {
            Ok(v) if !v.trim().is_empty() => v,
            _ => fallback.to_string(),
        }
    }

    pub fn run(out: &Path) -> Result<(), Vec<String>> {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Provenance — threaded from the caller (CI/release) with honest local fall-backs.
        let commit = env_or(
            "SPARQ_CONFORMANCE_COMMIT",
            "unknown (local build — set SPARQ_CONFORMANCE_COMMIT)",
        );
        let version = env_or("SPARQ_CONFORMANCE_VERSION", "unreleased (local build)");
        let run_source = env_or(
            "SPARQ_CONFORMANCE_RUN_URL",
            "local build (no CI run source)",
        );
        let generated_at = env_or("SPARQ_CONFORMANCE_DATE", &iso_utc(now_unix));

        // Re-run the two served-surface lanes in-process (hermetic; each stands up its own
        // sparq-server loopback endpoint on an ephemeral 127.0.0.1:0 port).
        let http = http_protocol::run_protocol_suite();
        let sdgsp = sd_gsp::run_sd_gsp_suite();
        let reports = [(&LANES[0], http), (&LANES[1], sdgsp)];

        let mut fatal: Vec<String> = Vec::new();
        let mut suites: Vec<Value> = Vec::new();
        let (mut tp, mut tf, mut td, mut tt) = (0usize, 0usize, 0usize, 0usize);
        for (lane, report) in &reports {
            tp += report.pass();
            tf += report.failures().len();
            td += report.divergences();
            tt += report.outcomes.len();
            suites.push(lane_json(lane, report, &mut fatal));
        }

        let doc = json!({
            "schema": "sparq.served-conformance/v1",
            "title": "Served-surface SPARQL 1.1 Protocol conformance",
            "provenance": {
                "commit": commit,
                "version": version,
                "generated_at": generated_at,
                "generated_at_unix": now_unix,
                "run_source": run_source,
                "runner": "sparq-conformance served-conformance-report",
                "note": "Re-runs the sq-jaj38 (http-protocol) and sq-1uuxz (federation-descriptors) \
                         served-surface lanes in-process against the sparq-server loopback endpoint. \
                         Pass/divergence/fail counts are conformance facts; `floor` is each suite's \
                         ratchet floor (may only rise) from crates/sparq-conformance/src/scoreboard.rs. \
                         Documented divergences are reported per-assertion and are NEVER summed into \
                         `pass`, so a documented gap can never inflate the conformance number."
            },
            "totals": { "pass": tp, "fail": tf, "divergence": td, "total": tt },
            "suites": suites,
        });

        // Write the artifact ALWAYS (it records the honest state even on a fatal condition), then
        // surface any fatal condition so the caller (CI/release) fails-closed.
        let pretty = serde_json::to_string_pretty(&doc)
            .map_err(|e| vec![format!("could not serialise report: {e}")])?;
        std::fs::write(out, format!("{pretty}\n"))
            .map_err(|e| vec![format!("could not write {}: {e}", out.display())])?;

        // Human summary for the CI step log / local run.
        eprintln!(
            "served-surface SPARQL 1.1 Protocol conformance — {} pass / {} divergence / {} fail \
             across {} assertions (2 suites)",
            tp, td, tf, tt
        );
        eprintln!("report written to {}", out.display());

        if fatal.is_empty() {
            Ok(())
        } else {
            Err(fatal)
        }
    }
}
