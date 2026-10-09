//! **Reasoner-materialization lineage** (`reason` feature): turn a
//! [`sparq-reason`](https://docs.rs/sparq-reason) `why()` **proof tree** into W3C
//! PROV-O RDF.
//!
//! # Inference *is* derivation
//!
//! When a reasoner materializes a triple, that triple is *derived* — exactly the
//! relationship PROV-O exists to record. `sparq-reason` already computes, per
//! materialized triple, a [`sparq_reason::ProofTree`]: a flat DAG of
//! [`sparq_reason::ProofNode`]s where every node carries a `conclusion`
//! (the fact), a `rule` id (`"asserted"` for a base fact, `axiom-*` for a
//! tautology, a W3C/engine rule id otherwise — `cax-sco`, `rdfs9`, `prp-trp`,
//! `n3-rule-<i>`, …), and the indices of its `premises`. That proof tree is a
//! *finer-grained* derivation record than a single CONSTRUCT-style PROV activity:
//! it names the rule that fired and the exact premises for **each** inferred fact.
//!
//! This module is the bridge. `prov_from_proof` walks one proof tree and emits,
//! for the proof's structure:
//!
//! * one [`prov:Entity`] per distinct fact (the node's conclusion), with a stable
//!   content-addressed IRI so the same fact names the same entity across proofs;
//! * one [`prov:Activity`] per rule application (a non-leaf node), labelled with
//!   the rule id ([`rdfs:label`]) and time-stamped if a clock is configured;
//! * the lineage edges: `entity` [`prov:wasGeneratedBy`] `activity`, and for each
//!   premise `activity` [`prov:used`] `premiseEntity` + `entity`
//!   [`prov:wasDerivedFrom`] `premiseEntity`.
//!
//! Asserted leaves and `axiom-*` tautologies are entities with no generating
//! activity (they were not *derived* — they are the inputs the derivation rests on),
//! which is the correct PROV reading: a leaf is the boundary of the lineage.
//!
//! # Why content-addressed IRIs
//!
//! A proof's `conclusion` strings are self-contained N-Triples-style term strings
//! (no engine ids leak — see the `sparq-reason::explain` module docs), so the same
//! fact stringifies identically everywhere. Hashing that string into the entity IRI
//! makes the lineage **stitchable**: emit PROV for two overlapping proofs and the
//! shared sub-facts coincide, so `wasDerivedFrom` chains join up into one DAG
//! instead of two disconnected trees. Activities are addressed by `(rule,
//! conclusion)` for the same reason (one activity per distinct rule firing).
//!
//! [`prov:Entity`]: https://www.w3.org/TR/prov-o/#Entity
//! [`prov:Activity`]: https://www.w3.org/TR/prov-o/#Activity
//! [`prov:wasGeneratedBy`]: https://www.w3.org/TR/prov-o/#wasGeneratedBy
//! [`prov:used`]: https://www.w3.org/TR/prov-o/#used
//! [`prov:wasDerivedFrom`]: https://www.w3.org/TR/prov-o/#wasDerivedFrom
//! [`rdfs:label`]: http://www.w3.org/2000/01/rdf-schema#label

use std::time::SystemTime;

use oxrdf::vocab::rdf;
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_reason::ProofTree;

use crate::{datetime_literal, fnv1a, prov, SPARQ_PROV_NS};

/// The `rule` id a leaf [`ProofNode`](sparq_reason::ProofNode) carries when it is an
/// explicitly asserted base triple (the proof's boundary — no generating activity).
const ASSERTED: &str = "asserted";
/// Prefix of the `rule` id for the few premise-free tautology nodes (the XSD
/// numeric-tower `rdfs:subClassOf` axioms, etc.). Treated like [`ASSERTED`]: an
/// entity boundary, but labelled with the axiom name for traceability.
const AXIOM_PREFIX: &str = "axiom-";

const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

/// How reasoner-materialization lineage is identified and time-stamped.
///
/// Unlike [`ProvConfig`](crate::ProvConfig) (which names *one* activity/entity for
/// a whole CONSTRUCT), a proof tree has *many* facts and rule firings, so the IRIs
/// here are minted **per node** (content-addressed; see the module docs). What the
/// caller controls is the namespace, the optional clock, and the optional agent.
#[derive(Clone, Debug)]
pub struct ProvProofConfig {
    /// IRI prefix the per-fact entities and per-rule activities are minted under.
    /// Defaults to [`SPARQ_PROV_NS`]. Set it to integrate with an external store.
    pub namespace: String,
    /// Optional wall clock: when `Some`, every rule-application activity is
    /// time-stamped with `prov:generatedAtTime` (a single instant — the moment
    /// lineage was captured, not a per-rule duration, which a finished proof no
    /// longer has). `None` ⇒ no timestamps (fully deterministic output).
    pub clock: Option<fn() -> SystemTime>,
    /// Optional [`prov:Agent`](https://www.w3.org/TR/prov-o/#Agent) every rule
    /// activity is [`prov:wasAssociatedWith`](https://www.w3.org/TR/prov-o/#wasAssociatedWith)
    /// (e.g. the reasoner profile / service identity).
    pub agent: Option<NamedNode>,
}

impl Default for ProvProofConfig {
    fn default() -> Self {
        ProvProofConfig {
            namespace: SPARQ_PROV_NS.to_string(),
            clock: None,
            agent: None,
        }
    }
}

impl ProvProofConfig {
    /// A config minting under the default [`SPARQ_PROV_NS`] namespace, time-stamping
    /// activities with the given clock (so `prov:generatedAtTime` is recorded).
    pub fn with_clock(clock: fn() -> SystemTime) -> Self {
        ProvProofConfig {
            clock: Some(clock),
            ..Self::default()
        }
    }
}

