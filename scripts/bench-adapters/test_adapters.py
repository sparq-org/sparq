#!/usr/bin/env python3
# sq-eifd: unit tests for the bench-adapter parsers.
#
# FIXTURE-based parser tests that DO NOT need a live engine:
#   - shacl_report_count : parse a SHACL ValidationReport graph -> (conforms, viol)
#   - http_sparql_adapter.parse_sparql_json : SELECT / ASK / COUNT(*) reductions
#   - vector_lib_adapter.recall_at_k + parse_neighbour_tsv : recall scoring + deficit
#   - beir_ir_adapter : TREC qrels/run parsers + Recall@k / nDCG@k + deficit (sq-1fz0)
# shacl_report_count's test needs rdflib (the SHACL report parser's only dep); if
# rdflib is absent that one test SKIPS (the http + vector tests are stdlib-only and
# always run). Run with the venv that has rdflib for full coverage:
#   /tmp/sq-eifd-venv/bin/python scripts/bench-adapters/test_adapters.py
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
FIX = os.path.join(HERE, "fixtures")

import beir_ir_adapter as beir  # noqa: E402
import http_sparql_adapter as http  # noqa: E402
import vector_lib_adapter as vec  # noqa: E402

PASS = 0
FAIL = 0


def check(name, got, want):
    global PASS, FAIL
    if got == want:
        PASS += 1
        print("ok   %-46s = %r" % (name, got))
    else:
        FAIL += 1
        print("FAIL %-46s got %r want %r" % (name, got, want))


def approx(name, got, want, tol=1e-6):
    check(name, abs(got - want) < tol, True)


# --- http-sparql SPARQL-JSON parser -----------------------------------------
def test_http():
    with open(os.path.join(FIX, "sparql_select.json")) as f:
        sel = http.parse_sparql_json(f.read())
    check("http.select.count", sel["count"], 3)
    check("http.select.boolean", sel["boolean"], None)
    # 2-var rows are not a COUNT(*) wrap -> no scalar extracted
    check("http.select.count_value", sel["count_value"], None)

    with open(os.path.join(FIX, "sparql_count.json")) as f:
        cnt = http.parse_sparql_json(f.read())
    check("http.count.count", cnt["count"], 1)
    check("http.count.count_value", cnt["count_value"], "3")

    with open(os.path.join(FIX, "sparql_ask.json")) as f:
        ask = http.parse_sparql_json(f.read())
    check("http.ask.count", ask["count"], 1)
    check("http.ask.boolean", ask["boolean"], True)

    try:
        http.parse_sparql_json('{"head":{}}')
        check("http.bad-doc-raises", False, True)
    except ValueError:
        check("http.bad-doc-raises", True, True)

    # A non-boolean "boolean" member (e.g. the string "false") is an
    # invalid SPARQL-JSON ASK result and must be rejected, not coerced via bool().
    try:
        http.parse_sparql_json('{"head":{},"boolean":"false"}')
        check("http.non-bool-boolean-raises", False, True)
    except ValueError:
        check("http.non-bool-boolean-raises", True, True)

    # sq-7d3dj.34: --profile helpers (the offline-testable half).
    check(
        "http.split_endpoint.plain",
        http.split_endpoint("http://localhost:3030/ds/query"),
        ("localhost", 3030, "/ds/query"),
    )
    check(
        "http.split_endpoint.default-port+query",
        http.split_endpoint("http://h/sparql?default-graph-uri=g"),
        ("h", 80, "/sparql?default-graph-uri=g"),
    )
    check(
        "http.split_endpoint.https-default",
        http.split_endpoint("https://h/q"),
        ("h", 443, "/q"),
    )
    try:
        http.split_endpoint("ftp://h/x")
        check("http.split_endpoint.non-http-raises", False, True)
    except ValueError:
        check("http.split_endpoint.non-http-raises", True, True)
    check(
        "http.format_profile_tsv",
        http.format_profile_tsv(
            "fuseki",
            452,
            {"best_us": 1200, "ttfb_us": 800, "reconnects": 0},
            {"best_us": 1500, "ttfb_us": 1100},
        ),
        "fuseki\t452\t1200\t800\t1500\t1100",
    )


