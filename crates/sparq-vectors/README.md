<!-- [OPUS-4.8] sq-inzv: README brought to template. -->
# sparq-vectors

<p>
  <a href="https://crates.io/crates/sparq-vectors"><img src="https://img.shields.io/crates/v/sparq-vectors.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-vectors"><img src="https://docs.rs/sparq-vectors/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**Opt-in embedding store + nearest-neighbour search** for the [sparq](../../README.md)
RDF engine (GenAI phase 4).

One f32 embedding per dictionary term id, in a flat memory-mapped `.spqv` file (sparse by
design). Query top-`k` by cosine with an exact brute-force scan or a persistent on-disk
DiskANN/Vamana graph by default, or an in-RAM HNSW index behind the opt-in `approx-ann`
feature (the only third-party ANN dependency; **recall < 1.0**). Embeddings are produced
**outside** the engine; the crate verbalizes entities to text, embeds via a provider-agnostic
trait, and fuses with another ranked signal for hybrid search. It is a **separate crate** —
nothing in the workspace (or the wasm build) depends on it.

## 🚀 Quickstart

```rust
# use oxrdf::{NamedNode, Term};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let ttl = "<http://ex/a> <http://www.w3.org/2000/01/rdf-schema#label> \"Alpha\" .";
# let some_term: Term = NamedNode::new("http://ex/a")?.into();
# let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
# let path = std::env::temp_dir()
#     .join(format!("sparq-vectors-quickstart-{}-{}.spqv", std::process::id(), nanos));
use sparq_core::Graph;
use sparq_vectors::{embed_labels, nearest_term_exact, HashEmbedder, VectorStore};

let graph = Graph::load_str(ttl, "turtle")?;
let embedder = HashEmbedder::new(64);              // test-only; bring your own Embedder
let mut store = VectorStore::create(&path, 64)?;
embed_labels(&graph, &mut store, &embedder)?;       // rdfs:label > skos:prefLabel > …
store.finalize()?;                                  // handle becomes mmap-backed

// Exact brute-force search — the default build (no third-party ANN crate).
let _neighbours = nearest_term_exact(&store, &graph, &some_term, 10);
# drop(store); std::fs::remove_file(&path).ok();
# Ok(()) }
```

## ✨ Features

- **`VectorStore`** — memory-mapped `.spqv` (native); `get` is one binary search + a contiguous
  `&[f32]`; corrupt files rejected up front. `StreamingWriter` builds stores bigger than RAM in
  O(1) memory, byte-identical to the in-RAM builder. **Compiles on wasm32** (sq-98c): memmap2 is
  target-gated out; `open_from_bytes` (`.spqv` and `.spqg` alike) is the filesystem-less read path.
