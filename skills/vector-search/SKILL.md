---
name: vector-search
description: "Semantic / ANN vector search over a sparq RDF graph: build a memory-mapped per-term-id embedding store (.spqv), then run cosine top-k with an in-RAM HNSW, a persistent on-disk DiskANN/Vamana graph (.spqg), or an exact brute-force baseline; verbalize entities (label+type+description) for embedding, scalar/product quantize (SQ/PQ) for large stores, fuse with another ranked signal (RRF / score blend) for hybrid retrieval, run predicate-constrained (filtered) ANN over a BGP-selected dict-id mask behind the opt-in `filtered-ann` feature, run recall-gated concept dedup + k-NN over a RAW (id, vector) matrix behind the opt-in `approx-ann` feature (build_ann/knn/dedup: merges apply only after measured ANN recall vs an exact ground truth clears a pre-registered gate), and — behind the opt-in `vec-predicate` feature — run k-NN INSIDE plain SPARQL via the `vec:nearest` / `vec:search` magic predicates. Use when adding embedding/semantic-search/nearest-neighbour/near-duplicate-merging over a sparq Graph or a raw concept-vector matrix in the sparq-vectors crate."
---

# sparq-vector-search

`sparq-vectors` is an **opt-in** crate that adds embedding storage + nearest-neighbour
(ANN) search to the sparq RDF engine. You store one f32 embedding per dictionary **term
id** in a flat memory-mapped `.spqv` file, then query top-`k` by **cosine** with an exact
scan, an in-RAM HNSW index, or a persistent on-disk DiskANN/Vamana graph — all
cosine-identical so their scores are directly comparable. Embeddings are produced
**out-of-process** (you supply the `Embedder`); the crate never runs a model and the
default engine build does not even compile it.

## Quickstart

`crates/sparq-vectors/Cargo.toml` (it consumes `sparq-core`; no features needed for the
core flow):

```toml
[dependencies]
sparq-core = { path = "../sparq-core" }
sparq-vectors = { path = "../sparq-vectors" }
oxrdf = "*"   # for oxrdf::{NamedNode, Term} in term-keyed queries
```

Embed entity labels, finalize the store, query the nearest neighbours of a term:

```rust
use sparq_core::Graph;
use sparq_vectors::{embed_labels, nearest_term_exact, HashEmbedder, VectorStore};
use oxrdf::{NamedNode, Term};

# fn main() -> Result<(), String> {
let ttl = r#"
  @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
  @prefix ex:   <http://example.org/> .
  ex:bolt  rdfs:label "Usain Bolt" ; a ex:Athlete .
  ex:bolt2 rdfs:label "Usain Bolt Junior" ; a ex:Athlete .
"#;
let graph = Graph::load_str(ttl, "turtle")?;

let embedder = HashEmbedder::new(64);             // TEST-ONLY embedder (see Gotchas)
let mut store = VectorStore::create("graph.spqv", 64)?; // dim must match embedder.dim()
embed_labels(&graph, &mut store, &embedder)?;     // rdfs:label > skos:prefLabel > foaf:name > schema:name > dc:title
store.finalize()?;                                // writes the file, handle becomes mmap-backed

// Exact brute-force search by term (default build — no third-party ANN dep):
let bolt = Term::NamedNode(NamedNode::new("http://example.org/bolt").map_err(|e| e.to_string())?);
let neighbours: Vec<(Term, f32)> = nearest_term_exact(&store, &graph, &bolt, 10); // (Term, cosine), best first, query term excluded
// For the APPROXIMATE in-RAM HNSW at scale, enable the opt-in `approx-ann` feature and build a
// `VectorIndex` (recall < 1.0; see "Exact vs HNSW" below) — gated so the heavy crate stays out of
// the default build.
# Ok(()) }
```

## Key APIs

