//! Provenance **weight extractor** `w(t)` — read a per-triple **provenance-quality weight**
//! `w(t) ∈ (0, 1]` out of a graph's PROV-O / DQV annotations into the layout the structure-aware
//! KGE trainer consumes (provenance-driven GenAI-KB **Phase 4**, design
//! `research/provenance-driven-genai-kb.md` §USE-1 / §5 Phase 4, epic `sq-2489d`).
//!
//! [OPUS-4.8] sq-2489d.4. This **mirrors `crate::shacl_priors`** (the P2 SHACL/OWL prior
//! extractor): a small, **read-only** reader keyed by graph term ids that consumes only
//! `sparq-core`'s public id-scan API. It does **not** redefine the provenance model — it
//! consumes the same `pkg:confidence` / `pkg:assurance` / `prov:wasDerivedFrom` terms the
//! Phase-1 join (`sparq-nlq`'s `provenance` module) reads and the Phase-3 DQV terms `pkg.ttl`
//! declares. Crucially it does this **without pulling the engine** (unlike the SPARQL-backed
//! Phase-1 join): the `structure` feature stays lean — this reader is a direct id-level scan,
//! exactly like `crate::shacl_priors`'s `functional_properties` axiom reader.
//!
//! # What `w(t)` combines (design §USE-1)
//!
//! `w(t)` is provenance **about the triple `t` itself** wherever the graph expresses it, and falls
//! back to the head otherwise:
//!
//! 1. **Statement-level (the design's `w(t)` proper).** If the graph *reifies* the triple — RDF 1.2
//!    `:r rdf:reifies <<( h r t )>>`, or RDF 1.1 standard reification (`rdf:subject` /
//!    `rdf:predicate` / `rdf:object`) — and the reifier carries provenance, then `w(t)` is that
//!    **reifier's** weight (the MIN over reifiers, mirroring the min-over-sources rule: a statement
//!    is only as trustworthy as its least-trustworthy annotation).
//! 2. **Head-level fallback** (no reifier, or no provenance on it): the **head** entity `h`'s own
//!    provenance. This is an *approximation* — it says `h` is a low-assurance entity, not that this
//!    particular assertion is doubtful — and it is the only signal a plain RDF-1.1 graph with no
//!    reification can offer. It is fail-open, never invented.
//!
//! Either way the scalar is built from the same annotations, read off whichever subject qualifies
//! (the reifier, or `h`):
//!
//! | provenance fact | contribution |
//! |---|---|
//! | `pkg:confidence "c"` on the subject (epistemic weight, `0..1`) | the **confidence factor** `c` |
//! | `pkg:confidence "c"` on its `prov:wasDerivedFrom` source (source reliability) | folded in as the source-reliability factor (min over sources) |
//! | `pkg:assurance secx:Proven` | high **assurance multiplier** ([`WeightConfig::proven`]) |
//! | `pkg:assurance secx:Claimed` | mid multiplier ([`WeightConfig::claimed`]) |
//! | `pkg:assurance secx:Conjectured` | low multiplier ([`WeightConfig::conjectured`]) |
//! | *no provenance at all* | `w(t) = 1.0` (fail-open: a plain graph is **unchanged**) |
//!
//! `w(t) = clamp( assurance_mult · confidence_factor · source_reliability , floor , 1.0 )`, with a
//! configurable `floor > 0` so a weight is never zero (a zero would silently delete a positive from
//! the loss — the honest behaviour is "down-weight, never drop").
//!
//! # The on/off ablation (load-bearing — design §5 Phase 4)
//!
//! Provenance-weighting ships **behind an on/off ablation** and is adopted **only on measured
//! lift**. [`WeightMode`] is that switch: [`WeightMode::Uniform`] returns `1.0` for **every** triple
//! (the ablation **off** baseline — identical to today's unweighted trainer);
//! [`WeightMode::Provenance`] applies the `w(t)` above. A harness runs the *same* trainer in both
//! modes and compares filtered Hits@k / MRR (see `crate::eval::run_weight_ablation` under the `kge`
//! feature). **No accuracy claim is made here** — the literal/confidence-aware KGE line reports
//! *inconsistent* gains, so any lift is unproven and dataset-dependent; the bead's acceptance bar is
//! "adopt only if the measured lift clears a pre-registered bar, and ABANDON otherwise".
//!
//! # Honesty
//!
//! Every input is **read from the graph, never invented**. A head with no provenance simply has
//! `w(t) = 1.0` (fail-open). The weight only ever *scales a training-loss contribution*; it changes
//! no exact answer and is never surfaced as a confidence number to a user.

use rustc_hash::FxHashMap;
use sparq_core::dict::{Id, TermParts, NO_ID};
use sparq_core::Graph;

/// `pkg:confidence` — the numeric confidence (`0..1`) on a Finding (epistemic weight) or a Source
/// (reliability). Mirrors `sparq-nlq`'s `provenance::PKG_CONFIDENCE`.
pub const PKG_CONFIDENCE: &str = "https://sparq.dev/ns/pkg#confidence";
/// `pkg:assurance` — the epistemic-basis axis (`secx:Proven` / `Claimed` / `Conjectured`).
pub const PKG_ASSURANCE: &str = "https://sparq.dev/ns/pkg#assurance";
/// `prov:wasDerivedFrom` — the PROV-O lineage edge to a fact's source (its sub-property
/// `pkg:discoveredFrom` is read transparently because the closure materialises it as the
/// super-property, but the super-property IRI is what is scanned here).
pub const PROV_WAS_DERIVED_FROM: &str = "http://www.w3.org/ns/prov#wasDerivedFrom";
/// `pkg:discoveredFrom` — the PKG-native lineage sub-property of `prov:wasDerivedFrom`. Scanned in
/// addition to the super-property so the source-reliability factor resolves on an un-closed graph
/// (where the sub→super entailment has not been materialised).
pub const PKG_DISCOVERED_FROM: &str = "https://sparq.dev/ns/pkg#discoveredFrom";

/// `rdf:reifies` — the RDF 1.2 reifier → triple-term edge (`:r rdf:reifies <<( s p o )>>`). The
/// reifier `:r` is an ordinary entity, so its own `pkg:confidence` / `pkg:assurance` /
/// `prov:wasDerivedFrom` annotations are provenance about **the statement**, which is what the
/// design's per-triple `w(t)` is defined over. [OPUS-5] sq-w2af4.
pub const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
/// `rdf:subject` — RDF 1.1 standard reification, the plain-RDF spelling of the same
/// statement → provenance mapping (`:st rdf:subject :s ; rdf:predicate :p ; rdf:object :o`).
pub const RDF_SUBJECT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#subject";
/// `rdf:predicate` — RDF 1.1 standard reification (see [`RDF_SUBJECT`]).
pub const RDF_PREDICATE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#predicate";
/// `rdf:object` — RDF 1.1 standard reification (see [`RDF_SUBJECT`]).
pub const RDF_OBJECT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#object";

/// The three `secx:` epistemic-basis IRIs (`pkg:assurance` objects). The `secx:` namespace is the
/// sec-prop extension the PKG reuses for the assurance axis; the design records both the
/// `sparq.dev/ns/secx#` form (used by the Phase-1 fixtures) and the canonical `w3id.org` form.
const SECX_PROVEN: [&str; 2] =
    ["https://sparq.dev/ns/secx#Proven", "https://w3id.org/zkp-sparql/sec-prop#Proven"];
