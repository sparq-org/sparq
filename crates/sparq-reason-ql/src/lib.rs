#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // sq-t5bne: crate has zero `unsafe`.

//! See the crate README (rendered above) for the capability overview and the honest scope.
//!
//! The load-bearing entry points are [`as_conjunctive_query`] and [`as_ucq`] (the always-present,
//! fail-closed **CQ/UCQ-shape gates**) and, behind the off-by-default `experimental` feature,
//! `rewrite` (the baseline PerfectRef DL-Lite_R query rewriter) and `rewrite_production` (the
//! production path: PerfectRef + tree-witness folding + UCQ-containment minimisation). Behind the
//! off-by-default `ql-consistency` feature (which pulls `experimental`), `check_consistency` /
//! `check_consistency_with` decide DL-Lite_R KB consistency by violation-query composition
//! (sq-p6yb7). All are plain code spans here, not intra-doc links, since those items are absent
//! from the default-feature doc surface. The algorithm lives in six modules: `cq` (the gate),
//! `dllite` (TBox extraction), `perfectref` (the rewrite/reduce saturation), `treewitness`
//! (bounded existential-witness folding), `minimise` (UCQ-containment minimisation by
//! homomorphism), and `consistency` (the violation-query satisfiability check).
//!
//! The rewriter is validated two independent ways: syntactically against a hand-checked DL-Lite_R
//! oracle, and executably against an OWL 2 RL materialise-then-query baseline
//! (`tests/rl_baseline_differential.rs`, sq-wxaas) — on fixtures inside the RL ∩ QL overlap the
//! rewrite-the-query and close-the-data strategies must return identical answers, while an
//! existential-generating axiom (which has no OWL 2 RL superclass form) pins the direction of the
//! profiles' legitimate divergence: QL strictly richer, never poorer. On the FORMAL DL-Lite_R
//! suite (the hand-derived certain-answer oracle from sq-g19x0) the rewrite is sound AND complete
//! case by case — that has GRADUATED to a pinned floor (sq-qo1a9): `sparq-conformance`'s
//! `ql_dllite_suite` runner rewrites each case and asserts its UCQ, evaluated over the unmodified
//! ABox, returns exactly the hand-derived certain answers; it is a `sparq extension` row in the
//! central scoreboard (tallied separately, NOT a full-OWL-2-QL-conformance claim, since no runnable
//! normative W3C QL certain-answer suite exists). The BROADER `pr:QL` `sparql11/entailment` arm
//! (sq-kuvu3, opt-in `sparq-conformance/ql-experimental`) stays experimental / OutOfScope — it
//! mixes intensional cases outside sound rewriting, so the harness reports honestly what it
//! computes (fail-closed abstain / computed-equivalent evidence / computed-divergent gap), never a
//! graduated conformance pass — see the README.

#[cfg(feature = "ql-consistency")]
mod consistency;
mod cq;
mod dllite;
#[cfg(feature = "experimental")]
mod emit;
#[cfg(feature = "experimental")]
mod minimise;
#[cfg(feature = "experimental")]
mod perfectref;
#[cfg(feature = "experimental")]
mod treewitness;

#[cfg(feature = "ql-consistency")]
pub use consistency::{
    check_consistency, check_consistency_with, QlConsistency, QlConsistencyGap, QlViolation,
};
pub use cq::{as_conjunctive_query, as_ucq, ConjunctiveQuery, CqError, Ucq, ValuesBlock};
pub use dllite::{Basic, ConceptInclusion, NegativeInclusion, Role, TBox};

#[cfg(feature = "experimental")]
use spargebra::algebra::GraphPattern;
#[cfg(feature = "experimental")]
use spargebra::Query;

/// The outcome of [`rewrite`]: either the rewritten UCQ query plus a [`RewriteReport`], or an
/// honest reason the input was out of QL rewriting scope.
#[cfg(feature = "experimental")]
#[derive(Clone, Debug)]
pub struct Rewritten {
    /// The rewritten SPARQL query — a union of conjunctive queries under the original
    /// projection/distinct — ready to run UNCHANGED through the engine's query path.
    pub query: Query,
    /// What the TBox extraction and rewrite produced (disjunct count, skipped non-QL axioms).
    pub report: RewriteReport,
}

