//! # NON-STANDARD RDF-1.2 triple-term canonicalization profile (`rdf12-triple-terms`)
//!
//! **⚠️ NON-STANDARD. This is NOT W3C RDFC-1.0.** [RDFC-1.0] is defined for the
//! RDF-1.1 data model only; it has **no** notion of RDF-1.2 triple terms
//! (`<<( s p o )>>` as an object). W3C has published no RDF-1.2 dataset
//! canonicalization specification, so the profile here is sparq's own opt-in
//! extension. The standard
//! [`crate::canonicalize`] / [`crate::canonicalize_triples`] paths, and the W3C
//! `rdf-canon` test suite, keep meaning **exactly** what they mean today: they
//! still fail closed with [`CanonError::TripleTerm`] on any triple-term input.
//!
//! [RDFC-1.0]: https://www.w3.org/TR/rdf-canon/
//!
//! ## What it does
//!
//! It is a **native** re-implementation of the RDFC-1.0 algorithm (issuer state,
//! Hash First Degree Quads, Hash Related Blank Node, **Hash N-Degree Quads**)
//! directly over sparq's oxrdf-0.3 term model, **extended** so the n-degree
//! gossip descends through [`oxrdf::Term::Triple`] objects: a blank node nested
//! inside a triple term (at any depth) is enrolled, gossiped, and relabelled
//! exactly like a top-level blank node. The serialization re-uses oxrdf-0.3's
//! canonical `Display` (the `<<( … )>>` triple-term token form and the
//! `"…"@lang--dir` directional-language form of RDF-1.2 N-Quads), so the token
//! rules are single-sourced in oxttl/oxrdf rather than hand-rolled here.
//!
//! Because the algorithm is structurally the RDFC-1.0 algorithm, on **any input
//! without triple terms** it produces byte-identical output to the standard
//! `rdf-canon`-backed path; that agreement is the strongest correctness anchor
//! and is asserted in the crate's tests against the W3C suite vectors.
//!
//! ## Triple terms only appear as objects
//!
//! In oxrdf 0.3 a [`Triple`]'s subject is a `NamedOrBlankNode` and only its
//! object may be a `Term::Triple`. So nesting descends strictly through the
//! **object** position; a nested blank node is, positionally, part of the
//! **object** of the top-level quad that contains the triple term, and is
//! gossiped with the object position marker `o` against that quad's predicate.
//!
//! That is an **upstream** invariant, not a choice made here, and every part of
//! the descent depends on it *silently*. If a future oxrdf revision admitted a
//! triple term in subject position, the code below would keep compiling and
//! start producing a **wrong canonical form**: `relabel_subject` and
//! `relabel_subject_canonical` would clone the subject through unchanged,
//! leaving blank nodes nested under it with their *input* labels;
//! `collect_bnodes_subject` would not enrol them, so they would never be
//! gossiped or issued a `c14nN`; and `triple_term_depth`'s object-only chain
//! walk would under-count, weakening the stack-overflow bound. So every
//! subject-position handler routes through `subject_bnode` or
//! `subject_nesting_depth`, whose matches are **exhaustive on purpose** (no
//! `_` arm): a new subject variant upstream makes this module fail to COMPILE,
//! at exactly the sites that must then be extended, instead of failing
//! silently. [OPUS-5] sq-tx21.
//!
//! ## Boundary
//!
//! - SHA-256 is the default (the spec hash). A `*_with::<D: Digest>` profile
//!   (e.g. [`canonicalize_rdf12_with`]) lets a caller select a different hash —
//!   notably SHA-384 (`sha2::Sha384`) for parity with the standard delegated
//!   path's [`crate::canonicalize_quads_with`]. The non-generic entry points
//!   are SHA-256 and produce byte-identical output to before. Because all
//!   intermediate RDFC-1.0 hashes are emitted as lowercase hex of the full
//!   digest, the chosen `D` flows through first-degree, related, and n-degree
//!   hashing uniformly; the canonical *labels* (`c14nN`) and their *order* are
//!   determined by hash comparison, so a different `D` can yield a different
//!   relabelling while every input remains isomorphism-stable under that `D`.
//! - Dataset-level (quads, named graphs) and single-graph (triples) entry
//!   points are both provided, each with a `*_with` hash-generic sibling.
//! - The HNDQ poison-graph call limit is enforced (default 4000) and surfaces as
//!   [`CanonError::Canonicalization`], so pathological inputs fail closed.
//!
//! ## Constrained ground-triple-term variant (no nested blank nodes)
//!
//! [`canonicalize_rdf12_ground_terms`] (and its `issue_dataset` /
//! `canonicalize_triples` / `*_with` siblings) is a **thin wrapper** over the
//! full profile that first checks every triple term is **ground** — contains
//! no blank node at any nesting depth — and fails closed with
//! [`CanonError::NestedBlankNode`] otherwise. This is the common
//! credential/VC shape (asserted/quoted statements about ground content), and
//! it is semantically the *least* adventurous slice of this profile: with all
//! triple terms ground, the nested-bnode HNDQ-descent extension is
//! unreachable (the descent finds nothing to enrol or gossip), so the run is
//! exactly the RDFC-1.0 algorithm over an alphabet extended with opaque
//! triple-term constants. A caller that wants a canonical form whose
//! well-definedness does **not** rest on the novel nested-bnode gossip design
//! uses these entry points; nested-bnode inputs are rejected rather than
//! silently canonicalized under the extension. Top-level blank nodes (quad
//! subject/object/graph) are ordinary RDFC-1.0 bnodes and remain fine. Still
//! NON-STANDARD: the serialization carries RDF-1.2 `<<( … )>>` tokens, which
//! W3C RDFC-1.0 has no notion of.
//!
//! ## Canonical-token tracking (sq-g6b6)
//!
//! Because the token rules above are single-sourced from oxrdf/oxttl, a
//! serializer bump (or a canonical-escaping change once W3C rdf12-n-quads is
//! final) would silently change every canonical document this profile emits.
//! `tests/rdf12_nquads_token_tracking.rs` pins the resolved oxrdf/oxttl
//! serializer versions and asserts the token edge cases byte-exactly, so any
//! such change fails loudly with re-verification instructions.
//!
//! Status checked 2026-08-01 (issue #3455): W3C rdf12-n-quads is a **Working
//! Draft** (23 July 2026) and has never been published past Working Draft, so
//! the "once final" wording above is current, not stale. The byte-exact
//! expectations were compared against that draft's grammar and all match it;
//! the rule-by-rule comparison table, and the two notes it surfaced to carry
//! forward to the REC re-check, are recorded in the test file's module docs.
//!
//! [OPUS-4.8] sq-hslb — full non-standard RDF-1.2 triple-term canon profile;
//! sq-5i1d — `*_with::<D: Digest>` hash-profile parity (SHA-384).
//! Fable unavailable; flag for re-review when Fable returns.
//! [FABLE-5] sq-iaxd — constrained ground-triple-term (no-nested-bnode) variant.
//! [FABLE-5] sq-g6b6 — canonical-token tracking suite (serializer version pin +
//! byte-exact token expectations; re-verify once W3C rdf12-n-quads is final).

use crate::CanonError;
use digest::Digest;
use oxrdf::{BlankNode, GraphName, NamedOrBlankNode, Quad, Term, Triple};
use sha2::Sha256;
use std::collections::BTreeMap;

/// The default HNDQ call limit (matches the standard `rdf-canon` path's guard).
const DEFAULT_HNDQ_CALL_LIMIT: usize = 4000;

// ---------------------------------------------------------------------------
// Subject-position tripwires ([OPUS-5] sq-tx21). Every read of a triple's
// subject in this module goes through one of these two functions, and both
// match EXHAUSTIVELY over `NamedOrBlankNode` — deliberately WITHOUT a `_` arm,
// even though a catch-all would be shorter. That is the point: the descent's
// correctness rests on the upstream fact that a subject cannot itself be a
// triple term (see the module docs), and a catch-all would absorb a new
// upstream variant into "not a blank node / contributes no nesting", which is
// exactly the wrong answer. Kept exhaustive, a future oxrdf revision that
// admits a subject-position triple term turns that silent miscanonicalization
// into a compile error here.
// ---------------------------------------------------------------------------

/// The blank node occupying `subject`, or `None` if it is an IRI.
///
/// **Tripwire — see the [module docs](self) and the section comment above.** If
/// this stops compiling because `NamedOrBlankNode` gained a triple-term
/// variant, the fix is not to add a `_` arm: it is to extend the descent to
/// subject-position triple terms — enrol nested blank nodes in
/// `collect_bnodes_subject`, relabel through them in [`relabel_subject`] and
/// [`relabel_subject_canonical`], and gossip them with the `s` position marker
/// (RDFC-1.0 §4.7) — and to add the matching isomorphism tests.
fn subject_bnode(subject: &NamedOrBlankNode) -> Option<&BlankNode> {
    match subject {
        NamedOrBlankNode::NamedNode(_) => None,
        NamedOrBlankNode::BlankNode(b) => Some(b),
    }
}

