//! [FABLE-5] (sq-7d3dj.30.14) Membership-cluster pre-materialisation for the greedy
//! BGP planner — the opt-in `cluster-materialize` feature (OFF by default).
//!
//! # The shape this fixes (SP2Bench q07)
//!
//! SP2Bench q07's outer BGP contains a **container-membership** pattern with an
//! UNBOUND predicate — the `rdf:_1 rdf:_2 …` bag-membership idiom written
//! `?bag ?member ?doc` — joined against a small "anchor" pattern
//! `?doc2 dcterms:references ?bag` (est ~139 rows) and the rest of the query
//! (`?class subClassOf …`, `?doc rdf:type ?class`, `?doc dc:title ?title`, which
//! binds `?doc` to ~27 000 documents).
//!
//! The greedy GOO planner seeds from the tiny `subClassOf` scan and extends the
//! `?doc` chain; by the time it reaches the membership pattern, `?doc` is bound to
//! ~27 000 values, so it BIND-JOINS the whole 250 000-row unbound-predicate
//! relation per document — the widest permutation, emitting every member row before
//! the selective `references` (est 139) pattern (reachable only through `?bag`) ever
//! prunes it. Evaluated the OTHER way — the two-pattern cluster
//! `{?bag ?member ?doc . ?doc2 references ?bag}` STANDALONE — the anchor bounds
//! `?bag` first and the cluster is a few-thousand-row relation that then joins the
//! `?doc` chain on `?doc`. Profiled on the 250k dataset this is the dominant cost of
//! q07 (the outer BGP dropped ~3× standalone; see the design record §5 and the PR
//! body — non-canonical work-box numbers, RELATIVE attribution only).
//!
//! # What this module does
//!
//! [`detect`] is a PURE, side-effect-free shape test over the already-prepared BGP
//! patterns. It returns a [`ClusterPlan`] partitioning the patterns into a
//! `cluster` (the anchor + membership pair) and the `rest`, OR `None` (DECLINE) when
//! any soundness or cost condition is not met. The executor (in `exec.rs`) then
//! evaluates the two sub-BGPs independently and NATURAL-JOINS them — a BGP is a
//! commutative/associative natural join, so ANY partition yields the SAME bag of
//! rows as the greedy single-plan order (differentially tested,
//! `tests/cluster_materialize_differential.rs`, run in BOTH feature states). The
//! whole module compiles away when the feature is off.
//!
//! # Why this is sound (and where it DECLINES)
//!
//! * **Result-equivalence.** Partitioning a conjunctive BGP into two connected
//!   sub-BGPs and natural-joining them is algebraically identical to any join order
//!   over the flat BGP. This is a pure JOIN-ORDER choice; no row is added or dropped.
//! * **No sideways-information hazard.** Both sub-BGPs are evaluated UNCORRELATED
//!   (no binding is passed from one to the other before the join), so the
//!   materialised cluster is the full UNION-compatible relation the greedy plan
//!   would also have produced — there is no site-specific SIP (#1741) or capped
//!   evaluation (#1781) that could make the two sites disagree, because there is
//!   only ONE evaluation of the cluster.
//! * **Multiplicity.** Natural join preserves bag multiplicity exactly; the cluster
//!   is NOT de-duplicated, so a per-site OPTIONAL that would emit a row several times
//!   still does (the join fans out identically).
//! * **DECLINE** (returns `None`) unless the shape is *exactly* the one profiled:
//!   exactly one unbound-predicate membership pattern, a single small bound-predicate
//!   anchor sharing exactly one variable with it and NOTHING outside the cluster, the
//!   membership pattern still connecting to the rest through another variable, no
//!   pushed-down scan filter on either cluster pattern, and the cost gate (below)
//!   satisfied. Any deviation falls straight through to the unchanged greedy plan.

use crate::exec::Prepared;
use oxrdf::Variable;

/// A partition of a BGP into a pre-materialised `cluster` and the `rest`, by index
/// into the original `prepared`/`patterns` slices. The executor evaluates both
/// sub-BGPs and natural-joins them. Field order carries no meaning — the executor
/// re-plans each side with the normal greedy planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClusterPlan {
    /// Pattern indices forming the standalone cluster (the anchor + membership pair).
    pub(crate) cluster: Vec<usize>,
    /// The remaining pattern indices (the driver chain).
    pub(crate) rest: Vec<usize>,
}

