#!/usr/bin/env python3
# [OPUS-4.8] sq-ve5dy / sq-zbyo7 (epic sq-2m6zm). 🤖 SPARQ agent — 3-arm
# $-weighted verdict for the PKG token A/B. Written while Fable unavailable; flag
# for re-review when Fable returns.
#
# THE DECISION METRIC IS MODEL-PRICE-WEIGHTED COST, not raw tokens (sq-ve5dy:
# "the decision metric must be MODEL-PRICE-WEIGHTED cost"). Arm C runs the verbose
# NL->SPARQL->run->NL middle on a model ~15x cheaper than the Opus arms, so a raw-
# token comparison would understate its win. $ = eff_input*price_in + output*price_out.
#
#   Arm A  = Opus reads the source docs to answer.
#   Arm B  = Opus runs pkg-query (introspect -> ground -> ask) itself.
#   Arm C  = a model:haiku sub-agent runs pkg-query as an NL tool; Opus only
#            emits the question + reads the NL answer.
#
# Tokens come from tokens_real.py over the real per-(task,arm) transcripts (NO char
# proxy). Quality is graded against the frozen tasks' gold keys (gold-key coverage;
# negative-stratum tasks graded by an honest-abstention regex).
#
# Usage:
#   analyze3.py --tasks tasks/abm_tasks.json \
#               --tokens tok_AB.json tok_C.json \
#               --answers answers.json [answers2.json ...] \
#               [--out verdict.json]
#
# --answers files are {(task,arm) -> answer} maps OR lists of
# {"task","arm","answer"} records (the workflow result rows). Both shapes accepted.

from __future__ import annotations

import argparse
import json
import re
import statistics as st
import sys
from collections import defaultdict

# List-price approximations (USD per Mtok). The RATIO between the model families is
# the load-bearing point; the absolute prices are stated openly and are easy to
# re-point if list prices move. (input, output)
PRICE = {
    "opus": (15.0, 75.0),
    "haiku": (1.0, 5.0),
}
ARM_MODEL = {"A": "opus", "B": "opus", "C": "haiku"}
ARM_NAME = {
    "A": "A=Opus read-docs",
    "B": "B=Opus pkg-query",
    "C": "C=Haiku pkg-query (NL-tool)",
}
STRATA = ("point-lookup", "multi-hop", "synthesis", "negative")

# A negative-stratum task is answered correctly iff the agent honestly abstained /
# reported the empty result, rather than inventing rows.
NEG_RE = re.compile(
    r"\b(none|no |0 row|zero|empty|nothing|not (found|exist|in|present)|"
    r"does not|doesn'?t|deliberately)\b",
    re.I,
)


def load_tokens(paths: list[str]) -> dict[tuple[str, str], dict]:
    tok: dict[tuple[str, str], dict] = {}
    for p in paths:
        with open(p, encoding="utf-8") as fh:
            rows = json.load(fh)
        for r in rows:
            if r.get("task") and r.get("arm"):
                tok[(r["task"], r["arm"])] = r
    return tok


def load_answers(paths: list[str]) -> dict[tuple[str, str], str]:
    """Accept either a {(task,arm)->answer} map (encoded as "task|arm" keys) or a
    list of {"task","arm","answer"} records (the raw workflow result rows)."""
    answers: dict[tuple[str, str], str] = {}
    for p in paths:
        with open(p, encoding="utf-8") as fh:
            data = json.load(fh)
        if isinstance(data, dict):
            # Possibly nested under result.results (a raw workflow .output dump).
            recs = data.get("result", {}).get("results") if "result" in data else None
            if recs is None:
                for k, v in data.items():
                    t, _, a = k.partition("|")
                    answers[(t, a)] = v
                continue
            data = recs
        for r in data:
            answers[(r["task"], r["arm"])] = r.get("answer")
    return answers


def grade(task: dict, ans: str | None) -> float:
    if not ans:
        return 0.0
    if task["stratum"] == "negative":
        return 1.0 if NEG_RE.search(ans) else 0.0
    gold = task["gold_keys"]
    if not gold:
        return 1.0
    al = ans.lower()
    return sum(1 for k in gold if k.lower() in al) / len(gold)


