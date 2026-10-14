//! Structure-aware vectorisation **preprocessing + sampling-logic** layer (P0).
//!
//! sq-0wo9e.1 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §5.A + §2 row "domain/range"). This module is the **lowest-risk, additive** slice of the
//! structure-aware-vectorisation epic. It is **opt-in** (the `structure` cargo feature, off by
//! default) and changes **nothing** in the default `sparq-vectors` build or the core engine.
//!
//! # What this is — and, honestly, what it is NOT
//!
//! `sparq-vectors` is a vector-**search** + **import** surface: embeddings are produced
//! **out-of-process** (see [`crate::embed`]) and the crate has **no KGE training path** — no
//! TransE / ComplEx / RotatE, no triple-scoring, no learning loop, no negative-sampling consumer.
//! P0 of the design ("restrict KGE *corruptions* to type-valid entities") therefore presupposes a
//! trainer that does not exist in-tree. Rather than build a large new training subsystem blind
//! (the design review #996 may reshape it), this module ships the two **buildable, additive**
//! halves of P0 as reusable primitives an out-of-process trainer (or a future in-tree KGE block)
//! consumes:
//!
//! 1. **closure-before-vectorise** ([`close_for_vectorise`] / [`materialise_closure`]) — run the
//!    `sparq-reason` closure ([`Profile::Rdfs`] or [`Profile::OwlRl`]) over the graph **before** the
//!    vectoriser sees it, so entailed type / `subClassOf` / domain / range triples are *materialised
//!    facts* the encoder, the type extractor, and the negative sampler all read.
//! 2. **type-constrained negative sampling** ([`TypeConstraints`] + [`NegativeSampler`]) — the
//!    established Krompass et al. 2015 technique: corrupt a positive triple's head only to
//!    entities whose type lies in the predicate's **domain**, its tail only to entities in the
//!    predicate's **range**. The constraints are read from `sparq-introspect` (declared
//!    `rdfs:domain`/`rdfs:range` **and** observed domain/range histograms) over the **closed**
//!    graph.
//!
//! The KGE trainer that *consumes* these negatives is the deferred remainder (bead noted in the
//! crate README / SKILL). This module trains nothing and serves no exact answer.
//!
//! # The on/off ablation hook (load-bearing, design §6.B)
//!
//! Every prior in this epic ships **behind an on/off ablation** and is adopted **only on measured
//! lift** — no accuracy claim is made in advance. [`SamplingMode`] is that switch:
//! [`SamplingMode::Unconstrained`] reproduces vanilla uniform corruption; [`SamplingMode::TypeConstrained`]
//! applies the domain/range restriction. A harness runs the *same* sampler in both modes and
//! compares Hits@k / MRR downstream. **No benchmark numbers exist; none are claimed here.**

use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Id, TermParts};
use sparq_core::Graph;
use sparq_introspect::Introspection;
use sparq_reason::Profile;

/// `rdf:type` — the predicate whose objects are an entity's declared classes.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// Outcome of [`materialise_closure`] / [`close_for_vectorise`]: the closed [`Graph`] plus a
/// small, **non-canonical** report of how many triples the closure added. The counts exist so a
/// caller can log / assert the closure ran (e.g. "every closed triple is visible to the encoder")
/// — they are *not* a performance or quality metric and must never be baked into docs.
pub struct ClosedGraph {
    /// The graph after materialisation: asserted triples **plus** every entailed triple of the
    /// chosen [`Profile`]. This is the graph the vectoriser, the type extractor, and the negative
    /// sampler should read.
    pub graph: Graph,
    /// Triples present before materialisation.
    pub asserted_triples: usize,
    /// Triples added by the closure (`entailed_triples` ⩾ 0; `0` ⇒ the graph was already closed
    /// under the profile, e.g. it has no schema axioms to fire on).
    pub entailed_triples: usize,
    /// The entailment regime that was materialised.
    pub profile: Profile,
}

impl ClosedGraph {
    /// Total triples after materialisation (`asserted_triples + entailed_triples`).
    pub fn closed_triples(&self) -> usize {
        self.asserted_triples + self.entailed_triples
    }
}

/// Materialise the `profile` closure over an **already-parsed** `(dict, triples)` pair, then
/// build the closed [`Graph`]. This is the closure-before-vectorise step (design §5.A): the
/// reasoner is sound and complete for its profile, so after this call every entailed type /
/// `subClassOf` / domain / range fact is a *real triple* the downstream vectorisation inputs see —
/// not just the asserted subset.
///
/// `dict`/`triples` come from [`Graph::parse_to_triples`]. Ownership is taken because the closed
/// graph is rebuilt from them via [`Graph::from_parts`].
///
/// ```
/// use sparq_core::Graph;
/// use sparq_reason::Profile;
/// use sparq_vectors::structure::materialise_closure;
///
/// let ttl = "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
///            @prefix ex: <http://ex/> .\n\
///            ex:Dog rdfs:subClassOf ex:Animal .\n\
///            ex:rex a ex:Dog .";
/// let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").unwrap();
/// let closed = materialise_closure(dict, triples, Profile::Rdfs);
/// // `ex:rex a ex:Animal` is now an entailed, materialised triple.
/// assert!(closed.entailed_triples >= 1);
/// ```
pub fn materialise_closure(
    mut dict: sparq_core::dict::Dict,
    mut triples: Vec<[Id; 3]>,
    profile: Profile,
) -> ClosedGraph {
    let asserted_triples = triples.len();
    let entailed_triples = sparq_reason::materialize(profile, &mut dict, &mut triples);
    let graph = Graph::from_parts(dict, triples);
    ClosedGraph {
        graph,
        asserted_triples,
        entailed_triples,
        profile,
    }
}

