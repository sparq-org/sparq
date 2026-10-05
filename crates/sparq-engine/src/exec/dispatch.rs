use super::*;

// ---- Algebra dispatch ---------------------------------------------------------

pub(super) fn eval_modified(graph: &Graph, local: &mut LocalVocab, p: &GraphPattern) -> Result<Bindings, String> {
    // Pin NOW() for this execution (SPARQL 1.1 §17.4.5.1). Outermost call samples
    // the clock once; the recursive / EXISTS re-entries see it active and keep the
    // outer instant (a Cell read). sq-98w7z.1
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
            // sq-7d3dj.30.4
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
                    // Result is byte-identical to the full-sort+slice path. sq-7d3dj.30.2
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

/// Attempts the bounded-heap ORDER BY optimisation for a `Slice` with a known
/// row budget `k = offset + limit`.
///
/// Peeks through `Project` / `Reduced` transparent wrappers to find an `OrderBy`
/// node. When found, evaluates the pattern below the `OrderBy`, sorts using
/// `order_bindings` with the top-k hint so only `k` rows survive, applies any
/// projection, and returns `Some(Bindings)`. Returns `None` when no `OrderBy`
/// is present at the transparent-wrapper boundary — the caller falls through to
/// full `eval_modified`. sq-7d3dj.30.2
pub(super) fn try_topk_orderby(
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
            // (sq-artifact-keeper-topk) Try the index-ordered seed path
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

/// (sq-artifact-keeper-topk) Attempts INDEX-ORDERED top-k evaluation for a
/// `Slice{OrderBy{BGP}}` shape whose primary sort key is bound by exactly one triple
/// pattern's OBJECT. `try_topk_orderby`'s existing bounded-heap path still calls
/// `eval_modified` to fully evaluate (and materialise) the WHOLE matching candidate
/// set before selecting the top `row_budget` rows — `O(n)` in the number of rows
/// CURRENTLY matching the BGP's equality filters, not in `row_budget`. That is fine
/// when some pattern is selective, but when every pattern matches nearly the same
/// rows (e.g. a job-queue "claim the next pending task for peer X, ordered by
/// priority" query, where `peer` + `status` don't shrink the candidate set at all),
/// there is no cheaper seed for the cardinality-based planner to pick, and the cost
/// scales with the size of the whole pending queue on every single claim — profiled
/// and confirmed via `EXPLAIN ANALYZE` (the BGP operator reports touching every
/// matching row) in the sparq/artifact-keeper migration's claim-queue throughput
/// investigation.
///
/// SOUNDNESS. Only activates for a narrow, syntactically-verified "star" shape, and
/// declines (`Ok(None)`) at the first sign of anything it cannot prove safe — the
/// caller's `eval_modified`-then-select path is always correct, just `O(n)`; this
/// function's only job is to be a provably-equivalent shortcut, never a new source
/// of results:
/// - `ord_inner` is a plain triple-pattern conjunction with NO residual FILTER and
///   no blank nodes (kept out of scope for this first cut).
/// - The FIRST order key is a bare `Variable` (secondary tie-break keys, if any,
///   are resolved by re-using `order_bindings` over the small collected set below —
///   not reimplemented here).
/// - Exactly one pattern (the "seed") binds that variable as its OBJECT, with a
///   CONSTANT predicate and a VARIABLE subject (the join "hub").
/// - Every OTHER pattern has that same hub variable as its SUBJECT and a CONSTANT
///   predicate (a star join around the hub); any object variable it introduces must
///   not appear anywhere else in the BGP. Any other shape (the hub appearing
///   elsewhere, a variable predicate, a reused object variable) declines.
/// - Every object id on the seed's sorted scan is an INLINE small integer
///   (`dict::is_inline`) — the only case where ascending dictionary-id order is
///   proven to equal ascending SPARQL `value()` order (the same guard
///   `scan_to_bindings`'s range-pruning already relies on for the same reason).
///   Checked for the WHOLE scan up front, before any result is built, so a
///   non-inline id never leaks a partially-computed (and potentially
///   wrongly-ordered) answer.
///
/// Under those conditions, walking the seed's sorted scan in the required
/// direction, testing each candidate hub value against the OTHER patterns via a
/// direct bound lookup (index-nested-loop / bind join over a per-pattern
/// subject-sorted range resolved once, not re-resolved per candidate — see the
/// `other_pats` construction below), and stopping once a COMPLETE sort-key group
/// (never split mid-tie) has produced at least `row_budget` confirmed joins
/// yields valid top rows: an unvisited group is strictly worse in the requested
/// primary-key direction, so it cannot displace the collected top `row_budget`.
/// If all ORDER BY keys tie, SPARQL permits different surviving
/// subsets and tie order; equivalence does not require the fallback's input-index
/// stability. Finishing a primary-key group preserves secondary-key selection.
///
/// PERFORMANCE, not soundness: a single escalation block is also capped at a
/// fraction of the candidate pool (see `max_group` below) — exceeding it
/// declines (`Ok(None)`) even though continuing would still be CORRECT, because
/// past that point this function's per-candidate cost stops being cheaper than
/// the fallback's bulk join (measured; see `max_group`'s comment). This keeps
/// the walk bounded, but setup and failed probes still precede a decline.
///
/// Recovery-stage restriction: budgeted queries, repeated variables
/// and multi-valued probes use the existing evaluator. The historical timing notes
/// below are unverified on current main; this candidate establishes no speedup.
pub(super) fn try_topk_orderby_indexed(
    graph: &Graph,
    local: &mut LocalVocab,
    ord_inner: &GraphPattern,
    expression: &[OrderExpression],
    row_budget: usize,
) -> Result<Option<Bindings>, String> {
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return Ok(None);
    }
    // Preserve the existing evaluator's intermediate row/byte
    // accounting and cooperative cancellation schedule before doing any new work.
    if budget::active() {
        return Ok(None);
    }
    if view::default_is_empty() {
        return Ok(None);
    }
    let Some((desc, order_var)) = expression.first().and_then(|oe| match oe {
        OrderExpression::Asc(Expression::Variable(v)) => Some((false, v.clone())),
        OrderExpression::Desc(Expression::Variable(v)) => Some((true, v.clone())),
        _ => None,
    }) else {
        return Ok(None);
    };
    if !is_conjunctive(ord_inner) {
        return Ok(None);
    }
    let mut patterns = Vec::new();
    let mut filters = Vec::new();
    flatten_conjunction(ord_inner, &mut patterns, &mut filters);
    if !filters.is_empty() || patterns.is_empty() {
        return Ok(None);
    }
    // prepare_pattern records repeated slots but does not enforce
    // their equality. Reuse the existing fast-path guard before bypassing build_row.
    if patterns.iter().any(has_intra_triple_repeated_var) {
        return Ok(None);
    }
    // `prepare_pattern` resolves a triple term as a ground term, so a variable nested
    // inside one would error instead of matching; decline and let the evaluator run it.
    if patterns.iter().any(has_quoted_triple_term) {
        return Ok(None);
    }
    // Out of scope for this first cut: blank nodes are treated as synthetic
    // variables by `prepare_pattern` (`bnode_var`) but not necessarily by
    // `collect_vars`'s header-construction — decline rather than risk a header
    // mismatch.
    let has_blank_node = patterns.iter().any(|tp| {
        matches!(tp.subject, TermPattern::BlankNode(_)) || matches!(tp.object, TermPattern::BlankNode(_))
    });
    if has_blank_node {
        return Ok(None);
    }

    // Locate the ONE seed pattern binding `order_var` as its object with a
    // constant predicate and a variable subject.
    let mut seed_idx = None;
    for (i, tp) in patterns.iter().enumerate() {
        if !matches!(&tp.object, TermPattern::Variable(v) if *v == order_var) {
            continue;
        }
        if !matches!(&tp.predicate, NamedNodePattern::NamedNode(_)) {
            return Ok(None);
        }
        if !matches!(&tp.subject, TermPattern::Variable(_)) {
            return Ok(None);
        }
        if seed_idx.is_some() {
            return Ok(None); // order_var bound by more than one pattern: not this shape
        }
        seed_idx = Some(i);
    }
    let Some(seed_idx) = seed_idx else { return Ok(None) };
    let hub_var = match &patterns[seed_idx].subject {
        TermPattern::Variable(v) => v.clone(),
        _ => unreachable!("checked above"),
    };

    // Every OTHER pattern must be `hub_var <constant predicate> (var|const)` — a
    // star around the hub — and any variable it introduces must be fresh.
    let mut other_vars: FxHashSet<Variable> = FxHashSet::default();
    for (i, tp) in patterns.iter().enumerate() {
        if i == seed_idx {
            continue;
        }
        if !matches!(&tp.subject, TermPattern::Variable(v) if *v == hub_var) {
            return Ok(None);
        }
        if !matches!(&tp.predicate, NamedNodePattern::NamedNode(_)) {
            return Ok(None);
        }
        if let TermPattern::Variable(v) = &tp.object {
            if *v == hub_var || *v == order_var || !other_vars.insert(v.clone()) {
                return Ok(None);
            }
        }
    }

    let out_vars = collect_vars(&patterns);
    if row_budget == 0 {
        return Ok(Some(Bindings::unsorted(out_vars, vec![])));
    }

    // Resolve + scan the seed pattern sorted by its OBJECT (canonical column 2).
    let (seed_id_pat, _seed_pos_vars, seed_unsat) = prepare_pattern(graph, &patterns[seed_idx])?;
    if seed_unsat {
        return Ok(Some(Bindings::unsorted(out_vars, vec![])));
    }
    let scan = graph.store.scan_sorted(&seed_id_pat, 2);
    let actual_sort = scan.perm.order().into_iter().find(|&c| seed_id_pat[c].is_none());
    if actual_sort != Some(2) {
        return Ok(None); // this store build can't give us an object-sorted scan
    }
    let rows: &[[Id; 3]] = scan.rows.as_ref();
    // Guard: EVERY object id on this scan must be an inline integer, or ascending
    // id order is not proven to be ascending value order.
    if !rows.iter().all(|r| dict::is_inline(scan.to_spo(r)[2])) {
        return Ok(None);
    }

    // The first block must include its entire leading key group.
    // Reject a group larger than the EXISTING block cap before retaining probe
    // scans and subject vectors. Sorted object ids make equality at the cap's
    // zero-based edge equivalent to group_len > max_group, in either direction.
    const MAX_INDEXED_GROUP_FLOOR: usize = 256;
    let n = rows.len();
    let max_group = (n / 2).max(MAX_INDEXED_GROUP_FLOOR).max(row_budget.saturating_mul(2));
    if n > max_group {
        let first = if desc { n - 1 } else { 0 };
        let edge = if desc { first - max_group } else { max_group };
        if scan.to_spo(&rows[first])[2] == scan.to_spo(&rows[edge])[2] {
            return Ok(None);
        }
    }

    // Prepare the other patterns ONCE ON DEMAND, each as a SUBJECT-sorted scan over its
    // (fixed predicate [+ fixed object]) range — not re-resolved per candidate.
    // The first cut of this function called `graph.store.scan(&probe_pat)`
    // fresh for every (candidate, other-pattern) pair, which re-runs
    // `Store::choose`'s permutation selection + bound computation every single
    // time even though the choice is IDENTICAL across all candidates for a
    // given pattern (only the subject varies). That repeated per-call setup —
    // not the underlying lookup — is what made a large tie-group lose to the
    // fallback's bulk merge-join (which resolves the permutation ONCE per
    // pattern, not once per row). Resolving it once here and binary-searching
    // the resulting sorted slice per candidate removes that repeated cost.
    struct OtherScan<'g> {
        scan: sparq_core::store::Scan<'g>,
        // Precomputed ONCE (not per candidate, not per binary-search
        // comparison step): the subject id of every row in `scan`, in the
        // SAME (subject-sorted) order. `partition_point` over this plain
        // `&[Id]` does a trivial integer compare per step; searching `scan`
        // directly would call `to_spo` (a permutation-order reconstruction)
        // on EVERY comparison, i.e. ~log(m) reconstructions per candidate per
        // pattern — measured as a real remaining cost for a long skip-prefix
        // (many candidates probed and rejected in a row): reconstructing the
        // full [S,P,O] triple just to read column 0 is wasted work when the
        // search only ever needs that one column.
        subject_ids: Vec<Id>,
    }
    // Resolve metadata eagerly, but retain a scan/vector only when
    // a candidate reaches this pattern in the original order. A failed constant
    // prefix therefore does not prepare later variable probes that it never uses.
    struct OtherPat<'g> {
        id_pat: IdPattern,
        obj_var: Option<Variable>,
        prepared: Option<OtherScan<'g>>,
    }
    fn prepare_other<'g>(graph: &'g Graph, op: &mut OtherPat<'g>, seed_card: usize) -> bool {
        if op.prepared.is_some() {
            return true;
        }
        let sub_scan = graph.store.scan_sorted(&op.id_pat, 0);
        let sub_actual_sort = sub_scan.perm.order().into_iter().find(|&c| op.id_pat[c].is_none());
        if sub_actual_sort != Some(0) {
            return false;
        }
        // The original exact-cardinality admission rule still applies to EVERY
        // probe before it can contribute to a row, including a late selective one.
        if sub_scan.rows.len().saturating_mul(2) < seed_card {
            return false;
        }
        let subject_ids: Vec<Id> = sub_scan.rows.iter().map(|r| sub_scan.to_spo(r)[0]).collect();
        #[cfg(test)]
        indexed_topk_preparation_tests::observe(subject_ids.len());
        op.prepared = Some(OtherScan { scan: sub_scan, subject_ids });
        true
    }
    let mut other_pats: Vec<OtherPat> = Vec::with_capacity(patterns.len().saturating_sub(1));
    for (i, tp) in patterns.iter().enumerate() {
        if i == seed_idx {
            continue;
        }
        let (id_pat, pos_vars, unsat) = prepare_pattern(graph, tp)?;
        if unsat {
            // A constant elsewhere in the BGP absent from the dictionary makes the
            // WHOLE conjunction empty (a BGP join against an empty relation).
            return Ok(Some(Bindings::unsorted(out_vars, vec![])));
        }
        other_pats.push(OtherPat {
            id_pat: [None, id_pat[1], id_pat[2]],
            obj_var: pos_vars[2].clone(),
            prepared: None,
        });
    }

    // The previous global min-cardinality check is now applied
    // by prepare_other to each exact scan count before that probe is used. A
    // successful row must visit every pattern, so it passes the same admission
    // rule. Unvisited patterns stay unproved and cannot authorize an empty result.

    // The output column list is FIXED across every candidate (hub, order, then
    // each other pattern's object variable, in pattern order) — compute it and
    // the out_vars -> column-position mapping ONCE, not per candidate.
    let mut cols: Vec<Variable> = Vec::with_capacity(2 + other_pats.len());
    cols.push(hub_var.clone());
    cols.push(order_var.clone());
    for op in &other_pats {
        if let Some(ov) = &op.obj_var {
            cols.push(ov.clone());
        }
    }
    let out_to_cols: Vec<Option<usize>> = out_vars.iter().map(|v| cols.iter().position(|c| c == v)).collect();
    let emit = |ids: &[Id], collected: &mut Vec<Row>| {
        let mut row = Row::with_capacity(out_to_cols.len());
        for pos in &out_to_cols {
            row.push(pos.map(|i| ids[i]).unwrap_or(NO_ID));
        }
        collected.push(row);
    };

    // Walk the sorted scan in the required direction, in escalating blocks aligned
    // to complete sort-key (object-id) groups, joining each candidate against the
    // other patterns via a direct bound lookup (index-nested-loop / bind join).
    //
    // `rows` is always ASCENDING by object id (the scan's actual sort order). For
    // DESC, "best first" means walking it back-to-front. Rather than slicing from
    // the front and reversing per block (which — subtly, and wrongly — visits the
    // SMALLEST values first regardless of direction, since the slice bounds
    // themselves were never flipped), define a single LOGICAL index `0..n` where
    // position 0 is always the best candidate, via `logical`, and do all
    // block/boundary arithmetic in that space. `logical` and `obj_id_at` are the
    // only direction-aware code; everything below them is direction-agnostic.
    let logical = |i: usize| -> usize { if desc { n - 1 - i } else { i } };
    let obj_id_at = |i: usize| -> Id { scan.to_spo(&rows[logical(i)])[2] };
    // A single escalation block is dominated by ONE large tie-group when the
    // sort key has low cardinality (e.g. a handful of discrete priority tiers:
    // "urgent/high/normal/low", not a near-unique value per row). This
    // function's per-candidate cost is roughly CONSTANT per candidate (a
    // binary search per other pattern, resolved against a scan fetched once —
    // see `other_pats` below), while the fallback's bulk merge-join cost is
    // roughly constant PER `n` regardless of tie structure (it materialises the
    // whole matching set before it can sort). So the crossover is a FRACTION of
    // `n`, not an absolute row count: measured at n=1600, a tie-group of 800
    // (half of n) still beat the fallback (~228us vs. the fallback's ~269us),
    // but a tie-group of 1600 (all of n) lost (would be ~450-700us vs. the
    // fallback's own ~269us) — and the same ~0.5-0.75 fraction held at n=8000
    // (4000 still competitive, 8000 clearly lost). `max_group` is
    // therefore `n / 2` (with a floor for small `n`, and never below
    // `row_budget` itself) — declining past it defers to the fallback, with
    // any preparation and failed probes already performed adding to its cost.
    let mut collected: Vec<Row> = Vec::new();
    let mut visited_to: usize = 0;
    // Block sizes grow GEOMETRICALLY from a small multiple of `row_budget`, not
    // from `CAPPED_SEED_BLOCK` (1024): each candidate here costs a real
    // per-pattern binary search (against the scan `other_pats` already resolved
    // once, above), unlike the bulk merge-join the fallback path uses, which
    // amortises its own permutation selection over the WHOLE scan in one pass.
    // For the common case (small `row_budget`, most candidates
    // join successfully), starting near `row_budget` means the first block alone
    // usually satisfies it — starting at 1024 would pay ~1024 point-probes even for
    // `LIMIT 1`, which is worse than the bulk path it's meant to beat (measured:
    // this was the actual cause of a regression at moderate `n`, not a win).
    // Cumulative count of candidates that failed an OTHER-pattern check, across
    // ALL blocks in this call — distinct from `max_group`'s per-block width
    // check. A workload that claims strictly in priority order (the realistic
    // shape: highest priority first) leaves an ever-growing prefix of
    // ALREADY-CLAIMED, high-priority-but-no-longer-`pending` rows at the head
    // of the seed scan — each one still costs a real per-pattern probe before
    // being rejected. That prefix is made of individually DISTINCT priority
    // values, so it never forms one oversized tie-group `max_group` would
    // catch; it spreads across many small geometric-growth blocks instead. The
    // per-pattern cost check (comparing exact scan cardinalities to
    // the seed's) already declines the CLEAR case — an other-pattern that's
    // globally selective enough for the fallback's own planner to prefer as
    // ITS seed — before that probe is used. This counter is a
    // SAFETY NET for what that check can't see: the seed's global cardinality
    // vs. an other-pattern's global cardinality doesn't capture every
    // possible skip-prefix shape (e.g. a correlation between scan order and
    // an other-pattern's matches that isn't visible from cardinality alone).
    //
    // The budget here must stay LARGE (matching `max_group`, not a small
    // constant): every failed probe before declining is pure waste stacked ON
    // TOP OF the fallback's full cost, but a SMALL budget bails on the far
    // more common "moderate skip-prefix" case too — one that would have
    // SUCCEEDED cheaply if allowed to keep going (a candidate ~100-800
    // positions in costs only tens of us to walk to, vastly cheaper than a
    // ~100-350us fallback). Measured directly: tightening this to a small
    // constant (64) made the COMMON moderate case (already-claimed ~100-400)
    // cost ~225-275us (forced into a fallback the upfront check didn't catch
    // and completion would have avoided entirely) instead of the ~15-70us it
    // gets by simply being allowed to finish. A large budget's own worst case
    // (a skip-prefix near `n/2`, right at the edge) is bounded and modest by
    // comparison (~30-50% over a clean fallback, not multiples of it) — an
    // acceptable price for a case the upfront check should catch almost
    // always in practice anyway.
    let failure_budget = max_group;
    let mut failed_count: usize = 0;
    let mut block_target = row_budget.saturating_mul(4).max(16);
    loop {
        if visited_to >= n {
            break;
        }
        let mut end = block_target.clamp(visited_to, n);
        // Extend to the next object-id group boundary so a tie is never split
        // across blocks (the early-stop check below requires a COMPLETE group).
        if end < n && end > 0 {
            let boundary_obj = obj_id_at(end - 1);
            while end < n && obj_id_at(end) == boundary_obj {
                end += 1;
            }
        }
        if end - visited_to > max_group {
            return Ok(None);
        }
        for i in visited_to..end {
            let spo = scan.to_spo(&rows[logical(i)]);
            let hub_id = spo[0];
            let order_id = spo[2];
            // Only single-valued probes are admitted in this slice.
            // General Cartesian expansion remains with the existing evaluator.
            let mut row_ids: SmallVec<[Id; 8]> = SmallVec::from_slice(&[hub_id, order_id]);
            let mut failed = false;
            for op in &mut other_pats {
                if !prepare_other(graph, op, rows.len()) {
                    return Ok(None);
                }
                let prepared = op.prepared.as_ref().expect("successful preparation");
                // Binary-search the PRECOMPUTED, plain-`Id` subject list for
                // this pattern (built once, above) — a trivial integer
                // compare per step, no `to_spo` reconstruction during the
                // search itself (that only happens below, per ACTUAL match,
                // not per comparison step — see `subject_ids`'s doc comment).
                let start = prepared.subject_ids.partition_point(|&id| id < hub_id);
                let stop = start + prepared.subject_ids[start..].partition_point(|&id| id == hub_id);
                if start == stop {
                    failed = true;
                    break;
                }
                let Some(_) = &op.obj_var else {
                    continue; // `obj_const` — existence-only, no column added
                };
                let op_rows: &[[Id; 3]] = prepared.scan.rows.as_ref();
                let match_count = stop - start;
                if match_count != 1 {
                    return Ok(None);
                }
                row_ids.push(prepared.scan.to_spo(&op_rows[start])[2]);
            }
            if failed {
                failed_count += 1;
                if failed_count > failure_budget {
                    return Ok(None);
                }
                continue;
            }
            emit(&row_ids, &mut collected);
        }
        visited_to = end;
        if collected.len() >= row_budget {
            break; // a COMPLETE group boundary was just finished — safe to stop
        }
        block_target = block_target.saturating_mul(4).max(visited_to + 1);
    }

    // An empty walk may never visit later patterns. Preserve their
    // unproved static-sort/cardinality exclusions through the existing fallback.
    if other_pats.iter().any(|op| op.prepared.is_none()) {
        return Ok(None);
    }
    let mut result = Bindings { vars: out_vars, rows: collected, sorted_by: None };
    let use_topk = result.rows.len() > row_budget;
    order_bindings(graph, local, &mut result, expression, if use_topk { Some(row_budget) } else { None })?;
    Ok(Some(result))
}


