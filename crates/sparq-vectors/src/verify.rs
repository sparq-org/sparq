//! Neuro-symbolic **propose(neural)-then-verify(deductive)** grounding pipeline
//! (structure-aware vectorisation **P5**).
//!
//! sq-0wo9e.6 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §5 (B) "Neuro-symbolic propose-then-verify" + §6.A "Verify-soundness / answer-safety").
//! This module is the **fifth** additive slice of the structure-aware-vectorisation epic, on top
//! of P0 ([`crate::structure`]: closure-before-vectorise + type-constrained negatives) and P4
//! ([`crate::grounding`]: the flexible grounding selector). It is **opt-in** (the
//! `neuro-symbolic` cargo feature, off by default) and changes **nothing** in the default
//! `sparq-vectors` build or the core engine.
//!
//! # What this is
//!
//! Grounding a tool slot `(subject, predicate, ?)` against a graph has two halves with
//! **deliberately asymmetric guarantees**, and this module keeps them honestly separate:
//!
//! 1. **PROPOSE (neural / vector — a candidate GENERATOR, recall only).** The structure-aware
//!    entity vectors propose high-recall candidate objects for the slot by vector similarity
//!    ([`propose_by_vector`], an exact-ANN scan of [`crate::ann::nearest_exact`] over the
//!    `.spqv` store). **This step has NO soundness or completeness guarantee** — it is a
//!    generator that may surface a candidate that is *type-inconsistent*, violates a *shape
//!    constraint*, or uses a *predicate invalid for the subject's type*. It is framed that way
//!    on purpose: a vector neighbour is a *hypothesis*, never a verified fact.
//! 2. **VERIFY (deductive / constraint — the soundness GATE).** Each proposed candidate is
//!    checked deductively ([`verify_candidate`]): the candidate binding `(subject, predicate,
//!    object)` is added to the graph and the result must (a) introduce **no new SHACL violation**
//!    on the affected focus nodes (`sparq-shacl`; datatype / cardinality / enum / `sh:class` /
//!    node-kind conformance) **and** (b) introduce **no new OWL inconsistency** — a disjoint /
//!    complement class clash, a `differentFrom`/`sameAs` clash, a max-cardinality-0 violation
//!    (`sparq-reason::inconsistencies`, the `cax-dw` / `cax-adc` / `prp-*` clash rules over the
//!    materialised closure). **A candidate that fails EITHER check is REJECTED — not merely
//!    down-ranked.**
//!
//! The returned set is therefore the **deductively-verified subset** of what the neural side
//! proposed: the pipeline can only ever *shrink* the candidate set toward a sound subset
//! (design §5: "the candidate set can only SHRINK to a sound subset"), generalising the
//! existing "ANN proposes, exact engine re-validates" invariant to SHACL + closure verification.
//!
//! # Soundness contract (load-bearing — design §6.A "Verify-soundness / answer-safety")
//!
//! - **The verify is a real DEDUCTIVE GATE, not a re-ranker.** A vector-proposed candidate that
//!   fails the shape/type/disjointness check is dropped from the output. [`verify_candidate`]
//!   returns a [`Verdict`] whose [`Verdict::is_verified`] is the **only** thing
//!   [`propose_then_verify`] surfaces as a grounding; a rejected candidate never appears.
//! - **FAIL-CLOSED.** When the deductive check *cannot be run to a clean conclusion* — e.g. the
//!   candidate id is not a real dictionary term, or building the augmented graph fails — the
//!   candidate is **rejected**, never surfaced as verified. The honest default is "unproven ⇒
//!   not surfaced", never "unproven ⇒ assume valid".
//! - **The neural step is NOT claimed sound.** [`propose_by_vector`] is documented and typed as a
//!   recall-oriented generator. No part of this module asserts the vector similarity is correct;
//!   correctness comes *only* from the deductive gate.
//!
//! # What is provable vs benchmarked (stated up front, honestly)
//!
//! - **Provable (and tested):** *verify-shrinks-to-sound* — every candidate in
//!   [`VerifyReport::verified`] is a subset of the proposed set **and** introduces no new SHACL
//!   violation / OWL inconsistency relative to the base graph. The load-bearing test asserts a
//!   vector-proposed-but-shape-invalid candidate is **rejected**.
//! - **Benchmarked-only (NOT claimed here):** whether the neural propose step actually raises
//!   end-task *recall* over a non-vector generator is **empirical and dataset-dependent**
//!   (GraphRAG/KG-RAG does not uniformly beat vector RAG — design §5 honesty line). The
//!   on/off-ablation harness ([`crate::eval`], sq-0wo9e.7) is where that is measured; **no
//!   accuracy number is asserted in this module or its docs.**

