//! Thin in-tree shallow-KGE **trainer** (DistMult) — the embedding producer the
//! structure-aware-vectorisation eval harness measures.
//!
//! [OPUS-4.8] sq-0wo9e.8 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §P6 eval-harness + sq-0wo9e.8 trainer). This module is **opt-in** (the `kge` cargo feature,
//! off by default, which implies `structure`) and changes **nothing** in the default
//! `sparq-vectors` build or the core engine.
//!
//! # What this is — and what it is NOT
//!
//! `sparq-vectors` is a vector-**search** + **import** surface; embeddings are normally produced
//! **out-of-process** ([`crate::embed`]). This module is the deferred remainder noted by P0
//! ([`crate::structure`]): a **minimal, CPU-only, deterministically-seeded** trainer that
//! *consumes* the P0 [`crate::structure::NegativeSampler`] (type-constrained
//! negatives) over the P0 closure ([`materialise_closure`](crate::structure::materialise_closure))
//! and emits per-entity + per-relation embeddings. Its only purpose is to **produce embeddings to
//! measure** — it is research-grade, not a production training subsystem (no GPU, no minibatch
//! parallelism, no learning-rate schedule beyond a fixed step, no early stopping). It is **not** a
//! claim that sparq ships SOTA KGE.
//!
//! # Two scoring models: DistMult and ComplEx (the asymmetry axis)
//!
//! DistMult (Yang et al. 2015, *Embedding Entities and Relations for Learning and Inference in
//! Knowledge Bases*) scores a triple `(h, r, t)` as the trilinear product
//! `score = Σ_i e_h[i] · w_r[i] · e_t[i]` — one entity matrix and one (diagonal) relation vector,
//! the fewest parameters of any standard bilinear KGE. **It is symmetric in `h`/`t`** (its score is
//! identical for `(h, r, t)` and `(t, r, h)`), so on a relation whose true edges are **directional**
//! it is structurally near-random *by construction* — it cannot place the head above the tail.
//!
//! ComplEx (Trouillon et al. 2016, *Complex Embeddings for Simple Link Prediction*) is the minimal
//! asymmetric extension: embeddings are complex, and the score is the real part of the Hermitian
//! trilinear product `Re(⟨e_h, w_r, conj(e_t)⟩)`. Splitting each complex vector into a real and an
//! imaginary half (`e = e_re + i·e_im`), the score is
//! `Σ_i [ h_re·r_re·t_re + h_re·r_im·t_im + h_im·r_re·t_im − h_im·r_im·t_re ]`. The final term
//! flips sign when `h` and `t` are swapped, so **ComplEx breaks the DistMult symmetry** and can
//! model directional relations. Picking the model is the [`ModelKind`] axis on [`TrainConfig`].
//!
//! **Why this matters for the ablation (adversarial-review finding):** both synthetic eval slices
//! are ~100 % directional (no `(a,r,b)` ever has a reverse `(b,r,a)`), so a DistMult ablation runs
//! entirely in a near-random regime where the inter-cell deltas are noise. The ablation **must** be
//! re-run with [`ModelKind::ComplEx`] before any prior is adopted; the example runs both models.
//!
//! # Training objective
//!
//! Sigmoid + binary-cross-entropy over each positive triple and its sampled negatives
//! (the standard "1-vs-N" / negative-sampling NLL, e.g. Trouillon et al. 2016): for a positive we
//! push `σ(score) → 1`, for each negative `σ(score) → 0`. Gradients are the closed-form DistMult
//! partials; we step with plain SGD plus a small L2 penalty (weight decay) to keep embeddings from
//! diverging. All randomness (init + negative draws) is a deterministic SplitMix64 stream seeded
//! from [`TrainConfig::seed`], so a fixed config reproduces bit-identical embeddings.
//!
//! # Does it actually learn?
//!
//! [`TrainReport`] records per-epoch mean loss; [`train`] asserts nothing itself, but the crate
//! tests (`tests/kge.rs`) check the load-bearing invariants: **loss decreases** over epochs and the
//! embeddings are **non-degenerate** (not all-equal / not collapsed to a point). The eval harness
//! ([`crate::eval`]) then measures filtered link-prediction quality.

use crate::structure::{
    is_embeddable, Corrupt, NegativeSampler, SamplingMode, TermScope, TypeConstraints,
};
use rustc_hash::FxHashMap;
use sparq_core::dict::{Id, TermParts};
use sparq_core::Graph;

/// SplitMix64 step — the same deterministic PRNG the rest of the crate uses (kept local so
/// `train` carries no cross-module coupling beyond the feature gate).
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniform `f32` in `(-1, 1)` from the PRNG stream (for embedding init).
#[inline]
fn next_unit(state: &mut u64) -> f32 {
    // 24 random bits → [0, 1), then map to (-1, 1).
    let bits = (splitmix64(state) >> 40) as u32; // top 24 bits
    let u = (bits as f32) / ((1u32 << 24) as f32);
    2.0 * u - 1.0
}

/// Numerically-stable logistic sigmoid.
#[inline]
fn sigmoid(x: f32) -> f32 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

/// ComplEx score `Re(⟨e_h, w_r, conj(e_t)⟩)` for three `dim`-float rows interpreted as complex
/// vectors (first `dim/2` real, last `dim/2` imaginary). Expanding the Hermitian trilinear product
/// over the `half = dim/2` complex components:
/// `Σ_i [ h_re·r_re·t_re + h_re·r_im·t_im + h_im·r_re·t_im − h_im·r_im·t_re ]`. The last term flips
/// sign under an `h ↔ t` swap, which is exactly what makes ComplEx **asymmetric** (DistMult is the
/// real-only case `r_im = h_im = t_im = 0`, which collapses to the symmetric `Σ h·r·t`).
#[inline]
fn score_complex(h: &[f32], r: &[f32], t: &[f32], dim: usize) -> f32 {
    let half = dim / 2;
    let mut s = 0.0f32;
    for i in 0..half {
        let (h_re, h_im) = (h[i], h[half + i]);
        let (r_re, r_im) = (r[i], r[half + i]);
        let (t_re, t_im) = (t[i], t[half + i]);
        s += h_re * r_re * t_re + h_re * r_im * t_im + h_im * r_re * t_im - h_im * r_im * t_re;
    }
    s
}

/// Which bilinear KGE scoring function the trainer uses — the **asymmetry axis**.
///
/// [`ModelKind::DistMult`] is symmetric in head/tail (cannot model directional relations);
/// [`ModelKind::ComplEx`] is the minimal asymmetric extension (complex embeddings, Hermitian
/// product). On directional data the symmetric model is structurally near-random, so the ablation
/// must be re-run with `ComplEx` before any prior is adopted — see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ModelKind {
    /// Yang et al. 2015 trilinear product `Σ e_h·w_r·e_t` — **symmetric** in `h`/`t`.
    DistMult,
    /// Trouillon et al. 2016 Hermitian product `Re(⟨e_h, w_r, conj(e_t)⟩)` — **asymmetric**. The
    /// `dim` floats are interpreted as `dim/2` complex components (real half then imaginary half);
    /// `dim` must be even (the trainer rounds an odd `dim` down to the nearest even value).
    #[default]
    ComplEx,
}

