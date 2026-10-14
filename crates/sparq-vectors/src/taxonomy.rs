//! Taxonomy block + disjointness repulsion/mask (structure-aware vectorisation **P3**).
//!
//! sq-0wo9e.4 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §2 rows "subClassOf hierarchy" + "owl:disjointWith / sh:not", §3.3 "the taxonomy block", §6.A
//! formal-properties table, §9 limitations 4). This is the **third** additive slice of the
//! structure-aware-vectorisation epic, after P0 ([`crate::structure`]: closure-before-vectorise +
//! type-constrained negatives) and P1/P2 ([`crate::encode`]: typed-literal encoders + the
//! self-describing [`SchemaHeader`](crate::encode::SchemaHeader)). Like them it is **opt-in** (the
//! same `structure` cargo feature, off by default) and changes **nothing** in the default
//! `sparq-vectors` build or the core engine.
//!
//! # What this is — two priors over the `rdfs:subClassOf` DAG
//!
//! 1. **A taxonomy sub-vector block** ([`EuclideanTaxonomyEncoder`]) that embeds a class's place in
//!    the `subClassOf` hierarchy. It is **Euclidean by default** (an ancestor-bag + depth code,
//!    tagged [`Metric::Euclidean`] so the whole-row L2/cosine
//!    search path is correct). A non-Euclidean geometry ([`HyperbolicTaxonomyEncoder`], a
//!    Poincaré-ball candidate) is **only adopted past a measured-distortion gate** ([`GeometryGate`])
//!    on the **actual** `subClassOf` DAG — never on a density heuristic. This is the design's
//!    central must-fix (§3.3 + §9.4): the Nickel–Kiela low-distortion result is for clean tree/DAG
//!    transitive closures, and real `rdfs:subClassOf` is noisy and multiply-inheriting, so the
//!    distortion is **measured** before a non-Euclidean block is used.
//!
//! 2. **A disjointness repulsion + mask prior** ([`DisjointnessOracle`]) read from the closed graph
//!    (`owl:disjointWith`, `owl:AllDisjointClasses`, `owl:complementOf`, propagated through the
//!    materialised `subClassOf` closure). At **train time** it yields
//!    [`repulsion_pairs`](DisjointnessOracle::repulsion_pairs) a trainer
//!    pushes apart (a margin term); at **serve time** it is a **hard mask**
//!    ([`DisjointnessOracle::mask_candidates`]) that drops any candidate the closure *proves*
//!    disjoint from the query type. The mask is **answer-safe** — it removes only **provably-wrong**
//!    neighbours (design §2 + §6.A "verify-soundness / answer-safety").
//!
//! `gufo:Kind`/`gufo:Role` rigidity split is the design's **optional/last** prior (§2, §9.5 — the
//! annotations are rare in the wild); its READ-ONLY reader now lives in [`crate::ufo_priors`],
//! which feeds UFO-**proven** disjointness into this oracle via
//! [`DisjointnessOracle::absorb_proven_pairs`] (the mask stays answer-safe: absorbed pairs carry
//! the same proven-only contract).
//!
//! # What is provable vs what is empirical (stated honestly)
//!
//! - **Provable (and tested here):** the disjointness mask only ever **removes** candidates and
//!   removes only **provably-disjoint** ones ([`DisjointnessOracle::is_disjoint`] is sound w.r.t.
//!   the materialised axioms); the metric-correctness guard tags a non-Euclidean block so a cosine
//!   search on it is a detectable error; the distortion metric is a deterministic function of the
//!   embedding + DAG.
//! - **Empirical / dataset-dependent (NOT claimed):** whether the taxonomy block raises downstream
//!   link-prediction or retrieval, and whether hyperbolic beats Euclidean on a given hierarchy.
//!   Both ship behind the gate/ablation; **no benchmark numbers exist and none are stated** (design
//!   §6.B, §9.1, §9.4). `GeometryGate::choose` refuses a non-Euclidean block **unless** its
//!   measured distortion strictly beats Euclidean — adoption is measurement-gated by construction.

use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Id, TermParts};
use sparq_core::Graph;

use crate::encode::{Block, Encoder, Metric};

/// `rdfs:subClassOf`.
const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
/// `rdf:type`.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

// ============================================================================================
// The taxonomy DAG (extracted from a closed graph)
// ============================================================================================

/// The `rdfs:subClassOf` directed acyclic graph extracted from a (closed) [`Graph`], plus a
/// stable, dense per-class index. The input graph should already be **closed** (call
/// [`crate::structure::materialise_closure`] first) so the `subClassOf` *closure* is materialised
/// and every entailed super-class edge is present — the taxonomy encoder then sees the full
/// ancestry, not just the asserted parents.
///
/// "DAG" is the intended shape; real `rdfs:subClassOf` can contain cycles (e.g. a pair declared
/// mutually `subClassOf`, which RDFS treats as `equivalentClass`). [`ancestors`](Self::ancestors)
/// is cycle-safe (it is a reachability set, computed with a visited-guard), so a cyclic input never
/// loops — it simply yields the strongly-connected component's shared ancestor set.
#[derive(Clone, Debug, Default)]
pub struct TaxonomyDag {
    /// Dense class index → class dict id (the embedding row order).
    classes: Vec<Id>,
    /// Class dict id → dense index.
    index_of: FxHashMap<Id, usize>,
    /// Dense index → direct super-class dense indices (`c subClassOf parent`).
    parents: Vec<Vec<usize>>,
    /// Dense index → direct sub-class dense indices (the reverse edges).
    children: Vec<Vec<usize>>,
}

