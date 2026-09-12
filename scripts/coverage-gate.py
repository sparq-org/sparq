#!/usr/bin/env python3
# [OPUS-4.8] Per-crate line-coverage RATCHET gate (sq-hbg7).
#
# Mirrors the conformance / perf ratchet idiom (a committed FLOOR that only ever
# RISES, reviewed in diffs). Two modes:
#
#   --seed   <summary.json>   regenerate bench/coverage-floor.json from a measured
#                             summary (scripts/coverage.sh output). Each crate's floor
#                             is set to floor(measured) - MARGIN (min 0) so ordinary
#                             run-to-run noise never trips the gate. Crates known to
#                             report a misleading number (sparq-cli subprocess artifact,
#                             sparq-conformance test-driver, sparq-gpu device-gated) get
#                             floor 0 + an annotated note (still presence-gated).
#   --check  <summary.json>   FAIL (exit 1) if any crate in the floor regressed below
#                             its floor in the measured summary. A crate present in the
#                             floor but MISSING from the summary is reported (only fatal
#                             with --require-all, e.g. the per-commit tier need not
#                             measure nightly-only crates).
#
#   --check-robust <summary>  [OPUS-4.8] sq-x4jy: the ROBUST gate driver — MEASURE-AND-
#                             TAKE-MAX across up to K independent measurements, then check.
#                             This replaces the old "re-measure the WHOLE suite once on
#                             failure" CI backstop, which was structurally insufficient
#                             (PR #62: pass-1 sparq-core flaked low -> re-measure the WHOLE
#                             suite -> sparq-engine, UNCHANGED by the PR, then flaked 0.28%
#                             low -> job failed). See the MAX-REMEASURE rationale below.
#
#   --merge-max <a> <b> [-o]  [OPUS-4.8] sq-x4jy: pure helper — merge two summaries by
#                             per-crate MAX(lines_pct), writing the result (used by an
#                             external shell loop, and unit-tested directly).
#
#   --check-monotonic         [OPUS-4.8] sq-neq8: RATCHET-DIRECTION gate. Diff THIS branch's
#                             bench/coverage-floor.json floors against the BASE (default
#                             `origin/main:bench/coverage-floor.json`, overridable via
#                             --base-ref / --base-file) and FAIL (exit 1) if a PR LOWERS any
#                             crate's floor. --check only compares measured-vs-floor, so a
#                             floor DECREASE in a PR is otherwise INVISIBLE — exactly how
#                             #661 silently dropped sparq-serve 92->83 (caught by a human,
#                             not CI). A deliberate, reviewed regression must pass --allow-
#                             lower. New crates and RAISED floors always pass.
#
#   --check-untracked         [OPUS-5] #6121: the UNTRACKED-CRATE arm (no compile, no
#                             summary). FAIL (exit 1) if a crate under crates/ is neither a
#                             row in the floor file nor DECLARED in NO_FLOOR_CRATES /
#                             AWAITING_SEED below — the drift the floor file had accumulated
#                             (28 crates behind crates/), which no other mode can see.
#
#   --check-advance-allowed   [SONNET-4.6] sq-6vshe.17: the RATCHET-ADVANCE PAUSE. The
#                             coverage MEASUREMENT is demoted off the merge_group blocking
#                             path (PR + push-to-main still measure), so `main` is an
#                             enforcement point — and while the post-merge coverage alarm
#                             issue is OPEN, RAISING a floor is unverifiable book-keeping
#                             (the numbers a raise cites are the numbers in doubt). FAIL
#                             (exit 1) iff this branch raises a floor AND that alarm is
#                             open. Fail-OPEN on any probe error, and it NEVER blocks the
#                             recovery path (lowering under --allow-lower).
#
# THE `seed_pending` FLAG (sq-iwf3c) [SONNET-4.6]
# -----------------------------------------------
# A floor entry may carry `"seed_pending": true`. It means: THIS FLOOR WAS NOT MEASURED
# OVER THE SURFACE THE GATE NOW MEASURES — it was carried forward across a change that
# WIDENED the crate's denominator (e.g. scripts/coverage.sh starting to compile in an
# opt-in feature's module), by an author who had no way to run llvm-cov. A carried-forward
# floor is not merely unproven: if the wider surface actually measures HIGHER, the entry
# silently installs a floor LOOSER than the seed procedure (floor(measured) - MARGIN)
# would give, and the ratchet quietly loses ground.
# `--check` therefore ENFORCES the seed procedure for such an entry instead of trusting a
# note: it recomputes floor(measured) - MARGIN and FAILS if that exceeds the committed
# floor, printing the exact number to write. If it does NOT exceed it, the measurement
# CONFIRMS the carried-forward floor and the gate prints that as evidence and passes.
# Either way CI — not the PR author's promise — is what settles it, and `--seed` clears
# the flag automatically because it rebuilds each entry from the measurement.
#
# THE MAX-REMEASURE PRINCIPLE (sq-x4jy) [OPUS-4.8]
# ------------------------------------------------
# llvm-cov instrumentation only ever UNDERCOUNTS: when a test process aborts/OOMs or a
# `.profraw` fails to merge, that contribution is LOST, pulling the number DOWN. It can
# NEVER spuriously overcount. So the principled "true" coverage of a crate is the MAXIMUM
# across repeated independent measurements. The robust gate therefore:
#   1. measures all crates once (the summary passed in),
#   2. finds the crates BELOW floor,
#   3. RE-MEASURES ONLY those crates (not the whole suite — re-measuring everything is
#      exactly why a *second*, PR-unrelated crate flaked in #62, and is needlessly slow),
#      keeping the per-crate MAX seen,
#   4. repeats (2)-(3) up to K total measurements per crate,
#   5. FAILS only if a crate is STILL below floor after K independent measurements — that
#      is a genuine regression. Otherwise PASS, and the final summary carries the per-crate
#      MAX (the most accurate number).
# This is SAFE — it never ACCEPTS a low number; it re-measures and keeps the best. A real
# regression is reproducible and fails ALL K measurements; a transient undercount is not.
#
# The floor only RISES: --seed will NOT lower an existing floor unless --allow-lower is
# passed (a deliberate, reviewed regression — e.g. a refactor that legitimately drops a
# crate's measurable surface). Seeding NEW crates / RAISING floors is automatic.
#
# WHY a margin: line% drifts a little with toolchain bumps / nondeterministic test
# ordering. MARGIN=2 (points) keeps the gate meaningful without flaking. The floor is
# the ratchet of record — raise it deliberately as coverage grows.
import argparse, json, math, os, subprocess, sys, tempfile

MARGIN = 2  # percentage points of slack below the measured value

# [OPUS-4.8] sq-x4jy: total independent measurements per crate in the robust gate
# (1 initial + up to K-1 targeted re-measures). K=3 caps the worst-case wall-clock of
# the targeted re-measure loop while giving any transiently-undercounted crate two extra
# chances to record its true (higher) number. Keep small: each round shells coverage.sh.
DEFAULT_K = 3

# Crates whose llvm-cov line% is NOT a meaningful gate (floor pinned to 0). They stay
# in the floor file (and the presence gate still guards their tests existing).
ARTIFACT_ZERO = {
    "sparq-cli": "subprocess artifact: tests spawn the COMPILED binary (assert_cmd / "
                 "CARGO_BIN_EXE), not the instrumented profile — line% is ~0 by "
                 "construction, not a real gap. Presence-gated instead.",
    "sparq-conformance": "test-driver crate: the W3C suites run as the BINARIES "
                 "(cargo run), so `cargo llvm-cov --package` (which runs cargo test) "
                 "sees only a few unit tests. Low % is expected; floor 0.",
    "sparq-gpu": "GPU kernels need a device; CPU-only line coverage is low by design.",
}
# Crates measured only in the nightly tier (heavy tests). The per-commit --check must
# NOT fail just because they are absent from a per-commit summary.
NIGHTLY_NOTE = {
    "sparq-vectors": "per-commit floor is measured with the two "
                     "*_recall_at_10_vs_brute_force_on_50k tests SKIPPED; the nightly "
                     "tier measures the full set.",
}

# [OPUS-4.8] sq-bjct: crates whose NIGHTLY measurement MERGES the W3C conformance
# binaries (sparq-conformance / sparq-inference-conformance) into the per-crate
# report (scripts/coverage.sh measure_merged). Those crates carry a separate, HIGHER
# `nightly_floor` gated only by the nightly tier; their base `floor` stays the cheaper
# test-only number gated per-commit. Seeding from a nightly summary RAISES nightly_floor;
# seeding from a per-commit summary RAISES the base floor — neither touches the other.
CONFORMANCE_MERGE_CRATES = {"sparq-core", "sparq-engine"}
CONFORMANCE_MERGE_NOTE = (
    "[OPUS-4.8] sq-bjct: nightly_floor gates the NIGHTLY tier, whose measurement MERGES "
    "the W3C SPARQL + inference conformance BINARIES (they run as `cargo run`, not "
    "`cargo test`) into this crate's llvm-cov report — a higher number than the test-only "
    "per-commit `floor`. See scripts/coverage.sh measure_merged + the coverage-nightly job."
)

# ---------------------------------------------------------------------------------------
# [OPUS-5] #6121: the UNTRACKED-CRATE arm — this gate's own defence against DRIFT.
#
# Every mode above reasons only about crates that ALREADY have a row in the floor file. A
# crate that never got one is invisible to all of them: `--check` has nothing to compare,
# `--check-monotonic` sees no base row to lower, and coverage.sh's `--check-shards` guard
# polices PER_COMMIT_CRATES against SHARD_GROUPS — not `crates/` against this file. So the
# floor file silently drifted 28 crates behind the tree (#6121), the same hole
# bench/coverage-presence.json had drifted into (#5140).
#
# `--check-untracked` closes it: every directory under crates/ carrying a Cargo.toml must be
# either (a) a row in bench/coverage-floor.json, or (b) DECLARED below with a reason. An
# UNDECLARED crate FAILS, so the next crate added to the workspace cannot repeat this.
# The two registries below are NOT the same kind of thing:
#
#   NO_FLOOR_CRATES  permanent, principled exemptions — a line-% floor would measure nothing
#                    meaningful for them. They stay guarded by the test-PRESENCE gate
#                    (bench/coverage-presence.json / scripts/coverage-presence.py, whose
#                    NO_TEST_CRATES records the same two crates for the same reasons).
#   AWAITING_SEED    DEBT, not exemption. These crates SHOULD carry a floor and do not,
#                    because seeding one is not a no-compile change: it needs a measured
#                    `cargo llvm-cov` run per crate AND a scripts/coverage.sh
#                    PER_COMMIT_CRATES + SHARD_GROUPS slot (without the slot, `coverage.sh
#                    --check-shards` reds). So the debt is written down HERE — enumerated,
#                    never silent — and the gate makes the list SHRINK-ONLY: once a crate
#                    gets a floor row its declaration is STALE and fails until deleted.
#
# Adding a crate to AWAITING_SEED instead of seeding it is a LAST resort, not the default
# path: it gates nothing. Prefer measuring the crate and giving it a real floor.
CRATES_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "crates"))

