//! Flexible **minimal-and-complete grounding** selector + verbaliser (structure-aware
//! vectorisation **P4**).
//!
//! [OPUS-4.8] sq-0wo9e.5 (epic sq-0wo9e; design `research/structure-aware-vectorisation.md`
//! §4 "Flexible grounding — modality chosen per request, minimal AND complete"). This module is
//! the **fourth** additive slice of the structure-aware-vectorisation epic, on top of P0
//! ([`crate::structure`]: closure-before-vectorise + type-constrained negatives) and P1
//! ([`crate::encode`]: typed-literal encoders). It is **opt-in** (the same `structure` cargo
//! feature, off by default) and changes **nothing** in the default `sparq-vectors` build or the
//! core engine.
//!
//! # What this is
//!
//! Grounding is a function `(node, graph) -> minimal-and-complete OBJECT` whose **modality is
//! chosen per request** by a dispatcher on the *consumer's declared output type* — the same node
//! projected into whichever object the consumer needs:
//!
//! 1. [`Modality::Subgraph`] — the smallest connected sub-BGP describing the node, bounded to the
//!    predicates the node's **effective (minimal) type** actually carries (ABSTAT-style minimal
//!    type patterns; Spahiu et al., ESWC 2016). For an LLM tool that needs verifiable facts.
//! 2. [`Modality::TypedSubVector`] — only the *relevant blocks* of the node's stored vector
//!    (numeric for similar-magnitude, text for lexical), projected via the `.spqv`
//!    [`SchemaHeader`](crate::encode::SchemaHeader). For a vector tool. Minimal by construction.
//! 3. [`Modality::NlString`] — a token-budgeted natural-language rendering (the
//!    [`verbalize`](mod@crate::verbalize) machinery, extended here to render **typed values** —
//!    unit-typed quantities and enum labels). For an LLM.
//! 4. [`Modality::TypedValue`] — a single typed slot filled directly (the enum member, the
//!    unit-typed quantity, or the boolean), exact (no cosine threshold, no recall loss). For a
//!    typed tool-call argument.
//!
//! # Minimality and completeness — both PROFILE-RELATIVE (design §4, load-bearing)
//!
//! Both properties are stated **relative to a criterion**, never as absolutes:
//!
//! - **Minimality** is *relative to a stated criterion*: the subgraph is the smallest sub-BGP
//!   under the node's *effective type's characteristic set* (fewest predicates the type actually
//!   has — not every asserted triple); the typed sub-vector is the smallest projection that
//!   contains the requested blocks. It is **not** an absolute minimum.
//! - **Completeness** is *relative to a profile*: the caller materialises the deductive closure
//!   ([`crate::structure::close_for_vectorise`]) **before** grounding, so entailed-but-not-asserted
//!   facts are present; the grounding is then complete **relative to that materialised entailment
//!   profile (RDFS / OWL-RL / N3) and the declared shapes**. It is **NOT** end-task
//!   answer-completeness, and this module makes **no answer-completeness claim**. A node grounded
//!   over an un-closed graph is silently incomplete outside the asserted facts — the honest
//!   contract is "complete relative to whatever you closed under".
//!
//! **Ambiguous default = subgraph (exact, re-validated).** When the consumer's needs are unclear,
//! the design's default is subgraph grounding: it is built directly from the graph's own triples
//! (never an approximate signal), so it is exact and re-checkable. This module never serves an
//! approximate signal as a final answer — the ANN candidate seam is the `crate::filter` /
//! `vec:`-predicate path the *exact engine* re-evaluates, not this projector.
//!
//! # What is NOT here (honest scope)
//!
//! - **No ANN re-ranking.** This module *projects* an already-identified node into a modality. The
//!   "structure-aware ANN proposes candidates the exact engine re-validates" loop is the existing
//!   `filtered-ann` / `vec-predicate` path; P4 is the **projection** half, deliberately decoupled.
//! - **Quantities render AS DECLARED by default; opt-in cross-unit reconciliation (sq-t80n4).**
//!   The typed-value / NL paths recognise the QUDT `qudt:numericValue` + `qudt:unit` shape and
//!   render the magnitude *as declared* (value + unit label) by default — a grounding object stays
//!   faithful to the underlying triple. [`GroundingConfig::reconcile_units`] (default `false`) opts
//!   into **cross-unit reconciliation**: each known-unit quantity is converted to the **canonical
//!   SI unit of its [`crate::units::QuantityKind`]** via the landed P2 table
//!   ([`crate::units::normalise`], sq-0wo9e.3), so two quantities in **different but commensurable**
//!   units (e.g. `1 mi` vs `1.609344 km`, or `0 °C` vs `273.15 K`) collapse to the *same* canonical
//!   unit before grounding / comparison / NL render, instead of reading as distinct.
//!
//!   **Conservative on unknown (load-bearing soundness stance).** Reconciliation is applied *only*
//!   when [`crate::units::normalise`] resolves the unit in the bundled table. If the unit is absent
//!   — including every **compound / rate** unit such as `unit:KiloM-PER-HR` (km/h), which the
//!   simple-unit table does **not** carry — the quantity is left rendered in its **own** declared
//!   unit, never converted with a fabricated factor. Only same-[`crate::units::QuantityKind`] units
//!   are ever reconciled (a length is never "converted" to a mass): the table establishes
//!   commensurability or the as-declared fallback holds. No accuracy / embedding claim is made —
//!   this is a grounding-ergonomics + correctness feature; any downstream payoff is empirical.

use crate::encode::{numeric_value, route, temporal_value, Encoder};
use crate::store::VectorStore;
use crate::units::{normalise, QuantityKind, QUDT_UNIT_NS};
use crate::verbalize::{truncate_at_word_boundary, verbalize, EntityTextConfig};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{self, Id, TermParts};
use sparq_core::Graph;
use sparq_introspect::Introspection;

/// `rdf:type` — an entity's declared classes.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// `rdfs:subClassOf` — the subsumption edge used for effective-type minimalisation.
const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
/// `xsd:boolean` datatype IRI.
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
/// QUDT `qudt:numericValue` — the numeric magnitude of a quantity value.
const QUDT_NUMERIC_VALUE: &str = "http://qudt.org/schema/qudt/numericValue";
/// QUDT `qudt:value` — an alternative magnitude predicate seen in QUDT data.
const QUDT_VALUE: &str = "http://qudt.org/schema/qudt/value";
/// QUDT `qudt:unit` — the unit IRI of a quantity value.
const QUDT_UNIT: &str = "http://qudt.org/schema/qudt/unit";

/// The grounding **modality** — the kind of object the consumer's declared output type wants. A
/// dispatcher selects it ([`Modality::for_output`]); [`ground`] produces the matching
/// [`Grounding`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modality {
    /// The smallest connected sub-BGP describing the node (verifiable facts / exact answer).
    Subgraph,
    /// Only the relevant typed blocks of the node's stored vector (a vector tool).
    TypedSubVector,
    /// A token-budgeted NL rendering (an LLM consumer).
    NlString,
    /// A single typed slot filled directly: enum member / unit-typed quantity / boolean (a typed
    /// tool-call argument).
    TypedValue,
}

/// The consumer's **declared output type** — the dispatcher input. A tiny taxonomy of what a tool
/// slot expects, mapped to a [`Modality`] by [`Modality::for_output`] so callers express *intent*
/// ("I need a boolean") rather than picking the modality machinery by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputType {
    /// Verifiable facts / an exact answer → [`Modality::Subgraph`].
    Facts,
    /// A vector / typed sub-vector for a vector tool → [`Modality::TypedSubVector`].
    Vector,
    /// Free-form natural language for an LLM → [`Modality::NlString`].
    Text,
    /// A single typed value (boolean / number+unit / enum member) → [`Modality::TypedValue`].
    Value,
    /// The intent is unclear → the design's safe default, [`Modality::Subgraph`] (exact,
    /// re-validatable), because an approximate signal must never be the final answer.
    Ambiguous,
}

impl Modality {
    /// The design's dispatcher: map a consumer's declared [`OutputType`] to the modality. Ambiguous
    /// requests fall back to [`Modality::Subgraph`] (design §4 "Default for ambiguous NL queries").
    pub fn for_output(output: OutputType) -> Modality {
        match output {
            OutputType::Facts => Modality::Subgraph,
            OutputType::Vector => Modality::TypedSubVector,
            OutputType::Text => Modality::NlString,
            OutputType::Value => Modality::TypedValue,
            OutputType::Ambiguous => Modality::Subgraph,
        }
    }
}

