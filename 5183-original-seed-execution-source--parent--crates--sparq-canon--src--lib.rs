//! # sparq-canon — RDFC-1.0 (RDF Dataset Canonicalization) as a public API
//!
//! [RDFC-1.0] (the W3C *RDF Dataset Canonicalization*, the URDNA2015
//! successor) gives an RDF dataset a **deterministic, blank-node-relabelled
//! canonical form**: two datasets are RDF-isomorphic iff their canonical
//! N-Quads serializations are byte-for-byte identical. That is the basis for
//! dataset hashing, signing, diffing, deduplication, and content-addressing.
//!
//! [RDFC-1.0]: https://www.w3.org/TR/rdf-canon/
//!
//! ## Opt-in by construction
//!
//! This is a **standalone, opt-in** crate. Nothing in sparq's default build or
//! the wasm artifact depends on it — `sparq-core` stays lean. Pull it in
//! explicitly (`sparq-canon = { path = "..." }`) only when you need
//! canonicalization. `sparq-zk` depends on it for the ZK commitment pipeline.
//!
//! ## The algorithm is single-sourced
//!
//! The RDFC-1.0 algorithm itself (issuer / first-degree + n-degree hashing /
//! the HNDQ recursion) is the maintained zkp-ld [`rdf_canon`] crate, validated
//! against the [W3C rdf-canon test suite] (see `tests/rdf_canon_suite.rs`).
//! `rdf_canon` speaks oxrdf 0.2 while sparq speaks oxrdf 0.3, so this crate
//! owns the single **canonical N-Quads text bridge** that every sparq consumer
//! shares: serialize 0.3 terms with their canonical `Display` form, parse with
//! oxttl 0.1 into 0.2 quads, canonicalize, parse the canonical N-Quads back.
//! N-Quads is the interchange form RDFC-1.0 is defined over, so the seam is
//! lossless by construction.
//!
//! [W3C rdf-canon test suite]: https://github.com/w3c/rdf-canon
//!
//! ## API shape
//!
//! - Dataset level (the general case): [`canonicalize`] / [`canonicalize_quads`]
//!   take quads and return the canonical N-Quads `String`;
//!   [`digest_quads_with`] returns a caller-selected digest of those exact
//!   canonical bytes; [`issued_identifiers`] / [`issue_quads`] return the
//!   canonical blank-node issuer map (input label → `c14nN`).
//! - Single graph (a default-graph-only dataset): [`canonicalize_triples`] /
//!   [`canonicalize_graph_content`] return a [`CanonicalGraph`] (the sorted
//!   canonical N-Quads lines + the re-parsed canonical triples), which is what
//!   the ZK per-graph commitment pipeline consumes.
//!
//! ## Pathological inputs
//!
//! RDFC-1.0 has worst-case blow-ups. `rdf_canon`'s HNDQ-call-limit guard is
//! kept at its default; limit hits surface as [`CanonError::Canonicalization`]
//! so a caller can fail closed on poison graphs.
//!
//! [OPUS-4.8] sq-0qip — surfaced from `sparq-zk::canon` (Fable unavailable; flag
//! for re-review when Fable returns).
//!
//! ## Opt-in NON-STANDARD RDF-1.2 triple-term profile (`rdf12-triple-terms`)
//!
//! Triple terms (`Term::Triple`, the RDF-1.2 `<<( s p o )>>` object) are
//! **outside** the W3C RDFC-1.0 data model, so the standard paths above fail
//! closed with [`CanonError::TripleTerm`]. Enabling the **opt-in, off-by-default**
//! `rdf12-triple-terms` cargo feature adds a *separate, clearly non-standard* v2
//! profile (`canonicalize_rdf12`, `canonicalize_triples_rdf12`, …) that
//! natively re-implements the RDFC-1.0 algorithm over oxrdf-0.3 and **descends
//! the Hash-N-Degree-Quads gossip into triple-term objects**, so blank nodes
//! nested inside triple terms get relabelled. It is byte-identical to the
//! standard path on triple-term-free input. This is **not** W3C RDFC-1.0;
//! W3C has published no RDF-1.2 dataset canonicalization specification. See the
//! `rdf12` module (only present with the feature on). With the feature OFF the
//! crate is byte-identical to before — the standard surface still returns
//! [`CanonError::TripleTerm`] on triple terms.
//!
//! The same feature also carries a **constrained ground-triple-term** variant
//! (`canonicalize_rdf12_ground_terms` and siblings): a thin wrapper that fails
//! closed with [`CanonError::NestedBlankNode`] if any blank node occurs inside
//! a triple term, and otherwise delegates. With every triple term ground
//! (blank-node-free — the common credential/VC case), none of the non-standard
//! nested-bnode HNDQ-descent semantics are reachable: the run is exactly the
//! RDFC-1.0 algorithm with triple terms as opaque constants. [FABLE-5] sq-iaxd.

#![forbid(unsafe_code)]

use oxrdf::{GraphName, Quad, Triple};
use oxttl::NQuadsParser;
use sparq_core::Graph;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Text-in / text-out API ([OPUS-4.8] sq-1dd5t). A self-contained N-Quads
// document is the interchange form RDFC-1.0 is defined over, so a `&str` ->
// `String` entry point is the natural seam for callers that already hold
// serialized RDF (the @sparq-org/sparq RDF/JS `Dataset` wasm binding) and do not
// want to reconstruct oxrdf term graphs across a language boundary.
// ---------------------------------------------------------------------------

