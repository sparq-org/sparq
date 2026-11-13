//! Shared SHACL 1.2 node-expression evaluation for validation and rules.
use crate::model::{sh, ShapesModel};
use crate::path::Path;
use crate::view::GraphView;
use oxrdf::Term;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::Graph;
const SHNEX: &str = "http://www.w3.org/ns/shacl-node-expr#";
#[path = "func.rs"]
mod func;
#[path = "node_order.rs"]
mod order;
fn ne_object(g: &GraphView, subject: &Term, local: &str) -> Option<Term> {
    g.object(subject, &format!("{SHNEX}{local}"))
        .or_else(|| g.object(subject, &sh(local)))
}
/// Parsed node expressions shared by SHACL 1.2 evaluation and SHACL-AF rules.
#[derive(Debug, Clone)]
pub(crate) enum NodeExpr {
    /// `sh:this` — evaluates to `{ focus }`.
    This,
    /// A constant IRI or literal node — evaluates to `{ term }`.
    Constant(Term),
    /// A path node expression `[ sh:path P ; sh:nodes N ]`: the path-`P` value
    /// nodes of every node in `Eval(N, focus)` (`N` defaults to `sh:this`).
    Path {
        nodes: Box<NodeExpr>,
        path: Path,
        single: bool,
    },
    /// A filter shape expression `[ sh:filterShape S ; sh:nodes N ]`: the nodes
    /// of `Eval(N, focus)` that conform to shape `S` (a [`ShapesModel`] shape id).
    FilterShape { nodes: Box<NodeExpr>, shape: usize },
    /// `[ sh:intersection ( E1 E2 … ) ]`: set intersection of the members.
    Intersection(Vec<NodeExpr>),
    /// `[ sh:union ( E1 E2 … ) ]`: set union of the members.
    Union(Vec<NodeExpr>),
    /// A function expression — a registered SHACL-function operator applied to its
    /// (already-parsed) operands. The [`func::Func`] carries the operator and the
    /// argument node expressions / inline filter shapes it was parsed with.
    Func(Box<func::Func>),
    /// A SHACL 1.2 *list expression* — a bare `rdf:list` whose members are
    /// themselves node expressions; evaluates to their members in order,
    /// preserving duplicates (a sequence, not a set). `rdf:nil` ⇒ `{ rdf:nil }`.
    List(Vec<NodeExpr>),
}

/// String variable names mapped to output nodes for `shnex:var`.
/// `focusNode` always denotes the current focus. An absent name has no output.
pub type Scope = FxHashMap<String, Vec<Term>>;

/// Internal scope retains RDF argument keys separately from string variable names.
#[derive(Default)]
pub(super) struct EvalScope {
    variables: Scope,
    arguments: FxHashMap<Term, Term>,
}

thread_local! {
    static PARSE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl NodeExpr {
    #[cfg(feature = "shacl-af")]
    pub(crate) fn eval(
        &self,
        graph: &Graph,
        view: &GraphView,
        model: &ShapesModel,
        focus: &Term,
        scope: &Scope,
    ) -> Vec<Term> {
        match self.try_eval(
            graph,
            view,
            model,
            focus,
            &EvalScope {
                variables: scope.clone(),
                ..Default::default()
            },
        ) {
            Ok(nodes) => nodes,
            Err(_) => {
                crate::functions::record_failure(focus);
                Vec::new()
            }
        }
    }

    pub(super) fn try_eval(
        &self,
        graph: &Graph,
        view: &GraphView,
        model: &ShapesModel,
        focus: &Term,
        scope: &EvalScope,
    ) -> Result<Vec<Term>, String> {
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                DEPTH.with(|d| d.set(d.get() - 1));
            }
        }
        if DEPTH.with(|d| d.get() >= 64) {
            return Err("Recursive node expression".into());
        }
        DEPTH.with(|d| d.set(d.get() + 1));
        let _guard = Guard;
        Ok(match self {
            Self::This => vec![focus.clone()],
            Self::Constant(t) => vec![t.clone()],
            Self::Path {
                nodes,
                path,
                single,
            } => {
                let starts = nodes.try_eval(graph, view, model, focus, scope)?;
                if *single && starts.len() > 1 {
                    return Err("Path focus must have at most one node".into());
                }
                crate::view::dedup(starts.iter().flat_map(|s| path.values(view, s)))
            }
            Self::FilterShape { nodes, shape } => {
                let mut out = Vec::new();
                for n in nodes.try_eval(graph, view, model, focus, scope)? {
                    if conforms(graph, model, *shape, &n)? {
                        out.push(n);
                    }
                }
                out
            }
            Self::Intersection(members) => {
                let mut sets = Vec::new();
                for m in members {
                    sets.push(m.try_eval(graph, view, model, focus, scope)?);
                }
                if sets.is_empty() {
                    return Ok(Vec::new());
                }
                let mut acc = crate::view::dedup(sets.remove(0));
                for set in sets {
                    let other: FxHashSet<_> = set.into_iter().collect();
                    acc.retain(|t| other.contains(t));
                }
                acc
            }
            Self::Union(members) | Self::List(members) => {
                let mut out = Vec::new();
                for m in members {
                    out.extend(m.try_eval(graph, view, model, focus, scope)?);
                }
                if matches!(self, Self::Union(_)) {
                    crate::view::dedup(out)
                } else {
                    out
                }
            }
            Self::Func(f) => f.eval(graph, view, model, focus, scope)?,
        })
    }
}