```rust
// --- store (src/store.rs) ---  Id = sparq_core::dict::Id (= u32)
VectorStore::create<P: Into<PathBuf>>(path, dim: usize) -> Result<VectorStore, String>
VectorStore::open<P: AsRef<Path>>(path) -> Result<VectorStore, String>          // mmap; validates header+index up front
VectorStore::open_from_bytes(bytes: Vec<u8>) -> Result<VectorStore, String>     // filesystem-less
impl VectorStore {
    fn put(&mut self, id: Id, vector: &[f32]) -> Result<(), String>             // build phase only; rejects all-zero / non-finite / dup id / dim mismatch
    fn finalize(&mut self) -> Result<(), String>                                // write + reopen mmap; idempotent
    fn get(&self, id: Id) -> Option<&[f32]>;  fn iter(&self) -> impl Iterator<Item=(Id, &[f32])>
    fn dim(&self) -> usize;  fn len(&self) -> usize;  fn is_empty(&self) -> bool
}
# feature = "metadata-sidecar": opaque per-vector tags persisted in `.spqv` v4; no new dependency
store.put_with_meta(id, vector, meta: &str) -> Result<(), String>               // build phase; exact UTF-8 bytes retained
store.meta(id) -> Option<&str>                                                  // None for untagged or absent ids
StreamingWriter::create(path, dim);  fn put(&mut self, id, &[f32]); fn finalize(self) -> Result<VectorStore, String>  // O(1) build RAM, byte-identical output

// --- bulk import of externally-computed embeddings (src/import.rs) --- bring-your-own vectors; reuses the store writer
ImportSpec { spqv_path, dim, ids: &[Id], binding: ImportBinding }   // ids[i] keys row i (row -> dict-id contract); ids.len() = rows
ImportBinding::Unbound | ImportBinding::Graph(&Graph)              // graph binding for the fingerprint header (Unbound = unverifiable)
VectorStore::import_npy(ImportSpec, npy_path) -> Result<VectorStore, String>          // 2-D C-order little-endian f4/f8 .npy; f8 narrowed to f32
VectorStore::import_numeric_dump(ImportSpec, dump_path) -> Result<VectorStore, String> // header-less flat rows*dim*4 LE f32 (row-major)
// both: fail-closed on dim/row-count/dtype/order/length mismatch; .npy header bounded (sq-tzwa) before any body alloc; NOT on wasm (std::fs)

// --- embedders (src/embed.rs) ---
trait Embedder { fn dim(&self) -> usize; fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String>; }
HashEmbedder::new(dim) / ::with_seed(dim, seed)                                 // TEST-ONLY lexical hashing, NO semantics
// feature = "provider": provider::{Transport, ProviderConfig, RemoteEmbedder<T: Transport>}  (caller supplies HTTP)

// --- text → vectors (src/labels.rs, src/verbalize.rs) ---
embed_labels(&Graph, &mut VectorStore, &impl Embedder) -> Result<usize, String> // store left UNFINALIZED
embed_labels_with(&Graph, &mut VectorStore, &impl Embedder, &LabelConfig) -> Result<usize, String>
verbalize(&Graph, &Term, &EntityTextConfig) -> Option<String>                   // inspect the passage before embedding
embed_entities(&Graph, &mut VectorStore, &impl Embedder, &EntityTextConfig) -> Result<usize, String>
// EntityTextConfig{ groups: Vec<PropertyGroup>, languages, naming_predicates, separator, max_chars, batch }
// PropertyGroup::literal(preds) / ::entity_label(preds) .with_prefix("a ") .with_max_values(3); ObjectKind::{Literal, EntityLabel}

// --- graph-staleness guard (src/fingerprint.rs) --- store is keyed by dict id; a rebuild can shift ids
VectorStore::create(p, dim)?.with_fingerprint(&Graph)   // bind store to its graph; finalize embeds it in the .spqv header
StreamingWriter::create_with_fingerprint(p, dim, &Graph) // same for the streaming builder
store.fingerprint() -> Option<Fingerprint>;  store.check_graph(&Graph) -> Result<(), String>   // None / mismatch -> Err
// fingerprint = dict_len + triple_count + content_hash over the dict term SET in a dict-id-order-INDEPENDENT (sorted) order
//   (sq-xhiv: stable across RAYON_NUM_THREADS, so re-loading the same RDF at a different thread count does NOT spuriously
//   mismatch; a genuine term add/remove/edit still does); legacy v1 files open but check_graph errs
// ID-KEYED STALENESS CONTRACT (sq-wlzi): the store is keyed by RAW dict id, so a passing check_graph is NECESSARY but NOT
//   SUFFICIENT -- the thread-count-stable fingerprint ALSO passes a re-parse of the same RDF whose ids merely permuted, which
//   then serves the WRONG vector. To SERVE a persisted .spqv/.spqg: persist its graph (Graph::save) and reopen THAT graph
//   (Graph::open -- mmaps the FROZEN id order) to resolve query terms; NEVER re-parse the source RDF (Graph::load_str etc.).
//   (Graph::save/open need sparq-core's `mmap` feature.) Round-trip vs re-parse trap pinned in tests/staleness_contract.rs.

// --- embedding provenance (.spqv v3) (src/spqv_provenance.rs) --- [FABLE-5] sq-lhcot.1 (review gap 1; spec sq-rvgr2.4)
// The v3 header records the EMBEDDING PIPELINE identity so an INCOMPATIBLE query embedder is REJECTED (not silently-wrong
// neighbours). v3 READ path always compiled; the v3 WRITE path is behind the opt-in `spqv-provenance` feature (LEAN, no new dep).
EmbeddingProvenance { model_id, model_version, content_version, metric: EmbeddingMetric, normalization: Normalization,
                      verbalization, reserved: Vec<u8> }   // string axes are OPAQUE caller tokens (no encoder privileged)
EmbeddingProvenance::new(model_id, EmbeddingMetric, Normalization)   // other axes empty; set fields directly (NOT Default — metric/norm load-bearing)
EmbeddingMetric::{Cosine, Dot, Euclidean};  Normalization::{None, L2}   // typed axes; from_tag() fail-closed on an unknown tag
prov.compatible_with(&query_prov) -> Result<(), String>   // compatible IFF every DEFINED axis equal; reserved area EXCLUDED (KERN boundary)
prov.to_rdf(store: NamedNodeRef, dim) -> Vec<Triple>   // [SONNET-4.6] sq-tb9p0 VG-PROV-5: the record as RDF in prov_vocab
//   (spqvp:) terms — model/metric/normalization/dimension always; version/verbalization axes only when non-empty
// KERN BOUNDARY: `reserved` is a versioned OPAQUE TLV — extension fields RESERVED pending the cross-implementation profile (#1746).
//   NO encoder-version-hash / codebook-hash / D semantics defined; it round-trips byte-for-byte and does NOT gate compatibility.
// v3 WRITE (feature = "spqv-provenance" ONLY): binding a provenance selects the v3 path; else the writer emits v2 (unchanged).
VectorStore::create(p, dim)?.with_provenance(EmbeddingProvenance)   // finalize writes v3 with the provenance in the header
StreamingWriter::create_with_provenance(p, dim, &Graph, EmbeddingProvenance)   // streaming v3 (also binds the graph fingerprint)
// v3 READ + the mandatory check (ALWAYS compiled — a v3 file opens on a feature-OFF build; the demanding check is always available):
store.provenance() -> Option<&EmbeddingProvenance>   // Some for a v3 file; None for a legacy v1/v2 file (predate embedding provenance)
store.check_provenance(&query_prov, LegacyMode) -> Result<(), String>   // REJECTS an incompatible query embedder (names each mismatched axis)
LegacyMode::{Reject, Allow}   // Reject (DEFAULT, fail-closed): a legacy no-provenance store is REJECTED; Allow: bypass for a LEGACY store ONLY
//   A v3 store is ALWAYS checked against its recorded provenance regardless of LegacyMode. Dimension is enforced structurally (wrong-width put/search errs).
//   RDF vocab: prov_vocab::{SPQVP_NS, MODEL, MODEL_VERSION, CONTENT_VERSION, METRIC, NORMALIZATION, DIMENSION, VERBALIZATION} (http://sparq.dev/spqv-prov#)
//   `compact` (delta feature) carries the provenance forward (a v3 store stays v3); SPQV_VERSION_V3 = 3. NEGATIVE tests: tests/spqv_v3_provenance.rs.

// --- incremental add/remove/update (src/delta.rs) --- feature = "delta" ONLY; LEAN, no new dep [OPUS-4.8] sq-pi44
// In-RAM DELTA SIDECAR over the immutable base: append map + tombstone set. No file rebuild; a single graph change no
// longer forces a full re-embed. get/iter/len (hence search) transparently union base+delta and honour tombstones.
store.add(id, &[f32]) -> Result<(), String>      // NEW id; errs if id already present (use update) or on the put validation
store.remove(id) -> bool                          // tombstone id (true if it had a vector); a removed id can be re-added
store.update(id, &[f32]) -> Result<(), String>   // replace an EXISTING id's vector; errs if absent (use add)
store.compact(out_path, &Graph) -> Result<VectorStore, String>  // fold delta into a FRESH base == a from-scratch rebuild
store.has_delta() -> bool;  store.delta() -> Option<&VectorDelta>;  store.take_delta() -> Option<VectorDelta>
store.apply_delta(VectorDelta) -> Result<(), String>  // GENERATION TIE: rejects a delta whose fingerprint != this base's
// --- PERSISTED on-disk delta sidecar (.spqd) --- [OPUS-4.8] sq-7e50; same `delta` feature, no new dep
store.save_delta() -> Result<PathBuf, String>          // persist the in-RAM delta to a sibling .spqd (tmp+fsync+rename)
store.save_delta_to(path) -> Result<(), String>        // …to an explicit path; an unstarted delta persists empty+gen-bound
VectorStore::open_with_delta(base) -> Result<VectorStore, String>   // open base mmap + replay the persisted sibling .spqd
VectorStore::open_with_delta_at(base, delta_path) -> Result<..>     // …from an explicit delta path (no sidecar ⇒ bare base)
VectorStore::sibling_delta_path(&Path) -> PathBuf;  VectorStore::has_persisted_delta(&Path) -> bool
// `.spqd` = little-endian header (magic SPQD, v1, dim, append/tomb counts, BASE fingerprint @ off 32) + appends + tombstones.
// CRASH-DURABLE (tmp+fsync+atomic rename, StreamingWriter discipline); PARTIAL-WRITE detected (exact-length check, fail-closed);
// open routes through apply_delta so a sidecar from a DIFFERENT graph generation is REJECTED, never silently mis-keyed.
// `put` is UNCHANGED (still errs after finalize) — `add` is the additive path; `compact` is the in-base durability path,
//   `save_delta`/`open_with_delta` the survive-a-restart-without-compact path. On a build-phase store add==put.

// --- search (src/ann.rs) --- all return cosine in [-1,1], best first; zero query -> empty
nearest_exact(&VectorStore, query: &[f32], k) -> Vec<(Id, f32)>                 // ground-truth full scan; ascending-id ties
nearest_exact_tiebreak(&VectorStore, &Graph, query: &[f32], k, exclude: Option<Id>) -> Result<Vec<(Id, f32)>, String>  // [SONNET-4.6] sq-tb9p0
//   VG-TIE-1 (spec site/specs/sparql-vector-genai.typ): membership at a BOUNDARY score tie is decided by ascending
//   Unicode-codepoint order of the candidates' canonical N-Triples serialisations (reproducible ACROSS implementations,
//   unlike id order); keys computed only for the boundary tie group. FAIL-CLOSED domain guard: a candidate containing a
//   blank node (document-local label => no stable key) is Err wherever its TERM (not score alone) decides membership —
//   admitted top-k or boundary tie group; IRIs, literals and GROUND triple terms rank. `exclude` = seed-self-exclusion.
//   Used by the answer-exact `vec:` mainline — exact unfiltered path AND (via nearest_filtered_costed_tiebreak,
//   `filtered-ann`) the filtered path over the mask-admitted pool; approximate backends keep plain search (no true boundary).
# feature = "metadata-sidecar": same ranking/scores, decorated after ranking
nearest_exact_with_meta(&VectorStore, query: &[f32], k) -> Vec<(Id, f32, Option<String>)>
nearest_term_exact(&VectorStore, &Graph, &Term, k) -> Vec<(Term, f32)>          // UNCHECKED: stale store -> silently wrong
nearest_term_exact_checked(&VectorStore, &Graph, &Term, k) -> Result<Vec<(Term, f32)>, String>  // errs on stale store
cosine(a: &[f32], b: &[f32]) -> f32
// HNSW = the APPROXIMATE backend: feature = "approx-ann" ONLY [OPUS-4.8] sq-ip3a (the ONLY thing
// pulling instant-distance; default build has NO third-party ANN dep). recall < 1.0 — NOT exact.
VectorIndex::build(&store) / ::build_with(&store, HnswConfig{ef_search, ef_construction, seed})
// [OPUS-4.8] sq-ose80: HnswConfig::fast_build() (efc=40, ~3x faster build) / ::high_recall() (efc=200) — pure config, default unchanged, recall floor-preserved
HnswConfig::default() / ::fast_build() / ::high_recall()
impl VectorIndex { fn nearest(&self, query: &[f32], k) -> Vec<(Id, f32)>;
                   fn nearest_with_ef(&self, query: &[f32], k, ef_search: usize) -> Vec<(Id, f32)>;  // [SONNET-4.6] sq-jo6ty: per-query ef_search sweep (Pareto API)
                   // nearest_with_ef: when ef==build_ef_search uses the primary map (zero overhead); other ef values
                   // trigger a lazy one-time build of a secondary map (same ef_construction/seed/points, only ef_search
                   // differs) cached by ef level — amortised for sweeps. Monotone-recall: higher ef >= lower ef recall.
                   fn nearest_term(&self, &Term, &Graph, &VectorStore, k) -> Vec<(Term, f32)>;
                   fn nearest_term_checked(..) -> Result<Vec<(Term, f32)>, String> }

// --- recall-gated concept ANN + dedup (src/dedup.rs) --- feature = "approx-ann" ONLY [FABLE-5] #2251
// HNSW over RAW (id, vector) pairs (no VectorStore needed) + type-level dedup whose merges are
// emitted ONLY after measured ANN recall vs an exact O(m^2) ground truth clears a pre-registered
// gate (fail-closed: below the gate dedup() is Err and NO merge is computed). Recipe 22.
build_ann(vectors: &[(Id, Vec<f32>)], policy: HnswConfig) -> Result<ConceptAnnIndex, String>  // fail-closed input validation
knn(&ConceptAnnIndex, query: &[f32], k) -> Vec<(Id, f32)>                        // free fn == index.knn(); APPROXIMATE, recall < 1.0
impl ConceptAnnIndex { fn knn(&self, query: &[f32], k) -> Vec<(Id, f32)>; fn knn_of(&self, Id, k) -> Vec<(Id, f32)>;  // by indexed id, self excluded
                       fn len()/is_empty()/dim(); fn ids() -> &[Id] }
exact_ground_truth(vectors, k) -> Result<GroundTruth, String>                    // the exact O(m^2) oracle; build ONCE at a tractable rung
GroundTruth::new(k, Vec<(Id, Vec<Id>)>) -> Result<GroundTruth, String>           // or wrap an externally-computed oracle; fn k(); fn neighbors()
DedupPolicy { recall_gate: f64 /*0.99*/, merge_threshold: f32 /*0.995, NON-canonical default*/, k /*10*/ }  // FREEZE one per scale track
dedup(&ConceptAnnIndex, &DedupPolicy, &GroundTruth) -> Result<DedupReport, String>   // gate FIRST, merges only after it passes
DedupReport { recall: f64, merges: Vec<(Id /*dup*/, Id /*canonical=smallest*/)>, groups: Vec<Vec<Id>> }

// --- persistent on-disk ANN (src/diskann.rs) ---
DiskAnnIndex::build(&VectorStore, path) / ::build_with(&store, path, VamanaConfig{degree, build_beam, search_beam, alpha, seed})
DiskAnnIndex::build_for(&store, path, &Graph) / ::build_with_for(&store, path, cfg, &Graph)  // embeds the graph fingerprint
DiskAnnIndex::build_with_pq(&store, path, cfg, PqConfig) -> Result<..>          // [OPUS-4.8] sq-qamd: + a PQ candidate cache (search on codes, re-rank off mmap); persisted as a trailing .spqg section (encoding tag 1)
DiskAnnIndex::open(path) -> Result<DiskAnnIndex, String>                        // mmap + header check, NO rebuild (reloads any PQ section)
DiskAnnIndex::open_from_bytes(bytes: Vec<u8>) -> Result<DiskAnnIndex, String>   // [FABLE-5] sq-98c: filesystem-less/wasm — identical validation, result-identical search
impl DiskAnnIndex { fn nearest(&self, &[f32], k) -> Vec<(Id, f32)>; fn nearest_term(..) -> Vec<(Term, f32)>; fn len()/dim();
                    fn has_pq_cache() -> bool;                                  // [OPUS-4.8] sq-qamd: PQ-guided search when true
                    fn fingerprint() -> Option<Fingerprint>; fn check_graph(&store, &Graph) -> Result<(), String>;
                    fn nearest_term_checked(&Term, &Graph, &store, k) -> Result<Vec<(Term, f32)>, String> }
sibling_graph_path(&Path) -> PathBuf                                            // foo.spqv -> foo.spqg

// --- predicate-constrained (filtered) ANN (src/filter.rs) --- feature = "filtered-ann" ONLY; LEAN, no new dep [OPUS-4.8] sq-1wc1
IdMask::new() / ::from_ids(impl IntoIterator<Item=Id>) / FromIterator<Id>   // the BGP-selected "visit mask" of permitted dict-ids
impl IdMask { fn insert(&mut self, Id) -> &mut Self; fn contains(Id)->bool; fn len()/is_empty(); fn iter() -> impl Iterator<Item=Id> }
nearest_exact_filtered(&VectorStore, query: &[f32], &IdMask, k) -> Vec<(Id, f32)>   // EXACT ground truth = pre-filter strategy (scan only the mask)
FilterConfig { prefilter_fraction: f32 /*0.01*/, prefilter_floor: usize /*256*/, traversal_beam_factor: usize /*4*/ }
impl FilterConfig { fn prefer_prefilter(mask_len, store_len) -> bool }       // selectivity crossover: <= max(frac*store, floor) -> pre-filter
impl DiskAnnIndex { fn nearest_filtered(&self, &[f32], &IdMask, &VectorStore, k) -> Vec<(Id, f32)>     // ACORN/NaviX filtered traversal OR pre-filter, by selectivity
                    fn nearest_filtered_with(&self, &[f32], &IdMask, &VectorStore, k, FilterConfig) -> Vec<(Id, f32)> }
impl VectorIndex  { fn nearest_filtered(&self, &[f32], &IdMask, &VectorStore, k) -> Vec<(Id, f32)> }   // HNSW: exact pre-filter only (instant-distance adjacency not exposed)
// empty mask -> no results; full mask -> equals the unfiltered search; every returned id is guaranteed in the mask

// --- pre-filter vs post-filter cost model (src/cost.rs) --- feature = "filtered-ann" ONLY [OPUS-4.8] sq-7hx6 (subsumes sq-ic0n)
CostModel { scatter_penalty: f32 /*2.0*/ }                                       // scattered masked-row cost vs sequential full-scan row; >= 1.0
impl CostModel { fn decide(mask_len, store_len, k) -> CostEstimate }             // pre-filter iff mask_len * scatter_penalty <= store_len
Strategy::{PreFilter, PostFilter}                                                // the chosen branch (assert the decision)
CostEstimate { mask_len, store_len, k, prefilter_cost, postfilter_cost, strategy }   // the modelled estimate behind a decision
postfilter_exact(&VectorStore, query: &[f32], &IdMask, k) -> Vec<(Id, f32)>      // scan WHOLE store, drop non-masked -> IDENTICAL to nearest_exact_filtered (no over-fetch boundary: full ranking)
nearest_filtered_costed(&VectorStore, &[f32], &IdMask, k, &CostModel) -> (Vec<(Id, f32)>, CostEstimate)   // decide + run chosen branch; ascending-id ties
nearest_filtered_costed_tiebreak(&VectorStore, &Graph, &[f32], &IdMask, k, exclude: Option<Id>, &CostModel) -> Result<(Vec<(Id, f32)>, CostEstimate), String>
//   [SONNET-4.6] the same decide+run with VG-TIE-1 boundary-tie membership over the mask-ADMITTED pool, `exclude` (seed)
//   dropped BEFORE the boundary is determined — what the filtered `vec:` rewrite path calls (keeps VG-FILT-2 exact);
//   Err = the same fail-closed blank-node domain guard as nearest_exact_tiebreak
overfetch_target(k, mask_len, store_len) -> usize                               // ceil(k/selectivity) clamped; the FIRST fetch size for the iterative over-fetch path below (exact backend never under-fills, so it's a no-op there)
// HEURISTIC over an ESTIMATE, not optimal: scatter_penalty is one modelled constant; pre/post return the IDENTICAL top-k either way (answer-safe)

// --- pluggable ANN backend + iterative over-fetch FILTERED path (src/backend.rs) --- feature = "filtered-ann" [OPUS-4.8] sq-ip3a (follow-up to sq-7hx6)
trait AnnBackend { fn candidates(&self, query: &[f32], fetch) -> Vec<(Id, f32)>; fn len()/is_empty(); }  // ranked top-`fetch`, best-first, prefix-stable
ExactBackend::new(&VectorStore)                                                  // answer-EXACT (full scan); fetch>=len => complete ranking => recall 1.0; NO third-party dep
ApproxBackend::new(&DiskAnnIndex)                                                // feature = "approx-ann" ALSO; APPROXIMATE (bounded beam) => recall < 1.0 — NOT exact
nearest_filtered_overfetch(&impl AnnBackend, query, &IdMask, k, max_rounds) -> Vec<(Id, f32)>           // fetch overfetch_target, post-filter, if <k DOUBLE the fetch & retry up to backend size / max_rounds
nearest_filtered_overfetch_default(&backend, query, &IdMask, k) -> Vec<(Id, f32)>                       // = ..._overfetch(.., DEFAULT_MAX_ROUNDS=24)
// FILTERED approx under-fills a bounded list when admitted ids cluster below the prefix; iterative over-fetch FILLS k whenever k admitted vectors exist & the backend surfaces them.
// HONESTY: over-fetch fixes UNDER-FILL, NOT recall — ApproxBackend recall stays < 1.0 (measured floor 0.90@10 in tests/overfetch.rs, NOT exactness). Only ExactBackend is answer-exact.

// --- quantization for large stores (src/quant.rs) ---
ScalarQuantizer::fit(dim, vectors: impl IntoIterator<Item=&[f32]>) -> Result<ScalarQuantizer, String>   // f32->u8, 4x
ProductQuantizer::fit(dim, vectors, PqConfig{m, k, iters, seed}) -> Result<ProductQuantizer, String>    // M bytes/vec, 8-32x
ProductQuantizer::to_bytes() -> Vec<u8> / ::from_bytes(&[u8]) -> Result<ProductQuantizer, String>       // [OPUS-4.8] sq-qamd: persist/reload the codebook (e.g. in a .spqg PQ section)
impl {Scalar,Product}Quantizer { fn encode(&self, &[f32]) -> Vec<u8>; fn reconstruct(&self, &[u8]) -> Vec<f32>;
                                  fn encode_store(&self, &VectorStore) -> Result<EncodedStore, String> }
DistanceTable::new(&ProductQuantizer, query: &[f32]);  fn distance(&self, code)->f32; fn cosine(&self, code)->f32  // ADC
EncodedStore::rank_pq(&self, &DistanceTable, k) -> Vec<(Id, f32)>;  cosine_from_sq_dist(sq: f32) -> f32
EncodedStore::from_parts(ids, codes, stride) -> Result<EncodedStore, String> / ::codes() -> &[u8]       // [OPUS-4.8] sq-qamd: reload a persisted candidate cache

// --- hybrid fusion (src/fuse.rs) --- lists are (item, f64) best-first; deterministic ties
fuse_rrf(lists: &[&[(T, f64)]], k: f64 /*RRF_K=60.0*/, top_k) -> Vec<(T, f64)>
fuse_rrf_weighted(lists: &[(&[(T, f64)], f64)], k, top_k) -> Vec<(T, f64)>      // weight 0.0 mutes a list entirely
fuse_scores(a: &[(T,f64)], b: &[(T,f64)], alpha /*1.0=a only*/, top_k) -> Vec<(T, f64)>
// one-call hybrid: run N retriever closures on one query, fuse by item via RRF, dedup
hybrid_search(query: &Q, top_k, k /*RRF_K*/, &mut [Retriever<'_, Q, T>]) -> Vec<(T, f64)>   // [OPUS-4.8] lifetime on alias use
//   Retriever<'r, Q, T> = &'r mut dyn FnMut(&Q) -> Vec<(T, f64)>  (e.g. nearest_term / most_similar closures)

// --- `vec:` magic predicate (src/rewrite.rs) --- feature = "vec-predicate" ONLY; pulls sparq-engine [OPUS-4.8] sq-k6ex
query_vec(&Graph, sparql: &str, &VectorStore) -> Result<QueryResult, String>      // parse + rewrite + evaluate
query_vec_with_budget(&Graph, &str, &VectorStore, &QueryBudget) -> Result<QueryResult, String>
prepare_vec(&Graph, &str, &VectorStore) -> Result<PreparedQuery, String>          // compose with engine *_prepared entry points
rewrite_query(Query, &Graph, &VectorStore) -> Result<Query, String>              // spargebra-algebra rewrite only
// re-exported when the feature is on: query_prepared, PreparedQuery, QueryBudget, QueryResult (no direct sparq-engine dep needed)
// vocab: vec::{VEC_NS, NEAREST, SEARCH, HYBRID, PROVISIONAL, VOCAB_REVISION=1}  (http://sparq.dev/vec#)  — exact-scan KNN
//   with the VG-TIE-1 boundary tie-break (nearest_exact_tiebreak); the VG-VOC-1 unknown-predicate error reports
//   VOCAB_REVISION (VG-GOV-3). vec:hybrid is PROVISIONAL: implemented ahead of the spec amendment, so it is listed in
//   vocab::PROVISIONAL and does NOT bump VOCAB_REVISION — its shape may change when that spec revision lands.
// [SONNET-4.6] sq-tb9p0 VG-MET-4 (mainline): prepare/query REJECT a store whose v3 provenance declares a NON-cosine
//   metric (the vec: surface evaluates cosine only); a legacy no-provenance store keeps the implicit-cosine behaviour
// [OPUS-4.8] sq-z589: with `approx-ann` ALSO on, the *_approx twins take a &DiskAnnIndex and run the
//   UNFILTERED vec: k-NN through that Vamana index instead of the full scan (APPROXIMATE, recall < 1.0):
query_vec_approx(&Graph, &str, &VectorStore, &DiskAnnIndex) -> Result<QueryResult, String>   // feature = "vec-predicate" + "approx-ann"
query_vec_approx_with_budget(&Graph, &str, &VectorStore, &DiskAnnIndex, &QueryBudget) -> Result<QueryResult, String>
prepare_vec_approx(&Graph, &str, &VectorStore, &DiskAnnIndex) -> Result<PreparedQuery, String>
//   The FILTERED path is unchanged (still cost-model'd nearest_filtered_costed_tiebreak); approx seam = unfiltered scan only.
// [OPUS-4.8] sq-36ol: with `filtered-ann` ALSO on, the BGP→IdMask a constrained `vec:` neighbour
//   derives is CACHED across prepares, keyed by (constraining sub-BGP, graph Fingerprint). The
//   fingerprint folds dict_len + triple_count + a content hash over the dict term SET in a
//   dict-id-order-INDEPENDENT (sorted) order (sq-xhiv), so ANY genuine graph change misses the cache
//   and recomputes — a stale mask is never served (invalidation is SOUND; when in doubt it misses) —
//   while a thread-count-only dict-id permutation of an unchanged graph correctly HITS (same mask).
//   The cache is thread-local and transparent (no API change; same answers).

// --- hybrid retrieval + reranking (src/hybrid.rs + `vec:hybrid`) --- feature = "vec-predicate" ONLY [SONNET-4.6] sq-lhcot.4
// SPARQL surface (subject list is PREFIX-OPTIONAL: ?node | ( ?node ?score ) | ( ?node ?score ?rank ) | + ?prov):
//   ( ?node ?score ?rank ?prov ) vec:hybrid ( <query> <k> )
//   <query> = node IRI | "0.1,0.9" (plain literal = dense query vector) | "machine learning"@en (LANG-TAGGED = text query,
//             needs HybridConfig::query_embedder — a hard error without one, never a silent dense-less fusion)
//   ?score = FINAL-stage score (fused RRF, or the reranker's own score once reranked — different scales)
//   ?rank  = 1-based FINAL rank (xsd:integer). VALUES rows carry no order through joins, so ORDER BY ?rank, not by ?score
//   ?prov  = rank provenance, "vector=1;text=3;rerank=2" (parse_provenance -> Vec<(arm, rank)>)
query_vec_hybrid(&Graph, &str, &VectorStore, &HybridConfig) -> Result<QueryResult, String>      // the ONLY entry points that
query_vec_hybrid_with_budget(&Graph, &str, &VectorStore, &HybridConfig, &QueryBudget) -> ...    //   carry the arms; a
prepare_vec_hybrid(&Graph, &str, &VectorStore, &HybridConfig) -> Result<PreparedQuery, String>  //   vec:hybrid pattern in
rewrite_query_hybrid(Query, &Graph, &VectorStore, &HybridConfig) -> Result<Query, String>       //   plain query_vec ERRORS
// HybridConfig (builder; the DENSE arm is built in under the reserved name VECTOR_ARM="vector" and runs the SAME path
//   vec:search takes — filtered-ann mask + VG-TIE-1 tie-break included):
// [OPUS-4.8] review #4519 — arm results are UNTRUSTED. An id outside the graph dictionary's domain (0, or past
//   dict.len() and not an inline-integer id) is a HARD arm-named query error, never a hit that resolves to the
//   dictionary's out-of-range placeholder term and is then silently dropped from the inlined VALUES table. With
//   `filtered-ann`, EVERY arm's ranking is then restricted to the SAME BGP-derived mask the dense arm searched under
//   (relative order preserved, so ranks compact to 1..n over the ADMISSIBLE candidates). The mask constrains the
//   ANSWER, not just the dense arm: fusion truncates to k BEFORE the surrounding join, so an unrestricted hit in the
//   fused top-k does not merely reorder the result — it evicts a qualifying candidate for good.
HybridConfig::new().arm(name, weight, ArmFn).vector_weight(w /*0.0 mutes -> pure sparse fusion*/)
//   .rrf_k(f64 /*RRF_K=60*/).over_fetch(n /*DEFAULT_OVER_FETCH=4; candidates(k)=k*n*/)
//   .query_embedder(QueryEmbedder).reranker(&dyn Reranker, RerankPolicy::{FailOpen,FailClosed})
//   ArmFn = Box<dyn Fn(&ArmQuery, usize) -> Result<Vec<(Id, f64)>, String>>   // an arm Err is a HARD query error:
//     an arm that prefers availability returns an EMPTY list itself (the policy switch is for the SECOND stage)
// [OPUS-4.8] review #4519 round 2 — PAGING CONTRACT (`filtered-ann`): masking one candidates(k)-long response can only
//   COMPACT the page the arm returned, so an arm whose admissible hits all sit below it would still lose them. When the
//   mask leaves an arm short, the query path RE-ASKS that arm with a doubled count until it has candidates(k) admissible
//   hits, has every admissible id, the arm returns fewer than asked (exhausted), or the request hits the per-request
//   CEILING. So an ArmFn must be prefix-consistent (top-n is a prefix of top-(2n)) and exhaustion-honest (a short answer
//   means "no more"). [round 5] The CORRECTNESS bound is dict.len() + the inline-integer id domain, NOT dict.len(): a
//   ?node constrained in an OBJECT position to a small canonical xsd:integer is admissible at an INLINE id far past
//   dict.len(), so a dictionary-length ceiling could stop paging while such an id was still unreached and lose it.
//   [round 6] That domain is ~1.07e9 wide and each request makes the arm MATERIALIZE a Vec, so the SAFETY cap
//   MAX_ARM_PAGE=65536 wins: escalation stops there (only the caller's own larger candidates(k) page may exceed it) and
//   reaching it is a HARD arm-named error — answering from a ranking whose admissible hits were never reached would be
//   the silent loss the paging exists to prevent. Only a PADDING arm ever sees the cap.
fuse_arms(&[ArmRanking], rrf_k, top_k) -> Result<Vec<FusedHit>, String>   // == fuse_rrf_weighted + per-rank provenance
validate_arms(&[ArmRanking]) -> Result<(), String>   // fail-closed on a lying arm: dup name/id, reserved name, bad weight
apply_rerank(&dyn Reranker, RerankPolicy, &ArmQuery, Vec<FusedHit>, top_k) -> Result<Vec<FusedHit>, String>
//   Reranker::rerank -> Vec<Rescored{index, score}>: may REORDER or DROP, never INVENT. An out-of-range/duplicate index or
//   a non-finite score is malformed and handled like an Err. FailOpen -> first-stage order, and NO row is marked rerank=…
evaluate(&[Id] /*ranked*/, &[Id] /*gold*/, k) -> RetrievalMetrics{k, hits, recall, mrr}
ablate(&[ArmRanking], &ArmQuery, Option<&dyn Reranker>, gold, k, rrf_k) -> Result<Vec<AblationRow>, String>
//   rows = one per arm, then FUSED_ROW, then RERANKED_ROW (fail-closed there — an ablation must not report fused as
//   reranked). It REPORTS: **no lift is claimed** anywhere in this crate; run it on YOUR corpus before quoting a number.
```

