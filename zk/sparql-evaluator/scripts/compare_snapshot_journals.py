#!/usr/bin/env python3
"""Compare snapshot_journals output, allowing only the reviewed numeric rejections."""

import argparse
import json
from pathlib import Path


RESTORED = {
    "decimal-alignment-overflow", "seconds-numeric-consumer", "stored-false-ask",
    "stored-false-count", "stored-identity-equality", "stored-order", "stored-topk",
}
ADDED = {
    "stored-order-alignment-overflow", "stored-topk-alignment-overflow",
    "stored-min-alignment-overflow",
}
CAPACITY_SOURCE = "zk/sparql-evaluator/fixtures/conformance/numeric-capacity.json"
VERSIONS = ("v1", "v2", "v3")


def load(path):
    records = {}
    for line in path.read_text().splitlines():
        record = json.loads(line)
        key = (record["source"], record["id"], record["holder"])
        if key in records:
            raise ValueError(f"duplicate record: {key}")
        records[key] = record
    if not records:
        raise ValueError("empty corpus output")
    return records


def compare(before, after):
    additions = {
        (CAPACITY_SOURCE, case, holder) for case in ADDED for holder in (False, True)
    }
    if before.keys() & additions or after.keys() != before.keys() | additions:
        raise ValueError("corpus record inventories differ")
    restored, preserved, successful = set(), 0, 0
    for key, old in before.items():
        new = after[key]
        for version in VERSIONS:
            if old[version] == new[version]:
                preserved += 1
                successful += "journal" in old[version]
                continue
            if (key[0] != CAPACITY_SOURCE or key[1] not in RESTORED
                    or "journal" not in old[version]
                    or new[version] != {"error": "Capacity(NumericRepresentation)"}):
                raise ValueError(f"unexpected journal/outcome change: {key}, {version}")
            restored.add((*key, version))
    expected = {
        (CAPACITY_SOURCE, case, holder, version)
        for case in RESTORED for holder in (False, True) for version in VERSIONS
    }
    if restored != expected:
        raise ValueError("missing restored numeric bound case/version/authority")
    for key in additions:
        for version in VERSIONS:
            if after[key][version] != {"error": "Capacity(NumericRepresentation)"}:
                raise ValueError(f"unexpected added capacity outcome: {key}, {version}")
    return {
        "records": len(before), "preserved_outcomes": preserved,
        "preserved_success_journals": successful, "restored_capacity_rejections": len(restored),
        "added_capacity_rejections": len(additions) * len(VERSIONS),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    args = parser.parse_args()
    print(json.dumps(compare(load(args.before), load(args.after)), sort_keys=True))


if __name__ == "__main__":
    main()
