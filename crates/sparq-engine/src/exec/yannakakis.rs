use super::*;

// ---- (sq-5zf8i / survey §A4) Yannakakis full-semijoin prepass -------
//
// research/codebase-improvement-opportunities-2026-06-23.md §A4 +
// research/optimization-techniques.md §1.1/§2(a2): for an ACYCLIC BGP, run a bottom-up
// then top-down FULL-SEMIJOIN sweep over the join tree BEFORE the main join, so the join
// never materialises a tuple that joins with nothing (a "dangling" tuple). This cuts the
// intermediate-result blow-up that is the materialising evaluator's #1 cost.
//
// REUSE of the A3 machinery: the membership test that drives each semijoin is the same
// exact `semijoin::KeyFilter` (bitmap on dense dictionary ids, exact set on sparse+huge
// keys) the A3 reducer uses; the cyclic/acyclic routing is the same `bgp_is_cyclic` /
// `bgp_uses_binary` GYO test the executor already has. The final join over the reduced
// relations reuses the executor's generic conjunctive `join_bindings`.
//
// CORRECTNESS (load-bearing). A semijoin is a pure FILTER — it removes only rows the
// final join would itself drop — so the reduced relations produce the SAME join result as
// the unreduced ones, for any join order. The feature is OFF by default; when off none of
// this compiles and the executor path is byte-identical. The on==off result equivalence
// is asserted by tests/yannakakis_differential.rs (NOT feature-gated: runs in BOTH states,
// each query checked against an independent brute-force reference).

/// One materialised relation in the prepass: the pattern's scanned rows, plus the
/// canonical *column index* of each JOIN variable (a variable shared with at least one
/// OTHER pattern) — those are the only columns a semijoin ever filters on.
pub(super) struct SjRelation {
    pub(super) bindings: Bindings,
    /// `(variable, column index in `bindings.vars`)` for each join variable this relation
    /// binds. Non-join (private) variables are excluded — they never connect two relations.
    pub(super) join_cols: Vec<(Variable, usize)>,
}

/// Below this estimated max single-pattern cardinality, the intermediate results cannot
/// blow up enough for the prepass to pay for itself, so the executor skips the prepass and
/// uses the ordinary binary plan (pure-overhead guard). Tuned conservatively: the prepass
/// only ever helps when at least one relation is large. (Never affects RESULTS — only
/// whether the reduction runs.)
pub(super) const YANNAKAKIS_MIN_REL: usize = 4_096;