use crate::ann::nearest_exact;
use crate::store::VectorStore;
use rustc_hash::FxHashSet;
use sparq_core::dict::{self, Dict, Id};
use sparq_core::Graph;
use sparq_reason::Profile;
use sparq_shacl::ShapesModel;

/// How a proposed candidate was generated — recorded on every [`Verdict`] so a caller can audit
/// the neural periphery without conflating it with the deductive gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Generated by the **vector** (neural) similarity step ([`propose_by_vector`]). Recall only;
    /// **no** soundness guarantee.
    Vector,
    /// Supplied directly by the caller (e.g. an LLM, or a different generator) and fed straight
    /// into the deductive gate. Also recall only.
    Caller,
}

/// Why a candidate was rejected by the deductive gate. A rejected candidate is **never** surfaced
/// as a verified grounding; the reason exists so a caller can log *why* the neural proposal was
/// unsound (the value of the pipeline is the audit trail, not just the filter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// Adding the binding introduced a **new SHACL violation** on an affected focus node
    /// (datatype / cardinality / enum / `sh:class` / node-kind ...). Carries the source
    /// constraint-component IRIs that newly failed.
    ShaclViolation(Vec<String>),
    /// Adding the binding introduced a **new OWL inconsistency** (disjoint/complement class clash,
    /// `differentFrom`/`sameAs` clash, max-cardinality-0 violation ...). Carries the clash
    /// messages `sparq-reason::inconsistencies` reported that were not present in the base graph.
    OwlInconsistency(Vec<String>),
    /// The candidate could not be checked to a clean conclusion (e.g. not a real dictionary term).
    /// **Fail-closed:** an uncheckable candidate is rejected, never surfaced as verified.
    Uncheckable(String),
}

/// The deductive verdict on one proposed candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The candidate object id (a dictionary term id of the base graph).
    pub object: Id,
    /// How it was proposed (audit only — never affects the gate).
    pub source: Source,
    /// `None` => the candidate passed the deductive gate (verified). `Some(reason)` => it was
    /// **rejected** (and is excluded from [`VerifyReport::verified`]).
    pub rejected: Option<Rejection>,
}

impl Verdict {
    /// `true` iff the candidate passed the deductive gate.
    pub fn is_verified(&self) -> bool {
        self.rejected.is_none()
    }
}

/// The outcome of verifying a batch of proposed candidates for one `(subject, predicate)` slot.
/// Keeps the verified subset and the full audit trail (every verdict, incl. rejections).
#[derive(Clone, Debug, Default)]
pub struct VerifyReport {
    /// Every candidate's verdict, in the order they were proposed (verified **and** rejected).
    pub verdicts: Vec<Verdict>,
}

impl VerifyReport {
    /// The **deductively-verified** candidate object ids — the only sound output. A candidate the
    /// gate rejected is *not* here. This is the set the design's "candidate set can only SHRINK to
    /// a sound subset" invariant guarantees is a subset of what was proposed.
    pub fn verified(&self) -> Vec<Id> {
        self.verdicts
            .iter()
            .filter(|v| v.is_verified())
            .map(|v| v.object)
            .collect()
    }

    /// The rejected verdicts (audit trail of why each unsound proposal was dropped).
    pub fn rejected(&self) -> impl Iterator<Item = &Verdict> {
        self.verdicts.iter().filter(|v| v.rejected.is_some())
    }

    /// How many candidates were proposed in total (verified + rejected).
    pub fn proposed_count(&self) -> usize {
        self.verdicts.len()
    }
}

/// Configuration for the deductive gate.
#[derive(Clone, Debug)]
pub struct VerifyConfig {
    /// The OWL/RDFS entailment profile to materialise before the disjointness / inconsistency
    /// check, so an *entailed* type clash (e.g. the candidate is `subClassOf` a class disjoint
    /// from the subject's declared type) is caught — not just an asserted one. Default
    /// [`Profile::OwlRl`] (disjointness lives in OWL, so RDFS alone would miss `cax-dw`).
    pub profile: Profile,
    /// Run the SHACL conformance half of the gate. When `false`, only the OWL-inconsistency half
    /// runs (useful when no shapes are available). Default `true`.
    pub check_shacl: bool,
    /// Run the OWL-inconsistency half of the gate (`sparq-reason::inconsistencies` over the
    /// closure). Default `true`.
    pub check_owl: bool,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        VerifyConfig {
            profile: Profile::OwlRl,
            check_shacl: true,
            check_owl: true,
        }
    }
}

