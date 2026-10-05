use super::*;

// ---- Aggregation --------------------------------------------------------------

/// Group row indexes by their group-key, preserving FIRST-SEEN order: `order[i]` is the i-th
/// distinct key in row order and `members[i]` its row indexes (ascending). Above PAR_THRESHOLD the
/// build is radix-partitioned (Tier-1 of research/parallelism-scaling.md): a parallel pass tags
/// each row with its key-hash partition, each partition then builds its private map lock-free
/// (within a partition rows are scanned in ascending index, so a group's first row IS its min),
/// and the global first-seen order is re-imposed by sorting groups on min row index — exactly the
/// serial first-seen order, so output stays byte-identical.
/// `key_cols[i]` is the bindings column for the i-th GROUP BY variable, or `None` when that
/// variable is never bound (no column) — its key id is then `NO_ID` (unbound) for every row.
pub(super) fn build_groups(b: &Bindings, key_cols: &[Option<usize>]) -> (Vec<Key>, Vec<Vec<usize>>) {
    #[cfg(feature = "parallel")]
    if b.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        use std::hash::{Hash, Hasher};
        const P: usize = 64;
        // Pass 1 (parallel): partition tag per row.
        let parts: Vec<u8> = b
            .rows
            .par_iter()
            .map(|row| {
                let mut h = rustc_hash::FxHasher::default();
                for &c in key_cols {
                    c.map(|i| row[i]).unwrap_or(NO_ID).hash(&mut h);
                }
                (h.finish() % P as u64) as u8
            })
            .collect();
        // Pass 2 (parallel over partitions): private per-partition group builds. Each partition
        // scans the cheap tag vector and only constructs keys for its own rows.
        let per: Vec<Vec<(Key, usize, Vec<usize>)>> = (0..P)
            .into_par_iter()
            .map(|p| {
                let mut idx: FxHashMap<Key, usize> = FxHashMap::default();
                let mut out: Vec<(Key, usize, Vec<usize>)> = Vec::new(); // (key, min_ri, members)
                for (ri, row) in b.rows.iter().enumerate() {
                    if parts[ri] as usize != p {
                        continue;
                    }
                    let key: Key = key_cols.iter().map(|&c| c.map(|i| row[i]).unwrap_or(NO_ID)).collect();
                    match idx.get(&key) {
                        Some(&i) => out[i].2.push(ri),
                        None => {
                            idx.insert(key.clone(), out.len());
                            out.push((key, ri, vec![ri]));
                        }
                    }
                }
                out
            })
            .collect();
        // Pass 3: merge + re-impose the global first-seen order via min row index.
        let mut all: Vec<(Key, usize, Vec<usize>)> = per.into_iter().flatten().collect();
        all.par_sort_unstable_by_key(|&(_, min_ri, _)| min_ri);
        return all.into_iter().map(|(k, _, m)| (k, m)).unzip();
    }
    let mut idx: FxHashMap<Key, usize> = FxHashMap::default();
    let mut order: Vec<Key> = Vec::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    for (ri, row) in b.rows.iter().enumerate() {
        let key: Key = key_cols.iter().map(|&c| c.map(|i| row[i]).unwrap_or(NO_ID)).collect();
        match idx.get(&key) {
            Some(&i) => members[i].push(ri),
            None => {
                idx.insert(key.clone(), order.len());
                order.push(key);
                members.push(vec![ri]);
            }
        }
    }
    (order, members)
}