NO_FLOOR_CRATES = {
    "sparq-bench": "perf-harness crate (criterion benchmarks, not a measurable library "
                   "surface); presence-gated instead — mirrors coverage-presence.py "
                   "NO_TEST_CRATES.",
    "sparq-py": "pyo3 bindings; exercised by pytest (crates/sparq-py/tests), not by Rust "
                "#[test]s, so a native llvm-cov line% is the same misleading artifact as "
                "sparq-cli's. Presence-gated instead.",
}

# The three shapes of "not yet seeded", so a reader can tell how much work each row is.
_SEED_NEEDS_FEATURES = (
    "seed must choose a feature set first: this crate has DEFAULT-OFF opt-in features, so a "
    "plain `cargo llvm-cov -p` instruments only the default build and can report a "
    "non-representative number (cf. the measure() `case` arms in scripts/coverage.sh, which "
    "name the feature set for sparq-core / sparq-policy / sparq-substrate and friends)"
)
_SEED_DEFAULT_SURFACE = (
    "whole surface is default-compiled (no default-off features to select), so seeding needs "
    "only a PER_COMMIT_CRATES + SHARD_GROUPS slot and one measured run — no measure() arm"
)
_SEED_EMPTY_SEAM = (
    "reserved seam stub: src/lib.rs is a doc comment with no executable code, so a line-% "
    "floor would measure nothing until the seam is actually implemented"
)

AWAITING_SEED = {
    # --- opt-in features are default-off; the seed must name the measured feature set.
    "sparq-arrow": _SEED_NEEDS_FEATURES,
    "sparq-fedplan-mpc": _SEED_NEEDS_FEATURES,
    "sparq-http3": _SEED_NEEDS_FEATURES,
    "sparq-kb": _SEED_NEEDS_FEATURES,
    "sparq-lws-core": _SEED_NEEDS_FEATURES,
    "sparq-lws-wasm": _SEED_NEEDS_FEATURES,
    "sparq-mcp": _SEED_NEEDS_FEATURES,
    "sparq-metamorph": _SEED_NEEDS_FEATURES,
    "sparq-reason-diff": _SEED_NEEDS_FEATURES,
    "sparq-reason-dl": _SEED_NEEDS_FEATURES,
    "sparq-terse": _SEED_NEEDS_FEATURES,
    "sparq-trust": _SEED_NEEDS_FEATURES,
    "sparq-vc": _SEED_NEEDS_FEATURES,
    "sparq-wac-oracle": _SEED_NEEDS_FEATURES,
    "sparq-wrapper": _SEED_NEEDS_FEATURES,
    # --- whole surface default-compiled; only a shard slot + a measured run are missing.
    "sparq-acbench": _SEED_DEFAULT_SURFACE,
    "sparq-conformance-floors": _SEED_DEFAULT_SURFACE,
    "sparq-crdt": _SEED_DEFAULT_SURFACE,
    "sparq-difftest": _SEED_DEFAULT_SURFACE,
    "sparq-e2ee-ng": _SEED_DEFAULT_SURFACE,
    "sparq-jsonld-registry": _SEED_DEFAULT_SURFACE,
    "sparq-secprop-vocab": _SEED_DEFAULT_SURFACE,
    "sparq-shaclc": _SEED_DEFAULT_SURFACE,
    # --- reserved seam stubs with no executable code yet.
    "sparq-wrapper-gen": _SEED_EMPTY_SEAM,
    "sparq-wrapper-integration": _SEED_EMPTY_SEAM,
    "sparq-wrapper-shacl": _SEED_EMPTY_SEAM,
}


def load(p):
    with open(p) as f:
        return json.load(f)

def seed(summary_path, floor_path, allow_lower):
    s = load(summary_path)
    tier = s.get("tier")                       # [OPUS-4.8] sq-bjct
    existing = load(floor_path)["crates"] if os.path.exists(floor_path) else {}
    out = {}
    raised, kept, new, lowered = [], [], [], []
    # A crate's MERGED nightly measurement seeds `nightly_floor`, NOT the base floor:
    # only when seeding FROM a nightly summary AND the row actually merged the binaries.
    def is_merged_row(crate, row):
        return (crate in CONFORMANCE_MERGE_CRATES and tier and tier != "per-commit"
                and "conformance-merge" in (row.get("features") or []))
    for crate, row in sorted(s["crates"].items()):
        prev_entry = dict(existing.get(crate, {}))   # carry forward the WHOLE entry
        if not row.get("measured", False):
            # carry an existing floor forward; never invent one for an unmeasured crate
            if crate in existing:
                out[crate] = prev_entry; kept.append(crate)
            continue
        if crate in ARTIFACT_ZERO:
            entry = {"floor": 0, "note": ARTIFACT_ZERO[crate]}
            # preserve a previously-seeded nightly_floor (the artifact note is base-only)
            if prev_entry.get("nightly_floor") is not None:
                entry["nightly_floor"] = prev_entry["nightly_floor"]
            out[crate] = entry
            continue
        measured = row["lines_pct"]
        proposed = max(0, math.floor(measured) - MARGIN)
        if is_merged_row(crate, row):
            # Ratchet the NIGHTLY floor up; leave the base `floor` (per-commit) untouched.
            entry = prev_entry if prev_entry else {"floor": 0}
            prev = entry.get("nightly_floor")
            label = f"{crate}.nightly_floor"
            if prev is None:
                entry["nightly_floor"] = proposed; new.append(f"{label}={proposed}")
            elif proposed > prev:
                entry["nightly_floor"] = proposed; raised.append(f"{label} {prev}->{proposed}")
            elif proposed < prev and allow_lower:
                entry["nightly_floor"] = proposed; lowered.append(f"{label} {prev}->{proposed}")
            else:
                entry["nightly_floor"] = prev; kept.append(label)  # ratchet
            entry.setdefault("nightly_note", CONFORMANCE_MERGE_NOTE)
            out[crate] = entry
            continue
        prev = existing.get(crate, {}).get("floor")
        note = NIGHTLY_NOTE.get(crate)
        if prev is None:
            chosen = proposed; new.append(f"{crate}={chosen}")
        elif proposed > prev:
            chosen = proposed; raised.append(f"{crate} {prev}->{chosen}")
        elif proposed < prev and allow_lower:
            chosen = proposed; lowered.append(f"{crate} {prev}->{chosen}")
        else:
            chosen = prev; kept.append(crate)  # ratchet: never auto-lower
        # Rebuilt from the measurement, so a `seed_pending` flag on the previous entry is
        # DELIBERATELY dropped here: this floor has now been seeded from a real run over
        # the current denominator, which is exactly what the flag was waiting for.
        # [SONNET-4.6] sq-iwf3c. (The nightly/merged branch above keeps it — it seeds
        # `nightly_floor` only and leaves the base floor unproven.)
        entry = {"floor": chosen}
        if note: entry["note"] = note
        # preserve a previously-seeded nightly_floor when seeding from a per-commit summary
        if prev_entry.get("nightly_floor") is not None:
            entry["nightly_floor"] = prev_entry["nightly_floor"]
            if prev_entry.get("nightly_note"):
                entry["nightly_note"] = prev_entry["nightly_note"]
        out[crate] = entry
    doc = {
        "_comment": [
            "[OPUS-4.8] COVERAGE RATCHET (sq-hbg7) — committed per-crate line-coverage "
            "FLOOR, reviewed in diffs exactly like the conformance ratchet counts in "
            ".github/workflows/ci.yml. CI runs scripts/coverage-gate.py --check-robust "
            "(sq-x4jy): it re-measures ONLY the sub-floor crates up to K=3 times and keeps "
            "the per-crate MAX (llvm-cov only undercounts), FAILING only if a crate is "
            "STILL below its floor after K independent measurements — a genuine regression.",
            f"Floor = floor(measured) - {MARGIN} (min 0): a small margin so run-to-run / "
            "toolchain noise never trips the gate. The floor only ever RISES "
            "(--seed will not lower it without --allow-lower).",
            "Tiering: the FAST/MEDIUM crates are measured + gated PER-COMMIT; the heavy "
            "sparq-vectors 50k recall/diskann tests are EXCLUDED per-commit (measured in "
            "the NIGHTLY tier — see the .github/workflows/ci.yml coverage-nightly job and "
            "the per-crate notes below). No crate is silently dropped.",
            "Floors with floor:0 are crates whose llvm-cov line% is a known measurement "
            "artifact (see each note) — they are guarded by the test-PRESENCE gate "
            "(bench/coverage-presence.json) instead of the % gate.",
            "[OPUS-4.8] sq-bjct: a crate with a `nightly_floor` (sparq-core / sparq-engine) "
            "is gated TWICE: per-commit on the test-only `floor`, and nightly on the higher "
            "`nightly_floor` — the nightly tier MERGES the W3C SPARQL + inference conformance "
            "BINARIES (which run as `cargo run`, not `cargo test`) into that crate's llvm-cov "
            "report. Seeding from a nightly summary raises `nightly_floor`; seeding from a "
            "per-commit summary raises the base `floor`. Neither seed touches the other.",
            "[OPUS-5] #6121: this file gates only the crates it LISTS — every other mode "
            "compares floors to a measurement or to a base file, so a crate with no row at "
            "all was invisible to all of them (that is how this file drifted 28 crates "
            "behind crates/). `coverage-gate.py --check-untracked`, run by the no-compile "
            "`coverage-floors` job, now FAILS unless every crate under crates/ is either a "
            "row here or DECLARED in coverage-gate.py's NO_FLOOR_CRATES (permanent "
            "exemptions) / AWAITING_SEED (written-down DEBT: crates that SHOULD carry a "
            "floor but cannot be seeded without a measured run plus a PER_COMMIT_CRATES + "
            "SHARD_GROUPS slot in scripts/coverage.sh). An AWAITING_SEED crate gates "
            "NOTHING here; seeding it is the fix, and the gate keeps that list shrink-only.",
            "[SONNET-4.6] sq-iwf3c: an entry with `seed_pending: true` carries a floor "
            "that was NOT measured over the surface the gate now measures (it was carried "
            "forward across a change that WIDENED the crate's denominator). --check "
            "recomputes floor(measured) - MARGIN for such an entry and FAILS if that "
            "exceeds the committed floor — so a carried-forward floor can never be looser "
            "than the measurement supports. --seed clears the flag.",
            "Regenerate after a deliberate coverage rise: scripts/coverage.sh && "
            "scripts/coverage-gate.py --seed target/coverage/coverage-summary.json "
            "(per-commit -> base floor); COVERAGE_TIER=nightly scripts/coverage.sh && "
            "scripts/coverage-gate.py --seed ... (nightly -> nightly_floor).",
        ],
        "margin_points": MARGIN,
        "crates": out,
    }
    with open(floor_path, "w") as f:
        json.dump(doc, f, indent=2, sort_keys=True); f.write("\n")
    print(f"seeded {floor_path}: {len(out)} crates")
    if new:     print("  NEW:    " + ", ".join(new))
    if raised:  print("  RAISED: " + ", ".join(raised))
    if lowered: print("  LOWERED (--allow-lower): " + ", ".join(lowered))
    if kept:    print(f"  kept:   {len(kept)} unchanged")
    return 0

