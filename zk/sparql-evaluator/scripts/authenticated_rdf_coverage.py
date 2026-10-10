#!/usr/bin/env python3
"""Build the authenticated-RDF (V5) coverage manifest from executor records.

Usage:
  authenticated_rdf_coverage.py SWEEP_JSON COVERAGE_JSONL BLANK_FREE_JSONL > coverage-authenticated-rdf.json

COVERAGE_JSONL is `paper_sweep --set sweep --suite eddsa --mode hidden --n 1 --size 32
--execute`; BLANK_FREE_JSONL is the same command with `--blank-free`, run for the
cases whose admission depends on blank nodes in the credential data. Each record is
one execution of the accepted V5 guest image in the RISC Zero executor: an admitted
case produced a journal, a rejected case aborted in the guest with the recorded
admission error. Executor runs are not proofs; receipt evidence is recorded
separately. Performance figures stay in the executor records, not in the manifest.
"""

import json
import sys

SPARQL = "https://www.w3.org/TR/sparql11-query/"
SPEC = {
    "bgp": SPARQL + "#BasicGraphPatterns",
    "optional": SPARQL + "#optionals",
    "union": SPARQL + "#alternatives",
    "minus": SPARQL + "#neg-minus",
    "filter-exists": SPARQL + "#neg-pattern",
    "filter-not-exists": SPARQL + "#neg-pattern",
    "bind": SPARQL + "#bind",
    "values": SPARQL + "#inline-data",
    "subquery": SPARQL + "#subqueries",
    "comparison": SPARQL + "#OperatorMapping",
    "fn": SPARQL + "#SparqlOps",
    "aggregate": SPARQL + "#aggregates",
    "modifier": SPARQL + "#solutionModifiers",
    "path": SPARQL + "#propertypaths",
    "form": SPARQL + "#QueryForms",
    "data": "https://www.w3.org/TR/rdf11-concepts/#section-blank-nodes",
    "probe-graph": SPARQL + "#queryDataset",
    "probe-from": SPARQL + "#specifyingDataset",
    "probe-service": "https://www.w3.org/TR/sparql11-federated-query/",
    "probe-query-blank-node": SPARQL + "#QSynBlankNodes",
    "probe-bnode-fn": SPARQL + "#func-bnode",
    "probe-now": SPARQL + "#func-now",
    "probe-rand": SPARQL + "#func-rand",
    "probe-nested-exists": SPARQL + "#neg-pattern",
    "probe-triple-term": "https://www.w3.org/TR/sparql12-query/#rdf-triple-terms",
    "probe-custom-function": SPARQL + "#extensionFunctions",
}

NOTES = {
    "probe-graph": "Admitted and evaluated as specified: the credentials form the default graph and the dataset has no named graphs, so GRAPH matches nothing.",
    "probe-from": "Admitted and evaluated as specified: FROM names a graph the dataset does not hold, so the default graph is empty.",
    "probe-query-blank-node": "A blank node in a query pattern is an existential variable; this is an ordinary basic graph pattern.",
    "blank-node-data": "Credential data containing blank nodes is supported; blank-node labels are renamed per credential so credentials never share one.",
}


def spec(case):
    return SPEC.get(case["id"]) or SPEC.get(case["id"].split("-")[0]) or SPEC.get(case["class"])


def load(path):
    with open(path) as handle:
        return {record["id"]: record for record in map(json.loads, handle)}


def status(record):
    if record["admitted"]:
        return "guest_executed"
    return "rejected_in_guest" if record.get("guest_aborted") else "rejected_before_guest"


def main():
    sweep_path, coverage_path, blank_free_path = sys.argv[1:4]
    with open(sweep_path) as handle:
        cases = json.load(handle)["cases"]
    data, blank_free = load(coverage_path), load(blank_free_path)
    features = []
    for case in cases:
        record = data[case["id"]]
        feature = {
            "id": case["id"],
            "class": case["class"],
            "spec": spec(case),
            "query": case["query"],
            "with_blank_node_data": status(record),
        }
        if record.get("rejection"):
            feature["with_blank_node_data_rejection"] = record["rejection"]
        if case["id"] in blank_free:
            other = blank_free[case["id"]]
            feature["blank_node_free_data"] = status(other)
            if other.get("rejection"):
                feature["blank_node_free_data_rejection"] = other["rejection"]
        if case["id"] in NOTES:
            feature["note"] = NOTES[case["id"]]
        features.append(feature)
    manifest = {
        "schema_version": 1,
        "relation": "authenticated RDF (V5): SPARQL over Data Integrity credentials",
        "dialect": "SparqSparql11GraphResultsV3",
        "evidence": {
            "layer": "guest_execution",
            "runner": "zk/sparql-evaluator/host/examples/paper_sweep.rs",
            "cases": "zk/sparql-evaluator/fixtures/paper/sweep.json",
            "command": "cargo run --release --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator --example paper_sweep --features authenticated-rdf -- --set sweep --suite eddsa --mode hidden --n 1 --size 32 --execute [--blank-free]",
            "generator": "zk/sparql-evaluator/scripts/authenticated_rdf_coverage.py",
            "credential": "one eddsa-rdfc-2022 credential of 32 statements, hidden signature mode",
            "guest_proof_coverage": "not_asserted",
            "full_w3c_conformance": "not_asserted",
        },
        "conditions": [
            "FILTER EXISTS and FILTER NOT EXISTS are admitted only when no credential in the presentation contains a blank node (model/src/v3/evaluate.rs).",
            "Results are exact SPARQL 1.1 answers over the default graph formed by the presented credentials; there are no named graphs.",
        ],
        "features": features,
    }
    json.dump(manifest, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