pub(super) fn group_aggregate(
    graph: &Graph,
    local: &mut LocalVocab,
    b: Bindings,
    group_vars: &[Variable],
    aggregates: &[(Variable, AggregateExpression)],
) -> Result<Bindings, String> {
    // A GROUP BY variable that is never bound anywhere in the WHERE clause has no column in
    // `b`. Per SPARQL 1.1 §11.1 the group key for such a variable evaluates to unbound for every
    // solution, so all rows share the same (unbound) key on that position — they collapse into one
    // group and the variable is unbound in the output. We model the missing column as `None` and
    // feed `NO_ID` (unbound) for it, rather than panicking. (Regression: bead sq-vymy4.)
    let key_cols: Vec<Option<usize>> = group_vars.iter().map(|v| b.col(v)).collect();

    // Group rows by the group-key id tuple, preserving first-seen order (parallel ≥ threshold).
    let (mut order, mut members) = build_groups(&b, &key_cols);
    // Whole-dataset aggregate with no GROUP BY: one (empty) group, even if input is empty.
    if group_vars.is_empty() && order.is_empty() {
        order.push(Key::new());
        members.push(Vec::new());
    }

    let mut out_vars: Vec<Variable> = group_vars.to_vec();
    for (v, _) in aggregates {
        out_vars.push(v.clone());
    }

    // (sq-pntvh.4) M4 Phase 4 columnar reducer seam (Seam B): for an eligible
    // aggregate (no DISTINCT, bare-variable argument, all-inline-integer column, supported
    // function), fold each group's ids with a tight integer reducer — no `eval_expr` dispatch
    // per member, no term materialisation, byte-identical output to the scalar path.
    // Declines (→ scalar path below) for anything not provably identical.
    #[cfg(feature = "vectorized")]
    if let Some(rows) = columnar_aggregate(graph, local, &b, group_vars, aggregates, &order, &members, &out_vars) {
        return Ok(Bindings::unsorted(out_vars, rows));
    }

    // Evaluate each group's aggregates (the expensive part — `eval_expr` over every member of
    // every group), then intern the result and build the output row, in first-seen `order`.
    // Interning needs `&mut LocalVocab` and so stays SERIAL and in order, making the ids
    // byte-identical regardless of how evaluation is scheduled.
    let mut rows: Vec<Row> = Vec::with_capacity(order.len());

    // roborev 1429 (Med): bound peak memory. The previous implementation
    // materialised the aggregate `Value`s (and then their resolved ids) for EVERY group at
    // once before building any row — for many-group / large GROUP_CONCAT queries that holds
    // all aggregate output in memory simultaneously. We now process in bounded BATCHES (and,
    // in the sequential path, one group at a time), interning and pushing each batch's rows
    // before evaluating the next, so only a bounded slice of `Value`s is live.
    #[cfg(feature = "parallel")]
    let parallel_eval = b.rows.len() >= PAR_THRESHOLD;
    #[cfg(not(feature = "parallel"))]
    let parallel_eval = false;

    #[cfg(feature = "parallel")]
    if parallel_eval {
        use rayon::prelude::*;
        // Thread-local extension-function registry and dataset view: snapshot +
        // per-item re-install (free when neither is installed) — see the FILTER branch.
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot(); // sq-mg9: keep the spatial index visible under EXISTS re-entry.
        // (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
        // a worker that missed the registry would dial the IRI instead of answering it.
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        // (sq-5qz9) keep a custom-aggregate registry visible off-thread, like `fns`.
        // (`self::` because the `aggregates` *parameter* below shadows the module name.)
        #[cfg(feature = "window-functions")]
        let aggs = self::aggregates::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        // Process `members`/`order` in PAR_THRESHOLD-sized batches: evaluate + read-only
        // resolve each batch in parallel, then serially intern only that batch's genuinely
        // new terms and emit its rows. Peak `Value` footprint is one batch, not all groups.
        for (key_chunk, member_chunk) in order.chunks(PAR_THRESHOLD).zip(members.chunks(PAR_THRESHOLD)) {
            let lv: &LocalVocab = local; // immutable reborrow for the read-only parallel phase
            let bref = &b;
            let resolved: Vec<Vec<Result<Id, Term>>> = member_chunk
                .par_iter()
                .map(|members| {
                    let _fns = functions::worker_install(&fns);
                    let _vw = view::worker_install(&vw);
                    let _spx = spatial::worker_install(&spx);
                    #[cfg(feature = "service-local")]
                    let _lsv = local_services::worker_install(&lsv);
                    #[cfg(feature = "window-functions")]
                    let _aggs = self::aggregates::worker_install(&aggs);
                    #[cfg(not(target_arch = "wasm32"))]
                    let _qn = query_now::worker_install(qn);
                    aggregates
                        .iter()
                        .map(|(_, agg)| eval_aggregate(graph, lv, bref, members, agg).map(|v| value_to_id_readonly(graph, lv, &v)))
                        .collect::<Result<Vec<_>, String>>()
                })
                .collect::<Result<Vec<_>, String>>()?;
            for (key, res) in key_chunk.iter().zip(resolved) {
                let mut row = Row::from_slice(key);
                for r in res {
                    row.push(match r {
                        Ok(id) => id,
                        Err(term) => local.intern(term),
                    });
                }
                rows.push(row);
            }
        }
    }

    if !parallel_eval {
        // Sequential STREAMING path: eval + intern + push per group, dropping each group's
        // `Value`s before moving to the next, so peak memory is a single group's aggregates.
        for (key, members) in order.iter().zip(&members) {
            let mut row = Row::from_slice(key);
            for (_, agg) in aggregates {
                let v = eval_aggregate(graph, local, &b, members, agg)?;
                let id = value_to_id(graph, local, &v);
                row.push(id);
            }
            rows.push(row);
        }
    }

    Ok(Bindings::unsorted(out_vars, rows))
}

