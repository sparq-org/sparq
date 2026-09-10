fn cross_product(left: Bindings, right: Bindings) -> Bindings {
    cross_product_ref(&left, &right)
}

// [GPT-6 Astra] Borrow rows so the capped caller can reuse its RHS without cloning it.
fn cross_product_ref(left: &Bindings, right: &Bindings) -> Bindings {
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
