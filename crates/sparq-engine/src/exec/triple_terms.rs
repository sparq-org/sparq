use super::*;

// ---- RDF 1.2 triple-term patterns with variables (F14) ----------------------

/// Prefix for the synthetic variables standing in for a triple-term pattern slot
/// (like [`BNODE_VAR_PREFIX`], `#` cannot appear in a SPARQL VARNAME).
pub(super) const QT_VAR_PREFIX: &str = "#qt#";

/// One BGP slot that held a triple-term pattern CONTAINING VARIABLES, replaced by a
/// synthetic variable. Its relation enumerates every stored triple term and unifies it
/// structurally against the triple-term pattern, binding the inner variables.
pub(super) struct QuotedConstraint {
    pub(super) var: Variable,
    pub(super) pattern: TriplePattern,
}

/// `true` when a triple-term pattern has a variable / blank node anywhere inside
/// (such a pattern cannot resolve to a single dictionary id).
pub(super) fn quoted_has_var(t: &TriplePattern) -> bool {
    fn slot(tp: &TermPattern) -> bool {
        match tp {
            TermPattern::Variable(_) | TermPattern::BlankNode(_) => true,
            TermPattern::Triple(inner) => quoted_has_var(inner),
            _ => false,
        }
    }
    slot(&t.subject) || matches!(t.predicate, NamedNodePattern::Variable(_)) || slot(&t.object)
}

/// Rewrites the BGP: every subject/object slot holding a variable-carrying triple-term
/// pattern becomes a fresh synthetic variable, with the triple-term pattern recorded as a
/// [`QuotedConstraint`]. Ground triple terms are untouched (they resolve to one id).
pub(super) fn extract_quoted_constraints(patterns: &[TriplePattern]) -> (Vec<TriplePattern>, Vec<QuotedConstraint>) {
    let mut out = Vec::with_capacity(patterns.len());
    let mut constraints: Vec<QuotedConstraint> = Vec::new();
    for tp in patterns {
        let mut tp = tp.clone();
        for s in [&mut tp.subject, &mut tp.object] {
            if let TermPattern::Triple(t) = s {
                if quoted_has_var(t) {
                    let var = Variable::new_unchecked(format!("{QT_VAR_PREFIX}{}", constraints.len()));
                    constraints.push(QuotedConstraint { var: var.clone(), pattern: (**t).clone() });
                    *s = TermPattern::Variable(var);
                }
            }
        }
        out.push(tp);
    }
    (out, constraints)
}