- **Exact vs approximate search** — `nearest_exact` (answer-exact ground truth) and the
  persistent on-disk `DiskAnnIndex` in the default build; the in-RAM HNSW `VectorIndex`
  behind the **opt-in `approx-ann`** feature, the **only** third-party-ANN dependency
  (`instant-distance`). `VectorIndex::nearest_with_ef(q, k, ef)` sweeps `ef_search` at
  query time (recall–QPS Pareto; monotone-non-decreasing recall as `ef` grows). The HNSW distance
  kernel is **explicit SIMD** (runtime-detected NEON / AVX2+FMA, scalar fallback, no new dependency —
  cuts build time + lifts QPS; recall **floor-preserved**, not bit-identical: FMA differs ≤1 ULP, absorbed by the recall floor gate). `HnswConfig::fast_build()` / `high_recall()` trade `ef_construction` for build speed vs recall (pure config, default unchanged, floor-preserved — sq-ose80). **Approximate search has recall `< 1.0`** (measured against `nearest_exact`) — only the exact path is answer-exact. The same feature carries the **recall-gated concept dedup** surface (`build_ann`/`knn`/`dedup` over raw `(id, vector)` pairs, no store needed): near-duplicate merges apply **only after** measured ANN recall vs an exact O(m²) ground truth clears a pre-registered gate (default 0.99, fail-closed — issue #2251, the Kernel-of-Truth scale track).
- **Predicate-constrained ANN (opt-in `filtered-ann`)** — restrict the search to the
  dict-ids a SPARQL BGP admits via an `IdMask` (lean, no new dependency). Composed with
  `approx-ann`, filtered over-fetch fills `k` whenever `k` admitted vectors exist *and the
  backend surfaces them* — **honest caveat:** over-fetch fixes *under-fill*, not the
  approximate index's inherent misses (recall stays `< 1.0`, `tests/overfetch.rs`).
- **k-NN inside SPARQL (opt-in `vec-predicate`)** — `vec:nearest` / `vec:search` magic
  predicates via a spargebra-algebra rewrite (the engine is unchanged). A constrained
  neighbour variable is searched as a *filtered* ANN over its join-connected sub-BGP,
  with the derived `IdMask` cached by sub-BGP + graph fingerprint (any graph change
  misses the cache). A per-query cost model picks pre- vs post-filter; both return the
  **byte-identical** top-k. The same feature carries **hybrid retrieval + reranking**: the *provisional* `vec:hybrid` predicate (`( ?node ?score ?rank ?prov ) vec:hybrid ( <query> <k> )`) fuses the dense k-NN with caller-supplied arms (lexical, structural, …) by deterministic weighted RRF, binds the per-arm **rank provenance** (`"vector=1;text=3;rerank=2"`), and optionally runs an out-of-process `Reranker` under an explicit fail-open / fail-closed policy. Arms and reranker are caller closures/traits, so the crate depends on none of them. **No lift is claimed:** whether fusion or reranking helps is an empirical, per-corpus question measured by `hybrid::ablate` (dense / sparse / structural / fused / reranked rows); the pinned-corpus run is not in-tree.
- **Graph-staleness guard** — `.spqv`/`.spqg` headers embed a dictionary **fingerprint**
  (thread-count-stable); the checked query paths reject a mismatch instead of serving wrong
  neighbours. **Id-keyed staleness contract (sq-wlzi):** a passing `check_graph` is **necessary but
  not sufficient** — to serve a persisted store, `Graph::save` its graph and reopen **that**
  (`Graph::open`, frozen id order); **never** re-parse the source RDF (`tests/staleness_contract.rs`).
- **Embedding provenance + mandatory rejection (`.spqv` v3, opt-in `spqv-provenance`)** — the v3
  header records the embedding pipeline (model id / model+content version / metric / normalization /
  dimension / verbalization regime); `check_provenance` REJECTS an incompatible query embedder (a
  different space ⇒ semantically wrong neighbours). v3 READ always compiled, the WRITE path opt-in (v2
  stays the default, unchanged); a legacy v2/v1 store fails **closed** unless the caller opts into
  `LegacyMode::Allow`. The extension area is **reserved, opaque, pending the cross-implementation
  profile (#1746)** — no encoder privileged. Closes review gap 1 (spec `sq-rvgr2.4`).
- **Incremental add / remove / update (opt-in `delta`)** — a `.spqv` is build-once-immutable; the
  `delta` feature adds a **delta sidecar** (`add`/`remove`/`update` → append map + tombstones) that
  `get`/`iter`/search transparently union, generation-tied to the base (sq-32i5); `compact` folds it
  into a from-scratch-equivalent base; `save_delta`/`open_with_delta` persist+replay a crash-durable
  `.spqd`. No new dep.
- **Verbalization, quantization, hybrid fusion** — `verbalize` / `embed_entities` render per-entity
  text (multilingual, char-budgeted); `ScalarQuantizer` (4×) and `ProductQuantizer` for large stores;
  `fuse_rrf` / `hybrid_search` combine vectors with another ranked signal (e.g. [`sparq-sim`](../sparq-sim)).
- **Bring-your-own embeddings** — `import_npy` / `import_numeric_dump` load an externally-computed
  matrix straight into a `.spqv` keyed by dict id (no model in-process, no new dependency).
  **Fail-closed**: any dtype / byte-order / shape / length / non-finite-row mismatch is an `Err`,
  never a silent reinterpretation; the declared `.npy` header is bounded before any body allocation.
- **Live embeddings (opt-in)** — the default build is **socket-free**. The non-default `provider`
  feature carries the OpenAI-compatible `/v1/embeddings` shape with a caller-supplied `Transport`;
  `embeddings` adds a concrete reqwest client + `RemoteEmbedder::from_env(dim)`. Never in the wasm bundle.
- **Structure-aware vectorisation (opt-in `structure` / `structure-shacl`; measurement behind `kge`)** — research-grade. **P0:** `close_for_vectorise` materialises the `sparq-reason` closure **before** vectorising; a `NegativeSampler` emits type-constrained corruptions (Krompass 2015) with an **on/off ablation**. [GPT-5.6] `TermScope::IriBlank` is the default, byte-stable entity scope; opt into `TermScope::Embeddable` with `NegativeSampler::new_scoped` to admit RDF 1.2 triple terms while keeping atomic and triple-term corruption pools separate.
  **P1/P2:** typed-literal encoders — `route`r, **order-preserving** `NumericEncoder`, `BooleanEncoder`, `DateEncoder`, enum `Codebook`, `SchemaHeader` (metric guard); QUDT unit-`normalise` (`1000 m` ≡ `1 km`); **`structure-shacl`** adds the `ShaclPriors` reader (enum/datatype/cardinality from `sparq-shacl`).
  **P3:** `TaxonomyDag` + `EuclideanTaxonomyEncoder` (Euclidean default; hyperbolic **only past** the measured-distortion `GeometryGate`) + an **answer-safe** `DisjointnessOracle` (train-time repulsion + serve-time hard mask dropping *provably-disjoint* candidates only).
  **P4:** `ground` — a per-request modality dispatcher (subgraph / typed sub-vector / NL / typed value), **profile-relative** completeness + ABSTAT-style minimality; ambiguous → the exact subgraph; opt-in `reconcile_units` canonicalises **known** commensurable units (`1 mi`≡`1.609344 km`), unknown/compound units stay as-declared. **P5 (`neuro-symbolic`):** `propose_then_verify` — vectors **propose** candidate bindings (recall only, NOT sound), then a **deductive gate** admits one only if it adds no new SHACL violation (`sparq-shacl`) **and** no new OWL inconsistency (`sparq-reason`); a failing candidate is **rejected** (fail-closed), so the verified set only ever **shrinks to a sound subset**.
  **Provenance-weighting (GenAI-KB Phase 4):** `ProvenanceWeights` mines a per-triple `w(t)∈(0,1]` from PROV-O/DQV (`pkg:confidence`/`pkg:assurance`/`prov:wasDerivedFrom`, mirroring `ShaclPriors`, **no engine dep**), threaded into three points behind `WeightMode` on/off ablation (**fail-open 1.0** on plain graphs): (1) CKRL down-weighting in the `kge` trainer; (2) `pool_weighted` — confidence-weighted **characteristic-set pooling** (≡ arithmetic mean under uniform); (3) `block_weight` → `Block::with_weight` — a per-`Block` query-time **fusion weight** consumed by `fuse_rrf_weighted`/`fuse_scores` (`.spqv` `SchemaHeader` v2; v1 sidecars still parse).
  **`kge`** (implies `structure`, no new dep — hand-rolled SGD): a thin CPU-only trainer (symmetric **DistMult** / asymmetric **ComplEx**) + a **filtered link-prediction** ablation (`run_ablation*`, `run_weight_ablation`, `run_quoted_ablation`). `TrainConfig::term_scope` defaults to `TermScope::IriBlank`; the dedicated RDF 1.2 slice (`synthetic_rdf12_ttl`) measures the opt-in triple-term arm without widening the atomic ranking pool; `TrainedModel::encode_quoted_term` derives a **compositional statement-level** encoding of a triple term's `(s,p,o)` (interaction-space, no new parameters; the node-level row path is unchanged). **No accuracy claim**; INDICATIVE only — read deltas off ComplEx (DistMult is symmetric → near-random on directional data).

## 📚 Learn more

- **How-to** — [`skills/vector-search/SKILL.md`](../../skills/vector-search/SKILL.md) (label /
  verbalized / hybrid pipelines, DiskANN, quantization, bulk import, API surface, `.spqv`/`.spqg`).
- **API reference** — [docs.rs/sparq-vectors](https://docs.rs/sparq-vectors); **design** —
  [`research/genai-text-embedding-practices.md`](../../research/genai-text-embedding-practices.md).
- **Accuracy & throughput** — not baked into docs; the recall / DiskANN / PQ / throughput gates are
  `cargo test`s, with live numbers on the [benchmarks dashboard](https://sparq.jeswr.org/dev/bench).
- **Verified against an established ANN library (sq-6te5)** — `tests/ref_lib_verify.rs` anchors
  recall against **hnswlib** via a committed capture (`tests/fixtures/hnswlib_ref.tsv`):
  `nearest_exact` reproduces the numpy exact-kNN oracle, and DiskANN / HNSW clear hnswlib's recall.
  Runs in CI with no native deps; the live re-capture is `#[ignore]`d. Recall figures NON-CANONICAL.
- **Contribute** — [`AGENTS.md`](../../AGENTS.md) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
