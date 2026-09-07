---
name: solid-lws-server
description: "Run and use the experimental native `sparq-lws-core` Solid/LDP (Linked Web Storage) server: configure its environment and storage backend, make authenticated LDP and WAC requests, negotiate Turtle or profile-aware JSON-LD, and query the WAC-scoped `/sparql` endpoint. Use for the Rust LWS server, not the separate `@sparq-org/solid-server` JavaScript development host."
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq native Solid/LWS server

## Persisted population research example

<!-- [GPT-6] -->
`pod_population_http` is an explicitly invoked research example, separate
from the native LDP server and its production routes. Build and prepare a
small controlled corpus:

```sh
cargo build --release --locked -p sparq-lws-core --example pod_population_http
target/release/examples/pod_population_http auth --auth-dir /tmp/pod-auth
target/release/examples/pod_population_http pack --corpus /tmp/pod-wac --profile smoke --pods 8 --model wac
target/release/examples/pod_population_http verify --corpus /tmp/pod-wac --verify-pods 8
target/release/examples/pod_population_http serve --corpus /tmp/pod-wac --auth-dir /tmp/pod-auth --bind 127.0.0.1:3100
```

In another process, send scheduled real HTTP requests:

```sh
target/release/examples/pod_population_http load --corpus /tmp/pod-wac --auth-dir /tmp/pod-auth --pods 8 --requests 1000 --rate 10 --arrival poisson --selection uniform --query-set population --out /tmp/pod-load.jsonl
```

All options are `--name value`. `pack --model acp` emits the alternative
policy encoding. `--profile history` selects the documented service-history
scenario; `--config-file file.json` supplies an explicit `PopulationConfig`.
`--profile entropy` retains the central record counts and rights while
using the declared larger, less compressible text payload scenario.
The `smoke` profile is a small control fixture and does not justify realistic
population capacity. `pack` refuses to overwrite existing packed files.
See the access-control benchmark skill for source evidence and assumptions.

The packed layout is `pods.nqpack` (one independent Zstandard frame per
populated Pod), `pods.index` (three little-endian unsigned 64-bit integers
per Pod: offset, compressed length, uncompressed length),
`pod-summaries.jsonl`, and a final `manifest.json` with configuration,
counts and file hashes. A Pod is generated and persisted before it can be
served; there is no request-time procedural population generation. Direct
indexed file lookup avoids keeping the whole population route map in RAM.

`serve` defaults to one Pod worker, a bounded queue, and a cache limited by
both `--cache-pods` and `--cache-bytes`. Those bytes are uncompressed source
bytes, **not measured graph heap bytes**. `--workers` divides both budgets
among deterministic Pod-affine workers. `--max-pod-bytes` rejects oversized
source snapshots rather than dropping their records. `--max-journal-bytes`
limits each Pod's update history. Use an external Linux cgroup with swap
disabled for a hard RAM limit and report charged page cache, process RSS,
admission failures and queue rejections. The example does not claim that
its source-byte cache limit is a complete memory bound.

`auth` generates distinct benchmark issuer and DPoP client keys. The private
`client-secrets.json` is created exclusively with owner-only permissions;
the server reads only `issuer-public.json`. Access tokens and fresh DPoP
proofs are cryptographically checked through `AuthContext`, including
audience, expiry, issuer trust, client identity and replay protection.
WebIDs passed to `Session` come from the verified token. The experiment
pins the benchmark issuer key and excludes live OIDC discovery, WebID
fetching, login/token-endpoint traffic, and TLS setup in its loopback
profile. It performs full token verification on each request. Set
`--base-url` identically in `serve` and `load` for the token audience and
DPoP target; `load --connect` independently selects the HTTP dial address.

The routes are:

| Route | Body | Authorization |
|---|---|---|
| `POST /pods/{id}/sparql` | SELECT or ASK | Verified session; WAC/ACP readable graph view |
| `POST /pods/{id}/update` | SPARQL Update | WAC/ACP data-update checks |
| `POST /pods/{id}/policy` | Explicit INSERT DATA / DELETE DATA | Shared manifest-owner administration |

Policy administration checks the verified WebID against the owner derived
from the trusted corpus manifest. It accepts only existing policy graphs
of the addressed Pod, validates all operations before applying any, rebuilds
authorization, atomically replaces the durable update journal, and syncs
the journal and directory entries before acknowledging success. A denied
or failed mutation evicts the affected cached dataset. Replay after
eviction/restart reconstructs the current policy view. A Pod is assigned
to one worker, so queries admitted after an acknowledged update observe
the new rights. This owner-management rule is identical for both language
variants and is **outside their policy-editing authorization semantics**.
The library's unsupported ACP ACR-editing path is not presented as working.

For ACP grant/revoke experiments, detach and restore the reader policy's
`acp:apply` link. Removing its sole matcher attribute can create an invalid
empty matcher and is not the benchmark's revocation encoding. The example
does not establish complete ACP conformance.