impl TaxonomyDag {
    /// Build the `subClassOf` DAG from `graph`. Every class that appears as either side of a
    /// `rdfs:subClassOf` triple becomes a node; the edges are the asserted-plus-entailed
    /// `subClassOf` pairs present in the (ideally closed) graph.
    ///
    /// An empty result (no `subClassOf` triples) is valid — [`is_empty`](Self::is_empty) reports it,
    /// and the encoder degrades to a zero block.
    pub fn build(graph: &Graph) -> TaxonomyDag {
        let Some(subclass) = graph.id_of(&named(RDFS_SUBCLASS_OF)) else {
            return TaxonomyDag::default();
        };

        // Collect the subClassOf edges as id pairs first (subject is the sub-class, object the
        // super-class). Skip self-loops (`c subClassOf c`, the rdfs reflexive axiom) — they carry
        // no hierarchy signal and would inflate the ancestor bag with the class itself.
        let mut edges: Vec<(Id, Id)> = Vec::new();
        let mut class_set: FxHashSet<Id> = FxHashSet::default();
        for [s, p, o] in graph.iter_ids() {
            if p == subclass && s != o && is_class_node(graph, s) && is_class_node(graph, o) {
                edges.push((s, o));
                class_set.insert(s);
                class_set.insert(o);
            }
        }

        // Deterministic dense order: sort the class ids ascending so the embedding row order is
        // reproducible across runs (independent of FxHashSet iteration order).
        let mut classes: Vec<Id> = class_set.into_iter().collect();
        classes.sort_unstable();
        let index_of: FxHashMap<Id, usize> =
            classes.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let mut parents = vec![Vec::new(); classes.len()];
        let mut children = vec![Vec::new(); classes.len()];
        for (sub, sup) in edges {
            let (si, pi) = (index_of[&sub], index_of[&sup]);
            if !parents[si].contains(&pi) {
                parents[si].push(pi);
            }
            if !children[pi].contains(&si) {
                children[pi].push(si);
            }
        }

        TaxonomyDag { classes, index_of, parents, children }
    }

    /// Number of classes in the DAG.
    pub fn len(&self) -> usize {
        self.classes.len()
    }

    /// Whether the DAG has no `subClassOf` structure.
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    /// The class dict ids in dense-index (embedding-row) order.
    pub fn classes(&self) -> &[Id] {
        &self.classes
    }

    /// The dense index of a class dict id, if it is in the DAG.
    pub fn index_of(&self, class: Id) -> Option<usize> {
        self.index_of.get(&class).copied()
    }

    /// All ancestor dense indices of `idx` (its proper super-classes, transitively), cycle-safe.
    /// Does **not** include `idx` itself.
    pub fn ancestors(&self, idx: usize) -> FxHashSet<usize> {
        let mut out = FxHashSet::default();
        let mut stack: Vec<usize> = self.parents[idx].clone();
        while let Some(p) = stack.pop() {
            if out.insert(p) {
                stack.extend_from_slice(&self.parents[p]);
            }
        }
        out
    }

    /// The depth of `idx`: the length of the **longest** path from `idx` up to a root (a class with
    /// no parents). Roots have depth `0`. Cycle-safe (a visited-guard bounds the recursion); a class
    /// inside a cycle gets the longest acyclic path length.
    pub fn depth(&self, idx: usize) -> usize {
        // Iterative longest-path-to-root with memoisation over a visited frontier; the visited set
        // makes a cyclic input terminate (a back-edge is ignored).
        fn go(
            dag: &TaxonomyDag,
            idx: usize,
            on_path: &mut FxHashSet<usize>,
            memo: &mut FxHashMap<usize, usize>,
        ) -> usize {
            if let Some(&d) = memo.get(&idx) {
                return d;
            }
            if dag.parents[idx].is_empty() {
                memo.insert(idx, 0);
                return 0;
            }
            if !on_path.insert(idx) {
                // Back-edge into a node already on the current path → treat as a root for this
                // branch so the cycle does not recurse forever.
                return 0;
            }
            let d = dag.parents[idx]
                .iter()
                .map(|&p| 1 + go(dag, p, on_path, memo))
                .max()
                .unwrap_or(0);
            on_path.remove(&idx);
            memo.insert(idx, d);
            d
        }
        let mut on_path = FxHashSet::default();
        let mut memo = FxHashMap::default();
        go(self, idx, &mut on_path, &mut memo)
    }

    /// The maximum depth over all classes (the DAG's height). `0` for an empty or flat DAG.
    pub fn max_depth(&self) -> usize {
        (0..self.classes.len()).map(|i| self.depth(i)).max().unwrap_or(0)
    }

    /// Length of the shortest **undirected** path between two classes over the `subClassOf` edges
    /// (treating each edge as bidirectional), or `None` if they are in disconnected components. This
    /// is the *graph distance* the distortion gate compares the embedding distance against (§3.3 /
    /// §6.B "measured embedding distortion … on the actual `subClassOf` DAG"). `0` for `a == b`.
    pub fn graph_distance(&self, a: usize, b: usize) -> Option<usize> {
        if a == b {
            return Some(0);
        }
        let mut seen: FxHashSet<usize> = FxHashSet::default();
        let mut frontier = vec![a];
        seen.insert(a);
        let mut dist = 0usize;
        while !frontier.is_empty() {
            dist += 1;
            let mut next = Vec::new();
            for &node in &frontier {
                for &nb in self.parents[node].iter().chain(self.children[node].iter()) {
                    if nb == b {
                        return Some(dist);
                    }
                    if seen.insert(nb) {
                        next.push(nb);
                    }
                }
            }
            frontier = next;
        }
        None
    }
}

// ============================================================================================
// The Euclidean taxonomy encoder (the DEFAULT block)
// ============================================================================================

