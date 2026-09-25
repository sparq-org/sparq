//! The `text:` magic predicates: full-text search inside plain SPARQL,
//! with ZERO engine changes.
//!
//! ```
//! use sparq_core::Graph;
//! use sparq_text::TextIndex;
//!
//! let g = Graph::load_str(r#"
//!     <http://ex/a> <http://ex/comment> "The quick brown fox" .
//!     <http://ex/b> <http://ex/comment> "A lazy dog" .
//! "#, "ntriples").unwrap();
//! let index = TextIndex::build(&g);
//! let r = sparq_text::query_text(&g,
//!     "PREFIX text: <http://sparq.dev/text#>
//!      SELECT ?s WHERE { ?s <http://ex/comment> ?lit . ?lit text:matches \"quick fox\" }",
//!     &index).unwrap();
//! assert_eq!(r.len(), 1); // only <http://ex/a>
//! ```
//!
//! ## The rewrite
//!
//! [`rewrite_query`] walks the parsed spargebra algebra; in every basic graph
//! pattern, each magic triple pattern
//!
//! - `?lit text:matches "query"` — AND of tokens, `*`-suffix = prefix token;
//! - `?lit text:matchesAny "query"` — OR of tokens;
//! - `?lit text:fuzzy "term"` — with the `fuzzy` feature, a typo-tolerant
//!   single-token match at edit distance one by default; a same-subject
//!   `text:maxDistance N` companion selects `0..=2`. [GPT-5.6]
//! - `?lit text:phrase "foo bar"` — tokens ADJACENT and IN ORDER (a positional
//!   phrase match; needs a positions-enabled index — see below). [OPUS-4.8]
//! - `?lit text:near "foo bar"` — tokens IN ORDER within a bounded gap (the
//!   proximity/slop generalisation of `text:phrase`), RELEVANCE-RANKED; the gap
//!   budget defaults to [`DEFAULT_SLOP`] and is set by a `?lit text:slop N`
//!   companion. Also needs a positions-enabled index. [OPUS-4.8]
//! - `?lit text:score ?s` — companion to exactly one scored match on the same
//!   `?lit` in the same BGP — `text:matches`/`text:matchesAny` (the BM25 score)
//!   or `text:near` (the proximity score `1/(1+gap)`) — binds it as an
//!   `xsd:double`. NOT valid for `text:phrase` (boolean adjacency, unscored).
//!   [OPUS-4.8]
//!
//! is REMOVED and replaced by an inline [`Values`](GraphPattern::Values)
//! table of the index's hits — the matching literal terms (and scores),
//! resolved through the graph's dictionary. The surrounding query then joins
//! those literals to triples through the store's ordinary permutation
//! indexes; the rewritten algebra runs through sparq-engine's prepared-query
//! seam ([`PreparedQuery::with_query`], retaining VERSION metadata), so the engine —
//! planner, executor, wasm bundle — is completely unaware of text search.
//!
//! Result ordering: `VALUES` rows carry no order through joins — sort with
//! `ORDER BY DESC(?s)` over a `text:score` variable.
//!
//! Constraints (each a hard query error, not a silent mismatch): the
//! subject must be a variable; the query string must be a constant literal
//! (bind-time rewriting has no per-row values); `text:score` needs exactly
//! one match pattern for its variable; `text:phrase` requires the index to
//! carry positions ([`TextIndex::build_with_positions`]) and rejects a
//! `text:score` companion; `text:near` likewise requires positions, takes an
//! optional `text:slop N` (a non-negative integer; at most one per `?lit`,
//! valid only beside a `text:near`); any other IRI in the `text:` namespace is
//! unknown. The index is per-graph (dictionary-local ids), so hits come from
//! the index you pass — typically the default graph's.

use crate::index::TextIndex;
use crate::vocab;
use oxrdf::{Literal, Term};
use spargebra::algebra::GraphPattern;
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;
use sparq_core::dict::Id;
use sparq_core::Graph;
use sparq_engine::{PreparedQuery, QueryBudget, QueryResult};

