# Upstream proposals — resolution status (re-checked 2026-07-27)

**Section A (oxigraph PRs): RESOLVED, nothing filed — all six fixes already exist on
oxigraph main.** The Chumsky/Logos parser rewrite (`dabda10`, 2026-05-02) subsumes
fixes 1–4 and 6; `c29be03` (2026-05-21) fixes 5. Verified against upstream main
`de4dc5f` (2026-06-09) with a 13-probe harness (each bug case + legal-counterpart
guards). These fixes are **still unreleased as of 2026-07-27**: crates.io tops out
at spargebra 0.4.6, oxigraph's released `v0.5.9` tag still ships `lib/spargebra` at
version 0.4.6, and main was bumped to `0.5.0-dev` on 2026-07-19 (`a3d8311e`). The
published crate therefore remains buggy and sparq's vendored copy stays until a
release above 0.4.6 ships.

Re-check with `python3 scripts/check-spargebra-release.py`. Retirement (bead
`sq-98w7z.8`) is **not** a drop-and-bump: 13 manifests depend on the vendored tree,
the next release is a semver-major `0.5.0` carrying a newer `oxrdf`, and four of the
ten vendored patches are sparq-local with no upstream home. The full scope and the
dated check log live in `vendor/spargebra/SPARQ-PATCHES.md` § *Upstream release
watch*.

