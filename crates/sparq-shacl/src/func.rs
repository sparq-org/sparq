//! Named operators, list-parameter SPARQL functions and custom expression bodies.
//! SPARQL scalar semantics come from the engine; custom argument scopes remain
//! local to the expression call.

use super::{EvalScope, NodeExpr};
use crate::model::{sh, ShapesModel};
use crate::view::{dedup, GraphView};
use oxrdf::{Literal, NamedNode, Term};

/// `http://www.w3.org/ns/shacl-node-expr#` — the SHACL 1.2 node-expression
/// vocabulary the W3C suite uses for the built-in operators.
const SHNEX: &str = "http://www.w3.org/ns/shacl-node-expr#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// A parsed function expression: an operator and the operands it was parsed with.
#[derive(Debug, Clone)]
pub(crate) struct Func {
    op: Op,
}

/// The recognised SHACL function operators. Each variant carries its
/// already-parsed operand node expressions / inline filter-shape ids.
#[derive(Debug, Clone)]
enum Op {
    /// `concat ( E1 E2 … )` — members of each argument, concatenated in order
    /// (NO dedup; preserves duplicates).
    Concat(Vec<NodeExpr>),
    /// `count E` — the count of nodes in `Eval(E)` as an `xsd:integer`.
    Count(Box<NodeExpr>),
    /// `sum E` — numeric sum of `Eval(E)` (empty ⇒ 0).
    Sum(Box<NodeExpr>),
    /// `min E` / `max E` under the SPARQL term order (empty ⇒ empty).
    MinMax {
        arg: Box<NodeExpr>,
        max: bool,
    },
    /// `distinct E` — `Eval(E)` deduplicated by RDF term equality.
    Distinct(Box<NodeExpr>),
    /// `if C ; then T ; else U?` — `Eval(T)` when `Eval(C) = { true }`, else
    /// `Eval(U)` (empty when no `else`).
    If {
        cond: Box<NodeExpr>,
        then: Option<Box<NodeExpr>>,
        els: Option<Box<NodeExpr>>,
    },
    /// `exists E` — `{ true }` if `Eval(E)` is non-empty, else `{ false }`.
    Exists(Box<NodeExpr>),
    /// `nodes N ; limit k` / `offset k` — the first `k` / the all-but-first-`k`
    /// nodes of `Eval(N)`. `limit`/`offset` may co-occur (offset then limit, but
    /// they nest as separate expressions in the suite).
    Slice {
        nodes: Box<NodeExpr>,
        limit: Option<usize>,
        offset: usize,
    },
    /// `instancesOf C` — the SHACL instances of class `C` (subclass closure).
    InstancesOf(Box<NodeExpr>),
    /// `nodesMatching S` — every node in the graph that conforms to shape `S`.
    NodesMatching(usize),
    /// `nodes N ; flatMap E` — for each `n` in `Eval(N)`, `Eval(E)` with the
    /// focus rebound to `n`; concatenated (NO dedup).
    FlatMap {
        nodes: Box<NodeExpr>,
        body: Box<NodeExpr>,
    },
    /// `findFirst S ; nodes N` — the first node of `Eval(N)` conforming to `S`
    /// (empty if none).
    FindFirst {
        nodes: Box<NodeExpr>,
        shape: usize,
    },
    /// `matchAll S ; nodes N` — `{ true }` iff every node of `Eval(N)` conforms
    /// to `S` (empty input ⇒ `{ true }`), else `{ false }`.
    MatchAll {
        nodes: Box<NodeExpr>,
        shape: usize,
    },
    /// `remove R ; nodes N` — `Eval(N)` minus the members of `Eval(R)` (term
    /// equality), order-preserving.
    Remove {
        nodes: Box<NodeExpr>,
        removed: Box<NodeExpr>,
    },
    /// Sort nodes by the first key value, with unbound keys first ascending.
    OrderBy {
        nodes: Box<NodeExpr>,
        key: Box<NodeExpr>,
        desc: bool,
    },
    /// `var "name"` — `"focusNode"` ⇒ `{ focus }`; any other name resolves against
    /// the caller-supplied [`crate::rules::Scope`] (⇒ empty when absent).
    Var(String),
    Arg(Term),
    Custom {
        body: Term,
        args: Vec<(Term, Term)>,
        scalar: bool,
    },
    Builtin {
        name: String,
        args: Vec<NodeExpr>,
        query: crate::sparql::PreparedSelectExpr,
    },
    Select(crate::sparql::PreparedSelectExpr),
    ConformsToShape(Vec<NodeExpr>),
    /// A custom `sh:SPARQLFunction`: its SELECT body, parameter order, and the
    /// (already-parsed) argument node expressions to pre-bind.
    Sparql {
        select: String,
        params: Vec<String>,
        args: Vec<NodeExpr>,
    },
}

