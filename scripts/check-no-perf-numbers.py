#!/usr/bin/env python3
# [OPUS-4.8] No-hard-coded-performance-numbers check for markdown (bead sq-5fd1).
#
# AGENTS.md house rule: benchmark figures MUST NOT be baked into markdown — they
# drift, and the single source of truth is the generated perf dashboard + the bench
# registry (`bench/benchmarks.toml` / `bench/CATALOG.md`). This script greps tracked
# markdown for measured-throughput / latency / ratio / size-per-triple patterns and
# reports them so reviewers can relocate the figure to bench/research and link it.
#
# STATUS: ADVISORY (run with --advisory in CI — never fails the build) because the
# violation backlog is still widespread (tracked beads: sq-h1s7 / sq-q2em / sq-rt1t /
# sq-puyy / sq-kuqa / sq-p3fk + the research/bench corpus). As crates are de-perf'd
# their READMEs can be moved into --enforce scope. With --enforce it exits non-zero
# on any finding outside the allowlist, so a clean directory can be ratcheted to HARD.
#
# It is deliberately conservative: it allowlists legit, NON-perf numeric facts
# (algorithm constants, platform/spec limits, dataset/fixture sizes, layout
# constants, dates/versions/hashes) so it flags *measured performance*, not "any
# number". False positives are tuned out via ALLOW_LINE_SUBSTRINGS / ALLOW_PATHS.

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys

# --- perf-figure patterns (case-insensitive). A NUMBER immediately followed by a
#     throughput / latency / ratio / size-per-record unit is treated as a measured
#     performance figure. Spec/algorithm constants are excluded by ALLOW_* below. ---
NUM = r"[0-9][0-9_.,]*"
PERF_PATTERNS = [
    # throughput
    (re.compile(rf"{NUM}\s*(?:[GMK]?B/s|[GMK]B/sec)\b", re.I), "throughput (bytes/s)"),
    (re.compile(rf"{NUM}\s*(?:M|K|G)?(?:triples?|rows?|entries|ops|queries|req)/s(?:ec)?\b", re.I), "throughput (records/s)"),
    (re.compile(rf"{NUM}\s*M/s\b"), "throughput (M/s)"),
    (re.compile(rf"{NUM}\s*ns/op\b", re.I), "latency (ns/op)"),
    # latency (a bare number + time unit, often with /query or /op)
    (re.compile(rf"{NUM}\s*(?:ns|µs|us|ms)\s*/\s*(?:op|query|row|call|item)\b", re.I), "latency (/op)"),
    (re.compile(rf"{NUM}\s*(?:ns|µs|us)\b(?!\w)", re.I), "latency (ns/µs)"),
    # speed-up ratios: "12× faster", "1.27x faster", "3× speedup"
    (re.compile(rf"{NUM}\s*(?:×|x)\s*(?:faster|slower|speed-?up)", re.I), "speed-up ratio"),
    (re.compile(rf"{NUM}\s*(?:×|x)\b(?=.{{0,30}}(?:faster|slower|speedup|throughput|latency))", re.I), "ratio (perf context)"),
    # bytes-per-record layout *measurements* (vs. fixed layout constants, see allow)
    (re.compile(rf"{NUM}\s*B/(?:triple|term|posting|row|node)\b", re.I), "size per record (B/record)"),
    (re.compile(rf"{NUM}\s*bytes?/(?:triple|term|posting|row|node)\b", re.I), "size per record (bytes/record)"),
]

# --- ALLOWLIST: lines containing any of these substrings are skipped entirely.
#     These mark legit non-perf numeric facts or already-relocated references. ---
ALLOW_LINE_SUBSTRINGS = [
    # algorithm / spec constants (not measurements)
    "k1=1.2", "b=0.75", "BM25",
    # platform / spec limits
    "wasm32", "4 GB", "2 GB", "4 GiB",
    # the canonical perf homes — a line that POINTS to them is fine
    "sparq.jeswr.org/dev/bench", "benchmarks.toml", "bench/CATALOG.md",
    "perf-baseline.json", "perf dashboard",
    # explicit opt-out marker a reviewer can add on a justified line
    "<!-- perf-ok -->", "perf-ok:",
]

