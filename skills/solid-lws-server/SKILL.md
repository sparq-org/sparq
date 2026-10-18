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
- `authn-ssi` and `authn-openid`, two of the LWS 1.0 authorization server's
  authentication suites (§ *Run the LWS 1.0 protocol* below). `authn-saml` is off.

Use `--no-default-features` for the engine-free Solid core tier; that tier keeps
LDP and WAC but does not expose `/sparql`, and it compiles in no authentication
suite, so LWS mode offers none.

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
  `SOLID_SERVER_SPARQ_ENDPOINT` to that service's `/sparql` URL. The service must
  read its own writes: a query sent after an update has completed must see what the
  update committed (one `sparq-server` does). An eventually consistent or replicated
  endpoint that can lag is not supported, because the LWS server decides what a failed
  write left behind by reading the store straight after it. An update that times out
  can still commit on the service afterwards. The creates and removals of grants,
  requests and subscriptions, and the content write of a resource create, are fenced
  against that, but other resource writes and their rollback are not, so the native LWS server still refuses this backend (below).

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
`acl:Control` mode. An N3 `PATCH` with a `solid:where` or `solid:deletes` reads the target, so
it needs `acl:Read` too (`acl:Control` for an `.acl`). A `PUT`'s or `PATCH`'s response carries
the new `ETag` only to a writer that may read the target. An insert-only `PATCH` by a writer
without `acl:Read` appends the statements as N-Triples lines to a stored Turtle, N-Triples or
N-Quads document without parsing it, so whether it succeeds does not depend on what is stored;
any other stored type (JSON-LD, ...) is refused to it as a write without access is. The stored
bytes are still read and rewritten whole, so the time an append takes follows their size. A
reader's `PATCH` reads the document by the same stored type, keeps that type, and leaves an
N-Quads document's named graphs as stored: a `PATCH` changes the default graph. Blank nodes a
`PATCH` inserts are always new ones. Deleting a container also needs `acl:Read` on it
(whether it may go depends on what it holds). A `POST` names its member
with the slug and a random suffix, so the name says nothing of what else was created.

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
an already-issued `receiveFrom` URL. Notifications follow the subscriber's current
access: one it may no longer read is not sent (nor does it close the socket, or
decide a lag close, `1011`), and an access timer (every 30 seconds) closes the
socket (code 1008) once its subscriber loses read access, so when a socket closes
never turns on changes it may not read. A channel lasts as long as its receive
token: the subscribe response's `endAt` says until when, and at that instant the
socket is closed (code 1008); a channel once expired never delivers again, even
when its `receiveFrom` URL is used again in the same process. Subscribe again for
a new channel. With the `odrl-authz` gate attached, its deny
applies here exactly as it does to `GET`.
Lacking the mode returns `403`, whether or not the topic exists. A topic outside this
server's storage root is refused with `400` before the WAC gate runs. A topic on it is taken
in one canonical form, the IRI `parse_target` gives its path (query and fragment dropped), for
the WAC check, the receive token and the notifications alike; a topic with none — a linkset
record (`X.meta`, in any case), anything under `/.lws/`, a dot segment (%-decoded too) — gets the same `403`
as a topic the caller may not read.

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
container. Use `If-Match`, `If-None-Match` and `If-Unmodified-Since` for
conditional mutations. `If-Unmodified-Since` is ignored when `If-Match` is
present, when the date is not one valid HTTP-date (IMF-fixdate, RFC 850 or
asctime; a date sent on several header lines is none), and for containers, whose
listing can change without moving their recorded time. `If-Modified-Since` on a
read accepts the same three forms. A `GET` or `HEAD` evaluates `If-Match` and
`If-Unmodified-Since` too, before `If-None-Match`, and answers `412` when they
fail; on a read `If-Match` must name the tag of the representation selected,
not another format's. An entity-tag list may span several header lines; one that
is not the RFC 9110 grammar (only space and tab may surround a tag) names no tag,
so it fails `If-Match` and never triggers `If-None-Match`. A `Range` applies only
while `If-Range`, when sent on one header line, is the strong `ETag` of the
representation selected; otherwise the whole body is served. As with a concrete
ETag validator, a writer without Read on the target is refused rather than told
the outcome.
Container reads include generated `ldp:contains` triples. A container's `ETag`
covers its own record and its membership: a tag read in any format is a valid
`If-Match` for the container until it or its membership changes. Creating a
container returns its current tag; replacing one returns no `ETag`. Generated
and re-serialised representations list children in IRI order and label blank
nodes `_:b0`, `_:b1`, … by position, so one state always reads back as the
same bytes.

