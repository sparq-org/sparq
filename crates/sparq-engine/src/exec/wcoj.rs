use super::*;

// ---- Worst-case-optimal join: Leapfrog Triejoin ------------------------------
//
// LFTJ (Veldhuizen 2014) evaluates a BGP one *variable* at a time in a fixed
// global order. At each variable it intersects, via the "leapfrog" galloping
// search, the sorted value streams of every pattern that mentions that variable,
// then recurses. Each pattern is a trie of its variable columns (projected from a
// permutation index, sorted in the global variable order). The total work is
// bounded by the AGM fractional-edge-cover bound on the BGP, so for cyclic
// queries it cannot produce the asymptotically-large intermediates a binary plan
// would. See research/ARCHITECTURE.md §4.

// sq-hknqs (epic sq-qonbz, Phase 3): the LFTJ NAVIGATION — the `Trie`/`TrieIter`
// cursor (open-on-entry, galloping `seek`, binary-search `run_end`), the `Leapfrog`
// intersection and `lftj_recurse` — now lives in the shared substrate
// (`sparq_substrate::join::{Trie, TrieIter, lftj_recurse}`). The engine keeps only
// `build_trie` below (which OWNS the store scan + the zk-trace hook + the global-order
// projection) and `eval_bgp_wcoj`, which drives the substrate cursor with the engine's
// budget. The probe loop is monomorphic over `Id`/`Row` with no `Box<dyn>`.

/// Builds a pattern's trie of projected variable tuples, plus the global levels
/// of those variables (sorted ascending). Repeated-variable patterns keep only
/// rows where all positions of a variable agree.
pub(super) fn build_trie(
    graph: &Graph,
    id_pat: &IdPattern,
    pos_vars: &[Option<Variable>; 3],
    var_levels: &FxHashMap<Variable, usize>,
) -> (sjoin::Trie, Vec<usize>) {
    let mut var_positions: Vec<(Variable, Vec<usize>)> = Vec::new();
    for (pos, ov) in pos_vars.iter().enumerate() {
        if let Some(v) = ov {
            if let Some(e) = var_positions.iter_mut().find(|(x, _)| x == v) {
                e.1.push(pos);
            } else {
                var_positions.push((v.clone(), vec![pos]));
            }
        }
    }
    var_positions.sort_by_key(|(v, _)| var_levels[v]);
    let levels: Vec<usize> = var_positions.iter().map(|(v, _)| var_levels[v]).collect();

    let scan = graph.store.scan(id_pat);
    let mut tuples: Vec<Vec<Id>> = Vec::with_capacity(scan.rows.len());
    // zk-trace hook: the WCOJ path's per-pattern input set is the trie's
    // source scan (the consistency-passing rows), recorded pre-projection.
    #[cfg(feature = "zk")]
    let mut zk_matched: Vec<[Id; 3]> = Vec::new();
    for row in scan.rows.iter() {
        let spo = scan.to_spo(row);
        let mut tup = Vec::with_capacity(var_positions.len());
        let mut ok = true;
        for (_, positions) in &var_positions {
            let v0 = spo[positions[0]];
            if positions.iter().any(|&p| spo[p] != v0) {
                ok = false;
                break;
            }
            tup.push(v0);
        }
        if ok {
            #[cfg(feature = "zk")]
            if crate::zk::enabled() {
                zk_matched.push(spo);
            }
            tuples.push(tup);
        }
    }
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        crate::zk::record_scan_ids(graph, id_pat, pos_vars, &zk_matched, false);
    }
    tuples.sort_unstable();
    tuples.dedup();
    (sjoin::Trie { tuples }, levels)
}

