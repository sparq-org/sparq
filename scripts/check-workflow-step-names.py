#!/usr/bin/env python3
# [OPUS-5] CI lint (#5574): a `name:` value in a workflow file must never be an
# UNQUOTED plain scalar containing ` #`.
#
# WHY THIS GATE EXISTS — silent step-name truncation. In YAML a `#` preceded by
# whitespace opens a comment, so
#
#     - name: Self-test the advisory-registry checker (C2/C3/C4 logic — sq-qcnn.32, #3773)
#
# parses as the name `Self-test the advisory-registry checker (C2/C3/C4 logic —
# sq-qcnn.32,` — truncated mid-clause with an unbalanced paren, and the `#3773`
# issue reference GONE. That reference is the whole reason the name carries a
# number: it is what makes a CI log line greppable back to the issue that
# introduced the step. Execution is unaffected, which is exactly why the drift is
# invisible — 12 step names across 5 workflows had accumulated it before #5574.
# Quoting the value ("...") makes the `#` literal text again.
#
# RULE: scan .github/workflows/*.yml (+ *.yaml). For each line whose key is `name:`
# (workflow name, job name, step name, or a `with: name:` input), FAIL when the
# value is a plain (unquoted) scalar that either contains ` #`/`\t#` or begins with
# `#` — both of which YAML eats as a comment. A value already wrapped in `"` or `'`
# is fine; a block scalar (`|`, `>`) is fine; lines INSIDE a block scalar are not
# YAML at all and are skipped.
#
# WHY A SOURCE-TEXT SCANNER AND NOT A YAML PARSER: a parser cannot see this defect
# AT ALL. By the time PyYAML has loaded the file the comment is already stripped and
# the truncated name is indistinguishable from a name someone meant to write that
# way — both are just the string `... (C2/C3/C4 logic — sq-qcnn.32,`. The bug exists
# only in the SOURCE TEXT, so the check must read source text. (This is why the
# scanner tracks block scalars by indentation itself: `run: |` bodies are embedded
# shell/python where a `# comment` is intentional, and it must not flag those.)
# It happens to need no third-party import; that is a side benefit, not the reason.
#
# EXIT: 0 when every `name:` is safe; 1 with a per-offence message otherwise.
#
# Usage:
#   check-workflow-step-names.py                  # scan .github/workflows
#   check-workflow-step-names.py --root <dir>     # scan <dir>/.github/workflows
#   check-workflow-step-names.py --self-test      # hermetic logic self-test
#
# stdlib-only.

from __future__ import annotations

import argparse
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# A `name:` key line: optional indent, optional `- ` list dash, the key, then the
# value. `(?:- )?` lets the key be introduced on the dash line of a step.
_NAME_RE = re.compile(r"^([ \t]*)(?:- )?name:[ \t]*(.*)$")
# Any key line, used to find the `foo: |`/`foo: >` that opens a block scalar.
_KEY_RE = re.compile(r"^([ \t]*)(?:- )?[A-Za-z_][\w.-]*:[ \t]*(.*)$")
# A block-scalar header: `|`, `>`, plus optional chomping/indent indicators
# (`|-`, `>+`, `|2`) and an optional trailing comment.
_BLOCK_RE = re.compile(r"^[|>][+-]?\d*[+-]?[ \t]*(?:#.*)?$")
# The offending shape: whitespace then `#` anywhere in the value.
_BARE_HASH_RE = re.compile(r"[ \t]#")


def _indent(line: str) -> int:
    """Column at which the line's content starts (tabs count as one column; these
    files are space-indented, and a tab would be invalid YAML indentation anyway)."""
    return len(line) - len(line.lstrip(" \t"))


def scan_text(text: str) -> list[tuple[int, str]]:
    """Return [(line_number, offending_line)] for one workflow file's text.

    Lines inside a block scalar (`run: |`, `script: >`) are embedded shell/python,
    not YAML, so a `# comment` there is intentional and must not be flagged. We
    track the block-scalar body by indentation: everything indented deeper than the
    key that opened it belongs to it.
    """
    offences: list[tuple[int, str]] = []
    block_indent: int | None = None
    for lineno, raw in enumerate(text.split("\n"), 1):
        stripped = raw.strip()
        if block_indent is not None:
            # Blank lines never terminate a block scalar; any line indented deeper
            # than the opening key is still body.
            if not stripped or _indent(raw) > block_indent:
                continue
            block_indent = None

        if not stripped or stripped.startswith("#"):
            continue

        key = _KEY_RE.match(raw)
        if key and _BLOCK_RE.match(key.group(2)):
            block_indent = _indent(raw)
            continue

        m = _NAME_RE.match(raw)
        if not m:
            continue
        value = m.group(2)
        if not value or value.startswith(('"', "'")):
            # Empty (a nested mapping follows) or already quoted → safe.
            continue
        if value.startswith("#") or _BARE_HASH_RE.search(value):
            offences.append((lineno, raw.rstrip()))
    return offences


def scan_workflows(root: Path) -> list[tuple[Path, int, str]]:
    """Scan every workflow file under `root`/.github/workflows."""
    wf_dir = root / ".github" / "workflows"
    results: list[tuple[Path, int, str]] = []
    if not wf_dir.is_dir():
        return results
    for path in sorted(wf_dir.iterdir()):
        if path.suffix not in (".yml", ".yaml"):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for lineno, line in scan_text(text):
            results.append((path, lineno, line))
    return results