/// (sq-7d3dj.30.4) Attempts the DISTINCT-projection loose skip-scan for the
/// pattern under a `Distinct`.
///
/// Returns `Some(bindings)` when `inner` is `Project{[?p]}{ body }` (a SINGLE projected
/// variable) and every UNION branch of `body` is a filter-free BGP that admits direct
/// enumeration of the DISTINCT `?p` values from an existing permutation sorted by `?p`
/// (a loose/skip scan — the general form of qlever's pattern trick over the six
/// permutations, NO new index). Returns `None` on ANY other shape, so the caller falls
/// back to the full materialise-then-dedup path. Because the enumerated set is exactly
/// `{ v : ∃ a solution of the branch with ?p = v }`, unioned across branches, the result
/// is DISTINCT-set equivalent to the fallback by construction.
///
/// The zk completeness witness needs the whole PRE-distinct input set, so the pushdown
/// declines while the recorder is armed.
pub(super) fn try_distinct_pushdown(
    graph: &Graph,
    _local: &mut LocalVocab,
    inner: &GraphPattern,
) -> Result<Option<Bindings>, String> {
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return Ok(None);
    }
    // An empty-default dataset view short-circuits the BGP at eval time; stay on the
    // general path so the view semantics are applied uniformly.
    if view::default_is_empty() {
        return Ok(None);
    }
    // Shape gate: DISTINCT over a single-variable projection.
    let (pvar, body) = match inner {
        GraphPattern::Project { inner, variables } if variables.len() == 1 => {
            (&variables[0], inner.as_ref())
        }
        _ => return Ok(None),
    };

    let mut branches: Vec<&GraphPattern> = Vec::new();
    collect_union_branches(body, &mut branches);

    // Global first-seen dedup across branches — a value found in an earlier branch is
    // never re-enumerated (the "already-seen" set of the loose scan).
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut out_ids: Vec<Id> = Vec::new();
    let mut total_scanned = 0usize;
    // (sq-7d3dj.30.10) Cache the sorted anchor membership set across branches:
    // UNION branches frequently share the same anchor pattern (SP2Bench q09's two branches
    // both anchor on `?person rdf:type foaf:Person`), so building the ~20k-member set once
    // instead of per-branch removes the second full anchor scan+sort. Keyed by the anchor's
    // prepared id-pattern + join position, which fully determines the set.
    let mut anchor_cache: AnchorCache = FxHashMap::default();
    // (sq-jnb1e) Opt-in predicate-incidence cache: shared across branches so the two
    // q09 semijoins (subject- and object-join) each build their incidence set at most once.
    #[cfg(feature = "cs-anchor-incidence")]
    let mut incidence_cache: IncidenceCache = FxHashMap::default();

    for branch in &branches {
        match branch_distinct_values(
            graph,
            branch,
            pvar,
            &seen,
            &mut anchor_cache,
            #[cfg(feature = "cs-anchor-incidence")]
            &mut incidence_cache,
        )? {
            Some((new_ids, scanned)) => {
                total_scanned = total_scanned.saturating_add(scanned);
                for id in new_ids {
                    if seen.insert(id) {
                        out_ids.push(id);
                    }
                }
            }
            // Any branch we cannot fully enumerate → decline the whole pushdown so the
            // fallback produces the (equivalent) answer for the entire pattern.
            None => return Ok(None),
        }
    }

    distinct_pushdown::record(out_ids.len(), total_scanned);
    let rows: Vec<Row> = out_ids.iter().map(|&id| std::iter::once(id).collect()).collect();
    Ok(Some(Bindings { vars: vec![pvar.clone()], rows, sorted_by: None }))
}

/// Flattens a (possibly nested) `UNION` into its branch patterns; a non-union pattern is
/// a single branch. DISTINCT over `A UNION B` equals DISTINCT over the union of the
/// per-branch value sets, so each branch is enumerated independently.
pub(super) fn collect_union_branches<'a>(p: &'a GraphPattern, out: &mut Vec<&'a GraphPattern>) {
    match p {
        GraphPattern::Union { left, right } => {
            collect_union_branches(left, out);
            collect_union_branches(right, out);
        }
        other => out.push(other),
    }
}

/// The canonical triple positions (0=subject, 1=predicate, 2=object) at which `var`
/// appears in a prepared pattern's `pos_vars`.
pub(super) fn var_positions_of(pos_vars: &[Option<Variable>; 3], var: &Variable) -> Vec<usize> {
    (0..3).filter(|&i| pos_vars[i].as_ref() == Some(var)).collect()
}

/// The single query variable shared by both patterns' `pos_vars`, or `None` unless
/// EXACTLY one distinct variable is shared (the clean single-join-variable case).
pub(super) fn single_shared_var<'a>(
    a: &'a [Option<Variable>; 3],
    b: &[Option<Variable>; 3],
) -> Option<&'a Variable> {
    let mut shared: Vec<&Variable> = Vec::new();
    for v in a.iter().flatten() {
        if b.iter().flatten().any(|w| w == v) && !shared.contains(&v) {
            shared.push(v);
        }
    }
    if shared.len() == 1 {
        Some(shared[0])
    } else {
        None
    }
}

/// The built permutation whose canonical column order is exactly `order`, if any.
pub(super) fn perm_for_order(order: [usize; 3]) -> Option<sparq_core::store::Perm> {
    sparq_core::store::Perm::ALL.into_iter().find(|p| p.order() == order)
}

