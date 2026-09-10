#!/usr/bin/env python3
# [OPUS-4.8] CI lint (bead sq-ur7o): every SHA-pinned `taiki-e/install-action`
# step MUST carry an explicit `with: tool:` input.
# Authored by Opus 4.8 (Fable unavailable; flag for re-review when Fable returns).
#
# WHY THIS GATE EXISTS — the coverage-gate regression (root-caused 2026-06-18):
# taiki-e/install-action selects WHICH tool to install from its git-ref TAG, i.e.
# `taiki-e/install-action@<tool>` (e.g. `@cargo-llvm-cov`). Our code-scanning=0
# SHA-pin policy rewrites every `uses:` to a 40-hex commit SHA, which DROPS that
# `@<tool>` selector. install-action then logs "no tool specified ... dependabot bug
# where @<tool_name> tags are replaced by @<version> tags" and installs NOTHING.
# The downstream `cargo <tool>` then ENOENTs ("error: no such command: llvm-cov"),
# but if the job swallowed that (as the per-commit coverage job did before sq-039g
# made measure-failures fatal) the gate becomes VACUOUS — measuring nothing, passing
# silently. The fix is always the same: add an explicit `with: tool: <name>` so the
# SHA-pinned action knows what to install. This lint makes the OMISSION itself fail
# CI, so the whole class of "SHA pin silently disabled an install-action gate" can
# never recur.
#
# RULE: scan .github/workflows/*.yml (+ *.yaml). For every step whose `uses:` value
# is `taiki-e/install-action@<ref>` where <ref> is a 40-char hex commit SHA (i.e. a
# SHA pin, not a `@v2`/`@cargo-llvm-cov` tag), FAIL unless that SAME step has a
# `with:` mapping containing a `tool:` key. A step pinned to a *tag* (`@v2.81.10`,
# `@cargo-llvm-cov`) is NOT flagged: a tag ref can still carry the selector, so the
# omission is only dangerous under a SHA pin — exactly the case the policy forces.
#
# WHY A HAND-ROLLED STDLIB PARSER (no PyYAML): this runs in the docs-quality
# `ci-scripts` job, which — like coverage-gate.py's self-test — installs no Python
# deps. GitHub-Actions workflow files are written in a tightly-constrained block
# style (2-space indent, `- name:` step lists, `uses:`/`with:` siblings), so a
# small indentation-aware step splitter is sufficient and keeps the gate
# dependency-free and fast. It deliberately does NOT try to be a general YAML
# parser; it only needs to (a) find each step block and (b) within it read the
# `uses:` line and detect a sibling `with:` mapping that has a `tool:` key.
#
# EXIT: 0 when every SHA-pinned install-action step has `with: tool:` (or there are
# none); 1 with a per-offence message otherwise.
#
# Usage:
#   check-install-action-tool.py                  # scan .github/workflows
#   check-install-action-tool.py --root <dir>     # scan <dir>/.github/workflows
#   check-install-action-tool.py --self-test      # hermetic logic self-test
#
# stdlib-only.

from __future__ import annotations

import argparse
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

ACTION = "taiki-e/install-action"
# A 40-char lowercase hex commit SHA — the form our SHA-pin policy enforces. A `@v2`
# or `@cargo-llvm-cov` tag never matches this, so tag-pinned steps are left alone.
_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
# `uses: <action>@<ref>` with an optional trailing `# comment`. The ref runs up to
# whitespace or a `#`.
_USES_RE = re.compile(r"^uses:\s*(\S+?)@(\S+?)(?:\s|#|$)")


def _indent(line: str) -> int:
    """Number of leading spaces (tabs are invalid YAML indent in these files)."""
    return len(line) - len(line.lstrip(" "))