/// Cost-gate thresholds for [`detect`]. These are JOIN-ORDER heuristics only —
/// results are identical for any value (the differential test proves it) — so they
/// are injectable for unit-testing the positive path on a small graph. The executor
/// uses [`Thresholds::PROD`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Thresholds {
    /// Minimum single-pattern estimate for the membership pattern to be a wide
    /// unbound-predicate probe worth hoisting; below this the greedy plan is fine and
    /// hoisting is pure overhead.
    pub(crate) member_min_est: usize,
    /// The membership estimate must exceed the anchor's by at least this factor for
    /// the standalone cluster to be the cheaper evaluation.
    pub(crate) member_anchor_ratio: usize,
    /// Largest anchor estimate we will hoist; a large anchor is not selective enough
    /// to make the standalone cluster small.
    pub(crate) anchor_max_est: usize,
}

impl Thresholds {
    /// Production thresholds (chosen well above any selective bound-predicate scan).
    pub(crate) const PROD: Thresholds = Thresholds {
        member_min_est: 50_000,
        member_anchor_ratio: 16,
        anchor_max_est: 10_000,
    };

    /// Test-only permissive thresholds: fire the cluster path on the small graphs an
    /// integration test can build, so the REAL executor path (`eval_bgp_cluster`) is
    /// exercised — not merely the pure `detect` shape test. Installed thread-locally by
    /// [`with_test_thresholds`]; only meaningful under `cfg(test)`/the doc-hidden hook.
    #[cfg(any(test, feature = "cluster-materialize"))]
    const PERMISSIVE: Thresholds = Thresholds { member_min_est: 1, member_anchor_ratio: 1, anchor_max_est: usize::MAX };
}