/// A typed value extracted from one of a node's literal objects — the [`Grounding::TypedValue`]
/// payload. **Exact**: the value is the literal's own value (boolean / number / number+unit / enum
/// member), never an approximate cosine match.
#[derive(Clone, Debug, PartialEq)]
pub enum TypedValue {
    /// An `xsd:boolean` rendered to its two-valued truth.
    Boolean(bool),
    /// A numeric literal's value (any `xsd` numeric subtype), with the datatype IRI it carried.
    Number { value: f64, datatype: String },
    /// A unit-typed quantity: a magnitude plus the unit IRI it was declared with. **As declared**
    /// by default; under [`GroundingConfig::reconcile_units`] a *known* unit is reconciled to the
    /// canonical SI unit of its [`crate::units::QuantityKind`] (so commensurable quantities share a
    /// unit), while an unknown / compound unit is left as declared (the conservative fallback). Both
    /// `value` and `unit` reflect whichever was rendered.
    Quantity { value: f64, unit: String },
    /// An enum member — an IRI object rendered by its own label (the closed-world enum slot). The
    /// `member` is the IRI; `label` its rendered name when one was found.
    Enum {
        member: String,
        label: Option<String>,
    },
}

/// The minimal-and-complete grounding object — one variant per [`Modality`].
#[derive(Clone, Debug, PartialEq)]
pub enum Grounding {
    /// The smallest sub-BGP describing the node: `(predicate, object)` pairs in stable order, each
    /// a fact present in the (possibly closed) graph. The subject is the grounded node itself.
    Subgraph(Vec<GroundFact>),
    /// The relevant typed blocks of the node's stored vector, concatenated in block order, plus
    /// which encoder families they came from and each block's **fusion weight**.
    TypedSubVector {
        values: Vec<f32>,
        blocks: Vec<Encoder>,
        /// [OPUS-5] sq-w2af4 — the per-block query-time modality multiplier
        /// ([`Block::fusion_weight`](crate::encode::Block::fusion_weight)), one entry per kept
        /// block, in the same order as `blocks`. It is what the caller feeds to
        /// [`fuse_rrf_weighted`](crate::fuse_rrf_weighted) / [`fuse_scores`](crate::fuse_scores)
        /// so a low-provenance modality contributes less to the fused ranking. Every entry is
        /// `1.0` on a header with no recorded weights (fail-open — grounding is unchanged).
        ///
        /// **Which weight you get depends on how you grounded.** [`ground`] returns the BLOCK-level
        /// defaults read straight off the shared `SchemaHeader` — when it was produced by
        /// [`ProvenanceWeights::weight_header`](crate::provenance::ProvenanceWeights::weight_header)
        /// each entry is a graph-global mean over every subject asserting that block's feeding
        /// predicate, identical for every node grounded against that header. [`ground_weighted`]
        /// with a [`NodeWeighting`] instead computes each entry from **this node's own incident
        /// edges** ([`node_block_weight`](crate::provenance::ProvenanceWeights::node_block_weight)),
        /// which is the design's per-node modality scaling.
        weights: Vec<f32>,
    },
    /// The token-budgeted NL rendering.
    NlString(String),
    /// The typed values extracted for the requested predicate(s) — usually one, but a multi-valued
    /// predicate yields several. Empty when the node carries no such typed value.
    TypedValue(Vec<TypedValue>),
}

/// One fact in a [`Grounding::Subgraph`]: a `(predicate, object)` describing the grounded node.
/// Strings are rendered once (IRIs bare, literals lexical) so the object is a serialisable,
/// re-checkable fact rather than an opaque id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroundFact {
    /// The predicate IRI.
    pub predicate: String,
    /// The object rendered to text: an IRI bare, a literal as its lexical value.
    pub object: String,
    /// Whether the object was a literal (`true`) or an IRI/blank entity (`false`).
    pub object_is_literal: bool,
}

/// Configuration for [`ground`]: the minimality + completeness criteria and the budgets.
#[derive(Clone, Debug)]
pub struct GroundingConfig {
    /// Restrict the subgraph to the predicates the node's **effective (minimal) type's
    /// characteristic set** carries (ABSTAT-style minimality). When `false`, every asserted
    /// predicate of the node is included (still bounded by `max_facts`). Default `true`.
    pub minimal_type_pattern: bool,
    /// Cap the number of facts in a [`Grounding::Subgraph`] (a `k`-budget). `0` = unbounded.
    /// Default 64.
    pub max_facts: usize,
    /// The NL rendering template + char (token-proxy) budget for [`Modality::NlString`].
    pub text: EntityTextConfig,
    /// Append **typed-value** renderings to the [`Modality::NlString`] output — unit-typed
    /// quantities (`"300 km/h"`-style, value + unit label) and enum labels — after the base
    /// [`verbalize`] passage, within the `text.max_chars` budget (design §4.3 "extended to render
    /// unit-normalised quantities and enum labels"). The base verbaliser leaves raw numbers OUT of
    /// the embedded text on purpose; this flag is for an LLM-facing NL grounding where a typed slot
    /// IS the useful content, distinct from the text that gets embedded. Quantities honour
    /// [`GroundingConfig::reconcile_units`] (default as-declared). Default `false` (the base passage
    /// only).
    pub render_typed_values: bool,
    /// **Cross-unit reconciliation** for [`TypedValue::Quantity`] (sq-t80n4, consuming the landed P2
    /// table sq-0wo9e.3). When `true`, every quantity whose unit is **known** to
    /// [`crate::units::normalise`] is rendered in the **canonical SI unit of its
    /// [`crate::units::QuantityKind`]** (value converted, `unit` set to that canonical unit's IRI),
    /// so two quantities in **different but commensurable** units (e.g. `mi` vs `km`, `°C` vs `K`)
    /// reconcile to the same unit before grounding / comparison / NL render. **Conservative on
    /// unknown:** an unknown or **compound / rate** unit (e.g. `unit:KiloM-PER-HR`, not in the
    /// simple-unit table) is left **as declared**, never converted with a fabricated factor; only
    /// same-`QuantityKind` units are ever reconciled. Default `false` (render every quantity exactly
    /// as the triple declared it).
    pub reconcile_units: bool,
    /// For [`Modality::TypedSubVector`], which encoder families to keep. Empty = keep all blocks.
    /// Default empty (keep all).
    pub keep_blocks: Vec<Encoder>,
}

impl Default for GroundingConfig {
    fn default() -> Self {
        GroundingConfig {
            minimal_type_pattern: true,
            max_facts: 64,
            text: EntityTextConfig::default(),
            render_typed_values: false,
            reconcile_units: false,
            keep_blocks: Vec::new(),
        }
    }
}

/// Ground `node` of `graph` into the object the consumer's `modality` needs, under `cfg`.
///
/// **Completeness is profile-relative**: pass a graph whose closure was materialised
/// ([`crate::structure::close_for_vectorise`]) for completeness over that entailment profile; over
/// an un-closed graph the grounding is complete only over the asserted facts. **No
/// answer-completeness claim is made.**
///
/// `introspection` is the mined schema (`Introspection::build`) — used only by
/// [`Modality::Subgraph`] under `minimal_type_pattern` to find the node's effective-type
/// characteristic set; pass `None` to skip minimalisation (every asserted predicate is kept). The
/// other modalities ignore it.
///
/// `store` supplies the stored vector for [`Modality::TypedSubVector`] (with `header` describing
/// the blocks); pass `None` for the other modalities. Returns `None` when the node is absent /
/// inline, or when the requested modality has nothing to ground (e.g. `TypedSubVector` with no
/// stored vector, `TypedValue` with no typed object).
pub fn ground(
    graph: &Graph,
    node: &oxrdf::Term,
    modality: Modality,
    cfg: &GroundingConfig,
    introspection: Option<&Introspection>,
    store: Option<(&VectorStore, &crate::encode::SchemaHeader)>,
) -> Option<Grounding> {
    ground_weighted(graph, node, modality, cfg, introspection, store, None)
}

