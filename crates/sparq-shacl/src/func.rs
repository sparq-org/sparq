//! (sq-mk9n) The SHACL-function registry and the **function-expression
//! form** of the SHACL-AF node-expression algebra (a child module of
//! [`crate::rules`], compiled only behind the `shacl-af` feature).
//!
//! A *function expression* is a SHACL function applied to operand node
//! expressions. SHACL 1.2 spells the common operators as predicate-named built-ins
//! in the `shnex:`/`sh:` namespaces (`shnex:concat`, `shnex:sum`, …); SHACL-AF also
//! allows a custom `sh:SPARQLFunction` IRI applied to a `sh:list` of arguments
//! (`[ ex:myFn ( <arg-expr> … ) ]`). This module is the single registry for both:
//!
//!   * [`Op`] — the recognised built-in operators, parsed from the operand
//!     predicates of a blank-node expression.
//!   * a **custom `sh:SPARQLFunction`** — its declaration (`sh:select` body and
//!     `sh:parameter` order) is read from the shapes graph and run through the
//!     SPARQL engine with the evaluated arguments pre-bound.
//!
//! An unrecognised function (a blank node carrying none of the built-in operator
//! predicates and whose head IRI names no declared `sh:SPARQLFunction`) yields
//! `None` from [`parse`], so the caller drops it leniently — a rule that uses an
//! unsupported function infers nothing rather than misfiring.
//!
//! ## Scope (honest)
//!
//! The implemented built-ins are the deterministic operators the SHACL 1.2
//! node-expression test suite exercises: `concat`, `count`, `sum`, `min`, `max`,
//! `distinct`, `if`/`then`/`else`, `exists`, `limit`, `offset`, `instancesOf`,
//! `nodesMatching`, `flatMap`, `findFirst`, `matchAll`, `remove`, `orderBy`,
//! and `var` (`"focusNode"` is the focus node; any other name resolves against a
//! caller-supplied variable scope — the W3C suite's `sht:scope-<name>` injection —
//! else the empty set, matching the `var-unbound` entry). `sh:SPARQLFunction`
//! custom functions are dispatched through the engine. (sq-u5rxj) the
//! `shnex:var "bound"` scope-injection entry is now supported via the threaded
//! [`crate::rules::Scope`].