/// The single object of `subject <ns#local>`, trying `shnex:` then `sh:`.
fn op_object(g: &GraphView, subject: &Term, local: &str) -> Option<Term> {
    g.object(subject, &format!("{SHNEX}{local}"))
        .or_else(|| g.object(subject, &sh(local)))
}

/// True iff `subject` carries the operator predicate `local` (in either namespace).
fn has_op(g: &GraphView, subject: &Term, local: &str) -> bool {
    op_object(g, subject, local).is_some()
}

/// Parses a function expression rooted at the blank node `node`. Returns `None`
/// when `node` carries no recognised operator predicate and names no declared
/// `sh:SPARQLFunction` — the lenient-drop contract.
pub(super) fn parse(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<Func> {
    if let Some(query) = crate::sparql::expression_query(g, node) {
        return mk(Op::Select(query));
    }
    if let Some(key) = op_object(g, node, "arg") {
        return mk(Op::Arg(key));
    }
    if let Some(head) = op_object(g, node, "conformsToShape") {
        let args = arguments(g, model, &head)?;
        return (args.len() == 2).then_some(Func {
            op: Op::ConformsToShape(args),
        });
    }
    // ---- single-argument aggregate / set operators ----
    if let Some(arg) = op_object(g, node, "count") {
        return mk(Op::Count(boxed(g, model, &arg)?));
    }
    if let Some(arg) = op_object(g, node, "sum") {
        return mk(Op::Sum(boxed(g, model, &arg)?));
    }
    if let Some(arg) = op_object(g, node, "min") {
        return mk(Op::MinMax {
            arg: boxed(g, model, &arg)?,
            max: false,
        });
    }
    if let Some(arg) = op_object(g, node, "max") {
        return mk(Op::MinMax {
            arg: boxed(g, model, &arg)?,
            max: true,
        });
    }
    if let Some(arg) = op_object(g, node, "distinct") {
        return mk(Op::Distinct(boxed(g, model, &arg)?));
    }
    if let Some(arg) = op_object(g, node, "exists") {
        return mk(Op::Exists(boxed(g, model, &arg)?));
    }
    if let Some(class) = op_object(g, node, "instancesOf") {
        return mk(Op::InstancesOf(boxed(g, model, &class)?));
    }
    if let Some(shape_term) = op_object(g, node, "nodesMatching") {
        let shape = model.by_node(&shape_term)?;
        return mk(Op::NodesMatching(shape));
    }
    if let Some(var) = op_object(g, node, "var") {
        if let Term::Literal(l) = &var {
            return mk(Op::Var(l.value().to_string()));
        }
        return None;
    }

    // ---- `concat ( E1 E2 … )`: a SHACL list of argument expressions ----
    if let Some(list_head) = op_object(g, node, "concat") {
        let members = parse_list(g, model, &list_head)?;
        return mk(Op::Concat(members));
    }

    // ---- `if … then … else?` ----
    if let Some(cond) = op_object(g, node, "if") {
        let then = op_object(g, node, "then");
        let els = op_object(g, node, "else");
        return mk(Op::If {
            cond: boxed(g, model, &cond)?,
            then: match then {
                Some(t) => Some(boxed(g, model, &t)?),
                None => None,
            },
            els: match els {
                Some(e) => Some(boxed(g, model, &e)?),
                None => None,
            },
        });
    }

    // ---- operators that read a `shnex:nodes` input expression ----
    // (limit / offset / flatMap / orderBy live on the SAME node as `nodes`.)
    if has_op(g, node, "flatMap") {
        let nodes = nodes_input(g, model, node)?;
        let body = op_object(g, node, "flatMap")?;
        return mk(Op::FlatMap {
            nodes: Box::new(nodes),
            body: boxed(g, model, &body)?,
        });
    }
    if has_op(g, node, "orderBy") {
        let nodes = nodes_input(g, model, node)?;
        let key = op_object(g, node, "orderBy")?;
        let desc = bool_op(g, node, "desc");
        return mk(Op::OrderBy {
            nodes: Box::new(nodes),
            key: boxed(g, model, &key)?,
            desc,
        });
    }
    if has_op(g, node, "limit") || has_op(g, node, "offset") {
        let nodes = nodes_input(g, model, node)?;
        let limit = int_op(g, node, "limit").map(|n| n.max(0) as usize);
        let offset = int_op(g, node, "offset")
            .map(|n| n.max(0) as usize)
            .unwrap_or(0);
        return mk(Op::Slice {
            nodes: Box::new(nodes),
            limit,
            offset,
        });
    }

    // ---- filter-shape operators (a shape applied to a `nodes` input) ----
    if let Some(shape_term) = op_object(g, node, "findFirst") {
        let shape = inline_or_named_shape(model, &shape_term)?;
        let nodes = nodes_input(g, model, node)?;
        return mk(Op::FindFirst {
            nodes: Box::new(nodes),
            shape,
        });
    }
    if let Some(shape_term) = op_object(g, node, "matchAll") {
        let shape = inline_or_named_shape(model, &shape_term)?;
        let nodes = nodes_input(g, model, node)?;
        return mk(Op::MatchAll {
            nodes: Box::new(nodes),
            shape,
        });
    }
    if let Some(removed) = op_object(g, node, "remove") {
        let nodes = nodes_input(g, model, node)?;
        return mk(Op::Remove {
            nodes: Box::new(nodes),
            removed: boxed(g, model, &removed)?,
        });
    }

    for (pred, object) in g.predicate_objects(node) {
        if let Term::NamedNode(iri) = &pred {
            if let Some(name) = iri.as_str().strip_prefix("http://www.w3.org/ns/sparql#") {
                let args = arguments(g, model, &object)?;
                let expression = builtin_expression(name, args.len())?;
                let query = crate::sparql::PreparedSelectExpr::build_expr("", &expression)?;
                return mk(Op::Builtin {
                    name: name.into(),
                    args,
                    query,
                });
            }
            if g.instances_of(
                &NamedNode::new_unchecked(sh("ListParameterExpressionFunction")).into(),
            )
            .contains(&pred)
            {
                let body = g.object(&pred, &sh("bodyExpression"))?;
                let args = argument_terms(g, &object)?
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| (Literal::from(i as i64).into(), t))
                    .collect();
                return mk(Op::Custom {
                    body,
                    args,
                    scalar: true,
                });
            }
        }
    }
    for function in
        g.instances_of(&NamedNode::new_unchecked(sh("NamedParameterExpressionFunction")).into())
    {
        let params = g.objects(&function, &sh("parameter"));
        let key_matches = params.iter().any(|p| bool_op(g, p, "keyParameter")
            && matches!(g.object(p, &sh("path")), Some(Term::NamedNode(n)) if g.object(node, n.as_str()).is_some()));
        if key_matches {
            let body = g.object(&function, &sh("bodyExpression"))?;
            let args = params
                .iter()
                .filter_map(|p| {
                    let key = g.object(p, &sh("path"))?;
                    let Term::NamedNode(n) = &key else {
                        return None;
                    };
                    Some((key.clone(), g.object(node, n.as_str())?))
                })
                .collect();
            return mk(Op::Custom {
                body,
                args,
                scalar: false,
            });
        }
    }
    parse_custom_function(g, model, node)
}