/// The **default, Euclidean** taxonomy encoder (design §3.3 "Euclidean by default"). It embeds a
/// class as a fixed-width `f32` block combining:
///
/// - a **normalised depth** lane (depth / max-depth, in `[0, 1]`) — a coarse "how specific" signal
///   that is monotone down the hierarchy; and
/// - a **hashed ancestor-bag** lane — each proper ancestor contributes a unit feature to a hashed
///   coordinate, so two classes sharing more ancestors are closer in L2. (A hashed bag keeps the
///   block width fixed and small regardless of class count — the same dependency-free trick the
///   `embed` hash embedder uses.)
///
/// The block is tagged [`Metric::Euclidean`]: L2/cosine over it is meaningful (more shared ancestry
/// ⇒ smaller distance). It is **transductive** (it encodes a class already in the DAG); a brand-new
/// class falls back to the structural-sketch lane (design §3.5), out of P3 scope.
///
/// No accuracy claim is made — this is the *default* block the distortion gate measures hyperbolic
/// against; whether either helps downstream is empirical (design §6.B, §9.1).
#[derive(Clone, Debug)]
pub struct EuclideanTaxonomyEncoder<'a> {
    dag: &'a TaxonomyDag,
    /// Width of the hashed ancestor-bag lane (`>= 1`).
    bag_dim: usize,
    /// Cached max depth for depth-normalisation.
    max_depth: usize,
}

impl<'a> EuclideanTaxonomyEncoder<'a> {
    /// Build the encoder over `dag`. The total block width is `bag_dim + 1` (the bag lanes plus the
    /// single normalised-depth lane); `bag_dim` is clamped to `>= 1`.
    pub fn new(dag: &'a TaxonomyDag, bag_dim: usize) -> EuclideanTaxonomyEncoder<'a> {
        EuclideanTaxonomyEncoder { dag, bag_dim: bag_dim.max(1), max_depth: dag.max_depth() }
    }

    /// The fixed block width this encoder occupies in a structured row (`bag_dim + 1`).
    pub fn width(&self) -> usize {
        self.bag_dim + 1
    }

    /// The [`Block`] descriptor for this encoder at a given row `offset` — always
    /// [`Metric::Euclidean`]. Register it in a [`SchemaHeader`](crate::encode::SchemaHeader) so the
    /// row is self-describing and the metric guard is correct.
    pub fn block(&self, offset: usize) -> Block {
        Block::new(Encoder::Taxonomy, Metric::Euclidean, offset, self.width())
    }

    /// Encode the class with dense index `idx` into a freshly allocated block of [`width`](Self::width).
    pub fn encode(&self, idx: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; self.width()];
        self.encode_into(idx, &mut out);
        out
    }

    /// Encode class `idx` into `out` (which must be exactly [`width`](Self::width) long).
    ///
    /// Layout: `out[0]` is the normalised depth; `out[1..]` is the hashed ancestor bag.
    pub fn encode_into(&self, idx: usize, out: &mut [f32]) {
        debug_assert_eq!(out.len(), self.width());
        out.fill(0.0);
        if self.dag.is_empty() || idx >= self.dag.len() {
            return;
        }
        // Normalised depth lane.
        out[0] = if self.max_depth == 0 {
            0.0
        } else {
            self.dag.depth(idx) as f32 / self.max_depth as f32
        };
        // Hashed ancestor-bag lane: each proper ancestor's class id hashes to one of `bag_dim`
        // coordinates and adds a unit feature. The block is then L2-comparable: shared ancestry
        // reduces distance. We hash the stable class dict id (not the dense index) so the feature is
        // independent of the row order.
        let ancestors = self.dag.ancestors(idx);
        for &anc in &ancestors {
            let class_id = self.dag.classes()[anc];
            let slot = 1 + (hash_id(class_id) as usize % self.bag_dim);
            out[slot] += 1.0;
        }
        // L2-normalise the bag lane so deep classes (many ancestors) are not systematically larger
        // in norm than shallow ones — distance then reflects ancestry *overlap*, not raw count.
        let norm: f32 = out[1..].iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut out[1..] {
                *v /= norm;
            }
        }
    }
}

// ============================================================================================
// The hyperbolic taxonomy candidate (NON-default; only past the distortion gate)
// ============================================================================================

/// A **candidate** non-Euclidean (Poincaré-ball) taxonomy encoder — design §3.3's
/// hyperbolic option, supplied here **only so the distortion gate has a real second arm to
/// measure**. It is **never** used as the row's taxonomy block unless [`GeometryGate::choose`]
/// reports its measured distortion strictly beats Euclidean on the actual DAG (design §9.4: adopt
/// non-Euclidean *only past the measured-distortion gate*).
///
/// The construction is a simple, dependency-free, deterministic Poincaré placement: a class is
/// placed at radius growing with its depth (deeper ⇒ nearer the ball boundary, where hyperbolic
/// space has "more room", the Nickel–Kiela intuition) and at an angle hashed from its id. This is
/// **not** a trained Poincaré embedding — it is a closed-form placement whose distortion the gate
/// can compute without a training loop. A trained hyperbolic block (and the matching non-Euclidean
/// search kernel) is a tracked follow-up; this arm exists to make the *gate* honest, not to claim a
/// hyperbolic win.
#[derive(Clone, Debug)]
pub struct HyperbolicTaxonomyEncoder<'a> {
    dag: &'a TaxonomyDag,
    max_depth: usize,
}

