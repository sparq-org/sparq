//! [FABLE-5] (sq-tonhr.2) The REUSABLE parser DIFFERENTIAL harness for the rdf-shuttle
//! generated-parser program (epic sq-tonhr): drive a CANDIDATE parser and an INCUMBENT
//! parser over the same inputs and require an IDENTICAL verdict on every one —
//! accept/reject agreement, and on accept an identical quad SET (blank-node-bijection
//! identity via [`crate::quadset`]; set semantics — duplicate statement emission is
//! not a divergence). This is the zero-regression gate every generated
//! NT/NQ/TriG/Turtle parser (sq-tonhr.8/.9) must pass over the full W3C suites + the
//! committed fuzz seed corpus BEFORE any default flip (sq-tonhr.11).
//!
//! Design notes:
//! - A parser is just a name + closure (`&str -> Result<quads, err>`), so a candidate can
//!   be an in-tree entry point, a generated parser behind a feature, or a deliberately
//!   seeded MUTANT (how `tests/parser_differential.rs` proves this harness non-vacuous).
//! - Divergences carry a MINIMAL REPRO: a greedy line-set shrink (ddmin-lite) re-runs the
//!   comparison on subsets of the input's lines and keeps the smallest still-diverging
//!   document. Exact for the line-oriented formats (N-Triples/N-Quads); for block formats
//!   (TriG) subsets that no longer parse on EITHER side count as non-diverging, so the
//!   shrink simply stops early and the repro stays sound (a diverging document), just not
//!   always minimal.
//! - An exhausted blank-node-isomorphism budget is reported as UNVERIFIED, never counted
//!   as agreement or divergence.

use crate::quadset::{compare_quad_sets, SetCompare};
use std::path::Path;

/// [FABLE-5] Why a differential comparison could not reach a verdict.
///
/// A comparison normally resolves to agreement or a concrete divergence. This
/// error is returned only when the blank-node isomorphism check exhausts its
/// budget, so neither agreement nor divergence can be soundly asserted (the
/// input is reported as UNVERIFIED, never counted as either).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareError {
    /// The blank-node isomorphism budget was exhausted before a verdict.
    BudgetExhausted,
}

impl std::fmt::Display for CompareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompareError::BudgetExhausted => {
                write!(
                    f,
                    "blank-node isomorphism budget exhausted before a verdict (unverified)"
                )
            }
        }
    }
}

impl std::error::Error for CompareError {}

/// A named parser under differential test: (text, base IRI) in, rendered quad set out.
/// The base IRI is the SAME for both sides of a comparison (the corpus walkers pass the
/// input file's own `file://` IRI), so formats without relative IRIs simply ignore it.
pub struct DiffParser<'a> {
    /// Display name (report + repro labelling).
    pub name: &'a str,
    /// The parse function. `[s, p, o, g]` rendered quads (empty `g` = default graph).
    #[allow(clippy::type_complexity)]
    pub parse: Box<dyn Fn(&str, &str) -> Result<Vec<[String; 4]>, String> + 'a>,
}

impl<'a> DiffParser<'a> {
    /// Convenience constructor.
    pub fn new(
        name: &'a str,
        parse: impl Fn(&str, &str) -> Result<Vec<[String; 4]>, String> + 'a,
    ) -> Self {
        DiffParser {
            name,
            parse: Box::new(parse),
        }
    }
}

/// How a candidate diverged from the incumbent on one input.
#[derive(Debug)]
pub enum DivergenceKind {
    /// Candidate rejected a document the incumbent accepts (the candidate's error).
    CandidateRejects(String),
    /// Candidate accepted a document the incumbent rejects (the incumbent's error).
    CandidateAccepts(String),
    /// Both accepted but produced different quad sets (bounded unmatched samples).
    QuadSet {
        /// Sample of quads only the candidate produced.
        only_candidate: Vec<[String; 4]>,
        /// Sample of quads only the incumbent produced.
        only_incumbent: Vec<[String; 4]>,
    },
}

/// One detected divergence, with its input label and a minimal(ised) repro document.
#[derive(Debug)]
pub struct Divergence {
    /// Where the diverging input came from (file path or synthetic label).
    pub label: String,
    /// What diverged.
    pub kind: DivergenceKind,
    /// The smallest line-subset of the input that still diverges (see module docs).
    pub minimal_repro: String,
}

/// Aggregate result of a differential run over a corpus.
#[derive(Debug, Default)]
pub struct DiffReport {
    /// Inputs compared.
    pub compared: usize,
    /// Inputs on which candidate and incumbent fully agreed.
    pub agreements: usize,
    /// Inputs whose quad-set comparison exhausted the isomorphism budget (labels).
    pub unverified: Vec<String>,
    /// Detected divergences.
    pub divergences: Vec<Divergence>,
}