# --- ALLOWLIST: whole paths/prefixes that are EXEMPT from the check.
#     bench/ and research/ are the SANCTIONED homes for measured numbers (AGENTS.md:
#     "Durable knowledge → … a research/ design record"; bench/ holds methodology +
#     baselines). The rule is about not baking numbers into USER-FACING docs. ---
ALLOW_PATH_PREFIXES = [
    "bench/",            # the benchmark corpus — numbers belong here
    "research/",         # design records + measured verdicts — numbers belong here
    "CHANGELOG.md",      # historical record of measured deltas at release time
    "conformance-report.md",
    "inference-conformance-report.md",
    "crates/sparq-conformance/FINDINGS.md",
    "zk/",               # research scaffolds (separate workspaces)
    "vendor/",
    "node_modules/",
    "target/",
    ".claude/",
    ".github/",
]

# Globs of files to scan. Markdown is the original surface; the published-paper Typst
# sources (`site/papers/**/*.typ`) AND the /specs Proposed-Specification Typst sources
# (`site/specs/**/*.typ`) are ALSO scanned so a hard-coded perf figure typed directly into
# a paper or a spec draft — the highest-stakes OUTWARD surfaces — trips the gate too
# (beads sq-mkza + sq-rvgr2.5). The `.typ` glob is git-tracked workspace-wide, so it is
# narrowed to those two trees by TYPST_SCAN_PREFIXES below; any other `.typ` (if one ever
# appears) is left alone. Result numbers in a paper flow through the Typst data accessor
# (`paper-evidence.json` → `#headline(...)` / `#ev(...)`) rather than being typed inline;
# a spec is a DESIGN surface and should carry no benchmark figure at all.
SCAN_GLOBS = ["*.md", "*.typ"]

# Only `.typ` files under one of these prefixes are scanned (the paper-factory sources and
# the /specs spec-factory sources). The accessor-aware, result-shaped-only `.typ` scan
# (scan_typst_file) is deliberately NARROWER than the markdown scan — see its docstring and
# design record research/paper-factory-honesty-gate-coverage.md §7 (Option b: scan for
# result-shaped numerics only, leave free-typed setup counts — dataset sizes / dimensions /
# k — alone, for author ergonomics). It catches only the COARSE perf-number class; subtle
# semantic overclaims remain Stage-5 human review. `str.startswith` accepts a tuple, so a
# path matches if it lives under ANY listed prefix. [OPUS-4.8] sq-rvgr2.5 added site/specs/.
TYPST_SCAN_PREFIXES = ("site/papers/", "site/specs/")


def is_typst_paper(path: str) -> bool:
    # [OPUS-4.8] sq-rvgr2.5: "paper" is historical — this is any scanned `.typ`, i.e. one
    # under the paper-factory OR the /specs spec-factory tree (TYPST_SCAN_PREFIXES).
    return path.endswith(".typ") and path.startswith(TYPST_SCAN_PREFIXES)


def tracked_sources() -> list[str]:
    """All git-tracked files in scope (markdown everywhere; `.typ` only under the
    paper / specs trees). Deterministic, no untracked scratch."""
    out = subprocess.run(
        ["git", "ls-files", *SCAN_GLOBS],
        capture_output=True, text=True, check=True,
    ).stdout
    return [
        p for p in out.splitlines()
        if p and (p.endswith(".md") or is_typst_paper(p))
    ]


def is_exempt_path(path: str) -> bool:
    return any(path == p or path.startswith(p) for p in ALLOW_PATH_PREFIXES)


class FileReadError(Exception):
    """[OPUS-4.8] Raised when a scanned file cannot be read/decoded.

    Surfaced rather than swallowed: silently skipping an unreadable file would hide
    any perf-number violations inside it and make --enforce non-deterministic.
    """


