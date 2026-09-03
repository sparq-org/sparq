#!/usr/bin/env python3
"""Emit deterministic sensitivity or scenario configurations as TSV."""

from __future__ import annotations

import argparse
import random
from dataclasses import dataclass


@dataclass(frozen=True)
class Cell:
    label: str
    domain: str
    pods: int
    documents: int
    triples: int
    depth: int
    coverage: int
    public: int
    private: int
    shared: int
    principal: str
    queries: str


def sensitivity_cells(domains: list[str]) -> list[Cell]:
    cells: list[Cell] = []
    for domain in domains:
        for documents in (1, 8, 32, 128, 512):
            cells.append(
                Cell(
                    f"documents-{documents}",
                    domain,
                    16,
                    documents,
                    8,
                    3,
                    250,
                    0,
                    1000,
                    0,
                    "owner",
                    "q1-point,q8-graph-scan",
                )
            )
        for triples in (1, 8, 32, 128):
            cells.append(
                Cell(
                    f"triples-{triples}",
                    domain,
                    16,
                    32,
                    triples,
                    3,
                    250,
                    0,
                    1000,
                    0,
                    "owner",
                    "q1-point,q8-graph-scan",
                )
            )
        for coverage in (0, 100, 250, 1000):
            for depth in (1, 3, 6):
                cells.append(
                    Cell(
                        f"placement-{coverage}-{depth}",
                        domain,
                        16,
                        16,
                        8,
                        depth,
                        coverage,
                        0,
                        1000,
                        0,
                        "owner",
                        "q1-point,q8-graph-scan",
                    )
                )
        for public in (10, 100, 500, 1000):
            cells.append(
                Cell(
                    f"visibility-{public}",
                    domain,
                    1,
                    512,
                    8,
                    3,
                    250,
                    public,
                    1000 - public,
                    0,
                    "anonymous",
                    "q1-point,q8-graph-scan",
                )
            )
    return cells


def scenario_cells() -> list[Cell]:
    return [
        Cell("social-count-small", "social", 100, 103, 22, 3, 250, 100, 700, 200, "owner", "q1-point"),
        Cell("social-count-anchor", "social", 1531, 103, 22, 3, 250, 100, 700, 200, "owner", "q1-point"),
        Cell("health-compact-small", "health", 64, 1, 32, 1, 1000, 0, 1000, 0, "owner", "q1-point"),
        Cell("health-compact-anchor", "health", 256, 1, 128, 1, 1000, 0, 1000, 0, "owner", "q1-point"),
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--campaign", choices=("sensitivity", "scenarios"), required=True)
    parser.add_argument("--lanes", default="materialized http")
    parser.add_argument("--domains", default="social health")
    parser.add_argument("--corpus-seed", required=True, type=int)
    parser.add_argument("--order-seed", required=True, type=int)
    args = parser.parse_args()

    lanes = args.lanes.split()
    domains = args.domains.split()
    if not lanes or any(lane not in {"materialized", "http"} for lane in lanes):
        parser.error("--lanes must contain materialized and/or http")
    if not domains or any(domain not in {"social", "health"} for domain in domains):
        parser.error("--domains must contain social and/or health")
    if len(lanes) != len(set(lanes)) or len(domains) != len(set(domains)):
        parser.error("lanes and domains must not contain duplicates")
    base = (
        sensitivity_cells(domains)
        if args.campaign == "sensitivity"
        else [cell for cell in scenario_cells() if cell.domain in domains]
    )
    scheduled = [(lane, cell) for lane in lanes for cell in base]
    random.Random(args.order_seed).shuffle(scheduled)
    for index, (lane, cell) in enumerate(scheduled):
        fields = (
            index,
            cell.label,
            lane,
            cell.domain,
            cell.pods,
            cell.documents,
            cell.triples,
            cell.depth,
            cell.coverage,
            cell.public,
            cell.private,
            cell.shared,
            cell.principal,
            args.corpus_seed,
            args.order_seed,
            cell.queries,
        )
        print("\t".join(map(str, fields)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
