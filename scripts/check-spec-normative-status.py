#!/usr/bin/env python3
# Fail closed when a deferred normative core has no tracking bead
# (bead sq-nj0ts).

"""Require tracking beads for deferred normative-core sections in Typst specs."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import sys


BEAD_RE = re.compile(r"\bsq-[a-z0-9.]+\b")
HEADING_RE = re.compile(r"^(?P<marks>=+)\s+(?P<title>.*)$")
DEFERRED_HEADING_RE = re.compile(r"\bdeferred\s+normative\s+cores?\b", re.IGNORECASE)
NOT_DEFINED_RE = re.compile(r"\bnot\s+defined\s+in\s+this\s+document\b", re.IGNORECASE)


@dataclass(frozen=True)
class Section:
    line: int
    title: str
    direct_text: str
    text: str


def sections(source: str) -> list[Section]:
    """Split Typst source into heading sections, including nested subsections."""
    lines = source.splitlines()
    headings = [
        (index, match)
        for index, line in enumerate(lines)
        if (match := HEADING_RE.match(line)) is not None
    ]
    result: list[Section] = []
    for position, (start, match) in enumerate(headings):
        level = len(match.group("marks"))
        end = len(lines)
        for next_start, next_match in headings[position + 1 :]:
            if len(next_match.group("marks")) <= level:
                end = next_start
                break
        result.append(
            Section(
                line=start + 1,
                title=match.group("title"),
                direct_text="\n".join(
                    lines[
                        start : headings[position + 1][0]
                        if position + 1 < len(headings)
                        else len(lines)
                    ]
                ),
                text="\n".join(lines[start:end]),
            )
        )
    return result


def missing_tracking_beads(source: str) -> list[Section]:
    """Return deferred-core sections that contain no in-repo bead identifier."""
    return [
        section
        for section in sections(source)
        if (
            DEFERRED_HEADING_RE.search(section.title)
            or NOT_DEFINED_RE.search(section.direct_text)
        )
        and not BEAD_RE.search(section.text)
    ]


def check_file(path: Path) -> list[str]:
    try:
        source = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        return [f"{path}: could not read Typst source: {error}"]

    return [
        f"{path}:{section.line}: deferred normative core "
        f"section {section.title!r} has no tracking bead (expected sq-...)"
        for section in missing_tracking_beads(source)
    ]


def self_test() -> int:
    untracked = """= Test spec
== Deferred normative core
The wire encoding is not defined in this document.
"""
    tracked = untracked + "Tracked by sq-example.1.\n"
    tracked_after_subsection = untracked + """=== Encoding details
Tracked by sq-example.2.
"""
    ordinary = """= Test spec
== Scope
This section is fully normative.
"""

    assert len(missing_tracking_beads(untracked)) == 1
    assert missing_tracking_beads(tracked) == []
    assert missing_tracking_beads(tracked_after_subsection) == []
    assert missing_tracking_beads(ordinary) == []
    print("check-spec-normative-status: self-test passed")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", type=Path, help="Typst spec sources")
    parser.add_argument("--self-test", action="store_true", help="run hermetic checks")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()
    if not args.paths:
        parser.error("provide at least one Typst spec source")

    findings = [finding for path in args.paths for finding in check_file(path)]
    if findings:
        for finding in findings:
            print(finding, file=sys.stderr)
        return 1
    print(f"check-spec-normative-status: checked {len(args.paths)} spec(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
