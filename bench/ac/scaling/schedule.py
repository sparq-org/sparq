#!/usr/bin/env python3
"""Emit a deterministic pseudorandom benchmark configuration schedule as TSV."""

from __future__ import annotations

import argparse
import itertools
import random


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lanes", required=True, help="space-separated lane names")
    parser.add_argument("--domains", required=True, help="space-separated domains")
    parser.add_argument("--pods", required=True, help="space-separated positive counts")
    parser.add_argument("--seed", required=True, type=int)
    args = parser.parse_args()

    lanes = args.lanes.split()
    domains = args.domains.split()
    try:
        pods = [int(value) for value in args.pods.split()]
    except ValueError as error:
        parser.error(f"--pods contains a non-integer: {error}")
    if not lanes or any(lane not in {"materialized", "http"} for lane in lanes):
        parser.error("--lanes must contain materialized and/or http")
    if not domains or any(domain not in {"social", "health"} for domain in domains):
        parser.error("--domains must contain social and/or health")
    if not pods or any(value <= 0 for value in pods):
        parser.error("Pod counts must be positive")
    if len(lanes) != len(set(lanes)) or len(domains) != len(set(domains)):
        parser.error("lanes and domains must not contain duplicates")
    if len(pods) != len(set(pods)):
        parser.error("Pod counts must not contain duplicates")

    configurations = list(itertools.product(lanes, domains, pods))
    random.Random(args.seed).shuffle(configurations)
    for index, (lane, domain, pod_count) in enumerate(configurations):
        print(f"{index}\t{lane}\t{domain}\t{pod_count}\t{args.seed}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