def check(summary_path, floor_path, require_all):
    # [OPUS-4.8] sq-039g: distinguish MISSING (a crate entirely ABSENT from this tier's
    # summary — legitimately not run in this tier, e.g. a nightly-only crate in a
    # per-commit summary; fatal only with --require-all) from UNMEASURED (a crate that
    # WAS attempted in this tier but whose coverage step ERRORED — coverage.sh records a
    # row with "measured": false, e.g. sparq-conformance exit 2 when fixtures are absent,
    # or — the dangerous case — sparq-core/sparq-engine failing to measure because a
    # fixture-dependent test aborted). An unmeasured crate WITH A NON-ZERO (effective)
    # FLOOR is ALWAYS fatal: it is supposed to be gated on its floor, and a failed
    # measurement would otherwise SILENTLY un-gate it. That was the bug — a measure-
    # failure was lumped into `missing`, so it was caught ONLY under --require-all, which
    # the nightly/per-commit --check-robust invocations do NOT pass. An unmeasured crate
    # with an effective floor of 0 (the ARTIFACT_ZERO crates) is not %-gated anyway, so
    # its measure-failure is reported but not fatal (the test-presence gate guards it).
    #
    # [SONNET-4.6] sq-3dr4t: a third shape joins those two — INHERITED. Under enforced
    # changed-cone coverage (COVERAGE_CONE, see scripts/coverage.sh) a crate outside the
    # PR's reverse-dep closure is deliberately not measured, and coverage.sh records it in
    # the summary's `cone.inherited` list. Such a crate is a MISSING crate (same
    # non-fatal-without---require-all semantics — its floor verdict is inherited from
    # main's last full run) but it is reported under its own label so an INTENDED skip is
    # never indistinguishable from a crate that silently fell out of this tier.
    s = load(summary_path); floors = load(floor_path)["crates"]
    measured = s["crates"]
    tier = s.get("tier")            # [OPUS-4.8] sq-bjct: tier-aware nightly_floor
    cone_inherited = set((s.get("cone") or {}).get("inherited") or [])
    fails, missing, unmeasured, oks = [], [], [], []
    # [SONNET-4.6] sq-iwf3c: (crate, measured, committed floor, seed-procedure floor) for
    # each `seed_pending` entry that this tier actually measured — see the header.
    pending = []
    for crate, fentry in sorted(floors.items()):
        floor = effective_floor(fentry, tier)
        row = measured.get(crate)
        if row is None:
            missing.append((crate, floor)); continue
        if not row.get("measured", False):
            unmeasured.append((crate, floor)); continue
        val = row["lines_pct"]
        if val + 1e-9 < floor:
            # A sub-floor crate is reported as the ordinary floor breach it is, with no
            # seed line: its seed cannot be stale anyway, since val < floor implies
            # floor(val) - MARGIN < floor.
            fails.append((crate, val, floor))
        else:
            oks.append((crate, val, floor))
            if isinstance(fentry, dict) and fentry.get("seed_pending"):
                pending.append((crate, val, floor, max(0, math.floor(val) - MARGIN)))
    # A crate that failed to MEASURE but carries a real (>0) effective floor is a hard
    # failure regardless of --require-all; a floor-0 (artifact) crate is not %-gated.
    unmeasured_gated = [(c, f) for c, f in unmeasured if f > 0]
    for crate, val, floor in oks:
        print(f"  ok   {crate:<20} {val:6.2f}% >= floor {floor}")
    for crate, floor in missing:
        if crate in cone_inherited:
            print(f"  --   {crate:<20} INHERITED (outside the PR's changed cone — "
                  f"unchanged since main; floor {floor} verdict carried forward)")
        else:
            print(f"  --   {crate:<20} MISSING (not in this tier's summary)")
    for crate, floor in unmeasured:
        if floor > 0:
            print(f"  FAIL {crate:<20} UNMEASURED (measure step errored) "
                  f"— floor {floor} NOT enforced")
        else:
            print(f"  --   {crate:<20} UNMEASURED (measure step errored) "
                  f"— floor 0, not %-gated")
    for crate, val, floor in fails:
        print(f"  FAIL {crate:<20} {val:6.2f}% < floor {floor}")
    # [SONNET-4.6] sq-iwf3c: a carried-forward floor is settled HERE, by the measurement.
    stale_seeds = [t for t in pending if t[3] > t[2]]
    for crate, val, floor, proposed in pending:
        if proposed > floor:
            print(f"  FAIL {crate:<20} seed_pending: measured {val:.2f}% over the CURRENT "
                  f"denominator seeds floor {proposed}, but the carried-forward floor is "
                  f"{floor} — the committed floor is LOOSER than the measurement supports")
        else:
            print(f"  ok   {crate:<20} seed_pending CONFIRMED: measured {val:.2f}% seeds "
                  f"floor {proposed} <= committed floor {floor} — the carried-forward "
                  f"floor is valid over the current denominator; drop the "
                  f"\"seed_pending\" flag and record this measurement in the note")
    bad = bool(fails) or bool(unmeasured_gated) or bool(stale_seeds) \
        or (require_all and bool(missing))
    if stale_seeds:
        print(f"::error::{len(stale_seeds)} crate(s) carry \"seed_pending\" but measure "
              f"ABOVE their carried-forward floor: "
              + "; ".join(f"set bench/coverage-floor.json crates.{c}.floor = {p} "
                          f"(floor({v:.2f}) - {MARGIN}), drop \"seed_pending\", and record "
                          f"the measurement in the note"
                          for c, v, _f, p in stale_seeds))
    if fails:
        print(f"::error::coverage regressed below the floor for {len(fails)} crate(s)")
    if unmeasured_gated:
        print(f"::error::{len(unmeasured_gated)} crate(s) FAILED TO MEASURE but have a "
              f"non-zero floor (would otherwise be SILENTLY un-gated): "
              f"{', '.join(c for c, _ in unmeasured_gated)}")
    if require_all and missing:
        print(f"::error::{len(missing)} floor crate(s) absent from the summary "
              f"(--require-all): {', '.join(c for c, _ in missing)}")
    n_inherited = sum(1 for c, _ in missing if c in cone_inherited)
    print(f"\ncoverage gate: {len(oks)} ok / {len(fails)} fail / "
          f"{len(unmeasured)} unmeasured ({len(unmeasured_gated)} gated) / "
          f"{len(missing) - n_inherited} missing / {n_inherited} inherited (changed cone) / "
          f"{len(pending)} seed_pending ({len(stale_seeds)} stale)")
    return 1 if bad else 0

# --- pure aggregation primitives (unit-tested by --self-test) -----------------
# [OPUS-4.8] sq-x4jy: these are PURE functions over plain dicts so the robust
# max-remeasure logic can be exercised with synthetic measurement sequences WITHOUT
# reproducing the CI flake. The CI driver below is the only thing that does I/O.

def floor_of(fentry):
    """Floor value from a floor-file entry (a dict {"floor": N} or a bare number)."""
    return fentry["floor"] if isinstance(fentry, dict) else fentry

def effective_floor(fentry, tier):
    """[OPUS-4.8] sq-bjct: the floor that APPLIES for a given measurement tier.

    A crate whose nightly measurement MERGES the conformance binaries (sparq-core /
    sparq-engine) carries a HIGHER `nightly_floor` alongside the per-commit `floor`:
    the per-commit `coverage` job measures test-only (lower) coverage and gates on
    `floor`; the nightly `coverage-nightly` job measures the suite-MERGED (higher)
    coverage and gates on `nightly_floor`. Without this split, ratcheting the merge
    crates to their (higher) nightly number — which sq-bjct asks for — would make the
    cheaper, test-only per-commit job fail. Any tier other than "per-commit" uses the
    nightly floor when present; per-commit always uses the base floor."""
    base = floor_of(fentry)
    if tier and tier != "per-commit" and isinstance(fentry, dict) \
       and fentry.get("nightly_floor") is not None:
        return fentry["nightly_floor"]
    return base

def sub_floor_crates(summary, floors):
    """Pure: the set of crates that are MEASURED in `summary` AND below their floor.

    A crate missing / not-measured is NOT returned (re-measuring a crate that did not
    even run cannot help; the absence is handled by --check's MISSING/--require-all
    semantics). This is exactly the set the robust gate re-measures."""
    out = []
    measured = summary.get("crates", {})
    tier = summary.get("tier")      # [OPUS-4.8] sq-bjct: tier-aware nightly_floor
    for crate, fentry in floors.items():
        row = measured.get(crate)
        if row is None or not row.get("measured", False):
            continue
        if row["lines_pct"] + 1e-9 < effective_floor(fentry, tier):
            out.append(crate)
    return sorted(out)

def floor_regressions(base_floors, new_floors):
    """[OPUS-4.8] sq-neq8: PURE — return the list of (crate, base_floor, new_floor) tuples
    where the new floor file LOWERED a crate's floor relative to `base_floors`.

    Both args are the `crates` mapping of a floor file (crate -> {"floor": N} | N). A crate
    only in `new_floors` (a newly-added crate) is NOT a regression; a crate only in
    `base_floors` (dropped from the floor file) is reported separately by the caller — it is
    NOT a *floor* lowering but it IS a way to erode the ratchet, so the caller treats a drop
    as a regression too. Here we report only the floor DECREASES; equal/raised floors pass.

    [OPUS-4.8] sq-bjct: a crate's `nightly_floor` (the merged-suite gate for
    sparq-core / sparq-engine) is part of the same ratchet, so LOWERING it — or
    DROPPING it once present — is reported too (as a "<crate>.nightly_floor" row).
    A newly-ADDED nightly_floor is not a regression."""
    out = []
    for crate, bentry in sorted(base_floors.items()):
        if crate not in new_floors:
            continue  # dropped-crate handling is the caller's (it is reported, not silent)
        nentry = new_floors[crate]
        bf, nf = floor_of(bentry), floor_of(nentry)
        if nf < bf:
            out.append((crate, bf, nf))
        # nightly_floor ratchet (only when the base had one — a new one cannot regress).
        bnf = bentry.get("nightly_floor") if isinstance(bentry, dict) else None
        if bnf is not None:
            nnf = nentry.get("nightly_floor") if isinstance(nentry, dict) else None
            if nnf is None or nnf < bnf:
                out.append((f"{crate}.nightly_floor", bnf,
                            nnf if nnf is not None else "DROPPED"))
    return out


