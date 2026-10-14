//! Oracle self-tests — the crate's non-vacuity + fail-closed invariant. sq-gum8.6
//!
//! * **Non-vacuity (mutation check):** a deliberately-injected wrong-result mutant
//!   (`FilterDropsRow`, which silently removes a row from any `FILTER` query's result)
//!   must be flagged by the TLP, NoREC, AND differential oracles. These tests run the
//!   REAL sparq engine (`InProcessSparq` over `sparq_engine::query_json`), not a mock
//!   evaluator.
//! * **Fail-closed:** an engine error (invalid query, failing driver) must yield
//!   `Verdict::EngineFailure` — never `Pass`, never `Violation` — keeping the
//!   wrong-result / engine-error classes strictly separated.
//! * **Three-branch coverage:** the fixed dataset exercises all three EBV outcomes —
//!   true, false, and error via BOTH error causes (a type error from an incomparable
//!   bound value, and an unbound OPTIONAL variable).

use sparq_difftest::QueryResults;
use sparq_metamorph::engine::{FilterDropsRow, InProcessSparq, SparqlEngine};
use sparq_metamorph::verdict::{FailureKind, Verdict};
use sparq_metamorph::{
    check_differential, check_differential_ordered, check_norec, check_tlp, check_tlp_aggregate,
    check_tlp_distinct, generate_case, norec_queries, ordered_query, tlp_distinct_queries,
    tlp_queries, PartitionedAggregate,
};

/// Ages straddle the predicate `?age < 25` (true for s2, false for s1), s3's age is a
/// string (type error under `<`), and s4 has an `ex:name` but NO `ex:age` — so the
/// OPTIONAL pattern leaves `?age` unbound (unbound-evaluation error).
const DATA: &str = concat!(
    "<http://example.org/s1> <http://example.org/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s1> <http://example.org/name> \"alice\" .\n",
    "<http://example.org/s2> <http://example.org/age> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s2> <http://example.org/name> \"bob\" .\n",
    "<http://example.org/s3> <http://example.org/age> \"twenty\" .\n",
    "<http://example.org/s3> <http://example.org/name> \"carol\" .\n",
    "<http://example.org/s4> <http://example.org/name> \"dave\" .\n",
);

/// A pattern whose OPTIONAL leaves `?age` unbound for s4.
const PATTERN: &str =
    "?s <http://example.org/name> ?n . OPTIONAL { ?s <http://example.org/age> ?age }";
/// True for s2 (20), false for s1 (30), type error for s3 ("twenty"), unbound error for s4.
const PREDICATE: &str = "?age < 25";

fn pristine() -> InProcessSparq {
    InProcessSparq::from_ntriples("sparq", DATA).expect("test data loads")
}

fn rows(engine: &dyn SparqlEngine, query: &str) -> usize {
    match engine.select(query).expect("query evaluates") {
        QueryResults::Solutions { solutions, .. } => solutions.len(),
        QueryResults::Boolean(_) => panic!("SELECT returned a boolean"),
    }
}

// sq-aglhv: independently guard the row-preserving Extend/Project step on
// which NoREC's relative optimized-count versus true-flag-count comparison relies.
fn norec_rewrite_preserves_cardinality(
    engine: &dyn SparqlEngine,
    pattern: &str,
    predicate: &str,
) -> bool {
    let base = format!("SELECT * WHERE {{ {pattern} }}");
    let rewritten = norec_queries(pattern, predicate).rewritten;
    rows(engine, &rewritten) == rows(engine, &base)
}

/// A test-only mutant that removes one row only from NoREC's FILTER-free rewrite.
struct RewriteDropsRow<E> {
    inner: E,
}

impl<E> RewriteDropsRow<E> {
    fn new(inner: E) -> Self {
        Self { inner }
    }
}

impl<E: SparqlEngine> SparqlEngine for RewriteDropsRow<E> {
    fn name(&self) -> &str {
        "rewrite-drops-row"
    }

    fn select(&self, sparql: &str) -> Result<QueryResults, sparq_metamorph::EngineFailure> {
        let mut results = self.inner.select(sparql)?;
        if sparql.starts_with("SELECT ( IF(") {
            if let QueryResults::Solutions { solutions, .. } = &mut results {
                solutions.pop();
            }
        }
        Ok(results)
    }
}

// --- the three-branch semantics itself (the TLP case analysis, checked concretely) ---

#[test]
fn tlp_branches_partition_the_base_as_the_case_analysis_predicts() {
    let engine = pristine();
    let queries = tlp_queries(PATTERN, PREDICATE);
    assert_eq!(rows(&engine, &queries.base), 4, "s1..s4");
    assert_eq!(rows(&engine, &queries.branch_true), 1, "s2: 20 < 25");
    assert_eq!(rows(&engine, &queries.branch_false), 1, "s1: 30 < 25 is false");
    // s3: "twenty" < 25 is a type error; s4: ?age unbound (OPTIONAL) is an error.
    assert_eq!(rows(&engine, &queries.branch_error), 2, "s3 type error + s4 unbound");
}

// --- non-vacuity: each oracle passes on the pristine engine, flags the mutant ---

#[test]
fn tlp_passes_on_pristine_sparq() {
    let verdict = check_tlp(&pristine(), PATTERN, PREDICATE);
    assert!(verdict.is_pass(), "expected a pass, got {verdict:?}");
}

