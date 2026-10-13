//! Planner-to-physical bridge (design §4.2) — **Phase 3**.
//!
//! `sparq-fedplan` plans a federated BGP into a [`JoinTree`](sparq_fedplan::JoinTree) of
//! **pattern indices** (`JoinNode::Leaf.pattern` / `JoinNode::Join.right`, both `usize`
//! into the BGP) and **source indices** (`PatternSources.candidates[].source`, a `usize`
//! into the descriptor slice). It speaks *indices only* — there is no endpoint-URL or
//! adapter mapping anywhere in the plan (the Phase-0 finding the design §3.2(4) calls out
//! as missing). This module supplies that missing layer:
//!
//! * [`SourceResolver`] — the **index → adapter resolution layer**. It pairs the BGP (so a
//!   plan `pattern: usize` resolves to a [`TriplePattern`](sparq_fedplan::TriplePattern))
//!   with the heterogeneous source adapters (so a `source: usize` resolves to a
//!   `&dyn `[`FederatedSource`](crate::FederatedSource)). The planner's `select_sources` /
//!   `plan_bgp` inputs and this resolver's adapter slice are kept in **the same source
//!   order**, so index `i` is descriptor `i` is adapter `i` — the resolver range-checks
//!   that invariant rather than trusting it.
//! * [`lower_leaf`] — lowers one BGP triple pattern to the most-precise [`SubQuery`] a
//!   full SPARQL endpoint can answer for that leaf (`SELECT <vars> WHERE { tp }`). This is
//!   the Phase-3 lowering: a single-pattern SELECT per leaf. Capability-aware narrowing of
//!   the pushed sub-query (projection trimming, FILTER / VALUES bind-join, ORDER/LIMIT) is
//!   the `pushdown` module's job in Phase 4 — `lower_leaf` is the seam Phase 4 refines.
//!
//! The client does **not** write a new planner: the cost-based join order, the bind-vs-hash
//! decision, and the characteristic-set star cardinality all stay in `sparq-fedplan`. This
//! bridge only supplies descriptors/adapters and consumes the produced `JoinTree`. The
//! materialised single-source *interpreter* that walks that tree lives in
//! [`crate::operators`].
//!
//! # Honest scope (Phase 3, single source)
//!
//! This phase lowers + interprets a plan against **one** source and asserts the
//! materialised answer equals local `sparq-engine` evaluation (the core correctness
//! property — see [`crate::operators`]). The lowering surface ([`SourceResolver`] +
//! [`lower_leaf`]) is written for the heterogeneous-source case, but the *interpreter* is
//! single-source: multi-source UNION-per-leaf fan-out, the bind-join operator, and the
//! `StreamJoin` feeder are Phase 5. The resolver and lowering are the real, tested seam
//! those phases build on.
//
// [OPUS-4.8] sq-j27p (epic sq-dnko): Phase-3 planner bridge — index→adapter resolution +
// per-leaf lowering. Flagged for Fable re-review when available.

use crate::source::{FederatedSource, FragPattern, FragTerm, PatternTerm, SubQuery};
use sparq_fedplan::{Bgp, Term, TriplePattern};

#[cfg(feature = "pattern_probe")]
use crate::discovery::{PatternProbeOutcome, PatternProbeSession};
#[cfg(feature = "pattern_probe")]
use sparq_fedplan::{select_sources, PatternSources, SourceCandidate, SourceDescriptor};

/// One endpoint and its optional served statistics for pattern-probed source selection.
///
/// The slice passed to [`select_sources_with_pattern_probes`] establishes source indices:
/// item `i` here is source `i` in the returned [`PatternSources`]. A missing descriptor means
/// capability/cardinality is unknown, never empty; the source starts with the configured
/// uniform fallback and is removed only by a definitive ASK `false`. [GPT-5.6] sq-fx5id.
#[cfg(feature = "pattern_probe")]
#[derive(Debug, Clone, Copy)]
pub struct ProbeSource<'a> {
    /// Stable SPARQL endpoint URL used for pattern probes and cache identity.
    pub endpoint: &'a str,
    /// Served VoID/characteristic-set descriptor, when discovery found one.
    pub descriptor: Option<&'a SourceDescriptor>,
}

