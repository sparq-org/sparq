# RDF/JS conformance for `@sparq-org/sparq`: what is done, what "100%" still costs

**Epic:** sq-iwhl8 (issue #2938, from maintainer issue #1116) · **Status:** design record for
maintainer review — an honest re-derivation of the epic's remaining surface, NOT an implementation ·
**Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-07-28

Companion record: `research/rdfjs-conformance-upstream.md` (the *other* half of the epic — offering
`packages/rdfjs-conformance/` to rdfjs/). That record covers the contribution question and is not
re-litigated here.

## Why this record exists

The epic asks for two things: (a) TS bindings that extend the RDF/JS types and pass the RDF/JS
conformance suites at N3.js parity, and (b) a separately-contributable RDF/JS test harness. Both
have substantial landed work. What did *not* exist was a statement of what is still between that
work and the epic's headline claim, grounded in the tree rather than in the earlier PR descriptions.

Everything below was re-derived from this checkout on 2026-07-28. **Nothing here was executed** —
`npm` is not on PATH in this environment and no `node_modules` tree is installed, so every claim is a
*reading* of source, config and CI, and is cited by `file:line`. Where a claim would need a run to
confirm, it is marked as such.

## 1. What is landed and verified (by reading, not by running)

| Surface | Where | State |
|---|---|---|
| Term classes | `js/src/terms.ts:7,15,27,69,77,87` | `NamedNode`/`BlankNode`/`Literal`/`Variable`/`DefaultGraph`/`Quad` each carry an `implements RDF.*` clause — a real compiler-checked obligation, not a comment |
| `DataFactory` | `js/src/terms.ts:149` | typed `RDF.DataFactory`, including the optional `fromTerm`/`fromQuad` and `variable` |
| RDF 1.2 literal direction | `js/src/terms.ts:29-66` | `direction` + `rdf:dirLangString` handled in the constructor and in `equals` |
| `Dataset` + factory | `js/src/dataset.ts:59,422` | `implements RDF.Dataset`; the full algebra (`addAll`…`toCanonical`) is present, and `datasetFactory` is typed as both `DatasetCoreFactory` and `DatasetFactory` |
| `Source`/`Sink`/`Store` | `js/src/source.ts:142` | `implements RDF.Store<RDF.Quad>` — `match`/`import`/`remove`/`removeMatches`/`deleteGraph` |
| Query-spec surface | `js/src/store.ts:155`, `js/src/bindings.ts:14` | `implements RDF.StringSparqlQueryable<…>` and `implements RDF.Bindings` |
| Shared harness | `packages/rdfjs-conformance/src/index.mjs:128,345,645,719` | `runDataFactoryTests` / `runDatasetTests` / `runStreamTests` / `runAll`, zero runtime deps |
| sparq runs the harness | `js/test/rdfjs-conformance.test.mjs` | all three runners against `@sparq-org/sparq` |
| N3.js parity | `packages/rdfjs-conformance/test/n3-parity.test.mjs` | same harness against N3.js `DataFactory` + `Store`, and `@rdfjs/dataset` |
| CI | `.github/workflows/js.yml:128-132` | harness typecheck + parity suite; the root `npm test` step runs the sparq-side suite |

That is a genuinely strong position, and the "N3.js parity" clause of the epic is met in the sense
that matters most: **one suite, three implementations, no per-implementation forks.**

The harness registers **49 tests — 18 DataFactory, 28 Dataset, 3 Stream** (counted over
`src/index.mjs`; the split is by `describe` boundary at lines 134/345/645). Many are feature-detected
and skip on implementations that lack the member.

## 2. The gap between "green" and "100% RDF/JS conformance"

Five findings. Each is a *verified reading of the tree*, with the code reference that establishes it.

### G1 — the evidence is entirely self-derived; no upstream reference suite is run

The harness header states it mirrors the design of `@rdfjs/data-model/test`'s `runTests({ factory,
mocha })` "BUT with ZERO external dependencies" and derives assertions from the spec prose
(`packages/rdfjs-conformance/src/index.mjs:7-15`). Grepping the tree for any consumer of the actual
upstream suite returns only that comment — **no test anywhere drives sparq's `DataFactory` through
`@rdfjs/data-model`'s own `runTests`.**

This matters because a self-written suite cannot detect a *misreading* of the spec: if our reading of
`literal('x','EN')` or of `match()` independence is wrong, our suite is wrong in the same direction
and stays green. The upstream suite is the only independent oracle available, and it is already
resolvable — `@rdfjs/data-model@1.3.4` is in the root lockfile (`package-lock.json:3631`, dev). The
cost is a mocha devDependency for one lane, which the harness's own design deliberately avoids for
*consumers* but which sparq can pay privately.

Until that lane exists, "passes the RDF/JS conformance suites" should be stated as "passes a
spec-derived conformance suite shared with N3.js" — which is true, defensible, and less than the
epic title claims.

### G2 — the Stream spec is the thinnest suite over the least-conformant implementation

Three of 49 tests cover Stream/Source/Sink (`src/index.mjs:657,682,699`, the third being the
skip-when-absent case). Meanwhile the implementation side has three separate emitter classes:

| class | file | members | `readable` event |
|---|---|---|---|
| `QuadStream` | `js/src/source.ts:44` | `read`/`on`/`once`/`removeListener`/`off`/`emit` | not emitted |
| `ArrayQuadStream` | `js/src/dataset.ts:610` | same six | not emitted |
| `ResultStream` | `js/src/result-stream.ts:21` | the full emitter surface — `addListener`, `prependListener`, `prependOnceListener`, `removeAllListeners`, `listeners`, `rawListeners`, `listenerCount`, `eventNames`, `setMaxListeners`, `getMaxListeners` (`:61-141`) | not emitted |

Three consequences follow, and none of them can be caught by the current suite:

1. **The type is a promise the runtime does not keep.** Both quad streams reach `RDF.Stream<Quad>`
   through `as unknown as` (`js/src/source.ts:129-130`, `js/src/dataset.ts:374`) — the double cast is
   required *precisely because* the classes are not structurally assignable. The repo says as much:
   the `RDF.Stream` "nominal type extends the whole `node:events` `EventEmitter`"
   (`js/src/source.ts:120-127`). So a consumer who calls `addListener` or `removeAllListeners` — both
   typed as present — gets a `TypeError` that TypeScript cannot warn about. `ResultStream` already
   implements every one of those members, so the fix is a shared emitter base, not new design.
2. **The two quad streams disagree with each other about `read()`.** `QuadStream.read()` and its
   microtask `#flush()` pull from *the same iterator* (`js/src/source.ts:61-68` vs `:105-119`), so a
   synchronous `read()` before the flush removes that quad from the `data` events. `ArrayQuadStream`
   indexes `#i` while `#flush()` iterates the array independently (`js/src/dataset.ts:621-623` vs
   `:657-661`), so the same quad is delivered *twice*. Two implementations of one interface, in one
   package, with opposite behaviour. The spec does not define mode-mixing — which is exactly why a
   conformance suite should pin whichever answer we choose.