#[test]
fn tlp_flags_the_seeded_wrong_result_mutant() {
    let mutant = FilterDropsRow::new(pristine());
    let verdict = check_tlp(&mutant, PATTERN, PREDICATE);
    assert!(verdict.is_violation(), "expected a violation, got {verdict:?}");
}

#[test]
fn norec_passes_on_pristine_sparq() {
    let verdict = check_norec(&pristine(), PATTERN, PREDICATE);
    assert!(verdict.is_pass(), "expected a pass, got {verdict:?}");
}

#[test]
fn norec_flags_the_seeded_wrong_result_mutant() {
    let mutant = FilterDropsRow::new(pristine());
    let verdict = check_norec(&mutant, PATTERN, PREDICATE);
    assert!(verdict.is_violation(), "expected a violation, got {verdict:?}");
}

#[test]
fn differential_passes_on_two_pristine_sparq_instances() {
    let a = pristine();
    let b = InProcessSparq::from_ntriples("sparq-b", DATA).unwrap();
    let query = format!("SELECT * WHERE {{ {PATTERN} FILTER( {PREDICATE} ) }}");
    let verdict = check_differential(&[&a, &b], &query);
    assert!(verdict.is_pass(), "expected a pass, got {verdict:?}");
}

#[test]
fn differential_flags_the_seeded_wrong_result_mutant() {
    let reference = pristine();
    let mutant = FilterDropsRow::new(pristine());
    let query = format!("SELECT * WHERE {{ {PATTERN} FILTER( {PREDICATE} ) }}");
    let verdict = check_differential(&[&reference, &mutant], &query);
    assert!(verdict.is_violation(), "expected a violation, got {verdict:?}");
}

// --- fail-closed: engine errors are engine failures, never passes or violations ---

/// A driver whose every evaluation fails, standing in for a crashed/unreachable engine.
struct AlwaysFails;
impl SparqlEngine for AlwaysFails {
    fn name(&self) -> &str {
        "always-fails"
    }
    fn select(&self, sparql: &str) -> Result<QueryResults, sparq_metamorph::EngineFailure> {
        Err(sparq_metamorph::EngineFailure {
            engine: "always-fails".into(),
            query: sparql.to_string(),
            kind: FailureKind::Evaluation,
            message: "synthetic failure".into(),
        })
    }
}

#[test]
fn all_three_oracles_fail_closed_on_an_erroring_engine() {
    let tlp = check_tlp(&AlwaysFails, PATTERN, PREDICATE);
    let norec = check_norec(&AlwaysFails, PATTERN, PREDICATE);
    let differential = check_differential(&[&AlwaysFails, &AlwaysFails], "ASK { ?s ?p ?o }");
    for verdict in [&tlp, &norec, &differential] {
        assert!(
            verdict.is_engine_failure(),
            "an engine error must be an EngineFailure verdict, got {verdict:?}"
        );
        assert!(!verdict.is_pass() && !verdict.is_violation());
    }
}

#[test]
fn a_syntactically_invalid_query_is_an_engine_failure_on_the_real_engine() {
    let engine = pristine();
    let verdict = check_differential(&[&engine, &engine], "SELECT * WHERE { broken");
    match verdict {
        Verdict::EngineFailure(f) => assert_eq!(f.kind, FailureKind::Evaluation),
        other => panic!("expected an engine failure, got {other:?}"),
    }
}

// --- the sq-996jn term-inspection atoms: LANG + isLiteral over the real engine ---

/// One object per EBV outcome of `LANG(?v) = "fr"`: a language-tagged literal (true),
/// a plain literal (`LANG` returns `""` — false), and an IRI (`LANG` on a non-literal
/// is a type error, SPARQL 1.1 §17.3 operand rules).
const LANG_DATA: &str = concat!(
    "<http://example.org/s1> <http://example.org/v> \"mot\"@fr .\n",
    "<http://example.org/s2> <http://example.org/v> \"str\" .\n",
    "<http://example.org/s3> <http://example.org/v> <http://example.org/o3> .\n",
);
const LANG_PATTERN: &str = "?s <http://example.org/v> ?v";

/// The `LANG(?v) = "fr"` atom must land one row in EACH TLP branch on the mixed
/// dataset — the concrete witness that the atom is genuine three-outcome fuel (the
/// whole point of adding it, sq-996jn), not just a true/false predicate.
#[test]
fn lang_atom_exercises_the_full_ebv_trichotomy() {
    let engine = InProcessSparq::from_ntriples("sparq", LANG_DATA).unwrap();
    let predicate = "LANG(?v) = \"fr\"";
    let queries = tlp_queries(LANG_PATTERN, predicate);
    assert_eq!(rows(&engine, &queries.base), 3, "s1..s3");
    assert_eq!(rows(&engine, &queries.branch_true), 1, "s1: LANG(\"mot\"@fr) = \"fr\"");
    assert_eq!(
        rows(&engine, &queries.branch_false),
        1,
        "s2: LANG on a plain literal returns \"\", which differs from \"fr\""
    );
    assert_eq!(
        rows(&engine, &queries.branch_error),
        1,
        "s3: LANG on an IRI is a type error"
    );
    assert!(check_tlp(&engine, LANG_PATTERN, predicate).is_pass());
    assert!(check_norec(&engine, LANG_PATTERN, predicate).is_pass());
}