/// A tally of the rewrite, so a caller can honestly surface what happened.
#[cfg(feature = "experimental")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RewriteReport {
    /// Number of conjunctive queries in the emitted UCQ. For [`rewrite`] this is the raw
    /// PerfectRef UCQ size (1 = the TBox added nothing); for [`rewrite_production`] this is the
    /// MINIMISED UCQ size (the count actually emitted into the query).
    pub disjuncts: usize,
    /// TBox axioms outside the DL-Lite_R (OWL 2 QL) fragment that were IGNORED for rewriting
    /// (never silently applied). See [`TBox::skipped`].
    pub skipped_axioms: usize,
    /// The UCQ size BEFORE containment minimisation (PerfectRef + tree-witness foldings). For
    /// [`rewrite`] this equals `disjuncts` (no minimisation is applied); for [`rewrite_production`]
    /// it is the pre-minimisation count, so `disjuncts_before_minimisation - disjuncts` is the
    /// number of redundant disjuncts dropped. Never below `disjuncts` (minimisation only removes).
    pub disjuncts_before_minimisation: usize,
}

/// Rewrite a conjunctive SPARQL query into the **union of conjunctive queries** whose evaluation
/// over the **unmodified data** returns the **certain answers** under the DL-Lite_R (OWL 2 QL)
/// TBox extracted from `tbox` (a slice of RDF triples carrying the schema). EXPERIMENTAL.
///
/// **UCQ input (B1):** a top-level `UNION` of CQ branches is also accepted; each branch is
/// rewritten independently and the results unioned (certain answers distribute over union in
/// DL-Lite_R). Use `rewrite` directly; the gate classifies the input via `as_ucq`.
///
/// **Literal-object atoms (B2):** role atoms with literal constants in the object position are
/// now accepted (e.g. `?x foaf:name "Alice"@en`). A literal is never an unbound position, so
/// the applicability condition is untouched; role inclusions carry it unchanged.
///
/// **FILTER on distinguished-only variables (B3):** a `FILTER(expr)` that mentions only
/// projected (answer) variables is passed through unmodified and re-applied after rewriting.
///
/// **Constant-only VALUES over distinguished variables (B4):** a `VALUES` block whose
/// variables are all distinguished and whose rows are fully bound constants is passed through
/// and re-applied after rewriting. `UNDEF` cells or non-distinguished variables → rejected.
///
/// **Per-branch FILTER/VALUES in a multi-branch UCQ (sq-sg542):** when the UCQ has more than one
/// branch and a branch carries its own FILTER/VALUES — e.g. an alternation path under a FILTER
/// (`?x :p1|:p2 ?y FILTER(?x != :Bad)`, whose desugaring distributes the FILTER into every
/// branch), or a hand-written `{ … FILTER } UNION { … }` — each branch emits ITS OWN modifiers
/// over ITS OWN sub-union, so a branch's FILTER constrains only that branch (never hoisted over
/// the whole union, never dropped). Each branch's modifier still obeys the B3/B4 distinguished-only
/// discipline. This shape was previously rejected fail-closed; it is now soundly answered.
///
/// **Intensional-atom guard (B6):** a role atom whose predicate is semantics-bearing schema
/// vocabulary (`rdfs:subClassOf`, `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`, any
/// `owl:` predicate) is **rejected** as `OutOfScope` — the rewriter evaluates over ABox data
/// only and would silently under-answer such a query. Annotation predicates (`rdfs:label`,
/// `rdfs:comment`, `rdfs:seeAlso`, `rdfs:isDefinedBy`) remain admitted as plain role atoms.
///
/// FAIL-CLOSED: if `query` is not a (union of) conjunctive quer(y/ies) the rewriter is sound
/// for, this returns [`CqError::OutOfScope`] naming the construct — it NEVER silently mis-answers
/// a non-CQ/UCQ query. The gate ([`as_ucq`]) runs first and unconditionally.
///
/// The returned [`Rewritten::query`] runs unchanged through the engine (the spargebra
/// rewrite seam): a UCQ folds to a `Union` tree of `Bgp`s under the original projection.
///
/// Scope (honest): the rewriter handles the POSITIVE DL-Lite_R inclusions
/// (`rdfs:subClassOf`/`subPropertyOf`/`domain`/`range`, `owl:inverseOf`, unqualified `∃R`
/// restrictions). It does NOT minimise the UCQ (no containment check — the deferred production
/// path) and does NOT check consistency. See the crate README for the full boundary.
///
/// ```
/// # #[cfg(feature = "experimental")] {
/// use oxrdf::Triple;
/// use spargebra::SparqlParser;
/// use sparq_reason_ql::rewrite;
/// use std::str::FromStr;
///
/// // TBox: Manager rdfs:subClassOf Employee.
/// let tbox = vec![Triple::from_str(
///     "<http://ex/Manager> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex/Employee> ."
/// ).unwrap()];
/// let q = SparqlParser::new().parse_query(
///     "SELECT ?x WHERE { ?x <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Employee> }"
/// ).unwrap();
/// let r = rewrite(&q, &tbox).unwrap();
/// // The UCQ now also matches Managers, who are certainly Employees.
/// assert_eq!(r.report.disjuncts, 2);
/// # }
/// ```
#[cfg(feature = "experimental")]
pub fn rewrite(query: &Query, tbox: &[oxrdf::Triple]) -> Result<Rewritten, CqError> {
    // 1. FAIL-CLOSED UCQ-shape gate (the soundness keystone — always first).
    let ucq = as_ucq(query)?;
    // 2. Extract the DL-Lite_R TBox once (shared across branches).
    let tbox_model = TBox::extract(tbox);
    // 3. Fold each branch's PerfectRef UCQ back into a spargebra body.
    //
    //    BRANCH-AWARE emission (sq-sg542): when the UCQ has MORE THAN ONE branch AND at least one
    //    branch carries a per-branch FILTER/VALUES — e.g. `?x :p1|:p2 ?y FILTER(?x != :Bad)`, whose
    //    #1671 alternation desugaring distributes the top-level FILTER into EACH branch, or a
    //    hand-written `{ ?x a :A FILTER(...) } UNION { ?x a :B }` — each branch emits ITS OWN
    //    modifiers over ITS OWN sub-union (`emit::ucq_to_pattern_per_branch`). A branch's FILTER
    //    then constrains only that branch, never hoisted over the whole union (the old
    //    single-passthrough emitter would have applied branch[0]'s filter to every branch AND
    //    dropped later branches' filters — the unsoundness this bead closes, formerly rejected
    //    fail-closed here). The gate enforces the distinguished-only B3/B4 discipline PER BRANCH,
    //    so every such multi-branch-modifier UCQ is now soundly ANSWERED rather than abstained.
    if ucq.branches.len() > 1 && any_branch_has_modifier(&ucq) {
        let mut per_branch: Vec<(Vec<perfectref::Cq>, &ConjunctiveQuery)> =
            Vec::with_capacity(ucq.branches.len());
        let mut disjuncts = 0usize;
        for branch in &ucq.branches {
            let (atoms, answer) = emit::cq_to_atoms(branch)?;
            let branch_ucq = perfectref::perfect_ref(atoms, answer, &tbox_model);
            disjuncts += branch_ucq.len();
            per_branch.push((branch_ucq, branch));
        }
        let body = emit::ucq_to_pattern_per_branch(per_branch);
        let rewritten = rewrap(query, &ucq, body);
        return Ok(Rewritten {
            query: rewritten,
            report: RewriteReport {
                disjuncts,
                skipped_axioms: tbox_model.skipped,
                disjuncts_before_minimisation: disjuncts,
            },
        });
    }

    //    FLAT path — a single-branch UCQ (the one branch IS the whole query, so its own
    //    filter_exprs/values_blocks are re-applied soundly by `ucq_to_pattern`) OR a multi-branch
    //    UCQ with NO per-branch modifier (branch[0] carries nothing to hoist). Byte-identical to
    //    the pre-sq-sg542 emission for every previously-accepted shape.
    let mut all_disjuncts: Vec<perfectref::Cq> = Vec::new();
    for branch in &ucq.branches {
        let (atoms, answer) = emit::cq_to_atoms(branch)?;
        let branch_ucq = perfectref::perfect_ref(atoms, answer, &tbox_model);
        all_disjuncts.extend(branch_ucq);
    }
    let disjuncts = all_disjuncts.len();
    let cq_for_passthrough = &ucq.branches[0];
    let body = emit::ucq_to_pattern(all_disjuncts, cq_for_passthrough);
    let rewritten = rewrap(query, &ucq, body);
    Ok(Rewritten {
        query: rewritten,
        report: RewriteReport {
            disjuncts,
            skipped_axioms: tbox_model.skipped,
            disjuncts_before_minimisation: disjuncts,
        },
    })
}

