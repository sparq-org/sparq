#!/usr/bin/env python3
# [OPUS-5] HEAVY-set drift alarm — sq-6vshe.7 phase 4 (sparq-org/sparq#5563). 🤖 SPARQ agent.
#
# WHAT THIS IS. ci.yml's `test` job is a LOAD-AWARE shard matrix: the two measured-heavy
# recall tests each get a dedicated shard and the uniform remainder is count-partitioned
# across three bulk shards. The whole design rests on ONE piece of hand-maintained state —
# the `HEAVY` nextest filterset env (`.github/workflows/ci.yml`, `jobs.test.env.HEAVY`) —
# and its upkeep path, documented in that job's header, was:
#
#     "When nextest's run output flags a NEW test SLOW (the `SLOW [> Ns]` markers), add
#      `| test(=its_name)` to HEAVY and ... give it its own matrix shard."
#
# i.e. a HUMAN reading a marker in a shard log. Nothing enforces that. A newly-slow test
# silently lands in a bulk shard and re-skews the split; the shards stop finishing
# together; the only symptom is a slower merge queue, which nobody attributes back here.
# `HEAVY` is exactly the kind of state that is correct on the day it is written and
# quietly wrong forever after.
#
# This script closes that loop by MEASURING instead of reading markers. It takes a
# nextest `libtest-json-plus` event stream for one full-workspace run, extracts the
# per-test wall-time each event carries, and asks the question the upkeep path asks a
# human to ask: **is `HEAVY` still the measured heavy set?** Three ways the answer can be
# no, all reported by name:
#
#   * UNDECLARED — a test measured at or above the heavy floor that `HEAVY` does not
#     name. This is the re-skew: it is running inside a bulk shard right now.
#   * MISSING    — a test `HEAVY` names that never appeared in the run. `test(=NAME)` is
#     an EXACT-match atom, so a rename/removal does not error — the heavy shard silently
#     runs ZERO tests while its test (if it still exists under a new name) rejoins the
#     bulk. Fails open and invisible; the worst of the three.
#   * DEMOTED    — a test `HEAVY` names that is no longer anywhere near heavy. Harmless
#     to correctness (every test still runs exactly once) but it burns a whole matrix
#     shard on a fast test, so the split is paying for parallelism it no longer gets.
#
# NOT A GATE, and NOT A BEAD-WRITER. On drift this exits 1 (its own scheduled run goes
# red, LOUD) and files ONE deduped, self-identified GitHub issue. It never touches `bd`
# or `.beads` — CI writing the bead store is forbidden repo-wide; the orchestrator
# reconciles alarm issues into beads out-of-band, exactly as selection-alarm.yml /
# formal-alarm.yml do. Redding this run blocks no merge, and that holds ONLY because the
# job is declared in `.github/advisory-registry.json` (since #3773 the sole mechanism
# that makes a check-run non-gating — a scheduled run on main lands its check-run on the
# same head SHA the push-triggered `ci-summary / gate` polls; see sq-huwr8). Pinned by
# scripts/tests/test_alarm_lanes_non_gating.py.
#
# FAIL-LOUD (exit 2), because the failure mode of a drift detector is reporting "all
# clear" over nothing at all. Every one of these is infrastructure breakage, not drift:
#   * ci.yml unreadable, or `jobs.test.env.HEAVY` gone / carrying no `test(=…)` atom —
#     the declared set could not be read, so nothing was compared;
#   * the event stream carried no test events — the measurement run produced nothing;
#   * test events were present but NONE carried a usable `exec_time` — the event schema
#     moved under us. Without this guard the script would find zero tests above the floor
#     forever and report a permanent, confident, meaningless all-clear. This is the one
#     guard the whole lane's value depends on.
#
# WHERE THE MEASUREMENT COMES FROM. #5563 proposed `scripts/ci_leg_walltime_inventory.py`
# as the home for the measurement half; no such script exists in this tree (nor the
# `research/ci-leg-walltime-inventory.md` record it cites), so the measurement lives here
# with its consumer. The event-stream source is the precedent already in the repo:
# miri.yml runs nextest under `NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 --message-format
# libtest-json-plus` and scripts/miri_classify_verdict.py parses the result.
#
# Usage:
#   heavy_set_alarm.py --events events.jsonl                 # real (gh; GITHUB_REPOSITORY)
#   heavy_set_alarm.py --events events.jsonl --dry-run       # print findings; no gh writes
#   heavy_set_alarm.py --events e.jsonl --ci-yaml f.yml ...  # hermetic (tests)
#   heavy_set_alarm.py --self-test                           # hermetic fixtures
#
# Stdlib only + the `gh` CLI (present on every GitHub runner). No PyYAML: the one value
# read out of ci.yml is a single scalar assignment, and scripts/check-advisory-registry.py
# already establishes the stdlib line-scan precedent for reading workflow YAML. Keeping it
# dependency-free means this alarm — and its self-test — run identically on a runner, on
# the work box, and in the pre-merge docs-quality suite.

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CI_YAML = REPO_ROOT / ".github" / "workflows" / "ci.yml"