fn argument_terms(g: &GraphView, object: &Term) -> Option<Vec<Term>> {
    if object == &NamedNode::new_unchecked(crate::view::RDF_NIL).into()
        || g.object(object, crate::view::RDF_FIRST).is_some()
    {
        g.list_strict(object)
    } else {
        Some(vec![object.clone()])
    }
}

fn arguments(g: &GraphView, model: &ShapesModel, object: &Term) -> Option<Vec<NodeExpr>> {
    argument_terms(g, object)?
        .iter()
        .map(|t| super::parse_node_expr(g, model, t))
        .collect()
}

fn builtin_expression(name: &str, arity: usize) -> Option<String> {
    let vars: Vec<_> = (0..arity).map(|i| format!("?arg{i}")).collect();
    let infix = match name {
        "plus" => Some("+"),
        "subtract" => Some("-"),
        "multiply" => Some("*"),
        "divide" => Some("/"),
        "equals" | "sameValue" => Some("="),
        "not-equals" => Some("!="),
        "less-than" => Some("<"),
        "less-than-or-equal" => Some("<="),
        "greater-than" => Some(">"),
        "greater-than-or-equal" => Some(">="),
        "logical-and" => Some("&&"),
        "logical-or" => Some("||"),
        _ => None,
    };
    if let Some(op) = infix {
        return (arity == 2).then(|| format!("({} {op} {})", vars[0], vars[1]));
    }
    let prefix = match name {
        "unary-plus" => Some("+"),
        "unary-minus" => Some("-"),
        "logical-not" => Some("!"),
        _ => None,
    };
    if let Some(op) = prefix {
        return (arity == 1).then(|| format!("{op}{}", vars[0]));
    }
    let function = match name {
        "encode" => "ENCODE_FOR_URI",
        other => other,
    };
    Some(format!("{function}({})", vars.join(",")))
}

