#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};

mod incremental;
// [FABLE-5] sq-pbz04.1.2 (epic sq-pbz04.1) — the opt-in substrate seam-3 adoption: the
// SHARED SPARQL term total order (`sparq_substrate::compare`) implemented for the
// reasoner's dictionary-id term representation, so entailed-solution ordering is
// parity-identical to the engine's ORDER BY. Behind the `substrate-compare` feature; when
// off the whole module is cfg'd out (the lean-core posture, like `substrate_join`/`dtype`).
#[cfg(feature = "substrate-compare")]
pub mod compare;
#[cfg(feature = "substrate-join")]
pub(crate) mod substrate_join;
// [SONNET-4.6] sq-qonbz.2 — delta-aware adjacency tables for the OWL-RL semi-naive fixpoint,
// gated behind `substrate-join` (same feature, no new dep).
#[cfg(feature = "substrate-join")]
mod owl_delta_adj;
// [FABLE-5] sq-6tykl.3 (epic sq-6tykl) — opt-in STRATIFIED DATALOG rules (RDFox-parity
// track): a small native rule dialect with NOT (negation as failure) + AGGREGATE
// (COUNT) atoms, a stratification checker, and a non-incremental per-stratum
// evaluator on the shared substrate join kernels. Behind the `datalog` feature; when
// off the whole module is cfg'd out (the lean-core posture, like `compiled-rules`).
#[cfg(feature = "datalog")]
pub mod datalog;
#[cfg(feature = "d-entail")]
pub mod dtype;
#[cfg(feature = "explain")]
pub mod explain;
pub mod n3;
mod owl;
#[cfg(feature = "profile")]
pub mod profile;
mod rdfs;
// [Kern] kern/quoted-triple-infer — opt-in QUOTED-TRIPLE (RDF 1.2 reifier) inference
// for the OWL 2 RL profile: the reif-dtr destructure / reif-ctr construct rules, with
// reif-ctr restricted to EXISTING triples and leaf-component terms so the Herbrand
// base stays finite (termination argument in the module docs). Behind the
// `quoted-triples` feature; when off the whole module is cfg'd out (the lean-core
// posture, like `dtype` / `datalog`) and `Profile::OwlRl` closures are byte-identical
// to before this module existed. [FABLE-5] sq-afun3 (second increment): `ReifyMode` —
// `materialize_owl_rl_reify(…, ReifyMode::DestructureOnly)` is the STRICT-OPACITY
// variant (reif-dtr only; inference never mints a triple term).
#[cfg(feature = "quoted-triples")]
mod reify;
// [OPUS-4.8] sq-rh4gu (epic sq-pbz04) — the opt-in RIF-Core (monotone Horn) rule
// front-end over the N3 chainer. Behind the `rif-core` feature; when off the whole
// module is cfg'd out (the lean-core posture, like `dtype` / `explain`).
#[cfg(feature = "rif-core")]
pub mod rif;
// [SONNET-4.6] sq-pbz04.5.3 — OPT-IN RIF/XML importer: parse the W3C RIF-Core
// XML presentation syntax into `rif::Document` with Or-split / Exists-flatten
// desugaring and a fail-closed taxonomy. Requires the `rif-core` feature (wired by
// the `rif-xml` feature below). When off, zero rif_xml code is compiled.
#[cfg(feature = "rif-xml")]
pub mod rif_xml;
#[cfg(feature = "d-entail")]
pub use dtype::{d_value_eq, d_value_key, materialize_d, DValue, Recognized};
#[cfg(feature = "explain")]
pub use explain::{ExplainOpts, ProofNode, ProofTree};
pub use incremental::{
    MaterializedGraph, MaterializedN3Graph, MaterializedOwlGraph, N3Mode, OwlMode,
};
pub use n3::{
    reason_n3, reason_n3_pass_all, reason_n3_proof, reason_n3_query, reason_n3_query_terms,
    reason_n3_stratified, reason_n3_terms, N3Closure, ProofStep, RuleKind, RuleVars,
    StratifiedN3Closure,
};
pub use owl::{inconsistencies, materialize_owl_rl};
#[cfg(feature = "quoted-triples")]
pub use owl::materialize_owl_rl_reify;
pub use rdfs::materialize_rdfs;
#[cfg(feature = "quoted-triples")]
pub use reify::ReifyMode;
pub(crate) use rdfs::RdfsIndex;

/// Which entailment regime to materialize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// RDFS (subset rdfs2,3,5,7,9,11 — subclass/subproperty/domain/range; no axiomatic or
    /// reflexive triples, which add no useful inferences and explode the store).
    Rdfs,
    /// OWL 2 RL (a useful subset: equality/sameAs, inverseOf, Symmetric/Transitive/
    /// Functional/InverseFunctional properties, equivalentClass/Property) — includes RDFS.
    OwlRl,
    /// D-entailment (datatype / value-space, RDF 1.1 Semantics §7-8): the rdfD1
    /// datatype-typing rule under a recognized datatype map, with CORRECT TYPED
    /// (value-space) literal equality — `"1"^^xsd:integer` is the SAME value as
    /// `"1.0"^^xsd:decimal`. OPT-IN, behind the `d-entail` feature; see the
    /// [`dtype`] module. [OPUS-4.8] sq-e5atd. (`materialize` uses the conservative
    /// STANDARD datatype map for this profile; for a custom map call
    /// [`dtype::materialize_d`] with a [`dtype::Recognized`] directly.)
    #[cfg(feature = "d-entail")]
    D,
}