/// Parse RDF `text` of the given `format`, then [`materialise_closure`] under `profile`
/// ([`Profile::Rdfs`] or [`Profile::OwlRl`]) — the convenience entry point for the whole
/// closure-before-vectorise step from a serialised source.
///
/// Returns `Err` with the parser's message on a parse failure (the closure step never fails:
/// materialisation only *adds* triples).
pub fn close_for_vectorise(
    text: &str,
    format: &str,
    profile: Profile,
) -> Result<ClosedGraph, String> {
    let (dict, triples) = Graph::parse_to_triples(text, format)?;
    Ok(materialise_closure(dict, triples, profile))
}

/// Per-predicate **type constraints** mined from a (closed) graph via `sparq-introspect`:
/// the set of class IRIs admissible as a predicate's subject (domain) and object (range), and a
/// per-entity type membership map. This is the input to [`NegativeSampler`].
///
/// "Admissible domain/range" unions the **declared** `rdfs:domain`/`rdfs:range` with the
/// **observed** domain/range histograms `sparq-introspect` mines from usage — declared axioms are
/// often absent or wrong in real data, so the observed classes broaden coverage. A predicate with
/// *no* known domain (or range) classes is treated as **unconstrained** on that side (every
/// entity is a valid corruption), so the sampler never deadlocks on a missing schema — it simply
/// degrades to uniform corruption for that side, which is the honest fail-open behaviour.
pub struct TypeConstraints {
    /// `rdf:type` of each entity id, as the **interned class ids** that entity carries. Entities
    /// with no `rdf:type` are absent (their lookup yields an empty set → unconstrained).
    entity_types: FxHashMap<Id, FxHashSet<Id>>,
    /// For each predicate id: the class ids admissible as its **subject** (domain). Empty ⇒
    /// unconstrained domain.
    predicate_domain: FxHashMap<Id, FxHashSet<Id>>,
    /// For each predicate id: the class ids admissible as its **object** (range). Empty ⇒
    /// unconstrained range.
    predicate_range: FxHashMap<Id, FxHashSet<Id>>,
}

impl TypeConstraints {
    /// Mine type constraints from `graph` (which should already be **closed** — call
    /// [`materialise_closure`] first so entailed `subClassOf`/type/domain/range facts are present).
    ///
    /// Builds the `sparq-introspect` [`Introspection`] once and reads its declared + observed
    /// domain/range histograms; the per-entity `rdf:type` map is read directly from the graph's
    /// `rdf:type` triples (so it includes every entailed type, not just a sampled subset).
    pub fn mine(graph: &Graph) -> TypeConstraints {
        let introspection = Introspection::build(graph);
        Self::from_introspection(graph, &introspection)
    }

    /// Mine type constraints from a graph and a **pre-built** [`Introspection`] (avoids a second
    /// `Introspection::build` when the caller already has one). The introspection must have been
    /// built from the *same* `graph` — class/predicate IRIs are resolved back to ids via the
    /// graph's dictionary.
    pub fn from_introspection(graph: &Graph, introspection: &Introspection) -> TypeConstraints {
        let dict = &graph.dict;

        // Resolve the rdf:type predicate id (NO_ID-equivalent if the graph never uses it → the
        // entity_types map is simply empty and every entity is unconstrained).
        let rdf_type_id = graph.id_of(&type_term());

        // Per-entity rdf:type set, read from the (closed) graph's rdf:type triples directly so it
        // reflects every entailed type. iter_ids is subject-sorted; we just scan once.
        let mut entity_types: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default();
        if let Some(type_pred) = rdf_type_id {
            for [s, p, o] in graph.iter_ids() {
                if p == type_pred {
                    entity_types.entry(s).or_default().insert(o);
                }
            }
        }

        // Resolve a class IRI string to its interned id; classes absent from the dict are skipped
        // (they can never be the type of any entity in this graph, so they cannot constrain).
        let class_id = |iri: &str| -> Option<Id> {
            let id = dict_iri_id(dict, iri);
            id.filter(|&i| i != sparq_core::dict::NO_ID)
        };

        let mut predicate_domain: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default();
        let mut predicate_range: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default();

        for pred in &introspection.predicates {
            let Some(pred_id) = class_id(&pred.predicate) else {
                continue;
            };

            let domain = predicate_domain.entry(pred_id).or_default();
            for c in &pred.declared_domains {
                if let Some(cid) = class_id(c) {
                    domain.insert(cid);
                }
            }
            for c in &pred.inferred_domains {
                if let Some(cid) = class_id(&c.iri) {
                    domain.insert(cid);
                }
            }

            let range = predicate_range.entry(pred_id).or_default();
            for c in &pred.declared_ranges {
                if let Some(cid) = class_id(c) {
                    range.insert(cid);
                }
            }
            for c in &pred.inferred_ranges {
                if let Some(cid) = class_id(&c.iri) {
                    range.insert(cid);
                }
            }
        }

        TypeConstraints {
            entity_types,
            predicate_domain,
            predicate_range,
        }
    }

