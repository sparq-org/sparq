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

## EARL and results table

```sh
bash scripts/fetch-odrl-suite.sh
bash crates/sparq-policy/earl-report.sh
```

The report script accepts an optional output directory (default `target/earl`).
It uses the fetched checkout without changing the pin, writes one outcome per
test-case IRI through the shared test-only EARL sink, and invokes
`scripts/earl-report.py` with an identity mapping for the upstream `urn:uuid:`
IRIs. Outputs are `sparq-odrl-earl.ttl`, `sparq-odrl-results.md`,
`odrl-outcomes.tsv`, `odrl-revisions.tsv` and `odrl.log`.

The table follows upstream's "Result of the test suite" columns. Its comparison
is the expected rule activation projected to an allow/deny decision, **not** full
compliance-report graph equality. Support means the runner can evaluate the case;
a supported decision mismatch has comparison `false` and support `true`.
Unsupported cases and harness errors are `failed` in EARL, even when the existing
ratchet permits a documented not-implemented bucket. The ratchets are unchanged.

For direct runner use, `SPARQ_EARL_OUT` (default unset) appends tab-separated
`IRI`/`passed|failed` lines; `SPARQ_ODRL_TABLE_OUT` (default unset) writes the
Markdown table. `ODRL_TESTS_DIR` defaults to `tests/odrl-test-suite/data` and
selects another suite data directory, including tiny hermetic fixtures used by
the reporting tests. Reporting finishes before ratchet assertions; an incomplete
run cannot produce a new final EARL report. The shell script preserves a complete
runner's nonzero exit status after writing its report.
