#!/usr/bin/env python3
# [OPUS-5] CI lint (#6123): a workflow `name:` written as a PLAIN (unquoted) scalar
# must not contain a whitespace-preceded `#`, because YAML parses that as the start
# of a comment and silently TRUNCATES the name. 🤖 SPARQ agent.
#
# WHY THIS GATE EXISTS. In YAML a `#` that is preceded by whitespace (or that opens
# the value) begins a comment, even in the middle of a plain scalar. So this step:
#
#     - name: Self-test ci-free-disk.sh (set -u safety + existence guards + #462 fold)
#
# parses to the name `Self-test ci-free-disk.sh (set -u safety + existence guards +`
# — the issue reference and the closing paren are eaten. Nothing fails: the step
# still runs, the workflow is still valid YAML, and the only symptom is a mangled
# label in the Actions UI and in every log/annotation that quotes it. That is exactly
# the readability these `Self-test …` / `… — GATING` step names are FOR, and the trap
# is invisible in review, so it recurred repeatedly (#6123 was itself spotted while
# adding a step in #5140). Quoting the name fixes it; this lint is what stops the
# next unquoted one from landing.
#
# RULE (deliberately NARROW, to keep the gate free of false positives). Scan
# .github/workflows/*.yml (+ *.yaml). For each `name:` key whose value is a plain
# scalar — not quoted, not a block scalar, not empty — flag it when:
#
#   (a) the value STARTS with `#`, so the name parses to null outright; or
#   (b) a whitespace-preceded `#` is followed immediately by a DIGIT — the `#1234`
#       issue/PR-reference form that is never a deliberate comment; or
#   (c) a whitespace-preceded `#` leaves the surviving prefix with an UNCLOSED
#       `(` or `[` — structural proof that the name was cut mid-phrase.
#
# A deliberate trailing comment on a balanced name (`name: build  # matrix leg`) is
# NOT flagged: it is legal, intentional, and indistinguishable from a mistake by any
# rule that would also catch it. The fix for a flagged name is always the same —
# wrap the whole name in double quotes, the form this repo already uses in several
# places (e.g. `- name: "Enforce … (C2/C3/C4) — GATING"`).
#
# WHY A HAND-ROLLED SCANNER RATHER THAN PyYAML: a YAML parse cannot see this defect
# at all. By the time the document is loaded the comment is gone and the parser hands
# back the already-truncated string, which is indistinguishable from a name that was
# always short. The offence exists only in the SOURCE line, so the scanner must read
# lines. It is not a general YAML parser: it needs only to find `name:` keys and skip
# block scalars (`run: |`), whose bodies are matched by indentation so a script line
# that happens to start `name:` can never be mistaken for a key. Being stdlib-only is
# then a free bonus — it keeps the lint fast and dependency-order-independent.
#
# EXIT: 0 when no plain `name:` scalar is truncated; 1 with a per-offence report
# showing what was written vs what actually renders.
#
# Usage:
#   check-workflow-name-truncation.py                # scan .github/workflows
#   check-workflow-name-truncation.py --root <dir>   # scan <dir>/.github/workflows
#   check-workflow-name-truncation.py --self-test    # hermetic logic self-test
#
# stdlib-only.

from __future__ import annotations

import argparse
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# A `name:` key, optionally introduced by a list dash, with its inline value.
_NAME_RE = re.compile(r"^(?P<lead>\s*(?:-\s+)?)name:[ \t]*(?P<value>.*)$")
# A block-scalar header: `<key>: |`, `- run: >-`, `script: |2`, with an optional
# trailing comment. Everything indented deeper is literal content, not YAML.
_BLOCK_RE = re.compile(r"^\s*(?:-\s+)?[\w.\-]+:[ \t]*[|>][-+]?\d*[ \t]*(?:#.*)?$")
# The first `#` that YAML would treat as a comment opener inside a plain scalar:
# one preceded by whitespace.
_COMMENT_RE = re.compile(r"\s#")

# Value prefixes that are not plain scalars, so the `#` rule does not apply:
# quotes, block scalars, anchors/aliases and tags.
_NOT_PLAIN = ("\"", "'", "|", ">", "&", "*", "!")


def _indent(line: str) -> int:
    """Number of leading spaces (tabs are invalid YAML indent in these files)."""
    return len(line) - len(line.lstrip(" "))


def _unclosed_bracket(text: str) -> bool:
    """True if `text` opens a `(` or `[` it never closes — i.e. it was cut short."""
    return text.count("(") > text.count(")") or text.count("[") > text.count("]")


def scan_text(text: str) -> list[tuple[int, str, str, str]]:
    """Find truncated plain `name:` scalars.

    Returns (line_number, written, rendered, reason) tuples, 1-based.
    """
    offences: list[tuple[int, str, str, str]] = []
    block_indent: int | None = None

    for lineno, line in enumerate(text.splitlines(), start=1):
        # Inside a block scalar: blank lines and anything indented deeper than the
        # header belong to the literal body and are not YAML.
        if block_indent is not None:
            if not line.strip() or _indent(line) > block_indent:
                continue
            block_indent = None
        if _BLOCK_RE.match(line):
            block_indent = _indent(line)
            continue

        m = _NAME_RE.match(line)
        if not m:
            continue
        value = m.group("value").rstrip()
        if not value or value.startswith(_NOT_PLAIN):
            continue

        if value.startswith("#"):
            offences.append(
                (lineno, value, "", "the value starts with `#`, so the name is null")
            )
            continue

        cut = _COMMENT_RE.search(value)
        if not cut:
            continue
        kept = value[: cut.start()].rstrip()
        eaten = value[cut.start() + 1 :]

        if eaten[1:2].isdigit():
            reason = f"`{eaten.split()[0]}` is an issue/PR reference, not a comment"
        elif _unclosed_bracket(kept):
            reason = "the surviving prefix has an unclosed bracket"
        else:
            continue
        offences.append((lineno, value, kept, reason))

    return offences


