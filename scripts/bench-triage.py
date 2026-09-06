#!/usr/bin/env python3
# [OPUS-4.8] Benchmark self-monitoring triage (noise-reduction-bench-selfmonitor).
#
# WHY — the maintainer gets @-mentioned on every benchmark alert. This script replaces the
# @-mention noise with SELF-MONITORING: it classifies each flagged benchmark into a two-zone
# model and routes it to the AUTONOMOUS-AGENT workflow instead of a human, with NO @mentions
# anywhere.
#
# TWO ZONES (thresholds derived empirically by scripts/bench-thresholds.py from the
# benchmark-data history; the values live in the workflow YAML, not here):
#   * HARD  — a regression clearly BEYOND any observed shared-runner noise. github-action-
#             benchmark's own fail-on-alert (with the hard fail-threshold) FAILS the benchmark
#             STEP directly, so an agent must fix it before merge. This script does NOT need to
#             re-implement the hard fail — it OWNS the two SOFT paths below.
#   * SOFT  — a regression that COULD be shared-runner flakiness (between the soft alert-
#             threshold and the hard fail-threshold). Do NOT notify anyone. Instead auto-file /
#             update ONE rolling deduped GitHub issue per benchmark suite (label `bench-flake`),
#             listing the flagged benchmarks + runs. No @mentions.
#
# FLOOR-EXEMPT HARD-BAND ([FABLE-5]): the median-of-history hard gate (scripts/bench_hardzone.py)
# never fails on a floor-exempt (sub-noise-floor us/milli) metric, and after a few accepted
# points the regressed median REBASES — so those hits would leave NO durable trail from the gate
# itself. `--hardzone-report` points at bench_hardzone.py's --report-out JSON; its
# `floor_exempt_hard` rows (previous = the MEDIAN-of-history, tagged "floor-exempt hard-band")
# are merged into the SOFT partition so they land in the same rolling deduped bench-flake issue.
# This NEVER weakens a gate — those rows were watch-only for the gate already.
#
# NIGHTLY ATTRIBUTION (--mode nightly): when the scheduled/nightly bench detects a regression
# vs the last green nightly baseline, rank the most likely culprit commits/PRs by intersecting
# each commit's touched paths with the crates the regressed suite exercises (a small static
# suite->crate map seeded from the bench-harness dir names). File ONE issue per regression
# cluster (label `bench-regression`, priority:P1, role:impl, area:<most-likely crate>) with the
# regression table + ranked suspect list + repro commands. No @mentions. These issues enter the
# normal autonomous dispatch frontier so agents fix them.
#
# The gating (HARD) decision stays in github-action-benchmark (fail-on-alert + fail-threshold).
# This script is invoked AFTER the action, reads its comparison output, and does the SOFT +
# nightly routing. It NEVER weakens a gate.
#
# USAGE
#   # soft-zone triage from a github-action-benchmark comparison JSON:
#   scripts/bench-triage.py --mode soft --comparison cmp.json \
#       --soft 175 --hard 200 --run-url URL
#   # nightly attribution:
#   scripts/bench-triage.py --mode nightly --comparison cmp.json \
#       --soft 175 --hard 200 --baseline-sha SHA --head-sha SHA --run-url URL
#   scripts/bench-triage.py --self-test          # hermetic; no gh, no git, no writes
from __future__ import annotations

import argparse
import json
import math
import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gh_dedupe  # noqa: E402  (sibling module; the sys.path insert above is the seam)

PROG = "bench-triage"
FLAKE_LABEL = "bench-flake"
REGRESSION_LABEL = "bench-regression"
FLAKE_MARKER = "[bench-flake]"
REGRESSION_MARKER = "[bench-regression]"

# ── suite -> crate map (seeded from the bench-harness dir names + real metric-name prefixes) ──
# Metric names in the github-action-benchmark series are prefixed by their suite/harness. We map
# each metric to the CRATES it exercises so a nightly regression can be attributed to the commits
# that touched those crates. This is a STATIC seed (a fine starting point per the directive); it
# is intentionally conservative — an unmapped metric falls back to the core engine closure, which
# every query benchmark ultimately exercises.
#
# Keys are metric-name PREFIXES (matched longest-first). Values are the crate dirs under crates/.
_PREFIX_CRATES: dict[str, list[str]] = {
    # well-known query suites — exercise the core engine + query path.
    "watdiv": ["sparq-engine", "sparq-core"],
    "sp2b": ["sparq-engine", "sparq-core"],
    "sp_": ["sparq-engine", "sparq-core"],
    "bsbm": ["sparq-engine", "sparq-core"],
    "lubm": ["sparq-engine", "sparq-core"],
    "dbpsb": ["sparq-engine", "sparq-core"],
    "sameas": ["sparq-engine", "sparq-core"],
    "snikmeta": ["sparq-engine", "sparq-core"],
    "deeptax": ["sparq-reason", "sparq-engine"],
    # core micro-op + hand-written query suites (q0N_*, op_*).
    "q0": ["sparq-engine", "sparq-core"],
    "op": ["sparq-engine", "sparq-core"],
    # deterministic layout metrics (still mapped so a nightly on them attributes correctly).
    "store": ["sparq-core"],
    "dict": ["sparq-core"],
    "parse": ["sparq-parse", "sparq-core"],
    "load": ["sparq-core"],
    "comp": ["sparq-core"],
    "wasm": ["sparq-wasm", "sparq-core"],
    # subsystem suites -> their crate.
    "fts": ["sparq-engine"],
    "text": ["sparq-engine"],
    "geo": ["sparq-geo"],
    "vectors": ["sparq-engine"],
    "vector": ["sparq-engine"],
    "rsp": ["sparq-rsp"],
    "hdt": ["sparq-hdt"],
    "solid": ["sparq-solid"],
    "nlq": ["sparq-nlq"],
    "shacl": ["sparq-shacl"],
    "zk": ["sparq-zk"],
    "rdfs": ["sparq-reason"],
}
_FALLBACK_CRATES = ["sparq-engine", "sparq-core"]


def log(msg: str) -> None:
    print(f"[{PROG}] {msg}", file=sys.stderr)


def _finite_num(v) -> float | None:
    """v as a float iff v is a REAL, FINITE number — else None. THE numeric admission gate.

    [FABLE-5 round 4] Every numeric field this script consumes from an artifact routes
    through here. Admissible inputs are int/float instances that are NOT bool (json
    true/false must never pass as 1/0), whose float() conversion succeeds — an
    out-of-float-range int like 10**400 raises OverflowError, which must DROP the value,
    never crash the triage filer — and whose converted value is finite (json.loads happily
    emits NaN/Infinity/-Infinity by default, so isfinite is load-bearing).
    """
    if not isinstance(v, (int, float)) or isinstance(v, bool):
        return None
    try:
        f = float(v)
    except (OverflowError, ValueError):
        return None
    return f if math.isfinite(f) else None


def _entry_date(e: dict) -> float:
    """Numeric sort key for a data.js entry — junk/absent dates sort as 0, never TypeError."""
    d = _finite_num(e.get("date"))
    return 0.0 if d is None else d