def dropped_crates(base_floors, new_floors):
    """[OPUS-4.8] sq-neq8: PURE — crates present in the BASE floor file but ABSENT from the
    new one. Removing a crate's floor row erodes the ratchet just as a floor decrease does
    (its coverage is no longer gated at all), so --check-monotonic treats it as a regression
    unless --allow-lower is given."""
    return sorted(c for c in base_floors if c not in new_floors)


def floor_advances(base_floors, new_floors):
    """[SONNET-4.6] sq-6vshe.17: PURE — the exact MIRROR of floor_regressions: the list of
    (label, base_floor, new_floor) rows where THIS branch RAISES a floor relative to
    `base_floors`. Used by --check-advance-allowed to answer "does this branch advance the
    ratchet?" without re-deriving the floor-file shape.

    Deliberately NARROW — only a RAISE of a floor the base already had counts:
      * a brand-NEW crate row is NOT an advance (adding a crate to the ratchet must stay
        possible while the post-merge alarm is open — it gates something previously
        un-gated, it does not move an existing bar);
      * a newly-ADDED `nightly_floor` is likewise not an advance;
      * a LOWERING is not an advance (floor_regressions owns that direction).
    Keeping it narrow keeps the advance BLOCK's blast radius to exactly the operation the
    demotion protocol pauses, and never blocks the RECOVERY path (a governed, reviewed
    re-baseline that lowers a floor under --allow-lower)."""
    out = []
    for crate, bentry in sorted(base_floors.items()):
        if crate not in new_floors:
            continue  # dropped, not raised — floor_regressions/dropped_crates own it
        nentry = new_floors[crate]
        bf, nf = floor_of(bentry), floor_of(nentry)
        if nf > bf:
            out.append((crate, bf, nf))
        bnf = bentry.get("nightly_floor") if isinstance(bentry, dict) else None
        if bnf is not None:
            nnf = nentry.get("nightly_floor") if isinstance(nentry, dict) else None
            if nnf is not None and nnf > bnf:
                out.append((f"{crate}.nightly_floor", bnf, nnf))
    return out


def advance_block_verdict(advances, alarm_open):
    """[SONNET-4.6] sq-6vshe.17: PURE — the exit code for --check-advance-allowed.

    The demotion protocol (research/ci-mergequeue-speedup-2026-07.md §3.4a) pauses
    RATCHET ADVANCES while the post-merge coverage measurement on `main` is RED: until
    main is green again the measured numbers a raise would be justified by are exactly the
    numbers in doubt, so raising a floor then is unverifiable book-keeping.

    BLOCK (exit 1) iff BOTH: (a) this branch raises a floor, AND (b) the alarm is KNOWN
    open. `alarm_open is None` means the probe could not run (no `gh`, no token, API
    error) => FAIL-OPEN (exit 0): an unavailable GitHub API must never block a PR whose
    only sin is raising a floor, and the floor DIRECTION gate (--check-monotonic) plus the
    measured --check are unaffected either way, so fail-open costs no soundness."""
    if not advances:
        return 0
    if alarm_open is True:
        return 1
    return 0


# The lane token the demoted-lane filer files the post-merge coverage alarm under
# (scripts/ci-file-demoted-lane-failure.py --lane); the alarm issue title is
# "[demoted-lane] lane=<lane>: full-form CI run failed".
COVERAGE_ALARM_LANE = "coverage-ratchet-main"


def open_alarm_issue_state(lane=COVERAGE_ALARM_LANE, log=print):
    """[SONNET-4.6] sq-6vshe.17: probe GitHub for an OPEN post-merge coverage alarm issue.

    Returns True (an open alarm exists), False (none), or None (the probe could not run —
    the caller FAILS OPEN). Matches the filer's own dedupe query + title contract exactly
    (scripts/ci-file-demoted-lane-failure.py find_open_issue), so the two cannot drift on
    which issue counts as "the alarm"."""
    marker = "[demoted-lane]"
    try:
        r = subprocess.run(
            ["gh", "issue", "list", "--state", "open",
             "--search", f'in:title "{marker} lane={lane}"',
             "--json", "number,title", "--limit", "10"],
            capture_output=True, text=True, timeout=120, check=True)
        items = json.loads(r.stdout or "[]")
    except (subprocess.SubprocessError, OSError, json.JSONDecodeError) as e:
        log(f"  note: could not probe the post-merge coverage alarm ({e}) — fail-OPEN")
        return None
    for item in items:
        title = item.get("title", "")
        if marker in title and f"lane={lane}" in title:
            log(f"  OPEN post-merge coverage alarm: issue #{item['number']} — {title}")
            return True
    return False


def check_advance_allowed(floor_path, base_ref, base_file, lane=COVERAGE_ALARM_LANE,
                         probe=None, log=print):
    """[SONNET-4.6] sq-6vshe.17: "no ratchet ADVANCE while post-merge coverage is RED".

    The companion to --check-monotonic in the same fast, no-compile `coverage-floors` job:
    monotonic forbids LOWERING a floor in any state; this forbids RAISING one while the
    demoted post-merge coverage lane's alarm is open. Returns an exit code."""
    new_floors = _load_floors_obj(load(floor_path))
    if base_file is not None:
        base_floors = _load_floors_obj(load(base_file))
        src = base_file
    else:
        base_floors = _base_floors_from_git(base_ref, floor_path, log=log)
        src = f"{base_ref}:{os.path.basename(floor_path)}"
    if base_floors is None:
        log("coverage advance gate: no base to compare — PASS")
        return 0

    advances = floor_advances(base_floors, new_floors)
    log(f"==> coverage advance gate: comparing floors vs {src}")
    if not advances:
        log("  this branch raises no committed floor — nothing for the advance pause to "
            "block. PASS")
        return 0
    for label, bf, nf in advances:
        log(f"  RAISED   {label:<20} floor {bf} -> {nf}")
    alarm_open = (probe or open_alarm_issue_state)(lane, log=log)
    rc = advance_block_verdict(advances, alarm_open)
    if rc == 0:
        state = "none open" if alarm_open is False else "probe unavailable (fail-OPEN)"
        log(f"  post-merge coverage alarm: {state} — the advance is allowed. PASS")
        return 0
    log(f"::error::coverage RATCHET ADVANCE paused: this branch raises "
        f"{len(advances)} floor(s) while the post-merge coverage lane on `main` is RED "
        f"(an open '[demoted-lane] lane={lane}' issue). The coverage MEASUREMENT is "
        f"demoted off the merge queue (sq-6vshe.17), so main is the enforcement point — "
        f"fix/close that alarm first, then re-push this raise. Nothing here lowers a "
        f"floor; drop the raise from this PR to unblock it.")
    log("coverage advance gate: FAIL")
    return 1


def merge_max(prev, new):
    """Pure: return a NEW summary that is `prev` with each crate's coverage replaced by
    the per-crate MAX(lines_pct) seen across `prev` and `new`.

    Undercount-only instrumentation (see header) means MAX is the most accurate estimate.
    The whole stat block (lines_covered/total/seconds/features) of whichever measurement
    had the higher lines_pct is kept, so the recorded row stays internally consistent.
    Crates only in `new` are added; crates only in `prev` are preserved. A non-measured
    `new` row never displaces a measured `prev` row (a failed re-measure must not lower
    the recorded max — that would defeat the point)."""
    out = {k: dict(v) for k, v in prev.get("crates", {}).items()}
    for crate, nrow in new.get("crates", {}).items():
        prow = out.get(crate)
        if prow is None:
            out[crate] = dict(nrow); continue
        # Only a MEASURED new row can win; and only if it is strictly higher.
        if not nrow.get("measured", False):
            continue
        if (not prow.get("measured", False)) or nrow["lines_pct"] > prow["lines_pct"]:
            out[crate] = dict(nrow)
    merged = dict(prev)
    merged["crates"] = out
    return merged

def robust_aggregate(measure_fn, initial, floors, k=DEFAULT_K, require_all=False, log=print):
    """Pure-ish ORCHESTRATION of the max-remeasure gate (the load-bearing logic), with
    all I/O injected via `measure_fn` so it is unit-testable with synthetic rounds.

    `measure_fn(crates)` -> a summary dict measuring exactly the named crates (a subset).
    `initial` is the round-1 whole-suite summary. Loops up to `k` TOTAL measurements,
    each round (a) finding crates still below floor, (b) re-measuring ONLY those, and
    (c) merging by per-crate MAX. Returns (exit_code, final_summary): 0 if every crate is
    at/above its floor on its best-of-k measurement, 1 if any crate is STILL below after k.
    Does NOT call sys.exit / subprocess — the CLI wrapper does the I/O + final --check."""
    summary = initial
    for rnd in range(2, k + 1):  # round 1 is `initial`; rounds 2..k are re-measures
        below = sub_floor_crates(summary, floors)
        if not below:
            log(f"  round {rnd-1}: all crates at/above floor — no re-measure needed")
            break
        log(f"  round {rnd-1}: {len(below)} crate(s) below floor -> re-measuring ONLY "
            f"{', '.join(below)} (round {rnd}/{k})")
        new = measure_fn(below)
        for crate in below:
            old = summary["crates"].get(crate, {}).get("lines_pct")
            nv = new.get("crates", {}).get(crate, {})
            nvp = nv.get("lines_pct") if nv.get("measured", False) else None
            log(f"    {crate:<20} prev={old}  remeasured="
                f"{nvp if nvp is not None else 'FAILED/absent'}  -> max="
                f"{max([x for x in (old, nvp) if x is not None], default=old)}")
        summary = merge_max(summary, new)
    # Final verdict on the per-crate MAX summary.
    still = sub_floor_crates(summary, floors)
    if still:
        log(f"  VERDICT: FAIL — {len(still)} crate(s) STILL below floor after {k} "
            f"measurement(s): {', '.join(still)} (genuine regression, not variance)")
        return 1, summary
    log(f"  VERDICT: PASS — every crate met its floor on its best of <= {k} measurements")
    return 0, summary

def check_robust(summary_path, floor_path, k, require_all,
                 out_path=None, extra_env=None):
    """[OPUS-4.8] sq-x4jy: CLI driver for the robust gate. Loads the round-1 summary +
    floors, then drives `robust_aggregate`, shelling out to scripts/coverage.sh (subset
    mode via COVERAGE_CRATES) for each targeted re-measure round. Writes the final
    per-crate-MAX summary back to `out_path` (default: the input summary path, so the
    uploaded artifact reflects the most-accurate numbers). Then performs the canonical
    --check on that final summary for the human-readable per-crate table + exit code."""
    floors = load(floor_path)["crates"]
    initial = load(summary_path)
    script_dir = os.path.dirname(os.path.abspath(__file__))
    cov_sh = os.path.join(script_dir, "coverage.sh")
    out_path = out_path or summary_path

    def measure_fn(crates):
        # Re-measure ONLY `crates` into a TEMP summary (do not clobber the accumulator),
        # via coverage.sh's documented COVERAGE_CRATES subset mode + COVERAGE_OUT.
        tmp = out_path + ".remeasure.json"
        env = dict(os.environ)
        env["COVERAGE_CRATES"] = " ".join(crates)
        env["COVERAGE_OUT"] = tmp
        # Fixtures are already fetched by round 1; skip the re-fetch to save time.
        env.setdefault("COVERAGE_FETCH_FIXTURES", "0")
        if extra_env:
            env.update(extra_env)
        subprocess.run(["bash", cov_sh], env=env, check=True)
        with open(tmp) as f:
            return json.load(f)

    print(f"==> robust coverage gate: max across up to K={k} measurement(s) "
          f"(re-measure ONLY sub-floor crates)")
    code, final = robust_aggregate(measure_fn, initial, floors, k=k,
                                   require_all=require_all)
    # Persist the per-crate-MAX summary so the uploaded artifact is the accurate one.
    with open(out_path, "w") as f:
        json.dump(final, f, indent=2, sort_keys=True); f.write("\n")
    print(f"==> wrote final per-crate-MAX summary to {out_path}")
    # Canonical check for the familiar per-crate table + the authoritative exit code.
    print("==> final verdict (canonical --check over the per-crate-MAX summary):")
    return check(out_path, floor_path, require_all)

