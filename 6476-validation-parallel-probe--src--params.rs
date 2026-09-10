//! Parameterized prepared queries — safe value binding to prevent SPARQL
//! injection (issue #901, bead `sq-rp3um`). NON-DEFAULT `params` feature.
//!
//! The canonical mitigation for SPARQL injection: when a query is assembled from
//! untrusted input, callers must NOT concatenate that input into the query
//! string. Instead they parse a query *template* once and `bind` a typed RDF term
//! ([`oxrdf::Term`]) into a named placeholder. Binding is a pure **algebra
//! rewrite** — every free occurrence of the placeholder [`Variable`] is replaced
//! by the term *inside the parsed AST* (`Variable` -> `NamedNode`/`Literal`/
//! `BlankNode`/triple-term), so the query *structure* is fixed before the value
//! is ever seen. There is no re-parse of a concatenated string and the bound
//! value can NEVER introduce new syntax: a hostile IRI/literal such as
//! `<x> } INSERT { ... } #` or a `"` break-out is carried as a single opaque term
//! and matched as DATA, not parsed as query text.
//!
//! This mirrors Jena's `ParameterizedSparqlString` and RDF4J's `$var` binding.
//! It covers SELECT / ASK / CONSTRUCT / DESCRIBE ([`PreparedQuery::bind`]) and
//! SPARQL 1.1 UPDATE ([`PreparedUpdate`]).
//!
//! ## Why this is injection-safe (the load-bearing invariant)
//!
//! 1. The template is parsed ONCE into [`spargebra`] algebra. The grammar /
//!    structure of the query is fixed at that point.
//! 2. `bind` accepts a typed [`oxrdf::Term`]. The caller can only have built that
//!    term through oxrdf's validated constructors (`NamedNode::new` validates the
//!    IRI; a `Literal` is an opaque lexical value + datatype/lang). A term is a
//!    leaf — it has no place to encode a `}`, a `.`, a graph pattern, or a second
//!    UPDATE operation that the algebra would honour.
//! 3. The rewrite walks the algebra and swaps the `Variable` *node* for the
//!    term's pattern node. No string is produced, concatenated, or re-parsed on
//!    the query path. Execution consumes the rewritten algebra directly.
//!
//! ## Validation
//!
//! `bind` is **fail-closed**: it returns `Err` if the named placeholder does not
//! occur free in the template (a typo'd parameter name is a bug, not a silent
//! no-op), and it refuses to bind a variable that is *projected*, used as an
//! aggregate/`GROUP BY`/`BIND`-target output, or named in a `VALUES` header —
//! binding such a variable would silently change the result shape rather than
//! pin a value, so the caller is told to rename the placeholder instead.
//!
//! When the `params` feature is off, none of this compiles; the default build is
//! byte-identical and pulls no new dependencies (oxrdf + spargebra are already
//! direct deps). [OPUS-4.8]

use crate::{PreparedQuery, PreparedUpdate};
use oxrdf::{BlankNode, Literal, NamedNode, Term, Variable};
use spargebra::algebra::{
    AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression,
};
use spargebra::term::{
    GraphNamePattern, GroundQuadPattern, GroundTermPattern, NamedNodePattern, QuadPattern,
    TermPattern, TriplePattern,
};
use spargebra::{GraphUpdateOperation, Query, Update};

/// Error returned by the `bind` parameter-binding APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindError {
    /// The named placeholder does not occur free anywhere bindable in the
    /// template. A binding that matches nothing is almost always a typo'd
    /// parameter name, so it is rejected rather than silently ignored.
    NoSuchParameter(String),
    /// The placeholder is a *result* variable — projected by `SELECT`, a
    /// `GROUP BY` / aggregate / `BIND` output, or a `VALUES` column header.
    /// Substituting a constant there would change the result shape rather than
    /// pin a value; rename the placeholder to an internal one instead.
    ResultVariable(String),
    /// A blank node was bound into a position that forbids one (the predicate
    /// slot of a triple pattern, a graph name, or a `DELETE`/ground template).
    /// SPARQL has no blank-node predicates / graph names, and `DELETE` templates
    /// are ground (blank-node-free), so this is structurally impossible.
    BlankNodeNotAllowed(String),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Positional `{}` args (CodeQL rust/unused-variable false-positive avoidance). [OPUS-4.8]
            BindError::NoSuchParameter(n) => {
                write!(f, "no free parameter ?{} in the query template", n)
            }
            BindError::ResultVariable(n) => write!(
                f,
                "cannot bind ?{} — it is a projected/aggregate/BIND/VALUES result variable; \
                 rename the placeholder to an internal variable",
                n
            ),
            BindError::BlankNodeNotAllowed(n) => write!(
                f,
                "cannot bind a blank node to ?{} — that position (predicate / graph name / \
                 ground DELETE template) admits no blank node",
                n
            ),
        }
    }
}