const SECX_CLAIMED: [&str; 2] =
    ["https://sparq.dev/ns/secx#Claimed", "https://w3id.org/zkp-sparql/sec-prop#Claimed"];
const SECX_CONJECTURED: [&str; 2] =
    ["https://sparq.dev/ns/secx#Conjectured", "https://w3id.org/zkp-sparql/sec-prop#Conjectured"];

/// The **ablation switch** for provenance-weighting (design §5 Phase 4). The trainer runs
/// identically under both modes except for the per-positive weight, so a harness can measure
/// provenance-weighted vs uniform on the *same* graph, seed, and negatives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightMode {
    /// Every triple weighs `1.0` — the ablation **off** baseline, byte-identical to the unweighted
    /// trainer (no provenance is read at all).
    Uniform,
    /// Apply the provenance-quality weight `w(t)` of [`ProvenanceWeights`] — the ablation **on** arm.
    Provenance,
}

/// Tunable factors for the `w(t)` combination. The defaults follow design §USE-1
/// (`secx:Proven` → high, `Claimed` → mid, `Conjectured` → low) and are exposed so the ablation
/// harness can sweep them; they are **not** canonical and carry no accuracy claim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeightConfig {
    /// Assurance multiplier for `pkg:assurance secx:Proven`. Default `1.0`.
    pub proven: f32,
    /// Assurance multiplier for `pkg:assurance secx:Claimed`. Default `0.7`.
    pub claimed: f32,
    /// Assurance multiplier for `pkg:assurance secx:Conjectured`. Default `0.4`.
    pub conjectured: f32,
    /// Assurance multiplier when `pkg:assurance` is **absent** (unknown basis). Default `1.0`
    /// (fail-open — a missing axis never *penalises*; only an explicit lower-assurance axis does).
    pub default_assurance: f32,
    /// The lower clamp on `w(t)` (`> 0`), so a positive is never *dropped* from the loss (a zero
    /// weight would silently delete it). Default `0.05`.
    pub floor: f32,
}

impl Default for WeightConfig {
    fn default() -> WeightConfig {
        WeightConfig { proven: 1.0, claimed: 0.7, conjectured: 0.4, default_assurance: 1.0, floor: 0.05 }
    }
}

/// Per-entity provenance facts mined from a graph: the `pkg:confidence` of each subject, its
/// `pkg:assurance` basis, and the source ids it was `prov:wasDerivedFrom`. Keyed by graph term id.
/// This is the input to [`ProvenanceWeights::weight_of`].
///
/// Built by a single id-level scan of the graph (no engine, no SPARQL) — the same shape as
/// `crate::shacl_priors`'s `functional_properties` axiom reader.
#[derive(Clone, Debug, Default)]
pub struct ProvenanceWeights {
    /// `pkg:confidence` decimal of each subject id (`0..1`), when asserted and parseable.
    confidence: FxHashMap<Id, f32>,
    /// `pkg:assurance` basis of each subject id, when asserted.
    assurance: FxHashMap<Id, Assurance>,
    /// The source ids each subject id was derived from (`prov:wasDerivedFrom` / `pkg:discoveredFrom`).
    derived_from: FxHashMap<Id, Vec<Id>>,
    /// **Statement-level** provenance: for each asserted triple the graph *reifies*, the reifier
    /// ids that themselves carry provenance. Reifiers with no annotation of their own are dropped
    /// here (they say nothing), so a non-empty entry always means real per-statement provenance.
    /// Empty on any graph without reification — the head fallback then covers every triple.
    /// [OPUS-5] sq-w2af4.
    statement: FxHashMap<[Id; 3], Vec<Id>>,
    config: WeightConfig,
}

/// The epistemic basis read off a `pkg:assurance` edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Assurance {
    Proven,
    Claimed,
    Conjectured,
}

impl ProvenanceWeights {
    /// Mine provenance facts from `graph` with the default [`WeightConfig`]. The graph should
    /// already be **closed** (call [`materialise_closure`](crate::structure::materialise_closure)
    /// first) so an entailed `prov:wasDerivedFrom` (from a `pkg:discoveredFrom` sub-property) is a
    /// real triple — but the reader scans the sub-property directly too, so it also works on an
    /// un-closed graph.
    pub fn mine(graph: &Graph) -> ProvenanceWeights {
        Self::mine_with(graph, WeightConfig::default())
    }

    /// Mine provenance facts from `graph` with an explicit [`WeightConfig`].
    pub fn mine_with(graph: &Graph, config: WeightConfig) -> ProvenanceWeights {
        let conf_pred = graph.id_of(&iri(PKG_CONFIDENCE));
        let assurance_pred = graph.id_of(&iri(PKG_ASSURANCE));
        let derived_pred = graph.id_of(&iri(PROV_WAS_DERIVED_FROM));
        let discovered_pred = graph.id_of(&iri(PKG_DISCOVERED_FROM));
        let reifies_pred = graph.id_of(&iri(RDF_REIFIES));
        let reif_subj_pred = graph.id_of(&iri(RDF_SUBJECT));
        let reif_pred_pred = graph.id_of(&iri(RDF_PREDICATE));
        let reif_obj_pred = graph.id_of(&iri(RDF_OBJECT));

        // Resolve the assurance-object ids up front (None when the graph never mentions them).
        let proven = first_present(graph, &SECX_PROVEN);
        let claimed = first_present(graph, &SECX_CLAIMED);
        let conjectured = first_present(graph, &SECX_CONJECTURED);

        let mut confidence: FxHashMap<Id, f32> = FxHashMap::default();
        let mut assurance: FxHashMap<Id, Assurance> = FxHashMap::default();
        let mut derived_from: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        // Reification, collected raw in the same pass and resolved after it: at scan time we do not
        // yet know which reifiers carry provenance of their own. `reifies` keeps graph order (a
        // Vec, not a map) so the mined result never depends on hash iteration order.
        let mut reifies: Vec<([Id; 3], Id)> = Vec::new();
        let mut reif_parts: FxHashMap<Id, [Id; 3]> = FxHashMap::default();

        for [s, p, o] in graph.iter_ids() {
            if Some(p) == conf_pred {
                // A subject may (malformedly) carry two confidences; the first parseable wins and
                // later ones are ignored — deterministic, never a panic.
                if let std::collections::hash_map::Entry::Vacant(e) = confidence.entry(s) {
                    if let Some(c) = literal_f32(graph, o) {
                        e.insert(c.clamp(0.0, 1.0));
                    }
                }
            } else if Some(p) == assurance_pred {
                let basis = if Some(o) == proven {
                    Some(Assurance::Proven)
                } else if Some(o) == claimed {
                    Some(Assurance::Claimed)
                } else if Some(o) == conjectured {
                    Some(Assurance::Conjectured)
                } else {
                    None
                };
                if let Some(b) = basis {
                    assurance.entry(s).or_insert(b);
                }
            } else if Some(p) == derived_pred || Some(p) == discovered_pred {
                derived_from.entry(s).or_default().push(o);
            } else if Some(p) == reifies_pred {
                // RDF 1.2: the object IS the triple term, stored structurally as its three ids.
                if let TermParts::Triple(ids) = graph.dict.term_parts(o) {
                    reifies.push((ids, s));
                }
            } else if Some(p) == reif_subj_pred {
                reif_parts.entry(s).or_insert([NO_ID; 3])[0] = o;
            } else if Some(p) == reif_pred_pred {
                reif_parts.entry(s).or_insert([NO_ID; 3])[1] = o;
            } else if Some(p) == reif_obj_pred {
                reif_parts.entry(s).or_insert([NO_ID; 3])[2] = o;
            }
        }

        let mut pw = ProvenanceWeights {
            confidence,
            assurance,
            derived_from,
            statement: FxHashMap::default(),
            config,
        };
        // Resolve reification now that every entity's own annotations are known: a reifier that
        // carries NO provenance says nothing about the statement, so it is dropped rather than
        // fail-opening the triple to 1.0 and masking the head fallback.
        for (triple, reifier) in reifies {
            if pw.has_provenance(reifier) {
                pw.statement.entry(triple).or_default().push(reifier);
            }
        }
        for (stmt, parts) in &reif_parts {
            if parts.contains(&NO_ID) || !pw.has_provenance(*stmt) {
                continue; // an incomplete or unannotated reification is not statement provenance
            }
            pw.statement.entry(*parts).or_default().push(*stmt);
        }
        pw
    }

