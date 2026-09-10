    // [GPT-6 Astra] Private: callers cannot forget or drop a frame out of order.
    struct Guard<'a> {
        previous: Limits,
        exceeded: Option<&'static str>,
        _budget: std::marker::PhantomData<&'a QueryBudget>,
        _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.set(self.previous));
            EXCEEDED.with(|e| e.set(self.exceeded));
        }
    }

    fn install(b: &QueryBudget) -> Guard<'_> {
        let cancel = b
            .cancel
            .as_ref()
            .map(|flag| CancelPtr(NonNull::from(flag.as_ref())));
        #[cfg(not(target_arch = "wasm32"))]
        let on = b.deadline.is_some()
            || b.max_rows.is_some()
            || b.max_bytes.is_some()
            || cancel.is_some();
        #[cfg(target_arch = "wasm32")]
        let on = b.max_rows.is_some() || b.max_bytes.is_some() || cancel.is_some();
        let previous = ACTIVE.with(|a| {
            a.replace(Limits {
                on,
                #[cfg(not(target_arch = "wasm32"))]
                deadline: b.deadline,
                max_rows: b.max_rows.unwrap_or(usize::MAX),
                max_bytes: b.max_bytes.unwrap_or(usize::MAX),
                byte_width: BYTES_PER_ID,
                extra_bytes: 0,
                cancel,
            })
        });
        let exceeded = EXCEEDED.with(|e| e.replace(None));
        Guard {
            previous,
            exceeded,
            _budget: std::marker::PhantomData,
            _not_send: std::marker::PhantomData,
        }
    }

    /// [GPT-6 Astra] Runs a child budget, restoring its parent on return or unwind.
    ///
    /// Guard never escapes this frame. This enforces LIFO even for reentrant
    /// callbacks, keeps each borrowed cancellation owner alive, and restores idle
    /// OFF/None after a top-level call. Aborting panics have no continuation.
    pub(crate) fn with_budget<T>(b: &QueryBudget, f: impl FnOnce() -> T) -> T {
        let _guard = install(b);
        f()
    }

        fn assert_state(want: Limits, sticky: Option<&'static str>) {
            let got = ACTIVE.with(Cell::get);
            assert_eq!(got.on, want.on);
            assert_eq!(got.max_rows, want.max_rows);
            assert_eq!(got.max_bytes, want.max_bytes);
            assert_eq!(got.byte_width, want.byte_width);
            assert_eq!(got.extra_bytes, want.extra_bytes);
            assert_eq!(got.cancel.map(|p| p.0), want.cancel.map(|p| p.0));
            #[cfg(not(target_arch = "wasm32"))]
            assert_eq!(got.deadline, want.deadline);
            assert_eq!(EXCEEDED.with(Cell::get), sticky);
        }

        #[test]
        fn public_extension_nested_query_preserves_outer_cancel() {
            use oxrdf::{Literal, Term};
            use sparq_core::Graph;
            use std::sync::atomic::AtomicUsize;
            for inner in 0..5 {
                let graph = Graph::load_str("", "turtle").unwrap();
                let nested = Graph::load_str("", "turtle").unwrap();
                let flag = Arc::new(AtomicBool::new(false));
                let calls = Arc::new(AtomicUsize::new(0));
                let callback_flag = Arc::clone(&flag);
                let callback_calls = Arc::clone(&calls);
                let owner = std::thread::current().id();
                let mut functions = crate::FunctionRegistry::new();
                functions.register("urn:nested", move |_| {
                    // Pin this one-row plan to serial callback evaluation: the
                    // nested call must overwrite the same TLS as the outer query.
                    assert_eq!(std::thread::current().id(), owner);
                    callback_calls.fetch_add(1, Ordering::Relaxed);
                    match inner {
                        0 => {}
                        1 => {
                            assert_eq!(crate::query(&nested, "ASK {}").unwrap().rows.len(), 1);
                        }
                        2 => {
                            let child = QueryBudget {
                                max_rows: Some(1),
                                ..QueryBudget::unlimited()
                            };
                            assert_eq!(
                                crate::query_with_budget(&nested, "ASK {}", &child)
                                    .unwrap()
                                    .rows
                                    .len(),
                                1
                            );
                        }
                        3 => {
                            // Parse rejection occurs before a child budget is installed.
                            assert!(crate::query(&nested, "not SPARQL").is_err());
                        }
                        _ => {
                            // [GPT-6 Astra] This valid graph form reaches the error
                            // arm inside query_prepared_with_budget's budget scope.
                            let sparql = "CONSTRUCT { ?s ?p ?o } WHERE {}";
                            assert!(crate::PreparedQuery::parse(sparql).unwrap().is_graph_form());
                            let outer = ACTIVE.with(Cell::get);
                            assert_eq!(
                                crate::query(&nested, sparql).unwrap_err(),
                                "only SELECT and ASK queries are supported"
                            );
                            assert_state(outer, None);
                        }
                    }
                    callback_flag.store(true, Ordering::Relaxed);
                    Ok(Term::Literal(Literal::from(7)))
                });
                let result = crate::query_with_functions_and_budget(
                    &graph,
                    "SELECT (<urn:nested>() AS ?value) WHERE {}",
                    &functions,
                    &QueryBudget::cancelled_by(flag),
                );
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(
                    result.unwrap_err(),
                    "query budget exceeded (cancelled)",
                    "inner={}",
                    inner
                );
                assert_eq!(crate::query(&graph, "ASK {}").unwrap().rows.len(), 1);
            }
        }