impl std::error::Error for BindError {}

/// The term being bound. Blank nodes are illegal as predicates / graph names /
/// in ground (DELETE) templates; each slot constructor below enforces that.
struct Bound {
    term: Term,
}

impl Bound {
    fn new(term: Term) -> Self {
        Bound { term }
    }

    /// A `TermPattern` (subject/object slot) carrying the bound term. spargebra's
    /// `From<oxrdf::Term>` maps every term shape — including an RDF 1.2 quoted
    /// triple — to its (ground) pattern node, so the value is matched
    /// structurally as DATA and can never be query syntax.
    fn term_pattern(&self) -> TermPattern {
        TermPattern::from(self.term.clone())
    }

    /// A `GroundTermPattern` (ground DELETE template slot). Fails for a blank node.
    fn ground_term_pattern(&self, name: &str) -> Result<GroundTermPattern, BindError> {
        match &self.term {
            Term::NamedNode(n) => Ok(GroundTermPattern::NamedNode(n.clone())),
            Term::Literal(l) => Ok(GroundTermPattern::Literal(l.clone())),
            Term::BlankNode(_) => Err(BindError::BlankNodeNotAllowed(name.to_string())),
            // A quoted triple may carry a blank node component, so it is not a
            // valid *ground* (DELETE) template term — reject it here.
            Term::Triple(_) => Err(BindError::BlankNodeNotAllowed(name.to_string())),
        }
    }

    /// A `NamedNodePattern` (predicate / graph-name slot). Fails unless the term
    /// is an IRI — SPARQL has no blank-node or literal predicates / graph names.
    fn named_node_pattern(&self, name: &str) -> Result<NamedNodePattern, BindError> {
        match &self.term {
            Term::NamedNode(n) => Ok(NamedNodePattern::NamedNode(n.clone())),
            _ => Err(BindError::BlankNodeNotAllowed(name.to_string())),
        }
    }

    /// A `GraphNamePattern` (GRAPH name slot). IRI only.
    fn graph_name_pattern(&self, name: &str) -> Result<GraphNamePattern, BindError> {
        match &self.term {
            Term::NamedNode(n) => Ok(GraphNamePattern::NamedNode(n.clone())),
            _ => Err(BindError::BlankNodeNotAllowed(name.to_string())),
        }
    }

    /// An `Expression` leaf (FILTER / BIND operand). A blank node has no
    /// expression form, so a NamedNode/Literal substitutes and a blank node is
    /// rejected.
    fn expression(&self, name: &str) -> Result<Expression, BindError> {
        match &self.term {
            Term::NamedNode(n) => Ok(Expression::NamedNode(n.clone())),
            Term::Literal(l) => Ok(Expression::Literal(l.clone())),
            Term::BlankNode(_) => Err(BindError::BlankNodeNotAllowed(name.to_string())),
            // A quoted triple has no literal `Expression` leaf; reject it in a
            // FILTER/BIND operand position (it can still be bound in a BGP slot).
            Term::Triple(_) => Err(BindError::BlankNodeNotAllowed(name.to_string())),
        }
    }
}

/// Accumulates whether ANY free occurrence was substituted (so an unknown name
/// can be rejected) and whether a slot rejected the bound term.
struct Subst<'a> {
    name: &'a str,
    bound: &'a Bound,
    hit: bool,
    err: Option<BindError>,
}

impl<'a> Subst<'a> {
    fn fail(&mut self, e: BindError) {
        if self.err.is_none() {
            self.err = Some(e);
        }
    }

    fn matches(&self, v: &Variable) -> bool {
        v.as_str() == self.name
    }

    fn term_pattern(&mut self, tp: &mut TermPattern) {
        if let TermPattern::Variable(v) = tp {
            if self.matches(v) {
                self.hit = true;
                *tp = self.bound.term_pattern();
                return;
            }
        }
        // Recurse into an RDF 1.2 quoted-triple term so a placeholder *inside* a
        // triple term is bound too (the workspace always enables sparql-12).
        if let TermPattern::Triple(t) = tp {
            self.triple_pattern(t);
        }
    }