#[cfg(feature = "cluster-materialize")]
thread_local! {
    /// When `true`, the executor uses [`Thresholds::PERMISSIVE`] instead of `PROD`, so
    /// a small-graph integration differential can force the cluster path. Test-only; the
    /// production query API never touches it. [FABLE-5]
    static USE_TEST_THRESHOLDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The thresholds the executor should use on this thread: `PERMISSIVE` while a
/// [`with_test_thresholds`] scope is active, else `PROD`.
#[cfg(feature = "cluster-materialize")]
pub(crate) fn active_thresholds() -> Thresholds {
    if USE_TEST_THRESHOLDS.with(|c| c.get()) {
        Thresholds::PERMISSIVE
    } else {
        Thresholds::PROD
    }
}

/// Test-only hook (`#[doc(hidden)]`): run `f` with the permissive thresholds installed,
/// so a small-graph query actually TRIGGERS the cluster path. Restores the prior value
/// on the way out (nestable). Not part of the stable query API. [FABLE-5]
#[cfg(feature = "cluster-materialize")]
#[doc(hidden)]
pub fn with_test_thresholds<R>(f: impl FnOnce() -> R) -> R {
    let prev = USE_TEST_THRESHOLDS.with(|c| c.replace(true));
    let r = f();
    USE_TEST_THRESHOLDS.with(|c| c.set(prev));
    r
}

/// Set of the (up to three) variables of a prepared pattern.
fn pattern_vars(p: &Prepared) -> Vec<&Variable> {
    p.pos_vars.iter().flatten().collect()
}

/// Is this pattern the container-membership shape: an UNBOUND predicate
/// (`id_pat[1] == None`), i.e. `?s ?p ?o` with `?p` a variable? The `rdf:_n` bag
/// members are ordinary triples with distinct predicates, so a bound-predicate
/// scan can never enumerate a whole bag — the membership idiom is exactly this
/// unbound-predicate pattern. (It does NOT assume contiguous `_1.._n` numbering.)
fn is_membership(p: &Prepared) -> bool {
    p.id_pat[1].is_none() && !p.unsatisfiable
}

/// Detect the q07 membership cluster over the prepared BGP. Pure and total: returns
/// `Some(plan)` only when EVERY soundness + cost condition holds, else `None`
/// (DECLINE → unchanged greedy plan). `has_filter(i)` reports whether pattern `i`
/// carries a pushed-down scan filter (a cluster pattern with one is declined so the
/// re-planned sub-BGP cannot lose it).
pub(crate) fn detect(
    prepared: &[Prepared],
    thr: Thresholds,
    has_filter: impl Fn(usize) -> bool,
) -> Option<ClusterPlan> {
    // Need at least anchor + membership + one "rest" pattern (else the cluster is the
    // whole BGP and hoisting is a no-op).
    if prepared.len() < 3 {
        return None;
    }

    // Exactly one unbound-predicate membership pattern. Zero → not this shape; two or
    // more → ambiguous, decline (keep the trigger tight to the profiled case).
    let members: Vec<usize> = (0..prepared.len()).filter(|&i| is_membership(&prepared[i])).collect();
    let [m] = members[..] else { return None };
    if has_filter(m) || prepared[m].est < thr.member_min_est {
        return None;
    }
    let m_vars = pattern_vars(&prepared[m]);
    if m_vars.len() < 2 {
        // Need a shared var with the anchor AND a distinct var connecting to the rest.
        return None;
    }

    // Find the anchor: a small BOUND-predicate pattern that shares EXACTLY ONE
    // variable with the membership pattern and shares NO variable with any pattern
    // outside the candidate cluster {anchor, membership}. Its own non-shared variable
    // (e.g. `?doc2`) must not appear elsewhere, otherwise materialising the cluster
    // standalone would drop a join edge the greedy plan keeps.
    for a in 0..prepared.len() {
        if a == m || prepared[a].id_pat[1].is_none() || has_filter(a) {
            continue;
        }
        if prepared[a].est == 0 || prepared[a].est > thr.anchor_max_est {
            continue;
        }
        if prepared[m].est / prepared[a].est.max(1) < thr.member_anchor_ratio {
            continue;
        }
        let a_vars = pattern_vars(&prepared[a]);
        // Variables shared between anchor and membership.
        let shared: Vec<&Variable> = a_vars.iter().copied().filter(|v| m_vars.contains(v)).collect();
        if shared.len() != 1 {
            continue;
        }
        let shared_var = shared[0];

        // The shared variable must be the membership pattern's INNER (bag) variable —
        // the one that does NOT connect to the rest — and the OTHER membership variable
        // must be the one that connects to the rest. This is what makes the hoist a WIN
        // (the anchor bounds the bag variable to a small set, so the standalone cluster
        // is small) and keeps the trigger tight to the q07 shape: a `?doc rdf:type
        // ?class` pattern that shares the OUTER `?doc` variable is NOT a bag anchor and
        // must not be picked (partitioning there is still result-correct but not the
        // profiled win). `shared_var` connects to the rest ⇒ wrong anchor, skip.
        let shared_connects_rest = (0..prepared.len())
            .any(|j| j != m && j != a && pattern_vars(&prepared[j]).contains(&shared_var));
        if shared_connects_rest {
            continue;
        }

        // The membership pattern must still connect to the REST through a variable
        // other than the shared one (else the cluster IS the whole join component).
        let m_connects_rest = m_vars.iter().any(|&v| {
            v != shared_var
                && (0..prepared.len()).any(|j| j != m && j != a && pattern_vars(&prepared[j]).contains(&v))
        });
        if !m_connects_rest {
            continue;
        }

        // Every anchor variable OTHER than the shared one must be local to the anchor
        // (not used by any pattern outside the cluster). If it leaks, the standalone
        // cluster would drop that join edge — decline (keep it exact).
        let anchor_leaks = a_vars.iter().any(|&v| {
            v != shared_var
                && (0..prepared.len()).any(|j| j != m && j != a && pattern_vars(&prepared[j]).contains(&v))
        });
        if anchor_leaks {
            continue;
        }

        let cluster = vec![a, m];
        let rest: Vec<usize> = (0..prepared.len()).filter(|&i| i != a && i != m).collect();
        return Some(ClusterPlan { cluster, rest });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;
    use spargebra::term::TriplePattern;

    fn g() -> Graph {
        // A tiny SP2Bench-shaped graph: a few Document subclasses, typed docs with
        // titles, and a references/bag/member membership structure — enough for the
        // planner's estimates to have the q07 shape at small scale.
        let nt = r#"<http://ex/InProceedings> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://xmlns.com/foaf/0.1/Document> .
<http://ex/d1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/InProceedings> .
<http://ex/d1> <http://purl.org/dc/elements/1.1/title> "T1" .
<http://ex/bagA> <http://www.w3.org/1999/02/22-rdf-syntax-ns#_1> <http://ex/d1> .
<http://ex/d2> <http://purl.org/dc/terms/references> <http://ex/bagA> .
"#;
        Graph::load_reader(nt.as_bytes(), "ntriples").unwrap()
    }

    fn tp(s: &str) -> Vec<TriplePattern> {
        let q = format!(
            "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
             PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
             PREFIX foaf: <http://xmlns.com/foaf/0.1/> \
             PREFIX dc: <http://purl.org/dc/elements/1.1/> \
             PREFIX dcterms: <http://purl.org/dc/terms/> \
             SELECT * WHERE {{ {} }}",
            s
        );
        let parsed = spargebra::SparqlParser::new().parse_query(&q).unwrap();
        // Pull the flat BGP out of the parsed algebra.
        let mut out = Vec::new();
        collect_bgp(query_pattern(&parsed), &mut out);
        out
    }

    fn query_pattern(q: &spargebra::Query) -> &spargebra::algebra::GraphPattern {
        match q {
            spargebra::Query::Select { pattern, .. } => pattern,
            _ => unreachable!(),
        }
    }

    fn collect_bgp(p: &spargebra::algebra::GraphPattern, out: &mut Vec<TriplePattern>) {
        use spargebra::algebra::GraphPattern as GP;
        match p {
            GP::Bgp { patterns } => out.extend(patterns.iter().cloned()),
            GP::Project { inner, .. } | GP::Distinct { inner } => collect_bgp(inner, out),
            GP::Join { left, right } => {
                collect_bgp(left, out);
                collect_bgp(right, out);
            }
            _ => {}
        }
    }

    // Test thresholds lowered so the small-graph estimates qualify — the SHAPE logic
    // is what the unit test must exercise; the est numbers are join-order-only.
    const TEST_THR: Thresholds = Thresholds { member_min_est: 1, member_anchor_ratio: 1, anchor_max_est: 1_000_000 };

    // The full q07 outer BGP → detects the {references, member} cluster and puts the
    // driver chain in `rest`.
    #[test]
    fn detects_q07_outer_cluster() {
        let pats = tp(
            "?class rdfs:subClassOf foaf:Document . \
             ?doc rdf:type ?class . \
             ?doc dc:title ?title . \
             ?bag2 ?member2 ?doc . \
             ?doc2 dcterms:references ?bag2",
        );
        let prepared = crate::exec::prepare_bgp(&g(), &pats).unwrap();
        // The membership pattern is index 3 (`?bag2 ?member2 ?doc`), anchor index 4.
        assert!(prepared[3].id_pat[1].is_none(), "membership pattern is unbound-predicate");
        assert!(prepared[4].id_pat[1].is_some(), "anchor pattern is bound-predicate");
        let plan = detect(&prepared, TEST_THR, |_| false).expect("q07 cluster detected");
        let mut cluster = plan.cluster.clone();
        cluster.sort_unstable();
        assert_eq!(cluster, vec![3, 4], "cluster is {{member, references-anchor}}");
        let mut rest = plan.rest.clone();
        rest.sort_unstable();
        assert_eq!(rest, vec![0, 1, 2], "rest is the subClassOf/type/title chain");
    }

    // No unbound-predicate pattern → DECLINE.
    #[test]
    fn declines_when_no_membership_pattern() {
        let pats = tp("?class rdfs:subClassOf foaf:Document . ?doc rdf:type ?class . ?doc dc:title ?title");
        let prepared = crate::exec::prepare_bgp(&g(), &pats).unwrap();
        assert_eq!(detect(&prepared, TEST_THR, |_| false), None);
    }

    // Two unbound-predicate patterns → ambiguous → DECLINE.
    #[test]
    fn declines_when_two_membership_patterns() {
        let pats = tp("?a ?p1 ?b . ?c ?p2 ?d . ?b <http://ex/x> ?c");
        let prepared = crate::exec::prepare_bgp(&g(), &pats).unwrap();
        assert_eq!(detect(&prepared, TEST_THR, |_| false), None);
    }

    // Fewer than three patterns → the cluster would be the whole BGP → DECLINE.
    #[test]
    fn declines_when_bgp_too_small() {
        let pats = tp("?bag ?m ?doc . ?doc2 dcterms:references ?bag");
        let prepared = crate::exec::prepare_bgp(&g(), &pats).unwrap();
        assert_eq!(detect(&prepared, TEST_THR, |_| false), None);
    }

    // A pushed-down scan filter on a cluster pattern → DECLINE (the re-planned
    // sub-BGP must not lose the filter).
    #[test]
    fn declines_when_cluster_pattern_has_filter() {
        let pats = tp(
            "?class rdfs:subClassOf foaf:Document . \
             ?doc rdf:type ?class . \
             ?doc dc:title ?title . \
             ?bag2 ?member2 ?doc . \
             ?doc2 dcterms:references ?bag2",
        );
        let prepared = crate::exec::prepare_bgp(&g(), &pats).unwrap();
        // Pretend the anchor (index 4) carries a pushed filter.
        assert_eq!(detect(&prepared, TEST_THR, |i| i == 4), None);
    }

    // The anchor's non-shared variable leaks into the rest → DECLINE (standalone
    // cluster would drop that join edge). Here `?doc2` is reused by another pattern.
    #[test]
    fn declines_when_anchor_variable_leaks() {
        let pats = tp(
            "?doc rdf:type ?class . \
             ?doc dc:title ?title . \
             ?bag2 ?member2 ?doc . \
             ?doc2 dcterms:references ?bag2 . \
             ?doc2 dc:title ?title2",
        );
        let prepared = crate::exec::prepare_bgp(&g(), &pats).unwrap();
        assert_eq!(detect(&prepared, TEST_THR, |_| false), None);
    }
}
