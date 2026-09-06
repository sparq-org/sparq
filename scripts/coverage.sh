#!/usr/bin/env bash
# [OPUS-4.8] PER-CRATE line-coverage measurement (sq-hbg7).
#
# WHY PER-CRATE (and not one `cargo llvm-cov --workspace` run)
# -----------------------------------------------------------
# A single whole-workspace instrumented run does NOT finish in a bounded CI window:
# the sparq-vectors recall/diskann HNSW accuracy gates (two NON-#[ignore]d tests,
# 50k vectors x 100 queries each) run ~minutes EACH under llvm-cov instrumentation,
# and the zk bb prove tests are heavy (those are already #[ignore]d, so the default
# test set skips them). Measuring PER-CRATE lets us (a) attribute coverage to the
# right crate, (b) apply the per-crate quirks below, and (c) EXCLUDE the pathological
# tests from the per-commit set by name (documented, no silent truncation).
#
# PER-CRATE QUIRKS this script encodes (discovered by the 2026-06-14 coverage audit,
# research/coverage-and-benchmark-plan.md §2.2/§2.3):
#   - sparq-core      MUST be measured with `--features mmap,dict-spill`. Two distinct
#                     reasons, BOTH load-bearing for a representative number:
#                       * dict-spill  -> dictspill.rs / extsort.rs (the spillable build
#                                       pipeline) are otherwise compiled out / 0%.
#                       * mmap        -> the on-disk store SECURITY surface this gate is
#                                       meant to guard is `#[cfg(feature = "mmap")]`:
#                                       `Dict::open_mmap` validation, `CompressedPerm::
#                                       from_mmap`, and the `tests/mmap_corruption_oracle.rs`
#                                       integration test (itself `#![cfg(feature="mmap")]`)
#                                       + the byte-identical save/open roundtrips. Without
#                                       `mmap` that code is compiled out, so the line% is
#                                       computed over a DIFFERENT (smaller, security-free)
#                                       denominator — a non-representative measurement.
#                     dict-spill ALREADY pulls in mmap transitively (see Cargo.toml:
#                     `dict-spill = ["mmap", ...]`), so naming `mmap` here is currently a
#                     no-op for the number (measured identical: 91.23% either way) — it is
#                     stated EXPLICITLY so a future refactor that decouples dict-spill from
#                     mmap cannot silently drop the security-code coverage this gate exists
#                     to enforce. [OPUS-4.8]
#                       * rdfxml      -> the OPT-IN RDF/XML parse arm in `parse_to_triples` /
#                                       `…_with_base` (`#[cfg(feature = "rdfxml")]`) + its
#                                       direct unit tests are otherwise compiled out / 0%.
#                                       [OPUS-4.8] sq-f47w1 (survey §B1).
#   - sparq-reason    MUST be measured with `--features datalog`. The crate is NOT empty
#                     by default (RDFS/OWL-RL/N3 are default-on), but the stratified
#                     Datalog module (`src/datalog/`) is entirely `#[cfg(feature =
#                     "datalog")]`, so a default-feature run computes the line% over a
#                     denominator that EXCLUDES it — the module could rot to 0% without
#                     moving this crate's floor. See the case arm in measure() for why
#                     only `datalog` (and not the crate's other default-off features) is
#                     named. [SONNET-4.6] sq-iwf3c
#   - sparq-vectors   the two `*_recall_at_10_vs_brute_force_on_50k` tests (HNSW +
#                     DiskANN) are EXCLUDED from the per-commit subset via `--skip`
#                     (they dominate wall-clock under instrumentation). They are
#                     measured in the NIGHTLY tier (COVERAGE_TIER=nightly) so their
#                     lines still count there.
#   - sparq-cli       reports ~0% line coverage: its tests spawn the COMPILED binary
#                     as a subprocess (assert_cmd / CARGO_BIN_EXE), which is NOT the
#                     instrumented profile. This is a measurement artifact, not a real
#                     gap — its floor is set to 0 and annotated in the floor file.
#   - sparq-gpu       GPU kernels need a device; CPU-only coverage is low by design.
#   - sparq-py        EXCLUDED entirely (pyo3 / maturin toolchain, not an llvm-cov
#                     target) and sparq-bench (no tests, harness crate).
#   - W3C fixtures    are fetched first (SPARQL + inference + SHACL); the SHACL /
#                     reason-explain / EYE / rdf-turtle suites SKIP when fixtures are
#                     absent, so without this the numbers are misleadingly low + flaky.
#
# WHAT THE CONFORMANCE BINARIES DO / DON'T contribute
# ---------------------------------------------------
# The two W3C suites run as the sparq-conformance / sparq-inference-conformance
# BINARIES (`cargo run`), NOT as `cargo test`. `cargo llvm-cov --package
# sparq-conformance` runs that crate's `cargo test` (a handful of unit tests), so its
# line% is low and is NOT a meaningful gate — its floor is set to 0 and annotated.
# The W3C suites' deep exercise of sparq-core / sparq-engine is already reflected by
# those crates' own large unit+integration test bodies; re-running the binaries under
# llvm-cov to merge their profraw into core/engine is left to the NIGHTLY tier (it
# roughly doubles the core/engine cost for a few extra points — not worth per-commit).
#
# OUTPUT: writes a machine-readable per-crate summary to $OUT (default
# target/coverage/coverage-summary.json):
#   { "generated": "<iso8601>", "tier": "...", "toolchain": "...",
#     "crates": { "<crate>": { "lines_pct": <float>, "lines_covered": N,
#                              "lines_total": N, "seconds": N, "skipped_tests": [...],
#                              "features": [...], "measured": true } },
#     # present ONLY when COVERAGE_CONE filtered the selection (sq-3dr4t):
#     "cone": { "filter": [...], "measured": [...], "inherited": [...] } }
#
# USAGE:
#   scripts/coverage.sh                 # per-commit tier (FAST + medium crates)
#   COVERAGE_TIER=nightly scripts/coverage.sh   # all crates incl. heavy vectors
#   COVERAGE_TIER=full    scripts/coverage.sh   # every measurable crate, no skips
#   COVERAGE_CRATES="sparq-core sparq-parse" scripts/coverage.sh   # ad-hoc subset
#   COVERAGE_CONE="sparq-core sparq-geo" scripts/coverage.sh  # changed-cone filter (sq-3dr4t)
#   scripts/coverage.sh --print-crates  # print the crates THIS invocation would measure
#
# COVERAGE_CONE (changed-cone coverage, bead sq-3dr4t — see scripts/cone_coverage.py):
#   A space-separated crate allowlist that INTERSECTS the tier/shard selection: a crate in
#   the selection but NOT in the cone is left UNMEASURED and inherits its floor verdict
#   from main's last full run (it is unchanged, and so is everything it depends on — the
#   same soundness argument ci_select.py already makes for test skipping). It can only ever
#   NARROW the selection, never add a crate. UNSET or EMPTY => no filter at all, so every
#   fail-safe path in cone_coverage.py degrades to the pre-flip full measurement.
#   An explicit COVERAGE_CRATES BYPASSES the filter (it is already an exact subset — this
#   is what keeps coverage-gate.py --check-robust's targeted re-measure working unchanged).
#   The skipped crates are recorded in the summary JSON under "cone" (no silent truncation),
#   which coverage-gate.py prints as INHERITED and cone_coverage.py --mode report renders.
#
# Honours the AGENTS.md "no silent truncation" rule: every crate that is excluded /
# skipped / measured-with-a-quirk is recorded explicitly in the JSON and printed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TIER="${COVERAGE_TIER:-per-commit}"          # per-commit | nightly | full
OUT="${COVERAGE_OUT:-$ROOT/target/coverage/coverage-summary.json}"
FETCH_FIXTURES="${COVERAGE_FETCH_FIXTURES:-1}"