/// (sq-7d3dj.30.10) The distinct ids at canonical position `target_pos` of
/// `id_pat`'s matching triples, returned SORTED ASCENDING and deduped. The membership set
/// for a semi-join, returned as a sorted `Vec<Id>` (rather than a hash set):
/// it lets the per-predicate existence check gallop the anchor (skipping large
/// disjoint id runs on the anchor side) instead of a per-join-value hash probe, and it
/// avoids the `HashMap` rehash cost that dominated the anchor build for a 20k-member
/// anchor (SP2Bench q09 profile). When the chosen permutation already yields
/// `target_pos` as its first unbound (leading) column the rows arrive grouped, so we
/// skip-scan the distinct values directly in sorted order (no post-sort); otherwise we
/// collect then `sort_unstable`. `None` declines under budget pressure so the caller
/// falls back to the general path.
pub(super) fn collect_var_ids_sorted(graph: &Graph, id_pat: &IdPattern, target_pos: usize) -> Option<Vec<Id>> {
    // A permutation sorted by `target_pos` as its first UNBOUND column yields the join
    // values already grouped and ascending — enumerate the distinct run heads directly.
    let scan = graph.store.scan_sorted(id_pat, target_pos);
    let order = scan.perm.order();
    let k = order.iter().position(|&c| c == target_pos)?;
    let rows = scan.rows.as_ref();
    if order.into_iter().find(|&c| id_pat[c].is_none()) == Some(target_pos) {
        let mut out: Vec<Id> = Vec::new();
        let mut i = 0usize;
        while i < rows.len() {
            if budget::exhausted(out.len()) {
                return None;
            }
            let v = rows[i][k];
            out.push(v);
            i += rows[i..].partition_point(|r| r[k] <= v).max(1);
        }
        // `out` is strictly increasing by construction (each pushed value exceeds the
        // previous block's) — already sorted + deduped.
        return Some(out);
    }
    // Not sorted by `target_pos`: collect then sort+dedup.
    let mut out: Vec<Id> = Vec::with_capacity(rows.len());
    for (n, r) in rows.iter().enumerate() {
        if n & 4095 == 0 && budget::exhausted(out.len()) {
            return None;
        }
        out.push(r[k]);
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// (sq-7d3dj.30.10) Existence check for the loose semi-join: does the
/// join-sorted `block` (join value at column `jk`) share ANY id with `anchor`? Drives from
/// whichever side is shorter and early-exits on the first common id:
/// * anchor ≥ block: gallop the block's DISTINCT join values, O(1) hash-probe each into
///   the anchor (the original per-value probe — best when the block is short or hits fast);
/// * anchor <  block: walk the (shorter) sorted anchor, binary-searching each member into
///   the block's join column over a monotonically shrinking window (`O(|anchor|·log|block|)`
///   — best when a large no-hit block is checked against a smaller anchor).
///
/// Both strategies are exact and order-independent; the choice only affects work, never the
/// boolean. `examined` (distinct values touched) feeds the diagnostic scan counter.
pub(super) fn block_intersects_anchor(block: &[[Id; 3]], jk: usize, anchor: &AnchorSet) -> (bool, usize) {
    // Cost model: the hash gallop touches ~`distinct(block)` values at O(1) each; the
    // anchor walk does `|anchor|` binary searches at O(log|block|) each. Prefer the anchor
    // walk only when it is strictly cheaper — i.e. when the anchor is small enough that
    // `|anchor|·log|block|` beats the block's distinct-value count (bounded by `block.len()`).
    // A conservative proxy: anchor smaller than `block.len() / ilog2(block.len())`.
    let block_len = block.len();
    let log_block = usize::BITS as usize - block_len.leading_zeros() as usize; // ~ceil(log2)
    let anchor_driven = block_len > 0 && anchor.sorted.len().saturating_mul(log_block.max(1)) < block_len;
    if !anchor_driven {
        // Gallop the block's distinct join values; O(1) hash membership.
        let mut examined = 0usize;
        let mut bi = 0usize;
        while bi < block.len() {
            let jv = block[bi][jk];
            examined += 1;
            if anchor.hash.contains(&jv) {
                return (true, examined);
            }
            bi += block[bi..].partition_point(|r| r[jk] <= jv).max(1);
        }
        (false, examined)
    } else {
        // Walk the shorter sorted anchor; binary-search each member into the block.
        let mut examined = 0usize;
        let mut lo = 0usize; // block window shrinks monotonically (both sorted ascending)
        for &av in &anchor.sorted {
            examined += 1;
            let rel = block[lo..].partition_point(|r| r[jk] < av);
            let at = lo + rel;
            if at < block.len() && block[at][jk] == av {
                return (true, examined);
            }
            lo = at;
            if lo >= block.len() {
                break;
            }
        }
        (false, examined)
    }
}

/// Enumerates the DISTINCT ids at canonical position `target_pos` via a loose/skip scan
/// over a permutation sorted by that column (galloping past each value's block with a
/// binary search rather than touching every row). Returns `(values, rows_touched)`, or
/// `None` when no built permutation sorts the matching range by `target_pos` (conservative
/// decline).
pub(super) fn skipscan_distinct(graph: &Graph, id_pat: &IdPattern, target_pos: usize) -> Option<(Vec<Id>, usize)> {
    let scan = graph.store.scan_sorted(id_pat, target_pos);
    let order = scan.perm.order();
    // The range's rows are sorted by `target_pos` only when it is the FIRST unbound
    // column of the chosen permutation's order.
    if order.into_iter().find(|&c| id_pat[c].is_none()) != Some(target_pos) {
        return None;
    }
    let k = order.iter().position(|&c| c == target_pos)?;
    let rows = scan.rows.as_ref();
    let mut out: Vec<Id> = Vec::new();
    let mut scanned = 0usize;
    let mut i = 0usize;
    while i < rows.len() {
        if budget::exhausted(out.len()) {
            return None;
        }
        let v = rows[i][k];
        out.push(v);
        scanned += 1;
        // Gallop to the end of this value's contiguous block (all rows[i..] have
        // `col >= v`, so `col <= v` is exactly the v-block).
        let block_len = rows[i..].partition_point(|r| r[k] <= v).max(1);
        i += block_len;
    }
    Some((out, scanned))
}

/// Returns `true` when the same query variable occupies more than one of the subject,
/// predicate, or object slots in `tp` (e.g. `?x ?p ?x`). The fallback path enforces
/// positional equality for repeated variables via `build_row`; the skip-scan helpers do
/// not, so we must decline to push down any branch that contains such a pattern.
pub(super) fn has_intra_triple_repeated_var(tp: &TriplePattern) -> bool {
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

/// Returns `true` when `tp` carries a nested RDF 1.2 quoted-triple term (`<<s p o>>` /
/// `<<( s p o )>>`) in its subject or object slot. Such a term, when it embeds a variable, is
/// decomposed by the general BGP planner before pattern preparation; the DISTINCT skip-scan
/// path prepares the raw pattern, so `prepare_pattern` would ERROR on the embedded variable
/// instead of declining. The pushdown must decline any branch containing one (the predicate
/// slot is a `NamedNodePattern`, which cannot be a triple term, so only S/O are checked).
/// (sq-7d3dj.30.4)
pub(super) fn has_quoted_triple_term(tp: &TriplePattern) -> bool {
    matches!(tp.subject, TermPattern::Triple(_)) || matches!(tp.object, TermPattern::Triple(_))
}

/// The DISTINCT projected-variable ids contributed by one UNION branch, together with the
/// permutation rows the skip scan touched. Returns only values NOT already in `seen`
/// (they are added there by the caller). `None` = shape the pushdown cannot enumerate.
pub(super) fn branch_distinct_values(
    graph: &Graph,
    branch: &GraphPattern,
    pvar: &Variable,
    seen: &FxHashSet<Id>,
    anchor_cache: &mut AnchorCache,
    #[cfg(feature = "cs-anchor-incidence")] incidence_cache: &mut IncidenceCache,
) -> Result<Option<(Vec<Id>, usize)>, String> {
    if !is_conjunctive(branch) {
        return Ok(None);
    }
    let mut patterns: Vec<TriplePattern> = Vec::new();
    let mut filters: Vec<Expression> = Vec::new();
    flatten_conjunction(branch, &mut patterns, &mut filters);
    if !filters.is_empty() {
        return Ok(None);
    }
    // Decline when any pattern has a variable repeated across S/P/O positions
    // (e.g. `?x ?p ?x`). The fallback enforces positional equality via build_row; without
    // this guard the skip scan would over-approximate (include predicates that have no
    // self-loop triple). Counterexample: data {ex:a ex:knows ex:a. ex:b ex:likes ex:c.},
    // query SELECT DISTINCT ?p WHERE { ?x ?p ?x } — correct={ex:knows}, pushdown
    // (unguarded)={ex:knows, ex:likes}.
    if patterns.iter().any(has_intra_triple_repeated_var) {
        return Ok(None);
    }
    // (sq-7d3dj.30.4) Decline when any pattern embeds an RDF 1.2 quoted-triple
    // term (e.g. `<<?s ?p "o">> ?p2 ?z`) in a subject/object slot. A quoted term that
    // carries a variable is decomposed by the general BGP planner (`extract_quoted_constraints`
    // -> `quoted_relation`) BEFORE pattern preparation runs; the skip-scan path prepares the
    // raw pattern, so `prepare_pattern` would try to resolve the whole quoted term to one
    // ground id and ERROR ("variable where a term was expected") rather than fall back. We
    // must always DECLINE (never error) so the general path produces the equivalent answer
    // (W3C sparql12 eval-triple-terms/pattern-10). See `has_quoted_triple_term`.
    if patterns.iter().any(has_quoted_triple_term) {
        return Ok(None);
    }
    match patterns.len() {
        1 => single_pattern_distinct(graph, &patterns[0], pvar, seen),
        2 => two_pattern_distinct(
            graph,
            &patterns,
            pvar,
            seen,
            anchor_cache,
            #[cfg(feature = "cs-anchor-incidence")]
            incidence_cache,
        ),
        _ => Ok(None),
    }
}

/// Single-pattern branch: every distinct `?p` value in the pattern's range is a solution,
/// so enumerate them directly via the skip scan.
pub(super) fn single_pattern_distinct(
    graph: &Graph,
    tp: &TriplePattern,
    pvar: &Variable,
    seen: &FxHashSet<Id>,
) -> Result<Option<(Vec<Id>, usize)>, String> {
    let (id_pat, pos_vars, unsat) = prepare_pattern(graph, tp)?;
    if unsat {
        return Ok(Some((Vec::new(), 0)));
    }
    let ppos = match var_positions_of(&pos_vars, pvar).as_slice() {
        [p] => *p,
        _ => return Ok(None),
    };
    let (vals, scanned) = match skipscan_distinct(graph, &id_pat, ppos) {
        Some(x) => x,
        None => return Ok(None),
    };
    let new: Vec<Id> = vals.into_iter().filter(|v| !seen.contains(v)).collect();
    Ok(Some((new, scanned)))
}

/// Two-pattern (anchor + probe) branch joined on a single variable: `?p` lives in exactly
/// one pattern (the probe); the other (the anchor) constrains the join variable. Enumerate
/// distinct `?p` from the probe and keep only those whose probe row set intersects the
/// anchor's join-variable set (a loose semi-join existence check).
pub(super) fn two_pattern_distinct(
    graph: &Graph,
    patterns: &[TriplePattern],
    pvar: &Variable,
    seen: &FxHashSet<Id>,
    anchor_cache: &mut AnchorCache,
    #[cfg(feature = "cs-anchor-incidence")] incidence_cache: &mut IncidenceCache,
) -> Result<Option<(Vec<Id>, usize)>, String> {
    let (idp0, pv0, uns0) = prepare_pattern(graph, &patterns[0])?;
    let (idp1, pv1, uns1) = prepare_pattern(graph, &patterns[1])?;
    let in0 = var_positions_of(&pv0, pvar);
    let in1 = var_positions_of(&pv1, pvar);
    // `?p` must appear EXACTLY ONCE, in exactly one of the two patterns (that is the probe).
    match (in0.as_slice(), in1.as_slice()) {
        ([ppos], []) => probe_anchor_distinct(
            graph,
            (&idp0, &pv0, uns0),
            (&idp1, &pv1, uns1),
            *ppos,
            seen,
            anchor_cache,
            #[cfg(feature = "cs-anchor-incidence")]
            incidence_cache,
        ),
        ([], [ppos]) => probe_anchor_distinct(
            graph,
            (&idp1, &pv1, uns1),
            (&idp0, &pv0, uns0),
            *ppos,
            seen,
            anchor_cache,
            #[cfg(feature = "cs-anchor-incidence")]
            incidence_cache,
        ),
        _ => Ok(None),
    }
}

/// Prepared-pattern triple `(id_pat, pos_vars, unsat)` passed by reference.
pub(super) type PreparedRef<'a> = (&'a IdPattern, &'a [Option<Variable>; 3], bool);

/// (sq-7d3dj.30.10) The anchor membership set, in BOTH forms needed by the
/// per-predicate existence probe: a `hash` set for O(1) membership, and the same ids
/// `sorted` ascending so a large block can be clipped to the anchor's id window and the
/// smaller side can drive a galloping intersection.
pub(super) struct AnchorSet {
    pub(super) hash: FxHashSet<Id>,
    pub(super) sorted: Vec<Id>,
}

/// (sq-7d3dj.30.10) Per-pushdown cache of anchor membership sets, keyed by the
/// anchor's prepared id-pattern plus its join position (which together fully determine the
/// set). Shared across a DISTINCT pushdown's UNION branches so a repeated anchor
/// (e.g. SP2Bench q09's shared `?person rdf:type foaf:Person`) is materialised once.
pub(super) type AnchorCache = FxHashMap<(IdPattern, usize), std::rc::Rc<AnchorSet>>;

/// The anchor membership set for `(anchor_ip, anchor_jpos)`, memoised in `cache`. Returns
/// `None` under budget pressure (the pushdown then declines). The set is wrapped in `Rc`
/// so a cache hit is a cheap clone, not a re-materialisation.
pub(super) fn cached_anchor_ids(
    graph: &Graph,
    anchor_ip: &IdPattern,
    anchor_jpos: usize,
    cache: &mut AnchorCache,
) -> Option<std::rc::Rc<AnchorSet>> {
    let key = (*anchor_ip, anchor_jpos);
    if let Some(v) = cache.get(&key) {
        return Some(v.clone());
    }
    let sorted = collect_var_ids_sorted(graph, anchor_ip, anchor_jpos)?;
    let hash: FxHashSet<Id> = sorted.iter().copied().collect();
    let set = std::rc::Rc::new(AnchorSet { hash, sorted });
    cache.insert(key, set.clone());
    Some(set)
}

/// (sq-jnb1e) Per-pushdown cache of anchor PREDICATE-INCIDENCE sets, keyed by the
/// anchor membership set (identified by the anchor's prepared id-pattern + join position) and
/// the probe's canonical JOIN position (0=subject, 2=object; a predicate never joins). The
/// value is `Some(set)` where `set` is exactly the predicates that relate SOME anchor member
/// at that position, or `None` when the set could not be computed (budget / overlay / no built
/// permutation) — cached so the two q09 branches never recompute the same incidence.
#[cfg(feature = "cs-anchor-incidence")]
pub(super) type IncidenceCache = FxHashMap<(IdPattern, usize, usize), Option<std::rc::Rc<FxHashSet<Id>>>>;

/// (sq-jnb1e) The set of predicates that relate ANY anchor member at the probe's
/// canonical join position `probe_jpos` (0=subject or 2=object) — a characteristic-set-style
/// incidence summary. Computed by ONE range-restricted scan over the anchor's id window: the
/// permutation sorted by `probe_jpos` groups triples by that column, so restricting to
/// `[a_min, a_max]` (every anchor member lies in that window) and, for each row whose join
/// value is an actual anchor member, recording its predicate, yields exactly
/// `{ predicate(t) : t is a triple with t[probe_jpos] ∈ anchor }`.
///
/// SOUNDNESS. This set is EXACT on the base index: a predicate P is in it iff some triple uses
/// P with an anchor member at `probe_jpos`, i.e. iff the (anchor ⋈ probe-on-P) branch is
/// non-empty. So `P ∉ set` ⇒ that branch contributes no solution ⇒ P can be pruned WITHOUT
/// scanning P's block, and `P ∈ set` still takes the exact per-block scan. Pruning only
/// provably-empty checks makes the DISTINCT answer identical to the block-scan path.
///
/// Returns `None` (caller keeps the exact block scan for every P) when:
/// * the graph carries a pending-update overlay — the derived set is built against the base
///   index only, and incremental maintenance is out of scope, so we DECLINE conservatively
///   rather than risk a stale prune;
/// * `probe_jpos` is the predicate slot (1) — a predicate never joins to the anchor here;
/// * no permutation sorted by `probe_jpos` is built (e.g. the compact wasm index);
/// * the scan exceeds the query budget.
#[cfg(feature = "cs-anchor-incidence")]
pub(super) fn anchor_probe_incidence(
    graph: &Graph,
    anchor: &AnchorSet,
    probe_jpos: usize,
) -> Option<FxHashSet<Id>> {
    // Conservative invalidation: a delta-overlay means the base-index incidence may be stale
    // (an inserted triple could add a predicate; a deleted one could remove the last member
    // relation). We do not maintain the set incrementally, so decline and let the exact scan
    // (which reads the overlay-merged rows) answer every candidate.
    if graph.store.has_overlay() {
        return None;
    }
    if probe_jpos == 1 || anchor.sorted.is_empty() {
        return None;
    }
    let (a_min, a_max) = (anchor.sorted[0], anchor.sorted[anchor.sorted.len() - 1]);
    // A permutation whose FIRST column is `probe_jpos` so the anchor id window is one
    // contiguous range and the predicate sits at a fixed column. Bind nothing; sort by the
    // join column. `scan_sorted` picks such a perm when built; if the chosen perm is not
    // actually led by `probe_jpos`, we cannot range-restrict soundly — decline.
    let all_unbound: IdPattern = [None, None, None];
    let scan = graph.store.scan_sorted(&all_unbound, probe_jpos);
    let order = scan.perm.order();
    if order[0] != probe_jpos {
        return None;
    }
    // Predicate lives at whichever column of this perm is canonical position 1.
    let pk = order.iter().position(|&c| c == 1)?;
    let jk = 0usize; // join column is the leading column (== probe_jpos)
    let rows = scan.rows.as_ref();
    // Restrict to the anchor id window `[a_min, a_max]` with two binary searches: rows outside
    // it can never carry an anchor member at the join column, so they cannot contribute a
    // predicate to the incidence set.
    let lo = rows.partition_point(|r| r[jk] < a_min);
    let hi = rows.partition_point(|r| r[jk] <= a_max);
    let window = &rows[lo..hi];
    let mut inc: FxHashSet<Id> = FxHashSet::default();
    for (n, r) in window.iter().enumerate() {
        if n & 4095 == 0 && budget::exhausted(inc.len()) {
            return None;
        }
        // Only rows whose join value is an ACTUAL anchor member contribute (the window can
        // include non-member ids that merely fall inside `[a_min, a_max]`).
        if anchor.hash.contains(&r[jk]) {
            inc.insert(r[pk]);
        }
    }
    Some(inc)
}

/// (sq-jnb1e) The predicate-incidence set for `(anchor, probe_jpos)`, memoised in
/// `cache`. Keyed by the anchor identity (`anchor_ip` + `anchor_jpos`) and the probe join
/// position, so q09's two branches share one computation per position. The cached value is an
/// `Option` (a computed-and-failed decline is cached too, so a second branch on the same
/// position does not retry a scan that already declined).
#[cfg(feature = "cs-anchor-incidence")]
pub(super) fn cached_incidence(
    graph: &Graph,
    anchor_ip: &IdPattern,
    anchor_jpos: usize,
    anchor: &AnchorSet,
    probe_jpos: usize,
    cache: &mut IncidenceCache,
) -> Option<std::rc::Rc<FxHashSet<Id>>> {
    let key = (*anchor_ip, anchor_jpos, probe_jpos);
    if let Some(v) = cache.get(&key) {
        return v.clone();
    }
    let computed = anchor_probe_incidence(graph, anchor, probe_jpos).map(std::rc::Rc::new);
    cache.insert(key, computed.clone());
    computed
}

/// (sq-7d3dj.30.10) The "pattern-trick" strategy for the loose semi-join: iterate
/// the ANCHOR members and, for each member `m`, enumerate the distinct `?p` (at `ppos`) on
/// probe triples whose join column (`jpos`) equals `m` — accumulating a running result set
/// that terminates the instant it covers the whole `?p` UNIVERSE (the distinct `?p` in the
/// probe's range ignoring the join, a superset the answer can never exceed). Returns the new
/// `?p` values (those not already in `seen`) plus a diagnostic touched-row count, or `None`
/// to DECLINE — in which case the caller falls back to the per-`?p`-block existence scan.
///
/// Correctness: the union over all anchor members `m` of `{ ?p : ∃ probe triple with
/// jpos=m }` is EXACTLY `{ ?p : ∃ a solution of the (anchor ⋈ probe) branch }` — binding the
/// join column to each anchor member and taking the union is the definition of the semi-join
/// projected onto `?p`. The universe early-exit is sound because the collected set is
/// monotonically growing and bounded above by the universe, so once they are equal no further
/// member can add a value. Declines when the required per-member permutation is not built or
/// when a per-member scan cannot skip-enumerate `?p` (so the general block scan answers it).
pub(super) fn maybe_anchor_driven(
    graph: &Graph,
    probe_ip: &IdPattern,
    ppos: usize,
    jpos: usize,
    anchor: &AnchorSet,
    seen: &FxHashSet<Id>,
) -> Result<Option<(Vec<Id>, usize)>, String> {
    // COST GATE: the per-member scan touches only the anchor's incident probe triples, so it
    // is a clear win when the anchor is SMALL — then we sweep a handful of members instead of
    // every candidate `?p`-block. For a LARGE anchor (q09's 20,602 persons) the per-member
    // machinery dominates, so we decline here and let the caller's cost-aware block scan run.
    // The threshold is a strategy choice only — either path returns the identical `?p` set.
    if anchor.sorted.len() > ANCHOR_DRIVEN_MAX {
        return Ok(None);
    }
    anchor_driven_distinct(graph, probe_ip, ppos, jpos, anchor, seen)
}

/// The largest anchor cardinality for which the per-member ("pattern trick") enumeration is
/// preferred over the per-`?p`-block existence scan. Above it, sweeping every anchor member
/// costs more than the block scan (measured: SP2Bench q09's 20k-person anchor is faster on
/// the block scan). A strategy threshold only — never affects the computed answer.
pub(super) const ANCHOR_DRIVEN_MAX: usize = 256;

/// Per-member ("pattern trick") enumeration: for each anchor member `m`, bind the join column
/// to `m` and skip-enumerate the distinct `?p`, unioning across members and stopping once the
/// `?p` universe is covered. Declines (→ block-scan fallback) when a per-member scan cannot
/// skip-enumerate `?p` for the bound join position.
pub(super) fn anchor_driven_distinct(
    graph: &Graph,
    probe_ip: &IdPattern,
    ppos: usize,
    jpos: usize,
    anchor: &AnchorSet,
    seen: &FxHashSet<Id>,
) -> Result<Option<(Vec<Id>, usize)>, String> {
    // The `?p` universe (distinct `?p` in the probe range, ignoring the join) — a cheap
    // superset the answer can never exceed, so once we have collected it we can stop reading.
    let (universe_vals, _) = match skipscan_distinct(graph, probe_ip, ppos) {
        Some(x) => x,
        None => return Ok(None),
    };
    let universe_len = universe_vals.len();
    if universe_len == 0 {
        return Ok(Some((Vec::new(), 0)));
    }
    let mut found: FxHashSet<Id> = FxHashSet::default();
    let mut out: Vec<Id> = Vec::new();
    let mut scanned = 0usize;
    for (idx, &m) in anchor.sorted.iter().enumerate() {
        if idx & 63 == 0 && budget::exhausted(out.len()) {
            return Ok(None);
        }
        // Bind the join column to this anchor member; keep the probe's other positions.
        let mut member_pat = *probe_ip;
        member_pat[jpos] = Some(m);
        let (vals, s) = match skipscan_distinct(graph, &member_pat, ppos) {
            Some(x) => x,
            None => return Ok(None),
        };
        scanned = scanned.saturating_add(s);
        for v in vals {
            if found.insert(v) {
                if !seen.contains(&v) {
                    out.push(v);
                }
                // Whole `?p` universe covered → no later member can add anything. Stop.
                if found.len() == universe_len {
                    return Ok(Some((out, scanned)));
                }
            }
        }
    }
    Ok(Some((out, scanned)))
}

/// The loose semi-join for `{ anchor, probe }`: `probe` binds `?p` (at `ppos`) and the
/// join variable; `anchor` constrains the join variable. Enumerates distinct `?p` from a
/// permutation ordered `[.., P, J, ..]` (so each `?p`-block is sorted by the join variable)
/// and keeps a `?p` iff its block contains a join-variable id present in the anchor set.
pub(super) fn probe_anchor_distinct(
    graph: &Graph,
    probe: PreparedRef<'_>,
    anchor: PreparedRef<'_>,
    ppos: usize,
    seen: &FxHashSet<Id>,
    anchor_cache: &mut AnchorCache,
    #[cfg(feature = "cs-anchor-incidence")] incidence_cache: &mut IncidenceCache,
) -> Result<Option<(Vec<Id>, usize)>, String> {
    let (probe_ip, probe_pv, probe_uns) = probe;
    let (anchor_ip, anchor_pv, anchor_uns) = anchor;
    if probe_uns || anchor_uns {
        return Ok(Some((Vec::new(), 0)));
    }
    // Exactly one shared (join) variable, which must not be the projected variable.
    let jvar = match single_shared_var(probe_pv, anchor_pv) {
        Some(v) => v,
        None => return Ok(None),
    };
    let jpos = match var_positions_of(probe_pv, jvar).as_slice() {
        [p] => *p,
        _ => return Ok(None),
    };
    let anchor_jpos = match var_positions_of(anchor_pv, jvar).first() {
        Some(&p) => p,
        None => return Ok(None),
    };
    if jpos == ppos {
        return Ok(None);
    }
    // The anchor's join-variable membership set (hash + sorted), shared across the
    // pushdown's UNION branches (SP2Bench q09's two branches share the same anchor).
    let a = match cached_anchor_ids(graph, anchor_ip, anchor_jpos, anchor_cache) {
        Some(s) => s,
        None => return Ok(None),
    };
    if a.sorted.is_empty() {
        // Empty anchor → the join is empty → no `?p` qualifies.
        return Ok(Some((Vec::new(), 0)));
    }
    // (sq-7d3dj.30.10) ANCHOR-DRIVEN predicate enumeration (the "pattern trick"):
    // walk the probe range grouped by join value and only descend into the ANCHOR blocks'
    // `?p` values, terminating once the `?p` universe is covered. This wins when the join
    // axis is LOW-cardinality (few distinct join values → few blocks to skip): a bound anchor
    // member set is then cheaper to sweep than the candidate `?p`-blocks. It is NOT a
    // universal win — when the distinct join values are as numerous as the `?p`-blocks
    // (SP2Bench q09: ~tens of thousands of distinct subjects/objects) walking every join
    // block costs as much as the block existence scan — so we gate it on the join axis being
    // low-cardinality relative to the `?p` universe, else fall through to the (cost-aware,
    // clipped) block scan below. Both paths return the identical `?p` set.
    if let Some((new, scanned)) = maybe_anchor_driven(graph, probe_ip, ppos, jpos, &a, seen)? {
        return Ok(Some((new, scanned)));
    }
    // We need a permutation ordered `[.., P, J, third]` so a `?p`-block is sorted by the
    // join variable (bounding the per-block existence scan). `scan_sorted(ppos)` only
    // pins the FIRST unbound column, so choose the exact permutation ourselves.
    let third = 3 - ppos - jpos;
    let want = perm_for_order([ppos, jpos, third]).ok_or("no such permutation")?;
    let scan = match graph.store.scan_perm(probe_ip, want) {
        Some(s) => s,
        // Permutation not built (e.g. the compact wasm index) or the pattern's bound
        // positions are not a prefix in it → decline (fall back to the full path).
        None => return Ok(None),
    };
    // `want.order() == [ppos, jpos, third]`, so P is column 0 and J is column 1, but only
    // when every bound position precedes them; `scan_perm` guarantees the bound prefix, and
    // both P and J are variables here, so this holds.
    let (kp, jk) = (0usize, 1usize);
    debug_assert_eq!(scan.perm.order()[kp], ppos);
    debug_assert_eq!(scan.perm.order()[jk], jpos);
    let rows = scan.rows.as_ref();
    // (sq-jnb1e) OPT-IN characteristic-set anchor-incidence prune. When the probe
    // pattern constrains ONLY the projected predicate and the join variable (every other
    // position free — the q09 shape `?subject ?predicate ?person` / `?person ?predicate
    // ?object`), a candidate predicate `pv_id` qualifies iff it relates SOME anchor member at
    // the join position — exactly the precomputed incidence set. A `pv_id` NOT in that set is
    // pruned by an O(1) membership test instead of the clip+gallop block scan. The prune is
    // sound ONLY when the probe has no OTHER bound position (else a predicate could relate an
    // anchor member on a DIFFERENT triple than the one satisfying the bound constraint); we
    // decline the incidence set otherwise and every candidate takes the exact scan.
    #[cfg(feature = "cs-anchor-incidence")]
    let incidence: Option<std::rc::Rc<FxHashSet<Id>>> = {
        // "Bound only P and J" ⇔ the probe id-pattern binds no position (both P and J are
        // variables, so a bound slot would be the third position).
        let probe_only_pj = probe_ip.iter().all(|s| s.is_none());
        if anchor_incidence::enabled() && probe_only_pj {
            let inc = cached_incidence(graph, anchor_ip, anchor_jpos, &a, jpos, incidence_cache);
            if inc.is_some() {
                anchor_incidence::note_built();
            }
            inc
        } else {
            None
        }
    };
    let mut out: Vec<Id> = Vec::new();
    let mut scanned = 0usize;
    let mut i = 0usize;
    // `a.sorted` is non-empty (checked above) and sorted, so min/max are its ends.
    let (a_min, a_max) = (a.sorted[0], a.sorted[a.sorted.len() - 1]);
    while i < rows.len() {
        if budget::exhausted(out.len()) {
            return Ok(None);
        }
        let pv_id = rows[i][kp];
        let block_len = rows[i..].partition_point(|r| r[kp] <= pv_id).max(1);
        let block = &rows[i..i + block_len];
        i += block_len;
        // Already emitted by an earlier branch — no re-check needed.
        if seen.contains(&pv_id) {
            continue;
        }
        // (sq-jnb1e) Incidence prune: when the precomputed set is available and does
        // NOT contain this predicate, the (anchor ⋈ probe-on-`pv_id`) branch is provably empty
        // — skip the whole block WITHOUT the clip+gallop scan (one metadata consultation counts
        // as one unit of examination). A `pv_id` IN the set falls through to the exact block
        // scan below, so the emitted answer is bit-for-bit identical to the feature-off path.
        #[cfg(feature = "cs-anchor-incidence")]
        if let Some(inc) = incidence.as_ref() {
            if !inc.contains(&pv_id) {
                scanned += 1;
                anchor_incidence::note_pruned(1);
                continue;
            }
        }
        // The block is join-variable-sorted, so its join range is [first, last]. When that
        // range is DISJOINT from the anchor set's id range, no anchor member can occur in
        // it — reject the whole `?p` in O(1) (the common loose-scan win for value-typed
        // predicates whose objects never overlap the entity id range).
        let (bmin, bmax) = (block[0][jk], block[block.len() - 1][jk]);
        scanned += 1;
        if bmax < a_min || bmin > a_max {
            continue;
        }
        // (sq-7d3dj.30.10) CLIP the join-sorted block to the anchor's id window
        // `[a_min, a_max]` with two binary searches: every anchor member lies in that
        // window, so no join value OUTSIDE it can be a hit — dropping them cannot change
        // the answer. When the anchor entities occupy a narrow id band this discards the
        // bulk of a large no-hit block in O(log n); when they do not it is two cheap
        // binary searches and the full gallop below still runs.
        let cs = block.partition_point(|r| r[jk] < a_min);
        let ce = block.partition_point(|r| r[jk] <= a_max);
        let clipped = &block[cs..ce];
        // Existence: does any join value in the clipped block belong to the anchor set?
        // Drive from whichever side is shorter: gallop the clipped block's distinct values
        // with an O(1) hash probe when the block (post-clip) is the shorter side; otherwise
        // walk the sorted anchor and binary-search each member into the block. Early-exit
        // on the first common id. (sq-7d3dj.30.10)
        let (hit, examined) = block_intersects_anchor(clipped, jk, &a);
        scanned += examined;
        if hit {
            out.push(pv_id);
        }
    }
    Ok(Some((out, scanned)))
}

/// Evaluates a pattern with an early-termination row cap, when that is safe. The
/// contract every arm preserves (and the `Slice` caller relies on): the returned
/// rows are a sub-multiset of the full result, and a return of FEWER than `cap`
/// rows is the COMPLETE result. Under that contract a caller that observes only
/// `min(cap, ·)` rows (LIMIT) or emptiness (ASK evaluates as `LIMIT 1`) gets the
/// same answer as full evaluation — differentially asserted by
/// `tests/ask_early_exit.rs`. (sq-7d3dj.30.8)
///
/// Covered shapes: a single-pattern scan (optionally with a pushed-down sargable
/// numeric filter) under projection — the scan itself stops at `cap` rows; a
/// conjunctive multi-pattern BGP (+ FILTERs) — the block-driven join chain
/// (`eval_bgp_binary_capped`); UNION — left branch first, the right branch is not
/// evaluated when the left already satisfies the cap; OPTIONAL — capping the LEFT
/// side caps the output, since no left row is ever dropped; Join — a capped-left
/// probe joined through the SIP correlated path, declining when the probe misses;
/// BIND — row-count preserving. Everything else returns `None` and the caller falls
/// back to full evaluation — in particular DISTINCT / ORDER BY / aggregation need
/// the full result, and a bare FILTER over a non-conjunctive inner cannot count
/// capped rows soundly (a row may only count once it has passed every filter).
pub(super) fn try_capped(
    graph: &Graph,
    local: &mut LocalVocab,
    inner: &GraphPattern,
    cap: usize,
) -> Result<Option<Bindings>, String> {
    // zk-trace: early termination would consume only part of each scan range,
    // recording a TRUNCATED input set — but the completeness witness (the
    // linear-sweep circuit) must see the whole scan range. Disable the cap
    // while recording; the full path is result-equivalent (LIMIT is
    // order-insensitive without ORDER BY).
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return Ok(None);
    }
    if view::default_is_empty() {
        return Ok(None); // empty-default view: the general path short-circuits at the BGP
    }
    match inner {
        GraphPattern::Project { inner, variables } => {
            Ok(try_capped(graph, local, inner, cap)?.map(|b| project_bindings(b, variables)))
        }
        GraphPattern::Reduced { inner } => try_capped(graph, local, inner, cap),
        p if is_conjunctive(p) => {
            let mut patterns = Vec::new();
            let mut filters = Vec::new();
            flatten_conjunction(p, &mut patterns, &mut filters);
            if patterns.is_empty() {
                return Ok(None); // the unit relation: nothing to cap
            }
            if patterns.len() == 1 {
                let (pat_filters, residual) = split_sargable(graph, &patterns, &filters);
                if residual.is_empty() {
                    // Single pattern, every FILTER pushed into the scan: stop the
                    // SCAN itself at `cap` rows (the cheapest capped form).
                    let (id_pat, pos_vars, unsat) = prepare_pattern(graph, &patterns[0])?;
                    if unsat {
                        return Ok(Some(Bindings::unsorted(collect_vars(&patterns), vec![])));
                    }
                    let filt = pat_filters[0];
                    let sort_col = filt.map(|(c, _)| c);
                    return Ok(Some(scan_to_bindings(
                        graph,
                        &id_pat,
                        &pos_vars,
                        sort_col,
                        filt,
                        Some(cap),
                        #[cfg(feature = "semijoin-bitmap")]
                        None,
                    )));
                }
            }
            // Multi-pattern (or residual-FILTER) conjunctive shape: the block-driven
            // capped join chain.
            eval_bgp_binary_capped(graph, local, &patterns, &filters, cap)
        }
        GraphPattern::Union { left, right } => {
            // Left branch first; the right branch is only touched when the left did
            // not already satisfy the cap (the ASK short-circuit: a non-empty left
            // branch answers the query alone).
            let l = match try_capped(graph, local, left, cap)? {
                Some(b) => b,
                None => eval_graph_pattern(graph, local, left)?,
            };
            if l.rows.len() >= cap {
                // The rows are genuine union solutions (right-only variables are
                // unbound); align the header with the un-evaluated right branch's
                // in-scope variables, exactly as `union_bindings` would have.
                return Ok(Some(union_bindings(l, Bindings::unsorted(in_scope_vars(right), vec![]))));
            }
            // Fewer than `cap` rows ⇒ `l` is the COMPLETE left result (the
            // contract): the union needs only `cap - |l|` further rows.
            let r = match try_capped(graph, local, right, cap - l.rows.len())? {
                Some(b) => b,
                None => eval_graph_pattern(graph, local, right)?,
            };
            Ok(Some(union_bindings(l, r)))
        }
        GraphPattern::Join { left, right } => {
            let l = match try_capped(graph, local, left, cap)? {
                Some(b) => b,
                None => return Ok(None),
            };
            // Fewer than `cap` rows ⇒ the COMPLETE left result (the contract); the
            // join below is then the full join. Otherwise `l` is a first-rows PROBE.
            let l_complete = l.rows.len() < cap;
            if l.rows.is_empty() {
                // Empty complete left: the full join is empty.
                return Ok(Some(union_bindings(l, Bindings::unsorted(in_scope_vars(right), vec![]))));
            }
            // Correlated (SIP) evaluation of the right child seeded from the capped
            // left — the same machinery as the full Join arm (sq-7d3dj.30.3). Joins
            // are per-left-row local, so the joined rows are exactly the full join's
            // rows originating from `l`'s rows.
            let joined = match try_sip_join(graph, local, &l, right)? {
                Some(j) => j,
                None => {
                    let r = eval_graph_pattern(graph, local, right)?;
                    join_bindings(l, r)
                }
            };
            if l_complete || joined.rows.len() >= cap {
                return Ok(Some(joined));
            }
            // The capped-left probe joined to fewer than `cap` rows, but more left
            // rows might join: decline, the caller runs the full path.
            Ok(None)
        }
        GraphPattern::LeftJoin { left, right, expression } => {
            // OPTIONAL never drops a left row (each yields >= 1 output row, joined
            // or kept with the right unbound), so capping the LEFT side caps the
            // output. The right side is still evaluated in full so every emitted row
            // is a genuine LeftJoin solution (v1 boundary: no correlated OPTIONAL).
            let l = match try_capped(graph, local, left, cap)? {
                Some(b) => b,
                None => return Ok(None),
            };
            let r = eval_graph_pattern(graph, local, right)?;
            Ok(Some(left_outer_join(graph, local, l, r, expression.as_ref())?))
        }
        GraphPattern::Extend { inner, variable, expression } => {
            // BIND is row-count preserving (an expression error leaves the variable
            // unbound, the row is kept), so extending a capped subset is sound.
            let b = match try_capped(graph, local, inner, cap)? {
                Some(b) => b,
                None => return Ok(None),
            };
            Ok(Some(extend_bindings(graph, local, b, variable, expression)?))
        }
        _ => Ok(None),
    }
}

/// The syntactic in-scope variables of a pattern (spargebra's authoritative
/// `on_in_scope_variable` set, first-occurrence order, deduplicated) — used by the
/// capped UNION / Join arms to shape a header whose right branch was never evaluated.
pub(super) fn in_scope_vars(p: &GraphPattern) -> Vec<Variable> {
    let mut vars: Vec<Variable> = Vec::new();
    p.on_in_scope_variable(|v| {
        if !vars.contains(v) {
            vars.push(v.clone());
        }
    });
    vars
}

/// First seed block of the capped conjunctive chain: small enough that an ASK /
/// LIMIT-1 hit in the first block costs a fraction of the full join, large enough
/// to amortise the per-block chain setup.
pub(super) const CAPPED_SEED_BLOCK: usize = 1024;
/// Second-tier block: one escalation before the remainder is processed whole, so a
/// first-block miss still avoids the full chain when a solution lives within the
/// first ~64k seed rows. Exactly two escalations bound a NO-solution query to
/// three blocks; identical non-bind RHS scans may be reused within a private storage
/// allowance when no budget is armed.
pub(super) const CAPPED_SEED_BLOCK_2: usize = 65_536;

/// Capped conjunctive (BGP + FILTER) evaluation — the ASK / LIMIT-k first-solutions
/// short-circuit THROUGH JOINS (sq-7d3dj.30.8). Runs the same greedy (GOO) binary
/// join chain as `eval_bgp_binary`, but drives it from at most three geometrically
/// growing SLICES of the seed scan (`CAPPED_SEED_BLOCK` rows, then up to
/// `CAPPED_SEED_BLOCK_2`, then the remainder), applying the residual FILTERs per
/// block and stopping as soon as `cap` fully-FILTERed rows exist.
///
/// SOUNDNESS. Every chain step is per-seed-row local — a join (bind / merge / hash /
/// cross) or a row-wise FILTER over a seed subset yields exactly the full result's
/// rows that originate from that subset — so the blocks are disjoint, their
/// concatenation over the whole seed IS the full result, and stopping early only
/// truncates it (the `try_capped` contract). A row counts toward `cap` only after
/// the WHOLE chain including every residual FILTER, so a first candidate eliminated
/// by a late FILTER never satisfies the cap. The join ORDER is re-derived per block
/// from the planner estimates (a block's size can change the bind-join admission),
/// which affects row order only — the contract is multiset-level and a BGP join is
/// order-independent.
///
/// Returns `None` (caller falls back to full evaluation) for the shapes the binary
/// chain does not cover: cyclic (WCOJ / LFTJ) BGPs and quoted-triple (RDF 1.2)
/// constraint decompositions.
pub(super) fn eval_bgp_binary_capped(
    graph: &Graph,
    local: &mut LocalVocab,
    patterns: &[TriplePattern],
    filters: &[Expression],
    cap: usize,
) -> Result<Option<Bindings>, String> {
    if !bgp_uses_binary(patterns) {
        return Ok(None); // cyclic -> LFTJ: no capped variant (v1 boundary)
    }
    let (_, constraints) = extract_quoted_constraints(patterns);
    if !constraints.is_empty() {
        return Ok(None); // triple-term decomposition: keep the full path
    }
    let (pat_filters, residual) = split_sargable(graph, patterns, filters);
    let pfilter = |i: usize| -> Option<(usize, ScanCmp)> { pat_filters.get(i).copied().flatten() };
    let prepared = prepare_bgp(graph, patterns)?;
    if prepared.iter().any(|p| p.unsatisfiable) {
        return Ok(Some(Bindings::unsorted(collect_vars(patterns), vec![])));
    }
    if cap == 0 {
        // Zero rows requested: an empty partial answer is valid under the contract
        // (`|result| >= cap` — the caller's slice truncates to nothing either way).
        return Ok(Some(Bindings::unsorted(collect_vars(patterns), vec![])));
    }
    let seed = goo_seed(&prepared);
    let seed_sort_col = goo_seed_sort(&prepared, seed, pfilter(seed).map(|(c, _)| c));
    let seed_all = scan_to_bindings(
        graph,
        &prepared[seed].id_pat,
        &prepared[seed].pos_vars,
        seed_sort_col,
        pfilter(seed),
        None,
        #[cfg(feature = "semijoin-bitmap")]
        None,
    );

    #[cfg(test)]
    capped_rhs_tests::observe(0);
    // Query-local and lazy: never scan an unreached step. Pattern
    // filters are immutable here; each slot also keys the requested scan order.
    // Armed budgets retain the old per-step lifetime and polling: their working-set
    // estimate does not account for multiple retained RHS relations.
    let reuse_rhs = !budget::active();
    let (mut rhs_cache, mut rhs_remaining) = capped_rhs_cache(prepared.len(), reuse_rhs);
    let mut acc: Option<Bindings> = None;
    let mut start = 0usize;
    while start < seed_all.rows.len() {
        #[cfg(test)]
        capped_rhs_tests::observe(1);
        let end = if start == 0 {
            CAPPED_SEED_BLOCK.min(seed_all.rows.len())
        } else if start == CAPPED_SEED_BLOCK {
            CAPPED_SEED_BLOCK_2.min(seed_all.rows.len())
        } else {
            seed_all.rows.len()
        };
        // One seed slice through the whole chain. A contiguous slice of a sorted
        // scan stays sorted, so merge-join eligibility is preserved.
        let mut result = Bindings {
            vars: seed_all.vars.clone(),
            rows: seed_all.rows[start..end].to_vec(),
            sorted_by: seed_all.sorted_by.clone(),
        };
        let mut cs_ctx = CsCtx::new(&prepared);
        let mut done = vec![false; prepared.len()];
        done[seed] = true;
        cs_ctx.note_done(seed);
        let mut cur_card = prepared[seed].est as f64;
        let mut var_ndv: FxHashMap<Variable, f64> = FxHashMap::default();
        record_pattern_ndv(graph, &prepared, seed, cur_card, &mut var_ndv, &cs_ctx);
        for _ in 1..prepared.len() {
            let (i, new_card, _connected) = goo_pick(graph, &prepared, &done, &var_ndv, cur_card, &cs_ctx);
            cur_card = new_card;
            done[i] = true;
            cs_ctx.note_done(i);
            // Same step selection as `eval_bgp_binary`: bind-join when the running
            // block is much smaller than the pattern; else merge / hash / cross.
            let connecting: Vec<Variable> =
                result.vars.iter().filter(|v| prepared[i].var_pos(v).is_some()).cloned().collect();
            if connecting.len() == 1
                && distinct_pattern_vars(&prepared[i].pos_vars)
                && result.rows.len().saturating_mul(8) < prepared[i].est
            {
                let jv = &connecting[0];
                let rk = result.col(jv).unwrap();
                let pp = prepared[i].var_pos(jv).unwrap();
                #[cfg(test)]
                capped_rhs_tests::observe(3);
                #[cfg(test)]
                capped_rhs_tests::step(start, i, "bind", None, false, None);
                result = bind_join(graph, result, &prepared[i].id_pat, &prepared[i].pos_vars, rk, pp, pfilter(i));
            } else {
                let filt = pfilter(i);
                let merge_var = result.sorted_by.clone().filter(|sv| prepared[i].var_pos(sv).is_some());
                let scan_sort = filt.map(|(c, _)| c).or_else(|| merge_var.as_ref().map(|jv| prepared[i].var_pos(jv).unwrap()));
                let mut uncached = None;
                let mut spare_slot = None;
                let slot = rhs_cache.get_mut(i).unwrap_or(&mut spare_slot);
                #[cfg(test)]
                let mut scanned = false;
                let rhs = capped_rhs(slot, &mut rhs_remaining, &mut uncached, scan_sort, || {
                    #[cfg(test)]
                    {
                        capped_rhs_tests::observe(2);
                        scanned = true;
                    }
                    scan_to_bindings(
                        graph,
                        &prepared[i].id_pat,
                        &prepared[i].pos_vars,
                        scan_sort,
                        filt,
                        None,
                        #[cfg(feature = "semijoin-bitmap")]
                        None,
                    )
                });
                let connected = prepared[i].pos_vars.iter().flatten().any(|v| result.vars.contains(v));
                if let Some(jv) = merge_var.filter(|jv| rhs.sorted_by.as_ref() == Some(jv)) {
                    #[cfg(test)]
                    capped_rhs_tests::step(start, i, "merge", scan_sort, scanned, rhs.sorted_by.as_ref());
                    result = merge_join_ref(&result, rhs, &jv);
                } else if connected {
                    #[cfg(test)]
                    capped_rhs_tests::step(start, i, "hash", scan_sort, scanned, rhs.sorted_by.as_ref());
                    result = hash_join_ref(&result, rhs);
                } else {
                    #[cfg(test)]
                    capped_rhs_tests::step(start, i, "cross", scan_sort, scanned, rhs.sorted_by.as_ref());
                    result = cross_product_ref(&result, rhs);
                }
            }
            record_pattern_ndv(graph, &prepared, i, cur_card, &mut var_ndv, &cs_ctx);
            if result.rows.is_empty() {
                break;
            }
        }
        // Residual FILTERs per block — a row only counts toward the cap once it has
        // passed EVERY filter (the late-FILTER safety property: a partial solution
        // never fires the early exit).
        // (sq-1ivw7) Install the snapshot-aware non-literal column set (indexing THIS
        // block's `result` layout — the BGP variables) so the id fast path fires on capped fused
        // BGP+FILTER shapes too. Drain-safe as in `eval_flat_conjunctive`.
        #[cfg(feature = "id-filter-fastpath")]
        let idfast_cols = {
            let bgp = GraphPattern::Bgp { patterns: patterns.to_vec() };
            nonliteral_filter_cols(graph, &bgp, &result)
        };
        for f in &residual {
            if result.rows.is_empty() {
                break;
            }
            #[cfg(feature = "id-filter-fastpath")]
            with_idfast_nonlit_cols(idfast_cols.clone(), || apply_filter(graph, local, &mut result, f))?;
            #[cfg(not(feature = "id-filter-fastpath"))]
            apply_filter(graph, local, &mut result, f)?;
        }
        let merged = match acc.take() {
            None => result,
            Some(a) => union_bindings(a, result),
        };
        budget::check(merged.rows.len())?;
        let satisfied = merged.rows.len() >= cap;
        acc = Some(merged);
        if satisfied {
            break;
        }
        start = end;
    }
    Ok(Some(acc.unwrap_or_else(|| Bindings::unsorted(collect_vars(patterns), vec![]))))
}

// Conservative private allowance for retained scan storage, not a
// public QueryBudget or a limit on transient join/scan allocations. Keep room below
// the diagnostic's extra-heap rejection threshold; do not retain one RHS per pattern.
pub(super) const CAPPED_RHS_STORAGE: usize = 4 * 1024 * 1024;
// Requested order, immutable scan relation, and its charged allocated storage.
pub(super) type CappedRhs = (Option<usize>, Bindings, usize);

pub(super) fn capped_rhs_cache(len: usize, enabled: bool) -> (Vec<Option<CappedRhs>>, usize) {
    let mut slots = Vec::new();
    if !enabled
        || len
            .checked_mul(std::mem::size_of::<Option<CappedRhs>>())
            .is_none_or(|bytes| bytes > CAPPED_RHS_STORAGE)
        || slots.try_reserve_exact(len).is_err()
    {
        return (slots, 0);
    }
    let Some(bytes) = slots
        .capacity()
        .checked_mul(std::mem::size_of::<Option<CappedRhs>>())
        .filter(|&bytes| bytes <= CAPPED_RHS_STORAGE)
    else {
        return (Vec::new(), 0);
    };
    slots.resize_with(len, || None);
    (slots, CAPPED_RHS_STORAGE - bytes)
}

// Count capacities, not planner estimates or populated lengths.
// Scan rows have at most three ids and fit inline; decline an unproved spilled
// representation. Variable owns a String, whose capacity is exposed by its safe
// consuming API; move it out and back without cloning or allocating its text.
pub(super) fn capped_rhs_storage(rhs: &mut Bindings, allowance: usize) -> Option<usize> {
    let mut bytes = rhs
        .rows
        .capacity()
        .checked_mul(std::mem::size_of::<Row>())?
        .checked_add(
            rhs.vars
                .capacity()
                .checked_mul(std::mem::size_of::<Variable>())?,
        )?;
    if bytes > allowance || rhs.rows.iter().any(Row::spilled) {
        return None;
    }
    for variable in rhs.vars.iter_mut().chain(rhs.sorted_by.iter_mut()) {
        let name =
            std::mem::replace(variable, Variable::new_unchecked(String::new())).into_string();
        let capacity = name.capacity();
        *variable = Variable::new_unchecked(name);
        bytes = bytes.checked_add(capacity)?;
        if bytes > allowance {
            return None;
        }
    }
    Some(bytes)
}

// Reuse requires a pure scan of the same immutable prepared pattern,
// filters and requested order. Actual sorted_by remains the scan's truthful value.
// Fitting entries live until order replacement/query exit; non-fitting entries live
// only in the caller's per-step scratch. Allocator metadata is outside this allowance.
pub(super) fn capped_rhs<'a>(
    slot: &'a mut Option<CappedRhs>,
    remaining: &mut usize,
    uncached: &'a mut Option<Bindings>,
    sort: Option<usize>,
    scan: impl FnOnce() -> Bindings,
) -> &'a Bindings {
    if slot
        .as_ref()
        .is_none_or(|(cached_sort, _, _)| *cached_sort != sort)
    {
        if let Some(old) = slot.take() {
            *remaining += old.2;
            drop(old); // release stale ownership/accounting before its replacement scan
        }
        let mut rhs = scan();
        if let Some(bytes) = capped_rhs_storage(&mut rhs, *remaining) {
            *remaining -= bytes;
            *slot = Some((sort, rhs, bytes));
        } else {
            *uncached = Some(rhs);
            return uncached.as_ref().unwrap();
        }
    }
    &slot.as_ref().unwrap().1
}

