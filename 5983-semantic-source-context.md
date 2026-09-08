### crates/sparq-engine/src/exec.rs:2332-2366 — eval_select
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn eval_select(graph: &Graph, pattern: &GraphPattern) -> Result<QueryResult, String> {
    let mut local = LocalVocab::default();
    let bindings = eval_modified(graph, &mut local, pattern)?;
    // Final budget gate: converts a row-capped/timed-out evaluation (including the
    // uninstrumented rayon branches) into the error before the expensive term
    // materialisation below.
    budget::check(bindings.rows.len())?;

    // SELECT * exposes only real variables, never synthetic blank-node variables.
    let out_vars: Vec<Variable> = bindings
        .vars
        .iter()
        .filter(|v| !v.as_str().starts_with(BNODE_VAR_PREFIX))
        .cloned()
        .collect();

    let col_of: Vec<Option<usize>> = out_vars.iter().map(|v| bindings.col(v)).collect();
    // Materialise each solution row's terms. This reconstructs an `oxrdf::Term` (an
    // IRI/string allocation) per cell and is the dominant cost of returning a large
    // result — but every row is independent, so do it in parallel on native (the wasm
    // build has no threads and keeps the sequential path). Order is preserved.
    let materialise = |row: &Row| -> Vec<Option<Term>> {
        col_of.iter().map(|c| c.and_then(|i| term_of(graph, &local, row[i]))).collect()
    };
    #[cfg(feature = "parallel")]
    let rows: Vec<Vec<Option<Term>>> = if bindings.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        bindings.rows.par_iter().map(materialise).collect()
    } else {
        bindings.rows.iter().map(materialise).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let rows: Vec<Vec<Option<Term>>> = bindings.rows.iter().map(materialise).collect();
    Ok(QueryResult { vars: out_vars, rows })
}
```

### crates/sparq-engine/src/exec.rs:3012-3108 — eval_modified
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn eval_modified(graph: &Graph, local: &mut LocalVocab, p: &GraphPattern) -> Result<Bindings, String> {
    // Pin NOW() for this execution (SPARQL 1.1 §17.4.5.1). Outermost call samples
    // the clock once; the recursive / EXISTS re-entries see it active and keep the
    // outer instant (a Cell read). [FABLE-5] sq-98w7z.1
    #[cfg(not(target_arch = "wasm32"))]
    let _query_now = query_now::scope();
    match p {
        GraphPattern::Project { inner, variables } => {
            let b = eval_modified(graph, local, inner)?;
            Ok(project_bindings(b, variables))
        }
        GraphPattern::Distinct { inner } => {
            // zk-trace: the enclosed pattern inputs are the PRE-DISTINCT
            // input sets (the reduction is verifier-side; zk module docs).
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Distinct);
            // DISTINCT-projection loose skip-scan: `Distinct{Project{[?p]}{BGP/Union}}`
            // can enumerate the DISTINCT projected values directly from a permutation
            // sorted by ?p (no full-join materialisation). Result-SET equivalent to the
            // path below; conservatively declines (→ None) on any other shape. The zk
            // recorder needs the whole PRE-distinct input set, so the pushdown is
            // suppressed while armed (checked inside `try_distinct_pushdown`).
            // [OPUS-4.8] sq-7d3dj.30.4
            if distinct_pushdown::enabled() {
                if let Some(b) = try_distinct_pushdown(graph, local, inner)? {
                    return Ok(b);
                }
            }
            let mut b = eval_modified(graph, local, inner)?;
            distinct_bindings(&mut b);
            Ok(b)
        }
        GraphPattern::Reduced { inner } => eval_modified(graph, local, inner),
        GraphPattern::Slice { inner, start, length } => {
            // LIMIT early-termination: a bare LIMIT (no ORDER BY / DISTINCT /
            // aggregation above it, which all need the full result) can stop after
            // start+length rows instead of materialising the whole relation — a
            // single-pattern scan stops mid-scan, and a conjunctive / UNION /
            // OPTIONAL / Join shape stops at the first seed blocks yielding enough
            // fully-FILTERed rows (`try_capped`, sq-7d3dj.30.8; ASK evaluates as
            // LIMIT 1 through here). LIMIT-without-ORDER-BY is order-insensitive so
            // any rows are valid.
            if let Some(len) = length {
                if let Some(cap) = start.checked_add(*len) {
                    if let Some(mut b) = try_capped(graph, local, inner, cap)? {
                        slice_bindings(&mut b, *start, *length);
                        return Ok(b);
                    }
                    // Top-k ORDER BY: when the inner pattern (possibly under Project/Reduced)
                    // contains an OrderBy and the budget k = offset + limit is within the
                    // threshold, use bounded selection instead of a full stable sort.
                    // Result is byte-identical to the full-sort+slice path. [SONNET-4.6] sq-7d3dj.30.2
                    if cap <= TOP_K_ORDER_BY_THRESHOLD {
                        if let Some(mut b) = try_topk_orderby(graph, local, inner, cap)? {
                            slice_bindings(&mut b, *start, *length);
                            return Ok(b);
                        }
                    }
                }
            }
            let mut b = eval_modified(graph, local, inner)?;
            slice_bindings(&mut b, *start, *length);
            Ok(b)
        }
        GraphPattern::OrderBy { inner, expression } => {
            let mut b = eval_modified(graph, local, inner)?;
            order_bindings(graph, local, &mut b, expression, None)?;
            Ok(b)
        }
        GraphPattern::Group { inner, variables, aggregates } => {
            // zk-trace: the enclosed pattern inputs are the PRE-AGGREGATION
            // input sets; the count pushdown below is disabled under an armed
            // recorder (inside `try_count`), so they are always captured.
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Group);
            // COUNT(*) pushdown: a whole-dataset COUNT over a single pattern is the
            // scan range size — no need to materialise the solutions just to count
            // them (QLever counts lazily too; this is the q02-style win).
            if variables.is_empty() && aggregates.len() == 1 {
                if let (av, AggregateExpression::CountSolutions { distinct: false }) = (&aggregates[0].0, &aggregates[0].1) {
                    // COUNT(*) == the inner pattern's solution count. Use `try_count`, not
                    // just `count_pushdown`: it also covers OPTIONAL (lazy left-join count,
                    // Σ over the join var) and LIMIT/OFFSET — so `COUNT(*)` over an OPTIONAL
                    // no longer materialises the whole left-join just to count it.
                    if let Some(n) = try_count(graph, inner) {
                        let id = value_to_id(graph, local, &Value::Num(Num::Int(n as i64)));
                        let row: Row = std::iter::once(id).collect();
                        return Ok(Bindings { vars: vec![av.clone()], rows: vec![row], sorted_by: None });
                    }
                }
            }
            let b = eval_graph_pattern(graph, local, inner)?;
            group_aggregate(graph, local, b, variables, aggregates)
        }
        other => eval_graph_pattern(graph, local, other),
    }
}
```

### crates/sparq-engine/src/exec.rs:3119-3156 — try_topk_orderby
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn try_topk_orderby(
    graph: &Graph,
    local: &mut LocalVocab,
    inner: &GraphPattern,
    row_budget: usize,
) -> Result<Option<Bindings>, String> {
    match inner {
        GraphPattern::Project { inner: proj_inner, variables } => {
            Ok(try_topk_orderby(graph, local, proj_inner, row_budget)?
                .map(|b| project_bindings(b, variables)))
        }
        GraphPattern::Reduced { inner: red_inner } => {
            try_topk_orderby(graph, local, red_inner, row_budget)
        }
        GraphPattern::OrderBy { inner: ord_inner, expression } => {
            // [SONNET-5] (sq-artifact-keeper-topk) Try the index-ordered seed path
            // first: for the narrow "star BGP, ORDER BY a variable bound by exactly
            // one pattern's object" shape, it walks a value-sorted permutation scan
            // directly instead of materialising every matching row — see
            // `try_topk_orderby_indexed`'s doc comment for the soundness argument.
            // Declines (`Ok(None)`) for any shape/datatype it can't prove safe, in
            // which case the existing materialize-then-select path below runs
            // unchanged.
            if let Some(b) = try_topk_orderby_indexed(graph, local, ord_inner, expression, row_budget)? {
                return Ok(Some(b));
            }
            let mut b = eval_modified(graph, local, ord_inner)?;
            // Use the top-k path only when the budget is strictly less than n;
            // otherwise the heap adds overhead with no benefit.
            let use_topk = b.rows.len() > row_budget;
            order_bindings(graph, local, &mut b, expression, if use_topk { Some(row_budget) } else { None })?;
            Ok(Some(b))
        }
        // Not an OrderBy under transparent wrappers: decline so the caller
        // falls through to the regular eval_modified path.
        _ => Ok(None),
    }
}
```

### crates/sparq-engine/src/exec.rs:3845-3864 — has_intra_triple_repeated_var
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn has_intra_triple_repeated_var(tp: &TriplePattern) -> bool {
    let sv = if let TermPattern::Variable(v) = &tp.subject {
        Some(v)
    } else {
        None
    };
    let pv = if let NamedNodePattern::Variable(v) = &tp.predicate {
        Some(v)
    } else {
        None
    };
    let ov = if let TermPattern::Variable(v) = &tp.object {
        Some(v)
    } else {
        None
    };
    (sv.is_some() && pv.is_some() && sv == pv)
        || (sv.is_some() && ov.is_some() && sv == ov)
        || (pv.is_some() && ov.is_some() && pv == ov)
}
```

### crates/sparq-engine/src/exec.rs:6286-6305 — is_conjunctive
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn is_conjunctive(p: &GraphPattern) -> bool {
    match p {
        GraphPattern::Bgp { .. } => true,
        // A FILTER may only be flattened into the enclosing conjunction when every
        // variable it mentions is bound INSIDE its own group — otherwise hoisting it
        // changes scope (`{ :x :p ?v . { FILTER(?v = 1) } }` must see ?v UNBOUND).
        // EXISTS is conservatively never flattened (it evaluates against the group's
        // in-scope bindings).
        GraphPattern::Filter { inner, expr } => {
            if !is_conjunctive(inner) {
                return false;
            }
            let mut inner_vars: FxHashSet<Variable> = FxHashSet::default();
            collect_pattern_vars(inner, &mut inner_vars);
            filter_scope_ok(expr, &inner_vars)
        }
        GraphPattern::Join { left, right } => is_conjunctive(left) && is_conjunctive(right),
        _ => false,
    }
}
```

### crates/sparq-engine/src/exec.rs:6346-6359 — flatten_conjunction
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn flatten_conjunction(p: &GraphPattern, patterns: &mut Vec<TriplePattern>, filters: &mut Vec<Expression>) {
    match p {
        GraphPattern::Bgp { patterns: tps } => patterns.extend(tps.iter().cloned()),
        GraphPattern::Join { left, right } => {
            flatten_conjunction(left, patterns, filters);
            flatten_conjunction(right, patterns, filters);
        }
        GraphPattern::Filter { expr, inner } => {
            flatten_conjunction(inner, patterns, filters);
            filters.push(expr.clone());
        }
        _ => unreachable!(),
    }
}
```

### crates/sparq-engine/src/exec.rs:8595-8605 — collect_vars
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn collect_vars(patterns: &[TriplePattern]) -> Vec<Variable> {
    let mut vars = Vec::new();
    for tp in patterns {
        for v in [tp_var(&tp.subject), nnp_var(&tp.predicate), tp_var(&tp.object)].into_iter().flatten() {
            if !vars.contains(&v) {
                vars.push(v);
            }
        }
    }
    vars
}
```

### crates/sparq-engine/src/exec.rs:8844-8873 — prepare_pattern
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn prepare_pattern(graph: &Graph, tp: &TriplePattern) -> Result<(IdPattern, [Option<Variable>; 3], bool), String> {
    let mut id_pat: IdPattern = [None, None, None];
    let mut pos_vars: [Option<Variable>; 3] = [None, None, None];
    let mut unsat = false;

    // Resolves one (subject/object) position: variable -> pos_vars; blank node ->
    // synthetic variable; concrete term -> dictionary id (absent term => unsat).
    let mut bind_term = |slot: usize, tp: &TermPattern| -> Result<(), String> {
        match tp {
            TermPattern::Variable(v) => pos_vars[slot] = Some(v.clone()),
            TermPattern::BlankNode(b) => pos_vars[slot] = Some(bnode_var(b)),
            other => match graph.id_of(&term_pattern_to_term(other)?) {
                Some(id) => id_pat[slot] = Some(id),
                None => unsat = true,
            },
        }
        Ok(())
    };
    bind_term(0, &tp.subject)?;
    bind_term(2, &tp.object)?;

    match &tp.predicate {
        NamedNodePattern::Variable(v) => pos_vars[1] = Some(v.clone()),
        NamedNodePattern::NamedNode(n) => match graph.id_of(&Term::NamedNode(n.clone())) {
            Some(id) => id_pat[1] = Some(id),
            None => unsat = true,
        },
    }
    Ok((id_pat, pos_vars, unsat))
}
```

