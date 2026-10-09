use super::*;

// ---- OPTIONAL / UNION / MINUS / VALUES / BIND --------------------------------

pub(super) fn left_outer_join(graph: &Graph, local: &mut LocalVocab, left: Bindings, right: Bindings, expr: Option<&Expression>) -> Result<Bindings, String> {
    let (out_vars, shared, right_only) = join_layout(&left, &right);
    let n_right_only = right_only.len();

    // Sort-merge left outer join for the common case — exactly one shared variable,
    // fully bound on both sides (the typical `?s p ?o OPTIONAL { ?s q ?r }`). It
    // avoids building a hash table over the entire right side (which dominates on
    // large OPTIONALs); QLever uses the same sorted-merge strategy here.
    if shared.len() == 1 {
        let (lk, rk) = shared[0];
        if !any_unbound(&left.rows, &[lk]) && !any_unbound(&right.rows, &[rk]) {
            return left_outer_merge(graph, local, left, right, lk, rk, &shared, &right_only, n_right_only, out_vars, expr);
        }
    }

    // Hash the right side by its shared-variable key when those columns are fully
    // bound; otherwise fall back to a compatibility scan (shared vars may be
    // unbound, in which case they act as wildcards).
    let lcols: Vec<usize> = shared.iter().map(|&(lc, _)| lc).collect();
    let rcols: Vec<usize> = shared.iter().map(|&(_, rc)| rc).collect();
    let table: Option<FxHashMap<Key, Posting>> =
        if !shared.is_empty() && !any_unbound(&left.rows, &lcols) && !any_unbound(&right.rows, &rcols) {
            let mut t: FxHashMap<Key, Posting> = FxHashMap::default();
            for (ri, row) in right.rows.iter().enumerate() {
                t.entry(rcols.iter().map(|&c| row[c]).collect()).or_default().push(ri);
            }
            Some(t)
        } else {
            None
        };

    let mut rows = Vec::new();
    for lrow in &left.rows {
        // Coarse budget check once per left row.
        if budget::exhausted(rows.len()) {
            break;
        }
        // Inline (no heap alloc per left row): an OPTIONAL key usually has 0–1
        // matches. The hashed branch copies the matching indices by value rather
        // than cloning the table's Vec.
        let candidates: SmallVec<[usize; 4]> = match &table {
            Some(t) => {
                let key: Key = lcols.iter().map(|&c| lrow[c]).collect();
                t.get(&key).map(|v| v.iter().copied().collect()).unwrap_or_default()
            }
            None => (0..right.rows.len()).filter(|&ri| compatible(lrow, &right.rows[ri], &shared)).collect(),
        };
        let mut matched = false;
        for ri in candidates {
            let combined = merge_rows(lrow, &right.rows[ri], &shared, &right_only);
            // OPTIONAL's filter is part of the join condition (evaluated on the
            // combined row); a row that fails it does not count as a match.
            let keep = match expr {
                None => true,
                Some(e) => {
                    let tmp = Bindings { vars: out_vars.clone(), rows: vec![], sorted_by: None };
                    effective_boolean(&eval_expr(graph, local, &tmp, &combined, e)?, local.ebv_semantics)
                }
            };
            if keep {
                rows.push(combined);
                matched = true;
            }
        }
        if !matched {
            let mut combined = lrow.clone();
            combined.extend(std::iter::repeat_n(NO_ID, n_right_only));
            rows.push(combined);
        }
    }
    Ok(Bindings::unsorted(out_vars, rows))
}

/// Sort-merge left outer join on a single shared variable (`lk`/`rk` columns).
#[allow(clippy::too_many_arguments)]
pub(super) fn left_outer_merge(
    graph: &Graph,
    local: &mut LocalVocab,
    mut left: Bindings,
    mut right: Bindings,
    lk: usize,
    rk: usize,
    shared: &[(usize, usize)],
    right_only: &[usize],
    n_right_only: usize,
    out_vars: Vec<Variable>,
    expr: Option<&Expression>,
) -> Result<Bindings, String> {
    // Both sides come from index scans that are usually already key-sorted, so
    // these sorts are near-linear (pattern-defeating quicksort detects sortedness).
    left.rows.sort_unstable_by_key(|r| r[lk]);
    right.rows.sort_unstable_by_key(|r| r[rk]);
    let (l, r) = (&left.rows, &right.rows);
    let mut rows: Vec<Row> = Vec::with_capacity(l.len());
    let mut j = 0usize;
    let mut i = 0usize;
    while i < l.len() {
        // Coarse budget check once per key group.
        if budget::exhausted(rows.len()) {
            break;
        }
        let key = l[i][lk];
        let mut i2 = i + 1;
        while i2 < l.len() && l[i2][lk] == key {
            i2 += 1;
        }
        while j < r.len() && r[j][rk] < key {
            j += 1;
        }
        let mut j2 = j;
        while j2 < r.len() && r[j2][rk] == key {
            j2 += 1;
        }
        for lrow in l.iter().take(i2).skip(i) {
            let mut matched = false;
            for rrow in r.iter().take(j2).skip(j) {
                let combined = merge_rows(lrow, rrow, shared, right_only);
                let keep = match expr {
                    None => true,
                    Some(e) => {
                        let tmp = Bindings { vars: out_vars.clone(), rows: vec![], sorted_by: None };
                        effective_boolean(&eval_expr(graph, local, &tmp, &combined, e)?, local.ebv_semantics)
                    }
                };
                if keep {
                    rows.push(combined);
                    matched = true;
                }
            }
            if !matched {
                let mut combined = lrow.clone();
                combined.extend(std::iter::repeat_n(NO_ID, n_right_only));
                rows.push(combined);
            }
        }
        i = i2;
        j = j2;
    }
    // Output is ordered by the join variable (at column lk of out_vars).
    let sorted_by = out_vars.get(lk).cloned();
    Ok(Bindings { vars: out_vars, rows, sorted_by })
}
