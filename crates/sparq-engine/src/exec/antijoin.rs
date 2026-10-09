use super::*;

// ---- Correlated (theta) anti-join (sq-7d3dj.30.9) --------------------------------
// See the `theta_antijoin` module comment near the top of this file for the
// shape, the SIP seeding, and the full soundness argument.

/// Which equality RELATION a correlation conjunct expresses. This is load-bearing for
/// soundness: value `=`, `sameTerm`, and id-equality are DIFFERENT relations on RDF
/// literals (a value-equal but id-distinct pair of literals exists), so the id-bucket
/// probe eligibility AND the per-candidate re-check expression differ by kind.
/// (sq-3cmr4)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CorrKind {
    /// SPARQL value equality `?outer = ?inner` (also the desugaring of a single-variable
    /// `?outer IN (?inner)`). Value-equality (sq-lr2ii): an id-bucket probe is exact ONLY
    /// when the correlation value is a NamedNode/BlankNode (where `=` coincides with term
    /// identity); a LITERAL key must take the value-correct scan with `=` re-checked.
    Eq,
    /// SPARQL `sameTerm(?outer, ?inner)` — TERM IDENTITY. Two terms are `sameTerm` iff
    /// identical (same IRI / same blank id / same lexical+datatype+language literal),
    /// which under dictionary interning is EXACTLY id-equality — for EVERY term kind,
    /// literals included. So an id-bucket probe is exact even for a literal key, and the
    /// per-candidate re-check (when a scan is taken) must be `sameTerm`, never `=`.
    SameTerm,
}

/// One correlation conjunct `?outer θ ?inner` extracted from the OPTIONAL condition,
/// where `?inner` is certain-in-`B` and `?outer` is a left variable and `θ` is the
/// relation named by `kind`. The remaining conjuncts (residual theta) are re-checked
/// verbatim per candidate right row.
pub(super) struct Correlation {
    /// Column of `?outer` in the LEFT bindings.
    pub(super) left_col: usize,
    /// The certain-in-`B` variable `?inner` seeded sideways into the right scan.
    pub(super) inner_var: Variable,
    /// The equality relation `θ` this conjunct expresses (`=` or `sameTerm`).
    pub(super) kind: CorrKind,
}

