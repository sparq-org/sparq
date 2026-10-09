---
name: solid-lws-server
description: "Run and use the experimental native `sparq-lws-core` Solid/LDP (Linked Web Storage) server: configure its environment and storage backend, make authenticated LDP and WAC requests, negotiate Turtle or profile-aware JSON-LD, and query the WAC-scoped `/sparql` endpoint, or run the W3C LWS 1.0 protocol mode (`SOLID_SERVER_PROTOCOL=lws`). Use for the Rust LWS server, not the separate `@sparq-org/solid-server` JavaScript development host."
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq native Solid/LWS server

Use `sparq-lws-core` as an experimental native Solid/LDP server. It is not a
replacement for the supported TypeScript `prod-solid-server`, and its default
storage is ephemeral. Use `skills/javascript-wasm/SKILL.md` instead for the
separate `@sparq-org/solid-server` loopback development host.

Query entry points and structural rewrites retain VERSION announcements;
see the [version-pinned EBV rules](../sparql-query/ebv-dialects.md). Unsupported
or incompatible labels do not silently select REC 2013.

## Start a local server

Run the default build with the in-memory backend:

```sh
cargo run -p sparq-lws-core
```

It listens on `127.0.0.1:3000` and uses `http://localhost:3000` as its public
base URL unless configured otherwise. The default Cargo features are:

- `embedded-sparq`, which enables the in-process SPARQ engine backend.
- `sparql-endpoint`, which mounts the query-only `/sparql` route.

Use `--no-default-features` for the engine-free Solid core tier; that tier keeps
LDP and WAC but does not expose `/sparql`.

For a deployed instance, set at least:

```sh
SOLID_SERVER_BASE_URL=https://solid.example \
SOLID_SERVER_BIND=127.0.0.1:3000 \
SOLID_SERVER_TRUSTED_ISSUER=https://idp.example \
cargo run -p sparq-lws-core
```

`SOLID_SERVER_BASE_URL` must be the public origin seen by clients and encoded in
resource IRIs. `SOLID_SERVER_AUDIENCE` defaults to that URL. Keep
`SOLID_SERVER_ALLOW_LOOPBACK` and `SOLID_SERVER_BIDIRECTIONAL=off` for local
development or conformance work only; the normal posture requires HTTPS IdP and
WebID URLs, DPoP, and strict WebID-to-issuer verification.

Terminate TLS at a trusted reverse proxy, or set both
`SOLID_SERVER_TLS_CERT` and `SOLID_SERVER_TLS_KEY` to readable PEM paths.
Setting only one makes startup fail. See `skills/http3-server/SKILL.md` for the
default-off `http3` feature and `skills/helm-deploy/SKILL.md` for Kubernetes.

## Choose a data backend

Set `PSS_SPARQ_BACKEND` to one of:

- `memory` (default): an ephemeral in-memory test double.
- `embedded`: the default-enabled in-process engine. Set
  `SOLID_SERVER_SPARQ_DIR` for a directory-backed graph; without it the graph is
  ephemeral.
- `http`: a remote SPARQ service. Build with `--features http-sparq` and set
  `SOLID_SERVER_SPARQ_ENDPOINT` to that service's `/sparql` URL.

The native binary currently uses an in-memory blob backend. A durable/shared
RDF index therefore does not by itself make resource bodies durable; do not
present the native image as a durable production data service.

The seed variables are test-only:

- `SOLID_SERVER_SEED_CONFORMANCE=1` provisions conformance fixtures.
- `SOLID_SERVER_SEED_DEMO=1` provisions a shared public demo playground.
- `SOLID_SERVER_SEED_BENCH=1` provisions benchmark fixtures.

They are refused on non-memory backends unless
`SOLID_SERVER_ALLOW_SEED_NONMEMORY=1` is explicitly set. Use that escape hatch
only for an ephemeral test instance.

## Authenticate requests

Except for public reads and discovery/health routes, requests pass through
Solid-OIDC access-token verification and DPoP proof verification, then WAC.
Supply both headers:

```text
Authorization: DPoP ACCESS_TOKEN
DPoP: FRESH_REQUEST_BOUND_PROOF
```

Generate the DPoP proof for the exact HTTP method and target URI. Reusing a
proof fails because its `jti` is replay-protected. A valid token proves identity
but does not bypass WAC; the applicable resource or inherited container ACL
must grant the requested `acl:Read`, `acl:Write`, `acl:Append`, or
`acl:Control` mode.