impl DiffReport {
    /// Fold one compared input into the tallies.
    fn record(
        &mut self,
        label: &str,
        text: &str,
        base: &str,
        candidate: &DiffParser,
        incumbent: &DiffParser,
        outcome: Result<Option<DivergenceKind>, CompareError>,
    ) {
        self.compared += 1;
        match outcome {
            Ok(None) => self.agreements += 1,
            Ok(Some(kind)) => {
                let minimal_repro = shrink_repro(candidate, incumbent, text, base);
                self.divergences.push(Divergence {
                    label: label.to_string(),
                    kind,
                    minimal_repro,
                });
            }
            Err(CompareError::BudgetExhausted) => self.unverified.push(label.to_string()),
        }
    }

    /// Human-readable summary of the divergences (for assertion messages).
    pub fn describe(&self) -> String {
        let mut s = format!(
            "{} compared, {} agree, {} unverified, {} divergences",
            self.compared,
            self.agreements,
            self.unverified.len(),
            self.divergences.len()
        );
        for d in self.divergences.iter().take(5) {
            s.push_str(&format!(
                "\n- {}: {:?}\n  minimal repro ({} bytes): {:?}",
                d.label,
                d.kind,
                d.minimal_repro.len(),
                &d.minimal_repro[..d.minimal_repro.len().min(400)]
            ));
        }
        s
    }
}

/// Compare candidate vs incumbent on ONE document. `Ok(None)` = agreement (both reject,
/// or both accept with bijection-identical quad sets); `Ok(Some(_))` = divergence;
/// `Err(CompareError::BudgetExhausted)` = isomorphism budget exhausted (unverified).
pub fn compare_doc(
    candidate: &DiffParser,
    incumbent: &DiffParser,
    text: &str,
    base: &str,
) -> Result<Option<DivergenceKind>, CompareError> {
    match ((candidate.parse)(text, base), (incumbent.parse)(text, base)) {
        (Ok(c), Ok(i)) => match compare_quad_sets(&c, &i) {
            SetCompare::Equal => Ok(None),
            SetCompare::Different { only_a, only_b } => Ok(Some(DivergenceKind::QuadSet {
                only_candidate: only_a,
                only_incumbent: only_b,
            })),
            SetCompare::Unverified => Err(CompareError::BudgetExhausted),
        },
        (Err(e), Ok(_)) => Ok(Some(DivergenceKind::CandidateRejects(e))),
        (Ok(_), Err(e)) => Ok(Some(DivergenceKind::CandidateAccepts(e))),
        (Err(_), Err(_)) => Ok(None), // both reject: agreement on the verdict
    }
}

fn is_divergent(candidate: &DiffParser, incumbent: &DiffParser, text: &str, base: &str) -> bool {
    matches!(compare_doc(candidate, incumbent, text, base), Ok(Some(_)))
}

/// Greedy line-set shrink (ddmin-lite) of a diverging document: repeatedly try dropping
/// chunks of lines (halving chunk size down to single lines), keeping any subset that
/// still diverges, under a bounded number of re-comparisons. Sound (returns a diverging
/// document, the original at worst), minimal for most line-oriented repros.
pub fn shrink_repro(
    candidate: &DiffParser,
    incumbent: &DiffParser,
    text: &str,
    base: &str,
) -> String {
    const EVAL_BUDGET: usize = 400;
    let mut evals = 0usize;
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 1 {
        return text.to_string();
    }
    let rejoin = |ls: &[&str]| -> String {
        let mut s = ls.join("\n");
        s.push('\n');
        s
    };
    // Fast path: a single line alone that still diverges.
    for &l in &lines {
        if evals >= EVAL_BUDGET {
            break;
        }
        evals += 1;
        let doc = format!("{l}\n");
        if is_divergent(candidate, incumbent, &doc, base) {
            return doc;
        }
    }
    // ddmin-lite: drop windows of `chunk` lines while the remainder still diverges,
    // halving the window down to single lines; repeat single-line passes to a fixpoint.
    let mut chunk = (lines.len().div_ceil(2)).max(1);
    loop {
        let mut i = 0;
        let mut shrunk = false;
        while i < lines.len() && evals < EVAL_BUDGET {
            let mut trial: Vec<&str> = Vec::with_capacity(lines.len().saturating_sub(chunk));
            trial.extend_from_slice(&lines[..i]);
            trial.extend_from_slice(&lines[(i + chunk).min(lines.len())..]);
            if trial.is_empty() {
                i += chunk;
                continue;
            }
            evals += 1;
            if is_divergent(candidate, incumbent, &rejoin(&trial), base) {
                lines = trial;
                shrunk = true;
                // Do not advance i: the lines that slid into position i are untested.
            } else {
                i += chunk;
            }
        }
        if evals >= EVAL_BUDGET || (chunk == 1 && !shrunk) {
            break;
        }
        if chunk > 1 {
            chunk = (chunk / 2).max(1).min(lines.len().max(1));
        }
    }
    rejoin(&lines)
}

