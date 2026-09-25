//! Query rewriting for the two session-query paths (design doc §4.4 / §5):
//!
//! 1. **GRAPH wrapping** ([`wrap_for_view`], used by BOTH paths): every default-graph
//!    triple/path pattern is wrapped in `GRAPH ?__sgN { … }` (fresh variable per
//!    pattern, joined above — cross-document joins keep working; this is the standard
//!    union-default-graph emulation, modulo duplicate rows when the same triple is
//!    asserted in several accessible graphs);
//! 2. **dataset-clause injection** ([`rewrite_for`] = step 1 + this, the v1/portability
//!    path): the dataset clause is replaced by `FROM NAMED <g>` for exactly the
//!    authorized graphs (intersected with any pre-existing FROM NAMED), so `GRAPH`
//!    patterns range over the authorized set only — enforced by the engine's
//!    `build_active` semantics (the store's own graphs do not leak in; absent graphs
//!    are empty).
//!
//! The honest cost of step 2: `build_active` decodes + rebuilds every listed graph PER
//! QUERY. The default `DatasetView` path (engine L1, design doc §5) deletes that copy:
//! it needs only step 1, because graph visibility is enforced by the view itself
//! (O(1) hash check, zero copy) — see [`crate::PodStore::query_as`].

use oxrdf::{NamedNode, Variable};
use spargebra::algebra::{Expression, GraphPattern, QueryDataset};
use spargebra::term::NamedNodePattern;
use spargebra::{Query, SparqlParser};

/// Rewrite `sparql` so it can only see `allowed` named graphs. Returns the rewritten
/// query string (parse → transform → serialize round-trip).
///
/// Two transformations (see the module docs for the why):
///
/// 1. default-graph triple/path patterns get wrapped in `GRAPH ?fresh { … }`
///    (per-pattern fresh variables — union-default emulation, cross-document joins
///    keep working);
/// 2. the dataset clause becomes `FROM NAMED <g>` for exactly the `allowed` graphs,
///    intersected with any `FROM NAMED` the query already carried (a query can
///    restrict further, never widen). `FROM` (default-graph) clauses are dropped —
///    pod data never lives in the store default graph.
///
/// Fail-closed invariant: an empty `allowed` set yields a query over a single
/// reserved **sentinel** graph (`<urn:sparq:nothing>`, guaranteed absent — the loader
/// strips reserved graphs), because an empty `FROM NAMED` list would not survive the
/// serialize→reparse round-trip and the store's graphs would leak back in.
///
/// Callers normally go through [`crate::PodStore::query_as`], which feeds this the
/// session's authorized graph set.
///
/// # Errors
///
/// Returns `Err` if `sparql` is not a valid SPARQL query (SELECT / ASK / CONSTRUCT /
/// DESCRIBE), or its VERSION announcements are unsupported or incompatible.
///
/// # Examples
///
/// ```
/// use oxrdf::NamedNode;
/// use sparq_solid::rewrite_for;
///
/// let allowed = [NamedNode::new("https://pod.ex/notes/n1").unwrap()];
/// let q = rewrite_for("SELECT ?t WHERE { ?s <https://ex.dev/ns#title> ?t }", &allowed)?;
/// assert!(q.contains("FROM NAMED <https://pod.ex/notes/n1>"));
/// assert!(q.contains("GRAPH ?__sg0")); // the pattern now ranges over authorized graphs
///
/// // empty set -> the absent sentinel graph keeps the dataset clause fail-closed
/// let none = rewrite_for("SELECT ?t WHERE { ?s ?p ?t }", &[])?;
/// assert!(none.contains("FROM NAMED <urn:sparq:nothing>"));
/// # Ok::<(), String>(())
/// ```
pub fn rewrite_for(sparql: &str, allowed: &[NamedNode]) -> Result<String, String> {
    let (mut q, versions) = SparqlParser::new().parse_query_with_versions(sparql).map_err(|e| e.to_string())?;
    let dataset = match &mut q {
        Query::Select { dataset, .. }
        | Query::Construct { dataset, .. }
        | Query::Describe { dataset, .. }
        | Query::Ask { dataset, .. } => dataset,
    };
    // restrict the dataset: FROM NAMED = allowed (∩ pre-existing), FROM = nothing
    let mut named: Vec<NamedNode> = match dataset.as_ref().and_then(|d| d.named.clone()) {
        Some(existing) => existing.into_iter().filter(|g| allowed.contains(g)).collect(),
        None => allowed.to_vec(),
    };
    if named.is_empty() {
        // an EMPTY FROM NAMED list would vanish in the serialize→reparse round-trip and
        // the store's own graphs would leak back in; a sentinel absent graph keeps the
        // dataset clause present (build_active: absent graph = empty graph)
        named.push(NamedNode::new_unchecked("urn:sparq:nothing"));
    }
    *dataset = Some(QueryDataset { default: Vec::new(), named: Some(named) });
    wrap_query(&mut q, sparql);
    serialize_with_versions(q, versions)
}