/// (sq-v411r, survey §B2) Reserved IRI the vendored spargebra parser emits for the
/// zero-arg `MULTIPLICITY()` extension builtin (as a `Function::Custom`, to keep the shared
/// `Function` enum byte-compatible with downstream exhaustive matchers). The engine recognises
/// it in aggregate evaluation and function dispatch; it is never a user-registrable IRI.
pub(crate) const MULTIPLICITY_FN_IRI: &str = "urn:sparq:fn:multiplicity";

/// (sq-v411r, survey §B2) The DISTINCT solutions of a group paired with each
/// one's bag cardinality (`multiplicity(μ|Ω)`): `(ri, card)` is a representative row index
/// for a distinct solution mapping and how many rows in `members` are byte-identical to it,
/// in first-seen order. Returns `None` (and does NO work) unless `expr` actually calls
/// `MULTIPLICITY()`, so an aggregate that never references it pays nothing and folds the
/// full bag exactly as before.
///
/// The collapse to distinct members is load-bearing for correctness, NOT an optimisation:
/// the SPARQL algebra's Set Functions are defined over the multiset by iterating its
/// DISTINCT elements weighted by `multiplicity`, so `SUM(?x * MULTIPLICITY())` must visit
/// `?x=10` once (with multiplicity 3), not three times — otherwise a value occurring `k`
/// times would contribute `k²·x` instead of `k·x`. With this collapse the standard identity
/// `SUM(?x * MULTIPLICITY()) == SUM(?x)` (over the bag) holds. Solution identity is the
/// whole row, matching `COUNT(DISTINCT *)`; BGP matching is bag semantics, so duplicate
/// solutions are preserved as repeated rows up to this point.
pub(super) fn group_multiplicities(b: &Bindings, members: &[usize], expr: &Expression) -> Option<Vec<(usize, u64)>> {
    if !expr_uses_multiplicity(expr) {
        return None;
    }
    // Count identical rows, keeping a first-seen representative index for each distinct row.
    let mut counts: FxHashMap<&Row, (usize, u64)> = FxHashMap::default();
    let mut order: Vec<&Row> = Vec::new();
    for &ri in members {
        match counts.get_mut(&b.rows[ri]) {
            Some(entry) => entry.1 += 1,
            None => {
                counts.insert(&b.rows[ri], (ri, 1));
                order.push(&b.rows[ri]);
            }
        }
    }
    Some(order.into_iter().map(|row| counts[row]).collect())
}

/// Whether `expr` contains a `MULTIPLICITY()` call anywhere in its tree. An `Exists`
/// sub-pattern is NOT descended into: a `MULTIPLICITY()` there belongs to that
/// (un-aggregated) sub-query's scope, not this aggregate's member — and would
/// correctly evaluate against the thread-local's restored (absent) context.
pub(super) fn expr_uses_multiplicity(expr: &Expression) -> bool {
    use Expression as E;
    match expr {
        // `MULTIPLICITY()` is parsed as a `Function::Custom` with the reserved IRI
        // (see `MULTIPLICITY_FN_IRI`) so the shared `Function` enum stays byte-compatible
        // with downstream exhaustive matchers (sparopt/spareval).
        E::FunctionCall(spargebra::algebra::Function::Custom(nn), args)
            if args.is_empty() && nn.as_str() == MULTIPLICITY_FN_IRI =>
        {
            true
        }
        E::NamedNode(_) | E::Literal(_) | E::Variable(_) | E::Bound(_) | E::Exists(_) => false,
        E::Or(a, c)
        | E::And(a, c)
        | E::Equal(a, c)
        | E::SameTerm(a, c)
        | E::Greater(a, c)
        | E::GreaterOrEqual(a, c)
        | E::Less(a, c)
        | E::LessOrEqual(a, c)
        | E::Add(a, c)
        | E::Subtract(a, c)
        | E::Multiply(a, c)
        | E::Divide(a, c) => expr_uses_multiplicity(a) || expr_uses_multiplicity(c),
        E::UnaryPlus(a) | E::UnaryMinus(a) | E::Not(a) => expr_uses_multiplicity(a),
        E::If(a, c, d) => {
            expr_uses_multiplicity(a) || expr_uses_multiplicity(c) || expr_uses_multiplicity(d)
        }
        E::In(a, list) => expr_uses_multiplicity(a) || list.iter().any(expr_uses_multiplicity),
        E::Coalesce(list) | E::FunctionCall(_, list) => list.iter().any(expr_uses_multiplicity),
    }
}

