#!/usr/bin/env python3
# sq-eifd: PYTHON-LIB (vector-lib) adapter KIND.
#
# In-process, cross-language ANN harness — the vector analogue of the embedded-Rust
# Oxigraph model, but Python. Covers FAISS / hnswlib / Lucene-via-Anserini
# (research/capability-benchmark-program.md §3.3 + G3 `python-lib`). The deliverable
# shape: build index -> query -> emit recall@k + latency TSV + an EXACT-kNN
# correctness oracle (G4 — ANN is approximate, so the gate is recall vs exact-kNN,
# not a row-count diff).
#
# TWO pieces, separable so the gather box can run the heavy half and CI can unit-
# test the light half:
#   1. recall_at_k(approx_ids, exact_ids, k)  — the SCORING/ORACLE. Pure function:
#      mean over queries of |approx_topk ∩ exact_topk| / k. This is fixture-unit-
#      tested (a captured neighbour TSV) WITHOUT FAISS/hnswlib installed.
#   2. exact_knn / run_hnswlib — the heavy harness: build the index, query, compute
#      the exact-kNN ground truth with numpy (the oracle), emit recall@k + latency.
#      Requires numpy (+ hnswlib for the ANN engine); gather-only.
#
# Output: `<engine>\t<recall_deficit_milli>\t<query_us>` TSV on stdout, where
# recall_deficit_milli = round((1 - recall@k) * 1000) — emitted as a DEFICIT so it
# slots into the smaller-is-better mode:"auto" ratchet with zero perf-gate change
# (the G4 trick). --json carries the raw recall + k for the dashboard.
#
# NEIGHBOUR-TSV format (what an external ANN tool dumps, and the fixture format):
#   one line per query: `<query_id>\t<id1>,<id2>,...,<idk>`  (ranked neighbour ids).
# recall_at_k consumes two such maps (approx + exact) and scores them.
#
# --- GATHER-TIER: SIFT1M / GloVe-100-angular recall-QPS Pareto (sq-aiup) ------
# sq-aiup. The per-commit suite (bench/vector/) gates the SYNTHETIC 50k
# corpus recall deficits. The big PUBLISHED-dataset recall-QPS Pareto is gather-tier:
# the corpora (SIFT1M = ann-benchmarks `sift-128-euclidean`; `glove-100-angular`) are
# NOT redistributable in-repo, so this is a download/gather step (nightly/EC2), never a
# per-PR gate (design research/capability-benchmark-program.md §3.3(c)).
#
# A single (recall, latency) point is MEANINGLESS for ANN — a faster engine at lower
# recall is not "faster". The deliverable is the recall-QPS PARETO at MATCHED recall:
# sweep the search-effort knob (hnswlib `ef`), and for EACH setting emit one
# `(recall_deficit, qps)` point. The Pareto FRONTIER (pareto_frontier) is the
# published-comparable curve; matched_recall_qps reads QPS off it at a target recall so
# two engines are compared at the SAME recall, never at a single latency.
#
# THREE pieces, split so the light half is fixture-unit-tested WITHOUT numpy/hnswlib:
#   1. recall_at_k / parse_neighbour_tsv      — scoring/oracle (pure).            [light]
#   2. read_fvecs / read_ivecs / pareto_frontier / matched_recall_qps /
#      qps_from_query_us                       — dataset parse + curve maths (pure). [light]
#   3. load_dataset / run_hnswlib_sweep        — the heavy harness: read the corpus,
#      build/query the engine over a param sweep, exact-kNN oracle.   [gather-only/heavy]
import json
import os
import struct
import sys
import time


def parse_neighbour_tsv(text):
    """`<qid>\\t<csv of ranked neighbour ids>` per line -> {qid: [ids...]}.

    Blank lines and `#`-comments are skipped. Ids are kept as strings (ids may be
    dict-encoded entity ids in sparq's case)."""
    out = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 2:
            raise ValueError("bad neighbour-tsv line: %r" % line)
        qid, ids = parts
        out[qid] = [t for t in ids.split(",") if t != ""]
    return out