def metric_crates(name: str) -> list[str]:
    """Map a benchmark metric name to the crates it exercises (longest-prefix wins)."""
    lname = name.lower()
    best = None
    for prefix in sorted(_PREFIX_CRATES, key=len, reverse=True):
        if lname.startswith(prefix):
            best = prefix
            break
    return list(_PREFIX_CRATES[best]) if best else list(_FALLBACK_CRATES)


def suite_of(name: str) -> str:
    """A stable suite bucket for a metric name (for the one-rolling-issue-per-suite dedupe).

    Uses the mapped-crate set as the suite key when known, else the leading alpha token.
    """
    lname = name.lower()
    for prefix in sorted(_PREFIX_CRATES, key=len, reverse=True):
        if lname.startswith(prefix):
            # group by the mapped crate list so e.g. all watdiv/sp2b/bsbm engine benches share a
            # single rolling flake issue keyed on the engine, keeping issue count bounded.
            return "+".join(_PREFIX_CRATES[prefix])
    m = re.match(r"([a-z]+)", lname)
    return m.group(1) if m else "misc"


# ── zone classification ──────────────────────────────────────────────────────────
def classify(ratio_pct: float, soft: float, hard: float) -> str:
    """Given a github-action-benchmark regression ratio (150 == 1.5x == +50%), return the zone.

    ratio_pct is (current/previous)*100 for a smaller-is-better metric (>100 == regression).
    """
    if ratio_pct >= hard:
        return "hard"
    if ratio_pct >= soft:
        return "soft"
    return "ok"


def _to_ratio_pct(cur: float, prev: float) -> float | None:
    if prev == 0:
        return None
    ratio = (cur / prev) * 100.0
    # Two finite floats can still divide to inf (1e308 / 1e-308) — an undefined comparison.
    return ratio if math.isfinite(ratio) else None


def _parse_data_js(text: str) -> dict:
    """Parse a github-action-benchmark data.js payload (window.BENCHMARK_DATA = {...};)."""
    m = re.search(r"window\.BENCHMARK_DATA\s*=\s*", text)
    if m:
        text = text[m.end():]
    text = text.strip().rstrip(";").strip()
    return json.loads(text)


def baseline_sha(prev_data_js: str, suite: str) -> str:
    """The commit id of the most-recent published point in `suite` (the last green baseline).

    Empty string if the file / suite / commit is absent (first run → no baseline). Fail-soft:
    any parse error yields "" so the caller no-ops attribution rather than crashing.
    """
    p = Path(prev_data_js)
    if not p.exists():
        return ""
    try:
        data = _parse_data_js(p.read_text(encoding="utf-8"))
        entries = data.get("entries", {})
        series = entries.get(suite) or (next(iter(entries.values())) if entries else [])
        series = sorted((e for e in series if isinstance(e, dict)), key=_entry_date)
        if not series:
            return ""
        commit = series[-1].get("commit")
        cid = commit.get("id", "") if isinstance(commit, dict) else ""
        return str(cid) if cid else ""
    except (json.JSONDecodeError, ValueError, OSError, TypeError, AttributeError):
        return ""


def build_comparison(results_path: str, prev_data_js: str | None, suite: str) -> list[dict]:
    """Construct the normalized comparison rows from THIS run's results + the previous point.

    github-action-benchmark does not emit a stable machine-readable comparison file, so the
    workflow reconstructs one: `results_path` is this run's output-file-path (a list of
    {name,value,unit}); `prev_data_js` is the previously-published dev/bench/data.js (the last
    committed point on the benchmark-data branch). We pair each current metric with its most
    recent previous value of the SAME name in `suite`. Metrics with no previous value are
    emitted with previous=null (skipped by parse_comparison — nothing to compare). Every
    numeric field is admitted through _finite_num only: bool / NaN / inf / out-of-float-range
    values are dropped with a counted warning, never floated blindly.
    """
    current = json.loads(Path(results_path).read_text(encoding="utf-8"))
    cur_rows = current if isinstance(current, list) else current.get("benches", [])
    prev_by_name: dict[str, float] = {}
    if prev_data_js and Path(prev_data_js).exists():
        try:
            data = _parse_data_js(Path(prev_data_js).read_text(encoding="utf-8"))
            entries = data.get("entries", {})
            # prefer the named suite; fall back to the only/first suite present.
            series = entries.get(suite) or (next(iter(entries.values())) if entries else [])
            ordered = sorted((e for e in series if isinstance(e, dict)), key=_entry_date)
            for e in ordered:  # later entries overwrite earlier -> most-recent value wins
                benches = e.get("benches", [])
                for b in (benches if isinstance(benches, list) else []):
                    if not isinstance(b, dict):
                        continue
                    fv = _finite_num(b.get("value"))
                    if b.get("name") is not None and fv is not None:
                        prev_by_name[str(b["name"])] = fv
        except (json.JSONDecodeError, ValueError, OSError, TypeError, AttributeError) as e:
            log(f"warning: could not parse previous data.js ({e}) — no comparison baseline")
    out = []
    dropped = 0
    for b in cur_rows:
        if not isinstance(b, dict):
            dropped += 1
            continue
        name = b.get("name")
        val = _finite_num(b.get("value"))
        if name is None or val is None:
            dropped += 1
            continue
        out.append({
            "name": str(name),
            "current": val,
            "previous": prev_by_name.get(str(name)),
            "unit": b.get("unit", ""),
        })
    if dropped:
        log(f"warning: dropped {dropped} malformed/non-finite result row(s) from {results_path}")
    return out


def parse_comparison(path: str) -> list[dict]:
    """Read a comparison JSON: a list of {name, current, previous[, unit]} rows.

    This is the small normalized shape the workflow builds from github-action-benchmark's
    output (the action does not emit a stable machine file, so the workflow constructs this
    from the current run's results + the previous data.js point). Rows with no previous value
    (a brand-new metric) are skipped — nothing to compare against. Numeric fields are
    admitted through _finite_num only: a present-but-non-finite/bool/unconvertible value is
    a MALFORMED row, dropped with a counted warning (never a float() crash).
    """
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    rows = data if isinstance(data, list) else data.get("benches", [])
    out = []
    dropped = 0
    for r in rows:
        if not isinstance(r, dict):
            dropped += 1
            continue
        name = r.get("name")
        cur_raw = r.get("current", r.get("value"))
        prev_raw = r.get("previous")
        if name is None or cur_raw is None or prev_raw is None:
            continue  # brand-new metric / incomplete row — nothing to compare
        cur = _finite_num(cur_raw)
        prev = _finite_num(prev_raw)
        if cur is None or prev is None:
            dropped += 1  # present but bool/NaN/inf/out-of-range — malformed, not "new"
            continue
        ratio = _to_ratio_pct(cur, prev)
        if ratio is None:
            continue  # zero previous / overflowing ratio — comparison undefined
        out.append({
            "name": str(name),
            "current": cur,
            "previous": prev,
            "ratio_pct": round(ratio, 1),
            "unit": r.get("unit", ""),
        })
    if dropped:
        log(f"warning: dropped {dropped} malformed/non-finite comparison row(s) from {path}")
    return out


