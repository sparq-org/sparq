// sq-jxl0: single-source the crate overview from README.md so crates.io
// (package.readme) and the docs.rs front page render identical content. The README's
// rust fences are API-map sketches marked `ignore` (they reference an external graph and
// the sibling `sparq-vectors` crate, which is not a dependency of `sparq-sim`).
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // sq-emay: crate has zero `unsafe`

use oxrdf::{NamedNode, Term};
use rustc_hash::FxHashMap;
#[cfg(feature = "multi-hop")]
use rustc_hash::FxHashSet;
use sparq_core::dict::{self, Id, NO_ID};
use sparq_core::store::Pattern;
use sparq_core::Graph;

/// What a signature element records about each adjacent triple.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SignatureMode {
    /// `(direction, predicate, neighbor)` — entities are similar when they relate to the
    /// SAME neighbors via the same predicates (co-citation / shared-context similarity).
    /// The default, and the mode `most_similar`'s index-driven candidate generation is
    /// designed around.
    #[default]
    PredicateNeighbor,
    /// `(direction, predicate)` only — entities are similar when they are USED the same
    /// way, regardless of which neighbors they touch (predicate-profile / role
    /// similarity, a characteristic-set overlap).
    Predicates,
}

/// T-box expansion data for the `tbox` feature: the transitive closure of
/// `rdfs:subClassOf` and `rdfs:subPropertyOf` extracted from the graph itself,
/// plus the id of `rdf:type`. Built once in [`Sim::with_config`]. sq-181
#[cfg(feature = "tbox")]
struct TboxExpander {
    /// ID of `rdf:type` in this graph; `NO_ID` when absent (no type expansions).
    rdf_type: Id,
    /// `class → all transitively-reachable superclasses` (rdfs:subClassOf closure).
    subclass_up: FxHashMap<Id, Vec<Id>>,
    /// `property → all transitively-reachable superproperties` (rdfs:subPropertyOf closure).
    subprop_up: FxHashMap<Id, Vec<Id>>,
    /// `class → all transitive subclasses` — the reverse closure, needed for SOUND
    /// candidate generation: entities entail an inferred `(rdf:type, C)` element via
    /// triples typing them with any subclass of C.
    subclass_down: FxHashMap<Id, Vec<Id>>,
    /// `property → all transitive subproperties` — the reverse closure, needed for
    /// SOUND candidate generation: entities entail an inferred `(dir, P, N)` element
    /// via triples under any subproperty of P.
    subprop_down: FxHashMap<Id, Vec<Id>>,
}

/// Sorted IRI local-name index for the lexical fallback tier (`lexical` feature).
/// Holds `(local_name, id)` pairs sorted ascending by local name, covering IRI
/// entities only. Built once in [`Sim::with_config`]. sq-181
#[cfg(feature = "lexical")]
struct LexicalIndex {
    sorted: Vec<(Box<str>, Id)>,
}

/// Configuration for a [`Sim`] instance.
#[derive(Clone, Debug)]
pub struct SimConfig {
    pub mode: SignatureMode,
    /// Weigh each element by its predicate's inverse frequency (`1 + ln(|G|/freq(p))`),
    /// from the store's existing per-predicate stats. `false` = plain Jaccard.
    pub idf: bool,
    /// Predicates to EXCLUDE from signatures entirely (e.g. `rdf:type` when evaluating
    /// against type ground truth, or noisy provenance predicates).
    pub exclude_predicates: Vec<NamedNode>,
    /// Candidate-generation cap: signature elements matched by more than this many
    /// triples are SKIPPED during `most_similar` candidate generation (they are the
    /// lowest-IDF, least-informative elements, and scanning a hub's full extent would
    /// dominate the cost). Exact similarity scoring is unaffected; see
    /// [`Sim::most_similar`] for what this approximates. Default: 10 000.
    pub max_pair_frequency: usize,
    /// Neighbor-sparse fallback (default `true`): when `most_similar`'s
    /// index-driven `(predicate, neighbor)` candidate generation yields fewer than
    /// `k` results — typical for entities whose signature elements all point at
    /// degree-1 neighbors (every event names exactly one sport) — fill the remaining
    /// slots with [`SignatureMode::Predicates`]-style candidates, ranked below every
    /// exactly-scored result and scored by the predicate-profile Jaccard. `false`
    /// restores the strict v1 behaviour (only neighbor-level evidence is returned).
    pub profile_fallback: bool,
    /// T-box-aware signature expansion (default `true`, available with the `tbox`
    /// Cargo feature): expand `rdf:type` elements with inferred superclass entries
    /// from the graph's own `rdfs:subClassOf` closure, and every predicate element
    /// with its `rdfs:subPropertyOf` superproperties. Inferred entries contribute at
    /// `original_weight × 0.5`. `most_similar` candidate generation scans the reverse
    /// (subclass / subproperty) closure of inferred elements, so entities sharing only
    /// an inferred element — e.g. sibling subproperties of a common superproperty —
    /// are still retrieved. Builds both closures once in [`Sim::with_config`].
    #[cfg(feature = "tbox")]
    pub tbox_aware: bool,
    /// Lexical fallback tier (default `true`, available with the `lexical` Cargo
    /// feature): after structural- and profile-fallback candidates are exhausted,
    /// fill remaining slots with entities whose IRI local name has high trigram-Jaccard
    /// similarity to the query entity's local name. Ranked below all other tiers.
    /// Only fires for [`Sim::most_similar`] (not [`Sim::similar_by_signature`]).
    #[cfg(feature = "lexical")]
    pub lexical_fallback: bool,
    /// Maximum structural-neighborhood depth. The default `1` preserves the original
    /// adjacent-triple signature exactly. Values greater than `1` expand breadth-first,
    /// retaining first-hop elements at full weight and attenuating hop `h` by
    /// `0.5^(h - 1)`. A value of `0` is treated as `1`.
    #[cfg(feature = "multi-hop")]
    pub depth: usize,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            mode: SignatureMode::PredicateNeighbor,
            idf: true,
            exclude_predicates: Vec::new(),
            max_pair_frequency: 10_000,
            profile_fallback: true,
            #[cfg(feature = "tbox")]
            tbox_aware: true,
            #[cfg(feature = "lexical")]
            lexical_fallback: true,
            #[cfg(feature = "multi-hop")]
            depth: 1,
        }
    }
}

/// Fixed per-hop attenuation used by the `multi-hop` feature.
#[cfg(feature = "multi-hop")]
const MULTI_HOP_ATTENUATION: f64 = 0.5;

/// Fixed weight attenuation for T-box-inferred signature elements. An element derived
/// one step up the rdfs:subClassOf / rdfs:subPropertyOf hierarchy contributes at
/// `original_weight × TBOX_ATTENUATION`. sq-181
#[cfg(feature = "tbox")]
const TBOX_ATTENUATION: f64 = 0.5;

/// Standard RDF/RDFS vocabulary IRI strings used by the `tbox` feature. sq-181
#[cfg(feature = "tbox")]
const RDF_TYPE_IRI: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
#[cfg(feature = "tbox")]
const RDFS_SUBCLASS_OF_IRI: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
#[cfg(feature = "tbox")]
const RDFS_SUBPROPERTY_OF_IRI: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";

/// Edge direction within a signature element (outgoing: the entity is the subject;
/// incoming: the entity is the object). Direction is part of the element — "directs
/// films" and "is directed by" are different roles.
const OUT: u8 = 0;
const IN: u8 = 1;

/// An entity's structural signature: its sorted, deduplicated `(direction, predicate,
/// neighbor)` elements (neighbor = [`NO_ID`] in [`SignatureMode::Predicates`]) with one
/// weight per element. Build via [`Sim::signature`]; compare via [`Sim::similarity`] /
/// [`Sim::similar_by_signature`].
#[derive(Clone, Debug)]
pub struct Signature {
    /// Sorted ascending, unique.
    elems: Vec<(u8, Id, Id)>,
    /// Parallel to `elems`.
    weights: Vec<f64>,
    /// Sum of `weights`.
    total: f64,
}

impl Signature {
    pub fn len(&self) -> usize {
        self.elems.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elems.is_empty()
    }

    /// The sum of element weights (the weighted-Jaccard denominator contribution).
    pub fn total_weight(&self) -> f64 {
        self.total
    }
}

/// Structural similarity over a graph. Cheap to construct ([`Sim::new`] resolves the
/// excluded predicates and snapshots the triple count; predicate frequencies come from
/// the store's already-computed planner stats, O(1) per lookup).
pub struct Sim<'g> {
    graph: &'g Graph,
    mode: SignatureMode,
    idf: bool,
    excluded: Vec<Id>,
    max_pair_frequency: usize,
    profile_fallback: bool,
    #[cfg(feature = "tbox")]
    tbox: Option<TboxExpander>,
    #[cfg(feature = "lexical")]
    lexical_idx: Option<LexicalIndex>,
    #[cfg(feature = "multi-hop")]
    depth: usize,
    /// |G| as f64, the IDF numerator.
    n_triples: f64,
}

impl<'g> Sim<'g> {
    /// A `Sim` with the default configuration (predicate+neighbor signatures, pred-IDF
    /// weighting on).
    pub fn new(graph: &'g Graph) -> Self {
        Self::with_config(graph, SimConfig::default())
    }

    pub fn with_config(graph: &'g Graph, cfg: SimConfig) -> Self {
        // Resolve excluded predicates to ids once; absent predicates can never occur.
        let mut excluded: Vec<Id> = cfg
            .exclude_predicates
            .iter()
            .filter_map(|p| graph.id_of(&Term::NamedNode(p.clone())))
            .collect();
        excluded.sort_unstable();
        Sim {
            graph,
            mode: cfg.mode,
            idf: cfg.idf,
            excluded,
            max_pair_frequency: cfg.max_pair_frequency,
            profile_fallback: cfg.profile_fallback,
            #[cfg(feature = "tbox")]
            tbox: if cfg.tbox_aware { Some(build_tbox_expander(graph)) } else { None },
            #[cfg(feature = "lexical")]
            lexical_idx: if cfg.lexical_fallback { Some(build_lexical_index(graph)) } else { None },
            #[cfg(feature = "multi-hop")]
            depth: cfg.depth.max(1),
            n_triples: graph.len().max(1) as f64,
        }
    }

    /// The weight of a signature element with predicate `p`: `1 + ln(|G| / freq(p))`
    /// when IDF weighting is on (freq from the store's per-predicate planner stats — no
    /// scan), else 1. Monotonically decreasing in predicate frequency, ≥ 1.
    #[inline]
    fn weight(&self, p: Id) -> f64 {
        if !self.idf {
            return 1.0;
        }
        let freq = self.graph.store.pred_stat(p).map_or(1, |s| s.count.max(1));
        1.0 + (self.n_triples / freq as f64).ln().max(0.0)
    }

    #[inline]
    fn is_excluded(&self, p: Id) -> bool {
        self.excluded.binary_search(&p).is_ok()
    }

    /// The structural signature of `term`, or `None` if the term is not in the graph's
    /// dictionary. `O(log n + degree)`: one SPO range scan (outgoing) + one OSP/OPS
    /// range scan (incoming).
    pub fn signature(&self, term: &Term) -> Option<Signature> {
        let id = self.graph.id_of(term)?;
        Some(self.signature_of_id(id))
    }

    fn signature_of_id(&self, id: Id) -> Signature {
        let sig = self.signature_of_id_mode(id, self.mode == SignatureMode::PredicateNeighbor);
        #[cfg(feature = "tbox")]
        let sig = if self.tbox.is_some() {
            self.tbox_expand_signature(sig, self.mode == SignatureMode::PredicateNeighbor)
        } else {
            sig
        };
        sig
    }

    fn signature_of_id_mode(&self, id: Id, keep_neighbor: bool) -> Signature {
        #[cfg(feature = "multi-hop")]
        if self.depth > 1 {
            return self.expanded_signature_of_id_mode(id, keep_neighbor);
        }

        self.one_hop_signature_of_id_mode(id, keep_neighbor)
    }

