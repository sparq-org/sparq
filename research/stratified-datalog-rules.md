# Stratified Datalog rules: NAF + aggregation with a checked stratification (sq-6tykl.3)

<!-- [FABLE-5] Design record for the RDFox-parity stratified-Datalog BIG ROCK (epic
sq-6tykl, cross-ref competitive-parity epic sq-lsp7k). Phase 1 ships WITH this record:
crates/sparq-reason/src/datalog/ behind the opt-in `datalog` feature. Later phases are
beaded from §6. -->

> 🤖 **SPARQ agent** — design-for-review, written under the standing
> proceed-and-document rule (the surface-syntax choice in §2 resolves the maintainer's
> open question 4 of `research/competitive-feature-analysis-2026-07.md`; steer post-hoc
> via the tracking issue).

## 1. Why

RDFox's rich Datalog dialect — `NOT`/`NOT EXISTS` and `AGGREGATE` atoms under
stratification, incrementally maintained — is the single feature enterprises buy RDFox
for beyond OWL: real business rules (thresholds, counts, absence checks) that OWL cannot
express. Stardog's equivalent (Stride) is beta; GraphDB's `.pie` has no aggregates.
sparq already has the substrate for it: forward chainers (RDFS/OWL-RL/N3), an id-level
compiled-rule pipeline on the shared `sparq-substrate::join` kernels, store-scoped NAF
(`log:notIncludes`) — but **no stratification checker** (the N3 compiled path documents
stratification as a *caller discipline*), **no aggregates in rule bodies**, and no
Datalog surface syntax.

## 2. Surface syntax: a small native dialect (open question 4)

Options considered:

| option | for | against |
|---|---|---|
| **N3 builtins** | parser + NAF (`log:notIncludes`) exist | no aggregate idiom short of cwm's list-valued `log:collectAllIn` (formula/list-valued facts are exactly what the compiled subset excludes); extending N3 semantics non-standardly taints a W3C-adjacent surface |
| **RIF extension** | `rif::Document` validator exists | RIF-Core is monotone *by definition* — NAF/aggregates would be a nonstandard fork of a standard; XML presentation syntax is hostile to hand-written business rules |
| **native dialect (CHOSEN)** | direct RDFox rule-migration parity (the business goal); stratification + aggregation are first-class in the syntax; N3/RIF keep their standard semantics untainted | a new (small) parser |

**Decision: a small native rule dialect modelled on RDFox's Datalog surface**, lowered
onto the SAME machinery everything else uses (substrate join kernels, the substrate
`numeric` tower, dictionary ids). The dialect is deliberately tiny; anything outside the
fragment is a loud parse error naming the construct.

```text
@prefix ex: <http://example.org/> .
[?x, ex:deg, ?c] :- AGGREGATE([?x, ex:edge, ?y] ON ?x BIND COUNT(?y) AS ?c) .
[?x, a, ex:Hub]  :- [?x, ex:deg, ?c], FILTER(?c >= 3) .
[?x, a, ex:Leaf] :- [?x, a, ex:Node], NOT [?x, a, ex:Hub] .
```

Grammar (Phase 1 plus the shipped Phase-4 extensions): `@prefix` declarations; rules
`head :- body .` with comma-separated triple-pattern atoms `[s, p, o]` (constant or
variable predicate; `a` = `rdf:type`; IRIs, prefixed
names, `?vars`, bare-integer/`"…"^^dt` literals); body elements `atom`, `NOT atom`,
`NOT { atom, atom }` / `NOT EXISTS { atom, atom }`,
`AGGREGATE(atoms [ON ?g…] BIND COUNT([DISTINCT] ?v) AS ?c)`, `FILTER(x op y)`;
multi-atom heads. <!-- [GPT-5.6] sq-a7bmo -->

## 3. Semantics

Textbook **perfect-model (stratified) semantics**: `stratify` builds the dependency
graph — positive edges for body atoms, **non-monotonic** edges for `NOT` atoms and every
predicate inside an `AGGREGATE` body (adding a fact can change a count) — and either
assigns strata (iterative stratum-raising fixpoint, Ullman §3.8) or rejects the program
naming a node on the negation/aggregation cycle. Evaluation runs strata in order, so
every negated/aggregated relation is complete before it is read.

