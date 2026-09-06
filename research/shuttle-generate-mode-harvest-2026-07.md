# Shuttle generate-mode harvest: measuring the improvement thesis (sq-tonhr.12)

> 🤖 SPARQ agent [OPUS-5], 2026-07-28. Bead **sq-tonhr.12** (issue #3040), child of the
> Shuttle epic **sq-tonhr** (design record:
> [`shacl-compact-and-shuttle-parsers-2026-07.md`](./shacl-compact-and-shuttle-parsers-2026-07.md)).
> Sibling records: the go/no-go measurement
> ([`shuttle-gen-rs-go-no-go-2026-07.md`](./shuttle-gen-rs-go-no-go-2026-07.md)), the backend-gap
> verification ([`shuttle-gen-rs-backend-gaps-2026-07.md`](./shuttle-gen-rs-backend-gaps-2026-07.md)),
> and the print-mode generalization ([`shuttle-generic-print-mode-2026-07.md`](./shuttle-generic-print-mode-2026-07.md)).
>
> **What this record adds:** the improvement thesis behind the whole Shuttle program — *generated
> conformance suites catch corner cases hand-written parsers miss* — had exactly ONE precedent:
> three RDF 1.2 pending-reifier conformance failures that coverage-directed sampling of the
> `annotation` production found in "a current, widely-used hand-written parser" (upstream `README.md`;
> the bead names it as N3.js, but **upstream leaves it unnamed**, and the pointer in
> `grammars/turtle12.shuttle` to "spec §8" does not resolve — that passage carries no such claim).
> This bead makes the thesis **measurable** and reports the first independent measurement.

## Result in one paragraph

An interim generate-mode harness ([`scripts/shuttle-generate-harvest.mjs`](../scripts/shuttle-generate-harvest.mjs))
derived a coverage-directed corpus from the upstream `turtle12` Shuttle grammar and ran it
differentially against **Oxigraph 0.5.9**, whose Turtle parser *is* `oxttl 0.2.3` — the exact
version sparq pins (`Cargo.lock`: `oxigraph 0.5.9 → oxrdfio 0.2.5 → oxttl 0.2.3`). One seeded run
of 400 generated documents produced **67 divergences that collapse to 2 distinct root-cause
defects**, both **confirmed by reading `oxttl`'s source** (not inferred from behaviour), both
**still present on oxigraph `main`**, and both **legal syntax per the W3C RDF 1.2 Turtle grammar**.
Both live in RDF-1.2-only corners (`reifier`, triple-term/reified-triple object position). Neither
has been **filed**; both are written up as ready-to-submit reports in
[`docs/upstream-proposals.md`](../docs/upstream-proposals.md) § E, the same way § A/§ B stage
oxigraph and rdf-tests submissions.
So the thesis now has support beyond its single precedent: **2 further real parser defects, in a
different parser family, from one seed** — with the honest caveats in *Limits* below, chief among
them that this is a lower bound from an interim harness, not the spec's generate mode.

## Why an interim harness at all

`spec/SHUTTLE.md` §9 derives a third artifact from a grammar: a conformance-pair generator
producing coverage-directed (document, expected-quads) pairs *valid by construction*, plus two
provably-sound negative families (LL-table mutants, per-`shtl:errorCode` semantic negatives),
emitted as a W3C-format `manifest.ttl`.

**That mode is not implemented upstream.** Verified at `jeswr/rdf-shuttle`
`89bbe6d2ee38b4ec3379f39f0c8bd25894f8efac` (the same pin
[`shuttle-gen-rs-backend-gaps-2026-07.md`](./shuttle-gen-rs-backend-gaps-2026-07.md) reads; sparq's
own vendored artifacts pin the older `b7801c5` in `crates/sparq-shaclc/src/provenance.rs`): there is
no generator source file in either backend's `src/` (`clausec`, `generate`, `lexer-gen`,
`parser-gen`, `residual-serializer-gen`, `serializer-gen` only), and neither `cli.js` accepts an
`--emit` flag — the JS CLI's own usage string is
`shuttle-gen-js <grammar.shuttle> [-o out.js] [--profile a,b]`.

