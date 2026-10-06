#!/usr/bin/env python3
# README length-cap + required-sections check (bead sq-5fd1).
#
# Pairs with the crate-README template from the doc-overhaul design (bead sq-9jw5):
# a concise (<=120-line) per-crate README with three emoji section markers —
# "## 🚀 Quickstart", "## ✨ Features", "## 📚 Learn more" — plus a License section.
# `publish=false` crates may instead use a <=30-line stub carrying the explicit
# `<!-- internal-stub -->` HTML-comment directive (no section requirement).
#
# STATUS: HARD / REQUIRED GATE (bead sq-8ic6 — promoted from advisory). The
# sq-9jw5 overhaul brought every crate README into line and sq-ixsf tightened the
# stub classifier, so docs-quality.yml now runs this with --enforce (exit non-zero
# on any deviation) and the ci-summary aggregator GATES on the job
# "readme-template (template)" — its name carries no advisory/informational token.
# Net: EVERY crate README must conform, or the build fails. When you create or edit
# a crate README, run `python3 scripts/check-readme-template.py` and make it pass:
# <=120 lines + "## 🚀 Quickstart" / "## ✨ Features" / "## 📚 Learn more" + a
# License section (or, for a `publish=false` crate, a <=30-line README carrying the
# `<!-- internal-stub -->` directive). Put verbose/incidental API detail in rustdoc,
# not the README. `--advisory` (report-only, always exit 0) remains for local
# preview, but CI no longer uses it.

from __future__ import annotations

import argparse
import glob
import os
import re
import sys

MAX_LINES = 120
REQUIRED_SECTIONS = [
    ("🚀", re.compile(r"^##\s*🚀", re.M)),   # Quickstart
    ("✨", re.compile(r"^##\s*✨", re.M)),    # Features
    ("📚", re.compile(r"^##\s*📚", re.M)),    # Learn more
]
# Require a real License *section*, not any inline mention of the word
# "License" (e.g. "licensed under", a "license" badge). The template's License is a
# heading or a one-line section label at line-start: a markdown heading (`## License`),
# a bold label (`**License**`), or a leading `License` followed by `:`/`—`/`-`. Each
# alternative is anchored to the start of a line (re.M) so prose mentions don't count.
LICENSE_RE = re.compile(
    r"^\s{0,3}(?:#{1,6}\s*License\b"          # markdown heading: ## License
    r"|\*\*License\*\*"                         # bold label: **License**
    r"|License\s*(?:[:—-]|$))",                # one-line section: "License:" / "License —"
    re.M | re.I,
)
# sq-ixsf: A README is exempt from the section requirement ONLY when it carries
# an EXPLICIT internal-stub directive — the HTML comment the README-overhaul (sq-9jw5/sq-inzv)
# stamps on every publish=false stub, e.g.
#   <!-- sq-4kr5: internal-stub README for a publish=false crate. -->
# The previous heuristic substring-matched body PROSE ("not published", "internal crate", …),
# so a FULL-TEMPLATE README that merely *mentioned* those words anywhere was mis-classified as
# a <=30-line stub and silently exempted from the cap + the 🚀/✨/📚 + License requirements —
# letting real template deviations through. Before the --enforce flip that broad match would
# be a hole in the gate, so the directive is now required to be an EXPLICIT `internal-stub`
# token INSIDE an HTML comment (`<!-- … internal-stub … -->`), which cannot appear by accident
# in rendered prose.
STUB_DIRECTIVE_RE = re.compile(r"<!--[^>]*\binternal-stub\b[^>]*-->", re.I | re.S)


def is_stub_readme(text: str) -> bool:
    """True iff the README carries the explicit `internal-stub` HTML-comment directive."""
    return STUB_DIRECTIVE_RE.search(text) is not None


def check_readme_text(text: str) -> list[str]:
    """Template-conformance issues for README *content* (filesystem-free; testable)."""
    issues: list[str] = []
    n_lines = text.count("\n") + (0 if text.endswith("\n") else 1)

    if is_stub_readme(text):
        if n_lines > 30:
            issues.append(f"declared a stub but {n_lines} lines (stub target: <=30)")
        return issues

    if n_lines > MAX_LINES:
        issues.append(f"{n_lines} lines (template cap: <={MAX_LINES})")
    for emoji, rx in REQUIRED_SECTIONS:
        if not rx.search(text):
            issues.append(f"missing '## {emoji} …' section")
    if not LICENSE_RE.search(text):
        issues.append("no License section/line")
    return issues


def check_readme(path: str) -> list[str]:
    try:
        text = open(path, encoding="utf-8").read()
    except OSError as e:
        return [f"could not read: {e}"]
    return check_readme_text(text)


