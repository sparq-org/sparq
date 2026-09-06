# Gap record: W3C JSON-LD 1.1 self-conformance scoreboard (2026-07)

> 🤖 SPARQ agent — written by **Claude Fable 5** [FABLE-5]
>
> Bead **sq-w42s3** (epic sq-hmd7l, comparative-benchmarking-EVERYTHING).
> Companion to `research/gap-conformance-cross-engine-2026-07.md` (sq-hmd7l.22),
> whose JSON-LD row this record refreshes with a durable, re-runnable
> sparq-side runner.

## 1. What this record is

sq-hmd7l.22 found JSON-LD 1.1 **BEHIND** the peers' published EARL results, but
sparq's own row in that table was assembled from the CI floor consts by hand.
This record pins the durable **self-scoreboard runner** that produces the
sparq-side row from an actual run, and reconciles the per-operation verdicts
against the current estate (the floors have RISEN since .22 was written —
compact 186→228, frame 61→92 — so several .22 cells are stale-low).

**Runner:** `crates/sparq-jsonld/examples/jsonld_conformance.rs`

```sh
scripts/fetch-jsonld-tests.sh && scripts/fetch-jsonld-framing-tests.sh
cargo run -p sparq-jsonld --release --example jsonld_conformance             # table + fail listing
cargo run -p sparq-jsonld --release --example jsonld_conformance -- --json   # machine-readable row
cargo run -p sparq-jsonld --release --example jsonld_conformance -- --smoke  # pinned acceptance subset
```

