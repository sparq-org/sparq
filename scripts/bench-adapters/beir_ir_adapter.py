#!/usr/bin/env python3
# sq-1fz0: BEIR IR-quality adapter KIND (`python-lib`).
#
# Wires the SECOND axis of the full-text-search suite (design §3.4): IR-quality on a
# small BEIR cut (SciFact / TREC-COVID + qrels) loaded as RDF literals -> Recall@100 /
# nDCG@10, emitted as DEFICITS (G4) so they slot into the smaller-is-better mode:"auto"
# ratchet with zero perf-gate change. The BM25 quality ORACLE is `lucene-anserini`
# (the EMBEDDED Lucene library via Anserini/pyserini — NOT Solr/ES the servers), run on
# the SAME BEIR cut + qrels so its Recall@100/nDCG@10 is directly comparable to
# sparq-text's. lucene-anserini is registered in bench/competitors.json
# (kind=python-lib, dashboard_engine_id=null = OFF the dashboard — it has no RDF/SPARQL
# surface; it is the kernel BM25 reference, "sub-component, not an RDF benchmark").
#
# WHY GATHER-ONLY (not a committed per-commit gate): the BEIR corpus is NOT
# redistributable in-repo, so this is a DOWNLOAD step (scripts/gather-competitors.sh
# --run --only lucene-anserini), not a committed corpus. The light scoring half below
# is fixture-unit-tested in CI WITHOUT any heavy dep; the heavy half (download + index +
# retrieve) runs only on a gather box.
#
# TWO pieces, separable so CI tests the light half and the gather box runs the heavy
# half (mirrors vector_lib_adapter.py exactly):
#   1. SCORING / ORACLE (pure, stdlib-only, fixture-tested):
#        parse_qrels(text)             TREC qrels  `<qid> 0 <docid> <rel>`  -> {qid:{docid:rel}}
#        parse_run(text)               TREC run    `<qid> Q0 <docid> <rank> <score> <tag>`
#                                                  -> {qid:[docid ranked]}
#        recall_at_k(run, qrels, k)    mean |retrieved_topk ∩ relevant| / |relevant|
#        ndcg_at_k(run, qrels, k)      mean DCG@k / IDCG@k with GRADED relevance
#        deficit_milli(metric)         round((1 - metric) * 1000)  (the G4 shape)
#   2. HEAVY HARNESS (gather-only; needs pyserini + a downloaded BEIR cut):
#        run_anserini_beir(dataset, k) download the BEIR cut, BM25-index it with
#                                      Anserini, retrieve top-k, return a TREC run +
#                                      the qrels -> scored against the SAME qrels.
#
# Output (stdout, the python-lib contract): one line per metric,
#   `<engine>\t<metric>\t<deficit_milli>`
# (recall_at_100, ndcg_at_10). --json adds the raw float metrics + k + the BEIR cut id
# for the results envelope / dashboard snapshot. The dashboard column is OFF (engine
# id null) — these numbers are the kernel IR oracle, never a SPARQL benchmark.
import json
import math
import sys


# --- TREC qrels / run parsers (BEIR + Anserini standard formats) -------------
def parse_qrels(text):
    """TREC qrels `<qid> <iter> <docid> <rel>` (whitespace-separated) -> {qid:{docid:rel}}.

    `<iter>` is conventionally `0` and ignored. `<rel>` is the GRADED relevance
    (>=1 relevant; 0 explicitly non-relevant). Blank lines and `#`-comments skipped.
    A graded judgement is kept as its integer for nDCG (BEIR SciFact is binary;
    TREC-COVID is graded 0/1/2). A negative relevance is rejected (malformed qrels)."""
    out = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) != 4:
            raise ValueError("bad qrels line (want 4 fields): %r" % line)
        qid, _iter, docid, rel = parts
        rel = int(rel)
        if rel < 0:
            raise ValueError("negative relevance in qrels: %r" % line)
        out.setdefault(qid, {})[docid] = rel
    return out


def parse_run(text):
    """TREC run `<qid> Q0 <docid> <rank> <score> <tag>` -> {qid:[docid ranked best-first]}.

    The canonical Anserini/pyserini output. Rows are SORTED by descending score (then
    by ascending rank as a deterministic tie-break) per query, so the resulting list is
    the true ranking regardless of input row order. A docid that appears twice for one
    query keeps its best (first-seen-after-sort) position. Blank lines / `#`-comments
    skipped."""
    scored = {}  # qid -> list of (score, rank, docid)
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) != 6:
            raise ValueError("bad run line (want 6 TREC fields): %r" % line)
        qid, _q0, docid, rank, score, _tag = parts
        scored.setdefault(qid, []).append((float(score), int(rank), docid))
    out = {}
    for qid, rows in scored.items():
        # best-first: higher score first; ties broken by smaller rank then docid so the
        # ordering is fully deterministic (a stable run regardless of file row order).
        rows.sort(key=lambda r: (-r[0], r[1], r[2]))
        seen = set()
        ranked = []
        for _score, _rank, docid in rows:
            if docid in seen:
                continue
            seen.add(docid)
            ranked.append(docid)
        out[qid] = ranked
    return out