/// Whether any branch of `ucq` carries a per-branch FILTER or VALUES — the condition that
/// selects the BRANCH-AWARE emitter (`emit::ucq_to_pattern_per_branch`, sq-sg542) over the flat
/// single-passthrough fold. A single-branch UCQ never needs it (the branch IS the whole query).
#[cfg(feature = "experimental")]
fn any_branch_has_modifier(ucq: &Ucq) -> bool {
    ucq.branches
        .iter()
        .any(|b| !b.filter_exprs.is_empty() || !b.values_blocks.is_empty())
}

/// Rewrite a conjunctive SPARQL query into the **minimised** union of conjunctive queries via the
/// PRODUCTION path: baseline PerfectRef saturation, **augmented** with bounded **tree-witness**
/// folding (existential witnesses captured without an unbounded chase), then **UCQ-containment
/// minimisation** (redundant disjuncts dropped by the homomorphism containment test). EXPERIMENTAL.
///
/// All broadened shapes from [`rewrite`] (B1 UCQ, B2 literal atoms, B3 FILTER, B4 VALUES, B6
/// intensional-atom guard) apply here as well.
///
/// SOUNDNESS — the certain-answer set is IDENTICAL to [`rewrite`]'s. PerfectRef is sound +
/// complete, so the augmented pre-minimisation UCQ has at least PerfectRef's answers; tree-witness
/// folding only adds disjuncts that are themselves PerfectRef-derivable (so it adds no NEW
/// answers); and minimisation drops only a disjunct **contained** in a retained one (so it removes
/// no answers). The net result returns exactly the certain answers, in a SMALLER UCQ. The
/// containment check is NP-complete and **fail-closed**: when containment cannot be decided within
/// the search budget the disjunct is KEPT (never dropped on uncertainty) — an over-aggressive
/// minimisation that dropped a non-contained disjunct would be an unsoundness bug.
///
/// FAIL-CLOSED CQ/UCQ-shape gate as in [`rewrite`]: a non-CQ/UCQ query is rejected as
/// [`CqError::OutOfScope`], never mis-answered.
///
/// ```
/// # #[cfg(feature = "experimental")] {
/// use oxrdf::Triple;
/// use spargebra::SparqlParser;
/// use sparq_reason_ql::rewrite_production;
/// use std::str::FromStr;
///
/// // TBox: Manager rdfs:subClassOf Employee.
/// let tbox = vec![Triple::from_str(
///     "<http://ex/Manager> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex/Employee> ."
/// ).unwrap()];
/// let q = SparqlParser::new().parse_query(
///     "SELECT ?x WHERE { ?x <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Employee> }"
/// ).unwrap();
/// let r = rewrite_production(&q, &tbox).unwrap();
/// // Employee + Manager: two incomparable disjuncts; nothing is redundant, so none is dropped.
/// assert_eq!(r.report.disjuncts, 2);
/// assert_eq!(r.report.disjuncts_before_minimisation, 2);
/// # }
/// ```
#[cfg(feature = "experimental")]
pub fn rewrite_production(query: &Query, tbox: &[oxrdf::Triple]) -> Result<Rewritten, CqError> {
    // 1. FAIL-CLOSED UCQ-shape gate (the soundness keystone — always first).
    let ucq = as_ucq(query)?;
    // 2. Extract the DL-Lite_R TBox once (shared across branches).
    let tbox_model = TBox::extract(tbox);
    let answer_for_minimise: Vec<String> = ucq
        .distinguished
        .iter()
        .map(|v| v.as_str().to_string())
        .collect();

    // 3. BRANCH-AWARE production path (sq-sg542): a multi-branch UCQ where any branch carries a
    //    per-branch FILTER/VALUES is minimised PER BRANCH — cross-branch minimisation would be
    //    unsound here, since a disjunct in branch i carries branch i's modifier and is NOT
    //    interchangeable with a disjunct in branch j — then each branch's minimal UCQ is wrapped in
    //    ITS OWN modifier and the branches are top-unioned. Same per-branch-isolation soundness as
    //    `rewrite`; minimisation stays fail-closed (a disjunct is dropped only when PROVEN contained
    //    in a RETAINED disjunct of the SAME branch, so no answer of any branch is removed).
    if ucq.branches.len() > 1 && any_branch_has_modifier(&ucq) {
        let mut per_branch: Vec<(Vec<perfectref::Cq>, &ConjunctiveQuery)> =
            Vec::with_capacity(ucq.branches.len());
        let mut before = 0usize;
        let mut disjuncts = 0usize;
        for branch in &ucq.branches {
            let (atoms, answer) = emit::cq_to_atoms(branch)?;
            let perfect = perfectref::perfect_ref(atoms, answer.clone(), &tbox_model);
            let mut augmented: Vec<perfectref::Cq> = Vec::new();
            for disjunct in perfect {
                for folded in treewitness::tree_witness_ucq(disjunct, &tbox_model) {
                    augmented.push(folded);
                }
            }
            // De-duplicate structurally WITHIN the branch (honest pre-minimisation tally).
            let mut seen = rustc_hash::FxHashSet::default();
            augmented.retain(|c| seen.insert(c.clone()));
            before += augmented.len();
            let minimal = minimise::minimise_ucq(augmented, &answer_for_minimise);
            disjuncts += minimal.len();
            per_branch.push((minimal, branch));
        }
        let body = emit::ucq_to_pattern_per_branch(per_branch);
        let rewritten = rewrap(query, &ucq, body);
        return Ok(Rewritten {
            query: rewritten,
            report: RewriteReport {
                disjuncts,
                skipped_axioms: tbox_model.skipped,
                disjuncts_before_minimisation: before,
            },
        });
    }

    // 4. FLAT path (unchanged): cross-branch dedup + containment minimisation for a single-branch
    //    UCQ or a multi-branch UCQ with NO per-branch modifier. Byte-identical to the pre-sq-sg542
    //    emission for every previously-accepted shape.
    let mut augmented: Vec<perfectref::Cq> = Vec::new();
    for branch in &ucq.branches {
        let (atoms, answer) = emit::cq_to_atoms(branch)?;
        let perfect = perfectref::perfect_ref(atoms, answer.clone(), &tbox_model);
        for disjunct in perfect {
            for folded in treewitness::tree_witness_ucq(disjunct, &tbox_model) {
                augmented.push(folded);
            }
        }
    }
    let before = {
        // De-duplicate structurally for the honest "before minimisation" tally.
        let mut seen = rustc_hash::FxHashSet::default();
        augmented.retain(|c| seen.insert(c.clone()));
        augmented.len()
    };
    let minimal = minimise::minimise_ucq(augmented, &answer_for_minimise);
    let disjuncts = minimal.len();
    let cq_for_passthrough = &ucq.branches[0];
    let body = emit::ucq_to_pattern(minimal, cq_for_passthrough);
    let rewritten = rewrap(query, &ucq, body);
    Ok(Rewritten {
        query: rewritten,
        report: RewriteReport {
            disjuncts,
            skipped_axioms: tbox_model.skipped,
            disjuncts_before_minimisation: before,
        },
    })
}