/// Select sources and refine missing per-pattern statistics with bounded live probes.
///
/// Served VoID cardinalities always win and issue no request. When a source has no descriptor,
/// or its descriptor lacks a cardinality for this pattern, the per-query `probe` session may:
///
/// * remove the source only after an exact ASK `false`;
/// * replace the uniform fallback with a successful capped SELECT observation; or
/// * retain the source and fallback unchanged on timeout, HTTP/parse error, inconsistency, or
///   budget exhaustion.
///
/// This function changes source ranking/join order, never the set of answer-producing sources
/// unless the endpoint itself definitively reported no match. [GPT-5.6] sq-fx5id.
#[cfg(feature = "pattern_probe")]
pub fn select_sources_with_pattern_probes(
    bgp: &Bgp,
    sources: &[ProbeSource<'_>],
    probe: &mut PatternProbeSession<'_>,
) -> Vec<PatternSources> {
    bgp.patterns
        .iter()
        .enumerate()
        .map(|(pattern_index, pattern)| {
            let mut candidates = Vec::new();
            for (source_index, source) in sources.iter().enumerate() {
                let (baseline, has_served_cardinality) = match source.descriptor {
                    Some(descriptor) => {
                        let selected = select_sources(
                            &Bgp::new(vec![pattern.clone()]),
                            std::slice::from_ref(descriptor),
                        );
                        let Some(candidate) = selected[0].candidates.first() else {
                            // The served capability descriptor definitively excludes this
                            // source. Preserve fedplan's existing recall-safe prune.
                            continue;
                        };
                        (
                            candidate.estimated_cardinality,
                            descriptor_has_cardinality(pattern, descriptor),
                        )
                    }
                    None => (probe.config().fallback_cardinality, false),
                };

                let outcome = if has_served_cardinality {
                    PatternProbeOutcome::Unknown
                } else {
                    probe.probe(source.endpoint, pattern)
                };
                match outcome {
                    PatternProbeOutcome::Empty => {}
                    PatternProbeOutcome::Cardinality { observed, .. } => {
                        candidates.push(SourceCandidate {
                            source: source_index,
                            estimated_cardinality: observed as f64,
                        });
                    }
                    PatternProbeOutcome::Unknown => candidates.push(SourceCandidate {
                        source: source_index,
                        estimated_cardinality: baseline,
                    }),
                }
            }
            PatternSources {
                pattern: pattern_index,
                candidates,
            }
        })
        .collect()
}

#[cfg(feature = "pattern_probe")]
fn descriptor_has_cardinality(tp: &TriplePattern, descriptor: &SourceDescriptor) -> bool {
    tp.predicate_iri()
        .map(|predicate| descriptor.predicate(predicate).is_some())
        .unwrap_or(descriptor.total_triples > 0)
}

/// An error from resolving a plan index — the plan referenced an out-of-range
/// pattern/source for the BGP / adapter slice this resolver carries. These are
/// *programmer* errors (a plan and a resolver built from different inputs), surfaced
/// rather than panicked so the interpreter can fail closed. [OPUS-4.8] sq-j27p.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// A plan pattern index is out of range for the BGP this resolver carries.
    PatternOutOfRange { index: usize, patterns: usize },
    /// A plan source index is out of range for the adapter slice.
    SourceOutOfRange { index: usize, sources: usize },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::PatternOutOfRange { index, patterns } => write!(
                f,
                "planner bridge: pattern index {} out of range (BGP has {} patterns)",
                index, patterns
            ),
            ResolveError::SourceOutOfRange { index, sources } => write!(
                f,
                "planner bridge: source index {} out of range ({} sources)",
                index, sources
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// The **index → adapter resolution layer** (the Phase-0 finding's missing piece).
///
/// A `sparq-fedplan` plan addresses patterns and sources by `usize` index; nothing in the
/// plan knows an endpoint URL or holds an adapter. `SourceResolver` is the one place that
/// bridges those indices back to the concrete query surface:
///
/// * a plan **pattern index** → the [`TriplePattern`] it names ([`pattern`](Self::pattern));
/// * a plan **source index** → the `&dyn `[`FederatedSource`] adapter that can answer it
///   ([`source`](Self::source)).
///
/// The contract the rest of federation relies on: the `descriptors` slice handed to
/// [`select_sources`] / [`plan_bgp`](sparq_fedplan::plan_bgp)
/// and the `adapters` slice handed here are in **the same order** — descriptor `i`
/// describes the source adapter `i`. The resolver does not re-derive that order; it is the
/// caller's single source of truth, and every lookup is range-checked so a mismatched plan
/// fails closed with a [`ResolveError`] instead of indexing past the end.
///
/// `SourceResolver` borrows the BGP and the adapters (it never owns transports), so a
/// caller can build one per query without cloning the heterogeneous source set.
/// [OPUS-4.8] sq-j27p.
pub struct SourceResolver<'a> {
    bgp: &'a Bgp,
    adapters: &'a [&'a dyn FederatedSource],
}