## Common recipes

### 1. Verbalize entities (label + type + description), then embed

Better than label-only when the graph has descriptions/types — the default
`EntityTextConfig` renders `"<label>. a <type>. <description>"`:

```rust
use sparq_core::Graph;
use sparq_vectors::{embed_entities, verbalize, EntityTextConfig, HashEmbedder, VectorStore};
use oxrdf::{NamedNode, Term};

let graph = Graph::load_str(ttl, "turtle")?;
let cfg = EntityTextConfig::default();

// Eyeball what would be embedded BEFORE paying a real model:
let bolt = Term::NamedNode(NamedNode::new("http://example.org/bolt").map_err(|e| e.to_string())?);
println!("{:?}", verbalize(&graph, &bolt, &cfg));   // Some("Usain Bolt. a athlete. Jamaican sprinter...")

let mut store = VectorStore::create("graph.spqv", 64)?;
let n = embed_entities(&graph, &mut store, &HashEmbedder::new(64), &cfg)?;  // entities with no literal text are skipped
store.finalize()?;
```

Tailor the template — add a categorical literal group, set the language chain:

```rust
use sparq_vectors::{EntityTextConfig, PropertyGroup};
use oxrdf::NamedNode;

let mut cfg = EntityTextConfig::default();
cfg.languages = vec!["en".into(), "".into()];   // "en" also matches en-GB and RDF 1.2 en--ltr; "" = untagged
cfg.groups.push(
    PropertyGroup::literal(vec![NamedNode::new("http://example.org/occupation").unwrap()])
        .with_prefix("occupation: ")            // Weaviate property-name prefix convention
        .with_max_values(3),
);
cfg.max_chars = 1024;                           // char budget; the leading label always survives
// Keep raw numbers/dates OUT of the text (models do not order numbers); verbalize only categorical values.
```

### 2. Exact vs HNSW (pick by scale)

HNSW (`VectorIndex`) is the APPROXIMATE backend behind the **opt-in `approx-ann` feature** — the
only thing pulling `instant-distance`, so the default build has NO third-party ANN dep (lean core).
It is approximate: recall < 1.0 (NOT answer-exact). Build with `--features approx-ann`.

[OPUS-4.8] (sq-lfo84) The HNSW squared-Euclidean distance is computed by an **explicit-SIMD kernel**
(`src/simd.rs`, `approx-ann`-only, no new dependency): runtime-detected **NEON** on aarch64 and
**AVX2+FMA** on x86_64, with a scalar fallback numerically bit-identical to the previous
auto-vectorised loop. It measurably cuts the graph-build time and lifts query QPS. **Recall is
floor-preserved, not bit-identical:** the SIMD kernels use FMA (one rounding) while the scalar path
does `d*d` then `+=` (two roundings), so a SIMD squared distance differs from the scalar one by ≤1
ULP — rankings are stable up to exact near-ties, and that residual is what the HNSW **floor gate**
(recall@10 ≥ 0.95, `tests/recall.rs`) absorbs (the gate is a floor, not a bit-identity assertion).
The deterministic exact / DiskANN / PQ paths keep the scalar reduction, so their EXACT-gated
`bench/vector/expected.tsv` deficits are byte-stable. Full recall-QPS + build-time evaluation matrix (SIMD vs instant-distance-scalar vs
hnsw_rs, NON-CANONICAL): `research/gap-vector-ann-simd-2026-07.md`.