/// Distinct (non-repeated) variable positions of a prepared pattern, or `None` if
/// a variable repeats (e.g. `?x p ?x`), which would make range counts over-count.
pub(crate) fn distinct_pattern_vars(pos_vars: &[Option<Variable>; 3]) -> bool {
    let vars: Vec<&Variable> = pos_vars.iter().flatten().collect();
    let mut sorted = vars.clone();
    sorted.sort();
    sorted.dedup();
    sorted.len() == vars.len()
}

/// A lazy stream of `(value, group-size)` pairs ascending by value, for the group
/// column `v_pos` of a pattern — the streaming form of [`group_counts`]. When the chosen
/// permutation already delivers `v_pos` order (the 6-perm case) it run-length-groups
/// directly over the borrowed scan rows, materialising NOTHING; otherwise it collects +
/// sorts the column once (reduced-permutation fallback). Lets a multi-pattern star COUNT
/// be summed by k-way merge with O(k) memory instead of one group vector per pattern.
pub(super) struct GroupStream<'a> {
    pub(super) scan: sparq_core::store::Scan<'a>,
    /// The STORED column (into the permutation's row layout) holding the group value —
    /// precomputed so the hot loop reads `row[col]` directly instead of rebuilding the
    /// canonical triple per row.
    pub(super) col: usize,
    pub(super) i: usize,
    pub(super) sorted_vals: Option<Vec<Id>>,
}

