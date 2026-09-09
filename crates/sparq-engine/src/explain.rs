//! T22 query-plan introspection: `EXPLAIN` / `EXPLAIN ANALYZE`.
//!
//! [`explain`] is a **planning-only dry run**: it parses the query to algebra
//! (spargebra), renders the operator tree, and for every conjunctive (BGP)
//! subtree REPLAYS the executor's planning decisions — the greedy GOO join
//! ordering with its cardinality estimates (`exec::goo_seed` / `exec::goo_pick`
//! / `exec::record_pattern_ndv`, the *same* functions the executor calls), the
//! binary-vs-worst-case-optimal dispatch (`exec::bgp_uses_binary`), the sargable
//! filter pushdown split (`exec::split_sargable`) and per-pattern index-range
//! estimates. **No data is read** beyond the index-range bounds the planner
//! itself consults, so EXPLAIN is cheap regardless of query cost.
//!
//! One honest caveat, stated in the output header: the executor picks the
//! *physical* join strategy (bind/merge/hash/cross) per step partly from the
//! ACTUAL intermediate row count at that point; the dry run predicts that choice
//! from the same cardinality estimates the ordering uses. The predicted scan
//! sort order replicates the store's permutation choice over the public
//! [`sparq_core::store::BUILT`] set (differentially tested below against the
//! store's real choice).
//!
//! [`explain_analyze`] EXECUTES the query (SELECT/ASK) with the thread-local
//! operator trace installed (`exec::trace`): every `eval_graph_pattern`
//! operator entry records its output row count and wall time, reported as a
//! tree after the plan. On `wasm32` wall times read 0 (no monotonic clock)
//! unless a host clock is installed (`explain-json`'s `set_trace_clock` —
//! the wasm binding routes `performance.now()` through it); row counts
//! remain exact either way.

use std::fmt::Write as _;

use oxrdf::Variable;
use rustc_hash::FxHashMap;
use sparq_core::store::{Pattern as IdPattern, Perm, BUILT};
use sparq_core::Graph;
use spargebra::algebra::GraphPattern;
use spargebra::term::TriplePattern;
use spargebra::{Query, SparqlParser};

use crate::exec::{self, Prepared, ScanCmp};
use crate::QueryBudget;

/// Renders the query's algebra and the physical plan the engine would choose —
/// join order with cardinality estimates, per-step join strategy, pushed-down
/// filters — WITHOUT executing the query (a planning-only dry run; see the
/// module docs for the one runtime-dependent caveat).
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

/// [`explain`] + actual execution (SELECT and ASK): appends a per-operator
/// execution trace — output rows and wall time per operator — plus totals.
pub fn explain_analyze(graph: &Graph, sparql: &str) -> Result<String, String> {
    explain_analyze_with_budget(graph, sparql, &QueryBudget::unlimited())
}

/// [`explain_analyze`] under a cooperative [`QueryBudget`] (deadline / max rows).
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
    let _bguard = exec::budget::install(budget);
    let _tguard = exec::trace::install();
    let sw = exec::trace::Stopwatch::start();
    let total_rows = match &q {
        Query::Select { pattern, .. } => exec::eval_select(graph, pattern)?.rows.len(),
        Query::Ask { pattern, .. } => usize::from(exec::eval_ask(graph, pattern)?),
        _ => unreachable!(),
    };
    let total_nanos = sw.elapsed_nanos();
    let nodes = exec::trace::take();

    let _ = writeln!(out, "Execution trace (operator → output rows, wall time):");
    for n in &nodes {
        let _ = writeln!(out, "{}{}  rows={}  time={}", indent(n.depth + 1), n.label, n.rows, fmt_nanos(n.nanos));
    }
    let _ = writeln!(out, "Total: {} result row(s) in {}", total_rows, fmt_nanos(total_nanos));
    Ok(out)
}

