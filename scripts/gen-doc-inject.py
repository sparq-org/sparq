#!/usr/bin/env python3
# [OPUS-5] issue #6132 — the BEGIN-INJECT/END-INJECT generator
# (research/docs-site-single-sourcing-anti-drift.md §5; §10 follow-up 2).
#
# WHY. mdBook's `{{#include file:anchor}}` single-sources prose into the guide, but it
# only works for files mdBook RENDERS. A crate `README.md` is consumed by three
# renderers that do not understand it — GitHub, crates.io, and rustdoc via
# `#![doc = include_str!("../README.md")]` — and a `skills/**/SKILL.md` is read by an
# agent straight off disk. So where the same stanza must appear in both a crate README
# and a SKILL.md, NEITHER can link to the other and NEITHER can preprocess. §5's answer
# is to MATERIALISE the copy into the file and check it:
#
#     <!-- BEGIN-INJECT: crates/sparq-vc/README.md#path-dep -->
#     …generated copy of that file's `ANCHOR: path-dep` region…
#     <!-- END-INJECT -->
#
# The file stays valid self-contained markdown for every renderer, the injected region
# is reviewable in the diff, and `--check` is the standard "generated file is up to
# date" gate. The source anchors are the SAME `<!-- ANCHOR: name -->` /
# `<!-- ANCHOR_END: name -->` markers mdBook already reads, so one anchor serves both
# the guide's `{{#include}}` and this generator — there is no second marker vocabulary.
#
# RELATIONSHIP TO THE DUPLICATION GATE (§4, issue #5019). That gate blanks
# BEGIN-INJECT/END-INJECT regions before comparing blocks, so an injected copy is not
# reported as a duplicate while a hand-copy of the same text still is. The two halves
# compose: this script keeps the region truthful, that gate keeps hand-copies out. It
# also means injecting a stanza OBSOLETES any allowlist entry that used to excuse it —
# and that gate FAILS on an entry which suppressed nothing (stale config), so the
# entry must be deleted in the same change. `--check` enforces exactly that
# coordination below (`obsoleted_allowlist_entries`), because the two changes land in
# different files and the failure is otherwise discovered on `ci-summary / gate`.
#
# SCOPE. Every tracked `*.md` outside vendored/build trees. A BEGIN-INJECT marker
# inside a fenced code block is DOCUMENTATION of the syntax, not a region, and is
# skipped — research/docs-site-single-sourcing-anti-drift.md §5 contains exactly such a
# fenced example, and processing it would rewrite the design record that specifies this
# script.
#
# Deterministic: no network, no build. Wired into docs-quality.yml's HARD quick-gates
# job (self-test + `--check`), alongside the other structural lints.

from __future__ import annotations

import argparse
import difflib
import json
import os
import re
import subprocess
import sys

# `<!-- BEGIN-INJECT: <repo-relative-source>#<anchor> -->` on its own line. The source
# path is resolved from the REPO ROOT (not the consumer's directory) so a region reads
# the same wherever it lives.
BEGIN_RE = re.compile(
    r"^([ \t]*)<!--\s*BEGIN-INJECT:\s*([^\s#]+)#([A-Za-z0-9_.\-]+)\s*-->[ \t]*$")
END_RE = re.compile(r"^[ \t]*<!--\s*END-INJECT\s*-->[ \t]*$")

# Any BEGIN/END marker anywhere on a line — used to reject a source anchor whose body
# carries one, which would corrupt the consumer's region boundaries on the next run.
ANY_MARKER_RE = re.compile(r"<!--\s*(BEGIN-INJECT:|END-INJECT\s*-->)")

# CommonMark fenced code block, ``` or ~~~ (3+). Info string allowed on the opener only.
FENCE_RE = re.compile(r"^[ \t]*(`{3,}|~{3,})")

# mdBook's anchor markers, matched anywhere on the line exactly as mdBook does.
ANCHOR_RE = re.compile(r"ANCHOR(_END)?:\s*([A-Za-z0-9_.\-]+)")

ALLOWLIST_PATH = "scripts/doc-duplication-allowlist.json"

# Tracked markdown that is vendored, generated or not part of any doc surface.
_SKIP_PARTS = ("/vendor/", "/target/", "/node_modules/", "/book/book/")
_SKIP_PREFIXES = ("vendor/", "target/", "node_modules/", ".beads/")


# --------------------------------------------------------------------------- sources