/// **Per-node modality weighting** for [`Modality::TypedSubVector`] (design §USE-1 integration
/// point 3): which predicate feeds each block of the [`SchemaHeader`](crate::encode::SchemaHeader),
/// and the mined provenance to score it with. [OPUS-5] sq-w2af4.
///
/// `block_predicates[i]` names the predicate feeding `header.blocks()[i]`; a `None` entry — or an
/// index past the end — leaves that block on the header's graph-global default (fail-open, e.g. a
/// text or taxonomy lane with no single feeding predicate).
pub struct NodeWeighting<'a> {
    /// Provenance mined from the same graph being grounded
    /// ([`ProvenanceWeights::mine`](crate::provenance::ProvenanceWeights::mine)).
    pub weights: &'a crate::provenance::ProvenanceWeights,
    /// The predicate feeding each header block, in block order.
    pub block_predicates: &'a [Option<&'a str>],
    /// The on/off ablation switch: [`WeightMode::Uniform`](crate::provenance::WeightMode::Uniform)
    /// leaves every weight at `1.0`.
    pub mode: crate::provenance::WeightMode,
}

/// [`ground`], with an optional [`NodeWeighting`] that computes the
/// [`Grounding::TypedSubVector`] block weights from **the grounded node's own incident edges** at
/// query time, instead of reading the shared header's graph-global defaults.
///
/// This is the design's §USE-1 point-3 behaviour: two nodes whose incident edges carry different
/// provenance receive different modality multipliers, each derived only from that node's own
/// contributions (see
/// [`node_block_weight`](crate::provenance::ProvenanceWeights::node_block_weight)). Pass `None` for
/// `weighting` — as [`ground`] does — to keep the persisted header defaults. Every other modality
/// ignores `weighting`.
pub fn ground_weighted(
    graph: &Graph,
    node: &oxrdf::Term,
    modality: Modality,
    cfg: &GroundingConfig,
    introspection: Option<&Introspection>,
    store: Option<(&VectorStore, &crate::encode::SchemaHeader)>,
    weighting: Option<&NodeWeighting<'_>>,
) -> Option<Grounding> {
    let id = graph.id_of(node)?;
    if dict::is_inline(id) {
        return None;
    }
    match modality {
        Modality::Subgraph => Some(Grounding::Subgraph(subgraph_facts(
            graph,
            id,
            cfg,
            introspection,
        ))),
        Modality::NlString => nl_string(graph, node, id, cfg).map(Grounding::NlString),
        Modality::TypedSubVector => {
            let (store, header) = store?;
            typed_sub_vector(graph, store, header, id, &cfg.keep_blocks, weighting).map(
                |(values, blocks, weights)| Grounding::TypedSubVector {
                    values,
                    blocks,
                    weights,
                },
            )
        }
        Modality::TypedValue => {
            let vals = typed_values(graph, id, cfg.reconcile_units);
            if vals.is_empty() {
                None
            } else {
                Some(Grounding::TypedValue(vals))
            }
        }
    }
}

// ---- NL-string modality -----------------------------------------------------------------------

/// The token-budgeted NL grounding: the base [`verbalize`] passage, optionally extended with
/// **typed-value** clauses (unit-typed quantities + enum labels) when
/// [`GroundingConfig::render_typed_values`] is set. The whole result is held within
/// `cfg.text.max_chars`. `None` when the node has no usable text at all.
fn nl_string(graph: &Graph, node: &oxrdf::Term, id: Id, cfg: &GroundingConfig) -> Option<String> {
    let base = verbalize(graph, node, &cfg.text);
    if !cfg.render_typed_values {
        return base;
    }
    let typed = typed_value_clauses(graph, id, cfg.reconcile_units);
    match (base, typed.is_empty()) {
        (base, true) => base,
        (None, false) => {
            // No base passage (e.g. a node with no label/description) but it carries typed values
            // — still ground it from those, within budget.
            Some(truncate_chars(&typed.join(". "), cfg.text.max_chars))
        }
        (Some(b), false) => {
            let extra = typed.join(". ");
            let sep = &cfg.text.separator;
            let joined = format!("{b}{sep}{extra}");
            Some(truncate_chars(&joined, cfg.text.max_chars))
        }
    }
}

/// Render each typed value to a short NL clause: `"colour: Red"`, `"top speed: 300 unit:KiloM-PER-HR"`,
/// `"electric: true"`. Deterministic order (the [`typed_values`] order). Unit quantities are rendered
/// as declared unless `reconcile` is set (then a *known* unit is reconciled to its canonical SI unit;
/// an unknown / compound unit stays as declared) — the unit label is the rendered unit IRI's local
/// name.
fn typed_value_clauses(graph: &Graph, id: Id, reconcile: bool) -> Vec<String> {
    typed_values(graph, id, reconcile)
        .into_iter()
        .map(|v| match v {
            TypedValue::Boolean(b) => b.to_string(),
            TypedValue::Number { value, .. } => format_number(value),
            TypedValue::Quantity { value, unit } => {
                format!("{} {}", format_number(value), local_name(&unit))
            }
            TypedValue::Enum { member, label } => label.unwrap_or_else(|| local_name(&member)),
        })
        .collect()
}

/// Render an `f64` value without a trailing `.0` for whole numbers (so `2.0` → `"2"`, `2.5` → `"2.5"`).
fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

/// The local name of an IRI (the part after the last `#` or `/`), for a human-readable unit /
/// member label when no `rdfs:label` was found.
fn local_name(iri: &str) -> String {
    let local = iri.rsplit(['#', '/']).next().unwrap_or(iri);
    if local.is_empty() {
        iri.to_string()
    } else {
        local.to_string()
    }
}

/// Truncate `s` at a word or sentence boundary to at most `max` chars (the same budget discipline
/// as [`verbalize`]).
fn truncate_chars(s: &str, max: usize) -> String {
    // [GPT-5.6] Share the exact boundary policy with entity verbalization.
    truncate_at_word_boundary(s, max)
}

// ---- subgraph modality ------------------------------------------------------------------------

/// The smallest sub-BGP describing `id`: its `(predicate, object)` facts, optionally restricted to
/// the predicates of `id`'s effective-type characteristic set (ABSTAT-style minimality), in stable
/// (predicate IRI, object text) order, capped at `cfg.max_facts`.
fn subgraph_facts(
    graph: &Graph,
    id: Id,
    cfg: &GroundingConfig,
    introspection: Option<&Introspection>,
) -> Vec<GroundFact> {
    // The minimal predicate allow-list, when minimalisation is requested AND an introspection is
    // available AND the node's effective type matched a characteristic set. None ⇒ keep all.
    let allow: Option<FxHashSet<String>> = if cfg.minimal_type_pattern {
        introspection.and_then(|i| minimal_predicate_set(graph, id, i))
    } else {
        None
    };

    let mut facts: Vec<GroundFact> = Vec::new();
    // One contiguous SPO range over the node's own triples (the node is the subject).
    let scan = graph.store.scan(&[Some(id), None, None]);
    for row in scan.rows.iter() {
        let [_, p, o] = scan.to_spo(row);
        let Some(predicate) = render_iri(graph, p) else {
            continue;
        };
        if let Some(allowset) = &allow {
            if !allowset.contains(&predicate) {
                continue;
            }
        }
        let (object, object_is_literal) = render_object(graph, o);
        facts.push(GroundFact {
            predicate,
            object,
            object_is_literal,
        });
    }
    // Stable, deterministic order independent of dictionary-id layout.
    facts.sort_by(|a, b| a.predicate.cmp(&b.predicate).then(a.object.cmp(&b.object)));
    facts.dedup();
    if cfg.max_facts != 0 && facts.len() > cfg.max_facts {
        facts.truncate(cfg.max_facts);
    }
    facts
}

/// The predicate allow-list for `id` under ABSTAT-style minimality: find `id`'s **most-specific
/// (minimal) type(s)**, then the introspection characteristic set whose declared `classes` best
/// match — its `predicates` are the minimal pattern. `None` when the node has no type, no matching
/// characteristic set, or the introspection carries no usable set (so the caller keeps all facts —
/// the honest fail-open behaviour: minimalisation only *narrows* when it can prove a type pattern).
fn minimal_predicate_set(
    graph: &Graph,
    id: Id,
    introspection: &Introspection,
) -> Option<FxHashSet<String>> {
    let minimal_types = minimal_types_of(graph, id);
    if minimal_types.is_empty() {
        return None;
    }
    // Pick the characteristic set whose `classes` overlap the node's minimal types the most (ties
    // broken toward the larger subject count — the dominant pattern for that type). The set's
    // predicate list is the minimal pattern.
    let mut best: Option<(usize, u64, &[String])> = None;
    for cs in &introspection.characteristic_sets.sets {
        let overlap = cs
            .classes
            .iter()
            .filter(|c| minimal_types.contains(&c.iri))
            .count();
        if overlap == 0 {
            continue;
        }
        let cand = (overlap, cs.subjects, cs.predicates.as_slice());
        if best
            .as_ref()
            .is_none_or(|(bo, bs, _)| (overlap, cs.subjects) > (*bo, *bs))
        {
            best = Some(cand);
        }
    }
    let preds = best?.2;
    // `rdf:type` is always part of a useful minimal pattern (it carries the type the pattern is
    // keyed on), so include it even if a CS omits it.
    let mut set: FxHashSet<String> = preds.iter().cloned().collect();
    set.insert(RDF_TYPE.to_string());
    Some(set)
}