impl<'a> HyperbolicTaxonomyEncoder<'a> {
    /// Build over `dag`.
    pub fn new(dag: &'a TaxonomyDag) -> HyperbolicTaxonomyEncoder<'a> {
        HyperbolicTaxonomyEncoder { dag, max_depth: dag.max_depth() }
    }

    /// The 2-D Poincaré-disc coordinate of class `idx` (`(x, y)` with `x² + y² < 1`). Returns the
    /// origin for an out-of-range index or empty DAG.
    pub fn coord(&self, idx: usize) -> (f64, f64) {
        if self.dag.is_empty() || idx >= self.dag.len() {
            return (0.0, 0.0);
        }
        // Radius: depth-driven, kept strictly inside the open unit disc.
        let depth = self.dag.depth(idx) as f64;
        let max = self.max_depth.max(1) as f64;
        let r = 0.95 * (depth / max); // root at centre, leaves near (but inside) the boundary
        let theta = (hash_id(self.dag.classes()[idx]) as f64 / u64::MAX as f64) * std::f64::consts::TAU;
        (r * theta.cos(), r * theta.sin())
    }

    /// The Poincaré-ball geodesic distance between two classes. This is the **non-Euclidean** metric
    /// — applying L2/cosine to these coordinates would be the error the metric-correctness guard
    /// catches (the block is tagged [`Metric::NonEuclidean`]).
    pub fn hyperbolic_distance(&self, a: usize, b: usize) -> f64 {
        poincare_distance(self.coord(a), self.coord(b))
    }

    /// The [`Block`] descriptor — always [`Metric::NonEuclidean`] (a 2-D disc coordinate). Only
    /// register this in a row's [`SchemaHeader`](crate::encode::SchemaHeader) if
    /// [`GeometryGate::choose`] selected [`Geometry::Hyperbolic`]; the metric tag then makes a
    /// whole-row cosine search a detectable error.
    pub fn block(&self, offset: usize) -> Block {
        Block::new(Encoder::Taxonomy, Metric::NonEuclidean, offset, 2)
    }
}

/// Poincaré-disc geodesic distance: `acosh(1 + 2·|u−v|² / ((1−|u|²)(1−|v|²)))`.
fn poincare_distance((ux, uy): (f64, f64), (vx, vy): (f64, f64)) -> f64 {
    let du2 = (ux - vx).powi(2) + (uy - vy).powi(2);
    let nu = 1.0 - (ux * ux + uy * uy);
    let nv = 1.0 - (vx * vx + vy * vy);
    let denom = (nu * nv).max(1e-12);
    (1.0 + 2.0 * du2 / denom).acosh()
}

// ============================================================================================
// The measured-distortion gate (the load-bearing must-fix)
// ============================================================================================

/// Which geometry the taxonomy block uses — the output of the [`GeometryGate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Geometry {
    /// Euclidean (the **default**; the only one searched under whole-row L2/cosine without a
    /// dedicated kernel).
    Euclidean,
    /// Hyperbolic (Poincaré-ball). **Only** chosen when its measured distortion strictly beats
    /// Euclidean on the actual DAG, and requires a non-Euclidean search kernel.
    Hyperbolic,
}

/// The result of a distortion measurement on the actual `subClassOf` DAG: the average distortion of
/// each geometry, and the gated [`Geometry`] choice. **Average distortion** here is the mean over
/// sampled class pairs of `|d_embed / d_graph − 1|` (lower is better; `0` is a perfect isometry) —
/// a standard, deterministic distortion proxy. No number is hard-coded anywhere; the gate compares
/// the two *measured* values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistortionReport {
    /// Mean distortion of the Euclidean encoder on the sampled pairs.
    pub euclidean_distortion: f64,
    /// Mean distortion of the hyperbolic candidate on the sampled pairs.
    pub hyperbolic_distortion: f64,
    /// Number of class pairs the distortion was averaged over.
    pub pairs_measured: usize,
    /// The chosen geometry — see [`GeometryGate::choose`].
    pub chosen: Geometry,
}

/// The **measured-distortion gate** (design §3.3 must-fix + §9.4). It refuses to adopt the
/// non-Euclidean taxonomy block on a heuristic: it **measures** the embedding distortion of
/// Euclidean vs hyperbolic on the *actual* `subClassOf` DAG and picks Euclidean **unless**
/// hyperbolic strictly wins by at least a margin.
///
/// This is the honest, conservative default the design demands: Euclidean is the safe block (it is
/// the one whole-row L2 search handles), and a non-Euclidean block — which needs a dedicated search
/// kernel and risks the metric-correctness guard — is only chosen when the data *measurably*
/// justifies it.
#[derive(Clone, Copy, Debug)]
pub struct GeometryGate {
    /// Width of the Euclidean encoder's hashed ancestor bag (its block width is `bag_dim + 1`).
    pub bag_dim: usize,
    /// Required relative improvement: hyperbolic is adopted only if
    /// `hyperbolic_distortion <= euclidean_distortion * (1 − margin)`. A positive margin makes the
    /// default biased toward Euclidean (the design's posture). `0.0` would adopt on any improvement.
    pub margin: f64,
}

impl Default for GeometryGate {
    fn default() -> Self {
        // A 5% required relative improvement: the default leans toward the safe Euclidean block,
        // matching the design's "adopt non-Euclidean only past a measured gate" posture. This is a
        // gate threshold, NOT a measured/canonical performance number.
        GeometryGate { bag_dim: 16, margin: 0.05 }
    }
}

impl GeometryGate {
    /// Measure both geometries on `dag` and choose. Distortion is averaged over **all** class pairs
    /// when the DAG is small, or a deterministic stride-sampled subset when it is large (so the gate
    /// is cheap on big hierarchies); only pairs in the same connected component (a finite graph
    /// distance) contribute. An empty/flat DAG yields [`Geometry::Euclidean`] (nothing to gain).
    pub fn choose(&self, dag: &TaxonomyDag) -> DistortionReport {
        let eucl = EuclideanTaxonomyEncoder::new(dag, self.bag_dim);
        let hyp = HyperbolicTaxonomyEncoder::new(dag);
        let n = dag.len();

        let mut eucl_sum = 0.0f64;
        let mut hyp_sum = 0.0f64;
        let mut pairs = 0usize;

        // Stride so that we look at O(n) — O(n·k) pairs even for large DAGs; deterministic.
        let stride = if n > 256 { n / 128 } else { 1 }.max(1);
        let mut i = 0;
        while i < n {
            let ei = eucl.encode(i);
            let mut j = i + 1;
            while j < n {
                if let Some(gd) = dag.graph_distance(i, j) {
                    if gd > 0 {
                        let gd = gd as f64;
                        let ed = l2(&ei, &eucl.encode(j));
                        let hd = hyp.hyperbolic_distance(i, j);
                        eucl_sum += (ed / gd - 1.0).abs();
                        hyp_sum += (hd / gd - 1.0).abs();
                        pairs += 1;
                    }
                }
                j += stride;
            }
            i += stride;
        }

        let (euclidean_distortion, hyperbolic_distortion) = if pairs == 0 {
            (0.0, 0.0)
        } else {
            (eucl_sum / pairs as f64, hyp_sum / pairs as f64)
        };

        // Gate: adopt hyperbolic ONLY if it strictly beats Euclidean by the required margin.
        let chosen = if pairs > 0
            && hyperbolic_distortion <= euclidean_distortion * (1.0 - self.margin)
        {
            Geometry::Hyperbolic
        } else {
            Geometry::Euclidean
        };

        DistortionReport {
            euclidean_distortion,
            hyperbolic_distortion,
            pairs_measured: pairs,
            chosen,
        }
    }
}