def extract_anchor(text: str, name: str) -> tuple[list[str] | None, str | None]:
    """The lines of `text`'s `ANCHOR: name` … `ANCHOR_END: name` region.

    mdBook semantics, deliberately: the marker lines themselves are excluded, and so is
    any line carrying ANOTHER anchor marker (nested anchors are common in the root
    README). Leading/trailing blank lines are trimmed so the materialised region is
    tight regardless of how the source spaces its markers.
    """
    lines = text.splitlines()
    starts = [i for i, l in enumerate(lines)
              if any(m.group(2) == name and not m.group(1) for m in ANCHOR_RE.finditer(l))]
    ends = [i for i, l in enumerate(lines)
            if any(m.group(2) == name and m.group(1) for m in ANCHOR_RE.finditer(l))]
    if not starts:
        return None, f"no `ANCHOR: {name}` in the source"
    if not ends:
        return None, f"no `ANCHOR_END: {name}` in the source"
    if len(starts) > 1 or len(ends) > 1:
        return None, f"anchor `{name}` is defined more than once in the source"
    if ends[0] < starts[0]:
        return None, f"`ANCHOR_END: {name}` precedes `ANCHOR: {name}` in the source"

    body = [l for l in lines[starts[0] + 1:ends[0]] if not ANCHOR_RE.search(l)]
    while body and not body[0].strip():
        body.pop(0)
    while body and not body[-1].strip():
        body.pop()
    if not body:
        return None, f"anchor `{name}` is empty"
    for l in body:
        if ANY_MARKER_RE.search(l):
            return None, (f"anchor `{name}` contains an INJECT marker — injecting it "
                          f"would corrupt the consumer's region boundaries")
    return body, None


def load_anchor(root: str, src: str, anchor: str,
                cache: dict[str, str | None]) -> tuple[list[str] | None, str | None]:
    if src not in cache:
        path = os.path.join(root, src)
        try:
            with open(path, encoding="utf-8") as fh:
                cache[src] = fh.read()
        except OSError:
            cache[src] = None
    text = cache[src]
    if text is None:
        return None, f"source file `{src}` does not exist"
    return extract_anchor(text, anchor)


# --------------------------------------------------------------------------- rewrite

def _closes(fm: re.Match, line: str, fence: str) -> bool:
    """A fence closes only with the same char, at least as long, and no info string."""
    tok = fm.group(1)
    return (tok[0] == fence[0] and len(tok) >= len(fence)
            and not line.strip()[len(tok):].strip())


def rewrite_text(path: str, text: str, root: str,
                 cache: dict[str, str | None]) -> tuple[str, list[str]]:
    """Materialise every BEGIN-INJECT region in `text`. Returns (new_text, errors)."""
    errors: list[str] = []
    keep_trailing_newline = text.endswith("\n")
    lines = text.splitlines()
    out: list[str] = []
    fence: str | None = None
    i = 0

    while i < len(lines):
        line = lines[i]
        fm = FENCE_RE.match(line)

        # Inside a fenced block: copy verbatim, watch only for the closer. A marker in
        # here is documentation of the syntax (research record §5), not a region.
        if fence is not None:
            if fm and _closes(fm, line, fence):
                fence = None
            out.append(line)
            i += 1
            continue
        if fm:
            fence = fm.group(1)
            out.append(line)
            i += 1
            continue

        bm = BEGIN_RE.match(line)
        if not bm:
            if END_RE.match(line):
                errors.append(f"{path}:{i + 1}: END-INJECT with no matching BEGIN-INJECT")
            out.append(line)
            i += 1
            continue

        # Locate the terminator. A second BEGIN before it means the region was never
        # closed, which we report rather than silently swallowing the file's remainder.
        j = i + 1
        while j < len(lines) and not END_RE.match(lines[j]) and not BEGIN_RE.match(lines[j]):
            j += 1
        if j >= len(lines) or not END_RE.match(lines[j]):
            errors.append(f"{path}:{i + 1}: unterminated BEGIN-INJECT region "
                          f"(no `<!-- END-INJECT -->`)")
            out.extend(lines[i:])
            break

        indent, src, anchor = bm.group(1), bm.group(2), bm.group(3)
        body, err = load_anchor(root, src, anchor, cache)
        if err is not None:
            errors.append(f"{path}:{i + 1}: BEGIN-INJECT {src}#{anchor}: {err}")
            out.extend(lines[i:j + 1])
            i = j + 1
            continue

        out.append(line)
        out.extend((indent + b) if b.strip() else "" for b in body)
        out.append(lines[j])
        i = j + 1

    new_text = "\n".join(out)
    if keep_trailing_newline and new_text:
        new_text += "\n"
    return new_text, errors


