#!/usr/bin/env python3
# sq-hmd7l.18 — Python-binding benchmark adapter: sparq-py vs pyoxigraph vs rdflib.
"""One adapter, three engines, four modes — the `python-bindings-bench` suite driver.

Measures the cost of driving an RDF engine FROM PYTHON, per engine:

  * ``sparq``      — sparq-py (pyo3 binding over sparq-core/engine; ``pip`` name
                     ``sparq-rdf``, import name ``sparq``)
  * ``pyoxigraph`` — the official Python binding of Oxigraph (also pyo3)
  * ``rdflib``     — pure Python (NO FFI boundary; its whole-call time is
                     ENGINE-bound, not binding-bound — label it honestly)

Modes (all emit a single JSON document to stdout, and to ``--json PATH`` if given):

  workload  --input data.ttl --queries <dir-or-.rq-files>
            Load the corpus (min-of-``--load-iters``, fresh store per iteration)
            then run each SELECT query warm (load once, ``--iters`` timed calls,
            min + median). Every solution row is fully materialised: each bound
            cell is extracted into a Python object, matching what sparq-py's
            ``Graph.query`` does eagerly. Row counts are recorded per query so
            ``--compare`` can enforce the cross-engine agreement gate.

  floor     No corpus argument (fixed 8-triple inline graph). Times the calls
            where the Python<->engine BOUNDARY dominates, because engine work is
            ~nil: ``len(g)`` (pure boundary crossing), a matching ``ASK``
            (boundary + query parse + trivial eval), a 0-row SELECT (boundary +
            parse, zero materialisation), a ``LIMIT 1`` SELECT (one-row
            materialisation) and a full 8-row SELECT. min + median over
            auto-calibrated iteration counts. This is the primary
            BINDING-OVERHEAD isolation instrument.

  slope     SELECT ?s ?p ?o over two synthetic N-Triples graphs (default 64 and
            8192 triples); the per-row time slope ≈ per-row cost of crossing the
            boundary WITH result materialisation. HONESTY: the slope includes the
            engine's scan cost per row too — for the two Rust engines the Python
            object construction dominates, for rdflib the engine does.

  compare   ``--compare a.json b.json [...] [--cli cli.json]`` — the agreement +
            report stage. Exits 1 unless every query's row count agrees across
            all engine JSONs (and the sparq-cli reference when given): the
            catalog's "no timing row without row-count agreement" invariant.
            Prints a combined TSV table; with ``--cli`` (a ``sparq-cli bench
            ... --json`` document, ``materialize`` mode) it also prints the
            binding-overhead column: sparq-py total minus engine-internal time.

The engines are gather-time-only dependencies (NEVER committed to the repo —
see bench/CATALOG.md); an engine that fails to import is reported by run.sh as
SKIPPED, this adapter just fails loudly.  Wall-clock caveat: quiet_box_sensitive
(bench/CATALOG.md) — absolute numbers from a busy box are indicative only.
"""

import argparse
import gc
import json
import platform
import statistics
import sys
import time
from datetime import datetime, timezone

# ---------------------------------------------------------------------------
# Fixed micro-workloads (deterministic, tiny — the point is the boundary).
# ---------------------------------------------------------------------------

FLOOR_NT = "".join(f'<urn:x:s{i}> <urn:x:p> "v{i}" .\n' for i in range(8))

FLOOR_OPS = [
    # (name, kind, sparql-or-None, expected_rows_or_answer)
    ("len_call", "len", None, 8),
    ("ask_hit", "ask", 'ASK { <urn:x:s0> <urn:x:p> "v0" }', True),
    ("select_0row", "select", "SELECT ?s WHERE { ?s <urn:x:absent> ?o }", 0),
    ("select_1row", "select", "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 1", 1),
    ("select_8row", "select", "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", 8),
]

SELECT_ALL = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }"


def synth_nt(n):
    """Deterministic n-triple N-Triples doc (distinct subjects, 7 predicates)."""
    return "".join(f'<urn:x:s{i}> <urn:x:p{i % 7}> "value-{i}" .\n' for i in range(n))


