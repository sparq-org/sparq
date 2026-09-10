const CAPPED_SEED_BLOCK: usize = 1024;
/// Second-tier block: one escalation before the remainder is processed whole, so a
/// first-block miss still avoids the full chain when a solution lives within the
/// first ~64k seed rows. Exactly two escalations bound a NO-solution query to
/// three blocks; identical non-bind RHS scans may be reused within a private storage
/// allowance when no budget is armed.
const CAPPED_SEED_BLOCK_2: usize = 65_536;

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
fn eval_bgp_binary_capped(
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
    // [GPT-6 Astra] Query-local and lazy: never scan an unreached step. Pattern
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
        // [FABLE-5] (sq-1ivw7) Install the snapshot-aware non-literal column set (indexing THIS
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

// [GPT-6 Astra] Conservative private allowance for retained scan storage, not a
// public QueryBudget or a limit on transient join/scan allocations. Keep room below
// the diagnostic's extra-heap rejection threshold; do not retain one RHS per pattern.
const CAPPED_RHS_STORAGE: usize = 4 * 1024 * 1024;
// Requested order, immutable scan relation, and its charged allocated storage.
type CappedRhs = (Option<usize>, Bindings, usize);

fn capped_rhs_cache(len: usize, enabled: bool) -> (Vec<Option<CappedRhs>>, usize) {
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

// [GPT-6 Astra] Count capacities, not planner estimates or populated lengths.
// Scan rows have at most three ids and fit inline; decline an unproved spilled
// representation. Variable owns a String, whose capacity is exposed by its safe
// consuming API; move it out and back without cloning or allocating its text.
fn capped_rhs_storage(rhs: &mut Bindings, allowance: usize) -> Option<usize> {
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

// [GPT-6 Astra] Reuse requires a pure scan of the same immutable prepared pattern,
// filters and requested order. Actual sorted_by remains the scan's truthful value.
// Fitting entries live until order replacement/query exit; non-fitting entries live
// only in the caller's per-step scratch. Allocator metadata is outside this allowance.
fn capped_rhs<'a>(
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

