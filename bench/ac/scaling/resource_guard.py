#!/usr/bin/env python3
"""Refuse a new benchmark cell after observed RSS reaches the memory budget."""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


class GuardError(RuntimeError):
    """A result file or host-memory invariant cannot be evaluated safely."""


@dataclass(frozen=True)
class GuardReport:
    physical_bytes: int
    limit_bytes: int
    peak_resident_bytes: int | None
    peak_source: Path | None

    @property
    def allowed(self) -> bool:
        return self.peak_resident_bytes is None or self.peak_resident_bytes < self.limit_bytes


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path, help="result directory to inspect recursively")
    parser.add_argument("--fraction", type=float, default=0.70)
    parser.add_argument(
        "--physical-memory-bytes",
        type=int,
        help="override host memory detection (intended for deterministic tests)",
    )
    parser.add_argument("--quiet", action="store_true")
    return parser.parse_args()


def physical_memory_bytes() -> int:
    try:
        page_size = os.sysconf("SC_PAGE_SIZE")
        page_count = os.sysconf("SC_PHYS_PAGES")
    except (OSError, ValueError) as error:
        raise GuardError(f"cannot determine physical memory: {error}") from error
    total = page_size * page_count
    if total <= 0:
        raise GuardError(f"physical memory is not positive: {total}")
    return total


def result_files(results: Path) -> tuple[Path, ...]:
    if not results.exists():
        return ()
    if not results.is_dir():
        raise GuardError(f"result path is not a directory: {results}")
    files = set(results.rglob("*.jsonl"))
    files.update(results.rglob("*.jsonl.partial"))
    return tuple(sorted(files))


def peak_resident_bytes(paths: Iterable[Path]) -> tuple[int | None, Path | None]:
    maximum: int | None = None
    source: Path | None = None
    for path in paths:
        with path.open(encoding="utf-8") as stream:
            for line_number, line in enumerate(stream, 1):
                if not line.strip():
                    continue
                try:
                    record = json.loads(line)
                except json.JSONDecodeError as error:
                    raise GuardError(f"{path}:{line_number}: invalid JSON: {error}") from error
                if record.get("record_type") != "construction":
                    continue
                value = record.get("peak_resident_bytes")
                if value is None:
                    continue
                if isinstance(value, bool) or not isinstance(value, int) or value < 0:
                    raise GuardError(
                        f"{path}:{line_number}: peak_resident_bytes must be a non-negative integer"
                    )
                if maximum is None or value > maximum:
                    maximum = value
                    source = path
    return maximum, source


def evaluate(results: Path, fraction: float, physical_bytes: int | None = None) -> GuardReport:
    if not 0.0 < fraction < 1.0:
        raise GuardError(f"memory fraction must be between zero and one: {fraction}")
    total = physical_memory_bytes() if physical_bytes is None else physical_bytes
    if total <= 0:
        raise GuardError(f"physical memory is not positive: {total}")
    peak, source = peak_resident_bytes(result_files(results))
    return GuardReport(total, int(total * fraction), peak, source)


def format_bytes(value: int) -> str:
    return f"{value / (1024**3):.2f} GiB"


def main() -> int:
    args = arguments()
    try:
        report = evaluate(args.results, args.fraction, args.physical_memory_bytes)
    except GuardError as error:
        print(f"resource guard cannot establish safety: {error}", file=sys.stderr)
        return 2
    if not report.allowed:
        assert report.peak_resident_bytes is not None
        print(
            "resource guard: observed peak RSS "
            f"{format_bytes(report.peak_resident_bytes)} reached the "
            f"{format_bytes(report.limit_bytes)} limit in {report.peak_source}",
            file=sys.stderr,
        )
        return 1
    if not args.quiet:
        observed = (
            "no construction sample"
            if report.peak_resident_bytes is None
            else f"peak {format_bytes(report.peak_resident_bytes)}"
        )
        print(f"resource guard: {observed}; limit {format_bytes(report.limit_bytes)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