def scan_file(path: str) -> list[tuple[int, str, str]]:
    findings: list[tuple[int, str, str]] = []
    in_code_fence = False
    try:
        with open(path, encoding="utf-8") as fh:
            for lineno, raw in enumerate(fh, 1):
                line = raw.rstrip("\n")
                # [OPUS-4.8] Strip leading blockquote markers (`>` + spaces, possibly
                # nested e.g. `> > `) BEFORE the fence test, so blockquoted code fences
                # (`> ```lang`, as in docs/upstream-proposals.md) are recognised — else
                # perf-number lines inside them would be mis-scanned as prose.
                stripped = re.sub(r"^(?:\s*>)+\s*", "", line).lstrip()
                # Track fenced code blocks; perf numbers inside example output /
                # console transcripts are illustrative, not doc claims — skip them.
                if stripped.startswith("```") or stripped.startswith("~~~"):
                    in_code_fence = not in_code_fence
                    continue
                if in_code_fence:
                    continue
                if any(sub in line for sub in ALLOW_LINE_SUBSTRINGS):
                    continue
                for pat, label in PERF_PATTERNS:
                    m = pat.search(line)
                    if m:
                        findings.append((lineno, label, m.group(0).strip()))
                        break  # one finding per line is enough to flag it
    except (OSError, UnicodeDecodeError) as e:
        # [OPUS-4.8] Don't swallow the error: surface it so the caller can warn and,
        # in --enforce mode, fail (a skipped file could hide real violations).
        raise FileReadError(f"{path}: could not read: {e}") from e
    return findings


# --- Typst (.typ) paper scan ------------------------------------------------------
# [OPUS-4.8] bead sq-mkza. The paper-factory rule is: RESULT numbers flow through the
# Typst data accessor (`paper-evidence.json` → `#headline(key)` / `#ev(key)` /
# `#provenance(key)`), so the PDF and the in-site HTML cannot disagree and a number
# auto-updates with the evidence. A result-shaped figure typed DIRECTLY into prose
# bypasses that single-source discipline — that is what this scan flags. It reuses the
# SAME result-shaped PERF_PATTERNS as the markdown scan (a number adjacent to a
# throughput / latency / speed-up / size-per-record unit), so it is NARROW by
# construction: bare SETUP counts (dataset sizes, dimensions, k, query counts, years,
# citation numbers) are NOT result-shaped and are left alone (design §7, Option b).
#
# Two `.typ`-specific suppressions, so the accessor path and Typst comments never trip:
#   - ACCESSOR-AWARE: the value a paper is allowed to cite comes IN through an accessor
#     call whose argument is a string KEY (e.g. `#headline("ann.recall_at_10_floor")`),
#     never a numeric literal — so we strip accessor-call argument spans before matching.
#     This makes the legitimate canonical-data path provably unflaggable even if a future
#     key string happened to contain a unit-like substring.
#   - COMMENTS: Typst `//` line comments and `/* … */` block comments are author notes,
#     not published claims — stripped before matching (mirrors the markdown code-fence skip).
# Honest scope: this catches only the COARSE hard-coded-number class. A subtle semantic
# overclaim phrased without a result-shaped unit is NOT caught here — that remains
# Stage-5 human review (and the build-time headline() canonical gate in bench.typ).

# Accessor calls whose (string-key) argument span must be removed before scanning, so a
# unit-like substring inside a legitimate evidence KEY can never be read as a perf claim.
# [OPUS-5] sq-gum8.16: `headline_timing` / `timing_provenance` (site/papers/_lib/timing.typ)
# are the MEASURED-timing accessors, whose values are DERIVED from the committed canonical
# envelopes (site/src/data/paper-timing.generated.json) and always render with their
# provenance. They are accessors like the rest, so their key spans are stripped too. Listed
# BEFORE the shorter names so the alternation matches the full accessor name.
TYPST_ACCESSOR_CALL_RE = re.compile(
    r"#(?:headline_timing|timing_provenance|headline|ev|provenance)\s*\([^)]*\)", re.I
)