**Section B (rdf-tests issues): not yet filed, awaiting go-ahead (tracked in beads).**
Tracker search 2026-06-11: Issues 3+4 fall under the already-open w3c/rdf-tests#58
"How to format decimals?" — file them as an evidence comment there (approved-vs-
unapproved contradictory pairs), not new issues. Issues 1+2 are unreported
(closed #81 was an author-retracted misreading, different specifics) — file as new issues.

The final-eleven conformance work surfaced six parser bugs in spargebra 0.4.6
(fixed in our vendored copy, `vendor/spargebra/SPARQ-PATCHES.md`) and four
defective expected-results files in w3c/rdf-tests (reported as documented
divergences by `sparq-conformance`). This file holds ready-to-submit PR
descriptions for oxigraph/oxigraph and issue drafts for w3c/rdf-tests; § D
tracks the maintainer-namespace (`jeswr/*`) SHACL-CS submissions, which are
already filed and awaiting maintainer review; **§ E (added 2026-07-28, bead
sq-tonhr.12) holds two unfiled `oxttl` RDF 1.2 Turtle parser issues** found by
the Shuttle generate-mode harvest — unlike § A they are NOT fixed on oxigraph
main, and they affect sparq's default Turtle path. Every item in § A and § B was
verified against w3c/rdf-tests @ `f25dbc092c654d792974848e81bb519d7328f0e8`;
sparq's full run is 1225 pass + 4 documented divergences / 0 fail / 0 skip over
the 1229-test scope.

---

## A. oxigraph/oxigraph PRs (spargebra, `lib/spargebra/src/parser.rs`)

Each section is a self-contained PR description; the diffs are the
corresponding commits on sparq's `final-eleven` branch (vendor/spargebra), which
apply to upstream `lib/spargebra/src/parser.rs` with only path changes.

### PR 1 — SPARQL parser: reject nested aggregate functions

**Title**: `Reject nested aggregate functions (W3C sparql12/syntax nested-aggregate-functions)`

**Body**:

> `Query::parse` currently accepts `SELECT (COUNT(COUNT(*)) AS ?c) WHERE {}`.
> Aggregate arguments cannot contain aggregates: the SPARQL 1.2 translation
> (§18.3.4.1) replaces each aggregate found in SELECT/HAVING/ORDER BY with a
> fresh `agg_i` and no derivation admits one inside another's argument; the
> W3C negative syntax test `sparql12/syntax#nested-aggregate-functions` states
> it directly ("The expression argument of an aggregate function can not
> contain an aggregate function").
>
> `ParserState::new_aggregation` already replaces every parsed aggregate with
> a synthetic `Variable` named by `format!("{:x}", random::<u128>())`, so a
> nested aggregate surfaces as one of those synthetic variables inside the
> argument expression of the aggregate being registered. This PR walks the
> argument (`mentions_aggregate_variable`, a sibling of `are_variables_bound`)
> and errors with "Aggregate functions cannot be nested" when one is found.
> Synthetic names cannot collide with user variables; sub-SELECTs push their
> own scope on `ParserState::aggregates`, so aggregates inside
> `EXISTS { { SELECT ... } }` stay legal (the walker does not descend into
> `Expression::Exists`).
>
> Tests: `sparql12/syntax#nested-aggregate-functions` flips to pass; the
> sparql11/aggregates and sparql12/grouping suites are unchanged.

### PR 2 — SPARQL parser: `ExprTripleTermSubject` is `iri | Var` only

**Title**: `Restrict expression triple term subjects to iri | Var (SPARQL 1.2 [138])`

**Body**:

> The SPARQL 1.2 grammar production `[138] ExprTripleTermSubject ::= iri | Var`
> excludes literals and nested triple terms from the subject slot of an
> expression triple term (an RDF 1.2 triple term subject is an IRI or blank
> node). spargebra's `ExprTripleTermSubject` rule delegates to
> `ExprTripleTermObject` ([139], which also derives `RDFLiteral |
> NumericLiteral | BooleanLiteral | ExprTripleTerm`), so it accepts
> `BIND( <<( "literal" :q :z )>> AS ?X )` and
> `BIND( <<( <<(:s :p :o )>> :q :z )>> AS ?X )`. The data
> (`TripleTermData`) and pattern paths already enforce the restriction — only
> the expression path is loose.
>
> This PR makes `ExprTripleTermSubject` derive exactly `iri | Var`.
>
> Tests: W3C `sparql12/syntax-triple-terms-negative#tripleterm-subject-03` and
> `#tripleterm-subject-06` flip to pass; all 113 positive triple-term syntax
> documents still parse.

### PR 3 — SPARQL parser: longest-match tokenization for `<` vs IRIREF (syn-bad-26)

**Title**: `Do not parse '<' as less-than when it starts a valid IRIREF token`

**Body**:

> The SPARQL grammar tokenization note (1.1 §19.8 / 1.2 §19.7, note 3) says
> "When tokenizing the input and choosing grammar rules, the longest match is
> chosen." In `FILTER (?x<?a&&?b>?y)` (W3C negative syntax test
> `sparql10/syntax-sparql3#syn-bad-26`, whose comment is `"longest token rule"
> means this isn't a "<" and "&&"`), the characters starting at `<` form a
> valid IRIREF token (`<?a&&?b>`), so the input tokenizes as `?x` IRIREF `?y`
> — a syntax error. spargebra's scannerless grammar reads `<` as the less-than
> operator and accepts the document.
>
> This PR adds a negative lookahead `!IRIREF_TOKEN()` to the `"<=" / "<"`
> alternative of `RelationalExpression_inner`, where `IRIREF_TOKEN` is the
> IRIREF terminal exactly as tokenized ([172]: `'<'`, characters excluding
> ``<>"{}|^`\`` and #x00–#x20, `'>'`). The guard covers `<=` as well
> (`<=?a>` is likewise the longer IRIREF token). Ordinary comparisons are
> unaffected: whitespace after `<` and a second `<` (as in `?x<<http://iri>`)
> both terminate the IRIREF scan.
>
> Tests: `syn-bad-26.rq` flips to rejected; zero changes across the
> 1.0/1.1/1.2 syntax suites (554 documents).

### PR 4 — SPARQL parser: allow earlier-SELECT-expression variable reuse under aggregation

**Title**: `Accept reuse of an earlier SELECT expression variable in aggregating queries (SPARQL 1.2 §11.4)`

**Body**:

> SPARQL 1.2 §11.4 (Aggregate Projection Restrictions) allows a variable
> occurrence in the projection of a grouping query when "the variable is
> introduced by an earlier SELECT expression in the same SELECT clause".
> spargebra validates SELECT expressions of aggregating queries against only
> the WHERE-visible variables, so the legal
>
> ```sparql
> SELECT (COUNT(?v) AS ?count) (?count + 1 AS ?countPlusOne) WHERE {
>   VALUES ?v { 0 1 2 3 }
> }
> ```
>
> (W3C `sparql12/grouping#select-variable-reuse`) is rejected with "The SELECT
> contains an expression with a variable that is unbound".
>
> This PR makes `build_select` insert each select-expression alias into the
> visible set right after emitting its `Extend`, so later expressions in the
> same SELECT clause see it. The "SELECT overrides an existing variable" check
> reads the same set, so *rebinding* an alias stays an error (§18.3.4.4: "var
> must not appear in VS nor in PV").
>
> Tests: `sparql12/grouping#select-variable-reuse` flips to pass (parse +
> evaluation); aggregates/grouping suites otherwise unchanged.

### PR 5 — SPARQL parser: match the boolean keywords case-insensitively

**Title**: `Parse TRUE/FALSE case-insensitively as xsd:boolean literals`

**Body**:

> "Keywords are matched in a case-insensitive manner with the exception of the
> keyword 'a'" (1.1 §19.8 / 1.2 §19.7, note 1). `true`/`false` are keyword
> terminals (`BooleanLiteral`, [173]) and §4.1.2 maps the token to a literal
> of datatype `xsd:boolean`, so `TRUE`/`False` denote the boolean literals
> with the canonical lexical forms `"true"`/`"false"` — see W3C test
> `sparql10/expr-builtin#case-insensitive-booleans` ("Boolean keywords are
> case insensitive, and produce valid boolean literals"), which expects
> `SELECT (TRUE as ?t) (False as ?f) {}` to yield `"true"`/`"false"`.
> spargebra matches only lowercase and errors.
>
> This PR switches `BooleanLiteral` to the grammar's case-insensitive keyword
> matcher `i()`, always emitting the lowercase lexical forms. Every
> alternation tries `iri()` before `BooleanLiteral`, so prefixed names with a
> `true`/`false` prefix (e.g. `true:x`) are unaffected.
>
> Tests: `case-insensitive-booleans` flips to pass; no other changes.

### PR 6 — SPARQL parser: keep `Join(Z, Filter)` for `OPTIONAL { { P FILTER } }` (preferred reading)

**Title**: `Follow the preferred reading for OPTIONAL over a doubly braced filter group`

**Body**:

> SPARQL 1.1 §18.2.2 / 1.2 §18.3.2 resolves the SPARQL 1.0 ambiguity for
> `OPTIONAL { { ... FILTER (...?x...) } }`: "Applying the simplification step
> after all the translation of graph patterns is the preferred reading." Under
> that reading, the OPTIONAL translation (§18.3.2.6) sees
> `Join(Z, Filter(F, A))` — not the `Filter(F, A)` form — so `F` must NOT be
> hoisted into the LeftJoin expression (where it would see the left side's
> bindings). spargebra's `new_join` simplifies `Join(Z, A) = A` eagerly during
> parsing, making the doubly braced group indistinguishable from a top-level
> filter, and hoists. The W3C manifest runs only the `-not-simplified` variant
> of the `dawg-optional-filter-005` pair, annotated "Preferred reading and
> SPARQL 1.1" (the `-simplified` variant is commented out).
>
> This PR makes `GroupGraphPatternSub` keep the spec's pre-simplification
> shape: when a group has no FILTER clause of its own but its body reduced to
> a bare `Filter` (i.e. the filter bubbled up from a nested group), it returns
> `Join(Z, Filter(F, A))`. Groups with their own top-level FILTER still
> translate to `Filter(F, A)` and keep hoisting per §18.3.2.6. The extra
> `Join` with the empty BGP is semantically a unit join, so only the algebra
> shape (and `to_sse`) changes for doubly braced filter groups.
>
> Tests: `sparql10/optional-filter#dawg-optional-filter-005-not-simplified`
> flips to pass; `optional`/`optional-filter` suites otherwise unchanged.
> Note: this changes evaluation results for the affected pattern, matching
> Jena/ARQ's behaviour and the suite's chosen reading.

---

## B. w3c/rdf-tests issue drafts

### Issue 1 — `sparql11/csv-tsv-res` tsv03: expected TSV changes the double's lexical form

**Title**: `csv-tsv-res: csvtsv03.tsv writes 1.0e6 for the data term "1.0E6"^^xsd:double`

**Body**:

> `tsv03 - TSV Result Format` runs the identity query `SELECT * WHERE { ?s ?p
> ?o }` (csvtsv01.rq) over `data2.ttl`, which contains
>
> ```turtle
> :s6 :p6 "1.0E6"^^xsd:double .
> ```
>
> The expected file `csvtsv03.tsv` serializes that binding as the Turtle
> shorthand `1.0e6`, which denotes the literal `"1.0e6"^^xsd:double` — a
> *different RDF term* (lexical forms are part of the term, RDF 1.2 Concepts
> §3.3). Graph pattern matching binds `?o` to the data's term, so a conforming
> engine must answer `"1.0E6"^^xsd:double` and can never match this file under
> term-equality. The fix is one character: write `1.0E6` (also a valid Turtle
> DOUBLE token). The sibling rows (and `csvtsv01.tsv`) already echo data terms
> verbatim.

### Issue 2 — `sparql11/cast` cast-decimal: expected file normalizes the echoed data terms

**Title**: `cast: cast-decimal.srx rewrites the data terms 0E1/1E0 as 0.0/1.0 in ?v`

**Body**:

> `xsd:decimal cast` projects the data-bound variable `?v` alongside
> `xsd:decimal(?v)`. `data.ttl` contains
>
> ```turtle
> :n07 :p 0E1 .
> :n08 :p 1E0 .
> ```
>
> i.e. the terms `"0E1"^^xsd:double` and `"1E0"^^xsd:double`, but
> `cast-decimal.srx` lists `?v` for those rows as `0.0` / `1.0`
> (`xsd:double`). `?v` is bound by graph pattern matching to the data's RDF
> term, whose lexical form must round-trip verbatim; only the *cast result*
> `?decimal` is a computed value. No engine that preserves data terms (as RDF
> Concepts requires) can match this file under term-equality. The expected
> `?v` entries for `:n07`/`:n08` should be `0E1` and `1E0` (the `?decimal`
> column is fine). The other `cast-*.srx` siblings echo `?v` verbatim — only
> the decimal file was normalized.

### Issue 3 — `sparql11/aggregates` agg-sum-distinct: lexical-form convention conflicts with its approved sibling

**Title**: `aggregates: agg-sum-distinct.srx expects "2100"^^xsd:double where approved agg-sum-02 expects canonical scientific`

**Body**:

> `SUM DISTINCT with GROUP BY` (`agg-sum-distinct`, no `dawgt:approval`)
> expects the `:doubles` group's sum as `"2100"^^xsd:double`, while the
> *approved* sibling `SUM with GROUP BY` (`agg-sum-02`) expects the same
> construct's result in canonical scientific notation
> (`"3.21E4"^^xsd:double`). SPARQL defines only the value of `SUM`; under the
> term-equality result comparison the two files demand contradictory
> serialization policies for computed doubles, so no engine can pass both
> while serializing consistently. Suggest aligning `agg-sum-distinct.srx` with
> the approved convention: `"2.1E3"^^xsd:double` (and the `:decimals` row
> `3.2`, `:ints` row `3`, `:mixed1` row `3.2` are consistent and fine).

### Issue 4 — `sparql10/expr-ops` divide-numbers-cast: decimal scale conflicts with the approved sparql11 suites

**Title**: `expr-ops: result-divide-numbers-cast.srx decimal scale contradicts sparql11 functions/aggregates`

**Body**:

> `/ operator on number mixed datatypes` expects the decimal-typed divisions
> (e.g. `"3"^^xsd:integer / "3"^^xsd:integer`) to yield `"1"^^xsd:decimal`
> (scale-0 lexical form). The approved SPARQL 1.1 suites expect scale-1 for
> the same `op:numeric-divide` on decimals: `sparql11/functions` `coalesce01`
> expects `0/2 = "0.0"^^xsd:decimal` and `sparql11/aggregates` `agg-avg-01`
> expects `"2.0"`. XPath/SPARQL define only the VALUE of the division;
> under term-equality comparison no single serialization policy satisfies both
> conventions. Suggest normalizing `result-divide-numbers-cast.srx`'s decimal
> results to the sparql11 convention (`"1.0"^^xsd:decimal`), or comparing
> numeric results by value.

---

## C. Status

| item | where fixed locally | upstream action |
|---|---|---|
| nested aggregates accepted | vendor/spargebra (patch 1) | already fixed upstream (`dabda10`, unreleased) |
| expr triple-term subject too loose | vendor/spargebra (patch 2) | already fixed upstream (`dabda10`, unreleased) |
| syn-bad-26 longest match | vendor/spargebra (patch 3) | already fixed upstream (`dabda10`, unreleased) |
| SELECT-var reuse rejected | vendor/spargebra (patch 4) | already fixed upstream (`dabda10`, unreleased) |
| TRUE/FALSE case-sensitive | vendor/spargebra (patch 5) | already fixed upstream (`c29be03`, unreleased) |
| OPTIONAL{{…FILTER}} hoisting | vendor/spargebra (patch 6) | main already implements the preferred reading |
| tsv03 expected file | divergence allowlist (runner) | Issue 1 (rdf-tests) — unreported, file as new issue |
| cast-decimal expected file | divergence allowlist (runner) | Issue 2 (rdf-tests) — unreported, file as new issue |
| agg-sum-distinct expected file | divergence allowlist (runner) | comment with evidence on open rdf-tests#58 |
| divide-numbers-cast expected file | divergence allowlist (runner) | comment with evidence on open rdf-tests#58 |

---

## D. Maintainer-namespace SHACL-CS repos (epic sq-tonhr, bead sq-tonhr.5)

**All three submissions are FILED and OPEN — do not re-author them.** Filed
2026-07-11; status re-verified 2026-07-27 via the GitHub API (each still
`state: open`, `mergeable_state: clean`, no merge commit). Nothing here is
blocked on sparq; each waits on the maintainer.

| upstream PR | deliverable | status (2026-07-27) |
|---|---|---|
| [jeswr/shaclcjs#199](https://github.com/jeswr/shaclcjs/pull/199) | strict-mode leak fix + isolated regression fixtures | open, awaiting review |
| [jeswr/shaclc-1.2#3](https://github.com/jeswr/shaclc-1.2/pull/3) | MIT `LICENSE` | open, awaiting review |
| [jeswr/shaclc-1.2#4](https://github.com/jeswr/shaclc-1.2/pull/4) | RDF 1.2 conformance pairs under `spec/tests/valid/` | open, awaiting review |

### The shaclcjs strict-mode leak (independently re-derived 2026-07-27)

`shaclcjs` gates four constructs behind `parse(str, { extendedSyntax: true })`,
but at `main` (`3aee328`) only two were enforced: `ensureExtended()` guarded the
`;` node-shape annotation (`turtleAnnotation`) and the `a` keyword. The
`% … %` property escape (`pcSection`) and the trailing-turtle section
(`ttlStatement`) had **no guard**, so a document using only one of those parsed
successfully with `extendedSyntax` at its strict default. Reproduced against a
freshly built parser: both constructs accepted in strict mode; the standard-only
control and the two guarded constructs behaved correctly.

The pre-existing suite could not catch this. `Conformance-test.js` does assert
`expect(() => parse(shaclc)).toThrowError()` for every `__tests__/extended/`
fixture — but every one of those fixtures opens with `shape ex:TestShape ; …`,
so the assertion is satisfied by the `;` guard alone and says nothing about the
other two constructs. **A vacuous-for-the-wrong-reason assertion, not a missing
one** — which is why the fix ships feature-isolated fixtures (no `;`, no `a`).

PR #199 adds `-> ensureExtended()` to the `pcSection` and `ttlStatement`
productions in `lib/shaclc.jison`, matching the guard style already used for
`turtleAnnotation`. An independent re-derivation this session reached the same
two productions and confirmed the fix behaves as claimed: all four constructs
rejected in strict mode, all still accepted under `extendedSyntax: true`, the
standard-syntax control unaffected, and the full upstream suite green. A
per-guard mutation check (remove one `ensureExtended()`, rebuild, re-run) put
the new fixtures red and the pre-existing suite green — confirming both that the
new tests are non-vacuous and that the old suite was blind to the leak.

### Why the LICENSE PR gates downstream work

`jeswr/shaclc-1.2` carries **no LICENSE** at `main` (`bfcde2d`; GitHub reports
`license: null`), so nothing may be vendored from it until #3 lands — the
constraint § 5 of
[`research/shacl-compact-and-shuttle-parsers-2026-07.md`](../research/shacl-compact-and-shuttle-parsers-2026-07.md)
records. `main` is still the one-commit README scaffold; the `spec/` layout its
README promises exists only on the open PR branches.

sparq is **not** blocked meanwhile: `crates/sparq-shaclc` carries its own
independently-authored corpus (`tests/fixtures/rdf12/`, plus the
two `leak-*-only.*` pairs that pin the strict/extended split from sparq's side), and
`tests/conformance.rs` already asserts that sparq's strict profile rejects every
extended fixture. Landing #3 unlocks *sharing* fixtures with upstream, not
sparq's own coverage.

---

## E. oxigraph/oxigraph issues (`oxttl`, RDF 1.2 Turtle parser) — epic sq-tonhr, bead sq-tonhr.12

**Not filed; ready to submit.** Both were found by the interim Shuttle generate-mode harvest
(`scripts/shuttle-generate-harvest.mjs`) and are recorded, with the method and its limits, in
[`research/shuttle-generate-mode-harvest-2026-07.md`](../research/shuttle-generate-mode-harvest-2026-07.md).
Each was reduced to a minimal repro, checked against the W3C grammar
(<https://www.w3.org/TR/rdf12-turtle/>, fetched 2026-07-28), and **confirmed in `oxttl`'s source**
rather than inferred from behaviour. Both reproduce on the newest release (`oxttl 0.2.3`; crates.io
has nothing above it) and are **still present on oxigraph `main`** (`lib/oxttl/src/terse.rs`, read
2026-07-28). sparq is affected on its default Turtle path, which is `oxttl` (`Cargo.lock`:
`oxigraph 0.5.9 → oxrdfio 0.2.5 → oxttl 0.2.3`).

### Issue 1 — Turtle 1.2: `~ []` (anonymous blank node as reifier) is rejected

**Title**: <code>oxttl: RDF 1.2 reifier does not accept ANON (~ [])</code>

**Body**:

> `reifier ::= '~' (iri | BlankNode)?` with `BlankNode ::= BLANK_NODE_LABEL | ANON`, so an
> anonymous blank node is a legal reifier. `oxttl` rejects it in both positions:
>
> ```turtle
> @prefix ex: <http://example.org/> .
> ex:s ex:p ex:o ~ [] .                    # "A dot is expected at the end of statements"
> << ex:s ex:p ex:o ~ [] >> ex:q ex:r .    # "Expecting '>>' to close a reified triple, found ["
> ```
>
> Controls that parse today: `~ _:r`, `~ ex:r`, and a bare `~`.
>
> `TriGState::Reifier` matches `N3Token::IriRef`, `N3Token::PrefixedName` and
> `N3Token::BlankNodeLabel`; there is no `Punctuation("[")` arm, so the `_` fallback treats `[` as
> "no reifier written", mints a fresh blank node and re-dispatches `[` into a state that cannot
> accept it — which is why the error surfaces at the following token rather than at the `[`.
>
> Fix shape: add a `Punctuation("[")` arm that emits the `rdf:reifies` quad with a fresh blank node
> and pushes the existing `TriGState::QuotedAnonEnd` (the same handling `ReifiedTripleSubject` /
> `ReifiedTripleObject` already use for `[`), so `'[' WS* ']'` is consumed as one ANON.

### Issue 2 — Turtle 1.2: long-quoted literals rejected in triple-term / reified-triple object position

**Title**: <code>oxttl: """…""" / '''…''' literals rejected inside &lt;&lt;( … )&gt;&gt; and &lt;&lt; … &gt;&gt;</code>

**Body**:

> `ttObject ::= iri | BlankNode | literal | tripleTerm` and
> `rtObject ::= iri | BlankNode | literal | tripleTerm | reifiedTriple`, and `String` includes
> `STRING_LITERAL_LONG_QUOTE` and `STRING_LITERAL_LONG_SINGLE_QUOTE` — so a long-quoted literal is
> legal in both positions. `oxttl` rejects it:
>
> ```turtle
> @prefix ex: <http://example.org/> .
> ex:s ex:p <<( ex:a ex:b """ab""" )>> .   # verbatim oxttl error, quoted so the issue is searchable — terminology-allow: third-party error string reproduced exactly
> << ex:s ex:p '''ab''' >> ex:q ex:r .     # same
> ```
>
> Controls that parse today: the same literals in ordinary object position, and `"ab"` / `'ab'`
> inside the same `<<( … )>>`.
>
> The lexer emits two distinct tokens, `N3Token::String` and `N3Token::LongString`. The ordinary
> object state matches `N3Token::String(value) | N3Token::LongString(value)`, but
> `TriGState::ReifiedTripleObject` — used for **both** `<< … >>` and `<<( … )>>` objects — matches
> `N3Token::String(value)` only.
>
> Fix shape: widen that arm to `N3Token::String(value) | N3Token::LongString(value)`, matching the
> ordinary object state.