Use a trailing slash for containers. A resource and container cannot coexist at
slash-equivalent paths. ACL resources use the sibling `.acl` convention and
are themselves protected by `acl:Control`. A name ending in `.meta` (any case, %-decoded too)
is where the LWS surface keeps a resource's linkset record, so the LDP surface answers `404` for
it on every method a request is routed for (a CORS preflight is answered before routing),
whichever surfaces are enabled. A container of that name (`X.meta/`) and what lies under it are
ordinary resources: a stored record never lists as a member, conflicts with that container or
serves as a group document, and a container whose only members are such records is empty. So
is any path under `/.lws/` below the storage root, where the LWS surface keeps its grants, access
requests and subscriptions (a `.lws` segment deeper down is an ordinary name). A path with an empty segment inside it (`/a//b`), or a dot segment (`.` or `..`, as sent or %-decoded, so `%2E%2E` too), is a `400`; an encoded separator (`%2F`) is part of a name; leading and trailing repeated slashes are kept as part of the name, so what is stored under such a name can be read and deleted, but a `PUT`, `PATCH` or `POST` naming one (a `POST`'s container included) is a `400`, decided on the name before anything is authorized or looked up.

## Negotiate RDF representations

Request Turtle, JSON-LD, N-Triples or N-Quads:

```sh
curl -H 'Accept: text/turtle' https://solid.example/alice/profile/card
curl -H 'Accept: application/ld+json' https://solid.example/alice/profile/card
curl -H 'Accept: application/n-triples' https://solid.example/alice/profile/card
```

N-Triples and N-Quads are read formats only. A PUT or POST body sent with
either media type is stored as an opaque non-RDF resource, not parsed as RDF,
and PATCH refuses them with 415. A resource is one default graph, so its
N-Quads lines carry no graph label.
`text/n3` is not produced and falls back to Turtle. When q-values tie, the
stored format wins, then Turtle, JSON-LD, N-Triples, N-Quads in that order,
so `application/*` still yields JSON-LD.

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
because a remote write reported as failed may still commit later. Only the creates and
removals of service records (grants, requests, subscriptions), and a resource create's
content write, are fenced against that (see below); other resource writes, and putting a
failed change back, are not.
Anyone authenticated may hold at most 16 access requests and 16 subscriptions (429 past
that; the owner's subscriptions are not counted), and each such request body is at most 16 KiB
(413). Every subscriber has that capacity of 16 in full: how many subscriptions anyone else
holds never makes it smaller, and the count behind a 429 is read from the requester's own
records alone. The server holds at most 512 access requests and 512 subscriptions in all (507),
the storage owner's subscriptions counted apart against 512 of its own. Those totals are
resource bounds, not capacities: a 507 says the storage is full of subscriptions (or of access
requests) **whoever filled it** — a newcomer holding none is refused the same. One subscriber
cannot reach a total on its own, being held to 16.
**Subscriptions have storage of their own.** In the in-memory store they are admitted against a
service pool (`in_service_pool`, `InMemoryStoreLimits::max_pool_resource_count` and
`max_pool_bytes`, sized for the 1,024 the notification service admits) and take nothing from the
content ceilings (`max_resource_count`, `max_total_bytes`), so no number of subscriptions, from
any number of identities, can make a write of content answer 507. A full pool refuses a
subscription create (507) and nothing else. **Residual:** that 507 is an aggregate-occupancy
signal — a requester well under its own 16 learns from it that the pool is full, which is
something about how many subscriptions others hold. It is the occupancy of a bounded shared
resource, as the access-request total is, and it carries no subscriber, topic or inbox; it is
the signal a shared resource bound cannot avoid giving, kept to the subscription pool so it
never reaches a content write.
A type-search filter is at most 8 KiB and an
access grant 256 KiB; these limits apply while the body is read. Stored requests or subscriptions
past these bounds at startup (stored
before one was lowered, or written to the store directly) are all loaded and none is removed;
the start logs a line naming the container and what is past which bound, and no new one is
admitted to a bound until it is under it. A subscription create removes up to 16 expired
subscriptions **of its own subscriber's** before it is counted, so the purging it waits for is
its own records'. Everyone else's are removed by an expiry sweeper that runs every 2 seconds
(and when a create goes through) for as long as the server does, whether any create is admitted
or refused. Each sweep looks at most at 256 subscriptions, from where the last one stopped, and
starts at most 16 removals at once, in places of their own that request load cannot take; each
removal is given up after 2 minutes, longer than a healthy store's calls take (a degraded
store may still cut one short; one that never completes holds up no other), and one that fails, is given up or (in a build that
unwinds) panics is tried again after a wait that doubles from 10
seconds to 10 minutes; a removal whose delete the store leaves unknown is settled within that
same deadline, never handed on, so its place comes back with it. The sweeper stops, aborting
the removals under way, as soon as the server's last handle is dropped. Removal is eventual, subject to the store making
progress: removals contend with requests for the store and its index, and a store that never
answers delays them indefinitely. A start removes every expired subscription it loads.
**Expiry is decided by a stored time.** The notification service keeps a wall-clock high-water
mark (seconds since the epoch), and a subscription is expired once its `expires` is at or before
the later of the clock and that mark. Every expiry decision goes through one function that reads
the clock once: before anything outside sees a subscription expire (a read of it answered `404`,
a listing that leaves it out, its removal, a delivery stopped), a time at or past its expiry is
stored as the mark, so a start whose clock was stepped back still finds it expired. The mark is
one small record in the store's index alone (`Store::raise_marker`, under a fragment of the
subscriptions container that no request can name), raised in one step under the index's own
write lock, so a raise that lands late never lowers it; it needs no blob capacity, and its index
entry has a slot of its own, admitted however full the subscription pool is (the slot is kept for
one IRI, the first marker stored; any other IRI of that shape takes room in the pool). The mark
is only as durable as the backend's own persistence: the in-memory index holds it for the life of
the process, and the embedded index writes it to disk only when it saves a snapshot
(`EmbeddedSparqClient::save`), so a start from a snapshot taken before a raise finds the earlier
mark, and the expiries decided after that snapshot are not durable. A listing,
and the listing a conditional create compares, is read as one snapshot once its lock is held: the
subscriptions it weighs are read, the clock is read once, and the mark is raised before any is
left out. It is written only
when a delivery, a request or the expiry sweeper finds a subscription past its expiry by the clock
and not yet by the mark (at most once for each such second, as concurrent raises wait on one
another); the choice of which subscriptions an event goes to reads only the stored mark and
writes nothing. Notifications start only
over a store whose raise is atomic (the in-memory and embedded indexes); over any other,
`LwsState::new` fails before reading or writing anything. **When a raise fails, the service
halts:** no delivery to any subscription, every request to `/.lws/subscriptions/` answers `503`,
and no subscription is removed, until the expiry sweeper's retry stores the latest time any
decision needed (never an earlier clock). While halted, requests attempt no write of their own:
the sweeper alone retries, waiting longer after each failure (up to ten minutes), and logs at
the first failure and at each doubling of their count (`Notifier::high_water_failures` and
`Notifier::paused` report it). A start that finds no mark, or one that is not a time, while
subscriptions with an expiry are stored expires every one of them (their subscribers may
subscribe again), and stores a mark only once they are all removed (what the start could not
remove, the sweeper removes first); no subscription is created until a mark is stored. **On the
first start after upgrading to a release with this mark, and after a restore that drops
index-only records, every subscription with an expiry is removed this way and its subscriber
must subscribe again.** A mark ahead of the clock (one stored while the clock had jumped
forward) stands, fail-closed: subscriptions expiring before it are expired, and a new
subscription must expire after it. A start logs a warning when it is more than 24 hours ahead.
**To recover**, correct the clock and start once with `SOLID_SERVER_LWS_RESET_SUBSCRIPTION_CLOCK=1`
(then unset it), or, embedding the library, set `LwsConfig::reset_subscription_clock` for one
`LwsState::new`. The recovery runs inside construction, before any subscription is loaded and
before the state (or its store) is handed out, so no request is in flight: it removes every stored
subscription with an expiry (their subscribers resubscribe) and only then stores the clock as
the mark; construction fails, the mark unchanged, while any of them is still stored (start again
once it can be removed). An `LwsState` holds a lease (`store::Lifecycle`) for as long as it
lives, and the lease is per store handle: it is shared by every clone of `LwsState::store` and by
any `Tracked` wrapper of one (`Tracked::new` takes its inner store's lease and offers no other), so
a second `LwsState` built in the same process over a clone or a wrapper of a running one's store
is refused, and only the holder may lower the mark (`Store::reset_marker` takes the lease's guard,
which only this crate makes). It is not exclusion per backend: a store opened again separately
over the same backend, or wrapped by a `Store` implementation that does not forward
`Store::lifecycle`, has a lease of its own. A state that replaces another must be built after the
old one is dropped. Running a second process over the same store directory is unsupported, and
nothing at this release keeps one out. The mark changes only through `Store::raise_marker` and
`Store::reset_marker`: the generic mutators of the store (`write`, `create_in_container`,
`restore`, `hold`, `reinstate`, `delete`, `delete_container_if_empty`) refuse an IRI shaped like
the mark (`ServerError::Forbidden`). A create whose
purge of its own expired subscriptions halts the service answers `503` and creates nothing. Once
its listing locks are held, a subscription create takes the mark's raise lock and checks again
that the service is not halted and that its expiry is after both the clock and the stored mark
(`503`, `400`), holding that lock through its write and registration, so no raise fails between
the check and the `201`. A create whose outcome the store leaves unknown gives the lock up as it
answers so (`500`), before it is settled. Removal of an expired subscription is plain, retryable cleanup with
nothing written first: a removal that fails, is cut short or whose outcome is unknown leaves it
expired, and it is tried again. Expiry is decided again under the removal's locks, so a
subscriber's `DELETE` of a subscription that expires while it waits answers `404`, as one
already expired does. **Residual (accepted):** the halt and the time it owes are held in the
process alone, and a halt shows no expiry, so a subscription never durably expired (a stop
between the clock passing its expiry and the mark being stored, or a stop while halted after a
failed raise, or a create whose store outcome was unknown, answered `500`, and committed late)
may be delivered to again after a restart whose clock was stepped back before its expiry; nothing
outside saw it expire. A WebSocket channel's expiry is a latch held in memory: its receive token
expires by the process's monotonic clock, and once expired the channel never delivers again in
that process; none survives a restart. A
subscriber's conditional subscription create compares its own listing, and holds only that
listing (and the container, shared) while it is evaluated and until the new subscription is
registered (when the store leaves the create's outcome unknown, it holds that listing shared
until the outcome is settled, so the subscriber's other creates and removals go on meanwhile):
what it waits on is a change to its own subscriptions, the inline purge of its own expired ones
above, or the owner's conditional create (which holds the container alone, and once queued keeps
later shared holders waiting behind it, so then, for at most the lock wait, another
subscriber's change too), never another subscriber's create or removal otherwise, and never
the sweeper's removal of anyone else's expired subscription, which is listed nowhere (once seen
expired, a subscription stays expired, even if the clock is stepped back; a delivery checks its
subscription again once it has decided the subscriber may read the resource, with nothing waited
on between that check and the send). An access document nested too deeply to store and read back gets 400. A
request's Link headers may declare at most 128 relations holding
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
| `SOLID_SERVER_LWS_RESET_SUBSCRIPTION_CLOCK` | `1` runs the notification recovery once at start, before anything is served: every stored subscription with an expiry is removed, then the clock is stored as the notification high water (the start fails, the mark unchanged, while one is left). For a mark stored while the clock had jumped forward; correct the clock first and unset it afterwards. |
| `SOLID_SERVER_LWS_ALLOW_INSECURE_FETCH` | `1` lets the server fetch and deliver to `http:`, loopback and private addresses (CID documents, OIDC issuers, webhook inboxes). Test-suite use only. |
| `SOLID_SERVER_LWS_AUTHN` | Comma-separated authentication suites to offer, of `ssi` (did:key and controlled identifiers), `openid` (OpenID Connect) and `saml` (SAML 2.0). Unset, every suite built in is offered (SAML when `SOLID_SERVER_LWS_SAML_IDPS_FILE` is set). A name that is no suite or that the build does not hold, `saml` without its identity providers, and a suite's own variable set while that suite is left out each stop the start. |
| `SOLID_SERVER_LWS_SAML_IDPS_FILE` | A JSON object of trusted SAML identity provider entity ids to their signing keys (PEM certificate, PEM public key, or public JWK); offers the SAML 2.0 suite. Needs a build with `--features authn-saml`; set without it, or with `SOLID_SERVER_LWS_AUTHN` leaving `saml` out, the server refuses to start. A key that cannot be read also stops the start. |
| `SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS` | Comma-separated OpenID Provider issuers (exactly as their ID Tokens' `iss`, which must equal the discovery document's `issuer`) whose identities get the reserved half of the DPoP replay cache. Every other provider shares the other half, each provider and each identity held to its own share, so open registration elsewhere cannot lock out these providers' users. Needs the `authn-openid` feature (on by default); set without it, or with `SOLID_SERVER_LWS_AUTHN` leaving `openid` out, the server refuses to start. |

What the server exposes, all discoverable from the storage description
(`GET /` with `Accept: application/lws+cid`):

- **Storage**: `application/lws+json` containers with paging links, data resources with
  conditional requests and single byte ranges, `POST` with `Slug`, `PUT`, `PATCH` of a JSON
  data resource with RFC 6902 JSON Patch (`application/json-patch+json`), `DELETE` (with
  `Depth: infinity` for non-empty containers), and RFC 9264 linksets at `{resource}.meta`
  (a linkset has none of its own: `{resource}.meta.meta` is `404`, or the challenge),
  which take `PATCH` with JSON Patch and `PUT` of a whole `application/linkset+json` document
  (both advertised in `Allow`, the patch format in `Accept-Patch`; a failed precondition gets
  `412`, another content type on a `PUT` `415`). A linkset `PUT` replaces every user-managed link
  in one write and is checked as a `PATCH` result is (its `204` carries no `ETag`, since what is
  stored is the document resolved and with the server-managed links; a `GET` reads the tag); its body is at most what the metadata may
  hold (`413` past it, unread), and before it is parsed a scan that keeps nothing charges it as
  the patch is charged and refuses it (`413`) past what keeps parsing it (the same reader again,
  keeping what it reads) within the linkset cost budget beyond the body itself; a number in it
  gets `422`, text that is not JSON (or nests past what the parser reads) `400`, a partial `PUT` (`Content-Range`) `400`. A linkset `PATCH` or `PUT` changes only user-managed links: the
  server-managed `up`, `type`, `self` and `linkset` relations of the resource keep the server's
  values, and a result that is not a linkset document gets `422`. Its relative `anchor` and
  `href` values are resolved against the linkset's own URI (RFC 9264 section 4) and stored
  absolute; one that is not a URI reference gets `422`, as does a target attribute not shaped as
  RFC 9264 section 4.2.4 says (`title`, `type`, `media` strings; `hreflang` and extension
  attributes arrays of strings; `name*` arrays of `{value, language}`). A linkset `PATCH` body
  is at most 8,192 bytes (`413` past it, before it is parsed): reading a JSON Patch holds at
  most a measured, doubled 256 bytes per byte of text whatever its shape, so the parsed patch
  stays within 2 MiB. The patch is applied with every value charged what it allocates (64 bytes
  each, plus 704 per object and 160 per member, since an object allocates a whole map node),
  within 2 MiB, checked before each value is copied. Before anything is resolved the result is
  held to 256 entries, 1024 targets and 4096 bytes per anchor, relation and `href` (resolved
  ones included), and its worst-case cost (every value it holds, its size twice, and each
  relative reference grown by the base) to 2 MiB; past any of these, `413`. A document with
  members besides `linkset` gets `422`. A `Link` header on a create or replace is held to the
  same target and length caps (`431` past them). Every write that stores a linkset (a create, a
  replace, a linkset `PATCH` or `PUT`) checks the document it would serve, with the server-managed links
  put back and the representation's size at its longest, against exactly what a linkset `PATCH`
  checks its target with (those caps, that cost, and the metadata's size limit), and gets `413`
  past it, so whatever is stored can be patched. A linkset stored before these checks that
  passes them takes only `remove` and `test` operations (others get `413`) until it is within
  them again. Metadata that nests too deeply to store gets
  `422`. The links the type index matches are the stored linkset's own. A
  `Link` header's `rel="anchor"` is never taken as a link (`anchor` names an entry's context).
  Errors are `application/problem+json`. Bodies are stored as sent, so a body with a
  `Content-Encoding` other than `identity` gets `415`. A precondition header sent as several
  lines counts every line; an entity-tag list that cannot be read by the RFC 9110 grammar gets
  `412` (a comma inside a quoted tag is part of the tag; an empty list names no tag), and a date that is not one valid
  HTTP-date is ignored. A `POST` whose name is taken (or is being created,
  written or deleted right now) gets a numbered name and then a random suffix; when every try is
  taken it gets `409`. A JSON Patch is applied whole or not at all: one that is not a valid
  patch document (an operation naming a member twice included) gets `400`, one that cannot be
  applied (a missing path, a failed `test`) gets `422`, another patch format or a resource not
  stored as JSON (`application/json` or a `+json` type, which it keeps) gets `415` with
  `Accept-Patch`, checked before the preconditions, and a result larger than the body limit, more
  than 1,000 operations, or work past four times the body limit gets `413`. Numbers are never
  read as binary floats: each is kept as its text, so one a patch does not touch is written back
  as it was (members are written in name order), and `test` compares numbers as the decimal
  values their texts denote (zero is zero whatever its sign or exponent). A number longer than
  1,024 characters, or whose power of ten does not fit 64 bits, in the patch or the resource, gets
  `422`; a resource whose text is not JSON as written gets `415`. Sizes are those of the JSON as
  written, and a `test` is charged for what it reads of both sides. A patch reads what it patches (whether
  even an `add` of one member applies depends on the document's shape, format and size), so a
  `PATCH`, of a resource or of its linkset, needs Read as well as Modify, and without it is
  refused (`401`/`403`) before anything about the target answers it. A `POST` whose request is cancelled after the member is written is
  still announced; a `DELETE` that fails, and is being put back, announces nothing. `livez` and
  `readyz` are never
  given to a member of the root container, because the probes answer those paths. Stored metadata that cannot be read
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
  spelling. The identity document may be Turtle (asked for first) or compact JSON about the
  subject whose only contexts are the CID, DID or LWS ones, read by its keys (the Solid issuer
  by its full IRI, as a node reference `{"id": …}`). Any other JSON-LD form (expanded,
  flattened, a graph, an inline or nested context) is not expanded and names no issuer. A token time
  (`exp`, `iat`, `nbf`) may carry a fraction (RFC 7519 NumericDate); one that is not a number
  between 1970 and 9999 is refused.
  A SAML 2.0 assertion (`urn:ietf:params:oauth:token-type:saml2`, base64url) is taken from the
  identity providers in `SOLID_SERVER_LWS_SAML_IDPS_FILE`: it needs an enveloped XML-DSig
  signature over the whole assertion (exclusive canonicalization, SHA-2, RSA or ECDSA
  SHA-256) under the issuer's key, a validity period around now, an audience restriction
  naming this server, an http(s) URL `NameID` (the agent) and at least one satisfied bearer
  confirmation, whose `Recipient` is the client. Subject confirmations are alternatives (SAML
  core 2.4.1): unsatisfied ones (expired, holder-of-key, ...) are ignored, the satisfied ones
  must name one client, and the assertion holds until the latest of them ends.
  The token endpoint checks the subject token's expiry once more just before it issues, so
  a token that expires while a suite fetches documents is refused.
  Each authentication suite lives in its own package (`sparq-lws-authn-*`, each an `authn-*`
  feature of `sparq-lws-core`) and implements `sparq_lws_authn::Suite`: did:key and controlled
  identifiers are `sparq-lws-authn-ssi` (`authn-ssi`, on by default), OpenID Connect is
  `sparq-lws-authn-openid` (`authn-openid`, on by default), SAML is `sparq-lws-authn-saml`
  (`authn-saml`). `LwsConfig::register_suite` offers one (never for a token type an offered
  suite takes), `LwsConfig::suites` lists them, and the metadata lists their token and
  identifier types. `SOLID_SERVER_LWS_AUTHN` picks among the suites built in.
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
  order). The owner and a resource's creator are always allowed. Grants and access requests
  are created and removed in one store step each, and memory follows the store: what the
  container lists is what a start puts in force. A create takes the container (waiting at most
  two seconds, then `503` with `Retry-After`) and the record's own lock, refuses a name already
  stored, and writes the record. A write that reports a failure is checked against the listing:
  listed, the create answers `201` and the record is in force; not listed, `500`; not known
  (the listing cannot be read), `500` "not known" with the request open to retry, and the
  record is put in force once the listing shows it. A write whose outcome can still change
  (over the `http` backend, an update that timed out or failed in transit, which `sparq-server`
  may still apply) is fenced first: the client writes a fence that the update, if it has not
  been applied yet, finds and then does nothing. Only a listing read after the fence counts
  as "not listed"; until the fence lands, the create answers `500` "not known" and keeps the
  record's lock and quota place, and the record is put in force if it landed. Until it is
  settled, the record's narrowest listing stays held, shared: a subscriber's own listing for
  a subscription, the container's for a grant, a request or an owner's subscription. So a
  conditional create evaluated on that listing waits for the outcome (`503` past the lock
  wait), and two with one entity tag cannot both register. A fence is
  one triple, kept for good in the index's reserved `urn:pss:index#fences` graph (the update it
  fences may still arrive at any time): the graph grows by one triple per write that failed this
  way. At most 256 record changes are in progress or being settled at once; past that, a new
  create or removal answers `503` with `Retry-After`. Residual: requests and the settling of
  outcomes the store left unknown share those 256 places, so while many outcomes wait to be
  settled (a store that keeps failing), any caller's create or removal can get that `503`. The tasks that settle them belong to the
  server's state and end when it is dropped, leaving the record as the store has it for the next
  start to load. A record is announced (`Create`) as it is
  put in force, so one confirmed only after its create answered is announced then. A revocation (or a request's withdrawal)
  takes the record's lock and the container's the same way (at most two seconds, then `503`),
  and takes the grant out of force while the removal runs, and keeps it out only once the removal
  is confirmed (it succeeded, or the store, read again, no longer lists the record): `204` once
  revoked; `500` "still stored" when the record is still listed (the grant back in force); `500` "not known" when it
  cannot be read again, with the grant left in force. A removal whose outcome can still change
  is fenced the same way: until its fence lands, the answer is `500` "not known yet", the grant
  stays out of force, and it is removed or put back once the listing read after the fence
  says which. A request waits for its fence at most as long as for a lock (two seconds, shared
  between the fences it writes), then answers so and leaves the fence to the background; the
  container stays held until that background settling is done. A removal the store refuses for
  want of room answers `507`, the record still stored and back in force. A subscription
  being cancelled hears of no change from the moment its removal begins until it is back in
  force. A record holds its place in the request
  quota until the store confirms it is gone: while its create's outcome is not known and while
  its withdrawal runs. Nothing a start puts back in the background begins until every step of
  the start that can fail has succeeded. A stored grant or request that cannot be read stops
  the start (a later start reads it again); one listed but gone is skipped; one whose stored
  body is not a record is quarantined at every start until the owner deletes it: it grants
  nothing, is listed and counts against its quota, and answers `500` to a read.
  A new request notifies the owner's inbox when the owner's JSON(-LD) identity document
  names one. A new grant notifies the inboxes of the requests its assignees made themselves,
  as well as its own `inbox`.
- **Webhook notifications** under `/.lws/subscriptions/`, signed per RFC 9421 with the
  key in the storage description's `verificationMethod`. The signature covers `@method`,
  `@scheme`, `@authority`, `@path`, `@query` (so a notification for an inbox with a query is not
  valid for another query), `content-type` and `content-digest`. Deliveries go through a bounded queue and worker pool (see the
  `SOLID_SERVER_LWS_DELIVERY_*` variables); a Delete is announced only once the removal happened.
  Each delivery attempt, retries included, first checks that its subscription still exists and
  has not expired, and that the subscriber may still read the resource, reading the resource under
  its shared lock, as a GET does. A Delete is checked against
  the resource as it was before removal; when that could not be read, only the owner (and
  everyone in open mode) hears of the Delete. A delivery that fails a check is dropped. A subscription
  to a resource's linkset (`{resource}.meta`) hears of each change to the resource as of the
  linkset, and is authorized as the resource when it is created. A delivery never waits on a
  resource that is set aside, even one set aside while it waits, and checks its subscription
  again once it holds the lock. An Update is announced by the task that runs the write, so a
  client that goes away mid-write does not keep subscribers from hearing of it. Who hears of a
  change is decided while the write's locks are still held, so a change that follows cannot hide
  it. Creating or removing an access grant, an access request or a subscription is announced to
  subscriptions to its own service container (not to one to the storage root): the owner and the
  record's author may hear of it. A stored subscription that cannot be read stops the start; one
  whose stored body is not a subscription is quarantined as access records are: listed to the
  owner, counted against both totals, `500` to a read, removed by the owner's `DELETE` as any
  record is. A subscription is created and cancelled as an access record is, by the same code.
  Each topic of a new subscription is checked before its lock is taken: one the subscriber may
  not read is refused (`403`, or `401` anonymous) at once, whatever its state. Access is decided
  before the store is asked whether the topic exists, so that refusal, and the work done before
  it, are the same whether it does. One it may read
  is checked again under its lock, waited for at most two seconds (then `503`), or answered `503`
  at once while set aside. The bound on deliveries prepared for one change counts only authorized
  ones: a subscriber that may not read the resource takes no place from one that may. Once it is
  reached the subscriptions left are neither authorized nor notified, and are logged and counted
  apart (`Notifier::unprepared`) from dropped deliveries (`Notifier::dropped`), each of which was
  owed. Both are counted in memory from the start, and only for a change that happened (a delete
  that fails counts nothing). The authorization for one change has no budget of its own (one
  party's subscriptions that may not read the resource could spend it before an authorized one is
  reached): the subscription totals cap it at one check per subscription in force (at most
  1,024). That caps the number of checks, not their cost: each reads from one snapshot of the
  resource's access taken for the change, so its metadata is read and the grants are scanned
  once per change. Taking a place in the delivery queue, parsing each inbox and running each
  delivery leave the request's task before any of them is queued, on a task of their own
  (`Notifier::send`), so a delivery target that never answers holds up neither that change nor
  the next, and the request does not do the per-delivery work of however many subscriptions the
  change has. When the runtime runs that task is the runtime's business: the deliveries are off
  the request's task, which is not a promise that they happen after the response reaches the
  client. Who hears of the change is still decided on the request's task, while its locks are
  held.
  A recursive delete too large to put back is refused before any of its
  notifications is prepared.
- Writes and deletes are **whole or not at all**: a PUT that changes metadata, a linkset PATCH or PUT and a `DELETE`
  (a whole `Depth: infinity` subtree included) record what each store step replaced and put it
  all back when a later step fails, so content, metadata, listings and validators (`ETag`,
  `Last-Modified`) are as they were. A recursive delete too large to put back is refused with
  `409` before anything is removed. Nothing is copied to put a resource back: the store holds
  the bytes its record points at (a hold, itself stored, so a stop leaves nothing unaccounted
  for) and the record is put back pointing at them, validators included. Each change's holds are
  its own (two changes holding the same bytes each release only theirs). Bytes a record leaves are
  reclaimed in two steps through the index, whichever server instance acts: they are first named
  as being reclaimed, in one step that also finds no record naming them (a resource's, or a
  hold's), so none can come to name them again; only then are they deleted (a reclaim a stop cut
  short is finished at the next start). That step relies on the index applying updates one at a
  time: the embedded store or sparq-server; another SPARQL endpoint is not supported behind a
  storage that holds bytes. The holds go once the change is kept or put back, and a start
  releases any of this storage's holds whose change's intent is gone or says it was kept. So no delete is
  refused for the size of what is stored, and none answers `507` for want of room to copy it.
  Residuals: what bounds a recursive delete (`409`) is how many resources it removes and the size
  of their metadata (types, links and linkset, which it reads to decide who may act on them while
  the delete is in flight), never their content; hold records (a few fixed fields per resource a
  change touches) are not counted against the storage's quota, but intents are (each carries the
  metadata records it would put back), so in a storage at its ceiling a change that needs one is
  refused with `507`; a store that fails may fail the change (`5xx`, below) whatever was asked;
  and a hold whose release fails keeps its bytes until the maintenance sweeper releases it again
  (or, the process stopping first, the sweeper's bounded wait list being full, which is counted
  and reported, or the hold being one whose taking failed part way, the next start); a start fails, rather than leave a hold to nobody, when the
  holds cannot be listed, a holder's intent cannot be looked up, or a release that fails cannot
  wait for the sweeper (each asked a few times first); and a response's status, timing and quota
  outcome are independent of the content of the resources a change touches, not of their
  metadata, of the room others leave in the shared quota, or of the backends' and the index's
  own work. A start
  settles every intent of its storage and releases holds as if no other process served it: one
  storage is served by one process at a time. When putting back fails too, it is tried a few more times with the
  resource's locks held; if it still fails the request ends with `5xx` and the resources
  (with the containers whose listings they change) are **set aside**: answered `503` (`Retry-After`) at once, to a caller that may act on them, while a background task, holding their
  locks and no request's admission slot, keeps putting the change back, so no other request sees
  the half-done state in between; a recursive delete sets aside everything under it it had not
  reached too. A request already waiting on a lock when its resource is set aside is answered
  `503` then, not left waiting. A resource stays set aside until every stuck change to it is
  put back. While set-aside changes hold 256 MiB or more to put back, new writes are answered
  `503` too (reads go on). Every route but the type index and search authorizes a request
  before it says anything of this:
  a caller that may not act on the resource (or record) gets the same `401`/`403`/`404` it gets
  when nothing is set aside. Whether it may is decided on what the resource was before the stuck
  change began (whether it was there, its metadata and its format, as the change's journal
  staged them before its first step), never on what the change wrote meanwhile: access the
  change would newly grant is not met, and access it would remove still is. When that cannot
  be read, only the owner hears of the state. A new subscription is told its container is set
  aside only once every topic is authorized. The same holds from a journaled change's first step
  until it is done or set aside (while it is part way, or being put back under its locks): a
  request, or a subscription's topic, that comes without the lock is decided on what the resource
  was before the change began, so a caller only the change's own new metadata would authorize is
  refused at once, and only one the earlier state authorizes waits on the lock (the check that
  counts is made again under it). A check without the lock is made again whenever a change to its
  resource began or ended while it read (a reply can come back after the change is over), and
  refuses if that keeps happening. A write's container is touched by the task that runs its store
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
  on a stale date. A listing's `Last-Modified` is the latest of the container's own time and
  every member's, so a change whose touch a stop lost still moves it on; a member whose time the
  backend does not record is listed without `modified`, and the listing then has no
  `Last-Modified`; so has one whose container's own time is not recorded. An unknown time is never
  stated as the epoch: a linkset's `self` entry then omits `modified`. Every change to the store (a request's own steps, a touch, a rollback or a
  retried undo) goes through one tracked path, and a listing made while a change to the
  container or a member is in flight has no `Last-Modified`; one that overlapped a change is
  made again. A PUT with `Content-Range`
  (a partial PUT) is refused with `400`.
