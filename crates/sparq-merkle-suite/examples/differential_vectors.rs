//! Deterministic random lexical witnesses and engine-checked comparison results.
#[path = "../tests/support/cases.rs"]
mod cases;
fn main() {
    let rows: Vec<_> = cases::cases()
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "datatype": c.datatype, "datatype_b": c.datatype_b, "a": c.a.value(), "b": c.b.value(),
                "key_a": c.key_a, "key_b": c.key_b,
                "order": c.order.map(|o| if o.is_lt() { -1 } else if o.is_gt() { 1 } else { 0 }),
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
}