def zone_partition(rows: list[dict], soft: float, hard: float) -> dict[str, list[dict]]:
    part = {"ok": [], "soft": [], "hard": []}
    for r in rows:
        part[classify(r["ratio_pct"], soft, hard)].append(r)
    return part


def load_hardzone_soft_rows(path: str | None) -> list[dict]:
    """Floor-exempt hard-band rows from bench_hardzone.py's --report-out JSON (fail-soft -> []).

    The hard gate never fails on a floor-exempt metric and its median rebases after a few
    accepted points, so these hits must land in the durable bench-flake issue trail instead.
    Rows arrive in the normalized comparison shape with `previous` holding the
    MEDIAN-of-history (not the single previous point) and a `note` tag rendered in the issue
    table. Malformed rows are dropped with a counted warning (this path must never break the
    triage filer): every numeric field is admitted through _finite_num — bool rejected
    (json true/false must not pass as 1/0), NaN/Infinity rejected, and the float() conversion
    itself guarded (an out-of-float-range int like 10**400 raises OverflowError — a drop,
    never a crash).
    """
    if not path:
        return []
    p = Path(path)
    if not p.exists():
        return []  # hardzone gate did not run on this event (PR/schedule) — nothing to merge
    try:
        data = json.loads(p.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, ValueError, OSError) as e:
        log(f"warning: could not parse hardzone report {path} ({e}) — skipping its rows")
        return []
    if not isinstance(data, dict):
        log(f"warning: hardzone report {path} is not a JSON object — skipping its rows")
        return []
    fe = data.get("floor_exempt_hard", [])
    if not isinstance(fe, list):
        # {"floor_exempt_hard": null} / a number / a string must degrade to zero merged
        # rows with a warning — never TypeError out of ordinary soft filing.
        log(f"warning: hardzone report {path}: floor_exempt_hard is "
            f"{type(fe).__name__}, expected a list — skipping its rows")
        return []
    out = []
    dropped = 0
    for r in fe:
        if not isinstance(r, dict):
            dropped += 1
            continue
        name = r.get("name")
        cur = _finite_num(r.get("current"))
        prev = _finite_num(r.get("previous"))
        ratio = _finite_num(r.get("ratio_pct"))
        if name is None or cur is None or prev is None or ratio is None:
            dropped += 1
            continue
        out.append({"name": str(name), "current": cur, "previous": prev,
                    "ratio_pct": ratio, "unit": str(r.get("unit") or ""),
                    "note": str(r.get("note") or "floor-exempt hard-band")})
    if dropped:
        log(f"warning: dropped {dropped} malformed floor_exempt_hard row(s) from {path}")
    return out


def merge_hardzone_rows(soft_rows: list[dict], extra: list[dict]) -> list[dict]:
    """Merge floor-exempt hard-band rows into the soft partition (tagged row wins by name).

    A metric can appear both in the single-previous-point soft band and in the hardzone
    report; the tagged median-of-history row is the more meaningful record, so it replaces
    the untagged one rather than duplicating the table row.
    """
    have = {r["name"] for r in extra}
    return [r for r in soft_rows if r["name"] not in have] + extra


# ── gh helpers (fail-soft) ─────────────────────────────────────────────────────────
def gh(*argv: str) -> str:
    return subprocess.run(["gh", *argv], check=True, capture_output=True, text=True).stdout.strip()


def ensure_label(name: str, color: str, desc: str) -> None:
    """Create the label if missing (idempotent via --force). Fail-soft."""
    try:
        gh("label", "create", name, "--color", color, "--description", desc, "--force")
    except subprocess.CalledProcessError as e:
        log(f"warning: label create {name} failed (non-fatal): {e.stderr.strip() if e.stderr else e}")


def find_open_issue(label: str, marker: str, key: str) -> str | None:
    """Number of the open triage issue already filed for (marker, key), if any.

    [OPUS-5] #5804: LIST BY LABEL and exact-substring-match the title locally
    (scripts/gh_dedupe.py) rather than trusting `gh --search`. The keys here are
    `suite=<suite>` and `cluster=sparq-engine+sparq-core` — the `+`, `-` and `[` are
    exactly the punctuation gh's search tokeniser handles unreliably, and the search
    index also lags, so the previous lookup could MISS an issue this lane already
    filed and re-file it.
    """
    res = gh_dedupe.find_open_issue(
        label,
        (marker, key),
        legacy_search=f'in:title "{marker} {key}"',
        log=log,
        backfill_label=True,
    )
    if res.issue is None and not res.probed:
        log("warning: issue dedupe lookup failed")
    return res.number


def _post_issue(title: str, body: str, labels: list[str], existing: str | None) -> str | None:
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as tf:
        tf.write(body)
        body_file = tf.name
    try:
        if existing:
            gh("issue", "comment", existing, "--body-file", body_file)
            log(f"commented on existing issue #{existing} (deduped)")
            return existing
        argv = ["issue", "create", "--title", title, "--body-file", body_file]
        for lab in labels:
            argv += ["--label", lab]
        url = gh(*argv)
        log(f"filed issue: {url}")
        return url
    except subprocess.CalledProcessError as e:
        log(f"warning: gh issue filing failed (non-fatal): {e.stderr.strip() if e.stderr else e}")
        return None
    finally:
        Path(body_file).unlink(missing_ok=True)


# ── defense-in-depth sanitizer for artifact-derived text in issue bodies ────────────
def _sanitize(text, *, code: bool = False) -> str:
    """[FABLE-5 round 4] THE sanitizer — every artifact-derived string interpolated into an
    issue/comment body routes through here (via _md_cell / _md_code_cell / directly).

    Layer 1 — HTML, first and unconditional: GitHub renders raw HTML in issue bodies, so a
    value like `</td></tr></table><h1>forged</h1>` would BREAK OUT of the surrounding table
    even with perfect Markdown escaping. Escape &, <, > (& first — that also defuses
    entity-encoded smuggling such as `&#64;user` / `&lt;`). Applied inside code spans too,
    belt-and-braces: a span that any renderer quirk fails to close must still contain no
    live tags.

    Layer 2 — Markdown, per cell type:
      * plain (default): strip newlines (no forged table rows), backslash-escape the
        escapables (\\ | ` — no forged cells / opened code spans), then split @-mentions
        with an empty HTML comment. The comment is inserted AFTER the HTML escape so it
        survives as a real comment; layer 1 already reduced every attacker-supplied < to
        &lt;, so the only raw HTML left in the output is this trusted defusal comment.
      * code span (code=True): backslash escapes do NOT work inside a code span, so
        span-terminating backticks become `'`; `\\|` is the one escape GFM honours inside
        code spans in table cells; the span itself neutralizes @-mentions.
    """
    s = re.sub(r"[\r\n]+", " ", str(text))
    s = s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    if code:
        s = s.replace("`", "'").replace("|", "\\|")
        return f"`{s}`"
    s = s.replace("\\", "\\\\").replace("|", "\\|").replace("`", "\\`")
    return s.replace("@", "@<!-- -->")