/// Rewrite step 1 ONLY — wrap every default-graph triple/path pattern in
/// `GRAPH ?fresh { … }` (union-default emulation), leaving any dataset clause alone.
///
/// This is the rewrite the **default** `DatasetView` query path needs
/// ([`crate::PodStore::query_as`]): under the view, graph visibility is already
/// enforced by the engine (`GRAPH ?g` enumerates only visible graphs, a dataset
/// clause is intersected with the view), and the view's default graph is empty —
/// so the only transformation left is making default-graph patterns range over the
/// (visible) named graphs.
///
/// # Errors
///
/// Returns `Err` for invalid syntax or unsupported/incompatible VERSION announcements.
///
/// # Examples
///
/// ```
/// let q = sparq_solid::wrap_for_view("SELECT ?t WHERE { ?s <https://ex.dev/ns#title> ?t }")?;
/// assert!(q.contains("GRAPH ?__sg0")); // pattern ranges over (visible) named graphs
/// assert!(!q.contains("FROM"));        // no dataset clause: the view enforces visibility
/// # Ok::<(), String>(())
/// ```
pub fn wrap_for_view(sparql: &str) -> Result<String, String> {
    let (mut q, versions) = SparqlParser::new().parse_query_with_versions(sparql).map_err(|e| e.to_string())?;
    wrap_query(&mut q, sparql);
    serialize_with_versions(q, versions)
}

/// [OPUS-4.8] sq-gq28y (issue #1546). The spec-minted reserved IRI a client uses to opt a
/// query into **union default graph mode**, per the *Access-Controlled SPARQL Query over a
/// Solid Pod* Editor's Draft §"Union default graph mode" (`jeswr/solid-sparql-query`). When
/// it appears among a query's default-graph IRIs — a `FROM` clause here (the
/// `default-graph-uri` protocol parameter is the equivalent signal at the HTTP layer, handled
/// by the Solid server, not this string API) — the default graph FOR THAT QUERY becomes the
/// RDF merge of the session's authorized named graphs. In `FROM NAMED` position it "names
/// nothing" (treated as absent) and never binds `GRAPH ?g`.
///
/// This is DEFAULT public surface: the spec-conformant empty-default + explicit-union read
/// path ([`wrap_for_view_opt_in`]) is always on. The opt-in is the ONLY way a bare
/// default-graph pattern sees anything; without it the standing default graph is empty.
pub const UNION_DEFAULT_GRAPH_IRI: &str = "http://www.w3.org/ns/solid/sparql#union-default-graph";

