#!/usr/bin/env python3
# Paired A/B statistics + §5.6 verdict-object emitter (bead sq-lhwo.4,
# epic sq-lhwo). 🤖 SPARQ agent.
#
# WHY THIS EXISTS
# --------------------------------------------------------------------------------
# Both agent-efficiency A/B tracks — the ast-grep+outline intervention (this bead)
# and the PKG-query intervention (sq-2m6zm.4) — share ONE statistics + verdict
# contract: research/dogfooding-sparq-knowledge-graph.md §5.3-§5.6. That contract
# was specified in prose and pre-registered in the per-track PREREG fixtures, but
# the EXECUTABLE step that turns paired per-task deltas into the verdict object did
# not exist yet. This module is that step. It is intervention-agnostic: feed it the
# per-task paired effective-token deltas (and an optional paired quality column) and
# it emits the frozen verdict object, applying the PRE-REGISTERED kill criteria
# mechanically — it cannot be talked into a positive.
#
# WHAT IT COMPUTES (stdlib only — no scipy/numpy; deterministic)
# --------------------------------------------------------------------------------
#   * Paired-median effective-token delta (arm A - arm B; positive = B cheaper) and
#     its relative reduction vs arm A's median.
#   * Wilcoxon SIGNED-RANK exact/normal p-value over the paired deltas (§5.1).
#   * Bootstrap 95% CI on the MEDIAN delta (seeded → reproducible; §5.1).
#   * Break-even N* from the cost model (§5.3), with the infinite-denominator guard.
#   * The §5.6 verdict object, with the three KILL criteria (§5.4) applied. It also
#     enforces the §5.4 cache-artifact guard: a "win" whose NOMINAL (fresh) input did
#     not drop — i.e. it is entirely the cache-discount component — is NOT a token win.
#
# HONESTY (load-bearing)
# --------------------------------------------------------------------------------
# This module does NOT measure anything. It only does arithmetic over deltas it is
# handed, so it cannot fabricate a result. Numbers it prints are work-box / runtime
# and NON-CANONICAL (project-ec2-execution-env): never freeze them into committed
# markdown — check-no-perf-numbers.py would (correctly) flag them. The committed
# artifacts are THIS CODE, the pre-registration fixture, and the verdict SCHEMA.
#
# Stdlib-only; mirrors scripts/agent-telemetry/agent_telemetry.py's dependency
# posture (nothing added to sparq-core / -engine).

from __future__ import annotations

import argparse
import json
import math
import random
import sys
from statistics import median
from typing import Optional

# Pre-registered significance bar (§5.1 / §5.4) — FROZEN. Editing these to fit a
# result defeats the pre-registration; the per-track PREREG.md restates them.
MIN_RELATIVE_REDUCTION = 0.20  # ≥20% paired-median relative reduction.
ALPHA = 0.05  # Wilcoxon p must be < this.
MIN_N = 30  # the §5.1 ≥30-task floor for a defensible p-value.
BOOTSTRAP_RESAMPLES = 10000
BOOTSTRAP_SEED = 0xA57  # fixed → reproducible CI (no env-dependent randomness).


# --------------------------------------------------------------------------------
# Wilcoxon signed-rank test (paired, two-sided) — stdlib implementation.
# --------------------------------------------------------------------------------
def _rankdata(values: list[float]) -> list[float]:
    """Average-rank assignment for ties (1-based), as Wilcoxon requires."""
    order = sorted(range(len(values)), key=lambda i: values[i])
    ranks = [0.0] * len(values)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and values[order[j + 1]] == values[order[i]]:
            j += 1
        avg = (i + j + 2) / 2.0  # average of 1-based ranks i+1..j+1
        for k in range(i, j + 1):
            ranks[order[k]] = avg
        i = j + 1
    return ranks


