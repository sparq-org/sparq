#!/usr/bin/env python3
# [FABLE-5] Hard-zone benchmark gate — median-of-history regression check (bench.yml, main pushes).
#
# WHY — github-action-benchmark's fail-on-alert compares each metric against the SINGLE previous
# benchmark-data point. On shared GitHub-hosted runners a one-off slow-runner environment shift
# flips many unrelated µs-scale timing metrics uniformly "worse" and hard-fails the run, while the
# very next push (back on a normal runner) goes green — a noise flap, not a regression signal
# (live 24h flap on main pushes 2026-07-22/23, e.g. red run 29988505852 between green neighbours).
# This script replaces that single-point hard zone with a deterministic MEDIAN-OF-HISTORY gate:
#
#   * Per metric, take the MEDIAN of its last up-to-HISTORY_WINDOW (5) published VALUES — collected
#     PER METRIC across the FULL retained history (the dev/bench/data.js the workflow already
#     fetched as prev-data.js), NOT just the values inside the last 5 commits, so a sparse metric
#     is never judged from a 1-point "median". A metric needs >= MIN_HISTORY (3) values to be
#     gated at all; below that it is listed as "insufficient history — ungated".
#   * DIRECTION: the suite is customSmallerIsBetter, but `*_per_s` metrics
#     (rsp_persistentdict_triples_per_s) store RAW THROUGHPUT, where larger is BETTER — a
#     pre-existing suite inconsistency this gate compensates for by inverting the ratio
#     (median/current) for names matching `_per_s$`. (Candidate follow-up: store the inverse —
#     seconds-per-triple — so the whole suite is genuinely smaller-is-better.)
#   * FAIL-CLOSED NUMERICS: a current value or history-window value that is NaN/non-numeric/
#     negative HARD-FAILS with a per-metric message — never silently skipped. Current values are
#     validated BEFORE the no-history / insufficient-history early exits (a bad current on a
#     brand-new or young metric still fails the run), and every value inside a metric's median
#     window is checked individually (a negative history point cannot hide behind a positive
#     median).
#   * ZERO BOUNDARY — RESOLVED BY DIRECTION, NOT BY "ratio undefined" (sq-jxeqz / #4559). A
#     zero boundary is not one situation, it is three, and only ONE of them is an error:
#       (i)  current == 0 AND median == 0 — a legitimate STABLE value. Four published series are
#            all-zero by design (geo_compliance_deficit, nlq_ask_repairs, rsp_*_w{2,3}_rows, and
#            the sub-resolution sameas_size8_closure_s), so a blanket "<= 0 fails" would
#            permanently red main. Compared at ratio 1.0 and listed.
#       (ii) current is at the metric's OPTIMUM — smaller-is-better at 0, or `*_per_s` throughput
#            recovering FROM a zero median. The ratio is perfectly well defined (0.0) and the
#            move is an IMPROVEMENT BY CONSTRUCTION: on a smaller-is-better scale nothing can be
#            better than 0, so no comparison against any median can make it a regression. The
#            previous revision routed this to `invalid` — an UNCONDITIONAL exit 1 that the
#            uniform-shift exemption never covers — and it red main repeatedly for achieving a
#            PERFECT result: vectors_hnsw_recall_at10 is a recall DEFICIT (bench/vector/README.md:
#            `recall_deficit_milli = round((1 - recall@10) * 1000)`), so its 0 is recall@10 =
#            1.000, the best value the metric can take. It is now compared at ratio 0.0 and
#            listed in `improved_to_zero` (visible, never fatal). Because the run then goes
#            GREEN it also PUBLISHES, so the median rebases through the ordinary path — this is
#            what breaks the self-perpetuating loop (see PUBLISH ORDER below).
#      (iii) current is at the metric's PESSIMAL side — smaller-is-better rising off a zero
#            median, or throughput collapsing TO zero. This IS a regression, of unbounded ratio.
#            It is bounded (and hence gated) only when the metric's measurement RESOLUTION is
#            known (RESOLUTION_QUANTA); with an unknown quantum the ratio genuinely cannot be
#            bounded and the row stays FAIL-CLOSED, exactly as before. A `*_per_s` throughput
#            collapsing to 0 therefore still hard-fails.
#   * MEASUREMENT RESOLUTION (sq-jxeqz / #4559): a metric cannot express a ratio finer than its
#     own quantization tick, so a ratio computed across one tick is comparing noise. The gate may
#     only hard-fail when the ratio is PROVABLY >= HARD_RATIO given the resolution — see
#     RESOLUTION_QUANTA and ratio_lower_bound(). A row that reaches HARD_RATIO on the point
#     estimate but whose resolution-aware LOWER BOUND does not is watch-only (same treatment as
#     the unit noise floor: printed, ::warning'd, and routed to the durable --report-out trail).
#   * PUBLISH ORDER (sq-jxeqz / #4559): this gate deliberately runs BEFORE the Store step, so a
#     failing run publishes NOTHING — a rejected regression can never rebase the median that
#     judges it. That ordering is load-bearing and is NOT changed here. Its cost is that the
#     published history is a CENSORED (survivorship-biased) sample: every honest run whose noise
#     exceeded the threshold is absent from the very history the floors are derived from, so the
#     measured bands below are LOWER BOUNDS on the honest bands. The fix for a self-perpetuating
#     red is therefore to make the provably-non-regressing case PASS (so it publishes through the
#     ordinary path), never to publish a failing run's measurements.
#   * DISAPPEARED METRICS: metrics with a gateable history (>= MIN_HISTORY values) that are absent
#     from this run's results are warn-listed loudly; the run HARD-FAILS only if more than
#     DISAPPEAR_FAIL_FRACTION (30%) of the gateable metrics disappeared (suite breakage) — a
#     plain rename or two must not red main.
#   * HARD FAIL (exit 1) if any gated metric is >= HARD_RATIO (2.0x) its median (direction-adjusted).
#   * NOISE FLOOR (UNIT-AWARE): only a metric whose UNIT is in the FLOOR_EXEMPT_UNITS
#     allow-list ("us" micros, "milli" — each with a measured per-unit floor) AND whose
#     median-of-window is below that unit's floor can never hard-fail the gate ALONE — at that
#     scale the measured honest run-to-run bands EXCEED the 2.0x hard threshold (empirical
#     bands at the FLOOR_EXEMPT_UNITS constant below). EVERY other unit — "s" seconds,
#     "ns_per_byte", "ratio", `*_per_s` throughputs, deterministic units, and anything
#     UNKNOWN — is NEVER floor-exempt regardless of magnitude (a stable 10x on a 0.4 s load
#     IS a real regression; unknown units fail toward gated). Floor-exempt is NOT ungated: the
#     metric still prints in the watch table flagged "under noise floor — watch only", still
#     emits a ::warning at >= HARD_RATIO, the Store step's soft comparison comment still
#     fires, and every floor-exempt hit at >= HARD_RATIO is written to the --report-out JSON,
#     which the Soft-zone triage step (scripts/bench-triage.py --hardzone-report) merges into
#     the rolling deduped `bench-flake` issue — the DURABLE trail: a persistent sub-floor
#     regression rebases the median after a few accepted points, so the issue trail is the
#     record that survives. Floor-exempt metrics are also EXCLUDED from every uniform-shift
#     computation (breadth/uniformity/cap) — their degenerate small-integer ratios would
#     distort all three. DETERMINISTIC metrics (bytes/count/gates/rows/...) are NEVER
#     floor-exempt — a 2x move on a deterministic output is real at any scale.
#   * UNIFORM-SHIFT EXEMPTION (exit 0 + loud warning + full table to stdout AND the job summary):
#     the run is treated as a runner-environment scaling — and the hard zone waived — ONLY if ALL
#     four conditions hold (computed over the GATED, i.e. non-floor-exempt, metrics):
#       (a) >= UNIFORM_FRACTION (50%) of gated metrics are >= WATCH_RATIO (1.75x) median;
#       (b) ratio uniformity: over the >= 1.75x metrics, stdev(ratio)/mean(ratio) <= 0.15 — a
#           multiplicative environment scaling moves every wall-clock metric by ~the same factor,
#           while a hot-path code regression does not hit unrelated metric families uniformly;
#       (c) NO metric is >= UNIFORM_CAP_RATIO (4.0x) median — a catastrophic regression must never
#           hide in the herd;
#       (d) every compared DETERMINISTIC metric (byte/count/row/gate-class, classified by unit
#           with an anchored-name fallback — see DETERMINISTIC_UNITS) is within DET_TOLERANCE (1%)
#           of its median — an environment shift moves wall-clock, never deterministic outputs; if
#           any deterministic metric moved, there is NO exemption.
#     HONESTY: the four conditions make a MASKED code regression implausible, not impossible —
#     which is exactly why the exemption still emits a ::warning and the full per-metric table
#     into GITHUB_STEP_SUMMARY instead of passing quietly. The exemption never covers fail-closed
#     numeric errors or the disappeared-metrics breakage check.
#   * TRANSPARENCY: every metric >= WATCH_RATIO (1.75x) median is printed as a table either way,
#     to stdout and (when anything is notable) to GITHUB_STEP_SUMMARY — load-bearing because this
#     gate runs BEFORE the Store step: on a hard-zone failure nothing is published and Store's
#     comparison comment never fires, so this table is the only visible evidence.
#   * SUITE NAME: exact match only. If the suite key is absent from the history, warn loudly and
#     exit 0 — NEVER fall back to a different suite's series.
#   * ABSENT HISTORY (no prev-data.js / empty file / empty series): warn + exit 0 (first-run /
#     bootstrap safe; the fetch step creates an EMPTY prev-data.js when the branch has no data.js
#     yet). A NON-empty but unparsable prev-data.js hard-fails (corrupt published history).
#
# The DETERMINISTIC byte-count metrics stay strictly gated by scripts/perf-gate.py (the committed
# best-ever floor ratchet) — this gate is the noisy-timing hard zone only, and it runs ONLY where
# the Store step auto-pushes (push-to-main), so PR behavior is unchanged.
#
# USAGE
#   scripts/bench_hardzone.py --results bench-results.json --prev-data prev-data.js \
#       --suite 'sparq engine'
#   scripts/bench_hardzone.py --self-test    # hermetic; no network, no git, no repo writes
#                                            # (end-to-end fixtures use a private tempdir)
from __future__ import annotations

import argparse
import contextlib
import io
import json
import math
import os
import re
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

PROG = "bench-hardzone"

HARD_RATIO = 2.0        # >= 2.0x median => hard fail (unless the uniform-shift exemption fires)
WATCH_RATIO = 1.75      # >= 1.75x median => printed in the watch table; feeds the uniform check
UNIFORM_FRACTION = 0.5  # exemption (a): >= 50% of compared metrics in the watch band
UNIFORM_MAX_CV = 0.15   # exemption (b): stdev/mean over the watch-band ratios must be <= 0.15
UNIFORM_CAP_RATIO = 4.0  # exemption (c): NO metric may be >= 4.0x median
DET_TOLERANCE = 0.01    # exemption (d): deterministic metrics must sit within 1% of median
HISTORY_WINDOW = 5      # median over the last up-to-5 published VALUES per metric
MIN_HISTORY = 3         # a metric needs >= 3 history values to be gated at all
DISAPPEAR_FAIL_FRACTION = 0.30  # > 30% of gateable metrics missing from results => suite breakage
# How many INDEPENDENT re-measurements to attempt before letting a timing row red main
# (see apply_remeasurement). 1 is the useful default because the re-measure command re-runs the
# whole suite: one clean confirmation already separates a runner slow-window from a code
# regression, and each extra attempt costs another full suite run for a strictly smaller marginal
# gain. It is only ever paid on a run that is ALREADY about to fail.
DEFAULT_REMEASURE_K = 1

# NOISE FLOOR — UNIT-AWARE ALLOW-LIST of (unit, floor) pairs. A metric is floor-exempt
# (watch-only, never a hard fail alone, excluded from the uniform-shift computations) ONLY if
# its unit appears here AND its median-of-window is below that unit's floor. Floors are
# justified PER-UNIT by the measured honest run-to-run bands in the published benchmark-data
# history (2026-07-23 replay):
#   * "us" (micros, floor 40.0): µs-scale timing metrics have HONEST noise bands exceeding the
#     2.0x hard threshold. Within the honest pre-red-era oldest-10 points alone, max/min bands:
#     sameas_size32_query_us 2.55x (values ~4-9µs), deeptax_d1000_query_us 3.25x (~4-13µs),
#     lubm_q14_count_us 2.45x. Live replays: the honest head fails sameas_size32_query_us at
#     2.14x (9 vs median 4.2); another honest entry fails deeptax_d1000_query_us at 2.00x
#     (8 vs 4).
#     RE-DERIVED 2026-07-25 (the original 20.0 was set from the oldest-10 window and its
#     "every false-fail has a median <~15" claim no longer holds). Replaying THIS gate's own
#     methodology — worst current/median-of-trailing-5 per metric — over the full published
#     `us` history, bucketed by metric magnitude, the honest band is:
#         median  0-10µs : n=48  p90 1.84x  MAX 3.00x   (2 metrics >= 2.0x)
#         median 10-20µs : n=40  p90 1.81x  MAX 2.09x   (1 metric  >= 2.0x)
#         median 20-40µs : n=15  p90 1.84x  MAX 2.23x   (1 metric  >= 2.0x)
#         median 40-80µs : n=30  p90 1.80x  MAX 1.88x   (none)
#         median 80-200µs: n=25  p90 1.59x  MAX 1.76x   (none)
#         median >=200µs : n=153 p90 1.52x  MAX 1.90x   (none)
#     These buckets are a LOWER BOUND on the honest band, not an estimate of it: bench.yml's
#     "Store + compare against history" step runs AFTER this gate and is skipped when the gate
#     fails, so the published series contains only runs that already passed — every honest run
#     whose noise exceeded the threshold is censored from the very history used to set the floor.
#     The honest band only drops below the 2.0x hard threshold at >= 40µs, so 20.0 left the
#     20-40µs band hard-gated INSIDE its own noise: honest history alone already reaches
#     watdiv_sf1_L2_json_us 2.23x (43.7 vs trailing median 19.6) and watdiv_sf1_L2_materialize_us
#     1.97x, and this floor's under-setting red main on run 30154655798 (a site-only commit —
#     no Rust in the diff) via watdiv_sf1_L5_materialize_us 2.03x (median 20.6) and
#     watdiv_sf1_S5_json_us 2.08x (median 39.5), both inside the measured band for their
#     magnitude. 40.0 moves 15 metrics from hard-gated to watch-only — 6 of which had ALREADY
#     reached >= 1.75x on honest history alone — and keeps every >= 40µs timing metric, where
#     the measured band tops out at 1.88x, fully hard-gated.
#   * "milli" (floor 20.0): the empirically-degenerate HNSW recall family —
#     vectors_hnsw_recall_at10 honestly oscillates 1<->4 (4.0x band) at integer-milli
#     resolution; its stable siblings (vectors_diskann_recall_at10 ~34,
#     vectors_pq_recall_at10 ~22) sit ABOVE the floor and stay hard-gated.
# EVERY unit NOT in this allow-list is NEVER floor-exempt regardless of magnitude. That
# includes "s" seconds (nine published series — load_s, text_build_s, rdfs_infer_s, ... at
# ~0.4-15 s — where a stable 10x IS a real regression; a raw magnitude-only floor wrongly
# exempted all of them), "ns_per_byte", "ratio", the `*_per_s` throughputs, every
# deterministic unit, and any UNKNOWN/absent unit: unknown units FAIL TOWARD GATED.
# DETERMINISTIC metrics are never floor-exempt even in an allow-listed unit.
FLOOR_EXEMPT_UNITS: dict[str, float] = {"us": 40.0, "milli": 20.0}