/// **PROPOSE (neural — recall only, NOT sound).** Propose up to `k` candidate object ids for the
/// slot by **vector similarity**: the query vector is the stored vector of `seed` (typically the
/// subject, or an exemplar object), and the candidates are its `k` nearest neighbours in `store`
/// by cosine ([`nearest_exact`]). `seed` itself is excluded.
///
/// **This is a candidate GENERATOR, not a soundness oracle.** A returned id may be
/// type-inconsistent, shape-invalid, or otherwise wrong for the slot — that is exactly what the
/// deductive [`verify_candidate`] gate exists to catch. Feed the output straight into
/// [`verify_candidates`] / [`propose_then_verify`]; never treat it as a verified grounding.
///
/// Returns the candidate ids (without scores — the gate is boolean, not a re-rank). Empty when
/// `seed` has no stored vector.
pub fn propose_by_vector(store: &VectorStore, seed: Id, k: usize) -> Vec<Id> {
    let Some(query) = store.get(seed) else {
        return Vec::new();
    };
    // Over-fetch by one so excluding the seed still yields up to k candidates.
    nearest_exact(store, query, k + 1)
        .into_iter()
        .map(|(id, _score)| id)
        .filter(|&id| id != seed)
        .take(k)
        .collect()
}

/// **VERIFY (deductive — the soundness gate).** Decide whether the single proposed binding
/// `(subject, predicate, object)` is *deductively admissible* against `graph` under `shapes`
/// (optional) and `cfg`.
///
/// The gate is **differential**: it adds the candidate triple to the graph and rejects the
/// candidate only when doing so introduces a **NEW** SHACL violation or OWL inconsistency that the
/// base graph did not already have — so a candidate is never blamed for a pre-existing, unrelated
/// defect elsewhere in the data. A candidate that introduces no new defect is **verified**.
///
/// **FAIL-CLOSED:** if `object` (or `subject`/`predicate`) is not a real term of `graph`, the
/// candidate cannot be checked and is rejected ([`Rejection::Uncheckable`]) — never surfaced as
/// verified.
pub fn verify_candidate(
    graph: &Graph,
    subject: Id,
    predicate: Id,
    object: Id,
    shapes: Option<&ShapesModel>,
    cfg: &VerifyConfig,
) -> Verdict {
    verify_candidate_with_source(
        graph,
        subject,
        predicate,
        object,
        shapes,
        cfg,
        Source::Caller,
    )
}

/// [`verify_candidate`] tagging the verdict with an explicit [`Source`] (used internally so a
/// vector-proposed candidate's verdict records [`Source::Vector`]).
fn verify_candidate_with_source(
    graph: &Graph,
    subject: Id,
    predicate: Id,
    object: Id,
    shapes: Option<&ShapesModel>,
    cfg: &VerifyConfig,
    source: Source,
) -> Verdict {
    let reject = |r: Rejection| Verdict {
        object,
        source,
        rejected: Some(r),
    };

    // Fail-closed: subject/predicate must be real, non-inline terms of the graph so the augmented
    // graph is well-formed and the deductive checks see the candidate as a fact.
    if dict::is_inline(subject) || dict::is_inline(predicate) {
        return reject(Rejection::Uncheckable(
            "subject/predicate is an inline literal id".to_string(),
        ));
    }
    if !graph_has_id(graph, subject) || !graph_has_id(graph, predicate) {
        return reject(Rejection::Uncheckable(
            "subject/predicate not present in the graph".to_string(),
        ));
    }
    // The object may be an inline literal id (a typed-value candidate) OR a real term id.
    if !dict::is_inline(object) && !graph_has_id(graph, object) {
        return reject(Rejection::Uncheckable(
            "candidate object not present in the graph dictionary".to_string(),
        ));
    }
    if graph_contains(graph, subject, predicate, object) {
        // The binding is already an asserted fact — trivially admissible (it is, definitionally,
        // consistent with a graph that already contains it). Verified.
        return Verdict {
            object,
            source,
            rejected: None,
        };
    }

    // --- OWL-inconsistency half (over the materialised closure) -------------------------------
    if cfg.check_owl {
        match new_owl_inconsistencies(graph, subject, predicate, object, cfg.profile) {
            Ok(new_clashes) if !new_clashes.is_empty() => {
                return reject(Rejection::OwlInconsistency(new_clashes));
            }
            Ok(_) => {}
            Err(e) => return reject(Rejection::Uncheckable(e)),
        }
    }

    // --- SHACL-conformance half ---------------------------------------------------------------
    if cfg.check_shacl {
        if let Some(model) = shapes {
            match new_shacl_violations(graph, subject, predicate, object, model) {
                Ok(new_comps) if !new_comps.is_empty() => {
                    return reject(Rejection::ShaclViolation(new_comps));
                }
                Ok(_) => {}
                Err(e) => return reject(Rejection::Uncheckable(e)),
            }
        }
    }

    Verdict {
        object,
        source,
        rejected: None,
    }
}