def split_steps(text: str) -> list[list[str]]:
    """Split a workflow file into per-step line blocks.

    A step is a list item (`- ` after some indent). We collect, for each
    `- `-introduced block at a given indent, all subsequent lines indented deeper
    than the dash until the next sibling dash or a dedent — that contiguous run is
    one step's body. Blank/comment lines inside a block are retained.

    The result is a list of blocks; each block is the list of physical lines (no
    trailing newline) belonging to one `- ...` item. Non-step content (top-level
    keys, job headers) never starts a block and is ignored by callers, which only
    act on blocks that contain a `taiki-e/install-action` `uses:`.
    """
    lines = text.splitlines()
    blocks: list[list[str]] = []
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        stripped = line.lstrip(" ")
        if stripped.startswith("- "):
            dash_indent = _indent(line)
            block = [line]
            i += 1
            # Absorb deeper-indented continuation lines (and blanks/comments) until a
            # sibling list item at the same indent or a dedent to <= dash_indent on a
            # non-blank, non-comment line.
            while i < n:
                nxt = lines[i]
                if not nxt.strip() or nxt.lstrip(" ").startswith("#"):
                    block.append(nxt)
                    i += 1
                    continue
                ind = _indent(nxt)
                nstripped = nxt.lstrip(" ")
                if ind == dash_indent and nstripped.startswith("- "):
                    # Next sibling step.
                    break
                if ind <= dash_indent:
                    # Dedent below the dash → end of this step's body.
                    break
                block.append(nxt)
                i += 1
            blocks.append(block)
        else:
            i += 1
    return blocks


def _step_uses(block: list[str]) -> tuple[str, str] | None:
    """Return (action, ref) for the step's `uses:` line, else None. The first line of
    a `- uses: ...` block has the dash; strip a leading `- ` before matching."""
    for raw in block:
        s = raw.lstrip(" ")
        if s.startswith("- "):
            s = s[2:]
        m = _USES_RE.match(s)
        if m:
            return m.group(1), m.group(2)
    return None


def _has_with_tool(block: list[str]) -> bool:
    """True if the step block contains a `with:` mapping carrying a `tool:` key.

    We locate the `with:` line (at the step-body indent), then read the lines
    indented DEEPER than `with:` for a `tool:` key. This correctly ignores a `tool:`
    that might appear under some other mapping and a `with:` that has no `tool:`."""
    with_indent: int | None = None
    for raw in block:
        if not raw.strip() or raw.lstrip(" ").startswith("#"):
            continue
        # Normalise: a key introduced on the dash line itself logically begins after
        # the "- "; account for that in both the indent and the key text.
        ind = _indent(raw)
        key_part = raw.lstrip(" ")
        if key_part.startswith("- "):
            ind += 2
            key_part = key_part[2:]
        if with_indent is None:
            if key_part.startswith("with:"):
                with_indent = ind
            continue
        # Inside the `with:` mapping while indented deeper than `with:`.
        if ind <= with_indent:
            # Dedented out of the `with:` block; this line could itself open another
            # `with:` (not idiomatic, but handle it).
            with_indent = ind if key_part.startswith("with:") else None
            continue
        if key_part.startswith("tool:"):
            return True
    return False


def scan_text(text: str) -> list[str]:
    """Return a list of human-readable offence descriptions for one workflow file's
    text: each SHA-pinned `taiki-e/install-action` step lacking `with: tool:`."""
    offences: list[str] = []
    for block in split_steps(text):
        uses = _step_uses(block)
        if uses is None:
            continue
        action, ref = uses
        if action != ACTION:
            continue
        if not _SHA_RE.match(ref):
            # Tag-pinned (`@v2`/`@cargo-llvm-cov`): the selector can still be present,
            # so the omission is not dangerous and is out of scope for this gate.
            continue
        if not _has_with_tool(block):
            head = next((ln.strip() for ln in block if ln.strip()), "<empty step>")
            offences.append(head)
    return offences


def scan_workflows(root: Path) -> list[tuple[Path, str]]:
    """Scan every workflow file under `root`/.github/workflows. Returns a list of
    (file, offending `uses:` line) pairs."""
    wf_dir = root / ".github" / "workflows"
    results: list[tuple[Path, str]] = []
    if not wf_dir.is_dir():
        return results
    for path in sorted(wf_dir.iterdir()):
        if path.suffix not in (".yml", ".yaml"):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for offence in scan_text(text):
            results.append((path, offence))
    return results


