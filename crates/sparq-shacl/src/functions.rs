//! Query-local SHACL function definitions and processing failures.
use crate::model::sh;
use crate::view::GraphView;
use oxrdf::{Literal, NamedNode, Term};
use sparq_core::Graph;
use sparq_engine::{FunctionRegistry, PreparedQuery, QueryResult};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

pub(crate) const BLANK: &str = "urn:sparq:shacl:prebound-blank";
const SHNEX: &str = "http://www.w3.org/ns/shacl-node-expr#";

struct Context {
    source: Arc<Graph>,
    graph_name: Term,
    messages: rustc_hash::FxHashMap<Term, Vec<Term>>,
    data: Option<Arc<Graph>>,
    dataset: std::sync::OnceLock<Arc<Graph>>,
    registry: std::sync::OnceLock<FunctionRegistry>,
    failures: std::sync::Mutex<Vec<Term>>,
}
thread_local! {
    static BINDING_TOKEN: String = oxrdf::BlankNode::default().as_str().to_string();
    static FUNCTION_DEPTH: Cell<usize> = const { Cell::new(0) };
    static CONTEXT: RefCell<Vec<Arc<Context>>> = const { RefCell::new(Vec::new()) };
    #[cfg(test)]
    static BUILD_COUNTS: Cell<[u64; 4]> = const { Cell::new([0; 4]) };
}

// Counts the registry, shapes dataset, caller-data forks and cached-data forks.
#[cfg(test)]
fn bump_build(index: usize) {
    BUILD_COUNTS.with(|c| {
        let mut counts = c.get();
        counts[index] += 1;
        c.set(counts);
    });
}

#[cfg(test)]
fn build_counts() -> [u64; 4] {
    BUILD_COUNTS.with(Cell::get)
}

fn fork_data(data: &Graph) -> Graph {
    #[cfg(test)]
    bump_build(2);
    data.fork()
}

fn with_context<T>(context: &Arc<Context>, f: impl FnOnce() -> T) -> T {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            CONTEXT.with(|c| {
                c.borrow_mut().pop();
            });
        }
    }
    CONTEXT.with(|c| c.borrow_mut().push(context.clone()));
    let _guard = Guard;
    f()
}

pub(crate) fn with_validation<T>(
    data: &Graph,
    source: &Graph,
    f: impl FnOnce() -> T,
) -> (T, Vec<Term>) {
    let source = Arc::new(source.fork());
    let functions = GraphView::new(&source)
        .instances_of(&NamedNode::new_unchecked(sh("ListParameterExpressionFunction")).into());
    let mut messages: rustc_hash::FxHashMap<Term, Vec<Term>> = Default::default();
    for [node, _, message] in GraphView::new(&source).triples(None, Some(&sh("message")), None) {
        messages.entry(node).or_default().push(message);
    }
    let context = Arc::new(Context {
        source,
        messages,
        graph_name: oxrdf::BlankNode::default().into(),
        data: (!functions.is_empty()).then(|| Arc::new(fork_data(data))),
        dataset: std::sync::OnceLock::new(),
        registry: std::sync::OnceLock::new(),
        failures: std::sync::Mutex::new(Vec::new()),
    });
    let registry = build_registry(Some(&context), functions);
    context.registry.set(registry).ok().unwrap();
    let result = with_context(&context, || {
        sparq_engine::with_functions(context.registry.get().unwrap(), f)
    });
    let failures = std::mem::take(&mut *context.failures.lock().unwrap());
    (result, failures)
}

pub(crate) fn record_failure(node: &Term) {
    CONTEXT.with(|c| {
        if let Some(ctx) = c.borrow().last() {
            ctx.failures.lock().unwrap().push(node.clone());
        }
    });
}

pub(crate) fn messages(node: &Term) -> Vec<Term> {
    CONTEXT.with(|c| {
        c.borrow()
            .last()
            .and_then(|ctx| ctx.messages.get(node).cloned())
            .unwrap_or_default()
    })
}

pub(crate) fn shapes_graph() -> Option<Term> {
    CONTEXT.with(|c| c.borrow().last().map(|ctx| ctx.graph_name.clone()))
}

