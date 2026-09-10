### QUERY_BASE storage

[crates/sparq-engine/src/exec.rs:15023-15027](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L15023-L15027)

```rust
thread_local! {
    /// The query's BASE IRI (when declared), used by IRI()/URI() to resolve relative
    /// references. Set by the `lib.rs` query entry points after parsing.
    static QUERY_BASE: std::cell::RefCell<Option<oxiri::Iri<String>>> = const { std::cell::RefCell::new(None) };
}
```

### set_query_base

[crates/sparq-engine/src/exec.rs:15031-15033](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L15031-L15033)

```rust
pub(crate) fn set_query_base(base: Option<&str>) {
    QUERY_BASE.with(|b| *b.borrow_mut() = base.and_then(|s| oxiri::Iri::parse(s.to_string()).ok()));
}
```

### resolve_iri

[crates/sparq-engine/src/exec.rs:15037-15047](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L15037-L15047)

```rust
fn resolve_iri(s: &str) -> Option<oxrdf::NamedNode> {
    if let Ok(abs) = oxiri::Iri::parse(s.to_string()) {
        return Some(oxrdf::NamedNode::new_unchecked(abs.into_inner()));
    }
    QUERY_BASE.with(|b| {
        b.borrow()
            .as_ref()
            .and_then(|base| base.resolve(s).ok())
            .map(|iri| oxrdf::NamedNode::new_unchecked(iri.into_inner()))
    })
}
```

### IRI expression operand then resolution

[crates/sparq-engine/src/exec.rs:14238-14246](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L14238-L14246)

```rust
        F::Iri => match ev(0)? {
            // An IRI argument passes through unchanged.
            Value::Term(Term::NamedNode(n)) => Value::Term(Term::NamedNode(n)),
            v => match str_lit(&v) {
                // String literal: absolute IRIs pass; relative ones resolve against BASE.
                Some((s, None)) => resolve_iri(&s).map(|n| Value::Term(Term::NamedNode(n))).unwrap_or(Value::Error),
                _ => Value::Error,
            },
        },
```

### query_prepared_with_budget

[crates/sparq-engine/src/lib.rs:1032-1054](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L1032-L1054)

```rust
pub fn query_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    exec::budget::with_budget(budget, || {
        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
        match q {
            Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
            // ASK as a QueryResult: zero variables, and one (empty) row iff the pattern
            // is satisfiable — the standard "unit row" encoding of a boolean result.
            Query::Ask { pattern, .. } => Ok(QueryResult {
                vars: Vec::new(),
                rows: if exec::eval_ask(graph, pattern)? { vec![Vec::new()] } else { Vec::new() },
            }),
            _ => Err("only SELECT and ASK queries are supported".into()),
        }
    })
}
```

### ask_prepared_with_budget

[crates/sparq-engine/src/lib.rs:1074-1086](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L1074-L1086)

```rust
pub fn ask_prepared_with_budget(graph: &Graph, prepared: &PreparedQuery, budget: &QueryBudget) -> Result<bool, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    exec::budget::with_budget(budget, || {
        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
        match q {
            Query::Ask { pattern, .. } => exec::eval_ask(graph, pattern),
            _ => Err("ask() requires an ASK query".into()),
        }
    })
}
```

### query_json_prepared_with_budget

[crates/sparq-engine/src/lib.rs:1107-1125](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L1107-L1125)

```rust
pub fn query_json_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
) -> Result<String, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    exec::budget::with_budget(budget, || {
        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
        match q {
            Query::Select { pattern, .. } => exec::eval_select_json(graph, pattern),
            // The SPARQL 1.1 JSON results boolean form.
            Query::Ask { pattern, .. } => Ok(format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)),
            _ => Err("only SELECT and ASK queries are supported".into()),
        }
    })
}
```

### query_json_chunks_with_budget

[crates/sparq-engine/src/lib.rs:1136-1152](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L1136-L1152)

```rust
pub fn query_json_chunks_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<Vec<String>, String> {
    let prepared = PreparedQuery::parse(sparql)?;
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    exec::budget::with_budget(budget, || {
        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
        match q {
            Query::Select { pattern, .. } => exec::eval_select_json_chunks(graph, pattern, Some(JSON_CHUNK_BYTES)),
            Query::Ask { pattern, .. } => {
                Ok(vec![format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?)])
            }
            _ => Err("only SELECT and ASK queries are supported".into()),
        }
    })
}
```

### query_json_stream_prepared_with_budget