# MEASUREMENT RESOLUTION — the COARSEST step a metric of this unit could possibly be printed at,
# read off the code that emits it (never guessed; a unit whose quantum is not derivable is simply
# absent, and absent means "no resolution allowance", i.e. fail toward gated). Entries:
#   * "s" (0.001): the seconds-unit closure metrics are scraped by bench/owl-sameas/run.sh
#     (`grep -oE 'in [0-9.]+s'`) out of crates/sparq-cli/src/main.rs's `... in {:.3}s` — THREE
#     decimal places, so one tick is 1 ms. sameas_size32_closure_s publishes 0.001 on every
#     retained point and its sibling sameas_size8_closure_s publishes 0 (sub-resolution): one
#     single tick up is EXACTLY the inclusive >= 2.0x hard threshold, with zero margin. A value
#     printed as 0.001 is really somewhere in [0.0005, 0.0015) and one printed as 0.002 is in
#     [0.0015, 0.0025), so the TRUE ratio of that "2.00x" spans [1.0, 5.0) — it is consistent
#     with no change at all. The gate must not call that a regression.
#   * "milli" (1.0): the vectors_*_recall_at10 deficits are integers by construction —
#     bench/vector/README.md defines `recall_deficit_milli = round((1 - recall@10) * 1000)`.
#
# THE UNIT IS A CEILING, NOT THE ANSWER — measured, not assumed. The `s` unit is NOT uniformly
# 1 ms-quantized: replaying the published history, of the ten retained `s` series only four
# (deeptax_d{1000,10000}_closure_s, load_s, rdfs_infer_s, sameas_size32_closure_s) actually print
# at 3 decimals; hdt_load_s / snikmeta_decode_s / text_build_s / vectors_build_s come from a
# different emitter and carry 6-7. snikmeta_decode_s in particular has a MEDIAN BELOW ONE
# 1 ms tick, so a blanket per-unit 1 ms allowance would have handed it an allowance several times
# its own median and gutted its gate — precisely the "widen the threshold until it passes" failure
# this fix exists to avoid. So the effective quantum is
#     min(this unit's ceiling, the granularity the metric's own values DEMONSTRATE)
# — see observed_quantum(). Per-metric inference can only ever make the allowance SMALLER, and the
# unit ceiling bounds it for a metric whose values are coincidentally round. Both halves are
# load-bearing and separately mutation-tested.
#
# The allowance this buys is an ABSOLUTE band of (HARD_RATIO + 1)/2 = 1.5 quanta (see
# ratio_lower_bound) — NOT a widened ratio threshold. As a fraction of the hard threshold that is
# ~150% for a 1 ms closure and ~0.19% for a 0.4 s load: it shrinks to nothing exactly where the
# ratio threshold is meaningful. It is not zero there, though — a load_s at EXACTLY 2.0000x is
# inside it (the honest cost of only failing when the resolution PROVES the threshold is
# reached); the band's far edge is pinned from both sides in the self-test.
RESOLUTION_QUANTA: dict[str, float] = {"s": 0.001, "milli": 1.0}

# Deterministic (byte/count/structural) metric classification. UNIT-first because the live metric
# NAMES are trappy: bsbm_query01_count_us / lubm_*_count_us are TIMING metrics whose names contain
# "count", store_bytes_per_triple carries "bytes" mid-name, and rsp_persistentdict_triples_per_s
# carries "triples" but is wall-clock throughput. The unit set below is the exact deterministic
# unit vocabulary observed in the published benchmark-data history (bytes/count/chars/fixtures/
# gates/rows/triples); us / s / ns/byte / ratio / triples_per_s / milli are the noisy families.
# "milli" is NOISY, not deterministic: it carries the three vectors_*_recall_at10 series, and the
# live history shows vectors_hnsw_recall_at10 honestly oscillating 1<->4 (randomized HNSW build
# at degenerate integer-milli resolution) — a measurement, not a deterministic output. Its stable
# siblings (diskann ~34, pq ~22) sit ABOVE the milli floor (FLOOR_EXEMPT_UNITS) and stay
# hard-gated as noisy metrics.
# The anchored-name fallback only fires when a row carries no unit at all.
DETERMINISTIC_UNITS = {"bytes", "count", "chars", "fixtures", "gates", "rows", "triples"}
DETERMINISTIC_NAME_RE = re.compile(r"(_bytes|_count|_chars|_deficit|_gates|_rows|_triples)$")

# Direction: `*_per_s` metrics store raw throughput => LARGER is better (ratio = median/current).
LARGER_IS_BETTER_RE = re.compile(r"_per_s$")


def log(msg: str) -> None:
    print(f"[{PROG}] {msg}", file=sys.stderr)


def _is_num(v) -> bool:
    """A finite real number (bool excluded — json true/false must not pass as 1/0).

    [FABLE-5 round 4] the float() probe is guarded: math.isfinite on an out-of-float-range
    int (e.g. 10**400, which JSON happily round-trips) raises OverflowError, and a gate
    CRASH is neither fail-open nor fail-closed — such a value is simply not a usable
    measurement, so this reports False and evaluate() fails closed with the per-metric
    record, exactly like NaN.
    """
    if not isinstance(v, (int, float)) or isinstance(v, bool):
        return False
    try:
        return math.isfinite(float(v))
    except (OverflowError, ValueError):
        return False


def parse_data_js(text: str) -> dict:
    """Parse a github-action-benchmark data.js payload (window.BENCHMARK_DATA = {...};)."""
    m = re.search(r"window\.BENCHMARK_DATA\s*=\s*", text)
    if m:
        text = text[m.end():]
    text = text.strip().rstrip(";").strip()
    return json.loads(text)


def history_values(series: list[dict], window: int = HISTORY_WINDOW) -> dict[str, list]:
    """Per-metric last up-to-`window` published VALUES across the FULL retained series.

    `series` is entries[suite] from data.js: a list of {commit, date, benches:[{name,value,..}]}.
    Entries are ordered by date; every entry contributes, and the window is applied PER METRIC to
    the metric's own value sequence — a metric published in only some entries still collects up to
    `window` values from across the whole retained history (the old per-commit windowing judged
    sparse metrics from as little as ONE point). Values are kept RAW (including non-numeric junk)
    so evaluate() can fail closed on garbage that lands inside a metric's window.
    """
    ordered = sorted(series, key=lambda e: e.get("date", 0))
    vals: dict[str, list] = {}
    for entry in ordered:
        for b in entry.get("benches", []):
            name = b.get("name")
            if name is not None:
                vals.setdefault(name, []).append(b.get("value"))
    return {n: v[-window:] for n, v in vals.items()}


def is_deterministic(name: str, unit: str) -> bool:
    if unit:
        return unit in DETERMINISTIC_UNITS
    return bool(DETERMINISTIC_NAME_RE.search(name))


def _floor_desc() -> str:
    """Compact rendering of the unit-aware floor allow-list for report/annotation text."""
    return ", ".join(f"{u} < {f:g}" for u, f in sorted(FLOOR_EXEMPT_UNITS.items()))


def observed_quantum(values: list[float], ceiling: float) -> float:
    """Coarsest decimal step <= `ceiling` that EVERY observed non-zero value is a multiple of.

    The published values are the only direct evidence of what precision the emitter actually
    prints at, and they bound the allowance from below: a series carrying 0.000462 demonstrably
    resolves to 1e-6 and must not be handed a 1 ms allowance just because its UNIT is seconds.
    Zeros constrain nothing (every step divides 0); with no non-zero value at all the ceiling
    stands, which only happens on the stable-zero path where no ratio is computed anyway.
    """
    nz = [abs(v) for v in values if v != 0.0]
    if not nz:
        return ceiling
    # Walk DECIMAL EXPONENTS, never `q /= 10.0`: repeated division accumulates float error and
    # would return 1.0000000000000002e-06 where 1e-06 is meant, making the quantum (and every
    # bound derived from it) non-canonical. Every RESOLUTION_QUANTA ceiling is a power of ten —
    # asserted in the self-test — so its exponent is exact.
    exp = round(math.log10(ceiling))
    q = ceiling
    for k in range(10):  # at most 9 decades below the unit ceiling
        q = 10.0 ** (exp - k)
        # Exact-multiple test with an absolute+relative epsilon: v/q must land on an integer.
        if all(abs(v / q - round(v / q)) * q <= 1e-12 * (1.0 + v) for v in nz):
            return q
    return q


def resolution_quantum(unit: str, det: bool, values: list[float]) -> float:
    """The smallest change this metric can EXPRESS, or 0.0 when it is not derivable.

    Two independent bounds, both load-bearing (see RESOLUTION_QUANTA):
      * the UNIT ceiling — the coarsest step the emitting code could print, from the allow-list;
        a unit that is not allow-listed gets NO allowance at all (fail toward gated);
      * the metric's OWN demonstrated granularity — never coarser than its values prove.

    DETERMINISTIC metrics get 0.0 unconditionally: a byte/count/gate output is exact, so there is
    no measurement resolution to allow for and a 2x move on one is real at any scale (the same
    rule the unit noise floor uses). This guard is LOAD-BEARING, not belt-and-braces: a count
    series reading 1000/2000/3000 would otherwise infer a quantum of 1000 and silence itself.
    """
    if det:
        return 0.0
    ceiling = RESOLUTION_QUANTA.get(unit)
    if ceiling is None:
        return 0.0
    return observed_quantum(values, float(ceiling))


def ratio_lower_bound(val: float, med: float, larger_better: bool, quantum: float) -> float:
    """SMALLEST direction-adjusted ratio consistent with two values quantized at `quantum`.

    A value REPORTED as `v` was really somewhere in [v - q/2, v + q/2); the median of a window of
    such values is an order statistic of them and carries the same half-tick uncertainty. The
    least-regressed reading is therefore the smallest possible numerator over the largest possible
    denominator. `quantum` must be > 0 (callers use resolution_quantum() and check), which also
    makes the denominator strictly positive — so this is total even at a zero median, the one
    place the plain ratio genuinely divides by zero.

    The gate hard-fails only when THIS bound reaches HARD_RATIO — i.e. only when the measurement's
    own resolution PROVES the threshold is reached. Algebraically that is the plain test plus an
    absolute allowance of exactly (HARD_RATIO + 1) * q / 2, independent of magnitude: a metric
    measured far above its resolution is unaffected, one measured AT it is fully protected.
    """
    half = quantum / 2.0
    if larger_better:
        return max(med - half, 0.0) / (val + half)
    return max(val - half, 0.0) / (med + half)


def resolution_exempts(val: float, med: float, larger_better: bool, quantum: float,
                       ratio: float) -> bool:
    """True iff `ratio` reaches HARD_RATIO but the measurement's resolution cannot PROVE it.

    NARROW BY CONSTRUCTION, and both conjuncts are load-bearing:
      * `ratio >= HARD_RATIO` — the exemption may only ever apply to a row that would otherwise
        hard-fail. Without it every seconds/milli row would be flagged exempt and flood the
        durable trail with metrics that are not in the hard band at all.
      * `lower_bound < HARD_RATIO` — STRICT. A row whose bound lands exactly ON the threshold is
        proven to reach it and stays gated.
    """
    return (quantum > 0.0 and ratio >= HARD_RATIO
            and ratio_lower_bound(val, med, larger_better, quantum) < HARD_RATIO)


# ── CONFIRM-BY-RE-MEASUREMENT (sq-jxeqz) ────────────────────────────────────────────────────────
def best_reading(values: list[float], larger_better: bool) -> float:
    """The LEAST-REGRESSED reading in a set — min for smaller-is-better, max for `*_per_s`.

    Direction-aware on purpose: taking `min` unconditionally would treat a throughput COLLAPSE as
    the best reading and relieve exactly the regression the gate exists to catch.
    """
    return max(values) if larger_better else min(values)


def _rows_by_name(rows: list[dict]) -> dict[str, float]:
    """{name: value} for the finite, non-negative numeric rows of a bench-results.json payload."""
    out: dict[str, float] = {}
    for row in rows or []:
        if not isinstance(row, dict):
            continue
        name, val = row.get("name"), row.get("value")
        if isinstance(name, str) and _is_num(val) and float(val) >= 0:
            out[name] = float(val)
    return out


def apply_remeasurement(report: dict, remeasure_fn, k: int = 1) -> None:
    """Re-measure the rows that are about to hard-fail and keep only the ones that REPRODUCE.

    WHY THIS EXISTS — the measured defect (sq-jxeqz / #4559)
    -------------------------------------------------------
    The hard zone decides from a SINGLE sample per metric, taken inside ONE contiguous bench run.
    Harvesting the live step output of 153 `Benchmarks` runs on main (the run logs are the only
    UNCENSORED record — the gate runs before Store, so a failing run publishes nothing and its
    reading never enters the very history the thresholds are derived from) gives 29 tripped runs
    (19%) carrying 60 fatal rows over 19 distinct metrics, every one of them bracketed by GREEN
    runs on the SAME metric with no intervening revert. The failures co-move within a run across
    metrics that share a workload (`op_q04_triangle_{count,json,materialize}`,
    `sp2b_q11_{count,json,materialize}`) — the signature of a runner slow WINDOW, which hits
    whichever benchmarks execute inside it rather than the whole suite.
    That event has no exemption today: the uniform-shift carve-out demands >= UNIFORM_FRACTION
    (50%) of the suite move together, and a slow window was observed moving 1-8 of 438 metrics.

    WHY NOT A WIDER BAND — measured, and rejected
    ---------------------------------------------
    Requiring an absolute delta above the metric's OWN historical dispersion (robust sigma from
    the median absolute deviation of its published series) was measured against those same 60
    fatal rows: at 5 sigma it relieves 11/60 rows (6/29 runs), and to relieve a majority it needs
    10-12 sigma, which by the same replay stops catching a genuine sustained 2.0x regression on
    6-7% of gated metrics. That is the false-positive-for-false-negative trade this gate must not
    make, so the band is NOT what is implemented here. Neither is a recalibrated breadth
    condition: green runs reach 15 watch-band metrics and tripped runs go as low as 1, so
    run-level breadth does not separate the two populations.

    WHAT IS IMPLEMENTED, AND WHAT IT STILL CATCHES
    ----------------------------------------------
    Confirmation. A row may only hard-fail if the breach REPRODUCES on an independent
    re-measurement. This costs NO sensitivity to a real regression, which is the point: a genuine
    regression is a property of the built artifact, so it re-measures at the same ratio and still
    reds. Every other guard is untouched — HARD_RATIO, the unit floor, the resolution bound, the
    four uniform-shift conditions, deterministic strictness, the disappeared-metric check and
    every fail-closed numeric path all behave exactly as before. The ONLY breach this removes is
    one that fails to reproduce, which by construction was not a property of the code.
    It is NOT a best-of-N squeeze of the PUBLISHED number: `report["rows"]` and the results file
    handed to the Store step are left untouched, so the trend keeps recording what was measured.

    PRECEDENT: scripts/perf-gate.py has done exactly this for the PR-leg timing metric since
    sq-dzfu (`--remeasure-cmd` / `--remeasure-k` / `gate_with_remeasure`), for the same reason
    ("shared-runner wall-clock variance exceeds any band tight enough to be useful").

    NOTE ON THE HARNESS'S OWN best-of-3: several suites already take a min over 3 iterations
    (e.g. `bench_shacl` mins `validate_us` over `iters`). That is NOT a substitute — three
    iterations of a ~140 us measurement span well under a millisecond, so all three samples sit
    inside the same contention window. Only a re-measurement separated by the rest of the suite
    is temporally independent.

    FAIL-CLOSED IN EVERY DIRECTION:
      * no `remeasure_fn`                       -> nothing is relieved (identical to today)
      * `remeasure_fn` raises / returns nothing -> nothing is relieved
      * a metric MISSING from a re-measurement  -> that row is NOT relieved
      * a DETERMINISTIC row in the hard set     -> no re-measurement is attempted at all
        (re-running cannot change an integer byte count, and its failure is already decisive)
      * a ZERO-median (zero-boundary) row        -> declined; evaluate()'s own zero-boundary
        decision stands, and no second semantics for it is invented here
    The first three hold STRUCTURALLY rather than by a separate branch, and that is deliberate:
    every metric's reading pool is SEEDED with the original measurement, which is by construction
    already at or above HARD_RATIO (it is in `hard`). So a pool that gained nothing — no attempt
    ran, the command emitted nothing, or it omitted this metric — still combines to the original
    reading and re-derives the original ratio, and the row stays fatal. Explicit `if` branches for
    those cases were written first and then REMOVED: mutation testing showed deleting them changed
    no behaviour, i.e. they read as protection while protecting nothing. The property is pinned
    instead by asserting the pool seeding directly (self-test 19g). The one remaining empty-pool
    branch is an INVARIANT guard, not live protection, and is labelled as such at its site.
    Mutates `report` in place: relieved rows leave `hard`, join `remeasure_relieved`, and ride the
    existing durable `floor_exempt_hard` trail so nothing goes quiet.
    """
    report.setdefault("remeasure_relieved", [])
    report.setdefault("remeasure_readings", {})
    report.setdefault("remeasure_attempts", 0)
    hard = report.get("hard") or []
    if not hard or remeasure_fn is None or k < 1:
        return
    if report.get("uniform_shift"):
        return                      # not failing on these rows anyway — do not spend the CI time
    if any(r[5] for r in hard):
        # A deterministic metric is in the hard set: it cannot be relieved and it fails the run on
        # its own, so a re-measurement could not change the outcome. Mirrors perf-gate.py.
        report["remeasure_skipped"] = ("a deterministic metric is in the hard set — its value "
                                       "cannot change on a re-run and it fails the run alone")
        return
    wanted = {r[0]: r for r in hard}
    readings: dict[str, list[float]] = {n: [r[1]] for n, r in wanted.items()}
    attempts = 0
    for _ in range(k):
        try:
            fresh = remeasure_fn()
        except Exception as e:                                  # noqa: BLE001 — fail-closed
            log(f"re-measurement attempt failed ({e!r}) — no row is relieved by it")
            continue
        attempts += 1
        by_name = _rows_by_name(fresh)
        for name in wanted:
            if name in by_name:
                readings[name].append(by_name[name])
    report["remeasure_attempts"] = attempts
    report["remeasure_readings"] = readings
    still_hard, relieved = [], []
    for name, r in wanted.items():
        # SEEDED WITH THE ORIGINAL READING — this is what makes every "no usable re-measurement"
        # path fail closed without a branch of its own. See the fail-closed note in the docstring.
        vals = readings[name]
        if not vals:
            # INVARIANT GUARD, unreachable while the seeding above holds. It is here so that a
            # future edit which breaks the seeding FAILS CLOSED (row stays fatal) instead of
            # raising out of min()/max() — and so the mutant that breaks the seeding is killed by
            # a NAMED assertion (self-test 19g) rather than by a traceback.
            still_hard.append(r)
            continue
        med, larger_better = r[2], r[6]
        best = best_reading(vals, larger_better)
        # ZERO-BOUNDARY ROWS ARE NEVER RELIEVED HERE. A pessimal zero crossing DOES reach the
        # hard set — `evaluate` admits one whose resolution-aware lower bound proves it passes
        # HARD_RATIO, and such a row has median 0 (e.g. x_s: 0 -> 0.010 at 19.0x). Re-deriving a
        # ratio from it would divide by zero, and inventing a second zero-boundary semantics here
        # would be a place for the two to disagree. The zero boundary already has dedicated,
        # tested handling in `evaluate` (improved_to_zero / zero_crossing / ratio_lower_bound), so
        # this pass declines the row and leaves that decision standing: fail-closed, one owner.
        if med <= 0 or best <= 0:
            still_hard.append(r)
            continue
        new_ratio = (med / best) if larger_better else (best / med)
        if new_ratio >= HARD_RATIO:
            still_hard.append(r)
        else:
            relieved.append((r, best, new_ratio, len(vals) - 1))
    report["hard"] = still_hard
    report["remeasure_relieved"] = relieved
    for r, best, new_ratio, n_re in relieved:
        report["floor_exempt_hard"].append({
            "name": r[0], "current": r[1], "previous": r[2],
            "ratio_pct": round(r[4] * 100.0, 1), "unit": report.get("units", {}).get(r[0], ""),
            "note": (f"NOT REPRODUCED — re-measured {n_re}x after the suite; best reading "
                     f"{best:.6g} is {new_ratio:.2f}x median (< {HARD_RATIO:g}x). Watch-only for "
                     f"the gate; the ORIGINAL {r[4]:.2f}x reading is what the trend publishes."),
        })


