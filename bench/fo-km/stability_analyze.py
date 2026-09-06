#!/usr/bin/env python3
# [OPUS-5] sq-mztg8.3 (FO-KM epic; design research/fo-llm-bridge.md §4.2 Metric 3 /
# §6 Phase 6). 🤖 SPARQ agent — the deterministic grader for the Metric-3 probe.
#
# WHAT THIS IS
# --------------------------------------------------------------------------------
# Turns a file of raw per-session answers into the per-model / per-arm stability table
# in STABILITY.md. Like Metric 1's analyze.py it is DETERMINISTIC with NO model in the
# loop (design §5.2 anti-circularity): every number below is a count over a closed
# four-label vocabulary.
#
# INPUT — one JSON object per line (bench/fo-km/metric3-sessions.jsonl):
#   {"model": "<served model id>", "arm": "ungrounded|gufo|schema-org",
#    "session": <int>, "answer": "p01: OCCURRENT\np02: ...\n"}
# The RAW answer text is stored, not a pre-parsed label map, so the parse step is part
# of what a reader can audit rather than something the run already decided.
#
# THE FIVE REPORTED QUANTITIES
#   1. cs   — CROSS-SESSION CONTRADICTION RATE. The headline, and the direct analogue of
#             the Köhler-Neuhaus measure: over the probes answered in >= 2 sessions of a
#             cell, the fraction whose session labels are NOT unanimous. A model with a
#             stable ontological commitment scores 0.
#   2. dis  — DISSENT RATE, 1 - mean(modal count / sessions). The graded form of `cs`:
#             `cs` counts a probe with one dissenting session the same as a probe that
#             was answered five different ways, `dis` does not. Reported because with a
#             small K the binary measure saturates easily.
#   3. ws   — WITHIN-SESSION INCONSISTENCY. Over the fixture's generic/instance pairs,
#             the fraction of (pair, session) cells where a session puts a KIND in one
#             category and a MEMBER of that kind in another. This is a contradiction
#             inside a single session, so it cannot be explained by sampling across
#             sessions — it is the stronger of the two instability signals.
#   4. und  — UNDECIDABLE RATE (decisiveness guard). An arm can score a perfect `cs`
#             purely by answering UNDECIDABLE to everything. Without this column that
#             degenerate arm would read as the most stable. `cs_dec` re-computes the
#             contradiction rate over probes where the cell committed in EVERY session,
#             so a scaffold cannot buy stability by abstaining.
#   5. adh  — SCAFFOLD ADHERENCE. Over the SC probes for which the arm's own overlay
#             entails a label (stability_probes.jsonl `fo_label`), the fraction of
#             session answers equal to it. This is adherence to THAT ARM'S ontology, not
#             to any universal truth — the arms disagree with each other by design, so
#             the column is only ever read down a single arm's row.
# `cs`, `dis` and `adh` are also broken out per stratum (SC scaffolded / US unscaffolded),
# because the US stratum is the one an in-context FO cannot solve by lookup — it is where
# "does the scaffold TRANSFER" is actually answered.
#
# MISSING-ANSWER POLICY (explicit, because it sets the denominators)
# --------------------------------------------------------------------------------
# A session may fail to answer a probe (omitted line, or a line whose label is outside
# the vocabulary and is therefore rejected). Each quantity states what it does with that:
#   * cs / dis — denominator is the probes with >= 2 OBSERVED labels in the cell. One
#                observation cannot agree or disagree with anything, so it is dropped;
#                a probe observed in 4 of 5 sessions IS scored, over those 4.
#   * cs_dec   — denominator is the probes that are BOTH complete (observed in EVERY
#                session of the cell) AND decisive (no session answered UNDECIDABLE).
#                Incompleteness cannot inflate it: an abstention and a silence are both
#                a failure to commit, and the column exists precisely to stop a cell
#                buying stability by not committing.
#   * und      — denominator is the answers actually given, not sessions x probes.
#   * ws / adh — denominators are the (pair, session) / (probe, session) cells where the
#                needed labels are present.
# Every cell reports `unparsed_lines`, `missing_answers`, `probes_scored` and
# `complete_probes` so the denominators are visible rather than implied, and `--strict`
# fails the run closed when any cell has an unparsed line or a missing answer.
#
# Usage:
#   python3 bench/fo-km/stability_analyze.py                       # self-check, no run
#   python3 bench/fo-km/stability_analyze.py bench/fo-km/metric3-sessions.jsonl
#   python3 bench/fo-km/stability_analyze.py <sessions> --json out.json
#   python3 bench/fo-km/stability_analyze.py <sessions> --strict    # fail on missing data

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_PROBES = os.path.join(HERE, "stability_probes.jsonl")
DEFAULT_SESSIONS = os.path.join(HERE, "metric3-sessions.jsonl")

