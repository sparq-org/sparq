# Value-level multi-oracle differential testing (design-for-review)

> Design record for bead **sq-qcnn.2** (epic **sq-qcnn**, the test-quality program). This is
> DESIGN + DECOMPOSE only — it specifies the architecture, the normalisation spec, the oracle
> abstraction and the divergence-triage policy, and enumerates the impl beads. It does **not**
> implement the harness. Author: SPARQ agent (Opus 4.8). [OPUS-4.8]

## Bottom line up front

The differential fuzzer (`crates/sparq-bench/src/fuzz.rs`) is the strongest correctness net sparq
has, but its **cross-oracle** check is cardinality-only: at `fuzz.rs:307` it compares
`sparq_engine::query(&g, &q).len()` against Oxigraph's solution **count**. It is therefore blind to a
whole bug class — a query that returns the right number of rows with a **wrong bound value** (wrong
number, wrong term, wrong order within a tie, wrong computed datatype). The recommendation is a
three-part upgrade, kept inside the existing nightly/bench harness (NOT the per-PR critical path):

1. Replace the cardinality-only cross-oracle check with a **canonical binding-multiset** comparison —
   term-by-term, order-insensitive for un-ordered results, order-sensitive-modulo-ties for `ORDER BY`
   — driven by an **engine-independent** normalisation library (the load-bearing design constraint:
   do not launder sparq's own value bugs through sparq's own comparator).
2. Extend the query/data generator to the datatypes and query forms it currently never emits
   (dateTime/duration/boolean, high-precision decimal, double INF/NaN, aggregates, BIND, string
   functions, CONSTRUCT/DESCRIBE/ASK-value).
3. Add a **second independent oracle** (Apache Jena via subprocess; rdflib optional) behind a
   pluggable `Oracle` trait, with an **N-way agreement** triage policy and an explicit,
   human-reviewed **allowlist** for spec-ambiguity / known oracle non-conformance — never a silent
   skip.

## 1. What the harness does today (verified against the code) — and three premise corrections

Verified against `crates/sparq-bench/src/fuzz.rs` as of commit `4d125edf` on `origin/main`.

**What exists (verified):**

- **`gen_graph`** builds a random 3–16-subject Turtle graph. Literal columns emitted: canonical
  non-negative `xsd:integer` (`ex:age`); a mixed `ex:val` column (inline integer, `xsd:int`,
  `xsd:decimal`, negative/non-canonical `xsd:integer`, `xsd:double`, an integer near 2^53, a
  high-precision decimal, plain string); and `ex:name` (plain **or** language-tagged string). Edges
  on `ex:p`.
- **`gen_query`** emits nine categories: `bgp`, `filter`, `equality`, `optional`, `union`, `minus`,
  `limit`, `distinct`, `order`. All are `SELECT` (or `SELECT *`).
- **Cross-oracle check (`run`, line 307):** `sparq_full != oxi` where `oxi = oxi_count(...)` is a
  **count** (`Solutions => count()`, `Boolean(_) => 1`, `Graph => count()`). **Cardinality only.**
- **Order check (`check_ordered`, lines 205–217, invoked 320–325):** for `ORDER BY` queries it does
  compare the ordered **sequence** element-for-element — but only of the single projected variable
  `?a`, printed via `Term::to_string()`, and only because the `order` generator emits exactly
  `SELECT ?a … ORDER BY ?a|DESC(?a)`.
- **Internal multiset check (`bindings_multiset`, lines 368–385, invoked 330–336):** a genuine
  term-by-term multiset comparison **already exists** — but it compares sparq's `query_json` output
  against sparq's `to_sparql_json(query())`. It is **sparq-vs-sparq** (serialiser consistency), never
  sparq-vs-oracle.
- **Count-path check (340–351):** sparq `count()` vs sparq `query().len()` — again sparq-internal.
- Extra store modes gated by env (`SPARQ_FUZZ_COMPRESS` / `_MMAP` / `_DICTSPILL`) re-run the same
  cases through alternative storage back-ends; deterministic seed repro throughout.

**Premise corrections (honesty gate):**

1. **The multiset machinery is not missing — its *cross-oracle* application is.** The bead frames the
   harness as cardinality-only; more precisely, a value-level multiset comparator (`bindings_multiset`)
   and an order-sensitive check (`check_ordered`) already exist but are wired **sparq-internally** (or,
   for `check_ordered`, to a single variable). The redesign is: *point the value-level comparison at
   the oracle*, and *generalise it* — not build it from nothing.