# ---------------------------------------------------------------------------
# Engine wrappers. Each exposes: version(), load_file(path, fmt), load_data(nt),
# query_rows(store, sparql) -> row count with every bound cell materialised,
# ask(store, sparql) -> bool, size(store) -> int.
# ---------------------------------------------------------------------------


class SparqEngine:
    name = "sparq"

    def __init__(self):
        import sparq  # noqa: F401

        self.m = sparq

    def version(self):
        import importlib.metadata

        try:
            return importlib.metadata.version("sparq-rdf")
        except importlib.metadata.PackageNotFoundError:
            return "unknown"

    def load_file(self, path, fmt):
        return self.m.Graph.load(path, format=fmt)

    def load_data(self, nt):
        return self.m.Graph.load(nt, format="ntriples")

    def query_rows(self, g, sparql):
        # Graph.query materialises every cell into Python Term objects EAGERLY
        # (that is the binding cost under test); touch each cell anyway so all
        # three engines end at the same place: every bound term in Python hands.
        rows = g.query(sparql).rows
        for row in rows:
            for t in row.values():
                pass
        return len(rows)

    def ask(self, g, sparql):
        return g.ask(sparql)

    def size(self, g):
        return len(g)


class PyoxigraphEngine:
    name = "pyoxigraph"

    def __init__(self):
        import pyoxigraph

        self.m = pyoxigraph

    def version(self):
        return self.m.__version__

    def _fmt(self, fmt):
        return {
            "turtle": self.m.RdfFormat.TURTLE,
            "ntriples": self.m.RdfFormat.N_TRIPLES,
        }[fmt]

    def load_file(self, path, fmt, bulk=True):
        s = self.m.Store()  # in-memory store
        if bulk:
            s.bulk_load(path=path, format=self._fmt(fmt))
        else:
            s.load(path=path, format=self._fmt(fmt))
        return s

    def load_data(self, nt):
        s = self.m.Store()
        s.load(input=nt.encode(), format=self.m.RdfFormat.N_TRIPLES)
        return s

    def query_rows(self, s, sparql):
        # pyoxigraph solutions are LAZY: query() returns a stream and each
        # sol[var] builds the Python term on access — so full materialisation
        # (the matched workload) requires touching every bound cell.
        res = s.query(sparql)
        variables = res.variables
        n = 0
        for sol in res:
            for v in variables:
                sol[v]
            n += 1
        return n

    def ask(self, s, sparql):
        return bool(s.query(sparql))

    def size(self, s):
        return len(s)


class RdflibEngine:
    name = "rdflib"

    def __init__(self):
        import rdflib

        self.m = rdflib

    def version(self):
        return self.m.__version__

    def load_file(self, path, fmt):
        g = self.m.Graph()
        g.parse(path, format={"turtle": "turtle", "ntriples": "nt"}[fmt])
        return g

    def load_data(self, nt):
        g = self.m.Graph()
        g.parse(data=nt, format="nt")
        return g

    def query_rows(self, g, sparql):
        n = 0
        for row in g.query(sparql):
            for t in row:
                pass
            n += 1
        return n

    def ask(self, g, sparql):
        return bool(g.query(sparql).askAnswer)

    def size(self, g):
        return len(g)


ENGINES = {e.name: e for e in (SparqEngine, PyoxigraphEngine, RdflibEngine)}


# ---------------------------------------------------------------------------
# Timing helpers.
# ---------------------------------------------------------------------------


def time_us(fn, iters, warmup):
    """(min_us, median_us) over `iters` calls after `warmup`; gc paused during
    each sample so a collection doesn't land inside one timing window."""
    for _ in range(warmup):
        fn()
    samples = []
    gc.collect()
    gc_was = gc.isenabled()
    gc.disable()
    try:
        for _ in range(iters):
            t0 = time.perf_counter_ns()
            fn()
            samples.append((time.perf_counter_ns() - t0) / 1e3)
    finally:
        if gc_was:
            gc.enable()
    return min(samples), statistics.median(samples)


def calibrated_iters(fn, cap, budget_s=1.5):
    """Iteration count targeting ~budget_s of sampling, clamped to [30, cap]."""
    t0 = time.perf_counter()
    fn()
    est = max(time.perf_counter() - t0, 1e-7)
    return max(30, min(cap, int(budget_s / est)))


