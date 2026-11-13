use envelope_core::envelope::{check_validity, Credential};
// Public: the request's exact whole-second offset instant in this fixture profile.
fn main(now: pub u64, c: Credential<4>) {
    check_validity(c, now);
}
