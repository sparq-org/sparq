use super::*;

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

/// Writes one binding's JSON value directly from its id — no intermediate `oxrdf::Term`
/// for the common (dictionary) case, which is the allocator-bound cost of materialising.
#[inline]
pub(super) fn write_id_json(graph: &Graph, local: &LocalVocab, id: Id, s: &mut String) {
    if dict::is_inline(id) {
        crate::json::inline_int_json(s, id - dict::INLINE_BASE);
    } else if is_local(id) {
        // Computed terms (BIND / aggregates) are rare; reconstruct just these.
        crate::json::term_to_json(s, local.term(id));
    } else {
        write_store_id_json(graph, id, s);
    }
}

/// Writes one binding value's JSON directly from a STORE id (never a local-vocab id,
/// so no `LocalVocab` needed) — for the streaming single-pattern scan path. An RDF 1.2
/// triple-term id recurses through its component ids (which are always store/inline
/// ids), producing the SPARQL 1.2 `{"type":"triple","value":{…}}` JSON encoding.
pub(super) fn write_store_id_json(graph: &Graph, id: Id, s: &mut String) {
    if dict::is_inline(id) {
        crate::json::inline_int_json(s, id - dict::INLINE_BASE);
    } else {
        match graph.dict.term_parts(id) {
            dict::TermParts::Triple([ts, tp, to]) => {
                s.push_str("{\"type\":\"triple\",\"value\":{\"subject\":");
                write_store_id_json(graph, ts, s);
                s.push_str(",\"predicate\":");
                write_store_id_json(graph, tp, s);
                s.push_str(",\"object\":");
                write_store_id_json(graph, to, s);
                s.push_str("}}");
            }
            parts => crate::json::parts_to_json(s, parts),
        }
    }
}

/// Concatenates JSON chunks back into the single-string form (the non-streamed API).
pub(super) fn join_chunks(mut chunks: Vec<String>) -> String {
    if chunks.len() == 1 {
        chunks.pop().expect("len checked")
    } else {
        chunks.concat()
    }
}