/// Re-wrap a rewritten UCQ `body` in the original query's projection + DISTINCT, preserving
/// SELECT-vs-ASK and the projected variables. The dataset/base-IRI are dropped (a rewritten UCQ
/// runs against the same default dataset; carrying a custom dataset through is out of MVP scope
/// — the gate accepts only default-dataset CQs implicitly, since GRAPH is rejected).
#[cfg(feature = "experimental")]
fn rewrap(original: &Query, ucq: &Ucq, body: GraphPattern) -> Query {
    use spargebra::term::Variable;
    let projected: Vec<Variable> = ucq.distinguished.clone();
    // Apply DISTINCT first (innermost), then Project — matching spargebra's nesting.
    let inner = if ucq.distinct {
        GraphPattern::Distinct {
            inner: Box::new(body),
        }
    } else {
        body
    };
    match original {
        Query::Ask { .. } => Query::Ask {
            dataset: None,
            pattern: inner,
            base_iri: None,
        },
        _ => Query::Select {
            dataset: None,
            pattern: GraphPattern::Project {
                inner: Box::new(inner),
                variables: projected,
            },
            base_iri: None,
        },
    }
}

#[cfg(all(test, feature = "experimental"))]
mod tests {
    use super::*;
    use oxrdf::Triple;
    use spargebra::SparqlParser;
    use std::str::FromStr;