    /// The configured factors (so a harness can read what it swept).
    pub fn config(&self) -> &WeightConfig {
        &self.config
    }

    /// The number of subjects with at least one mined provenance fact (confidence / assurance /
    /// lineage). `0` ⇒ the graph carries no provenance and every `w(t)` is `1.0`.
    pub fn annotated_subjects(&self) -> usize {
        let mut seen: rustc_hash::FxHashSet<Id> = rustc_hash::FxHashSet::default();
        seen.extend(self.confidence.keys().copied());
        seen.extend(self.assurance.keys().copied());
        seen.extend(self.derived_from.keys().copied());
        seen.len()
    }

    /// The number of **triples** the graph carries statement-level provenance for — a reified
    /// statement (RDF 1.2 `rdf:reifies` or RDF 1.1 `rdf:subject`/`rdf:predicate`/`rdf:object`)
    /// whose reifier is itself annotated. `0` ⇒ no per-statement signal exists and every `w(t)`
    /// comes from the head fallback, so any *within-subject* weighting (e.g. pooling one node's
    /// edges) is an exact no-op. Read this before claiming a pooling result is provenance-driven.
    /// [OPUS-5] sq-w2af4.
    pub fn annotated_statements(&self) -> usize {
        self.statement.len()
    }

    /// Is the graph entirely free of provenance (so weighting is a no-op `1.0` everywhere)?
    pub fn is_empty(&self) -> bool {
        self.confidence.is_empty()
            && self.assurance.is_empty()
            && self.derived_from.is_empty()
            && self.statement.is_empty()
    }

    /// Does `id` carry at least one mined provenance fact of its own?
    fn has_provenance(&self, id: Id) -> bool {
        self.confidence.contains_key(&id)
            || self.assurance.contains_key(&id)
            || self.derived_from.contains_key(&id)
    }

    /// The provenance weight `w(t) ∈ [floor, 1.0]` for the object-property triple `(h, r, t)` under
    /// `mode` — provenance about **the statement itself** when the graph reifies it, else the
    /// **head** `h`'s (see the module header for the two-level rule).
    ///
    /// Statement-level wins because it is the design's `w(t)`: the head fallback only reports that
    /// `h` is a low-assurance *entity*, which is a strictly weaker claim than "this assertion is
    /// doubtful". Consequence worth internalising: **without reification every triple sharing a
    /// head has an identical weight**, so any weighting *within* one subject's edges degenerates to
    /// the uniform mean. [`annotated_statements`](Self::annotated_statements) reports whether the
    /// per-statement signal exists at all.
    ///
    /// Under [`WeightMode::Uniform`] this is **always** `1.0` (the ablation-off baseline — provenance
    /// is not even consulted). Under [`WeightMode::Provenance`] it is the design-§USE-1 combination;
    /// an unannotated statement and head still return `1.0` (fail-open).
    pub fn weight_of(&self, triple: [Id; 3], mode: WeightMode) -> f32 {
        if mode == WeightMode::Uniform {
            return 1.0;
        }
        self.statement_weight(triple).unwrap_or_else(|| self.weight_for_subject(triple[0]))
    }

    /// The weight of `triple`'s **reified statement**, or `None` when the graph asserts no
    /// statement-level provenance for it. The MIN over the triple's annotated reifiers — a
    /// statement is only as trustworthy as its least-trustworthy annotation, mirroring the
    /// min-over-sources rule — so the result never depends on reifier iteration order.
    fn statement_weight(&self, triple: [Id; 3]) -> Option<f32> {
        let reifiers = self.statement.get(&triple)?;
        reifiers.iter().map(|&r| self.weight_for_subject(r)).fold(None, |acc, w| {
            Some(match acc {
                None => w,
                Some(a) => a.min(w),
            })
        })
    }

    /// The provenance weight contributed by a single subject `h` (the head of a positive). Public so
    /// a future confidence-biased walker / pooler can reuse the same scalar.
    pub fn weight_for_subject(&self, h: Id) -> f32 {
        let cfg = &self.config;

        // Assurance multiplier (default when the axis is absent — fail-open, never a penalty).
        let assurance_mult = match self.assurance.get(&h) {
            Some(Assurance::Proven) => cfg.proven,
            Some(Assurance::Claimed) => cfg.claimed,
            Some(Assurance::Conjectured) => cfg.conjectured,
            None => cfg.default_assurance,
        };

        // The head's own confidence (epistemic weight), default 1.0.
        let head_conf = self.confidence.get(&h).copied().unwrap_or(1.0);

        // Source reliability: the MIN confidence over the head's sources (a fact is only as reliable
        // as its least reliable source). A source with no asserted confidence contributes 1.0
        // (fail-open). No sources ⇒ 1.0.
        let source_reliability = match self.derived_from.get(&h) {
            None => 1.0,
            Some(sources) => sources
                .iter()
                .map(|s| self.confidence.get(s).copied().unwrap_or(1.0))
                .fold(1.0f32, f32::min),
        };

        let w = assurance_mult * head_conf * source_reliability;
        // Clamp to [floor, 1.0]: never zero (would drop the positive), never above 1.0.
        w.clamp(cfg.floor.max(f32::MIN_POSITIVE), 1.0)
    }

    // ---- integration point 2: confidence-weighted structural-sketch pooling -------------------
    // [OPUS-4.8] sq-oy9ya (design §USE-1 point 2), per-triple keying [OPUS-5] sq-w2af4. When a node
    // has MULTIPLE values for a multi-valued predicate (a characteristic set / structural sketch),
    // the standard pooling is a uniform mean of the value contributions. This pools by the
    // contributing ASSERTION's own `w(t)` instead, so a fact the graph reifies as low-assurance
    // pulls the pooled vector less. With uniform weights (all 1.0, i.e. a provenance-free graph)
    // this reduces EXACTLY to the arithmetic mean — a plain graph is unchanged. No accuracy claim
    // is made; it ships behind the same on/off ablation (`WeightMode`).