    /// The original depth-1 implementation stays on its own path so the default feature
    /// state and `depth = 1` retain byte-identical floating-point operations and ordering.
    fn one_hop_signature_of_id_mode(&self, id: Id, keep_neighbor: bool) -> Signature {
        let mut elems: Vec<(u8, Id, Id)> = Vec::new();
        // Outgoing: (id, p, o) — one contiguous SPO range.
        let scan = self.graph.store.scan(&[Some(id), None, None]);
        for row in scan.rows.iter() {
            let [_, p, o] = scan.to_spo(row);
            if !self.is_excluded(p) {
                elems.push((OUT, p, if keep_neighbor { o } else { NO_ID }));
            }
        }
        // Incoming: (s, p, id) — one contiguous OSP/OPS range.
        let scan = self.graph.store.scan(&[None, None, Some(id)]);
        for row in scan.rows.iter() {
            let [s, p, _] = scan.to_spo(row);
            if !self.is_excluded(p) {
                elems.push((IN, p, if keep_neighbor { s } else { NO_ID }));
            }
        }
        elems.sort_unstable();
        elems.dedup();
        let weights: Vec<f64> = elems.iter().map(|&(_, p, _)| self.weight(p)).collect();
        let total = weights.iter().sum();
        Signature { elems, weights, total }
    }

    /// Breadth-first expansion for the opt-in `multi-hop` feature. Each layer scans in
    /// canonical id order; previously visited nodes are not reintroduced by backtracking.
    /// When several paths produce one element, the nearest (largest) weight wins.
    #[cfg(feature = "multi-hop")]
    fn expanded_signature_of_id_mode(&self, id: Id, keep_neighbor: bool) -> Signature {
        let mut element_weights: FxHashMap<(u8, Id, Id), f64> = FxHashMap::default();
        let mut visited: FxHashSet<Id> = FxHashSet::default();
        visited.insert(id);
        let mut frontier = vec![id];
        let mut attenuation = 1.0;

        for hop in 1..=self.depth {
            let mut next_frontier = Vec::new();
            for center in frontier {
                let outgoing = self.graph.store.scan(&[Some(center), None, None]);
                for row in outgoing.rows.iter() {
                    let [_, p, neighbor] = outgoing.to_spo(row);
                    if !self.is_excluded(p) && (hop == 1 || !visited.contains(&neighbor)) {
                        let elem = (OUT, p, if keep_neighbor { neighbor } else { NO_ID });
                        let weight = self.weight(p) * attenuation;
                        element_weights
                            .entry(elem)
                            .and_modify(|current| *current = current.max(weight))
                            .or_insert(weight);
                    }
                    if !self.is_excluded(p) && !visited.contains(&neighbor) {
                        next_frontier.push(neighbor);
                    }
                }

                let incoming = self.graph.store.scan(&[None, None, Some(center)]);
                for row in incoming.rows.iter() {
                    let [neighbor, p, _] = incoming.to_spo(row);
                    if !self.is_excluded(p) && (hop == 1 || !visited.contains(&neighbor)) {
                        let elem = (IN, p, if keep_neighbor { neighbor } else { NO_ID });
                        let weight = self.weight(p) * attenuation;
                        element_weights
                            .entry(elem)
                            .and_modify(|current| *current = current.max(weight))
                            .or_insert(weight);
                    }
                    if !self.is_excluded(p) && !visited.contains(&neighbor) {
                        next_frontier.push(neighbor);
                    }
                }
            }

            if next_frontier.is_empty() {
                break;
            }
            next_frontier.sort_unstable();
            next_frontier.dedup();
            visited.extend(next_frontier.iter().copied());
            frontier = next_frontier;
            attenuation *= MULTI_HOP_ATTENUATION;
        }

        let mut weighted: Vec<((u8, Id, Id), f64)> = element_weights.into_iter().collect();
        weighted.sort_unstable_by_key(|&(elem, _)| elem);
        let (elems, weights): (Vec<_>, Vec<_>) = weighted.into_iter().unzip();
        let total = weights.iter().sum();
        Signature {
            elems,
            weights,
            total,
        }
    }

    /// Weighted Jaccard similarity of two terms' structural signatures, in `[0, 1]`:
    /// `Σ w(e) over A∩B / Σ w(e) over A∪B`. 0 if either term is absent from the graph
    /// or both signatures are empty; 1 iff the (non-empty) signatures are identical.
    /// Symmetric. Cost: two signature builds + one sorted merge.
    pub fn similarity(&self, a: &Term, b: &Term) -> f64 {
        let (Some(a), Some(b)) = (self.graph.id_of(a), self.graph.id_of(b)) else {
            return 0.0;
        };
        weighted_jaccard(&self.signature_of_id(a), &self.signature_of_id(b))
    }

    /// The `k` entities most similar to `a` (excluding `a` itself), best first.
    ///
    /// **Candidate generation is index-driven, not a full scan.** For each element of
    /// `a`'s signature, the entities sharing it form one contiguous index range:
    /// - outgoing `(p, n)`: subjects of `(?, p, n)` — a POS range scan;
    /// - incoming `(p, n)`: objects of `(n, p, ?)` — an SPO range scan;
    /// - [`SignatureMode::Predicates`]: the distinct subjects (PSO) / objects (POS) of
    ///   the predicate's whole block.
    ///
    /// Candidates accumulate the weight of each element they share (an upper bound on
    /// their intersection weight); the top `max(4k, 64)` accumulated candidates are then
    /// re-scored EXACTLY with [`similarity`](Self::similarity) semantics.
    ///
    /// **Complexity** `O( Σ_e min(freq(e), F) )` for generation over the signature's
    /// elements `e` (each a binary search + contiguous range read), plus
    /// `O( M · (log n + d̄) )` for re-scoring `M = max(4k, 64)` candidates of average
    /// degree `d̄`.
    ///
    /// **Approximation:** elements matched by more than `max_pair_frequency` (`F`)
    /// triples are skipped during *generation* (hub elements — the lowest-IDF, least
    /// informative, and the only unbounded cost). An entity is therefore guaranteed to
    /// be found iff it shares at least one sub-cap element with `a`; re-scoring is
    /// always exact over full signatures. Raise the cap (or set it to `usize::MAX`) for
    /// exact-but-slower candidate generation.
    ///
    /// **Neighbor-sparse fallback** (on by default, [`SimConfig::profile_fallback`]):
    /// when generation yields fewer than `k` candidates, the remaining slots are
    /// filled with entities sharing a `(direction, predicate)` ROLE with `a`,
    /// ranked below every exactly-scored result and scored by the predicate-profile
    /// Jaccard (what [`similarity`](Self::similarity) would return in
    /// [`SignatureMode::Predicates`]). Fallback scores are comparable among
    /// themselves, not with the exactly-scored block.
    pub fn most_similar(&self, a: &Term, k: usize) -> Vec<(Term, f64)> {
        let Some(id) = self.graph.id_of(a) else {
            return Vec::new();
        };
        let sig = self.signature_of_id(id);
        self.similar_by_sig(&sig, k, Some(id))
    }

    /// Like [`most_similar`](Self::most_similar) but for an arbitrary (possibly
    /// synthetic) signature — e.g. a probe built from a handful of (predicate, neighbor)
    /// constraints, or a signature carried over from another graph.
    pub fn similar_by_signature(&self, sig: &Signature, k: usize) -> Vec<(Term, f64)> {
        self.similar_by_sig(sig, k, None)
    }

    fn similar_by_sig(&self, sig: &Signature, k: usize, exclude: Option<Id>) -> Vec<(Term, f64)> {
        if k == 0 || sig.is_empty() {
            return Vec::new();
        }
        // 1. Accumulate, per candidate entity, the total weight of shared elements.
        let mut acc: FxHashMap<Id, f64> = FxHashMap::default();
        for (i, &(dir, p, n)) in sig.elems.iter().enumerate() {
            let w = sig.weights[i];
            // The candidates sharing this element are one contiguous range; which
            // pattern (and which row column holds the candidate) depends on the
            // direction and mode.
            let pattern_for = |cp: Id, cn: Id| -> (Pattern, usize) {
                match (self.mode, dir) {
                    // Who else has (p, n) outgoing? subjects of (?, p, n) — POS range.
                    (SignatureMode::PredicateNeighbor, OUT) => ([None, Some(cp), Some(cn)], 0),
                    // Who else has (p, n) incoming? objects of (n, p, ?) — SPO range.
                    (SignatureMode::PredicateNeighbor, _) => ([Some(cn), Some(cp), None], 2),
                    // Predicate-profile mode: every subject / object of p's block.
                    (SignatureMode::Predicates, OUT) => ([None, Some(cp), None], 0),
                    (SignatureMode::Predicates, _) => ([None, Some(cp), None], 2),
                }
            };
            let (pattern, sorted_by) = pattern_for(p, n);
            // T-box-aware generation: an INFERRED element (superproperty, or a
            // superclass neighbor of rdf:type) can have zero direct triples — other
            // entities entail it only via triples under a transitive SUBproperty of
            // `p` (or typing them with a transitive SUBclass of `n`). Scanning just
            // the direct pattern would miss those candidates even though re-scoring
            // gives them a positive similarity, so when the element has a non-empty
            // down-closure every concrete pattern is scanned, deduping candidates
            // across scans so each receives this element's weight exactly once.
            // The hub cap applies per concrete pattern (same documented
            // approximation as the direct path).
            #[cfg(feature = "tbox")]
            if let Some(tbox) = &self.tbox {
                let sub_preds: &[Id] =
                    tbox.subprop_down.get(&p).map(Vec::as_slice).unwrap_or_default();
                let is_type_elem = self.mode == SignatureMode::PredicateNeighbor
                    && dir == OUT
                    && p == tbox.rdf_type
                    && tbox.rdf_type != NO_ID;
                let sub_classes: &[Id] = if is_type_elem {
                    tbox.subclass_down.get(&n).map(Vec::as_slice).unwrap_or_default()
                } else {
                    &[]
                };
                if !sub_preds.is_empty() || !sub_classes.is_empty() {
                    let mut seen: rustc_hash::FxHashSet<Id> = rustc_hash::FxHashSet::default();
                    for &cp in std::iter::once(&p).chain(sub_preds) {
                        for &cn in std::iter::once(&n).chain(sub_classes) {
                            let (pattern, sorted_by) = pattern_for(cp, cn);
                            if self.graph.store.estimate(&pattern) > self.max_pair_frequency {
                                continue; // hub pattern: skipped (documented approximation)
                            }
                            let scan = self.graph.store.scan_sorted(&pattern, sorted_by);
                            for row in scan.rows.iter() {
                                let cand = scan.to_spo(row)[sorted_by];
                                if Some(cand) == exclude || !self.is_entity(cand) {
                                    continue;
                                }
                                if seen.insert(cand) {
                                    *acc.entry(cand).or_insert(0.0) += w;
                                }
                            }
                        }
                    }
                    continue;
                }
            }
            if self.graph.store.estimate(&pattern) > self.max_pair_frequency {
                continue; // hub element: skipped (documented approximation)
            }
            // `scan_sorted` orders rows by the candidate column, so duplicates (one
            // subject with many objects under p, etc.) are consecutive — dedup as we go
            // so each candidate receives this element's weight exactly once.
            let scan = self.graph.store.scan_sorted(&pattern, sorted_by);
            let mut last: Option<Id> = None;
            for row in scan.rows.iter() {
                let cand = scan.to_spo(row)[sorted_by];
                if last == Some(cand) {
                    continue;
                }
                last = Some(cand);
                if Some(cand) == exclude || !self.is_entity(cand) {
                    continue;
                }
                *acc.entry(cand).or_insert(0.0) += w;
            }
        }
        // 2. Keep the strongest M candidates by accumulated (upper-bound) weight …
        let m = k.saturating_mul(4).max(64);
        let mut cands: Vec<(Id, f64)> = acc.into_iter().collect();
        cands.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        cands.truncate(m);
        // 3. … and re-score them exactly against the full signature.
        let mut scored: Vec<(Id, f64)> = cands
            .into_iter()
            .map(|(c, _)| (c, weighted_jaccard(sig, &self.signature_of_id(c))))
            .collect();
        scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.truncate(k);
        // 4. Neighbor-sparse fallback: when index-driven generation starves
        // (fewer than k candidates), fill the remaining slots with
        // predicate-PROFILE matches (see `profile_fallback`).
        if self.profile_fallback && self.mode == SignatureMode::PredicateNeighbor && scored.len() < k
        {
            self.fill_profile_fallback(sig, k, exclude, &mut scored);
        }
        // 5. Lexical fallback (opt-in `lexical` feature): after structural and profile
        // fallbacks, fill remaining slots from the sorted IRI local-name index using
        // trigram-Jaccard similarity. Only fires for `most_similar` (exclude.is_some()).
        // `lexical_idx.is_some()` is equivalent to `cfg.lexical_fallback` at build time.
        // sq-181
        #[cfg(feature = "lexical")]
        if self.lexical_idx.is_some() && exclude.is_some() && scored.len() < k {
            self.fill_lexical_fallback(k, exclude, &mut scored);
        }
        scored.into_iter().map(|(c, s)| (self.graph.dict.term(c), s)).collect()
    }