/// The triple-term nesting depth contributed by a **subject**: zero for every
/// subject kind oxrdf 0.3 admits, which is what makes [`triple_term_depth`]'s
/// single-chain walk down the object position a correct depth measure rather
/// than an under-count of a tree.
///
/// **Tripwire**, on the same terms as [`subject_bnode`]: if a subject can nest,
/// the depth of a triple term becomes `1 + max(subject_depth, object_depth)`
/// and the loop in [`triple_term_depth`] must become a (still iterative) tree
/// walk before the stack-overflow bound means anything again.
fn subject_nesting_depth(subject: &NamedOrBlankNode) -> usize {
    match subject {
        NamedOrBlankNode::NamedNode(_) | NamedOrBlankNode::BlankNode(_) => 0,
    }
}

// ---------------------------------------------------------------------------
// Public entry points (the v2, non-standard profile).
// ---------------------------------------------------------------------------

/// **NON-STANDARD.** Canonicalizes an RDF-1.2 dataset (a slice of [`Quad`]s),
/// **including triple terms**, into its canonical N-Quads document under the
/// sparq `rdf12-triple-terms` profile (SHA-256).
///
/// On triple-term-free input this is byte-identical to the standard
/// [`crate::canonicalize`]. With triple terms it additionally relabels blank
/// nodes nested inside triple-term objects via the HNDQ descent.
///
/// This is **not** W3C RDFC-1.0; W3C has published no RDF-1.2 dataset
/// canonicalization specification. See the [module docs](self).
pub fn canonicalize_rdf12(dataset: &[Quad]) -> Result<String, CanonError> {
    canonicalize_rdf12_with::<Sha256>(dataset)
}

/// **NON-STANDARD.** Like [`canonicalize_rdf12`] but parameterized over the
/// RDFC-1.0 hash function `D` ([`crate::Digest`]). The profile default is
/// SHA-256 ([`canonicalize_rdf12`]); pass `sha2::Sha384` to select the SHA-384
/// profile, for parity with the standard delegated path's
/// [`crate::canonicalize_quads_with`].
///
/// The relabelling and line order are determined by hash comparison, so a
/// different `D` may produce a different (but still canonical and
/// isomorphism-stable) `c14nN` assignment; the SHA-256 default
/// ([`canonicalize_rdf12`]) is byte-identical to before.
pub fn canonicalize_rdf12_with<D: Digest>(dataset: &[Quad]) -> Result<String, CanonError> {
    let issued = issue_dataset_rdf12_with::<D>(dataset)?;
    Ok(serialize_canonical(dataset, &issued))
}

/// **NON-STANDARD.** The issued-identifier map (input blank-node label →
/// canonical `c14nN` label) for an RDF-1.2 dataset under the
/// `rdf12-triple-terms` profile (SHA-256). See [`canonicalize_rdf12`].
pub fn issue_dataset_rdf12(
    dataset: &[Quad],
) -> Result<std::collections::HashMap<String, String>, CanonError> {
    issue_dataset_rdf12_with::<Sha256>(dataset)
}

/// **NON-STANDARD.** Like [`issue_dataset_rdf12`] but parameterized over the
/// RDFC-1.0 hash function `D` ([`crate::Digest`]). See
/// [`canonicalize_rdf12_with`].
pub fn issue_dataset_rdf12_with<D: Digest>(
    dataset: &[Quad],
) -> Result<std::collections::HashMap<String, String>, CanonError> {
    // Crate-wide nesting-depth bound, checked iteratively BEFORE any recursive
    // descent (this is the funnel every full-profile entry point drains
    // through). [FABLE-5] sq-x3oj2.
    ensure_triple_term_depth(dataset)?;
    let state = CanonState::compute::<D>(dataset)?;
    Ok(state.canonical_issuer.issued.into_iter().collect())
}

/// **NON-STANDARD.** Canonicalizes one graph's content (a slice of [`Triple`]s,
/// treated as the default graph), **including triple terms**, into a
/// [`crate::CanonicalGraph`] under the `rdf12-triple-terms` profile.
///
/// On triple-term-free input this agrees byte-for-byte with the standard
/// [`crate::canonicalize_triples`]. See the [module docs](self) for the
/// non-standard caveat.
pub fn canonicalize_triples_rdf12(triples: &[Triple]) -> Result<crate::CanonicalGraph, CanonError> {
    canonicalize_triples_rdf12_with::<Sha256>(triples)
}

/// **NON-STANDARD.** Like [`canonicalize_triples_rdf12`] but parameterized over
/// the RDFC-1.0 hash function `D` ([`crate::Digest`]). See
/// [`canonicalize_rdf12_with`].
pub fn canonicalize_triples_rdf12_with<D: Digest>(
    triples: &[Triple],
) -> Result<crate::CanonicalGraph, CanonError> {
    // Depth-bound BEFORE the per-triple `clone` below: oxrdf's `Clone` (and
    // `Drop`) recurse through nested triple terms, so an over-deep term must
    // fail closed before it is ever cloned. [FABLE-5] sq-x3oj2.
    ensure_triple_term_depth_triples(triples)?;
    let dataset: Vec<Quad> = triples
        .iter()
        .map(|t| {
            Quad::new(
                t.subject.clone(),
                t.predicate.clone(),
                t.object.clone(),
                GraphName::DefaultGraph,
            )
        })
        .collect();
    let issued = issue_dataset_rdf12_with::<D>(&dataset)?;
    let lines = canonical_lines(&dataset, &issued);
    // Re-parse each canonical default-graph line back into a triple so the
    // returned `triples` carry the canonical (`c14nN`) labels, matching the
    // standard single-graph API's contract.
    let mut triples_out: Vec<Triple> = Vec::with_capacity(lines.len());
    for line in &lines {
        let mut parsed = None;
        for item in oxttl::NQuadsParser::new().for_slice(line.as_bytes()) {
            let q = item.map_err(|e| CanonError::Bridge(e.to_string()))?;
            parsed = Some(Triple::new(q.subject, q.predicate, q.object));
        }
        triples_out.push(parsed.ok_or_else(|| {
            CanonError::Bridge(format!(
                "canonical line did not parse as one quad: {}",
                line
            ))
        })?);
    }
    Ok(crate::CanonicalGraph {
        lines,
        triples: triples_out,
    })
}

/// **NON-STANDARD.** Canonicalizes a [`sparq_core::Graph`]'s content, including
/// triple terms, under the `rdf12-triple-terms` profile. See
/// [`canonicalize_triples_rdf12`].
pub fn canonicalize_graph_content_rdf12(
    g: &sparq_core::Graph,
) -> Result<crate::CanonicalGraph, CanonError> {
    canonicalize_graph_content_rdf12_with::<Sha256>(g)
}

/// **NON-STANDARD.** Like [`canonicalize_graph_content_rdf12`] but parameterized
/// over the RDFC-1.0 hash function `D` ([`crate::Digest`]). See
/// [`canonicalize_rdf12_with`].
pub fn canonicalize_graph_content_rdf12_with<D: Digest>(
    g: &sparq_core::Graph,
) -> Result<crate::CanonicalGraph, CanonError> {
    // `graph_triples` materializes the stored triples as-is, keeping any
    // triple-term objects intact (the standard *canonicalize* paths reject
    // triple terms downstream, not here), so it is the right source for the v2
    // profile too.
    let triples = crate::graph_triples(g)?;
    canonicalize_triples_rdf12_with::<D>(&triples)
}

// ---------------------------------------------------------------------------
// Constrained ground-triple-term variant ([FABLE-5] sq-iaxd): thin wrappers
// that fail closed on any blank node nested inside a triple term, then
// delegate. With every triple term ground, the nested-bnode HNDQ-descent
// extension above is unreachable, so these entry points exercise exactly the
// RDFC-1.0 algorithm with triple terms as opaque constants (see module docs).
// ---------------------------------------------------------------------------

/// **NON-STANDARD (constrained).** Like [`canonicalize_rdf12`], but requires
/// every triple term to be **ground** (no blank node at any nesting depth) and
/// fails closed with [`CanonError::NestedBlankNode`] otherwise — the common
/// credential/VC case. Top-level blank nodes are ordinary RDFC-1.0 bnodes and
/// are relabelled as usual. On accepted input the output is byte-identical to
/// [`canonicalize_rdf12`] (this is a thin guard + delegate wrapper), and the
/// nested-bnode HNDQ-descent extension is never exercised.
pub fn canonicalize_rdf12_ground_terms(dataset: &[Quad]) -> Result<String, CanonError> {
    canonicalize_rdf12_ground_terms_with::<Sha256>(dataset)
}

/// **NON-STANDARD (constrained).** Like [`canonicalize_rdf12_ground_terms`] but
/// parameterized over the RDFC-1.0 hash function `D` ([`crate::Digest`]); see
/// [`canonicalize_rdf12_with`] for the hash-profile semantics.
pub fn canonicalize_rdf12_ground_terms_with<D: Digest>(
    dataset: &[Quad],
) -> Result<String, CanonError> {
    // Depth-bound first: the ground guard walks nested terms, so an over-deep
    // term reports `TripleTermDepthExceeded`, not `NestedBlankNode`.
    ensure_triple_term_depth(dataset)?;
    ensure_ground_triple_terms(dataset)?;
    canonicalize_rdf12_with::<D>(dataset)
}

