use envelope_core::envelope::{check_freshness, Credential};
// Public: request freshness requirement and exact inclusive whole-second bound.
// Creation evidence remains private and awaits the suite metadata opener.
fn main(required: pub bool, earliest_created: pub u64, c: Credential<4>) {
    check_freshness(c, required, earliest_created);
}