/// Canonicalizes an **N-Quads document** and returns its RDFC-1.0 canonical
/// N-Quads (canonically sorted, one quad per line, blank nodes relabelled to
/// `c14nN`, each line `\n`-terminated). Two N-Quads documents that denote
/// RDF-isomorphic datasets — i.e. differ only in blank-node labels and/or quad
/// order — produce byte-identical output, which is the basis for an
/// isomorphism-aware dataset `equals` / `contains` / content hash.
///
/// `input` is parsed as oxrdf-0.3 N-Quads (the default graph is written as a
/// 3-term line; named graphs carry their graph term). RDF-1.2 triple terms are
/// outside the W3C RDFC-1.0 data model and fail closed with
/// [`CanonError::TripleTerm`] (use the opt-in `rdf12-triple-terms` profile).
///
/// ```
/// let doc = "_:b0 <http://ex/p> _:b1 .\n_:b1 <http://ex/q> \"v\" .\n";
/// let canon = sparq_canon::canonicalize_nquads(doc).unwrap();
/// assert!(canon.contains("_:c14n"));
/// // Relabelling the blank nodes yields byte-identical canonical output.
/// let doc2 = "_:x <http://ex/p> _:y .\n_:y <http://ex/q> \"v\" .\n";
/// assert_eq!(canon, sparq_canon::canonicalize_nquads(doc2).unwrap());
/// ```
pub fn canonicalize_nquads(input: &str) -> Result<String, CanonError> {
    let quads = parse_nquads_03(input)?;
    canonicalize_quads(&quads)
}

/// Parses an oxrdf-0.3 N-Quads document into [`Quad`]s (the input side of
/// [`canonicalize_nquads`]). Surfaced so a caller that needs the parsed quads
/// (e.g. to also issue identifiers) does not re-implement the parse.
pub fn parse_nquads(input: &str) -> Result<Vec<Quad>, CanonError> {
    parse_nquads_03(input)
}

fn parse_nquads_03(input: &str) -> Result<Vec<Quad>, CanonError> {
    let mut quads = Vec::new();
    for item in NQuadsParser::new().for_slice(input.as_bytes()) {
        quads.push(item.map_err(|e| CanonError::Bridge(e.to_string()))?);
    }
    Ok(quads)
}

/// **NON-STANDARD, opt-in (`rdf12-triple-terms` feature).** Native RDF-1.2
/// triple-term canonicalization profile — see the [module docs](rdf12) and the
/// crate-level banner. Not W3C RDFC-1.0.
#[cfg(feature = "rdf12-triple-terms")]
pub mod rdf12;

#[cfg(feature = "rdf12-triple-terms")]
pub use rdf12::{
    canonicalize_graph_content_rdf12, canonicalize_graph_content_rdf12_ground_terms,
    canonicalize_graph_content_rdf12_ground_terms_with, canonicalize_graph_content_rdf12_with,
    canonicalize_rdf12,
    canonicalize_rdf12_ground_terms, canonicalize_rdf12_ground_terms_with, canonicalize_rdf12_with,
    canonicalize_triples_rdf12, canonicalize_triples_rdf12_ground_terms,
    canonicalize_triples_rdf12_ground_terms_with, canonicalize_triples_rdf12_with,
    issue_dataset_rdf12, issue_dataset_rdf12_ground_terms, issue_dataset_rdf12_ground_terms_with,
    issue_dataset_rdf12_with,
};

// **Opt-in (`concept` feature).** `urn:concept:` content-addressed record
// verification — the multibase/multihash envelope plus the
// recompute-and-byte-compare ingestion guard over this crate's RDFC-1.0 digest.
// Documented entirely by its own `//!` module docs (which is also where its
// intra-doc links must resolve): an outer doc comment here would be merged into
// them and resolved in *this* module's scope instead, breaking every link to a
// `concept`-module item under `cargo doc --all-features`.
#[cfg(feature = "concept")]
pub mod concept;

/// The hash-function trait RDFC-1.0 is parameterized over (`digest::Digest`),
/// re-exported so callers of [`canonicalize_quads_with`], [`digest_quads_with`],
/// or [`issue_quads_with`] can name a hasher (e.g. `sha2::Sha256`,
/// `sha2::Sha384`) without taking a direct `digest` dependency. The spec
/// default is SHA-256 ([`canonicalize_quads`]).
pub use digest::Digest;

/// The crate-wide bound on RDF 1.2 triple-term **nesting depth**.
///
/// A triple term whose object is not itself a triple term has depth 1; each
/// additional level of `<<( … )>>`-in-`<<( … )>>` nesting adds 1. The opt-in
/// `rdf12-triple-terms` profile walks triple terms with recursive descent
/// (HNDQ gossip, bnode collection, relabelling, serialization — and `oxrdf`'s
/// own `Drop`/`Clone` recurse too), so unbounded nesting is a stack-overflow
/// vector. Every profile entry point therefore fails closed with
/// [`CanonError::TripleTermDepthExceeded`] — via an **iterative**
/// (recursion-free) pre-check — when any triple term nests deeper than this
/// bound. Real credential/VC data nests one or two levels; 128 is far above
/// any non-adversarial input. The standard (feature-off) paths reject every
/// triple term outright with [`CanonError::TripleTerm`] before any descent, so
/// the bound only ever fires in the opt-in profile. [FABLE-5] sq-x3oj2.
pub const MAX_TRIPLE_TERM_DEPTH: usize = 128;