A rule may name its subject with `acl:agent`, `acl:agentClass foaf:Agent` (public)
or `acl:AuthenticatedAgent`, or `acl:agentGroup`. A group grant resolves only when
the group document — the group IRI without its fragment — is stored **on this pod**
and lists the requester as `<group> vcard:hasMember <webid>`. Everything else is
fail-closed and grants nothing: an anonymous request, a group document that is
missing, malformed or silent about the requester, and a group IRI on another origin
(the server resolves membership from its own storage and makes no outbound fetch).
A group grant is per-requester, so it never appears in the `public=` audience of
`WAC-Allow`.

WAC also covers notification subscriptions: a `POST` to the
`WebSocketChannel2023` subscription service needs `acl:Read` on the topic
(`acl:Control` when the topic is an `.acl`), and the WebSocket receive endpoint
re-checks that same mode for the subscriber when the socket connects and again
before every notification it sends — so a revoked grant is not replayable through
an already-issued `receiveFrom` URL, and an open socket is closed (code 1008) once
its subscriber loses read access. With the `odrl-authz` gate attached, its deny
applies here exactly as it does to `GET`.
Lacking the mode returns `403`, whether or not the topic exists. A topic outside this
server's storage root is refused with `400` before the WAC gate runs.

## Serve provider WebIDs off the pod (optional)

`SOLID_SERVER_IDENTITY_ENABLE=1` is **off by default**. When set, the server serves
provider-issued WebID documents from a separate identity host — `id.<base authority>`
unless `SOLID_SERVER_IDENTITY_HOST` overrides it — with WebIDs of the form
`https://<identity-host>/<handle>#me`. Those documents answer `GET` and `HEAD` only, are
publicly readable, and carry no `.acl` link.

Enable this when you do not want the Solid-OIDC trust root inside the pod. A WebID
document tells every resource server which issuers may mint tokens for that WebID, so an
in-pod, owner-writable, WAC-governed WebID is one over-broad `acl:default` grant away from
letting someone else's grant rewrite that trust root.

Two consequences to design around:

- The reserved `/.identity/**` path is refused with `404` on the LDP surface for every
  method, **whether or not the flag is set**. You cannot create, read, or write an ACL for
  anything under it. That is deliberate, not a gap.
- With the flag on, the conformance seed mints identity-host WebIDs whose documents hold
  the `solid:oidcIssuer` and `pim:storage` statements, and demotes the in-pod
  `/{u}/profile/card` to a user-editable profile that carries neither. Read identity from
  the WebID document, not from the in-pod card.

## Use the LDP surface

Use ordinary HTTP methods against resource IRIs:

```sh
# Read an RDF resource.
curl -H 'Accept: text/turtle' https://solid.example/alice/profile/card

# Replace or create a resource. Add fresh Authorization and DPoP headers.
curl -X PUT \
  -H 'Authorization: DPoP ACCESS_TOKEN' \
  -H 'DPoP: FRESH_REQUEST_BOUND_PROOF' \
  -H 'Content-Type: text/turtle' \
  --data-binary '<#me> <http://xmlns.com/foaf/0.1/name> "Alice" .' \
  https://solid.example/alice/profile/card

# Create a child below a container; use the returned Location.
curl -i -X POST \
  -H 'Authorization: DPoP ACCESS_TOKEN' \
  -H 'DPoP: FRESH_REQUEST_BOUND_PROOF' \
  -H 'Content-Type: text/turtle' \
  -H 'Slug: note' \
  --data-binary '<#it> <http://purl.org/dc/terms/title> "Note" .' \
  https://solid.example/alice/
```

`GET` and `HEAD` read resources. `PUT` creates or replaces, `POST` mints a
collision-resistant child IRI below a container, `PATCH` accepts supported
Solid/SPARQL patch forms, and `DELETE` removes a resource or an empty
container. Use `If-Match` and `If-None-Match` for conditional mutations.
Container reads include generated `ldp:contains` triples.

Use a trailing slash for containers. A resource and container cannot coexist at
slash-equivalent paths. ACL resources use the sibling `.acl` convention and
are themselves protected by `acl:Control`.

## Negotiate RDF representations

Request Turtle or JSON-LD:

```sh
curl -H 'Accept: text/turtle' https://solid.example/alice/profile/card
curl -H 'Accept: application/ld+json' https://solid.example/alice/profile/card
```