def wilcoxon_signed_rank(deltas: list[float]) -> dict:
    """Two-sided Wilcoxon signed-rank over paired deltas.

    Zero deltas are dropped (Wilcoxon convention). Uses the normal approximation
    with continuity + tie correction; for very small n the p-value is conservative.
    Returns {"W", "z", "p", "n_nonzero"}. p is None when no non-zero deltas remain.
    """
    nz = [d for d in deltas if d != 0]
    n = len(nz)
    if n == 0:
        return {"W": 0.0, "z": None, "p": None, "n_nonzero": 0}
    abs_ranks = _rankdata([abs(d) for d in nz])
    w_plus = sum(r for d, r in zip(nz, abs_ranks) if d > 0)
    w_minus = sum(r for d, r in zip(nz, abs_ranks) if d < 0)
    w = min(w_plus, w_minus)
    mean_w = n * (n + 1) / 4.0
    # tie correction for the variance
    from collections import Counter

    tie_term = 0.0
    for _, c in Counter(abs(d) for d in nz).items():
        if c > 1:
            tie_term += c**3 - c
    var_w = (n * (n + 1) * (2 * n + 1) - tie_term / 2.0) / 24.0
    if var_w <= 0:
        return {"W": w, "z": None, "p": None, "n_nonzero": n}
    # continuity correction toward the mean
    cc = 0.5 if w < mean_w else -0.5
    z = (w - mean_w + cc) / math.sqrt(var_w)
    p = 2.0 * (1.0 - _norm_cdf(abs(z)))
    p = max(0.0, min(1.0, p))
    return {"W": w, "z": z, "p": p, "n_nonzero": n}


def _norm_cdf(x: float) -> float:
    return 0.5 * (1.0 + math.erf(x / math.sqrt(2.0)))


# --------------------------------------------------------------------------------
# Bootstrap 95% CI on the median paired delta (seeded; reproducible).
# --------------------------------------------------------------------------------
def bootstrap_median_ci(
    deltas: list[float],
    resamples: int = BOOTSTRAP_RESAMPLES,
    seed: int = BOOTSTRAP_SEED,
    ci: float = 0.95,
) -> tuple[Optional[float], Optional[float]]:
    if not deltas:
        return (None, None)
    rng = random.Random(seed)
    n = len(deltas)
    meds = []
    for _ in range(resamples):
        sample = [deltas[rng.randrange(n)] for _ in range(n)]
        meds.append(median(sample))
    meds.sort()
    lo_idx = int((1 - ci) / 2 * resamples)
    hi_idx = int((1 + ci) / 2 * resamples) - 1
    return (meds[lo_idx], meds[hi_idx])


# --------------------------------------------------------------------------------
# Break-even N* (§5.3): N* = C_ingest / (C_docread - C_query - C_maintain_per_use)
# All in cache-discounted effective tokens. Denominator ≤ 0 → infinite (never pays).
# For ast-grep+outline the "ingest" cost is the one-time SKILL/tool-def + index
# build; C_query is the per-use ast-grep/outline payload; C_docread is the per-use
# full-Read/grep payload. Caller supplies the three amortisable means.
# --------------------------------------------------------------------------------
def break_even_n(
    c_ingest: float, c_docread: float, c_query: float, c_maintain_per_use: float = 0.0
) -> dict:
    denom = c_docread - c_query - c_maintain_per_use
    if denom <= 0:
        return {"break_even_N": None, "break_even_infinite": True, "denominator": denom}
    n_star = c_ingest / denom
    return {
        "break_even_N": int(math.ceil(n_star)),
        "break_even_infinite": False,
        "denominator": denom,
    }