BASE_LABELS = ["heavy-set-alarm", "auto"]
KEY_PREFIX = "heavy-set-alarm-key"
# ONE issue per subject, not per offending test: the subject of this alarm is the single
# `HEAVY` env, so a second drift while the first is still open must not mint a second
# issue. Same non-spammy contract as formal_lane_alarm.py's per-lane key.
ISSUE_KEY = "ci.yml:jobs.test.env.HEAVY"

# The exact-match filterset atom ci.yml's HEAVY is built from: `test(=NAME)`.
HEAVY_ATOM_RE = re.compile(r"test\(\s*=\s*([^)]+?)\s*\)")
# The `HEAVY: <value>` assignment line itself. Matched against COMMENT-STRIPPED lines, so
# the job header's prose about `test(=its_name)` cannot be mistaken for a declaration.
HEAVY_ASSIGN_RE = re.compile(r"^\s*HEAVY:\s*(\S.*?)\s*$")

# CALIBRATION. ci.yml's header records the measured shape of this suite: two recall tests
# dominate the run and the ~1827 others are ~0.01s each — two populations separated by
# almost four orders of magnitude. The floor only has to land in that gap. It sits far
# above the entire bulk band (so ordinary runner jitter, a cold cache or a contended
# runner can never push a bulk test over it) and comfortably below the smaller of the two
# declared heavies (so neither is demoted by a merely slow night).
HEAVY_FLOOR_SECONDS = 20.0
# Hysteresis for DEMOTED only. A declared heavy has to fall well below the floor — not
# merely dip under it on one slow run — before the alarm calls its shard wasted, so a
# test hovering near the boundary cannot flap the lane red every week.
DEMOTE_FLOOR_SECONDS = HEAVY_FLOOR_SECONDS / 2


class AlarmError(Exception):
    """Alarm-INFRASTRUCTURE failure — surfaced LOUD (exit 2), never as an all-clear."""