### crates/sparq-engine/src/exec.rs:8875-9008 — scan_to_bindings
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn scan_to_bindings(
    graph: &Graph,
    id_pat: &IdPattern,
    pos_vars: &[Option<Variable>; 3],
    sort_col: Option<usize>,
    filter: Option<(usize, ScanCmp)>,
    limit: Option<usize>,
    // [OPUS-4.8] (sq-gr8mb / §A3) Optional semi-join prefilter: `(canonical position of
    // the connecting variable, membership filter over the other side's join keys)`. A
    // scanned row whose key at that position is ABSENT from the filter cannot match the
    // downstream join, so it is dropped before projection. The filter is membership-exact
    // (no false positives), so this never changes the RESULT — only fewer rows are kept.
    // Only present under the opt-in `semijoin-bitmap` feature, so the default build's
    // signature and per-row path are byte-identical.
    #[cfg(feature = "semijoin-bitmap")] prefilter: Option<(usize, &crate::semijoin::KeyFilter)>,
) -> Bindings {
    let mut vars: Vec<Variable> = Vec::new();
    let mut var_positions: Vec<Vec<usize>> = Vec::new();
    for (pos, v) in pos_vars.iter().enumerate() {
        if let Some(v) = v {
            if let Some(idx) = vars.iter().position(|x| x == v) {
                var_positions[idx].push(pos);
            } else {
                vars.push(v.clone());
                var_positions.push(vec![pos]);
            }
        }
    }
    let scan = match sort_col {
        Some(c) => graph.store.scan_sorted(id_pat, c),
        None => graph.store.scan(id_pat),
    };
    // The TRUE sort column is the first unbound canonical column in the chosen
    // permutation's order — NOT necessarily the requested `sort_col`: with fewer than
    // six permutations the store may not have the requested order, in which case the
    // engine must report the real one so merge joins fall back to hash and range-
    // pruning is skipped (both keyed off the truthful `sorted_by` / `actual_sort`).
    let actual_sort = scan.perm.order().into_iter().find(|&c| id_pat[c].is_none());
    let sorted_by = actual_sort.and_then(|c| pos_vars[c].clone());

    // Range-pruning: when the pushed-down filter is on the scan's ACTUAL sort column and
    // that column holds inline integers (which sort by value), binary-search to the
    // passing value range instead of scanning + filtering the whole relation. Safe
    // only when EVERY value in the column is inline (so no dictionary-encoded
    // numeric in another datatype, scattered below INLINE_BASE, is skipped).
    let mut scan_rows: &[[Id; 3]] = scan.rows.as_ref();
    if let Some((fpos, cmp)) = filter {
        if actual_sort == Some(fpos) && scan_rows.first().is_some_and(|r| dict::is_inline(scan.to_spo(r)[fpos])) {
            scan_rows = match inline_pass_values(cmp) {
                Some((lo, hi)) => {
                    let (lo_id, hi_id) = (dict::INLINE_BASE + lo, dict::INLINE_BASE + hi);
                    let start = scan_rows.partition_point(|r| scan.to_spo(r)[fpos] < lo_id);
                    let end = scan_rows.partition_point(|r| scan.to_spo(r)[fpos] <= hi_id);
                    &scan_rows[start..end]
                }
                None => &[],
            };
        }
    }

    // Per-row builder: apply the semi-join prefilter (if any) and the pushed-down
    // filter, then project (with the repeated-variable consistency check); `None`
    // drops the row.
    let build_row = |row: &[Id; 3]| -> Option<Row> {
        let spo = scan.to_spo(row);
        // [OPUS-4.8] (sq-gr8mb / §A3) Semi-join prefilter: drop a row whose connecting-
        // variable id is absent from the other side's join-key set — it cannot survive the
        // downstream join. EXACT membership, so the result is unchanged (only this wasted
        // row is skipped before projection). Checked first: it is the cheapest reject.
        #[cfg(feature = "semijoin-bitmap")]
        if let Some((jpos, kf)) = prefilter {
            if !kf.contains(spo[jpos]) {
                return None;
            }
        }
        if let Some((fpos, cmp)) = filter {
            if !cmp.test_id(graph, spo[fpos]) {
                return None;
            }
        }
        let mut out = Row::with_capacity(vars.len());
        for positions in &var_positions {
            let v0 = spo[positions[0]];
            if positions.iter().any(|&p| spo[p] != v0) {
                return None;
            }
            out.push(v0);
        }
        Some(out)
    };

    // zk-trace hook (feature `zk`, armed recorder only): record the matched
    // triples of this pattern scan — the rows `build_row` keeps, BEFORE
    // projection (the witness needs whole triples, not just variable columns).
    // One `enabled()` check per scan; zero per-row cost when disarmed.
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        let kept: Vec<[Id; 3]> = scan_rows
            .iter()
            .filter(|r| build_row(r).is_some())
            .map(|r| scan.to_spo(r))
            .collect();
        crate::zk::record_scan_ids(graph, id_pat, pos_vars, &kept, false);
    }

    // No LIMIT and a large relation: build the rows in parallel (order-preserving).
    #[cfg(feature = "parallel")]
    if limit.is_none() && scan_rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        let rows: Vec<Row> = scan_rows.par_iter().filter_map(build_row).collect();
        return Bindings { vars, rows, sorted_by };
    }

    // Reserve only up to the LIMIT so a small LIMIT over a huge scan does not
    // allocate for the whole relation (the point of early termination).
    let cap = limit.map_or(scan_rows.len(), |n| n.min(scan_rows.len()));
    let mut rows: Vec<Row> = Vec::with_capacity(budget::cap_alloc(cap));
    for (i, row) in scan_rows.iter().enumerate() {
        // Coarse budget check every 4096 scanned rows.
        if i & 4095 == 0 && budget::exhausted(rows.len()) {
            break;
        }
        if let Some(out) = build_row(row) {
            rows.push(out);
            // LIMIT early-termination: stop scanning once we have enough rows.
            if let Some(n) = limit {
                if rows.len() >= n {
                    break;
                }
            }
        }
    }
    Bindings { vars, rows, sorted_by }
}
```

### crates/sparq-engine/src/exec.rs:4991-4998 — eval_graph_named
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn eval_graph_named(
    graph: &Graph,
    local: &mut LocalVocab,
    name: &NamedNodePattern,
    inner: &GraphPattern,
) -> Result<Bindings, String> {
    eval_graph_named_pref(graph, local, name, inner, None)
}
```

