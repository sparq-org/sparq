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
    failures: Vec<Term>,
}
thread_local! {
    static BINDING_TOKEN: String = oxrdf::BlankNode::default().as_str().to_string();
    static FUNCTION_DEPTH: Cell<usize> = const { Cell::new(0) };
    static CONTEXT: RefCell<Vec<Context>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn with_validation<T>(source: &Graph, f: impl FnOnce() -> T) -> (T, Vec<Term>) {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            CONTEXT.with(|c| {
                c.borrow_mut().pop();
            });
        }
    }
    CONTEXT.with(|c| {
        c.borrow_mut().push(Context {
            source: Arc::new(source.fork()),
            graph_name: oxrdf::BlankNode::default().into(),
            failures: Vec::new(),
        })
    });
    let guard = Guard;
    let result = f();
    let failures =
        CONTEXT.with(|c| std::mem::take(&mut c.borrow_mut().last_mut().unwrap().failures));
    drop(guard);
    (result, failures)
}

pub(crate) fn record_failure(node: &Term) {
    CONTEXT.with(|c| {
        if let Some(ctx) = c.borrow_mut().last_mut() {
            ctx.failures.push(node.clone());
        }
    });
}

pub(crate) fn messages(node: &Term) -> Vec<Term> {
    CONTEXT.with(|c| {
        c.borrow()
            .last()
            .map(|ctx| GraphView::new(&ctx.source).objects(node, &sh("message")))
            .unwrap_or_default()
    })
}

pub(crate) fn shapes_graph() -> Option<Term> {
    CONTEXT.with(|c| c.borrow().last().map(|ctx| ctx.graph_name.clone()))
}

fn with_dataset<T>(data: &Graph, f: impl FnOnce(&Graph) -> T) -> T {
    let context = CONTEXT.with(|c| {
        c.borrow()
            .last()
            .map(|ctx| (ctx.source.clone(), ctx.graph_name.clone()))
    });
    if let Some((source, name)) = context {
        if data.named_graph(&name).is_some() {
            return f(data);
        }
        let mut dataset = data.fork();
        dataset
            .add_named(name, source.fork())
            .expect("fresh shapes graph name");
        f(&dataset)
    } else {
        f(data)
    }
}

pub(crate) fn query(data: &Graph, query: &PreparedQuery) -> Result<QueryResult, String> {
    with_dataset(data, |data| {
        with_registry(data, || sparq_engine::query_prepared(data, query))
    })
}

pub(crate) fn ask(data: &Graph, query: &PreparedQuery) -> Result<bool, String> {
    with_dataset(data, |data| {
        with_registry(data, || sparq_engine::ask_prepared(data, query))
    })
}

pub(crate) fn binding_token() -> String {
    BINDING_TOKEN.with(Clone::clone)
}

pub(crate) fn with_registry<T>(data: &Graph, f: impl FnOnce() -> T) -> T {
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
    let source = CONTEXT.with(|c| c.borrow().last().map(|ctx| ctx.source.clone()));
    if let Some(source) = source {
        let shapes = GraphView::new(&source);
        let functions = shapes
            .instances_of(&NamedNode::new_unchecked(sh("ListParameterExpressionFunction")).into());
        if !functions.is_empty() {
            let data = Arc::new(data.fork());
            for function in functions {
                let Term::NamedNode(iri) = &function else {
                    continue;
                };
                // The engine resolves its built-ins before consulting this registry.
                if iri.as_str() == BLANK {
                    continue;
                }
                let source = source.clone();
                let data = data.clone();
                registry.register(iri.as_str().to_string(), move |args| {
                    struct DepthGuard;
                    impl Drop for DepthGuard {
                        fn drop(&mut self) { FUNCTION_DEPTH.with(|d| d.set(d.get() - 1)); }
                    }
                    if FUNCTION_DEPTH.with(|d| d.get() >= 8) { return Err("Recursive custom function".into()); }
                    FUNCTION_DEPTH.with(|d| d.set(d.get() + 1));
                    let _guard = DepthGuard;
                    if args.iter().any(|a| matches!(a, Term::BlankNode(_))) {
                        return Err("Blank nodes are not constant node expressions".into());
                    }
                    let shapes = GraphView::new(&source);
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
                    let (result, _) = with_validation(&source, || eval_body(&source, &data, &body, &function, args, 0));
                    let result = result?;
                    match result.as_slice() {
                        [one] => Ok(one.clone()),
                        _ => Err("A custom SPARQL function must return exactly one node".into()),
                    }
                });
            }
        }
    }
    sparq_engine::with_functions(&registry, f)
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