/// Detect the union-default-graph opt-in and STRIP the reserved IRI from the query's
/// dataset clause. Returns `true` iff the reserved IRI was present in a **default-graph**
/// (`FROM`) position (the opt-in signal per the Editor's Draft §"Union default graph mode").
///
/// The reserved IRI is a *signal*, never a real graph name, so it is removed from BOTH the
/// default and named dataset positions before evaluation:
/// - in `FROM NAMED` position it must be treated as **absent** (draft: "names nothing …
///   `GRAPH ?g` never binds to it") — leaving it would intersect the view's authorized
///   named-graph set down to the empty set and wrongly collapse `GRAPH ?g` to zero solutions;
/// - when stripping empties the whole dataset clause (e.g. the query carried ONLY
///   `FROM <reserved>`), the clause is dropped so the query round-trips as "no dataset
///   clause" and the view applies the FULL authorized named-graph set (draft: "when only
///   the reserved IRI is given, the named-graph set remains the authorized set, so `GRAPH`
///   patterns stay usable alongside the union default graph").
///
/// Matching is EXACT (a near-miss IRI is a normal, absent, per-pod-model dataset reference
/// that contributes nothing and never widens): an unrecognised value therefore fails
/// **closed** — it does NOT silently enable the union default graph.
fn take_union_default_opt_in(q: &mut Query) -> bool {
    let dataset = match q {
        Query::Select { dataset, .. }
        | Query::Construct { dataset, .. }
        | Query::Describe { dataset, .. }
        | Query::Ask { dataset, .. } => dataset,
    };
    let Some(d) = dataset.as_mut() else { return false };
    let reserved = NamedNode::new_unchecked(UNION_DEFAULT_GRAPH_IRI);
    let opt_in = d.default.contains(&reserved);
    d.default.retain(|g| *g != reserved);
    if let Some(named) = d.named.as_mut() {
        named.retain(|g| *g != reserved);
    }
    let empty = d.default.is_empty() && d.named.as_ref().is_none_or(|n| n.is_empty());
    if empty {
        *dataset = None;
    }
    opt_in
}

/// [OPUS-4.8] sq-gq28y (issue #1546). The **spec-conformant default** read-path rewrite: the
/// *Access-Controlled SPARQL Query over a Solid Pod* Editor's Draft empty-default +
/// explicit-union semantics, layered on top of [`wrap_for_view`]. This is the rewrite
/// [`crate::PodStore::query_as`] uses by default (the `legacy-union-default-graph` feature
/// swaps it back to [`wrap_for_view`] for a union-always escape hatch).
///
/// Detects + strips the reserved [`UNION_DEFAULT_GRAPH_IRI`] from the dataset clause, then:
/// - **opt-in present** (reserved IRI in a `FROM` clause) → apply the per-pattern GRAPH
///   wrap ([`wrap_for_view`]'s transformation) so bare default-graph patterns range over
///   the session's authorized named graphs — the union default graph, emulated at the
///   query layer (the engine's view keeps its `Empty` default graph; no `UnionOfVisible`
///   engine mode is needed — design record §5.4(a));
/// - **opt-in absent** → leave default-graph patterns UNWRAPPED, so they evaluate against
///   the view's **empty** default graph and yield zero solutions (draft: "The standing
///   default graph of the queried dataset MUST be empty").
///
/// Per-request only: the choice is a pure function of THIS query's dataset clause, so it
/// cannot leak across requests (the store/session carry no union-default state).
///
/// # Errors
///
/// Returns `Err` for invalid syntax or unsupported/incompatible VERSION announcements.
///
/// # Examples
///
/// ```
/// use sparq_solid::{wrap_for_view_opt_in, UNION_DEFAULT_GRAPH_IRI};
///
/// // No opt-in: the bare default-graph pattern is left unwrapped (empty default graph).
/// let plain = wrap_for_view_opt_in("SELECT ?t WHERE { ?s <urn:p> ?t }").unwrap();
/// assert!(!plain.contains("GRAPH"));
///
/// // Opt-in via `FROM <reserved>`: the pattern is wrapped to range over named graphs, and
/// // the reserved IRI is stripped (it never reaches the engine as a graph name).
/// let opted = wrap_for_view_opt_in(
///     &format!("SELECT ?t FROM <{}> WHERE {{ ?s <urn:p> ?t }}", UNION_DEFAULT_GRAPH_IRI),
/// )
/// .unwrap();
/// assert!(opted.contains("GRAPH"));
/// assert!(!opted.contains(UNION_DEFAULT_GRAPH_IRI));
/// ```
pub fn wrap_for_view_opt_in(sparql: &str) -> Result<String, String> {
    let (mut q, versions) = SparqlParser::new().parse_query_with_versions(sparql).map_err(|e| e.to_string())?;
    if take_union_default_opt_in(&mut q) {
        wrap_query(&mut q, sparql);
    }
    serialize_with_versions(q, versions)
}