3. **`readable` is never emitted** by any of the three, though the stream spec lists it.

This is the clearest instance of the general shape of the remaining work: **the suite is thinnest
exactly where the implementation is weakest**, so green tells us least there.

### G3 — the RDF/JS Query spec has an implementation and no suite at all

RDF/JS is four specs. The harness covers data-model, dataset and stream. sparq implements the fourth
(`RDF.StringSparqlQueryable` at `js/src/store.ts:155`, `RDF.Bindings` at `js/src/bindings.ts:14`) and
**no conformance runner exercises it** — there is no `runQueryTests` in the harness's exports
(`src/index.mjs:732`). `js/test/` has query-shaped tests, but they are sparq-specific, not a
portable suite another implementation could run.

For an epic whose headline is "100% RDF/JS conformance", an un-suited quarter of the spec family is
the single largest structural hole. It is also the most valuable thing to contribute upstream: the
Query spec is the newest and has the least shared tooling, so a `runQueryTests` is more differentiated
than a fourth restatement of the data-model tests.

### G4 — `@rdfjs/types` is a devDependency, and the published `.d.ts` will import it

`@rdfjs/types` appears only in `devDependencies` (`js/package.json`), but nine source files import it
(`import type * as RDF from '@rdfjs/types'`) and reference `RDF.*` in *exported* signatures — e.g.
`add(quad: RDF.Quad): this` on the exported `Dataset`. With `declaration: true`
(`js/tsconfig.json`), `tsc` must therefore re-emit that import into `dist/*.d.ts`, so an installed
`@sparq-org/sparq` carries a type reference to a package npm never installed for the consumer.

**This is the epic's own headline claim failing at the package boundary**: "TS bindings that extend
the RDF/JS types" only holds for a consumer who happens to depend on `@rdfjs/types` themselves. How
loudly it fails for everyone else was *not* established here: an unresolved import inside a `.d.ts`
is a module-resolution diagnostic, which is not the same thing as the declaration-file *type*
checking `skipLibCheck` disables, so it may well still be reported. Whether such a consumer gets a
hard error or silently degraded types needs a fresh-consumer fixture (§ 7) — but under either
outcome the claim above does not hold.