fn mk(op: Op) -> Option<Func> {
    Some(Func { op })
}

fn boxed(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<Box<NodeExpr>> {
    super::parse_node_expr(g, model, node).map(Box::new)
}

/// Parses a SHACL list of node expressions; `None` if any member is unsupported.
fn parse_list(g: &GraphView, model: &ShapesModel, head: &Term) -> Option<Vec<NodeExpr>> {
    g.list_strict(head)?
        .iter()
        .map(|m| super::parse_node_expr(g, model, m))
        .collect()
}

/// The `shnex:nodes`/`sh:nodes` input expression, defaulting to `sh:this`.
fn nodes_input(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<NodeExpr> {
    match op_object(g, node, "nodes") {
        Some(n) => super::parse_node_expr(g, model, &n),
        None => Some(NodeExpr::This),
    }
}

/// A filter shape that is either a model-declared shape (by node) or an inline
/// anonymous shape blank node — the latter is parsed into the model on demand.
fn inline_or_named_shape(model: &ShapesModel, shape_term: &Term) -> Option<usize> {
    // Inline filter shapes (e.g. `[ sh:minInclusive 3 ]`) are anonymous shapes the
    // shapes-model parser already discovers (every blank node bearing a constraint
    // parameter is a shape). So a model lookup by node covers both cases.
    model.by_node(shape_term)
}

fn bool_op(g: &GraphView, node: &Term, local: &str) -> bool {
    matches!(op_object(g, node, local), Some(Term::Literal(l)) if parse_bool(l.value()) == Some(true))
}

fn int_op(g: &GraphView, node: &Term, local: &str) -> Option<i64> {
    match op_object(g, node, local) {
        Some(Term::Literal(l)) => l.value().trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Parses a custom `sh:SPARQLFunction` application `[ <funcIRI> ( args… ) ]`.
fn parse_custom_function(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<Func> {
    // The function IRI is the lone predicate of the expression node whose object
    // is a SHACL list (or rdf:nil) of argument expressions.
    for (pred, obj) in g.predicate_objects(node) {
        let Term::NamedNode(fn_iri) = &pred else {
            continue;
        };
        // Skip the SHACL/shnex namespaces — those are operator predicates, not
        // function IRIs (already handled above; reaching here means unsupported).
        if fn_iri.as_str().starts_with(SHNEX) || fn_iri.as_str().starts_with(&sh("")) {
            continue;
        }
        let (select, params) = sparql_function_decl(g, fn_iri)?;
        let args = parse_list(g, model, &obj).unwrap_or_default();
        // Argument count must match the declared parameter order.
        if args.len() != params.len() {
            return None;
        }
        return mk(Op::Sparql {
            select,
            params,
            args,
        });
    }
    None
}

/// Reads a `sh:SPARQLFunction` declaration: its `sh:select` body and the ordered
/// `sh:parameter` `sh:path` predicate local-variable names. `None` if `iri` is
/// not a declared SPARQL function.
fn sparql_function_decl(g: &GraphView, iri: &NamedNode) -> Option<(String, Vec<String>)> {
    let fn_term = Term::NamedNode(iri.clone());
    // Must be declared a sh:SPARQLFunction (typed) with a sh:select body.
    let is_fn = g
        .objects(&fn_term, crate::view::RDF_TYPE)
        .iter()
        .any(|t| matches!(t, Term::NamedNode(n) if n.as_str() == sh("SPARQLFunction")));
    if !is_fn {
        return None;
    }
    let select = match g.object(&fn_term, &sh("select")) {
        Some(Term::Literal(l)) => l.value().to_string(),
        _ => return None,
    };
    // sh:parameter order: each parameter declares a sh:path predicate; its local
    // name (the trailing path segment) is the SPARQL variable to bind.
    let mut params = Vec::new();
    for p in g.objects(&fn_term, &sh("parameter")) {
        if let Some(Term::NamedNode(n)) = g.object(&p, &sh("path")) {
            let local = n
                .as_str()
                .rsplit(['#', '/'])
                .next()
                .unwrap_or(n.as_str())
                .to_string();
            params.push(local);
        }
    }
    Some((select, params))
}

impl Func {
    pub(super) fn eval(
        &self,
        graph: &sparq_core::Graph,
        view: &GraphView,
        model: &ShapesModel,
        focus: &Term,
        scope: &EvalScope,
    ) -> Result<Vec<Term>, String> {
        let eval = |e: &NodeExpr| e.try_eval(graph, view, model, focus, scope);
        Ok(match &self.op {
            Op::Concat(members) => {
                let mut out = Vec::new();
                for m in members {
                    out.extend(eval(m)?);
                }
                out
            }
            Op::Count(arg) => vec![int_literal(eval(arg)?.len() as i128)],
            Op::Sum(arg) => vec![sum_literal(&eval(arg)?)],
            Op::MinMax { arg, max } => min_max(&eval(arg)?, *max).into_iter().collect(),
            Op::Distinct(arg) => dedup(eval(arg)?),
            Op::Exists(arg) => vec![bool_literal(!eval(arg)?.is_empty())],
            Op::If { cond, then, els } => {
                let branch = if is_true_set(&eval(cond)?) { then } else { els };
                match branch {
                    Some(e) => eval(e)?,
                    None => Vec::new(),
                }
            }
            Op::Slice {
                nodes,
                limit,
                offset,
            } => {
                let after = eval(nodes)?.into_iter().skip(*offset);
                match limit {
                    Some(k) => after.take(*k).collect(),
                    None => after.collect(),
                }
            }
            Op::InstancesOf(class) => {
                let classes = eval(class)?;
                if classes.iter().any(|c| !matches!(c, Term::NamedNode(_))) {
                    return Err("Class must be an IRI".into());
                }
                dedup(classes.iter().flat_map(|c| view.instances_of(c)))
            }
            Op::NodesMatching(shape) => {
                let mut out = Vec::new();
                for n in candidate_nodes(view) {
                    if super::conforms(graph, model, *shape, &n)? {
                        out.push(n);
                    }
                }
                out
            }
            Op::FlatMap { nodes, body } => {
                let mut out = Vec::new();
                for n in eval(nodes)? {
                    out.extend(body.try_eval(graph, view, model, &n, scope)?);
                }
                out
            }
            Op::FindFirst { nodes, shape } => {
                let mut out = Vec::new();
                for n in eval(nodes)? {
                    if super::conforms(graph, model, *shape, &n)? {
                        out.push(n);
                        break;
                    }
                }
                out
            }
            Op::MatchAll { nodes, shape } => {
                let mut all = true;
                for n in eval(nodes)? {
                    if !super::conforms(graph, model, *shape, &n)? {
                        all = false;
                        break;
                    }
                }
                vec![bool_literal(all)]
            }
            Op::Remove { nodes, removed } => {
                let drop: std::collections::HashSet<_> = eval(removed)?.into_iter().collect();
                eval(nodes)?
                    .into_iter()
                    .filter(|n| !drop.contains(n))
                    .collect()
            }
            Op::OrderBy { nodes, key, desc } => {
                let mut keyed = Vec::new();
                for n in eval(nodes)? {
                    let k = key
                        .try_eval(graph, view, model, &n, scope)?
                        .into_iter()
                        .next();
                    keyed.push((n, k));
                }
                keyed.sort_by(|(_, a), (_, b)| {
                    let order = cmp_key(a.as_slice(), b.as_slice());
                    if *desc {
                        order.reverse()
                    } else {
                        order
                    }
                });
                keyed.into_iter().map(|(n, _)| n).collect()
            }
            Op::Var(name) => {
                if name == "focusNode" {
                    vec![focus.clone()]
                } else {
                    scope.variables.get(name).cloned().unwrap_or_default()
                }
            }
            Op::Arg(key) => {
                if let Some(value) = scope.arguments.get(key) {
                    let expr = super::parse_node_expr(&GraphView::new(&model.source), model, value)
                        .ok_or("Invalid argument expression")?;
                    expr.try_eval(graph, view, model, focus, &EvalScope::default())?
                } else {
                    Vec::new()
                }
            }
            Op::Custom { body, args, scalar } => {
                let mut inner = EvalScope::default();
                let mut empty = false;
                for (key, value) in args {
                    let value = if *scalar {
                        let arg =
                            super::parse_node_expr(&GraphView::new(&model.source), model, value)
                                .ok_or("Invalid argument")?;
                        let value = arg.try_eval(graph, view, model, focus, scope)?;
                        if value.len() > 1 {
                            return Err("Function argument has multiple nodes".into());
                        }
                        let Some(value) = value.into_iter().next() else {
                            empty = true;
                            continue;
                        };
                        value
                    } else {
                        value.clone()
                    };
                    inner.arguments.insert(key.clone(), value);
                }
                if empty {
                    return Ok(Vec::new());
                }
                let body = super::parse_node_expr(&GraphView::new(&model.source), model, body)
                    .ok_or("Invalid function body")?;
                let out = body.try_eval(graph, view, model, focus, &inner)?;
                if *scalar && out.len() > 1 {
                    return Err("Function body has multiple nodes".into());
                }
                out
            }
            Op::ConformsToShape(args) => {
                let mut values = Vec::new();
                let mut empty = false;
                for arg in args {
                    let value = eval(arg)?;
                    if value.len() > 1 {
                        return Err("Function argument has multiple nodes".into());
                    }
                    if let Some(value) = value.into_iter().next() {
                        values.push(value);
                    } else {
                        empty = true;
                    }
                }
                if empty {
                    return Ok(Vec::new());
                }
                let shape = model.by_node(&values[1]).ok_or("Unknown shape")?;
                vec![bool_literal(super::conforms(
                    graph, model, shape, &values[0],
                )?)]
            }
            Op::Select(query) => {
                let mut owned = vec![("this".to_string(), focus.clone())];
                for (name, values) in &scope.variables {
                    if name == "this" || oxrdf::Variable::new(name).is_err() {
                        return Err("Invalid scope variable".into());
                    }
                    if values.len() > 1 {
                        return Err("Scope variable has multiple nodes".into());
                    }
                    if let Some(value) = values.first() {
                        owned.push((name.clone(), value.clone()));
                    }
                }
                for (key, value) in &scope.arguments {
                    let name = match key {
                        Term::Literal(l) => format!("arg{}", l.value()),
                        _ => format!("arg{key}"),
                    };
                    if oxrdf::Variable::new(&name).is_err() {
                        return Err("Invalid argument variable".into());
                    }
                    owned.push((name, value.clone()));
                }
                let bindings: Vec<_> = owned
                    .iter()
                    .map(|(name, value)| (name.as_str(), value))
                    .collect();
                crate::sparql::expression_bindings(query, graph, &bindings)?
            }
            Op::Builtin { name, args, query } => {
                let handles_unbound = matches!(
                    name.as_str(),
                    "bound" | "coalesce" | "if" | "logical-and" | "logical-or"
                );
                let mut owned = Vec::new();
                let mut empty = false;
                let mut error = false;
                for (i, arg) in args.iter().enumerate() {
                    match eval(arg) {
                        Ok(v) if v.len() == 1 => {
                            owned.push((format!("arg{i}"), v.into_iter().next().unwrap()))
                        }
                        Ok(v) if v.is_empty() => empty = true,
                        _ => error = true,
                    }
                }
                if !handles_unbound {
                    if error {
                        return Err("SPARQL argument evaluation failed".into());
                    }
                    if empty {
                        return Ok(Vec::new());
                    }
                }
                let bindings: Vec<_> = owned.iter().map(|(n, v)| (n.as_str(), v)).collect();
                let result = crate::sparql::expression_bindings(query, graph, &bindings)?;
                if result.is_empty() {
                    return Err("SPARQL function evaluation failed".into());
                }
                result
            }
            Op::Sparql {
                select,
                params,
                args,
            } => {
                let mut owned = Vec::new();
                for (param, arg) in params.iter().zip(args) {
                    let value = eval(arg)?;
                    if value.len() > 1 {
                        return Ok(Vec::new());
                    }
                    if let Some(v) = value.into_iter().next() {
                        owned.push((param.as_str(), v));
                    } else {
                        return Ok(Vec::new());
                    }
                }
                let query = crate::sparql::PreparedSelectExpr::build_select("", select)
                    .ok_or("Invalid function SELECT")?;
                let bindings: Vec<_> = owned.iter().map(|(n, v)| (*n, v)).collect();
                crate::sparql::function_bindings(&query, graph, &bindings)?
            }
        })
    }
}

/// The candidate node set of a graph for `nodesMatching`: every subject and every
/// object that is not a literal (shapes apply to IRIs / blank nodes), deduplicated.
fn candidate_nodes(view: &GraphView) -> Vec<Term> {
    let mut out = Vec::new();
    for [s, _, o] in view.triples(None, None, None) {
        if !matches!(s, Term::Literal(_)) {
            out.push(s);
        }
        if !matches!(o, Term::Literal(_)) {
            out.push(o);
        }
    }
    dedup(out)
}

/// A set is "true" iff it is exactly `{ xsd:boolean true }`.
fn is_true_set(set: &[Term]) -> bool {
    set.len() == 1 && is_true(&set[0])
}

fn is_true(t: &Term) -> bool {
    matches!(t, Term::Literal(l)
        if l.datatype().as_str() == format!("{XSD}boolean") && parse_bool(l.value()) == Some(true))
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Numeric value of a literal as `f64`, if it is an XSD numeric type.
fn numeric(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) if is_numeric(l.datatype().as_str()) => l.value().trim().parse().ok(),
        _ => None,
    }
}

fn is_numeric(dt: &str) -> bool {
    let Some(local) = dt.strip_prefix(XSD) else {
        return false;
    };
    matches!(
        local,
        "integer"
            | "decimal"
            | "double"
            | "float"
            | "long"
            | "int"
            | "short"
            | "byte"
            | "nonNegativeInteger"
            | "positiveInteger"
            | "nonPositiveInteger"
            | "negativeInteger"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
    )
}

/// True iff a numeric literal is integer-valued (no fractional part) — so a sum of
/// integers stays `xsd:integer` rather than `xsd:decimal`.
fn is_integer_typed(t: &Term) -> bool {
    matches!(t, Term::Literal(l) if matches!(
        l.datatype().as_str().strip_prefix(XSD),
        Some("integer" | "long" | "int" | "short" | "byte"
            | "nonNegativeInteger" | "positiveInteger" | "nonPositiveInteger"
            | "negativeInteger" | "unsignedLong" | "unsignedInt"
            | "unsignedShort" | "unsignedByte")
    ))
}

fn int_literal(n: i128) -> Term {
    Term::Literal(Literal::new_typed_literal(
        n.to_string(),
        NamedNode::new_unchecked(format!("{XSD}integer")),
    ))
}

fn decimal_literal(v: f64) -> Term {
    // Render without an exponent and ALWAYS with a fractional part, so the lexical
    // form is a canonical xsd:decimal: a whole-valued decimal must read `42.0`, not
    // `42` (the W3C node-expr `sum-totalRevenue` sums decimals to a whole number
    // and expects `42.0`^^xsd:decimal). (sq-mue75)
    let s = format!("{v}");
    let s = if s.contains('.') { s } else { format!("{s}.0") };
    Term::Literal(Literal::new_typed_literal(
        s,
        NamedNode::new_unchecked(format!("{XSD}decimal")),
    ))
}

fn bool_literal(b: bool) -> Term {
    Term::Literal(Literal::new_typed_literal(
        if b { "true" } else { "false" },
        NamedNode::new_unchecked(format!("{XSD}boolean")),
    ))
}

/// Sum of the numeric members; `xsd:integer` if every member is integer-typed,
/// else `xsd:decimal`. Non-numeric members are ignored (empty ⇒ 0).
fn sum_literal(vals: &[Term]) -> Term {
    let mut total = 0.0_f64;
    let mut all_int = true;
    for v in vals {
        if let Some(n) = numeric(v) {
            total += n;
            all_int &= is_integer_typed(v);
        }
    }
    if all_int && total.fract() == 0.0 {
        int_literal(total as i128)
    } else {
        decimal_literal(total)
    }
}

/// MIN/MAX use the same term ordering as ORDER BY.
fn min_max(vals: &[Term], max: bool) -> Option<Term> {
    if max {
        vals.iter().max_by(|a, b| cmp_term(a, b))
    } else {
        vals.iter().min_by(|a, b| cmp_term(a, b))
    }
    .cloned()
}

/// Comparison of two sort keys (each a node set) for `orderBy`: compares the least
/// value of each. An empty key sorts before any value (ascending).
fn cmp_key(a: &[Term], b: &[Term]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let la = least(a);
    let lb = least(b);
    match (la, lb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => cmp_term(x, y),
    }
}

/// The least term of a set under [`cmp_term`] (for the orderBy sort key).
fn least(set: &[Term]) -> Option<&Term> {
    set.iter().min_by(|a, b| cmp_term(a, b))
}

fn cmp_term(a: &Term, b: &Term) -> std::cmp::Ordering {
    super::order::compare(a, b)
}