# --- IR metrics (pure; the ORACLE) -------------------------------------------
def _relevant_docs(qrels_q):
    """The set of docids judged relevant (rel >= 1) for one query."""
    return {d for d, r in qrels_q.items() if r >= 1}


def recall_at_k(run, qrels, k):
    """Mean Recall@k of `run` vs `qrels` (BEIR's primary recall metric).

    Recall@k for a query = |retrieved[:k] ∩ relevant| / |relevant|, averaged over the
    queries that HAVE at least one relevant doc in qrels (a query with no positive
    judgement has an undefined recall and is skipped — the BEIR convention). Returns
    (recall: float, n_queries: int). Raises if NO query has a positive judgement (so a
    mis-aligned qrels/run pair fails loudly, not silently as recall 0)."""
    qids = [q for q in qrels if _relevant_docs(qrels[q])]
    if not qids:
        raise ValueError("no query in qrels has a relevant (rel>=1) judgement")
    total = 0.0
    for q in qids:
        rel = _relevant_docs(qrels[q])
        topk = run.get(q, [])[:k]
        total += len(rel.intersection(topk)) / len(rel)
    return total / len(qids), len(qids)


def _dcg(gains):
    """DCG of an ordered list of gains: sum_i gain_i / log2(i + 2) (0-based i)."""
    return sum(g / math.log2(i + 2) for i, g in enumerate(gains))


def ndcg_at_k(run, qrels, k):
    """Mean nDCG@k of `run` vs `qrels` with GRADED relevance (the standard BEIR nDCG).

    For each query: DCG@k over the run's top-k (gain = the qrels relevance grade, 0 for
    an unjudged/non-relevant doc) divided by the IDCG@k (DCG of the top-k highest grades
    available for that query). Averaged over queries with at least one positive
    judgement. Returns (ndcg: float, n_queries: int). Raises if no query is judged."""
    qids = [q for q in qrels if _relevant_docs(qrels[q])]
    if not qids:
        raise ValueError("no query in qrels has a relevant (rel>=1) judgement")
    total = 0.0
    for q in qids:
        judged = qrels[q]
        gains = [judged.get(d, 0) for d in run.get(q, [])[:k]]
        ideal = sorted((g for g in judged.values() if g >= 1), reverse=True)[:k]
        idcg = _dcg(ideal)
        if idcg > 0:
            total += _dcg(gains) / idcg
    return total / len(qids), len(qids)


def deficit_milli(metric):
    """(1 - metric) scaled to integer milli-units — the smaller-is-better metric the
    mode:"auto" ratchet wants (G4). Clamped to [0, 1000] (a metric is in [0,1])."""
    d = int(round((1.0 - metric) * 1000))
    return max(0, min(1000, d))


# --- sparq-side input conversion (pure; the apples-to-apples bridge) ----------
# The sparq-text `beir_text` example takes plain inputs (NO JSON dep): an N-Triples
# corpus `<beir:doc:DOCID> <beir:text> "<text>" .` and a `<qid>\t<text>` queries TSV.
# These two pure writers convert a BEIR {docid: text} corpus + {qid: text} queries into
# those formats so sparq-text is retrieved on the SAME cut the Anserini oracle is, and
# scored by the SAME recall_at_k/ndcg_at_k against the SAME qrels (design §3.4 fairness).
DOC_IRI_PREFIX = "beir:doc:"
TEXT_PREDICATE = "beir:text"


def escape_nt_literal(s):
    r"""Escape a string for an N-Triples double-quoted literal (\\, ", \n, \r, \t).

    Per the N-Triples grammar's ECHAR set — enough for arbitrary BEIR document text
    (which is plain UTF-8; the byte-level parser accepts non-ASCII verbatim)."""
    return (
        s.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\n", "\\n")
        .replace("\r", "\\r")
        .replace("\t", "\\t")
    )


def corpus_to_ntriples(corpus):
    """A BEIR corpus {docid: text} -> N-Triples lines for the sparq-text beir_text example.

    `text` is the document's searchable body (title + text, joined, as BEIR/Anserini
    index it). Emits one `<beir:doc:DOCID> <beir:text> "<escaped>" .` line per doc, in
    sorted docid order for a deterministic file."""
    lines = []
    for docid in sorted(corpus):
        lines.append(
            '<%s%s> <%s> "%s" .' % (DOC_IRI_PREFIX, docid, TEXT_PREDICATE, escape_nt_literal(corpus[docid]))
        )
    return "\n".join(lines) + ("\n" if lines else "")