# --------------------------------------------------------------------------------
# The §5.6 verdict object — kill criteria applied MECHANICALLY.
# --------------------------------------------------------------------------------
def build_verdict(
    arm_a: list[float],
    arm_b: list[float],
    *,
    nominal_fresh_a: Optional[list[float]] = None,
    nominal_fresh_b: Optional[list[float]] = None,
    quality_delta: Optional[dict] = None,
    cost_model: Optional[dict] = None,
    measurement_fidelity: str = "read-payload-proxy",
    min_relative_reduction: float = MIN_RELATIVE_REDUCTION,
    alpha: float = ALPHA,
    min_n: int = MIN_N,
) -> dict:
    """Compute the frozen §5.6 verdict object from paired arm-A / arm-B effective
    tokens.

    `arm_a[i]` and `arm_b[i]` are the SAME task i's effective input tokens. The
    paired delta is A - B (positive = B cheaper). `quality_delta` is the §5.2 pair
    (caller supplies it from the deterministic grader; None → quality unknown, which
    blocks recommend_adopt). `cost_model` (optional) drives break-even.
    """
    if len(arm_a) != len(arm_b):
        raise ValueError("arm_a and arm_b must be paired (equal length)")
    n = len(arm_a)
    deltas = [a - b for a, b in zip(arm_a, arm_b)]
    med_delta = median(deltas) if deltas else 0.0
    med_a = median(arm_a) if arm_a else 0.0
    rel_reduction = (med_delta / med_a) if med_a > 0 else 0.0

    wil = wilcoxon_signed_rank(deltas)
    ci_lo, ci_hi = bootstrap_median_ci(deltas)

    # §5.4 cache-artifact guard: did the NOMINAL (fresh) input also drop? If the
    # caller supplies the fresh-token components, a "win" that exists only in the
    # cache-discounted view (nominal fresh did NOT drop) is rejected as an artifact.
    cache_artifact_only = False
    if nominal_fresh_a is not None and nominal_fresh_b is not None:
        nom_deltas = [a - b for a, b in zip(nominal_fresh_a, nominal_fresh_b)]
        nom_med = median(nom_deltas) if nom_deltas else 0.0
        cache_artifact_only = med_delta > 0 and nom_med <= 0

    # KILL 1 (token).
    enough_power = n >= min_n
    p = wil["p"]
    p_ok = (p is not None) and (p < alpha)
    ci_excludes_zero = (ci_lo is not None) and (ci_lo > 0)
    rel_ok = rel_reduction >= min_relative_reduction
    token_win = bool(
        rel_ok
        and p_ok
        and ci_excludes_zero
        and enough_power
        and not cache_artifact_only
    )

    # KILL 2 (break-even).
    be = {"break_even_N": None, "break_even_infinite": True, "denominator": None}
    if cost_model:
        be = break_even_n(
            c_ingest=cost_model.get("c_ingest", 0.0),
            c_docread=cost_model.get("c_docread", 0.0),
            c_query=cost_model.get("c_query", 0.0),
            c_maintain_per_use=cost_model.get("c_maintain_per_use", 0.0),
        )
    break_even_ok = not be["break_even_infinite"]
    if cost_model and "realistic_horizon" in cost_model and be["break_even_N"] is not None:
        break_even_ok = break_even_ok and be["break_even_N"] <= cost_model["realistic_horizon"]

    # KILL 3 (quality). Non-regression requires: accuracy lower-CI ≥ 0 AND
    # hallucination not increased. Unknown quality → cannot certify non-regression.
    quality_non_regression: Optional[bool] = None
    quality_fully_graded = False
    if quality_delta is not None:
        acc = quality_delta.get("exec_acc", {})
        hall = quality_delta.get("hallucination_rate", {})
        acc_lo = acc.get("delta_ci_lo")
        hall_delta = hall.get("delta")
        acc_ok = acc_lo is None or acc_lo >= 0
        hall_ok = hall_delta is None or hall_delta <= 0
        quality_non_regression = bool(acc_ok and hall_ok)
        # A quality pair is FULLY graded only if the end-to-end accuracy axis was
        # model-graded (exec_acc.model_graded == True). A deterministic-only pair
        # (no model-in-the-loop locate/shape accuracy) is PARTIAL and must NOT
        # certify adoption — the §5/PREREG honest off-ramp.
        quality_fully_graded = bool(acc.get("model_graded") is True)

    # `measurement_fidelity` distinguishes the proxy from the §5 gold standard. A
    # read-payload-proxy run, OR a run whose quality pair is not fully model-graded,
    # is PROVISIONAL and CANNOT recommend adoption (PREREG: "a proxy-only verdict
    # cannot reach recommend_adopt").
    provisional = (measurement_fidelity != "full-session") or (not quality_fully_graded)

    recommend_adopt = bool(
        token_win
        and break_even_ok
        and (quality_non_regression is True)
        and not provisional
    )

    # `honest` is a self-attestation that the verdict OBJECT faithfully reflects what
    # was measured — NOT that the run was the gold standard. It is True when the
    # object's preconditions hold (paired, ≥min_n, a quality pair was supplied, no
    # cache-only artifact) AND the object correctly marks itself provisional when it
    # is. A provisional run is still honest (it does not over-claim); it is the
    # `recommend_adopt`/`provisional` flags, not `honest`, that gate adoption.
    honest = bool(
        n == len(arm_b)
        and not cache_artifact_only
        and (quality_delta is not None)
        and enough_power
        # if provisional, recommend_adopt MUST be false for the object to be honest
        and (not provisional or recommend_adopt is False)
    )

    return {
        "n": n,
        "token_win": token_win,
        "token_delta_median": med_delta,
        "token_delta_median_pct": round(rel_reduction * 100.0, 4),
        "token_delta_ci": [ci_lo, ci_hi],
        "wilcoxon": wil,
        "cache_artifact_only": cache_artifact_only,
        "quality_delta": quality_delta,
        "quality_non_regression": quality_non_regression,
        "quality_fully_graded": quality_fully_graded,
        "measurement_fidelity": measurement_fidelity,
        "provisional": provisional,
        "break_even_N": be["break_even_N"],
        "break_even_infinite": be["break_even_infinite"],
        "honest": honest,
        "recommend_adopt": recommend_adopt,
        "_kill_criteria": {
            "KILL1_token": {
                "rel_reduction_ge_20pct": rel_ok,
                "wilcoxon_p_lt_alpha": p_ok,
                "ci_lower_gt_0": ci_excludes_zero,
                "n_ge_min": enough_power,
                "not_cache_artifact": not cache_artifact_only,
            },
            "KILL2_break_even": {"finite_and_within_horizon": break_even_ok},
            "KILL3_quality": {"non_regression": quality_non_regression},
        },
        "_thresholds": {
            "min_relative_reduction": min_relative_reduction,
            "alpha": alpha,
            "min_n": min_n,
        },
        "_note": (
            "NON-CANONICAL work-box numbers; never frozen into committed markdown. "
            "Decide on the verdict OBJECT, never one number (arm-on-verdict)."
        ),
    }