/// Acyclic-BGP executor with the Yannakakis full-semijoin prepass (opt-in `yannakakis`).
///
/// Plan: cost-gate on the planner estimates; if worth it, materialise each pattern once,
/// run a bottom-up (leaf → root) then top-down (root → leaf) semijoin sweep over the join
/// tree, then join the REDUCED relations with the existing conjunctive join. The result is
/// identical to `eval_bgp_binary` (semijoin reduction is answer-preserving).
pub(super) fn eval_bgp_yannakakis(
    graph: &Graph,
    patterns: &[TriplePattern],
    pat_filters: &[Option<(usize, ScanCmp)>],
) -> Result<Bindings, String> {
    let pfilter = |i: usize| -> Option<(usize, ScanCmp)> { pat_filters.get(i).copied().flatten() };
    // The prepass only helps a MULTI-pattern join. An empty BGP (`{}` → the unit row) and a
    // single pattern (no join to reduce) have nothing to reduce and their edge cases (unit
    // row, empty-default view, unsatisfiable constant) are handled precisely by the binary
    // executor — defer to it so the prepass never has to replicate that logic.
    if patterns.len() < 2 {
        return eval_bgp_binary(graph, patterns, pat_filters);
    }
    // zk-trace: when the per-obligation recorder is ARMED, the prover needs the established
    // executor's exact per-pattern matched-triple input sets and join order; the prepass
    // reduces relations and re-orders the join, which would change the recorded witness.
    // Defer to the binary plan while recording so the zk witness is unchanged (the prepass
    // is a perf-only optimisation; the un-reduced trace is the canonical one).
    #[cfg(feature = "zk")]
    if crate::zk::enabled() {
        return eval_bgp_binary(graph, patterns, pat_filters);
    }
    // The conjunctive-flattening / triple-term / empty-view short-circuits and the
    // unsatisfiable-constant short-circuit all live in `eval_bgp_binary`; defer to it for
    // those so the prepass only ever sees a plain, satisfiable, materialisable BGP. Every
    // fallback forwards `pat_filters` so pushed-down FILTER semantics are preserved.
    if view::default_is_empty() {
        return Ok(Bindings::unsorted(collect_vars(patterns), vec![]));
    }
    let (_rewritten, constraints) = extract_quoted_constraints(patterns);
    if !constraints.is_empty() {
        // Triple-term decomposition is a binary-plan concern; let the binary executor
        // handle the rewrite + structural-unification joins (no prepass over synthetics).
        return eval_bgp_binary(graph, patterns, pat_filters);
    }

    let prepared = prepare_bgp(graph, patterns)?;
    if prepared.iter().any(|p| p.unsatisfiable) {
        // An absent constant makes the BGP provably empty; reuse the binary path's
        // short-circuit (it also records the zk-trace empties).
        return eval_bgp_binary(graph, patterns, pat_filters);
    }

    // Cost-gate (pure-overhead guard): unless at least one relation is estimated large,
    // the intermediates are already tiny and the prepass is pure overhead — use the plain
    // binary plan. This never affects results, only whether the reduction runs.
    let max_est = prepared.iter().map(|p| p.est).max().unwrap_or(0);
    if max_est < YANNAKAKIS_MIN_REL {
        return eval_bgp_binary(graph, patterns, pat_filters);
    }

    // Identify the join variables: a variable mentioned by ≥2 patterns. Only these connect
    // relations and are ever semijoined; a variable private to one pattern is irrelevant to
    // the reduction. Count the NUMBER OF PATTERNS mentioning each variable (a variable
    // repeated within a single pattern counts once for that pattern).
    let all_vars = collect_vars(patterns);
    let mut var_degree: FxHashMap<Variable, usize> = FxHashMap::default();
    for p in &prepared {
        let mut seen: FxHashSet<&Variable> = FxHashSet::default();
        for v in p.pos_vars.iter().flatten() {
            seen.insert(v);
        }
        for v in seen {
            *var_degree.entry(v.clone()).or_default() += 1;
        }
    }
    let is_join_var = |v: &Variable| var_degree.get(v).copied().unwrap_or(0) >= 2;

    // If NO variable is shared, the BGP is a pure cross product — there is nothing to
    // reduce. Defer to the binary plan (which builds the cross product directly).
    if !all_vars.iter().any(is_join_var) {
        return eval_bgp_binary(graph, patterns, pat_filters);
    }

    // Materialise each pattern's relation once (ordinary scan, no prefilter — the prepass
    // computes its own, sharper, multi-edge reduction). Then record each relation's
    // join-variable columns for the sweep, and assign each join variable a dense id for the
    // pure tree builder.
    let mut var_id: FxHashMap<Variable, u32> = FxHashMap::default();
    let mut relations: Vec<SjRelation> = Vec::with_capacity(prepared.len());
    for (idx, p) in prepared.iter().enumerate() {
        // A pushed-down sargable FILTER on this pattern is applied during materialisation
        // (its own column order for range-pruning), exactly as the binary plan does — so a
        // FILTERed pattern feeds only its passing rows into the reduction and the join.
        let filt = pfilter(idx);
        let sort_col = filt.map(|(c, _)| c);
        let bindings = scan_to_bindings(
            graph,
            &p.id_pat,
            &p.pos_vars,
            sort_col,
            filt,
            None,
            #[cfg(feature = "semijoin-bitmap")]
            None,
        );
        // An empty relation makes the whole acyclic BGP empty (every join is inner).
        if bindings.rows.is_empty() {
            return Ok(Bindings::unsorted(all_vars, vec![]));
        }
        let mut join_cols: Vec<(Variable, usize)> = Vec::new();
        for (col, v) in bindings.vars.iter().enumerate() {
            if is_join_var(v) {
                join_cols.push((v.clone(), col));
                let next = var_id.len() as u32;
                var_id.entry(v.clone()).or_insert(next);
            }
        }
        relations.push(SjRelation { bindings, join_cols });
    }

    // Build the join tree over the relations' join-variable id sets (pure topology).
    let rel_vars: Vec<Vec<u32>> = relations
        .iter()
        .map(|r| r.join_cols.iter().map(|(v, _)| var_id[v]).collect())
        .collect();
    let tree = crate::semijoin::build_join_tree(&rel_vars);

    // BOTTOM-UP sweep (children before parents): the tree nodes are in pre-order, so
    // iterating in REVERSE visits each child before its parent. Semijoin each parent with
    // each of its children on the shared join variables — drops parent rows with no child
    // partner. After this pass the root holds only rows that survive every downstream join.
    for ni in (0..tree.len()).rev() {
        let node = tree[ni];
        if node.is_root() {
            continue;
        }
        let parent_rel = tree[node.parent].rel;
        semijoin_reduce(&mut relations, parent_rel, node.rel);
        if relations[parent_rel].bindings.rows.is_empty() {
            return Ok(Bindings::unsorted(all_vars, vec![]));
        }
    }

    // TOP-DOWN sweep (parents before children): iterate FORWARD; semijoin each child with
    // its (already fully-reduced) parent. After this pass EVERY relation holds only rows
    // that participate in the final answer — no dangling tuples remain.
    for &node in &tree {
        if node.is_root() {
            continue;
        }
        let parent_rel = tree[node.parent].rel;
        semijoin_reduce(&mut relations, node.rel, parent_rel);
        if relations[node.rel].bindings.rows.is_empty() {
            return Ok(Bindings::unsorted(all_vars, vec![]));
        }
    }

    // Join the reduced relations. Order by the tree (parent before child) so each join has
    // a shared variable with the accumulated result whenever the component is connected;
    // the generic `join_bindings` handles the shared-variable merge/hash and the
    // cross-product across disconnected components. The result equals the binary plan's.
    let mut order: Vec<usize> = tree.iter().map(|n| n.rel).collect();
    // De-dup defensively (tree places each rel once, but keep the invariant explicit).
    order.dedup();
    let mut iter = order.into_iter();
    let first = iter.next().expect("≥2 relations on this path");
    let mut acc = std::mem::replace(
        &mut relations[first].bindings,
        Bindings::unsorted(vec![], vec![]),
    );
    for ri in iter {
        let rhs = std::mem::replace(
            &mut relations[ri].bindings,
            Bindings::unsorted(vec![], vec![]),
        );
        acc = join_bindings(acc, rhs);
        if acc.rows.is_empty() {
            break;
        }
    }
    Ok(acc)
}