### crates/sparq-engine/src/exec.rs:5049-5233 — eval_graph_named_pref
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn eval_graph_named_pref(
    graph: &Graph,
    local: &mut LocalVocab,
    name: &NamedNodePattern,
    inner: &GraphPattern,
    prefix: Option<&str>,
) -> Result<Bindings, String> {
    fn eval_translated(
        graph: &Graph,
        local: &mut LocalVocab,
        sub: &Graph,
        #[cfg(feature = "zk")] gname: &Term,
        inner: &GraphPattern,
    ) -> Result<Bindings, String> {
        // Inside GRAPH the evaluation graph IS the named sub-graph: suspend a
        // view's empty-default short-circuit for the inner pattern (L1 view).
        let _scope = view::enter_graph();
        // zk-trace: tag the enclosed scans/filters with the named graph (the
        // sub-graph has its own dictionary; terms are materialized at record
        // time against it, so the tag is what attributes them). The `gname`
        // parameter is cfg'd out entirely when the feature is off, so the
        // default (wasm) build is byte-identical.
        #[cfg(feature = "zk")]
        let _zk = crate::zk::graph_scope(gname);
        let mut sub_local = LocalVocab::default();
        let b = eval_graph_pattern(sub, &mut sub_local, inner)?;
        let rows: Vec<Row> = b
            .rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&id| match term_of(sub, &sub_local, id) {
                        Some(t) => value_to_id(graph, local, &Value::Term(t)),
                        None => NO_ID,
                    })
                    .collect()
            })
            .collect();
        Ok(Bindings::unsorted(b.vars, rows))
    }
    match name {
        NamedNodePattern::NamedNode(n) => {
            let target = Term::NamedNode(n.clone());
            // A graph outside an installed dataset view takes the absent-graph
            // branch below: non-visible must be INDISTINGUISHABLE from absent
            // (the L1 view's security property).
            let sub = if view::allows(&target) {
                graph.named.iter().find(|(t, _)| *t == target).map(|(_, sub)| sub)
            } else {
                None
            };
            match sub {
                Some(sub) => eval_translated(
                    graph,
                    local,
                    sub,
                    #[cfg(feature = "zk")]
                    &target,
                    inner,
                ),
                // The named graph is absent → ZERO solutions (even for `GRAPH <g> {}`,
                // which must NOT yield the unit row), but with `inner`'s variable
                // schema — evaluate against an empty graph for the columns, then drop
                // any rows (an empty group pattern would otherwise produce one).
                None => {
                    let _scope = view::enter_graph(); // schema eval matches the present-graph path
                    // zk-trace: an absent graph still records the operator
                    // boundary + (empty) pattern input sets under its name.
                    #[cfg(feature = "zk")]
                    let _zk = crate::zk::graph_scope(&target);
                    let empty = Graph::load_str("", "ntriples").map_err(|e| e.to_string())?;
                    let mut el = LocalVocab::default();
                    let mut b = eval_graph_pattern(&empty, &mut el, inner)?;
                    b.rows.clear();
                    Ok(b)
                }
            }
        }
        NamedNodePattern::Variable(v) => {
            // [OPUS-4.8] (sq-zz8z) Accumulate the per-graph relations into ONE flat row buffer in
            // a stable column schema (`?g` first, then the inner pattern's columns) rather than
            // folding with `union_bindings` once per graph. The old fold re-copied the WHOLE
            // accumulated relation on every graph, making `GRAPH ?g` over G graphs O(G²); a single
            // shared schema makes it O(total rows). The `?g`-first schema matches the old
            // `None`-branch insert order, so projected results are unchanged.
            let mut out_vars: Option<Vec<Variable>> = None;
            let mut out_rows: Vec<Row> = Vec::new();
            let mut per_graph = |graph: &Graph, local: &mut LocalVocab, gname: &Term, sub: &Graph| -> Result<(), String> {
                // zk-trace: each iteration of `GRAPH ?g` tags the enclosed scans/filters with the
                // iteration's named graph — the scope is installed INSIDE eval_translated (one
                // place), so the operator boundary stream is not double-nested.
                let mut b = eval_translated(
                    graph,
                    local,
                    sub,
                    #[cfg(feature = "zk")]
                    gname,
                    inner,
                )?;
                let gid = value_to_id(graph, local, &Value::Term(gname.clone()));
                // Resolve this graph's columns into the shared `?g`-first schema (set on the first
                // graph; every named sub-graph yields the same `inner` schema, so it is stable).
                let schema = out_vars.get_or_insert_with(|| {
                    let mut s = Vec::with_capacity(b.vars.len() + 1);
                    s.push(v.clone());
                    for var in &b.vars {
                        if var != v {
                            s.push(var.clone());
                        }
                    }
                    s
                });
                match b.col(v) {
                    // The inner pattern itself binds the graph variable (e.g.
                    // `GRAPH ?g { ?g :p ?o }` or a VALUES/OPTIONAL inside): JOIN with
                    // the active graph name — keep rows already bound to this graph,
                    // fill unbound cells, drop conflicting rows.
                    Some(c) => {
                        b.rows.retain_mut(|row| {
                            if row[c] == NO_ID {
                                row[c] = gid;
                                true
                            } else {
                                row[c] == gid
                            }
                        });
                    }
                    None => {
                        b.vars.insert(0, v.clone());
                        for row in &mut b.rows {
                            row.insert(0, gid);
                        }
                    }
                }
                // Map each row into the shared schema (column positions can differ per graph only
                // if the inner schema ever reordered — it does not — so this is a cheap permute).
                // Precompute schema-column -> position-in-`b.vars` ONCE per binding-set (instead of
                // an O(vars) linear `position(..)` search per (row, var) cell — an O(rows·vars²)
                // hotspot on large `GRAPH ?g` scans), then map each row with O(1) indexed lookups.
                let col_map: Vec<Option<usize>> = schema
                    .iter()
                    .map(|var| b.vars.iter().position(|x| x == var))
                    .collect();
                for row in &b.rows {
                    out_rows.push(
                        col_map
                            .iter()
                            .map(|&pos| pos.map(|i| row[i]).unwrap_or(NO_ID))
                            .collect(),
                    );
                }
                Ok(())
            };
            match prefix {
                // Indexed range scan over only the prefix-matching graphs (O(log G + matches)).
                // The view-visibility (L1) check stays — a non-visible graph is still skipped.
                Some(pref) => {
                    let mut err: Option<String> = None;
                    graph.for_named_graphs_with_prefix(pref, |gname, sub| {
                        if err.is_some() || !view::allows(gname) {
                            return;
                        }
                        if let Err(e) = per_graph(graph, local, gname, sub) {
                            err = Some(e);
                        }
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                }
                // Full enumeration (no prefix restriction).
                None => {
                    for (gname, sub) in &graph.named {
                        if !view::allows(gname) {
                            continue; // not visible under the installed dataset view (L1)
                        }
                        per_graph(graph, local, gname, sub)?;
                    }
                }
            }
            let vars = out_vars.unwrap_or_else(|| vec![v.clone()]);
            Ok(Bindings::unsorted(vars, out_rows))
        }
    }
}
```

### crates/sparq-engine/src/exec.rs:11683-11936 — order_bindings
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn order_bindings(
    graph: &Graph,
    local: &LocalVocab,
    b: &mut Bindings,
    exprs: &[OrderExpression],
    row_budget: Option<usize>,
) -> Result<(), String> {
    // Pre-resolve Variable → column index once for each ORDER expression. [OPUS-4.8] sq-7d3dj.4.
    let compiled_order: Vec<(bool, CompiledExpr)> = exprs
        .iter()
        .map(|oe| match oe {
            OrderExpression::Asc(e) => (false, compile_expr(e, b)),
            OrderExpression::Desc(e) => (true, compile_expr(e, b)),
        })
        .collect();

    // The sort key cell for one compiled ORDER expression of one row. Numeric keys use the
    // numerics cache and temporal keys the temporals cache (no per-comparison reparse); IRI
    // terms precompute the IRI string once (SortCell::Iri) — eliminating per-comparison
    // term_of materialisation + value_str allocation for IRI ORDER BY columns; other
    // expressions fall back to identity-preserving evaluation. The plain-variable case is
    // unpacked here so the column lookup and the cache probes happen exactly once per row
    // (column index was pre-resolved above). [OPUS-4.8] sq-7d3dj.4 / [SONNET-4.6] sq-7d3dj.30.2 (Iri).
    let cell_of = |row: &Row, e: &CompiledExpr| -> Result<SortCell, String> {
        if let CompiledExpr::Var(Some(c)) = e {
            let id = row[*c];
            if id != NO_ID && !is_local(id) {
                if let Some(n) = graph.numeric_value(id) {
                    // [OPUS-4.8] sq-rikm7: keep the id so an f64 TIE can be rechecked
                    // exactly (integers > 2^53 / high-precision decimals sharing one f64).
                    return Ok(SortCell::Num { f: n, id });
                }
                if let Some(t) = graph.temporal_value(id) {
                    return Ok(SortCell::Temp { t, id });
                }
                // [FABLE-5] sq-7d3dj.30.21 — LAZY STRING-LITERAL key: a plain `xsd:string`
                // store-literal becomes a zero-allocation `SortCell::StrId(id)` (compared via
                // the literal's ZERO-COPY `Dict::term_parts` `value` bytes) instead of eagerly
                // reconstructing a `Literal` (`term_of`) AND allocating a `value_str` collation
                // key (`sort_cell_val`) for every input row. `plain_string_value` is a single
                // record probe that returns `Some` for EXACTLY the `LiteralKind::String` set
                // (datatype `xsd:string`, no lang tag), so the SPARQL value-order is preserved.
                // The eager `Val{..}` key remains the feature-OFF path (byte-identical order).
                // This targets the SP2Bench q11 residual: N=17663 keyed, k=60 survive.
                #[cfg(feature = "topk-lazy-strkey")]
                if graph.dict.plain_string_value(id).is_some() {
                    return Ok(SortCell::StrId(id));
                }
                // Neither numeric nor temporal: materialise the term once. For IRI terms,
                // store the IRI string in SortCell::Iri so comparisons are direct &str
                // comparisons with no per-comparison allocation. [SONNET-4.6] sq-7d3dj.30.2
                let term = term_of(graph, local, id).expect("bound id resolves");
                if let Term::NamedNode(n) = &term {
                    return Ok(SortCell::Iri(n.as_str().into()));
                }
                return Ok(sort_cell_val(Value::Term(term)));
            }
        }
        Ok(match eval_compiled_numeric(graph, local, row, e) {
            Some(n) => sort_cell_val(Value::Num(Num::Double(n))),
            None => sort_cell_val(eval_compiled(graph, local, b, row, e)?),
        })
    };
    // The sort key (vector of (descending, SortCell)) for one row.
    let key_of = |row: &Row| -> Result<Vec<(bool, SortCell)>, String> {
        let mut key = Vec::with_capacity(compiled_order.len());
        for (desc, ce) in &compiled_order {
            key.push((*desc, cell_of(row, ce)?));
        }
        Ok(key)
    };

    let n = b.rows.len();

    // Top-k bounded-selection path: O(n + k log k) instead of O(n log n).
    // Activated when the caller supplies a row_budget k < n. The comparator
    // adds `input_idx` as a tiebreaker so the partition and final sort reproduce
    // the stable-sort tie semantics exactly (smaller input_idx appears first).
    // [SONNET-4.6] sq-7d3dj.30.2
    if let Some(k) = row_budget {
        // LIMIT 0 (k == 0) is legal SPARQL and is exercised by the W3C conformance
        // suite. The result is empty regardless of order, so short-circuit here:
        // `select_nth_unstable_by(k - 1)` below would underflow `k - 1` to
        // `usize::MAX` and abort the process (SIGABRT). [OPUS-4.8] sq-7d3dj.30.2
        if k == 0 {
            b.rows.clear();
            b.sorted_by = None;
            return Ok(());
        }
        // Invariant for the bounded-selection path below: 0 < k < n, so
        // `k - 1 < n = keyed.len()` and `select_nth_unstable_by(k - 1)` is in range.
        if k < n {
            // Build (key, input_idx) — parallel for large sets. INDEX-CARRY: the keyed
            // vector holds only the sort key + the original `b.rows` index, NOT a cloned
            // `Row`. `key_of` borrows each row read-only (via `cell_of`, which touches
            // `graph`/`local` but never `b.rows`), so `b.rows` is still owned + intact
            // after this build; only the k surviving rows are cloned when we gather the
            // output below. This eliminates the n up-front `Row::clone`s of the previous
            // `(key, input_idx, Row)` tuple — only k clones happen (n=17663 → k=60 on the
            // SP2Bench q11 residual). Byte-identical output: the tuple index reproduces the
            // same stable-sort tie order, and the gathered row is `b.rows[i]` unchanged.
            // [SONNET-4.6] sq-7d3dj.30.23
            // Use Vec<_> to avoid the clippy::type_complexity lint on the explicit type.
            #[cfg(feature = "parallel")]
            let mut keyed: Vec<_> = if n >= PAR_THRESHOLD {
                use rayon::prelude::*;
                // sq-6aefu: mirror FILTER/BIND worker_install pattern — key_of -> eval_compiled
                // can re-enter EXISTS / custom extension functions / spatial expressions on rayon
                // workers; without the snapshot+reinstall the workers see NO view (named-graph
                // leak) and NO function registry (spurious error). [SONNET-4.6]
                let fns = functions::snapshot();
                let vw = view::snapshot();
                let spx = spatial::snapshot();
                // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
                // a worker that missed the registry would dial the IRI instead of answering it.
                #[cfg(feature = "service-local")]
                let lsv = local_services::snapshot();
                #[cfg(not(target_arch = "wasm32"))]
                let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
                b.rows
                    .par_iter()
                    .enumerate()
                    .map(|(i, row)| {
                        let _fns = functions::worker_install(&fns);
                        let _vw = view::worker_install(&vw);
                        let _spx = spatial::worker_install(&spx);
                        #[cfg(feature = "service-local")]
                        let _lsv = local_services::worker_install(&lsv);
                        #[cfg(not(target_arch = "wasm32"))]
                        let _qn = query_now::worker_install(qn);
                        Ok((key_of(row)?, i))
                    })
                    .collect::<Result<Vec<_>, String>>()?
            } else {
                b.rows
                    .iter()
                    .enumerate()
                    .map(|(i, row)| Ok((key_of(row)?, i)))
                    .collect::<Result<Vec<_>, String>>()?
            };
            #[cfg(not(feature = "parallel"))]
            let mut keyed: Vec<_> = b
                .rows
                .iter()
                .enumerate()
                .map(|(i, row)| Ok((key_of(row)?, i)))
                .collect::<Result<_, String>>()?;

            // Strict total-order comparator: sort key first (descending as flagged),
            // then input_idx as tiebreaker (smaller index = appears earlier in the
            // stable sort, so it sorts LESS here — reproduces the stable sort exactly).
            let cmp_total = |a: &(Vec<(bool, SortCell)>, usize), c: &(Vec<(bool, SortCell)>, usize)| {
                for ((desc, av), (_, cv)) in a.0.iter().zip(c.0.iter()) {
                    let ord = cmp_sort_cells(graph, local, av, cv);
                    let ord = if *desc { ord.reverse() } else { ord };
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                // Tiebreak: smaller input_idx wins (matches stable sort input order).
                a.1.cmp(&c.1)
            };

            // Partition: after select_nth_unstable_by(k-1), keyed[..k] holds the k
            // smallest elements by cmp_total (not yet sorted within that prefix).
            // select_nth_unstable_by requires k > 0 and k-1 < len, both guaranteed here.
            debug_assert!(k > 0 && k <= keyed.len());
            keyed.select_nth_unstable_by(k - 1, |a, c| cmp_total(a, c));

            // Sort the selected prefix into the correct final order.
            keyed[..k].sort_by(|a, c| cmp_total(a, c));

            // Gather the k surviving rows by index — the ONLY `Row::clone`s in this path.
            // `keyed[..k]` is now in final sorted order, so the gathered rows land in that
            // exact order. [SONNET-4.6] sq-7d3dj.30.23
            b.rows = keyed[..k].iter().map(|(_, i)| b.rows[*i].clone()).collect();
            b.sorted_by = None;
            return Ok(());
        }
        // k >= n: top-k budget covers all rows, fall through to full stable sort.
    }

    // Full stable sort path (unchanged). Precompute the keys (independent, read-only)
    // — in parallel for large result sets.
    #[cfg(feature = "parallel")]
    let mut keyed: Vec<(Vec<(bool, SortCell)>, Row)> = if n >= PAR_THRESHOLD {
        use rayon::prelude::*;
        // sq-6aefu: mirror FILTER/BIND worker_install pattern — same rationale as the
        // top-k path above: key_of -> eval_compiled can re-enter EXISTS / custom
        // extension functions / spatial on rayon workers. [SONNET-4.6]
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot();
        // [OPUS-5] (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
        // a worker that missed the registry would dial the IRI instead of answering it.
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        b.rows
            .par_iter()
            .map(|row| {
                let _fns = functions::worker_install(&fns);
                let _vw = view::worker_install(&vw);
                let _spx = spatial::worker_install(&spx);
                #[cfg(feature = "service-local")]
                let _lsv = local_services::worker_install(&lsv);
                #[cfg(not(target_arch = "wasm32"))]
                let _qn = query_now::worker_install(qn);
                Ok((key_of(row)?, row.clone()))
            })
            .collect::<Result<_, String>>()?
    } else {
        b.rows.iter().map(|row| Ok((key_of(row)?, row.clone()))).collect::<Result<_, String>>()?
    };
    #[cfg(not(feature = "parallel"))]
    let mut keyed: Vec<(Vec<(bool, SortCell)>, Row)> =
        b.rows.iter().map(|row| Ok((key_of(row)?, row.clone()))).collect::<Result<_, String>>()?;

    let cmp = |a: &(Vec<(bool, SortCell)>, Row), c: &(Vec<(bool, SortCell)>, Row)| {
        for ((desc, av), (_, cv)) in a.0.iter().zip(c.0.iter()) {
            let ord = cmp_sort_cells(graph, local, av, cv);
            let ord = if *desc { ord.reverse() } else { ord };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    };
    // ORDER BY tie handling [OPUS-4.8]: `cmp` returns `Ordering::Equal` only for rows that
    // tie on *every* ORDER BY key (no secondary tie-breaker), and SPARQL leaves the relative
    // order of ORDER BY-tied solutions unspecified, so any tie order is conformant. We still
    // keep it *deterministic at a fixed thread count*: both branches use a STABLE sort
    // (`rayon::par_sort_by` is a stable parallel merge sort, same guarantee as `slice::sort_by`),
    // so ties retain their `b.rows` (scan) input order rather than being shuffled by the
    // parallel partitioning. The only residual cross-thread-count variation is that `b.rows`
    // itself is in dict-id (scan) order, which is thread-count-dependent — that is the
    // umbrella dict-id-order property (research/dict-id-order-determinism-audit.md), not a
    // tie-break defect in this sort. So no total-order tie-breaker is added: it would cost a
    // term materialisation per tied row for an order the spec does not constrain.
    #[cfg(feature = "parallel")]
    if keyed.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        keyed.par_sort_by(cmp);
    } else {
        keyed.sort_by(cmp);
    }
    #[cfg(not(feature = "parallel"))]
    keyed.sort_by(cmp);

    b.rows = keyed.into_iter().map(|(_, r)| r).collect();
    b.sorted_by = None;
    Ok(())
}
```

### crates/sparq-engine/src/exec.rs:68-476 — budget production portion (unchanged test modules omitted)
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) mod budget {
    use crate::QueryBudget;
    use sparq_core::dict::Id;
    use std::cell::Cell;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Copyable view of a cancellation flag owned by the installed [`QueryBudget`].
    ///
    /// The [`Guard`] lifetime keeps that budget (and therefore its `Arc<AtomicBool>`)
    /// alive until the pointer has been cleared from the thread-local state. Rayon
    /// snapshots are consumed only by scoped parallel iterators that join before the
    /// guard is dropped. The pointer is dereferenced only for atomic loads.
    #[derive(Clone, Copy)]
    struct CancelPtr(NonNull<AtomicBool>);

    // SAFETY: `AtomicBool` is `Sync`; moving this shared pointer to a worker is
    // sound because it is only dereferenced for atomic loads while `Guard` keeps
    // the owning `Arc` alive, including across scoped rayon work.
    unsafe impl Send for CancelPtr {}
    // SAFETY: `AtomicBool` is `Sync`; all shared access through `CancelPtr` is an
    // atomic load, and `Guard` keeps the allocation alive until worker joins finish.
    unsafe impl Sync for CancelPtr {}

    /// Bytes one id-level binding cell occupies in a materialised `Row`. The
    /// byte-accounted cap ([OPUS-4.8] sq-s5is) costs the id-level working set as
    /// `rows × width × BYTES_PER_ID` — a portable LOWER bound on real heap (it
    /// ignores allocator overhead / `SmallVec` inline-vs-spill), conservative in the
    /// same direction the row cap is.
    pub(crate) const BYTES_PER_ID: usize = std::mem::size_of::<Id>();

    /// The installed limits, flattened for a cheap per-check read.
    #[derive(Clone, Copy)]
    pub(crate) struct Limits {
        on: bool,
        #[cfg(not(target_arch = "wasm32"))]
        deadline: Option<std::time::Instant>,
        max_rows: usize,
        /// [OPUS-4.8] (sq-s5is) Byte ceiling on the estimated working set; `usize::MAX`
        /// when no byte cap is set. Compared against `rows × byte_width + extra_bytes`.
        max_bytes: usize,
        /// [OPUS-4.8] (sq-s5is) Bytes per row of the working set CURRENTLY being checked
        /// — `width(in ids) × BYTES_PER_ID`. Set per operator by [`set_width`] so the
        /// row-count check sites also price WIDTH (the dimension the row cap misses). A
        /// scalar/streaming path that never sets a width leaves this at `BYTES_PER_ID`
        /// (one id per "row"), so the byte cap degrades to the row cap there, never wider.
        byte_width: usize,
        /// [OPUS-4.8] (sq-s5is) Bytes of query-computed terms interned into the per-query
        /// local vocabulary (BIND / aggregate / CONSTRUCT scratch) — the NON-row dimension
        /// the row cap also misses. A running high-water sum, added to the working-set
        /// estimate on every check.
        extra_bytes: usize,
        cancel: Option<CancelPtr>,
    }

    const OFF: Limits = Limits {
        on: false,
        #[cfg(not(target_arch = "wasm32"))]
        deadline: None,
        max_rows: usize::MAX,
        max_bytes: usize::MAX,
        byte_width: BYTES_PER_ID,
        extra_bytes: 0,
        cancel: None,
    };

    impl Limits {
        /// `rows × byte_width + extra_bytes`, saturating — the estimated working-set
        /// byte size compared against `max_bytes`. [OPUS-4.8] (sq-s5is)
        #[inline]
        fn bytes(&self, rows: usize) -> usize {
            rows.saturating_mul(self.byte_width).saturating_add(self.extra_bytes)
        }

        /// WHY the limits are hit at `rows`, or `None` when they are not — the pure (no
        /// thread-local) counterpart of [`exhausted`]'s reason, for rayon closures where
        /// the installing thread's sticky flag is out of reach. The reasons are the SAME
        /// strings [`exhausted`] records, so a worker can raise EXACTLY the error
        /// [`check`] would rather than inventing one (or guessing a result). [SONNET-4.6]
        /// (sq-qk6ac)
        #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
        #[inline]
        pub(crate) fn why(&self, rows: usize) -> Option<&'static str> {
            if !self.on {
                return None;
            }
            if rows > self.max_rows {
                return Some("max-rows");
            }
            if self.bytes(rows) > self.max_bytes {
                return Some("max-bytes");
            }
            #[cfg(not(target_arch = "wasm32"))]
            if self.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                return Some("timeout");
            }
            if let Some(cancel) = self.cancel {
                // SAFETY: `CancelPtr`'s invariant and the lifetime-bound `Guard`
                // keep the `AtomicBool` alive for this scoped snapshot load.
                if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                    return Some("cancelled");
                }
            }
            None
        }

        /// Pure (no thread-local) exhaustion test for rayon closures, where the
        /// installing thread's sticky flag is out of reach: a worker that sees
        /// `hit` stops producing, and the caller's next on-thread check fires
        /// (the deadline is global time; a hit row/byte cap leaves the snapshot's
        /// estimate over the limit). Only the rayon-parallel branches call this
        /// (and `snapshot`); the non-parallel (wasm) build compiles them out.
        #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
        #[inline]
        pub(crate) fn hit(&self, rows: usize) -> bool {
            self.why(rows).is_some()
        }
    }

    thread_local! {
        static ACTIVE: Cell<Limits> = const { Cell::new(OFF) };
        static EXCEEDED: Cell<Option<&'static str>> = const { Cell::new(None) };
    }

    /// Clears the budget when the `*_with_budget` entry point returns (also on
    /// error/unwind, so a poisoned thread never leaks a stale budget).
    pub(crate) struct Guard<'a> {
        _budget: std::marker::PhantomData<&'a QueryBudget>,
        _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.set(OFF));
            EXCEEDED.with(|e| e.set(None));
        }
    }

    pub(crate) fn install(b: &QueryBudget) -> Guard<'_> {
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
        ACTIVE.with(|a| {
            a.set(Limits {
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
        EXCEEDED.with(|e| e.set(None));
        Guard {
            _budget: std::marker::PhantomData,
            _not_send: std::marker::PhantomData,
        }
    }

    /// [OPUS-4.8] (sq-s5is) Sets the per-row byte width (= `width_in_ids ×
    /// BYTES_PER_ID`) of the working set the next row-count checks price. Called once
    /// per operator with that operator's output arity, so a check on `rows` correctly
    /// estimates `rows × width` bytes — the WIDE-row dimension the row cap misses.
    /// No-op (and no thread-local write on the unbudgeted hot path) when no budget is
    /// installed. Returns the previous width so callers can restore it.
    #[inline]
    pub(crate) fn set_width(width_in_ids: usize) -> usize {
        ACTIVE.with(|c| {
            let mut a = c.get();
            let prev = a.byte_width;
            if a.on {
                a.byte_width = width_in_ids.max(1).saturating_mul(BYTES_PER_ID);
                c.set(a);
            }
            prev
        })
    }

    /// [OPUS-4.8] (sq-s5is) Restores a byte width previously returned by [`set_width`]
    /// (cheap: one thread-local write, only while budgeted).
    #[inline]
    pub(crate) fn restore_width(prev: usize) {
        ACTIVE.with(|c| {
            let mut a = c.get();
            if a.on {
                a.byte_width = prev;
                c.set(a);
            }
        });
    }

    /// [OPUS-4.8] (sq-s5is) Adds `n` bytes of query-computed terms to the local-vocab
    /// high-water accumulator (the NON-row dimension). Trips the sticky flag immediately
    /// if it pushes the estimate over `max_bytes`, so an oversized CONSTRUCT template /
    /// aggregate scratch is caught even between row-count checks. No-op when unbudgeted.
    #[inline]
    pub(crate) fn add_bytes(n: usize) {
        ACTIVE.with(|c| {
            let mut a = c.get();
            if !a.on {
                return;
            }
            a.extra_bytes = a.extra_bytes.saturating_add(n);
            c.set(a);
            if a.extra_bytes > a.max_bytes {
                EXCEEDED.with(|e| {
                    if e.get().is_none() {
                        e.set(Some("max-bytes"));
                    }
                });
            }
        });
    }

    /// Snapshot of the installed limits, for the rayon-parallel branches.
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    #[inline]
    pub(crate) fn snapshot() -> Limits {
        ACTIVE.with(|a| a.get())
    }

    /// Decides whether the multi-core SELECT-JSON serializer may fan out under the
    /// CURRENTLY installed budget, returning the limit snapshot its workers re-check at
    /// each par-chunk boundary. [OPUS-4.8] (sq-7d3dj.10, roborev 1538, audit item 6)
    ///
    /// The parallel path builds every matching JSON fragment before it can know a row or
    /// byte count, so it cannot enforce a ROW / BYTE cap mid-serialize:
    ///
    /// * `Some(limits)` — fan out. Either NO budget is installed (`limits.on == false`,
    ///   making the per-chunk `hit` re-check a no-op) OR the budget is DEADLINE-ONLY
    ///   (both row and byte caps at their `usize::MAX` sentinel). Under a deadline-only
    ///   budget the per-chunk `limits.hit(0)` re-check stops launching new chunks once
    ///   the wall-clock deadline has passed, so the worst-case CPU overrun is bounded to
    ///   the chunks already in flight — approximately one per worker, a bounded constant
    ///   — not the unbounded burn an uncheckable fan-out under a row/byte cap would allow.
    /// * `None` — a row and/or byte cap is installed; the caller must take the
    ///   cooperative SERIAL loop, which cannot over-produce (it checks the sticky flag
    ///   every 1024 rows and stops early). A blanket "fan out whenever a budget is
    ///   installed" was REJECTED for exactly this reason (roborev 1538 / audit item 6).
    ///
    /// Compiled only for the `parallel` feature — the wasm/serial build never fans out.
    #[cfg(feature = "parallel")]
    #[inline]
    pub(crate) fn parallel_json_fanout() -> Option<Limits> {
        ACTIVE.with(|a| {
            let l = a.get();
            if l.on && (l.max_rows != usize::MAX || l.max_bytes != usize::MAX) {
                None // a row/byte cap the fan-out cannot enforce mid-serialize → serial loop
            } else {
                Some(l)
            }
        })
    }

    /// Time remaining until the installed wall-clock deadline, if any. [OPUS-4.8] (sq-d4p)
    ///
    /// The SERVICE HTTP transport uses this to bound a remote round-trip by the SAME
    /// budget that bounds local evaluation: a query under a 5s deadline must not block
    /// for the transport's fixed default on an unresponsive endpoint. Returns:
    /// * `None` — no deadline installed (no budget, or a row/byte-only budget); the
    ///   transport keeps its own finite default.
    /// * `Some(Duration::ZERO)` — the deadline has already passed; the caller should
    ///   refuse the remote call immediately rather than dial.
    /// * `Some(d)` — the remaining time, which the transport caps its own default to.
    ///
    /// Always compiled only off-wasm (no `Instant` there, and the `service` feature
    /// never reaches a wasm build).
    #[cfg(not(target_arch = "wasm32"))]
    #[cfg_attr(not(feature = "service"), allow(dead_code))]
    #[inline]
    pub(crate) fn remaining_timeout() -> Option<std::time::Duration> {
        ACTIVE.with(|a| {
            let lim = a.get();
            lim.deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()))
        })
    }

    /// A savepoint of the local-vocab byte accumulator + the sticky exhaustion flag,
    /// taken BEFORE a speculative interning burst — a streaming SERVICE block whose
    /// rows a SILENT error must discard. [`restore_bytes`] rewinds to it so the
    /// discarded interns leave the byte budget EXACTLY as if they never happened,
    /// keeping SILENT SERVICE behaviour-neutral with the pre-streaming
    /// collect-then-intern-on-success path (which charged nothing on a swallowed
    /// remote error). [OPUS-4.8] (sq-my8wd.4)
    #[cfg(any(feature = "service", feature = "service-local"))]
    #[derive(Clone, Copy)]
    pub(crate) struct ByteSavepoint {
        extra_bytes: usize,
        exceeded: Option<&'static str>,
    }

    /// Capture the current byte accumulator + exhaustion flag. [OPUS-4.8] (sq-my8wd.4)
    #[cfg(any(feature = "service", feature = "service-local"))]
    #[inline]
    pub(crate) fn byte_savepoint() -> ByteSavepoint {
        ByteSavepoint {
            extra_bytes: ACTIVE.with(|c| c.get().extra_bytes),
            exceeded: EXCEEDED.with(|e| e.get()),
        }
    }

    /// Rewind the byte accumulator + exhaustion flag to a [`ByteSavepoint`]. Only the
    /// bytes charged (and any max-bytes exhaustion tripped) SINCE the savepoint are
    /// undone; an exhaustion that fired for an independent reason before it is
    /// preserved. Sound because the interning burst it brackets is synchronous and
    /// single-threaded — the SERVICE sink is the only writer between the savepoint and
    /// here — so the pre-burst snapshot is exactly the current state minus this burst.
    /// A deadline that elapsed during the burst is not masked: the next `exhausted`
    /// re-derives it from the wall clock. [OPUS-4.8] (sq-my8wd.4)
    #[cfg(any(feature = "service", feature = "service-local"))]
    #[inline]
    pub(crate) fn restore_bytes(sp: ByteSavepoint) {
        ACTIVE.with(|c| {
            let mut a = c.get();
            a.extra_bytes = sp.extra_bytes;
            c.set(a);
        });
        EXCEEDED.with(|e| e.set(sp.exceeded));
    }

    /// `true` once the budget is exhausted (sticky) — row-producing loops break
    /// on it; `rows` is the loop's current output size.
    #[inline]
    pub(crate) fn exhausted(rows: usize) -> bool {
        let a = ACTIVE.with(|c| c.get());
        if !a.on {
            return false;
        }
        if EXCEEDED.with(|e| e.get()).is_some() {
            return true;
        }
        if rows > a.max_rows {
            EXCEEDED.with(|e| e.set(Some("max-rows")));
            return true;
        }
        if a.bytes(rows) > a.max_bytes {
            EXCEEDED.with(|e| e.set(Some("max-bytes")));
            return true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if a.deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            EXCEEDED.with(|e| e.set(Some("timeout")));
            return true;
        }
        if let Some(cancel) = a.cancel {
            // SAFETY: `CancelPtr`'s invariant and the lifetime-bound `Guard` keep
            // the `AtomicBool` alive until this thread-local pointer is cleared.
            // Relaxed is sufficient because cancellation gates control flow only;
            // it never publishes or guards a shared query buffer. If that changes,
            // the load/store pair must become Acquire/Release.
            if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                EXCEEDED.with(|e| e.set(Some("cancelled")));
                return true;
            }
        }
        false
    }

    /// Propagates an exhausted budget as the query error.
    #[inline]
    pub(crate) fn check(rows: usize) -> Result<(), String> {
        if exhausted(rows) {
            let why = EXCEEDED.with(|e| e.get()).unwrap_or("timeout");
            return Err(format!("query budget exceeded ({why})"));
        }
        Ok(())
    }

    /// Returns `true` when a budget is currently installed (even if not yet exhausted).
    /// The columnar path uses this for the I3 fallback rule: when a budget is armed the
    /// seam declines to the scalar path (the scalar debit schedule is not uniform-per-row
    /// inside `apply_filter`, so the `k = min(batch_len, budget_remaining)` prefix rule
    /// cannot be applied; the fallback is budget-armed ⇒ decline per the design record
    /// `research/vector-at-a-time-m4-completion-design.md` §1 I3). [SONNET-4.6] (sq-pntvh.5)
    #[cfg_attr(not(feature = "vectorized"), allow(dead_code))]
    #[inline]
    pub(crate) fn active() -> bool {
        ACTIVE.with(|c| c.get().on)
    }

    /// Caps a speculative `Vec` pre-allocation while a budget is active, so a
    /// budgeted cross-product cannot allocate its full (possibly astronomical)
    /// output up front before the first cooperative check fires. Honours BOTH the
    /// row cap and (via `byte_width`) the byte cap — whichever admits fewer rows.
    #[inline]
    pub(crate) fn cap_alloc(cap: usize) -> usize {
        let a = ACTIVE.with(|c| c.get());
        if !a.on {
            return cap;
        }
        // Rows the byte cap still admits, given the current width and accrued extra.
        let by_bytes = a
            .max_bytes
            .saturating_sub(a.extra_bytes)
            .checked_div(a.byte_width.max(1))
            .unwrap_or(usize::MAX)
            .saturating_add(1);
        cap.min(a.max_rows.saturating_add(1)).min(by_bytes).min(1 << 20)
    }

