#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // sq-ntcg: crate has zero `unsafe`.

//! # sparq-prov — W3C PROV-O lineage for DERIVED data
//!
//! When a sparq operation **derives** new triples from existing data, this crate
//! optionally records *who/what/when* produced them, as standard
//! [W3C PROV-O](https://www.w3.org/TR/prov-o/) RDF: a [`prov:Activity`] (the
//! operation, time-stamped with [`prov:startedAtTime`]/[`prov:endedAtTime`]), a
//! [`prov:Entity`] (the derived result graph), and the lineage edges
//! [`prov:wasGeneratedBy`], [`prov:used`] (the inputs), and
//! [`prov:wasDerivedFrom`].
//!
//! ## Opt-in by construction, lean core
//!
//! This is a **standalone, opt-in** crate. Nothing in sparq's default build or
//! the wasm artifact depends on it — `sparq-core`/`sparq-engine` stay lean and
//! pay **zero overhead** when it is absent. Pull it in explicitly
//! (`sparq-prov = { path = "..." }`) only when you want lineage capture.
//!
//! ## Scope (covered today vs deferred)
//!
//! The **CONSTRUCT** derivation path is covered: `CONSTRUCT`/`DESCRIBE` queries
//! produce a *new RDF graph* from the queried data, which is exactly a PROV
//! derivation. [`derive_construct`] runs the query, times it, and returns a
//! [`Derivation`] carrying both the derived triples and a PROV-O lineage graph.
//!
//! The **SPARQL UPDATE** path is covered: an `INSERT … WHERE` / `INSERT DATA` /
//! `DELETE …` mutation has a two-sided PROV reading — the triples it **inserts** are
//! *generated* (a `prov:Entity` `wasGeneratedBy` the update, `wasDerivedFrom` the
//! matched inputs), the triples it **deletes** are *invalidated*
//! (`prov:wasInvalidatedBy`). [`derive_update`] applies the update in place, captures
//! the engine's resolved effect log, and returns an [`UpdateDerivation`] carrying the
//! inserted + deleted triples and the PROV-O lineage. See the `update` module.
//!
//! **Reasoner-materialization** lineage is covered under the (non-default)
//! `reason` feature: `prov_from_proof` maps a `sparq-reason` `why()`
//! `ProofTree` to PROV-O — one `prov:Entity` per
//! inferred fact, one `prov:Activity` per rule firing, with
//! `wasGeneratedBy`/`used`/`wasDerivedFrom` edges. Inference *is* derivation, and
//! the proof tree is a finer-grained provenance (it names the rule and exact
//! premises for each fact) than a single CONSTRUCT-style activity.
//! `prov_ntriples` renders that lineage as N-Triples in one call. See the
//! `reason` module. <!-- sq-8jn86 -->
//!
//! **Missing-answer explanation** is available under the non-default `why-not`
//! feature. The `why_not` function accepts one basic graph pattern and a fully
//! ground target binding, then reports exactly the substituted triples absent
//! from the graph. `why_not_report_ntriples` and `why_not_report_turtle` render
//! that result as deterministic, PROV-flavoured RDF 1.2. More general SPARQL
//! algebra is rejected as unsupported. <!-- sq-lsp7k -->
//!
//! **Out of scope for per-triple lineage** (a deliberate honesty boundary, not a TODO):
//! the structural UPDATE operations `CLEAR` / `DROP` / `CREATE` change a graph's
//! existence/emptiness rather than its triples-as-data and carry no resolved triple set,
//! so [`derive_update`] records them as an activity kind but emits no per-triple entity.
//!
//! [`prov:Activity`]: https://www.w3.org/TR/prov-o/#Activity
//! [`prov:Entity`]: https://www.w3.org/TR/prov-o/#Entity
//! [`prov:startedAtTime`]: https://www.w3.org/TR/prov-o/#startedAtTime
//! [`prov:endedAtTime`]: https://www.w3.org/TR/prov-o/#endedAtTime
//! [`prov:wasGeneratedBy`]: https://www.w3.org/TR/prov-o/#wasGeneratedBy
//! [`prov:used`]: https://www.w3.org/TR/prov-o/#used
//! [`prov:wasDerivedFrom`]: https://www.w3.org/TR/prov-o/#wasDerivedFrom

