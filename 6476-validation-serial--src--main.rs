// [GPT-6 Astra] Public-callback reproduction for issue #6476; no production instrumentation.
use oxrdf::{Literal, Term};
use sparq_core::Graph;
use sparq_engine::{query, query_with_budget, query_with_functions_and_budget, FunctionRegistry, QueryBudget};
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

#[derive(Clone, Copy, Debug)]
enum Inner { None, Unlimited, ExplicitBudget, ParseError }

fn run(inner: Inner) -> bool {
    let graph = Graph::load_str("", "turtle").expect("empty graph fixture");
    let nested = Graph::load_str("", "turtle").expect("empty nested fixture");
    let cancel = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_cancel = Arc::clone(&cancel);
    let callback_calls = Arc::clone(&calls);
    let thread = std::thread::current().id();
    let mut registry = FunctionRegistry::new();
    registry.register("urn:nested", move |_| {
        assert_eq!(std::thread::current().id(), thread, "witness must run on the installing thread");
        callback_calls.fetch_add(1, Ordering::Relaxed);
        match inner {
            Inner::None => {},
            Inner::Unlimited => assert_eq!(query(&nested, "ASK {}").unwrap().rows.len(), 1),
            Inner::ExplicitBudget => {
                let budget = QueryBudget { max_rows: Some(1), ..QueryBudget::unlimited() };
                assert_eq!(query_with_budget(&nested, "ASK {}", &budget).unwrap().rows.len(), 1);
            },
            Inner::ParseError => assert!(query(&nested, "not a SPARQL query").is_err()),
        }
        callback_cancel.store(true, Ordering::Relaxed);
        Ok(Term::Literal(Literal::from(7)))
    });
    let budget = QueryBudget::cancelled_by(Arc::clone(&cancel));
    let result = query_with_functions_and_budget(&graph, "SELECT (<urn:nested>() AS ?value) WHERE {}", &registry, &budget);
    assert_eq!(calls.load(Ordering::Relaxed), 1, "callback must execute exactly once");
    assert!(cancel.load(Ordering::Relaxed));
    let error = result.as_ref().err().map(String::as_str);
    let enforced = error == Some("query budget exceeded (cancelled)");
    if let Ok(result) = &result {
        assert_eq!(result.rows, vec![vec![Some(Term::Literal(Literal::from(7)))]]);
    }
    println!("case={inner:?} callback_calls=1 same_thread=true cancel=true enforced={enforced} result={result:?}");
    match inner {
        Inner::None | Inner::ParseError => assert!(enforced, "control requires a real post-callback budget poll"),
        Inner::Unlimited | Inner::ExplicitBudget => {},
    }
    // A subsequent independent top-level query must remain usable.
    assert_eq!(query(&graph, "ASK {}").unwrap().rows.len(), 1);
    enforced
}

fn main() {
    let results = [Inner::None, Inner::Unlimited, Inner::ExplicitBudget, Inner::ParseError].map(run);
    println!("outer_cancellation_enforced={results:?}");
    if std::env::args().any(|arg| arg == "--require-restoration") {
        assert!(results.iter().all(|&enforced| enforced), "nested public query erased outer cancellation budget");
    }
}