LABELS = ("OCCURRENT", "CONTINUANT", "ABSTRACT", "UNDECIDABLE")
ARMS = ("ungrounded", "gufo", "schema-org")
STRATA = ("SC", "US")

# `p07: CONTINUANT`, tolerating surrounding whitespace, a bullet, or bold markers — a
# formatting slip is not an ontological commitment, so it is parsed rather than scored
# as a contradiction. A line whose LABEL is outside the vocabulary is NOT tolerated: it
# is rejected as unparsed and shows up in the cell's `unparsed lines` note.
ANSWER_LINE = re.compile(r"^\s*[-*>\s]*\**\s*(p\d{2})\**\s*[:\-]\s*\**\s*([A-Za-z]+)\**\s*\.?\s*$")


def parse_answer(text: str, probe_ids: set[str]) -> tuple[dict[str, str], list[str]]:
    """Raw session text -> ({probe_id: LABEL}, [rejected lines]).

    Last write wins if a session repeats a probe id (it has restated its own answer;
    the final statement is the one it stands behind).
    """
    labels: dict[str, str] = {}
    rejected: list[str] = []
    for line in (text or "").splitlines():
        if not line.strip():
            continue
        m = ANSWER_LINE.match(line)
        if not m:
            rejected.append(line.strip())
            continue
        pid, lab = m.group(1), m.group(2).upper()
        if pid not in probe_ids or lab not in LABELS:
            rejected.append(line.strip())
            continue
        labels[pid] = lab
    return labels, rejected


def _rate(hits: int, total: int) -> float | None:
    return round(hits / total, 3) if total else None


def _fmt(v: float | None) -> str:
    return f"{v:.2f}" if v is not None else "  - "


def fmt_row(model: str, arm: str, g: dict) -> str:
    """One published table row. Shared by the report and the self-check, so the numbers
    the self-check pins are literally the ones a reader sees in STABILITY.md."""
    sc, us = g["by_stratum"]["SC"], g["by_stratum"]["US"]
    return (f"{model:26s} {arm:11s} {g['sessions']:>2d} {_fmt(g['cs']):>5s} "
            f"{_fmt(g['dissent']):>5s} {_fmt(g['ws']):>5s} {_fmt(g['undecidable']):>5s} "
            f"{_fmt(g['cs_decided']):>6s} {_fmt(g['adherence']):>5s}   "
            f"{_fmt(sc['cs'])}/{_fmt(us['cs'])}")


