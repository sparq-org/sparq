#!/usr/bin/env python3
# [OPUS-5] sq-6vshe.15 item 3 (issue #5164) — the VERDICT half of the sccache A/B.
#
# WHAT THIS DECIDES, AND WHAT IT REFUSES TO DECIDE
# -----------------------------------------------
# `research/ci-mergequeue-speedup-2026-07.md` §3.2 lever 2 item 3 asks for an A/B of
# `sccache` (GitHub Actions cache backend) on ci.yml's `build + archive test binaries`
# job, with ONE adoption rule: adopt iff the MEDIAN win is at least the bar
# (60 s, `--bar-seconds`). Anything less is a NEGATIVE RESULT to be recorded in the
# design record and closed. Both outcomes are complete deliverables; neither is a CI
# failure, so both exit 0.
#
# The dangerous third outcome is the one this script exists to make impossible:
# a BROKEN INSTRUMENT reporting a false negative. The expected physics of this
# experiment is a small win — the design record says so up front (deps are already
# warm via Swatinem/rust-cache, a PR's changed crates always miss, feature-matrix legs
# key differently per cfg, so only unchanged workspace crates can hit). "No win" and
# "sccache never actually cached anything" produce the SAME wall-clock table and
# differ only in the sccache counters. If the harness mis-wires the GHA backend, or
# the prime job fails to populate it, or `CARGO_INCREMENTAL` is non-zero (sccache
# refuses to cache incremental compilation outright — MEASURED while authoring this:
# every request lands in `not_cached: {"incremental": N}` with zero hits AND zero
# misses), the treatment arm degrades to "control plus wrapper overhead" and a naive
# reader records a permanent, wrong "sccache does not help this repo" in a research
# record nobody re-measures. So:
#
#   * treatment records MUST carry sccache counters showing real cacheable traffic,
#     and the arm MUST show at least one cache HIT overall — otherwise the run is
#     INCONCLUSIVE (exit 2), never a negative result;
#   * control records MUST carry no sccache counters at all — a control arm that
#     silently inherited RUSTC_WRAPPER is a contaminated comparison (exit 2);
#   * a missing/short/lopsided trial set is a broken run (exit 2), not a median over
#     whatever happened to survive.
#
# Exit codes: 0 = a verdict was reached (ADOPT or DO-NOT-ADOPT); 2 = INCONCLUSIVE,
# the instrument did not earn the right to a verdict; 1 = `--self-test` found a
# regression in the guards above. NOTE: argparse's own usage errors also exit 2, so
# 2 is precisely "no verdict was produced" rather than "the data was bad" — the two
# are distinguished by the report on stdout, and both correctly deny a verdict, which
# is the property that matters.
#
# INPUT: a directory of per-(arm, trial) JSON files written by
# .github/workflows/sccache-ab.yml. Schema per file:
#   {"arm": "control"|"treatment", "trial": <int>, "compile_seconds": <number>,
#    "sccache": null | {"compile_requests": int, "cache_hits": int,
#                       "cache_misses": int, "cache_writes": int,
#                       "not_cached": {str: int}},
#    ... free-form provenance keys (rustc, runner, run_id, commit, key_namespace) }
#
# Usage:
#   sccache_ab_verdict.py --results-dir DIR [--bar-seconds 60]
#   sccache_ab_verdict.py --self-test
# stdlib-only (no PyYAML, no network, no gh) so it runs anywhere.

from __future__ import annotations

import argparse
import json
import os
import statistics
import sys
from pathlib import Path

# The adoption bar from research/ci-mergequeue-speedup-2026-07.md §3.2 item 3. This is
# a POLICY threshold set before any measurement, not a measured figure — that is the
# whole point of a measure-first bead: the bar cannot be moved to fit the result.
DEFAULT_BAR_SECONDS = 60.0

# A median over fewer than this many trials is noise on shared runners. The workflow
# ships 5 paired trials; this is the floor below which the script refuses a verdict.
MIN_TRIALS_PER_ARM = 3

ARMS = ("control", "treatment")


class Inconclusive(Exception):
    """The instrument did not earn the right to a verdict (exit 2)."""