def remeasure_via_shell(cmd: str, out_path: str):
    """Build a `remeasure_fn` that runs `cmd` and reads the bench-results JSON it writes.

    Contract (identical to scripts/perf-gate.py's `--remeasure-cmd`): the command re-runs the
    benchmark suite and writes a customSmallerIsBetter payload — a [{name,unit,value}] list — to
    `out_path`. Anything else (non-zero exit, missing/corrupt file) yields NO rows, which relieves
    NOTHING: the gate stays fail-closed on a broken re-measure command.
    """
    def _fn() -> list[dict]:
        p = Path(out_path)
        with contextlib.suppress(OSError):
            p.unlink()
        rc = subprocess.call(cmd, shell=True)
        if rc != 0:
            log(f"re-measure command exited {rc}; its readings are discarded (nothing relieved)")
            return []
        if not p.exists():
            log(f"re-measure command wrote no {out_path}; nothing relieved")
            return []
        try:
            payload = json.loads(p.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, ValueError, OSError) as e:
            log(f"re-measure output {out_path} is unreadable ({e}); nothing relieved")
            return []
        if isinstance(payload, dict):
            payload = payload.get("benches", [])
        return payload if isinstance(payload, list) else []

    return _fn


def evaluate(current: list[dict], hist: dict[str, list]) -> tuple[int, dict]:
    """Pure gate logic — unit-tested by --self-test. No I/O, no env access.

    `current` is this run's bench-results.json rows ({name,value,unit}); `hist` is the per-metric
    history value lists (see history_values). Returns (exit_code, report) where report carries:
      compared      — number of ratio-compared metrics (gated + floor-exempt)
      rows          — [(name, cur, median, n, ratio, deterministic, larger_better, floor_exempt,
                      resolution_exempt)]
      watch         — the >= WATCH_RATIO subset of rows (exempt rows included, flagged)
      hard          — the >= HARD_RATIO subset of GATED rows, minus the resolution-exempt ones
                      (neither exemption can hard-fail the run alone)
      floor_exempt  — rows whose UNIT is in FLOOR_EXEMPT_UNITS with median under that unit's
                      floor (watch-only; unit-aware allow-list, unknown units stay gated)
      resolution_exempt — rows at >= HARD_RATIO on the point estimate whose resolution-aware
                      LOWER BOUND (see ratio_lower_bound) does not reach HARD_RATIO (watch-only)
      floor_exempt_hard — BOTH exemptions' hard-band rows, in the soft-triage comparison
                      shape ({name,current,previous,ratio_pct,unit,note}; previous = the
                      MEDIAN-of-history) — the durable-trail payload written by --report-out
                      and merged into the deduped bench-flake issue by bench-triage.py
      gated_compared / gated_watch — counts over the GATED (non-floor-exempt) rows, backing
                      the uniform-shift annotation (gated numerator/denominator)
      invalid       — [(name, reason)] fail-closed numeric errors (any entry => exit 1)
      ungated       — [(name, n)] metrics with 1..MIN_HISTORY-1 history values (listed, not gated)
      new           — metric names present in results with NO history at all
      stable_zero   — metrics compared as legitimate 0 == 0
      improved_to_zero — [(name, cur, median, unit)] metrics that reached their OPTIMUM across a
                      zero boundary (ratio 0.0): visible, never fatal (#4559)
      zero_crossing — [(name, cur, median, ratio, unit)] PESSIMAL-side zero crossings whose ratio
                      is bounded by the metric's measurement resolution rather than undefined
      disappeared   — gateable metrics absent from this run's results
      disappeared_frac / uniform_shift / exemption_reasons
    """
    rows: list[tuple] = []
    invalid: list[tuple[str, str]] = []
    ungated: list[tuple[str, int]] = []
    new: list[str] = []
    stable_zero: list[str] = []
    improved_to_zero: list[tuple] = []
    zero_crossing: list[tuple] = []
    units: dict[str, str] = {}
    quanta: dict[str, float] = {}

    current_by_name: dict[str, dict] = {}
    for row in current:
        name = row.get("name")
        if name is not None:
            current_by_name[name] = row

    gateable = {n for n, pts in hist.items() if len(pts) >= MIN_HISTORY}
    disappeared = sorted(n for n in gateable if n not in current_by_name)
    disappeared_frac = (len(disappeared) / len(gateable)) if gateable else 0.0

    for name in sorted(current_by_name):
        row = current_by_name[name]
        val = row.get("value")
        unit = row.get("unit") or ""
        # FAIL-CLOSED, pre-history: validate the current value BEFORE the no-history /
        # insufficient-history early exits — a NaN/non-numeric/negative current on a brand-new
        # or young metric must still fail the run, not slide out through "new"/"ungated".
        if not _is_num(val):
            invalid.append((name, f"current value {val!r} is not a finite number — fail-closed"))
            continue
        val = float(val)
        if val < 0:
            invalid.append((name, f"negative current measurement ({val:g}) — fail-closed"))
            continue
        pts = hist.get(name)
        if not pts:
            new.append(name)
            continue
        if len(pts) < MIN_HISTORY:
            ungated.append((name, len(pts)))
            continue
        # FAIL-CLOSED, per-window value: every value inside the median window must itself be a
        # finite non-negative number — a negative history point must not hide behind a positive
        # median. Zeros stay legal here: the stable-zero carve-out below needs all-zero windows.
        bad_hist = [p for p in pts if not _is_num(p) or p < 0]
        if bad_hist:
            invalid.append((name, f"history window contains non-finite/negative value(s) "
                                  f"{bad_hist!r} — fail-closed (published history is corrupt "
                                  f"for this metric)"))
            continue
        med = float(statistics.median(pts))
        larger_better = bool(LARGER_IS_BETTER_RE.search(name))
        det = is_deterministic(name, unit)
        # The metric's OWN observations — this run's value plus its median window — are what
        # bound the resolution allowance from below (see resolution_quantum).
        quantum = resolution_quantum(unit, det, [val] + [float(p) for p in pts])
        # ZERO BOUNDARY — resolved by DIRECTION (see the header). The denominator of the
        # direction-adjusted ratio is the median for a smaller-is-better metric and the CURRENT
        # value for a `*_per_s` throughput; only a zero DENOMINATOR is undefined, and only that
        # case is on the metric's pessimal side. A zero NUMERATOR is the metric sitting at its
        # optimum, which is ratio 0.0 — an improvement by construction, never a regression.
        denom = val if larger_better else med
        if val == 0 and med == 0:
            # Stable zero — legitimate for deterministic zero-counts and the sub-resolution
            # timing series that live history proves are all-zero by design. Compared, not skipped.
            stable_zero.append(name)
            ratio = 1.0
        elif denom == 0:
            # PESSIMAL-side zero crossing: a real regression of unbounded ratio. Bounded ONLY by
            # the metric's measurement resolution; with no derivable quantum the ratio truly
            # cannot be bounded, so this stays fail-closed exactly as before (a `*_per_s`
            # throughput collapsing to 0 has no quantum and therefore still hard-fails).
            if quantum <= 0.0:
                invalid.append((name, f"zero-boundary change (current {val:g} vs median "
                                      f"{med:g}) on the metric's PESSIMAL side and unit "
                                      f"{unit or '<none>'!r} has no derivable measurement "
                                      f"resolution — ratio unbounded; fail-closed"))
                continue
            ratio = ratio_lower_bound(val, med, larger_better, quantum)
            zero_crossing.append((name, val, med, ratio, unit))
        else:
            # customSmallerIsBetter suite, EXCEPT `*_per_s` throughput (larger is better): invert.
            # A zero NUMERATOR lands here and yields ratio 0.0 — the metric is at its optimum.
            ratio = (med / val) if larger_better else (val / med)
            if val == 0 or med == 0:
                improved_to_zero.append((name, val, med, unit))
        # NOISE FLOOR — UNIT-AWARE: exempt ONLY allow-listed units ("us"/"milli") whose median
        # sits under that unit's measured floor (see FLOOR_EXEMPT_UNITS). Every other unit —
        # "s" seconds, "ns_per_byte", "ratio", `*_per_s`, unknown — is gated regardless of
        # magnitude (unknown units fail toward gated), and deterministic metrics are NEVER
        # exempt (belt-and-braces: no deterministic unit is allow-listed).
        #
        # THIS IS COMPUTED FOR EVERY COMPARED ROW, INCLUDING THE ZERO-BOUNDARY ONES. The previous
        # revision `continue`d out of the zero branch BEFORE reaching here, which made the floor
        # UNREACHABLE at zero — the defect that red main for a perfect vectors_hnsw_recall_at10
        # even though the "milli" floor had been added for precisely that metric (#4559).
        unit_floor = FLOOR_EXEMPT_UNITS.get(unit)
        floor_exempt = (not det) and unit_floor is not None and med < unit_floor
        # RESOLUTION EXEMPTION — narrow by construction: it can only ever apply to a row that
        # would otherwise be in the hard set, and only when the row's resolution-aware LOWER
        # BOUND does not reach the threshold. "Would hard-fail on the point estimate, but the
        # measurement cannot PROVE it reaches HARD_RATIO." Watch-only, never ungated.
        resolution_exempt = resolution_exempts(val, med, larger_better, quantum, ratio)
        rows.append((name, val, med, len(pts), ratio, det, larger_better, floor_exempt,
                     resolution_exempt))
        units[name] = unit
        quanta[name] = quantum

    compared = len(rows)
    # Floor-exempt rows stay in the watch TABLE (flagged "under noise floor — watch only") but
    # are excluded from the hard set AND from every uniform-shift computation below — their
    # degenerate small-integer ratios would distort breadth (a), uniformity (b) and the cap (c)
    # alike (the honest sub-floor history reaches 4.0x on pure noise).
    gated_rows = [r for r in rows if not r[7]]
    floor_exempt_rows = [r for r in rows if r[7]]
    # Resolution-exempt rows deliberately STAY inside gated_rows. They are excluded from `hard`
    # (they cannot fail the run alone) but keep feeding every uniform-shift condition, where an
    # inflated near-resolution ratio can only make the exemption HARDER to obtain — the
    # fail-toward-gated direction. Only the magnitude-based floor exemption removes rows outright.
    resolution_exempt_rows = [r for r in gated_rows if r[8]]
    watch = [r for r in rows if r[4] >= WATCH_RATIO]
    hard = [r for r in gated_rows if r[4] >= HARD_RATIO and not r[8]]
    # DURABLE TRAIL — floor-exempt rows at/above HARD_RATIO never red the gate, and after a few
    # accepted points the regressed median REBASES, erasing the gate-side evidence. Emit them in
    # the soft-triage comparison-row shape (`previous` = the MEDIAN-of-history, tagged in `note`)
    # so bench-triage.py --mode soft --hardzone-report merges them into the rolling deduped
    # bench-flake issue (see --report-out).
    #
    # [sq-jxeqz / #4559] RESOLUTION-exempt rows ride the SAME trail under the same key: they are
    # watch-only for exactly the same reason (a measurement the gate must not call a regression
    # alone) and need exactly the same durable record. bench-triage.py consumes `floor_exempt_hard`
    # positionally and only reads {name,current,previous,ratio_pct,unit,note}, so widening the
    # producer needs no change there — the `note` carries which exemption applied.
    def _trail_row(r: tuple, note: str) -> dict:
        return {"name": r[0], "current": r[1], "previous": r[2],
                "ratio_pct": round(r[4] * 100.0, 1), "unit": units.get(r[0], ""), "note": note}

    floor_exempt_hard = [
        _trail_row(r, f"floor-exempt hard-band (>= {HARD_RATIO:g}x median-of-history; "
                      f"watch-only for the gate)")
        for r in floor_exempt_rows if r[4] >= HARD_RATIO
    ] + [
        _trail_row(r, f"resolution-exempt hard-band (>= {HARD_RATIO:g}x median-of-history on the "
                      f"point estimate, but the metric's measurement quantum of "
                      f"{quanta.get(r[0], 0.0):g} {units.get(r[0], '')} cannot prove it; "
                      f"watch-only for the gate)")
        for r in resolution_exempt_rows
    ]

    # Uniform-shift exemption — ALL four conditions must hold (see header), computed over the
    # GATED (non-floor-exempt) rows only.
    uniform = False
    exemption_reasons: list[str] = []
    gated_watch = [r for r in gated_rows if r[4] >= WATCH_RATIO]
    if gated_rows and gated_watch:
        frac = len(gated_watch) / len(gated_rows)
        cond_a = frac >= UNIFORM_FRACTION
        if not cond_a:
            exemption_reasons.append(
                f"(a) breadth: only {len(gated_watch)}/{len(gated_rows)} gated metrics >= "
                f"{WATCH_RATIO}x (< {UNIFORM_FRACTION:.0%}; floor-exempt metrics excluded)")
        ratios = [r[4] for r in gated_watch]
        if len(ratios) >= 2:
            cv = statistics.stdev(ratios) / statistics.mean(ratios)
            cond_b = cv <= UNIFORM_MAX_CV
            if not cond_b:
                exemption_reasons.append(
                    f"(b) uniformity: stdev/mean over the watch-band ratios is {cv:.3f} "
                    f"(> {UNIFORM_MAX_CV}) — the shift is not a uniform scaling")
        else:
            cond_b = False
            exemption_reasons.append(
                "(b) uniformity: a single shifted metric is not an environment-wide scaling")
        worst = max(r[4] for r in gated_rows)
        cond_c = worst < UNIFORM_CAP_RATIO
        if not cond_c:
            exemption_reasons.append(
                f"(c) cap: worst gated ratio {worst:.2f}x >= {UNIFORM_CAP_RATIO}x — a "
                f"catastrophic regression never hides in the herd")
        # Deterministic rows are never floor-exempt, so gated_rows covers every one of them.
        det_moved = [r for r in gated_rows if r[5] and abs(r[4] - 1.0) > DET_TOLERANCE]
        cond_d = not det_moved
        if not cond_d:
            names = ", ".join(f"{r[0]} ({r[4]:.3f}x)" for r in det_moved[:5])
            exemption_reasons.append(
                f"(d) deterministic drift: {len(det_moved)} deterministic metric(s) moved "
                f"> {DET_TOLERANCE:.0%} of median ({names}) — an environment shift moves "
                f"wall-clock, never deterministic outputs")
        uniform = cond_a and cond_b and cond_c and cond_d

    report = {
        "compared": compared, "rows": rows, "watch": watch, "hard": hard,
        "floor_exempt": floor_exempt_rows, "floor_exempt_hard": floor_exempt_hard,
        "resolution_exempt": resolution_exempt_rows, "units": units,
        "gated_compared": len(gated_rows), "gated_watch": len(gated_watch),
        "invalid": invalid, "ungated": ungated, "new": new, "stable_zero": stable_zero,
        "improved_to_zero": improved_to_zero, "zero_crossing": zero_crossing,
        "disappeared": disappeared, "disappeared_frac": disappeared_frac,
        "uniform_shift": uniform, "exemption_reasons": exemption_reasons,
    }
    return gate_exit_code(report), report