Waiting for it would leave the thesis unmeasured. The interim harness therefore reuses what upstream
*does* ship — the front-end grammar AST (`packages/gen-js/src/meta.js` `parseGrammar`), the
generated reference parser (`packages/gen-js/generated/turtle12.js`), and the isomorphism checker
(`packages/gen-js/test/iso.js`) — and supplies only the derivation, the terminal sampling and the
differential comparison.

## What the harness does (and what it therefore cannot claim)

| stage | spec §9 generate mode | this interim harness |
|---|---|---|
| positive documents | run the relation with nothing ground → valid **by construction** | derive over (production × alternative) with **sampled** terminals, then keep only documents the grammar's own generated parser accepts (**oracle-filtered**, not construction-valid) |
| expected quads | co-derived with the document | the **oracle's** parse output (so a bug in the oracle is invisible here) |
| coverage direction | production × alternative × token-boundary bucket × print-guard both ways | production × alternative (+ the grammar's `@covers` labels); token-boundary buckets only insofar as the hand-written hazard pools reach them |
| terminal sampling | boundary-biased, derived from the token patterns | boundary-biased **hand-written pools** per token (escape classes, PN_LOCAL dot/colon/percent, long-string quote runs, `--rtl` dir tags, numeric sign/exponent corners) |
| negatives | LL-table mutants (**provably** outside the language) + per-`errorCode` semantic negatives | token-level edits kept only if the oracle rejects them (**oracle-rejected**, a strictly weaker property), bucketed by the error code the oracle raised |
| output | W3C-format `manifest.ttl` + pairs | same layout (`positive/*.ttl` + `.nt`, `negative/*.ttl`, `manifest.ttl` naming each via `mf:action`) — the shape `sparq-conformance`'s `differential::run_suite_actions` already walks |

Consequence, stated plainly: **every divergence this harness reports needs human triage before it is
anyone's bug.** The two defects below were each reduced to a hand-written minimal repro, checked
against the W3C grammar, and confirmed in `oxttl`'s source — that is what promotes them from
"harness output" to "finding".

The harness carries its own non-vacuity proof: `--self-test` runs the whole pipeline against two
seeded MUTANT comparators (one drops a quad from every parse, one accepts everything the oracle
rejects) and exits non-zero unless **both** are caught. It is the executable answer to "would this
harness notice if it stopped comparing anything?".

## Reproduction

```sh
git clone https://github.com/jeswr/rdf-shuttle /tmp/rdf-shuttle   # pin 89bbe6d
npm pack oxigraph@0.5.9 && tar xzf oxigraph-0.5.9.tgz             # ships oxttl 0.2.3
node scripts/shuttle-generate-harvest.mjs --shuttle /tmp/rdf-shuttle --self-test
node scripts/shuttle-generate-harvest.mjs --shuttle /tmp/rdf-shuttle \
     --seed 7 --count 400 --mutants 800 \
     --compare oxigraph:./package/node.js --report report.json --out corpus/
```

Node only — no Rust toolchain is required, and none was available in the environment that produced
this record (see *What is deliberately not done*).

## The measured run (seed 7)

| quantity | value |
|---|---|
| documents derived / kept as positives | 400 / 400 (the oracle rejected none) |
| oracle-rejected mutants kept as negatives | 691 |
| negative error-code buckets | `UNEXPECTED_TOKEN` 633, `UNDECLARED_PREFIX` 58 |
| top-level (production × alternative) coverage | **74 / 74** |
| `@covers`-labelled print-guard alternatives reached | all 5 (`obj-collection`, `obj-triple-term`, `obj-reified-triple`, `obj-bnpl`, `obj-bnode-label`) |
| divergences vs Oxigraph 0.5.9 — positives | **67**, all *candidate-rejects* |
| divergences vs Oxigraph 0.5.9 — quad-set (both accept, different graph) | **0** |
| divergences vs Oxigraph 0.5.9 — negatives (candidate accepts what the oracle rejects) | **0** |
| distinct root causes behind the 67 | **2** (30 + 37, no residue) |

A second seed (11) over 200 documents reproduces the run shape exactly — 74/74 alternatives, 32
divergences, all *candidate-rejects*, splitting 14 / 18 across the **same two** root causes, with no
third class and again no quad-set or negative-corpus divergence.

### The harness's own false positives — found, fixed, and reported

The first run of this harness reported **177** divergences: the 67 above plus **110 quad-set
divergences that were the harness's fault**. It compared through Oxigraph's `Store.load()` +
`match()`, and the *store* canonicalizes numeric literal lexical forms — `"1.0e+2"^^xsd:double`
comes back as `"100"`, `"-0.0"^^xsd:decimal` as `"0"`. That is a store design choice, not parser
behaviour: `oxigraph.parse()` (raw `oxttl`) preserves the lexical form exactly. The comparator now
uses `parse()`, and the quad-set divergence count went 110 → 0.

This is recorded rather than quietly fixed because it is the honest shape of the work: **62% of the
first raw count was harness artefact.** Any future "N divergences found" number from a generated
corpus should be assumed to contain such a fraction until each class has been reduced and confirmed.

## The two findings

Both are **`oxttl` defects**, confirmed in the released source (`oxttl 0.2.3`, the newest release —
crates.io has nothing above it) and re-confirmed against `oxigraph/oxigraph` `main`
(`lib/oxttl/src/terse.rs`, fetched 2026-07-28). W3C productions quoted below are from
<https://www.w3.org/TR/rdf12-turtle/>, fetched the same day.

### F1 — an anonymous blank node is not accepted as a reifier (`~ []`)

```turtle
ex:s ex:p ex:o ~ [] .                    # oxttl: "A dot is expected at the end of statements"
<< ex:s ex:p ex:o ~ [] >> ex:q ex:r .    # oxttl: "Expecting '>>' to close a reified triple, found ["
```

The grammar says `reifier ::= '~' (iri | BlankNode)?` and `BlankNode ::= BLANK_NODE_LABEL | ANON`,
so `~ []` is legal. Controls that DO parse in `oxttl`: `~ _:r`, `~ ex:r`, and a bare `~`. Source:
`TriGState::Reifier` matches `IriRef`, `PrefixedName` and `BlankNodeLabel` and has no
`Punctuation("[")` arm; the `_` fallback treats `[` as "no reifier written", mints a fresh blank
node, and re-dispatches `[` into a state that cannot accept it — which is why the error surfaces at
the *next* token rather than at the `[`. 30 of the 67 divergences are this class.

### F2 — long-quoted string literals are rejected in triple-term / reified-triple object position

```turtle
ex:s ex:p <<( ex:a ex:b """ab""" )>> .   # oxttl: '"""ab""" is not a valid RDF quoted triple object'
<< ex:s ex:p '''ab''' >> ex:q ex:r .     # same, for the reified-triple form
```

`ttObject ::= iri | BlankNode | literal | tripleTerm` and
`rtObject ::= iri | BlankNode | literal | tripleTerm | reifiedTriple`, and
`String ::= STRING_LITERAL_QUOTE | STRING_LITERAL_SINGLE_QUOTE | STRING_LITERAL_LONG_QUOTE |
STRING_LITERAL_LONG_SINGLE_QUOTE` — so a long-quoted literal is legal in both positions. Controls
that DO parse: the same literals in ordinary object position, and `"ab"` / `'ab'` inside the same
`<<( )>>`. Source: `oxttl`'s lexer emits two distinct tokens, `N3Token::String` and
`N3Token::LongString`; the ordinary object state matches `String(value) | LongString(value)`, while
`TriGState::ReifiedTripleObject` — used for **both** `<< … >>` and `<<( … )>>` objects — matches
`String(value)` only. 37 of the 67 divergences are this class.

Both fixes are small and local — a missing match arm each, with F1's arm additionally reusing the
`TriGState::QuotedAnonEnd` state the neighbouring subject/object states already use to consume
`'[' WS* ']'`. Every repro and control above was executed at statement level *and* inside `<< … >>`
(`~ [ ]` with internal whitespace fails identically to `~ []`). Ready-to-submit reports (repro,
control, source pointer, proposed change) are in
[`docs/upstream-proposals.md`](../docs/upstream-proposals.md) § E.

## What this means for sparq (code-read, NOT executed here)

No Rust toolchain was available in the environment that produced this record (`rustup` could not
write to a read-only filesystem), so everything in this section is **read from sparq's source and
must be confirmed by running the differential lane** — captured as follow-up work, not asserted.

1. **sparq's default Turtle ingest is `oxttl`**, so it inherits F1 and F2 as written.
2. **sparq's opt-in native Turtle parser (`crates/sparq-core/src/ttl.rs`) does not share them.**
   `reifier_id_or_fresh()` has an explicit `[`-then-`]` arm, and triple-term / reified-triple
   objects are parsed by the *same* `object()` function as ordinary objects, so long-quoted literals
   are accepted there. That means the native path and `oxttl` **disagree on exactly these two
   constructs** — and the `sq-tonhr.2` differential lane pins its known native-vs-oxttl divergences
   as an exact adjudicated set, which these two are not on. Whether that is because the lane's
   inputs (the W3C suites + `fuzz/seeds/`) never reach these constructs is **not verified here** —
   the W3C suite is not fetched in this environment — and is filed as follow-up work.
3. **A suspected native-parser gap of its own:** that `[`-then-`]` arm requires `]` to *immediately*
   follow `[`, but `ANON ::= '[' WS* ']'`, so `~ [ ]` and `~ [` + newline + `]` look W3C-legal and
   look rejected by the native path. The generated corpus contains all three ANON spellings, and the
   Shuttle oracle accepts all three. Needs a Rust run to confirm.

## Limits (what would make this stronger, in order)

1. **This is a lower bound, not a count of "all divergences".** One grammar (`turtle12`), one
   candidate family, two seeds, 400 documents. `shaclc12ext` was not harvested (the bead names it
   "when available" — it is available as a grammar, but sparq's SHACL-CS consumer is itself the
   Shuttle artifact, so a differential needs a *second* independent SHACL-CS implementation to be
   meaningful).
2. **The oracle is trusted.** Every expected-quads value comes from the generated `gen-js` parser. A
   defect there is invisible to this harness and would be reported as a candidate divergence. The
   upstream fixed point for that is spec §9's trust anchor (replay the official W3C rdf12 suites
   through the reference interpreter before using a Shuttle spec as an oracle) — not yet in place.
