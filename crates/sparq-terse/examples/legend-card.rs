//! sq-bzign — prints the frozen `sparq-terse` keyword legend card verbatim, so the
//! Phase-5 A/B harness charges arms B and C the *real* in-context legend tokens (design §5.2
//! "charge each arm all its costs"; §1.6 "the token win is a caching property"). This is exactly
//! the artifact `legend_card()` produces — no copy that could drift from the frozen set. Default
//! build (no feature needed): the legend layer is lean.
fn main() {
    print!("{}", sparq_terse::legend_card());
}
