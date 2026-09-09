# Upstream rdfjs/: offer the RDF/JS conformance harness (`@rdfjs-test/conformance`)

**Bead:** sq-hiza4 (from sq-iwhl8 / issue #1116) · **Status:** proposal drafted, **NOT yet filed**
upstream — awaiting @jeswr review per the upstream-contribution protocol (`AGENTS.md` §
*Upstream contributions — how to open the PR*). Three questions the issue asks to *settle* (venue,
package name, publish-to-npm) are **maintainer decisions**; this record gives a recommendation and
the evidence behind it, not a decision · **Author:** SPARQ agent 🤖 [OPUS-5] · **Date:** 2026-07-27

Artifact under discussion: `packages/rdfjs-conformance/` (npm name `@rdfjs-test/conformance`,
`"private": true`, v0.1.0).

## Why this record exists

Issue #2937 asks for the *proposal*, not more harness code: open a discussion/issue on the rdfjs
org, settle the (provisional) package name, and decide whether it gets published to npm. Under the
upstream protocol nothing is filed on a third-party repo before @jeswr has reviewed it, so the
deliverable here is the **ready-to-post draft plus the decision material** — the filing itself is a
separate, explicitly-authorised step.

## What is actually being offered (verified against the tree, 2026-07-27)

Everything in this section was re-derived from `packages/rdfjs-conformance/` in this checkout, not
copied from the package README.

| property | value | how verified |
|---|---|---|
| runtime dependencies | **none** — `dependencies` and `peerDependencies` are both absent | `package.json` |
| runtime imports | Node built-ins only: a static `import { strict as assert } from 'node:assert'`, plus a lazy `await import('node:test')` inside `resolveTestApi` when no test API is injected | `src/index.mjs:23`, `:48` |
| public surface | `runDataFactoryTests`, `runDatasetTests`, `runStreamTests`, `runAll` (+ a default-export bag) | `src/index.mjs:128/345/645/719/732` |
| size | 733 lines of plain ESM, 7 `describe` blocks, **49 `test(...)` registrations** (18 DataFactory / 28 Dataset / 3 Stream) | counted over `src/index.mjs` |
| types | hand-written `src/index.d.ts` against `@rdfjs/types`, no build step | `src/index.d.ts` |
| test-framework coupling | none required: every runner takes an optional `{ describe, test }` override and otherwise defaults to `node:test` | `resolveTestApi`, `src/index.mjs:44-51` |
| licence | MIT (package) under the repo's MIT LICENSE, © 2026 Jesse Wright | `package.json`, `/LICENSE` |
| devDeps (test-only) | `@rdfjs/dataset`, `@rdfjs/types`, `@types/node`, `n3`, `typescript` | `package.json` |

Note the `test(...)` count is *registrations*, not always-executed assertions: the Dataset-algebra
and `variable`/Stream parts are feature-detected and skip cleanly against an implementation that
does not provide them. That adaptivity is the point — it is what lets one suite run against both a
full `Dataset` and a `DatasetCore`-only implementation.

Three independent implementations are wired as consumers in-repo:

- `packages/rdfjs-conformance/test/n3-parity.test.mjs` — N3.js `DataFactory` + `Store`, and
  `@rdfjs/dataset` (`DatasetCore`-only, algebra skipped).
- `js/test/rdfjs-conformance.test.mjs` — `@sparq-org/sparq`, exercising all three runners including
  Stream/Source/Sink.
- CI runs both: `.github/workflows/js.yml:119-132` (`npm test` at the root, then `npm run typecheck
  && npm test` in `packages/rdfjs-conformance`).

**Honesty note on "proven green".** Those suites are green *in CI*; they could **not** be re-run in
the environment this record was written in — `npm` is unavailable and `node_modules` is not
installed, so `node --test` fails at `ERR_MODULE_NOT_FOUND: n3` before reaching any assertion. The
green claim rests on the CI job above and on #1116, not on a fresh local run. Re-run locally before
filing so the proposal's central claim is first-hand.

## The three decisions the issue asks to settle

### 1. Venue — **unresolved here; must be confirmed at filing time**

This environment has no network access (WebFetch/WebSearch are unavailable and `gh` is out of scope
for this task), so the current rdfjs org repo list, whether GitHub Discussions are enabled on it,
and which repo the community currently uses for cross-package proposals **could not be verified**.
Do not file against a venue asserted from memory. Before filing, check in this order and pick the
first that exists and is active:

1. **Discussions on the rdfjs org's spec/coordination repo** — a proposal that spans three specs
   (data-model, dataset, stream) and asks for a new package is a discussion, not a bug report.
2. An **issue on the spec repo whose surface dominates** (data-model) cross-referencing the others.
3. Failing both, an issue on the package repo that owns the closest prior art —
   `@rdfjs/data-model`'s mocha-driven `runTests({ factory, mocha })` harness, which is the thing
   this package re-imagines without the mocha dependency.

The README already cites that prior art; **re-read it before filing** rather than restating the
README's characterisation of it — that claim is inherited from #1116 and was not re-verified here.

### 2. Package name — recommend **`@rdfjs/conformance`**, maintainers decide

`@rdfjs-test/conformance` was chosen locally to avoid squatting a scope the project does not own.
It has one concrete drawback: it needs a **brand-new npm org (`rdfjs-test`)** that, as far as this
record can establish, nobody has created. That is a new administrative surface (owners, 2FA policy,
publish tokens) for a single package.

The `@rdfjs` scope, by contrast, is demonstrably live and broad — this repo's lockfile alone
resolves published `@rdfjs/data-model`, `dataset`, `types`, `fetch`, `fetch-lite`, `formats-common`,
`namespace`, `parser-jsonld`, `parser-n3`, `serializer-jsonld`, `serializer-ntriples`, `sink`,
`sink-map`, `to-ntriples`. A conformance suite is the same *kind* of shared infrastructure as
`@rdfjs/types`, so it belongs beside them.

| candidate | for | against |
|---|---|---|
| **`@rdfjs/conformance`** (recommended) | existing scope, no new org, reads as first-class shared infrastructure alongside `@rdfjs/types` | maintainers must want it in the core scope; occupies a short, general name |
| `@rdfjs/test-conformance` | existing scope; leaves `@rdfjs/conformance` free for a future umbrella | longer, and the distinction it preserves may never be needed |
| `@rdfjs-test/conformance` (status quo) | keeps test tooling off the core scope | requires standing up a new npm org for one package |

Whether `@rdfjs/conformance` is actually unclaimed on npm was **not** verified (no network); confirm
before proposing it as available. The recommendation is about *which scope is right*, and it is only
a recommendation — naming inside their own scope is the rdfjs maintainers' call.

### 3. Publish to npm — recommend **yes, once it has an upstream home**

The package is deliberately `"private": true` today and should stay that way while it lives here.
Publishing is what makes it useful: a conformance suite consumed by copy-paste is a fork per
consumer, and the value proposition ("every RDF/JS implementation checks itself against the same
spec-derived assertions") only holds if there is one shared version to depend on. Zero runtime
dependencies and Node's built-in test runner make it about as cheap a devDependency as exists.

Suggested sequence, so publishing is never the irreversible first step: transfer/accept → settle the
name → publish `0.1.0` **from the rdfjs repo** → sparq switches to the published package and drops
its local copy. Do not publish from this repo under a name in someone else's scope.

## Draft — post verbatim once @jeswr approves

Adjust the venue and the name to whatever § 1 and § 2 resolve to. Keeps the protocol's shape: agent
self-identification, Why before What, @jeswr as the review gate, explicitly not-yet-ready.

> **Title: Proposal: a dependency-free RDF/JS conformance test suite any implementation can run**
>
> 🤖 This proposal was opened by an autonomous agent (a SPARQ agent) operating on @jeswr's behalf.
> It is **not yet ready for maintainer review** — @jeswr reviews it first; please don't treat it as
> a request for a decision until he marks it ready.
>
> **Why.** Every RDF/JS implementation re-writes the same tests. Does `literal('x', 'EN')` lowercase
> the language tag? Does a plain literal get `xsd:string`? Does `dataset.match()` return a *new,
> independent* dataset? Does `add` of an equal quad stay a no-op? These are spec answers, but each
> implementation re-derives them by hand, so they drift — and a new implementation has no ready way
> to find out where it is wrong. `@rdfjs/data-model` already ships a reusable `runTests({ factory,
> mocha })` for the data-model half; it requires mocha, and it stops at the data model.
>
> **What.** Building an RDF/JS surface for a new engine, we wrote a reusable conformance harness and
> would like to offer it to rdfjs/ rather than keep it. It covers **DataFactory + the Term/`equals`
> hierarchy**, **DatasetCore + the Dataset algebra**, and the optional **Stream/Source/Sink**
> surface, as three runners plus a `runAll`:
>
> ```js
> import { DataFactory, Store } from 'n3';
> import { runDataFactoryTests, runDatasetTests } from '<package>';
>
> await runDataFactoryTests({ factory: DataFactory, label: 'n3' });
> await runDatasetTests({
>   factory: DataFactory,
>   datasetFactory: (quads) => new Store(quads ? [...quads] : undefined),
>   label: 'n3 (Store)',
> });
> ```
> ```bash
> node --test "test/*.test.mjs"
> ```
>
> **How it differs from the existing `runTests`:**
>
> - **No test-framework dependency.** It defaults to Node's built-in `node:test`, so a consumer runs
>   `node --test` and installs nothing. Each runner also accepts `{ describe, test }`, so mocha,
>   vitest or anything with that shape can drive it instead.
> - **Feature-detected, so partial implementations still get value.** Dataset-algebra methods,
>   `variable`, and the Stream surface are probed and skipped when absent — a `DatasetCore`-only
>   implementation passes the core suite rather than erroring out.
> - **Assertions derive from the specs**, not from a reference implementation. Quads are compared
>   with an implementation-agnostic key over `.value`/`.termType`/`.language`/`.datatype.value`, so
>   a quad built by one library is a first-class member of another library's dataset.
> - **Types with no build step** — hand-written declarations against `@rdfjs/types` alongside plain
>   ESM.
> - **Zero runtime dependencies**; MIT; Node >= 18.
>
> It currently runs green against **N3.js** (DataFactory + Store), **`@rdfjs/dataset`**
> (`DatasetCore`-only, algebra skipped), and **`@sparq-org/sparq`** (all three surfaces).
>
> **What we're asking:**
>
> 1. Does rdfjs/ want this at all? If the answer is "fold it into `@rdfjs/data-model`'s existing
>    harness instead", that is a fine outcome and we'll do the work that way.
> 2. If yes — **what should it be called?** The current `@rdfjs-test/conformance` is a placeholder
>    picked only to avoid assuming a scope we don't own; `@rdfjs/conformance` seems more natural if
>    you're happy to have it in the core scope.
> 3. **Publish to npm?** It is unpublished today. It is only useful as a shared dependency, so we'd
>    suggest publishing once it has a home here — but that's your call.
>
> Happy to transfer the code as a PR to a new repo, or to open it wherever you'd prefer. We'll also
> keep maintaining it either way.

## If it is accepted — what changes here

None of this is done yet; it is the follow-on work, to be cut as beads when a decision arrives.

- `package.json`: `private: false`, the settled name, and `repository` repointed off
  `sparq-org/sparq`.
- Attribution/licence: MIT is already compatible, but carry the copyright line explicitly into the
  transferred package rather than relying on this repo's root `LICENSE`.
- README: strip the sparq-specific framing (the "candidate contribution" banner, the `@sparq-org/sparq`
  quickstart as the *second* example) and lead with a neutral implementation.
- CI: the harness's own parity suite (`test/n3-parity.test.mjs`) moves with it; sparq keeps only
  `js/test/rdfjs-conformance.test.mjs`, retargeted at the published package.
- `.github/workflows/js.yml:128-132` loses its `working-directory: packages/rdfjs-conformance` step
  once the package no longer lives here.

If it is **declined**, the package stays here, stays `private`, and keeps earning its place as the
thing that stops sparq's RDF/JS surface from drifting — nothing is lost.

## Open questions for the maintainers (raise in the thread, do not pre-decide)

1. Fold into `@rdfjs/data-model`'s `runTests` rather than a new package? That would mean a mocha
   dependency or a compatible dual-mode harness — a real design cost, and worth their opinion before
   anyone writes code.
2. Should the suites track a spec *version* explicitly (RDF 1.1 vs 1.2 term types), or stay
   version-agnostic and feature-detect?
3. Is `node:test` acceptable as the default, given some rdfjs packages are mocha-based?