/// Streaming fast path: `SELECT ... WHERE { <one triple pattern> }` (optionally
/// projected) serialised straight from the index scan to SPARQL-JSON — no `Bindings`,
/// no per-row `Row`, no `Term`. Returns `None` if the query is not this shape (the
/// caller falls back to the general evaluator). The vector-at-a-time idea for the most
/// common (single-pattern) browser query. Output is a chunk sequence (see
/// [`eval_select_json_chunks`]); concatenated it is the exact JSON document.
///
/// (sq-7d3dj.34.2) Emit-based: hands each serialised chunk to `emit` **as it is
/// produced** instead of collecting a `Vec<String>`, so the server can begin writing the
/// results header + early solutions to the socket before the whole result is serialised
/// (TTFB streaming). `emit` returns [`ControlFlow::Break`] when the consumer has gone away
/// (the HTTP client disconnected); the scan then stops early. Returns `Some(())` when this
/// shape was handled (fell into the streaming path), `None` when the caller must use the
/// general evaluator. The concatenation of the chunks handed to `emit` is byte-identical to
/// the pre-emit `Vec<String>` output.
pub(super) fn single_pattern_scan_json_emit(
    graph: &Graph,
    pattern: &GraphPattern,
    flush: Option<usize>,
    emit: &mut dyn FnMut(String) -> ControlFlow<()>,
) -> Option<()> {
    // zk-trace: this streaming path serialises straight from the scan without
    // materialising Bindings, so it never hits the scan-recording hook. Fall
    // through to the Bindings path (which records) while a recorder is armed —
    // result-equivalent, only the plan changes.
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return None;
    }
    if view::default_is_empty() {
        return None; // empty-default view: the general path short-circuits at the BGP
    }
    let (proj, inner): (Option<&[Variable]>, &GraphPattern) = match pattern {
        GraphPattern::Project { inner, variables } => (Some(variables), inner),
        other => (None, other),
    };
    if !is_conjunctive(inner) {
        return None;
    }
    let mut patterns = Vec::new();
    let mut filters = Vec::new();
    flatten_conjunction(inner, &mut patterns, &mut filters);
    if patterns.len() != 1 {
        return None;
    }
    // Only pushed-down sargable numeric FILTER(s); a residual filter needs the general
    // expression evaluator.
    let (pat_filters, residual) = split_sargable(graph, &patterns, &filters);
    if !residual.is_empty() {
        return None;
    }
    let filt: Option<(usize, ScanCmp)> = pat_filters[0];

    let (id_pat, pos_vars, unsat) = prepare_pattern(graph, &patterns[0]).ok()?;
    if !distinct_pattern_vars(&pos_vars) {
        return None; // a repeated variable needs the consistency check — use the general path
    }
    let out_vars: Vec<Variable> = match proj {
        Some(vs) => vs.iter().filter(|v| !v.as_str().starts_with(BNODE_VAR_PREFIX)).cloned().collect(),
        None => pos_vars.iter().flatten().filter(|v| !v.as_str().starts_with(BNODE_VAR_PREFIX)).cloned().collect(),
    };
    let cols: Vec<Option<usize>> = out_vars.iter().map(|v| pos_vars.iter().position(|x| x.as_ref() == Some(v))).collect();

    let mut head = String::from("{\"head\":{\"vars\":[");
    for (i, v) in out_vars.iter().enumerate() {
        if i > 0 {
            head.push(',');
        }
        head.push('"');
        crate::json::escape_into(&mut head, v.as_str());
        head.push('"');
    }
    head.push_str("]},\"results\":{\"bindings\":[");
    if unsat {
        head.push_str("]}}");
        let _ = emit(head);
        return Some(());
    }

    let scan = match filt {
        Some((c, _)) => graph.store.scan_sorted(&id_pat, c),
        None => graph.store.scan(&id_pat),
    };
    // Range-prune an all-inline filter column (identical to scan_to_bindings) so the
    // order matches the general path exactly (byte-identical output).
    let mut scan_rows: &[[Id; 3]] = scan.rows.as_ref();
    if let Some((fpos, cmp)) = filt {
        let actual_sort = scan.perm.order().into_iter().find(|&c| id_pat[c].is_none());
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
    // Per-row check: a no-op on a fully range-pruned slice; required for a mixed-datatype
    // column where pruning was skipped.
    let passes = |row: &[Id; 3]| -> bool {
        match filt {
            Some((fpos, cmp)) => cmp.test_id(graph, scan.to_spo(row)[fpos]),
            None => true,
        }
    };
    let write_row = |row: &[Id; 3], s: &mut String| {
        let spo = scan.to_spo(row);
        s.push('{');
        let mut first = true;
        for (vi, &col) in cols.iter().enumerate() {
            let Some(c) = col else { continue };
            if !first {
                s.push(',');
            }
            first = false;
            s.push('"');
            crate::json::escape_into(s, out_vars[vi].as_str());
            s.push_str("\":");
            write_store_id_json(graph, spo[c], s);
        }
        s.push('}');
    };

    let mut s = head;
    // roborev 1538 / sq-7d3dj.10 (audit item 6): fan the JSON serialize out
    // across cores when the installed budget cannot be violated by doing so — NO budget,
    // or a DEADLINE-ONLY budget (the default HTTP server's 30s timeout, which has no
    // row/byte cap). The fan-out builds every matching fragment before it can know a row
    // or byte count, so a ROW / BYTE cap stays on the cooperative serial loop below
    // (which checks `budget::exhausted` every 1024 rows and stops early). A deadline-only
    // budget is admitted because the coarse `limits.hit(0)` re-check at each par-chunk
    // boundary stops launching new chunks once the wall-clock deadline passes, bounding
    // the overrun to ~one chunk per worker. A blanket !budget-active → true flip is
    // REJECTED (see `parallel_json_fanout`).
    #[cfg(feature = "parallel")]
    if scan_rows.len() >= PAR_THRESHOLD {
        if let Some(limits) = budget::parallel_json_fanout() {
            use rayon::prelude::*;
            // One string per chunk (≈ per worker), not per row — avoids one heap
            // allocation per result cell. Chunks stay in order, so on the success path
            // the bytes are identical to the serial path.
            let chunk = scan_rows.len().div_ceil(rayon::current_num_threads() * 4).max(1);
            let frags: Vec<(usize, String)> = scan_rows
                .par_chunks(chunk)
                .map(|rows| {
                    // Coarse deadline re-check at the chunk boundary: once the wall-clock
                    // deadline has passed, every later chunk produces nothing, so at most
                    // the chunks already in flight (~one per worker) run to completion.
                    // The installing thread's post-fan-out gate turns the passed deadline
                    // into the timeout error, discarding this (now partial) result — so a
                    // skipped chunk NEVER escapes as a truncated body. Under no budget / an
                    // unexpired deadline this is one non-tripping `Instant` read per chunk.
                    if limits.hit(0) {
                        return (0usize, String::new());
                    }
                    let mut n = 0usize;
                    let mut f = String::new();
                    for row in rows {
                        if !passes(row) {
                            continue;
                        }
                        if !f.is_empty() {
                            f.push(',');
                        }
                        n += 1;
                        write_row(row, &mut f);
                    }
                    (n, f)
                })
                .collect();
            // Budget gate on the installing thread over the total row count — and, for a
            // deadline-only budget, the now-past wall clock: sets the sticky flag the
            // caller's `budget::check(0)` converts into the budget error (a chunk skipped
            // above means the deadline is globally past, so this fires deterministically).
            let _ = budget::exhausted(frags.iter().map(|(n, _)| n).sum());
            // Accumulate into `pending` and hand a chunk to `emit` at each flush boundary
            // (byte-identical concatenation to the old `emit_chunk` Vec layout — only the
            // chunk *boundaries* differ, and the concat is what the byte-identity contract
            // covers). `s` already holds the head.
            let mut pending = s;
            let mut wrote = false;
            for (_, f) in frags {
                if f.is_empty() {
                    continue;
                }
                if wrote {
                    pending.push(',');
                }
                wrote = true;
                pending.push_str(&f);
                if flush.is_some_and(|n| pending.len() >= n)
                    && emit(std::mem::take(&mut pending)).is_break()
                {
                    return Some(());
                }
            }
            pending.push_str("]}}");
            let _ = emit(pending);
            return Some(());
        }
    }
    let mut written = 0usize;
    for (i, row) in scan_rows.iter().enumerate() {
        // Coarse budget check every 1024 scanned rows; the caller's sticky check
        // turns an early stop into the budget error (never a truncated result).
        if i & 1023 == 0 && budget::exhausted(written) {
            break;
        }
        if !passes(row) {
            continue;
        }
        if written > 0 {
            s.push(',');
        }
        written += 1;
        write_row(row, &mut s);
        if flush.is_some_and(|n| s.len() >= n) && emit(std::mem::take(&mut s)).is_break() {
            return Some(());
        }
    }
    let _ = budget::exhausted(written); // final row-count gate (sticky)
    s.push_str("]}}");
    let _ = emit(s);
    Some(())
}