Load-bearing details, each pinned by a test:

- **Class granularity.** Dependency nodes are predicates, EXCEPT `rdf:type` atoms with a
  constant class, which get a per-CLASS node — RDF encodes unary relations as classes,
  so `NOT [?x, a, ex:Hub]` must not collide with an unrelated `[?y, a, ex:Leaf]` head
  (predicate-granular checking rejected exactly that program in development). A
  variable-class `rdf:type` atom is conservative: reading one sits above every class;
  deriving one feeds every class reader.
- **Safety (range restriction).** Every head/`FILTER` variable must be bound by a
  positive atom or an `AGGREGATE` (`ON`/`AS`) — checked at parse. `NOT` variables not so
  bound are existential wildcards (a repeated wildcard inside one `NOT` atom must match
  equal terms — the `log:notIncludes` idiom, kept). In a grouped `NOT`, wildcard
  bindings join every atom in that group and are discarded outside it.
- **Aggregate scoping.** Non-`ON` aggregate-body variables are aggregate-local; a name
  collision with the outer rule is a loud error (silent capture would be ambiguous).
  The `AS` output must be fresh. `COUNT(?v)` counts DISTINCT full body matches per group
  (set semantics — Datalog relations are sets); `COUNT(DISTINCT ?v)` de-duplicates the
  projected value within each group; empty groups produce no row, and counts
  mint `xsd:integer` literals into the caller's `Dict`.
- **FILTER** compares exact/float/double numerics through the shared substrate
  `Num::cmp_relational`; non-numeric values and NaN fail the row.
- **Variable predicates.** The dependency checker maps them to a conservative top
  node coupled to predicates, rdf:type classes, and the variable-class node. Dynamic
  head predicates emit only IRI bindings. The incremental relevance index mirrors the
  top node with read-any/head-any flags.
- **Co-head predicates** of one rule share a stratum (mutual positive edges), so a
  rule's stratum is well-defined.

## 4. Architecture (Phase 1, shipped with this record)

`crates/sparq-reason/src/datalog/` behind the **opt-in `datalog` feature** (lean-core
posture: default build carries zero of it; pulls only the substrate `rows`+`join`
slices, no new transitive crate):

- `parser.rs` — recursive-descent, interns constants into the caller's `Dict` at parse.
- `stratify.rs` — the checker; public `stratify(&Dict, &Program) -> Result<Stratification>`
  so tooling can validate without evaluating.
- `eval.rs` — non-incremental per-stratum fixpoint. Positive-atom and aggregate-table
  joins drive the SHARED substrate kernels (`build_table` + `hash_probe_serial`) via the
  same thin layout-adapter shape as `crate::substrate_join` / `n3::compiled` — no new
  join implementation. Aggregate tables are computed once per stratum entry (their
  bodies read strictly-lower strata); `NOT` is a per-row absence check against the
  stratum-complete predicate index.
- `oracle.rs` (test-only) — the **differential reference**: an independent naive
  substitution evaluator (no kernels, no indexes, `i128` integer compare) that must
  agree with the engine on every fixture and on seed-randomised graphs (deterministic
  LCG). Rationale: an in-tree independent-implementation oracle is the house pattern
  (`compiled_equivalence`, the Rust-vs-N3 ODRL differential).
- `souffle.rs` (test-only) — the **EXTERNAL-engine** differential arm (sq-xzb9p, §6.4):
  translates the relational fragment into Soufflé Datalog and compares closures. Soufflé
  is an external binary, never a cargo dependency, so the default build and the dependency
  graph are untouched; the fixtures skip (loudly) when it is absent.

Public API (5 items, each with a doctest + direct unit test): `parse_program`,
`Program::n_rules`, `stratify`, `Stratification::n_strata`, `eval`.

## 5. Historical Phase-1 scope and later status

- **Naive rounds within a stratum** (dedup at insertion) — correct, fixture-scale;
  semi-naive delta restriction is the next perf phase. No performance claims are made.
