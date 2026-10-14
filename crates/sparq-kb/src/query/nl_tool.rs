//! The PKG **natural-language tool** envelope (agent flavor) — sq-ve5dy.
//!
//! sq-ve5dy (epic sq-2m6zm); reframes PKG access (sq-2m6zm.3) as a
//! *natural-language tool call* the cheap-model sub-agent makes on the expensive
//! orchestrator's behalf. 🤖 SPARQ agent — dogfooding sparq as a project knowledge
//! graph. Written while Fable unavailable; flag for re-review when Fable returns.
//!
//! # Why this module exists
//!
//! The maintainer reframe (2026-06-21) is: a cheap model (Haiku/Sonnet) does the whole
//! round-trip — NL question → introspect/ground → SPARQL → run via `ask_pkg` →
//! result rows → concise NL answer — so the expensive orchestrator only emits the
//! question and reads the answer. The orchestrator never sees the schema card, the
//! SPARQL, or the raw rows.
//!
//! The **AGENT flavor** (this slice, sq-ve5dy) needs no API key: the cheap model is a
//! `model: haiku` Claude-Code sub-agent (definition in
//! `.claude/agents/sparq-pkg-nl.md`) that drives the `pkg-query` binary. The PRODUCT
//! flavor — `sparq-nlq` calling a configurable cheap-model endpoint as a real GenAI
//! integration — needs sparq's own Anthropic key (an external-credential dependency)
//! and is left to a follow-up (see the README + the bead note).
//!
//! # The soundness guardrail (load-bearing)
//!
//! A cheap model translating NL → SPARQL can guess a *plausible-but-wrong* query and
//! return a confident answer the data never supported. So the tool must **never
//! silently answer from a guessed query**: every answer is returned together with the
//! `NlToolResult` envelope — the **executed SPARQL**, the **resolved IRIs** the query
//! actually relied on, the **row count**, and a **grounding confidence** — so the
//! caller can verify the answer was computed, not fabricated. The rows are computed by
//! sparq's own engine over the ingested graph; within the store the model cannot invent
//! a fact the data does not contain, and an empty result is the honest "none" answer.
//!
//! `Confidence` is a **query-grounding** signal (was the query canned / fully
//! dictionary-grounded / partially ungrounded), explicitly *not* an answer-correctness
//! claim — we have no gold answer to score against, and this module makes no such
//! claim.

use serde::{Deserialize, Serialize};
use sparq_core::dict::TermParts;
use sparq_core::strdist::edit_distance;
use sparq_core::Graph;
use sparq_engine::QueryResult;

use oxrdf::{NamedNode, Term};
use spargebra::algebra::{Expression, GraphPattern, PropertyPathExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::Query;

/// `rdf:type` — the predicate whose object is a class IRI (the one object position we
/// treat as schema vocabulary).
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// How many sample answer rows the deterministic NL summary renders before eliding.
const SUMMARY_SAMPLE_ROWS: usize = 5;

/// Grounding confidence of the executed query. A **structural** signal about how
/// grounded the query was — NOT a claim about whether the answer is *correct* (no gold
/// answer is scored here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    /// The query is one of the curated, verified-expressible `canned::` templates
    /// (introspect → ground → ask). Highest structural confidence.
    Canned,
    /// A raw query whose every predicate / `rdf:type` class IRI is present in the live
    /// PKG dictionary, so each vocabulary term it relies on actually exists in the data.
    Grounded,
    /// A raw query that parses but uses one or more predicate / class IRIs **absent**
    /// from the dictionary — it matched no triples for those terms, so the answer may be
    /// partial or empty for the wrong reason. The caller should re-check the
    /// `ungrounded_iris` before trusting the answer.
    Ungrounded,
}

impl Confidence {
    /// A stable one-word label for prompts / logs.
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Canned => "canned",
            Confidence::Grounded => "grounded",
            Confidence::Ungrounded => "ungrounded",
        }
    }
}