[crates/sparq-engine/src/lib.rs:1188-1212](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L1188-L1212)

```rust
pub fn query_json_stream_prepared_with_budget(
    graph: &Graph,
    prepared: &PreparedQuery,
    budget: &QueryBudget,
    mut sink: impl FnMut(String) -> std::ops::ControlFlow<()>,
) -> Result<(), String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    exec::budget::with_budget(budget, || {
        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
        match q {
            Query::Select { pattern, .. } => {
                exec::eval_select_json_emit(graph, pattern, Some(JSON_CHUNK_BYTES), &mut sink)
            }
            Query::Ask { pattern, .. } => {
                let doc = format!("{{\"head\":{{}},\"boolean\":{}}}", exec::eval_ask(graph, pattern)?);
                let _ = sink(doc);
                Ok(())
            }
            _ => Err("only SELECT and ASK queries are supported".into()),
        }
    })
}
```

### count_prepared_with_budget

[crates/sparq-engine/src/lib.rs:1232-1246](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L1232-L1246)

```rust
pub fn count_prepared_with_budget(graph: &Graph, prepared: &PreparedQuery, budget: &QueryBudget) -> Result<usize, String> {
    let q = &prepared.query;
    let active = active_dataset(graph, q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = view_scope(&active);
    exec::budget::with_budget(budget, || {
        exec::set_query_base(q.base_iri().map(|b| b.as_str()));
        match q {
            Query::Select { pattern, .. } => exec::count_select(graph, pattern),
            // An ASK counts its unit row: 1 when satisfiable, 0 otherwise.
            Query::Ask { pattern, .. } => Ok(usize::from(exec::eval_ask(graph, pattern)?)),
            _ => Err("only SELECT and ASK queries are supported".into()),
        }
    })
}
```

### explain