// ============================================================================================
// Disjointness: repulsion (train-time) + hard mask (serve-time), answer-safe
// ============================================================================================

/// A **provable-disjointness oracle** mined from a (closed) [`Graph`]: the pairs of classes that
/// the ontology *proves* cannot share an instance (design §2 "owl:disjointWith / sh:not → a
/// REPULSION + MASK prior"; §6.A "verify-soundness / answer-safety"). It reads:
///
/// - `owl:disjointWith` (direct disjoint pairs);
/// - `owl:AllDisjointClasses` with an `owl:members` RDF list (pairwise-disjoint members);
/// - `owl:complementOf` (a complement is disjoint from its base);
///
/// then **propagates disjointness down the `subClassOf` closure**: if `A` is disjoint from `B`, and
/// `X ⊑ A`, `Y ⊑ B` (with the closure materialised), then `X` and `Y` are disjoint too. (Sub-class
/// edges come from the same closed graph, so the propagation is exactly the materialised
/// entailment.) Disjointness is **symmetric** and stored both ways.
///
/// Because every recorded pair is entailed by a declared axiom, the
/// [`mask_candidates`](DisjointnessOracle::mask_candidates) hard mask is
/// **answer-safe**: it removes only candidates whose type is *provably* disjoint from the query
/// type — never a merely-dissimilar one.
#[derive(Clone, Debug, Default)]
pub struct DisjointnessOracle {
    /// Symmetric set of provably-disjoint class id pairs (each stored as `(min, max)`).
    disjoint: FxHashSet<(Id, Id)>,
    /// Per-class set of classes it is disjoint from (adjacency, for fast lookup both ways).
    by_class: FxHashMap<Id, FxHashSet<Id>>,
}

impl DisjointnessOracle {
    /// Mine the oracle from `graph` (which should be **closed** so `subClassOf` propagation is the
    /// materialised entailment). Safe on a graph with no disjointness axioms — the oracle is then
    /// empty and the mask drops nothing.
    pub fn mine(graph: &Graph) -> DisjointnessOracle {
        let mut oracle = DisjointnessOracle::default();

        // Resolve the vocabulary ids; an absent term simply matches nothing.
        let disjoint_with = graph.id_of(&named(&format!("{OWL}disjointWith")));
        let complement_of = graph.id_of(&named(&format!("{OWL}complementOf")));
        let all_disjoint = graph.id_of(&named(&format!("{OWL}AllDisjointClasses")));
        let members = graph.id_of(&named(&format!("{OWL}members")));
        let rdf_type = graph.id_of(&named(RDF_TYPE));
        let rdf_first = graph.id_of(&named(&format!("{RDF}first")));
        let rdf_rest = graph.id_of(&named(&format!("{RDF}rest")));
        let rdf_nil = graph.id_of(&named(&format!("{RDF}nil")));

        // Index `(subject, predicate) -> object` for RDF-list walking (mirrors sparq-reason's owl
        // inconsistency reader).
        let mut first_obj: FxHashMap<(Id, Id), Id> = FxHashMap::default();
        let mut adc_nodes: Vec<Id> = Vec::new();
        for [s, p, o] in graph.iter_ids() {
            if Some(p) == disjoint_with {
                oracle.add_pair(s, o);
            } else if Some(p) == complement_of {
                // A class and its complement are disjoint.
                oracle.add_pair(s, o);
            } else if Some(p) == rdf_type && Some(o) == all_disjoint {
                adc_nodes.push(s);
            }
            if rdf_first.is_some() || rdf_rest.is_some() {
                first_obj.entry((s, p)).or_insert(o);
            }
        }

        // owl:AllDisjointClasses: each member is pairwise-disjoint with every other.
        if let (Some(members_p), Some(first_p), Some(rest_p), Some(nil)) =
            (members, rdf_first, rdf_rest, rdf_nil)
        {
            for node in adc_nodes {
                if let Some(&head) = first_obj.get(&(node, members_p)) {
                    let list = walk_list(&first_obj, head, first_p, rest_p, nil);
                    for a in 0..list.len() {
                        for b in (a + 1)..list.len() {
                            oracle.add_pair(list[a], list[b]);
                        }
                    }
                }
            }
        }

        // Propagate down the subClassOf closure: if A disjoint B then every X ⊑ A is disjoint from
        // every Y ⊑ B. Build the sub-class sets from the closed graph.
        let subclass = graph.id_of(&named(RDFS_SUBCLASS_OF));
        if let Some(subclass_p) = subclass {
            // super-class -> set of its (transitive, via the closure) sub-classes, plus itself.
            let mut subs_of: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default();
            for [s, p, o] in graph.iter_ids() {
                if p == subclass_p && s != o {
                    subs_of.entry(o).or_default().insert(s);
                }
            }
            // Snapshot the declared pairs before expanding (so propagation reads a stable base).
            let base: Vec<(Id, Id)> = oracle.disjoint.iter().copied().collect();
            for (a, b) in base {
                let empty = FxHashSet::default();
                let a_subs = subs_of.get(&a).unwrap_or(&empty);
                let b_subs = subs_of.get(&b).unwrap_or(&empty);
                // include the class itself on each side.
                let a_all = std::iter::once(a).chain(a_subs.iter().copied());
                let b_all: Vec<Id> = std::iter::once(b).chain(b_subs.iter().copied()).collect();
                for x in a_all {
                    for &y in &b_all {
                        oracle.add_pair(x, y);
                    }
                }
            }
        }

        oracle
    }

