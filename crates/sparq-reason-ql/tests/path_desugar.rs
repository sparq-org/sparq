// sq-pbz04.3.2 (epic sq-pbz04 / sq-6tykl):
// Tests for B5 — non-recursive property-path desugaring (`/`, `^`, `|`) ahead of the CQ gate;
// recursive / zero-length / negated forms stay FAIL-CLOSED.
//
// 🤖 SPARQ agent. STEP 0 finding baked into the tests: the vendored spargebra parser already
// lowers a TOP-LEVEL sequence (`p1/p2`) to a BGP joined by a fresh blank node, and a top-level
// inverse of a simple predicate (`^p`) to a swapped BGP triple — so those never reach the gate
// as a `Path`, and the emitter already lifts the blank-node intermediate to a fresh existential
// (sq-pbz04.3.6). The ONLY non-recursive form that survives parsing as a `GraphPattern::Path`
// is ALTERNATION (whose arms may nest sequence/inverse/named-node). The desugarer therefore
// rewrites exactly those surviving `Path` nodes; it does NOT re-translate what the parser
// already normalised (no double-translation). These tests exercise all three forms end-to-end.
//
// SOUNDNESS INVARIANT (load-bearing): each accepted non-recursive path form must rewrite
// RESULT-EQUIVALENTLY to its hand-desugared conjunctive/UCQ form over a fixture TBox. The gated
// module below proves this with the strengthened DIFFERENTIAL ORACLE from broadened_shapes.rs:
// both rewritten UCQs are EVALUATED over a concrete ABox by a minimal faithful BGP-union matcher
// and their PROJECTED answer sets are compared for equality AND against a hand-derived oracle
// (strictly stronger than a disjunct COUNT, and invariant to how the fresh intermediate is
// named). Fresh sequence intermediates are never distinguished. Recursive (`+`/`*`),
// zero-length (`?`), and negated (`!p`) forms must still be REJECTED fail-closed.

use spargebra::term::{NamedNodePattern, TermPattern, Variable};
use spargebra::{Query, SparqlParser};

use sparq_reason_ql::{as_conjunctive_query, as_ucq, CqError};

const PRE: &str = "PREFIX : <http://ex/> \
                   PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>";

fn q(s: &str) -> Query {
    SparqlParser::new().parse_query(s).expect("parse")
}

fn reject_reason(query: &str) -> String {
    match as_ucq(&q(query)) {
        Err(CqError::OutOfScope(r)) => r,
        Ok(_) => panic!("expected OutOfScope rejection for: {}", query),
    }
}

/// The predicate IRI of a triple-pattern atom (all gate atoms have a named predicate).
fn pred_iri(tp: &spargebra::term::TriplePattern) -> String {
    match &tp.predicate {
        NamedNodePattern::NamedNode(n) => n.as_str().to_string(),
        NamedNodePattern::Variable(_) => panic!("variable predicate should have been rejected"),
    }
}

fn var_name(t: &TermPattern) -> Option<String> {
    match t {
        TermPattern::Variable(v) => Some(v.as_str().to_string()),
        _ => None,
    }
}

// =============================================================================
// Part A — GATE-LEVEL tests (always compiled; run in BOTH feature states).
// The CQ/UCQ-shape gate is always present, so these assert the classification
// without the (experimental) rewriter.
// =============================================================================

#[test]
fn alternation_accepted_as_two_branch_ucq() {
    // `?x :p1|:p2 ?y` desugars to a 2-branch UCQ (branch multiplication into the B1 machinery).
    let ucq = as_ucq(&q(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :p1|:p2 ?y }}"))).expect("UCQ");
    assert_eq!(ucq.branches.len(), 2, "alternation must produce two UCQ branches");
    let preds: Vec<String> = ucq
        .branches
        .iter()
        .map(|b| {
            assert_eq!(b.atoms.len(), 1, "each alternation branch is a single atom");
            pred_iri(&b.atoms[0])
        })
        .collect();
    assert!(preds.contains(&"http://ex/p1".to_string()));
    assert!(preds.contains(&"http://ex/p2".to_string()));
}

#[test]
fn alternation_three_way_accepted() {
    let ucq = as_ucq(&q(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :p1|:p2|:p3 ?y }}"))).expect("UCQ");
    assert_eq!(ucq.branches.len(), 3, "three-way alternation → three branches");
}

