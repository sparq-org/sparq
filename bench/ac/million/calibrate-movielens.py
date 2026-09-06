#!/usr/bin/env python3
"""[GPT-6] Reproduce anonymous retained-record aggregates from the public archive.

The original archive is not redistributed. Download MovieLens 1M from its publisher,
read its research-use license, then pass the local ZIP path. Only counts are emitted;
movie choices, ratings, timestamps and user demographics are not copied.
"""

import argparse
import collections
import hashlib
import json
from pathlib import Path
import zipfile


ARCHIVE_SHA256 = "a6898adb50b9ca05aa231689da44c217cb524e7ebd39d264c56e2832f2c54e20"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--cdf", type=Path, required=True)
    parser.add_argument("--statistics", type=Path, required=True)
    args = parser.parse_args()
    archive = args.archive.read_bytes()
    digest = hashlib.sha256(archive).hexdigest()
    if digest != ARCHIVE_SHA256:
        parser.error("archive SHA-256 differs from the pinned calibration input")
    with zipfile.ZipFile(args.archive) as source:
        raw = source.read("ml-1m/ratings.dat")
    counts = collections.Counter(int(row.split(b"::", 1)[0]) for row in raw.splitlines())
    values = sorted(counts.values())
    cdf, cumulative = [], 0
    for count, frequency in sorted(collections.Counter(values).items()):
        cumulative += frequency
        cdf.append([count, cumulative])
    rendered = json.dumps(cdf, separators=(",", ":")) + "\n"
    args.cdf.write_text(rendered)
    statistics = {
        "archive_sha256": digest,
        "ratings_dat_sha256": hashlib.sha256(raw).hexdigest(),
        "cdf_sha256": hashlib.sha256(rendered.encode()).hexdigest(),
        "source_users": len(values),
        "source_records": sum(values),
        "min": min(values),
        "p50": values[int(0.50 * (len(values) - 1))],
        "p95": values[int(0.95 * (len(values) - 1))],
        "p99": values[int(0.99 * (len(values) - 1))],
        "max": max(values),
        "cdf_bins": len(cdf),
        "quantile_method": "order statistic floor(p*(n-1)); zero-based",
    }
    args.statistics.write_text(json.dumps(statistics, indent=2) + "\n")


if __name__ == "__main__":
    main()