# --------------------------------------------------------------------------- #
# The declared set: ci.yml jobs.test.env.HEAVY
# --------------------------------------------------------------------------- #
def load_declared_heavy(ci_yaml: Path) -> list[str]:
    """-> the test names `HEAVY` declares.

    Reads the single `HEAVY:` ASSIGNMENT, not the file's text: whole-line comments are
    dropped first, so the shard job's header — which discusses `test(=its_name)` and
    `test(=NAME)` in prose — cannot be mistaken for a declaration. Two or more live
    assignments is an AMBIGUITY, not a first-wins: which one the workflow actually uses
    depends on YAML scoping this scanner does not model, and guessing would let the alarm
    silently watch the wrong set.
    """
    try:
        text = ci_yaml.read_text(encoding="utf-8")
    except OSError as exc:
        raise AlarmError(f"cannot read {ci_yaml}: {exc}") from exc

    assignments: list[str] = []
    for line in text.splitlines():
        if line.lstrip().startswith("#"):
            continue
        match = HEAVY_ASSIGN_RE.match(line)
        if match:
            assignments.append(match.group(1))

    if not assignments:
        raise AlarmError(
            f"{ci_yaml}: no `HEAVY:` assignment — the load-aware shard matrix's single "
            f"source of truth is gone, so there is nothing to check drift against"
        )
    if len(assignments) > 1:
        raise AlarmError(
            f"{ci_yaml}: {len(assignments)} live `HEAVY:` assignments ({assignments!r}); "
            f"this scanner cannot tell which one the shard matrix uses"
        )

    value = assignments[0]
    # Prefer the double-quoted scalar when there is one, so a trailing `# ...` comment on
    # the assignment line can never contribute atoms.
    if value.startswith('"') and value.count('"') >= 2:
        value = value[1 : value.index('"', 1)]

    names = HEAVY_ATOM_RE.findall(value)
    if not names:
        raise AlarmError(
            f"{ci_yaml}: `HEAVY` carries no `test(=NAME)` atom (value: {value!r}); the "
            f"filterset syntax changed and this parser no longer understands it"
        )
    return names


# --------------------------------------------------------------------------- #
# The measured set: a nextest libtest-json-plus event stream
# --------------------------------------------------------------------------- #
def parse_walltimes(lines: list[str]) -> dict[str, float]:
    """-> {test name: slowest observed exec_time}, from a libtest-json-plus stream.

    nextest reports test names as `<binary-id>$<test-name>`, while `test(=NAME)` matches
    the test-name half; we key on that half so the two sides are comparable. Non-JSON
    lines (nextest's human-readable output shares stdout) are skipped, as in
    miri_classify_verdict.py. `max` on collision because a retried flaky test emits an
    event per attempt and the slowest attempt is the one that sets a shard's length.
    """
    walltimes: dict[str, float] = {}
    saw_test_event = False
    for raw in lines:
        raw = raw.strip()
        if not raw.startswith("{"):
            continue
        try:
            event = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if event.get("type") != "test":
            continue
        saw_test_event = True
        # Only terminal events carry a wall-time; `started` events do not.
        if event.get("event") not in ("ok", "failed", "timeout"):
            continue
        name = str(event.get("name") or "")
        if not name:
            continue
        exec_time = event.get("exec_time")
        if not isinstance(exec_time, (int, float)):
            continue
        short = name.rsplit("$", 1)[-1]
        walltimes[short] = max(walltimes.get(short, 0.0), float(exec_time))

    if not saw_test_event:
        raise AlarmError(
            "the libtest-json-plus stream carried no `\"type\": \"test\"` events — the "
            "measurement run produced nothing to measure (nextest failed to start, or the "
            "stream was not captured)"
        )
    if not walltimes:
        # THE anti-vacuity guard. Test events but no timings means the event schema moved;
        # silently continuing would report a confident all-clear over an empty measurement,
        # forever, which is precisely the invisible rot this lane exists to prevent.
        raise AlarmError(
            "the libtest-json-plus stream carried test events but NOT ONE usable `exec_time` "
            "— the event schema changed. Refusing to report on an empty measurement: every "
            "test would score 0s and the HEAVY set would look permanently correct"
        )
    return walltimes


# --------------------------------------------------------------------------- #
# The check (pure)
# --------------------------------------------------------------------------- #
def check_heavy_set(
    declared: list[str],
    walltimes: dict[str, float],
    heavy_floor: float = HEAVY_FLOOR_SECONDS,
    demote_floor: float = DEMOTE_FLOOR_SECONDS,
) -> list[dict]:
    """-> the drift findings, newest-skew-first. Empty list == HEAVY is still correct."""
    declared_set = set(declared)
    findings: list[dict] = []

    for name, seconds in sorted(walltimes.items(), key=lambda kv: -kv[1]):
        if seconds >= heavy_floor and name not in declared_set:
            findings.append({"kind": "UNDECLARED", "test": name, "seconds": seconds})

    for name in declared:
        # `.get`, not `in` + `[]`: a mutation that disables the MISSING branch must fail
        # this function's VERDICT (a finding that stops being produced), not crash it with
        # a KeyError — a crash kill proves the line is reachable, not that it is correct.
        measured = walltimes.get(name)
        if measured is None:
            findings.append({"kind": "MISSING", "test": name, "seconds": None})
        elif measured < demote_floor:
            findings.append({"kind": "DEMOTED", "test": name, "seconds": measured})

    return findings