def load_records(results_dir: Path) -> list[dict]:
    """Read every *.json under results_dir (one level of nesting tolerated).

    download-artifact places each artifact in its own subdirectory when several are
    downloaded at once, so this globs recursively rather than assuming a flat dir.
    """
    files = sorted(results_dir.rglob("*.json"))
    if not files:
        raise Inconclusive(
            f"no result JSON under {results_dir} — every measure job failed to upload, "
            "so there is nothing to compare. Fix the harness and re-dispatch."
        )
    records = []
    for f in files:
        try:
            rec = json.loads(f.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, UnicodeDecodeError) as exc:
            raise Inconclusive(f"{f}: unparseable result file ({exc})") from exc
        if not isinstance(rec, dict):
            raise Inconclusive(f"{f}: result file is not a JSON object")
        for field in ("arm", "trial", "compile_seconds"):
            if field not in rec:
                raise Inconclusive(f"{f}: result file is missing required field {field!r}")
        if rec["arm"] not in ARMS:
            raise Inconclusive(f"{f}: unknown arm {rec['arm']!r} (expected one of {ARMS})")
        if not isinstance(rec["compile_seconds"], (int, float)) or rec["compile_seconds"] <= 0:
            raise Inconclusive(
                f"{f}: compile_seconds must be a positive number, got {rec['compile_seconds']!r}"
            )
        rec["_source"] = str(f)
        records.append(rec)
    return records


def check_arm_shape(by_arm: dict[str, list[dict]]) -> None:
    """Both arms present, deep enough, and equally deep. A lopsided set is not a median."""
    for arm in ARMS:
        if not by_arm.get(arm):
            raise Inconclusive(
                f"arm {arm!r} produced no results — an A/B with one arm is not a comparison."
            )
    counts = {arm: len(by_arm[arm]) for arm in ARMS}
    short = [f"{arm}={n}" for arm, n in counts.items() if n < MIN_TRIALS_PER_ARM]
    if short:
        raise Inconclusive(
            f"fewer than {MIN_TRIALS_PER_ARM} trials in {', '.join(short)} — a median over "
            "that few runs on shared runners is noise, not a measurement."
        )
    if counts["control"] != counts["treatment"]:
        raise Inconclusive(
            f"unequal trial counts (control={counts['control']}, "
            f"treatment={counts['treatment']}); some jobs died, so the arms saw different "
            "runner populations and the medians are not comparable."
        )


def sccache_totals(records: list[dict]) -> dict[str, int]:
    total = {"compile_requests": 0, "cache_hits": 0, "cache_misses": 0, "cache_writes": 0}
    for rec in records:
        stats = rec.get("sccache") or {}
        for key in total:
            total[key] += int(stats.get(key, 0) or 0)
    return total


def check_instrument(by_arm: dict[str, list[dict]]) -> dict[str, int]:
    """The heart of this script: prove the treatment arm actually used sccache.

    Returns the treatment arm's summed sccache counters.
    """
    contaminated = [r["_source"] for r in by_arm["control"] if r.get("sccache")]
    if contaminated:
        raise Inconclusive(
            "control arm reported sccache counters — RUSTC_WRAPPER leaked into the "
            f"baseline, so this is not an A/B: {', '.join(contaminated)}"
        )

    missing = [r["_source"] for r in by_arm["treatment"] if not r.get("sccache")]
    if missing:
        raise Inconclusive(
            "treatment arm produced no sccache counters — sccache was not running (or "
            f"`--show-stats` failed), so its wall-clock says nothing: {', '.join(missing)}"
        )

    totals = sccache_totals(by_arm["treatment"])
    if totals["compile_requests"] <= 0:
        raise Inconclusive(
            "treatment arm saw zero sccache compile requests — the wrapper was installed "
            "but never invoked, so the arm is control with extra steps."
        )
    cacheable = totals["cache_hits"] + totals["cache_misses"]
    if cacheable <= 0:
        not_cached = {}
        for rec in by_arm["treatment"]:
            for reason, n in ((rec.get("sccache") or {}).get("not_cached") or {}).items():
                not_cached[reason] = not_cached.get(reason, 0) + int(n)
        raise Inconclusive(
            "treatment arm cached NOTHING: every one of "
            f"{totals['compile_requests']} compile requests was rejected as non-cacheable "
            f"(reasons: {not_cached or 'unreported'}). `incremental` here means "
            "CARGO_INCREMENTAL is not 0 — sccache cannot cache incremental compilation. "
            "This is a harness defect, NOT evidence that sccache does not help."
        )
    if totals["cache_hits"] <= 0:
        raise Inconclusive(
            f"treatment arm recorded {totals['cache_misses']} cacheable misses and ZERO "
            "hits — the prime job did not populate the GHA backend that the measure jobs "
            "read (check SCCACHE_GHA_VERSION agreement and Actions cache scoping). "
            "Measuring a 100%-miss arm answers a question nobody asked."
        )
    return totals