impl<'a> GroupStream<'a> {
    pub(super) fn new(graph: &'a Graph, id_pat: &IdPattern, v_pos: usize) -> Self {
        let scan = graph.store.scan_sorted(id_pat, v_pos);
        let order = scan.perm.order();
        // Canonical column `v_pos` is stored at this position in the permutation's rows.
        let col = order.iter().position(|&c| c == v_pos).unwrap();
        let sorted = order.into_iter().find(|&c| id_pat[c].is_none()) == Some(v_pos);
        let sorted_vals = (!sorted).then(|| {
            let mut v: Vec<Id> = scan.rows.iter().map(|r| r[col]).collect();
            v.sort_unstable();
            v
        });
        GroupStream { scan, col, i: 0, sorted_vals }
    }

    /// The next `(value, run-length)` in ascending value order, or `None` at the end.
    pub(super) fn next(&mut self) -> Option<(Id, usize)> {
        let (slice, col): (&[[Id; 3]], usize) = match &self.sorted_vals {
            // The fallback stores bare values in column 0 of a 1-wide logical view; reuse
            // the same run-length code by treating the Vec as the source.
            Some(vals) => {
                if self.i >= vals.len() {
                    return None;
                }
                let v = vals[self.i];
                let mut c = 0;
                while self.i < vals.len() && vals[self.i] == v {
                    self.i += 1;
                    c += 1;
                }
                return Some((v, c));
            }
            None => (&self.scan.rows, self.col),
        };
        if self.i >= slice.len() {
            return None;
        }
        let v = slice[self.i][col];
        let mut c = 0;
        while self.i < slice.len() && slice[self.i][col] == v {
            self.i += 1;
            c += 1;
        }
        Some((v, c))
    }
}

/// Counts the solutions of a single triple pattern constrained by sargable numeric
/// FILTER(s), WITHOUT materialising a row per solution. Returns `None` (fall back) if
/// any filter is not a sargable numeric comparison on a variable of the pattern.
pub(super) fn count_single_filtered(
    graph: &Graph,
    id_pat: &IdPattern,
    pos_vars: &[Option<Variable>; 3],
    filters: &[Expression],
) -> Option<usize> {
    // Resolve every filter to (canonical position, comparison); all must be sargable.
    let mut cmps: Vec<(usize, ScanCmp)> = Vec::with_capacity(filters.len());
    for f in filters {
        let (var, cmp) = extract_sargable(graph, f)?;
        let pos = pos_vars.iter().position(|v| v.as_ref() == Some(&var))?;
        cmps.push((pos, cmp));
    }

    // Fast path: one filter on an all-inline column -> binary-searched range size
    // (the same value-sorted slice range-pruning uses, but we only need its length).
    // Requires the scan to be ACTUALLY sorted by the filter column (a reduced
    // permutation set may not deliver that order, in which case we fall through to the
    // count-scan below).
    if let [(fpos, cmp)] = cmps[..] {
        let scan = graph.store.scan_sorted(id_pat, fpos);
        let sorted_by_f = scan.perm.order().into_iter().find(|&c| id_pat[c].is_none()) == Some(fpos);
        if sorted_by_f && scan.rows.first().is_some_and(|r| dict::is_inline(scan.to_spo(r)[fpos])) {
            return Some(match inline_pass_values(cmp) {
                Some((lo, hi)) => {
                    let (lo_id, hi_id) = (dict::INLINE_BASE + lo, dict::INLINE_BASE + hi);
                    let start = scan.rows.partition_point(|r| scan.to_spo(r)[fpos] < lo_id);
                    let end = scan.rows.partition_point(|r| scan.to_spo(r)[fpos] <= hi_id);
                    end - start
                }
                None => 0,
            });
        }
    }

    // General: scan once, count rows passing ALL comparisons via the value caches.
    // No solution row is built. The single-comparison shapes are specialised so the
    // per-row test is a direct cache probe with no predicate-kind dispatch (this loop
    // is the COUNT-mode hot path for cached-numeric FILTERs).
    let scan = graph.store.scan(id_pat);
    let total = match cmps[..] {
        [(pos, ScanCmp::Num(cmp))] => scan
            .rows
            .iter()
            .filter(|row| graph.numeric_value(scan.to_spo(row)[pos]).is_some_and(|x| cmp.test(x)))
            .count(),
        [(pos, ScanCmp::Temp(op, t))] => scan
            .rows
            .iter()
            .filter(|row| {
                graph
                    .temporal_value(scan.to_spo(row)[pos])
                    .and_then(|v| Temporal::cmp_t(v, t))
                    .is_some_and(|o| op.eval(o))
            })
            .count(),
        _ => scan
            .rows
            .iter()
            .filter(|row| {
                let spo = scan.to_spo(row);
                cmps.iter().all(|(pos, cmp)| cmp.test_id(graph, spo[*pos]))
            })
            .count(),
    };
    Some(total)
}

/// Exact solution count of a filter-free conjunctive BGP without materialising the
/// result, for two shapes: a single pattern (index range size) and an N-pattern STAR
/// — every pattern sharing one common variable `v*`, with every other variable local
/// to a single pattern — counted as `Σ_v Π_i c_i(v)` over per-pattern group sizes,
/// streamed from the sorted indexes. Returns `None` (fall back to full evaluation)
/// for non-star shapes (e.g. 3+-pattern chains, where the product formula overcounts).
pub(super) fn count_pushdown(graph: &Graph, inner: &GraphPattern) -> Option<usize> {
    if !is_conjunctive(inner) {
        return None;
    }
    let mut patterns = Vec::new();
    let mut filters = Vec::new();
    flatten_conjunction(inner, &mut patterns, &mut filters);

    if patterns.len() == 1 {
        let (id_pat, pos_vars, unsat) = prepare_pattern(graph, &patterns[0]).ok()?;
        if unsat {
            return Some(0);
        }
        if !distinct_pattern_vars(&pos_vars) {
            return None;
        }
        if filters.is_empty() {
            return Some(graph.store.estimate(&id_pat));
        }
        // Single pattern + sargable numeric FILTER(s): count the passing rows without
        // materialising — a binary-searched range size on an all-inline column, else a
        // count-scan (still no Row built per solution).
        return count_single_filtered(graph, &id_pat, &pos_vars, &filters);
    }

    // Multi-pattern star count is filter-free (a pushed filter changes the per-value
    // group counts; leave that to full evaluation).
    if !filters.is_empty() {
        return None;
    }

    // Prepare every pattern; each must have distinct in-pattern vars (no repeated var,
    // so a group-count per value is well defined).
    let mut prepared: Vec<(IdPattern, [Option<Variable>; 3])> = Vec::with_capacity(patterns.len());
    for p in &patterns {
        let (ip, pv, unsat) = prepare_pattern(graph, p).ok()?;
        if unsat {
            return Some(0);
        }
        if !distinct_pattern_vars(&pv) {
            return None;
        }
        prepared.push((ip, pv));
    }

    // Star test: find a variable in EVERY pattern; require every OTHER variable to
    // occur in exactly one pattern (so the only join is on the centre — otherwise the
    // product formula would overcount a second shared variable).
    let mut occ: FxHashMap<&Variable, usize> = FxHashMap::default();
    for (_, pv) in &prepared {
        for v in pv.iter().flatten() {
            *occ.entry(v).or_insert(0) += 1;
        }
    }
    let center = *occ.iter().find(|(_, &n)| n == prepared.len()).map(|(v, _)| v)?;
    if occ.iter().any(|(v, &n)| *v != center && n != 1) {
        return None;
    }

    // Σ_v Π_i c_i(v) over the centre value v, computed by k-way INTERSECTION MERGE of the
    // per-pattern group-count streams — only values present in EVERY pattern contribute.
    // O(k) memory: no per-pattern group vector is materialised (the dominant memory of a
    // star COUNT at scale), just one cursor per pattern.
    let mut streams: Vec<GroupStream> = Vec::with_capacity(prepared.len());
    for (ip, pv) in &prepared {
        let cpos = pv.iter().position(|v| v.as_ref() == Some(center))?;
        streams.push(GroupStream::new(graph, ip, cpos));
    }
    let mut heads: Vec<(Id, usize)> = Vec::with_capacity(streams.len());
    for s in &mut streams {
        match s.next() {
            Some(h) => heads.push(h),
            None => return Some(0), // a pattern with no rows → empty intersection.
        }
    }
    let mut total = 0usize;
    loop {
        let max_v = heads.iter().map(|(v, _)| *v).max().unwrap();
        // Advance every cursor up to `max_v`; track whether all reach exactly it.
        let mut all_equal = true;
        for (i, head) in heads.iter_mut().enumerate() {
            while head.0 < max_v {
                match streams[i].next() {
                    Some(h) => *head = h,
                    None => return Some(total), // this pattern exhausted → done.
                }
            }
            if head.0 != max_v {
                all_equal = false;
            }
        }
        if all_equal {
            let prod: usize = heads.iter().map(|(_, c)| *c).product();
            total += prod;
            for (i, head) in heads.iter_mut().enumerate() {
                match streams[i].next() {
                    Some(h) => *head = h,
                    None => return Some(total),
                }
            }
        }
    }
}