    /// The neighbor-sparse fallback of [`most_similar`](Self::most_similar):
    /// when `(direction, predicate, neighbor)` candidate generation yields
    /// fewer than `k` results (every signature element points at degree-1
    /// neighbors — e.g. each event names exactly one sport, so the reverse
    /// scan from each neighbor finds only the query entity itself), the
    /// remaining slots are filled by [`SignatureMode::Predicates`]-style
    /// generation: entities sharing a `(direction, predicate)` ROLE with the
    /// query, scored by the **predicate-profile weighted Jaccard** (the score
    /// `similarity` would return in `Predicates` mode).
    ///
    /// Fallback entries always rank BELOW every exactly-scored primary result
    /// (neighbor-level evidence outranks role-only evidence), and their scores
    /// are profile Jaccards — comparable among themselves, not with the
    /// primary block. Predicate blocks are scanned most-selective-first and
    /// generation stops once `max(4k, 64)` fallback candidates accumulate, so
    /// a hub predicate is only scanned when the starvation is real and nothing
    /// cheaper fills the budget.
    fn fill_profile_fallback(
        &self,
        sig: &Signature,
        k: usize,
        exclude: Option<Id>,
        scored: &mut Vec<(Id, f64)>,
    ) {
        let have: rustc_hash::FxHashSet<Id> = scored.iter().map(|&(c, _)| c).collect();
        // Distinct (direction, predicate) roles of the query's signature,
        // most selective predicate block first.
        let mut roles: Vec<(u8, Id, usize)> = Vec::new();
        for &(dir, p, _) in sig.elems.iter() {
            if roles.iter().all(|&(d, q, _)| (d, q) != (dir, p)) {
                let est = self.graph.store.estimate(&[None, Some(p), None]);
                roles.push((dir, p, est));
            }
        }
        roles.sort_unstable_by_key(|&(_, _, est)| est);
        let budget = k.saturating_mul(4).max(64);
        let mut acc: FxHashMap<Id, f64> = FxHashMap::default();
        for &(dir, p, _) in &roles {
            // Who else plays this role? Every subject (OUT) / object (IN) of
            // the predicate's block.
            let sorted_by = if dir == OUT { 0 } else { 2 };
            let scan = self.graph.store.scan_sorted(&[None, Some(p), None], sorted_by);
            let mut last: Option<Id> = None;
            for row in scan.rows.iter() {
                let cand = scan.to_spo(row)[sorted_by];
                if last == Some(cand) {
                    continue;
                }
                last = Some(cand);
                if Some(cand) == exclude || have.contains(&cand) || !self.is_entity(cand) {
                    continue;
                }
                if !acc.contains_key(&cand) && acc.len() >= budget {
                    continue; // budget full: existing candidates may still accrue
                }
                *acc.entry(cand).or_insert(0.0) += self.weight(p);
            }
            if acc.len() >= budget {
                break;
            }
        }
        // Re-score by predicate-profile Jaccard and fill the remaining slots.
        let probe = self.profile_of(sig);
        let mut fb: Vec<(Id, f64)> = acc
            .into_keys()
            .map(|c| (c, weighted_jaccard(&probe, &self.signature_of_id_mode(c, false))))
            .filter(|&(_, s)| s > 0.0)
            .collect();
        fb.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        fb.truncate(k - scored.len());
        scored.extend(fb);
    }

    /// Collapses a signature to its predicate profile: distinct
    /// `(direction, predicate)` elements (neighbor = [`NO_ID`]), weighted like
    /// a [`SignatureMode::Predicates`] signature.
    fn profile_of(&self, sig: &Signature) -> Signature {
        #[cfg(feature = "multi-hop")]
        if self.depth > 1 {
            let mut role_weights: FxHashMap<(u8, Id, Id), f64> = FxHashMap::default();
            for (&(direction, predicate, _), &weight) in sig.elems.iter().zip(&sig.weights) {
                role_weights
                    .entry((direction, predicate, NO_ID))
                    .and_modify(|current| *current = current.max(weight))
                    .or_insert(weight);
            }
            let mut weighted: Vec<((u8, Id, Id), f64)> = role_weights.into_iter().collect();
            weighted.sort_unstable_by_key(|&(elem, _)| elem);
            let (elems, weights): (Vec<_>, Vec<_>) = weighted.into_iter().unzip();
            let total = weights.iter().sum();
            return Signature {
                elems,
                weights,
                total,
            };
        }

        let mut elems: Vec<(u8, Id, Id)> =
            sig.elems.iter().map(|&(d, p, _)| (d, p, NO_ID)).collect();
        elems.sort_unstable();
        elems.dedup();
        let weights: Vec<f64> = elems.iter().map(|&(_, p, _)| self.weight(p)).collect();
        let total = weights.iter().sum();
        Signature { elems, weights, total }
    }

    /// Whether an id names an entity (IRI or blank node) — literals (incl. inline
    /// integers) are valid signature *neighbors* but are never returned as similar
    /// *entities*.
    #[inline]
    fn is_entity(&self, id: Id) -> bool {
        if dict::is_inline(id) {
            return false;
        }
        matches!(
            self.graph.dict.term_parts(id),
            dict::TermParts::Iri { .. } | dict::TermParts::Blank(_)
        )
    }

    /// Lexical fallback tier (`lexical` feature): fill remaining `k - scored.len()` slots
    /// with IRI entities whose local name has high trigram-Jaccard similarity to the query
    /// entity's local name. Scans the FULL index — trigram overlap does not imply a shared
    /// prefix (`xalice` shares `ali`/`lic`/`ice` with `alice`), so a prefix-bounded scan
    /// would silently narrow the documented metric; the O(index) cost only arises when the
    /// structural and profile tiers have already starved. Only fires when
    /// `exclude = Some(query_id)`. sq-181
    #[cfg(feature = "lexical")]
    fn fill_lexical_fallback(
        &self,
        k: usize,
        exclude: Option<Id>,
        scored: &mut Vec<(Id, f64)>,
    ) {
        let Some(lexical) = &self.lexical_idx else { return };
        let Some(query_id) = exclude else { return };

        let query_key: Option<String> = {
            let parts = self.graph.dict.term_parts(query_id);
            lexical_key_of(&parts)
        };
        let Some(query_key) = query_key else { return };

        let have: rustc_hash::FxHashSet<Id> = scored.iter().map(|&(c, _)| c).collect();
        let remaining = k - scored.len();

        let mut fb: Vec<(Id, f64)> = lexical
            .sorted
            .iter()
            .filter_map(|(key, id)| {
                if Some(*id) == exclude || have.contains(id) || !self.is_entity(*id) {
                    return None;
                }
                let s = trigram_jaccard(&query_key, key);
                if s > 0.0 { Some((*id, s)) } else { None }
            })
            .collect();

        fb.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        fb.truncate(remaining);
        scored.extend(fb);
    }
}

/// T-box-aware signature expansion methods (`tbox` feature). sq-181
#[cfg(feature = "tbox")]
impl<'g> Sim<'g> {
    /// Expand `sig` with elements inferred from the T-box hierarchy:
    /// - `(OUT, rdf:type, C)` → add `(OUT, rdf:type, Ci)` for each transitive superclass Ci.
    /// - `(dir, P, N)` → add `(dir, Pi, N)` for each transitive superproperty Pi of P.
    ///
    /// Inferred elements receive `original_weight × TBOX_ATTENUATION`. When an inferred
    /// element duplicates a direct element the direct weight (higher) wins. sq-181
    fn tbox_expand_signature(&self, sig: Signature, keep_neighbor: bool) -> Signature {
        let Some(tbox) = &self.tbox else { return sig; };

        let n_base = sig.elems.len();
        let mut new_elems: Vec<(u8, Id, Id)> = Vec::new();
        let mut new_weights: Vec<f64> = Vec::new();

        for i in 0..n_base {
            let (dir, p, nbr) = sig.elems[i];
            let base_w = sig.weights[i];

            // Expand via rdfs:subPropertyOf: add (dir, sp, nbr) for each superproperty sp.
            if let Some(superprops) = tbox.subprop_up.get(&p) {
                for &sp in superprops {
                    let e = (dir, sp, nbr);
                    if sig.elems.binary_search(&e).is_err() && !new_elems.contains(&e) {
                        new_elems.push(e);
                        new_weights.push(base_w * TBOX_ATTENUATION);
                    }
                }
            }

            // Expand via rdfs:subClassOf: add (OUT, rdf:type, sc) for each superclass sc.
            // Only when keep_neighbor = true (PredicateNeighbor mode) and p = rdf:type.
            if keep_neighbor
                && dir == OUT
                && p == tbox.rdf_type
                && tbox.rdf_type != NO_ID
            {
                if let Some(superclasses) = tbox.subclass_up.get(&nbr) {
                    for &sc in superclasses {
                        let e = (OUT, tbox.rdf_type, sc);
                        if sig.elems.binary_search(&e).is_err() && !new_elems.contains(&e) {
                            new_elems.push(e);
                            new_weights.push(base_w * TBOX_ATTENUATION);
                        }
                    }
                }
            }
        }

        if new_elems.is_empty() {
            return sig;
        }

        // Merge originals + inferred; sort by element key; dedup keeping MAX weight.
        let mut merged: Vec<((u8, Id, Id), f64)> = sig
            .elems
            .iter()
            .copied()
            .zip(sig.weights.iter().copied())
            .chain(new_elems.into_iter().zip(new_weights))
            .collect();
        merged.sort_unstable_by_key(|&(e, _)| e);

        let mut deduped: Vec<((u8, Id, Id), f64)> = Vec::with_capacity(merged.len());
        for (e, w) in merged {
            if let Some(last) = deduped.last_mut() {
                if last.0 == e {
                    last.1 = last.1.max(w);
                    continue;
                }
            }
            deduped.push((e, w));
        }

        let (elems, weights): (Vec<_>, Vec<_>) = deduped.into_iter().unzip();
        let total = weights.iter().sum();
        Signature { elems, weights, total }
    }
}

// ---- T-box helpers (`tbox` feature) -----------------------------------------------

/// Build the `TboxExpander` for a graph: scan for `rdfs:subClassOf` /
/// `rdfs:subPropertyOf` triples, compute their transitive closures, and record
/// the `rdf:type` id. sq-181
#[cfg(feature = "tbox")]
fn build_tbox_expander(graph: &Graph) -> TboxExpander {
    let rdf_type = graph
        .id_of(&Term::NamedNode(NamedNode::new_unchecked(RDF_TYPE_IRI)))
        .unwrap_or(NO_ID);

    let mut sc_direct: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    if let Some(sc_id) =
        graph.id_of(&Term::NamedNode(NamedNode::new_unchecked(RDFS_SUBCLASS_OF_IRI)))
    {
        let scan = graph.store.scan(&[None, Some(sc_id), None]);
        for row in scan.rows.iter() {
            let [s, _, o] = scan.to_spo(row);
            sc_direct.entry(s).or_default().push(o);
        }
    }

    let mut sp_direct: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    if let Some(sp_id) =
        graph.id_of(&Term::NamedNode(NamedNode::new_unchecked(RDFS_SUBPROPERTY_OF_IRI)))
    {
        let scan = graph.store.scan(&[None, Some(sp_id), None]);
        for row in scan.rows.iter() {
            let [s, _, o] = scan.to_spo(row);
            sp_direct.entry(s).or_default().push(o);
        }
    }

    TboxExpander {
        rdf_type,
        subclass_up: tbox_transitive_closure(&sc_direct),
        subprop_up: tbox_transitive_closure(&sp_direct),
        subclass_down: tbox_transitive_closure(&tbox_invert(&sc_direct)),
        subprop_down: tbox_transitive_closure(&tbox_invert(&sp_direct)),
    }
}

/// Reverse a `node → direct successors` edge map (`successor → direct predecessors`),
/// so the same closure walk produces the DOWN closures.
#[cfg(feature = "tbox")]
fn tbox_invert(direct: &FxHashMap<Id, Vec<Id>>) -> FxHashMap<Id, Vec<Id>> {
    let mut inverted: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for (&sub, supers) in direct {
        for &sup in supers {
            inverted.entry(sup).or_default().push(sub);
        }
    }
    inverted
}