def render(by_arm: dict[str, list[dict]], totals: dict[str, int], bar: float) -> tuple[str, bool]:
    """Build the markdown report. Returns (report, adopt)."""
    med = {arm: statistics.median(r["compile_seconds"] for r in by_arm[arm]) for arm in ARMS}
    delta = med["control"] - med["treatment"]
    adopt = delta >= bar

    cacheable = totals["cache_hits"] + totals["cache_misses"]
    hit_rate = 100.0 * totals["cache_hits"] / cacheable if cacheable else 0.0

    lines = [
        "## sccache (GHA backend) A/B — build-archive compile step",
        "",
        f"Adoption bar: **>= {bar:g} s median win** "
        "(research/ci-mergequeue-speedup-2026-07.md §3.2 lever 2 item 3).",
        "",
        "| arm | trials | median compile (s) | min | max |",
        "| --- | --- | --- | --- | --- |",
    ]
    for arm in ARMS:
        secs = sorted(r["compile_seconds"] for r in by_arm[arm])
        lines.append(
            f"| {arm} | {len(secs)} | {med[arm]:.1f} | {secs[0]:.1f} | {secs[-1]:.1f} |"
        )
    lines += [
        "",
        f"**Median win: {delta:.1f} s** (control - treatment).",
        "",
        "sccache counters, treatment arm summed over trials: "
        f"{totals['compile_requests']} compile requests, {totals['cache_hits']} hits, "
        f"{totals['cache_misses']} misses, {totals['cache_writes']} writes "
        f"({hit_rate:.1f}% hit rate over cacheable requests).",
        "",
    ]
    if adopt:
        lines += [
            f"### VERDICT: ADOPT — the {delta:.1f} s median win clears the {bar:g} s bar.",
            "",
            "Next step is a real change to `.github/workflows/ci.yml`, which this "
            "dispatch-only harness deliberately does not make. That change must carry the "
            "sq-6vshe.5 cache-poisoning discipline (no key aliasing across toolchain / "
            "Cargo.lock / feature-set; PR contexts read-only) and re-run this A/B after "
            "landing to confirm the win survives contact with the real queue.",
        ]
    else:
        lines += [
            f"### VERDICT: DO NOT ADOPT — the {delta:.1f} s median win does not clear "
            f"the {bar:g} s bar.",
            "",
            "This is the outcome the design record predicted, and it is a COMPLETE result: "
            "record it in research/ci-mergequeue-speedup-2026-07.md §3.2 and close the item. "
            f"The {hit_rate:.1f}% hit rate is the explanation, not an excuse — with "
            "dependencies already warm via Swatinem/rust-cache, only unchanged workspace "
            "crates can hit, and that population is small.",
        ]
    return "\n".join(lines) + "\n", adopt


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--results-dir", type=Path, help="directory of per-trial result JSON")
    ap.add_argument(
        "--bar-seconds",
        type=float,
        default=DEFAULT_BAR_SECONDS,
        help=f"median-win adoption bar in seconds (default {DEFAULT_BAR_SECONDS:g})",
    )
    ap.add_argument("--self-test", action="store_true", help="run the hermetic self-test")
    args = ap.parse_args(argv)

    if args.self_test:
        return _self_test()
    if not args.results_dir:
        ap.error("--results-dir is required (or pass --self-test)")

    try:
        records = load_records(args.results_dir)
        by_arm: dict[str, list[dict]] = {arm: [] for arm in ARMS}
        for rec in records:
            by_arm[rec["arm"]].append(rec)
        check_arm_shape(by_arm)
        totals = check_instrument(by_arm)
    except Inconclusive as exc:
        report = (
            "## sccache A/B — INCONCLUSIVE\n\n"
            f"The run did not produce a comparable measurement:\n\n> {exc}\n\n"
            "**No verdict may be recorded from this run** — in particular, do NOT write a "
            "negative result into research/ci-mergequeue-speedup-2026-07.md on the strength "
            "of it. Fix the harness and re-dispatch.\n"
        )
        _emit(report)
        return 2

    report, _adopt = render(by_arm, totals, args.bar_seconds)
    _emit(report)
    return 0