fn query_form_pattern(q: &Query) -> (&'static str, &GraphPattern) {
    match q {
        Query::Select { pattern, .. } => ("SELECT", pattern),
        Query::Ask { pattern, .. } => ("ASK", pattern),
        Query::Construct { pattern, .. } => ("CONSTRUCT", pattern),
        Query::Describe { pattern, .. } => ("DESCRIBE", pattern),
    }
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

fn fmt_nanos(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}s", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.2}ms", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.1}µs", n as f64 / 1e3)
    } else {
        format!("{n}ns")
    }
}

/// Estimates rendered as integers (they are products of integer range sizes and
/// selectivities — fractional precision is noise).
fn fmt_est(card: f64) -> String {
    format!("{:.0}", card.max(0.0).round())
}

// ---- Algebra tree rendering ----------------------------------------------------

/// Renders an operator subtree. Conjunctive subtrees (BGP / Join-of-BGPs /
/// flattenable FILTERs — exactly the shapes `eval_graph_pattern` routes to the
/// BGP planner) are rendered as one planned BGP node; everything else mirrors
/// the evaluator's operator dispatch.
fn render_pattern(graph: &Graph, p: &GraphPattern, out: &mut String, d: usize) -> Result<(), String> {
    if exec::is_conjunctive(p) {
        return render_conjunctive(graph, p, out, d);
    }
    let mut line = |s: String| {
        let _ = writeln!(out, "{}{}", indent(d), s);
    };
    match p {
        GraphPattern::Bgp { .. } => unreachable!("Bgp is conjunctive"),
        GraphPattern::Filter { expr, inner } => {
            line(format!("Filter {expr}"));
            render_pattern(graph, inner, out, d + 1)
        }
        GraphPattern::Join { left, right } => {
            line("Join (hash/merge on shared variables; compatibility join if optional vars overlap)".into());
            render_pattern(graph, left, out, d + 1)?;
            render_pattern(graph, right, out, d + 1)
        }
        GraphPattern::LeftJoin { left, right, expression } => {
            match expression {
                Some(e) => line(format!("LeftJoin / OPTIONAL (filter: {e})")),
                None => line("LeftJoin / OPTIONAL".into()),
            }
            render_pattern(graph, left, out, d + 1)?;
            render_pattern(graph, right, out, d + 1)
        }
        GraphPattern::Union { left, right } => {
            line("Union".into());
            render_pattern(graph, left, out, d + 1)?;
            render_pattern(graph, right, out, d + 1)
        }
        GraphPattern::Extend { inner, variable, expression } => {
            line(format!("Extend / BIND(?{} := {expression})", variable.as_str()));
            render_pattern(graph, inner, out, d + 1)
        }
        GraphPattern::Minus { left, right } => {
            line("Minus".into());
            render_pattern(graph, left, out, d + 1)?;
            render_pattern(graph, right, out, d + 1)
        }
        GraphPattern::Values { variables, bindings } => {
            line(format!(
                "Values ({} row(s) over {})",
                bindings.len(),
                variables.iter().map(|v| format!("?{}", v.as_str())).collect::<Vec<_>>().join(", ")
            ));
            Ok(())
        }
        GraphPattern::Path { subject, path, object } => {
            line(format!("PropertyPath {subject} {path} {object} (materialised pair relation)"));
            Ok(())
        }
        GraphPattern::Graph { name, inner } => {
            line(format!("Graph {name}"));
            render_pattern(graph, inner, out, d + 1)
        }
        GraphPattern::Project { inner, variables } => {
            line(format!(
                "Project {}",
                variables.iter().map(|v| format!("?{}", v.as_str())).collect::<Vec<_>>().join(", ")
            ));
            render_pattern(graph, inner, out, d + 1)
        }
        GraphPattern::Distinct { inner } => {
            line("Distinct".into());
            render_pattern(graph, inner, out, d + 1)
        }
        GraphPattern::Reduced { inner } => {
            line("Reduced".into());
            render_pattern(graph, inner, out, d + 1)
        }
        GraphPattern::Slice { inner, start, length } => {
            line(format!(
                "Slice (offset {start}{})",
                length.map(|l| format!(", limit {l}")).unwrap_or_default()
            ));
            render_pattern(graph, inner, out, d + 1)
        }
        GraphPattern::OrderBy { inner, expression } => {
            line(format!("OrderBy ({} key(s))", expression.len()));
            render_pattern(graph, inner, out, d + 1)
        }
        GraphPattern::Group { inner, variables, aggregates } => {
            line(format!(
                "Group (by {}; {} aggregate(s))",
                if variables.is_empty() {
                    "<all>".to_string()
                } else {
                    variables.iter().map(|v| format!("?{}", v.as_str())).collect::<Vec<_>>().join(", ")
                },
                aggregates.len()
            ));
            render_pattern(graph, inner, out, d + 1)
        }
        other => {
            line(format!("{other:?}"));
            Ok(())
        }
    }
}

