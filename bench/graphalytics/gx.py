#!/usr/bin/env python3
# [SONNET-4.6] sq-hmd7l.13 — LDBC Graphalytics reference oracle + validation gate.
#
# TWO ROLES, one stdlib-only file:
#
#   1. `reference` — an INDEPENDENT implementation of the Graphalytics algorithm
#      specifications (LDBC Graphalytics specification, Iosup et al.), written from the
#      spec in a different language from the implementation under test. It is the oracle
#      the committed reference outputs in data/*/ are generated from and re-verified
#      against on every run, so those files are not "whatever sparq printed" — a circular
#      check that would validate nothing.
#
#   2. `validate` — the CORRECTNESS GATE. Compares one engine's output against a reference
#      output using the per-algorithm Graphalytics rule:
#        * PR          -> per-vertex floating-point agreement within a relative epsilon.
#        * WCC / CDLP  -> EQUIVALENCE OF PARTITIONING (the official rule): the labels need
#                         not be equal, but the induced partitions must be identical.
#      Non-zero exit = the gate is RED, and no timing from that engine may be believed.
#
# Nothing here times anything, and nothing here is allowed to "fix up" a mismatch.
#
# Usage:
#   gx.py reference --dataset DIR [--name N] --algo pr|wcc|cdlp [--iterations N] --out FILE
#   gx.py validate  --reference FILE --actual FILE --algo pr|wcc|cdlp [--epsilon 1e-4]
#   gx.py info      --dataset DIR [--name N]
import argparse
import math
import os
import sys
from collections import Counter

# Relative tolerance for the floating-point (PageRank) gate. The LDBC validator compares
# doubles with a relative epsilon; 1e-4 is its documented default.
DEFAULT_EPSILON = 1e-4
# Absolute floor added to the relative bound so vertices whose reference score is ~0 are
# not held to an impossible relative tolerance.
ABS_FLOOR = 1e-12


# --------------------------------------------------------------------------------------
# Dataset loading (the LDBC Graphalytics on-disk format)
# --------------------------------------------------------------------------------------


def discover_name(dataset_dir):
    """The dataset name = the stem of the single *.properties file in the directory."""
    stems = sorted(
        os.path.splitext(f)[0]
        for f in os.listdir(dataset_dir)
        if f.endswith(".properties")
    )
    if len(stems) == 1:
        return stems[0]
    if not stems:
        die("no *.properties file in %s — pass --name" % dataset_dir)
    die("%d *.properties files in %s (%s) — pass --name" % (len(stems), dataset_dir, ", ".join(stems)))


def load_properties(dataset_dir, name):
    """Reads `<name>.properties`, returning the part of each key after `graph.<name>.`."""
    path = os.path.join(dataset_dir, name + ".properties")
    prefix = "graph.%s." % name
    props = {}
    with open(path) as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#") or line.startswith("!"):
                continue
            if "=" not in line:
                continue
            k, v = line.split("=", 1)
            k = k.strip()
            if k.startswith(prefix):
                props[k[len(prefix):]] = v.strip()
    return props


def load_graph(dataset_dir, name=None):
    """Loads a dataset into `(props, vertices, out_adj, in_adj)`.

    `vertices` is a sorted list of int ids; `out_adj`/`in_adj` map a vertex to a list of
    neighbours. An UNDIRECTED dataset lists each edge once in the `.e` file and is
    expanded to both directions here, exactly as the spec requires.
    """
    name = name or discover_name(dataset_dir)
    props = load_properties(dataset_dir, name)
    directed = props.get("directed", "true").strip().lower() == "true"
    vfile = os.path.join(dataset_dir, props.get("vertex-file", name + ".v"))
    efile = os.path.join(dataset_dir, props.get("edge-file", name + ".e"))

    vertices = []
    with open(vfile) as fh:
        for line in fh:
            parts = line.split()
            if parts:
                vertices.append(int(parts[0]))
    vertices.sort()

    out_adj = {v: [] for v in vertices}
    in_adj = {v: [] for v in vertices}

    def add(s, o):
        if s not in out_adj:
            die("edge endpoint %d is not in the vertex file %s" % (s, vfile))
        if o not in in_adj:
            die("edge endpoint %d is not in the vertex file %s" % (o, efile))
        out_adj[s].append(o)
        in_adj[o].append(s)

    with open(efile) as fh:
        for line in fh:
            parts = line.split()
            if len(parts) < 2:
                continue  # blank line; a third weight column is ignored (SSSP is a gap)
            s, o = int(parts[0]), int(parts[1])
            add(s, o)
            if not directed:
                add(o, s)

    return props, vertices, out_adj, in_adj


# --------------------------------------------------------------------------------------
# Reference implementations, written from the Graphalytics specification
# --------------------------------------------------------------------------------------