def dollars(arm: str, r: dict) -> float:
    pi, po = PRICE[ARM_MODEL[arm]]
    return r["eff_input_tokens"] / 1e6 * pi + r.get("output_tokens", 0) / 1e6 * po


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tasks", required=True, help="frozen tasks JSON (abm_tasks.json)")
    ap.add_argument("--tokens", nargs="+", required=True, help="tokens_real.py output JSON(s)")
    ap.add_argument("--answers", nargs="+", required=True, help="answer map/record JSON(s)")
    ap.add_argument("--out", default=None, help="write the verdict object here")
    args = ap.parse_args(argv[1:])

    tasks = {t["id"]: t for t in json.load(open(args.tasks, encoding="utf-8"))}
    tok = load_tokens(args.tokens)
    answers = load_answers(args.answers)

    # arm -> list of (task, eff_in, out, $, quality)
    rows: dict[str, list[tuple]] = defaultdict(list)
    for (t, arm), r in tok.items():
        if t not in tasks:
            continue
        rows[arm].append(
            (t, r["eff_input_tokens"], r.get("output_tokens", 0), dollars(arm, r),
             grade(tasks[t], answers.get((t, arm))))
        )

    print("=== 3-ARM VERDICT (N=%d tasks, real cache-discounted tokens) ==="
          % len(tasks))
    print("prices/Mtok (list approx): Opus $15 in/$75 out  Haiku $1 in/$5 out\n")
    print(f"{'arm':30} {'med eff_in':>11} {'med out':>8} {'total $':>9} "
          f"{'med $/task':>11} {'quality':>8}")
    tot: dict[str, float] = {}
    verdict_arms: dict[str, dict] = {}
    for arm in ("A", "B", "C"):
        if not rows[arm]:
            continue
        e = [x[1] for x in rows[arm]]
        o = [x[2] for x in rows[arm]]
        d = [x[3] for x in rows[arm]]
        q = [x[4] for x in rows[arm]]
        tot[arm] = sum(d)
        print(f"{ARM_NAME[arm]:30} {st.median(e):>11,.0f} {st.median(o):>8,.0f} "
              f"{sum(d):>9.3f} {st.median(d):>11.4f} {st.mean(q):>8.2f}")
        verdict_arms[arm] = {
            "median_eff_input_tokens": round(st.median(e), 1),
            "median_output_tokens": round(st.median(o), 1),
            "total_dollars": round(sum(d), 4),
            "median_dollars_per_task": round(st.median(d), 6),
            "mean_quality": round(st.mean(q), 4),
            "n": len(rows[arm]),
        }

    print("\n=== $-COST RATIOS (total over the task set) ===")
    if {"A", "B", "C"} <= tot.keys():
        print(f"  A read-docs total: ${tot['A']:.3f}")
        print(f"  B pkg-query total: ${tot['B']:.3f}   -> A is {tot['A']/tot['B']:.1f}x B")
        print(f"  C Haiku NL-tool:   ${tot['C']:.3f}   -> A is {tot['A']/tot['C']:.1f}x C ; "
              f"B is {tot['B']/tot['C']:.1f}x C")

    print("\n=== QUALITY per arm per stratum (mean gold-coverage) ===")
    qs: dict[str, dict[str, list]] = defaultdict(lambda: defaultdict(list))
    for arm in ("A", "B", "C"):
        for t, _e, _o, _d, qq in rows[arm]:
            qs[tasks[t]["stratum"]][arm].append(qq)
    print(f"  {'stratum':12} {'A':>6} {'B':>6} {'C':>6}")
    for s in STRATA:
        print(f"  {s:12} " + " ".join(
            f"{(st.mean(qs[s][a]) if qs[s][a] else 0):>6.2f}" for a in ("A", "B", "C")))

    if args.out:
        verdict = {
            "bead": "sq-ve5dy",
            "n_tasks": len(tasks),
            "metric": "model-price-weighted $ from real cache-discounted transcript tokens",
            "prices_per_mtok": PRICE,
            "arms": verdict_arms,
            "ratios": {
                "A_over_C": round(tot["A"] / tot["C"], 2) if {"A", "C"} <= tot.keys() else None,
                "B_over_C": round(tot["B"] / tot["C"], 2) if {"B", "C"} <= tot.keys() else None,
                "A_over_B": round(tot["A"] / tot["B"], 2) if {"A", "B"} <= tot.keys() else None,
            },
        }
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(verdict, fh, indent=2)
        print(f"\nwrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
