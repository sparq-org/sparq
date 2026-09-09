//! Well-formedness guard for the trust-expression conformance suite DATA
//! (`tests/trust-expression/**`; bead `sq-6syab.3`, epic `sq-6syab`, issue
//! [#1592](https://github.com/sparq-org/sparq/issues/1592); design record
//! `research/trust-expression-spec.md` §6).
//!
//! The companion `trust_expression_conformance.rs` runs the suite's SEMANTICS
//! through the holder-side evaluation API. This file guards the DATA itself, so a
//! malformed or self-contradicting fixture is a red test rather than a case that
//! silently stops testing anything:
//!
//! * the manifest parses, declares one `mf:Manifest`, and lists a well-formed
//!   `rdf:List` of entries — all ten of them, covering every one of design §6's
//!   eight case classes;
//! * every referenced file exists, is non-empty, and parses in its own syntax —
//!   Turtle for `TR`, TriG for the holder dataset (which must declare at least one
//!   named graph, since the contract evaluates each statement inside its
//!   attestation bundle's graph). `Q` is only checked to be an ASK or SELECT here:
//!   the SPARQL parser lives behind the `expression` feature, so parsing `Q` for
//!   real is the conformance RUNNER's job, and it does exactly that;
//! * every trust-requirements document is a well-formed `TR` per design §3.2 —
//!   exactly one `trustx:TrustRequirements` node carrying exactly one question and
//!   one status instant, and at least one trust mode;
//! * every `trustx:` IRI used anywhere in the suite is a REAL term of
//!   [`sparq_trust::framework_vocab`] (the drift guard: a fixture cannot invent
//!   vocabulary the implementation does not define, and renaming a constant
//!   without re-authoring the fixtures goes red);
//! * the manifest's own expectations are internally consistent — every
//!   `tec:NoBinding` case pins zero contributing statements and every
//!   `tec:Admitted` case pins at least one;
//! * every negative case class of §6 is actually present, so the suite cannot
//!   quietly lose its fail-closed coverage;
//! * no `KNOWN_FAILING` marker exists anywhere in the suite (design §6: zero
//!   unbeaded known-failing entries — sparq passes ALL of this suite).
//!
//! Behind the default-OFF `framework-vocab` feature, exactly like the vocabulary
//! it pins against. Run:
//! `cargo test -p sparq-trust --features framework-vocab --test trust_expression_fixtures`.
//!
//! [SONNET-4.6] sq-6syab.3 (epic sq-6syab; issue #1592). 🤖 SPARQ agent —
//! trust-expression conformance suite data.
#![cfg(feature = "framework-vocab")]

use std::collections::BTreeSet;

use oxrdf::{NamedOrBlankNode, Term};
use oxttl::{TriGParser, TurtleParser};
use sparq_trust::framework_vocab::{
    ALL_TRUSTX_IRIS, TRUSTX_NS, TRUSTX_QUESTION, TRUSTX_REQUIRES_VALID_STATUS_AT,
    TRUSTX_TRUSTS_FRAMEWORK, TRUSTX_TRUSTS_ISSUER, TRUSTX_TRUST_REQUIREMENTS,
};

#[path = "trust_expression_manifest/mod.rs"]
mod manifest;

use manifest::{load_manifest, suite_dir, Expect};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// Every `trustx:` IRI appearing in `text` (as a full `<…>` IRI, which is how the
/// fixtures' expanded prefixes serialise once parsed).
fn trustx_iris(triples: impl Iterator<Item = (String, String, Term)>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut note = |iri: &str| {
        if iri.starts_with(TRUSTX_NS) {
            found.insert(iri.to_string());
        }
    };
    for (subject, predicate, object) in triples {
        note(&subject);
        note(&predicate);
        if let Term::NamedNode(n) = &object {
            note(n.as_str());
        }
    }
    found
}

fn read(path: &std::path::Path) -> String {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));
    assert!(
        !text.trim().is_empty(),
        "{} is empty — a fixture that supplies nothing tests nothing",
        path.display()
    );
    text
}