2. **`check_ordered` is correct only for the current narrow generator.** Element-for-element equality
   of a full `ORDER BY` result is **wrong in general**: SPARQL `ORDER BY` is a *partial* order, so rows
   equal on all sort keys (a "tie run") may appear in any relative order across engines. It happens to
   be safe today only because the generator orders by `?a` and projects `?a`, so tied rows are
   identical. Any multi-variable `ORDER BY` over a *subset* of projected variables would produce
   spurious mismatches. The value-level design must compare `ORDER BY` results **up to permutation
   within each sort-key-equivalence class** (see §2.2).
3. **The file/line anchors in the sibling beads point at an unmerged doc.** sq-rikm7 cites
   `compare.rs:160` and `research/fable-work-plan.md sec5.2`; neither exists on `origin/main`
   (`sparq-engine/src/compare.rs` is absent; the Fable work-plan is unmerged PR #1318). The real
   numeric-comparison seam is `sparq_engine::exec::num_compare` (`exec.rs:6306`) and the shared total
   order is `sparq_substrate::compare::compare_terms` (`compare.rs` under `sparq-substrate`). This
   record is written self-contained and does not depend on the Fable plan.

The bead's core claim — *the cross-engine check is cardinality-blind, the generator omits key
datatypes/forms, and a single Oxigraph oracle misses shared-assumption bugs* — is **correct**.

## 2. Term-by-term binding-multiset comparison (the core)

### 2.1 The comparison model

A SPARQL `SELECT` result is a **solution sequence** — a bag (multiset) of solution mappings, in an
implementation-defined order unless `ORDER BY` is present. `DISTINCT` is **not** implicit, so
duplicate solutions are semantically significant: the comparison is **multiset (bag) equality**, not
set equality. The neutral unit of comparison is one *solution* = an unordered map `{ var → term }`
over the projected variables; a term is one of IRI / blank node / literal `(lexical, datatype,
lang?)` / (RDF-1.2) triple term, plus the "unbound" marker for a projected-but-unbound variable
(which is distinct from bound-to-empty-string).

Two result forms need two comparators:

- **Un-ordered `SELECT` (no `ORDER BY`), and the un-ordered part of any query.** Canonicalise each
  solution to a stable string key (variables sorted; each term in canonical form per §3), sort the
  bag of keys, and require **exact multiset equality**. This subsumes and replaces the `fuzz.rs:307`
  count check (equal multisets ⇒ equal cardinality, plus the values).
- **`ORDER BY` results.** The projected+ordered sequence must match **up to permutation within each
  maximal run of sort-key-equal rows.** Algorithm: walk both sequences in lockstep; partition each
  into maximal runs where consecutive rows are equal on the *sort key expression list*; require the
  runs to line up (same length, same order) and each corresponding run-pair to be equal as a
  **multiset** (§2.1 un-ordered comparator). This is the correct generalisation of `check_ordered`
  and fixes premise-correction #2. Practically, the harness computes the sort-key equivalence from the
  query's `ORDER BY` clause; when in doubt it can inject a **total tiebreaker** (append every
  remaining projected var to `ORDER BY`) to make the order a function of the query, then demand exact
  sequence equality — the simpler and stricter option, preferred where the generator controls the
  query.

### 2.2 `LIMIT`/`OFFSET` without a total order is not differential-testable at value level

With `LIMIT`/`OFFSET` and **no** `ORDER BY` (or a non-total one), *which* rows survive is
implementation-defined — the result is not a function of the query, so a value-level cross-oracle
comparison is unsound (only the cardinality is well-defined, and even that only when the total
exceeds the offset+limit window). The current `limit` generator emits `… LIMIT k OFFSET off` with no
`ORDER BY`. Policy: for such queries the harness keeps a **cardinality-only** cross-oracle check, OR
the generator makes the order **total** (append a total tiebreaker) so the surviving rows *are*
determined and full value-level comparison applies. Recommend the latter for generated queries and
the former as the guard for any query that reaches the harness without a total order.

### 2.3 The independence trap (the load-bearing constraint)