/// Attempts the correlated theta anti-join for `Filter(!bound(?nb), LeftJoin(A, B, F))`.
///
/// Returns `Ok(Some(result))` when the shape matched AND the path fired (result is
/// bag-equivalent to the cold `Filter{LeftJoin}` plan), or `Ok(None)` to DECLINE — in
/// which case the caller runs the identical prior plan.
pub(super) fn try_theta_antijoin(
    graph: &Graph,
    local: &mut LocalVocab,
    filter_expr: &Expression,
    inner: &GraphPattern,
) -> Result<Option<Bindings>, String> {
    if !theta_antijoin::enabled() {
        return Ok(None);
    }
    // Shape: outer filter is exactly `!bound(?nb)` over a `LeftJoin` WITH a condition.
    let Some(nb) = as_not_bound_var(filter_expr) else { return Ok(None) };
    let GraphPattern::LeftJoin { left, right, expression: Some(cond) } = inner else {
        return Ok(None);
    };

    // `?nb` must be a right-only anti-join witness: certain in `B` (so the LeftJoin
    // binds it on EVERY match, hence `!bound` selects exactly the non-matches) and
    // ABSENT from `A` (else `!bound(?nb)` is not the anti-join predicate — decline).
    let mut b_certain = FxHashSet::default();
    certain_vars(right, &mut b_certain);
    if !b_certain.contains(&nb) || pattern_has_var(left, &nb) {
        return Ok(None);
    }

    // (sq-7d3dj.30.20) Static early-decline: this path exists ONLY to SEED the
    // right scan sideways from a var-to-var correlation in the OPTIONAL condition. If no
    // conjunct of `cond` can be such a seedable correlation — decided PURELY STATICALLY from
    // the left/right variable SETS, no graph access — the path would evaluate the mandatory
    // left side and then decline (`correlations.is_empty()` below), forcing the cold
    // `Filter{LeftJoin}` fallback to RE-EVALUATE that same left side. SP2Bench q07 is exactly
    // this case (its OPTIONAL condition is a bare `!bound(?doc4)`), and its left side is the
    // dominant ~30ms outer BGP — so the redundant second evaluation is the query's residual.
    // Declining HERE, before the left side is touched, removes the wasted evaluation. This is a
    // pure evaluation-ORDERING change: it declines exactly the same shapes the
    // `correlations.is_empty()` check below would (that check is total over `cond` and does not
    // depend on any left ROW), and a decline always yields the identical cold-plan result, so it
    // is BAG-RESULT-EQUIVALENT to the feature-off build. OPT-IN; when off, byte-identical.
    #[cfg(feature = "antijoin-static-decline")]
    if !cond_has_seedable_correlation(cond, left, &b_certain) {
        theta_antijoin::record_early_decline();
        return Ok(None);
    }

    // Evaluate the mandatory left side once.
    let left_b = eval_graph_pattern(graph, local, left)?;

    // Output layout, computed STATICALLY (never evaluate `B` cold — that is exactly
    // what this path avoids): the LeftJoin's variables = left's vars, then each
    // in-scope right variable not already present. Surviving rows carry only left
    // values + `NO_ID` padding for the right-only columns.
    let right_scope = in_scope_vars(right);
    let mut out_vars = left_b.vars.clone();
    for v in &right_scope {
        if !out_vars.contains(v) {
            out_vars.push(v.clone());
        }
    }
    let n_right_only = out_vars.len() - left_b.vars.len();
    if left_b.rows.is_empty() {
        return Ok(Some(Bindings::unsorted(out_vars, Vec::new())));
    }

    // Partition the OPTIONAL condition into correlation conjuncts and a residual.
    // A correlation conjunct is a var-to-var equality `?outer θ ?inner` (either
    // orientation) with `?inner` certain in `B` and `?outer` a LEFT variable, where θ is
    // value `=`, `sameTerm`, OR a single-variable `?a IN (?b)` membership (which
    // desugars to `?a = ?b`). Everything else — a constant, a multi-element `IN`, a
    // non-var operand, an inequality — is residual theta, re-checked VERBATIM on the
    // merged row (always sound). A `?a IN (const, …)` list with no in/outer variable is
    // NOT a correlation (nothing to seed sideways). (sq-3cmr4)
    let conjuncts = split_and(cond);
    let mut correlations: Vec<Correlation> = Vec::new();
    let mut residual: Vec<&Expression> = Vec::new();
    for c in &conjuncts {
        if let Some((va, vb, kind)) = as_correlation_pair(c) {
            // Orient: the certain-in-B side is `?inner`, the left side is `?outer`.
            let (outer, inner_var) = if b_certain.contains(&vb) && left_b.col(&va).is_some() {
                (va.clone(), vb.clone())
            } else if b_certain.contains(&va) && left_b.col(&vb).is_some() {
                (vb.clone(), va.clone())
            } else {
                residual.push(c);
                continue;
            };
            // `?inner` must not also be a left variable (that would be a shared join
            // var handled by compatibility, not a seedable correlation) and the outer
            // must be resolvable to a column.
            if left_b.col(&inner_var).is_some() {
                residual.push(c);
                continue;
            }
            let Some(left_col) = left_b.col(&outer) else {
                residual.push(c);
                continue;
            };
            correlations.push(Correlation { left_col, inner_var, kind });
        } else {
            residual.push(c);
        }
    }
    // No correlation to seed sideways → the SIP win is absent; decline so the cold
    // plan runs (this path exists to seed the right scan, not to reimplement it).
    if correlations.is_empty() {
        return Ok(None);
    }

    // Distinct correlation tuples on the LEFT (dedup for evaluation planning only).
    let mut order: Vec<Vec<Id>> = Vec::new();
    let mut groups: FxHashMap<Vec<Id>, Vec<usize>> = FxHashMap::default();
    for (ri, row) in left_b.rows.iter().enumerate() {
        let key: Vec<Id> = correlations.iter().map(|c| row[c.left_col]).collect();
        match groups.entry(key.clone()) {
            std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().push(ri),
            std::collections::hash_map::Entry::Vacant(e) => {
                order.push(key);
                e.insert(vec![ri]);
            }
        }
    }

    // The residual theta, evaluated on the merged (A-row ⊕ B-row) tuple via full SPARQL
    // 3-valued semantics; a conjunct that is not effectively-true (false OR error OR
    // unbound) means this `b` does NOT match, so the left row is NOT eliminated by it.
    let residual_owned: Vec<Expression> = residual.into_iter().cloned().collect();

    // STRATEGY SELECTION.
    //  * Large correlation cardinality → HASH ANTI-JOIN: evaluate `B` ONCE cold,
    //    partition its rows by the `?inner` id tuple, then probe each left row's bucket
    //    for a theta match. One `B` scan, O(|A| + |B|) — the q06 win. A left row whose
    //    correlation values are all IRIs id-probes its bucket EXACTLY (IRI value-
    //    equality IS id-equality); a left row with a non-IRI (literal) correlation value
    //    can NOT be id-probed (the sq-lr2ii value-equality class), so it takes a
    //    VALUE-CORRECT full scan over `B` with the correlation `=` re-checked verbatim.
    //  * Small correlation cardinality (≤ SIP_MAX_SMALL_ROWS) → SIP-SEED: evaluate a
    //    tiny correlated `B'` per distinct correlation (few evals amortise well, and
    //    seeding restricts each scan).
    let use_hash = order.len() > SIP_MAX_SMALL_ROWS;

    let mut result_rows: Vec<Row> = Vec::with_capacity(left_b.rows.len());
    let mut total_child_rows = 0usize;

    if use_hash {
        // ---- Hash anti-join (evaluate B once, partition by ?inner id) ----
        let b_all = eval_graph_pattern(graph, local, right)?;
        total_child_rows += b_all.rows.len();
        // `?inner` id columns in `b_all` (certain in B ⇒ present and bound).
        let inner_cols: Option<Vec<usize>> =
            correlations.iter().map(|c| b_all.col(&c.inner_var)).collect();
        let Some(inner_cols) = inner_cols else {
            // A correlation `?inner` unexpectedly missing from B's header — decline.
            return Ok(None);
        };
        // Partition B rows by their ?inner id tuple (exact for IRI keys).
        let mut buckets: FxHashMap<Vec<Id>, Vec<usize>> = FxHashMap::default();
        for (bi, brow) in b_all.rows.iter().enumerate() {
            let key: Vec<Id> = inner_cols.iter().map(|&c| brow[c]).collect();
            buckets.entry(key).or_default().push(bi);
        }
        let all_b: Vec<usize> = (0..b_all.rows.len()).collect();
        let (out_src, tmp_vars) = merged_row_plan(&out_vars, &left_b, &b_all);
        let shared: Vec<(usize, usize)> = left_b
            .vars
            .iter()
            .enumerate()
            .filter_map(|(lc, v)| b_all.col(v).map(|bc| (lc, bc)))
            .collect();
        // The correlation re-checks, for the value-correct scan of a NON-id-probeable
        // left row (an id-probeable row's correlation is already exact via the id bucket).
        // Each conjunct uses ITS OWN relation: `=` for a value-equality correlation, but
        // `sameTerm` for a sameTerm correlation (they DIFFER on value-equal id-distinct
        // literals). (sq-3cmr4)
        let corr_checks: Vec<Expression> =
            correlations.iter().map(|c| corr_recheck_expr(c, &out_vars)).collect();
        let mut value_checks: Vec<Expression> = corr_checks.clone();
        value_checks.extend(residual_owned.iter().cloned());

        // Per-left-row elimination verdict: `true` iff SOME right row eliminates `lrow`
        // (so the row does NOT survive the anti-join). Every input it reads — `graph`,
        // `local` (immutable), the buckets, the checks — is shared read-only, so this is
        // safe to evaluate in parallel across left rows. `local` is `&LocalVocab` here
        // (never mutated inside the probe): a probe interns nothing, so no per-row id
        // divergence. (sq-3cmr4 / sq-f1emb)
        let eliminated = |lrow: &Row| -> Result<bool, String> {
            // Every correlation value id-bucket-probeable? Then an exact id-bucket probe
            // is sound (see `corr_id_probeable` for the per-kind argument): for a value
            // `=` correlation the value must be a NamedNode/BlankNode (where `=` coincides
            // with term identity — per SPARQL 1.1 §17.4.1.7 `=` on two DISTINCT non-literal
            // terms is FALSE, and FALSE/error both mean NO MATCH, which an id-bucket
            // non-match reproduces); a LITERAL under `=` is the sq-lr2ii value-equality
            // hazard → value-correct scan. For a `sameTerm` correlation EVERY kind is
            // id-probeable (id-equality IS sameTerm for all term kinds, literals included).
            // (sq-3cmr4)
            let id_hashable = correlations
                .iter()
                .all(|c| corr_id_probeable(c.kind, term_of(graph, local, lrow[c.left_col]).as_ref()));
            if id_hashable {
                let key: Vec<Id> = correlations.iter().map(|c| lrow[c.left_col]).collect();
                match buckets.get(&key) {
                    None => Ok(false),
                    Some(cands) => antijoin_row_matches(
                        graph, local, lrow, &b_all, cands, &shared, &out_src, &tmp_vars,
                        &residual_owned,
                    ),
                }
            } else {
                // Literal-keyed left row: value-correct full scan with the correlation
                // relation re-checked (`=` or `sameTerm` per kind).
                antijoin_row_matches(
                    graph, local, lrow, &b_all, &all_b, &shared, &out_src, &tmp_vars,
                    &value_checks,
                )
            }
        };

        // (sq-f1emb) For a VERY LARGE anchor side, the per-left-row probe is
        // embarrassingly parallel (each verdict reads only shared read-only state).
        // Mirror the parallel hash-join / BIND threshold (`PAR_THRESHOLD`): fan the
        // VERDICT computation out over cores, collecting one `bool` per row IN ORDER, then
        // build + budget-truncate the surviving output rows SERIALLY in the identical left
        // order. Because only the verdicts are parallelised (never the ordered output
        // build, never `local` interning), the result is byte-identical to the serial
        // probe — the order-preserving `collect` + serial truncation reproduce exactly the
        // serial loop's early-`break`-on-budget survivor prefix. The EXISTS residual re-
        // enters the thread-local function / view / spatial state, so each worker installs
        // the snapshot exactly like the FILTER / BIND parallel paths.
        //
        // (sq-qk6ac) An exhausted budget must block the VERDICT loop, not
        // just the output build: one verdict is a whole probe of `B` (a bucket scan plus
        // 3-valued expression evaluation — a WHOLE-`B` scan for a literal-keyed row), so a
        // timed-out / cancelled query used to grind every remaining left row only for the
        // build below to discard the lot on its FIRST `budget::exhausted` check. A rayon
        // worker cannot see the installing thread's sticky flag, so it re-checks a
        // captured `Limits` snapshot (the parallel hash-join / JSON-serialize pattern) and,
        // when hit, returns the CANONICAL budget error — `collect` into `Result`
        // short-circuits, so the queued probes are abandoned. Raising the error rather
        // than guessing a placeholder verdict also means a skipped row can never escape as
        // a silently truncated result, and it is behaviour-identical on the observable
        // path: today the build truncates to nothing and the caller's operator-exit
        // `budget::check` raises this same message, just after the wasted work.
        #[cfg(feature = "parallel")]
        if left_b.rows.len() >= PAR_THRESHOLD && !budget::evaluation_capacity_active() {
            use rayon::prelude::*;
            let limits = budget::snapshot();
            let fns = functions::snapshot();
            let vw = view::snapshot();
            let spx = spatial::snapshot();
            // (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
            // a worker that missed the registry would dial the IRI instead of answering it.
            #[cfg(feature = "service-local")]
            let lsv = local_services::snapshot();
            #[cfg(not(target_arch = "wasm32"))]
            let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
            let qb = query_base_snapshot(); // keep BASE visible to IRI()/URI() on workers
            let verdicts: Vec<bool> = left_b
                .rows
                .par_iter()
                .map(|lrow| {
                    // One non-tripping `Instant` read per row under a deadline budget, and
                    // a single `on` test when no budget is installed.
                    if let Some(why) = limits.why(0) {
                        return Err((Some(why), format!("query budget exceeded ({})", why)));
                    }
                    let _fns = functions::worker_install(&fns);
                    let _vw = view::worker_install(&vw);
                    let _spx = spatial::worker_install(&spx);
                    #[cfg(feature = "service-local")]
                    let _lsv = local_services::worker_install(&lsv);
                    #[cfg(not(target_arch = "wasm32"))]
                    let _qn = query_now::worker_install(qn);
                    let _qb = query_base_worker_install(&qb);
                    eliminated(lrow).map_err(|message| (None, message))
                })
                .collect::<Result<Vec<bool>, (Option<crate::BudgetExceeded>, String)>>()
                .map_err(|(cause, message)| {
                    if let Some(cause) = cause { budget::record_worker_failure(cause); }
                    message
                })?;

            // Serial ordered build + budget truncation: identical to the serial probe
            // loop's `if !matched { push } ; break on budget` — the survivor prefix and
            // its order are reproduced exactly whether the verdicts were computed
            // serially or in parallel.
            for (lrow, &elim) in left_b.rows.iter().zip(&verdicts) {
                if budget::exhausted(result_rows.len()) {
                    break;
                }
                if !elim {
                    let mut combined: Row = lrow.clone();
                    combined.extend(std::iter::repeat_n(NO_ID, n_right_only));
                    result_rows.push(combined);
                }
            }
            theta_antijoin::record(total_child_rows, order.len());
            return Ok(Some(Bindings::unsorted(out_vars, result_rows)));
        }

        // Serial probe loop: the verdict is computed INSIDE the cooperative build loop, so
        // an exhausted budget stops probing at exactly the row it stops emitting — the
        // loop the parallel branch above is documented to reproduce.
        // (sq-qk6ac)
        for lrow in &left_b.rows {
            if budget::exhausted(result_rows.len()) {
                break;
            }
            if !eliminated(lrow)? {
                let mut combined: Row = lrow.clone();
                combined.extend(std::iter::repeat_n(NO_ID, n_right_only));
                result_rows.push(combined);
            }
        }
        theta_antijoin::record(total_child_rows, order.len());
        return Ok(Some(Bindings::unsorted(out_vars, result_rows)));
    }

    // ---- SIP-seed anti-join (small correlation cardinality / literal keys) ----
    let mut fired = false;
    'groups: for key in &order {
        let members = &groups[key];
        let ri0 = members[0];

        // Seed iff every correlation value in this group is an IRI (`=` IS term
        // identity for IRIs). A literal value routes to the cold `B` + value `=` recheck.
        let mut smap: FxHashMap<Variable, oxrdf::NamedNode> = FxHashMap::default();
        let mut all_iri = true;
        for c in &correlations {
            match term_of(graph, local, left_b.rows[ri0][c.left_col]) {
                Some(Term::NamedNode(n)) => {
                    smap.insert(c.inner_var.clone(), n);
                }
                _ => {
                    all_iri = false;
                    break;
                }
            }
        }
        let (b_prime, seeded) = if all_iri {
            match subst_pattern(right, &smap) {
                Some(subst) => (eval_graph_pattern(graph, local, &subst)?, true),
                None => (eval_graph_pattern(graph, local, right)?, false),
            }
        } else {
            (eval_graph_pattern(graph, local, right)?, false)
        };
        total_child_rows += b_prime.rows.len();
        fired = true;

        let (mut out_src, tmp_vars) = merged_row_plan(&out_vars, &left_b, &b_prime);
        // When seeded, `?inner` was folded to a constant inside `B`, so `b_prime` no
        // longer binds it — re-materialise its id so a residual referencing `?inner`
        // still sees the seeded value on the merged row.
        if seeded {
            for c in &correlations {
                if let Some(nn) = smap.get(&c.inner_var) {
                    if let Some(oi) = out_vars.iter().position(|v| v == &c.inner_var) {
                        if matches!(out_src[oi], MergeSrc::Unbound) {
                            let t = Term::NamedNode(nn.clone());
                            let id = graph.id_of(&t).unwrap_or_else(|| local.intern(t));
                            out_src[oi] = MergeSrc::Const(id);
                        }
                    }
                }
            }
        }
        let shared: Vec<(usize, usize)> = left_b
            .vars
            .iter()
            .enumerate()
            .filter_map(|(lc, v)| b_prime.col(v).map(|bc| (lc, bc)))
            .collect();
        // When NOT seeded, the correlation equalities were not enforced by substitution
        // — prepend them to the theta so they are re-checked with their FULL relation
        // (`=` for a value-equality correlation, `sameTerm` for a sameTerm correlation:
        // they differ on value-equal id-distinct literals). (sq-3cmr4)
        let mut checks: Vec<Expression> = Vec::new();
        if !seeded {
            for c in &correlations {
                checks.push(corr_recheck_expr(c, &out_vars));
            }
        }
        checks.extend(residual_owned.iter().cloned());

        let all_cands: Vec<usize> = (0..b_prime.rows.len()).collect();
        for &ri in members {
            let lrow = &left_b.rows[ri];
            // Stop the whole SIP strategy, not just this correlation group, so it ends
            // exactly like the hash strategy: no further seeded `B'` is evaluated and the
            // caller's operator-exit check raises the budget error (#4158).
            if budget::exhausted(result_rows.len()) {
                break 'groups;
            }
            let matched = antijoin_row_matches(
                graph, local, lrow, &b_prime, &all_cands, &shared, &out_src, &tmp_vars, &checks,
            )?;
            if !matched {
                let mut combined: Row = lrow.clone();
                combined.extend(std::iter::repeat_n(NO_ID, n_right_only));
                result_rows.push(combined);
            }
        }
    }

    if fired {
        theta_antijoin::record(total_child_rows, order.len());
    }
    Ok(Some(Bindings::unsorted(out_vars, result_rows)))
}