def _load_floors_obj(doc):
    """A floor file's `crates` map, accepting either the full doc or a bare crates map."""
    return doc["crates"] if isinstance(doc, dict) and "crates" in doc else doc


def _base_floors_from_git(base_ref, floor_path, log=print):
    """[OPUS-4.8] sq-neq8: load the BASE floor file from git (`<base_ref>:<repo-rel path>`).
    Returns the `crates` map, or None if the base ref / path is unavailable (e.g. the file
    did not exist on base — a brand-new floor file cannot regress anything, so the caller
    fail-OPENs). Resolves the floor's path relative to the repo root so the git pathspec is
    correct regardless of CWD."""
    try:
        root = subprocess.run(["git", "rev-parse", "--show-toplevel"],
                              capture_output=True, text=True, check=True).stdout.strip()
        rel = os.path.relpath(os.path.abspath(floor_path), root)
        # git uses forward slashes in pathspecs on every platform.
        rel = rel.replace(os.sep, "/")
        r = subprocess.run(["git", "show", f"{base_ref}:{rel}"],
                           capture_output=True, text=True)
        if r.returncode != 0:
            log(f"  note: base '{base_ref}:{rel}' unavailable "
                f"({r.stderr.strip() or 'not found'}) — monotonic gate fail-OPEN (PASS)")
            return None
        return _load_floors_obj(json.loads(r.stdout))
    except (subprocess.SubprocessError, OSError, json.JSONDecodeError) as e:
        log(f"  note: could not read base floors via git ({e}) — fail-OPEN (PASS)")
        return None


def workspace_crate_dirs(crates_dir=CRATES_DIR):
    """[OPUS-5] #6121: every directory under crates/ that carries a Cargo.toml.

    Deliberately the SAME rule scripts/coverage-presence.py scan() uses, so the two coverage
    gates can never disagree about what counts as "a crate in the tree"."""
    if not os.path.isdir(crates_dir):
        return []
    return sorted(c for c in os.listdir(crates_dir)
                  if os.path.isfile(os.path.join(crates_dir, c, "Cargo.toml")))


def untracked_crates(disk, floors, declared):
    """[OPUS-5] #6121: PURE — crates in the tree with NO floor row and NO declaration.

    This is the gate's failure set: a crate here is measured by nothing and gated by
    nothing, and (unlike a floor that regressed) no other mode can see it."""
    return sorted(set(disk) - set(floors) - set(declared))


def stale_declarations(disk, floors, declared):
    """[OPUS-5] #6121: PURE — (crate, why) for every declaration that no longer describes
    reality. This is what makes the AWAITING_SEED ledger SHRINK-ONLY: the moment a crate
    earns a real floor row, its "not floored yet" note is a lie and the gate says so."""
    out = []
    for crate in sorted(declared):
        if crate in floors:
            out.append((crate, "now HAS a floor row — delete the declaration"))
        elif crate not in disk:
            out.append((crate, "no longer exists under crates/ — delete the declaration"))
    return out


def orphan_floors(disk, floors):
    """[OPUS-5] #6121: PURE — floor rows whose crate is gone from crates/ (the mirror-image
    drift: the floor file running AHEAD of the tree). coverage-presence.py --check already
    fails on this shape ("crate DISAPPEARED from crates/"); this keeps the two consistent."""
    return sorted(set(floors) - set(disk))


def check_untracked(floor_path, crates_dir=CRATES_DIR, declared=None, disk=None, log=print):
    """[OPUS-5] #6121: FAIL if crates/ and the floor file have drifted apart. Fast + hermetic
    (no compile, no measured summary, no network), so it belongs in the same no-compile
    `coverage-floors` job as --check-monotonic. Returns an exit code."""
    floors = _load_floors_obj(load(floor_path))
    if declared is None:
        overlap = sorted(set(NO_FLOOR_CRATES) & set(AWAITING_SEED))
        assert not overlap, f"a crate may be exempt OR awaiting a seed, not both: {overlap}"
        declared = {**NO_FLOOR_CRATES, **AWAITING_SEED}
    if disk is None:
        disk = workspace_crate_dirs(crates_dir)

    untracked = untracked_crates(disk, floors, declared)
    stale = stale_declarations(disk, floors, declared)
    orphans = orphan_floors(disk, floors)
    debt = sorted(c for c in disk if c in AWAITING_SEED and c not in floors)

    log(f"==> coverage untracked-crate gate: {len(disk)} crate(s) under crates/ vs "
        f"{len(floors)} floor row(s) + {len(declared)} declaration(s)")
    # The debt is grouped by REASON (26 identical paragraphs would bury the FAIL lines) but
    # every crate is still named — no silent truncation.
    for reason in sorted({AWAITING_SEED[c] for c in debt}):
        members = [c for c in debt if AWAITING_SEED[c] == reason]
        log(f"  debt ({len(members)}) NO line-% floor: {', '.join(members)}")
        log(f"         -> {reason}")
    for crate in untracked:
        log(f"  FAIL {crate:<28} UNTRACKED: no floor row and no declaration")
    for crate, why in stale:
        log(f"  FAIL {crate:<28} STALE declaration: {why}")
    for crate in orphans:
        log(f"  FAIL {crate:<28} ORPHAN floor row: crate absent from crates/")

    if untracked:
        log(f"::error::{len(untracked)} crate(s) under crates/ carry NO coverage floor and "
            f"are not declared: {', '.join(untracked)}. Seed a floor (scripts/coverage.sh "
            f"then coverage-gate.py --seed, plus a PER_COMMIT_CRATES + SHARD_GROUPS slot in "
            f"scripts/coverage.sh), or — only if a line-% floor genuinely cannot measure the "
            f"crate — declare it in scripts/coverage-gate.py NO_FLOOR_CRATES/AWAITING_SEED "
            f"with a reason. Silently un-gated crates are what #6121 fixed.")
    if stale:
        log(f"::error::{len(stale)} stale declaration(s) in scripts/coverage-gate.py: "
            + "; ".join(f"{c} ({w})" for c, w in stale))
    if orphans:
        log(f"::error::{len(orphans)} floor row(s) name a crate that no longer exists under "
            f"crates/: {', '.join(orphans)}. Drop the row (coverage-gate.py "
            f"--check-monotonic needs --allow-lower for a deliberate removal).")
    bad = bool(untracked) or bool(stale) or bool(orphans)
    log(f"\nuntracked-crate gate: {len(floors)} floored / {len(debt)} awaiting a seed / "
        f"{len(untracked)} untracked / {len(stale)} stale / {len(orphans)} orphan — "
        + ("FAIL" if bad else "PASS"))
    return 1 if bad else 0


def check_monotonic(floor_path, base_ref, base_file, allow_lower, log=print):
    """[OPUS-4.8] sq-neq8: FAIL if THIS branch's floor file LOWERS or DROPS any crate's floor
    vs the base (origin/main by default). Mirrors the conformance ratchet's only-rises rule.
    `--allow-lower` permits a deliberate, reviewed regression. Returns an exit code."""
    new_floors = _load_floors_obj(load(floor_path))
    if base_file is not None:
        base_floors = _load_floors_obj(load(base_file))
        src = base_file
    else:
        base_floors = _base_floors_from_git(base_ref, floor_path, log=log)
        src = f"{base_ref}:{os.path.basename(floor_path)}"
    if base_floors is None:
        log("coverage monotonic gate: no base to compare — PASS")
        return 0

    regressions = floor_regressions(base_floors, new_floors)
    dropped = dropped_crates(base_floors, new_floors)
    log(f"==> coverage monotonic gate: comparing floors vs {src}")
    for crate, bf, nf in regressions:
        log(f"  LOWERED  {crate:<20} floor {bf} -> {nf}")
    for crate in dropped:
        log(f"  DROPPED  {crate:<20} (floor {floor_of(base_floors[crate])} -> absent)")
    bad = bool(regressions) or bool(dropped)
    if not bad:
        log("  ok: every base crate's floor is preserved or RAISED")
        log("coverage monotonic gate: PASS")
        return 0
    if allow_lower:
        log("  --allow-lower: the floor regression(s) above are an EXPLICIT, reviewed "
            "decrease — PASS")
        log("coverage monotonic gate: PASS (--allow-lower)")
        return 0
    log(f"::error::coverage floor regressed for {len(regressions)} crate(s) and "
        f"{len(dropped)} dropped crate(s) vs {src}. The coverage ratchet only RISES "
        f"(sq-neq8). If this lowering is intentional, re-run with --allow-lower.")
    log("coverage monotonic gate: FAIL")
    return 1


