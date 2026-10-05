#![doc = include_str!("../README.md")]
// [OPUS-4.8] MS-G2 (sq-8wbn): make `// SAFETY:` mandatory on every first-party unsafe block.
#![warn(clippy::undocumented_unsafe_blocks)]

pub mod ann;
// [OPUS-4.8] (sq-lfo84) Explicit-SIMD (NEON/AVX2) squared-Euclidean distance kernel for the HNSW
// ANN hot loop. `approx-ann` only (the HNSW backend it serves); no new dependency — pure
// `core::arch` intrinsics with a scalar fallback bit-identical to the previous auto-vectorised loop.
#[cfg(feature = "approx-ann")]
mod simd;
// [OPUS-4.8] (sq-ip3a) Pluggable ANN backend seam (exact default vs approximate) + the iterative
// over-fetch FILTERED path — `filtered-ann` only (the approximate impl is additionally `approx-ann`).
#[cfg(feature = "filtered-ann")]
pub mod backend;
// [OPUS-4.8] (sq-7hx6) Filtered-ANN pre-filter vs post-filter cost model — `filtered-ann` only.
#[cfg(feature = "filtered-ann")]
pub mod cost;
// [OPUS-4.8] (sq-pi44) Incremental add/remove/update of vectors against a finalized store via an
// in-RAM delta sidecar (+ `compact`) — `delta` feature only. Additive and dependency-free; the
// default build carries no delta code.
#[cfg(feature = "delta")]
pub mod delta;
pub mod diskann;
pub mod embed;
// [OPUS-4.8] sq-0wo9e.2 (epic sq-0wo9e): the P1 structure-aware-vectorisation TYPED-LITERAL
// ENCODERS — the datatype router + order-preserving numeric + boolean-sign + date encoders, plus
// the self-describing `.spqv` SchemaHeader (per-block metric tags + metric-correctness guard).
// `structure` feature only; pure, dependency-light functions keyed by datatype (only `temporal_value`
// / the date encoder touch sparq-core, already in the tree). The default build carries zero encoder code.
#[cfg(feature = "structure")]
pub mod encode;
// [OPUS-4.8] sq-0wo9e.3 (epic sq-0wo9e): P2 — the categorical CODEBOOK encoder for `sh:in`/`owl:oneOf`
// enum members (slot-match exactness, reserved out-of-enum invalid code) and the QUDT UNIT
// normaliser (`1000 m` and `1 km` share a code before the order-preserving numeric encoder).
// `structure` feature only; pure, dependency-light (no SHACL/engine dep — the SHACL *reader* below
// is the only thing that pulls sparq-shacl). The default build carries zero P2 code.
#[cfg(feature = "structure")]
pub mod codebook;
#[cfg(feature = "structure")]
pub mod units;
// [OPUS-4.8] sq-0wo9e.3 (epic sq-0wo9e): P2 — the SHACL/OWL PRIOR EXTRACTOR. Reads enum (`sh:in`) /
// datatype (`sh:datatype`) / cardinality (`sh:min,maxCount`, `owl:FunctionalProperty`) priors out of
// a parsed `sparq-shacl` shapes model (no SHACL changes — a read-only reader). `structure-shacl`
// feature only: it is the ONLY feature pulling `sparq-shacl` into this crate's graph, so neither the
// default build nor the lean `structure` feature gains a SHACL/engine dependency.
#[cfg(feature = "filtered-ann")]
pub mod filter;
pub mod fingerprint;
#[cfg(feature = "structure-shacl")]
pub mod shacl_priors;
// [OPUS-4.8] sq-0wo9e.5 (epic sq-0wo9e): the P4 structure-aware-vectorisation FLEXIBLE
// MINIMAL-AND-COMPLETE GROUNDING selector + verbaliser — a modality dispatcher on the consumer's
// declared output type producing the minimal-and-complete object (subgraph / typed sub-vector / NL
// string / typed value), with PROFILE-RELATIVE completeness and ABSTAT-style minimality. `structure`
// feature only (reuses `encode`/`structure`/`verbalize` + `sparq-introspect`); the default build
// carries zero grounding code.
#[cfg(feature = "structure")]
pub mod grounding;
// [OPUS-4.8] sq-0wo9e.6 (epic sq-0wo9e): the P5 NEURO-SYMBOLIC propose(neural)-then-verify(deductive)
// grounding pipeline — vector ANN PROPOSES candidates (recall only, NOT sound), then a DEDUCTIVE GATE
// admits a binding only if it introduces no new SHACL violation (`sparq-shacl`) and no new OWL
// inconsistency (`sparq-reason::inconsistencies` over the closure); a failing candidate is REJECTED
// (fail-closed). `neuro-symbolic` feature only (implies `structure-shacl` → reuses sparq-shacl +
// sparq-reason); the default build carries zero pipeline code and makes no accuracy claim.
#[cfg(feature = "neuro-symbolic")]
pub mod verify;
// [OPUS-4.8] sq-mztg8.1 (epic sq-mztg8, design research/fo-llm-bridge.md §2.3/§3.3/§6 Phase 3):
// the READ-PATH URI-HIDING "compose" step + a closed NL→NL A/B fidelity/cost harness — present an
// answer-row binding to the agent as a label/grounded view instead of a raw IRI, behind the
// #1074 echo/confidence envelope, and measure (model-free) whether the hidden view preserves
// answer identity. `compose` feature only (implies `structure`, reusing `verbalize`); the default
// build carries zero compose code and the accuracy half (real-model A/B) is registered UNMEASURED.
#[cfg(feature = "compose")]
pub mod compose;
pub mod fuse;
// [SONNET-4.6] sq-lhcot.4 (GenAI review gap 6; spec estate sq-rvgr2.4): the HYBRID RETRIEVAL +
// RERANKING core behind the `vec:hybrid` magic predicate — named weighted-RRF arms carrying
// per-rank PROVENANCE, an out-of-process second-stage `Reranker` with an explicit
// fail-open/fail-closed policy, and the dense/sparse/structural/fused/reranked ablation harness.
// `vec-predicate` feature only: the module exists to serve that query surface and ships with it,
// so the default build carries zero hybrid code. It adds NO new dependency (weighted RRF is the
// in-tree `fuse` algorithm plus bookkeeping) and NO accuracy claim — `ablate` reports, it never
// asserts that fusion or reranking wins.
#[cfg(feature = "vec-predicate")]
pub mod hybrid;
pub mod import;
pub mod labels;
// [GPT-5.6] sq-lsp7k.25: deterministic `.spqv` v4 metadata-block codec. Private implementation;
// the public surface is `VectorStore::{put_with_meta,meta}` + `nearest_exact_with_meta`, all behind
// the default-OFF `metadata-sidecar` feature.
#[cfg(feature = "metadata-sidecar")]
mod metadata;
// [FABLE-5] (#2251) Recall-gated ANN over raw concept vectors — `build_ann` / `knn` / `dedup`:
// an HNSW index over caller-supplied (id, vector) pairs plus type-level dedup whose merges are
// emitted ONLY after the index's measured recall vs an exact O(m²) ground truth clears a
// pre-registered gate (default 0.99; fail-closed). `approx-ann` only — it rides the same opt-in
// instant-distance backend as `VectorIndex`, so the default build carries none of it and gains
// no new dependency. First consumer: the Kernel-of-Truth scale track (issue #2251).
#[cfg(feature = "approx-ann")]
pub mod dedup;
pub mod quant;
// [FABLE-5] sq-lhcot.1 (GenAI review gap 1; spec estate sq-rvgr2.4): the `.spqv` v3 EMBEDDING
// PROVENANCE record (model id / model+content version / metric / normalization / verbalization
// regime) + the mandatory compatibility rejection. This module is ALWAYS compiled — a v3 store
// written by a feature-on build must still OPEN on a feature-off build (mirroring how v1/v2 both
// open regardless of features); only the v3 WRITE path (`VectorStore::with_provenance` → finalize)
// and the demanding query check are gated behind the opt-in `spqv-provenance` feature. Lean: a
// hand-rolled fixed-width LE codec, no new dependency. The extension point stays a reserved, opaque,
// versioned area with NO fields defined (pending the #1746 profile freeze — the KERN boundary).
#[cfg(feature = "vec-predicate")]
pub mod rewrite;
pub mod spqv_provenance;
pub mod store;
// [OPUS-4.8] sq-0wo9e.1 (epic sq-0wo9e): the P0 structure-aware-vectorisation preprocessing +
// sampling-logic layer — closure-before-vectorise + type-constrained negative sampling. `structure`
// feature only; the only feature pulling sparq-reason + sparq-introspect, so the default build
// carries zero structure-prep code.
#[cfg(feature = "structure")]
pub mod structure;
// [OPUS-4.8] sq-2489d.4 (epic sq-2489d, GenAI-KB Phase 4, design
// research/provenance-driven-genai-kb.md §USE-1 / §5 Phase 4): the PKG→`w(t)` PROVENANCE-WEIGHT
// reader — a read-only id-level scan of `pkg:confidence`/`pkg:assurance`/`prov:wasDerivedFrom` into
// a per-triple provenance-quality weight, mirroring `shacl_priors.rs`. `structure` feature only; it
// consumes only sparq-core's public id-scan API and pulls NO new dependency (it does NOT use the
// SPARQL-backed Phase-1 join, so the engine never enters this crate's `structure` graph). The
// default build carries zero provenance-weight code.
#[cfg(feature = "structure")]
pub mod provenance;
// [OPUS-4.8] sq-0wo9e.8 (epic sq-0wo9e): the MEASUREMENT FOUNDATION — a thin shallow-KGE trainer
// (`train`; symmetric DistMult or asymmetric ComplEx via `ModelKind`) over the P0 closure +
// type-constrained negatives, and the filtered link-prediction eval harness (`eval`) with the
// {closure}×{type-neg} ablation matrix (single- and multi-seed), long-tail breakdown, and synthetic
// gUFO slice. `kge` feature only (implies `structure`); the default build carries zero trainer/eval
// code and no new dependency.
#[cfg(feature = "kge")]
pub mod eval;
#[cfg(feature = "kge")]
pub mod train;
// [OPUS-4.8] sq-0wo9e.4 (epic sq-0wo9e): the P3 structure-aware-vectorisation TAXONOMY block
// (Euclidean default; hyperbolic only past a measured-distortion gate) + the answer-safe
// disjointness repulsion/mask. Same `structure` feature (off by default); reads sparq-reason's
// materialised closure via the P0 `close_for_vectorise` path and adds no new dependency.
#[cfg(feature = "structure")]
pub mod taxonomy;
// [FABLE-5] kern/ufo-priors (epic sq-0wo9e): the READ-ONLY UFO/gUFO PRIORS READER — mines gUFO
// meta-types (Kind/Role/Phase/Category/…), rigidity, identity providers, ontological natures, and
// mediation/inherence witnesses from the graph, and derives a UFO-PROVABLE disjointness +
// subsumption mask fed into the P3 `DisjointnessOracle` (answer-safe: only proven pairs, fail
// closed on ill-formed models). Same `structure` feature (off by default); no new dependency —
// the default build carries zero UFO-prior code. The eval harness's `gufo_prior` axis (default
// OFF) is the only consumer that changes behaviour, and only when explicitly enabled.
#[cfg(feature = "structure")]
pub mod ufo_priors;
pub mod verbalize;