    /// The interned `rdf:type` class ids of `entity` (empty if it has none).
    pub fn types_of(&self, entity: Id) -> Option<&FxHashSet<Id>> {
        self.entity_types.get(&entity)
    }

    /// Is `entity` admissible as the **subject** of `predicate`? `true` when the predicate has no
    /// known domain (unconstrained), or `entity` carries at least one type in the domain.
    pub fn admissible_subject(&self, predicate: Id, entity: Id) -> bool {
        self.admissible(&self.predicate_domain, predicate, entity)
    }

    /// Is `entity` admissible as the **object** of `predicate`? `true` when the predicate has no
    /// known range (unconstrained), or `entity` carries at least one type in the range.
    pub fn admissible_object(&self, predicate: Id, entity: Id) -> bool {
        self.admissible(&self.predicate_range, predicate, entity)
    }

    fn admissible(&self, side: &FxHashMap<Id, FxHashSet<Id>>, predicate: Id, entity: Id) -> bool {
        match side.get(&predicate) {
            // Unconstrained side (no declared/observed classes) → every entity is admissible.
            None => true,
            Some(classes) if classes.is_empty() => true,
            Some(classes) => match self.entity_types.get(&entity) {
                // The entity has no type → cannot be proven admissible against a constrained
                // side; treat it as inadmissible so type-constrained mode is strictly the
                // restriction the literature describes (Unconstrained mode bypasses this).
                None => false,
                Some(types) => types.iter().any(|t| classes.contains(t)),
            },
        }
    }
}

/// Which term **sorts** participate in KGE entity space — the RDF 1.2 **quoted-terms ablation
/// switch** (mirrors [`SamplingMode`]'s on/off discipline: ship the switch OFF, adopt nothing
/// without a measured, multi-seed, paired-delta result).
///
/// [`TermScope::IriBlank`] is the pre-existing behaviour and the **default everywhere**: with it,
/// `is_embeddable` reduces to the exact named-node/blank-node match the trainer, eval, and
/// sampler previously performed via three private `is_entity` copies — baselines are
/// byte-identical (asserted bit-for-bit by the `kge` tests). [`TermScope::Embeddable`]
/// additionally admits RDF 1.2 quoted-triple terms (`TermParts::Triple`, the object of
/// `rdf:reifies`) into entity space, so statement-level structure — `rdf:reifies` edges and
/// content-addressed shared quoted-term nodes — becomes visible to the embedding layer instead of
/// being silently dropped. Literals stay out under both scopes.
///
/// **What the ON arm does and does not buy:** a quoted term is embedded as a *node* — its
/// `rdf:reifies` edge(s) and hub-sharing (two reifiers of the same claim share one content-
/// addressed quoted-term node) become graph structure the trainer sees. The term's *compositional*
/// `(s, p, o)` content stays opaque to the *trainer*; the deterministic statement-level encoding
/// derived from a trained model is `TrainedModel::encode_quoted_term` (sq-1e5kk), a separate
/// representation whose adoption stays measurement-gated. **No accuracy claim is made** — the
/// switch exists so the quoted-term-visibility axis is measurable at all
/// ([`crate::eval::run_quoted_ablation`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TermScope {
    /// Named + blank nodes only — the ablation **OFF** baseline (and the default). Reduces
    /// `is_embeddable` to exactly the former `is_entity` matcher, so every existing caller's
    /// behaviour is byte-identical.
    #[default]
    IriBlank,
    /// Also RDF 1.2 quoted-triple terms (`TermParts::Triple`) — the ablation **ON** arm.
    Embeddable,
}

/// Is the term id **embeddable** under `scope` — i.e. does it get a row in KGE entity space?
///
/// Under [`TermScope::IriBlank`] this is the identical `Iri | Blank` match the three former
/// private `is_entity` copies (train / eval / sampler) performed — the structural byte-stability
/// guarantee: no float path, PRNG stream, or iteration order changes when the switch is off.
/// Under [`TermScope::Embeddable`] quoted-triple terms are additionally admitted. Literals and
/// inline values are excluded under both scopes.
pub(crate) fn is_embeddable(graph: &Graph, id: Id, scope: TermScope) -> bool {
    match graph.dict.term_parts(id) {
        TermParts::Iri { .. } | TermParts::Blank(_) => true,
        TermParts::Triple(_) => scope == TermScope::Embeddable,
        _ => false,
    }
}

/// Which negative-sampling regime to apply — the **on/off ablation switch** (design §6.B). The
/// sampler runs identically under both modes except for the admissibility filter, so a harness can
/// measure type-constrained vs unconstrained on the *same* seed and positives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplingMode {
    /// Vanilla uniform corruption: any entity is a candidate (the ablation **off** baseline).
    Unconstrained,
    /// Krompass et al. 2015 type-constrained corruption: head corruptions restricted to the
    /// predicate's domain types, tail corruptions to its range types (the ablation **on** arm).
    TypeConstrained,
}

/// Which position of a positive triple a negative corrupts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corrupt {
    /// Replace the head (subject), keeping `(?, r, t)`.
    Head,
    /// Replace the tail (object), keeping `(h, r, ?)`.
    Tail,
}