/// Renders a conjunctive subtree as the planned BGP: filter-pushdown split, the
/// binary-vs-WCOJ dispatch, and (for binary plans) the replayed GOO join order
/// with estimates and per-step strategy.
fn render_conjunctive(graph: &Graph, p: &GraphPattern, out: &mut String, d: usize) -> Result<(), String> {
    let mut patterns: Vec<TriplePattern> = Vec::new();
    let mut filters = Vec::new();
    exec::flatten_conjunction(p, &mut patterns, &mut filters);

    if patterns.is_empty() {
        let _ = writeln!(out, "{}BGP (empty — unit solution)", indent(d));
        for f in &filters {
            let _ = writeln!(out, "{}Filter {f}", indent(d));
        }
        return Ok(());
    }

    let prepared = exec::prepare_bgp(graph, &patterns)?;
    if prepared.iter().any(|p| p.unsatisfiable) {
        let _ = writeln!(
            out,
            "{}BGP — unsatisfiable: a constant term does not occur in the graph → 0 rows (no scans run)",
            indent(d)
        );
        for (tp, pr) in patterns.iter().zip(&prepared) {
            let _ = writeln!(
                out,
                "{}pattern {tp} (est {} rows{})",
                indent(d + 1),
                pr.est,
                if pr.unsatisfiable { ", absent constant" } else { "" }
            );
        }
        return Ok(());
    }

    if !exec::bgp_uses_binary(&patterns) {
        // Cyclic BGP → worst-case-optimal Leapfrog Triejoin; filters apply after.
        let _ = writeln!(
            out,
            "{}BGP [worst-case-optimal join: Leapfrog Triejoin] — cyclic BGP ({} patterns)",
            indent(d),
            patterns.len()
        );
        let prepared_wcoj: Vec<(IdPattern, [Option<Variable>; 3])> =
            prepared.iter().map(|p| (p.id_pat, p.pos_vars.clone())).collect();
        let order = exec::wcoj_global_order(graph, &patterns, &prepared_wcoj);
        let _ = writeln!(
            out,
            "{}global variable order: {}",
            indent(d + 1),
            order.iter().map(|v| format!("?{}", v.as_str())).collect::<Vec<_>>().join(", ")
        );
        for (tp, pr) in patterns.iter().zip(&prepared) {
            let _ = writeln!(out, "{}trie {tp} (est {} rows)", indent(d + 1), pr.est);
        }
        for f in &filters {
            let _ = writeln!(out, "{}post-join Filter {f}", indent(d + 1));
        }
        return Ok(());
    }

    // Binary plan: split the sargable filters, then replay the GOO loop.
    let (pat_filters, residual) = exec::split_sargable(graph, &patterns, &filters);
    let pfilter = |i: usize| -> Option<(usize, ScanCmp)> { pat_filters.get(i).copied().flatten() };
    let _ = writeln!(
        out,
        "{}BGP [binary join plan: greedy GOO ordering] ({} patterns)",
        indent(d),
        patterns.len()
    );

    let mut cs_ctx = exec::CsCtx::new(&prepared);
    let seed = exec::goo_seed(&prepared);
    let seed_sort_col = exec::goo_seed_sort(&prepared, seed, pfilter(seed).map(|(c, _)| c));
    let mut sorted_by = predicted_sort_var(&prepared[seed], seed_sort_col);
    let mut vars: Vec<Variable> = Vec::new();
    add_vars(&mut vars, &prepared[seed]);
    let mut cur_card = prepared[seed].est as f64;
    let mut var_ndv: FxHashMap<Variable, f64> = FxHashMap::default();
    let mut done = vec![false; prepared.len()];
    done[seed] = true;
    cs_ctx.note_done(seed);
    exec::record_pattern_ndv(graph, &prepared, seed, cur_card, &mut var_ndv, &cs_ctx);

    let _ = writeln!(
        out,
        "{}1. scan {}{} (est {} rows{}) [seed: smallest estimate]",
        indent(d + 1),
        patterns[seed],
        filter_note(&prepared[seed], pfilter(seed)),
        prepared[seed].est,
        sorted_note(&sorted_by),
    );

    for step in 2..=prepared.len() {
        let est_rows_before = cur_card.max(0.0).round() as usize;
        let (i, new_card, connected) = exec::goo_pick(graph, &prepared, &done, &var_ndv, cur_card, &cs_ctx);
        cur_card = new_card;
        done[i] = true;
        cs_ctx.note_done(i);
        // Fold the joined pattern's ndv estimates exactly as the executor does, so
        // the replayed pick sequence cannot drift from the executed one.
        exec::record_pattern_ndv(graph, &prepared, i, cur_card, &mut var_ndv, &cs_ctx);
        let pr = &prepared[i];
        let filt = pfilter(i);

        // Predict the executor's per-step physical strategy (see module docs:
        // the bind-join cutover is decided from the ACTUAL row count at run
        // time; here the same inequality is evaluated on the estimate).
        let connecting: Vec<&Variable> = vars.iter().filter(|v| pr.var_pos(v).is_some()).collect();
        let strategy;
        let mut new_sorted: Option<Variable> = None;
        if connecting.len() == 1
            && exec::distinct_pattern_vars(&pr.pos_vars)
            && est_rows_before.saturating_mul(8) < pr.est
        {
            strategy = format!(
                "bind join (index nested-loop, predicted) on ?{} into {} (est {} rows)",
                connecting[0].as_str(),
                patterns[i],
                pr.est
            );
        } else {
            let merge_var = sorted_by.clone().filter(|sv| pr.var_pos(sv).is_some());
            let scan_sort = filt.map(|(c, _)| c).or_else(|| merge_var.as_ref().and_then(|jv| pr.var_pos(jv)));
            let rhs_sorted = predicted_sort_var(pr, scan_sort);
            if let Some(jv) = merge_var.filter(|jv| rhs_sorted.as_ref() == Some(jv)) {
                strategy = format!(
                    "merge join on ?{} with scan {}{} (est {} rows, sorted by ?{})",
                    jv.as_str(),
                    patterns[i],
                    filter_note(pr, filt),
                    pr.est,
                    jv.as_str()
                );
                new_sorted = Some(jv);
            } else if connected {
                strategy = format!(
                    "hash join on {} with scan {}{} (est {} rows)",
                    connecting.iter().map(|v| format!("?{}", v.as_str())).collect::<Vec<_>>().join(", "),
                    patterns[i],
                    filter_note(pr, filt),
                    pr.est
                );
            } else {
                strategy = format!(
                    "cross product with scan {}{} (est {} rows) [disconnected]",
                    patterns[i],
                    filter_note(pr, filt),
                    pr.est
                );
            }
        }
        sorted_by = new_sorted;
        add_vars(&mut vars, pr);
        let _ = writeln!(out, "{}{step}. {strategy} → est {} rows", indent(d + 1), fmt_est(cur_card));
    }

    for f in &residual {
        let _ = writeln!(out, "{}post-join Filter {f}", indent(d + 1));
    }
    Ok(())
}