def recall_at_k(approx, exact, k):
    """Mean recall@k of `approx` vs the `exact` ground truth (both {qid:[ids]}).

    recall@k for a query = |approx[:k] ∩ exact[:k]| / k, averaged over the queries
    present in BOTH maps. Returns (recall: float, n_queries: int). Raises if there
    is no overlapping query (so a mis-aligned fixture fails loudly, not silently
    as recall 0)."""
    qids = [q for q in exact if q in approx]
    if not qids:
        raise ValueError("no query ids common to approx and exact neighbour sets")
    total = 0.0
    for q in qids:
        a = set(approx[q][:k])
        e = set(exact[q][:k])
        denom = min(k, len(e)) or 1
        total += len(a & e) / denom
    return total / len(qids), len(qids)


def recall_deficit_milli(recall):
    """(1 - recall) scaled to integer milli-units — the smaller-is-better metric
    the mode:"auto" ratchet wants (G4)."""
    return int(round((1.0 - recall) * 1000))


# --- published-dataset parse + curve maths (pure, fixture-tested) ------------
# sq-aiup. These are stdlib-only so they are unit-tested WITHOUT numpy.
# `.fvecs`/`.ivecs` is the TEXMEX/SIFT1M on-disk format (http://corpus-texmex.irisa.fr):
# each record is a little-endian int32 dimension `d`, then `d` little-endian values
# (float32 for .fvecs, int32 for .ivecs). GloVe-100-angular ships as ann-benchmarks
# HDF5 instead (loaded via h5py in the heavy half).
def read_vecs(raw, value_fmt):
    """Decode a TEXMEX `.fvecs`/`.ivecs` byte string -> list of equal-length rows.

    `value_fmt` is "f" (float32, .fvecs) or "i" (int32, .ivecs). Each record is
    `<int32 d><d values>`; `d` must be constant across records (raises otherwise, so
    a truncated/mismatched file fails loudly rather than yielding ragged rows)."""
    if value_fmt not in ("f", "i"):
        raise ValueError("value_fmt must be 'f' or 'i'")
    rows = []
    off = 0
    n = len(raw)
    dim = None
    while off < n:
        if off + 4 > n:
            raise ValueError("truncated .vecs record header at byte %d" % off)
        (d,) = struct.unpack_from("<i", raw, off)
        if d < 0:
            raise ValueError("negative dimension %d in .vecs at byte %d" % (d, off))
        if dim is None:
            dim = d
        elif d != dim:
            raise ValueError("ragged .vecs: dim %d != %d at byte %d" % (d, dim, off))
        off += 4
        end = off + 4 * d
        if end > n:
            raise ValueError("truncated .vecs row at byte %d (need %d bytes)" % (off, 4 * d))
        rows.append(list(struct.unpack_from("<%d%s" % (d, value_fmt), raw, off)))
        off = end
    return rows


def read_fvecs(path):
    """Read a `.fvecs` file (SIFT1M base/query vectors) -> list[list[float]]."""
    with open(path, "rb") as fh:
        return read_vecs(fh.read(), "f")


def read_ivecs(path):
    """Read a `.ivecs` file (SIFT1M ground-truth neighbour ids) -> list[list[int]]."""
    with open(path, "rb") as fh:
        return read_vecs(fh.read(), "i")


def qps_from_query_us(query_us):
    """Per-query microseconds -> queries-per-second (0 for non-positive latency)."""
    return (1e6 / query_us) if query_us > 0 else 0.0


def pareto_frontier(points):
    """Recall-QPS Pareto frontier of `points` = list of (recall, qps) tuples.

    A point is on the frontier if NO other point dominates it (>= on BOTH recall and
    qps, and strictly greater on at least one). Returns the frontier sorted by ascending
    recall — the published-comparable curve (higher-and-to-the-right is better). Ties
    (identical recall AND qps) collapse to one point."""
    uniq = sorted(set((float(r), float(q)) for r, q in points))
    front = []
    for r, q in uniq:
        dominated = any(
            (or_ >= r and oq >= q) and (or_ > r or oq > q) for or_, oq in uniq if (or_, oq) != (r, q)
        )
        if not dominated:
            front.append((r, q))
    return front


def matched_recall_qps(points, target_recall):
    """QPS at MATCHED recall: the best QPS achievable at recall >= `target_recall`.

    Reads the answer off the recall-QPS frontier — this is the ONLY honest cross-engine
    number (two engines compared at the SAME recall floor, never a single latency). The
    max over all frontier points clearing the floor (a higher-recall, higher-qps point
    also satisfies a lower floor). Returns None if no point reaches the target recall
    (the engine simply cannot hit that recall on this dataset)."""
    qualifying = [q for r, q in pareto_frontier(points) if r >= target_recall]
    return max(qualifying) if qualifying else None