/// A deterministic, type-aware **negative sampler** for KGE training. It does **not** train — it
/// only *emits* corrupted triples a trainer scores. This is the reusable P0(b) primitive.
///
/// The entity pool is the graph's distinct subjects-and-objects of object-property triples (the
/// nodes a KGE places in entity space). Corruptions are drawn deterministically from a SplitMix64
/// stream seeded per call, so a fixed `(seed, mode)` is reproducible across runs and platforms —
/// the on/off ablation compares like with like.
pub struct NegativeSampler<'a> {
    constraints: &'a TypeConstraints,
    /// Atomic (IRI/blank) candidate entity ids (sorted, deduplicated) — the corruption pool for
    /// atomic slots. Its meaning is unchanged from the pre-[`TermScope`] sampler.
    entities: Vec<Id>,
    /// RDF 1.2 quoted-triple-term ids (sorted, deduplicated) — the corruption pool for
    /// quoted-term slots. **Always empty under [`TermScope::IriBlank`]** (no positive can carry a
    /// quoted-term endpoint there), so the OFF-scope draw loop, PRNG stream, and rejection
    /// sequence are bit-identical to the pre-scope sampler.
    triple_terms: Vec<Id>,
    /// Positive `(h, r, t)` triples, used to reject a corruption that accidentally reproduces a
    /// true triple (the standard "filtered" negative-sampling guard).
    positives: FxHashSet<[Id; 3]>,
    mode: SamplingMode,
}

impl<'a> NegativeSampler<'a> {
    /// Build a sampler over `graph`'s object-property triples (literal objects are skipped — a KGE
    /// places only entities in entity space; literals are handled by the typed-literal encoders of
    /// design Phase 1, not here). `constraints` should be mined from the **same, closed** graph.
    ///
    /// `mode` is the ablation switch ([`SamplingMode`]). The term scope is the byte-stable
    /// [`TermScope::IriBlank`] default — the existing signature and behaviour are preserved;
    /// [`NegativeSampler::new_scoped`] is the quoted-terms opt-in.
    pub fn new(
        graph: &Graph,
        constraints: &'a TypeConstraints,
        mode: SamplingMode,
    ) -> NegativeSampler<'a> {
        Self::new_scoped(graph, constraints, mode, TermScope::IriBlank)
    }

    /// [`NegativeSampler::new`] with an explicit [`TermScope`] — the quoted-terms ablation entry
    /// point. Under [`TermScope::Embeddable`], triples with a quoted-term endpoint (RDF 1.2
    /// `rdf:reifies` edges) count as positives, and their quoted terms land in a **separate,
    /// sort-preserving corruption pool** (see [`NegativeSampler::sample`]).
    pub fn new_scoped(
        graph: &Graph,
        constraints: &'a TypeConstraints,
        mode: SamplingMode,
        scope: TermScope,
    ) -> NegativeSampler<'a> {
        let mut entity_set: FxHashSet<Id> = FxHashSet::default();
        let mut triple_set: FxHashSet<Id> = FxHashSet::default();
        let mut positives: FxHashSet<[Id; 3]> = FxHashSet::default();

        for [s, p, o] in graph.iter_ids() {
            // Only object-property triples (object is an entity — or, under `Embeddable`, a
            // quoted term — not a literal/inline value).
            if is_embeddable(graph, o, scope) && is_embeddable(graph, s, scope) {
                for id in [s, o] {
                    // Route each endpoint into the pool of its sort. Under `IriBlank` no quoted
                    // term can pass `is_embeddable`, so `triple_set` stays empty by construction.
                    if matches!(graph.dict.term_parts(id), TermParts::Triple(_)) {
                        triple_set.insert(id);
                    } else {
                        entity_set.insert(id);
                    }
                }
                positives.insert([s, p, o]);
            }
        }

        // Deterministic order: sort the dedup'd ids ascending, independent of the FxHashSet
        // iteration order, so the sampler is reproducible across runs.
        let mut entities: Vec<Id> = entity_set.into_iter().collect();
        entities.sort_unstable();
        let mut triple_terms: Vec<Id> = triple_set.into_iter().collect();
        triple_terms.sort_unstable();

        NegativeSampler {
            constraints,
            entities,
            triple_terms,
            positives,
            mode,
        }
    }

    /// The number of candidate entities in the **atomic** (IRI/blank) corruption pool.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// The number of quoted-triple-term ids in the quoted corruption pool (0 unless the sampler
    /// was built with [`TermScope::Embeddable`] over a graph bearing RDF 1.2 quoted terms).
    pub fn triple_term_count(&self) -> usize {
        self.triple_terms.len()
    }

    /// Sample up to `n` negatives for the positive triple `(h, r, t)` corrupting `side`, using
    /// `seed` to drive a deterministic stream. Returns fewer than `n` only when the admissible
    /// pool is exhausted (e.g. a tightly-typed predicate with few range entities).
    ///
    /// **Sort-preserving corruption:** the candidate pool is chosen by the *sort of the term being
    /// replaced* — a quoted-term slot draws only from the quoted pool, an atomic slot only from
    /// the atomic pool. Replacing a quoted-triple object with an atomic IRI would yield a
    /// sort-trivial negative the model detects from term class alone, polluting the training
    /// margin (the same type-of-negative hygiene as Krompass type-constrained corruption). Under
    /// [`TermScope::IriBlank`] the quoted pool is empty and no positive carries a quoted term, so
    /// the draw loop, PRNG stream, and rejection sequence are bit-identical to the pre-scope
    /// sampler.
    ///
    /// A candidate is rejected when it (a) equals the original entity, (b) reproduces a true
    /// positive triple (filtered guard), or (c) under [`SamplingMode::TypeConstrained`] is not
    /// admissible for the corrupted side. Under [`SamplingMode::Unconstrained`] only (a) and (b)
    /// apply — that is the ablation baseline. Quoted-term candidates **bypass the class filter**
    /// of (c): a quoted term carries no `rdf:type`, so a constrained side would reject the whole
    /// pool — statements have no class discipline until a statement-typing prior exists (a
    /// tracked follow-up), and deadlocking the ON arm would be dishonest.
    pub fn sample(&self, triple: [Id; 3], side: Corrupt, n: usize, seed: u64) -> Vec<[Id; 3]> {
        let [h, r, t] = triple;
        let original = match side {
            Corrupt::Head => h,
            Corrupt::Tail => t,
        };
        // Sort-preserving pool selection: a slot holding a quoted term draws from the quoted
        // pool. Membership is decided against the (sorted) quoted pool itself — every endpoint of
        // a collected positive is in exactly one pool, and an id in neither pool falls back to the
        // atomic pool (today's behaviour for out-of-graph callers).
        let quoted_slot = self.triple_terms.binary_search(&original).is_ok();
        let pool: &[Id] = if quoted_slot {
            &self.triple_terms
        } else {
            &self.entities
        };
        if pool.is_empty() || n == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(n.min(pool.len()));
        let mut emitted: FxHashSet<Id> = FxHashSet::default();
        let mut state = seed ^ mix_triple(triple) ^ (side_salt(side));

        // Bounded attempts: at most a small multiple of the pool, so a near-empty admissible set
        // terminates instead of spinning. The cap is generous enough that a healthy pool fills `n`.
        let max_attempts = pool.len().saturating_mul(8).max(64);
        for _ in 0..max_attempts {
            if out.len() >= n {
                break;
            }
            let idx = (splitmix64(&mut state) as usize) % pool.len();
            let candidate = pool[idx];

            let (nh, nt) = match side {
                Corrupt::Head => (candidate, t),
                Corrupt::Tail => (h, candidate),
            };

            if candidate == original || emitted.contains(&candidate) {
                continue;
            }
            // Filtered guard: never emit a corruption that is itself a true triple.
            if self.positives.contains(&[nh, r, nt]) {
                continue;
            }
            // Type-constraint filter — the only behavioural difference between the two modes.
            // Quoted-term candidates bypass it (no `rdf:type` on a quoted term; see above).
            if self.mode == SamplingMode::TypeConstrained && !quoted_slot {
                let ok = match side {
                    Corrupt::Head => self.constraints.admissible_subject(r, candidate),
                    Corrupt::Tail => self.constraints.admissible_object(r, candidate),
                };
                if !ok {
                    continue;
                }
            }

            emitted.insert(candidate);
            out.push([nh, r, nt]);
        }
        out
    }
}