```

### crates/sparq-engine/src/exec.rs:2021-2129 — view
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) mod view {
    use crate::{DatasetView, DefaultGraphMode};
    use oxrdf::Term;
    use rustc_hash::FxHashSet;
    use std::cell::RefCell;
    use std::sync::Arc;

    /// The installed view, plus the "inside GRAPH" suspend flag:
    /// `eval_graph_named` swaps evaluation to the named sub-`Graph`, whose inner
    /// patterns must NOT be empty-defaulted (only the TOP-LEVEL graph scope is).
    #[derive(Clone, Default)]
    pub(crate) struct State {
        named: Option<Arc<FxHashSet<Term>>>,
        default_empty: bool,
        suspended: bool,
    }

    thread_local! {
        static ACTIVE: RefCell<State> = RefCell::new(State::default());
    }

    /// Restores the pre-install state when the installing entry point returns
    /// (also on error/unwind, so a poisoned thread never leaks a stale view).
    pub(crate) struct Guard(State);
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.with(|a| *a.borrow_mut() = std::mem::take(&mut self.0));
        }
    }

    pub(crate) fn install(v: &DatasetView) -> Guard {
        let new = State {
            named: Some(Arc::clone(&v.named)),
            default_empty: matches!(v.default, DefaultGraphMode::Empty),
            suspended: false,
        };
        Guard(ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), new)))
    }

    /// Fully suspends the view (named filter AND empty default) for a scope —
    /// used by the entry points once `dataset::build_active` has folded the view
    /// into a dataset-clause ACTIVE graph: the restriction is already applied,
    /// and re-filtering would make a non-visible FROM NAMED graph behave
    /// differently from an absent one (both must be the EMPTY active graph).
    pub(crate) fn suspend_all() -> Guard {
        Guard(ACTIVE.with(|a| std::mem::take(&mut *a.borrow_mut())))
    }

    /// RAII suspension of the empty-default short-circuit only, for GRAPH scope
    /// (the named-graph visibility filter stays active). Restores the previous
    /// flag on drop, so nested scopes compose.
    pub(crate) struct GraphScope(bool);
    impl Drop for GraphScope {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.borrow_mut().suspended = self.0);
        }
    }

    pub(crate) fn enter_graph() -> GraphScope {
        GraphScope(ACTIVE.with(|a| std::mem::replace(&mut a.borrow_mut().suspended, true)))
    }

    /// `true` when `name` is a visible named graph under the installed view
    /// (always true with no view installed).
    #[inline]
    pub(crate) fn allows(name: &Term) -> bool {
        ACTIVE.with(|a| a.borrow().named.as_ref().is_none_or(|s| s.contains(name)))
    }

    /// `true` when the view's default graph is EMPTY at the current scope —
    /// false with no view, under `StoreDefault`, or inside a GRAPH pattern.
    #[inline]
    pub(crate) fn default_is_empty() -> bool {
        ACTIVE.with(|a| {
            let s = a.borrow();
            s.default_empty && !s.suspended
        })
    }

    /// Snapshot of the installed view for the rayon-parallel expression branches
    /// (`None` — no view, the common case — makes [`worker_install`] free).
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn snapshot() -> Option<State> {
        ACTIVE.with(|a| {
            let s = a.borrow();
            (s.named.is_some() || s.default_empty).then(|| s.clone())
        })
    }

    /// Scoped re-install of a snapshot inside a rayon worker item. Restores the
    /// PREVIOUS thread-local value on drop: rayon runs some items on the
    /// installing thread itself, whose view must survive the item.
    pub(crate) struct WorkerGuard(Option<State>);
    impl Drop for WorkerGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.0.take() {
                ACTIVE.with(|a| *a.borrow_mut() = prev);
            }
        }
    }

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn worker_install(snap: &Option<State>) -> WorkerGuard {
        match snap {
            None => WorkerGuard(None),
            Some(s) => WorkerGuard(Some(ACTIVE.with(|a| std::mem::replace(&mut *a.borrow_mut(), s.clone())))),
        }
    }
}
```

