//! SHACL 1.2 Inference Rules, separate from the SHACL-AF execution APIs.
//!
//! Rules use a snapshot per order group, iterate each layer separately, and run
//! run-once rules before iterating rules. Inputs are immutable. Failures are
//! returned rather than converted into empty inferences.

use crate::model::{sh, ShapesModel};
use crate::rules::{expand_graph, Inference, MAX_ITERATIONS};
use crate::view::{GraphView, RDFS_SUBCLASS_OF};
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use rustc_hash::FxHashSet;
use sparq_core::Graph;
use std::cmp::Ordering;

use crate::rule_schedule::{clock, saturate};

const NEX: &str = "http://www.w3.org/ns/shacl-node-expr#";
const SPARQL: &str = "http://www.w3.org/ns/sparql#";

fn iri(s: &str) -> Term {
    NamedNode::new_unchecked(s).into()
}
fn typed(g: &GraphView, n: &Term, local: &str) -> bool {
    g.is_instance_of(n, &iri(&sh(local)))
}
fn boolean(g: &GraphView, n: &Term, local: &str) -> bool {
    matches!(g.object(n, &sh(local)), Some(Term::Literal(l)) if l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#boolean" && matches!(l.value(), "true" | "1"))
}
fn number(g: &GraphView, n: &Term, local: &str) -> Result<String, String> {
    match g.object(n, &sh(local)) {
        None => Ok("0".into()),
        Some(Term::Literal(l))
            if matches!(
                l.datatype().as_str(),
                "http://www.w3.org/2001/XMLSchema#integer"
                    | "http://www.w3.org/2001/XMLSchema#decimal"
            ) && sparq_substrate::numeric::split_decimal(l.value()).is_some() =>
        {
            Ok(l.value().into())
        }
        _ => Err(format!("Invalid sh:{local} on {n}")),
    }
}
fn cmp(a: &str, b: &str) -> Ordering {
    sparq_substrate::numeric::cmp_plain_decimal(a, b).expect("validated decimal")
}

struct Rule {
    node: Term,
    owners: Vec<usize>,
    global: bool,
    layer: String,
    order: String,
    once: bool,
    kind: Kind,
    conditions: Vec<usize>,
    expected: Vec<Term>,
}
enum Kind {
    Triple([Option<Term>; 3]),
    Sparql {
        query: spargebra::Query,
        params: Vec<Vec<(String, Term)>>,
    },
}

/// Apply SHACL 1.2 rules, returning only new inferred triples.
///
/// `rule_set = None` selects every rule in the shapes graph; `Some(iri)` selects
/// that rule set and its transitive `sh:includesRuleSet` closure. Unsupported rule
/// types, expression/query failures and an exhausted iteration bound return `Err`.
/// Unsupported processors on any rule or rule set in the shapes graph fail even
/// when that node is unselected or deactivated. The legacy [`crate::apply_rules`]
/// retains SHACL-AF semantics.
pub fn apply_rules(
    data: &Graph,
    shapes: &Graph,
    rule_set: Option<&Term>,
) -> Result<Inference, String> {
    let sets: Vec<Term> = rule_set.cloned().into_iter().collect();
    execute(data, shapes, &sets, rule_set.is_none())
}

pub(crate) fn entailments(shapes: &Graph) -> (Vec<Term>, bool) {
    let g = GraphView::new(shapes);
    let mut sets = Vec::new();
    let mut all = false;
    for [_, _, regime] in g.triples(None, Some(&sh("entailment")), None) {
        if regime == iri(&sh("RulesEntailment")) {
            all = true;
        } else if typed(&g, &regime, "RuleSet") {
            sets.push(regime);
        }
    }
    (sets, all)
}

fn selected(g: &GraphView, sets: &[Term], all: bool) -> Result<Vec<Term>, String> {
    let mut out = Vec::new();
    let mut todo = sets.to_vec();
    let mut seen = FxHashSet::default();
    while let Some(set) = todo.pop() {
        if !seen.insert(set.clone()) {
            continue;
        }
        if !typed(g, &set, "RuleSet") {
            return Err(format!("Unknown rule set {set}"));
        }
        out.extend(g.objects(&set, &sh("hasRule")));
        todo.extend(g.objects(&set, &sh("includesRuleSet")));
    }
    if all {
        out.extend(all_rules(g));
    }
    Ok(crate::view::dedup(out))
}

fn all_rules(g: &GraphView) -> Vec<Term> {
    let mut out = Vec::new();
    for local in ["Rule", "SPARQLRule", "TripleRule"] {
        out.extend(g.instances_of(&iri(&sh(local))));
    }
    for template in g.instances_of(&iri(&sh("SPARQLRuleTemplate"))) {
        out.extend(g.instances_of(&template));
    }
    for [_, _, rule] in g.triples(None, Some(&sh("rule")), None) {
        out.push(rule);
    }
    out
}

fn check_processors(g: &GraphView) -> Result<(), String> {
    // Processor requirements apply across the shapes graph, before selection or
    // deactivation. Checking included sets also covers their inherited values:
    // no custom processor is currently supported by this executor.
    let mut nodes = all_rules(g);
    nodes.extend(g.instances_of(&iri(&sh("RuleSet"))));
    for local in ["hasRule", "includesRuleSet"] {
        for [set, _, member] in g.triples(None, Some(&sh(local)), None) {
            nodes.extend([set, member]);
        }
    }
    for node in crate::view::dedup(nodes) {
        if !g.objects(&node, &sh("ruleProcessor")).is_empty() {
            return Err(format!("Unsupported rule processor on {node}"));
        }
    }
    Ok(())
}

fn parse(shapes: &Graph, model: &mut ShapesModel, nodes: Vec<Term>) -> Result<Vec<Rule>, String> {
    let g = GraphView::new(shapes);
    let mut rules = Vec::new();
    for node in nodes {
        if boolean(&g, &node, "deactivated") {
            continue;
        }
        for local in [
            "layer",
            "order",
            "runOnce",
            "deactivated",
            "subject",
            "predicate",
            "object",
        ] {
            if g.objects(&node, &sh(local)).len() > 1 {
                return Err(format!("Multiple sh:{local} values on {node}"));
            }
        }
        for local in ["runOnce", "deactivated"] {
            if let Some(value) = g.object(&node, &sh(local)) {
                if !matches!(value, Term::Literal(l) if l.datatype().as_str()=="http://www.w3.org/2001/XMLSchema#boolean" && matches!(l.value(),"true"|"false"|"1"|"0"))
                {
                    return Err(format!("Invalid sh:{local} on {node}"));
                }
            }
        }
        let owners: Vec<_> = g
            .subjects(&sh("rule"), &node)
            .iter()
            .filter_map(|owner| model.ensure_shape(shapes, owner))
            .collect();
        let global = g.subjects(&sh("rule"), &node).is_empty();
        let mut conditions = Vec::new();
        for condition in g.objects(&node, &sh("condition")) {
            let mut stack = vec![condition];
            let mut seen = FxHashSet::default();
            while let Some(c) = stack.pop() {
                if !seen.insert(c.clone()) {
                    continue;
                }
                let shape_class = |n: &Term| {
                    typed(&g, n, "ShapeClass")
                        || (typed(&g, n, "NodeShape")
                            && g.is_instance_of(
                                n,
                                &iri("http://www.w3.org/2000/01/rdf-schema#Class"),
                            ))
                };
                if seen.len() == 1 || shape_class(&c) {
                    let id = model
                        .ensure_shape(shapes, &c)
                        .ok_or("Invalid condition shape")?;
                    conditions.push(id);
                }
                if seen.len() > 1 || shape_class(&c) {
                    stack.extend(g.objects(&c, RDFS_SUBCLASS_OF));
                }
            }
        }
        let kind = if typed(&g, &node, "SPARQLRule") {
            sparql_kind(&g, &node, &node, global, false)?
        } else if typed(&g, &node, "TripleRule") {
            Kind::Triple([
                g.object(&node, &sh("subject")),
                g.object(&node, &sh("predicate")),
                g.object(&node, &sh("object")),
            ])
        } else {
            let template = g
                .instances_of(&iri(&sh("SPARQLRuleTemplate")))
                .into_iter()
                .find(|t| g.is_instance_of(&node, t))
                .ok_or_else(|| format!("Unsupported rule type on {node}"))?;
            sparql_kind(&g, &node, &template, global, true)?
        };
        rules.push(Rule {
            layer: number(&g, &node, "layer")?,
            order: number(&g, &node, "order")?,
            once: boolean(&g, &node, "runOnce"),
            expected: g.objects(&node, &sh("expectedPredicate")),
            node,
            owners,
            global,
            kind,
            conditions,
        });
    }
    rules.sort_by(|a, b| cmp(&a.layer, &b.layer).then_with(|| cmp(&a.order, &b.order)));
    Ok(rules)
}

fn sparql_kind(
    g: &GraphView,
    node: &Term,
    template: &Term,
    global: bool,
    is_template: bool,
) -> Result<Kind, String> {
    let bodies = g.objects(template, &sh("construct"));
    let [Term::Literal(body)] = bodies.as_slice() else {
        return Err("Expected exactly one sh:construct string".into());
    };
    if body.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#string" {
        return Err("sh:construct must be a string".into());
    }
    let body = body.value();
    let prefixes = crate::model::collect_prefixes_from(g, &g.objects(template, &sh("prefixes")));
    let query = spargebra::SparqlParser::new()
        .parse_query(&format!("{prefixes}\n{body}"))
        .map_err(|e| e.to_string())?;
    let spargebra::Query::Construct { pattern, .. } = &query else {
        return Err("Rule query must be CONSTRUCT".into());
    };
    let mut params = vec![Vec::new()];
    let mut names = Vec::new();
    if is_template {
        for p in g.objects(template, &sh("parameter")) {
            let Term::NamedNode(path) =
                g.object(&p, &sh("path")).ok_or("Parameter without path")?
            else {
                return Err("Parameter path must be an IRI".into());
            };
            let name = path
                .as_str()
                .rsplit(['#', '/'])
                .next()
                .unwrap_or("")
                .to_string();
            names.push(name.clone());
            let values = g.objects(node, path.as_str());
            if values.is_empty() {
                if !boolean(g, &p, "optional") {
                    return Err(format!("Missing parameter {path} on {node}"));
                }
                continue;
            }
            params = params
                .into_iter()
                .flat_map(|row| {
                    values
                        .iter()
                        .map(|v| {
                            let mut r = row.clone();
                            r.push((name.clone(), v.clone()));
                            r
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
        }
    }
    if !global {
        names.push("this".into());
    }
    if !names.is_empty() {
        crate::sparql::check_pre_binding(
            pattern,
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .map_err(|e| format!("{e:?}"))?;
    }
    Ok(Kind::Sparql { query, params })
}

pub(crate) fn execute(
    data: &Graph,
    shapes: &Graph,
    sets: &[Term],
    all: bool,
) -> Result<Inference, String> {
    let mut model = ShapesModel::parse(shapes);
    let g = GraphView::new(shapes);
    check_processors(&g)?;
    let mut rules = parse(shapes, &mut model, selected(&g, sets, all)?)?;
    if !model.pre_binding_failures().is_empty() || !model.ill_formed().is_empty() {
        return Err("Ill-formed rule shapes or condition query".into());
    }
    if rules
        .iter()
        .any(|r| matches!(&r.kind, Kind::Sparql { query, .. } if clock::needed(query)))
    {
        let clock = sparq_engine::query(data, "SELECT (NOW() AS ?now) WHERE {}")?;
        let Some(Some(Term::Literal(now))) = clock.rows.first().and_then(|r| r.first()) else {
            return Err("NOW unavailable".into());
        };
        for rule in &mut rules {
            if let Kind::Sparql { query, .. } = &mut rule.kind {
                clock::freeze(query, now);
            }
        }
    }
    let base: FxHashSet<Triple> = triples(data).into_iter().collect();
    let mut inferred: FxHashSet<Triple> = FxHashSet::default();
    let mut iterations = 0;
    let mut start = 0;
    while start < rules.len() {
        let end = start
            + rules[start..]
                .partition_point(|r| cmp(&r.layer, &rules[start].layer) == Ordering::Equal);
        let layer = &rules[start..end];
        let snapshot = expand_graph(data, &inferred.iter().cloned().collect::<Vec<_>>());
        let (derived, failures) = crate::functions::with_validation(&snapshot, shapes, || {
            derive(data, shapes, &model, &inferred, layer)
        });
        if !failures.is_empty() {
            return Err("Derived expression evaluation failed".into());
        }
        let derived = derived?;
        iteration(
            data,
            shapes,
            &model,
            &base,
            &derived,
            &mut inferred,
            layer,
            true,
        )?;
        iterations += saturate(MAX_ITERATIONS, "SHACL 1.2 rule iteration limit exceeded", || {
            iteration(data, shapes, &model, &base, &derived, &mut inferred, layer, false)
        })?;
        start = end;
    }
    remove_annotated(&mut inferred, &sh("tempTriple"));
    let mut triples: Vec<_> = inferred.into_iter().collect();
    triples.sort_by_key(ToString::to_string);
    Ok(Inference {
        triples,
        iterations,
        capped: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn iteration(
    data: &Graph,
    shapes: &Graph,
    model: &ShapesModel,
    base: &FxHashSet<Triple>,
    derived: &[Triple],
    inferred: &mut FxHashSet<Triple>,
    rules: &[Rule],
    once: bool,
) -> Result<bool, String> {
    let mut changed = false;
    let mut start = 0;
    while start < rules.len() {
        let end = start
            + rules[start..]
                .partition_point(|r| cmp(&r.order, &rules[start].order) == Ordering::Equal);
        let snapshot = expand_graph(
            data,
            &inferred
                .iter()
                .cloned()
                .chain(derived.iter().cloned())
                .collect::<Vec<_>>(),
        );
        let view = GraphView::new(&snapshot);
        let (output, failures) = crate::functions::with_validation(&snapshot, shapes, || {
            let mut output = Vec::new();
            for rule in &rules[start..end] {
                if rule.once != once {
                    continue;
                }
                let foci = if rule.global {
                    vec![None]
                } else {
                    crate::view::dedup(
                        rule.owners
                            .iter()
                            .filter(|&&sid| !model.shapes[sid].deactivated)
                            .flat_map(|&sid| target_nodes(&view, model, sid)),
                    )
                    .into_iter()
                    .map(Some)
                    .collect()
                };
                for focus in foci {
                    if let Some(focus) = &focus {
                        if !rule
                            .conditions
                            .iter()
                            .filter(|&&sid| !model.shapes[sid].deactivated)
                            .all(|&sid| crate::eval::conforms_node(&snapshot, model, sid, focus))
                        {
                            continue;
                        }
                    }
                    output.extend(fire(&snapshot, shapes, rule, focus.as_ref())?);
                }
            }
            Ok::<_, String>(output)
        });
        if !failures.is_empty() {
            return Err("Rule condition evaluation failed".into());
        }
        for triple in output? {
            if !base.contains(&triple) && inferred.insert(triple) {
                changed = true;
            }
        }
        start = end;
    }
    Ok(changed)
}

fn fire(
    data: &Graph,
    shapes: &Graph,
    rule: &Rule,
    focus: Option<&Term>,
) -> Result<Vec<Triple>, String> {
    match &rule.kind {
        Kind::Triple(exprs) => {
            let mut values = Vec::new();
            for expr in exprs {
                values.push(match expr {
                    Some(e) => expression(data, shapes, e, focus, 0)?,
                    None => focus.cloned().into_iter().collect(),
                });
            }
            let mut out = Vec::new();
            for s in &values[0] {
                let Ok(subject) = NamedOrBlankNode::try_from(s.clone()) else {
                    continue;
                };
                for p in &values[1] {
                    let Term::NamedNode(predicate) = p else {
                        continue;
                    };
                    for object in &values[2] {
                        out.push(Triple::new(
                            subject.clone(),
                            predicate.clone(),
                            object.clone(),
                        ));
                    }
                }
            }
            Ok(out)
        }
        Kind::Sparql { query, params } => {
            let mut out = Vec::new();
            for row in params {
                let mut bindings: Vec<_> = row.iter().map(|(n, t)| (n.as_str(), t)).collect();
                if let Some(f) = focus {
                    bindings.push(("this", f));
                }
                out.extend(
                    crate::sparql::construct_bound(data, query, &bindings)
                        .map_err(|e| format!("{}: {e}", rule.node))?,
                );
            }
            Ok(out)
        }
    }
}

fn derive(
    data: &Graph,
    shapes: &Graph,
    model: &ShapesModel,
    inferred: &FxHashSet<Triple>,
    rules: &[Rule],
) -> Result<Vec<Triple>, String> {
    let snapshot = expand_graph(data, &inferred.iter().cloned().collect::<Vec<_>>());
    let view = GraphView::new(&snapshot);
    let g = GraphView::new(shapes);
    let predicates: FxHashSet<_> = rules
        .iter()
        .flat_map(|r| r.expected.iter().cloned())
        .collect();
    let mut out = FxHashSet::default();
    for (sid, shape) in model.shapes.iter().enumerate() {
        if shape.deactivated {
            continue;
        }
        let Some(crate::Path::Predicate(path)) = &shape.path else {
            continue;
        };
        if !predicates.contains(&iri(path)) {
            continue;
        }
        let foci = property_foci(&snapshot, shapes, model, sid, &mut FxHashSet::default())?;
        for focus in crate::view::dedup(foci) {
            let Ok(subject) = NamedOrBlankNode::try_from(focus.clone()) else {
                continue;
            };
            let mut values = view.objects(&focus, path);
            let mut derived = Vec::new();
            for expr in g.objects(&shape.node, &sh("values")) {
                derived.extend(expression(&snapshot, shapes, &expr, Some(&focus), 0)?);
            }
            values.extend(derived.iter().cloned());
            if values.is_empty() {
                for expr in g.objects(&shape.node, &sh("defaultValue")) {
                    derived.extend(expression(&snapshot, shapes, &expr, Some(&focus), 0)?);
                }
            }
            for value in derived {
                out.insert(Triple::new(
                    subject.clone(),
                    NamedNode::new_unchecked(path),
                    value,
                ));
            }
        }
    }
    Ok(out.into_iter().collect())
}

fn target_nodes(view: &GraphView, model: &ShapesModel, sid: usize) -> Vec<Term> {
    crate::view::dedup(
        crate::rules::focus_nodes(view, model, sid)
            .into_iter()
            .chain(view.subjects(&sh("shape"), &model.shapes[sid].node)),
    )
}

fn property_foci(
    data: &Graph,
    shapes: &Graph,
    model: &ShapesModel,
    sid: usize,
    visiting: &mut FxHashSet<usize>,
) -> Result<Vec<Term>, String> {
    if model.shapes[sid].deactivated {
        return Ok(Vec::new());
    }
    if !visiting.insert(sid) {
        return Err("Recursive derived property focus".into());
    }
    let view = GraphView::new(data);
    let g = GraphView::new(shapes);
    let mut out = target_nodes(&view, model, sid);
    for (parent_id, parent) in model.shapes.iter().enumerate() {
        if parent.deactivated || !parent.property_children.contains(&sid) {
            continue;
        }
        for focus in property_foci(data, shapes, model, parent_id, visiting)? {
            if let Some(path) = &parent.path {
                let mut values = path.values(&view, &focus);
                for expr in g.objects(&parent.node, &sh("values")) {
                    values.extend(expression(data, shapes, &expr, Some(&focus), 0)?);
                }
                if values.is_empty() {
                    for expr in g.objects(&parent.node, &sh("defaultValue")) {
                        values.extend(expression(data, shapes, &expr, Some(&focus), 0)?);
                    }
                }
                out.extend(values);
            } else {
                out.push(focus);
            }
        }
    }
    visiting.remove(&sid);
    Ok(crate::view::dedup(out))
}

fn remove_annotated(inferred: &mut FxHashSet<Triple>, predicate: &str) {
    let g = crate::graph_from_triples(inferred.iter().cloned());
    let view = GraphView::new(&g);
    let mut remove = FxHashSet::default();
    for [reifier, _, value] in view.triples(None, Some(predicate), None) {
        if !matches!(value, Term::Literal(l) if l == Literal::from(true)) {
            continue;
        }
        for t in view.objects(
            &reifier,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies",
        ) {
            if let Term::Triple(triple) = &t {
                remove.insert(*triple.clone());
            }
            for annotation in
                view.subjects("http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies", &t)
            {
                for [s, p, o] in view.triples(Some(&annotation), None, None) {
                    if let (Ok(s), Term::NamedNode(p)) = (NamedOrBlankNode::try_from(s), p) {
                        remove.insert(Triple::new(s, p, o));
                    }
                }
            }
        }
    }
    inferred.retain(|t| !remove.contains(t));
}

fn triples(data: &Graph) -> Vec<Triple> {
    GraphView::new(data)
        .triples(None, None, None)
        .into_iter()
        .map(|[s, p, o]| {
            let Term::NamedNode(p) = p else {
                unreachable!()
            };
            Triple::new(NamedOrBlankNode::try_from(s).expect("RDF subject"), p, o)
        })
        .collect()
}

fn expression(
    data: &Graph,
    shapes: &Graph,
    expr: &Term,
    focus: Option<&Term>,
    depth: usize,
) -> Result<Vec<Term>, String> {
    if depth >= 64 {
        return Err("Node expression nesting limit exceeded".into());
    }
    if *expr == iri(&sh("this")) {
        return Ok(focus.cloned().into_iter().collect());
    }
    if matches!(
        expr,
        Term::NamedNode(_) | Term::Literal(_) | Term::Triple(_)
    ) {
        return Ok(vec![expr.clone()]);
    }
    let g = GraphView::new(shapes);
    let eval = |e: &Term, f: Option<&Term>| expression(data, shapes, e, f, depth + 1);
    if g.predicate_objects(expr).is_empty() {
        return Ok(Vec::new());
    }
    if g.object(expr, crate::view::RDF_FIRST).is_some() {
        let mut out = Vec::new();
        for e in g.list_strict(expr).ok_or("Malformed expression list")? {
            out.extend(eval(&e, focus)?);
        }
        return Ok(out);
    }
    if let Some(path) = g
        .object(expr, &format!("{NEX}pathValues"))
        .or_else(|| g.object(expr, &sh("path")))
    {
        let path = crate::Path::parse(&g, &path)?;
        let f = g
            .object(expr, &format!("{NEX}focusNode"))
            .or_else(|| g.object(expr, &sh("nodes")));
        let foci = match f {
            Some(e) => eval(&e, focus)?,
            None => focus.cloned().into_iter().collect(),
        };
        if foci.is_empty() {
            return Ok(Vec::new());
        }
        if foci.len() != 1 {
            return Err("Path expression requires one focus node".into());
        }
        return Ok(path.values(&GraphView::new(data), &foci[0]));
    }
    if let Some(e) = g.object(expr, &format!("{NEX}count")) {
        return Ok(vec![Literal::from(eval(&e, focus)?.len() as i64).into()]);
    }
    if let Some(e) = g.object(expr, &format!("{NEX}instancesOf")) {
        let classes = eval(&e, focus)?;
        if classes.len() != 1 || !matches!(classes[0], Term::NamedNode(_)) {
            return Err("instancesOf requires one class".into());
        }
        return Ok(GraphView::new(data).instances_of(&classes[0]));
    }
    if let Some(args) = g.object(expr, &format!("{SPARQL}multiply")) {
        let args = g.list_strict(&args).ok_or("Malformed multiply arguments")?;
        if args.len() != 2 {
            return Err("multiply requires two arguments".into());
        }
        let a = eval(&args[0], focus)?;
        let b = eval(&args[1], focus)?;
        if a.len() > 1 || b.len() > 1 {
            return Err("multiply argument cardinality".into());
        }
        if a.is_empty() || b.is_empty() {
            return Ok(Vec::new());
        }
        let query = sparq_engine::query(
            data,
            &format!("SELECT (({} * {}) AS ?v) WHERE {{}}", a[0], b[0]),
        )?;
        let value = query
            .rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next().flatten())
            .ok_or("SPARQL multiply evaluation failed")?;
        return Ok(vec![value]);
    }
    if g.object(expr, &sh("select")).is_some() || g.object(expr, &sh("sparqlExpr")).is_some() {
        let q =
            crate::sparql::expression_query(&g, expr).ok_or("Invalid SPARQL node expression")?;
        let bindings: Vec<_> = focus.map(|f| ("this", f)).into_iter().collect();
        return crate::sparql::expression_bindings(&q, data, &bindings);
    }
    Err(format!("Unsupported node expression {expr}"))
}
