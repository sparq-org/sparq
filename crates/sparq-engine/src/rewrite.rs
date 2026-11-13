//! (sq-7d3dj.30.1) Pre-execution SPARQL algebra rewrite pass.
//!
//! 🤖 SPARQ agent. A pure, **bag-result-equivalent** transform over the parsed
//! `spargebra` algebra, applied once by [`crate::PreparedQuery::parse`] (and the
//! EXPLAIN parse sites) BEFORE evaluation. It is the fix for the SP2Bench
//! complex-shape latency deficit (design record:
//! `research/sp2bench-complex-shape-deficit.md` §2.1 / §2.5 / §4). Two rewrites:
//!
//! **(a) Equality substitution** — `FILTER(?v = <iri>)` (also `<iri> = ?v` and
//! `sameTerm(?v, <iri>)`) whose `?v` occurs in the group's triple patterns folds
//! the IRI constant into those patterns, turning a post-join FILTER over every
//! enumerated row (SP2Bench q03b/q03c: ~250k rows) into a constant-seeded indexed
//! scan. `?v` is re-bound to the constant via a spargebra `Extend` node so it
//! stays in scope (SELECT `*`, downstream joins, the residual FILTER).
//!
//! Restricted to **IRI (`NamedNode`) constants**: `=` on two IRIs is
//! term-identity, so the substitution is exact. Literal `=` is *value* equality
//! (`"1"^^xsd:int = "01"^^xsd:integer`, and the open high-precision `xsd:decimal`
//! class tracked by `sq-lr2ii`) and is **never** rewritten — this restriction is
//! the stated `sq-lr2ii` avoidance contract (design record §4). No numeric
//! promotion, no `f64`, no value space is ever touched here.
//!
//! **(b) Anti-join** — `FILTER(!bound(?v), LeftJoin(A, B))` (an `OPTIONAL … FILTER
//! not bound` negation idiom, SP2Bench q07) rewrites to `Minus(A, B)` when `?v` is
//! *certain* in `B` (bound in every solution of `B`), does not occur in `A`, and
//! `A` and `B` share a *certain* variable. Those three conditions are exactly what
//! makes `Filter(!bound(?v), LeftJoin(A,B))` and `Minus(A,B)` coincide (they differ
//! only when `A` and `B` share no bound variable, or when `?v` is merely optional in
//! `B`; both are excluded here).
//!
//! **Conservatism is load-bearing.** Any shape that does not *exactly* meet a
//! rewrite's conditions is returned VERBATIM — the un-rewritten algebra is the
//! current behaviour, so declining is always safe. The pass never crosses an
//! `OPTIONAL`/`UNION`/`MINUS`/subquery/`GRAPH`/`SERVICE` scope boundary during
//! substitution, and it re-checks every well-formedness condition before firing.

use oxrdf::{NamedNode, Variable};
use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::Query;
use std::borrow::Cow;

/// Rewrites a parsed query's graph pattern with the two result-equivalent
/// rewrites (see the module docs), moving an unchanged query through verbatim.
/// See [`rewrite_query_ref`] for explicit change reporting.
pub fn rewrite_query(query: Query) -> Query {
    rewrite_query_ref(&query).unwrap_or(query)
}

/// Reports a rewrite with `Some(rewritten)`, or `None` when no pass fires.
///
/// The passes propagate changes directly, without comparing input and output.
/// An unchanged query is only borrowed: no algebra is cloned. When a pass fires,
/// only changed ancestors and their retained fields/siblings are materialized.
/// Dataset clauses, base IRIs and CONSTRUCT templates are preserved verbatim.
pub fn rewrite_query_ref(query: &Query) -> Option<Query> {
    match query {
        Query::Select {
            dataset,
            pattern,
            base_iri,
        } => rewrite_pattern(pattern).map(|pattern| Query::Select {
            dataset: dataset.clone(),
            pattern,
            base_iri: base_iri.clone(),
        }),
        Query::Ask {
            dataset,
            pattern,
            base_iri,
        } => rewrite_pattern(pattern).map(|pattern| Query::Ask {
            dataset: dataset.clone(),
            pattern,
            base_iri: base_iri.clone(),
        }),
        Query::Construct {
            template,
            dataset,
            pattern,
            base_iri,
        } => rewrite_pattern(pattern).map(|pattern| Query::Construct {
            template: template.clone(),
            dataset: dataset.clone(),
            pattern,
            base_iri: base_iri.clone(),
        }),
        Query::Describe {
            dataset,
            pattern,
            base_iri,
        } => rewrite_pattern(pattern).map(|pattern| Query::Describe {
            dataset: dataset.clone(),
            pattern,
            base_iri: base_iri.clone(),
        }),
    }
}

/// Rewrites both children, cloning an unchanged sibling only if the other changed.
fn rewrite_pair(
    left: &GraphPattern,
    right: &GraphPattern,
) -> Option<(Box<GraphPattern>, Box<GraphPattern>)> {
    let l = rewrite_pattern(left);
    let r = rewrite_pattern(right);
    if l.is_none() && r.is_none() {
        return None;
    }
    Some((
        Box::new(l.unwrap_or_else(|| left.clone())),
        Box::new(r.unwrap_or_else(|| right.clone())),
    ))
}