/// Hyper-parameters for the [`train`] loop. All fields are explicit (no hidden defaults) so a run
/// is fully reproducible from its config; [`TrainConfig::small`] gives a sane, work-box-sized
/// preset for tests and the indicative ablation.
#[derive(Clone, Copy, Debug)]
pub struct TrainConfig {
    /// The scoring model — the asymmetry axis ([`ModelKind`]). Defaults to the asymmetric
    /// [`ModelKind::ComplEx`] in [`TrainConfig::small`] (the directional-slice-safe choice).
    pub model: ModelKind,
    /// Embedding dimensionality (entity vectors and the diagonal relation vector share it). For
    /// [`ModelKind::ComplEx`] it is the number of `f32`s, i.e. `dim/2` complex components, and must
    /// be even (an odd value is rounded down).
    pub dim: usize,
    /// Number of full passes over the positive triples.
    pub epochs: usize,
    /// SGD step size.
    pub lr: f32,
    /// L2 weight-decay coefficient applied to the parameters touched each step (keeps embeddings
    /// bounded; the DistMult objective is otherwise unbounded below for collinear vectors).
    pub l2: f32,
    /// Negatives sampled per positive, split evenly between head- and tail-corruption.
    pub negatives_per_positive: usize,
    /// The P0 ablation switch: [`SamplingMode::TypeConstrained`] (type-valid negatives) vs
    /// [`SamplingMode::Unconstrained`] (uniform-random negatives).
    pub sampling: SamplingMode,
    /// [OPUS-4.8] sq-2489d.4 — the Phase-4 PROVENANCE-WEIGHTING ablation switch
    /// ([`WeightMode`](crate::provenance::WeightMode)). Under
    /// [`WeightMode::Provenance`](crate::provenance::WeightMode::Provenance) each positive's SGD step
    /// is scaled by the design-§USE-1 provenance weight `w(t)` (the CKRL confidence-weighted-loss
    /// move — a high-assurance fact contributes a full-strength gradient, a low-assurance/low-source
    /// fact a down-weighted one); under
    /// [`WeightMode::Uniform`](crate::provenance::WeightMode::Uniform) every weight is `1.0` and the
    /// loop is byte-identical to the unweighted trainer (the ablation-off baseline).
    pub weight_mode: crate::provenance::WeightMode,
    /// The RDF 1.2 **quoted-terms ablation switch** ([`TermScope`]): under the default
    /// [`TermScope::IriBlank`] the trainer is byte-identical to the pre-switch trainer (quoted
    /// triple terms are invisible — the historical behaviour); under [`TermScope::Embeddable`]
    /// triples with a quoted-term endpoint (`rdf:reifies` edges) become positives, quoted terms
    /// get entity rows, and negative corruption is sort-preserving (see
    /// [`NegativeSampler::sample`]). Every preset defaults this OFF.
    pub term_scope: TermScope,
    /// Master seed for init + negative draws + positive shuffle. Fixed ⇒ reproducible.
    pub seed: u64,
}

impl TrainConfig {
    /// A small, work-box-sized preset: the asymmetric [`ModelKind::ComplEx`] model, 32-dim, a
    /// handful of epochs, modest LR. Sized to train a few-thousand-triple graph in seconds on a CPU
    /// — **not** a tuned production configuration. ComplEx is the default because both eval slices
    /// are directional, where the symmetric DistMult is near-random (see the module docs).
    pub fn small(sampling: SamplingMode, seed: u64) -> TrainConfig {
        Self::small_with_model(ModelKind::ComplEx, sampling, seed)
    }

    /// Like [`TrainConfig::small`] but with an explicit [`ModelKind`] — used by the ablation to run
    /// the **same** config under both the symmetric (DistMult) and asymmetric (ComplEx) model so
    /// the asymmetry's effect is itself measurable.
    pub fn small_with_model(model: ModelKind, sampling: SamplingMode, seed: u64) -> TrainConfig {
        TrainConfig {
            model,
            dim: 32,
            epochs: 50,
            lr: 0.1,
            l2: 1e-4,
            negatives_per_positive: 8,
            sampling,
            // Default OFF: an existing caller (and the P0 ablation) keeps the unweighted baseline
            // until it explicitly opts into provenance-weighting via `weight_mode`. [OPUS-4.8]
            weight_mode: crate::provenance::WeightMode::Uniform,
            // Default OFF (`TermScope::IriBlank`): quoted-term visibility is opt-in per ablation
            // arm; every existing baseline stays byte-identical. [FABLE-5]
            term_scope: TermScope::default(),
            seed,
        }
    }
}

/// The trained model: dense parameter matrices plus the id→row maps that translate a graph
/// dictionary id into a parameter row. Scoring ([`TrainedModel::score`]) and the eval harness read
/// only this struct, so a model is self-contained once trained.
///
/// [OPUS-5] `Clone` so an eval harness can derive a *post-processed* model — e.g. the
/// confidence-weighted structural-sketch augmentation `eval::run_pooling_ablation` measures —
/// without retraining, keeping the two ablation arms on genuinely identical parameters.
#[derive(Clone)]
pub struct TrainedModel {
    /// The scoring model — [`ModelKind::DistMult`] (symmetric) or [`ModelKind::ComplEx`]
    /// (asymmetric). [`TrainedModel::score`] dispatches on it.
    pub model: ModelKind,
    /// Embedding dimensionality (number of `f32`s per row). For [`ModelKind::ComplEx`] the first
    /// `dim/2` are the real components and the last `dim/2` the imaginary components.
    pub dim: usize,
    /// `entity_emb[row*dim .. row*dim+dim]` is the embedding of the entity whose row is `row`.
    pub entity_emb: Vec<f32>,
    /// `rel_emb[row*dim .. row*dim+dim]` is the diagonal relation vector of relation row `row`.
    pub rel_emb: Vec<f32>,
    /// Graph entity dict-id → entity row.
    entity_row: FxHashMap<Id, usize>,
    /// Graph relation (predicate) dict-id → relation row.
    rel_row: FxHashMap<Id, usize>,
}

impl TrainedModel {
    /// Number of entity rows.
    pub fn num_entities(&self) -> usize {
        self.entity_row.len()
    }

    /// Number of relation rows.
    pub fn num_relations(&self) -> usize {
        self.rel_row.len()
    }

    /// The parameter row of an entity dict-id, if the model knows it.
    pub fn entity_row(&self, id: Id) -> Option<usize> {
        self.entity_row.get(&id).copied()
    }

    /// The parameter row of a relation dict-id, if the model knows it.
    pub fn rel_row(&self, id: Id) -> Option<usize> {
        self.rel_row.get(&id).copied()
    }