    /// **Provenance-weighted pool** of a node's per-value contributions for one multi-valued
    /// predicate (design §USE-1 point 2). `contributions` is the list of `(triple, value_vector)`
    /// pairs that feed this node's block for the predicate — one entry per asserted value, keyed by
    /// **the asserting triple itself**. The pooled vector is `Σ w_i · v_i / Σ w_i`, with
    /// `w_i = self.weight_of(triple_i, mode)`.
    ///
    /// **The weight is the assertion's `w(t)`** (see [`weight_of`](Self::weight_of)): the reified
    /// statement's provenance where the graph carries it, the head's otherwise. That fallback makes
    /// the *within-subject* case honest rather than proxied — every `(node, p, ·)` contribution
    /// shares the head `node`, so on a graph with no statement-level provenance
    /// ([`annotated_statements`](Self::annotated_statements) `== 0`) all weights are equal and the
    /// pool is **exactly** the arithmetic mean. It is deliberately NOT keyed on the value entity:
    /// "the object is a low-assurance entity" is a different heuristic from "this assertion is
    /// doubtful", and substituting it would misreport the design point as implemented.
    ///
    /// Under [`WeightMode::Uniform`] every `w_i = 1.0`, so the result is the **plain arithmetic
    /// mean** — byte-identical to the unweighted pooler (the ablation-off baseline). Under
    /// [`WeightMode::Provenance`] a value asserted by a lower-quality statement contributes
    /// proportionally less. Returns `None` for an empty `contributions` list, or `Err` if the
    /// vectors differ in length (a layout bug, never silently truncated). The denominator can never
    /// be zero: the weight floor (`> 0`) guarantees `Σ w_i > 0` whenever there is at least one
    /// contribution.
    pub fn pool_weighted(
        &self,
        contributions: &[([Id; 3], Vec<f32>)],
        mode: WeightMode,
    ) -> Result<Option<Vec<f32>>, String> {
        if contributions.is_empty() {
            return Ok(None);
        }
        let dim = contributions[0].1.len();
        let mut acc = vec![0.0f32; dim];
        let mut wsum = 0.0f32;
        for (triple, vec) in contributions {
            if vec.len() != dim {
                return Err(format!(
                    "pool_weighted: contribution vectors differ in length ({} vs {})",
                    vec.len(),
                    dim
                ));
            }
            let w = self.weight_of(*triple, mode);
            wsum += w;
            for (a, x) in acc.iter_mut().zip(vec.iter()) {
                *a += w * *x;
            }
        }
        // wsum > 0 always (each w >= floor > 0 under Provenance, and == 1.0 under Uniform), so the
        // division is safe; guard anyway so a degenerate (all-zero) weight set fails open to the
        // mean rather than producing NaNs.
        if wsum <= 0.0 {
            let n = contributions.len() as f32;
            for (a, _) in acc.iter_mut().zip(0..dim) {
                *a /= n;
            }
            return Ok(Some(acc));
        }
        for a in acc.iter_mut() {
            *a /= wsum;
        }
        Ok(Some(acc))
    }

    // ---- integration point 3 (producer): per-Block default fusion weight ----------------------
    // [OPUS-4.8] sq-oy9ya (design §USE-1 point 3). The query-time per-`Block` fusion weight is an
    // AGGREGATE of the provenance of the edges that fed the block. `block_weight` computes that
    // aggregate; the caller attaches it to a `Block` via `Block::with_weight` and the existing
    // `fuse_rrf_weighted`/`fuse_scores` path consumes it.

    /// The aggregate per-block **fusion weight** for a set of `subjects` whose incident edges fed a
    /// block (design §USE-1 point 3). It is the **mean** of the subjects' provenance weights under
    /// `mode` — a block fed mostly by low-quality facts gets a smaller multiplier, so at query time
    /// that modality contributes less to the fused ranking.
    ///
    /// Under [`WeightMode::Uniform`] this is **always** `1.0` (every subject weighs 1.0), so the
    /// block is the fail-open default and the fusion path is unchanged — the ablation-off baseline.
    /// An empty `subjects` set is also `1.0` (a block with no provenance-bearing edges is not
    /// penalised). The result is in `(0, 1]` (the per-subject floor lifts to a per-block floor), so
    /// it is a valid non-negative [`fuse_rrf_weighted`](crate::fuse_rrf_weighted) weight.
    pub fn block_weight(&self, subjects: impl IntoIterator<Item = Id>, mode: WeightMode) -> f32 {
        let mut sum = 0.0f32;
        let mut n = 0u32;
        for s in subjects {
            sum += self.weight_for_subject_under(s, mode);
            n += 1;
        }
        if n == 0 {
            return 1.0;
        }
        sum / n as f32
    }

    /// [`weight_for_subject`](Self::weight_for_subject) gated by `mode`: `1.0` under
    /// [`WeightMode::Uniform`], the provenance weight under [`WeightMode::Provenance`]. The
    /// mode-aware form the pooler and the block-weight aggregator share. [OPUS-4.8]
    pub fn weight_for_subject_under(&self, h: Id, mode: WeightMode) -> f32 {
        match mode {
            WeightMode::Uniform => 1.0,
            WeightMode::Provenance => self.weight_for_subject(h),
        }
    }

    // ---- integration point 3 (WIRING): graph → per-block weights on a real SchemaHeader --------
    // [OPUS-5] sq-w2af4. sq-oy9ya landed the `block_weight` aggregate and the `Block::weight`
    // field but left the caller that JOINS them unwritten, so nothing in-tree ever produced a
    // weighted header. These two methods are that caller: they derive each block's weight from the
    // graph itself (the subjects of the predicate that feeds the block) and hand back a
    // `SchemaHeader` whose blocks carry it — which the `.spqv` SPQS-v2 sidecar then persists and
    // `Block::fusion_weight` serves to the query-time fusion path.

    /// The **graph-global** default fusion weight for the block a `predicate` feeds:
    /// [`block_weight`](Self::block_weight) aggregated over **every** subject of that predicate's
    /// triples in `graph`.
    ///
    /// **Scope (do not over-read this).** The result is a single scalar per *predicate*, mined
    /// across the whole graph — it is **not** a per-node quantity and carries no information about
    /// any individual node's incident edges. A node with no incident `predicate` edge at all still
    /// receives this same multiplier once it is attached to the shared
    /// [`SchemaHeader`](crate::encode::SchemaHeader) (see [`weight_header`](Self::weight_header)).
    /// It is the persisted **default**; the design's §USE-1 point-3 per-node scaling is
    /// [`node_block_weight`](Self::node_block_weight), computed at grounding time.
    ///
    /// `1.0` (fail-open) when the graph never mentions `predicate`, and — as everywhere in this
    /// module — always exactly `1.0` under [`WeightMode::Uniform`], so the ablation-off arm leaves
    /// every block at the unweighted default.
    pub fn predicate_block_weight(&self, graph: &Graph, predicate: &str, mode: WeightMode) -> f32 {
        let Some(pid) = id_of_iri(graph, predicate) else {
            return 1.0;
        };
        let scan = graph.store.scan(&[None, Some(pid), None]);
        let subjects: Vec<Id> = scan.rows.iter().map(|row| scan.to_spo(row)[0]).collect();
        self.block_weight(subjects, mode)
    }

