# Graph-Analytics Gap Record (LDBC Graphalytics) — 2026-07

<!-- [SONNET-4.6] sq-hmd7l.13 — graph-analytics axis gap record per the sq-hmd7l epic protocol. -->
<!-- DO NOT hard-code performance numbers in this file. Measured envelopes land in -->
<!-- bench/competitor-results/ (git-ignored). Any timing produced on the work box or a CI -->
<!-- runner is NON-CANONICAL — see bench/CATALOG.md QUIET-BOX. -->

**Axis:** graph analytics — LDBC Graphalytics algorithms and their validation outputs.
**Epic:** sq-hmd7l (comparative-benchmarking-everything).
**Bead:** sq-hmd7l.13.
**Date:** 2026-07.
**Status of every number produced by this harness so far: NON-CANONICAL.** No quiet-box
gather has been run; the harness has only been exercised on the committed smoke fixture.

---

## 1. What is being compared

[LDBC Graphalytics](https://ldbcouncil.org/benchmarks/graphalytics/) is a graph-analytics
benchmark that pairs each dataset with **per-algorithm reference outputs**. That makes it a
correctness-gate-then-time suite rather than a pure timing suite, which is the pattern this
repository prefers: an engine's timing is only meaningful once its *answer* has been
checked against the reference.

Reference engines, both already pinned in `bench/competitors.json` by the registry bead
(sq-hmd7l.1): **igraph** (C core, Python API) and **NetworKit** (C++/Python). Both are MIT
licensed, so measured numbers are publishable.

Harness: `bench/graphalytics/run.sh`. sparq leg:
`crates/sparq-algos/examples/graphalytics.rs`. Competitor leg:
`scripts/bench-adapters/graph_analytics_adapter.py`. Oracle + gate:
`bench/graphalytics/gx.py`.

## 2. Correction to the brief's premise

The bead brief describes sparq-algos as having "pagerank/centrality/community" against a
Graphalytics wish-list of "BFS/PR/WCC/CDLP/LCC/SSSP", implying the intersection is roughly
`{PR, WCC, CDLP}`. Verified against the crate rather than assumed, the intersection is
**smaller than that**, and in one direction larger:

- `{PR, WCC}` are genuinely conformant — see §3, where the conformance argument is spelled
  out rather than asserted.
- **CDLP is NOT in the intersection.** `sparq_algos::label_propagation` and Graphalytics
  CDLP are different algorithms that share a name. This was *measured*, not predicted: on
  the committed fixture the two partitions differ, and the harness prints the divergence.
- `sparq-algos` also implements betweenness/closeness centrality, k-core, SCC and
  topological sort, none of which Graphalytics specifies. They are real capability and are
  simply outside this axis — they are not evidence for or against it.

## 3. Per-algorithm status

| Graphalytics | sparq-algos | verdict |
|---|---|---|
| PR | `pagerank` | conformant, validated |
| WCC | `weakly_connected_components` | conformant, validated |
| CDLP | `label_propagation` | **semantic divergence** — measured, not assumed |
| BFS | — | **feature gap** |
| LCC | — | **feature gap** |
| SSSP | — | **feature gap** |

### 3.1 Why PR is conformant

The Graphalytics PageRank recurrence is

```text
PR_0(v) = 1/n
PR_i(v) = (1-d)/n + d * ( SUM_{u in Nin(v)} PR_{i-1}(u)/outdeg(u)
                        + SUM_{w : outdeg(w)=0} PR_{i-1}(w)/n )
```

run for a fixed `pr.num-iterations`. That is term-for-term the body of
`sparq_algos::pagerank`: uniform initialisation, the `(1-d)/n` teleport, dangling mass
redistributed uniformly, and each in-neighbour's contribution divided by its out-degree.
The library's one *additional* behaviour is an early stop once the L1 delta falls below
`tolerance`; setting `tolerance = 0.0` makes that condition unsatisfiable, so the power
method runs exactly `max_iterations` sweeps.

The consequence matters for the honesty of the claim: the harness validates the **library
function**, configured, not a Graphalytics-shaped re-implementation living in the example.
A harness that re-implemented the algorithm would be benchmarking the harness.

### 3.2 Why WCC is conformant

Weak connectivity is unambiguous, and the official validation rule for WCC is *equivalence
of partitioning* — component labels need not match, the induced partitions must. The
runner additionally relabels each component to the smallest vertex id it contains, which is
the reference-output convention; this is presentation only, and the gate compares
partitions either way.

### 3.3 Why CDLP diverges (measured)

Graphalytics CDLP is **synchronous** — every vertex updates from the *previous* iteration's
label vector — over a fixed `cdlp.max-iterations`. `sparq_algos::label_propagation` is
**asynchronous**: it updates labels in place during a sweep and runs to a fixed point. The
two agree on the parts both get right (a reciprocal edge votes twice; ties break toward the
smallest label) and disagree on the update discipline.

That is not a cosmetic difference. On the committed fixture, synchronous propagation
*oscillates* on two mutually-linked pairs and leaves them in distinct communities, whereas
the asynchronous variant merges each pair. The harness reports the merge explicitly. The
row is marked `MISMATCH`, carries no timing, and does not fail the run — a documented
semantic gap is not a regression, but it is also not a pass.

### 3.4 The three feature gaps

Filed as separate P2 feature requests rather than being papered over:

- **BFS** — no traversal / hop-distance surface exists in `sparq-algos`. Note that
  `centrality_extended` already runs BFS *internally* for closeness and betweenness, so the
  gap is an exposed API, not an absent capability.
- **LCC** — local clustering coefficient. Needs neighbour-set intersection over the
  undirected projection; nothing in the crate computes it today.
- **SSSP** — the deepest of the three. `NodeGraph` is deliberately an *unweighted*,
  predicate-erased, parallel-edge-collapsing view, so weighted shortest paths need a
  weighted projection first (e.g. edge weights drawn from a numeric predicate). That is a
  design question, not just a missing function.

## 4. Honest framing of the comparison itself

**Comparable.** WCC across sparq / igraph / NetworKit: identical semantics, exact
partition, no termination ambiguity.

**Not comparable — igraph PageRank.** igraph's `pagerank()` solves for the stationary
distribution (PRPACK) and exposes no fixed-sweep knob, so it cannot reproduce a ten-sweep
reference; the adapter reports `semantics=converged`. The suite holds only the fixed-sweep
reference, which is *not* an oracle for a converged solve — gating against it would print a
per-vertex delta that reads as an igraph correctness failure when it is only a difference of
termination rule. The harness therefore records the row as `SEMANTIC-GAP`: not gated, not
timed. Generating an independent converged oracle so igraph's PageRank *can* be validated is
open work (a Phase 2 item), and until it exists no igraph PageRank number is reported.
NetworKit does expose `maxIterations`, runs the spec form, and is gated normally.

**Not the same job — and this cuts both ways.** igraph and NetworKit are embedded graph
libraries handed a prepared edge array; `sparq-algos` runs over an RDF triple store's
dictionary ids. sparq's differentiator is that analytics need **no export**: the projection
step goes straight from the store to an adjacency view. The corresponding cost an
RDF-store user would otherwise pay — serialising the graph out to flat edge-list text — is
measured in the same run as `export_edgelist`. Both halves are reported per algorithm, and
neither should be quoted without the other.

**Also not free, and not yet measured at scale.** The sparq leg materialises the
Graphalytics files as N-Triples and loads them, which the embedded libraries do not do.
Those steps are timed separately (`materialize`, `load`) precisely so they cannot hide
inside an "algorithm" number — but on a real LDBC dataset they are likely to *dominate*,
and no dominance claim on this axis is available until §5 Phase 2 has run.

## 5. Phased plan (each phase a future bead)

1. **Phase 1 — harness + validation gate + honest gap inventory.** *This bead
   (sq-hmd7l.13).* `run.sh --smoke` is green on the committed fixture with the PR and WCC
   gates validated against an independent from-the-spec oracle; CDLP's divergence and the
   three feature gaps are recorded rather than hidden. No competitor numbers.
2. **Phase 2 — same-box gather on real LDBC datasets.** Install python-igraph + networkit,
   fetch a real Graphalytics dataset with its shipped reference outputs, run the full panel
   on a quiet box, and record envelopes to `bench/competitor-results/`. This is the first
   phase permitted to state any comparative result. It should also cross-check our
   from-the-spec oracle against LDBC's own shipped reference outputs — agreement there is
   the strongest available evidence that the oracle is right.
3. **Phase 3 — close the BFS gap.** Expose the traversal `centrality_extended` already
   performs internally as a public, Graphalytics-conformant BFS, and promote the BFS row
   from GAP to gated.
4. **Phase 4 — close the LCC gap.** Local clustering coefficient over the undirected
   projection.
5. **Phase 5 — decide the CDLP question.** Either add a synchronous, fixed-iteration
   Graphalytics-conformant mode to `label_propagation` (an opt-in config, so the existing
   asynchronous default is unchanged), or record a deliberate decision not to, and keep the
   row permanently marked as a semantic divergence. Adding the mode is what would let the
   CDLP row become a real comparison.
6. **Phase 6 — decide the SSSP question.** Requires a weighted projection design (where do
   edge weights come from in RDF?) before any implementation bead is worth writing.

Phases 3–6 are the P2 feature requests referenced in §3.4; phase 2 is the prerequisite for
any published number on this axis.

## 6. Open questions for the maintainer

- **Is a Graphalytics-conformant CDLP wanted at all?** The asynchronous variant is arguably
  the better algorithm for sparq's own users (it converges rather than oscillating). Adding
  a synchronous mode is a benchmark-conformance decision, not a product one, and it should
  be made deliberately.
- **Where should SSSP edge weights come from?** A numeric predicate selected by the caller
  is the obvious answer, but it changes `NodeGraph` from a predicate-erased view into a
  predicate-aware one for that use. Worth a design record before implementation.
- **Which LDBC scale is the target for Phase 2?** The dataset sizes span several orders of
  magnitude, and the disk-discipline rules in `bench/CATALOG.md` cap what can be fetched.