/// The most-specific (minimal) `rdf:type` class IRIs of `id`: its declared types, minus any that
/// are a proper superclass of another declared type via `rdfs:subClassOf` in `graph`. Mirrors the
/// ABSTAT minimalisation [`sparq_introspect`] does, but scoped to one node so the caller need not
/// rebuild a minimalised introspection.
fn minimal_types_of(graph: &Graph, id: Id) -> FxHashSet<String> {
    let Some(type_pid) = graph.id_of(&named(RDF_TYPE)) else {
        return FxHashSet::default();
    };
    let mut type_ids: Vec<Id> = Vec::new();
    let scan = graph.store.scan(&[Some(id), Some(type_pid), None]);
    for row in scan.rows.iter() {
        let [_, _, o] = scan.to_spo(row);
        if !dict::is_inline(o) && matches!(graph.dict.term_parts(o), TermParts::Iri { .. }) {
            type_ids.push(o);
        }
    }
    if type_ids.is_empty() {
        return FxHashSet::default();
    }
    // Build the (subclass -> superclasses) closure-by-one-step map only over the node's own types,
    // then drop any type that is a proper superclass of another of the node's types.
    let supers = superclasses_of(graph, &type_ids);
    let mut minimal: FxHashSet<String> = FxHashSet::default();
    for &t in &type_ids {
        let is_proper_super = type_ids
            .iter()
            .any(|&other| other != t && supers.get(&other).is_some_and(|s| s.contains(&t)));
        if !is_proper_super {
            if let Some(iri) = render_iri(graph, t) {
                minimal.insert(iri);
            }
        }
    }
    minimal
}

/// For each class id in `classes`, its set of (transitive) superclass ids via `rdfs:subClassOf` in
/// `graph`. Transitive so a multi-step chain `A ⊑ B ⊑ C` makes both `B` and `C` superclasses of
/// `A`. Bounded to avoid a cycle spinning (a malformed `subClassOf` cycle terminates).
fn superclasses_of(graph: &Graph, classes: &[Id]) -> FxHashMap<Id, FxHashSet<Id>> {
    let mut out: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default();
    let Some(sco_pid) = graph.id_of(&named(RDFS_SUBCLASS_OF)) else {
        return out;
    };
    for &c in classes {
        let mut acc: FxHashSet<Id> = FxHashSet::default();
        let mut frontier = vec![c];
        // Bounded BFS over subClassOf; the visited guard makes a cycle terminate.
        while let Some(cur) = frontier.pop() {
            let scan = graph.store.scan(&[Some(cur), Some(sco_pid), None]);
            for row in scan.rows.iter() {
                let [_, _, sup] = scan.to_spo(row);
                if sup != cur && acc.insert(sup) {
                    frontier.push(sup);
                }
            }
        }
        out.insert(c, acc);
    }
    out
}

// ---- typed-value modality ---------------------------------------------------------------------

