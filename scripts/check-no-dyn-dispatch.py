#!/usr/bin/env python3
# Perf-neutrality structural gate: NO dynamic dispatch in the
# sparq-substrate hot loops (bead sq-0ja86, epic sq-6tykl / sq-qonbz — the shared
# zero-overhead eval substrate).
#
# WHY. crates/sparq-substrate is a leaf crate (depends only on sparq-core) holding the
# shared evaluation substrate consumed by BOTH sparq-engine AND the reasoners. The
# architectural invariant (maintainer directive, research/shared-eval-substrate.md §2.3
# / §4) is that its hot paths — the four join kernels (merge / hash / bind / leapfrog
# trie-join) behind the generic `JoinKeys` descriptor + generic `Budget` cancel hook,
# and the XSD numeric value tower — must be MONOMORPHISED. A `Box<dyn …>` / `&dyn` /
# `dyn` trait object on a per-row / per-key-group / per-distinct-value hot loop would
# insert a vtable indirect call the optimiser cannot inline, making the substrate NOT
# zero-overhead for its two consumers and risking a regression of the deterministic
# byte ratchets (wasm_bundle_bytes, store/dict bytes). Generic type parameters bounded
# by a trait (`fn merge_join<B: Budget>(…)`) are FINE — they monomorphise; trait
# OBJECTS (`dyn`) are not. This grep enforces the distinction structurally.
#
# This MIRRORS scripts/check-no-perf-numbers.py (the repo's structural-grep gate
# convention): a small Python script that scans a fixed file set, prints clear
# `path:line:` findings, feeds the GitHub step summary, and exits non-zero on any
# violation outside a narrow, explicitly-commented allowlist. It is deterministic
# (grep-only, no network, no build) and is wired into the same docs-quality
# structural-checks gate surface as check-no-perf-numbers.py.
#
# COMMENT-AWARE. The substrate source legitimately *mentions* "`Box<dyn>`" / "`&dyn`"
# in its doc-comments to DOCUMENT the invariant ("there is NO `Box<dyn>` … anywhere").
# Flagging those prose mentions would be a false positive, so Rust line comments
# (`//`, `///`, `//!`) and block comments (`/* … */`, including doc `/** … */`) are
# stripped before matching — exactly as the perf-numbers gate strips code fences /
# Typst comments. Only `dyn` in actual *code* is a violation.
#
# ALLOWLIST. A genuinely-cold path that needs a trait object can opt out with an
# explicit, reason-bearing trailing marker on that line:
#     let f: Box<dyn Fn()> = …;  // perf-neutrality-allow: cold one-time planner setup
# The marker MUST carry a non-empty reason; a bare marker is rejected so the opt-out
# stays auditable. Use it sparingly — the whole point of the substrate is that the hot
# loops have none.

from __future__ import annotations

import argparse
import os
import re
import sys

# --- The substrate hot-path source files this gate guards. Scoped narrowly to the
#     leaf crate's hot paths (join kernels + numeric tower + the shared row/key
#     vocabulary they operate on) plus lib.rs, which today is module wiring + the
#     zero-overhead doc-contract but is included so a future hot helper added there is
#     also covered. Adding a new hot-path module to the substrate? Add it here.
#
#     sq-qonbz.7: EXPLICIT enumeration of the FOUR substrate hot-loop modules
#     (join::delta is a SUBMODULE of join, not a fifth top-level module — the count and the
#     list agree at four):
#     - rows (shared Row/Key/Posting id-tuple vocabulary)
#     - numeric (XSD numeric value tower + arithmetic + lexical helpers)
#     - join (four id-tuple join kernels: merge-join, hash-join, bind-join, trie-join/WCOJ;
#       INCLUDES the join::delta submodule — persistent extendable build-side hash table for
#       semi-naive Δ⋈full join)
#     - compare (SPARQL term total order over a generic CompareTerm trait)
#     This enumeration makes the perf-neutrality boundary structurally explicit and auditable.
SUBSTRATE_HOT_PATHS = [
    # Rows: shared Row/Key/Posting vocabulary for both join kernels and reasoner consumers.
    "crates/sparq-substrate/src/rows.rs",
    # Numeric: XSD numeric value tower (Num/Dec + as_numeric + arithmetic ops).
    "crates/sparq-substrate/src/numeric.rs",
    # Join: four id-tuple join kernels (merge-join, hash-join, bind-join, trie-join/WCOJ)
    # behind a generic JoinKeys descriptor and generic Budget cooperative-cancel hook.
    # INCLUDES join::delta submodule (persistent extendable hash table for semi-naive
    # Δ⋈full join — no Box<dyn> on the delta probe path either).
    "crates/sparq-substrate/src/join.rs",
    # sq-vezew (epic sq-qonbz, Phase 4): SPARQL term total order
    # (compare::compare_terms, generic over the CompareTerm trait) — an ORDER BY / sort /
    # range-filter hot path, so it joins the guarded set: the algorithm must stay monomorphic
    # (a `dyn CompareTerm` on the per-comparison loop would defeat the zero-overhead seam).
    "crates/sparq-substrate/src/compare.rs",
    # Library wiring and zero-overhead doc-contract.
    "crates/sparq-substrate/src/lib.rs",
    # sq-2n1q3.4: the first guarded CONSUMER probe path — sparq-rsp's windowed
    # materialisation drives join::delta::DeltaTable for the EvalMode::Delta/Snapshot
    # consecutive-window (ISTREAM/DSTREAM-shaped) diff (WindowDiff::contains /
    # apply_window_delta). The invariant applies on the consumer side of the seam too: the
    # probe's emit hook is a monomorphised closure and its budget the NoBudget ZST — never
    # a trait object between the probe loop and its comparison.
    "crates/sparq-rsp/src/eval.rs",
    # sq-pbz04.1.2: the reasoner-side CompareTerm adoption (substrate seam 3) —
    # sparq-reason's `compare` module orders entailed solutions through the shared
    # compare_terms total order. The consumer-side invariant applies here too: IdTerm is a
    # generic CompareTerm impl monomorphised into the sort loop — never a trait object
    # between a comparison and the term observations it makes.
    "crates/sparq-reason/src/compare.rs",
]

