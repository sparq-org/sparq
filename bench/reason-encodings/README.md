<!-- [FABLE-5] Authored by Claude Fable 5. 🤖 SPARQ agent — sq-hmd7l.30 / sq-hmd7l.31. -->
# reason-encodings — validated VLog / Nemo Datalog encodings of sparq's LUBM closure

Datalog rule programs that make the **VLog** and **Nemo** columns of the materialization
same-box comparison ([`scripts/bench/materialize-same-box.sh`](../../scripts/bench/materialize-same-box.sh))
run a *like-for-like* closure vs `sparq-cli reason`. VLog and Nemo are **general Datalog
engines**, not native OWL 2 RL reasoners — a fair comparison needs a rule program whose
forward materialization reproduces sparq's closure. These encodings do exactly that, and
are **VALIDATED set-for-set** against sparq on LUBM(1) (`-univ 1 -seed 0`):

| profile | sparq closure | VLog `T` | Nemo `closed` | set-identical? |
|---------|--------------:|---------:|--------------:|:--------------:|
| `owl`  (OWL 2 RL) | **150589** | 150589 | 150589 | yes (0 diff both directions) |
| `rdfs` (RDFS subset) | **126732** | 126732 | 126732 | yes (0 diff both directions) |

"Set-identical" = every non-blank-node triple matches, and the blank-node-containing
triples match in count and predicate histogram (blank-node *labels* are opaque, so the
19 fixed TBox bnodes are compared structurally). The counts are the harness's pinned
acceptance oracle (`KNOWN_CLOSURE`).

## Layout

```
bench/reason-encodings/
├── vlog/rdfs.dlog     RDFS 6-rule program (VLog Datalog)
├── vlog/owl-rl.dlog   OWL-RL program: RDFS-6 + the OWL rules the LUBM TBox exercises
├── nemo/rdfs.rls      RDFS 6-rule program (Nemo .rls)
└── nemo/owl-rl.rls    OWL-RL program (Nemo .rls; @@DATA@@/@@OUT@@ placeholders)
```

Each program closes the input into a **single ternary predicate** (`T` for VLog,
`closed` for Nemo) holding the *whole* materialized graph (base + entailed), so its
cardinality is directly comparable to sparq's self-reported closure count. The adapters
[`scripts/bench-adapters/vlog_adapter.py`](../../scripts/bench-adapters/vlog_adapter.py)
and [`nemo_adapter.py`](../../scripts/bench-adapters/nemo_adapter.py) wire these as the
default rules file per profile.

## Rule set (why these rules, and not others)

The LUBM Univ-Bench TBox uses only `subClassOf/domain/range/subPropertyOf`,
`owl:inverseOf` (3 pairs), `owl:TransitiveProperty` (`subOrganizationOf`),
`owl:someValuesFrom` restrictions (8), and `owl:intersectionOf` (6). It has **no**
`owl:sameAs`, functional/cardinality, `unionOf`, `oneOf`, `hasValue`, `propertyChain`,
or `hasKey`. So each program is `rdfs2/3/5/7/9/11` **plus** exactly `prp-inv`, `prp-trp`,
`cls-svf1`, `cls-int1`, `scm-int`, `scm-dom1/rng1` (+ the subPropertyOf and inverseOf
domain/range propagations), and `scm-svf2` — the rules sparq derives on this corpus, and
**no reflexive `scm-*`/`eq-*` or axiomatic `owl:Thing` triples** (sparq emits none on
LUBM, so any would over-count). The attribution is empirical: an independent semi-naive
fixpoint of exactly these rules is set-identical to sparq's closure.

`cls-int1` and `scm-int` are both written as a **recursive list walk**, so they generalise
to any `owl:intersectionOf` list length. (`cls-int1` used to be an explicit decode for
lengths 1 and 2 — the only arities the LUBM TBox uses — which silently derived *nothing*
for a 3+-conjunct intersection, where sparq itself decodes the whole list and requires
membership in every conjunct. `sq-3xsx0`.) The pinned LUBM counts above are unaffected: on
lists of length 1 and 2 the walk derives exactly the facts the explicit decode did, which
`crates/sparq-reason/tests/datalog_intersection_arity.rs` pins by running both rule shapes
through a semi-naive fixpoint — that test also guards the shipped `.rls`/`.dlog` against a
revert. Neither engine was re-run for that change (both are gather-only); the harness
re-asserts the pinned closure count before recording any timing, so a divergence in the
encodings fails loudly rather than silently.

## Running / reproducing the validation

The engines are gather-only deps (**not** committed, per AGENTS.md). Build from source:

```sh
# VLog (github.com/karmaresearch/vlog, Apache-2.0)
git clone https://github.com/karmaresearch/vlog && cd vlog && mkdir build && cd build
cmake -DCMAKE_CXX_FLAGS="-include cstdint" .. && make -j   # cstdint flag: GCC-13 workaround
# Nemo (github.com/knowsys/nemo, Apache-2.0)
git clone https://github.com/knowsys/nemo && cd nemo && cargo build -r -p nemo-cli

# then, from the repo root:
VLOG=/path/to/vlog/build/vlog NEMO=/path/to/nemo/target/release/nmo \
  LUBM_UNIVS=1 MAT_ITERS=1 scripts/bench/materialize-same-box.sh
```

The harness asserts each engine's closure against the pinned count before recording any
timing. Work-box timings are **non-canonical** (directional only); the canonical
`univ>=100` run belongs to `sq-hmd7l.32` on a dedicated quiet EC2 box.

## Nemo convergence note

Nemo's semi-naive evaluation does not converge in reasonable time if `cls-svf1` /
`cls-int1` inline the restriction/intersection schema atoms into the recursive `closed`
rule. The structural difference: every rule in the program derives into the **one**
predicate `closed`, so semi-naive must instantiate each round's delta at every body
position of every rule. Inlined, `cls-svf1` is a 4-atom rule over `closed` alone — four
delta-rules per round, each a 4-way join over a still-growing relation — and nothing in
the program marks the two restriction atoms as *schema* rather than *data*, so whether a
round gets planned schema-first is left to the engine.

The `.rls` encoding therefore **splits the predicate**: the schema is folded into small
`svfR` / `intTail` / `intAll` relations first, and only then joined against the type index.
Those relations saturate early (on this corpus nothing derives a new `owl:onProperty` /
`owl:someValuesFrom` triple after the opening rounds), so their delta goes empty and what
is re-done each round is a join of a small, stable relation against the type index. This is
not a separate recursive component — the helpers are themselves derived from `closed` — it
makes "schema first, data after" a property of the *program* rather than a hope about the
planner, which is the only form of that hint a `.rls` file can carry. The result is
logically identical and converges in a couple of seconds. VLog evaluates the inlined
`cls-svf1` form directly. Both agree with sparq set-for-set.

**Unmeasured** (`sq-3xsx0`): whether some rule ordering or engine-side hint makes the
*inlined* form converge under Nemo has not been tested — both engines are gather-only
builds, so re-testing that needs a Nemo binary (see *Running / reproducing* above). The
paragraphs above describe this program's join structure, which is inspectable in the file.

## License

Apache-2.0 (the workspace license). The referenced engines (VLog, Nemo) are Apache-2.0.
