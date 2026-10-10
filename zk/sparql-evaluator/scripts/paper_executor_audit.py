#!/usr/bin/env python3
"""Summarise `paper_sweep --execute` records as an evidence audit record.

Usage:
  paper_executor_audit.py [--reruns RERUNS_JSONL_GZ] RECORDS_JSONL_GZ SOURCE_COMMIT IMAGE_ID PHASES_IMAGE_ID R0VM_VERSION [SOURCE_NOTE] > audit.json

RECORDS_JSONL_GZ is the gzipped stdout of
`host/examples/paper_sweep.rs --set main|sweep --execute --phases`. Executor
cycle counts are deterministic for a given guest image and input; wall-clock
fields in the records were taken on a shared container and are not summarised.
No proofs were produced.

Each natively rejected run is also executed in the guest. Its `rejection_cause`
is `guest_abort` when the guest aborted with the relation's rejection. Records
that predate the harness's `guest_error` field do not say why a run did not
abort; RERUNS_JSONL_GZ holds re-executions of those runs, matched by case and
configuration, that do (the executor is deterministic for a given image and
input). A cause other than the guest abort or the session limit is an error.
"""

import gzip
import hashlib
import json
import statistics
import sys

# The prover-local session ceiling shared by the exact and V5 provers.
SESSION_LIMIT_CYCLES = 1 << 25


def key(r):
    return (r["id"], r["suite"], r["mode"], r["n"], r["statements_per_credential"])


def load(path):
    raw = gzip.open(path, "rb").read()
    return raw, [json.loads(line) for line in raw.splitlines() if line.strip()]


def cause(record, reruns):
    """Why a natively rejected run ended, from its own guest outcome or its re-execution."""
    if record.get("guest_aborted"):
        return "guest_abort"
    source = record if "guest_error" in record else reruns.get(key(record))
    if source is None:
        sys.exit(f"{key(record)}: did not abort in the guest and has no recorded executor error")
    if source.get("admitted") or source.get("rejection") != record["rejection"]:
        sys.exit(f"{key(record)}: re-execution disagrees with the native rejection")
    if source.get("guest_aborted"):
        sys.exit(f"{key(record)}: re-execution aborted in the guest but the record did not")
    error = source.get("guest_error", "")
    if error.startswith("Session limit exceeded"):
        return "session_limit"
    sys.exit(f"{key(record)}: unexpected executor error {error!r}")


def main():
    args = sys.argv[1:]
    reruns, reruns_raw = {}, None
    if args[:1] == ["--reruns"]:
        reruns_raw, rerun_records = load(args[1])
        reruns = {key(r): r for r in rerun_records}
        args = args[2:]
    path, source, image_id, phases_image_id, r0vm = args[:5]
    source_note = args[5] if len(args) > 5 else None
    raw, records = load(path)
    sets = {r["set"] for r in records}
    if len(sets) != 1:
        sys.exit(f"records mix sets {sorted(sets)}")
    grid = {}
    cases = {}
    for r in records:
        cases.setdefault(r["id"], r["query_profile"])
        config = (r["suite"], r["mode"], r["n"], r["statements_per_credential"])
        grid.setdefault(config, []).append(r)
    configurations = []
    for (suite, mode, n, size), rows in sorted(grid.items()):
        executed = [r for r in rows if r["admitted"] and "error" not in r.get("execution", {})]
        over = [r["id"] for r in rows if r["admitted"] and "error" in r.get("execution", {})]
        for r in rows:
            error = r.get("execution", {}).get("error")
            if error and not error.startswith("Session limit exceeded"):
                sys.exit(f"{r['id']}: unexpected execution error {error}")
        user = [r["execution"]["user_cycles"] for r in executed]
        total = [r["execution"]["padded_cycles"] for r in executed]
        entry = {
            "suite": suite,
            "signature_mode": mode,
            "credentials": n,
            "statements_per_credential": size,
            "cases": len(rows),
            "admitted": sum(r["admitted"] for r in rows),
            "rejected": sum(not r["admitted"] for r in rows),
            "executed": len(executed),
            "over_session_limit": sorted(over),
        }
        rejected = [r for r in rows if not r["admitted"]]
        if rejected:
            entry["rejected_runs"] = [
                {"case": r["id"], "rejection": r["rejection"], "rejection_cause": cause(r, reruns)}
                for r in sorted(rejected, key=lambda r: r["id"])
            ]
        if user:
            entry["user_cycles"] = {"median": statistics.median(user), "min": min(user), "max": max(user)}
            entry["total_cycles"] = {"median": statistics.median(total), "min": min(total), "max": max(total)}
            entry["segments"] = {
                "median": statistics.median(r["execution"]["segments"] for r in executed),
                "max": max(r["execution"]["segments"] for r in executed),
            }
        configurations.append(entry)
    phases = []
    for r in records:
        if r["set"] == "main" and isinstance(r.get("phase_cycles"), dict) and "error" not in r["phase_cycles"]:
            phases.append({
                "case": r["id"],
                "suite": r["suite"],
                "signature_mode": r["mode"],
                "credentials": r["n"],
                "phase_cycles": r["phase_cycles"],
            })
    audit = {
        "schema": "sparq.zk-paper-executor-audit.v1",
        "source": source,
        **({"source_note": source_note} if source_note else {}),
        "harness": "zk/sparql-evaluator/host/examples/paper_sweep.rs",
        "generator": "zk/sparql-evaluator/scripts/paper_executor_audit.py",
        "set": sets.pop(),
        "guest_image_id": image_id,
        "phase_measurement_image_id": phases_image_id,
        "r0vm": r0vm,
        "executor_only": True,
        "proofs": 0,
        "records_sha256": hashlib.sha256(raw).hexdigest(),
        "records": len(records),
        **({
            "reruns_sha256": hashlib.sha256(reruns_raw).hexdigest(),
            "reruns": len(reruns),
        } if reruns_raw is not None else {}),
        "session_limit_cycles": SESSION_LIMIT_CYCLES,
        "note": (
            "RISC Zero executor runs of the accepted V5 image; user_cycles exclude padding, "
            "total_cycles are padded segment cycles. Phase cycles come from the separate "
            "phase measurement image and include its instrumentation. Wall-clock fields in "
            "the records are from a shared container and are not evidence."
        ),
        "cases": [{"id": k, "host_query_profile": v} for k, v in cases.items()],
        "configurations": configurations,
    }
    if phases:
        audit["phase_breakdown"] = phases
    json.dump(audit, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