    /// The **per-node** fusion weight for the block a `predicate` feeds on **`node`'s own**
    /// incident edges (design §USE-1 point 3): the mean of `w(t)` over the `(node, predicate, ·)`
    /// triples `node` actually asserts. This is the per-node quantity the design asks for — it
    /// reads nothing from any other subject, so two nodes with different incident-edge provenance
    /// get different weights, and it is computed at grounding/query time
    /// ([`ground_weighted`](crate::grounding::ground_weighted)) rather than baked into the shared
    /// header.
    ///
    /// `1.0` (fail-open) when `node` has **no** incident `predicate` edge, when the graph never
    /// mentions `predicate`, and always under [`WeightMode::Uniform`] — so a node the modality does
    /// not describe is never penalised, and the ablation-off arm is the unweighted default. The
    /// result is in `(0, 1]`, a valid [`fuse_rrf_weighted`](crate::fuse_rrf_weighted) weight.
    /// [OPUS-5] sq-w2af4.
    pub fn node_block_weight(
        &self,
        graph: &Graph,
        node: Id,
        predicate: &str,
        mode: WeightMode,
    ) -> f32 {
        let Some(pid) = id_of_iri(graph, predicate) else {
            return 1.0;
        };
        let scan = graph.store.scan(&[Some(node), Some(pid), None]);
        let mut sum = 0.0f32;
        let mut n = 0u32;
        for row in scan.rows.iter() {
            sum += self.weight_of(scan.to_spo(row), mode);
            n += 1;
        }
        if n == 0 {
            return 1.0;
        }
        sum / n as f32
    }

    /// Attach provenance-derived per-block fusion weights to `header`, returning the weighted
    /// header (design §USE-1 point 3 — the graph-global **default** half; the per-node half is
    /// [`node_block_weight`](Self::node_block_weight)). `block_predicates[i]` names the predicate
    /// that feeds `header.blocks()[i]`; a `None` entry leaves that block at the fail-open default
    /// (`weight: 1.0` — e.g. a text or taxonomy lane with no single feeding predicate).
    ///
    /// **A `SchemaHeader` is graph-global, so these weights are too.** Each block's multiplier is
    /// [`predicate_block_weight`](Self::predicate_block_weight) — the mean over *all* subjects
    /// asserting that predicate anywhere in `graph`. Every node subsequently grounded against this
    /// header therefore receives the **same** per-block weights, whatever its own incident edges
    /// are. Treat it as a per-modality *default* ("this lane is fed by low-assurance data on this
    /// graph"), never as per-node evidence. For per-node evidence pass a
    /// [`NodeWeighting`](crate::grounding::NodeWeighting) to
    /// [`ground_weighted`](crate::grounding::ground_weighted), which overrides these defaults with
    /// [`node_block_weight`](Self::node_block_weight) for the node being grounded.
    ///
    /// The returned header is layout-identical to `header` ([`Block`](crate::encode::Block)'s
    /// `PartialEq` excludes the weight), so a weighted header is a drop-in replacement everywhere
    /// the unweighted one was used; only [`Block::fusion_weight`](crate::encode::Block::fusion_weight)
    /// — and the SPQS-v2 bytes — differ. Under [`WeightMode::Uniform`] every block resolves to
    /// `1.0`, so the ablation-off arm is behaviourally identical to today's unweighted header.
    ///
    /// `Err` if `block_predicates` is not exactly one entry per block (a caller/layout mismatch is
    /// a bug, never silently padded).
    pub fn weight_header(
        &self,
        graph: &Graph,
        header: &crate::encode::SchemaHeader,
        block_predicates: &[Option<&str>],
        mode: WeightMode,
    ) -> Result<crate::encode::SchemaHeader, String> {
        if block_predicates.len() != header.blocks().len() {
            return Err(format!(
                "weight_header: {} block predicates for {} blocks",
                block_predicates.len(),
                header.blocks().len()
            ));
        }
        let blocks = header
            .blocks()
            .iter()
            .zip(block_predicates.iter())
            .map(|(b, pred)| match pred {
                Some(p) => b.with_weight(self.predicate_block_weight(graph, p, mode)),
                None => *b,
            })
            .collect();
        crate::encode::SchemaHeader::new(blocks)
    }
}

// ---- helpers ----------------------------------------------------------------------------------

/// A named-node term for an IRI string.
fn iri(s: &str) -> oxrdf::Term {
    oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
}

/// Resolve an IRI string to its interned id, or `None` if the graph never mentions it.
fn id_of_iri(graph: &Graph, s: &str) -> Option<Id> {
    graph.id_of(&iri(s)).filter(|&i| i != NO_ID)
}

/// The first of `candidates` that the graph interns (so a graph using either the `sparq.dev` or the
/// `w3id.org` secx form resolves). `None` when none is present.
fn first_present(graph: &Graph, candidates: &[&str]) -> Option<Id> {
    candidates.iter().find_map(|s| id_of_iri(graph, s))
}