# sq-ixsf: hermetic self-test of the stub-classification + section/cap logic.
# No filesystem, no network — pins the gate's OWN behaviour so a future regression in the
# stub narrowing (or a re-broadening back to a prose substring match) fails CI here. Mirrors
# the perf-gate.py --self-test / coverage-gate.py --self-test pattern.
def _self_test() -> int:
    failures: list[str] = []

    def expect(name: str, cond: bool) -> None:
        if not cond:
            failures.append(name)

    # A FULL-TEMPLATE, over-cap README that merely MENTIONS "not published" / "internal
    # crate" in prose must NOT be exempted as a stub (the sq-ixsf false-positive). It has
    # no 🚀/✨/📚 sections, so with the old broad match it would have wrongly passed.
    full_mentions_stub_words = (
        "# sparq-foo\n\n"
        "This crate is internal, not published to crates.io in the usual sense, but it "
        "is documented in full here.\n\n"
        "## Overview\n\n" + ("filler line\n" * 200)
    )
    expect("prose-mention is NOT a stub", not is_stub_readme(full_mentions_stub_words))
    issues = check_readme_text(full_mentions_stub_words)
    expect("prose-mention README is flagged (cap + missing sections)",
           any("template cap" in i for i in issues)
           and any("missing '## 🚀" in i for i in issues))

    # An EXPLICIT internal-stub directive (HTML comment) IS a stub → exempt from sections,
    # passing while <=30 lines.
    stub_ok = (
        "<!-- sq-4kr5: internal-stub README for a publish=false crate. -->\n"
        "# sparq-bar\n\nInternal crate; see the design record.\n\n## License\n\nMIT.\n"
    )
    expect("explicit directive IS a stub", is_stub_readme(stub_ok))
    expect("short stub passes", check_readme_text(stub_ok) == [])

    # A stub directive but OVER the 30-line stub budget must still be flagged.
    stub_too_long = stub_ok + ("\nextra line\n" * 40)
    expect("over-long stub flagged",
           any("stub but" in i for i in check_readme_text(stub_too_long)))

    # A conformant full-template README (all sections + License, under cap) passes.
    good = (
        "# sparq-good\n\nA real crate.\n\n"
        "## 🚀 Quickstart\n\n`cargo add sparq-good`\n\n"
        "## ✨ Features\n\n- does things\n\n"
        "## 📚 Learn more\n\n- docs\n\n"
        "## License\n\nMIT.\n"
    )
    expect("conformant full template passes", check_readme_text(good) == [])

    # The directive must be inside an HTML comment — a bare word in prose is not enough.
    expect("bare 'internal-stub' word in prose is NOT a directive",
           not is_stub_readme("# x\n\nthis is an internal-stub of a sentence\n"))

    if failures:
        for f in failures:
            print(f"SELF-TEST FAIL: {f}")
        print(f"\ncheck-readme-template self-test: {len(failures)} failure(s).")
        return 1
    print("check-readme-template self-test: all cases pass ✅")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--advisory", action="store_true",
                    help="report only, always exit 0 (local preview; CI uses --enforce)")
    ap.add_argument("--enforce", action="store_true",
                    help="exit non-zero on any template deviation (the HARD CI gate, sq-8ic6)")
    ap.add_argument("--self-test", action="store_true",
                    help="run the hermetic stub-classification self-test and exit")
    ap.add_argument("paths", nargs="*",
                    help="explicit README paths (default: crates/*/README.md)")
    args = ap.parse_args()

    if args.self_test:
        return _self_test()

    paths = args.paths or sorted(glob.glob("crates/*/README.md"))
    total = 0
    summary: list[str] = []
    for path in paths:
        issues = check_readme(path)
        if issues:
            total += len(issues)
            for it in issues:
                print(f"{path}: {it}")
                summary.append(f"- `{path}` — {it}")

    step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if step_summary:
        with open(step_summary, "a", encoding="utf-8") as fh:
            if total == 0:
                fh.write("### readme-template: all conform ✅\n")
            else:
                fh.write(f"### readme-template: {total} deviation(s) ❌ (GATING — sq-8ic6)\n\n")
                fh.write("Crate READMEs MUST follow the concise template from sq-9jw5 "
                         f"(<= {MAX_LINES} lines; 🚀 Quickstart / ✨ Features / 📚 Learn more; "
                         "License) — or be a <=30-line `<!-- internal-stub -->` stub for a "
                         "`publish=false` crate. This is a HARD gate (--enforce); fix the "
                         "README(s) below to unblock the merge.\n\n")
                for ln in summary[:60]:
                    fh.write(ln + "\n")

    print(f"\nreadme-template: {total} deviation(s) across {len(paths)} README(s).")
    if total and args.enforce and not args.advisory:
        print("::error::crate READMEs deviate from the template (see above).")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