# --------------------------------------------------------------------------- tests
# The real #5574 shape: an unquoted step name with a trailing ` #1234)` reference.
_BAD_TRAILING_REF = """\
jobs:
  a:
    steps:
      - name: Self-test the advisory-registry checker (C2/C3/C4 logic — sq-qcnn.32, #3773)
        run: python3 scripts/check-advisory-registry.py --self-test
"""

# The same name, quoted — the fix.
_GOOD_QUOTED = """\
jobs:
  a:
    steps:
      - name: "Self-test the advisory-registry checker (C2/C3/C4 logic — sq-qcnn.32, #3773)"
        run: python3 scripts/check-advisory-registry.py --self-test
"""

# Single quotes are equally fine.
_GOOD_SINGLE_QUOTED = """\
jobs:
  a:
    steps:
      - name: 'Publish-cadence guard (interval, #1135)'
        run: true
"""

# A name with no `#` at all needs no quoting.
_GOOD_NO_HASH = """\
jobs:
  a:
    steps:
      - name: Self-test install-action tool-selector gate (pure logic)
        run: python3 scripts/check-install-action-tool.py --self-test
"""

# A `#` NOT preceded by whitespace does not open a comment, so `issue#7` survives.
_GOOD_HASH_NO_SPACE = """\
jobs:
  a:
    steps:
      - name: Guard for issue#7 regressions
        run: true
"""

# A value that is nothing but a comment parses as a NULL name.
_BAD_LEADING_HASH = """\
jobs:
  a:
    steps:
      - name: #5574
        run: true
"""

# `#` inside a `run: |` block scalar is a shell comment — must NOT be flagged, even
# when the embedded text itself contains a `name:` key (we generate YAML in CI).
_GOOD_BLOCK_SCALAR = """\
jobs:
  a:
    steps:
      - name: Emit a workflow
        run: |
          # write a workflow fixture, see #1234
          cat > out.yml <<'EOF'
          - name: inner step (#4321)
          EOF
      - name: after the block
        run: true
"""

# Block-scalar tracking must not swallow the following sibling step: this file's
# SECOND step is a real offence and must still be caught.
_BAD_AFTER_BLOCK = """\
jobs:
  a:
    steps:
      - name: Emit
        run: |
          echo hi # not a yaml comment
      - name: Publish-cadence guard (interval, #2552)
        run: true
"""

# Workflow-level, job-level and `with:` inputs are all real YAML scalars with the
# same truncation risk.
_BAD_NON_STEP_LEVELS = """\
name: Docs quality (see #1)
jobs:
  a:
    name: build (see #2)
    steps:
      - uses: actions/upload-artifact@1111111111111111111111111111111111111111
        with:
          name: report (see #3)
"""

# A blank line inside a block scalar must not end it.
_GOOD_BLANK_IN_BLOCK = """\
jobs:
  a:
    steps:
      - name: Emit
        run: |
          echo one # comment

          echo two # comment
"""


def self_test() -> int:
    cases = [
        ("bad (unquoted trailing #ref — the #5574 shape)", _BAD_TRAILING_REF, 1),
        ("good (double-quoted)", _GOOD_QUOTED, 0),
        ("good (single-quoted)", _GOOD_SINGLE_QUOTED, 0),
        ("good (no # at all)", _GOOD_NO_HASH, 0),
        ("good (# not preceded by whitespace)", _GOOD_HASH_NO_SPACE, 0),
        ("bad (value is only a comment → null name)", _BAD_LEADING_HASH, 1),
        ("good (# inside a run: | block scalar)", _GOOD_BLOCK_SCALAR, 0),
        ("bad (offence on the step AFTER a block scalar)", _BAD_AFTER_BLOCK, 1),
        ("bad (workflow + job + with: name levels)", _BAD_NON_STEP_LEVELS, 3),
        ("good (blank line does not end a block scalar)", _GOOD_BLANK_IN_BLOCK, 0),
    ]
    failures = 0
    for label, text, expected in cases:
        got = len(scan_text(text))
        ok = got == expected
        print(
            f"  [{'PASS' if ok else 'FAIL'}] {label}: "
            f"{got} offence(s) (want {expected})"
        )
        if not ok:
            failures += 1
    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print("\nself-test: all cases PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Fail if any workflow `name:` is an unquoted plain scalar whose "
        "` #` YAML would silently strip as a comment (#5574)."
    )
    ap.add_argument(
        "--root",
        default=str(REPO_ROOT),
        help="repo root whose .github/workflows is scanned (default: this repo).",
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="run the hermetic logic self-test and exit.",
    )
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    offences = scan_workflows(Path(args.root))
    if not offences:
        print(
            "workflow name truncation gate: PASS — no unquoted `name:` value "
            "contains a YAML-comment-opening ` #`."
        )
        return 0

    print("workflow name truncation gate: FAIL\n")
    print(
        "These `name:` values are UNQUOTED plain scalars containing ` #`. YAML treats\n"
        "the rest of the line as a comment, so the name is silently TRUNCATED and the\n"
        "issue reference — the thing that makes the CI log greppable — is dropped\n"
        "(#5574). Wrap each value in double quotes:\n"
    )
    root_path = Path(args.root)
    for path, lineno, line in offences:
        rel = path.relative_to(root_path) if path.is_relative_to(root_path) else path
        print(f"    - {rel}:{lineno}: {line.strip()}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