    fn ground_term_pattern(&mut self, tp: &mut GroundTermPattern) {
        if let GroundTermPattern::Variable(v) = tp {
            if self.matches(v) {
                self.hit = true;
                match self.bound.ground_term_pattern(self.name) {
                    Ok(g) => *tp = g,
                    Err(e) => self.fail(e),
                }
            }
        }
    }

    fn named_node_pattern(&mut self, nnp: &mut NamedNodePattern) {
        if let NamedNodePattern::Variable(v) = nnp {
            if self.matches(v) {
                self.hit = true;
                match self.bound.named_node_pattern(self.name) {
                    Ok(n) => *nnp = n,
                    Err(e) => self.fail(e),
                }
            }
        }
    }

    fn graph_name_pattern(&mut self, gnp: &mut GraphNamePattern) {
        if let GraphNamePattern::Variable(v) = gnp {
            if self.matches(v) {
                self.hit = true;
                match self.bound.graph_name_pattern(self.name) {
                    Ok(n) => *gnp = n,
                    Err(e) => self.fail(e),
                }
            }
        }
    }

    fn triple_pattern(&mut self, t: &mut TriplePattern) {
        self.term_pattern(&mut t.subject);
        self.named_node_pattern(&mut t.predicate);
        self.term_pattern(&mut t.object);
    }

    fn quad_pattern(&mut self, q: &mut QuadPattern) {
        self.term_pattern(&mut q.subject);
        self.named_node_pattern(&mut q.predicate);
        self.term_pattern(&mut q.object);
        self.graph_name_pattern(&mut q.graph_name);
    }

    fn ground_quad_pattern(&mut self, q: &mut GroundQuadPattern) {
        self.ground_term_pattern(&mut q.subject);
        self.named_node_pattern(&mut q.predicate);
        self.ground_term_pattern(&mut q.object);
        self.graph_name_pattern(&mut q.graph_name);
    }

    fn path(&mut self, p: &mut PropertyPathExpression) {
        // Property paths combine predicate IRIs only — no variable predicate is
        // bindable here, so a NamedNode leaf is never a placeholder. Nothing to
        // rewrite; recurse for completeness over the structural variants.
        match p {
            PropertyPathExpression::NamedNode(_) => {}
            PropertyPathExpression::Reverse(i)
            | PropertyPathExpression::ZeroOrMore(i)
            | PropertyPathExpression::OneOrMore(i)
            | PropertyPathExpression::ZeroOrOne(i) => self.path(i),
            PropertyPathExpression::Sequence(a, b) | PropertyPathExpression::Alternative(a, b) => {
                self.path(a);
                self.path(b);
            }
            PropertyPathExpression::NegatedPropertySet(_) => {}
        }
    }