    /// A view of an entity row's embedding slice.
    #[inline]
    pub fn entity_vec(&self, row: usize) -> &[f32] {
        &self.entity_emb[row * self.dim..row * self.dim + self.dim]
    }

    /// A view of a relation row's diagonal vector.
    #[inline]
    pub fn rel_vec(&self, row: usize) -> &[f32] {
        &self.rel_emb[row * self.dim..row * self.dim + self.dim]
    }

    /// The model score for parameter rows `(h_row, r_row, t_row)`. Dispatches on [`TrainedModel::model`]:
    /// DistMult `Σ_i e_h[i]·w_r[i]·e_t[i]` (symmetric) or ComplEx `Re(⟨e_h, w_r, conj(e_t)⟩)`
    /// (asymmetric, computed by the private `score_complex` helper). [OPUS-4.8] sq-0wo9e.8
    #[inline]
    pub fn score_rows(&self, h_row: usize, r_row: usize, t_row: usize) -> f32 {
        let h = self.entity_vec(h_row);
        let r = self.rel_vec(r_row);
        let t = self.entity_vec(t_row);
        match self.model {
            ModelKind::DistMult => {
                let mut s = 0.0f32;
                for i in 0..self.dim {
                    s += h[i] * r[i] * t[i];
                }
                s
            }
            ModelKind::ComplEx => score_complex(h, r, t, self.dim),
        }
    }

    /// DistMult score for a triple of dict-ids, or `None` if any element is unknown to the model
    /// (e.g. an entity that only appeared as a literal object, which has no entity row).
    pub fn score(&self, h: Id, r: Id, t: Id) -> Option<f32> {
        let hr = *self.entity_row.get(&h)?;
        let rr = *self.rel_row.get(&r)?;
        let tr = *self.entity_row.get(&t)?;
        Some(self.score_rows(hr, rr, tr))
    }

    /// The **compositional STATEMENT-level encoding** of parameter rows `(h_row, r_row, t_row)` —
    /// the per-component interaction terms of the model's own scoring product, as a `dim`-float
    /// vector. [SONNET-4.6] sq-1e5kk (follow-up to the node-level quoted-terms visibility arm).
    ///
    /// Construction, per [`ModelKind`]:
    /// - [`ModelKind::DistMult`]: `v[i] = e_h[i]·w_r[i]·e_t[i]` — the trilinear product *before*
    ///   the final sum, so `v.iter().sum::<f32>()` recovers [`TrainedModel::score_rows`] exactly
    ///   (same products, same summation order).
    /// - [`ModelKind::ComplEx`]: the complex Hadamard product `e_h ∘ w_r ∘ conj(e_t)` in the
    ///   crate's real-half-then-imaginary-half layout (`v[i] = Re_i`, `v[dim/2 + i] = Im_i`). The
    ///   real half sums to [`TrainedModel::score_rows`] (the Hermitian score keeps only the real
    ///   part); the imaginary half carries the direction information the scalar score discards.
    ///
    /// The encoding is **deterministic and derived** — no new parameters are trained, so it
    /// inherits the trainer's bit-for-bit reproducibility. It lives in the model's **interaction
    /// space, not entity space**: never compare it against entity rows (or store it beside them in
    /// one index) — its only meaningful comparisons are with other statement encodings from the
    /// same model. **No accuracy claim is made**: whether this representation helps any downstream
    /// task is empirical, and adoption stays measurement-gated like every other axis here.
    pub fn encode_statement_rows(&self, h_row: usize, r_row: usize, t_row: usize) -> Vec<f32> {
        let h = self.entity_vec(h_row);
        let r = self.rel_vec(r_row);
        let t = self.entity_vec(t_row);
        match self.model {
            ModelKind::DistMult => (0..self.dim).map(|i| h[i] * r[i] * t[i]).collect(),
            ModelKind::ComplEx => {
                let half = self.dim / 2;
                let mut v = vec![0.0f32; self.dim];
                for i in 0..half {
                    let (h_re, h_im) = (h[i], h[half + i]);
                    let (r_re, r_im) = (r[i], r[half + i]);
                    let (t_re, t_im) = (t[i], t[half + i]);
                    // Real part: the identical term order `score_complex` sums, so the real half
                    // of the encoding accumulates to the score bit-for-bit.
                    v[i] = h_re * r_re * t_re + h_re * r_im * t_im + h_im * r_re * t_im
                        - h_im * r_im * t_re;
                    // Imaginary part of h·r·conj(t): Im((A + iB)(t_re − i·t_im)) = B·t_re − A·t_im
                    // with A = Re(h·r), B = Im(h·r).
                    v[half + i] = (h_re * r_im + h_im * r_re) * t_re
                        - (h_re * r_re - h_im * r_im) * t_im;
                }
                v
            }
        }
    }

    /// [`TrainedModel::encode_statement_rows`] for a triple of dict-ids, or `None` if any
    /// constituent is unknown to the model (mirrors [`TrainedModel::score`]): the head/tail need
    /// entity rows, the predicate a relation row. A nested quoted-triple *constituent* resolves
    /// through its own node-level entity row (present only under [`TermScope::Embeddable`]
    /// training where it appeared as a triple endpoint) — composition is deliberately **one level
    /// deep**, never recursive: a
    /// statement encoding lives in interaction space and must not be re-fed as an entity vector.
    pub fn encode_statement(&self, h: Id, r: Id, t: Id) -> Option<Vec<f32>> {
        let hr = self.entity_row(h)?;
        let rr = self.rel_row(r)?;
        let tr = self.entity_row(t)?;
        Some(self.encode_statement_rows(hr, rr, tr))
    }

    /// The compositional STATEMENT-level encoding of the RDF 1.2 **quoted-triple term** `term` —
    /// unpack its `(s, p, o)` constituent ids from the dictionary and delegate to
    /// [`TrainedModel::encode_statement`]. `None` if `term` is not a quoted-triple term or any
    /// constituent is unknown to the model. [SONNET-4.6] sq-1e5kk.
    ///
    /// This is the compositional counterpart of the **node-level** quoted-terms path (which stays
    /// intact and independent): under [`TermScope::Embeddable`] a quoted term gets an *opaque*
    /// entity row ([`TrainedModel::entity_row`]); this encoder
    /// instead composes a vector from the quoted `(s, p, o)` *content*. The two coexist — and
    /// because the constituents are ordinary atomic terms, the statement encoding is available
    /// even under the default `IriBlank` scope, where the quoted term itself has **no** row.
    pub fn encode_quoted_term(&self, graph: &Graph, term: Id) -> Option<Vec<f32>> {
        match graph.dict.term_parts(term) {
            TermParts::Triple([h, p, t]) => self.encode_statement(h, p, t),
            _ => None,
        }
    }

    /// Mean L2 norm of the entity embeddings — a cheap **non-degeneracy** probe. A collapsed model
    /// (all rows equal / all-zero) has a tiny or zero spread; see [`TrainedModel::row_spread`].
    pub fn mean_entity_norm(&self) -> f32 {
        if self.entity_row.is_empty() {
            return 0.0;
        }
        let n = self.num_entities();
        let mut total = 0.0f32;
        for row in 0..n {
            let v = self.entity_vec(row);
            total += v.iter().map(|x| x * x).sum::<f32>().sqrt();
        }
        total / n as f32
    }

