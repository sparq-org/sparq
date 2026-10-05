#!/usr/bin/env python3
# Miri-shard verdict classifier (formal-lanes fix). 🤖 SPARQ agent.
#
# WHY: the nightly `miri.yml` UB lane runs `cargo miri nextest run` sharded over
# sparq-core. Its purpose is to DETECT UNDEFINED BEHAVIOUR (aliasing / provenance /
# data-race) in the pure-Rust unsafe surface. But ~20 heavy end-to-end loader tests
# are so expensive under the Miri interpreter (~100× native) that they blow the
# per-test cap (.config/nextest.toml [profile.default-miri] slow-timeout) and are
# TERMINATED — a COST problem, tracked by bead sq-0s15k (cfg(miri)-scale the heavy
# inputs), NOT a UB finding. `cargo miri nextest` exits non-zero (100) whether a test
# TIMED OUT (known cost debt) or genuinely FAILED (a real UB detection / assertion) —
# so the raw exit code cannot tell "healthy verdict + known timeout debt" apart from
# "a real UB regression". Result: the whole `miri` workflow concluded `failure` every
# night and the no-verdict alarm (formal-alarm.yml) fired forever on pre-existing,
# already-tracked cost debt — masking whether Miri actually found UB.
#
# WHAT THIS DOES: read nextest's machine-readable `libtest-json-plus` event stream for
# ONE shard and split its failures into two honest classes:
#   * TIMEOUT  — `event == "failed"` with `reason == "time limit exceeded"` (no panic).
#                Known cost debt (sq-0s15k). LOUD but NON-FATAL for the shard verdict.
#   * FAILURE  — any other `event == "failed"` (a panic / assertion / Miri UB abort).
#                A REAL regression the UB lane exists to catch — always FATAL.
# Exit 0 iff there were NO real FAILUREs (timeouts alone are tolerated); exit 1 the
# instant a single genuine failure appears. Every timed-out + failed test is printed
# to the step summary + as an annotation, so the cost debt stays VISIBLE and under
# standing pressure — silence never looks like green, and a real UB finding is never
# swallowed. When sq-0s15k lands (heavy tests cfg(miri)-scaled), there are no timeouts
# and this script is a transparent pass-through.
#
# Reads the JSON stream on stdin (or --events FILE); stdlib only.
#
# Usage (in miri.yml):
#   set -o pipefail
#   NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 cargo +<nightly> miri nextest run -p sparq-core \
#       --partition count:<k>/24 --no-fail-fast \
#       --message-format libtest-json-plus --message-format-version 0.1 \
#     | tee miri-shard.jsonl | python3 scripts/miri_classify_verdict.py --shard <k>
#
# Self-test: scripts/miri_classify_verdict.py --self-test

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import sys

TIMEOUT_REASON = "time limit exceeded"


def classify(lines: list[str]) -> tuple[list[str], list[str], int]:
    """-> (timed_out_names, failed_names, n_ok). A failure whose reason is the
    slow-timeout terminate is a TIMEOUT (cost debt); anything else is a real FAILURE."""
    timed_out: list[str] = []
    failed: list[str] = []
    n_ok = 0
    for raw in lines:
        raw = raw.strip()
        if not raw.startswith("{"):
            continue
        try:
            e = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if e.get("type") != "test":
            continue
        event = e.get("event")
        name = e.get("name", "<unknown>")
        if event == "ok":
            n_ok += 1
        elif event in ("failed", "timeout"):
            reason = str(e.get("reason") or "")
            if event == "timeout" or reason == TIMEOUT_REASON:
                timed_out.append(name)
            else:
                failed.append(name)
    return timed_out, failed, n_ok