def gate_exit_code(report: dict) -> int:
    """THE fail predicate — the single source of truth for the gate's exit code.

    Deliberately a named function rather than an inline computation inside `evaluate`: the
    confirm-by-re-measurement pass (apply_remeasurement) removes rows from `report["hard"]` and
    the run's outcome must then be re-derived through the IDENTICAL predicate. Two independent
    copies would be free to drift, and the copy that ran second would silently become the gate.
    """
    if report["invalid"]:
        return 1
    if report["disappeared_frac"] > DISAPPEAR_FAIL_FRACTION:
        return 1
    if report["hard"] and not report["uniform_shift"]:
        return 1
    return 0


def _gating_label(floor: bool, res: bool) -> str:
    """Which exemption (if any) makes a row watch-only, for the table's `gating` column."""
    if floor and res:
        return "under noise floor + at measurement resolution — watch only"
    if floor:
        return "under noise floor — watch only"
    if res:
        return "at measurement resolution — watch only"
    return "hard-zone"


def _table_lines(entries: list[tuple], markdown: bool) -> list[str]:
    """Render [(name, cur, med, n, ratio, det, larger_better, floor, resolution)] rows."""
    out = []
    if markdown:
        out.append("| metric | current | median (n) | ratio | direction | gating |")
        out.append("|---|---:|---:|---:|---|---|")
        for name, cur, med, n, ratio, _det, lb, floor, res in sorted(entries, key=lambda t: -t[4]):
            out.append(f"| `{name}` | {cur:.6g} | {med:.6g} (n={n}) | {ratio:.2f}x |"
                       f" {'larger-is-better' if lb else 'smaller-is-better'} |"
                       f" {_gating_label(floor, res)} |")
    else:
        out.append(f"  {'metric':<60} {'current':>14} {'median(n)':>18} {'ratio':>7}")
        for name, cur, med, n, ratio, _det, lb, floor, res in sorted(entries, key=lambda t: -t[4]):
            suffix = "  [larger-is-better]" if lb else ""
            if floor:
                suffix += "  [under noise floor — watch only]"
            if res:
                suffix += "  [at measurement resolution — watch only]"
            out.append(f"  {name:<60} {cur:>14.4g} {med:>13.4g}(n={n}) {ratio:>6.2f}x{suffix}")
    return out


def render_report(report: dict, markdown: bool) -> list[str]:
    """Human-readable report lines (plain for stdout, markdown for GITHUB_STEP_SUMMARY)."""
    h2 = "## " if markdown else ""
    lines: list[str] = [f"{h2}bench hard zone — median-of-history gate"]
    relieved = report.get("remeasure_relieved") or []
    # Report the hard band as MEASURED (confirmed + relieved), never only the confirmed count:
    # after the confirmation pass `report["hard"]` holds the reproduced rows alone, so quoting it
    # as "N gated >= HARD_RATIOx" would silently under-report what this run actually measured.
    lines.append(f"{report['compared']} metrics compared; {len(report['watch'])} >= "
                 f"{WATCH_RATIO}x median; {len(report['hard']) + len(relieved)} gated >= "
                 f"{HARD_RATIO}x median"
                 + (f" ({len(relieved)} of them NOT REPRODUCED on re-measurement, "
                    f"{len(report['hard'])} confirmed)." if relieved else "."))
    if relieved:
        lines.append("")
        lines.append("**Re-measured — breach did NOT reproduce (watch-only):**" if markdown
                     else "re-measured after the suite; the breach did NOT reproduce "
                          "(watch-only, not failing the run):")
        for r, best, new_ratio, n_re in sorted(relieved, key=lambda t: -t[0][4]):
            if markdown:
                lines.append(f"| `{r[0]}` | first read {r[1]:.6g} ({r[4]:.2f}x) | best of "
                             f"{n_re} re-measurement(s) {best:.6g} ({new_ratio:.2f}x) | "
                             f"median {r[2]:.6g} |")
            else:
                lines.append(f"  {r[0]:<60} first {r[1]:.4g} ({r[4]:.2f}x) -> best-of-{n_re} "
                             f"{best:.4g} ({new_ratio:.2f}x) vs median {r[2]:.4g}")
        lines.append(f"  (the ORIGINAL reading is what the trend publishes — this pass only "
                     f"decides whether the row may RED the run; see apply_remeasurement)")
    if report.get("remeasure_skipped"):
        lines.append(f"  re-measurement skipped: {report['remeasure_skipped']}")
    if report["watch"]:
        lines.append("")
        lines.append(f"**Watch band (>= {WATCH_RATIO}x median):**" if markdown
                     else f"metrics >= {WATCH_RATIO}x their median of the last up-to-"
                          f"{HISTORY_WINDOW} published values:")
        lines.extend(_table_lines(report["watch"], markdown))
    if report["floor_exempt"]:
        floor_watch = sum(1 for r in report["floor_exempt"] if r[4] >= WATCH_RATIO)
        lines.append(f"{len(report['floor_exempt'])} noisy metric(s) under their unit's noise "
                     f"floor (allow-list: {_floor_desc()}): watch-only, never a hard fail alone "
                     f"(measured honest bands at that scale exceed {HARD_RATIO}x); "
                     f"{floor_watch} currently >= {WATCH_RATIO}x median (flagged in the table).")
    if report.get("resolution_exempt"):
        names = ", ".join(f"{r[0]} ({r[4]:.2f}x, median {r[2]:.6g})"
                          for r in report["resolution_exempt"])
        lines.append(f"{len(report['resolution_exempt'])} metric(s) at/above {HARD_RATIO}x on the "
                     f"point estimate but NOT provably so at their measurement resolution "
                     f"(quanta: {', '.join(f'{u} {q:g}' for u, q in sorted(RESOLUTION_QUANTA.items()))}"
                     f"): watch-only, routed to the durable trail — {names}")
    if report.get("improved_to_zero"):
        names = ", ".join(f"{n} ({c:g} vs median {m:g} {u})"
                          for n, c, m, u in report["improved_to_zero"])
        lines.append(f"{len(report['improved_to_zero'])} metric(s) reached their OPTIMUM across a "
                     f"zero boundary (compared at ratio 0.0 — an improvement by construction on "
                     f"this direction, never a regression): {names}")
    if report.get("zero_crossing"):
        names = ", ".join(f"{n} ({c:g} vs median {m:g} {u}, resolution-bounded {r:.2f}x)"
                          for n, c, m, r, u in report["zero_crossing"])
        lines.append(f"{len(report['zero_crossing'])} PESSIMAL-side zero crossing(s) — ratio "
                     f"unbounded in principle, bounded here by the unit's measurement "
                     f"resolution: {names}")
    if report["invalid"]:
        lines.append("")
        lines.append("**Fail-closed numeric errors (each one fails the run):**" if markdown
                     else "fail-closed numeric errors (each one fails the run):")
        lines.extend(f"  - `{n}`: {reason}" if markdown else f"  {n}: {reason}"
                     for n, reason in report["invalid"])
    if report["disappeared"]:
        lines.append("")
        frac = report["disappeared_frac"]
        verdict = ("FAIL — suite breakage" if frac > DISAPPEAR_FAIL_FRACTION
                   else "warn only (below the "
                        f"{DISAPPEAR_FAIL_FRACTION:.0%} suite-breakage threshold)")
        lines.append(f"**Disappeared metrics** ({len(report['disappeared'])}, "
                     f"{frac:.0%} of gateable — {verdict}):" if markdown else
                     f"DISAPPEARED metrics ({len(report['disappeared'])}, {frac:.0%} of "
                     f"gateable — {verdict}):")
        lines.extend(f"  - `{n}`" if markdown else f"  {n}" for n in report["disappeared"])
    if report["ungated"]:
        lines.append("")
        names = ", ".join(f"{n} (n={c})" for n, c in report["ungated"])
        lines.append(f"insufficient history — ungated (need >= {MIN_HISTORY} values): {names}")
    if report["new"]:
        shown = ", ".join(report["new"][:15])
        more = f" (+{len(report['new']) - 15} more)" if len(report["new"]) > 15 else ""
        lines.append(f"new metrics with no history yet (ungated): {shown}{more}")
    if report["stable_zero"]:
        lines.append(f"stable-zero metrics compared at ratio 1.0: "
                     f"{', '.join(report['stable_zero'])}")
    if report["uniform_shift"]:
        lines.append("")
        # GATED-only cap wording, matching the ::warning: the exemption predicate excludes
        # floor-exempt rows, and one of those may honestly sit at/above the cap — "no >= 4x
        # outlier" (unqualified) would be FALSE on exactly such a run.
        lines.append(f"UNIFORM-SHIFT EXEMPTION applied: all four conditions hold over the GATED "
                     f"(non-floor-exempt) metrics — breadth, ratio uniformity, no GATED metric "
                     f"reached {UNIFORM_CAP_RATIO:g}x median, deterministic metrics unmoved. "
                     f"This makes a masked code regression implausible — NOT impossible — hence "
                     f"this loud record instead of a quiet pass.")
    elif report["exemption_reasons"]:
        lines.append("")
        lines.append("uniform-shift exemption NOT applied:")
        lines.extend(f"  - {r}" for r in report["exemption_reasons"])
    return lines


def emit_report(report: dict) -> None:
    """Print the report to stdout, and to GITHUB_STEP_SUMMARY when anything is notable.

    Load-bearing: this gate runs BEFORE the Store step, so on a hard-zone failure the run
    publishes nothing and Store's comparison comment never fires — this output is the only
    visible evidence of what tripped.
    """
    for line in render_report(report, markdown=False):
        print(line)
    notable = (report["watch"] or report["hard"] or report["invalid"]
               or report["disappeared"] or report["uniform_shift"]
               # [#4559] a zero-boundary move never fails the run any more, so the SUMMARY is
               # the only place it stays visible — it must make the run "notable".
               or report.get("improved_to_zero") or report.get("zero_crossing")
               or report.get("resolution_exempt")
               # A row relieved by re-measurement stops failing the run, so — exactly like the
               # zero-boundary case above — the step summary becomes the place it stays visible.
               or report.get("remeasure_relieved"))
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if notable and summary_path:
        with open(summary_path, "a", encoding="utf-8") as f:
            f.write("\n".join(render_report(report, markdown=True)) + "\n")
    if report["uniform_shift"]:
        # GATED numbers only — the exemption predicate is computed over the gated
        # (non-floor-exempt) rows, so the annotation must report exactly those counts:
        # total-row numbers would misstate the breadth and "no metric reached 4x" would be
        # FALSE whenever a floor-exempt row honestly spiked past the cap.
        print(f"::warning title=bench hard zone — uniform environment shift, not gating::"
              f"{report['gated_watch']} of {report['gated_compared']} GATED metrics "
              f"(floor-exempt excluded) are >= {WATCH_RATIO}x their median-of-history "
              f"simultaneously, the shifted ratios are uniform (stdev/mean <= "
              f"{UNIFORM_MAX_CV}), no GATED metric reached {UNIFORM_CAP_RATIO}x, and every "
              f"deterministic metric is within {DET_TOLERANCE:.0%} of its median. Treated as "
              f"a runner-environment scaling and NOT failing the run — a masked code "
              f"regression is implausible under these four conditions, not impossible, so "
              f"review the table in the job summary.")
    for name, reason in report["invalid"]:
        print(f"::error title=bench hard zone — invalid measurement::{name}: {reason}")
    if report["disappeared_frac"] > DISAPPEAR_FAIL_FRACTION:
        print(f"::error title=bench hard zone — suite breakage::"
              f"{len(report['disappeared'])} gateable metrics "
              f"({report['disappeared_frac']:.0%}) are missing from this run's results — more "
              f"than {DISAPPEAR_FAIL_FRACTION:.0%} of the gated suite disappeared at once, "
              f"which is a bench-suite breakage, not a rename. See the disappeared list above.")
    floor_hot = [r for r in report["floor_exempt"] if r[4] >= HARD_RATIO]
    if floor_hot:
        names = ", ".join(f"{r[0]} ({r[4]:.2f}x, median {r[2]:.6g})" for r in floor_hot)
        print(f"::warning title=bench hard zone — under noise floor, watch only::{names} "
              f"at/above {HARD_RATIO}x median, but the median is under the unit's noise floor "
              f"(allow-list: {_floor_desc()}) — measured honest run-to-run bands at that scale "
              f"exceed {HARD_RATIO}x, so this cannot fail the gate alone. It stays visible "
              f"here, in the watch table, in the Store step's comparison comment, AND it is "
              f"routed via the --report-out JSON into the soft-triage deduped bench-flake "
              f"issue — the durable trail that survives the median rebasing.")
    res_hot = report.get("resolution_exempt") or []
    if res_hot:
        names = ", ".join(f"{r[0]} ({r[4]:.2f}x, median {r[2]:.6g})" for r in res_hot)
        print(f"::warning title=bench hard zone — at measurement resolution, watch only::{names} "
              f"at/above {HARD_RATIO}x median on the point estimate, but the metric cannot "
              f"EXPRESS a ratio that fine: one quantization tick of its unit already spans the "
              f"threshold, so the resolution-aware lower bound (see ratio_lower_bound) stays "
              f"under {HARD_RATIO}x. It cannot fail the gate alone. It stays visible here, in "
              f"the watch table, AND it is routed via the --report-out JSON into the soft-triage "
              f"deduped bench-flake issue — the durable trail that survives the median rebasing.")
    relieved = report.get("remeasure_relieved") or []
    if relieved:
        names = ", ".join(f"{r[0]} (first {r[4]:.2f}x, re-measured {nr:.2f}x)"
                          for r, _b, nr, _n in relieved)
        print(f"::warning title=bench hard zone — breach did NOT reproduce, watch only::{names} "
              f"reached {HARD_RATIO}x median on the first reading but NOT on an independent "
              f"re-measurement taken after the rest of the suite, so the breach is not a "
              f"property of this build and cannot red the run alone (see apply_remeasurement). "
              f"The ORIGINAL reading is still what the trend publishes, it stays in the watch "
              f"table, and it is routed via the --report-out JSON into the soft-triage deduped "
              f"bench-flake issue — the durable trail that survives the median rebasing.")
    if report["hard"] and not report["uniform_shift"]:
        for name, cur, med, n, ratio, _det, lb, _floor, _res in report["hard"]:
            direction = "median/current (larger-is-better)" if lb else "current/median"
            print(f"::error title=bench hard zone::{name} is {ratio:.2f}x its "
                  f"median-of-{n} history ({direction}: {cur:.6g} vs median {med:.6g}) — "
                  f"at/above the {HARD_RATIO}x hard threshold, and the uniform-shift exemption "
                  f"does not apply. Treating as a real regression.")


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(prog=PROG, description=__doc__)
    ap.add_argument("--results", required=True, help="this run's bench-results.json")
    ap.add_argument("--prev-data", required=True,
                    help="previously-published data.js (prev-data.js; absent => warn + pass)")
    ap.add_argument("--suite", default="sparq engine",
                    help="data.js entries suite name (EXACT match; absent => warn + pass)")
    ap.add_argument("--report-out", default=None,
                    help="write a machine-readable report JSON here (carries the "
                         "floor_exempt_hard rows bench-triage.py --hardzone-report merges "
                         "into the deduped bench-flake issue — the durable trail for "
                         "sub-floor watch-only regressions); optional")
    ap.add_argument("--remeasure-cmd", default=None,
                    help="shell command that RE-RUNS the benchmark suite and writes a "
                         "customSmallerIsBetter [{name,unit,value}] JSON to --remeasure-out. A "
                         "row that would hard-fail is only fatal if the breach REPRODUCES in "
                         "that independent measurement (see apply_remeasurement). Omitted => "
                         "no confirmation pass and behaviour identical to before.")
    ap.add_argument("--remeasure-out", default="bench-results-remeasure.json",
                    help="path the --remeasure-cmd writes its results JSON to")
    ap.add_argument("--remeasure-k", type=int, default=DEFAULT_REMEASURE_K,
                    help=f"how many independent re-measurements to attempt "
                         f"(default {DEFAULT_REMEASURE_K}); readings are combined "
                         f"least-regressed-wins, direction-aware")
    return ap