/// Parse the lexical form of a literal object id as an `f32` (`0..1` confidence). `None` for a
/// non-literal object or an unparseable lexical form (fail-open — a malformed confidence is
/// treated as "no confidence asserted", never a panic).
fn literal_f32(graph: &Graph, o: Id) -> Option<f32> {
    match graph.dict.term_parts(o) {
        TermParts::Lit { value, .. } => value.trim().parse::<f32>().ok().filter(|c| c.is_finite()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::close_for_vectorise;
    use sparq_reason::Profile;

    // A PKG-shaped fixture mirroring the real pkg.ttl predicate set so the reader exercises the
    // REAL id-scan path, not a mock. Three facts: high-assurance Proven (alice), mid-assurance
    // Claimed with a so-so source (bob), and a bare fact with NO provenance (carol). [OPUS-4.8]
    const PKG: &str = r#"
@prefix pkg:  <https://sparq.dev/ns/pkg#> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix secx: <https://sparq.dev/ns/secx#> .
@prefix ex:   <http://ex/> .

ex:source-reliable  pkg:confidence "1.0" .
ex:source-soso      pkg:confidence "0.5" .

ex:alice
  pkg:confidence "0.9" ;
  pkg:assurance  secx:Proven ;
  prov:wasDerivedFrom ex:source-reliable ;
  ex:knows ex:bob .

ex:bob
  pkg:confidence "0.8" ;
  pkg:assurance  secx:Claimed ;
  prov:wasDerivedFrom ex:source-soso ;
  ex:knows ex:carol .

ex:carol ex:knows ex:alice .
"#;

    fn graph() -> Graph {
        // Close so any pkg:discoveredFrom → prov:wasDerivedFrom entailment is materialised; the
        // fixture uses the super-property directly, so the closure simply confirms the read path.
        close_for_vectorise(PKG, "turtle", Profile::Rdfs).unwrap().graph
    }

    fn id(g: &Graph, s: &str) -> Id {
        g.id_of(&iri(s)).unwrap()
    }

    #[test]
    fn uniform_mode_is_always_one() {
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let alice = id(&g, "http://ex/alice");
        let knows = id(&g, "http://ex/knows");
        let bob = id(&g, "http://ex/bob");
        // Even a fully-annotated head weighs exactly 1.0 under the ablation-off baseline.
        assert_eq!(pw.weight_of([alice, knows, bob], WeightMode::Uniform), 1.0);
    }

    #[test]
    fn provenance_mode_reads_confidence_and_assurance() {
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let cfg = WeightConfig::default();
        let alice = id(&g, "http://ex/alice");
        let bob = id(&g, "http://ex/bob");
        let knows = id(&g, "http://ex/knows");
        let carol = id(&g, "http://ex/carol");

        // alice: Proven(1.0) · conf(0.9) · source-reliable(1.0) = 0.9.
        let wa = pw.weight_of([alice, knows, bob], WeightMode::Provenance);
        assert!((wa - cfg.proven * 0.9 * 1.0).abs() < 1e-6, "alice w = {wa}");

        // bob: Claimed(0.7) · conf(0.8) · source-soso(0.5) = 0.7·0.8·0.5 = 0.28.
        let wb = pw.weight_of([bob, knows, carol], WeightMode::Provenance);
        assert!((wb - cfg.claimed * 0.8 * 0.5).abs() < 1e-6, "bob w = {wb}");

        // bob's lower-assurance + so-so source down-weights it below alice (the load-bearing
        // ordering: high-assurance facts rank higher).
        assert!(wb < wa, "lower-assurance/source fact must weigh less ({wb} < {wa})");
    }

    #[test]
    fn no_provenance_is_fail_open_one() {
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let knows = id(&g, "http://ex/knows");
        let carol = id(&g, "http://ex/carol");
        let alice = id(&g, "http://ex/alice");
        // carol has NO confidence/assurance/source → w = 1.0 (a plain fact is unchanged).
        assert_eq!(pw.weight_of([carol, knows, alice], WeightMode::Provenance), 1.0);
    }

    #[test]
    fn plain_graph_has_no_provenance() {
        // A graph with zero PROV-O/DQV annotations: the reader is empty and every weight is 1.0,
        // so the trainer is byte-identical to the unweighted path.
        let plain = "@prefix ex: <http://ex/> .\nex:a ex:knows ex:b .\nex:b ex:knows ex:a .";
        let g = Graph::load_str(plain, "turtle").unwrap();
        let pw = ProvenanceWeights::mine(&g);
        assert!(pw.is_empty(), "plain graph carries no provenance");
        assert_eq!(pw.annotated_subjects(), 0);
        let a = id(&g, "http://ex/a");
        let b = id(&g, "http://ex/b");
        let knows = id(&g, "http://ex/knows");
        assert_eq!(pw.weight_of([a, knows, b], WeightMode::Provenance), 1.0);
    }

    #[test]
    fn weight_never_zero_floor_clamped() {
        // A degenerate config (conjectured = 0) must still floor-clamp, never emit a 0 weight that
        // would silently delete the positive from the loss.
        let ttl = r#"
@prefix pkg:  <https://sparq.dev/ns/pkg#> .
@prefix secx: <https://sparq.dev/ns/secx#> .
@prefix ex:   <http://ex/> .
ex:x pkg:assurance secx:Conjectured ; pkg:confidence "0.0" ; ex:rel ex:y .
ex:y ex:rel ex:x .
"#;
        let g = Graph::load_str(ttl, "turtle").unwrap();
        let cfg = WeightConfig { conjectured: 0.0, ..WeightConfig::default() };
        let pw = ProvenanceWeights::mine_with(&g, cfg);
        let x = id(&g, "http://ex/x");
        let y = id(&g, "http://ex/y");
        let rel = id(&g, "http://ex/rel");
        let w = pw.weight_of([x, rel, y], WeightMode::Provenance);
        assert!(w >= cfg.floor.max(f32::MIN_POSITIVE), "w = {w} must be floor-clamped > 0");
        assert!(w > 0.0, "w must never be zero (would drop the positive)");
    }

    #[test]
    fn malformed_confidence_is_ignored_not_panic() {
        // A non-numeric confidence is treated as "no confidence" (fail-open), never a panic.
        let ttl = r#"
@prefix pkg: <https://sparq.dev/ns/pkg#> .
@prefix ex:  <http://ex/> .
ex:a pkg:confidence "not-a-number" ; ex:rel ex:b .
ex:b ex:rel ex:a .
"#;
        let g = Graph::load_str(ttl, "turtle").unwrap();
        let pw = ProvenanceWeights::mine(&g);
        let a = id(&g, "http://ex/a");
        let b = id(&g, "http://ex/b");
        let rel = id(&g, "http://ex/rel");
        // No parseable confidence + no assurance → w = 1.0.
        assert_eq!(pw.weight_of([a, rel, b], WeightMode::Provenance), 1.0);
    }

    // ---- integration point 2: confidence-weighted structural-sketch pooling (sq-oy9ya) --------

    #[test]
    fn pool_uniform_mode_is_the_arithmetic_mean() {
        // The ablation-OFF baseline: every contribution weighs 1.0, so the pool is the plain mean,
        // byte-identical to the unweighted pooler — a plain graph is unchanged.
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let alice = id(&g, "http://ex/alice");
        let bob = id(&g, "http://ex/bob");
        let carol = id(&g, "http://ex/carol");
        let knows = id(&g, "http://ex/knows");
        let contribs = vec![
            ([alice, knows, bob], vec![3.0, 0.0]),
            ([bob, knows, carol], vec![0.0, 3.0]),
            ([carol, knows, alice], vec![3.0, 3.0]),
        ];
        let pooled = pw.pool_weighted(&contribs, WeightMode::Uniform).unwrap().unwrap();
        // Mean of (3,0),(0,3),(3,3) = (2,2).
        assert!((pooled[0] - 2.0).abs() < 1e-6 && (pooled[1] - 2.0).abs() < 1e-6, "{pooled:?}");
    }

    #[test]
    fn pool_provenance_mode_downweights_low_quality_contributions() {
        // alice (Proven, conf 0.9, reliable source) weighs 0.9; bob (Claimed 0.7 · conf 0.8 ·
        // source 0.5) weighs 0.28. The pooled vector must lean toward alice's assertion.
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let alice = id(&g, "http://ex/alice");
        let bob = id(&g, "http://ex/bob");
        let carol = id(&g, "http://ex/carol");
        let knows = id(&g, "http://ex/knows");
        let wa = pw.weight_for_subject(alice);
        let wb = pw.weight_for_subject(bob);
        assert!(wb < wa, "precondition: bob is lower quality ({wb} < {wa})");

        // alice's edge → (1,0), bob's → (0,1). Neither is reified, so each falls back to its head:
        // the weighted pool is (wa, wb)/(wa+wb) — more mass on dim 0.
        let contribs =
            vec![([alice, knows, bob], vec![1.0, 0.0]), ([bob, knows, carol], vec![0.0, 1.0])];
        let pooled = pw.pool_weighted(&contribs, WeightMode::Provenance).unwrap().unwrap();
        let expect0 = wa / (wa + wb);
        let expect1 = wb / (wa + wb);
        assert!((pooled[0] - expect0).abs() < 1e-6, "dim0 {} vs {}", pooled[0], expect0);
        assert!((pooled[1] - expect1).abs() < 1e-6, "dim1 {} vs {}", pooled[1], expect1);
        // The load-bearing invariant: alice's (higher-quality) contribution dominates.
        assert!(pooled[0] > pooled[1], "higher-quality contribution must dominate the pool");
    }

    #[test]
    fn pool_empty_is_none_and_mismatched_lengths_err() {
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let alice = id(&g, "http://ex/alice");
        let bob = id(&g, "http://ex/bob");
        let carol = id(&g, "http://ex/carol");
        let knows = id(&g, "http://ex/knows");
        assert!(pw.pool_weighted(&[], WeightMode::Provenance).unwrap().is_none());
        let bad =
            vec![([alice, knows, bob], vec![1.0, 2.0]), ([bob, knows, carol], vec![1.0])];
        assert!(pw.pool_weighted(&bad, WeightMode::Provenance).is_err(), "length mismatch is Err");
    }

    #[test]
    fn pool_single_contribution_is_itself() {
        // A single value pools to itself under either mode (the weighted mean of one item).
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let bob = id(&g, "http://ex/bob");
        let carol = id(&g, "http://ex/carol");
        let knows = id(&g, "http://ex/knows");
        let one = vec![([bob, knows, carol], vec![7.0, -2.0, 0.5])];
        for mode in [WeightMode::Uniform, WeightMode::Provenance] {
            let pooled = pw.pool_weighted(&one, mode).unwrap().unwrap();
            assert_eq!(pooled, vec![7.0, -2.0, 0.5], "single contribution pools to itself ({mode:?})");
        }
    }

    // ---- STATEMENT-level provenance: `w(t)` about the assertion itself (sq-w2af4) -------------

    /// Two edges of the SAME head, one reified low-confidence: the design's per-triple `w(t)`
    /// separates them, which no head- or object-keyed proxy can do. Both reification spellings are
    /// read — RDF 1.2 `rdf:reifies` and RDF 1.1 `rdf:subject`/`rdf:predicate`/`rdf:object`.
    #[test]
    fn statement_provenance_beats_the_head_fallback() {
        let ttl = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix pkg:  <https://sparq.dev/ns/pkg#> .
@prefix secx: <https://sparq.dev/ns/secx#> .
@prefix ex:   <http://ex/> .

ex:hub pkg:assurance secx:Proven ; ex:cites ex:a , ex:b , ex:c .

# RDF 1.2: the reifier annotates ONE of the hub's statements as doubtful.
ex:st1 rdf:reifies <<( ex:hub ex:cites ex:a )>> ; pkg:confidence "0.2" .
# RDF 1.1 standard reification: the same mapping, spelled for a plain-RDF graph.
ex:st2 rdf:subject ex:hub ; rdf:predicate ex:cites ; rdf:object ex:b ;
       pkg:assurance secx:Conjectured .
# An UNANNOTATED reifier says nothing — it must not fail-open over the head.
ex:st3 rdf:reifies <<( ex:hub ex:cites ex:c )>> .
"#;
        let g = Graph::load_str(ttl, "turtle").unwrap();
        let pw = ProvenanceWeights::mine(&g);
        let hub = id(&g, "http://ex/hub");
        let cites = id(&g, "http://ex/cites");
        let (a, b, c) = (id(&g, "http://ex/a"), id(&g, "http://ex/b"), id(&g, "http://ex/c"));

        assert_eq!(pw.annotated_statements(), 2, "only the ANNOTATED reifiers count");
        let head = pw.weight_for_subject(hub); // Proven, no confidence → 1.0
        let wa = pw.weight_of([hub, cites, a], WeightMode::Provenance);
        let wb = pw.weight_of([hub, cites, b], WeightMode::Provenance);
        let wc = pw.weight_of([hub, cites, c], WeightMode::Provenance);
        assert!((wa - 0.2).abs() < 1e-6, "RDF 1.2 reifier confidence is w(t): {}", wa);
        assert!(
            (wb - WeightConfig::default().conjectured).abs() < 1e-6,
            "RDF 1.1 reification is read too: {}",
            wb
        );
        assert_eq!(wc, head, "an unannotated reifier falls back to the head");
        // The load-bearing point: one subject's edges now carry DIFFERENT weights.
        assert!(wa < wc && wb < wc, "the doubted statements weigh less than the plain one");
        // Ablation-off is still exactly 1.0 everywhere.
        assert_eq!(pw.weight_of([hub, cites, a], WeightMode::Uniform), 1.0);
    }

    #[test]
    fn statement_weight_is_the_min_over_reifiers() {
        // Two reifiers disagree about the same statement: it is only as trustworthy as its
        // least-trustworthy annotation, so the result cannot depend on reifier order.
        let ttl = r#"
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix pkg: <https://sparq.dev/ns/pkg#> .
@prefix ex:  <http://ex/> .
ex:a ex:rel ex:b .
ex:r1 rdf:reifies <<( ex:a ex:rel ex:b )>> ; pkg:confidence "0.9" .
ex:r2 rdf:reifies <<( ex:a ex:rel ex:b )>> ; pkg:confidence "0.3" .
"#;
        let g = Graph::load_str(ttl, "turtle").unwrap();
        let pw = ProvenanceWeights::mine(&g);
        let (a, rel, b) = (id(&g, "http://ex/a"), id(&g, "http://ex/rel"), id(&g, "http://ex/b"));
        let w = pw.weight_of([a, rel, b], WeightMode::Provenance);
        assert!((w - 0.3).abs() < 1e-6, "min over reifiers, got {}", w);
    }

    #[test]
    fn pool_weighted_is_the_exact_mean_without_statement_provenance() {
        // The honest no-op: every contribution of ONE subject shares the head, so on a graph with
        // no reification the weighted pool IS the arithmetic mean — the pooling axis reports
        // itself unimplemented rather than substituting a different heuristic.
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        assert_eq!(pw.annotated_statements(), 0, "the PKG fixture is entity-level only");
        let bob = id(&g, "http://ex/bob");
        let knows = id(&g, "http://ex/knows");
        let carol = id(&g, "http://ex/carol");
        let alice = id(&g, "http://ex/alice");
        let contribs =
            vec![([bob, knows, carol], vec![4.0, 0.0]), ([bob, knows, alice], vec![0.0, 2.0])];
        let pooled = pw.pool_weighted(&contribs, WeightMode::Provenance).unwrap().unwrap();
        assert!((pooled[0] - 2.0).abs() < 1e-6 && (pooled[1] - 1.0).abs() < 1e-6, "{:?}", pooled);
    }

    // ---- integration point 3 (producer): per-block fusion weight aggregate (sq-oy9ya) ---------

    #[test]
    fn block_weight_uniform_is_one_and_provenance_is_mean() {
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let alice = id(&g, "http://ex/alice");
        let bob = id(&g, "http://ex/bob");
        let carol = id(&g, "http://ex/carol"); // no provenance → 1.0

        // Uniform: always 1.0 (ablation-off baseline).
        assert_eq!(pw.block_weight([alice, bob, carol], WeightMode::Uniform), 1.0);

        // Provenance: the MEAN of the per-subject weights.
        let wa = pw.weight_for_subject(alice);
        let wb = pw.weight_for_subject(bob);
        let wc = pw.weight_for_subject(carol); // 1.0
        let got = pw.block_weight([alice, bob, carol], WeightMode::Provenance);
        assert!((got - (wa + wb + wc) / 3.0).abs() < 1e-6, "block weight is the mean: {got}");
        // It is a valid (0,1] fuse weight.
        assert!(got > 0.0 && got <= 1.0, "block weight must be a valid fuse weight: {got}");

        // A block fed by only low-quality edges weighs less than one fed by high-quality edges.
        let high = pw.block_weight([alice], WeightMode::Provenance);
        let low = pw.block_weight([bob], WeightMode::Provenance);
        assert!(low < high, "lower-quality block must get a smaller fusion weight ({low} < {high})");

        // An empty subject set is the fail-open 1.0 (a block with no provenance edges).
        assert_eq!(pw.block_weight(std::iter::empty(), WeightMode::Provenance), 1.0);
    }

    // ---- integration point 3 (WIRING): graph → weighted SchemaHeader (sq-w2af4) ---------------

    #[test]
    fn predicate_block_weight_aggregates_the_predicate_subjects() {
        // [OPUS-5] `ex:knows` is asserted by alice (w=0.9), bob (w=0.28) and carol (no provenance,
        // w=1.0), so the block that predicate feeds weighs the mean of the three.
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let alice = id(&g, "http://ex/alice");
        let bob = id(&g, "http://ex/bob");
        let carol = id(&g, "http://ex/carol");
        let expect = (pw.weight_for_subject(alice)
            + pw.weight_for_subject(bob)
            + pw.weight_for_subject(carol))
            / 3.0;
        let got = pw.predicate_block_weight(&g, "http://ex/knows", WeightMode::Provenance);
        assert!((got - expect).abs() < 1e-6, "{} vs {}", got, expect);
        // Ablation-off, and an unknown predicate, are both the fail-open 1.0.
        assert_eq!(pw.predicate_block_weight(&g, "http://ex/knows", WeightMode::Uniform), 1.0);
        assert_eq!(pw.predicate_block_weight(&g, "http://ex/absent", WeightMode::Provenance), 1.0);
    }

    /// [OPUS-5] sq-w2af4 — the PER-NODE half of point 3: each node's modality weight is derived
    /// from **that node's own** incident edges and from nothing else.
    #[test]
    fn node_block_weight_reads_only_that_nodes_incident_edges() {
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let alice = id(&g, "http://ex/alice");
        let bob = id(&g, "http://ex/bob");
        let carol = id(&g, "http://ex/carol");

        // alice / bob / carol each assert exactly one `ex:knows` edge, of differing quality.
        let wa = pw.node_block_weight(&g, alice, "http://ex/knows", WeightMode::Provenance);
        let wb = pw.node_block_weight(&g, bob, "http://ex/knows", WeightMode::Provenance);
        let wc = pw.node_block_weight(&g, carol, "http://ex/knows", WeightMode::Provenance);
        assert!((wa - pw.weight_for_subject(alice)).abs() < 1e-6, "alice's own edge: {}", wa);
        assert!((wb - pw.weight_for_subject(bob)).abs() < 1e-6, "bob's own edge: {}", wb);
        assert_eq!(wc, 1.0, "carol has no provenance → fail-open");
        // Distinct nodes get DISTINCT weights — the property the graph-global default cannot have.
        assert!(wb < wa && wa < wc, "per-node ordering ({} < {} < {})", wb, wa, wc);
        // And each differs from the graph-global mean over ALL subjects of the predicate.
        let global = pw.predicate_block_weight(&g, "http://ex/knows", WeightMode::Provenance);
        assert!((wa - global).abs() > 1e-6 && (wb - global).abs() > 1e-6);

        // A node with NO incident edge for the predicate is the fail-open 1.0, not the global mean.
        assert_eq!(pw.node_block_weight(&g, alice, "http://ex/absent", WeightMode::Provenance), 1.0);
        // Ablation-off leaves every node at the unweighted default.
        assert_eq!(pw.node_block_weight(&g, bob, "http://ex/knows", WeightMode::Uniform), 1.0);
    }

    #[test]
    fn weight_header_attaches_block_weights_and_survives_the_spqs_round_trip() {
        use crate::encode::{Block, Encoder, Metric, SchemaHeader};
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let plain = SchemaHeader::new(vec![
            Block::new(Encoder::Numeric, Metric::Euclidean, 0, 4),
            Block::new(Encoder::Other, Metric::Euclidean, 4, 2),
        ])
        .unwrap();

        // Block 0 is fed by `ex:knows`; block 1 has no single feeding predicate (fail-open 1.0).
        let preds = [Some("http://ex/knows"), None];
        let weighted = pw.weight_header(&g, &plain, &preds, WeightMode::Provenance).unwrap();
        let w0 = weighted.blocks()[0].fusion_weight();
        assert!(w0 > 0.0 && w0 < 1.0, "the provenance-fed block is down-weighted: {}", w0);
        assert_eq!(weighted.blocks()[1].fusion_weight(), 1.0, "an unfed block stays fail-open");
        // The LAYOUT is unchanged — a weighted header is a drop-in for the plain one.
        assert_eq!(weighted, plain, "the weight is not part of the header's identity");

        // And the weight survives the real SPQS-v2 sidecar round trip (not just in RAM).
        let back = SchemaHeader::from_bytes(&weighted.to_bytes()).unwrap();
        assert!((back.blocks()[0].fusion_weight() - w0).abs() < 1e-6);

        // Ablation-off: every block resolves to the unweighted 1.0.
        let off = pw.weight_header(&g, &plain, &preds, WeightMode::Uniform).unwrap();
        assert!(off.blocks().iter().all(|b| b.fusion_weight() == 1.0));

        // A length mismatch is a caller bug, never silently padded.
        assert!(pw.weight_header(&g, &plain, &[None], WeightMode::Provenance).is_err());
    }

    #[test]
    fn block_weight_feeds_weighted_rrf() {
        // End-to-end: the block weight is a real fuse_rrf_weighted weight. A low-quality modality
        // (bob-fed block) down-weights its list relative to a high-quality one (alice-fed) — the
        // design §USE-1 point-3 query-time effect, exercised through the REAL fusion path.
        use crate::fuse::fuse_rrf_weighted;
        let g = graph();
        let pw = ProvenanceWeights::mine(&g);
        let alice = id(&g, "http://ex/alice");
        let bob = id(&g, "http://ex/bob");

        let high_w = pw.block_weight([alice], WeightMode::Provenance) as f64;
        let low_w = pw.block_weight([bob], WeightMode::Provenance) as f64;

        // Two modalities each rank a different item first. With provenance weights, the
        // high-quality modality's top item should win the fusion.
        let high_list: Vec<(&str, f64)> = vec![("from_high", 1.0)];
        let low_list: Vec<(&str, f64)> = vec![("from_low", 1.0)];
        let fused =
            fuse_rrf_weighted(&[(&high_list, high_w), (&low_list, low_w)], crate::RRF_K, 10);
        assert_eq!(fused[0].0, "from_high", "the higher-provenance modality must rank first");
        assert!(fused[0].1 > fused[1].1, "and strictly outrank the lower-provenance one");
    }
}
