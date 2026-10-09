use super::*;

// ---- Shared GOO planning decisions (executor + T22 EXPLAIN dry run) -----------
//
// The greedy-ordering decisions of `eval_bgp_binary` are factored into the small
// pure helpers below so EXPLAIN can REPLAY the planner without executing anything
// and without duplicating the logic (no drift): the executor calls them on its hot
// path (inlined, zero extra cost), the explain module replays them symbolically.

/// One BGP triple pattern prepared for planning: resolved constant ids, the
/// variable at each canonical position, and the index-range cardinality estimate.
pub(crate) struct Prepared {
    pub(crate) id_pat: IdPattern,
    pub(crate) pos_vars: [Option<Variable>; 3],
    pub(crate) est: usize,
    pub(crate) unsatisfiable: bool,
}

impl Prepared {
    /// Canonical position (0=s, 1=p, 2=o) of `v` in this pattern, if present.
    #[inline]
    pub(crate) fn var_pos(&self, v: &Variable) -> Option<usize> {
        self.pos_vars.iter().position(|pv| pv.as_ref() == Some(v))
    }
}

/// Prepares every pattern of a BGP for planning (constant resolution + estimates).
pub(crate) fn prepare_bgp(graph: &Graph, patterns: &[TriplePattern]) -> Result<Vec<Prepared>, String> {
    let mut prepared: Vec<Prepared> = Vec::with_capacity(patterns.len());
    for tp in patterns {
        let (id_pat, pos_vars, unsat) = prepare_pattern(graph, tp)?;
        let est = if unsat { 0 } else { graph.store.estimate(&id_pat) };
        prepared.push(Prepared { id_pat, pos_vars, est, unsatisfiable: unsat });
    }
    Ok(prepared)
}

/// The planner's estimated FINAL output cardinality of a conjunctive (BGP) subtree —
/// the same number the EXPLAIN dry-run prints as the last `est N rows`, computed by
/// REPLAYING the GOO loop (`goo_seed` → `goo_pick`) without executing anything. Used
/// only by the structured EXPLAIN (`explain-json`) to attach the BGP node's estimate
/// next to its ANALYZE actual-row count so the q-error is the engine's own estimate
/// vs reality (not a re-derived number). Returns `0.0` for an empty/unsatisfiable BGP.
/// sq-u4lgr
#[cfg(feature = "explain-json")]
pub(crate) fn bgp_estimate(graph: &Graph, p: &GraphPattern) -> Result<f64, String> {
    let mut patterns = Vec::new();
    let mut filters = Vec::new();
    flatten_conjunction(p, &mut patterns, &mut filters);
    let _ = &filters; // selectivity of post-join filters is not modelled (matches the dry run).
    if patterns.is_empty() {
        return Ok(0.0);
    }
    let prepared = prepare_bgp(graph, &patterns)?;
    if prepared.iter().any(|p| p.unsatisfiable) {
        return Ok(0.0);
    }
    if !bgp_uses_binary(&patterns) {
        // WCOJ (cyclic) path: the dry run does not print a single final estimate, so
        // approximate with the smallest single-pattern estimate (a conservative lower
        // bound the q-error treats like any other estimate).
        return Ok(prepared.iter().map(|p| p.est).min().unwrap_or(0) as f64);
    }
    let mut cs_ctx = CsCtx::new(&prepared);
    let seed = goo_seed(&prepared);
    let mut cur_card = prepared[seed].est as f64;
    let mut var_ndv: FxHashMap<Variable, f64> = FxHashMap::default();
    let mut done = vec![false; prepared.len()];
    done[seed] = true;
    cs_ctx.note_done(seed);
    record_pattern_ndv(graph, &prepared, seed, cur_card, &mut var_ndv, &cs_ctx);
    for _ in 2..=prepared.len() {
        let (i, new_card, _connected) = goo_pick(graph, &prepared, &done, &var_ndv, cur_card, &cs_ctx);
        cur_card = new_card;
        done[i] = true;
        cs_ctx.note_done(i);
        record_pattern_ndv(graph, &prepared, i, cur_card, &mut var_ndv, &cs_ctx);
    }
    Ok(cur_card.max(0.0))
}

/// GOO seed choice: the pattern with the smallest single-pattern cardinality.
pub(crate) fn goo_seed(prepared: &[Prepared]) -> usize {
    (0..prepared.len()).min_by_key(|&i| prepared[i].est).unwrap()
}

/// The seed scan's requested sort column: a pushed-down filter scans in its own
/// column's order (sequential numeric access); otherwise sort by the first seed
/// variable shared with another pattern, to enable a merge join.
pub(crate) fn goo_seed_sort(prepared: &[Prepared], seed: usize, filter_col: Option<usize>) -> Option<usize> {
    if filter_col.is_some() {
        return filter_col;
    }
    prepared[seed]
        .pos_vars
        .iter()
        .flatten()
        .find(|v| (0..prepared.len()).any(|j| j != seed && prepared[j].var_pos(v).is_some()))
        .and_then(|v| prepared[seed].var_pos(v))
}