- Phase 1 shipped COUNT only; `SUM`/`MIN`/`MAX`/`AVG` shipped in sq-citho.
- `COUNT(DISTINCT ?v)`, float/double FILTER, grouped NOT, and variable predicates
  shipped together in sq-a7bmo. <!-- [GPT-5.6] -->
- **No incremental maintenance** — inserts/deletes re-evaluate; DRed/FBF-grade
  maintenance across strata is the BIG follow-up phase and must be SEQUENCED with the
  deletion-maintenance bead sq-6tykl.4 (same-crate collision, per the epic note).
  *(Since shipped as Phase 3, sq-4foq0 — see §6 item 3 and `datalog::incr`.)*
- **No CLI/`MaterializedGraph` wiring** — library API only. *(Both since shipped. The
  `MaterializedGraph`-style handle landed as Phase 3's `MaterializedProgram` (sq-4foq0, §6 item 3)
  — the same `new`/`insert`/`delete`/`contains`/`closure`/`len` shape as
  `MaterializedGraph`/`MaterializedOwlGraph`, plus a batched `update`. The CLI
  landed as sq-p4zci (§6 item 6): `--reason datalog:<rules.dlog>` on `query` and
  `reason <f> <fmt> datalog:<rules.dlog> [out.nt]`, behind the CLI's own opt-in `datalog` feature
  forwarding this crate's. The one-shot CLI path calls `eval`, not `MaterializedProgram` — there
  is nothing to maintain across a single process run.)* <!-- [SONNET-4.6] sq-p4zci -->
- The `datalog` feature IS now in `scripts/coverage.sh`'s per-crate measurement
  (sq-iwf3c): sparq-reason has a `measure()` case arm naming `--features datalog`, the
  `sparq-reason-el` `rbox,hasse` pattern, so the module is no longer compiled out of the
  crate's line-coverage denominator. The committed floor could NOT be re-seeded in that
  change (the authoring environment had no cargo-llvm-cov, so no honest with-datalog number
  could be taken there); it is the pre-datalog default-feature floor carried over the larger
  denominator, and the entry is marked `"seed_pending": true` to say so in the one place the
  gate reads. That flag makes CI, not the author, settle the seed: `coverage-gate.py --check`
  recomputes `floor(measured) - MARGIN` for a `seed_pending` entry and FAILS if it exceeds
  the committed floor, naming the number to write. So the carried-forward floor cannot land
  wrong in either direction — measuring below it fails as an ordinary floor breach, and
  measuring far enough above it fails as a stale seed; only a measurement that genuinely
  seeds the same floor passes. Editing `scripts/coverage.sh` is a full-run trigger, so
  `coverage ratchet (shard 2/3)` — the shard that owns sparq-reason — runs that check with
  `--features datalog` on the non-draft PR head and again in the merge queue. `--seed` clears
  the flag, because it rebuilds each entry from the measurement.

## 6. Phased decomposition (beaded)

1. **Semi-naive per-stratum evaluation + scale bench** (sq-8sve7) — delta-restricted positive
   joins (the `n3::compiled` `join_steps` discipline); measure before claiming.
2. **SUM/MIN/MAX/AVG aggregate functions** (sq-citho) — value slot on `AggAtom`, substrate `Num`
   tower for input values, overflow semantics decided against SPARQL's.
3. **Incremental maintenance under insert/delete across strata** (sq-4foq0) — SHIPPED:
   `MaterializedProgram` in `datalog::incr` — DRed (delete-and-rederive) for positive strata,
   stratum-boundary rederivation for `NOT`/`AGGREGATE` strata, predicate-level stratum
   skipping, differential-pinned against from-scratch `eval` on randomized insert/delete
   sequences. DRed over counting: counting is unsound under recursion without derivation-depth
   tracking. v1 honest scope: per-affected-stratum set/index bookkeeping is O(visible input)
   (no persistent deletable index); the incrementality is delta-driven rule-firing work,
   measured by deterministic counters. FBF-style over-deletion limits await the sq-6tykl.4
   deletion-heavy benchmark (profile first).
