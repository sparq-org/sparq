<!-- [FABLE-5] sparq-server HTTP wire + error contract doc-of-record (bead sq-fdurb; #1416, PSS ADR ask). -->
# sparq-server HTTP wire contract — v1

This is the **doc-of-record for the HTTP wire surface** of `sparq-server`: the endpoints,
parameters, request/response media types, content-negotiation rules, status codes and error-body
shape an **HTTP-only consumer** (one that talks to sparq exclusively over the network, e.g.
[solid-server-rs / PSS](https://github.com/jeswr/solid-server-rs)) may rely on. It is the HTTP
counterpart of [`docs/api-stability.md`](api-stability.md), which covers the **in-process
embedding** surface ([#1248](https://github.com/sparq-org/sparq/issues/1248)); this document covers
the ask in [#1416](https://github.com/sparq-org/sparq/issues/1416).

Everything in [The frozen v1 surface](#the-frozen-v1-surface) is enumerated **as-implemented**
and pinned end-to-end by `crates/sparq-server/tests/wire_contract.rs` (the served-surface
snapshot suite) together with `crates/sparq-server/tests/status_contract.rs` (the retry
contract) — an accidental wire change fails CI before it can reach a consumer.

## Status: v1 PROPOSED, not yet ratified

> **The wire freeze is NOT in force.** Declaring it in force is the maintainer's (@jeswr's)
> governance call, exactly as for the embedding-API freeze
> ([`docs/api-stability.md` § Status](api-stability.md#status-proposed-not-yet-in-force)).
> Until ratification the workspace is pre-`1.0` and a release MAY still change this surface —
> but any such change now fails the snapshot suite first, so it is always a *deliberate,
> release-noted* act, never an accident. This document tees up the ratification so the freeze
> is a one-line governance decision.

## Versioning & semver policy for the wire surface

The wire contract is versioned independently of crate versions (this document is **v1**).
The intended guarantee, once ratified: **the frozen surface is stable within a release
line** — a consumer pins a release line (e.g. `0.MINOR.*`), not a SHA.

**Breaking** (requires a new release line, a `CHANGELOG.md` release note naming the change,
and a wire-contract major bump — v1 → v2):

- Removing or renaming a frozen endpoint, method, or request parameter.
- Changing the status code emitted for a documented condition, or moving a status between
  the transient and permanent classes.
- Changing the error-envelope shape, a documented stable message sentinel, or an emitted
  `Content-Type` string.
- Changing a negotiation default (e.g. the no-`Accept` result format) or dropping a supported
  request/response media type.
- Requiring auth, or a new required parameter/header, on a previously-open frozen request form.

**Additive / non-breaking** (allowed in any release within the line; may extend this document
in-place as v1.x):

- New endpoints, new *optional* parameters, new negotiable media types, new response headers.
- New error conditions mapped onto the existing status classes, and new stable sentinels for
  *new* conditions.
- New opt-in cargo features / runtime flags (default-off), and anything in the
  [explicitly-unstable surface](#explicitly-unstable-surface).
- Changes to the free *wording* of an error message outside its documented sentinel substring
  (clients must classify on status + documented sentinels, never full-text — see below).

Operator-configurable limits (timeouts, body/row/byte caps, concurrency) are **knobs, not
contract**: their *default values* may change; the status code each cap emits when tripped may
not.

## The frozen v1 surface

The default build (`cargo build -p sparq-server`, features `server` + `jsonld`) serves all of
the following. Behaviour marked *(feature: `jsonld`)* is part of the default build but absent
from a `--no-default-features --features server` build.

### 1. SPARQL 1.1 Protocol — `/sparql`

**Methods:** `GET`, `HEAD`, `POST`, and the query-only HTTP `QUERY` method
(w3c/sparql-protocol#40, Oxigraph interop). Any other method → `405` with an `Allow` header.

**Query request forms** (SPARQL 1.1 Protocol §2.1):

| Form | How the query travels | Dataset params |
| --- | --- | --- |
| `GET /sparql?query=…` | URL-encoded `query` parameter | `default-graph-uri`, `named-graph-uri` (both repeatable) |
| `POST` with `Content-Type: application/x-www-form-urlencoded` | `query` form field | same, as form fields |
| `POST` with `Content-Type: application/sparql-query` | body **is** the query | as URL query-string params |
| `QUERY` with `Content-Type: application/sparql-query` | body **is** the query | as URL query-string params |

`GET /sparql` with no `query` parameter → `400` (on a build with the opt-in
`federation-descriptors` feature *and* its runtime flag enabled, it is a SPARQL Service
Description instead — that variant is [unstable](#explicitly-unstable-surface)).
`QUERY` is query-only by contract: an `application/sparql-update` body → `415`; an `update=`
form field → `400`.

**Update request forms** (SPARQL 1.1 Protocol §2.2):

| Form | How the update travels | Dataset params |
| --- | --- | --- |
| `POST` with `Content-Type: application/sparql-update` | body **is** the update | `using-default-graph-uri`, `using-named-graph-uri` (repeatable) |
| `POST` with `Content-Type: application/x-www-form-urlencoded` | `update` form field | same, as form fields |

A successful update → **`204 No Content`** (empty body), atomic: a failed update has **no
partial effect**. With `--persist`, the `204` is acked only after the write is WAL-durable.

A `POST /sparql` with any other `Content-Type` → `415`.

### 2. Query-result media types & negotiation

Negotiation is `Accept`-header driven, q-value aware (`q=0` rejects; an exact type beats a
wildcard at equal q). An **absent / empty / `*/*`** `Accept` selects the default and is never
an error. A **present, non-empty `Accept` naming no supported type and no wildcard** →
**`406 Not Acceptable`** (Oxigraph parity) for *both* result classes below.

**SELECT / ASK** (default: **SPARQL Results JSON**):

| `Accept` (also accepted aliases) | Emitted `Content-Type` |
| --- | --- |
| *(absent)* / `*/*` / `application/sparql-results+json` (`application/json`) | `application/sparql-results+json` |
| `application/sparql-results+xml` (`application/xml`, `text/xml`) | `application/sparql-results+xml` |
| `text/csv` | `text/csv; charset=utf-8` |
| `text/tab-separated-values` | `text/tab-separated-values; charset=utf-8` |

ASK has no CSV/TSV form; a CSV/TSV `Accept` on an ASK yields the JSON boolean form.

**Result envelopes** (SPARQL 1.1 Query Results JSON Format): a SELECT body is
`{"head":{"vars":[…]},"results":{"bindings":[…]}}`; an ASK body is `{"head":{},"boolean":…}`.

<!-- [SONNET-4.6] sq-7d3dj.26 -->
**Streamed SELECT-JSON and truncation safety.** A SELECT-JSON result larger than one 64 KiB
chunk is sent under **chunked transfer-encoding with no `Content-Length`**; smaller results stay
buffered with a `Content-Length`. Once the first byte is on the wire the status is committed, so
a later row/byte-cap or deadline trip can only **truncate** — and a truncated stream is always
distinguishable from a complete one: the closing `]}}` is **never** written, so the body fails
to parse as `sparql-results+json`. A **well-formed but short `200` is not a possible outcome.**
A client that sends **`TE: trailers`** additionally receives a `Trailer` header and, after the
body, `X-Sparq-Complete: true` or `X-Sparq-Truncated: deadline|max-rows|max-bytes|cancelled|panic|error`;
a client that does not gets the chunked stream aborted without its terminating zero-length chunk.

**CONSTRUCT / DESCRIBE** (default: **N-Triples**):

| `Accept` (aliases) | Emitted `Content-Type` |
| --- | --- |
| *(absent)* / `*/*` / `application/n-triples` | `application/n-triples; charset=utf-8` |
| `text/turtle` | `text/turtle; charset=utf-8` |
| `application/rdf+xml` (`application/xml`, `text/xml` at lower specificity) | `application/rdf+xml; charset=utf-8` |
| `application/ld+json` *(feature: `jsonld`, default-on)* | `application/ld+json; charset=utf-8` |

<!-- [FABLE-5] sq-0kq6k -->
**Body framing.** A CONSTRUCT / DESCRIBE response small enough to fit one 64 KiB chunk carries a
`Content-Length`; a larger one is streamed under **chunked transfer-encoding** with **no
`Content-Length`** (the same two shapes the streamed SELECT-JSON body uses). `HEAD` always
carries the `Content-Length` a `GET` would have had. The status is never committed early — the
result graph is fully evaluated before any byte is rendered, so a `413` / `503` refusal is
always clean and a graph body is never truncated mid-stream.

### 3. Graph Store HTTP Protocol

Two addressing forms, same semantics:

- **Indirect** — `/sparql/graph?default` or `/sparql/graph?graph=<IRI>` (exactly one; a
  selector-less `POST` mints a fresh server-named graph).
- **Direct** — `/graphs/{path}`: the graph IRI is reconstructed from the request
  (`http://<Host>/graphs/<path>`).

**Methods & statuses:** `GET`/`HEAD` read (negotiated as CONSTRUCT above; a `GET` of an
absent *named* graph serves the **empty graph** — `200`, empty body — it does not `404`);
`PUT` replaces (**`201`** if the graph did not exist, **`204`** if replaced — the
created/replaced distinction is advisory, see `status_contract`); `POST` merges additively
(`204`; `201` when it creates); `DELETE` drops
(`204`; `404` for an absent named graph; `?default` always `204` — CLEAR DEFAULT); `PATCH`
applies an **atomic graph-scoped SPARQL Update** (`Content-Type: application/sparql-update`,
always-on → `204`; the opt-in `text/n3` N3-Patch dialect is
[unstable](#explicitly-unstable-surface)).

**Accepted write body types:** `text/turtle` (also the default for an absent/empty
`Content-Type`), `application/x-turtle`, `application/n-triples`, `text/plain`,
`application/n-quads`, `application/trig`, `application/rdf+xml`, and
`application/ld+json` *(feature: `jsonld`)*. Anything else → `415`; a malformed RDF body →
`400`.

### 4. Liveness — `GET /health`

`200` with the plain-text body `ok`. Never auth-gated. (Deeper health/readiness semantics are
not part of v1.)

### 5. Authentication — Bearer token

- `--auth-token <TOKEN>` gates the **write** surface (SPARQL Update, GSP
  `PUT`/`POST`/`DELETE`/`PATCH`, admin routes). Classification is by *what the request does*
  (a mutating payload is a write regardless of route form).
- `--auth-token-read` additionally gates the **read** surface (queries, GSP reads, `/metrics`,
  subscription streams).
- Credential: `Authorization: Bearer <token>` (scheme case-insensitive).
- Failure → **`401`** with `WWW-Authenticate: Bearer`, `Cache-Control: no-store`, and a
  response that is **byte-identical for a missing vs a wrong token** (no oracle). Token
  comparison is constant-time.
- No token configured (default): all surfaces open, loopback-only bind by default.

### 6. Error contract

Every error response is the structured envelope **`{"error":"<message>"}`** with
`Content-Type: application/json` (the `405` additionally carries `Allow`). `<message>` is a
**stable generic class string** — sanitised, never echoing the caller's query/RDF/paths/tokens.
**Classify on the status code**, plus at most the documented sentinel substrings below; never
on full message text.

| Status | Condition | Class | Stable sentinel |
| --- | --- | --- | --- |
| `400` | malformed query / update / RDF body; missing `query=`; bad `generation` token | permanent | — |
| `401` | missing/wrong Bearer token on a gated surface | permanent | — (see §5 headers) |
| `403` | *(feature: `service`)* non-`SILENT` `SERVICE` to a host the default-deny egress allowlist refuses | permanent (policy refusal) | `egress allowlist` |
| `404` | unknown route; `DELETE` of an absent named graph (a `GET` serves the empty graph instead, §3); a compiled-but-flag-disabled opt-in route | permanent | `not found` (unknown route) |
| `405` | method not allowed on a route | permanent | — (carries `Allow`) |
| `406` | present-but-unsatisfiable `Accept` (both result classes, §2) | permanent | — |
| `410` | `?generation=N` aged out of the retention window (default build: the ring's concurrency-retention window; wider under the `time-travel` feature) | permanent | `aged out` |
| `413` | body over cap; result row/byte working-set cap; gzip ratio cap | permanent for the identical request (honest refusal, never truncation) | `row limit` / `byte limit` |
| `415` | unsupported request `Content-Type` (§1, §3) | permanent | — |
| `429` | concurrency cap: request shed **before it ran** | **transient** | — |
| `500` | caught handler panic / unclassified internal error | defect — surface, do not hot-retry | — |
| `503` | query/update timeout; durable-write refusal (write **not** applied); subscription capacity | **transient** | `timed out` (timeout case) |

**Transient = `429` and `503` only; everything else is permanent.** The full
transient-vs-permanent rationale, per-condition, is the versioned `status_contract` rustdoc
module in `sparq-server` (`crates/sparq-server/src/status_contract.rs`) — that module is part
of this contract. No `Retry-After` header is currently emitted.

## Explicitly-unstable surface

Served today, **NOT covered by the v1 freeze** — may change or disappear in any release
(consumers should pin an exact version if they depend on these):

- `/metrics` — the route and read-gating are stable habits, but the exposition *content*
  (metric names/labels) is unstable.
- `/subscriptions` (WebSocket) and `/subscriptions/sse` — the whole subscription protocol,
  including the `Sec-WebSocket-Protocol: bearer.<token>` auth form
  (`crates/sparq-server/SUBSCRIPTIONS.md`).
- `/admin/compact`, and the `backup`-feature routes `/admin/backup`, `/admin/backup/delta`,
  `/admin/restore` (and their `409` semantics).
- All opt-in-feature routes and parameters: `/complete`, `/tpf` (+`brtpf`),
  `/shacl/validate`, `/terse/transpile`, `/streams`, `/.well-known/void`, the Service-Description response on a
  query-less `GET /sparql`, the `text/n3` GSP `PATCH` dialect, and the `?generation` /
  `Sparq-Generation` generation-pinning surface (default build since `sq-ci2d6`, bounded to the
  ring's concurrency-retention window; `time-travel` extends it) — its *error statuses*, where
  documented in §6, are frozen; the surface itself is NOT yet part of the v1 freeze (rides gh-1416).
- `EXPLAIN` / `EXPLAIN ANALYZE` output format; audit/log record formats; CORS specifics;
  response headers not named in this document; exact error-message wording beyond the §6
  sentinels; default values of operator caps.

## How this contract is enforced

- `crates/sparq-server/tests/wire_contract.rs` — the served-surface snapshot suite: one direct
  test per documented endpoint behaviour and per §6 error class, asserting the documented
  status code, body shape, and `Content-Type` against the real hardened router (in-process,
  no network flakiness). A wire change that contradicts this document turns CI red.
- `crates/sparq-server/tests/status_contract.rs` — the retry-contract twin (transient vs
  permanent, sanitisation, byte-identical `401`).
- Supporting suites: `tests/protocol.rs`, `tests/query_method.rs`,
  `tests/jsonld_content_negotiation.rs`, `tests/hardening.rs`, `tests/auth.rs`.

## Ratification checklist (for the maintainer)

1. Confirm the [frozen v1 surface](#the-frozen-v1-surface) partition (promote/demote items —
   e.g. whether `/metrics`' exposition or the subscription protocol should join v1).
2. Cut the release line that carries it (see [`docs/release.md`](release.md)) and record the
   freeze in `CHANGELOG.md`.
3. Flip this document's [Status](#status-v1-proposed-not-yet-ratified) from *PROPOSED* to
   *in force*, and note it on [#1416](https://github.com/sparq-org/sparq/issues/1416) so PSS can
   pin the release line.

## Related

- [`docs/api-stability.md`](api-stability.md) — the in-process embedding freeze (#1248) and
  the shared tier model.
- `crates/sparq-server/src/status_contract.rs` — the versioned retry contract (rustdoc).
- [`skills/http-server/SKILL.md`](../skills/http-server/SKILL.md) — the full operational
  endpoint + hardening reference (superset; not a stability contract).
- [#1416](https://github.com/sparq-org/sparq/issues/1416) (PSS HTTP-freeze ask) ·
  [#1248](https://github.com/sparq-org/sparq/issues/1248) (embedding freeze) ·
  [#50](https://github.com/sparq-org/sparq/issues/50) (transient-vs-permanent contract).