def main():
    # --self-test is a standalone mode (mirrors scripts/perf-gate.py --self-test):
    # unit-test the PURE aggregation logic on synthetic measurement sequences, no files.
    if "--self-test" in sys.argv[1:]:
        sys.exit(self_test())
    # --merge-max is a tiny 2-positional helper with its own shape; parse it directly so
    # the main parser's single-`summary` contract is not muddied.
    if "--merge-max" in sys.argv[1:]:
        sys.exit(_cli_merge_max(sys.argv[1:]))

    ap = argparse.ArgumentParser(description="per-crate coverage ratchet gate")
    # `summary` is required for --seed/--check/--check-robust but NOT for --check-monotonic
    # (which diffs floor FILES, not a measured summary); validated below after parsing.
    ap.add_argument("summary", nargs="?", default=None,
                    help="coverage-summary.json from scripts/coverage.sh "
                         "(not used by --check-monotonic)")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--seed", action="store_true", help="(re)generate the floor file")
    g.add_argument("--check", action="store_true", help="enforce the floor file")
    g.add_argument("--check-robust", action="store_true",
                   help="[OPUS-4.8] robust gate: re-measure ONLY sub-floor crates up to "
                        "K times, keep the per-crate MAX, fail only if still below floor")
    g.add_argument("--check-monotonic", action="store_true",
                   help="[OPUS-4.8] sq-neq8: FAIL if the floor file LOWERS/DROPS any crate's "
                        "floor vs the base (origin/main); the ratchet only RISES")
    g.add_argument("--check-untracked", action="store_true",
                   help="[OPUS-5] #6121: FAIL if a crate under crates/ has neither a floor "
                        "row nor a declaration in NO_FLOOR_CRATES/AWAITING_SEED (no compile, "
                        "no summary)")
    g.add_argument("--check-advance-allowed", action="store_true",
                   help="[SONNET-4.6] sq-6vshe.17: FAIL if the floor file RAISES a floor "
                        "while the post-merge coverage alarm issue is OPEN (fail-OPEN on "
                        "any probe error; never blocks a lowering)")
    ap.add_argument("--floor", default=os.path.join(os.path.dirname(__file__), "..",
                    "bench", "coverage-floor.json"))
    ap.add_argument("--allow-lower", action="store_true",
                    help="permit --seed/--check-monotonic to LOWER a floor "
                         "(deliberate, reviewed regression)")
    ap.add_argument("--require-all", action="store_true",
                    help="--check fails if a floor crate is absent from the summary")
    ap.add_argument("-k", "--max-measurements", type=int, default=DEFAULT_K,
                    help=f"--check-robust: total measurements per crate (default {DEFAULT_K})")
    ap.add_argument("--out", default=None,
                    help="--check-robust: write the final per-crate-MAX summary here "
                         "(default: overwrite the input summary)")
    ap.add_argument("--base-ref", default="origin/main",
                    help="--check-monotonic: git ref of the base floor file "
                         "(default origin/main)")
    ap.add_argument("--base-file", default=None,
                    help="--check-monotonic: compare against a base floor FILE on disk "
                         "instead of a git ref (overrides --base-ref)")
    ap.add_argument("--crates-dir", default=CRATES_DIR,
                    help="--check-untracked: the workspace crates/ directory to scan "
                         "(default: the repo's own)")
    ap.add_argument("--alarm-lane", default=COVERAGE_ALARM_LANE,
                    help="--check-advance-allowed: demoted-lane token of the post-merge "
                         f"coverage alarm issue (default {COVERAGE_ALARM_LANE})")
    a = ap.parse_args()
    floor = os.path.abspath(a.floor)
    if a.check_monotonic:
        sys.exit(check_monotonic(floor, a.base_ref, a.base_file, a.allow_lower))
    if a.check_untracked:
        sys.exit(check_untracked(floor, os.path.abspath(a.crates_dir)))
    if a.check_advance_allowed:
        sys.exit(check_advance_allowed(floor, a.base_ref, a.base_file, a.alarm_lane))
    if a.summary is None:
        ap.error("the 'summary' argument is required for "
                 "--seed/--check/--check-robust")
    if a.seed:
        sys.exit(seed(a.summary, floor, a.allow_lower))
    if a.check_robust:
        sys.exit(check_robust(a.summary, floor, a.max_measurements, a.require_all,
                              out_path=a.out))
    sys.exit(check(a.summary, floor, a.require_all))

def _cli_merge_max(argv):
    """`coverage-gate.py --merge-max a.json b.json [-o out.json]` — per-crate MAX merge.
    Prints to stdout if no -o is given. A pure I/O wrapper over merge_max()."""
    args = [x for x in argv if x != "--merge-max"]
    out = None
    if "-o" in args:
        i = args.index("-o"); out = args[i + 1]; del args[i:i + 2]
    elif "--out" in args:
        i = args.index("--out"); out = args[i + 1]; del args[i:i + 2]
    if len(args) != 2:
        sys.stderr.write("usage: coverage-gate.py --merge-max <a.json> <b.json> "
                         "[-o out.json]\n")
        return 1
    merged = merge_max(load(args[0]), load(args[1]))
    text = json.dumps(merged, indent=2, sort_keys=True) + "\n"
    if out:
        with open(out, "w") as f:
            f.write(text)
        print(f"merged per-crate MAX -> {out}")
    else:
        sys.stdout.write(text)
    return 0