4. **External-engine differential arm** (sq-xzb9p) — SHIPPED, with a deliberately NARROW
   scope. `datalog::souffle` (test-only) translates a `Program` into Soufflé Datalog and
   compares closures over the same seed-randomised graphs the oracle arm uses; the optional
   `datalog-souffle.yml` lane (heavy tier, never a PR check-run) installs Soufflé and runs it
   with `SPARQ_DATALOG_SOUFFLE_REQUIRED=1` so a missing binary fails instead of skipping.
   *Tooling decision:* **Soufflé as an EXTERNAL BINARY, not a cargo dependency** — so the arm
   adds no crate and needs no cargo-vet audit at all (`crepe` would have been a new proc-macro
   dep plus a shared attestation; this sidesteps that sequencing entirely).
   *Why it is worth having on top of `oracle`:* the translation does NOT consult our
   `stratify()` — it emits one relation per dependency node and lets Soufflé stratify the
   program itself — so it checks BOTH directions (Soufflé must accept what we accept and
   refuse what we refuse) and can catch a stratification bug an in-tree oracle structurally
   cannot. The per-CLASS relation encoding is what makes Soufflé accept the class-granular
   negation of §3, so that decision is now externally corroborated too.
   *Honest scope:* the arm covers the RELATIONAL fragment only — constant-predicate atoms,
   recursion, single-atom and grouped `NOT`, multi-atom heads. `AGGREGATE`/`FILTER` and
   variable predicates/classes are OUT, and translating them is a loud error rather than a
   silent partial comparison: their semantics are term-level and XSD-typed, so encoding them
   into Soufflé's untyped-symbol domain would mean re-implementing the substrate numeric tower
   inside the translator, at which point the "external reference" would be running largely on
   our own code. Those paths keep the in-tree `oracle` differential. Extending the arm past
   the relational fragment is NOT beaded — it should stay unbeaded unless someone first shows
   a faithful encoding that does not import our numeric tower.
5. **Fragment extensions** (sq-a7bmo) — SHIPPED: grouped `NOT` / `NOT EXISTS`,
   projected `COUNT(DISTINCT ?v)`, relational float/double FILTER, and variable
   predicates with conservative top-node stratification and incremental relevance.
   <!-- [GPT-5.6] -->
6. **Surface wiring** (sq-p4zci) — SHIPPED: CLI `--reason datalog:<rules.dlog>` on `query` plus
   `reason <f> <fmt> datalog:<rules.dlog> [out.nt]`, behind sparq-cli's opt-in `datalog` feature
   (forwards `sparq-reason/datalog`; no new third-party dep). Reasoning runs at the same
   parse → index-build seam as `--reason rdfs`, so the default path is untouched; the profile is
   intercepted before the RDFS/RL profile parse (as `el` is) and, feature-off, is a hard exit-2
   error naming the feature with NO fall-back — RDFS/OWL-RL are monotone, so substituting one
   would silently drop `NOT`/`AGGREGATE`. Parse errors and stratification rejections are exit-1
   and name the construct / a predicate on the cycle. Worked examples in `skills/cli/SKILL.md`
   and `skills/inference/SKILL.md`; end-to-end tests in `crates/sparq-cli/tests/datalog_cli.rs`
   with the feature-OFF half in `tests/error_paths_cli.rs`, run by the
   `sparq-cli (datalog …)` feature-matrix leg. The `MaterializedGraph`-style handle this item
   also listed was ALREADY shipped by Phase 3 (`MaterializedProgram`, item 3), so no second
   handle was added. <!-- [SONNET-4.6] -->
7. **N3-compiled adoption of the checker** (sq-pi2k0) — replace the documented caller-discipline
   stratification of `n3::compiled` `log:notIncludes` with this checked stratification.
8. **Coverage measure-case wiring** (sq-iwf3c) — SHIPPED: the sparq-reason `measure()` case
   arm in `scripts/coverage.sh` names `--features datalog` (the `sparq-reason-el` pattern),
   so `src/datalog/` enters the crate's line-coverage denominator. Only `datalog` is named —
   the crate's other default-off features (explain/profile/d-entail/rif/compiled-rules/reify)
   are separately beaded. The floor is enforced against the expanded denominator by that
   change's own CI (shard 2 `--check-robust`), and the entry's `"seed_pending": true` makes
   that run also reject a floor looser than the measurement supports — so the re-seed is a
   gate, not a promise (see §5).
