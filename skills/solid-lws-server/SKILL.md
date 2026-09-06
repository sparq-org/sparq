---
name: solid-lws-server
description: "Run and use the experimental native `sparq-lws-core` Solid/LDP (Linked Web Storage) server: configure its environment and storage backend, make authenticated LDP and WAC requests, negotiate Turtle or profile-aware JSON-LD, and query the WAC-scoped `/sparql` endpoint. Use for the Rust LWS server, not the separate `@sparq-org/solid-server` JavaScript development host."
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
direction. Protocol `default-graph-uri` and `named-graph-uri` parameters may
select from that authorized dataset; they cannot make an unreadable resource
visible.

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