It is a decision, not a typo, because sparq deliberately keeps the runtime dependency tree at
`{fzstd}` so the published-client CycloneDX SBOM stays clean (`.github/workflows/js.yml:100-110`
records that policy). Options:

| option | effect on the SBOM | effect on consumers |
|---|---|---|
| `dependencies` | adds a types-only package to the published runtime tree (it ships no executable code, but it *is* an SBOM component) | always resolves; matches what most RDF/JS libraries do |
| `peerDependencies`, **not** optional | stays out of the published runtime tree | npm 7+ auto-installs it, so it resolves — at the cost of a hard unmet-peer failure on version conflict, and pnpm does not auto-install peers by default |
| `peerDependencies` + `peerDependenciesMeta.optional` | stays out of the runtime tree | **not** auto-installed — npm skips optional peers entirely — so a consumer who installs only `@sparq-org/sparq` is exactly where they started |
| vendor the used declarations into `dist/` | clean | always resolves, but sparq's types then *restate* rather than extend RDF/JS's, and must be kept in sync |
| leave as-is | clean | the claim above is not true for a fresh consumer |

There is no option that keeps the runtime tree at `{fzstd}` *and* satisfies definition-of-done (iii)
below without a caveat, so this is a genuine tradeoff for the maintainer rather than a recommendation
this record can make for them:

- to hold (iii) as written — types resolve for a consumer who installs only `@sparq-org/sparq` — the
  choices are `dependencies` or vendoring;
- to keep the SBOM at `{fzstd}`, the choice is a non-optional peer (resolves under npm 7+ only) or an
  optional peer with (iii) *weakened* to "resolves for a consumer who also installs the declared
  optional peer". An optional peer must not be sold as an auto-installed fix; it is not one.

Whichever is chosen, `js/guardrails/check-package.mjs` should grow a check that every module named by
an emitted `.d.ts` import is declared somewhere in the manifest, so this class of drift cannot recur
silently.

### G5 — nothing ratchets

`research/codebase-improvement-opportunities-2026-06-23.md:473-475` already parked "RDF/JS
conformance ratchet wiring" as "a natural tail of this bead". It is still untied: the suites are
pass/fail, so a *skipped* test (the feature-detected ones skip silently) is indistinguishable from a
passing one in CI output. An implementation could lose `fromTerm` and the suite would go quieter, not
redder. A recorded floor — "sparq must run at least N of the M registrations un-skipped" — turns
silent regression into a failure, and costs one JSON file plus an assertion.

## 3. Recommendation

Do **not** chase the literal words "100%". As written the epic is unfalsifiable: RDF/JS publishes
specs and per-package tests, not a numbered conformance suite with a percentage. Replace the headline
with a definition of done that can actually be checked, and then close the epic against it:

> `@sparq-org/sparq` is RDF/JS-conformant when, for each of the four RDF/JS specs, (i) every interface
> member sparq claims via an `implements` clause is exercised un-skipped by the shared harness,
> (ii) the harness's verdict is corroborated by at least one independent upstream suite, (iii) the
> published package's types resolve for a consumer who installs only `@sparq-org/sparq`, and (iv) a
> recorded floor makes a lost capability a red gate rather than a quieter log.

Against that definition the standing is roughly: data-model strong (needs (ii)), dataset strong
(needs (ii)), stream partial (fails (i)), query absent (fails (i)), packaging fails (iii), and (iv)
is unwired throughout. That is a *good* position, honestly stated — and materially different from
"100%".

## 4. Phased plan (each phase = one future bead, ordered)

Ordered so each phase is independently mergeable and de-risks the next. Sizes are relative effort,
not estimates in time.