fn add_vars(vars: &mut Vec<Variable>, p: &Prepared) {
    for v in p.pos_vars.iter().flatten() {
        if !vars.contains(v) {
            vars.push(v.clone());
        }
    }
}

fn filter_note(p: &Prepared, filt: Option<(usize, ScanCmp)>) -> String {
    match filt {
        Some((pos, cmp)) => {
            let v = p.pos_vars[pos].as_ref().map(|v| v.as_str()).unwrap_or("?");
            format!(", filter ?{v} {} pushed into scan", cmp.render())
        }
        None => String::new(),
    }
}

fn sorted_note(sorted: &Option<Variable>) -> String {
    match sorted {
        Some(v) => format!(", sorted by ?{}", v.as_str()),
        None => String::new(),
    }
}

// ---- Predicted scan order ------------------------------------------------------

/// The variable a scan's output would be sorted by: replicates the store's
/// permutation choice (`PermSet::choose` / `choose_sorted`) over the public
/// [`BUILT`] permutation set — bound positions must form the permutation's
/// leading prefix; with a requested sort column, prefer the permutation whose
/// first unbound column is that column. Differentially tested below against the
/// store's actual choice.
fn predicted_sort_var(p: &Prepared, sort_col: Option<usize>) -> Option<Variable> {
    let perm = predicted_perm(&p.id_pat, sort_col);
    perm.order().into_iter().find(|&c| p.id_pat[c].is_none()).and_then(|c| p.pos_vars[c].clone())
}

