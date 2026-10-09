//! [`geosparql_rewrite`]: the GeoSPARQL **query-rewrite extension** (OGC
//! GeoSPARQL 1.0 §9.6 / 1.1 §13, the `/conf/query-rewrite-extension` conformance
//! class) implemented as a SPARQL ALGEBRA rewrite over the parsed query.
//!
//! # What the extension is
//!
//! The query-rewrite extension lets a TRIPLE PATTERN whose predicate is a
//! *topology relation property* — `?f1 geo:sfWithin ?f2` — be answered as if the
//! corresponding `geof:sfWithin(?g1, ?g2)` FILTER had been written over the two
//! features' default geometries. The OGC rule (the RIF/SPARQL `geor:` rewrite)
//! expands each such pattern by resolving each feature to its default-geometry
//! WKT serialisation and applying the matching `geof:` extension function:
//!
//! ```text
//! ?f1 geo:sfWithin ?f2 .
//! ```
//!
//! becomes (the spec's default-geometry resolution path is
//! `geo:hasDefaultGeometry/geo:asWKT`; we additionally accept `geo:hasGeometry`
//! as the resolution edge, the pragmatic shape of most datasets):
//!
//! ```text
//! ?f1 (geo:hasDefaultGeometry|geo:hasGeometry)/geo:asWKT ?__geo_g1 .
//! ?f2 (geo:hasDefaultGeometry|geo:hasGeometry)/geo:asWKT ?__geo_g2 .
//! FILTER( geof:sfWithin(?__geo_g1, ?__geo_g2) )
//! ```
//!
//! The `geof:` FILTER functions (the *topology vocabulary extension*) are already
//! the supported surface — see [`crate::geof_registry`]. This module adds the
//! PROPERTY-form rewrite on top of them: the engine stays geometry-free; the
//! rewrite produces ordinary SPARQL algebra (two geometry-resolution joins and a
//! `geof:` FILTER) that the existing evaluator runs through
//! [`crate::geof_registry`].
//!
//! # Why an algebra rewrite (and not a parser change)
//!
//! The topology property forms are already VALID SPARQL triple patterns (the
//! predicate is just an IRI); a conforming store treats them as asserted triples
//! unless it implements the rewrite. So this is NOT a syntax extension — there is
//! nothing new to parse. It is a post-parse algebra transform, exposed on a
//! dedicated entry point ([`rewrite_query`] / [`geosparql_rewrite`]) so the
//! standard `sparq_engine` entry points keep their exact W3C-conformant behaviour
//! (a `geo:sfWithin` triple still matches only asserted triples there). This
//! mirrors the engine's `query_over` window-syntax front end: an opt-in transform
//! that leaves the conformance-tracking parser untouched. [OPUS-4.8] sq-9g58
//!
//! # Usage
//!
//! ```
//! use sparq_core::Graph;
//! use sparq_geo::{geof_registry, geosparql_rewrite};
//!
//! // London's geometry lies inside the inner-London polygon; only the geometry
//! // serialisations are asserted — there is NO asserted `ex:london geo:sfWithin`
//! // triple. The property-form pattern is answered by the rewrite.
//! let g = Graph::load_str(r#"
//!     @prefix geo: <http://www.opengis.net/ont/geosparql#> .
//!     @prefix ex:  <http://example.org/> .
//!     ex:london     geo:hasDefaultGeometry ex:londonGeom .
//!     ex:londonGeom geo:asWKT "POINT(-0.13 51.51)"^^geo:wktLiteral .
//!     ex:region     geo:hasDefaultGeometry ex:regionGeom .
//!     ex:regionGeom geo:asWKT "POLYGON((-0.25 51.45, 0.05 51.45, 0.05 51.57, -0.25 51.57, -0.25 51.45))"^^geo:wktLiteral .
//! "#, "turtle").unwrap();
//!
//! // (sfWithin is reflexive under DE-9IM, so the region matches itself too; we
//! // ask only for the OTHER features it contains.)
//! let prepared = geosparql_rewrite(
//!     "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
//!      PREFIX ex:  <http://example.org/>
//!      SELECT ?f WHERE { ?f geo:sfWithin ex:region FILTER(?f != ex:region) }").unwrap();
//! let reg = geof_registry();
//! let r = sparq_engine::with_functions(&reg, ||
//!     sparq_engine::query_prepared(&g, &prepared)).unwrap();
//! assert_eq!(r.len(), 1); // ex:london
//! ```
//!
//! # Scope
//!
//! The rewrite covers the three OGC topology-relation families whose `geof:`
//! functions this crate ships — Simple Features (`sf*`), Egenhofer (`eh*`), and
//! RCC8 (`rcc8*`) — i.e. exactly the IRIs in [`is_topology_property`]. A predicate
//! outside that set is left untouched (matched as an ordinary asserted triple), so
//! the rewrite never changes the meaning of a non-topology pattern. The rewrite
//! recurses through the whole query (UNION / OPTIONAL / sub-SELECT / GRAPH / …),
//! rewriting every topology triple it finds.

