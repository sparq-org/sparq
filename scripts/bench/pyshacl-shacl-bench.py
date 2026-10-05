#!/usr/bin/env python3
# sq-7d3dj.33 — in-process pySHACL driver for the same-box SHACL
# comparison harness (scripts/bench/shacl-same-box.sh).
#
# WHY IN-PROCESS (not the `pyshacl` CLI via report_cli_adapter.py): the CLI's
# wall clock bundles Python interpreter start-up + rdflib data parse + validate
# + report serialisation, which is NOT comparable to sparq's validate-only
# `validate_us` (bench_shacl loads once, times validate best-of-N). This driver
# mirrors bench_shacl's methodology exactly: parse the data graph ONCE (timed,
# advisory), then per shapes file time `pyshacl.validate` best-of-N — so the
# per-workload number is pySHACL's validation cost, same contract as sparq's.
#
# USAGE
#   pyshacl-shacl-bench.py <data> <nt|ttl> <shapes-dir> <iters> [timeout_s]
#
# OUTPUT (stdout) — the bench_shacl 6-column TSV, one line per shapes/*.ttl
# (sorted by workload name); `focus_nodes` is `na` (pySHACL does not expose a
# target-selection count):
#   <workload>\t<violations>\t<validate_us>\t<conforms 0/1>\t<na>\t<load_us>
# On a per-workload timeout/error the row degrades honestly:
#   <workload>\tERROR\ttimeout|error:<msg>\tna\tna\t<load_us>
#
# stderr carries a one-line JSON meta block: {"pyshacl": ..., "rdflib": ...}.
#
# The per-workload timeout uses signal.alarm (pySHACL is pure Python, so the
# alarm interrupts it cleanly); remaining workloads still run.
import json
import signal
import sys
import time
from pathlib import Path


class WorkloadTimeout(Exception):
    pass


def _alarm(_sig, _frm):
    raise WorkloadTimeout()


def main() -> int:
    if len(sys.argv) < 5:
        print(
            "usage: pyshacl-shacl-bench.py <data> <nt|ttl> <shapes-dir> <iters> [timeout_s]",
            file=sys.stderr,
        )
        return 2
    data_path, fmt, shapes_dir = sys.argv[1], sys.argv[2], sys.argv[3]
    iters = max(1, int(sys.argv[4]))
    timeout_s = int(sys.argv[5]) if len(sys.argv) > 5 else 900

    import rdflib
    from rdflib.namespace import SH
    import pyshacl

    print(
        json.dumps({"pyshacl": pyshacl.__version__, "rdflib": rdflib.__version__}),
        file=sys.stderr,
    )

    rdflib_fmt = {"nt": "nt", "ntriples": "nt", "ttl": "turtle", "turtle": "turtle"}[fmt]

    # ---- load the data graph once (timed: the ADVISORY load metric) ----
    t0 = time.perf_counter()
    data = rdflib.Graph()
    data.parse(data_path, format=rdflib_fmt)
    load_us = (time.perf_counter() - t0) * 1e6

    signal.signal(signal.SIGALRM, _alarm)

    for shapes_file in sorted(Path(shapes_dir).glob("*.ttl")):
        name = shapes_file.stem
        shapes = rdflib.Graph()
        shapes.parse(str(shapes_file), format="turtle")
        best_us, violations, conforms = float("inf"), None, None
        err = None
        for _ in range(iters):
            signal.alarm(timeout_s)
            try:
                t = time.perf_counter()
                # Defaults mirror the comparison contract: no inference, no
                # SHACL-AF advanced mode — plain W3C SHACL Core + SPARQL
                # constraints, same as sparq-shacl's validate() and Jena's
                # ShaclValidator (inference='none' is pySHACL's default too).
                ok, results_graph, _text = pyshacl.validate(
                    data, shacl_graph=shapes, inference="none"
                )
                best_us = min(best_us, (time.perf_counter() - t) * 1e6)
                conforms = ok
                # #violations comparable to sparq's report.results.len():
                # count sh:result edges off the report node (the same reduction
                # scripts/bench-adapters/shacl_report_count.py uses).
                violations = len(list(results_graph.objects(None, SH.result)))
            except WorkloadTimeout:
                err = "timeout"
                break
            except Exception as e:  # honest ERROR row, keep the other workloads
                err = f"error:{type(e).__name__}"
                break
            finally:
                signal.alarm(0)
        if err is not None or violations is None:
            print(f"{name}\tERROR\t{err or 'error'}\tna\tna\t{load_us:.1f}")
        else:
            print(
                f"{name}\t{violations}\t{best_us:.1f}\t{int(bool(conforms))}\tna\t{load_us:.1f}"
            )
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