/// How to fill each `out_vars` slot in a merged (left ⊕ right) candidate row.
pub(super) enum MergeSrc {
    Left(usize),
    Right(usize),
    /// A SHARED column: take the left cell, but fall through to the right cell when the
    /// left is `NO_ID`. This mirrors `merge_rows` (sparq-substrate `join.rs`) EXACTLY,
    /// so the anti-join's condition-evaluation row matches the cold `left_outer_join`'s
    /// even when `A` binds the shared var only partially (e.g. an OPTIONAL/UNION inside
    /// `A`). Without the fall-through a left-unbound shared cell stayed `NO_ID`, the
    /// residual saw UNBOUND → error → no match, and the left row spuriously survived
    /// (the reviewer's counterexample).
    LeftThenRight(usize, usize),
    /// A seeded `?inner` constant (its id) that the right side no longer binds.
    Const(Id),
    Unbound,
}

/// Builds the per-`(left, right)` merge plan for the static `out_vars` layout: how to
/// source each output column, plus the `tmp_vars` header for `eval_expr`.
pub(super) fn merged_row_plan(
    out_vars: &[Variable],
    left: &Bindings,
    right: &Bindings,
) -> (Vec<MergeSrc>, Vec<Variable>) {
    let out_src = out_vars
        .iter()
        .map(|v| match (left.col(v), right.col(v)) {
            // Shared column: mirror `merge_rows` — left, falling through to right when
            // the left cell is unbound.
            (Some(lc), Some(bc)) => MergeSrc::LeftThenRight(lc, bc),
            (Some(lc), None) => MergeSrc::Left(lc),
            (None, Some(bc)) => MergeSrc::Right(bc),
            (None, None) => MergeSrc::Unbound,
        })
        .collect();
    (out_src, out_vars.to_vec())
}

