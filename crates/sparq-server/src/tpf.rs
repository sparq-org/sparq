//! [OPUS-4.8] sq-bzh1 (epic sq-3183): OPT-IN Triple Pattern Fragments (TPF) / Linked Data
//! Fragments server endpoint.
//!
//! A [Triple Pattern Fragments](https://www.hydra-cg.com/spec/latest/triple-pattern-fragments/)
//! (Hartig & Verborgh, [LDF](http://linkeddatafragments.org/)) interface turns `sparq-server`
//! into a *low-cost federation SOURCE*: instead of accepting a full SPARQL query, the endpoint
//! answers a single **triple pattern** (`?s ?p ?o` with any of the three positions bound) and
//! returns a *page* of the matching triples plus Hydra controls so a client-side TPF query
//! engine (Comunica, the LDF client) can drive a join itself. This pushes far less work onto
//! the server than a full SPARQL endpoint — the per-request cost is bounded by ONE page plus a
//! cheap cardinality estimate — which is exactly why TPF exists.
//!
//! ## What this module is
//!
//! A PURE (no async, no HTTP types) builder — like [`crate::graph`] / [`crate::descriptors`] —
//! that, given a parsed triple pattern, a pinned `Graph` snapshot and a page number,
//! produces the response **triples** (the page of data triples + the Hydra metadata/control
//! triples) ready for the crate's existing graph serialisers ([`crate::graph::triples_to_turtle`]
//! / [`triples_to_ntriples`](crate::graph::triples_to_ntriples)). The async handler that wires
//! it onto the axum router and content-negotiates the serialisation lives in
//! [`crate::http`] (`tpf_endpoint`). Keeping the logic here makes the pattern-match, the
//! `hydra:totalItems` estimate, and the paging boundary unit-testable without booting a server.
//!
//! ## Pattern selection + the cardinality estimate (cheap, not a full scan)
//!
//! The three positions come from the `subject` / `predicate` / `object` query parameters, each
//! an **N-Triples term** (`<iri>`, `"lit"`, `"lit"@en`, `"lit"^^<dt>`); an absent or empty
//! parameter is a *variable* (unbound). The pattern resolves through the public
//! `Graph::pattern` dictionary lookup, then:
//!
//!   * `hydra:totalItems` (and the void:triples count on the metadata node) is the engine's
//!     **cardinality ESTIMATE** for the pattern ([`sparq_core::store::TripleStore::estimate`]) —
//!     the same cheap index-range count the query planner uses (binary-search bounds on the raw
//!     indexes; two boundary blocks on the compressed mode), NEVER a full scan. On sparq's
//!     in-memory permutation indexes this estimate is in fact EXACT, but we advertise it as an
//!     estimate (the TPF spec's `void:triples` is explicitly an estimate) so the contract holds
//!     if a future storage mode makes it approximate. A bound term that is *absent from the
//!     dictionary* (so the pattern can match nothing) short-circuits to a count of 0 and an
//!     empty page without touching the index.
//!   * The page itself scans only the requested window: the sorted index range for the pattern,
//!     sliced to `[page_offset, page_offset + page_size)`. Resolving each row's three ids back
//!     to terms is O(page_size), independent of the pattern's total cardinality.
//!
//! ## Hydra controls
//!
//! The fragment is a small RDF document with three logical parts (all in the SAME graph, the
//! TPF convention):
//!
//!   * the **data** — the page of matching triples;
//!   * the **fragment metadata** — a node for *this page URL* carrying `void:triples`
//!     /`hydra:totalItems` (the estimated count), `hydra:itemsPerPage` (the page size), and the
//!     `hydra:next` / `hydra:previous` paging controls (present only when there is a next /
//!     previous page);
//!   * the **Hydra search control** — a `hydra:search` / `hydra:template` / `hydra:mapping`
//!     description of the `{subject}` / `{predicate}` / `{object}` URI template, so a generic
//!     TPF client can discover how to request any other pattern from this same dataset.
//!
//! ## OPT-IN — feature `tpf` AND a config flag, both OFF by default
//!
//! Compiled only behind the `tpf` cargo feature; even then the route refuses (`404`) unless
//! [`ServerConfig::tpf`](crate::ServerConfig::tpf) is set (CLI `--tpf` / env `SPARQ_TPF=1`).
//! This mirrors the `federation-descriptors` double-opt-in exactly. A build without the feature
//! pays ZERO cost — no route, no new dependency, byte-identical to before — and
//! `sparq-core`/`sparq-engine` are untouched (this reads a pinned `&Graph` snapshot through the
//! existing public `Graph::pattern` / `store.estimate` / `store.scan` / `dict.term` API).
//!
//! It is a **READ-only** source interface — there is no write path.

use std::str::FromStr;

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};

use sparq_core::Graph;

/// The default page size — the maximum number of data triples one fragment carries. A bounded
/// page is the whole point of TPF (the per-request cost ceiling); 100 is the conventional LDF
/// server default (the reference Node LDF server / Comunica use it), small enough to keep a
/// page cheap and large enough that a join's per-page overhead amortises.
pub const DEFAULT_PAGE_SIZE: usize = 100;

/// `hydra:` — the Hydra Core vocabulary (the TPF control vocabulary).
const HYDRA: &str = "http://www.w3.org/ns/hydra/core#";
/// `void:` — VoID (the fragment's `void:triples` count + the dataset/subset linkage).
const VOID: &str = "http://rdfs.org/ns/void#";
/// `rdf:type`.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// `xsd:integer` — the datatype of the count / page-size literals.
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// [OPUS-4.8] sq-dxhb: the brTPF `values` control property — the Hydra `hydra:property` the
/// `{values}` template variable maps to. brTPF (Hartig & Buil-Aranda, ODBASE 2016) has no term
/// in a standardised vocabulary for "the attached solution-mapping set", so we mint one under a
/// sparq namespace; a client reads it from the `hydra:mapping` to learn the dataset accepts a
/// bindings restriction. Compiled only with the `brtpf` feature.
#[cfg(feature = "brtpf")]
const BRTPF_VALUES_PROPERTY: &str = "https://sparq.dev/ns/brtpf#values";

/// A parsed triple pattern: each position is `Some(term)` when the request bound it, or `None`
/// (a variable). The predicate is a [`NamedNode`] (a predicate is always an IRI), matching the
/// shape [`Graph::pattern`] wants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TriplePattern {
    pub subject: Option<Term>,
    pub predicate: Option<NamedNode>,
    pub object: Option<Term>,
}

/// An error parsing one of the `subject` / `predicate` / `object` parameters into a term. The
/// HTTP layer maps it to a `400`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TriplePattern {
    /// Parses a triple pattern from the three raw query-parameter values (each already
    /// form-decoded). An absent (`None`) or whitespace-only value is a *variable* (unbound);
    /// a present value is parsed as an **N-Triples term** — `<iri>`, `"literal"`,
    /// `"literal"@lang`, `"literal"^^<datatype>`. The subject and object accept any term
    /// (IRI / blank node / literal — though a literal subject can never match); the predicate
    /// MUST be an IRI (a non-IRI predicate is a parse error, since a predicate is always an
    /// IRI and the [`Graph::pattern`] API takes a [`NamedNode`]).
    pub fn parse(
        subject: Option<&str>,
        predicate: Option<&str>,
        object: Option<&str>,
    ) -> Result<Self, ParseError> {
        Ok(Self {
            subject: parse_term(subject, "subject")?,
            predicate: parse_predicate(predicate)?,
            object: parse_term(object, "object")?,
        })
    }

    /// `true` when no position is bound — the `?s ?p ?o` "everything" pattern (a full dump,
    /// still paged).
    pub fn is_all(&self) -> bool {
        self.subject.is_none() && self.predicate.is_none() && self.object.is_none()
    }
}