/// Bottom-up transform. `Some` propagates every local or descendant rewrite;
/// `None` borrows the original subtree. A FILTER consumes an already rewritten
/// child directly, without cloning it or materializing a throwaway expression.
/// EXISTS sub-patterns remain untouched, matching the pass's scope boundary.
fn rewrite_pattern(p: &GraphPattern) -> Option<GraphPattern> {
    use GraphPattern as G;
    let r = |inner: &G| rewrite_pattern(inner).map(Box::new);
    match p {
        G::Bgp { .. } | G::Path { .. } | G::Values { .. } => None,
        G::Join { left, right } => {
            rewrite_pair(left, right).map(|(left, right)| G::Join { left, right })
        }
        G::LeftJoin {
            left,
            right,
            expression,
        } => rewrite_pair(left, right).map(|(left, right)| G::LeftJoin {
            left,
            right,
            expression: expression.clone(),
        }),
        G::Lateral { left, right } => {
            rewrite_pair(left, right).map(|(left, right)| G::Lateral { left, right })
        }
        G::Filter { expr, inner } => rewrite_filter(
            expr,
            rewrite_pattern(inner).map_or(Cow::Borrowed(inner.as_ref()), Cow::Owned),
        ),
        G::Union { left, right } => {
            rewrite_pair(left, right).map(|(left, right)| G::Union { left, right })
        }
        G::Graph { name, inner } => r(inner).map(|inner| G::Graph {
            name: name.clone(),
            inner,
        }),
        G::Extend {
            inner,
            variable,
            expression,
        } => r(inner).map(|inner| G::Extend {
            inner,
            variable: variable.clone(),
            expression: expression.clone(),
        }),
        G::Minus { left, right } => {
            rewrite_pair(left, right).map(|(left, right)| G::Minus { left, right })
        }
        G::OrderBy { inner, expression } => r(inner).map(|inner| G::OrderBy {
            inner,
            expression: expression.clone(),
        }),
        G::Project { inner, variables } => r(inner).map(|inner| G::Project {
            inner,
            variables: variables.clone(),
        }),
        G::Distinct { inner } => r(inner).map(|inner| G::Distinct { inner }),
        G::Reduced { inner } => r(inner).map(|inner| G::Reduced { inner }),
        G::Slice {
            inner,
            start,
            length,
        } => r(inner).map(|inner| G::Slice {
            inner,
            start: *start,
            length: *length,
        }),
        G::Group {
            inner,
            variables,
            aggregates,
        } => r(inner).map(|inner| G::Group {
            inner,
            variables: variables.clone(),
            aggregates: aggregates.clone(),
        }),
        G::Service {
            name,
            inner,
            silent,
        } => r(inner).map(|inner| G::Service {
            name: name.clone(),
            inner,
            silent: *silent,
        }),
    }
}

/// Reports a local or descendant rewrite, checking anti-join eligibility first.
/// An owned child has already been rebuilt and must be moved through either
/// pass (or retained under the original FILTER when both passes decline).
fn rewrite_filter(expr: &Expression, inner: Cow<'_, GraphPattern>) -> Option<GraphPattern> {
    if let Some(v) = as_not_bound(expr) {
        if let GraphPattern::LeftJoin {
            left,
            right,
            expression: None,
        } = inner.as_ref()
        {
            if antijoin_ok(v, left, right) {
                let GraphPattern::LeftJoin { left, right, .. } = inner.into_owned() else {
                    unreachable!()
                };
                return Some(GraphPattern::Minus { left, right });
            }
        }
    }
    let applied = equality_substitutions(expr, &inner);
    if !applied.is_empty() {
        return Some(equality_substitute(expr, inner, applied));
    }
    match inner {
        Cow::Owned(inner) => Some(GraphPattern::Filter {
            expr: expr.clone(),
            inner: Box::new(inner),
        }),
        Cow::Borrowed(_) => None,
    }
}

/// `Some(v)` iff `e` is exactly `!BOUND(?v)` (`Not(Bound(v))`).
fn as_not_bound(e: &Expression) -> Option<&Variable> {
    match e {
        Expression::Not(inner) => match inner.as_ref() {
            Expression::Bound(v) => Some(v),
            _ => None,
        },
        _ => None,
    }
}

/// The anti-join well-formedness conditions (see module docs): `v` certain in
/// `right`, absent from `left`, and `left`/`right` share a certain variable.
fn antijoin_ok(v: &Variable, left: &GraphPattern, right: &GraphPattern) -> bool {
    let right_certain = certain_vars(right);
    if !right_certain.contains(v) {
        return false;
    }
    if pattern_contains_var(left, v) {
        return false;
    }
    let left_certain = certain_vars(left);
    let mut shared = false;
    left.on_in_scope_variable(|w| {
        // Probe the right first so disjoint variables never rescan the left.
        shared = shared || (right_certain.contains(w) && left_certain.contains(w));
    });
    shared
}

/// Eligible IRI substitutions in conjunct order. A declined pass returns an
/// empty vector without cloning or allocating.
fn equality_substitutions<'a>(
    expr: &'a Expression,
    inner: &GraphPattern,
) -> Vec<(&'a Variable, &'a NamedNode)> {
    // Check candidate IRI-equality conjuncts, keyed by variable. A variable
    // that appears in more than one equality conjunct is dropped (a possible
    // contradiction, e.g. `?v = <a> && ?v = <b>`), keeping those conjuncts as
    // residual — conservative.
    let mut applied: Vec<(&Variable, &NamedNode)> = Vec::new();
    visit_conjuncts(expr, &mut |c| {
        let Some((var, iri)) = as_iri_equality(c) else {
            return;
        };
        let mut occurrences = 0;
        visit_conjuncts(expr, &mut |other| {
            if as_iri_equality(other).is_some_and(|(v, _)| v == var) {
                occurrences += 1;
            }
        });
        if occurrences != 1 {
            return;
        }
        // The variable must occur in a substitutable spine position and NOWHERE
        // inside an opaque scope boundary.
        let (in_spine, in_opaque) = scan_var(inner, var);
        if in_spine && !in_opaque {
            applied.push((var, iri));
        }
    });

    applied
}

/// Applies known eligible substitutions, consuming the child rebuilt below us.
fn equality_substitute(
    expr: &Expression,
    inner: Cow<'_, GraphPattern>,
    applied: Vec<(&Variable, &NamedNode)>,
) -> GraphPattern {
    // Eligibility only borrows the input. Once a substitution is known to fire,
    // materialize a borrowed spine or reuse an already rebuilt owned spine.
    let mut inner = inner.into_owned();
    for (var, iri) in &applied {
        apply_sub_spine(&mut inner, var, iri);
    }
    for (var, iri) in &applied {
        inner = GraphPattern::Extend {
            inner: Box::new(inner),
            variable: (*var).clone(),
            expression: Expression::NamedNode((*iri).clone()),
        };
    }

    // Rebuild the residual FILTER from the non-consumed conjuncts (in order).
    let mut residual = Vec::new();
    visit_conjuncts(expr, &mut |c| {
        if !as_iri_equality(c).is_some_and(|(v, _)| applied.iter().any(|(var, _)| *var == v)) {
            residual.push(c.clone());
        }
    });
    match fold_and(residual) {
        Some(residual_expr) => GraphPattern::Filter {
            expr: residual_expr,
            inner: Box::new(inner),
        },
        None => inner,
    }
}