/// Emit W3C PROV-O lineage RDF for one [`ProofTree`] (a `sparq-reason` `why()`
/// result) — see the module docs for the shape.
///
/// Returns the lineage as a `Vec<Triple>` (a valid RDF graph that round-trips
/// through any parser). The proof's *conclusion* (root) is the explained,
/// materialized fact; its leaves are the asserted facts / axioms the derivation
/// rests on; every internal node becomes a rule-application activity.
///
/// Deterministic for a given proof + config (modulo the optional clock): node
/// order follows the proof's premises-before-conclusion order, and all IRIs are
/// content-addressed.
pub fn prov_from_proof(proof: &ProofTree, config: &ProvProofConfig) -> Vec<Triple> {
    let now = config.clock.map(|c| datetime_literal(c()));
    let mut out: Vec<Triple> = Vec::new();
    let mut push = |s: NamedOrBlankNode, p: NamedNode, o: Term| {
        out.push(Triple {
            subject: s,
            predicate: p,
            object: o,
        });
    };

    // First pass: a stable entity IRI per node (content-addressed by conclusion).
    let entity_iris: Vec<NamedNode> = proof
        .nodes()
        .iter()
        .map(|n| fact_entity(&config.namespace, &n.conclusion))
        .collect();

    for (i, node) in proof.nodes().iter().enumerate() {
        let entity = NamedOrBlankNode::NamedNode(entity_iris[i].clone());

        // Every node is a prov:Entity (the fact it concludes).
        push(
            entity.clone(),
            NamedNode::from(rdf::TYPE),
            Term::NamedNode(prov("Entity")),
        );

        // A leaf (asserted base fact / axiom tautology) is the boundary of the
        // lineage: it has no generating activity. Axioms keep a label so the
        // tautology is still traceable.
        let is_asserted = node.rule == ASSERTED;
        let is_axiom = node.rule.starts_with(AXIOM_PREFIX);
        if is_asserted || is_axiom {
            if is_axiom {
                push(
                    entity.clone(),
                    NamedNode::new_unchecked(RDFS_LABEL),
                    Term::Literal(Literal::new_simple_literal(&node.rule)),
                );
            }
            continue;
        }

        // An internal node: the rule fired to GENERATE this fact from its premises.
        let activity_iri = rule_activity(&config.namespace, &node.rule, &node.conclusion);
        let activity = NamedOrBlankNode::NamedNode(activity_iri.clone());
        push(
            activity.clone(),
            NamedNode::from(rdf::TYPE),
            Term::NamedNode(prov("Activity")),
        );
        // The rule id (cax-sco / rdfs9 / n3-rule-i / …) as an rdfs:label.
        push(
            activity.clone(),
            NamedNode::new_unchecked(RDFS_LABEL),
            Term::Literal(Literal::new_simple_literal(&node.rule)),
        );
        if let Some(ref lit) = now {
            push(
                activity.clone(),
                prov("generatedAtTime"),
                Term::Literal(lit.clone()),
            );
        }
        if let Some(agent) = &config.agent {
            push(
                activity.clone(),
                prov("wasAssociatedWith"),
                Term::NamedNode(agent.clone()),
            );
        }
        // Generation: the derived fact was generated by this rule application.
        push(
            entity.clone(),
            prov("wasGeneratedBy"),
            Term::NamedNode(activity_iri.clone()),
        );
        // Lineage to each premise: the activity used it, the fact derives from it.
        for &p in &node.premises {
            let prem = &entity_iris[p as usize];
            push(
                activity.clone(),
                prov("used"),
                Term::NamedNode(prem.clone()),
            );
            push(
                entity.clone(),
                prov("wasDerivedFrom"),
                Term::NamedNode(prem.clone()),
            );
        }
    }
    out
}

/// Render one reasoner proof tree's W3C PROV-O lineage as N-Triples.
///
/// This is the string-serialization counterpart to `prov_from_proof`; it preserves
/// that function's deterministic triple order and content-addressed IRIs.
///
/// [GPT-5.6] sq-8jn86
pub fn prov_ntriples(proof: &ProofTree, config: &ProvProofConfig) -> String {
    sparq_engine::triples_to_ntriples(&prov_from_proof(proof, config))
}

/// Mint a stable `…fact:<hash>` entity IRI from a fact's term strings. The
/// conclusion strings are canonical (one rendering per term), so the same fact
/// always names the same entity — letting lineage from different proofs stitch.
fn fact_entity(ns: &str, conclusion: &[String; 3]) -> NamedNode {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in conclusion {
        h = mix(h, fnv1a(part.as_bytes()));
    }
    NamedNode::new_unchecked(format!("{ns}fact:{h:016x}"))
}

/// Mint a stable `…rule:<hash>` activity IRI for one rule firing — addressed by
/// `(rule, conclusion)` so one activity exists per distinct rule application.
fn rule_activity(ns: &str, rule: &str, conclusion: &[String; 3]) -> NamedNode {
    let mut h = fnv1a(rule.as_bytes());
    for part in conclusion {
        h = mix(h, fnv1a(part.as_bytes()));
    }
    NamedNode::new_unchecked(format!("{ns}rule:{h:016x}"))
}

/// Order-sensitive 64-bit mix of two hashes (so `(a,b,c)` and a permutation differ).
fn mix(acc: u64, x: u64) -> u64 {
    // FNV-1a-style fold of `x` into `acc`, byte by byte (keeps it dependency-free).
    let mut h = acc;
    for b in x.to_le_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