/// `true` iff SOME candidate right row (`cands` = indices into `b.rows`) is compatible
/// with `lrow` on the shared columns AND makes every `checks` conjunct effectively-true
/// (correlation `=` re-checks — when present — and the residual theta), evaluated on the
/// merged row in `out_vars` order. This is the anti-join elimination test: a `true`
/// result DROPS the left row. Multiplicity: the caller emits each surviving row once.
#[allow(clippy::too_many_arguments)]
pub(super) fn antijoin_row_matches(
    graph: &Graph,
    local: &LocalVocab,
    lrow: &[Id],
    b: &Bindings,
    cands: &[usize],
    shared: &[(usize, usize)],
    out_src: &[MergeSrc],
    tmp_vars: &[Variable],
    checks: &[Expression],
) -> Result<bool, String> {
    let tmp = Bindings { vars: tmp_vars.to_vec(), rows: vec![], sorted_by: None };
    for &bi in cands {
        let rrow = &b.rows[bi];
        if !compatible(lrow, rrow, shared) {
            continue;
        }
        let combined: Row = out_src
            .iter()
            .map(|s| match s {
                MergeSrc::Left(lc) => lrow[*lc],
                MergeSrc::Right(bc) => rrow[*bc],
                MergeSrc::LeftThenRight(lc, bc) => {
                    if lrow[*lc] == NO_ID {
                        rrow[*bc]
                    } else {
                        lrow[*lc]
                    }
                }
                MergeSrc::Const(id) => *id,
                MergeSrc::Unbound => NO_ID,
            })
            .collect();
        let mut ok = true;
        for e in checks {
            if !effective_boolean(&eval_expr(graph, local, &tmp, &combined, e)?, local.ebv_semantics) {
                ok = false;
                break;
            }
        }
        if ok {
            return Ok(true); // one match eliminates the left row.
        }
    }
    Ok(false)
}