/// The characteristic-set planning context (the opt-in `cs-planner` feature) the
/// GOO helpers consult for STAR joins: with a `crate::cs::CsTable` installed
/// (see [`crate::with_cs_table`]), candidate scoring and subject-variable ndv for
/// patterns of the star shape `?s <p> ?o` come from the CS table instead of the
/// per-predicate independence model. Without the feature this is a zero-sized
/// no-op; without an installed table every method is `None` and the `PredStat`
/// path runs unchanged. Either way, only JOIN ORDER is affected — never results.
pub(crate) struct CsCtx {
    #[cfg(feature = "cs-planner")]
    pub(super) inner: Option<crate::cs::StarCtx>,
}

impl CsCtx {
    pub(crate) fn new(prepared: &[Prepared]) -> CsCtx {
        #[cfg(feature = "cs-planner")]
        {
            let inner = crate::cs::active().map(|table| {
                crate::cs::StarCtx::new(
                    table,
                    prepared.iter().map(|p| match (&p.pos_vars[0], p.id_pat) {
                        // The star shape: subject VARIABLE, bound predicate, unbound object.
                        (Some(v), [None, Some(pid), None]) => Some((v.clone(), pid)),
                        _ => None,
                    }),
                )
            });
            CsCtx { inner }
        }
        #[cfg(not(feature = "cs-planner"))]
        {
            let _ = prepared;
            CsCtx {}
        }
    }

    /// Marks pattern `i` joined, so later star estimates condition on it.
    #[inline]
    pub(crate) fn note_done(&mut self, i: usize) {
        #[cfg(feature = "cs-planner")]
        if let Some(s) = &mut self.inner {
            s.note_done(i);
        }
        #[cfg(not(feature = "cs-planner"))]
        let _ = i;
    }

    /// CS-based candidate output estimate (see `cs::StarCtx::pick_score`).
    #[inline]
    pub(super) fn pick_score(&self, i: usize, cur_card: f64) -> Option<f64> {
        #[cfg(feature = "cs-planner")]
        {
            self.inner.as_ref().and_then(|s| s.pick_score(i, cur_card))
        }
        #[cfg(not(feature = "cs-planner"))]
        {
            let _ = (i, cur_card);
            None
        }
    }

    /// CS-based subject-variable ndv (see `cs::StarCtx::subject_ndv`).
    #[inline]
    pub(super) fn subject_ndv(&self, i: usize) -> Option<f64> {
        #[cfg(feature = "cs-planner")]
        {
            self.inner.as_ref().and_then(|s| s.subject_ndv(i))
        }
        #[cfg(not(feature = "cs-planner"))]
        {
            let _ = i;
            None
        }
    }
}

/// Folds pattern `i`'s per-variable distinct-value estimates into the running
/// `var_ndv` map (each variable keeps its smallest — most selective — estimate,
/// capped by the running result cardinality). With a CS table installed, a star
/// pattern's SUBJECT variable uses the table's `Σ_{C ⊇ Q} count(C)` over the star
/// joined so far (call after `CsCtx::note_done`) instead of the `PredStat` marginal.
pub(crate) fn record_pattern_ndv(
    graph: &Graph,
    prepared: &[Prepared],
    i: usize,
    cur_card: f64,
    var_ndv: &mut FxHashMap<Variable, f64>,
    cs: &CsCtx,
) {
    let p = &prepared[i];
    for (pos, ov) in p.pos_vars.iter().enumerate() {
        if let Some(v) = ov {
            let raw = match pos {
                0 => cs.subject_ndv(i).unwrap_or_else(|| pattern_var_ndv(graph, &p.id_pat, pos, p.est)),
                _ => pattern_var_ndv(graph, &p.id_pat, pos, p.est),
            };
            let ndv = raw.min(cur_card.max(1.0));
            let e = var_ndv.entry(v.clone()).or_insert(ndv);
            *e = e.min(ndv);
        }
    }
}

