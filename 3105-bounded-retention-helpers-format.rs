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