def grade_cell(sessions: list[dict], probes: list[dict]) -> dict:
    """All the reported quantities for one (model, arm) cell."""
    by_id = {p["id"]: p for p in probes}
    parsed = [parse_answer(s.get("answer", ""), set(by_id)) for s in sessions]
    per_session = [p[0] for p in parsed]
    n_bad = sum(len(p[1]) for p in parsed)
    n_missing = sum(len(by_id) - len(ls) for ls in per_session)

    # --- 1/2. cross-session agreement, whole and per stratum ---------------------
    unstable: list[str] = []
    dissents: list[float] = []
    strat: dict[str, dict] = {k: {"unstable": [], "dissents": [], "n": 0} for k in STRATA}
    decided_unstable = decided_n = complete_n = 0
    for pid, probe in by_id.items():
        obs = [ls[pid] for ls in per_session if pid in ls]
        complete = len(obs) == len(sessions)
        complete_n += int(complete)
        if len(obs) < 2:
            continue  # a single observation cannot agree or disagree with anything
        counts = Counter(obs)
        is_unstable = len(counts) > 1
        dissent = 1.0 - counts.most_common(1)[0][1] / len(obs)
        if is_unstable:
            unstable.append(pid)
        dissents.append(dissent)
        st = strat[probe["kind"]]
        st["n"] += 1
        st["dissents"].append(dissent)
        if is_unstable:
            st["unstable"].append(pid)
        # decisive subset: EVERY session of the cell answered this probe (complete), and
        # every one of those answers committed to a real category. A probe some session
        # left unanswered is not a probe the cell committed on, so it is excluded rather
        # than silently scored over the sessions that happened to answer it.
        if complete and all(o != "UNDECIDABLE" for o in obs):
            decided_n += 1
            decided_unstable += int(is_unstable)

    n_probes_scored = len(dissents)

    # --- 3. within-session generic/instance consistency --------------------------
    seen_pairs = set()
    ws_hits = ws_total = 0
    for pid, probe in by_id.items():
        other = probe.get("pair")
        if not other:
            continue
        key = tuple(sorted((pid, other)))
        if key in seen_pairs:
            continue
        seen_pairs.add(key)
        for ls in per_session:
            if pid in ls and other in ls:
                ws_total += 1
                ws_hits += int(ls[pid] != ls[other])

    # --- 4. decisiveness ---------------------------------------------------------
    all_labels = [lab for ls in per_session for lab in ls.values()]
    n_und = sum(1 for lab in all_labels if lab == "UNDECIDABLE")

    # --- 5. scaffold adherence (per arm; None where the overlay entails nothing) --
    adh_hits = adh_total = 0
    adh_strat = {k: [0, 0] for k in STRATA}
    arm = sessions[0]["arm"] if sessions else None
    for pid, probe in by_id.items():
        gold = (probe.get("fo_label") or {}).get(arm)
        if gold is None:
            continue
        for ls in per_session:
            if pid in ls:
                adh_total += 1
                hit = int(ls[pid] == gold)
                adh_hits += hit
                adh_strat[probe["kind"]][0] += hit
                adh_strat[probe["kind"]][1] += 1

    return {
        "sessions": len(sessions),
        "probes_scored": n_probes_scored,
        "unparsed_lines": n_bad,
        "missing_answers": n_missing,
        "cs": _rate(len(unstable), n_probes_scored),
        "dissent": round(sum(dissents) / len(dissents), 3) if dissents else None,
        "ws": _rate(ws_hits, ws_total),
        "ws_n": ws_total,
        "undecidable": _rate(n_und, len(all_labels)),
        "cs_decided": _rate(decided_unstable, decided_n),
        "decided_probes": decided_n,
        "complete_probes": complete_n,
        "adherence": _rate(adh_hits, adh_total),
        "by_stratum": {
            k: {
                "n": strat[k]["n"],
                "cs": _rate(len(strat[k]["unstable"]), strat[k]["n"]),
                "dissent": (round(sum(strat[k]["dissents"]) / len(strat[k]["dissents"]), 3)
                            if strat[k]["dissents"] else None),
                "adherence": _rate(adh_strat[k][0], adh_strat[k][1]),
            }
            for k in STRATA
        },
        "unstable_probes": sorted(unstable),
        "label_mix": dict(Counter(all_labels).most_common()),
    }


# --------------------------------------------------------------------------------------
# SELF-CHECK — deterministic fixtures with asserted outputs for every reported quantity.
# The fixtures are hand-computed, NOT recorded from a run of this file, so they fail when
# a formula changes rather than tracking it. They deliberately include the awkward inputs:
# a MISSING answer, a DUPLICATE probe id, a line with an out-of-vocabulary label, and an
# UNDECIDABLE answer.
# --------------------------------------------------------------------------------------

# p91/p92 are a generic/instance pair; p94 is the one US probe and has no entailed label.
# The ids are outside the real battery's range but keep its `pNN` shape, which the parser
# requires.
_FIXTURE_PROBES = [
    {"id": "p91", "kind": "SC", "pair": "p92", "fo_label": {"gufo": "OCCURRENT"}},
    {"id": "p92", "kind": "SC", "pair": "p91", "fo_label": {"gufo": "OCCURRENT"}},
    {"id": "p93", "kind": "SC", "fo_label": {"gufo": "CONTINUANT"}},
    {"id": "p94", "kind": "US"},
]

_FIXTURE_MIXED = [
    # session 1 — clean and complete.
    {"arm": "gufo", "answer": "p91: OCCURRENT\np92: CONTINUANT\np93: CONTINUANT\n"
                              "p94: ABSTRACT\n"},
    # session 2 — same commitments, reached through a restated probe (last write wins)
    # and one line whose label is outside the vocabulary (rejected, not scored).
    {"arm": "gufo", "answer": "p91: CONTINUANT\np91: OCCURRENT\n- **p92**: CONTINUANT\n"
                              "p93: CONTINUANT\np94: PROCESS\np94: ABSTRACT\n"},
    # session 3 — contradicts session 1/2 on p92, abstains on p93, never answers p94.
    {"arm": "gufo", "answer": "p91: OCCURRENT\np92: OCCURRENT\np93: UNDECIDABLE\n"},
]