fn turtle_triples(path: &std::path::Path) -> Vec<(NamedOrBlankNode, String, Term)> {
    let text = read(path);
    TurtleParser::new()
        .for_reader(text.as_bytes())
        .map(|r| {
            let t = r.unwrap_or_else(|e| panic!("{} must be valid Turtle: {}", path.display(), e));
            (t.subject, t.predicate.as_str().to_string(), t.object)
        })
        .collect()
}

/// The manifest resolves to a non-empty, well-formed case list covering every one
/// of design §6's eight case classes.
#[test]
fn manifest_covers_every_design_case_class() {
    let cases = load_manifest();
    assert_eq!(
        cases.len(),
        10,
        "the suite must carry the eight §6 case classes across ten entries (case 6 \
         adds its in-scope positive twin, case 7 both of its halves); found {}",
        cases.len()
    );
    let classes: BTreeSet<u32> = cases.iter().map(|c| c.case_class).collect();
    for class in 1..=8u32 {
        assert!(
            classes.contains(&class),
            "design §6 case class {} has no manifest entry — the suite lost coverage",
            class
        );
    }
    let ids: BTreeSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), cases.len(), "manifest entry IRIs must be unique");
    let nonces: BTreeSet<&str> = cases.iter().map(|c| c.nonce.as_str()).collect();
    assert_eq!(
        nonces.len(),
        cases.len(),
        "each case must carry its OWN challenge nonce — a shared nonce would let a \
         response from one case satisfy another's freshness binding"
    );
}

/// The manifest's expectations are internally consistent: fail-closed cases pin
/// zero disclosed statements, binding cases pin at least one.
#[test]
fn manifest_expectations_are_self_consistent() {
    let cases = load_manifest();
    let mut negatives = 0usize;
    for case in &cases {
        match case.expect {
            Expect::NoBinding => {
                negatives += 1;
                assert_eq!(
                    case.contributing,
                    0,
                    "{}: a fail-closed case MUST pin zero contributing statements — \
                     no admissible derivation means no disclosure at all",
                    case.label()
                );
            }
            Expect::Admitted => assert!(
                case.contributing > 0,
                "{}: an admitted case pinning zero contributing statements would assert \
                 nothing about the response",
                case.label()
            ),
        }
    }
    assert!(
        negatives >= 6,
        "design §6 defines six fail-closed case classes (2, 3, 4, 6, 7 expired, 7 revoked); \
         only {} negative entries remain",
        negatives
    );
}

/// Every file the manifest references exists and parses in its own syntax.
#[test]
fn every_referenced_fixture_parses() {
    for case in load_manifest() {
        // The trust-requirements document is Turtle.
        let tr = turtle_triples(&case.requirements);
        assert!(
            !tr.is_empty(),
            "{}: {} parsed to zero triples",
            case.label(),
            case.requirements.display()
        );

        // The holder's attested dataset is TriG (named graphs are load-bearing:
        // one graph per attestation bundle).
        let data = read(&case.data);
        let mut named_graphs = 0usize;
        for result in TriGParser::new().for_reader(data.as_bytes()) {
            let quad = result.unwrap_or_else(|e| {
                panic!("{} must be valid TriG: {}", case.data.display(), e)
            });
            if !matches!(quad.graph_name, oxrdf::GraphName::DefaultGraph) {
                named_graphs += 1;
            }
        }
        assert!(
            named_graphs > 0,
            "{}: {} declares no named graph — the contract evaluates each statement \
             inside its attestation bundle's graph, so a default-graph-only holder \
             fixture can never bind",
            case.label(),
            case.data.display()
        );

        // The query is SPARQL the contract's v1 fragment accepts (ASK / SELECT).
        let query = read(&case.query);
        let head = query.to_ascii_uppercase();
        assert!(
            head.contains("ASK") || head.contains("SELECT"),
            "{}: {} is neither an ASK nor a SELECT — outside the supported fragment",
            case.label(),
            case.query.display()
        );
    }
}