_KIND_BLURB = {
    "UNDECLARED": (
        "measured heavy but NOT in `HEAVY` — it is running inside a bulk shard right now, "
        "re-skewing the load-aware split"
    ),
    "MISSING": (
        "declared in `HEAVY` but ABSENT from the run — `test(=NAME)` is an exact match, so "
        "its dedicated shard is silently running ZERO tests"
    ),
    "DEMOTED": (
        "declared in `HEAVY` but no longer heavy — its dedicated matrix shard is being spent "
        "on a fast test"
    ),
}


def render_issue(findings: list[dict], heavy_floor: float) -> tuple[str, str]:
    kinds = sorted({f["kind"] for f in findings})
    title = f"heavy-set-alarm: ci.yml `HEAVY` no longer matches the measured heavy set ({', '.join(kinds)})"

    def _fmt(f: dict) -> str:
        secs = "did not run" if f["seconds"] is None else f"{f['seconds']:.1f}s"
        return f"* **{f['kind']}** `{f['test']}` ({secs}) — {_KIND_BLURB[f['kind']]}"

    body = "\n".join(
        [
            "> 🤖 **SPARQ agent** — automated HEAVY-set drift alarm "
            "(scripts/heavy_set_alarm.py, bead sq-6vshe.7). @jeswr runs multiple agents on "
            "this account; this issue was filed by CI automation.",
            "",
            f"<!-- {KEY_PREFIX}: {ISSUE_KEY} -->",
            "",
            "`.github/workflows/ci.yml` `jobs.test.env.HEAVY` is the single source of truth "
            "for the load-aware shard matrix. A measured full-workspace run disagrees with it:",
            "",
            *[_fmt(f) for f in findings],
            "",
            f"Heavy floor: **{heavy_floor:g}s** (demotion floor {DEMOTE_FLOOR_SECONDS:g}s).",
            "",
            "**Fix**: for an UNDECLARED test, add `| test(=its_name)` to `HEAVY` and — if it "
            "is comparable to the existing heavies — give it its own `matrix.include` shard "
            "(the bulk filter is the negation of `HEAVY`, so it re-narrows automatically). "
            "For a MISSING one, update the atom to the test's current name or drop it along "
            "with its shard. For a DEMOTED one, drop it from `HEAVY` and delete its shard so "
            "the parallelism goes back to the bulk split.",
            "",
            "Left alone, the shards stop finishing together and the merge queue gets slower "
            "with no attributable symptom — the failure this lane exists to make visible.",
        ]
    )
    return title, body


# --------------------------------------------------------------------------- #
# Issue minting (mirrors formal_lane_alarm.py: label upsert + open-issue dedupe)
# --------------------------------------------------------------------------- #
def _run_gh(args: list[str]) -> str:
    try:
        out = subprocess.run(args, check=True, capture_output=True, text=True, timeout=120)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as exc:
        detail = getattr(exc, "stderr", "") or str(exc)
        raise AlarmError(f"{' '.join(args)} failed: {detail}") from exc
    return out.stdout


def ensure_labels(repo: str) -> None:
    for label in BASE_LABELS:
        _run_gh(["gh", "label", "create", label, "--repo", repo, "--force"])