use std::time::{SystemTime, UNIX_EPOCH};

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_core::Graph;
use sparq_engine::QueryBudget;

// sq-xwdd: SPARQL UPDATE lineage — capture PROV-O for the data an
// `INSERT … WHERE` / `INSERT DATA` / `DELETE …` operation changes in a store. Like
// the CONSTRUCT path it depends only on sparq-core/sparq-engine (no extra feature).
mod update;
pub use update::{derive_update, derive_update_with_budget, UpdateDerivation};
// sq-m3i0: reasoner-materialization lineage — map a sparq-reason
// `why()` proof tree → PROV-O RDF. Non-default feature, so the sparq-reason dep
// is pulled only when asked for.
#[cfg(feature = "reason")]
pub mod reason;
#[cfg(feature = "reason")]
pub use reason::{prov_from_proof, prov_ntriples, ProvProofConfig};
// sq-lsp7k.17: exact missing-conjunct explanation, opt-in so the
// default public API remains unchanged.
#[cfg(feature = "why-not")]
mod why_not;
#[cfg(feature = "why-not")]
pub use why_not::{
    why_not, why_not_report_ntriples, why_not_report_turtle, MissingPattern, WhyNotError,
};

// ── PROV-O vocabulary IRIs ─────────────────────────────────────────────────
const PROV: &str = "http://www.w3.org/ns/prov#";

// sq-ijw35: one Turtle writer for both derivation surfaces so they
// always serialise the identical `prov_graph()` triple set with the same prefixes.
fn triples_to_prov_turtle(triples: &[Triple]) -> String {
    let serializer = oxttl::TurtleSerializer::new()
        .with_prefix("prov", PROV)
        .expect("the PROV namespace must be a valid prefix IRI")
        .with_prefix("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#")
        .expect("the RDF namespace must be a valid prefix IRI")
        .with_prefix("rdfs", "http://www.w3.org/2000/01/rdf-schema#")
        .expect("the RDFS namespace must be a valid prefix IRI")
        .with_prefix("xsd", "http://www.w3.org/2001/XMLSchema#")
        .expect("the XSD namespace must be a valid prefix IRI");
    let mut writer = serializer.for_writer(Vec::new());
    for triple in triples {
        writer
            .serialize_triple(triple)
            .expect("serialising Turtle into memory must succeed");
    }
    let bytes = writer
        .finish()
        .expect("finishing Turtle serialisation into memory must succeed");
    String::from_utf8(bytes).expect("Turtle output must be valid UTF-8")
}

/// `prov:` term IRI (`http://www.w3.org/ns/prov#{local}`).
fn prov(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{PROV}{local}"))
}

/// IRI prefix sparq mints derivation Activity/Entity nodes under when the caller
/// does not supply explicit IRIs. Stable + dereferenceable-shaped so the records
/// are self-describing.
pub const SPARQ_PROV_NS: &str = "urn:sparq:prov:";

// ───────────────────────────────────────────────────────────────────────────

/// How a derived result's lineage is identified and attributed.
///
/// IRIs default to freshly-minted `urn:sparq:prov:` nodes; set them explicitly
/// to integrate with an external provenance store / dataset-graph naming scheme.
#[derive(Clone, Debug)]
pub struct ProvConfig {
    /// IRI naming the [`prov:Activity`](https://www.w3.org/TR/prov-o/#Activity)
    /// (the derivation operation). `None` ⇒ mint a fresh `urn:sparq:prov:activity:…`.
    pub activity: Option<NamedNode>,
    /// IRI naming the derived-result [`prov:Entity`](https://www.w3.org/TR/prov-o/#Entity).
    /// `None` ⇒ mint a fresh `urn:sparq:prov:entity:…`.
    pub entity: Option<NamedNode>,
    /// Input-source IRIs the activity [`prov:used`](https://www.w3.org/TR/prov-o/#used)
    /// and the result [`prov:wasDerivedFrom`](https://www.w3.org/TR/prov-o/#wasDerivedFrom).
    /// Typically the source dataset / named-graph IRI(s). May be empty.
    pub used: Vec<NamedNode>,
    /// Optional [`prov:Agent`](https://www.w3.org/TR/prov-o/#Agent) the activity is
    /// [`prov:wasAssociatedWith`](https://www.w3.org/TR/prov-o/#wasAssociatedWith)
    /// (e.g. the service / user that ran it).
    pub agent: Option<NamedNode>,
    /// Wall clock for `startedAtTime`/`endedAtTime`. Injectable for deterministic
    /// tests; defaults to [`SystemTime::now`].
    pub clock: fn() -> SystemTime,
}

