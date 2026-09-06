#!/usr/bin/env python3
# [SONNET-4.6] sq-hmd7l.13 — igraph / NetworKit leg of the LDBC Graphalytics comparison.
#
# Reads a dataset in the Graphalytics on-disk format, runs ONE algorithm on the named
# engine, and writes the result in the Graphalytics reference-output format so
# `bench/graphalytics/gx.py validate` gates it with the SAME oracle sparq is gated with.
# Timing is emitted as `metric_us algo=<us>` (best of --iters) and is only meaningful once
# that gate is green.
#
# HONESTY — TERMINATION SEMANTICS DIFFER, AND THAT IS REPORTED, NOT HIDDEN.
#   Graphalytics PageRank is defined as a FIXED number of power-method sweeps.
#     * igraph's `Graph.pagerank()` does not expose that: it SOLVES for the stationary
#       distribution (PRPACK). It therefore cannot reproduce a 10-sweep reference, and the
#       adapter says so via `semantics=converged`. There is no converged oracle in this
#       suite, and the fixed-sweep reference is NOT one — gating igraph against it would
#       print a large per-vertex delta that reads as an igraph correctness failure when it
#       is only a difference of termination rule. So the harness does not gate that row at
#       all: it records an incomparable SEMANTIC-GAP and prints no timing. Timing igraph's
#       exact solve against sparq's 10 sweeps would not be an iteration-matched comparison
#       either way.
#     * NetworKit's `centrality.PageRank` exposes `maxIterations`, so it CAN run the
#       fixed-sweep form; the adapter sets it and reports `semantics=fixed-iterations`.
#   WCC has no such ambiguity on any engine — that row is directly comparable.
#
# NOT-COMPARABLE axis, stated once here and again in the gap record: igraph and NetworKit
# are embedded graph libraries fed a prepared edge array. sparq-algos runs over an RDF
# triple store's dictionary ids. The harness reports sparq's export-to-edge-list cost
# separately so the "no export needed" claim can be checked rather than asserted.
#
# Neither library is a repo dependency; both are gather-time only. If the requested engine
# is not importable the adapter exits 4 (SKIPPED) and the harness records an honest skip
# rather than a number.
#
# Usage:
#   graph_analytics_adapter.py --engine igraph|networkit --dataset DIR [--name N] \
#       --algo pr|wcc|cdlp --out FILE [--iters 3]
import argparse
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
GX = os.path.join(os.path.dirname(HERE), "..", "bench", "graphalytics")
sys.path.insert(0, os.path.abspath(GX))

import gx  # noqa: E402  (the shared dataset loader + reference oracle)

# Exit code meaning "the engine is not installed" — distinct from a real failure so the
# harness can print SKIPPED instead of inventing a row.
EXIT_SKIPPED = 4


def load_dense(dataset_dir, name):
    """Loads the dataset and remaps vertex ids onto a dense `0..n-1` range.

    Both libraries index vertices by position, so the mapping (and its inverse, used to
    write Graphalytics-format output back out) is unavoidable. It is built BEFORE the
    timed region: it is dataset preparation, not algorithm work.
    """
    props, vertices, out_adj, _in_adj = gx.load_graph(dataset_dir, name)
    index = {v: i for i, v in enumerate(vertices)}
    edges = [(index[s], index[o]) for s in vertices for o in out_adj[s]]
    return props, vertices, index, edges


def best_of(fn, iters):
    """Runs `fn` `iters` times, returning `(result, best_microseconds)`."""
    best = None
    result = None
    for _ in range(max(1, iters)):
        t = time.perf_counter()
        result = fn()
        elapsed = int((time.perf_counter() - t) * 1e6)
        best = elapsed if best is None else min(best, elapsed)
    return result, best