    fn tbox(nt: &[&str]) -> Vec<Triple> {
        nt.iter().map(|l| Triple::from_str(l).unwrap()).collect()
    }

    fn q(s: &str) -> Query {
        SparqlParser::new().parse_query(s).unwrap()
    }

    const RDFS_SUB: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
    const TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

    #[test]
    fn subclass_doubles_the_ucq() {
        let t = tbox(&[&format!("<http://ex/A> <{RDFS_SUB}> <http://ex/B> .")]);
        let query = q(&format!("SELECT ?x WHERE {{ ?x <{TYPE}> <http://ex/B> }}"));
        let r = rewrite(&query, &t).unwrap();
        assert_eq!(r.report.disjuncts, 2);
        // The rewritten query must serialise as a UNION (sanity that the fold happened).
        assert!(r.query.to_string().to_uppercase().contains("UNION"));
    }

    #[test]
    fn non_cq_is_rejected_not_rewritten() {
        let query = q(&format!(
            "SELECT ?x WHERE {{ ?x <{TYPE}> <http://ex/B> OPTIONAL {{ ?x <http://ex/r> ?y }} }}"
        ));
        let err = rewrite(&query, &[]).unwrap_err();
        assert!(matches!(err, CqError::OutOfScope(_)));
    }

    #[test]
    fn empty_tbox_is_identity_ucq() {
        let query = q(&format!("SELECT ?x WHERE {{ ?x <{TYPE}> <http://ex/B> }}"));
        let r = rewrite(&query, &[]).unwrap();
        assert_eq!(r.report.disjuncts, 1, "no TBox => UCQ is just the input CQ");
    }