#[test]
fn alternation_rejected_by_as_conjunctive_query() {
    // A single-CQ classifier must reject a UCQ (alternation is a UCQ). Use `as_ucq` for these.
    let r = as_conjunctive_query(&q(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :p1|:p2 ?y }}")));
    assert!(
        matches!(r, Err(CqError::OutOfScope(ref s)) if s.contains("UCQ") || s.contains("UNION")),
        "as_conjunctive_query must reject an alternation as a multi-branch UCQ; got {:?}",
        r
    );
}

#[test]
fn nested_alternation_of_sequence_uses_fresh_nondistinguished_intermediate() {
    // `(:a/:b)|:c`: branch 0 is the desugared sequence (2 atoms sharing a fresh intermediate),
    // branch 1 is the single `:c` atom.
    let ucq = as_ucq(&q(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x (:a/:b)|:c ?y }}"))).expect("UCQ");
    assert_eq!(ucq.branches.len(), 2);

    let seq_branch = ucq
        .branches
        .iter()
        .find(|b| b.atoms.len() == 2)
        .expect("one branch must be the desugared 2-atom sequence");
    let single = ucq
        .branches
        .iter()
        .find(|b| b.atoms.len() == 1)
        .expect("one branch must be the single :c atom");
    assert_eq!(pred_iri(&single.atoms[0]), "http://ex/c");

    // The sequence branch: `?x :a ?f . ?f :b ?y` — the intermediate ?f is the OBJECT of atom 0
    // and the SUBJECT of atom 1 (shared), and is NON-distinguished.
    let a_atom = &seq_branch.atoms[0];
    let b_atom = &seq_branch.atoms[1];
    assert_eq!(pred_iri(a_atom), "http://ex/a");
    assert_eq!(pred_iri(b_atom), "http://ex/b");
    let mid_obj = var_name(&a_atom.object).expect("sequence intermediate is a variable");
    let mid_subj = var_name(&b_atom.subject).expect("sequence intermediate is a variable");
    assert_eq!(mid_obj, mid_subj, "the two sequence atoms must SHARE the intermediate var");

    let distinguished: Vec<String> = seq_branch
        .distinguished
        .iter()
        .map(|v: &Variable| v.as_str().to_string())
        .collect();
    assert!(
        !distinguished.contains(&mid_obj),
        "fresh sequence intermediate ?{} must be NON-distinguished; distinguished = {:?}",
        mid_obj,
        distinguished
    );
    // Sanity: the intermediate is not one of the projected variables x/y either.
    assert_ne!(mid_obj, "x");
    assert_ne!(mid_obj, "y");
}

#[test]
fn inverse_in_alternation_arm_swaps_subject_object() {
    // `?x ^:p1|:p2 ?y` → branch 0 = `?y :p1 ?x` (swapped), branch 1 = `?x :p2 ?y`.
    let ucq = as_ucq(&q(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x ^:p1|:p2 ?y }}"))).expect("UCQ");
    assert_eq!(ucq.branches.len(), 2);
    let inv = ucq
        .branches
        .iter()
        .find(|b| pred_iri(&b.atoms[0]) == "http://ex/p1")
        .expect("a :p1 branch");
    // Inverse: subject/object swapped relative to the query's ?x/?y.
    assert_eq!(var_name(&inv.atoms[0].subject).as_deref(), Some("y"));
    assert_eq!(var_name(&inv.atoms[0].object).as_deref(), Some("x"));
}

#[test]
fn top_level_sequence_still_accepted_no_double_translation() {
    // STEP 0: a top-level sequence is lowered to a BGP by spargebra (blank-node intermediate),
    // so it reaches the gate as a single 2-atom conjunction — the desugarer leaves it untouched.
    let cq = as_conjunctive_query(&q(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :p1/:p2 ?y }}")))
        .expect("sequence is a single CQ (spargebra lowered it to a BGP)");
    assert_eq!(cq.atoms.len(), 2, "sequence lowers to a 2-atom conjunction");
    let preds: Vec<String> = cq.atoms.iter().map(pred_iri).collect();
    assert!(preds.contains(&"http://ex/p1".to_string()));
    assert!(preds.contains(&"http://ex/p2".to_string()));
}

#[test]
fn top_level_inverse_still_accepted_no_double_translation() {
    // STEP 0: `^:r` is lowered to a swapped BGP triple by spargebra — a single CQ atom.
    let cq = as_conjunctive_query(&q(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x ^:r ?y }}")))
        .expect("inverse of a simple predicate is a single CQ");
    assert_eq!(cq.atoms.len(), 1);
    assert_eq!(pred_iri(&cq.atoms[0]), "http://ex/r");
    // Swapped: subject is ?y, object is ?x.
    assert_eq!(var_name(&cq.atoms[0].subject).as_deref(), Some("y"));
    assert_eq!(var_name(&cq.atoms[0].object).as_deref(), Some("x"));
}