/// Canonicalization failure (RDFC-1.0 over sparq's term model).
///
/// `#[non_exhaustive]`: variants have been added ungated as the crate grew
/// ([`CanonError::NestedBlankNode`], [`CanonError::TripleTermDepthExceeded`] —
/// per the [`CanonError::TripleTerm`] precedent), so downstream `match`es keep
/// a wildcard arm rather than assuming the set is closed. [FABLE-5] sq-x3oj2.
#[derive(Debug)]
#[non_exhaustive]
pub enum CanonError {
    /// RDF 1.2 triple terms are outside W3C RDFC-1.0's data model; the
    /// standard paths fail closed on them. Enable the opt-in `rdf12-triple-terms`
    /// feature and use the non-standard `canonicalize_rdf12` /
    /// `canonicalize_triples_rdf12` profile to canonicalize triple terms.
    TripleTerm,
    /// A blank node occurs **inside** an RDF 1.2 triple term. Returned only by
    /// the opt-in `rdf12-triple-terms` profile's constrained ground-triple-term
    /// entry points (`canonicalize_rdf12_ground_terms` and siblings), which
    /// require every triple term to be blank-node-free and fail closed
    /// otherwise. Use the full `canonicalize_rdf12` descent to relabel nested
    /// blank nodes instead. [FABLE-5] sq-iaxd.
    NestedBlankNode,
    /// An RDF 1.2 triple term nests deeper than [`MAX_TRIPLE_TERM_DEPTH`].
    /// Returned only by the opt-in `rdf12-triple-terms` profile's entry points
    /// (`canonicalize_rdf12` and siblings), which pre-check depth iteratively
    /// before any recursive descent — deeply-nested adversarial input fails
    /// closed instead of risking a stack overflow. The variant itself is always
    /// present (not feature-gated), per the `NestedBlankNode` precedent.
    /// [FABLE-5] sq-x3oj2.
    TripleTermDepthExceeded,
    /// Bridge serialization/parse failure (should not happen for RDFC-1.0-model
    /// content; surfaced rather than swallowed).
    Bridge(String),
    /// `rdf_canon` rejected the dataset (including the HNDQ poison-graph limit).
    Canonicalization(String),
}

impl std::fmt::Display for CanonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CanonError::TripleTerm => {
                write!(
                    f,
                    "RDF 1.2 triple terms are outside the W3C RDFC-1.0 data model; \
                     enable the `rdf12-triple-terms` profile to canonicalize them"
                )
            }
            CanonError::NestedBlankNode => {
                write!(
                    f,
                    "blank node nested inside an RDF 1.2 triple term; the constrained \
                     ground-triple-term profile requires blank-node-free triple terms \
                     (use the full `canonicalize_rdf12` descent to relabel nested blank nodes)"
                )
            }
            CanonError::TripleTermDepthExceeded => {
                write!(
                    f,
                    "RDF 1.2 triple term nests deeper than the crate-wide bound of \
                     {MAX_TRIPLE_TERM_DEPTH} levels; failing closed (deeply-nested \
                     triple terms are a stack-overflow vector for recursive descent)"
                )
            }
            CanonError::Bridge(e) => write!(f, "oxrdf bridge error: {e}"),
            CanonError::Canonicalization(e) => write!(f, "RDFC-1.0 canonicalization failed: {e}"),
        }
    }
}

impl std::error::Error for CanonError {}

/// A single graph's RDFC-1.0 canonical form: the sorted canonical N-Quads
/// lines (a stable total order over the triples) and the same triples
/// re-parsed with canonical blank-node labels (`c14nN`).
///
/// In the ZK commitment pipeline the index of a line is its **leaf index**.
#[derive(Debug, Clone)]
pub struct CanonicalGraph {
    /// Canonical N-Quads lines in canonical (code point) order, one triple
    /// each, no trailing newline. `lines[i]` is the canonical serialization of
    /// `triples[i]`.
    pub lines: Vec<String>,
    /// The canonical triples (blank-node labels are `c14nN`), same order as
    /// [`Self::lines`].
    pub triples: Vec<Triple>,
}