# --- vector recall scoring ---------------------------------------------------
def test_vector():
    with open(os.path.join(FIX, "knn_approx.tsv")) as f:
        approx_map = vec.parse_neighbour_tsv(f.read())
    with open(os.path.join(FIX, "knn_exact.tsv")) as f:
        exact_map = vec.parse_neighbour_tsv(f.read())
    check("vec.parse.nqueries", len(approx_map), 3)
    recall, n = vec.recall_at_k(approx_map, exact_map, 4)
    check("vec.recall.nqueries", n, 3)
    approx("vec.recall@4", recall, 11.0 / 12.0)
    check("vec.recall_deficit_milli", vec.recall_deficit_milli(recall), 83)
    # perfect recall -> deficit 0
    perfect, _ = vec.recall_at_k(exact_map, exact_map, 4)
    check("vec.recall@4.perfect", vec.recall_deficit_milli(perfect), 0)
    # disjoint query ids must raise (catch a mis-aligned fixture loudly)
    try:
        vec.recall_at_k({"zz": ["1"]}, {"yy": ["1"]}, 4)
        check("vec.disjoint-raises", False, True)
    except ValueError:
        check("vec.disjoint-raises", True, True)


# --- gather-tier Pareto maths (sq-aiup; stdlib-only, no numpy/hnswlib) --------
def _fvecs_bytes(rows, fmt):
    """Encode rows as TEXMEX .fvecs/.ivecs bytes: per row <int32 d><d values>."""
    out = b""
    for r in rows:
        out += struct.pack("<i", len(r))
        out += struct.pack("<%d%s" % (len(r), fmt), *r)
    return out


def test_pareto():
    # .fvecs / .ivecs round-trip (the SIFT1M on-disk format) via read_vecs.
    fvecs = _fvecs_bytes([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]], "f")
    check("vec.read_fvecs", vec.read_vecs(fvecs, "f"), [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]])
    ivecs = _fvecs_bytes([[10, 20], [30, 40]], "i")
    check("vec.read_ivecs", vec.read_vecs(ivecs, "i"), [[10, 20], [30, 40]])
    # ragged dims must raise (truncated/mismatched corpus fails loudly).
    try:
        vec.read_vecs(_fvecs_bytes([[1.0, 2.0], [3.0]], "f"), "f")
        check("vec.read_vecs.ragged-raises", False, True)
    except ValueError:
        check("vec.read_vecs.ragged-raises", True, True)
    # truncated trailing row must raise.
    try:
        vec.read_vecs(struct.pack("<i", 3) + struct.pack("<2f", 1.0, 2.0), "f")
        check("vec.read_vecs.truncated-raises", False, True)
    except ValueError:
        check("vec.read_vecs.truncated-raises", True, True)

    # qps_from_query_us: inverse of latency; 0 for non-positive.
    approx("vec.qps.1000us", vec.qps_from_query_us(1000.0), 1000.0)
    check("vec.qps.zero", vec.qps_from_query_us(0), 0.0)

    # Pareto frontier: a dominated point (lower recall AND lower qps than another) drops.
    # (0.99, 500) dominates (0.95, 300); (0.999, 100) is on the frontier (best recall).
    pts = [(0.95, 300.0), (0.99, 500.0), (0.999, 100.0), (0.95, 200.0)]
    front = vec.pareto_frontier(pts)
    check("vec.pareto.frontier", front, [(0.99, 500.0), (0.999, 100.0)])
    # ties collapse to one point.
    check("vec.pareto.ties", vec.pareto_frontier([(0.9, 100.0), (0.9, 100.0)]), [(0.9, 100.0)])

    # matched_recall_qps: best QPS at recall >= target (the only honest cross-engine #).
    # At recall>=0.9: both frontier points qualify -> max qps = 500.
    check("vec.matched.0_9", vec.matched_recall_qps(pts, 0.9), 500.0)
    # At recall>=0.999: only (0.999,100) qualifies -> 100.
    check("vec.matched.0_999", vec.matched_recall_qps(pts, 0.999), 100.0)
    # Unreachable recall -> None (engine cannot hit that recall on this dataset).
    check("vec.matched.unreachable", vec.matched_recall_qps(pts, 1.0), None)