/// The verifiable result envelope the NL tool returns — the soundness guardrail. Carries
/// everything the caller needs to confirm the answer was *computed* from the data, not
/// guessed: the executed SPARQL, the IRIs the query relied on (and which of them were
/// ungrounded), the row count, and the grounding confidence. Serialisable to JSON for
/// the `pkg-query --json` output mode the agent-flavor sub-agent consumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NlToolResult {
    /// A concise, **deterministic** NL summary of the answer (header + a few sample
    /// rows, computed by the engine). The cheap-model sub-agent may rephrase this into
    /// fluent prose, but even with no model the envelope already carries a sound answer.
    pub answer: String,
    /// The SPARQL that was executed (post-substitution for a canned query). Always
    /// present — the caller can re-run it to verify.
    pub executed_sparql: String,
    /// Every predicate / `rdf:type` class IRI the query relied on, de-duplicated, in
    /// first-appearance order — the schema vocabulary the answer depends on.
    pub resolved_iris: Vec<String>,
    /// The subset of `resolved_iris` that is **absent from the live dictionary** (so it
    /// matched no triples). Empty for a fully grounded query; non-empty downgrades the
    /// confidence to [`Confidence::Ungrounded`].
    pub ungrounded_iris: Vec<String>,
    /// **Advisory** same-namespace repair hints for each ungrounded IRI (empty when the
    /// query is fully grounded) — so the agent-flavor sub-agent can re-ground its next
    /// query. Advisory only: NOT part of the soundness guarantee.
    pub hints: Vec<String>,
    /// How many solution rows the query returned (0 is the honest "none" answer).
    pub row_count: usize,
    /// The grounding confidence (see [`Confidence`]).
    pub confidence: Confidence,
}

impl NlToolResult {
    /// Serialise the envelope to pretty JSON (the `pkg-query --json` payload). Infallible:
    /// every field is a plain string / number / enum.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("NlToolResult serialises to JSON")
    }
}

/// Run a **canned** PKG query (by name, with an optional `--arg` substitution) as a
/// natural-language tool call, returning the verifiable [`NlToolResult`] envelope.
/// Confidence is [`Confidence::Canned`] (the template is curated + verified-expressible).
///
/// `graph` is the loaded PKG ([`super::load_pkg`] or [`super::close::load_pkg_closed`]).
pub fn run_canned(graph: &Graph, name: &str, arg: Option<&str>) -> Result<NlToolResult, String> {
    let q = super::canned::by_name(name)
        .ok_or_else(|| format!("no canned query named `{name}` (try the canned registry)"))?;
    let sparql = q.render(arg);
    run_sparql_with_confidence(graph, &sparql, Confidence::Canned)
}

/// Run a **raw** SPARQL string as a natural-language tool call, returning the verifiable
/// [`NlToolResult`] envelope. Confidence is derived from the query's grounding against
/// the live dictionary: [`Confidence::Grounded`] when every predicate / class IRI exists
/// in the data, else [`Confidence::Ungrounded`] (with the offending IRIs surfaced in
/// `ungrounded_iris`).
pub fn run_raw(graph: &Graph, sparql: &str) -> Result<NlToolResult, String> {
    // Parse once to drive both grounding and (re-)execution; the engine re-parses but
    // the algebra walk needs the parsed form for exact IRI provenance.
    let parsed = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| format!("the question's SPARQL did not parse: {e}"))?;
    let resolved = resolved_iris(&parsed);
    let ungrounded: Vec<String> = resolved
        .iter()
        .filter(|iri| !iri_in_dictionary(graph, iri))
        .cloned()
        .collect();
    let confidence = if ungrounded.is_empty() {
        Confidence::Grounded
    } else {
        Confidence::Ungrounded
    };
    finish(graph, sparql, resolved, ungrounded, confidence)
}