/// `isLiteral(?v)` over a bound variable splits literals (true) from IRIs (false)
/// and never errors — the two-outcome companion atom (mirrors `isIRI`).
#[test]
fn is_literal_atom_splits_literals_from_iris() {
    let engine = InProcessSparq::from_ntriples("sparq", LANG_DATA).unwrap();
    let predicate = "isLiteral(?v)";
    let queries = tlp_queries(LANG_PATTERN, predicate);
    assert_eq!(rows(&engine, &queries.branch_true), 2, "s1 lang-tagged + s2 plain");
    assert_eq!(rows(&engine, &queries.branch_false), 1, "s3: an IRI is not a literal");
    assert_eq!(rows(&engine, &queries.branch_error), 0, "?v is always bound here");
    assert!(check_tlp(&engine, LANG_PATTERN, predicate).is_pass());
    assert!(check_norec(&engine, LANG_PATTERN, predicate).is_pass());
}

/// The oracles still catch the seeded wrong-result mutant on a LANG predicate —
/// the non-vacuity anchor extended to the new atom family.
#[test]
fn mutant_is_flagged_on_a_lang_predicate() {
    let mutant =
        FilterDropsRow::new(InProcessSparq::from_ntriples("sparq", LANG_DATA).unwrap());
    let verdict = check_tlp(&mutant, LANG_PATTERN, "LANG(?v) = \"fr\"");
    assert!(verdict.is_violation(), "expected a violation, got {verdict:?}");
}

// --- generated cases: the oracles hold on the real engine across seeds ---

/// TLP + NoREC over the seeded generator against real sparq: every verdict must be a
/// pass (the relations hold) or — never — a violation. Engine failures would surface
/// generator/grammar bugs, so they are also rejected here: sparq accepts the full
/// generated grammar today, and a grammar extension that sparq rejects should be
/// caught at generation time, not silently skipped.
#[test]
fn generated_cases_hold_on_pristine_sparq() {
    for seed in 0..50 {
        let case = generate_case(seed);
        let engine = InProcessSparq::from_ntriples("sparq", &case.data_ntriples)
            .unwrap_or_else(|e| panic!("seed {seed}: generated data must load: {e}"));
        let tlp = check_tlp(&engine, &case.pattern, &case.predicate);
        assert!(
            tlp.is_pass(),
            "seed {seed}: TLP must hold on sparq (predicate: {}): {tlp:?}",
            case.predicate
        );
        let norec = check_norec(&engine, &case.pattern, &case.predicate);
        assert!(
            norec.is_pass(),
            "seed {seed}: NoREC must hold on sparq (predicate: {}): {norec:?}",
            case.predicate
        );
    }
}

#[test]
fn norec_rewrite_preserves_absolute_cardinality_across_generated_cases() {
    for seed in 0..10 {
        let case = generate_case(seed);
        let engine = InProcessSparq::from_ntriples("sparq", &case.data_ntriples)
            .unwrap_or_else(|e| panic!("seed {seed}: generated data must load: {e}"));
        assert!(
            norec_rewrite_preserves_cardinality(&engine, &case.pattern, &case.predicate),
            "seed {seed}: the NoREC rewrite must preserve the base row cardinality"
        );
        let verdict = check_norec(&engine, &case.pattern, &case.predicate);
        assert!(
            verdict.is_pass(),
            "seed {seed}: NoREC sanity check must pass: {verdict:?}"
        );
    }
}

#[test]
fn norec_rewrite_cardinality_check_witnesses_a_dropped_rewrite_row() {
    for seed in 0..10 {
        let case = generate_case(seed);
        let engine = InProcessSparq::from_ntriples("sparq", &case.data_ntriples)
            .unwrap_or_else(|e| panic!("seed {seed}: generated data must load: {e}"));
        let mutant = RewriteDropsRow::new(engine);
        assert!(
            !norec_rewrite_preserves_cardinality(&mutant, &case.pattern, &case.predicate),
            "seed {seed}: mutation witness must make the cardinality check fail"
        );
    }
}

/// Generated cases whose predicates carry the sq-996jn atoms hold on real sparq:
/// hunt the deterministic seed stream for a fixed number of LANG- and
/// isLiteral-bearing cases and require both oracles to pass on each. Fail-closed on
/// reachability too: if the hunt cannot find enough such cases, the test fails
/// (a grammar regression, not a skip).
#[test]
fn generated_cases_with_the_new_atoms_hold_on_pristine_sparq() {
    const WANT: u32 = 10;
    let (mut lang_checked, mut is_literal_checked) = (0u32, 0u32);
    for seed in 0..2000u64 {
        if lang_checked >= WANT && is_literal_checked >= WANT {
            break;
        }
        let case = generate_case(seed);
        let has_lang = case.predicate.contains("LANG(?v)");
        let has_is_literal = case.predicate.contains("isLiteral(?v)");
        if !(has_lang && lang_checked < WANT) && !(has_is_literal && is_literal_checked < WANT)
        {
            continue;
        }
        let engine = InProcessSparq::from_ntriples("sparq", &case.data_ntriples)
            .unwrap_or_else(|e| panic!("seed {seed}: generated data must load: {e}"));
        let tlp = check_tlp(&engine, &case.pattern, &case.predicate);
        assert!(
            tlp.is_pass(),
            "seed {seed}: TLP must hold on sparq (predicate: {}): {tlp:?}",
            case.predicate
        );
        let norec = check_norec(&engine, &case.pattern, &case.predicate);
        assert!(
            norec.is_pass(),
            "seed {seed}: NoREC must hold on sparq (predicate: {}): {norec:?}",
            case.predicate
        );
        lang_checked += u32::from(has_lang);
        is_literal_checked += u32::from(has_is_literal);
    }
    assert!(
        lang_checked >= WANT && is_literal_checked >= WANT,
        "seeds 0..2000 must reach {WANT} LANG and {WANT} isLiteral cases \
         (got lang={lang_checked}, isLiteral={is_literal_checked})"
    );
}