/// Executes a SPARQL query that may use the `text:` magic predicates: parse,
/// [`rewrite_query`], then evaluate with the standard engine.
pub fn query_text(graph: &Graph, sparql: &str, index: &TextIndex) -> Result<QueryResult, String> {
    sparq_engine::query_prepared(graph, &prepare_text(graph, sparql, index)?)
}

/// [`query_text`] under a cooperative [`QueryBudget`].
pub fn query_text_with_budget(
    graph: &Graph,
    sparql: &str,
    index: &TextIndex,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    sparq_engine::query_prepared_with_budget(graph, &prepare_text(graph, sparql, index)?, budget)
}

/// Parses and rewrites into a [`PreparedQuery`] — compose with any of the
/// engine's `*_prepared` entry points (`ask_prepared`, `construct_prepared`,
/// …). Note the hits are frozen at rewrite time: re-prepare after the graph
/// (and index) change.
pub fn prepare_text(
    graph: &Graph,
    sparql: &str,
    index: &TextIndex,
) -> Result<PreparedQuery, String> {
    let (query, versions) = spargebra::SparqlParser::new()
        .parse_query_with_versions(sparql)
        .map_err(|e| e.to_string())?;
    let prepared = PreparedQuery::from_query_with_versions(query, versions)?;
    Ok(prepared.with_query(rewrite_query(prepared.query().clone(), graph, index)?))
}

/// Rewrites every `text:` magic pattern in the query into inline `VALUES`
/// over the index's hits (see the module docs). A query without `text:`
/// patterns passes through unchanged.
pub fn rewrite_query(mut query: Query, graph: &Graph, index: &TextIndex) -> Result<Query, String> {
    let pattern = match &mut query {
        Query::Select { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. } => pattern,
    };
    rewrite_pattern(pattern, graph, index)?;
    Ok(query)
}