def queries_to_tsv(queries):
    """A BEIR queries {qid: text} -> `<qid>\\t<text>` TSV (newlines/tabs in the query
    text stripped to single spaces so each query stays one line)."""
    lines = []
    for qid in sorted(queries):
        text = " ".join(queries[qid].split())
        lines.append("%s\t%s" % (qid, text))
    return "\n".join(lines) + ("\n" if lines else "")


def qrels_to_trec(qrels):
    """A qrels {qid:{docid:rel}} -> TREC `<qid> 0 <docid> <rel>` lines (sorted), the
    format parse_qrels reads back and the sparq-text run is scored against."""
    lines = []
    for qid in sorted(qrels):
        for docid in sorted(qrels[qid]):
            lines.append("%s 0 %s %d" % (qid, docid, int(qrels[qid][docid])))
    return "\n".join(lines) + ("\n" if lines else "")


# --- heavy harness (gather-only; needs pyserini + a downloaded BEIR cut) ------
# BEIR cuts wired (the design's recommended small redistributable-at-gather cuts).
# pyserini ships pre-built Anserini Lucene indexes for these under `beir-v1.0.0-<id>`,
# and the BEIR qrels are fetched by the `beir` package at gather time. Both are
# Apache-2.0 / CC and downloaded ON the gather box (never committed in-repo).
BEIR_CUTS = {
    "scifact": "beir-v1.0.0-scifact.flat",
    "trec-covid": "beir-v1.0.0-trec-covid.flat",
}


def download_beir_cut(dataset):
    """Download + load a BEIR cut -> (corpus, queries, qrels). Gather-only.

    Lazily imports the `beir` data loader so CI can import this module (and unit-test
    the pure scorers/converters) WITHOUT the heavy dep. `corpus` is {docid: text} (BEIR's
    title + text joined — what Anserini indexes), `queries` is {qid: text}, `qrels` is
    {qid:{docid:rel}}. The download lands under BEIR_DATA_DIR (default /tmp/beir-data),
    never committed in-repo (the corpus is not redistributable)."""
    if dataset not in BEIR_CUTS:
        raise ValueError("unknown BEIR cut %r (have: %s)" % (dataset, ", ".join(sorted(BEIR_CUTS))))
    import os

    from beir import util
    from beir.datasets.data_loader import GenericDataLoader

    url = "https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/%s.zip" % dataset
    out_dir = os.path.join(os.environ.get("BEIR_DATA_DIR", "/tmp/beir-data"), dataset)
    data_path = util.download_and_unzip(url, out_dir)
    corpus_raw, queries, qrels_raw = GenericDataLoader(data_folder=data_path).load(split="test")
    # BEIR corpus values are {"title":..., "text":...}; join them the way Anserini's
    # default BEIR indexing does so sparq-text indexes the SAME searchable body.
    corpus = {
        did: (("%s %s" % (d.get("title", ""), d.get("text", ""))).strip()) for did, d in corpus_raw.items()
    }
    qrels = {qid: {d: int(r) for d, r in rels.items()} for qid, rels in qrels_raw.items()}
    return corpus, queries, qrels


def run_anserini_beir(dataset, k=100, bm25_k1=1.2, bm25_b=0.75):
    """Download a BEIR cut, BM25-retrieve top-k with Anserini, return (run, qrels).

    Gather-only: imports pyserini (the Anserini Python binding) lazily so CI can import
    this module (and unit-test the scorers above) WITHOUT the heavy dep. BM25 is pinned
    to k1=1.2 / b=0.75 to match sparq-text (a fair kernel comparison; design §3.4).
    Returns the run as {qid:[docid ranked]} and the qrels as {qid:{docid:rel}} — both fed
    straight into recall_at_k / ndcg_at_k against the SAME judgements sparq-text is
    scored on."""
    from pyserini.search.lucene import LuceneSearcher

    _corpus, queries, qrels = download_beir_cut(dataset)
    searcher = LuceneSearcher.from_prebuilt_index(BEIR_CUTS[dataset])
    searcher.set_bm25(bm25_k1, bm25_b)
    run = {}
    for qid, qtext in queries.items():
        hits = searcher.search(qtext, k=k)
        run[qid] = [h.docid for h in hits]
    return run, qrels