// ===========================================================================
// sq-gum8.12 oracle extensions — each derivation checked against the REAL engine,
// each law carrying a mutant that flags it non-vacuously.
// ===========================================================================

/// The `DISTINCT` fixture. `?g` is the *projected* column and `?v` drives the partition,
/// so the branches deliberately overlap on `?g`:
///
/// | subject | `?v`     | branch | `?g` |
/// |---------|----------|--------|------|
/// | s1      | 10       | true   | A    |
/// | s2      | 20       | true   | A    |
/// | s3      | 30       | false  | A    |  ← same `?g` as the true branch
/// | s4      | `"xx"`   | error  | B    |
/// | s5      | 40       | false  | C    |
const DISTINCT_DATA: &str = concat!(
    "<http://example.org/s1> <http://example.org/v> \"10\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s1> <http://example.org/g> \"A\" .\n",
    "<http://example.org/s2> <http://example.org/v> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s2> <http://example.org/g> \"A\" .\n",
    "<http://example.org/s3> <http://example.org/v> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s3> <http://example.org/g> \"A\" .\n",
    "<http://example.org/s4> <http://example.org/v> \"xx\" .\n",
    "<http://example.org/s4> <http://example.org/g> \"B\" .\n",
    "<http://example.org/s5> <http://example.org/v> \"40\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s5> <http://example.org/g> \"C\" .\n",
);
const DISTINCT_PATTERN: &str =
    "?s <http://example.org/v> ?v . ?s <http://example.org/g> ?g";
const DISTINCT_PREDICATE: &str = "?v < 25";

fn distinct_engine() -> InProcessSparq {
    InProcessSparq::from_ntriples("sparq", DISTINCT_DATA).expect("test data loads")
}

/// The derivation witness for `∪` rather than `⊎`: on this fixture the *set* law holds
/// while the multiset law is genuinely false on a **correct** engine, because `s3`
/// (false branch) projects onto the same `?g` as `s1`/`s2` (true branch). Reusing the
/// plain-TLP multiset law here would manufacture a violation out of correct behaviour.
#[test]
fn distinct_partitions_recombine_by_set_union_not_multiset_union() {
    let engine = distinct_engine();
    let queries = tlp_distinct_queries(DISTINCT_PATTERN, DISTINCT_PREDICATE, &["g"]);
    let base = rows(&engine, &queries.base);
    let kept_true = rows(&engine, &queries.branch_true);
    let kept_false = rows(&engine, &queries.branch_false);
    let kept_error = rows(&engine, &queries.branch_error);
    assert_eq!(base, 3, "distinct ?g over s1..s5 is {{A, B, C}}");
    assert_eq!(kept_true, 1, "s1 + s2 both carry ?g = A");
    assert_eq!(kept_false, 2, "s3 (A) + s5 (C)");
    assert_eq!(kept_error, 1, "s4 (B): \"xx\" < 25 is a type error");
    assert_eq!(
        kept_true + kept_false + kept_error,
        4,
        "the branch cardinalities sum to MORE than the distinct base — the multiset law \
         is false here, which is exactly why the recombination is a set union"
    );
    assert_ne!(base, kept_true + kept_false + kept_error);
    let verdict = check_tlp_distinct(&engine, DISTINCT_PATTERN, DISTINCT_PREDICATE, &["g"]);
    assert!(verdict.is_pass(), "expected a pass, got {verdict:?}");
}

/// `SELECT DISTINCT *`: the partitions are provably disjoint, so the checker also
/// enforces the stronger multiset law there.
#[test]
fn distinct_star_partitions_are_disjoint_so_cardinalities_add() {
    let engine = distinct_engine();
    let queries = tlp_distinct_queries(DISTINCT_PATTERN, DISTINCT_PREDICATE, &[]);
    assert_eq!(rows(&engine, &queries.base), 5, "s1..s5 are pairwise distinct");
    assert_eq!(
        rows(&engine, &queries.branch_true)
            + rows(&engine, &queries.branch_false)
            + rows(&engine, &queries.branch_error),
        5,
        "with no projection the branches cannot share a solution mapping"
    );
    let verdict = check_tlp_distinct(&engine, DISTINCT_PATTERN, DISTINCT_PREDICATE, &[]);
    assert!(verdict.is_pass(), "expected a pass, got {verdict:?}");
}

#[test]
fn distinct_oracle_flags_the_seeded_wrong_result_mutant() {
    let mutant = FilterDropsRow::new(distinct_engine());
    for projection in [&["g"][..], &[][..]] {
        let verdict = check_tlp_distinct(&mutant, DISTINCT_PATTERN, DISTINCT_PREDICATE, projection);
        assert!(
            verdict.is_violation(),
            "projection {projection:?}: expected a violation, got {verdict:?}"
        );
    }
}

