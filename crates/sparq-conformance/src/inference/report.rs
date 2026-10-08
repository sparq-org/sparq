//! Scoreboard plumbing for the inference suites: per-test outcomes in four
//! HONEST buckets (pass / fail / documented divergence / out-of-scope with
//! reason — no silent skips), rendered as the same per-suite tables +
//! `Overall` lines the SPARQL conformance report uses.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum Outcome {
    Pass,
    Fail(String),
    /// A test whose expected result is judged wrong/inapplicable for a
    /// documented reason (rationale, observed behavior). Counts as
    /// pass+divergence in the rate, listed in full.
    Divergence(&'static str, String),
    /// Deliberately not run, with the honest reason (unsupported feature,
    /// not-applicable semantics, …). Excluded from the rate, counted in
    /// coverage, histogrammed in the report.
    OutOfScope(String),
}

#[derive(Debug, Clone)]
pub struct TestResult {
    /// Sub-suite key inside the section (table row), e.g. `cwm_includes`.
    pub suite: String,
    pub name: String,
    pub outcome: Outcome,
}

/// One report section (a wired test suite).
pub struct Section {
    pub label: String,
    /// Free-form scope/method notes rendered under the section heading.
    pub notes: Vec<String>,
    pub results: Vec<TestResult>,
}

#[derive(Default, Clone, Copy)]
pub struct Counts {
    pub pass: usize,
    pub fail: usize,
    pub divergence: usize,
    pub oos: usize,
}

impl Counts {
    fn add(&mut self, o: &Outcome) {
        match o {
            Outcome::Pass => self.pass += 1,
            Outcome::Fail(_) => self.fail += 1,
            Outcome::Divergence(_, _) => self.divergence += 1,
            Outcome::OutOfScope(_) => self.oos += 1,
        }
    }
    pub fn run(&self) -> usize {
        self.pass + self.fail + self.divergence
    }
}

fn pct(n: usize, d: usize) -> String {
    if d == 0 {
        "—".into()
    } else {
        format!("{:.1}%", 100.0 * n as f64 / d as f64)
    }
}

fn git_head(dir: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn render(sections: &[Section], roots: &[(&str, &Path)]) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# sparq inference conformance report\n");
    for (label, dir) in roots {
        let _ = writeln!(md, "- {label}: `{}`", git_head(dir));
    }
    let _ = writeln!(md, "- sparq commit: `{}`", git_head(Path::new(".")));
    let _ = writeln!(
        md,
        "\nEvery manifest entry lands in exactly one bucket — pass, fail, documented \
         divergence, or out-of-scope WITH its reason (no silent skips). \
         Pass rate is `(pass + divergence) / run`; out-of-scope entries are \
         excluded from the rate but counted in coverage.\n"
    );

    let mut grand = Counts::default();
    for sec in sections {
        let _ = writeln!(md, "## {}\n", sec.label);
        for n in &sec.notes {
            let _ = writeln!(md, "{n}\n");
        }
        let mut by_suite: BTreeMap<&str, Counts> = BTreeMap::new();
        let mut total = Counts::default();
        for r in &sec.results {
            by_suite
                .entry(r.suite.as_str())
                .or_default()
                .add(&r.outcome);
            total.add(&r.outcome);
        }
        let _ = writeln!(
            md,
            "| suite | pass | fail | divergence | out-of-scope | pass-rate (of run) |"
        );
        let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|");
        for (suite, c) in &by_suite {
            let _ = writeln!(
                md,
                "| {suite} | {} | {} | {} | {} | {} |",
                c.pass,
                c.fail,
                c.divergence,
                c.oos,
                pct(c.pass + c.divergence, c.run())
            );
        }
        let _ = writeln!(
            md,
            "| **total** | **{}** | **{}** | **{}** | **{}** | **{}** |\n",
            total.pass,
            total.fail,
            total.divergence,
            total.oos,
            pct(total.pass + total.divergence, total.run())
        );
        let _ = writeln!(
            md,
            "**Overall ({}): {} pass / {} fail / {} documented divergence / {} out-of-scope — pass+divergence {} of run, {} of all in-scope tests.**\n",
            sec.label,
            total.pass,
            total.fail,
            total.divergence,
            total.oos,
            pct(total.pass + total.divergence, total.run()),
            pct(total.pass + total.divergence, total.run() + total.oos)
        );

        // Divergences in full.
        let divs: Vec<&TestResult> = sec
            .results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Divergence(_, _)))
            .collect();
        if !divs.is_empty() {
            let _ = writeln!(md, "### Documented divergences\n");
            for r in divs {
                if let Outcome::Divergence(rationale, observed) = &r.outcome {
                    let _ = writeln!(
                        md,
                        "- `{}` — **{}**: {rationale}.\n  *observed*: {observed}",
                        r.suite, r.name
                    );
                }
            }
            let _ = writeln!(md);
        }

        // Out-of-scope histogram.
        let mut oos_hist: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &sec.results {
            if let Outcome::OutOfScope(reason) = &r.outcome {
                *oos_hist.entry(reason.as_str()).or_default() += 1;
            }
        }
        if !oos_hist.is_empty() {
            let _ = writeln!(md, "### Out-of-scope reasons\n");
            let mut hist: Vec<_> = oos_hist.into_iter().collect();
            hist.sort_by_key(|e| std::cmp::Reverse(e.1));
            let _ = writeln!(md, "| reason | tests |");
            let _ = writeln!(md, "|---|---:|");
            for (reason, n) in hist {
                let _ = writeln!(md, "| {reason} | {n} |");
            }
            let _ = writeln!(md);
        }

        // Failures in full (collapsible).
        let fails: Vec<&TestResult> = sec
            .results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Fail(_)))
            .collect();
        if !fails.is_empty() {
            let _ = writeln!(
                md,
                "<details><summary>All failures ({})</summary>\n",
                fails.len()
            );
            for r in fails {
                if let Outcome::Fail(reason) = &r.outcome {
                    let _ = writeln!(md, "- `{}` — **{}**: {reason}", r.suite, r.name);
                }
            }
            let _ = writeln!(md, "\n</details>\n");
        }

        grand.pass += total.pass;
        grand.fail += total.fail;
        grand.divergence += total.divergence;
        grand.oos += total.oos;
    }

    let _ = writeln!(md, "## Overall (all inference suites)\n");
    let _ = writeln!(
        md,
        "**{} pass / {} fail / {} documented divergence / {} out-of-scope — \
         pass+divergence {} of run, {} of all in-scope tests.**",
        grand.pass,
        grand.fail,
        grand.divergence,
        grand.oos,
        pct(grand.pass + grand.divergence, grand.run()),
        pct(grand.pass + grand.divergence, grand.run() + grand.oos)
    );
    md
}
