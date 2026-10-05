<!-- [OPUS-5] sq-mi1b8 (#3151): completion audit of beads sq-t9d (Phase-0) and sq-t9v
     (streams A-H) against the EXTERNALIZED noir_XPath face repo. Read-only verification
     record. Structural evidence only -- `nargo` is absent on the authoring box, so no
     circuit was executed here; the executing evidence is the face repo's own CI. This
     record makes NO soundness or privacy claim (sq-qhy4 remains OPEN). -->

# noir_XPath: completion audit of Phase-0 (`sq-t9d`) and streams A–H (`sq-t9v`)

> 🤖 **SPARQ agent** — read-only verification record for `sq-mi1b8` (#3151). Nothing was
> modified in either repository. This is a **completeness** audit of two beads, not a
> correctness or soundness audit. The ZK estate remains research-grade and **NOT externally
> audited** (`sq-qhy4`).

## 0. Why this audit had to leave the repo

Both beads were filed from markdown TODOs inside the then-**in-tree** `zk/xpath/` Noir tree
(`zk/xpath/IMPLEMENTATION_PLAN.md:11` → `sq-t9d`; `zk/xpath/PARALLEL_WORK_ITEMS.md:76` →
`sq-t9v`). Those plan docs were deleted by the hygiene sweep `sq-u3u15` (#1505), and the
whole tree was then **externalized** to the [`sparq-org/noir_XPath`](https://github.com/sparq-org/noir_XPath)
face repo and removed from sparq by `sq-5reoy` (#1602). What survives in sparq under
`zk/xpath/` is only the **oracle half** of the M1 differential harness
([`zk/xpath/differential`](../zk/xpath/differential)) — six files, no library source, no
`test_packages/`.

**Consequence for any future agent:** a bead that cites a `zk/xpath/**` path other than
`differential/`, `scripts/`, or `tests/differential_oracle/` predates #1602 and tracks work
that now lives in the face repo. It cannot be discharged by editing sparq. (`sq-5vv7` is
another bead in this class — it cites `zk/xpath/test_packages/.../chunk_18.nr`.)

## 1. Method and its limits

- Evidence base: a read-only shallow clone of the pinned release **`v0.3.0`**, commit
  `0aa74d01f6e17e717c67ef370122ce2f257ce81d` — the same tag the sparq differential lane
  pins as `XPATH_TAG` (`.github/workflows/xpath-differential.yml`,
  `zk/xpath/scripts/run_differential_harness.sh`), bumped off `v0.2.0` by #5456.
- Checks performed: presence of each stream's *Files to Create*; presence of each
  acceptance-criterion function in `xpath/src/`; registration of every generated package in
  the root `Nargo.toml` workspace; and — the load-bearing one — whether each package is in
  the **REAL** or the **STUB-wired** partition as `scripts/run_real_tests.sh` itself defines
  it (a package is STUB if any `src/**.nr` contains `stub_` or the generator's
  `No qt3tests cases could be converted` placeholder; only the REAL subset is gated).
- **What was NOT done here:** no circuit was compiled or executed. `nargo` is absent on the
  authoring box. Counts below are of `#[test]` items and their assertions as committed, not
  of observed passes. The executing evidence is the face repo's own `CI` workflow, which
  runs `scripts/run_real_tests.sh` on every push/PR to `main` against the pinned
  `nargo 1.0.0-beta.21`.
- At `v0.3.0` the suite partitions **113 REAL / 247 STUB-wired** across 360 generated
  packages, and the runner's `KNOWN_FAILING` skip-list is **empty** — so no real package is
  being skipped. That independently corroborates the claim already made in
  [`ASSURANCE.md`](../ASSURANCE.md) §2.

## 2. `sq-t9d` — Phase-0 workspace/CI setup

| Phase-0 item | Status at `v0.3.0` | Evidence |
| --- | --- | --- |
| Convert to workspace structure | Done | root `Nargo.toml` is `[workspace]` |
| Create `xpath/` main package | Done | `xpath/Nargo.toml`, `lib.nr` + 25 modules in `xpath/src/` |
| Create `xpath_unit_tests/` package | Done | `main.nr` + 14 test modules in `xpath_unit_tests/src/` |
| Create `test_packages/` directory | Done | 360 generated packages |
| Create `scripts/` directory | Done | `generate_tests.py`, `run_real_tests.sh`, + 4 more |
| ieee754 dependency on `xpath` | Done | `sparq_ieee754 = { path = "../vendor/ieee754" }` |
| Configure workspace members | Done | all 360 packages + both libs listed |
| GitHub Actions workflow for testing | Done | `.github/workflows/ci.yml`, SHA-pinned actions |
| Test chunking for parallel CI | **Partial** | see below |

**The one residual.** Chunking exists at the *generated-file* level — each package splits
its cases into `chunk_0.nr`, `chunk_1.nr`, … — and the runner iterates package-by-package.
But CI itself is a **single `test` job with no matrix shard** and a 40-minute timeout; the
workflow header states the no-matrix choice deliberately. So the Phase-0 *deliverables*
("working workspace structure", "CI pipeline running") are met and only this sub-item is
unmet, as a face-repo CI-ergonomics choice rather than outstanding sparq work.

**Verdict: substantially complete.** Every deliverable is met; the sole gap is job-level CI
shard parallelism, which is not actionable from this repository.

## 3. `sq-t9v` — parallel work streams A–H

Every package below is present, registered as a workspace member, and in the **REAL**
(gated) partition — none is stub-wired or a placeholder.

| Stream | Scope | Packages | `#[test]` fns | Status |
| --- | --- | --- | --- | --- |
| A | duration component extraction (`fn:days/hours/minutes/seconds-from-duration`) | 4 | 24 | Complete |
| B | dateTime↔duration arithmetic + dayTimeDuration comparison | 8 | 46 | Complete |
| C | `XsdDate` type + component/comparison fns | 6 | 48 | Complete |
| D | `XsdTime` type + component/comparison fns | 6 | 42 | Complete |
| E | `op:numeric-unary-plus` / `-minus` | 2 | 68 | Complete |
| F | boolean comparison (`lt`/`gt`, plus `op:boolean-equal` beyond the plan) | 3 | 79 | Complete |
| G | float/double `round`/`ceiling`/`floor` incl. NaN/Infinity | 6 | 187 | Complete as scoped |
| H | `fn:adjust-{dateTime,date,time}-to-timezone` | 3 | 23 | Complete |

Source-side acceptance criteria are met too: `xpath/src/date.nr` and `xpath/src/time.nr`
carry the full component-extraction, timezone and comparison surfaces the plan enumerated;
`xpath/src/numeric_types.nr` carries `round_float`/`round_double`, `ceil_float`/`ceil_double`
and `floor_float`/`floor_double`; the three `adjust_*_to_timezone` functions (each with a
`_none` timezone-stripping variant) live in `date.nr`, `time.nr` and `datetime.nr`. Unit
tests exist for each new type (`date_tests.nr`, `time_tests.nr`, `datetime_tests.nr`,
`numeric_tests.nr`).

**Verdict: complete.** All eight streams meet their stated acceptance criteria.

**One residual, scoped honestly.** Stream G's *Tasks* list named
`round_half_to_even_float`/`round_half_to_even_double`; only `round_half_to_even_int` exists
(`xpath/src/numeric.nr`, re-exported as `round_half_to_even` by `xpath_fn.nr`), and the
`xpath_test_fnround_half_to_even` package is **stub-wired**, so it is excluded from the
gated set and does not fail CI. This item appears in neither Stream G's *Files to Create*
nor its *Acceptance Criteria*, so it does not block the stream — but it is a real
unimplemented function, and reading "streams A–H are done" as "`fn:round-half-to-even` works
on floats" would be wrong.

## 4. What this record does and does not support

It supports closing `sq-t9d` and `sq-t9v` as done, citing `v0.3.0` /
`0aa74d01f6e17e717c67ef370122ce2f257ce81d`. Bead state is not edited here — hand-editing
`.beads/` is forbidden (`AGENTS.md` § *Task tracking*); the closes are the orchestrator's
`bd close`.

It supports **no** claim about circuit correctness, soundness, or privacy. Structural
presence of a passing-by-construction generated test is evidence of *coverage*, not of a
correct circuit; the qt3 vectors are themselves derived by the face repo's
`generate_tests.py` oracle, and the independent sparq-side cross-check is the M1
differential harness described in [`ASSURANCE.md`](../ASSURANCE.md) §2 — which is
**verification, not proof**. The external accredited-cryptographer audit `sq-qhy4` remains
**OPEN**.