    // sq-pbz04.3.1 — UCQ rewrite (B1): a top-level UNION rewrites each branch.
    #[test]
    fn ucq_input_rewrites_both_branches() {
        let t = tbox(&[&format!(
            "<http://ex/Manager> <{RDFS_SUB}> <http://ex/Employee> ."
        )]);
        let query = q(&format!(
            "SELECT ?x WHERE {{ {{ ?x <{TYPE}> <http://ex/Employee> }} UNION {{ ?x <{TYPE}> <http://ex/Contractor> }} }}"
        ));
        let r = rewrite(&query, &t).unwrap();
        // Employee branch rewrites to {Employee, Manager}; Contractor branch stays {Contractor}.
        // Total: 3 disjuncts.
        assert_eq!(
            r.report.disjuncts, 3,
            "UCQ input: Employee->2 + Contractor->1 = 3; report = {:?}",
            r.report
        );
    }

    // sq-pbz04.3.1 — intensional-atom guard (B6): rdfs:subClassOf as atom predicate
    // must be rejected even though it is a valid IRI constant.
    #[test]
    fn intensional_atom_rejected() {
        let query = q("PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
             PREFIX : <http://ex/> \
             SELECT ?c WHERE { ?c rdfs:subClassOf :A }");
        let err = rewrite(&query, &[]).unwrap_err();
        assert!(
            matches!(err, CqError::OutOfScope(ref r) if r.contains("intensional")),
            "rdfs:subClassOf as atom predicate must be rejected as intensional; got {:?}",
            err
        );
    }

    // ---- BRANCH-AWARE emitter (sq-sg542): multi-branch UCQ with per-branch FILTER/VALUES ----
    // These shapes were previously REJECTED fail-closed (the single-passthrough emitter would have
    // hoisted branch[0]'s modifier over the WHOLE union — constraining branches that do not own it
    // and dropping later branches' modifiers). The branch-aware emitter now ANSWERS them: each
    // branch emits ITS OWN filter/values over ITS OWN sub-union. Full result-equivalence + the
    // leak-in-both-directions probes live in tests/branch_aware_emit.rs (with a faithful UCQ
    // evaluator); these unit tests pin ACCEPTANCE + emitted structure via both entry points.
    // sq-sg542

    /// `SELECT ?x { { ?x a :A FILTER(?x != :Bad) } UNION { ?x a :B } }` — branch[0] carries a
    /// FILTER. The branch-aware emitter wraps it around branch[0]'s sub-union ONLY, so branch[1]
    /// is unconstrained. Accepted + answered (formerly OutOfScope). sq-sg542
    #[test]
    fn union_with_branch_filter_now_answered() {
        let query = q(&format!(
            "PREFIX : <http://ex/> \
             SELECT ?x WHERE {{ \
               {{ ?x <{TYPE}> :A FILTER(?x != :Bad) }} \
               UNION \
               {{ ?x <{TYPE}> :B }} \
             }}"
        ));
        let r = rewrite(&query, &[]).unwrap();
        assert_eq!(
            r.report.disjuncts, 2,
            "two branches, identity rewrite each (no TBox); report = {:?}",
            r.report
        );
        assert!(
            r.query.to_string().to_uppercase().contains("FILTER"),
            "branch[0]'s FILTER must be re-applied in the rewritten query; got: {}",
            r.query
        );
        // The production path accepts + answers it too.
        let r2 = rewrite_production(&query, &[]).unwrap();
        assert_eq!(
            r2.report.disjuncts, 2,
            "rewrite_production: same two branches; report = {:?}",
            r2.report
        );
    }

    /// Branch[1]-filter variant: `SELECT ?x { { ?x a :A } UNION { ?x a :B FILTER(?x != :Bad) } }`.
    /// Branch[1]'s FILTER used to be silently dropped by the single-passthrough emitter; it is now
    /// applied to branch[1]'s sub-union only. Accepted + answered. sq-sg542
    #[test]
    fn union_with_branch1_filter_now_answered() {
        let query = q(&format!(
            "PREFIX : <http://ex/> \
             SELECT ?x WHERE {{ \
               {{ ?x <{TYPE}> :A }} \
               UNION \
               {{ ?x <{TYPE}> :B FILTER(?x != :Bad) }} \
             }}"
        ));
        let r = rewrite(&query, &[]).unwrap();
        assert_eq!(
            r.report.disjuncts, 2,
            "two branches; report = {:?}",
            r.report
        );
        assert!(
            r.query.to_string().to_uppercase().contains("FILTER"),
            "branch[1]'s FILTER must be re-applied; got: {}",
            r.query
        );
    }

    /// `SELECT ?x { { ?x a :A } UNION { ?x a :B VALUES ?x { :C } } }` — branch[1] carries a
    /// VALUES block. It used to be silently dropped by the single-passthrough emitter; the
    /// branch-aware emitter joins it onto branch[1]'s sub-union only. Accepted. sq-sg542
    #[test]
    fn union_with_branch_values_now_answered() {
        let query = q(&format!(
            "PREFIX : <http://ex/> \
             SELECT ?x WHERE {{ \
               {{ ?x <{TYPE}> :A }} \
               UNION \
               {{ ?x <{TYPE}> :B VALUES ?x {{ :C }} }} \
             }}"
        ));
        let r = rewrite(&query, &[]).unwrap();
        assert_eq!(
            r.report.disjuncts, 2,
            "two branches; report = {:?}",
            r.report
        );
        assert!(
            r.query.to_string().to_uppercase().contains("VALUES"),
            "branch[1]'s VALUES must be re-applied; got: {}",
            r.query
        );
        let r2 = rewrite_production(&query, &[]).unwrap();
        assert_eq!(
            r2.report.disjuncts, 2,
            "rewrite_production: same; report = {:?}",
            r2.report
        );
    }

    /// The SOUND case: a single-branch CQ with a FILTER on the distinguished variable
    /// must still be accepted, and the rewritten query must return the original answers.
    /// (branch[0] IS the whole query, so hoisting its filter is correct.)
    #[test]
    fn single_branch_filter_still_accepted_and_sound() {
        // No TBox — identity UCQ (1 disjunct). The FILTER is passed through.
        let query = q(&format!(
            "PREFIX : <http://ex/> \
             SELECT ?x WHERE {{ ?x <{TYPE}> :A FILTER(?x != :Bad) }}"
        ));
        let r = rewrite(&query, &[]).unwrap();
        // 1 disjunct (identity with no TBox), and the rewritten query must contain FILTER.
        assert_eq!(
            r.report.disjuncts, 1,
            "single-branch FILTER: identity UCQ (no TBox) = 1 disjunct; report = {:?}",
            r.report
        );
        assert!(
            r.query.to_string().to_uppercase().contains("FILTER"),
            "FILTER must be re-applied in the rewritten query; got: {}",
            r.query
        );
        // rewrite_production must also accept the single-branch case.
        let r2 = rewrite_production(&query, &[]).unwrap();
        assert_eq!(
            r2.report.disjuncts, 1,
            "rewrite_production: same single-branch result"
        );
    }
}