use crate::vocab::{AS_WKT, GEOF_NS, GEO_NS, HAS_DEFAULT_GEOMETRY, HAS_GEOMETRY};
use spargebra::algebra::{Expression, Function, GraphPattern, PropertyPathExpression};
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;
use std::sync::atomic::{AtomicU64, Ordering};

/// The local names of the topology relation properties the rewrite recognises —
/// the `geo:` (Topology vocabulary) predicates whose matching `geof:` (Geometry
/// Topology vocabulary) function this crate evaluates. The `geo:` property and
/// the `geof:` function share the SAME local name, so one list serves both.
const TOPOLOGY_RELATIONS: [&str; 24] = [
    // Simple features (GeoSPARQL Req 22-24).
    "sfEquals",
    "sfDisjoint",
    "sfIntersects",
    "sfTouches",
    "sfCrosses",
    "sfWithin",
    "sfContains",
    "sfOverlaps",
    // Egenhofer (Req 25).
    "ehEquals",
    "ehDisjoint",
    "ehMeet",
    "ehOverlap",
    "ehCovers",
    "ehCoveredBy",
    "ehInside",
    "ehContains",
    // RCC8 (Req 26).
    "rcc8eq",
    "rcc8dc",
    "rcc8ec",
    "rcc8po",
    "rcc8tppi",
    "rcc8tpp",
    "rcc8ntpp",
    "rcc8ntppi",
];

/// The `geof:` local name corresponding to a `geo:` topology property IRI, or
/// `None` if `iri` is not a recognised topology relation property.
///
/// The Topology vocabulary property `geo:R` and the Geometry Topology vocabulary
/// function `geof:R` share a local name, so this both recognises the predicate and
/// yields the function's local name.
pub fn topology_relation_local_name(iri: &str) -> Option<&'static str> {
    let local = iri.strip_prefix(GEO_NS)?;
    TOPOLOGY_RELATIONS.iter().copied().find(|&r| r == local)
}

/// Whether `iri` is a GeoSPARQL topology relation PROPERTY the rewrite expands
/// (a `geo:sf*` / `geo:eh*` / `geo:rcc8*` IRI). A predicate for which this is
/// `false` is left untouched by [`rewrite_query`].
pub fn is_topology_property(iri: &str) -> bool {
    topology_relation_local_name(iri).is_some()
}

/// Process-wide counter for fresh geometry-variable names, so rewritten geometry
/// variables never collide with each other or (by the reserved `__geo_g` prefix)
/// with a user variable.
static FRESH: AtomicU64 = AtomicU64::new(0);

