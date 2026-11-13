#!/usr/bin/env python3
"""Validate complete benchmark outcomes and write a requirement/query results table."""

import argparse
from pathlib import Path
import re


QUERY = re.compile(r"query-r(\d{2})(?:-\d+)*\Z")
PUBLISHED = "https://github.com/OpenLinkSoftware/GeoSPARQLBenchmark/blob/"


def render(outcomes, resources, revision):
    expected = {p.stem for p in (resources / "gsb_queries").glob("query-r*.rq")}
    if len(expected) != 206:
        raise ValueError(f"incomplete benchmark inventory: {len(expected)} queries")
    statuses = {}
    for line in outcomes.splitlines():
        query, sep, status = line.partition("\t")
        match = QUERY.fullmatch(query)
        if not sep or status not in ("passed", "failed") or not match:
            raise ValueError(f"invalid outcome: {line!r}")
        if not 1 <= int(match[1]) <= 30 or int(match[1]) == 17:
            raise ValueError(f"invalid requirement: {query}")
        if query in statuses:
            raise ValueError(f"duplicate outcome: {query}")
        statuses[query] = status
    if statuses.keys() != expected:
        raise ValueError("outcomes do not cover exactly the benchmark query inventory")
    rows = ["# GeoSPARQL Compliance Benchmark results", "",
            f"Suite revision: `{revision}`. RDFS, accessors and query rewrite enabled.", "",
            "Test counts only; this community benchmark is not OGC certification.", "",
            "| Requirement | Queries (pass/fail) | Passed | Failed |",
            "| --- | --- | ---: | ---: |"]
    for requirement in range(1, 31):
        queries = sorted(q for q in statuses if int(QUERY.fullmatch(q)[1]) == requirement)
        passed = sum(statuses[q] == "passed" for q in queries)
        links = ", ".join(
            f"[{q}]({PUBLISHED}{revision}/src/main/resources/gsb_queries/{q}.rq) {statuses[q]}"
            for q in queries
        ) or "No query (unscored)"
        rows.append(f"| R{requirement:02} | {links} | {passed} | {len(queries) - passed} |")
    passed = sum(s == "passed" for s in statuses.values())
    rows.append(f"| Total | {len(statuses)} queries | {passed} | {len(statuses) - passed} |")
    return "\n".join(rows) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("outcomes", type=Path)
    parser.add_argument("resources", type=Path)
    parser.add_argument("revision")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        report = render(args.outcomes.read_text(), args.resources, args.revision)
    except ValueError as error:
        parser.exit(1, f"benchmark-report: {error}\n")
    args.output.write_text(report)


if __name__ == "__main__":
    main()