- A `Link` target that is not a URI reference is refused with `400`; entity tags in
  `If-Match`/`If-None-Match` compare byte for byte, obs-text included, and an empty list matches
  nothing. A write whose precondition compares the target's state (a tag in `If-Match` or
  `If-None-Match`, or an `If-Unmodified-Since` the write evaluates) needs Read on the target too,
  and on a conditional `POST` Read on the container whose listing it compares (an access request's
  listing is the owner's alone): a writer without it is refused (`401`/`403`) whatever the
  outcome would have been. A bare `*` asks only whether the target exists and needs no Read: a writer learns
  that much from any write's response (`201` or `204`, a `DELETE`'s `404`). The Read check comes
  first, before the target's existence, format or metadata decide anything, and an
  `If-Unmodified-Since` counts as a comparison on every route that evaluates dates, whether or not
  the target has a recorded time. A
  precondition that compares no state (`*`, or a field the write ignores or cannot read) is
  evaluated on the target's existence alone, without its listing, so neither a page past the last
  nor an `Accept` the listing cannot serve refuses the write. A conditional `POST` to the storage
  root compares its public storage description, which needs no Read. A data `PUT`'s, or any
  `PATCH`'s, `204` carries the new `ETag` (of the resource, or of its linkset) only to a writer
  that may read the result; a linkset `PUT`'s `204` carries none. A write whose outcome would
  follow from what is stored needs Read as well as Modify (or Delete): a `PUT` replacing a data
  resource (it keeps the stored links and types, which decide whether it is admitted), a whole
  linkset `PUT`, every `PATCH`, and a container `DELETE`. A `Slug`
  names a new member only for a writer that may read the container; anyone else's member gets a
  name the server chooses, so the name says nothing about which members are there. A
  subscriber's subscription listing, and its `ETag`, change only with its own subscriptions, and
  are read through a per-subscriber index, so neither what it is shown nor the work taken to
  show it grows with how many subscriptions anyone else holds.