/// The `vec:` vocabulary — magic predicates recognised by `rewrite`
/// (`http://sparq.dev/vec#`, the sparq extension namespace). [OPUS-4.8] (sq-k6ex)
pub mod vocab {
    /// `vec:` — the sparq vector-search namespace.
    pub const VEC_NS: &str = "http://sparq.dev/vec#";
    /// [SONNET-4.6] (sq-tb9p0) The highest `vec:` **vocabulary revision** this build implements
    /// (spec `site/specs/sparql-vector-genai.typ`, VG-GOV-2/VG-GOV-3). Revision 1's recognised
    /// terms are exactly [`NEAREST`] and [`SEARCH`]; new terms are introduced only by a new spec
    /// revision, and an unrecognised `vec:` IRI is a HARD query error naming this revision — the
    /// loud failure IS the forward-compatibility mechanism (a query using a newer revision's term
    /// against this processor fails instead of silently matching data).
    pub const VOCAB_REVISION: u32 = 1;
    /// `?node vec:nearest ( <query> <k> )` — binds `?node` to the `<k>` nearest
    /// neighbours (best first) of `<query>`, which is either a node IRI (whose
    /// stored vector is the query, the seed excluded) or a comma-separated
    /// vector literal.
    pub const NEAREST: &str = "http://sparq.dev/vec#nearest";
    /// `( ?node ?score ) vec:search ( <query> <k> )` — like [`NEAREST`] but also
    /// binds `?score` to each neighbour's cosine similarity (`xsd:double`).
    pub const SEARCH: &str = "http://sparq.dev/vec#search";
    /// [SONNET-4.6] (sq-lhcot.4) `( ?node ?score ?rank ?prov ) vec:hybrid ( <query> <k> )` —
    /// **hybrid retrieval**: the dense k-NN fused (deterministic weighted RRF) with the caller's
    /// other retrieval arms and optionally reranked, binding the final score, the final 1-based
    /// rank, and the per-arm rank PROVENANCE. See the `hybrid` and `rewrite` modules (both
    /// `vec-predicate`-gated, hence code spans rather than intra-doc links here: this constant is
    /// always compiled).
    ///
    /// **PROVISIONAL** — listed in [`PROVISIONAL`], NOT in revision [`VOCAB_REVISION`]: the term
    /// is implemented ahead of the matching amendment to the SPARQL Vector+GenAI extension draft,
    /// and its shape may change when that spec revision lands.
    pub const HYBRID: &str = "http://sparq.dev/vec#hybrid";
    /// [SONNET-4.6] (sq-lhcot.4) `vec:` terms this build implements that are **not yet part of a
    /// published spec revision**. They are usable (behind their feature) but unfrozen: a
    /// provisional term may change shape without a [`VOCAB_REVISION`] bump, so a query relying on
    /// one is relying on an unstable surface. A term moves out of this list, and the revision
    /// bumps, only when the spec draft is amended.
    pub const PROVISIONAL: &[&str] = &[HYBRID];
}