`verify` compares all generated query templates and principal classes
against physically filtered readable records, using the same SPARQ engine
for reference evaluation. It compares exact bags or ordered results and
checks record counts with an independent arithmetic oracle. This provides
independent authorization selection, not an independent SPARQL evaluator.
Sampling uses `--verify-pods` or explicit `--verify-pod-ids`. The campaign
selects minimum/maximum source sizes in every observed intensity class, verifies
each in a separate capped cgroup, and records OOM/admission separately from wrong
results. An unverified class prevents dataset capacity/equivalence admission.
An exploratory sample is not a
claim that every Pod has undergone every query.

The load driver supports `--arrival constant|poisson`,
`--selection sequential|uniform|hot|skew80-20`, `--seed`,
`--principal owner|recipient|outsider|public`, `--max-inflight`,
`--timeout-ms`, `--query-file`, and `--query-set population`.
`hot` is an explicitly named diagnostic with most requests to a small
working set; `skew80-20` is the protocol's declared skew scenario.
`--mix population --workload-file file.json` derives query/background/data
write/policy-write proportions from the workload input. Reads currently
sample declared templates uniformly; content writes replace one existing
record value, and policy writes alternate intended grant/revoke states.
These implementation choices must be frozen in the campaign protocol.
Policy triple deltas expose idempotent/no-op operations after drops or
failures; successful requests alone do not prove effective policy churn.

For the prospective main workload use schema v2:

```sh
target/release/examples/pod_population_http load --mix journeys --corpus /tmp/pod-wac --auth-dir /tmp/pod-auth --workload-file bench/ac/million/workload.json --scenario busy-period --selection uniform --seed 2026090601 --mutation-epoch 1 --duration-seconds 120 --requests 1000 --out /tmp/journeys.jsonl
# After stopping the server, reconcile durable commits and verify replayed record state:
target/release/examples/pod_population_http audit --corpus /tmp/pod-wac
```

`--mix population` preserves the exploratory pilot approximation. `--mix journeys`
selects journeys by their derived server-request demand and templates by their
frozen conditional family weights. Read identities use the declared owner,
recipient and anonymous proportions; writes use the owner. Every template uses
explicit named graphs, with the unnamed default graph empty. `verify
--workload-file ...` adds all journey templates to the neutral result oracle.

The main loader streams the complete persisted inventory into per-service prefix
counts; this driver memory is reported separately from server memory. It derives
ingestion and expiry volume from the actual retained service counts and declared
retention horizon, with modification rates and batch sizes from `execution_v2`.
Half of ingestion is background across all hosted Pods, including inactive ones;
foreground ingestion targets the active cohort. Write targets are weighted by
service inventory; the selected read skew does not erase heavy-user write demand.
Contacts are a snapshot, modified at the explicit annual rate. These are finite
stationary-retention scenarios, not observed future Solid traffic. New records go
into existing allowed resource graphs with the full corpus schema and literal
profile; resource/container creation is outside this workload.

`--plan-only true` emits demand/inventory metadata without sending requests.
Without `--rate`, the selected scenario's actual-inventory rate is used; an
explicit rate is a saturation/diagnostic override and both rates are recorded.
`--duration-seconds` and `--requests` are joint minima on the precomputed Poisson
schedule. The mutation epoch must be unique for repeated runs on mutated data;
independent paired runs reset journals and use the same epoch/seed. Scheduled
expiry ranges never overlap or wrap; exhausted ranges remain offered failures.

The optional `x-benchmark-records` observation header is accepted only on verified
owner mutations. It names at most eight existing content resource graphs/record
subjects, snapshots their current outgoing triples before/after, and writes actual
record/triple deltas and final hashes into the atomic journal before returning
the receipt. Snapshot work is included in request time. Policy observations name
only an ID and record the actual policy triple delta. These are bounded benchmark
instruments, not a second authorization path. `churn --state grant|revoke|probe-granted|probe-revoked` tests generated calendar
inheritance/private exceptions over HTTP with owner, recipient and anonymous
probes. `--expected-delta` requires an actual transition or declared no-op;
`--evict-pod` uses another Pod to test a one-entry cache. The campaign restarts
the server between revoked-state probes and reconciles policy receipts too.
`audit` must run without a live
writer; it reopens touched Pods, reconciles committed IDs (including responses
lost to transport errors), and compares the last recorded state of every selected
record with reconstructed data. Acknowledged operations, actual changed records,
no-ops, planned/unsent operations and unknown commits remain separate.

`bench/ac/million/run-campaign.py` consumes a frozen campaign JSON, uses separate
server systemd cgroups inside the capped study slice, records charged page cache
and CPU/I/O, compresses raw JSONL losslessly, and retains stop/quarantine outcomes.
The launcher owns paid resource creation and budget authorization; this runner
does not provision cloud resources. Both successful and failed runs close and
hash collected artifacts before publishing their completion marker; partial raw
streams are preserved losslessly. Cancellation stops the owned process group,
including wrapper children, and a failed warmup cannot pass a measured cell.
Network journey experiments remain separate
and unmeasured unless an explicit implementation and run artifact establish them.

