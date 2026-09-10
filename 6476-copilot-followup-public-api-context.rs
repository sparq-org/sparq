/// A cooperative resource budget for one query evaluation (T15 server hardening).
///
/// The executor checks it at coarse sites only (operator entry, once per outer
/// iteration of the big scan/join loops), so enforcement is approximate but cheap:
/// an unlimited budget (the default) costs nothing on the hot paths. When a limit
/// trips, evaluation stops and the query fails with
/// `"query budget exceeded (timeout)"` / `"query budget exceeded (max-rows)"` /
/// `"query budget exceeded (max-bytes)"` / `"query budget exceeded (cancelled)"`.
///
/// [GPT-6 Astra] A nested engine call from a callback uses its own budget. The
/// outer budget resumes when that call returns (also after errors or unwind).
/// Its deadline and cancellation are checked at the next outer poll; this does
/// not interrupt arbitrary callback work or combine budgets across queries.
/// Executes a SPARQL query string against a graph, materialising the solutions.
pub fn query(graph: &Graph, sparql: &str) -> Result<QueryResult, String> {
    query_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`query`] under a cooperative [`QueryBudget`] (deadline / max result rows).
pub fn query_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}

/// [`query`] over a [`PreparedQuery`] — no per-execution parse.
pub fn query_prepared(graph: &Graph, prepared: &PreparedQuery) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, prepared, &QueryBudget::unlimited())
}

/// [`query_prepared`] under a cooperative [`QueryBudget`] (deadline / max result rows).
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

