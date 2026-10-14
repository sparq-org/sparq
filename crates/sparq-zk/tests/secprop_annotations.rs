//! End-to-end exercise of the PUBLIC per-method annotation-graph + over-claim guard
//! surface (`sparq_zk::secprop`, Phase 3, sq-bevd3). The module's unit tests pin the
//! internal invariants against the bundled Turtle; this integration test confirms the
//! PUBLIC API a downstream caller (e.g. the trust-graph admission gate, Phase 5) sees:
//! parse the graph, run all three over-claim guards, and check the source-layer
//! non-transfer rule through the public functions only.
//!
//! Behind the default-OFF `secprop-annotations` feature — the matching feature leg
//! exercises it (the silent-skip guard); a default build compiles this file out.
//!
//! sq-bevd3 (epic sq-0dksu; design §5a). 🤖 SPARQ agent —
//! security-properties ontology.
#![cfg(feature = "secprop-annotations")]

use sparq_zk::secprop::{
    audit_overclaim_violations, completeness_violations, parse_annotations,
    production_method_iris, source_layer_transfer_violations, Assurance,
};

const SOUNDNESS: &str = "https://w3id.org/zkp-sparql/sec-prop#Soundness";
const KNOWLEDGE_SOUND: &str = "https://w3id.org/zkp-sparql/sec-prop#KnowledgeSound";
const UNLINKABILITY_SCOPE: &str = "https://w3id.org/zkp-sparql/sec-prop#UnlinkabilityScope";
const CROSS_PRESENTATION: &str = "https://w3id.org/zkp-sparql/sec-prop#CrossPresentation";
const ILLUSTRATIVE_SOURCE: &str = "https://sparq.dev/ns/zk#illustrative-source-bbs-2023";

/// The three over-claim guards all pass over the shipped graph — the load-bearing
/// invariant of the deliverable, checked through the public API.
#[test]
fn shipped_graph_satisfies_all_three_guards() {
    let ann = parse_annotations();

    // Guard 1: no positive property is Proven while sq-qhy4 is open.
    let overclaim = audit_overclaim_violations(&ann);
    assert!(
        overclaim.is_empty(),
        "shipped graph over-claims assurance: {:?}",
        overclaim,
    );

    // Guard 3: every production-selectable scheme is annotated.
    let prod = production_method_iris();
    assert!(!prod.is_empty(), "there must be production-selectable schemes");
    let missing = completeness_violations(&ann, &prod);
    assert!(
        missing.is_empty(),
        "shipped graph is incomplete: {:?}",
        missing,
    );

    // Guard 2: a query-proof-layer property transfers; a source-layer-only one does not.
    let ok = source_layer_transfer_violations(
        &ann,
        &[(prod[0], SOUNDNESS, Some(KNOWLEDGE_SOUND))],
    );
    assert!(ok.is_empty(), "query-proof-layer property wrongly blocked: {:?}", ok);

    let blocked = source_layer_transfer_violations(
        &ann,
        &[(ILLUSTRATIVE_SOURCE, UNLINKABILITY_SCOPE, Some(CROSS_PRESENTATION))],
    );
    assert_eq!(
        blocked.len(),
        1,
        "source-layer-only property wrongly transferred to a query proof: {:?}",
        blocked,
    );
}

/// A relying party introspecting an admitted method sees the honest assurance basis:
/// every positive claim is `Claimed` (never `Proven`) on the unaudited estate.
#[test]
fn no_positive_property_is_proven() {
    let ann = parse_annotations();
    for m in ann.values() {
        for a in &m.assertions {
            if a.assurance == Assurance::Proven {
                // Only the settled NEGATIVE facts may be Proven; the public
                // over-claim guard already enforces this — assert the dual here so the
                // public Assurance enum round-trips as expected.
                assert!(
                    a.level.is_some(),
                    "{} has a Proven assertion with no level",
                    m.method,
                );
            }
        }
    }
    // The guard is the authority; an empty result is the contract.
    assert!(audit_overclaim_violations(&ann).is_empty());
}
