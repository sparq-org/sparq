//! Shared loader for the trust-expression conformance manifest
//! (`tests/trust-expression/manifest.ttl`; design record
//! `research/trust-expression-spec.md` §6, bead `sq-6syab.3`).
//!
//! Included by BOTH suite drivers via `#[path]` — `trust_expression_fixtures.rs`
//! (the data's well-formedness guard, behind `framework-vocab`) and
//! `trust_expression_conformance.rs` (the semantics runner, behind `expression`)
//! — so neither can drift from the other's reading of the manifest.
//!
//! Cargo compiles only the top-level `tests/*.rs` files as test targets, so this
//! subdirectory module is never built as a test binary of its own.
//!
//! The manifest is standard W3C `mf:`/`qt:` vocabulary over relative IRIs against
//! an `@base`, so this loader resolves an entry's file references by stripping
//! [`BASE`] back off — keeping the directory liftable verbatim into the
//! specification's upstream home (design §9.3).
//!
//! [SONNET-4.6] sq-6syab.3 (epic sq-6syab; issue #1592). 🤖 SPARQ agent —
//! trust-expression conformance suite.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use oxrdf::Term;
use oxttl::TurtleParser;

/// The manifest's `@base`. Entry and file IRIs resolve against it; this loader
/// strips it back off to recover the on-disk relative path.
pub const BASE: &str = "https://sparq.dev/tests/trust-expression/";

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const QT: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-query#";
const TEC: &str = "https://sparq.dev/ns/trust-expression-conformance#";

/// The outcome the specification requires of a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// The contract binds: the answer is `true` / at least one row, and the
    /// response discloses the contributing statements with their provenance.
    Admitted,
    /// Fail-closed: no admissible derivation, so no binding — `false` / zero rows
    /// AND a response carrying zero bundles. Never a derived denial.
    NoBinding,
}

/// One manifest entry, resolved to on-disk paths.
#[derive(Debug, Clone)]
pub struct Case {
    /// The entry IRI relative to [`BASE`] (e.g. `#mode1-pass`).
    pub id: String,
    /// `mf:name`.
    pub name: String,
    /// The design §6 case class this entry realises (1–8).
    pub case_class: u32,
    /// `qt:query` — the SPARQL query `Q`.
    pub query: PathBuf,
    /// `qt:data` — the holder's attested dataset (TriG).
    pub data: PathBuf,
    /// `tec:requirements` — the trust-requirements document `TR` (Turtle).
    pub requirements: PathBuf,
    /// `tec:nonce` — the verifier's challenge nonce.
    pub nonce: String,
    /// `mf:result`.
    pub expect: Expect,
    /// `tec:contributingStatements` — the exact number of statements the response
    /// must disclose (0 for every fail-closed case).
    pub contributing: usize,
}

impl Case {
    /// A human-readable label for assertion messages.
    pub fn label(&self) -> String {
        format!(
            "{} [§6 case class {}] — {}",
            self.id, self.case_class, self.name
        )
    }
}

/// The suite directory (`crates/sparq-trust/tests/trust-expression`).
pub fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/trust-expression")
}

type Store = BTreeMap<String, Vec<(String, Term)>>;