/// **VERIFY a batch.** Run [`verify_candidate`] over every proposed `candidate` for the
/// `(subject, predicate)` slot, returning the full [`VerifyReport`] (verified subset + audit
/// trail). Candidates keep their proposed order; `source` tags every verdict.
pub fn verify_candidates(
    graph: &Graph,
    subject: Id,
    predicate: Id,
    candidates: &[Id],
    source: Source,
    shapes: Option<&ShapesModel>,
    cfg: &VerifyConfig,
) -> VerifyReport {
    let verdicts = candidates
        .iter()
        .map(|&object| {
            verify_candidate_with_source(graph, subject, predicate, object, shapes, cfg, source)
        })
        .collect();
    VerifyReport { verdicts }
}

/// **The full neuro-symbolic pipeline.** PROPOSE candidate objects for the `(subject, predicate)`
/// slot by vector similarity (the `seed` vector's `k` nearest neighbours — recall only), then
/// VERIFY each deductively, returning the [`VerifyReport`] whose [`VerifyReport::verified`] is the
/// sound subset.
///
/// `seed` is the term whose stored vector drives the neural proposal (typically `subject`). The
/// returned report's verified set is **guaranteed a subset of the proposed candidates** (the gate
/// only ever drops) and contains only bindings that introduce no new SHACL violation / OWL
/// inconsistency — the design's verify-shrinks-to-sound property.
#[allow(clippy::too_many_arguments)]
pub fn propose_then_verify(
    graph: &Graph,
    store: &VectorStore,
    seed: Id,
    subject: Id,
    predicate: Id,
    k: usize,
    shapes: Option<&ShapesModel>,
    cfg: &VerifyConfig,
) -> VerifyReport {
    let candidates = propose_by_vector(store, seed, k);
    verify_candidates(
        graph,
        subject,
        predicate,
        &candidates,
        Source::Vector,
        shapes,
        cfg,
    )
}

// ---- deductive primitives ---------------------------------------------------------------------

/// The SHACL constraint-components that NEWLY fail when `(subject, predicate, object)` is added to
/// `graph` — i.e. those present in the with-candidate report but not the base report (keyed by
/// `(focus, component)` so an unrelated pre-existing violation is not blamed on the candidate).
fn new_shacl_violations(
    graph: &Graph,
    subject: Id,
    predicate: Id,
    object: Id,
    model: &ShapesModel,
) -> Result<Vec<String>, String> {
    let base_keys = shacl_violation_keys(graph, model);
    let augmented = augment(graph, subject, predicate, object)?;
    let aug_keys = shacl_violation_keys(&augmented, model);
    let mut newly: Vec<String> = aug_keys
        .difference(&base_keys)
        .map(|(_focus, comp)| comp.clone())
        .collect();
    newly.sort();
    newly.dedup();
    Ok(newly)
}

/// The set of `(focus-node, source-component-IRI)` keys of every violation the SHACL validator
/// reports for `data` under `model`. Keyed so a candidate is only blamed for a violation the base
/// graph did not already have on the same focus node.
fn shacl_violation_keys(data: &Graph, model: &ShapesModel) -> FxHashSet<(String, String)> {
    let report = sparq_shacl::validate_with_model(data, model);
    report
        .results
        .iter()
        .map(|r| (r.focus_node.to_string(), r.source_component.clone()))
        .collect()
}