pub(super) fn eval_bgp_wcoj(graph: &Graph, patterns: &[TriplePattern]) -> Result<Bindings, String> {
    // Prepare patterns; an unsatisfiable constant makes the whole BGP empty.
    let mut prepared: Vec<(IdPattern, [Option<Variable>; 3])> = Vec::with_capacity(patterns.len());
    for tp in patterns {
        let (id_pat, pos_vars, unsat) = prepare_pattern(graph, tp)?;
        if unsat {
            // zk-trace: record ONLY this provably-empty pattern (see the
            // binary path) — siblings are not proven empty here.
            #[cfg(feature = "zk")]
            if crate::zk::enabled() {
                crate::zk::record_empty_pattern(crate::zk::key_of_algebra_pattern(tp));
            }
            return Ok(Bindings::unsorted(collect_vars(patterns), vec![]));
        }
        prepared.push((id_pat, pos_vars));
    }

    // Global variable order: most-constrained first (highest degree, then most
    // selective). Constant-only patterns contribute no variables but must match.
    // (Shared with the T22 EXPLAIN dry run.)
    let order_vars = wcoj_global_order(graph, patterns, &prepared);
    let n_levels = order_vars.len();

    let var_levels: FxHashMap<Variable, usize> =
        order_vars.iter().enumerate().map(|(i, v)| (v.clone(), i)).collect();

    // Build a trie per pattern that has variables; check constant-only patterns
    // for existence (empty range => empty BGP).
    let mut tries: Vec<sjoin::Trie> = Vec::new();
    let mut trie_levels: Vec<Vec<usize>> = Vec::new();
    for (id_pat, pos_vars) in &prepared {
        if pos_vars.iter().all(|v| v.is_none()) {
            if graph.store.estimate(id_pat) == 0 {
                return Ok(Bindings::unsorted(order_vars, vec![]));
            }
            continue;
        }
        let (trie, levels) = build_trie(graph, id_pat, pos_vars, &var_levels);
        if trie.tuples.is_empty() {
            return Ok(Bindings::unsorted(order_vars, vec![]));
        }
        tries.push(trie);
        trie_levels.push(levels);
    }

    // No variables: BGP is a ground check that already succeeded.
    if n_levels == 0 {
        return Ok(Bindings { vars: vec![], rows: vec![Row::new()], sorted_by: None });
    }

    // Participating tries per global level.
    let mut parts_at_level: Vec<Vec<usize>> = vec![Vec::new(); n_levels];
    for (ti, levels) in trie_levels.iter().enumerate() {
        for &lvl in levels {
            parts_at_level[lvl].push(ti);
        }
    }

    // The LFTJ navigation (cursor + leapfrog intersection + recursion) is the shared substrate's
    // (sq-hknqs); the engine supplies the built tries, the per-level participation, and its
    // thread-local budget as the generic cooperative-cancel hook.
    let mut iters: Vec<sjoin::TrieIter> = tries.iter().map(sjoin::TrieIter::new).collect();
    let mut out: Vec<Row> = Vec::new();
    let mut current = vec![NO_ID; n_levels];
    sjoin::lftj_recurse(&mut iters, &parts_at_level, 0, n_levels, &mut current, &EngineBudget, &mut out);

    // Output rows are produced in global-order lexicographic order.
    let sorted_by = order_vars.first().cloned();
    Ok(Bindings { vars: order_vars, rows: out, sorted_by })
}

/// The Leapfrog-Triejoin global variable order: most-constrained variables first
/// (highest pattern degree, ties broken by the smallest estimate among the
/// patterns mentioning the variable). Shared by `eval_bgp_wcoj` and EXPLAIN.
pub(crate) fn wcoj_global_order(
    graph: &Graph,
    patterns: &[TriplePattern],
    prepared: &[(IdPattern, [Option<Variable>; 3])],
) -> Vec<Variable> {
    let degree = |v: &Variable| prepared.iter().filter(|(_, pv)| pv.iter().flatten().any(|x| x == v)).count();
    let min_est = |v: &Variable| {
        prepared
            .iter()
            .filter(|(_, pv)| pv.iter().flatten().any(|x| x == v))
            .map(|(ip, _)| graph.store.estimate(ip))
            .min()
            .unwrap_or(usize::MAX)
    };
    let mut order_vars = collect_vars(patterns);
    order_vars.sort_by(|a, b| degree(b).cmp(&degree(a)).then(min_est(a).cmp(&min_est(b))));
    order_vars
}