/// Every typed value `id` carries: booleans, numbers, unit-typed quantities (the QUDT
/// `qudt:numericValue`/`qudt:value` + `qudt:unit` shape on a directly-attached quantity-value
/// node), and enum members (IRI objects of predicates whose object is an entity). Deterministic
/// order (predicate IRI, then value). **Exact** — each value is the literal's own value.
///
/// When `reconcile` is set, each [`TypedValue::Quantity`] whose unit the bundled table knows is
/// converted to the canonical SI unit of its [`QuantityKind`] (see [`reconcile_quantity`]); an
/// unknown / compound unit is left as declared (the conservative fallback). The deterministic
/// ordering key is computed *after* reconciliation, so two commensurable quantities sort identically.
fn typed_values(graph: &Graph, id: Id, reconcile: bool) -> Vec<TypedValue> {
    let mut out: Vec<(String, TypedValue)> = Vec::new();
    let scan = graph.store.scan(&[Some(id), None, None]);
    for row in scan.rows.iter() {
        let [_, p, o] = scan.to_spo(row);
        let Some(pred) = render_iri(graph, p) else {
            continue;
        };
        // A unit-typed quantity: the object is a blank/IRI quantity-value node with a magnitude +
        // a unit. Recognised before the literal path so a QUDT structured value renders as a
        // Quantity, not as the raw blank node. Reconciled to canonical SI units only when asked AND
        // the unit is known — otherwise rendered exactly as declared (conservative on unknown).
        if let Some(q) = quantity_value(graph, o) {
            out.push((pred, if reconcile { reconcile_quantity(q) } else { q }));
            continue;
        }
        match literal_typed_value(graph, o) {
            Some(v) => out.push((pred, v)),
            None => {
                // An IRI object → an enum member (the closed-world enum slot). Only entities, not
                // structured quantity nodes already handled above.
                if !dict::is_inline(o) {
                    if let TermParts::Iri { .. } = graph.dict.term_parts(o) {
                        let member = render_iri(graph, o).unwrap_or_default();
                        let label = enum_label(graph, o);
                        out.push((pred, TypedValue::Enum { member, label }));
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(typed_value_key(&a.1).cmp(&typed_value_key(&b.1)))
    });
    out.into_iter().map(|(_, v)| v).collect()
}

/// A boolean / numeric literal object as a [`TypedValue`], or `None` for a non-literal / a string /
/// an ill-formed numeric.
fn literal_typed_value(graph: &Graph, o: Id) -> Option<TypedValue> {
    if dict::is_inline(o) {
        // Inline-integer ids decode to an `xsd:integer` literal.
        let oxrdf::Term::Literal(l) = graph.dict.term(o) else {
            return None;
        };
        let value = numeric_value(l.value())?;
        return Some(TypedValue::Number {
            value,
            datatype: l.datatype().as_str().to_string(),
        });
    }
    let TermParts::Lit {
        value,
        datatype,
        lang,
    } = graph.dict.term_parts(o)
    else {
        return None;
    };
    if lang.is_some() {
        return None; // a language-tagged string is text, not a typed value
    }
    if datatype == XSD_BOOLEAN {
        return match value.trim() {
            "true" | "1" => Some(TypedValue::Boolean(true)),
            "false" | "0" => Some(TypedValue::Boolean(false)),
            _ => None,
        };
    }
    match route(datatype) {
        Encoder::Numeric => numeric_value(value).map(|v| TypedValue::Number {
            value: v,
            datatype: datatype.to_string(),
        }),
        Encoder::Date => temporal_value(value, datatype).map(|v| TypedValue::Number {
            value: v,
            datatype: datatype.to_string(),
        }),
        _ => None,
    }
}

/// Recognise a directly-attached **QUDT quantity value**: `o` is a blank/IRI node carrying a
/// magnitude (`qudt:numericValue` or `qudt:value`) and a `qudt:unit`. Returns the
/// [`TypedValue::Quantity`] **as declared** — the value is not converted between units here; the
/// optional cross-unit reconciliation is [`reconcile_quantity`], applied by [`typed_values`] only
/// under [`GroundingConfig::reconcile_units`]. `None` when `o` is not such a node.
fn quantity_value(graph: &Graph, o: Id) -> Option<TypedValue> {
    if dict::is_inline(o) {
        return None;
    }
    if !matches!(
        graph.dict.term_parts(o),
        TermParts::Iri { .. } | TermParts::Blank(_)
    ) {
        return None;
    }
    let value = first_numeric(graph, o, QUDT_NUMERIC_VALUE)
        .or_else(|| first_numeric(graph, o, QUDT_VALUE))?;
    let unit = first_iri_object(graph, o, QUDT_UNIT)?;
    Some(TypedValue::Quantity { value, unit })
}

/// Reconcile a [`TypedValue::Quantity`] to the **canonical SI unit of its [`QuantityKind`]** via the
/// landed P2 table ([`crate::units::normalise`], sq-0wo9e.3), so two quantities in **different but
/// commensurable** units (e.g. `1 mi` and `1.609344 km`, both `Length`; or `0 °C` and `273.15 K`,
/// both `Temperature`) reconcile to the same `(value, unit)` and are no longer read as distinct.
///
/// **Soundness — conservative on unknown.** Reconciliation happens *only* when
/// [`crate::units::normalise`] resolves the declared unit in the bundled table; the result then
/// carries the canonical SI value and that kind's canonical-unit IRI. If the unit is **absent** —
/// any unit the curated simple-unit table does not list, **including every compound / rate unit**
/// such as `unit:KiloM-PER-HR` (km/h) — the input is returned **unchanged** (rendered in its own
/// declared unit), never converted with a fabricated factor. Because [`crate::units::normalise`]
/// keys conversion on a single [`QuantityKind`], commensurability is established by the table or the
/// as-declared fallback holds — a length is never "converted" to a mass. A non-[`TypedValue::Quantity`]
/// is returned unchanged. This is exposed so a caller can reconcile a quantity it extracted directly
/// (the same primitive the grounding render path uses under [`GroundingConfig::reconcile_units`]).
pub fn reconcile_quantity(v: TypedValue) -> TypedValue {
    let TypedValue::Quantity { value, unit } = &v else {
        return v;
    };
    match normalise(*value, unit) {
        Some(n) => TypedValue::Quantity {
            value: n.canonical_value,
            unit: canonical_unit_iri(n.kind),
        },
        // Unknown / compound unit, or a non-finite magnitude — leave exactly as declared.
        None => v,
    }
}

/// The full QUDT IRI of a [`QuantityKind`]'s canonical SI unit (e.g. `Length` → `…/unit/M`), so a
/// reconciled quantity carries a resolvable unit IRI rather than a bare local name.
fn canonical_unit_iri(kind: QuantityKind) -> String {
    format!("{}{}", QUDT_UNIT_NS, kind.canonical_unit())
}

/// The first numeric object of `(subject, predicate)`, parsed to `f64`, or `None`.
fn first_numeric(graph: &Graph, subject: Id, predicate: &str) -> Option<f64> {
    let pid = graph.id_of(&named(predicate))?;
    let scan = graph.store.scan(&[Some(subject), Some(pid), None]);
    for row in scan.rows.iter() {
        let [_, _, o] = scan.to_spo(row);
        if dict::is_inline(o) {
            if let oxrdf::Term::Literal(l) = graph.dict.term(o) {
                if let Some(v) = numeric_value(l.value()) {
                    return Some(v);
                }
            }
            continue;
        }
        if let TermParts::Lit { value, .. } = graph.dict.term_parts(o) {
            if let Some(v) = numeric_value(value) {
                return Some(v);
            }
        }
    }
    None
}

/// The first IRI object of `(subject, predicate)` rendered to text, or `None`.
fn first_iri_object(graph: &Graph, subject: Id, predicate: &str) -> Option<String> {
    let pid = graph.id_of(&named(predicate))?;
    let scan = graph.store.scan(&[Some(subject), Some(pid), None]);
    for row in scan.rows.iter() {
        let [_, _, o] = scan.to_spo(row);
        if !dict::is_inline(o) && matches!(graph.dict.term_parts(o), TermParts::Iri { .. }) {
            return render_iri(graph, o);
        }
    }
    None
}

/// A stable, comparable key for a [`TypedValue`] so [`typed_values`] is deterministic across
/// dictionary-id layouts.
fn typed_value_key(v: &TypedValue) -> String {
    match v {
        TypedValue::Boolean(b) => format!("0bool:{}", b),
        TypedValue::Number { value, datatype } => format!("1num:{}:{}", datatype, value),
        TypedValue::Quantity { value, unit } => format!("2qty:{}:{}", unit, value),
        TypedValue::Enum { member, .. } => format!("3enum:{}", member),
    }
}

// ---- typed-sub-vector modality ----------------------------------------------------------------

/// Project `id`'s stored vector to only the blocks in `keep` (empty = all), concatenated in block
/// order, plus the encoder families kept and each kept block's modality weight ([OPUS-5] sq-w2af4).
/// The weight is [`node_block_weight`](crate::provenance::ProvenanceWeights::node_block_weight) —
/// mined from **`id`'s own** incident edges — when `weighting` supplies a predicate for that block,
/// and otherwise the header's graph-global
/// [`fusion_weight`](crate::encode::Block::fusion_weight) default (`1.0` when the header records
/// none). `None` when the store has no vector for `id`, or the kept projection is empty.
fn typed_sub_vector(
    graph: &Graph,
    store: &VectorStore,
    header: &crate::encode::SchemaHeader,
    id: Id,
    keep: &[Encoder],
    weighting: Option<&NodeWeighting<'_>>,
) -> Option<(Vec<f32>, Vec<Encoder>, Vec<f32>)> {
    let full = store.get(id)?;
    let mut values: Vec<f32> = Vec::new();
    let mut blocks: Vec<Encoder> = Vec::new();
    let mut weights: Vec<f32> = Vec::new();
    for (i, block) in header.blocks().iter().enumerate() {
        if !keep.is_empty() && !keep.contains(&block.encoder) {
            continue;
        }
        let end = block.offset.saturating_add(block.width);
        if end > full.len() {
            continue; // header/store mismatch on this block — skip rather than panic
        }
        values.extend_from_slice(&full[block.offset..end]);
        blocks.push(block.encoder);
        // Per-node when the caller supplied a feeding predicate for this block; the header's
        // graph-global default otherwise (a short/absent `block_predicates` fails open, never
        // panics on a caller/layout mismatch).
        weights.push(match weighting {
            Some(w) => match w.block_predicates.get(i).copied().flatten() {
                Some(pred) => w.weights.node_block_weight(graph, id, pred, w.mode),
                None => block.fusion_weight(),
            },
            None => block.fusion_weight(),
        });
    }
    if values.is_empty() {
        None
    } else {
        Some((values, blocks, weights))
    }
}

// ---- integration point 2 (WIRING): assertion-weighted structural-sketch pooling ----------------
// [OPUS-5] sq-w2af4. sq-oy9ya landed `ProvenanceWeights::pool_weighted` but no in-tree caller ever
// pooled anything with it. `sketch_predicate` is that caller: it is the grounding-path
// structural-sketch pooler for a node's MULTI-VALUED object predicate.

/// **Assertion-weighted structural sketch** of `node`'s multi-valued `predicate` (design §USE-1
/// integration point 2): pool the stored vectors of the node's object neighbours under that
/// predicate, weighting each contribution by **the `(node, predicate, o)` assertion's own** `w(t)`
/// via [`ProvenanceWeights::pool_weighted`](crate::provenance::ProvenanceWeights::pool_weighted).
///
/// **What the weight actually measures (load-bearing — read this before trusting it).** Each
/// contribution is keyed on the *triple*, so the weight is the reified statement's provenance where
/// the graph carries it (RDF 1.2 `rdf:reifies`, or RDF 1.1 `rdf:subject`/`rdf:predicate`/
/// `rdf:object` — see [`ProvenanceWeights::weight_of`](crate::provenance::ProvenanceWeights::weight_of))
/// and the head's otherwise. Two consequences, stated plainly rather than glossed:
/// - On a graph with **no statement-level provenance** every `(node, predicate, ·)` contribution
///   falls back to the same head weight, so the pool is **exactly** the arithmetic mean — this axis
///   is an honest no-op there rather than a substituted proxy.
///   [`annotated_statements`](crate::provenance::ProvenanceWeights::annotated_statements) reports
///   whether the signal exists on your graph; check it before calling a sketch provenance-driven.
/// - It is deliberately **not** keyed on the neighbour `o`. Provenance about `o` says `o` is a
///   low-assurance *entity*, which is a different heuristic from "this assertion is doubtful";
///   substituting it would report the design point as implemented when it is not.
///
/// Returns `Ok(None)` when the node is absent/inline, the predicate is unknown to the graph, or no
/// neighbour has a stored vector. `Err` only if the store's vectors disagree in length (a layout
/// bug, never silently truncated).
///
/// Under [`WeightMode::Uniform`](crate::provenance::WeightMode::Uniform) — and over any
/// provenance-free graph — this is **exactly** the arithmetic mean of the neighbour vectors, i.e.
/// the ablation-off baseline. **No accuracy claim is made**: the sketch is measured, not asserted
/// (`eval::run_pooling_ablation`, under the `kge` feature, is the instrument).
pub fn sketch_predicate(
    graph: &Graph,
    store: &VectorStore,
    node: &oxrdf::Term,
    predicate: &str,
    weights: &crate::provenance::ProvenanceWeights,
    mode: crate::provenance::WeightMode,
) -> Result<Option<Vec<f32>>, String> {
    let Some(id) = graph.id_of(node) else {
        return Ok(None);
    };
    if dict::is_inline(id) {
        return Ok(None);
    }
    let Some(pid) = graph.id_of(&named(predicate)) else {
        return Ok(None);
    };
    let scan = graph.store.scan(&[Some(id), Some(pid), None]);
    let mut contributions: Vec<([Id; 3], Vec<f32>)> = Vec::new();
    for row in scan.rows.iter() {
        let spo = scan.to_spo(row);
        if let Some(v) = store.get(spo[2]) {
            contributions.push((spo, v.to_vec()));
        }
    }
    weights.pool_weighted(&contributions, mode)
}

// ---- shared helpers ---------------------------------------------------------------------------

/// An IRI named-node term.
fn named(iri: &str) -> oxrdf::Term {
    oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(iri))
}

/// Render an id that should be an IRI to its full IRI string, or `None` if it is not an IRI.
fn render_iri(graph: &Graph, id: Id) -> Option<String> {
    if dict::is_inline(id) {
        return None;
    }
    match graph.dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => Some(format!("{}{}", prefix, suffix)),
        _ => None,
    }
}

/// Render an object id to `(text, is_literal)`: an IRI bare, a literal as its lexical value, a
/// blank node as `_:label`, an RDF 1.2 quoted triple term in its `<<( s p o )>>` term syntax.
/// Inline integers decode to their lexical value.
fn render_object(graph: &Graph, id: Id) -> (String, bool) {
    if dict::is_inline(id) {
        if let oxrdf::Term::Literal(l) = graph.dict.term(id) {
            return (l.value().to_string(), true);
        }
    }
    match graph.dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => (format!("{}{}", prefix, suffix), false),
        TermParts::Lit { value, .. } => (value.to_string(), true),
        TermParts::Blank(b) => (format!("_:{}", b), false),
        // BUGFIX [FABLE-5]: this arm returned the EMPTY STRING, silently corrupting every
        // NL-string / subgraph-text grounding whose object is an RDF 1.2 quoted triple. The dict
        // reconstructs the (nested, depth-capped) term and oxrdf's `Display` renders the RDF 1.2
        // `<<( s p o )>>` triple-term syntax.
        TermParts::Triple(_) => (graph.dict.term(id).to_string(), false),
    }
}

