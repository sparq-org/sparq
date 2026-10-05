use super::*;

// ---- FILTER + expression evaluation ------------------------------------------

/// FILTER application. When the opt-in `vectorized` feature is on and the residual filter is
/// eligible (see `columnar_filter`), a columnar vector-at-a-time path runs and the row loop is
/// skipped; otherwise the scalar row path (`apply_filter_scalar`) runs verbatim. When the
/// feature is OFF this compiles to exactly the scalar body — byte-identical, zero overhead.
pub(super) fn apply_filter(graph: &Graph, local: &LocalVocab, b: &mut Bindings, expr: &Expression) -> Result<(), String> {
    // (sq-pntvh.5, M4 Phase 5 / sq-y5ew5, M4 hybrid tri-mask) Dispatcher-gated
    // columnar residual-FILTER seam (Seam A): the `columnar_filter` helper runs through the
    // shared `vec_dispatch` eligibility gate (VEC_MIN_BATCH threshold, I2/I3 checks) and
    // executes morsel-by-morsel (VEC_MORSEL rows per decode buffer), now with tri-mask
    // delegation for tie/unknown lanes. Declines (→ scalar path) for anything not provably
    // byte-identical to `apply_filter_scalar`. I5 probe counters updated on each call.
    #[cfg(feature = "vectorized")]
    if let Some(rows) = columnar_filter(graph, local, b, expr)? {
        b.rows = rows;
        return Ok(());
    }
    apply_filter_scalar(graph, local, b, expr)
}

/// (sq-7d3dj.30.11) The set of `b` column indices proven NON-LITERAL by the static
/// term-kind analysis over the FILTER's inner pattern (the id-level fast-path enabler). Empty
/// when the feature is off. Called once at the FILTER dispatch, before the row loop.
///
/// (sq-1ivw7) The analysis is SNAPSHOT-AWARE: it also credits an OBJECT-position variable
/// of a constant-predicate BGP triple as non-literal when that predicate's object column is proven
/// literal-free in the CURRENT store snapshot (`graph.predicate_has_literal_object` is false). This
/// is what unblocks q08/q12b-shaped FILTERs (`?author`/`?erdoes` appear only as `dc:creator`
/// objects, an IRI-only column). The classifier reads the LIVE `graph` handed to THIS eval, so an
/// UPDATE that inserted a literal object publishes a new snapshot the next eval re-checks — the
/// verdict never outlives its snapshot. A per-predicate memo caches the (early-exit) scan so a
/// predicate probed by several object slots of the same FILTER is scanned at most once.
#[cfg(feature = "id-filter-fastpath")]
pub(super) fn nonliteral_filter_cols(graph: &Graph, inner: &GraphPattern, b: &Bindings) -> FxHashSet<usize> {
    let memo = std::cell::RefCell::new(FxHashMap::<oxrdf::NamedNode, bool>::default());
    let obj_may_be_literal = |pred: &oxrdf::NamedNode| -> bool {
        if let Some(&v) = memo.borrow().get(pred) {
            return v;
        }
        // Conservative default when the predicate IRI is absent from the dictionary: it matches no
        // triple, so the object variable is unbound — treating it as literal-risk simply declines
        // the fast path (safe, and correct: an unbound operand is a type error, not id equality).
        let has_lit = match graph.id_of(&oxrdf::Term::NamedNode(pred.clone())) {
            Some(pid) => graph.predicate_has_literal_object(pid),
            None => true,
        };
        memo.borrow_mut().insert(pred.clone(), has_lit);
        has_lit
    };
    let vars = nonliteral_vars(inner, &obj_may_be_literal);
    vars.iter().filter_map(|v| b.col(v)).collect()
}