/// Evaluates a SELECT and serialises it straight to SPARQL-JSON, skipping the
/// `QueryResult` (and its per-cell `oxrdf::Term` allocation). On native the per-row
/// fragments are built in parallel; the wasm build is sequential.
pub fn eval_select_json(graph: &Graph, pattern: &GraphPattern) -> Result<String, String> {
    Ok(join_chunks(eval_select_json_chunks(graph, pattern, None)?))
}

/// [`eval_select_json`] as an ordered chunk sequence: the concatenation of the chunks
/// is byte-identical to the single-string result. `flush = Some(n)` starts a new chunk
/// roughly every `n` serialised bytes (and hands each parallel fragment over without
/// re-copying); `flush = None` produces the single-string layout (one chunk on the
/// sequential paths, head+fragments concatenated on the parallel path — exactly the
/// old behaviour and allocation profile).
pub fn eval_select_json_chunks(graph: &Graph, pattern: &GraphPattern, flush: Option<usize>) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    eval_select_json_emit(graph, pattern, flush, &mut |c| {
        out.push(c);
        ControlFlow::Continue(())
    })?;
    Ok(out)
}

/// [`eval_select_json_chunks`] driven through a per-chunk `emit` callback instead of a
/// collected `Vec<String>`.
///
/// (sq-7d3dj.34.2) This is the emit core both the buffered `Vec` API (above)
/// and the server's TTFB-streaming HTTP body ([`crate::query_json_stream_with_budget`])
/// share, so the byte layout is defined once. Each serialised chunk is handed to `emit`
/// **as it is produced**: on the streaming path the server writes the results header +
/// early solutions to the socket before the whole result is serialised, and — for the
/// single-pattern scan fast path — before the scan even finishes. `emit` returns
/// [`ControlFlow::Break`] to abandon the rest of the work (the HTTP client disconnected).
///
/// The concatenation of the chunks handed to `emit` is **byte-identical** to
/// [`eval_select_json`]/[`eval_select_json_chunks`] for the same `flush`; only the chunk
/// *boundaries* may differ (the byte-identity contract is over the concatenation). A
/// cooperative budget (row / byte cap or deadline) that trips is surfaced by the caller's
/// `budget::check` as an `Err` exactly as before — but note that on the streaming path
/// early chunks may already have been flushed when the trip is detected (the server maps a
/// post-first-byte trip to a truncated body, since the HTTP status is already committed).
///
/// (sq-yfcu2) The budget also bounds the SERIALIZE step, not just evaluation:
/// the general (multi-pattern) path re-checks it mid-serialize and gates the final chunk on
/// `budget::check`, so a deadline that falls due *while the materialised set is being
/// serialised* is reported as `Err("query budget exceeded (timeout)")` rather than answered
/// with a complete-but-late result. That is a deliberate behaviour change for late-but-
/// complete results — the budget is a bound on the whole request, and a caller that has
/// stopped waiting must not be charged for a body produced after its deadline.
pub fn eval_select_json_emit(
    graph: &Graph,
    pattern: &GraphPattern,
    flush: Option<usize>,
    emit: &mut dyn FnMut(String) -> ControlFlow<()>,
) -> Result<(), String> {
    // Refuse an exhausted budget before scanning, emitting a header, or
    // queuing Rayon work: chunk checks cannot bound time spent waiting for a busy pool.
    budget::check(0)?;
    // Streaming fast paths — no Bindings materialised at all.
    if single_pattern_scan_json_emit(graph, pattern, flush, emit).is_some() {
        budget::check(0)?; // sticky: the streaming loop may have stopped mid-scan
        return Ok(());
    }
    let mut local = LocalVocab::default();
    let bindings = eval_modified(graph, &mut local, pattern)?;
    budget::check(bindings.rows.len())?; // final gate (see eval_select)

    let out_vars: Vec<&Variable> = bindings.vars.iter().filter(|v| !v.as_str().starts_with(BNODE_VAR_PREFIX)).collect();
    let col_of: Vec<Option<usize>> = out_vars.iter().map(|v| bindings.col(v)).collect();

    let mut head = String::from("{\"head\":{\"vars\":[");
    for (i, v) in out_vars.iter().enumerate() {
        if i > 0 {
            head.push(',');
        }
        head.push('"');
        crate::json::escape_into(&mut head, v.as_str());
        head.push('"');
    }
    head.push_str("]},\"results\":{\"bindings\":[");

    let write_row = |row: &Row, s: &mut String| {
        s.push('{');
        let mut first = true;
        for (vi, &col) in col_of.iter().enumerate() {
            let Some(ci) = col else { continue };
            let id = row[ci];
            if id == NO_ID {
                continue; // unbound (e.g. OPTIONAL) — omitted from the binding
            }
            if !first {
                s.push(',');
            }
            first = false;
            s.push('"');
            crate::json::escape_into(s, out_vars[vi].as_str());
            s.push_str("\":");
            write_id_json(graph, &local, id, s);
        }
        s.push('}');
    };

    let mut s = head;
    // (sq-yfcu2) The serialize loops below are budget-checked too: the
    // pre-serialize `budget::check` above prices the ROW / BYTE caps exactly (the rows
    // are already materialised, so the count is known — unlike the single-pattern
    // streaming path, which is why this path may fan out under any budget), but
    // serialising a large materialised set is itself unbounded WORK, so a DEADLINE (or a
    // cancellation) can fall due *during* it. Both branches therefore re-check the budget
    // mid-serialize and gate the final chunk on `budget::check` — a late-but-complete
    // result is reported as the budget error, never returned as if it were in time.
    #[cfg(feature = "parallel")]
    if bindings.rows.len() >= PAR_THRESHOLD {
        use rayon::prelude::*;
        // Limit snapshot the workers re-check at each par-chunk boundary (the installing
        // thread's sticky flag is out of reach inside rayon).
        let limits = budget::snapshot();
        // One string per chunk (≈ per worker), not per row. Chunks stay in order → identical bytes.
        let chunk = bindings.rows.len().div_ceil(rayon::current_num_threads() * 4).max(1);
        let frags: Vec<String> = bindings
            .rows
            .par_chunks(chunk)
            .map(|rows| {
                // Coarse deadline/cancel re-check at the chunk boundary: once the budget is
                // past, every later chunk produces nothing, so at most the chunks already in
                // flight (~one per worker) run to completion. The post-fan-out gate below
                // turns that into the budget error and discards this (now partial) result —
                // a skipped chunk never escapes as a truncated body. Under no budget this is
                // one non-tripping read per chunk.
                if limits.hit(0) {
                    return String::new();
                }
                let mut f = String::new();
                for (k, row) in rows.iter().enumerate() {
                    if k > 0 {
                        f.push(',');
                    }
                    write_row(row, &mut f);
                }
                f
            })
            .collect();
        // Accumulate into `s` (which already holds the head) and hand a chunk to `emit` at
        // each flush boundary. The concatenation is byte-identical to the old `emit_chunk`
        // Vec layout; only the chunk boundaries differ. A skipped (empty) fragment is
        // dropped rather than separated by a comma; every non-skipped chunk holds at least
        // one row object, so on the untripped path the `wrote` flag is exactly `i > 0`.
        let mut wrote = false;
        for f in frags {
            if f.is_empty() {
                continue;
            }
            if wrote {
                s.push(',');
            }
            wrote = true;
            s.push_str(&f);
            if flush.is_some_and(|n| s.len() >= n) && emit(std::mem::take(&mut s)).is_break() {
                return Ok(());
            }
        }
        // Post-serialization gate: a deadline that fell due (or a cancellation raised)
        // while the fan-out ran is the query's answer, not this now-late result.
        budget::check(bindings.rows.len())?;
        s.push_str("]}}");
        let _ = emit(s);
        return Ok(());
    }
    for (i, row) in bindings.rows.iter().enumerate() {
        // Coarse re-check every 1024 serialised rows; the post-loop gate turns the early
        // stop into the budget error, so a truncated body is never returned as success.
        if i & 1023 == 0 && budget::exhausted(bindings.rows.len()) {
            break;
        }
        if i > 0 {
            s.push(',');
        }
        write_row(row, &mut s);
        if flush.is_some_and(|n| s.len() >= n) && emit(std::mem::take(&mut s)).is_break() {
            return Ok(());
        }
    }
    budget::check(bindings.rows.len())?; // post-serialization gate (sticky)
    s.push_str("]}}");
    let _ = emit(s);
    Ok(())
}