// ---- helpers ----------------------------------------------------------------------------------

/// The `rdf:type` named-node term.
fn type_term() -> oxrdf::Term {
    oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(RDF_TYPE))
}

/// Resolve an IRI string to its interned dictionary id, or `None` if absent.
fn dict_iri_id(dict: &sparq_core::dict::Dict, iri: &str) -> Option<Id> {
    let id = dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
        iri,
    )));
    (id != sparq_core::dict::NO_ID).then_some(id)
}

/// Mix a triple into a 64-bit value so the per-triple stream differs for each positive (the
/// sampler is otherwise deterministic for a fixed seed).
fn mix_triple([s, p, o]: [Id; 3]) -> u64 {
    let mut v = (s as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    v ^= (p as u64)
        .rotate_left(21)
        .wrapping_mul(0xBF58_476D_1CE4_E5B9);
    v ^= (o as u64)
        .rotate_left(42)
        .wrapping_mul(0x94D0_49BB_1331_11EB);
    v
}

/// A small per-side salt so head and tail streams of the same triple diverge.
fn side_salt(side: Corrupt) -> u64 {
    match side {
        Corrupt::Head => 0x1111_2222_3333_4444,
        Corrupt::Tail => 0x5555_6666_7777_8888,
    }
}

/// SplitMix64 — a small, fast, well-distributed deterministic PRNG step. Reused (not imported)
/// from the same algorithm `embed.rs` uses; kept local so `structure` carries no cross-module
/// `pub(crate)` coupling beyond the feature gate.
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small schema-rich graph: a subclass hierarchy + typed entities + an object property with a
    // declared domain/range, plus a "wrong-typed" entity that type-constrained sampling must
    // exclude as a corruption.
    const TTL: &str = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex:   <http://ex/> .

ex:Dog   rdfs:subClassOf ex:Animal .
ex:Cat   rdfs:subClassOf ex:Animal .
ex:Owner rdfs:subClassOf ex:Person .

ex:owns rdfs:domain ex:Person ; rdfs:range ex:Animal .

ex:alice a ex:Owner .
ex:bob   a ex:Owner .
ex:rex   a ex:Dog .
ex:tom   a ex:Cat .
ex:car   a ex:Vehicle .

ex:alice ex:owns ex:rex .
ex:bob   ex:owns ex:tom .
"#;

    fn closed() -> ClosedGraph {
        close_for_vectorise(TTL, "turtle", Profile::Rdfs).unwrap()
    }

    #[test]
    fn closure_materialises_entailed_types() {
        let c = closed();
        // RDFS must add at least: rex a Animal, tom a Animal, alice a Person, bob a Person (rdfs9
        // via subClassOf), and the domain/range entailments (rdfs2/rdfs3).
        assert!(
            c.entailed_triples > 0,
            "closure added nothing: {}",
            c.entailed_triples
        );
        assert_eq!(c.closed_triples(), c.asserted_triples + c.entailed_triples);

        // The entailed `ex:rex a ex:Animal` must now be a real triple visible to the encoder.
        let rex = c.graph.id_of(&iri("http://ex/rex")).unwrap();
        let animal = c.graph.id_of(&iri("http://ex/Animal")).unwrap();
        let type_p = c.graph.id_of(&type_term()).unwrap();
        let has = c
            .graph
            .iter_ids()
            .any(|[s, p, o]| s == rex && p == type_p && o == animal);
        assert!(
            has,
            "closure-before-vectorise did not materialise ex:rex a ex:Animal"
        );
    }

    #[test]
    fn type_constraints_read_domain_and_range() {
        let c = closed();
        let tc = TypeConstraints::mine(&c.graph);
        let owns = c.graph.id_of(&iri("http://ex/owns")).unwrap();

        let alice = c.graph.id_of(&iri("http://ex/alice")).unwrap(); // Owner ⊑ Person
        let rex = c.graph.id_of(&iri("http://ex/rex")).unwrap(); // Dog ⊑ Animal
        let car = c.graph.id_of(&iri("http://ex/car")).unwrap(); // Vehicle — neither

        // Subject (domain = Person): a Person is admissible; an Animal/Vehicle is not.
        assert!(
            tc.admissible_subject(owns, alice),
            "Owner⊑Person must satisfy domain Person"
        );
        assert!(
            !tc.admissible_subject(owns, car),
            "Vehicle must not satisfy domain Person"
        );

        // Object (range = Animal): an Animal is admissible; a Person/Vehicle is not.
        assert!(
            tc.admissible_object(owns, rex),
            "Dog⊑Animal must satisfy range Animal"
        );
        assert!(
            !tc.admissible_object(owns, car),
            "Vehicle must not satisfy range Animal"
        );
        assert!(
            !tc.admissible_object(owns, alice),
            "Person must not satisfy range Animal"
        );
    }

    #[test]
    fn type_constrained_sampler_excludes_wrong_types() {
        let c = closed();
        let tc = TypeConstraints::mine(&c.graph);
        let owns = c.graph.id_of(&iri("http://ex/owns")).unwrap();
        let alice = c.graph.id_of(&iri("http://ex/alice")).unwrap();
        let rex = c.graph.id_of(&iri("http://ex/rex")).unwrap();
        let car = c.graph.id_of(&iri("http://ex/car")).unwrap();

        let sampler = NegativeSampler::new(&c.graph, &tc, SamplingMode::TypeConstrained);

        // Corrupt the TAIL of (alice owns rex): every emitted object must be range-admissible
        // (an Animal), so `ex:car` (Vehicle) can NEVER appear.
        let negs = sampler.sample([alice, owns, rex], Corrupt::Tail, 16, 42);
        assert!(
            !negs.is_empty(),
            "expected some type-valid tail corruptions"
        );
        for [h, r, t] in &negs {
            assert_eq!(*h, alice);
            assert_eq!(*r, owns);
            assert!(tc.admissible_object(owns, *t), "emitted non-range tail {t}");
            assert_ne!(
                *t, car,
                "Vehicle must never be a type-constrained range corruption"
            );
            assert_ne!(*t, rex, "must not reproduce the original tail");
        }
    }

    #[test]
    fn ablation_off_admits_wrong_types() {
        let c = closed();
        let tc = TypeConstraints::mine(&c.graph);
        let owns = c.graph.id_of(&iri("http://ex/owns")).unwrap();
        let alice = c.graph.id_of(&iri("http://ex/alice")).unwrap();
        let rex = c.graph.id_of(&iri("http://ex/rex")).unwrap();
        let car = c.graph.id_of(&iri("http://ex/car")).unwrap();

        // Unconstrained mode is the ablation OFF baseline: the type filter is bypassed, so the
        // wrong-typed `ex:car` is reachable as a tail corruption (it would never be under ON).
        let off = NegativeSampler::new(&c.graph, &tc, SamplingMode::Unconstrained);
        let mut saw_car = false;
        // Several seeds, since any single draw is random; the pool is tiny so this is reliable.
        for seed in 0..32u64 {
            let negs = off.sample([alice, owns, rex], Corrupt::Tail, 8, seed);
            if negs.iter().any(|[_, _, t]| *t == car) {
                saw_car = true;
                break;
            }
        }
        assert!(
            saw_car,
            "Unconstrained ablation arm must be able to emit the wrong-typed entity"
        );
        let _ = rex;
    }

    #[test]
    fn sampler_is_deterministic_for_fixed_seed() {
        let c = closed();
        let tc = TypeConstraints::mine(&c.graph);
        let owns = c.graph.id_of(&iri("http://ex/owns")).unwrap();
        let alice = c.graph.id_of(&iri("http://ex/alice")).unwrap();
        let rex = c.graph.id_of(&iri("http://ex/rex")).unwrap();
        let s = NegativeSampler::new(&c.graph, &tc, SamplingMode::TypeConstrained);
        let a = s.sample([alice, owns, rex], Corrupt::Tail, 4, 7);
        let b = s.sample([alice, owns, rex], Corrupt::Tail, 4, 7);
        assert_eq!(
            a, b,
            "fixed (seed, mode, triple) must reproduce identical negatives"
        );
    }

    #[test]
    fn sampler_never_reproduces_a_true_triple() {
        let c = closed();
        let tc = TypeConstraints::mine(&c.graph);
        let owns = c.graph.id_of(&iri("http://ex/owns")).unwrap();
        let alice = c.graph.id_of(&iri("http://ex/alice")).unwrap();
        let bob = c.graph.id_of(&iri("http://ex/bob")).unwrap();
        let rex = c.graph.id_of(&iri("http://ex/rex")).unwrap();
        let tom = c.graph.id_of(&iri("http://ex/tom")).unwrap();
        // (bob owns tom) is a true triple; corrupting (alice owns rex)'s head to `bob` together
        // with tail `tom` is not what `sample` does (it fixes the tail), but corrupting the tail of
        // (bob owns ?) must never re-emit `tom`.
        let s = NegativeSampler::new(&c.graph, &tc, SamplingMode::TypeConstrained);
        for seed in 0..32 {
            let negs = s.sample([bob, owns, tom], Corrupt::Tail, 8, seed);
            assert!(
                !negs.contains(&[bob, owns, tom]),
                "filtered guard must drop the true triple"
            );
        }
        let _ = (alice, rex);
    }

    #[test]
    fn unconstrained_predicate_is_fail_open() {
        // A predicate with no declared/observed range still samples (degrades to uniform), so a
        // missing schema never deadlocks the trainer.
        let ttl = r#"
@prefix ex: <http://ex/> .
ex:a ex:rel ex:b .
ex:b ex:rel ex:c .
ex:c ex:rel ex:a .
"#;
        let c = close_for_vectorise(ttl, "turtle", Profile::Rdfs).unwrap();
        let tc = TypeConstraints::mine(&c.graph);
        let rel = c.graph.id_of(&iri("http://ex/rel")).unwrap();
        let a = c.graph.id_of(&iri("http://ex/a")).unwrap();
        let b = c.graph.id_of(&iri("http://ex/b")).unwrap();
        let s = NegativeSampler::new(&c.graph, &tc, SamplingMode::TypeConstrained);
        let negs = s.sample([a, rel, b], Corrupt::Tail, 4, 1);
        assert!(
            !negs.is_empty(),
            "unconstrained predicate must still produce negatives"
        );
    }

    fn iri(s: &str) -> oxrdf::Term {
        oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
    }

    // ---- RDF 1.2 quoted-terms scope (TermScope / is_embeddable / sort-preserving sampling) ----

    /// An N-Triples fixture bearing RDF 1.2 quoted-triple terms (`rdf:reifies` objects), atomic
    /// triples, a blank-node subject, and a plain literal — one term of every sort.
    const RDF12_NT: &str = "\
<http://ex/alice> <http://ex/knows> <http://ex/bob> .\n\
<http://ex/bob> <http://ex/knows> <http://ex/carol> .\n\
<http://ex/carol> <http://ex/knows> <http://ex/alice> .\n\
<http://ex/stmt1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( <http://ex/alice> <http://ex/knows> <http://ex/bob> )>> .\n\
<http://ex/stmt2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( <http://ex/bob> <http://ex/knows> <http://ex/carol> )>> .\n\
<http://ex/stmt1> <http://ex/assertedBy> <http://ex/src1> .\n\
<http://ex/stmt2> <http://ex/assertedBy> <http://ex/src2> .\n\
<http://ex/alice> <http://ex/nick> \"al\" .\n\
_:b0 <http://ex/knows> <http://ex/alice> .\n";

    /// The quoted-triple-term ids of `graph` (objects whose parts are `TermParts::Triple`).
    fn quoted_term_ids(graph: &Graph) -> Vec<Id> {
        let mut ids: Vec<Id> = graph
            .iter_ids()
            .map(|[_, _, o]| o)
            .filter(|&o| matches!(graph.dict.term_parts(o), TermParts::Triple(_)))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    #[test]
    fn is_embeddable_scope_matrix() {
        let c = close_for_vectorise(RDF12_NT, "ntriples", Profile::Rdfs).unwrap();
        let g = &c.graph;

        // IRIs and blank nodes: embeddable under BOTH scopes (the pre-existing entity sorts).
        let alice = g.id_of(&iri("http://ex/alice")).unwrap();
        assert!(is_embeddable(g, alice, TermScope::IriBlank));
        assert!(is_embeddable(g, alice, TermScope::Embeddable));
        let blank = g
            .iter_ids()
            .map(|[s, _, _]| s)
            .find(|&s| matches!(g.dict.term_parts(s), TermParts::Blank(_)))
            .expect("fixture has a blank-node subject");
        assert!(is_embeddable(g, blank, TermScope::IriBlank));
        assert!(is_embeddable(g, blank, TermScope::Embeddable));

        // Quoted-triple terms: embeddable ONLY under `Embeddable` — the ablation switch.
        let quoted = quoted_term_ids(g);
        assert_eq!(quoted.len(), 2, "fixture has two distinct quoted terms");
        for &tt in &quoted {
            assert!(!is_embeddable(g, tt, TermScope::IriBlank));
            assert!(is_embeddable(g, tt, TermScope::Embeddable));
        }

        // Literals: excluded under BOTH scopes.
        let lit = g
            .iter_ids()
            .map(|[_, _, o]| o)
            .find(|&o| matches!(g.dict.term_parts(o), TermParts::Lit { .. }))
            .expect("fixture has a literal object");
        assert!(!is_embeddable(g, lit, TermScope::IriBlank));
        assert!(!is_embeddable(g, lit, TermScope::Embeddable));

        // The default scope IS the byte-stable baseline.
        assert_eq!(TermScope::default(), TermScope::IriBlank);
    }

    #[test]
    fn default_scope_sampler_is_blind_to_quoted_terms_and_matches_unscoped() {
        // On a quoted-term-BEARING graph, the unscoped `new` and the explicit `IriBlank` scope
        // must be the same sampler: an empty quoted pool and bit-identical draws (the structural
        // byte-stability guarantee: the OFF arm's PRNG stream is untouched by this change).
        let c = close_for_vectorise(RDF12_NT, "ntriples", Profile::Rdfs).unwrap();
        let g = &c.graph;
        let tc = TypeConstraints::mine(g);
        let unscoped = NegativeSampler::new(g, &tc, SamplingMode::Unconstrained);
        let off =
            NegativeSampler::new_scoped(g, &tc, SamplingMode::Unconstrained, TermScope::IriBlank);
        assert_eq!(unscoped.triple_term_count(), 0);
        assert_eq!(off.triple_term_count(), 0);
        assert_eq!(unscoped.entity_count(), off.entity_count());

        let alice = g.id_of(&iri("http://ex/alice")).unwrap();
        let knows = g.id_of(&iri("http://ex/knows")).unwrap();
        let bob = g.id_of(&iri("http://ex/bob")).unwrap();
        for side in [Corrupt::Head, Corrupt::Tail] {
            for seed in 0..8u64 {
                assert_eq!(
                    unscoped.sample([alice, knows, bob], side, 4, seed),
                    off.sample([alice, knows, bob], side, 4, seed),
                    "unscoped `new` must bit-reproduce the explicit IriBlank draws"
                );
            }
        }
    }

    #[test]
    fn embeddable_scope_corruption_is_sort_preserving() {
        let c = close_for_vectorise(RDF12_NT, "ntriples", Profile::Rdfs).unwrap();
        let g = &c.graph;
        let tc = TypeConstraints::mine(g);
        let s =
            NegativeSampler::new_scoped(g, &tc, SamplingMode::Unconstrained, TermScope::Embeddable);
        let quoted = quoted_term_ids(g);
        assert_eq!(
            s.triple_term_count(),
            quoted.len(),
            "quoted pool holds the quoted terms"
        );

        let stmt1 = g.id_of(&iri("http://ex/stmt1")).unwrap();
        let reifies = g
            .id_of(&iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies"))
            .unwrap();
        let tt1 = quoted
            .iter()
            .copied()
            .find(|&tt| {
                g.iter_ids()
                    .any(|[su, p, o]| su == stmt1 && p == reifies && o == tt)
            })
            .expect("stmt1 reifies a quoted term");
        let is_quoted = |id: Id| matches!(g.dict.term_parts(id), TermParts::Triple(_));

        // Corrupting the quoted-term TAIL slot must only ever emit quoted terms…
        let mut tail_negs = Vec::new();
        for seed in 0..16u64 {
            tail_negs.extend(s.sample([stmt1, reifies, tt1], Corrupt::Tail, 4, seed));
        }
        assert!(
            !tail_negs.is_empty(),
            "expected quoted-term tail corruptions"
        );
        for [h, r, t] in &tail_negs {
            assert_eq!((*h, *r), (stmt1, reifies));
            assert!(
                is_quoted(*t),
                "a quoted slot must never be corrupted to an atomic entity"
            );
            assert_ne!(*t, tt1, "must not reproduce the original quoted term");
        }

        // …and corrupting the atomic HEAD slot of the same positive only atomic entities.
        let mut head_negs = Vec::new();
        for seed in 0..16u64 {
            head_negs.extend(s.sample([stmt1, reifies, tt1], Corrupt::Head, 4, seed));
        }
        assert!(!head_negs.is_empty(), "expected atomic head corruptions");
        for [h, r, t] in &head_negs {
            assert_eq!((*r, *t), (reifies, tt1));
            assert!(
                !is_quoted(*h),
                "an atomic slot must never be corrupted to a quoted term"
            );
        }

        // Deterministic per (seed, scope).
        let a = s.sample([stmt1, reifies, tt1], Corrupt::Tail, 4, 9);
        let b = s.sample([stmt1, reifies, tt1], Corrupt::Tail, 4, 9);
        assert_eq!(
            a, b,
            "fixed (seed, scope, triple, side) must reproduce identical negatives"
        );
    }
}