# Hand-computed over _FIXTURE_MIXED. Cross-session scoring sees all four probes (p94 has
# 2 observations, enough to score); dissent = mean(0, 1/3, 1/3, 0) = 0.167; ws counts the
# p91/p92 pair in 3 sessions, split in 2 of them; und = 1 UNDECIDABLE / 11 answers.
# cs_dec covers p91 and p92 ONLY: p93 abstained, and p94 is missing from session 3 — a
# probe the cell did not answer everywhere is not a probe it committed on everywhere.
_EXPECT_MIXED = {
    "sessions": 3, "probes_scored": 4, "unparsed_lines": 1, "missing_answers": 1,
    "cs": 0.5, "dissent": 0.167, "ws": 0.667, "ws_n": 3, "undecidable": 0.091,
    "cs_decided": 0.5, "decided_probes": 2, "complete_probes": 3, "adherence": 0.667,
    "unstable_probes": ["p92", "p93"],
}
_EXPECT_MIXED_STRATA = {
    "SC": {"n": 3, "cs": 0.667, "dissent": 0.222, "adherence": 0.667},
    "US": {"n": 1, "cs": 0.0, "dissent": 0.0, "adherence": None},
}

# The degenerate arm the decisiveness guard exists for: perfect cs bought entirely by
# abstaining. cs must read 0.00 and cs_dec must read undefined, never 0.00.
_FIXTURE_ABSTAIN = [
    {"arm": "ungrounded", "answer": "p91: UNDECIDABLE\np92: UNDECIDABLE\n"
                                    "p93: UNDECIDABLE\np94: UNDECIDABLE\n"} for _ in range(2)
]
_EXPECT_ABSTAIN = {
    "sessions": 2, "probes_scored": 4, "unparsed_lines": 0, "missing_answers": 0,
    "cs": 0.0, "dissent": 0.0, "ws": 0.0, "ws_n": 2, "undecidable": 1.0,
    "cs_decided": None, "decided_probes": 0, "complete_probes": 4,
    "adherence": None,  # the ungrounded control entails no label anywhere
}

# The published table over the committed 45-session record, transcribed from STABILITY.md
# "The measured result" — every column, not just the headline, so a formula change that
# moves ANY published number is caught. Whitespace is normalised before comparison: this
# pins the VALUES, not the column padding.
#   model / arm / K / cs / dis / ws / und / cs_dec / adh / cs SC/US
_COMMITTED_TABLE = """
haiku  gufo        5 0.00 0.00 0.25 0.00 0.00 0.62 0.00/0.00
haiku  schema-org  5 0.17 0.07 0.40 0.00 0.17 0.00 0.12/0.25
haiku  ungrounded  5 0.17 0.03 0.50 0.00 0.17    - 0.12/0.25
opus   gufo        5 0.33 0.08 0.10 0.00 0.33 0.35 0.38/0.25
opus   schema-org  5 0.17 0.03 0.10 0.08 0.18 0.00 0.25/0.00
opus   ungrounded  5 0.08 0.02 0.45 0.00 0.08    - 0.12/0.00
sonnet gufo        5 0.00 0.00 0.25 0.00 0.00 0.38 0.00/0.00
sonnet schema-org  5 0.00 0.00 0.25 0.00 0.00 0.00 0.00/0.00
sonnet ungrounded  5 0.00 0.00 0.25 0.00 0.00    - 0.00/0.00
"""


def _norm(row: str) -> str:
    return " ".join(row.split())


def self_check(probes_path: str) -> int:
    """Assert every reported formula against the fixtures. 0 = pass, 1 = mismatch."""
    fails: list[str] = []

    def expect(what: str, got: object, want: object) -> None:
        if got != want:
            fails.append(f"{what}: got {got!r}, expected {want!r}")

    for name, sessions, want, want_strata in (
        ("mixed", _FIXTURE_MIXED, _EXPECT_MIXED, _EXPECT_MIXED_STRATA),
        ("abstain", _FIXTURE_ABSTAIN, _EXPECT_ABSTAIN, None),
    ):
        got = grade_cell(sessions, _FIXTURE_PROBES)
        for field, value in want.items():
            expect(f"{name}.{field}", got[field], value)
        for stratum, fields in (want_strata or {}).items():
            for field, value in fields.items():
                expect(f"{name}.{stratum}.{field}", got["by_stratum"][stratum][field], value)
    print(f"fixtures: {len(_EXPECT_MIXED) + len(_EXPECT_ABSTAIN)} asserted quantities over "
          "2 cells (missing / duplicate / invalid-label / UNDECIDABLE inputs covered)")

    # The committed record is part of the contract: it must still parse completely and
    # still re-derive the headline table published in STABILITY.md.
    if os.path.exists(DEFAULT_SESSIONS) and os.path.exists(probes_path):
        probes = _load(probes_path)
        cells: dict[tuple[str, str], list[dict]] = {}
        for r in _load(DEFAULT_SESSIONS):
            cells.setdefault((r["model"], r["arm"]), []).append(r)
        want = {_norm(r).split(" ", 2)[0] + "|" + _norm(r).split(" ", 2)[1]: _norm(r)
                for r in _COMMITTED_TABLE.strip().splitlines()}
        for (model, arm), ss in sorted(cells.items()):
            g = grade_cell(ss, probes)
            key = f"{model}|{arm}"
            expect(f"committed.{key}.row", _norm(fmt_row(model, arm, g)), want.get(key))
            expect(f"committed.{key}.unparsed_lines", g["unparsed_lines"], 0)
            expect(f"committed.{key}.missing_answers", g["missing_answers"], 0)
        expect("committed.cells", sorted(f"{m}|{a}" for m, a in cells), sorted(want))
        print(f"committed record: {len(cells)} cells, every published column re-derived "
              f"from {os.path.basename(DEFAULT_SESSIONS)}")
    else:
        print(f"committed record: SKIPPED ({os.path.basename(DEFAULT_SESSIONS)} absent)")

    for f in fails:
        print(f"  FAIL {f}")
    print(f"\nself-check: {'FAILED' if fails else 'PASSED'} ({len(fails)} mismatches)")
    return 1 if fails else 0