# [SONNET-4.6] sq-3dr4t: --print-crates — resolve the tier/shard selection AND the
# COVERAGE_CONE filter, print the crate list, exit. NO cargo, NO fixtures, NO measurement,
# so the selection+filter logic is unit-testable hermetically (scripts/tests/
# test_cone_coverage.py drives it) instead of only being observable by reading the log of
# a full instrumented CI run.
PRINT_CRATES=0
if [ "${1:-}" = "--print-crates" ]; then
  PRINT_CRATES=1
  FETCH_FIXTURES=0
fi

mkdir -p "$(dirname "$OUT")"

# ---- crate tiers ------------------------------------------------------------
# FAST/MEDIUM crates measured EVERY commit. (Timings in the floor file header.)
PER_COMMIT_CRATES=(
  sparq-core sparq-engine sparq-cli sparq-server sparq-serve
  sparq-reason sparq-shacl sparq-geo sparq-text sparq-hdt sparq-rsp
  sparq-introspect sparq-sim sparq-solid sparq-nlq sparq-mpc
  sparq-parse sparq-gpu sparq-wasm sparq-zk sparq-zk-compose
  sparq-vectors                # measured with the two 50k tests SKIPPED (see below)
  sparq-conformance            # low %, floor 0 (test driver) — kept for presence
  # [OPUS-4.8] sq-bif.1: the three OPT-IN native crates that were untracked by BOTH
  # coverage gates. Their whole surface is feature-gated (fedclient/fedplan empty by
  # default; prov has a `reason` extra), so they MUST be measured WITH those features
  # on — the `case` in measure() below names them, or a default-feature build would
  # report an empty-crate number. The 4 tier-b `-wasm` crates are NOT added here:
  # their gate-exercising surface is the `#[wasm_bindgen]` JS API that only the
  # `wasm-pack test --node` runner reaches, so a NATIVE llvm-cov number is a
  # misleadingly-low artifact (measured 55-79% native; same class as sparq-cli's
  # subprocess artifact) — they are floor-0 + presence-gated in the JSONs instead.
  sparq-fedclient sparq-fedplan sparq-prov
  # [FABLE-5] sq-lsp7k.1.1: sparq-forms — opt-in headless SHACL-to-form derivation.
  # No cargo features (whole surface default-compiled), so no measure() case arm.
  sparq-forms
  # [SONNET-4.6] sq-97cxm: sparq-jsonld — native JSON-LD 1.1 processing and RDF conversion.
  # No cargo features (whole surface default-compiled), so no measure() case arm.
  sparq-jsonld
  # [OPUS-4.8] sq-bif.7: the OPT-IN ODRL usage-control policy crate, untracked by BOTH
  # gates. The STATELESS evaluator (parse/eval/compare/hierarchy) is default-on, but the
  # stateful `odrl:count` counter stores (the `count`/`count_file`/`count_backend` modules
  # + their tests) are `#![cfg(feature = "count-enforcement")]`. So it MUST be measured
  # WITH --features count-enforcement (the `case` in measure() below names it) — a default
  # build compiles that whole surface out and reports a non-representative number.
  sparq-policy
  # [OPUS-4.8] sq-bif.8 / sq-bif.9: two OPT-IN, standalone crates that nothing in the
  # default build depends on, both untracked by BOTH gates. Unlike the federation/policy
  # crates above they have NO opt-in features (sparq-algos: `default = []` only; sparq-canon:
  # no `[features]` table), so their WHOLE surface is compiled in a default-feature build —
  # they need NO `case` arm in measure() and are measured exactly as-is. sparq-canon's number
  # reflects its tests/rdf_canon_suite.rs driving the full 86-entry W3C RDFC-1.0 manifest plus
  # the focused bnode-isomorphism / oxrdf-bridge unit cases; sparq-algos' reflects the inline
  # PageRank/centrality/community oracles plus the tests/topology_oracles.rs integration suite.
  sparq-algos sparq-canon
  # [OPUS-4.8] sq-qcnn.23 (epic sq-qcnn): the two CORE OWL 2 reasoners that were
  # untracked by BOTH coverage gates.  Their whole gate-exercising surface is behind
  # DEFAULT-OFF features, so a default-feature `cargo llvm-cov -p <crate>` would build
  # the crate with the EL classifier / QL rewriter compiled OUT and report a
  # non-representative (low or empty) number.  Each MUST be measured WITH its
  # whole-surface features on — the `case` in measure() below names them:
  #   * sparq-reason-el  —  `rbox` (CR10/CR11 role-inclusion automaton, Phase E2) +
  #                          `hasse` (DirectHierarchy transitive-reduction, Phase E3).
  #                          The E1 classify core is always present; rbox+hasse together
  #                          exercise the full normalise->saturate->reduce pipeline.
  #   * sparq-reason-ql  —  `experimental` (the PerfectRef rewriting pass; without it
  #                          only the cheap CQ-shape gate types compile in, which is not
  #                          a representative number for the rewriter floor).
  # Neither cdomain (requires sparq-substrate dep) nor abox is included in the EL
  # measurement: the bead architect explicitly specified rbox,hasse as the whole-surface
  # ceiling for this gate wiring pass (those features close the SNOMED/EL+ gap; cdomain/
  # abox are separately tracked beads).  Nothing in the workspace depends on these crates
  # by default, so their tests are otherwise run ONLY by the feature-matrix leg; wiring
  # them here brings both cores under the line-coverage ratchet ("no crate silently
  # dropped", matching sparq-substrate/sparq-canon above). [OPUS-4.8]
  sparq-reason-el sparq-reason-ql
  # [OPUS-4.8] sq-qcnn.3 (epic sq-qcnn, umbrella sq-qonbz): the shared zero-overhead
  # evaluation SUBSTRATE — the correctness core (the id-tuple Row/Key/Posting vocabulary,
  # the XSD numeric value tower, the four id-tuple join kernels, and the SPARQL term total
  # order). Its ENTIRE surface is behind DEFAULT-OFF features (numeric/join/compare/rows),
  # so a default-feature `cargo llvm-cov -p sparq-substrate` would instrument an EMPTY crate
  # and report a meaningless number — it MUST be measured WITH those features on (the `case`
  # in measure() below names `--features numeric,join,compare,rows`, mirroring the sparq-core
  # `mmap,dict-spill` and sparq-fedclient/-policy quirks). Nothing in the workspace depends on
  # this crate, so its unit tests are otherwise run ONLY by the feature-matrix leg; wiring it
  # here brings the correctness core under the line-coverage ratchet ("no crate silently
  # dropped").
  sparq-substrate
  # [OPUS-4.8] sq-6vshe.4: seam 1 of the sparq-engine facade split — the RDF writer matrix
  # (Turtle/TriG/N-Quads/JSON-LD, buffered + streaming) peeled into an internal sub-crate. Its
  # whole surface is behind DEFAULT-OFF `serialize-rdf`, so it MUST be measured WITH
  # `serialize-rdf,streaming-serialization` (the `case` in measure() below names them). Bringing
  # the moved writer + its ~2.9k test LOC under the ratchet keeps "no crate silently dropped".
  sparq-engine-serialize
  # [OPUS-4.8] sq-6vshe.4: seam A2 of the sparq-engine facade split — the SPARQL 1.1 federated-
  # SERVICE client (HTTP transport, SPARQL-Results JSON/XML parse, bound-join batching, SSRF
  # egress policy) peeled into an internal sub-crate. Its whole surface is behind DEFAULT-OFF
  # `service`, so it MUST be measured WITH `service` (the `case` in measure() below names it).
  # Brings the moved client + its ~1.3k test LOC under the ratchet ("no crate silently dropped").
  sparq-engine-service
)
# Crates whose HEAVY tests are only run in the nightly tier.
NIGHTLY_ONLY_NOTE="sparq-vectors heavy 50k recall/diskann tests run only in nightly tier"