/// The columnar residual-FILTER attempt (M4 Phase 5 / sq-y5ew5 hybrid tri-mask, Seam A).
///
/// Returns `Ok(Some(new_rows))` (the surviving rows, in the scalar path's order) when the
/// dispatcher admits the operator invocation, `Ok(None)` to fall through to
/// `apply_filter_scalar`, or `Err(e)` if scalar delegation on a tie/unknown lane fails.
/// The returned rows are **byte-identical** to `apply_filter_scalar`'s output.
///
/// ## Decline hierarchy (I1–I5, owned by `vec_dispatch`)
///
/// Checks are applied in this order; any failing check falls through to the scalar path
/// with a `vec_dispatch::record_decline()` increment (I1: decline is total, silent-safe):
///
/// 1. **I2 (zk-decline):** when the `zk` proof-trace is armed the seam declines so the
///    scalar path records the complete per-row FILTER obligation set.
/// 2. **I3 (budget parity — fallback rule):** when a query budget is installed, the seam
///    declines. The scalar `apply_filter` debit schedule is NOT uniform-per-row (budget is
///    checked at operator entry, not inside the row loop), so the `k = min(batch, remaining)`
///    prefix rule cannot be applied without a scalar-side budget-tracking refactor. The
///    fallback is: budget-armed ⇒ decline. This is zero-risk; revisit when budget tracking
///    is uniform-per-row.
/// 3. **Operator shape:** only a single sargable `NUMERIC` comparison `?v OP const` (temporal
///    comparisons have no vector kernel yet). A NaN threshold (e.g. from an XSD double `NaN`
///    literal constant) is also declined: a NaN threshold makes all lanes Unknown, producing
///    a delegated fraction of 100% with no columnar benefit.
/// 4. **`VEC_MIN_BATCH`:** the batch must have at least `vec_dispatch::VEC_MIN_BATCH`
///    rows. Smaller batches are cheaper on the scalar path.
///
/// ## Execution (hybrid tri-mask, morsel-by-morsel, single-column extract)
///
/// For eligible invocations the function iterates morsels of `VEC_MORSEL` rows. Each morsel:
/// - Extracts only the filter column (O(morsel × 1), not a full-width transpose).
/// - Decodes to a contiguous `f64` column via `decode_numeric_column`.
/// - Runs `chunk_select::tri_mask_select` to split lanes into Confident and Delegated.
/// - Delegated (Tie + Unknown) lanes are evaluated by the full scalar `eval_expr` per row.
/// - Confident-passes and delegated-passes are merged (ascending) for this morsel.
/// - Records the morsel via `vec_dispatch::record_chunk` and delegated count via
///   `vec_dispatch::record_delegated`.
///
/// After all morsels, survivors are gathered from the original `b.rows` (order-preserving).
///
/// ## Byte-identity argument (§3 of the completion-design record, by construction)
///
/// - Confident lanes: the scalar predicate on the same row sees the same two f64 values;
///   no exact-lexical recheck applies (non-tie), so `cmp.test(x)` gives the correct answer.
/// - Tie / Unknown lanes: the hybrid's verdict IS the scalar predicate's verdict.
/// - Order: both index lists are ascending; the merge is ascending; `apply_selection` is
///   order-preserving.
///
/// **ZK coupling (soundness-relevant):** see I2 above and `research/vector-at-a-time-m4.md`
/// §3.2. (sq-y5ew5)
#[cfg(feature = "vectorized")]
pub(super) fn columnar_filter(
    graph: &Graph,
    local: &LocalVocab,
    b: &Bindings,
    expr: &Expression,
) -> Result<Option<Vec<Row>>, String> {
    use crate::chunk::{DataChunk, VecCmp};
    use crate::chunk_select;
    use crate::vec_dispatch;

    // I2: ZK trace armed ⇒ decline so the scalar path records the FILTER obligation set.
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        vec_dispatch::record_decline();
        return Ok(None);
    }

    // I3: budget armed ⇒ decline (fallback rule — debit schedule is not uniform-per-row).
    if budget::active() {
        vec_dispatch::record_decline();
        return Ok(None);
    }

    // Operator-shape check: only a single sargable NUMERIC comparison is eligible.
    // NOTE: do NOT use `extract_sargable(graph, expr)?` — the `?` operator returns None without
    // recording the decline, making the I5 counter miss non-sargable expressions.
    let (var, cmp) = match extract_sargable(graph, expr) {
        None => {
            vec_dispatch::record_decline();
            return Ok(None);
        }
        Some((v, ScanCmp::Num(c))) => (v, c),
        Some((_, ScanCmp::Temp(..))) => {
            vec_dispatch::record_decline();
            return Ok(None);
        }
    };

    // Decline a NaN threshold: all lanes would be Unknown → 100% delegation, no benefit.
    // Also ensures the tri-mask tie check (x == c) is well-defined (NaN == anything is false).
    let veccmp = match cmp {
        NumCmp::Gt(t) if t.is_nan() => { vec_dispatch::record_decline(); return Ok(None); }
        NumCmp::Ge(t) if t.is_nan() => { vec_dispatch::record_decline(); return Ok(None); }
        NumCmp::Lt(t) if t.is_nan() => { vec_dispatch::record_decline(); return Ok(None); }
        NumCmp::Le(t) if t.is_nan() => { vec_dispatch::record_decline(); return Ok(None); }
        NumCmp::Eq(t) if t.is_nan() => { vec_dispatch::record_decline(); return Ok(None); }
        NumCmp::Gt(t) => VecCmp::Gt(t),
        NumCmp::Ge(t) => VecCmp::Ge(t),
        NumCmp::Lt(t) => VecCmp::Lt(t),
        NumCmp::Le(t) => VecCmp::Le(t),
        NumCmp::Eq(t) => VecCmp::Eq(t),
    };

    let col_idx = match b.col(&var) {
        Some(col) => col,
        None => {
            vec_dispatch::record_decline();
            return Ok(None);
        }
    };

    // VEC_MIN_BATCH: batches below the threshold are cheaper on the scalar path.
    if b.rows.len() < vec_dispatch::VEC_MIN_BATCH {
        vec_dispatch::record_decline();
        return Ok(None);
    }

    // Morsel-by-morsel execution: extract only the filter column (not a full-width
    // transpose), decode it, run the tri-mask, delegate tie/unknown lanes to scalar,
    // and merge the passing indices.
    let morsel_size = vec_dispatch::VEC_MORSEL;
    let mut survivor_indices: Vec<usize> = Vec::new();
    let rows = &b.rows;

    for start in (0..rows.len()).step_by(morsel_size) {
        let end = (start + morsel_size).min(rows.len());
        let morsel_len = end - start;

        // Extract just the filter column for this morsel (O(morsel × 1), not O(morsel × width)).
        let col: Vec<sparq_core::dict::Id> = rows[start..end].iter().map(|r| r[col_idx]).collect();
        let morsel_chunk = match DataChunk::from_columns(vec![col], morsel_len) {
            Some(mc) => mc,
            None => {
                // Should not happen: col.len() == morsel_len by construction.
                vec_dispatch::record_decline();
                return Ok(None);
            }
        };

        // Decode the column once (gather-free fast path for all-inline columns, general
        // path for mixed columns — NaN sentinel for non-numeric / local-vocab / unbound ids).
        let decoded = morsel_chunk.decode_numeric_column(graph, 0);

        // Tri-mask pass: classify each lane as Confident (unambiguous f64 verdict) or
        // Delegated (tie or unknown). Returns confident-passes and delegated indices
        // in ascending order.
        let tri = chunk_select::tri_mask_select(&decoded, veccmp);

        // Record I5 counters for this morsel.
        vec_dispatch::record_chunk(morsel_len);
        vec_dispatch::record_delegated(tri.delegated.len());

        // Evaluate the full scalar predicate for each delegated (tie / unknown) lane.
        // The scalar predicate handles the exact-lexical recheck for ties and the
        // type-error path for unknowns — byte-identical to `apply_filter_scalar`.
        let mut delegated_passes: Vec<usize> = Vec::with_capacity(tri.delegated.len());
        for &local_idx in &tri.delegated {
            let global_idx = start + local_idx;
            let row = &rows[global_idx];
            let val = eval_expr(graph, local, b, row.as_ref(), expr)?;
            if effective_boolean(&val) {
                delegated_passes.push(local_idx);
            }
        }

        // Merge confident-passes and delegated-passes (both ascending) for this morsel,
        // then translate to global row indices.
        let morsel_sel = chunk_select::merge_ascending(&tri.confident_passes, &delegated_passes);
        survivor_indices.extend(morsel_sel.iter().map(|&i| start + i));
    }

    // Materialise survivors from the original rows (order-preserving, full-width).
    let result: Vec<Row> = survivor_indices.iter().map(|&r| Row::from_slice(rows[r].as_ref())).collect();
    Ok(Some(result))
}