/// Parses an optional N-Triples term parameter; empty / whitespace-only ⇒ a variable (`None`).
fn parse_term(raw: Option<&str>, pos: &str) -> Result<Option<Term>, ParseError> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) => Term::from_str(s)
            .map(Some)
            .map_err(|e| ParseError(format!("invalid {} term '{}': {e}", pos, s))),
    }
}

/// Parses the optional predicate parameter — which must be an IRI (a [`NamedNode`]).
fn parse_predicate(raw: Option<&str>) -> Result<Option<NamedNode>, ParseError> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) => match Term::from_str(s) {
            Ok(Term::NamedNode(n)) => Ok(Some(n)),
            Ok(_) => Err(ParseError(format!("predicate must be an IRI, got '{s}'"))),
            Err(e) => Err(ParseError(format!("invalid predicate term '{s}': {e}"))),
        },
    }
}

/// One TPF page (fragment) for a triple pattern: the matched-triple count estimate and the
/// data triples on THIS page, plus the paging position. The HTTP layer combines this with the
/// page URL to add the Hydra control triples ([`fragment_triples`]).
#[derive(Debug, Clone)]
pub struct Fragment {
    /// The estimated total number of matches for the pattern (`hydra:totalItems` /
    /// `void:triples`) — the cheap index estimate, NOT a count of the page.
    pub total_estimate: usize,
    /// The (zero-based) page number this fragment represents.
    pub page: usize,
    /// The page size (`hydra:itemsPerPage`).
    pub page_size: usize,
    /// The data triples on this page (at most `page_size`).
    pub triples: Vec<Triple>,
}

impl Fragment {
    /// `true` when a page after this one exists (there are more matches than
    /// `(page + 1) * page_size`) — drives the `hydra:next` control.
    pub fn has_next(&self) -> bool {
        // A page is non-final iff at least one match lies beyond its window. We key this on the
        // ESTIMATE (the same number we advertise as totalItems), so `next` is consistent with
        // the count we report. The final-page-exactly boundary therefore reports no `next`.
        (self.page + 1) * self.page_size < self.total_estimate
    }

    /// `true` when there is a page before this one (`page > 0`) — drives `hydra:previous`.
    pub fn has_previous(&self) -> bool {
        self.page > 0
    }

    /// The (zero-based) index of the LAST page of this result — drives `hydra:last`. With
    /// `n = total_estimate` matches at `page_size` per page there are `ceil(n / page_size)`
    /// pages, so the last index is `ceil(n / page_size) - 1`, or `0` when there are no matches
    /// (a single empty page IS the last page — there is always a page 0). Keyed on the same
    /// ESTIMATE we advertise as `hydra:totalItems`, so `last` is consistent with `next` and the
    /// reported count.
    pub fn last_page(&self) -> usize {
        if self.total_estimate == 0 {
            0
        } else {
            (self.total_estimate - 1) / self.page_size
        }
    }
}

/// Evaluates `pattern` against the pinned `graph` snapshot, returning the [`Fragment`] for
/// (zero-based) `page` with `page_size` data triples.
///
/// The cardinality estimate is the cheap index-range count (`store.estimate`); the page
/// scans only its `[page*page_size, …)` window and resolves that window's ids back to terms.
/// A bound term absent from the dictionary makes the pattern unmatchable: estimate 0, empty
/// page, no index work.
pub fn evaluate(graph: &Graph, pattern: &TriplePattern, page: usize, page_size: usize) -> Fragment {
    let page_size = page_size.max(1);
    // Resolve the bound terms to dictionary ids. `None` here means a BOUND term that is absent
    // from the dictionary — the pattern can match nothing, so short-circuit to an empty page.
    let id_pattern = match graph.pattern(
        pattern.subject.as_ref(),
        pattern.predicate.as_ref(),
        pattern.object.as_ref(),
    ) {
        Some(p) => p,
        None => {
            return Fragment {
                total_estimate: 0,
                page,
                page_size,
                triples: Vec::new(),
            }
        }
    };

    // Cheap cardinality estimate for `hydra:totalItems` — the planner's index-range count.
    let total_estimate = graph.store.estimate(&id_pattern);

    // Scan the pattern's sorted index range and slice out this page's window. We materialise the
    // page's ids first (the scan borrows the index), then resolve them to terms — O(page_size),
    // independent of the total cardinality.
    let offset = page.saturating_mul(page_size);
    let scan = graph.store.scan(&id_pattern);
    let page_ids: Vec<[sparq_core::dict::Id; 3]> = scan
        .rows
        .iter()
        .skip(offset)
        .take(page_size)
        .map(|row| scan.to_spo(row))
        .collect();

    let triples = page_ids
        .into_iter()
        .filter_map(|[s, p, o]| spo_to_triple(graph, s, p, o))
        .collect();

    Fragment {
        total_estimate,
        page,
        page_size,
        triples,
    }
}

// ---------------------------------------------------------------------------
// [OPUS-4.8] sq-dxhb: bindings-restricted Triple Pattern Fragments (brTPF).
// ---------------------------------------------------------------------------

/// [OPUS-4.8] sq-dxhb: one brTPF solution mapping — a PARTIAL binding of the triple-pattern
/// positions. Each of `subject` / `predicate` / `object` is `Some(term)` when this mapping
/// constrains that position, or `None` (the mapping says nothing about it). A mapping that
/// binds a position the PATTERN already bound is allowed (it is simply redundant — the merge
/// keeps the pattern's term, and a contradictory binding makes that mapping match nothing).
///
/// Compiled only with the `brtpf` feature.
#[cfg(feature = "brtpf")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Binding {
    pub subject: Option<Term>,
    pub predicate: Option<NamedNode>,
    pub object: Option<Term>,
}

#[cfg(feature = "brtpf")]
impl Binding {
    /// `true` when this mapping constrains no position — the empty mapping μ₀, which is
    /// compatible with EVERY triple, so it does not restrict the fragment at all.
    pub fn is_empty(&self) -> bool {
        self.subject.is_none() && self.predicate.is_none() && self.object.is_none()
    }
}

/// [OPUS-4.8] sq-r74h: the DoS caps on a brTPF binding set, enforced by [`parse_bindings_capped`]
/// BEFORE the (potentially large) payload is fully parsed and BEFORE any index work. `0` on either
/// field disables that cap (the historical uncapped behaviour). These bound the binding set
/// *independently of* the server's `--max-body-bytes` body limit, for two reasons the body limit
/// does not cover:
///
///   * **The `values` query-string transport is NOT a body** — a `GET /tpf?values=…` carries the
///     binding set in the URL, which `--max-body-bytes` (an HTTP *body* limit) never sees. So a
///     payload-byte cap is the only bound on the query-string carrier.
///   * **Cost is super-linear in the mapping COUNT, not the bytes** — [`evaluate_brtpf`] runs ONE
///     index scan per mapping and unions the rows into a `BTreeSet`, so N tiny mappings (each a
///     handful of bytes, so well under any byte limit) still cost N scans + an O(result) set. A
///     mapping-COUNT cap bounds that fan-out directly; a byte limit only bounds it transitively
///     and far too loosely (a 1 MiB body of `s=<a>\n`-sized mappings is ~150k scans).
///
/// Compiled only with the `brtpf` feature.
#[cfg(feature = "brtpf")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingLimits {
    /// Maximum number of (non-empty) solution mappings accepted. `0` disables the count cap.
    pub max_mappings: usize,
    /// Maximum size, in bytes, of the raw `values` payload accepted. `0` disables the byte cap.
    pub max_payload_bytes: usize,
}