/// The OWL inconsistency messages that NEWLY appear when `(subject, predicate, object)` is added
/// and the closure is materialised — differential against the base graph's own closure
/// inconsistencies (so a pre-existing clash is not attributed to the candidate).
fn new_owl_inconsistencies(
    graph: &Graph,
    subject: Id,
    predicate: Id,
    object: Id,
    profile: Profile,
) -> Result<Vec<String>, String> {
    let base = closure_inconsistencies(graph, profile);
    let augmented = augment(graph, subject, predicate, object)?;
    let aug = closure_inconsistencies(&augmented, profile);
    let base_set: FxHashSet<&String> = base.iter().collect();
    let mut newly: Vec<String> = aug
        .iter()
        .filter(|m| !base_set.contains(m))
        .cloned()
        .collect();
    newly.sort();
    newly.dedup();
    Ok(newly)
}

/// Materialise the `profile` closure over a clone of `data`'s triples and run
/// `sparq-reason::inconsistencies` over the result, returning the clash messages. The disjointness
/// / `differentFrom` / cardinality-0 clash detection lives in OWL, so the closure surfaces an
/// *entailed* clash (e.g. via `subClassOf` to a disjoint class), not only asserted ones.
fn closure_inconsistencies(data: &Graph, profile: Profile) -> Vec<String> {
    let mut dict = data.dict().clone();
    let mut triples: Vec<[Id; 3]> = data.iter_ids().collect();
    sparq_reason::materialize(profile, &mut dict, &mut triples);
    sparq_reason::inconsistencies(&dict, &triples)
}

/// Build a new [`Graph`] that is `data` plus the single triple `(subject, predicate, object)`.
/// The ids are already interned in `data`'s dictionary (the caller checked), so the augmented
/// graph reuses the same dictionary and simply adds the triple.
fn augment(data: &Graph, subject: Id, predicate: Id, object: Id) -> Result<Graph, String> {
    let dict: Dict = data.dict().clone();
    let mut triples: Vec<[Id; 3]> = data.iter_ids().collect();
    triples.push([subject, predicate, object]);
    Ok(Graph::from_parts(dict, triples))
}

/// Whether `id` is a real (non-inline) term present in `graph`'s dictionary — used by the
/// fail-closed checks so an unknown candidate is rejected, not silently treated as valid.
fn graph_has_id(graph: &Graph, id: Id) -> bool {
    if dict::is_inline(id) {
        return true;
    }
    // A round-trip through the dictionary: the id resolves to a term that re-interns to the same
    // id. `lookup` returns NO_ID for an absent term, and a fabricated out-of-range id resolves to
    // a term that does NOT re-look-up to the same id, so it is rejected.
    let term = graph.dict().term(id);
    graph.dict().lookup(&term) == id
}