### crates/sparq-engine/src/exec.rs:11312-11365 — SortCell
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
enum SortCell {
    Temp { t: Temporal, id: Id },
    /// A numeric GRAPH term: the cached f64 for the fast compare PLUS its dictionary id, so
    /// an f64 TIE can be rechecked EXACTLY from the id's exact lexical — distinct integers
    /// beyond 2^53 / high-precision decimals that share one f64 (the numerics cache stores
    /// only f64, so the fast path alone collapses them). This makes `ORDER BY ?v` over a
    /// numeric column agree with the relational `<` (`cmp_expr`) and MIN/MAX, which already
    /// recheck. [OPUS-4.8] sq-rikm7
    Num { f: f64, id: Id },
    /// A named-node (IRI) GRAPH term: the IRI string precomputed once at key-build time,
    /// eliminating per-comparison `term_of` materialisation and `value_str` allocation for
    /// IRI-typed ORDER BY columns (the SP2Bench q11 case: all ?ee values are IRIs, so
    /// every comparison calls into `compare_terms` which allocates the string twice).
    /// `cmp_sort_cells` can then directly compare two `&str` slices — no allocation.
    /// [SONNET-4.6] sq-7d3dj.30.2
    Iri(Box<str>),
    /// [FABLE-5] sq-7d3dj.30.21 — a PLAIN `xsd:string` LITERAL GRAPH term carried as its
    /// DICTIONARY ID only, deferring the value materialisation the eager [`SortCell::Val`] arm
    /// does at construction time (a `reconstruct_ref` `Literal` alloc + a `value_str`
    /// collation-key alloc per input row). Under the `topk-lazy-strkey` feature the top-k
    /// ORDER BY key build emits this instead of `Val{..}` for a plain-string column, so an
    /// ORDER-BY on such a column over N input rows does ZERO key allocation up front (the
    /// SP2Bench q11 residual: only k of the 17663 keyed rows survive). `cmp_sort_cells`
    /// compares two `StrId` cells by their ZERO-COPY `Dict::term_parts` `value` bytes, which is
    /// BYTE-IDENTICAL to comparing the two eager `value_str` keys (a plain string orders by its
    /// `value()` lexical bytes — the sq-7d3dj.30.12 differential test proves it). Cross-kind /
    /// cross-class comparisons use the fixed `String`-kind / `Literal`-class ranks (a plain
    /// string literal is always Literal-class, String-kind), so those arms need no
    /// materialisation either.
    #[cfg(feature = "topk-lazy-strkey")]
    StrId(Id),
    /// Any other key value (a plain / typed / language-tagged literal, a blank node, a
    /// quoted triple term, a computed value, or an unbound / error). Extends the `Iri`
    /// fast-path idea to EVERY `SortCell` kind (bead sq-7d3dj.30.12): the SPARQL
    /// total-order dispatch is HOISTED to cell-CONSTRUCTION time so a heap comparison is
    /// an integer / byte-slice compare, never a per-comparison `lit_kind` +
    /// `is_numeric_dt` + `value_str`-allocation re-derivation.
    ///
    /// - `class` — the [`TermClass`] rank (cross-class compares reduce to `class.cmp`).
    /// - `kind` — the within-Literal [`LiteralKind`] rank (cross-kind compares reduce to
    ///   `kind.cmp`); a fixed filler for non-literal classes (never consulted there).
    /// - `key` — the PRECOMPUTED lexical collation key (`value_str`), present for exactly
    ///   the kinds whose within-kind order IS the lexical `value()` order — the `String`,
    ///   `Lang` and `Other` literal kinds and the `Blank` class (see `cmp_sort_cells`'s
    ///   `(Val, Val)` arm for why this reproduces `compare_terms` exactly there). `None`
    ///   for the VALUE-ordered kinds (numeric exact-tie / boolean / temporal timeline) and
    ///   for triple terms, which keep `v`'s full `compare_values` semantics.
    /// - `v` — retained so a value-ordered same-kind pair (and the triple-term recursion)
    ///   stays byte-identical to `compare_terms`.
    ///
    /// All ranks + the key are taken ONCE, from the very `CompareTerm` observations
    /// `compare_terms` would otherwise recompute on every comparison. [FABLE-5] sq-7d3dj.30.12
    Val { class: u8, kind: u8, key: Option<Box<str>>, v: Value },
}
```

### crates/sparq-engine/src/exec.rs:11374-11401 — sort_cell_val
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn sort_cell_val(v: Value) -> SortCell {
    let class = v.term_class() as u8;
    // Only the literal class consults the kind rank; skip the (cheap but non-trivial)
    // `lit_kind` dispatch entirely for the non-literal classes.
    let kind = if class == TermClass::Literal as u8 { v.literal_kind() as u8 } else { 0 };
    // Precompute the lexical key for EXACTLY the kinds whose within-kind (and, for the
    // literal kinds, same-KIND) order is the `value_str` lexical order — so a same-kind
    // compare is a direct slice compare with NO per-comparison allocation, reproducing
    // `compare_terms`' within-kind result (proven byte-identical by the differential test):
    //   • String / Lang / Other literal kinds — `compare_terms`' `strict_cmp` arm (where it
    //     decides: same-tag / same-other-XSD) yields the SAME `value()` order as its
    //     `value_str` fallback (cross-tag / cross-other-XSD), so the whole kind orders by
    //     `value_str` (`value()`);
    //   • the Blank class — `compare_terms` orders blanks by their `value_str` label.
    // Numeric (exact-tie recheck), Boolean and the temporal kinds are VALUE-ordered, and
    // triple terms recurse — those keep `None` and defer to `compare_values`.
    let key = if class == TermClass::Blank as u8
        || (class == TermClass::Literal as u8
            && (kind == LiteralKind::String as u8
                || kind == LiteralKind::Lang as u8
                || kind == LiteralKind::Other as u8))
    {
        value_str(&v).map(String::into_boxed_str)
    } else {
        None
    };
    SortCell::Val { class, kind, key, v }
}
```