/// Visits a top-level AND tree's conjuncts in left-to-right order without a buffer.
fn visit_conjuncts<'a>(e: &'a Expression, visit: &mut impl FnMut(&'a Expression)) {
    match e {
        Expression::And(a, b) => {
            visit_conjuncts(a, visit);
            visit_conjuncts(b, visit);
        }
        other => visit(other),
    }
}

/// Left-associative `&&`-fold of a conjunct list (inverse of [`visit_conjuncts`]).
/// `None` for an empty list (the FILTER is fully consumed).
fn fold_and(conjuncts: Vec<Expression>) -> Option<Expression> {
    let mut it = conjuncts.into_iter();
    let first = it.next()?;
    Some(it.fold(first, |acc, e| Expression::And(Box::new(acc), Box::new(e))))
}

/// `Some((v, iri))` iff `e` is `?v = <iri>` / `<iri> = ?v` / `sameTerm(?v, <iri>)`
/// / `sameTerm(<iri>, ?v)`. **IRI-only**: a `Literal` operand (numeric or plain)
/// returns `None` and is never rewritten (the `sq-lr2ii` avoidance contract).
fn as_iri_equality(e: &Expression) -> Option<(&Variable, &NamedNode)> {
    let (a, b) = match e {
        Expression::Equal(a, b) | Expression::SameTerm(a, b) => (a.as_ref(), b.as_ref()),
        _ => return None,
    };
    match (a, b) {
        (Expression::Variable(v), Expression::NamedNode(n))
        | (Expression::NamedNode(n), Expression::Variable(v)) => Some((v, n)),
        _ => None,
    }
}

/// Walks the substitutable *spine* of `p` (BGP / property-path / `Join`), reporting
/// `(occurs_in_spine, occurs_in_opaque)` for `var`. An opaque child (any operator
/// other than `Bgp`/`Path`/`Join`, e.g. `LeftJoin`/`Union`/`Minus`/`Graph`/a
/// subquery `Project`/a nested `Filter`) is checked whole via
/// [`pattern_contains_var`]: if `var` appears inside one, it is NOT substitutable.
fn scan_var(p: &GraphPattern, var: &Variable) -> (bool, bool) {
    let mut in_spine = false;
    let mut in_opaque = false;
    scan_var_inner(p, var, &mut in_spine, &mut in_opaque);
    (in_spine, in_opaque)
}

fn scan_var_inner(p: &GraphPattern, var: &Variable, in_spine: &mut bool, in_opaque: &mut bool) {
    match p {
        GraphPattern::Bgp { patterns } => {
            if patterns.iter().any(|t| triple_has_var(t, var)) {
                *in_spine = true;
            }
        }
        GraphPattern::Path { subject, path, object } => {
            if term_has_var(subject, var) || term_has_var(object, var) {
                // A path that can match zero steps pairs a CONSTANT endpoint with itself
                // even when it is not in the graph, while a variable endpoint ranges only
                // over graph terms: substituting would invent matches. Decline.
                if path_may_be_empty(path) {
                    *in_opaque = true;
                } else {
                    *in_spine = true;
                }
            }
        }
        GraphPattern::Join { left, right } => {
            scan_var_inner(left, var, in_spine, in_opaque);
            scan_var_inner(right, var, in_spine, in_opaque);
        }
        other => {
            if pattern_contains_var(other, var) {
                *in_opaque = true;
            }
        }
    }
}

/// Whether `p` can match a zero-length path (conservative: `true` when unsure).
fn path_may_be_empty(p: &PropertyPathExpression) -> bool {
    use PropertyPathExpression as P;
    match p {
        P::ZeroOrMore(_) | P::ZeroOrOne(_) => true,
        P::NamedNode(_) | P::NegatedPropertySet(_) => false,
        P::OneOrMore(q) => path_may_be_empty(q),
        P::Reverse(q) => path_may_be_empty(q),
        P::Sequence(a, b) => path_may_be_empty(a) && path_may_be_empty(b),
        P::Alternative(a, b) => path_may_be_empty(a) || path_may_be_empty(b),
    }
}

/// Substitutes `iri` for `var` in every triple-pattern position along the spine
/// (BGP / property-path / `Join`). Opaque children are left untouched — the caller
/// has already proven (`scan_var`) that `var` does not occur inside any of them.
fn apply_sub_spine(p: &mut GraphPattern, var: &Variable, iri: &NamedNode) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for t in patterns.iter_mut() {
                subst_triple(t, var, iri);
            }
        }
        GraphPattern::Path { subject, object, .. } => {
            subst_term(subject, var, iri);
            subst_term(object, var, iri);
        }
        GraphPattern::Join { left, right } => {
            apply_sub_spine(left, var, iri);
            apply_sub_spine(right, var, iri);
        }
        _ => {}
    }
}

// ---- term / triple substitution + occurrence helpers -----------------------

fn subst_triple(t: &mut TriplePattern, var: &Variable, iri: &NamedNode) {
    subst_term(&mut t.subject, var, iri);
    subst_named_node(&mut t.predicate, var, iri);
    subst_term(&mut t.object, var, iri);
}

fn subst_term(tp: &mut TermPattern, var: &Variable, iri: &NamedNode) {
    match tp {
        TermPattern::Variable(v) if v == var => *tp = TermPattern::NamedNode(iri.clone()),
        // RDF-star nested triple term (sparql-12 is always on in the workspace).
        TermPattern::Triple(t) => subst_triple(t, var, iri),
        _ => {}
    }
}