/// One GOO step: the connected not-yet-joined pattern with the smallest estimated
/// join output (`|R| · |P| / max(ndv)` per shared variable), or — when nothing
/// connects — the smallest remaining pattern (cross product). Returns the chosen
/// pattern index, the updated result-cardinality estimate, and whether it was
/// connected. With a CS table installed, a star candidate's subject-variable
/// contribution is the table's conditional expansion `star(Q ∪ {p}) / star(Q)`
/// (predicate-correlation-aware) instead of the independence product; selectivity
/// over any OTHER shared variables keeps the independence model.
pub(crate) fn goo_pick(
    graph: &Graph,
    prepared: &[Prepared],
    done: &[bool],
    var_ndv: &FxHashMap<Variable, f64>,
    cur_card: f64,
    cs: &CsCtx,
) -> (usize, f64, bool) {
    let mut best: Option<(usize, f64)> = None;
    for i in 0..prepared.len() {
        if done[i] {
            continue;
        }
        let cs_score = cs.pick_score(i, cur_card);
        let mut sel = 1.0f64;
        let mut shared = cs_score.is_some();
        for (pos, ov) in prepared[i].pos_vars.iter().enumerate() {
            if let Some(v) = ov {
                // The subject variable of a CS-scored star candidate is already
                // accounted for by the conditional star estimate.
                if pos == 0 && cs_score.is_some() {
                    continue;
                }
                if let Some(&rndv) = var_ndv.get(v) {
                    shared = true;
                    let pndv = pattern_var_ndv(graph, &prepared[i].id_pat, pos, prepared[i].est);
                    sel /= rndv.max(pndv).max(1.0);
                }
            }
        }
        if !shared {
            continue;
        }
        let out = match cs_score {
            Some(star) => star * sel,
            None => cur_card * prepared[i].est as f64 * sel,
        };
        if best.is_none_or(|(_, bc)| out < bc) {
            best = Some((i, out));
        }
    }
    match best {
        Some((i, out)) => (i, out.max(0.0), true),
        None => {
            // Disconnected: smallest-cardinality remaining (cross product).
            let i = (0..prepared.len()).filter(|&j| !done[j]).min_by_key(|&j| prepared[j].est).unwrap();
            (i, cur_card * prepared[i].est as f64, false)
        }
    }
}

/// Estimated number of distinct values of the variable at canonical position
/// `pos` in a pattern, from the per-predicate characteristic stats. Falls back to
/// the pattern's cardinality (an upper bound) when the predicate is unbound or the
/// other terminal is bound (so the column is effectively keyed).
pub(super) fn pattern_var_ndv(graph: &Graph, id_pat: &IdPattern, pos: usize, est: usize) -> f64 {
    let est = (est as f64).max(1.0);
    #[cfg(feature = "persistent-stats")]
    if let Some(ndv) = id_pat[1].and_then(|pid| crate::stats::predicate_ndv(pid, pos)) {
        return (ndv as f64).clamp(1.0, est);
    }
    let stat = id_pat[1].and_then(|pid| graph.store.pred_stat(pid));
    match (pos, stat) {
        // subject var, predicate bound, object unbound -> distinct subjects of P
        (0, Some(s)) if id_pat[2].is_none() => (s.ndv_subj as f64).clamp(1.0, est),
        // object var, predicate bound, subject unbound -> distinct objects of P
        (2, Some(s)) if id_pat[0].is_none() => (s.ndv_obj as f64).clamp(1.0, est),
        _ => est,
    }
}

// ---- (sq-iywur) DP join-order planner bridge -----------------------
//
// Builds the DP planner's `crate::dp::QueryGraph` from the prepared BGP patterns
// (reusing the SAME `pattern_var_ndv` estimator the greedy planner consumes), runs
// the DPccp enumerator, and evaluates the chosen bushy join tree. Both functions are
// only compiled under the opt-in `dp-planner` feature and are reached only when a
// planner is installed with `with_dp_planner`; the default build never sees them.

/// Plans and evaluates a BGP with the DP enumerator, or returns `None` to fall back
/// to greedy GOO (see `crate::dp::plan` for the fall-back conditions). The prepared
/// patterns are assumed already satisfiable (the caller short-circuits an
/// unsatisfiable BGP before this point).
#[cfg(feature = "dp-planner")]
pub(super) fn eval_bgp_dp(
    graph: &Graph,
    prepared: &[Prepared],
    pat_filters: &[Option<(usize, ScanCmp)>],
    cfg: crate::dp::DpConfig,
) -> Option<Bindings> {
    let n = prepared.len();
    if !(2..=63).contains(&n) {
        return None;
    }
    // Dense-index every variable; record its pattern bitmask and per-pattern
    // distinct-value estimate (the join-selectivity input the greedy planner uses).
    let mut var_idx: FxHashMap<Variable, usize> = FxHashMap::default();
    let mut var_pats: Vec<u64> = Vec::new();
    let mut var_ndv: Vec<Vec<(usize, f64)>> = Vec::new();
    for (p, prep) in prepared.iter().enumerate() {
        for (pos, ov) in prep.pos_vars.iter().enumerate() {
            if let Some(v) = ov {
                let ndv = pattern_var_ndv(graph, &prep.id_pat, pos, prep.est);
                let k = *var_idx.entry(v.clone()).or_insert_with(|| {
                    var_pats.push(0);
                    var_ndv.push(Vec::new());
                    var_pats.len() - 1
                });
                var_pats[k] |= 1u64 << p;
                // A variable can appear at several positions of one pattern — keep its
                // most selective (smallest) per-pattern estimate.
                if let Some(e) = var_ndv[k].iter_mut().find(|(pp, _)| *pp == p) {
                    e.1 = e.1.min(ndv);
                } else {
                    var_ndv[k].push((p, ndv));
                }
            }
        }
    }
    let est: Vec<f64> = prepared.iter().map(|p| p.est as f64).collect();
    let qg = crate::dp::QueryGraph::build(est, var_pats, var_ndv);
    let tree = crate::dp::plan(&qg, cfg.max_subgraphs)?;
    Some(eval_join_tree(graph, prepared, &tree, pat_filters))
}