/// **NON-STANDARD (constrained).** Like [`issue_dataset_rdf12`], but fails
/// closed with [`CanonError::NestedBlankNode`] if any blank node occurs inside
/// a triple term. See [`canonicalize_rdf12_ground_terms`].
pub fn issue_dataset_rdf12_ground_terms(
    dataset: &[Quad],
) -> Result<std::collections::HashMap<String, String>, CanonError> {
    issue_dataset_rdf12_ground_terms_with::<Sha256>(dataset)
}

/// **NON-STANDARD (constrained).** Like [`issue_dataset_rdf12_ground_terms`]
/// but parameterized over the RDFC-1.0 hash function `D` ([`crate::Digest`]).
pub fn issue_dataset_rdf12_ground_terms_with<D: Digest>(
    dataset: &[Quad],
) -> Result<std::collections::HashMap<String, String>, CanonError> {
    // Depth-bound first — same ordering rationale as
    // `canonicalize_rdf12_ground_terms_with`.
    ensure_triple_term_depth(dataset)?;
    ensure_ground_triple_terms(dataset)?;
    issue_dataset_rdf12_with::<D>(dataset)
}

/// **NON-STANDARD (constrained).** Like [`canonicalize_triples_rdf12`], but
/// requires every triple term to be ground (no nested blank node) and fails
/// closed with [`CanonError::NestedBlankNode`] otherwise. See
/// [`canonicalize_rdf12_ground_terms`].
pub fn canonicalize_triples_rdf12_ground_terms(
    triples: &[Triple],
) -> Result<crate::CanonicalGraph, CanonError> {
    canonicalize_triples_rdf12_ground_terms_with::<Sha256>(triples)
}

/// **NON-STANDARD (constrained).** Like
/// [`canonicalize_triples_rdf12_ground_terms`] but parameterized over the
/// RDFC-1.0 hash function `D` ([`crate::Digest`]).
pub fn canonicalize_triples_rdf12_ground_terms_with<D: Digest>(
    triples: &[Triple],
) -> Result<crate::CanonicalGraph, CanonError> {
    // Depth-bound first — same ordering rationale as
    // `canonicalize_rdf12_ground_terms_with`.
    ensure_triple_term_depth_triples(triples)?;
    if triples.iter().any(|t| term_has_nested_bnode(&t.object)) {
        return Err(CanonError::NestedBlankNode);
    }
    canonicalize_triples_rdf12_with::<D>(triples)
}

/// **NON-STANDARD (constrained).** Like [`canonicalize_graph_content_rdf12`],
/// but requires every triple term stored in the graph to be **ground** (no
/// blank node at any nesting depth) and fails closed with
/// [`CanonError::NestedBlankNode`] otherwise. The [`sparq_core::Graph`]-level
/// parity sibling of [`canonicalize_triples_rdf12_ground_terms`] ([FABLE-5]
/// sq-l3pk7); equivalent to composing [`crate::graph_triples`] with that
/// function. See [`canonicalize_rdf12_ground_terms`] for the guard semantics.
pub fn canonicalize_graph_content_rdf12_ground_terms(
    g: &sparq_core::Graph,
) -> Result<crate::CanonicalGraph, CanonError> {
    canonicalize_graph_content_rdf12_ground_terms_with::<Sha256>(g)
}

/// **NON-STANDARD (constrained).** Like
/// [`canonicalize_graph_content_rdf12_ground_terms`] but parameterized over
/// the RDFC-1.0 hash function `D` ([`crate::Digest`]); see
/// [`canonicalize_rdf12_with`] for the hash-profile semantics.
pub fn canonicalize_graph_content_rdf12_ground_terms_with<D: Digest>(
    g: &sparq_core::Graph,
) -> Result<crate::CanonicalGraph, CanonError> {
    let triples = crate::graph_triples(g)?;
    canonicalize_triples_rdf12_ground_terms_with::<D>(&triples)
}

/// The ground-triple-term guard: `Err(NestedBlankNode)` iff any blank node
/// occurs **inside** a triple-term object of any quad, at any nesting depth.
/// Top-level subject/object/graph blank nodes do not count — those are
/// ordinary RDFC-1.0 blank nodes.
fn ensure_ground_triple_terms(dataset: &[Quad]) -> Result<(), CanonError> {
    if dataset.iter().any(|q| term_has_nested_bnode(&q.object)) {
        return Err(CanonError::NestedBlankNode);
    }
    Ok(())
}

/// True iff `term` is a triple term containing a blank node at any depth.
/// A top-level `Term::BlankNode` is NOT nested and returns `false`.
fn term_has_nested_bnode(term: &Term) -> bool {
    match term {
        Term::Triple(t) => triple_contains_bnode(t),
        _ => false,
    }
}

/// True iff the (triple-term) triple contains a blank node in its subject or
/// (transitively) its object. Predicates are always IRIs; in oxrdf 0.3 a
/// triple's subject is `NamedOrBlankNode`, so only the object chain descends —
/// walked with a **loop**, not recursion, so the walk itself is stack-safe on
/// any depth (the entry points additionally bound depth up front; see
/// [`ensure_triple_term_depth`]). [FABLE-5] sq-x3oj2.
fn triple_contains_bnode(t: &Triple) -> bool {
    let mut cur = t;
    loop {
        if subject_bnode(&cur.subject).is_some() {
            return true;
        }
        match &cur.object {
            Term::BlankNode(_) => return true,
            Term::Triple(inner) => cur = inner,
            _ => return false,
        }
    }
}

// ---------------------------------------------------------------------------
// Crate-wide triple-term nesting-depth bound ([FABLE-5] sq-x3oj2): the profile
// walks triple terms with recursive descent (HNDQ gossip, bnode collection,
// relabelling, `Display` serialization — and oxrdf's own `Drop`/`Clone`
// recurse), so unbounded nesting is a stack-overflow vector. Every public
// entry point pre-checks depth ITERATIVELY (the checker itself cannot
// overflow) and fails closed before any recursion or deep `clone` happens.
// ---------------------------------------------------------------------------

/// The nesting depth of a term: 0 for a non-triple term; a triple term whose
/// object is not itself a triple term has depth 1; each further level adds 1.
/// In oxrdf 0.3 only a triple's **object** can be a triple term (the subject is
/// [`NamedOrBlankNode`]), so nesting is a single chain — walked with a loop.
fn triple_term_depth(term: &Term) -> usize {
    let mut depth = 0usize;
    let mut cur = term;
    while let Term::Triple(t) = cur {
        // The subject term of `t` contributes nothing to the nesting depth —
        // `subject_nesting_depth` is what pins that (and what breaks the build
        // if it ever stops being true; [OPUS-5] sq-tx21), so accumulating down
        // the object chain alone measures the whole term.
        depth += 1 + subject_nesting_depth(&t.subject);
        cur = &t.object;
    }
    depth
}

/// Depth guard over a dataset: `Err(TripleTermDepthExceeded)` iff any quad's
/// object nests triple terms deeper than [`crate::MAX_TRIPLE_TERM_DEPTH`].
fn ensure_triple_term_depth(dataset: &[Quad]) -> Result<(), CanonError> {
    if dataset
        .iter()
        .any(|q| triple_term_depth(&q.object) > crate::MAX_TRIPLE_TERM_DEPTH)
    {
        return Err(CanonError::TripleTermDepthExceeded);
    }
    Ok(())
}

/// Depth guard over one graph's triples (see [`ensure_triple_term_depth`]).
fn ensure_triple_term_depth_triples(triples: &[Triple]) -> Result<(), CanonError> {
    if triples
        .iter()
        .any(|t| triple_term_depth(&t.object) > crate::MAX_TRIPLE_TERM_DEPTH)
    {
        return Err(CanonError::TripleTermDepthExceeded);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Canonicalization state (RDFC-1.0 §4.2), extended to triple terms.
// Structurally mirrors zkp-ld `rdf-canon` 0.15.3 (the path the standard W3C
// suite validates) so the two agree on triple-term-free input.
// ---------------------------------------------------------------------------

/// RDFC-1.0 §4.3 blank-node identifier issuer.
#[derive(Clone, PartialEq, Eq, Debug)]
struct IdentifierIssuer {
    prefix: String,
    counter: usize,
    /// input label → issued label.
    issued: BTreeMap<String, String>,
    /// zero-padded issuance index → input label, so a BTreeMap value iteration
    /// recovers §4.4(6) issuance order without a parallel Vec.
    order: BTreeMap<String, String>,
}

impl IdentifierIssuer {
    fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            counter: 0,
            issued: BTreeMap::new(),
            order: BTreeMap::new(),
        }
    }

    fn get(&self, existing: &str) -> Option<String> {
        self.issued.get(existing).cloned()
    }

    /// RDFC-1.0 §4.5 Issue Identifier.
    fn issue(&mut self, existing: &str) -> String {
        if let Some(id) = self.get(existing) {
            return id;
        }
        let issued = format!("{}{}", self.prefix, self.counter);
        self.issued.insert(existing.to_string(), issued.clone());
        self.order
            .insert(format!("{:020}", self.counter), existing.to_string());
        self.counter += 1;
        issued
    }
}