/// Evaluate `GRAPH <name> { inner }`. Each named graph is a self-contained sub-`Graph` with its own
/// dictionary, so we evaluate `inner` against it and then TRANSLATE the result ids into the outer
/// graph's id space (materialise each term in the sub-graph, re-intern in the outer dict / local
/// vocab) — so the bindings join and serialise correctly. `GRAPH ?g { … }` unions over every named
/// graph, prepending the graph-name binding.
pub(super) fn eval_graph_named(
    graph: &Graph,
    local: &mut LocalVocab,
    name: &NamedNodePattern,
    inner: &GraphPattern,
) -> Result<Bindings, String> {
    eval_graph_named_pref(graph, local, name, inner, None)
}

/// (sq-zz8z, gh-51) Extracts a SOUND graph-IRI prefix from a FILTER expression scoping
/// the graph variable `gv`, when the filter is — or AND-contains — `STRSTARTS(STR(?gv), "lit")`
/// with a SIMPLE / `xsd:string` constant `"lit"`. Returns the prefix the named-graph enumeration
/// can range-scan on; `None` if no such conjunct exists (the engine then falls back to the full
/// scan, so this is purely an optimisation hint and never affects correctness).
///
/// SOUNDNESS: the range scan keeps exactly the graphs whose `STR(?g)` starts with `"lit"`, which
/// is precisely what `STRSTARTS(STR(?g), "lit")` keeps — AND the original FILTER still runs
/// afterwards, so even if this recogniser were over-eager the result would be unchanged. We only
/// recognise a constant SIMPLE/`xsd:string` second argument (a lang-tagged or typed non-string
/// literal would make `STRSTARTS` itself a type error, so no prefix is meaningful there) and only
/// pull a prefix out of a top-level conjunction (`a && b`), never a disjunction/negation.
pub(super) fn recognise_graph_prefix(expr: &Expression, gv: &Variable) -> Option<String> {
    use spargebra::algebra::Function;
    match expr {
        // STRSTARTS(STR(?gv), "lit")
        Expression::FunctionCall(Function::StrStarts, args) if args.len() == 2 => {
            // arg0 must be STR(?gv)
            let is_str_of_gv = matches!(
                &args[0],
                Expression::FunctionCall(Function::Str, inner)
                    if inner.len() == 1 && matches!(&inner[0], Expression::Variable(v) if v == gv)
            );
            if !is_str_of_gv {
                return None;
            }
            // arg1 must be a constant simple / xsd:string literal.
            match &args[1] {
                Expression::Literal(l) if l.language().is_none() && l.datatype() == oxrdf::vocab::xsd::STRING => {
                    Some(l.value().to_string())
                }
                _ => None,
            }
        }
        // A top-level conjunction: a prefix from EITHER side is sound (the other conjunct is
        // still enforced by the surviving FILTER). Prefer the left, else the right.
        Expression::And(l, r) => recognise_graph_prefix(l, gv).or_else(|| recognise_graph_prefix(r, gv)),
        _ => None,
    }
}

/// (sq-zz8z, gh-51) `eval_graph_named` with an optional graph-IRI PREFIX restriction
/// for the `GRAPH ?g` (variable) case: when `Some(prefix)`, only named graphs whose `STR(?g)`
/// starts with `prefix` are enumerated — via the sorted prefix index's RANGE SCAN
/// ([`Graph::for_named_graphs_with_prefix`]) rather than a full O(graphs) scan. This is purely an
/// enumeration restriction equivalent to a `FILTER(STRSTARTS(STR(?g), prefix))` applied to the
/// result (the caller's residual filter, if any, still runs and is then a no-op on the same rows),
/// so results are IDENTICAL to the unindexed path; the prefix only shrinks how many graphs are
/// visited. `None` (and the concrete-IRI case) keeps the original full enumeration.
pub(super) fn eval_graph_named_pref(
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
            // (sq-zz8z) Accumulate the per-graph relations into ONE flat row buffer in
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

/// Operator-entry dispatcher. When an EXPLAIN ANALYZE trace is installed (T22) it
/// routes through the `#[cold]` timing wrapper; otherwise it is a direct call into
/// the evaluator — the only cost on the normal path is one thread-local flag read
/// per *operator* (the same class of check `budget::check` already does here).
pub(super) fn eval_graph_pattern(graph: &Graph, local: &mut LocalVocab, p: &GraphPattern) -> Result<Bindings, String> {
    let b = if trace::enabled() {
        eval_graph_pattern_traced(graph, local, p)?
    } else {
        eval_graph_pattern_inner(graph, local, p)?
    };
    // (sq-s5is) Byte-accounted cap: price THIS operator's output at its true
    // WIDTH (`vars × BYTES_PER_ID`), the dimension the row cap misses. The operator
    // dispatcher is the single cooperative chokepoint every materialised intermediate
    // flows back through, so one width-aware check here bounds the widest intermediate.
    // No-op (one thread-local read) when no budget is installed.
    let prev = budget::set_width(b.vars.len());
    let r = budget::check(b.rows.len());
    budget::restore_width(prev);
    r?;
    Ok(b)
}

/// EXPLAIN ANALYZE wrapper: records one trace node per operator with its output
/// row count and wall time. `#[cold]` keeps it (and the `Instant` plumbing) off
/// the normal path entirely.
#[cold]
pub(super) fn eval_graph_pattern_traced(graph: &Graph, local: &mut LocalVocab, p: &GraphPattern) -> Result<Bindings, String> {
    let idx = trace::enter(trace_label(p));
    // Structured EXPLAIN (`explain-json`): attach the planner's estimated output
    // cardinality to conjunctive (BGP) nodes so the q-error compares the engine's own
    // estimate against the ANALYZE actual. Only BGP nodes carry a cardinality model;
    // an estimate failure (e.g. an unresolvable constant) just leaves `est = None`.
    // sq-u4lgr
    #[cfg(feature = "explain-json")]
    if is_conjunctive(p) {
        if let Ok(est) = bgp_estimate(graph, p) {
            trace::set_est(idx, est);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let start = std::time::Instant::now();
    let r = eval_graph_pattern_inner(graph, local, p);
    #[cfg(not(target_arch = "wasm32"))]
    let nanos = start.elapsed().as_nanos() as u64;
    // `Instant` is unusable on wasm32-unknown-unknown (it panics): rows only there.
    #[cfg(target_arch = "wasm32")]
    let nanos = 0u64;
    trace::exit(idx, r.as_ref().map_or(0, |b| b.rows.len()), nanos);
    r
}

/// Crate-internal accessor for [`trace_label`], used by the structured EXPLAIN
/// (`explain-json`) to label planning-only tree nodes with the SAME operator labels
/// the text ANALYZE trace prints. sq-u4lgr
#[cfg(feature = "explain-json")]
pub(crate) fn trace_label_pub(p: &GraphPattern) -> String {
    trace_label(p)
}

/// The operator label an EXPLAIN ANALYZE trace node carries (only built while tracing).
pub(super) fn trace_label(p: &GraphPattern) -> String {
    if is_conjunctive(p) {
        let mut patterns = Vec::new();
        let mut filters = Vec::new();
        flatten_conjunction(p, &mut patterns, &mut filters);
        let plan = if bgp_uses_binary(&patterns) { "binary GOO" } else { "worst-case-optimal (LFTJ)" };
        return format!("BGP [{plan}] ({} patterns, {} filters)", patterns.len(), filters.len());
    }
    match p {
        GraphPattern::Bgp { patterns } => format!("BGP ({} patterns)", patterns.len()),
        GraphPattern::Filter { .. } => "Filter".into(),
        GraphPattern::Join { .. } => "Join".into(),
        GraphPattern::LeftJoin { .. } => "LeftJoin (OPTIONAL)".into(),
        GraphPattern::Union { .. } => "Union".into(),
        GraphPattern::Extend { variable, .. } => format!("Extend (BIND ?{})", variable.as_str()),
        GraphPattern::Minus { .. } => "Minus".into(),
        GraphPattern::Values { .. } => "Values".into(),
        GraphPattern::Path { .. } => "PropertyPath".into(),
        GraphPattern::Graph { .. } => "Graph".into(),
        GraphPattern::Project { .. } => "Project (sub-select)".into(),
        GraphPattern::Distinct { .. } => "Distinct".into(),
        GraphPattern::Reduced { .. } => "Reduced".into(),
        GraphPattern::Slice { .. } => "Slice".into(),
        GraphPattern::OrderBy { .. } => "OrderBy".into(),
        GraphPattern::Group { .. } => "Group".into(),
        _ => "Other".into(),
    }
}

pub(super) fn eval_graph_pattern_inner(graph: &Graph, local: &mut LocalVocab, p: &GraphPattern) -> Result<Bindings, String> {
    budget::check(0)?; // coarse cooperative cancellation: once per operator entry
    if is_conjunctive(p) {
        let mut patterns = Vec::new();
        let mut filters = Vec::new();
        flatten_conjunction(p, &mut patterns, &mut filters);
        // (sq-7d3dj.30.7) Equality-FILTER → value-join unification (opt-in
        // `value-join` feature): a `FILTER(?a = ?b)` whose operands come from otherwise
        // DISCONNECTED components of the pattern set executes as a value-keyed hash join
        // between the components instead of a cross-product-then-filter. Any shape not
        // provably safe declines (`Ok(None)`) to the verbatim path below — see the
        // `eqjoin` module docs for the eligibility conditions and the soundness argument.
        #[cfg(feature = "value-join")]
        if let Some(b) = eqjoin::try_eq_component_join(graph, local, &patterns, &filters)? {
            return Ok(b);
        }
        return eval_flat_conjunctive(graph, local, &patterns, filters);
    }
    match p {
        GraphPattern::Bgp { patterns } => eval_bgp(graph, patterns),
        GraphPattern::Filter { expr, inner } => {
            // (sq-zz8z, gh-51) Prefix-scoped-aggregate fast path: a
            // `GRAPH ?g { … } FILTER(STRSTARTS(STR(?g), "prefix"))` (the PSS multi-tenant
            // `usage(prefix)` shape) only needs the named graphs whose IRI starts with `prefix`.
            // Push the prefix into the graph enumeration so it RANGE-SCANS the sorted graph-IRI
            // index (O(log G + matches)) instead of enumerating all G graphs. The FILTER below
            // STILL runs and is exact, so the result is unchanged — the pushdown only restricts
            // which graphs are visited.
            if let GraphPattern::Graph { name: NamedNodePattern::Variable(gv), inner: ginner } = inner.as_ref() {
                if let Some(prefix) = recognise_graph_prefix(expr, gv) {
                    let mut b = eval_graph_named_pref(
                        graph,
                        local,
                        &NamedNodePattern::Variable(gv.clone()),
                        ginner,
                        Some(&prefix),
                    )?;
                    // (sq-7d3dj.30.11) Install the static non-literal column set for the
                    // id-level FILTER fast path, scoped to this one dispatch.
                    #[cfg(feature = "id-filter-fastpath")]
                    {
                        let cols = nonliteral_filter_cols(graph, inner, &b);
                        with_idfast_nonlit_cols(cols, || apply_filter(graph, local, &mut b, expr))?;
                    }
                    #[cfg(not(feature = "id-filter-fastpath"))]
                    apply_filter(graph, local, &mut b, expr)?;
                    return Ok(b);
                }
            }
            // (sq-7d3dj.30.9) Correlated (theta) anti-join: the negation idiom
            // `Filter(!bound(?nb), LeftJoin(A, B, Some(F)))` with an OPTIONAL condition
            // that references OUTER variables (SP2Bench q06) — the #1735 rewrite declines
            // it (it needs `expression: None`). This recognises the shape, seeds the right
            // scan sideways with the outer correlation IRI, and evaluates a correlated
            // anti-join with the residual theta re-checked verbatim. Any miss returns
            // `None` and falls through to the identical cold `Filter{LeftJoin}` plan below.
            if let Some(b) = try_theta_antijoin(graph, local, expr, inner)? {
                return Ok(b);
            }
            let mut b = eval_graph_pattern(graph, local, inner)?;
            #[cfg(feature = "id-filter-fastpath")]
            {
                let cols = nonliteral_filter_cols(graph, inner, &b);
                with_idfast_nonlit_cols(cols, || apply_filter(graph, local, &mut b, expr))?;
            }
            #[cfg(not(feature = "id-filter-fastpath"))]
            apply_filter(graph, local, &mut b, expr)?;
            Ok(b)
        }
        GraphPattern::Join { left, right } => {
            let l = eval_graph_pattern(graph, local, left)?;
            // Bind-join pushdown: if the RIGHT side is a SERVICE and the left has
            // already bound its join variables, push those bindings to the remote as a
            // VALUES block instead of materialising the whole remote relation. Join is
            // symmetric, so try either side as the SERVICE. (sq-sjkj)
            #[cfg(feature = "service")]
            {
                if let Some(r) = try_bound_join_service(graph, local, &l, right)? {
                    return Ok(join_bindings(l, r));
                }
                // Symmetric: SERVICE on the left, bindings produced by the right.
                if matches!(left.as_ref(), GraphPattern::Service { .. }) {
                    let r = eval_graph_pattern(graph, local, right)?;
                    if let Some(sl) = try_bound_join_service(graph, local, &r, left)? {
                        return Ok(join_bindings(r, sl));
                    }
                    // Fall through with the already-evaluated right; recompute left verbatim.
                    let l2 = eval_graph_pattern(graph, local, left)?;
                    return Ok(join_bindings(l2, r));
                }
            }
            // Sideways information passing (SIP): when the already-evaluated `l` is
            // SMALL, evaluate the big `right` child CORRELATED on it — seeding scans
            // (incl. inside UNION branches) from `l`'s bound IRIs instead of cold.
            // Conservative: any scope/threshold condition failure returns `None` and
            // we fall through to the cold join below, bit-for-bit. (sq-7d3dj.30.3)
            if let Some(joined) = try_sip_join(graph, local, &l, right)? {
                return Ok(joined);
            }
            let r = eval_graph_pattern(graph, local, right)?;
            Ok(join_bindings(l, r))
        }
        GraphPattern::LeftJoin { left, right, expression } => {
            // zk-trace: operator boundary marker (one thread-local read; the
            // scope is a no-op when the recorder is disarmed). NOTE: the
            // embedded OPTIONAL condition is evaluated inside the join and is
            // NOT recorded as a FilterObligation (see zk module docs).
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Optional);
            let l = eval_graph_pattern(graph, local, left)?;
            // Bind-join pushdown for `… OPTIONAL { SERVICE { … } }`: the SERVICE may
            // only be the RIGHT (preserved) side of a left join — pushing the left's
            // bound join vars restricts the remote relation, then the SAME
            // `left_outer_join` reattaches it, so OPTIONAL semantics are unchanged
            // (a left row with no remote match still survives, unbound). (sq-sjkj)
            #[cfg(feature = "service")]
            if let Some(r) = try_bound_join_service(graph, local, &l, right)? {
                return left_outer_join(graph, local, l, r, expression.as_ref());
            }
            let r = eval_graph_pattern(graph, local, right)?;
            left_outer_join(graph, local, l, r, expression.as_ref())
        }
        GraphPattern::Union { left, right } => {
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Union);
            let l = eval_graph_pattern(graph, local, left)?;
            let r = eval_graph_pattern(graph, local, right)?;
            Ok(union_bindings(l, r))
        }
        GraphPattern::Extend { inner, variable, expression } => {
            let b = eval_graph_pattern(graph, local, inner)?;
            extend_bindings(graph, local, b, variable, expression)
        }
        GraphPattern::Minus { left, right } => {
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Minus);
            let l = eval_graph_pattern(graph, local, left)?;
            let r = eval_graph_pattern(graph, local, right)?;
            Ok(minus_bindings(l, r))
        }
        GraphPattern::Values { variables, bindings } => Ok(values_bindings(graph, local, variables, bindings)),
        GraphPattern::Path { subject, path, object } => {
            // zk-trace: property-path expansion scans the store without
            // per-pattern attribution — record an Op::Path marker so a
            // consumer fails closed (ZkTrace::first_uncaptured) rather than
            // building an insufficient witness set.
            #[cfg(feature = "zk")]
            let _zk = crate::zk::op_scope(crate::zk::Op::Path);
            eval_path(graph, local, subject, path, object)
        }
        GraphPattern::Graph { name, inner } => eval_graph_named(graph, local, name, inner),
        // SPARQL 1.1 federated query. Behind the NON-DEFAULT `service` feature; when
        // off this falls through to the generic "unsupported" error below (and never
        // compiles the HTTP stack).
        #[cfg(feature = "service")]
        GraphPattern::Service { name, inner, silent } => {
            // (sq-lsp7k.2.2) A registered LOCAL handler intercepts the IRI
            // BEFORE the transport is reached. A miss returns `None` and the HTTP path
            // below runs unchanged (egress allowlist and all). Compiled out entirely
            // when `service-local` is off.
            #[cfg(feature = "service-local")]
            if let Some(b) = try_local_service(graph, local, name, inner, *silent)? {
                return Ok(b);
            }
            eval_service(graph, local, name, inner, *silent)
        }
        // (sq-lsp7k.2.2) `service-local` WITHOUT `service`: local handlers are
        // usable on their own — no HTTP/TLS stack is compiled, and an IRI with no
        // handler keeps exactly the "unsupported" outcome the feature-off build gives
        // it (under SILENT, the join identity, matching how the HTTP path degrades).
        #[cfg(all(feature = "service-local", not(feature = "service")))]
        GraphPattern::Service { name, inner, silent } => {
            match try_local_service(graph, local, name, inner, *silent)? {
                Some(b) => Ok(b),
                None if *silent => Ok(Bindings::unsorted(Vec::new(), vec![Row::new()])),
                None => Err(format!("unsupported graph pattern: {:?}", p)),
            }
        }
        GraphPattern::Project { .. }
        | GraphPattern::Distinct { .. }
        | GraphPattern::Reduced { .. }
        | GraphPattern::Slice { .. }
        | GraphPattern::OrderBy { .. }
        | GraphPattern::Group { .. } => eval_modified(graph, local, p),
        other => Err(format!("unsupported graph pattern: {other:?}")),
    }
}