### crates/sparq-engine/src/exec.rs:11410-11550 — cmp_sort_cells
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn cmp_sort_cells(graph: &Graph, local: &LocalVocab, a: &SortCell, c: &SortCell) -> Ordering {
    match (a, c) {
        (SortCell::Temp { t: ta, .. }, SortCell::Temp { t: tb, .. }) => {
            // [FABLE-5] sq-wjl8i KIND-FIRST + [SONNET-4.6] sq-2k5py: both now live in the
            // shared `Temporal::cmp_t_total` — a dateTime never value-compares against a
            // date (`LiteralKind::DateTime < Date`), and within one kind the timeline order
            // is TOTAL (instant, then timezone presence). The former lexical fallback for
            // the indeterminate window is gone: it mixed timeline-decided and
            // lexical-decided pairs inside one kind, which is intransitive. Ill-formed
            // temporals never enter the cache, so both cells here are well-formed.
            Temporal::cmp_t_total(*ta, *tb)
        }
        // Two numeric graph terms: f64 fast compare, with the EXACT tie recheck below.
        (SortCell::Num { f: fa, id: ia }, SortCell::Num { f: fb, id: ib }) => {
            cmp_sort_num(graph, *fa, *ia, *fb, *ib)
        }
        (SortCell::Temp { id, .. }, SortCell::Val { v, .. }) => {
            compare_values(&sort_cell_term(graph, local, *id), v).unwrap_or(Ordering::Equal)
        }
        (SortCell::Val { v, .. }, SortCell::Temp { id, .. }) => {
            compare_values(v, &sort_cell_term(graph, local, *id)).unwrap_or(Ordering::Equal)
        }
        // A numeric cell against a temporal or a general Value (a MIXED-type column, cold):
        // reconstruct the pre-change `Val(Num::Double)` representation and defer to the shared
        // `compare_values`, so the mixed-column order stays byte-identical to pre-change — only
        // the same-numeric-column (`Num`, `Num`) arm above adds the exact f64-tie recheck.
        (SortCell::Num { f, .. }, SortCell::Temp { id, .. }) => {
            compare_values(&Value::Num(Num::Double(*f)), &sort_cell_term(graph, local, *id)).unwrap_or(Ordering::Equal)
        }
        (SortCell::Temp { id, .. }, SortCell::Num { f, .. }) => {
            compare_values(&sort_cell_term(graph, local, *id), &Value::Num(Num::Double(*f))).unwrap_or(Ordering::Equal)
        }
        (SortCell::Num { f, .. }, SortCell::Val { v, .. }) => {
            compare_values(&Value::Num(Num::Double(*f)), v).unwrap_or(Ordering::Equal)
        }
        (SortCell::Val { v, .. }, SortCell::Num { f, .. }) => {
            compare_values(v, &Value::Num(Num::Double(*f))).unwrap_or(Ordering::Equal)
        }
        // Two IRI sort cells: direct string comparison — no allocation, no term_class
        // dispatch. [SONNET-4.6] sq-7d3dj.30.2
        (SortCell::Iri(a), SortCell::Iri(b)) => a.as_ref().cmp(b.as_ref()),
        // IRI against a Num/Temp cell (a MIXED-type column): IRIs are term-class Iri
        // (rank 2 in the SPARQL total order), literals are Literal (rank 3) — so IRIs
        // always sort before any literal, regardless of numeric or temporal subtype.
        (SortCell::Iri(_), SortCell::Num { .. }) => Ordering::Less,
        (SortCell::Num { .. }, SortCell::Iri(_)) => Ordering::Greater,
        (SortCell::Iri(_), SortCell::Temp { .. }) => Ordering::Less,
        (SortCell::Temp { .. }, SortCell::Iri(_)) => Ordering::Greater,
        // IRI against a generic Val: dispatch on the Val variant to determine rank.
        // Val can hold Unbound (rank 0), BlankNode (rank 1), NamedNode/IRI (rank 2),
        // Literal/Num/Bool (rank 3), Triple (rank 4).
        (SortCell::Iri(a), SortCell::Val { v, .. }) => match v {
            Value::Unbound | Value::Error => Ordering::Greater, // IRI after unbound
            Value::Term(Term::BlankNode(_)) => Ordering::Greater, // IRI after blank node
            Value::Term(Term::NamedNode(n)) => a.as_ref().cmp(n.as_str()), // same class
            _ => Ordering::Less, // IRI before literals and triple terms
        },
        (SortCell::Val { v, .. }, SortCell::Iri(a)) => match v {
            Value::Unbound | Value::Error => Ordering::Less,
            Value::Term(Term::BlankNode(_)) => Ordering::Less,
            Value::Term(Term::NamedNode(n)) => n.as_str().cmp(a.as_ref()),
            _ => Ordering::Greater,
        },
        // [FABLE-5] sq-7d3dj.30.21 — LAZY plain-string-literal cells. `StrId(id)` denotes a
        // plain `xsd:string` literal: TermClass::Literal (rank 3), LiteralKind::String (rank
        // 4), value = the literal's `value` slice (via `Dict::term_parts`). Every arm below
        // reproduces the order the eager `Val{class:3, kind:4, key:Some(value)}` cell would
        // give BYTE-IDENTICALLY, with no per-row `value_str` allocation. The homogeneous
        // `(StrId, StrId)` arm is the hot q11 comparator; the cross-type arms cover a mixed key
        // column (an ORDER BY expression that yields plain strings on some rows and other terms
        // on others).
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(a), SortCell::StrId(b)) => {
            // Same class (Literal) + same kind (String) → lexical value-byte order, exactly
            // the eager `(Some(la), Some(lb)) => la.cmp(lb)` `Val` arm.
            str_id_value(graph, *a).cmp(str_id_value(graph, *b))
        }
        // String literal (Literal-class rank 3) vs an IRI (rank 2): the literal sorts AFTER the
        // IRI — regardless of value.
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Iri(_)) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Iri(_), SortCell::StrId(_)) => Ordering::Less,
        // String literal (kind String rank 4) vs a numeric (kind Numeric rank 0) or temporal
        // (kind DateTime rank 2 / Date rank 3) — all Literal-class, and String's kind rank is
        // strictly greater than every one of those, so the string sorts AFTER, by kind rank.
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Num { .. }) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Num { .. }, SortCell::StrId(_)) => Ordering::Less,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Temp { .. }) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Temp { .. }, SortCell::StrId(_)) => Ordering::Less,
        // String literal vs a generic Val — mirror the eager `Val` dispatch by the OTHER cell's
        // precomputed class/kind ranks against the fixed (Literal=3, String=4) ranks of a plain
        // string, then compare value bytes for the same-class-same-kind (String) case.
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(a), SortCell::Val { class: cb, kind: kb, key: kb_key, v: cv }) => {
            cmp_strid_val(graph, *a, *cb, *kb, kb_key.as_deref(), cv)
        }
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Val { class: ca, kind: ka, key: ka_key, v: av }, SortCell::StrId(b)) => {
            cmp_strid_val(graph, *b, *ca, *ka, ka_key.as_deref(), av).reverse()
        }
        // Two generic key values: compare the PRECOMPUTED class rank, then (within the
        // literal class) the PRECOMPUTED kind rank — both integer compares, no per-comparison
        // `lit_kind` / `is_numeric_dt` re-derivation. Only when the ranks tie (same class,
        // and same literal kind if literal) do we fall through to `compare_values` for the
        // within-kind value order — which, given equal class+kind, reaches EXACTLY the same
        // within-kind arm `compare_terms` would (the numeric exact-tie recheck, the
        // timeline / strict compare, or the lexical fallback). So the total order is
        // byte-identical to `compare_values(av, cv)`, just with the cross-class / cross-kind
        // dispatch hoisted to construction time. [FABLE-5] sq-7d3dj.30.12
        (
            SortCell::Val { class: ca, kind: ka, key: ka_key, v: av },
            SortCell::Val { class: cb, kind: kb, key: kb_key, v: cv },
        ) => match ca.cmp(cb) {
            Ordering::Equal => {
                // Same class. Within the literal class, rank by kind first; other classes
                // (blank / IRI / triple / error-or-unbound) have no kind split.
                if *ca == TermClass::Literal as u8 && ka != kb {
                    return ka.cmp(kb);
                }
                // Same class (and same literal kind, if literal). When BOTH cells carry a
                // precomputed lexical key — the `String` / `Lang` / `Other` literal kinds
                // and the `Blank` class — the within-kind order IS the `value_str` (lexical
                // `value()`) order, so compare the precomputed slices directly: no
                // per-comparison `value_str` allocation, no re-dispatch. This is byte-
                // identical to `compare_values(av, cv)` for those kinds (the differential
                // test proves it). Anything else (numeric exact-tie, boolean, temporal,
                // triple, error/unbound) keeps `None` and defers to the shared comparator.
                match (ka_key, kb_key) {
                    (Some(la), Some(lb)) => la.cmp(lb),
                    _ => compare_values(av, cv).unwrap_or(Ordering::Equal),
                }
            }
            ord => ord,
        },
    }
}
```

### crates/sparq-engine/src/exec.rs:11563-11591 — cmp_sort_num
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn cmp_sort_num(graph: &Graph, fa: f64, ia: Id, fb: f64, ib: Id) -> Ordering {
    match fa.partial_cmp(&fb) {
        Some(Ordering::Equal) => {
            // Same dictionary id ⇒ identical value: skip the two lexical allocations.
            // In a numeric ORDER BY column with repeated values this is the common f64
            // tie (a sort compares equal-id rows often). [OPUS-4.8] sq-rikm7
            if ia == ib {
                return Ordering::Equal;
            }
            match (graph.exact_numeric_lexical(ia), graph.exact_numeric_lexical(ib)) {
                (Some(la), Some(lb)) => cmp_decimal_str(&la, &lb).unwrap_or(Ordering::Equal),
                (Some(la), None) => cmp_exact_lex_f64(&la, fb),
                (None, Some(lb)) => cmp_exact_lex_f64(&lb, fa).reverse(),
                // Both float/double: the value IS the tied f64 — a true tie.
                (None, None) => Ordering::Equal,
            }
        }
        Some(o) => o,
        // NaN. Unreachable from ORDER BY sort cells today (the numerics cache uses NaN
        // as its "not numeric" sentinel, so a NaN literal never becomes a `SortCell::Num`
        // and routes through `compare_values` instead) — kept in lock-step with the
        // total order's NaN-first rule so the fast path can never diverge. [FABLE-5] sq-wjl8i
        None => match (fa.is_nan(), fb.is_nan()) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        },
    }
}
```

### crates/sparq-engine/src/exec.rs:11599-11610 — cmp_exact_lex_f64
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn cmp_exact_lex_f64(lex: &str, f: f64) -> Ordering {
    if f == f64::INFINITY {
        return Ordering::Less;
    }
    if f == f64::NEG_INFINITY {
        return Ordering::Greater;
    }
    match f64_exact_decimal(f) {
        Some(exp) => cmp_decimal_str(lex, &exp).unwrap_or(Ordering::Equal),
        None => Ordering::Equal, // NaN: handled by the caller's NaN rule
    }
}
```

### crates/sparq-engine/src/exec.rs:11619-11621 — sort_cell_term
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn sort_cell_term(graph: &Graph, local: &LocalVocab, id: Id) -> Value {
    Value::Term(term_of(graph, local, id).expect("sort key id resolves"))
}
```

### crates/sparq-engine/src/exec.rs:11630-11632 — str_id_value
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn str_id_value(graph: &Graph, id: Id) -> &str {
    graph.dict.plain_string_value(id).unwrap_or("")
}
```

### crates/sparq-engine/src/exec.rs:11645-11667 — cmp_strid_val
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn cmp_strid_val(graph: &Graph, a: Id, vclass: u8, vkind: u8, vkey: Option<&str>, v: &Value) -> Ordering {
    let str_class = TermClass::Literal as u8;
    let str_kind = LiteralKind::String as u8;
    match str_class.cmp(&vclass) {
        Ordering::Equal => {
            // Both Literal-class. Rank by kind first (String vs the Val's kind).
            if str_kind != vkind {
                return str_kind.cmp(&vkind);
            }
            // Both Literal-class String-kind: value-byte order. A String-kind `Val` always
            // carries a precomputed lexical key; fall back to `value_str` only if (unexpectedly)
            // absent, so the comparison never silently mis-orders.
            match vkey {
                Some(lb) => str_id_value(graph, a).cmp(lb),
                None => match value_str(v) {
                    Some(lb) => str_id_value(graph, a).cmp(lb.as_str()),
                    None => Ordering::Equal,
                },
            }
        }
        ord => ord,
    }
}
```

### crates/sparq-engine/src/exec.rs:14089-14091 — compare_values
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn compare_values(x: &Value, y: &Value) -> Option<Ordering> {
    compare_terms(x, y)
}
```

### crates/sparq-engine/src/lib.rs:761-764 — with_view
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn with_view<T>(v: &DatasetView, f: impl FnOnce() -> T) -> T {
    let _guard = exec::view::install(v);
    f()
}
```

### crates/sparq-engine/src/lib.rs:768-770 — query_view
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query_view(v: &DatasetView, sparql: &str) -> Result<QueryResult, String> {
    query_view_with_budget(v, sparql, &QueryBudget::unlimited())
}
```

### crates/sparq-engine/src/lib.rs:773-775 — query_view_with_budget
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query_view_with_budget(v: &DatasetView, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    with_view(v, || query_with_budget(v.base, sparql, budget))
}
```

### crates/sparq-engine/src/lib.rs:812-814 — active_dataset
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn active_dataset(graph: &Graph, q: &Query) -> Option<Graph> {
    q.dataset().map(|ds| dataset::build_active(graph, ds))
}
```

### crates/sparq-engine/src/lib.rs:822-824 — view_scope
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn view_scope(active: &Option<Graph>) -> Option<exec::view::Guard> {
    active.is_some().then(exec::view::suspend_all)
}
```

### crates/sparq-engine/src/lib.rs:1012-1014 — query
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query(graph: &Graph, sparql: &str) -> Result<QueryResult, String> {
    query_with_budget(graph, sparql, &QueryBudget::unlimited())
}
```

### crates/sparq-engine/src/lib.rs:1017-1019 — query_with_budget
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn query_with_budget(graph: &Graph, sparql: &str, budget: &QueryBudget) -> Result<QueryResult, String> {
    query_prepared_with_budget(graph, &PreparedQuery::parse(sparql)?, budget)
}
```

