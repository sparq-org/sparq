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