#[cfg(feature = "brtpf")]
impl BindingLimits {
    /// No caps — the historical uncapped behaviour (used by [`parse_bindings`]).
    pub const UNLIMITED: BindingLimits = BindingLimits {
        max_mappings: 0,
        max_payload_bytes: 0,
    };
}

/// [OPUS-4.8] sq-r74h: a brTPF binding-set parse outcome that DISTINGUISHES a malformed payload
/// (the client sent garbage → `400`) from a payload that exceeds a DoS cap (the client sent a
/// well-formed but too-large set → `413`, the same refusal class as `--max-body-bytes`). The HTTP
/// layer maps the two variants to those two distinct statuses.
///
/// Compiled only with the `brtpf` feature.
#[cfg(feature = "brtpf")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    /// The payload was malformed (a bad `position=term` pair, a non-IRI predicate, …) → `400`.
    Malformed(ParseError),
    /// The payload was well-formed but exceeded a [`BindingLimits`] DoS cap → `413`. The message
    /// names which cap and its limit (NOT any caller-supplied term — no info-leak / no echo).
    TooLarge(String),
}

#[cfg(feature = "brtpf")]
impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindingError::Malformed(e) => write!(f, "{e}"),
            BindingError::TooLarge(m) => write!(f, "{m}"),
        }
    }
}

#[cfg(feature = "brtpf")]
impl From<ParseError> for BindingError {
    fn from(e: ParseError) -> Self {
        BindingError::Malformed(e)
    }
}

/// [OPUS-4.8] sq-dxhb: parses a brTPF `values` payload into the set of solution mappings.
///
/// The wire format is one mapping per line; within a line, whitespace-separated
/// `position=term` pairs, where `position` is `subject` / `predicate` / `object` (or the short
/// `s` / `p` / `o`) and `term` is an **N-Triples term** (`<iri>`, `"lit"`, `"lit"@en`,
/// `"lit"^^<dt>`) — the SAME term grammar as the `subject`/`predicate`/`object` query
/// parameters, so a client encodes a binding exactly like a bound pattern position. A blank
/// line is skipped; the empty payload is the empty binding set (no restriction). A line with no
/// pairs is the empty mapping μ₀ (compatible with everything) and is skipped (it would defeat
/// the restriction). The predicate position must be an IRI (same rule as the pattern).
///
/// This is the UNCAPPED parse — for the DoS-capped form a public server should use, see
/// [`parse_bindings_capped`] (sq-r74h). This entrypoint is retained for callers that have already
/// bounded the input (e.g. unit tests) and is equivalent to
/// `parse_bindings_capped(payload, BindingLimits::UNLIMITED)` with the error re-narrowed to
/// [`ParseError`] (it can only fail malformed when uncapped).
///
/// Example payload (URL-decoded), restricting `?s foaf:knows ?o` to two `?s` values:
///
/// ```text
/// s=<http://ex/alice>
/// s=<http://ex/carol>
/// ```
#[cfg(feature = "brtpf")]
pub fn parse_bindings(payload: &str) -> Result<Vec<Binding>, ParseError> {
    match parse_bindings_capped(payload, BindingLimits::UNLIMITED) {
        Ok(b) => Ok(b),
        // UNLIMITED can never produce a TooLarge; only Malformed is reachable here.
        Err(BindingError::Malformed(e)) => Err(e),
        Err(BindingError::TooLarge(m)) => {
            unreachable!("BindingLimits::UNLIMITED never trips a cap: {m}")
        }
    }
}

/// [OPUS-4.8] sq-r74h: [`parse_bindings`] with the DoS caps in [`BindingLimits`] enforced.
///
/// Enforces, in this order:
///   1. **The payload-byte cap** (`limits.max_payload_bytes`) — checked FIRST, on the raw byte
///      length, BEFORE any line scanning, so an oversized payload is rejected without parsing it.
///      This is the bound on the `values` *query-string* carrier, which `--max-body-bytes` (a
///      *body* limit) never sees.
///   2. **The mapping-count cap** (`limits.max_mappings`) — checked as non-empty mappings
///      accumulate, so parsing stops at the limit + 1th mapping rather than building the whole
///      vector first. This bounds [`evaluate_brtpf`]'s per-mapping index-scan fan-out directly.
///
/// A `0` limit disables that cap. A cap breach returns [`BindingError::TooLarge`] (the HTTP layer
/// → `413`); a malformed payload returns [`BindingError::Malformed`] (→ `400`). The byte cap is
/// checked before the malformed check, so a payload that is BOTH oversized and malformed is
/// reported as too-large (the cheaper, earlier refusal — we never parse an over-cap payload).
#[cfg(feature = "brtpf")]
pub fn parse_bindings_capped(
    payload: &str,
    limits: BindingLimits,
) -> Result<Vec<Binding>, BindingError> {
    // 1. Payload-byte cap — rejected on raw length before any parse work. NB: the message names
    // only the limit, never the payload, so an over-cap request leaks nothing back to the client.
    if limits.max_payload_bytes != 0 && payload.len() > limits.max_payload_bytes {
        return Err(BindingError::TooLarge(format!(
            "brTPF binding payload exceeds the {}-byte cap (--brtpf-max-values-bytes)",
            limits.max_payload_bytes
        )));
    }
    let mut out = Vec::new();
    for line in payload.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut b = Binding::default();
        for pair in line.split_whitespace() {
            let (pos, term) = pair.split_once('=').ok_or_else(|| {
                ParseError(format!(
                    "brTPF binding pair must be 'position=term', got '{pair}'"
                ))
            })?;
            match pos {
                "subject" | "s" => b.subject = parse_term(Some(term), "subject")?,
                "predicate" | "p" => b.predicate = parse_predicate(Some(term))?,
                "object" | "o" => b.object = parse_term(Some(term), "object")?,
                other => {
                    return Err(BindingError::Malformed(ParseError(format!(
                        "brTPF binding position must be subject/predicate/object (s/p/o), got '{other}'"
                    ))))
                }
            }
        }
        // A mapping that constrains nothing (an all-blank μ₀) does not restrict the fragment;
        // drop it rather than let it widen the result to the whole pattern.
        if !b.is_empty() {
            // 2. Mapping-count cap — checked as each non-empty mapping is accepted, so we stop at
            // the (limit+1)th rather than materialising the whole set first.
            if limits.max_mappings != 0 && out.len() == limits.max_mappings {
                return Err(BindingError::TooLarge(format!(
                    "brTPF binding set exceeds the {}-mapping cap (--brtpf-max-bindings)",
                    limits.max_mappings
                )));
            }
            out.push(b);
        }
    }
    Ok(out)
}