# ---------------------------------------------------------------------------
# Modes.
# ---------------------------------------------------------------------------


def run_workload(eng, args):
    import os

    qfiles = []
    for q in args.queries:
        if os.path.isdir(q):
            qfiles += sorted(
                os.path.join(q, f) for f in os.listdir(q) if f.endswith(".rq")
            )
        else:
            qfiles.append(q)
    queries = [
        (os.path.splitext(os.path.basename(p))[0], open(p).read()) for p in qfiles
    ]

    # Load: fresh store per iteration, min-of-K. (One-call FFI; binding share is
    # a single boundary crossing — the time is parse+index ENGINE work.)
    load_min, load_med = time_us(
        lambda: eng.load_file(args.input, args.format), args.load_iters, 0
    )
    store = eng.load_file(args.input, args.format)
    out = {
        "load": {
            "min_us": load_min,
            "median_us": load_med,
            "iters": args.load_iters,
            "triples": eng.size(store),
        },
        "queries": [],
    }
    if eng.name == "pyoxigraph":
        nb_min, nb_med = time_us(
            lambda: eng.load_file(args.input, args.format, bulk=False),
            args.load_iters,
            0,
        )
        out["load"]["note"] = "bulk_load (pyoxigraph's documented initial-load fast path)"
        out["load_nonbulk"] = {"min_us": nb_min, "median_us": nb_med, "iters": args.load_iters}

    for name, sparql in queries:
        rows = eng.query_rows(store, sparql)  # warm + count check
        q_min, q_med = time_us(
            lambda s=sparql: eng.query_rows(store, s), args.iters, 1
        )
        out["queries"].append(
            {"name": name, "rows": rows, "min_us": q_min, "median_us": q_med}
        )
    return out