def self_test():
    """[OPUS-4.8] sq-x4jy: unit-test the PURE max-remeasure aggregation on SYNTHETIC
    measurement sequences — this is how we KNOW the robust gate works without reproducing
    the CI flake. No files, no subprocess; `measure_fn` is a synthetic round generator.
    Mirrors scripts/perf-gate.py --self-test."""

    def crate(pct, measured=True):
        # Minimal row in the coverage.sh summary shape.
        r = {"measured": measured}
        if measured:
            r.update(lines_pct=pct, lines_covered=int(pct), lines_total=100, seconds=1)
        return r

    def summ(d, tier=None):
        s = {"crates": {k: crate(*v) if isinstance(v, tuple) else crate(v)
                        for k, v in d.items()}}
        if tier is not None:
            s["tier"] = tier
        return s

    FLOORS = {"a": {"floor": 80}, "b": {"floor": 83}, "c": {"floor": 90}}
    quiet = lambda *a, **k: None

    # --- effective_floor / tier-aware nightly_floor (sq-bjct) [OPUS-4.8] --------
    fe = {"floor": 90, "nightly_floor": 94}
    assert effective_floor(fe, "per-commit") == 90, "per-commit uses base floor"
    assert effective_floor(fe, "nightly") == 94, "nightly uses nightly_floor"
    assert effective_floor(fe, "full") == 94, "any non-per-commit tier uses nightly_floor"
    assert effective_floor(fe, None) == 90, "absent tier defaults to base floor"
    assert effective_floor({"floor": 90}, "nightly") == 90, "no nightly_floor -> base"
    assert effective_floor(88, "nightly") == 88, "bare-number entry -> itself"
    # A merge crate at 92% PASSES per-commit (floor 90) but FAILS nightly (floor 94).
    FL_M = {"core": {"floor": 90, "nightly_floor": 94}}
    assert sub_floor_crates(summ({"core": 92.0}, tier="per-commit"), FL_M) == []
    assert sub_floor_crates(summ({"core": 92.0}, tier="nightly"), FL_M) == ["core"]
    # ...and a merge crate at 95% passes BOTH tiers.
    assert sub_floor_crates(summ({"core": 95.0}, tier="nightly"), FL_M) == []

    # --- merge_max: per-crate MAX, measured-only, internal consistency ---------
    m = merge_max(summ({"a": 70.0, "b": 90.0}), summ({"a": 95.0, "b": 88.0}))
    assert m["crates"]["a"]["lines_pct"] == 95.0, m
    assert m["crates"]["b"]["lines_pct"] == 90.0, m            # prev higher wins
    # a FAILED (non-measured) re-measure must NOT displace a measured prev row.
    m2 = merge_max(summ({"a": 85.0}), {"crates": {"a": crate(0, measured=False)}})
    assert m2["crates"]["a"]["lines_pct"] == 85.0, m2
    # a crate only in `new` is added.
    m3 = merge_max(summ({"a": 80.0}), summ({"z": 99.0}))
    assert m3["crates"]["z"]["lines_pct"] == 99.0, m3

    # --- sub_floor_crates: only measured-and-below are returned ----------------
    s = summ({"a": 79.9, "b": 84.0, "c": 91.0})
    assert sub_floor_crates(s, FLOORS) == ["a"], sub_floor_crates(s, FLOORS)
    s_missing = summ({"a": 90.0})  # b, c absent -> not "below", just missing
    assert sub_floor_crates(s_missing, FLOORS) == [], sub_floor_crates(s_missing, FLOORS)
    s_unmeasured = {"crates": {"a": crate(0, measured=False)}}  # not measured -> skip
    assert sub_floor_crates(s_unmeasured, FLOORS) == [], "unmeasured must not be 'below'"

    # === SCENARIO 1: a crate flakes low once then recovers above floor -> PASS ===
    # (max wins) — the exact #62 sparq-core shape: pass-1 below floor, re-measure above.
    init1 = summ({"a": 69.6, "b": 90.0, "c": 91.0})   # 'a' flaked low (floor 80)
    rounds1 = iter([summ({"a": 91.2})])               # round-2 re-measure recovers
    code1, fin1 = robust_aggregate(lambda cs: next(rounds1), init1, FLOORS, k=3, log=quiet)
    assert code1 == 0, "scenario1 should PASS (max recovers above floor)"
    assert fin1["crates"]["a"]["lines_pct"] == 91.2, fin1

    # === SCENARIO 2: two DIFFERENT crates flake on different rounds -> PASS ======
    # round1: 'a' low. round2 re-measures {a}: 'a' recovers but the WHOLE-suite is NOT
    # touched, so 'b' can only flake if IT was re-measured. To exercise per-crate MAX
    # across rounds we model: round1 -> a low; round2 (remeasure a) -> a recovers AND we
    # also feed a fresh whole-ish summary where b dipped; per-crate MAX must keep both ups.
    init2 = summ({"a": 70.0, "b": 90.0, "c": 91.0})   # only 'a' below floor in round1
    # round-2 re-measures {a}; return a low 'a' AND a low 'b' (b shouldn't matter — only
    # 'a' was asked for, but merge_max must not let b's low value overwrite the good prev).
    rounds2 = iter([summ({"a": 81.0, "b": 10.0})])
    code2, fin2 = robust_aggregate(lambda cs: next(rounds2), init2, FLOORS, k=3, log=quiet)
    assert code2 == 0, "scenario2 should PASS"
    assert fin2["crates"]["a"]["lines_pct"] == 81.0, fin2   # a recovered (max 70->81)
    assert fin2["crates"]["b"]["lines_pct"] == 90.0, fin2   # b's good prev preserved

    # 2b: genuinely-distinct flakes across rounds both recover via per-crate MAX.
    #   round1: a=70 (low), b=82 (low). round2 remeasures {a,b}: a=85, b=70 (b still low,
    #   a ok). round3 remeasures {b}: b=84 (recovers). All three pass on best-of-K.
    init2b = summ({"a": 70.0, "b": 82.0, "c": 95.0})
    seq2b = iter([summ({"a": 85.0, "b": 70.0}), summ({"b": 84.0})])
    code2b, fin2b = robust_aggregate(lambda cs: next(seq2b), init2b, FLOORS, k=3, log=quiet)
    assert code2b == 0, "scenario2b should PASS (per-crate max across rounds)"
    assert fin2b["crates"]["a"]["lines_pct"] == 85.0 and fin2b["crates"]["b"]["lines_pct"] == 84.0, fin2b

    # === SCENARIO 3: a crate below floor on ALL K measurements -> FAIL (exit 1) ==
    init3 = summ({"a": 50.0, "b": 90.0, "c": 91.0})
    seq3 = iter([summ({"a": 51.0}), summ({"a": 52.0})])  # never reaches floor 80
    code3, fin3 = robust_aggregate(lambda cs: next(seq3), init3, FLOORS, k=3, log=quiet)
    assert code3 == 1, "scenario3 should FAIL (real regression below floor on all K)"
    assert fin3["crates"]["a"]["lines_pct"] == 52.0, "final keeps the best (max) seen"

    # 3b: K=1 means NO re-measure at all — a low initial fails immediately.
    code3b, _ = robust_aggregate(lambda cs: (_ for _ in ()).throw(AssertionError(
        "K=1 must not re-measure")), summ({"a": 50.0}), FLOORS, k=1, log=quiet)
    assert code3b == 1, "K=1 with a sub-floor crate must FAIL without re-measuring"

    # === SCENARIO 4: a floor crate MISSING from the summary ====================
    # sub_floor_crates ignores it (can't re-measure what didn't run); robust PASSES the
    # aggregation, and require_all is enforced by the FINAL canonical --check (CLI layer),
    # matching existing --check semantics. Verify robust_aggregate itself treats missing
    # as not-below (so it doesn't spuriously fail / loop).
    init4 = summ({"a": 90.0})  # b, c missing
    code4, fin4 = robust_aggregate(lambda cs: (_ for _ in ()).throw(AssertionError(
        "must not re-measure a missing crate")), init4, FLOORS, k=3, log=quiet)
    assert code4 == 0, "missing crates are not 'below floor' for the robust loop"

    # === SCENARIO 5: no re-measure needed (all pass round 1) -> measure_fn never called =
    init5 = summ({"a": 85.0, "b": 90.0, "c": 95.0})
    code5, _ = robust_aggregate(lambda cs: (_ for _ in ()).throw(AssertionError(
        "should not re-measure when all pass")), init5, FLOORS, k=3, log=quiet)
    assert code5 == 0

    # === MONOTONIC gate (sq-neq8): floor-vs-base ratchet direction =============
    # floor_regressions: only DECREASES are returned; raises / equal / new pass.
    base = {"a": {"floor": 80}, "b": 83, "c": {"floor": 90}}
    # b lowered 83->70 (bare-int form), c raised, a equal, d is new -> only b regresses.
    nw = {"a": {"floor": 80}, "b": {"floor": 70}, "c": {"floor": 95}, "d": {"floor": 50}}
    assert floor_regressions(base, nw) == [("b", 83, 70)], floor_regressions(base, nw)
    # the exact #661 shape: sparq-serve 92 -> 83 silently lowered by a re-seed.
    b661 = {"sparq-serve": {"floor": 92}}
    n661 = {"sparq-serve": {"floor": 83}}
    assert floor_regressions(b661, n661) == [("sparq-serve", 92, 83)], "must catch #661"
    # no regression when every floor is preserved or raised.
    assert floor_regressions(base, {"a": {"floor": 80}, "b": 83, "c": {"floor": 91}}) == []
    # a DROPPED crate is not a "floor lowering" but IS reported by dropped_crates.
    assert dropped_crates(base, {"a": {"floor": 80}, "c": {"floor": 90}}) == ["b"], \
        dropped_crates(base, {"a": {"floor": 80}, "c": {"floor": 90}})
    assert dropped_crates(base, nw) == []  # nothing dropped (a,b,c all present)
    # a brand-new crate present only in `new` is never a regression.
    assert floor_regressions(base, {**base, "z": {"floor": 99}}) == []
    assert dropped_crates(base, {**base, "z": {"floor": 99}}) == []

    # [OPUS-4.8] sq-bjct: nightly_floor is part of the ratchet — lowering or DROPPING
    # one (once present in base) is a regression; ADDING one is not.
    bnf = {"core": {"floor": 90, "nightly_floor": 94}}
    assert floor_regressions(bnf, {"core": {"floor": 90, "nightly_floor": 92}}) \
        == [("core.nightly_floor", 94, 92)]
    assert floor_regressions(bnf, {"core": {"floor": 90}}) \
        == [("core.nightly_floor", 94, "DROPPED")]
    assert floor_regressions(bnf, {"core": {"floor": 90, "nightly_floor": 95}}) == []  # raised
    # adding a nightly_floor where base had none is fine; base floor unchanged.
    assert floor_regressions({"core": {"floor": 90}},
                             {"core": {"floor": 90, "nightly_floor": 92}}) == []

    # === ADVANCE PAUSE (sq-6vshe.17): floor_advances + advance_block_verdict ====
    # [SONNET-4.6] floor_advances is the exact MIRROR of floor_regressions: only RAISES.
    # Same fixtures as above: c is raised 90->95, b LOWERED, a equal, d new.
    assert floor_advances(base, nw) == [("c", 90, 95)], floor_advances(base, nw)
    # a LOWERING is never an advance (that direction belongs to floor_regressions).
    assert floor_advances(b661, n661) == []
    # a brand-NEW crate row is NOT an advance — adding a crate to the ratchet must stay
    # possible while the alarm is open (it gates something previously un-gated).
    assert floor_advances(base, {**base, "z": {"floor": 99}}) == []
    # a DROPPED crate is not an advance either.
    assert floor_advances(base, {"a": {"floor": 80}, "c": {"floor": 90}}) == []
    # bare-int floor form on either side is handled (floor_of normalises).
    assert floor_advances({"b": 83}, {"b": {"floor": 90}}) == [("b", 83, 90)]
    # nightly_floor RAISE is an advance; ADDING one where base had none is not.
    assert floor_advances(bnf, {"core": {"floor": 90, "nightly_floor": 95}}) \
        == [("core.nightly_floor", 94, 95)]
    assert floor_advances({"core": {"floor": 90}},
                          {"core": {"floor": 90, "nightly_floor": 92}}) == []
    # both directions at once on one crate: floor raised AND nightly_floor raised.
    assert floor_advances({"core": {"floor": 90, "nightly_floor": 94}},
                          {"core": {"floor": 91, "nightly_floor": 95}}) \
        == [("core", 90, 91), ("core.nightly_floor", 94, 95)]

    # advance_block_verdict: BLOCK iff (raises a floor) AND (alarm KNOWN open).
    adv = [("c", 90, 95)]
    assert advance_block_verdict(adv, True) == 1, "a raise + an OPEN alarm must BLOCK"
    assert advance_block_verdict(adv, False) == 0, "a raise with no alarm is allowed"
    assert advance_block_verdict(adv, None) == 0, "an unavailable probe must FAIL-OPEN"
    # no advance => never blocked, whatever the alarm says (incl. the recovery path: a
    # governed lowering under --allow-lower raises nothing, so it is never paused).
    for st in (True, False, None):
        assert advance_block_verdict([], st) == 0, "no raise must never block"

    # check_advance_allowed end-to-end over the PURE seam (injected probe, no gh):
    # a raise + open alarm FAILS; the same tree with no alarm PASSES.
    with tempfile.TemporaryDirectory() as td:
        bfp = os.path.join(td, "base-floor.json")
        nfp = os.path.join(td, "coverage-floor.json")
        with open(bfp, "w") as fh:
            json.dump({"crates": {"a": {"floor": 80}}}, fh)
        with open(nfp, "w") as fh:
            json.dump({"crates": {"a": {"floor": 85}}}, fh)
        assert check_advance_allowed(nfp, "origin/main", bfp,
                                     probe=lambda lane, log=print: True, log=quiet) == 1
        assert check_advance_allowed(nfp, "origin/main", bfp,
                                     probe=lambda lane, log=print: False, log=quiet) == 0
        assert check_advance_allowed(nfp, "origin/main", bfp,
                                     probe=lambda lane, log=print: None, log=quiet) == 0
        # No raise (identical floors) => the probe must not even be consulted.
        assert check_advance_allowed(bfp, "origin/main", bfp,
                                     probe=lambda lane, log=print: (_ for _ in ()).throw(
                                         AssertionError("must not probe without a raise")),
                                     log=quiet) == 0

    # === UNMEASURED vs MISSING in check() (sq-039g) [OPUS-4.8] ==================
    # The bug: a crate present in the summary with "measured": false (its coverage step
    # ERRORED — e.g. conformance fixtures absent -> sparq-conformance exit 2, or a
    # fixture-dependent sparq-core test aborting) was lumped into `missing`, so it was
    # fatal ONLY under --require-all. The per-commit / nightly --check-robust gate does
    # NOT pass --require-all, so such a crate was SILENTLY un-gated instead of gated on
    # its floor. Fix: an UNMEASURED crate with a non-zero (effective) floor ALWAYS fails;
    # a genuinely-MISSING crate keeps the --require-all semantics; a floor-0 unmeasured
    # crate (artifact) is reported but not fatal.
    # [SONNET-4.6] `tempfile` is now a MODULE-level import (the sq-6vshe.17 advance-pause
    # scenarios above use it earlier in this same function; a function-local `import
    # tempfile` here would make the name local for the WHOLE function body and turn those
    # earlier uses into an UnboundLocalError).
    import contextlib, io
    def run_check(summary_crates, floor_crates, require_all, tier=None, cone=None,
                  capture=None):
        """Drive the real check() over tempfile summary + floor; return its exit code.
        `cone` populates the summary's sq-3dr4t `cone` block; `capture` (a list) collects
        check()'s stdout so a test can assert on the per-crate labels."""
        sdoc = {"crates": {k: (crate(*v) if isinstance(v, tuple) else crate(v))
                           for k, v in summary_crates.items()}}
        if tier is not None:
            sdoc["tier"] = tier
        if cone is not None:
            sdoc["cone"] = cone
        fdoc = {"crates": floor_crates}
        with tempfile.NamedTemporaryFile("w", suffix=".sum.json", delete=False) as sf, \
             tempfile.NamedTemporaryFile("w", suffix=".floor.json", delete=False) as ff:
            json.dump(sdoc, sf); sp = sf.name
            json.dump(fdoc, ff); fp = ff.name
        try:
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                rc = check(sp, fp, require_all)
            if capture is not None:
                capture.append(buf.getvalue())
            return rc
        finally:
            os.unlink(sp); os.unlink(fp)

    FL = {"a": {"floor": 80}, "z": {"floor": 0, "note": "artifact"}}
    # a crate that MEASURED at/above floor + an artifact-zero crate -> PASS.
    assert run_check({"a": 90.0, "z": (0.0,)}, FL, require_all=False) == 0
    # UNMEASURED gated crate (a present, measured:false, floor 80) -> FAIL even WITHOUT
    # --require-all (the core of sq-039g: a measure-failure must not silently un-gate).
    assert run_check({"a": (0, False), "z": (0.0,)}, FL, require_all=False) == 1, \
        "unmeasured crate with a non-zero floor MUST fail without --require-all"
    # ...and it still fails WITH --require-all (regression guard).
    assert run_check({"a": (0, False), "z": (0.0,)}, FL, require_all=True) == 1
    # UNMEASURED ARTIFACT crate (z, floor 0) is NOT fatal — not %-gated, presence-gated.
    assert run_check({"a": 90.0, "z": (0, False)}, FL, require_all=False) == 0, \
        "an unmeasured floor-0 (artifact) crate must not fail the gate"
    # genuinely-MISSING crate (absent from summary) keeps the --require-all semantics:
    #   not fatal without --require-all ...
    assert run_check({"a": 90.0}, FL, require_all=False) == 0, \
        "a crate absent from the summary is not fatal without --require-all"
    #   ... fatal WITH --require-all.
    assert run_check({"a": 90.0}, FL, require_all=True) == 1, \
        "an absent crate must fail under --require-all"
    # The exact sq-039g danger: a conformance-fixture-dependent crate (sparq-core, real
    # floor) failing to MEASURE under the nightly tier -> FAIL even without --require-all.
    FLC = {"sparq-core": {"floor": 90, "nightly_floor": 94}}
    assert run_check({"sparq-core": (0, False)}, FLC, require_all=False,
                     tier="nightly") == 1, \
        "sparq-core failing to measure (fixtures absent) must FAIL the nightly gate"

    # === INHERITED (enforced changed cone, sq-3dr4t) [SONNET-4.6] ================
    # A crate the cone filter deliberately did not measure is absent from `crates`, so it
    # keeps the MISSING pass/fail semantics — but it MUST be reported under its own label,
    # or an intended skip is indistinguishable from a crate that silently fell out of the
    # tier (exactly the sq-039g failure class, one level up).
    CONE = {"filter": ["a"], "measured": ["a"], "inherited": ["b"]}
    FLI = {"a": {"floor": 80}, "b": {"floor": 80}}
    log_i = []
    assert run_check({"a": 90.0}, FLI, require_all=False, cone=CONE, capture=log_i) == 0, \
        "an inherited (outside-cone) crate must not fail the gate"
    assert "b" in log_i[0] and "INHERITED" in log_i[0], \
        f"the skipped crate must be labeled INHERITED, got:\n{log_i[0]}"
    assert "MISSING" not in log_i[0], \
        f"an inherited crate must NOT be reported as plain MISSING:\n{log_i[0]}"
    assert "1 inherited (changed cone)" in log_i[0], \
        f"the tally must count inherited crates separately:\n{log_i[0]}"
    # WITHOUT the cone block the same absent crate is plain MISSING (proving the block,
    # not something else, produces the label).
    log_m = []
    assert run_check({"a": 90.0}, FLI, require_all=False, capture=log_m) == 0
    assert "MISSING" in log_m[0] and "INHERITED" not in log_m[0], \
        f"an absent crate with no cone block stays MISSING:\n{log_m[0]}"
    # An absent crate NOT listed in cone.inherited is still plain MISSING even when a cone
    # block exists — the label follows the recorded list, not the mere presence of a cone.
    log_p = []
    assert run_check({"a": 90.0}, {"a": {"floor": 80}, "c": {"floor": 80}},
                     require_all=False, cone=CONE, capture=log_p) == 0
    assert "c" in log_p[0] and "MISSING" in log_p[0], \
        f"a crate outside cone.inherited must stay MISSING:\n{log_p[0]}"
    # An inherited crate that DID somehow get measured below floor still FAILS — the label
    # is reporting only and can never rescue a real breach.
    assert run_check({"a": 90.0, "b": 10.0}, FLI, require_all=False, cone=CONE) == 1, \
        "a measured, below-floor crate must fail even if listed as inherited"

    # --- `seed_pending`: a carried-forward floor is settled by the measurement (sq-iwf3c)
    # [SONNET-4.6] The failure mode this closes: coverage.sh starts measuring a crate over
    # a WIDER denominator, the floor is carried forward unmeasured, and --check happily
    # passes it because it only ever compares measured-vs-floor. If the wider surface
    # measures HIGHER, the entry has silently installed a floor LOOSER than the seed
    # procedure (floor(measured) - MARGIN) gives, and the ratchet loses ground with no
    # signal at all. With the flag, --check re-runs the seed procedure and FAILS on that.
    FSP = {"p": {"floor": 90, "seed_pending": True}}
    # measured 95.0 -> seed floor 93 > 90: the carried floor is too loose -> FAIL, even
    # though 95.0 is comfortably ABOVE 90 (this is exactly what plain --check misses).
    assert run_check({"p": 95.0}, FSP, require_all=False) == 1, \
        "a seed_pending crate measuring above its seed floor must FAIL (stale seed)"
    # ...and WITHOUT the flag the identical measurement passes — proving the flag, not
    # some other rule, is what fails it.
    assert run_check({"p": 95.0}, {"p": {"floor": 90}}, require_all=False) == 0, \
        "the same measurement must PASS without seed_pending (flag is the cause)"
    # measured 92.99 -> seed floor 90 == committed 90: the measurement CONFIRMS the
    # carried-forward floor -> PASS (top of the confirming band).
    assert run_check({"p": 92.99}, FSP, require_all=False) == 0, \
        "a seed_pending crate whose measurement confirms its floor must PASS"
    # 93.0 is the first value that seeds 91 > 90 -> FAIL (boundary of the band).
    assert run_check({"p": 93.0}, FSP, require_all=False) == 1, \
        "floor(93.0) - 2 = 91 > 90 -> stale seed"
    # A seed_pending crate BELOW its floor still fails as an ordinary floor breach.
    assert run_check({"p": 88.0}, FSP, require_all=False) == 1, \
        "a seed_pending crate below its floor must still fail the ordinary floor check"
    # A seed_pending crate that this tier did not MEASURE is not settleable here: the
    # unmeasured/missing rules apply unchanged, never the seed check.
    assert run_check({}, FSP, require_all=False) == 0, \
        "a seed_pending crate absent from the summary keeps the MISSING semantics"
    # The flag is tier-aware via effective_floor: nightly gates the (higher) nightly_floor,
    # so the same 95.0 that is a stale seed against floor 90 CONFIRMS a nightly_floor 93.
    FSPN = {"p": {"floor": 90, "nightly_floor": 93, "seed_pending": True}}
    assert run_check({"p": 95.0}, FSPN, require_all=False, tier="nightly") == 0, \
        "seed_pending compares against the EFFECTIVE (tier) floor"
    # --seed rebuilds the entry from the measurement, which is what drops the flag.
    with tempfile.NamedTemporaryFile("w", suffix=".sum.json", delete=False) as sf, \
         tempfile.NamedTemporaryFile("w", suffix=".floor.json", delete=False) as ff:
        json.dump({"crates": {"p": crate(95.0)}}, sf); sp = sf.name
        json.dump({"crates": FSP}, ff); fp = ff.name
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            seed(sp, fp, allow_lower=False)
        reseeded = load(fp)["crates"]["p"]
    finally:
        os.unlink(sp); os.unlink(fp)
    assert reseeded["floor"] == 93, f"--seed must raise the floor to 93, got {reseeded}"
    assert "seed_pending" not in reseeded, "--seed must clear the seed_pending flag"

    # === UNTRACKED-CRATE arm (#6121) [OPUS-5] ==================================
    # The drift no other mode can see: a crate in the tree with NO floor row. --check has
    # nothing to compare it against, --check-monotonic sees no base row to lower, and
    # coverage.sh --check-shards only compares PER_COMMIT_CRATES to SHARD_GROUPS. That is
    # how bench/coverage-floor.json ended up 28 crates behind crates/.
    DISK = ["a", "b", "c", "d"]
    FLOORED = {"a": {"floor": 80}}
    DECL = {"b": "exempt", "c": "awaiting seed"}
    # 'd' is in the tree, has no floor row and no declaration -> the failure set.
    assert untracked_crates(DISK, FLOORED, DECL) == ["d"], untracked_crates(DISK, FLOORED, DECL)
    # a fully-declared tree is clean...
    assert untracked_crates(DISK, FLOORED, {**DECL, "d": "x"}) == []
    # ...and so is one where every crate carries a real floor (the end state we want).
    assert untracked_crates(DISK, {c: {"floor": 1} for c in DISK}, {}) == []
    # SHRINK-ONLY: a declaration for a crate that NOW has a floor is stale (this is what
    # stops AWAITING_SEED from outliving the debt it records).
    assert stale_declarations(DISK, {"a": {"floor": 80}, "c": {"floor": 70}}, DECL) \
        == [("c", "now HAS a floor row — delete the declaration")]
    # ...as is a declaration for a crate that no longer exists in the tree.
    assert stale_declarations(["a", "b"], FLOORED, DECL) \
        == [("c", "no longer exists under crates/ — delete the declaration")]
    assert stale_declarations(DISK, FLOORED, DECL) == [], "declarations matching reality"
    # ORPHAN: the mirror-image drift — a floor row for a deleted crate.
    assert orphan_floors(["a"], {"a": 1, "gone": 2}) == ["gone"]
    assert orphan_floors(DISK, FLOORED) == []
    # The two registries are different KINDS of row; a crate in both would be incoherent
    # (and the merged `declared` map would silently hide one reason).
    assert not (set(NO_FLOOR_CRATES) & set(AWAITING_SEED)), \
        "a crate is either permanently exempt OR awaiting a seed, never both"

    # check_untracked end-to-end over a synthetic tree (proves the wiring, not just the
    # pure seam: a real crates/ dir + a real floor file on disk).
    with tempfile.TemporaryDirectory() as td:
        cdir = os.path.join(td, "crates")
        for c in ("floored", "declared", "undeclared"):
            os.makedirs(os.path.join(cdir, c))
            open(os.path.join(cdir, c, "Cargo.toml"), "w").close()
        os.makedirs(os.path.join(cdir, "not-a-crate"))   # no Cargo.toml -> not a crate
        fp = os.path.join(td, "floor.json")
        with open(fp, "w") as fh:
            json.dump({"crates": {"floored": {"floor": 80}}}, fh)
        ok_decl = {"declared": "documented exemption", "undeclared": "documented exemption"}
        assert check_untracked(fp, cdir, declared=ok_decl, log=quiet) == 0, \
            "a fully floored-or-declared tree must PASS"
        # Drop the declaration for 'undeclared' -> the #6121 shape -> FAIL.
        assert check_untracked(fp, cdir, declared={"declared": "x"}, log=quiet) == 1, \
            "a crate with no floor row and no declaration must FAIL the gate"
        # A directory without a Cargo.toml is not a crate and must not fail the gate.
        assert "not-a-crate" not in workspace_crate_dirs(cdir), \
            "only directories carrying a Cargo.toml count as crates"
        # A declaration for a crate that now has a floor is STALE -> FAIL.
        assert check_untracked(fp, cdir, declared={**ok_decl, "floored": "x"}, log=quiet) == 1, \
            "a declaration for an already-floored crate must FAIL as stale"
        # A floor row for a crate that is not in the tree is an ORPHAN -> FAIL.
        with open(fp, "w") as fh:
            json.dump({"crates": {"floored": {"floor": 80}, "deleted": {"floor": 50}}}, fh)
        assert check_untracked(fp, cdir, declared=ok_decl, log=quiet) == 1, \
            "a floor row naming a crate absent from the tree must FAIL as an orphan"

    print("coverage-gate self-test: ALL ASSERTIONS PASSED")
    return 0

if __name__ == "__main__":
    main()