# --- heavy harness (gather-only; needs numpy [+ hnswlib]) --------------------
def exact_knn(data, queries, k):
    """numpy exact L2 kNN ground truth -> {qid:[ids]} with qid/ids as str(index)."""
    import numpy as np  # local import: oracle is gather-only

    data = np.asarray(data, dtype="float32")
    queries = np.asarray(queries, dtype="float32")
    out = {}
    for qi, q in enumerate(queries):
        d = ((data - q) ** 2).sum(axis=1)
        idx = np.argsort(d)[:k]
        out[str(qi)] = [str(int(i)) for i in idx]
    return out


def run_hnswlib(data, queries, k, ef=64, m=16, ef_construction=200):
    """Build an hnswlib index, query top-k, time it -> ({qid:[ids]}, query_us)."""
    import hnswlib  # local import: gather-only
    import numpy as np

    data = np.asarray(data, dtype="float32")
    queries = np.asarray(queries, dtype="float32")
    dim = data.shape[1]
    index = hnswlib.Index(space="l2", dim=dim)
    index.init_index(max_elements=len(data), ef_construction=ef_construction, M=m)
    index.add_items(data, list(range(len(data))))
    index.set_ef(ef)
    t0 = time.perf_counter()
    labels, _ = index.knn_query(queries, k=k)
    query_us = (time.perf_counter() - t0) * 1e6 / max(1, len(queries))
    approx = {str(qi): [str(int(i)) for i in row] for qi, row in enumerate(labels)}
    return approx, int(round(query_us))


# --- published-dataset loaders + Pareto sweep (gather-only; sq-aiup) ---------
def load_dataset(name, root):
    """Load a published ANN dataset -> (data, queries, space, ground_truth-or-None).

    Two on-disk layouts (the corpora are gather-only, NOT redistributable in-repo):
      * SIFT1M (`sift`, `sift-128-euclidean`): the TEXMEX .fvecs/.ivecs files under
        `<root>/sift/` — sift_base.fvecs, sift_query.fvecs, and the precomputed
        sift_groundtruth.ivecs (used as the exact-kNN oracle, so no numpy recompute).
        L2 space.
      * GloVe-100-angular (`glove`, `glove-100-angular`): the ann-benchmarks HDF5 file
        `<root>/glove-100-angular.hdf5` with train/test/neighbors datasets (needs h5py).
        Angular (cosine) space; hnswlib uses `cosine`.
    Returns ground_truth as {qid:[ids]} when the dataset ships it (SIFT/GloVe both do),
    else None (caller computes exact_knn)."""
    import numpy as np  # gather-only

    key = name.lower()
    if key in ("sift", "sift1m", "sift-128-euclidean"):
        base = os.path.join(root, "sift")
        data = np.asarray(read_fvecs(os.path.join(base, "sift_base.fvecs")), dtype="float32")
        queries = np.asarray(read_fvecs(os.path.join(base, "sift_query.fvecs")), dtype="float32")
        gt = None
        gt_path = os.path.join(base, "sift_groundtruth.ivecs")
        if os.path.exists(gt_path):
            gt = {str(qi): [str(i) for i in row] for qi, row in enumerate(read_ivecs(gt_path))}
        return data, queries, "l2", gt
    if key in ("glove", "glove-100-angular", "glove100"):
        import h5py  # gather-only

        path = os.path.join(root, "glove-100-angular.hdf5")
        with h5py.File(path, "r") as fh:
            data = np.asarray(fh["train"], dtype="float32")
            queries = np.asarray(fh["test"], dtype="float32")
            gt = {
                str(qi): [str(int(i)) for i in row] for qi, row in enumerate(np.asarray(fh["neighbors"]))
            }
        return data, queries, "cosine", gt
    raise ValueError("unknown dataset %r (use sift-128-euclidean or glove-100-angular)" % name)