If the harness canonicalises **both** sparq's and the oracle's results using **sparq's own** value
code (`sparq_substrate::numeric::Num`, `compare_terms`), then any bug in sparq's value model is
applied identically to both sides and **cancels** — the differential goes blind exactly where the
bead wants it to see (wrong bound values collapsing to equal canonical forms). Therefore:

- The normalisation library used to compare oracle outputs **must be independent of the
  engine-under-test's value code.** It parses the *neutral wire form* each engine emits (SPARQL
  Results JSON — `{type,value,datatype,xml:lang}` per binding) and canonicalises using **mature
  third-party** exact-arithmetic and temporal crates (e.g. `num-bigint`/`bigdecimal` for
  integer/decimal value equality, `time`/`chrono` for dateTime), **not** sparq's `Num`.
- The tension is real and must be stated: an independent normaliser is itself new, untested code and
  a potential source of *its own* bugs. Mitigations: (i) keep the normaliser small and
  direct-unit-tested (one test per public fn, per the coverage-ratchet rule); (ii) the **second
  oracle** (§5) is an orthogonal value model — three independent readings (sparq / Oxigraph-normalised
  / Jena-normalised) make a correlated normaliser bug far less likely to hide a real divergence.

## 3. Normalisation spec + subtle-correctness traps

The spec has **two regimes**, because the correct equality differs by term provenance.

### 3.1 Data-sourced terms → exact RDF term equality (with two RDF-1.1 quirks)

A variable bound *directly* to a term that appears in the input graph must be the **same RDF term**
in every engine — RDF does **not** canonicalise lexical form on parse, so `"01"^^xsd:integer` stays
`"01"^^xsd:integer`. Compare by `(kind, canonical-datatype-IRI, lexical, lang)` with **no numeric
canonicalisation**, modulo exactly two RDF-1.1 equalities the serialiser may render either way:

- **Simple literal ≡ `xsd:string`.** RDF 1.1: a literal with no datatype and no language tag *is* an
  `xsd:string`. `"abc"` and `"abc"^^xsd:string` are the **same term**; the normaliser folds the
  absent datatype to `xsd:string` before comparison.
- **Language tags compare case-insensitively.** BCP-47 tags are case-insensitive; `"chat"@en` and
  `"chat"@EN` are the same term. The normaliser **lowercases** the tag before comparison. (sparq
  already lowercases tags at `exec.rs:7155`/`7555`; the normaliser must do so *independently* — the
  independence trap.)

Consequence worth stating: two dateTimes that are the same *instant* but different *lexical*
(`…T13:00:00Z` vs `…T14:00:00+01:00`) are **different RDF terms** and, as data-sourced bindings, must
compare **unequal**. Value-equality of instants only matters for the *decision* regime (§3.3).

### 3.2 Numeric-value traps (for the decision regime and computed terms)

These are where cardinality-blindness bites — a wrong `FILTER`/`ORDER BY`/aggregate decision changes
*which* rows or *what* order, not the lexical form of a data-sourced term:

- **`xsd:integer` beyond `2^53`** (e.g. `9007199254740993`): `f64` collapses distinct integers. The
  independent normaliser must compare as **arbitrary-precision** integers (`num-bigint`). Note the
  in-engine boundary this cross-references: sparq's exact tier is `Dec { mant: i128, … }`
  (`numeric.rs`), so sparq itself falls back to `f64` beyond **i128**, not merely 2^53 — the
  generator should be able to emit integers beyond i128 to probe that far boundary. **Cross-ref
  sq-rikm7** (the `ORDER BY`/`MIN`/`MAX` exact-recheck seam: `num_compare` rechecks via `to_dec()`
  but the `compare.rs` ordering path historically did not — an asymmetry this harness would catch).
- **High-precision decimals sharing an `f64`** (`0.123456789012345678` vs `…679`): compare as exact
  **`bigdecimal`**. The generator already emits these; the *comparator* must be exact.
- **`xsd:decimal` canonical form**: value-equal lexical variants (`1.0`/`1`, `+1`/`1`, `01.50`/`1.5`,
  trailing zeros). For **data-sourced** terms these stay distinct RDF terms (§3.1); for **computed**
  terms both engines should emit the canonical form and any residual difference is decimal-canonical
  normalisation, handled here.
