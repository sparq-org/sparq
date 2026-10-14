//! **TLP under SET semantics: the `DISTINCT` partition variant.** sq-gum8.12
//!
//! The shipped [`crate::tlp`] oracle recombines the three partitions with **multiset**
//! union, which is why its scope preconditions exclude `DISTINCT`/`REDUCED`. That
//! exclusion is a statement about the *recombination law*, not about testability:
//! `DISTINCT` is perfectly partitionable once the law is re-derived, and the derivation
//! below shows the union becomes a **set** union — with a *strictly stronger*
//! (multiset) law recoverable in the one case where the partitions are provably
//! disjoint.
//!
//! # Derivation
//!
//! Write `Ω = eval(P)` for the solution **bag** of the group graph pattern, and split it
//! on the effective boolean value of the deterministic filter expression `c`:
//!
//! ```text
//! Ω  =  Ω_t ⊎ Ω_f ⊎ Ω_e ,    Ω_x = { μ ∈ Ω | ebv(c, μ) = x }
//! ```
//!
//! This is the same trichotomy [`crate::tlp`] derives (SPARQL 1.1 §17.2.2; total and
//! exclusive because `ebv(c, μ)` is a function of `μ` alone), and the three branch
//! queries append exactly the same three `FILTER` clauses (the shared
//! `tlp::branch_filters` constructor, so the variants cannot drift apart).
//!
//! Let `π_V` be projection onto the variable list `V` and `D` be `Distinct` (SPARQL 1.1
//! §18.5: `Distinct(Ψ)` keeps one copy of each solution mapping — i.e. it maps a bag to
//! its **support set**). Two facts do the work:
//!
//! * `π_V` is applied **per solution mapping**, so it is a bag homomorphism:
//!   `π_V(A ⊎ B) = π_V(A) ⊎ π_V(B)`.
//! * `supp(A ⊎ B) = supp(A) ∪ supp(B)` — the support of a multiset union is the *set*
//!   union of the supports.
//!
//! Composing them:
//!
//! ```text
//! D(π_V(Ω))  =  D(π_V(Ω_t))  ∪  D(π_V(Ω_f))  ∪  D(π_V(Ω_e))
//! ```
//!
//! The union is `∪`, **not** `⊎`, and the difference is not cosmetic: `D` destroys
//! multiset additivity exactly when two solutions in *different* branches project to the
//! same row. Then `|D(π(Ω))| < |D(π(Ω_t))| + |D(π(Ω_f))| + |D(π(Ω_e))|`, so a naive
//! reuse of the multiset law would manufacture a "violation" on a correct engine.
//! `tests/oracle_self_tests.rs` pins exactly that witness against the real engine (the
//! test named `distinct_partitions_recombine_by_set_union_not_multiset_union`), so the
//! choice of `∪` is checked rather than asserted.
//!
//! # The `SELECT DISTINCT *` strengthening (disjointness)
//!
//! With `V` = all in-scope variables (`SELECT DISTINCT *`), `π_V` is the identity and the
//! three sets are **pairwise disjoint**: a solution mapping `μ` in two of them would need
//! `ebv(c, μ)` to take two of {true, false, error} at once, contradicting the trichotomy.
//! A disjoint set union *is* a multiset union, so in that case the stronger law
//!
//! ```text
//! D(Ω)  =  D(Ω_t) ⊎ D(Ω_f) ⊎ D(Ω_e)      (cardinalities add)
//! ```
//!
//! also holds, and [`check_tlp_distinct`] checks it in addition to the set law whenever
//! the projection is `*`. (`FILTER` contributes no bindings and does not change the
//! in-scope variables — §18.2.1 — so all four queries project the same variables and
//! `DISTINCT` compares like-shaped solutions.)
//!
//! # What this widens
//!
//! The base [`crate::tlp`] oracle never exercises an engine's duplicate-elimination
//! path. This variant runs it in four different query shapes and cross-checks that
//! `Distinct` **commutes with the partition** — the surface where a hash-dedup bug
//! (over-eager collapse of two term-distinct rows, a lost row on a hash collision,
//! dedup applied before rather than after a projection) shows up as a wrong result with
//! nothing crashing.
//!
//! # Comparator note (a sensitivity limit, not a soundness hole)
//!
//! SPARQL `Distinct` is defined by **RDF term** equality of solution mappings, whereas
//! the harness keys rows with [`sparq_difftest::canonical_key`], which is *value*
//! canonical (`"01"^^xsd:integer` and `"1"^^xsd:integer` share a key). The coarser key
//! can only merge rows the engine kept apart, so equal exact result sets always have
//! equal key images: the check can **mask** a divergence (a false pass) but can never
//! manufacture one (a false violation). Same regime as the shipped oracles — see
//! [`crate::differential`].
//!
//! # Scope preconditions
//!
//! Preconditions 1, 2 and 4 of [`crate::tlp`] apply unchanged (deterministic `c`, no
//! `EXISTS`/`NOT EXISTS`, top-level filter placement). Precondition 3 is *relaxed* to
//! allow `DISTINCT` and a projection list, and no further: still no `REDUCED` (it is
//! explicitly implementation-defined how many duplicates survive, §18.5, so no law
//! holds), and still no `ORDER BY`/`LIMIT`/`OFFSET` (slicing does not commute with
//! partitioning). `V` must be a subset of the in-scope variables of `P`.
//!
//! `EXISTS` stays excluded here for the same reason as in [`crate::tlp`]: its
//! substitution semantics is a known SPARQL 1.1 defect under revision for SPARQL 1.2, so
//! a "violation" involving it would measure the standard, not the engine. Revisit when
//! SPARQL 1.2 settles it — not before.