# ---- per-commit MATRIX shard groups (sq-p0hcd) [OPUS-4.8] --------------------
# WHY: the per-commit `coverage` job measured ALL of PER_COMMIT_CRATES in ONE serial
# loop, which ran ~28 min wall-clock (2026-07-02 CI profile, run 28624093902): a single
# `Measure + enforce` step = 1691s, of which the per-crate loop was 1676s, DOMINATED by
# sparq-engine (668s ≈ 40%), then sparq-core (210s), sparq-vectors (167s), sparq-solid
# (165s), sparq-server (114s). Serial => that whole cost is on the merge-queue critical
# path. Splitting the loop across parallel MATRIX shards makes crates measure concurrently;
# the wall-clock then floors at the slowest single shard.
#
# The groups are LPT-bin-packed by that measured per-crate wall time. [FABLE-5] sq-piapk:
# the sparq-engine elephant (668s) — which WAS the slowest shard ALONE — is no longer here;
# it moved to the dedicated cross-runner SPLIT pipeline (DEDICATED_PIPELINE_CRATES below),
# so the slowest of these 3 remaining shards (~336s of measured work EACH, balanced) now sets
# the SHARD_GROUPS wall-clock floor, and the engine split's merged wall-clock sits alongside
# it (target: both well under the old ~668s engine pole). Each shard runs the IDENTICAL
# coverage.sh + `coverage-gate.py --check-robust` over its subset, so the ratchet semantics
# are unchanged (every per-crate floor still enforced; a shard whose crate is below floor
# fails, failing the gate). NB: these seconds are a MEASUREMENT-ORDER lower bound — a fresh
# shard also re-builds the shared instrumented deps — so keep the count SMALL: more shards
# only add duplicated build overhead without lowering the (now non-engine) wall-clock floor.
#
# [FABLE-5] sq-piapk: sparq-engine is NO LONGER a SHARD_GROUPS entry — it is measured by a
# DEDICATED CROSS-RUNNER SPLIT pipeline (scripts/coverage-engine-shard.sh + the
# coverage-engine-{archive,run,merge} jobs in ci.yml). Its per-commit `floor` is STILL
# enforced — by that pipeline's merge+`coverage-gate.py --check` — so it must NOT appear in
# any SHARD_GROUPS shard (measuring it twice would double the wall-clock it exists to cut),
# but it must ALSO NOT be flagged "missing from every shard => un-gated" by --check-shards.
# The DEDICATED_PIPELINE_CRATES set below records exactly that: crates that are in
# PER_COMMIT_CRATES and gated, but gated OUTSIDE SHARD_GROUPS. The invariant becomes:
#     union(SHARD_GROUPS) ∪ DEDICATED_PIPELINE_CRATES  ==  PER_COMMIT_CRATES   (disjoint).
DEDICATED_PIPELINE_CRATES=(sparq-engine)