impl<'a> SourceResolver<'a> {
    /// Pair a BGP with the source adapters, **in the same order** as the descriptor slice
    /// the planner was given. Index `i` here must be the same source as descriptor `i` in
    /// `select_sources`/`plan_bgp`.
    pub fn new(bgp: &'a Bgp, adapters: &'a [&'a dyn FederatedSource]) -> Self {
        SourceResolver { bgp, adapters }
    }

    /// The BGP this resolver maps plan pattern indices into.
    pub fn bgp(&self) -> &Bgp {
        self.bgp
    }

    /// How many source adapters this resolver carries (the valid `source` index range).
    pub fn source_count(&self) -> usize {
        self.adapters.len()
    }

    /// Resolve a plan **pattern index** to its [`TriplePattern`]. Range-checked.
    pub fn pattern(&self, index: usize) -> Result<&'a TriplePattern, ResolveError> {
        self.bgp
            .patterns
            .get(index)
            .ok_or(ResolveError::PatternOutOfRange {
                index,
                patterns: self.bgp.patterns.len(),
            })
    }

    /// Resolve a plan **source index** to its adapter. Range-checked.
    pub fn source(&self, index: usize) -> Result<&'a dyn FederatedSource, ResolveError> {
        self.adapters
            .get(index)
            .copied()
            .ok_or(ResolveError::SourceOutOfRange {
                index,
                sources: self.adapters.len(),
            })
    }
}

/// Lower one BGP triple pattern to the [`SubQuery`] a full SPARQL endpoint answers for that
/// leaf: `SELECT <distinct-vars> WHERE { <tp> }`, projecting exactly the pattern's
/// variables in position order (subject, predicate, object), de-duplicated.
///
/// The rendered SPARQL is what the Phase-2 [`Endpoint`](crate::source::Endpoint) adapter
/// forwards verbatim to its transport. Each position is rendered strictly as one SPARQL term
/// (see [`RenderError`]); a position that cannot be is an error and nothing is sent. A
/// pattern with **no** variables (all three positions bound) lowers to a
/// `SELECT * WHERE { tp } LIMIT 1` existence probe, so the interpreter still sees a
/// (possibly empty) solution table rather than a boolean.
///
/// This is the deliberately-simple Phase-3 lowering — one self-contained single-pattern
/// SELECT per leaf. Capability-aware sub-query *construction* (only join+output vars,
/// pushable FILTERs, VALUES bind-join blocks, ORDER/LIMIT) is Phase 4's `pushdown` module;
/// `lower_leaf` is the seam it narrows. [OPUS-4.8] sq-j27p.
pub fn lower_leaf(tp: &TriplePattern) -> Result<SubQuery, RenderError> {
    debug_assert!(
        !matches!(tp.subject, Term::Literal(_)),
        "lower_leaf: literal in subject position is not valid RDF/SPARQL 1.1; \
         sparq-fedplan must never produce such a pattern"
    );
    let project = pattern_vars(tp);
    let pattern = render_pattern_clause(tp)?;
    let sparql = if project.is_empty() {
        // Fully-bound pattern: a 1-row existence probe (kept as a SELECT so the
        // interpreter's relation model is uniform — never an ASK boolean).
        format!("SELECT * WHERE {{ {pattern} }} LIMIT 1")
    } else {
        let proj = project
            .iter()
            .map(|v| format!("?{}", v))
            .collect::<Vec<_>>()
            .join(" ");
        format!("SELECT {proj} WHERE {{ {pattern} }}")
    };
    Ok(SubQuery { sparql, project })
}

/// Render one triple pattern's body without the surrounding query form. Shared by normal leaf
/// lowering and the opt-in pattern probe so the two paths cannot disagree about term escaping.
/// [GPT-5.6] sq-fx5id.
pub(crate) fn render_pattern_clause(tp: &TriplePattern) -> Result<String, RenderError> {
    Ok(format!(
        "{} {} {}",
        render_term(&tp.subject)?,
        render_term(&tp.predicate)?,
        render_term(&tp.object)?
    ))
}

/// The variables a pattern produces, de-duplicated in position order (subject, predicate,
/// object). Mirrors `TriplePattern::vars` but returns owned names (the [`SubQuery::project`]
/// hint the interpreter binds rows against). [OPUS-4.8] sq-j27p.
pub fn pattern_vars(tp: &TriplePattern) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in [&tp.subject, &tp.predicate, &tp.object] {
        if let Term::Var(v) = t {
            if !out.iter().any(|n| n == &v.0) {
                out.push(v.0.clone());
            }
        }
    }
    out
}