#[test]
fn distinct_oracle_fails_closed_on_an_erroring_engine() {
    let verdict = check_tlp_distinct(&AlwaysFails, DISTINCT_PATTERN, DISTINCT_PREDICATE, &["g"]);
    assert!(verdict.is_engine_failure(), "got {verdict:?}");
}

/// The aggregate / `ORDER BY` fixture: `?v` drives the partition (with `s4` erroring),
/// `?w` is an `OPTIONAL` `xsd:integer` column — bound on s1/s2/s5, unbound on s3/s4/s6 —
/// so it is inside the exactness precondition and still supplies unbound rows.
const AGG_DATA: &str = concat!(
    "<http://example.org/s1> <http://example.org/v> \"10\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s1> <http://example.org/w> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s2> <http://example.org/v> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s2> <http://example.org/w> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s3> <http://example.org/v> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s4> <http://example.org/v> \"xx\" .\n",
    "<http://example.org/s5> <http://example.org/v> \"40\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s5> <http://example.org/w> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
    "<http://example.org/s6> <http://example.org/v> \"50\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
);
const AGG_PATTERN: &str =
    "?s <http://example.org/v> ?v . OPTIONAL { ?s <http://example.org/w> ?w }";
const AGG_PREDICATE: &str = "?v < 25";
/// An `xsd:integer` cast: errors exactly on `s4`'s `"xx"`, so the aggregate error lands
/// in ONE branch — the concrete witness for the error-status law.
const CAST_V: &str = "<http://www.w3.org/2001/XMLSchema#integer>(?v)";

fn agg_engine() -> InProcessSparq {
    InProcessSparq::from_ntriples("sparq", AGG_DATA).expect("test data loads")
}

fn agg_cell(engine: &dyn SparqlEngine, query: &str) -> Option<String> {
    match engine.select(query).expect("query evaluates") {
        QueryResults::Solutions { solutions, .. } => {
            assert_eq!(solutions.len(), 1, "one group, one row: {query}");
            solutions[0].get("tlpAgg").map(|term| match term {
                sparq_difftest::Term::Literal { lexical, .. } => lexical.clone(),
                other => panic!("aggregate cell is not a literal: {other:?}"),
            })
        }
        QueryResults::Boolean(_) => panic!("SELECT returned a boolean"),
    }
}

/// The additive law, checked value-by-value on the real engine: `COUNT(*)` and
/// `SUM(?w)` over the three branches recombine to the base. Unbound `?w` rows are
/// removed from the fold row-locally (s3/s4/s6 contribute nothing), and an EMPTY branch
/// would contribute the identity `0` — never unbound — which is what makes the sum law
/// hold at the edges.
#[test]
fn aggregate_partitions_recombine_by_addition() {
    let engine = agg_engine();
    for (agg, base, kept_true, kept_false, kept_error) in [
        (PartitionedAggregate::CountStar, "6", "2", "3", "1"),
        (
            PartitionedAggregate::SumInteger("?w".into()),
            "13",
            "6",
            "7",
            "0",
        ),
        (PartitionedAggregate::Count("?w".into()), "3", "2", "1", "0"),
    ] {
        let q = sparq_metamorph::tlp_aggregate_queries(AGG_PATTERN, AGG_PREDICATE, &agg);
        assert_eq!(agg_cell(&engine, &q.base).as_deref(), Some(base), "{agg:?}");
        assert_eq!(
            agg_cell(&engine, &q.branch_true).as_deref(),
            Some(kept_true),
            "{agg:?}"
        );
        assert_eq!(
            agg_cell(&engine, &q.branch_false).as_deref(),
            Some(kept_false),
            "{agg:?}"
        );
        assert_eq!(
            agg_cell(&engine, &q.branch_error).as_deref(),
            Some(kept_error),
            "{agg:?} — the error branch (s4, no ?w) folds to the identity, not unbound"
        );
        let verdict = check_tlp_aggregate(&engine, AGG_PATTERN, AGG_PREDICATE, &agg);
        assert!(verdict.is_pass(), "{agg:?}: expected a pass, got {verdict:?}");
    }
}

/// The error-status law: `SUM(xsd:integer(?v))` errors on `s4` only, so the base
/// aggregate is unbound and so is exactly the branch that owns `s4`. The other two
/// branches stay bound — which is what makes this a real test of the law rather than an
/// everything-unbound degenerate case.
#[test]
fn an_aggregate_error_lands_in_exactly_one_branch_and_unbinds_the_base() {
    let engine = agg_engine();
    let agg = PartitionedAggregate::SumInteger(CAST_V.into());
    let q = sparq_metamorph::tlp_aggregate_queries(AGG_PATTERN, AGG_PREDICATE, &agg);
    assert_eq!(
        agg_cell(&engine, &q.base),
        None,
        "s4's failed cast makes the whole base aggregate a type error -> unbound"
    );
    assert_eq!(agg_cell(&engine, &q.branch_true).as_deref(), Some("30"), "10 + 20");
    assert_eq!(
        agg_cell(&engine, &q.branch_false).as_deref(),
        Some("120"),
        "30 + 40 + 50"
    );
    assert_eq!(agg_cell(&engine, &q.branch_error), None, "s4 alone");
    let verdict = check_tlp_aggregate(&engine, AGG_PATTERN, AGG_PREDICATE, &agg);
    assert!(verdict.is_pass(), "expected a pass, got {verdict:?}");
}