It drives the **native `sparq-jsonld` document-level pipeline** (the crate's
public `expand` / `compact::compact` / `flatten` / `frame::frame` /
`from_rdf::from_rdf` entries) over the pinned W3C suites
(`w3c/json-ld-api` `8654ac22b6cf`, `w3c/json-ld-framing` `3bf782b`), and
emits per-operation `pass / fail / skip / total`, a TRUNCATED (never
rounded-up) pass-rate, and the complete failing-test listing. Full runs assert
the ratchet floors (`pass >= FLOOR`); `--smoke` asserts EXACT pinned counts on
the first-30-per-manifest subset (the bead's acceptance gate).

**Invariant (sq-w42s3):** every rate below comes ONLY from this runner's
output at the pins above; every failing test is listed; nothing is rounded up;
the BEHIND verdicts are preserved honestly. Conformance pass-rates are
correctness counts, not performance figures — no latency/throughput numbers
appear in this record.

## 2. Measured scoreboard (runner output, 2026-07-17, pins above)

**Superseded by §2a (2026-07-27, bead sq-gzsky) for the `expand` and `compact`
rows** — kept here as the before-picture the negative-lane rise is measured
against.

| operation | pass | fail | skip | total | rate (strict, skips count against) | rate over RUN tests |
|---|---|---|---|---|---|---|
| expand  | 276 | 0 | 109 | 385 | 71.6% | 276/276 = 100% |
| compact | 228 | 0 | 18  | 246 | 92.6% | 228/228 = 100% |
| flatten | 53  | 0 | 5   | 58  | 91.3% | 53/53 = 100% |
| frame   | 92  | 0 | 0   | 92  | 100.0% | 92/92 (negatives RUN) |
| fromRdf | 52  | 0 | 1   | 53  | 98.1% | 52/52 (negatives RUN) |
| toRdf   | — | — | — | — | NOT IMPLEMENTED natively | see §4 |

There are **0 outright fails** at the pin: every sub-100% strict rate is a
**skip bucket**, itemised below. (The 2026-07-12 snapshot had 1 fail — compact
`#t0038`, a JSON-LD-**1.0**-only positive the compact lane RAN rather than
skipped; bead sq-uzdw7 resolved it as an honest 1.0 skip — see the compact
skip-bucket below and `floors::compact`.)

## 2a. Re-measured after the negative lanes land (bead sq-gzsky, 2026-07-27)

Lever **sq-oy1f.31** in §6 — "the whole remaining expand gap" — is spent. The
`expand` and `compact` lanes now RUN every `NegativeEvaluationTest` in BOTH the
ratchet lane (`crates/sparq-conformance/tests/jsonld_suite/`) and this runner:
a negative passes iff the algorithm raises **exactly** the manifest's
`expectErrorCode`; a *wrong* code is a FAIL, never a pass. Same pins, same
document oracle, same command:

| operation | pass | fail | skip | total | rate (strict) | Δ vs §2 |
|---|---|---|---|---|---|---|
| expand  | 381 | 4 | 0  | 385 | 98.9% | +105 pass, 109 → 0 skip |
| compact | 243 | 2 | 1  | 246 | 98.7% | +15 pass, 18 → 1 skip |
| flatten | 53  | 0 | 5  | 58  | 91.3% | unchanged |
| frame   | 92  | 0 | 0  | 92  | 100.0% | unchanged (at the suite ceiling) |
| fromRdf | 52  | 0 | 1  | 53  | 98.1% | unchanged |

Wiring the negatives alone measured expand 371/14/0 and compact 242/3/1; seven
spec-faithful `sparq-jsonld` fixes closed 10 more of the revealed divergences
(`@included` arrayification, `@type`+`@direction`, datatype-IRI validation,
blank-node datatypes, the term round-trip against a keyword, `@container`
arrays in 1.0 mode, relative `@vocab` in 1.0 mode — itemised in
`floors::expand`). Floors raised **expand 276 → 381**, **compact 228 → 243**;
`frame` stays at 92 because 92/92 IS the pinned suite's ceiling.

**The 6 remaining fails are recorded as FAILS, not absorbed into a skip
bucket**, so they stay in the runner's complete failure listing:

- `expand #ter02`/`#ter03` (`recursive context inclusion`),
  `expand #ter24`/`#ter32` + `compact #te001` (`list of lists` /
  `compaction to list of lists`) — five cases expecting JSON-LD **1.0** error
  codes the 1.1 REC REMOVED from the `JsonLdErrorCode` registry (1.1 replaces
  the first with `context overflow` on a processor-defined limit and ALLOWS a
  list of lists outright). `sparq_jsonld::JsonLdErrorCode` is a deliberately
  CLOSED mirror of the 1.1 registry and cannot name them.
- `compact #te002` (`IRI confused with prefix`) — a REAL 1.1 gap: IRI
  Compaction step 8 must abort when the scheme of an authority-less IRI matches
  a `@prefix`-flagged term. `context::inverse::compact_iri` is infallible
  (`-> String`), so raising it needs a fallible signature threaded through the
  compaction walk; scoped as follow-up rather than folded into this rise.

This trades the §2 "0 outright fails" property for 120 fewer unverified skips
and a gated error-code registry — the honest direction, but the property is
genuinely gone and is not claimed below.
The document-level oracle is the pinned sq-kk1mq comparator (key order
insignificant; array order significant only inside `@list`; exact integral /
f64-fallback numerics) — the same oracle the `sparq-conformance` ratchet lane
asserts, ported onto the crate's own `Json` AST so this runner stays
dependency-free.

Skip-bucket composition (from the pinned manifests, no estimates):

- **expand: 109 skips = exactly the 109 NegativeEvaluationTests.** The
  expander raises spec error codes but error-code COMPLETENESS is unverified,
  so the negative lane is deferred (bead sq-oy1f.31) and honestly skipped —
  never counted as passes. This bucket is the entire expand gap.
- **compact: 18 skips = 17 NegativeEvaluationTests** (same deferral) **+ the
  one `specVersion: json-ld-1.0` positive (`#t0038`)**. sq-uzdw7 decision:
  `#t0038`'s 1.0-era expectation (compact-IRI creation through an expanded
  term definition) directly contradicts what `#tp001` pins for 1.0 processing
  mode under the 1.1 REC — both cannot pass, jsonld.js/pyld make the same
  trade, and the suite's `vocab.jsonld` defines `specVersion` as "the JSON-LD
  version to which the test applies" — so the lane skips it, NARROWLY PINNED
  to the exact manifest id (never a blanket `specVersion` match): the runner's
  `t0038_skip_is_narrowly_scoped` test asserts exactly `#t0038` is skipped,
  that the `processingMode: json-ld-1.0` cases of the 1.1 suite still run (and
  pass), and that a future 1.0-only positive at a suite-pin bump runs rather
  than being silently skipped.
- **flatten: 5 skips** = 1 negative + the JSON-LD-1.0-only positives + the
  post-flatten-compaction (`context`-member) compositions.
- **fromRdf: 1 skip** = the one `specVersion: json-ld-1.0` case (a 1.1
  processor intentionally does not reproduce the 1.0 partial-list algorithm);
  its 2 negatives are RUN and pass.

## 3. Cross-engine reconciliation (peer cells from sq-hmd7l.22, unchanged provenance)

Peer numbers are the official JSON-LD 1.1 EARL report subjects quoted in
`research/gap-conformance-cross-engine-2026-07.md` §3.5 (retrieved 2026-07-10;
report denominators are SMALLER — a real suite-version mismatch, so rates are
loosely comparable only).

Rates below are the §2a re-measurement (sq-gzsky) for `expand`/`compact`.

