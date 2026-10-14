//! Exec-accuracy harness for the NL→SPARQL loop — the design doc's `sparq-nlq`
//! accuracy gate (`research/genai-design.md` §4): exec-accuracy on a QALD-style set,
//! **oracle-linking vs end-to-end reported separately** AND **≥ the same LLM without
//! grounding** ("the grounding must pay for itself"). sq-05rv
//!
//! ## What "exec-accuracy" means here
//!
//! Following the QALD / Text2SPARQL convention (`research/genai-nl-to-sparql.md` §1,
//! §4.2): grade on the **answer set**, not query-string equality. We execute both the
//! candidate query (from the loop) and the gold query on the **same `sparq` graph**,
//! then take precision / recall / F1 over the resulting bind-row sets. Executing the
//! gold query against the same instance is the doc's recipe for eliminating endpoint
//! drift (§4.3): the gold is a query, the answer set is recomputed locally, so there
//! is no checked-in answer blob to go stale.
//!
//! ## The two axes the design doc asks to be reported separately
//!
//! 1. **Linking** — [`Linking::EndToEnd`] (the model writes the whole query from the
//!    grounding prompt) vs [`Linking::Oracle`] (the schema/entity linking is given:
//!    the harness supplies the correct query directly, isolating the loop's
//!    *validate → execute → repair* contribution from the model's linking ability).
//! 2. **Grounding** — grounded (`NlqConfig::ground` `true`) vs the ungrounded
//!    baseline (`false`). The headline claim is *grounded end-to-end* F1 **>**
//!    *ungrounded end-to-end* F1.
//!
//! ## Offline by construction
//!
//! Nothing in this module touches the network. It drives [`crate::Nlq`] over any
//! [`crate::Llm`]; in CI that is a [`crate::ReplayLlm`] (recorded fixture) or a
//! scripted in-memory backend. The *live* run (a real model, `--features live` +
//! [`crate::RecordingLlm`]) is wired by the caller and is `#[ignore]`'d / feature-
//! gated OFF in the test suite — see `tests/exec_accuracy.rs`.

use crate::Nlq;
use sparq_engine::{query_with_budget, QueryBudget, QueryResult};

/// Which linking regime a case is scored under — reported separately per the design
/// doc (`oracle-linking vs end-to-end separately`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Linking {
    /// The model writes the whole query from the (grounded or ungrounded) prompt.
    EndToEnd,
    /// Schema/entity linking is given: the harness feeds the correct query straight
    /// into validate→execute, isolating the engine-side loop from model linking.
    Oracle,
}

/// One evaluation item: a question and the **gold SPARQL** whose answer set is the
/// ground truth. The gold is executed on the same graph at scoring time (no checked-in
/// answer blob — `research/genai-nl-to-sparql.md` §4.3).
#[derive(Debug, Clone)]
pub struct EvalCase {
    pub question: String,
    /// Gold query; executed on the eval graph to materialise the gold answer set.
    pub gold_sparql: String,
}

impl EvalCase {
    pub fn new(question: impl Into<String>, gold_sparql: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            gold_sparql: gold_sparql.into(),
        }
    }
}

/// Precision / recall / F1 over two answer sets, with the raw set sizes for auditing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F1 {
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    /// `|predicted ∩ gold|`.
    pub intersection: usize,
    /// `|predicted|`.
    pub predicted: usize,
    /// `|gold|`.
    pub gold: usize,
}

impl F1 {
    /// The QALD edge convention: when both sets are empty the system answered
    /// correctly (a true negative — an `ASK` that is false, or an empty SELECT that
    /// is genuinely empty), so P = R = F1 = 1. When exactly one side is empty the
    /// other metrics fall out of the ratios (the empty side drives its ratio to 0).
    pub fn score(predicted: &AnswerSet, gold: &AnswerSet) -> F1 {
        let p = predicted.len();
        let g = gold.len();
        if p == 0 && g == 0 {
            return F1 {
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                intersection: 0,
                predicted: 0,
                gold: 0,
            };
        }
        let inter = predicted.intersection_count(gold);
        let precision = if p == 0 { 0.0 } else { inter as f64 / p as f64 };
        let recall = if g == 0 { 0.0 } else { inter as f64 / g as f64 };
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };
        F1 {
            precision,
            recall,
            f1,
            intersection: inter,
            predicted: p,
            gold: g,
        }
    }

    /// A correct answer is an exact set match — the strict `EM`-on-answers reading,
    /// reported alongside F1.
    pub fn is_exact(&self) -> bool {
        self.intersection == self.predicted && self.intersection == self.gold
    }
}