impl Default for ProvConfig {
    fn default() -> Self {
        ProvConfig {
            activity: None,
            entity: None,
            used: Vec::new(),
            agent: None,
            clock: SystemTime::now,
        }
    }
}

impl ProvConfig {
    /// A config that records `used` / `wasDerivedFrom` from the given input IRIs.
    pub fn with_inputs(used: impl IntoIterator<Item = NamedNode>) -> Self {
        ProvConfig {
            used: used.into_iter().collect(),
            ..Self::default()
        }
    }
}

/// A completed derivation: the derived triples plus the recorded PROV-O lineage.
///
/// Produced by [`derive_construct`]. [`prov_graph`](Self::prov_graph) materialises
/// the lineage as PROV-O RDF; [`triples`](Self::triples) is the derived data itself.
#[derive(Clone, Debug)]
pub struct Derivation {
    triples: Vec<Triple>,
    activity: NamedNode,
    entity: NamedNode,
    used: Vec<NamedNode>,
    agent: Option<NamedNode>,
    started: SystemTime,
    ended: SystemTime,
    /// The SPARQL text of the derivation (recorded as `rdfs:label`-free
    /// `prov:value`-style annotation on the activity).
    query: String,
    /// Operation kind label (e.g. `"CONSTRUCT"`), surfaced on the activity.
    kind: &'static str,
}

impl Derivation {
    /// The derived triples (the data the operation produced).
    pub fn triples(&self) -> &[Triple] {
        &self.triples
    }

    /// The activity IRI ([`prov:Activity`](https://www.w3.org/TR/prov-o/#Activity)).
    pub fn activity(&self) -> &NamedNode {
        &self.activity
    }

    /// The derived-result entity IRI ([`prov:Entity`](https://www.w3.org/TR/prov-o/#Entity)).
    pub fn entity(&self) -> &NamedNode {
        &self.entity
    }

    /// The configured input-source IRIs, in their original order.
    // sq-cg237
    pub fn used_inputs(&self) -> &[NamedNode] {
        &self.used
    }

    /// The PROV-O lineage of this derivation as an RDF graph (a `Vec<Triple>`).
    ///
    /// Emits, for the result entity `E`, activity `A`, and each input `Iᵢ`:
    ///
    /// - `A a prov:Activity`, `E a prov:Entity`
    /// - `A prov:startedAtTime "…"^^xsd:dateTime`, `A prov:endedAtTime "…"^^xsd:dateTime`
    /// - `E prov:wasGeneratedBy A`
    /// - `A prov:used Iᵢ`, `E prov:wasDerivedFrom Iᵢ` (for each input)
    /// - `A prov:wasAssociatedWith G` (if an agent is configured)
    ///
    /// All IRIs are absolute, so the graph is valid PROV-O and round-trips through
    /// any RDF parser.
    pub fn prov_graph(&self) -> Vec<Triple> {
        let a = NamedOrBlankNode::NamedNode(self.activity.clone());
        let e = NamedOrBlankNode::NamedNode(self.entity.clone());
        let mut out: Vec<Triple> = Vec::new();
        let mut push = |s: &NamedOrBlankNode, p: NamedNode, o: Term| {
            out.push(Triple {
                subject: s.clone(),
                predicate: p,
                object: o,
            });
        };

        // Types.
        push(
            &a,
            NamedNode::from(rdf::TYPE),
            Term::NamedNode(prov("Activity")),
        );
        push(
            &e,
            NamedNode::from(rdf::TYPE),
            Term::NamedNode(prov("Entity")),
        );

        // Activity timing (xsd:dateTime, UTC, RFC 3339 with a `Z` offset).
        push(
            &a,
            prov("startedAtTime"),
            Term::Literal(datetime_literal(self.started)),
        );
        push(
            &a,
            prov("endedAtTime"),
            Term::Literal(datetime_literal(self.ended)),
        );

        // The operation kind + the query text, as plain annotations on the activity
        // (prov:label is rdfs:label; prov:value carries the literal generating recipe).
        push(
            &a,
            NamedNode::new_unchecked("http://www.w3.org/2000/01/rdf-schema#label"),
            Term::Literal(Literal::new_simple_literal(self.kind)),
        );
        push(
            &a,
            prov("value"),
            Term::Literal(Literal::new_simple_literal(&self.query)),
        );

        // Generation + derivation lineage.
        push(
            &e,
            prov("wasGeneratedBy"),
            Term::NamedNode(self.activity.clone()),
        );
        for input in &self.used {
            push(&a, prov("used"), Term::NamedNode(input.clone()));
            push(&e, prov("wasDerivedFrom"), Term::NamedNode(input.clone()));
        }
        if let Some(agent) = &self.agent {
            push(
                &a,
                prov("wasAssociatedWith"),
                Term::NamedNode(agent.clone()),
            );
        }
        out
    }