The JSON-LD reader also recognizes the canonical expanded and compacted
profile parameters and echoes the honored profile in `Content-Type`. Its
compacted form is a local, context-free structural compaction: it does not
fetch a remote context and is not the full W3C Compaction Algorithm. See
`skills/jsonld/SKILL.md` for the exact profile behavior.

An unknown `Accept` media type falls back to the Solid default
`text/turtle`. Explicitly refusing every producible type with `q=0` returns
`406 Not Acceptable`. Non-RDF resources preserve their binary media type and
support byte-range reads.

## Query readable RDF with `/sparql`

The default `sparql-endpoint` feature exposes authenticated `GET` and `POST`
SPARQL Protocol query operations:

```sh
curl -G https://solid.example/sparql \
  -H 'Authorization: DPoP ACCESS_TOKEN' \
  -H 'DPoP: FRESH_REQUEST_BOUND_PROOF' \
  -H 'Accept: application/sparql-results+json' \
  --data-urlencode 'query=SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }'
```

For `POST`, send either `application/sparql-query` or
`application/x-www-form-urlencoded`. The v1 route supports `SELECT`, `ASK`,
and `CONSTRUCT`; it rejects `DESCRIBE` and SPARQL Update. `SELECT` and `ASK`
return `application/sparql-results+json`; `CONSTRUCT` returns
`application/n-triples`.

The endpoint assembles one named graph per RDF resource that the caller may
read under WAC. The default graph is empty. Failed enumeration,
authorization, body reads, or RDF parsing exclude a resource in the safe
direction. A resource rewritten concurrently (its body reclaimed between
authorization and read) is re-planned and re-authorized instead; if the race
persists the query fails with `503` and `Retry-After` rather than silently
dropping that graph. Protocol `default-graph-uri` and `named-graph-uri` parameters may
select from that authorized dataset; they cannot make an unreadable resource
visible.

## Run the LWS 1.0 protocol (`SOLID_SERVER_PROTOCOL=lws`)

Set `SOLID_SERVER_PROTOCOL=lws` to serve the W3C Linked Web Storage 1.0 protocol
(tracking `w3c/lws-protocol` main) instead of the Solid/LDP surface. The mode is
self-contained in `src/lws/`: its own authorization server, access grants,
notifications and type index, over the same `Store` backend. It runs over the
`memory` and `embedded` backends only; startup fails with `PSS_SPARQ_BACKEND=http`,
because a remote write reported as failed may still commit later and nothing fences it yet.
Anyone authenticated may hold at most 16 access requests and 16 subscriptions (429 past
that; the owner's subscriptions are not counted), the server at most 512 of each (507),
and each such request body is at most 16 KiB (413). A type-search filter is at most 8 KiB and an
access grant 256 KiB; these limits apply while the body is read. The owner's subscriptions have a
separate share of 512. A request's Link headers may declare at most 128 relations holding
64 KiB of targets between them, and `Accept`, `Prefer`, `If-Match` and `If-None-Match` at most
64 members each (431 past either); the targets are weighed as resolved against the resource.
A resource's types, links and linkset are at most 256 KiB together, as stored: a write that would
make them larger gets `409`, and neither its content nor its metadata is written.

```bash
SOLID_SERVER_PROTOCOL=lws \
SOLID_SERVER_LWS_OWNER=https://alice.example/profile#me \
SOLID_SERVER_LWS_AS_KEY_FILE=./as-key.json \
SOLID_SERVER_LWS_NOTIFY_KEY_FILE=./notify-key.json \
cargo run -p sparq-lws-core
```

| Variable | Meaning |
|---|---|
| `SOLID_SERVER_LWS_OWNER` | Agent IRI allowed every action on the storage, and the only one who may list or issue grants. |
| `SOLID_SERVER_LWS_OPEN` (or `SOLID_SERVER_OPEN_MODE`) | `1` disables authorization entirely and implies `SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH`. Test-suite use only. |
| `SOLID_SERVER_LWS_PAGE_SIZE` | Container and type-search page size (default 100). |
| `SOLID_SERVER_LWS_AS_KEY_FILE`, `SOLID_SERVER_LWS_NOTIFY_KEY_FILE` | P-256 private JWKs for access tokens and webhook signatures. A missing file is created with a fresh key (mode 0600); a generated access-token key's `kid` is its RFC 7638 thumbprint (the webhook key's is `notify-key`). Without a file a key lives only for the process. |
| `SOLID_SERVER_LWS_AS_PREVIOUS_KEY_FILE` | Key rotation: the P-256 JWK (public or private; only the public part is used) the access-token key replaced. It must exist and be a different key from the current one. Keys generated by older builds all carry `kid` `lws-as-1`; when both keys do, the current key goes by its thumbprint instead and the previous key keeps `lws-as-1`, so its tokens still verify. Startup still fails when both files hold the same key, or when the kids collide even after that. It is published in the JWKS, and tokens whose `kid` names it still validate until they expire. To rotate, move the old key file here and point `SOLID_SERVER_LWS_AS_KEY_FILE` at a new path; drop this variable once the token lifetime has passed. |
| `SOLID_SERVER_LWS_TOKEN_TTL_SECS` | Access-token lifetime (default 300). |
| `SOLID_SERVER_LWS_DELIVERY_QUEUE`, `SOLID_SERVER_LWS_DELIVERY_WORKERS`, `SOLID_SERVER_LWS_DELIVERY_PER_INBOX` | Webhook delivery bounds: deliveries waiting or in flight (default 1024; past it a notification is dropped and logged), deliveries in flight (default 16), and in flight to one inbox origin (default 2). |
| `SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH` | `1` lets the server fetch and deliver to `http:`, loopback and private addresses (CID documents, OIDC issuers, webhook inboxes). Test-suite use only. |
| `SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS` | Comma-separated OpenID Provider issuers (exactly as their ID Tokens' `iss`, which must equal the discovery document's `issuer`) whose identities get the reserved half of the DPoP replay cache. Every other provider shares the other half, each provider and each identity held to its own share, so open registration elsewhere cannot lock out these providers' users. |