def open_issue_exists(repo: str) -> bool:
    out = _run_gh(
        [
            "gh", "issue", "list", "--repo", repo, "--state", "open",
            "--label", BASE_LABELS[0], "--json", "number,body", "--limit", "100",
        ]
    )
    marker = f"{KEY_PREFIX}: {ISSUE_KEY}"
    return any(marker in (item.get("body") or "") for item in json.loads(out or "[]"))


def file_issue(repo: str, findings: list[dict], heavy_floor: float) -> None:
    title, body = render_issue(findings, heavy_floor)
    args = ["gh", "issue", "create", "--repo", repo, "--title", title, "--body", body]
    for label in BASE_LABELS:
        args += ["--label", label]
    print(f"filed: {_run_gh(args).strip()}")


# --------------------------------------------------------------------------- #
# main
# --------------------------------------------------------------------------- #
def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="ci.yml HEAVY-set drift alarm.")
    p.add_argument("--events", type=Path, help="nextest libtest-json-plus stream (JSONL)")
    p.add_argument("--ci-yaml", type=Path, default=DEFAULT_CI_YAML)
    p.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""))
    p.add_argument("--heavy-floor-seconds", type=float, default=HEAVY_FLOOR_SECONDS)
    p.add_argument("--dry-run", action="store_true", help="print findings; no gh writes")
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args(argv)

    if args.self_test:
        return self_test()

    try:
        if args.events is None:
            raise AlarmError("--events is required (the measured libtest-json-plus stream)")
        if not args.dry_run and not args.repo:
            raise AlarmError("--repo (or GITHUB_REPOSITORY) is required for a live run")

        declared = load_declared_heavy(args.ci_yaml)
        try:
            lines = args.events.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError as exc:
            raise AlarmError(f"cannot read event stream {args.events}: {exc}") from exc
        walltimes = parse_walltimes(lines)
        demote_floor = args.heavy_floor_seconds / 2
        findings = check_heavy_set(declared, walltimes, args.heavy_floor_seconds, demote_floor)

        print(
            f"HEAVY declares {len(declared)} test(s): {declared}; measured {len(walltimes)} "
            f"test(s), slowest "
            f"{sorted(walltimes.items(), key=lambda kv: -kv[1])[:3]}"
        )
        if not findings:
            print(
                f"heavy-set alarm: `HEAVY` still matches the measured heavy set "
                f"(floor {args.heavy_floor_seconds:g}s). No drift."
            )
            return 0

        for f in findings:
            secs = "did not run" if f["seconds"] is None else f"{f['seconds']:.1f}s"
            print(
                f"::error title=HEAVY-set drift ({f['kind']})::{f['test']} ({secs}): "
                f"{_KIND_BLURB[f['kind']]}"
            )

        if args.dry_run:
            title, body = render_issue(findings, args.heavy_floor_seconds)
            print(f"--- DRY RUN issue ---\n{title}\n{body}\n")
        else:
            ensure_labels(args.repo)
            if open_issue_exists(args.repo):
                print("dedupe: an open heavy-set-alarm issue already tracks this drift")
            else:
                file_issue(args.repo, findings, args.heavy_floor_seconds)
        # LOUD: the alarm run itself reds while HEAVY disagrees with the measurement.
        return 1
    except AlarmError as exc:
        # FAIL-LOUD: infrastructure breakage must never look like a quiet all-clear.
        print(f"::error title=heavy-set alarm infrastructure error::{exc}")
        return 2


# --------------------------------------------------------------------------- #
# Hermetic self-test
# --------------------------------------------------------------------------- #
def _event(binary: str, name: str, seconds: float | None, event: str = "ok") -> str:
    payload: dict = {"type": "test", "event": event, "name": f"{binary}${name}"}
    if seconds is not None:
        payload["exec_time"] = seconds
    return json.dumps(payload)


_FIXTURE_CI_YAML = """\
name: ci
on: [push]
jobs:
  test:
    name: test (load-aware shard ${{ matrix.name }})
    env:
      # A comment naming test(=decoy_from_a_comment) must NOT be read as a declaration.
      HEAVY: "test(=slow_a) | test(=slow_b)"
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
"""