[SONNET-4.6] (#5065) Which kernel actually runs is decided at runtime, so a test that only checks
the dispatcher against a reference is satisfied by the scalar fallback and proves nothing about the
intrinsic kernel. `simd::tests` therefore asserts the SELECTED kernel: on x86_64 it fails closed
when `CI` is set and AVX2+FMA are absent, rather than reporting green on an unexecuted `l2_sq_avx2`.
Set `SPARQ_VECTORS_REQUIRE_SIMD=1` to demand kernel execution on a dev box, or `=0` to accept
scalar-only coverage on a runner that genuinely lacks the extension. The aarch64/NEON side is
covered instead by the `vectors-aarch64` workflow's fail-closed `asimd` preflight (#5028).

```rust
# #[cfg(feature = "approx-ann")] {
use sparq_vectors::{nearest_exact, VectorIndex, HnswConfig};

let store = sparq_vectors::VectorStore::open("graph.spqv")?;

// Below ~10^5 vectors the exact scan is a fine default (no build cost, answer-exact):
let hits = nearest_exact(&store, query, 10);          // Vec<(Id, f32)>

// At higher query volume, build HNSW once (rayon-parallel inside instant-distance):
let index = VectorIndex::build_with(&store, HnswConfig { ef_search: 100, ef_construction: 100, seed: 0 });
let approx = index.nearest(query, 10);                // APPROXIMATE: ef_search must be >= k; recall@10 < 1.0 (run tests/recall.rs)

// [OPUS-4.8] (sq-ose80) BUILD-TIME presets — ef_construction is the dominant build knob. The
// instant-distance build is ALREADY rayon-parallel (per-layer into_par_iter); its cost is the
// per-insert greedy distance search whose beam width IS ef_construction. fast_build (efc=40) built
// ~3x faster than the default (efc=100) and ~4.4x faster than efc=200 on a 200k SIFT slice at
// recall@10 = 0.9944 (NON-CANONICAL, above the 0.95 floor); high_recall (efc=200) is the opposite
// trade. PURE CONFIG: no new dep, default UNCHANGED, deterministic for a fixed seed; query path +
// nearest_with_ef monotone contract + exact/DiskANN/PQ paths untouched. Options-eval (hnsw_rs
// re-weighed for BUILD then rejected — heavier AND slower here): research/gap-vector-ann-simd-2026-07.md §7.
let fast = VectorIndex::build_with(&store, HnswConfig::fast_build());   // efc=40 — ~3x faster build, recall floor-preserved
let dense = VectorIndex::build_with(&store, HnswConfig::high_recall()); // efc=200 — build-once, query-forever
# let _ = (fast, dense);
# }
```

### 3. Persistent on-disk index that survives process restart

```rust
use sparq_vectors::{DiskAnnIndex, VectorStore};

let store = VectorStore::open("entities.spqv")?;
let _ = DiskAnnIndex::build(&store, "entities.spqg")?;  // build + persist ONCE (single-threaded Vamana)
// every later run: no rebuild, near-instant (mmap + header check):
let index = DiskAnnIndex::open("entities.spqg")?;
let hits = index.nearest(query, 10);                    // Vec<(Id, cosine)>, recall@10 ~0.966
```

### 4. Quantize a large store, then PQ-filter + full-precision re-rank (the DiskANN loop)

PQ codes are a coarse RAM-resident *filter*, not a final ranking — re-rank the candidates
against the full-precision store. [OPUS-4.8] sq-qamd: `DiskAnnIndex::build_with_pq` now drives
this loop INSIDE the index (rank each visited node's neighbours on the in-RAM codes, re-rank the
final beam off the mmap — `nearest` reports the exact full-precision cosine), and persists the
codebook + codes as a trailing `.spqg` section so `open` reloads the cache with no rebuild:

```rust
use sparq_vectors::{DiskAnnIndex, PqConfig, VamanaConfig, VectorStore};
let store = VectorStore::open("entities.spqv")?;
let idx = DiskAnnIndex::build_with_pq(&store, "entities.spqg", VamanaConfig::default(), PqConfig::default())?;
assert!(idx.has_pq_cache());
let top10 = idx.nearest(query, 10);                    // PQ-guided traversal, exact re-ranked cosine
```

To drive the same coarse-filter-then-re-rank loop by hand (e.g. over the exact store with no graph):

```rust
use sparq_vectors::{cosine, DistanceTable, ProductQuantizer, PqConfig, VectorStore};

let store = VectorStore::open("entities.spqv")?;
let pq = ProductQuantizer::fit(store.dim(), store.iter().map(|(_, v)| v), PqConfig::default())?; // M=16,K=256 -> 16B codes, 8x at dim=32
let cache = pq.encode_store(&store)?;                  // count x M bytes, RAM-resident
let table = DistanceTable::new(&pq, query);            // asymmetric distance (query stays full-precision)

let candidates = cache.rank_pq(&table, 50);            // RAM-only coarse top-50  (PQ-alone recall ~0.60)
let mut rescored: Vec<(u32, f32)> = candidates
    .into_iter()
    .map(|(id, _)| (id, cosine(query, store.get(id).unwrap())))   // re-score on full precision
    .collect();
rescored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
let top10: Vec<_> = rescored.into_iter().take(10).collect();       // recall@10 ~0.98
```

`ScalarQuantizer` (4×, `f32→u8`) is the cheaper alternative; `reconstruct(code)` gives the
lossy preview for re-ranking. Both quantizers and all searchers share the L2-normalized
cosine convention (`cos = 1 − d²/2`).

### 5. Hybrid retrieval — fuse text vectors with another ranked signal

Neither crate depends on the other; the fusion helpers take plain `(item, score)` lists.
Over-fetch each signal (e.g. 50) for a top-10 fusion so RRF has overlap to reward.

```rust
use sparq_sim::Sim;                                    // structural similarity (separate crate)
use sparq_vectors::{fuse_rrf, fuse_scores, RRF_K};

let text: Vec<(oxrdf::Term, f64)> =
    index.nearest_term(&query, &graph, &store, 50).into_iter().map(|(t, s)| (t, s as f64)).collect();
let structural: Vec<(oxrdf::Term, f64)> = Sim::new(&graph).most_similar(&query, 50);

let hybrid  = fuse_rrf(&[&text, &structural], RRF_K, 10);   // rank-only; right default for differing score scales
let blended = fuse_scores(&text, &structural, 0.7, 10);     // min-max normalize then blend; alpha 1.0 = text only
```

`hybrid_search` packages the common RRF case — drive N retriever closures off one query
`Term` and fuse by `Term` in a single call (dedups; a term in only one list still surfaces;
RRF ignores the score scales). Widen `nearest_term`'s `f32` to `f64` so both lists share the
`(Term, f64)` shape:

```rust
use sparq_vectors::{hybrid_search, RRF_K};
let fused = hybrid_search(&query, 10, RRF_K, &mut [
    &mut |t: &oxrdf::Term| index.nearest_term(t, &graph, &store, 50)
        .into_iter().map(|(t, s)| (t, s as f64)).collect(),   // ANN (cosine)
    &mut |t: &oxrdf::Term| sparq_sim::Sim::new(&graph).most_similar(t, 50),  // structural (Jaccard) [OPUS-4.8] FQ path: block has no `use Sim`
]);
```

### 6. Out-of-RAM build / filesystem-less open

```rust
use sparq_vectors::{StreamingWriter, VectorStore};

let mut w = StreamingWriter::create("/tmp/big.spqv", 384)?;  // O(1) build memory; appends straight to disk
w.put(7, &[0.1; 384])?;                                      // duplicate ids reported at finalize, not here
let store = w.finalize()?;                                   // a normal, validated VectorStore

let bytes = std::fs::read("/tmp/big.spqv").unwrap();         // or fetched/embedded
let store2 = VectorStore::open_from_bytes(bytes)?;           // identical validation, no filesystem
let idx = sparq_vectors::DiskAnnIndex::open_from_bytes(std::fs::read("/tmp/big.spqg").unwrap())?; // .spqg counterpart
```

**wasm32 (sq-98c):** the crate **compiles to wasm** with the default features — `memmap2` is
**target-gated out** of every wasm32 build (a `[target.'cfg(not(target_arch = "wasm32"))']`
dependency, NOT a cargo feature: features are additive, so a feature could never *remove* the
dependency, and a target cfg can't leak into the native build via feature unification). On wasm
`VectorStore::open` / `DiskAnnIndex::open` fall back to a buffered `std::fs::read` into the same
f32-aligned owned backing (works on wasm targets WITH a filesystem, e.g. WASI; on browser
`wasm32-unknown-unknown` the read fails with a clean I/O error) — `open_from_bytes` is the
supported browser path for both file kinds. The CI `wasm` lane build+clippy-gates this and
asserts the wasm graph stays memmap2-free. `import_*` still need `std::fs` and are compiled off
the wasm target.

### 7. Bulk-import embeddings computed ELSEWHERE (NumPy `.npy` / flat dump)

When vectors come from an external pipeline (Python/sentence-transformers, a vendor batch job)
rather than an in-process `Embedder`, load the matrix straight into a `.spqv`. The matrix carries
no term identity, so you supply a **parallel slice of dict ids** — row `i` is stored for `ids[i]`
(the **row → dict-id contract**). Emit `(id, text)` pairs from the same scan, embed the texts
out-of-process, then import the matrix with the ids in the same order.

```rust
use sparq_vectors::{ImportBinding, ImportSpec, VectorStore};

// row i of vecs.npy lines up with ids[i]; resolve each id with graph.id_of(&term).
let ids: Vec<u32> = vec![/* alice */ 10, /* bob */ 20, /* carol */ 30];

// (a) NumPy: numpy.save("vecs.npy", arr.astype(np.float32))  -> 2-D C-order <f4/<f8
let store = VectorStore::import_npy(
    ImportSpec { spqv_path: "graph.spqv", dim: 384, ids: &ids, binding: ImportBinding::Graph(&graph) },
    "vecs.npy",
)?;                       // finalized, mmap-backed; binding=Graph embeds the fingerprint

// (b) header-less dump: arr.astype("<f4").tofile("vecs.f32")  -> exactly rows*dim*4 LE bytes
let store = VectorStore::import_numeric_dump(
    ImportSpec { spqv_path: "graph.spqv", dim: 384, ids: &ids, binding: ImportBinding::Unbound },
    "vecs.f32",
)?;
```

Fail-closed: a dtype/byte-order/fortran-order/dimensionality mismatch, a `shape[1] != dim`, a
`shape[0] != ids.len()`, a wrong dump length, a duplicate/zero/non-finite row, or a
malformed/oversized `.npy` header is an `Err` — never a silent reinterpretation. The `.npy` header
length and declared shape are bounded against the actual file size before any body allocation
(sq-tzwa hardening). `import_*` need `std::fs` and are compiled off the wasm target.

### 8. k-NN INSIDE SPARQL — the `vec:` magic predicate (opt-in, feature = `vec-predicate`)

Express nearest-neighbour search in plain SPARQL, mirroring `sparq-text`'s `text:` predicate:
the rewrite runs the k-NN, inlines the hit nodes as a `VALUES` table at the spargebra-algebra
level, and evaluates through the engine's prepared-query seam — so the **engine, planner,
executor and wasm bundle are unchanged**. This is the **only** SPARQL-level hook, and it is
**OFF by default**: the feature is the only thing that pulls `sparq-engine` into this crate, so
without it `sparq-vectors` is a pure storage+ANN crate and the base query path carries zero
`vec:` code (`cargo tree -p sparq-vectors` lists no `sparq-engine`/`spargebra`).

```toml
sparq-vectors = { path = "../sparq-vectors", features = ["vec-predicate"] }
```

```rust
use sparq_vectors::{query_vec, VectorStore};   // query_vec only exists with the feature on
// store built/finalized as above, keyed by the SAME graph's dict ids.

// vec:nearest — bind ?node to the k nearest neighbours of a query.
//   query = a node IRI (its stored vector; the seed is excluded) OR a "f1,f2,…" vector literal.
let r = query_vec(&graph,
    "PREFIX vec: <http://sparq.dev/vec#>
     SELECT ?label WHERE {
       ?node vec:nearest ( <http://example.org/bolt> 5 ) .   # 5 neighbours of bolt
       ?node rdfs:label ?label .                              # join to ordinary triples
     }", &store)?;

// vec:search — also bind a cosine score; ORDER BY DESC recovers best-first (VALUES is unordered).
let r = query_vec(&graph,
    "PREFIX vec: <http://sparq.dev/vec#>
     SELECT ?node ?score WHERE {
       ( ?node ?score ) vec:search ( \"0.1,0.9,…\" 10 )
     } ORDER BY DESC(?score)", &store)?;
```

The argument lists `( … )` are ordinary SPARQL RDF collections (spargebra lowers them to
`rdf:first`/`rdf:rest`, which the rewrite walks). Hard query errors (not silent mismatches):
the neighbour position(s) must be variables; `query`/`k` must be constants; the object list must
be exactly `( query k )` and the `vec:search` subject exactly `( ?node ?score )`; a query-vector
literal's dimension must match the store; any other `vec:` IRI is unknown. An absent/unembedded
seed IRI yields no rows. By default the unfiltered search is the **exact** full scan
(deterministic, answer-exact — a fine default below ~10⁵ vectors), with top-k membership at a
boundary score tie decided by ascending N-Triples codepoint order (VG-TIE-1, sq-tb9p0) so two
answer-exact implementations return the same top-k set on the same store. The rule is fail-closed
on the embeddable domain: a candidate containing a blank node (whose N-Triples label is
document-local, so it has no stable key) is a hard query error wherever its term — not its score
alone — would decide membership; IRIs, literals, and ground triple terms rank normally.

**Approximate `vec:` for large stores (opt-in, ALSO `approx-ann`, sq-z589).** With BOTH
`vec-predicate` *and* `approx-ann` on, `query_vec_approx` / `prepare_vec_approx` take an extra
on-disk `DiskAnnIndex` argument and run the **unfiltered** k-NN through that Vamana index instead
of the full scan — for large `.spqv` stores where brute force is the bottleneck. Everything else
(parse, rewrite, VALUES inlining, joins, error checks, seed-self-exclusion) is identical to
`query_vec`. **APPROXIMATE: recall < 1.0** — the index can miss a true neighbour; use `query_vec`
(exact) when answer-exactness matters. Pass an index built with `DiskAnnIndex::build_for(&store,
path, &graph)` (and a store bound via `.with_fingerprint(&graph)`) so the staleness guard catches a
stale index. The **filtered** path (below) is unaffected — it still uses the cost-model'd filtered
search; the approximate seam is only the unfiltered scan.

```rust
# #[cfg(all(feature = "vec-predicate", feature = "approx-ann"))] {
use sparq_vectors::{query_vec_approx, DiskAnnIndex};
let index = DiskAnnIndex::build_for(&store, "entities.spqg", &graph)?;   // build/open the Vamana graph once
let r = query_vec_approx(&graph,
    "PREFIX vec: <http://sparq.dev/vec#>
     SELECT ?node WHERE { ?node vec:nearest ( \"0.1,0.9,…\" 10 ) }",
    &store, &index)?;                                                     // APPROXIMATE top-10
# }
```

**Automatic filtered ANN (compose with `filtered-ann`, sq-bvmd + sq-3tjd).** When **both**
`vec-predicate` *and* `filtered-ann` are on, the rewrite derives the candidate id-set
([`IdMask`], recipe 9) automatically: if the `vec:` neighbour variable is **constrained by
ordinary patterns in the same BGP**, those patterns carve out the eligible nodes and the k-NN is
run as a **filtered** search (`nearest_exact_filtered`) over just that set — no separate API call.

The constraint is the **join-connected sub-BGP** of the neighbour variable (sq-3tjd): not only
patterns that *directly mention* it, but every pattern reachable from it through shared variables
— so the mask honours **transitive / multi-variable** constraints. A direct-mention-only BGP is a
special case of this (the connected component is just those patterns), so the single-variable
behaviour from sq-bvmd is unchanged.

```rust
// Direct (single-variable): ?node is the neighbour AND is itself a :Car —
// the search only ever considers Cars, so a closer non-Car is never returned.
let r = query_vec(&graph,
    "PREFIX vec: <http://sparq.dev/vec#>
     SELECT ?node WHERE {
       ?node vec:nearest ( \"1,0\" 5 ) .
       ?node <http://ex/kind> <http://ex/Car> .   # ← carves out the candidate id-set
     }", &store)?;

// Transitive (multi-variable, 2-hop): ?node is restricted to subjects that own a
// :Vehicle even though ?node never appears in the `?x a :Vehicle` pattern — the
// connected component {?node :owns ?x, ?x a :Vehicle} is evaluated and ?node projected.
let r = query_vec(&graph,
    "PREFIX vec: <http://sparq.dev/vec#> PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
     SELECT ?node WHERE {
       ?node vec:nearest ( \"1,0\" 5 ) .
       ?node <http://ex/owns> ?x .
       ?x rdf:type <http://ex/Vehicle> .          # ← reached transitively through ?x
     }", &store)?;
```

The mask is exactly the set the engine binds to the neighbour variable when that connected
sub-BGP is evaluated and the neighbour variable projected, so the filtered top-k is **identical to
post-filtering the unfiltered top-k** by that same (now transitive) constraint — and therefore a
subset of the unfiltered result. Boundary-score-tie membership in the admitted pool follows the
same VG-TIE-1 N-Triples rule as the unfiltered path (`nearest_filtered_costed_tiebreak`,
sq-tb9p0), with a node-seed excluded from the pool *before* the boundary is determined — so the
post-filter equivalence holds exactly, ties included (VG-FILT-2 in answer-exact mode). A pattern **disconnected** from the neighbour variable (no
shared-variable path) is excluded, so it never narrows the mask. Each `vec:` request in a BGP gets
its **own** connected-component mask, derived independently. If the neighbour variable is
**unconstrained** (no pattern mentions it) the search falls back to the plain unfiltered
`nearest_exact` (recipe 8's exact behaviour, unchanged). With `filtered-ann` **off**, the `vec:`
predicate is always unfiltered — this composition adds nothing to the `vec-predicate`-only build.
**Cyclic** join sub-BGPs (a back-edge to the neighbour variable like `?node :owns ?x . ?x
:ownedBy ?node`, or a longer cycle among intermediates) are handled correctly (sq-p5oy): the
connected-component computation is a per-pattern fixpoint so it always **terminates** on a cycle,
and the cyclic sub-BGP is evaluated through the standard engine, so the mask is exactly the engine's
(cycle-correct) binding — `filtered == post-filter` holds for cyclic constraints too.

**Pre-filter vs post-filter — the cost model (sq-7hx6, subsumes sq-ic0n).** Once the mask is derived,
the rewrite still has a *choice*: **pre-filter** (scan only the masked ids, `nearest_exact_filtered`)
or **post-filter** (scan the whole store unfiltered, then drop the disallowed hits, `postfilter_exact`).
Both return the **byte-identical** top-k (same ids, same order — see recipe 10), so the choice is purely
about throughput. The rewrite picks per query via `CostModel` (recipe 10): pre-filter when the mask is
selective (`mask_len · scatter_penalty ≤ store_len`, default crossover ≈ half the store), post-filter
when it is broad. This is a HEURISTIC over a cost ESTIMATE, not an optimum — it never affects the answer,
only the work. (Deriving the mask itself is unconditional: we must, to stay answer-safe; the cost model
only decides how to *use* it.)

### 9. Predicate-constrained (filtered) ANN — only neighbours a BGP admits (opt-in, feature = `filtered-ann`)

The RDF-native vector differentiator: a SPARQL BGP carves out the *eligible* graph nodes (e.g.
`?node a :Car ; :seats ?s`), and the nearest-neighbour search returns only neighbours **inside that
candidate set**. You compute the exact id-set from the BGP (via `graph.id_of` / the permutation
indexes / a `query`'s solution `?node` column) and hand it to the search as an `IdMask` — the
"visit mask". OPT-IN and OFF by default; the feature is **lean** — it adds *no new dependency* (the
mask reuses the in-tree `rustc-hash` set) and pulls in neither the engine nor any heavy crate.

```toml
sparq-vectors = { path = "../sparq-vectors", features = ["filtered-ann"] }
```

```rust
use sparq_vectors::{nearest_exact_filtered, DiskAnnIndex, IdMask, VectorIndex, VectorStore};

let store = VectorStore::open("entities.spqv")?;

// 1. Build the mask from whatever the BGP selected — here, ids of nodes typed :Car.
let car_ids = /* graph.query("SELECT ?node { ?node a :Car }") -> ?node column -> graph.id_of */;
let mask: IdMask = car_ids.into_iter().collect();          // empty mask => no results

// 2a. Exact filtered (ground truth; the pre-filter strategy) — scans only the masked ids:
let hits = nearest_exact_filtered(&store, query, &mask, 10); // every id is guaranteed in the mask

// 2b. On-disk DiskANN filtered: picks the strategy by SELECTIVITY (FilterConfig::default):
//   selective mask (<= 1% of store, or <= 256) -> exact pre-filter scan;
//   broad mask -> ACORN/NaviX filtered traversal (walk the graph through ALL nodes for
//   connectivity, ACCEPT only masked ones; the beam is widened so k accepted hits are collected).
let idx = DiskAnnIndex::open("entities.spqg")?;
let approx = idx.nearest_filtered(query, &mask, &store, 10); // recall@10 vs exact-filtered ~0.998 on a ~50% mask

// 2c. In-RAM HNSW filtered: instant-distance adjacency is NOT exposed, so this is the exact
//     pre-filter (the right path for a selective mask; use DiskAnnIndex for a broad mask):
let vidx = VectorIndex::build(&store);
let hnsw = vidx.nearest_filtered(query, &mask, &store, 10);
```

Tune the crossover / beam with `nearest_filtered_with(query, &mask, &store, k, FilterConfig { .. })`.
You build the mask yourself here, which keeps this filtered API a pure sparq-vectors surface with
zero engine coupling. The engine-side **automatic** BGP → mask wiring — deriving the `IdMask` from
the surrounding BGP and running a filtered `vec:` search — is wired in recipe 8 (sq-bvmd, when both
`vec-predicate` and `filtered-ann` are on), and is built on exactly this `nearest_exact_filtered`
seam.

### 10. Pre-filter vs post-filter — the cost model (opt-in, feature = `filtered-ann`)

Deriving a mask is one cost; *using* it is another. With a mask in hand there are two ways to the
**same** top-k: **pre-filter** (scan only the masked ids) or **post-filter** (scan the whole store
unfiltered, then drop the disallowed hits). `CostModel` estimates both and picks the cheaper per
query. The `vec:` rewrite (recipe 8) calls this automatically; you can also drive it directly.

```rust
use sparq_vectors::{nearest_filtered_costed, postfilter_exact, CostModel, Strategy};

// Decision only (no search): pre-filter iff mask_len * scatter_penalty <= store_len.
let est = CostModel::default().decide(/*mask_len*/ 50, /*store_len*/ 1000, /*k*/ 10);
assert_eq!(est.strategy, Strategy::PreFilter);   // 50*2 = 100 <= 1000 → selective → pre-filter
//   est.prefilter_cost / est.postfilter_cost expose WHY.

// Decide + run the chosen branch in one call; the second tuple element is the estimate.
let (hits, est) = nearest_filtered_costed(&store, query, &mask, 10, &CostModel::default());

// Post-filter directly (the broad-mask branch). IDENTICAL result to nearest_exact_filtered:
let post = postfilter_exact(&store, query, &mask, 10);
assert_eq!(post, sparq_vectors::nearest_exact_filtered(&store, query, &mask, 10));
```

**The model.** `m = |mask|`, `n = store_len`. Pre-filter touches `m` *scattered* rows (random-access
gather into the mmap), post-filter streams `n` rows sequentially; a scattered row is modelled as
`scatter_penalty ×` (default `2.0`) a sequential one. **Pre-filter iff `m · scatter_penalty ≤ n`** —
default crossover ≈ half the store. `k`/`dim` cancel out of the exact-scan comparison (both branches
do `dim`-width work per touched row, then take `k`), so they do not enter the crossover; they are
surfaced in `CostEstimate` for a future approximate backend.

**Why it is answer-safe.** `nearest_exact` returns a *complete* ranking of the store, so post-filtering
it and pre-filtering the mask reduce to ranking the same admitted ids by the same comparator — the
top-k is **byte-identical** (asserted in `tests/cost_model.rs`). There is **no over-fetch recall
boundary** for this exact backend: a full ranking can only under-fill `k` when the mask admits fewer
than `k` stored vectors, and then the pre-filter scan is equally short. (`overfetch_target(k, m, n) =
ceil(k/selectivity)` sizes the first fetch for the **approximate** backend, whose bounded candidate
list CAN under-fill — the iterative over-fetch path that handles it is recipe 11.)

**Honest scope.** This is a **HEURISTIC over a cost estimate, not an optimum**: `scatter_penalty` is a
single modelled constant (non-canonical, not a measured fit), real cache/SIMD behaviour is
hardware-dependent, and a planner that has *not* yet evaluated the sub-BGP would feed an *estimated*
cardinality (here the mask is materialised, so `m` is exact). A mis-estimate costs only throughput —
never the answer.

### 11. Approximate filtered ANN with iterative over-fetch (opt-in, feature = `filtered-ann` + `approx-ann`)

The cost model (recipe 10) is for the **exact** backend, which post-filters a *complete* ranking and
so never under-fills. An **approximate** backend returns a *bounded* candidate list: post-filtering it
can under-fill `k` when the admitted ids cluster *below* the fetched prefix (the recall boundary).
`nearest_filtered_overfetch` removes that boundary by growing the fetch and retrying.

```rust
# #[cfg(all(feature = "filtered-ann", feature = "approx-ann"))] {
use sparq_vectors::{
    nearest_filtered_overfetch_default, ApproxBackend, ExactBackend, DiskAnnIndex, VectorStore,
};
let store = VectorStore::open("entities.spqv")?;
let idx = DiskAnnIndex::open("entities.spqg")?;

// EXACT backend: answer-exact (recall 1.0) — the ground-truth reference, no third-party dep.
let exact = ExactBackend::new(&store);
let exact_hits = nearest_filtered_overfetch_default(&exact, query, &mask, 10);

// APPROXIMATE backend over the on-disk Vamana graph: recall < 1.0 (NOT exact).
let approx = ApproxBackend::new(&idx);
let approx_hits = nearest_filtered_overfetch_default(&approx, query, &mask, 10);
// fetch overfetch_target(k, selectivity); post-filter; if < k survive, DOUBLE the fetch & retry
// up to the backend size — fills k whenever k admitted vectors exist AND the index surfaces them.
# let _ = (exact_hits, approx_hits); }
```

**Honesty (load-bearing).** Iterative over-fetch fixes **under-fill** (returning < k when k exist); it
does **not** make the approximate backend exact. Even a full-store fetch from `ApproxBackend` is the
*approximate* ranking, so a true admitted neighbour the index never visits is still missed — **recall
stays < 1.0** (measured floor 0.90@10 in `tests/overfetch.rs`, NOT claimed as exactness). Only
`ExactBackend` is answer-exact. The A1-paper recall evidence is for the **exact/transitive** answer-safe
path (recipe 10), and does **not** transfer to this approximate path. Implement your own backend by
`impl`-ing `AnnBackend` (one `candidates(query, fetch)` method + `len`).

### 12. Incremental add / remove / update without a full rebuild (opt-in, feature = `delta`)

A `.spqv` is build-once-immutable, so one new/removed entity would otherwise force a full re-embed +
rebuild. The `delta` feature adds a **delta sidecar** over the immutable base — `add` / `remove`
/ `update` write an append map + tombstone set, and `get` / `iter` / `len` (hence search) transparently
union base+delta and honour tombstones. `compact` folds it back into a fresh base. Lean (no new dep).

```rust,ignore
# // cargo build -p sparq-vectors --features delta
use sparq_vectors::VectorStore;
let mut store = VectorStore::open("graph.spqv")?;   // a finalized base

store.add(new_id, &vec_for_new_id)?;                 // NEW id (errs if already present)
store.update(existing_id, &new_vec)?;                // replace an existing id (errs if absent)
store.remove(stale_id);                              // tombstone (returns bool; re-add allowed)

// search transparently sees the delta — no rebuild, no extra API:
let hits = sparq_vectors::nearest_exact(&store, &query, 10);

// PERSIST the delta so the mutations survive a restart WITHOUT a compact (sq-7e50):
store.save_delta()?;                                 // crash-durable .spqd sibling (tmp+fsync+rename)
// …later, in a fresh process: reopen the base AND replay the persisted delta:
let reopened = VectorStore::open_with_delta("graph.spqv")?;  // == the (base + delta) view it had before

// or fold the delta into a fresh base == a from-scratch rebuild over the final vector set:
let compacted = store.compact("graph.spqv", &graph)?;   // bound to graph's fingerprint generation
# Ok::<(), String>(())
```

**Generation tie.** A delta carries the graph **generation** (the sq-32i5 fingerprint) it was built
against; `apply_delta` rejects a delta whose generation differs from the receiving base, so its dict-ids
can never mis-key onto a different graph. The persisted `.spqd` header carries that same generation, so
`open_with_delta` routes the replay through `apply_delta` and a sidecar written against a *different*
graph generation is rejected, never silently mis-keyed. **Durability (sq-7e50).** Two durability paths:
`compact` materializes the delta into a fresh validated `.spqv`; `save_delta` persists the *delta itself*
to a crash-durable `.spqd` (write-tmp + `fsync` + atomic rename) that `open_with_delta` replays on reopen
— so incremental mutations survive a process restart with no compact. A truncated/torn sidecar is
detected (an exact-length check) and rejected, never read out of bounds. `put` is unchanged (still errors
after `finalize`) — `add` is the additive path.

### 13. Structure-aware preprocessing — closure + type-constrained negatives (opt-in, feature = `structure`)

<!-- [OPUS-4.8] sq-0wo9e.1 (epic sq-0wo9e; design research/structure-aware-vectorisation.md §5.A/§2). -->
Research-grade **P0** of the structure-aware-vectorisation epic. Two additive, buildable primitives an
**out-of-process** KGE trainer consumes — this crate **trains nothing** (embeddings are produced
outside it) and serves no exact answer.

1. **Closure-before-vectorise** — run the `sparq-reason` RDFS/OWL-RL closure **before** the
   vectoriser sees the graph, so entailed `rdf:type`/`subClassOf`/domain/range triples are *real
   facts* the encoder, type extractor, and sampler read (design §5.A; the reasoner is sound+complete
   for its profile, incremental closure property-tested == from-scratch).
2. **Type-constrained negative sampling** (Krompass et al. 2015) — a `NegativeSampler` corrupts a
   positive triple's head only to **domain**-consistent entities and its tail only to **range**-consistent
   ones, reading declared+observed domain/range from `sparq-introspect`. The `SamplingMode`
   (`Unconstrained` / `TypeConstrained`) is the **on/off ablation switch** (design §6.B) — a harness
   measures Hits@k/MRR both ways on the same seed. **No benchmark numbers exist; none are claimed.**

```rust,ignore
# // cargo build -p sparq-vectors --features structure
use sparq_reason::Profile;
use sparq_vectors::{
    close_for_vectorise, Corrupt, NegativeSampler, SamplingMode, TermScope, TypeConstraints,
};

let closed = close_for_vectorise(turtle_src, "turtle", Profile::Rdfs)?;  // materialise entailed facts
let constraints = TypeConstraints::mine(&closed.graph);                  // declared+observed domain/range
let sampler = NegativeSampler::new(&closed.graph, &constraints, SamplingMode::TypeConstrained);
let negatives = sampler.sample([h, r, t], Corrupt::Tail, 16, /*seed*/ 42);  // type-valid tail corruptions

// [GPT-5.6] RDF 1.2 triple terms remain excluded by default. The explicit ablation arm admits
// them, with atomic slots drawing only atomic candidates and triple-term slots only triple terms.
let scoped = NegativeSampler::new_scoped(
    &closed.graph,
    &constraints,
    SamplingMode::TypeConstrained,
    TermScope::Embeddable,
);
let triple_term_pool_size = scoped.triple_term_count();
# let _ = (negatives, triple_term_pool_size);
# Ok::<(), String>(())
```

**Fail-open by design.** A predicate with no known domain (or range) class degrades to uniform
corruption for that side — a missing/wrong schema never deadlocks the sampler. The sampler is
deterministic for a fixed `(seed, mode, triple)` and applies the standard "filtered" guard (never
emits a corruption that is itself a true triple). **Honesty:** the empirical benefit of either prior is
**unproven** — both ship behind the ablation precisely because the literal/type-aware KGE literature
reports inconsistent gains; this slice is the buildable inputs. **The trainer that consumes them now
exists** behind the `kge` feature (recipe 14).

`TermScope::IriBlank` is the `Default` and preserves the former named-node/blank-node entity space.
`TermScope::Embeddable` is an explicit RDF 1.2 triple-term measurement arm. It does not make triple
terms compositional: it exposes each term as a graph node through `rdf:reifies`, while the embedded
term's internal `(s, p, o)` remains opaque. `NegativeSampler::new` retains the default scope;
`new_scoped` is the opt-in entry point. The separate corruption pools prevent sort-trivial negatives.

### 14. KGE measurement foundation — DistMult/ComplEx trainer + filtered link-prediction ablation (opt-in, feature = `kge`)

<!-- [OPUS-4.8] sq-0wo9e.8 / P6 (epic sq-0wo9e; design research/structure-aware-vectorisation.md §P6 + sq-0wo9e.8). -->
The `kge` feature (implies `structure`) is the **measurement foundation** for the epic: a **thin,
CPU-only, deterministically-seeded shallow KGE** that *consumes* the P0 closure + type-constrained
negatives to produce embeddings, plus the standard **filtered link-prediction** harness that measures
them. It is the *instrument* — it makes **no accuracy claim**. Two scoring models behind the
`ModelKind` axis: **DistMult** (Yang 2015, symmetric) and **ComplEx** (Trouillon 2016, asymmetric).
No new dependency (hand-rolled SGD, no ML crate).

```rust,ignore
# // cargo build -p sparq-vectors --features kge
use sparq_vectors::{close_for_vectorise, train, ModelKind, TrainConfig, SamplingMode, TypeConstraints};
use sparq_reason::Profile;

let closed = close_for_vectorise(turtle_src, "turtle", Profile::Rdfs)?;   // closure-before-vectorise
let tc = TypeConstraints::mine(&closed.graph);
// small() defaults to the ASYMMETRIC ComplEx; small_with_model(..) selects DistMult explicitly.
let cfg = TrainConfig::small(SamplingMode::TypeConstrained, /*seed*/ 7);  // tiny, reproducible
let (model, report) = train(&closed.graph, &tc, cfg);
assert!(report.loss_decreased());                  // it learns
let score = model.score(h, r, t);                  // model.model is ModelKind::{DistMult,ComplEx}
# Ok::<(), String>(())
```

**The model axis is load-bearing.** DistMult scores `(h,r,t)` and `(t,r,h)` identically, so on a
relation whose true edges are **directional** it is structurally near-random — it cannot place the
head above the tail. Both synthetic eval slices here are ~100 % directional, so a DistMult ablation
runs in a near-random regime where the inter-cell deltas are noise. **`EvalConfig::small` defaults to
the asymmetric ComplEx**, and any ablation delta must be read off the ComplEx run, not DistMult.

The **eval harness** runs the 2×2 P0 ablation matrix and reports filtered MRR / Hits@1/3/10 with a
long-tail breakdown. Because a single seed's inter-cell delta can be a handful of rank hits (noise),
report each cell as a **mean ± std over several seeds** with `run_ablation_multiseed`:

```rust,ignore
# // cargo build -p sparq-vectors --features kge
use sparq_vectors::{run_ablation_multiseed, synthetic_relational_ttl, EvalConfig};
let ttl = synthetic_relational_ttl(400, /*seed*/ 42);   // or your own KG text
let template = EvalConfig::small(13);                    // ComplEx by default
let cells = run_ablation_multiseed(&ttl, "turtle", template, &[1,2,3,4,5])?;  // [(F,F),(F,T),(T,F),(T,T)]
for c in &cells {
    println!("closure={} type-neg={} MRR={:.4}+/-{:.4}",
        c.closure, c.type_constrained, c.metrics.mrr.mean, c.metrics.mrr.std);
}
# Ok::<(), String>(())
// A delta smaller than the combined spread of the two cells is NOT yet evidence.
```

**Read the effect off the PAIRED delta, not the unpaired means (sq-4891y).** Comparing two cells'
`mean ± std` and eyeballing the gap is the *unpaired* view — needlessly noisy, because the four cells
of one seed share the split / init / negatives, so most per-seed variance is *shared* and cancels in
their difference. `run_ablation_multiseed_paired` returns the **paired** per-seed closure delta (and
type-negative delta) as a `PairedDelta { mean, std, se, n }` whose paired `std` is far smaller than
the unpaired sum-of-stds, with a `significant_at(k)` gate (`mean − k·se > 0`, requires `n ≥ 2`):

```rust,ignore
# // cargo build -p sparq-vectors --features kge
use sparq_vectors::{run_ablation_multiseed_paired, synthetic_gufo_ttl_sized, EvalConfig};
let ttl = synthetic_gufo_ttl_sized(400, /*density*/ 3, 9); // denser → more held-out triples/seed
let template = EvalConfig::small(13);                       // ComplEx; bump epochs for a tighter run
let r = run_ablation_multiseed_paired(&ttl, "turtle", template, &(0..12).collect::<Vec<_>>())?;
let d = r.closure_mrr; // headline closure-prior lift, variance-reduced
println!("closure delta={:.4} se={:.4} sig@2se={}", d.mean, d.se, d.significant_at(2.0));
# Ok::<(), String>(())
// Firm-up bar: a positive PAIRED delta clearing 2·se on a SCHEMA-BEARING slice under ComplEx.
```

`synthetic_gufo_ttl_sized(n, density, seed)` is the **schema-bearing** firm-up slice: the rigid
`Person` kind is asserted on **nobody**, so the RDFS closure must materialise it (the closure axis
genuinely bites — unlike schema-free WN18RR/FB237). A higher `density` adds more learnable held-out
triples per seed, shrinking the per-seed MRR variance, while keeping the decoy set (non-triviality)
intact. `examples/kge_ablation.rs` prints the paired delta + an LR sweep.

**Filtered protocol (load-bearing).** Each ranking removes **every** known-true triple (train + valid +
test) before scoring the held-out one — the established Bordes 2013 protocol; getting it wrong
invalidates every number. Train/valid/test are a leakage-free partition (a triple is in exactly one
split); the trainer sees **train only**. Only **non-schema relations** are prediction targets
(`SCHEMA_PREDICATES` — `rdf:type`/`subClassOf`/domain/range are *structural context*, never targets,
so the closure axis is not distorted by trivially-derivable entailed types). The runnable ablation is
`examples/kge_ablation.rs` (runs BOTH models, multi-seed; a real dataset goes behind
`SPARQ_KGE_DATASET=/path/to/kg.nt`). **All numbers are INDICATIVE only** (the work-box is
non-canonical) and are **never** baked into docs. **Adoption gate:** no prior is adopted from these
synthetic, work-box, single-seed figures — adoption requires a **real dataset** (WN18RR/FB15k-237
subset), on a **canonical machine**, under the **asymmetric model**, with **multi-seed** reporting.

### 14a. UFO/gUFO priors — answer-safe serve-time disjointness mask (opt-in, feature = `kge`)

<!-- [GPT-5.6] PR #2143 / issue #2149: document the public UFO-prior surface. -->
The gUFO-prior cell is wired as an explicitly selected serve-time ablation. `EvalConfig::small`
sets `gufo_prior = false`, so the default run does not construct the mask and retains the baseline
ranking path. Set it to `true` to mine only UFO-provable disjointness from the input graph and remove
provably incompatible candidates using the predicate's declared `rdfs:domain` or `rdfs:range`.
Training is unchanged. The reader fails closed when identity or nature evidence is ambiguous, and an
untyped candidate or predicate without a declared signature is never excluded.

```rust,ignore
# // cargo build -p sparq-vectors --features kge
use sparq_vectors::{run_ablation, EvalConfig, GUFO_NS};

let mut cfg = EvalConfig::small(13);
assert!(!cfg.gufo_prior); // the behavioural prior is default OFF
cfg.gufo_prior = true;
cfg.gufo_ns = GUFO_NS; // or an explicitly declared non-canonical namespace
let cells = run_ablation(turtle_src, "turtle", cfg)?;
assert!(cells.iter().all(|cell| cell.gufo_prior));
# Ok::<(), String>(())
```

The `structure` feature exposes the read-only `UfoPriors`, `UfoVocabulary`, `Rigidity`,
`OntologicalNature`, and `GUFO_NS` API without the trainer. Use `UfoPriors::mine(graph)` for the
canonical namespace or `mine_with_namespace(graph, ns)` when the dataset explicitly uses another
namespace. `proven_disjoint_pairs()` and `proven_subsumptions()` return dictionary-id facts only;
`augment_oracle()` feeds the proven pairs into `DisjointnessOracle::absorb_proven_pairs`. These APIs
do not mint terms or write inferred triples back into the graph.

The load-bearing guards are exact: the feature is absent from `default = []`, `kge` merely implies
the already opt-in `structure` feature, `EvalConfig::small` keeps the behavioural switch false, and
tests compare OFF runs deterministically plus ON/OFF output exactly on a gUFO-free graph. The mask
may improve or preserve a filtered rank, never remove the held-out answer.

**RDF 1.2 triple-term visibility is also default-off.** [GPT-5.6] Every `TrainConfig` preset sets
`term_scope` to `TermScope::IriBlank`, preserving the existing pipeline. To measure statement-level
structure, use `synthetic_rdf12_ttl` (or your own N-Triples data with `rdf:reifies`) and the paired
`run_quoted_ablation` runner. Its `QuotedAblation` reports common-random-number ON−OFF deltas; split
membership and the ranking pool remain atomic in both arms, so the comparison changes only training
visibility. `synthetic_rdf12_parts` exposes the generated fixture partitions when a caller needs to
audit them. This is a measurement surface, not an accuracy claim.

**Statement-level quoted-triple encoding is compositional and derived (sq-1e5kk).** [SONNET-4.6]
`TrainedModel::encode_quoted_term(&graph, id)` (and the unpacked forms `encode_statement(h, r, t)` /
`encode_statement_rows`) returns a triple term's per-component interaction vector composed from
its `(s, p, o)` constituents' trained rows — DistMult `h∘r∘t` (sums to the score), ComplEx
`h∘r∘conj(t)` in real/imaginary halves (the real half sums to the score) — so the quoted *content*
is reachable even under the default `IriBlank` scope, where the term itself has no node row. It is
deterministic and derived (no new parameters, one level deep, never recursive), it lives in the
model's **interaction space** (never compare it against entity rows or store it beside them), and it
leaves the node-level opaque-row path above intact. No accuracy claim — adopting either
representation stays measurement-gated.

### 14b. Provenance-weighting `w(t)` — weight training by PROV-O/DQV quality (opt-in, feature = `structure`; measurement under `kge`)

<!-- [OPUS-4.8] sq-2489d.4 (epic sq-2489d, GenAI-KB Phase 4; design research/provenance-driven-genai-kb.md §USE-1 / §5 Phase 4). -->
Research-grade **Phase 4** of the provenance-driven GenAI-KB epic: derive a per-triple
**provenance-quality weight** `w(t) ∈ (0,1]` from a graph's PROV-O / DQV annotations so a
high-assurance fact contributes a full-strength training gradient and a low-assurance / low-source
fact a down-weighted one (the CKRL confidence-weighted-loss move). `ProvenanceWeights` **mirrors
`ShaclPriors`**: a read-only id-level scan of `pkg:confidence` / `pkg:assurance` /
`prov:wasDerivedFrom` (the same terms the Phase-1 `sparq-nlq` join reads and the Phase-3 `pkg.ttl`
DQV terms declare) that pulls **no engine** — it consumes only `sparq-core`'s public id-scan API, so
the `structure` feature stays lean.

```rust,ignore
# // cargo build -p sparq-vectors --features structure
use sparq_vectors::{close_for_vectorise, ProvenanceWeights, WeightMode};
use sparq_reason::Profile;
let closed = close_for_vectorise(ttl, "turtle", Profile::Rdfs)?;
let weights = ProvenanceWeights::mine(&closed.graph);
// w(t) = clamp(assurance_mult · head_confidence · min_source_reliability, floor, 1.0).
// Uniform mode is the ablation-OFF baseline (always 1.0); Provenance mode applies the quality weight.
let w = weights.weight_of([h, r, t], WeightMode::Provenance); // a head with NO provenance → 1.0 (fail-open)
# Ok::<(), String>(())
```

`w(t)` combines the qualifying subject's `pkg:confidence` (epistemic weight), an **assurance
multiplier** (`secx:Proven` → high, `Claimed` → mid, `Conjectured` → low; configurable via
`WeightConfig`), and the **min** `pkg:confidence` over its `prov:wasDerivedFrom` sources (a fact is
only as reliable as its least-reliable source). The qualifying subject is the **reified statement**
where the graph carries one (RDF 1.2 `rdf:reifies`, or RDF 1.1
`rdf:subject`/`rdf:predicate`/`rdf:object`) — that is `w(t)` proper — and otherwise the **head**,
a documented fallback that reports "this entity is low-assurance", not "this assertion is doubtful".
It is clamped to `[floor, 1.0]` so a positive is **down-weighted, never dropped** (a zero weight
would silently delete it from the loss). The `kge` trainer reads it via the
new `TrainConfig::weight_mode` (default `Uniform`); under `Provenance` the **positive** step's
effective LR is scaled by `w(t)` (negatives are unweighted — a corruption has no provenance).

**Ablation + the adopt/abandon bar.** `run_weight_ablation` trains the **same** config twice per seed
(weighting ON vs OFF, holding closure + type-negatives fixed) and returns a **paired** per-seed MRR /
Hits@10 delta — `WeightAblation::mrr_significant_at(k)` is the firm-up gate:

```rust,ignore
# // cargo build -p sparq-vectors --features kge
use sparq_vectors::{run_weight_ablation, synthetic_provenance_ttl, EvalConfig};
let ttl = synthetic_provenance_ttl(160, 5); // low-assurance edges are deliberately the NOISIER ones
let ab = run_weight_ablation(&ttl, "turtle", EvalConfig::small(21), &[1,2,3,4])?;
println!("Δmrr={:.4} se={:.4} adopt@2se={}", ab.mrr.mean, ab.mrr.se, ab.mrr_significant_at(2.0));
# Ok::<(), String>(())
```

**No accuracy claim is made.** Over a provenance-free graph the two arms are byte-identical and the
delta is **exactly zero** (the no-op invariant — a plain graph is unchanged). The
confidence/literal-aware KGE literature reports *inconsistent* gains, so any lift is unproven and
dataset-dependent; the bead's discipline is **adopt only if the measured lift clears a pre-registered
bar, and ABANDON (and say so) otherwise**.

**Integration points 2–3 (sq-oy9ya, design §USE-1).** `w(t)` is now also threaded into the two
other points `sparq-vectors` has, both behind the **same `WeightMode` ablation** and both no-ops
under `Uniform` / a provenance-free graph:

```rust,ignore
# // cargo build -p sparq-vectors --features structure
use sparq_vectors::{ProvenanceWeights, WeightMode, Block, Encoder, Metric};
# fn demo(weights: &ProvenanceWeights, edge_a: [u32; 3], edge_b: [u32; 3], subj_a: u32,
#         subj_b: u32) -> Result<(), String> {
// (2) confidence-weighted STRUCTURAL-SKETCH / characteristic-set pooling: pool a node's
//     multi-valued contributions weighted by each CONTRIBUTING ASSERTION's w(t), NOT a uniform
//     mean. Under WeightMode::Uniform this is EXACTLY the arithmetic mean (the ablation-off
//     baseline) — and so is Provenance when no reified statement qualifies the edges.
let contribs = vec![(edge_a, vec![1.0, 0.0]), (edge_b, vec![0.0, 1.0])];
let pooled = weights.pool_weighted(&contribs, WeightMode::Provenance)?; // higher-quality value dominates

// (3) per-Block query-time FUSION weight: aggregate the incident-edge provenance into a per-block
//     multiplier and attach it to a Block; the existing fuse_rrf_weighted / fuse_scores path
//     consumes Block::fusion_weight() so a low-quality modality contributes less to the ranking.
let bw = weights.block_weight([subj_a, subj_b], WeightMode::Provenance); // mean of the per-subject w(t)
let block = Block::new(Encoder::Numeric, Metric::Euclidean, 0, 16).with_weight(bw);
assert!(block.fusion_weight() > 0.0); // a valid (0,1] fuse weight; None ≡ 1.0 (fail-open)
# let _ = pooled; let _ = block; Ok(()) }
```

The `.spqv` `SchemaHeader` round-trips the per-block weight (format **v2**; a **v1** sidecar still
parses, every block read back as the fail-open `1.0`). The weight is **layout metadata**, not part
of a `Block`'s identity — `PartialEq`/`Eq` ignore it, so the header round-trip contract is unchanged.
**No accuracy claim**; like point 1, adoption is measurement-gated.

**Wired end-to-end (sq-w2af4).** Points 2–3 above are the *primitives*; these are the in-tree
callers that actually consume them, so the loop runs graph → provenance → vector path without a
hand-written middle:

```rust,ignore
# // cargo build -p sparq-vectors --features structure
use sparq_vectors::{ground_weighted, sketch_predicate, Grounding, GroundingConfig, Modality};
use sparq_vectors::{NodeWeighting, ProvenanceWeights, WeightMode};
# fn demo(graph: &sparq_core::Graph, store: &sparq_vectors::VectorStore,
#         header: &sparq_vectors::SchemaHeader) -> Result<(), String> {
let pw = ProvenanceWeights::mine(graph);
let block_preds = [Some("http://ex/good"), None];

// (3) DEFAULT: derive each block's GRAPH-GLOBAL fusion weight — block i is fed by
//     block_predicates[i]; `None` leaves that block at the fail-open 1.0. The header is shared,
//     so this is one multiplier per BLOCK, persisted in the `.spqv` sidecar.
let weighted = pw.weight_header(graph, header, &block_preds, WeightMode::Provenance)?;

// (3) PER-NODE: `ground_weighted` overrides those defaults with weights mined from THIS node's
//     own incident edges, ready for `fuse_rrf_weighted` / `fuse_scores`. Pass `None` (or call
//     plain `ground`) to keep the persisted header defaults instead.
let weighting = NodeWeighting {
    weights: &pw, block_predicates: &block_preds, mode: WeightMode::Provenance };
if let Some(Grounding::TypedSubVector { weights, .. }) = ground_weighted(
    graph, &node, Modality::TypedSubVector, &GroundingConfig::default(), None,
    Some((store, &weighted)), Some(&weighting)) { let _ = weights; }

// (2) the structural-sketch pooler: pool a node's multi-valued predicate over its neighbours'
//     stored vectors, each weighted by THAT ASSERTION's w(t). Uniform ⇒ exactly the arithmetic
//     mean, and so is Provenance on a graph with no reified statements.
let sketch = sketch_predicate(graph, store, &node, "http://ex/cites", &pw, WeightMode::Provenance)?;
# let _ = sketch; Ok(()) }
```

**What `w(t)` keys on — and where it is honestly a no-op.** `ProvenanceWeights` reads `w(t)` at two
levels: **statement-level** when the graph reifies the triple (RDF 1.2 `:st rdf:reifies
<<( s p o )>>`, or RDF 1.1 `rdf:subject`/`rdf:predicate`/`rdf:object`) and the reifier itself
carries `pkg:confidence` / `pkg:assurance` / `prov:wasDerivedFrom`; otherwise **head-level**, the
subject entity's own annotations. Two consequences to internalise before quoting a result:

- `sketch_predicate` keys each contribution on **the asserting triple**. Where the graph carries no
  statement-level provenance (`ProvenanceWeights::annotated_statements() == 0`), every
  `(node, predicate, ·)` edge falls back to the same head weight and the pool is **exactly** the
  arithmetic mean — this axis is an honest **no-op** there, not a substituted heuristic. It is
  deliberately not keyed on the object entity: "the object is a low-assurance entity" is a
  different claim from "this assertion is doubtful".
- `weight_header` keys each block on the **graph-global** mean over every subject asserting that
  block's feeding predicate. `SchemaHeader` is one shared, graph-wide layout header, so that is a
  per-block **default** — every node grounded through `ground` sees the same weights. For the
  design's per-node scaling use `ground_weighted` with a `NodeWeighting`, which computes each
  block's multiplier from **that node's own incident edges**
  (`ProvenanceWeights::node_block_weight`); a node with no incident edge for the predicate fails
  open at `1.0` rather than inheriting the graph average.

**Measurement-gated, with no result published here.** `eval::run_pooling_ablation` (feature `kge`)
is the paired instrument for the pooling axis — one training run per seed, both arms
post-processing the *same* parameters, so the delta isolates the pooling weights. It returns paired
per-seed MRR / Hits@10 deltas with their standard error; `mrr_significant_at(k)` reports whether the
lift clears `k` standard errors of its own paired spread. Read it that way: a delta that does not
clear the pre-registered bar is **no measured lift**, and the honest verdict for that axis on that
slice is ABANDON, not a defect to be explained away. On a graph with no **statement-level**
provenance (including any provenance-free graph) both arms pool with identical weights and every
delta is exactly zero by construction — read a zero there as "no per-statement signal in this
graph", not as "weighting did not help".

Run it yourself rather than trusting a number quoted here — results are seed-, fixture- and
box-dependent and are **not** canonical:
`cargo run -p sparq-vectors --release --features kge --example kge_ablation`. Any adoption decision
must be re-measured on a real, provenance-bearing KG.

### 15. Typed-literal encoders — order-preserving numeric / boolean / date + schema header (opt-in, feature = `structure`)

<!-- [OPUS-4.8] sq-0wo9e.2 (epic sq-0wo9e; design research/structure-aware-vectorisation.md §3.1/§3.2/§6.A). -->
Research-grade **P1**: the typed-literal encoders that turn a node vector into a **structured
partitioned object** (design §3). All are **pure functions keyed by datatype** in the `encode`
module — no training, no graph state, no I/O.

- **Datatype `route`r** — the datatype IRI alone selects the encoder family
  (`Encoder::{Numeric,Boolean,Date,Other}`). Exact, free.
- **`NumericEncoder`** — the **order-preserving** magnitude encoder (the one the design gates as
  formally provable): per-predicate quantile-normalise + a strictly-monotone **thermometer** code
  whose **L2 distance is monotone in value distance over the whole observed range** (NOT a periodic
  Fourier code). `metamorphic_monotone` is the provable gate; a sine code FAILS it by design.
- **`BooleanEncoder`** — one ±1 sign dimension, **exact, 2-valued**, round-trips (`true`→+1,
  `false`→−1; `decode(encode(b)) == b`).
- **`DateEncoder`** — maps `xsd:date`/`dateTime`/`dateTimeStamp`/`gYear` to an order-preserving
  epoch lane (via `sparq-core`'s `Timeline`) and reuses the numeric encoder, so chronological order
  is preserved.
- **`SchemaHeader`** — the self-describing `.spqv` partition descriptor: contiguous `Block`s
  (encoder + `Metric` + dim span) with a **metric-correctness guard** (`check_euclidean`) so a whole-row
  L2/cosine search never silently runs over a non-Euclidean (e.g. future taxonomy) block. Round-trips
  to bytes (`SPQS` magic) for a sidecar.

```rust,ignore
# // cargo build -p sparq-vectors --features structure
use sparq_vectors::{route, Encoder, NumericEncoder, BooleanEncoder, DateEncoder,
    SchemaHeader, Block, Metric};

assert_eq!(route("http://www.w3.org/2001/XMLSchema#integer"), Encoder::Numeric);
let enc = NumericEncoder::fit([1.0, 30.0, 31.0, 70.0, 5000.0], /*dim*/ 16); // per-predicate fit
let v30 = enc.encode(30.0);                                                 // order-preserving block
assert_eq!(BooleanEncoder::decode(BooleanEncoder::encode(true)[0]), true);  // exact round-trip
let header = SchemaHeader::new(vec![
    Block::new(Encoder::Numeric, Metric::Euclidean, 0, 16), // .with_weight(w) for a fusion weight
])?;
header.check_euclidean()?;  // every block is L2-searchable
# Ok::<(), String>(())
```

**Honesty.** Order-preservation, boolean exactness, datatype-dispatch correctness, and the metric
guard are **proven** (`encode.rs` tests). Whether the typed encoders raise downstream
link-prediction / retrieval is **empirical and dataset-dependent** (design §6.B/§9) — no accuracy
claim is made. The encoder-quality ablation runner is `examples/bench_typed_encoders.rs` (P1 ON vs
OFF, with a long-tail slice); its numbers are **work-box NON-CANONICAL**. The encoders are inputs a
trainer would consume — the thin KGE trainer that consumes them now exists behind the `kge` feature
(recipe 14). <!-- [OPUS-4.8] sq-0wo9e.8 -->

### 16. SHACL/OWL priors + QUDT unit-normalisation — enum codebook / cardinality pooling / SI magnitudes (opt-in, `structure` + `structure-shacl`)

<!-- [OPUS-4.8] sq-0wo9e.3 (epic sq-0wo9e; design research/structure-aware-vectorisation.md §2 + §7). -->
Research-grade **P2**: read the schema sparq already holds — `sh:in` enums, `sh:datatype`,
`sh:min/maxCount`, `owl:FunctionalProperty`, and QUDT units — as **priors over the encoder layout**,
not post-hoc filters. Two slices:

- **QUDT unit-normalisation (`structure`, pure — no SHACL dep)** — `units::normalise(value, unit_iri)`
  converts a unit-annotated magnitude to its **canonical SI value + `QuantityKind`** via a bundled
  affine table, **before** the order-preserving `NumericEncoder`, so `1000 m` and `1 km` map to the
  **identical** code (design §2 "unit-normalise-before-magnitude"). `same_quantity(a, b)` makes a
  length-vs-mass mismatch a **detectable error** (a length never shares a numeric block with a mass).
  Unknown unit / non-finite value → `None` (**fail-closed**, never silently dimensionless).
- **Enum `Codebook` (`structure`, pure)** — a one-hot block over a closed enum: slot `0` is the
  reserved **out-of-enum / invalid** code, slots `1..=n` the members. **Enum equality is a slot match,
  not a cosine threshold** (design §2 "no recall loss"); a non-member encodes to the invalid slot a
  closed-world `sh:in` shape rejects. `encode`/`decode` round-trip every member.
- **`ShaclPriors` reader (`structure-shacl`, pulls `sparq-shacl`)** — `ShaclPriors::from_model` /
  `from_model_and_data` mine, **per predicate** from a parsed `ShapesModel` (read-only — no SHACL
  changes): the enum `Codebook` (`sh:in`), the router-confirmed `Encoder` (`sh:datatype`), and the
  **pooling rule** `Cardinality::{Functional,Multi}` (`sh:maxCount 1` or `owl:FunctionalProperty` from
  the data graph → one deterministic slot; else a permutation-invariant pooled block).

```rust,ignore
# // cargo build -p sparq-vectors --features structure-shacl
use sparq_vectors::{normalise, QuantityKind, NumericEncoder, ShaclPriors, Cardinality};
use sparq_shacl::model::ShapesModel;

// unit-normalise before the magnitude encoder: 1000 m == 1 km share a code.
let m = normalise(1000.0, "http://qudt.org/vocab/unit/M").unwrap();
let km = normalise(1.0, "http://qudt.org/vocab/unit/KiloM").unwrap();
assert_eq!(m.canonical_value, km.canonical_value);          // same physical length
let enc = NumericEncoder::fit([m.canonical_value], 16);
assert_eq!(enc.encode(m.canonical_value), enc.encode(km.canonical_value));

// SHACL priors: sh:in → codebook (slot match), sh:maxCount 1 → functional pooling.
let model = ShapesModel::parse(&shapes_graph);
let priors = ShaclPriors::from_model(&model);
if let Some(p) = priors.get("http://ex/status") {
    let cb = p.enum_codebook.as_ref().unwrap();             // out-of-enum → reserved invalid slot
    assert!(matches!(p.cardinality, Cardinality::Functional | Cardinality::Multi));
    let _ = cb.member_count();
}
```

**Honesty.** Enum/boolean exactness (slot match), the affine unit conversion, and the
SHACL-detectable mismatch are **proven** (`codebook.rs` / `units.rs` / `shacl_priors.rs` tests +
`tests/structure_shacl.rs`). The QUDT table is a **curated, deliberately small** subset (common SI +
a few customary units); an absent unit fails closed — extending it is additive. Whether these priors
raise downstream retrieval is **empirical, ablation-gated** — no accuracy claim. A predicate with no
declared shape simply gets **no** prior (fail-open, the encoder falls back to the datatype router).

### 17. Taxonomy block + disjointness repulsion/mask (opt-in, feature = `structure`)

<!-- [OPUS-4.8] sq-0wo9e.4 (epic sq-0wo9e; design research/structure-aware-vectorisation.md §2/§3.3/§6.A/§9). -->
Research-grade **P3**: two priors over the `rdfs:subClassOf` DAG, in the `taxonomy` module. Read them
over a **closed** graph (`close_for_vectorise` first, recipe 13) so the `subClassOf` closure and
entailed disjointness are materialised.

- **`TaxonomyDag::build`** — extracts the `subClassOf` DAG (cycle-safe `ancestors`/`depth`/
  `graph_distance`).
- **`EuclideanTaxonomyEncoder`** — the **default** taxonomy block (normalised-depth lane + hashed
  ancestor bag), tagged `Metric::Euclidean` so whole-row L2/cosine is correct: classes sharing more
  ancestry are closer. A `HyperbolicTaxonomyEncoder` (Poincaré candidate, tagged `NonEuclidean`)
  exists **only** as the gate's second arm.
- **`GeometryGate`** — the load-bearing must-fix: it **measures** Euclidean-vs-hyperbolic distortion
  on the *actual* DAG and adopts non-Euclidean **only** when it strictly beats Euclidean by a margin
  (never on a density heuristic; design §3.3/§9.4).
- **`DisjointnessOracle`** — mines `owl:disjointWith`/`AllDisjointClasses`/`complementOf`, propagates
  down the `subClassOf` closure, and yields **train-time** `repulsion_pairs` + a **serve-time**
  `mask_candidates` hard mask. The mask is **answer-safe**: it drops only candidates whose type is
  *provably* disjoint from a query type (∀ output ⊆ input).

```rust,ignore
# // cargo build -p sparq-vectors --features structure
use sparq_vectors::{close_for_vectorise, TaxonomyDag, EuclideanTaxonomyEncoder, GeometryGate,
    Geometry, DisjointnessOracle};
use sparq_reason::Profile;

let closed = close_for_vectorise(turtle_src, "turtle", Profile::Rdfs)?;   // materialise the closure
let dag = TaxonomyDag::build(&closed.graph);
let report = GeometryGate::default().choose(&dag);                        // measured-distortion gate
assert_eq!(report.chosen, Geometry::Euclidean);                          // default unless hyperbolic wins
let enc = EuclideanTaxonomyEncoder::new(&dag, /*bag_dim*/ 16);            // the default taxonomy block
let oracle = DisjointnessOracle::mine(&closed.graph);                     // answer-safe disjointness
let kept = oracle.mask_candidates(&query_types, &candidates);            // drops only provably-disjoint
# Ok::<(), String>(())
```

**Honesty.** Mask answer-safety (removes only provably-disjoint, never invents), the metric guard on a
hyperbolic block, and the gate's decision rule are **proven** (`taxonomy.rs` tests). Whether the
taxonomy block (or hyperbolic geometry) raises retrieval is **empirical/dataset-dependent** — no
accuracy claim; the gate adopts non-Euclidean only on **measured** lift. The gUFO rigid/role split
(design §2/§9.5, rare annotations) is the optional/last prior and is **deferred**.

### 18. Flexible minimal-complete grounding — modality chosen per request (opt-in, feature = `structure`)

<!-- [OPUS-4.8] sq-0wo9e.5 (epic sq-0wo9e; design research/structure-aware-vectorisation.md §4). -->
Research-grade **P4**: grounding is a function `(node, graph) -> minimal-and-complete OBJECT` whose
**modality is chosen per request** by a dispatcher on the *consumer's declared output type* — the
same node projected into whichever object a tool needs. `ground` (the `grounding` module) returns a
`Grounding` enum:

- **`Modality::Subgraph`** — the smallest sub-BGP describing the node, bounded to the predicates of
  the node's **effective (minimal) type's** characteristic set (ABSTAT-style minimality; Spahiu et
  al. ESWC 2016, via `sparq-introspect`). Verifiable facts only — every fact is a real triple of the
  graph, never an approximate signal.
- **`Modality::TypedSubVector`** — only the relevant `SchemaHeader` blocks of the node's stored
  vector (e.g. just the numeric block). Minimal by construction. Also returns each kept block's
  `weights` entry — the per-block fusion multiplier for `fuse_rrf_weighted` (`1.0` fail-open; see
  §14 sq-w2af4).
- **`Modality::NlString`** — the token-budgeted `verbalize` passage, optionally **extended to render
  typed values** (unit-typed quantities + enum labels) via `render_typed_values`.
- **`Modality::TypedValue`** — a single typed slot filled directly: `TypedValue::{Boolean, Number,
  Quantity, Enum}`. **Exact** (no cosine threshold, no recall loss).

<!-- [OPUS-4.8] sq-t80n4: cross-unit reconciliation (consumes the P2 units.rs table, sq-0wo9e.3). -->
**Cross-unit reconciliation (opt-in `GroundingConfig::reconcile_units`, default off).** A quantity
renders **as declared** by default. Set `reconcile_units` and each quantity whose unit is **known** to
the P2 table (recipe 16, `units::normalise`) is rendered in the **canonical SI unit of its
`QuantityKind`** — so `1 mi` and `1.609344 km` (both `Length`) collapse to the same `1609.344 M`, and
`0 °C` / `273.15 K` to the same `273.15 K`, before grounding / comparison / NL render. **Conservative
on unknown:** an unknown or **compound / rate** unit (`unit:KiloM-PER-HR` = km/h is *not* in the
simple-unit table) is left as declared — never a fabricated conversion; only same-`QuantityKind` units
reconcile. `reconcile_quantity(TypedValue) -> TypedValue` is the standalone primitive. No accuracy
claim — a correctness/ergonomics feature.

```rust,ignore
# // cargo build -p sparq-vectors --features structure
use sparq_vectors::{ground, Grounding, GroundingConfig, Modality, OutputType};
use sparq_vectors::structure::close_for_vectorise;
use sparq_reason::Profile;

// Completeness is PROFILE-RELATIVE: close the graph FIRST so entailed facts are present.
let g = close_for_vectorise(ttl, "turtle", Profile::Rdfs)?.graph;
let node = oxrdf::NamedNode::new("http://ex/bolt")?.into();

// A dispatcher maps the consumer's declared output type to a modality (ambiguous → subgraph).
let modality = Modality::for_output(OutputType::Facts);
if let Some(Grounding::Subgraph(facts)) =
    ground(&g, &node, modality, &GroundingConfig::default(), None, None)
{
    // each `facts[i]` is a (predicate, object) re-checkable against the graph
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Honesty.** Both minimality (smallest pattern under the *stated* criterion) and completeness
(relative to the *materialised entailment profile* + declared shapes) are **profile-relative, not
absolute** — and **NOT** end-task answer-completeness (no answer-completeness claim is made). This is
the **projection** half only: the "ANN proposes, exact engine re-validates" loop is the
`filtered-ann` / `vec-predicate` path (recipes 8–9). Quantities render **as declared** by default;
opt into **cross-unit reconciliation** via `reconcile_units` (above) — known units canonicalise via
the P2 QUDT `normalise` (recipe 16; sq-0wo9e.3, sq-t80n4), unknown/compound units stay as declared.

### 19. Neuro-symbolic propose-then-verify grounding (opt-in, feature = `neuro-symbolic`)

<!-- [OPUS-4.8] sq-0wo9e.6 (epic sq-0wo9e; design research/structure-aware-vectorisation.md §5 (B) + §6.A). -->
Research-grade **P5**: grounding a tool slot `(subject, predicate, ?)` has two halves with
**deliberately asymmetric guarantees**, and the `verify` module keeps them honestly separate:

- **PROPOSE (neural — recall only, NOT sound).** `propose_by_vector(store, seed, k)` returns the `k`
  nearest neighbours of `seed`'s stored vector as candidate objects. It is a candidate **generator**:
  a returned id may be type-inconsistent, shape-invalid, or wrong for the slot. **No soundness
  claim.**
- **VERIFY (deductive — the soundness GATE).** `verify_candidate` adds the binding to the graph and
  **rejects** it if doing so introduces a **new SHACL violation** (`sparq-shacl`; datatype /
  cardinality / enum / `sh:class` / node-kind) **or** a **new OWL inconsistency**
  (`sparq-reason::inconsistencies` over the materialised closure — `cax-dw` disjoint-class clash,
  `differentFrom`/`sameAs`, max-cardinality-0). A failing candidate is **REJECTED, not down-ranked**,
  and the gate **fails closed** (an uncheckable candidate is never surfaced as verified). The check is
  **differential**: only a defect the base graph did not already have blocks the candidate.

`propose_then_verify` runs both: the returned `VerifyReport::verified()` is **guaranteed a subset of
the proposed candidates** and contains only deductively-admissible bindings — the design's
**verify-shrinks-to-sound** property.

```rust,ignore
# // cargo build -p sparq-vectors --features neuro-symbolic
use sparq_vectors::{propose_then_verify, VerifyConfig};
use sparq_shacl::ShapesModel;

let shapes = ShapesModel::parse(&shapes_graph);
let report = propose_then_verify(
    &graph, &store, /*seed*/ alice, /*subject*/ alice, /*predicate*/ works_on,
    /*k*/ 8, Some(&shapes), &VerifyConfig::default(),
);
for object in report.verified() { /* a vector-proposed binding that PASSED the deductive gate */ }
for v in report.rejected() { /* audit: WHY each unsound proposal was dropped */ }
```

**Honesty.** The neural propose is a high-**recall** generator with **no soundness guarantee**; the
deductive verify is the **only** soundness gate (correctness comes from it, never from the vector
step). *Verify-shrinks-to-sound* is **provable + tested** (the load-bearing test asserts a
vector-proposed-but-shape-invalid candidate is rejected). Whether the neural periphery raises end-task
**recall** is **empirical, dataset-dependent, EC2-gated** (GraphRAG/KG-RAG does not uniformly beat
vector RAG) — measured by the on/off ablation harness (recipe 14; sq-0wo9e.7), **never asserted here**.

### 20. Embedding provenance + mandatory compatibility rejection (`.spqv` v3, opt-in, feature = `spqv-provenance`)

An embedding is a coordinate in **one** model's space. A query vector from a *different* model,
content revision, metric, normalization, dimension, or verbalization regime lands in a *different*
space — its cosine against the stored vectors is arithmetically defined but **semantically wrong**,
with no error. The v2 `.spqv` container recorded only the dimension + a graph fingerprint (a shifted
dictionary, not a shifted embedding space). The **v3** format records the embedding pipeline's
identity in the header and REJECTS an incompatible query. This closes review gap 1 / the spec's
reproducibility obligation (`sq-rvgr2.4`).

```toml
sparq-vectors = { path = "../sparq-vectors", features = ["spqv-provenance"] }
```

```rust,ignore
use sparq_vectors::{EmbeddingMetric, EmbeddingProvenance, LegacyMode, Normalization, VectorStore};

// Record the pipeline that produced the vectors, then WRITE v3 (binding a provenance selects v3;
// without it the writer emits v2 exactly as before — the default on-disk format is unchanged):
let mut prov = EmbeddingProvenance::new("text-embedding-3-small", EmbeddingMetric::Cosine, Normalization::L2);
prov.content_version = "verb-v2".into();          // the graph→text (verbaliser) revision
prov.verbalization = "entity-verbalized".into();
let mut store = VectorStore::create("graph.spqv", 384)?.with_provenance(prov.clone());
// … put/embed vectors …
store.finalize()?;                                // written v3, provenance in the header

// At QUERY time, declare the embedder that produced the query vector and CHECK before searching:
let store = VectorStore::open("graph.spqv")?;     // v3 opens on ANY build (the read path is always compiled)
store.check_provenance(&prov, LegacyMode::Reject)?;   // Ok iff every DEFINED axis matches; else a descriptive Err
// A WRONG model id / metric / normalization / verbalization is REJECTED (fail-closed) — a distinct
// error per axis; dimension is enforced structurally (a wrong-width query vector errs on search/put).
# Ok::<(), String>(())
```

**Legacy stores (fail-closed default).** A v2/v1 store carries no provenance (`store.provenance()`
is `None`), so `check_provenance(.., LegacyMode::Reject)` REJECTS it — the embedding pipeline is
unverifiable. A caller that **knows** the store is compatible passes `LegacyMode::Allow` to bypass
the check **for a legacy store only** (a v3 store is always checked against its recorded provenance).
`compact` (the `delta` feature) carries the provenance forward, so a v3 store stays v3.

**KERN boundary (do not extend ahead of the profile freeze).** `EmbeddingProvenance::reserved` is a
versioned **opaque** byte area — *extension fields reserved pending the cross-implementation profile
(#1746)*. **No** fields are defined over it (no encoder-version-hash, no codebook-hash, no `D`
semantics); it round-trips byte-for-byte and does **not** participate in `compatible_with`, so a v3
store written by a future implementation that populated it stays queryable here. The format defines
and privileges **no** encoder — the string axes are caller-supplied opaque tokens.

**Honest conformance note (closes review gap 1; spec estate `sq-rvgr2.4`).** Revision 2 of the vector
spec normatively requires **embedding provenance + rejection of incompatible queries**. The v2
container was knowingly **non-conforming** on that reproducibility obligation (it persisted only the
dimension + graph fingerprint). The v3 format + the mandatory `check_provenance` **close that gap** for
the DEFINED axes. The extensible-provenance profile itself (the `#1746` reserved-area semantics) is
NOT frozen, so the reserved area stays opaque here — it is the remaining, deliberately-deferred, part.

### 21. Carry opaque metadata beside exact-search hits (opt-in, feature = `metadata-sidecar`)

Use the metadata sidecar when each vector needs a caller-owned label, partition token, or
post-filtering tag without a second id-to-tag lookup table. Tags do not affect similarity or rank.

```toml
sparq-vectors = { path = "../sparq-vectors", features = ["metadata-sidecar"] }
```

```rust,ignore
use sparq_vectors::{nearest_exact_with_meta, VectorStore};

let mut store = VectorStore::create("graph.spqv", 2)?;
store.put_with_meta(10, &[1.0, 0.0], "tenant-a")?;
store.put(20, &[0.0, 1.0])?; // untagged is valid
store.finalize()?;

let hits = nearest_exact_with_meta(&store, &[1.0, 0.0], 2);
assert_eq!(hits[0].2.as_deref(), Some("tenant-a"));
assert_eq!(store.meta(20), None);

let reopened = VectorStore::open_from_bytes(std::fs::read("graph.spqv")?)?;
assert_eq!(reopened.meta(10), Some("tenant-a"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

The first `put_with_meta` selects `.spqv` v4. A build that only calls `put` still emits the existing
v2 format, or v3 when embedding provenance is bound, even when `metadata-sidecar` is enabled. Tagged
and untagged vectors can be mixed; an empty string is a present tag (`Some("")`), distinct from
`None`. The metadata is returned only after `nearest_exact` has fixed the top-`k`, so the `(id,
score)` sequence and deterministic tie-break are identical to the undecorated search. The v4 reader
and writer are both feature-gated; enable `metadata-sidecar` in every process that opens a tagged
store. A feature-off build continues to support the pre-existing v1/v2/v3 formats only.

### 22. Recall-gated concept dedup + k-NN over raw vectors (opt-in, feature = `approx-ann`) [FABLE-5] #2251

For a **raw concept-vector matrix** (no sparq `Graph` / `VectorStore` — e.g. the
Kernel-of-Truth's structure-aware concept vectors) that needs **type-level near-duplicate
merging at a scale where exact O(m²) dedup is intractable**. The contract: build one exact
O(m²) ground truth at a rung where that is still tractable (~10⁵), then let ANN carry the
larger rungs — but every merge pass first **proves, on that ground truth, that the index's
recall clears a pre-registered gate** (default `0.99`). Below the gate `dedup` is an `Err`
and **no merge is emitted** (fail-closed: an under-recalling index silently *misses* true
duplicates, which is a correctness loss, not a graceful degradation).

```rust,ignore
# // cargo build -p sparq-vectors --features approx-ann
use sparq_vectors::{build_ann, dedup, exact_ground_truth, knn, DedupPolicy, HnswConfig};

let vectors: Vec<(u32, Vec<f32>)> = /* (concept id, vector) pairs, any consistent dim */;

// ONE-OFF at the tractable rung: the exact O(m^2) oracle (or wrap your own external
// oracle with GroundTruth::new(k, pairs)).
let truth = exact_ground_truth(&vectors, 10)?;

// Every rung: build the ANN index (HnswConfig is the build policy; deterministic per seed)…
let index = build_ann(&vectors, HnswConfig::high_recall())?;

// …and dedup under ONE FROZEN policy. The recall gate runs FIRST; merges are computed
// only after it passes.
let policy = DedupPolicy { recall_gate: 0.99, merge_threshold: 0.995, k: 10 };
let report = dedup(&index, &policy, &truth)?;   // Err (no merges) below the gate
println!("recall {:.4}", report.recall);        // >= 0.99, or we wouldn't be here
for (dup, canonical) in &report.merges { /* merge dup -> canonical (smallest id in group) */ }

// Plain k-NN over the same index (crosstalk/collision checks, composition):
let hits = knn(&index, &query, 10);             // Vec<(Id, f32 cosine)>, best first
# Ok::<(), String>(())
```

**Honest scope.** The gate is **evidence on the measured corpus**, not a universal recall
guarantee — a different corpus, dimension, vectoriser, or policy needs its own ground
truth; re-gate after any re-vectorisation. `merge_threshold = 0.995` is a plain default,
NOT a canonical value (the right threshold is a property of the consumer's vectoriser);
**freeze one `DedupPolicy` per scale track** and report per-rung metrics under it — the
issue-#2251 protocol. Groups are the union-find transitive closure of above-threshold
neighbour pairs, so `policy.k` bounds the per-vector merge fan-in; the canonical id is the
group's smallest (deterministic). Misconfigurations are `Err`, never silent: a `k` the
build `ef_search` beam cannot serve, a ground-truth id absent from the index, a
gate/threshold outside its range, an all-zero/non-finite/duplicate-id input.

## Gotchas / feature flags / prerequisites

- **Opt-in.** Nothing in the workspace depends on `sparq-vectors`; the default engine
  build does not compile it. The core Rust flow (store/ANN/embed) is a standalone library
  over sparq-core's public read API — you wire it into application code yourself. The **one**
  SPARQL-level integration is the optional `vec:` magic predicate (recipe 8 below), behind
  the **non-default `vec-predicate`** feature; with it OFF this crate has zero `sparq-engine`
  dependency and the base engine/core query path is byte-identical (see recipe 8). There is
  still no `SERVICE`/function binding. Predicate-constrained (filtered) ANN (recipe 9) is a
  separate **non-default `filtered-ann`** feature — also lean (no new dep, no engine pull).
- **Incremental add/remove/update is the non-default `delta` feature (sq-pi44, persistence sq-7e50).**
  Also lean (no new dep, no engine pull). With it OFF the store is build-once-immutable as before
  (`put` errors after `finalize`; no `add`/`remove`/`update`/`compact`/`save_delta`); with it ON those
  methods write a delta the read paths consult transparently (recipe 12). The delta lives in RAM on the
  handle; `save_delta` persists it to a crash-durable `.spqd` sidecar and `open_with_delta` replays it,
  so mutations survive a restart without a `compact` (the two durability paths).
- **Structure-aware preprocessing is the non-default `structure` feature (sq-0wo9e.1 P0, sq-0wo9e.2 P1, sq-0wo9e.3 P2, sq-0wo9e.4 P3, sq-0wo9e.5 P4).**
  It is the ONLY feature pulling `sparq-reason` + `sparq-introspect` into this crate, both **optional**, so
  with it OFF the default build compiles zero structure-prep code and gains no new required deps. With
  it ON it exposes the **P0** closure + sampler (`close_for_vectorise` / `materialise_closure` /
  `ClosedGraph`, `TypeConstraints`, `NegativeSampler` + `SamplingMode`, recipe 13), the **P1**
  typed-literal encoders (`route`, `NumericEncoder`, `BooleanEncoder`, `DateEncoder`, `SchemaHeader`,
  recipe 15), the **P2** enum `Codebook` + QUDT unit-`normalise` (recipe 16), the **P3** taxonomy
  block + disjointness (`TaxonomyDag`, `EuclideanTaxonomyEncoder`, `GeometryGate`, `DisjointnessOracle`,
  recipe 17), AND the **P4** flexible-grounding selector (`ground` / `Grounding` / `Modality` /
  `OutputType` / `TypedValue`, recipe 18). `structure` itself serves no exact answer; the disjointness
  mask is **answer-safe** (drops only provably-disjoint), encoder invariants are proven, and grounding
  minimality/completeness are **profile-relative**, but embedding-quality benefit is **unproven**
  (research-grade, no accuracy claim, no canonical numbers).
- **The SHACL/OWL prior reader is the non-default `structure-shacl` feature (sq-0wo9e.3 P2).** It
  **implies `structure`** and is the ONLY feature pulling `sparq-shacl` (and transitively, on native,
  the engine) into this crate's graph — so neither the default build nor the lean `structure` feature
  gains a SHACL/engine dependency. With it ON it exposes `ShaclPriors` / `PredicatePrior` /
  `Cardinality` (recipe 16): a **read-only** reader that mines enum/datatype/cardinality priors out of
  a parsed `ShapesModel` (no SHACL behaviour change). The enum codebook + QUDT normaliser themselves
  ship under plain `structure` (pure, no SHACL dep) — only the *reader* needs `structure-shacl`.
- **The neuro-symbolic propose-then-verify pipeline is the non-default `neuro-symbolic` feature (sq-0wo9e.6 P5).**
  It **implies `structure-shacl`** (and thus `structure`), so it reuses exactly the `sparq-shacl` +
  `sparq-reason` + `sparq-introspect` deps those features already pull and adds **no new dependency**.
  With it OFF the default build compiles zero pipeline code; with it ON it exposes `propose_by_vector`
  / `verify_candidate` / `verify_candidates` / `propose_then_verify` + `VerifyReport` / `Verdict` /
  `Rejection` / `Source` / `VerifyConfig` (recipe 19). The neural propose is **recall only (NOT
  sound)**; the deductive verify is the soundness gate — a failing candidate is **rejected**
  (fail-closed), the verified set only **shrinks to a sound subset**, and **no recall/accuracy number
  is claimed** (empirical, EC2-gated).
- **The KGE measurement foundation is the non-default `kge` feature (sq-0wo9e.8 / P6).** It **implies
  `structure`** and adds **no new dependency** (a hand-rolled CPU-only trainer — no ML crate). With it
  OFF the default build compiles zero trainer/eval code; with it ON it exposes `train` / `TrainConfig`
  / `TrainedModel` / `ModelKind` (DistMult **or** the asymmetric ComplEx), the filtered
  link-prediction harness (`run_ablation` / `run_ablation_multiseed` / `EvalConfig` / `Splits` /
  `Metrics` / `LongTail` / `AblationCell` / `MultiSeedCell` / `CellStats` / `MeanStd`), the
  RDF 1.2 triple-term visibility surface (`TermScope`, `TrainConfig::term_scope`,
  `run_quoted_ablation`, `QuotedAblation`, `synthetic_rdf12_ttl`, `synthetic_rdf12_parts`), the
  synthetic-graph generators, and `SCHEMA_PREDICATES` (recipe 14). It is the *measurement instrument*
  the design requires before any prior is adopted — **no accuracy claim**, indicative numbers only
  (work-box non-canonical). **DistMult is symmetric → near-random on directional relations; read
  ablation deltas off the asymmetric ComplEx, multi-seed.**
- **`approx-ann` is the ONLY heavy ANN dependency, and it is OFF by default (sq-ip3a).** The HNSW
  index (`VectorIndex`/`HnswConfig`), the `ApproxBackend`, and the recall-gated concept-ANN dedup
  surface (`build_ann`/`knn`/`dedup`, recipe 22 — [FABLE-5] #2251) are gated behind it — it is the
  only thing pulling `instant-distance`. With it OFF the default build is lean: exact brute-force
  (`nearest_exact`, answer-exact) + the hand-rolled on-disk Vamana graph (`DiskAnnIndex`, no extra
  dep). Approximate search is **APPROXIMATE** — recall < 1.0, NOT answer-exact (recipes 2 & 11);
  the dedup surface's recall gate is measured evidence against an exact oracle, not exactness.
- **Reference-library verification (sq-6te5).** The recall floors are anchored not only against
  this crate's own exact searcher but against an **established ANN library** (hnswlib/FAISS):
  `tests/ref_lib_verify.rs` loads a committed fixture (`tests/fixtures/hnswlib_ref.tsv`, a REAL
  hnswlib capture) and asserts (a) `nearest_exact` reproduces numpy's exact-kNN oracle exactly —
  same metric, same corpus — and (b) sparq-vectors' DiskANN (and HNSW under `approx-ann`) clears
  hnswlib's OWN recall on the shared oracle. **This runs in CI with no native deps** — the corpus
  is regenerated from a splitmix64 seed on both sides (bit-identical Python and Rust), so the
  fixture is just neighbour ids. The **live** hnswlib re-capture
  (`scripts/capture_hnswlib_ref.py`, needs numpy + hnswlib) is `#[ignore]`d / **gather-only**: run
  `cargo test -p sparq-vectors --test ref_lib_verify -- --ignored` (optionally
  `VECTOR_PY=<venv>/bin/python`). Recall numbers are work-box NON-CANONICAL and are recomputed off
  the ids, never baked.
- **`HashEmbedder` is TEST-ONLY.** It is lexical n-gram hashing with **no semantics**
  ("car" and "automobile" are unrelated). Use it to exercise the store/ANN/pipeline; bring
  a real `Embedder` for actual retrieval.
- **Live embeddings — two opt-in tiers, default build is socket-free.** For a real
  OpenAI-compatible `/v1/embeddings` endpoint:
  - **`provider`** (non-default) carries the API shape only — it builds the request body and
    parses the response but **never opens a socket**; you supply a `Transport` impl over your
    HTTP client (reqwest/ureq/recorded cassette). `cargo build -p sparq-vectors --features provider`.
  - **`embeddings`** (non-default; enables `provider` + a blocking reqwest `Transport`) needs
    **no caller glue**: `RemoteEmbedder::from_env(dim)` reads `SPARQ_EMBEDDINGS_API_KEY`
    (required), `SPARQ_EMBEDDINGS_BASE_URL` (default `https://api.openai.com/v1/embeddings`),
    and `SPARQ_EMBEDDINGS_MODEL` (default `text-embedding-3-small`), then `embed_entities`
    works against the live endpoint. Mirrors `sparq-nlq`'s `live` feature; requests carry a
    hard timeout. `cargo build -p sparq-vectors --features embeddings`.
  The **default build pulls no HTTP client** (no reqwest) and the crate never enters the wasm
  bundle, so neither tier affects the default or wasm builds.
- **Keys are dictionary term ids** (`sparq_core::dict::Id`, a `u32`). Resolve a `Term` with
  `graph.id_of(&term) -> Option<Id>` and an `Id` back with `graph.dict.term(id)`. Coverage
  is **sparse by design** — only entities get vectors, not every literal.
- **Graph scoping (sq-quuu).** Every `&Graph` entry point (`verbalize`, `embed_entities`,
  `embed_labels`, `nearest_term*`) reads the store of the `Graph` it is handed — the
  **default graph** for the top-level `&graph`, or a **single named graph** when you pass
  that graph's sub-`Graph`. On a quad dataset (N-Quads / TriG via `Graph::load_dataset`)
  each named graph is a self-contained `Graph`; fetch one by name with
  `graph.named_graph(&name)` and build a store **per graph** (or over the default graph).
  Verbalization labels never cross graphs and there is no union-of-all-graphs mode — the
  quads are not merged for you. Because keys are dict ids and a named graph has its **own**
  dictionary, a store is bound to **one** `Graph`: the per-named-graph store and the
  default-graph store carry distinct fingerprints, so `check_graph` catches a cross-graph
  mix-up.
- **Dim must match.** `VectorStore::create(path, dim)` fixes `dim`; `embed_*` errors if
  `embedder.dim() != store.dim()`. `embed_labels`/`embed_entities` leave the store
  **unfinalized** — call `store.finalize()` before opening/searching (or before a fresh
  `VectorStore::open`).
- **Degenerate vectors:** `put` rejects all-zero and non-finite vectors (cosine is
  undefined); a zero **query** returns no results from every searcher, so exact/HNSW/DiskANN
  agree on the degenerate case.
- **HNSW is per-process.** `VectorIndex` is rebuilt from the mmap on `build` (~33–83 s for
  50k×32 on an M1). Use `DiskAnnIndex` for a persistent index that reopens with no rebuild;
  its **build** is single-threaded (slower than the rayon HNSW build) — only the **open** is
  cheap. `ef_search` / `search_beam` must be ≥ the `k` you query.
- **DiskANN honest scope:** `DiskAnnIndex` searches full-precision vectors straight from the
  mmap by default. The PQ-compressed in-RAM candidate cache that *full* DiskANN ranks on
  (`quant.rs`) exists, is tested, and **is now wired into** `search_slots` (sq-qamd / #620):
  when a PQ cache is present, `search_slots` dispatches to `search_slots_pq` — rank on the
  RAM-resident codes, re-rank the final beam off the mmap. Build it via
  `DiskAnnIndex::build_with_pq` and check `has_pq_cache()`; recipe 4 shows the same loop the
  index now drives internally. <!-- [OPUS-4.8] de-staled: PQ cache wired into search_slots (sq-qamd/#620) -->
  Without a PQ cache, search stays full-precision-from-mmap as above.
- **Filtered ANN (`filtered-ann` feature):** predicate-constrained search returns only ids in the
  `IdMask`, and **every returned id is guaranteed in the mask**. An **empty** mask -> no results
  (distinct from passing no mask = an unfiltered search); a **full** mask -> identical to the
  unfiltered search. `DiskAnnIndex::nearest_filtered` is **exact** when it pre-filters (selective
  mask) and **approximate** when it traverses (broad mask) — but pre-filter is exact, so erring
  toward it (the conservative default crossover) only costs mild throughput, never recall. The
  in-RAM `VectorIndex::nearest_filtered` is **always** the exact pre-filter (instant-distance
  adjacency is not exposed, so the HNSW graph cannot be walked with predicate-aware acceptance);
  use `DiskAnnIndex` for filtered traversal over a broad mask. Lean feature: no new dependency, no
  engine pull. NON-CANONICAL timing.
- **Little-endian only.** `.spqv`/`.spqg` reject big-endian targets at create/open.
- **Determinism:** ties break on ascending id (searchers) or first appearance (fusion);
  HNSW/Vamana/PQ seeds are fixed by default so builds are reproducible.

## See also

- `sparql-formal-semantics` — the SPARQL algebra the engine evaluates (vectors are not part
  of it).
- `hdt-format`, `fused-decompress-parse`, `rust-parallel-parsing` — getting RDF into the
  `Graph` you then embed.
- `mpc-protocols`, `noir-circuit-patterns` — unrelated sibling skills in this workspace.