def run_hnswlib_sweep(data, queries, k, space, ef_values, m=16, ef_construction=200, ground_truth=None):
    """Build ONE hnswlib index and query it at a sweep of `ef` search-effort settings.

    Yields one Pareto point per ef: (ef, recall@k, query_us, qps). The exact-kNN oracle
    is `ground_truth` if supplied (the dataset's precomputed neighbours), else computed
    once with numpy. Higher ef -> higher recall + higher latency: this IS the recall-QPS
    trade-off curve the ann-benchmarks Pareto plots."""
    import hnswlib
    import numpy as np

    data = np.asarray(data, dtype="float32")
    queries = np.asarray(queries, dtype="float32")
    dim = data.shape[1]
    index = hnswlib.Index(space=space, dim=dim)
    index.init_index(max_elements=len(data), ef_construction=ef_construction, M=m)
    index.add_items(data, list(range(len(data))))
    exact = ground_truth if ground_truth is not None else exact_knn(data, queries, k)
    out = []
    for ef in ef_values:
        index.set_ef(int(ef))
        t0 = time.perf_counter()
        labels, _ = index.knn_query(queries, k=k)
        query_us = (time.perf_counter() - t0) * 1e6 / max(1, len(queries))
        approx = {str(qi): [str(int(i)) for i in row] for qi, row in enumerate(labels)}
        recall, _n = recall_at_k(approx, exact, k)
        out.append((int(ef), recall, int(round(query_us)), qps_from_query_us(query_us)))
    return out