/// The variables of a triple-term pattern in first-occurrence order (blank nodes as
/// their synthetic existential variables), appended to `out` without duplicates.
pub(super) fn collect_quoted_vars(t: &TriplePattern, out: &mut Vec<Variable>) {
    fn push(out: &mut Vec<Variable>, v: Variable) {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    fn slot(tp: &TermPattern, out: &mut Vec<Variable>) {
        match tp {
            TermPattern::Variable(v) => push(out, v.clone()),
            TermPattern::BlankNode(b) => push(out, bnode_var(b)),
            TermPattern::Triple(inner) => collect_quoted_vars(inner, out),
            _ => {}
        }
    }
    slot(&t.subject, out);
    if let NamedNodePattern::Variable(v) = &t.predicate {
        push(out, v.clone());
    }
    slot(&t.object, out);
}

/// Builds the constraint's relation: one row per stored triple term that structurally
/// unifies with the quoted pattern — columns are the synthetic slot variable (bound to
/// the triple term's own id) followed by the quoted pattern's inner variables.
///
/// Enumeration scans the dictionary for `TermParts::Triple` records: triple terms are a
/// vanishing fraction of real dictionaries and the scan only runs for queries that quote
/// variables, so no ordinary query pays for it. (A persistent side index of triple-term
/// ids is the obvious upgrade if quoted-pattern workloads ever matter at scale.)
pub(super) fn quoted_relation(graph: &Graph, c: &QuotedConstraint) -> Bindings {
    let mut vars = vec![c.var.clone()];
    collect_quoted_vars(&c.pattern, &mut vars);
    let mut rows: Vec<Row> = Vec::new();
    for id in 1..=graph.dict.len() as Id {
        let dict::TermParts::Triple(comps) = graph.dict.term_parts(id) else {
            continue;
        };
        let mut binds: Row = std::iter::repeat_n(NO_ID, vars.len()).collect();
        binds[0] = id;
        if unify_quoted(graph, &c.pattern, comps, &vars, &mut binds) {
            rows.push(binds);
        }
    }
    Bindings::unsorted(vars, rows)
}

/// Binds `v` to `id`, or checks consistency when the variable is already bound
/// (the same variable repeated inside a quoted pattern must match the same id).
pub(super) fn bind_quoted_var(v: &Variable, id: Id, vars: &[Variable], binds: &mut [Id]) -> bool {
    let i = vars.iter().position(|x| x == v).expect("quoted var collected");
    if binds[i] == NO_ID {
        binds[i] = id;
        true
    } else {
        binds[i] == id
    }
}

/// Structurally unifies a triple-term pattern against a stored triple term's
/// component ids, recursing through nested triple-term patterns.
pub(super) fn unify_quoted(graph: &Graph, pat: &TriplePattern, comps: [Id; 3], vars: &[Variable], binds: &mut [Id]) -> bool {
    fn slot(graph: &Graph, tp: &TermPattern, id: Id, vars: &[Variable], binds: &mut [Id]) -> bool {
        match tp {
            TermPattern::Variable(v) => bind_quoted_var(v, id, vars, binds),
            TermPattern::BlankNode(b) => bind_quoted_var(&bnode_var(b), id, vars, binds),
            TermPattern::Triple(inner) => {
                if dict::is_inline(id) {
                    return false;
                }
                match graph.dict.term_parts(id) {
                    dict::TermParts::Triple(c) => unify_quoted(graph, inner, c, vars, binds),
                    _ => false,
                }
            }
            // A ground component: term-identity match (same as ordinary BGP slots).
            other => match term_pattern_to_term(other) {
                Ok(t) => graph.id_of(&t) == Some(id),
                Err(_) => false,
            },
        }
    }
    if !slot(graph, &pat.subject, comps[0], vars, binds) {
        return false;
    }
    match &pat.predicate {
        NamedNodePattern::NamedNode(n) => {
            if graph.id_of(&Term::NamedNode(n.clone())) != Some(comps[1]) {
                return false;
            }
        }
        NamedNodePattern::Variable(v) => {
            if !bind_quoted_var(v, comps[1], vars, binds) {
                return false;
            }
        }
    }
    slot(graph, &pat.object, comps[2], vars, binds)
}

/// Whether this conjunctive BGP would be routed to the worst-case-optimal plan
/// (so the caller knows whether sargable-filter pushdown into the binary scan
/// applies).
pub(crate) fn bgp_uses_binary(patterns: &[TriplePattern]) -> bool {
    !(patterns.len() >= 3 && bgp_is_cyclic(patterns))
}

/// Binary-join BGP plan: greedy cardinality ordering with sort-merge joins on the
/// current sort variable (falling back to hash, then cross product). `pat_filters`
/// holds an optional pushed-down numeric FILTER per pattern (by original index).
pub(super) fn eval_bgp_binary(graph: &Graph, patterns: &[TriplePattern], pat_filters: &[Option<(usize, ScanCmp)>]) -> Result<Bindings, String> {
    if patterns.is_empty() {
        return Ok(Bindings { vars: vec![], rows: vec![Row::new()], sorted_by: None });
    }
    // L1 dataset view: the conjunctive-flattening path calls this directly
    // (bypassing eval_bgp), so the empty-default short-circuit must be here too.
    if view::default_is_empty() {
        return Ok(Bindings::unsorted(collect_vars(patterns), vec![]));
    }
    // Triple-term patterns with variables (F14): the conjunctive-flattening path calls
    // this directly (bypassing eval_bgp), so the decomposition must happen here too.
    // The rewrite preserves pattern count/order, so `pat_filters` indexes stay aligned.
    let (rewritten, constraints) = extract_quoted_constraints(patterns);
    if !constraints.is_empty() {
        #[cfg(feature = "zk")]
        let _zk = crate::zk::op_scope(crate::zk::Op::QuotedTriples);
        let mut b = eval_bgp_binary(graph, &rewritten, pat_filters)?;
        for c in &constraints {
            b = join_bindings(b, quoted_relation(graph, c));
        }
        return Ok(b);
    }
    let pfilter = |i: usize| -> Option<(usize, ScanCmp)> { pat_filters.get(i).copied().flatten() };

    let prepared = prepare_bgp(graph, patterns)?;
    if prepared.iter().any(|p| p.unsatisfiable) {
        // zk-trace: an unsatisfiable constant (a term absent from the
        // dictionary) is a PROVABLY-EMPTY input set — the per-property proof
        // must witness "no such triple exists". Record ONLY the patterns that
        // are provably empty; a satisfiable SIBLING was never consumed (the
        // join short-circuits), so claiming it empty would over-state the
        // trace.
        #[cfg(feature = "zk")]
        if crate::zk::enabled() {
            for (tp, prep) in patterns.iter().zip(&prepared) {
                if prep.unsatisfiable {
                    crate::zk::record_empty_pattern(crate::zk::key_of_algebra_pattern(tp));
                }
            }
        }
        return Ok(Bindings::unsorted(collect_vars(patterns), vec![]));
    }

    // (sq-iywur) Optional DP join-order planner path. When the `dp-planner`
    // feature is compiled AND a planner is installed on this thread (`with_dp_planner`*),
    // enumerate connected-subgraph-complement pairs (DPccp) for a Cout-optimal BUSHY join
    // tree and evaluate that, instead of the greedy GOO order below. Falls back to greedy
    // (returns `None`) when no planner is installed, the BGP join graph is disconnected /
    // has an all-constant pattern (no single connected plan), or the connected-subgraph
    // count exceeds the budget. A BGP is a commutative/associative natural join, so the DP
    // tree yields the SAME rows as greedy (differentially tested); the default build
    // (feature off) is byte-identical — this whole block compiles away.
    #[cfg(feature = "dp-planner")]
    if let Some(cfg) = crate::dp::active() {
        if let Some(bindings) = eval_bgp_dp(graph, &prepared, pat_filters, cfg) {
            return Ok(bindings);
        }
    }

    // (sq-7d3dj.30.14) Membership-cluster pre-materialisation (opt-in
    // `cluster-materialize`). When the BGP has the SP2Bench-q07 shape — one
    // unbound-predicate container-membership pattern + a small bound-predicate anchor
    // sharing exactly one variable — evaluate that {anchor, membership} pair STANDALONE
    // (bounding the shared variable from the small anchor) and natural-join it to the
    // rest, instead of letting greedy GOO bind-join the wide membership relation per
    // driver binding. A BGP is a commutative/associative natural join, so partitioning
    // into two connected sub-BGPs and joining yields the SAME rows as any greedy order
    // (differentially tested, tests/cluster_materialize_differential.rs). `detect`
    // DECLINES (returns None → unchanged greedy plan) on any non-matching shape. The
    // whole block compiles away when the feature is off (default + wasm byte-identical).
    #[cfg(feature = "cluster-materialize")]
    if let Some(plan) = crate::cluster::detect(&prepared, crate::cluster::active_thresholds(), |i| pfilter(i).is_some()) {
        return eval_bgp_cluster(graph, patterns, pat_filters, &plan);
    }

    let var_pos = |i: usize, v: &Variable| -> Option<usize> { prepared[i].var_pos(v) };

    // Cost-based greedy (GOO): seed with the smallest single-pattern cardinality,
    // then repeatedly add the connected pattern that yields the smallest *estimated
    // join result*, using the per-predicate characteristic stats (distinct
    // subjects/objects) to estimate join selectivity. The join order only affects
    // performance (the result is identical for any order — differentially tested).
    // The decision logic lives in `goo_seed` / `goo_seed_sort` / `goo_pick` /
    // `record_pattern_ndv`, shared verbatim with the T22 EXPLAIN dry-run planner.
    let mut cs_ctx = CsCtx::new(&prepared);
    let seed = goo_seed(&prepared);
    let seed_sort_col = goo_seed_sort(&prepared, seed, pfilter(seed).map(|(c, _)| c));

    let mut result = scan_to_bindings(
        graph,
        &prepared[seed].id_pat,
        &prepared[seed].pos_vars,
        seed_sort_col,
        pfilter(seed),
        None,
        // The seed is the FIRST scan — there is no materialised side to build a
        // semi-join prefilter from yet.
        #[cfg(feature = "semijoin-bitmap")]
        None,
    );
    let mut done = vec![false; prepared.len()];
    done[seed] = true;
    cs_ctx.note_done(seed);

    // Running estimate of the result cardinality and the per-variable distinct
    // count (ndv), used to score the next join.
    let mut cur_card = prepared[seed].est as f64;
    let mut var_ndv: FxHashMap<Variable, f64> = FxHashMap::default();
    record_pattern_ndv(graph, &prepared, seed, cur_card, &mut var_ndv, &cs_ctx);

    for _ in 1..prepared.len() {
        // Pick the connected candidate with the smallest estimated output.
        let (i, new_card, _connected) = goo_pick(graph, &prepared, &done, &var_ndv, cur_card, &cs_ctx);
        cur_card = new_card;
        done[i] = true;
        cs_ctx.note_done(i);

        // Index-nested-loop (bind) join: when the running result is MUCH smaller than
        // the next pattern and exactly one variable connects them, look up each result
        // join value in the pattern's index (a bound scan) instead of scanning the whole
        // (large) relation and merge/hash-joining. This is the win on a selective join —
        // e.g. a chain whose far end is selective — where the merge would scan millions
        // of rows to match a few thousand. Same result, validated differentially.
        let connecting: Vec<Variable> = result.vars.iter().filter(|v| var_pos(i, v).is_some()).cloned().collect();
        if connecting.len() == 1
            && distinct_pattern_vars(&prepared[i].pos_vars)
            && result.rows.len().saturating_mul(8) < prepared[i].est
        {
            let jv = &connecting[0];
            let rk = result.col(jv).unwrap();
            let pp = var_pos(i, jv).unwrap();
            result = bind_join(graph, result, &prepared[i].id_pat, &prepared[i].pos_vars, rk, pp, pfilter(i));
            record_pattern_ndv(graph, &prepared, i, cur_card, &mut var_ndv, &cs_ctx);
            if result.rows.is_empty() {
                break;
            }
            continue;
        }

        // Execute: a pushed-down filter forces the scan into its own column order
        // (and filters inline); otherwise sort by the join variable for a merge.
        let filt = pfilter(i);
        let merge_var = result.sorted_by.clone().filter(|sv| var_pos(i, sv).is_some());
        let scan_sort = filt.map(|(c, _)| c).or_else(|| merge_var.as_ref().map(|jv| var_pos(i, jv).unwrap()));

        // (sq-gr8mb / §A3) Semi-join prefilter: build a membership filter over
        // ONE connecting variable's ids from the already-materialised `result`, and pass
        // it to the scan so rows that cannot survive the join are dropped before they are
        // built/joined. A row survives the downstream join only if it matches on EVERY
        // shared variable, so filtering on one of them removes only rows that would fail
        // anyway — the result is identical, just fewer rows scanned. Built only when a
        // single-column membership test is well-defined: a connecting variable that does
        // not repeat in the pattern (a repeat is handled by `build_row`'s consistency
        // check; the prefilter checks the variable's first canonical position).
        #[cfg(feature = "semijoin-bitmap")]
        let prefilter_keys = result
            .vars
            .iter()
            .find_map(|v| var_pos(i, v).map(|pos| (v.clone(), pos)))
            .and_then(|(v, pos)| {
                let rk = result.col(&v)?;
                crate::semijoin::KeyFilter::build(result.rows.iter().map(|r| r[rk])).map(|kf| (pos, kf))
            });
        #[cfg(feature = "semijoin-bitmap")]
        let prefilter = prefilter_keys.as_ref().map(|(pos, kf)| (*pos, kf));

        let rhs = scan_to_bindings(
            graph,
            &prepared[i].id_pat,
            &prepared[i].pos_vars,
            scan_sort,
            filt,
            None,
            #[cfg(feature = "semijoin-bitmap")]
            prefilter,
        );
        let connected = prepared[i].pos_vars.iter().flatten().any(|v| result.vars.contains(v));
        // Merge only when both sides are sorted on the join variable (a filter may
        // have forced the scan into a different order).
        if let Some(jv) = merge_var.filter(|jv| rhs.sorted_by.as_ref() == Some(jv)) {
            result = merge_join(result, rhs, &jv);
        } else if connected {
            result = hash_join(result, rhs);
        } else {
            result = cross_product(result, rhs);
        }
        record_pattern_ndv(graph, &prepared, i, cur_card, &mut var_ndv, &cs_ctx);

        if result.rows.is_empty() {
            break;
        }
    }
    Ok(result)
}