def _strip_typst_noise(line: str, in_block_comment: bool) -> tuple[str, bool]:
    """Return (scannable-text, still-in-block-comment) for one Typst source line.

    Removes `/* … */` block-comment spans (possibly multi-line), `//` line comments,
    and accessor-call argument spans — what's left is the free-typed prose the gate
    actually judges. Conservative: a `//` or `/*` inside a string literal is rare in
    these papers and erring toward STRIPPING (a false-negative on a contrived line)
    is the safe direction for an author-ergonomics gate; the result tables themselves
    are accessor-driven and unaffected.
    """
    out: list[str] = []
    i = 0
    n = len(line)
    while i < n:
        if in_block_comment:
            end = line.find("*/", i)
            if end == -1:
                return "".join(out), True  # comment continues past EOL
            i = end + 2
            in_block_comment = False
            continue
        # Start of a block comment?
        if line.startswith("/*", i):
            in_block_comment = True
            i += 2
            continue
        # Start of a line comment? (rest of the line is a comment)
        if line.startswith("//", i):
            break
        out.append(line[i])
        i += 1
    text = "".join(out)
    # Remove accessor-call argument spans (the canonical-data path).
    text = TYPST_ACCESSOR_CALL_RE.sub(" ", text)
    return text, in_block_comment


def scan_typst_file(path: str) -> list[tuple[int, str, str]]:
    """Accessor-aware, result-shaped-only scan of a paper `.typ` source (bead sq-mkza)."""
    findings: list[tuple[int, str, str]] = []
    in_block_comment = False
    try:
        with open(path, encoding="utf-8") as fh:
            for lineno, raw in enumerate(fh, 1):
                line = raw.rstrip("\n")
                # The inline allow-substrings (incl. the explicit `perf-ok:` opt-out and
                # the canonical-home pointers) apply here too, on the RAW line.
                if any(sub in line for sub in ALLOW_LINE_SUBSTRINGS):
                    continue
                scannable, in_block_comment = _strip_typst_noise(line, in_block_comment)
                if not scannable.strip():
                    continue
                for pat, label in PERF_PATTERNS:
                    m = pat.search(scannable)
                    if m:
                        findings.append((lineno, label, m.group(0).strip()))
                        break  # one finding per line is enough to flag it
    except (OSError, UnicodeDecodeError) as e:
        raise FileReadError(f"{path}: could not read: {e}") from e
    return findings


# --- paper-evidence.json prose scan ----------------------------------------------
# [OPUS-4.8] bead sq-4hga. The paper-factory evidence file carries human PROSE in its `note`
# (and the top-level `_comment`) free-text fields, which surface inline in a published paper
# via the `provenance()` helper. A result-shaped perf number typed into a record `note` (e.g.
# "…runs at 12× faster on WatDiv…") would bypass the canonical accessor discipline exactly
# like one typed into a `.typ` — so the SAME result-shaped PERF_PATTERNS are applied to those
# prose strings only. Everything else is left alone by design:
#   - Structured numeric metadata (`value`, `n_vectors`, `dim`, `k`, `n_queries`,
#     `scatter_penalty`, `schemaVersion`) is the canonical data path — the values a paper
#     cites THROUGH the accessor — the legitimate counterpart of the `.typ` accessor-call skip.
#   - The `source` field is a test/dataset PATH (e.g. "…recall.rs::hnsw_recall_at_10_vs_brute_
#     force_on_50k"), NOT prose: a count-like token in a test name is a legitimate identifier,
#     not a published claim, so `source` is deliberately NOT in the prose set.
# The narrow result-shaped net also means bare setup counts inside a `note` ("on a 20k
# synthetic set") are left alone (no throughput/latency/ratio unit). Honest scope: COARSE
# perf-number class only; a subtle semantic overclaim in a note remains Stage-5 human review
# (and the privacy-phrase half of the note scan is covered by check-privacy-claims.sh).
EVIDENCE_PATH = "site/src/data/paper-evidence.json"
# Only these string fields are PROSE; every other string (and all numbers) is structured data.
EVIDENCE_PROSE_FIELDS = frozenset({"note", "_comment"})