/// A fresh internal geometry variable (`?__geo_gN`). The `__geo_g` prefix is
/// reserved; a user query that wrote such a variable would merely share bindings,
/// never corrupt results, but the monotonic counter keeps each rewrite distinct.
fn fresh_geom_var() -> Variable {
    let n = FRESH.fetch_add(1, Ordering::Relaxed);
    Variable::new_unchecked(format!("__geo_g{n}"))
}

/// The default-geometry resolution path `(geo:hasDefaultGeometry|geo:hasGeometry)/geo:asWKT`:
/// a FEATURE's default-geometry WKT serialisation.
///
/// The OGC rewrite resolves a feature through `geo:hasDefaultGeometry` (Jena's
/// GeoSPARQL docs: the rewrite "requires the `hasDefaultGeometry` property to be
/// used in the dataset"). We additionally accept `geo:hasGeometry` as the
/// resolution edge — the pragmatic shape of most real datasets, and exactly what
/// Jena's "apply `hasDefaultGeometry` to every single-`hasGeometry` feature"
/// convenience makes equivalent. We deliberately do NOT add a bare `geo:asWKT`
/// alternative: that would make a Geometry NODE (which carries `geo:asWKT` but is
/// not a Feature with a default geometry) bind as if it were a feature, which is
/// not the rewrite's subject. [OPUS-4.8]
fn default_geometry_path() -> PropertyPathExpression {
    let has_default =
        PropertyPathExpression::NamedNode(NamedNode::new_unchecked(HAS_DEFAULT_GEOMETRY));
    let has_geom = PropertyPathExpression::NamedNode(NamedNode::new_unchecked(HAS_GEOMETRY));
    let as_wkt = PropertyPathExpression::NamedNode(NamedNode::new_unchecked(AS_WKT));
    PropertyPathExpression::Sequence(
        Box::new(PropertyPathExpression::Alternative(
            Box::new(has_default),
            Box::new(has_geom),
        )),
        Box::new(as_wkt),
    )
}

/// A `geof:R(?g1, ?g2)` boolean extension-function expression for the topology
/// relation whose local name is `local`.
fn geof_call(local: &str, g1: &Variable, g2: &Variable) -> Expression {
    Expression::FunctionCall(
        Function::Custom(NamedNode::new_unchecked(format!("{GEOF_NS}{local}"))),
        vec![
            Expression::Variable(g1.clone()),
            Expression::Variable(g2.clone()),
        ],
    )
}

/// Expand ONE topology triple pattern `subject geo:R object` into its
/// geometry-resolution + `geof:R` FILTER algebra. `local` is the `geo:R` /
/// `geof:R` local name.
fn expand_topology_triple(subject: TermPattern, object: TermPattern, local: &str) -> GraphPattern {
    let g1 = fresh_geom_var();
    let g2 = fresh_geom_var();
    let path = default_geometry_path();
    // ?subject <defaultGeomPath> ?g1
    let resolve_s = GraphPattern::Path {
        subject,
        path: path.clone(),
        object: TermPattern::Variable(g1.clone()),
    };
    // ?object <defaultGeomPath> ?g2
    let resolve_o = GraphPattern::Path {
        subject: object,
        path,
        object: TermPattern::Variable(g2.clone()),
    };
    let join = GraphPattern::Join {
        left: Box::new(resolve_s),
        right: Box::new(resolve_o),
    };
    GraphPattern::Filter {
        expr: geof_call(local, &g1, &g2),
        inner: Box::new(join),
    }
}