### crates/sparq-engine/src/lib.rs:1027-1048 — query_prepared_with_budget
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

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
    let _guard = exec::budget::install(budget);
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
}
```

### crates/sparq-engine/src/lib.rs:272-303 — QueryBudget
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub struct QueryBudget {
    /// Wall-clock deadline. Native only: `std::time::Instant` is unusable on
    /// `wasm32-unknown-unknown` (it panics), so the field does not exist there —
    /// the row budget below stays fully portable.
    #[cfg(not(target_arch = "wasm32"))]
    pub deadline: Option<std::time::Instant>,
    /// Upper bound on the rows of any materialised (intermediate or final) result.
    /// This is a *working-set* bound: a query whose intermediate result exceeds it
    /// is refused even if a later operator (e.g. LIMIT) would have shrunk it.
    pub max_rows: Option<usize>,
    /// [OPUS-4.8] (sq-s5is) Upper bound, in BYTES, on the estimated working-set size of
    /// any materialised (intermediate or final) result — the byte-accounted twin of
    /// `max_rows`. Where `max_rows` counts ROWS and so misses a query with FEW but very
    /// WIDE rows (many projected variables, or huge computed string literals), this bounds
    /// the estimated heap footprint of the id-level working set: `rows × width ×
    /// size_of::<Id>()` for each materialised intermediate, PLUS the bytes of any
    /// query-computed terms (BIND / aggregate / CONSTRUCT scratch) interned into the
    /// per-query local vocabulary. Checked cooperatively at the same coarse sites as
    /// `max_rows` (operator entry / per outer-loop iteration); a query whose estimate
    /// crosses it aborts with `"query budget exceeded (max-bytes)"`. The estimate is a
    /// portable LOWER bound on real heap (it ignores allocator overhead and `SmallVec`
    /// inline storage), so it is conservative in the SAME direction `max_rows` is — a blunt
    /// anti-OOM ceiling, not an exact RSS quota. `None` (the default) disables it; it
    /// composes with `max_rows` (whichever trips first aborts).
    pub max_bytes: Option<usize>,
    /// Cross-thread cooperative cancellation flag. The executor observes `true`
    /// at the same coarse polling sites as the other limits and aborts with
    /// `"query budget exceeded (cancelled)"`; cancellation is therefore prompt at
    /// the next poll, not immediate. The flag controls evaluation only and does
    /// not publish shared query data.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
```

### crates/sparq-engine/src/lib.rs:723-730 — DatasetView
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub struct DatasetView<'g> {
    /// The full store the view restricts.
    pub base: &'g Graph,
    /// The visible named-graph names.
    pub named: std::sync::Arc<FxHashSet<Term>>,
    /// What the view exposes as the default graph.
    pub default: DefaultGraphMode,
}
```

### crates/sparq-engine/src/lib.rs:737-743 — DefaultGraphMode
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub enum DefaultGraphMode {
    /// The store's own default graph (today's behaviour).
    #[default]
    StoreDefault,
    /// An empty default graph (e.g. data lives only in named graphs).
    Empty,
}
```

### crates/sparq-engine/src/lib.rs:305-322 — QueryBudget methods
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
impl QueryBudget {
    /// The do-nothing budget every non-budgeted entry point uses.
    pub fn unlimited() -> Self {
        Self::default()
    }

    /// Creates an otherwise unlimited budget cancelled by `flag`.
    pub fn cancelled_by(flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self::unlimited().with_cancel(flag)
    }

    /// Adds a cross-thread cooperative cancellation flag to this budget.
    #[must_use]
    pub fn with_cancel(mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }
}
```

### crates/sparq-engine/src/dataset.rs:72-96 — build_active
Blob 13ef5cb15c0d0886d310a7523b7f3bb329186aa7 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub(crate) fn build_active(graph: &Graph, ds: &QueryDataset) -> Graph {
    let visible = |n: &NamedNode| crate::exec::view::allows(&Term::NamedNode(n.clone()));
    let mut default = TripleSet::default();
    for n in &ds.default {
        if !visible(n) {
            continue; // view: non-visible ≡ absent
        }
        if let Some(g) = find_named(graph, n) {
            default.extend(decode_triples(g));
        }
    }
    let mut out = build(&default);
    for n in ds.named.as_deref().unwrap_or_default() {
        let name = Term::NamedNode(n.clone());
        if out.named.iter().any(|(g, _)| *g == name) {
            continue; // a repeated FROM NAMED still names ONE graph
        }
        let g = match find_named(graph, n).filter(|_| visible(n)) {
            Some(g) => build(&decode_triples(g)),
            None => empty_graph(),
        };
        out.named.push((name, g));
    }
    out
}
```

### crates/sparq-engine/src/explain.rs:68-70 — explain_analyze
Blob 2f6952ec3bc67f8d2d84e32466ff1d8e421acdd7 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn explain_analyze(graph: &Graph, sparql: &str) -> Result<String, String> {
    explain_analyze_with_budget(graph, sparql, &QueryBudget::unlimited())
}
```

### crates/sparq-engine/src/explain.rs:73-114 — explain_analyze_with_budget
Blob 2f6952ec3bc67f8d2d84e32466ff1d8e421acdd7 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

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
    let _bguard = exec::budget::install(budget);
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
}
```

### crates/sparq-core/src/store.rs:1-56 — Perm and BUILT compile-time inventory
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
//! Triple store: the six sorted permutation indexes over dictionary-encoded
//! triples (Hexastore / RDF-3X / QLever design).
//!
//! Storing all six orderings (SPO SOP PSO POS OSP OPS) means every triple
//! pattern is answered by a single contiguous range (binary search on the
//! bound prefix), and the scan output is sorted by the remaining positions —
//! which is exactly what merge joins need. M1 holds each permutation as a
//! sorted `Vec<[Id; 3]>`; later milestones replace these with block-compressed,
//! optionally memory-mapped columns.

use crate::dict::Id;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// The six permutations. Each names the order of (subject, predicate, object)
/// columns as stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perm {
    Spo,
    Sop,
    Pso,
    Pos,
    Osp,
    Ops,
}

/// The permutations actually built and searched. The full six give every triple
/// pattern a sorted scan in the order any merge join wants. The `compact-index` set
/// {SPO, POS, OSP} still answers EVERY triple pattern from one index (SPO→S*/SP*,
/// POS→P*/PO*, OSP→O*/OS*) at half the memory, at the cost of some merge joins (and
/// some lazy-count fast paths) falling back to hashing / sorting.
// Compact set on wasm ALWAYS (memory-bound target), or on native opt-in via the
// `compact-index` feature (for testing). Keyed on `target_arch` — NOT just a feature —
// so the wasm choice does not leak to the native build via Cargo feature unification.
#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Pos, Perm::Osp];

impl Perm {
    pub const ALL: [Perm; 6] = [Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];

    /// The column indices (into a canonical s,p,o triple) in this permutation's
    /// sort order. e.g. POS -> 1,2,0.
    #[inline]
    pub fn order(self) -> [usize; 3] {
        match self {
            Perm::Spo => [0, 1, 2],
            Perm::Sop => [0, 2, 1],
            Perm::Pso => [1, 0, 2],
            Perm::Pos => [1, 2, 0],
            Perm::Osp => [2, 0, 1],
            Perm::Ops => [2, 1, 0],
        }
    }
}
```

### crates/sparq-core/src/store.rs:977-994 — choose
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn choose(pattern: &Pattern) -> (Perm, usize) {
        // Prefer an order where every bound position precedes every unbound one.
        let bound = |i: usize| pattern[i].is_some();
        for &perm in BUILT {
            let order = perm.order();
            // count leading bound columns
            let mut lead = 0;
            while lead < 3 && bound(order[lead]) {
                lead += 1;
            }
            // valid if all bound positions are within the leading prefix
            let total_bound = (0..3).filter(|&i| bound(i)).count();
            if lead == total_bound {
                return (perm, lead);
            }
        }
        (Perm::Spo, 0)
    }
```

### crates/sparq-core/src/store.rs:1000-1016 — choose_sorted
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn choose_sorted(pattern: &Pattern, sort_col: usize) -> (Perm, usize) {
        let bound = |i: usize| pattern[i].is_some();
        let total_bound = (0..3).filter(|&i| bound(i)).count();
        // Prefer: bound positions form the leading prefix AND column `sort_col`
        // is the first column after the prefix.
        for &perm in BUILT {
            let order = perm.order();
            let mut lead = 0;
            while lead < 3 && bound(order[lead]) {
                lead += 1;
            }
            if lead == total_bound && lead < 3 && order[lead] == sort_col {
                return (perm, lead);
            }
        }
        Self::choose(pattern)
    }
```

### crates/sparq-core/src/store.rs:1027-1030 — scan_sorted
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn scan_sorted(&self, pattern: &Pattern, sort_col: usize) -> Scan<'_> {
        let (perm, lead) = Self::choose_sorted(pattern, sort_col);
        self.scan_with(pattern, perm, lead)
    }
```

### crates/sparq-core/src/store.rs:1060-1070 — bounds
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn bounds(pattern: &Pattern, perm: Perm, lead: usize) -> ([Id; 3], [Id; 3]) {
        let order = perm.order();
        let mut lo = [Id::MIN; 3];
        let mut hi = [Id::MAX; 3];
        for k in 0..lead {
            let v = pattern[order[k]].unwrap();
            lo[k] = v;
            hi[k] = v;
        }
        (lo, hi)
    }
```

### crates/sparq-core/src/store.rs:1072-1097 — scan_with
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn scan_with(&self, pattern: &Pattern, perm: Perm, lead: usize) -> Scan<'_> {
        let (lo, hi) = Self::bounds(pattern, perm, lead);
        let base = self.perms[perm as usize].rows_in(lo, hi);
        // The single overlay branch on the scan hot path: with no pending updates the
        // base range is returned untouched (borrowed, zero copies); with an overlay the
        // deleted triples are filtered out and the inserted ones merge-interleaved, so
        // the rows keep the permutation's sort order (merge joins stay valid).
        //
        // ZERO-COPY FAST PATH (sq-7d3dj.3) [OPUS-4.8]: even WITH an overlay, most ranges a small
        // overlay does not touch. `count_correction` tells us exactly how many
        // `added`/`deleted` triples fall in this range; when it is `(0, 0)` the
        // overlay contributes nothing here — no `added` row projects into `[lo, hi]` (so
        // nothing is interleaved) and no in-range base row is deleted (so nothing is
        // dropped) — hence `merge` would reproduce `base` verbatim, rows AND sort order.
        // We therefore return the BORROWED base slice directly, restoring allocation-free
        // scans for every untouched range (the read-mostly mutated-server common case)
        // instead of paying the owned merge path — which copies the whole base range into a
        // fresh `Vec` and merge-interleaves the (separately, already perm-sorted) in-range
        // `added` rows. It never re-sorts the range; the cost is the copy plus the interleave.
        let rows = match &self.overlay {
            None => base,
            Some(ov) if ov.count_correction(perm, lo, hi) == (0, 0) => base,
            Some(ov) => std::borrow::Cow::Owned(ov.merge(&base, perm, lo, hi)),
        };
        Scan { rows, perm }
    }
```

### crates/sparq-core/src/store.rs:117-127 — rows_in
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn rows_in(&self, lo: [Id; 3], hi: [Id; 3]) -> std::borrow::Cow<'_, [[Id; 3]]> {
        match self {
            PermData::Compressed(c) => std::borrow::Cow::Owned(c.range(lo, hi)),
            _ => {
                let rows = self.as_slice();
                let s = lower_bound(rows, &lo);
                let e = upper_bound(rows, &hi);
                std::borrow::Cow::Borrowed(&rows[s..e])
            }
        }
    }
```