def run_gate(results_path: str, prev_data_path: str, suite: str,
             report_out: str | None = None, remeasure_fn=None,
             remeasure_k: int = DEFAULT_REMEASURE_K) -> int:
    rp = Path(results_path)
    if not rp.exists():
        print(f"::error title=bench hard zone::results file {results_path} is absent — the "
              f"benchmark step should have written it on this path; fail-closed.")
        return 1
    try:
        current = json.loads(rp.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, ValueError) as e:
        print(f"::error title=bench hard zone::could not parse {results_path} ({e}) — "
              f"fail-closed (this run's own artifact is corrupt).")
        return 1
    if not isinstance(current, list):
        current = current.get("benches", [])
    pp = Path(prev_data_path)
    if not pp.exists():
        print(f"::warning title=bench hard zone — no history::{prev_data_path} is absent "
              f"(benchmark-data branch missing or empty — first run). Nothing to compare; passing.")
        return 0
    text = pp.read_text(encoding="utf-8")
    if not text.strip():
        # The fetch step creates an EMPTY prev-data.js when the branch has no data.js yet
        # (`git show ... > prev-data.js` truncates before git show fails) — bootstrap, not corrupt.
        print(f"::warning title=bench hard zone — no history::{prev_data_path} is empty "
              f"(benchmark-data branch has no data.js yet — bootstrap). Nothing to compare; "
              f"passing.")
        return 0
    try:
        data = parse_data_js(text)
    except (json.JSONDecodeError, ValueError) as e:
        print(f"::error title=bench hard zone::could not parse non-empty {prev_data_path} "
              f"({e}) — fail-closed (the published history is corrupt; investigate the "
              f"benchmark-data branch).")
        return 1
    entries = data.get("entries", {})
    # EXACT suite match only — never fall back to a different suite's series.
    series = entries.get(suite)
    if series is None:
        print(f"::warning title=bench hard zone — suite not found::suite '{suite}' is not in "
              f"{prev_data_path} (available: {sorted(entries)}). Refusing to compare against a "
              f"different suite's series; nothing to compare; passing.")
        return 0
    if not series:
        print(f"::warning title=bench hard zone — empty history::no published points for suite "
              f"'{suite}' in {prev_data_path}; nothing to compare; passing.")
        return 0
    code, report = evaluate(current, history_values(series))
    # CONFIRM BEFORE FAILING. Runs BEFORE emit_report so the table, the annotations and the exit
    # code all describe the SAME, final decision — a relieved row must never be ::error::'d.
    if code != 0:
        apply_remeasurement(report, remeasure_fn, remeasure_k)
        code = gate_exit_code(report)
    emit_report(report)
    if report_out:
        # Written on pass AND fail (the step may fail on another metric while a floor-exempt
        # row still needs its durable trail); the Soft-zone triage step runs `if: always()`.
        Path(report_out).write_text(json.dumps(
            {"suite": suite, "hard_ratio": HARD_RATIO,
             "floor_exempt_units": FLOOR_EXEMPT_UNITS,
             # The per-unit resolution CEILINGS, so a trail row tagged "resolution-exempt" is
             # self-describing (the per-metric quantum that actually applied is in its `note`).
             "resolution_quanta": RESOLUTION_QUANTA,
             "floor_exempt_hard": report["floor_exempt_hard"]},
            indent=2) + "\n", encoding="utf-8")
        log(f"report JSON ({len(report['floor_exempt_hard'])} floor-exempt hard-band row(s)) "
            f"written to {report_out}")
    if code == 0 and not report["hard"] and not report["uniform_shift"]:
        log(f"hard zone clean: {report['compared']} metrics compared against their "
            f"median-of-history; {len(report['watch'])} in the watch band, none failing.")
    return code


# ── self-test (hermetic: inline fixtures; no network, no git, no repo writes) ────────────────────
def _series(points: list[list[tuple]]) -> list[dict]:
    """Build a data.js-shaped series from [[(name, value), ...] per commit] (oldest first)."""
    return [
        {"date": i, "benches": [{"name": n, "value": v, "unit": "us"} for n, v in benches]}
        for i, benches in enumerate(points)
    ]


def _cur(rows: list[tuple], unit: str = "us") -> list[dict]:
    return [{"name": n, "value": v, "unit": unit} for n, v in rows]


def _quiet(fn, *a, **kw):
    """Run `fn` with stdout captured; return (result, captured_text).

    LOAD-BEARING, not tidiness. emit_report writes GitHub workflow commands (`::error::`,
    `::warning::`) to stdout, and Actions turns any such line into a real annotation REGARDLESS of
    which process wrote it. A self-test fixture that drives the real emitter therefore publishes
    FAILURE-LEVEL annotations that are textually indistinguishable from a live finding: run
    30426908184 emitted three, two of them fixtures, and the first human read of it counted three
    regressions where there was one. Capturing keeps fixture output inside the test, and lets the
    check PIN the emitted text rather than discard it.
    """
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        res = fn(*a, **kw)
    return res, buf.getvalue()