// ---- (sq-7d3dj.30.14) membership-cluster pre-materialisation ---------
//
// Given a `crate::cluster::ClusterPlan` partition of the BGP, evaluate the two
// sub-BGPs (the standalone {anchor, membership} cluster and the driver `rest`)
// INDEPENDENTLY with the ordinary greedy `eval_bgp_binary`, then NATURAL-JOIN them
// with the shared `join_bindings`. Both sides are evaluated UNCORRELATED, so the
// materialised cluster is the full union-compatible relation greedy would also have
// produced — there is no cross-site sideways-information hazard (there is only one
// evaluation of the cluster). A BGP is a commutative/associative natural join, so the
// result bag is IDENTICAL to the flat greedy plan for any partition — the win is
// purely that bounding the shared variable from the small anchor avoids driving the
// wide unbound-predicate relation per `rest` binding. Only compiled under the opt-in
// `cluster-materialize` feature.
//
// `pat_filters[i]` is a (COLUMN-position, cmp) pushed-down sargable filter for pattern
// `i`; the column position is intrinsic to the pattern (not a BGP-relative index), so
// re-slicing per sub-BGP needs no remapping. `detect` has already declined any cluster
// pattern that carries a filter, so a cluster pattern's slot is always `None` here; the
// `rest` slots carry through verbatim.
#[cfg(feature = "cluster-materialize")]
pub(super) fn eval_bgp_cluster(
    graph: &Graph,
    patterns: &[TriplePattern],
    pat_filters: &[Option<(usize, ScanCmp)>],
    plan: &crate::cluster::ClusterPlan,
) -> Result<Bindings, String> {
    let sub = |idxs: &[usize]| -> (Vec<TriplePattern>, Vec<Option<(usize, ScanCmp)>>) {
        (
            idxs.iter().map(|&i| patterns[i].clone()).collect(),
            idxs.iter().map(|&i| pat_filters.get(i).copied().flatten()).collect(),
        )
    };
    let (cluster_pats, cluster_filts) = sub(&plan.cluster);
    let (rest_pats, rest_filts) = sub(&plan.rest);
    let cluster = eval_bgp_binary(graph, &cluster_pats, &cluster_filts)?;
    // An empty cluster makes the whole (inner) BGP empty — short-circuit like the
    // greedy loop does when a bind join empties the running result.
    if cluster.rows.is_empty() {
        return Ok(Bindings::unsorted(collect_vars(patterns), vec![]));
    }
    let rest = eval_bgp_binary(graph, &rest_pats, &rest_filts)?;
    Ok(join_bindings(cluster, rest))
}

/// Evaluates a DP `JoinTree`: a `Leaf` scans its pattern (applying any pushed-down
/// sargable filter, exactly like the greedy seed scan), a `Join` natural-joins the
/// two sub-results with the shared `join_bindings` (merge / hash / compatibility
/// nested-loop). The tree is bushy; `join_bindings` builds on the smaller side.
#[cfg(feature = "dp-planner")]
pub(super) fn eval_join_tree(
    graph: &Graph,
    prepared: &[Prepared],
    tree: &crate::dp::JoinTree,
    pat_filters: &[Option<(usize, ScanCmp)>],
) -> Bindings {
    match tree {
        crate::dp::JoinTree::Leaf(i) => {
            let p = &prepared[*i];
            scan_to_bindings(
                graph,
                &p.id_pat,
                &p.pos_vars,
                None,
                pat_filters.get(*i).copied().flatten(),
                None,
                #[cfg(feature = "semijoin-bitmap")]
                None,
            )
        }
        crate::dp::JoinTree::Join(l, r) => {
            let left = eval_join_tree(graph, prepared, l, pat_filters);
            let right = eval_join_tree(graph, prepared, r, pat_filters);
            join_bindings(left, right)
        }
    }
}

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