/// [FABLE-5] (sq-lhcot.1) The `spqvp:` **embedding-provenance vocabulary** — the RDF terms a caller
/// uses to ASSERT (in the graph or a sidecar dataset) the embedding pipeline a `.spqv` v3 store was
/// built with: model identifier, model/content version, similarity metric, normalization, dimension,
/// and verbalization regime. These IRIs describe the compatibility axes the v3 header records so the
/// provenance is expressible in RDF as well as in the binary header; the crate does NOT define or
/// privilege any encoder — the values are caller-supplied.
///
/// The namespace is `http://sparq.dev/spqv-prov#`. The reserved extension terms are deliberately
/// NOT defined here — extension fields are reserved pending the cross-implementation profile (#1746).
pub mod prov_vocab {
    /// `spqvp:` — the sparq embedding-provenance namespace.
    pub const SPQVP_NS: &str = "http://sparq.dev/spqv-prov#";
    /// `spqvp:model` — the model identifier the store's vectors were produced by (a literal token).
    pub const MODEL: &str = "http://sparq.dev/spqv-prov#model";
    /// `spqvp:modelVersion` — the model revision (a literal token).
    pub const MODEL_VERSION: &str = "http://sparq.dev/spqv-prov#modelVersion";
    /// `spqvp:contentVersion` — the graph→text (verbaliser) content revision (a literal token).
    pub const CONTENT_VERSION: &str = "http://sparq.dev/spqv-prov#contentVersion";
    /// `spqvp:metric` — the similarity metric (`cosine` / `dot` / `euclidean`, a literal token).
    pub const METRIC: &str = "http://sparq.dev/spqv-prov#metric";
    /// `spqvp:normalization` — the vector normalization (`none` / `l2`, a literal token).
    pub const NORMALIZATION: &str = "http://sparq.dev/spqv-prov#normalization";
    /// `spqvp:dimension` — the vector dimension (an `xsd:integer` literal; also the store header dim).
    pub const DIMENSION: &str = "http://sparq.dev/spqv-prov#dimension";
    /// `spqvp:verbalization` — the verbalization regime name (a literal token).
    pub const VERBALIZATION: &str = "http://sparq.dev/spqv-prov#verbalization";
}