pub(super) fn eval_aggregate(graph: &Graph, local: &LocalVocab, b: &Bindings, members: &[usize], agg: &AggregateExpression) -> Result<Value, String> {
    match agg {
        AggregateExpression::CountSolutions { distinct } => {
            let n = if *distinct {
                let mut seen = std::collections::HashSet::new();
                members.iter().filter(|&&ri| seen.insert(b.rows[ri].clone())).count()
            } else {
                members.len()
            };
            Ok(Value::Num(Num::Int(n as i64)))
        }
        AggregateExpression::FunctionCall { name, expr, distinct } => {
            // MIN/MAX over an all-temporal column: fold at the ID level through the
            // temporals cache — no term materialised, no per-comparison lexical
            // re-parse. Falls through to the general path on any non-temporal member.
            if let AggregateFunction::Min | AggregateFunction::Max = name {
                let is_min = matches!(name, AggregateFunction::Min);
                if let Some(v) = minmax_temporal(graph, local, b, members, expr, *distinct, is_min) {
                    return Ok(v);
                }
            }
            // (sq-5qz9) A declared custom aggregate IRI: fold the group's
            // per-member values through the installed registry. Unlike the builtins,
            // the user aggregate sees the FULL member sequence (an unbound member is
            // passed as `None`, not skipped) so it can implement count-style or
            // unbound-sensitive aggregates; DISTINCT de-duplicates the materialised
            // member terms first (treating two unbound members as equal). The closure
            // works in `oxrdf::Term`, so the per-member value is materialised to a term.
            #[cfg(feature = "window-functions")]
            if let AggregateFunction::Custom(iri) = name {
                return eval_custom_aggregate(graph, local, b, members, expr, *distinct, iri.as_str());
            }
            // Collect the per-member values of `expr`. An UNBOUND member (a variable
            // with no binding in that row, e.g. an OPTIONAL one) is SKIPPED for every aggregate,
            // matching SPARQL "aggregate over the bound values": SUM/AVG over an OPTIONAL column
            // must still sum the rows that do have a value, not collapse to unbound. Only a
            // genuine expression ERROR (`Value::Error`, e.g. a type error inside `expr`) is fatal
            // for SUM/AVG. A BOUND but non-numeric member is pushed into `vals` and turns SUM/AVG
            // into a type error downstream (`as_numeric` -> None), per agg-err-01.
            let mut vals: Vec<Value> = Vec::with_capacity(members.len());
            let mut errored = false;
            // (sq-v411r, §B2) Only build the multiplicity table when the argument
            // actually calls `MULTIPLICITY()` — the overwhelmingly common aggregate pays
            // nothing (no table, no thread-local write) and folds the full bag below. When
            // present, fold the DISTINCT solutions instead, each weighted by its bag
            // cardinality via the thread-local, per the Set-Function algebra (see
            // `group_multiplicities`).
            match group_multiplicities(b, members, expr) {
                None => {
                    for &ri in members {
                        let v = eval_expr(graph, local, b, &b.rows[ri], expr)?;
                        match v {
                            Value::Unbound => {}             // skip: aggregate ignores unbound rows
                            Value::Error => errored = true,  // fatal for SUM/AVG (-> unbound aggregate)
                            _ => vals.push(v),
                        }
                    }
                }
                Some(distinct) => {
                    for (ri, card) in distinct {
                        let _mg = multiplicity::set(card);
                        let v = eval_expr(graph, local, b, &b.rows[ri], expr)?;
                        match v {
                            Value::Unbound => {}
                            Value::Error => errored = true,
                            _ => vals.push(v),
                        }
                    }
                }
            }
            if *distinct {
                dedup_values(&mut vals);
            }
            match name {
                AggregateFunction::Count => Ok(Value::Num(Num::Int(vals.len() as i64))),
                // SUM/AVG with operand-type promotion: int+int stays integer, decimals
                // stay decimal (exact), floats/doubles promote. Any non-numeric or
                // errored member makes the whole aggregate a type error (-> unbound).
                AggregateFunction::Sum => Ok(sum_values(&vals, errored).map(num_canonical_term).unwrap_or(Value::Error)),
                AggregateFunction::Avg => {
                    if vals.is_empty() && !errored {
                        return Ok(Value::Num(Num::Int(0))); // AVG({}) = 0 per SPARQL
                    }
                    Ok(sum_values(&vals, errored)
                        .and_then(|s| s.binop(Num::Int(vals.len() as i64), ArithOp::Div))
                        .map(Value::Num)
                        .unwrap_or(Value::Error))
                }
                // MIN/MAX over an all-numeric group return the typed VALUE (promoted,
                // canonically serialised — "2.0E-1"^^xsd:double); mixed groups keep the
                // lenient term path.
                AggregateFunction::Min => Ok(minmax_values(vals, Ordering::Less)),
                AggregateFunction::Max => Ok(minmax_values(vals, Ordering::Greater)),
                AggregateFunction::GroupConcat { separator } => {
                    let sep = separator.clone().unwrap_or_else(|| " ".to_string());
                    let joined = vals.iter().filter_map(value_str).collect::<Vec<_>>().join(&sep);
                    Ok(Value::Term(Term::Literal(Literal::new_simple_literal(joined))))
                }
                AggregateFunction::Sample => Ok(vals.into_iter().next().unwrap_or(Value::Unbound)),
                _ => Err("M2: unsupported aggregate".into()),
            }
        }
    }
}