def _emit(report: str) -> None:
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(report)


# ---------------------------------------------------------------------------
# Hermetic self-test — the mutation tripwire for the checks above.
# ---------------------------------------------------------------------------

def _rec(arm: str, trial: int, secs: float, **sccache_kw) -> dict:
    rec = {"arm": arm, "trial": trial, "compile_seconds": secs, "sccache": None}
    if arm == "treatment":
        stats = {
            "compile_requests": 400,
            "cache_hits": 120,
            "cache_misses": 260,
            "cache_writes": 260,
            "not_cached": {},
        }
        stats.update(sccache_kw)
        rec["sccache"] = stats
    return rec


def _self_test() -> int:
    import tempfile

    failures: list[str] = []

    def run(label: str, records: list[dict], expect_exit: int, expect_in: str = "") -> None:
        with tempfile.TemporaryDirectory() as td:
            for i, rec in enumerate(records):
                (Path(td) / f"r{i}.json").write_text(json.dumps(rec), encoding="utf-8")
            import contextlib
            import io

            buf = io.StringIO()
            env = os.environ.pop("GITHUB_STEP_SUMMARY", None)
            try:
                with contextlib.redirect_stdout(buf):
                    got = main(["--results-dir", td])
            finally:
                if env is not None:
                    os.environ["GITHUB_STEP_SUMMARY"] = env
            out = buf.getvalue()
            if got != expect_exit:
                failures.append(f"{label}: exit {got}, expected {expect_exit}\n{out}")
            elif expect_in and expect_in not in out:
                failures.append(f"{label}: output missing {expect_in!r}\n{out}")

    def paired(control_secs: list[float], treat_secs: list[float], **kw) -> list[dict]:
        return [_rec("control", i, s) for i, s in enumerate(control_secs)] + [
            _rec("treatment", i, s, **kw) for i, s in enumerate(treat_secs)
        ]

    # A clear win clears the bar; a small win does not. Same shape, different verdict —
    # this is what proves the bar is actually consulted rather than decorative.
    run("adopt", paired([200, 205, 210], [100, 105, 110]), 0, "VERDICT: ADOPT")
    run("negative", paired([200, 205, 210], [190, 195, 200]), 0, "VERDICT: DO NOT ADOPT")
    # A win that lands exactly ON the bar adopts (>=, not >).
    run("adopt-at-bar", paired([200, 200, 200], [140, 140, 140]), 0, "VERDICT: ADOPT")

    # Every instrument failure must be INCONCLUSIVE (2), never a verdict.
    run("no-results", [], 2, "no result JSON")
    run("one-armed", [_rec("control", i, 200) for i in range(3)], 2, "produced no results")
    run("too-short", paired([200, 205], [100, 105]), 2, "fewer than 3 trials")
    run("lopsided", paired([200, 205, 210, 215], [100, 105, 110]), 2, "unequal trial counts")
    run("all-non-cacheable", paired(
        [200, 205, 210], [200, 205, 210],
        compile_requests=400, cache_hits=0, cache_misses=0, cache_writes=0,
        not_cached={"incremental": 400},
    ), 2, "cached NOTHING")
    run("zero-hits", paired(
        [200, 205, 210], [200, 205, 210],
        compile_requests=400, cache_hits=0, cache_misses=400, cache_writes=400,
    ), 2, "ZERO")
    run("no-counters", [_rec("control", i, 200) for i in range(3)]
        + [{"arm": "treatment", "trial": i, "compile_seconds": 100, "sccache": None}
           for i in range(3)], 2, "no sccache counters")

    contaminated = paired([200, 205, 210], [100, 105, 110])
    contaminated[0]["sccache"] = {"compile_requests": 1, "cache_hits": 1,
                                  "cache_misses": 0, "cache_writes": 0}
    run("contaminated-control", contaminated, 2, "RUSTC_WRAPPER leaked")

    bad = paired([200, 205, 210], [100, 105, 110])
    bad[0].pop("compile_seconds")
    run("missing-field", bad, 2, "missing required field")

    bad2 = paired([200, 205, 210], [100, 105, 110])
    bad2[0]["compile_seconds"] = 0
    run("non-positive-seconds", bad2, 2, "positive number")

    if failures:
        print("SELF-TEST FAILURES:\n" + "\n\n".join(failures), file=sys.stderr)
        return 1
    print("sccache_ab_verdict self-test: all clear")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