- A resource's types come from two places, kept apart: `Link: <…>; rel="type"` headers and
  `<> a <…>` statements in Turtle content. A PUT replaces the content-stated types, and replaces
  the header-declared types only if it sends `rel="type"` headers of its own. Other Link
  relations change only with `Prefer: set-linkset`.
- Link relations, registered and extension (URI) alike, compare case-insensitively (RFC 8288
  section 2.1.2) wherever they are keyed, matched or searched: two spellings of one relation in
  a request's Link headers or a linkset are one relation, stored and served under the first
  spelling met with the targets of both, and a type search names it in any case.
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
  Filter IRIs must be valid absolute IRIs (RFC 3987), or the filter gets `400`. A filter is at
  most 8 KiB, so that its page links fit a request head: a longer well-formed one (up to 32
  KiB) is too complex to answer (`422`), as one with too many groups is; a QUERY body past 32 KiB
  is refused as it is read, and a page link carrying a filter past 8 KiB, which this server never
  makes, gets `413`. Whether the caller may read a resource is decided before anything else
  is asked of it: one it may not read is passed over as one that is not there (only a
  container's listing is read, for what under it may be readable), so neither the answer nor
  the walk turns on whether it is there. When any listing, permission check or metadata read
  fails, the whole index fails with one generic `500` that names no resource; a resource whose
  access cannot be decided is passed over unless a grant names the caller for it. Names of
  linkset records a listing holds are not followed and not charged to the walk's budget. Both carry an `ETag` of what they serve and
  answer `If-Match` / `If-None-Match` as a read does; a coded QUERY body gets `415`.
  While walking the storage they keep only what they return (a search its matches, the index
  its distinct types) and the walk's own state (the listing in hand and the members still to
  visit), at most 16 MiB in all; past that the request gets `507`. Each resource, and each
  container's listing, is read under that resource's lock, so a change in flight (a recursive
  delete part way) is waited out, never read half done. When the walk reaches a resource a
  stuck change is to (set aside), or anything is set aside while it walks (what it already
  visited included), the request gets `503` (`Retry-After`) at once, rather than waiting on
  it or answering an index that leaves out what is under it.

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