/// ASK evaluation: `true` iff `pattern` has at least one solution. ASK observes only
/// solution EXISTENCE, so the plan is first simplified shape-locally (`ask_simplify`:
/// ORDER BY / DISTINCT stripped where provably emptiness-neutral) and then wrapped in
/// a `LIMIT 1` slice so the capped paths — the early-terminating single-pattern scan
/// AND the block-driven conjunctive join chain plus the capped UNION / OPTIONAL /
/// Join arms of `try_capped` — stop at the first complete solution instead of
/// materialising the full result. Shapes without a capped path evaluate normally
/// (the row count is irrelevant — only emptiness is observed).
/// (sq-7d3dj.30.8)
pub fn eval_ask(graph: &Graph, pattern: &GraphPattern) -> Result<bool, String> {
    // Fail-closed: an armed zk-trace recorder keeps the ORIGINAL plan (its operator
    // markers reference the unsimplified tree; every capped path below already
    // declines while recording, so the recorded evaluation is the full one).
    #[cfg(feature = "zk")]
    let simplified = if crate::zk::enabled() { pattern.clone() } else { ask_simplify(pattern) };
    #[cfg(not(feature = "zk"))]
    let simplified = ask_simplify(pattern);
    // Exact-count fast path (a single-pattern BGP answers from the index).
    if let Some(n) = try_count(graph, &simplified) {
        return Ok(n > 0);
    }
    let sliced = GraphPattern::Slice { inner: Box::new(simplified), start: 0, length: Some(1) };
    let mut local = LocalVocab::default();
    let b = eval_modified(graph, &mut local, &sliced)?;
    budget::check(b.rows.len())?;
    Ok(!b.rows.is_empty())
}