/// Map alias used throughout: a hash → the bnode labels that produced it.
type HashToBnodes = BTreeMap<String, Vec<String>>;

/// RDFC-1.0 §4.2 canonicalization state, extended for triple terms.
struct CanonState {
    /// bnode label → the quads it is a component of (recursively through
    /// triple terms). The same quad is recorded once per *distinct* bnode it
    /// contains, matching the standard algorithm.
    bnode_to_quads: BTreeMap<String, Vec<Quad>>,
    canonical_issuer: IdentifierIssuer,
}

impl CanonState {
    fn compute<D: Digest>(dataset: &[Quad]) -> Result<Self, CanonError> {
        let mut state = CanonState {
            bnode_to_quads: BTreeMap::new(),
            canonical_issuer: IdentifierIssuer::new("c14n"),
        };
        state.build_bnode_to_quads(dataset);

        // §4.4(3) first-degree hashes.
        let mut hash_to_bnodes: HashToBnodes = BTreeMap::new();
        for n in state.bnode_to_quads.keys() {
            let h = state.hash_first_degree_quads::<D>(n)?;
            hash_to_bnodes.entry(h).or_default().push(n.clone());
        }

        // §4.4(4) unique first-degree hashes → issue canonical labels.
        let mut shared: HashToBnodes = BTreeMap::new();
        for (h, list) in &hash_to_bnodes {
            if list.len() == 1 {
                state.canonical_issuer.issue(&list[0]);
            } else {
                shared.insert(h.clone(), list.clone());
            }
        }

        // §4.4(5) shared hashes → HNDQ.
        let mut counter = HndqCallCounter::new(DEFAULT_HNDQ_CALL_LIMIT);
        for list in shared.values() {
            let mut hash_path_list: Vec<HndqResult> = Vec::new();
            for n in list {
                if state.canonical_issuer.get(n).is_some() {
                    continue;
                }
                let mut temp = IdentifierIssuer::new("b");
                temp.issue(n);
                let result = state.hash_n_degree_quads::<D>(n, &temp, &mut counter)?;
                hash_path_list.push(result);
            }
            hash_path_list.sort_by(|a, b| a.hash.cmp(&b.hash));
            for result in &hash_path_list {
                // Issue canonical labels in temporary-issuance order (§4.4 5.3.1).
                for existing in result.issuer.order.values() {
                    state.canonical_issuer.issue(existing);
                }
            }
        }

        Ok(state)
    }

    // ---- §4.4(2) build the bnode→quads map, recursing into triple terms ----

    fn build_bnode_to_quads(&mut self, dataset: &[Quad]) {
        for quad in dataset {
            // Collect every distinct bnode label that is a component of this
            // quad (subject/object/graph + any depth of triple-term nesting),
            // and record the quad once for each.
            let mut labels: Vec<String> = Vec::new();
            collect_bnodes_subject(&quad.subject, &mut labels);
            collect_bnodes_term(&quad.object, &mut labels);
            if let GraphName::BlankNode(b) = &quad.graph_name {
                push_unique(&mut labels, b.as_str());
            }
            for l in labels {
                self.bnode_to_quads.entry(l).or_default().push(quad.clone());
            }
        }
    }

    fn quads_for(&self, identifier: &str) -> Option<&Vec<Quad>> {
        self.bnode_to_quads.get(identifier)
    }

    // ---- §4.6 Hash First Degree Quads (triple-term aware) ----

    fn hash_first_degree_quads<D: Digest>(&self, reference: &str) -> Result<String, CanonError> {
        let quads = self
            .quads_for(reference)
            .ok_or_else(|| CanonError::Canonicalization("quads not found for bnode".into()))?;
        let mut nquads: Vec<String> = quads
            .iter()
            .map(|q| {
                let relabelled = relabel_quad_first_degree(q, reference);
                serialize_quad_line(&relabelled)
            })
            .collect();
        nquads.sort();
        Ok(hash_hex::<D>(nquads.concat().as_bytes()))
    }

    // ---- §4.8 Hash N-Degree Quads (triple-term aware) ----

    fn hash_n_degree_quads<D: Digest>(
        &self,
        identifier: &str,
        path_issuer: &IdentifierIssuer,
        counter: &mut HndqCallCounter,
    ) -> Result<HndqResult, CanonError> {
        counter.add()?;
        let mut issuer = path_issuer.clone();

        // §4.8(1) Hn: related hash → related bnode labels.
        let mut h_n: HashToBnodes = BTreeMap::new();
        let quads = self
            .quads_for(identifier)
            .ok_or_else(|| CanonError::Canonicalization("quads not found for bnode".into()))?;

        // §4.8(3) for each quad, for each related bnode component (recursing
        // through triple terms), hash-related-blank-node into Hn.
        for quad in quads {
            // subject position
            let mut subj_bnodes: Vec<String> = Vec::new();
            collect_bnodes_subject(&quad.subject, &mut subj_bnodes);
            for related in &subj_bnodes {
                if related != identifier {
                    let h = self.hash_related_blank_node::<D>(
                        related,
                        quad,
                        &issuer,
                        Position::Subject,
                    )?;
                    h_n.entry(h).or_default().push(related.clone());
                }
            }
            // object position (includes bnodes nested inside a triple term)
            let mut obj_bnodes: Vec<String> = Vec::new();
            collect_bnodes_term(&quad.object, &mut obj_bnodes);
            for related in &obj_bnodes {
                if related != identifier {
                    let h = self.hash_related_blank_node::<D>(
                        related,
                        quad,
                        &issuer,
                        Position::Object,
                    )?;
                    h_n.entry(h).or_default().push(related.clone());
                }
            }
            // graph position
            if let GraphName::BlankNode(b) = &quad.graph_name {
                let related = b.as_str();
                if related != identifier {
                    let h =
                        self.hash_related_blank_node::<D>(related, quad, &issuer, Position::Graph)?;
                    h_n.entry(h).or_default().push(related.to_string());
                }
            }
        }

        // §4.8(4,5) build data-to-hash.
        let mut data_to_hash = String::new();
        for (related_hash, blank_node_list) in h_n {
            data_to_hash.push_str(&related_hash);
            let mut chosen_path = String::new();
            let mut chosen_issuer: Option<IdentifierIssuer> = None;

            // §4.8(5.4) permutations.
            for perm in permutations(&blank_node_list) {
                let mut issuer_copy = issuer.clone();
                let mut path = String::new();
                let mut recursion_list: Vec<String> = Vec::new();
                let mut skip = false;

                for related in &perm {
                    if let Some(cid) = self.canonical_issuer.get(related) {
                        path.push_str(&format!("_:{}", cid));
                    } else {
                        if issuer_copy.get(related).is_none() {
                            recursion_list.push(related.clone());
                        }
                        path.push_str(&format!("_:{}", issuer_copy.issue(related)));
                    }
                    if !chosen_path.is_empty()
                        && path.len() >= chosen_path.len()
                        && path >= chosen_path
                    {
                        skip = true;
                        break;
                    }
                }
                if skip {
                    continue;
                }

                for related in &recursion_list {
                    let result = self.hash_n_degree_quads::<D>(related, &issuer_copy, counter)?;
                    path.push_str(&format!("_:{}", issuer_copy.issue(related)));
                    path.push('<');
                    path.push_str(&result.hash);
                    path.push('>');
                    issuer_copy = result.issuer;
                    if !chosen_path.is_empty()
                        && path.len() >= chosen_path.len()
                        && path >= chosen_path
                    {
                        skip = true;
                        break;
                    }
                }
                if skip {
                    continue;
                }

                if chosen_path.is_empty() || path < chosen_path {
                    chosen_path = path;
                    chosen_issuer = Some(issuer_copy);
                }
            }

            data_to_hash.push_str(&chosen_path);
            if let Some(ci) = chosen_issuer {
                issuer = ci;
            }
        }

        Ok(HndqResult {
            hash: hash_hex::<D>(data_to_hash.as_bytes()),
            issuer,
        })
    }

    // ---- §4.7 Hash Related Blank Node ----

    fn hash_related_blank_node<D: Digest>(
        &self,
        related: &str,
        quad: &Quad,
        issuer: &IdentifierIssuer,
        position: Position,
    ) -> Result<String, CanonError> {
        let mut input = match position {
            Position::Graph => "g".to_string(),
            Position::Subject => format!("s{}", quad.predicate),
            Position::Object => format!("o{}", quad.predicate),
        };
        let identifier = match self.canonical_issuer.get(related) {
            Some(id) => format!("_:{}", id),
            None => match issuer.get(related) {
                Some(id) => format!("_:{}", id),
                None => self.hash_first_degree_quads::<D>(related)?,
            },
        };
        input.push_str(&identifier);
        Ok(hash_hex::<D>(input.as_bytes()))
    }
}