/// The columnar per-group aggregate attempt (M4 Phase 4, `sq-pntvh.4`). Returns `Some(rows)`
/// (the output rows, byte-identical to the scalar path in first-seen group order) when ALL
/// aggregates are columnar-eligible, or `None` to fall back to the scalar group fold.
///
/// **Eligibility (conservative — decline is always safe, I1):**
/// - `vectorized` feature ON (compile-gate only, not checked here)
/// - ZK trace NOT armed (I2, checked below)
/// - Budget NOT armed (I3, checked below — fallback rule, same as `columnar_filter`)
/// - `VEC_MIN_BATCH` rows or more in the input (dispatcher threshold)
/// - No DISTINCT aggregate
/// - Only `SUM / COUNT / AVG / MIN / MAX` aggregate functions and `COUNT(*)`
/// - Aggregate argument is a bare variable `?v` (not an expression)
/// - The variable `?v` is a present column in `b`
/// - Every row in `b` has an inline-integer id in that column (all-inline gate)
///   This ensures byte-identity: see `research/vector-at-a-time-m4-completion-design.md` §2.
///
/// When there are multiple aggregates, ALL must be eligible and use the SAME column (or
/// `COUNT(*)` which needs no column). If any aggregate uses a different column, decline.
/// (sq-pntvh.4 / sq-pntvh.5 dispatcher hook)
#[cfg(feature = "vectorized")]
#[allow(clippy::too_many_arguments)]
pub(super) fn columnar_aggregate(
    graph: &Graph,
    local: &mut LocalVocab,
    b: &Bindings,
    _group_vars: &[Variable],
    aggregates: &[(Variable, AggregateExpression)],
    order: &[Key],
    members: &[Vec<usize>],
    _out_vars: &[Variable],
) -> Option<Vec<Row>> {
    use crate::reduce::{reduce_count, reduce_max_id, reduce_min_id, reduce_sum};
    use crate::vec_dispatch;

    // I2: ZK trace armed → decline so the scalar path records the aggregate obligations.
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        vec_dispatch::record_decline();
        return None;
    }

    // I3: budget armed ⇒ decline (same fallback rule as columnar_filter).
    if budget::active() {
        vec_dispatch::record_decline();
        return None;
    }

    // VEC_MIN_BATCH dispatcher threshold: small inputs are cheaper on the scalar path.
    if b.rows.len() < vec_dispatch::VEC_MIN_BATCH {
        vec_dispatch::record_decline();
        return None;
    }

    // ── Eligibility pass ──────────────────────────────────────────────────────────────────
    // Walk all aggregates to determine whether they are columnar-eligible and which
    // column (if any) the non-COUNT(*) aggregates operate on.
    let mut agg_col: Option<usize> = None; // the single eligible column index

    for (_, agg) in aggregates {
        match agg {
            AggregateExpression::CountSolutions { distinct } => {
                if *distinct {
                    vec_dispatch::record_decline();
                    return None; // DISTINCT COUNT(*) requires full row dedup
                }
                // CountSolutions needs no column — always eligible.
            }
            AggregateExpression::FunctionCall { name, expr, distinct } => {
                if *distinct {
                    vec_dispatch::record_decline();
                    return None; // DISTINCT requires per-group value dedup
                }
                // Only the five standard numeric aggregates are supported.
                match name {
                    AggregateFunction::Sum
                    | AggregateFunction::Count
                    | AggregateFunction::Avg
                    | AggregateFunction::Min
                    | AggregateFunction::Max => {}
                    _ => {
                        vec_dispatch::record_decline();
                        return None;
                    }
                }
                // Argument must be a bare variable (not an expression).
                let v = match expr {
                    Expression::Variable(v) => v,
                    _ => {
                        vec_dispatch::record_decline();
                        return None;
                    }
                };
                // The variable must have a column in the input bindings.
                let c = match b.col(v) {
                    Some(ci) => ci,
                    None => {
                        vec_dispatch::record_decline();
                        return None;
                    }
                };
                // All column-dependent aggregates must reference the SAME column.
                match agg_col {
                    None => agg_col = Some(c),
                    Some(existing) if existing == c => {}
                    _ => {
                        vec_dispatch::record_decline();
                        return None;
                    }
                }
            }
        }
    }

    // ── All-inline gate ───────────────────────────────────────────────────────────────────
    // For every row in `b`, the aggregate column must hold an inline-integer id.
    // This is the byte-identity invariant: inline-int ids encode their value directly,
    // so the reducer sees the exact same integer the scalar eval_expr would materialise.
    if let Some(c) = agg_col {
        if !b.rows.iter().all(|r| dict::is_inline(r[c])) {
            vec_dispatch::record_decline();
            return None;
        }
    }

    // ── Per-group reduction ───────────────────────────────────────────────────────────────
    let mut rows: Vec<Row> = Vec::with_capacity(order.len());

    for (key, member_indices) in order.iter().zip(members.iter()) {
        // Gather the per-group inline ids for the aggregate column (if any).
        let group_ids: Vec<Id> = if let Some(c) = agg_col {
            member_indices.iter().map(|&ri| b.rows[ri][c]).collect()
        } else {
            Vec::new()
        };

        let mut row = Row::from_slice(key);

        for (_, agg) in aggregates {
            let id: Id = match agg {
                AggregateExpression::CountSolutions { .. } => {
                    // Non-distinct COUNT(*) = number of solutions in this group.
                    // Mirrors scalar: Value::Num(Num::Int(members.len() as i64)).
                    let n = member_indices.len() as i64;
                    value_to_id(graph, local, &Value::Num(Num::Int(n)))
                }
                AggregateExpression::FunctionCall { name, .. } => {
                    match name {
                        AggregateFunction::Count => {
                            // COUNT(?v) over an all-inline group = group size (no NULLs).
                            let n = reduce_count(&group_ids) as i64;
                            value_to_id(graph, local, &Value::Num(Num::Int(n)))
                        }
                        AggregateFunction::Sum => {
                            // SUM({}) = 0 per SPARQL; for non-empty, reduce_sum is exact i128.
                            let sum_i128 = reduce_sum(&group_ids)?;
                            // Guard the i128→i64 narrowing: group cardinality is unbounded so
                            // the sum CAN exceed i64::MAX. Decline to scalar on overflow; the
                            // scalar path promotes via Num::Double, matching scalar semantics.
                            // (C1-fix sq-pntvh.4 adversarial review)
                            let sum_i64 = crate::reduce::narrow_sum_to_i64(sum_i128)?;
                            // Mirrors scalar: sum_values → num_canonical_term → value_to_id.
                            value_to_id(graph, local, &Value::Num(Num::Int(sum_i64)))
                        }
                        AggregateFunction::Avg => {
                            if group_ids.is_empty() {
                                // AVG({}) = 0 per SPARQL (mirrors scalar path line ~6378).
                                value_to_id(graph, local, &Value::Num(Num::Int(0)))
                            } else {
                                let sum_i128 = reduce_sum(&group_ids)?;
                                let count_i64 = group_ids.len() as i64;
                                // Guard the i128→i64 narrowing (same as SUM).
                                // (C1-fix sq-pntvh.4 adversarial review)
                                let sum_i64 = crate::reduce::narrow_sum_to_i64(sum_i128)?;
                                // integer / integer → Decimal (SPARQL §17.4.4.3).
                                // Mirrors: sum_values → binop(Div) → value_to_id.
                                let result = Num::Int(sum_i64).binop(Num::Int(count_i64), ArithOp::Div)?;
                                value_to_id(graph, local, &Value::Num(result))
                            }
                        }
                        AggregateFunction::Min => {
                            if group_ids.is_empty() {
                                NO_ID // unbound — mirrors scalar minmax_values([]) = Value::Unbound
                            } else {
                                // reduce_min_id returns None only if a non-inline id slipped
                                // through the all-inline gate (should not happen; decline safely).
                                reduce_min_id(&group_ids)?
                            }
                        }
                        AggregateFunction::Max => {
                            if group_ids.is_empty() {
                                NO_ID // unbound
                            } else {
                                reduce_max_id(&group_ids)?
                            }
                        }
                        _ => return None, // unreachable given the eligibility check above
                    }
                }
            };
            row.push(id);
        }

        rows.push(row);
    }

    // I5: record one "chunk" for the aggregate seam (the whole group-table counts as one pass).
    // (sq-pntvh.5 dispatcher hook)
    vec_dispatch::record_chunk(b.rows.len());
    Some(rows)
}