/// Shape-local ASK plan simplification (sq-7d3dj.30.8): strips solution modifiers
/// that provably cannot change solution EXISTENCE, along the top "modifier spine"
/// only — `Project` / `Distinct` / `Reduced` / `OrderBy` chains and `Union`
/// branches, exactly the operators through which emptiness of the root is a function
/// of emptiness of the subtree. `ORDER BY` only reorders rows and `DISTINCT` only
/// removes duplicates, so neither turns an empty result non-empty or vice versa;
/// stripping them lets the LIMIT-1 capped evaluation stop at the first solution
/// instead of sorting / deduplicating a full result nothing observes.
///
/// Deliberately conservative — the recursion STOPS (subtree kept verbatim) at:
/// * `Slice` — OFFSET observes the inner COUNT (`DISTINCT` below an `OFFSET 2` can
///   flip the boolean) and which rows survive a slice depends on the inner order, so
///   everything below a `Slice` stays exactly as written;
/// * every non-modifier operator (`Join` / `Filter` / `Group` / …) whose output
///   depends on the actual rows. In particular aggregation / HAVING is NEVER
///   touched: an empty-group aggregate still yields a solution, so `Group` must
///   evaluate as-is.
pub(super) fn ask_simplify(p: &GraphPattern) -> GraphPattern {
    match p {
        GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner } => ask_simplify(inner),
        GraphPattern::Project { inner, variables } => GraphPattern::Project {
            inner: Box::new(ask_simplify(inner)),
            variables: variables.clone(),
        },
        GraphPattern::Union { left, right } => GraphPattern::Union {
            left: Box::new(ask_simplify(left)),
            right: Box::new(ask_simplify(right)),
        },
        other => other.clone(),
    }
}