/// Run the differential over every file in `dir` (recursively) whose path satisfies
/// `filter`. Non-UTF-8 bytes are replaced (both parsers see the SAME lossy text, so the
/// comparison stays sound). Returns the aggregate report.
pub fn run_dir(
    candidate: &DiffParser,
    incumbent: &DiffParser,
    dir: &Path,
    filter: &dyn Fn(&Path) -> bool,
) -> DiffReport {
    let mut report = DiffReport::default();
    let mut stack = vec![dir.to_path_buf()];
    let mut files = Vec::new();
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if filter(&p) {
                files.push(p);
            }
        }
    }
    files.sort();
    for p in files {
        let Ok(bytes) = std::fs::read(&p) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let base = crate::rdf::file_iri(&p);
        let outcome = compare_doc(candidate, incumbent, &text, &base);
        report.record(
            &p.display().to_string(),
            &text,
            &base,
            candidate,
            incumbent,
            outcome,
        );
    }
    report
}

/// Run the differential over every `mf:action` file of a W3C suite manifest — the
/// suite-driven corpus mode (positive AND negative actions both count: on a negative
/// action the two parsers must AGREE to reject).
pub fn run_suite_actions(
    candidate: &DiffParser,
    incumbent: &DiffParser,
    suite_root: &Path,
) -> Result<DiffReport, String> {
    let manifest = suite_root.join("manifest.ttl");
    let g = crate::rdf::MiniGraph::load(&manifest)?;
    let mf_action = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#action";
    let mut report = DiffReport::default();
    let mut seen = std::collections::HashSet::new();
    for t in &g.triples {
        if t.predicate.as_str() != mf_action {
            continue;
        }
        let oxrdf::Term::NamedNode(n) = &t.object else {
            continue;
        };
        let Some(path) = crate::rdf::iri_to_path(n.as_str()) else {
            continue;
        };
        if !seen.insert(path.clone()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let base = crate::rdf::file_iri(&path);
        let outcome = compare_doc(candidate, incumbent, &text, &base);
        report.record(
            &path.display().to_string(),
            &text,
            &base,
            candidate,
            incumbent,
            outcome,
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oxttl_nt(name: &str) -> DiffParser<'_> {
        DiffParser::new(name, |text, _base| {
            let mut quads = Vec::new();
            for t in oxttl::NTriplesParser::new().for_slice(text.as_bytes()) {
                let t = t.map_err(|e| e.to_string())?;
                quads.push(crate::quadset::quad_strings(&oxrdf::Quad::new(
                    t.subject,
                    t.predicate,
                    t.object,
                    oxrdf::GraphName::DefaultGraph,
                )));
            }
            Ok(quads)
        })
    }

    #[test]
    fn identical_parsers_agree() {
        let a = oxttl_nt("a");
        let b = oxttl_nt("b");
        let doc = "<http://ex/s> <http://ex/p> \"v\" .\n";
        assert!(matches!(compare_doc(&a, &b, doc, "http://ex/"), Ok(None)));
        // Both-reject is also agreement.
        assert!(matches!(
            compare_doc(&a, &b, "not rdf\n", "http://ex/"),
            Ok(None)
        ));
    }

    #[test]
    fn shrink_finds_single_diverging_line() {
        // Mutant: silently DROPS any triple whose object literal contains "x".
        let incumbent = oxttl_nt("incumbent");
        let mutant = DiffParser::new("mutant", |text, base| {
            (oxttl_nt("i").parse)(text, base)
                .map(|qs| qs.into_iter().filter(|q| !q[2].contains('x')).collect())
        });
        let mut doc = String::new();
        for i in 0..40 {
            doc.push_str(&format!("<http://ex/s{i}> <http://ex/p> \"v{i}\" .\n"));
        }
        doc.push_str("<http://ex/s> <http://ex/p> \"x-marks-the-spot\" .\n");
        let repro = shrink_repro(&mutant, &incumbent, &doc, "http://ex/");
        assert_eq!(
            repro,
            "<http://ex/s> <http://ex/p> \"x-marks-the-spot\" .\n"
        );
    }
}