/// Does `var` occur ANYWHERE in `p` (any triple position, any expression including
/// `EXISTS` sub-patterns)? A SOUND OVER-approximation for the anti-join scope guard:
/// reporting a spurious occurrence only makes the rewrite DECLINE (safe); missing a
/// real one would be unsound, so every node kind recurses fully.
pub(super) fn pattern_has_var(p: &GraphPattern, var: &Variable) -> bool {
    use GraphPattern as G;
    let te = |t: &TermPattern| term_pattern_has_var(t, var);
    let ne = |n: &NamedNodePattern| matches!(n, NamedNodePattern::Variable(v) if v == var);
    match p {
        G::Bgp { patterns } => patterns
            .iter()
            .any(|tp| te(&tp.subject) || ne(&tp.predicate) || te(&tp.object)),
        G::Path { subject, object, .. } => te(subject) || te(object),
        G::Join { left, right } | G::Union { left, right } | G::Minus { left, right } | G::Lateral { left, right } => {
            pattern_has_var(left, var) || pattern_has_var(right, var)
        }
        G::LeftJoin { left, right, expression } => {
            pattern_has_var(left, var)
                || pattern_has_var(right, var)
                || expression.as_ref().is_some_and(|e| expr_has_var(e, var))
        }
        G::Filter { expr, inner } => expr_has_var(expr, var) || pattern_has_var(inner, var),
        G::Graph { name, inner } => ne(name) || pattern_has_var(inner, var),
        G::Extend { inner, variable, expression } => {
            variable == var || expr_has_var(expression, var) || pattern_has_var(inner, var)
        }
        G::Values { variables, .. } => variables.iter().any(|v| v == var),
        G::OrderBy { inner, expression } => {
            pattern_has_var(inner, var)
                || expression.iter().any(|o| match o {
                    OrderExpression::Asc(e) | OrderExpression::Desc(e) => expr_has_var(e, var),
                })
        }
        G::Project { inner, variables } | G::Group { inner, variables, .. } => {
            variables.iter().any(|v| v == var) || pattern_has_var(inner, var)
        }
        G::Distinct { inner } | G::Reduced { inner } | G::Slice { inner, .. } => {
            pattern_has_var(inner, var)
        }
        G::Service { name, inner, .. } => ne(name) || pattern_has_var(inner, var),
    }
}

