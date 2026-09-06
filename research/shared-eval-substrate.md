# Design — shared zero-overhead evaluation substrate for the SPARQL engine and the reasoners

<!-- [OPUS-4.8] Design-for-review for epic sq-qonbz (under umbrella sq-6tykl). NO production
code in this PR — this is an investigation + plan the maintainer reviews before any refactor.
🤖 SPARQ agent. -->

> 🤖 **SPARQ agent** — design record for @jeswr's review. DESIGN-FOR-REVIEW only: this PR
> changes no engine/reasoner code. It proposes *how* to extract a shared eval substrate and
> *how to prove* the extraction is perf-neutral, then decomposes the work into beads.

**Status:** DESIGN / design-for-review. **Epic:** sq-qonbz (shared substrate), under umbrella
sq-6tykl. **Gates:** the reasoner refactor (sq-pbz04) depends on the substrate *design* landing
here; federation (sq-my8wd), RSP (sq-2n1q3) and geo (sq-lk3aw) parallelise independently.

**Recommendation in one line:** extract the engine's id-level join / numeric / term-compare
hot loops into a **new leaf crate `sparq-substrate`** that depends only on `sparq-core`, is
**monomorphic over `Id = u32` and the existing `SmallVec` row/key aliases with zero `Box<dyn>`
on the hot path**, is **feature-gated identically to today** so the lean wasm bundle stays
byte-identical, and is consumed by *both* `sparq-engine` (which keeps its planner) *and* the
reasoners (which gain a shared semi-naive join instead of their hand-rolled `FxHashMap`
adjacency). The extraction is a **pure code-move + generalise**, validated as perf-neutral by
the existing deterministic ratchets plus the engine micro-benches — not a rewrite.

---

## 0. Premise check (honesty first — what the brief got right and what it got wrong)

The parallel research brief says the engine's join/arith/term machinery "can be REUSED without
modification" by the reasoners, and frames the substrate as already-shared. **Verified against
the code, that is the aspiration, not the present reality, and the brief slightly mis-states the
dependency graph.** Two corrections:

1. **The reasoners do NOT currently call the engine's joins.** `crates/sparq-reason` and
   `crates/sparq-reason-el` depend **only on `sparq-core`** (`default-features = false`), *not*
   on `sparq-engine`. The engine's join families (`merge_join`, `hash_join`, `bind_join`,
   `lftj_recurse`) live in `crates/sparq-engine/src/exec.rs` and are private to that crate. The
   RL materialiser in `crates/sparq-reason/src/owl.rs` does its joining with its **own**
   `FxHashMap<Id, FxHashMap<Id, Vec<Id>>>` out/inc adjacency indexes and a bespoke `UnionFind`
   (lines ~122–326). So today there are **two independent join implementations**, not one shared
   substrate. "Share the substrate" means *extract and unify*, which is real, larger work — not a
   no-op reuse.

2. **The shared code cannot live in `sparq-engine`.** Because `sparq-engine` depends on
   `sparq-core` and the reasoners must *not* take a dependency on the whole engine (it would pull
   the planner, `service`/`ureq`, serializers and aggregates into the reasoner crates and into
   the tier-b reasoner wasm bundle), the substrate must sit **at or below `sparq-core`** in the
   dependency DAG. A new leaf crate `sparq-substrate` (depending only on `sparq-core`, or on
   nothing and re-exported by `sparq-core`) is the only placement where engine *and* both
   reasoners can reach it without a cycle.

Everything else in the brief checks out and is load-bearing:

- `type Row = SmallVec<[Id; 4]>` (`exec.rs:720`), `type Key = SmallVec<[Id; 2]>` (`exec.rs:732`),
  monomorphic joins, `eval_numeric` fast path (`exec.rs:7185`), `compare_values` total order
  (`exec.rs:7869`), `as_numeric` (`exec.rs:7840`), the three BGP plans and `scan_to_bindings`
  (`exec.rs:5138`) all exist as described. Line numbers have drifted a little from the brief's
  snapshot (the file is now 11 411 lines) but the structure is exactly as claimed.