fn with_dataset<T>(data: &Graph, shapes_graph: bool, f: impl FnOnce(&Graph) -> T) -> T {
    if !shapes_graph && CONTEXT.with(|c| !c.borrow().is_empty()) {
        return f(data);
    }
    let context = CONTEXT.with(|c| c.borrow().last().cloned());
    if let Some(ctx) = context {
        if shapes_graph {
            let dataset = ctx.dataset.get_or_init(|| {
                #[cfg(test)]
                bump_build(1);
                let mut dataset = if let Some(snapshot) = &ctx.data {
                    #[cfg(test)]
                    bump_build(3);
                    snapshot.fork()
                } else {
                    fork_data(data)
                };
                dataset
                    .add_named(ctx.graph_name.clone(), ctx.source.fork())
                    .expect("fresh shapes graph name");
                Arc::new(dataset)
            });
            f(dataset)
        } else {
            f(data)
        }
    } else {
        // Internal pre-binding unit tests also run outside a validation context.
        sparq_engine::with_functions(&build_registry(None, Vec::new()), || f(data))
    }
}

pub(crate) fn query(
    data: &Graph,
    query: &PreparedQuery,
    shapes_graph: bool,
) -> Result<QueryResult, String> {
    with_dataset(data, shapes_graph, |data| {
        sparq_engine::query_prepared(data, query)
    })
}

pub(crate) fn ask(data: &Graph, query: &PreparedQuery) -> Result<bool, String> {
    with_dataset(data, false, |data| sparq_engine::ask_prepared(data, query))
}

pub(crate) fn binding_token() -> String {
    BINDING_TOKEN.with(Clone::clone)
}

fn build_registry(context: Option<&Arc<Context>>, functions: Vec<Term>) -> FunctionRegistry {
    #[cfg(test)]
    bump_build(0);
    let mut registry = FunctionRegistry::new();
    let token = binding_token();
    registry.register(BLANK, move |args| match args {
        [Term::Literal(l), Term::Literal(key)] if key.value() == token => {
            oxrdf::BlankNode::new(l.value())
                .map(Term::BlankNode)
                .map_err(|e| e.to_string())
        }
        _ => Err("Invalid pre-bound blank node".into()),
    });
    if let Some(context) = context {
        for function in functions {
            let Term::NamedNode(iri) = &function else {
                continue;
            };
            // The engine resolves its built-ins before consulting this registry.
            if iri.as_str() == BLANK {
                continue;
            }
            let context = Arc::downgrade(context);
            registry.register(iri.as_str().to_string(), move |args| {
                struct DepthGuard;
                impl Drop for DepthGuard {
                    fn drop(&mut self) {
                        FUNCTION_DEPTH.with(|d| d.set(d.get() - 1));
                    }
                }
                if FUNCTION_DEPTH.with(|d| d.get() >= 8) {
                    return Err("Recursive custom function".into());
                }
                FUNCTION_DEPTH.with(|d| d.set(d.get() + 1));
                let _guard = DepthGuard;
                if args.iter().any(|a| matches!(a, Term::BlankNode(_))) {
                    return Err("Blank nodes are not constant node expressions".into());
                }
                let context = context.upgrade().ok_or("Function outside validation")?;
                let shapes = GraphView::new(&context.source);
                let parameters = shapes.objects(&function, &sh("parameter"));
                for p in parameters {
                    let index = shapes.object(&p, &sh("path")).and_then(|t| match t {
                        Term::NamedNode(n) => n.as_str().strip_prefix(&format!("{SHNEX}arg")).and_then(|s| s.parse::<usize>().ok()),
                        _ => None,
                    });
                    let optional = matches!(shapes.object(&p, &sh("optional")), Some(Term::Literal(l)) if matches!(l.value(), "true" | "1"));
                    if !optional && index.is_some_and(|i| i >= args.len()) {
                        return Err("Missing mandatory function argument".into());
                    }
                }
                let body = shapes.object(&function, &sh("bodyExpression")).ok_or("Missing function body")?;
                let result = with_context(&context, || {
                    eval_body(&context.source, context.data.as_ref().unwrap(), &body, &function, args, 0)
                })?;
                match result.as_slice() {
                    [one] => Ok(one.clone()),
                    _ => Err("A custom SPARQL function must return exactly one node".into()),
                }
            });
        }
    }
    registry
}