use super::NodeExpr;
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
    /// `min E` / `max E` — the numerically least / greatest of `Eval(E)` (empty
    /// ⇒ empty). `true` = max.
    MinMax { arg: Box<NodeExpr>, max: bool },
    /// `distinct E` — `Eval(E)` deduplicated by RDF term equality.
    Distinct(Box<NodeExpr>),
    /// `if C ; then T ; else U?` — `Eval(T)` when `Eval(C) = { true }`, else
    /// `Eval(U)` (empty when no `else`).
    If {
        cond: Box<NodeExpr>,
        then: Box<NodeExpr>,
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
    InstancesOf(Term),
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
    FindFirst { nodes: Box<NodeExpr>, shape: usize },
    /// `matchAll S ; nodes N` — `{ true }` iff every node of `Eval(N)` conforms
    /// to `S` (empty input ⇒ `{ true }`), else `{ false }`.
    MatchAll { nodes: Box<NodeExpr>, shape: usize },
    /// `remove R ; nodes N` — `Eval(N)` minus the members of `Eval(R)` (term
    /// equality), order-preserving.
    Remove {
        nodes: Box<NodeExpr>,
        removed: Box<NodeExpr>,
    },
    /// `nodes N ; orderBy K ; desc?` — `Eval(N)` sorted by the sort key
    /// `Eval(K, n)` (its least value), ascending (or descending). Nodes with no
    /// key sort first (ascending).
    OrderBy {
        nodes: Box<NodeExpr>,
        key: Box<NodeExpr>,
        desc: bool,
    },
    /// `var "name"` — `"focusNode"` ⇒ `{ focus }`; any other name resolves against
    /// the caller-supplied [`crate::rules::Scope`] (⇒ empty when absent).
    Var(String),
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
        return mk(Op::InstancesOf(class));
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
        let then = op_object(g, node, "then")?;
        let els = op_object(g, node, "else");
        return mk(Op::If {
            cond: boxed(g, model, &cond)?,
            then: boxed(g, model, &then)?,
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

    // ---- a custom `sh:SPARQLFunction` IRI applied to `( args… )` ----
    // SHACL-AF function-expression syntax: `[ <funcIRI> ( <arg> … ) ]` — the
    // blank node has exactly one predicate, the function IRI, whose object is the
    // argument list. Only a declared `sh:SPARQLFunction` is dispatched.
    parse_custom_function(g, model, node)
}

fn mk(op: Op) -> Option<Func> {
    Some(Func { op })
}

fn boxed(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<Box<NodeExpr>> {
    super::parse_node_expr(g, model, node).map(Box::new)
}

/// Parses a SHACL list of node expressions; `None` if any member is unsupported.
fn parse_list(g: &GraphView, model: &ShapesModel, head: &Term) -> Option<Vec<NodeExpr>> {
    g.list(head)
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
    /// Evaluates this function expression against `focus` over `graph`.
    pub(super) fn eval(
        &self,
        graph: &sparq_core::Graph,
        view: &GraphView,
        model: &ShapesModel,
        focus: &Term,
        scope: &crate::rules::Scope,
    ) -> Vec<Term> {
        match &self.op {
            Op::Concat(members) => members
                .iter()
                .flat_map(|m| m.eval(graph, view, model, focus, scope))
                .collect(),
            Op::Count(arg) => {
                let n = arg.eval(graph, view, model, focus, scope).len();
                vec![int_literal(n as i128)]
            }
            Op::Sum(arg) => {
                let vals = arg.eval(graph, view, model, focus, scope);
                vec![sum_literal(&vals)]
            }
            Op::MinMax { arg, max } => {
                let vals = arg.eval(graph, view, model, focus, scope);
                min_max(&vals, *max).into_iter().collect()
            }
            Op::Distinct(arg) => dedup(arg.eval(graph, view, model, focus, scope)),
            Op::Exists(arg) => {
                let nonempty = !arg.eval(graph, view, model, focus, scope).is_empty();
                vec![bool_literal(nonempty)]
            }
            Op::If { cond, then, els } => {
                if is_true_set(&cond.eval(graph, view, model, focus, scope)) {
                    then.eval(graph, view, model, focus, scope)
                } else if let Some(e) = els {
                    e.eval(graph, view, model, focus, scope)
                } else {
                    Vec::new()
                }
            }
            Op::Slice {
                nodes,
                limit,
                offset,
            } => {
                let v = nodes.eval(graph, view, model, focus, scope);
                let after_offset = v.into_iter().skip(*offset);
                match limit {
                    Some(k) => after_offset.take(*k).collect(),
                    None => after_offset.collect(),
                }
            }
            Op::InstancesOf(class) => view.instances_of(class),
            Op::NodesMatching(shape) => {
                // Every node in the graph that conforms to `shape`. The candidate
                // set is every subject and object of the data graph.
                candidate_nodes(view)
                    .into_iter()
                    .filter(|n| crate::eval::conforms_node(graph, model, *shape, n))
                    .collect()
            }
            Op::FlatMap { nodes, body } => nodes
                .eval(graph, view, model, focus, scope)
                .iter()
                .flat_map(|n| body.eval(graph, view, model, n, scope))
                .collect(),
            Op::FindFirst { nodes, shape } => nodes
                .eval(graph, view, model, focus, scope)
                .into_iter()
                .find(|n| crate::eval::conforms_node(graph, model, *shape, n))
                .into_iter()
                .collect(),
            Op::MatchAll { nodes, shape } => {
                let all = nodes
                    .eval(graph, view, model, focus, scope)
                    .iter()
                    .all(|n| crate::eval::conforms_node(graph, model, *shape, n));
                vec![bool_literal(all)]
            }
            Op::Remove { nodes, removed } => {
                let drop: std::collections::HashSet<Term> = removed
                    .eval(graph, view, model, focus, scope)
                    .into_iter()
                    .collect();
                nodes
                    .eval(graph, view, model, focus, scope)
                    .into_iter()
                    .filter(|n| !drop.contains(n))
                    .collect()
            }
            Op::OrderBy { nodes, key, desc } => {
                let mut v = nodes.eval(graph, view, model, focus, scope);
                // Sort by the least key value of each node; a node with no key
                // sorts first (ascending) per the suite's `orderBy-height`.
                v.sort_by(|a, b| {
                    let ka = key.eval(graph, view, model, a, scope);
                    let kb = key.eval(graph, view, model, b, scope);
                    let ord = cmp_key(&ka, &kb);
                    if *desc {
                        ord.reverse()
                    } else {
                        ord
                    }
                });
                v
            }
            // (sq-u5rxj) `var "name"`: `"focusNode"` is the focus node;
            // any other name resolves against the caller-supplied `scope` (the W3C
            // suite's `sht:scope-<name>` injection), or the empty set when absent.
            Op::Var(name) => {
                if name == "focusNode" {
                    vec![focus.clone()]
                } else {
                    scope.get(name).cloned().unwrap_or_default()
                }
            }
            Op::Sparql {
                select,
                params,
                args,
            } => self.eval_sparql_function(graph, view, model, focus, scope, select, params, args),
        }
    }

    /// Dispatches a custom `sh:SPARQLFunction`: evaluate each argument to a single
    /// node (a function argument is scalar), pre-bind the parameter variables via a
    /// VALUES row, run the SELECT, and return the `?result` column.
    #[allow(clippy::too_many_arguments)]
    fn eval_sparql_function(
        &self,
        graph: &sparq_core::Graph,
        view: &GraphView,
        model: &ShapesModel,
        focus: &Term,
        scope: &crate::rules::Scope,
        select: &str,
        params: &[String],
        args: &[NodeExpr],
    ) -> Vec<Term> {
        // Each argument must evaluate to exactly one node (SPARQL scalar binding);
        // if any is empty/multi-valued the function call is undefined ⇒ empty set.
        let mut bound: Vec<Term> = Vec::with_capacity(params.len());
        for a in args {
            let mut vs = a.eval(graph, view, model, focus, scope);
            if vs.len() != 1 {
                return Vec::new();
            }
            bound.push(vs.remove(0));
        }
        let Some(query) = build_function_query(select, params, &bound) else {
            return Vec::new();
        };
        // `result` is the conventional SHACL function return variable; collect its
        // bound column across all solution rows (deduplicated).
        let Ok(res) = sparq_engine::query(graph, &query) else {
            return Vec::new();
        };
        let Some(col) = res.vars.iter().position(|v| v.as_str() == "result") else {
            return Vec::new();
        };
        dedup(
            res.rows
                .iter()
                .filter_map(|row| row.get(col).and_then(|c| c.clone())),
        )
    }
}

/// Wraps a `sh:select` body so the parameter variables are pre-bound to the
/// evaluated arguments through a `VALUES` row, returning a runnable SELECT. `None`
/// if any argument has no SPARQL ground form (a blank node).
fn build_function_query(select: &str, params: &[String], args: &[Term]) -> Option<String> {
    // Strip an outer `SELECT … WHERE { … }` we cannot reliably; instead inject a
    // VALUES table before the final `}` of the body (the SELECT's WHERE group),
    // exactly as the rule CONSTRUCT path pre-binds `$this`.
    let vars: String = params.iter().map(|p| format!("?{p} ")).collect();
    let mut row = String::new();
    for a in args {
        let g = ground(a)?;
        row.push_str(&g);
        row.push(' ');
    }
    let close = select.rfind('}')?;
    let mut out = String::with_capacity(select.len() + vars.len() + row.len() + 32);
    out.push_str(&select[..close]);
    out.push_str(&format!(" VALUES ({vars}) {{ ({row}) }} "));
    out.push_str(&select[close..]);
    Some(out)
}

/// The SPARQL ground-term form of a node (for a VALUES row); `None` for a blank.
fn ground(t: &Term) -> Option<String> {
    match t {
        Term::NamedNode(n) => Some(format!("<{}>", n.as_str())),
        Term::Literal(l) => Some(l.to_string()),
        Term::BlankNode(_) => None,
        #[allow(unreachable_patterns)]
        _ => None,
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

/// The numerically least (or greatest) member; `None` if no numeric member.
fn min_max(vals: &[Term], max: bool) -> Option<Term> {
    let mut best: Option<(f64, Term)> = None;
    for v in vals {
        let Some(n) = numeric(v) else { continue };
        let take = match &best {
            None => true,
            Some((b, _)) => {
                if max {
                    n > *b
                } else {
                    n < *b
                }
            }
        };
        if take {
            best = Some((n, v.clone()));
        }
    }
    best.map(|(_, t)| t)
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

/// SPARQL ORDER BY-style comparison of two terms: numeric by value, otherwise by
/// lexical form (stable, total order so the sort is deterministic).
fn cmp_term(a: &Term, b: &Term) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if let (Some(na), Some(nb)) = (numeric(a), numeric(b)) {
        return na.partial_cmp(&nb).unwrap_or(Ordering::Equal);
    }
    lexical(a).cmp(&lexical(b))
}

fn lexical(t: &Term) -> String {
    match t {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::Literal(l) => l.value().to_string(),
        Term::BlankNode(b) => b.as_str().to_string(),
        #[allow(unreachable_patterns)]
        _ => String::new(),
    }
}
