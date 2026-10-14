//! **Source selection**: for each triple pattern of a BGP, which sources can
//! contribute, and at what estimated cardinality.
//!
//! Two complementary techniques, both reading only the served [`SourceDescriptor`]s:
//!
//! ## HiBISCuS-style authority/prefix pruning
//!
//! A source is *pruned* for a pattern only when its capability set makes a contribution
//! **impossible**. Three prune rules, each firing only on positive evidence:
//!
//! 1. **Bound-predicate prune** — when the pattern's predicate is a bound IRI and the
//!    source's predicate capability set (its `void:propertyPartition`s, which *are* the
//!    complete set of predicates it holds) does not contain it. A source with no triple
//!    using predicate `p` cannot match `?s p ?o` — provably.
//! 2. **Authority prune** (subject / object positions) — when a position is a bound IRI
//!    whose authority is *not* in the source's authority capability set, AND that set is
//!    declared **complete** ([`SourceDescriptor::authorities_complete`]). A
//!    VoID-parsed descriptor has an *incomplete* authority set (it sees only
//!    predicate/class authorities, never instance authorities), so this rule is
//!    **disabled** for it — never pruning on partial knowledge.
//! 3. **Bound-class prune** — when the pattern is `?x rdf:type C` with `C` a bound IRI
//!    and the source declares class partitions but none for `C`. (Only fires when the
//!    source declares *some* class partition — an absent class section means "unknown",
//!    not "no instances".)
//!
//! ### Recall-safety invariant (the load-bearing property)
//!
//! > **If a source `S` could return any binding for a pattern `tp`, [`select_sources`]
//! > retains `S` for `tp`.**
//!
//! Equivalently: a source is dropped for a pattern only when the descriptor *proves* it
//! holds no matching triple. Every prune rule fires only on positive evidence of
//! non-contribution; on any uncertainty (unknown predicate set membership impossible —
//! the predicate set is complete; incomplete authority set; absent class section) the
//! source is **kept**. The cardinality estimate never causes a prune: a source with an
//! estimate of 0 is still retained (estimates are lossy; only capability evidence
//! prunes). This is exactly HiBISCuS's design goal — maximise pruning subject to never
//! losing a result. Proven by the `recall_safe_*` tests below.
//!
//! ## CostFed-style skew-aware cardinality
//!
//! For each retained (pattern, source) pair, the cardinality is estimated from the
//! served per-predicate statistics — `void:triples` and the per-subject average
//! multiplicity — **not** a uniform-distribution guess, so per-predicate skew is
//! preserved. Bound subject/object positions divide the predicate's triple count by its
//! distinct-subject / distinct-object count respectively (CostFed's selectivity buckets).
//!
//! ## Retrieval-capability ordering hint (advisory only)
//!
//! sq-3uijg. A source's declared
//! [`RetrievalCapability`](crate::RetrievalCapability) is consumed here as a
//! **STATIC ordering hint** over each pattern's retained candidates: sources declaring a
//! `sret:cardinalityHint` order first, ascending by hint (the source expecting the
//! smallest per-request result set is contacted first — the same selective-first
//! discipline the join planner applies), sources declaring none keep their relative
//! index order after them, and ascending source index is the deterministic tie-break —
//! so with no hints declared anywhere the ordering is exactly the historical
//! ascending-index order. **Answer-safety invariant (load-bearing):** the hint is read
//! only *after* the prune decision, so a wrong or absent value may reorder the
//! candidates but can NEVER change the retained-source SET or any cardinality estimate
//! (proven by the `retrieval_hint_*` differential tests below). The vector/text
//! endpoint flags are deliberately not consulted — they describe retrieval operators,
//! not BGP answering.
//!
//! sq-a35t.

use crate::descriptor::SourceDescriptor;
use crate::pattern::{Bgp, Term, TriplePattern};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// One source retained for a pattern, with its estimated contributed cardinality.
#[derive(Debug, Clone)]
pub struct SourceCandidate {
    /// Index into the `sources` slice passed to [`select_sources`].
    pub source: usize,
    /// CostFed-style estimated triples this source contributes to the pattern.
    pub estimated_cardinality: f64,
}

/// The sources retained for one triple pattern (by [`select_sources`]).
#[derive(Debug, Clone)]
pub struct PatternSources {
    /// Index of the pattern within the BGP.
    pub pattern: usize,
    /// Retained sources, ordered by the advisory retrieval-capability rank (declared
    /// `sret:cardinalityHint` ascending, undeclared last), tie-broken by ascending
    /// source index — plain ascending index order when no source declares a hint.
    /// The ordering is ADVISORY ONLY: the retained SET never depends on the hint
    /// (see the module docs). sq-3uijg.
    pub candidates: Vec<SourceCandidate>,
}