impl CanonicalGraph {
    /// The canonical N-Quads document (joined lines, each terminated by `\n`).
    pub fn to_nquads(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Dataset-level API (the general RDFC-1.0 case: a set of quads).
// ---------------------------------------------------------------------------

/// Canonicalizes an RDF dataset (a slice of [`Quad`]s) and returns its
/// RDFC-1.0 **canonical N-Quads** document (canonically sorted, one quad per
/// line, blank nodes relabelled to `c14nN`, trailing newline on each line).
///
/// Two datasets that are RDF-isomorphic produce byte-identical output; blank
/// node identifiers and quad order in the input do not affect it.
///
/// ```
/// use oxrdf::{Quad, NamedNode, NamedOrBlankNode, Term, GraphName, BlankNode};
/// let p = NamedNode::new("http://ex/p").unwrap();
/// let q = Quad::new(
///     NamedOrBlankNode::BlankNode(BlankNode::new("x").unwrap()),
///     p.clone(),
///     Term::Literal(oxrdf::Literal::new_simple_literal("v")),
///     GraphName::DefaultGraph,
/// );
/// let canon = sparq_canon::canonicalize(&[q]).unwrap();
/// assert!(canon.contains("_:c14n0"));
/// ```
pub fn canonicalize(dataset: &[Quad]) -> Result<String, CanonError> {
    canonicalize_quads(dataset)
}

/// Alias of [`canonicalize`] for callers that prefer the explicit `_quads`
/// name (mirrors [`rdf_canon::canonicalize_quads`]).
pub fn canonicalize_quads(dataset: &[Quad]) -> Result<String, CanonError> {
    let quads02 = bridge_to_02(dataset)?;
    rdf_canon::canonicalize_quads(&quads02).map_err(|e| CanonError::Canonicalization(e.to_string()))
}

/// Like [`canonicalize_quads`] but parameterized over the RDFC-1.0 hash
/// function `D` (the spec default is SHA-256; e.g. `sha2::Sha384` selects the
/// SHA-384 profile). Uses the default HNDQ call limit.
pub fn canonicalize_quads_with<D: Digest>(dataset: &[Quad]) -> Result<String, CanonError> {
    let quads02 = bridge_to_02(dataset)?;
    let opts = rdf_canon::CanonicalizationOptions::default();
    rdf_canon::canonicalize_quads_with::<D>(&quads02, &opts)
        .map_err(|e| CanonError::Canonicalization(e.to_string()))
}

/// Returns the digest bytes of the exact canonical N-Quads document produced
/// by [`canonicalize_quads`].
///
/// `D` selects only the final digest algorithm; canonicalization retains the
/// RDFC-1.0 default hash profile. Every canonical byte is hashed, including the
/// final trailing newline when the dataset is non-empty. [GPT-5.6] sq-ddws7.
pub fn digest_quads_with<D: Digest>(dataset: &[Quad]) -> Result<Vec<u8>, CanonError> {
    let c = canonicalize_quads(dataset)?;
    let mut h = D::new();
    h.update(c.as_bytes());
    Ok(h.finalize().to_vec())
}

/// Like [`issue_quads`] but parameterized over the RDFC-1.0 hash function `D`.
pub fn issue_quads_with<D: Digest>(
    dataset: &[Quad],
) -> Result<HashMap<String, String>, CanonError> {
    let quads02 = bridge_to_02(dataset)?;
    let opts = rdf_canon::CanonicalizationOptions::default();
    let map = rdf_canon::issue_quads_with::<D>(&quads02, &opts)
        .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
    Ok(map.into_iter().collect())
}

/// Returns the RDFC-1.0 **issued-identifier map** for a dataset: input
/// blank-node label → canonical `c14nN` label. Cheap relative to a full
/// canonicalization-and-reparse when only the relabelling is needed.
pub fn issued_identifiers(dataset: &[Quad]) -> Result<HashMap<String, String>, CanonError> {
    issue_quads(dataset)
}

/// Alias of [`issued_identifiers`] (mirrors [`rdf_canon::issue_quads`]).
pub fn issue_quads(dataset: &[Quad]) -> Result<HashMap<String, String>, CanonError> {
    let quads02 = bridge_to_02(dataset)?;
    let map = rdf_canon::issue_quads(&quads02)
        .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
    Ok(map.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Single-graph API (a default-graph-only dataset). What the ZK per-graph
// commitment pipeline consumes; kept here so the bridge is single-sourced.
// ---------------------------------------------------------------------------

/// Canonicalizes a slice of triples (one graph's content, treated as the
/// default graph of a single-graph dataset) into a [`CanonicalGraph`].
pub fn canonicalize_triples(triples: &[Triple]) -> Result<CanonicalGraph, CanonError> {
    for t in triples {
        if contains_triple_term(t) {
            return Err(CanonError::TripleTerm);
        }
    }
    let quads02 = bridge_triples_to_02(triples)?;
    let canonical = rdf_canon::canonicalize_quads(&quads02)
        .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
    parse_canonical(&canonical)
}

/// Canonicalizes the content of a [`sparq_core::Graph`] into a
/// [`CanonicalGraph`].
pub fn canonicalize_graph_content(g: &Graph) -> Result<CanonicalGraph, CanonError> {
    let triples = graph_triples(g)?;
    canonicalize_triples(&triples)
}

/// The RDFC-1.0 issued-identifier map for a single graph's content (input
/// blank-node label → canonical `c14nN` label).
pub fn issue_triples(triples: &[Triple]) -> Result<HashMap<String, String>, CanonError> {
    for t in triples {
        if contains_triple_term(t) {
            return Err(CanonError::TripleTerm);
        }
    }
    let quads02 = bridge_triples_to_02(triples)?;
    let map = rdf_canon::issue_quads(&quads02)
        .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
    Ok(map.into_iter().collect())
}

/// Materializes a store graph's triples as oxrdf [`Triple`]s.
pub fn graph_triples(g: &Graph) -> Result<Vec<Triple>, CanonError> {
    let mut out = Vec::with_capacity(g.len());
    for t in g.iter_ids() {
        let s = g.dict.term(t[0]);
        let p = g.dict.term(t[1]);
        let o = g.dict.term(t[2]);
        out.push(terms_to_triple(s, p, o)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Bridge internals (the single oxrdf 0.3 -> text -> oxrdf 0.2 seam).
// ---------------------------------------------------------------------------

/// Serialize oxrdf-0.3 quads to canonical N-Quads text and parse into oxrdf-0.2
/// quads for `rdf_canon`. `GraphName::Display` renders the default graph as the
/// literal word `DEFAULT`, so the default graph is emitted explicitly as a
/// 3-term line.
fn bridge_to_02(dataset: &[Quad]) -> Result<Vec<oxrdf02::Quad>, CanonError> {
    let doc = serialize_quads(dataset)?;
    parse_02(&doc)
}

#[cfg(not(feature = "bridge-lowcopy"))]
fn serialize_quads(dataset: &[Quad]) -> Result<String, CanonError> {
    serialize_quads_default(dataset)
}

#[cfg(any(not(feature = "bridge-lowcopy"), test))]
fn serialize_quads_default(dataset: &[Quad]) -> Result<String, CanonError> {
    let mut doc = String::new();
    for q in dataset {
        if matches!(q.object, oxrdf::Term::Triple(_)) {
            return Err(CanonError::TripleTerm);
        }
        match &q.graph_name {
            GraphName::DefaultGraph => {
                doc.push_str(&format!("{} {} {} .\n", q.subject, q.predicate, q.object));
            }
            g => {
                doc.push_str(&format!(
                    "{} {} {} {} .\n",
                    q.subject, q.predicate, q.object, g
                ));
            }
        }
    }
    Ok(doc)
}

/// [GPT-5.6] sq-occip: serialize directly into one reusable buffer. `String`'s
/// `fmt::Write` implementation is infallible, so the expectation cannot fire.
#[cfg(feature = "bridge-lowcopy")]
fn serialize_quads(dataset: &[Quad]) -> Result<String, CanonError> {
    use std::fmt::Write as _;

    let mut doc = String::new();
    for q in dataset {
        if matches!(q.object, oxrdf::Term::Triple(_)) {
            return Err(CanonError::TripleTerm);
        }
        match &q.graph_name {
            GraphName::DefaultGraph => {
                writeln!(doc, "{} {} {} .", q.subject, q.predicate, q.object)
                    .expect("writing to a String cannot fail");
            }
            g => {
                writeln!(doc, "{} {} {} {} .", q.subject, q.predicate, q.object, g)
                    .expect("writing to a String cannot fail");
            }
        }
    }
    Ok(doc)
}

fn bridge_triples_to_02(triples: &[Triple]) -> Result<Vec<oxrdf02::Quad>, CanonError> {
    let mut doc = String::new();
    #[cfg(feature = "bridge-lowcopy")]
    use std::fmt::Write as _;
    for t in triples {
        #[cfg(not(feature = "bridge-lowcopy"))]
        doc.push_str(&format!("{} {} {} .\n", t.subject, t.predicate, t.object));
        #[cfg(feature = "bridge-lowcopy")]
        writeln!(doc, "{} {} {} .", t.subject, t.predicate, t.object)
            .expect("writing to a String cannot fail");
    }
    parse_02(&doc)
}

fn parse_02(doc: &str) -> Result<Vec<oxrdf02::Quad>, CanonError> {
    let mut quads02 = Vec::new();
    for item in oxttl01::NQuadsParser::new().for_reader(doc.as_bytes()) {
        quads02.push(item.map_err(|e| CanonError::Bridge(e.to_string()))?);
    }
    Ok(quads02)
}

fn terms_to_triple(s: oxrdf::Term, p: oxrdf::Term, o: oxrdf::Term) -> Result<Triple, CanonError> {
    use oxrdf::Term;
    let subject = match s {
        Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
        Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
        other => return Err(CanonError::Bridge(format!("invalid subject term: {other}"))),
    };
    let predicate = match p {
        Term::NamedNode(n) => n,
        other => {
            return Err(CanonError::Bridge(format!(
                "invalid predicate term: {other}"
            )))
        }
    };
    Ok(Triple::new(subject, predicate, o))
}

fn parse_canonical(canonical: &str) -> Result<CanonicalGraph, CanonError> {
    let mut lines: Vec<String> = Vec::new();
    let mut triples: Vec<Triple> = Vec::new();
    for line in canonical.lines() {
        if line.trim().is_empty() {
            continue;
        }
        lines.push(line.to_string());
        let mut parsed = None;
        for item in NQuadsParser::new().for_slice(line.as_bytes()) {
            let q = item.map_err(|e| CanonError::Bridge(e.to_string()))?;
            parsed = Some(Triple::new(q.subject, q.predicate, q.object));
        }
        triples.push(parsed.ok_or_else(|| {
            CanonError::Bridge(format!("canonical line did not parse as one quad: {line}"))
        })?);
    }
    Ok(CanonicalGraph { lines, triples })
}

fn contains_triple_term(t: &Triple) -> bool {
    matches!(t.object, oxrdf::Term::Triple(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term};

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    /// [GPT-5.6] sq-occip: witnesses that the opt-in writer preserves every
    /// interchange byte and therefore the canonical result, including blank
    /// nodes, escaped literals, default graphs, and named graphs.
    #[cfg(feature = "bridge-lowcopy")]
    #[test]
    fn lowcopy_bridge_is_byte_identical_to_default() {
        let datasets = [
            parse_nquads_03("<http://ex/s> <http://ex/p> \"plain\" .\n").unwrap(),
            parse_nquads_03(
                "_:a <http://ex/p> _:b .\n_:b <http://ex/q> \"line\\nquote\\\"\"@en .\n",
            )
            .unwrap(),
            parse_nquads_03(
                "_:a <http://ex/p> <http://ex/o> <http://ex/g> .\n\
                 <http://ex/s> <http://ex/p> _:a <http://ex/g> .\n",
            )
            .unwrap(),
        ];

        for dataset in datasets {
            let default_doc = serialize_quads_default(&dataset).unwrap();
            let lowcopy_doc = serialize_quads(&dataset).unwrap();
            assert_eq!(lowcopy_doc.as_bytes(), default_doc.as_bytes());

            let default_quads = parse_02(&default_doc).unwrap();
            let default_canonical = rdf_canon::canonicalize_quads(&default_quads).unwrap();
            assert_eq!(canonicalize(&dataset).unwrap(), default_canonical);
        }
    }

    // [OPUS-4.8] sq-qcnn.14 — direct unit tests per public fn, asserting exact values.
    // One test per uncovered public fn; each assert is non-vacuous (flip a value → red).

    /// [FABLE-5] sq-t8z0r / GH #1903 (randomized-fuzz finding): an RDF dataset is a
    /// SET of quads, so a duplicated input line is deduplicated by RDFC-1.0
    /// canonicalization — 3 parsed lines (one repeated) canonicalize to 2 quads.
    /// Documents the set semantics the fuzz-harness invariant must respect.
    #[test]
    fn canonicalize_nquads_deduplicates_repeated_quads() {
        let doc = "_:b0 <http://ex/p> \"x\" .\n<http://ex/s> <http://ex/p> \"y\" .\n_:b0 <http://ex/p> \"x\" .\n";
        assert_eq!(parse_nquads(doc).unwrap().len(), 3, "the parse itself keeps duplicates");
        let canon = canonicalize_nquads(doc).unwrap();
        let canon_quads = parse_nquads(&canon).unwrap();
        assert_eq!(canon_quads.len(), 2, "canonical form is a set: the duplicate collapses");
        // Idempotence: re-canonicalizing the canonical form is a fixed point.
        assert_eq!(canonicalize_nquads(&canon).unwrap(), canon);
    }

    /// Direct test for `parse_nquads` (public wrapper). Asserts exact quad content,
    /// killing "return empty vec" / "skip error propagation" mutation classes.
    #[test]
    fn parse_nquads_exact_content() {
        let doc = "<http://ex/s> <http://ex/p> \"hello\" .\n";
        let quads = parse_nquads(doc).unwrap();
        assert_eq!(quads.len(), 1, "one line -> one quad");
        assert_eq!(quads[0].subject.to_string(), "<http://ex/s>");
        assert_eq!(quads[0].predicate.to_string(), "<http://ex/p>");
        assert_eq!(quads[0].object.to_string(), "\"hello\"");
        // Multiple quads: ensure each is returned.
        let two = "<http://ex/s> <http://ex/p> <http://ex/a> .\n\
                   <http://ex/s> <http://ex/q> <http://ex/b> .\n";
        assert_eq!(parse_nquads(two).unwrap().len(), 2, "two lines -> two quads");
    }

    /// `parse_nquads` error path: malformed N-Quads must return `CanonError::Bridge`,
    /// not silently succeed. Kills the "always return Ok" mutation.
    #[test]
    fn parse_nquads_rejects_malformed() {
        let result = parse_nquads("not n-quads !!!\n");
        assert!(
            matches!(result, Err(CanonError::Bridge(_))),
            "malformed input must return Bridge error, got: {:?}",
            result
        );
    }

    /// Direct test for `issue_quads_with` (hash-generic issuer, SHA-384 profile).
    /// Asserts the EXACT issued label: "c14n0" for a single bnode. Kills mutations
    /// that change the prefix, omit the counter, or skip inserting into the map.
    #[test]
    fn issue_quads_with_sha384_exact_label() {
        use sha2::Sha384;
        let q = Quad::new(
            NamedOrBlankNode::BlankNode(BlankNode::new("b0").unwrap()),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("val")),
            GraphName::DefaultGraph,
        );
        let map = issue_quads_with::<Sha384>(&[q]).unwrap();
        assert_eq!(map.len(), 1, "one bnode -> one issued id");
        assert_eq!(
            map.get("b0").map(String::as_str),
            Some("c14n0"),
            "single bnode must issue as c14n0 under SHA-384: {map:?}"
        );
    }

    /// Direct test for `canonicalize_quads_with` (hash-generic, SHA-384 profile).
    /// Asserts exact structural properties so mutations to the format string or
    /// the hash dispatch are killed.
    #[test]
    fn canonicalize_quads_with_sha384_canonical_form() {
        use sha2::Sha384;
        let q = Quad::new(
            NamedOrBlankNode::BlankNode(BlankNode::new("b0").unwrap()),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("val")),
            GraphName::DefaultGraph,
        );
        let canon = canonicalize_quads_with::<Sha384>(&[q]).unwrap();
        assert!(
            canon.contains("_:c14n0"),
            "bnode must be relabelled to c14n0: {canon:?}"
        );
        assert!(
            canon.contains("<http://ex/p>"),
            "predicate must appear in canonical output: {canon:?}"
        );
        assert!(
            canon.ends_with('\n'),
            "canonical doc must end with newline: {canon:?}"
        );
        // Isomorphism: relabelling the bnode gives byte-identical output.
        let q2 = Quad::new(
            NamedOrBlankNode::BlankNode(BlankNode::new("other").unwrap()),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("val")),
            GraphName::DefaultGraph,
        );
        assert_eq!(
            canon,
            canonicalize_quads_with::<Sha384>(&[q2]).unwrap(),
            "isomorphic datasets must agree under SHA-384"
        );
    }

    /// Direct test for `issue_triples` (single-graph, SHA-256 issuer map).
    /// Asserts the EXACT issued label "c14n0". Kills bnode-label prefix mutations.
    #[test]
    fn issue_triples_exact_label() {
        let t = Triple::new(
            NamedOrBlankNode::BlankNode(BlankNode::new("x").unwrap()),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
        );
        let map = issue_triples(&[t]).unwrap();
        assert_eq!(map.len(), 1, "one bnode -> one issued id");
        assert_eq!(
            map.get("x").map(String::as_str),
            Some("c14n0"),
            "single bnode must issue as c14n0: {map:?}"
        );
    }

    /// `issue_triples` must reject triple-term objects (same guard as `canonicalize_triples`).
    /// Kills the "skip the triple-term check" mutation class.
    #[test]
    fn issue_triples_rejects_triple_terms() {
        let inner = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
        );
        let t = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/a")),
            iri("http://ex/asserts"),
            Term::Triple(Box::new(inner)),
        );
        assert!(
            matches!(issue_triples(&[t]), Err(CanonError::TripleTerm)),
            "`issue_triples` must fail closed on triple-term objects"
        );
    }

    /// Direct test for `graph_triples` — materializes a `sparq_core::Graph`'s triples
    /// as oxrdf Terms, asserting EXACT count + each term. Kills "return empty vec" and
    /// "skip one term" mutation classes.
    #[test]
    fn graph_triples_materializes_exact_terms() {
        use sparq_core::Graph;
        let g = Graph::load_str(
            "<http://ex/s> <http://ex/p> <http://ex/o> .\n",
            "ntriples",
        )
        .expect("load_str");
        let triples = graph_triples(&g).unwrap();
        assert_eq!(triples.len(), 1, "one stored triple -> one materialized triple");
        let t = &triples[0];
        assert_eq!(t.subject.to_string(), "<http://ex/s>", "subject preserved");
        assert_eq!(t.predicate.to_string(), "<http://ex/p>", "predicate preserved");
        assert_eq!(t.object.to_string(), "<http://ex/o>", "object preserved");
    }

    /// Direct test for `canonicalize_graph_content` — the ZK pipeline's per-graph entry
    /// point over a `sparq_core::Graph`. Asserts bnode relabelling and exact line count,
    /// so mutations to the bnode-relabelling logic or the graph→triples extraction are killed.
    #[test]
    fn canonicalize_graph_content_relabels_bnode() {
        use sparq_core::Graph;
        let g = Graph::load_str(
            "_:b0 <http://ex/p> <http://ex/o> .\n",
            "ntriples",
        )
        .expect("load_str");
        let c = canonicalize_graph_content(&g).unwrap();
        assert_eq!(c.lines.len(), 1, "one stored triple -> one canonical line");
        assert!(
            c.lines[0].contains("_:c14n0"),
            "bnode must be relabelled to c14n0: {:?}",
            c.lines[0]
        );
        assert!(
            c.lines[0].contains("<http://ex/p>"),
            "predicate must be preserved: {:?}",
            c.lines[0]
        );
        // The `triples` slice must match the canonical line (re-parsed canonical triple).
        assert_eq!(c.triples.len(), 1, "one line -> one canonical triple");
    }

    /// `to_nquads` on a multi-triple graph must join ALL lines, each newline-terminated.
    /// Kills mutations that omit `push('\n')`, or return only the first line.
    #[test]
    fn to_nquads_multi_line_joins_all_lines() {
        let b1 = BlankNode::new("x").unwrap();
        let b2 = BlankNode::new("y").unwrap();
        let t1 = Triple::new(
            NamedOrBlankNode::BlankNode(b1),
            iri("http://ex/p"),
            Term::BlankNode(b2.clone()),
        );
        let t2 = Triple::new(
            NamedOrBlankNode::BlankNode(b2),
            iri("http://ex/q"),
            Term::Literal(Literal::new_simple_literal("leaf")),
        );
        let c = canonicalize_triples(&[t1, t2]).unwrap();
        assert_eq!(c.lines.len(), 2, "two triples -> two canonical lines");
        let doc = c.to_nquads();
        let expected = format!("{}\n{}\n", c.lines[0], c.lines[1]);
        assert_eq!(doc, expected, "to_nquads must join all lines with newlines");
        // Each line must appear in order and be newline-terminated.
        let parts: Vec<&str> = doc.lines().collect();
        assert_eq!(parts.len(), 2, "two lines in doc");
        assert_eq!(parts[0], c.lines[0]);
        assert_eq!(parts[1], c.lines[1]);
    }

    #[test]
    fn canonical_labels_and_order() {
        let b1 = BlankNode::new("zzz").unwrap();
        let b2 = BlankNode::new("aaa").unwrap();
        let t1 = Triple::new(
            NamedOrBlankNode::BlankNode(b1.clone()),
            iri("http://ex/p"),
            Term::BlankNode(b2.clone()),
        );
        let t2 = Triple::new(
            NamedOrBlankNode::BlankNode(b2),
            iri("http://ex/q"),
            Term::Literal(Literal::new_simple_literal("x")),
        );
        let c = canonicalize_triples(&[t1, t2]).unwrap();
        assert_eq!(c.lines.len(), 2);
        let mut sorted = c.lines.clone();
        sorted.sort();
        assert_eq!(c.lines, sorted, "canonical lines must be sorted");
        assert!(c.lines.iter().all(|l| l.contains("_:c14n")));
    }

    /// Blank-node relabelling invariance: the same graph under different input
    /// bnode labels and a permuted triple order canonicalizes identically.
    #[test]
    fn relabelling_and_order_invariance() {
        let make = |l1: &str, l2: &str, swap: bool| -> CanonicalGraph {
            let b1 = BlankNode::new(l1).unwrap();
            let b2 = BlankNode::new(l2).unwrap();
            let t1 = Triple::new(
                NamedOrBlankNode::BlankNode(b1),
                iri("http://ex/p"),
                Term::BlankNode(b2.clone()),
            );
            let t2 = Triple::new(
                NamedOrBlankNode::BlankNode(b2),
                iri("http://ex/q"),
                Term::Literal(Literal::new_simple_literal("x")),
            );
            let triples = if swap { vec![t2, t1] } else { vec![t1, t2] };
            canonicalize_triples(&triples).unwrap()
        };
        let a = make("zzz", "aaa", false);
        let b = make("other1", "other2", true);
        assert_eq!(
            a.lines, b.lines,
            "isomorphic graphs must canonicalize identically"
        );
    }

    /// Dataset-level API: a named-graph quad round-trips through the bridge and
    /// the canonical form carries the graph name.
    #[test]
    fn dataset_named_graph() {
        let g = GraphName::NamedNode(iri("http://ex/g"));
        let q = Quad::new(
            NamedOrBlankNode::BlankNode(BlankNode::new("x").unwrap()),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
            g,
        );
        let canon = canonicalize(std::slice::from_ref(&q)).unwrap();
        assert!(
            canon.contains("<http://ex/g>"),
            "graph name preserved: {canon}"
        );
        assert!(canon.contains("_:c14n0"));
        // Issuer map relabels the one bnode.
        let map = issued_identifiers(&[q]).unwrap();
        assert_eq!(map.get("x").map(String::as_str), Some("c14n0"));
    }

    #[test]
    fn rejects_triple_terms() {
        let inner = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
        );
        let t = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::Triple(Box::new(inner)),
        );
        assert!(matches!(
            canonicalize_triples(&[t]),
            Err(CanonError::TripleTerm)
        ));
    }

    /// [OPUS-4.8] sq-1dd5t: the text-in/text-out `canonicalize_nquads` entry point
    /// (what the @sparq-org/sparq RDF/JS `Dataset` wasm binding calls). Two N-Quads
    /// documents that are RDF-isomorphic but differ in blank-node labels AND quad
    /// order canonicalize to byte-identical output; a label-only diff that is NOT
    /// isomorphic (an extra edge) does not.
    #[test]
    fn canonicalize_nquads_isomorphism() {
        let a = "_:b0 <http://ex/p> _:b1 .\n_:b1 <http://ex/q> \"v\" .\n";
        // Relabelled blank nodes + reordered lines — same dataset up to isomorphism.
        let b = "_:y <http://ex/q> \"v\" .\n_:x <http://ex/p> _:y .\n";
        let ca = canonicalize_nquads(a).unwrap();
        let cb = canonicalize_nquads(b).unwrap();
        assert_eq!(ca, cb, "isomorphic N-Quads must canonicalize identically");
        assert!(
            ca.contains("_:c14n"),
            "blank nodes relabelled to c14nN: {ca}"
        );

        // A genuinely different graph (extra edge) must NOT match.
        let c = "_:x <http://ex/p> _:y .\n_:y <http://ex/q> \"v\" .\n_:x <http://ex/r> \"w\" .\n";
        assert_ne!(ca, canonicalize_nquads(c).unwrap());
    }

    /// Named graphs round-trip through the text bridge (the graph term is carried).
    #[test]
    fn canonicalize_nquads_named_graph() {
        let doc = "_:b0 <http://ex/p> \"v\" <http://ex/g> .\n";
        let c = canonicalize_nquads(doc).unwrap();
        assert!(c.contains("<http://ex/g>"), "graph name preserved: {c}");
        assert!(c.contains("_:c14n0"));
    }

    /// An empty document canonicalizes to the empty string (the empty dataset).
    #[test]
    fn canonicalize_nquads_empty() {
        assert_eq!(canonicalize_nquads("").unwrap(), "");
        assert_eq!(canonicalize_nquads("\n\n").unwrap(), "");
    }

    /// [FABLE-5] sq-iaxd: `CanonError::NestedBlankNode` is an ungated variant
    /// (only *constructed* by the opt-in `rdf12-triple-terms` ground-terms
    /// wrappers), so its `Display` must render in the default feature state
    /// too. Pins the message's two load-bearing halves: the condition and the
    /// full-descent escape hatch.
    #[test]
    fn nested_blank_node_error_display_default_features() {
        let msg = CanonError::NestedBlankNode.to_string();
        assert!(
            msg.contains("blank node nested inside an RDF 1.2 triple term"),
            "Display must name the condition: {msg:?}"
        );
        assert!(
            msg.contains("canonicalize_rdf12"),
            "Display must point at the full-descent escape hatch: {msg:?}"
        );
    }

    /// [FABLE-5] sq-x3oj2: `CanonError::TripleTermDepthExceeded` is likewise an
    /// ungated variant (only *constructed* by the opt-in `rdf12-triple-terms`
    /// profile's iterative depth pre-check), so its `Display` must render in
    /// the default feature state too — and must carry the actual crate-wide
    /// bound, so the message can never drift from [`MAX_TRIPLE_TERM_DEPTH`].
    #[test]
    fn triple_term_depth_error_display_default_features() {
        let msg = CanonError::TripleTermDepthExceeded.to_string();
        assert!(
            msg.contains("nests deeper"),
            "Display must name the condition: {msg:?}"
        );
        assert!(
            msg.contains(&MAX_TRIPLE_TERM_DEPTH.to_string()),
            "Display must carry the crate-wide bound ({MAX_TRIPLE_TERM_DEPTH}): {msg:?}"
        );
        // The bound itself is a load-bearing public constant: deep enough for
        // any real credential/VC nesting, small enough that bounded recursive
        // descent cannot overflow the stack.
        assert_eq!(MAX_TRIPLE_TERM_DEPTH, 128, "crate-wide depth bound");
    }

    #[test]
    fn to_nquads_round_trips_lines() {
        let t = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
        );
        let c = canonicalize_triples(&[t]).unwrap();
        assert_eq!(c.to_nquads(), format!("{}\n", c.lines[0]));
    }
}