#[cfg(feature = "metadata-sidecar")]
pub use ann::nearest_exact_with_meta;
pub use ann::{
    cosine, nearest_exact, nearest_exact_tiebreak, nearest_term_exact, nearest_term_exact_checked,
};
// [OPUS-4.8] (sq-ip3a) The in-RAM HNSW index is the approximate backend — `approx-ann` only
// (the only feature pulling `instant-distance`). The default build re-exports just the exact
// searchers above.
#[cfg(feature = "approx-ann")]
pub use ann::{HnswConfig, VectorIndex};
// [FABLE-5] (#2251) The recall-gated concept-ANN + dedup surface — `approx-ann` only (`dedup.rs`).
#[cfg(feature = "approx-ann")]
pub use dedup::{
    build_ann, dedup, exact_ground_truth, knn, ConceptAnnIndex, DedupPolicy, DedupReport,
    GroundTruth,
};
pub use diskann::{sibling_graph_path, DiskAnnIndex, VamanaConfig, SPQG_MAGIC, SPQG_VERSION};
pub use embed::{Embedder, HashEmbedder};
// [OPUS-4.8] (sq-1wc1) Predicate-constrained (filtered) ANN — the `filtered-ann` feature only.
#[cfg(feature = "filtered-ann")]
pub use filter::{nearest_exact_filtered, FilterConfig, IdMask};
// [OPUS-4.8] (sq-7hx6) Pre-filter vs post-filter cost model — the `filtered-ann` feature only.
#[cfg(feature = "filtered-ann")]
pub use cost::{
    nearest_filtered_costed, nearest_filtered_costed_tiebreak, overfetch_target, postfilter_exact,
    CostEstimate, CostModel, Strategy,
};
// [OPUS-4.8] (sq-ip3a) Pluggable ANN backend + iterative over-fetch filtered path — `filtered-ann`
// only. `ApproxBackend` is additionally `approx-ann` (re-exported below).
#[cfg(all(feature = "filtered-ann", feature = "approx-ann"))]
pub use backend::ApproxBackend;
#[cfg(feature = "filtered-ann")]
pub use backend::{
    nearest_filtered_overfetch, nearest_filtered_overfetch_default, AnnBackend, ExactBackend,
    DEFAULT_MAX_ROUNDS,
};
pub use fingerprint::{check_against, Artifact, CheckResult, Fingerprint, FINGERPRINT_LEN};
pub use fuse::{fuse_rrf, fuse_rrf_weighted, fuse_scores, hybrid_search, Retriever, RRF_K};
pub use import::{ImportBinding, ImportSpec, MAX_NPY_HEADER_LEN};
pub use labels::{embed_labels, embed_labels_with, LabelConfig};
pub use quant::{
    cosine_from_sq_dist, DistanceTable, EncodedStore, PqConfig, ProductQuantizer, ScalarQuantizer,
};
#[cfg(feature = "vec-predicate")]
pub use rewrite::{prepare_vec, query_vec, query_vec_with_budget, rewrite_query};
// [SONNET-4.6] sq-lhcot.4: the `vec:hybrid` entry points + the fusion/rerank surface they consume.
// `vec-predicate` only, like the rest of the magic-predicate API.
#[cfg(feature = "vec-predicate")]
pub use hybrid::{
    ablate, apply_rerank, evaluate, fuse_arms, parse_provenance, validate_arms, AblationRow,
    ArmFn, ArmQuery, ArmRanking, FusedHit, HybridConfig, QueryEmbedder, RerankPolicy, Reranker,
    Rescored, RetrievalMetrics, DEFAULT_OVER_FETCH, FUSED_ROW, MAX_ARM_PAGE, RERANKED_ROW,
    RERANK_KEY, VECTOR_ARM,
};
#[cfg(feature = "vec-predicate")]
pub use rewrite::{
    prepare_vec_hybrid, query_vec_hybrid, query_vec_hybrid_with_budget, rewrite_query_hybrid,
};
// [OPUS-4.8] (sq-z589, epic sq-3183) The APPROXIMATE `vec:` entry points — the unfiltered k-NN runs
// through an on-disk `DiskAnnIndex` (Vamana) instead of the exact full scan, for large `.spqv`
// stores. APPROXIMATE (recall < 1.0); gated on `approx-ann` (the only feature that compiles an
// approximate index) on top of `vec-predicate`.
#[cfg(all(feature = "vec-predicate", feature = "approx-ann"))]
pub use rewrite::{prepare_vec_approx, query_vec_approx, query_vec_approx_with_budget};
// Re-export the engine result/budget types the `vec:` entry points return/take (and `query_prepared`
// so callers can evaluate a `prepare_vec*` `PreparedQuery`), so callers (and tests) need not also
// declare a direct `sparq-engine` dependency. [OPUS-4.8] (sq-k6ex; query_prepared added sq-z589)
#[cfg(feature = "vec-predicate")]
pub use sparq_engine::{query_prepared, PreparedQuery, QueryBudget, QueryResult};
// [OPUS-4.8] (sq-pi44) The incremental delta sidecar value type — `delta` feature only. The
// add/remove/update/compact APIs live on `VectorStore` (also `delta`-gated).
// [OPUS-4.8] (sq-7e50) The persisted `.spqd` sidecar magic/version — the save/open APIs
// (`save_delta`/`open_with_delta`/`sibling_delta_path`) live on `VectorStore` (also `delta`-gated).
#[cfg(feature = "delta")]
pub use delta::{VectorDelta, SPQD_MAGIC, SPQD_VERSION};
pub use store::{
    LegacyMode, StreamingWriter, VectorStore, SPQV_MAGIC, SPQV_VERSION, SPQV_VERSION_V3,
};
// [FABLE-5] sq-lhcot.1: the `.spqv` v3 embedding-provenance surface — the record, its typed metric /
// normalization axes, and the RDF vocabulary IRIs. Always compiled (the v3 read path); the vocab
// namespace lets a caller assert provenance in RDF (see `skills/vector-search/SKILL.md`).
pub use spqv_provenance::{
    EmbeddingProvenance, Metric as EmbeddingMetric, Normalization, MAX_PROVENANCE_BLOCK_LEN,
    PROVENANCE_BLOCK_VERSION,
};
// [OPUS-4.8] sq-0wo9e.1 (epic sq-0wo9e): the structure-aware-vectorisation P0 surface — the
// closure-before-vectorise step, the type-constraint extractor, and the type-constrained negative
// sampler with its on/off ablation switch. `structure` feature only.
#[cfg(feature = "structure")]
pub use structure::{
    close_for_vectorise, materialise_closure, ClosedGraph, Corrupt, NegativeSampler, SamplingMode,
    TermScope, TypeConstraints,
};
// [OPUS-4.8] sq-2489d.4 (epic sq-2489d, GenAI-KB Phase 4): the provenance-weight surface — the
// PKG→`w(t)` reader, its on/off ablation switch, and the tunable factors. `structure` feature only.
#[cfg(feature = "structure")]
pub use provenance::{ProvenanceWeights, WeightConfig, WeightMode};
// [OPUS-4.8] sq-0wo9e.8 (epic sq-0wo9e): the DistMult trainer + filtered link-prediction eval
// harness surface — `kge` feature only.
#[cfg(feature = "kge")]
pub use eval::{
    run_ablation, run_ablation_multiseed, run_ablation_multiseed_paired, run_pooling_ablation,
    run_quoted_ablation, run_weight_ablation, synthetic_gufo_ttl, synthetic_gufo_ttl_sized,
    synthetic_provenance_ttl, synthetic_rdf12_parts, synthetic_rdf12_ttl, synthetic_relational_ttl,
    AblationCell, CellStats, EvalConfig, LongTail, MeanStd, Metrics, MultiSeedCell, PairedAblation,
    PairedDelta, PoolingAblation, QuotedAblation, Rdf12Parts, Splits, WeightAblation,
    SCHEMA_PREDICATES,
};
#[cfg(feature = "kge")]
pub use train::{train, ModelKind, TrainConfig, TrainReport, TrainedModel};
// [OPUS-4.8] sq-0wo9e.2 (epic sq-0wo9e): the structure-aware-vectorisation P1 surface — the typed
// literal encoders (datatype router + order-preserving numeric + boolean-sign + date) and the
// self-describing `.spqv` SchemaHeader (block partition + per-block metric tag + cosine guard).
// `structure` feature only.
#[cfg(feature = "structure")]
pub use encode::{
    metamorphic_monotone, numeric_value, route, temporal_value, Block, BooleanEncoder, DateEncoder,
    Encoder, Metric, NumericEncoder, SchemaHeader, SPQS_MAGIC, SPQS_VERSION,
};
// [OPUS-4.8] sq-0wo9e.3 (epic sq-0wo9e): the P2 enum codebook + QUDT unit-normaliser surface —
// `structure` feature only (pure, no SHACL dep).
#[cfg(feature = "structure")]
pub use codebook::{Codebook, INVALID_SLOT};
#[cfg(feature = "structure")]
pub use units::{
    is_known, normalise, normalise_lexical, quantity_kind, same_quantity, Normalised, QuantityKind,
    QUDT_UNIT_NS,
};
// [OPUS-4.8] sq-0wo9e.3 (epic sq-0wo9e): the P2 SHACL/OWL prior-extractor surface —
// `structure-shacl` feature only (the only feature pulling sparq-shacl).
#[cfg(feature = "structure-shacl")]
pub use shacl_priors::{Cardinality, PredicatePrior, ShaclPriors};
// [OPUS-4.8] sq-0wo9e.4 (epic sq-0wo9e): the structure-aware-vectorisation P3 surface — the
// `subClassOf` taxonomy DAG + Euclidean (default) / hyperbolic (candidate) encoders, the
// measured-distortion `GeometryGate`, and the answer-safe `DisjointnessOracle` (train-time
// repulsion pairs + serve-time hard mask). `structure` feature only.
#[cfg(feature = "structure")]
pub use taxonomy::{
    DisjointnessOracle, DistortionReport, EuclideanTaxonomyEncoder, Geometry, GeometryGate,
    HyperbolicTaxonomyEncoder, TaxonomyDag,
};
// [FABLE-5] kern/ufo-priors (epic sq-0wo9e): the READ-ONLY UFO/gUFO priors surface — the reader,
// its meta-type/rigidity/nature vocabulary, and the canonical gUFO namespace. `structure` only.
#[cfg(feature = "structure")]
pub use ufo_priors::{MetaType, Nature, Rigidity, UfoPriors, GUFO_NS};
// [OPUS-4.8] sq-0wo9e.5 (epic sq-0wo9e): the structure-aware-vectorisation P4 surface — the
// flexible minimal-and-complete grounding selector + verbaliser. `structure` feature only.
#[cfg(feature = "structure")]
pub use grounding::{
    ground, ground_weighted, reconcile_quantity, sketch_predicate, GroundFact, Grounding,
    GroundingConfig, Modality, NodeWeighting, OutputType, TypedValue,
};
pub use verbalize::{
    description_predicates, embed_entities, label_predicates, verbalize, EntityTextConfig,
    ObjectKind, PropertyGroup,
};
// [OPUS-4.8] sq-0wo9e.6 (epic sq-0wo9e): the structure-aware-vectorisation P5 surface — the
// neuro-symbolic propose(neural)-then-verify(deductive) grounding pipeline. `neuro-symbolic`
// feature only (implies `structure-shacl`).
#[cfg(feature = "neuro-symbolic")]
pub use verify::{
    propose_by_vector, propose_then_verify, verify_candidate, verify_candidates, Rejection, Source,
    Verdict, VerifyConfig, VerifyReport,
};