/// Evaluates a flattened conjunctive group (triple `patterns` + group `filters`).
///
/// Extracted VERBATIM from the `is_conjunctive` arm of [`eval_graph_pattern_inner`]
/// (sq-7d3dj.30.7) so the `value-join` component path can evaluate each connected
/// component through EXACTLY the machinery the whole group would have used; with the
/// feature off this is simply the old inline body. Sargable numeric FILTERs are pushed
/// down into the binary-plan scans (the WCOJ path applies them afterwards); the rest
/// run as residual filters over the joined result.
pub(super) fn eval_flat_conjunctive(
    graph: &Graph,
    local: &mut LocalVocab,
    patterns: &[TriplePattern],
    filters: Vec<Expression>,
) -> Result<Bindings, String> {
    let (mut b, residual) = if bgp_uses_binary(patterns) {
        let (pat_filters, residual) = split_sargable(graph, patterns, &filters);
        // (sq-5zf8i / §A4) Acyclic BGP: run the Yannakakis full-semijoin
        // prepass before the binary join when the opt-in `yannakakis` feature is on
        // (it internally cost-gates + falls back to `eval_bgp_binary`, threading the
        // same pushed-down `pat_filters` through the materialising scans so the result
        // — and the FILTER semantics — are identical). OFF by default => the next line
        // is the only code that compiles, byte-identical to before.
        #[cfg(feature = "yannakakis")]
        let bound = eval_bgp_yannakakis(graph, patterns, &pat_filters)?;
        #[cfg(not(feature = "yannakakis"))]
        let bound = eval_bgp_binary(graph, patterns, &pat_filters)?;
        (bound, residual)
    } else {
        (eval_bgp(graph, patterns)?, filters)
    };
    // (sq-1ivw7) The non-literal column set for the residual FILTERs of THIS fused BGP.
    // Its columns index `b`'s layout (the BGP variables); an object of a literal-free constant
    // predicate is proven non-literal by the snapshot-aware analysis, unblocking the id fast path
    // for flat BGP+FILTER shapes too (previously only the wrapped `Filter` dispatch installed a
    // set). Drain-safe: `with_idfast_nonlit_cols` drains on the first consuming `apply_filter`, so
    // a nested EXISTS on a different layout sees the empty default. Built once (cheap) and reused.
    #[cfg(feature = "id-filter-fastpath")]
    let idfast_cols = {
        let bgp = GraphPattern::Bgp { patterns: patterns.to_vec() };
        nonliteral_filter_cols(graph, &bgp, &b)
    };
    for f in &residual {
        // Spatial pushdown (sq-mg9): if this residual FILTER is a pushable geof:
        // predicate and a SpatialProvider is installed, pre-restrict the rows to the
        // index's candidate superset over the geometry variable. The FILTER below
        // STILL runs and does the exact refinement, so the result is unchanged — the
        // pushdown only shrinks how many rows the exact `geof:` check examines.
        if let Some(pd) = recognise_spatial(f) {
            // (sq-lk3aw.4) EXACT-candidate pushdown first: when the provider
            // CERTIFIES the exact indexed answer set AND every surviving row is
            // certified, the residual `geof:` FILTER is provably an identity on `b` —
            // skip it (the `continue`). Any other outcome keeps the residual FILTER:
            // `Partial` already restricted the rows (the residual judges the remaining
            // not-indexed bindings), `Declined` falls back to the superset pushdown,
            // unchanged. Soundness argument: see `apply_spatial_pushdown_exact`.
            #[cfg(feature = "spatial-exact-pushdown")]
            match apply_spatial_pushdown_exact(graph, &mut b, &pd) {
                ExactPushdown::AllCertified => continue,
                ExactPushdown::Partial => {}
                ExactPushdown::Declined => {
                    apply_spatial_pushdown(graph, &mut b, &pd);
                }
            }
            #[cfg(not(feature = "spatial-exact-pushdown"))]
            apply_spatial_pushdown(graph, &mut b, &pd);
        }
        #[cfg(feature = "id-filter-fastpath")]
        with_idfast_nonlit_cols(idfast_cols.clone(), || apply_filter(graph, local, &mut b, f))?;
        #[cfg(not(feature = "id-filter-fastpath"))]
        apply_filter(graph, local, &mut b, f)?;
    }
    Ok(b)
}

/// SPARQL 1.1 federated query (`SERVICE`) — the VERBATIM path.
///
/// Forwards `SELECT * WHERE { <inner> }` to the remote `endpoint`, parses the
/// SPARQL-Results-JSON response (see the `sparq-engine-service` crate) and turns it into a
/// [`Bindings`] relation interned against this query's dictionaries — exactly as
/// `VALUES` does — so the caller joins it with the surrounding group via the normal
/// `join_bindings` path.
///
/// This is the fallback path: when the SERVICE is the right side of a join whose join
/// vars are already bound, [`try_bound_join_service`] instead pushes those bindings as
/// a `VALUES` block (the bind-join, bead sq-sjkj) and this verbatim forward is skipped.
/// A VARIABLE endpoint that the surrounding query binds is likewise handled there
/// (per-endpoint dispatch, bead sq-d4p). `eval_service` still runs for a top-level /
/// unbound SERVICE and for every case the bind-join declines (no bound join var,
/// blank-node join key, or a variable endpoint with nothing to bind it).
///
/// * `name` is a `NamedNodePattern`: a concrete IRI endpoint is evaluated; a
///   `?var` (variable) endpoint reaching THIS path has no surrounding binding to supply
///   the endpoint IRI (a top-level `SERVICE ?ep`), so there is nothing to call — it
///   errors, or, under SILENT, yields the join identity. (A bindable variable endpoint
///   never reaches here; it is dispatched by [`try_bound_join_service`].)
/// * `silent`: on ANY failure (variable endpoint, DNS/connect error, non-2xx,
///   malformed body), produce a single empty solution (the identity for join) so the
///   surrounding query keeps its bindings and does not fail.
#[cfg(feature = "service")]
pub(super) fn eval_service(
    graph: &Graph,
    local: &mut LocalVocab,
    name: &NamedNodePattern,
    inner: &GraphPattern,
    silent: bool,
) -> Result<Bindings, String> {
    // The join identity: one row with zero columns. Joining it leaves the other
    // side unchanged (it is `{ {} }`, the single empty mapping). This is the
    // SILENT-failure fallback AND the result of an empty remote relation under the
    // standard's evalService when the pattern binds nothing.
    let identity = || Bindings::unsorted(Vec::new(), vec![Row::new()]);

    // The variables the inner pattern can bind (their textual names drive the row
    // layout we build from the SRJ `head.vars`).
    let endpoint = match name {
        NamedNodePattern::NamedNode(n) => n.as_str().to_string(),
        NamedNodePattern::Variable(_) => {
            // `SERVICE ?endpoint { … }` — not supported (would need a remote call per
            // binding of ?endpoint). Documented scope-out.
            if silent {
                return Ok(identity());
            }
            return Err(
                "SERVICE with a variable endpoint (`SERVICE ?var { … }`) is not supported".into(),
            );
        }
    };

    // Render the inner algebra back to SPARQL syntax and wrap as a SELECT *. spargebra's
    // Display round-trips algebra → concrete syntax, so OPTIONAL/FILTER/UNION/sub-SELECT
    // inside the SERVICE block are all forwarded verbatim.
    let query = format!("SELECT * WHERE {{ {inner} }}");

    // (sq-my8wd.4) STREAMING consumption: each remote row is interned to a
    // compact id-level `Row` AS IT IS PARSED (the owned terms are dropped immediately)
    // instead of collecting the whole remote relation as `Term`s first. Result-identical
    // to the collect-then-intern path — same rows, multiplicity and order (pinned by the
    // service.rs `streaming_equivalence` tests) — but the per-response peak memory is
    // the response body plus the id-level relation the join needs anyway, not a
    // whole-document DOM plus a second term-level copy.
    //
    // (sq-my8wd.5) READER-SEAM: use `eval_remote_into_read` so the HTTP body
    // is consumed as a STREAM (never buffered as a full `String`). Peak memory now stays
    // BELOW the response body size (not just O(body)) — the body String is eliminated.
    // Test transports are wrapped via `TransportAsReader` so all existing tests pass.
    let mut id_rows: Vec<Row> = Vec::new();
    // (sq-my8wd.4) Savepoint the local vocab + byte budget BEFORE streaming so
    // a SILENT error can roll the partially-interned rows back out — see the SILENT arm.
    let vocab_mark = local.savepoint();
    let byte_mark = budget::byte_savepoint();
    let fetched = service_reader_transport::with(|t| {
        sparq_engine_service::service::eval_remote_into_read(t, &endpoint, &query, &mut |row| {
            id_rows.push(intern_remote_row(graph, local, &row));
            Ok(())
        })
    });
    match fetched {
        Ok(vars) => Ok(Bindings::unsorted(vars, id_rows)),
        Err(e) if silent => {
            // SILENT: swallow the error, keep the surrounding bindings. Rows already
            // interned from a partially-parsed response are discarded with `id_rows` —
            // and, so the discard is behaviour-NEUTRAL with the pre-streaming
            // collect-then-intern-on-success path (which interned nothing on a swallowed
            // error), we ROLL the interned terms out of the local vocab and REFUND the
            // byte budget they charged. Without this a partial stream would retain memory
            // and could trip `max_bytes`, turning a query the old path answered into a
            // "query budget exceeded" error. (sq-my8wd.4)
            let _ = e;
            id_rows.clear();
            local.rollback_to(vocab_mark);
            budget::restore_bytes(byte_mark);
            Ok(identity())
        }
        Err(e) => Err(e),
    }
}

/// Answer `SERVICE <iri> { inner }` from a LOCAL in-process handler, if one is
/// registered for `iri`. (sq-lsp7k.2.2)
///
/// Returns:
/// * `Ok(Some(rel))` — a handler served the IRI; `rel` is its rows interned against
///   this query's dictionaries (exactly as `VALUES` and the remote SERVICE relation
///   are), ready for the caller's ordinary join.
/// * `Ok(None)` — no handler applies (a variable endpoint, or an IRI absent from the
///   registry / no registry installed). The caller proceeds EXACTLY as it did before
///   this feature existed: the `service` HTTP path, egress allowlist included.
/// * `Err(_)` — a non-SILENT handler failure (a returned `Err`, or a violation of the
///   [`crate::LocalServiceRows`] header/arity contract, or a returned column that is
///   NOT in scope in the SERVICE group).
///
/// ## Why an out-of-scope column is rejected
///
/// The relation must be one the SERVICE group itself could have produced: a remote
/// endpoint answering `SELECT * WHERE { … }` can only ever return the group's in-scope
/// variables. A handler returning some OTHER variable would silently join against an
/// identically-named variable of the SURROUNDING query — `VALUES ?x { 1 } SERVICE <h>
/// { ?s ?p ?o }` with a returned `?x = 2` would drop the outer row — which is neither
/// the SERVICE group's semantics nor the remote path's. It is a handler bug, so it is
/// reported like any other invalid relation rather than projected away silently.
///
/// ## Why the intercept sits here
///
/// This is upstream of `eval_service`, so a handled SERVICE never constructs a
/// transport and never consults the egress policy — there is no request to police. The
/// registry can therefore only REMOVE outbound reach, never grant it; the SSRF
/// default-deny filter still guards every IRI that misses.
///
/// SILENT is honoured identically to the remote path: the failure is swallowed and the
/// join identity (one zero-column row) is returned so the surrounding solutions
/// survive. As on that path, terms interned before the failure are rolled back out of
/// the local vocab and their byte-budget charge refunded, so a swallowed failure is
/// resource-neutral.
#[cfg(feature = "service-local")]
pub(super) fn try_local_service(
    graph: &Graph,
    local: &mut LocalVocab,
    name: &NamedNodePattern,
    inner: &GraphPattern,
    silent: bool,
) -> Result<Option<Bindings>, String> {
    // Only a CONCRETE endpoint is dispatched locally — `SERVICE ?ep { … }` keeps its
    // existing behaviour exactly (documented scope-out, see the module docs).
    let NamedNodePattern::NamedNode(iri) = name else {
        return Ok(None);
    };
    let Some(handler) = local_services::lookup(iri.as_str()) else {
        return Ok(None);
    };

    // The in-scope variables of the SERVICE group, first-occurrence order — the same
    // set the bind-join computes for its VALUES head.
    let mut vars: Vec<Variable> = Vec::new();
    inner.on_in_scope_variable(|v| {
        if !vars.contains(v) {
            vars.push(v.clone());
        }
    });
    // Byte-for-byte the string the HTTP transport would have sent for this SERVICE.
    let query = format!("SELECT * WHERE {{ {} }}", inner);
    let patterns = local_service_patterns(inner);
    let req = crate::LocalServiceRequest {
        service: iri.as_str(),
        query: &query,
        vars: &vars,
        patterns: &patterns,
    };

    let vocab_mark = local.savepoint();
    let byte_mark = budget::byte_savepoint();
    let produced = handler(&req).and_then(|rows| {
        rows.validate()
            .and_then(|()| {
                // The returned relation may only name columns the SERVICE group itself
                // puts in scope (see the doc comment): anything else would join with a
                // same-named variable OUTSIDE the group.
                match rows.vars.iter().find(|v| !vars.contains(v)) {
                    Some(v) => Err(format!(
                        "variable ?{} is not in scope in the SERVICE group",
                        v.as_str()
                    )),
                    None => Ok(()),
                }
            })
            .map_err(|e| {
                format!("local SERVICE <{}> returned an invalid relation: {}", iri.as_str(), e)
            })?;
        let id_rows: Vec<Row> =
            rows.rows.iter().map(|r| intern_remote_row(graph, local, r)).collect();
        budget::check(id_rows.len())?;
        Ok(Bindings::unsorted(rows.vars, id_rows))
    });
    match produced {
        Ok(b) => Ok(Some(b)),
        Err(e) if silent => {
            let _ = e;
            local.rollback_to(vocab_mark);
            budget::restore_bytes(byte_mark);
            Ok(Some(Bindings::unsorted(Vec::new(), vec![Row::new()])))
        }
        Err(e) => Err(e),
    }
}

/// Decompose a SERVICE group into the neutral triple-pattern view handed to a local
/// handler — the table-function argument channel. ALL-OR-NOTHING on purpose: only a
/// FLAT basic graph pattern decomposes; any richer group (OPTIONAL / UNION / FILTER /
/// sub-SELECT / paths) yields an EMPTY vec so a handler can never mistake a partial
/// view for the whole group and answer as though it had understood it. Such a handler
/// works from [`crate::LocalServiceRequest::query`] instead. (sq-lsp7k.2.2)
#[cfg(feature = "service-local")]
pub(super) fn local_service_patterns(inner: &GraphPattern) -> Vec<crate::LocalServicePattern> {
    use crate::{LocalServicePattern, LocalServiceSlot as S};
    let GraphPattern::Bgp { patterns } = inner else {
        return Vec::new();
    };
    // An RDF 1.2 quoted-triple slot may itself contain variables, so it is neither a
    // `Term` nor a `Variable`: decline the whole decomposition rather than flatten it.
    let term_slot = |t: &TermPattern| match t {
        TermPattern::NamedNode(n) => Some(S::Term(Term::from(n.clone()))),
        TermPattern::BlankNode(b) => Some(S::Term(Term::from(b.clone()))),
        TermPattern::Literal(l) => Some(S::Term(Term::from(l.clone()))),
        TermPattern::Variable(v) => Some(S::Var(v.clone())),
        TermPattern::Triple(_) => None,
    };
    let mut out: Vec<LocalServicePattern> = Vec::with_capacity(patterns.len());
    for tp in patterns {
        let (Some(subject), Some(object)) = (term_slot(&tp.subject), term_slot(&tp.object)) else {
            return Vec::new();
        };
        out.push(LocalServicePattern {
            subject,
            predicate: match &tp.predicate {
                NamedNodePattern::NamedNode(n) => S::Term(Term::from(n.clone())),
                NamedNodePattern::Variable(v) => S::Var(v.clone()),
            },
            object,
        });
    }
    out
}

/// Intern ONE remote solution row into an id-level [`Row`] against this query's
/// dictionaries — exactly as `VALUES` does — so it joins with local BGP results: each
/// term resolves against the graph dictionary first, else the query-local vocab.
/// Shared by the verbatim and bound-join paths, which feed it row-by-row from the
/// streaming SERVICE parser rather than materialising the remote relation first.
/// (sq-my8wd.4; formerly the whole-relation `service_relation_to_bindings`,
/// sq-sjkj)
#[cfg(any(feature = "service", feature = "service-local"))]
pub(super) fn intern_remote_row(graph: &Graph, local: &mut LocalVocab, row: &[Option<Term>]) -> Row {
    row.iter()
        .map(|cell| match cell {
            None => NO_ID,
            Some(t) => graph.id_of(t).unwrap_or_else(|| local.intern(t.clone())),
        })
        .collect()
}