// [GPT-6] Query Display omits VERSION metadata. Validate labels before emitting
// the retained announcements, so unknown text cannot enter the serialized prologue.
fn serialize_with_versions(query: Query, versions: Vec<String>) -> Result<String, String> {
    let prepared = sparq_engine::PreparedQuery::from_query_with_versions(query, versions)?;
    let announcements = prepared.versions().iter()
        .map(|label| format!("VERSION \"{label}\"\n")).collect::<String>();
    Ok(format!("{announcements}{}", prepared.query()))
}

/// Apply the per-pattern GRAPH wrap to a parsed query, with a graph-variable prefix
/// that cannot collide with any user variable: lengthen until it appears nowhere in
/// the original query text.
fn wrap_query(q: &mut Query, original: &str) {
    let pattern = match q {
        Query::Select { pattern, .. }
        | Query::Construct { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. } => pattern,
    };
    let mut prefix = "__sg".to_owned();
    while original.contains(&prefix) {
        prefix.push('x');
    }
    let mut fresh = Fresh { prefix, n: 0 };
    wrap_in_graph(pattern, &mut fresh, false);
}

struct Fresh {
    prefix: String,
    n: usize,
}

fn fresh_graph_var(fresh: &mut Fresh) -> NamedNodePattern {
    let v = Variable::new_unchecked(format!("{}{}", fresh.prefix, fresh.n));
    fresh.n += 1;
    NamedNodePattern::Variable(v)
}

/// Recursively wrap default-graph-scoped triple/path patterns in `GRAPH ?fresh { … }`.
/// `in_graph` = already inside a GRAPH scope (leave those patterns alone).
fn wrap_in_graph(p: &mut GraphPattern, fresh: &mut Fresh, in_graph: bool) {
    match p {
        GraphPattern::Bgp { patterns } => {
            if in_graph || patterns.is_empty() {
                return;
            }
            // one GRAPH scope PER TRIPLE pattern: a join above the graph operator, so a
            // BGP may join triples from different documents (union-default semantics)
            let triples = std::mem::take(patterns);
            let mut acc: Option<GraphPattern> = None;
            for t in triples {
                let wrapped = GraphPattern::Graph {
                    name: fresh_graph_var(fresh),
                    inner: Box::new(GraphPattern::Bgp { patterns: vec![t] }),
                };
                acc = Some(match acc {
                    None => wrapped,
                    Some(left) => {
                        GraphPattern::Join { left: Box::new(left), right: Box::new(wrapped) }
                    }
                });
            }
            *p = acc.expect("non-empty");
        }
        GraphPattern::Path { .. } => {
            if in_graph {
                return;
            }
            let inner = std::mem::replace(p, GraphPattern::Bgp { patterns: Vec::new() });
            *p = GraphPattern::Graph { name: fresh_graph_var(fresh), inner: Box::new(inner) };
        }
        GraphPattern::Graph { inner, .. } => wrap_in_graph(inner, fresh, true),
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            wrap_in_graph(left, fresh, in_graph);
            wrap_in_graph(right, fresh, in_graph);
        }
        GraphPattern::Lateral { left, right } => {
            wrap_in_graph(left, fresh, in_graph);
            wrap_in_graph(right, fresh, in_graph);
        }
        GraphPattern::LeftJoin { left, right, expression } => {
            wrap_in_graph(left, fresh, in_graph);
            wrap_in_graph(right, fresh, in_graph);
            if let Some(e) = expression {
                wrap_expr(e, fresh, in_graph);
            }
        }
        GraphPattern::Filter { expr, inner } => {
            wrap_expr(expr, fresh, in_graph);
            wrap_in_graph(inner, fresh, in_graph);
        }
        GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. } => wrap_in_graph(inner, fresh, in_graph),
        GraphPattern::Service { inner, .. } => wrap_in_graph(inner, fresh, true),
        GraphPattern::Values { .. } => {}
    }
}

