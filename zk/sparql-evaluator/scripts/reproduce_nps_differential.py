#!/usr/bin/env python3
# [GPT-6] Opt-in observations over original synthetic REC-derived fixtures.
"""Reproduce the bounded NPS disagreement; never install dependencies or edit goldens."""

from __future__ import annotations

import argparse
import importlib
import importlib.metadata
import json
from pathlib import Path


VERSIONS = {"pyoxigraph": "0.5.11", "rdflib": "7.6.0"}
CASE_IDS = (
    "path-negated-duplicate-forward",
    "path-negated-duplicate-reverse",
    "path-negated-both-orientations",
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", required=True, choices=tuple(VERSIONS))
    args = parser.parse_args()
    try:
        version = importlib.metadata.version(args.engine)
        engine = importlib.import_module(args.engine)
    except (ImportError, importlib.metadata.PackageNotFoundError) as error:
        parser.error(f"{args.engine} {VERSIONS[args.engine]} must already be installed: {error}")
    if version != VERSIONS[args.engine]:
        parser.error(f"expected {args.engine} {VERSIONS[args.engine]}, found {version}")

    corpus_path = Path(__file__).resolve().parents[1] / "fixtures/conformance/cases.json"
    corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
    cases = {case["id"]: case for case in corpus["cases"]}
    observations = []
    for case_id in CASE_IDS:
        case = cases[case_id]
        expected = case["expected"]["result"]["Select"]
        assert expected["variables"] == ["o"] and expected["order"] == "Bag"
        observation = {
            "case_id": case_id,
            "query": case["query"],
            "dataset_ntriples": case["dataset"],
            "spec": case["spec"],
            "rec_derived_expected_rows": expected["rows"],
        }
        try:
            if args.engine == "pyoxigraph":
                graph = engine.Store()
                graph.load(input=case["dataset"].encode(), format=engine.RdfFormat.N_TRIPLES)
                rows = [
                    [None if row["o"] is None else str(row["o"])]
                    for row in graph.query(case["query"])
                ]
            else:
                graph = engine.Graph()
                graph.parse(data=case["dataset"], format="nt")
                rows = [
                    [None if row[0] is None else row[0].n3()]
                    for row in graph.query(case["query"])
                ]
            observation["actual_rows"] = sorted(rows, key=json.dumps)
            observation["actual_cardinality"] = len(rows)
        except Exception as error:
            # An implementation error is an observation, never an empty result.
            observation["error"] = {"type": type(error).__name__, "message": str(error)}
        observations.append(observation)
    print(json.dumps({
        "schema_version": 1,
        "engine": args.engine,
        "version": version,
        "fixture_source": "fixtures/conformance/cases.json",
        "evidence_kind": "bounded_external_implementation_observation",
        "is_conformance_oracle": False,
        "is_guest_or_receipt_evidence": False,
        "observations": observations,
    }, indent=2))


if __name__ == "__main__":
    main()
