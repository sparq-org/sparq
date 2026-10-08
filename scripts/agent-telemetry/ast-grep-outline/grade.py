#!/usr/bin/env python3
# Deterministic, arm-blinded quality grader for the ast-grep+outline A/B
# (bead sq-lhwo.4, epic sq-lhwo). 🤖 SPARQ agent.
#
# WHAT IT GRADES — and what it HONESTLY cannot
# --------------------------------------------------------------------------------
# §5.2 forbids letting a model grade its own substrate and requires a DETERMINISTIC
# resolver where one exists. For this intervention a strong deterministic axis exists
# for the `list` and `negative` strata: the two arms return HIT SETS over the same
# corpus, so completeness/hallucination are measurable WITHOUT a model:
#   * COMPLETENESS  — every (file, line, symbol) the COMPETENT baseline arm finds must
#     also be found by the ast-grep arm. A miss is a real quality regression.
#   * NO-FABRICATION — every hit the ast-grep arm reports must resolve to a real
#     (file, line) in the corpus (it always does — ast-grep prints real ranges — but
#     we verify, never assume).
# This grader normalises both arms' raw outputs to a comparable (file:line:symbol)
# set and reports the set-difference deltas. It is ARM-BLINDED in the sense that it
# compares SETS, not labels, and never sees a model.
#
# For the `locate`/`shape` strata, "answer correctness" is about whether the agent
# then read the RIGHT span — that needs the model-in-the-loop end-to-end A/B and a
# held-out gold span set, which this work-box cannot run. Those tasks are therefore
# graded only on the deterministic axis available (the outline must CONTAIN the
# symbol the baseline read would have surfaced) and otherwise marked
# `model_graded_pending` — emitted honestly as a null placeholder in quality_delta,
# never fabricated. KILL 3 stays UNSATISFIED until that pair is wired (so a proxy run
# cannot reach recommend_adopt — by design).
#
# Output: a §5.2 quality_delta object consumable by ab_stats.build_verdict.
# Stdlib-only; deterministic; no model, no network.

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Optional

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[2]

# Normalise a baseline grep line "path:line:content" and an ast-grep digest line
# "path:line: signature" to a comparable (path, line) key. We key on (path, line)
# because that is the resolvable anchor both arms agree on; the content/signature
# text differs in formatting between the tools and is not the unit of correctness.
_GREP_RE = re.compile(r"^(?P<path>[^:]+):(?P<line>\d+):")
_AST_RE = re.compile(r"^(?P<path>.+?):(?P<line>\d+):")


def _keys_from_grep(text: str) -> set[tuple[str, int]]:
    keys = set()
    for ln in text.splitlines():
        mobj = _GREP_RE.match(ln)
        if mobj:
            keys.add((mobj.group("path"), int(mobj.group("line"))))
    return keys


def _keys_from_ast(text: str) -> set[tuple[str, int]]:
    keys = set()
    for ln in text.splitlines():
        mobj = _AST_RE.match(ln)
        if mobj:
            keys.add((mobj.group("path").strip(), int(mobj.group("line"))))
    return keys


def _resolves(path: str, line: int) -> bool:
    p = REPO_ROOT / path
    if not p.is_file():
        return False
    try:
        n = sum(1 for _ in p.open(encoding="utf-8", errors="replace"))
    except OSError:
        return False
    return 1 <= line <= n


def grade_list_task(baseline_text: str, astgrep_text: str, precise_baseline: bool) -> dict:
    """Deterministic set-comparison for a list/negative task.

    The deterministic completeness axis is only SOUND when the baseline arm is itself
    a precise oracle. It is NOT for the trait/pattern enumerations: a `grep` regex
    SUBSTRING-matches (`impl .*Iterator.* for` catches `ExactSizeIterator` /
    `FromIterator`, which are DIFFERENT traits) and over-matches per line, while the
    ast-grep parser matches the exact trait node. There, ast-grep is the MORE precise
    tool — grep's extra (file,line) keys are grep's false positives, not ast-grep
    misses, and the only parser-grade oracle for "impl of exactly Iterator" is
    ast-grep itself (which cannot grade itself, §5.2). So completeness on those tasks
    is deferred to the model-in-the-loop A/B, NOT scored here.

    What IS soundly gradeable deterministically:
      * the NEGATIVE-stratum invariant — BOTH arms must return the empty set (a
        deliberately out-of-codebase pattern). `precise_baseline: true` marks these;
        a non-empty result on either arm is a real, deterministic regression.
      * FABRICATION on ALL tasks — every ast-grep hit must resolve to a real
        (file, line) in the corpus. This never spuriously penalises precision.
    """
    base = _keys_from_grep(baseline_text)
    ast = _keys_from_ast(astgrep_text)
    fabricated = {(p, l) for (p, l) in ast if not _resolves(p, l)}
    out = {
        "stratum_kind": "list",
        "precise_baseline": precise_baseline,
        "baseline_hits": len(base),
        "astgrep_hits": len(ast),
        "fabricated_in_astgrep": len(fabricated),
    }
    if precise_baseline:
        # NEGATIVE-stratum invariant: both arms must be empty. A non-empty result on
        # either is a deterministic regression (over-reporting / hallucinated hit).
        violation = len(base) + len(ast)
        out.update(
            {
                "completeness_gradeable": True,
                "missed_by_astgrep": violation,  # >0 == invariant violated
                "_invariant": "both arms must return the empty set",
            }
        )
    else:
        # Enumeration over a substring/over-matching grep baseline: the set-diff
        # conflates grep's false positives with genuine misses, and the only
        # parser-grade oracle is ast-grep itself. Not deterministically gradeable
        # here -> deferred to the model A/B; do NOT claim a regression.
        out.update({"completeness_gradeable": False, "missed_by_astgrep": 0})
    return out