1. **Packaging truth (S).** Declare `@rdfjs/types` in the manifest — as `dependencies` if (iii) is
   kept as written, or as a peer with (iii) weakened accordingly (the § G4 tradeoff is the
   maintainer's call and blocks this phase) — and extend `js/guardrails/check-package.mjs` with an
   "every `.d.ts` import is a declared dependency" assertion. Acceptance: the guardrail fails on a
   deliberately-undeclared type import, **and** a fresh-consumer fixture that installs only the
   packed tarball typechecks (with and without `skipLibCheck`), which is also what settles the § G4
   question of how the current state fails.
2. **One emitter, conformant (M).** Extract the full emitter surface `ResultStream` already
   implements into a shared internal base; make `QuadStream` and `ArrayQuadStream` use it; delete
   both `as unknown as RDF.Stream` casts so the assignability is compiler-checked. Settle and
   document the `read()`-vs-`data` interaction (§ G2.2) — one behaviour, both classes. Acceptance:
   `implements RDF.Stream<RDF.Quad>` compiles with no cast.
3. **Thicken `runStreamTests` (M).** Grow the 3-test Stream suite to cover the members phase 2 makes
   real: `read()` semantics, the `error` path, `Source.match` term filters, `Store.remove` /
   `removeMatches` / `deleteGraph`, and the emitter members a consumer may legitimately call. This is
   harness work, so N3.js gets checked by it too — the parity claim strengthens for free. Depends on
   phase 2 (write the tests against the settled behaviour, not the current one).
4. **Independent oracle (M).** Add a sparq-only CI lane that runs `@rdfjs/data-model`'s upstream
   `runTests({ factory, mocha })` against `js/src/terms.ts`'s `DataFactory`. Mocha stays a private
   devDependency of that lane; the contributable harness keeps its zero-dependency property.
   Acceptance: the lane is red if a term-equality rule is changed. Independent of phases 1-3.
5. **`runQueryTests` (L).** A fourth runner in `packages/rdfjs-conformance` over the Query spec —
   `Bindings` (Map-like semantics, `equals`, immutability), and the `*Queryable` shape. Feature-
   detected like the others so a non-query implementation skips cleanly. Wire it into both
   `js/test/rdfjs-conformance.test.mjs` and the harness's own parity suite. This is the phase most
   worth doing *before* the upstream proposal in `rdfjs-conformance-upstream.md` is filed — it is
   what makes the offer distinctive.
6. **Ratchet (S).** Record the per-implementation un-skipped registration count as a tracked floor
   and assert it in CI (§ G5). Do this last, so the floor is set against the finished surface rather
   than being re-baselined five times.

Phases 1 and 4 are independent of everything else and can run in parallel with 2→3.

## 5. Correction to the epic's premise

The epic reads as if the harness and the bindings are the work and conformance is the outcome. The
tree shows the harness and bindings largely landed, and the *outcome* under-evidenced: the suite is
self-derived (§ G1), thinnest where the implementation is weakest (§ G2), silent on a whole spec
(§ G3), and the type claim does not survive `npm install` (§ G4). The remaining work is therefore
mostly **evidence and packaging**, not new RDF/JS surface — which is a cheaper epic than its title
suggests, but not a finished one.

## 6. Open questions for the maintainer

1. **`dependencies`, a peer, or vendoring for `@rdfjs/types`?** § G4 shows no option protects the
   `{fzstd}`-only runtime SBOM *and* satisfies definition-of-done (iii) unconditionally — so this is a
   choice between the SBOM property and (iii) as written. It is your policy call, and it is the one
   item blocking phase 1.
2. **Is a private mocha lane acceptable** (§ G1 / phase 4) to get an independent oracle, given the
   harness's zero-dependency promise is about *consumers* and would be untouched?
3. **`read()` vs `data` (§ G2.2)** — deliver every quad to both (ArrayQuadStream's behaviour), or
   treat them as exclusive modes and make the second one throw? The spec is silent; whichever we pick
   becomes a conformance assertion other implementations would inherit if the harness is accepted
   upstream.
4. **Does `runQueryTests` (phase 5) belong in the same package**, or does bundling the least-settled
   spec with three settled ones weaken the upstream proposal?

## 7. What could not be verified here

- **No suite was executed.** `npm` is absent and `node_modules` is not installed, so every "green"
  statement about the existing suites rests on `.github/workflows/js.yml:128-132` and the prior PRs,
  not on a run in this environment. The same limitation is recorded in
  `research/rdfjs-conformance-upstream.md`.
- **`dist/*.d.ts` was not inspected** — the emitted-import claim in § G4 is derived from the source
  imports plus `declaration: true`, which is a sound inference but not a first-hand look at a built
  artifact. Confirm with one `tsc` run before acting on phase 1.
- **No fresh-consumer install was performed**, so § G4 states *that* the type reference is undeclared,
  not what a consumer observes: whether an unresolved `.d.ts` import surfaces as an error or as a
  silent degradation to `any`, and whether `skipLibCheck` changes that, is unverified. The fixture in
  phase 1 is what would settle it.
- **No network.** The exact contents of the current upstream `@rdfjs/data-model` test export, and the
  present wording of the stream and query specs, were not re-read; § G1 and § G3 rely on the
  in-repo lockfile entry and on the harness's own description of the prior art. Re-read both before
  writing phase 4 or 5.