pub(super) fn term_pattern_has_var(t: &TermPattern, var: &Variable) -> bool {
    match t {
        TermPattern::Variable(v) => v == var,
        TermPattern::Triple(inner) => {
            term_pattern_has_var(&inner.subject, var)
                || matches!(&inner.predicate, NamedNodePattern::Variable(v) if v == var)
                || term_pattern_has_var(&inner.object, var)
        }
        _ => false,
    }
}

/// Does `var` occur anywhere in expression `e` (including `EXISTS` sub-patterns)?
/// Over-approximation for the scope guard (spurious `true` only declines).
pub(super) fn expr_has_var(e: &Expression, var: &Variable) -> bool {
    use Expression as E;
    match e {
        E::Variable(v) | E::Bound(v) => v == var,
        E::NamedNode(_) | E::Literal(_) => false,
        E::Or(a, b)
        | E::And(a, b)
        | E::Equal(a, b)
        | E::SameTerm(a, b)
        | E::Greater(a, b)
        | E::GreaterOrEqual(a, b)
        | E::Less(a, b)
        | E::LessOrEqual(a, b)
        | E::Add(a, b)
        | E::Subtract(a, b)
        | E::Multiply(a, b)
        | E::Divide(a, b) => expr_has_var(a, var) || expr_has_var(b, var),
        E::In(a, list) => expr_has_var(a, var) || list.iter().any(|x| expr_has_var(x, var)),
        E::UnaryPlus(i) | E::UnaryMinus(i) | E::Not(i) => expr_has_var(i, var),
        E::If(a, b, c) => expr_has_var(a, var) || expr_has_var(b, var) || expr_has_var(c, var),
        E::Coalesce(list) | E::FunctionCall(_, list) => list.iter().any(|x| expr_has_var(x, var)),
        E::Exists(p) => pattern_has_var(p, var),
    }
}