def _md_cell(text) -> str:
    """Sanitize an artifact-derived value for a plain Markdown table cell / prose slot."""
    return _sanitize(text)


def _md_code_cell(text) -> str:
    """Sanitize an artifact-derived value into a Markdown code span (table-cell safe)."""
    return _sanitize(text, code=True)


def _md_url(text) -> str:
    """Sanitize an interpolated link DESTINATION (the `(...)` part of a Markdown link).

    Strip all whitespace, then percent-encode every character that could terminate the
    destination, escape out of it, or open raw HTML. Display text next to the destination
    goes through _sanitize separately.
    """
    s = re.sub(r"\s+", "", str(text))
    for ch, enc in (("\\", "%5C"), ("(", "%28"), (")", "%29"), ("<", "%3C"),
                    (">", "%3E"), ('"', "%22"), ("'", "%27"), ("`", "%60")):
        s = s.replace(ch, enc)
    return s


# ── SOFT zone: one rolling deduped bench-flake issue per suite ──────────────────────
def build_flake_body(suite: str, rows: list[dict], soft: float, hard: float, run_url: str) -> str:
    tagged_rows = [r for r in rows if r.get("note")]
    plain_rows = [r for r in rows if not r.get("note")]
    lines = [
        "> 🤖 **SPARQ agent** — auto-filed by the benchmark self-monitoring soft zone "
        "(noise-reduction-bench-selfmonitor). No human is @-mentioned by design.",
        "",
    ]
    # [FABLE-5 round 4] the class sentence is derived from the row classes ACTUALLY present —
    # soft-band only, floor-exempt only, or mixed — so the preamble never describes a class
    # the table below does not contain.
    if tagged_rows and plain_rows:
        # MIXED: merged floor-exempt hard-band rows sit AT/ABOVE the hard ratio (vs their
        # median-of-history) — a "everything below the hard threshold" preamble would lie.
        lines.append(
            f"Benchmarks in suite {_md_code_cell(suite)} were flagged by the benchmark "
            f"self-monitor. This run's rows fall in **two classes**: **soft-band** rows — "
            f"above the soft alert-threshold (`{soft}%`) but below the hard fail-threshold "
            f"(`{hard}%`) vs their single previous point — and tagged **floor-exempt "
            f"hard-band** rows — at/above the hard ratio vs their median-of-history but "
            f"under their unit's noise floor, so watch-only for the gate. Either way this "
            f"**could be shared-runner flakiness** rather than a real regression, and it "
            f"does **not** fail CI. This is a rolling, deduped record — new hits append below."
        )
    elif tagged_rows:
        # FLOOR-EXEMPT ONLY: no soft-band row this run — claiming "two classes" (or any
        # "below the hard fail-threshold" soft framing) would describe rows that are not
        # in the table.
        lines.append(
            f"Benchmarks in suite {_md_code_cell(suite)} were flagged by the benchmark "
            f"self-monitor. **Every row this run is a tagged floor-exempt hard-band hit** — "
            f"at/above the hard ratio vs its median-of-history but under its unit's noise "
            f"floor, so watch-only for the gate; there are no soft-band rows this run. At "
            f"that scale the measured honest run-to-run bands exceed the hard threshold, so "
            f"this **could be shared-runner flakiness** rather than a real regression, and "
            f"it does **not** fail CI. This is a rolling, deduped record — new hits append "
            f"below."
        )
    else:
        # SOFT ONLY: the classic soft-zone framing stays exact.
        lines.append(
            f"Benchmarks in suite {_md_code_cell(suite)} regressed into the **soft zone** — "
            f"above the soft alert-threshold (`{soft}%`) but below the hard fail-threshold "
            f"(`{hard}%`), so this "
            "**could be shared-runner flakiness** rather than a real regression. It does **not** "
            "fail CI. This is a rolling, deduped record — new soft-zone hits append below."
        )
    lines += [
        "",
        f"- **Run:** {_md_cell(run_url)}",
        "",
        "### Flagged benchmarks (this run)",
        "",
        "| benchmark | previous | current | ratio |",
        "|---|---|---|---|",
    ]
    for r in sorted(rows, key=lambda x: -x["ratio_pct"]):
        tag = f" — {_md_cell(r['note'])}" if r.get("note") else ""
        unit = _md_cell(r["unit"])
        lines.append(
            f"| {_md_code_cell(r['name'])}{tag} | {r['previous']:g}{unit} | "
            f"{r['current']:g}{unit} | {r['ratio_pct']:g}% |"
        )
    if tagged_rows:
        lines += [
            "",
            "Rows tagged \"floor-exempt hard-band\" come from the median-of-history hard gate "
            "(`scripts/bench_hardzone.py`): they are at/above the HARD ratio against their "
            "median-of-history but sit under their unit's noise floor, so they are watch-only "
            "for the gate. They are recorded here so a persistent sub-floor regression keeps a "
            "durable trail even after the median rebases — for those rows the `previous` "
            "column is the median-of-history, not the single previous point.",
        ]
    lines += [
        "",
        "If these persist across several runs the swing is likely real — promote to a "
        f"`{REGRESSION_LABEL}` investigation. If they come and go, they are runner noise; "
        "re-derive the thresholds with `scripts/bench-thresholds.py` if the noise floor has "
        "shifted.",
    ]
    return "\n".join(lines)


def file_flake_issues(part: dict, soft: float, hard: float, run_url: str) -> list[str]:
    soft_rows = part["soft"]
    if not soft_rows:
        log("no soft-zone regressions — nothing to file.")
        return []
    ensure_label(FLAKE_LABEL, "fbca04", "benchmark soft-zone (possible runner flakiness)")
    by_suite: dict[str, list[dict]] = {}
    for r in soft_rows:
        by_suite.setdefault(suite_of(r["name"]), []).append(r)
    filed = []
    for suite, rows in sorted(by_suite.items()):
        key = f"suite={suite}"
        title = f"{FLAKE_MARKER} {key}: benchmarks in the soft zone (possible runner noise)"
        existing = find_open_issue(FLAKE_LABEL, FLAKE_MARKER, key)
        body = build_flake_body(suite, rows, soft, hard, run_url)
        res = _post_issue(title, body, [FLAKE_LABEL], existing)
        if res:
            filed.append(res)
    return filed


# ── NIGHTLY attribution: rank suspect commits, file one bench-regression issue per cluster ──
def commits_between(baseline_sha: str, head_sha: str) -> list[dict]:
    """List commits (with touched paths) landed on main in (baseline, head]. Fail-soft -> []."""
    try:
        raw = subprocess.run(
            ["git", "log", "--no-merges", "--name-only", "--pretty=format:%x00%H%x00%s%x00%an",
             f"{baseline_sha}..{head_sha}"],
            check=True, capture_output=True, text=True,
        ).stdout
    except subprocess.CalledProcessError as e:
        log(f"warning: git log failed ({e}) — no attribution")
        return []
    commits = []
    for block in raw.split("\x00\x00"):
        block = block.strip("\x00\n")
        if not block:
            continue
        parts = block.split("\x00")
        if len(parts) < 3:
            continue
        sha, subject, rest = parts[0], parts[1], parts[2]
        rest_lines = rest.split("\n")
        author = rest_lines[0]
        paths = [p for p in rest_lines[1:] if p.strip()]
        commits.append({"sha": sha, "subject": subject, "author": author, "paths": paths})
    return commits