impl PatternSources {
    /// Total estimated cardinality of the pattern across all retained sources (the
    /// per-pattern input size the join planner starts from — a union over sources).
    pub fn total_cardinality(&self) -> f64 {
        self.candidates
            .iter()
            .map(|c| c.estimated_cardinality)
            .sum()
    }

    /// Whether any source was retained for this pattern.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

/// Source selection for every pattern of `bgp` against `sources`.
///
/// Returns, per pattern, the retained sources and their CostFed cardinality estimates.
/// **Recall-safe** (see the module docs): a source is pruned for a pattern only when
/// the descriptor proves it cannot contribute. Deterministic: candidates are ordered by
/// the advisory retrieval-capability rank (declared cardinality hint ascending,
/// undeclared last), then by source index — which is the plain historical ascending
/// index order whenever no source declares a hint. The rank is ADVISORY ONLY: it is
/// applied strictly after the prune decision, so it can reorder candidates but never
/// change the retained SET (see the module docs). sq-3uijg.
pub fn select_sources(bgp: &Bgp, sources: &[SourceDescriptor]) -> Vec<PatternSources> {
    bgp.patterns
        .iter()
        .enumerate()
        .map(|(pi, tp)| {
            let mut candidates: Vec<SourceCandidate> = Vec::new();
            for (si, src) in sources.iter().enumerate() {
                if can_contribute(tp, src) {
                    candidates.push(SourceCandidate {
                        source: si,
                        estimated_cardinality: estimate_cardinality(tp, src),
                    });
                }
            }
            // Advisory ordering hint ONLY — the membership decision above is final.
            candidates.sort_by_key(|c| (retrieval_rank(&sources[c.source]), c.source));
            PatternSources {
                pattern: pi,
                candidates,
            }
        })
        .collect()
}

/// The STATIC ordering rank of a source's declared retrieval capability: an explicit
/// presence discriminator (`is_none()` — every declared hint, including `u64::MAX`,
/// sorts before every undeclared one) then the `sret:cardinalityHint` ascending, with
/// the historical ascending-index order among unhinted sources preserved via the
/// source-index tie-break. Read only after the prune decision — never affects the
/// retained-source set (the answer-safety invariant). sq-3uijg.
fn retrieval_rank(src: &SourceDescriptor) -> (bool, u64) {
    let hint = src.retrieval().and_then(|r| r.cardinality_hint);
    (hint.is_none(), hint.unwrap_or_default())
}

/// The HiBISCuS prune decision for one (pattern, source): `true` iff the source might
/// contribute. **Recall-safe**: returns `true` on any uncertainty; returns `false` only
/// on positive evidence of non-contribution.
fn can_contribute(tp: &TriplePattern, src: &SourceDescriptor) -> bool {
    // Rule 1 — bound-predicate prune. The predicate partition set is the COMPLETE set
    // of predicates the source holds, so a bound predicate absent from it ⇒ no match.
    if let Some(p) = tp.predicate_iri() {
        if !src.has_predicate(p) {
            return false;
        }
    }

    // Rule 3 — bound-class prune for `?x rdf:type C`. Only when the source declares
    // SOME class partition (otherwise the class section is unknown, not empty).
    if matches!(&tp.predicate, Term::Iri(p) if p == RDF_TYPE) {
        if let Some(c) = tp.object.as_iri() {
            // `src.has_class` membership is meaningful only if the source declares any
            // class at all; a descriptor with no class section can't prune here.
            if src.declares_any_class() && !src.has_class(c) {
                return false;
            }
        }
    }

    // Rule 2 — authority prune (subject/object). Disabled unless the source's authority
    // set is COMPLETE; `may_hold_authority` returns true (keep) when incomplete.
    if let Some(s) = tp.subject.as_iri() {
        if !src.may_hold_authority(s) {
            return false;
        }
    }
    if let Some(o) = tp.object.as_iri() {
        if !src.may_hold_authority(o) {
            return false;
        }
    }

    true
}

/// CostFed-style skew-aware cardinality estimate for a (pattern, source) the prune kept.
///
/// * Unbound predicate (`?s ?p ?o`): the source's total triples — the only sound bound
///   when the predicate is open.
/// * Bound predicate, both subject and object unbound: the predicate's `void:triples`.
/// * Bound predicate + bound subject: `triples / distinctSubjects` (avg multiplicity) —
///   the expected fan-out of one subject.
/// * Bound predicate + bound object: `triples / distinctObjects` when known, else the
///   conservative `triples` (recall-safe over-estimate — never prunes).
/// * Bound predicate + both bound: `triples / (distinctSubjects · distinctObjects)`,
///   floored at 0 (presence is uncertain; the join planner treats it as a probe).
fn estimate_cardinality(tp: &TriplePattern, src: &SourceDescriptor) -> f64 {
    let Some(p) = tp.predicate_iri() else {
        // Open predicate: bound only by the source's total triples.
        return src.total_triples.max(1) as f64;
    };
    let Some(part) = src.predicate(p) else {
        // Prune kept it but predicate absent — only reachable for non-IRI predicate;
        // be conservative.
        return src.total_triples.max(1) as f64;
    };
    let triples = part.triples.max(1) as f64;
    let s_bound = tp.subject.is_bound();
    let o_bound = tp.object.is_bound();
    match (s_bound, o_bound) {
        (false, false) => triples,
        (true, false) => {
            // Expected objects for one subject = avg multiplicity.
            triples / part.distinct_subjects.max(1) as f64
        }
        (false, true) => {
            if part.distinct_objects > 0 {
                triples / part.distinct_objects as f64
            } else {
                triples // unknown distinct-objects ⇒ conservative.
            }
        }
        (true, true) => {
            let denom = part.distinct_subjects.max(1) as f64 * part.distinct_objects.max(1) as f64;
            (triples / denom).max(0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{CharSet, ClassPartition, PredPartition, SourceId};
    use crate::pattern::Var;

    fn iri(s: &str) -> Term {
        Term::Iri(s.to_string())
    }
    fn var(s: &str) -> Term {
        Term::Var(Var::new(s))
    }
    fn pred(p: &str, triples: u64, subjects: u64, objects: u64) -> PredPartition {
        PredPartition {
            predicate: p.into(),
            triples,
            distinct_subjects: subjects,
            distinct_objects: objects,
        }
    }

    // Source A holds foaf:knows (dbpedia authority); Source B holds foaf:name only.
    fn source_a() -> SourceDescriptor {
        SourceDescriptor::builder(SourceId::new("A"))
            .total_triples(1000)
            .predicate(pred("http://xmlns.com/foaf/0.1/knows", 200, 100, 80))
            .build()
    }
    fn source_b() -> SourceDescriptor {
        SourceDescriptor::builder(SourceId::new("B"))
            .total_triples(500)
            .predicate(pred("http://xmlns.com/foaf/0.1/name", 300, 300, 290))
            .build()
    }

    #[test]
    fn bound_predicate_prunes_source_without_it() {
        // ?s foaf:knows ?o — only A holds foaf:knows; B is pruned.
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://xmlns.com/foaf/0.1/knows"),
            var("o"),
        )]);
        let sel = select_sources(&bgp, &[source_a(), source_b()]);
        assert_eq!(sel[0].candidates.len(), 1);
        assert_eq!(sel[0].candidates[0].source, 0); // A
    }

    #[test]
    fn recall_safe_unbound_predicate_keeps_all_sources() {
        // ?s ?p ?o — predicate open: NO source may be pruned (any source could match).
        let bgp = Bgp::new(vec![TriplePattern::new(var("s"), var("p"), var("o"))]);
        let sel = select_sources(&bgp, &[source_a(), source_b()]);
        assert_eq!(
            sel[0].candidates.len(),
            2,
            "open predicate must keep every source"
        );
    }

    #[test]
    fn recall_safe_incomplete_authority_never_prunes_subject_object() {
        // A bound subject IRI on a foreign authority must NOT prune a source whose
        // authority set is incomplete (VoID-parsed) — only the predicate prunes.
        // Source A holds foaf:knows; subject is a wikidata IRI A never declared an
        // authority for. Because A's authority set is incomplete, A is KEPT.
        let bgp = Bgp::new(vec![TriplePattern::new(
            iri("http://www.wikidata.org/entity/Q42"),
            iri("http://xmlns.com/foaf/0.1/knows"),
            var("o"),
        )]);
        let sel = select_sources(&bgp, &[source_a()]);
        assert_eq!(
            sel[0].candidates.len(),
            1,
            "incomplete authority set must not prune"
        );
    }

    #[test]
    fn complete_authority_prunes_foreign_subject_but_keeps_local() {
        // A source with a COMPLETE authority set (HiBISCuS capability) DOES prune a
        // bound subject on a foreign authority — but keeps a local-authority subject.
        let src = SourceDescriptor::builder(SourceId::new("DBp"))
            .predicate(pred("http://xmlns.com/foaf/0.1/knows", 200, 100, 80))
            .authorities_complete() // declares: I only mint foaf authority terms.
            .build();
        let srcs = [src];
        // foreign-authority subject ⇒ pruned.
        let foreign = Bgp::new(vec![TriplePattern::new(
            iri("http://www.wikidata.org/entity/Q42"),
            iri("http://xmlns.com/foaf/0.1/knows"),
            var("o"),
        )]);
        assert!(select_sources(&foreign, &srcs)[0].is_empty());
        // local-authority subject ⇒ kept (foaf authority is in the capability set).
        let local = Bgp::new(vec![TriplePattern::new(
            iri("http://xmlns.com/foaf/0.1/agentX"),
            iri("http://xmlns.com/foaf/0.1/knows"),
            var("o"),
        )]);
        assert!(!select_sources(&local, &srcs)[0].is_empty());
    }

    #[test]
    fn bound_class_prunes_only_when_class_section_declared() {
        // ?x rdf:type :Person — a source that declares classes but NOT :Person prunes;
        // a source with NO class section is KEPT (recall-safe: absent ≠ empty).
        let typed = TriplePattern::new(var("x"), iri(RDF_TYPE), iri("http://ex/Person"));
        let bgp = Bgp::new(vec![typed]);
        // Source with class section but no :Person ⇒ pruned.
        let declares = SourceDescriptor::builder(SourceId::new("C"))
            .predicate(pred(RDF_TYPE, 10, 10, 5))
            .class(ClassPartition {
                class: "http://ex/Company".into(),
                entities: 5,
            })
            .build();
        assert!(select_sources(&bgp, &[declares])[0].is_empty());
        // Source with rdf:type predicate but NO class section ⇒ kept.
        let no_classes = SourceDescriptor::builder(SourceId::new("D"))
            .predicate(pred(RDF_TYPE, 10, 10, 5))
            .build();
        assert!(!select_sources(&bgp, &[no_classes])[0].is_empty());
    }

    #[test]
    fn costfed_cardinality_is_skew_aware() {
        // foaf:knows on A: 200 triples, 100 subjects, 80 objects.
        let a = source_a();
        // ?s knows ?o — full predicate triples.
        let open = TriplePattern::new(var("s"), iri("http://xmlns.com/foaf/0.1/knows"), var("o"));
        assert_eq!(estimate_cardinality(&open, &a), 200.0);
        // bound subject ⇒ avg multiplicity 200/100 = 2.
        let sb = TriplePattern::new(
            iri("http://ex/x"),
            iri("http://xmlns.com/foaf/0.1/knows"),
            var("o"),
        );
        assert_eq!(estimate_cardinality(&sb, &a), 2.0);
        // bound object ⇒ 200/80 = 2.5.
        let ob = TriplePattern::new(
            var("s"),
            iri("http://xmlns.com/foaf/0.1/knows"),
            iri("http://ex/y"),
        );
        assert_eq!(estimate_cardinality(&ob, &a), 2.5);
        // open predicate ⇒ total triples bound.
        let op = TriplePattern::new(var("s"), var("p"), var("o"));
        assert_eq!(estimate_cardinality(&op, &a), 1000.0);
    }

    #[test]
    fn estimate_never_prunes_zero_cardinality_source() {
        // A source whose CS-derived estimate would be tiny must still be RETAINED:
        // estimates are lossy, only capability evidence prunes (recall-safety).
        let a = SourceDescriptor::builder(SourceId::new("A"))
            .total_triples(1)
            .predicate(pred("http://ex/p", 1, 1, 1))
            .char_set(CharSet {
                predicates: vec!["http://ex/p".into()],
                subjects: 1,
                avg_multiplicity: vec![0.0], // pathological zero-multiplicity.
            })
            .build();
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        let sel = select_sources(&bgp, &[a]);
        assert_eq!(
            sel[0].candidates.len(),
            1,
            "low/zero estimate must not prune the source"
        );
    }

    // ============================================================================
    // sq-bif.3 — correctness suite: previously-uncovered selection branches
    // (object-authority prune, the full CostFed cardinality matrix incl. both-bound and
    // the unknown-distinct-objects fallback, the literal/non-IRI prune paths, multi-source
    // union + ordering, and the PatternSources accessors). Drives the REAL `select_sources`
    // / `estimate_cardinality` / `can_contribute` code, not a re-implementation.
    // ============================================================================

    fn foaf(local: &str) -> String {
        format!("http://xmlns.com/foaf/0.1/{}", local)
    }
    fn lit(s: &str) -> Term {
        Term::Literal(s.to_string())
    }

    // ---- Rule 2 (authority prune) also fires on the OBJECT position, symmetrically with
    //      the subject path the existing tests cover. A COMPLETE-authority source with a
    //      foreign-authority bound OBJECT is pruned; a local-authority object is kept.
    #[test]
    fn complete_authority_prunes_foreign_object_but_keeps_local() {
        let src = SourceDescriptor::builder(SourceId::new("DBp"))
            .predicate(pred(&foaf("knows"), 200, 100, 80))
            .authorities_complete() // I only mint foaf-authority terms.
            .build();
        let srcs = [src];
        // foreign-authority OBJECT ⇒ pruned (the object branch of Rule 2).
        let foreign = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri(&foaf("knows")),
            iri("http://www.wikidata.org/entity/Q42"),
        )]);
        assert!(
            select_sources(&foreign, &srcs)[0].is_empty(),
            "foreign-authority bound object must prune a complete-authority source"
        );
        // local-authority OBJECT ⇒ kept.
        let local = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri(&foaf("knows")),
            iri(&foaf("agentY")),
        )]);
        assert!(
            !select_sources(&local, &srcs)[0].is_empty(),
            "local-authority bound object stays in the capability set"
        );
    }

    // ---- A bound LITERAL object never triggers an authority prune (a literal has no
    //      authority), even on a complete-authority source — recall-safe.
    #[test]
    fn literal_object_never_authority_prunes() {
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .predicate(pred(&foaf("name"), 300, 300, 290))
            .authorities_complete()
            .build();
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri(&foaf("name")),
            lit("\"Alice\""),
        )]);
        assert_eq!(
            select_sources(&bgp, &[src])[0].candidates.len(),
            1,
            "a bound literal object has no authority ⇒ no authority prune"
        );
    }

    // ---- CostFed cardinality: the both-bound `(true, true)` branch =
    //      triples / (distinctSubjects · distinctObjects), and the bound-object
    //      `distinct_objects == 0` unknown-fallback branch (⇒ conservative `triples`).
    #[test]
    fn costfed_cardinality_both_bound_and_unknown_objects() {
        // knows: 200 triples, 100 subjects, 80 objects.
        let a = source_a();
        // both bound ⇒ 200 / (100 * 80) = 0.025.
        let both = TriplePattern::new(iri("http://ex/x"), iri(&foaf("knows")), iri("http://ex/y"));
        assert_eq!(estimate_cardinality(&both, &a), 200.0 / (100.0 * 80.0));

        // A predicate whose distinct_objects is UNKNOWN (0): bound-object falls back to the
        // conservative full triple count (recall-safe over-estimate, never prunes).
        let unknown_obj = SourceDescriptor::builder(SourceId::new("U"))
            .predicate(pred("http://ex/p", 500, 250, 0)) // objects unknown.
            .build();
        let ob = TriplePattern::new(var("s"), iri("http://ex/p"), iri("http://ex/y"));
        assert_eq!(
            estimate_cardinality(&ob, &unknown_obj),
            500.0,
            "unknown distinct-objects ⇒ conservative full-triples estimate"
        );
        // …and with the SAME predicate, a bound SUBJECT still divides by distinct_subjects
        // (250) even though objects are unknown: 500 / 250 = 2.
        let sb = TriplePattern::new(iri("http://ex/x"), iri("http://ex/p"), var("o"));
        assert_eq!(estimate_cardinality(&sb, &unknown_obj), 2.0);
    }

    // ---- The both-bound estimate is floored at 0 and the `triples`/`distinct*` are floored
    //      at 1 so a tiny/degenerate partition never divides by zero (it stays a positive
    //      probe-sized estimate, which — being just an estimate — never prunes).
    #[test]
    fn costfed_cardinality_degenerate_counts_do_not_divide_by_zero() {
        let degenerate = SourceDescriptor::builder(SourceId::new("D"))
            .predicate(pred("http://ex/p", 0, 0, 0)) // everything unknown / zero.
            .build();
        let both = TriplePattern::new(iri("http://ex/x"), iri("http://ex/p"), iri("http://ex/y"));
        let est = estimate_cardinality(&both, &degenerate);
        assert!(est.is_finite(), "no division by zero on a degenerate partition");
        assert!(est >= 0.0, "estimate is floored at 0");
        assert_eq!(est, 1.0, "triples.max(1)/(1*1) = 1 for an all-zero partition");
    }

    // sq-2b7h7: sweep the estimator's finite, non-negative postcondition
    // across every binding branch and both early returns. The zero-count probe makes
    // the denominator guards observable; the high-distinct-count source exercises a
    // both-bound estimate far below one without allowing it to become negative.
    #[test]
    fn estimate_cardinality_is_finite_and_non_negative_across_branches() {
        let high_distinct = SourceDescriptor::builder(SourceId::new("high-distinct"))
            .total_triples(2)
            .predicate(pred("http://ex/p", 1, 1_000_000, 2_000_000))
            .build();
        let zero_distinct = SourceDescriptor::builder(SourceId::new("zero-distinct"))
            .predicate(pred("http://ex/p", 1, 0, 0))
            .build();

        let cases = [
            (
                "neither term bound",
                TriplePattern::new(var("s"), iri("http://ex/p"), var("o")),
                &high_distinct,
            ),
            (
                "subject bound",
                TriplePattern::new(iri("http://ex/s"), iri("http://ex/p"), var("o")),
                &high_distinct,
            ),
            (
                "object bound",
                TriplePattern::new(var("s"), iri("http://ex/p"), iri("http://ex/o")),
                &high_distinct,
            ),
            (
                "both terms bound",
                TriplePattern::new(iri("http://ex/s"), iri("http://ex/p"), iri("http://ex/o")),
                &high_distinct,
            ),
            (
                "open predicate",
                TriplePattern::new(var("s"), var("p"), var("o")),
                &high_distinct,
            ),
            (
                "predicate absent",
                TriplePattern::new(var("s"), iri("http://ex/absent"), var("o")),
                &high_distinct,
            ),
            (
                "zero-count denominator guard",
                TriplePattern::new(iri("http://ex/s"), iri("http://ex/p"), iri("http://ex/o")),
                &zero_distinct,
            ),
        ];

        for (branch, pattern, source) in cases {
            let estimate = estimate_cardinality(&pattern, source);
            assert!(
                estimate.is_finite() && estimate >= 0.0,
                "{branch} returned an invalid estimate: {estimate}"
            );
        }
    }

    // ---- The open-predicate estimate is the source's total triples, floored at 1 even for
    //      an empty source (this is also the `estimate_cardinality` fallback for a non-IRI
    //      predicate position).
    #[test]
    fn estimate_open_predicate_is_total_triples_bounded() {
        let a = source_a(); // total_triples 1000.
        let open = TriplePattern::new(var("s"), var("p"), var("o"));
        assert_eq!(estimate_cardinality(&open, &a), 1000.0);
        // A source with total_triples 0 still yields a positive (floored-at-1) estimate.
        let empty = SourceDescriptor::builder(SourceId::new("E")).build();
        assert_eq!(
            estimate_cardinality(&open, &empty),
            1.0,
            "open-predicate estimate is total_triples.max(1)"
        );
    }

    // ---- Multi-source union: a pattern matched by SEVERAL sources retains them all, in
    //      ascending source-index order, and `total_cardinality` sums the per-source
    //      estimates (the per-pattern input size the join planner starts from).
    #[test]
    fn multi_source_union_orders_and_sums() {
        // Three sources, all holding foaf:knows with different triple counts.
        let s0 = SourceDescriptor::builder(SourceId::new("s0"))
            .predicate(pred(&foaf("knows"), 100, 100, 100))
            .build();
        let s1 = SourceDescriptor::builder(SourceId::new("s1"))
            .predicate(pred(&foaf("knows"), 50, 50, 50))
            .build();
        let s2 = SourceDescriptor::builder(SourceId::new("s2"))
            .predicate(pred(&foaf("name"), 999, 999, 999)) // no knows ⇒ pruned for this pattern.
            .build();
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri(&foaf("knows")),
            var("o"),
        )]);
        let sel = select_sources(&bgp, &[s0, s1, s2]);
        let cands = &sel[0].candidates;
        assert_eq!(cands.len(), 2, "only the two knows-holders are retained");
        // Ascending source index (s0 then s1); s2 pruned.
        assert_eq!(cands[0].source, 0);
        assert_eq!(cands[1].source, 1);
        // Open ?s knows ?o ⇒ each source contributes its full predicate triple count.
        assert_eq!(cands[0].estimated_cardinality, 100.0);
        assert_eq!(cands[1].estimated_cardinality, 50.0);
        assert_eq!(
            sel[0].total_cardinality(),
            150.0,
            "union cardinality is the sum over retained sources"
        );
        assert!(!sel[0].is_empty());
        assert_eq!(sel[0].pattern, 0);
    }

    // ---- A pattern NO source can answer yields an empty `PatternSources`
    //      (`is_empty` true, `total_cardinality` 0). The join planner reads this as a
    //      zero-input leaf, not as a reason to drop the pattern (recall is the selector's
    //      job, not the planner's).
    #[test]
    fn pattern_with_no_matching_source_is_empty() {
        let only_name = SourceDescriptor::builder(SourceId::new("N"))
            .predicate(pred(&foaf("name"), 10, 10, 10))
            .build();
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri(&foaf("knows")), // no source holds knows.
            var("o"),
        )]);
        let sel = select_sources(&bgp, &[only_name]);
        assert!(sel[0].is_empty());
        assert_eq!(sel[0].total_cardinality(), 0.0);
    }

    // ---- An rdf:type pattern with a VARIABLE class object is never class-pruned (no bound
    //      class to test), even on a source that declares a class section — recall-safe.
    #[test]
    fn variable_class_object_is_never_class_pruned() {
        let declares = SourceDescriptor::builder(SourceId::new("C"))
            .predicate(pred(RDF_TYPE, 10, 10, 5))
            .class(ClassPartition {
                class: "http://ex/Company".into(),
                entities: 5,
            })
            .build();
        // ?x rdf:type ?c — the class is a variable ⇒ the bound-class prune cannot fire.
        let bgp = Bgp::new(vec![TriplePattern::new(var("x"), iri(RDF_TYPE), var("c"))]);
        assert_eq!(
            select_sources(&bgp, &[declares])[0].candidates.len(),
            1,
            "a variable class object must keep the source (no bound class to prove absent)"
        );
    }

    // ---- Per-pattern independence: in a multi-pattern BGP the selection is computed
    //      independently per pattern, and the returned `PatternSources.pattern` indices line
    //      up with the BGP positions (the planner relies on this alignment).
    #[test]
    fn selection_is_per_pattern_and_index_aligned() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("s"), iri(&foaf("knows")), var("o")),
            TriplePattern::new(var("o"), iri(&foaf("name")), var("n")),
        ]);
        let sel = select_sources(&bgp, &[source_a(), source_b()]);
        assert_eq!(sel.len(), 2);
        assert_eq!(sel[0].pattern, 0);
        assert_eq!(sel[1].pattern, 1);
        // Pattern 0 (knows) ⇒ only A; pattern 1 (name) ⇒ only B.
        assert_eq!(sel[0].candidates.len(), 1);
        assert_eq!(sel[0].candidates[0].source, 0);
        assert_eq!(sel[1].candidates.len(), 1);
        assert_eq!(sel[1].candidates[0].source, 1);
    }

    // ============================================================================
    // sq-3uijg — the RetrievalCapability STATIC ordering hint. Advisory
    // ONLY: it may reorder each pattern's candidates but can NEVER change the
    // retained-source SET or any cardinality estimate (the inherited answer-safety
    // invariant from sq-222my, proven by the differential test below).
    // ============================================================================

    use crate::descriptor::RetrievalCapability;

    /// A source holding `foaf:knows`, optionally declaring a retrieval capability
    /// with the given cardinality hint.
    fn knows_source(id: &str, triples: u64, hint: Option<Option<u64>>) -> SourceDescriptor {
        let mut b = SourceDescriptor::builder(SourceId::new(id))
            .total_triples(triples)
            .predicate(pred(&foaf("knows"), triples, triples.max(1), triples.max(1)));
        if let Some(cardinality_hint) = hint {
            b = b.retrieval(RetrievalCapability {
                vector: true,
                text: false,
                cardinality_hint,
            });
        }
        b.build()
    }

    // ---- Declared hints order candidates ascending-by-hint FIRST; sources without a
    //      declared hint (no capability at all, or a capability without the hint) keep
    //      their relative index order AFTER every hinted source.
    #[test]
    fn retrieval_hint_orders_declared_hints_first_ascending() {
        let srcs = [
            knows_source("s0", 100, Some(Some(500))), // hint 500 ⇒ second.
            knows_source("s1", 100, None),            // no capability ⇒ last group.
            knows_source("s2", 100, Some(Some(5))),   // hint 5 ⇒ first.
            knows_source("s3", 100, Some(None)),      // capability, hint undeclared ⇒ last group.
        ];
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri(&foaf("knows")),
            var("o"),
        )]);
        let order: Vec<usize> = select_sources(&bgp, &srcs)[0]
            .candidates
            .iter()
            .map(|c| c.source)
            .collect();
        assert_eq!(
            order,
            vec![2, 0, 1, 3],
            "hinted ascending (5, 500) first, then unhinted in index order"
        );
    }

    // ---- A declared `u64::MAX` hint is NOT the undeclared sentinel: the presence
    //      discriminator (not the hint value) separates the groups, so a later-index
    //      source declaring u64::MAX still precedes an earlier-index unhinted source.
    #[test]
    fn retrieval_hint_u64_max_precedes_unhinted() {
        let srcs = [
            knows_source("m0", 100, None),                 // unhinted, earlier index.
            knows_source("m1", 100, Some(Some(u64::MAX))), // declared max hint.
        ];
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri(&foaf("knows")),
            var("o"),
        )]);
        let order: Vec<usize> = select_sources(&bgp, &srcs)[0]
            .candidates
            .iter()
            .map(|c| c.source)
            .collect();
        assert_eq!(
            order,
            vec![1, 0],
            "a declared u64::MAX hint sorts before every unhinted source"
        );
    }

    // ---- Equal hints (and the all-unhinted case) fall back to the deterministic
    //      ascending source-index tie-break — with no hints anywhere the ordering is
    //      exactly the historical ascending-index order.
    #[test]
    fn retrieval_hint_ties_and_absence_keep_index_order() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri(&foaf("knows")),
            var("o"),
        )]);
        // Equal hints ⇒ index order.
        let tied = [
            knows_source("t0", 100, Some(Some(7))),
            knows_source("t1", 100, Some(Some(7))),
        ];
        let order: Vec<usize> = select_sources(&bgp, &tied)[0]
            .candidates
            .iter()
            .map(|c| c.source)
            .collect();
        assert_eq!(order, vec![0, 1], "equal hints tie-break on source index");
        // No hints at all ⇒ the historical ascending-index order.
        let unhinted = [knows_source("u0", 100, None), knows_source("u1", 100, None)];
        let order: Vec<usize> = select_sources(&bgp, &unhinted)[0]
            .candidates
            .iter()
            .map(|c| c.source)
            .collect();
        assert_eq!(order, vec![0, 1], "no hints ⇒ ascending index order");
    }

    // ---- THE inherited invariant (differential acceptance test): selection WITH hints
    //      vs WITHOUT hints yields, per pattern, the IDENTICAL selected-source SET and
    //      identical per-source cardinality estimates — only the ordering may differ.
    //      The hints are chosen adversarially WRONG (a zero hint on a huge source, a
    //      huge hint on a tiny one) to show a bad hint still cannot change the set.
    #[test]
    fn retrieval_hint_never_changes_selected_source_sets() {
        use std::collections::BTreeMap;
        // Two-pattern BGP: knows (held by 3 of 4 sources) and name (held by 1).
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("s"), iri(&foaf("knows")), var("o")),
            TriplePattern::new(var("o"), iri(&foaf("name")), var("n")),
        ]);
        let with_hints = [
            knows_source("w0", 10_000, Some(Some(0))), // adversarial: zero hint, huge source.
            knows_source("w1", 3, Some(Some(u64::MAX))), // adversarial: max hint, tiny source.
            source_b(), // name-only ⇒ pruned for knows regardless of any hint.
            knows_source("w3", 100, Some(Some(50))),
        ];
        let without_hints = [
            knows_source("w0", 10_000, None),
            knows_source("w1", 3, None),
            source_b(),
            knows_source("w3", 100, None),
        ];
        // Per pattern: the (source ⇒ estimate) map, i.e. the selected SET + estimates
        // with the advisory ordering erased.
        let sets = |srcs: &[SourceDescriptor]| -> Vec<BTreeMap<usize, u64>> {
            select_sources(&bgp, srcs)
                .iter()
                .map(|ps| {
                    ps.candidates
                        .iter()
                        .map(|c| (c.source, c.estimated_cardinality.to_bits()))
                        .collect()
                })
                .collect()
        };
        assert_eq!(
            sets(&with_hints),
            sets(&without_hints),
            "hints must not change the selected-source sets or the estimates"
        );
        // …while the ordering DID change for the knows pattern (the hint is consumed):
        // zero-hint w0 first, then w3 (50), then max-hint w1.
        let order: Vec<usize> = select_sources(&bgp, &with_hints)[0]
            .candidates
            .iter()
            .map(|c| c.source)
            .collect();
        assert_eq!(order, vec![0, 3, 1], "the hint reorders (and only reorders)");
    }
}