#[derive(Clone, Copy)]
enum Position {
    Subject,
    Object,
    Graph,
}

#[derive(Debug)]
struct HndqResult {
    hash: String,
    issuer: IdentifierIssuer,
}

/// Poison-graph guard: caps total HNDQ invocations (RDFC-1.0 §4.8 worst case is
/// super-polynomial). Mirrors the standard path's default limit so the v2
/// profile fails closed identically.
struct HndqCallCounter {
    count: usize,
    limit: usize,
}

impl HndqCallCounter {
    fn new(limit: usize) -> Self {
        Self { count: 0, limit }
    }
    fn add(&mut self) -> Result<(), CanonError> {
        self.count += 1;
        if self.count > self.limit {
            Err(CanonError::Canonicalization(format!(
                "HNDQ call limit ({}) exceeded (poison graph)",
                self.limit
            )))
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Bnode collection + relabelling, recursing through triple terms.
// ---------------------------------------------------------------------------

fn push_unique(labels: &mut Vec<String>, label: &str) {
    if !labels.iter().any(|l| l == label) {
        labels.push(label.to_string());
    }
}

fn collect_bnodes_subject(subject: &NamedOrBlankNode, out: &mut Vec<String>) {
    if let Some(b) = subject_bnode(subject) {
        push_unique(out, b.as_str());
    }
}

fn collect_bnodes_term(term: &Term, out: &mut Vec<String>) {
    match term {
        Term::BlankNode(b) => push_unique(out, b.as_str()),
        Term::Triple(t) => {
            collect_bnodes_subject(&t.subject, out);
            collect_bnodes_term(&t.object, out);
        }
        _ => {}
    }
}

/// RDFC-1.0 §4.6 (3.1.1) special relabelling: a bnode equal to the reference
/// becomes `a`, any other bnode becomes `z`. Applied recursively through triple
/// terms so nested bnodes get the same special-identifier treatment.
fn relabel_quad_first_degree(quad: &Quad, reference: &str) -> Quad {
    Quad::new(
        relabel_subject(&quad.subject, reference),
        quad.predicate.clone(),
        relabel_term(&quad.object, reference),
        relabel_graph(&quad.graph_name, reference),
    )
}

fn relabel_subject(subject: &NamedOrBlankNode, reference: &str) -> NamedOrBlankNode {
    match subject_bnode(subject) {
        Some(b) => NamedOrBlankNode::BlankNode(special_label(b.as_str(), reference)),
        None => subject.clone(),
    }
}

fn relabel_term(term: &Term, reference: &str) -> Term {
    match term {
        Term::BlankNode(b) => Term::BlankNode(special_label(b.as_str(), reference)),
        Term::Triple(t) => Term::Triple(Box::new(Triple::new(
            relabel_subject(&t.subject, reference),
            t.predicate.clone(),
            relabel_term(&t.object, reference),
        ))),
        other => other.clone(),
    }
}

fn relabel_graph(graph: &GraphName, reference: &str) -> GraphName {
    match graph {
        GraphName::BlankNode(b) => GraphName::BlankNode(special_label(b.as_str(), reference)),
        other => other.clone(),
    }
}

fn special_label(label: &str, reference: &str) -> BlankNode {
    if label == reference {
        BlankNode::new_unchecked("a")
    } else {
        BlankNode::new_unchecked("z")
    }
}

// ---------------------------------------------------------------------------
// Final serialization with canonical labels.
// ---------------------------------------------------------------------------

/// §5 serialize: relabel every bnode to its canonical `c14nN`, sort the lines
/// in code-point order, concatenate (one `\n`-terminated line per quad).
fn serialize_canonical(
    dataset: &[Quad],
    issued: &std::collections::HashMap<String, String>,
) -> String {
    let mut doc = String::new();
    for line in canonical_lines(dataset, issued) {
        doc.push_str(&line);
        doc.push('\n');
    }
    doc
}

/// The canonical N-Quads lines (each `… .`, NO trailing newline — matching the
/// standard `CanonicalGraph::lines` contract), sorted in code-point order and
/// **deduplicated**: RDF is a set, so identical quads (which canonicalize to the
/// same line) collapse, exactly as the `rdf-canon` `Dataset` path does.
fn canonical_lines(
    dataset: &[Quad],
    issued: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let mut lines: Vec<String> = dataset
        .iter()
        .map(|q| {
            let relabelled = Quad::new(
                relabel_subject_canonical(&q.subject, issued),
                q.predicate.clone(),
                relabel_term_canonical(&q.object, issued),
                relabel_graph_canonical(&q.graph_name, issued),
            );
            canonical_line_no_newline(&relabelled)
        })
        .collect();
    lines.sort();
    lines.dedup();
    lines
}

/// A canonical N-Quads line WITHOUT the trailing ` .\n` separator — the
/// `CanonicalGraph::lines` form. (The hashing path uses [`serialize_quad_line`],
/// which keeps the ` .\n` the RDFC-1.0 spec hashes over.)
fn canonical_line_no_newline(quad: &Quad) -> String {
    match &quad.graph_name {
        GraphName::DefaultGraph => {
            format!("{} {} {} .", quad.subject, quad.predicate, quad.object)
        }
        graph => format!(
            "{} {} {} {} .",
            quad.subject, quad.predicate, quad.object, graph
        ),
    }
}

fn issued_label(label: &str, issued: &std::collections::HashMap<String, String>) -> BlankNode {
    match issued.get(label) {
        Some(c) => BlankNode::new_unchecked(c.clone()),
        // Unlabelled bnode (shouldn't happen for a fully canonicalized dataset);
        // keep the input label rather than panic so the failure is visible.
        None => BlankNode::new_unchecked(label),
    }
}

fn relabel_subject_canonical(
    subject: &NamedOrBlankNode,
    issued: &std::collections::HashMap<String, String>,
) -> NamedOrBlankNode {
    match subject_bnode(subject) {
        Some(b) => NamedOrBlankNode::BlankNode(issued_label(b.as_str(), issued)),
        None => subject.clone(),
    }
}

fn relabel_term_canonical(term: &Term, issued: &std::collections::HashMap<String, String>) -> Term {
    match term {
        Term::BlankNode(b) => Term::BlankNode(issued_label(b.as_str(), issued)),
        Term::Triple(t) => Term::Triple(Box::new(Triple::new(
            relabel_subject_canonical(&t.subject, issued),
            t.predicate.clone(),
            relabel_term_canonical(&t.object, issued),
        ))),
        other => other.clone(),
    }
}

fn relabel_graph_canonical(
    graph: &GraphName,
    issued: &std::collections::HashMap<String, String>,
) -> GraphName {
    match graph {
        GraphName::BlankNode(b) => GraphName::BlankNode(issued_label(b.as_str(), issued)),
        other => other.clone(),
    }
}

/// One canonical N-Quads line, `… .\n`, using oxrdf-0.3's canonical `Display`
/// (which renders triple terms as `<<( … )>>` and directional literals as
/// `"…"@lang--dir` per RDF-1.2 N-Quads). Default-graph quads omit the graph
/// term (3-term line), matching the standard serializer.
fn serialize_quad_line(quad: &Quad) -> String {
    match &quad.graph_name {
        GraphName::DefaultGraph => {
            format!("{} {} {} .\n", quad.subject, quad.predicate, quad.object)
        }
        graph => {
            format!(
                "{} {} {} {} .\n",
                quad.subject, quad.predicate, quad.object, graph
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Small helpers.
// ---------------------------------------------------------------------------

/// Lowercase-hex of `D`'s digest over `data`. The RDFC-1.0 algorithm is
/// parameterized over its hash function; `D` is threaded through every hashing
/// step (first-degree, related, n-degree) so the canonical labels are computed
/// entirely under the caller's chosen hash. SHA-256 (`canonicalize_rdf12`) is
/// the spec default; SHA-384 (`canonicalize_rdf12_with::<Sha384>`) the parity
/// target.
fn hash_hex<D: Digest>(data: &[u8]) -> String {
    let digest = D::digest(data);
    let mut s = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        s.push_str(&format!("{:02x}", byte));
    }
    s
}

/// All permutations of `items` (RDFC-1.0 §4.8 5.4). Lists here are the bnodes
/// sharing one related hash; the spec's factorial blow-up is bounded by the
/// HNDQ call limit applied in the recursion.
fn permutations(items: &[String]) -> Vec<Vec<String>> {
    if items.is_empty() {
        return vec![vec![]];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let head = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, head.clone());
            out.push(p);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-qcnn.14 — direct unit tests for PRIVATE helper functions in
// rdf12.rs. These kill mutation survivors that the external functional tests
// cannot see (the external tests verify final canonical OUTPUT; internal
// mutations that produce the same output via different intermediate steps
// survive unless the intermediates themselves are asserted on here).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod private_tests {
    use super::*;
    use oxrdf::{BlankNode, GraphName, Literal, NamedNode, NamedOrBlankNode, Term, Triple};

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }
    fn bn(s: &str) -> BlankNode {
        BlankNode::new(s).unwrap()
    }

    // ---- special_label ----

    /// `special_label(label, reference)` must return `"a"` when label == reference
    /// and `"z"` otherwise. Kills the `== with !=` comparison mutation (line 609).
    #[test]
    fn special_label_reference_becomes_a() {
        let label = special_label("x", "x");
        assert_eq!(
            label.as_str(),
            "a",
            "the reference bnode must receive the 'a' special label"
        );
    }

    #[test]
    fn special_label_other_becomes_z() {
        let label = special_label("y", "x");
        assert_eq!(
            label.as_str(),
            "z",
            "any non-reference bnode must receive the 'z' special label"
        );
    }

    // ---- push_unique ----

    /// `push_unique` must NOT add a label that is already in the list.
    /// Kills the `delete !` mutation (line 546) that would always push.
    #[test]
    fn push_unique_does_not_add_duplicates() {
        let mut labels: Vec<String> = vec!["a".to_string()];
        push_unique(&mut labels, "a");
        assert_eq!(labels.len(), 1, "duplicate must not be added: {labels:?}");
    }

    #[test]
    fn push_unique_adds_new_label() {
        let mut labels: Vec<String> = vec!["a".to_string()];
        push_unique(&mut labels, "b");
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[1], "b");
    }

    // ---- collect_bnodes_term ----

    /// A `Term::BlankNode` must be collected. Kills `delete match arm
    /// Term::BlankNode(b)` (line 559).
    #[test]
    fn collect_bnodes_term_collects_blank_node() {
        let mut out: Vec<String> = Vec::new();
        collect_bnodes_term(&Term::BlankNode(bn("myb")), &mut out);
        assert_eq!(out, vec!["myb".to_string()]);
    }

    /// A `Term::Triple` must recurse into subject and object to collect nested
    /// bnodes. Kills `delete match arm Term::Triple(t)` (line 560).
    #[test]
    fn collect_bnodes_term_recurses_into_triple_term() {
        let inner_bnode = Term::BlankNode(bn("inner"));
        let tt = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::BlankNode(bn("subj")),
            iri("http://ex/p"),
            inner_bnode,
        )));
        let mut out: Vec<String> = Vec::new();
        collect_bnodes_term(&tt, &mut out);
        // Both the subject bnode and the inner object bnode must be collected.
        assert!(out.contains(&"subj".to_string()), "subject bnode in triple term: {out:?}");
        assert!(out.contains(&"inner".to_string()), "object bnode in triple term: {out:?}");
        assert_eq!(out.len(), 2, "exactly two bnodes: {out:?}");
    }

    /// A `Term::NamedNode` must NOT contribute any bnodes.
    #[test]
    fn collect_bnodes_term_ignores_iri() {
        let mut out: Vec<String> = Vec::new();
        collect_bnodes_term(&Term::NamedNode(iri("http://ex/n")), &mut out);
        assert!(out.is_empty(), "IRI must not contribute a bnode: {out:?}");
    }

    // ---- collect_bnodes_subject ----

    #[test]
    fn collect_bnodes_subject_collects_blank_node() {
        let mut out: Vec<String> = Vec::new();
        collect_bnodes_subject(&NamedOrBlankNode::BlankNode(bn("s")), &mut out);
        assert_eq!(out, vec!["s".to_string()]);
    }

    #[test]
    fn collect_bnodes_subject_ignores_named_node() {
        let mut out: Vec<String> = Vec::new();
        collect_bnodes_subject(&NamedOrBlankNode::NamedNode(iri("http://ex/s")), &mut out);
        assert!(out.is_empty());
    }

    // ---- subject_bnode / subject_nesting_depth (sq-tx21 tripwires) ----

    /// `subject_bnode` must hand back the blank node itself, not merely report
    /// that one is present — every caller relabels or enrols by its label, so
    /// returning the wrong node (or `None`) silently drops it from the descent.
    #[test]
    fn subject_bnode_yields_the_blank_node() {
        let subject = NamedOrBlankNode::BlankNode(bn("mysubj"));
        assert_eq!(
            subject_bnode(&subject).map(|b| b.as_str()),
            Some("mysubj"),
            "a blank-node subject must be returned by label"
        );
    }

    /// An IRI subject carries no blank node. Kills a swap of the two arms.
    #[test]
    fn subject_bnode_is_none_for_iri() {
        let subject = NamedOrBlankNode::NamedNode(iri("http://ex/s"));
        assert!(
            subject_bnode(&subject).is_none(),
            "an IRI subject must yield no blank node"
        );
    }

    /// A subject contributes ZERO nesting depth, for both kinds oxrdf 0.3
    /// admits. This is the assumption [`triple_term_depth`]'s object-only chain
    /// walk — and therefore the `MAX_TRIPLE_TERM_DEPTH` stack-overflow bound —
    /// rests on; a non-zero answer here would inflate every depth measurement
    /// and reject legal input.
    #[test]
    fn subject_nesting_depth_is_zero_for_both_subject_kinds() {
        assert_eq!(
            subject_nesting_depth(&NamedOrBlankNode::NamedNode(iri("http://ex/s"))),
            0,
            "an IRI subject nests nothing"
        );
        assert_eq!(
            subject_nesting_depth(&NamedOrBlankNode::BlankNode(bn("s"))),
            0,
            "a blank-node subject nests nothing"
        );
    }

    /// The end-to-end consequence of the two tripwires: a blank node sitting in
    /// the SUBJECT of a triple term is enrolled and relabelled exactly like one
    /// in its object. (What sq-tx21 tracks is the *other* case — a triple term
    /// nested in subject position — which oxrdf 0.3 cannot express at all.)
    #[test]
    fn blank_node_in_triple_term_subject_is_relabelled() {
        let tt = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::BlankNode(bn("inner")),
            iri("http://ex/p"),
            Term::NamedNode(iri("http://ex/o")),
        )));
        let quad = oxrdf::Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/says"),
            tt,
            GraphName::DefaultGraph,
        );
        let doc = canonicalize_rdf12(&[quad]).unwrap();
        assert!(
            doc.contains("_:c14n0"),
            "subject bnode of a triple term must receive a canonical label: {doc:?}"
        );
        assert!(
            !doc.contains("_:inner"),
            "the input label must not survive canonicalization: {doc:?}"
        );
    }

    // ---- HndqCallCounter ----

    /// `add` must increment the counter and return `Ok(())` until the limit, then
    /// return an error. Kills `replace += with -=` (line 529) and `replace add
    /// with Ok(())` (line 529 / replace fn).
    #[test]
    fn hndq_call_counter_increments_and_fails_at_limit() {
        let mut c = HndqCallCounter::new(2);
        assert!(c.add().is_ok(), "first call must succeed");
        assert!(c.add().is_ok(), "second call (at limit) must succeed");
        assert!(c.add().is_err(), "third call (over limit) must fail");
    }

    // ---- IdentifierIssuer ----

    /// `get` must return `None` for an unlabelled node and `Some(...)` for an
    /// issued one. Kills `replace get with None` (line 227) and
    /// `replace get with Some(String::new())` (line 227).
    #[test]
    fn identifier_issuer_get_returns_none_for_unlabelled() {
        let issuer = IdentifierIssuer::new("c14n");
        assert_eq!(issuer.get("x"), None);
    }

    #[test]
    fn identifier_issuer_get_returns_issued_label() {
        let mut issuer = IdentifierIssuer::new("c14n");
        let issued = issuer.issue("x");
        assert_eq!(issuer.get("x"), Some(issued.clone()));
        assert_eq!(issued, "c14n0");
    }

    /// `issue` must return incrementing labels `c14n0`, `c14n1`, ... and be
    /// idempotent (re-issuing the same label returns the same value).
    /// Kills `replace issue with String::new()` (line 232).
    #[test]
    fn identifier_issuer_issues_sequential_labels() {
        let mut issuer = IdentifierIssuer::new("c14n");
        assert_eq!(issuer.issue("a"), "c14n0");
        assert_eq!(issuer.issue("b"), "c14n1");
        assert_eq!(issuer.issue("c"), "c14n2");
        // Idempotent: re-issuing returns the SAME label, counter does not advance.
        assert_eq!(issuer.issue("a"), "c14n0", "re-issue must be idempotent");
        assert_eq!(issuer.issue("d"), "c14n3", "counter must not advance on re-issue");
    }

    // ---- hash_hex ----

    /// `hash_hex::<Sha256>` must produce the EXACT lowercase hex of the SHA-256
    /// digest. Kills format-string mutations (e.g. `{:02x}` → `{:02X}` or `{}`).
    #[test]
    fn hash_hex_sha256_known_value() {
        use sha2::Sha256;
        // SHA-256("") is a well-known constant; must be exact + lowercase + 64 chars.
        let hex = hash_hex::<Sha256>(b"");
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "hash_hex must produce lowercase hex of SHA-256"
        );
        assert_eq!(hex.len(), 64, "SHA-256 hex must be 64 chars");
        // A non-empty input must differ from the empty-string digest (not hardcoded).
        let hex2 = hash_hex::<Sha256>(b"abc");
        assert_ne!(hex, hex2, "different inputs must produce different hashes");
        assert_eq!(hex2.len(), 64, "SHA-256 hex must be 64 chars");
        // Must be all-lowercase hex (no uppercase A-F).
        assert!(
            hex.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "hash_hex must be lowercase hex: {hex}"
        );
    }

    // ---- permutations ----

    /// `permutations` must return the complete factorial-sized set of orderings.
    /// Kills `replace with vec![]` and `replace with vec![vec![...]]`.
    #[test]
    fn permutations_empty_gives_one_empty_perm() {
        let result = permutations(&[]);
        assert_eq!(result, vec![vec![] as Vec<String>]);
    }

    #[test]
    fn permutations_one_item() {
        let result = permutations(&["a".to_string()]);
        assert_eq!(result, vec![vec!["a".to_string()]]);
    }

    #[test]
    fn permutations_two_items_exact() {
        let mut result = permutations(&["a".to_string(), "b".to_string()]);
        result.sort();
        // Both orderings must appear.
        assert_eq!(
            result,
            vec![
                vec!["a".to_string(), "b".to_string()],
                vec!["b".to_string(), "a".to_string()],
            ],
            "two-item permutations must yield exactly [a,b] and [b,a]"
        );
    }

    #[test]
    fn permutations_three_items_count() {
        let result = permutations(&["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(result.len(), 6, "3! = 6 permutations");
        // Every distinct item must appear as the first element in exactly 2 perms.
        for head in &["a", "b", "c"] {
            let count = result.iter().filter(|p| p[0].as_str() == *head).count();
            assert_eq!(count, 2, "each item must head exactly 2 permutations");
        }
    }

    // ---- serialize_quad_line ----

    /// `serialize_quad_line` must produce `"s p o .\n"` for a default-graph quad
    /// and `"s p o g .\n"` for a named-graph quad. Kills the `replace with
    /// "xyzzy".into()` and `replace with String::new()` mutations.
    #[test]
    fn serialize_quad_line_default_graph() {
        let q = oxrdf::Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::NamedNode(iri("http://ex/o")),
            GraphName::DefaultGraph,
        );
        let line = serialize_quad_line(&q);
        assert_eq!(
            line,
            "<http://ex/s> <http://ex/p> <http://ex/o> .\n",
            "default-graph quad must serialize without a graph term"
        );
    }

    #[test]
    fn serialize_quad_line_named_graph() {
        let q = oxrdf::Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::NamedNode(iri("http://ex/o")),
            GraphName::NamedNode(iri("http://ex/g")),
        );
        let line = serialize_quad_line(&q);
        assert_eq!(
            line,
            "<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g> .\n",
            "named-graph quad must carry the graph term"
        );
    }

    // ---- canonical_line_no_newline ----

    /// `canonical_line_no_newline` must produce `"s p o ."` (NO trailing newline).
    /// Kills `replace with "xyzzy".into()`. Distinct from `serialize_quad_line`
    /// which DOES have a trailing `\n`.
    #[test]
    fn canonical_line_no_newline_default_graph() {
        let q = oxrdf::Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
            GraphName::DefaultGraph,
        );
        let line = canonical_line_no_newline(&q);
        assert_eq!(
            line,
            "<http://ex/s> <http://ex/p> \"v\" .",
            "default-graph line must have NO trailing newline"
        );
        assert!(
            !line.ends_with('\n'),
            "canonical_line_no_newline must NOT end with newline: {line:?}"
        );
    }

    #[test]
    fn canonical_line_no_newline_named_graph() {
        let q = oxrdf::Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
            GraphName::NamedNode(iri("http://ex/g")),
        );
        let line = canonical_line_no_newline(&q);
        assert_eq!(
            line,
            "<http://ex/s> <http://ex/p> \"v\" <http://ex/g> .",
        );
    }

    // ---- relabel_quad_first_degree ----

    /// `relabel_quad_first_degree` must replace the reference bnode with `_:a`
    /// and any other bnode with `_:z`. Kills `replace with Default::default()`.
    #[test]
    fn relabel_quad_first_degree_reference_becomes_a_others_z() {
        let q = oxrdf::Quad::new(
            NamedOrBlankNode::BlankNode(bn("ref")),
            iri("http://ex/p"),
            Term::BlankNode(bn("other")),
            GraphName::DefaultGraph,
        );
        let relabelled = relabel_quad_first_degree(&q, "ref");
        assert_eq!(
            relabelled.subject,
            NamedOrBlankNode::BlankNode(BlankNode::new_unchecked("a")),
            "reference bnode in subject must become 'a'"
        );
        assert_eq!(
            relabelled.object,
            Term::BlankNode(BlankNode::new_unchecked("z")),
            "non-reference bnode in object must become 'z'"
        );
    }

    // ---- relabel_term / relabel_subject / relabel_graph (first-degree) ----

    /// `relabel_term` for a `Term::Triple` must recurse through the triple term
    /// and relabel bnodes at every depth. Kills `replace relabel_term -> Term
    /// with Default::default()`.
    #[test]
    fn relabel_term_recurses_through_triple_term() {
        let inner = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::BlankNode(bn("ref")),
            iri("http://ex/p"),
            Term::BlankNode(bn("other")),
        )));
        let relabelled = relabel_term(&inner, "ref");
        if let Term::Triple(t) = relabelled {
            assert_eq!(
                t.subject,
                NamedOrBlankNode::BlankNode(BlankNode::new_unchecked("a")),
                "reference bnode in inner subject must become 'a'"
            );
            assert_eq!(
                t.object,
                Term::BlankNode(BlankNode::new_unchecked("z")),
                "non-reference bnode in inner object must become 'z'"
            );
        } else {
            panic!("relabel_term must preserve Term::Triple wrapper");
        }
    }

    /// `relabel_graph` for a `GraphName::BlankNode` must apply the special label.
    #[test]
    fn relabel_graph_blank_graph_name() {
        let g = GraphName::BlankNode(bn("ref"));
        let relabelled = relabel_graph(&g, "ref");
        assert_eq!(
            relabelled,
            GraphName::BlankNode(BlankNode::new_unchecked("a")),
            "bnode graph name equal to reference must become 'a'"
        );
        let g2 = GraphName::BlankNode(bn("other"));
        let relabelled2 = relabel_graph(&g2, "ref");
        assert_eq!(
            relabelled2,
            GraphName::BlankNode(BlankNode::new_unchecked("z")),
            "bnode graph name not equal to reference must become 'z'"
        );
    }

    // ---- serialize_canonical / canonical_lines ----

    /// `serialize_canonical` must concatenate `canonical_lines` with trailing
    /// newlines. Kills `replace serialize_canonical -> String with String::new()`.
    #[test]
    fn serialize_canonical_concatenates_lines() {
        use std::collections::HashMap;
        let q = oxrdf::Quad::new(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
            GraphName::DefaultGraph,
        );
        let mut issued = HashMap::new();
        issued.insert("b".to_string(), "c14n0".to_string());
        let doc = serialize_canonical(&[q], &issued);
        assert!(!doc.is_empty(), "serialize_canonical must not return empty string");
        assert!(doc.ends_with('\n'), "each line must be newline-terminated");
        assert!(
            doc.contains("_:c14n0"),
            "issued label must appear in serialized output: {doc:?}"
        );
    }

    /// `canonical_lines` must deduplicate identical canonical lines and sort them.
    /// Kills `replace canonical_lines -> Vec<String> with vec![]` and
    /// `replace with vec![String::new()]`.
    #[test]
    fn canonical_lines_deduplicates_and_sorts() {
        use std::collections::HashMap;
        // Two quads that are IDENTICAL after canonicalization (dup-test).
        let q1 = oxrdf::Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/a")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
            GraphName::DefaultGraph,
        );
        let q2 = q1.clone();
        let issued: HashMap<String, String> = HashMap::new();
        let lines = canonical_lines(&[q1, q2], &issued);
        assert_eq!(lines.len(), 1, "duplicate quads must be deduplicated");

        // Two distinct quads — must be sorted in code-point order.
        let qa = oxrdf::Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/z")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("b")),
            GraphName::DefaultGraph,
        );
        let qb = oxrdf::Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/a")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("a")),
            GraphName::DefaultGraph,
        );
        let lines2 = canonical_lines(&[qa, qb], &issued);
        assert_eq!(lines2.len(), 2, "distinct quads give two lines");
        let mut sorted = lines2.clone();
        sorted.sort();
        assert_eq!(lines2, sorted, "canonical_lines must be in sorted order");
    }

    // ---- relabel_subject_canonical / relabel_term_canonical / relabel_graph_canonical ----

    /// `relabel_term_canonical` for a `Term::Triple` must recurse into the nested
    /// triple and replace any bnodes with their issued canonical labels.
    /// Kills `replace relabel_term_canonical -> Term with Default::default()`.
    #[test]
    fn relabel_term_canonical_recurses_through_triple_term() {
        use std::collections::HashMap;
        let inner = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::BlankNode(bn("b")),
            iri("http://ex/p"),
            Term::BlankNode(bn("c")),
        )));
        let mut issued = HashMap::new();
        issued.insert("b".to_string(), "c14n0".to_string());
        issued.insert("c".to_string(), "c14n1".to_string());
        let relabelled = relabel_term_canonical(&inner, &issued);
        if let Term::Triple(t) = relabelled {
            assert_eq!(
                t.subject,
                NamedOrBlankNode::BlankNode(BlankNode::new_unchecked("c14n0")),
                "inner subject bnode must be relabelled canonically"
            );
            assert_eq!(
                t.object,
                Term::BlankNode(BlankNode::new_unchecked("c14n1")),
                "inner object bnode must be relabelled canonically"
            );
        } else {
            panic!("relabel_term_canonical must preserve Term::Triple wrapper");
        }
    }

    /// `relabel_graph_canonical` for a `GraphName::BlankNode` must substitute
    /// the issued canonical label. Kills `replace with Default::default()`.
    #[test]
    fn relabel_graph_canonical_substitutes_issued_label() {
        use std::collections::HashMap;
        let mut issued = HashMap::new();
        issued.insert("g".to_string(), "c14n0".to_string());
        let result = relabel_graph_canonical(&GraphName::BlankNode(bn("g")), &issued);
        assert_eq!(
            result,
            GraphName::BlankNode(BlankNode::new_unchecked("c14n0")),
            "bnode graph name must be replaced with its canonical label"
        );
        // DefaultGraph stays DefaultGraph.
        assert_eq!(
            relabel_graph_canonical(&GraphName::DefaultGraph, &issued),
            GraphName::DefaultGraph
        );
    }

    // ---- ground-triple-term guard helpers ([FABLE-5] sq-iaxd) ----

    /// A TOP-LEVEL blank-node object is NOT "nested" — the guard must accept it
    /// (it is an ordinary RDFC-1.0 bnode). Kills a `_ => false` → `_ => true`
    /// arm mutation and pins the load-bearing top-level/nested distinction.
    #[test]
    fn term_has_nested_bnode_false_for_top_level_bnode_and_iri() {
        assert!(!term_has_nested_bnode(&Term::BlankNode(bn("top"))));
        assert!(!term_has_nested_bnode(&Term::NamedNode(iri("http://ex/o"))));
        assert!(!term_has_nested_bnode(&Term::Literal(
            oxrdf::Literal::new_simple_literal("v")
        )));
    }

    /// A bnode in the triple term's SUBJECT position must be detected.
    #[test]
    fn term_has_nested_bnode_detects_inner_subject() {
        let tt = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::BlankNode(bn("inner")),
            iri("http://ex/p"),
            Term::Literal(oxrdf::Literal::new_simple_literal("v")),
        )));
        assert!(term_has_nested_bnode(&tt));
    }

    /// A bnode in the triple term's OBJECT position must be detected, and a
    /// fully ground triple term must NOT be.
    #[test]
    fn term_has_nested_bnode_detects_inner_object_and_accepts_ground() {
        let with_bnode = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::BlankNode(bn("inner")),
        )));
        assert!(term_has_nested_bnode(&with_bnode));
        let ground = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::NamedNode(iri("http://ex/o")),
        )));
        assert!(!term_has_nested_bnode(&ground));
    }

    /// `triple_contains_bnode` must recurse through a depth-2 triple term
    /// (a bnode only at the deepest level). Kills a "no recursion" mutation.
    #[test]
    fn triple_contains_bnode_recurses_to_depth_two() {
        let deepest = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/x")),
            iri("http://ex/p"),
            Term::BlankNode(bn("deep")),
        );
        let mid = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/y")),
            iri("http://ex/q"),
            Term::Triple(Box::new(deepest)),
        );
        assert!(triple_contains_bnode(&mid), "depth-2 bnode must be found");
        // Same shape but ground at every depth: not detected.
        let ground_deepest = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/x")),
            iri("http://ex/p"),
            Term::Literal(oxrdf::Literal::new_simple_literal("v")),
        );
        let ground_mid = Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/y")),
            iri("http://ex/q"),
            Term::Triple(Box::new(ground_deepest)),
        );
        assert!(!triple_contains_bnode(&ground_mid));
    }

    /// `ensure_ground_triple_terms` must accept a dataset whose only bnodes are
    /// top-level (subject/graph) and reject one with a nested bnode.
    #[test]
    fn ensure_ground_triple_terms_guard() {
        let ground_tt = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::NamedNode(iri("http://ex/o")),
        )));
        let ok = Quad::new(
            NamedOrBlankNode::BlankNode(bn("top")),
            iri("http://ex/says"),
            ground_tt,
            GraphName::BlankNode(bn("g")),
        );
        assert!(ensure_ground_triple_terms(std::slice::from_ref(&ok)).is_ok());
        let bad_tt = Term::Triple(Box::new(Triple::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::BlankNode(bn("inner")),
        )));
        let bad = Quad::new(
            NamedOrBlankNode::NamedNode(iri("http://ex/a")),
            iri("http://ex/says"),
            bad_tt,
            GraphName::DefaultGraph,
        );
        assert!(matches!(
            ensure_ground_triple_terms(&[ok, bad]),
            Err(CanonError::NestedBlankNode)
        ));
    }

    // ---- [FABLE-5] sq-x3oj2: crate-wide triple-term nesting-depth bound ----

    /// Builds a ground triple-term chain of exactly `depth` nesting levels
    /// (iteratively — the test helper must not itself recurse).
    fn ground_chain(depth: usize) -> Term {
        let mut obj = Term::NamedNode(iri("http://ex/leaf"));
        for _ in 0..depth {
            obj = Term::Triple(Box::new(Triple::new(
                NamedOrBlankNode::NamedNode(iri("http://ex/s")),
                iri("http://ex/p"),
                obj,
            )));
        }
        obj
    }

    /// Direct test for `triple_term_depth`: exact values at 0 / 1 / 3, killing
    /// off-by-one and "count only the top level" mutations.
    #[test]
    fn triple_term_depth_exact_counts() {
        assert_eq!(triple_term_depth(&Term::NamedNode(iri("http://ex/o"))), 0);
        assert_eq!(triple_term_depth(&ground_chain(1)), 1);
        assert_eq!(triple_term_depth(&ground_chain(3)), 3);
    }

    /// Direct boundary test for `ensure_triple_term_depth` (quads): depth ==
    /// MAX is accepted, MAX+1 fails closed. Knocking out the guard turns the
    /// Err arm into Ok — red.
    #[test]
    fn ensure_triple_term_depth_boundary() {
        let quad_at = |depth: usize| {
            Quad::new(
                NamedOrBlankNode::NamedNode(iri("http://ex/a")),
                iri("http://ex/says"),
                ground_chain(depth),
                GraphName::DefaultGraph,
            )
        };
        assert!(ensure_triple_term_depth(&[quad_at(crate::MAX_TRIPLE_TERM_DEPTH)]).is_ok());
        assert!(matches!(
            ensure_triple_term_depth(&[quad_at(crate::MAX_TRIPLE_TERM_DEPTH + 1)]),
            Err(CanonError::TripleTermDepthExceeded)
        ));
    }

    /// Direct boundary test for `ensure_triple_term_depth_triples`.
    #[test]
    fn ensure_triple_term_depth_triples_boundary() {
        let triple_at = |depth: usize| {
            Triple::new(
                NamedOrBlankNode::NamedNode(iri("http://ex/a")),
                iri("http://ex/says"),
                ground_chain(depth),
            )
        };
        assert!(
            ensure_triple_term_depth_triples(&[triple_at(crate::MAX_TRIPLE_TERM_DEPTH)]).is_ok()
        );
        assert!(matches!(
            ensure_triple_term_depth_triples(&[triple_at(crate::MAX_TRIPLE_TERM_DEPTH + 1)]),
            Err(CanonError::TripleTermDepthExceeded)
        ));
    }

    /// `triple_contains_bnode` is a loop, not recursion: a chain far deeper
    /// than any recursion budget must answer without overflowing, both for the
    /// all-ground case (false) and with a blank node buried at the innermost
    /// level (true). The load-bearing assert is the innermost-bnode `true`
    /// (kills "stop at first level" mutations).
    #[test]
    fn triple_contains_bnode_deep_chain_loop_safe() {
        // All-ground deep chain -> false.
        let Term::Triple(ground) = ground_chain(2048) else {
            panic!("ground_chain must return a triple term")
        };
        assert!(!triple_contains_bnode(&ground));
        // Same chain with a bnode OBJECT at the innermost level -> true.
        let mut obj = Term::BlankNode(bn("innermost"));
        for _ in 0..2048 {
            obj = Term::Triple(Box::new(Triple::new(
                NamedOrBlankNode::NamedNode(iri("http://ex/s")),
                iri("http://ex/p"),
                obj,
            )));
        }
        let Term::Triple(with_bnode) = obj else {
            panic!("chain must be a triple term")
        };
        assert!(triple_contains_bnode(&with_bnode));
    }
}