# INVARIANT (guarded by `coverage.sh --check-shards`, a fast no-compile CI step): the union
# of SHARD_GROUPS PLUS DEDICATED_PIPELINE_CRATES equals PER_COMMIT_CRATES exactly and is
# pairwise-disjoint — so a crate ADDED to PER_COMMIT_CRATES but not to a shard NOR the
# dedicated set fails the guard LOUDLY instead of being silently un-gated (measured nowhere
# => floor unenforced), and a crate in BOTH a shard and the dedicated set (double-measured)
# also fails loudly.
SHARD_GROUPS=(
  # [FABLE-5] sq-piapk: shard 1 (sparq-engine, ~668s ALONE) was REMOVED — engine now has the
  # dedicated cross-runner split pipeline (DEDICATED_PIPELINE_CRATES). The 3 remaining shards
  # are the old shards 2-4, unchanged. Their COVERAGE_SHARD indices renumber 1..3, but each
  # shard's CRATE SET is byte-identical to before, so every non-engine floor is enforced by
  # the SAME crate group as prior — only the shard NUMBER moved (the ci.yml matrix is [1,2,3]).
  # shard 1 (was 2; ~336s measured)
  "sparq-core sparq-mpc sparq-fedclient sparq-geo sparq-cli sparq-conformance sparq-text sparq-policy sparq-nlq sparq-sim sparq-jsonld"
  # shard 2 (was 3; ~336s measured; + sparq-reason-el [EL rbox+hasse] + sparq-reason-ql [QL experimental], sq-qcnn.23)
  "sparq-vectors sparq-zk-compose sparq-gpu sparq-serve sparq-reason sparq-reason-el sparq-reason-ql sparq-hdt sparq-shacl sparq-fedplan sparq-zk sparq-substrate sparq-introspect"
  # shard 3 (was 4; ~336s measured; + sparq-engine-serialize [seam 1] + sparq-engine-service [seam A2], sq-6vshe.4)
  "sparq-solid sparq-server sparq-wasm sparq-canon sparq-rsp sparq-prov sparq-parse sparq-algos sparq-engine-serialize sparq-engine-service sparq-forms"
)
SHARD_TOTAL=${#SHARD_GROUPS[@]}

# --check-shards: verify the SHARD_GROUPS + DEDICATED_PIPELINE_CRATES partition invariant,
# then exit. Fast, NO compile — run as an early CI gate (and locally) so a partition drift
# fails before any build. Emits the offending crates on failure so the fix is obvious.
if [ "${1:-}" = "--check-shards" ]; then
  python3 - "$SHARD_TOTAL" "${PER_COMMIT_CRATES[*]}" "${DEDICATED_PIPELINE_CRATES[*]}" "${SHARD_GROUPS[@]}" <<'PY'
import sys
total = int(sys.argv[1])
per_commit = sys.argv[2].split()
dedicated = sys.argv[3].split()      # [FABLE-5] sq-piapk: gated OUTSIDE SHARD_GROUPS
groups = [g.split() for g in sys.argv[4:]]
assert len(groups) == total, f"SHARD_TOTAL={total} but {len(groups)} groups"
flat = [c for g in groups for c in g] + dedicated
dupes = sorted({c for c in flat if flat.count(c) > 1})
union = set(flat)
want = set(per_commit)
missing = sorted(want - union)      # in PER_COMMIT_CRATES but NO shard/dedicated -> UN-GATED
extra = sorted(union - want)        # in a shard/dedicated but not PER_COMMIT_CRATES -> stale
ok = True
if dupes:
    print(f"::error::coverage --check-shards: crate(s) measured in >1 place "
          f"(shard overlap or shard∩dedicated — double-measured): {dupes}"); ok = False
if missing:
    print(f"::error::coverage --check-shards: PER_COMMIT_CRATES crate(s) missing from every "
          f"shard AND the dedicated pipeline (would be silently UN-GATED): {missing}"); ok = False
if extra:
    print(f"::error::coverage --check-shards: shard/dedicated crate(s) not in "
          f"PER_COMMIT_CRATES (stale/typo): {extra}"); ok = False
if ok:
    print(f"coverage --check-shards: OK — {total} shard(s) + {len(dedicated)} dedicated-"
          f"pipeline crate(s) ({', '.join(dedicated)}) partition "
          f"{len(per_commit)} PER_COMMIT_CRATES exactly (disjoint, complete)")
sys.exit(0 if ok else 1)
PY
  exit $?
fi

# Tests EXCLUDED from the per-commit subset, by name (libtest --skip substring).
# DOCUMENTED here so the exclusion is never silent.
VECTORS_HEAVY_SKIP="recall_at_10_vs_brute_force_on_50k"   # matches HNSW + DiskANN 50k

# ---- conformance-binary merge (sq-bjct, NIGHTLY tier only) [OPUS-4.8] --------
# The W3C SPARQL + inference suites run as the sparq-conformance /
# sparq-inference-conformance BINARIES (`cargo run`), NOT as `cargo test`. So a
# plain `cargo llvm-cov --package sparq-core` (which only runs that crate's
# `cargo test`) does NOT count the suites' deep exercise of sparq-core /
# sparq-engine — their per-crate floors otherwise reflect only unit+integration
# tests.
#
# In the nightly/full tiers we MERGE the two conformance binaries' coverage into
# the sparq-core / sparq-engine reports via cargo-llvm-cov's documented
# accumulate-then-report flow (profraw is NOT clobbered between `--no-report`
# invocations; it only resets on `clean`):
#   1. `cargo llvm-cov clean --workspace`            — empty the profraw dir
#   2. `cargo llvm-cov test  --no-report -p X ...`   — capture X's unit+integration
#      tests' profraw (no report yet)
#   3. `cargo llvm-cov run   --no-report --bin sparq-conformance ...`            \
#      `cargo llvm-cov run   --no-report --bin sparq-inference-conformance ...`  —
#      capture each suite binary's profraw ON TOP of (2) without wiping it
#   4. `cargo llvm-cov report -p X --json --summary-only` — merge ALL accumulated
#      profraw and emit X's line% over the merged set.
# Step 4's report for package X therefore includes every X line that any of
# (2)/(3) executed. We `report -p X` per crate so the number stays attributed to
# X alone (the binaries themselves stay floor-0 / presence-gated).
#
# This roughly DOUBLES the core/engine measurement cost (the suites are a second
# heavy exercise on top of the tests), which is exactly why it is nightly-only —
# the per-commit tier keeps the cheap test-only measurement unchanged.
#
# FIXTURE-ABSENCE BEHAVIOUR (NOT a graceful degrade): step (3)'s suite binaries
# REQUIRE their fixtures. If `<root>/sparql` (sparq-conformance) or the inference
# RDF-tests dir (sparq-inference-conformance) is absent, that binary prints
# "test data not found … run scripts/fetch-*.sh first" and `std::process::exit(2)`s
# (see crates/sparq-conformance/src/main.rs and src/bin/inference.rs) — it does NOT
# skip-and-continue. A non-zero exit BREAKS measure_merged()'s `&&` chain, so step
# (4)'s `report` never runs, rc!=0, and the crate is recorded UNMEASURED — the
# step-2 test-only profraw is DISCARDED for that crate, not reported as a lower
# number. So fixtures must be present (fetched above) for the merge to produce a
# number at all; on absence the crate falls through to the unmeasured-row path
# (which the robust gate then re-measures test-only). This is fail-closed, not a
# silent lower measurement.
CONFORMANCE_MERGE_CRATES="sparq-core sparq-engine"
# Enable the merge only outside the cheap per-commit tier (set to 0 to force-disable).
CONFORMANCE_MERGE="${CONFORMANCE_MERGE:-auto}"

# ---- fixtures (idempotent; no-ops when already pinned) ----------------------
if [ "$FETCH_FIXTURES" = "1" ]; then
  echo "==> Fetching pinned W3C fixtures (idempotent)…"
  ./scripts/fetch-conformance.sh        || { echo "WARN: SPARQL fixture fetch failed"; }
  ./scripts/fetch-inference-suites.sh   || { echo "WARN: inference fixture fetch failed"; }
  ./crates/sparq-shacl/fetch-shacl-tests.sh || { echo "WARN: SHACL fixture fetch failed"; }
fi

# ---- pick the crate list for this run --------------------------------------
# Precedence (sq-p0hcd) [OPUS-4.8]:
#   1. COVERAGE_CRATES  — explicit subset (ad-hoc runs AND the robust gate's targeted
#      re-measure of sub-floor crates; must win so a shard re-measures only ITS offenders).
#   2. COVERAGE_SHARD=N — 1-based matrix shard index: measure SHARD_GROUPS[N-1] (the
#      per-commit MATRIX leg). Validated against SHARD_TOTAL.
#   3. neither          — the whole PER_COMMIT_CRATES set (single-job / nightly / local).
if [ -n "${COVERAGE_CRATES:-}" ]; then
  read -r -a CRATES <<<"$COVERAGE_CRATES"
elif [ -n "${COVERAGE_SHARD:-}" ]; then
  case "$COVERAGE_SHARD" in
    ''|*[!0-9]*) echo "ERROR: COVERAGE_SHARD='$COVERAGE_SHARD' is not a positive integer" >&2; exit 2 ;;
  esac
  if [ "$COVERAGE_SHARD" -lt 1 ] || [ "$COVERAGE_SHARD" -gt "$SHARD_TOTAL" ]; then
    echo "ERROR: COVERAGE_SHARD=$COVERAGE_SHARD out of range 1..$SHARD_TOTAL" >&2; exit 2
  fi
  read -r -a CRATES <<<"${SHARD_GROUPS[$((COVERAGE_SHARD - 1))]}"
  echo "==> shard $COVERAGE_SHARD/$SHARD_TOTAL: measuring ${#CRATES[@]} crate(s): ${CRATES[*]}"
else
  CRATES=("${PER_COMMIT_CRATES[@]}")
fi