# --- The dynamic-dispatch patterns. We match the three trait-OBJECT spellings; a
#     generic `<B: Budget>` bound is NOT matched (it monomorphises, which is the whole
#     design). `\bdyn\b` catches `dyn Trait`, `impl SomeTrait + dyn …`, `Rc<dyn …>`,
#     `Arc<dyn …>`, `*const dyn …`, etc.; the `Box<dyn` / `&dyn` entries make the two
#     most common spellings explicit in the message. One match per line is enough. ---
DYN_PATTERNS = [
    (re.compile(r"Box\s*<\s*dyn\b"), "Box<dyn …> heap trait object"),
    (re.compile(r"&\s*(?:'\w+\s+)?(?:mut\s+)?dyn\b"), "&dyn … borrowed trait object"),
    (re.compile(r"\bdyn\b"), "dyn … trait object"),
]

# The explicit per-line opt-out marker for a genuinely-cold path. Must carry a reason.
ALLOW_RE = re.compile(r"//\s*perf-neutrality-allow:\s*(\S.*)$")


def strip_rust_comments(text: str) -> str:
    """Replace Rust `//` line comments and `/* … */` block comments (incl. doc
    variants `///`, `//!`, `/** … */`, `/*! … */`) with spaces, preserving line
    structure (newlines and column count) so reported line numbers stay exact.

    String/char literals are NOT specially handled: a `dyn` inside a string literal in
    these hot-path files would itself be unusual and worth a human glance, and the
    allowlist marker covers any real false positive. The conservative direction here is
    to NOT silently drop a candidate match. The comment strip exists solely to avoid
    flagging the documented invariant prose, which is the only real false-positive
    source in this file set.
    """
    out: list[str] = []
    i = 0
    n = len(text)
    in_line_comment = False
    in_block_comment = False
    while i < n:
        c = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if in_line_comment:
            if c == "\n":
                in_line_comment = False
                out.append(c)
            else:
                out.append(" ")
            i += 1
            continue
        if in_block_comment:
            if c == "*" and nxt == "/":
                in_block_comment = False
                out.append("  ")
                i += 2
            else:
                # Preserve newlines so line numbers don't shift.
                out.append("\n" if c == "\n" else " ")
                i += 1
            continue
        # Not currently in a comment.
        if c == "/" and nxt == "/":
            in_line_comment = True
            out.append("  ")
            i += 2
            continue
        if c == "/" and nxt == "*":
            in_block_comment = True
            out.append("  ")
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def scan_file(path: str) -> tuple[list[tuple[int, str, str]], list[str]]:
    """Return (findings, warnings) for one hot-path file.

    A finding is `(lineno, label, snippet)`. A line carrying a valid
    `// perf-neutrality-allow: <reason>` marker is skipped (and not a finding). A bare
    marker with no reason is itself reported as a finding so the opt-out can't be
    smuggled in without justification.
    """
    findings: list[tuple[int, str, str]] = []
    warnings: list[str] = []
    try:
        with open(path, encoding="utf-8") as fh:
            raw = fh.read()
    except (OSError, UnicodeDecodeError) as e:
        # Surface, never swallow: a hot-path file we cannot read might be hiding a
        # violation, so it counts as a failure (handled by the caller).
        warnings.append(f"{path}: could not read: {e}")
        return findings, warnings

    raw_lines = raw.splitlines()
    code = strip_rust_comments(raw)
    code_lines = code.splitlines()

    for idx, code_line in enumerate(code_lines):
        lineno = idx + 1
        raw_line = raw_lines[idx] if idx < len(raw_lines) else ""

        # Does the ORIGINAL line carry an opt-out marker? (The marker lives in a
        # comment, which strip_rust_comments removed, so test the raw line.)
        allow_m = ALLOW_RE.search(raw_line)
        has_allow = allow_m is not None
        allow_reason = allow_m.group(1).strip() if allow_m else ""

        match = None
        match_label = ""
        for pat, label in DYN_PATTERNS:
            m = pat.search(code_line)
            if m:
                match = m
                match_label = label
                break

        if match is None:
            # No dyn in code on this line. A stray allow marker with no dyn is harmless;
            # leave it (it documents intent and may guard a multi-line construct's head).
            continue

        if has_allow and allow_reason:
            # Explicitly justified cold-path opt-out — permitted.
            continue
        if has_allow and not allow_reason:
            findings.append((
                lineno,
                "empty perf-neutrality-allow (reason required)",
                raw_line.strip(),
            ))
            continue

        findings.append((lineno, match_label, raw_line.strip()))

    return findings, warnings


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "paths", nargs="*",
        help="optional explicit hot-path files to scan (default: the substrate "
             "hot-path file set). Mainly for the self-test / planted-violation check.",
    )
    args = ap.parse_args()

    files = args.paths or SUBSTRATE_HOT_PATHS

    total = 0
    read_errors = 0
    summary_lines: list[str] = []
    for path in files:
        if not os.path.exists(path):
            # A configured hot-path file that has vanished is a real problem (the gate
            # would silently guard nothing) — surface it as a failure.
            read_errors += 1
            print(f"::warning::{path}: configured hot-path file is missing")
            summary_lines.append(f"- `{path}` — MISSING configured hot-path file")
            continue
        findings, warnings = scan_file(path)
        for w in warnings:
            read_errors += 1
            print(f"::warning::{w}")
            summary_lines.append(f"- `{path}` — UNREADABLE ({w})")
        for lineno, label, snippet in findings:
            total += 1
            print(f"{path}:{lineno}: dynamic-dispatch [{label}]: {snippet!r}")
            summary_lines.append(f"- `{path}:{lineno}` — {label}: `{snippet}`")

    step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if step_summary:
        with open(step_summary, "a", encoding="utf-8") as fh:
            if total == 0 and read_errors == 0:
                fh.write("### no-dyn-dispatch (substrate perf-neutrality): clean ✅\n\n"
                         "No dynamic dispatch in the sparq-substrate hot loops — the join "
                         "kernels and numeric tower stay monomorphised.\n")
            else:
                fh.write(f"### no-dyn-dispatch (substrate perf-neutrality): "
                         f"{total} violation(s)"
                         f"{f', {read_errors} unreadable/missing file(s)' if read_errors else ''} ❌\n\n")
                fh.write("A `dyn` / `Box<dyn>` / `&dyn` trait object on a substrate hot path "
                         "inserts a vtable the optimiser cannot inline — breaking the "
                         "zero-overhead invariant (research/shared-eval-substrate.md §2.3/§4). "
                         "Use a generic type parameter bounded by the trait instead "
                         "(`fn …<B: Budget>(…)`), or — for a genuinely-cold path — annotate the "
                         "line `// perf-neutrality-allow: <reason>`.\n\n")
                for ln in summary_lines[:60]:
                    fh.write(ln + "\n")
                if len(summary_lines) > 60:
                    fh.write(f"\n…and {len(summary_lines) - 60} more (see the job log).\n")

    print(f"\nno-dyn-dispatch: {total} violation(s) across {len(files)} hot-path file(s)"
          f"{f'; {read_errors} unreadable/missing file(s)' if read_errors else ''}.")
    if total or read_errors:
        if total:
            print("::error::dynamic dispatch found in the sparq-substrate hot loops "
                  "(see above) — the substrate must stay monomorphised "
                  "(research/shared-eval-substrate.md §2.3/§4). Use a generic <B: Budget>-style "
                  "bound, or mark a cold path `// perf-neutrality-allow: <reason>`.")
        if read_errors:
            print(f"::error::{read_errors} hot-path file(s) missing/unreadable — cannot certify clean.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