3. **Negatives are oracle-rejected, not provably negative.** With the LL tables the mutants become
   provably outside the language; without them a mutant that the oracle wrongly rejects would be
   filed as a candidate-accepts divergence against a *correct* parser. Zero negative-corpus
   divergences were observed, so this did not bite here.
4. **The repro shrink is not minimal.** It is a chunk-descending token-level ddmin-lite: it cannot
   remove *balanced* structure (`<<`…`>>`, `{|`…`|}`), so a repro can retain nesting that plays no
   part in the divergence. The two findings above were minimised by hand from its output.
5. **`sparq`/`oxttl` were not run in-process.** The bead asks for divergence counts across
   oxttl/sparq/Oxigraph; Oxigraph (= `oxttl`) was measured, sparq was read. Running the corpus
   through the `sq-tonhr.2` Rust harness closes this.

## What is deliberately not done

* **No upstream code was written.** `--emit tests` belongs in `jeswr/rdf-shuttle`; this session had
  no mandate to push there, and the interim harness is explicitly not a substitute — it cannot make
  positives valid-by-construction or negatives provable, because those properties come from running
  the compiled relation and the LL tables, which only the upstream generator has.
* **No bug was filed.** Both findings are written up as ready-to-submit reports in
  `docs/upstream-proposals.md` § E, matching how § A/§ B stage oxigraph and rdf-tests submissions.
* **The harness is not wired into CI, and gates nothing.** It needs an upstream checkout and an npm
  package that no lane fetches today, and its value here is a measurement, not a ratchet. The
  ratchet belongs in the `sq-tonhr.2` Rust harness once the corpus feeds it; until then `--self-test`
  is a thing an author runs, not a check that runs itself.
* **No corpus was committed.** The generated corpus is reproducible from the seed and would be a
  large, churning binary-ish addition; the harness writes it to `--out` on demand. Committing a
  frozen slice as fuzz seeds is a decision for the lane-wiring work, not this record.