fn subst_named_node(nnp: &mut NamedNodePattern, var: &Variable, iri: &NamedNode) {
    if let NamedNodePattern::Variable(v) = nnp {
        if v == var {
            *nnp = NamedNodePattern::NamedNode(iri.clone());
        }
    }
}

fn triple_has_var(t: &TriplePattern, var: &Variable) -> bool {
    term_has_var(&t.subject, var) || nnp_has_var(&t.predicate, var) || term_has_var(&t.object, var)
}

fn term_has_var(tp: &TermPattern, var: &Variable) -> bool {
    match tp {
        TermPattern::Variable(v) => v == var,
        TermPattern::Triple(t) => triple_has_var(t, var),
        _ => false,
    }
}

fn nnp_has_var(nnp: &NamedNodePattern, var: &Variable) -> bool {
    matches!(nnp, NamedNodePattern::Variable(v) if v == var)
}

// ---- whole-subtree occurrence + certain-variable analysis ------------------

/// Does `var` occur ANYWHERE in `p` — any triple position or any expression
/// (including `EXISTS` sub-patterns)? Used for the `v ∉ vars(A)` anti-join guard
/// and for the opaque-scope check.
fn pattern_contains_var(p: &GraphPattern, var: &Variable) -> bool {
    match p {
        GraphPattern::Bgp { patterns } => patterns.iter().any(|t| triple_has_var(t, var)),
        GraphPattern::Path { subject, object, .. } => {
            term_has_var(subject, var) || term_has_var(object, var)
        }
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right }
        | GraphPattern::Lateral { left, right } => {
            pattern_contains_var(left, var) || pattern_contains_var(right, var)
        }
        GraphPattern::LeftJoin { left, right, expression } => {
            pattern_contains_var(left, var)
                || pattern_contains_var(right, var)
                || expression.as_ref().is_some_and(|e| expr_has_var(e, var))
        }
        GraphPattern::Filter { expr, inner } => {
            expr_has_var(expr, var) || pattern_contains_var(inner, var)
        }
        GraphPattern::Graph { name, inner } => {
            nnp_has_var(name, var) || pattern_contains_var(inner, var)
        }
        GraphPattern::Extend { inner, variable, expression } => {
            variable == var || expr_has_var(expression, var) || pattern_contains_var(inner, var)
        }
        GraphPattern::Values { variables, .. } => variables.iter().any(|v| v == var),
        GraphPattern::OrderBy { inner, expression } => {
            pattern_contains_var(inner, var)
                || expression.iter().any(|o| match o {
                    OrderExpression::Asc(e) | OrderExpression::Desc(e) => expr_has_var(e, var),
                })
        }
        GraphPattern::Project { inner, variables } => {
            variables.iter().any(|v| v == var) || pattern_contains_var(inner, var)
        }
        GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => pattern_contains_var(inner, var),
        GraphPattern::Group { inner, variables, aggregates } => {
            variables.iter().any(|v| v == var)
                || aggregates.iter().any(|(o, agg)| o == var || agg_has_var(agg, var))
                || pattern_contains_var(inner, var)
        }
        GraphPattern::Service { name, inner, .. } => {
            nnp_has_var(name, var) || pattern_contains_var(inner, var)
        }
    }
}

fn agg_has_var(agg: &AggregateExpression, var: &Variable) -> bool {
    match agg {
        AggregateExpression::CountSolutions { .. } => false,
        AggregateExpression::FunctionCall { expr, .. } => expr_has_var(expr, var),
    }
}

fn expr_has_var(e: &Expression, var: &Variable) -> bool {
    match e {
        Expression::Variable(v) | Expression::Bound(v) => v == var,
        Expression::NamedNode(_) | Expression::Literal(_) => false,
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
        | Expression::Divide(a, b) => expr_has_var(a, var) || expr_has_var(b, var),
        Expression::In(a, list) => expr_has_var(a, var) || list.iter().any(|x| expr_has_var(x, var)),
        Expression::UnaryPlus(i) | Expression::UnaryMinus(i) | Expression::Not(i) => {
            expr_has_var(i, var)
        }
        Expression::If(a, b, c) => {
            expr_has_var(a, var) || expr_has_var(b, var) || expr_has_var(c, var)
        }
        Expression::Coalesce(list) | Expression::FunctionCall(_, list) => {
            list.iter().any(|x| expr_has_var(x, var))
        }
        Expression::Exists(p) => pattern_contains_var(p, var),
    }
}

/// A SOUND under-approximation of the variables bound in EVERY solution of `p`
/// (the "certain" / definitely-bound variables). Used only by the anti-join
/// guard; being conservative (omitting a genuinely-certain variable) can only
/// make the rewrite decline, never fire wrongly.
fn certain_vars(p: &GraphPattern) -> CertainVars<'_> {
    CertainVars(p)
}

/// A borrowed set view keeps declined anti-join checks allocation-free.
struct CertainVars<'a>(&'a GraphPattern);

