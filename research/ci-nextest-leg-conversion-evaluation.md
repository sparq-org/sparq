# Converting the remaining plain `cargo test` CI legs to `nextest --profile ci` — evaluation

> 🤖 SPARQ agent [OPUS-5] — decision record for **sq-yhcge**, the follow-up to
> registry `jeswr/agent-account-registry#563` item 2 (PR `fable/oss-adopt-nextest-dequeued`,
> which moved ci.yml's sharded runs onto `.config/nextest.toml [profile.ci]`).
> EVALUATION ONLY: this record changes no workflow behaviour. It exists so the next
> agent to notice "these legs still use plain `cargo test`" reads the verdict instead
> of re-deriving it — or worse, converting them and re-opening the phantom-regression
> class that `sq-shacl-flake` / #1236 closed.

## Verdict

**Do not convert any of the remaining legs.** Every one of them falls into one of four
buckets, and in three of them conversion is a net negative rather than a cost/benefit
toss-up:

| Bucket | Legs | Verdict |
|---|---|---|
| A. Doctests | ci.yml:484, feature-matrix.yml:943 | **Never convertible** — nextest does not run doctests |
| B. Ratchet legs that pipe test stdout into a parsed `.out` | 19 legs in ci.yml | **Do not convert** — actively hazardous (§2) |
| C. Legs with a documented anti-nextest rationale | zk-toolchain.yml `forge-suite` (2), gui.yml `tauri-build` (6), shacl-diff-fuzz.yml `diff-fuzz` (1) | **Do not convert** — rationale still holds (§3) |
| D. Legs where conversion is merely not worth it | ci.yml:1556, ci.yml:2522, feature-matrix.yml (4 legs + group runner) | **Do not convert** — no flake history, install cost not repaid (§4) |

Counts verified against the tree at the time of writing: ci.yml holds **22** plain
`cargo test` invocations — 1 doctest, **19** piped into a parsed `.out`, and 2 unparsed.
`metamorph.yml` and `differential-update.yml` were listed on the bead but run a **built
binary**, not `cargo test`, so there is nothing to convert there at all.

The one thing the bead asked that is worth doing regardless of the verdict is recorded as
follow-up in §6: nothing here is blocked on it.

## 1. What conversion would actually cost

None of these jobs install `cargo-nextest` today — nextest is installed only in ci.yml's
`build-archive` job and in the shard jobs that consume its archive. So "convert a leg"
means, per job: add a `taiki-e/install-action` step, and accept a new network dependency
on the gating path. That was already ruled out of scope by the maintainer-approved
adoption in registry#563 item 2; this record is about whether the *follow-up* changes
that answer.

It also means these legs cannot share the `nextest.tar.zst` archive. The archive is built
`--features approx-ann,filtered-ann,vec-predicate`; the ratchet legs each need a different
feature overlay (`shacl-af`, `geosparql_rewrite`, `jsonld-suite`, `service`, `http-protocol`,
`federation-descriptors`, `service-loopback`, `d-entail`, `rif-wg-core`, `rif-core`,
`el-suite,el-suite-par`, `ql-experimental`, `dl-direct`) and two — ci.yml:1556 and
ci.yml:2910 — use `--profile release-fast`. Each converted leg
would build its own binaries and run its own `nextest run` — so conversion buys the retry
semantics and nothing else. There is no build-sharing upside to weigh against the cost.

## 2. Bucket B — the ratchet legs, where conversion is *hazardous*

This is the substantive finding, and it is the answer to the bead's second question
("does its ratchet-output parsing survive nextest's per-process output model?").

**Per-process isolation is fine.** Each ratchet suite prints its scoreboard from a *single*
`#[test]` fn — e.g. `crates/sparq-geo/tests/ogc_compliance_ratchet.rs:490`
`ogc_topology_compliance_ratchet` prints `pass {pass} / fail {fail} (floor {…})` at
`:497`; likewise `odrl_test_suite_decision_parity` and `bm25_differential_oracle`. Nothing
accumulates across test processes, so nextest running each test in its own process would
not split a scoreboard. The bead's stated concern is, on this axis, unfounded.

**Two other properties of the run model do break these legs.**

### 2.1 Capture: a naive translation silently produces an empty scoreboard

The legs run `cargo test … -- --nocapture | tee <lane>.out` and a later step greps that
file. `--nocapture` is a *libtest* flag; nextest does not accept it in that position and
by design captures test output, surfacing it for failures rather than for passes. So the
mechanical translation `cargo test … -- --nocapture` → `cargo nextest run …` yields a
`.out` with **no scoreboard line at all** — for a run in which every test *passed*.

That is precisely the failure the whole retry-wrapper apparatus exists to prevent. From
ci.yml's own comment on the geo leg (`:1786`):

> A transient crates.io download hiccup (`curl failed` mid-fetch) aborts `cargo test`
> BEFORE any test runs, leaving geo-conformance.out empty; a single-shot invocation then
> let the ratchet step below mis-report that empty scoreboard as a phantom "regressed
> below the ratchet".

The `sq-shacl-flake` / #1236 hardening (an explicit "scoreboard is missing/empty → this is
INFRA, not a regression" guard at `:1825`) means a converted leg would not *lie* — it would
fail with the honest infra diagnostic. But it would fail on every single run, permanently,
for every one of the 19 legs. Restoring the output means `--no-capture` (which forces
serial execution, discarding nextest's parallelism) or a `success-output` setting in
`.config/nextest.toml`. Both are reachable; neither is free, and both would have to be
validated against the exact `grep`/`awk` anchors below.

### 2.2 Retries interact *destructively* with the parsers

`[profile.ci]` sets `retries = 2`. The parsers are anchored and take the **first** match:

```sh
grep -oE 'pass [0-9]+ / fail' geo-conformance.out | grep -oE '[0-9]+' | head -1
grep -E '^TOTAL service ' service-eval-conformance.out | awk '{print $3}' | head -1
grep -E '^OWL 2 EL ratchet pass ' el-suite-conformance.out | awk '{print $6}' | head -1
```

A ratchet test asserts its own floor internally (`assert!(pass >= floor)` / `fail == 0` —
see the ci.yml:1786 comment). So under `--profile ci`, a genuinely flaky ratchet run would:
fail attempt 1 (emitting a scoreboard with a **lower** pass count), pass attempt 2 (emitting
the correct one), and be reported by nextest as **FLAKY, exit 0**. Both scoreboards land in
the same `tee`d file, and `head -1` picks the *failing* one. The grep gate then reds the job
with "regressed below the ratchet" — on the exact flake the retry was added to absorb, and
with a *false regression claim*, which is the specific outcome #1236 was fixed to eliminate.

Making the retry safe would require rewriting all 19 parsers from `head -1` to last-match
semantics — a change to gating floor-enforcement logic across every conformance lane, in
exchange for a retry these legs do not need (see §2.3). The bead asked whether the parsing
survives; it does not, and the repair is disproportionate.

Note the two `^`-anchored families (`^TOTAL …`, `^OWL 2 EL ratchet pass …`,
`^RIF-Core expressivity assertions …`, `^QL entailment graduated floor …`) carry an
additional silent-failure risk: if nextest's restored-output framing indents or prefixes
captured stdout, the anchor stops matching and the lane reports an empty scoreboard. Which
of nextest's output modes preserve a bare line start is **not verified in this record** —
the toolchain is not
installable on the box this was written on (read-only rustup root). Any future attempt must
verify it empirically first; see §6.

### 2.3 The flake class these legs actually see is one nextest cannot retry

Every retry comment in ci.yml attributes the observed flake to the same cause: a transient
crates.io fetch failure that aborts the run **before any test executes**. Nextest's retry is
*per-test*; if no test process ever starts, it never engages. The existing whole-step
2-attempt wrapper (present on the shacl, geo ×2, solid, odrl, text, rsp, jsonld and
inference legs) is the correct tool for that class and already covers it. Converting would
add a retry mechanism aimed at a flake class these legs have no recorded history of, while
breaking the gate that catches the class they do have.

## 3. Bucket C — legs whose existing rationale still holds

- **zk-toolchain.yml `forge-suite`** (`:338`, `:356`) — the file states it outright at
  `:344`: *"nextest is NOT used here because these are slow, serialized, toolchain-bound
  cases (the retry/process-isolation nextest buys us on the fast lane is not what this lane
  needs)."* Both legs pin `--test-threads=1` to avoid concurrent `bb` subprocess contention;
  nextest's per-test process model is the opposite of what the lane wants. Rationale intact —
  no change.
- **gui.yml `tauri-build`** (6 legs across a 5-platform matrix) — deterministic lane,
  `retries = 0` by intent, and the job is `continue-on-error: true` (advisory). A retry
  profile on an advisory lane changes nothing that anyone gates on, at the cost of installing
  nextest on five runners including Windows and two macOS images. Clear no.
- **shacl-diff-fuzz.yml `diff-fuzz`** (`:174`) — `-- --ignored --nocapture`, fail-closed
  differential fuzz on a nightly `schedule`/`workflow_dispatch` trigger, never on a PR head,
  so it never reaches the `ci-summary / gate` aggregator. Retrying a *fuzz disagreement* is
  the wrong semantics: a seed that disagrees once is a real finding, and a retry that passes
  would mask it. Actively undesirable, not merely unnecessary.

## 4. Bucket D — conversion possible, not worth it

- **ci.yml:1556** (`sparq-conformance` W3C, `--profile release-fast`, no output parsing) and
  **ci.yml:2522** (in-process SERVICE loopback smoke) are the only two ci.yml legs whose
  output nothing parses, so §2 does not bite. Neither has a recorded flake history, and both
  would need a nextest install added to a gating job. Revisit only if one of them starts
  flaking.
- **feature-matrix.yml** — the three `--no-default-features` legs (`:793`, `:864`, `:941`),
  the doctest leg (`:943`, bucket A), and the group runner
  (`scripts/run-feature-matrix-group.py:186`, which shells `cargo test -p <crate> --features
  <set>` per leg). Converting the group runner means changing a Python driver *and* installing
  nextest in the unprivileged group job — and that job sits inside the PR #3511 trust boundary
  (it runs token-less and writes results to an artifact for the default-branch-owned reporter).
  Adding a tool-install step to a deliberately-minimal unprivileged job is a supply-chain
  regression for no test-reliability gain. Strong no.

## 5. What would change this verdict

Convert a leg only when **all** of these hold, per leg:

1. It has a *recorded* flake history of the per-test kind (a test that fails and passes on
   re-run), not the fetch-transient kind the step wrapper already absorbs.
2. Its job already installs nextest, or the leg is hot enough to repay the install.
3. Either it does not parse test stdout, or §2.1/§2.2 have been resolved *first* — output
   restored under a verified nextest mode, and the parser moved to last-match semantics.

No leg in the tree currently satisfies (1).

## 6. Follow-up (not blocking, filed separately)

The `head -1` / first-match parsing in the 19 ratchet gates is fragile independently of
nextest: any future change that causes a scoreboard line to be emitted twice into the same
`.out` turns a pass into a false "regressed below the ratchet". Hardening those parsers to
last-match is a small, self-contained robustness fix worth doing on its own merits, and doing
it would also remove hazard §2.2 from any future conversion attempt.

## References

- `.config/nextest.toml` — `[profile.ci]` (`retries = 2`, `final-status-level = "flaky"`) and
  `[profile.default-miri]`.
- `.github/workflows/ci.yml` — the 22 plain `cargo test` legs; the `sq-shacl-flake` /
  #1236 empty-scoreboard guards; the per-leg 2-attempt transient self-heal wrappers.
- `.github/workflows/zk-toolchain.yml:344` — the one pre-existing in-file anti-nextest rationale.
- `.github/workflows/feature-matrix.yml:40-54` — the PR #3511 unprivileged-group-job trust boundary.
- registry `jeswr/agent-account-registry#563` item 2 — the maintainer-approved adoption whose
  scope this record closes out.