| operation | sparq (strict) | best published peers | verdict |
|---|---|---|---|
| expand  | 98.9% | jsonld-cpp / JSON-LD.ex 100.0, Titanium 98.1, jsonld.js 97.3 | **AT PARITY** with the mid-field at sparq's pin (denominators differ — loosely comparable); .22's 71.7 and §2's 71.6 both pre-date the negative lane. Residual = 4 retired-1.0-code fails |
| compact | 98.7% | JSON-LD.ex 99.6, Titanium 98.0, jsonld.js 97.5 | **AT PARITY**, from .22's stale 75.6 via 186→228→243; residual = 1 retired-1.0-code fail, the `IRI confused with prefix` gap, and the `#t0038` 1.0-only skip |
| flatten | 91.3% | listed peers 100.0 | **BEHIND** (small; 5 skips) |
| frame   | 100.0% | jsonld.js 97.8, Titanium 96.7 | **AHEAD at sparq's pin** (.22's 66.3 is stale — floor rose 61→92; denominator 92 vs report 91, loosely comparable) |
| fromRdf | 98.1% | Titanium / Sophia 98.1 (of 52) | **PARITY** (.22 cited the then-floor 51; current measured 52/53) |
| toRdf (native) | not implemented | jsonld-cpp 99.8, JSON-LD.ex 99.6 | **BEHIND** natively; engine-path toRdf is a separate 413/467 = 88.4% ratchet (§4) |

The epic-level verdict stays **BEHIND**, now on a single driver: **native toRdf
does not exist yet** (bead sq-oy1f.30 — see §4). Driver (a), the deferred
negative-test lanes, is spent for `expand` and `compact` (sq-gzsky, §2a); only
`flatten`'s 1 negative remains in that bucket. (Driver (c), compact `#t0038`,
was resolved earlier as an honest 1.0 skip per sq-uzdw7, still counted against
the strict rate.)

## 4. toRdf honesty note

`sparq_jsonld::to_rdf` is still the sq-oy1f.23 scaffold; the native
Deserialization-to-RDF algorithm is bead **sq-oy1f.30**. The ENGINE's toRdf
ingest (via `oxjsonld`) is ratcheted separately in `sparq-conformance`
(`floors::to_rdf`, 413/467 at the pin). The runner reports the native toRdf
row as `implemented: false` rather than borrowing the engine number — the two
are different implementations and conflating them would overstate the native
pipeline.

## 5. Relationship to the ratchet lane (no drift, no double ownership)

The CI ratchet of record remains
`crates/sparq-conformance/tests/jsonld_suite.rs` + the lib-side
`sparq_conformance::floors::<lane>` consts (sq-oy1f.40). This runner cannot
import those consts (sparq-conformance depends on sparq-jsonld, not vice
versa), so it carries documented mirrors asserted with FLOOR (`>=`) semantics
— an upstream ratchet rise never breaks it, a regression below the shared
floor fails loudly. Its `fromRdf` oracle is document-level only (the ratchet
lane adds a scoped oxjsonld round-trip leg), calibrated independently: at the
current pin both measure 52. No overlap with `bench/jsonld` (throughput,
sq-hmd7l.15/.43) or the cross-engine table script (sq-hmd7l.22).

## 6. Gap-closure levers (beads, not TODOs)

- **sq-oy1f.31** — negative-test lanes (error-code completeness) for
  expand/compact/flatten: worth up to +109 / +17 / +1 strict-rate points and
  is the whole remaining expand gap. **SPENT for expand + compact**
  (bead sq-gzsky, 2026-07-27 — §2a: +105 / +15 realised, floors 276 → 381 and
  228 → 243). `flatten`'s single negative is the only part still open.
- **sq-oy1f.30** — native toRdf (Deserialization to RDF); once landed, the
  runner grows a real toRdf row.
- **sq-uzdw7** — RESOLVED (2026-07-17): compact `#t0038` ("Index map
  round-tripping", json-ld-1.0-only positive), formerly the single outright
  failing test at the pin, is reclassified as an honest skip in BOTH the
  ratchet lane and this runner, NARROWLY PINNED to that exact manifest id
  (scope enforced by the ratchet lane's `t0038_skip_is_narrowly_scoped` test —
  a blanket `specVersion: json-ld-1.0` skip was rejected in PR review so no
  future 1.0-only positive is silently absorbed). "Fixing" it was rejected as
  unreachable: its 1.0-era prefixing expectation contradicts `#tp001` (a
  1.1-suite test run in 1.0 processing mode), so a REC-conformant processor
  cannot pass both. Floors unchanged at the time (compact stayed 228; fail 1→0,
  skip 17→18); the `#t0038` skip itself is unchanged by sq-gzsky.
- **`IRI confused with prefix` in IRI Compaction** — the one non-retired-code
  fail left (compact `#te002`, §2a). Needs `context::inverse::compact_iri` to
  become fallible so step 8 can abort. NEW, opened by sq-gzsky.
- **The 5 retired-1.0-error-code cases** (expand `#ter02`/`#ter03`/`#ter24`/
  `#ter32`, compact `#te001`, §2a) — a maintainer decision, not an
  implementation task: modelling them means adding JSON-LD **1.0** codes to
  `JsonLdErrorCode`, whose closed-enum rationale is that it mirrors the **1.1**
  registry exactly. NEW, opened by sq-gzsky.