# [OPUS-5] sparq-org/sparq#4145. PUBLISHED prose that does not live in a `*.md`/`*.typ` file.
# `orchestration/start-here.toml` is the curated source of the maintainer front door (#1135):
# every line of `ask`/`what`/`process`/`flight` in it is rendered VERBATIM into an issue body by
# scripts/render-start-here.py, so it is exactly the surface this gate exists for — and it sat
# outside SCAN_GLOBS (`*.md`, `*.typ`) entirely. Listed explicitly rather than by widening
# SCAN_GLOBS to `*.toml`, which would sweep in every Cargo.toml / deny.toml in the workspace,
# where a bare number is configuration and not a claim. Its content is line-shaped prose, so it
# takes the ordinary markdown scan (fenced blocks and the ALLOW_LINE_SUBSTRINGS escape included).
# Pinned by test_start_here_render.py::test_the_front_door_toml_is_inside_the_perf_number_gate.
EXTRA_SCAN_PATHS = ("orchestration/start-here.toml",)


def scan_evidence_json(path: str) -> list[tuple[int, str, str]]:
    """Scan the prose (`note`/`_comment`) fields of paper-evidence.json (bead sq-4hga).

    Reports the 1-based line number of the offending prose field within the file, so a CI
    reader can jump straight to it (the JSON is pretty-printed, one field per line).
    """
    findings: list[tuple[int, str, str]] = []
    try:
        with open(path, encoding="utf-8") as fh:
            raw = fh.read()
    except (OSError, UnicodeDecodeError) as e:
        raise FileReadError(f"{path}: could not read: {e}") from e
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as e:
        # A malformed evidence file is a hard failure, not a silent skip — the build-time
        # honesty gate (build-papers.mjs) also parses it, but surfacing here keeps the
        # perf gate from passing a file it could not actually inspect.
        raise FileReadError(f"{path}: invalid JSON: {e}") from e

    # Map line numbers for the prose values so findings point at the real line. We re-scan
    # the raw text for each offending value rather than trust json positions (stdlib json
    # gives none), which is exact enough: prose `note`s are unique strings in this file.
    lines = raw.splitlines()

    def line_of(substr: str) -> int:
        for i, ln in enumerate(lines, 1):
            if substr in ln:
                return i
        return 0

    def walk(obj, field_name: str | None) -> None:
        if isinstance(obj, dict):
            for k, v in obj.items():
                walk(v, k)
        elif isinstance(obj, list):
            for v in obj:
                walk(v, field_name)
        elif isinstance(obj, str) and field_name in EVIDENCE_PROSE_FIELDS:
            if any(sub in obj for sub in ALLOW_LINE_SUBSTRINGS):
                return
            for pat, label in PERF_PATTERNS:
                m = pat.search(obj)
                if m:
                    snippet = m.group(0).strip()
                    findings.append((line_of(snippet), label, snippet))
                    break

    walk(data, None)
    return findings


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--advisory", action="store_true",
        help="report findings but always exit 0 (default CI mode while the backlog clears)",
    )
    ap.add_argument(
        "--enforce", action="store_true",
        help="exit non-zero if any finding remains (use once a scope is clean to ratchet it HARD)",
    )
    ap.add_argument(
        # [OPUS-5] #4145: print the RESOLVED default scan list and exit. Emitted from the same
        # `files` variable the scan loop below consumes, so a coverage test cannot pass against
        # a gate that has stopped scanning the file (which is the whole point of asking it).
        "--list-scanned", action="store_true",
        help="print the resolved default scan list (one path per line) and exit 0",
    )
    ap.add_argument(
        "paths", nargs="*",
        help="optional explicit paths to scan (default: all git-tracked markdown + the "
             "paper-factory .typ sources, minus the bench/research/changelog "
             "allowlisted homes). A `.typ` path uses the accessor-aware Typst scan.",
    )
    args = ap.parse_args()

    files = args.paths or [p for p in tracked_sources() if not is_exempt_path(p)]
    # [OPUS-4.8] bead sq-4hga: in default (whole-tree) mode, also scan the paper-evidence
    # prose fields. Added explicitly because the file is *.json (not in SCAN_GLOBS) and its
    # scan is field-aware, not line-shaped. An explicit-paths invocation that names the file
    # gets the same field-aware dispatch below.
    if not args.paths and os.path.exists(EVIDENCE_PATH) and EVIDENCE_PATH not in files:
        files = [*files, EVIDENCE_PATH]
    # [OPUS-5] #4145: published prose outside SCAN_GLOBS — see EXTRA_SCAN_PATHS.
    if not args.paths:
        files = [*files, *(p for p in EXTRA_SCAN_PATHS
                           if os.path.exists(p) and p not in files)]

    if args.list_scanned:
        for path in sorted(files):
            if not args.paths and is_exempt_path(path):
                continue
            print(path)
        return 0

    total = 0
    read_errors = 0  # [OPUS-4.8] unreadable files — never silently skipped
    summary_lines: list[str] = []
    for path in sorted(files):
        if not args.paths and is_exempt_path(path):
            continue
        try:
            # [OPUS-4.8] sq-mkza: `.typ` paper sources use the accessor-aware,
            # result-shaped-only scan; sq-4hga: the evidence JSON uses the field-aware
            # prose scan; everything else uses the markdown scan.
            if path.endswith("paper-evidence.json"):
                file_findings = scan_evidence_json(path)
            elif path.endswith(".typ"):
                file_findings = scan_typst_file(path)
            else:
                file_findings = scan_file(path)
        except FileReadError as e:
            # [OPUS-4.8] Warn on stderr always; in --enforce/non-advisory mode this
            # also counts as a failure below so a hidden file can't pass silently.
            read_errors += 1
            print(f"::warning::{e}", file=sys.stderr)
            summary_lines.append(f"- `{path}` — UNREADABLE (counted as a failure under --enforce)")
            continue
        for lineno, label, snippet in file_findings:
            total += 1
            print(f"{path}:{lineno}: perf-number [{label}]: {snippet!r}")
            summary_lines.append(f"- `{path}:{lineno}` — {label}: `{snippet}`")

    # Feed the GitHub step summary when available.
    step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if step_summary:
        with open(step_summary, "a", encoding="utf-8") as fh:
            if total == 0 and read_errors == 0:
                fh.write("### no-perf-numbers: clean ✅\n\n"
                         "No hard-coded performance figures found in the scanned docs.\n")
            else:
                fh.write(f"### no-perf-numbers (ADVISORY): {total} finding(s)"
                         f"{f', {read_errors} unreadable file(s)' if read_errors else ''} ⚠️\n\n")
                fh.write("Benchmark figures drift — relocate these to `bench/`/`research/` "
                         "and link the perf dashboard (AGENTS.md house rule). "
                         "Allowlist a justified line with `<!-- perf-ok -->`.\n\n")
                # Cap the inline list so the summary stays readable.
                for ln in summary_lines[:60]:
                    fh.write(ln + "\n")
                if len(summary_lines) > 60:
                    fh.write(f"\n…and {len(summary_lines) - 60} more (see the job log).\n")

    print(f"\nno-perf-numbers: {total} finding(s) across {len(files)} scanned file(s)"
          f"{f'; {read_errors} unreadable file(s)' if read_errors else ''}.")
    # [OPUS-4.8] --advisory always exits 0 (reports only). In --enforce/non-advisory
    # mode, BOTH perf-number findings AND unreadable files are failures — a file we
    # couldn't scan might be hiding a violation, so it must not pass silently.
    if args.enforce and not args.advisory and (total or read_errors):
        if total:
            print("::error::hard-coded performance numbers found (see above) — relocate to bench/research.")
        if read_errors:
            print(f"::error::{read_errors} file(s) could not be read (see warnings) — cannot certify clean.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