// (sq-7d3dj.30.11) The columns the FILTER dispatch proved non-literal, handed to
// `apply_filter_scalar` for the id-level fast-path rewrite. Set (and cleared) around the single
// `apply_filter` call on the DISPATCHING thread, read once before the row loop — rayon workers
// evaluate the already-rewritten expression, so they never read this. A missing/empty value
// means "no static non-literal columns known" (every other FILTER caller), so the rewrite is a
// no-op there and the exact path is unchanged.
#[cfg(feature = "id-filter-fastpath")]
thread_local! {
    pub(super) static IDFAST_NONLIT_COLS: std::cell::RefCell<FxHashSet<usize>> =
        std::cell::RefCell::new(FxHashSet::default());
}

/// Runs `f` with the id-fast non-literal column set installed, restoring the previous set after
/// (also on unwind). Only wraps the FILTER dispatch that computed a set from its inner pattern.
#[cfg(feature = "id-filter-fastpath")]
pub(super) fn with_idfast_nonlit_cols<R>(cols: FxHashSet<usize>, f: impl FnOnce() -> R) -> R {
    let prev = IDFAST_NONLIT_COLS.with(|c| c.replace(cols));
    struct Restore(FxHashSet<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            IDFAST_NONLIT_COLS.with(|c| *c.borrow_mut() = std::mem::take(&mut self.0));
        }
    }
    let _restore = Restore(prev);
    f()
}