/// `Some(v)` iff `e` is exactly `!BOUND(?v)` (`Not(Bound(v))`) — the anti-join
/// filter predicate.
pub(super) fn as_not_bound_var(e: &Expression) -> Option<Variable> {
    match e {
        Expression::Not(inner) => match inner.as_ref() {
            Expression::Bound(v) => Some(v.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// (sq-7d3dj.30.20) STATIC over-approximation of "does the OPTIONAL condition
/// `cond` contain at least one conjunct the theta anti-join could seed sideways?", decided
/// from the left-side variable SET and the certain-in-`B` set — WITHOUT evaluating either
/// side (no graph access). Used by [`try_theta_antijoin`]'s early-decline gate (opt-in
/// `antijoin-static-decline`) so it never pays to evaluate the mandatory left side only to
/// discard it on the `correlations.is_empty()` decline.
///
/// A conjunct is a SEEDABLE correlation exactly when the runtime partition (in
/// `try_theta_antijoin`) would place it in `correlations` rather than `residual`: it is a
/// var-to-var pair (`?a = ?b` / `sameTerm(?a, ?b)` / `?a IN (?b)`) with ONE side certain in
/// `B` and the OTHER a LEFT variable, and the certain-in-`B` side (the `?inner`) NOT also a
/// left variable. This mirrors that logic with `left_vars` standing in for the runtime
/// `left_b.col(v).is_some()` test — SOUND because for the conjunctive/BGP left sides this path
/// runs over, `left_b.vars` equals `left`'s in-scope variables. The over-approximation
/// direction is SAFE regardless: if this returns `true` when no correlation actually seeds, the
/// gate simply does NOT decline early and the unchanged runtime partition declines instead
/// (same result, one extra left evaluation — i.e. exactly the pre-feature behaviour). It only
/// ever declines early when the runtime would ALSO have found `correlations.is_empty()`.
#[cfg(feature = "antijoin-static-decline")]
pub(super) fn cond_has_seedable_correlation(
    cond: &Expression,
    left: &GraphPattern,
    b_certain: &FxHashSet<Variable>,
) -> bool {
    let left_vars: FxHashSet<Variable> = in_scope_vars(left).into_iter().collect();
    split_and(cond).into_iter().any(|c| {
        let Some((va, vb, _)) = as_correlation_pair(c) else { return false };
        // Same orientation choice as the runtime partition: pick the certain-in-B side as
        // `inner` and the left side as `outer`; a seedable correlation additionally requires
        // the chosen `inner` NOT to be a left variable.
        if b_certain.contains(&vb) && left_vars.contains(&va) {
            !left_vars.contains(&vb)
        } else if b_certain.contains(&va) && left_vars.contains(&vb) {
            !left_vars.contains(&va)
        } else {
            false
        }
    })
}

/// `Some((a, b, kind))` iff `e` is a VAR-TO-VAR correlation conjunct the theta anti-join
/// can seed sideways: `?a = ?b` (`CorrKind::Eq`), `sameTerm(?a, ?b)` (`CorrKind::SameTerm`),
/// or a SINGLE-variable membership `?a IN (?b)` (which desugars to `?a = ?b`, so `Eq`).
/// Returns `None` for a constant operand, an inequality, or a multi-element / constant
/// `IN` list — those stay in the residual (always sound). Orientation (which is inner /
/// outer) is decided by the caller from the certain-in-B / left-column sets.
/// (sq-3cmr4)
pub(super) fn as_correlation_pair(e: &Expression) -> Option<(Variable, Variable, CorrKind)> {
    match e {
        Expression::Equal(a, b) => match (a.as_ref(), b.as_ref()) {
            (Expression::Variable(va), Expression::Variable(vb)) => {
                Some((va.clone(), vb.clone(), CorrKind::Eq))
            }
            _ => None,
        },
        Expression::SameTerm(a, b) => match (a.as_ref(), b.as_ref()) {
            (Expression::Variable(va), Expression::Variable(vb)) => {
                Some((va.clone(), vb.clone(), CorrKind::SameTerm))
            }
            _ => None,
        },
        // `?a IN (?b)` is exactly `?a = ?b` (SPARQL 1.1 §17.4.1.9): a single-variable
        // membership. A list with a constant element, more than one element, or a
        // non-variable target is NOT this seedable shape — decline (residual, sound).
        Expression::In(target, list) => match (target.as_ref(), list.as_slice()) {
            (Expression::Variable(va), [Expression::Variable(vb)]) => {
                Some((va.clone(), vb.clone(), CorrKind::Eq))
            }
            _ => None,
        },
        _ => None,
    }
}

/// The per-candidate re-check expression for a correlation, in `out_vars` order: `=`
/// for a value-equality correlation, `sameTerm` for a sameTerm correlation. These are
/// DIFFERENT relations on value-equal id-distinct literals, so the kind is load-bearing
/// for soundness whenever a value-correct scan is taken (a non-id-probeable key, or the
/// un-seeded SIP path). (sq-3cmr4)
pub(super) fn corr_recheck_expr(c: &Correlation, out_vars: &[Variable]) -> Expression {
    let lhs = Box::new(Expression::Variable(out_vars[c.left_col].clone()));
    let rhs = Box::new(Expression::Variable(c.inner_var.clone()));
    match c.kind {
        CorrKind::Eq => Expression::Equal(lhs, rhs),
        CorrKind::SameTerm => Expression::SameTerm(lhs, rhs),
    }
}

/// Is a left-row correlation value of the given `term` id-bucket-probeable under `kind`?
///
/// * `SameTerm` — YES for every term kind (a dictionary id equality IS `sameTerm`: two
///   terms share an id iff identical). An `Unbound` (`None`) key is NOT probeable — an
///   unbound operand makes `sameTerm` error, so route it through the scan where the
///   3-valued residual decides (defensive: a correlation column is certain-in-A, so this
///   is unreachable in practice).
/// * `Eq` — YES only for a NamedNode/BlankNode (`=` coincides with term identity there);
///   a literal is the sq-lr2ii value-equality hazard and an unbound is unprobeable.
///
/// (sq-3cmr4)
pub(super) fn corr_id_probeable(kind: CorrKind, term: Option<&Term>) -> bool {
    match kind {
        CorrKind::SameTerm => term.is_some(),
        CorrKind::Eq => matches!(term, Some(Term::NamedNode(_) | Term::BlankNode(_))),
    }
}

/// Flattens a top-level `&&` (`Expression::And`) tree into its conjuncts, left to
/// right (single-element for a non-`And`).
pub(super) fn split_and(e: &Expression) -> Vec<&Expression> {
    fn go<'a>(e: &'a Expression, out: &mut Vec<&'a Expression>) {
        match e {
            Expression::And(a, b) => {
                go(a, out);
                go(b, out);
            }
            other => out.push(other),
        }
    }
    let mut v = Vec::new();
    go(e, &mut v);
    v
}

pub(super) fn union_bindings(left: Bindings, right: Bindings) -> Bindings {
    let mut out_vars = left.vars.clone();
    for v in &right.vars {
        if !out_vars.contains(v) {
            out_vars.push(v.clone());
        }
    }
    let mut rows: Vec<Row> = Vec::with_capacity(left.rows.len() + right.rows.len());
    let map_row = |src_vars: &[Variable], row: &[Id], out_vars: &[Variable]| -> Row {
        out_vars
            .iter()
            .map(|v| src_vars.iter().position(|x| x == v).map(|i| row[i]).unwrap_or(NO_ID))
            .collect()
    };
    for row in &left.rows {
        rows.push(map_row(&left.vars, row, &out_vars));
    }
    for row in &right.rows {
        rows.push(map_row(&right.vars, row, &out_vars));
    }
    Bindings::unsorted(out_vars, rows)
}

pub(super) fn minus_bindings(left: Bindings, right: Bindings) -> Bindings {
    // SPARQL MINUS: drop a left row iff some right row is *compatible* with it AND
    // their bound domains overlap on at least one shared variable. (Disjoint
    // domains never remove anything.)
    let shared: Vec<(usize, usize)> = left
        .vars
        .iter()
        .enumerate()
        .filter_map(|(li, v)| right.col(v).map(|ri| (li, ri)))
        .collect();
    if shared.is_empty() {
        return left; // disjoint domains -> MINUS removes nothing
    }
    let lcols: Vec<usize> = shared.iter().map(|&(lc, _)| lc).collect();
    let rcols: Vec<usize> = shared.iter().map(|&(_, rc)| rc).collect();

    // Fast path: shared columns fully bound -> compatibility is exact-key equality
    // and the domains always overlap, so membership in a hash set suffices.
    if !any_unbound(&left.rows, &lcols) && !any_unbound(&right.rows, &rcols) {
        let mut table: FxHashMap<Key, ()> = FxHashMap::default();
        for row in &right.rows {
            table.insert(rcols.iter().map(|&c| row[c]).collect(), ());
        }
        let rows: Vec<Row> = left
            .rows
            .into_iter()
            .filter(|row| !table.contains_key(&lcols.iter().map(|&c| row[c]).collect::<Key>()))
            .collect();
        return Bindings { vars: left.vars, rows, sorted_by: left.sorted_by };
    }

    // General path: per-row compatibility with a bound-domain-overlap check.
    let keep = |lrow: &Row| -> bool {
        !right.rows.iter().any(|rrow| {
            let mut overlap = false;
            for &(lc, rc) in &shared {
                let (a, b) = (lrow[lc], rrow[rc]);
                if a != NO_ID && b != NO_ID {
                    if a != b {
                        return false; // incompatible
                    }
                    overlap = true;
                }
            }
            overlap
        })
    };
    let rows: Vec<Row> = left.rows.iter().filter(|r| keep(r)).cloned().collect();
    Bindings { vars: left.vars, rows, sorted_by: left.sorted_by }
}

pub(super) fn values_bindings(graph: &Graph, local: &mut LocalVocab, variables: &[Variable], bindings: &[Vec<Option<GroundTerm>>]) -> Bindings {
    let rows = bindings
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    None => NO_ID,
                    Some(gt) => {
                        // Resolve against the graph dictionary so the id joins with
                        // BGP results; a term absent from the graph gets a local id
                        // (it can only match other VALUES, never the data).
                        let t = ground_to_term(gt);
                        graph.id_of(&t).unwrap_or_else(|| local.intern(t))
                    }
                })
                .collect()
        })
        .collect();
    Bindings::unsorted(variables.to_vec(), rows)
}