def run_floor(eng, args):
    g = eng.load_data(FLOOR_NT)
    assert eng.size(g) == 8
    out = {"ops": []}
    for name, kind, sparql, expect in FLOOR_OPS:
        if kind == "len":
            fn = lambda: eng.size(g)  # noqa: E731
            got = fn()
        elif kind == "ask":
            fn = lambda s=sparql: eng.ask(g, s)  # noqa: E731
            got = fn()
        else:
            fn = lambda s=sparql: eng.query_rows(g, s)  # noqa: E731
            got = fn()
        if got != expect:
            print(
                f"FATAL floor op {name}: got {got!r} want {expect!r}", file=sys.stderr
            )
            sys.exit(1)
        iters = calibrated_iters(fn, args.floor_iters)
        f_min, f_med = time_us(fn, iters, min(iters // 10, 200))
        out["ops"].append(
            {"name": name, "min_us": f_min, "median_us": f_med, "iters": iters}
        )
    return out


def run_slope(eng, args):
    sizes = sorted(int(s) for s in args.slope_sizes.split(","))
    points = []
    for n in sizes:
        g = eng.load_data(synth_nt(n))
        assert eng.size(g) == n
        fn = lambda: eng.query_rows(g, SELECT_ALL)  # noqa: E731
        assert fn() == n
        iters = calibrated_iters(fn, args.iters)
        s_min, s_med = time_us(fn, iters, min(iters // 10, 50))
        points.append({"rows": n, "min_us": s_min, "median_us": s_med, "iters": iters})
    small, big = points[0], points[-1]
    ns_per_row = (big["min_us"] - small["min_us"]) * 1e3 / (big["rows"] - small["rows"])
    return {
        "points": points,
        "ns_per_row": ns_per_row,
        "note": "slope = engine scan + binding materialisation per row (combined; "
        "see docstring)",
    }


# ---------------------------------------------------------------------------
# compare: cross-engine row-count agreement gate + combined report.
# ---------------------------------------------------------------------------


def check_agreement(docs, cli_doc=None):
    """Return (ok, table_rows). docs: list of adapter workload JSONs.
    Row counts must agree per query across all docs (and cli_doc if given)."""
    ok = True
    by_engine = {d["engine"]: {q["name"]: q for q in d["results"]["queries"]} for d in docs}
    cli_by_name = {}
    if cli_doc is not None:
        for q in cli_doc.get("queries", []):
            if "error" in q:
                ok = False
                print(f"DISAGREE {q['name']}: sparq-cli ERROR {q['error']}", file=sys.stderr)
            else:
                cli_by_name[q["name"]] = q
    names = sorted({n for qs in by_engine.values() for n in qs})
    table = []
    for name in names:
        counts = {e: qs[name]["rows"] for e, qs in by_engine.items() if name in qs}
        if name in cli_by_name:
            counts["sparq-cli"] = cli_by_name[name]["rows"]
        if len(set(counts.values())) != 1:
            ok = False
            print(f"DISAGREE {name}: row counts {counts}", file=sys.stderr)
            continue
        row = {"name": name, "rows": next(iter(counts.values()))}
        for e, qs in by_engine.items():
            if name in qs:
                row[e + "_us"] = qs[name]["min_us"]
        if name in cli_by_name:
            row["cli_engine_us"] = cli_by_name[name]["min_micros"]
            if "sparq" in by_engine and name in by_engine["sparq"]:
                row["sparq_binding_overhead_us"] = (
                    by_engine["sparq"][name]["min_us"] - cli_by_name[name]["min_micros"]
                )
        table.append(row)
    return ok, table


def run_compare(args):
    docs = [json.load(open(p)) for p in args.compare]
    cli_doc = json.load(open(args.cli)) if args.cli else None
    ok, table = check_agreement(docs, cli_doc)
    engines = [d["engine"] for d in docs]
    hdr = ["query", "rows"] + [e + "_min_us" for e in engines]
    if cli_doc:
        hdr += ["cli_engine_us", "sparq_binding_overhead_us"]
    print("\t".join(hdr))
    for row in table:
        cells = [row["name"], str(row["rows"])]
        cells += ["%.1f" % row[e + "_us"] if e + "_us" in row else "-" for e in engines]
        if cli_doc:
            cells.append("%.1f" % row["cli_engine_us"] if "cli_engine_us" in row else "-")
            cells.append(
                "%.1f" % row["sparq_binding_overhead_us"]
                if "sparq_binding_overhead_us" in row
                else "-"
            )
        print("\t".join(cells))
    if not ok:
        print("ROW-COUNT AGREEMENT FAILED — timings above are NOT trustworthy", file=sys.stderr)
        return 1
    print(f"row-count agreement OK across: {', '.join(engines + (['sparq-cli'] if cli_doc else []))}", file=sys.stderr)
    return 0


# ---------------------------------------------------------------------------


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--engine", choices=sorted(ENGINES))
    ap.add_argument("--mode", choices=["workload", "floor", "slope"], default="workload")
    ap.add_argument("--input", help="RDF corpus file (workload mode)")
    ap.add_argument("--format", default="turtle", choices=["turtle", "ntriples"])
    ap.add_argument("--queries", nargs="*", default=[], help=".rq files or a dir")
    ap.add_argument("--iters", type=int, default=5, help="timed query iterations")
    ap.add_argument("--load-iters", type=int, default=3)
    ap.add_argument("--floor-iters", type=int, default=3000, help="cap for floor ops")
    ap.add_argument("--slope-sizes", default="64,8192")
    ap.add_argument("--json", help="also write the JSON document here")
    ap.add_argument("--compare", nargs="*", help="workload JSONs to gate + report")
    ap.add_argument("--cli", help="sparq-cli bench --json document (materialize mode)")
    args = ap.parse_args()

    if args.compare:
        sys.exit(run_compare(args))

    if not args.engine:
        ap.error("--engine is required (unless --compare)")
    eng = ENGINES[args.engine]()
    results = {
        "workload": run_workload,
        "floor": run_floor,
        "slope": run_slope,
    }[args.mode](eng, args)

    doc = {
        "suite": "python-bindings-bench",
        "engine": eng.name,
        "engine_version": eng.version(),
        "mode": args.mode,
        "python": platform.python_version(),
        "machine": platform.machine(),
        "utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "quiet_box_sensitive": True,
        "note": "wall-clock from whatever box ran this — non-canonical unless a quiet box",
        "input": args.input,
        "results": results,
    }
    text = json.dumps(doc, indent=1)
    print(text)
    if args.json:
        with open(args.json, "w") as f:
            f.write(text)


if __name__ == "__main__":
    main()