- **`xsd:double`/`xsd:float` canonical lexical** (`6` vs `6.0E0`, `INF`/`-INF`/`NaN`): the mantissa/
  exponent canonical form differs across engines. Normalise doubles by **value** (with `NaN`
  self-equal for multiset bucketing, and `+0.0`/`-0.0` treated per the chosen policy) and by the XSD
  canonical lexical for term comparison. **Cross-ref sq-rkzhr** (sparq's `as_num` currently rejects
  XSD `INF`/`-INF`/`NaN` that the typed path accepts, and `fmt_xsd_double` prints integral doubles
  non-canonically) — a live per-surface policy question this harness must not silently paper over.

### 3.3 Value regime: dateTime / duration / timezone

Used for `FILTER`, relational `<`/`=`, `ORDER BY`, `MIN`/`MAX`, and computed comparisons:

- **`xsd:dateTime`/`xsd:date` value equality** = same instant on the timeline; `Z` vs `+00:00` equal;
  offset-shifted-but-same-instant equal *as values*. Compare via an independent temporal crate.
- **Timezone-less vs timezoned** is the **±14h indeterminate window** (XPath): a bare dateTime
  compared to a timezoned one can be *indeterminate*, and engines may substitute an implicit
  timezone differently → a legitimate divergence source → **triage**, not auto-fail (§5.2).
- **`xsd:duration` is only *partially* ordered.** `P1M` (one month) vs `P30D` (30 days) are
  **incomparable**; `xsd:yearMonthDuration` vs `xsd:dayTimeDuration` cross-comparisons are undefined.
  A generated `ORDER BY`/`<` over durations can legitimately differ or *error* across engines →
  triage. The generator should emit durations but the comparator must expect partial-order gaps.

### 3.4 Blank-node isomorphism across oracles

Blank-node **labels are engine-local and arbitrary** — Oxigraph's `_:b0` and sparq's `_:c14n2` may
denote "the same" node. So a solution multiset (or a CONSTRUCT/DESCRIBE graph) containing blank nodes
cannot be compared by label; it needs a **consistent bijection** between the two engines' blank nodes
under which the whole result matches. This is solution-set / graph **isomorphism** (NP-hard in
general, tractable in practice via canonical labelling — RDFC-1.0 / URDNA2015-style):

- **CONSTRUCT/DESCRIBE (result is a Graph):** canonicalise both graphs with an RDF dataset
  canonicalisation algorithm and compare canonical N-Triples. Prefer an **independent** `rdf-canon`
  implementation over reusing either engine's canonicaliser (independence trap again).
- **`SELECT` projecting blank nodes:** the whole solution table shares blank-node identity across its
  rows (blank-node scope is the result), so it must be canonicalised as a structure, not row-by-row.
  This is genuinely harder; **v1 scoping recommendation:** detect blank nodes in a result and **route
  the case to triage** (explicit, counted — never a silent skip) until the graph-canonicalisation
  phase lands. Honest: this is a real coverage gap in the first cut.

**Status (bead D / `sq-qcnn.7`, landed):** both comparators now exist in
`crates/sparq-difftest/src/iso.rs` — `canonical_graph`/`graph_isomorphic` for graph results, and
`canonical_solutions`/`solutions_isomorphic` for a `SELECT` table (reified one-row-node-per-row, so
duplicate rows and cross-row blank-node sharing are both preserved). The labelling is the
third-party `rdf-canon` crate, **not** sparq's own `sparq-canon`, per the §2.3 independence trap.
The gap is NOT yet closed end-to-end: `crates/sparq-bench`'s Oxigraph fuzzer predates the
`sparq-difftest` wiring (bead B / `sq-qcnn.5`) and so still routes a blank-node answer to triage —
now under its own counted `bindings_triage(bnode)` bucket rather than folded into the row-choice
skip, so the remaining gap is a visible number.

## 4. Generator coverage extension

Extend `gen_graph`/`gen_query` (the value comparator makes these *meaningful*; without it they would
still only exercise cardinality):

- **Typed data literals:** `xsd:dateTime`/`xsd:date` (varied timezones incl. bare, and the ±14h
  window), `xsd:duration`/`xsd:yearMonthDuration`/`xsd:dayTimeDuration` (incl. incomparable pairs),
  `xsd:boolean`, `xsd:decimal` (incl. high-precision and value-equal lexical variants),
  `xsd:double`/`xsd:float` (incl. `INF`/`-INF`/`NaN`), and integers **beyond i128**.