/// The label of an enum-member IRI: the first `rdfs:label` / `skos:prefLabel` literal, else the
/// IRI's local name.
fn enum_label(graph: &Graph, id: Id) -> Option<String> {
    for pred in [
        "http://www.w3.org/2000/01/rdf-schema#label",
        "http://www.w3.org/2004/02/skos/core#prefLabel",
    ] {
        if let Some(pid) = graph.id_of(&named(pred)) {
            let scan = graph.store.scan(&[Some(id), Some(pid), None]);
            for row in scan.rows.iter() {
                let [_, _, o] = scan.to_spo(row);
                if let TermParts::Lit { value, .. } = graph.dict.term_parts(o) {
                    let v = value.trim();
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    // Fall back to the IRI's local name.
    let iri = render_iri(graph, id)?;
    let local = iri.rsplit(['#', '/']).next().unwrap_or(&iri);
    (!local.is_empty()).then(|| local.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::close_for_vectorise;
    use sparq_reason::Profile;

    fn iri(s: &str) -> oxrdf::Term {
        oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
    }

    // A schema-rich graph: a subclass hierarchy, a typed entity with enum + numeric + boolean +
    // quantity properties, plus the labels enum members and types carry.
    const TTL: &str = r#"
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix qudt: <http://qudt.org/schema/qudt/> .
@prefix unit: <http://qudt.org/vocab/unit/> .
@prefix ex:   <http://ex/> .

ex:SportsCar rdfs:subClassOf ex:Car .
ex:Car       rdfs:subClassOf ex:Vehicle .

ex:Red   rdfs:label "Red" .
ex:Green rdfs:label "Green" .

ex:bolt a ex:SportsCar, ex:Car ;
        rdfs:label "Bolt" ;
        ex:seats 2 ;
        ex:electric true ;
        ex:colour ex:Red ;
        ex:topSpeed ex:bolt-speed .

ex:bolt-speed qudt:numericValue "300"^^xsd:decimal ;
              qudt:unit unit:KiloM-PER-HR .

ex:other a ex:Car ;
         rdfs:label "Other" ;
         ex:seats 4 ;
         ex:electric false .
"#;

    fn closed() -> sparq_core::Graph {
        close_for_vectorise(TTL, "turtle", Profile::Rdfs)
            .unwrap()
            .graph
    }

    #[test]
    fn dispatcher_maps_output_type_to_modality() {
        assert_eq!(Modality::for_output(OutputType::Facts), Modality::Subgraph);
        assert_eq!(
            Modality::for_output(OutputType::Vector),
            Modality::TypedSubVector
        );
        assert_eq!(Modality::for_output(OutputType::Text), Modality::NlString);
        assert_eq!(
            Modality::for_output(OutputType::Value),
            Modality::TypedValue
        );
        // The load-bearing default: an ambiguous request grounds as the EXACT subgraph.
        assert_eq!(
            Modality::for_output(OutputType::Ambiguous),
            Modality::Subgraph
        );
    }

    #[test]
    fn subgraph_grounds_to_facts_present_in_the_graph() {
        let g = closed();
        let cfg = GroundingConfig {
            minimal_type_pattern: false,
            ..Default::default()
        };
        let Some(Grounding::Subgraph(facts)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::Subgraph,
            &cfg,
            None,
            None,
        ) else {
            panic!("expected a subgraph grounding");
        };
        // Every fact must be a real triple of the graph (re-checkable, never approximate).
        let bolt = g.id_of(&iri("http://ex/bolt")).unwrap();
        for f in &facts {
            let pid = g.id_of(&named(&f.predicate)).expect("predicate in graph");
            let scan = g.store.scan(&[Some(bolt), Some(pid), None]);
            let hit = scan.rows.iter().any(|row| {
                let [_, _, o] = scan.to_spo(row);
                let (txt, _) = render_object(&g, o);
                txt == f.object
            });
            assert!(hit, "grounded fact not present in graph: {:?}", f);
        }
        // The closure made `ex:bolt a ex:Vehicle` a real fact → completeness over the RDFS profile.
        assert!(
            facts
                .iter()
                .any(|f| f.predicate == RDF_TYPE && f.object == "http://ex/Vehicle"),
            "closure-before-grounding should surface the entailed Vehicle type"
        );
    }

    #[test]
    fn minimal_type_pattern_narrows_the_subgraph() {
        let g = closed();
        let introspection = Introspection::build(&g);
        let full_cfg = GroundingConfig {
            minimal_type_pattern: false,
            ..Default::default()
        };
        let min_cfg = GroundingConfig {
            minimal_type_pattern: true,
            ..Default::default()
        };

        let Some(Grounding::Subgraph(full)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::Subgraph,
            &full_cfg,
            Some(&introspection),
            None,
        ) else {
            panic!()
        };
        let Some(Grounding::Subgraph(min)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::Subgraph,
            &min_cfg,
            Some(&introspection),
            None,
        ) else {
            panic!()
        };
        // Minimality is relative to a criterion: the minimal pattern is a SUBSET of the full set,
        // and never larger. (It may equal it when the CS already covers every predicate.)
        let min_preds: FxHashSet<&String> = min.iter().map(|f| &f.predicate).collect();
        let full_preds: FxHashSet<&String> = full.iter().map(|f| &f.predicate).collect();
        assert!(
            min_preds.is_subset(&full_preds),
            "minimal pattern must be a subset of all facts"
        );
    }

    #[test]
    fn typed_value_extracts_boolean_number_quantity_enum() {
        let g = closed();
        let Some(Grounding::TypedValue(vals)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::TypedValue,
            &GroundingConfig::default(),
            None,
            None,
        ) else {
            panic!("expected typed values");
        };
        // Boolean (electric true) — exact, two-valued.
        assert!(
            vals.contains(&TypedValue::Boolean(true)),
            "missing boolean: {:?}",
            vals
        );
        // Number (seats 2) — exact.
        assert!(
            vals.iter()
                .any(|v| matches!(v, TypedValue::Number { value, .. } if *value == 2.0)),
            "missing seats number: {:?}",
            vals
        );
        // Unit-typed quantity rendered AS DECLARED (300 km/h, not converted).
        assert!(
            vals.iter()
                .any(|v| matches!(v, TypedValue::Quantity { value, unit }
                if *value == 300.0 && unit == "http://qudt.org/vocab/unit/KiloM-PER-HR")),
            "missing quantity: {:?}",
            vals
        );
        // Enum member (colour Red) with its label.
        assert!(
            vals.iter()
                .any(|v| matches!(v, TypedValue::Enum { member, label }
                if member == "http://ex/Red" && label.as_deref() == Some("Red"))),
            "missing enum member: {:?}",
            vals
        );
    }

    #[test]
    fn nl_string_grounds_via_verbalize() {
        let g = closed();
        let Some(Grounding::NlString(s)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::NlString,
            &GroundingConfig::default(),
            None,
            None,
        ) else {
            panic!("expected an NL string");
        };
        assert!(
            s.contains("Bolt"),
            "NL grounding should carry the label: {s:?}"
        );
    }

    #[test]
    fn nl_string_renders_typed_values_when_requested() {
        let g = closed();
        let cfg = GroundingConfig {
            render_typed_values: true,
            ..Default::default()
        };
        let Some(Grounding::NlString(s)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::NlString,
            &cfg,
            None,
            None,
        ) else {
            panic!("expected an NL string");
        };
        // Base passage survives.
        assert!(
            s.contains("Bolt"),
            "NL grounding should carry the label: {s:?}"
        );
        // Enum label rendered (design §4.3 "enum labels").
        assert!(
            s.contains("Red"),
            "typed-value NL should render the enum label: {s:?}"
        );
        // Unit-typed quantity rendered AS DECLARED (value + unit local name, not converted).
        assert!(
            s.contains("300 KiloM-PER-HR"),
            "typed-value NL should render the quantity: {s:?}"
        );
        // Without the flag, the base passage carries NO raw number (design: numbers out of embeds).
        let plain = GroundingConfig::default();
        let Some(Grounding::NlString(p)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::NlString,
            &plain,
            None,
            None,
        ) else {
            panic!()
        };
        assert!(
            !p.contains("300 KiloM-PER-HR"),
            "default NL must omit the typed quantity: {p:?}"
        );
    }

    #[test]
    fn nl_typed_values_respect_char_budget() {
        let g = closed();
        let mut text = EntityTextConfig {
            max_chars: 8,
            ..Default::default()
        };
        text.languages = vec!["en".into(), String::new()];
        let cfg = GroundingConfig {
            render_typed_values: true,
            text,
            ..Default::default()
        };
        let Some(Grounding::NlString(s)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::NlString,
            &cfg,
            None,
            None,
        ) else {
            panic!()
        };
        assert!(
            s.chars().count() <= 8,
            "budget must cap the typed-value NL: {s:?}"
        );
    }

    #[test]
    fn ground_rejects_absent_and_inline_nodes() {
        let g = closed();
        // Absent node.
        assert!(ground(
            &g,
            &iri("http://ex/nope"),
            Modality::Subgraph,
            &GroundingConfig::default(),
            None,
            None
        )
        .is_none());
        // A node with no typed value grounds to None for TypedValue.
        assert!(ground(
            &g,
            &iri("http://ex/Red"),
            Modality::TypedValue,
            &GroundingConfig::default(),
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn subgraph_respects_max_facts_budget() {
        let g = closed();
        let cfg = GroundingConfig {
            minimal_type_pattern: false,
            max_facts: 2,
            ..Default::default()
        };
        let Some(Grounding::Subgraph(facts)) = ground(
            &g,
            &iri("http://ex/bolt"),
            Modality::Subgraph,
            &cfg,
            None,
            None,
        ) else {
            panic!()
        };
        assert_eq!(facts.len(), 2, "k-budget must cap the subgraph");
    }

    #[test]
    fn minimal_types_drops_superclasses() {
        let g = closed();
        let bolt = g.id_of(&iri("http://ex/bolt")).unwrap();
        // bolt is asserted/entailed SportsCar, Car, Vehicle; minimal = {SportsCar} (Car, Vehicle
        // are proper superclasses).
        let minimal = minimal_types_of(&g, bolt);
        assert!(
            minimal.contains("http://ex/SportsCar"),
            "minimal must keep SportsCar: {:?}",
            minimal
        );
        assert!(
            !minimal.contains("http://ex/Vehicle"),
            "minimal must drop Vehicle superclass: {:?}",
            minimal
        );
        assert!(
            !minimal.contains("http://ex/Car"),
            "minimal must drop Car superclass: {:?}",
            minimal
        );
    }

    // ---- sq-t80n4: cross-unit reconciliation (consuming the landed P2 units.rs table) ----------

    // Two entities whose quantities are in DIFFERENT but COMMENSURABLE simple units the bundled
    // table knows: a length declared in miles vs the same length in kilometres, and a temperature
    // in °C vs the same point in K. Plus a compound rate unit (km/h) the simple-unit table does NOT
    // carry, for the conservative-on-unknown negative case.
    const UNITS_TTL: &str = r#"
@prefix qudt: <http://qudt.org/schema/qudt/> .
@prefix unit: <http://qudt.org/vocab/unit/> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:   <http://ex/> .

# 1 mile == 1.609344 km (same Length; reconciles to metres).
ex:a ex:dist ex:a-dist .
ex:a-dist qudt:numericValue "1"^^xsd:decimal ; qudt:unit unit:MI .
ex:b ex:dist ex:b-dist .
ex:b-dist qudt:numericValue "1.609344"^^xsd:decimal ; qudt:unit unit:KiloM .

# 0 °C == 273.15 K (same Temperature, AFFINE; reconciles to kelvin).
ex:c ex:temp ex:c-temp .
ex:c-temp qudt:numericValue "0"^^xsd:decimal ; qudt:unit unit:DEG_C .
ex:d ex:temp ex:d-temp .
ex:d-temp qudt:numericValue "273.15"^^xsd:decimal ; qudt:unit unit:K .

# A compound rate unit (km/h) the simple-unit table does NOT know — must stay as declared.
ex:e ex:speed ex:e-speed .
ex:e-speed qudt:numericValue "300"^^xsd:decimal ; qudt:unit unit:KiloM-PER-HR .
"#;

    fn units_graph() -> sparq_core::Graph {
        sparq_core::Graph::load_str(UNITS_TTL, "turtle").unwrap()
    }

    fn quantity_of(g: &sparq_core::Graph, node: &str, reconcile: bool) -> TypedValue {
        let id = g.id_of(&iri(node)).unwrap();
        typed_values(g, id, reconcile)
            .into_iter()
            .find(|v| matches!(v, TypedValue::Quantity { .. }))
            .unwrap_or_else(|| panic!("no quantity on {node}"))
    }

    // POSITIVE round-trip: two commensurable quantities in DIFFERENT units reconcile to the SAME
    // canonical unit + value (mi/km → metres) within a tight epsilon; without reconciliation they
    // stay distinct (the as-declared default). This is the load-bearing correctness invariant.
    #[test]
    fn commensurable_length_reconciles_mi_and_km() {
        let g = units_graph();

        // As declared (reconcile OFF): the two quantities are DISTINCT — different units, raw value.
        let mi_raw = quantity_of(&g, "http://ex/a", false);
        let km_raw = quantity_of(&g, "http://ex/b", false);
        assert!(
            matches!(&mi_raw, TypedValue::Quantity { value, unit }
                if *value == 1.0 && unit.ends_with("/MI")),
            "as-declared mile: {mi_raw:?}"
        );
        assert_ne!(
            mi_raw, km_raw,
            "different units must read as distinct when not reconciled"
        );

        // Reconciled (reconcile ON): BOTH collapse to the canonical metre with the SAME value.
        let mi = quantity_of(&g, "http://ex/a", true);
        let km = quantity_of(&g, "http://ex/b", true);
        let (
            TypedValue::Quantity {
                value: vm,
                unit: um,
            },
            TypedValue::Quantity {
                value: vk,
                unit: uk,
            },
        ) = (&mi, &km)
        else {
            panic!("expected reconciled quantities: {mi:?} {km:?}");
        };
        assert_eq!(
            um, "http://qudt.org/vocab/unit/M",
            "reconciled to canonical metre"
        );
        assert_eq!(
            uk, "http://qudt.org/vocab/unit/M",
            "reconciled to canonical metre"
        );
        assert!((vm - 1_609.344).abs() < 1e-9, "1 mi == 1609.344 m: {vm}");
        assert!(
            (vm - vk).abs() < 1e-9,
            "1 mi == 1.609344 km after reconciliation: {vm} vs {vk}"
        );
        assert_eq!(
            mi, km,
            "commensurable quantities reconcile to an identical value"
        );
    }

    // POSITIVE affine round-trip: °C and K (offset units) reconcile to the same kelvin value.
    #[test]
    fn commensurable_temperature_reconciles_celsius_and_kelvin() {
        let g = units_graph();
        let c = quantity_of(&g, "http://ex/c", true);
        let k = quantity_of(&g, "http://ex/d", true);
        let (
            TypedValue::Quantity {
                value: vc,
                unit: uc,
            },
            TypedValue::Quantity {
                value: vk,
                unit: uk,
            },
        ) = (&c, &k)
        else {
            panic!("expected reconciled temperatures: {c:?} {k:?}");
        };
        assert_eq!(uc, "http://qudt.org/vocab/unit/K");
        assert_eq!(uk, "http://qudt.org/vocab/unit/K");
        assert!((vc - 273.15).abs() < 1e-9, "0 °C == 273.15 K: {vc}");
        assert!(
            (vc - vk).abs() < 1e-9,
            "0 °C == 273.15 K after reconciliation: {vc} vs {vk}"
        );
        assert_eq!(
            c, k,
            "affine-commensurable quantities reconcile to an identical value"
        );
    }

    // NEGATIVE: a compound / unknown unit (km/h) is left AS DECLARED even with reconcile ON — never
    // a fabricated conversion. The conservative-on-unknown soundness stance.
    #[test]
    fn unknown_compound_unit_is_left_as_declared() {
        let g = units_graph();
        let declared = quantity_of(&g, "http://ex/e", false);
        let reconciled = quantity_of(&g, "http://ex/e", true);
        assert_eq!(
            declared, reconciled,
            "an unknown/compound unit (km/h) must be untouched by reconciliation: {reconciled:?}"
        );
        assert!(
            matches!(&reconciled, TypedValue::Quantity { value, unit }
                if *value == 300.0 && unit == "http://qudt.org/vocab/unit/KiloM-PER-HR"),
            "km/h must stay 300 KiloM-PER-HR, never a fabricated conversion: {reconciled:?}"
        );
    }

    // The standalone `reconcile_quantity` primitive: same conservative contract, and a non-quantity
    // passes through unchanged.
    #[test]
    fn reconcile_quantity_primitive_is_conservative() {
        // Known simple unit → canonical SI.
        let r = reconcile_quantity(TypedValue::Quantity {
            value: 1.0,
            unit: "http://qudt.org/vocab/unit/KiloM".into(),
        });
        assert!(matches!(r, TypedValue::Quantity { value, unit }
            if (value - 1000.0).abs() < 1e-9 && unit == "http://qudt.org/vocab/unit/M"));
        // Bare local name resolves too (units.rs accepts either form).
        let r2 = reconcile_quantity(TypedValue::Quantity {
            value: 1.0,
            unit: "MI".into(),
        });
        assert!(
            matches!(r2, TypedValue::Quantity { value, .. } if (value - 1609.344).abs() < 1e-9)
        );
        // Unknown unit → unchanged (NOT fabricated).
        let unknown = TypedValue::Quantity {
            value: 42.0,
            unit: "http://qudt.org/vocab/unit/NotARealUnit".into(),
        };
        assert_eq!(reconcile_quantity(unknown.clone()), unknown);
        // A non-quantity typed value is returned unchanged.
        let b = TypedValue::Boolean(true);
        assert_eq!(reconcile_quantity(b.clone()), b);
    }

    // Reconciliation flows through the NL render path too: a reconciled quantity's canonical unit +
    // value appears in the NL string; the as-declared default keeps the original unit.
    #[test]
    fn nl_render_honours_reconcile_units() {
        let g = units_graph();
        let base = EntityTextConfig {
            max_chars: 200,
            ..Default::default()
        };
        let cfg_on = GroundingConfig {
            render_typed_values: true,
            reconcile_units: true,
            text: base.clone(),
            ..Default::default()
        };
        let Some(Grounding::NlString(on)) = ground(
            &g,
            &iri("http://ex/a"),
            Modality::NlString,
            &cfg_on,
            None,
            None,
        ) else {
            panic!("expected an NL string");
        };
        assert!(
            on.contains("1609.344 M"),
            "reconciled NL should render canonical metres: {on:?}"
        );
        assert!(
            !on.contains(" MI"),
            "reconciled NL should not keep the declared mile: {on:?}"
        );

        let cfg_off = GroundingConfig {
            render_typed_values: true,
            reconcile_units: false,
            text: base,
            ..Default::default()
        };
        let Some(Grounding::NlString(off)) = ground(
            &g,
            &iri("http://ex/a"),
            Modality::NlString,
            &cfg_off,
            None,
            None,
        ) else {
            panic!("expected an NL string");
        };
        assert!(
            off.contains("1 MI"),
            "as-declared NL keeps the original mile: {off:?}"
        );
    }

    // ---- RDF 1.2 quoted-triple verbalisation (the empty-string regression) --------------------

    /// An N-Triples fixture whose subject carries a plain object property AND two RDF 1.2
    /// quoted-triple objects — one flat, one nested.
    const RDF12_NT: &str = "\
<http://ex/claim> <http://ex/statedBy> <http://ex/alice> .\n\
<http://ex/claim> <http://ex/about> <<( <http://ex/sky> <http://ex/hasColour> <http://ex/blue> )>> .\n\
<http://ex/claim> <http://ex/aboutNested> <<( <http://ex/rumour> <http://ex/says> <<( <http://ex/sky> <http://ex/hasColour> <http://ex/green> )>> )>> .\n";

    #[test]
    fn quoted_triple_objects_render_as_triple_terms_not_empty_strings() {
        // REGRESSION (bugfix): `render_object` returned the EMPTY STRING for a quoted-triple
        // object, silently corrupting every NL/subgraph grounding over RDF 1.2 data.
        let (dict, triples) = sparq_core::Graph::parse_to_triples(RDF12_NT, "ntriples").unwrap();
        let g = sparq_core::Graph::from_parts(dict, triples);

        let quoted: Vec<_> = g
            .iter_ids()
            .map(|[_, _, o]| o)
            .filter(|&o| matches!(g.dict.term_parts(o), TermParts::Triple(_)))
            .collect();
        // Two quoted OBJECTS are asserted (flat + nested-outer); the nested-inner term is a
        // component of the outer one and reachable only through its rendering (checked below).
        assert_eq!(quoted.len(), 2, "flat + nested-outer quoted-term objects");
        for &tt in &quoted {
            let (text, is_literal) = render_object(&g, tt);
            assert!(
                !text.is_empty(),
                "a quoted triple must never verbalise to the empty string"
            );
            assert!(!is_literal, "a quoted triple is not a literal");
            assert!(
                text.starts_with("<<(") && text.ends_with(")>>"),
                "expected RDF 1.2 triple-term syntax, got: {text:?}"
            );
        }

        // The nested term renders its inner term too (depth-capped by the dict reconstruction).
        let nested = quoted
            .iter()
            .map(|&tt| render_object(&g, tt).0)
            .find(|t| t.matches("<<(").count() >= 2)
            .expect("the nested quoted term must render its inner triple term");
        assert!(
            nested.contains("http://ex/green"),
            "inner term content present: {nested:?}"
        );
    }

    #[test]
    fn subgraph_grounding_over_rdf12_has_no_empty_object_fact() {
        let (dict, triples) = sparq_core::Graph::parse_to_triples(RDF12_NT, "ntriples").unwrap();
        let g = sparq_core::Graph::from_parts(dict, triples);
        // `minimal_type_pattern: false`: the fixture is deliberately schema-free, so the grounding
        // takes every asserted predicate of the node (including the quoted-triple objects).
        let cfg = GroundingConfig {
            minimal_type_pattern: false,
            ..Default::default()
        };
        let Some(Grounding::Subgraph(facts)) = ground(
            &g,
            &iri("http://ex/claim"),
            Modality::Subgraph,
            &cfg,
            None,
            None,
        ) else {
            panic!("expected a subgraph grounding");
        };
        assert!(
            facts.iter().any(|f| f.object.starts_with("<<(")),
            "the grounding must include the quoted-triple fact(s): {facts:?}"
        );
        for f in &facts {
            assert!(
                !f.object.is_empty(),
                "REGRESSION: empty-object fact for predicate {}",
                f.predicate
            );
        }
    }
}