/// Shared path for a query whose confidence is fixed by the caller (the canned case):
/// still computes the resolved/ungrounded IRIs (provenance), but does not let the
/// grounding override the supplied confidence.
fn run_sparql_with_confidence(
    graph: &Graph,
    sparql: &str,
    confidence: Confidence,
) -> Result<NlToolResult, String> {
    let parsed = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| format!("the canned query did not parse: {e}"))?;
    let resolved = resolved_iris(&parsed);
    let ungrounded: Vec<String> = resolved
        .iter()
        .filter(|iri| !iri_in_dictionary(graph, iri))
        .cloned()
        .collect();
    finish(graph, sparql, resolved, ungrounded, confidence)
}

/// Execute the query and assemble the envelope from the already-computed provenance.
fn finish(
    graph: &Graph,
    sparql: &str,
    resolved_iris: Vec<String>,
    ungrounded_iris: Vec<String>,
    confidence: Confidence,
) -> Result<NlToolResult, String> {
    let result = super::ask_pkg(graph, sparql)?;
    let answer = summarise(&result);
    // Advisory same-namespace hints for any ungrounded term, de-duplicated.
    let mut hints: Vec<String> = Vec::new();
    for iri in &ungrounded_iris {
        for h in nearest_known(graph, iri) {
            if !hints.contains(&h) {
                hints.push(h);
            }
        }
    }
    Ok(NlToolResult {
        answer,
        executed_sparql: sparql.trim().to_string(),
        resolved_iris,
        ungrounded_iris,
        hints,
        row_count: result.len(),
        confidence,
    })
}

/// A concise, deterministic NL summary of a result set: a one-line shape, the variable
/// header, and up to [`SUMMARY_SAMPLE_ROWS`] sample rows with the rest elided. This is
/// the *answer-sized* payload the cheap model rephrases; it is sound on its own.
fn summarise(r: &QueryResult) -> String {
    let n = r.len();
    if n == 0 {
        return "No rows matched — the data holds no answer to this question (the honest \
                \"none\" result)."
            .to_string();
    }
    let header: Vec<&str> = r.vars.iter().map(|v| v.as_str()).collect();
    let mut s = format!(
        "{n} row(s). Columns: {}.\n",
        if header.is_empty() {
            "(none — ASK/unit result)".to_string()
        } else {
            header.join(", ")
        }
    );
    for row in r.rows.iter().take(SUMMARY_SAMPLE_ROWS) {
        let cells: Vec<String> = row.iter().map(cell).collect();
        s.push_str(&format!("- {}\n", cells.join(" | ")));
    }
    if n > SUMMARY_SAMPLE_ROWS {
        s.push_str(&format!("… and {} more row(s).\n", n - SUMMARY_SAMPLE_ROWS));
    }
    s.trim_end().to_string()
}