/// [OPUS-4.8] sq-dxhb: evaluates `pattern` against `graph` RESTRICTED to the brTPF binding set
/// `bindings`, returning the [`Fragment`] for (zero-based) `page`.
///
/// The bindings-restricted fragment is the set of triples matching `pattern` that are
/// COMPATIBLE WITH AT LEAST ONE supplied mapping (brTPF's semi-join pushdown). We evaluate it
/// the way a brTPF server is meant to — NOT by scanning the whole pattern and filtering, but by
/// MERGING each mapping into the pattern (a mapping's bound positions specialise the matching
/// pattern's variable positions) and scanning each specialised, more-selective pattern, then
/// taking the DEDUPLICATED UNION. A mapping whose bound term is absent from the dictionary, or
/// whose bound position CONTRADICTS the pattern's own bound term, contributes nothing (it can
/// match no triple) and is skipped without index work.
///
/// `total_estimate` is the EXACT size of the deduplicated union (this is a count of the real
/// restricted result, so we can be exact and still advertise it as `hydra:totalItems`); the
/// page slices `[page*page_size, …)` of that union in numeric `[s,p,o]` id-lexicographic order
/// (the `BTreeSet` order). That order is DETERMINISTIC and INTRA-RESULT CONSISTENT across this
/// fragment's pages — the property paging actually needs (page boundaries never overlap and the
/// pages partition the result). It is NOT necessarily the same order plain [`evaluate`] uses:
/// plain TPF slices the chosen INDEX's scan order, which for a given bound-position shape may
/// sort on a different id permutation. Both orders are deterministic; they just need not be
/// identical to each other.
///
/// An EMPTY binding set means "no restriction" and is delegated to the plain [`evaluate`] — a
/// brTPF request with zero mappings is exactly a TPF request (the spec's μ₀-only case).
#[cfg(feature = "brtpf")]
pub fn evaluate_brtpf(
    graph: &Graph,
    pattern: &TriplePattern,
    bindings: &[Binding],
    page: usize,
    page_size: usize,
) -> Fragment {
    let page_size = page_size.max(1);
    if bindings.is_empty() {
        return evaluate(graph, pattern, page, page_size);
    }

    // The deduplicated union of every binding's specialised-pattern matches, as canonical
    // [s,p,o] id triples. A BTreeSet keeps them sorted in the canonical order (so paging is
    // deterministic) AND deduplicated (two mappings can specialise to overlapping patterns).
    let mut union: std::collections::BTreeSet<[sparq_core::dict::Id; 3]> =
        std::collections::BTreeSet::new();

    for b in bindings {
        // Merge the mapping into the pattern: each position takes the PATTERN's term if it bound
        // one, else the mapping's term (the mapping only specialises a variable position). If the
        // mapping and the pattern both bind a position to DIFFERENT terms the merged pattern is
        // contradictory — `Graph::pattern` returns `None` for an absent term, and we additionally
        // detect a direct subject/predicate/object clash below, so a contradictory mapping
        // contributes nothing.
        let subject = merge_term(pattern.subject.as_ref(), b.subject.as_ref());
        let predicate = merge_predicate(pattern.predicate.as_ref(), b.predicate.as_ref());
        let object = merge_term(pattern.object.as_ref(), b.object.as_ref());
        let (subject, predicate, object) = match (subject, predicate, object) {
            (Some(s), Some(p), Some(o)) => (s, p, o),
            // A clash (the mapping bound a position the pattern already bound, to a different
            // term): this mapping matches nothing. Skip it.
            _ => continue,
        };

        let id_pattern = match graph.pattern(subject.as_ref(), predicate.as_ref(), object.as_ref())
        {
            Some(p) => p,
            // A bound term absent from the dictionary: this mapping matches nothing.
            None => continue,
        };
        let scan = graph.store.scan(&id_pattern);
        for row in scan.rows.iter() {
            union.insert(scan.to_spo(row));
        }
    }

    let total_estimate = union.len();
    let offset = page.saturating_mul(page_size);
    let triples = union
        .into_iter()
        .skip(offset)
        .take(page_size)
        .filter_map(|[s, p, o]| spo_to_triple(graph, s, p, o))
        .collect();

    Fragment {
        total_estimate,
        page,
        page_size,
        triples,
    }
}

/// [OPUS-4.8] sq-dxhb: merges a pattern position (the term the PATTERN bound, if any) with a
/// brTPF mapping's term for the same position. Returns `Some(merged)` — `Some(Some(term))` when
/// a term is fixed, `Some(None)` when the merged position is still a variable — or `None` on a
/// CLASH (both bound, to different terms ⇒ the mapping is incompatible with the pattern).
#[cfg(feature = "brtpf")]
fn merge_term(pat: Option<&Term>, bind: Option<&Term>) -> Option<Option<Term>> {
    match (pat, bind) {
        (Some(p), Some(b)) if p == b => Some(Some(p.clone())),
        (Some(_), Some(_)) => None, // clash
        (Some(p), None) => Some(Some(p.clone())),
        (None, Some(b)) => Some(Some(b.clone())),
        (None, None) => Some(None),
    }
}

/// [OPUS-4.8] sq-dxhb: [`merge_term`] for the predicate position (a [`NamedNode`]).
#[cfg(feature = "brtpf")]
fn merge_predicate(pat: Option<&NamedNode>, bind: Option<&NamedNode>) -> Option<Option<NamedNode>> {
    match (pat, bind) {
        (Some(p), Some(b)) if p == b => Some(Some(p.clone())),
        (Some(_), Some(_)) => None, // clash
        (Some(p), None) => Some(Some(p.clone())),
        (None, Some(b)) => Some(Some(b.clone())),
        (None, None) => Some(None),
    }
}

/// Resolves a canonical `[s, p, o]` id triple to an [`oxrdf::Triple`] via the public
/// [`Dict::term`](sparq_core::dict::Dict::term). A row whose subject is a literal or whose
/// predicate is not an IRI cannot form a valid RDF triple (it never arises from a well-formed
/// store, but we are total rather than panic): such a row is dropped (`None`).
fn spo_to_triple(
    graph: &Graph,
    s: sparq_core::dict::Id,
    p: sparq_core::dict::Id,
    o: sparq_core::dict::Id,
) -> Option<Triple> {
    let subject = match graph.dict.term(s) {
        Term::NamedNode(n) => oxrdf::NamedOrBlankNode::NamedNode(n),
        Term::BlankNode(b) => oxrdf::NamedOrBlankNode::BlankNode(b),
        // A literal or a triple-term subject cannot be a NamedOrBlankNode — drop the row
        // (never arises from a well-formed store; we are total rather than panic).
        Term::Literal(_) | Term::Triple(_) => return None,
    };
    let predicate = match graph.dict.term(p) {
        Term::NamedNode(n) => n,
        _ => return None,
    };
    let object = graph.dict.term(o);
    Some(Triple::new(subject, predicate, object))
}