// ---- Recursive / zero-length / negated forms stay FAIL-CLOSED ----

#[test]
fn one_or_more_rejected() {
    let r = reject_reason(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :r+ ?y }}"));
    assert!(r.contains("property path") && r.contains("fail-closed"), "got: {}", r);
}

#[test]
fn zero_or_more_rejected() {
    let r = reject_reason(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :r* ?y }}"));
    assert!(r.contains("property path") && r.contains("fail-closed"), "got: {}", r);
}

#[test]
fn zero_or_one_rejected() {
    let r = reject_reason(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :r? ?y }}"));
    assert!(r.contains("property path") && r.contains("fail-closed"), "got: {}", r);
}

#[test]
fn negated_property_set_rejected() {
    let r = reject_reason(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x !:r ?y }}"));
    assert!(r.contains("negated property set") && r.contains("fail-closed"), "got: {}", r);
}

#[test]
fn alternation_with_recursive_arm_rejected() {
    // Fail-closed must dominate: an alternation with ONE recursive arm rejects the whole query.
    let r = reject_reason(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :p|(:r+) ?y }}"));
    assert!(r.contains("property path") && r.contains("fail-closed"), "got: {}", r);
}

#[test]
fn sequence_with_recursive_arm_rejected() {
    let r = reject_reason(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x :p/(:r+) ?y }}"));
    assert!(r.contains("property path") && r.contains("fail-closed"), "got: {}", r);
}

#[test]
fn recursive_inside_inverse_rejected() {
    // `^(:r+)` — reverse of a recursive path is still recursive; reject fail-closed.
    let r = reject_reason(&format!("{PRE} SELECT ?x ?y WHERE {{ ?x ^(:r+) ?y }}"));
    assert!(r.contains("property path") && r.contains("fail-closed"), "got: {}", r);
}

// =============================================================================
// Part B — RESULT-EQUIVALENCE ORACLE (behind `experimental`).
// Each accepted path form must rewrite RESULT-EQUIVALENTLY to its hand-desugared
// conjunctive/UCQ form, proven by evaluating BOTH rewritten UCQs over a concrete
// ABox and comparing PROJECTED answer sets (and a hand-derived oracle).
// =============================================================================

#[cfg(not(feature = "experimental"))]
#[test]
fn path_desugar_oracle_skipped_without_experimental() {
    eprintln!(
        "SKIP: property-path result-equivalence oracle requires the `experimental` feature — \
         build with `--features experimental` to run it."
    );
}

#[cfg(feature = "experimental")]
mod gated {
    use super::*;
    use oxrdf::Triple;
    use sparq_reason_ql::rewrite;
    use spargebra::algebra::GraphPattern;
    use std::collections::{BTreeMap, BTreeSet};
    use std::str::FromStr;

    // ---- Minimal FAITHFUL UCQ evaluator (same style as broadened_shapes.rs) ----
    // A rewritten CQ/UCQ is a `(Distinct?) Project (Union of Bgp)`; blank nodes and
    // non-distinguished variables match as ordinary non-distinguished BGP variables (SPARQL BGP
    // semantics). Evaluating the emitted query over a concrete ABox and comparing the PROJECTED
    // answers is a faithful check of result-equivalence, invariant to intermediate naming.

    enum Slot {
        Fixed(String),
        Bind(String),
    }

    fn term_slot(t: &TermPattern) -> Slot {
        match t {
            TermPattern::NamedNode(n) => Slot::Fixed(n.to_string()),
            TermPattern::Literal(l) => Slot::Fixed(l.to_string()),
            TermPattern::Variable(v) => Slot::Bind(format!("?{}", v.as_str())),
            TermPattern::BlankNode(b) => Slot::Bind(format!("_:{}", b.as_str())),
            #[allow(unreachable_patterns)]
            other => panic!("unexpected term pattern in rewritten UCQ: {other:?}"),
        }
    }

    fn pred_slot(p: &NamedNodePattern) -> Slot {
        match p {
            NamedNodePattern::NamedNode(n) => Slot::Fixed(n.to_string()),
            NamedNodePattern::Variable(v) => Slot::Bind(format!("?{}", v.as_str())),
        }
    }