def _load_pairs(path: str):
    """Load a per-task JSONL of {task, arm_a_eff, arm_b_eff[, *_fresh, measurement]}.

    Returns (arm_a, arm_b, fresh_a|None, fresh_b|None, measurement_fidelity). The
    measurement label is taken from the rows (all rows must agree); defaults to
    "read-payload-proxy" so an unlabelled row set is treated as provisional, never
    as the gold standard.
    """
    a, b, fa, fb = [], [], [], []
    have_fresh = True
    fidelities = set()
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            a.append(float(row["arm_a_eff"]))
            b.append(float(row["arm_b_eff"]))
            fidelities.add(row.get("measurement", "read-payload-proxy"))
            if "arm_a_fresh" in row and "arm_b_fresh" in row:
                fa.append(float(row["arm_a_fresh"]))
                fb.append(float(row["arm_b_fresh"]))
            else:
                have_fresh = False
    fidelity = fidelities.pop() if len(fidelities) == 1 else "read-payload-proxy"
    return (a, b, (fa if have_fresh and fa else None), (fb if have_fresh and fb else None), fidelity)


def main(argv: Optional[list[str]] = None) -> int:
    ap = argparse.ArgumentParser(
        description="Emit the §5.6 verdict object from paired A/B effective-token rows."
    )
    ap.add_argument("pairs", help="JSONL of {task, arm_a_eff, arm_b_eff[, *_fresh]}")
    ap.add_argument("--quality-json", help="optional §5.2 quality_delta JSON file")
    ap.add_argument("--cost-model", help="optional break-even cost-model JSON file")
    ap.add_argument(
        "--fidelity",
        choices=["read-payload-proxy", "full-session"],
        help="override the measurement fidelity (default: from rows). Only "
        "'full-session' + a model-graded quality pair can reach recommend_adopt.",
    )
    ap.add_argument("--out", help="write the verdict object JSON here")
    args = ap.parse_args(argv)

    a, b, fa, fb, fidelity = _load_pairs(args.pairs)
    if args.fidelity:
        fidelity = args.fidelity
    quality = None
    if args.quality_json:
        with open(args.quality_json, encoding="utf-8") as fh:
            quality = json.load(fh)
    cost_model = None
    if args.cost_model:
        with open(args.cost_model, encoding="utf-8") as fh:
            cost_model = json.load(fh)

    verdict = build_verdict(
        a, b, nominal_fresh_a=fa, nominal_fresh_b=fb, quality_delta=quality,
        cost_model=cost_model, measurement_fidelity=fidelity,
    )
    text = json.dumps(verdict, indent=2)
    print(text)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
