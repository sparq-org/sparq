fn try_capped(
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
fn in_scope_vars(p: &GraphPattern) -> Vec<Variable> {
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