- The deterministic perf ratchets are real and currently pinned (verified in
  `bench/perf-baseline.json`): `wasm_bundle_bytes = 1686907`, `store_bytes_per_triple = 92`,
  `dict_bytes_per_term = 53`. `parse_ns_per_byte = 4.9721` is pinned alongside them but is
  wall-clock-derived (`mode: noise`) — an advisory timing signal (tracked/warned,
  non-blocking), not a deterministic ratchet. The brief's older figure of 1656470
  is stale — the floor was re-baselined to **1686907** (#1286, 2026-06-29). **Use 1686907.**
- `Graph::from_parts(dict, triples)` (`sparq-core/src/lib.rs:1114`) is the existing seam the
  reasoners already use to hand materialised closures back as a queryable graph.
- The entailment harness already calls `sparq_reason::materialize(Profile::Rdfs|OwlRl, …)` and
  `materialize_owl_rl` (`crates/sparq-conformance/src/inference/{sparql_entail,owl_suite}.rs`).

**Net:** the epic is sound and worth doing, but it is an *extraction-and-unification* project
with a real (small-to-medium) refactor surface, not a relabelling. The honest framing is "build
the shared substrate the brief assumes already exists, and prove the move costs nothing."

---

## 1. Problem framing

sparq wants the SPARQL engine *and* every reasoner (RL-complete, EL, QL, Direct, RIF, D) to
share the parts of evaluation that are genuinely common — joining sets of id-tuples, comparing
RDF terms by SPARQL's total order, and doing numeric value arithmetic on dict-encoded literals —
**without** any of them paying for the others, and **without** regressing the engine's hot
loops, the wasm bundle, or the deterministic byte ratchets.

The tension is concrete:

- **Reuse pressure.** A reasoner's rule body is a conjunctive query; firing a rule is a
  multi-way join over id-tuples; that is precisely what `hash_join`/`merge_join`/`lftj_recurse`
  already do well, with cardinality ordering, radix partitioning, and galloping search. Today the
  reasoners reimplement a weaker version (per-predicate `FxHashMap` adjacency) and miss the
  engine's WCOJ/merge machinery entirely.
- **Isolation pressure.** The engine's joins are wired to `Bindings { vars, rows, sorted_by }`,
  to `LocalVocab`, to the query budget, to the planner's `ScanCmp` pushdown, and (behind feature
  gates) to rayon. A reasoner does not want the planner, the budget-per-WHERE-solution model, or
  the `service`/serializer dependencies. Naïvely sharing by depending on `sparq-engine` would
  bloat the reasoner crates and the reasoner wasm bundle.
- **Perf pressure.** The engine's hot loops are deliberately monomorphic with near-zero dynamic
  dispatch (only `ExtFn`, `SpatialProvider`, custom aggregates use `dyn`, all off the hot path).
  Any "make it generic so it can be shared" refactor that introduces a `Box<dyn>` or a vtable in
  the probe loop, or that defeats inlining, is an immediate regression risk the byte/throughput
  ratchets must catch.

So the design question is: **what is the largest set of primitives we can lift into a shared
leaf crate such that the engine keeps identical codegen on its hot loops, the reasoners gain a
real join, and nobody takes an unwanted dependency?**

---

## 2. What is genuinely shareable vs what must stay engine-private

Separating the truly-common kernel from the engine-specific glue is the heart of the design.

### 2.1 Shareable (lift into `sparq-substrate`)

These operate purely on `Id` tuples and the `Dict`/value caches that already live in
`sparq-core`. They have no dependency on the planner, the budget, `LocalVocab`, or feature-gated
I/O:

- **Join kernels** over `&[Row]` / `&[Key]`: sorted **merge join**, radix-partitioned **hash
  join**, **index-nested-loop / bind join**, and **leapfrog trie-join** (WCOJ). These are the
  `merge_join`/`hash_join`/`bind_join`/`lftj_recurse` bodies, generalised to take their inputs as
  plain row slices + a join-key spec rather than the engine's `Bindings` struct.
- **Term total order**: `compare_values` (`exec.rs:7869`) — the SPARQL 3-valued / RDF 1.2 total
  order (unbound < blank < IRI < literal-by-value < triple-term-componentwise). Used by ORDER BY,
  MIN/MAX, equality *and* by any reasoner type/identity check.