def self_test() -> int:
    failures: list[str] = []
    checks = 0

    def check(cond: bool, what: str) -> None:
        nonlocal checks
        checks += 1
        if cond:
            log(f"self-test ok: {what}")
        else:
            failures.append(what)
            log(f"self-test FAIL: {what}")

    # Baseline 5-commit history: four metrics stable at 100.0 — deliberately ABOVE NOISE_FLOOR
    # so these fixtures keep exercising the hard zone proper (sub-floor behavior is tested in
    # its own section below).
    series = _series([[("a", 100.0), ("b", 100.0), ("c", 100.0), ("d", 100.0)]] * 5)

    # 1. Median semantics: window trims to the last 5 VALUES per metric.
    hist = history_values(_series([[("m", 100.0)], [("m", 1.0)], [("m", 2.0)], [("m", 3.0)],
                                   [("m", 4.0)], [("m", 5.0)]]))
    check(hist["m"] == [1.0, 2.0, 3.0, 4.0, 5.0], "history window keeps only the last 5 values")
    check(statistics.median(hist["m"]) == 3.0, "median of 5-value history is the middle value")
    hist_even = history_values(_series([[("m", 1.0)], [("m", 2.0)], [("m", 3.0)], [("m", 8.0)]]))
    check(statistics.median(hist_even["m"]) == 2.5, "median of an even count averages the middle two")
    code, rep = evaluate(_cur([("m", 150.0)]), history_values(
        _series([[("m", 100.0)], [("m", 100.0)], [("m", 100.0)], [("m", 100.0)],
                 [("m", 300.0)]])))
    check(code == 0 and not rep["hard"],
          "single outlier history point does not move the median (1.5x vs median passes)")

    # 2. SPARSE-metric per-metric windowing: a metric present in only 5 of 10 commits still
    #    collects all 5 of its values (the old per-commit window judged it from 1 point).
    sparse_pts: list[list[tuple]] = []
    for i in range(10):
        commit: list[tuple] = [("dense", 100.0)]
        if i % 2 == 0:
            commit.append(("sparse", 100.0 + 10 * i))  # 100,120,140,160,180 at commits 0,2,4,6,8
        sparse_pts.append(commit)
    hist_sparse = history_values(_series(sparse_pts))
    check(hist_sparse["sparse"] == [100.0, 120.0, 140.0, 160.0, 180.0],
          "sparse metric collects its last 5 values across the FULL retained history")
    code, rep = evaluate(_cur([("sparse", 150.0), ("dense", 100.0)]), hist_sparse)
    check(code == 0 and rep["compared"] == 2,
          "sparse metric is gated against its own 5-value median (140.0), not a 1-point window")

    # 3. Insufficient history (< 3 values) => listed as ungated, not silently gated or failed.
    two_pt = history_values(_series([[("young", 10.0)], [("young", 10.0)]]))
    code, rep = evaluate(_cur([("young", 100.0)]), two_pt)
    check(code == 0 and rep["compared"] == 0 and rep["ungated"] == [("young", 2)],
          "a metric with only 2 history values is ungated + listed (even at 10x)")

    # 4. Absent history: no overlapping metrics => pass (exit 0), nothing compared.
    code, rep = evaluate(_cur([("new_metric", 42.0)]), {})
    check(code == 0 and rep["compared"] == 0 and rep["new"] == ["new_metric"],
          "absent history passes with nothing compared; metric listed as new")

    # 5. Fail-closed numerics: NaN / non-numeric / negative current, NaN in the history
    #    window, and zero-boundary changes all hard-fail with a per-metric record.
    code, rep = evaluate(_cur([("a", float("nan"))]), history_values(series))
    check(code == 1 and len(rep["invalid"]) == 1, "NaN current value fails closed")
    code, rep = evaluate([{"name": "a", "value": "fast", "unit": "us"}], history_values(series))
    check(code == 1 and len(rep["invalid"]) == 1, "non-numeric current value fails closed")
    code, rep = evaluate(_cur([("a", -1.0)]), history_values(series))
    check(code == 1 and len(rep["invalid"]) == 1, "negative current value fails closed")
    #    [FABLE-5 round 4] bool and out-of-float-range int currents: isinstance(int,float)
    #    admits bool, and math.isfinite(10**400) raises OverflowError — both must land in
    #    the fail-closed invalid list, never pass as 1/0 and never crash the gate.
    code, rep = evaluate(_cur([("a", True)]), history_values(series))
    check(code == 1 and len(rep["invalid"]) == 1, "boolean current value fails closed")
    code, rep = evaluate(_cur([("a", 10**400)]), history_values(series))
    check(code == 1 and len(rep["invalid"]) == 1,
          "out-of-float-range int current (10**400) fails closed — OverflowError guarded")
    huge_hist = history_values(_series([[("a", 10.0)], [("a", 10**400)], [("a", 10.0)],
                                        [("a", 10.0)]]))
    code, rep = evaluate(_cur([("a", 10.0)]), huge_hist)
    check(code == 1 and len(rep["invalid"]) == 1,
          "out-of-float-range int inside the history window fails closed — no crash")
    bad_hist = history_values(_series([[("a", 10.0)], [("a", 10.0)],
                                       [("a", float("nan"))], [("a", 10.0)]]))
    code, rep = evaluate(_cur([("a", 10.0)]), bad_hist)
    check(code == 1 and len(rep["invalid"]) == 1, "NaN inside the history window fails closed")
    #    Pre-history validation: a bad CURRENT value must fail even when the metric has no or
    #    young history — never slide out through the "new"/"ungated" early exits.
    code, rep = evaluate(_cur([("brandnew", float("nan"))]), {})
    check(code == 1 and len(rep["invalid"]) == 1 and not rep["new"],
          "NaN current on a NO-history metric fails closed (not silently listed as new)")
    young_hist = history_values(_series([[("young", 10.0)], [("young", 10.0)]]))
    code, rep = evaluate([{"name": "young", "value": "junk", "unit": "us"}], young_hist)
    check(code == 1 and len(rep["invalid"]) == 1 and not rep["ungated"],
          "non-numeric current on a young-history metric fails closed (not silently ungated)")
    code, rep = evaluate(_cur([("fresh", -3.0)]), {})
    check(code == 1 and len(rep["invalid"]) == 1 and not rep["new"],
          "negative current on a no-history metric fails closed")
    #    Per-window negativity: a negative history point must not hide behind a positive median.
    neg_hist = history_values(_series([[("a", -1.0)], [("a", 10.0)], [("a", 10.0)]]))
    code, rep = evaluate(_cur([("a", 10.0)]), neg_hist)
    check(code == 1 and len(rep["invalid"]) == 1,
          "negative history value inside the window fails closed despite a positive median")
    zero_series = history_values(_series([[("z", 0.0)]] * 5))
    code, rep = evaluate(_cur([("z", 5.0)]), zero_series)
    check(code == 1 and len(rep["invalid"]) == 1,
          "positive current vs zero median on a unit with NO derivable resolution is a "
          "zero-boundary change — fails closed")
    code, rep = evaluate(_cur([("z", 0.0)]), zero_series)
    check(code == 0 and rep["stable_zero"] == ["z"] and rep["compared"] == 1,
          "stable zero (current 0, median 0) is legitimate — compared at ratio 1.0")

    # 5b. ZERO BOUNDARY BY DIRECTION (sq-jxeqz / #4559). The previous revision routed EVERY
    #     zero boundary to `invalid` (an unconditional exit 1 the uniform-shift exemption never
    #     covers) and `continue`d BEFORE the noise floor was computed. That red main for
    #     achieving a PERFECT vectors_hnsw_recall_at10 — a recall DEFICIT (bench/vector/README.md:
    #     `recall_deficit_milli = round((1 - recall@10) * 1000)`), whose 0 is recall@10 = 1.000,
    #     the best value the metric can take. Live: run 30279965305 @ cb0c6739c7, "current 0 vs
    #     median 2 — ratio undefined; fail-closed".
    #     Each check below names the mutation that must turn exactly IT red.
    #     T1 — the headline: a smaller-is-better metric AT ITS OPTIMUM cannot fail.
    #     MUTATION: restore `elif val == 0 or med == 0: invalid.append(...); continue` => red.
    code, rep = evaluate(_cur([("a", 0.0), ("b", 100.0), ("c", 100.0), ("d", 100.0)]),
                         history_values(series))
    check(code == 0 and not rep["invalid"] and not rep["hard"]
          and [r for r in rep["rows"] if r[0] == "a"][0][4] == 0.0
          and [n for n, _c, _m, _u in rep["improved_to_zero"]] == ["a"],
          "smaller-is-better metric AT ITS OPTIMUM (current 0 vs positive median) is compared "
          "at ratio 0.0 and passes — an improvement can never be a regression")
    #     T2 — the unit noise floor is REACHABLE AT ZERO. The `milli` floor was added FOR
    #     vectors_hnsw_recall_at10, but the old `continue` fired before floor_exempt was ever
    #     computed, so the metric the floor exists for could never reach it.
    #     MUTATION: move the floor_exempt computation back below a zero-boundary `continue` => red.
    hnsw = history_values([{"date": i, "benches": [
        {"name": "vectors_hnsw_recall_at10", "value": 2.0, "unit": "milli"}]} for i in range(5)])
    code, rep = evaluate([{"name": "vectors_hnsw_recall_at10", "value": 0.0, "unit": "milli"}],
                         hnsw)
    check(code == 0 and [r[0] for r in rep["floor_exempt"]] == ["vectors_hnsw_recall_at10"]
          and rep["compared"] == 1 and not rep["invalid"],
          "the unit noise floor is REACHABLE at zero — a floor-exempt metric at its optimum is "
          "COMPARED and floor-flagged, not routed to invalid before the floor is computed")
    #     T3 — QUANTIFIER DIRECTION: "a zero is an improvement" is true only on the OPTIMUM side.
    #     A `*_per_s` THROUGHPUT COLLAPSING TO ZERO is the metric's PESSIMAL value and must still
    #     hard-fail. MUTATION: treat any `val == 0` as the optimum (drop the `denom` direction
    #     switch, i.e. `denom = med` unconditionally) => red.
    thr0 = history_values(_series([[("rsp_x_triples_per_s", 1000.0)]] * 5))
    code, rep = evaluate(_cur([("rsp_x_triples_per_s", 0.0)], unit="triples_per_s"), thr0)
    check(code == 1 and len(rep["invalid"]) == 1 and not rep["improved_to_zero"],
          "a `_per_s` throughput COLLAPSING to zero is the PESSIMAL side — still fails closed "
          "(a zero is only an improvement on the metric's optimum side)")
    #     T4 — the mirror image: a throughput RECOVERING from a zero median IS an improvement.
    thr_zero = history_values(_series([[("rsp_x_triples_per_s", 0.0)]] * 5))
    code, rep = evaluate(_cur([("rsp_x_triples_per_s", 1000.0)], unit="triples_per_s"), thr_zero)
    check(code == 0 and not rep["invalid"]
          and [n for n, _c, _m, _u in rep["improved_to_zero"]] == ["rsp_x_triples_per_s"],
          "a `_per_s` throughput recovering FROM a zero median is an improvement (ratio 0.0)")
    #     T5 — a PESSIMAL-side zero crossing on a unit WITH a known quantum is bounded, not
    #     undefined — and a big one still REDs. MUTATION: make the pessimal branch pass
    #     unconditionally => red.
    s_zero = history_values(_series([[("closure_s", 0.0)]] * 5))
    code, rep = evaluate(_cur([("closure_s", 0.05)], unit="s"), s_zero)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["invalid"]
          and len(rep["zero_crossing"]) == 1,
          "a PESSIMAL-side zero crossing far above the unit's resolution still HARD-FAILS "
          "(bounded by the quantum, not silently undefined)")
    #     ...while a crossing of ONE tick off a zero median cannot be distinguished from noise.
    code, rep = evaluate(_cur([("closure_s", 0.001)], unit="s"), s_zero)
    check(code == 0 and not rep["hard"] and len(rep["zero_crossing"]) == 1
          and abs(rep["zero_crossing"][0][3] - 1.0) < 1e-9,
          "a ONE-TICK pessimal zero crossing is resolution-bounded to 1.00x — listed, not fatal")

    # 5c. MEASUREMENT RESOLUTION (sq-jxeqz / #4559). sameas_size32_closure_s is scraped from a
    #     `{:.3}s` print, so one tick is 1 ms; it publishes 0.001 on every retained point and one
    #     tick up is EXACTLY the inclusive >= 2.0x threshold. Live: run 30279965305 failed on
    #     "0.002 vs median 0.001 — 2.00x".
    #     T6 — one tick no longer fails. MUTATION: delete RESOLUTION_QUANTA["s"] (or the
    #     `resolution_exempt` term in the `hard` filter) => red.
    tick = history_values(_series([[("sameas_size32_closure_s", 0.001)]] * 5))
    code, rep = evaluate(_cur([("sameas_size32_closure_s", 0.002)], unit="s"), tick)
    check(code == 0 and not rep["hard"] and len(rep["resolution_exempt"]) == 1
          and abs(rep["resolution_exempt"][0][4] - 2.0) < 1e-9,
          "a 1 ms-quantized seconds metric moving ONE tick (0.001 -> 0.002 = exactly 2.00x) is "
          "watch-only — the metric cannot express a ratio that fine")
    #     ...and it is routed to the DURABLE TRAIL under the key bench-triage.py consumes, so the
    #     evidence survives the median rebasing that the now-green run performs.
    check(len(rep["floor_exempt_hard"]) == 1
          and rep["floor_exempt_hard"][0]["name"] == "sameas_size32_closure_s"
          and rep["floor_exempt_hard"][0]["previous"] == 0.001
          and "resolution-exempt hard-band" in rep["floor_exempt_hard"][0]["note"],
          "the resolution-exempt hit rides the soft-triage durable trail, tagged with its reason")
    #     T7 — CONTROL, the SAME metric: the real 5.00x incident (run 30235240748, 0.005 vs
    #     0.001) STILL HARD-FAILS. This is the proof the metric was not silenced.
    code, rep = evaluate(_cur([("sameas_size32_closure_s", 0.005)], unit="s"), tick)
    check(code == 1 and len(rep["hard"]) == 1
          and rep["hard"][0][0] == "sameas_size32_closure_s" and not rep["resolution_exempt"],
          "the SAME metric at the real 5.00x incident value STILL hard-fails — the resolution "
          "allowance is one tick wide, not a silenced metric")
    #     T8 — the allowance is ABSOLUTE (1.5 quanta), NOT a widened ratio threshold. HONEST
    #     DISCLOSURE: absolute does not mean zero. On a 0.4 s metric the 1.5 ms band is 0.19% of
    #     the 2.0x point, so a metric sitting at EXACTLY 2.0000x is inside it (the cost of only
    #     failing when the resolution PROVES the threshold) — but 0.19% past it REDs, where the
    #     1 ms closure needs 3.5x. Pinned from both sides so the band cannot silently widen.
    #     MUTATION: turn the exemption into a per-unit RATIO exemption => the second check reds.
    load = history_values(_series([[("load_s", 0.4)]] * 5))
    code, rep = evaluate(_cur([("load_s", 0.8)], unit="s"), load)
    check(code == 0 and len(rep["resolution_exempt"]) == 1,
          "an `s` metric at 0.4 s is inside the absolute 1.5 ms band at EXACTLY 2.0000x "
          "(disclosed cost: 0.19% of the threshold)")
    code, rep = evaluate(_cur([("load_s", 0.8016)], unit="s"), load)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["resolution_exempt"],
          "the same 0.4 s metric HARD-FAILS 0.2% past 2.0x — the allowance is 1.5 ms absolute, "
          "not a ratio widening (the 1 ms closure needs 3.5x for the same allowance)")
    #     T8b — THE UNIT IS ONLY A CEILING. snikmeta_decode_s is an `s` metric whose real
    #     emitter prints 6 decimals and whose MEDIAN is below one 1 ms tick; a blanket per-unit
    #     1 ms allowance would exceed its median several times over and silence its gate
    #     completely. Per-metric inference must cut the quantum to what its values demonstrate.
    #     MUTATION: drop observed_quantum() from resolution_quantum() (use the unit ceiling
    #     directly) => this reds.
    snik = history_values(_series([[("snikmeta_decode_s", 0.000462)]] * 5))
    code, rep = evaluate(_cur([("snikmeta_decode_s", 0.001386)], unit="s"), snik)  # 3.00x
    check(code == 1 and len(rep["hard"]) == 1 and not rep["resolution_exempt"],
          "a sub-millisecond `s` metric TRIPLING still HARD-FAILS — with the unit ceiling used "
          "directly its 1 ms allowance would exceed its own median and exempt even this")
    check(observed_quantum([0.000462, 0.000924], 0.001) == 1e-6
          and observed_quantum([0.001, 0.002], 0.001) == 0.001
          and observed_quantum([0.388, 0.3, 0.547], 0.001) == 0.001
          and observed_quantum([2.0, 1.0], 1.0) == 1.0
          and observed_quantum([0.0, 0.0], 0.001) == 0.001,
          "observed_quantum: coarsest decimal step every non-zero value is a multiple of, "
          "capped by the unit ceiling; zeros constrain nothing; result is a canonical power of 10")
    check(all(abs(q - 10.0 ** round(math.log10(q))) < 1e-18 for q in RESOLUTION_QUANTA.values()),
          "every RESOLUTION_QUANTA ceiling is an exact power of ten (observed_quantum walks "
          "decimal EXPONENTS and relies on it)")
    #     T9 — the allowance boundary pinned from BOTH sides, in values the `{:.3}s` emitter can
    #     actually PRINT (a 1 ms-quantized metric can never report 0.0035). Against a 0.001
    #     median the exempt band is exactly {0.002, 0.003} and 0.004 REDs.
    #     MUTATION: widening the half-tick to a full tick moves the boundary => the second reds.
    code, rep = evaluate(_cur([("sameas_size32_closure_s", 0.003)], unit="s"), tick)
    check(code == 0 and len(rep["resolution_exempt"]) == 1,
          "TWO ticks (0.003 vs 0.001, 3.00x) is still inside the resolution allowance")
    code, rep = evaluate(_cur([("sameas_size32_closure_s", 0.004)], unit="s"), tick)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["resolution_exempt"],
          "THREE ticks (0.004 vs 0.001) HARD-FAILS — the allowance ends inside the values this "
          "metric can actually print, so it is a real gate, not a silenced one")
    #     T10 — `milli` metrics ABOVE the noise floor keep a gate that is only ~1.5 milli wide.
    #     The vectors_diskann/pq recall deficits (~34 / ~22) sit above the milli floor and stay
    #     hard-gated; the resolution allowance costs them 1.5 milli, disclosed and pinned here.
    #     (They are ALSO exact-gated by bench/vector/run.sh against expected.tsv and ratcheted by
    #     scripts/perf-gate.py, so this lane is not their only line of defence.)
    dk = history_values([{"date": i, "benches": [
        {"name": "vectors_diskann_recall_at10", "value": 34.0, "unit": "milli"}]}
        for i in range(5)])
    code, rep = evaluate([{"name": "vectors_diskann_recall_at10", "value": 70.0,
                           "unit": "milli"}], dk)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["floor_exempt"],
          "an above-floor `milli` deficit doubling (34 -> 70) still HARD-FAILS — a real recall "
          "regression on the gated recall family is detected")
    #     T11 — deterministic metrics get NO resolution allowance, even if their unit ever gains
    #     a quantum. MUTATION: drop the `det` guard in resolution_quantum() => red.
    check(resolution_quantum("s", False, [0.001, 0.002]) == 0.001
          and resolution_quantum("s", True, [0.001, 0.002]) == 0.0
          and resolution_quantum("mystery_unit", False, [0.001, 0.002]) == 0.0
          and resolution_quantum("count", False, [1000.0, 2000.0]) == 0.0,
          "resolution_quantum: allow-listed unit only, and NEVER for a deterministic metric "
          "(a 1000/2000 count series would otherwise infer a quantum of 1000 and silence itself)")
    #     T12 — ratio_lower_bound math, both directions, including the zero-median case the
    #     plain ratio cannot express. MUTATION: any sign/side error here reds.
    check(abs(ratio_lower_bound(0.002, 0.001, False, 0.001) - 1.0) < 1e-12
          and abs(ratio_lower_bound(0.005, 0.001, False, 0.001) - 3.0) < 1e-12
          and abs(ratio_lower_bound(0.001, 0.0, False, 0.001) - 1.0) < 1e-12
          and abs(ratio_lower_bound(500.0, 1000.0, True, 100.0) - 1.7272727272727273) < 1e-9,
          "ratio_lower_bound: least-regressed reading in both directions, total at a zero median")
    #     T12b — the bound is CLAMPED at 0 and never returns a negative ratio, for any quantum a
    #     caller passes (evaluate() only ever passes a quantum that divides the values, but the
    #     helper is public and a negative "ratio" would corrupt every comparison downstream).
    #     MUTATION: drop the max(..., 0.0) clamps => red.
    check(ratio_lower_bound(0.0, 1.0, False, 4.0) == 0.0
          and ratio_lower_bound(1.0, 0.0, True, 4.0) == 0.0,
          "ratio_lower_bound clamps at 0 — a coarse quantum never yields a negative ratio")
    #     T13 — THE SECOND-ORDER LOOP, closed. This fix makes a perfect 0 PUBLISH, so the recall
    #     deficit's MEDIAN can now legitimately become 0 — and the very next honest run at 1 is a
    #     PESSIMAL-side zero crossing. Without the `milli` quantum that is fail-closed and main
    #     REDS AGAIN for the same reason, one publication later. The integer-milli quantum
    #     (bench/vector/README.md: the deficit is `round(...)`) is what keeps it bounded at 1.00x.
    #     MUTATION: delete RESOLUTION_QUANTA["milli"] => red.
    hnsw0 = history_values([{"date": i, "benches": [
        {"name": "vectors_hnsw_recall_at10", "value": v, "unit": "milli"}]}
        for i, v in enumerate([0.0, 0.0, 0.0, 1.0, 0.0])])
    code, rep = evaluate([{"name": "vectors_hnsw_recall_at10", "value": 1.0, "unit": "milli"}],
                         hnsw0)
    check(code == 0 and not rep["invalid"] and len(rep["zero_crossing"]) == 1
          and abs(rep["zero_crossing"][0][3] - 1.0) < 1e-9,
          "once a perfect 0 has PUBLISHED and the median is 0, the next honest 1-milli deficit "
          "is resolution-bounded to 1.00x — the loop does not simply move one publication later")
    #     T14 — resolution_exempts(): BOTH conjuncts pinned.
    #     MUTATION: dropping `ratio >= HARD_RATIO` reds the first; `<` -> `<=` reds the second.
    check(not resolution_exempts(1.2, 1.0, False, 1.0, 1.2),
          "resolution_exempts is False below the hard band — it never flags a row that would "
          "not hard-fail anyway (which would flood the durable trail)")
    check(not resolution_exempts(3.5, 1.0, False, 1.0, 3.5)
          and resolution_exempts(3.4, 1.0, False, 1.0, 3.4),
          "resolution_exempts is STRICT at the threshold: a bound landing exactly ON "
          f"{HARD_RATIO:g}x is proven to reach it and stays gated")
    #     ...and the same "below the hard band is not on the trail" property end-to-end.
    code, rep = evaluate(_cur([("sameas_size32_closure_s", 0.001)], unit="s"), tick)
    check(code == 0 and not rep["resolution_exempt"] and not rep["floor_exempt_hard"],
          "an unchanged resolution-scale metric is neither resolution-exempt nor on the trail")
    #     T15 — the CURRENT value constrains the inferred quantum, not just the history window.
    #     A run reporting 0.0025 PROVES the emitter resolves to 1e-4, so the 1 ms allowance is no
    #     longer justified and the move REDs. MUTATION: infer from `pts` only => red.
    code, rep = evaluate(_cur([("sameas_size32_closure_s", 0.0025)], unit="s"), tick)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["resolution_exempt"],
          "a current value with finer granularity than the window SHRINKS the allowance — "
          "0.0025 vs median 0.001 proves 1e-4 resolution and hard-fails")
    #     T16 — the markdown table names WHICH exemption applies. This column is what a
    #     maintainer reads in GITHUB_STEP_SUMMARY, and on a green run it is the only record.
    #     MUTATION: make _gating_label always return "hard-zone" => red.
    code, rep = evaluate(_cur([("sameas_size32_closure_s", 0.002)], unit="s"), tick)
    md = "\n".join(render_report(rep, markdown=True))
    check("at measurement resolution — watch only" in md and "| hard-zone |" not in md,
          "the markdown table's gating column names the resolution exemption")
    check(_gating_label(True, False) == "under noise floor — watch only"
          and _gating_label(False, True) == "at measurement resolution — watch only"
          and _gating_label(True, True).count("watch only") == 1
          and _gating_label(False, False) == "hard-zone",
          "_gating_label distinguishes all four exemption combinations")
    #     T17 — a zero-boundary/resolution event makes the run NOTABLE, so the step summary is
    #     still written even though the run is now GREEN. This gate runs before the Store step,
    #     so on a green run this summary is the only place the event is recorded.
    #     MUTATION: drop the new terms from `notable` => red.
    code, rep = evaluate(_cur([("a", 0.0), ("b", 100.0), ("c", 100.0), ("d", 100.0)]),
                         history_values(series))
    saved = os.environ.pop("GITHUB_STEP_SUMMARY", None)
    try:
        with tempfile.TemporaryDirectory(prefix="bench-hardzone-selftest-") as td17:
            sf = Path(td17) / "summary.md"
            os.environ["GITHUB_STEP_SUMMARY"] = str(sf)
            with contextlib.redirect_stdout(io.StringIO()):
                emit_report(rep)
            written = sf.read_text(encoding="utf-8") if sf.exists() else ""
    finally:
        if saved is not None:
            os.environ["GITHUB_STEP_SUMMARY"] = saved
        else:
            os.environ.pop("GITHUB_STEP_SUMMARY", None)
    check(code == 0 and "reached their OPTIMUM" in written,
          "a GREEN run whose only event is a zero-boundary improvement still writes the step "
          "summary — the event stays visible after it stopped being fatal")

    # 6. Disappeared metrics: warn below the 30% threshold, hard-fail above it.
    code, rep = evaluate(_cur([("a", 10.0), ("b", 10.0), ("c", 10.0)]), history_values(series))
    check(code == 0 and rep["disappeared"] == ["d"] and rep["disappeared_frac"] == 0.25,
          "1 of 4 gateable metrics disappeared (25%) => loud warn-list, no fail")
    code, rep = evaluate(_cur([("a", 10.0), ("b", 10.0)]), history_values(series))
    check(code == 1 and rep["disappeared"] == ["c", "d"],
          "2 of 4 gateable metrics disappeared (50% > 30%) => suite breakage, hard fail")

    # 7. `_per_s` direction inversion (throughput: larger is better).
    thr = history_values(_series([[("rsp_x_triples_per_s", 1000.0)]] * 5))
    code, rep = evaluate(_cur([("rsp_x_triples_per_s", 500.0)], unit="triples_per_s"), thr)
    check(code == 1 and len(rep["hard"]) == 1 and abs(rep["hard"][0][4] - 2.0) < 1e-9,
          "_per_s throughput halving is a 2.0x regression (ratio inverted) — hard fail")
    code, rep = evaluate(_cur([("rsp_x_triples_per_s", 2000.0)], unit="triples_per_s"), thr)
    check(code == 0 and not rep["watch"],
          "_per_s throughput doubling is an improvement (ratio 0.5) — passes")

    # 8. Single-metric hard fail: one metric >= 2.0x median, the rest normal => exit 1.
    lone = _cur([("a", 210.0), ("b", 102.0), ("c", 99.0), ("d", 100.0)])
    code, rep = evaluate(lone, history_values(series))
    check(code == 1 and not rep["uniform_shift"] and len(rep["hard"]) == 1
          and rep["hard"][0][0] == "a",
          "single sustained >= 2.0x regression hard-fails (no uniform exemption)")
    watch_only = _cur([("a", 180.0), ("b", 100.0), ("c", 100.0), ("d", 100.0)])
    code, rep = evaluate(watch_only, history_values(series))
    check(code == 0 and len(rep["watch"]) == 1 and not rep["hard"],
          "a lone watch-band metric (1.8x, < 2.0x) is reported but does not fail")

    # 9. Uniform-shift exemption POSITIVE: broad (3/4 >= 1.75x), uniform ratios (cv <= 0.15),
    #    nothing >= 4x, no deterministic metric moved => exempt, exit 0.
    shifted = _cur([("a", 190.0), ("b", 195.0), ("c", 205.0), ("d", 110.0)])
    code, rep = evaluate(shifted, history_values(series))
    check(code == 0 and rep["uniform_shift"] and len(rep["watch"]) == 3 and len(rep["hard"]) == 1,
          "uniform environment shift (3/4 >= 1.75x, uniform, capped, no det drift) is exempted")

    # 10. Uniformity rejection: broad shift with NON-uniform ratios => NOT exempt => fail.
    ragged = _cur([("a", 180.0), ("b", 250.0), ("c", 350.0), ("d", 100.0)])  # 1.8/2.5/3.5
    code, rep = evaluate(ragged, history_values(series))
    check(code == 1 and not rep["uniform_shift"]
          and any("(b) uniformity" in r for r in rep["exemption_reasons"]),
          "broad but NON-uniform shift (cv > 0.15) is NOT exempt — hard fail stands")

    # 11. The 4x cap ISOLATED: conditions (a) breadth, (b) uniformity and (d) deterministic all
    #     PASS — ratios 4.05/4.1/4.08/4.06 put 4/4 metrics in the watch band with CV ~0.005 and
    #     no deterministic metrics — so ONLY the cap (c) blocks the exemption. Removing the cap
    #     from the predicate would flip exactly this check red.
    capped = _cur([("a", 405.0), ("b", 410.0), ("c", 408.0), ("d", 406.0)])
    code, rep = evaluate(capped, history_values(series))
    check(code == 1 and not rep["uniform_shift"] and len(rep["hard"]) == 4
          and len(rep["exemption_reasons"]) == 1
          and "(c) cap" in rep["exemption_reasons"][0],
          "a >= 4x metric is the SOLE exemption blocker ((a)/(b)/(d) pass) — hard fail stands")
    #     Control: the SAME uniform shape kept below the cap (all ~1.9x) IS exempt — proving the
    #     cap is the discriminating condition, not breadth/uniformity/deterministic drift.
    under_cap = _cur([("a", 190.0), ("b", 195.0), ("c", 192.0), ("d", 191.0)])
    code, rep = evaluate(under_cap, history_values(series))
    check(code == 0 and rep["uniform_shift"] and not rep["exemption_reasons"],
          "same uniform shape below the cap IS exempt — the 4x cap is the discriminating condition")

    # 12. Deterministic-metric drift blocks the exemption: same broad uniform timing shift, but a
    #     byte-count metric moved > 1% => NOT exempt => fail.
    det_series = [{"date": i, "benches": [
        {"name": "a", "value": 100.0, "unit": "us"},
        {"name": "b", "value": 100.0, "unit": "us"},
        {"name": "c", "value": 100.0, "unit": "us"},
        {"name": "wasm_bundle_bytes", "value": 1000.0, "unit": "bytes"},
    ]} for i in range(5)]
    det_cur = [{"name": "a", "value": 190.0, "unit": "us"},
               {"name": "b", "value": 195.0, "unit": "us"},
               {"name": "c", "value": 205.0, "unit": "us"},
               {"name": "wasm_bundle_bytes", "value": 1030.0, "unit": "bytes"}]  # +3%
    code, rep = evaluate(det_cur, history_values(det_series))
    check(code == 1 and not rep["uniform_shift"]
          and any("(d) deterministic" in r for r in rep["exemption_reasons"]),
          "a moved deterministic metric (bytes +3%) blocks the exemption — hard fail stands")
    det_cur_ok = [dict(r) for r in det_cur]
    det_cur_ok[3]["value"] = 1000.0  # deterministic metric unmoved => exemption restored
    code, rep = evaluate(det_cur_ok, history_values(det_series))
    check(code == 0 and rep["uniform_shift"],
          "same shift with the deterministic metric unmoved IS exempt (control case)")

    # 13. Deterministic classification is unit-first (name traps are real: *_count_us is timing).
    #     "milli" is NOISY: vectors_hnsw_recall_at10 honestly oscillates 1<->4 in the live
    #     history (randomized HNSW build at degenerate resolution) — a measurement, not a count.
    check(is_deterministic("wasm_bundle_bytes", "bytes")
          and not is_deterministic("bsbm_query01_count_us", "us")
          and not is_deterministic("rsp_persistentdict_triples_per_s", "triples_per_s")
          and not is_deterministic("vectors_hnsw_recall_at10", "milli")
          and is_deterministic("zk_compose_filter_f64_gates", ""),
          "deterministic classification: unit-first, anchored-name fallback only when unit absent")

    # 14. NOISE FLOOR — sub-floor "us" timing metric at 3.0x median: watch-only, NEVER a hard
    #     fail alone (measured honest µs-scale bands exceed 2x — see FLOOR_EXEMPT_UNITS).
    #     MUTATION CHECK vs check 15: zeroing the "us" floor in FLOOR_EXEMPT_UNITS (or removing
    #     the entry) must turn exactly THIS red.
    sub = history_values(_series([[("tiny_query_us", 4.0)]] * 5))
    code, rep = evaluate(_cur([("tiny_query_us", 12.0)]), sub)
    check(code == 0 and not rep["hard"] and len(rep["floor_exempt"]) == 1
          and len(rep["watch"]) == 1 and rep["watch"][0][0] == "tiny_query_us",
          "sub-floor timing metric at 3.0x median is watch-only — no hard fail")
    check("under noise floor — watch only" in "\n".join(render_report(rep, markdown=False)),
          "sub-floor watch row is flagged 'under noise floor — watch only' in the report")
    #     DURABLE TRAIL: the same hit is emitted in floor_exempt_hard in the soft-triage
    #     comparison shape (previous = median-of-history, tagged "floor-exempt hard-band") —
    #     the exact contract bench-triage.py --hardzone-report consumes.
    check(rep["floor_exempt_hard"] == [{
              "name": "tiny_query_us", "current": 12.0, "previous": 4.0, "ratio_pct": 300.0,
              "unit": "us",
              "note": f"floor-exempt hard-band (>= {HARD_RATIO:g}x median-of-history; "
                      f"watch-only for the gate)"}],
          "floor-exempt hard-band hit is emitted in the soft-triage comparison shape")
    code, rep = evaluate(_cur([("tiny_query_us", 7.0)]), sub)  # 1.75x: watch, below hard band
    check(code == 0 and len(rep["floor_exempt"]) == 1 and not rep["floor_exempt_hard"],
          "a sub-floor row below the hard band is NOT routed to the durable trail")

    # 15. The floor is the DISCRIMINATOR: the same 3.0x shape with its median ABOVE the floor
    #     hard-fails; and a sub-floor DETERMINISTIC metric at 2.0x still hard-fails (the floor
    #     never applies to deterministic outputs — a 2x on a count/bytes/gates value is real).
    over = history_values(_series([[("big_query_us", 60.0)]] * 5))
    code, rep = evaluate(_cur([("big_query_us", 180.0)]), over)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["floor_exempt"],
          "the same 3.0x shape with median above the floor hard-fails")

    # 15a. The "us" floor SITS AT 40.0, not 20.0 — re-derived 2026-07-25 from the published
    #      history (see FLOOR_EXEMPT_UNITS: the honest band only drops under 2.0x at >= 40µs;
    #      the 20-40µs bucket honestly reaches 2.23x). This pair pins the boundary from BOTH
    #      sides so neither a silent revert to 20.0 nor an open-ended raise passes:
    #        - a 30µs-median metric at 2.0x is watch-only (would have RED main on run
    #          30154655798, whose two hard failures had medians 20.6 and 39.5),
    #        - a 45µs-median metric at the same 2.0x still hard-fails.
    #      MUTATION CHECK: setting the "us" floor back to 20.0 turns the first check red;
    #      raising it to >= 45.0 (or 90.0) turns the second red. The boundary is exclusive
    #      (`med < floor`), so a metric whose median is EXACTLY the floor stays gated.
    band = history_values(_series([[("midband_query_us", 30.0)]] * 5))
    code, rep = evaluate(_cur([("midband_query_us", 60.0)]), band)
    check(code == 0 and not rep["hard"] and len(rep["floor_exempt"]) == 1
          and len(rep["floor_exempt_hard"]) == 1,
          "20-40µs metric at 2.0x is watch-only under the re-derived 40µs floor (not a hard fail)")
    above = history_values(_series([[("aboveband_query_us", 45.0)]] * 5))
    code, rep = evaluate(_cur([("aboveband_query_us", 90.0)]), above)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["floor_exempt"],
          ">= 40µs metric at the SAME 2.0x still hard-fails — 40.0 is the discriminating floor")
    exact = history_values(_series([[("exact_floor_query_us", 40.0)]] * 5))
    code, rep = evaluate(_cur([("exact_floor_query_us", 80.0)]), exact)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["floor_exempt"],
          "a median EXACTLY at the floor stays gated (`med < floor` is exclusive)")
    det_small = history_values([{"date": i, "benches": [
        {"name": "solid_fixture_count", "value": 5.0, "unit": "count"}]} for i in range(5)])
    code, rep = evaluate([{"name": "solid_fixture_count", "value": 10.0, "unit": "count"}],
                         det_small)
    check(code == 1 and len(rep["hard"]) == 1 and not rep["floor_exempt"],
          "sub-floor DETERMINISTIC metric at 2.0x still hard-fails (floor never applies)")

    # 15b. UNIT-AWARE floor ([FABLE-5] round-2 review): the floor is an explicit per-unit
    #      allow-list ("us"/"milli"), NOT a raw magnitude comparison. The published history
    #      carries nine `s`-unit series (load_s, text_build_s, rdfs_infer_s, ... at ~0.4-15 s)
    #      whose RAW medians sit under 20 — a raw floor wrongly exempted ALL of them, so a
    #      stable 0.4s -> 4s (10x) load regression would pass and rebase the median.
    #      MUTATION CHECK: adding "s" to FLOOR_EXEMPT_UNITS must turn exactly THIS red.
    load_hist = history_values(_series([[("load_s", 0.4)]] * 5))
    code, rep = evaluate(_cur([("load_s", 4.0)], unit="s"), load_hist)
    check(code == 1 and len(rep["hard"]) == 1 and rep["hard"][0][0] == "load_s"
          and not rep["floor_exempt"],
          "s-unit metric (median 0.4 s) at 10x median HARD-FAILS — seconds are never "
          "floor-exempt regardless of magnitude")
    #      Every other non-allow-listed unit with a small median stays gated too — including
    #      an UNKNOWN unit and a missing unit (allow-list fails toward gated, never open).
    for u in ("ns_per_byte", "ratio", "mystery_unit", ""):
        code, rep = evaluate(_cur([("small_metric", 12.0)], unit=u),
                             history_values(_series([[("small_metric", 4.0)]] * 5)))
        check(code == 1 and len(rep["hard"]) == 1 and not rep["floor_exempt"],
              f"unit {u!r} (median 4) at 3.0x median hard-fails — not in the floor allow-list")
    #      "milli" IS allow-listed: the empirically-degenerate HNSW recall family (honest
    #      1<->4 oscillation at integer-milli resolution) stays watch-only under the floor.
    code, rep = evaluate(_cur([("vectors_hnsw_recall_at10", 4.0)], unit="milli"),
                         history_values(_series([[("vectors_hnsw_recall_at10", 1.0)]] * 5)))
    check(code == 0 and not rep["hard"] and len(rep["floor_exempt"]) == 1
          and len(rep["floor_exempt_hard"]) == 1,
          "milli-unit metric (median 1) at 4.0x is floor-exempt (HNSW recall family) and "
          "routed to the durable trail")

    # 16. Uniform-shift computations EXCLUDE floor-exempt metrics — degenerate sub-floor ratios
    #     must distort neither breadth (a) nor uniformity (b) nor the cap (c).
    mix_series = [{"date": i, "benches":
                   [{"name": n, "value": 100.0, "unit": "us"} for n in "abcd"]
                   + [{"name": f"tiny{j}_us", "value": 4.0, "unit": "us"} for j in range(5)]}
                  for i in range(5)]
    #     4 gated metrics uniformly ~2.05x (a hard fail without the exemption) + 5 sub-floor
    #     metrics unmoved: excluding the floor-exempt rows, breadth is 4/4 => exempt (exit 0);
    #     counting them, breadth would be 4/9 (44% < 50%) => condition (a) fails => exit 1.
    mix_cur = _cur([("a", 210.0), ("b", 205.0), ("c", 208.0), ("d", 195.0)]
                   + [(f"tiny{j}_us", 4.0) for j in range(5)])
    code, rep = evaluate(mix_cur, history_values(mix_series))
    check(code == 0 and rep["uniform_shift"] and len(rep["floor_exempt"]) == 5,
          "breadth excludes floor-exempt metrics (4/4 gated uniform => exempt; 4/9 would fail)")
    #     A sub-floor metric spiking 4.0x (the honest vectors_hnsw 1<->4 shape) must block
    #     neither the cap (c) nor uniformity (b) for a genuine environment shift.
    mix_cur2 = _cur([("a", 210.0), ("b", 205.0), ("c", 208.0), ("d", 195.0), ("tiny0_us", 16.0)]
                    + [(f"tiny{j}_us", 4.0) for j in range(1, 5)])
    code, rep = evaluate(mix_cur2, history_values(mix_series))
    check(code == 0 and rep["uniform_shift"],
          "a 4.0x sub-floor spike blocks neither the cap nor uniformity (excluded from both)")

    # 16b. The uniform-shift ::warning reports GATED numerator/denominator + "no GATED metric
    #      reached" wording — the predicate is gated-rows-only, so total-row numbers would
    #      misstate the breadth (5/9 here) and "no metric reached 4x" would be FALSE (the
    #      floor-exempt tiny0_us honestly sits at exactly 4.0x in this fixture).
    buf = io.StringIO()
    saved_summary = os.environ.pop("GITHUB_STEP_SUMMARY", None)  # keep the CI summary clean
    try:
        with tempfile.TemporaryDirectory(prefix="bench-hardzone-selftest-") as td16:
            summary_file = Path(td16) / "step-summary.md"
            os.environ["GITHUB_STEP_SUMMARY"] = str(summary_file)
            with contextlib.redirect_stdout(buf):
                emit_report(rep)
            summary = summary_file.read_text(encoding="utf-8") if summary_file.exists() else ""
    finally:
        if saved_summary is not None:
            os.environ["GITHUB_STEP_SUMMARY"] = saved_summary
        else:
            os.environ.pop("GITHUB_STEP_SUMMARY", None)
    out = buf.getvalue()
    check("4 of 4 GATED metrics" in out and "no GATED metric reached" in out,
          "uniform-shift annotation reports the gated numerator/denominator + GATED wording")
    check("5 of 9" not in out and "no metric reached" not in out,
          "uniform-shift annotation no longer reports total-row numbers")
    # [FABLE-5 round 3] the stdout report AND the step-summary prose use the gated-only cap
    # wording too — the floor-exempt tiny0_us sits at exactly 4.0x in this fixture, so the
    # old unqualified "no >= 4x outlier" claim would be FALSE in both sinks.
    check("no GATED metric reached" in summary and summary.strip() != "",
          "step-summary exemption prose claims the cap over GATED metrics only")
    check(">= 4x outlier" not in out and ">= 4x outlier" not in summary,
          "neither stdout nor the step summary carries the unqualified 4x-outlier claim")

    # 17. Argument handling: the parser accepts the documented flags on a fixture path.
    ns = build_parser().parse_args(
        ["--results", "fixtures/bench-results.json", "--prev-data", "fixtures/prev-data.js",
         "--suite", "sparq engine"])
    check(ns.results == "fixtures/bench-results.json"
          and ns.prev_data == "fixtures/prev-data.js" and ns.suite == "sparq engine",
          "argument parser maps --results/--prev-data/--suite onto the expected namespace")

    # 18. End-to-end run_gate over real files in a private tempdir: suite-name mismatch must
    #     warn + exit 0 (never fall back to another suite), exact match must gate.
    with tempfile.TemporaryDirectory(prefix="bench-hardzone-selftest-") as td:
        results = Path(td) / "bench-results.json"
        results.write_text(json.dumps(_cur([("a", 210.0), ("b", 100.0), ("c", 100.0),
                                            ("d", 100.0)])), encoding="utf-8")
        prev = Path(td) / "prev-data.js"
        payload = {"entries": {"sparq engine": _series(
            [[("a", 100.0), ("b", 100.0), ("c", 100.0), ("d", 100.0)]] * 5)}}
        prev.write_text("window.BENCHMARK_DATA = " + json.dumps(payload) + ";",
                        encoding="utf-8")
        # QUIET — see _quiet(): these fixtures drive the REAL emitter, so without a capture their
        # `::error::` workflow commands become genuine GitHub annotations, indistinguishable from
        # a live finding. Capturing also lets each check PIN what was emitted instead of
        # discarding it.
        rc, out = _quiet(run_gate, str(results), str(prev), "some other suite")
        check(rc == 0 and "is not in" in out,
              "suite-name mismatch warns + exits 0 (no cross-suite fallback)")
        rc, out = _quiet(run_gate, str(results), str(prev), "sparq engine")
        check(rc == 1 and "::error title=bench hard zone::a is 2.10x" in out,
              "exact suite match gates for real (2.1x lone regression fails end-to-end)")
        empty = Path(td) / "empty-prev.js"
        empty.write_text("", encoding="utf-8")
        rc, out = _quiet(run_gate, str(results), str(empty), "sparq engine")
        check(rc == 0 and "::error" not in out,
              "empty prev-data.js (bootstrap fetch artifact) warns + exits 0")
        corrupt = Path(td) / "corrupt-prev.js"
        corrupt.write_text("window.BENCHMARK_DATA = {not json", encoding="utf-8")
        rc, out = _quiet(run_gate, str(results), str(corrupt), "sparq engine")
        check(rc == 1 and "could not parse non-empty" in out,
              "non-empty unparsable prev-data.js fails closed (corrupt published history)")
        # --report-out: the run writes the machine-readable report JSON carrying the
        # floor-exempt hard-band rows (the soft-triage durable-trail hand-off).
        sub_results = Path(td) / "sub-results.json"
        sub_results.write_text(json.dumps(_cur([("tiny_us", 12.0)])), encoding="utf-8")
        sub_prev = Path(td) / "sub-prev.js"
        sub_prev.write_text("window.BENCHMARK_DATA = " + json.dumps(
            {"entries": {"sparq engine": _series([[("tiny_us", 4.0)]] * 5)}}) + ";",
            encoding="utf-8")
        report_path = Path(td) / "hardzone-report.json"
        rc, _out = _quiet(run_gate, str(sub_results), str(sub_prev), "sparq engine",
                          str(report_path))
        payload = json.loads(report_path.read_text(encoding="utf-8"))
        check(rc == 0 and [r["name"] for r in payload["floor_exempt_hard"]] == ["tiny_us"]
              and "floor-exempt hard-band" in payload["floor_exempt_hard"][0]["note"]
              and payload["floor_exempt_hard"][0]["previous"] == 4.0,
              "--report-out writes the floor-exempt hard-band rows for the soft-triage trail")

    # 19. CONFIRM-BY-RE-MEASUREMENT (sq-jxeqz). The gate may only red main on a timing breach
    #     that REPRODUCES. Every check below drives the real `evaluate` -> `apply_remeasurement`
    #     -> `gate_exit_code` path, and the CONTROL pair (19a vs 19b) is what proves the
    #     mechanism discriminates rather than relieving everything it is handed.
    def _mk(cur_val, hist_val, name="q_us", unit="us", n=5):
        cur = [{"name": name, "value": cur_val, "unit": unit}]
        hist = {name: [hist_val] * n}
        return cur, hist

    # 19a. CONTROL — a breach that REPRODUCES is still fatal. Without this the whole mechanism
    #      could be "relieve everything" and every other check here would still pass.
    #      MUTATION: make apply_remeasurement relieve unconditionally => THIS goes red.
    cur, hist = _mk(2400.0, 1000.0)
    code, rep = evaluate(cur, hist)
    check(code == 1 and len(rep["hard"]) == 1, "19a precondition: 2.40x lone breach hard-fails")
    apply_remeasurement(rep, lambda: [{"name": "q_us", "value": 2300.0, "unit": "us"}], 1)
    check(gate_exit_code(rep) == 1 and len(rep["hard"]) == 1 and not rep["remeasure_relieved"],
          "a breach that REPRODUCES on re-measurement (2.30x) still HARD-FAILS")

    # 19b. A breach that does NOT reproduce is relieved — exit 0, row leaves `hard`, and the
    #      ORIGINAL reading is preserved for the trend.
    #      MUTATION: delete the apply_remeasurement call in run_gate => the end-to-end check
    #      (19j) goes red; delete the `report["hard"] = still_hard` assignment => THIS goes red.
    cur, hist = _mk(2400.0, 1000.0)
    code, rep = evaluate(cur, hist)
    apply_remeasurement(rep, lambda: [{"name": "q_us", "value": 1010.0, "unit": "us"}], 1)
    check(gate_exit_code(rep) == 0 and rep["hard"] == []
          and len(rep["remeasure_relieved"]) == 1
          and rep["remeasure_relieved"][0][1] == 1010.0
          and abs(rep["remeasure_relieved"][0][2] - 1.01) < 1e-9
          and rep["rows"][0][1] == 2400.0,
          "a breach that does NOT reproduce (1.01x) is relieved, and the ORIGINAL 2400 reading "
          "is left in `rows` for the trend")

    # 19c. THRESHOLD — a re-measured best landing EXACTLY on HARD_RATIO is NOT relieved.
    #      MUTATION: relax `new_ratio >= HARD_RATIO` to `>` => THIS goes red.
    cur, hist = _mk(3000.0, 1000.0)
    code, rep = evaluate(cur, hist)
    apply_remeasurement(rep, lambda: [{"name": "q_us", "value": 2000.0, "unit": "us"}], 1)
    check(gate_exit_code(rep) == 1 and len(rep["hard"]) == 1,
          "a re-measurement landing EXACTLY on 2.0x is still a breach (the bound is inclusive)")
    #      ...and one ULP under it IS relieved — the threshold is real, not a silenced metric.
    cur, hist = _mk(3000.0, 1000.0)
    code, rep = evaluate(cur, hist)
    apply_remeasurement(rep, lambda: [{"name": "q_us", "value": 1999.0, "unit": "us"}], 1)
    check(gate_exit_code(rep) == 0 and len(rep["remeasure_relieved"]) == 1,
          "a re-measurement just UNDER 2.0x is relieved — the threshold discriminates")

    # 19d. DIRECTION — the soundness check. `*_per_s` is larger-is-better, so the least-regressed
    #      reading is the MAXIMUM. Taking `min` unconditionally would treat the WORST throughput
    #      as the best reading and relieve a genuine collapse.
    #      MUTATION: change best_reading to `min(values)` unconditionally => THIS goes red.
    check(best_reading([100.0, 400.0], True) == 400.0
          and best_reading([100.0, 400.0], False) == 100.0,
          "best_reading is direction-aware: max for larger-is-better, min for smaller-is-better")
    cur = [{"name": "t_per_s", "value": 400.0, "unit": "triples_per_s"}]
    code, rep = evaluate(cur, {"t_per_s": [1000.0] * 5})
    check(code == 1 and len(rep["hard"]) == 1,
          "19d precondition: a throughput COLLAPSE (400 vs 1000 = 2.50x) hard-fails")
    apply_remeasurement(rep, lambda: [{"name": "t_per_s", "value": 420.0,
                                       "unit": "triples_per_s"}], 1)
    check(gate_exit_code(rep) == 1 and len(rep["hard"]) == 1,
          "a REPRODUCED throughput collapse stays fatal — the re-measure cannot invert direction")

    # 19e. FAIL-CLOSED: a re-measurement that emits nothing relieves nothing.
    #      MUTATION: treat an empty reading list as relief => THIS goes red.
    cur, hist = _mk(2400.0, 1000.0)
    code, rep = evaluate(cur, hist)
    apply_remeasurement(rep, lambda: [], 1)
    check(gate_exit_code(rep) == 1 and len(rep["hard"]) == 1,
          "an EMPTY re-measurement relieves nothing (fail-closed)")

    # 19f. FAIL-CLOSED: a re-measurement that RAISES relieves nothing, and does not crash the gate.
    def _boom():
        raise RuntimeError("bench runner died")
    cur, hist = _mk(2400.0, 1000.0)
    code, rep = evaluate(cur, hist)
    _quiet(apply_remeasurement, rep, _boom, 1)
    check(gate_exit_code(rep) == 1 and len(rep["hard"]) == 1 and rep["remeasure_attempts"] == 0,
          "a re-measurement that RAISES relieves nothing and is not fatal to the gate")

    # 19g. FAIL-CLOSED: a re-measurement that omits the tripping metric relieves nothing, even
    #      though the attempt itself succeeded. Without this a runner that silently skipped the
    #      suite (the `|| true` guards in ci-bench.sh) would turn every trip green.
    #      The property holds because the reading pool is SEEDED with the original measurement,
    #      so a pool that gained nothing re-derives the original ratio. Both the mechanism and
    #      the behaviour are pinned, because the behaviour alone cannot distinguish a correct
    #      implementation from one that never relieves anything.
    #      MUTATION: seed the pool empty (`readings = {n: [] for n in wanted}`) => THIS goes red.
    cur, hist = _mk(2400.0, 1000.0)
    code, rep = evaluate(cur, hist)
    apply_remeasurement(rep, lambda: [{"name": "other_us", "value": 1.0, "unit": "us"}], 1)
    check(gate_exit_code(rep) == 1 and len(rep["hard"]) == 1 and rep["remeasure_attempts"] == 1
          and rep["remeasure_readings"]["q_us"][:1] == [2400.0],
          "a re-measurement that OMITS the tripping metric relieves nothing — the reading pool "
          "is seeded with the ORIGINAL measurement (fail-closed without a branch of its own)")

    # 19g2. ZERO-BOUNDARY rows reach the hard set (a pessimal zero crossing whose resolution-aware
    #      lower bound proves it passes HARD_RATIO), and re-deriving a ratio from a ZERO median
    #      would divide by zero. The pass must DECLINE them, leaving `evaluate`'s dedicated
    #      zero-boundary decision standing — not crash, and not invent a second semantics.
    #      MUTATION: drop the `med <= 0 or best <= 0` guard => THIS goes red (ZeroDivisionError).
    code, rep = evaluate([{"name": "x_s", "value": 0.010, "unit": "s"}], {"x_s": [0.0] * 5})
    check(code == 1 and len(rep["hard"]) == 1 and rep["hard"][0][2] == 0.0,
          "19g2 precondition: a pessimal zero crossing above resolution IS in the hard set, "
          "with a ZERO median")
    apply_remeasurement(rep, lambda: [{"name": "x_s", "value": 0.010, "unit": "s"}], 1)
    check(gate_exit_code(rep) == 1 and len(rep["hard"]) == 1 and not rep["remeasure_relieved"],
          "a ZERO-median row is declined by the confirmation pass (fail-closed, no divide by "
          "zero, and evaluate's zero-boundary decision stands)")

    # 19h. A DETERMINISTIC row in the hard set is never re-measured — re-running cannot change a
    #      byte count, and it fails the run alone.
    #      MUTATION: delete the `any(r[5] for r in hard)` early return => THIS goes red.
    cur = [{"name": "store_bytes", "value": 200.0, "unit": "bytes"}]
    code, rep = evaluate(cur, {"store_bytes": [100.0] * 5})
    check(code == 1 and rep["hard"][0][5] is True, "19h precondition: deterministic row is hard")
    apply_remeasurement(rep, lambda: [{"name": "store_bytes", "value": 100.0,
                                       "unit": "bytes"}], 1)
    check(gate_exit_code(rep) == 1 and rep["remeasure_attempts"] == 0
          and "deterministic" in rep.get("remeasure_skipped", ""),
          "a DETERMINISTIC hard row is never re-measured and stays fatal")

    # 19i. gate_exit_code is the SINGLE source of truth: relieving every hard row does NOT rescue
    #      a run that is failing for another reason.
    #      MUTATION: return 0 from run_gate whenever `hard` empties => THIS goes red.
    cur = [{"name": "q_us", "value": 2400.0, "unit": "us"},
           {"name": "bad_us", "value": float("nan"), "unit": "us"}]
    code, rep = evaluate(cur, {"q_us": [1000.0] * 5, "bad_us": [1.0] * 5})
    apply_remeasurement(rep, lambda: [{"name": "q_us", "value": 1010.0, "unit": "us"}], 1)
    check(rep["hard"] == [] and gate_exit_code(rep) == 1,
          "relieving every hard row does NOT rescue a run failing on an invalid measurement")

    # 19j. END-TO-END through run_gate + the CLI seam, with a real --remeasure-cmd, and the
    #      annotation contract: a relieved row must be a ::warning::, never a ::error::.
    #      MUTATION: delete the `apply_remeasurement` call in run_gate => THIS goes red.
    with tempfile.TemporaryDirectory(prefix="bench-hardzone-selftest-") as td19:
        res19 = Path(td19) / "bench-results.json"
        res19.write_text(json.dumps(_cur([("q_us", 2400.0)])), encoding="utf-8")
        prev19 = Path(td19) / "prev-data.js"
        prev19.write_text("window.BENCHMARK_DATA = " + json.dumps(
            {"entries": {"sparq engine": _series([[("q_us", 1000.0)]] * 5)}}) + ";",
            encoding="utf-8")
        trail19 = Path(td19) / "report.json"
        rc, out = _quiet(run_gate, str(res19), str(prev19), "sparq engine", str(trail19),
                         lambda: [{"name": "q_us", "value": 1010.0, "unit": "us"}], 1)
        payload19 = json.loads(trail19.read_text(encoding="utf-8"))
        check(rc == 0 and "::error" not in out
              and "did NOT reproduce" in out
              and [r["name"] for r in payload19["floor_exempt_hard"]] == ["q_us"]
              and "NOT REPRODUCED" in payload19["floor_exempt_hard"][0]["note"]
              and payload19["floor_exempt_hard"][0]["current"] == 2400.0,
              "end-to-end: an unreproduced breach exits 0, is ::warning'd not ::error'd, and "
              "rides the durable trail carrying its ORIGINAL reading")
        # ...and the same fixture with a REPRODUCING re-measure still exits 1 and ::error::s.
        rc, out = _quiet(run_gate, str(res19), str(prev19), "sparq engine", None,
                         lambda: [{"name": "q_us", "value": 2300.0, "unit": "us"}], 1)
        check(rc == 1 and "::error title=bench hard zone::q_us is 2.40x" in out,
              "end-to-end control: a REPRODUCED breach still exits 1 and is ::error'd")
        # ...and with NO remeasure_fn the behaviour is byte-identical to before this change.
        rc, out = _quiet(run_gate, str(res19), str(prev19), "sparq engine")
        check(rc == 1 and "::error title=bench hard zone::q_us is 2.40x" in out
              and "did NOT reproduce" not in out,
              "no --remeasure-cmd => no confirmation pass and the pre-existing behaviour stands")
        # remeasure_via_shell honours the documented contract, and a FAILING command relieves
        # nothing. MUTATION: ignore the exit code in remeasure_via_shell => the second half reds.
        out19 = Path(td19) / "remeasure.json"
        payload_txt = "[{\"name\":\"q_us\",\"unit\":\"us\",\"value\":1010}]"
        good = remeasure_via_shell(f"printf '%s' '{payload_txt}' > {out19}", str(out19))
        # The FAILING command writes a payload that WOULD relieve the row — otherwise the exit
        # code could be ignored with no observable difference and this check would be vacuous.
        bad = remeasure_via_shell(f"printf '%s' '{payload_txt}' > {out19}; exit 3", str(out19))
        got_good, (got_bad, _txt) = good(), _quiet(bad)
        check(got_good == [{"name": "q_us", "unit": "us", "value": 1010}] and got_bad == [],
              "remeasure_via_shell reads the command's JSON, and a NON-ZERO exit discards even a "
              "well-formed relieving payload")

    # 19k. The parser exposes the confirmation flags with the documented defaults.
    ns19 = build_parser().parse_args(["--results", "r.json", "--prev-data", "p.js",
                                      "--remeasure-cmd", "scripts/ci-bench.sh"])
    check(ns19.remeasure_cmd == "scripts/ci-bench.sh"
          and ns19.remeasure_k == DEFAULT_REMEASURE_K
          and ns19.remeasure_out == "bench-results-remeasure.json",
          "parser exposes --remeasure-cmd/--remeasure-k/--remeasure-out with documented defaults")

    # 20. SELF-TEST ANNOTATION CONTAINMENT (class-level guard). Every fixture above drives the
    #     real emitter, and Actions promotes ANY `::error::`/`::warning::` line to a real
    #     annotation no matter who printed it — so a fixture leak is indistinguishable from a
    #     live finding (run 30426908184 emitted three; two were fixtures). This asserts the
    #     WHOLE self-test, run as a subprocess exactly as bench.yml runs it, emits none. It is a
    #     class guard, not a spot fix: a future check that forgets _quiet() reds HERE.
    #     MUTATION: drop the _quiet() wrapper from any run_gate call above => THIS goes red.
    if os.environ.get("BENCH_HARDZONE_SELFTEST_INNER") != "1":
        env = dict(os.environ, BENCH_HARDZONE_SELFTEST_INNER="1", PYTHONDONTWRITEBYTECODE="1")
        proc = subprocess.run([sys.executable, "-B", os.path.abspath(__file__), "--self-test"],
                              capture_output=True, text=True, env=env, check=False)
        leaked = [ln for ln in (proc.stdout + proc.stderr).splitlines()
                  if ln.lstrip().startswith(("::error", "::warning", "::notice"))]
        check(proc.returncode == 0 and not leaked,
              "the self-test emits ZERO GitHub annotations — fixture output can never be "
              f"mistaken for a live finding (leaked: {leaked[:2]})")

    if failures:
        log(f"self-test FAILED ({len(failures)}/{checks} checks): " + "; ".join(failures))
        return 1
    log(f"self-test OK ({checks} checks)")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    args = build_parser().parse_args(argv)
    fn = (remeasure_via_shell(args.remeasure_cmd, args.remeasure_out)
          if args.remeasure_cmd else None)
    return run_gate(args.results, args.prev_data, args.suite, args.report_out,
                    remeasure_fn=fn, remeasure_k=args.remeasure_k)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