def regions_in(text: str) -> list[tuple[str, str]]:
    """The (source_path, anchor) pairs of every real (non-fenced) region in `text`."""
    found: list[tuple[str, str]] = []
    fence: str | None = None
    for line in text.splitlines():
        fm = FENCE_RE.match(line)
        if fence is not None:
            if fm and _closes(fm, line, fence):
                fence = None
            continue
        if fm:
            fence = fm.group(1)
            continue
        bm = BEGIN_RE.match(line)
        if bm:
            found.append((bm.group(2), bm.group(3)))
    return found


# ----------------------------------------------------- allowlist coordination (#6132)

def obsoleted_allowlist_entries(root: str,
                                regions: list[tuple[str, str, str]]) -> list[str]:
    """Allowlist entries that this file's injection has made obsolete.

    `regions` is (consumer, source, anchor). An entry in
    scripts/doc-duplication-allowlist.json excuses a duplication between a named set of
    FILES; once one of those files INJECTS its copy from another file in the same set,
    the duplication the entry excused no longer exists as a hand-copy, so the entry
    suppresses nothing and check-doc-duplication.py reds it as stale config. Deleting
    it belongs in the same change as the injection — this is what says so, by name,
    instead of leaving a generic stale-config failure on `ci-summary / gate`.

    Deliberately narrow: it fires only when the entry names BOTH the consumer and the
    source of a real region, so an entry covering an unrelated second duplication
    between two files that happen to also carry an injection is left alone. It is a
    coordination guard, not a duplication detector; the general stale-entry verdict
    remains check-doc-duplication.py's.

    Inert when the allowlist does not exist (it lands with #5019).
    """
    path = os.path.join(root, ALLOWLIST_PATH)
    if not os.path.exists(path):
        return []
    try:
        with open(path, encoding="utf-8") as fh:
            obj = json.load(fh)
    except (OSError, ValueError) as exc:
        return [f"{ALLOWLIST_PATH}: unreadable ({exc})"]

    findings: list[str] = []
    for n, entry in enumerate(obj.get("allow") or []):
        files = set(entry.get("files") or [])
        for consumer, source, anchor in regions:
            if consumer in files and source in files:
                findings.append(
                    f"{ALLOWLIST_PATH}: allow[{n}] naming {sorted(files)} is OBSOLETED by "
                    f"the BEGIN-INJECT region in `{consumer}` (source `{source}#{anchor}`)"
                    f" — DELETE the entry in this same change. It now suppresses nothing,"
                    f" which check-doc-duplication.py reds as stale config (issue #6132).")
                break
    return findings


# ------------------------------------------------------------------------------- run

def discover(root: str) -> list[str]:
    out = subprocess.run(["git", "-C", root, "ls-files", "-z", "*.md"],
                         check=True, capture_output=True, text=True).stdout
    files = []
    for rel in out.split("\0"):
        if not rel:
            continue
        if rel.startswith(_SKIP_PREFIXES) or any(p in "/" + rel for p in _SKIP_PARTS):
            continue
        files.append(rel)
    return sorted(files)


def run(root: str, files: list[str], check: bool) -> tuple[int, list[str]]:
    """Rewrite (or verify) every region. Returns (exit_code, report_lines)."""
    report: list[str] = []
    errors: list[str] = []
    drifted: list[str] = []
    all_regions: list[tuple[str, str, str]] = []
    cache: dict[str, str | None] = {}

    for rel in files:
        path = os.path.join(root, rel)
        try:
            with open(path, encoding="utf-8") as fh:
                text = fh.read()
        except OSError as exc:
            errors.append(f"{rel}: unreadable ({exc})")
            continue
        if "BEGIN-INJECT" not in text and "END-INJECT" not in text:
            continue

        for src, anchor in regions_in(text):
            all_regions.append((rel, src, anchor))

        new_text, errs = rewrite_text(rel, text, root, cache)
        errors.extend(errs)
        if new_text == text:
            continue
        drifted.append(rel)
        if check:
            report.extend(difflib.unified_diff(
                text.splitlines(), new_text.splitlines(),
                fromfile=f"{rel} (committed)", tofile=f"{rel} (generated)", lineterm=""))
        else:
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(new_text)
            report.append(f"updated {rel}")

    errors.extend(obsoleted_allowlist_entries(root, all_regions))

    if errors:
        report.append("")
        report.extend(f"ERROR: {e}" for e in errors)
        return 1, report
    if check and drifted:
        report.append("")
        report.append(f"{len(drifted)} injected region(s) are STALE: "
                      + ", ".join(drifted))
        report.append("Regenerate with: python3 scripts/gen-doc-inject.py")
        return 1, report
    if check:
        report.append(f"OK: {len(all_regions)} injected region(s) match their source anchors.")
    return 0, report