/// Lower one BGP triple pattern to the [`FragPattern`] a Triple-Pattern-Fragments source
/// ([`TpfSource`](crate::source::TpfSource) / [`BrTpfSource`](crate::source::BrTpfSource))
/// answers for that leaf — the TPF/brTPF access unit is exactly one triple pattern.
///
/// Each fedplan [`Term`] position maps to a [`PatternTerm`]: a variable becomes
/// [`PatternTerm::Var`] (the bare name, so it round-trips through the fragment's variable
/// binding), a bound IRI becomes a [`FragTerm::Iri`], and a bound literal becomes a
/// [`FragTerm::Literal`] carrying its already-rendered SPARQL/N-Triples lexical form verbatim
/// (the `Term::Literal` model stores literals already rendered, and the `FragTerm::Literal`
/// model likewise stores the decorated form — so the lexical identity is preserved end-to-end,
/// which the fragment server's term parser and the adapter's `bind_triple` both compare on).
///
/// This is the fragment-source twin of [`lower_leaf`] (which produces a SPARQL `SubQuery` for an
/// [`Endpoint`](crate::source::Endpoint)); the interpreter dispatches on the source's interface
/// to pick which lowering to use. [OPUS-4.8] sq-yzca.
pub fn lower_leaf_fragment(tp: &TriplePattern) -> FragPattern {
    FragPattern::new(
        term_to_pattern_term(&tp.subject),
        term_to_pattern_term(&tp.predicate),
        term_to_pattern_term(&tp.object),
    )
}

/// Map one fedplan [`Term`] position to a [`PatternTerm`] for a [`FragPattern`]. [OPUS-4.8].
fn term_to_pattern_term(t: &Term) -> PatternTerm {
    match t {
        Term::Var(v) => PatternTerm::Var(v.0.clone()),
        Term::Iri(iri) => PatternTerm::Bound(FragTerm::Iri(iri.clone())),
        // A bound literal carries its already-rendered lexical form verbatim (see the doc on
        // `lower_leaf_fragment` / `FragTerm::Literal`). The fragment server compares on it.
        Term::Literal(lit) => PatternTerm::Bound(FragTerm::Literal(lit.clone())),
    }
}