/// Semijoin `relations[target] ⋉ relations[source]` IN PLACE: keep only the `target` rows
/// whose join-key values (on the variables `target` and `source` SHARE) appear in
/// `source`. Uses the exact `semijoin::KeyFilter` membership test (reused from A3) on each
/// shared variable; a row survives iff it passes on EVERY shared variable. Pure filter —
/// removes only rows that would fail the join, so the answer is unchanged.
pub(super) fn semijoin_reduce(relations: &mut [SjRelation], target: usize, source: usize) {
    // Shared join variables and their (target_col, source_col) positions.
    let shared: Vec<(usize, usize)> = relations[target]
        .join_cols
        .iter()
        .filter_map(|(v, tcol)| {
            relations[source]
                .join_cols
                .iter()
                .find(|(sv, _)| sv == v)
                .map(|(_, scol)| (*tcol, *scol))
        })
        .collect();
    if shared.is_empty() {
        return; // not actually adjacent (disconnected component edge) — nothing to reduce.
    }
    // Build one exact membership filter per shared variable over the SOURCE's distinct key
    // values at that column. `KeyFilter::build` returns `None` only for an empty source —
    // but an empty relation short-circuits the whole BGP before we get here, so a present
    // source always yields a filter.
    let filters: Vec<(usize, crate::semijoin::KeyFilter)> = shared
        .iter()
        .filter_map(|&(tcol, scol)| {
            crate::semijoin::KeyFilter::build(relations[source].bindings.rows.iter().map(|r| r[scol]))
                .map(|kf| (tcol, kf))
        })
        .collect();
    if filters.len() != shared.len() {
        return; // defensive: a degenerate empty source — leave target unchanged.
    }
    relations[target]
        .bindings
        .rows
        .retain(|row| filters.iter().all(|(tcol, kf)| kf.contains(row[*tcol])));
    // A retain can break a previously-recorded sort order only by REMOVING rows, which
    // preserves the order of the survivors — so `sorted_by` (if any) stays valid. No reset
    // needed; the generic join re-derives sortedness as required.
}