/// α-acyclicity test via GYO reduction: repeatedly drop variables that occur in
/// only one pattern and patterns whose variable set is contained in another.
/// A BGP is cyclic iff anything remains. Cyclic BGPs benefit from WCOJ.
pub(super) fn bgp_is_cyclic(patterns: &[TriplePattern]) -> bool {
    let mut next_id = 0u32;
    let mut ids: FxHashMap<Variable, u32> = FxHashMap::default();
    let mut edges: Vec<std::collections::HashSet<u32>> = Vec::new();
    for tp in patterns {
        let mut e = std::collections::HashSet::new();
        for v in [tp_var(&tp.subject), nnp_var(&tp.predicate), tp_var(&tp.object)].into_iter().flatten() {
            let id = *ids.entry(v).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            });
            e.insert(id);
        }
        if !e.is_empty() {
            edges.push(e);
        }
    }

    loop {
        let mut changed = false;

        // Drop variables occurring in exactly one edge.
        let mut count: FxHashMap<u32, usize> = FxHashMap::default();
        for e in &edges {
            for &v in e {
                *count.entry(v).or_default() += 1;
            }
        }
        for e in edges.iter_mut() {
            let before = e.len();
            e.retain(|v| count[v] > 1);
            if e.len() != before {
                changed = true;
            }
        }
        let before = edges.len();
        edges.retain(|e| !e.is_empty());
        if edges.len() != before {
            changed = true;
        }

        // Drop an edge contained in a distinct edge (also removes duplicates).
        let mut removed = None;
        'outer: for i in 0..edges.len() {
            for j in 0..edges.len() {
                if i != j && edges[i].is_subset(&edges[j]) {
                    removed = Some(i);
                    break 'outer;
                }
            }
        }
        if let Some(i) = removed {
            edges.remove(i);
            changed = true;
        }

        if !changed {
            break;
        }
    }

    !edges.is_empty()
}