[crates/sparq-engine/src/explain.rs:44-64](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/explain.rs#L44-L64)

```rust
pub fn explain(graph: &Graph, sparql: &str) -> Result<String, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    // [OPUS-4.8] (sq-7d3dj.30.1) EXPLAIN the ACTUAL executed plan: apply the same
    // pre-execution algebra rewrite `PreparedQuery::parse` does (feature-gated).
    #[cfg(feature = "algebra-rewrite")]
    let q = crate::rewrite::rewrite_query(q);
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    let (form, pattern) = query_form_pattern(&q);
    let mut out = String::new();
    let _ = writeln!(out, "EXPLAIN ({form}) — planning-only dry run; nothing is executed.");
    let _ = writeln!(
        out,
        "Cardinalities are index-range estimates; join strategies marked (predicted) depend on actual row counts at run time."
    );
    let _ = writeln!(out, "Plan:");
    render_pattern(graph, pattern, &mut out, 1)?;
    Ok(out)
}
```

### explain_analyze_with_budget

[crates/sparq-engine/src/explain.rs:73-115](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/explain.rs#L73-L115)

```rust
pub fn explain_analyze_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<String, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    // [OPUS-4.8] (sq-7d3dj.30.1) ANALYZE the ACTUAL executed plan (feature-gated rewrite).
    #[cfg(feature = "algebra-rewrite")]
    let q = crate::rewrite::rewrite_query(q);
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    let (form, pattern) = query_form_pattern(&q);
    if !matches!(q, Query::Select { .. } | Query::Ask { .. }) {
        return Err("EXPLAIN ANALYZE supports SELECT and ASK queries only (use EXPLAIN for CONSTRUCT/DESCRIBE)".into());
    }

    let mut out = String::new();
    let _ = writeln!(out, "EXPLAIN ANALYZE ({form}) — plan below, then the per-operator execution trace.");
    let _ = writeln!(out, "Plan:");
    render_pattern(graph, pattern, &mut out, 1)?;

    // Execute under the budget with the operator trace installed.
    exec::budget::with_budget(budget, || {
        let _tguard = exec::trace::install();
        #[cfg(not(target_arch = "wasm32"))]
        let start = std::time::Instant::now();
        let total_rows = match &q {
            Query::Select { pattern, .. } => exec::eval_select(graph, pattern)?.rows.len(),
            Query::Ask { pattern, .. } => usize::from(exec::eval_ask(graph, pattern)?),
            _ => unreachable!(),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let total_nanos = start.elapsed().as_nanos() as u64;
        #[cfg(target_arch = "wasm32")]
        let total_nanos = 0u64;
        let nodes = exec::trace::take();

        let _ = writeln!(out, "Execution trace (operator → output rows, wall time):");
        for n in &nodes {
            let _ = writeln!(out, "{}{}  rows={}  time={}", indent(n.depth + 1), n.label, n.rows, fmt_nanos(n.nanos));
        }
        let _ = writeln!(out, "Total: {} result row(s) in {}", total_rows, fmt_nanos(total_nanos));
        Ok(out)
    })
}
```

### explain_plan

[crates/sparq-engine/src/explain_json.rs:191-199](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/explain_json.rs#L191-L199)

```rust
pub fn explain_plan(graph: &Graph, sparql: &str) -> Result<PlanNode, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    let pattern = query_pattern(&q);
    Ok(plan_from_pattern(graph, pattern))
}
```

### explain_plan_analyze_with_budget

[crates/sparq-engine/src/explain_json.rs:210-236](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/explain_json.rs#L210-L236)

```rust
pub fn explain_plan_analyze_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<PlanNode, String> {
    let q = SparqlParser::new().parse_query(sparql).map_err(|e| e.to_string())?;
    let active = crate::active_dataset(graph, &q);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::set_query_base(q.base_iri().map(|b| b.as_str()));
    if !matches!(q, Query::Select { .. } | Query::Ask { .. }) {
        return Err("EXPLAIN ANALYZE supports SELECT and ASK queries only (use explain_plan for CONSTRUCT/DESCRIBE)".into());
    }

    // Execute under the budget with the operator trace installed (exactly as the
    // text `explain_analyze` does), then reconstruct the typed tree from the trace.
    exec::budget::with_budget(budget, || {
        let _tguard = exec::trace::install();
        match &q {
            Query::Select { pattern, .. } => {
                exec::eval_select(graph, pattern)?;
            }
            Query::Ask { pattern, .. } => {
                exec::eval_ask(graph, pattern)?;
            }
            _ => unreachable!(),
        }
        let nodes = exec::trace::take();
        tree_from_trace(&nodes).ok_or_else(|| "empty execution trace".to_string())
    })
}
```

### eval

[crates/sparq-engine/src/cache.rs:360-383](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/cache.rs#L360-L383)

```rust
fn eval(
    graph: &sparq_core::Graph,
    query: &Query,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    let active = crate::active_dataset(graph, query);
    let graph = active.as_ref().unwrap_or(graph);
    let _view_scope = crate::view_scope(&active);
    exec::budget::with_budget(budget, || {
        exec::set_query_base(query.base_iri().map(|b| b.as_str()));
        match query {
            Query::Select { pattern, .. } => exec::eval_select(graph, pattern),
            Query::Ask { pattern, .. } => Ok(QueryResult {
                vars: Vec::new(),
                rows: if exec::eval_ask(graph, pattern)? {
                    vec![Vec::new()]
                } else {
                    Vec::new()
                },
            }),
            _ => Err("result cache only stores SELECT and ASK queries".into()),
        }
    })
}
```

### with_functions

[crates/sparq-engine/src/lib.rs:401-404](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L401-L404)

```rust
pub fn with_functions<T>(fns: &FunctionRegistry, f: impl FnOnce() -> T) -> T {
    let _guard = exec::functions::install(fns);
    f()
}
```

### query_with_functions_and_budget

[crates/sparq-engine/src/lib.rs:704-711](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L704-L711)

```rust
pub fn query_with_functions_and_budget(
    graph: &Graph,
    sparql: &str,
    fns: &FunctionRegistry,
    budget: &QueryBudget,
) -> Result<QueryResult, String> {
    with_functions(fns, || query_with_budget(graph, sparql, budget))
}
```

### query_with_budget

[crates/sparq-engine/src/lib.rs:1022-1024](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L1022-L1024)

```rust
pub fn query_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}
```

### view_scope

[crates/sparq-engine/src/lib.rs:827-829](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/lib.rs#L827-L829)

```rust
pub(crate) fn view_scope(active: &Option<Graph>) -> Option<exec::view::Guard> {
    active.is_some().then(exec::view::suspend_all)
}
```

### with_budget

[crates/sparq-engine/src/exec.rs:251-254](https://github.com/sparq-org/sparq/blob/f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464/crates/sparq-engine/src/exec.rs#L251-L254)

```rust
    pub(crate) fn with_budget<T>(b: &QueryBudget, f: impl FnOnce() -> T) -> T {
        let _guard = install(b);
        f()
    }
```
