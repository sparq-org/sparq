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

fn merge_join(left: Bindings, right: Bindings, jv: &Variable) -> Bindings {
    merge_join_ref(&left, &right, jv)
}

// [GPT-6 Astra] Borrow rows so the capped caller can reuse its RHS without cloning it.
fn merge_join_ref(left: &Bindings, right: &Bindings, jv: &Variable) -> Bindings {
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
fn join_layout(left: &Bindings, right: &Bindings) -> (Vec<Variable>, Vec<(usize, usize)>, Vec<usize>) {
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

// [OPUS-4.8] sq-hknqs (epic sq-qonbz, Phase 3): the id-tuple combine/compatibility helpers
// (`compatible` / `merge_rows` / `any_unbound`) now live in the shared substrate
// (`sparq_substrate::join`). The engine's OPTIONAL / UNION / MINUS / VALUES-UNDEF nested-loop
// fallbacks and the hash-join build/probe call them under these private aliases, so every
// existing `Bindings`-side call site is unchanged.
use sjoin::{any_unbound, compatible, merge_rows};
// The join hash (`key_hash` / `JOIN_PARTS`) is only reached by the radix-partitioned PARALLEL
// hash-join build (native only); the wasm / serial build computes the table without them.
#[cfg(feature = "parallel")]
use sjoin::{key_hash, JOIN_PARTS};

fn hash_join(left: Bindings, right: Bindings) -> Bindings {
    hash_join_ref(&left, &right)
}

// [GPT-6 Astra] Borrow rows so the capped caller can reuse its RHS without cloning it.
fn hash_join_ref(left: &Bindings, right: &Bindings) -> Bindings {
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
    // avoids a dependency on rustc_hash::FxHashMap in the type annotation. [SONNET-4.6] sq-7d3dj.19
    #[cfg(feature = "parallel")]
    let tables = if build.rows.len() >= PAR_THRESHOLD {
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
    if probe.rows.len() >= PAR_THRESHOLD {
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
