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
self-contained in `src/lws/`: its own authorization server, over the same `Store` backend.
It runs over the
`memory` and `embedded` backends only; startup fails with `PSS_SPARQ_BACKEND=http`,
because a remote write reported as failed may still commit later and nothing fences it yet.
A request's Link headers may declare at most 128 relations holding
64 KiB of targets between them, and `Accept`, `Prefer`, `If-Match` and `If-None-Match` at most
64 members each (431 past either); the targets are weighed as resolved against the resource.
A resource's types, links and linkset are at most 256 KiB together, as stored: a write that would
make them larger gets `409`, and neither its content nor its metadata is written.

```bash
SOLID_SERVER_PROTOCOL=lws \
SOLID_SERVER_LWS_OWNER=https://alice.example/profile#me \
SOLID_SERVER_LWS_AS_KEY_FILE=./as-key.json \
cargo run -p sparq-lws-core
```

| Variable | Meaning |
|---|---|
| `SOLID_SERVER_LWS_OWNER` | Agent IRI allowed every action on the storage. Everyone else may act only on the resources they created. |
| `SOLID_SERVER_LWS_OPEN` (or `SOLID_SERVER_OPEN_MODE`) | `1` disables authorization entirely and implies `SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH`. Test-suite use only. |
| `SOLID_SERVER_LWS_PAGE_SIZE` | Container page size (default 100). |
| `SOLID_SERVER_LWS_AS_KEY_FILE` | P-256 private JWK for access tokens. A missing file is created with a fresh key (mode 0600) whose `kid` is its RFC 7638 thumbprint. Without a file the key lives only for the process. |
| `SOLID_SERVER_LWS_AS_PREVIOUS_KEY_FILE` | Key rotation: the P-256 JWK (public or private; only the public part is used) the access-token key replaced. It must exist and be a different key from the current one. Keys generated by older builds all carry `kid` `lws-as-1`; when both keys do, the current key goes by its thumbprint instead and the previous key keeps `lws-as-1`, so its tokens still verify. Startup still fails when both files hold the same key, or when the kids collide even after that. It is published in the JWKS, and tokens whose `kid` names it still validate until they expire. To rotate, move the old key file here and point `SOLID_SERVER_LWS_AS_KEY_FILE` at a new path; drop this variable once the token lifetime has passed. |
| `SOLID_SERVER_LWS_TOKEN_TTL_SECS` | Access-token lifetime (default 300). |
| `SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH` | `1` lets the server fetch from `http:`, loopback and private addresses (CID documents). Test-suite use only. |

What the server exposes, all discoverable from the storage description
(`GET /` with `Accept: application/lws+cid`):

- **Storage**: `application/lws+json` containers with paging links, data resources with
  conditional requests and single byte ranges, `POST` with `Slug`, `PUT`, `PATCH` of a JSON
  data resource with RFC 6902 JSON Patch (`application/json-patch+json`), `DELETE` (with
  `Depth: infinity` for non-empty containers), and read-only RFC 9264 linksets at
  `{resource}.meta`.
  Errors are `application/problem+json`. Bodies are stored as sent, so a body with a
  `Content-Encoding` other than `identity` gets `415`. A precondition header sent as several
  lines counts every line; an entity-tag list that cannot be read by the RFC 9110 grammar gets
  `412` (a comma inside a quoted tag is part of the tag; an empty list names no tag), and a date that is not one valid
  HTTP-date is ignored. A `POST` whose name is taken (or is being created,
  written or deleted right now) gets a numbered name and then a random suffix; when every try is
  taken it gets `409`. A JSON Patch is applied whole or not at all: one that is not a valid
  patch document gets `400`, one that cannot be applied (a missing path, a failed `test`) gets
  `422`, another patch format gets `415` with `Accept-Patch`, and a result larger than the body
  limit, more than 1,000 operations, or work past four times the body limit gets `413`. A patch
  that reads the content (`test`, `copy`, `move`) needs Read as well as Modify. `livez` and `readyz` are never
  given to a member of the root container, because the probes answer those paths. Stored metadata that cannot be read
  makes a request fail with `500` rather than fall back to defaults.