fn eval_body(
    source: &Graph,
    data: &Graph,
    expr: &Term,
    focus: &Term,
    args: &[Term],
    depth: usize,
) -> Result<Vec<Term>, String> {
    if depth >= 64 {
        return Err("Recursive function expression".into());
    }
    if matches!(expr, Term::NamedNode(_) | Term::Literal(_)) {
        return Ok(vec![expr.clone()]);
    }
    let shapes = GraphView::new(source);
    if let Some(Term::Literal(index)) = shapes.object(expr, &format!("{SHNEX}arg")) {
        let index = index.value().parse::<usize>().map_err(|e| e.to_string())?;
        return Ok(args.get(index).cloned().into_iter().collect());
    }
    for op in ["count", "instancesOf"] {
        if let Some(operand) = shapes.object(expr, &format!("{SHNEX}{op}")) {
            let values = eval_body(source, data, &operand, focus, args, depth + 1)?;
            return if op == "count" {
                Ok(vec![Literal::from(values.len() as i64).into()])
            } else {
                let view = GraphView::new(data);
                Ok(crate::view::dedup(
                    values.iter().flat_map(|v| view.instances_of(v)),
                ))
            };
        }
    }
    if let Some(prepared) = crate::sparql::expression_query(&shapes, expr) {
        let mut owned = vec![("this".to_string(), focus.clone())];
        owned.extend(
            args.iter()
                .enumerate()
                .map(|(i, a)| (format!("arg{i}"), a.clone())),
        );
        let bindings: Vec<_> = owned.iter().map(|(n, t)| (n.as_str(), t)).collect();
        return crate::sparql::expression_bindings(&prepared, data, &bindings);
    }
    Err("Unsupported custom function body expression".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_builds_once_for_many_per_focus_queries() {
        let mut data = String::new();
        for i in 0..48 {
            data.push_str(&format!("<urn:n{i}> <urn:p> {i} .\n"));
        }
        // Keep the caller's graph flat: freezing it would conceal repeated copies.
        let data = Graph::load_str(&data, "turtle").unwrap();
        for (body, functions, expected) in [
            ("$this <urn:p> ?v", "", [1, 0, 0, 0]),
            ("GRAPH $shapesGraph { ?s ?p ?o }", "", [1, 1, 1, 0]),
            ("FILTER(<urn:identity>(1) = 1)",
             "<urn:identity> a sh:ListParameterExpressionFunction; sh:bodyExpression [shnex:arg 0] .", [1, 0, 1, 0]),
            ("GRAPH $shapesGraph { ?s ?p ?o } FILTER(<urn:identity>(1) = 1)",
             "<urn:identity> a sh:ListParameterExpressionFunction; sh:bodyExpression [shnex:arg 0] .", [1, 1, 1, 1]),
            ("FILTER(<urn:nested>(1) = 1)",
             "<urn:nested> a sh:ListParameterExpressionFunction; sh:bodyExpression [sh:select \"SELECT ?arg0 WHERE {}\"] .", [1, 0, 1, 0]),
        ] {
            let shapes = Graph::load_str(&format!(r#"
                @prefix sh: <http://www.w3.org/ns/shacl#> .
                @prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> .
                <urn:S> sh:targetSubjectsOf <urn:p>; sh:sparql [sh:select """
                    SELECT $this WHERE {{ {body} }} LIMIT 1
                """] . {functions}
            "#), "turtle").unwrap();
            let model = crate::ShapesModel::parse(&shapes);
            for _ in 0..2 {
                let before = build_counts();
                let queries = crate::sparql::exec_count();
                let report = crate::validate_with_model(&data, &model);
                assert_eq!(report.results.len(), 48, "{body}");
                assert_eq!(crate::sparql::exec_count() - queries, 48, "per-focus path must run");
                let after = build_counts();
                let delta: [u64; 4] = std::array::from_fn(|i| after[i] - before[i]);
                assert_eq!(delta, expected, "{body}");
            }
        }
    }
    #[test]
    fn component_validators_reuse_the_validation_registry() {
        let mut text = String::new();
        for i in 0..48 {
            text.push_str(&format!("<urn:n{i}> <urn:p> {i} .\n"));
        }
        let data = Graph::load_str(&text, "turtle").unwrap();
        for validator in [
            "sh:select \"SELECT $this WHERE {FILTER(<urn:identity>(1) = 1)}\"",
            "sh:ask \"ASK {FILTER(<urn:identity>(1) != 1)}\"",
        ] {
            let shapes = Graph::load_str(&format!(r#"
                @prefix sh: <http://www.w3.org/ns/shacl#> .
                @prefix shnex: <http://www.w3.org/ns/shacl-node-expr#> .
                <urn:identity> a sh:ListParameterExpressionFunction; sh:bodyExpression [shnex:arg 0] .
                <urn:C> a sh:ConstraintComponent; sh:parameter [sh:path <urn:parameter>];
                    sh:validator [{validator}] .
                <urn:S> sh:targetSubjectsOf <urn:p>; <urn:parameter> true .
            "#), "turtle").unwrap();
            let before = build_counts();
            let report = crate::validate(&data, &shapes);
            assert_eq!(report.results.len(), 48, "{validator}");
            let after = build_counts();
            assert_eq!(
                std::array::from_fn::<_, 4, _>(|i| after[i] - before[i]),
                [1, 0, 1, 0]
            );
        }
    }
}