def reference_pr(vertices, out_adj, in_adj, damping, iterations):
    """Graphalytics PageRank.

        PR_0(v) = 1/n
        PR_i(v) = (1-d)/n + d * ( SUM_{u in Nin(v)} PR_{i-1}(u)/outdeg(u)
                                + SUM_{w : outdeg(w)=0} PR_{i-1}(w)/n )

    run for exactly `iterations` sweeps. Dangling vertices redistribute their mass
    uniformly, which is what keeps the vector a probability distribution.
    """
    n = len(vertices)
    if n == 0:
        return {}
    pr = {v: 1.0 / n for v in vertices}
    outdeg = {v: len(out_adj[v]) for v in vertices}
    dangling_vertices = [v for v in vertices if outdeg[v] == 0]
    teleport = (1.0 - damping) / n
    for _ in range(iterations):
        dangling = sum(pr[w] for w in dangling_vertices) / n
        nxt = {}
        for v in vertices:
            acc = 0.0
            for u in in_adj[v]:
                acc += pr[u] / outdeg[u]
            nxt[v] = teleport + damping * (acc + dangling)
        pr = nxt
    return pr


def reference_wcc(vertices, out_adj, in_adj):
    """Graphalytics WCC — weakly connected components, each labelled with the SMALLEST
    vertex id in the component (the reference-output convention)."""
    parent = {v: v for v in vertices}

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    def union(a, b):
        ra, rb = find(a), find(b)
        if ra != rb:
            # Union toward the smaller id so the root IS the component minimum.
            if rb < ra:
                ra, rb = rb, ra
            parent[rb] = ra

    for v in vertices:
        for w in out_adj[v]:
            union(v, w)
    return {v: find(v) for v in vertices}


def reference_cdlp(vertices, out_adj, in_adj, max_iterations):
    """Graphalytics CDLP — community detection by label propagation.

    Spec semantics, and every clause matters:
      * initial label of v is v itself;
      * updates are SYNCHRONOUS — every vertex reads the PREVIOUS iteration's labels;
      * both incoming and outgoing neighbours vote, so a reciprocal edge votes TWICE;
      * ties break toward the smallest label;
      * run for a fixed `cdlp.max-iterations`.

    The early exit below is observationally equivalent to running the full count:
    synchronous propagation is a deterministic function of the label vector, so once a
    sweep changes nothing, no later sweep can either.
    """
    labels = {v: v for v in vertices}
    for _ in range(max_iterations):
        nxt = {}
        for v in vertices:
            counts = Counter()
            for u in out_adj[v]:
                counts[labels[u]] += 1
            for u in in_adj[v]:
                counts[labels[u]] += 1
            if not counts:
                nxt[v] = labels[v]  # isolated vertex keeps its own label
                continue
            best = max(counts.values())
            nxt[v] = min(l for l, c in counts.items() if c == best)
        if nxt == labels:
            break
        labels = nxt
    return labels


# --------------------------------------------------------------------------------------
# Output files
# --------------------------------------------------------------------------------------


def write_output(path, pairs, float_values):
    """Writes `<vertex-id> <value>` lines, ascending by vertex id."""
    with open(path, "w") as fh:
        for v in sorted(pairs):
            value = pairs[v]
            fh.write("%d %s\n" % (v, ("%.12e" % value) if float_values else str(value)))


def read_output(path, float_values):
    """Reads a Graphalytics output file into `{vertex_id: value}`."""
    values = {}
    with open(path) as fh:
        for lineno, line in enumerate(fh, 1):
            parts = line.split()
            if not parts:
                continue
            if len(parts) < 2:
                die("%s:%d: expected '<vertex> <value>', got %r" % (path, lineno, line.strip()))
            v = int(parts[0])
            if v in values:
                die("%s:%d: duplicate vertex id %d" % (path, lineno, v))
            values[v] = float(parts[1]) if float_values else parts[1]
    return values


# --------------------------------------------------------------------------------------
# The validation gate
# --------------------------------------------------------------------------------------


def validate_float(reference, actual, epsilon):
    """Per-vertex relative agreement. Returns a list of human-readable failures.

    NON-FINITE VALUES ARE REJECTED BEFORE THE TOLERANCE COMPARISON, and that ordering is
    load-bearing: every comparison against a NaN is false, so `abs(nan - r) > bound` does
    not fire and an output that was NaN at every vertex would otherwise validate — the gate
    would be decoration on exactly the failure mode a diverging power method produces. The
    infinities are covered by the same explicit rule rather than by the luck of the
    arithmetic happening to exceed the bound, and a non-finite REFERENCE is a broken oracle,
    which is a failure and not a pass.
    """
    failures = []
    for v in sorted(reference):
        r = reference[v]
        a = actual[v]
        if not math.isfinite(r):
            failures.append(
                "vertex %d: the REFERENCE value is not finite (%s) — the oracle is broken"
                % (v, repr(r))
            )
            continue
        if not math.isfinite(a):
            failures.append(
                "vertex %d: expected %.12e got a NON-FINITE value (%s)" % (v, r, repr(a))
            )
            continue
        bound = epsilon * abs(r) + ABS_FLOOR
        if abs(a - r) > bound:
            failures.append(
                "vertex %d: expected %.12e got %.12e (|delta| %.3e > %.3e)"
                % (v, r, a, abs(a - r), bound)
            )
    return failures