/// (sq-5qz9) Evaluate a declared custom aggregate IRI against the
/// installed [`crate::CustomAggregateRegistry`]. Materialises each group member's
/// value of `expr` to an `Option<Term>` (`None` ≡ unbound for that member —
/// passed THROUGH, not skipped, so a user count-style aggregate can see it),
/// applies DISTINCT over the materialised members, then folds via the closure.
/// A missing registry / unregistered IRI is the same hard error the registry-free
/// path raises; a closure `Err` is a SPARQL expression error (→ unbound result).
#[cfg(feature = "window-functions")]
pub(super) fn eval_custom_aggregate(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    members: &[usize],
    expr: &Expression,
    distinct: bool,
    iri: &str,
) -> Result<Value, String> {
    let Some(f) = aggregates::lookup(iri) else {
        return Err(format!("unsupported SPARQL function: no custom aggregate registered for <{iri}>"));
    };
    let mut args: Vec<Option<Term>> = Vec::with_capacity(members.len());
    // (sq-v411r) `MULTIPLICITY()` works inside a custom aggregate's argument too:
    // when it appears, fold the DISTINCT solutions weighted by each one's bag cardinality
    // (same algebra as the builtins); otherwise fold the full bag exactly as before.
    let mults = group_multiplicities(b, members, expr);
    let iter: Vec<(usize, Option<u64>)> = match &mults {
        None => members.iter().map(|&ri| (ri, None)).collect(),
        Some(distinct) => distinct.iter().map(|&(ri, card)| (ri, Some(card))).collect(),
    };
    for (ri, card) in iter {
        let _mg = card.map(multiplicity::set);
        let v = eval_expr(graph, local, b, &b.rows[ri], expr)?;
        // A genuine expression error inside the argument makes the aggregate a
        // SPARQL error (→ unbound), mirroring SUM/AVG; an unbound member is a
        // first-class `None` argument the user aggregate decides how to treat.
        if matches!(v, Value::Error) {
            return Ok(Value::Error);
        }
        args.push(value_as_term(&v));
    }
    if distinct {
        let mut seen: FxHashSet<Option<Term>> = FxHashSet::default();
        args.retain(|t| seen.insert(t.clone()));
    }
    match f(&args) {
        Ok(Some(t)) => Ok(Value::Term(t)),
        Ok(None) => Ok(Value::Unbound),
        Err(_) => Ok(Value::Error), // expression error → unbound aggregate (builtin discipline)
    }
}

/// Typed SUM with XPath promotion; `None` (a type error) if any member was non-numeric
/// or errored. The empty sum is `"0"^^xsd:integer`.
pub(super) fn sum_values(vals: &[Value], errored: bool) -> Option<Num> {
    if errored {
        return None;
    }
    let mut acc = Num::Int(0);
    for v in vals {
        acc = acc.binop(as_numeric(v)?, ArithOp::Add)?;
    }
    Some(acc)
}