def self_test() -> int:
    failures: list[str] = []

    def check(cond: bool, label: str) -> None:
        if not cond:
            failures.append(label)

    declared = ["slow_a", "slow_b"]
    healthy = {"slow_a": 108.7, "slow_b": 61.2, "tiny_1": 0.01, "tiny_2": 0.02}

    # 1. The declared set IS the measured heavy set => no findings.
    check(check_heavy_set(declared, healthy) == [], "matching HEAVY set is clean")

    # 2. A new heavy test nobody declared => UNDECLARED, named.
    f = check_heavy_set(declared, {**healthy, "new_slow": 45.0})
    check(
        [x for x in f if x["kind"] == "UNDECLARED" and x["test"] == "new_slow"],
        "a new above-floor test is UNDECLARED",
    )
    check(len(f) == 1, "a new heavy produces exactly one finding")

    # 3. A test just BELOW the floor is not a finding — the band that keeps jitter quiet.
    check(
        check_heavy_set(declared, {**healthy, "borderline": HEAVY_FLOOR_SECONDS - 0.1}) == [],
        "below-floor test does not alarm",
    )
    check(
        len(check_heavy_set(declared, {**healthy, "borderline": HEAVY_FLOOR_SECONDS})) == 1,
        "at-floor test does alarm (the boundary is inclusive)",
    )

    # 4. A declared heavy that vanished (renamed test / dead atom) => MISSING.
    f = check_heavy_set(declared, {"slow_a": 108.7, "tiny_1": 0.01})
    check(
        [x for x in f if x["kind"] == "MISSING" and x["test"] == "slow_b"],
        "a declared test absent from the run is MISSING",
    )

    # 5. A declared heavy that got fast => DEMOTED, but only past the hysteresis band.
    check(
        check_heavy_set(declared, {**healthy, "slow_b": DEMOTE_FLOOR_SECONDS + 0.1}) == [],
        "a declared heavy inside the hysteresis band is NOT demoted",
    )
    f = check_heavy_set(declared, {**healthy, "slow_b": 0.5})
    check(
        [x for x in f if x["kind"] == "DEMOTED" and x["test"] == "slow_b"],
        "a declared heavy well below the floor is DEMOTED",
    )

    # 6. Wall-time parsing: binary-id stripping, retry max, non-JSON tolerance, failed events.
    wt = parse_walltimes(
        [
            "    Starting 3 tests across 2 binaries",  # nextest human output on the same stream
            # A retry emits one event per attempt. The SLOWEST attempt is what sets a
            # shard's length, so it must win — and it is deliberately NOT last here, or a
            # plain last-writer-wins implementation would satisfy this assertion too.
            _event("sparq-vectors::ann", "slow_a", 108.7),
            _event("sparq-vectors::ann", "slow_a", 30.0),
            _event("sparq-core::store", "tiny_1", 0.01),
            _event("sparq-core::store", "boom", 12.5, event="failed"),
            json.dumps({"type": "suite", "event": "started", "test_count": 3}),
            "{ not json",
        ]
    )
    check(wt.get("slow_a") == 108.7, "retried test keeps its slowest attempt")
    check("sparq-vectors::ann$slow_a" not in wt, "binary-id prefix is stripped")
    check(wt.get("boom") == 12.5, "a FAILED test still contributes its wall-time")
    check(len(wt) == 3, "suite events and junk lines are ignored")

    # 7. FAIL-LOUD: a stream with no test events at all.
    try:
        parse_walltimes(["    Starting 0 tests", json.dumps({"type": "suite", "event": "ok"})])
        check(False, "empty measurement must raise")
    except AlarmError:
        pass

    # 8. FAIL-LOUD: test events present but NO exec_time — the schema-moved case. This is
    #    the guard that stops a permanent, meaningless all-clear.
    try:
        parse_walltimes([_event("b", "slow_a", None), _event("b", "tiny_1", None)])
        check(False, "timing-less measurement must raise")
    except AlarmError:
        pass

    with tempfile.TemporaryDirectory() as td:
        ci = Path(td) / "ci.yml"
        ci.write_text(_FIXTURE_CI_YAML)
        events = Path(td) / "events.jsonl"

        # 9. The declared set comes from the ASSIGNMENT — the decoy atom in the adjacent
        #    comment is not picked up, and both real atoms are.
        check(
            load_declared_heavy(ci) == ["slow_a", "slow_b"],
            "HEAVY parsed from the assignment, not from prose",
        )

        # 9b. AMBIGUITY is fail-loud, never first-wins: a second live assignment must not
        #     leave the alarm quietly watching the wrong set.
        two = Path(td) / "two-heavy.yml"
        two.write_text(_FIXTURE_CI_YAML + '    env:\n      HEAVY: "test(=other)"\n')
        try:
            load_declared_heavy(two)
            check(False, "two HEAVY assignments must raise")
        except AlarmError:
            pass

        # 9c. A trailing comment on the assignment line contributes no atoms.
        trailing = Path(td) / "trailing.yml"
        trailing.write_text('      HEAVY: "test(=slow_a)" # not test(=decoy_from_inline)\n')
        check(
            load_declared_heavy(trailing) == ["slow_a"],
            "an inline trailing comment is not read as a declaration",
        )

        # 10. End-to-end, clean => exit 0.
        events.write_text(
            "\n".join(
                [_event("b", "slow_a", 108.7), _event("b", "slow_b", 61.2), _event("b", "t", 0.01)]
            )
        )
        rc = main(["--events", str(events), "--ci-yaml", str(ci), "--dry-run"])
        check(rc == 0, "hermetic e2e: HEAVY still correct => exit 0")

        # 11. End-to-end, drifted => exit 1 (LOUD) and the issue names the offender.
        events.write_text(
            "\n".join(
                [
                    _event("b", "slow_a", 108.7),
                    _event("b", "slow_b", 61.2),
                    _event("b", "newly_slow", 55.0),
                ]
            )
        )
        rc = main(["--events", str(events), "--ci-yaml", str(ci), "--dry-run"])
        check(rc == 1, "hermetic e2e: drift => exit 1")
        _, body = render_issue(
            check_heavy_set(["slow_a", "slow_b"], {"slow_a": 1.0, "newly_slow": 55.0}),
            HEAVY_FLOOR_SECONDS,
        )
        check("newly_slow" in body and "slow_b" in body, "issue body names every offender")
        check(f"{KEY_PREFIX}: {ISSUE_KEY}" in body, "issue body carries the dedupe key")

        # 12. FAIL-LOUD e2e: a ci.yml with no HEAVY => exit 2, not a false all-clear. The
        #     decoy comment is present so this also proves prose alone cannot satisfy it.
        no_heavy = Path(td) / "no-heavy.yml"
        no_heavy.write_text(
            "name: ci\non: [push]\njobs:\n  test:\n    # add test(=its_name) to HEAVY\n"
            "    steps: []\n"
        )
        rc = main(["--events", str(events), "--ci-yaml", str(no_heavy), "--dry-run"])
        check(rc == 2, "missing HEAVY => fail-loud exit 2")

        # 13. FAIL-LOUD e2e: an unreadable event stream => exit 2.
        rc = main(["--events", str(Path(td) / "gone.jsonl"), "--ci-yaml", str(ci), "--dry-run"])
        check(rc == 2, "unreadable event stream => fail-loud exit 2")

    if failures:
        for f in failures:
            print(f"::error::heavy_set_alarm self-test FAILED: {f}")
        return 1
    print(
        "heavy_set_alarm self-test: UNDECLARED/MISSING/DEMOTED classification, the floor "
        "boundary + demotion hysteresis, libtest-json-plus parsing, both empty-measurement "
        "fail-louds and the e2e exit codes all behaved."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