/// Evaluates a SELECT but returns only the solution count. When the count can be
/// derived without materialising the result (a single-pattern scan, possibly under
/// projection / LIMIT — like QLever's lazy count) it short-circuits; otherwise it
/// evaluates and counts the rows.
pub fn count_select(graph: &Graph, pattern: &GraphPattern) -> Result<usize, String> {
    if let Some(n) = try_count(graph, pattern) {
        return Ok(n);
    }
    let mut local = LocalVocab::default();
    let bindings = eval_modified(graph, &mut local, pattern)?;
    budget::check(bindings.rows.len())?; // final gate (see eval_select)
    Ok(bindings.rows.len())
}

/// The solution count without materialising, for shapes whose count is exact from
/// the index: a single-pattern BGP (range size) under projection / OFFSET-LIMIT.
pub(super) fn try_count(graph: &Graph, p: &GraphPattern) -> Option<usize> {
    // zk-trace: a count/ASK answered from the index range consumes NO
    // attributable input triples, so an armed recorder would capture an
    // empty (insufficient) witness set. Disable the pushdown while recording
    // — the result is identical via the materialising path, only the plan
    // changes (zk module docs: "result-preserving plan changes").
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return None;
    }
    if view::default_is_empty() {
        return None; // empty-default view: the index ranges are not the active dataset
    }
    match p {
        GraphPattern::Project { inner, .. } | GraphPattern::Reduced { inner } => try_count(graph, inner),
        GraphPattern::Slice { inner, start, length } => try_count(graph, inner).map(|n| {
            let after_offset = n.saturating_sub(*start);
            length.map_or(after_offset, |l| after_offset.min(l))
        }),
        // OPTIONAL: count the left join without materialising (Σ over the join var).
        GraphPattern::LeftJoin { left, right, expression } => {
            count_leftjoin(graph, left, right, expression.as_ref())
        }
        // A single-pattern, filter-free BGP: the range size is the exact count.
        _ => count_pushdown(graph, p),
    }
}