- **Aggregates:** `GROUP BY` with `COUNT`/`SUM`/`AVG`/`MIN`/`MAX`/`SAMPLE`/`GROUP_CONCAT` (note:
  `SAMPLE` is non-deterministic → its bound value is not differential-testable; test the *group set*
  and the deterministic aggregates, and either project `SAMPLE` into triage or omit it).
- **`BIND` / expression projection:** arithmetic (probes numeric-tower promotion + the sq-rkzhr
  double-canonical question), `IF`/`COALESCE`, datatype constructors.
- **String functions:** `CONCAT`, `SUBSTR`, `STRLEN`, `UCASE`/`LCASE`, `REPLACE`, `REGEX`,
  `STRBEFORE`/`STRAFTER`, `LANG`/`DATATYPE`/`STR` (surface xsd:string vs simple-literal handling).
- **Result forms:** `CONSTRUCT`, `DESCRIBE` (both need §3.4 graph canonicalisation), and `ASK` —
  where the current oracle is doubly blind: `oxi_count` maps `Boolean(_) => 1` regardless of truth
  value, so **ASK false is indistinguishable from ASK true**. The value comparator must compare the
  **boolean itself**. CONSTRUCT is currently triple-**count**-only; it must compare the canonical
  graph.

Generation stays **deterministic** (the existing SplitMix64 seed model) so every case has a
`seed`-only repro — a non-negotiable property to preserve.

## 5. The second oracle + divergence triage

### 5.1 A pluggable `Oracle` trait

Introduce a small trait so oracles are interchangeable and the harness compares *neutral normalised
results*, not engine-specific types:

```rust
/// A neutral, engine-independent SPARQL result (produced by §3 normalisation).
enum OracleResult { Solutions(CanonMultiset), Graph(CanonGraph), Boolean(bool) }

trait Oracle {
    fn name(&self) -> &str;
    /// Evaluate `query` over `data` (N-Triples/Turtle); Err = the oracle could not run it
    /// (parse/feature-unsupported), which is a SKIP-for-this-oracle, not a divergence.
    fn eval(&self, data: &str, query: &str) -> Result<OracleResult, OracleError>;
}
```

- **Oxigraph** — the existing in-process oracle, refactored to this trait. Already a dev-dep
  (`oxigraph = "0.5"`, `rdf-12`); no new runtime dependency.
- **Apache Jena (recommended second oracle)** — a subprocess adapter shelling to a tiny Java harness
  (jena-arq) that reads data+query and emits SPARQL Results JSON / N-Triples on stdout. Jena is the
  most mature *independent* SPARQL 1.1 engine and shares **no lineage** with Oxigraph — exactly the
  property that breaks "shared-assumption" bugs. Cost, stated honestly: needs a **JVM** and a pinned
  jar in the differential lane (not on the per-PR critical path), plus subprocess marshalling and
  startup latency.
- **rdflib (optional third)** — pure-Python subprocess, cheap to provision, but rdflib's SPARQL is
  **less conformant** (known aggregate/function gaps) → more spurious divergences → more triage. Keep
  as an *optional* extra oracle, not the primary second one.

All non-Oxigraph oracles are **opt-in** (feature/env gated) so the default `cargo test`/bench run
needs no JVM/Python; the nightly differential lane provisions the toolchain (matching the existing
nightly-fuzz/miri posture — the per-PR `fuzz` gate is the separate cargo-fuzz libFuzzer target set in
`fuzz/`, not this harness).

### 5.2 Honest divergence triage (N-way agreement + reviewed allowlist)

Run sparq + Oxigraph + (opt) Jena on each case; classify the normalised results:

- **All agree** → pass.
- **sparq disagrees with the oracles that agree with each other** → strong signal of a **sparq bug**
  → **FAIL** with deterministic repro (seed + query + graph + the three normalised results).
- **The oracles disagree with each other** (sparq may side with either) → **spec-ambiguity** or
  **oracle-specific non-conformance**, *not* auto-attributable to sparq → recorded to an explicit,
  **human-reviewed allowlist**, **never a silent skip**.

The allowlist is a checked-in file (the idiom of the existing conformance floors / known-fails),
each entry scoped as tightly as possible and carrying a **reason** and, where applicable, an upstream
issue link. Two distinct kinds, kept separate:

- **Known oracle non-conformance** — "sparq + Oracle A are right; Oracle B is known-wrong here"
  (keyed by oracle + feature; ideally an upstream bug link). sparq is still *checked* against the
  conformant oracle.
- **Genuine spec-ambiguity** — e.g. the timezone-less dateTime implicit-timezone window (§3.3), a
  non-deterministic `SAMPLE`, an underspecified `xsd:double` canonical form pending the sq-rkzhr
  policy decision (keyed by spec clause).

Adding an allowlist entry is a **reviewed** act with a written reason — it is emphatically not the
harness auto-suppressing any case that happens to diverge. The report prints counts per bucket so
the allowlist's size is always visible (an anti-complacency ratchet, like the parity floors).

## 6. Where it lives and how it gates

Extend the **existing** `sparq-bench fuzz` harness (a bench binary run in a nightly/dedicated lane),
not a new crate. Oxigraph value-level comparison is cheap enough for a per-PR **smoke** slice; the
Jena/rdflib oracles and the heavier datatype/aggregate space run in the **nightly** differential lane.
Gate model, following the repo's ratchet idiom: **0 divergences modulo the reviewed allowlist**, with
the harness printing a machine-greppable summary line (extending the current
`fuzz[cat] … full_mismatch=… count_mismatch=…` line) so a CI step can assert the floor. This is the
gating follow-on, decomposed as a distinct bead so the design/build lands before the CI enforcement.

## 7. Recommendation

Build the value-level comparator on an **engine-independent** normalisation library (§2.3/§3), point
it at Oxigraph first (no new dependency), then add **Jena** as the second oracle behind the `Oracle`
trait with N-way triage. Sequence blank-node isomorphism (§3.4) and the CI ratchet as later phases so
the core value-level win lands early. Keep everything in the existing nightly harness — this is a
correctness net, not a per-PR latency cost.

## 8. Phased plan (ordered future beads)

Created under epic `sq-qcnn`, each `--deps sq-qcnn.2` (this record) plus the inter-bead deps below.
The `A–G` tags are this record's internal labels; real bead ids are listed in the PR body.

1. **(A) normalisation-lib** — the engine-**independent** value-normalisation + canonical-result
   library: parse SPARQL-Results-JSON into the neutral `{var→term}` model; RDF term equality with
   simple-literal≡`xsd:string` folding and lang-tag lowercasing (§3.1); exact integer/decimal value
   equality via `num-bigint`/`bigdecimal`; `xsd:double`/`float` value + canonical-lexical handling
   incl. INF/NaN (cross-ref sq-rkzhr); dateTime/duration value comparison with timezone + the ±14h
   indeterminate marker (§3.3); the **multiset** comparator and the `ORDER BY`
   sort-key-equivalence-class comparator (§2.1). One direct unit test per public fn (coverage
   ratchet). *No deps beyond sq-qcnn.2. Prereq for all others.*