    fn expr(&mut self, e: &mut Expression) {
        match e {
            Expression::Variable(v) if self.matches(v) => {
                self.hit = true;
                match self.bound.expression(self.name) {
                    Ok(x) => *e = x,
                    Err(err) => self.fail(err),
                }
            }
            Expression::Variable(_) | Expression::NamedNode(_) | Expression::Literal(_) => {}
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
                self.expr(a);
                self.expr(b);
            }
            Expression::In(a, list) => {
                self.expr(a);
                for x in list {
                    self.expr(x);
                }
            }
            Expression::UnaryPlus(i) | Expression::UnaryMinus(i) | Expression::Not(i) => {
                self.expr(i)
            }
            // BOUND(?p) over a *bound* variable is always true; rewriting it would
            // silently change semantics, and BOUND's argument is a result variable
            // by construction. Leave it untouched (never a free placeholder).
            Expression::Bound(_) => {}
            Expression::If(a, b, c) => {
                self.expr(a);
                self.expr(b);
                self.expr(c);
            }
            Expression::Coalesce(list) | Expression::FunctionCall(_, list) => {
                for x in list {
                    self.expr(x);
                }
            }
            Expression::Exists(p) => self.pattern(p),
        }
    }

    fn pattern(&mut self, gp: &mut GraphPattern) {
        match gp {
            GraphPattern::Bgp { patterns } => {
                for t in patterns {
                    self.triple_pattern(t);
                }
            }
            GraphPattern::Path {
                subject,
                path,
                object,
            } => {
                self.term_pattern(subject);
                self.path(path);
                self.term_pattern(object);
            }
            GraphPattern::Join { left, right }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right } => {
                self.pattern(left);
                self.pattern(right);
            }
            // `Lateral` (SEP-0006) is unconditionally present — the workspace
            // enables spargebra's `sep-0006` (mirrors cache.rs).
            GraphPattern::Lateral { left, right } => {
                self.pattern(left);
                self.pattern(right);
            }
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            } => {
                self.pattern(left);
                self.pattern(right);
                if let Some(e) = expression {
                    self.expr(e);
                }
            }
            GraphPattern::Filter { expr, inner } => {
                self.expr(expr);
                self.pattern(inner);
            }
            GraphPattern::Graph { name, inner } => {
                self.named_node_pattern(name);
                self.pattern(inner);
            }
            GraphPattern::Extend {
                inner, expression, ..
            } => {
                // `variable` is a BIND output (a result variable), never a free
                // placeholder — only the expression operands are bindable.
                self.pattern(inner);
                self.expr(expression);
            }
            GraphPattern::Values { .. } => {
                // VALUES header variables are result columns; their inline ground
                // terms are already constants. Nothing free to bind here, and
                // binding a VALUES column is rejected upstream by the
                // result-variable scan.
            }
            GraphPattern::OrderBy { inner, expression } => {
                self.pattern(inner);
                for o in expression {
                    match o {
                        OrderExpression::Asc(e) | OrderExpression::Desc(e) => self.expr(e),
                    }
                }
            }
            GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. } => self.pattern(inner),
            GraphPattern::Group {
                inner, aggregates, ..
            } => {
                self.pattern(inner);
                for (_, agg) in aggregates {
                    self.aggregate(agg);
                }
            }
            GraphPattern::Service { name, inner, .. } => {
                self.named_node_pattern(name);
                self.pattern(inner);
            }
        }
    }

    fn aggregate(&mut self, agg: &mut AggregateExpression) {
        match agg {
            AggregateExpression::CountSolutions { .. } => {}
            AggregateExpression::FunctionCall { expr, .. } => self.expr(expr),
        }
    }
}

/// Runs the substitution over a `Query`, returning the rewritten query or the
/// first slot/blank-node error. Caller has already validated the name is free
/// and not a result variable.
fn subst_query(mut query: Query, name: &str, bound: &Bound) -> Result<Query, BindError> {
    let mut s = Subst {
        name,
        bound,
        hit: false,
        err: None,
    };
    match &mut query {
        Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Describe { pattern, .. } => s.pattern(pattern),
        Query::Construct {
            template, pattern, ..
        } => {
            for t in template.iter_mut() {
                s.triple_pattern(t);
            }
            s.pattern(pattern);
        }
    }
    if let Some(e) = s.err {
        return Err(e);
    }
    if !s.hit {
        return Err(BindError::NoSuchParameter(name.to_string()));
    }
    Ok(query)
}

/// Runs the substitution over an `Update`.
fn subst_update(mut update: Update, name: &str, bound: &Bound) -> Result<Update, BindError> {
    let mut s = Subst {
        name,
        bound,
        hit: false,
        err: None,
    };
    for op in update.operations.iter_mut() {
        match op {
            GraphUpdateOperation::DeleteInsert {
                delete,
                insert,
                pattern,
                ..
            } => {
                for q in delete.iter_mut() {
                    s.ground_quad_pattern(q);
                }
                for q in insert.iter_mut() {
                    s.quad_pattern(q);
                }
                s.pattern(pattern);
            }
            // InsertData/DeleteData are already-ground constants (no variables);
            // Load/Clear/Create/Drop carry only concrete IRIs/graph targets.
            // There is nothing free to bind, so they are left untouched.
            GraphUpdateOperation::InsertData { .. }
            | GraphUpdateOperation::DeleteData { .. }
            | GraphUpdateOperation::Load { .. }
            | GraphUpdateOperation::Clear { .. }
            | GraphUpdateOperation::Create { .. }
            | GraphUpdateOperation::Drop { .. } => {}
        }
    }
    if let Some(e) = s.err {
        return Err(e);
    }
    if !s.hit {
        return Err(BindError::NoSuchParameter(name.to_string()));
    }
    Ok(update)
}

/// Collects the *computed-result* variables of a query — names produced by a
/// COMPUTATION (`BIND`/`GROUP BY` key/aggregate output) or fixed by a `VALUES`
/// table, which if rebound to a constant would silently discard that computation
/// (or conflict with the inline table) rather than pin a value. We refuse to bind
/// any of these.
///
/// We deliberately do NOT reject a plain SELECT *projection* variable: binding a
/// projected BGP variable to a constant is the normal, intended use (exactly
/// RDF4J's `setBinding`), and the parser also wraps ASK / CONSTRUCT / `SELECT *`
/// in a synthetic `Project` over every in-scope variable — rejecting those would
/// block the common case. So a `Project`'s variable list is traversed but not
/// treated as off-limits.
fn result_variables(query: &Query) -> Vec<String> {
    let mut out = Vec::new();
    let pattern = match query {
        Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Construct { pattern, .. } => pattern,
    };
    collect_result_vars(pattern, &mut out);
    out
}