/// BFS transitive closure: `node → all reachable successors` in a directed graph
/// given as `node → direct successors`. sq-181
#[cfg(feature = "tbox")]
fn tbox_transitive_closure(
    direct: &FxHashMap<Id, Vec<Id>>,
) -> FxHashMap<Id, Vec<Id>> {
    let mut closure: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for &start in direct.keys() {
        let mut seen: rustc_hash::FxHashSet<Id> = rustc_hash::FxHashSet::default();
        let mut stack: Vec<Id> = direct.get(&start).cloned().unwrap_or_default();
        while let Some(n) = stack.pop() {
            if seen.insert(n) {
                if let Some(succ) = direct.get(&n) {
                    stack.extend(succ.iter().copied());
                }
            }
        }
        if !seen.is_empty() {
            closure.insert(start, seen.into_iter().collect());
        }
    }
    closure
}

// ---- Lexical-tier helpers (`lexical` feature) -------------------------------------

/// Build the sorted `(local_name, id)` lexical index for all IRI entities in
/// `graph`. sq-181
#[cfg(feature = "lexical")]
fn build_lexical_index(graph: &Graph) -> LexicalIndex {
    let mut entries: Vec<(Box<str>, Id)> = graph
        .dict
        .iter()
        .filter_map(|(id, parts)| {
            if dict::is_inline(id) {
                return None;
            }
            let key = lexical_key_of(&parts)?;
            Some((key.into_boxed_str(), id))
        })
        .collect();
    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    LexicalIndex { sorted: entries }
}

/// Extract the lexical sort key for a term: the IRI local name (suffix) for IRI
/// terms, or the literal value string for literals. `None` for blank nodes, triple
/// terms, or empty keys. sq-181
#[cfg(feature = "lexical")]
fn lexical_key_of(parts: &dict::TermParts<'_>) -> Option<String> {
    match parts {
        dict::TermParts::Iri { suffix, .. } if !suffix.is_empty() => Some((*suffix).to_owned()),
        dict::TermParts::Iri { prefix, .. } if !prefix.is_empty() => Some((*prefix).to_owned()),
        dict::TermParts::Lit { value, .. } if !value.is_empty() => Some((*value).to_owned()),
        _ => None,
    }
}

/// Character-trigram Jaccard similarity of two strings: `|tri(a) ∩ tri(b)| / |tri(a) ∪ tri(b)|`,
/// with trigrams built over `char`s — a multi-byte character occupies ONE trigram
/// position, never split across its UTF-8 bytes. Falls back to a longest-common-prefix
/// ratio over characters for strings shorter than 3 characters. Returns `1.0` for two
/// identical empty strings. sq-181 · char-trigram fix
#[cfg(feature = "lexical")]
fn trigram_jaccard(a: &str, b: &str) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let ca: Vec<char> = a.chars().collect();
    let cb: Vec<char> = b.chars().collect();
    if ca.len().min(cb.len()) < 3 {
        let lcp = ca.iter().zip(cb.iter()).take_while(|(x, y)| x == y).count();
        return lcp as f64 / ca.len().max(cb.len()) as f64;
    }
    let ta: rustc_hash::FxHashSet<[char; 3]> =
        ca.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
    let tb: rustc_hash::FxHashSet<[char; 3]> =
        cb.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
    let inter = ta.iter().filter(|t| tb.contains(*t)).count();
    let union = ta.len() + tb.len() - inter;
    if union == 0 { 1.0 } else { inter as f64 / union as f64 }
}

// ---- Signature metric free functions -------------------------------------------

/// Weighted Jaccard of two prebuilt signatures — `Σ min(wA, wB) / Σ max(wA, wB)` by a
/// single merge of the two sorted element lists; 0 when the union is empty. The
/// signature-level counterpart of [`Sim::similarity`], for callers that cache
/// signatures (e.g. all-pairs evaluation).
pub fn weighted_jaccard(a: &Signature, b: &Signature) -> f64 {
    let inter = weighted_intersection(a, b);
    let union = a.total + b.total - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Weighted Sorensen-Dice coefficient of two prebuilt signatures:
/// `2 * Σ min(wA, wB) / (Σ wA + Σ wB)`. Returns 0 when both totals are 0.
pub fn dice_coefficient(a: &Signature, b: &Signature) -> f64 {
    let denom = a.total + b.total;
    if denom <= 0.0 {
        0.0
    } else {
        2.0 * weighted_intersection(a, b) / denom
    }
}

/// Weighted overlap (Szymkiewicz-Simpson) coefficient of two prebuilt signatures:
/// `Σ min(wA, wB) / min(Σ wA, Σ wB)`. Returns 0 when either total is 0.
pub fn overlap_coefficient(a: &Signature, b: &Signature) -> f64 {
    let min_total = a.total.min(b.total);
    if min_total <= 0.0 {
        0.0
    } else {
        weighted_intersection(a, b) / min_total
    }
}

// sq-da2bz: share the canonical sorted merge across signature metrics.
fn weighted_intersection(a: &Signature, b: &Signature) -> f64 {
    let mut inter = 0.0;
    let (mut i, mut j) = (0, 0);
    while i < a.elems.len() && j < b.elems.len() {
        match a.elems[i].cmp(&b.elems[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                inter += a.weights[i].min(b.weights[j]);
                i += 1;
                j += 1;
            }
        }
    }
    inter
}

/// Edge direction of a [`SharedElement`], relative to the FIRST argument of
/// [`Sim::explain_similarity`]: whether the entity participates in the shared element as
/// the triple's subject (`Out`) or object (`In`) — the same in/out split
/// [`SignatureMode`] signatures record (direction is part of the element: "directs
/// films" and "is directed by" are different roles).
#[cfg(feature = "explain")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Outgoing — `entity --predicate--> neighbor`.
    Out,
    /// Incoming — `neighbor --predicate--> entity`.
    In,
}

/// One signature element two entities have in COMMON — the decoded reason behind a
/// [`Sim::similarity`] / [`Sim::most_similar`] score, returned by
/// [`Sim::explain_similarity`].
#[cfg(feature = "explain")]
#[derive(Clone, Debug, PartialEq)]
pub struct SharedElement {
    /// The element's edge direction (see [`Direction`]).
    pub direction: Direction,
    /// The shared predicate, decoded from the graph's dictionary.
    pub predicate: Term,
    /// The shared neighbor, decoded from the graph's dictionary. `None` in
    /// [`SignatureMode::Predicates`] (a predicate-profile element has no neighbor).
    pub neighbor: Option<Term>,
    /// This element's contribution to the weighted-Jaccard NUMERATOR:
    /// `min(weight_a, weight_b)` — the two weights only differ under the `multi-hop`
    /// feature's hop attenuation; at depth 1 both equal the predicate's IDF weight.
    pub weight: f64,
}

#[cfg(feature = "explain")]
impl Sim<'_> {
    /// The shared signature elements behind [`similarity`](Self::similarity)`(a, b)` —
    /// the weighted-Jaccard intersection, decoded to terms so a UI or agent can show
    /// WHY two entities scored similar.
    ///
    /// The returned weights sum to exactly the intersection weight the score's
    /// numerator uses: with `Σ` that sum and `|A|`/`|B|` the two signatures'
    /// [`total_weight`](Signature::total_weight)s, `Σ / (|A| + |B| − Σ)` reconstructs
    /// [`similarity`](Self::similarity)`(a, b)`. Every element is present in BOTH
    /// signatures.
    ///
    /// Deterministic order, strongest evidence first: weight descending, then
    /// predicate / neighbor / direction id ascending. Empty when either term is absent
    /// from the graph or the signatures share nothing. Cost: two signature builds +
    /// one sorted merge (the same shape [`similarity`](Self::similarity) pays).
    pub fn explain_similarity(&self, a: &Term, b: &Term) -> Vec<SharedElement> {
        let (Some(a), Some(b)) = (self.graph.id_of(a), self.graph.id_of(b)) else {
            return Vec::new();
        };
        let sa = self.signature_of_id(a);
        let sb = self.signature_of_id(b);
        // The same sorted merge as `weighted_intersection`, keeping the elements.
        let mut shared: Vec<((u8, Id, Id), f64)> = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < sa.elems.len() && j < sb.elems.len() {
            match sa.elems[i].cmp(&sb.elems[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    shared.push((sa.elems[i], sa.weights[i].min(sb.weights[j])));
                    i += 1;
                    j += 1;
                }
            }
        }
        shared.sort_unstable_by(|&((da, pa, na), wa), &((db, pb, nb), wb)| {
            wb.total_cmp(&wa).then_with(|| (pa, na, da).cmp(&(pb, nb, db)))
        });
        shared
            .into_iter()
            .map(|((dir, p, n), w)| SharedElement {
                direction: if dir == OUT { Direction::Out } else { Direction::In },
                predicate: self.graph.dict.term(p),
                neighbor: (n != NO_ID).then(|| self.graph.dict.term(n)),
                weight: w,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// sq-lgw: opt-in `sketch` feature — MinHash/LSH signature sketches, the
// Phase-4 escape hatch for graphs dense enough that `most_similar`'s per-query,
// per-element range-scan candidate generation hits its scaling wall
// (research/genai-design.md §6). Candidate generation becomes LSH bucket lookups
// against a prebuilt index; returned scores stay EXACT (the same weighted-Jaccard
// re-scoring `most_similar` performs), so only candidate RECALL is approximate.
// ---------------------------------------------------------------------------

/// Configuration for [`Sim::sketch_index`] (the opt-in `sketch` feature).
///
/// `num_hashes` MinHash components per entity estimate the **unweighted** Jaccard
/// similarity of two entities' signature element *sets* (IDF weights play no part in
/// the estimate — only in the exact re-scoring). The components split into `bands`
/// LSH bands of `num_hashes / bands` rows each: two entities become candidates for
/// each other iff they agree on EVERY row of at least one band, so more bands (fewer
/// rows per band) trade precision for recall. A fixed `seed` makes index builds fully
/// deterministic.
#[cfg(feature = "sketch")]
#[derive(Clone, Debug)]
pub struct SketchConfig {
    /// MinHash components per sketch (default 128). Estimation error shrinks as
    /// `O(1/√num_hashes)`; memory is `4 · num_hashes` bytes per indexed entity.
    pub num_hashes: usize,
    /// LSH bands (default 32 — 4 rows per band at the default `num_hashes`). Clamped
    /// to `1..=num_hashes`; when `bands` does not divide `num_hashes` the trailing
    /// remainder components contribute to estimates but not to banding.
    pub bands: usize,
    /// Hash-family seed. The default is fixed so builds reproduce byte-identically.
    pub seed: u64,
}

#[cfg(feature = "sketch")]
impl Default for SketchConfig {
    fn default() -> Self {
        // Default seed: "SKETCH" in ASCII. With 128 hashes / 32 bands (4 rows), an
        // entity pair with unweighted Jaccard s collides with prob 1 − (1 − s⁴)³².
        SketchConfig { num_hashes: 128, bands: 32, seed: 0x534B_4554_4348 }
    }
}

/// SplitMix64 finalizer (public-domain constants) — the crate's dependency-free
/// 64-bit mixer for the MinHash hash family and the LSH band keys.
#[cfg(feature = "sketch")]
#[inline]
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The LSH key of one band: the band's row components folded through the mixer,
/// domain-separated by the band index. Shared by build and query so an entity always
/// finds its own bucket.
#[cfg(feature = "sketch")]
fn band_key(band: usize, rows: usize, sketch: &[u32]) -> u64 {
    let mut key = splitmix64(band as u64);
    for &component in &sketch[band * rows..(band + 1) * rows] {
        key = splitmix64(key ^ u64::from(component));
    }
    key
}

/// A prebuilt MinHash/LSH index over every entity of a [`Sim`]'s graph — the
/// dense-graph candidate generator behind [`SketchIndex::most_similar`]. Build via
/// [`Sim::sketch_index`]; the index snapshots the graph at build time (rebuild after
/// updates). Sketches are built from the SAME signature elements exact scoring reads
/// (mode, excluded predicates, and — under `multi-hop` — depth all respected); only
/// the element weights are invisible to the sketch.
#[cfg(feature = "sketch")]
pub struct SketchIndex<'s, 'g> {
    sim: &'s Sim<'g>,
    num_hashes: usize,
    bands: usize,
    rows: usize,
    /// Indexed entity ids, ascending; an entity's SLOT is its position here.
    entities: Vec<Id>,
    slot_of: FxHashMap<Id, u32>,
    /// Concatenated fixed-length sketches: slot `s` owns `data[s·H..][..H]`.
    data: Vec<u32>,
    /// `(band, band key) -> slots` — the LSH buckets.
    buckets: FxHashMap<(u32, u64), Vec<u32>>,
}

#[cfg(feature = "sketch")]
impl<'g> Sim<'g> {
    /// Builds a [`SketchIndex`] over every entity of this `Sim`'s graph: one full
    /// scan to enumerate entities, then per entity one signature build plus
    /// `num_hashes` hashes per element — `O(Σ degree · num_hashes)` total, paid once
    /// instead of per query. Entities with an empty signature (e.g. every adjacent
    /// predicate excluded) are not indexed.
    pub fn sketch_index(&self, cfg: SketchConfig) -> SketchIndex<'_, 'g> {
        let num_hashes = cfg.num_hashes.max(1);
        let bands = cfg.bands.clamp(1, num_hashes);
        let rows = num_hashes / bands;
        let hash_seeds: Vec<u64> = (0..num_hashes as u64)
            .map(|i| splitmix64(cfg.seed.wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15))))
            .collect();

        // The entity universe: every subject + entity object, one full scan,
        // ascending-id order (slot order therefore equals id order).
        let scan = self.graph.store.scan(&[None, None, None]);
        let mut ids: Vec<Id> = Vec::with_capacity(2 * scan.rows.len());
        for row in scan.rows.iter() {
            let [s, _, o] = scan.to_spo(row);
            ids.push(s);
            ids.push(o);
        }
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|&id| self.is_entity(id));

        let mut entities: Vec<Id> = Vec::new();
        let mut slot_of: FxHashMap<Id, u32> = FxHashMap::default();
        let mut data: Vec<u32> = Vec::new();
        let mut buckets: FxHashMap<(u32, u64), Vec<u32>> = FxHashMap::default();
        let mut mins = vec![u32::MAX; num_hashes];
        for id in ids {
            let sig = self.signature_of_id(id);
            if sig.is_empty() {
                continue;
            }
            mins.iter_mut().for_each(|m| *m = u32::MAX);
            for &(dir, p, n) in sig.elems.iter() {
                let mut base = splitmix64(u64::from(dir));
                base = splitmix64(base ^ u64::from(p));
                base = splitmix64(base ^ u64::from(n));
                for (min, &seed) in mins.iter_mut().zip(&hash_seeds) {
                    // Truncating the mix to its top 32 bits halves sketch memory; a
                    // per-component collision (2⁻³²) only perturbs the estimate — a
                    // returned score is always re-scored exactly.
                    let h = (splitmix64(base ^ seed) >> 32) as u32;
                    if h < *min {
                        *min = h;
                    }
                }
            }
            let slot = entities.len() as u32;
            for b in 0..bands {
                buckets.entry((b as u32, band_key(b, rows, &mins))).or_default().push(slot);
            }
            entities.push(id);
            slot_of.insert(id, slot);
            data.extend_from_slice(&mins);
        }
        SketchIndex { sim: self, num_hashes, bands, rows, entities, slot_of, data, buckets }
    }
}