/// EXISTS / NOT EXISTS carry nested patterns; wrap those too.
fn wrap_expr(e: &mut Expression, fresh: &mut Fresh, in_graph: bool) {
    match e {
        Expression::Exists(inner) => wrap_in_graph(inner, fresh, in_graph),
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
            wrap_expr(a, fresh, in_graph);
            wrap_expr(b, fresh, in_graph);
        }
        Expression::In(a, list) => {
            wrap_expr(a, fresh, in_graph);
            for x in list {
                wrap_expr(x, fresh, in_graph);
            }
        }
        Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
            wrap_expr(a, fresh, in_graph);
        }
        Expression::If(a, b, c) => {
            wrap_expr(a, fresh, in_graph);
            wrap_expr(b, fresh, in_graph);
            wrap_expr(c, fresh, in_graph);
        }
        Expression::Coalesce(list) | Expression::FunctionCall(_, list) => {
            for x in list {
                wrap_expr(x, fresh, in_graph);
            }
        }
        Expression::Bound(_) | Expression::NamedNode(_) | Expression::Literal(_) | Expression::Variable(_) => {}
    }
}

// [OPUS-4.8] sq-gq28y: direct unit tests for the spec-conformant default opt-in rewrite
// (`wrap_for_view_opt_in`). End-to-end enforcement (row counts through `PodStore`) is in
// `tests/union_default_graph.rs`; these pin the STRING-rewrite contract directly.
#[cfg(test)]
mod opt_in_tests {
    use super::{wrap_for_view_opt_in, UNION_DEFAULT_GRAPH_IRI};

    fn from_reserved(body: &str) -> String {
        format!("SELECT ?t FROM <{}> WHERE {{ {} }}", UNION_DEFAULT_GRAPH_IRI, body)
    }

    #[test]
    fn no_opt_in_leaves_bare_pattern_unwrapped() {
        // A bare default-graph pattern with NO opt-in is left as-is: no GRAPH wrap, so it
        // evaluates against the view's empty default graph (zero rows) — draft §4.
        let out = wrap_for_view_opt_in("SELECT ?t WHERE { ?s <urn:p> ?t }").unwrap();
        assert!(!out.contains("GRAPH"), "no opt-in must not wrap: {out}");
        assert!(!out.contains(UNION_DEFAULT_GRAPH_IRI));
    }

    #[test]
    fn opt_in_in_default_position_wraps_and_strips_reserved_iri() {
        let out = wrap_for_view_opt_in(&from_reserved("?s <urn:p> ?t")).unwrap();
        assert!(out.contains("GRAPH ?__sg0"), "opt-in must wrap default-graph pattern: {out}");
        // the reserved IRI is a signal, never a real graph name: it must not survive.
        assert!(!out.contains(UNION_DEFAULT_GRAPH_IRI), "reserved IRI must be stripped: {out}");
    }

    #[test]
    fn reserved_iri_in_from_named_is_stripped_not_opt_in() {
        // In FROM NAMED position the reserved IRI names nothing (draft §4): stripped, and
        // NOT treated as the (default-position) opt-in — so a bare pattern stays unwrapped.
        let q = format!(
            "SELECT ?t FROM NAMED <{}> WHERE {{ ?s <urn:p> ?t }}",
            UNION_DEFAULT_GRAPH_IRI
        );
        let out = wrap_for_view_opt_in(&q).unwrap();
        assert!(!out.contains(UNION_DEFAULT_GRAPH_IRI), "reserved IRI must be stripped: {out}");
        assert!(!out.contains("GRAPH"), "FROM NAMED position is not the opt-in: {out}");
    }

    #[test]
    fn near_miss_iri_does_not_trigger_opt_in_fail_closed() {
        // Exact-match discipline: a near-miss IRI is a normal (absent) dataset reference,
        // NOT the opt-in — the bare pattern stays unwrapped (fail-closed, no silent union).
        let q = "SELECT ?t FROM <http://www.w3.org/ns/solid/sparql#union-default-graphX> \
                 WHERE { ?s <urn:p> ?t }";
        let out = wrap_for_view_opt_in(q).unwrap();
        assert!(!out.contains("GRAPH"), "near-miss IRI must not enable union: {out}");
    }

    #[test]
    fn invalid_query_errors() {
        assert!(wrap_for_view_opt_in("SELECT WHERE nonsense").is_err());
    }
}