/// Render one result cell: short local name for an IRI, truncated value for a literal —
/// the same human/agent-readable rendering the `pkg-query` table uses.
fn cell(c: &Option<Term>) -> String {
    match c {
        Some(Term::NamedNode(n)) => n
            .as_str()
            .rsplit(['#', '/'])
            .next()
            .unwrap_or("")
            .to_string(),
        Some(Term::Literal(l)) => {
            let v = l.value();
            if v.chars().count() > 110 {
                let cut: String = v.chars().take(110).collect();
                format!("{cut}…")
            } else {
                v.to_string()
            }
        }
        Some(t) => t.to_string(),
        None => "(unbound)".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Resolved-IRI provenance (algebra walk) + dictionary grounding
// ---------------------------------------------------------------------------

/// Collect every predicate / `rdf:type` class IRI the parsed `query` relies on,
/// de-duplicated in first-appearance order — the schema vocabulary the answer depends
/// on. This is the *resolved IRIs* half of the soundness envelope: the caller can see
/// exactly which terms the answer was computed over.
///
/// Scope mirrors `sparq_nlq::constrain` (predicate + class position): those are the
/// vocabulary terms a grounded query must get right. Specific entity IRIs in
/// subject/object data position are left out (they are values, not schema).
pub fn resolved_iris(query: &Query) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |iri: &str| {
        if seen.insert(iri.to_string()) {
            found.push(iri.to_string());
        }
    };
    match query {
        Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Describe { pattern, .. } => walk_pattern(pattern, &mut push),
        Query::Construct {
            pattern, template, ..
        } => {
            walk_pattern(pattern, &mut push);
            for tp in template {
                walk_triple(tp, &mut push);
            }
        }
    }
    found
}

/// Is `iri` a term in the live dictionary? An absent IRI matches no triple.
fn iri_in_dictionary(graph: &Graph, iri: &str) -> bool {
    match NamedNode::new(iri) {
        Ok(node) => graph.id_of(&Term::NamedNode(node)).is_some(),
        Err(_) => false,
    }
}

// --- algebra walk (predicate + class position) ---------------------------------------

fn walk_pattern<F: FnMut(&str)>(p: &GraphPattern, f: &mut F) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                walk_triple(tp, f);
            }
        }
        GraphPattern::Path { path, .. } => walk_path(path, f),
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right }
        | GraphPattern::Lateral { left, right } => {
            walk_pattern(left, f);
            walk_pattern(right, f);
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            walk_pattern(left, f);
            walk_pattern(right, f);
            if let Some(e) = expression {
                walk_expr(e, f);
            }
        }
        GraphPattern::Filter { expr, inner } => {
            walk_expr(expr, f);
            walk_pattern(inner, f);
        }
        GraphPattern::Graph { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Group { inner, .. } => walk_pattern(inner, f),
        GraphPattern::Extend {
            inner, expression, ..
        } => {
            walk_pattern(inner, f);
            walk_expr(expression, f);
        }
        // A SERVICE block's vocabulary belongs to a remote dictionary; out of scope.
        GraphPattern::Service { .. } => {}
        GraphPattern::Values { .. } => {}
    }
}

fn walk_triple<F: FnMut(&str)>(tp: &TriplePattern, f: &mut F) {
    if let NamedNodePattern::NamedNode(p) = &tp.predicate {
        let p = p.as_str();
        f(p);
        if p == RDF_TYPE {
            if let TermPattern::NamedNode(o) = &tp.object {
                f(o.as_str());
            }
        }
    }
    if let TermPattern::Triple(inner) = &tp.subject {
        walk_triple(inner, f);
    }
    if let TermPattern::Triple(inner) = &tp.object {
        walk_triple(inner, f);
    }
}

fn walk_path<F: FnMut(&str)>(path: &PropertyPathExpression, f: &mut F) {
    match path {
        PropertyPathExpression::NamedNode(n) => f(n.as_str()),
        PropertyPathExpression::Reverse(p)
        | PropertyPathExpression::ZeroOrMore(p)
        | PropertyPathExpression::OneOrMore(p)
        | PropertyPathExpression::ZeroOrOne(p) => walk_path(p, f),
        PropertyPathExpression::Sequence(a, b) | PropertyPathExpression::Alternative(a, b) => {
            walk_path(a, f);
            walk_path(b, f);
        }
        PropertyPathExpression::NegatedPropertySet(ns) => {
            for n in ns {
                f(n.as_str());
            }
        }
    }
}

fn walk_expr<F: FnMut(&str)>(e: &Expression, f: &mut F) {
    match e {
        Expression::Exists(p) => walk_pattern(p, f),
        Expression::Or(a, b)
        | Expression::And(a, b)
        | Expression::Equal(a, b)
        | Expression::SameTerm(a, b)
        | Expression::Greater(a, b)
        | Expression::GreaterOrEqual(a, b)
        | Expression::Less(a, b)
        | Expression::LessOrEqual(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b) => {
            walk_expr(a, f);
            walk_expr(b, f);
        }
        Expression::In(a, list) => {
            walk_expr(a, f);
            for e in list {
                walk_expr(e, f);
            }
        }
        Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
            walk_expr(a, f)
        }
        Expression::FunctionCall(_, args) => {
            for a in args {
                walk_expr(a, f);
            }
        }
        Expression::If(a, b, c) => {
            walk_expr(a, f);
            walk_expr(b, f);
            walk_expr(c, f);
        }
        Expression::Coalesce(list) => {
            for e in list {
                walk_expr(e, f);
            }
        }
        _ => {}
    }
}