# --------------------------------------------------------------------------- tests
_GOOD = """\
jobs:
  cov:
    steps:
      - uses: actions/checkout@deadbeefdeadbeefdeadbeefdeadbeefdeadbeef # v6
      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # cargo-llvm-cov
        with:
          tool: cargo-llvm-cov
      - name: do
        run: cargo llvm-cov
"""

_BAD_NO_WITH = """\
jobs:
  cov:
    steps:
      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # cargo-llvm-cov
      - name: do
        run: cargo llvm-cov
"""

_BAD_WITH_NO_TOOL = """\
jobs:
  cov:
    steps:
      - uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # nextest
        with:
          checksum: "true"
"""

# A `with:` carrying `tool:` among other keys still passes.
_GOOD_EXTRA_WITH = """\
jobs:
  cov:
    steps:
      - uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # nextest
        with:
          tool: nextest
          checksum: "true"
        continue-on-error: true
"""

# Tag-pinned (NOT a SHA): out of scope — the @<tool> selector can still be present.
_TAG_PINNED_OK = """\
jobs:
  cov:
    steps:
      - uses: taiki-e/install-action@cargo-llvm-cov
      - uses: taiki-e/install-action@v2.81.10
"""

# A different action with a `with:` but no `tool:` must NOT be flagged.
_OTHER_ACTION_OK = """\
jobs:
  cov:
    steps:
      - uses: actions/setup-node@1234567890123456789012345678901234567890 # v6
        with:
          node-version: 22
"""

# Two SHA-pinned install-action steps, only the second is bad.
_MIXED = """\
jobs:
  a:
    steps:
      - uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # nextest
        with:
          tool: nextest
  b:
    steps:
      - uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # cargo-deny
"""

# The `with: tool:` step is followed by a sibling step that has no `with:` at all —
# the splitter must not let the first step's `with:` "leak" into the second.
_NO_LEAK = """\
jobs:
  a:
    steps:
      - uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # nextest
        with:
          tool: nextest
      - uses: taiki-e/install-action@7a79fe8c3a13344501c80d99cae481c1c9085912 # cargo-deny
        continue-on-error: true
"""


def self_test() -> int:
    cases = [
        ("good (with: tool:)", _GOOD, 0),
        ("bad (no with:)", _BAD_NO_WITH, 1),
        ("bad (with: but no tool:)", _BAD_WITH_NO_TOOL, 1),
        ("good (tool: among other with-keys)", _GOOD_EXTRA_WITH, 0),
        ("ok (tag-pinned, out of scope)", _TAG_PINNED_OK, 0),
        ("ok (other action, no tool:)", _OTHER_ACTION_OK, 0),
        ("mixed (1 good, 1 bad)", _MIXED, 1),
        ("no with: leak across sibling steps", _NO_LEAK, 1),
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
        description="Fail if any SHA-pinned taiki-e/install-action step lacks "
        "`with: tool:` (bead sq-ur7o)."
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
            "install-action tool-selector gate: PASS — every SHA-pinned "
            "taiki-e/install-action step has an explicit `with: tool:`."
        )
        return 0

    print("install-action tool-selector gate: FAIL\n")
    print(
        "These SHA-pinned `taiki-e/install-action` steps have NO `with: tool:` input.\n"
        "Under our SHA-pin policy the `@<tool>` selector is dropped, so install-action\n"
        "installs NOTHING and the downstream `cargo <tool>` ENOENTs — silently turning\n"
        "the gate VACUOUS (bead sq-ur7o). Add `with:` then `tool: <name>` to each:\n"
    )
    root_path = Path(args.root)
    for path, offence in offences:
        rel = (
            path.relative_to(root_path) if path.is_relative_to(root_path) else path
        )
        print(f"    - {rel}: {offence}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
