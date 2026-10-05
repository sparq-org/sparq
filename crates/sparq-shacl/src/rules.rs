//! [OPUS-4.8] (sq-d1dw) SHACL Advanced Features (SHACL-AF) **rules** — `sh:rule`.
//!
//! OPT-IN: this whole module is gated behind the `shacl-af` cargo feature, so the
//! base SHACL Core + SHACL-SPARQL validation path carries zero rule code or parse
//! cost when the feature is off.
//!
//! Three rule types from the W3C SHACL-AF Recommendation are supported:
//!
//!   * **`sh:TripleRule`** (§4.2) — carries `sh:subject` / `sh:predicate` /
//!     `sh:object` node expressions. For each focus node, the inferred triples are
//!     the cartesian product `S × P × O` of the three evaluated node-expression
//!     sets.
//!   * **`sh:SPARQLRule`** (§4.3) — carries an `sh:construct` CONSTRUCT query (with
//!     optional `sh:prefixes`). For each focus node, the query runs with `$this`
//!     pre-bound to the focus node and the constructed triples are inferred. This
//!     reuses the very SPARQL engine the `sh:sparql` constraint path already
//!     depends on (here through `sparq_engine::construct`).
//!   * **`sh:values`** value rule (SHACL-AF *Property value rules*) — a property
//!     shape whose `sh:path` is a single predicate IRI and which carries an
//!     `sh:values` node expression infers, for each focus node `n` and each `v` in
//!     `Eval(values-expr, n)`, the triple `(n, predicate, v)`. (A non-predicate
//!     path has no inferable subject/predicate form, so such a shape is skipped.)
//!
//! ## Node-expression algebra (SHACL-AF *Node Expressions*)
//!
//! `sh:subject`/`sh:predicate`/`sh:object` (TripleRule) and `sh:values` (value
//! rule) take **node expressions**, evaluated against a focus node `$this` to a
//! set of nodes. The full algebra is implemented (`NodeExpr`):
//!
//!   * **focus** — the IRI `sh:this` ⇒ `{ $this }`.
//!   * **constant** — any other IRI / literal ⇒ `{ term }`.
//!   * **path** — `[ sh:path P ]` with optional `sh:nodes` (a *nested* node
//!     expression giving the start nodes; defaults to `sh:this`) ⇒ the union of
//!     the path-`P` value nodes of every start node.
//!   * **filter shape** — `[ sh:filterShape S ; sh:nodes N ]` ⇒ the nodes of `N`
//!     that **conform** to shape `S` (evaluated through the constraint validator,
//!     exactly like `sh:condition`).
//!   * **intersection** — `[ sh:intersection ( E1 E2 … ) ]` ⇒ set intersection of
//!     each member's nodes (order-preserving, deduplicated).
//!   * **union** — `[ sh:union ( E1 E2 … ) ]` ⇒ set union of each member's nodes.
//!
//! These nest arbitrarily (a path's `sh:nodes`, a filter's `sh:nodes`, and the
//! intersection/union members are themselves node expressions). The **function**
//! expression form (a SHACL function IRI applied to an argument list) is NOT
//! supported here — it needs a SHACL-function registry the crate does not carry;
//! an unrecognised blank-node expression is dropped (lenient), so a rule that uses
//! one simply infers nothing rather than misfiring.
//!
//! Both rule types honour:
//!   * **`sh:condition`** (§4.1) — a rule only fires for focus nodes that conform
//!     to *every* condition shape (evaluated through the existing constraint
//!     validator: a focus node conforms iff it produces no validation results).
//!   * **`sh:order`** (§4.1) — rules run in ascending numeric order (default `0`);
//!     a rule sees the triples inferred by every *earlier* order group.
//!   * **`sh:deactivated`** — deactivated rules are ignored.
//!
//! ## Execution model & fixpoint
//!
//! The normative SHACL-AF algorithm runs each order group once, with triples from
//! one group visible to later groups. This crate runs the whole shape/order
//! schedule and then, because triple/SPARQL rules can feed each other across
//! shapes (a rule's output can satisfy another rule's `sh:condition` or match
//! another rule's CONSTRUCT/path), **iterates the schedule to a fixpoint**: it
//! repeats until a pass infers no new triple, bounded by [`MAX_ITERATIONS`] so an
//! ill-behaved rule set always terminates. The returned set is the *inferred*
//! triples only (the input data graph is never mutated); [`apply_rules`] returns
//! that set, and [`expand`] returns a fresh graph of data ∪ inferred.

use crate::model::{sh, ShapesModel};
use crate::path::Path;
use crate::view::GraphView;
use oxrdf::{NamedNode, NamedOrBlankNode, Term, Triple};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::Graph;

/// [OPUS-4.8] (sq-mue75) The SHACL 1.2 node-expression namespace. The SHACL-AF
/// public API the crate's parser was originally written against spells the
/// node-expression operators in the `sh:` namespace, but the W3C SHACL 1.2
/// node-expression vocabulary (and its conformance suite) uses `shnex:`. The
/// blank-node parser below accepts BOTH spellings of the shared algebra so the
/// real evaluation path drives the conformance suite directly (no harness-side
/// re-implementation). The function-operator registry (`func.rs`) already does
/// this via its own `op_object`.
const SHNEX: &str = "http://www.w3.org/ns/shacl-node-expr#";

/// The single object of `subject <ns#local>`, trying the SHACL 1.2 `shnex:`
/// spelling first then the SHACL-AF `sh:` spelling (the two share local names for
/// the shared node-expression algebra).
fn ne_object(g: &GraphView, subject: &Term, local: &str) -> Option<Term> {
    g.object(subject, &format!("{SHNEX}{local}"))
        .or_else(|| g.object(subject, &sh(local)))
}

// [OPUS-4.8] (sq-mk9n) The SHACL-function registry + the function-expression form
// ([`NodeExpr::Func`]). A child module so its (sizeable) operator table stays out
// of this file; it reaches the node-expression machinery through `super::`.
#[path = "func.rs"]
mod func;

/// Upper bound on fixpoint iterations of the rule schedule. SHACL-AF leaves
/// multi-pass entailment to "external orchestration"; we iterate until a pass
/// adds nothing, but cap it so a pathological rule set (e.g. one that mints a
/// fresh blank node each pass) always terminates rather than looping forever.
pub const MAX_ITERATIONS: usize = 100;

/// A parsed SHACL-AF rule attached to a shape.
#[derive(Debug, Clone)]
struct Rule {
    kind: RuleKind,
    /// `sh:order` (default 0.0), the schedule key (ascending).
    order: f64,
    /// `sh:condition` shape ids (into [`ShapesModel::shapes`]); the rule fires for
    /// a focus node only if it conforms to every one.
    conditions: Vec<usize>,
    deactivated: bool,
}

#[derive(Debug, Clone)]
enum RuleKind {
    /// `sh:TripleRule`: the three node expressions for subject/predicate/object.
    Triple {
        subject: NodeExpr,
        predicate: NodeExpr,
        object: NodeExpr,
    },
    /// `sh:SPARQLRule`: the (prefix-prepended) CONSTRUCT query text. Parsed per
    /// run via the engine (CONSTRUCT is a graph-form query, not a SELECT, so it
    /// does not go through the SELECT pre-binding path; `$this` is injected as a
    /// `VALUES` table textually-safe at parse time via the algebra).
    Sparql { construct: String },
    /// `sh:values` value rule: a property shape with a single-predicate `sh:path`
    /// and an `sh:values` node expression. For each focus node `n`, infers
    /// `(n, predicate, v)` for every `v` in `Eval(values, n)`.
    Values {
        predicate: NamedNode,
        values: NodeExpr,
    },
}