/// sparq is **not uniform** across the two readings of SPARQL's aggregate error
/// semantics — measured here rather than assumed, because the derivation in
/// `crate::aggregate` rests on the claim that *either* reading is row-local. `?w + 1`
/// errors on every solution whose `OPTIONAL ?w` is unbound (s3/s4/s6): `SUM` takes that
/// as fatal and goes unbound, while `COUNT` drops the erroring members and stays bound.
/// The recombination law must — and does — hold in both regimes.
#[test]
fn sum_is_fatal_on_an_aggregate_error_while_count_drops_the_erroring_member() {
    let engine = agg_engine();
    let sum = PartitionedAggregate::SumInteger("?w + 1".into());
    let count = PartitionedAggregate::Count("?w + 1".into());
    let sum_q = sparq_metamorph::tlp_aggregate_queries(AGG_PATTERN, AGG_PREDICATE, &sum);
    let count_q = sparq_metamorph::tlp_aggregate_queries(AGG_PATTERN, AGG_PREDICATE, &count);
    assert_eq!(
        agg_cell(&engine, &sum_q.base),
        None,
        "SUM: one erroring member makes the whole aggregate a type error -> unbound"
    );
    assert_eq!(
        agg_cell(&engine, &count_q.base).as_deref(),
        Some("3"),
        "COUNT: the erroring members are dropped, so s1 + s2 + s5 are still counted"
    );
    // The false branch owns s3 and s6 (both erroring) and s5 (bound): SUM unbound,
    // COUNT = 1. The law holds either way.
    assert_eq!(agg_cell(&engine, &sum_q.branch_false), None);
    assert_eq!(agg_cell(&engine, &count_q.branch_false).as_deref(), Some("1"));
    for agg in [&sum, &count] {
        let verdict = check_tlp_aggregate(&engine, AGG_PATTERN, AGG_PREDICATE, agg);
        assert!(verdict.is_pass(), "{agg:?}: expected a pass, got {verdict:?}");
    }
}

/// A test-only mutant that shifts the value of an aggregate cell on the partition
/// queries only. Unlike `FilterDropsRow` it perturbs no cardinality at all, so it is the
/// witness that the **sum** law — not just the row-count guard — is live.
struct AggregateOffByOne<E> {
    inner: E,
}

impl<E: SparqlEngine> SparqlEngine for AggregateOffByOne<E> {
    fn name(&self) -> &str {
        "aggregate-off-by-one"
    }

    fn select(&self, sparql: &str) -> Result<QueryResults, sparq_metamorph::EngineFailure> {
        let mut results = self.inner.select(sparql)?;
        if sparql.contains("FILTER") && sparql.contains("AS ?tlpAgg") {
            if let QueryResults::Solutions { solutions, .. } = &mut results {
                for solution in solutions.iter_mut() {
                    if let Some(sparq_difftest::Term::Literal { lexical, .. }) =
                        solution.get_mut("tlpAgg")
                    {
                        if let Ok(value) = lexical.parse::<i128>() {
                            *lexical = (value + 1).to_string();
                        }
                    }
                }
            }
        }
        Ok(results)
    }
}

/// A test-only mutant that silently unbinds an aggregate cell on the partition queries,
/// keeping the row. It is the witness for the **error-status** law: a wrong "the
/// aggregate errored" answer is invisible to any cardinality- or value-based check.
struct AggregateUnbinds<E> {
    inner: E,
}

impl<E: SparqlEngine> SparqlEngine for AggregateUnbinds<E> {
    fn name(&self) -> &str {
        "aggregate-unbinds"
    }

    fn select(&self, sparql: &str) -> Result<QueryResults, sparq_metamorph::EngineFailure> {
        let mut results = self.inner.select(sparql)?;
        if sparql.contains("FILTER") && sparql.contains("AS ?tlpAgg") {
            if let QueryResults::Solutions { solutions, .. } = &mut results {
                for solution in solutions.iter_mut() {
                    solution.remove("tlpAgg");
                }
            }
        }
        Ok(results)
    }
}

#[test]
fn aggregate_oracle_flags_every_seeded_mutant() {
    let count_star = PartitionedAggregate::CountStar;
    let sum_w = PartitionedAggregate::SumInteger("?w".into());
    for agg in [&count_star, &sum_w] {
        // Cardinality mutant: each branch query returns exactly one row, so dropping it
        // breaks the single-group rule.
        let dropped = FilterDropsRow::new(agg_engine());
        assert!(
            check_tlp_aggregate(&dropped, AGG_PATTERN, AGG_PREDICATE, agg).is_violation(),
            "{agg:?}: FilterDropsRow must be flagged"
        );
        // Value mutant: same rows, same boundness, wrong sum.
        let shifted = AggregateOffByOne {
            inner: agg_engine(),
        };
        assert!(
            check_tlp_aggregate(&shifted, AGG_PATTERN, AGG_PREDICATE, agg).is_violation(),
            "{agg:?}: an off-by-one branch aggregate must be flagged"
        );
        // Error-status mutant: same rows, no value to compare, wrong error status.
        let unbound = AggregateUnbinds {
            inner: agg_engine(),
        };
        assert!(
            check_tlp_aggregate(&unbound, AGG_PATTERN, AGG_PREDICATE, agg).is_violation(),
            "{agg:?}: a silently-unbound branch aggregate must be flagged"
        );
    }
}