def scan_workflows(root: Path) -> list[tuple[Path, int, str, str, str]]:
    wf_dir = root / ".github" / "workflows"
    found: list[tuple[Path, int, str, str, str]] = []
    paths = sorted(list(wf_dir.glob("*.yml")) + list(wf_dir.glob("*.yaml")))
    for path in paths:
        for lineno, written, rendered, reason in scan_text(
            path.read_text(encoding="utf-8")
        ):
            found.append((path, lineno, written, rendered, reason))
    return found


_TRUNCATED_ISSUE_REF = """
jobs:
  a:
    steps:
      - name: Self-test foo.sh (guards + #462 fold)
        run: bash foo.sh
"""

_TRUNCATED_AT_START = """
jobs:
  a:
    steps:
      - name: #1135 render the manifest
        run: true
"""

# Balanced brackets, so ONLY the `#<digit>` issue-ref rule can catch this one.
_TRUNCATED_BALANCED_ISSUE_REF = """
jobs:
  a:
    steps:
      - name: Render #1135 from the manifest
        run: true
"""

# The eaten text is prose, not a `#<digit>` ref, so ONLY the unclosed-bracket rule
# can catch this one.
_TRUNCATED_UNCLOSED = """
jobs:
  a:
    steps:
      - name: Drift tripwire (regression guard # see the design record)
        run: true
"""

_QUOTED_OK = """
jobs:
  a:
    steps:
      - name: "Self-test foo.sh (guards + #462 fold)"
        run: bash foo.sh
      - name: 'Render #1135 from the manifest'
        run: true
"""

_HASH_NOT_PRECEDED_BY_SPACE_OK = """
jobs:
  a:
    steps:
      - name: Self-test the ring guard (#3475 negative fixtures)
        run: true
"""

_DELIBERATE_COMMENT_OK = """
jobs:
  a:
    steps:
      - name: build  # only the matrix leg needs this
        run: true
"""

# The heredoc body contains a line that IS syntactically a `name:` key — the only
# shape that can reach the scanner if block-scalar skipping is dropped.
_RUN_BODY_IS_NOT_YAML = """
jobs:
  a:
    steps:
      - name: "Emit a manifest"
        run: |
          cat > manifest.yml <<'EOF'
          name: widget (size + #462 variant)
          EOF
      - name: after the block
        run: true
"""

_BLOCK_THEN_TRUNCATION = """
jobs:
  a:
    steps:
      - name: "Emit"
        run: |
          name: inner (ignored + #1 too)
      - name: Self-test the sweeper (isolation — #3760)
        run: true
"""

_MIXED = _TRUNCATED_ISSUE_REF + _QUOTED_OK + _TRUNCATED_UNCLOSED


def self_test() -> int:
    cases = [
        ("truncated: eaten `#462` issue ref", _TRUNCATED_ISSUE_REF, 1),
        ("truncated: value starts with `#`", _TRUNCATED_AT_START, 1),
        (
            "truncated: balanced name, issue-ref rule is the only catcher",
            _TRUNCATED_BALANCED_ISSUE_REF,
            1,
        ),
        ("truncated: unclosed bracket left behind", _TRUNCATED_UNCLOSED, 1),
        ("ok: double- and single-quoted names", _QUOTED_OK, 0),
        ("ok: `(#3475` has no space before the `#`", _HASH_NOT_PRECEDED_BY_SPACE_OK, 0),
        ("ok: deliberate trailing comment, balanced name", _DELIBERATE_COMMENT_OK, 0),
        ("ok: `name:` inside a run: block body is not YAML", _RUN_BODY_IS_NOT_YAML, 0),
        ("truncated: found after a block scalar ends", _BLOCK_THEN_TRUNCATION, 1),
        ("mixed (2 truncated, 2 clean)", _MIXED, 2),
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

    # The rendered name must be the truncation a real YAML parser produces — that
    # equivalence is the whole claim, so pin it rather than trusting the regex.
    rendered = scan_text(_TRUNCATED_ISSUE_REF)[0][2]
    expected_rendered = "Self-test foo.sh (guards +"
    if rendered != expected_rendered:
        print(f"  [FAIL] rendered name: {rendered!r} (want {expected_rendered!r})")
        failures += 1
    else:
        print(f"  [PASS] rendered name matches the YAML truncation: {rendered!r}")

    if failures:
        print(f"\nself-test: {failures} case(s) FAILED")
        return 1
    print("\nself-test: all cases PASS")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Fail if a plain (unquoted) workflow `name:` is truncated by an "
        "inline YAML comment (#6123)."
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

    root_path = Path(args.root)
    offences = scan_workflows(root_path)
    if not offences:
        print(
            "workflow name-truncation gate: PASS — no plain `name:` scalar is cut "
            "short by an inline YAML comment."
        )
        return 0

    print("workflow name-truncation gate: FAIL\n")
    print(
        "These `name:` values are PLAIN scalars containing a whitespace-preceded `#`,\n"
        "so YAML treats the rest as a comment and the Actions UI shows a truncated\n"
        "name (#6123). Wrap each whole name in double quotes:\n"
    )
    for path, lineno, written, rendered, reason in offences:
        rel = path.relative_to(root_path) if path.is_relative_to(root_path) else path
        print(f"    {rel}:{lineno} — {reason}")
        print(f"        written  : {written}")
        print(f"        renders  : {rendered}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