/// Every trust-requirements document is a well-formed `TR` per design §3.2.
#[test]
fn every_requirements_document_is_a_well_formed_tr() {
    for case in load_manifest() {
        let triples = turtle_triples(&case.requirements);
        let where_ = format!("{} ({})", case.label(), case.requirements.display());

        let nodes: BTreeSet<String> = triples
            .iter()
            .filter(|(_, p, o)| {
                p == RDF_TYPE
                    && matches!(o, Term::NamedNode(n) if n.as_str() == TRUSTX_TRUST_REQUIREMENTS)
            })
            .map(|(s, _, _)| s.to_string())
            .collect();
        assert_eq!(
            nodes.len(),
            1,
            "{}: TR must declare exactly ONE trustx:TrustRequirements node (design D1)",
            where_
        );
        let node = nodes.iter().next().expect("length checked above").clone();

        let count = |predicate: &str| {
            triples
                .iter()
                .filter(|(s, p, _)| s.to_string() == node && p == predicate)
                .count()
        };
        assert_eq!(
            count(TRUSTX_QUESTION),
            1,
            "{}: TR must name exactly one trustx:question",
            where_
        );
        assert_eq!(
            count(TRUSTX_REQUIRES_VALID_STATUS_AT),
            1,
            "{}: TR must pin exactly one trustx:requiresValidStatusAt instant t — \
             the status window is a verifier parameter, never a spec constant",
            where_
        );
        assert!(
            count(TRUSTX_TRUSTS_ISSUER) + count(TRUSTX_TRUSTS_FRAMEWORK) > 0,
            "{}: TR declares NO trust mode — it would admit nothing and is refused \
             at parse time, so it can never be a conformance fixture",
            where_
        );
    }
}

/// No fixture may invent `trustx:` vocabulary the implementation does not define.
#[test]
fn every_trustx_iri_in_the_suite_is_a_real_vocabulary_term() {
    let known: BTreeSet<&str> = ALL_TRUSTX_IRIS.iter().copied().collect();
    for case in load_manifest() {
        for path in [&case.requirements, &case.data] {
            let text = read(path);
            let used = if path == &case.data {
                trustx_iris(TriGParser::new().for_reader(text.as_bytes()).map(|r| {
                    let q = r.unwrap_or_else(|e| panic!("{} must be valid TriG: {}", path.display(), e));
                    (q.subject.to_string(), q.predicate.as_str().to_string(), q.object)
                }))
            } else {
                trustx_iris(TurtleParser::new().for_reader(text.as_bytes()).map(|r| {
                    let t = r.unwrap_or_else(|e| panic!("{} must be valid Turtle: {}", path.display(), e));
                    (t.subject.to_string(), t.predicate.as_str().to_string(), t.object)
                }))
            };
            assert!(
                !used.is_empty(),
                "{}: {} uses no trustx: term at all",
                case.label(),
                path.display()
            );
            for iri in &used {
                assert!(
                    known.contains(iri.as_str()),
                    "{}: {} uses <{}>, which is NOT a term of framework_vocab::ALL_TRUSTX_IRIS \
                     — the fixture and the vocabulary have drifted",
                    case.label(),
                    path.display(),
                    iri
                );
            }
        }
    }
}

/// Design §6: zero `KNOWN_FAILING` entries. sparq passes every case in this suite,
/// so a marker appearing here (without the beaded reason the discipline requires)
/// is itself the failure.
#[test]
fn the_suite_declares_no_known_failing_case() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("suite directory is readable") {
            let path = entry.expect("readable directory entry").path();
            if path.is_dir() {
                walk(&path, hits);
            } else if std::fs::read_to_string(&path)
                .map(|t| t.contains("KNOWN_FAILING"))
                .unwrap_or(false)
            {
                hits.push(path.display().to_string());
            }
        }
    }
    let mut hits = Vec::new();
    walk(&suite_dir(), &mut hits);
    assert!(
        hits.is_empty(),
        "the trust-expression suite must carry ZERO KNOWN_FAILING entries; found in: {:?}",
        hits
    );
}