/// A SHACL-AF node expression (SHACL-AF *Node Expressions*), evaluated against a
/// focus node to a *set* of nodes. The full algebra is supported, including the
/// **function-expression form** ([`NodeExpr::Func`]) backed by the SHACL-function
/// registry ([`func`]): the SHACL 1.2 built-in node-expression operators
/// (`concat`/`sum`/`count`/`if`/`distinct`/`min`/`max`/`exists`/`limit`/`offset`/
/// `instancesOf`/`flatMap`/`findFirst`/`matchAll`/`remove`/`orderBy`/
/// `nodesMatching`/`var`) and a custom `sh:SPARQLFunction` IRI applied to a SHACL
/// list of argument node expressions. A function IRI with no registered
/// implementation is dropped leniently (so a rule using one infers nothing rather
/// than misfiring) — see the module docs.
#[derive(Debug, Clone)]
pub(crate) enum NodeExpr {
    /// `sh:this` — evaluates to `{ focus }`.
    This,
    /// A constant IRI or literal node — evaluates to `{ term }`.
    Constant(Term),
    /// A path node expression `[ sh:path P ; sh:nodes N ]`: the path-`P` value
    /// nodes of every node in `Eval(N, focus)` (`N` defaults to `sh:this`).
    Path { nodes: Box<NodeExpr>, path: Path },
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

/// [OPUS-4.8] (sq-u5rxj) A caller-supplied node-expression VARIABLE SCOPE: a map
/// of variable name → bound node set, consulted by `shnex:var "<name>"`
/// (`func::Op::Var`). `"focusNode"` is always the focus node and need not appear
/// here; any other name resolves against this scope (empty when absent). The W3C
/// node-expr suite injects `sht:scope-<name>` bindings the harness threads in.
pub type Scope = FxHashMap<String, Vec<Term>>;

impl NodeExpr {
    /// The node-set this expression evaluates to for `focus` over `data`. Filter
    /// shape expressions need the parsed `model` (to run the constraint validator
    /// for conformance) and the owning `Graph` (the validator is graph-, not
    /// view-, oriented) — both threaded through. `scope` carries any
    /// caller-supplied `shnex:var` bindings (besides `"focusNode"`).
    pub(crate) fn eval(
        &self,
        graph: &Graph,
        view: &GraphView,
        model: &ShapesModel,
        focus: &Term,
        scope: &Scope,
    ) -> Vec<Term> {
        match self {
            NodeExpr::This => vec![focus.clone()],
            NodeExpr::Constant(t) => vec![t.clone()],
            NodeExpr::Path { nodes, path } => {
                let starts = nodes.eval(graph, view, model, focus, scope);
                crate::view::dedup(starts.iter().flat_map(|s| path.values(view, s)))
            }
            NodeExpr::FilterShape { nodes, shape } => nodes
                .eval(graph, view, model, focus, scope)
                .into_iter()
                .filter(|n| crate::eval::conforms_node(graph, model, *shape, n))
                .collect(),
            NodeExpr::Intersection(members) => {
                let mut iter = members.iter();
                // Empty intersection ⇒ empty (no constraining set); a degenerate
                // single-member intersection is just that member.
                let Some(first) = iter.next() else {
                    return Vec::new();
                };
                let mut acc = crate::view::dedup(first.eval(graph, view, model, focus, scope));
                for m in iter {
                    let other: FxHashSet<Term> = m
                        .eval(graph, view, model, focus, scope)
                        .into_iter()
                        .collect();
                    acc.retain(|t| other.contains(t));
                    if acc.is_empty() {
                        break;
                    }
                }
                acc
            }
            NodeExpr::Union(members) => crate::view::dedup(
                members
                    .iter()
                    .flat_map(|m| m.eval(graph, view, model, focus, scope)),
            ),
            NodeExpr::Func(f) => f.eval(graph, view, model, focus, scope),
            NodeExpr::List(members) => members
                .iter()
                .flat_map(|m| m.eval(graph, view, model, focus, scope))
                .collect(),
        }
    }
}

/// All rules of the shapes graph, grouped per owning shape and sorted into the
/// execution schedule. Built once (behind the feature) alongside the model.
struct RuleSet {
    /// `(shape_id, rule)` pairs, sorted by `rule.order` ascending so the schedule
    /// is a single left-to-right pass over order groups.
    scheduled: Vec<(usize, Rule)>,
}

impl RuleSet {
    /// Parses every `sh:rule` of every shape in `model` from the shapes graph
    /// `shapes`, plus every `sh:values` value rule on a property shape. A rule
    /// applies to its owning shape's focus nodes (its targets).
    fn parse(shapes: &Graph, model: &ShapesModel) -> RuleSet {
        let g = GraphView::new(shapes);
        let mut scheduled: Vec<(usize, Rule)> = Vec::new();
        for sid in 0..model.shapes.len() {
            let shape_node = model.shapes[sid].node.clone();
            for rule_node in g.objects(&shape_node, &sh("rule")) {
                if let Some(rule) = parse_rule(&g, model, &rule_node) {
                    scheduled.push((sid, rule));
                }
            }
            // `sh:values` value rule (SHACL-AF): directly on a property shape, NOT
            // a `sh:rule` value. Needs a single-predicate `sh:path`; the values
            // node expression is read from the same shape node.
            if let Some(rule) = parse_values_rule(&g, model, sid, &shape_node) {
                // Focus selection: a property shape with its OWN targets fires for
                // those; otherwise (the common `sh:property` child case) it fires
                // for every targeted node shape that references it as a child, so
                // the rule sees the parent's focus nodes (SHACL-AF property value
                // rules attach to the property shape but range over focus nodes).
                if !model.shapes[sid].targets.is_empty() {
                    scheduled.push((sid, rule));
                } else {
                    for parent in parents_of(model, sid) {
                        if !model.shapes[parent].targets.is_empty() {
                            scheduled.push((parent, rule.clone()));
                        }
                    }
                }
            }
        }
        // Stable sort by order keeps same-order rules in shapes-graph discovery
        // order (the spec leaves intra-group order unobservable — same-order rules
        // are mutually invisible within a single pass — so any stable order is fine).
        scheduled.sort_by(|a, b| {
            a.1.order
                .partial_cmp(&b.1.order)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        RuleSet { scheduled }
    }
}

/// Parses one rule node. `None` for a deactivated-irrelevant ill-formed rule
/// (e.g. a `sh:SPARQLRule` with no `sh:construct`, or a `sh:TripleRule` missing a
/// node expression) — ill-formed rules are skipped, matching this crate's lenient
/// handling of ill-formed shapes.
fn parse_rule(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<Rule> {
    let order = match g.object(node, &sh("order")) {
        Some(Term::Literal(l)) => l.value().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };
    let deactivated = matches!(
        g.object(node, &sh("deactivated")),
        Some(Term::Literal(l)) if l.value() == "true"
    );
    // sh:condition shapes → ids into the already-parsed model (a condition that
    // names an unknown shape is dropped; it cannot gate anything meaningfully).
    let conditions: Vec<usize> = g
        .objects(node, &sh("condition"))
        .iter()
        .filter_map(|c| model.by_node(c))
        .collect();

    // Rule kind: a sh:construct ⇒ SPARQLRule; sh:subject/predicate/object ⇒ TripleRule.
    let kind = if let Some(Term::Literal(l)) = g.object(node, &sh("construct")) {
        let prefixes = collect_rule_prefixes(g, node);
        let construct = format!("{prefixes}\n{}", l.value());
        RuleKind::Sparql { construct }
    } else {
        let subject = parse_node_expr(g, model, &g.object(node, &sh("subject"))?)?;
        let predicate = parse_node_expr(g, model, &g.object(node, &sh("predicate"))?)?;
        let object = parse_node_expr(g, model, &g.object(node, &sh("object"))?)?;
        RuleKind::Triple {
            subject,
            predicate,
            object,
        }
    };
    Some(Rule {
        kind,
        order,
        conditions,
        deactivated,
    })
}

/// Parses a `sh:values` value rule on property shape `sid` (SHACL-AF property
/// value rules). Requires a single-predicate `sh:path` (only a predicate path
/// has an inferable subject/predicate form) and an `sh:values` node expression.
/// `None` when the shape carries no `sh:values`, has a non-predicate path, or the
/// values expression is unsupported (skipped — lenient). `sh:order` /
/// `sh:deactivated` / `sh:condition` on the property shape itself are honoured.
fn parse_values_rule(
    g: &GraphView,
    model: &ShapesModel,
    sid: usize,
    shape_node: &Term,
) -> Option<Rule> {
    let values_term = g.object(shape_node, &sh("values"))?;
    let predicate = match &model.shapes[sid].path {
        Some(Path::Predicate(p)) => NamedNode::new(p).ok()?,
        _ => return None, // a non-predicate (or absent) path cannot be inferred
    };
    let values = parse_node_expr(g, model, &values_term)?;
    let order = match g.object(shape_node, &sh("order")) {
        Some(Term::Literal(l)) => l.value().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };
    let conditions: Vec<usize> = g
        .objects(shape_node, &sh("condition"))
        .iter()
        .filter_map(|c| model.by_node(c))
        .collect();
    Some(Rule {
        kind: RuleKind::Values { predicate, values },
        order,
        conditions,
        deactivated: model.shapes[sid].deactivated,
    })
}

/// Parses a SHACL-AF node expression value (the full algebra — see [`NodeExpr`]).
/// `None` for an unsupported / ill-formed blank-node expression (e.g. a function
/// expression, or a filter shape whose shape is not in the model) — skipped.
pub(crate) fn parse_node_expr(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<NodeExpr> {
    match node {
        Term::NamedNode(n) if n.as_str() == sh("this") => Some(NodeExpr::This),
        Term::NamedNode(_) | Term::Literal(_) => Some(NodeExpr::Constant(node.clone())),
        Term::BlankNode(_) => parse_blank_node_expr(g, model, node),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Parses a blank-node node expression: path / filter-shape / intersection /
/// union. The `sh:nodes` of a path or filter expression is itself a node
/// expression (defaulting to `sh:this` for a path), so these nest.
fn parse_blank_node_expr(g: &GraphView, model: &ShapesModel, node: &Term) -> Option<NodeExpr> {
    // [OPUS-4.8] (sq-mk9n) A bare `rdf:list` blank node is a SHACL 1.2 *list
    // expression* — its members are themselves node expressions. (Checked first:
    // a list head carries `rdf:first`, which no operator/filter expression does.)
    if g.object(node, crate::view::RDF_FIRST).is_some() {
        let members: Vec<NodeExpr> = g
            .list(node)
            .iter()
            .filter_map(|m| parse_node_expr(g, model, m))
            .collect();
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
                .list(&list_head)
                .iter()
                .filter_map(|m| parse_node_expr(g, model, m))
                .collect();
            if members.is_empty() {
                return None; // an empty/all-unparsable set is degenerate — skip
            }
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

/// Assembles SPARQL `PREFIX` declarations from a rule node's `sh:prefixes`
/// (SHACL-AF reuses the SHACL-SPARQL `sh:prefixes` mechanism). Delegates to the
/// model's shared prefix collector via a tiny re-walk here (the collector is
/// crate-private to model.rs; we mirror the simple direct-declare case and the
/// owl:imports chase by reusing GraphView).
fn collect_rule_prefixes(g: &GraphView, node: &Term) -> String {
    crate::model::collect_prefixes_for(g, &g.objects(node, &sh("prefixes")))
}

/// The result of running the rule engine: the inferred triples (a set), plus the
/// number of fixpoint iterations actually performed (for diagnostics/tests).
#[derive(Debug, Clone, Default)]
pub struct Inference {
    /// The inferred triples, in stable first-seen order (deduplicated).
    pub triples: Vec<Triple>,
    /// Fixpoint iterations performed (1 = the schedule reached a fixpoint after a
    /// single pass; higher = later passes inferred more before settling).
    pub iterations: usize,
    /// True iff the [`MAX_ITERATIONS`] cap was hit before a fixpoint (the inferred
    /// set may be incomplete — an ill-behaved, non-terminating rule set).
    pub capped: bool,
}

/// Runs the SHACL-AF rule engine: parses every `sh:rule` of `shapes`/`model` and
/// applies them to a fixpoint, returning the **inferred** triples (the input
/// `data` is never mutated). Rules fire only for focus nodes conforming to every
/// `sh:condition`, run in ascending `sh:order`, and are iterated until a pass adds
/// nothing (bounded by [`MAX_ITERATIONS`]).
///
/// This is the SHACL-AF (`shacl-af` feature) entry point; it has no effect on —
/// and is independent of — the base [`crate::validate`] path.
pub fn apply_rules(data: &Graph, shapes: &Graph) -> Inference {
    let model = ShapesModel::parse(shapes);
    apply_rules_with_model(data, shapes, &model)
}

/// [`apply_rules`] against an already-parsed shapes model (amortises shape parsing
/// across many data graphs). `shapes` is still required because rule node
/// expressions / CONSTRUCT prefixes are read directly from the shapes graph.
pub fn apply_rules_with_model(data: &Graph, shapes: &Graph, model: &ShapesModel) -> Inference {
    run_rules(data, &[], shapes, model)
}

/// The rule fixpoint over the working default graph `data`, with `named` exposed
/// as the ONLY named graphs a `sh:SPARQLRule` body can reach (`GRAPH <g>` /
/// `GRAPH ?g` / `FROM NAMED`). Inferred triples feed later passes through the
/// working default graph. The legacy entry points pass no named graphs.
fn run_rules(
    data: &Graph,
    named: &[(Term, Graph)],
    shapes: &Graph,
    model: &ShapesModel,
) -> Inference {
    let ruleset = RuleSet::parse(shapes, model);
    if ruleset.scheduled.is_empty() {
        return Inference::default();
    }

    // The accumulated inferred-triple set (dedup + stable order).
    let mut inferred: Vec<Triple> = Vec::new();
    let mut inferred_set: FxHashSet<Triple> = FxHashSet::default();

    let mut iterations = 0;
    let mut capped = false;
    loop {
        iterations += 1;
        // Each pass runs over data ∪ inferred-so-far. Build the augmented graph
        // once per pass (cheap relative to the rule SPARQL/conformance work).
        let mut augmented = expand_graph(data, &inferred);
        augmented.named = named.iter().map(|(n, g)| (n.clone(), g.fork())).collect();
        let view = GraphView::new(&augmented);
        let mut pass_new = false;

        for (sid, rule) in &ruleset.scheduled {
            if rule.deactivated {
                continue;
            }
            for focus in focus_nodes(&view, model, *sid) {
                if !conforms_to_all(&augmented, model, &rule.conditions, &focus) {
                    continue;
                }
                for triple in fire_rule(&augmented, &view, model, &focus, rule) {
                    if inferred_set.insert(triple.clone()) {
                        inferred.push(triple);
                        pass_new = true;
                    }
                }
            }
        }

        if !pass_new {
            break;
        }
        if iterations >= MAX_ITERATIONS {
            capped = true;
            break;
        }
    }

    Inference {
        triples: inferred,
        iterations,
        capped,
    }
}

/// Convenience: the data graph **expanded** with every inferred triple — a fresh
/// `Graph` of `data ∪ apply_rules(data, shapes)` (the input is not mutated).
pub fn expand(data: &Graph, shapes: &Graph) -> Graph {
    let inf = apply_rules(data, shapes);
    expand_graph(data, &inf.triples)
}

/// (gh-6614) Which asserted graphs of a dataset the SHACL-AF rule engine sees.
///
/// The selected graphs are merged into the **working default graph** that target
/// selection, paths, `sh:condition`, node expressions and `sh:SPARQLRule` bodies
/// evaluate against. The selected *named* graphs are additionally exposed as the
/// only named graphs a SPARQL rule body can reach (`GRAPH <g>` / `GRAPH ?g`); an
/// unselected graph is indistinguishable from an absent one, and a query dataset
/// clause (`FROM` / `FROM NAMED`) cannot widen the boundary. Nothing is ever
/// unioned implicitly: including the default graph in a union is explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphScope {
    /// The default graph only — the same view as [`apply_rules`].
    Default,
    /// One named graph (an absent name denotes the empty graph).
    Named(Term),
    /// The merge of the listed named graphs, plus the default graph iff `default`.
    Union {
        /// Whether the default graph is part of the union.
        default: bool,
        /// The named graphs in the union (duplicates are ignored; an absent name
        /// contributes nothing).
        named: Vec<Term>,
    },
}

/// (gh-6614) Where [`expand_dataset`] materializes the derived triples. The
/// destination only affects the returned dataset: during the fixpoint, inferred
/// triples always feed later passes through the working scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Into the default graph.
    Default,
    /// Into this named graph (merged with it if the dataset already has it,
    /// otherwise appended as a new named graph).
    Named(Term),
}

/// (gh-6614) The result of [`expand_dataset`]: the expanded dataset plus the
/// rule-engine diagnostics (`iterations`, `capped`) and the inferred triples.
pub struct DatasetExpansion {
    /// The input dataset with every asserted graph preserved and the derived
    /// triples added to the chosen [`Destination`] (deduplicated there).
    pub dataset: Graph,
    /// The inference run that produced the derived triples.
    pub inference: Inference,
}

/// (gh-6614) [`apply_rules`] over an explicit [`GraphScope`] of a dataset (a
/// `Graph` with named graphs). `GraphScope::Default` is equivalent to
/// [`apply_rules`]. The input is never mutated.
pub fn apply_rules_in_scope(data: &Graph, shapes: &Graph, scope: &GraphScope) -> Inference {
    let model = ShapesModel::parse(shapes);
    apply_rules_in_scope_with_model(data, shapes, &model, scope)
}

/// (gh-6614) [`apply_rules_in_scope`] against an already-parsed shapes model.
pub fn apply_rules_in_scope_with_model(
    data: &Graph,
    shapes: &Graph,
    model: &ShapesModel,
    scope: &GraphScope,
) -> Inference {
    match scope {
        GraphScope::Default => apply_rules_with_model(data, shapes, model),
        _ => {
            let (working, named) = scoped_working_graph(data, scope);
            run_rules(&working, &named, shapes, model)
        }
    }
}

/// (gh-6614) Dataset-preserving SHACL-AF expansion: runs the rules over `scope`
/// and returns a fresh copy of the WHOLE input dataset — every asserted named
/// graph, selected or not, is preserved — with the derived triples materialized
/// into `destination` (deduplicated against what that graph already asserts; an
/// identical triple asserted in another graph is left alone). The input is not
/// mutated, and the fixpoint diagnostics are returned rather than hidden.
pub fn expand_dataset(
    data: &Graph,
    shapes: &Graph,
    scope: &GraphScope,
    destination: &Destination,
) -> DatasetExpansion {
    let inference = apply_rules_in_scope(data, shapes, scope);
    let copy_named = |skip: Option<&Term>| -> Vec<(Term, Graph)> {
        data.named
            .iter()
            .filter(|(n, _)| Some(n) != skip)
            .map(|(n, g)| (n.clone(), g.fork()))
            .collect()
    };
    let dataset = match destination {
        Destination::Default => {
            let mut out = merge_into(data, &inference.triples);
            out.named = copy_named(None);
            out
        }
        Destination::Named(name) => {
            let mut out = data.fork();
            let merged = match data.named_graph(name) {
                Some(g) => merge_into(g, &inference.triples),
                None => merge_into(&Graph::default(), &inference.triples),
            };
            // Keep the positional order of the existing graphs; replace in place.
            match out.named.iter().position(|(n, _)| n == name) {
                Some(i) => out.named[i].1 = merged,
                None => out.named.push((name.clone(), merged)),
            }
            out
        }
    };
    DatasetExpansion { dataset, inference }
}

/// `g`'s own triples plus `extra`, deduplicated, as a fresh graph (no named graphs).
fn merge_into(g: &Graph, extra: &[Triple]) -> Graph {
    let mut seen: FxHashSet<Triple> = FxHashSet::default();
    let triples: Vec<Triple> = graph_triples(g)
        .chain(extra.iter().cloned())
        .filter(|t| seen.insert(t.clone()))
        .collect();
    crate::graph_from_triples(triples)
}

/// The working default graph for `scope` (the merge of the selected graphs) and
/// the selected named graphs a SPARQL rule body may reach.
fn scoped_working_graph(data: &Graph, scope: &GraphScope) -> (Graph, Vec<(Term, Graph)>) {
    let (include_default, names): (bool, Vec<&Term>) = match scope {
        GraphScope::Default => (true, Vec::new()),
        GraphScope::Named(n) => (false, vec![n]),
        GraphScope::Union { default, named } => (*default, named.iter().collect()),
    };
    let mut selected: Vec<(Term, &Graph)> = Vec::new();
    for n in names {
        if selected.iter().any(|(s, _)| s == n) {
            continue;
        }
        if let Some(g) = data.named_graph(n) {
            selected.push((n.clone(), g));
        }
    }
    let mut seen: FxHashSet<Triple> = FxHashSet::default();
    let default_part = include_default.then_some(data).into_iter();
    let triples: Vec<Triple> = default_part
        .chain(selected.iter().map(|(_, g)| *g))
        .flat_map(graph_triples)
        .filter(|t| seen.insert(t.clone()))
        .collect();
    let named = selected
        .into_iter()
        .map(|(n, g)| (n, merge_into(g, &[])))
        .collect();
    (crate::graph_from_triples(triples), named)
}

/// Evaluates the SHACL-AF **node expression** rooted at `expr` (a term in the
/// `shapes` graph) against `focus` over `data`, returning the set of result nodes
/// (deduplicated, in discovery order). `None` if `expr` is not a supported node
/// expression form (e.g. a function expression, or a filter shape whose shape is
/// not declared in the shapes graph) — the caller can treat that as "skip".
///
/// This is the production public seam onto the implemented algebra
/// (focus / constant / path-with-`sh:nodes` / filter-shape / intersection /
/// union / the function operators): it is the same `parse_node_expr` →
/// `NodeExpr::eval` machinery that backs `apply_rules`' `sh:values` rules and the
/// `sh:expression` / `sh:nodeByExpression` constraints. Both the SHACL 1.2
/// `shnex:` and the SHACL-AF `sh:` spellings of the shared algebra are accepted.
///
/// [OPUS-4.8] (sq-mue75) The W3C conformance harness (`tests/w3c_node_expr.rs`)
/// now drives THIS function for real (it previously re-implemented the algebra
/// in-harness — a fidelity gap). Inline anonymous filter shapes the expression
/// references (`sh:filterShape` / `shnex:findFirst`/`matchAll`/`nodesMatching`
/// `[ … ]`) are registered into the model before parsing so they resolve at eval
/// time even when they are not target-bearing roots. `shapes` is parsed fresh;
/// for repeated evaluation parse a model once and use the engine entry points.
pub fn eval_node_expression(
    data: &Graph,
    shapes: &Graph,
    expr: &Term,
    focus: &Term,
) -> Option<Vec<Term>> {
    eval_node_expression_with_scope(data, shapes, expr, focus, &Scope::default())
}

/// [OPUS-4.8] (sq-u5rxj) [`eval_node_expression`] with a caller-supplied node
/// expression VARIABLE SCOPE: `scope` maps variable names to bound node sets that
/// a `shnex:var "<name>"` (other than `"focusNode"`) resolves against. The W3C
/// node-expr suite supplies these via its `sht:scope-<name>` action bindings; the
/// conformance harness builds a [`Scope`] from them and calls this. With an empty
/// scope this is identical to [`eval_node_expression`] (so the plain seam is a
/// thin wrapper).
pub fn eval_node_expression_with_scope(
    data: &Graph,
    shapes: &Graph,
    expr: &Term,
    focus: &Term,
    scope: &Scope,
) -> Option<Vec<Term>> {
    let mut model = ShapesModel::parse(shapes);
    // Register inline filter/function shapes the expression references so a
    // `findFirst`/`matchAll`/`nodesMatching`/`filterShape` over an anonymous
    // `[ … ]` shape resolves (mirrors the `sh:expression` constraint parse path).
    model.register_node_expr_shapes(shapes, expr);
    let g = GraphView::new(shapes);
    let parsed = parse_node_expr(&g, &model, expr)?;
    let view = GraphView::new(data);
    Some(parsed.eval(data, &view, &model, focus, scope))
}

/// [OPUS-4.8] (sq-mk9n) Evaluates a parsed node expression `expr` against a value
/// node `value` over the data `graph` (with `model` for filter-shape conformance),
/// returning its result node set. The `sh:expression` constraint (in `eval.rs`)
/// uses this to test whether an expression evaluates to `{ true }` for a value.
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
pub(crate) fn is_true_result(set: &[Term]) -> bool {
    set.len() == 1
        && matches!(&set[0], Term::Literal(l)
            if l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#boolean"
                && l.value() == "true")
}

/// True iff `focus` **conforms** to the shape identified by `shape_node` in the
/// `shapes` graph — i.e. validating `focus` against that shape as a standalone
/// focus node (ignoring the shape's own targets) yields no results. `false` if
/// `shape_node` names no parsed shape. This surfaces the `sh:condition` /
/// `sh:filterShape` conformance primitive for callers (the harness uses it).
pub fn conforms<'a>(data: &'a Graph, shapes: &Graph, shape_node: &Term) -> ConformanceCheck<'a> {
    let mut model = ShapesModel::parse(shapes);
    // Register the target shape on demand: an INLINE anonymous filter shape used by
    // a node expression (`[ sh:minInclusive 3 ]` under `shnex:findFirst`/`matchAll`/
    // `nodesMatching`) is not a top-level root, so `by_node` alone would miss it.
    let sid = model.ensure_shape(shapes, shape_node);
    ConformanceCheck { data, model, sid }
}

/// A reusable conformance check for one shape (the result of [`conforms`]): call
/// [`ConformanceCheck::holds`] per focus node. Holding the parsed model lets a
/// caller test many nodes against the same shape without re-parsing.
pub struct ConformanceCheck<'a> {
    data: &'a Graph,
    model: ShapesModel,
    sid: Option<usize>,
}

impl ConformanceCheck<'_> {
    /// True iff `focus` conforms to the shape (always `false` if the shape was
    /// not found in the shapes graph).
    pub fn holds(&self, focus: &Term) -> bool {
        match self.sid {
            Some(sid) => crate::eval::conforms_node(self.data, &self.model, sid, focus),
            None => false,
        }
    }
}

/// Fires one rule for one focus node, yielding the triples it infers.
fn fire_rule(
    augmented: &Graph,
    view: &GraphView,
    model: &ShapesModel,
    focus: &Term,
    rule: &Rule,
) -> Vec<Triple> {
    match &rule.kind {
        RuleKind::Triple {
            subject,
            predicate,
            object,
        } => {
            let scope = Scope::default();
            let subjects = subject.eval(augmented, view, model, focus, &scope);
            let predicates = predicate.eval(augmented, view, model, focus, &scope);
            let objects = object.eval(augmented, view, model, focus, &scope);
            let mut out = Vec::new();
            // The cartesian product S × P × O; only well-typed RDF triples are
            // emitted (subject IRI/blank, predicate IRI), the rest are dropped.
            for s in &subjects {
                let Some(subj) = as_subject(s) else { continue };
                for p in &predicates {
                    let Term::NamedNode(pred) = p else { continue };
                    for o in &objects {
                        out.push(Triple {
                            subject: subj.clone(),
                            predicate: pred.clone(),
                            object: o.clone(),
                        });
                    }
                }
            }
            out
        }
        RuleKind::Sparql { construct } => {
            // Pre-bind $this to the focus node by injecting a VALUES table, then
            // run the CONSTRUCT through the engine (graph-form query → Vec<Triple>).
            match build_construct_with_this(construct, focus) {
                Some(sparql) => sparq_engine::construct(augmented, &sparql).unwrap_or_default(),
                None => Vec::new(),
            }
        }
        RuleKind::Values { predicate, values } => {
            // (focus, predicate, v) for every value-expression result v. The focus
            // node is the subject, so it must be an IRI/blank node (else skipped).
            let Some(subj) = as_subject(focus) else {
                return Vec::new();
            };
            values
                .eval(augmented, view, model, focus, &Scope::default())
                .into_iter()
                .map(|o| Triple {
                    subject: subj.clone(),
                    predicate: predicate.clone(),
                    object: o,
                })
                .collect()
        }
    }
}

/// The shape ids that reference shape `sid` as a `sh:property` child (the parent
/// node shapes of a property shape). Used to range a `sh:values` value rule over
/// the parent's focus nodes when the property shape has no targets of its own.
fn parents_of(model: &ShapesModel, sid: usize) -> Vec<usize> {
    (0..model.shapes.len())
        .filter(|&p| model.shapes[p].property_children.contains(&sid))
        .collect()
}

/// Builds the CONSTRUCT query text with `$this` pre-bound to `focus` via a
/// `VALUES` table appended to the WHERE clause. We append `VALUES (?this) {
/// (<focus>) }` at the end of the (single, outermost) WHERE block. This is robust
/// for the rule case because a CONSTRUCT's WHERE is a group graph pattern and a
/// trailing VALUES joins with it. Returns `None` if the focus node has no SPARQL
/// ground-term form (a blank node — rare as a rule focus).
fn build_construct_with_this(construct: &str, focus: &Term) -> Option<String> {
    let ground = match focus {
        Term::NamedNode(n) => format!("<{}>", n.as_str()),
        Term::Literal(l) => l.to_string(),
        // A blank-node focus cannot be written in VALUES; such a rule is skipped
        // for that focus (rule focus nodes are IRIs in practice).
        Term::BlankNode(_) => return None,
        #[allow(unreachable_patterns)]
        _ => return None,
    };
    // Find the last `}` (end of the WHERE group) and inject the VALUES before it.
    let close = construct.rfind('}')?;
    let mut out = String::with_capacity(construct.len() + ground.len() + 32);
    out.push_str(&construct[..close]);
    out.push_str(&format!(" VALUES (?this) {{ ({ground}) }} "));
    out.push_str(&construct[close..]);
    Some(out)
}

/// The focus nodes of shape `sid` (its target nodes) over `view`. Mirrors the
/// validator's own target selection so rules and constraints agree on targets.
fn focus_nodes(view: &GraphView, model: &ShapesModel, sid: usize) -> Vec<Term> {
    use crate::model::Target;
    let mut out: Vec<Term> = Vec::new();
    for t in &model.shapes[sid].targets {
        match t {
            Target::Node(n) => out.push(n.clone()),
            Target::Class(c) | Target::ImplicitClass(c) => out.extend(view.instances_of(c)),
            Target::SubjectsOf(p) => out.extend(view.subjects_of(p)),
            Target::ObjectsOf(p) => out.extend(view.objects_of(p)),
            // [OPUS-4.8] (sq-rnkdh) SHACL 1.2 SPARQL-valued `sh:targetNode`: the
            // node expression SELECTs the focus nodes (no `$this` to bind).
            Target::Sparql(idx) => out.extend(model.select_exprs[*idx].eval_target(view.graph())),
            // [OPUS-4.8] (sq-rnkdh) SHACL 1.2 `sh:targetWhere`: every data-graph node
            // conforming to the inline shape. Conformance is checked through the
            // full validator (`conforms_node`), mirroring `sh:condition` gating.
            Target::Where(inner) => {
                let mut seen: FxHashSet<Term> = FxHashSet::default();
                for [s, _, o] in view.triples(None, None, None) {
                    for n in [s, o] {
                        if seen.insert(n.clone())
                            && crate::eval::conforms_node(view.graph(), model, *inner, &n)
                        {
                            out.push(n);
                        }
                    }
                }
            }
        }
    }
    crate::view::dedup(out)
}

/// True iff `focus` conforms to every `sh:condition` shape — i.e. validating
/// `focus` against each condition shape (as a standalone target) yields no
/// results. Reuses the full constraint validator so conditions can be arbitrary
/// shapes (Core constraints, sh:sparql, nested node/property shapes, …).
fn conforms_to_all(
    augmented: &Graph,
    model: &ShapesModel,
    conditions: &[usize],
    focus: &Term,
) -> bool {
    conditions
        .iter()
        .all(|&cid| crate::eval::conforms_node(augmented, model, cid, focus))
}

/// `data` expanded with `extra` triples as a fresh `Graph`. Interns both the data
/// (re-read from its dictionary) and the extra triples into a new dictionary.
pub(crate) fn expand_graph(data: &Graph, extra: &[Triple]) -> Graph {
    crate::graph_from_triples(graph_triples(data).chain(extra.iter().cloned()))
}

/// `data`'s own (default-graph) triples, decoded from its dictionary.
fn graph_triples(data: &Graph) -> impl Iterator<Item = Triple> {
    let view = GraphView::new(data);
    view.triples(None, None, None).into_iter().map(|[s, p, o]| {
        Triple {
            subject: as_subject(&s).unwrap_or_else(|| {
                // A literal subject is impossible in a well-formed graph; fall back
                // to a placeholder blank node (never reached for real data).
                NamedOrBlankNode::BlankNode(oxrdf::BlankNode::default())
            }),
            predicate: match &p {
                Term::NamedNode(n) => n.clone(),
                _ => NamedNode::new_unchecked("urn:x-sparq:ill-formed-predicate"),
            },
            object: o,
        }
    })
}

/// An RDF term as a triple subject (IRI or blank node); `None` for a literal.
fn as_subject(t: &Term) -> Option<NamedOrBlankNode> {
    match t {
        Term::NamedNode(n) => Some(NamedOrBlankNode::NamedNode(n.clone())),
        Term::BlankNode(b) => Some(NamedOrBlankNode::BlankNode(b.clone())),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::RDF_TYPE;

    const PRELUDE: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
        @prefix ex: <http://example.org/> .\n\
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
        @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n";

    fn g(ttl: &str) -> Graph {
        Graph::load_str(&format!("{PRELUDE}{ttl}"), "turtle").unwrap()
    }

    /// True iff the inferred set contains a triple `<s> <p> <o>` (IRI terms).
    fn has(inf: &Inference, s: &str, p: &str, o: &str) -> bool {
        inf.triples.iter().any(|t| {
            t.subject.to_string() == format!("<{}>", s)
                && t.predicate.as_str() == p
                && t.object.to_string() == format!("<{}>", o)
        })
    }

    // ---- sh:TripleRule: infer a triple ----
    #[test]
    fn triple_rule_infers_type() {
        // For every ex:Person focus node, infer (this, rdf:type, ex:Agent).
        let data = g("ex:alice a ex:Person . ex:bob a ex:Person .");
        let shapes = g(r#"
            ex:PersonShape a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:TripleRule ;
                sh:subject sh:this ;
                sh:predicate rdf:type ;
                sh:object ex:Agent ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(has(
            &inf,
            "http://example.org/alice",
            RDF_TYPE,
            "http://example.org/Agent"
        ));
        assert!(has(
            &inf,
            "http://example.org/bob",
            RDF_TYPE,
            "http://example.org/Agent"
        ));
        assert_eq!(inf.triples.len(), 2);
    }

    // ---- sh:TripleRule with a path node expression for the object ----
    #[test]
    fn triple_rule_path_object() {
        // Infer (this, ex:grandparent, X) where X = this/ex:parent/ex:parent.
        let data = g(r#"
            ex:a a ex:Person ; ex:parent ex:b .
            ex:b ex:parent ex:c .
        "#);
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:TripleRule ;
                sh:subject sh:this ;
                sh:predicate ex:grandparent ;
                sh:object [ sh:path ( ex:parent ex:parent ) ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(
            has(
                &inf,
                "http://example.org/a",
                "http://example.org/grandparent",
                "http://example.org/c"
            ),
            "{:?}",
            inf.triples
        );
        assert_eq!(inf.triples.len(), 1);
    }

    // ---- sh:SPARQLRule with sh:construct ----
    #[test]
    fn sparql_rule_construct() {
        let data = g("ex:alice a ex:Person ; ex:firstName \"Alice\" .");
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:SPARQLRule ;
                sh:construct """
                  CONSTRUCT { $this <http://example.org/label> ?n }
                  WHERE { $this <http://example.org/firstName> ?n }
                """ ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert_eq!(inf.triples.len(), 1, "{:?}", inf.triples);
        let t = &inf.triples[0];
        assert_eq!(t.predicate.as_str(), "http://example.org/label");
        assert_eq!(t.object.to_string(), "\"Alice\"");
    }

    // ---- sh:SPARQLRule with sh:prefixes ----
    #[test]
    fn sparql_rule_with_prefixes() {
        let data = g("ex:alice a ex:Person ; ex:firstName \"Alice\" .");
        let shapes = g(r#"
            ex:prefixes sh:declare [ sh:prefix "ex" ; sh:namespace "http://example.org/"^^xsd:anyURI ] .
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:SPARQLRule ;
                sh:prefixes ex:prefixes ;
                sh:construct "CONSTRUCT { $this ex:label ?n } WHERE { $this ex:firstName ?n }" ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert_eq!(inf.triples.len(), 1, "{:?}", inf.triples);
        assert_eq!(
            inf.triples[0].predicate.as_str(),
            "http://example.org/label"
        );
    }

    // ---- sh:condition gating ----
    #[test]
    fn condition_gates_rule() {
        // Only persons with an ex:active "true" flag get the ex:Agent type.
        let data = g(r#"
            ex:alice a ex:Person ; ex:active true .
            ex:bob   a ex:Person .
        "#);
        let shapes = g(r#"
            ex:ActiveShape a sh:NodeShape ;
              sh:property [ sh:path ex:active ; sh:hasValue true ] .
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:TripleRule ;
                sh:condition ex:ActiveShape ;
                sh:subject sh:this ;
                sh:predicate rdf:type ;
                sh:object ex:Agent ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(has(
            &inf,
            "http://example.org/alice",
            RDF_TYPE,
            "http://example.org/Agent"
        ));
        assert!(
            !has(
                &inf,
                "http://example.org/bob",
                RDF_TYPE,
                "http://example.org/Agent"
            ),
            "bob lacks the condition, must not be inferred: {:?}",
            inf.triples
        );
        assert_eq!(inf.triples.len(), 1);
    }

    // ---- sh:order sequencing (a later rule depends on an earlier rule's output) ----
    #[test]
    fn order_sequences_dependent_rules() {
        // Rule order 1 types every Person as ex:Agent; rule order 2 has a condition
        // (must be an ex:Agent) that ONLY holds because rule 1 already fired.
        let data = g("ex:alice a ex:Person .");
        let shapes = g(r#"
            ex:AgentShape a sh:NodeShape ;
              sh:property [ sh:path rdf:type ; sh:hasValue ex:Agent ] .
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:TripleRule ;
                sh:order 1 ;
                sh:subject sh:this ;
                sh:predicate rdf:type ;
                sh:object ex:Agent ;
              ] ;
              sh:rule [
                a sh:TripleRule ;
                sh:order 2 ;
                sh:condition ex:AgentShape ;
                sh:subject sh:this ;
                sh:predicate rdf:type ;
                sh:object ex:Verified ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(has(
            &inf,
            "http://example.org/alice",
            RDF_TYPE,
            "http://example.org/Agent"
        ));
        assert!(
            has(
                &inf,
                "http://example.org/alice",
                RDF_TYPE,
                "http://example.org/Verified"
            ),
            "the order-2 rule must see the order-1 inference: {:?}",
            inf.triples
        );
        assert_eq!(inf.triples.len(), 2);
    }

    // ---- sh:deactivated ----
    #[test]
    fn deactivated_rule_is_ignored() {
        let data = g("ex:alice a ex:Person .");
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:TripleRule ;
                sh:deactivated true ;
                sh:subject sh:this ;
                sh:predicate rdf:type ;
                sh:object ex:Agent ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(inf.triples.is_empty(), "{:?}", inf.triples);
    }

    // ---- fixpoint termination on a transitively-chaining rule ----
    #[test]
    fn fixpoint_transitive_closure_terminates() {
        // A rule that closes ex:sub transitively: (this, ex:sub, Y) where
        // Y = this/ex:sub/ex:sub. Iterating to a fixpoint must add the closure
        // edges and then terminate (no infinite growth).
        let data = g(r#"
            ex:a a ex:N ; ex:sub ex:b .
            ex:b a ex:N ; ex:sub ex:c .
            ex:c a ex:N ; ex:sub ex:d .
            ex:d a ex:N .
        "#);
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:N ;
              sh:rule [
                a sh:TripleRule ;
                sh:subject sh:this ;
                sh:predicate ex:sub ;
                sh:object [ sh:path ( ex:sub ex:sub ) ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        // a reaches c (a/sub/sub) and d (after c→? ... a/sub*/sub) transitively.
        assert!(has(
            &inf,
            "http://example.org/a",
            "http://example.org/sub",
            "http://example.org/c"
        ));
        assert!(has(
            &inf,
            "http://example.org/a",
            "http://example.org/sub",
            "http://example.org/d"
        ));
        assert!(has(
            &inf,
            "http://example.org/b",
            "http://example.org/sub",
            "http://example.org/d"
        ));
        assert!(
            !inf.capped,
            "transitive closure must reach a fixpoint, not the cap"
        );
        assert!(inf.iterations >= 2, "needs >1 pass: {inf:?}");
    }

    // ---- the bounded cap protects against a non-terminating rule ----
    #[test]
    fn nonterminating_rule_is_capped() {
        // A CONSTRUCT that mints a FRESH blank node each pass never reaches a
        // fixpoint; the cap must stop it (and flag `capped`).
        let data = g("ex:a a ex:N .");
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:N ;
              sh:rule [
                a sh:SPARQLRule ;
                sh:construct "CONSTRUCT { $this <http://example.org/has> _:x } WHERE { }" ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(inf.capped, "a fresh-bnode-per-pass rule must hit the cap");
        assert_eq!(inf.iterations, MAX_ITERATIONS);
    }

    // ---- expand() returns data ∪ inferred as a queryable graph ----
    #[test]
    fn expand_merges_inferred() {
        let data = g("ex:alice a ex:Person .");
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:TripleRule ;
                sh:subject sh:this ;
                sh:predicate rdf:type ;
                sh:object ex:Agent ;
              ] .
        "#);
        let expanded = expand(&data, &shapes);
        let view = GraphView::new(&expanded);
        // Both the original type and the inferred type are present.
        let types = view.objects(
            &Term::NamedNode(NamedNode::new_unchecked("http://example.org/alice")),
            RDF_TYPE,
        );
        let strs: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        assert!(strs.iter().any(|s| s.contains("Person")));
        assert!(strs.iter().any(|s| s.contains("Agent")));
    }

    // ---- no rules ⇒ empty inference, no iteration ----
    #[test]
    fn no_rules_is_noop() {
        let data = g("ex:alice a ex:Person .");
        let shapes = g("ex:S a sh:NodeShape ; sh:targetClass ex:Person ; sh:property [ sh:path ex:name ; sh:minCount 1 ] .");
        let inf = apply_rules(&data, &shapes);
        assert!(inf.triples.is_empty());
        assert_eq!(inf.iterations, 0);
    }

    // [OPUS-4.8] (sq-1m0n) -------- full node-expression algebra + sh:values --------

    /// All inferred object IRIs for `(s, p, ?)`, as ex:-local sorted strings.
    fn objs(inf: &Inference, s: &str, p: &str) -> Vec<String> {
        let mut v: Vec<String> = inf
            .triples
            .iter()
            .filter(|t| t.subject.to_string() == format!("<{s}>") && t.predicate.as_str() == p)
            .map(|t| t.object.to_string())
            .collect();
        v.sort();
        v
    }

    // ---- path node expression with explicit sh:nodes (start != focus) ----
    #[test]
    fn node_expr_path_with_explicit_nodes() {
        // Infer (this, ex:grandparentName, X) where the VALUE expression starts at
        // this/ex:parent (sh:nodes) and then walks ex:name — i.e. the parent's name.
        let data = g(r#"
            ex:a a ex:Person ; ex:parent ex:b .
            ex:b ex:name "Bob" .
        "#);
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [
                sh:path ex:grandparentName ;
                sh:values [ sh:nodes [ sh:path ex:parent ] ; sh:path ex:name ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        // (ex:a, ex:grandparentName, "Bob")
        assert_eq!(inf.triples.len(), 1, "{:?}", inf.triples);
        let t = &inf.triples[0];
        assert_eq!(t.subject.to_string(), "<http://example.org/a>");
        assert_eq!(t.predicate.as_str(), "http://example.org/grandparentName");
        assert_eq!(t.object.to_string(), "\"Bob\"");
    }

    // ---- filter-shape node expression ----
    #[test]
    fn node_expr_filter_shape() {
        // Of this/ex:item values, keep only those conforming to ex:HasLabelShape
        // (those with an rdfs:label), inferred under ex:goodItem.
        let data = g(r#"
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:a a ex:Coll ; ex:item ex:x , ex:y , ex:z .
            ex:x rdfs:label "X" .
            ex:y rdfs:label "Y" .
            ex:z ex:other "no label" .
        "#);
        let shapes = g(r#"
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:HasLabelShape a sh:NodeShape ;
              sh:property [ sh:path rdfs:label ; sh:minCount 1 ] .
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Coll ;
              sh:property [
                sh:path ex:goodItem ;
                sh:values [ sh:filterShape ex:HasLabelShape ; sh:nodes [ sh:path ex:item ] ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert_eq!(
            objs(&inf, "http://example.org/a", "http://example.org/goodItem"),
            vec![
                "<http://example.org/x>".to_string(),
                "<http://example.org/y>".to_string()
            ],
            "only x and y carry a label: {:?}",
            inf.triples
        );
    }

    // ---- union node expression ----
    #[test]
    fn node_expr_union() {
        // Union of this/ex:a values and this/ex:b values, deduplicated.
        let data = g(r#"
            ex:n a ex:T ; ex:a ex:x , ex:y ; ex:b ex:y , ex:z .
        "#);
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:T ;
              sh:property [
                sh:path ex:all ;
                sh:values [ sh:union ( [ sh:path ex:a ] [ sh:path ex:b ] ) ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert_eq!(
            objs(&inf, "http://example.org/n", "http://example.org/all"),
            vec![
                "<http://example.org/x>".to_string(),
                "<http://example.org/y>".to_string(),
                "<http://example.org/z>".to_string()
            ],
            "union {{x,y}} ∪ {{y,z}} = {{x,y,z}}: {:?}",
            inf.triples
        );
    }

    // ---- intersection node expression (with dedup) ----
    #[test]
    fn node_expr_intersection() {
        // Intersection of this/ex:a values and this/ex:b values.
        let data = g(r#"
            ex:n a ex:T ; ex:a ex:x , ex:y , ex:w ; ex:b ex:y , ex:z , ex:w .
        "#);
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:T ;
              sh:property [
                sh:path ex:both ;
                sh:values [ sh:intersection ( [ sh:path ex:a ] [ sh:path ex:b ] ) ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert_eq!(
            objs(&inf, "http://example.org/n", "http://example.org/both"),
            vec![
                "<http://example.org/w>".to_string(),
                "<http://example.org/y>".to_string()
            ],
            "intersection {{x,y,w}} ∩ {{y,z,w}} = {{y,w}}: {:?}",
            inf.triples
        );
    }

    // ---- a constant node expression in sh:values ----
    #[test]
    fn values_rule_constant() {
        // Every ex:Person gets ex:species ex:Human (a constant value expression).
        let data = g("ex:alice a ex:Person . ex:bob a ex:Person .");
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [ sh:path ex:species ; sh:values ex:Human ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(has(
            &inf,
            "http://example.org/alice",
            "http://example.org/species",
            "http://example.org/Human"
        ));
        assert!(has(
            &inf,
            "http://example.org/bob",
            "http://example.org/species",
            "http://example.org/Human"
        ));
        assert_eq!(inf.triples.len(), 2);
    }

    // ---- sh:values value rule with a path expression (the canonical "copy" rule) ----
    #[test]
    fn values_rule_path_copies() {
        // ex:fullName := this/ex:firstName (a property value rule on a property shape
        // that is a sh:property CHILD of the targeted node shape — fires for the
        // PARENT's focus nodes).
        let data =
            g(r#"ex:a a ex:Person ; ex:firstName "Ann" . ex:b a ex:Person ; ex:firstName "Ben" ."#);
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [
                sh:path ex:fullName ;
                sh:values [ sh:path ex:firstName ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert_eq!(inf.triples.len(), 2, "{:?}", inf.triples);
        let ann = inf
            .triples
            .iter()
            .find(|t| t.subject.to_string() == "<http://example.org/a>")
            .unwrap();
        assert_eq!(ann.predicate.as_str(), "http://example.org/fullName");
        assert_eq!(ann.object.to_string(), "\"Ann\"");
    }

    // ---- a non-predicate path on a sh:values shape is skipped (cannot be inferred) ----
    #[test]
    fn values_rule_non_predicate_path_skipped() {
        // sh:path is a sequence (not a single predicate) — no inferable predicate.
        let data = g("ex:a a ex:Person ; ex:x ex:b . ex:b ex:y ex:c .");
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [
                sh:path ( ex:x ex:y ) ;
                sh:values ex:K ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(inf.triples.is_empty(), "{:?}", inf.triples);
    }

    // ---- sh:values honours sh:deactivated on the property shape ----
    #[test]
    fn values_rule_deactivated() {
        let data = g("ex:a a ex:Person .");
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [
                a sh:PropertyShape ;
                sh:deactivated true ;
                sh:path ex:species ;
                sh:values ex:Human ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(inf.triples.is_empty(), "{:?}", inf.triples);
    }

    // ---- a TripleRule using the new union object expression ----
    #[test]
    fn triple_rule_union_object() {
        // (this, ex:contact, X) for X in (this/ex:email ∪ this/ex:phone).
        let data = g(r#"ex:a a ex:Person ; ex:email ex:e1 ; ex:phone ex:p1 ."#);
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:rule [
                a sh:TripleRule ;
                sh:subject sh:this ;
                sh:predicate ex:contact ;
                sh:object [ sh:union ( [ sh:path ex:email ] [ sh:path ex:phone ] ) ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert_eq!(
            objs(&inf, "http://example.org/a", "http://example.org/contact"),
            vec![
                "<http://example.org/e1>".to_string(),
                "<http://example.org/p1>".to_string()
            ],
            "{:?}",
            inf.triples
        );
    }

    // ---- an UNDECLARED custom function IRI is dropped, not misfired ----
    #[test]
    fn function_expression_is_dropped() {
        // [ ex:someFunction ( sh:this ) ] names a function IRI that is NOT declared
        // as a sh:SPARQLFunction, so the registry has no implementation: the value
        // rule must infer nothing (lenient drop), never panic or misfire.
        let data = g("ex:a a ex:Person .");
        let shapes = g(r#"
            ex:S a sh:NodeShape ;
              sh:targetClass ex:Person ;
              sh:property [
                sh:path ex:fx ;
                sh:values [ ex:someFunction ( sh:this ) ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(inf.triples.is_empty(), "{:?}", inf.triples);
    }

    // ---- [OPUS-4.8] (sq-mk9n) function-expression operators in a sh:values rule ----
    #[test]
    fn values_rule_function_concat() {
        // A `shnex:concat` function expression in sh:values infers one triple per
        // concatenated member: (focus, ex:both, v) for v in {ex:a-vals ∪ ex:b-vals}.
        let data = g("ex:a a ex:N ; ex:x ex:p ; ex:y ex:q .");
        let shapes = g(r#"
            @prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> .
            ex:S a sh:NodeShape ;
              sh:targetClass ex:N ;
              sh:property [
                sh:path ex:both ;
                sh:values [ shnex:concat ( [ sh:path ex:x ] [ sh:path ex:y ] ) ] ;
              ] .
        "#);
        let inf = apply_rules(&data, &shapes);
        assert!(
            has(
                &inf,
                "http://example.org/a",
                "http://example.org/both",
                "http://example.org/p"
            ),
            "{:?}",
            inf.triples
        );
        assert!(
            has(
                &inf,
                "http://example.org/a",
                "http://example.org/both",
                "http://example.org/q"
            ),
            "{:?}",
            inf.triples
        );
        assert_eq!(inf.triples.len(), 2);
    }

    #[test]
    fn eval_node_expression_function_operators() {
        use crate::view::GraphView;
        // Exercise the registry directly through the public seam: sum/count/if.
        let data = g("ex:a a ex:N .");
        let shapes = g(r#"
            @prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> .
            ex:Decl
              ex:sumExpr   [ shnex:sum ( 4 5 3 ) ] ;
              ex:countExpr [ shnex:count ( 4 3 3 ) ] ;
              ex:ifExpr    [ shnex:if true ; shnex:then 42 ] ;
              ex:distExpr  [ shnex:distinct ( 4 2 4 ) ] .
        "#);
        let view = GraphView::new(&shapes);
        let decl = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://example.org/Decl"));
        let focus = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://example.org/a"));
        let lex = |p: &str| -> Vec<String> {
            let e = view
                .object(&decl, &format!("http://example.org/{p}"))
                .unwrap();
            let mut v: Vec<String> = eval_node_expression(&data, &shapes, &e, &focus)
                .unwrap()
                .iter()
                .map(|t| match t {
                    Term::Literal(l) => l.value().to_string(),
                    _ => t.to_string(),
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(lex("sumExpr"), vec!["12".to_string()]);
        assert_eq!(lex("countExpr"), vec!["3".to_string()]);
        assert_eq!(lex("ifExpr"), vec!["42".to_string()]);
        assert_eq!(lex("distExpr"), vec!["2".to_string(), "4".to_string()]);
    }

    #[test]
    fn eval_node_expression_custom_sparql_function() {
        use crate::view::GraphView;
        // A declared sh:SPARQLFunction applied to one argument: doubles its input.
        let data = g("ex:a a ex:N .");
        let shapes = g(r#"
            ex:double a sh:SPARQLFunction ;
              sh:parameter [ sh:path ex:arg ] ;
              sh:select "SELECT ((?arg * 2) AS ?result) WHERE { }" .
            ex:Decl ex:call [ ex:double ( 21 ) ] .
        "#);
        let view = GraphView::new(&shapes);
        let decl = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://example.org/Decl"));
        let call = view.object(&decl, "http://example.org/call").unwrap();
        let focus = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://example.org/a"));
        let got: Vec<String> = eval_node_expression(&data, &shapes, &call, &focus)
            .expect("declared sh:SPARQLFunction must evaluate")
            .iter()
            .map(|t| match t {
                Term::Literal(l) => l.value().to_string(),
                _ => t.to_string(),
            })
            .collect();
        assert_eq!(got, vec!["42".to_string()], "21 * 2 = 42");
    }

    // ---- the public `eval_node_expression` seam, exercised directly ----
    #[test]
    fn eval_node_expression_seam() {
        use crate::view::GraphView;

        // Evaluate node expressions directly through the public seam (NOT via
        // apply_rules). The expression terms are the blank nodes that the SHACL-AF
        // `sh:values` algebra ranges over, so they exercise the real
        // parse_node_expr → NodeExpr::eval machinery — not a Constant short-circuit.
        let data = g(r#"
            ex:a ex:parent ex:b , ex:c .
            ex:b ex:name "Bob" .
            ex:c ex:name "Carol" .
            ex:d ex:name "Dave" .
        "#);
        let shapes = g(r#"
            ex:Decl
              # a path-with-sh:nodes expression: focus/ex:parent, then ex:name
              ex:goodExpr [ sh:nodes [ sh:path ex:parent ] ; sh:path ex:name ] ;
              # an unsupported function expression
              ex:fnExpr [ ex:someFunction ( sh:this ) ] .
        "#);
        let view = GraphView::new(&shapes);
        let decl = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://example.org/Decl"));
        let good_expr = view
            .object(&decl, "http://example.org/goodExpr")
            .expect("goodExpr node-expression bnode present");
        let fn_expr = view
            .object(&decl, "http://example.org/fnExpr")
            .expect("fnExpr node-expression bnode present");
        let focus = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://example.org/a"));

        let mut got: Vec<String> = eval_node_expression(&data, &shapes, &good_expr, &focus)
            .expect("supported path-with-sh:nodes form must evaluate")
            .iter()
            .map(|t| t.to_string())
            .collect();
        got.sort();
        // focus/ex:parent = {ex:b, ex:c}; their ex:name = {"Bob","Carol"}.
        // ex:d's name is NOT reachable from the focus, so it must be absent.
        assert_eq!(got, vec!["\"Bob\"".to_string(), "\"Carol\"".to_string()]);

        // An unsupported (function) expression returns None — the documented
        // "skip" contract — rather than a wrong/empty result set.
        assert!(
            eval_node_expression(&data, &shapes, &fn_expr, &focus).is_none(),
            "a function expression is not a supported node-expression form"
        );
    }

    /// [OPUS-4.8] (sq-u5rxj) A `shnex:var "<name>"` resolves a caller-supplied
    /// scope binding (other than `"focusNode"`); an absent name is the empty set.
    #[test]
    fn node_expr_var_resolves_caller_scope() {
        use crate::view::GraphView;
        let data = g("");
        let shapes = g(r#"
            ex:Decl ex:e [ <http://www.w3.org/ns/shacl-node-expr#var> "bound" ] ;
                    ex:u [ <http://www.w3.org/ns/shacl-node-expr#var> "missing" ] .
        "#);
        let view = GraphView::new(&shapes);
        let decl = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://example.org/Decl"));
        let bound_expr = view.object(&decl, "http://example.org/e").unwrap();
        let unbound_expr = view.object(&decl, "http://example.org/u").unwrap();
        let focus = Term::NamedNode(oxrdf::NamedNode::new_unchecked("http://example.org/x"));

        let mut scope = Scope::default();
        let val = Term::Literal(oxrdf::Literal::new_simple_literal("hit"));
        scope.insert("bound".to_string(), vec![val.clone()]);

        // The bound var resolves to the scope value; the unbound var is empty.
        let got = eval_node_expression_with_scope(&data, &shapes, &bound_expr, &focus, &scope)
            .expect("var expression parses");
        assert_eq!(got, vec![val]);
        let none = eval_node_expression_with_scope(&data, &shapes, &unbound_expr, &focus, &scope)
            .expect("var expression parses");
        assert!(none.is_empty(), "an unbound var resolves to the empty set");
    }
}