/// Attempt a bind-join (`VALUES` pushdown) of a SERVICE sub-query against the
/// already-evaluated `left` bindings. (sq-sjkj — research candidate C1)
///
/// Returns:
/// * `Ok(Some(service_bindings))` — the bound-join applied: a remote relation
///   restricted to `left`'s bound join-key tuples, ready for the caller to join (or
///   left-outer-join) with `left` exactly as it would the verbatim relation. The
///   answer is identical to the unbound-then-local-join path.
/// * `Ok(None)` — the pushdown does NOT apply (the right side is not a concrete-IRI
///   SERVICE, there is no shared bound join var, a join key is bound to a blank node,
///   or the left side is empty). The caller falls back to the verbatim SERVICE path.
/// * `Err(_)` — a non-SILENT remote failure (propagated, same as the verbatim path).
///
/// ## Why it preserves semantics
///
/// Injecting `VALUES (?j…) { (v…)… }` for the join variables restricts the remote
/// pattern to exactly the tuples `left` already binds (SPARQL 1.1 §10.2.1 inner-joins
/// the VALUES relation with the pattern). The remote therefore returns a SUBSET of
/// the unbound relation — precisely the rows that can survive the local join — and we
/// reattach them with the SAME `join_bindings` / `left_outer_join`. For a left-outer
/// join, a left row whose key has no remote match simply finds no compatible remote
/// row and survives unbound, identical to the verbatim path. SILENT is honoured: a
/// failed block degrades to the join identity (empty remote relation) so the
/// surrounding bindings are kept, exactly as the verbatim SILENT path does.
#[cfg(feature = "service")]
pub(super) fn try_bound_join_service(
    graph: &Graph,
    local: &mut LocalVocab,
    left: &Bindings,
    right: &GraphPattern,
) -> Result<Option<Bindings>, String> {
    let GraphPattern::Service { name, inner, silent } = right else {
        return Ok(None);
    };
    let silent = *silent;

    // The endpoint is either a concrete IRI (the original bind-join) or a VARIABLE
    // (`SERVICE ?ep { … }`, bead sq-d4p). A variable endpoint is dispatched per
    // distinct endpoint IRI bound by `left`: see `bound_join_variable_endpoint`.
    let endpoint = match name {
        NamedNodePattern::NamedNode(n) => n.as_str().to_string(),
        NamedNodePattern::Variable(ep_var) => {
            return bound_join_variable_endpoint(graph, local, left, ep_var, inner, silent);
        }
    };

    bound_join_to_endpoint(graph, local, left, &endpoint, inner, silent)
}

/// Bind-join the SERVICE sub-`inner` against one CONCRETE `endpoint` over the `left`
/// bindings, returning the interned remote relation (see [`try_bound_join_service`] for
/// the contract). Factored out so the variable-endpoint path
/// ([`bound_join_variable_endpoint`]) can call it once per distinct endpoint IRI.
/// (sq-sjkj / sq-d4p)
#[cfg(feature = "service")]
pub(super) fn bound_join_to_endpoint(
    graph: &Graph,
    local: &mut LocalVocab,
    left: &Bindings,
    endpoint: &str,
    inner: &GraphPattern,
    silent: bool,
) -> Result<Option<Bindings>, String> {
    // (sq-lsp7k.2.2) A locally-handled IRI never reaches the network, so there
    // is nothing to push a `VALUES` block AT. Decline, and the caller falls back to the
    // verbatim path — where `try_local_service` answers it and the outer join happens
    // locally. (This also covers `SERVICE ?ep` whose endpoint resolves to a handled
    // IRI: that endpoint's sub-join declines, which abandons the whole variable-endpoint
    // pushdown, matching the documented "concrete IRIs only" scope-out.)
    #[cfg(feature = "service-local")]
    if local_services::handles(endpoint) {
        return Ok(None);
    }
    // The remote pattern's in-scope variables; the join keys are the ones ALSO bound
    // by `left`. We must intersect with `left.vars` (textual) so the VALUES we push
    // names variables the remote pattern actually mentions.
    let mut inner_vars: Vec<Variable> = Vec::new();
    inner.on_in_scope_variable(|v| {
        if !inner_vars.contains(v) {
            inner_vars.push(v.clone());
        }
    });
    // Join keys: variables shared between the left relation and the remote pattern,
    // in `left`-column order (so we can read each left row's tuple positionally).
    let join_vars: Vec<Variable> = left
        .vars
        .iter()
        .filter(|v| inner_vars.contains(v))
        .cloned()
        .collect();
    if join_vars.is_empty() {
        // No bound join variable to push — the verbatim path is the correct (and only)
        // evaluation.
        return Ok(None);
    }
    if left.rows.is_empty() {
        // Nothing to push. An empty left makes the whole join empty anyway, but the
        // verbatim path also handles SILENT/error uniformly, so defer to it.
        return Ok(None);
    }

    // Column indices of the join vars in the left relation.
    let key_cols: Vec<usize> = join_vars.iter().map(|v| left.col(v).expect("join var is a left var")).collect();

    // Collect the DISTINCT, fully-bound, pushable join-key tuples from the left side.
    // A row with any unbound (`NO_ID`) join key, or a key bound to a non-pushable
    // term (blank node / triple term), means we cannot faithfully constrain the
    // remote — abandon the pushdown for the verbatim path to keep exact semantics.
    let mut seen: FxHashSet<Row> = FxHashSet::default();
    let mut tuples: Vec<Vec<Term>> = Vec::new();
    for row in &left.rows {
        let mut key: Row = SmallVec::new();
        for &c in &key_cols {
            key.push(row[c]);
        }
        if key.contains(&NO_ID) {
            return Ok(None); // an unbound join key — wildcard; cannot push.
        }
        if !seen.insert(key.clone()) {
            continue; // already pushed this tuple
        }
        let mut terms: Vec<Term> = Vec::with_capacity(key.len());
        for &id in &key {
            match term_of(graph, local, id) {
                Some(t) if sparq_engine_service::service::pushable_term(&t) => terms.push(t),
                _ => return Ok(None), // blank node / triple term / missing — fall back.
            }
        }
        tuples.push(terms);
    }
    if tuples.is_empty() {
        return Ok(None);
    }

    // Render the inner pattern once; each block re-uses it with a fresh VALUES head.
    let inner_sparql = format!("{inner}");
    let block = sparq_engine_service::service::bind_block_size();

    // Accumulate the union of the per-block remote relations, interning each row to the
    // id level AS IT ARRIVES from the streaming parser (sq-my8wd.4) — no
    // block's relation is ever held as owned `Term` rows. All blocks share the remote
    // `head.vars`, so we keep the first block's var list and concatenate rows
    // positionally: the same accumulation, and the same row order, as the previous
    // collect-then-intern path.
    let mut acc_vars: Option<Vec<Variable>> = None;
    let mut acc_rows: Vec<Row> = Vec::new();
    // (sq-my8wd.4) Savepoint the local vocab + byte budget BEFORE the first
    // block so a SILENT failure in ANY block can roll EVERY block's interns back out —
    // a SILENT failure discards all blocks' rows together (see the SILENT arm), so the
    // interns must all be rolled back too, or the discarded stream would retain memory
    // and charge `max_bytes`.
    let vocab_mark = local.savepoint();
    let byte_mark = budget::byte_savepoint();

    for chunk in tuples.chunks(block) {
        let values = sparq_engine_service::service::render_values_block(&join_vars, chunk);
        // Inject the VALUES inside the SELECT * group, alongside the inner pattern, so
        // the remote inner-joins the pushed bindings with its pattern.
        let query = format!("SELECT * WHERE {{ {values} {inner_sparql} }}");
        // (sq-my8wd.5) Use the reader seam here too: the bound-join path
        // fetches one block per VALUES chunk; each block's body is streamed, not buffered.
        let fetched = service_reader_transport::with(|t| {
            sparq_engine_service::service::eval_remote_into_read(t, endpoint, &query, &mut |row| {
                acc_rows.push(intern_remote_row(graph, local, &row));
                Ok(())
            })
        });
        match fetched {
            Ok(vars) => {
                if acc_vars.is_none() {
                    acc_vars = Some(vars);
                }
            }
            // SILENT: a failed block means the SERVICE as a whole must behave EXACTLY
            // as the verbatim single-request SILENT path — which yields the JOIN
            // IDENTITY (a single empty solution) so the surrounding bindings are KEPT
            // unchanged. We therefore discard any partial block results and hand the
            // caller the identity relation (one zero-column row); joining / left-outer
            // joining `left` with it leaves `left` exactly as it was. This matches the
            // unbound-then-local-join path's SILENT semantics precisely.
            // (Rows already interned from earlier blocks — or a partially-parsed
            // failing block — are ROLLED BACK out of the local vocab below, and their
            // byte-budget charge refunded, so no stray entry or `max_bytes` charge
            // survives the discard. sq-my8wd.4)
            Err(_) if silent => {
                acc_rows.clear();
                local.rollback_to(vocab_mark);
                budget::restore_bytes(byte_mark);
                return Ok(Some(Bindings::unsorted(Vec::new(), vec![Row::new()])));
            }
            Err(e) => return Err(e),
        }
    }

    // Vars: a non-empty `tuples` always produced at least one successful block above
    // (failures returned early), so `acc_vars` is set unless every block returned an
    // EMPTY result with no head — in which case the join vars are a safe head (the
    // relation has zero rows, so the var list only names columns that, being the join
    // keys, always exist on both sides).
    let vars = acc_vars.unwrap_or_else(|| join_vars.clone());
    Ok(Some(Bindings::unsorted(vars, acc_rows)))
}

/// Evaluate `SERVICE ?ep { inner }` — a VARIABLE endpoint — against the `left`
/// bindings, by dispatching one bind-join PER DISTINCT endpoint IRI that `left` binds
/// to `ep_var`. (sq-d4p)
///
/// SPARQL 1.1 federated query evaluates `SERVICE ?ep { P }` per in-scope solution μ:
/// substitute μ(?ep) for ?ep and evaluate `SERVICE μ(?ep) { P }`. The `left` relation
/// IS those in-scope solutions, so we partition `left` by its `?ep` binding and run the
/// existing bind-join ([`bound_join_to_endpoint`]) once per endpoint, then TAG every
/// remote row with `?ep = <that endpoint IRI>` so the surrounding join re-attaches the
/// remote rows to exactly the left rows that named that endpoint. The union of the
/// per-endpoint relations is the SERVICE result.
///
/// Returns:
/// * `Ok(Some(rel))` — the per-endpoint dispatch applied; `rel` carries `?ep` plus the
///   remote variables, ready for the caller's `join_bindings` / `left_outer_join`.
/// * `Ok(None)` — cannot dispatch (the endpoint variable is not bound by `left`, or the
///   bind-join declined for every endpoint — e.g. no shared join variable); the caller
///   falls back to the verbatim `eval_service`, which reports the documented
///   variable-endpoint error (or, under SILENT, the join identity).
/// * `Err(_)` — a non-SILENT remote failure (propagated).
///
/// A left row whose `?ep` is UNBOUND or not an IRI names no valid endpoint, so it
/// contributes no remote solution (it simply finds no `?ep`-tagged remote row to join
/// with) — matching per-solution `evalService`, where substituting a non-IRI yields no
/// federated answer for that solution.
#[cfg(feature = "service")]
pub(super) fn bound_join_variable_endpoint(
    graph: &Graph,
    local: &mut LocalVocab,
    left: &Bindings,
    ep_var: &Variable,
    inner: &GraphPattern,
    silent: bool,
) -> Result<Option<Bindings>, String> {
    // The endpoint variable must be a left column (bound by the surrounding query);
    // otherwise there is nothing to dispatch on — defer to the verbatim path.
    let Some(ep_col) = left.col(ep_var) else {
        return Ok(None);
    };
    if left.rows.is_empty() {
        return Ok(None);
    }

    // Partition the left rows by their endpoint id, preserving first-seen order so the
    // dispatch (and the resulting row order) is deterministic. A row whose `?ep` is
    // unbound or a non-IRI term is dropped from dispatch (it names no endpoint).
    let mut order: Vec<Id> = Vec::new();
    let mut groups: FxHashMap<Id, Vec<Row>> = FxHashMap::default();
    for row in &left.rows {
        let id = row[ep_col];
        if id == NO_ID {
            continue; // unbound endpoint — no remote call for this solution.
        }
        match term_of(graph, local, id) {
            Some(Term::NamedNode(_)) => {}
            _ => continue, // a non-IRI ?ep is not a valid endpoint — skip.
        }
        // The sub-left for an endpoint reuses the FULL left layout (same `vars`), so the
        // recursive bind-join reads join-key columns positionally exactly as before.
        groups
            .entry(id)
            .or_insert_with(|| {
                order.push(id);
                Vec::new()
            })
            .push(row.clone());
    }
    if order.is_empty() {
        // No left row binds `?ep` to a concrete endpoint IRI: nothing to dispatch, and
        // (for an inner join) the result is empty. Defer to the verbatim path so SILENT
        // vs error is decided uniformly.
        return Ok(None);
    }

    // (sq-b93pv) PRE-DISPATCH remote-request cap. `order` is the set of
    // DISTINCT endpoint IRIs this `SERVICE ?ep` would dial — known now, before any
    // socket is opened — so a high-cardinality `?ep` (an endpoint var bound to many
    // distinct IRIs) is BOUNDED HERE rather than after the requests have gone out. The
    // refusal is a hard, typed error (the `SERVICE_REMOTE_CAP_MARKER` substring) and is
    // NOT swallowed by SILENT: SILENT masks an endpoint being unreachable, not a
    // deliberate resource-policy refusal (the same stance `budget::check` takes). When no
    // cap is installed (the default) this is a single thread-local read and a no-op.
    if let Some(cap) = sparq_engine_service::service::remote_request_cap() {
        if order.len() > cap {
            return Err(format!(
                "{}: SERVICE ?{} would dispatch to {} distinct endpoints (cap {})",
                sparq_engine_service::service::SERVICE_REMOTE_CAP_MARKER,
                ep_var.as_str(),
                order.len(),
                cap,
            ));
        }
    }

    // Resolve the endpoint IRI strings once (id -> "http://…").
    let mut acc_vars: Vec<Variable> = vec![ep_var.clone()];
    let mut acc_rows: Vec<Row> = Vec::new();
    // Whether at least one endpoint's bind-join actually applied. If EVERY endpoint
    // declined (e.g. no shared join var), there is no faithful pushdown — defer wholly
    // to the verbatim path rather than returning a partial/empty result.
    let mut any_applied = false;

    for id in order {
        let sub_rows = groups.remove(&id).expect("group for ordered id");
        let endpoint = match term_of(graph, local, id) {
            Some(Term::NamedNode(n)) => n.into_string(),
            _ => continue, // unreachable: filtered above.
        };
        let sub_left = Bindings::unsorted(left.vars.clone(), sub_rows);
        let Some(rel) = bound_join_to_endpoint(graph, local, &sub_left, &endpoint, inner, silent)?
        else {
            // This endpoint's sub-join declined the pushdown (no shared join var, an
            // unbound/blank join key, …). For correctness we cannot mix a pushed
            // endpoint with a verbatim one, so abandon the whole variable-endpoint
            // pushdown and let the verbatim path handle it.
            return Ok(None);
        };
        any_applied = true;
        // Tag every remote row with `?ep = <endpoint>` (its first column) and align the
        // remaining columns to the accumulated header, extending `acc_vars` with any new
        // remote variable as it first appears.
        for v in &rel.vars {
            if !acc_vars.contains(v) {
                acc_vars.push(v.clone());
            }
        }
        // Column index of each accumulated var within THIS endpoint's relation (skip
        // `?ep`, which the remote relation does not carry).
        let src_col: Vec<Option<usize>> = acc_vars
            .iter()
            .map(|v| if v == ep_var { None } else { rel.col(v) })
            .collect();
        for r in &rel.rows {
            let mut out: Row = SmallVec::with_capacity(acc_vars.len());
            for (i, c) in src_col.iter().enumerate() {
                if i == 0 {
                    out.push(id); // the `?ep` column.
                } else {
                    out.push(c.map(|j| r[j]).unwrap_or(NO_ID));
                }
            }
            acc_rows.push(out);
        }
    }

    if !any_applied {
        return Ok(None);
    }

    // Normalise every row to the final header width (a later endpoint may have added a
    // variable absent from an earlier one; those earlier rows get `NO_ID` in the new
    // column).
    let width = acc_vars.len();
    for r in &mut acc_rows {
        while r.len() < width {
            r.push(NO_ID);
        }
    }
    Ok(Some(Bindings::unsorted(acc_vars, acc_rows)))
}



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

/// All variables bound by the triple patterns of a conjunctive subtree.
pub(super) fn collect_pattern_vars(p: &GraphPattern, out: &mut FxHashSet<Variable>) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                for v in [tp_var(&tp.subject), nnp_var(&tp.predicate), tp_var(&tp.object)].into_iter().flatten() {
                    out.insert(v);
                }
            }
        }
        GraphPattern::Filter { inner, .. } => collect_pattern_vars(inner, out),
        GraphPattern::Join { left, right } => {
            collect_pattern_vars(left, out);
            collect_pattern_vars(right, out);
        }
        _ => {}
    }
}

/// `true` if a filter expression's variables are all in `bound` (and it has no
/// EXISTS), so applying it at the top of the flattened conjunction is equivalent.
pub(super) fn filter_scope_ok(e: &Expression, bound: &FxHashSet<Variable>) -> bool {
    use Expression::*;
    match e {
        NamedNode(_) | Literal(_) => true,
        Variable(v) | Bound(v) => bound.contains(v),
        UnaryPlus(a) | UnaryMinus(a) | Not(a) => filter_scope_ok(a, bound),
        And(a, b) | Or(a, b) | Equal(a, b) | SameTerm(a, b) | Greater(a, b) | GreaterOrEqual(a, b) | Less(a, b)
        | LessOrEqual(a, b) | Add(a, b) | Subtract(a, b) | Multiply(a, b) | Divide(a, b) => {
            filter_scope_ok(a, bound) && filter_scope_ok(b, bound)
        }
        In(a, list) => filter_scope_ok(a, bound) && list.iter().all(|c| filter_scope_ok(c, bound)),
        If(c, t, f) => filter_scope_ok(c, bound) && filter_scope_ok(t, bound) && filter_scope_ok(f, bound),
        Coalesce(es) => es.iter().all(|c| filter_scope_ok(c, bound)),
        FunctionCall(_, args) => args.iter().all(|c| filter_scope_ok(c, bound)),
        Exists(_) => false,
    }
}

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