def _crate_of_path(path: str) -> str | None:
    m = re.match(r"crates/([^/]+)/", path)
    return m.group(1) if m else None


def rank_suspects(regressed_crates: set[str], commits: list[dict]) -> list[dict]:
    """Rank commits by how much their touched paths intersect the regressed suite's crates."""
    ranked = []
    for c in commits:
        touched_crates = {cr for cr in (_crate_of_path(p) for p in c["paths"]) if cr}
        overlap = touched_crates & regressed_crates
        # score: direct crate-overlap dominates; a bench-harness / scripts touch is a weak signal.
        score = len(overlap) * 10
        if any(p.startswith("bench/") for p in c["paths"]):
            score += 2
        if any(p.startswith("scripts/") for p in c["paths"]):
            score += 1
        if score > 0:
            ranked.append({
                "sha": c["sha"],
                "subject": c["subject"],
                "author": c["author"],
                "overlap": sorted(overlap),
                "touched_crates": sorted(touched_crates),
                "score": score,
            })
    ranked.sort(key=lambda x: (-x["score"], x["sha"]))
    return ranked


def _pr_ref(subject: str) -> str:
    """Extract a trailing (#NNNN) PR ref from a squash-merge subject, else empty."""
    m = re.search(r"\(#(\d+)\)\s*$", subject)
    return f"#{m.group(1)}" if m else ""


def build_regression_body(cluster_crates: list[str], rows: list[dict], ranked: list[dict],
                          hard: float, run_url: str, repo: str, baseline_sha: str,
                          head_sha: str) -> str:
    lines = [
        "> 🤖 **SPARQ agent** — auto-filed by the nightly benchmark attribution "
        "(noise-reduction-bench-selfmonitor). No human is @-mentioned by design; this issue "
        "enters the autonomous dispatch frontier for an agent to fix.",
        "",
        f"The nightly full-suite benchmark detected a regression **beyond the hard "
        f"fail-threshold (`{hard}%`)** vs the last green nightly baseline, in the "
        f"{_md_code_cell('+'.join(cluster_crates))} suite cluster.",
        "",
        f"- **Run:** {_md_cell(run_url)}",
        f"- **Baseline (last green nightly):** {_md_code_cell(baseline_sha[:12])}",
        f"- **Head:** {_md_code_cell(head_sha[:12])}",
        "",
        "### Regression table",
        "",
        "| benchmark | previous | current | ratio |",
        "|---|---|---|---|",
    ]
    for r in sorted(rows, key=lambda x: -x["ratio_pct"]):
        unit = _md_cell(r["unit"])
        lines.append(
            f"| {_md_code_cell(r['name'])} | {r['previous']:g}{unit} | "
            f"{r['current']:g}{unit} | {r['ratio_pct']:g}% |"
        )
    lines += ["", "### Ranked suspect commits", ""]
    if ranked:
        lines += ["| rank | commit | PR | overlap crates | subject |", "|---|---|---|---|---|"]
        for i, c in enumerate(ranked[:10], 1):
            pr = _pr_ref(c["subject"])  # regex-constrained to "#<digits>" — safe by shape
            pr_link = f"[{pr}]({_md_url(repo)}/pull/{pr[1:]})" if pr else "—"
            commit_link = (f"[{_md_code_cell(c['sha'][:10])}]"
                           f"({_md_url(repo)}/commit/{_md_url(c['sha'])})")
            lines.append(
                f"| {i} | {commit_link} | {pr_link} | "
                f"{_md_cell(', '.join(c['overlap'])) or '—'} | {_md_cell(c['subject'])} |"
            )
    else:
        lines.append("_No commit in the baseline..head range touched the regressed crates; "
                     "the regression may be environmental (investigate the runner) or in a "
                     "shared dependency._")
    lines += [
        "",
        "### Reproduce locally",
        "",
        "```bash",
        "# full nightly timing suite at the CI scale (quiet box recommended):",
        "BENCH_FULL_SUITE=1 BENCH_SCALE=200000 bash scripts/ec2-bench.sh   # or, locally:",
        "bash scripts/ci-bench.sh   # (writes bench-results.json; compare the flagged metrics)",
        "# bisect the ranked suspects above against the regressed benchmark.",
        "```",
    ]
    return "\n".join(lines)


def file_regression_issues(part: dict, soft: float, hard: float, baseline_sha: str,
                           head_sha: str, run_url: str, repo: str) -> list[str]:
    hard_rows = part["hard"]
    if not hard_rows:
        log("no hard-zone regressions — no nightly attribution issue.")
        return []
    ensure_label(REGRESSION_LABEL, "b60205", "benchmark hard-zone regression (needs a fix)")
    commits = commits_between(baseline_sha, head_sha) if baseline_sha and head_sha else []
    # Cluster hard rows by their mapped crate set (one issue per regression cluster).
    by_cluster: dict[tuple, list[dict]] = {}
    for r in hard_rows:
        by_cluster.setdefault(tuple(metric_crates(r["name"])), []).append(r)
    filed = []
    for cluster_crates_t, rows in sorted(by_cluster.items()):
        cluster_crates = list(cluster_crates_t)
        regressed = set(cluster_crates)
        ranked = rank_suspects(regressed, commits)
        area = ranked[0]["overlap"][0] if (ranked and ranked[0]["overlap"]) else cluster_crates[0]
        key = f"cluster={'+'.join(cluster_crates)}"
        title = f"{REGRESSION_MARKER} {key}: nightly benchmark regression (P1)"
        existing = find_open_issue(REGRESSION_LABEL, REGRESSION_MARKER, key)
        body = build_regression_body(cluster_crates, rows, ranked, hard, run_url, repo,
                                     baseline_sha, head_sha)
        labels = [REGRESSION_LABEL, "priority:P1", "role:impl", f"area:{area}"]
        res = _post_issue(title, body, labels, existing)
        if res:
            filed.append(res)
    return filed


