### crates/sparq-engine/src/exec.rs:2332-2366 — eval_select
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub fn eval_select(graph: &Graph, pattern: &GraphPattern) -> Result<QueryResult, String> {
    let mut local = LocalVocab::default();
    let bindings = eval_modified(graph, &mut local, pattern)?;
    budget::check(bindings.rows.len())?;

    let out_vars: Vec<Variable> = bindings
        .vars
        .iter()
        .filter(|v| !v.as_str().starts_with(BNODE_VAR_PREFIX))
        .cloned()
        .collect();

    let col_of: Vec<Option<usize>> = out_vars.iter().map(|v| bindings.col(v)).collect();
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
    #[cfg(not(target_arch = "wasm32"))]
    let _query_now = query_now::scope();
    match p {
        GraphPattern::Project { inner, variables } => {
            let b = eval_modified(graph, local, inner)?;
            Ok(project_bindings(b, variables))
        }
        GraphPattern::Distinct { inner } => {
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Distinct);
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
            if let Some(len) = length {
                if let Some(cap) = start.checked_add(*len) {
                    if let Some(mut b) = try_capped(graph, local, inner, cap)? {
                        slice_bindings(&mut b, *start, *length);
                        return Ok(b);
                    }
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
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Group);
            if variables.is_empty() && aggregates.len() == 1 {
                if let (av, AggregateExpression::CountSolutions { distinct: false }) = (&aggregates[0].0, &aggregates[0].1) {
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
            if let Some(b) = try_topk_orderby_indexed(graph, local, ord_inner, expression, row_budget)? {
                return Ok(Some(b));
            }
            let mut b = eval_modified(graph, local, ord_inner)?;
            let use_topk = b.rows.len() > row_budget;
            order_bindings(graph, local, &mut b, expression, if use_topk { Some(row_budget) } else { None })?;
            Ok(Some(b))
        }
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
    let actual_sort = scan.perm.order().into_iter().find(|&c| id_pat[c].is_none());
    let sorted_by = actual_sort.and_then(|c| pos_vars[c].clone());

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

    let build_row = |row: &[Id; 3]| -> Option<Row> {
        let spo = scan.to_spo(row);
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

    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        let kept: Vec<[Id; 3]> = scan_rows
            .iter()
            .filter(|r| build_row(r).is_some())
            .map(|r| scan.to_spo(r))
            .collect();
        crate::zk::record_scan_ids(graph, id_pat, pos_vars, &kept, false);
    }

    #[cfg(feature = "parallel")]
    if limit.is_none() && scan_rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        let rows: Vec<Row> = scan_rows.par_iter().filter_map(build_row).collect();
        return Bindings { vars, rows, sorted_by };
    }

    let cap = limit.map_or(scan_rows.len(), |n| n.min(scan_rows.len()));
    let mut rows: Vec<Row> = Vec::with_capacity(budget::cap_alloc(cap));
    for (i, row) in scan_rows.iter().enumerate() {
        if i & 4095 == 0 && budget::exhausted(rows.len()) {
            break;
        }
        if let Some(out) = build_row(row) {
            rows.push(out);
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
        let _scope = view::enter_graph();
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
                None => {
                    let _scope = view::enter_graph(); // schema eval matches the present-graph path
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
            let mut out_vars: Option<Vec<Variable>> = None;
            let mut out_rows: Vec<Row> = Vec::new();
            let mut per_graph = |graph: &Graph, local: &mut LocalVocab, gname: &Term, sub: &Graph| -> Result<(), String> {
                let mut b = eval_translated(
                    graph,
                    local,
                    sub,
                    #[cfg(feature = "zk")]
                    gname,
                    inner,
                )?;
                let gid = value_to_id(graph, local, &Value::Term(gname.clone()));
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
    let compiled_order: Vec<(bool, CompiledExpr)> = exprs
        .iter()
        .map(|oe| match oe {
            OrderExpression::Asc(e) => (false, compile_expr(e, b)),
            OrderExpression::Desc(e) => (true, compile_expr(e, b)),
        })
        .collect();

    let cell_of = |row: &Row, e: &CompiledExpr| -> Result<SortCell, String> {
        if let CompiledExpr::Var(Some(c)) = e {
            let id = row[*c];
            if id != NO_ID && !is_local(id) {
                if let Some(n) = graph.numeric_value(id) {
                    return Ok(SortCell::Num { f: n, id });
                }
                if let Some(t) = graph.temporal_value(id) {
                    return Ok(SortCell::Temp { t, id });
                }
                #[cfg(feature = "topk-lazy-strkey")]
                if graph.dict.plain_string_value(id).is_some() {
                    return Ok(SortCell::StrId(id));
                }
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
    let key_of = |row: &Row| -> Result<Vec<(bool, SortCell)>, String> {
        let mut key = Vec::with_capacity(compiled_order.len());
        for (desc, ce) in &compiled_order {
            key.push((*desc, cell_of(row, ce)?));
        }
        Ok(key)
    };

    let n = b.rows.len();

    if let Some(k) = row_budget {
        if k == 0 {
            b.rows.clear();
            b.sorted_by = None;
            return Ok(());
        }
        if k < n {
            #[cfg(feature = "parallel")]
            let mut keyed: Vec<_> = if n >= PAR_THRESHOLD {
                use rayon::prelude::*;
                let fns = functions::snapshot();
                let vw = view::snapshot();
                let spx = spatial::snapshot();
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

            let cmp_total = |a: &(Vec<(bool, SortCell)>, usize), c: &(Vec<(bool, SortCell)>, usize)| {
                for ((desc, av), (_, cv)) in a.0.iter().zip(c.0.iter()) {
                    let ord = cmp_sort_cells(graph, local, av, cv);
                    let ord = if *desc { ord.reverse() } else { ord };
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                a.1.cmp(&c.1)
            };

            debug_assert!(k > 0 && k <= keyed.len());
            keyed.select_nth_unstable_by(k - 1, |a, c| cmp_total(a, c));

            keyed[..k].sort_by(|a, c| cmp_total(a, c));

            b.rows = keyed[..k].iter().map(|(_, i)| b.rows[*i].clone()).collect();
            b.sorted_by = None;
            return Ok(());
        }
    }

    #[cfg(feature = "parallel")]
    let mut keyed: Vec<(Vec<(bool, SortCell)>, Row)> = if n >= PAR_THRESHOLD {
        use rayon::prelude::*;
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot();
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

    #[derive(Clone, Copy)]
    struct CancelPtr(NonNull<AtomicBool>);

    unsafe impl Send for CancelPtr {}
    unsafe impl Sync for CancelPtr {}

    pub(crate) const BYTES_PER_ID: usize = std::mem::size_of::<Id>();

    #[derive(Clone, Copy)]
    pub(crate) struct Limits {
        on: bool,
        #[cfg(not(target_arch = "wasm32"))]
        deadline: Option<std::time::Instant>,
        max_rows: usize,
        max_bytes: usize,
        byte_width: usize,
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
        #[inline]
        fn bytes(&self, rows: usize) -> usize {
            rows.saturating_mul(self.byte_width).saturating_add(self.extra_bytes)
        }

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
                if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                    return Some("cancelled");
                }
            }
            None
        }

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

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    #[inline]
    pub(crate) fn snapshot() -> Limits {
        ACTIVE.with(|a| a.get())
    }

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

    #[cfg(not(target_arch = "wasm32"))]
    #[cfg_attr(not(feature = "service"), allow(dead_code))]
    #[inline]
    pub(crate) fn remaining_timeout() -> Option<std::time::Duration> {
        ACTIVE.with(|a| {
            let lim = a.get();
            lim.deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()))
        })
    }

    #[cfg(any(feature = "service", feature = "service-local"))]
    #[derive(Clone, Copy)]
    pub(crate) struct ByteSavepoint {
        extra_bytes: usize,
        exceeded: Option<&'static str>,
    }

    #[cfg(any(feature = "service", feature = "service-local"))]
    #[inline]
    pub(crate) fn byte_savepoint() -> ByteSavepoint {
        ByteSavepoint {
            extra_bytes: ACTIVE.with(|c| c.get().extra_bytes),
            exceeded: EXCEEDED.with(|e| e.get()),
        }
    }

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
            if unsafe { cancel.0.as_ref() }.load(Ordering::Relaxed) {
                EXCEEDED.with(|e| e.set(Some("cancelled")));
                return true;
            }
        }
        false
    }

    #[inline]
    pub(crate) fn check(rows: usize) -> Result<(), String> {
        if exhausted(rows) {
            let why = EXCEEDED.with(|e| e.get()).unwrap_or("timeout");
            return Err(format!("query budget exceeded ({why})"));
        }
        Ok(())
    }

    #[cfg_attr(not(feature = "vectorized"), allow(dead_code))]
    #[inline]
    pub(crate) fn active() -> bool {
        ACTIVE.with(|c| c.get().on)
    }

    #[inline]
    pub(crate) fn cap_alloc(cap: usize) -> usize {
        let a = ACTIVE.with(|c| c.get());
        if !a.on {
            return cap;
        }
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

    #[derive(Clone, Default)]
    pub(crate) struct State {
        named: Option<Arc<FxHashSet<Term>>>,
        default_empty: bool,
        suspended: bool,
    }

    thread_local! {
        static ACTIVE: RefCell<State> = RefCell::new(State::default());
    }

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

    pub(crate) fn suspend_all() -> Guard {
        Guard(ACTIVE.with(|a| std::mem::take(&mut *a.borrow_mut())))
    }

    pub(crate) struct GraphScope(bool);
    impl Drop for GraphScope {
        fn drop(&mut self) {
            ACTIVE.with(|a| a.borrow_mut().suspended = self.0);
        }
    }

    pub(crate) fn enter_graph() -> GraphScope {
        GraphScope(ACTIVE.with(|a| std::mem::replace(&mut a.borrow_mut().suspended, true)))
    }

    #[inline]
    pub(crate) fn allows(name: &Term) -> bool {
        ACTIVE.with(|a| a.borrow().named.as_ref().is_none_or(|s| s.contains(name)))
    }

    #[inline]
    pub(crate) fn default_is_empty() -> bool {
        ACTIVE.with(|a| {
            let s = a.borrow();
            s.default_empty && !s.suspended
        })
    }

    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn snapshot() -> Option<State> {
        ACTIVE.with(|a| {
            let s = a.borrow();
            (s.named.is_some() || s.default_empty).then(|| s.clone())
        })
    }

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
    Num { f: f64, id: Id },
    Iri(Box<str>),
    #[cfg(feature = "topk-lazy-strkey")]
    StrId(Id),
    Val { class: u8, kind: u8, key: Option<Box<str>>, v: Value },
}
```

### crates/sparq-engine/src/exec.rs:11374-11401 — sort_cell_val
Blob c0f63fb14243bc403092fb9146674f8dbb669c35 at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
fn sort_cell_val(v: Value) -> SortCell {
    let class = v.term_class() as u8;
    let kind = if class == TermClass::Literal as u8 { v.literal_kind() as u8 } else { 0 };
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
            Temporal::cmp_t_total(*ta, *tb)
        }
        (SortCell::Num { f: fa, id: ia }, SortCell::Num { f: fb, id: ib }) => {
            cmp_sort_num(graph, *fa, *ia, *fb, *ib)
        }
        (SortCell::Temp { id, .. }, SortCell::Val { v, .. }) => {
            compare_values(&sort_cell_term(graph, local, *id), v).unwrap_or(Ordering::Equal)
        }
        (SortCell::Val { v, .. }, SortCell::Temp { id, .. }) => {
            compare_values(v, &sort_cell_term(graph, local, *id)).unwrap_or(Ordering::Equal)
        }
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
        (SortCell::Iri(a), SortCell::Iri(b)) => a.as_ref().cmp(b.as_ref()),
        (SortCell::Iri(_), SortCell::Num { .. }) => Ordering::Less,
        (SortCell::Num { .. }, SortCell::Iri(_)) => Ordering::Greater,
        (SortCell::Iri(_), SortCell::Temp { .. }) => Ordering::Less,
        (SortCell::Temp { .. }, SortCell::Iri(_)) => Ordering::Greater,
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
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(a), SortCell::StrId(b)) => {
            str_id_value(graph, *a).cmp(str_id_value(graph, *b))
        }
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Iri(_)) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Iri(_), SortCell::StrId(_)) => Ordering::Less,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Num { .. }) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Num { .. }, SortCell::StrId(_)) => Ordering::Less,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(_), SortCell::Temp { .. }) => Ordering::Greater,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Temp { .. }, SortCell::StrId(_)) => Ordering::Less,
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::StrId(a), SortCell::Val { class: cb, kind: kb, key: kb_key, v: cv }) => {
            cmp_strid_val(graph, *a, *cb, *kb, kb_key.as_deref(), cv)
        }
        #[cfg(feature = "topk-lazy-strkey")]
        (SortCell::Val { class: ca, kind: ka, key: ka_key, v: av }, SortCell::StrId(b)) => {
            cmp_strid_val(graph, *b, *ca, *ka, ka_key.as_deref(), av).reverse()
        }
        (
            SortCell::Val { class: ca, kind: ka, key: ka_key, v: av },
            SortCell::Val { class: cb, kind: kb, key: kb_key, v: cv },
        ) => match ca.cmp(cb) {
            Ordering::Equal => {
                if *ca == TermClass::Literal as u8 && ka != kb {
                    return ka.cmp(kb);
                }
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
            if ia == ib {
                return Ordering::Equal;
            }
            match (graph.exact_numeric_lexical(ia), graph.exact_numeric_lexical(ib)) {
                (Some(la), Some(lb)) => cmp_decimal_str(&la, &lb).unwrap_or(Ordering::Equal),
                (Some(la), None) => cmp_exact_lex_f64(&la, fb),
                (None, Some(lb)) => cmp_exact_lex_f64(&lb, fa).reverse(),
                (None, None) => Ordering::Equal,
            }
        }
        Some(o) => o,
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
            if str_kind != vkind {
                return str_kind.cmp(&vkind);
            }
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
    #[cfg(not(target_arch = "wasm32"))]
    pub deadline: Option<std::time::Instant>,
    pub max_rows: Option<usize>,
    pub max_bytes: Option<usize>,
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
```

### crates/sparq-engine/src/lib.rs:723-730 — DatasetView
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub struct DatasetView<'g> {
    pub base: &'g Graph,
    pub named: std::sync::Arc<FxHashSet<Term>>,
    pub default: DefaultGraphMode,
}
```

### crates/sparq-engine/src/lib.rs:737-743 — DefaultGraphMode
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
pub enum DefaultGraphMode {
    #[default]
    StoreDefault,
    Empty,
}
```

### crates/sparq-engine/src/lib.rs:305-322 — QueryBudget methods
Blob e6f51e6360599db7a3e298bd378ac92c0eb3f4cf at 9e8bdfc95c916b62550fb8c3142f804bbbfb2444.

```rust
impl QueryBudget {
    pub fn unlimited() -> Self {
        Self::default()
    }

    pub fn cancelled_by(flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self::unlimited().with_cancel(flag)
    }

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

use crate::dict::Id;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perm {
    Spo,
    Sop,
    Pso,
    Pos,
    Osp,
    Ops,
}

#[cfg(not(any(target_arch = "wasm32", feature = "compact-index")))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];
#[cfg(any(target_arch = "wasm32", feature = "compact-index"))]
pub const BUILT: &[Perm] = &[Perm::Spo, Perm::Pos, Perm::Osp];

impl Perm {
    pub const ALL: [Perm; 6] = [Perm::Spo, Perm::Sop, Perm::Pso, Perm::Pos, Perm::Osp, Perm::Ops];

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
        let bound = |i: usize| pattern[i].is_some();
        for &perm in BUILT {
            let order = perm.order();
            let mut lead = 0;
            while lead < 3 && bound(order[lead]) {
                lead += 1;
            }
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

use hashbrown::HashTable;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashMap;
use std::hash::Hasher;

pub type Id = u32;

pub const NO_ID: Id = 0;

pub const INLINE_BASE: Id = 1 << 31;
const INLINE_MAX: u32 = (1 << 30) - 1;

#[allow(dead_code)]
pub(crate) const DICT_META_MAGIC: u32 = 0x31_56_4D_44; // b"DMV1" little-endian
#[allow(dead_code)]
pub(crate) const DICT_META_VERSION: u32 = 1;

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

fn try_inline(term: &Term) -> Option<Id> {
    match term {
        Term::Literal(l) => try_inline_lit(l.value(), l.datatype().as_str()),
        _ => None,
    }
}

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
            high_precision_decimal: std::sync::atomic::AtomicU8::new(0),
            named: self.named.iter().map(|(name, g)| (name.clone(), g.fork())).collect(),
            graph_prefix_index: std::sync::Mutex::new(None),
            #[cfg(feature = "mmap")]
            wal: None,
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