# --- BEIR IR scoring (sq-1fz0; stdlib-only, no pyserini/beir) ----------------
def test_beir():
    with open(os.path.join(FIX, "beir_qrels.txt")) as f:
        qrels = beir.parse_qrels(f.read())
    with open(os.path.join(FIX, "beir_run.txt")) as f:
        run = beir.parse_run(f.read())
    # parse: 3 queries; q1 keeps the rel-0 dZ as an explicit non-relevant judgement.
    check("beir.parse.nqueries", len(qrels), 3)
    check("beir.parse.q1_judged", sorted(qrels["q1"]), ["dA", "dB", "dZ"])
    check("beir.parse.q1_relevant", sorted(beir._relevant_docs(qrels["q1"])), ["dA", "dB"])
    # parse_run sorts by score regardless of file row order: q1 rows are dB,dA,dX in
    # the file but dA(9)>dX(8)>dB(7) by score.
    check("beir.parse.q1_ranked", run["q1"], ["dA", "dX", "dB"])
    # Recall@3 = mean(1.0, 0.5, 0.0) = 0.5  -> deficit 500 (see fixture header).
    recall, n = beir.recall_at_k(run, qrels, 3)
    check("beir.recall.nqueries", n, 3)
    approx("beir.recall@3", recall, 0.5)
    check("beir.recall_deficit_milli", beir.deficit_milli(recall), 500)
    # nDCG@3 = 0.4332715... -> deficit 567 (graded q2: dC grade-2 missed).
    ndcg, nn = beir.ndcg_at_k(run, qrels, 3)
    check("beir.ndcg.nqueries", nn, 3)
    approx("beir.ndcg@3", ndcg, 0.4332715186)
    check("beir.ndcg_deficit_milli", beir.deficit_milli(ndcg), 567)
    # A run that returns every relevant doc first -> perfect recall + nDCG -> deficit 0.
    perfect = {q: list(beir._relevant_docs(qrels[q])) for q in qrels}
    pr, _ = beir.recall_at_k(perfect, qrels, 100)
    check("beir.recall.perfect_deficit", beir.deficit_milli(pr), 0)
    # qrels with NO positive judgement must raise (catch a mis-aligned pair loudly).
    try:
        beir.recall_at_k({"q1": ["dA"]}, {"q1": {"dA": 0}}, 3)
        check("beir.no-positive-raises", False, True)
    except ValueError:
        check("beir.no-positive-raises", True, True)
    # malformed qrels (wrong field count) must raise.
    try:
        beir.parse_qrels("q1 0 dA")
        check("beir.bad-qrels-raises", False, True)
    except ValueError:
        check("beir.bad-qrels-raises", True, True)
    # malformed run (wrong field count) must raise.
    try:
        beir.parse_run("q1 Q0 dA 1 9.0")
        check("beir.bad-run-raises", False, True)
    except ValueError:
        check("beir.bad-run-raises", True, True)
    # --- sparq-side input conversion (the apples-to-apples bridge) -----------
    # corpus -> N-Triples: one `<beir:doc:DOCID> <beir:text> "..." .` per doc, sorted,
    # with ECHAR escaping of the literal body.
    nt = beir.corpus_to_ntriples({"d2": 'has "quotes"\nand newline', "d1": "plain text"})
    check(
        "beir.corpus_to_ntriples",
        nt,
        '<beir:doc:d1> <beir:text> "plain text" .\n'
        '<beir:doc:d2> <beir:text> "has \\"quotes\\"\\nand newline" .\n',
    )
    check("beir.escape_nt_literal", beir.escape_nt_literal('a\\b"c'), 'a\\\\b\\"c')
    # queries -> TSV: `<qid>\t<text>`, sorted, whitespace collapsed to single spaces.
    check(
        "beir.queries_to_tsv",
        beir.queries_to_tsv({"q2": "two", "q1": "a\tb\n c"}),
        "q1\ta b c\nq2\ttwo\n",
    )
    # qrels round-trips through parse_qrels (write TREC -> parse back -> identical map).
    qr = {"q1": {"dB": 1, "dA": 2}}
    check("beir.qrels_to_trec.roundtrip", beir.parse_qrels(beir.qrels_to_trec(qr)), qr)
    # empty inputs produce empty (not malformed) files.
    check("beir.corpus_to_ntriples.empty", beir.corpus_to_ntriples({}), "")
    check("beir.queries_to_tsv.empty", beir.queries_to_tsv({}), "")


# --- shacl report-count parser (needs rdflib) --------------------------------
def test_shacl():
    try:
        import rdflib  # noqa: F401
    except ImportError:
        print("skip shacl_report_count tests (rdflib not installed in this python)")
        return
    import shacl_report_count as src

    # A minimal non-conforming report: 2 results + sh:conforms false.
    report = """
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    [] a sh:ValidationReport ;
       sh:conforms false ;
       sh:result [ a sh:ValidationResult ;
                   sh:sourceConstraintComponent sh:MinCountConstraintComponent ] ;
       sh:result [ a sh:ValidationResult ;
                   sh:sourceConstraintComponent sh:DatatypeConstraintComponent ] .
    """
    r = src.count_report(report, fmt="turtle")
    check("shacl.report.violations", r["violations"], 2)
    check("shacl.report.conforms", r["conforms"], False)

    conforming = """
    @prefix sh: <http://www.w3.org/ns/shacl#> .
    [] a sh:ValidationReport ; sh:conforms true .
    """
    rc = src.count_report(conforming, fmt="turtle")
    check("shacl.report.conforming.violations", rc["violations"], 0)
    check("shacl.report.conforming.conforms", rc["conforms"], True)