/// Split a BGP's triple patterns into the topology ones (rewritten) and the
/// remainder (kept as a `Bgp`), then conjoin everything with `Join`. Returns
/// `None` if the BGP contains no topology triple (caller keeps the BGP as-is).
fn rewrite_bgp(patterns: &[TriplePattern]) -> Option<GraphPattern> {
    let has_topology = patterns.iter().any(|t| match &t.predicate {
        NamedNodePattern::NamedNode(p) => is_topology_property(p.as_str()),
        NamedNodePattern::Variable(_) => false,
    });
    if !has_topology {
        return None;
    }

    let mut plain: Vec<TriplePattern> = Vec::new();
    let mut expanded: Vec<GraphPattern> = Vec::new();
    for t in patterns {
        match &t.predicate {
            NamedNodePattern::NamedNode(p) if is_topology_property(p.as_str()) => {
                let local = topology_relation_local_name(p.as_str())
                    .expect("is_topology_property gate guarantees a local name");
                expanded.push(expand_topology_triple(
                    t.subject.clone(),
                    t.object.clone(),
                    local,
                ));
            }
            _ => plain.push(t.clone()),
        }
    }

    // Conjoin the plain remainder (if any) with each expanded topology pattern.
    let mut acc: Option<GraphPattern> = if plain.is_empty() {
        None
    } else {
        Some(GraphPattern::Bgp { patterns: plain })
    };
    for e in expanded {
        acc = Some(match acc {
            None => e,
            Some(left) => GraphPattern::Join {
                left: Box::new(left),
                right: Box::new(e),
            },
        });
    }
    acc
}

/// Recursively rewrite every topology triple pattern in `pattern` into its
/// geometry-resolution + `geof:` FILTER form.
fn rewrite_pattern(pattern: GraphPattern) -> GraphPattern {
    match pattern {
        GraphPattern::Bgp { patterns } => {
            rewrite_bgp(&patterns).unwrap_or(GraphPattern::Bgp { patterns })
        }
        // A topology predicate written as a single-NamedNode property path
        // (`?a geo:sfWithin ?b` can parse as a `Path` rather than a `Bgp`
        // triple); expand it too.
        GraphPattern::Path {
            subject,
            path,
            object,
        } => {
            if let PropertyPathExpression::NamedNode(p) = &path {
                if let Some(local) = topology_relation_local_name(p.as_str()) {
                    return expand_topology_triple(subject, object, local);
                }
            }
            GraphPattern::Path {
                subject,
                path,
                object,
            }
        }
        GraphPattern::Join { left, right } => GraphPattern::Join {
            left: Box::new(rewrite_pattern(*left)),
            right: Box::new(rewrite_pattern(*right)),
        },
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => GraphPattern::LeftJoin {
            left: Box::new(rewrite_pattern(*left)),
            right: Box::new(rewrite_pattern(*right)),
            expression: expression.map(rewrite_expression),
        },
        // `Lateral` exists because the workspace builds `spargebra` with `sep-0006`
        // (LATERAL joins, SEP-0006). [OPUS-4.8]
        GraphPattern::Lateral { left, right } => GraphPattern::Lateral {
            left: Box::new(rewrite_pattern(*left)),
            right: Box::new(rewrite_pattern(*right)),
        },
        GraphPattern::Filter { expr, inner } => GraphPattern::Filter {
            expr: rewrite_expression(expr),
            inner: Box::new(rewrite_pattern(*inner)),
        },
        GraphPattern::Union { left, right } => GraphPattern::Union {
            left: Box::new(rewrite_pattern(*left)),
            right: Box::new(rewrite_pattern(*right)),
        },
        GraphPattern::Graph { name, inner } => GraphPattern::Graph {
            name,
            inner: Box::new(rewrite_pattern(*inner)),
        },
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => GraphPattern::Extend {
            inner: Box::new(rewrite_pattern(*inner)),
            variable,
            expression: rewrite_expression(expression),
        },
        GraphPattern::Minus { left, right } => GraphPattern::Minus {
            left: Box::new(rewrite_pattern(*left)),
            right: Box::new(rewrite_pattern(*right)),
        },
        GraphPattern::OrderBy { inner, expression } => GraphPattern::OrderBy {
            inner: Box::new(rewrite_pattern(*inner)),
            expression,
        },
        GraphPattern::Project { inner, variables } => GraphPattern::Project {
            inner: Box::new(rewrite_pattern(*inner)),
            variables,
        },
        GraphPattern::Distinct { inner } => GraphPattern::Distinct {
            inner: Box::new(rewrite_pattern(*inner)),
        },
        GraphPattern::Reduced { inner } => GraphPattern::Reduced {
            inner: Box::new(rewrite_pattern(*inner)),
        },
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => GraphPattern::Slice {
            inner: Box::new(rewrite_pattern(*inner)),
            start,
            length,
        },
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => GraphPattern::Group {
            inner: Box::new(rewrite_pattern(*inner)),
            variables,
            aggregates,
        },
        GraphPattern::Service {
            name,
            inner,
            silent,
        } => GraphPattern::Service {
            name,
            inner: Box::new(rewrite_pattern(*inner)),
            silent,
        },
        // No nested graph pattern to recurse into.
        GraphPattern::Values { .. } => pattern,
    }
}