pub(super) fn apply_filter_scalar(graph: &Graph, local: &LocalVocab, b: &mut Bindings, expr: &Expression) -> Result<(), String> {
    // Pre-resolve Variable → column index once before the row loop. sq-7d3dj.4.
    #[cfg_attr(not(feature = "id-filter-fastpath"), allow(unused_mut))]
    let mut compiled = compile_expr(expr, b);
    // (sq-7d3dj.30.11) Rewrite eligible `=` nodes into the id-level fast path using the
    // non-literal columns the FILTER dispatch computed for THIS operator (empty for every other
    // caller → no-op). Done ONCE, before the row loop; rayon workers see the rewritten program.
    // TAKE (drain) the installed set: it is valid ONLY for THIS `Bindings` layout, and this
    // consuming call is the one the wrapping FILTER dispatch installed it for. Draining to empty
    // means a nested FILTER re-entered during row evaluation (an `EXISTS` inner, whose columns
    // index a DIFFERENT `Bindings`) — or any later unwrapped `apply_filter` on this thread — sees
    // the empty default and rewrites nothing, so it can never apply the outer pattern's column
    // indices to a mismatched layout. (sq-7d3dj.30.11, PR #1785 review).
    #[cfg(feature = "id-filter-fastpath")]
    {
        let cols = IDFAST_NONLIT_COLS.with(|cols| std::mem::take(&mut *cols.borrow_mut()));
        if !cols.is_empty() {
            idfast_rewrite(&mut compiled, &cols);
        }
    }
    // Per-row FILTER evaluation is independent and read-only over the graph/bindings, so a
    // large residual (non-pushed-down) filter is evaluated in parallel on native.
    // Row identity for BNODE(str)'s per-solution scoping (see ROW_SCOPE).
    let scope = b.rows.as_ptr() as usize;
    #[cfg(feature = "parallel")]
    let keep: Vec<bool> = if b.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        // The extension-function registry and the dataset view are thread-local:
        // snapshot them here and re-install per worker item (free when neither is
        // installed). The view matters because a FILTER can re-enter pattern
        // evaluation via EXISTS — without the re-install it would silently
        // evaluate UNRESTRICTED on a rayon worker.
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot(); // sq-mg9: keep the spatial index visible under EXISTS re-entry.
        // (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
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
                ROW_SCOPE.set((scope, i));
                Ok(effective_boolean(&eval_compiled(graph, local, b, row, &compiled)?))
            })
            .collect::<Result<Vec<bool>, String>>()?
    } else {
        let mut keep = Vec::with_capacity(b.rows.len());
        for (i, row) in b.rows.iter().enumerate() {
            ROW_SCOPE.set((scope, i));
            keep.push(effective_boolean(&eval_compiled(graph, local, b, row, &compiled)?));
        }
        keep
    };
    #[cfg(not(feature = "parallel"))]
    let keep: Vec<bool> = {
        let mut keep = Vec::with_capacity(b.rows.len());
        for (i, row) in b.rows.iter().enumerate() {
            ROW_SCOPE.set((scope, i));
            keep.push(effective_boolean(&eval_compiled(graph, local, b, row, &compiled)?));
        }
        keep
    };
    // zk-trace hook: record the FILTER obligation — expression, in-scope
    // variables, and per-row operand bindings with the verdict (the witness
    // builder needs the operands of every hidden-filter application). The
    // per-row path records OPERAND-TABLE INDICES (one memo probe per cell);
    // terms are materialized once per distinct operand id. Suppressed inside
    // EXISTS (per-row re-evaluation would flood the obligation list; EXISTS
    // is outside the stage-1 fragment).
    #[cfg(feature = "zk")]
    if crate::zk::enabled() && !crate::zk::in_exists() {
        let mut memo = crate::zk::OperandMemo::new();
        let rows: Vec<(Vec<u32>, bool)> = b
            .rows
            .iter()
            .zip(keep.iter())
            .map(|(row, &k)| {
                let cells = (0..b.vars.len())
                    .map(|c| memo.index(row[c], |id| term_of(graph, local, id)))
                    .collect();
                (cells, k)
            })
            .collect();
        crate::zk::record_filter(format!("{expr:?}"), &b.vars, memo.operands, rows);
    }
    let mut i = 0;
    b.rows.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) enum Value {
    Bool(bool),
    Num(Num),
    Term(Term),
    Unbound,
    /// A SPARQL type error (e.g. an ordering comparison between incompatible
    /// types). Its effective boolean value is false, it propagates through the
    /// logical operators by the SPARQL 3-valued rules, and a BIND of it is unbound.
    Error,
}
