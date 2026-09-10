//! [OPUS-4.8] (sq-7d3dj.30.1) Pre-execution SPARQL algebra rewrite pass.
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
use rustc_hash::FxHashSet;
use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::Query;

/// Rewrites a parsed query's graph pattern with the two result-equivalent
/// rewrites (see the module docs). Non-`WHERE` structure (dataset clause, base
/// IRI, CONSTRUCT template, projection) is preserved verbatim; only the
/// evaluated `pattern` is transformed. Idempotent-safe: re-running it on an
/// already-rewritten pattern is a no-op (no equality FILTER or `!bound` idiom
/// remains to consume).
pub fn rewrite_query(query: Query) -> Query {
    match query {
        Query::Select { dataset, pattern, base_iri } => {
            Query::Select { dataset, pattern: rewrite_pattern(pattern), base_iri }
        }
        Query::Ask { dataset, pattern, base_iri } => {
            Query::Ask { dataset, pattern: rewrite_pattern(pattern), base_iri }
        }
        Query::Construct { template, dataset, pattern, base_iri } => {
            Query::Construct { template, dataset, pattern: rewrite_pattern(pattern), base_iri }
        }
        Query::Describe { dataset, pattern, base_iri } => {
            Query::Describe { dataset, pattern: rewrite_pattern(pattern), base_iri }
        }
    }
}

/// Bottom-up transform: rewrite the children first, then apply the local
/// FILTER-node rewrites. Bottom-up so a nested equality FILTER is already folded
/// before an enclosing operator is considered.
fn rewrite_pattern(p: GraphPattern) -> GraphPattern {
    let p = map_children(p);
    match p {
        GraphPattern::Filter { expr, inner } => rewrite_filter(expr, *inner),
        other => other,
    }
}

/// Reconstructs `p` with [`rewrite_pattern`] applied to every child *pattern*.
/// Expressions (including `EXISTS` sub-patterns) are left untouched — the two
/// rewrites only read expressions, never rewrite inside them.
fn map_children(p: GraphPattern) -> GraphPattern {
    use GraphPattern as G;
    let r = |b: Box<G>| Box::new(rewrite_pattern(*b));
    match p {
        G::Bgp { .. } | G::Path { .. } | G::Values { .. } => p,
        G::Join { left, right } => G::Join { left: r(left), right: r(right) },
        G::LeftJoin { left, right, expression } => {
            G::LeftJoin { left: r(left), right: r(right), expression }
        }
        // `Lateral` (SEP-0006) is unconditionally present in the workspace build.
        G::Lateral { left, right } => G::Lateral { left: r(left), right: r(right) },
        G::Filter { expr, inner } => G::Filter { expr, inner: r(inner) },
        G::Union { left, right } => G::Union { left: r(left), right: r(right) },
        G::Graph { name, inner } => G::Graph { name, inner: r(inner) },
        G::Extend { inner, variable, expression } => {
            G::Extend { inner: r(inner), variable, expression }
        }
        G::Minus { left, right } => G::Minus { left: r(left), right: r(right) },
        G::OrderBy { inner, expression } => G::OrderBy { inner: r(inner), expression },
        G::Project { inner, variables } => G::Project { inner: r(inner), variables },
        G::Distinct { inner } => G::Distinct { inner: r(inner) },
        G::Reduced { inner } => G::Reduced { inner: r(inner) },
        G::Slice { inner, start, length } => G::Slice { inner: r(inner), start, length },
        G::Group { inner, variables, aggregates } => {
            G::Group { inner: r(inner), variables, aggregates }
        }
        G::Service { name, inner, silent } => G::Service { name, inner: r(inner), silent },
    }
}

/// Applies the local FILTER rewrites to `Filter(expr, inner)`. Tries the
/// anti-join rewrite first (it matches a whole-filter `!bound` shape), then the
/// equality substitution. Returns the original `Filter` node verbatim if neither
/// fires.
fn rewrite_filter(expr: Expression, inner: GraphPattern) -> GraphPattern {
    // (b) Anti-join: the ENTIRE filter is `!bound(?v)` over `LeftJoin(A, B)`.
    if let Some(v) = as_not_bound(&expr) {
        if let GraphPattern::LeftJoin { left, right, expression: None } = &inner {
            if antijoin_ok(&v, left, right) {
                return GraphPattern::Minus { left: left.clone(), right: right.clone() };
            }
        }
    }
    // (a) Equality substitution.
    equality_substitute(expr, inner)
}