/// Builds the COMPLETE fragment document as a triple list: the page's data triples PLUS the
/// Hydra metadata + control triples, all in one graph (the TPF convention). Ready for the
/// crate's graph serialisers.
///
/// * `fragment_url` is the absolute URL of THIS page (used as the metadata node and the subject
///   of the paging controls); it already carries this page's `subject`/`predicate`/`object` and
///   `page` parameters.
/// * `dataset_url` names the `void:Dataset` (conventionally the TPF endpoint URL, e.g.
///   `http://host/tpf`); it is the `hydra:search` host and the `void:subset` parent.
/// * `template` is the URI template advertised by `hydra:template` (e.g.
///   `http://host/tpf{?subject,predicate,object}`).
/// * `next_url` / `prev_url` are the `hydra:next` / `hydra:previous` page URLs, present only
///   when [`Fragment::has_next`] / [`Fragment::has_previous`].
/// * `first_url` / `last_url` are the `hydra:first` / `hydra:last` page URLs — the fuller Hydra
///   `PartialCollectionView` paging vocabulary (sq-dxhb). `first` is always page 0; `last` is
///   [`Fragment::last_page`] (the page holding the final match, or page 0 for an empty result).
///   Pass `None` to omit either (e.g. on a malformed-Host fallback where the URL would not be a
///   valid IRI). Unlike `next`/`prev`, these are emitted on EVERY page — a client can jump to
///   either end of the view from any page, which is exactly what `hydra:first`/`hydra:last` are
///   for.
///
/// Defensive on IRI validity: the URLs are derived from the request `Host` header (attacker-
/// controlled), so any that is not a valid IRI is dropped rather than emitted as malformed RDF.
#[allow(clippy::too_many_arguments)]
pub fn fragment_triples(
    frag: &Fragment,
    fragment_url: &str,
    dataset_url: &str,
    template: &str,
    next_url: Option<&str>,
    prev_url: Option<&str>,
    first_url: Option<&str>,
    last_url: Option<&str>,
) -> Vec<Triple> {
    let hydra = |local: &str| NamedNode::new(format!("{HYDRA}{local}")).ok();
    let void = |local: &str| NamedNode::new(format!("{VOID}{local}")).ok();
    let iri = |s: &str| NamedNode::new(s).ok();
    let count =
        |n: usize| Literal::new_typed_literal(n.to_string(), NamedNode::new_unchecked(XSD_INTEGER));

    let mut out = Vec::with_capacity(frag.triples.len() + 12);

    // ---- The data: the page of matching triples.
    out.extend(frag.triples.iter().cloned());

    // ---- Fragment + dataset nodes (need valid IRIs to attach metadata to).
    let (Some(frag_node), Some(dataset_node)) = (iri(fragment_url), iri(dataset_url)) else {
        // No valid fragment/dataset IRI (a hostile Host header) — return just the data; the
        // document is still well-formed RDF, just without controls.
        return out;
    };

    // A `subject predicate object` triple, emitted only when the predicate IRI resolved. Term
    // construction is explicit (`Term::from` / `Term::NamedNode` / `Term::BlankNode`) because
    // several crates in scope (oxttl / spargebra) also impl `From<…>`, making bare `.into()`
    // ambiguous.
    let mut emit = |s: NamedOrBlankNode, p: Option<NamedNode>, o: Term| {
        if let Some(p) = p {
            out.push(Triple::new(s, p, o));
        }
    };
    let frag_subj = || NamedOrBlankNode::NamedNode(frag_node.clone());
    let dataset_subj = || NamedOrBlankNode::NamedNode(dataset_node.clone());

    // Type the fragment as hydra:Collection + hydra:PartialCollectionView (the TPF view typing)
    // and link the dataset → this page as a void:subset.
    if let Some(t) = iri(RDF_TYPE) {
        if let Some(coll) = hydra("Collection") {
            emit(frag_subj(), Some(t.clone()), Term::NamedNode(coll));
        }
        if let Some(view) = hydra("PartialCollectionView") {
            emit(frag_subj(), Some(t), Term::NamedNode(view));
        }
    }
    emit(
        dataset_subj(),
        void("subset"),
        Term::NamedNode(frag_node.clone()),
    );

    // ---- Counts: hydra:totalItems + void:triples (the estimate) and hydra:itemsPerPage.
    emit(
        frag_subj(),
        hydra("totalItems"),
        Term::Literal(count(frag.total_estimate)),
    );
    emit(
        frag_subj(),
        void("triples"),
        Term::Literal(count(frag.total_estimate)),
    );
    emit(
        frag_subj(),
        hydra("itemsPerPage"),
        Term::Literal(count(frag.page_size)),
    );

    // ---- Paging controls: hydra:next / hydra:previous.
    if let (Some(next), Some(p)) = (next_url, hydra("next")) {
        if let Some(n) = iri(next) {
            emit(frag_subj(), Some(p), Term::NamedNode(n));
        }
    }
    if let (Some(prev), Some(p)) = (prev_url, hydra("previous")) {
        if let Some(pv) = iri(prev) {
            emit(frag_subj(), Some(p), Term::NamedNode(pv));
        }
    }

    // ---- Paging controls: hydra:first / hydra:last (sq-dxhb — the fuller Hydra paging
    // vocabulary). Emitted on every page (a client can jump to either end from anywhere).
    if let (Some(first), Some(p)) = (first_url, hydra("first")) {
        if let Some(f) = iri(first) {
            emit(frag_subj(), Some(p), Term::NamedNode(f));
        }
    }
    if let (Some(last), Some(p)) = (last_url, hydra("last")) {
        if let Some(l) = iri(last) {
            emit(frag_subj(), Some(p), Term::NamedNode(l));
        }
    }

    // ---- The Hydra search control: how to request ANY pattern from this dataset.
    // dataset hydra:search _:search . _:search hydra:template "…{?subject,predicate,object}" ;
    //   hydra:mapping _:s, _:p, _:o . _:s hydra:variable "subject" ; hydra:property rdf:subject .
    let search = oxrdf::BlankNode::default();
    emit(
        dataset_subj(),
        hydra("search"),
        Term::BlankNode(search.clone()),
    );
    emit(
        NamedOrBlankNode::BlankNode(search.clone()),
        hydra("template"),
        Term::Literal(Literal::new_simple_literal(template)),
    );
    // The three variable mappings (subject/predicate/object), each binding the template variable
    // to the corresponding rdf: property so a client knows which position it controls.
    #[allow(unused_mut)]
    let mut mappings = vec![
        (
            "subject",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#subject",
        ),
        (
            "predicate",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#predicate",
        ),
        (
            "object",
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#object",
        ),
    ];
    // brTPF (sq-dxhb): advertise the `values` control variable so a TPF/brTPF client discovers
    // it may attach a set of solution mappings. It binds to the brTPF spec's `values` property
    // (`http://www.w3.org/ns/sparql-service-description#` has no term for it; brTPF uses a
    // dedicated property under its own namespace). Compiled out without the feature, so a
    // `tpf`-only fragment is byte-identical to before.
    #[cfg(feature = "brtpf")]
    mappings.push(("values", BRTPF_VALUES_PROPERTY));
    for (var, prop) in mappings {
        let mapping = oxrdf::BlankNode::default();
        emit(
            NamedOrBlankNode::BlankNode(search.clone()),
            hydra("mapping"),
            Term::BlankNode(mapping.clone()),
        );
        emit(
            NamedOrBlankNode::BlankNode(mapping.clone()),
            hydra("variable"),
            Term::Literal(Literal::new_simple_literal(var)),
        );
        emit(
            NamedOrBlankNode::BlankNode(mapping),
            hydra("property"),
            // `prop` is a constant rdf: IRI; if it ever failed to parse, the mapping simply
            // omits the property link (still valid RDF) rather than panicking.
            match iri(prop) {
                Some(n) => Term::NamedNode(n),
                None => continue,
            },
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:alice a ex:Person ; ex:knows ex:bob ; ex:name "Alice" .
        ex:bob   a ex:Person ; ex:name "Bob" .
        ex:carol a ex:Person ; ex:knows ex:alice .
    "#;

    fn graph() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    // ---- Pattern parsing -------------------------------------------------------------

    #[test]
    fn parse_unbound_is_all() {
        assert!(TriplePattern::parse(None, None, None).unwrap().is_all());
        // Empty / whitespace-only are also variables.
        assert!(TriplePattern::parse(Some(""), Some("  "), None)
            .unwrap()
            .is_all());
    }

    #[test]
    fn parse_iri_and_literal_terms() {
        let p = TriplePattern::parse(
            Some("<http://ex/alice>"),
            Some("<http://ex/name>"),
            Some("\"Alice\""),
        )
        .unwrap();
        assert_eq!(
            p.subject,
            Some(Term::NamedNode(NamedNode::new("http://ex/alice").unwrap()))
        );
        assert_eq!(p.predicate, Some(NamedNode::new("http://ex/name").unwrap()));
        assert!(matches!(p.object, Some(Term::Literal(_))));
        assert!(!p.is_all());
    }

    #[test]
    fn parse_rejects_non_iri_predicate() {
        let err = TriplePattern::parse(None, Some("\"not an iri\""), None);
        assert!(err.is_err(), "a literal predicate must be rejected");
    }

    #[test]
    fn parse_rejects_malformed_term() {
        assert!(TriplePattern::parse(Some("not a term"), None, None).is_err());
    }

    // ---- Pattern match → correct triples ---------------------------------------------

    #[test]
    fn predicate_bound_matches_all_with_that_predicate() {
        // ?s ex:knows ?o  → alice knows bob, carol knows alice (2 triples).
        let p = TriplePattern::parse(None, Some("<http://ex/knows>"), None).unwrap();
        let f = evaluate(&graph(), &p, 0, DEFAULT_PAGE_SIZE);
        assert_eq!(f.total_estimate, 2);
        assert_eq!(f.triples.len(), 2);
        assert!(f
            .triples
            .iter()
            .all(|t| t.predicate.as_str() == "http://ex/knows"));
    }

    #[test]
    fn subject_bound_matches_that_subject() {
        // ex:alice ?p ?o → a Person, knows bob, name "Alice" (3 triples).
        let p = TriplePattern::parse(Some("<http://ex/alice>"), None, None).unwrap();
        let f = evaluate(&graph(), &p, 0, DEFAULT_PAGE_SIZE);
        assert_eq!(f.total_estimate, 3);
        assert_eq!(f.triples.len(), 3);
        assert!(f.triples.iter().all(|t| t.subject
            == oxrdf::NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/alice").unwrap())));
    }

    #[test]
    fn fully_bound_exact_match() {
        let p = TriplePattern::parse(
            Some("<http://ex/alice>"),
            Some("<http://ex/knows>"),
            Some("<http://ex/bob>"),
        )
        .unwrap();
        let f = evaluate(&graph(), &p, 0, DEFAULT_PAGE_SIZE);
        assert_eq!(f.total_estimate, 1);
        assert_eq!(f.triples.len(), 1);
    }

    #[test]
    fn absent_term_yields_empty_page_no_match() {
        // A bound term that is NOT in the dictionary: empty page, estimate 0.
        let p = TriplePattern::parse(Some("<http://ex/nobody>"), None, None).unwrap();
        let f = evaluate(&graph(), &p, 0, DEFAULT_PAGE_SIZE);
        assert_eq!(f.total_estimate, 0);
        assert!(f.triples.is_empty());
        assert!(!f.has_next());
        assert!(!f.has_previous());
    }

    #[test]
    fn all_pattern_returns_every_triple() {
        // ?s ?p ?o over the whole graph: 3 type + 2 knows + 2 name = 7 triples.
        let p = TriplePattern::parse(None, None, None).unwrap();
        let f = evaluate(&graph(), &p, 0, DEFAULT_PAGE_SIZE);
        assert_eq!(f.total_estimate, 7);
        assert_eq!(f.triples.len(), 7);
    }

    // ---- Paging boundary -------------------------------------------------------------

    #[test]
    fn paging_splits_and_next_boundary() {
        // Page size 3 over 7 triples → pages 0,1 have next; page 2 (the 7th item) does not.
        let p = TriplePattern::parse(None, None, None).unwrap();
        let g = graph();

        let p0 = evaluate(&g, &p, 0, 3);
        assert_eq!(p0.triples.len(), 3);
        assert!(p0.has_next());
        assert!(!p0.has_previous());

        let p1 = evaluate(&g, &p, 1, 3);
        assert_eq!(p1.triples.len(), 3);
        assert!(p1.has_next());
        assert!(p1.has_previous());

        // Page 2: the final item (7 = 2*3 + 1) — has previous, NO next.
        let p2 = evaluate(&g, &p, 2, 3);
        assert_eq!(p2.triples.len(), 1);
        assert!(!p2.has_next());
        assert!(p2.has_previous());

        // A page beyond the data is empty but still consistent.
        let p3 = evaluate(&g, &p, 3, 3);
        assert!(p3.triples.is_empty());
        assert!(!p3.has_next());
        assert!(p3.has_previous());
    }

    #[test]
    fn pages_partition_the_results_without_overlap() {
        // Concatenating page 0 + page 1 + page 2 (size 3) reproduces the full result set.
        let p = TriplePattern::parse(None, None, None).unwrap();
        let g = graph();
        let mut all = Vec::new();
        for page in 0..3 {
            all.extend(evaluate(&g, &p, page, 3).triples);
        }
        let full = evaluate(&g, &p, 0, DEFAULT_PAGE_SIZE).triples;
        // Same multiset of triples (the scan order is deterministic, so they even align).
        assert_eq!(all.len(), full.len());
        let mut a: Vec<String> = all.iter().map(|t| t.to_string()).collect();
        let mut b: Vec<String> = full.iter().map(|t| t.to_string()).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    // ---- Hydra controls --------------------------------------------------------------

    #[test]
    fn fragment_carries_hydra_controls() {
        let p = TriplePattern::parse(None, None, None).unwrap();
        let f = evaluate(&graph(), &p, 0, 3);
        let next = "http://host/tpf?page=2";
        let triples = fragment_triples(
            &f,
            "http://host/tpf?page=1",
            "http://host/tpf",
            "http://host/tpf{?subject,predicate,object}",
            f.has_next().then_some(next),
            None, // page 0 → no previous
            Some("http://host/tpf?page=0"),
            Some(&format!("http://host/tpf?page={}", f.last_page())),
        );
        let doc = crate::graph::triples_to_ntriples(&triples).unwrap();
        // totalItems + itemsPerPage estimate.
        assert!(doc.contains("hydra/core#totalItems"), "{doc}");
        assert!(doc.contains("hydra/core#itemsPerPage"), "{doc}");
        assert!(doc.contains("void#triples"), "{doc}");
        // The search template control.
        assert!(doc.contains("hydra/core#search"), "{doc}");
        assert!(doc.contains("hydra/core#template"), "{doc}");
        assert!(doc.contains("subject,predicate,object"), "{doc}");
        // next present (page 0 has a next), previous absent.
        assert!(doc.contains("hydra/core#next"), "{doc}");
        assert!(!doc.contains("hydra/core#previous"), "{doc}");
        // first + last always present (the fuller Hydra paging vocabulary, sq-dxhb).
        assert!(doc.contains("hydra/core#first"), "{doc}");
        assert!(doc.contains("hydra/core#last"), "{doc}");
        // The data triples are in the document too (7 total > 3 page → 3 data triples present).
        assert_eq!(f.triples.len(), 3);
    }

    #[test]
    fn fragment_previous_present_on_later_page() {
        let p = TriplePattern::parse(None, None, None).unwrap();
        let f = evaluate(&graph(), &p, 1, 3);
        let triples = fragment_triples(
            &f,
            "http://host/tpf?page=1",
            "http://host/tpf",
            "http://host/tpf{?subject,predicate,object}",
            f.has_next().then_some("http://host/tpf?page=2"),
            f.has_previous().then_some("http://host/tpf?page=0"),
            Some("http://host/tpf?page=0"),
            Some(&format!("http://host/tpf?page={}", f.last_page())),
        );
        let doc = crate::graph::triples_to_ntriples(&triples).unwrap();
        assert!(doc.contains("hydra/core#previous"), "{doc}");
        assert!(doc.contains("hydra/core#next"), "{doc}");
    }

    #[test]
    fn empty_pattern_page_still_well_formed_rdf() {
        // An empty page (no matches) must still produce a valid RDF document with the metadata
        // (totalItems 0) and the search control — a TPF client relies on that to learn the count.
        let p = TriplePattern::parse(Some("<http://ex/nobody>"), None, None).unwrap();
        let f = evaluate(&graph(), &p, 0, DEFAULT_PAGE_SIZE);
        let triples = fragment_triples(
            &f,
            "http://host/tpf?subject=%3Chttp%3A%2F%2Fex%2Fnobody%3E",
            "http://host/tpf",
            "http://host/tpf{?subject,predicate,object}",
            None,
            None,
            Some("http://host/tpf?subject=%3Chttp%3A%2F%2Fex%2Fnobody%3E&page=0"),
            Some("http://host/tpf?subject=%3Chttp%3A%2F%2Fex%2Fnobody%3E&page=0"),
        );
        let ttl = crate::graph::triples_to_turtle(&triples).unwrap();
        // Re-parses as valid Turtle.
        let g = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
        assert!(g.len() >= 3, "metadata + search control present: {ttl}");
        let nt = crate::graph::triples_to_ntriples(&triples).unwrap();
        // The count is a typed integer literal: `…#totalItems> "0"^^<xsd:integer>`.
        assert!(nt.contains("totalItems> \"0\""), "{nt}");
    }

    // ---- Hydra first/last paging vocabulary (sq-dxhb) ---------------------------------

    #[test]
    fn last_page_index_matches_paging() {
        // 7 triples, page size 3 → pages 0,1,2 ; the last page is index 2.
        let p = TriplePattern::parse(None, None, None).unwrap();
        let g = graph();
        for page in 0..4 {
            assert_eq!(evaluate(&g, &p, page, 3).last_page(), 2, "page {page}");
        }
        // The whole result on one page → last page is 0.
        assert_eq!(evaluate(&g, &p, 0, DEFAULT_PAGE_SIZE).last_page(), 0);
        // An exact multiple: 6 matches / size 3 = 2 full pages → last index 1 (not 2).
        let knows = TriplePattern::parse(None, Some("<http://ex/knows>"), None).unwrap();
        // 2 `knows` triples, size 1 → 2 pages, last index 1.
        assert_eq!(evaluate(&g, &knows, 0, 1).last_page(), 1);
        // An empty result still has a (single, empty) page 0 — last index 0.
        let none = TriplePattern::parse(Some("<http://ex/nobody>"), None, None).unwrap();
        assert_eq!(evaluate(&g, &none, 0, 3).last_page(), 0);
    }

    #[test]
    fn fragment_emits_first_and_last() {
        let p = TriplePattern::parse(None, None, None).unwrap();
        let f = evaluate(&graph(), &p, 1, 3);
        let triples = fragment_triples(
            &f,
            "http://host/tpf?page=1",
            "http://host/tpf",
            "http://host/tpf{?subject,predicate,object}",
            f.has_next().then_some("http://host/tpf?page=2"),
            f.has_previous().then_some("http://host/tpf?page=0"),
            Some("http://host/tpf?page=0"),
            Some(&format!("http://host/tpf?page={}", f.last_page())),
        );
        let doc = crate::graph::triples_to_ntriples(&triples).unwrap();
        assert!(doc.contains("hydra/core#first"), "{doc}");
        assert!(doc.contains("hydra/core#last"), "{doc}");
        // first → page 0, last → page 2 (7 triples / size 3).
        assert!(doc.contains("page=0"), "{doc}");
        assert!(doc.contains("page=2"), "{doc}");
    }

    // ---- brTPF — bindings-restricted fragments (sq-dxhb) ------------------------------

    #[cfg(feature = "brtpf")]
    mod brtpf {
        use super::*;

        #[test]
        fn parse_bindings_one_per_line() {
            let b = parse_bindings("s=<http://ex/alice>\ns=<http://ex/carol>").unwrap();
            assert_eq!(b.len(), 2);
            assert_eq!(
                b[0].subject,
                Some(Term::NamedNode(NamedNode::new("http://ex/alice").unwrap()))
            );
            assert_eq!(
                b[1].subject,
                Some(Term::NamedNode(NamedNode::new("http://ex/carol").unwrap()))
            );
            assert!(b[0].predicate.is_none() && b[0].object.is_none());
        }

        #[test]
        fn parse_bindings_multi_position_and_long_keys() {
            // `subject`/`object` long forms + multiple positions on one line.
            let b = parse_bindings("subject=<http://ex/alice> object=<http://ex/bob>").unwrap();
            assert_eq!(b.len(), 1);
            assert_eq!(
                b[0].subject,
                Some(Term::NamedNode(NamedNode::new("http://ex/alice").unwrap()))
            );
            assert_eq!(
                b[0].object,
                Some(Term::NamedNode(NamedNode::new("http://ex/bob").unwrap()))
            );
        }

        #[test]
        fn parse_bindings_empty_payload_and_blank_lines() {
            assert!(parse_bindings("").unwrap().is_empty());
            assert!(parse_bindings("\n  \n\t\n").unwrap().is_empty());
        }

        #[test]
        fn parse_bindings_rejects_malformed() {
            assert!(parse_bindings("s").is_err()); // no '='
            assert!(parse_bindings("bogus=<http://ex/x>").is_err()); // bad position
            assert!(parse_bindings("p=\"a literal\"").is_err()); // non-IRI predicate
            assert!(parse_bindings("s=not a term").is_err()); // malformed term
        }

        #[test]
        fn restricts_to_compatible_bindings_only() {
            // ?s ex:knows ?o has alice→bob, carol→alice. Restrict to {?s=alice}: only alice→bob.
            let g = graph();
            let p = TriplePattern::parse(None, Some("<http://ex/knows>"), None).unwrap();
            let bindings = parse_bindings("s=<http://ex/alice>").unwrap();
            let f = evaluate_brtpf(&g, &p, &bindings, 0, DEFAULT_PAGE_SIZE);
            assert_eq!(f.total_estimate, 1);
            assert_eq!(f.triples.len(), 1);
            assert_eq!(f.triples[0].subject.to_string(), "<http://ex/alice>");
            assert_eq!(f.triples[0].object.to_string(), "<http://ex/bob>");
        }

        #[test]
        fn union_over_multiple_bindings() {
            // Restrict to {?s=alice, ?s=carol}: BOTH knows triples (the union).
            let g = graph();
            let p = TriplePattern::parse(None, Some("<http://ex/knows>"), None).unwrap();
            let bindings = parse_bindings("s=<http://ex/alice>\ns=<http://ex/carol>").unwrap();
            let f = evaluate_brtpf(&g, &p, &bindings, 0, DEFAULT_PAGE_SIZE);
            assert_eq!(f.total_estimate, 2);
            assert_eq!(f.triples.len(), 2);
        }

        #[test]
        fn binding_absent_from_data_contributes_nothing() {
            // A binding whose term is not in the dictionary matches nothing; the other still does.
            let g = graph();
            let p = TriplePattern::parse(None, Some("<http://ex/knows>"), None).unwrap();
            let bindings = parse_bindings("s=<http://ex/nobody>\ns=<http://ex/alice>").unwrap();
            let f = evaluate_brtpf(&g, &p, &bindings, 0, DEFAULT_PAGE_SIZE);
            assert_eq!(f.total_estimate, 1, "only alice contributes");
        }

        #[test]
        fn binding_contradicting_the_pattern_contributes_nothing() {
            // Pattern fixes ?s=alice; a binding ?s=carol CLASHES → contributes nothing. A second
            // binding ?o=bob is compatible (specialises the object) → the one alice→bob triple.
            let g = graph();
            let p =
                TriplePattern::parse(Some("<http://ex/alice>"), Some("<http://ex/knows>"), None)
                    .unwrap();
            let bindings = parse_bindings("s=<http://ex/carol>\no=<http://ex/bob>").unwrap();
            let f = evaluate_brtpf(&g, &p, &bindings, 0, DEFAULT_PAGE_SIZE);
            assert_eq!(f.total_estimate, 1);
            assert_eq!(f.triples[0].object.to_string(), "<http://ex/bob>");
        }

        #[test]
        fn empty_binding_set_is_plain_tpf() {
            // No restriction ⇒ identical to plain evaluate (the μ₀-only case).
            let g = graph();
            let p = TriplePattern::parse(None, Some("<http://ex/knows>"), None).unwrap();
            let restricted = evaluate_brtpf(&g, &p, &[], 0, DEFAULT_PAGE_SIZE);
            let plain = evaluate(&g, &p, 0, DEFAULT_PAGE_SIZE);
            assert_eq!(restricted.total_estimate, plain.total_estimate);
            assert_eq!(restricted.triples.len(), plain.triples.len());
        }

        #[test]
        fn restricted_result_pages_consistently() {
            // Restrict ?s ?p ?o to {?s=alice} (alice has 3 triples), page size 2 → pages 0 (2) + 1 (1).
            let g = graph();
            let p = TriplePattern::parse(None, None, None).unwrap();
            let bindings = parse_bindings("s=<http://ex/alice>").unwrap();
            let p0 = evaluate_brtpf(&g, &p, &bindings, 0, 2);
            assert_eq!(p0.total_estimate, 3);
            assert_eq!(p0.triples.len(), 2);
            assert!(p0.has_next());
            assert!(!p0.has_previous());
            assert_eq!(p0.last_page(), 1);
            let p1 = evaluate_brtpf(&g, &p, &bindings, 1, 2);
            assert_eq!(p1.triples.len(), 1);
            assert!(!p1.has_next());
            assert!(p1.has_previous());
            // The two pages partition the 3-triple restricted result with no overlap.
            let mut all: Vec<String> = p0
                .triples
                .iter()
                .chain(p1.triples.iter())
                .map(|t| t.to_string())
                .collect();
            all.sort();
            all.dedup();
            assert_eq!(all.len(), 3, "no duplicate across pages");
            assert!(all.iter().all(|t| t.contains("http://ex/alice")));
        }

        #[test]
        fn overlapping_bindings_deduplicate() {
            // Two bindings that select the SAME triple must not double-count it.
            let g = graph();
            let p = TriplePattern::parse(None, None, None).unwrap();
            // Both {?s=alice,?o=bob} and {?p=knows} select alice→bob (among others); the union
            // counts each matching triple once.
            let bindings =
                parse_bindings("s=<http://ex/alice> o=<http://ex/bob>\np=<http://ex/knows>")
                    .unwrap();
            let f = evaluate_brtpf(&g, &p, &bindings, 0, DEFAULT_PAGE_SIZE);
            // {alice,bob} → alice knows bob (1). {knows} → alice→bob, carol→alice (2). Union = 2.
            assert_eq!(f.total_estimate, 2);
            assert_eq!(f.triples.len(), 2);
        }

        // ---- sq-r74h: DoS caps on the binding set (count + payload bytes) ----------------

        #[test]
        fn capped_unlimited_matches_uncapped() {
            // UNLIMITED trips no cap — identical to the uncapped parse_bindings.
            let payload = "s=<http://ex/alice>\ns=<http://ex/carol>";
            let capped = parse_bindings_capped(payload, BindingLimits::UNLIMITED).unwrap();
            let plain = parse_bindings(payload).unwrap();
            assert_eq!(capped, plain);
            assert_eq!(capped.len(), 2);
        }

        #[test]
        fn mapping_count_cap_rejects_excess_as_too_large() {
            // 3 mappings, cap of 2 → TooLarge (413), naming the count cap, not the payload.
            let payload = "s=<http://ex/a>\ns=<http://ex/b>\ns=<http://ex/c>";
            let limits = BindingLimits {
                max_mappings: 2,
                max_payload_bytes: 0,
            };
            let err = parse_bindings_capped(payload, limits).unwrap_err();
            assert!(matches!(err, BindingError::TooLarge(_)), "{err:?}");
            assert!(err.to_string().contains("brtpf-max-bindings"), "{err}");
            // Exactly at the cap is accepted (boundary: count == cap is OK, cap+1 is not).
            let at_cap = "s=<http://ex/a>\ns=<http://ex/b>";
            assert_eq!(parse_bindings_capped(at_cap, limits).unwrap().len(), 2);
        }

        #[test]
        fn empty_mappings_do_not_count_toward_the_cap() {
            // Blank lines + an all-blank μ₀ line are dropped, so they don't consume the count cap:
            // 2 real mappings interleaved with skipped lines is fine under a cap of 2.
            let payload = "\ns=<http://ex/a>\n   \ns=<http://ex/b>\n";
            let limits = BindingLimits {
                max_mappings: 2,
                max_payload_bytes: 0,
            };
            assert_eq!(parse_bindings_capped(payload, limits).unwrap().len(), 2);
        }

        #[test]
        fn payload_byte_cap_rejects_oversize_as_too_large() {
            let payload = "s=<http://ex/alice>"; // 19 bytes
            let limits = BindingLimits {
                max_mappings: 0,
                max_payload_bytes: 10,
            };
            let err = parse_bindings_capped(payload, limits).unwrap_err();
            assert!(matches!(err, BindingError::TooLarge(_)), "{err:?}");
            assert!(err.to_string().contains("brtpf-max-values-bytes"), "{err}");
            // A payload exactly at the byte cap is accepted.
            let ok = BindingLimits {
                max_mappings: 0,
                max_payload_bytes: payload.len(),
            };
            assert_eq!(parse_bindings_capped(payload, ok).unwrap().len(), 1);
        }

        #[test]
        fn byte_cap_is_checked_before_parsing_so_oversize_garbage_is_413_not_400() {
            // A payload that is BOTH oversize AND malformed is reported as 413 (TooLarge), because
            // the byte cap is enforced before any line is parsed — we never parse over-cap input.
            let garbage = "this is not a valid binding payload at all";
            let limits = BindingLimits {
                max_mappings: 0,
                max_payload_bytes: 5,
            };
            let err = parse_bindings_capped(garbage, limits).unwrap_err();
            assert!(matches!(err, BindingError::TooLarge(_)), "{err:?}");
        }

        #[test]
        fn malformed_under_caps_is_still_malformed_not_too_large() {
            // Within both caps, a malformed pair is Malformed (→ 400), not TooLarge (→ 413).
            let limits = BindingLimits {
                max_mappings: 100,
                max_payload_bytes: 1024,
            };
            let err = parse_bindings_capped("bogus=<http://ex/x>", limits).unwrap_err();
            assert!(matches!(err, BindingError::Malformed(_)), "{err:?}");
        }

        #[test]
        fn too_large_message_does_not_echo_the_payload() {
            // The TooLarge message must not leak any caller-supplied term (same no-echo posture as
            // the malformed-term path) — it names only the cap.
            const SENTINEL: &str = "brtpf_secret_sentinel";
            let payload = format!("s=<http://ex/{SENTINEL}>");
            let limits = BindingLimits {
                max_mappings: 0,
                max_payload_bytes: 5,
            };
            let err = parse_bindings_capped(&payload, limits).unwrap_err();
            assert!(!err.to_string().contains(SENTINEL), "leaked payload: {err}");
        }
    }
}