fn predicted_perm(id_pat: &IdPattern, sort_col: Option<usize>) -> Perm {
    let bound = |i: usize| id_pat[i].is_some();
    let total_bound = (0..3).filter(|&i| bound(i)).count();
    let lead_of = |perm: Perm| {
        let order = perm.order();
        let mut lead = 0;
        while lead < 3 && bound(order[lead]) {
            lead += 1;
        }
        lead
    };
    if let Some(sc) = sort_col {
        for &perm in BUILT {
            let lead = lead_of(perm);
            if lead == total_bound && lead < 3 && perm.order()[lead] == sc {
                return perm;
            }
        }
    }
    for &perm in BUILT {
        if lead_of(perm) == total_bound {
            return perm;
        }
    }
    Perm::Spo
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
        ex:bob   ex:knows ex:carol ; ex:age 25 ; ex:name "Bob" .
        ex:carol ex:age 35 ; ex:name "Carol" .
    "#;

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    #[test]
    fn explain_names_join_order_and_strategy() {
        // knows (2 rows) is the seed; age joins on ?b via a merge join (both
        // sides sorted on ?b: POS for knows-object, PSO for age-subject).
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?a ?b ?age WHERE { ?a ex:knows ?b . ?b ex:age ?age }",
        )
        .unwrap();
        assert!(plan.contains("planning-only"), "{plan}");
        assert!(plan.contains("BGP [binary join plan: greedy GOO ordering] (2 patterns)"), "{plan}");
        // Join order: the smaller knows pattern seeds the plan...
        assert!(plan.contains("1. scan ?a <http://ex/knows> ?b (est 2 rows, sorted by ?b) [seed: smallest estimate]"), "{plan}");
        // ...then a merge join on the shared variable with its estimate.
        assert!(plan.contains("2. merge join on ?b with scan ?b <http://ex/age> ?age (est 3 rows, sorted by ?b)"), "{plan}");
    }

    #[test]
    fn explain_shows_filter_pushdown_and_modifiers() {
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . ?s ex:knows ?k . FILTER(?a > 28) } ORDER BY ?s LIMIT 5",
        )
        .unwrap();
        assert!(plan.contains("filter ?a > 28 pushed into scan"), "{plan}");
        assert!(plan.contains("Slice (offset 0, limit 5)"), "{plan}");
        assert!(plan.contains("OrderBy (1 key(s))"), "{plan}");
        assert!(plan.contains("Project ?s"), "{plan}");
    }

    #[test]
    fn explain_cyclic_bgp_uses_wcoj() {
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:knows ?b . ?b ex:knows ?c . ?c ex:knows ?a }",
        )
        .unwrap();
        assert!(plan.contains("worst-case-optimal join: Leapfrog Triejoin"), "{plan}");
        assert!(plan.contains("global variable order:"), "{plan}");
    }

    #[test]
    fn explain_predicts_bind_join_on_selective_seed() {
        // 1 'special' triple vs 20 'age' triples: 1·8 < 20 → the executor's
        // index-nested-loop cutover is predicted.
        let mut ttl = String::from("@prefix ex: <http://ex/> .\nex:root ex:special ex:n1 .\n");
        for i in 1..=20 {
            ttl.push_str(&format!("ex:n{i} ex:age {i} .\n"));
        }
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        let plan = explain(&g, "PREFIX ex: <http://ex/> SELECT * WHERE { ?r ex:special ?k . ?k ex:age ?v }").unwrap();
        assert!(plan.contains("bind join (index nested-loop, predicted) on ?k"), "{plan}");
    }

    #[test]
    fn explain_unsatisfiable_and_optional_and_union() {
        let plan = explain(&g(), "SELECT ?s WHERE { ?s <http://ex/nope> ?o }").unwrap();
        assert!(plan.contains("unsatisfiable"), "{plan}");

        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:age ?a OPTIONAL { ?s ex:knows ?k } }",
        )
        .unwrap();
        assert!(plan.contains("LeftJoin / OPTIONAL"), "{plan}");

        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?x WHERE { { ?x ex:age 30 } UNION { ?x ex:age 25 } }",
        )
        .unwrap();
        assert!(plan.contains("Union"), "{plan}");
    }

    #[test]
    fn explain_construct_and_malformed() {
        let plan = explain(&g(), "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:q ?o } WHERE { ?s ex:knows ?o }").unwrap();
        assert!(plan.contains("EXPLAIN (CONSTRUCT)"), "{plan}");
        assert!(explain(&g(), "SELECT WHERE {").is_err());
    }

    #[test]
    fn predicted_perm_matches_store_choice() {
        // The dry run's replicated permutation choice must agree with the
        // store's actual `scan_sorted` choice for every bound-mask × sort-col
        // combination (the differential guard against drift).
        let g = g();
        let some_id = g.id_of(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://ex/knows").unwrap())).unwrap();
        let masks: Vec<IdPattern> = vec![
            [None, None, None],
            [Some(some_id), None, None],
            [None, Some(some_id), None],
            [None, None, Some(some_id)],
            [Some(some_id), Some(some_id), None],
            [None, Some(some_id), Some(some_id)],
            [Some(some_id), None, Some(some_id)],
        ];
        for pat in &masks {
            for sort_col in [None, Some(0), Some(1), Some(2)] {
                let predicted = predicted_perm(pat, sort_col);
                let actual = match sort_col {
                    Some(c) => g.store.scan_sorted(pat, c).perm,
                    None => g.store.scan(pat).perm,
                };
                assert_eq!(predicted as usize, actual as usize, "pattern {pat:?} sort {sort_col:?}");
            }
        }
    }

    #[test]
    fn explain_analyze_reports_rows_and_times() {
        let r = explain_analyze(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?a ?b ?age WHERE { ?a ex:knows ?b . ?b ex:age ?age }",
        )
        .unwrap();
        assert!(r.contains("Execution trace"), "{r}");
        assert!(r.contains("BGP [binary GOO] (2 patterns, 0 filters)  rows=2"), "{r}");
        assert!(r.contains("Total: 2 result row(s)"), "{r}");
        // ASK runs too.
        let r = explain_analyze(&g(), "PREFIX ex: <http://ex/> ASK { ?s ex:age ?a }").unwrap();
        assert!(r.contains("Total: 1 result row(s)"), "{r}");
        // CONSTRUCT is explain-only.
        assert!(explain_analyze(&g(), "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }").is_err());
    }

    #[test]
    fn explain_analyze_traces_optional_operators() {
        let r = explain_analyze(
            &g(),
            "PREFIX ex: <http://ex/> SELECT * WHERE { ?s ex:age ?a OPTIONAL { ?s ex:knows ?k } }",
        )
        .unwrap();
        assert!(r.contains("LeftJoin (OPTIONAL)"), "{r}");
        assert!(r.contains("rows=3"), "{r}");
    }

    #[test]
    fn explain_does_not_execute() {
        // A cross-product query whose result would be huge: EXPLAIN must return
        // instantly with a plan (cross product named), proving nothing ran.
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for i in 0..2000 {
            ttl.push_str(&format!("ex:s{i} ex:p ex:o{i} .\nex:s{i} ex:q ex:r{i} .\n"));
        }
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        let plan = explain(&g, "PREFIX ex: <http://ex/> SELECT * WHERE { ?a ex:p ?b . ?c ex:q ?d }").unwrap();
        assert!(plan.contains("cross product"), "{plan}");
    }

    // [SONNET-4.6] sq-qcnn.35 — direct unit tests for previously uncovered paths.

    /// `fmt_nanos` covers all four scale branches (ns / µs / ms / s).
    /// This function is private so it can only be tested from inside this module.
    #[test]
    fn fmt_nanos_all_scale_branches() {
        // nanoseconds branch: n < 1_000
        assert_eq!(fmt_nanos(0), "0ns");
        assert_eq!(fmt_nanos(999), "999ns");
        // microseconds branch: 1_000 <= n < 1_000_000
        assert_eq!(fmt_nanos(1_000), "1.0\u{00b5}s");
        assert_eq!(fmt_nanos(1_500), "1.5\u{00b5}s");
        // milliseconds branch: 1_000_000 <= n < 1_000_000_000
        assert_eq!(fmt_nanos(1_000_000), "1.00ms");
        assert_eq!(fmt_nanos(2_500_000), "2.50ms");
        // seconds branch: n >= 1_000_000_000
        assert_eq!(fmt_nanos(1_000_000_000), "1.00s");
        assert_eq!(fmt_nanos(2_500_000_000), "2.50s");
    }

    /// `render_pattern` branches not hit by the existing explain tests:
    /// Filter (non-conjunctive EXISTS), Join (non-conjunctive with Values),
    /// LeftJoin with Some(expression), Extend (non-conjunctive inner), Minus,
    /// Values (standalone), Path (property path), Graph, Distinct, Reduced,
    /// Group, and the `other` fallback (SERVICE).
    #[test]
    fn explain_render_pattern_uncovered_arms() {
        // Filter (non-conjunctive): FILTER EXISTS is not filter_scope_ok, so
        // is_conjunctive(Filter(Bgp, EXISTS)) = false → Filter arm fires.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a FILTER EXISTS { ?s ex:knows ?o } }",
        )
        .unwrap();
        assert!(plan.contains("Filter"), "{plan}");

        // Join (non-conjunctive): Join(Bgp, Values) is not conjunctive because
        // is_conjunctive(Values) = false → Join arm fires.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a . VALUES ?a { 30 35 } }",
        )
        .unwrap();
        assert!(plan.contains("Join"), "{plan}");

        // LeftJoin with Some(expression): OPTIONAL { ... FILTER(...) } creates
        // LeftJoin with an expression argument → Some(e) arm.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT * WHERE { \
             ?s ex:age ?a OPTIONAL { ?s ex:knows ?k FILTER(?k != ex:carol) } }",
        )
        .unwrap();
        assert!(plan.contains("LeftJoin / OPTIONAL (filter:"), "{plan}");

        // Extend (non-conjunctive): BIND after OPTIONAL creates Extend(LeftJoin, ...) which
        // is_conjunctive(Extend) = is_conjunctive(inner) = is_conjunctive(LeftJoin) = false.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?copy WHERE { \
             ?s ex:age ?a OPTIONAL { ?s ex:knows ?k } BIND(?a AS ?copy) }",
        )
        .unwrap();
        assert!(plan.contains("Extend"), "{plan}");

        // Minus: MINUS { ... } is non-conjunctive → Minus arm.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a MINUS { ?s ex:knows ?k } }",
        )
        .unwrap();
        assert!(plan.contains("Minus"), "{plan}");

        // Values (standalone): SELECT ?x WHERE { VALUES ?x { ... } } →
        // Project(Values{...}) → is_conjunctive(Values) = false → Values arm.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?x WHERE { VALUES ?x { ex:alice ex:bob } }",
        )
        .unwrap();
        assert!(plan.contains("Values (2 row(s) over ?x)"), "{plan}");

        // PropertyPath: ?s ex:knows+ ?o → Path{..} which is non-conjunctive → Path arm.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:knows+ ?o }",
        )
        .unwrap();
        assert!(plan.contains("PropertyPath"), "{plan}");

        // Distinct: SELECT DISTINCT generates Distinct wrapper → Distinct arm.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT DISTINCT ?a WHERE { ?s ex:age ?a }",
        )
        .unwrap();
        assert!(plan.contains("Distinct"), "{plan}");

        // Reduced: SELECT REDUCED generates Reduced wrapper → Reduced arm.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT REDUCED ?a WHERE { ?s ex:age ?a }",
        )
        .unwrap();
        assert!(plan.contains("Reduced"), "{plan}");

        // Group: GROUP BY generates Group operator → Group arm.
        let plan = explain(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s (COUNT(*) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?s",
        )
        .unwrap();
        assert!(plan.contains("Group"), "{plan}");

        // `other` fallback: SERVICE clause is not explicitly matched → other arm prints debug.
        let plan = explain(
            &g(),
            "SELECT * WHERE { SERVICE <http://ex/endpoint> { ?s ?p ?o } }",
        )
        .unwrap();
        assert!(plan.contains("Service"), "{plan}");
    }

    /// `render_conjunctive` :: empty-patterns branch — `SELECT * WHERE {}` produces a
    /// `Bgp { patterns: [] }` which is conjunctive; `flatten_conjunction` yields an
    /// empty patterns vec, hitting the `if patterns.is_empty()` arm.
    #[test]
    fn explain_empty_bgp_unit_solution() {
        let plan = explain(&g(), "SELECT * WHERE {}").unwrap();
        assert!(plan.contains("BGP (empty \u{2014} unit solution)"), "{plan}");
    }

    /// `query_form_pattern` :: DESCRIBE arm (the Describe variant).
    #[test]
    fn explain_describe_form() {
        let plan = explain(&g(), "PREFIX ex: <http://ex/> DESCRIBE ex:alice").unwrap();
        assert!(plan.contains("EXPLAIN (DESCRIBE)"), "{plan}");
    }

    /// `Graph` arm in `render_pattern`: a GRAPH clause over a dataset with named graphs.
    #[test]
    fn explain_render_graph_clause() {
        let nq = "<http://ex/a> <http://ex/p> <http://ex/x> <http://ex/g1> .\n";
        let ds = Graph::load_dataset(nq, "nquads").unwrap();
        let plan = explain(&ds, "SELECT ?s WHERE { GRAPH ?g { ?s ?p ?o } }").unwrap();
        assert!(plan.contains("Graph"), "{plan}");
    }

    /// `explain_analyze_with_budget` direct call with a non-unlimited budget.
    /// `explain_analyze` calls it with `unlimited()`, so calling directly with
    /// a `max_rows` budget exercises the same path while confirming the budget
    /// argument flows through correctly.
    #[test]
    fn explain_analyze_with_budget_direct_call() {
        let budget = QueryBudget { max_rows: Some(100), ..QueryBudget::unlimited() };
        let r = explain_analyze_with_budget(
            &g(),
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a }",
            &budget,
        )
        .unwrap();
        assert!(r.contains("Execution trace"), "{r}");
        assert!(r.contains("Total: 3 result row(s)"), "{r}");
        // Non-SELECT/ASK is rejected even with a budget.
        assert!(explain_analyze_with_budget(
            &g(),
            "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
            &QueryBudget::unlimited(),
        )
        .is_err());
    }
}