def _load(path: str) -> list[dict]:
    with open(path, encoding="utf-8") as fh:
        return [json.loads(l) for l in fh if l.strip()]


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="FO-KM Metric-3 stability grader")
    ap.add_argument("sessions", nargs="?", default=None,
                    help=f"session records jsonl (default {DEFAULT_SESSIONS}); omit to "
                         "run the asserted self-check of the grading contract")
    ap.add_argument("--probes", default=DEFAULT_PROBES)
    ap.add_argument("--json", dest="json_out", help="write the full per-cell JSON here")
    ap.add_argument("--strict", action="store_true",
                    help="exit nonzero if any cell has an unparsed line or a missing "
                         "answer, instead of grading over the answers that are present")
    args = ap.parse_args(argv[1:])

    probes = _load(args.probes)
    kinds = Counter(p["kind"] for p in probes)
    pairs = sum(1 for p in probes if p.get("pair")) // 2
    print(f"probes: {len(probes)} {dict(kinds)}, {pairs} generic/instance pairs, "
          f"labels {'/'.join(LABELS)}")

    if not args.sessions:
        print("\nNo session file given — self-checking the grading contract:")
        print("  * cs        : probes (>= 2 observations) whose labels are not unanimous")
        print("  * dissent   : 1 - mean(modal label count / observations)")
        print("  * ws        : generic/instance pairs a SINGLE session labels differently")
        print("  * und       : share of answers that are UNDECIDABLE (decisiveness guard)")
        print("  * cs_dec    : cs over probes answered in EVERY session with no abstention")
        print("  * adh       : answers matching the arm's OWN overlay-entailed fo_label")
        print("\nNo model is in the loop: every quantity is a count over the closed "
              "label set.\n")
        return self_check(args.probes)

    rows = _load(args.sessions)
    cells: dict[tuple[str, str], list[dict]] = {}
    for r in rows:
        cells.setdefault((r["model"], r["arm"]), []).append(r)
    print(f"sessions: {len(rows)} across {len(cells)} (model, arm) cells")

    graded = {f"{m}|{a}": grade_cell(ss, probes) for (m, a), ss in sorted(cells.items())}

    print(f"\n{'model':26s} {'arm':11s} {'K':>2s} {'cs':>5s} {'dis':>5s} {'ws':>5s} "
          f"{'und':>5s} {'cs_dec':>6s} {'adh':>5s}   cs SC/US")
    for key, g in graded.items():
        model, arm = key.split("|")
        print(fmt_row(model, arm, g))
        if g["unparsed_lines"] or g["missing_answers"]:
            print(f"{'':26s} {'':11s}  (unparsed lines {g['unparsed_lines']}, "
                  f"missing answers {g['missing_answers']})")

    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as fh:
            json.dump(graded, fh, indent=2, sort_keys=True)
        print(f"\nwrote {args.json_out}")

    incomplete = {k: (g["unparsed_lines"], g["missing_answers"]) for k, g in graded.items()
                  if g["unparsed_lines"] or g["missing_answers"]}
    if incomplete and args.strict:
        print("\n--strict: refusing to report over incomplete cells "
              "(unparsed lines, missing answers):")
        for key, (bad, miss) in sorted(incomplete.items()):
            print(f"  {key}: {bad} unparsed, {miss} missing")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