    /// Mean pairwise-ish spread of entity rows: the mean over dimensions of the per-dimension
    /// standard deviation across rows. Near-zero ⇒ all entity vectors collapsed to one point (a
    /// degenerate solution). Used by the non-degeneracy test.
    pub fn row_spread(&self) -> f32 {
        let n = self.num_entities();
        if n < 2 {
            return 0.0;
        }
        let mut total_std = 0.0f32;
        for d in 0..self.dim {
            let mut mean = 0.0f32;
            for row in 0..n {
                mean += self.entity_emb[row * self.dim + d];
            }
            mean /= n as f32;
            let mut var = 0.0f32;
            for row in 0..n {
                let x = self.entity_emb[row * self.dim + d] - mean;
                var += x * x;
            }
            total_std += (var / n as f32).sqrt();
        }
        total_std / self.dim as f32
    }
}

/// Per-run training diagnostics. **Non-canonical** — these are *learning* signals (the loss curve),
/// not a quality or performance metric, and must never be baked into committed docs.
#[derive(Clone, Debug)]
pub struct TrainReport {
    /// Mean BCE loss per epoch (length == `config.epochs`), in epoch order.
    pub epoch_loss: Vec<f32>,
    /// Positive triples trained on (object-property triples of the graph).
    pub positives: usize,
    /// Entity rows / relation rows in the model.
    pub entities: usize,
    pub relations: usize,
}

impl TrainReport {
    /// The first epoch's mean loss (the "before learning" baseline).
    pub fn first_loss(&self) -> Option<f32> {
        self.epoch_loss.first().copied()
    }
    /// The last epoch's mean loss.
    pub fn last_loss(&self) -> Option<f32> {
        self.epoch_loss.last().copied()
    }
    /// Did the loss decrease overall (last < first)? The load-bearing "it learned" check.
    pub fn loss_decreased(&self) -> bool {
        match (self.first_loss(), self.last_loss()) {
            (Some(a), Some(b)) => b < a,
            _ => false,
        }
    }
}

/// The output of [`collect_positives`]: the positive triples, the entity id→row map, and the
/// relation id→row map.
type CollectedPositives = (Vec<[Id; 3]>, FxHashMap<Id, usize>, FxHashMap<Id, usize>);

/// Collect the **object-property** positives `(h, r, t)` of `graph` (both ends embeddable under
/// `scope` — see [`is_embeddable`]) plus the distinct entity and relation id sets, assigning each
/// a dense parameter row (sorted id order, so row assignment is deterministic and
/// platform-independent). Under [`TermScope::IriBlank`] this is the identical collection the
/// pre-scope trainer performed; under [`TermScope::Embeddable`], `rdf:reifies` edges become
/// positives and their quoted terms get entity rows.
fn collect_positives(graph: &Graph, scope: TermScope) -> CollectedPositives {
    let mut positives: Vec<[Id; 3]> = Vec::new();
    let mut entity_ids: Vec<Id> = Vec::new();
    let mut rel_ids: Vec<Id> = Vec::new();
    let mut seen_e = rustc_hash::FxHashSet::default();
    let mut seen_r = rustc_hash::FxHashSet::default();

    for [s, p, o] in graph.iter_ids() {
        if is_embeddable(graph, s, scope) && is_embeddable(graph, o, scope) {
            positives.push([s, p, o]);
            if seen_e.insert(s) {
                entity_ids.push(s);
            }
            if seen_e.insert(o) {
                entity_ids.push(o);
            }
            if seen_r.insert(p) {
                rel_ids.push(p);
            }
        }
    }
    entity_ids.sort_unstable();
    rel_ids.sort_unstable();
    let entity_row: FxHashMap<Id, usize> = entity_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect();
    let rel_row: FxHashMap<Id, usize> =
        rel_ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    (positives, entity_row, rel_row)
}