/// Fail-closed on the exactness precondition: a `SUM` that promotes to `xsd:decimal` is
/// reported as a harness failure, never as a wrong-result claim — floating-point /
/// promoted folds are not associative, so an exact comparison there would be unsound.
#[test]
fn a_promoted_sum_is_a_harness_failure_not_a_violation() {
    const MIXED: &str = concat!(
        "<http://example.org/s1> <http://example.org/v> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "<http://example.org/s2> <http://example.org/v> \"2.5\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n",
    );
    let engine = InProcessSparq::from_ntriples("sparq", MIXED).unwrap();
    let verdict = check_tlp_aggregate(
        &engine,
        "?s <http://example.org/v> ?v",
        "?v < 2",
        &PartitionedAggregate::SumInteger("?v".into()),
    );
    match verdict {
        Verdict::EngineFailure(f) => assert_eq!(f.kind, FailureKind::Harness),
        other => panic!("expected a harness failure, got {other:?}"),
    }
}

#[test]
fn aggregate_oracle_fails_closed_on_an_erroring_engine() {
    let verdict = check_tlp_aggregate(
        &AlwaysFails,
        AGG_PATTERN,
        AGG_PREDICATE,
        &PartitionedAggregate::CountStar,
    );
    assert!(verdict.is_engine_failure(), "got {verdict:?}");
}

// --- ORDER BY differential mode ---

/// `ORDER BY ?w` over the fixture: `?w` is `xsd:integer`-or-unbound, so §15.1 fixes the
/// order totally (unbound sorts below every literal) and the sort column is inside the
/// comparability precondition. The predicate keeps s1, s2, s3, s5, s6 (s4 errors), so
/// the first sort-key run — the unbound-`?w` rows s3 and s6 — has **two** members whose
/// relative order the spec leaves free.
fn ordered_fixture() -> (String, Vec<String>) {
    let built = ordered_query(AGG_PATTERN, "?v < 100", &["w"]);
    (built.query, built.sort_vars)
}

#[test]
fn ordered_query_builds_the_order_by_clause_and_its_sort_var_list() {
    let (query, sort_vars) = ordered_fixture();
    assert!(query.ends_with(" ORDER BY ?w"), "{query}");
    assert!(query.contains("FILTER( ?v < 100 )"), "{query}");
    assert_eq!(sort_vars, vec!["w".to_string()]);
    // No sort variables => no ORDER BY clause (the check then degrades to multiset).
    assert!(!ordered_query("?s ?p ?o", "true", &[])
        .query
        .contains("ORDER BY"));
}

#[test]
fn the_ordered_fixture_has_a_multi_row_tie_run_and_a_total_sort_key() {
    let engine = agg_engine();
    let (query, _) = ordered_fixture();
    let solutions = match engine.select(&query).expect("query evaluates") {
        QueryResults::Solutions { solutions, .. } => solutions,
        QueryResults::Boolean(_) => panic!("SELECT returned a boolean"),
    };
    assert_eq!(solutions.len(), 5, "s1, s2, s3, s5, s6 (s4's ?v errors)");
    let bound: Vec<bool> = solutions.iter().map(|s| s.get("w").is_some()).collect();
    assert_eq!(
        bound,
        vec![false, false, true, true, true],
        "§15.1: unbound sorts below every literal, so the two ?w-less rows lead"
    );
}

/// A test-only mutant that reverses the result sequence of an `ORDER BY` query. It is
/// the witness that the ordered mode tests something the shipped differential oracle
/// **provably cannot see**: the bag is untouched, so `check_differential` passes while
/// `check_differential_ordered` flags it.
struct ReversesOrder<E> {
    inner: E,
}

impl<E: SparqlEngine> SparqlEngine for ReversesOrder<E> {
    fn name(&self) -> &str {
        "reverses-order"
    }

    fn select(&self, sparql: &str) -> Result<QueryResults, sparq_metamorph::EngineFailure> {
        let mut results = self.inner.select(sparql)?;
        if sparql.contains("ORDER BY") {
            if let QueryResults::Solutions { solutions, .. } = &mut results {
                solutions.reverse();
            }
        }
        Ok(results)
    }
}

/// A test-only mutant that swaps the first two rows of an `ORDER BY` result. On the
/// fixture those are exactly the two unbound-`?w` rows — one sort-key equivalence class
/// — so this is a **spec-permitted** reordering that the oracle must NOT report. An
/// order oracle that flags legal latitude is worse than no order oracle.
struct PermutesWithinTie<E> {
    inner: E,
}

impl<E: SparqlEngine> SparqlEngine for PermutesWithinTie<E> {
    fn name(&self) -> &str {
        "permutes-within-tie"
    }

    fn select(&self, sparql: &str) -> Result<QueryResults, sparq_metamorph::EngineFailure> {
        let mut results = self.inner.select(sparql)?;
        if sparql.contains("ORDER BY") {
            if let QueryResults::Solutions { solutions, .. } = &mut results {
                if solutions.len() >= 2 {
                    solutions.swap(0, 1);
                }
            }
        }
        Ok(results)
    }
}