# --- vlog / nemo self-reported-materialization parsers (sq-hmd7l.32) ----
# The materialize timing basis: both adapters prefer the engine's OWN loaded-graph
# materialization figure over whole-process wall (which would include the .nt load
# + closure export — an overstatement at LUBM(100) scale). These tests pin the parse
# of the exact upstream formats (VLog src/launcher/main.cpp, nemo-cli v0.9.1
# print_finished_message).
def test_reason_parsers():
    import nemo_adapter as nemo
    import vlog_adapter as vlog

    # VLog: ostream double — plain, fractional, and 6-sig-fig scientific forms.
    check(
        "vlog.parse.plain",
        vlog.parse_materialize_us("... Runtime materialization = 227 milliseconds ..."),
        227000.0,
    )
    check(
        "vlog.parse.fractional",
        vlog.parse_materialize_us("Runtime materialization = 226.885 milliseconds"),
        226885.0,
    )
    check(
        "vlog.parse.scientific",
        vlog.parse_materialize_us("Runtime materialization = 1.23457e+06 milliseconds"),
        1.23457e9,
    )
    # last occurrence wins; unrelated runtimes (total / pre-mat / store) never match.
    multi = (
        "Runtime pre-materialization = 5 milliseconds\n"
        "Runtime materialization = 100 milliseconds\n"
        "Runtime materialization = 90 milliseconds\n"
        "Time to index and store the materialization on disk = 3 seconds\n"
        "Runtime = 5000 milliseconds\n"
    )
    check("vlog.parse.last_of_multi", vlog.parse_materialize_us(multi), 90000.0)
    check("vlog.parse.absent", vlog.parse_materialize_us("Runtime = 5000 milliseconds"), None)
    check("vlog.parse.empty", vlog.parse_materialize_us(""), None)

    # Nemo: the `Reasoning:` breakdown row (right-aligned ms), NOT the coloured
    # "Reasoning completed in <total>ms" summary (that total includes import/export).
    report = (
        "Reasoning completed in \x1b[1;32m2650\x1b[0mms. Derived 47179 facts.\n"
        "   Data import:      450ms\n"
        "   Reasoning:       2100ms\n"
        "   Data export:      100ms\n"
    )
    check("nemo.parse.breakdown", nemo.parse_reasoning_us(report), 2100000.0)
    check(
        "nemo.parse.no_export_row",
        nemo.parse_reasoning_us("Reasoning completed in 10ms. Derived 1 facts.\n   Data import:   4ms\n   Reasoning:   6ms\n"),
        6000.0,
    )
    check(
        "nemo.parse.summary_only_is_absent",
        nemo.parse_reasoning_us("Reasoning completed in 2650ms. Derived 47179 facts.\n"),
        None,
    )
    check("nemo.parse.empty", nemo.parse_reasoning_us(""), None)


def test_python_bindings():
    # sq-hmd7l.18: the cross-engine row-count agreement gate + the
    # binding-overhead column (python whole-call minus sparq-cli engine-internal).
    # Stdlib-only: check_agreement takes parsed docs, no engine imports needed.
    import python_rdf_adapter as pyb

    def wdoc(engine, rows_q1, us_q1):
        return {
            "engine": engine,
            "results": {"queries": [{"name": "q01", "rows": rows_q1, "min_us": us_q1}]},
        }

    cli = {"queries": [{"name": "q01", "rows": 5, "min_micros": 10.0}]}
    ok, table = pyb.check_agreement([wdoc("sparq", 5, 17.5), wdoc("rdflib", 5, 900.0)], cli)
    check("pyb.agree.ok", ok, True)
    check("pyb.agree.rows", table[0]["rows"], 5)
    approx("pyb.agree.overhead", table[0]["sparq_binding_overhead_us"], 7.5)

    ok, table = pyb.check_agreement([wdoc("sparq", 5, 17.5), wdoc("rdflib", 4, 900.0)])
    check("pyb.disagree.flagged", ok, False)
    check("pyb.disagree.no_timing_row", table, [])  # invariant: no timing on disagreement

    ok, _ = pyb.check_agreement(
        [wdoc("sparq", 5, 17.5)], {"queries": [{"name": "q01", "error": "boom"}]}
    )
    check("pyb.cli_error.flagged", ok, False)


def main():
    test_http()
    test_vector()
    test_pareto()
    test_beir()
    test_shacl()
    test_reason_parsers()
    test_python_bindings()
    print("\n%d passed, %d failed" % (PASS, FAIL))
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