<!-- [GPT-6] -->
The separate [preloaded evaluation](../../bench/ac/preloaded/README.md) adds
`--features population-native`, `prepare-native --corpus PATH`, and
`serve --storage-mode native|memory|cached`. Preparation writes `pods.native`
and its source-bound checksum sidecar; `--native-archive PATH` overrides the
archive location and `--native-compressed true|false` selects its encoding.
Native startup validates the whole archive once, opens every data index,
materializes real WAC/ACP authorization and replays trusted update journals.
Every worker retains its disjoint Pod partition before `all-population-ready`
and HTTP listening. No dataset opening, RDF parsing, journal replay or initial
materialization is allowed after readiness. Mapped content pages can leave RAM;
heap metadata, authorization and mutation overlays do not automatically spill.
Keep archive bytes and length immutable for every derived graph view lifetime.

Preloaded serving requires `--control-token-file PATH`: an unpredictable
alphanumeric operator capability in an owner-readable-only file, separate from
Pod ownership and DPoP. `drain` sends it to `POST /__benchmark/drain`, pausing
admission before body extraction and waiting for a FIFO fence from every worker,
including work whose HTTP client stopped waiting. A previously admitted slow body
must finish or fail first; never-admitted transport connections are outside the
barrier. `--connect URL` and `--drain-timeout-seconds N`
configure this operator request. Never publish the secret. Use read-only warmup
and a successful full drain before measurement; drain again after the client
finishes. Independent repetitions stop the server and reset task-owned application
journals, while dedicated restart correctness checks deliberately retain them.
Failed timed mutations restore both graph and authorization without dataset
reload; ambiguous journal failures poison the worker and require restart/replay.
These rules do not retroactively change the completed cached campaign.

The prospective `bench/ac/preloaded/run-preparation.py` executor consumes a
separate checksum-bound footprint proposal, keeps generated/native data and auth
on the dedicated data0 mount, and writes only bounded logs/JSON under the result
root. It measures complete native preparation and all-Pod readiness/drain in
separate bounded cgroups, preserving incomplete and unattempted populations.
It performs no timed workload. Its [reference](../../bench/ac/preloaded/README.md)
documents CLI identities, source equivalence, reuse, resource accounting and
the distinction between footprint completion and final capacity admission.

The separate `bench/ac/preloaded/run-campaign.py` consumes an externally frozen
main matrix after that pilot. It requires identical executor/binary build
revisions, preserved generator shape references and complete native preparation
of every selected population before any timed cell. Its versioned input and
explicit stop rules are documented in the same reference; it never invokes
the original cached server path. Every repetition retains preloaded state,
complete request schedules, drain accounting and native offline receipt audits.
[GPT-6] Closed offline audit logs are copied into their cell and losslessly compressed
at the parent location before the next cell; both evidence copies remain available.
Partial execution and unattempted populations/cells remain explicit in the
source-bound result rather than being counted as successful capacity evidence.
With `preloaded.admission_outcomes_version: 1`, the frozen
`preload_admission_failure` stop rule permits only confirmed local OOM and live
startup-deadline failures to continue the declared memory grid after checked unit
cleanup. `ExecStopPost` preserves local cgroup counters; missing/failed capture,
unknown exits and integrity/readiness errors remain stopping errors. These are
`admission-failed` attempts with no warmup or request workload, never percentiles.

`bench/ac/preloaded/analyze-campaign.py` independently reads that recursive native
result format. Its separate `native-preloaded-campaign-independent-accounting`
analysis kind requires exact source/binary/campaign/manifest review binding,
complete native population/storage checks, both no-activation drains and the
existing complete-offered response/mutation validators before headline eligibility.
It keeps partial/rejected evidence descriptive and does not fall back to the
earlier cached analysis. The reference documents resource/paging fields and the
optional prospectively frozen practical-equivalence margin.
Failed startup attempts have a separate `valid_for_admission_inference` field and
`admission_failure_cells` list. Their request metrics stay null and they cannot
enter latency headline cells; unsupported continuation invalidates the campaign.
The separate `retained_history_million_pod_admission` account derives per-model
startup states from validated central-history shapes and complete populations;
it excludes compact controls and keeps admission distinct from response capacity.
The optional `site/papers/solid-pod-preloaded-results.typ` helper accepts only
finalized, reviewed native analysis and loads no default or cached result JSON.

Raw JSONL preserves every offered request, including unsent requests at
the client's concurrency limit. The schedule is computed before load and
never slowed in response to the server. Records separate preparation and
dispatch lag, complete client response-body latency, scheduled-arrival
latency, failures and server diagnostic timers. `x-server-us` starts in
request middleware before body extraction and ends after result
serialization; it excludes socket delivery and any earlier transport
waiting. Use complete local HTTP latency as the conservative local SLO
measure and scheduled latency to expose overload. Neither component
percentiles nor this loopback profile establish a remote network SLO.

`bench/ac/million/run-instance.sh pilot` runs a bounded exploratory matrix
on a disposable Linux host. The cloud launcher preserves the study budget,
watchdog and teardown controls. Canonical mode requires an explicitly
frozen population ladder, rate and request count; workstation timings and
exploratory pilots remain noncanonical.

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