    /// The PROV-O lineage serialised as N-Triples (also a valid Turtle document).
    pub fn prov_ntriples(&self) -> String {
        sparq_engine::triples_to_ntriples(&self.prov_graph())
    }

    /// The same PROV-O lineage as [`prov_graph`](Self::prov_graph), serialised as
    /// prefix-compacted Turtle.
    ///
    /// sq-ijw35
    pub fn prov_turtle(&self) -> String {
        triples_to_prov_turtle(&self.prov_graph())
    }

    /// Start/end wall-clock instants of the derivation activity.
    pub fn timing(&self) -> (SystemTime, SystemTime) {
        (self.started, self.ended)
    }
}

/// Run a `CONSTRUCT`/`DESCRIBE` query against `graph`, capturing W3C PROV-O lineage
/// for the derived graph.
///
/// The returned [`Derivation`] carries the derived triples and a PROV-O record:
/// a time-stamped `prov:Activity` (the CONSTRUCT), a `prov:Entity` (the result),
/// and `wasGeneratedBy` / `used` / `wasDerivedFrom` edges to the configured inputs.
///
/// Errors if `sparql` is not a `CONSTRUCT`/`DESCRIBE` query (the engine rejects
/// SELECT/ASK as non-graph-valued).
pub fn derive_construct(
    graph: &Graph,
    sparql: &str,
    config: ProvConfig,
) -> Result<Derivation, String> {
    derive_construct_with_budget(graph, sparql, config, &QueryBudget::unlimited())
}

/// [`derive_construct`] under a cooperative [`QueryBudget`].
pub fn derive_construct_with_budget(
    graph: &Graph,
    sparql: &str,
    config: ProvConfig,
    budget: &QueryBudget,
) -> Result<Derivation, String> {
    let started = (config.clock)();
    let triples = sparq_engine::construct_or_describe_with_budget(graph, sparql, budget)?;
    let ended = (config.clock)();

    let activity = config
        .activity
        .unwrap_or_else(|| mint("activity", started, ended, sparql));
    let entity = config
        .entity
        .unwrap_or_else(|| mint("entity", started, ended, sparql));

    Ok(Derivation {
        triples,
        activity,
        entity,
        used: config.used,
        agent: config.agent,
        started,
        ended,
        query: sparql.to_string(),
        kind: "CONSTRUCT",
    })
}

/// Mint a stable `urn:sparq:prov:{role}:…` IRI from the timing + query hash, so the
/// same derivation names the same nodes across runs without a global counter.
fn mint(role: &str, started: SystemTime, ended: SystemTime, sparql: &str) -> NamedNode {
    let s = started
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let e = ended
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let h = fnv1a(sparql.as_bytes());
    NamedNode::new_unchecked(format!("{SPARQ_PROV_NS}{role}:{s:x}-{e:x}-{h:016x}"))
}

/// 64-bit FNV-1a (no crypto requirement — just a stable, dependency-free id hash).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `SystemTime` → an `xsd:dateTime` literal (UTC, `…Z`).
fn datetime_literal(t: SystemTime) -> Literal {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Literal::new_typed_literal(sparq_core::civil::format_utc(secs), xsd::DATE_TIME)
}

#[cfg(test)]
mod tests;