use std::collections::BTreeSet;

use sparq_difftest::{canonical_key, multiset_equal, Solution};

use crate::engine::SparqlEngine;
use crate::tlp::{branch_filters, run_select};
use crate::verdict::{OracleKind, Verdict, Violation};

/// The four queries of one `DISTINCT` TLP instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlpDistinctQueries {
    /// `SELECT DISTINCT <V> WHERE { P }` — the unpartitioned base.
    pub base: String,
    /// Keeps `ebv(c, μ) = true`, then deduplicates.
    pub branch_true: String,
    /// Keeps `ebv(c, μ) = false`, then deduplicates.
    pub branch_false: String,
    /// Keeps `ebv(c, μ) = error`, then deduplicates.
    pub branch_error: String,
    /// The projected variable names (no leading `?`); empty means `SELECT DISTINCT *`.
    pub projection: Vec<String>,
}

/// Render a projection list as SPARQL: `*` for an empty list, else `?a ?b …`.
fn render_projection(projection: &[&str]) -> String {
    if projection.is_empty() {
        "*".to_string()
    } else {
        projection
            .iter()
            .map(|v| format!("?{v}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Build the four `DISTINCT` TLP queries for pattern `P`, filter expression `c`, and
/// projection `projection` (variable names without `?`; empty ⇒ `SELECT DISTINCT *`).
/// See the module docs for the recombination law and the scope preconditions.
pub fn tlp_distinct_queries(
    pattern: &str,
    predicate: &str,
    projection: &[&str],
) -> TlpDistinctQueries {
    let proj = render_projection(projection);
    let [keep_true, keep_false, keep_error] = branch_filters(predicate);
    TlpDistinctQueries {
        base: format!("SELECT DISTINCT {proj} WHERE {{ {pattern} }}"),
        branch_true: format!("SELECT DISTINCT {proj} WHERE {{ {pattern} {keep_true} }}"),
        branch_false: format!("SELECT DISTINCT {proj} WHERE {{ {pattern} {keep_false} }}"),
        branch_error: format!("SELECT DISTINCT {proj} WHERE {{ {pattern} {keep_error} }}"),
        projection: projection.iter().map(|v| (*v).to_string()).collect(),
    }
}

/// One solution's value-canonical key: the sorted `(var, term-key)` pairs, held
/// structurally rather than delimiter-joined for the anti-collision reason spelled out
/// in `sparq_difftest::multiset`.
type RowKey = Vec<(String, String)>;

fn row_key(solution: &Solution) -> RowKey {
    // `Solution` is a `BTreeMap`, so iteration is already in sorted-variable order.
    solution
        .iter()
        .map(|(var, term)| (var.clone(), canonical_key(term)))
        .collect()
}

fn row_set(solutions: &[Solution]) -> BTreeSet<RowKey> {
    solutions.iter().map(row_key).collect()
}

/// Check the `DISTINCT` (set-semantics) TLP relation on one engine: the deduplicated
/// base must equal the **set** union of the three deduplicated branches — plus, when the
/// projection is `*`, the stronger disjoint-union (multiset) law.
///
/// Fail-closed: if any of the four queries fails to evaluate, the verdict is
/// [`Verdict::EngineFailure`] (never a pass, never a wrong-result claim).
pub fn check_tlp_distinct(
    engine: &dyn SparqlEngine,
    pattern: &str,
    predicate: &str,
    projection: &[&str],
) -> Verdict {
    let queries = tlp_distinct_queries(pattern, predicate, projection);
    let mut results = Vec::with_capacity(4);
    for query in [
        &queries.base,
        &queries.branch_true,
        &queries.branch_false,
        &queries.branch_error,
    ] {
        match run_select(engine, query) {
            Ok(rows) => results.push(rows),
            Err(failure) => return Verdict::EngineFailure(failure),
        }
    }
    let (base, branches) = results.split_first().expect("four results were pushed");

    let base_set = row_set(base);
    let mut union_set = BTreeSet::new();
    for branch in branches {
        union_set.extend(row_set(branch));
    }
    let detail = format!(
        "base={} (distinct {}) true={} false={} error={} (distinct union {})",
        base.len(),
        base_set.len(),
        branches[0].len(),
        branches[1].len(),
        branches[2].len(),
        union_set.len()
    );
    let violation = |reason: &str| {
        Verdict::Violation(Violation {
            oracle: OracleKind::TlpDistinct,
            engines: vec![engine.name().to_string()],
            queries: vec![
                queries.base.clone(),
                queries.branch_true.clone(),
                queries.branch_false.clone(),
                queries.branch_error.clone(),
            ],
            detail: format!("{reason} ({detail})"),
        })
    };

    if base_set != union_set {
        let base_only = base_set.difference(&union_set).count();
        let branch_only = union_set.difference(&base_set).count();
        return violation(&format!(
            "distinct base differs from the set union of the distinct branches: \
             base-only={base_only} branch-only={branch_only}"
        ));
    }
    // `SELECT DISTINCT *` only: the branches are provably disjoint, so cardinalities
    // must add too (see the module docs). With a projection list they need not.
    if projection.is_empty() {
        let mut union: Vec<Solution> = Vec::new();
        for branch in branches {
            union.extend(branch.iter().cloned());
        }
        if !multiset_equal(base, &union) {
            return violation(
                "SELECT DISTINCT *: the branch partitions must be disjoint, so the \
                 distinct base must equal their multiset union",
            );
        }
    }
    Verdict::Pass { detail }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_queries_carry_distinct_the_projection_and_the_shared_branch_filters() {
        let queries = tlp_distinct_queries("?s <http://example.org/g> ?g", "?g > 5", &["g"]);
        assert_eq!(
            queries.base,
            "SELECT DISTINCT ?g WHERE { ?s <http://example.org/g> ?g }"
        );
        assert!(queries.branch_true.contains("FILTER( ?g > 5 )"));
        assert!(queries.branch_false.contains("FILTER( !( ?g > 5 ) )"));
        assert!(queries
            .branch_error
            .contains("FILTER( COALESCE( IF( ?g > 5 , false, false), true) )"));
        for query in [
            &queries.base,
            &queries.branch_true,
            &queries.branch_false,
            &queries.branch_error,
        ] {
            assert!(query.starts_with("SELECT DISTINCT ?g WHERE"), "{query}");
        }
        assert_eq!(queries.projection, vec!["g".to_string()]);
    }

    #[test]
    fn an_empty_projection_renders_select_distinct_star() {
        let queries = tlp_distinct_queries("?s ?p ?o", "true", &[]);
        assert_eq!(queries.base, "SELECT DISTINCT * WHERE { ?s ?p ?o }");
        assert!(queries.projection.is_empty());
        assert_eq!(render_projection(&["a", "b"]), "?a ?b");
    }
}