pub(super) fn prepare_pattern(graph: &Graph, tp: &TriplePattern) -> Result<(IdPattern, [Option<Variable>; 3], bool), String> {
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

pub(super) fn scan_to_bindings(
    graph: &Graph,
    id_pat: &IdPattern,
    pos_vars: &[Option<Variable>; 3],
    sort_col: Option<usize>,
    filter: Option<(usize, ScanCmp)>,
    limit: Option<usize>,
    // (sq-gr8mb / §A3) Optional semi-join prefilter: `(canonical position of
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
        // (sq-gr8mb / §A3) Semi-join prefilter: drop a row whose connecting-
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
    if limit.is_none() && scan_rows.len() >= PAR_THRESHOLD && !budget::evaluation_capacity_active() {
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

pub(super) fn merge_join(left: Bindings, right: Bindings, jv: &Variable) -> Bindings {
    merge_join_ref(&left, &right, jv)
}

// Borrow rows so the capped caller can reuse its RHS without cloning it.
pub(super) fn merge_join_ref(left: &Bindings, right: &Bindings, jv: &Variable) -> Bindings {
    let lk = left.col(jv).unwrap();
    let rk = right.col(jv).unwrap();
    let mut out_vars = left.vars.clone();
    let mut right_only: Vec<usize> = Vec::new();
    let mut extra_shared: Vec<(usize, usize)> = Vec::new();
    for (ri, v) in right.vars.iter().enumerate() {
        match left.col(v) {
            Some(li) if v != jv => extra_shared.push((li, ri)),
            Some(_) => {}
            None => {
                out_vars.push(v.clone());
                right_only.push(ri);
            }
        }
    }
    // The sorted-merge probe loop lives in the shared substrate (sq-hknqs); the engine supplies
    // the `Bindings`-derived key columns + its thread-local query budget, monomorphically.
    let mut rows = Vec::new();
    sjoin::merge_join(&left.rows, lk, &right.rows, rk, &extra_shared, &right_only, &EngineBudget, &mut rows);
    Bindings { vars: out_vars, rows, sorted_by: Some(jv.clone()) }
}

/// Layout for combining two bindings: shared (left col, right col) pairs and the
/// right-only columns appended after left's vars.
pub(super) fn join_layout(left: &Bindings, right: &Bindings) -> (Vec<Variable>, Vec<(usize, usize)>, Vec<usize>) {
    let mut out_vars = left.vars.clone();
    let mut shared = Vec::new();
    let mut right_only = Vec::new();
    for (ri, v) in right.vars.iter().enumerate() {
        match left.col(v) {
            Some(li) => shared.push((li, ri)),
            None => {
                out_vars.push(v.clone());
                right_only.push(ri);
            }
        }
    }
    (out_vars, shared, right_only)
}


pub(super) fn hash_join(left: Bindings, right: Bindings) -> Bindings {
    hash_join_ref(&left, &right)
}

// Borrow rows so the capped caller can reuse its RHS without cloning it.
pub(super) fn hash_join_ref(left: &Bindings, right: &Bindings) -> Bindings {
    // Build the hash table on the smaller side.
    let (build, probe) = if left.rows.len() <= right.rows.len() {
        (left, right)
    } else {
        (right, left)
    };
    // Shared vars relative to (build, probe).
    let shared: Vec<(usize, usize)> = build
        .vars
        .iter()
        .enumerate()
        .filter_map(|(bi, v)| probe.col(v).map(|pi| (bi, pi)))
        .collect();
    let mut out_vars = build.vars.clone();
    let probe_only: Vec<usize> = probe
        .vars
        .iter()
        .enumerate()
        .filter(|(_, v)| !build.vars.contains(v))
        .map(|(i, v)| {
            out_vars.push(v.clone());
            i
        })
        .collect();
    // The join column layout for the shared substrate kernel: `key_cols` are the (build, probe)
    // shared-variable index pairs (the equi-join key); the probe-only columns are appended after
    // the build row. The build/probe phases below are the substrate's `build_*` / `probe_emit`
    // (sq-hknqs) — the engine only supplies this `Bindings`-derived layout and its budget.
    let keys = sjoin::JoinKeys { key_cols: shared.clone(), right_only: Vec::new() };
    // Build phase. Above PAR_THRESHOLD the build is radix-partitioned (Tier-1 #5 of
    // research/parallelism-scaling.md): rows are tagged with their key-hash partition in
    // parallel, then each partition builds its private map lock-free. Within a partition rows
    // are scanned in ascending index, so each posting list stays in ascending build-row order —
    // exactly the serial build — and the probe output is byte-identical.
    // JoinTable = hashbrown::HashMap<Key, Posting, FxBuildHasher>; the type inference here
    // avoids a dependency on rustc_hash::FxHashMap in the type annotation. sq-7d3dj.19
    #[cfg(feature = "parallel")]
    let tables = if build.rows.len() >= PAR_THRESHOLD && !budget::evaluation_capacity_active() {
        use rayon::prelude::*;
        let parts: Vec<u8> = build
            .rows
            .par_iter()
            .map(|row| (key_hash(&keys.left_key(row)) % JOIN_PARTS as u64) as u8)
            .collect();
        sjoin::build_partitioned(&build.rows, &keys, &parts)
    } else {
        vec![sjoin::build_table(&build.rows, &keys)]
    };
    #[cfg(not(feature = "parallel"))]
    let tables = vec![sjoin::build_table(&build.rows, &keys)];
    // The probe is read-only over the (partitioned) table, so for a large probe side build the
    // output in parallel on native.
    #[cfg(feature = "parallel")]
    if probe.rows.len() >= PAR_THRESHOLD && !budget::evaluation_capacity_active() {
        use rayon::prelude::*;
        // Budget snapshot for the workers (the installing thread's thread-local is
        // invisible to them): a worker that hits the limits stops adding to its own
        // accumulator; the caller's next on-thread check raises the actual error.
        let snap = EngineSnapshot(budget::snapshot());
        let rows: Vec<Row> = probe
            .rows
            .par_iter()
            .fold(Vec::new, |mut acc, prow| {
                if !sjoin::BudgetSnapshot::hit(&snap, acc.len()) {
                    sjoin::probe_emit(prow, &keys, &build.rows, &tables, &probe_only, &mut acc);
                }
                acc
            })
            .reduce(Vec::new, |mut a, mut b| {
                a.append(&mut b);
                a
            });
        let _ = budget::exhausted(rows.len()); // sticky gate on the combined size
        return Bindings::unsorted(out_vars, rows);
    }
    let mut rows = Vec::new();
    sjoin::hash_probe_serial(&probe.rows, &keys, &build.rows, &tables, &probe_only, &EngineBudget, &mut rows);
    Bindings::unsorted(out_vars, rows)
}

/// Index-nested-loop join of a (small) `result` with a single triple pattern on one
/// shared variable: groups the result by the join value, and for each distinct value
/// looks up the pattern's matches with that variable BOUND (a binary-search range on a
/// permutation index) — so a large, selective pattern is never fully scanned. The
/// pattern must have distinct variables; a pushed-down sargable filter is applied inline.
pub(super) fn bind_join(
    graph: &Graph,
    result: Bindings,
    id_pat: &IdPattern,
    pos_vars: &[Option<Variable>; 3],
    rk: usize,
    pp: usize,
    filt: Option<(usize, ScanCmp)>,
) -> Bindings {
    // The pattern's NEW variable columns (every variable position except the join one;
    // the only shared variable is the join variable, so the rest are new).
    let new_positions: Vec<usize> = (0..3).filter(|&p| p != pp && pos_vars[p].is_some()).collect();
    let mut out_vars = result.vars.clone();
    for &p in &new_positions {
        out_vars.push(pos_vars[p].clone().unwrap());
    }

    // Verify metadata candidates without allocating: stale sortedness
    // must not split a key across runs and repeat scans (or accumulate duplicate zk
    // matches). Unsorted bags keep the existing hash grouping. Verified runs carry
    // contiguous row ranges, avoiding an allocated identity-index vector.
    let nrows = result.rows.len();
    let presorted = result.sorted_by.as_ref().is_some_and(|sv| result.vars.get(rk) == Some(sv))
        && result.rows.windows(2).all(|pair| pair[0][rk] <= pair[1][rk]);
    #[cfg(test)]
    bind_join_run_grouping::observe_strategy(presorted);
    enum GroupRows {
        Contiguous(std::ops::Range<usize>),
        Indexed(Vec<usize>),
    }
    let mut groups: FxHashMap<Id, Vec<usize>> = FxHashMap::default();
    if !presorted {
        for (ri, row) in result.rows.iter().enumerate() {
            groups.entry(row[rk]).or_default().push(ri);
        }
    }

    debug_assert!(!presorted || groups.is_empty());
    // Both strategies share the scan/filter/budget body. Owned hash-group vectors
    // still drop after their iteration; contiguous ranges contain only two indices.
    let mut start = 0usize;
    let runs = std::iter::from_fn(|| {
        if !presorted || start == nrows {
            return None;
        }
        let val = result.rows[start][rk];
        let mut end = start + 1;
        while end < nrows && result.rows[end][rk] == val {
            end += 1;
        }
        let range = start..end;
        start = end;
        Some((val, GroupRows::Contiguous(range)))
    });
    let hashed = groups.into_iter().map(|(val, ris)| (val, GroupRows::Indexed(ris)));

    let mut out_rows: Vec<Row> = Vec::new();
    // zk-trace hook: accumulate the matched triples across all bound rescans
    // of this pattern (recorded once, under the pattern's ORIGINAL key, so
    // the input set merges with any full scans of the same pattern).
    #[cfg(feature = "zk")]
    let mut zk_matched: Vec<[Id; 3]> = Vec::new();
    for (val, ris) in runs.chain(hashed) {
        // Coarse budget check once per distinct join value.
        if budget::exhausted(out_rows.len()) {
            break;
        }
        let mut bound = *id_pat;
        bound[pp] = Some(val);
        #[cfg(test)]
        bind_join_run_grouping::observe_scan();
        let scan = graph.store.scan(&bound);
        for prow in scan.rows.iter() {
            let pspo = scan.to_spo(prow);
            if let Some((fpos, cmp)) = filt {
                if !cmp.test_id(graph, pspo[fpos]) {
                    continue;
                }
            }
            #[cfg(feature = "zk")]
            if crate::zk::enabled() {
                zk_matched.push(pspo);
            }
            let new_vals: SmallVec<[Id; 4]> = new_positions.iter().map(|&p| pspo[p]).collect();
            // The per-(result-row, match) combine is the shared substrate's `bind_combine`
            // (sq-hknqs): the scan + filter pushdown above stay engine-private (they own the
            // store + `ScanCmp`); only the id-tuple combine is shared.
            match &ris {
                GroupRows::Contiguous(range) => {
                    sjoin::bind_combine_rows(&result.rows[range.clone()], &new_vals, &mut out_rows);
                }
                GroupRows::Indexed(indices) => {
                    sjoin::bind_combine(&result.rows, indices, &new_vals, &mut out_rows);
                }
            }
        }
    }
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        crate::zk::record_scan_ids(graph, id_pat, pos_vars, &zk_matched, true);
    }
    Bindings::unsorted(out_vars, out_rows)
}


pub(super) fn cross_product(left: Bindings, right: Bindings) -> Bindings {
    cross_product_ref(&left, &right)
}

// Borrow rows so the capped caller can reuse its RHS without cloning it.
pub(super) fn cross_product_ref(left: &Bindings, right: &Bindings) -> Bindings {
    let mut out_vars = left.vars.clone();
    out_vars.extend(right.vars.iter().cloned());
    let mut rows = Vec::with_capacity(budget::cap_alloc(left.rows.len().saturating_mul(right.rows.len())));
    for l in &left.rows {
        // Coarse budget check once per left row.
        if budget::exhausted(rows.len()) {
            break;
        }
        for r in &right.rows {
            let mut row = l.clone();
            row.extend_from_slice(r);
            rows.push(row);
        }
    }
    Bindings::unsorted(out_vars, rows)
}

/// Generic join used for Join of non-conjunctive sub-results. With fully-bound
/// shared columns it takes the fast path (merge if both are sorted on the join
/// var, else hash); when a shared column can be unbound (from OPTIONAL / UNION /
/// VALUES UNDEF), it falls back to a correct solution-compatibility nested loop.
pub(super) fn join_bindings(left: Bindings, right: Bindings) -> Bindings {
    let (out_vars, shared, right_only) = join_layout(&left, &right);
    if shared.is_empty() {
        return cross_product(left, right);
    }
    let lcols: Vec<usize> = shared.iter().map(|&(lc, _)| lc).collect();
    let rcols: Vec<usize> = shared.iter().map(|&(_, rc)| rc).collect();
    if !any_unbound(&left.rows, &lcols) && !any_unbound(&right.rows, &rcols) {
        if let (Some(lv), Some(rv)) = (&left.sorted_by, &right.sorted_by) {
            if lv == rv && right.col(lv).is_some() {
                let jv = lv.clone();
                return merge_join(left, right, &jv);
            }
        }
        return hash_join(left, right);
    }
    let mut rows = Vec::new();
    for lrow in &left.rows {
        // Coarse budget check once per left row.
        if budget::exhausted(rows.len()) {
            break;
        }
        for rrow in &right.rows {
            if compatible(lrow, rrow, &shared) {
                rows.push(merge_rows(lrow, rrow, &shared, &right_only));
            }
        }
    }
    Bindings::unsorted(out_vars, rows)
}