def main(argv):
    """CLI:
      vector_lib_adapter.py --smoke
            light self-test: exercises recall_at_k, pareto_frontier, matched_recall_qps,
            read_vecs (pure functions — no numpy/hnswlib). Exits 0 on pass, 1 on failure.
      vector_lib_adapter.py --score --approx <tsv> --exact <tsv> --k K [--engine N]
            offline: score two neighbour TSVs (fixture-testable, no numpy/hnswlib).
      vector_lib_adapter.py --hnswlib --npz <file.npz> --k K [--engine N]
            heavy: build hnswlib index over npz {data,queries}, score vs exact-kNN.
      vector_lib_adapter.py --pareto --dataset sift-128-euclidean|glove-100-angular
            --root <dir> --k K [--ef 32,64,...] [--engine N] [--match-recall R]
            gather-tier (sq-aiup): build ONE hnswlib index over the published corpus,
            sweep `ef`, and emit the recall-QPS Pareto. One `<engine>\\t<recall_deficit_milli>
            \\t<query_us>\\t<qps>\\t<ef>` line PER ef point; --json adds the frontier +
            QPS-at-matched-recall (the only honest cross-engine number).
    Emits `<engine>\\t<recall_deficit_milli>\\t<query_us>`; --json adds raw recall."""
    mode = None
    approx_f = exact_f = npz_f = None
    dataset = root = None
    ef_values = [16, 32, 64, 128, 256]
    match_recall = 0.9
    k = 10
    engine = "engine"
    want_json = False
    query_us = 0
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--smoke":
            mode = "smoke"
        elif a == "--score":
            mode = "score"
        elif a == "--hnswlib":
            mode = "hnswlib"
        elif a == "--pareto":
            mode = "pareto"
        elif a == "--approx":
            i += 1
            approx_f = argv[i]
        elif a == "--exact":
            i += 1
            exact_f = argv[i]
        elif a == "--npz":
            i += 1
            npz_f = argv[i]
        elif a == "--dataset":
            i += 1
            dataset = argv[i]
        elif a == "--root":
            i += 1
            root = argv[i]
        elif a == "--ef":
            i += 1
            ef_values = [int(x) for x in argv[i].split(",") if x != ""]
        elif a == "--match-recall":
            i += 1
            match_recall = float(argv[i])
        elif a == "--k":
            i += 1
            k = int(argv[i])
        elif a == "--engine":
            i += 1
            engine = argv[i]
        elif a == "--json":
            want_json = True
        else:
            sys.stderr.write("vector_lib_adapter: unknown arg %s\n" % a)
            return 2
        i += 1

    try:
        if mode == "smoke":
            # sq-hmd7l.19: lightweight self-test for the pure (no-numpy/hnswlib)
            # functions — recall_at_k, pareto_frontier, matched_recall_qps, read_vecs.
            # Exits 0 on pass, 1 on failure.
            import struct as _struct
            _ok = True

            def _check(label, got, want):
                if got != want:
                    sys.stderr.write("smoke FAIL %s: got %r want %r\n" % (label, got, want))
                    nonlocal _ok
                    _ok = False

            # recall_at_k: trivial perfect recall
            approx_sm = {"0": ["a", "b", "c"], "1": ["d", "e", "f"]}
            exact_sm  = {"0": ["a", "b", "c"], "1": ["d", "e", "f"]}
            r, n = recall_at_k(approx_sm, exact_sm, 3)
            _check("smoke.recall.perfect", abs(r - 1.0) < 1e-9, True)
            _check("smoke.recall.n", n, 2)

            # recall_at_k: zero overlap
            approx_z = {"0": ["x", "y", "z"]}
            r_z, _ = recall_at_k(approx_z, {"0": ["a", "b", "c"]}, 3)
            _check("smoke.recall.zero", abs(r_z) < 1e-9, True)

            # pareto_frontier: a dominated point is removed
            pts_sm = [(0.95, 300.0), (0.99, 500.0), (0.999, 100.0)]
            front_sm = pareto_frontier(pts_sm)
            _check("smoke.pareto.len", len(front_sm), 2)
            _check("smoke.pareto.has_099", any(abs(r - 0.99) < 1e-9 for r, _ in front_sm), True)

            # matched_recall_qps: returns best QPS at floor, None when unreachable
            _check("smoke.matched.0_9", matched_recall_qps(pts_sm, 0.9), 500.0)
            _check("smoke.matched.none", matched_recall_qps(pts_sm, 1.0), None)

            # read_vecs: round-trip a tiny .fvecs payload
            _raw = _struct.pack("<i", 2) + _struct.pack("<2f", 1.0, 2.0)
            _rows = read_vecs(_raw, "f")
            _check("smoke.read_vecs.row", _rows, [[1.0, 2.0]])

            if _ok:
                sys.stdout.write("vector_lib_adapter smoke OK\n")
                return 0
            return 1
        elif mode == "score":
            if not (approx_f and exact_f):
                raise ValueError("--score needs --approx <tsv> --exact <tsv>")
            with open(approx_f, encoding="utf-8") as fh:
                approx = parse_neighbour_tsv(fh.read())
            with open(exact_f, encoding="utf-8") as fh:
                exact = parse_neighbour_tsv(fh.read())
            recall, _n = recall_at_k(approx, exact, k)
        elif mode == "hnswlib":
            import numpy as np

            if not npz_f:
                raise ValueError("--hnswlib needs --npz <file.npz {data,queries}>")
            npz = np.load(npz_f)
            approx, query_us = run_hnswlib(npz["data"], npz["queries"], k)
            exact = exact_knn(npz["data"], npz["queries"], k)
            recall, _n = recall_at_k(approx, exact, k)
        elif mode == "pareto":
            if not (dataset and root):
                raise ValueError("--pareto needs --dataset <name> --root <dir>")
            data, queries, space, gt = load_dataset(dataset, root)
            sweep = run_hnswlib_sweep(data, queries, k, space, ef_values, ground_truth=gt)
            # One TSV row per ef point (the recall-QPS curve, never a single latency).
            for ef, rec, qus, qps in sweep:
                sys.stdout.write(
                    "%s\t%d\t%d\t%.2f\t%d\n" % (engine, recall_deficit_milli(rec), qus, qps, ef)
                )
            if want_json:
                pts = [(rec, qps) for _ef, rec, _qus, qps in sweep]
                front = pareto_frontier(pts)
                sys.stderr.write(
                    json.dumps(
                        {
                            "engine": engine,
                            "dataset": dataset,
                            "k": k,
                            "space": space,
                            "points": [
                                {"ef": ef, "recall_at_k": rec, "query_us": qus, "qps": qps}
                                for ef, rec, qus, qps in sweep
                            ],
                            "pareto_frontier": [{"recall_at_k": r, "qps": q} for r, q in front],
                            "match_recall": match_recall,
                            "qps_at_matched_recall": matched_recall_qps(pts, match_recall),
                        }
                    )
                    + "\n"
                )
            return 0
        else:
            sys.stderr.write("vector_lib_adapter: pick --smoke, --score, --hnswlib or --pareto\n")
            return 2
    except Exception as e:  # noqa: BLE001 — adapter boundary
        sys.stderr.write("vector_lib_adapter: %s\n" % e)
        return 1

    sys.stdout.write("%s\t%d\t%d\n" % (engine, recall_deficit_milli(recall), query_us))
    if want_json:
        sys.stderr.write(
            json.dumps({"engine": engine, "recall_at_k": recall, "k": k, "query_us": query_us})
            + "\n"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