What the server exposes, all discoverable from the storage description
(`GET /` with `Accept: application/lws+cid`):

- **Storage**: `application/lws+json` containers with paging links, data resources with
  conditional requests and single byte ranges, `POST` with `Slug`, `PUT`, `PATCH`
  (`application/merge-patch+json` or `application/json-patch+json`), `DELETE` (with
  `Depth: infinity` for non-empty containers), and RFC 9264 linksets at `{resource}.meta`.
  Errors are `application/problem+json`. Bodies are stored as sent, so a body with a
  `Content-Encoding` other than `identity` gets `415`. A precondition header sent as several
  lines counts every line; an entity-tag list that cannot be read by the RFC 9110 grammar gets
  `412` (a comma inside a quoted tag is part of the tag; an empty list names no tag), and a date that is not one valid
  HTTP-date is ignored. A `POST` whose name is taken (or is being created,
  written or deleted right now) gets a numbered name and then a random suffix; when every try is
  taken it gets `409`. A `PATCH` whose result would exceed the body limit gets `413`, for merge
  patches as well as JSON Patch. Every JSON Patch operation, `move` included, is counted by its
  full serialized size (keys and separators as well as values). A JSON Patch may hold at most
  1,000 operations, and the bytes its operations copy, move, add, replace or test are charged
  against a work budget of four times the body limit; past either it gets `413`. A `POST`
  whose request is cancelled after the member is written is still announced; a `DELETE` that
  fails, and is being put back, announces nothing. `livez` and `readyz` are never
  given to a member of the root container, because the probes answer those paths. A linkset
  `PATCH` whose result nests too deeply to store gets `422`. A linkset `PATCH` is measured against the
  body limit as it will be served, with the server-managed links put back. Its relative `anchor`
  and `href` values are resolved against the linkset's own URI (RFC 9264 section 4) and stored
  absolute, and one that is not a URI reference gets `422`. JSON Patch paths are
  RFC 6901 pointers read by one parser: an array index is `0` or digits without a leading zero
  (`-` only where an add may append), and an escape other than `~0` or `~1` gets `400`. Stored metadata that cannot be read
  makes a request fail with `500` rather than fall back to defaults.