/// The single triple pattern of a filter-free one-pattern BGP, else `None`.
pub(super) fn single_pattern(p: &GraphPattern) -> Option<TriplePattern> {
    if !is_conjunctive(p) {
        return None;
    }
    let mut patterns = Vec::new();
    let mut filters = Vec::new();
    flatten_conjunction(p, &mut patterns, &mut filters);
    (patterns.len() == 1 && filters.is_empty()).then(|| patterns.pop().unwrap())
}

/// Exact solution count of `left OPTIONAL right` for the common shape — `left` and
/// `right` each a single filter-free pattern sharing exactly one variable `v`, with
/// no OPTIONAL filter — as `Σ_v c_left(v)·max(1, c_right(v))`, streamed from the
/// sorted indexes (each left binding survives, joined with its ≥1 right matches or
/// kept once with the right vars unbound). Returns `None` otherwise.
pub(super) fn count_leftjoin(
    graph: &Graph,
    left: &GraphPattern,
    right: &GraphPattern,
    expression: Option<&Expression>,
) -> Option<usize> {
    if expression.is_some() {
        return None; // an OPTIONAL filter changes which right rows are compatible.
    }
    let (lp, rp) = (single_pattern(left)?, single_pattern(right)?);
    let (lip, lpv, lu) = prepare_pattern(graph, &lp).ok()?;
    if lu {
        return Some(0); // no left bindings at all.
    }
    if !distinct_pattern_vars(&lpv) {
        return None;
    }
    let (rip, rpv, ru) = prepare_pattern(graph, &rp).ok()?;
    if !distinct_pattern_vars(&rpv) {
        return None;
    }
    // Exactly one shared variable, at positions (lpos, rpos).
    let shared: Vec<(usize, usize)> = lpv
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.as_ref().and_then(|v| rpv.iter().position(|x| x.as_ref() == Some(v)).map(|j| (i, j))))
        .collect();
    let [(lpos, rpos)] = shared[..] else {
        return None;
    };
    // Σ_v c_left(v)·max(1, c_right(v)), streamed by merging the two sorted group-count
    // streams — left drives, right advances to match — so neither side is materialised.
    let mut left = GroupStream::new(graph, &lip, lpos);
    // Right unsatisfiable: every left binding is kept once with the right unbound.
    if ru {
        let mut total = 0usize;
        while let Some((_, cl)) = left.next() {
            total += cl;
        }
        return Some(total);
    }
    let mut right = GroupStream::new(graph, &rip, rpos);
    let mut rhead = right.next();
    let mut total = 0usize;
    while let Some((v, cl)) = left.next() {
        while let Some((rv, _)) = rhead {
            if rv < v {
                rhead = right.next();
            } else {
                break;
            }
        }
        let cr = match rhead {
            Some((rv, rc)) if rv == v => rc,
            _ => 0,
        };
        total += cl * cr.max(1);
    }
    Some(total)
}
