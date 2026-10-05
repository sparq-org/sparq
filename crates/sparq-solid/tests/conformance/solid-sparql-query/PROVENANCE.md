<!-- [OPUS-4.8] sq-gq28y (issue #1546) re-pinned sq-38lqs [SONNET-4.6] -->
# Vendored: solid-sparql-query query-semantics conformance suite

`manifest.json` + `fixture.nq` here are a **pinned copy** of the upstream conformance suite
for the *Access-Controlled SPARQL Query over a Solid Pod* Editor's Draft. `sparq` runs them
via `crates/sparq-solid/tests/conformance_solid_sparql_query.rs` and passes every case
(the standing "contribute spec tests and pass all of them" directive on `sparq-org/sparq#1546`).

| Field | Value |
|-------|-------|
| Upstream repo | <https://github.com/jeswr/solid-sparql-query> |
| Upstream path | `test-suite/query-semantics/` |
| Upstream commit | `5ea9718f13c89fb790a6bb3ada77bffeef926841` |
| Upstream PR | <https://github.com/jeswr/solid-sparql-query/pull/1> |
| Upstream merge date | 2026-07-05 |
| Conformance class | `query-semantics` |
| Cases | 15 (all MUST/MAY for the query-engine seam) |

## Pin note

The commit above is the **merge commit** of upstream PR #1 (merged 2026-07-05).
The file content at the merge commit is byte-for-byte identical to the contribution branch head
(`8d031bb7e1344aeaf9758d27ac0836cdbcf566fc`) — the merge was clean with no fixups applied.
Do NOT hand-edit `manifest.json`/`fixture.nq` in this directory — change them upstream and
re-vendor, so the suite stays a faithful mirror of the spec's own tests.

## Scope

Only the **query-semantics** conformance class is vendored/run here (empty default graph,
union-default-graph opt-in, non-disclosure invariants). The HTTP-**protocol** class (protocol
bindings, Service Description, Update refusal, caching) and the **content-mapping** class
(JSON-LD flatten / no-raw-preserve) are implemented by the Solid HTTP server and the RDF
parser respectively; they are enumerated in the manifest's `outOfClassScenarios`, not run by
this query-engine runner.

<!-- [OPUS-4.8] sq-tlzvw (issue #1654): vendored spec text + machine-readable companion -->
<!-- [FABLE-5] sq-bojxd: companion re-vendored from its canonical upstream home; provisional
     companion + local shapes superseded (sq-dhnn3 namespace question resolved) -->
## Spec text (`spec.html`)

`spec.html` is a **pinned, verbatim copy** of the upstream `index.html` (the ReSpec
Editor's Draft) at the upstream commit `5ea9718…` (unchanged through `306da22…`, the
current companion pin below) — byte-identical to the upstream `HEAD` at vendoring time.
It is present so the companion's quotes are fidelity-checkable against spec text **in
this tree**, offline. Do NOT hand-edit it; re-vendor from upstream on any spec change
and re-run the fidelity check below.

## Machine-readable companion (`spec.statements.ttl`)

`spec.statements.ttl` is a **pinned, byte-identical mirror** of the upstream
normative-statement companion. Its **canonical home is the upstream repo** (repo root,
beside the spec source `index.html`); like the suite above, change it upstream and
re-vendor — never hand-edit the copy here.

| Field | Value |
|-------|-------|
| Upstream commit | `306da228c6d1cfca08bebdf1bf8d21f14169b618` |
| Upstream PRs | authoring `d321f4f` (direct commit) + coverage links <https://github.com/jeswr/solid-sparql-query/pull/3> |
| Format | [`jeswr/spec-companion`](https://github.com/jeswr/spec-companion) (canonical vocab + SHACL shapes + validator) |
| Statements | 83 (one `spec:Requirement` per normative statement, verbatim quotes) |

Each statement carries its RFC 2119 level, actor binding, an E / A-int / A-exist / P
testability tag, a resolvable anchor into the spec, and either `spec:testCase` link(s) to
the query-semantics conformance vector(s) that exercise it (22 statements link
`test-suite/query-semantics/manifest.json#<case-id>` — the suite vendored as
`manifest.json` here) or an honest `sc:testGap`.

### History (supersession of the provisional companion)

sparq PR #1659 (bead `sq-tlzvw`, responding to #1654) authored a **provisional** companion
here (86 statements; provisional `sc: <https://w3id.org/spec-companion#>` namespace; a local
`spec-companion.shapes.ttl` SHACL guardrail) while the canonical `jeswr/spec-companion`
repository was not fetchable. In parallel, that canonical format landed and the upstream
spec repo received its own companion in it (`d321f4f`, 83 statements,
`sc: <https://w3id.org/jeswr/spec-companion#>`). Per this suite's own rule (**upstream is
canonical; change upstream and re-vendor**), bead `sq-bojxd` contributed the provisional
companion's unique value upstream — its conformance-vector coverage adjudication,
re-derived conservatively as the `spec:testCase` links of upstream PR #3 — and replaced the
provisional companion + local shapes file with this mirror. That also resolves the
provisional-namespace question (bead `sq-dhnn3`): the canonical namespace is adopted
wholesale. The 86-vs-83 statement-count difference is a consolidation difference over the
same normative text (the canonical file records keywordless / lowercase-keyword clauses as
`sc:extractionNote`s on the companion document instead of minting statements).

### Validation

Validate the mirror with the canonical tooling (a checkout of `jeswr/spec-companion`);
`spec.html` is byte-identical to upstream `index.html` at the pin, so the quote-fidelity
check holds against the in-tree copy:

```sh
node <spec-companion>/tools/validate.mjs spec.statements.ttl --spec-html spec.html
```

PASS = 83 statements, 0 errors, 0 warnings (verified in-tree at the pin above).