pub(super) fn conforms(
    graph: &Graph,
    model: &ShapesModel,
    shape: usize,
    node: &Term,
) -> Result<bool, String> {
    if !model.ill_formed.is_empty() || !model.pre_binding_failures.is_empty() {
        return Err("Shape validation failed".into());
    }
    crate::functions::check_failures(|| crate::eval::conforms_node(graph, model, shape, node))
}

pub(crate) fn eval_function_body(
    model: &ShapesModel,
    data: &Graph,
    expr: &NodeExpr,
    focus: &Term,
    args: &[Term],
) -> Result<Vec<Term>, String> {
    let scope = EvalScope {
        variables: Scope::default(),
        arguments: args
            .iter()
            .enumerate()
            .map(|(i, t)| (oxrdf::Literal::from(i as i64).into(), t.clone()))
            .collect(),
    };
    expr.try_eval(data, &GraphView::new(data), model, focus, &scope)
}

/// Parses a SHACL-AF node expression value (the full algebra — see [`NodeExpr`]).
/// `None` for an unsupported / ill-formed blank-node expression (e.g. a function
/// expression, or a filter shape whose shape is not in the model) — skipped.
pub(crate) fn parse_node_expr(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<NodeExpr> {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            PARSE_DEPTH.with(|d| d.set(d.get() - 1));
        }
    }
    if PARSE_DEPTH.with(|d| d.get() >= 64) {
        return None;
    }
    PARSE_DEPTH.with(|d| d.set(d.get() + 1));
    let _guard = Guard;
    match node {
        Term::NamedNode(n) if n.as_str() == sh("this") => Some(NodeExpr::This),
        Term::NamedNode(_) | Term::Literal(_) => Some(NodeExpr::Constant(node.clone())),
        Term::BlankNode(_) => parse_blank_node_expr(g, model, node),
        Term::Triple(_) => Some(NodeExpr::Constant(node.clone())),
    }
}