/// MIN/MAX: an all-numeric group compares by VALUE (exact for int/decimal) and returns
/// the typed value; any non-numeric member falls back to the lenient total-order term
/// comparison (which must order across types for the SPARQL MIN/MAX-over-anything case).
pub(super) fn minmax_values(vals: Vec<Value>, keep: Ordering) -> Value {
    if vals.is_empty() {
        return Value::Unbound;
    }
    let nums: Option<Vec<Num>> = vals.iter().map(as_numeric).collect();
    match nums {
        Some(nums) => {
            let mut best = nums[0];
            for &n in &nums[1..] {
                if num_compare(n, best) == Some(keep) {
                    best = n;
                }
            }
            num_canonical_term(best)
        }
        None => {
            let cmp = |a: &Value, c: &Value| compare_values(a, c).unwrap_or(Ordering::Equal);
            match keep {
                Ordering::Less => vals.into_iter().min_by(cmp).unwrap(),
                _ => vals.into_iter().max_by(cmp).unwrap(),
            }
        }
    }
}

/// MIN/MAX over a variable whose group members are ALL well-formed temporal
/// (dateTime/date) graph terms, folded at the id level through the temporals cache.
/// `None` falls back to the general (materialise + compare_values) path: any unbound
/// member is skipped (as the general path skips it), but a local-vocab or
/// non-temporal member aborts the fast path entirely.
///
/// Tie semantics replicate `minmax_values` exactly: the comparator is
/// `compare_values(..).unwrap_or(Equal)` — which for two temporals IS
/// `Temporal::cmp_t_total` (kind-first, then the timeline order extended over the
/// indeterminate mixed-timezone window) — with MIN keeping the FIRST of
/// equal members (`Iterator::min_by`) and MAX the LAST (`Iterator::max_by`); DISTINCT
/// drops later duplicate terms first (same term ⇔ same id for graph terms), which can
/// change which of two equal-VALUED but distinct terms MAX returns, exactly as the
/// general path's `dedup_values` does.
pub(super) fn minmax_temporal(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    members: &[usize],
    expr: &Expression,
    distinct: bool,
    is_min: bool,
) -> Option<Value> {
    let Expression::Variable(v) = expr else { return None };
    let col = b.col(v)?;
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut best: Option<(Temporal, Id)> = None;
    for &ri in members {
        let id = b.rows[ri][col];
        if id == NO_ID {
            continue; // unbound member: MIN/MAX skips it
        }
        if is_local(id) {
            return None; // computed term: general path
        }
        let t = graph.temporal_value(id)?; // non-temporal/ill-formed: general path
        if distinct && !seen.insert(id) {
            continue;
        }
        best = Some(match best {
            None => (t, id),
            Some((bt, bid)) => {
                // sq-wjl8i KIND-FIRST + sq-2k5py, both now in the
                // shared `Temporal::cmp_t_total`, so this fold cannot drift from
                // `compare_values`: a cross-kind (dateTime vs date) pair ranks by
                // `LiteralKind` (DateTime < Date), never lexically, and within a kind the
                // timeline order is TOTAL (instant, then timezone presence) — the former
                // lexical fallback for the indeterminate window was intransitive.
                let ord = Temporal::cmp_t_total(t, bt);
                let replace = if is_min { ord == Ordering::Less } else { ord != Ordering::Less };
                if replace {
                    (t, id)
                } else {
                    (bt, bid)
                }
            }
        });
    }
    Some(match best {
        Some((_, id)) => Value::Term(term_of(graph, local, id).expect("aggregate member id resolves")),
        None => Value::Unbound, // no bound members
    })
}

/// Value comparison of two typed numerics: exact when both are int/decimal, f64 otherwise.
pub(super) fn num_compare(a: Num, c: Num) -> Option<Ordering> {
    if let (Some(x), Some(y)) = (a.to_dec(), c.to_dec()) {
        if let Some(o) = x.cmp(y) {
            return Some(o);
        }
    }
    a.f64().partial_cmp(&c.f64())
}

pub(super) fn dedup_values(vals: &mut Vec<Value>) {
    let mut seen = std::collections::HashSet::new();
    vals.retain(|v| seen.insert(value_key(v)));
}

pub(super) fn value_key(v: &Value) -> String {
    match v {
        Value::Term(t) => format!("T{t}"),
        Value::Num(n) => format!("N{}^^{}", n.lexical(), n.datatype().as_str()),
        Value::Bool(b) => format!("B{b}"),
        Value::Unbound => "U".to_string(),
        Value::Error => "E".to_string(),
    }
}