- **Authorization server**: metadata at `/.well-known/lws-configuration`, keys at
  `/.well-known/lws/jwks`, and RFC 8693 token exchange at `/.well-known/lws/token`.
  A Solid-OIDC ID Token is one carrying `webid` or addressed to `solid`. It must carry a
  `webid` that is an http(s) URL (that WebID is the agent), have an `aud` array holding
  both `solid` and its `azp`, be
  bound by `cnf.jkt`, and come with a `DPoP` proof of the bound key on the token request
  (RFC 9449: `htm` POST, `htu` the token endpoint, fresh `iat`, unused `jti`). Any other ID
  Token's agent is its `sub`, whose controlled identifier document names its OpenID
  Provider; it must be addressed to this server, and when bound by `cnf.jkt` it needs the
  same proof. The proof key is matched by the thumbprint of the key itself, whatever its JWK
  spelling. The identity document may be compact JSON whose only contexts are the CID, DID or
  LWS ones (read by its keys; the Solid issuer by its full IRI, as a node reference
  `{"id": …}`), any other JSON-LD form
  (expanded, flattened, a graph, an inline context; read as RDF without loading remote
  contexts, and refused when what it could expand to, counted before parsing from its values,
  longest string and inline contexts, is past a fixed multiple of its size) or Turtle. A token time
  (`exp`, `iat`, `nbf`) may carry a fraction (RFC 7519 NumericDate); one that is not a number
  between 1970 and 9999 is refused.
  Storage requests take `Authorization: Bearer <access token>`; a missing or bad token
  gets `401` with `WWW-Authenticate: Bearer as_uri="…", realm="…"`.
- **Access grants and requests** (Access Profile) under `/.lws/grants/` and
  `/.lws/requests/`. Documents need an `@context` that includes
  `https://www.w3.org/ns/lws/v1` (otherwise `400`). Grants are ODRL-style policies with
  client, format, type, purpose and dateTime constraints. A target's `type` is
  `DataResource`, `Container` or `StorageResource`, and its values name single resources,
  not their members. A policy with no `target` covers every resource of the grant's
  `storage`. A `purpose` constraint never holds, because the draft does not say how a
  request states its purpose. A `format` constraint compares media types as RFC 9110 does
  (case-insensitive type, subtype and parameter names, quoted or bare values, any parameter
  order). The owner and a resource's creator are always allowed. A
  new request notifies the owner's inbox when the owner's JSON(-LD) identity document
  names one. A new grant notifies the inboxes of the requests its assignees made themselves,
  as well as its own `inbox`.
- **Webhook notifications** under `/.lws/subscriptions/`, signed per RFC 9421 with the
  key in the storage description's `verificationMethod`. Deliveries go through a bounded queue and worker pool (see the
  `SOLID_SERVER_LWS_DELIVERY_*` variables); a Delete is announced only once the removal happened.
  Each delivery attempt, retries included, first checks that its subscription still exists and
  has not expired, and that the subscriber may still read the resource, reading the resource under
  its shared lock, as a GET does. A Delete is checked against
  the resource as it was before removal. A delivery that fails a check is dropped.
- Writes and deletes are **whole or not at all**: a PUT or PATCH that changes metadata and a
  `DELETE` (a whole `Depth: infinity` subtree included) record what each store step replaced and put it
  all back when a later step fails, so content, metadata, listings and validators (`ETag`,
  `Last-Modified`) are as they were. A delete too large to put back is refused with `409` before
  anything is removed. When putting back fails too, it is retried with the resource's locks still
  held until it succeeds, so no other request sees the half-done state in between; a create
  whose store reply was lost is removed whole the same way.
- A `Link` target that is not a URI reference is refused with `400`; entity tags in
  `If-Match`/`If-None-Match` compare byte for byte, obs-text included, and an empty list matches
  nothing.
- A resource's types come from two places, kept apart: `Link: <…>; rel="type"` headers and
  `<> a <…>` statements in Turtle content. A PUT replaces the content-stated types, and replaces
  the header-declared types only if it sends `rel="type"` headers of its own. Other Link
  relations change only with `Prefer: set-linkset`.
- Every write and delete runs its store calls in a task that holds the resource's locks until
  the store answers, so a client that disconnects cannot release them early.
- A grant or subscription `DELETE` takes effect once its stored record is gone, even if cleaning
  up its bytes then fails.
- An access request notifies the owner through the bounded delivery queue. The lookup of the
  owner's inbox is shared and cached.
- The grant, request and subscription services are containers: their listings are
  negotiated (`lws+json`, `ld+json` or `json`), paged at the page size, and carry an ETag
  and `up`/`type`/`linkset` links. The linksets are read-only.