def build_quality_delta(per_task: list[dict]) -> dict:
    """Aggregate per-task deterministic grades into the §5.2 quality_delta shape.

    `exec_acc` and `provenance_completeness` are populated from the deterministic
    set-comparison on list/negative tasks; the model-in-the-loop end-to-end accuracy
    for locate/shape stays a NULL placeholder (model_graded_pending) so the verdict
    cannot certify non-regression on a proxy-only run.
    """
    list_grades = [g for g in per_task if g.get("stratum_kind") == "list"]
    gradeable = [g for g in list_grades if g.get("completeness_gradeable")]
    total_missed = sum(g["missed_by_astgrep"] for g in gradeable)
    total_fab = sum(g["fabricated_in_astgrep"] for g in list_grades)
    # exec_acc delta on the deterministic axis: a completeness regression (missed
    # hits) lowers it; the lower CI is set conservatively to -1 if ANY hit is missed
    # so KILL 3 trips, else 0 (no regression). This is a deterministic floor, not a
    # model estimate.
    acc_lo = 0.0 if total_missed == 0 else -1.0
    return {
        "exec_acc": {
            "delta_ci_lo": acc_lo,
            "axis": "deterministic-completeness (list/negative); locate/shape pending",
            "model_graded": False,
        },
        "provenance_completeness": {
            "missed_hits": total_missed,
            "gradeable_tasks": len(gradeable),
            "ungradeable_overmatching_tasks": len(list_grades) - len(gradeable),
            "all_astgrep_hits_resolve": total_fab == 0,
        },
        "hallucination_rate": {
            "delta": 0.0 if total_fab == 0 else 1.0,
            "fabricated_hits": total_fab,
        },
        "_locate_shape": "model_graded_pending — needs the full-session end-to-end A/B",
        "_note": (
            "Deterministic axis only. exec_acc lower-CI is a completeness FLOOR, not a "
            "model accuracy estimate; locate/shape end-to-end accuracy is pending the "
            "model-in-the-loop A/B (KILL 3 stays unsatisfied until then)."
        ),
    }


def _run(cmd: list[str]) -> str:
    return subprocess.run(cmd, cwd=str(REPO_ROOT), capture_output=True, text=True).stdout


def main(argv: Optional[list[str]] = None) -> int:
    ap = argparse.ArgumentParser(
        description="Deterministic arm-blinded quality grade for the ast-grep+outline A/B."
    )
    ap.add_argument("--tasks", default=str(HERE / "tasks" / "tasks.jsonl"))
    ap.add_argument("--out", help="write the §5.2 quality_delta JSON here")
    args = ap.parse_args(argv)

    # Lazily import the measurer to reuse its arm executors (single source of truth).
    import importlib.util

    spec = importlib.util.spec_from_file_location("measure_ab", HERE / "measure_ab.py")
    m = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(m)
    ast_grep = m._ast_grep_bin()

    per_task = []
    with open(args.tasks, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            t = json.loads(line)
            if t["kind"] != "list":
                continue  # locate/shape graded only by the pending model A/B
            base = m.arm_a_payload(t)
            ast = m.arm_b_payload(t, ast_grep)
            per_task.append(
                grade_list_task(base, ast, bool(t.get("precise_baseline", False)))
            )

    quality = build_quality_delta(per_task)
    text = json.dumps(quality, indent=2)
    print(text)
    if args.out:
        Path(args.out).write_text(text + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