def validate_partition(reference, actual):
    """EQUIVALENCE OF PARTITIONING — the official Graphalytics rule for WCC and CDLP.

    Labels themselves are irrelevant; what must hold is that the map from a reference
    label to an actual label is a well-defined BIJECTION. Checking both directions is the
    point: a one-directional check would accept a partition that merged two reference
    groups (or split one).
    """
    fwd, rev = {}, {}
    failures = []
    for v in sorted(reference):
        r, a = reference[v], actual[v]
        if r in fwd and fwd[r] != a:
            failures.append(
                "vertices with reference label %s were SPLIT: %s vs %s (at vertex %d)"
                % (r, fwd[r], a, v)
            )
        if a in rev and rev[a] != r:
            failures.append(
                "reference labels %s and %s were MERGED into %s (at vertex %d)"
                % (rev[a], r, a, v)
            )
        fwd.setdefault(r, a)
        rev.setdefault(a, r)
    return failures


def cmd_validate(args):
    float_values = args.algo == "pr"
    reference = read_output(args.reference, float_values)
    actual = read_output(args.actual, float_values)

    missing = sorted(set(reference) - set(actual))
    extra = sorted(set(actual) - set(reference))
    failures = []
    if missing:
        failures.append("%d vertices missing from the output (e.g. %s)" % (len(missing), missing[:5]))
    if extra:
        failures.append("%d vertices not in the reference (e.g. %s)" % (len(extra), extra[:5]))

    if not failures:
        if float_values:
            failures = validate_float(reference, actual, args.epsilon)
        else:
            failures = validate_partition(reference, actual)

    if failures:
        print("VALIDATION FAILED (%s): %d problem(s)" % (args.algo, len(failures)))
        for f in failures[: args.max_failures]:
            print("  " + f)
        if len(failures) > args.max_failures:
            print("  ... and %d more" % (len(failures) - args.max_failures))
        return 1
    rule = "relative epsilon %g" % args.epsilon if float_values else "partition equivalence"
    print("VALIDATION OK (%s): %d vertices agree by %s" % (args.algo, len(reference), rule))
    return 0


# --------------------------------------------------------------------------------------
# Reference generation / dataset info
# --------------------------------------------------------------------------------------


def required(props, key):
    if key not in props:
        die("required property graph.<name>.%s is missing — it is a conformance parameter "
            "and has no safe default" % key)
    return props[key]


def cmd_reference(args):
    props, vertices, out_adj, in_adj = load_graph(args.dataset, args.name)
    if args.algo == "pr":
        iterations = args.iterations or int(required(props, "pr.num-iterations"))
        values = reference_pr(
            vertices, out_adj, in_adj, float(required(props, "pr.damping-factor")), iterations
        )
        write_output(args.out, values, float_values=True)
    elif args.algo == "wcc":
        write_output(args.out, reference_wcc(vertices, out_adj, in_adj), float_values=False)
    elif args.algo == "cdlp":
        iterations = args.iterations or int(required(props, "cdlp.max-iterations"))
        values = reference_cdlp(vertices, out_adj, in_adj, iterations)
        write_output(args.out, values, float_values=False)
    else:
        die("no reference implementation for %s" % args.algo)
    print("reference written: %s (%d vertices)" % (args.out, len(vertices)))
    return 0


def cmd_info(args):
    props, vertices, out_adj, in_adj = load_graph(args.dataset, args.name)
    name = args.name or discover_name(args.dataset)
    edges = sum(len(a) for a in out_adj.values())
    print("name=%s" % name)
    print("directed=%s" % props.get("directed", "true"))
    print("vertices=%d" % len(vertices))
    print("edges=%d" % edges)
    for key in ("pr.damping-factor", "pr.num-iterations", "cdlp.max-iterations"):
        if key in props:
            print("%s=%s" % (key, props[key]))
    return 0


def die(msg):
    sys.stderr.write("[gx] ERROR: %s\n" % msg)
    raise SystemExit(2)


def main(argv):
    ap = argparse.ArgumentParser(description="LDBC Graphalytics reference oracle + validation gate")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("reference", help="compute a reference output from the specification")
    p.add_argument("--dataset", required=True)
    p.add_argument("--name")
    p.add_argument("--algo", required=True, choices=("pr", "wcc", "cdlp"))
    p.add_argument("--iterations", type=int, help="override the dataset's iteration count")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_reference)

    p = sub.add_parser("validate", help="gate an engine output against a reference")
    p.add_argument("--reference", required=True)
    p.add_argument("--actual", required=True)
    p.add_argument("--algo", required=True, choices=("pr", "wcc", "cdlp"))
    p.add_argument("--epsilon", type=float, default=DEFAULT_EPSILON)
    p.add_argument("--max-failures", type=int, default=10)
    p.set_defaults(func=cmd_validate)

    p = sub.add_parser("info", help="print the parsed dataset metadata")
    p.add_argument("--dataset", required=True)
    p.add_argument("--name")
    p.set_defaults(func=cmd_info)

    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