def prepare_sparq_inputs(dataset, out_dir):
    """Download a BEIR cut and write the sparq-text `beir_text` example's inputs to
    `out_dir` (corpus.nt, queries.tsv, qrels.txt) so sparq-text is retrieved + scored on
    the SAME cut + qrels the Anserini oracle is. Gather-only. Returns the three paths."""
    import os

    corpus, queries, qrels = download_beir_cut(dataset)
    os.makedirs(out_dir, exist_ok=True)
    paths = {
        "corpus": os.path.join(out_dir, "corpus.nt"),
        "queries": os.path.join(out_dir, "queries.tsv"),
        "qrels": os.path.join(out_dir, "qrels.txt"),
    }
    with open(paths["corpus"], "w", encoding="utf-8") as fh:
        fh.write(corpus_to_ntriples(corpus))
    with open(paths["queries"], "w", encoding="utf-8") as fh:
        fh.write(queries_to_tsv(queries))
    with open(paths["qrels"], "w", encoding="utf-8") as fh:
        fh.write(qrels_to_trec(qrels))
    return paths


def main(argv):
    """CLI:
      beir_ir_adapter.py --score --run <trec-run> --qrels <trec-qrels> [--engine N]
            offline: score a TREC run vs qrels (fixture-testable, NO pyserini/beir).
            ENGINE-AGNOSTIC: scores any TREC run — the Anserini oracle's OR sparq-text's
            (from the beir_text example) — against the SAME qrels (apples-to-apples).
      beir_ir_adapter.py --anserini --dataset scifact [--k 100] [--engine N]
            heavy (gather-only): download the BEIR cut, Anserini BM25 retrieve, score.
      beir_ir_adapter.py --prepare-sparq --dataset scifact --out <dir>
            heavy (gather-only): download the cut, write corpus.nt + queries.tsv +
            qrels.txt into <dir> for the sparq-text `beir_text` example (same cut/qrels).
    Score modes emit one line per metric `<engine>\\t<metric>\\t<deficit_milli>`
    (recall_at_100 + ndcg_at_10); --json adds the raw floats + k + cut id."""
    mode = None
    run_f = qrels_f = dataset = out_dir = None
    k_recall = 100
    k_ndcg = 10
    engine = "lucene-anserini"
    want_json = False
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--score":
            mode = "score"
        elif a == "--anserini":
            mode = "anserini"
        elif a == "--prepare-sparq":
            mode = "prepare-sparq"
        elif a == "--run":
            i += 1
            run_f = argv[i]
        elif a == "--qrels":
            i += 1
            qrels_f = argv[i]
        elif a == "--dataset":
            i += 1
            dataset = argv[i]
        elif a == "--out":
            i += 1
            out_dir = argv[i]
        elif a == "--k":
            i += 1
            k_recall = int(argv[i])
        elif a == "--engine":
            i += 1
            engine = argv[i]
        elif a == "--json":
            want_json = True
        else:
            sys.stderr.write("beir_ir_adapter: unknown arg %s\n" % a)
            return 2
        i += 1

    # The prepare-sparq mode writes files and exits (it produces no metric line).
    if mode == "prepare-sparq":
        try:
            if not dataset or not out_dir:
                raise ValueError("--prepare-sparq needs --dataset <cut> --out <dir>")
            paths = prepare_sparq_inputs(dataset, out_dir)
        except Exception as e:  # noqa: BLE001 — adapter boundary
            sys.stderr.write("beir_ir_adapter: %s\n" % e)
            return 1
        sys.stderr.write("wrote %s\n" % json.dumps(paths))
        return 0

    try:
        if mode == "score":
            if not (run_f and qrels_f):
                raise ValueError("--score needs --run <trec-run> --qrels <trec-qrels>")
            with open(run_f, encoding="utf-8") as fh:
                run = parse_run(fh.read())
            with open(qrels_f, encoding="utf-8") as fh:
                qrels = parse_qrels(fh.read())
        elif mode == "anserini":
            if not dataset:
                raise ValueError("--anserini needs --dataset <scifact|trec-covid>")
            run, qrels = run_anserini_beir(dataset, k=k_recall)
        else:
            sys.stderr.write("beir_ir_adapter: pick --score, --anserini or --prepare-sparq\n")
            return 2
        recall, _nr = recall_at_k(run, qrels, k_recall)
        ndcg, _nn = ndcg_at_k(run, qrels, k_ndcg)
    except Exception as e:  # noqa: BLE001 — adapter boundary
        sys.stderr.write("beir_ir_adapter: %s\n" % e)
        return 1

    sys.stdout.write("%s\trecall_at_%d\t%d\n" % (engine, k_recall, deficit_milli(recall)))
    sys.stdout.write("%s\tndcg_at_%d\t%d\n" % (engine, k_ndcg, deficit_milli(ndcg)))
    if want_json:
        sys.stderr.write(
            json.dumps(
                {
                    "engine": engine,
                    "recall_at_k": recall,
                    "recall_k": k_recall,
                    "ndcg_at_k": ndcg,
                    "ndcg_k": k_ndcg,
                    "beir_cut": dataset,
                }
            )
            + "\n"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
