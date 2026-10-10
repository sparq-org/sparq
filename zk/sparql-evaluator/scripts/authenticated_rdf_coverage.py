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
    "probe-from-named": SPARQL + "#specifyingDataset",
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
    "form-describe": "The guest evaluates DESCRIBE, but the host rejects it in the request before proving and verification.",
    "probe-graph": "Admitted and evaluated as specified: the credentials form the default graph and the dataset has no named graphs, so GRAPH matches nothing.",
    "probe-from": "The guest evaluates it as specified (FROM names a graph the dataset does not hold, so the default graph is empty), but the host rejects dataset clauses in the request before proving and verification.",
    "probe-from-named": "The guest evaluates it as specified (the named graph is absent, so GRAPH matches nothing), but the host rejects dataset clauses in the request before proving and verification.",
    "probe-query-blank-node": "A blank node in a query pattern is an existential variable; this is an ordinary basic graph pattern.",
    "blank-node-data": "Credential data containing blank nodes is supported; blank-node labels are renamed per credential so credentials never share one.",
}


def spec(case):
    return SPEC.get(case["id"]) or SPEC.get(case["id"].split("-")[0]) or SPEC.get(case["class"])


def load(path):
    with open(path) as handle:
        return {record["id"]: record for record in map(json.loads, handle)}


CONFIGURATION = {"set": "sweep", "suite": "eddsa-rdfc-2022", "mode": "hidden", "n": 1, "statements_per_credential": 32}


def status(record, blank_nodes):
    """Classify one record, refusing anything that is not guest-execution evidence."""
    expected = dict(CONFIGURATION, credential_blank_nodes=blank_nodes)
    for key, value in expected.items():
        if record.get(key) != value:
            sys.exit(f"{record['id']}: {key} is {record.get(key)!r}, expected {value!r}")
    if record["admitted"]:
        execution = record.get("execution")
        if not isinstance(execution, dict) or "error" in execution or not execution.get("user_cycles"):
            sys.exit(f"{record['id']}: admitted natively but has no successful guest execution")
        return "guest_executed"
    if not record.get("guest_aborted"):
        sys.exit(f"{record['id']}: rejected natively but the guest did not run to an abort")
    return "rejected_in_guest"


def main():
    sweep_path, coverage_path, blank_free_path = sys.argv[1:4]
    with open(sweep_path) as handle:
        cases = json.load(handle)["cases"]
    data, blank_free = load(coverage_path), load(blank_free_path)
    features = []
    for case in cases:
        if case["id"] not in data:
            sys.exit(f"{case['id']}: no record in {coverage_path}")
        record = data[case["id"]]
        feature = {
            "id": case["id"],
            "class": case["class"],
            "spec": spec(case),
            "query": case["query"],
            "with_blank_node_data": status(record, True),
        }
        if record.get("rejection"):
            feature["with_blank_node_data_rejection"] = record["rejection"]
        if "query_profile" not in record:
            sys.exit(f"{case['id']}: record has no host query profile result")
        if record["query_profile"] == "accepted":
            feature["host_query_profile"] = "accepted"
        else:
            feature["host_query_profile"] = "rejected"
            feature["host_query_profile_rejection"] = record["query_profile"]
        if case["id"] in blank_free:
            other = blank_free[case["id"]]
            feature["blank_node_free_data"] = status(other, False)
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
            "The host rejects DESCRIBE, FROM and FROM NAMED in the request before proving and before verification (check_query_profile in host/src/authenticated_rdf.rs); the guest itself evaluates them. host_query_profile records that check.",
            "Results are exact SPARQL 1.1 answers over the default graph formed by the presented credentials; there are no named graphs.",
        ],
        "features": features,
    }
    json.dump(manifest, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