# ---- changed-cone filter (sq-3dr4t) [SONNET-4.6] ----------------------------
# INTERSECT the selection above with COVERAGE_CONE (see the COVERAGE_CONE header block).
# Pure narrowing: a cone entry that is not in the selection is ignored, so this can never
# widen what is measured, and an unset/empty COVERAGE_CONE is a no-op. Precedence-wise it
# sits BELOW COVERAGE_CRATES: an explicit subset is already exact (the --check-robust
# re-measure path), so filtering it again could only drop a crate the gate asked for.
declare -a CONE_INHERITED=()
declare -a CONE_LIST=()
# Split FIRST and gate on the WORD COUNT, never on string-emptiness: a whitespace-only
# COVERAGE_CONE (e.g. the CI `tr '\n' ' '` of an EMPTY cone-crates.txt) is a non-empty
# STRING that splits to ZERO crates. Gating on `-n "$COVERAGE_CONE"` would treat that as
# "a cone containing nothing", intersect to nothing, and measure NOTHING — inverting the
# fail-safe into the worst possible outcome. Word count makes it the intended no-op.
if [ -n "${COVERAGE_CONE:-}" ]; then read -r -a CONE_LIST <<<"$COVERAGE_CONE"; fi
if [ ${#CONE_LIST[@]} -gt 0 ] && [ -z "${COVERAGE_CRATES:-}" ]; then
  declare -a CONE_KEPT=()
  for c in ${CRATES[@]+"${CRATES[@]}"}; do
    in_cone=0
    for k in ${CONE_LIST[@]+"${CONE_LIST[@]}"}; do
      if [ "$c" = "$k" ]; then in_cone=1; break; fi
    done
    if [ "$in_cone" -eq 1 ]; then CONE_KEPT+=("$c"); else CONE_INHERITED+=("$c"); fi
  done
  CRATES=(${CONE_KEPT[@]+"${CONE_KEPT[@]}"})
  echo "==> changed-cone filter (sq-3dr4t): measuring ${#CRATES[@]} crate(s)" \
       "(${#CONE_INHERITED[@]} inherit their floor verdict from main — unchanged)"
  [ ${#CRATES[@]} -gt 0 ] && echo "    measured : ${CRATES[*]}"
  [ ${#CONE_INHERITED[@]} -gt 0 ] && echo "    inherited: ${CONE_INHERITED[*]}"
  if [ ${#CRATES[@]} -eq 0 ]; then
    echo "    NOTE: nothing in this shard is in the cone — no instrumented run needed."
  fi
fi

if [ "$PRINT_CRATES" -eq 1 ]; then
  # One crate per line so a caller can read it without re-splitting a joined string.
  for c in ${CRATES[@]+"${CRATES[@]}"}; do echo "$c"; done
  exit 0
fi

TOOLCHAIN="$(rustc --version 2>/dev/null || echo unknown)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/sparq-cov.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

declare -a ROWS=()
TOTAL_START=$(date +%s)

# [OPUS-4.8] sq-bjct: decide whether THIS crate's measurement should merge the
# conformance binaries' coverage. Yes only when (a) the crate is one of the
# CONFORMANCE_MERGE_CRATES, AND (b) the merge is enabled for this tier: "auto"
# enables it for any tier except per-commit (it ~doubles core/engine cost), and an
# explicit CONFORMANCE_MERGE=1/0 forces it on/off (1 even lets a `full`-tier ad-hoc
# run merge; 0 disables for debugging / a fast nightly).
want_conformance_merge() {
  local crate="$1"
  case " $CONFORMANCE_MERGE_CRATES " in *" $crate "*) ;; *) return 1 ;; esac
  case "$CONFORMANCE_MERGE" in
    0|false|no) return 1 ;;
    1|true|yes) return 0 ;;
    auto|*)     [ "$TIER" != "per-commit" ] && return 0 || return 1 ;;
  esac
}

# [OPUS-4.8] sq-bjct: measure ONE crate with the conformance binaries MERGED into its
# report (nightly tier). Uses cargo-llvm-cov's accumulate-then-report flow — see the
# CONFORMANCE-BINARY MERGE header block for the full rationale. Emits the SAME JSON row
# shape as measure(), tagging features += "conformance-merge" so the summary records that
# this crate's number includes the suite binaries. On ANY failure to capture the merged
# profraw — including fixture-absence, where a suite binary `exit(2)`s and breaks the `&&`
# chain BEFORE step (4)'s report (see the FIXTURE-ABSENCE note in the header) — it leaves
# rc!=0 and the caller falls through to the unmeasured-row path (the robust gate then
# re-measures test-only). It records the crate UNMEASURED rather than a silent low number.
measure_merged() {
  local crate="$1"
  local -a features=("conformance-merge")
  local -a feat_flags=()
  # sparq-core keeps its mmap,dict-spill security surface (see PER-CRATE QUIRKS).
  # [OPUS-4.8] sq-f47w1 (survey §B1): `rdfxml` is added so the OPT-IN RDF/XML parse arm
  # (`#[cfg(feature = "rdfxml")]` in `parse_to_triples` / `…_with_base`) is COMPILED and its
  # direct unit tests run, MEASURING the new lines for the coverage ratchet (without the
  # feature those lines are cfg'd out and never enter the report).
  if [ "$crate" = "sparq-core" ]; then
    feat_flags+=(--features mmap,dict-spill,rdfxml); features+=("mmap" "dict-spill" "rdfxml")
  fi
  local start end rc=0 json="$WORK/$crate.json" err="$WORK/$crate.err"
  start=$(date +%s)
  : > "$err"
  {
    # 1. Start from an empty profraw set so ONLY this measurement's runs count.
    cargo llvm-cov clean --workspace &&
    # 2. Capture the crate's own unit+integration tests (serial — same profraw-race
    #    mitigation as measure(); see sq-x4jy). No report yet.
    cargo llvm-cov test --no-report --package "$crate" "${feat_flags[@]}" \
      -- --test-threads=1 &&
    # 3. Capture each suite binary ON TOP of (2) without wiping the profraw. Reports
    #    are written into $WORK so they never litter the repo root.
    cargo llvm-cov run --no-report --release -p sparq-conformance \
      --bin sparq-conformance -- --report "$WORK/conformance-report.md" &&
    cargo llvm-cov run --no-report --release -p sparq-conformance \
      --bin sparq-inference-conformance -- --report "$WORK/inference-report.md" &&
    # 4. Merge ALL accumulated profraw and emit X's line% over the merged set.
    #    NB: `report` does NOT accept --features (that is a build-time flag, applied
    #    in steps 2-3 which chose what code was compiled); passing it here errors with
    #    "invalid option '--features' for subcommand 'report'". The report just
    #    summarises the accumulated profraw for package X.
    cargo llvm-cov report --package "$crate" \
      --summary-only --json --output-path "$json"
  } >/dev/null 2>>"$err" || rc=$?
  end=$(date +%s)

  if [ "$rc" -ne 0 ] || [ ! -s "$json" ]; then
    echo "  !! $crate (conformance-merge) FAILED to measure (rc=$rc) — see error tail:"
    tail -8 "$err" | sed 's/^/     /'
    ROWS+=("$(python3 - "$crate" "$((end-start))" <<'PY'
import json,sys
print(json.dumps({"crate":sys.argv[1],"seconds":int(sys.argv[2]),"measured":False}))
PY
)")
    return
  fi

  local feats_json
  feats_json=$(printf '%s\n' "${features[@]}" | python3 -c 'import sys,json;print(json.dumps([l for l in sys.stdin.read().split("\n") if l]))')
  local row
  row=$(python3 - "$crate" "$json" "$((end-start))" "$feats_json" <<'PY'
import json,sys
crate,path,secs,feats=sys.argv[1],sys.argv[2],int(sys.argv[3]),json.loads(sys.argv[4])
d=json.load(open(path))
t=d["data"][0]["totals"]["lines"]
print(json.dumps({"crate":crate,"lines_pct":round(t["percent"],2),
  "lines_covered":t["covered"],"lines_total":t["count"],
  "seconds":secs,"features":feats,"skipped_tests":[],"measured":True}))
PY
)
  ROWS+=("$row")
  printf "  %-20s lines=%6s%%  %4ss  feat=%s\n" "$crate" \
    "$(echo "$row" | python3 -c 'import sys,json;print(json.load(sys.stdin)["lines_pct"])')" \
    "$((end-start))" "${features[*]}"
}

measure() {
  local crate="$1"; shift
  # [OPUS-4.8] sq-bjct: in the nightly/full tiers, sparq-core / sparq-engine are
  # measured with the W3C conformance binaries MERGED into the report (those suites
  # run as BINARIES, not `cargo test`, so the plain per-crate measurement misses
  # their deep exercise of these crates). Per-commit is unchanged.
  if want_conformance_merge "$crate"; then
    measure_merged "$crate"
    return
  fi
  local features=()           # array of feature names recorded in JSON
  local skips=()              # array of skipped-test substrings recorded in JSON
  local -a cargo_args=(--package "$crate")
  # [OPUS-4.8] sq-x4jy (2nd flake): ALWAYS run the per-crate test set SERIALLY
  # (`--test-threads=1`). This kills a profraw-merge race behind a DISTINCT flake from
  # the env-var race fixed in ea0ca3e: `coverage ratchet` intermittently reported
  # sparq-core ~71% (between the dict-spill-only ~65% and the full ~91%) as a VALID
  # rc=0 measurement — NOT an aborted binary, so the rc!=0/empty-JSON retry above did
  # not (and should not) catch it. Root cause: libtest runs a binary's #[test]s on N
  # threads by default; with `-Cinstrument-coverage`, every test process writes to a
  # `%p-%8m` LLVM merge-pool .profraw with file-locked ONLINE merging on exit. Under CI
  # contention (fewer cores, cold cache, mem pressure) a binary's counters could fail to
  # land in / merge into its .profraw before `llvm-profdata merge` collected them, so its
  # covered lines showed UNCOVERED — a low-but-valid number, never retried. Forcing one
  # test thread per binary serialises all writes to that binary's profile counters,
  # removing the intra-binary race; combined with the clean profraw dir cargo-llvm-cov
  # already establishes per run, the cross-binary collection is deterministic. Measured
  # STABLY ≥ 91.28% for sparq-core across repeated runs after this change. The (small)
  # wall-clock cost is acceptable for the per-commit tier and is the price of a
  # deterministic gate; it does NOT mask regressions (it re-measures the SAME tests).
  local -a test_args=(--test-threads=1)
  local subcmd=""             # "" => `cargo llvm-cov`, "test" => filterable form

  case "$crate" in
    sparq-core)
      # mmap is named EXPLICITLY (not relied on via the dict-spill -> mmap transitive
      # dep) so the on-disk-store security surface this gate guards is always compiled +
      # exercised. See the PER-CRATE QUIRKS header for the full rationale. [OPUS-4.8]
      # [OPUS-4.8] sq-f47w1: `rdfxml` so the OPT-IN RDF/XML parse arm is compiled + its
      # direct tests run (measured by the ratchet); cfg'd out otherwise. Mirrors line 227.
      cargo_args+=(--features mmap,dict-spill,rdfxml); features+=("mmap" "dict-spill" "rdfxml") ;;
    sparq-vectors)
      # Only skip the heavy 50k tests OUTSIDE the nightly/full tiers.
      if [ "$TIER" = "per-commit" ]; then
        subcmd="test"; test_args+=(--skip "$VECTORS_HEAVY_SKIP"); skips+=("$VECTORS_HEAVY_SKIP")
      fi ;;
    # [OPUS-4.8] sq-bif.1: the opt-in federation/provenance crates are ENTIRELY
    # feature-gated — a default-feature `cargo llvm-cov -p <crate>` would build an
    # empty crate and report a meaningless number. Name the features that turn the
    # whole surface ON so the measured line% reflects the real (feature-on) code the
    # floor gates. (Mirrors the sparq-core `--features mmap,dict-spill` quirk above.)
    sparq-fedclient)
      # `fedclient` enables the module surface + REUSE seams; `fedclient-adaptive`
      # turns on the Phase-7 adaptive re-planning module (its tests are gated on it).
      cargo_args+=(--features fedclient,fedclient-adaptive)
      features+=("fedclient" "fedclient-adaptive") ;;
    sparq-fedplan)
      # `fedplan` enables the planner; `adaptive-replan` the live re-planning module.
      cargo_args+=(--features fedplan,adaptive-replan)
      features+=("fedplan" "adaptive-replan") ;;
    sparq-prov)
      # `reason` turns on the sparq-reason proof-tree -> PROV-O lineage bridge module
      # (its integration test, tests/reason_prov.rs, is gated on it). The CONSTRUCT/
      # update lineage core is default-on.
      cargo_args+=(--features reason); features+=("reason") ;;
    # [OPUS-4.8] sq-bif.7: `count-enforcement` turns on the stateful `odrl:count`
    # counter-store surface (the `count`/`count_file`/`count_backend` modules + their
    # `#![cfg(feature = "count-enforcement")]` integration tests). The stateless ODRL
    # evaluator is default-on; without this feature that whole surface is compiled out
    # and the number is non-representative. (Mirrors the sparq-prov `reason` quirk above.)
    sparq-policy)
      cargo_args+=(--features count-enforcement); features+=("count-enforcement") ;;
    # [OPUS-4.8] sq-qcnn.3 (epic sq-qcnn): the shared eval SUBSTRATE's whole surface is
    # DEFAULT-OFF — measure it with the four features that turn the correctness core ON
    # (`numeric` = XSD value tower, `join` = the four id-tuple join kernels, `compare` =
    # the SPARQL term total order, `rows` = the id-tuple Row/Key/Posting vocabulary). A
    # default-feature build compiles NONE of it (empty crate -> meaningless number); this
    # names the CORRECTNESS-CORE set so the measured line% reflects the real code the floor
    # gates, exactly as sparq-core/-fedclient/-policy above name their whole-surface features.
    #
    # [OPUS-5] PR #3799: do NOT add `overhead` here. It used to be true that this was also the
    # crate's MAXIMAL feature set; it no longer is — the feature-matrix leg now enables
    # `rows,numeric,join,compare,overhead` so `src/overhead.rs`'s tests actually gate (they
    # were compiled by NO required check before). `overhead` is the zero-overhead DELTA
    # TIMING harness, not correctness surface: instrumenting it would fold ~2k lines of
    # measurement/reporting code into the denominator behind only a handful of tests and
    # would push this crate under its floor of 96 for no correctness gain. Coverage measures
    # the correctness core; the matrix leg EXECUTES everything. They are deliberately
    # different sets — keep them that way.
    sparq-substrate)
      cargo_args+=(--features numeric,join,compare,rows)
      features+=("numeric" "join" "compare" "rows") ;;
    # [OPUS-4.8] sq-qcnn.23 (epic sq-qcnn): OWL 2 EL consequence-based classifier.
    # Whole gate-exercising surface is behind DEFAULT-OFF `rbox` + `hasse`:
    #   * `rbox`  — CR10/CR11 role-inclusion + property-chain automaton (Phase E2).
    #   * `hasse` — DirectHierarchy transitive reduction to direct-subsumer diagram (E3).
    # Without BOTH, the normalize->saturate->reduce pipeline is only partially compiled
    # and the line% reflects only the E1 core — a non-representative measurement.
    # Note: `cdomain` (concrete domains, requires sparq-substrate dep) and `abox`
    # (ABox internalization) are NOT included: the architect-specified whole-surface for
    # this gate wiring pass is rbox+hasse (Cargo.toml Phases E2+E3, the SNOMED/EL+ gap).
    sparq-reason-el)
      cargo_args+=(--features rbox,hasse)
      features+=("rbox" "hasse") ;;
    # [SONNET-4.6] sq-iwf3c (epic sq-6tykl, design record
    # research/stratified-datalog-rules.md §5/§6 item 8): the OPT-IN STRATIFIED DATALOG
    # module. Unlike the crates above, sparq-reason is NOT empty by default — the
    # RDFS/OWL-RL/N3 chainers are default-on — but `crates/sparq-reason/src/datalog/`
    # (parser + stratification checker + per-stratum evaluator + `datalog::incr` DRed
    # maintenance, plus its in-crate differential-oracle suite) is entirely behind the
    # DEFAULT-OFF `datalog` feature, so a default-feature run compiles it OUT and the
    # module never enters this crate's line-coverage floor at all. Naming the feature
    # here brings it under the ratchet, the same way rbox+hasse does for sparq-reason-el.
    #
    # ONLY `datalog` is named, deliberately. sparq-reason carries several other
    # default-off features (explain / profile / d-entail / rif / compiled-rules /
    # reify / …) whose wiring is separately beaded; adding them here would change the
    # denominator for reasons this bead did not measure. Scope = the one feature the
    # bead names.
    #
    # `datalog`'s tests are ALL in-crate unit tests (`#[cfg(test)] mod tests` +
    # `#[cfg(test)] mod oracle` in src/datalog/mod.rs) — there is no
    # `#![cfg(feature = "datalog")]` integration test under tests/ — so `cargo llvm-cov
    # test -p sparq-reason --features datalog` runs the whole suite with no extra args.
    sparq-reason)
      cargo_args+=(--features datalog); features+=("datalog") ;;
    # [OPUS-4.8] sq-qcnn.23 (epic sq-qcnn): OWL 2 QL query-rewriting reasoner.
    # Whole gate-exercising surface is behind DEFAULT-OFF `experimental`:
    # without it only the cheap CQ-shape gate types compile in and the PerfectRef
    # rewriter is compiled OUT — a non-representative (low) number. Mirrors the
    # sparq-reason-el rbox+hasse quirk and the sparq-fedplan `fedplan` quirk above.
    # [FABLE-5] sq-p6yb7: + `ql-consistency` (the DL-Lite_R violation-query consistency
    # check module; without it consistency.rs compiles out and its tests run zero).
    sparq-reason-ql)
      cargo_args+=(--features experimental,ql-consistency)
      features+=("experimental" "ql-consistency") ;;
    # [OPUS-4.8] sq-6vshe.4: seam 1 of the sparq-engine facade split — the RDF writer matrix
    # (Turtle/TriG/N-Quads/JSON-LD) peeled into this internal sub-crate. Its WHOLE surface is
    # behind DEFAULT-OFF `serialize-rdf` (mirroring the gating it had inside sparq-engine), so a
    # default-feature `cargo llvm-cov -p sparq-engine-serialize` builds an EMPTY crate — measure
    # it WITH `serialize-rdf,streaming-serialization` (the maximal surface, incl. the streaming
    # writers + their tests), exactly as sparq-substrate/-fedplan above name their whole-surface
    # features. Brings the moved writer + its ~2.9k test LOC under the line-coverage ratchet.
    sparq-engine-serialize)
      cargo_args+=(--features serialize-rdf,streaming-serialization)
      features+=("serialize-rdf" "streaming-serialization") ;;
    # [OPUS-4.8] sq-6vshe.4: seam A2 of the sparq-engine facade split — the SPARQL 1.1 federated-
    # SERVICE client peeled into this internal sub-crate. Its WHOLE surface is behind DEFAULT-OFF
    # `service` (mirroring the gating it had inside sparq-engine), so a default-feature
    # `cargo llvm-cov -p sparq-engine-service` builds an EMPTY crate — measure it WITH `service`
    # (the maximal surface: HTTP transport + SRJ/SRX parse + bound-join + SSRF egress policy + their
    # ~1.3k test LOC), exactly as the sibling extractions above name their whole-surface features.
    sparq-engine-service)
      cargo_args+=(--features service)
      features+=("service") ;;
  esac

  local start end rc=0 json="$WORK/$crate.json"
  start=$(date +%s)
  # [OPUS-4.8] sq-x4jy: retry a per-crate measurement up to MEASURE_ATTEMPTS times, but
  # ONLY on the aborted-binary signature: a non-zero rc (e.g. rc=101 when a test binary
  # aborts / "was never executed") OR a missing/empty JSON output (a partial profraw that
  # llvm-cov couldn't summarise). A run that exits 0 with a valid, non-empty JSON is taken
  # as a REAL result and is NOT retried — retrying a valid-but-low number would MASK a
  # genuine coverage regression, so this loop deliberately does not.
  #
  # TWO DISTINCT FLAKES, two distinct mitigations (do not conflate them):
  #   1. env-var test race  -> sparq-core build+test/coverage ABORTED a binary (rc=101 /
  #      empty JSON). Root-fixed in ea0ca3e (the dict-spill / service-allow tests no longer
  #      mutate the process-global environment); this rc!=0/empty-JSON retry is the
  #      belt-and-braces for any residual runner flake of THAT aborted-binary shape.
  #   2. profraw-merge race -> sparq-core reported ~71% (between dict-spill-only ~65% and
  #      the full ~91%) as a VALID rc=0 number, so it is NOT — and must NOT be — caught by
  #      this retry. It is fixed by SERIALISING each binary's tests (`--test-threads=1`,
  #      set in `test_args` above), which removes the intra-binary profile-counter race; a
  #      CI-level re-MEASURE backstop (.github/workflows/ci.yml coverage job) re-runs the
  #      whole suite once if the gate fails, which is safe because it re-measures rather
  #      than accepting a low number — a real regression fails BOTH passes.
  local attempt=0 attempts="${MEASURE_ATTEMPTS:-2}"
  # Validate MEASURE_ATTEMPTS is a positive integer; fall back to 2 otherwise (a
  # non-numeric value would break the `-ge` test below and could loop surprisingly).
  case "$attempts" in
    ''|*[!0-9]*) attempts=2 ;;
  esac
  [ "$attempts" -ge 1 ] 2>/dev/null || attempts=2
  while :; do
    attempt=$((attempt + 1))
    rc=0
    # Both invocation forms pass the libtest args after `--` (cargo llvm-cov accepts
    # `[SUBCOMMAND] [OPTIONS] [-- <args>...]`), so `--test-threads=1` (and any `--skip`)
    # apply whether or not a `test` subcommand is used. [OPUS-4.8] sq-x4jy
    if [ -n "$subcmd" ]; then
      cargo llvm-cov "$subcmd" "${cargo_args[@]}" --summary-only --json \
        --output-path "$json" -- "${test_args[@]}" >/dev/null 2>"$WORK/$crate.err" || rc=$?
    else
      cargo llvm-cov "${cargo_args[@]}" --summary-only --json \
        --output-path "$json" -- "${test_args[@]}" >/dev/null 2>"$WORK/$crate.err" || rc=$?
    fi
    if { [ "$rc" -eq 0 ] && [ -s "$json" ]; } || [ "$attempt" -ge "$attempts" ]; then
      break
    fi
    echo "  .. $crate measure attempt $attempt failed (rc=$rc) — retrying (transient binary race?)"
  done
  end=$(date +%s)

  if [ "$rc" -ne 0 ] || [ ! -s "$json" ]; then
    echo "  !! $crate FAILED to measure (rc=$rc) after $attempt attempt(s) — see error tail:"
    tail -8 "$WORK/$crate.err" | sed 's/^/     /'
    ROWS+=("$(python3 - "$crate" "$((end-start))" <<'PY'
import json,sys
print(json.dumps({"crate":sys.argv[1],"seconds":int(sys.argv[2]),"measured":False}))
PY
)")
    return
  fi

  local feats_json skips_json
  feats_json=$(printf '%s\n' "${features[@]:-}" | python3 -c 'import sys,json;print(json.dumps([l for l in sys.stdin.read().split("\n") if l]))')
  skips_json=$(printf '%s\n' "${skips[@]:-}"     | python3 -c 'import sys,json;print(json.dumps([l for l in sys.stdin.read().split("\n") if l]))')

  local row
  row=$(python3 - "$crate" "$json" "$((end-start))" "$feats_json" "$skips_json" <<'PY'
import json,sys
crate,path,secs,feats,skips=sys.argv[1],sys.argv[2],int(sys.argv[3]),json.loads(sys.argv[4]),json.loads(sys.argv[5])
d=json.load(open(path))
t=d["data"][0]["totals"]["lines"]   # llvm-cov shape: {count, covered, percent}
print(json.dumps({"crate":crate,"lines_pct":round(t["percent"],2),
  "lines_covered":t["covered"],"lines_total":t["count"],
  "seconds":secs,"features":feats,"skipped_tests":skips,"measured":True}))
PY
)
  ROWS+=("$row")
  printf "  %-20s lines=%6s%%  %4ss%s%s\n" "$crate" \
    "$(echo "$row" | python3 -c 'import sys,json;print(json.load(sys.stdin)["lines_pct"])')" \
    "$((end-start))" \
    "$([ ${#features[@]} -gt 0 ] && echo "  feat=${features[*]}")" \
    "$([ ${#skips[@]} -gt 0 ] && echo "  skip=${skips[*]}")"
}