#[cfg(feature = "sketch")]
impl SketchIndex<'_, '_> {
    /// Number of indexed entities (entities with a non-empty signature).
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    #[inline]
    fn sketch_of(&self, slot: u32) -> &[u32] {
        let start = slot as usize * self.num_hashes;
        &self.data[start..start + self.num_hashes]
    }

    /// The fraction of matching MinHash components — an unbiased estimate of the
    /// **unweighted** Jaccard similarity of two entities' signature element sets, in
    /// `[0, 1]`. 0 when either term is not in the index (absent from the graph, a
    /// literal, or an empty signature). This is the ranking signal candidate
    /// generation uses; it is NOT the exact IDF-weighted [`Sim::similarity`] score.
    pub fn estimate(&self, a: &Term, b: &Term) -> f64 {
        let (Some(a), Some(b)) = (self.sim.graph.id_of(a), self.sim.graph.id_of(b)) else {
            return 0.0;
        };
        let (Some(&sa), Some(&sb)) = (self.slot_of.get(&a), self.slot_of.get(&b)) else {
            return 0.0;
        };
        self.matching_fraction(self.sketch_of(sa), self.sketch_of(sb))
    }

    fn matching_fraction(&self, a: &[u32], b: &[u32]) -> f64 {
        let matching = a.iter().zip(b).filter(|(x, y)| x == y).count();
        matching as f64 / self.num_hashes as f64
    }

    /// The `k` entities most similar to `a` (excluding `a`), best first — the
    /// dense-graph counterpart of [`Sim::most_similar`]. Candidate generation reads
    /// LSH buckets instead of index ranges: every indexed entity agreeing with `a` on
    /// at least one full band is a candidate, candidates rank by
    /// [`estimate`](Self::estimate), and the top `max(4k, 64)` re-score EXACTLY with
    /// [`Sim::similarity`] semantics — a returned `(entity, score)` pair always
    /// satisfies `score == similarity(a, entity)`. Zero-scoring candidates (LSH
    /// band-key false positives with no real signature overlap) are dropped.
    ///
    /// **Approximation:** recall is probabilistic — an entity at unweighted Jaccard
    /// `s` from `a` becomes a candidate with probability `1 − (1 − s^rows)^bands`
    /// (identical-signature twins always collide). Unlike [`Sim::most_similar`]
    /// there is no shares-a-sub-cap-element recall guarantee, and no profile
    /// fallback. In exchange, per-query cost is the matched buckets plus the exact
    /// re-scoring — independent of hub-element frequency, which is the point of the
    /// escape hatch.
    pub fn most_similar(&self, a: &Term, k: usize) -> Vec<(Term, f64)> {
        let Some(id) = self.sim.graph.id_of(a) else {
            return Vec::new();
        };
        let Some(&slot) = self.slot_of.get(&id) else {
            return Vec::new();
        };
        if k == 0 {
            return Vec::new();
        }
        let sketch = self.sketch_of(slot);
        // 1. LSH candidate generation: union of `a`'s buckets across all bands.
        let mut cand_slots: Vec<u32> = Vec::new();
        for b in 0..self.bands {
            if let Some(bucket) = self.buckets.get(&(b as u32, band_key(b, self.rows, sketch))) {
                cand_slots.extend(bucket.iter().copied());
            }
        }
        cand_slots.sort_unstable();
        cand_slots.dedup();
        // 2. Keep the strongest M candidates by estimated Jaccard (slot order equals
        //    id order, so the tie-break matches `most_similar`'s id-ascending rule) …
        let mut ranked: Vec<(u32, f64)> = cand_slots
            .into_iter()
            .filter(|&s| s != slot)
            .map(|s| (s, self.matching_fraction(sketch, self.sketch_of(s))))
            .collect();
        ranked.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(k.saturating_mul(4).max(64));
        // 3. … and re-score them exactly against the full weighted signature.
        let sig = self.sim.signature_of_id(id);
        let mut scored: Vec<(Id, f64)> = ranked
            .into_iter()
            .map(|(s, _)| {
                let c = self.entities[s as usize];
                (c, weighted_jaccard(&sig, &self.sim.signature_of_id(c)))
            })
            .filter(|&(_, v)| v > 0.0)
            .collect();
        scored.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.truncate(k);
        scored.into_iter().map(|(c, v)| (self.sim.graph.dict.term(c), v)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(format!("http://ex.org/{s}")).unwrap())
    }

    fn graph(ttl: &str) -> Graph {
        let prefix = "@prefix : <http://ex.org/> . @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";
        Graph::load_str(&format!("{prefix}{ttl}"), "turtle").unwrap()
    }