/// `Some(v)` iff `e` is exactly `!BOUND(?v)` (`Not(Bound(v))`).
fn as_not_bound(e: &Expression) -> Option<Variable> {
    match e {
        Expression::Not(inner) => match inner.as_ref() {
            Expression::Bound(v) => Some(v.clone()),
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
    left_certain.iter().any(|w| right_certain.contains(w))
}

/// Equality-substitution rewrite over `Filter(expr, inner)`.
fn equality_substitute(expr: Expression, inner: GraphPattern) -> GraphPattern {
    let conjuncts = into_conjuncts(expr.clone());

    // Collect candidate IRI-equality conjuncts, keyed by variable. A variable
    // that appears in more than one equality conjunct is dropped (a possible
    // contradiction, e.g. `?v = <a> && ?v = <b>`), keeping those conjuncts as
    // residual — conservative.
    let mut candidate_iri: Vec<(Variable, NamedNode, usize)> = Vec::new();
    for (i, c) in conjuncts.iter().enumerate() {
        if let Some((var, iri)) = as_iri_equality(c) {
            candidate_iri.push((var, iri, i));
        }
    }
    // Which candidate variables are unique (appear in exactly one candidate).
    let mut applied: Vec<(Variable, NamedNode)> = Vec::new();
    let mut consumed: FxHashSet<usize> = FxHashSet::default();
    let mut inner = inner;
    for (var, iri, idx) in &candidate_iri {
        let dup = candidate_iri.iter().filter(|(v, ..)| v == var).count() > 1;
        if dup {
            continue;
        }
        // The variable must occur in a substitutable spine position and NOWHERE
        // inside an opaque scope boundary.
        let (in_spine, in_opaque) = scan_var(&inner, var);
        if in_spine && !in_opaque {
            apply_sub_spine(&mut inner, var, iri);
            applied.push((var.clone(), iri.clone()));
            consumed.insert(*idx);
        }
    }

    if applied.is_empty() {
        // Nothing fired — return the original FILTER verbatim (exact expr).
        return GraphPattern::Filter { expr, inner: Box::new(inner) };
    }

    // Re-bind each substituted variable to its constant so it stays in scope.
    for (var, iri) in applied {
        inner = GraphPattern::Extend {
            inner: Box::new(inner),
            variable: var,
            expression: Expression::NamedNode(iri),
        };
    }

    // Rebuild the residual FILTER from the non-consumed conjuncts (in order).
    let residual: Vec<Expression> =
        conjuncts.into_iter().enumerate().filter(|(i, _)| !consumed.contains(i)).map(|(_, c)| c).collect();
    match fold_and(residual) {
        Some(residual_expr) => GraphPattern::Filter { expr: residual_expr, inner: Box::new(inner) },
        None => inner,
    }
}

/// Flattens a top-level `&&` (`Expression::And`) tree into its conjuncts,
/// left-to-right. A non-`And` expression yields a single-element list.
fn into_conjuncts(e: Expression) -> Vec<Expression> {
    match e {
        Expression::And(a, b) => {
            let mut v = into_conjuncts(*a);
            v.extend(into_conjuncts(*b));
            v
        }
        other => vec![other],
    }
}

/// Left-associative `&&`-fold of a conjunct list (inverse of [`into_conjuncts`]).
/// `None` for an empty list (the FILTER is fully consumed).
fn fold_and(conjuncts: Vec<Expression>) -> Option<Expression> {
    let mut it = conjuncts.into_iter();
    let first = it.next()?;
    Some(it.fold(first, |acc, e| Expression::And(Box::new(acc), Box::new(e))))
}

/// `Some((v, iri))` iff `e` is `?v = <iri>` / `<iri> = ?v` / `sameTerm(?v, <iri>)`
/// / `sameTerm(<iri>, ?v)`. **IRI-only**: a `Literal` operand (numeric or plain)
/// returns `None` and is never rewritten (the `sq-lr2ii` avoidance contract).
fn as_iri_equality(e: &Expression) -> Option<(Variable, NamedNode)> {
    let (a, b) = match e {
        Expression::Equal(a, b) | Expression::SameTerm(a, b) => (a.as_ref(), b.as_ref()),
        _ => return None,
    };
    match (a, b) {
        (Expression::Variable(v), Expression::NamedNode(n))
        | (Expression::NamedNode(n), Expression::Variable(v)) => Some((v.clone(), n.clone())),
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
        GraphPattern::Path { subject, object, .. } => {
            if term_has_var(subject, var) || term_has_var(object, var) {
                *in_spine = true;
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
fn certain_vars(p: &GraphPattern) -> FxHashSet<Variable> {
    let mut s = FxHashSet::default();
    match p {
        GraphPattern::Bgp { patterns } => {
            for t in patterns {
                collect_triple_vars(t, &mut s);
            }
        }
        GraphPattern::Path { subject, object, .. } => {
            collect_term_vars(subject, &mut s);
            collect_term_vars(object, &mut s);
        }
        GraphPattern::Join { left, right } | GraphPattern::Lateral { left, right } => {
            s = certain_vars(left);
            s.extend(certain_vars(right));
        }
        GraphPattern::LeftJoin { left, .. } | GraphPattern::Minus { left, .. } => {
            s = certain_vars(left);
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. } => s = certain_vars(inner),
        GraphPattern::Union { left, right } => {
            let l = certain_vars(left);
            let r = certain_vars(right);
            s = l.intersection(&r).cloned().collect();
        }
        GraphPattern::Graph { name, inner } => {
            s = certain_vars(inner);
            if let NamedNodePattern::Variable(v) = name {
                s.insert(v.clone());
            }
        }
        // Extend's variable may be left unbound on an evaluation error, so it is
        // NOT certain — conservatively use only the inner certain set.
        GraphPattern::Extend { inner, .. } => s = certain_vars(inner),
        GraphPattern::Project { inner, variables } => {
            let c = certain_vars(inner);
            s = variables.iter().filter(|v| c.contains(v)).cloned().collect();
        }
        GraphPattern::Values { variables, bindings } => {
            for (i, v) in variables.iter().enumerate() {
                if bindings.iter().all(|row| row.get(i).is_some_and(|cell| cell.is_some())) {
                    s.insert(v.clone());
                }
            }
        }
        GraphPattern::Group { inner, variables, aggregates: _ } => {
            // GROUP BY keys that are certain in `inner` stay certain. Aggregate
            // OUTPUTS are intentionally EXCLUDED: an aggregate (e.g. SUM/AVG over a
            // non-numeric term) can error and leave its output variable UNBOUND, so
            // it is not bound in every solution. Including it would over-approximate
            // and could let the anti-join guard fire unsoundly on a
            // `Filter(!bound(?agg), LeftJoin(A, B))` where `?agg` is an aggregate
            // output inside `B`. Omitting it is a strictly-safe under-approximation.
            // [OPUS-4.8]
            let c = certain_vars(inner);
            for v in variables {
                if c.contains(v) {
                    s.insert(v.clone());
                }
            }
        }
        // A `SILENT` SERVICE may yield the unit solution (nothing certain); a
        // non-silent one binds its inner certain set (best-effort, static view).
        GraphPattern::Service { inner, silent, .. } => {
            if !silent {
                s = certain_vars(inner);
            }
        }
    }
    s
}

fn collect_triple_vars(t: &TriplePattern, out: &mut FxHashSet<Variable>) {
    collect_term_vars(&t.subject, out);
    if let NamedNodePattern::Variable(v) = &t.predicate {
        out.insert(v.clone());
    }
    collect_term_vars(&t.object, out);
}

fn collect_term_vars(tp: &TermPattern, out: &mut FxHashSet<Variable>) {
    match tp {
        TermPattern::Variable(v) => {
            out.insert(v.clone());
        }
        TermPattern::Triple(t) => collect_triple_vars(t, out),
        _ => {}
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

    // ---- into_conjuncts / fold_and round trip ----
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
        let cs = into_conjuncts(e.clone());
        assert_eq!(cs.len(), 3, "three top-level && conjuncts");
        assert_eq!(cs[0], a);
        assert_eq!(cs[1], b);
        assert_eq!(cs[2], c);
        assert_eq!(fold_and(cs), Some(e), "fold_and is the left-assoc inverse of into_conjuncts");
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
    // with the fix it is absent. [OPUS-4.8]
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
    // pins the observable end-to-end behaviour. [OPUS-4.8]
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
}