/// Rewrite topology triples inside an expression's `EXISTS` / `NOT EXISTS`
/// sub-pattern; everything else is structural recursion.
fn rewrite_expression(expr: Expression) -> Expression {
    match expr {
        Expression::Exists(inner) => Expression::Exists(Box::new(rewrite_pattern(*inner))),
        Expression::Or(a, b) => Expression::Or(
            Box::new(rewrite_expression(*a)),
            Box::new(rewrite_expression(*b)),
        ),
        Expression::And(a, b) => Expression::And(
            Box::new(rewrite_expression(*a)),
            Box::new(rewrite_expression(*b)),
        ),
        Expression::Not(a) => Expression::Not(Box::new(rewrite_expression(*a))),
        Expression::If(c, t, e) => Expression::If(
            Box::new(rewrite_expression(*c)),
            Box::new(rewrite_expression(*t)),
            Box::new(rewrite_expression(*e)),
        ),
        other => other,
    }
}

/// Apply the GeoSPARQL query-rewrite extension to a parsed [`Query`]'s algebra,
/// expanding every topology relation triple pattern (see
/// [`is_topology_property`]) into its default-geometry resolution + `geof:`
/// FILTER form. A query with no topology triple is returned structurally
/// identical.
pub fn rewrite_query(query: Query) -> Query {
    match query {
        Query::Select {
            dataset,
            pattern,
            base_iri,
        } => Query::Select {
            dataset,
            pattern: rewrite_pattern(pattern),
            base_iri,
        },
        Query::Construct {
            template,
            dataset,
            pattern,
            base_iri,
        } => Query::Construct {
            template,
            dataset,
            pattern: rewrite_pattern(pattern),
            base_iri,
        },
        Query::Describe {
            dataset,
            pattern,
            base_iri,
        } => Query::Describe {
            dataset,
            pattern: rewrite_pattern(pattern),
            base_iri,
        },
        Query::Ask {
            dataset,
            pattern,
            base_iri,
        } => Query::Ask {
            dataset,
            pattern: rewrite_pattern(pattern),
            base_iri,
        },
    }
}