impl CertainVars<'_> {
    fn contains(&self, var: &Variable) -> bool {
        match self.0 {
            GraphPattern::Bgp { patterns } => patterns.iter().any(|t| triple_has_var(t, var)),
            GraphPattern::Path { subject, object, .. } => {
                term_has_var(subject, var) || term_has_var(object, var)
            }
            GraphPattern::Join { left, right } | GraphPattern::Lateral { left, right } => {
                certain_vars(left).contains(var) || certain_vars(right).contains(var)
            }
            GraphPattern::LeftJoin { left, .. } | GraphPattern::Minus { left, .. } => {
                certain_vars(left).contains(var)
            }
            GraphPattern::Filter { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::OrderBy { inner, .. }
            // Extend may error, so its output is not certain.
            | GraphPattern::Extend { inner, .. } => certain_vars(inner).contains(var),
            GraphPattern::Union { left, right } => {
                certain_vars(left).contains(var) && certain_vars(right).contains(var)
            }
            GraphPattern::Graph { name, inner } => {
                nnp_has_var(name, var) || certain_vars(inner).contains(var)
            }
            GraphPattern::Project { inner, variables }
            // Aggregate outputs may error; only certain grouping keys survive.
            | GraphPattern::Group { inner, variables, .. } => {
                variables.contains(var) && certain_vars(inner).contains(var)
            }
            GraphPattern::Values { variables, bindings } => {
                variables.iter().enumerate().any(|(i, v)| {
                    v == var && bindings.iter().all(|row| row.get(i).is_some_and(|cell| cell.is_some()))
                })
            }
            // SILENT SERVICE may yield the unit solution (nothing certain).
            GraphPattern::Service { inner, silent, .. } => {
                !silent && certain_vars(inner).contains(var)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spargebra::SparqlParser;

    fn parse(q: &str) -> Query {
        SparqlParser::new().parse_query(q).unwrap()
    }

    const PFX: &str = "PREFIX ex: <http://ex/> PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

    fn pattern_of(q: &Query) -> &GraphPattern {
        match q {
            Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
            _ => unreachable!(),
        }
    }

    // ---- rewrite_query: equality substitution fires on an IRI FILTER ----
    #[test]
    fn iri_equality_folds_and_drops_filter() {
        let raw = parse(&format!(
            "{PFX} SELECT ?s ?v WHERE {{ ?s a ex:Thing . ?s ?p ?v . FILTER(?p = ex:target) }}"
        ));
        let rw = rewrite_query(raw.clone());
        assert_ne!(rw, raw, "IRI-equality FILTER must be rewritten");
        let dbg = format!("{:?}", pattern_of(&rw));
        assert!(!dbg.contains("Filter"), "the equality FILTER must be consumed: {}", dbg);
        assert!(dbg.contains("Extend"), "?p must be re-bound via Extend: {}", dbg);
        assert!(dbg.contains("http://ex/target"), "the IRI must be folded in: {}", dbg);
    }

    // ---- rewrite_query: literal equality is NEVER rewritten (sq-lr2ii) ----
    #[test]
    fn integer_literal_equality_not_rewritten() {
        let raw = parse(&format!("{PFX} SELECT ?s ?v WHERE {{ ?s ex:num ?v . FILTER(?v = 1) }}"));
        assert_eq!(rewrite_query(raw.clone()), raw, "numeric-literal FILTER must not be rewritten");
    }

    #[test]
    fn decimal_literal_equality_not_rewritten() {
        let raw = parse(&format!(
            "{PFX} SELECT ?s ?v WHERE {{ ?s ex:num ?v . FILTER(?v = \"1\"^^xsd:decimal) }}"
        ));
        assert_eq!(rewrite_query(raw.clone()), raw, "decimal-literal FILTER must not be rewritten");
    }

    #[test]
    fn plain_literal_equality_not_rewritten() {
        let raw =
            parse(&format!("{PFX} SELECT ?s ?v WHERE {{ ?s ex:name ?v . FILTER(?v = \"x\") }}"));
        assert_eq!(rewrite_query(raw.clone()), raw, "plain-literal FILTER must not be rewritten");
    }

    // ---- as_iri_equality direct unit coverage ----
    #[test]
    fn as_iri_equality_matches_both_orientations_and_sameterm() {
        let v = Variable::new("p").unwrap();
        let n = NamedNode::new("http://ex/target").unwrap();
        let ev = Expression::Variable(v.clone());
        let en = Expression::NamedNode(n.clone());
        assert!(as_iri_equality(&Expression::Equal(Box::new(ev.clone()), Box::new(en.clone())))
            .is_some());
        assert!(as_iri_equality(&Expression::Equal(Box::new(en.clone()), Box::new(ev.clone())))
            .is_some());
        assert!(as_iri_equality(&Expression::SameTerm(Box::new(ev.clone()), Box::new(en.clone())))
            .is_some());
        // Literal operand → None.
        let lit = Expression::Literal(oxrdf::Literal::new_simple_literal("x"));
        assert!(as_iri_equality(&Expression::Equal(Box::new(ev), Box::new(lit))).is_none());
    }

    // ---- visit_conjuncts / fold_and round trip ----
    #[test]
    fn conjunct_split_and_fold_roundtrip() {
        assert!(fold_and(Vec::new()).is_none());
        let a = Expression::Variable(Variable::new("a").unwrap());
        let b = Expression::Variable(Variable::new("b").unwrap());
        let c = Expression::Variable(Variable::new("c").unwrap());
        // ((a && b) && c) → three conjuncts, and folding them re-associates back.
        let e = Expression::And(
            Box::new(Expression::And(Box::new(a.clone()), Box::new(b.clone()))),
            Box::new(c.clone()),
        );
        let mut cs = Vec::new();
        visit_conjuncts(&e, &mut |c| cs.push(c));
        assert_eq!(cs.len(), 3, "three top-level && conjuncts");
        assert_eq!(cs[0], &a);
        assert_eq!(cs[1], &b);
        assert_eq!(cs[2], &c);
        assert_eq!(fold_and(cs.into_iter().cloned().collect()), Some(e), "fold_and is the left-assoc inverse of visit_conjuncts");
    }

    // ---- residual conjunct is preserved when only part of the AND is IRI-eq ----
    #[test]
    fn residual_conjunct_kept() {
        let raw = parse(&format!(
            "{PFX} SELECT ?s ?v WHERE {{ ?s ?p ?v . FILTER(?p = ex:target && ?v != ex:skip) }}"
        ));
        let rw = rewrite_query(raw.clone());
        let dbg = format!("{:?}", pattern_of(&rw));
        assert_ne!(rw, raw);
        // The `?p = ex:target` conjunct is consumed; the `?v != ex:skip` residual
        // FILTER stays (over the Extend that re-binds ?p).
        assert!(dbg.contains("Filter"), "residual FILTER must remain: {}", dbg);
        assert!(dbg.contains("Extend"), "?p re-bound: {}", dbg);
        assert!(dbg.contains("http://ex/target"));
    }

    // ---- variable in an opaque OPTIONAL scope → declines ----
    #[test]
    fn opaque_scope_declines() {
        // ?p occurs in an OPTIONAL (a LeftJoin, opaque) — the whole inner is a
        // LeftJoin so the spine never reaches a substitutable BGP: not rewritten.
        let raw = parse(&format!(
            "{PFX} SELECT ?s WHERE {{ ?s ex:has ?p . OPTIONAL {{ ?z ?p ?w }} FILTER(?p = ex:target) }}"
        ));
        assert_eq!(rewrite_query(raw.clone()), raw, "must decline when ?p spans an opaque scope");
    }

    // ---- anti-join rewrite fires ----
    #[test]
    fn antijoin_fires() {
        let raw = parse(&format!(
            "{PFX} SELECT ?doc WHERE {{ ?doc a ex:Article . OPTIONAL {{ ?doc ex:ref ?bag }} FILTER(!bound(?bag)) }}"
        ));
        let rw = rewrite_query(raw.clone());
        let dbg = format!("{:?}", pattern_of(&rw));
        assert_ne!(rw, raw);
        assert!(dbg.contains("Minus"), "OPTIONAL+!bound must become Minus: {}", dbg);
        assert!(!dbg.contains("LeftJoin"), "LeftJoin must be gone: {}", dbg);
        assert!(!dbg.contains("Bound"), "the !bound FILTER must be gone: {}", dbg);
    }

    // ---- anti-join declines with no shared variable ----
    #[test]
    fn antijoin_declines_no_shared_var() {
        let raw = parse(&format!(
            "{PFX} SELECT ?doc WHERE {{ ?doc a ex:Article . OPTIONAL {{ ex:other ex:ref ?bag }} FILTER(!bound(?bag)) }}"
        ));
        assert_eq!(
            rewrite_query(raw.clone()),
            raw,
            "no shared variable between A and B → Minus is NOT equivalent → decline"
        );
    }

    #[test]
    fn large_disjoint_antijoin_stays_declined() {
        let predicate = NamedNode::new("http://ex/p").unwrap();
        let left = GraphPattern::Bgp {
            patterns: (0..1000)
                .map(|i| TriplePattern {
                    subject: TermPattern::Variable(Variable::new(format!("s{i}")).unwrap()),
                    predicate: NamedNodePattern::NamedNode(predicate.clone()),
                    object: TermPattern::Variable(Variable::new(format!("o{i}")).unwrap()),
                })
                .collect(),
        };
        let missing = Variable::new("missing").unwrap();
        let right = GraphPattern::Bgp {
            patterns: vec![TriplePattern {
                subject: TermPattern::Variable(Variable::new("right").unwrap()),
                predicate: NamedNodePattern::NamedNode(predicate),
                object: TermPattern::Variable(missing.clone()),
            }],
        };
        assert!(certain_vars(&right).contains(&missing));
        assert!(!pattern_contains_var(&left, &missing));
        assert!(!antijoin_ok(&missing, &left, &right));

        let raw = Query::Ask {
            dataset: None,
            pattern: GraphPattern::Filter {
                expr: Expression::Not(Box::new(Expression::Bound(missing))),
                inner: Box::new(GraphPattern::LeftJoin {
                    left: Box::new(left),
                    right: Box::new(right),
                    expression: None,
                }),
            },
            base_iri: None,
        };
        assert!(rewrite_query_ref(&raw).is_none());
        assert_eq!(rewrite_query(raw.clone()), raw);
    }

    // ---- REGRESSION (soundness, PR #1735 review finding #2): `certain_vars`'
    // Group arm must NOT treat an aggregate OUTPUT as certain. An aggregate can
    // error (e.g. SUM/AVG over a non-numeric term) and leave its output UNBOUND
    // while the group (with its GROUP BY key bound) still exists, so an aggregate
    // output is *not* bound in every solution. Before the fix the Group arm
    // inserted every aggregate output unconditionally — an over-approximation that
    // could make `antijoin_ok` read the output as certain-in-`right` and rewrite
    // `Filter(!bound(?agg), LeftJoin(A,B))` → `Minus(A,B)` unsoundly.
    //
    // This builds the `Group` node DIRECTLY (surface SPARQL `(SUM(?n) AS ?total)`
    // wraps the aggregate in an `Extend` that already guards `?total`, so it does
    // not exercise the Group arm — this constructs the raw shape the arm sees).
    // With the buggy loop present `?total` appears in the set and this fails;
    // with the fix it is absent.
    #[test]
    fn certain_vars_group_excludes_aggregate_output() {
        use spargebra::algebra::{AggregateExpression, AggregateFunction};
        use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
        let doc = Variable::new("doc").unwrap();
        let n = Variable::new("n").unwrap();
        let total = Variable::new("total").unwrap();
        // inner: `?doc ex:cites ?n`  (binds both ?doc and ?n)
        let inner = GraphPattern::Bgp {
            patterns: vec![TriplePattern {
                subject: TermPattern::Variable(doc.clone()),
                predicate: NamedNodePattern::NamedNode(NamedNode::new("http://ex/cites").unwrap()),
                object: TermPattern::Variable(n.clone()),
            }],
        };
        // GROUP BY ?doc with aggregate output ?total = SUM(?n).
        let group = GraphPattern::Group {
            inner: Box::new(inner),
            variables: vec![doc.clone()],
            aggregates: vec![(
                total.clone(),
                AggregateExpression::FunctionCall {
                    name: AggregateFunction::Sum,
                    expr: Expression::Variable(n.clone()),
                    distinct: false,
                },
            )],
        };
        let cv = certain_vars(&group);
        assert!(cv.contains(&doc), "GROUP BY key ?doc is certain");
        assert!(
            !cv.contains(&total),
            "aggregate output ?total must NOT be certain (aggregate errors leave it unbound)"
        );
    }

    // ---- End-to-end (defense-in-depth): the reviewer's scenario — a GROUP BY
    // sub-SELECT aggregate `?total` inside an OPTIONAL body, then FILTER(!bound) —
    // must leave the plan UNCHANGED (no anti-join). At the surface-SPARQL level
    // `?total` is bound by an `Extend` over the `Group` (spargebra desugaring), so
    // it is declined by the Extend rule regardless of the Group-arm fix; this
    // pins the observable end-to-end behaviour.
    #[test]
    fn antijoin_declines_over_optional_aggregate_subselect() {
        let raw = parse(&format!(
            "{PFX} SELECT ?doc WHERE {{ \
               ?doc a ex:Article . \
               OPTIONAL {{ SELECT ?doc (SUM(?n) AS ?total) WHERE {{ ?doc ex:cites ?n }} GROUP BY ?doc }} \
               FILTER(!bound(?total)) }}"
        ));
        // Sanity: the baseline algebra really has the anti-join candidate shape (a
        // LeftJoin whose right is a Project over a Group) — else the test is vacuous.
        let raw_dbg = format!("{:?}", pattern_of(&raw));
        assert!(raw_dbg.contains("LeftJoin"), "baseline must have a LeftJoin: {}", raw_dbg);
        assert!(raw_dbg.contains("Group"), "baseline OPTIONAL body must be a Group: {}", raw_dbg);
        assert_eq!(
            rewrite_query(raw.clone()),
            raw,
            "aggregate output is not certain → anti-join must NOT fire (plan unchanged)"
        );
    }

    // ---- certain_vars: BGP binds all its vars; LeftJoin only the left ----
    #[test]
    fn certain_vars_bgp_and_leftjoin() {
        let q_bgp = parse(&format!("{PFX} SELECT * WHERE {{ ?a ex:p ?b }}"));
        let cv = certain_vars(pattern_of(&q_bgp));
        assert!(cv.contains(&Variable::new("a").unwrap()));
        assert!(cv.contains(&Variable::new("b").unwrap()));

        let q_lj =
            parse(&format!("{PFX} SELECT * WHERE {{ ?a ex:p ?b OPTIONAL {{ ?a ex:q ?c }} }}"));
        let cv = certain_vars(pattern_of(&q_lj));
        assert!(cv.contains(&Variable::new("a").unwrap()));
        assert!(cv.contains(&Variable::new("b").unwrap()));
        assert!(!cv.contains(&Variable::new("c").unwrap()), "optional ?c is not certain");
    }

    #[test]
    fn nested_filter_rewrites_move_owned_subtrees() {
        use GraphPattern as G;

        let leaf = Box::new(G::Bgp {
            patterns: Vec::new(),
        });
        let leaf_allocation = std::ptr::from_ref(leaf.as_ref());
        let mut rewritten = G::Distinct { inner: leaf };
        let mut raw = rewritten.clone();
        for depth in 0..128 {
            let variable = Variable::new(format!("v{depth}")).unwrap();
            let iri = NamedNode::new("http://ex/target").unwrap();
            let expr = Expression::Equal(
                Box::new(Expression::Variable(variable.clone())),
                Box::new(Expression::NamedNode(iri.clone())),
            );
            let bgp = G::Bgp {
                patterns: vec![TriplePattern {
                    subject: TermPattern::NamedNode(NamedNode::new("http://ex/s").unwrap()),
                    predicate: NamedNodePattern::NamedNode(NamedNode::new("http://ex/p").unwrap()),
                    object: TermPattern::Variable(variable.clone()),
                }],
            };
            raw = G::Filter {
                expr: expr.clone(),
                inner: Box::new(G::Join {
                    left: Box::new(raw),
                    right: Box::new(bgp.clone()),
                }),
            };

            // The bottom-up traversal hands each FILTER an already rebuilt child.
            // Keep its Box identity across the next local substitution.
            let left = Box::new(rewritten);
            let allocation = std::ptr::from_ref(left.as_ref());
            rewritten = rewrite_filter(
                &expr,
                Cow::Owned(G::Join {
                    left,
                    right: Box::new(bgp),
                }),
            )
            .expect("the equality must be substituted");
            let G::Extend {
                inner,
                variable: rebound,
                expression,
            } = &rewritten
            else {
                panic!("the substituted variable must stay bound")
            };
            assert_eq!(rebound, &variable);
            assert_eq!(expression, &Expression::NamedNode(iri.clone()));
            let G::Join { left, right } = inner.as_ref() else {
                unreachable!()
            };
            assert_eq!(
                std::ptr::from_ref(left.as_ref()),
                allocation,
                "an already rebuilt subtree was cloned at layer {depth}"
            );
            let G::Bgp { patterns } = right.as_ref() else {
                unreachable!()
            };
            assert_eq!(patterns[0].object, TermPattern::NamedNode(iri));
        }

        // Check the innermost allocation too, and compare the entire chain with
        // the public borrowed entry point's output (not just the local helper).
        let mut current = &rewritten;
        while let G::Extend { inner, .. } = current {
            let G::Join { left, .. } = inner.as_ref() else {
                unreachable!()
            };
            current = left;
        }
        let G::Distinct { inner } = current else {
            unreachable!()
        };
        assert_eq!(std::ptr::from_ref(inner.as_ref()), leaf_allocation);
        let query = Query::Ask {
            dataset: None,
            pattern: raw,
            base_iri: None,
        };
        let output = rewrite_query_ref(&query).expect("nested filters must rewrite");
        assert_eq!(pattern_of(&output), &rewritten);
        assert!(rewrite_query_ref(&output).is_none());
    }

    #[test]
    fn antijoin_and_declined_filter_move_owned_children() {
        let query = parse(&format!(
            "{PFX} ASK {{ \
            ?s ex:p ?o OPTIONAL {{ ?s ex:q ?v }} FILTER(!bound(?v)) }}"
        ));
        let mut filter = pattern_of(&query);
        while let GraphPattern::Project { inner, .. } = filter {
            filter = inner;
        }
        let GraphPattern::Filter { expr, inner } = filter else {
            unreachable!()
        };
        let owned = inner.as_ref().clone();
        let GraphPattern::LeftJoin { left, right, .. } = &owned else {
            unreachable!()
        };
        let left_allocation = std::ptr::from_ref(left.as_ref());
        let right_allocation = std::ptr::from_ref(right.as_ref());
        let rewritten = rewrite_filter(expr, Cow::Owned(owned)).expect("anti-join must fire");
        let GraphPattern::Minus { left, right } = &rewritten else {
            unreachable!()
        };
        assert_eq!(std::ptr::from_ref(left.as_ref()), left_allocation);
        assert_eq!(std::ptr::from_ref(right.as_ref()), right_allocation);
        assert_eq!(rewrite_pattern(filter).as_ref(), Some(&rewritten));

        let expr = Expression::Literal(oxrdf::Literal::from(true));
        let rewritten = rewrite_filter(&expr, Cow::Owned(rewritten))
            .expect("a descendant rewrite must still be reported");
        let GraphPattern::Filter { inner, .. } = rewritten else {
            unreachable!()
        };
        let GraphPattern::Minus { left, right } = inner.as_ref() else {
            unreachable!()
        };
        assert_eq!(std::ptr::from_ref(left.as_ref()), left_allocation);
        assert_eq!(std::ptr::from_ref(right.as_ref()), right_allocation);
        assert!(rewrite_filter(&expr, Cow::Borrowed(&inner)).is_none());
    }

    #[test]
    fn change_report_propagates_both_passes_through_every_child() {
        use GraphPattern as G;
        let seeds = [
            "?s ?p ?v FILTER(?p = ex:target)",
            "?s a ex:Thing OPTIONAL { ?s ex:ref ?bag } FILTER(!bound(?bag))",
        ];
        for body in seeds {
            let Query::Ask { pattern: seed, .. } = parse(&format!("{PFX} ASK {{ {body} }}")) else {
                unreachable!()
            };
            let stable = G::Bgp {
                patterns: Vec::new(),
            };
            let boxed = || Box::new(seed.clone());
            let other = || Box::new(stable.clone());
            let variable = Variable::new("s").unwrap();
            let name = NamedNodePattern::NamedNode(NamedNode::new("http://ex/g").unwrap());
            let expression = Expression::Literal(oxrdf::Literal::from(true));
            let mut patterns = vec![
                ("root", seed.clone()),
                (
                    "filter",
                    G::Filter {
                        expr: expression.clone(),
                        inner: boxed(),
                    },
                ),
                (
                    "graph",
                    G::Graph {
                        name: name.clone(),
                        inner: boxed(),
                    },
                ),
                (
                    "extend",
                    G::Extend {
                        inner: boxed(),
                        variable: variable.clone(),
                        expression: expression.clone(),
                    },
                ),
                (
                    "order",
                    G::OrderBy {
                        inner: boxed(),
                        expression: Vec::new(),
                    },
                ),
                (
                    "project",
                    G::Project {
                        inner: boxed(),
                        variables: vec![variable],
                    },
                ),
                ("distinct", G::Distinct { inner: boxed() }),
                ("reduced", G::Reduced { inner: boxed() }),
                (
                    "slice",
                    G::Slice {
                        inner: boxed(),
                        start: 1,
                        length: Some(1),
                    },
                ),
                (
                    "group",
                    G::Group {
                        inner: boxed(),
                        variables: Vec::new(),
                        aggregates: Vec::new(),
                    },
                ),
                (
                    "service",
                    G::Service {
                        name,
                        inner: boxed(),
                        silent: true,
                    },
                ),
            ];
            for reverse in [false, true] {
                let pair = || {
                    if reverse {
                        (other(), boxed())
                    } else {
                        (boxed(), other())
                    }
                };
                let (left, right) = pair();
                patterns.push(("join", G::Join { left, right }));
                let (left, right) = pair();
                patterns.push((
                    "left join",
                    G::LeftJoin {
                        left,
                        right,
                        expression: Some(expression.clone()),
                    },
                ));
                let (left, right) = pair();
                patterns.push(("lateral", G::Lateral { left, right }));
                let (left, right) = pair();
                patterns.push(("union", G::Union { left, right }));
                let (left, right) = pair();
                patterns.push(("minus", G::Minus { left, right }));
            }
            for (label, pattern) in patterns {
                let queries = [
                    Query::Select {
                        dataset: None,
                        pattern: pattern.clone(),
                        base_iri: None,
                    },
                    Query::Ask {
                        dataset: None,
                        pattern: pattern.clone(),
                        base_iri: None,
                    },
                    Query::Construct {
                        template: Vec::new(),
                        dataset: None,
                        pattern: pattern.clone(),
                        base_iri: None,
                    },
                    Query::Describe {
                        dataset: None,
                        pattern,
                        base_iri: None,
                    },
                ];
                for raw in queries {
                    let rewritten = rewrite_query_ref(&raw)
                        .unwrap_or_else(|| panic!("lost {body} rewrite under {label}"));
                    assert_ne!(rewritten, raw, "{body} under {label}");
                    assert!(
                        rewrite_query_ref(&rewritten).is_none(),
                        "idempotence: {label}"
                    );
                }
            }
        }
    }

    #[test]
    fn declined_rewrites_report_none_and_owning_wrapper_keeps_input() {
        for body in [
            "?s ?p ?v",
            "?s ex:p ?v FILTER(?v > 7)",
            "?s ex:p ?v FILTER(?v = 1)",
            "?s ?p ?v FILTER(?p = ex:a && ?p = ex:b)",
            "?s ex:p ?v OPTIONAL { ?z ?v ?w } FILTER(?v = ex:a)",
            "?s ex:p ?v OPTIONAL { ex:other ex:q ?bag } FILTER(!bound(?bag))",
            "?s ex:p ?v FILTER EXISTS { ?x ?p ?o FILTER(?p = ex:a) }",
        ] {
            let raw = parse(&format!(
                "{PFX} CONSTRUCT {{ ?s ex:copy ?v }} WHERE {{ {body} }}"
            ));
            assert!(rewrite_query_ref(&raw).is_none(), "{body}");
            let Query::Construct { template, .. } = &raw else {
                unreachable!()
            };
            let allocation = template.as_ptr();
            let rewritten = rewrite_query(raw);
            let Query::Construct { template, .. } = &rewritten else {
                unreachable!()
            };
            assert_eq!(
                template.as_ptr(),
                allocation,
                "unchanged input was cloned: {body}"
            );
        }
    }

}