# [OPUS-4.8] (sq-v411r) Start from a CLEAN instrumented build so the per-crate line
# DENOMINATOR is deterministic. cargo-llvm-cov reuses `target/llvm-cov-target` object
# files between invocations; if a RESTORED cache (Swatinem/rust-cache) or an earlier
# wider-feature build left a crate's object compiled with extra OPT-IN features baked
# in, the next default-feature `--package X` REUSES that object and instruments the
# feature-gated regions the default-feature tests never exercise — inflating `count`
# and crashing the %. This is exactly what bit PR #1257: sparq-engine measured 65% over
# 14184 instrumented lines on a cache-restored runner while a clean build measures 88%
# over 10478 (the 9233 COVERED lines were identical — only the denominator was poisoned).
# `clean` here forces every per-crate build to instrument only the regions its own
# requested feature set compiles, matching the `measure_merged` path (which already
# cleans). It does NOT mask a regression — it re-compiles + re-measures the SAME tests.
cargo llvm-cov clean --workspace
echo "==> Measuring per-crate line coverage (tier=$TIER)…"
for c in ${CRATES[@]+"${CRATES[@]}"}; do measure "$c"; done
TOTAL_END=$(date +%s)

# ---- assemble the summary JSON ---------------------------------------------
# NB: write rows to a file and pass its path as argv — do NOT pipe rows into a
# `python3 - <<'PY'` heredoc (the heredoc captures stdin, so a pipe would be lost).
ROWS_FILE="$WORK/rows.ndjson"
: > "$ROWS_FILE"
[ ${#ROWS[@]} -gt 0 ] && printf '%s\n' "${ROWS[@]}" > "$ROWS_FILE"
# [SONNET-4.6] sq-3dr4t: pass the cone filter + the crates it SKIPPED so the summary
# records the skip explicitly (AGENTS.md "no silent truncation"): coverage-gate.py prints
# them as INHERITED rather than the indistinguishable MISSING, and cone_coverage.py
# --mode report renders them as inherited rows.
python3 - "$OUT" "$TIER" "$TOOLCHAIN" "$((TOTAL_END-TOTAL_START))" "$NIGHTLY_ONLY_NOTE" \
  "$ROWS_FILE" "${COVERAGE_CONE:-}" "${CONE_INHERITED[*]:-}" <<'PY'
import json,sys,datetime
out,tier,toolchain,total,note,rows_file,cone_filter,cone_inherited=sys.argv[1:9]
total=int(total)
crates={}
for line in open(rows_file):
    line=line.strip()
    if not line: continue
    r=json.loads(line); crates[r.pop("crate")]=r
doc={"generated":datetime.datetime.now(datetime.timezone.utc).isoformat(),"tier":tier,
     "toolchain":toolchain,"total_seconds":total,"note":note,"crates":crates}
inherited=sorted(cone_inherited.split())
if cone_filter.split() and inherited:
    # Only when the filter actually NARROWED this run — an unset/empty COVERAGE_CONE, or a
    # cone that happened to contain the whole selection, leaves no "cone" block at all, so
    # the summary shape is byte-identical to the pre-flip one on every unfiltered path.
    doc["cone"]={"filter":sorted(set(cone_filter.split())),
                 "measured":sorted(crates),"inherited":inherited}
json.dump(doc,open(out,"w"),indent=2,sort_keys=True); open(out,"a").write("\n")
print(f"\n==> wrote {out}  ({len(crates)} crates, {total}s total"
      + (f", {len(inherited)} inherited via the changed cone" if inherited else "") + ")")
PY