    fn bind(
        sol: &BTreeMap<String, String>,
        slot: Slot,
        value: &str,
    ) -> Option<BTreeMap<String, String>> {
        match slot {
            Slot::Fixed(f) => (f == value).then(|| sol.clone()),
            Slot::Bind(k) => match sol.get(&k) {
                Some(existing) if existing != value => None,
                _ => {
                    let mut next = sol.clone();
                    next.insert(k, value.to_string());
                    Some(next)
                }
            },
        }
    }

    fn triples_as_strings(data: &[Triple]) -> Vec<(String, String, String)> {
        data.iter()
            .map(|t| {
                (
                    t.subject.to_string(),
                    t.predicate.to_string(),
                    t.object.to_string(),
                )
            })
            .collect()
    }

    fn eval(gp: &GraphPattern, data: &[(String, String, String)]) -> Vec<BTreeMap<String, String>> {
        match gp {
            GraphPattern::Bgp { patterns } => {
                let mut sols = vec![BTreeMap::new()];
                for pat in patterns {
                    let mut next = Vec::new();
                    for sol in &sols {
                        for (s, p, o) in data {
                            let Some(a) = bind(sol, term_slot(&pat.subject), s) else {
                                continue;
                            };
                            let Some(b) = bind(&a, pred_slot(&pat.predicate), p) else {
                                continue;
                            };
                            if let Some(c) = bind(&b, term_slot(&pat.object), o) {
                                next.push(c);
                            }
                        }
                    }
                    sols = next;
                }
                sols
            }
            GraphPattern::Join { left, right } => {
                // The emitter never produces a Join in a rewritten CQ (each disjunct is one Bgp),
                // but handle it faithfully (natural join on shared keys) for robustness.
                let ls = eval(left, data);
                let rs = eval(right, data);
                let mut out = Vec::new();
                for l in &ls {
                    for r in &rs {
                        let mut merged = l.clone();
                        let mut ok = true;
                        for (k, v) in r {
                            match merged.get(k) {
                                Some(existing) if existing != v => {
                                    ok = false;
                                    break;
                                }
                                _ => {
                                    merged.insert(k.clone(), v.clone());
                                }
                            }
                        }
                        if ok {
                            out.push(merged);
                        }
                    }
                }
                out
            }
            GraphPattern::Union { left, right } => {
                let mut sols = eval(left, data);
                sols.extend(eval(right, data));
                sols
            }
            GraphPattern::Project { inner, variables } => {
                let keep: BTreeSet<String> =
                    variables.iter().map(|v| format!("?{}", v.as_str())).collect();
                eval(inner, data)
                    .into_iter()
                    .map(|sol| sol.into_iter().filter(|(k, _)| keep.contains(k)).collect())
                    .collect()
            }
            GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. } => eval(inner, data),
            other => panic!("unexpected graph pattern in rewritten UCQ: {other:?}"),
        }
    }

    fn top_pattern(query: &Query) -> &GraphPattern {
        match query {
            Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
            other => panic!("rewritten query is neither SELECT nor ASK: {other:?}"),
        }
    }

    fn projection(query: &Query) -> Vec<String> {
        fn find(gp: &GraphPattern) -> Option<Vec<String>> {
            match gp {
                GraphPattern::Project { variables, .. } => {
                    Some(variables.iter().map(|v| format!("?{}", v.as_str())).collect())
                }
                GraphPattern::Distinct { inner }
                | GraphPattern::Reduced { inner }
                | GraphPattern::Slice { inner, .. } => find(inner),
                _ => None,
            }
        }
        find(top_pattern(query)).expect("rewritten SELECT must carry a Project node")
    }

    fn answers(query: &Query, data: &[Triple]) -> BTreeSet<Vec<String>> {
        let strs = triples_as_strings(data);
        let vars = projection(query);
        eval(top_pattern(query), &strs)
            .into_iter()
            .map(|sol| {
                vars.iter()
                    .map(|v| sol.get(v).cloned().unwrap_or_else(|| "<<unbound>>".into()))
                    .collect()
            })
            .collect()
    }

    fn nt(lines: &[&str]) -> Vec<Triple> {
        lines.iter().map(|l| Triple::from_str(l).unwrap()).collect()
    }

    fn iri(local: &str) -> String {
        format!("<http://ex/{local}>")
    }

    // A fixture TBox with a role inclusion, so the rewrite is NON-trivial (PerfectRef fires on
    // the desugared branches, not just an identity copy): `:sub rdfs:subPropertyOf :p1`.
    fn fixture_tbox() -> Vec<Triple> {
        nt(&["<http://ex/sub> <http://www.w3.org/2000/01/rdf-schema#subPropertyOf> <http://ex/p1> ."])
    }

    // A fixture ABox exercising every path form.
    fn fixture_abox() -> Vec<Triple> {
        nt(&[
            "<http://ex/x1> <http://ex/p1> <http://ex/y1> .", // direct p1
            "<http://ex/x2> <http://ex/sub> <http://ex/y2> .", // sub ⊑ p1 (certain p1)
            "<http://ex/x3> <http://ex/p2> <http://ex/y3> .", // p2
            "<http://ex/x4> <http://ex/c>  <http://ex/y4> .", // c
            "<http://ex/x5> <http://ex/a>  <http://ex/m5> .", // a/b chain
            "<http://ex/m5> <http://ex/b>  <http://ex/y5> .",
            "<http://ex/rs> <http://ex/r>  <http://ex/ro> .", // for inverse
            "<http://ex/x6> <http://ex/a>  <http://ex/m6> .", // dangling a (no matching b)
        ])
    }

    /// Rewrite BOTH `path_query` and `hand_query` under `tbox`, evaluate over `abox`, and assert
    /// the projected answer sets are EQUAL and equal to `oracle` — the load-bearing invariant.
    fn assert_result_equivalent(
        path_query: &str,
        hand_query: &str,
        tbox: &[Triple],
        abox: &[Triple],
        oracle: BTreeSet<Vec<String>>,
    ) {
        let rp = rewrite(&q(path_query), tbox).expect("path form must rewrite");
        let rh = rewrite(&q(hand_query), tbox).expect("hand form must rewrite");
        let ap = answers(&rp.query, abox);
        let ah = answers(&rh.query, abox);
        assert_eq!(
            ap, ah,
            "path form and hand-desugared form must be RESULT-EQUIVALENT\n path: {}\n hand: {}\n path answers: {:?}\n hand answers: {:?}",
            rp.query, rh.query, ap, ah
        );
        assert_eq!(
            ap, oracle,
            "path form must match the hand-derived oracle\n rewritten: {}\n got: {:?}\n oracle: {:?}",
            rp.query, ap, oracle
        );
    }

    fn pairs(ps: &[(&str, &str)]) -> BTreeSet<Vec<String>> {
        ps.iter().map(|(a, b)| vec![iri(a), iri(b)]).collect()
    }

    #[test]
    fn alternation_result_equivalent_to_union() {
        // `?x :p1|:p2 ?y`  ≡  `{ ?x :p1 ?y } UNION { ?x :p2 ?y }`.
        // Under {sub ⊑ p1}: p1 matches x1/y1 and (via sub) x2/y2; p2 matches x3/y3.
        assert_result_equivalent(
            &format!("{PRE} SELECT ?x ?y WHERE {{ ?x :p1|:p2 ?y }}"),
            &format!("{PRE} SELECT ?x ?y WHERE {{ {{ ?x :p1 ?y }} UNION {{ ?x :p2 ?y }} }}"),
            &fixture_tbox(),
            &fixture_abox(),
            pairs(&[("x1", "y1"), ("x2", "y2"), ("x3", "y3")]),
        );
    }

    #[test]
    fn sequence_result_equivalent_to_named_intermediate() {
        // `?x :a/:b ?y`  ≡  `?x :a ?mid . ?mid :b ?y` (fresh non-distinguished ?mid).
        // Only x5→m5→y5 forms a full chain; the dangling x6→m6 yields no answer.
        assert_result_equivalent(
            &format!("{PRE} SELECT ?x ?y WHERE {{ ?x :a/:b ?y }}"),
            &format!("{PRE} SELECT ?x ?y WHERE {{ ?x :a ?mid . ?mid :b ?y }}"),
            &fixture_tbox(),
            &fixture_abox(),
            pairs(&[("x5", "y5")]),
        );
    }

    #[test]
    fn inverse_result_equivalent_to_swapped_triple() {
        // `?x ^:r ?y`  ≡  `?y :r ?x`. Data `rs :r ro` binds ?x=ro, ?y=rs.
        assert_result_equivalent(
            &format!("{PRE} SELECT ?x ?y WHERE {{ ?x ^:r ?y }}"),
            &format!("{PRE} SELECT ?x ?y WHERE {{ ?y :r ?x }}"),
            &fixture_tbox(),
            &fixture_abox(),
            pairs(&[("ro", "rs")]),
        );
    }

    #[test]
    fn alternation_of_sequence_result_equivalent() {
        // `?x (:a/:b)|:c ?y`  ≡  `{ ?x :a ?mid . ?mid :b ?y } UNION { ?x :c ?y }`.
        assert_result_equivalent(
            &format!("{PRE} SELECT ?x ?y WHERE {{ ?x (:a/:b)|:c ?y }}"),
            &format!(
                "{PRE} SELECT ?x ?y WHERE {{ {{ ?x :a ?mid . ?mid :b ?y }} UNION {{ ?x :c ?y }} }}"
            ),
            &fixture_tbox(),
            &fixture_abox(),
            pairs(&[("x5", "y5"), ("x4", "y4")]),
        );
    }

    #[test]
    fn inverse_in_alternation_result_equivalent() {
        // `?x ^:p1|:p2 ?y`  ≡  `{ ?y :p1 ?x } UNION { ?x :p2 ?y }`.
        // Under {sub ⊑ p1}, the inverse-p1 branch also matches via sub: `?y :sub ?x`.
        // Data: x1 p1 y1 → (?x=y1,?y=x1); x2 sub y2 → (?x=y2,?y=x2); x3 p2 y3 → (?x=x3,?y=y3).
        assert_result_equivalent(
            &format!("{PRE} SELECT ?x ?y WHERE {{ ?x ^:p1|:p2 ?y }}"),
            &format!("{PRE} SELECT ?x ?y WHERE {{ {{ ?y :p1 ?x }} UNION {{ ?x :p2 ?y }} }}"),
            &fixture_tbox(),
            &fixture_abox(),
            pairs(&[("y1", "x1"), ("y2", "x2"), ("x3", "y3")]),
        );
    }

    #[test]
    fn ask_over_alternation_result_equivalent() {
        // ASK form: `ASK { ?x :p1|:p2 ?y }` is true iff some p1/p2 (or sub) edge exists.
        let path = q(&format!("{PRE} ASK {{ ?x :p1|:p2 ?y }}"));
        let hand = q(&format!("{PRE} ASK {{ {{ ?x :p1 ?y }} UNION {{ ?x :p2 ?y }} }}"));
        let tb = fixture_tbox();
        let ab = fixture_abox();
        let rp = rewrite(&path, &tb).unwrap();
        let rh = rewrite(&hand, &tb).unwrap();
        // ASK answer = non-empty solution set.
        let ansp = !eval(top_pattern(&rp.query), &triples_as_strings(&ab)).is_empty();
        let ansh = !eval(top_pattern(&rh.query), &triples_as_strings(&ab)).is_empty();
        assert!(ansp && ansh, "both ASK forms must be TRUE over the fixture ABox");
    }

    #[test]
    fn alternation_with_filter_now_answered() {
        // A path alternation combined with a FILTER produces a multi-branch UCQ whose branches EACH
        // carry the FILTER the #1671 DNF pass distributed into them. The branch-aware emitter
        // (sq-sg542) now ANSWERS it — each branch emits the FILTER over ITS OWN sub-union (formerly
        // rejected fail-closed here). Acceptance + structure are pinned here; the full
        // result-equivalence oracle (with a FILTER-capable evaluator) is in
        // tests/branch_aware_emit.rs. sq-sg542
        let query = q(&format!("{PRE} SELECT ?x WHERE {{ ?x :p1|:p2 ?y FILTER(?x != :Bad) }}"));
        let r = rewrite(&query, &fixture_tbox()).expect("alternation + FILTER must now rewrite");
        assert!(
            r.query.to_string().to_uppercase().contains("FILTER"),
            "the distributed FILTER must appear in the rewritten UCQ; got: {}",
            r.query
        );
    }

    #[test]
    fn recursive_and_negated_forms_stay_rejected_under_rewrite() {
        for path in ["?x :r+ ?y", "?x :r* ?y", "?x :r? ?y", "?x !:r ?y"] {
            let query = q(&format!("{PRE} SELECT ?x ?y WHERE {{ {path} }}"));
            let err = rewrite(&query, &fixture_tbox()).unwrap_err();
            assert!(
                matches!(err, CqError::OutOfScope(_)),
                "recursive/negated path `{}` must be rejected fail-closed under rewrite; got {:?}",
                path,
                err
            );
        }
    }
}