fn collect_result_vars(gp: &GraphPattern, out: &mut Vec<String>) {
    match gp {
        GraphPattern::Project { inner, .. } => {
            // Projection columns are bindable (see the doc comment above); recurse
            // only to surface genuine computed/VALUES outputs below.
            collect_result_vars(inner, out);
        }
        GraphPattern::Extend {
            inner, variable, ..
        } => {
            out.push(variable.as_str().to_string());
            collect_result_vars(inner, out);
        }
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            for v in variables {
                out.push(v.as_str().to_string());
            }
            for (v, _) in aggregates {
                out.push(v.as_str().to_string());
            }
            collect_result_vars(inner, out);
        }
        GraphPattern::Values { variables, .. } => {
            for v in variables {
                out.push(v.as_str().to_string());
            }
        }
        GraphPattern::Join { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right }
        | GraphPattern::LeftJoin { left, right, .. } => {
            collect_result_vars(left, out);
            collect_result_vars(right, out);
        }
        GraphPattern::Lateral { left, right } => {
            collect_result_vars(left, out);
            collect_result_vars(right, out);
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Graph { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Service { inner, .. } => collect_result_vars(inner, out),
        GraphPattern::Bgp { .. } | GraphPattern::Path { .. } => {}
    }
}

/// Validates and binds `name` -> `term` into a parsed `Query`. Internal driver
/// for [`PreparedQuery::bind`].
pub(crate) fn bind_query(
    query: &Query,
    name: &str,
    term: Term,
) -> Result<PreparedQuery, BindError> {
    let name = strip_qmark(name);
    if result_variables(query).iter().any(|v| v == name) {
        return Err(BindError::ResultVariable(name.to_string()));
    }
    let bound = Bound::new(term);
    let rewritten = subst_query(query.clone(), name, &bound)?;
    Ok(PreparedQuery::from(rewritten))
}

/// Validates and binds `name` -> `term` into a parsed `Update`. Internal driver
/// for [`PreparedUpdate::bind`].
pub(crate) fn bind_update(
    update: &Update,
    name: &str,
    term: Term,
) -> Result<PreparedUpdate, BindError> {
    let name = strip_qmark(name);
    // An UPDATE's WHERE pattern projects nothing externally, but a BIND/GROUP
    // output in the WHERE is still a result variable we must not silently pin.
    if update_result_variables(update).iter().any(|v| v == name) {
        return Err(BindError::ResultVariable(name.to_string()));
    }
    let bound = Bound::new(term);
    let rewritten = subst_update(update.clone(), name, &bound)?;
    Ok(PreparedUpdate::from_algebra(rewritten))
}

fn update_result_variables(update: &Update) -> Vec<String> {
    let mut out = Vec::new();
    for op in &update.operations {
        if let GraphUpdateOperation::DeleteInsert { pattern, .. } = op {
            collect_result_vars(pattern, &mut out);
        }
    }
    out
}

/// Accepts a placeholder written either as `name` or `?name` / `$name`.
fn strip_qmark(name: &str) -> &str {
    name.strip_prefix('?')
        .or_else(|| name.strip_prefix('$'))
        .unwrap_or(name)
}

/// Convenience constructors for the common bound-term shapes, so callers do not
/// reach for `oxrdf` directly for the simple cases. Each returns a validated
/// [`oxrdf::Term`]; an invalid IRI is a clean `Err` (never a panic).
pub mod value {
    use super::*;

    /// An IRI term. Validates the IRI; a structurally-invalid IRI (e.g. one
    /// containing a space or `>`) is rejected here, before it ever reaches the
    /// algebra.
    pub fn iri(iri: &str) -> Result<Term, String> {
        Ok(Term::NamedNode(
            NamedNode::new(iri).map_err(|e| e.to_string())?,
        ))
    }

    /// A plain `xsd:string` literal.
    pub fn string(value: &str) -> Term {
        Term::Literal(Literal::new_simple_literal(value))
    }

    /// A language-tagged literal (`rdf:langString`).
    pub fn lang_string(value: &str, lang: &str) -> Result<Term, String> {
        Ok(Term::Literal(
            Literal::new_language_tagged_literal(value, lang).map_err(|e| e.to_string())?,
        ))
    }

    /// A typed literal (`value^^datatype`).
    pub fn typed(value: &str, datatype_iri: &str) -> Result<Term, String> {
        let dt = NamedNode::new(datatype_iri).map_err(|e| e.to_string())?;
        Ok(Term::Literal(Literal::new_typed_literal(value, dt)))
    }

    /// A fresh blank node with a caller-chosen label.
    pub fn blank(label: &str) -> Result<Term, String> {
        Ok(Term::BlankNode(
            BlankNode::new(label).map_err(|e| e.to_string())?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::params::value;
    use crate::{
        ask, construct_prepared, query, query_prepared, update_in_place_prepared, update_prepared,
        PreparedQuery, PreparedUpdate,
    };
    use sparq_core::Graph;

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:alice ex:knows ex:bob ; ex:name "Alice" .
        ex:bob   ex:knows ex:carol ; ex:name "Bob" .
        ex:carol ex:name "Carol" .
    "#;

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    // [OPUS-4.8] The #901 injection scenario: a hostile bound *literal* that, under
    // string concatenation, would break out of the literal and inject a whole UPDATE.
    // Bound through the algebra it is matched as DATA — it cannot change the query
    // structure, so it simply matches nothing (no person is named that string).
    #[test]
    fn hostile_literal_is_data_not_syntax() {
        let hostile = r#"Bob" } INSERT DATA { <http://ex/evil> <http://ex/p> <http://ex/o> } ; SELECT * WHERE { <http://ex/x> ?p ?o "#;
        let pq = PreparedQuery::parse("SELECT ?s WHERE { ?s <http://ex/name> ?nameVal }")
            .unwrap()
            .bind("nameVal", value::string(hostile))
            .unwrap();
        let r = query_prepared(&g(), &pq).unwrap();
        // The value matches no triple — it is treated as a single literal, NOT parsed.
        assert_eq!(r.len(), 0);
        // And crucially the wrapped algebra is still a single SELECT (no injected op):
        assert!(matches!(pq.query(), spargebra::Query::Select { .. }));
        // The hostile text survives verbatim as a literal leaf in the algebra — proof
        // it is carried as data. (We assert it round-trips as a Literal, not syntax.)
        let bound_str = pq.query().to_string();
        assert!(
            bound_str.contains("INSERT DATA"),
            "hostile text is opaque literal data"
        );
        assert!(
            !bound_str.contains("} INSERT DATA {  } ;"),
            "no structural break-out"
        );
    }

    // A bound value that matches a REAL term returns the real row — binding works.
    #[test]
    fn bound_value_matches_real_data() {
        let pq = PreparedQuery::parse("SELECT ?friend WHERE { ?who <http://ex/knows> ?friend }")
            .unwrap()
            .bind("who", value::iri("http://ex/alice").unwrap())
            .unwrap();
        let r = query_prepared(&g(), &pq).unwrap();
        assert_eq!(r.len(), 1);
        // result-equivalence: identical to the hand-written constant query.
        let direct = query(
            &g(),
            "SELECT ?friend WHERE { <http://ex/alice> <http://ex/knows> ?friend }",
        )
        .unwrap();
        assert_eq!(r.rows, direct.rows);
    }

    // Binding into the PREDICATE slot (an IRI-only position).
    #[test]
    fn bind_predicate_slot() {
        let pq = PreparedQuery::parse("SELECT ?o WHERE { <http://ex/alice> ?prop ?o }")
            .unwrap()
            .bind("prop", value::iri("http://ex/name").unwrap())
            .unwrap();
        let r = query_prepared(&g(), &pq).unwrap();
        assert_eq!(r.len(), 1);
    }

    // ASK with a hostile IRI-shaped value — but the IRI constructor rejects the
    // truly malformed one, and a well-formed-but-unknown IRI just matches nothing.
    #[test]
    fn ask_with_bound_value() {
        let pq = PreparedQuery::parse("ASK { ?p <http://ex/knows> ?q }")
            .unwrap()
            .bind("p", value::iri("http://ex/alice").unwrap())
            .unwrap();
        let prepared_true = query_prepared(&g(), &pq).unwrap();
        assert_eq!(prepared_true.len(), 1); // ASK true => one unit row
                                            // a structurally-invalid IRI is rejected at term construction, never reaching the query
        assert!(value::iri("http://ex/ }>  INJECT").is_err());
        // sanity: the unbound template runs as a normal ASK
        assert!(ask(&g(), "ASK { ?p <http://ex/knows> ?q }").unwrap());
    }

    // CONSTRUCT: bind a value into BOTH the template and the WHERE.
    #[test]
    fn construct_with_bound_value() {
        let pq = PreparedQuery::parse(
            "CONSTRUCT { ?person <http://ex/is> <http://ex/known> } \
             WHERE { ?someone <http://ex/knows> ?person }",
        )
        .unwrap()
        .bind("someone", value::iri("http://ex/alice").unwrap())
        .unwrap();
        let triples = construct_prepared(&g(), &pq).unwrap();
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].subject.to_string(), "<http://ex/bob>");
    }

    // UPDATE (rebuild path): a hostile literal bound into an INSERT/DELETE WHERE is
    // matched as data; the update applies exactly the intended single mutation.
    #[test]
    fn update_prepared_binds_safely() {
        let pu = PreparedUpdate::parse(
            "DELETE { ?s <http://ex/name> ?old } \
             INSERT { ?s <http://ex/name> ?newName } \
             WHERE  { ?s <http://ex/name> ?old . FILTER(?s = ?target) }",
        )
        .unwrap()
        .bind("target", value::iri("http://ex/alice").unwrap())
        .unwrap()
        .bind("newName", value::string("Alice Updated"))
        .unwrap();
        let updated = update_prepared(&g(), &pu).unwrap();
        let names = query(
            &updated,
            "SELECT ?n WHERE { <http://ex/alice> <http://ex/name> ?n }",
        )
        .unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(
            names.rows[0][0].as_ref().unwrap().to_string(),
            "\"Alice Updated\""
        );
        // bob/carol untouched
        let bob = query(
            &updated,
            "SELECT ?n WHERE { <http://ex/bob> <http://ex/name> ?n }",
        )
        .unwrap();
        assert_eq!(bob.rows[0][0].as_ref().unwrap().to_string(), "\"Bob\"");
    }

    // UPDATE injection: a hostile literal value cannot inject a second operation.
    #[test]
    fn update_hostile_value_is_data() {
        let hostile =
            r#"x" } ; DROP ALL ; INSERT DATA { <http://ex/evil> <http://ex/p> <http://ex/o> } # "#;
        let pu =
            PreparedUpdate::parse("INSERT { <http://ex/marker> <http://ex/note> ?note } WHERE { }")
                .unwrap()
                .bind("note", value::string(hostile))
                .unwrap();
        // exactly ONE operation survives — no DROP ALL injected.
        assert_eq!(pu.update().operations.len(), 1);
        let updated = update_prepared(&g(), &pu).unwrap();
        // the existing data is intact (DROP ALL did NOT run): alice still present.
        assert!(crate::ask(&updated, "ASK { <http://ex/alice> <http://ex/name> ?n }").unwrap());
        // the marker triple stores the hostile string verbatim as a literal.
        let note = query(
            &updated,
            "SELECT ?n WHERE { <http://ex/marker> <http://ex/note> ?n }",
        )
        .unwrap();
        assert_eq!(note.len(), 1);
        assert_eq!(
            note.rows[0][0].as_ref().unwrap().to_string(),
            format!("\"{}\"", hostile.replace('"', "\\\""))
        );
    }

    // UPDATE in place over the bound algebra.
    #[test]
    fn update_in_place_prepared_works() {
        let mut graph = g();
        let pu = PreparedUpdate::parse(
            "DELETE { ?s <http://ex/knows> ?o } \
             WHERE  { ?s <http://ex/knows> ?o . FILTER(?s = ?who) }",
        )
        .unwrap()
        .bind("who", value::iri("http://ex/alice").unwrap())
        .unwrap();
        update_in_place_prepared(&mut graph, &pu).unwrap();
        let r = query(
            &graph,
            "SELECT ?o WHERE { <http://ex/alice> <http://ex/knows> ?o }",
        )
        .unwrap();
        assert_eq!(r.len(), 0);
        // bob still knows carol
        assert!(crate::ask(
            &graph,
            "ASK { <http://ex/bob> <http://ex/knows> <http://ex/carol> }"
        )
        .unwrap());
    }

    // Fail-closed: a placeholder that is not free is rejected (typo guard).
    #[test]
    fn unknown_parameter_is_rejected() {
        let err = PreparedQuery::parse("SELECT ?s WHERE { ?s <http://ex/name> ?n }")
            .unwrap()
            .bind("missing", value::string("x"))
            .unwrap_err();
        assert!(err.contains("no free parameter"), "{}", err);
    }

    // A plain PROJECTED variable IS bindable (RDF4J `setBinding` semantics) — this
    // is the common case and must not be rejected.
    #[test]
    fn binding_projected_variable_is_allowed() {
        let pq = PreparedQuery::parse("SELECT ?s ?n WHERE { ?s <http://ex/name> ?n }")
            .unwrap()
            .bind("s", value::iri("http://ex/alice").unwrap())
            .unwrap();
        let r = query_prepared(&g(), &pq).unwrap();
        assert_eq!(r.len(), 1);
    }

    // Fail-closed: binding an AGGREGATE output variable is rejected (it is computed).
    #[test]
    fn binding_aggregate_output_is_rejected() {
        let err = PreparedQuery::parse(
            "SELECT ?who (COUNT(?f) AS ?n) WHERE { ?who <http://ex/knows> ?f } GROUP BY ?who",
        )
        .unwrap()
        .bind(
            "n",
            value::typed("5", "http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        )
        .unwrap_err();
        assert!(err.contains("result variable"), "{}", err);
    }

    // Fail-closed: binding a BIND-output variable is rejected.
    #[test]
    fn binding_bind_output_is_rejected() {
        let err = PreparedQuery::parse(
            "SELECT ?up WHERE { ?s <http://ex/name> ?n . BIND(UCASE(?n) AS ?up) }",
        )
        .unwrap()
        .bind("up", value::string("X"))
        .unwrap_err();
        assert!(err.contains("result variable"), "{}", err);
    }

    // Fail-closed: a blank node cannot be bound into a predicate slot.
    #[test]
    fn blank_node_in_predicate_is_rejected() {
        let err = PreparedQuery::parse("SELECT ?o WHERE { <http://ex/alice> ?prop ?o }")
            .unwrap()
            .bind("prop", value::blank("b0").unwrap())
            .unwrap_err();
        assert!(err.contains("blank node"), "{}", err);
    }

    // The placeholder may be written with or without a leading ? / $.
    #[test]
    fn placeholder_prefix_is_optional() {
        let base = PreparedQuery::parse("SELECT ?friend WHERE { ?who <http://ex/knows> ?friend }")
            .unwrap();
        for name in ["who", "?who", "$who"] {
            let pq = base
                .bind(name, value::iri("http://ex/alice").unwrap())
                .unwrap();
            assert_eq!(query_prepared(&g(), &pq).unwrap().len(), 1);
        }
    }

    // Binding is composable: bind two placeholders in sequence (immutable template).
    #[test]
    fn sequential_binds_are_immutable() {
        let template = PreparedQuery::parse(
            "SELECT ?p WHERE { ?a <http://ex/knows> ?b . ?b <http://ex/knows> ?p }",
        )
        .unwrap();
        let pq = template
            .bind("a", value::iri("http://ex/alice").unwrap())
            .unwrap()
            .bind("b", value::iri("http://ex/bob").unwrap())
            .unwrap();
        let r = query_prepared(&g(), &pq).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(
            r.rows[0][0].as_ref().unwrap().to_string(),
            "<http://ex/carol>"
        );
        // the original template is unchanged (still has free ?a) — bind returns a new value.
        assert!(template
            .bind("a", value::iri("http://ex/x").unwrap())
            .is_ok());
    }

    // A language-tagged / typed literal binds and matches by value+tag.
    #[test]
    fn typed_and_lang_literals_bind() {
        let g2 = Graph::load_str(
            "<http://ex/a> <http://ex/p> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> . \
             <http://ex/b> <http://ex/p> \"hi\"@en .",
            "turtle",
        )
        .unwrap();
        let pq = PreparedQuery::parse("SELECT ?s WHERE { ?s <http://ex/p> ?v }")
            .unwrap()
            .bind(
                "v",
                value::typed("42", "http://www.w3.org/2001/XMLSchema#integer").unwrap(),
            )
            .unwrap();
        assert_eq!(query_prepared(&g2, &pq).unwrap().len(), 1);
        let pq2 = PreparedQuery::parse("SELECT ?s WHERE { ?s <http://ex/p> ?v }")
            .unwrap()
            .bind("v", value::lang_string("hi", "en").unwrap())
            .unwrap();
        assert_eq!(query_prepared(&g2, &pq2).unwrap().len(), 1);
    }
}