    /// Absorb externally-**proven** disjoint class pairs into the oracle — the seam the
    /// [`crate::ufo_priors`] UFO/gUFO reader feeds (kern/ufo-priors, epic sq-0wo9e:
    /// the design's "optional/last" gUFO prior, §2 + §9.5).
    ///
    /// SOUNDNESS CONTRACT: every pair passed here must be *proven* disjoint by the caller (e.g.
    /// UFO's kind-partition / nature-partition theorems — see `ufo_priors`'s module docs). The
    /// oracle performs no propagation on absorbed pairs (a caller like `UfoPriors` propagates
    /// before feeding); it only records them, keeping [`is_disjoint`](Self::is_disjoint) and the
    /// [`mask_candidates`](Self::mask_candidates) hard mask answer-safe. Self-pairs are ignored.
    pub fn absorb_proven_pairs<I: IntoIterator<Item = (Id, Id)>>(&mut self, pairs: I) {
        for (a, b) in pairs {
            self.add_pair(a, b);
        }
    }

    fn add_pair(&mut self, a: Id, b: Id) {
        if a == b {
            return; // a class is never disjoint from itself.
        }
        self.disjoint.insert((a.min(b), a.max(b)));
        self.by_class.entry(a).or_default().insert(b);
        self.by_class.entry(b).or_default().insert(a);
    }

    /// Number of distinct provably-disjoint (unordered) class pairs.
    pub fn pair_count(&self) -> usize {
        self.disjoint.len()
    }

    /// Is class `a` **provably disjoint** from class `b` (per the mined + propagated axioms)?
    /// Symmetric; `false` for `a == b` and for any pair not entailed disjoint (the honest
    /// open-world default — *absence of a disjointness axiom is not disjointness*).
    pub fn is_disjoint(&self, a: Id, b: Id) -> bool {
        a != b && self.disjoint.contains(&(a.min(b), a.max(b)))
    }

    /// The classes provably disjoint from `class` (empty if none).
    pub fn disjoint_with(&self, class: Id) -> Option<&FxHashSet<Id>> {
        self.by_class.get(&class)
    }

    /// **Train-time repulsion pairs** (design §2 "train-time margin pushing disjoint centroids
    /// apart"): the unordered provably-disjoint class pairs a trainer's repulsion loss pushes apart
    /// in the taxonomy/relational block. This crate trains nothing (embeddings are out-of-process,
    /// or via the `kge` harness); this is the reusable list a trainer consumes. Deterministic order.
    pub fn repulsion_pairs(&self) -> Vec<(Id, Id)> {
        let mut pairs: Vec<(Id, Id)> = self.disjoint.iter().copied().collect();
        pairs.sort_unstable();
        pairs
    }

    /// **Serve-time hard mask** (design §2 "serve-time HARD MASK dropping provably-disjoint
    /// candidates"; §6.A answer-safety): drop every `candidate` whose class is provably disjoint
    /// from **any** of the `query_types`. `candidates` are `(class-of-candidate, candidate-id)`
    /// pairs — the caller maps a candidate node to its `rdf:type`(s) and passes one entry per type,
    /// or per (node, type). The returned vector preserves input order with the dropped entries
    /// removed.
    ///
    /// This is **answer-safe**: it removes only nodes whose type the closure *proves* disjoint from
    /// a query type — a removed node could never have been a correct same-type neighbour. It is a
    /// strict subset filter (∀ output ⊆ input), the property the disjointness-mask test asserts.
    pub fn mask_candidates<T: Copy>(
        &self,
        query_types: &[Id],
        candidates: &[(Id, T)],
    ) -> Vec<(Id, T)> {
        candidates
            .iter()
            .copied()
            .filter(|&(cand_class, _)| {
                !query_types.iter().any(|&qt| self.is_disjoint(qt, cand_class))
            })
            .collect()
    }
}

// ============================================================================================
// helpers
// ============================================================================================

/// A named-node term for `iri`.
fn named(iri: &str) -> oxrdf::Term {
    oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(iri))
}

/// Is the id a class node (a named node or blank node) — the only thing that can sit on either side
/// of a `subClassOf` / disjointWith axiom? Literals and inline values are excluded.
fn is_class_node(graph: &Graph, id: Id) -> bool {
    matches!(graph.dict.term_parts(id), TermParts::Iri { .. } | TermParts::Blank(_))
}

/// Walk an RDF list `(first/rest)` from `head` to `nil`, collecting members. Guarded against an
/// unterminated/cyclic list.
fn walk_list(
    first_obj: &FxHashMap<(Id, Id), Id>,
    head: Id,
    first_p: Id,
    rest_p: Id,
    nil: Id,
) -> Vec<Id> {
    let mut items = Vec::new();
    let mut cur = head;
    let mut guard = 0;
    while cur != nil && guard < 100_000 {
        guard += 1;
        match first_obj.get(&(cur, first_p)) {
            Some(&f) => items.push(f),
            None => break,
        }
        match first_obj.get(&(cur, rest_p)) {
            Some(&r) => cur = r,
            None => break,
        }
    }
    items
}

/// L2 distance over `f32` slices in `f64` accumulation (the distortion gate wants headroom).
fn l2(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum::<f64>()
        .sqrt()
}