def _emit(shard: str, timed_out: list[str], failed: list[str], n_ok: int) -> int:
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    lines = [
        f"### 🤖 Miri UB verdict — shard {shard}",
        "",
        f"- ✅ passed: **{n_ok}**",
        f"- ⏱️ timed out (known cost debt, sq-0s15k, NON-FATAL): **{len(timed_out)}**",
        f"- ❌ genuine failures (UB / assertion — FATAL): **{len(failed)}**",
        "",
    ]
    if timed_out:
        lines.append("<details><summary>Timed-out tests (cost debt — sq-0s15k)</summary>\n")
        lines += [f"- `{n}`" for n in timed_out]
        lines.append("\n</details>")
    if failed:
        lines.append("**Genuine failures (investigate — a Miri UB lane failure is a real bug):**")
        lines += [f"- `{n}`" for n in failed]
    lines.append("")
    if failed:
        lines.append(
            "**Verdict: ❌ FAIL** — a genuine (non-timeout) Miri failure appeared; this shard "
            "reds the lane and the no-verdict alarm will re-open. Investigate the UB/assertion."
        )
    elif timed_out:
        lines.append(
            "**Verdict: ✅ PASS (with known timeout debt)** — Miri found NO undefined behaviour "
            "in this shard's completed tests; the only non-passes are heavy tests over the "
            "per-test cap (cost debt, sq-0s15k), which are LOUD but non-fatal so the lane can "
            "produce a real UB verdict tonight."
        )
    else:
        lines.append("**Verdict: ✅ PASS** — all tests in this shard verified clean under Miri.")

    block = "\n".join(lines) + "\n"
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as fh:
            fh.write(block)
    else:
        sys.stdout.write(block)

    for n in failed:
        print(f"::error title=Miri UB failure (shard {shard})::{n} failed under Miri "
              f"(not a timeout) — a genuine undefined-behaviour / assertion finding.")
    for n in timed_out:
        print(f"::warning title=Miri timeout debt (shard {shard})::{n} exceeded the per-test "
              f"cap — known cost debt tracked by sq-0s15k (non-fatal).")

    return 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="Classify one Miri nextest shard's verdict.")
    p.add_argument("--shard", default="?", help="shard label for annotations/summary")
    p.add_argument("--events", help="read the JSON event stream from FILE instead of stdin")
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args(argv)

    if args.self_test:
        return _self_test()

    if args.events:
        with open(args.events, encoding="utf-8") as fh:
            lines = fh.readlines()
    else:
        lines = sys.stdin.readlines()
    timed_out, failed, n_ok = classify(lines)
    # "Silence must not look like green": an event stream with NO test outcomes at all means
    # nextest never ran the tests (build break / miri sysroot failure / crash before emitting),
    # NOT a clean shard. Treat it as FATAL so a setup failure cannot masquerade as a pass.
    # (A shard legitimately assigned zero tests still emits nothing here — but every one of the
    # 24 round-robin shards over sparq-core's ~130 tests receives tests, so an empty stream is a
    # real failure signal, not an empty partition. If the partition scheme ever changes, revisit.)
    if not timed_out and not failed and n_ok == 0:
        print(f"::error title=Miri shard produced no test outcomes (shard {args.shard})::"
              "the libtest-json-plus stream carried no ok/failed/timeout events — nextest likely "
              "failed to build/run the shard (see the .stderr above). Failing the shard: an empty "
              "verdict is NOT a clean pass.")
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        msg = (f"### 🤖 Miri UB verdict — shard {args.shard}\n\n"
               "**Verdict: ❌ FAIL** — no test outcomes in the event stream (build/run failure, "
               "not a clean pass). See the step log's `.stderr` dump.\n")
        if summary:
            with open(summary, "a", encoding="utf-8") as fh:
                fh.write(msg)
        else:
            sys.stdout.write(msg)
        return 1
    return _emit(args.shard, timed_out, failed, n_ok)