- **Authorization server**: metadata at `/.well-known/lws-configuration`, keys at
  `/.well-known/lws/jwks`, and RFC 8693 token exchange at `/.well-known/lws/token` for
  self-issued did:key and controlled identifier JWTs.
  Storage requests take `Authorization: Bearer <access token>`; a missing or bad token
  gets `401` with `WWW-Authenticate: Bearer as_uri="…", realm="…"`.
- **Authorization**: the owner may do anything, and the agent that created a resource may do
  anything with it.
- Writes and deletes are **whole or not at all**: a PUT that changes metadata and a `DELETE`
  (a whole `Depth: infinity` subtree included) record what each store step replaced and put it
  all back when a later step fails, so content, metadata, listings and validators (`ETag`,
  `Last-Modified`) are as they were. A delete too large to put back is refused with `409` before
  anything is removed. When putting back fails too, it is tried a few more times with the
  resource's locks held; if it still fails the request ends with `5xx` and the resources
  (with the containers whose listings they change) are **set aside**: answered `503` (`Retry-After`) at once while a background task, holding their
  locks and no request's admission slot, keeps putting the change back, so no other request sees
  the half-done state in between; a recursive delete sets aside everything under it it had not
  reached too. A request already waiting on a lock when its resource is set aside is answered
  `503` then, not left waiting. A resource stays set aside until every stuck change to it is
  put back. While set-aside changes hold 256 MiB or more to put back, new writes are answered
  `503` too (reads go on). A write's container is touched by the task that runs its store
  calls, so a client that goes away mid-write cannot leave the container's date behind. A
  create whose store reply was lost is removed whole the same way. A write that changes
  metadata and a recursive delete first store a durable intent: what each resource it will
  change holds now, kept as a member of a `urn:` container outside the storage (no request can
  name it). The intent is cleared once the change is put back, or once it is kept and its
  container's date has been moved on (until then it names that container); one a process stop
  left is settled when the server next starts, before it serves anything: a change cut short is
  put back, and only then (once nothing is left set aside) is a kept change's container
  touched, so no change put back can restore a container date from before it; a touch that
  waited is made, retried until it lands, once the set-aside changes are settled (every one is
  hidden before any is put back). Every intent is
  read before any is settled, so one that cannot be read stops the start with nothing changed
  or left running; what cannot be put back then is set aside as above. When a kept change's
  intent cannot be recorded as kept, it is read back (the store may have done it, the reply
  lost): still holding its plan, the change is put back and the request fails; otherwise the
  change is kept. While it cannot be read, the request fails and the change is set aside, to be
  settled the same way from its intent, and its container touched after. A
  create needs none: its metadata is written before its content and removed after it, so no
  stop leaves content without its metadata. A container listing never shows a change in
  flight to a member: it reads each member under that member's lock, and is read again once the
  change is over (`503` if its members keep changing); removing a member holds its container
  exclusively. A container whose modification time has not yet been, or could not be, moved on
  after a change has no `Last-Modified` until it is, so `If-Modified-Since` never answers `304`
  on a stale date. Every change to the store (a request's own steps, a touch, a rollback or a
  retried undo) goes through one tracked path, and a listing made while a change to the
  container or a member is in flight has no `Last-Modified`; one that overlapped a change is
  made again. A PUT with `Content-Range`
  (a partial PUT) is refused with `400`.
- A `Link` target that is not a URI reference is refused with `400`; entity tags in
  `If-Match`/`If-None-Match` compare byte for byte, obs-text included, and an empty list matches
  nothing.
- A resource's types come from two places, kept apart: `Link: <…>; rel="type"` headers and
  `<> a <…>` statements in Turtle content. A PUT replaces the content-stated types, and replaces
  the header-declared types only if it sends `rel="type"` headers of its own. Other Link
  relations change only with `Prefer: set-linkset`.
- Every write and delete runs its store calls in a task that holds the resource's locks until
  the store answers, so a client that disconnects cannot release them early.

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