### crates/sparq-core/src/store.rs:229-253 — merge
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn merge(&self, base: &[[Id; 3]], perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> Vec<[Id; 3]> {
        let add = self.added_rows(perm, lo, hi);
        let order = perm.order();
        let mut out = Vec::with_capacity(base.len() + add.len());
        let mut ai = 0;
        let check_deleted = !self.deleted.is_empty();
        for &row in base {
            if check_deleted {
                let mut spo = [0; 3];
                spo[order[0]] = row[0];
                spo[order[1]] = row[1];
                spo[order[2]] = row[2];
                if self.deleted.contains(&spo) {
                    continue;
                }
            }
            while ai < add.len() && add[ai] < row {
                out.push(add[ai]);
                ai += 1;
            }
            out.push(row);
        }
        out.extend_from_slice(&add[ai..]);
        out
    }
```

### crates/sparq-core/src/store.rs:259-271 — count_correction
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    fn count_correction(&self, perm: Perm, lo: [Id; 3], hi: [Id; 3]) -> (usize, usize) {
        let order = perm.order();
        let add = self.added_rows(perm, lo, hi).len();
        let del = self
            .deleted
            .iter()
            .filter(|t| {
                let r = [t[order[0]], t[order[1]], t[order[2]]];
                r >= lo && r <= hi
            })
            .count();
        (add, del)
    }
```

### crates/sparq-core/src/store.rs:1127-1134 — to_spo
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn to_spo(&self, row: &[Id; 3]) -> [Id; 3] {
        let order = self.perm.order();
        let mut out = [0; 3];
        out[order[0]] = row[0];
        out[order[1]] = row[1];
        out[order[2]] = row[2];
        out
    }
```

### crates/sparq-core/src/store.rs:1119-1122 — Scan
Blob e4b19e441dcd32eac5e3fde63c38dabeb4493f4c at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub struct Scan<'a> {
    pub rows: std::borrow::Cow<'a, [[Id; 3]]>,
    pub perm: Perm,
}
```

### crates/sparq-core/src/dict.rs:1-86 — Inline integer encoding contract
Blob 658390a3634ed7788e875c42727633f32f1e9c57 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
//! Term dictionary: bijection between RDF terms and dense `u32` ids.
//!
//! Dictionary encoding is the foundation of every fast triplestore (RDF-3X,
//! QLever, RDFox): triples are stored and joined as fixed-width integers, and the
//! (large, string-heavy) terms live once in the dictionary. Terms are stored COMPACT
//! and prefix-factored (IRIs share a namespace table); the interner is single-storage
//! (the hash table holds only ids) and content-addressed by a hash reproducible from
//! either an `oxrdf::Term` or the parsed byte components — so a byte-level parser can
//! intern straight from slices with no intermediate `Term`.

use hashbrown::HashTable;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashMap;
use std::hash::Hasher;

/// A dense term id. `u32` (≤ 4.29 B distinct terms) keeps index entries small
/// and cache-friendly; the id space is widened to `u64` only if a dataset needs
/// it. Id 0 is reserved as a sentinel ("no such term").
pub type Id = u32;

pub const NO_ID: Id = 0;

/// Tagged-ValueId base: ids in `[INLINE_BASE, INLINE_BASE + 2^30)` encode an
/// `xsd:integer` whose value is `id - INLINE_BASE` *inline* — no dictionary entry,
/// no string parse. Numeric FILTER / comparison / ORDER BY read the value straight
/// from the id (QLever's value-id idea, kept in `u32` so the index stays compact).
/// Inline integers also sort by value in the permutations (enabling range pruning).
///
/// The `u32` id space is partitioned: dictionary ids `[1, INLINE_BASE)` (≈2.1 billion
/// distinct terms — enough for e.g. full-Wikidata's term count without widening to `u64`,
/// which would double the index), inline integers `[INLINE_BASE, INLINE_BASE + 2^30)`, and
/// the engine's local-vocab ids `[INLINE_BASE + 2^30, 2^32)`. `0` is `NO_ID`.
pub const INLINE_BASE: Id = 1 << 31;
/// The largest value encodable inline (the inline range stays 2^30 wide; bigger integers
/// fall back to the dictionary).
const INLINE_MAX: u32 = (1 << 30) - 1;

/// [OPUS-4.8] (review 1409) On-disk format marker for the mmap dictionary (`dict-meta.bin`).
/// `INLINE_BASE` partitions the `u32` id space, so persisted RAW ids only mean what they say
/// under the SAME partition the file was written with. A file written before `INLINE_BASE`
/// moved (from `1 << 30` to `1 << 31`) encodes inline integers in `[1<<30, 1<<31)`, which the
/// current code would misread as dictionary ids — silent numeric corruption / panics. The
/// header records the partition so `open_mmap` can REJECT a mismatched/legacy store with a
/// clear rebuild-required error instead of silently misinterpreting its ids.
///
/// `dict-meta.bin` previously began with `prefixes.len() as u32` (a small count). This magic
/// is chosen to be distinguishable from any plausible legacy prefix count, so the reader can
/// detect header-less legacy files. ("DMV1" — Dict Meta, V1; little-endian.)
// clippy/dead_code: the on-disk meta header is read/written only by the `mmap`/`dict-spill`
// persistence paths, which are cfg'd out of the default feature set.
#[allow(dead_code)]
pub(crate) const DICT_META_MAGIC: u32 = 0x31_56_4D_44; // b"DMV1" little-endian
/// Bump when the on-disk meta layout changes incompatibly.
#[allow(dead_code)]
pub(crate) const DICT_META_VERSION: u32 = 1;

/// If a literal `value`/`datatype` is a canonical non-negative `xsd:integer` in
/// range, its inline id. Only the canonical lexical form (no leading zeros / sign)
/// inlines, so `"030"^^integer` stays a distinct dictionary term.
#[inline]
fn try_inline_lit(value: &str, datatype: &str) -> Option<Id> {
    if datatype == xsd::INTEGER.as_str() {
        if let Ok(v) = value.parse::<u32>() {
            if v <= INLINE_MAX && v.to_string() == value {
                return Some(INLINE_BASE + v);
            }
        }
    }
    None
}

/// If `term` is a canonical non-negative `xsd:integer` in range, its inline id.
fn try_inline(term: &Term) -> Option<Id> {
    match term {
        Term::Literal(l) => try_inline_lit(l.value(), l.datatype().as_str()),
        _ => None,
    }
}

/// Whether an id encodes an inline integer value. (`INLINE_BASE << 1` would overflow `u32`,
/// so the upper bound is expressed via the inline width.)
#[inline]
pub fn is_inline(id: Id) -> bool {
    id >= INLINE_BASE && id - INLINE_BASE <= INLINE_MAX
}
```

### crates/sparq-core/src/lib.rs:2885-2903 — fork
Blob ab2cf013677d4d82c778dd64723ec825c214c3b0 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn fork(&self) -> Graph {
        Graph {
            dict: self.dict.fork(),
            store: self.store.fork(),
            numerics: self.numerics.fork(),
            temporals: self.temporals.fork(),
            // sq-lr2ii: the fork shares the same values; recompute the guard lazily.
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named.iter().map(|(name, g)| (name.clone(), g.fork())).collect(),
            // A fork is a fresh logical copy; rebuild the prefix index lazily on first use.
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
            // [OPUS-4.8] (sq-ycle) A fork/snapshot is a logically-independent in-memory copy with
            // NO directory association — no WAL and no redo journal (like `wal: None`).
            #[cfg(feature = "mmap")]
            txn: None,
        }
    }
```

### crates/sparq-core/src/lib.rs:2931-2935 — pending_delta_len
Blob ab2cf013677d4d82c778dd64723ec825c214c3b0 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn pending_delta_len(&self) -> usize {
        self.store.overlay_len()
            + self.dict.appended_len()
            + self.named.iter().map(|(_, g)| g.pending_delta_len()).sum::<usize>()
    }
```

### crates/sparq-core/src/lib.rs:3406-3443 — compact
Blob ab2cf013677d4d82c778dd64723ec825c214c3b0 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
    pub fn compact(&mut self) -> Result<(), String> {
        #[cfg(feature = "mmap")]
        let dir = self.wal.as_ref().map(|w| w.dir.clone());
        // Fold the structural-fork layers first (ids unchanged throughout): named
        // graphs recursively, then this graph's dictionary extension into a fresh
        // frozen base (so the NEXT fork is O(1) again) and the forked caches flat.
        // No-ops on a never-forked graph.
        for (_, g) in &mut self.named {
            g.compact()?;
        }
        if self.dict.is_forked() {
            self.dict = self.dict.compacted();
            let n = self.dict.len();
            let numerics = std::mem::replace(&mut self.numerics, NumData::Sparse(rustc_hash::FxHashMap::default()));
            self.numerics = numerics.fold(n);
            let temporals = std::mem::replace(&mut self.temporals, TempData::Sparse(rustc_hash::FxHashMap::default()));
            self.temporals = temporals.fold(n);
        }
        if self.store.has_overlay() {
            let triples: Vec<[Id; 3]> = {
                let scan = self.store.scan(&[None, None, None]);
                scan.rows.iter().map(|r| scan.to_spo(r)).collect()
            };
            self.store = TripleStore::from_triples(triples);
        } else {
            // Nothing pending: for a directory-backed graph just discard the (no-op) log.
            #[cfg(feature = "mmap")]
            if let Some(w) = &mut self.wal {
                w.truncate().map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        #[cfg(feature = "mmap")]
        if let Some(dir) = dir {
            self.persist_swap(&dir)?;
        }
        Ok(())
    }
```

### crates/sparq-engine/Cargo.toml:25-42 — engine features
Blob ac05dcf9c7b70a03a63310c03dfb9103a6f9a9ce at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```toml
# Forwards sparq-core's parallel index build. Default on for native; the wasm
# crate disables defaults so rayon is never pulled into the bundle.
# `regex` powers SPARQL REGEX/REPLACE; default-on for native, off for wasm (the wasm crate
# disables defaults) so the regex automata don't bloat the browser bundle.
# `digest` powers the SPARQL hash builtins (MD5/SHA1/SHA256/SHA384/SHA512); default-on
# for native, off for wasm (the wasm crate disables defaults) so the hash cores never
# enter the browser bundle.
default = ["parallel", "regex", "digest"]
parallel = ["dep:rayon", "sparq-core/parallel"]
regex = ["dep:regex"]
digest = ["dep:md-5", "dep:sha1", "dep:sha2"]
# Characteristic-set star-join cardinality estimation (Neumann & Moerkotte):
# `cs::CsTable` + `with_cs_table` make the greedy planner consult an injected CS
# table for star joins instead of the per-predicate independence model. OPT-IN and
# OFF by default (the wasm bundle and the default native build carry zero CS code);
# results are identical either way — only join order is affected.
cs-planner = []
# [FABLE-5] (sq-dzbg2) Persistent, explicitly-built planner statistics.  The
```

### crates/sparq-engine/Cargo.toml:416-428 — core regular dependency
Blob ac05dcf9c7b70a03a63310c03dfb9103a6f9a9ce at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```toml
required-features = ["vectorized"]

[dependencies]
# `version` alongside `path`: cargo uses the path locally and the version on crates.io
# (required to publish — crates.io strips `path`).
sparq-core = { path = "../sparq-core", version = "0.1.1", default-features = false }
# [OPUS-4.8] sq-ev41x + sq-hknqs + sq-vezew (epic sq-qonbz): the shared id-level evaluation
# substrate — the numeric value tower (`Num` / `Dec` / `as_numeric` + XSD lexical helpers,
# Phase 2), the four id-tuple join kernels (merge / hash / bind / leapfrog-trie behind the
# `JoinKeys` descriptor, Phase 3) AND the SPARQL term total order (`compare::compare_terms` —
# the engine's `compare_values`, over a generic `CompareTerm` trait, Phase 4) — moved out of
# `exec.rs` into the `sparq-substrate` leaf crate so the engine AND the reasoners can share one
# definition with NO `Box<dyn>` on the hot path (research/shared-eval-substrate.md, Option C).
```

### crates/sparq-engine/Cargo.toml:465-487 — dev dependency feature unification
Blob ac05dcf9c7b70a03a63310c03dfb9103a6f9a9ce at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```toml
sha2 = { version = "0.11", optional = true }

# UUID()/STRUUID() need an OS RNG; depending on uuid from the wasm build would drag
# getrandom into the browser dependency graph, so the functions are native-only.
# [OPUS-4.8] (sq-6vshe.4) The SERVICE feature's blocking HTTP client (ureq) moved with
# the `service` module to the `sparq-engine-service` sub-crate (seam A2), where it keeps
# the same `cfg(not(wasm32))` gating so no HTTP/TLS stack ever enters the browser bundle.
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
# The dict-consolidation differential test compares the serial, sharded-parallel and
# external (out-of-core) build paths — the tests need sparq-core's mmap feature even
# though the library itself doesn't. `dict-spill` adds the spilled-dictionary build's
# byte-identity differential (tests/dict_spill_differential.rs).
sparq-core = { path = "../sparq-core", version = "0.1.1", features = ["mmap", "parallel", "dict-spill"] }
rayon.workspace = true
# [OPUS-4.8] (sq-7d3dj.30.1) DEV-only re-listing of the ALREADY-present `spargebra` dep so
# tests/rewrite_pass.rs can parse a query to RAW `spargebra` algebra (the un-rewritten
# baseline) and feed it through `PreparedQuery::from` — the on-vs-off oracle for the
# `algebra-rewrite` pass. `spargebra` is already a normal dependency (above), so this adds
# ZERO new crate to the lock/graph; it only makes the type visible to the integration-test
# crate. Normal library code never uses this entry.
```

### .github/workflows/ci.yml:490-523 — workspace all-targets test archive caller
Blob 2291a011c578932b6c78cf1eae574bf4ad981002 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```yaml
      # [OPUS-4.8] sq-x4jy: cargo-nextest is the test runner — each test in its OWN
      # process (a stray abort can't take the whole binary down), a never-executed
      # BINARY reported as a DETERMINISTIC failure, and retries = 2 (now set by
      # .config/nextest.toml [profile.ci], selected on the run side — registry#563
      # item 2) lets a residual transient binary race self-heal. SHA-pinned install
      # action (a pin already vetted elsewhere in this workflow).
      - name: Install cargo-nextest
        uses: taiki-e/install-action@18b1216eba7f8039b0f8d131d5473787f0edce68 # v2.85.3
        with:
          tool: nextest
      # [OPUS-4.8] Build + ARCHIVE the whole workspace test set once. `--all-targets`
      # mirrors the old `cargo build --workspace --all-targets` (unit + integration +
      # bin test targets), so the archived set == what the un-sharded run built. The
      # .tar.zst is self-contained (binaries + the metadata nextest needs to run them
      # on another runner with `--archive-file`), so the shards never recompile.
      #
      # [OPUS-4.8] The archive runs `--features approx-ann,filtered-ann,vec-predicate`
      # (#363, LOAD-BEARING): sparq-vectors' heavy recall/over-fetch/vec-predicate tests are
      # MODULE-gated (`#![cfg(feature = ...)]`) and MUST run in this sharded lane — without
      # these the gated binaries compile EMPTY and the heavy-hnsw shard's exact filter matches
      # ZERO tests (nextest exit 4). These three are the ONLY opt-in features carried here.
      # GUARD (sq-vya1): the archive carries NO OTHER opt-in features (we deliberately avoid
      # `--all-features` — cross-crate conflicts + heavy/native/network deps would not even
      # resolve; see feature-matrix.yml's SCOPE). Any test behind a DEFAULT-OFF feature OTHER
      # than those three compiles EMPTY here and runs SILENTLY-zero in the shards — its coverage
      # is the JOB OF feature-matrix.yml (per-leg `cargo test -p <crate> --features <set>`,
      # gated by ci-summary). When you add/feature-gate a test: a sparq-vectors recall/vec test
      # rides these archive features; ANYTHING ELSE must be wired into a feature-matrix.yml leg
      # (read that file's GUARD block). Prove a suite is reached: `cargo nextest list -p <crate>
      # --features <set>` must SHOW its test names.
      - name: Build + archive test binaries (nextest archive)
        run: cargo nextest archive --workspace --all-targets --features approx-ann,filtered-ann,vec-predicate --archive-file nextest.tar.zst
      # [OPUS-4.8] Upload the archive immediately after the build, BEFORE doctests, so
      # the shards' input artifact is produced even if the doctest step later fails.
```