- **Numeric value layer**: the `Num` enum (`Int`/`Float`/`Decimal` with xsd promotion),
  `as_numeric` (`exec.rs:7840`), and the `binop` arithmetic — the value-space machinery a
  D-entailment / RIF-builtin reasoner needs and that the engine uses for FILTER/BIND arithmetic.
- **The id-tuple vocabulary**: the `Row`/`Key`/`Posting` `SmallVec` aliases and the inline-int
  helpers (`dict::inline_id_of_int`). These already conceptually belong with `Dict` in
  `sparq-core`; the substrate crate re-exports them so both consumers agree on representation.

### 2.2 Engine-private (stays in `sparq-engine`)

These are query-shaped and must *not* leak into the reasoners:

- The **planner**: cardinality estimation, `bgp_is_cyclic` (GYO), `extract_sargable`, the choice
  between `eval_bgp_binary` and `eval_bgp_wcoj`. A reasoner picks its own join order from its rule
  structure; it does not want the SPARQL planner.
- `Bindings { vars, rows, sorted_by }`, `LocalVocab` interning of BIND/aggregate/CONSTRUCT
  results, the per-WHERE-solution **`QueryBudget`** model, and `ScanCmp` filter pushdown (which is
  about *scanning a store with a SPARQL filter*, not about firing a rule).
- All of `service.rs`, the serializers, EXISTS/subquery, GROUP BY/aggregation. The reasoners reach
  the materialised result through `Graph::from_parts` + plain SPARQL, exactly as today, so they
  inherit aggregation/EXISTS *for free at query time* without sharing that code at build time.

### 2.3 The interface that keeps both happy

The join kernels are exposed as **free functions generic over a tiny `JoinKeys` descriptor**, not
as methods on `Bindings`:

```rust
// in sparq-substrate (sketch — not implementation)
pub fn hash_join(
    left: &[Row], left_key: &[u16],      // column indices forming the key
    right: &[Row], right_key: &[u16],
    out: &mut Vec<Row>,                  // caller-owned output buffer
);
```

The engine wraps these with a thin adapter that pulls `left_key`/`right_key` from its `Bindings`
variable layout and threads its budget around the call; the reasoner wraps them with an adapter
that derives keys from the shared variables of a rule's body atoms. **Neither wrapper is on the
hot loop** — the hot loop is inside the kernel and is identical for both callers. Because the
kernel is monomorphic over the concrete `Row = SmallVec<[Id;4]>` (not over a `dyn` trait or a
generic `T: Ord` resolved at runtime), the compiler emits one specialised, inlinable body. This
is the zero-overhead property, stated precisely: **shared by source, monomorphised per call site,
no vtable, no `Box<dyn>` between the probe and the comparison.**

---

## 3. Design options considered

### Option A — Depend on `sparq-engine` from the reasoners (rejected)

Make the reasoners `use sparq_engine::{hash_join, …}`. **Rejected:** creates the wrong dependency
direction, pulls the planner + `service` + serializers into the reasoner crates and the tier-b
reasoner wasm bundle, and risks the `wasm_bundle_bytes` floor on the reasoner bundles. Also makes
every engine change a reasoner-API change.

### Option B — Copy the kernels into each reasoner (status quo, rejected)