def run_igraph(args, props, vertices, edges):
    try:
        import igraph
    except ImportError:
        sys.stderr.write("[adapter] SKIPPED: python-igraph is not installed (pip install python-igraph)\n")
        raise SystemExit(EXIT_SKIPPED)

    directed = props.get("directed", "true").strip().lower() == "true"
    g = igraph.Graph(n=len(vertices), edges=edges, directed=directed)
    print("engine=igraph version=%s" % igraph.__version__)

    if args.algo == "pr":
        damping = float(props["pr.damping-factor"])
        # PRPACK solves for the stationary distribution; there is no fixed-sweep knob. The
        # harness reads `semantics=converged` and declines to gate this row against the
        # fixed-sweep reference rather than reporting a meaningless mismatch.
        scores, us = best_of(lambda: g.pagerank(damping=damping, directed=directed), args.iters)
        print("semantics=converged")
        return {v: scores[i] for i, v in enumerate(vertices)}, us, True
    if args.algo == "wcc":
        labels, us = best_of(lambda: g.connected_components(mode="weak").membership, args.iters)
        return relabel_to_min_vertex(vertices, labels), us, False
    if args.algo == "cdlp":
        # igraph's label propagation is the RANDOMISED Raghavan variant, not Graphalytics
        # CDLP. Reported as such; the gate is expected to be red and is never suppressed.
        labels, us = best_of(lambda: g.community_label_propagation().membership, args.iters)
        print("semantics=randomised-label-propagation")
        return relabel_to_min_vertex(vertices, labels), us, False
    gx.die("igraph adapter: unsupported algo %s" % args.algo)


def run_networkit(args, props, vertices, edges):
    try:
        import networkit as nk
    except ImportError:
        sys.stderr.write("[adapter] SKIPPED: networkit is not installed (pip install networkit)\n")
        raise SystemExit(EXIT_SKIPPED)

    directed = props.get("directed", "true").strip().lower() == "true"
    g = nk.Graph(len(vertices), weighted=False, directed=directed)
    for s, o in edges:
        g.addEdge(s, o)
    print("engine=networkit version=%s" % nk.__version__)

    if args.algo == "pr":
        damping = float(props["pr.damping-factor"])
        iterations = int(props["pr.num-iterations"])

        def go():
            pr = nk.centrality.PageRank(g, damp=damping)
            # NetworKit DOES expose the sweep cap, so it can run the spec's fixed-iteration
            # form. `tol` is driven to 0 so the cap, not convergence, is what stops it.
            pr.maxIterations = iterations
            pr.tol = 0.0
            pr.run()
            return pr.scores()

        scores, us = best_of(go, args.iters)
        print("semantics=fixed-iterations")
        return {v: scores[i] for i, v in enumerate(vertices)}, us, True
    if args.algo == "wcc":

        def go():
            cc = nk.components.WeaklyConnectedComponents(g) if directed else nk.components.ConnectedComponents(g)
            cc.run()
            return cc.getPartition().getVector()

        labels, us = best_of(go, args.iters)
        return relabel_to_min_vertex(vertices, labels), us, False
    if args.algo == "cdlp":

        def go():
            plm = nk.community.PLP(g)
            plm.run()
            return plm.getPartition().getVector()

        labels, us = best_of(go, args.iters)
        print("semantics=asynchronous-label-propagation")
        return relabel_to_min_vertex(vertices, labels), us, False
    gx.die("networkit adapter: unsupported algo %s" % args.algo)


def relabel_to_min_vertex(vertices, membership):
    """Rewrites a positional membership vector into the Graphalytics convention: every
    group is labelled with the smallest ORIGINAL vertex id it contains. Pure relabelling —
    the partition is untouched, and the gate compares partitions anyway."""
    smallest = {}
    for i, v in enumerate(vertices):
        m = membership[i]
        if m not in smallest or v < smallest[m]:
            smallest[m] = v
    return {v: smallest[membership[i]] for i, v in enumerate(vertices)}


def main(argv):
    ap = argparse.ArgumentParser(description="igraph / NetworKit Graphalytics adapter")
    ap.add_argument("--engine", required=True, choices=("igraph", "networkit"))
    ap.add_argument("--dataset", required=True)
    ap.add_argument("--name")
    ap.add_argument("--algo", required=True, choices=("pr", "wcc", "cdlp"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--iters", type=int, default=3)
    args = ap.parse_args(argv)

    props, vertices, _index, edges = load_dense(args.dataset, args.name)
    runner = run_igraph if args.engine == "igraph" else run_networkit
    values, us, float_values = runner(args, props, vertices, edges)
    gx.write_output(args.out, values, float_values)
    print("metric_us algo=%d" % us)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
