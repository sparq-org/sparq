#!/usr/bin/env python3
# sq-eifd: SHARED SHACL-validation-report -> (conforms, violation-count)
# parser.
#
# This is THE contract that makes cross-engine SHACL counts comparable. Every
# report-cli adapter (pySHACL, Jena `shacl validate`, TopBraid) emits a SHACL
# ValidationReport graph (Turtle / N-Triples); this module reduces it to the SAME
# two numbers sparq-shacl exposes via `report.conforms` + `report.results.len()`:
#
#   conforms   : the value of `sh:conforms` (bool) on the sh:ValidationReport node;
#                falls back to (violations == 0) when no sh:conforms triple exists.
#   violations : the number of sh:ValidationResult nodes (the `sh:result` set).
#
# These are EXACTLY sparq-shacl's semantics:
#   - sparq's `report.conforms` == `results.is_empty()` (report.rs), and its
#     `to_turtle()` emits `sh:conforms <bool>` + one `sh:result [ a
#     sh:ValidationResult ... ]` per result. So sparq's OWN --turtle report parses
#     through this same function and yields the same (conforms, violations) pair —
#     the basis of the cross-engine count-agreement check (sparq vs pySHACL vs
#     rdf-validate-shacl on one fixture).
#   - It REUSES the count semantics of the diff-fuzz pyshacl_adapter
#     (crates/sparq-shacl/tests/diff_fuzz/pyshacl_adapter.py): count
#     sh:ValidationResult, read sh:conforms. That adapter additionally captures the
#     per-violation (focus, component, path) set for differential FUZZING; here we
#     only need the COUNT (the benchmark cross-check), so we keep it minimal.
#
# HONEST CAVEAT (carried from research/capability-benchmark-program.md §3.1c):
# engines that DEDUP results across traversal routes report a different #violations
# for the same data than sparq (which does not dedup). The count this returns is the
# RAW sh:result count of whatever the engine emitted; the per-engine expected count
# (not one shared constant) is what a suite's expected.tsv must record.
import sys


SH = "http://www.w3.org/ns/shacl#"


def count_report(report_ttl, fmt="turtle"):
    """Parse a SHACL ValidationReport graph -> {"conforms": bool, "violations": int}.

    `report_ttl` is the serialised report graph; `fmt` is an rdflib format string
    ("turtle" | "nt" | "n3" | "xml" ...). Raises on a parse error (the caller's
    adapter boundary turns that into a non-zero exit so a benchmark can tell
    "engine could not run" from "engine says conforms")."""
    import rdflib

    g = rdflib.Graph()
    g.parse(data=report_ttl, format=fmt)
    return count_graph(g)


def count_graph(g):
    """Count over an already-parsed rdflib.Graph (lets the js/oracle paths share
    the exact same reduction without re-serialising)."""
    import rdflib

    result_t = rdflib.URIRef(SH + "ValidationResult")
    rdf_type = rdflib.RDF.type
    p_conforms = rdflib.URIRef(SH + "conforms")
    report_t = rdflib.URIRef(SH + "ValidationReport")

    violations = sum(1 for _ in g.subjects(rdf_type, result_t))

    conforms = None
    # Prefer the sh:conforms on a sh:ValidationReport node; else any sh:conforms.
    for report in g.subjects(rdf_type, report_t):
        v = g.value(report, p_conforms)
        if v is not None:
            conforms = bool(v.toPython())
            break
    if conforms is None:
        for _s, _p, v in g.triples((None, p_conforms, None)):
            conforms = bool(v.toPython())
            break
    if conforms is None:
        # No explicit sh:conforms triple: derive it the way sparq does
        # (conforms iff there are no results).
        conforms = violations == 0
    return {"conforms": conforms, "violations": violations}


def main(argv):
    """CLI: shacl_report_count.py [--format FMT] [report-file | -]  -> JSON on stdout.

    Reads a validation-report graph from a file (or stdin with `-`), prints
    {"conforms":bool,"violations":int}."""
    fmt = "turtle"
    args = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--format":
            i += 1
            fmt = argv[i]
        elif a.startswith("--format="):
            fmt = a.split("=", 1)[1]
        else:
            args.append(a)
        i += 1
    src = args[0] if args else "-"
    if src == "-":
        text = sys.stdin.read()
    else:
        with open(src, "r", encoding="utf-8") as fh:
            text = fh.read()
    import json

    try:
        out = count_report(text, fmt=fmt)
    except Exception as e:  # noqa: BLE001 — adapter boundary: report + non-zero exit
        sys.stderr.write("shacl_report_count: parse error: %s\n" % e)
        return 1
    json.dump(out, sys.stdout)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