/// Parses a blank-node node expression: path / filter-shape / intersection /
/// union. The `sh:nodes` of a path or filter expression is itself a node
/// expression (defaulting to `sh:this` for a path), so these nest.
fn parse_blank_node_expr(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<NodeExpr> {
    // (sq-mk9n) A bare `rdf:list` blank node is a SHACL 1.2 *list
    // expression* — its members are themselves node expressions. (Checked first:
    // a list head carries `rdf:first`, which no operator/filter expression does.)
    if g.object(node, crate::view::RDF_FIRST).is_some() {
        let members: Vec<NodeExpr> = g
            .list_strict(node)?
            .iter()
            .map(|m| parse_node_expr(g, model, m))
            .collect::<Option<_>>()?;
        return Some(NodeExpr::List(members));
    }
    // An empty blank node `[]` (no predicates) is the EMPTY node expression.
    if g.predicate_objects(node).is_empty() {
        return Some(NodeExpr::List(Vec::new()));
    }
    // Intersection / union: a SHACL list of member node expressions
    // (`shnex:`/`sh:` spelling, sq-mue75).
    for (pred, is_union) in [("intersection", false), ("union", true)] {
        if let Some(list_head) = ne_object(g, node, pred) {
            let members: Vec<NodeExpr> = g
                .list_strict(&list_head)?
                .iter()
                .map(|m| parse_node_expr(g, model, m))
                .collect::<Option<_>>()?;
            return Some(if is_union {
                NodeExpr::Union(members)
            } else {
                NodeExpr::Intersection(members)
            });
        }
    }
    // Filter shape: [ shnex:filterShape S ; shnex:nodes N ] (or `sh:` spelling).
    // The shape must be parsed into the model (typed sh:NodeShape / referenced) —
    // exactly as sh:condition.
    if let Some(shape_term) = ne_object(g, node, "filterShape") {
        let shape = model.by_node(&shape_term)?;
        let nodes = parse_nodes_input(g, model, node)?;
        return Some(NodeExpr::FilterShape {
            nodes: Box::new(nodes),
            shape,
        });
    }
    // Path-values: the SHACL 1.2 `[ shnex:pathValues P ; shnex:focusNode N? ]` and
    // the SHACL-AF `[ sh:path P ; sh:nodes N? ]` both evaluate a property path P
    // from a start-node expression (defaulting to `sh:this`) (sq-mue75).
    if let Some(p) = ne_object(g, node, "pathValues").or_else(|| g.object(node, &sh("path"))) {
        let path = Path::parse(g, &p).ok()?;
        let nodes = parse_nodes_input(g, model, node)?;
        return Some(NodeExpr::Path {
            nodes: Box::new(nodes),
            path,
            single: ne_object(g, node, "pathValues").is_some(),
        });
    }
    // Function expression: a registered SHACL-function operator (`shnex:`/`sh:`
    // built-in, or a custom `sh:SPARQLFunction` IRI) applied to its operands. An
    // unregistered function IRI / ill-formed operand list yields `None` (dropped).
    func::parse(g, model, node).map(|f| NodeExpr::Func(Box::new(f)))
}

/// The input/start node expression of a path / filter expression, defaulting to
/// the focus-node expression (`sh:this`) when absent. Accepts the SHACL 1.2
/// `shnex:nodes` (filter input) / `shnex:focusNode` (path start) and the SHACL-AF
/// `sh:nodes` spellings (sq-mue75).
fn parse_nodes_input(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<NodeExpr> {
    match ne_object(g, node, "nodes").or_else(|| ne_object(g, node, "focusNode")) {
        Some(n) => parse_node_expr(g, model, &n),
        None => Some(NodeExpr::This),
    }
}

/// Evaluates `expr` from `shapes` against `focus` in `data`.
/// Output order and duplicates are preserved, except for set-valued operators.
/// Returns `None` for an unsupported/ill-formed expression or an evaluation
/// failure. Use [`eval_node_expression_result`] to retain failure diagnostics.
/// Available with or without `shacl-af`; rule execution still requires that feature.
pub fn eval_node_expression(
    data: &Graph,
    shapes: &Graph,
    expr: &Term,
    focus: &Term,
) -> Option<Vec<Term>> {
    eval_node_expression_with_scope(data, shapes, expr, focus, &Scope::default())
}

/// [`eval_node_expression`] with string-named scope variables.
pub fn eval_node_expression_with_scope(
    data: &Graph,
    shapes: &Graph,
    expr: &Term,
    focus: &Term,
    scope: &Scope,
) -> Option<Vec<Term>> {
    eval_node_expression_result(data, shapes, expr, focus, scope).ok()
}

/// Evaluates a node expression, preserving the distinction between an empty
/// output and an evaluation failure. Available with or without `shacl-af`.
/// `scope` supplies string-named variables; custom function argument keys are local.
pub fn eval_node_expression_result(
    data: &Graph,
    shapes: &Graph,
    expr: &Term,
    focus: &Term,
    scope: &Scope,
) -> Result<Vec<Term>, String> {
    let mut model = ShapesModel::parse(shapes);
    model.register_node_expr_shapes(shapes, expr);
    for [_, _, body] in GraphView::new(shapes).triples(None, Some(&sh("bodyExpression")), None) {
        model.register_node_expr_shapes(shapes, &body);
    }
    let parsed = parse_node_expr(&GraphView::new(shapes), &model, expr)
        .ok_or("Unsupported or ill-formed node expression")?;
    crate::functions::with_validation(data, shapes, || {
        parsed.try_eval(
            data,
            &GraphView::new(data),
            &model,
            focus,
            &EvalScope {
                variables: scope.clone(),
                ..Default::default()
            },
        )
    })
    .0
}

/// (sq-mk9n) Evaluates a parsed node expression `expr` against a value
/// node `value` over the data `graph` (with `model` for filter-shape conformance),
/// returning its result node set. The `sh:expression` constraint (in `eval.rs`)
/// uses this to test whether an expression evaluates to `{ true }` for a value.
#[cfg(feature = "shacl-af")]
pub(crate) fn eval_parsed_expr(
    graph: &Graph,
    model: &ShapesModel,
    expr: &NodeExpr,
    value: &Term,
) -> Vec<Term> {
    let view = GraphView::new(graph);
    expr.eval(graph, &view, model, value, &Scope::default())
}

/// True iff a node-expression result set is exactly `{ xsd:boolean "true" }` — the
/// SHACL-AF `sh:expression` "satisfied" condition.
#[cfg(feature = "shacl-af")]
pub(crate) fn is_true_result(set: &[Term]) -> bool {
    set.len() == 1
        && matches!(&set[0], Term::Literal(l)
            if l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#boolean"
                && l.value() == "true")
}
