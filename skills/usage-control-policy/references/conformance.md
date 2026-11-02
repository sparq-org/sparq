## Conformance — SolidLab ODRL Test Suite — sq-tmsd6

The evaluator is ratcheted against the MIT-licensed
[SolidLab ODRL Test Suite](https://github.com/SolidLabResearch/ODRL-Test-Suite)
(crate-local runner `crates/sparq-policy/tests/odrl_test_suite.rs`). Each of the 68
self-describing Turtle cases references — by UUID — a policy / request /
state-of-the-world / *expected compliance report*; the runner loads the policy
through the real `parse_policy_str`, derives a real `Request`, evaluates it through
the real `evaluate`, and asserts the decision matches the oracle the expected
report encodes:

| expected rule report | `report:activationState` | expected decision |
|---|---|---|
| `report:PermissionReport` | `report:Active` | ALLOW |
| `report:ProhibitionReport` | `report:Active` | DENY (prohibition fires) |
| either kind | `report:Inactive` | DENY |

At the pinned suite revision **67/68 pass** (floor `ODRL_SUITE_FLOOR`, may only
RISE; mirrored in the central conformance scoreboard). The constraint-matching batch
(sq-euhr3 / sq-k7itg / sq-a0zef) raised the floor from 59 to 67 by
implementing — through the real `evaluate` path — `odrl:LogicalConstraint`
(`odrl:and`/`odrl:or`/`odrl:xone`, incl. nesting) compound constraints,
`odrl:PartyCollection` / `odrl:AssetCollection` membership matching, and the
`odrl:use` action hierarchy (`use` subsumes its sub-actions but **not** the
ownership-`transfer` subtree — so a `use` permission is Active for `read`/`write` but
Inactive for `sell`). The **1** remaining case is a documented **not-implemented**
divergence — a duty whose discharge state is unknown (`report:NonSet`); sparq is
fail-closed (a duty must be explicitly discharged) where SolidLab treats a
not-violated duty as active. It does NOT count as a failure and does NOT gate. Fetch
with `bash scripts/fetch-odrl-suite.sh` (the data is never committed); the runner
self-skips cleanly when the fetched dir is absent. Run:
`cargo test -p sparq-policy --test odrl_test_suite -- --nocapture`.