Leave the engine as-is and let each reasoner keep / grow its own join code. **Rejected:** this is
exactly the duplication the epic exists to remove; the reasoners' `FxHashMap` adjacency is weaker
than the engine's WCOJ/merge/galloping machinery, so EL/QL/Direct would each reinvent a worse
join, and a `Dict`/id-format change would have to be applied in N places (the brief's own
regression risk #9).

### Option C — Extract a shared leaf crate `sparq-substrate` (recommended)

Lift §2.1 into a new crate that depends only on `sparq-core` (or on nothing, re-exported by
`sparq-core`). `sparq-engine` and both reasoners depend on it. **Chosen.** It is the only
placement with no dependency cycle, it removes the duplication, it lets the reasoners adopt the
*good* joins, and — done as a pure code-move — it is provably perf-neutral because the engine's
hot loop is the same source compiled to the same code. The one cost is a moderate refactor of
`exec.rs` to call across a crate boundary (mitigated: the boundary is a function-call boundary at
the *top* of each join, with the loop body fully inside the callee, so cross-crate inlining via
`#[inline]` / LTO preserves codegen).

### Option D — Make `sparq-core` itself host the kernels (viable fallback)

Put the join kernels directly in `sparq-core` rather than a new crate. **Viable but not
preferred:** `sparq-core` is the lean ingest/storage crate that the *lean* wasm bundle is built
from; adding join code there risks the lean-bundle floor unless every kernel is behind a default-
off feature. A separate `sparq-substrate` keeps `sparq-core` lean by construction and makes the
feature-gating explicit. Recommend C; fall back to D only if a build-graph constraint forbids the
new crate.

---

## 4. Recommendation and the zero-overhead contract

Adopt **Option C**. The substrate crate publishes the join/term/numeric kernels under the same
default-off feature discipline the workspace already uses, with these invariants written as CI-
checkable rules:

1. **No `Box<dyn>` / `&dyn` / vtable between a join's probe and its key comparison.** Enforced by
   a clippy-lint / grep gate over `sparq-substrate/src` hot-loop modules and by the kernels being
   generic only over the concrete `Id`/`SmallVec` aliases.
2. **`sparq-core` lean wasm bundle byte-identical.** The substrate links no new external
   dependency; its code is feature-gated so the lean bundle compiles *none* of it. Proven by the
   `wasm_bundle_bytes` ratchet (floor 1686907) staying green on the lean build.
3. **Engine hot-loop codegen preserved.** The move is source-identical; cross-crate inlining is
   kept via `#[inline]` on the kernels + workspace LTO. Proven by the engine join/scan
   micro-benches and the deterministic byte ratchets (the substrate move touches no storage
   layout, so `store_bytes_per_triple` / `dict_bytes_per_term` are unaffected; any change there is
   a red flag).
4. **Reasoner wasm bundles do not regress.** The tier-b `sparq-reason-wasm` / `sparq-reason-el`
   bundles gain only the kernels they actually call; if a reasoner adopts the shared join, its
   bundle delta is measured and either justified or feature-gated.

---

## 5. Perf-neutrality strategy — how we PROVE the move costs nothing

This is the make-or-break of the epic. The proof is layered and uses **only deterministic gates**
(the EC2/work-box is non-canonical; we never bake a wall-clock number into a ratchet):

| Layer | Gate | What it proves |
|---|---|---|
| Bundle | `wasm_bundle_bytes` floor 1686907 (`scripts/perf-gate.py`) | Lean wasm unchanged; substrate compiled out by default |
| Storage | `store_bytes_per_triple` 92, `dict_bytes_per_term` 53 | The move touches no storage/dict layout |
| Codegen | engine join/scan/BGP micro-benches in `sparq-bench` / `bench/` | Hot-loop throughput within noise of pre-move baseline |
| Correctness | W3C SPARQL conformance (floor pinned in `sparq-conformance`) | The engine still computes identical answers after the move |
| Reasoner | inference conformance (RDFS 48/48, OWL RL ratchet) unchanged | Reasoners that adopt the shared join still materialise the same closure |

The decisive trick is to land the extraction as a **pure refactor PR first** (no behaviour
change, no reasoner re-wiring) and require *every* ratchet above to stay byte-identical /
within-noise on that PR. Only once the move is proven neutral do we let the reasoners adopt the
shared join (a separate, independently-revertible PR). This keeps the perf-risk and the
behaviour-risk on different PRs.

**Honest perf risks (stated, not hidden):**

- *Cross-crate inlining failure.* If LTO/`#[inline]` does not carry the loop body across the new
  crate boundary, the engine could regress. Mitigation: micro-bench the move before merge; if a
  kernel won't inline, keep it `#[inline(always)]` or, worst case, fall back to Option D
  (in-`sparq-core`) for that kernel.
- *Materialisation avalanche in reasoners* (brief risk #1). A reasoner's fixpoint can explode the
  closure; the engine's per-query `QueryBudget` does **not** bound a rule loop. The shared join
  does not fix this — the reasoner must install its **own** closure-level budget around the
  fixpoint. This is a reasoner concern (sq-pbz04), not a substrate concern, but the substrate
  should expose a cooperative cancellation hook the reasoner can poll.
- *Numeric precision* (brief risk #6). Pushing a high-precision `xsd:decimal` rule threshold
  through an f64 `ScanCmp` loses precision silently. The substrate's `Num` keeps the typed
  decimal path; reasoners must use the typed compare, not the f64 fast path, for value-space
  rules. Documented as a soundness note, not a perf trick.

---

## 6. Soundness notes (substrate scope)

- **Single id space is the soundness keystone.** A join is only correct if both sides agree on
  term identity. Every reasoner output term MUST be interned through the *same* `Dict` (and
  `LocalVocab` for computed terms) as query-computed terms, so equal terms get equal ids
  (brief risk #2). The substrate must NOT offer a constructor that takes raw ids from an external
  cache. Enforced by making the only term→id path go through `sparq-core::Dict::intern` /
  `LocalVocab::intern`.
- **The substrate is value-correct, not entailment-aware.** It computes joins and comparisons; it
  makes **no** claim about which triples *should* exist. Soundness of a regime (RL completeness,
  EL CR-rules, QL applicability) lives entirely in the reasoner layer (sq-pbz04) and is verified
  by the inference conformance suite — the substrate cannot make an unsound reasoner sound, and a
  sound reasoner on top of a correct substrate stays sound.
- **No privacy/ZK/MPC surface here.** The substrate is plain id-tuple evaluation. It does not
  touch the ZK verifier or the MPC seam; nothing in this record makes any privacy claim.

---

## 7. Phased plan (each phase = a future bead under sq-qonbz)

1. **Stand up `sparq-substrate` crate** (leaf, depends only on `sparq-core`, default-off
   features, README ≤120 lines). No logic yet — just the crate, the `Row`/`Key`/`Num` re-exports,
   and the feature wiring. *Acceptance:* workspace builds; lean wasm byte-identical (floor
   1686907); `cargo deny` clean.
2. **Move `compare_values` + `Num`/`as_numeric` + arithmetic into the substrate**; engine and the
   value-space machinery call across the boundary. *Acceptance:* W3C SPARQL conformance floor
   unchanged; engine ORDER BY / FILTER micro-benches within noise; byte ratchets unchanged.
3. **Move the four join kernels** (merge / hash / bind / LFTJ) behind the `JoinKeys` descriptor;
   engine adapts its `Bindings` callers. *Acceptance:* full W3C SPARQL conformance unchanged; join
   micro-benches within noise; no `dyn` in hot-loop modules (grep/clippy gate).
4. **Cooperative-cancellation hook** in the substrate join driver (a cheap `&AtomicBool` /
   closure poll) so a reasoner can bound a runaway fixpoint. *Acceptance:* a unit test cancels a
   long join; engine path unaffected when the hook is `None`.
5. **Reasoner adoption (RL)**: rewrite `sparq-reason`'s `owl.rs` rule-firing to use the shared
   join instead of its `FxHashMap` adjacency, keeping `UnionFind` for sameAs. *Acceptance:*
   inference conformance (RDFS 48/48, OWL RL ratchet + documented divergences) unchanged;
   `sparq-reason-wasm` bundle delta measured + justified/gated. *Depends on phases 3–4.*
6. **Perf-neutrality CI rule**: a workspace lint/gate asserting the hot-loop modules carry no
   `Box<dyn>`/`&dyn`, plus a doc note in `AGENTS.md` recording the substrate boundary as a
   durable architecture fact. *Acceptance:* gate green on main; AGENTS.md updated.

---

## 8. Open questions for the maintainer

1. **New crate (`sparq-substrate`, Option C) vs hosting the kernels in `sparq-core` (Option D)?**
   I recommend C for explicit lean-bundle gating; D is a smaller diff. Your call on crate-count
   vs locality.
2. **Do you want the RL reasoner adopted onto the shared join *in this program* (phase 5), or is
   the substrate extraction (phases 1–4) enough for now**, leaving reasoner adoption to land
   alongside the EL/QL build-out under sq-pbz04? Phase 5 is the riskier, higher-value half.
3. **Budget model unification.** Should the substrate own a single cooperative-cancellation
   abstraction that *both* the query budget and the reasoner closure-budget implement, or keep
   them separate? Unifying is cleaner but touches the engine's budget plumbing.