#[test]
fn ordered_differential_passes_on_two_pristine_engines() {
    let a = agg_engine();
    let b = InProcessSparq::from_ntriples("sparq-b", AGG_DATA).unwrap();
    let (query, sort_vars) = ordered_fixture();
    let keys: Vec<&str> = sort_vars.iter().map(String::as_str).collect();
    let verdict = check_differential_ordered(&[&a, &b], &query, &keys);
    assert!(verdict.is_pass(), "expected a pass, got {verdict:?}");
}

#[test]
fn ordered_differential_flags_a_reordering_the_unordered_oracle_cannot_see() {
    let reference = agg_engine();
    let mutant = ReversesOrder {
        inner: agg_engine(),
    };
    let (query, sort_vars) = ordered_fixture();
    let keys: Vec<&str> = sort_vars.iter().map(String::as_str).collect();
    let unordered = check_differential(&[&reference, &mutant], &query);
    assert!(
        unordered.is_pass(),
        "the bag is unchanged, so the unordered oracle is blind to this bug: {unordered:?}"
    );
    let ordered = check_differential_ordered(&[&reference, &mutant], &query, &keys);
    assert!(ordered.is_violation(), "expected a violation, got {ordered:?}");
}

#[test]
fn ordered_differential_does_not_flag_a_permutation_within_a_tie_run() {
    let reference = agg_engine();
    let permuted = PermutesWithinTie {
        inner: agg_engine(),
    };
    let (query, sort_vars) = ordered_fixture();
    let keys: Vec<&str> = sort_vars.iter().map(String::as_str).collect();
    let verdict = check_differential_ordered(&[&reference, &permuted], &query, &keys);
    assert!(
        verdict.is_pass(),
        "SPARQL 1.1 §15.1 leaves the order within a sort-key equivalence class free — \
         reporting it would be a false violation: {verdict:?}"
    );
}

#[test]
fn ordered_differential_still_flags_the_row_dropping_mutant() {
    let reference = agg_engine();
    let mutant = FilterDropsRow::new(agg_engine());
    let (query, sort_vars) = ordered_fixture();
    let keys: Vec<&str> = sort_vars.iter().map(String::as_str).collect();
    let verdict = check_differential_ordered(&[&reference, &mutant], &query, &keys);
    assert!(verdict.is_violation(), "expected a violation, got {verdict:?}");
}

#[test]
fn ordered_differential_fails_closed_on_an_erroring_engine() {
    let verdict =
        check_differential_ordered(&[&AlwaysFails, &AlwaysFails], "ASK { ?s ?p ?o }", &["w"]);
    assert!(verdict.is_engine_failure(), "got {verdict:?}");
}

// --- the extensions over the seeded generator, against the real engine ---

/// The `DISTINCT` and aggregate laws must hold on every generated case, and the sweep
/// must be non-vacuous: the seeded mutant has to be flagged on a decent share of the
/// same seeds (a sweep that only ever sees empty results would prove nothing).
#[test]
fn extension_oracles_hold_on_generated_cases_and_flag_the_mutant() {
    let sum_w = PartitionedAggregate::SumInteger("?w".into());
    // `?w + 1` errors on every solution whose OPTIONAL ?w is unbound, so this variant
    // exercises the error-status law across the seed stream while staying inside the
    // integer exactness precondition.
    let sum_w_plus_one = PartitionedAggregate::SumInteger("?w + 1".into());
    let mut flagged_distinct = 0u32;
    let mut flagged_aggregate = 0u32;
    for seed in 0..50 {
        let case = generate_case(seed);
        let engine = InProcessSparq::from_ntriples("sparq", &case.data_ntriples)
            .unwrap_or_else(|e| panic!("seed {seed}: generated data must load: {e}"));
        for projection in [&["v"][..], &[][..]] {
            let verdict = check_tlp_distinct(&engine, &case.pattern, &case.predicate, projection);
            assert!(
                verdict.is_pass(),
                "seed {seed}: the DISTINCT law must hold on sparq (projection {projection:?}, \
                 predicate: {}): {verdict:?}",
                case.predicate
            );
        }
        for agg in [
            &PartitionedAggregate::CountStar,
            &sum_w,
            &sum_w_plus_one,
            &PartitionedAggregate::Count("?w".into()),
        ] {
            let verdict = check_tlp_aggregate(&engine, &case.pattern, &case.predicate, agg);
            assert!(
                verdict.is_pass(),
                "seed {seed}: the aggregate law must hold on sparq ({agg:?}, predicate: {}): \
                 {verdict:?}",
                case.predicate
            );
        }
        let mutant = FilterDropsRow::new(
            InProcessSparq::from_ntriples("sparq", &case.data_ntriples).unwrap(),
        );
        if check_tlp_distinct(&mutant, &case.pattern, &case.predicate, &["v"]).is_violation() {
            flagged_distinct += 1;
        }
        if check_tlp_aggregate(&mutant, &case.pattern, &case.predicate, &sum_w).is_violation() {
            flagged_aggregate += 1;
        }
    }
    assert!(
        flagged_distinct > 0,
        "the DISTINCT sweep never flagged the seeded mutant — it would be vacuous"
    );
    assert_eq!(
        flagged_aggregate, 50,
        "every aggregate branch query returns exactly one row, so the row-dropping \
         mutant must be flagged on every seed"
    );
}