/// How many advisory repair hints to surface per ungrounded IRI.
const MAX_HINTS: usize = 4;

/// Same-namespace dictionary IRIs **nearest** (smallest local-name edit distance) to
/// `iri` — an **advisory** repair hint surfaced alongside an ungrounded term so the
/// agent-flavor sub-agent can re-ground its next query without a full re-introspection.
/// Advisory only: it is NOT part of the soundness guarantee (the guarantee is the
/// executed SPARQL + the grounding flag). Ranked by edit distance so the closest
/// candidates lead, capped at `MAX_HINTS`.
pub fn nearest_known(graph: &Graph, iri: &str) -> Vec<String> {
    let Some(split) = iri.rfind(['#', '/']) else {
        return Vec::new();
    };
    let prefix = &iri[..=split];
    let suffix = &iri[split + 1..];
    let mut scored: Vec<(usize, String)> = Vec::new();
    for (_, parts) in graph.dict.iter() {
        if let TermParts::Iri {
            prefix: p,
            suffix: s,
        } = parts
        {
            if p == prefix && s != suffix {
                let full = format!("{p}{s}");
                if !scored.iter().any(|(_, h)| h == &full) {
                    scored.push((edit_distance(suffix, s), full));
                }
            }
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(MAX_HINTS);
    scored.into_iter().map(|(_, iri)| iri).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    /// A tiny PKG-shaped graph so the tests exercise the REAL engine path without
    /// depending on the full ingested instances file (which is large + churny). The
    /// `pkg:` prefix and predicate IRIs match the ontology so grounding is meaningful.
    fn tiny_pkg() -> Graph {
        let ttl = r#"
            @prefix pkg:     <https://sparq.dev/ns/pkg#> .
            @prefix dcterms: <http://purl.org/dc/terms/> .
            @prefix rdfs:    <http://www.w3.org/2000/01/rdf-schema#> .
            pkg:t1 a pkg:Task ; dcterms:identifier "sq-aaaa" ; pkg:status pkg:Open ;
                   dcterms:title "first task" .
            pkg:t2 a pkg:Task ; dcterms:identifier "sq-bbbb" ; pkg:status pkg:Closed ;
                   dcterms:title "second task" .
            pkg:f1 a pkg:Finding ; rdfs:label "a sourced finding" ; pkg:confidence "0.9" .
        "#;
        Graph::load_str(ttl, "turtle").expect("tiny PKG parses")
    }

    #[test]
    fn run_raw_grounded_query_returns_sound_envelope() {
        let g = tiny_pkg();
        let q = "PREFIX pkg: <https://sparq.dev/ns/pkg#>\n\
                 SELECT (COUNT(?t) AS ?n) WHERE { ?t a pkg:Task }";
        let r = run_raw(&g, q).expect("runs");
        assert_eq!(r.confidence, Confidence::Grounded);
        assert!(r.ungrounded_iris.is_empty(), "{:?}", r.ungrounded_iris);
        assert!(
            r.hints.is_empty(),
            "a grounded query needs no hints: {:?}",
            r.hints
        );
        // The resolved IRIs include rdf:type (the predicate) and pkg:Task (the class).
        assert!(r.resolved_iris.contains(&RDF_TYPE.to_string()));
        assert!(r
            .resolved_iris
            .contains(&"https://sparq.dev/ns/pkg#Task".to_string()));
        // The executed SPARQL is echoed back verbatim for verification.
        assert!(r.executed_sparql.contains("COUNT(?t)"));
        assert_eq!(r.row_count, 1);
        assert!(r.answer.contains("row(s)"));
    }

    #[test]
    fn run_raw_ungrounded_predicate_lowers_confidence_and_is_surfaced() {
        let g = tiny_pkg();
        // `pkg:nope` is a perfectly valid IRI absent from this graph's dictionary.
        let q = "PREFIX pkg: <https://sparq.dev/ns/pkg#>\n\
                 SELECT ?s WHERE { ?s pkg:nope ?o }";
        let r = run_raw(&g, q).expect("runs (the query is valid, just ungrounded)");
        assert_eq!(r.confidence, Confidence::Ungrounded);
        assert_eq!(
            r.ungrounded_iris,
            vec!["https://sparq.dev/ns/pkg#nope".to_string()]
        );
        // Advisory same-namespace hints help the agent re-ground: every hint is a real
        // term from the same `pkg:` namespace (and never the ungrounded term itself).
        assert!(
            !r.hints.is_empty(),
            "expected same-namespace hints, got none"
        );
        assert!(
            r.hints
                .iter()
                .all(|h| h.starts_with("https://sparq.dev/ns/pkg#")
                    && h != "https://sparq.dev/ns/pkg#nope"),
            "hints must be same-namespace, non-self terms, got {:?}",
            r.hints
        );
        // The honest "no rows" answer, computed not guessed.
        assert_eq!(r.row_count, 0);
        assert!(r.answer.contains("No rows matched"));
    }

    #[test]
    fn run_canned_uses_canned_confidence_even_when_arg_matches_nothing() {
        let g = tiny_pkg();
        // `task-depends-on` is a curated template; over this tiny graph it returns 0
        // rows, but the confidence stays `Canned` (the template is verified-expressible
        // — the empty answer is honest, not a low-confidence guess).
        let r = run_canned(&g, "task-depends-on", Some("sq-aaaa")).expect("runs");
        assert_eq!(r.confidence, Confidence::Canned);
        assert_eq!(r.row_count, 0);
        // The substituted argument is visible in the echoed SPARQL.
        assert!(r.executed_sparql.contains("sq-aaaa"));
    }

    #[test]
    fn run_canned_unknown_name_is_an_error_not_a_guess() {
        let g = tiny_pkg();
        let err = run_canned(&g, "no-such-query", None).unwrap_err();
        assert!(err.contains("no canned query named"));
    }

    #[test]
    fn syntactically_invalid_sparql_is_an_error_not_a_silent_answer() {
        let g = tiny_pkg();
        let err = run_raw(&g, "SELECT ?s WHERE { ?s ?p").unwrap_err();
        assert!(err.contains("did not parse"), "{err}");
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let g = tiny_pkg();
        let q = "PREFIX pkg: <https://sparq.dev/ns/pkg#>\n\
                 SELECT ?id WHERE { ?t a pkg:Task ; \
                 <http://purl.org/dc/terms/identifier> ?id } ORDER BY ?id";
        let r = run_raw(&g, q).expect("runs");
        let json = r.to_json();
        let back: NlToolResult = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, r);
        // The two task identifiers are in the deterministic sample.
        assert!(r.answer.contains("sq-aaaa"));
        assert!(r.answer.contains("sq-bbbb"));
        assert_eq!(r.row_count, 2);
    }

    #[test]
    fn confidence_labels_are_stable() {
        assert_eq!(Confidence::Canned.label(), "canned");
        assert_eq!(Confidence::Grounded.label(), "grounded");
        assert_eq!(Confidence::Ungrounded.label(), "ungrounded");
    }

    #[test]
    fn nearest_known_ranks_the_closest_sibling_first() {
        let g = tiny_pkg();
        // `pkg:Tas` (typo) — edit distance 1 from `pkg:Task`, which must therefore LEAD
        // the (distance-ranked) same-namespace hint list, ahead of farther terms.
        let hints = nearest_known(&g, "https://sparq.dev/ns/pkg#Tas");
        assert_eq!(
            hints.first().map(String::as_str),
            Some("https://sparq.dev/ns/pkg#Task"),
            "the distance-1 sibling pkg:Task must rank first, got {hints:?}"
        );
        assert!(
            hints.len() <= MAX_HINTS,
            "capped at MAX_HINTS, got {hints:?}"
        );
    }
}