pub(super) fn extend_bindings(graph: &Graph, local: &mut LocalVocab, mut b: Bindings, var: &Variable, expr: &Expression) -> Result<Bindings, String> {
    // Pre-resolve Variable → column index once before the row loop. sq-7d3dj.4.
    let compiled = compile_expr(expr, &b, local);
    // BIND was fully serial because each row's computed value was interned immediately. Split it
    // (T1.0b): a PARALLEL pass evaluates the expression (read-only) and resolves the value to an
    // id read-only (inline / graph-dict / already-local); only genuinely new terms fall through to
    // the serial intern below, applied in row order → ids byte-identical to the serial path.
    // (Safe to parallelise: rows reference only ids created by EARLIER operators, never ids
    // created within this BIND loop.)
    // Row identity for BNODE(str)'s per-solution scoping (see ROW_SCOPE).
    let scope = b.rows.as_ptr() as usize;
    #[cfg(feature = "parallel")]
    let resolved: Vec<Result<Id, Term>> = if b.rows.len() >= PAR_THRESHOLD && !budget::evaluation_capacity_active() {
        use rayon::prelude::*;
        let lv: &LocalVocab = local;
        let bref = &b;
        // Thread-local extension-function registry and dataset view: snapshot +
        // per-item re-install (free when neither is installed) — see the FILTER
        // branch (the view matters here via EXISTS in the expression).
        let fns = functions::snapshot();
        let vw = view::snapshot();
        let spx = spatial::snapshot(); // sq-mg9: keep the spatial index visible under EXISTS re-entry.
        // (sq-lsp7k.2.2) Keep LOCAL SERVICE handlers visible on the worker too:
        // a worker that missed the registry would dial the IRI instead of answering it.
        #[cfg(feature = "service-local")]
        let lsv = local_services::snapshot();
        #[cfg(not(target_arch = "wasm32"))]
        let qn = query_now::snapshot(); // sq-98w7z.1: keep NOW() pinned on workers
        let qb = query_base_snapshot(); // keep BASE visible to IRI()/URI() on workers
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
                let _qb = query_base_worker_install(&qb);
                ROW_SCOPE.set((scope, i));
                let v = eval_compiled(graph, lv, bref, row, &compiled)?;
                Ok(value_to_id_readonly(graph, lv, &v))
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        let mut out = Vec::with_capacity(b.rows.len());
        for (i, row) in b.rows.iter().enumerate() {
            ROW_SCOPE.set((scope, i));
            let v = eval_compiled(graph, local, &b, row, &compiled)?;
            out.push(value_to_id_readonly(graph, local, &v));
        }
        out
    };
    #[cfg(not(feature = "parallel"))]
    let resolved: Vec<Result<Id, Term>> = {
        let mut out = Vec::with_capacity(b.rows.len());
        for (i, row) in b.rows.iter().enumerate() {
            ROW_SCOPE.set((scope, i));
            let v = eval_compiled(graph, local, &b, row, &compiled)?;
            out.push(value_to_id_readonly(graph, local, &v));
        }
        out
    };
    let col: Vec<Id> = resolved
        .into_iter()
        .map(|r| match r {
            Ok(id) => id,
            Err(term) => local.intern(term),
        })
        .collect();
    b.vars.push(var.clone());
    for (row, id) in b.rows.iter_mut().zip(col) {
        row.push(id);
    }
    b.sorted_by = None;
    Ok(b)
}