/// Whether `graph` already asserts the exact triple `(s, p, o)`.
fn graph_contains(graph: &Graph, s: Id, p: Id, o: Id) -> bool {
    let scan = graph.store().scan(&[Some(s), Some(p), Some(o)]);
    scan.rows.iter().any(|row| {
        let [rs, rp, ro] = scan.to_spo(row);
        rs == s && rp == p && ro == o
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::VectorStore;
    use sparq_shacl::ShapesModel;

    fn iri(s: &str) -> oxrdf::Term {
        oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
    }

    fn id_of(graph: &Graph, s: &str) -> Id {
        graph
            .id_of(&iri(s))
            .unwrap_or_else(|| panic!("missing term {s}"))
    }

    // A schema-rich data graph: two disjoint classes (Person/Project), a person and a project,
    // a `worksOn` predicate, and an `age` integer property. The vectors (built below) deliberately
    // place the *wrong-typed* candidate nearest, so the neural propose step surfaces an unsound
    // candidate the deductive gate must reject.
    const DATA: &str = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:   <http://ex/> .

ex:Person  owl:disjointWith ex:Project .

ex:alice a ex:Person ; ex:age 30 .
ex:bob   a ex:Person ; ex:age 41 .
ex:proj1 a ex:Project .
ex:proj2 a ex:Project .
ex:alice ex:worksOn ex:proj1 .
"#;

    // A SHACL shape: ex:worksOn values must be ex:Project (sh:class). A candidate that is a
    // Person, not a Project, introduces a new sh:class violation => must be REJECTED.
    const SHAPES: &str = r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix ex:  <http://ex/> .
ex:PersonShape a sh:NodeShape ;
  sh:targetClass ex:Person ;
  sh:property [
    sh:path ex:worksOn ;
    sh:class ex:Project ;
  ] .
"#;

    fn data_graph() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    fn shapes_model() -> ShapesModel {
        let shapes = Graph::load_str(SHAPES, "turtle").unwrap();
        ShapesModel::parse(&shapes)
    }

    // Build a tiny vector store over the data graph's entity ids where `ex:bob` (a Person) is the
    // nearest neighbour of `ex:alice` — so a vector propose for alice's worksOn slot surfaces bob,
    // a WRONG-TYPED (Person, not Project) candidate. The store is keyed by dict id.
    fn vector_store(graph: &Graph, path: &std::path::Path) -> VectorStore {
        let dim = 4;
        let mut store = VectorStore::create(path, dim).unwrap();
        let put = |store: &mut VectorStore, term: &str, v: [f32; 4]| {
            let id = id_of(graph, term);
            store.put(id, &v).unwrap();
        };
        // alice and bob point in nearly the same direction (close); projects are orthogonal.
        put(&mut store, "http://ex/alice", [1.0, 0.0, 0.0, 0.0]);
        put(&mut store, "http://ex/bob", [0.99, 0.1, 0.0, 0.0]);
        put(&mut store, "http://ex/proj1", [0.0, 0.0, 1.0, 0.0]);
        put(&mut store, "http://ex/proj2", [0.0, 0.0, 0.9, 0.1]);
        store.finalize().unwrap();
        store
    }

    fn tmp_spqv(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sparq-vectors-verify-{tag}-{}-{}.spqv",
            std::process::id(),
            nanos
        ))
    }

    // ===== THE LOAD-BEARING SOUNDNESS TEST =====
    // A vector-proposed-but-shape-invalid candidate is REJECTED by the deductive gate, not merely
    // down-ranked. ex:bob (a Person) is the nearest vector neighbour of ex:alice, so the neural
    // propose surfaces bob for alice's `worksOn` slot; the SHACL `sh:class ex:Project` shape and
    // the OWL `Person disjointWith Project` axiom both make bob unsound => the gate must drop it.
    #[test]
    fn verify_rejects_vector_proposed_shape_invalid_candidate() {
        let graph = data_graph();
        let model = shapes_model();
        let path = tmp_spqv("reject");
        let store = vector_store(&graph, &path);

        let alice = id_of(&graph, "http://ex/alice");
        let works_on = id_of(&graph, "http://ex/worksOn");
        let bob = id_of(&graph, "http://ex/bob");
        let proj2 = id_of(&graph, "http://ex/proj2");

        // The neural propose surfaces bob as the top candidate (nearest to alice).
        let proposed = propose_by_vector(&store, alice, 3);
        assert!(
            proposed.contains(&bob),
            "vector propose should surface the wrong-typed neighbour bob: {:?}",
            proposed
        );

        let report = propose_then_verify(
            &graph,
            &store,
            alice,
            alice,
            works_on,
            3,
            Some(&model),
            &VerifyConfig::default(),
        );
        let verified = report.verified();

        // SOUNDNESS: bob (a Person, violating sh:class ex:Project AND Person/Project disjointness)
        // is REJECTED — it must NOT be in the verified set even though it was the nearest vector.
        assert!(
            !verified.contains(&bob),
            "DEDUCTIVE GATE FAILED: a shape-invalid vector candidate (bob) was surfaced as verified"
        );
        // ...and it IS recorded as rejected with a real reason (the audit trail).
        let bob_verdict = report
            .verdicts
            .iter()
            .find(|v| v.object == bob)
            .expect("bob must have a verdict");
        assert!(
            bob_verdict.rejected.is_some(),
            "bob must be rejected, not verified"
        );

        // A type-valid candidate (proj2, a Project) verifies — the gate shrinks to a sound subset,
        // it does not reject everything.
        let proj2_verdict = verify_candidate(
            &graph,
            alice,
            works_on,
            proj2,
            Some(&model),
            &VerifyConfig::default(),
        );
        assert!(
            proj2_verdict.is_verified(),
            "a type-valid Project candidate must pass the gate: {:?}",
            proj2_verdict.rejected
        );

        // verify-shrinks-to-sound: the verified set is a SUBSET of the proposed set.
        let proposed_set: FxHashSet<Id> = proposed.iter().copied().collect();
        for v in &verified {
            assert!(
                proposed_set.contains(v),
                "verified candidate {v} was not in the proposed set (gate must only shrink)"
            );
        }

        drop(store);
        std::fs::remove_file(&path).ok();
    }

    // The OWL half alone (no SHACL) still rejects a disjoint-class clash: typing alice as a
    // Project (she is already a Person, disjoint with Project) is an inconsistency the closure
    // catches via `cax-dw`.
    #[test]
    fn verify_rejects_disjoint_class_binding_via_owl() {
        let graph = data_graph();
        let alice = id_of(&graph, "http://ex/alice");
        let rdf_type = graph
            .id_of(&iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"))
            .unwrap();
        let project = id_of(&graph, "http://ex/Project");

        // Binding `alice rdf:type Project` clashes with `alice a Person` + `Person disjointWith
        // Project`. Run with SHACL off so ONLY the deductive OWL gate decides.
        let cfg = VerifyConfig {
            check_shacl: false,
            ..Default::default()
        };
        let verdict = verify_candidate(&graph, alice, rdf_type, project, None, &cfg);
        assert!(
            matches!(verdict.rejected, Some(Rejection::OwlInconsistency(_))),
            "binding a disjoint type must be rejected by the OWL gate: {:?}",
            verdict
        );
    }

    // A binding that introduces no new violation/inconsistency is VERIFIED (a real Project for the
    // worksOn slot). Confirms the gate is not a reject-everything filter.
    #[test]
    fn verify_accepts_type_valid_binding() {
        let graph = data_graph();
        let model = shapes_model();
        let alice = id_of(&graph, "http://ex/alice");
        let works_on = id_of(&graph, "http://ex/worksOn");
        let proj2 = id_of(&graph, "http://ex/proj2");

        let verdict = verify_candidate(
            &graph,
            alice,
            works_on,
            proj2,
            Some(&model),
            &VerifyConfig::default(),
        );
        assert!(verdict.is_verified(), "{:?}", verdict.rejected);
    }

    // FAIL-CLOSED: a candidate id that is not a real term of the graph is REJECTED as
    // uncheckable, never surfaced as verified.
    #[test]
    fn verify_fails_closed_on_unknown_candidate() {
        let graph = data_graph();
        let alice = id_of(&graph, "http://ex/alice");
        let works_on = id_of(&graph, "http://ex/worksOn");
        // An id far outside the dictionary's range (not inline, not present).
        let bogus: Id = u32::MAX - 7;
        let verdict = verify_candidate(
            &graph,
            alice,
            works_on,
            bogus,
            None,
            &VerifyConfig::default(),
        );
        assert!(
            matches!(verdict.rejected, Some(Rejection::Uncheckable(_))),
            "an unknown candidate must fail closed: {:?}",
            verdict
        );
    }

    // An already-asserted binding is trivially verified (it is, by definition, consistent with a
    // graph that already contains it).
    #[test]
    fn verify_accepts_already_asserted_binding() {
        let graph = data_graph();
        let model = shapes_model();
        let alice = id_of(&graph, "http://ex/alice");
        let works_on = id_of(&graph, "http://ex/worksOn");
        let proj1 = id_of(&graph, "http://ex/proj1"); // alice worksOn proj1 is asserted
        let verdict = verify_candidate(
            &graph,
            alice,
            works_on,
            proj1,
            Some(&model),
            &VerifyConfig::default(),
        );
        assert!(verdict.is_verified(), "{:?}", verdict.rejected);
    }

    // The pipeline's verified set is always a subset of what the neural step proposed (the
    // shrink-to-sound invariant), over a propose with a larger k.
    #[test]
    fn pipeline_verified_is_subset_of_proposed() {
        let graph = data_graph();
        let model = shapes_model();
        let path = tmp_spqv("subset");
        let store = vector_store(&graph, &path);
        let alice = id_of(&graph, "http://ex/alice");
        let works_on = id_of(&graph, "http://ex/worksOn");

        let report = propose_then_verify(
            &graph,
            &store,
            alice,
            alice,
            works_on,
            4,
            Some(&model),
            &VerifyConfig::default(),
        );
        let proposed: FxHashSet<Id> = propose_by_vector(&store, alice, 4).into_iter().collect();
        for v in report.verified() {
            assert!(proposed.contains(&v), "verified {v} not proposed");
        }
        assert!(
            report.proposed_count() >= report.verified().len(),
            "the gate can only shrink"
        );
        drop(store);
        std::fs::remove_file(&path).ok();
    }
}