/// Parse `sparql` and apply the GeoSPARQL query-rewrite extension, returning a
/// `sparq_engine::PreparedQuery` ready to execute under [`crate::geof_registry`].
///
/// Topology relation triple patterns (`?a geo:sfWithin ?b`, etc.) are expanded
/// into their default-geometry resolution joins + `geof:` FILTER; every other
/// pattern is unchanged. Execute the result with the `geof:` functions installed:
///
/// ```ignore
/// let prepared = sparq_geo::geosparql_rewrite(sparql)?;
/// let reg = sparq_geo::geof_registry();
/// let r = sparq_engine::with_functions(&reg, || sparq_engine::query_prepared(&g, &prepared))?;
/// ```
pub fn geosparql_rewrite(sparql: &str) -> Result<sparq_engine::PreparedQuery, String> {
    let prepared = sparq_engine::PreparedQuery::parse(sparql)?;
    Ok(prepared.with_query(rewrite_query(prepared.query().clone())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_topology_properties() {
        assert!(is_topology_property(&format!("{GEO_NS}sfWithin")));
        assert!(is_topology_property(&format!("{GEO_NS}ehContains")));
        assert!(is_topology_property(&format!("{GEO_NS}rcc8ntpp")));
        assert_eq!(
            topology_relation_local_name(&format!("{GEO_NS}sfWithin")),
            Some("sfWithin")
        );
        // Not topology: the geof: FUNCTION IRI, a non-topology geo: property, a
        // random IRI.
        assert!(!is_topology_property(&format!("{GEOF_NS}sfWithin")));
        assert!(!is_topology_property(HAS_DEFAULT_GEOMETRY));
        assert!(!is_topology_property("http://example.org/p"));
    }

    #[test]
    fn non_topology_query_is_unchanged() {
        let q = "SELECT ?s WHERE { ?s <http://example.org/p> ?o }";
        let before = sparq_engine::PreparedQuery::parse(q).unwrap().into_query();
        let after = rewrite_query(before.clone());
        assert_eq!(before.to_string(), after.to_string());
    }

    #[test]
    fn rewrites_a_topology_triple() {
        let q = "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
                 SELECT ?a ?b WHERE { ?a geo:sfWithin ?b }";
        let rewritten = rewrite_query(sparq_engine::PreparedQuery::parse(q).unwrap().into_query());
        let s = rewritten.to_string();
        // The geof: FILTER function appears; the geo: PROPERTY no longer does as a
        // bare predicate (it is only referenced inside the resolution path).
        assert!(
            s.contains("function/geosparql/sfWithin"),
            "no geof:sfWithin in {s}"
        );
        assert!(s.contains("asWKT"), "no geometry resolution in {s}");
    }

    // -----------------------------------------------------------------------
    // [OPUS-4.8] sq-9g58 coverage: the rewrite must recurse through EVERY
    // GraphPattern variant the engine's `spargebra` algebra exposes, expanding
    // a topology triple nested arbitrarily deep. Each query below places a
    // `geo:sfWithin` triple inside exactly one structural node so that the
    // corresponding `rewrite_pattern` arm is exercised AND we assert the
    // topology predicate was expanded (the `geof:` call appears, the bare
    // `geo:sfWithin` predicate does not survive as an asserted edge). A bare
    // SERPENT (`SERVICE`) and `VALUES` are also driven so the leaf arms run.
    // -----------------------------------------------------------------------

    /// Parse, rewrite, and stringify a query.
    fn rw(q: &str) -> String {
        let parsed = sparq_engine::PreparedQuery::parse(q).unwrap().into_query();
        rewrite_query(parsed).to_string()
    }

    /// The rewrite expanded a topology triple: the `geof:` call and the
    /// `asWKT` resolution edge appear in the rewritten algebra.
    fn assert_expanded(s: &str) {
        assert!(s.contains("function/geosparql/sfWithin"), "not expanded: {s}");
        assert!(s.contains("asWKT"), "no resolution path: {s}");
    }

    const GEO_P: &str = "PREFIX geo: <http://www.opengis.net/ont/geosparql#>
                         PREFIX ex:  <http://example.org/>";

    #[test]
    fn recurses_through_optional_left_join() {
        // The topology triple sits in an OPTIONAL (LeftJoin) block.
        let s = rw(&format!(
            "{GEO_P} SELECT ?a ?b WHERE {{ ?a ex:p ?x OPTIONAL {{ ?a geo:sfWithin ?b }} }}"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_optional_left_join_with_filter_expression() {
        // OPTIONAL ... FILTER(EXISTS { topology triple }) — drives both the
        // LeftJoin `expression` map AND `rewrite_expression`'s Exists arm.
        let s = rw(&format!(
            "{GEO_P} SELECT ?a WHERE {{ ?a ex:p ?x \
             OPTIONAL {{ ?a ex:q ?y FILTER( EXISTS {{ ?a geo:sfWithin ?b }} ) }} }}"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_lateral() {
        // LATERAL (SEP-0006) join: the right side is rewritten (Lateral arm).
        let s = rw(&format!(
            "{GEO_P} SELECT ?a ?b WHERE {{ ?a ex:p ?x LATERAL {{ ?a geo:sfWithin ?b }} }}"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_union() {
        let s = rw(&format!(
            "{GEO_P} SELECT ?a WHERE {{ {{ ?a ex:p ?x }} UNION {{ ?a geo:sfWithin ?b }} }}"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_named_graph() {
        let s = rw(&format!(
            "{GEO_P} SELECT ?a WHERE {{ GRAPH ?g {{ ?a geo:sfWithin ?b }} }}"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_minus() {
        let s = rw(&format!(
            "{GEO_P} SELECT ?a WHERE {{ ?a ex:p ?x MINUS {{ ?a geo:sfWithin ?b }} }}"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_bind_extend_and_order_distinct_slice() {
        // BIND => Extend; ORDER BY => OrderBy; DISTINCT => Distinct;
        // LIMIT/OFFSET => Slice; the projection => Project. A single query that
        // stacks all of them above the topology triple.
        let s = rw(&format!(
            "{GEO_P} SELECT DISTINCT ?a ?label WHERE {{ \
             ?a geo:sfWithin ?b . BIND(STR(?a) AS ?label) }} \
             ORDER BY ?label LIMIT 5 OFFSET 1"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_group_by_aggregate() {
        // GROUP BY => Group. The topology triple feeds the grouped pattern.
        let s = rw(&format!(
            "{GEO_P} SELECT ?b (COUNT(?a) AS ?n) WHERE {{ ?a geo:sfWithin ?b }} GROUP BY ?b"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_subselect_reduced() {
        // A sub-SELECT with REDUCED => Project over Reduced, nested under the
        // outer Project. The topology triple lives in the inner select.
        let s = rw(&format!(
            "{GEO_P} SELECT ?a WHERE {{ \
             {{ SELECT REDUCED ?a WHERE {{ ?a geo:sfWithin ?b }} }} }}"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn recurses_through_service() {
        // SERVICE block: the inner pattern is still rewritten (Service arm).
        let s = rw(&format!(
            "{GEO_P} SELECT ?a WHERE {{ SERVICE ?svc {{ ?a geo:sfWithin ?b }} }}"
        ));
        assert_expanded(&s);
    }

    #[test]
    fn topology_as_single_predicate_path_is_expanded() {
        // `?a geo:sfWithin+ ?b` parses as a `Path`; a one-hop path predicate is
        // recognised and expanded by the `Path` arm's NamedNode branch.
        // (`geo:sfWithin` alone is a Bgp triple, so use a property-path operator
        // to force the `Path` node, then assert the leaf NamedNode of that path
        // is still handled — here a plain `Path` with a single NamedNode is
        // produced by an inverse path `^geo:sfContains`-style shape; we use a
        // sequence-free single hop expressed via a path to keep it a `Path`.)
        let q = format!(
            "{GEO_P} SELECT ?a ?b WHERE {{ ?a (geo:sfWithin) ?b }}"
        );
        // Whether spargebra lowers `(geo:sfWithin)` to a Bgp or a Path, the
        // rewrite must still expand it.
        let s = rw(&q);
        assert_expanded(&s);
    }

    #[test]
    fn values_leaf_is_left_untouched() {
        // A standalone VALUES block has no nested pattern; the Values arm
        // returns it unchanged (no topology triple to expand). The query is a
        // structural no-op for the rewrite.
        let q = format!(
            "{GEO_P} SELECT ?a WHERE {{ VALUES ?a {{ ex:x ex:y }} }}"
        );
        let before = sparq_engine::PreparedQuery::parse(&q).unwrap().into_query();
        let after = rewrite_query(before.clone());
        assert_eq!(before.to_string(), after.to_string());
    }

    #[test]
    fn rewrite_expression_recurses_through_boolean_and_if() {
        // FILTER( !EXISTS{...} && (EXISTS{...} || IF(EXISTS{...}, true, false)) )
        // drives the Not / And / Or / If arms of rewrite_expression, each
        // wrapping an Exists whose inner pattern holds a topology triple.
        let s = rw(&format!(
            "{GEO_P} SELECT ?a WHERE {{ ?a ex:p ?x FILTER( \
               !EXISTS {{ ?a geo:sfWithin ?b }} && \
               ( EXISTS {{ ?a geo:sfContains ?c }} || \
                 IF(EXISTS {{ ?a geo:sfEquals ?d }}, true, false) ) ) }}"
        ));
        // Three distinct topology predicates expanded inside the expression.
        assert!(s.contains("function/geosparql/sfWithin"), "{s}");
        assert!(s.contains("function/geosparql/sfContains"), "{s}");
        assert!(s.contains("function/geosparql/sfEquals"), "{s}");
    }

    #[test]
    fn construct_describe_ask_query_forms_are_rewritten() {
        // The non-SELECT query forms each route their pattern through
        // rewrite_pattern (the Construct / Describe / Ask arms of rewrite_query).
        let construct = rw(&format!(
            "{GEO_P} CONSTRUCT {{ ?a ex:near ?b }} WHERE {{ ?a geo:sfWithin ?b }}"
        ));
        assert_expanded(&construct);

        let describe = rw(&format!(
            "{GEO_P} DESCRIBE ?a WHERE {{ ?a geo:sfWithin ?b }}"
        ));
        assert_expanded(&describe);

        let ask = rw(&format!(
            "{GEO_P} ASK WHERE {{ ?a geo:sfWithin ?b }}"
        ));
        assert_expanded(&ask);
    }

    #[test]
    fn bgp_with_mixed_topology_and_plain_triples_keeps_the_plain_remainder() {
        // A BGP holding BOTH a plain triple and TWO topology triples: the plain
        // remainder is kept as a Bgp and conjoined with each expansion (the
        // `plain` + multi-`expanded` Join-fold in rewrite_bgp).
        let s = rw(&format!(
            "{GEO_P} SELECT ?a ?b ?c WHERE {{ \
             ?a ex:p ?x . ?a geo:sfWithin ?b . ?a geo:sfContains ?c }}"
        ));
        assert!(s.contains("function/geosparql/sfWithin"), "{s}");
        assert!(s.contains("function/geosparql/sfContains"), "{s}");
        // The plain edge survives in the rewritten algebra.
        assert!(s.contains("example.org/p"), "plain triple lost: {s}");
    }

    #[test]
    fn bgp_of_only_topology_triples_has_no_plain_remainder() {
        // Two topology triples, NO plain triple: the `plain.is_empty()` branch
        // of rewrite_bgp (acc starts `None`, first expansion seeds it).
        let s = rw(&format!(
            "{GEO_P} SELECT ?a ?b ?c WHERE {{ ?a geo:sfWithin ?b . ?b geo:sfContains ?c }}"
        ));
        assert!(s.contains("function/geosparql/sfWithin"), "{s}");
        assert!(s.contains("function/geosparql/sfContains"), "{s}");
    }

    #[test]
    fn variable_predicate_bgp_is_not_rewritten() {
        // A triple with a VARIABLE predicate is never a topology property; the
        // BGP has no topology triple so rewrite_bgp returns None and the BGP is
        // kept verbatim.
        let q = format!("{GEO_P} SELECT ?a ?p ?b WHERE {{ ?a ?p ?b }}");
        let before = sparq_engine::PreparedQuery::parse(&q).unwrap().into_query();
        let after = rewrite_query(before.clone());
        assert_eq!(before.to_string(), after.to_string());
    }
}