2. **(B) multiset-comparator (harness wiring)** — replace the `fuzz.rs:307` cardinality check with the
   canonical binding-multiset comparison against Oxigraph via (A); generalise `check_ordered` to
   compare `ORDER BY` up to within-equivalence-class permutation over **all** projected vars (fix
   premise-correction #2); guard `LIMIT`-without-total-order as cardinality-only or inject a total
   tiebreaker (§2.2); compare ASK booleans and CONSTRUCT triple **sets** not counts. *Deps: A.*
3. **(C) generator-extension** — extend `gen_graph`/`gen_query` per §4 (typed literals incl.
   dateTime/duration/boolean/high-precision-decimal/double-INF-NaN/beyond-i128 ints; aggregates;
   BIND; string functions; CONSTRUCT/DESCRIBE/ASK), preserving deterministic seed repro. *Deps: B
   (so new forms are value-checked, not just counted).*
4. **(D) blank-node-isomorphism** — canonical labelling (independent RDFC-1.0/`rdf-canon`) for
   CONSTRUCT/DESCRIBE graph results and for `SELECT` results projecting blank nodes (§3.4); until it
   lands, blank-node results are explicitly counted into triage, never silently skipped. *Deps: A
   (wires through B).*
5. **(E) oracle-trait + 2nd-oracle-adapter** — define the pluggable `Oracle` trait (§5.1); refactor
   Oxigraph onto it (in-process); add the **Jena** subprocess adapter (Java harness → SPARQL-JSON)
   behind an opt-in feature/env; optional rdflib adapter. *Deps: A (needs the neutral result form).*
6. **(F) divergence-triage** — the N-way agreement classifier (§5.2), the checked-in reviewed
   allowlist (oracle-non-conformance vs spec-ambiguity, with reasons/upstream links), the reporting
   that distinguishes "sparq vs agreeing-oracles" (fail) from "oracles disagree" (triage), and the
   per-bucket counted summary line. *Deps: B, E.*
7. **(G) ci-lane ratchet** — wire the value-level multi-oracle differential as the nightly lane
   (Oxigraph value-level smoke per-PR; Jena/rdflib nightly), asserting **0 divergences modulo
   allowlist** off the summary line (§6). *Deps: B, F.*

DAG: `A → B → {C, F, G}`, `A → {D, E}`, `E → F`, `B,F → G`.

## 9. Honest risk / limitations (no overclaim)

- **The independent normaliser is new code and a bug surface of its own** (§2.3). It is only as good
  as its own tests; the second oracle is the structural mitigation. Do not present the harness as
  "proving" correctness — it raises the cost of a value bug surviving, it does not certify absence.
- **Blank-node `SELECT`-result isomorphism**: the comparator landed with bead (D) in
  `sparq-difftest::iso` (§3.4), but the `sparq-bench` fuzzer does not call it until bead (B) wires
  `sparq-difftest` in; until then those cases are still triaged, under their own counted bucket.
  A stated coverage gap that is now measured, not a solved problem.
- **A second oracle is not an oracle of truth.** Jena and rdflib are *implementations*, not the spec.
  A divergence between oracles is triaged, never auto-read as "sparq is wrong". The allowlist must
  stay small and reasoned or it silently erodes the net — hence the visible per-bucket count.
- **JVM-in-CI cost is real** for the Jena oracle; it belongs in the nightly lane, off the per-PR
  critical path. rdflib is cheaper but less conformant (more triage).
- **Non-deterministic constructs** (`SAMPLE`, `LIMIT`/`OFFSET` without a total order, the ±14h
  dateTime window) are **not** value-differential-testable and are handled by exclusion/tiebreaker/
  triage, not by pretending they are deterministic.
- **No performance claims** are made here; any timing from this work box is non-canonical.
- **Scope:** this strengthens sparq's *value-level* correctness net. It does not touch, and makes no
  claim about, any ZK/MPC privacy or soundness property.

## 10. Open questions for the maintainer

1. **Second-oracle choice — Jena vs rdflib as the *primary* second oracle.** Jena is more conformant
   but needs a JVM in the nightly lane; rdflib is trivially provisioned but less conformant (more
   triage). Recommendation: **Jena primary, rdflib optional**. Confirm the JVM-in-nightly appetite.
2. **`xsd:double` canonical-form policy (blocks part of §3.2/§4).** The double-canonical /
   INF-NaN-routing question is the subject of **sq-rkzhr** and is a genuine per-surface policy call.
   Should the normaliser compare doubles purely by **value** (side-stepping the lexical policy), or
   also assert the XSD canonical lexical (making the harness enforce sq-rkzhr's outcome)?
3. **Allowlist governance.** Who reviews an allowlist addition, and should each spec-ambiguity entry
   be required to carry an upstream issue link (as the Solid differential-oracle record proposes for
   its JS twin)?
4. **CI placement of bead (G).** Value-level Oxigraph comparison per-PR smoke + full multi-oracle
   nightly, or all-nightly to keep the per-PR gate minimal?

## Cross-references (do not duplicate)

- Harness under design: `crates/sparq-bench/src/fuzz.rs` (this record's subject).
- Numeric seam beads this harness would exercise: **sq-rikm7** (ORDER BY/MIN/MAX exact-recheck
  total-order seam) and **sq-rkzhr** (xsd:double canonical lexical + INF/NaN routing).
- Shared total order the engine uses: `sparq_substrate::compare::compare_terms`
  (`crates/sparq-substrate/src/compare.rs`); numeric tower: `sparq_substrate::numeric::{Num, Dec}`.
- Prior differential-oracle design pattern (triage/allowlist/2nd-oracle idiom):
  [`research/solid-acp-differential-oracle-design.md`](./solid-acp-differential-oracle-design.md).
- Epic: **sq-qcnn** (test-quality program — cargo-mutants ratchet + coverage floors).