fn objects(store: &Store, subject: &str, predicate: &str) -> Vec<Term> {
    store
        .get(subject)
        .map(|po| {
            po.iter()
                .filter(|(p, _)| p == predicate)
                .map(|(_, o)| o.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn object(store: &Store, subject: &str, predicate: &str) -> Term {
    let found = objects(store, subject, predicate);
    assert_eq!(
        found.len(),
        1,
        "manifest.ttl: expected exactly one <{}> on {}, found {}",
        predicate,
        subject,
        found.len()
    );
    found.into_iter().next().expect("length checked above")
}

fn iri_of(term: &Term, what: &str) -> String {
    match term {
        Term::NamedNode(n) => n.as_str().to_string(),
        other => panic!("manifest.ttl: {} must be an IRI, found {}", what, other),
    }
}

fn text_of(term: &Term, what: &str) -> String {
    match term {
        Term::Literal(l) => l.value().to_string(),
        other => panic!("manifest.ttl: {} must be a literal, found {}", what, other),
    }
}

/// Resolve a file IRI back to its on-disk path under the suite directory.
fn relative(term: &Term, what: &str) -> PathBuf {
    let iri = iri_of(term, what);
    let rel = iri.strip_prefix(BASE).unwrap_or_else(|| {
        panic!(
            "manifest.ttl: {} ({}) must be relative to the manifest @base {}",
            what, iri, BASE
        )
    });
    suite_dir().join(rel)
}

/// Walk an `rdf:List` from its head term.
fn list(store: &Store, head: &Term) -> Vec<Term> {
    let nil = format!("<{}nil>", RDF);
    let mut out = Vec::new();
    let mut cursor = head.clone();
    while cursor.to_string() != nil {
        let key = cursor.to_string();
        out.push(object(store, &key, &format!("{}first", RDF)));
        cursor = object(store, &key, &format!("{}rest", RDF));
        assert!(
            out.len() < 1024,
            "manifest.ttl: mf:entries list is not well-formed (no rdf:nil terminator)"
        );
    }
    out
}

/// Parse `manifest.ttl` and resolve every entry. Panics with a precise message on
/// any malformation — a conformance manifest that does not parse is a suite bug,
/// never a silently-skipped case.
pub fn load_manifest() -> Vec<Case> {
    let path = suite_dir().join("manifest.ttl");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));

    let mut store: Store = BTreeMap::new();
    for result in TurtleParser::new().for_reader(text.as_bytes()) {
        let triple = result.expect("manifest.ttl must be valid Turtle");
        store
            .entry(triple.subject.to_string())
            .or_default()
            .push((triple.predicate.as_str().to_string(), triple.object));
    }

    let rdf_type = format!("{}type", RDF);
    let manifest_class = Term::NamedNode(oxrdf::NamedNode::new_unchecked(format!("{}Manifest", MF)));
    let manifests: Vec<String> = store
        .iter()
        .filter(|(_, po)| po.iter().any(|(p, o)| *p == rdf_type && *o == manifest_class))
        .map(|(s, _)| s.clone())
        .collect();
    assert_eq!(
        manifests.len(),
        1,
        "manifest.ttl must declare exactly one mf:Manifest, found {}",
        manifests.len()
    );

    let head = object(&store, &manifests[0], &format!("{}entries", MF));
    let entries = list(&store, &head);
    assert!(
        !entries.is_empty(),
        "manifest.ttl declares an EMPTY mf:entries list — the suite would pass vacuously"
    );

    entries
        .iter()
        .map(|entry| {
            let entry_iri = iri_of(entry, "an mf:entries member");
            let id = entry_iri
                .strip_prefix(BASE)
                .unwrap_or(&entry_iri)
                .to_string();
            let key = entry.to_string();
            let action = object(&store, &key, &format!("{}action", MF)).to_string();
            let expect = match iri_of(
                &object(&store, &key, &format!("{}result", MF)),
                "mf:result",
            )
            .as_str()
            {
                x if x == format!("{}Admitted", TEC) => Expect::Admitted,
                x if x == format!("{}NoBinding", TEC) => Expect::NoBinding,
                other => panic!("manifest.ttl: {} has an unknown mf:result <{}>", id, other),
            };
            let class_text = text_of(
                &object(&store, &key, &format!("{}caseClass", TEC)),
                "tec:caseClass",
            );
            let contributing_text = text_of(
                &object(&store, &key, &format!("{}contributingStatements", TEC)),
                "tec:contributingStatements",
            );
            Case {
                name: text_of(&object(&store, &key, &format!("{}name", MF)), "mf:name"),
                case_class: class_text
                    .parse()
                    .unwrap_or_else(|_| panic!("manifest.ttl: {} has a non-integer tec:caseClass", id)),
                query: relative(&object(&store, &action, &format!("{}query", QT)), "qt:query"),
                data: relative(&object(&store, &action, &format!("{}data", QT)), "qt:data"),
                requirements: relative(
                    &object(&store, &action, &format!("{}requirements", TEC)),
                    "tec:requirements",
                ),
                nonce: text_of(
                    &object(&store, &action, &format!("{}nonce", TEC)),
                    "tec:nonce",
                ),
                expect,
                contributing: contributing_text.parse().unwrap_or_else(|_| {
                    panic!("manifest.ttl: {} has a non-integer tec:contributingStatements", id)
                }),
                id,
            }
        })
        .collect()
}