/// A pattern position that cannot be written as a SPARQL term, so the pattern is not sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// A variable name that is not a SPARQL `VARNAME`.
    Var(String),
    /// An IRI containing a character SPARQL's `IRIREF` forbids: a control or space, or one of
    /// `<`, `>`, `"`, `{`, `}`, `|`, `^`, `\`, or the backtick.
    Iri(String),
    /// A literal that starts with `"` but is not exactly one SPARQL literal.
    Literal(String),
    /// A `VALUES` row that is not one bound term per variable: a row of the wrong length,
    /// or a variable where a term is required.
    Values(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Var(v) => write!(f, "not a SPARQL variable name: {v:?}"),
            RenderError::Iri(i) => write!(f, "IRI has a character SPARQL forbids: {i:?}"),
            RenderError::Literal(l) => write!(f, "not exactly one SPARQL literal: {l:?}"),
            RenderError::Values(v) => write!(f, "not a VALUES row of bound terms: {v}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Render a light `sparq-fedplan` [`Term`] as a SPARQL term string, strictly:
///
/// * a variable becomes `?name` when `name` is a SPARQL `VARNAME`, else [`RenderError::Var`];
/// * an IRI is wrapped in `<>` when it has no character `IRIREF` forbids, else
///   [`RenderError::Iri`];
/// * a literal that starts with `"` is `sparq-fedplan`'s "already rendered" form: it is parsed
///   with the SPARQL parser as exactly one literal (short or long quotes, then an optional
///   language tag or `^^<datatype>`) and re-serialised with canonical escaping, else
///   [`RenderError::Literal`]. Any other literal is
///   a bare lexical value and becomes an escaped plain string.
///
/// So whatever the pattern holds, the rendered text is exactly one SPARQL term.
pub(crate) fn render_term(t: &Term) -> Result<String, RenderError> {
    match t {
        Term::Var(v) if is_varname(&v.0) => Ok(format!("?{}", v.0)),
        Term::Var(v) => Err(RenderError::Var(v.0.clone())),
        Term::Iri(iri) if is_iriref_body(iri) => Ok(format!("<{iri}>")),
        Term::Iri(iri) => Err(RenderError::Iri(iri.clone())),
        Term::Literal(lit) if lit.starts_with('"') => parse_sparql_literal(lit)
            .map(|l| l.to_string())
            .ok_or_else(|| RenderError::Literal(lit.clone())),
        Term::Literal(lit) => Ok(oxrdf::Literal::new_simple_literal(lit.as_str()).to_string()),
    }
}

/// Parse `lit` as exactly one SPARQL literal, in any of SPARQL's literal forms (short or long
/// quotes, a language tag or `^^` datatype), by parsing it with the SPARQL parser in the object
/// position of a one-pattern query and requiring that the whole query is that one pattern with
/// a literal object. Anything else, including extra syntax after the literal, is `None`.
fn parse_sparql_literal(lit: &str) -> Option<oxrdf::Literal> {
    use spargebra::algebra::GraphPattern;
    use spargebra::term::TermPattern;
    let query = spargebra::SparqlParser::new()
        .parse_query(&format!("ASK {{ ?s ?p {lit} }}"))
        .ok()?;
    // ASK wraps its group in a projection; anything but one BGP inside it is extra syntax.
    let spargebra::Query::Ask {
        pattern: GraphPattern::Project { inner, .. },
        ..
    } = query
    else {
        return None;
    };
    let GraphPattern::Bgp { patterns } = *inner else {
        return None;
    };
    match patterns.as_slice() {
        [tp] => match &tp.object {
            TermPattern::Literal(l) => Some(l.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// SPARQL `IRIREF` body: no control or space, and none of `<`, `>`, `"`, `{`, `}`, `|`, `^`,
/// `\`, or the backtick.
fn is_iriref_body(iri: &str) -> bool {
    !iri.chars().any(|c| {
        c <= ' ' || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\')
    })
}

/// SPARQL `VARNAME`: `( PN_CHARS_U | [0-9] ) ( PN_CHARS_U | [0-9] | #xB7 | [#x300-#x36F] |
/// [#x203F-#x2040] )*`.
pub(crate) fn is_varname(name: &str) -> bool {
    fn pn_chars_u_or_digit(c: char) -> bool {
        c == '_'
            || c.is_ascii_alphanumeric()
            || matches!(c,
                '\u{C0}'..='\u{D6}'
                | '\u{D8}'..='\u{F6}'
                | '\u{F8}'..='\u{2FF}'
                | '\u{370}'..='\u{37D}'
                | '\u{37F}'..='\u{1FFF}'
                | '\u{200C}'..='\u{200D}'
                | '\u{2070}'..='\u{218F}'
                | '\u{2C00}'..='\u{2FEF}'
                | '\u{3001}'..='\u{D7FF}'
                | '\u{F900}'..='\u{FDCF}'
                | '\u{FDF0}'..='\u{FFFD}'
                | '\u{10000}'..='\u{EFFFF}')
    }
    let mut chars = name.chars();
    chars.next().is_some_and(pn_chars_u_or_digit)
        && chars.all(|c| {
            pn_chars_u_or_digit(c)
                || matches!(c, '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Endpoint, SourceType, Transport};
    use sparq_fedplan::Var;

    fn iri(s: &str) -> Term {
        Term::Iri(s.to_string())
    }
    fn var(s: &str) -> Term {
        Term::Var(Var::new(s))
    }

    /// A transport double that returns an empty SRJ — used only to build a concrete
    /// `Endpoint` adapter for resolver tests (no network). [OPUS-4.8] sq-j27p.
    struct NullTransport;
    impl Transport for NullTransport {
        fn fetch(&self, _e: &str, _q: &str) -> Result<String, String> {
            Ok(r#"{"head":{"vars":[]},"results":{"bindings":[]}}"#.to_string())
        }
    }

    #[test]
    fn resolver_maps_pattern_and_source_indices() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("s"), iri("http://ex/p"), var("o")),
            TriplePattern::new(var("o"), iri("http://ex/q"), var("z")),
        ]);
        let ep = Endpoint::new("http://8.8.8.8/sparql", Box::new(NullTransport));
        let adapters: Vec<&dyn FederatedSource> = vec![&ep];
        let r = SourceResolver::new(&bgp, &adapters);
        // pattern index → the right TriplePattern.
        assert_eq!(r.pattern(0).unwrap().predicate_iri(), Some("http://ex/p"));
        assert_eq!(r.pattern(1).unwrap().predicate_iri(), Some("http://ex/q"));
        // source index → the adapter.
        assert!(matches!(
            r.source(0).unwrap().source_type(),
            SourceType::Endpoint(_)
        ));
        assert_eq!(r.source_count(), 1);
    }

    #[test]
    fn resolver_range_checks_fail_closed() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        let ep = Endpoint::new("http://8.8.8.8/sparql", Box::new(NullTransport));
        let adapters: Vec<&dyn FederatedSource> = vec![&ep];
        let r = SourceResolver::new(&bgp, &adapters);
        assert_eq!(
            r.pattern(9).unwrap_err(),
            ResolveError::PatternOutOfRange {
                index: 9,
                patterns: 1,
            }
        );
        // `&dyn FederatedSource` is not `Debug`, so match the error out by hand.
        assert_eq!(
            r.source(9).err(),
            Some(ResolveError::SourceOutOfRange {
                index: 9,
                sources: 1,
            })
        );
        assert!(r.source(0).is_ok());
    }

    #[test]
    fn lower_leaf_projects_pattern_vars_in_order() {
        let tp = TriplePattern::new(var("s"), iri("http://ex/p"), var("o"));
        let sub = lower_leaf(&tp).unwrap();
        assert_eq!(sub.project, vec!["s".to_string(), "o".to_string()]);
        assert_eq!(sub.sparql, "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }");
    }

    #[test]
    fn lower_leaf_dedups_repeated_var() {
        // ?s :p ?s — the same var subject + object: projected once.
        let tp = TriplePattern::new(var("s"), iri("http://ex/p"), var("s"));
        let sub = lower_leaf(&tp).unwrap();
        assert_eq!(sub.project, vec!["s".to_string()]);
        assert_eq!(sub.sparql, "SELECT ?s WHERE { ?s <http://ex/p> ?s }");
    }

    #[test]
    fn lower_leaf_fully_bound_is_existence_probe() {
        let tp = TriplePattern::new(iri("http://ex/a"), iri("http://ex/p"), iri("http://ex/b"));
        let sub = lower_leaf(&tp).unwrap();
        assert!(sub.project.is_empty());
        assert_eq!(
            sub.sparql,
            "SELECT * WHERE { <http://ex/a> <http://ex/p> <http://ex/b> } LIMIT 1"
        );
    }

    #[test]
    fn lower_leaf_fragment_maps_positions() {
        use crate::source::{FragTerm, PatternTerm};
        // ?s foaf:knows ?o — predicate bound, subject + object variable.
        let tp = TriplePattern::new(var("s"), iri("http://xmlns.com/foaf/0.1/knows"), var("o"));
        let pat = lower_leaf_fragment(&tp);
        assert_eq!(pat.subject, PatternTerm::Var("s".into()));
        assert_eq!(
            pat.predicate,
            PatternTerm::Bound(FragTerm::Iri("http://xmlns.com/foaf/0.1/knows".into()))
        );
        assert_eq!(pat.object, PatternTerm::Var("o".into()));
        assert_eq!(pat.vars(), vec!["s".to_string(), "o".to_string()]);
        // A bound literal object carries its already-rendered lexical form verbatim.
        let tp2 = TriplePattern::new(
            var("s"),
            iri("http://ex/label"),
            Term::Literal("\"hi\"@en".to_string()),
        );
        assert_eq!(
            lower_leaf_fragment(&tp2).object,
            PatternTerm::Bound(FragTerm::Literal("\"hi\"@en".into()))
        );
    }

    #[test]
    fn lower_leaf_renders_and_escapes_literal_object() {
        // A bare lexical literal is quoted + escaped; a pre-rendered one is verbatim.
        let bare = TriplePattern::new(
            var("s"),
            iri("http://ex/label"),
            Term::Literal("a\"b".to_string()),
        );
        assert_eq!(
            lower_leaf(&bare).unwrap().sparql,
            "SELECT ?s WHERE { ?s <http://ex/label> \"a\\\"b\" }"
        );
        let rendered = TriplePattern::new(
            var("s"),
            iri("http://ex/label"),
            Term::Literal("\"hi\"@en".to_string()),
        );
        assert_eq!(
            lower_leaf(&rendered).unwrap().sparql,
            "SELECT ?s WHERE { ?s <http://ex/label> \"hi\"@en }"
        );
    }

    // [OPUS-4.8] sq-qcnn.22: Additional direct unit tests for coverage ratchet
    #[test]
    fn pattern_vars_all_distinct() {
        // All three positions hold distinct variables.
        let tp = TriplePattern::new(var("s"), var("p"), var("o"));
        assert_eq!(pattern_vars(&tp), vec!["s", "p", "o"]);
    }

    #[test]
    fn pattern_vars_predicate_bound() {
        // Only subject and object are variables; predicate is bound.
        let tp = TriplePattern::new(var("s"), iri("http://ex/type"), var("o"));
        assert_eq!(pattern_vars(&tp), vec!["s", "o"]);
    }

    #[test]
    fn pattern_vars_repeated_variable() {
        // ?s :p ?s — subject and object are the same variable.
        let tp = TriplePattern::new(var("s"), iri("http://ex/relates"), var("s"));
        assert_eq!(pattern_vars(&tp), vec!["s"]);
    }

    #[test]
    fn lower_leaf_all_bound_existence_probe() {
        // Fully bound pattern: all three positions are IRIs → existence probe with LIMIT 1.
        let tp = TriplePattern::new(
            iri("http://ex/alice"),
            iri("http://ex/knows"),
            iri("http://ex/bob"),
        );
        let sub = lower_leaf(&tp).unwrap();
        assert!(
            sub.project.is_empty(),
            "fully bound pattern projects nothing"
        );
        assert_eq!(
            sub.sparql,
            "SELECT * WHERE { <http://ex/alice> <http://ex/knows> <http://ex/bob> } LIMIT 1"
        );
    }

    #[test]
    fn lower_leaf_subject_and_object_vars() {
        // Subject and object are variables; predicate is bound. Projects s, o in that order.
        let tp = TriplePattern::new(var("s"), iri("http://ex/type"), var("o"));
        let sub = lower_leaf(&tp).unwrap();
        assert_eq!(sub.project, vec!["s".to_string(), "o".to_string()]);
        assert_eq!(sub.sparql, "SELECT ?s ?o WHERE { ?s <http://ex/type> ?o }");
    }

    #[test]
    #[should_panic(expected = "literal in subject position is not valid RDF/SPARQL 1.1")]
    fn lower_leaf_literal_in_subject() {
        // A literal in subject position is invalid in RDF/SPARQL 1.1; lower_leaf debug-asserts
        // this never happens. sparq-fedplan must never produce such a pattern — this test
        // documents the rejection path. [OPUS-4.8] sq-qcnn.22
        let _ = lower_leaf(&TriplePattern::new(
            Term::Literal("hello".to_string()),
            iri("http://ex/prop"),
            var("o"),
        ));
    }

    #[test]
    fn lower_leaf_fragment_all_positions_bound() {
        // Fully bound fragment pattern: all bound → all positions map to FragTerm::Iri/Literal.
        use crate::source::{FragTerm, PatternTerm};
        let tp = TriplePattern::new(
            iri("http://ex/alice"),
            iri("http://ex/knows"),
            iri("http://ex/bob"),
        );
        let pat = lower_leaf_fragment(&tp);
        assert_eq!(
            pat.subject,
            PatternTerm::Bound(FragTerm::Iri("http://ex/alice".into()))
        );
        assert_eq!(
            pat.predicate,
            PatternTerm::Bound(FragTerm::Iri("http://ex/knows".into()))
        );
        assert_eq!(
            pat.object,
            PatternTerm::Bound(FragTerm::Iri("http://ex/bob".into()))
        );
        assert_eq!(
            pat.vars(),
            Vec::<String>::new(),
            "fully bound pattern has no variables"
        );
    }

    /// Parse a lowered leaf back with the SPARQL parser and return its one triple pattern.
    fn parse_back(sparql: &str) -> spargebra::term::TriplePattern {
        use spargebra::algebra::GraphPattern;
        let q = spargebra::SparqlParser::new()
            .parse_query(sparql)
            .unwrap_or_else(|e| panic!("{sparql}: {e}"));
        let spargebra::Query::Select { pattern, .. } = q else {
            panic!("not a SELECT: {sparql}")
        };
        let mut p = &pattern;
        loop {
            match p {
                GraphPattern::Project { inner, .. }
                | GraphPattern::Slice { inner, .. }
                | GraphPattern::Distinct { inner } => p = inner,
                GraphPattern::Bgp { patterns } => {
                    assert_eq!(patterns.len(), 1, "{sparql}");
                    return patterns[0].clone();
                }
                other => panic!("unexpected algebra {other:?} for {sparql}"),
            }
        }
    }

    #[test]
    fn strict_render_refuses_iris_with_forbidden_characters() {
        for c in ['<', '>', '"', '{', '}', '|', '^', '`', '\\', ' ', '\n', '\t', '\u{0}', '\u{1F}']
        {
            let bad = format!("http://ex/a{c}b");
            let tp = TriplePattern::new(var("s"), iri(&bad), var("o"));
            assert_eq!(lower_leaf(&tp), Err(RenderError::Iri(bad.clone())), "{c:?}");
            let tp = TriplePattern::new(iri(&bad), iri("http://ex/p"), var("o"));
            assert!(lower_leaf(&tp).is_err(), "{c:?} in subject");
        }
    }

    #[test]
    fn strict_render_refuses_bad_variable_names() {
        for bad in ["", "a b", "o }", "x.y", "-x", "a?b", "a\u{0}"] {
            let tp = TriplePattern::new(var(bad), iri("http://ex/p"), var("o"));
            assert_eq!(lower_leaf(&tp), Err(RenderError::Var(bad.to_string())), "{bad:?}");
        }
        for good in ["s", "_x", "9", "é", "a\u{B7}b", "x\u{300}"] {
            let tp = TriplePattern::new(var(good), iri("http://ex/p"), var("o"));
            let sub = lower_leaf(&tp).unwrap();
            let back = parse_back(&sub.sparql);
            assert_eq!(back.subject.to_string(), format!("?{good}"), "{}", sub.sparql);
        }
    }

    #[test]
    fn strict_render_refuses_malformed_prerendered_literals() {
        for bad in [
            "\"a\" } . ?x ?y ?z",
            "\"a\"@en } UNION { ?x ?y ?z",
            "\"a\"^^<http://ex/d> . ?x ?y ?z",
            "\"a",
            "\"a\"^^<http://ex/d d>",
            "\"a\"@",
            "\"a\" # } UNION { ?x ?y ?z",
            "\"a\" . ?x ?y \"b\"",
            "\"a\" , \"b\"",
            "\"a\" FILTER(true)",
            "\"\"\"a\"\"\" } . { ?x ?y ?z",
            "\"a\"^^xsd:string",
        ] {
            let tp = TriplePattern::new(var("s"), iri("http://ex/p"), Term::Literal(bad.into()));
            assert_eq!(lower_leaf(&tp), Err(RenderError::Literal(bad.to_string())), "{bad:?}");
        }
    }

    #[test]
    fn rendered_terms_parse_back_to_the_same_terms() {
        use oxrdf::{Literal, NamedNode};
        let cases: Vec<(Term, oxrdf::Term)> = vec![
            (iri("http://ex/a%20b?q=1#f"), NamedNode::new("http://ex/a%20b?q=1#f").unwrap().into()),
            (iri("http://ex/é"), NamedNode::new("http://ex/é").unwrap().into()),
            (
                Term::Literal("a\"b\\c\nd\re\tf\u{0}g\u{7F}} . ?x ?y ?z".into()),
                Literal::new_simple_literal("a\"b\\c\nd\re\tf\u{0}g\u{7F}} . ?x ?y ?z").into(),
            ),
            (Term::Literal("\"hi\"@en".into()), Literal::new_language_tagged_literal("hi", "en").unwrap().into()),
            (
                Term::Literal("\"q\\\"} \\\\\"^^<http://ex/d>".into()),
                Literal::new_typed_literal("q\"} \\", NamedNode::new("http://ex/d").unwrap()).into(),
            ),
            (Term::Literal("\"x\"".into()), Literal::new_simple_literal("x").into()),
            // Long-string forms, which the SPARQL grammar accepts in every literal position.
            (
                Term::Literal("\"\"\"line 1\nline \"2\" }\"\"\"".into()),
                Literal::new_simple_literal("line 1\nline \"2\" }").into(),
            ),
            (
                Term::Literal("\"\"\"it's\nhere\"\"\"@en".into()),
                Literal::new_language_tagged_literal("it's\nhere", "en").unwrap().into(),
            ),
            (
                Term::Literal("\"\"\"a\nb\"\"\"^^<http://ex/d>".into()),
                Literal::new_typed_literal("a\nb", NamedNode::new("http://ex/d").unwrap()).into(),
            ),
            // Only a leading `"` marks a pre-rendered literal; anything else is a bare value.
            (Term::Literal("'''x'''".into()), Literal::new_simple_literal("'''x'''").into()),
        ];
        for (term, want) in cases {
            let tp = TriplePattern::new(var("s"), iri("http://ex/p"), term.clone());
            let sub = lower_leaf(&tp).unwrap();
            let back = parse_back(&sub.sparql);
            assert_eq!(back.object.to_string(), want.to_string(), "{term:?} -> {}", sub.sparql);
            assert_eq!(back.subject.to_string(), "?s");
            assert_eq!(back.predicate.to_string(), "<http://ex/p>");
        }
    }
}