    fn unweighted(g: &Graph) -> Sim<'_> {
        Sim::with_config(g, SimConfig { idf: false, ..SimConfig::default() })
    }

    fn explicit_signature(entries: &[((u8, Id, Id), f64)]) -> Signature {
        let mut entries = entries.to_vec();
        entries.sort_unstable_by_key(|&(elem, _)| elem);
        let total = entries.iter().map(|(_, weight)| weight).sum();
        let (elems, weights) = entries.into_iter().unzip();
        Signature {
            elems,
            weights,
            total,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-12,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn signature_coefficients_identical_are_one() {
        let sig = explicit_signature(&[((OUT, 10, 100), 1.5), ((IN, 20, 200), 2.5)]);

        assert_close(dice_coefficient(&sig, &sig), 1.0);
        assert_close(overlap_coefficient(&sig, &sig), 1.0);
    }

    #[test]
    fn signature_coefficients_disjoint_are_zero() {
        let a = explicit_signature(&[((OUT, 10, 100), 2.0)]);
        let b = explicit_signature(&[((IN, 20, 200), 3.0)]);

        assert_eq!(dice_coefficient(&a, &b), 0.0);
        assert_eq!(overlap_coefficient(&a, &b), 0.0);
    }

    #[test]
    fn signature_coefficients_partial_overlap_are_weighted_and_symmetric() {
        // A totals 3; B totals 4; their shared element contributes min(2, 1) = 1.
        let a = explicit_signature(&[((OUT, 10, 100), 2.0), ((OUT, 20, 200), 1.0)]);
        let b = explicit_signature(&[((OUT, 10, 100), 1.0), ((IN, 30, 300), 3.0)]);
        let dice = dice_coefficient(&a, &b);
        let overlap = overlap_coefficient(&a, &b);
        let jaccard = weighted_jaccard(&a, &b);

        assert_close(dice, 2.0 / 7.0);
        assert_close(overlap, 1.0 / 3.0);
        assert_close(jaccard, 1.0 / 6.0);
        assert!(dice > jaccard, "Dice need not be at most Jaccard");
        assert!(overlap >= jaccard);
        assert!(jaccard >= 0.0);
        assert_close(dice_coefficient(&b, &a), dice);
        assert_close(overlap_coefficient(&b, &a), overlap);
    }

    #[test]
    fn signature_coefficients_with_empty_signature_are_zero_and_finite() {
        let empty = explicit_signature(&[]);
        let nonempty = explicit_signature(&[((OUT, 10, 100), 2.0)]);

        for value in [
            dice_coefficient(&empty, &nonempty),
            dice_coefficient(&nonempty, &empty),
            overlap_coefficient(&empty, &nonempty),
            overlap_coefficient(&nonempty, &empty),
        ] {
            assert_eq!(value, 0.0);
            assert!(!value.is_nan());
        }
    }

    #[cfg(feature = "multi-hop")]
    fn multi_hop_unweighted(g: &Graph, depth: usize) -> Sim<'_> {
        Sim::with_config(
            g,
            SimConfig {
                idf: false,
                depth,
                profile_fallback: false,
                ..SimConfig::default()
            },
        )
    }

    #[cfg(feature = "multi-hop")]
    #[test]
    fn default_depth_pins_original_signature_and_similarity() {
        let g = graph(":a :p :b . :twin :p :b . :b :q :c .");
        let sim = Sim::with_config(
            &g,
            SimConfig {
                idf: false,
                profile_fallback: false,
                ..SimConfig::default()
            },
        );
        let p = g.id_of(&iri("p")).unwrap();
        let b = g.id_of(&iri("b")).unwrap();
        let signature = sim.signature(&iri("a")).unwrap();

        assert_eq!(signature.elems, vec![(OUT, p, b)]);
        assert_eq!(signature.weights, vec![1.0]);
        assert_eq!(signature.total_weight(), 1.0);
        assert_eq!(sim.similarity(&iri("a"), &iri("twin")), 1.0);
    }

    #[cfg(feature = "multi-hop")]
    #[test]
    fn depth_two_chain_has_exact_attenuated_signature() {
        let g = graph(":a :p :b . :b :q :c .");
        let sim = multi_hop_unweighted(&g, 2);
        let mut expected = vec![
            (
                (
                    OUT,
                    g.id_of(&iri("p")).unwrap(),
                    g.id_of(&iri("b")).unwrap(),
                ),
                1.0,
            ),
            (
                (
                    OUT,
                    g.id_of(&iri("q")).unwrap(),
                    g.id_of(&iri("c")).unwrap(),
                ),
                0.5,
            ),
        ];
        expected.sort_unstable_by_key(|&(elem, _)| elem);
        let signature = sim.signature(&iri("a")).unwrap();
        let actual: Vec<_> = signature
            .elems
            .iter()
            .copied()
            .zip(signature.weights.iter().copied())
            .collect();

        assert_eq!(actual, expected);
        assert_eq!(signature.total_weight(), 1.5);
    }

    #[test]
    fn identical_signatures_similarity_one() {
        let g = graph(":alice :knows :bob ; :worksAt :acme . :carol :knows :bob ; :worksAt :acme .");
        let sim = unweighted(&g);
        assert_eq!(sim.similarity(&iri("alice"), &iri("carol")), 1.0);
        assert_eq!(sim.similarity(&iri("alice"), &iri("alice")), 1.0);
    }

    #[test]
    fn plain_jaccard_hand_computed() {
        // sig(alice) = {(out,knows,bob), (out,knows,eve), (out,worksAt,acme)}  (3 elems)
        // sig(dave)  = {(out,knows,eve), (out,plays,chess)}                    (2 elems)
        // intersection = {(out,knows,eve)} = 1; union = 4 → 0.25.
        let g = graph(":alice :knows :bob, :eve ; :worksAt :acme . :dave :knows :eve ; :plays :chess .");
        let sim = unweighted(&g);
        assert_eq!(sim.similarity(&iri("alice"), &iri("dave")), 0.25);
        // symmetric
        assert_eq!(sim.similarity(&iri("dave"), &iri("alice")), 0.25);
    }

    #[test]
    fn disjoint_signatures_zero_and_absent_term_zero() {
        let g = graph(":a :p :x . :b :q :y .");
        let sim = unweighted(&g);
        assert_eq!(sim.similarity(&iri("a"), &iri("b")), 0.0);
        assert_eq!(sim.similarity(&iri("a"), &iri("missing")), 0.0);
        assert!(sim.signature(&iri("missing")).is_none());
        assert!(sim.most_similar(&iri("missing"), 5).is_empty());
    }

    #[test]
    fn incoming_edges_count() {
        // a and b share ONLY their incoming context: x→a and x→b via :p.
        let g = graph(":x :p :a . :x :p :b .");
        let sim = unweighted(&g);
        // sig(a) = sig(b) = {(in, p, x)} → 1.0
        assert_eq!(sim.similarity(&iri("a"), &iri("b")), 1.0);
        // and direction matters: x's OUTGOING (p, a) is not a's INCOMING (p, x)
        assert_eq!(sim.similarity(&iri("x"), &iri("a")), 0.0);
    }

    #[test]
    fn predicates_only_mode_ignores_neighbors() {
        let g = graph(":a :knows :x ; :worksAt :acme . :b :knows :y ; :worksAt :corp .");
        let pred_only = Sim::with_config(
            &g,
            SimConfig { mode: SignatureMode::Predicates, idf: false, ..SimConfig::default() },
        );
        // Same predicate profile, different neighbors.
        assert_eq!(pred_only.similarity(&iri("a"), &iri("b")), 1.0);
        let full = unweighted(&g);
        assert_eq!(full.similarity(&iri("a"), &iri("b")), 0.0);
    }

    #[test]
    fn idf_weights_rare_predicates_higher() {
        // a∩b share the RARE predicate (2 triples); a∩c share the COMMON one (8 triples).
        // Unweighted the two pairs tie; with IDF the rare-shared pair must win.
        let mut ttl = String::from(":a :rare :r ; :common :c1 . :b :rare :r . :c :common :c1 .");
        for i in 0..6 {
            ttl.push_str(&format!(" :f{i} :common :c{i} ."));
        }
        let g = graph(&ttl);
        let plain = unweighted(&g);
        let ab = plain.similarity(&iri("a"), &iri("b"));
        let ac = plain.similarity(&iri("a"), &iri("c"));
        assert_eq!(ab, ac, "unweighted: both pairs share exactly one of a's two elements");
        let idf = Sim::new(&g);
        assert!(
            idf.similarity(&iri("a"), &iri("b")) > idf.similarity(&iri("a"), &iri("c")),
            "IDF must favour the rare shared predicate"
        );
    }

    #[test]
    fn excluded_predicates_drop_from_signature() {
        let g = graph(":a rdf:type :T ; :p :x . :b rdf:type :T ; :q :y .");
        let rdf_type = NamedNode::new("http://www.w3.org/1999/02/22-rdf-syntax-ns#type").unwrap();
        let with_type = unweighted(&g);
        assert!(with_type.similarity(&iri("a"), &iri("b")) > 0.0);
        let no_type = Sim::with_config(
            &g,
            SimConfig { idf: false, exclude_predicates: vec![rdf_type], ..SimConfig::default() },
        );
        assert_eq!(no_type.similarity(&iri("a"), &iri("b")), 0.0);
        assert_eq!(no_type.signature(&iri("a")).unwrap().len(), 1); // just (out, :p, :x)
    }

    #[test]
    fn most_similar_ranks_and_excludes_self() {
        let g = graph(
            ":a :knows :bob, :eve ; :worksAt :acme .
             :twin :knows :bob, :eve ; :worksAt :acme .
             :half :knows :bob ; :worksAt :other .
             :far  :plays :chess .",
        );
        let sim = unweighted(&g);
        let top = sim.most_similar(&iri("a"), 10);
        let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
        assert_eq!(names[0], "<http://ex.org/twin>");
        assert_eq!(top[0].1, 1.0);
        assert_eq!(names[1], "<http://ex.org/half>"); // shares 1 elem; union = 3+2-1 → 1/4
        assert_eq!(top[1].1, 0.25);
        assert!(!names.iter().any(|n| n == "<http://ex.org/a>"), "self must be excluded");
        assert!(!names.iter().any(|n| n == "<http://ex.org/far>"), "disjoint entity must not appear");
        // most_similar scores must agree with the pairwise API.
        assert_eq!(top[1].1, sim.similarity(&iri("a"), &iri("half")));
        // … and literals must never be returned as entities.
        let g2 = graph(":s1 :p \"lit\" . :s2 :p \"lit\" .");
        let sim2 = unweighted(&g2);
        assert!(sim2.most_similar(&iri("s1"), 5).iter().all(|(t, _)| !matches!(t, Term::Literal(_))));
    }

    #[test]
    fn similar_by_signature_matches_most_similar_modulo_self() {
        let g = graph(
            ":a :knows :bob ; :worksAt :acme .
             :twin :knows :bob ; :worksAt :acme .
             :half :knows :bob .",
        );
        let sim = unweighted(&g);
        let sig = sim.signature(&iri("a")).unwrap();
        let by_sig = sim.similar_by_signature(&sig, 10);
        // Without an exclusion, `a` itself is the best match for its own signature.
        assert_eq!(by_sig[0].0.to_string(), "<http://ex.org/a>");
        assert_eq!(by_sig[0].1, 1.0);
        assert_eq!(by_sig[1].0.to_string(), "<http://ex.org/twin>");
    }

    // Exercise both the primary and profile-fallback candidate budgets.
    #[test]
    fn huge_k_does_not_overflow_non_sketch_candidate_budgets() {
        let g = graph(":a :plays :chess . :b :plays :tennis .");
        let sim = unweighted(&g);

        let top = sim.most_similar(&iri("a"), usize::MAX);

        assert_eq!(top, vec![(iri("b"), 1.0)]);
    }

    #[test]
    fn hub_elements_are_capped_but_rescore_is_exact() {
        // (p, hub) is shared by 6 entities; with the cap below 6 the hub element cannot
        // GENERATE candidates, but :a and :b also share the rare (q, r) pair, so :b is
        // still found — and its score is the EXACT similarity including the hub element.
        let mut ttl = String::from(":a :q :r . :b :q :r .");
        for e in ["a", "b", "c", "d", "e", "f"] {
            ttl.push_str(&format!(" :{e} :p :hub ."));
        }
        let g = graph(&ttl);
        let strict = Sim::with_config(
            &g,
            SimConfig {
                idf: false,
                max_pair_frequency: 5,
                profile_fallback: false,
                ..SimConfig::default()
            },
        );
        let top = strict.most_similar(&iri("a"), 10);
        assert_eq!(top[0].0.to_string(), "<http://ex.org/b>");
        assert_eq!(top[0].1, 1.0, "re-score must include the capped hub element");
        // c..f share only the capped hub element → not generated (the approximation;
        // fallback disabled pins the strict v1 behaviour).
        assert_eq!(top.len(), 1);
        // With the default fallback, the starved slots fill with role-similar
        // entities — BELOW the exactly-scored :b, scored by profile Jaccard
        // (c..f share 1 of a's 2 roles → 0.5).
        let sim = Sim::with_config(
            &g,
            SimConfig { idf: false, max_pair_frequency: 5, ..SimConfig::default() },
        );
        let top = sim.most_similar(&iri("a"), 10);
        assert_eq!(top[0].0.to_string(), "<http://ex.org/b>");
        assert_eq!(top[0].1, 1.0);
        assert_eq!(top.len(), 5, "fallback fills with c..f");
        assert!(top[1..].iter().all(|(_, s)| *s == 0.5));
    }

    /// The neighbor-sparse fallback (olympics Sport 0/0): entities whose every
    /// signature element points at a degree-1 neighbor generate NO candidates
    /// through (predicate, neighbor) ranges — every event names exactly one
    /// sport, so the reverse scan finds only the query itself. The fallback
    /// fills with role-similar entities scored by profile Jaccard.
    #[test]
    fn neighbor_sparse_entities_fall_back_to_profile_candidates() {
        // Star topology: each sport is named by its own events, via one predicate.
        let g = graph(
            ":e1 :inSport :s1 . :e2 :inSport :s1 .
             :e3 :inSport :s2 . :e4 :inSport :s3 .",
        );
        // Pin the STRUCTURAL starvation invariant: the lexical tier (when compiled in)
        // would legitimately fill slots here (`s2`/`s3` are lexically close to `s1`),
        // so it is disabled along with the profile fallback.
        let strict_cfg = SimConfig { idf: false, profile_fallback: false, ..SimConfig::default() };
        #[cfg(feature = "lexical")]
        let strict_cfg = SimConfig { lexical_fallback: false, ..strict_cfg };
        let strict = Sim::with_config(&g, strict_cfg);
        assert!(
            strict.most_similar(&iri("s1"), 5).is_empty(),
            "v1 starvation: no shared (predicate, neighbor) element"
        );
        let sim = unweighted(&g);
        let top = sim.most_similar(&iri("s1"), 5);
        let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
        assert_eq!(names, ["<http://ex.org/s2>", "<http://ex.org/s3>"]);
        // Both share s1's full role profile {(In, inSport)} → profile Jaccard 1.
        assert!(top.iter().all(|(_, s)| *s == 1.0));
        // The events do NOT leak in: e1..e4 have OUTGOING :inSport, not incoming.
        assert!(!names.iter().any(|n| n.starts_with("<http://ex.org/e")));
    }

    /// Exactly-scored candidates always rank ABOVE fallback entries, even when
    /// the fallback profile score is numerically higher.
    #[test]
    fn fallback_entries_rank_below_exact_matches() {
        // :a shares a concrete neighbor with :twin (exact score < 1) and only a
        // role with :role (profile score 1.0).
        let g = graph(
            ":a :knows :bob ; :likes :x1 .
             :twin :knows :bob ; :likes :x2 .
             :role :knows :y1 ; :likes :y2 .",
        );
        let sim = unweighted(&g);
        let top = sim.most_similar(&iri("a"), 5);
        let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
        assert_eq!(names[0], "<http://ex.org/twin>", "exact match first: {names:?}");
        assert_eq!(top[0].1, sim.similarity(&iri("a"), &iri("twin")));
        assert!(names.contains(&"<http://ex.org/role>".to_string()), "{names:?}");
        // No fallback when generation already yields k candidates.
        let top1 = sim.most_similar(&iri("a"), 1);
        assert_eq!(top1.len(), 1);
        assert_eq!(top1[0].0.to_string(), "<http://ex.org/twin>");
    }

    /// The accuracy gate on a GENERATED TAXONOMY (research/genai-design.md §4): entities
    /// of the same class share class-correlated structure plus per-entity noise; with
    /// rdf:type EXCLUDED from signatures, same-class pairs must rank above cross-class
    /// pairs with AUC > 0.9.
    #[test]
    fn generated_taxonomy_auc_gate() {
        const CLASSES: usize = 6;
        const PER_CLASS: usize = 20;
        // Deterministic xorshift, no rand dep.
        let mut st = 0xC0FFEE123u64;
        let mut rng = move |m: usize| {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            (st % m as u64) as usize
        };
        let mut ttl = String::new();
        for c in 0..CLASSES {
            for e in 0..PER_CLASS {
                let s = format!(":c{c}e{e}");
                ttl.push_str(&format!("{s} rdf:type :Class{c} .\n"));
                // Class-correlated structure: each class has 3 hub neighbors reached via
                // class-specific predicates (each entity links to 2 of the 3).
                for h in 0..3 {
                    if (e + h) % 3 != 0 {
                        ttl.push_str(&format!("{s} :p{c}_{h} :hub{c}_{h} .\n"));
                    }
                }
                // Noise: 2 random edges via shared predicates to random targets.
                for _ in 0..2 {
                    ttl.push_str(&format!("{s} :noise{} :n{} .\n", rng(4), rng(40)));
                }
            }
        }
        let g = graph(&ttl);
        let rdf_type = NamedNode::new("http://www.w3.org/1999/02/22-rdf-syntax-ns#type").unwrap();
        let sim = Sim::with_config(
            &g,
            SimConfig { exclude_predicates: vec![rdf_type], ..SimConfig::default() },
        );
        // All pairwise similarities over the population, labelled by same-class.
        let entities: Vec<(usize, Term)> = (0..CLASSES)
            .flat_map(|c| (0..PER_CLASS).map(move |e| (c, iri(&format!("c{c}e{e}")))))
            .collect();
        let sigs: Vec<(usize, Signature)> =
            entities.iter().map(|(c, t)| (*c, sim.signature(t).unwrap())).collect();
        let mut pos: Vec<f64> = Vec::new();
        let mut neg: Vec<f64> = Vec::new();
        for i in 0..sigs.len() {
            for j in (i + 1)..sigs.len() {
                let s = weighted_jaccard(&sigs[i].1, &sigs[j].1);
                if sigs[i].0 == sigs[j].0 {
                    pos.push(s);
                } else {
                    neg.push(s);
                }
            }
        }
        let auc = auc(&pos, &neg);
        assert!(auc > 0.9, "taxonomy AUC gate failed: {auc:.4}");
    }

    /// AUC by pairwise comparison (ties count ½) — exact, fine at test scale.
    fn auc(pos: &[f64], neg: &[f64]) -> f64 {
        let mut wins = 0.0;
        for &p in pos {
            for &n in neg {
                if p > n {
                    wins += 1.0;
                } else if p == n {
                    wins += 0.5;
                }
            }
        }
        wins / (pos.len() as f64 * neg.len() as f64)
    }

    // sq-lsp7k: the `explain` feature's explanation-surface tests.

    /// Re-encode a decoded [`SharedElement`] to the private signature tuple, for
    /// membership checks against `Signature::elems`.
    #[cfg(feature = "explain")]
    fn encode(g: &Graph, e: &SharedElement) -> (u8, Id, Id) {
        let d = match e.direction {
            Direction::Out => OUT,
            Direction::In => IN,
        };
        let p = g.id_of(&e.predicate).unwrap();
        let n = e.neighbor.as_ref().map_or(NO_ID, |t| g.id_of(t).unwrap());
        (d, p, n)
    }

    /// Hand-computed exact outputs (single shared element, so no ordering freedom):
    /// any mutation of direction, predicate, neighbor, or weight goes red.
    #[cfg(feature = "explain")]
    #[test]
    fn explain_similarity_hand_computed_exact() {
        // Same graph as `plain_jaccard_hand_computed`: the ONLY shared element is
        // (out, knows, eve); unweighted, so its weight is exactly 1.0.
        let g = graph(":alice :knows :bob, :eve ; :worksAt :acme . :dave :knows :eve ; :plays :chess .");
        let sim = unweighted(&g);
        let expected = vec![SharedElement {
            direction: Direction::Out,
            predicate: iri("knows"),
            neighbor: Some(iri("eve")),
            weight: 1.0,
        }];
        assert_eq!(sim.explain_similarity(&iri("alice"), &iri("dave")), expected);
        assert_eq!(sim.explain_similarity(&iri("dave"), &iri("alice")), expected);
        // Σ / (|A| + |B| − Σ) = 1 / (3 + 2 − 1) reconstructs the 0.25 score exactly.
        assert_eq!(sim.similarity(&iri("alice"), &iri("dave")), 0.25);
        assert_eq!(1.0 / (3.0 + 2.0 - 1.0), 0.25);

        // Incoming-only shared context: a and b share ONLY x's incoming :p edges.
        let g = graph(":x :p :a . :x :p :b .");
        let sim = unweighted(&g);
        let ex = sim.explain_similarity(&iri("a"), &iri("b"));
        assert_eq!(
            ex,
            vec![SharedElement {
                direction: Direction::In,
                predicate: iri("p"),
                neighbor: Some(iri("x")),
                weight: 1.0,
            }]
        );
        assert_eq!(sim.similarity(&iri("a"), &iri("b")), 1.0); // 1 / (1 + 1 − 1)
    }

    /// The load-bearing differential: for every pair, Σ(explain weights) plugged into
    /// Σ / (|A| + |B| − Σ) reconstructs `similarity(a, b)` EXACTLY (IDF weights on),
    /// every returned element is present in BOTH signatures, and the element count
    /// equals the true intersection size (no unique-to-one or missing element).
    #[cfg(feature = "explain")]
    #[test]
    fn explain_similarity_weights_reconstruct_score() {
        let g = graph(
            ":alice :knows :bob, :eve ; :worksAt :acme .
             :carol :knows :bob, :eve ; :worksAt :acme .
             :dave :knows :eve ; :plays :chess .
             :far :plays :go .",
        );
        let sim = Sim::new(&g); // IDF weighting on — weights differ per predicate
        let pairs = [
            ("alice", "carol", 3), // identical signatures
            ("alice", "dave", 1),  // partial overlap
            ("alice", "far", 0),   // disjoint
            ("dave", "far", 0),    // same predicate, different neighbor: disjoint
        ];
        for (a, b, expected_len) in pairs {
            let (ta, tb) = (iri(a), iri(b));
            let ex = sim.explain_similarity(&ta, &tb);
            assert_eq!(ex.len(), expected_len, "{a} vs {b}");
            // Symmetric: same shared set, same min-weights, same deterministic order.
            assert_eq!(ex, sim.explain_similarity(&tb, &ta), "{a} vs {b}");
            let (sa, sb) = (sim.signature(&ta).unwrap(), sim.signature(&tb).unwrap());
            // Every element genuinely present in BOTH signatures …
            for e in &ex {
                let t = encode(&g, e);
                assert!(sa.elems.binary_search(&t).is_ok(), "{t:?} not in sig({a})");
                assert!(sb.elems.binary_search(&t).is_ok(), "{t:?} not in sig({b})");
                assert!(e.weight > 0.0);
            }
            // … and NO true intersection element missing.
            let inter = sa.elems.iter().filter(|e| sb.elems.binary_search(e).is_ok()).count();
            assert_eq!(ex.len(), inter, "{a} vs {b}");
            // Σ(explain) reconstructs the exact weighted-Jaccard score.
            let sigma: f64 = ex.iter().map(|e| e.weight).sum();
            let score = sim.similarity(&ta, &tb);
            if sigma == 0.0 {
                assert_eq!(score, 0.0, "{a} vs {b}");
            } else {
                assert_close(sigma / (sa.total_weight() + sb.total_weight() - sigma), score);
            }
        }
        // Non-vacuity anchors: the overlapping pairs really scored.
        assert_eq!(sim.similarity(&iri("alice"), &iri("carol")), 1.0);
        assert!(sim.similarity(&iri("alice"), &iri("dave")) > 0.0);
    }

    /// Deterministic ordering: weight DESCENDING first (the rare, higher-IDF predicate
    /// beats the common one even with a larger dictionary id), then ids ASCENDING on ties.
    #[cfg(feature = "explain")]
    #[test]
    fn explain_similarity_orders_weight_desc_then_ids_asc() {
        let g = graph(
            ":a :common :c1 ; :rare :r1 . :b :common :c1 ; :rare :r1 .
             :f0 :common :x0 . :f1 :common :x1 . :f2 :common :x2 .",
        );
        let sim = Sim::new(&g); // freq(common)=5 > freq(rare)=2 → w(rare) > w(common)
        let ex = sim.explain_similarity(&iri("a"), &iri("b"));
        assert_eq!(ex.len(), 2);
        assert_eq!(ex[0].predicate, iri("rare"));
        assert_eq!(ex[1].predicate, iri("common"));
        assert!(ex[0].weight > ex[1].weight);

        // Equal weights (same predicate): neighbor id ascending breaks the tie.
        let g = graph(":a :p :n1, :n2 . :b :p :n1, :n2 .");
        let sim = unweighted(&g);
        let ex = sim.explain_similarity(&iri("a"), &iri("b"));
        assert!(ex.iter().all(|e| e.weight == 1.0));
        let (n1, n2) = (g.id_of(&iri("n1")).unwrap(), g.id_of(&iri("n2")).unwrap());
        let expected = if n1 < n2 {
            vec![Some(iri("n1")), Some(iri("n2"))]
        } else {
            vec![Some(iri("n2")), Some(iri("n1"))]
        };
        let got: Vec<Option<Term>> = ex.iter().map(|e| e.neighbor.clone()).collect();
        assert_eq!(got, expected);
    }

    /// [`SignatureMode::Predicates`] elements have no neighbor: `neighbor` is `None`,
    /// and the reconstruction identity holds against that mode's similarity.
    #[cfg(feature = "explain")]
    #[test]
    fn explain_similarity_predicates_mode_has_no_neighbor() {
        let g = graph(":a :knows :x ; :worksAt :acme . :b :knows :y ; :worksAt :corp .");
        let sim = Sim::with_config(
            &g,
            SimConfig { mode: SignatureMode::Predicates, idf: false, ..SimConfig::default() },
        );
        let ex = sim.explain_similarity(&iri("a"), &iri("b"));
        assert_eq!(ex.len(), 2, "shared roles: knows + worksAt");
        assert!(ex.iter().all(|e| e.neighbor.is_none()));
        let preds: Vec<&Term> = ex.iter().map(|e| &e.predicate).collect();
        assert!(preds.contains(&&iri("knows")) && preds.contains(&&iri("worksAt")));
        let sigma: f64 = ex.iter().map(|e| e.weight).sum();
        assert_eq!(sigma / (2.0 + 2.0 - sigma), 1.0);
        assert_eq!(sim.similarity(&iri("a"), &iri("b")), 1.0);
    }

    /// Absent terms and empty intersections explain to an empty Vec; self-similarity
    /// explains to the full signature and reconstructs 1.
    #[cfg(feature = "explain")]
    #[test]
    fn explain_similarity_absent_disjoint_and_self() {
        let g = graph(":a :p :x . :b :q :y .");
        let sim = unweighted(&g);
        assert!(sim.explain_similarity(&iri("a"), &iri("missing")).is_empty());
        assert!(sim.explain_similarity(&iri("missing"), &iri("a")).is_empty());
        assert!(sim.explain_similarity(&iri("a"), &iri("b")).is_empty());
        let ex = sim.explain_similarity(&iri("a"), &iri("a"));
        assert_eq!(ex.len(), sim.signature(&iri("a")).unwrap().len());
        let sigma: f64 = ex.iter().map(|e| e.weight).sum();
        let total = sim.signature(&iri("a")).unwrap().total_weight();
        assert_eq!(sigma / (total + total - sigma), 1.0);
    }

    /// Under multi-hop attenuation the two signatures can weight one shared element
    /// DIFFERENTLY (nearest hop wins per side); the explanation must carry
    /// `min(w_a, w_b)` — exactly the score's numerator term — so the reconstruction
    /// stays exact. A `min → max` mutation goes red here (at depth 1 the weights
    /// coincide, so only this differential can catch it).
    #[cfg(all(feature = "explain", feature = "multi-hop"))]
    #[test]
    fn explain_similarity_multi_hop_uses_min_weight() {
        // a reaches (out, q, c) at hop 2 (weight 0.5); x has it at hop 1 (weight 1.0).
        let g = graph(":a :p :b . :b :q :c . :x :q :c .");
        let sim = multi_hop_unweighted(&g, 2);
        let ex = sim.explain_similarity(&iri("a"), &iri("x"));
        assert_eq!(
            ex,
            vec![SharedElement {
                direction: Direction::Out,
                predicate: iri("q"),
                neighbor: Some(iri("c")),
                weight: 0.5,
            }]
        );
        let ta = sim.signature(&iri("a")).unwrap().total_weight();
        let tx = sim.signature(&iri("x")).unwrap().total_weight();
        let sigma: f64 = ex.iter().map(|e| e.weight).sum();
        let score = sim.similarity(&iri("a"), &iri("x"));
        assert_eq!(sigma / (ta + tx - sigma), score);
        assert_close(score, 0.2); // 0.5 / (1.5 + 1.5 − 0.5)
    }

    // ---- tbox feature tests -------------------------------------------------------

    /// T-box-aware signature of `:alice rdf:type :Professor`; `:Professor rdfs:subClassOf :Person`
    /// must include the inferred `(OUT, rdf:type, :Person)` element at 0.5× the direct weight.
    /// sq-181
    #[cfg(feature = "tbox")]
    #[test]
    fn tbox_signature_expands_supertype() {
        let g = graph(
            ":alice rdf:type :Professor .
             :Professor <http://www.w3.org/2000/01/rdf-schema#subClassOf> :Person .",
        );
        let tbox_sim = Sim::with_config(
            &g,
            SimConfig { idf: false, tbox_aware: true, ..SimConfig::default() },
        );
        let sig = tbox_sim.signature(&iri("alice")).unwrap();
        let rdf_type_id = g
            .id_of(&Term::NamedNode(
                NamedNode::new("http://www.w3.org/1999/02/22-rdf-syntax-ns#type").unwrap(),
            ))
            .expect("rdf:type must be in the graph");
        let person_id = g.id_of(&iri("Person")).expect(":Person must be in the graph");
        let professor_id = g.id_of(&iri("Professor")).expect(":Professor must be in the graph");
        // Direct type element present.
        assert!(sig.elems.contains(&(OUT, rdf_type_id, professor_id)), "direct type missing");
        // Inferred supertype element present.
        assert!(sig.elems.contains(&(OUT, rdf_type_id, person_id)), "inferred supertype missing");
        // Inferred element has attenuated weight (0.5 × direct weight = 0.5 × 1.0 = 0.5).
        let pos = sig
            .elems
            .iter()
            .position(|&e| e == (OUT, rdf_type_id, person_id))
            .unwrap();
        assert!((sig.weights[pos] - 0.5).abs() < 1e-12, "inferred weight should be 0.5");
    }

    /// Without T-box, alice (Professor) and bob (Person) share no structural element.
    /// With T-box, alice's inferred `(OUT, rdf:type, :Person)` matches bob's direct
    /// `(OUT, rdf:type, :Person)` — non-zero similarity. sq-181
    #[cfg(feature = "tbox")]
    #[test]
    fn tbox_awareness_creates_cross_subclass_similarity() {
        let g = graph(
            ":alice rdf:type :Professor .
             :bob   rdf:type :Person .
             :Professor <http://www.w3.org/2000/01/rdf-schema#subClassOf> :Person .",
        );
        let plain = Sim::with_config(
            &g,
            SimConfig { idf: false, tbox_aware: false, ..SimConfig::default() },
        );
        let tbox_sim = Sim::with_config(
            &g,
            SimConfig { idf: false, tbox_aware: true, ..SimConfig::default() },
        );
        assert_eq!(
            plain.similarity(&iri("alice"), &iri("bob")),
            0.0,
            "without T-box there is no shared element"
        );
        assert!(
            tbox_sim.similarity(&iri("alice"), &iri("bob")) > 0.0,
            "with T-box the inferred supertype bridges the gap"
        );
    }

    /// T-box with no subClassOf / subPropertyOf triples is a no-op — signatures are
    /// identical to the plain (tbox_aware = false) mode. sq-181
    #[cfg(feature = "tbox")]
    #[test]
    fn tbox_noop_when_no_hierarchy() {
        let g = graph(":a :p :b . :c :p :b .");
        let plain = Sim::with_config(
            &g,
            SimConfig { idf: false, tbox_aware: false, ..SimConfig::default() },
        );
        let tbox_sim = Sim::with_config(
            &g,
            SimConfig { idf: false, tbox_aware: true, ..SimConfig::default() },
        );
        let sa_plain = plain.signature(&iri("a")).unwrap();
        let sa_tbox = tbox_sim.signature(&iri("a")).unwrap();
        assert_eq!(sa_plain.elems, sa_tbox.elems, "elements must be identical without hierarchy");
        assert_eq!(sa_plain.weights, sa_tbox.weights, "weights must be identical without hierarchy");
    }

    /// Sibling-subproperty RETRIEVAL: `:a :p1 :n` and `:b :p2 :n` share ONLY the
    /// T-box-inferred `(OUT, :p, :n)` element (`:p1`, `:p2` ⊑ `:p`); no triple uses
    /// `:p` directly, so candidate generation must scan the subproperty down-closure
    /// of the inferred element — a direct-pattern scan finds nothing. All fallback
    /// tiers are disabled so the test cannot pass vacuously.
    #[cfg(feature = "tbox")]
    #[test]
    fn tbox_most_similar_finds_sibling_subproperty_entities() {
        let g = graph(
            ":p1 <http://www.w3.org/2000/01/rdf-schema#subPropertyOf> :p .
             :p2 <http://www.w3.org/2000/01/rdf-schema#subPropertyOf> :p .
             :a :p1 :n .
             :b :p2 :n .",
        );
        let cfg = SimConfig { idf: false, profile_fallback: false, ..SimConfig::default() };
        #[cfg(feature = "lexical")]
        let cfg = SimConfig { lexical_fallback: false, ..cfg };
        // Without T-box awareness (and no fallbacks) :b is unreachable from :a.
        let plain = Sim::with_config(&g, SimConfig { tbox_aware: false, ..cfg.clone() });
        assert!(
            plain.most_similar(&iri("a"), 5).is_empty(),
            "without T-box, :a and :b share no element"
        );
        let tbox_sim = Sim::with_config(&g, cfg);
        // Direct similarity is positive via the shared inferred (:p, :n) element …
        let pairwise = tbox_sim.similarity(&iri("a"), &iri("b"));
        assert!(pairwise > 0.0, "expanded signatures must share (:p, :n)");
        // … and RETRIEVAL must agree: :b is only generated by scanning (?, :p2, :n)
        // from the down-closure of the inferred element.
        let top = tbox_sim.most_similar(&iri("a"), 5);
        let b_score = top
            .iter()
            .find(|(t, _)| t.to_string() == "<http://ex.org/b>")
            .map(|&(_, s)| s)
            .expect("sibling-subproperty entity :b must be retrieved");
        assert_eq!(b_score, pairwise, "retrieval score must equal the pairwise score");
    }

    // ---- lexical feature tests ----------------------------------------------------

    /// `trigram_jaccard` sanity: identical strings → 1, disjoint → 0, partial → (0,1).
    /// sq-181
    #[cfg(feature = "lexical")]
    #[test]
    fn trigram_jaccard_basic() {
        assert_close(trigram_jaccard("alice", "alice"), 1.0);
        assert_eq!(trigram_jaccard("abc", "xyz"), 0.0);
        let partial = trigram_jaccard("alice_smith", "alice_jones");
        assert!(partial > 0.0 && partial < 1.0, "partial overlap: {partial}");
        assert!(partial > trigram_jaccard("alice_smith", "bob_jones"),
                "alice_smith closer to alice_jones than to bob_jones");
        // Short-string fallback
        assert_close(trigram_jaccard("ab", "ab"), 1.0);
        assert_eq!(trigram_jaccard("ab", "cd"), 0.0);
    }

    /// `trigram_jaccard` operates on CHARACTER trigrams, not UTF-8 bytes: a
    /// multi-byte character is one trigram position, and the short-string gate
    /// counts characters. Every expected value here differs from what byte-window
    /// trigrams would produce.
    #[cfg(feature = "lexical")]
    #[test]
    fn trigram_jaccard_uses_character_trigrams() {
        // Char trigrams: {hél, éll, llo} vs {hal, all, llo} → 1/5. Byte windows
        // would split the two-byte `é` into 4-vs-3 trigrams sharing one → 1/6.
        assert_close(trigram_jaccard("héllo", "hallo"), 0.2);
        // Two CHARACTERS (six bytes) take the short-string LCP path: 1 shared
        // leading char / 2 → 0.5. Byte windows would score 1/7 via trigrams.
        assert_close(trigram_jaccard("日本", "日中"), 0.5);
        // Identical two-char non-ASCII strings → 1.0 via the char LCP path.
        assert_close(trigram_jaccard("日本", "日本"), 1.0);
    }

    /// Lexical fallback finds entities with similar IRI local names even when
    /// structural (and profile) candidate generation yields nothing. sq-181
    #[cfg(feature = "lexical")]
    #[test]
    fn lexical_fallback_finds_similar_local_names() {
        // Three entities with completely disjoint structural signatures (unique predicates)
        // but sharing a local-name prefix. Profile fallback is disabled.
        let g = graph(
            ":alice_smith :uniq1 :x1 .
             :alice_jones :uniq2 :x2 .
             :bob_clark   :uniq3 :x3 .",
        );
        let sim_with = Sim::with_config(
            &g,
            SimConfig {
                idf: false,
                profile_fallback: false,
                lexical_fallback: true,
                ..SimConfig::default()
            },
        );
        let sim_without = Sim::with_config(
            &g,
            SimConfig {
                idf: false,
                profile_fallback: false,
                lexical_fallback: false,
                ..SimConfig::default()
            },
        );
        // Without lexical fallback: no candidates at all (disjoint structure + no profile).
        assert!(
            sim_without.most_similar(&iri("alice_smith"), 5).is_empty(),
            "no structural or profile candidates"
        );
        // With lexical fallback: alice_jones must appear (shares prefix "ali").
        let top = sim_with.most_similar(&iri("alice_smith"), 5);
        let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
        assert!(
            names.contains(&"<http://ex.org/alice_jones>".to_string()),
            "lexical tier must find alice_jones; got: {names:?}"
        );
        // alice_smith itself must not appear in its own results.
        assert!(!names.contains(&"<http://ex.org/alice_smith>".to_string()), "self must be excluded");
    }

    /// `lexical_fallback` ranks BELOW structural exact results. sq-181
    #[cfg(feature = "lexical")]
    #[test]
    fn lexical_fallback_ranks_below_structural_results() {
        // :a and :twin share a structural neighbor; :alice_a has a similar local name.
        let g = graph(
            ":a :knows :bob .
             :twin :knows :bob .
             :alice_a :other :x .",
        );
        let sim = Sim::with_config(
            &g,
            SimConfig { idf: false, lexical_fallback: true, ..SimConfig::default() },
        );
        let top = sim.most_similar(&iri("a"), 5);
        // :twin must appear before any lexical candidate.
        let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
        assert_eq!(names[0], "<http://ex.org/twin>", "structural match must lead; got {names:?}");
    }

    /// Trigram overlap does not require a shared prefix: `xalice` shares the
    /// `ali`/`lic`/`ice` trigrams with `alice` despite a different first byte, so the
    /// lexical tier must return it (a prefix-bounded candidate scan would miss it).
    /// sq-181
    #[cfg(feature = "lexical")]
    #[test]
    fn lexical_fallback_matches_across_different_prefixes() {
        let g = graph(
            ":alice  :uniq1 :x1 .
             :xalice :uniq2 :x2 .
             :zzz_unrelated :uniq3 :x3 .",
        );
        let sim = Sim::with_config(
            &g,
            SimConfig {
                idf: false,
                profile_fallback: false,
                lexical_fallback: true,
                ..SimConfig::default()
            },
        );
        let top = sim.most_similar(&iri("alice"), 5);
        let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
        assert!(
            names.contains(&"<http://ex.org/xalice>".to_string()),
            "positive-Jaccard match with a different prefix must be found; got: {names:?}"
        );
        // Zero trigram overlap → not a candidate.
        assert!(
            !names.contains(&"<http://ex.org/zzz_unrelated>".to_string()),
            "zero-Jaccard key must not appear; got: {names:?}"
        );
    }

    /// The lexical tier must not panic on non-ASCII local names. In `abécédaire`,
    /// byte offset 3 falls inside the two-byte `é`, so any byte-offset string slice
    /// of the query key would panic. sq-181
    #[cfg(feature = "lexical")]
    #[test]
    fn lexical_fallback_handles_multibyte_local_names() {
        let g = graph(
            "<http://ex.org/abécédaire> :uniq1 :x1 .
             <http://ex.org/abécédé>    :uniq2 :x2 .",
        );
        let sim = Sim::with_config(
            &g,
            SimConfig {
                idf: false,
                profile_fallback: false,
                lexical_fallback: true,
                ..SimConfig::default()
            },
        );
        let top = sim.most_similar(&iri("abécédaire"), 5);
        let names: Vec<String> = top.iter().map(|(t, _)| t.to_string()).collect();
        assert!(
            names.contains(&"<http://ex.org/abécédé>".to_string()),
            "non-ASCII trigram match must be found; got: {names:?}"
        );
    }
}