/// Train a [`TrainedModel`] over `graph` (which should already be **closed** if the closure arm of
/// the ablation is on — call [`materialise_closure`](crate::structure::materialise_closure) first).
///
/// `constraints` are the P0 type constraints mined from the same graph; they only bite when
/// `config.sampling` is [`SamplingMode::TypeConstrained`] (otherwise the sampler ignores them).
///
/// Returns the trained model and a [`TrainReport`] (the per-epoch loss curve). The loop:
/// 1. init every entity/relation parameter from the seeded PRNG in `(-1, 1)`, scaled by `1/√dim`;
/// 2. for each epoch, shuffle the positives (seeded), and for each positive take one SGD step over
///    the positive plus `negatives_per_positive` sampled negatives (half head-, half tail-corrupt);
/// 3. accumulate the per-step BCE into the epoch's mean loss.
///
/// CPU-only, single-threaded, allocation-light: the only large buffers are the two parameter
/// matrices. Determinism: identical `(graph, constraints, config)` ⇒ identical model.
pub fn train(
    graph: &Graph,
    constraints: &TypeConstraints,
    config: TrainConfig,
) -> (TrainedModel, TrainReport) {
    let (positives, entity_row, rel_row) = collect_positives(graph, config.term_scope);
    // ComplEx needs an even `dim` (real + imaginary halves); round an odd value down. DistMult is
    // unconstrained. `dim.max(2)` keeps a degenerate dim=0 config from producing empty rows.
    let dim = match config.model {
        ModelKind::DistMult => config.dim.max(1),
        ModelKind::ComplEx => (config.dim & !1usize).max(2),
    };
    let n_ent = entity_row.len();
    let n_rel = rel_row.len();

    // Parameter init: small uniform values scaled by 1/√dim (a standard KGE init that keeps the
    // initial trilinear score near zero so the sigmoid starts in its linear regime).
    let scale = 1.0 / (dim as f32).sqrt();
    let mut init_state = config.seed ^ 0xA5A5_5A5A_F0F0_0F0F;
    let mut entity_emb = vec![0.0f32; n_ent * dim];
    for x in entity_emb.iter_mut() {
        *x = next_unit(&mut init_state) * scale;
    }
    let mut rel_emb = vec![0.0f32; n_rel.max(1) * dim];
    for x in rel_emb.iter_mut() {
        *x = next_unit(&mut init_state) * scale;
    }

    let sampler =
        NegativeSampler::new_scoped(graph, constraints, config.sampling, config.term_scope);

    // [OPUS-4.8] sq-2489d.4 (Phase 4): mine the per-triple provenance weights ONCE. Under
    // `WeightMode::Uniform` this still reads the graph but every `weight_of` returns 1.0, so the
    // loop below is byte-identical to the unweighted trainer (the ablation-off baseline); the read
    // is cheap (a single id-scan) and keeps the two arms sharing the same code path.
    let prov_weights = crate::provenance::ProvenanceWeights::mine(graph);

    let mut epoch_loss = Vec::with_capacity(config.epochs);
    let lr = config.lr;
    let l2 = config.l2;
    let half_neg = config.negatives_per_positive.div_ceil(2);

    for epoch in 0..config.epochs {
        // Deterministic per-epoch shuffle: Fisher–Yates over an index permutation seeded by
        // (seed, epoch), so the positive order varies between epochs but reproduces across runs.
        let mut order: Vec<usize> = (0..positives.len()).collect();
        let mut shuf_state = config.seed ^ (epoch as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for i in (1..order.len()).rev() {
            let j = (splitmix64(&mut shuf_state) as usize) % (i + 1);
            order.swap(i, j);
        }

        let mut loss_sum = 0.0f64;
        let mut loss_terms = 0usize;

        for &pi in &order {
            let [h, r, t] = positives[pi];
            // rows are guaranteed present: every positive's ends/relation were rowed above.
            let hr = entity_row[&h];
            let rr = rel_row[&r];
            let tr = entity_row[&t];

            // Negative-sampling seed: stable per (seed, epoch, positive index) so the draws are
            // reproducible yet differ across epochs (the trainer sees fresh negatives each pass).
            let neg_seed = config
                .seed
                .wrapping_add((epoch as u64).wrapping_mul(0x100_0000))
                .wrapping_add(pi as u64);

            // The training batch: 1 positive (label 1) + negatives (label 0).
            // [OPUS-4.8] sq-2489d.4 (Phase 4): the CKRL confidence-weighted-loss move — scale the
            // POSITIVE step's effective learning rate by the triple's provenance weight `w(t)` so a
            // high-assurance fact contributes a full-strength gradient and a low-assurance/low-source
            // fact a down-weighted one. Negatives are NOT weighted (a corruption has no provenance of
            // its own; weighting only the positive is the canonical CKRL form). Under
            // `WeightMode::Uniform` `w == 1.0`, so `lr * w == lr` and the step is unchanged.
            let w = prov_weights.weight_of([h, r, t], config.weight_mode);
            let (l, _) = step(
                config.model,
                &mut entity_emb,
                &mut rel_emb,
                dim,
                hr,
                rr,
                tr,
                1.0,
                lr * w,
                l2,
            );
            loss_sum += l as f64;
            loss_terms += 1;

            // Tail-corrupted negatives.
            for neg in sampler.sample([h, r, t], Corrupt::Tail, half_neg, neg_seed) {
                let ntr = entity_row[&neg[2]];
                let (l, _) = step(
                    config.model,
                    &mut entity_emb,
                    &mut rel_emb,
                    dim,
                    hr,
                    rr,
                    ntr,
                    0.0,
                    lr,
                    l2,
                );
                loss_sum += l as f64;
                loss_terms += 1;
            }
            // Head-corrupted negatives.
            for neg in sampler.sample(
                [h, r, t],
                Corrupt::Head,
                config.negatives_per_positive - half_neg,
                neg_seed ^ 0xDEAD_BEEF,
            ) {
                let nhr = entity_row[&neg[0]];
                let (l, _) = step(
                    config.model,
                    &mut entity_emb,
                    &mut rel_emb,
                    dim,
                    nhr,
                    rr,
                    tr,
                    0.0,
                    lr,
                    l2,
                );
                loss_sum += l as f64;
                loss_terms += 1;
            }
        }

        let mean = if loss_terms == 0 {
            0.0
        } else {
            (loss_sum / loss_terms as f64) as f32
        };
        epoch_loss.push(mean);
    }

    let report = TrainReport {
        epoch_loss,
        positives: positives.len(),
        entities: n_ent,
        relations: n_rel,
    };
    let model = TrainedModel {
        model: config.model,
        dim,
        entity_emb,
        rel_emb,
        entity_row,
        rel_row,
    };
    (model, report)
}

/// One SGD step on a single labelled triple (rows `h,r,t`, `label ∈ {0,1}`), dispatched on the
/// [`ModelKind`]. BCE loss `L = -[y·log σ(s) + (1-y)·log(1-σ(s))]` over the model score `s`; the
/// gradient w.r.t. `s` is `σ(s) - y`; an L2 penalty `½·l2·‖θ‖²` (gradient `l2·θ`) is added to the
/// touched rows. Returns `(loss, score)`.
#[allow(clippy::too_many_arguments)]
fn step(
    model: ModelKind,
    entity_emb: &mut [f32],
    rel_emb: &mut [f32],
    dim: usize,
    h_row: usize,
    r_row: usize,
    t_row: usize,
    label: f32,
    lr: f32,
    l2: f32,
) -> (f32, f32) {
    match model {
        ModelKind::DistMult => {
            step_distmult(entity_emb, rel_emb, dim, h_row, r_row, t_row, label, lr, l2)
        }
        ModelKind::ComplEx => {
            step_complex(entity_emb, rel_emb, dim, h_row, r_row, t_row, label, lr, l2)
        }
    }
}

/// DistMult SGD step. Score `s = Σ_i e_h[i]·w_r[i]·e_t[i]`; the parameter partials are the standard
/// trilinear products: `dL/de_h[i] = g·w_r[i]·e_t[i]`, `dL/dw_r[i] = g·e_h[i]·e_t[i]`,
/// `dL/de_t[i] = g·e_h[i]·w_r[i]` where `g = σ(s) - y`.
#[allow(clippy::too_many_arguments)]
fn step_distmult(
    entity_emb: &mut [f32],
    rel_emb: &mut [f32],
    dim: usize,
    h_row: usize,
    r_row: usize,
    t_row: usize,
    label: f32,
    lr: f32,
    l2: f32,
) -> (f32, f32) {
    let hb = h_row * dim;
    let rb = r_row * dim;
    let tb = t_row * dim;

    // Forward: score.
    let mut s = 0.0f32;
    for i in 0..dim {
        s += entity_emb[hb + i] * rel_emb[rb + i] * entity_emb[tb + i];
    }
    let (g, loss) = bce_grad(s, label);

    // Parameter gradients (note h_row could equal t_row for a self-loop; we read into locals
    // first so the simultaneous update is consistent, then write back).
    for i in 0..dim {
        let eh = entity_emb[hb + i];
        let et = entity_emb[tb + i];
        let wr = rel_emb[rb + i];

        let gh = g * wr * et + l2 * eh;
        let gt = g * eh * wr + l2 * et;
        let gr = g * eh * et + l2 * wr;

        entity_emb[hb + i] = eh - lr * gh;
        // If h_row == t_row this second write must use the *updated* h slot's logical value, but
        // because gh and gt are computed from the pre-update reads above, applying both as separate
        // subtractions on the same cell yields eh - lr*gh - lr*gt, which is the correct combined
        // gradient step for a shared parameter. So read-then-write per index is safe.
        entity_emb[tb + i] -= lr * gt;
        rel_emb[rb + i] = wr - lr * gr;
    }

    (loss, s)
}

/// ComplEx SGD step. Each row holds `half = dim/2` complex components (real half `[0..half]`,
/// imaginary half `[half..dim]`). Score `s = Re(⟨e_h, w_r, conj(e_t)⟩)` (see [`score_complex`]).
/// With `g = σ(s) - y`, the closed-form partials of each component pair are derived directly from
/// `s = Σ_i [ h_re·r_re·t_re + h_re·r_im·t_im + h_im·r_re·t_im − h_im·r_im·t_re ]`:
/// the asymmetry lives entirely in the sign of the `h_im·r_im·t_re` term, which is why the head and
/// tail gradients are NOT interchangeable (unlike DistMult).
#[allow(clippy::too_many_arguments)]
fn step_complex(
    entity_emb: &mut [f32],
    rel_emb: &mut [f32],
    dim: usize,
    h_row: usize,
    r_row: usize,
    t_row: usize,
    label: f32,
    lr: f32,
    l2: f32,
) -> (f32, f32) {
    let half = dim / 2;
    let hb = h_row * dim;
    let rb = r_row * dim;
    let tb = t_row * dim;

    // Forward.
    let mut s = 0.0f32;
    for i in 0..half {
        let (h_re, h_im) = (entity_emb[hb + i], entity_emb[hb + half + i]);
        let (r_re, r_im) = (rel_emb[rb + i], rel_emb[rb + half + i]);
        let (t_re, t_im) = (entity_emb[tb + i], entity_emb[tb + half + i]);
        s += h_re * r_re * t_re + h_re * r_im * t_im + h_im * r_re * t_im - h_im * r_im * t_re;
    }
    let (g, loss) = bce_grad(s, label);

    // Component-wise gradients. We read ALL six components into locals first (h and t may be the
    // same row — a self-loop — so a pre-read keeps the simultaneous update consistent; the separate
    // subtractions then compose to the correct combined step exactly as in the DistMult case).
    for i in 0..half {
        let h_re = entity_emb[hb + i];
        let h_im = entity_emb[hb + half + i];
        let r_re = rel_emb[rb + i];
        let r_im = rel_emb[rb + half + i];
        let t_re = entity_emb[tb + i];
        let t_im = entity_emb[tb + half + i];

        // ∂s/∂h_re = r_re·t_re + r_im·t_im ; ∂s/∂h_im = r_re·t_im − r_im·t_re
        let d_h_re = g * (r_re * t_re + r_im * t_im) + l2 * h_re;
        let d_h_im = g * (r_re * t_im - r_im * t_re) + l2 * h_im;
        // ∂s/∂r_re = h_re·t_re + h_im·t_im ; ∂s/∂r_im = h_re·t_im − h_im·t_re
        let d_r_re = g * (h_re * t_re + h_im * t_im) + l2 * r_re;
        let d_r_im = g * (h_re * t_im - h_im * t_re) + l2 * r_im;
        // ∂s/∂t_re = h_re·r_re − h_im·r_im ; ∂s/∂t_im = h_re·r_im + h_im·r_re
        let d_t_re = g * (h_re * r_re - h_im * r_im) + l2 * t_re;
        let d_t_im = g * (h_re * r_im + h_im * r_re) + l2 * t_im;

        entity_emb[hb + i] = h_re - lr * d_h_re;
        entity_emb[hb + half + i] = h_im - lr * d_h_im;
        rel_emb[rb + i] = r_re - lr * d_r_re;
        rel_emb[rb + half + i] = r_im - lr * d_r_im;
        // Read-then-write per index: if h_row == t_row these subtractions land on the same cell and
        // compose to the correct combined head+tail gradient (the locals were read pre-update).
        entity_emb[tb + i] -= lr * d_t_re;
        entity_emb[tb + half + i] -= lr * d_t_im;
    }

    (loss, s)
}

/// Shared BCE forward+backward at the score level: returns `(dL/ds = σ(s) − y, loss)` with a
/// numerically-stable, eps-clamped log.
#[inline]
fn bce_grad(s: f32, label: f32) -> (f32, f32) {
    let p = sigmoid(s);
    let eps = 1e-7f32;
    let pc = p.clamp(eps, 1.0 - eps);
    let loss = -(label * pc.ln() + (1.0 - label) * (1.0 - pc).ln());
    (p - label, loss)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::close_for_vectorise;
    use sparq_reason::Profile;

    const TTL: &str = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:   <http://ex/> .

ex:Dog   rdfs:subClassOf ex:Animal .
ex:Cat   rdfs:subClassOf ex:Animal .
ex:Owner rdfs:subClassOf ex:Person .

ex:owns rdfs:domain ex:Person ; rdfs:range ex:Animal .

ex:alice a ex:Owner . ex:bob a ex:Owner . ex:carol a ex:Owner .
ex:rex a ex:Dog . ex:tom a ex:Cat . ex:fido a ex:Dog . ex:milo a ex:Cat .

ex:alice ex:owns ex:rex .
ex:bob   ex:owns ex:tom .
ex:carol ex:owns ex:fido .
ex:alice ex:owns ex:milo .
"#;

    #[test]
    fn trains_and_loss_decreases() {
        let c = close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let cfg = TrainConfig::small(SamplingMode::TypeConstrained, 7);
        let (model, report) = train(&c.graph, &tc, cfg);
        assert!(
            report.positives >= 4,
            "expected the 4 owns triples as positives"
        );
        assert!(model.num_entities() >= 7);
        assert!(model.num_relations() >= 1);
        assert!(
            report.loss_decreased(),
            "loss must decrease: first={:?} last={:?}",
            report.first_loss(),
            report.last_loss()
        );
    }

    #[test]
    fn embeddings_are_non_degenerate() {
        let c = close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let cfg = TrainConfig::small(SamplingMode::TypeConstrained, 11);
        let (model, _) = train(&c.graph, &tc, cfg);
        // Non-degeneracy: rows must not have collapsed to a single point.
        assert!(
            model.row_spread() > 1e-3,
            "entity rows collapsed: spread={}",
            model.row_spread()
        );
        assert!(model.mean_entity_norm() > 1e-3, "entity norms collapsed");
    }

    #[test]
    fn is_deterministic_for_fixed_config() {
        let c = close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let cfg = TrainConfig::small(SamplingMode::TypeConstrained, 3);
        let (m1, r1) = train(&c.graph, &tc, cfg);
        let (m2, r2) = train(&c.graph, &tc, cfg);
        assert_eq!(r1.epoch_loss, r2.epoch_loss, "loss curve must reproduce");
        assert_eq!(m1.entity_emb, m2.entity_emb, "entity params must reproduce");
        assert_eq!(m1.rel_emb, m2.rel_emb, "relation params must reproduce");
    }

    #[test]
    fn learns_to_rank_true_above_false() {
        // After training, a known-true positive should score higher than a type-valid corruption
        // for most positives (the basic "it learned the signal" check, weaker than the harness MRR).
        let c = close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let cfg = TrainConfig::small(SamplingMode::TypeConstrained, 5);
        let (model, _) = train(&c.graph, &tc, cfg);
        let owns = c.graph.id_of(&iri("http://ex/owns")).unwrap();
        let alice = c.graph.id_of(&iri("http://ex/alice")).unwrap();
        let rex = c.graph.id_of(&iri("http://ex/rex")).unwrap();
        let tom = c.graph.id_of(&iri("http://ex/tom")).unwrap();
        // (alice owns rex) is true; (alice owns tom) is false (bob owns tom).
        let pos = model.score(alice, owns, rex).unwrap();
        let neg = model.score(alice, owns, tom).unwrap();
        assert!(
            pos > neg,
            "true triple {} should outrank false {}",
            pos,
            neg
        );
    }

    /// A purely DIRECTIONAL relation: a chain `a0 -> a1 -> a2 -> ...` under `ex:next`, never the
    /// reverse. This is the structure on which a symmetric model is provably near-random.
    fn directional_chain_ttl(n: usize) -> String {
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..n - 1 {
            ttl.push_str(&format!("ex:a{} ex:next ex:a{} .\n", i, i + 1));
        }
        ttl
    }

    #[test]
    fn complex_breaks_distmult_symmetry() {
        // The load-bearing must_fix #2 invariant: on a directional relation the SYMMETRIC DistMult
        // scores (a next b) == (b next a) by construction, while the ASYMMETRIC ComplEx can make
        // them differ. We verify the symmetry property directly (it is exact for DistMult and must
        // be broken by ComplEx), not via a noisy metric.
        let ttl = directional_chain_ttl(40);
        let c = close_for_vectorise(&ttl, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let next = c.graph.id_of(&iri("http://ex/next")).unwrap();
        let a0 = c.graph.id_of(&iri("http://ex/a0")).unwrap();
        let a1 = c.graph.id_of(&iri("http://ex/a1")).unwrap();

        // DistMult: the forward and reverse scores are MATHEMATICALLY identical (structural
        // symmetry); they differ only by f32 summation-order rounding, which is exactly the point —
        // DistMult cannot represent any directional preference, so it is near-random on this slice.
        let dm_cfg =
            TrainConfig::small_with_model(ModelKind::DistMult, SamplingMode::Unconstrained, 7);
        let (dm, _) = train(&c.graph, &tc, dm_cfg);
        let fwd = dm.score(a0, next, a1).unwrap();
        let rev = dm.score(a1, next, a0).unwrap();
        assert!(
            (fwd - rev).abs() <= 1e-5 * fwd.abs().max(1.0),
            "DistMult MUST be (mathematically) symmetric: (a0 next a1)={} vs (a1 next a0)={}",
            fwd,
            rev
        );

        // ComplEx: trained on the forward-only chain, it must learn to score the forward edge above
        // its reverse on average over the chain (the asymmetry it exists to capture).
        let cx_cfg =
            TrainConfig::small_with_model(ModelKind::ComplEx, SamplingMode::Unconstrained, 7);
        let (cx, _) = train(&c.graph, &tc, cx_cfg);
        let mut wins = 0usize;
        let mut nontrivial_gap = 0usize;
        let mut total = 0usize;
        for i in 0..39 {
            let x = c.graph.id_of(&iri(&format!("http://ex/a{}", i))).unwrap();
            let y = c
                .graph
                .id_of(&iri(&format!("http://ex/a{}", i + 1)))
                .unwrap();
            let f = cx.score(x, next, y).unwrap();
            let r = cx.score(y, next, x).unwrap();
            if f > r {
                wins += 1;
            }
            // The forward/reverse scores must differ by a NON-TRIVIAL margin for most edges — the
            // asymmetry is structural in ComplEx and must not collapse to float noise.
            if (f - r).abs() > 1e-3 {
                nontrivial_gap += 1;
            }
            total += 1;
        }
        // ComplEx must DISTINGUISH direction on a clear majority of edges (not float noise) and
        // PREFER the trained forward direction on a clear majority.
        assert!(
            nontrivial_gap * 2 > total,
            "ComplEx must distinguish direction with a real gap on most edges: {}/{}",
            nontrivial_gap,
            total
        );
        assert!(
            wins * 2 > total,
            "ComplEx should rank forward > reverse for most directional edges: {}/{}",
            wins,
            total
        );
    }

    #[test]
    fn complex_trains_and_is_non_degenerate_and_deterministic() {
        let c = close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let cfg =
            TrainConfig::small_with_model(ModelKind::ComplEx, SamplingMode::TypeConstrained, 5);
        let (m1, r1) = train(&c.graph, &tc, cfg);
        assert!(r1.loss_decreased(), "ComplEx loss must decrease");
        assert!(m1.row_spread() > 1e-3, "ComplEx rows collapsed");
        assert!(m1.mean_entity_norm() > 1e-3, "ComplEx norms collapsed");
        assert_eq!(m1.model, ModelKind::ComplEx);
        // Even dim is enforced.
        assert_eq!(m1.dim % 2, 0, "ComplEx dim must be even");
        // Determinism.
        let (m2, r2) = train(&c.graph, &tc, cfg);
        assert_eq!(r1.epoch_loss, r2.epoch_loss);
        assert_eq!(m1.entity_emb, m2.entity_emb);
        assert_eq!(m1.rel_emb, m2.rel_emb);
    }

    #[test]
    fn complex_odd_dim_rounds_down_to_even() {
        let c = close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let mut cfg =
            TrainConfig::small_with_model(ModelKind::ComplEx, SamplingMode::Unconstrained, 1);
        cfg.dim = 31; // odd → rounded to 30
        let (m, _) = train(&c.graph, &tc, cfg);
        assert_eq!(m.dim, 30, "odd ComplEx dim must round down to even");
    }

    fn iri(s: &str) -> oxrdf::Term {
        oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
    }

    // ---- RDF 1.2 quoted-terms visibility (TermScope::Embeddable) ------------------------------

    #[test]
    fn presets_default_to_the_byte_stable_scope() {
        // The presets audit: every constructor produces the OFF (byte-identical) baseline.
        let a = TrainConfig::small(SamplingMode::TypeConstrained, 1);
        assert_eq!(a.term_scope, TermScope::IriBlank);
        let b = TrainConfig::small_with_model(ModelKind::DistMult, SamplingMode::Unconstrained, 2);
        assert_eq!(b.term_scope, TermScope::IriBlank);
    }

    #[test]
    fn embeddable_scope_gives_quoted_terms_rows_and_learns() {
        // Under `TermScope::Embeddable` on the RDF 1.2 slice: `rdf:reifies` edges become
        // positives, quoted terms get entity rows, training is deterministic, and loss decreases.
        // Under the default `IriBlank` the same graph yields FEWER positives and NO quoted rows.
        use sparq_core::dict::TermParts;
        let ttl = crate::eval::synthetic_rdf12_ttl(48, 7);
        let c = close_for_vectorise(&ttl, "ntriples", Profile::Rdfs).unwrap();
        let g = &c.graph;
        let tc = TypeConstraints::mine(g);

        let quoted: Vec<sparq_core::dict::Id> = {
            let mut ids: Vec<_> = g
                .iter_ids()
                .map(|[_, _, o]| o)
                .filter(|&o| matches!(g.dict.term_parts(o), TermParts::Triple(_)))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        assert!(
            !quoted.is_empty(),
            "the rdf12 slice must carry quoted terms"
        );

        let mut on_cfg = TrainConfig::small(SamplingMode::Unconstrained, 5);
        on_cfg.term_scope = TermScope::Embeddable;
        let off_cfg = TrainConfig::small(SamplingMode::Unconstrained, 5);

        let (on_model, on_report) = train(g, &tc, on_cfg);
        let (off_model, off_report) = train(g, &tc, off_cfg);

        // Visibility: the ON arm counts every `rdf:reifies` positive the OFF arm drops, and every
        // quoted term has a dense row; the OFF arm has none.
        assert!(
            on_report.positives > off_report.positives,
            "reifies positives must be visible ON ({}) and dropped OFF ({})",
            on_report.positives,
            off_report.positives
        );
        // Statement subjects are atomic and already rowed in BOTH arms (via their `rdf:type` /
        // `ex:assertedBy` metadata triples), so the ON−OFF row delta is EXACTLY the quoted terms.
        assert_eq!(on_report.entities, off_report.entities + quoted.len());
        for &tt in &quoted {
            assert!(
                on_model.entity_row(tt).is_some(),
                "quoted term must have an entity row ON"
            );
            assert!(
                off_model.entity_row(tt).is_none(),
                "quoted term must have NO row OFF"
            );
        }

        // It learns, and it reproduces.
        assert!(on_report.loss_decreased(), "ON-arm loss must decrease");
        let (on_model2, on_report2) = train(g, &tc, on_cfg);
        assert_eq!(on_report.epoch_loss, on_report2.epoch_loss);
        assert_eq!(on_model.entity_emb, on_model2.entity_emb);
        assert_eq!(on_model.rel_emb, on_model2.rel_emb);
    }

    // ---- Compositional STATEMENT-level encoder (sq-1e5kk) -------------------------------------

    #[test]
    fn statement_encoding_sums_to_the_score_for_both_models() {
        // The load-bearing construction invariant: the encoding is the model's OWN interaction
        // product before the final sum, so summing it (DistMult: all of it; ComplEx: the real
        // half) must recover the scalar score — a wrong composition goes red here.
        let c = close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let owns = c.graph.id_of(&iri("http://ex/owns")).unwrap();
        let alice = c.graph.id_of(&iri("http://ex/alice")).unwrap();
        let rex = c.graph.id_of(&iri("http://ex/rex")).unwrap();
        let tom = c.graph.id_of(&iri("http://ex/tom")).unwrap();

        for kind in [ModelKind::DistMult, ModelKind::ComplEx] {
            let cfg = TrainConfig::small_with_model(kind, SamplingMode::TypeConstrained, 5);
            let (model, _) = train(&c.graph, &tc, cfg);

            let v = model.encode_statement(alice, owns, rex).unwrap();
            assert_eq!(v.len(), model.dim, "{:?}: full-dim encoding", kind);
            let score = model.score(alice, owns, rex).unwrap();
            let summed: f32 = match kind {
                ModelKind::DistMult => v.iter().sum(),
                ModelKind::ComplEx => v[..model.dim / 2].iter().sum(),
            };
            assert!(
                (summed - score).abs() <= 1e-5 * score.abs().max(1.0),
                "{:?}: encoding must sum to the model score: {} vs {}",
                kind,
                summed,
                score
            );

            // Compositional: changing one constituent changes the encoding.
            let v2 = model.encode_statement(alice, owns, tom).unwrap();
            assert_ne!(v, v2, "{:?}: the tail must change the encoding", kind);

            // Unknown constituents abstain: `alice` has no RELATION row.
            assert!(
                model.encode_statement(alice, alice, rex).is_none(),
                "{:?}: a non-relation predicate must yield None",
                kind
            );
        }
    }

    #[test]
    fn quoted_terms_encode_compositionally_under_both_scopes() {
        // The statement-level encoder gives compositional access to a quoted triple's (s, p, o)
        // content under BOTH scopes — including the default `IriBlank`, where the quoted term has
        // no node row at all — and leaves the node-level path intact under `Embeddable`.
        let ttl = crate::eval::synthetic_rdf12_ttl(48, 7);
        let c = close_for_vectorise(&ttl, "ntriples", Profile::Rdfs).unwrap();
        let g = &c.graph;
        let tc = TypeConstraints::mine(g);

        // A quoted term + its constituents (the slice reifies BASE claims, so every constituent
        // is an already-rowed atomic term under both scopes).
        let tt = g
            .iter_ids()
            .map(|[_, _, o]| o)
            .find(|&o| matches!(g.dict.term_parts(o), TermParts::Triple(_)))
            .expect("the rdf12 slice must carry quoted terms");
        let TermParts::Triple([h, p, t]) = g.dict.term_parts(tt) else {
            unreachable!()
        };

        // OFF arm (default `IriBlank`): NO node-level row, yet the statement encoding composes.
        let (off, _) = train(g, &tc, TrainConfig::small(SamplingMode::Unconstrained, 5));
        assert!(off.entity_row(tt).is_none(), "no node-level row OFF");
        let v = off
            .encode_quoted_term(g, tt)
            .expect("statement encoding must compose from the constituents OFF");
        assert_eq!(
            v,
            off.encode_statement(h, p, t).unwrap(),
            "quoted-term encoding == the encoding of its unpacked (s, p, o)"
        );
        let score = off.score(h, p, t).unwrap();
        let summed: f32 = v[..off.dim / 2].iter().sum(); // small() is ComplEx
        assert!(
            (summed - score).abs() <= 1e-5 * score.abs().max(1.0),
            "real half must sum to the quoted content's score: {} vs {}",
            summed,
            score
        );
        // An atomic term is not a statement: the encoder abstains (the node-level path for
        // atomic terms is `entity_row`/`entity_vec`, unchanged).
        assert!(off.encode_quoted_term(g, h).is_none());

        // ON arm (`Embeddable`): the node-level opaque row is INTACT and the statement-level
        // encoding coexists as a distinct representation.
        let mut on_cfg = TrainConfig::small(SamplingMode::Unconstrained, 5);
        on_cfg.term_scope = TermScope::Embeddable;
        let (on, _) = train(g, &tc, on_cfg);
        let row = on.entity_row(tt).expect("node-level row intact ON");
        let sv = on.encode_quoted_term(g, tt).expect("statement encoding ON");
        assert_ne!(
            sv.as_slice(),
            on.entity_vec(row),
            "statement-level encoding must be distinct from the node-level row"
        );

        // Derived, no new parameters ⇒ determinism is inherited from the trainer.
        let (on2, _) = train(g, &tc, on_cfg);
        assert_eq!(sv, on2.encode_quoted_term(g, tt).unwrap());
    }
}