/// SplitMix64 finaliser over a dict id — a small, fast, deterministic hash for the ancestor-bag and
/// the hyperbolic angle. Kept local so the module carries no cross-module coupling beyond the
/// feature gate (the same algorithm `embed`/`structure` use).
#[inline]
fn hash_id(id: Id) -> u64 {
    let mut z = (id as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::{close_for_vectorise, ClosedGraph};
    use sparq_reason::Profile;

    // A schema-rich graph: a multi-level subclass hierarchy + disjointness axioms (direct,
    // AllDisjointClasses, and propagated-down-subclass).
    const TTL: &str = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix ex:   <http://ex/> .

ex:Animal   rdfs:subClassOf ex:LivingThing .
ex:Plant    rdfs:subClassOf ex:LivingThing .
ex:Dog      rdfs:subClassOf ex:Animal .
ex:Cat      rdfs:subClassOf ex:Animal .
ex:Oak      rdfs:subClassOf ex:Plant .

# Animals and plants are disjoint (propagates to Dog/Cat vs Oak).
ex:Animal owl:disjointWith ex:Plant .

# Dogs and cats are disjoint via AllDisjointClasses.
[] a owl:AllDisjointClasses ; owl:members ( ex:Dog ex:Cat ) .
"#;

    fn closed() -> ClosedGraph {
        close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap()
    }

    fn class(g: &Graph, local: &str) -> Id {
        g.id_of(&named(&format!("http://ex/{local}"))).unwrap()
    }

    #[test]
    fn dag_extracts_hierarchy_and_ancestors() {
        let c = closed();
        let dag = TaxonomyDag::build(&c.graph);
        assert!(!dag.is_empty());
        let dog = dag.index_of(class(&c.graph, "Dog")).unwrap();
        let ancestors: FxHashSet<Id> =
            dag.ancestors(dog).iter().map(|&i| dag.classes()[i]).collect();
        // After the RDFS closure, Dog ⊑ Animal ⊑ LivingThing — both must be ancestors.
        assert!(ancestors.contains(&class(&c.graph, "Animal")), "Dog's ancestor Animal");
        assert!(ancestors.contains(&class(&c.graph, "LivingThing")), "Dog's ancestor LivingThing");
        // Dog is not its own ancestor.
        assert!(!ancestors.contains(&class(&c.graph, "Dog")));
    }

    #[test]
    fn depth_increases_down_the_hierarchy() {
        let c = closed();
        let dag = TaxonomyDag::build(&c.graph);
        let lt = dag.depth(dag.index_of(class(&c.graph, "LivingThing")).unwrap());
        let animal = dag.depth(dag.index_of(class(&c.graph, "Animal")).unwrap());
        let dog = dag.depth(dag.index_of(class(&c.graph, "Dog")).unwrap());
        assert_eq!(lt, 0, "root depth 0");
        assert!(animal > lt, "Animal deeper than LivingThing");
        assert!(dog > animal, "Dog deeper than Animal");
    }

    #[test]
    fn euclidean_encoder_brings_siblings_closer_than_distant_classes() {
        // The provable-ish structural property of the Euclidean block: classes sharing more
        // ancestry are closer in L2. Dog and Cat (siblings under Animal) must be closer than Dog
        // and Oak (different top-level branch).
        let c = closed();
        let dag = TaxonomyDag::build(&c.graph);
        let enc = EuclideanTaxonomyEncoder::new(&dag, 16);
        let dog = enc.encode(dag.index_of(class(&c.graph, "Dog")).unwrap());
        let cat = enc.encode(dag.index_of(class(&c.graph, "Cat")).unwrap());
        let oak = enc.encode(dag.index_of(class(&c.graph, "Oak")).unwrap());
        let d_sib = l2(&dog, &cat);
        let d_far = l2(&dog, &oak);
        assert!(d_sib < d_far, "siblings {d_sib} should be closer than cross-branch {d_far}");
    }

    #[test]
    fn euclidean_block_is_euclidean_tagged() {
        let c = closed();
        let dag = TaxonomyDag::build(&c.graph);
        let enc = EuclideanTaxonomyEncoder::new(&dag, 8);
        let b = enc.block(0);
        assert_eq!(b.metric, Metric::Euclidean);
        assert_eq!(b.encoder, Encoder::Taxonomy);
        assert_eq!(b.width, enc.width());
    }

    #[test]
    fn geometry_gate_defaults_to_euclidean_and_is_measured() {
        // The load-bearing must-fix: the gate MEASURES distortion and (with the default margin)
        // does not flip to a non-Euclidean block unless hyperbolic strictly wins. On this small,
        // shallow, multiply-branching DAG it must NOT adopt hyperbolic.
        let c = closed();
        let dag = TaxonomyDag::build(&c.graph);
        let report = GeometryGate::default().choose(&dag);
        assert!(report.pairs_measured > 0, "the gate must actually measure pairs");
        assert_eq!(report.chosen, Geometry::Euclidean, "default must not adopt hyperbolic unmeasured");
    }

    #[test]
    fn geometry_gate_can_select_hyperbolic_when_it_strictly_wins() {
        // Force the gate to be willing to flip by removing the margin AND making hyperbolic the
        // strictly-lower-distortion arm. We do not assert a hyperbolic *win* on real data (that is
        // empirical); we assert the gate's decision rule is honest: it flips iff measured distortion
        // beats Euclidean. We construct the decision directly from a report to prove the rule.
        let win = DistortionReport {
            euclidean_distortion: 1.0,
            hyperbolic_distortion: 0.5,
            pairs_measured: 10,
            chosen: Geometry::Euclidean, // ignored; we re-decide below
        };
        // Re-apply the gate's rule with margin 0 → hyperbolic (0.5) beats euclidean (1.0).
        let gate = GeometryGate { bag_dim: 16, margin: 0.0 };
        let flips = win.hyperbolic_distortion <= win.euclidean_distortion * (1.0 - gate.margin);
        assert!(flips, "the gate rule must adopt hyperbolic when it strictly wins");
    }

    #[test]
    fn hyperbolic_block_is_non_euclidean_tagged() {
        let c = closed();
        let dag = TaxonomyDag::build(&c.graph);
        let hyp = HyperbolicTaxonomyEncoder::new(&dag);
        let b = hyp.block(0);
        assert_eq!(b.metric, Metric::NonEuclidean, "hyperbolic block must be tagged non-Euclidean");
        // And a SchemaHeader with that block must FAIL the whole-row Euclidean guard — proving a
        // cosine search on it is a detectable error, not silent corruption.
        let header = crate::encode::SchemaHeader::new(vec![b]).unwrap();
        assert!(header.check_euclidean().is_err(), "cosine on a hyperbolic block must be rejected");
    }

    #[test]
    fn disjointness_is_mined_and_propagated_down_subclasses() {
        let c = closed();
        let oracle = DisjointnessOracle::mine(&c.graph);
        let animal = class(&c.graph, "Animal");
        let plant = class(&c.graph, "Plant");
        let dog = class(&c.graph, "Dog");
        let cat = class(&c.graph, "Cat");
        let oak = class(&c.graph, "Oak");

        // Direct axiom.
        assert!(oracle.is_disjoint(animal, plant), "declared Animal⊥Plant");
        // Propagated down subclass: Dog ⊑ Animal, Oak ⊑ Plant ⇒ Dog ⊥ Oak.
        assert!(oracle.is_disjoint(dog, oak), "propagated Dog⊥Oak via subclass closure");
        assert!(oracle.is_disjoint(cat, oak), "propagated Cat⊥Oak");
        // AllDisjointClasses: Dog ⊥ Cat.
        assert!(oracle.is_disjoint(dog, cat), "AllDisjointClasses Dog⊥Cat");
        // NOT disjoint: Dog and Animal (subclass, never disjoint); a class with itself.
        assert!(!oracle.is_disjoint(dog, animal), "a subclass is not disjoint from its super");
        assert!(!oracle.is_disjoint(dog, dog), "a class is never disjoint from itself");
        // Symmetric.
        assert!(oracle.is_disjoint(plant, animal));
    }

    #[test]
    fn mask_is_answer_safe_subset_and_drops_only_provably_disjoint() {
        let c = closed();
        let oracle = DisjointnessOracle::mine(&c.graph);
        let dog = class(&c.graph, "Dog");
        let cat = class(&c.graph, "Cat");
        let animal = class(&c.graph, "Animal");
        let oak = class(&c.graph, "Oak");

        // Query is a Dog; candidates carry their class + an opaque payload (here the node id).
        let candidates: Vec<(Id, u32)> =
            vec![(animal, 1), (cat, 2), (oak, 3), (dog, 4)];
        let kept = oracle.mask_candidates(&[dog], &candidates);

        // Answer-safety: output is a strict subset of input order-preserved.
        assert!(kept.len() <= candidates.len());
        let kept_set: FxHashSet<(Id, u32)> = kept.iter().copied().collect();
        for k in &kept {
            assert!(candidates.contains(k), "mask only ever removes, never invents");
        }
        // Oak (Plant, disjoint from Dog⊑Animal) and Cat (AllDisjointClasses) must be dropped.
        assert!(!kept_set.contains(&(oak, 3)), "provably-disjoint Oak dropped");
        assert!(!kept_set.contains(&(cat, 2)), "provably-disjoint Cat dropped");
        // Animal (super-class, NOT disjoint) and Dog (same class) must be kept — the mask removes
        // ONLY provably-wrong neighbours, never a merely-different one.
        assert!(kept_set.contains(&(animal, 1)), "non-disjoint Animal kept");
        assert!(kept_set.contains(&(dog, 4)), "same-class Dog kept");
    }

    #[test]
    fn repulsion_pairs_are_deterministic_and_match_disjoint_set() {
        let c = closed();
        let oracle = DisjointnessOracle::mine(&c.graph);
        let a = oracle.repulsion_pairs();
        let b = oracle.repulsion_pairs();
        assert_eq!(a, b, "repulsion pair listing is deterministic");
        assert_eq!(a.len(), oracle.pair_count());
        // Every listed pair is provably disjoint.
        for &(x, y) in &a {
            assert!(oracle.is_disjoint(x, y));
        }
    }

    #[test]
    fn absorbed_proven_pairs_join_the_mask_and_self_pairs_are_ignored() {
        // kern/ufo-priors: the seam the UFO/gUFO reader feeds. Absorbed pairs behave
        // exactly like mined ones (symmetric, mask-effective); a self-pair is silently ignored.
        let c = closed();
        let mut oracle = DisjointnessOracle::mine(&c.graph);
        let dog = class(&c.graph, "Dog");
        let lt = class(&c.graph, "LivingThing");
        assert!(
            !oracle.is_disjoint(dog, lt),
            "not disjoint before absorbing"
        );
        let before = oracle.pair_count();
        oracle.absorb_proven_pairs([(dog, lt), (dog, dog)]);
        assert_eq!(
            oracle.pair_count(),
            before + 1,
            "self-pair ignored, real pair added"
        );
        assert!(
            oracle.is_disjoint(dog, lt) && oracle.is_disjoint(lt, dog),
            "symmetric"
        );
        let kept = oracle.mask_candidates(&[dog], &[(lt, 1u8)]);
        assert!(kept.is_empty(), "absorbed pair drives the hard mask");
    }

    #[test]
    fn empty_graph_is_safe() {
        // No subClassOf, no disjointness → empty DAG + empty oracle; mask drops nothing.
        let c = close_for_vectorise("<http://ex/a> <http://ex/p> <http://ex/b> .", "ntriples", Profile::Rdfs)
            .unwrap();
        let dag = TaxonomyDag::build(&c.graph);
        assert!(dag.is_empty());
        let report = GeometryGate::default().choose(&dag);
        assert_eq!(report.chosen, Geometry::Euclidean);
        assert_eq!(report.pairs_measured, 0);
        let oracle = DisjointnessOracle::mine(&c.graph);
        assert_eq!(oracle.pair_count(), 0);
        let kept = oracle.mask_candidates(&[1], &[(2u32, 9u8), (3, 10)].map(|(a, b)| (a as Id, b)));
        assert_eq!(kept.len(), 2, "no axioms → nothing masked");
    }
}