# ── self-test (hermetic: no gh, no git, no writes) ────────────────────────────────
def self_test() -> int:
    # zone classification.
    assert classify(210, 175, 200) == "hard"
    assert classify(200, 175, 200) == "hard"      # >= hard is hard
    assert classify(180, 175, 200) == "soft"
    assert classify(175, 175, 200) == "soft"      # >= soft is soft
    assert classify(120, 175, 200) == "ok"
    assert classify(100, 175, 200) == "ok"

    # ratio helper.
    assert _to_ratio_pct(15, 10) == 150.0
    assert _to_ratio_pct(10, 0) is None
    assert _to_ratio_pct(1e308, 1e-308) is None   # finite/finite can still overflow to inf

    # [FABLE-5 round 4] _finite_num — THE numeric admission gate for every consumed field:
    # bool rejected, NaN/inf rejected, the float() conversion guarded (10**400 raises
    # OverflowError — a drop, never a crash), strings/None rejected, real numbers pass.
    assert _finite_num(5) == 5.0 and _finite_num(2.5) == 2.5 and _finite_num(0) == 0.0
    assert _finite_num(True) is None and _finite_num(False) is None
    assert _finite_num(float("nan")) is None and _finite_num(float("inf")) is None
    assert _finite_num(float("-inf")) is None
    assert _finite_num(10**400) is None           # OverflowError path — dropped, not raised
    assert _finite_num("5") is None and _finite_num(None) is None and _finite_num([1]) is None
    # junk dates sort as 0 instead of TypeError-ing the whole comparison build.
    assert _entry_date({"date": "garbage"}) == 0.0 and _entry_date({"date": 7}) == 7.0

    # suite->crate map: longest-prefix + fallback.
    assert metric_crates("watdiv_sf1_q1_us") == ["sparq-engine", "sparq-core"]
    assert metric_crates("geo_within_us") == ["sparq-geo"]
    assert metric_crates("rsp_window_us") == ["sparq-rsp"]
    assert metric_crates("hdt_load_us") == ["sparq-hdt"]
    assert metric_crates("zk_prove_ms") == ["sparq-zk"]
    assert metric_crates("q02_type_person_count_us") == ["sparq-engine", "sparq-core"]
    assert metric_crates("totally_unknown_metric") == _FALLBACK_CRATES  # fallback

    # suite bucketing groups engine query suites under one key (bounded issue count).
    assert suite_of("watdiv_q1_us") == suite_of("sp2b_q3_us") == "sparq-engine+sparq-core"
    assert suite_of("geo_x_us") == "sparq-geo"

    # parse_comparison + zone_partition over a fixture.
    with tempfile.TemporaryDirectory() as td:
        cmp = Path(td) / "cmp.json"
        cmp.write_text(json.dumps([
            {"name": "watdiv_q1_us", "current": 30.0, "previous": 10.0, "unit": "us"},  # 300% hard
            {"name": "geo_within_us", "current": 18.0, "previous": 10.0, "unit": "us"},  # 180% soft
            {"name": "q02_type_person_count_us", "current": 11.0, "previous": 10.0},      # 110% ok
            {"name": "new_metric_us", "current": 5.0},                                    # no prev -> skip
            {"name": "zero_prev_us", "current": 5.0, "previous": 0.0},                    # prev 0 -> skip
            # [FABLE-5 round 4] malformed numerics: dropped with a warning, NEVER a crash —
            # bool current, string current (the old float() call ValueError'd on this),
            # out-of-float-range int previous (OverflowError), NaN current.
            {"name": "bool_cur_us", "current": True, "previous": 10.0},
            {"name": "str_cur_us", "current": "fast", "previous": 10.0},
            {"name": "huge_prev_us", "current": 5.0, "previous": 10**400},
            {"name": "nan_cur_us", "current": float("nan"), "previous": 10.0},
        ]), encoding="utf-8")
        rows = parse_comparison(str(cmp))
        assert len(rows) == 3, [r["name"] for r in rows]   # 2 skipped + 4 malformed dropped
        part = zone_partition(rows, 175, 200)
        assert [r["name"] for r in part["hard"]] == ["watdiv_q1_us"]
        assert [r["name"] for r in part["soft"]] == ["geo_within_us"]
        assert [r["name"] for r in part["ok"]] == ["q02_type_person_count_us"]

        # flake body is @mention-free + lists the flagged bench.
        fbody = build_flake_body("sparq-geo", part["soft"], 175, 200, "http://run/1")
        assert "@" not in fbody.replace("@-mentioned", "").replace("@-mention", "")
        assert "geo_within_us" in fbody and "🤖" in fbody and "soft zone" in fbody

    # build_comparison: pair current results with the previous data.js point.
    with tempfile.TemporaryDirectory() as tdc:
        results = Path(tdc) / "bench-results.json"
        results.write_text(json.dumps([
            {"name": "watdiv_q1_us", "value": 30.0, "unit": "us"},
            {"name": "new_only_us", "value": 5.0, "unit": "us"},   # no prev -> previous null
            {"name": "bool_val_us", "value": True, "unit": "us"},  # bool value -> dropped
            {"name": "huge_val_us", "value": 10**400},             # OverflowError -> dropped
        ]), encoding="utf-8")
        prev = Path(tdc) / "prev-data.js"
        prev.write_text(
            'window.BENCHMARK_DATA = ' + json.dumps({"entries": {"sparq engine": [
                # a garbage date sorts as 0 (oldest) instead of TypeError-ing the sort.
                {"date": "garbage", "commit": {"id": "c0"}, "benches": [
                    {"name": "watdiv_q1_us", "value": 7.0, "unit": "us"}]},
                {"date": 1, "commit": {"id": "c1"}, "benches": [
                    {"name": "watdiv_q1_us", "value": 8.0, "unit": "us"}]},
                {"date": 2, "commit": {"id": "c2"}, "benches": [
                    {"name": "watdiv_q1_us", "value": 10.0, "unit": "us"},  # most-recent wins
                    {"name": "bool_prev_us", "value": True},                # never a baseline
                    {"name": "huge_prev_us", "value": 10**400}]},           # never a baseline
            ]}}) + ';\n', encoding="utf-8")
        cmp_rows = build_comparison(str(results), str(prev), "sparq engine")
        by = {r["name"]: r for r in cmp_rows}
        assert by["watdiv_q1_us"]["previous"] == 10.0, by["watdiv_q1_us"]  # newest prev value
        assert by["watdiv_q1_us"]["current"] == 30.0
        assert by["new_only_us"]["previous"] is None
        # [FABLE-5 round 4] non-finite/bool/out-of-range values never survive admission.
        assert "bool_val_us" not in by and "huge_val_us" not in by, sorted(by)
        # feeding it through parse_comparison drops the no-previous row + computes ratio.
        cmp_file = Path(tdc) / "cmp.json"
        cmp_file.write_text(json.dumps(cmp_rows), encoding="utf-8")
        parsed = parse_comparison(str(cmp_file))
        assert [r["name"] for r in parsed] == ["watdiv_q1_us"]
        assert parsed[0]["ratio_pct"] == 300.0
        # missing prev file => all previous null (first-ever run).
        cmp_noprev = build_comparison(str(results), str(Path(tdc) / "absent.js"), "sparq engine")
        assert all(r["previous"] is None for r in cmp_noprev)
        # baseline_sha returns the newest published commit id (empty when absent / unparseable).
        assert baseline_sha(str(prev), "sparq engine") == "c2"
        assert baseline_sha(str(prev), "nonexistent suite") == "c2"  # falls back to only suite
        assert baseline_sha(str(Path(tdc) / "absent.js"), "sparq engine") == ""

    # [FABLE-5] hardzone floor-exempt hard-band rows: loaded fail-soft, merged into the soft
    # partition (tagged row wins on a name collision), rendered tagged in the flake body.
    with tempfile.TemporaryDirectory() as td3:
        hz = Path(td3) / "hardzone-report.json"
        hz.write_text(json.dumps({"floor_exempt_hard": [
            {"name": "tiny_query_us", "current": 12.0, "previous": 4.0, "ratio_pct": 300.0,
             "unit": "us",
             "note": "floor-exempt hard-band (>= 2x median-of-history; watch-only for the gate)"},
            {"name": "malformed_row", "current": "not-a-number"},   # dropped, never crashes
            # [FABLE-5 round 4] numeric admission on EVERY consumed field: bool current
            # (isinstance(int,float) passes it — must be rejected), NaN current and inf
            # previous (json.loads emits both by default), and an out-of-float-range int
            # ratio_pct whose float() raises OverflowError — all dropped with a counted
            # warning, never a crash.
            {"name": "bool_row", "current": True, "previous": 4.0, "ratio_pct": 300.0},
            {"name": "nan_row", "current": float("nan"), "previous": 4.0, "ratio_pct": 300.0},
            {"name": "inf_row", "current": 12.0, "previous": float("inf"), "ratio_pct": 300.0},
            {"name": "huge_row", "current": 12.0, "previous": 4.0, "ratio_pct": 10**400},
        ]}), encoding="utf-8")
        extra = load_hardzone_soft_rows(str(hz))
        assert [r["name"] for r in extra] == ["tiny_query_us"], extra
        soft_rows = [
            {"name": "tiny_query_us", "current": 8.0, "previous": 4.0, "ratio_pct": 200.0,
             "unit": "us"},                                          # same-name untagged row
            {"name": "geo_within_us", "current": 18.0, "previous": 10.0, "ratio_pct": 180.0,
             "unit": "us"},
        ]
        merged = merge_hardzone_rows(soft_rows, extra)
        assert [r["name"] for r in merged] == ["geo_within_us", "tiny_query_us"], merged
        assert merged[-1]["note"].startswith("floor-exempt hard-band")
        mbody = build_flake_body("sparq-engine+sparq-core", merged, 175, 200, "http://run/3")
        assert "floor-exempt hard-band" in mbody and "tiny_query_us" in mbody
        assert "median-of-history" in mbody   # the tagged-row explainer paragraph
        stripped3 = mbody.replace("@-mentioned", "").replace("@-mention", "")
        assert "@" not in stripped3, "flake body with hardzone rows must not @-mention anyone"
        # [FABLE-5 round 3] the preamble distinguishes the two row classes — a merged
        # floor-exempt row sits AT/ABOVE the hard ratio, so the soft-only "below the hard
        # fail-threshold" framing would contradict the table it introduces.
        assert "two classes" in mbody, "tagged body must distinguish soft vs floor-exempt rows"
        assert "regressed into the **soft zone**" not in mbody
        # untagged-only bodies carry no explainer paragraph (soft-band prose stays accurate).
        plain = build_flake_body("sparq-geo", soft_rows[1:], 175, 200, "http://run/4")
        assert "floor-exempt hard-band" not in plain
        assert "regressed into the **soft zone**" in plain and "two classes" not in plain
        # [FABLE-5 round 4] three-way class sentence — the FLOOR-EXEMPT-ONLY case: every row
        # tagged, no soft-band row, so the body must claim NEITHER "two classes" NOR the
        # soft-zone framing; the all-floor-exempt sentence + explainer paragraph instead.
        fe_only = build_flake_body("sparq-engine+sparq-core", extra, 175, 200, "http://run/7")
        assert "Every row this run is a tagged floor-exempt hard-band hit" in fe_only
        assert "two classes" not in fe_only
        assert "regressed into the **soft zone**" not in fe_only
        assert "median-of-history" in fe_only          # explainer paragraph still present
        assert "tiny_query_us" in fe_only
        stripped_fe = fe_only.replace("@-mentioned", "").replace("@-mention", "")
        assert "@" not in stripped_fe, "floor-exempt-only body must not @-mention anyone"
        # fail-soft: no arg / absent file / unparsable file all -> [] (triage must not break).
        assert load_hardzone_soft_rows(None) == []
        assert load_hardzone_soft_rows(str(Path(td3) / "absent.json")) == []
        bad = Path(td3) / "bad.json"
        bad.write_text("{not json", encoding="utf-8")
        assert load_hardzone_soft_rows(str(bad)) == []
        # [FABLE-5 round 3] fail-soft on WRONG-TYPED payloads: {"floor_exempt_hard": null},
        # a number, and a non-object top level must all degrade to zero merged rows with a
        # warning — never TypeError out of ordinary soft filing.
        nul = Path(td3) / "nul.json"
        nul.write_text(json.dumps({"floor_exempt_hard": None}), encoding="utf-8")
        assert load_hardzone_soft_rows(str(nul)) == []
        num = Path(td3) / "num.json"
        num.write_text(json.dumps({"floor_exempt_hard": 7}), encoding="utf-8")
        assert load_hardzone_soft_rows(str(num)) == []
        arr = Path(td3) / "arr.json"
        arr.write_text(json.dumps([1, 2]), encoding="utf-8")
        assert load_hardzone_soft_rows(str(arr)) == []

        # [FABLE-5 round 3] Markdown-injection hardening: pipes/backticks/newlines/@ in
        # artifact-derived fields render escaped — no forged table rows, no code-span
        # breakout, no naked @-mention.
        evil = [{"name": "evil_us|`name\ninjected", "current": 12.0, "previous": 4.0,
                 "ratio_pct": 300.0, "unit": "us|`u",
                 "note": "pinged @someone | `boom`\nrow"}]
        ebody = build_flake_body("sparq-engine+sparq-core", evil, 175, 200, "http://run/5")
        evil_lines = [ln for ln in ebody.splitlines() if "evil_us" in ln]
        assert len(evil_lines) == 1, evil_lines            # a newline never splits the row
        erow = evil_lines[0]
        # code-span cell: | escaped GFM-style, span-breaking ` defused, newline stripped.
        assert "`evil_us\\|'name injected`" in erow, erow
        assert "us\\|\\`u" in erow, erow                   # plain cell: | and ` escaped
        assert "\\`boom\\`" in erow, erow                  # note ` cannot open a code span
        assert "@someone" not in ebody                     # mention split by an HTML comment
        assert "@<!-- -->someone" in ebody

        # [FABLE-5 round 4] defense-in-depth HTML layer: GitHub renders raw HTML in issue
        # bodies, so a raw-HTML table breakout in ANY interpolated field must render with
        # < > & escaped — in plain cells AND inside code spans (belt-and-braces) alike.
        breakout = "</td></tr></table><h1>forged</h1>"
        forged = [{"name": f"sneaky_us{breakout}", "current": 12.0, "previous": 4.0,
                   "ratio_pct": 300.0, "unit": f"us{breakout}", "note": f"n{breakout}"}]
        hbody = build_flake_body("sparq-engine+sparq-core", forged, 175, 200, "http://run/6")
        assert "<h1>" not in hbody and "</td>" not in hbody and "</table>" not in hbody, hbody
        assert "&lt;/td&gt;&lt;/tr&gt;&lt;/table&gt;&lt;h1&gt;forged&lt;/h1&gt;" in hbody, hbody
        # entity-encoded @-mention smuggling is defused by escaping & first.
        amp_note = [{"name": "amp_us", "current": 12.0, "previous": 4.0, "ratio_pct": 300.0,
                     "unit": "us", "note": "ping &#64;someone"}]
        abody = build_flake_body("sparq-engine+sparq-core", amp_note, 175, 200, "http://run/8")
        assert "&#64;" not in abody and "&amp;#64;" in abody, abody

    # nightly attribution ranking: overlap with regressed crates ranks first.
    commits = [
        {"sha": "a" * 40, "subject": "feat(geo): add index (#101)", "author": "x",
         "paths": ["crates/sparq-geo/src/index.rs"]},
        {"sha": "b" * 40, "subject": "docs: readme (#102)", "author": "y",
         "paths": ["README.md"]},
        {"sha": "c" * 40, "subject": "perf(engine): join (#103)", "author": "z",
         "paths": ["crates/sparq-engine/src/join.rs", "bench/watdiv/run.sh"]},
    ]
    ranked = rank_suspects({"sparq-geo"}, commits)
    assert ranked and ranked[0]["sha"] == "a" * 40, ranked
    assert all(c["sha"] != "b" * 40 for c in ranked)  # no-overlap docs commit excluded
    # engine regression ranks the engine+bench commit.
    ranked2 = rank_suspects({"sparq-engine"}, commits)
    assert ranked2[0]["sha"] == "c" * 40 and ranked2[0]["score"] >= 12  # overlap*10 + bench 2

    # PR-ref extraction.
    assert _pr_ref("feat(geo): add (#101)") == "#101"
    assert _pr_ref("no pr here") == ""

    # regression body: table + ranked suspects + repro, no @mention.
    with tempfile.TemporaryDirectory() as td2:
        cmp2 = Path(td2) / "c.json"
        cmp2.write_text(json.dumps([
            {"name": "geo_within_us", "current": 30.0, "previous": 10.0, "unit": "us"},
        ]), encoding="utf-8")
        rows2 = parse_comparison(str(cmp2))
        part2 = zone_partition(rows2, 175, 200)
        rbody = build_regression_body(["sparq-geo"], part2["hard"], ranked, 200,
                                      "http://run/9", "https://github.com/o/r",
                                      "d" * 40, "e" * 40)
        assert "geo_within_us" in rbody and "Ranked suspect commits" in rbody
        assert "Reproduce locally" in rbody and "#101" in rbody
        assert "🤖" in rbody
        # no naked @mention (the only '@' allowed is inside '@-mention' prose).
        stripped = rbody.replace("@-mentioned", "").replace("@-mention", "")
        assert "@" not in stripped, "regression body must not @-mention anyone"
        # [FABLE-5 round 4] a forged-HTML commit SUBJECT renders escaped in the suspect
        # table too (every interpolated field routes through the one sanitizer).
        evil_commits = [{"sha": "f" * 40, "subject": f"pwn {breakout} (#104)", "author": "m",
                         "paths": ["crates/sparq-geo/src/lib.rs"]}]
        ranked_evil = rank_suspects({"sparq-geo"}, evil_commits)
        rbody2 = build_regression_body(["sparq-geo"], part2["hard"], ranked_evil, 200,
                                       "http://run/10", "https://github.com/o/r",
                                       "d" * 40, "e" * 40)
        assert "<h1>" not in rbody2 and "</table>" not in rbody2, rbody2
        assert "&lt;h1&gt;forged&lt;/h1&gt;" in rbody2 and "#104" in rbody2, rbody2

    log("self-test OK")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(prog=PROG)
    ap.add_argument("--mode",
                    choices=["soft", "nightly", "build-comparison", "baseline-sha"],
                    default="soft")
    ap.add_argument("--comparison", help="normalized comparison JSON (list of name/current/previous)")
    ap.add_argument("--results", help="this run's bench results json (for build-comparison)")
    ap.add_argument("--prev-data-js", help="previously-published data.js (for build-comparison)")
    ap.add_argument("--suite", default="sparq engine", help="suite/entries key (for build-comparison)")
    ap.add_argument("--out", help="where build-comparison writes the normalized JSON")
    ap.add_argument("--soft", type=float, default=175.0, help="soft alert-threshold %% (150==1.5x)")
    ap.add_argument("--hard", type=float, default=200.0, help="hard fail-threshold %% (150==1.5x)")
    ap.add_argument("--hardzone-report",
                    help="bench_hardzone.py --report-out JSON; its floor_exempt_hard rows are "
                         "merged into the soft partition tagged 'floor-exempt hard-band' "
                         "(absent file = no-op; soft mode only)")
    ap.add_argument("--run-url", default="")
    ap.add_argument("--baseline-sha", default="")
    ap.add_argument("--head-sha", default="")
    ap.add_argument("--repo-url", default="")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    if args.mode == "baseline-sha":
        # Prints the last-green-baseline commit id to stdout (empty if none). Fail-soft.
        print(baseline_sha(args.prev_data_js or "", args.suite))
        return 0

    if args.mode == "build-comparison":
        if not args.results:
            ap.error("--results is required for --mode build-comparison")
        rows = build_comparison(args.results, args.prev_data_js, args.suite)
        payload = json.dumps(rows, indent=2)
        if args.out:
            Path(args.out).write_text(payload + "\n", encoding="utf-8")
            log(f"wrote {len(rows)} comparison rows to {args.out}")
        else:
            print(payload)
        return 0

    if not args.comparison:
        ap.error("--comparison is required (or use --self-test)")

    rows = parse_comparison(args.comparison)
    part = zone_partition(rows, args.soft, args.hard)
    log(f"zones: ok={len(part['ok'])} soft={len(part['soft'])} hard={len(part['hard'])}")

    repo = args.repo_url or "https://github.com/sparq-org/sparq"
    if args.mode == "soft":
        # [FABLE-5] durable trail: floor-exempt hard-band hits from the median-of-history gate
        # never red that gate and would vanish once the median rebases — merge them into the
        # soft partition so the deduped bench-flake issue records every one.
        extra = load_hardzone_soft_rows(args.hardzone_report)
        if extra:
            part["soft"] = merge_hardzone_rows(part["soft"], extra)
            log(f"merged {len(extra)} floor-exempt hard-band row(s) from the hardzone report "
                f"into the soft partition (durable bench-flake trail)")
        file_flake_issues(part, args.soft, args.hard, args.run_url)
    else:  # nightly
        file_regression_issues(part, args.soft, args.hard, args.baseline_sha,
                               args.head_sha, args.run_url, repo)
    return 0


if __name__ == "__main__":
    sys.exit(main())