- **Type index** (`GET /.lws/types/index`) and **type search** (`QUERY /.lws/types/search`
  with an `application/lws-query+json` filter), both scoped to what the caller may read.
  A QUERY's filter is always its body; the `q` parameter only carries it on the `GET` page
  links. The filter is a JSON object (`{}` matches everything; an empty body gets `400`).
  Filter IRIs must be valid absolute IRIs (RFC 3987), or the filter gets `400`. When
  any listing, permission check or metadata read fails, the whole index fails with one
  generic `500` that names no resource. Both carry an `ETag` of what they serve and
  answer `If-Match` / `If-None-Match` as a read does; a coded QUERY body gets `415`.
  While walking the storage they keep only what they return (a search its matches, the index
  its distinct types) and the walk's own state (the listing in hand and the members still to
  visit), at most 16 MiB in all; past that the request gets `507`.

Conformance runs against the public suites; the scripts and the CI floor live in
`crates/sparq-lws-core/conformance/lws/` (`touchstone.sh <module>`, `lws-net.sh`,
`floor.json`) and run in `.github/workflows/lws-conformance.yml`.

## Follow the normative specs, not this server

Several behaviours here implement an external specification, and **the spec is the
contract** — where this server and the spec disagree, treat the spec as right and the
server as the defect. Write integration code against the spec, and pin the same revision
the server pins:

- **Solid-OIDC access tokens and DPoP proofs**: baseline verification on the normal
  cache-miss path is delegated to the pinned
  [`solid-oidc-verifier`](https://github.com/jeswr/solid-oidc-verifier) git dependency
  (see `crates/sparq-lws-core/Cargo.toml` for the exact revision), so what a token or
  proof must look like is whatever that revision enforces. Delegated is not
  pass-through, though, and integrators should know the two places this crate
  participates. On a **verified-token-cache hit** the token signature and claims are not
  re-verified (they were checked once on the miss, and the entry is keyed by the token
  and expires at `min(token exp, a shorter validation-freshness TTL)`), while the fresh
  DPoP proof is verified locally on every request — proof signature, `htm`/`htu`/`iat`,
  `ath`, the `jti` replay mark against the same shared replay store, and the `cnf.jkt`
  binding — orchestrated from the verifier's own public primitives
  (`crates/sparq-lws-core/src/auth_cache.rs`). Separately, the
  server layers **opt-in** proof-of-possession tiers on top of an already-verified token:
  RFC 8705 mTLS cert-bound tokens, and DPoP-SK below. Both are off by default
  (`crates/sparq-lws-core/src/auth.rs`).
- **DPoP-SK**, the sender-key proof-of-possession tier, follows the
  [DPoP-SK profile](https://jeswr.github.io/dpop-sk-spec/). The spec's Appendix-A worked
  example is executed as a test vector in `crates/sparq-lws-core/src/pop/sk/derive.rs`, so
  a drift between the spec's derivation and this implementation fails a test rather than
  going unnoticed. Derive session keys per the spec, not by reading the Rust.
- **WAC**, **LDP**, and the **Solid Protocol** govern the access-control and resource
  surfaces above; the sections of this skill describe how this server exposes them, not
  what they require.

For the design decisions behind the identity host, the in-process engine backend, the
public-read fast path, and the existence-non-disclosure guards, read
[`research/lws-design-records.md`](../../research/lws-design-records.md). It is
reconstructed from the code and cites it line by line; it also maps the source-repo
`decisions/` and `docs/design/` paths that this crate's doc-comments still reference.

## Operational checks and boundaries

- `GET /livez` and `GET /readyz` are unauthenticated probes.
- Do not expose the development seed modes or loopback auth escape hatch in
  production.
- Scale authenticated instances with a shared replay store only via the
  default-off `redis-replay` feature and
  `SOLID_SERVER_REPLAY_REDIS_URL`; otherwise DPoP replay state is
  per-instance.
- Use `crates/sparq-lws-core/README.md` and `src/main.rs` as the authoritative
  inventory for advanced cache, transport, identity-host, reconciliation, and
  proof-of-possession environment variables.
- Use `skills/usage-control-policy/SKILL.md` for the default-off
  `odrl-authz` read/query gate.
- Use `skills/access-control/SKILL.md` § *LWS-server admission seam* for the
  default-off `trust-graph` feature. It adds the library function
  `authz::trust_admit::trust_admit_verdict`, which is deliberately not wired
  into the request pipeline, so enabling it changes no request's outcome.