/// Recursively rewrites the magic patterns inside `p`.
fn rewrite_pattern(p: &mut GraphPattern, graph: &Graph, index: &TextIndex) -> Result<(), String> {
    match p {
        GraphPattern::Bgp { patterns } => {
            let patterns = std::mem::take(patterns);
            *p = rewrite_bgp(patterns, graph, index)?;
        }
        GraphPattern::Join { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right }
        | GraphPattern::Lateral { left, right } => {
            rewrite_pattern(left, graph, index)?;
            rewrite_pattern(right, graph, index)?;
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Graph { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Service { inner, .. } => rewrite_pattern(inner, graph, index)?,
        // No BGPs inside: property paths and inline VALUES pass through.
        GraphPattern::Path { .. } | GraphPattern::Values { .. } => {}
    }
    Ok(())
}

/// Default proximity gap budget for `text:near` when no `text:slop` companion
/// is given. Zero means `text:near "q"` defaults to exact adjacency — the same
/// hits as `text:phrase "q"` — and a `text:slop N` companion only ever widens
/// it; an explicit budget is the common case. [OPUS-4.8]
pub const DEFAULT_SLOP: u32 = 0;

#[cfg(feature = "fuzzy")]
fn is_match_predicate(iri: &str) -> bool {
    matches!(
        iri,
        vocab::MATCHES | vocab::MATCHES_ANY | vocab::PHRASE | vocab::NEAR | vocab::FUZZY
    )
}

#[cfg(not(feature = "fuzzy"))]
fn is_match_predicate(iri: &str) -> bool {
    matches!(
        iri,
        vocab::MATCHES | vocab::MATCHES_ANY | vocab::PHRASE | vocab::NEAR
    )
}

/// Which index query a `text:` match request runs.
enum ReqKind {
    /// `text:matches` — AND of tokens (BM25-scored).
    All,
    /// `text:matchesAny` — OR of tokens (BM25-scored).
    Any,
    /// `text:fuzzy` — bounded single-token Levenshtein lookup. [GPT-5.6]
    #[cfg(feature = "fuzzy")]
    Fuzzy { max_distance: Option<u8> },
    /// `text:phrase` — adjacent, in-order tokens (positional, unscored). [OPUS-4.8]
    Phrase,
    /// `text:near` — in-order tokens within a bounded total gap, proximity-scored
    /// (`1/(1+gap)`); `slop` is `Some` once a `text:slop` companion has set it,
    /// `None` (→ [`DEFAULT_SLOP`] at execution) otherwise. The `Option` makes a
    /// duplicate `text:slop` detectable for ANY value (including 0). [OPUS-4.8]
    Near { slop: Option<u32> },
}

/// One `text:matches`/`text:matchesAny`/`text:phrase`/`text:near` request found
/// in a BGP. [OPUS-4.8]
struct MatchReq {
    var: Variable,
    query: String,
    kind: ReqKind,
    score: Option<Variable>,
}

/// Splits a BGP's magic patterns out, runs the index, and joins the `VALUES`
/// hit tables onto the remaining ordinary patterns.
fn rewrite_bgp(
    patterns: Vec<TriplePattern>,
    graph: &Graph,
    index: &TextIndex,
) -> Result<GraphPattern, String> {
    let mut rest: Vec<TriplePattern> = Vec::with_capacity(patterns.len());
    let mut reqs: Vec<MatchReq> = Vec::new();
    let mut scores: Vec<(Variable, Variable)> = Vec::new();
    // (subject var, parsed slop) collected from `text:slop` patterns, attached
    // to the matching `text:near` after the scan (order-independent). [OPUS-4.8]
    let mut slops: Vec<(Variable, u32)> = Vec::new();
    #[cfg(feature = "fuzzy")]
    let mut max_distances: Vec<(Variable, u8)> = Vec::new();

    for tp in patterns {
        let NamedNodePattern::NamedNode(pred) = &tp.predicate else {
            rest.push(tp);
            continue;
        };
        let iri = pred.as_str();
        if !iri.starts_with(vocab::TEXT_NS) {
            rest.push(tp);
            continue;
        }
        let subject_var = |s: &TermPattern| -> Result<Variable, String> {
            match s {
                TermPattern::Variable(v) => Ok(v.clone()),
                other => Err(format!(
                    "text: the subject of <{iri}> must be a variable, got {other}"
                )),
            }
        };
        match iri {
            iri if is_match_predicate(iri) => {
                let var = subject_var(&tp.subject)?;
                let TermPattern::Literal(q) = &tp.object else {
                    return Err(format!(
                        "text: the object of <{iri}> must be a constant query-string literal, got {}",
                        tp.object
                    ));
                };
                let kind = match iri {
                    vocab::MATCHES_ANY => ReqKind::Any,
                    vocab::PHRASE => ReqKind::Phrase,
                    vocab::NEAR => ReqKind::Near { slop: None },
                    #[cfg(feature = "fuzzy")]
                    vocab::FUZZY => ReqKind::Fuzzy { max_distance: None },
                    _ => ReqKind::All,
                };
                reqs.push(MatchReq {
                    var,
                    query: q.value().to_string(),
                    kind,
                    score: None,
                });
            }
            vocab::SLOP => {
                let var = subject_var(&tp.subject)?;
                let TermPattern::Literal(n) = &tp.object else {
                    return Err(format!(
                        "text: the object of text:slop must be a constant non-negative integer \
                         literal, got {}",
                        tp.object
                    ));
                };
                // Accept any literal whose lexical form is a non-negative integer
                // (xsd:integer and friends print bare digits); reject anything else.
                let slop: u32 = n.value().parse().map_err(|_| {
                    format!(
                        "text: text:slop must be a non-negative integer, got \"{}\"",
                        n.value()
                    )
                })?;
                slops.push((var, slop));
            }
            #[cfg(feature = "fuzzy")]
            vocab::MAX_DISTANCE => {
                let var = subject_var(&tp.subject)?;
                let TermPattern::Literal(n) = &tp.object else {
                    return Err(format!(
                        "text: the object of text:maxDistance must be a constant integer in \
                         0..={}, got {}",
                        crate::fuzzy::MAX_DISTANCE,
                        tp.object
                    ));
                };
                let distance: u8 = n.value().parse().map_err(|_| {
                    format!(
                        "text: text:maxDistance must be an integer in 0..={}, got \"{}\"",
                        crate::fuzzy::MAX_DISTANCE,
                        n.value()
                    )
                })?;
                if distance > crate::fuzzy::MAX_DISTANCE {
                    return Err(format!(
                        "text: text:maxDistance must be at most {}, got {distance}",
                        crate::fuzzy::MAX_DISTANCE
                    ));
                }
                max_distances.push((var, distance));
            }
            vocab::SCORE => {
                let var = subject_var(&tp.subject)?;
                let TermPattern::Variable(score_var) = &tp.object else {
                    return Err(format!(
                        "text: the object of text:score must be a variable, got {}",
                        tp.object
                    ));
                };
                scores.push((var, score_var.clone()));
            }
            _ => {
                return Err(format!(
                    "text: unknown magic predicate <{iri}> (supported: text:matches, \
                     text:matchesAny, text:phrase, text:near, text:slop, text:score{})",
                    if cfg!(feature = "fuzzy") {
                        ", text:fuzzy, text:maxDistance"
                    } else {
                        ""
                    }
                ))
            }
        }
    }

    #[cfg(feature = "fuzzy")]
    for (var, value) in max_distances {
        let mut owners = reqs.iter_mut().filter(|r| r.var == var);
        match (owners.next(), owners.next()) {
            (Some(req), None) => match &mut req.kind {
                ReqKind::Fuzzy { max_distance } => {
                    if max_distance.replace(value).is_some() {
                        return Err(format!("text: duplicate text:maxDistance for {var}"));
                    }
                }
                _ => {
                    return Err(format!(
                        "text: text:maxDistance on {var} is only valid alongside text:fuzzy"
                    ))
                }
            },
            (None, _) => {
                return Err(format!(
                    "text: text:maxDistance on {var} has no text:fuzzy in the same basic graph pattern"
                ))
            }
            (Some(_), Some(_)) => {
                return Err(format!(
                    "text: text:maxDistance on {var} is ambiguous ({var} has more than one text: match pattern in this basic graph pattern)"
                ))
            }
        }
    }

    // Attach each text:slop to the single text:near request on its variable
    // (before scores: scoring is uniform across the resolved kind). [OPUS-4.8]
    for (var, value) in slops {
        let mut owners = reqs.iter_mut().filter(|r| r.var == var);
        match (owners.next(), owners.next()) {
            (Some(req), None) => match &mut req.kind {
                ReqKind::Near { slop } => {
                    // A second text:slop on the same near is ambiguous (caught
                    // for any value, including 0, via the Option).
                    if slop.replace(value).is_some() {
                        return Err(format!("text: duplicate text:slop for {var}"));
                    }
                }
                _ => {
                    return Err(format!(
                        "text: text:slop on {var} is only valid alongside text:near (got a \
                         different text: match for {var})"
                    ))
                }
            },
            (None, _) => {
                return Err(format!(
                    "text: text:slop on {var} has no text:near in the same basic graph pattern"
                ))
            }
            (Some(_), Some(_)) => {
                return Err(format!(
                    "text: text:slop on {var} is ambiguous ({var} has more than one text: match \
                     pattern in this basic graph pattern)"
                ))
            }
        }
    }

    // Attach each text:score to the single match request on its variable.
    for (var, score_var) in scores {
        let mut owners = reqs.iter_mut().filter(|r| r.var == var);
        match (owners.next(), owners.next()) {
            (Some(req), None) => {
                if matches!(req.kind, ReqKind::Phrase) {
                    return Err(format!(
                        "text: text:score on {var} is not valid for text:phrase (a phrase \
                         match is boolean adjacency, not a ranking; use text:near for a scored \
                         proximity variant)"
                    ));
                }
                if req.score.replace(score_var).is_some() {
                    return Err(format!("text: duplicate text:score for {var}"));
                }
            }
            (None, _) => {
                return Err(format!(
                    "text: text:score on {var} has no text:matches/text:matchesAny/text:near in \
                     the same basic graph pattern"
                ))
            }
            (Some(_), Some(_)) => {
                return Err(format!(
                    "text: text:score on {var} is ambiguous ({var} has more than one match \
                     pattern in this basic graph pattern)"
                ))
            }
        }
    }

    // Join the hit tables onto the remaining ordinary patterns.
    let mut out = GraphPattern::Bgp { patterns: rest };
    for req in reqs {
        // Each hit is a matching literal id; the scored modes (BM25 and the
        // proximity `text:near`) also carry a score, the positional phrase mode
        // is unscored (id only). [OPUS-4.8]
        let hits: Vec<(Id, Option<f32>)> = match req.kind {
            ReqKind::All => index
                .search(&req.query)
                .into_iter()
                .map(|h| (h.id, Some(h.score)))
                .collect(),
            ReqKind::Any => index
                .search_any(&req.query)
                .into_iter()
                .map(|h| (h.id, Some(h.score)))
                .collect(),
            #[cfg(feature = "fuzzy")]
            ReqKind::Fuzzy { max_distance } => index
                .fuzzy(
                    &req.query,
                    max_distance.unwrap_or(crate::fuzzy::DEFAULT_MAX_DISTANCE),
                )
                .map_err(|error| format!("text: {error}"))?
                .into_iter()
                .map(|hit| (hit.id, Some(hit.score)))
                .collect(),
            ReqKind::Phrase => {
                if !index.has_positions() {
                    return Err(
                        "text: text:phrase requires a positions-enabled index; build it with \
                         TextIndex::build_with_positions (the default index stores no positions)"
                            .to_string(),
                    );
                }
                index
                    .phrase(&req.query)
                    .into_iter()
                    .map(|id| (id, None))
                    .collect()
            }
            ReqKind::Near { slop } => {
                if !index.has_positions() {
                    return Err(
                        "text: text:near requires a positions-enabled index; build it with \
                         TextIndex::build_with_positions (the default index stores no positions)"
                            .to_string(),
                    );
                }
                let slop = slop.unwrap_or(DEFAULT_SLOP);
                index
                    .phrase_near(&req.query, slop)
                    .into_iter()
                    .map(|h| (h.id, Some(h.score)))
                    .collect()
            }
        };
        let mut variables = vec![req.var];
        if let Some(s) = &req.score {
            variables.push(s.clone());
        }
        let bindings = hits
            .iter()
            .map(|&(id, score)| {
                let Term::Literal(lit) = graph.dict.term(id) else {
                    unreachable!("text index documents are literals");
                };
                let mut row = vec![Some(GroundTerm::Literal(lit))];
                if req.score.is_some() {
                    let score = score.expect("text:score only attaches to scored match requests");
                    row.push(Some(GroundTerm::Literal(Literal::from(f64::from(score)))));
                }
                row
            })
            .collect();
        let values = GraphPattern::Values {
            variables,
            bindings,
        };
        out = match out {
            // An all-magic BGP leaves an empty Bgp behind: drop the unit table.
            GraphPattern::Bgp { ref patterns } if patterns.is_empty() => values,
            other => GraphPattern::Join {
                left: Box::new(values),
                right: Box::new(other),
            },
        };
    }
    Ok(out)
}