impl Profile {
    /// Parse a CLI/profile name (`"rdfs"`, `"owl"`/`"owl-rl"`; with the `d-entail`
    /// feature also `"d"`).
    pub fn parse(s: &str) -> Option<Profile> {
        match s.to_ascii_lowercase().as_str() {
            "rdfs" => Some(Profile::Rdfs),
            "owl" | "owl-rl" | "owlrl" => Some(Profile::OwlRl),
            #[cfg(feature = "d-entail")]
            "d" => Some(Profile::D),
            _ => None,
        }
    }
}

/// Materialize the closure for `profile` in place: expands `triples` with every entailed
/// triple (deduplicated against the input), interning any vocabulary terms it needs into
/// `dict`. Returns the number of NEW triples added. Idempotent: a second call adds nothing.
pub fn materialize(profile: Profile, dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> usize {
    match profile {
        Profile::Rdfs => materialize_rdfs(dict, triples),
        Profile::OwlRl => materialize_owl_rl(dict, triples),
        #[cfg(feature = "d-entail")]
        Profile::D => dtype::materialize_d(&dtype::Recognized::standard(), dict, triples),
    }
}

/// Materialize a closure and return opt-in rule instrumentation.
#[cfg(feature = "profile")]
pub fn materialize_profiled(
    profile: Profile,
    dict: &mut Dict,
    triples: &mut Vec<[Id; 3]>,
) -> (usize, profile::Report) {
    materialize_profiled_with(profile::Profiler::new(), profile, dict, triples)
}

/// Materialize with a configured profiler, including its progress callback.
#[cfg(feature = "profile")]
pub fn materialize_profiled_with(
    mut profiler: profile::Profiler,
    profile: Profile,
    dict: &mut Dict,
    triples: &mut Vec<[Id; 3]>,
) -> (usize, profile::Report) {
    let added = match profile {
        Profile::Rdfs => {
            profiler.observe("RDFS rules", |ts| rdfs::materialize_rdfs(dict, ts), triples)
        }
        Profile::OwlRl => profiler.observe(
            "OWL RL fixpoint rules",
            |ts| owl::materialize_owl_rl(dict, ts),
            triples,
        ),
        #[cfg(feature = "d-entail")]
        Profile::D => profiler.observe(
            "rdfD1",
            |ts| dtype::materialize_d(&dtype::Recognized::standard(), dict, ts),
            triples,
        ),
    };
    (added, profiler.finish())
}

/// Standard RDF/RDFS vocabulary ids, interned once. Interning is idempotent — it returns
/// the id the data already uses for e.g. `rdf:type`, or assigns a fresh one if the term is
/// absent (in which case it simply matches nothing). Predicates are never inline ids.
pub(crate) struct Vocab {
    pub ty: Id,
    pub sub_class: Id,
    pub sub_prop: Id,
    pub domain: Id,
    pub range: Id,
}

impl Vocab {
    pub fn intern(dict: &mut Dict) -> Vocab {
        use oxrdf::vocab::{rdf, rdfs};
        Vocab {
            ty: dict.intern_iri(rdf::TYPE.as_str()),
            sub_class: dict.intern_iri(rdfs::SUB_CLASS_OF.as_str()),
            sub_prop: dict.intern_iri(rdfs::SUB_PROPERTY_OF.as_str()),
            domain: dict.intern_iri(rdfs::DOMAIN.as_str()),
            range: dict.intern_iri(rdfs::RANGE.as_str()),
        }
    }
}

/// A small typed view over the schema (TBox) edges, rebuilt from the current triple set
/// each fixpoint round. Shared by the RDFS rules (and, later, OWL-RL).
#[derive(Default)]
pub(crate) struct Schema {
    /// `c -> direct super-classes d` from `(c rdfs:subClassOf d)`.
    pub sub_class: FxHashMap<Id, Vec<Id>>,
    /// `p -> direct super-properties q` from `(p rdfs:subPropertyOf q)`.
    pub sub_prop: FxHashMap<Id, Vec<Id>>,
    /// `p -> domain classes` from `(p rdfs:domain c)`.
    pub domain: FxHashMap<Id, Vec<Id>>,
    /// `p -> range classes` from `(p rdfs:range c)`.
    pub range: FxHashMap<Id, Vec<Id>>,
}

impl Schema {
    pub fn build(all: &FxHashSet<[Id; 3]>, v: &Vocab) -> Schema {
        let mut s = Schema::default();
        for &[su, p, o] in all {
            if p == v.sub_class {
                s.sub_class.entry(su).or_default().push(o);
            } else if p == v.sub_prop {
                s.sub_prop.entry(su).or_default().push(o);
            } else if p == v.domain {
                s.domain.entry(su).or_default().push(o);
            } else if p == v.range {
                s.range.entry(su).or_default().push(o);
            }
        }
        s
    }
}