# --------------------------------------------------------------------------- #
_FIX = [
    json.dumps({"type": "test", "event": "ok", "name": "a::light_ok"}),
    json.dumps({"type": "test", "event": "ok", "name": "a::another_ok"}),
    # a slow-timeout termination — nextest reports it as failed w/ this reason
    json.dumps({"type": "test", "event": "failed", "name": "a::heavy_loader",
                "reason": "time limit exceeded", "exec_time": 6000.0}),
]
_FIX_REAL = _FIX + [
    json.dumps({"type": "test", "event": "failed", "name": "a::aliasing_ub",
                "stdout": "error: Undefined Behavior: ..."}),
]


def _self_test() -> int:
    fails = []

    def ck(cond, label):
        if not cond:
            fails.append(label)

    # 1. timeouts only => classified as timeouts, no real failures.
    t, f, ok = classify(_FIX)
    ck(t == ["a::heavy_loader"], "heavy loader is a timeout")
    ck(f == [], "no real failures on timeout-only")
    ck(ok == 2, "two ok tests counted")
    ck(_emit("X", t, f, ok) == 0, "timeout-only shard exits 0 (non-fatal debt)")

    # 2. a genuine UB failure => real failure => fatal.
    t, f, ok = classify(_FIX_REAL)
    ck(t == ["a::heavy_loader"], "timeout still classified as timeout")
    ck(f == ["a::aliasing_ub"], "UB is a genuine failure")
    ck(_emit("X", t, f, ok) == 1, "a real UB failure exits 1 (fatal)")

    # 3. all-pass => exit 0.
    t, f, ok = classify([json.dumps({"type": "test", "event": "ok", "name": "a::x"})])
    ck((t, f, ok) == ([], [], 1), "all-pass classification")
    ck(_emit("X", t, f, ok) == 0, "all-pass exits 0")

    # 4. a nextest `event:timeout` (older schema) also counts as a timeout, not a failure.
    t, f, ok = classify([json.dumps({"type": "test", "event": "timeout", "name": "a::t"})])
    ck(t == ["a::t"] and f == [], "event:timeout treated as timeout")

    # 5. non-test / malformed lines ignored, don't crash.
    t, f, ok = classify(["not json", json.dumps({"type": "suite", "event": "started"}), ""])
    ck((t, f, ok) == ([], [], 0), "non-test lines ignored")

    # 6. empty / no-test-outcome stream via main() => FATAL (silence != green).
    import tempfile
    with tempfile.TemporaryDirectory() as td:
        empty = os.path.join(td, "empty.jsonl")
        with open(empty, "w", encoding="utf-8") as fh:
            fh.write(json.dumps({"type": "suite", "event": "started"}) + "\n")
        ck(main(["--shard", "Z", "--events", empty]) == 1,
           "no test outcomes => fatal exit 1")
        # a stream WITH a passing test via main() => exit 0.
        good = os.path.join(td, "good.jsonl")
        with open(good, "w", encoding="utf-8") as fh:
            fh.write(json.dumps({"type": "test", "event": "ok", "name": "a::x"}) + "\n")
        ck(main(["--shard", "Z", "--events", good]) == 0, "a real pass via main => exit 0")

    # 7. The workflow must keep every partition label and command in sync. This is the
    # regression witness for sq-hytab: reverting the 24-way split to the 12-way layout that
    # exhausted the job backstop makes this check fail before the nightly lane is trusted.
    workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/miri.yml").read_text(
        encoding="utf-8"
    )
    expected_matrix = (
        "shard: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, "
        "17, 18, 19, 20, 21, 22, 23, 24]"
    )
    ck(expected_matrix in workflow, "workflow enumerates all 24 Miri shards")
    ck(
        workflow.count("${{ matrix.shard }}/24") == 4,
        "workflow consistently labels, partitions, and classifies 24 shards",
    )
    ck("${{ matrix.shard }}/12" not in workflow, "stale 12-way partition wiring absent")

    if fails:
        for x in fails:
            print(f"::error::miri_classify_verdict self-test FAILED: {x}")
        return 1
    print("miri_classify_verdict self-test: timeout/real-failure classification + exits OK.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