/// A SPARQL answer as a **multiset of rows**, each row canonicalised to a vector of
/// stringified `Option<Term>` cells (the engine's `Term` has a stable RDF-term
/// `Display`). Two answers compare equal iff they bind the same rows, independent of
/// row order — the QALD answer-set semantics.
///
/// `SELECT` → one entry per result row. `ASK` → the engine's unit-row encoding (zero
/// vars; one empty row iff true), which scores naturally: a true `ASK` is a one-row
/// set, a false `ASK` the empty set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnswerSet {
    /// Sorted canonical row strings (multiset preserved by keeping duplicates).
    rows: Vec<String>,
}

impl AnswerSet {
    /// Canonicalise a [`QueryResult`] into a comparable answer set.
    pub fn from_result(result: &QueryResult) -> AnswerSet {
        let mut rows: Vec<String> = result.rows.iter().map(|r| canon_row(r)).collect();
        rows.sort_unstable();
        AnswerSet { rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// `|self ∩ other|` as multisets — pairs each `self` row with at most one equal
    /// `other` row (a greedy merge over the two sorted vectors).
    fn intersection_count(&self, other: &AnswerSet) -> usize {
        let (mut i, mut j, mut n) = (0, 0, 0);
        while i < self.rows.len() && j < other.rows.len() {
            match self.rows[i].cmp(&other.rows[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    n += 1;
                    i += 1;
                    j += 1;
                }
            }
        }
        n
    }
}

/// Canonical string for one result row. Variable *names* do not affect the answer set
/// (QALD scores values, not projection labels), so a row is just its cells joined with
/// a unit separator that cannot occur inside an RDF term's `Display`. Generic over the
/// cell type to avoid naming the engine's (un-re-exported) `Term` — any `Display` cell
/// works, and the engine's `Term` is `Display`.
fn canon_row<T: std::fmt::Display>(row: &[Option<T>]) -> String {
    let mut s = String::new();
    for (i, cell) in row.iter().enumerate() {
        if i > 0 {
            s.push('\u{1f}'); // ASCII unit separator — not producible by Term::Display.
        }
        match cell {
            Some(t) => s.push_str(&t.to_string()),
            None => s.push_str("\u{1e}UNBOUND"), // record separator marks an unbound cell.
        }
    }
    s
}

/// The scored part of one case: the F1, the query that produced it, and the repair
/// rounds it cost. (`Answer` is not `Clone` — it carries the materialised result — so
/// the report keeps only the auditable summary, not the whole answer.)
#[derive(Debug, Clone)]
pub struct CaseScore {
    pub f1: F1,
    /// The query that produced the scored answer (post-repair, if any).
    pub sparql: String,
    pub repairs: usize,
}

/// One scored case in a [`Report`].
#[derive(Debug, Clone)]
pub struct CaseResult {
    pub question: String,
    /// `Ok` is a produced-and-executed query with its score; `Err` is a loop failure
    /// (no parseable/executable query after the repair budget) — scored as F1 = 0
    /// against a non-empty gold, the honest cost of a hard failure.
    pub outcome: Result<CaseScore, String>,
}

impl CaseResult {
    /// The F1 to aggregate: the produced query's F1, or 0 on a loop failure (unless
    /// the gold itself is empty, in which case a "no answer" is the right answer and
    /// scores 1 — the same true-negative convention as [`F1::score`]).
    pub fn f1(&self, gold_empty: bool) -> f64 {
        match &self.outcome {
            Ok(s) => s.f1.f1,
            Err(_) => {
                if gold_empty {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// Aggregate over one (linking, grounding) configuration.
#[derive(Debug, Clone)]
pub struct Report {
    pub linking: Linking,
    pub grounded: bool,
    pub cases: Vec<CaseResult>,
    /// Macro-averaged F1 (mean of per-case F1) — the QALD headline metric.
    pub macro_f1: f64,
    /// Fraction of cases whose produced answer set exactly equals gold.
    pub exact_match: f64,
    /// Fraction of cases that produced a parseable+executable query at all (validity).
    pub validity: f64,
    /// Total repair rounds consumed across the set.
    pub total_repairs: usize,
}

impl Report {
    pub fn case_count(&self) -> usize {
        self.cases.len()
    }
}

/// Runs one (linking, grounding) configuration over the cases against `graph`,
/// executing each gold query on the same `graph` to score answer-set F1.
///
/// `llm` is whatever backend the caller wired (replay fixture in CI, a live recorder
/// for the real run). For [`Linking::Oracle`] the `llm` is **not consulted** — the
/// gold query is fed straight through validate→execute, so an oracle run needs no
/// fixture and isolates the engine-side loop.
pub fn run_config(
    graph: &sparq_core::Graph,
    cases: &[EvalCase],
    llm: Box<dyn crate::Llm>,
    config: crate::NlqConfig,
    linking: Linking,
) -> Report {
    let grounded = config.ground;
    let nlq = Nlq::with_config(graph, llm, config);
    let budget = oracle_budget();

    let mut results = Vec::with_capacity(cases.len());
    let mut sum_f1 = 0.0;
    let mut exact = 0usize;
    let mut valid = 0usize;
    let mut total_repairs = 0usize;

    for case in cases {
        // Gold answer set: execute the gold query on the same graph.
        let gold_set = match query_with_budget(graph, &case.gold_sparql, &budget) {
            Ok(r) => AnswerSet::from_result(&r),
            // A gold query that does not execute is a harness authoring error, not a
            // model failure — surface it loudly rather than silently scoring 0.
            Err(e) => {
                results.push(CaseResult {
                    question: case.question.clone(),
                    outcome: Err(format!("GOLD query failed to execute: {e}")),
                });
                continue;
            }
        };
        let gold_empty = gold_set.is_empty();

        let outcome: Result<CaseScore, String> = match linking {
            // Oracle linking: the correct query is given; run it through the same
            // validate→execute path the loop uses (proves the engine side end-to-end).
            Linking::Oracle => match query_with_budget(graph, &case.gold_sparql, &budget) {
                Ok(r) => Ok(CaseScore {
                    f1: F1::score(&AnswerSet::from_result(&r), &gold_set),
                    sparql: case.gold_sparql.clone(),
                    repairs: 0,
                }),
                Err(e) => Err(e),
            },
            // End-to-end: the model writes the query from the prompt.
            Linking::EndToEnd => match nlq.ask(&case.question) {
                Ok(answer) => Ok(CaseScore {
                    f1: F1::score(&AnswerSet::from_result(&answer.result), &gold_set),
                    sparql: answer.sparql,
                    repairs: answer.repairs,
                }),
                Err(e) => Err(e.to_string()),
            },
        };

        match &outcome {
            Ok(score) => {
                sum_f1 += score.f1.f1;
                if score.f1.is_exact() {
                    exact += 1;
                }
                valid += 1;
                total_repairs += score.repairs;
            }
            Err(_) => {
                // Loop/gold failure: true-negative convention if gold is empty too.
                sum_f1 += if gold_empty { 1.0 } else { 0.0 };
            }
        }

        results.push(CaseResult {
            question: case.question.clone(),
            outcome,
        });
    }

    let n = cases.len().max(1) as f64;
    Report {
        linking,
        grounded,
        macro_f1: sum_f1 / n,
        exact_match: exact as f64 / n,
        validity: valid as f64 / n,
        total_repairs,
        cases: results,
    }
}

/// The four-cell comparison the design doc requires: end-to-end and oracle, each
/// grounded and ungrounded. Returned as a struct so the caller can assert the
/// load-bearing inequality (`headline_grounding_pays`).
#[derive(Debug, Clone)]
pub struct Comparison {
    pub grounded_end_to_end: Report,
    pub ungrounded_end_to_end: Report,
    pub grounded_oracle: Report,
    pub ungrounded_oracle: Report,
}

impl Comparison {
    /// The headline claim from `research/genai-design.md` §4: grounded end-to-end
    /// exec-accuracy must be **strictly greater** than the same LLM ungrounded —
    /// "the grounding must pay for itself". Returns `true` iff that holds for macro-F1.
    pub fn headline_grounding_pays(&self) -> bool {
        self.grounded_end_to_end.macro_f1 > self.ungrounded_end_to_end.macro_f1
    }

    /// A one-line-per-row summary table for stderr / a benchmark log (no perf numbers
    /// baked into committed markdown — printed at run time only).
    pub fn summary(&self) -> String {
        let row = |r: &Report| {
            format!(
                "  {:<10} grounded={:<5} macroF1={:.3} EM={:.3} validity={:.3} repairs={}",
                format!("{:?}", r.linking),
                r.grounded,
                r.macro_f1,
                r.exact_match,
                r.validity,
                r.total_repairs,
            )
        };
        format!(
            "exec-accuracy ({} cases):\n{}\n{}\n{}\n{}\n  grounding pays for itself: {}",
            self.grounded_end_to_end.case_count(),
            row(&self.grounded_end_to_end),
            row(&self.ungrounded_end_to_end),
            row(&self.grounded_oracle),
            row(&self.ungrounded_oracle),
            self.headline_grounding_pays(),
        )
    }
}

/// Drives all four configurations. `make_llm` is called once per end-to-end config
/// (a fresh backend each time — `ReplayLlm` is consumed by the loop; the closure lets
/// the caller hand out independent fixtures for the grounded vs ungrounded prompts,
/// which are distinct strings). Oracle configs do not consult the LLM, so they are
/// driven with a backend that always errors.
pub fn run_comparison(
    graph: &sparq_core::Graph,
    cases: &[EvalCase],
    base_config: &crate::NlqConfig,
    mut make_llm: impl FnMut(bool) -> Box<dyn crate::Llm>,
) -> Comparison {
    let cfg = |ground: bool| crate::NlqConfig {
        ground,
        ..base_config.clone()
    };
    let grounded_end_to_end =
        run_config(graph, cases, make_llm(true), cfg(true), Linking::EndToEnd);
    let ungrounded_end_to_end =
        run_config(graph, cases, make_llm(false), cfg(false), Linking::EndToEnd);
    let grounded_oracle = run_config(graph, cases, Box::new(NoLlm), cfg(true), Linking::Oracle);
    let ungrounded_oracle = run_config(graph, cases, Box::new(NoLlm), cfg(false), Linking::Oracle);
    Comparison {
        grounded_end_to_end,
        ungrounded_end_to_end,
        grounded_oracle,
        ungrounded_oracle,
    }
}

/// Per-query budget for gold/oracle execution: bounded like the loop's default but a
/// little longer-lived (gold queries are trusted but may aggregate over a big graph).
fn oracle_budget() -> QueryBudget {
    let mut b = QueryBudget::unlimited();
    b.max_rows = Some(1_000_000);
    #[cfg(not(target_arch = "wasm32"))]
    {
        b.deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
    }
    b
}

/// An [`crate::Llm`] that always errors — used to drive [`Linking::Oracle`] runs,
/// which never call the model (so any call is a harness bug worth surfacing).
struct NoLlm;
impl crate::Llm for NoLlm {
    fn complete(&self, _prompt: &str) -> Result<String, String> {
        Err("oracle-linking run must not consult the LLM".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Llm, NlqConfig};
    use sparq_core::Graph;

    fn tiny_graph() -> Graph {
        let ttl = r#"
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:alice a ex:Person ; rdfs:label "Alice" ; ex:knows ex:bob .
            ex:bob a ex:Person ; rdfs:label "Bob" .
            ex:acme a ex:Company ; rdfs:label "Acme" .
        "#;
        Graph::load_str(ttl, "turtle").expect("tiny graph parses")
    }

    /// Closure-backed backend.
    struct FnLlm<F: Fn(&str) -> Result<String, String>>(F);
    impl<F: Fn(&str) -> Result<String, String>> Llm for FnLlm<F> {
        fn complete(&self, prompt: &str) -> Result<String, String> {
            (self.0)(prompt)
        }
    }

    const PEOPLE: &str = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
         PREFIX ex: <http://example.org/>\n\
         SELECT ?p WHERE { ?p rdf:type ex:Person }";

    #[test]
    fn answer_set_is_order_independent() {
        let g = tiny_graph();
        let a = query_with_budget(&g, PEOPLE, &oracle_budget()).unwrap();
        let set = AnswerSet::from_result(&a);
        assert_eq!(set.len(), 2);
        // Same query → identical canonical set regardless of internal row order.
        let b = query_with_budget(&g, PEOPLE, &oracle_budget()).unwrap();
        assert_eq!(set, AnswerSet::from_result(&b));
    }

    #[test]
    fn f1_perfect_partial_and_disjoint() {
        let g = tiny_graph();
        let people =
            AnswerSet::from_result(&query_with_budget(&g, PEOPLE, &oracle_budget()).unwrap());

        // Perfect overlap.
        let perfect = F1::score(&people, &people);
        assert!(perfect.is_exact());
        assert_eq!(perfect.f1, 1.0);

        // Predicted is a subset (one of the two people) → recall 0.5, precision 1.
        let alice = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?p WHERE { ?p rdf:type ex:Person FILTER(?p = ex:alice) }";
        let alice_set =
            AnswerSet::from_result(&query_with_budget(&g, alice, &oracle_budget()).unwrap());
        let partial = F1::score(&alice_set, &people);
        assert_eq!(partial.precision, 1.0);
        assert_eq!(partial.recall, 0.5);
        assert!((partial.f1 - 2.0 / 3.0).abs() < 1e-9);
        assert!(!partial.is_exact());

        // Disjoint: companies vs people → F1 0.
        let companies = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?c WHERE { ?c rdf:type ex:Company }";
        let comp_set =
            AnswerSet::from_result(&query_with_budget(&g, companies, &oracle_budget()).unwrap());
        let disjoint = F1::score(&comp_set, &people);
        assert_eq!(disjoint.f1, 0.0);
    }

    #[test]
    fn empty_vs_empty_is_a_true_negative() {
        let empty = AnswerSet::default();
        let f1 = F1::score(&empty, &empty);
        assert_eq!(f1.f1, 1.0);
        assert!(f1.is_exact());
    }

    /// The ASYMMETRIC empty cases — only one of the two sides is empty — which the
    /// perfect/partial/disjoint and empty-vs-empty tests never reach. An empty PREDICTED
    /// against a non-empty gold is a total miss (recall 0); a non-empty predicted against
    /// an empty gold is a total false-positive (precision 0). Both score F1 0 and are NOT
    /// exact. This is the `p == 0` / `g == 0` ratio guard in `F1::score`.
    #[test]
    fn f1_one_side_empty_scores_zero_and_is_not_exact() {
        let g = tiny_graph();
        let people =
            AnswerSet::from_result(&query_with_budget(&g, PEOPLE, &oracle_budget()).unwrap());
        let empty = AnswerSet::default();
        assert!(!people.is_empty());

        // Predicted nothing, gold has rows → recall 0, precision 0 (no intersection).
        let miss = F1::score(&empty, &people);
        assert_eq!(miss.precision, 0.0);
        assert_eq!(miss.recall, 0.0);
        assert_eq!(miss.f1, 0.0);
        assert_eq!(miss.intersection, 0);
        assert_eq!(miss.predicted, 0);
        assert_eq!(miss.gold, people.len());
        assert!(!miss.is_exact());

        // Predicted rows, gold is empty → over-answered: precision 0, recall 0.
        let over = F1::score(&people, &empty);
        assert_eq!(over.precision, 0.0);
        assert_eq!(over.recall, 0.0);
        assert_eq!(over.f1, 0.0);
        assert_eq!(over.predicted, people.len());
        assert_eq!(over.gold, 0);
        assert!(!over.is_exact());
    }

    /// Answer-set comparison is a MULTISET — duplicate rows are preserved, so a query that
    /// returns a row twice does NOT collapse it. Guards the `intersection_count` greedy
    /// merge over duplicates and the `from_result` dup-keeping.
    #[test]
    fn answer_set_is_a_multiset_over_duplicate_rows() {
        let g = tiny_graph();
        // UNION of the same person twice yields two identical rows (no DISTINCT).
        let dup_q = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?p WHERE { { ?p rdf:type ex:Person FILTER(?p = ex:alice) } \
             UNION { ?p rdf:type ex:Person FILTER(?p = ex:alice) } }";
        let dup = AnswerSet::from_result(&query_with_budget(&g, dup_q, &oracle_budget()).unwrap());
        assert_eq!(dup.len(), 2, "the duplicate row is kept, not collapsed");

        // The single-row version (alice once) intersects the multiset in exactly one row,
        // so precision against the 2-row predicted set is 1/2.
        let single_q = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
             PREFIX ex: <http://example.org/>\n\
             SELECT ?p WHERE { ?p rdf:type ex:Person FILTER(?p = ex:alice) }";
        let single =
            AnswerSet::from_result(&query_with_budget(&g, single_q, &oracle_budget()).unwrap());
        assert_eq!(single.len(), 1);
        let f1 = F1::score(&dup, &single);
        assert_eq!(f1.intersection, 1, "one of the two duplicate rows matches");
        assert_eq!(f1.precision, 0.5);
        assert_eq!(f1.recall, 1.0);
    }

    #[test]
    fn oracle_linking_scores_perfect_without_the_llm() {
        let g = tiny_graph();
        let cases = vec![EvalCase::new("how many people", PEOPLE)];
        // NoLlm would error if consulted — oracle must not consult it.
        let report = run_config(
            &g,
            &cases,
            Box::new(NoLlm),
            NlqConfig::default(),
            Linking::Oracle,
        );
        assert_eq!(report.macro_f1, 1.0);
        assert_eq!(report.exact_match, 1.0);
        assert_eq!(report.validity, 1.0);
    }

    #[test]
    fn end_to_end_grounded_beats_ungrounded() {
        let g = tiny_graph();
        let cases = vec![EvalCase::new("Who are the people?", PEOPLE)];

        // Grounded model: answers correctly (it has the schema deck). Ungrounded
        // model: guesses a wrong predicate, producing a valid-but-empty answer set
        // (F1 0 against the non-empty gold). This is the "grounding pays" shape.
        let comparison = run_comparison(&g, &cases, &NlqConfig::default(), |ground| {
            if ground {
                Box::new(FnLlm(|_| Ok(format!("```sparql\n{PEOPLE}\n```"))))
            } else {
                Box::new(FnLlm(|_| {
                    Ok("```sparql\nPREFIX ex: <http://example.org/>\n\
                        SELECT ?p WHERE { ?p ex:nonexistentPredicate ?x }\n```"
                        .into())
                }))
            }
        });

        assert_eq!(comparison.grounded_end_to_end.macro_f1, 1.0);
        assert_eq!(comparison.ungrounded_end_to_end.macro_f1, 0.0);
        assert!(comparison.headline_grounding_pays());
        // Oracle is perfect under both grounding settings (linking is given).
        assert_eq!(comparison.grounded_oracle.macro_f1, 1.0);
        assert_eq!(comparison.ungrounded_oracle.macro_f1, 1.0);
        // The summary renders without panicking and names the headline verdict.
        assert!(comparison
            .summary()
            .contains("grounding pays for itself: true"));
    }

    #[test]
    fn loop_failure_scores_zero_against_nonempty_gold() {
        let g = tiny_graph();
        let cases = vec![EvalCase::new("unanswerable here", PEOPLE)];
        let report = run_config(
            &g,
            &cases,
            // Never produces SPARQL → loop fails after the repair budget.
            Box::new(FnLlm(|_| Ok("I cannot help with that.".into()))),
            NlqConfig::default(),
            Linking::EndToEnd,
        );
        assert_eq!(report.macro_f1, 0.0);
        assert_eq!(report.validity, 0.0);
        assert!(report.cases[0].outcome.is_err());
    }

    #[test]
    fn ungrounded_prompt_omits_the_schema_summary() {
        let g = tiny_graph();
        let grounded = Nlq::with_config(
            &g,
            Box::new(NoLlm),
            NlqConfig {
                ground: true,
                ..NlqConfig::default()
            },
        );
        let ungrounded = Nlq::with_config(
            &g,
            Box::new(NoLlm),
            NlqConfig {
                ground: false,
                ..NlqConfig::default()
            },
        );
        let q = "Who are the people?";
        assert!(grounded.prompt_for(q).contains("# Schema summary"));
        assert!(!ungrounded.prompt_for(q).contains("# Schema summary"));
        // Both still carry the question and the few-shot examples.
        assert!(ungrounded
            .prompt_for(q)
            .ends_with("Question: Who are the people?"));
        assert!(ungrounded
            .prompt_for(q)
            .contains("How many entities of each type are there?"));
    }
}