# ------------------------------------------------------------------------- self-test

def _self_test() -> int:
    import tempfile

    failures: list[str] = []

    def case(name: str, files: dict[str, str], *, check: bool,
             want_code: int, want_files: dict[str, str] | None = None,
             want_in_report: str | None = None) -> None:
        with tempfile.TemporaryDirectory() as td:
            for rel, body in files.items():
                p = os.path.join(td, rel)
                os.makedirs(os.path.dirname(p), exist_ok=True)
                with open(p, "w", encoding="utf-8") as fh:
                    fh.write(body)
            md = sorted(r for r in files if r.endswith(".md"))
            code, report = run(td, md, check)
            blob = "\n".join(report)
            if code != want_code:
                failures.append(f"{name}: exit {code}, want {want_code}\n{blob}")
                return
            if want_in_report and want_in_report not in blob:
                failures.append(f"{name}: report missing {want_in_report!r}\n{blob}")
                return
            for rel, expect in (want_files or {}).items():
                with open(os.path.join(td, rel), encoding="utf-8") as fh:
                    got = fh.read()
                if got != expect:
                    failures.append(f"{name}: {rel} is\n{got!r}\nwant\n{expect!r}")

    SRC = ("intro\n"
           "<!-- ANCHOR: stanza -->\n"
           "```toml\n"
           "dep = { path = \"x\" }\n"
           "```\n"
           "<!-- ANCHOR_END: stanza -->\n"
           "outro\n")
    FRESH = ("<!-- BEGIN-INJECT: src.md#stanza -->\n"
             "```toml\n"
             "dep = { path = \"x\" }\n"
             "```\n"
             "<!-- END-INJECT -->\n")
    STALE = ("<!-- BEGIN-INJECT: src.md#stanza -->\n"
             "```toml\n"
             "dep = { path = \"OLD\" }\n"
             "```\n"
             "<!-- END-INJECT -->\n")

    # --- the headline guard, both directions -------------------------------------
    case("check REDS on a region that drifted from its source",
         {"src.md": SRC, "c.md": STALE}, check=True, want_code=1,
         want_in_report="are STALE")
    case("check PASSES on a region that matches its source",
         {"src.md": SRC, "c.md": FRESH}, check=True, want_code=0,
         want_in_report="OK: 1 injected region(s)")

    # --- generation ----------------------------------------------------------------
    case("generate materialises the source anchor into the region",
         {"src.md": SRC, "c.md": STALE}, check=False, want_code=0,
         want_files={"c.md": FRESH})
    case("generate is idempotent on an already-fresh region",
         {"src.md": SRC, "c.md": FRESH}, check=False, want_code=0,
         want_files={"c.md": FRESH})
    case("an empty region is filled in",
         {"src.md": SRC,
          "c.md": "<!-- BEGIN-INJECT: src.md#stanza -->\n<!-- END-INJECT -->\n"},
         check=False, want_code=0, want_files={"c.md": FRESH})
    case("the BEGIN marker's indentation is applied to the body",
         {"src.md": "<!-- ANCHOR: a -->\nx\n\ny\n<!-- ANCHOR_END: a -->\n",
          "c.md": "  <!-- BEGIN-INJECT: src.md#a -->\n  <!-- END-INJECT -->\n"},
         check=False, want_code=0,
         want_files={"c.md": "  <!-- BEGIN-INJECT: src.md#a -->\n"
                             "  x\n\n  y\n  <!-- END-INJECT -->\n"})
    case("a nested ANCHOR marker line is dropped from the body (mdBook semantics)",
         {"src.md": "<!-- ANCHOR: a -->\nkeep\n<!-- ANCHOR: inner -->\nalso\n"
                    "<!-- ANCHOR_END: inner -->\n<!-- ANCHOR_END: a -->\n",
          "c.md": "<!-- BEGIN-INJECT: src.md#a -->\n<!-- END-INJECT -->\n"},
         check=False, want_code=0,
         want_files={"c.md": "<!-- BEGIN-INJECT: src.md#a -->\nkeep\nalso\n"
                             "<!-- END-INJECT -->\n"})

    # --- a marker inside a code fence is DOCUMENTATION, not a region ---------------
    #     research/docs-site-single-sourcing-anti-drift.md §5 is exactly this shape;
    #     processing it would rewrite the record that specifies this script.
    FENCED = ("```text\n"
              "<!-- BEGIN-INJECT: src.md#stanza -->\n"
              "…generated copy…\n"
              "<!-- END-INJECT -->\n"
              "```\n")
    case("a fenced BEGIN-INJECT example is left untouched",
         {"src.md": SRC, "doc.md": FENCED}, check=False, want_code=0,
         want_files={"doc.md": FENCED})
    case("...and does not red --check either",
         {"src.md": SRC, "doc.md": FENCED}, check=True, want_code=0)

    # --- errors --------------------------------------------------------------------
    case("a missing source file is an error",
         {"c.md": "<!-- BEGIN-INJECT: nope.md#a -->\n<!-- END-INJECT -->\n"},
         check=True, want_code=1, want_in_report="does not exist")
    case("a missing anchor is an error",
         {"src.md": SRC, "c.md": "<!-- BEGIN-INJECT: src.md#nope -->\n<!-- END-INJECT -->\n"},
         check=True, want_code=1, want_in_report="no `ANCHOR: nope`")
    case("an unterminated region is an error",
         {"src.md": SRC, "c.md": "<!-- BEGIN-INJECT: src.md#stanza -->\ntail\n"},
         check=True, want_code=1, want_in_report="unterminated BEGIN-INJECT")
    case("a stray END-INJECT is an error",
         {"c.md": "<!-- END-INJECT -->\n"},
         check=True, want_code=1, want_in_report="no matching BEGIN-INJECT")
    case("a source anchor carrying an INJECT marker is refused (would corrupt bounds)",
         {"src.md": "<!-- ANCHOR: a -->\n<!-- END-INJECT -->\n<!-- ANCHOR_END: a -->\n",
          "c.md": "<!-- BEGIN-INJECT: src.md#a -->\n<!-- END-INJECT -->\n"},
         check=True, want_code=1, want_in_report="contains an INJECT marker")
    case("a doubly-defined anchor is an error",
         {"src.md": "<!-- ANCHOR: a -->\nx\n<!-- ANCHOR_END: a -->\n"
                    "<!-- ANCHOR: a -->\ny\n<!-- ANCHOR_END: a -->\n",
          "c.md": "<!-- BEGIN-INJECT: src.md#a -->\n<!-- END-INJECT -->\n"},
         check=True, want_code=1, want_in_report="defined more than once")

    # --- the #6132 coordination guard, both directions -----------------------------
    def allowlist(files: list[str]) -> str:
        return json.dumps({"allow": [{"files": files, "digests": ["d"], "why": "w"}]})

    case("an allowlist entry naming the consumer AND the source is called obsolete",
         {"src.md": SRC, "c.md": FRESH,
          "scripts/doc-duplication-allowlist.json": allowlist(["c.md", "src.md"])},
         check=True, want_code=1, want_in_report="OBSOLETED by the BEGIN-INJECT region")
    case("an entry naming an UNRELATED pair is left alone",
         {"src.md": SRC, "c.md": FRESH,
          "scripts/doc-duplication-allowlist.json": allowlist(["p.md", "q.md"])},
         check=True, want_code=0)
    case("an entry naming the consumer but NOT the source is left alone",
         {"src.md": SRC, "c.md": FRESH,
          "scripts/doc-duplication-allowlist.json": allowlist(["c.md", "q.md"])},
         check=True, want_code=0)
    case("the guard is inert while the allowlist does not exist (#5019 unmerged)",
         {"src.md": SRC, "c.md": FRESH}, check=True, want_code=0)

    for f in failures:
        print(f"FAIL {f}", file=sys.stderr)
    print(f"gen-doc-inject self-test: {len(failures)} failure(s)")
    return 1 if failures else 0


# ------------------------------------------------------------------------------ main

def main() -> int:
    ap = argparse.ArgumentParser(
        description="Materialise BEGIN-INJECT/END-INJECT regions from their source "
                    "anchors (research/docs-site-single-sourcing-anti-drift.md §5).")
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    ap.add_argument("--check", action="store_true",
                    help="verify the committed regions are up to date; do not write")
    ap.add_argument("--self-test", action="store_true", help="run hermetic fixtures and exit")
    args = ap.parse_args()

    if args.self_test:
        return _self_test()

    code, report = run(args.root, discover(args.root), args.check)
    for line in report:
        print(line)
    return code


if __name__ == "__main__":
    sys.exit(main())
