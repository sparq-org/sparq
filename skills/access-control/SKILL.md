---
name: access-control
description: "Graph-level Solid-style access control over a sparq RDF dataset with the opt-in sparq-solid crate: store pods as named-graph-per-document, keep WAC (.acl) / ACP (.acr) documents as queryable triples, materialize their semantics to a queryable authorization view (urn:sparq:auth) via N3 rules, then filter every SPARQL query/update per (WebID, client, issuer) session to the authorized graph set — fail-closed. Use when gating which named graphs a session may read/append/write, mapping WAC/ACP to an allow-deny model, or querying the materialized auth view. RESEARCH/architecture track — this is the authorization LAYER, NOT a production Solid Pod HTTP server, and it does NOT authenticate (the caller asserts the WebID, client, and issuer)."
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq-solid — graph-level WAC/ACP access control

`sparq-solid` is an **opt-in** crate that enforces Solid-style access control over a
sparq RDF dataset at the **named-graph** granularity. A pod is stored as
**one named graph per document**; the WAC (`.acl`) and ACP (`.acr`) access-control
documents stay as plain queryable triples; their semantics are encoded as **N3 rules**
(run by `sparq-reason`) that **materialize** a queryable authorization view in the
reserved `<urn:sparq:auth>` graph. Every query/update is then filtered per
`(WebID, client, issuer)` session to the named graphs that session may access —
**fail-closed**, with zero Solid-specific code in the engine itself (the crate is a
dependency of nothing in the workspace).

## What this is — and is NOT (read first)

This is an **architecture / research-track** layer, not a deployable Solid server.

- It is the **authorization** half only. It answers *"may this `(WebID, client, issuer)`
  session read/append/write named graph G?"* by materializing WAC/ACP into a queryable
  view and filtering the dataset. It does **NOT authenticate**: a `Session.agent` is a
  caller-asserted WebID string — there is **no WebID-OIDC / DPoP / token verification**.
  The relying application is responsible for authenticating the WebID before it hands one
  to `query_as`. Treat a `Session` as a trusted claim, not a verified one.
- The `Session.issuer` field (ACP `acp:issuer`) does **NOT** add WebID-OIDC
  authentication. It is one more **caller-asserted** matcher dimension: the caller states
  *which OIDC issuer it claims vouched for the WebID*, and an ACP `acp:issuer` matcher
  decides whether that asserted issuer is acceptable. sparq-solid never contacts an IdP,
  validates an ID token, or verifies the issuer binding — it only string-matches the
  asserted issuer against the grant. As with `agent`, the relying application must verify
  the issuer↔WebID binding before asserting it. WAC has no issuer notion, so a WAC pod
  ignores the field entirely.
- It is **NOT a Solid Pod HTTP server.** There is no HTTP resource protocol, no LDP
  container CRUD, no `Link rel=acl` discovery over the wire, no Solid notifications. The
  document→named-graph naming is a storage convention (design §2.2), not an HTTP server.
- The honest support matrix, threat model, security boundaries, and measured baseline
  live in the design record
  ([`research/solid-access-control-design.md`](../../research/solid-access-control-design.md));
  the README's support matrix and `cargo doc -p sparq-solid` are the user docs. Do not
  present this crate as a production Solid implementation.

## Quickstart

`crates/sparq-solid/Cargo.toml` (consumes `sparq-core` / `sparq-engine` / `sparq-reason`;
no cargo features of its own):

```toml
[dependencies]
sparq-core  = { path = "../sparq-core" }
sparq-solid = { path = "../sparq-solid" }
```

The same query, different sessions, different results — fail-closed:

```rust
use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session, wac_fixture};

# fn main() -> Result<(), String> {
// A pod is a dataset, one named graph per document (here the bundled fixture).
let graph = Graph::load_dataset(&wac_fixture(), "nquads")?;
let mut store = PodStore::new(graph);
store.materialize_wac()?;                 // run the N3 rules → install <urn:sparq:auth>

let q = "SELECT ?title WHERE { ?s <https://ex.dev/ns#title> ?title }";

// WebID / client / issuer are all CALLER-ASSERTED — sparq-solid does NOT authenticate them.
// `now` is the request clock (xsd:dateTime), used only by time-windowed conditional grants.
let alice =
    Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
let authorized = store.query_as(&alice, Mode::Read, q)?;            // alice's authorized graphs
let public_only = store.query_as(&Session::default(), Mode::Read, q)?; // anonymous: public graphs only
let _ = (authorized.rows.len(), public_only.rows.len());
# Ok(()) }
```

## Public API

Materialize the authorization view from the access-control documents, then enforce:

- `PodStore::new(graph) -> PodStore` — wrap a loaded dataset. Before the first
  `materialize_*` call **every** session (including the owner) sees nothing.
- `store.materialize_wac()` / `store.materialize_acp() -> Result<MaterializeStats, _>` —
  run the N3 rules to (re)install `<urn:sparq:auth>`. ACP uses one three-stratum
  in-memory reasoning run; `MaterializeStats::strata_facts` reports its term-level closure
  size after each stratum, before RDF list expansion and interning. On `wasm32-unknown-unknown` the
  informational `MaterializeStats::millis` is reported as `0.0` (no `std::time::Instant`);
  the auth view is identical ([OPUS-4.8] sq-7agop).
- `store.materialize_acp_with(&AccessProvenance) -> Result<MaterializeStats, _>` —
  ([OPUS-4.8] sq-3jtd.5) ACP materialization that ALSO resolves `acp:CreatorAgent` /
  `acp:OwnerAgent` matchers against per-resource creator/owner WebIDs supplied by the
  TRUSTED caller. `AccessProvenance::set_creator(resource, webid)` /
  `set_owner(resource, webid)` build the map; the loader synthesizes
  `<r> solidx:creator|owner <w>` facts from THAT map ONLY. The loader hard-rejects any
  control-document triple whose predicate is in the derivation-internal `solidx:` namespace
  (§2.4), so a writer who embeds `<r> solidx:creator <self>` in a content document OR in the
  `.acr` they control cannot self-grant (this also blocks forged `solidx:appliesToResource`
  policy redirection). Each grant is RESOURCE-SCOPED (the creator of `R1` is never
  granted `R2`) and composes with the matcher's own `acp:client`/`acp:issuer` constraints.
  `materialize_acp()` is `materialize_acp_with(&AccessProvenance::new())` — no provenance ⇒
  no `CreatorAgent`/`OwnerAgent` grant (fail-closed).
- `store.materialize_acp_with_credentials(&AccessProvenance, &VerifiedCredentials)` —
  ([SONNET-4.6] sq-ysv3u, issue #2935) the same, ALSO resolving **`acp:vc`** matchers.
  `VerifiedCredentials::hold(agent_webid, requirement_iri)` asserts that the TRUSTED caller
  has **already verified** a credential proving `agent` satisfies the requirement; the loader
  synthesizes `<agent> solidx:holdsVc <requirement>` from THAT map only, so the same `solidx:`
  reserved-predicate guard blocks a forged holding in an `.acr`. Requirements are matched by
  **exact IRI** (structured claim comparison such as `age >= 18` is deliberately not
  interpreted in the rules — the requirement IRI names the whole predicate and the verifier
  decides). Within one matcher, `acp:vc` values are disjunctive and combine **conjunctively**
  with `acp:agent`/`acp:client`/`acp:issuer`. `materialize_acp_with(&prov)` is
  `materialize_acp_with_credentials(&prov, &VerifiedCredentials::new())` — no credential ⇒
  **no `acp:vc` matcher accepts anybody** (fail-closed; and note this corrected an over-grant:
  before sq-ysv3u `acp:vc` was an unrecognized attribute, so such a matcher looked
  agent-unconstrained and granted `auth:Public`, i.e. everyone including anonymous).
  Opt-in `acp-vc` feature — the **trust-the-issuer** backend that populates the map:
  `VcRequirement::new(requirement, issuer_did, credential_type).with_claim(pred, obj)` plus
  `VerifiedCredentials::admit_data_integrity(credential_triples, proof, resolver, reqs)`,
  which verifies a W3C Data Integrity `eddsa-rdfc-2022` proof via `sparq-vc` and records the
  requirements the credential satisfies for its `credentialSubject` — refused
  (`VcAdmitError::AmbiguousSubject`, nothing recorded) unless the graph names exactly ONE
  distinct subject, since the proof binds to the RDFC-1.0 CANONICAL form and a two-subject
  credential would otherwise bind its holding to whichever subject was presented first.
  **Honest scope:**
  authenticity + integrity + non-repudiation ONLY — it reveals the whole credential to the
  verifier and makes **no** privacy, unlinkability, zero-knowledge or selective-disclosure
  claim; revocation and expiry are not checked (a holding lasts until re-materialization).
  The zero-knowledge backend (prove an attribute without revealing the credential) is NOT
  wired: it needs the ZK estate, whose external accredited-cryptographer audit is PENDING
  (`sq-qhy4`).
- `store.query_as(&Session, Mode, sparql)` → `QueryResult` (`.rows`);
  `store.query_json_as(...)` → JSON string; `store.ask_as(...)` → `bool`. These evaluate
  through the engine's **zero-copy `DatasetView`** filtered to the session's authorized
  graphs (the default, fast path).
- **Empty default graph + union-default-graph opt-in — SPEC-COMPLIANT BY DEFAULT**
  ([OPUS-4.8] sq-gq28y, issue #1546; maintainer decision "spec compliant - empty by default").
  The read path (`query_as`/`query_json_as`/`ask_as`) implements the *Access-Controlled SPARQL
  Query over a Solid Pod* [Editor's Draft](https://github.com/jeswr/solid-sparql-query)
  empty-default + explicit-union semantics **out of the box**. The standing default graph is
  **empty** (a bare `{ ?s ?p ?o }` matches nothing) UNLESS the query opts in with
  `FROM <http://www.w3.org/ns/solid/sparql#union-default-graph>` (the spec-minted
  `UNION_DEFAULT_GRAPH_IRI`), which makes the default graph the union of the authorized named
  graphs **for that request only** (a pure function of the query's dataset clause — no
  cross-request state leak). An explicit `GRAPH ?g` / `GRAPH <g>` pattern is unaffected either
  way. Semantics + honest scope:
  - the reserved IRI is a **signal, never a graph name**: it is stripped from BOTH `FROM`
    and `FROM NAMED` positions before evaluation and never binds `GRAPH ?g`. In `FROM NAMED`
    position it is treated as **absent** (stripped, not intersected) so `GRAPH ?g` keeps
    ranging over the full authorized set (draft: "when only the reserved IRI is given, the
    named-graph set remains the authorized set");
  - **fail-closed, exact-match**: a near-miss IRI is a normal (absent, per-pod-model) dataset
    reference that contributes nothing and never widens — it does NOT silently enable the
    union default graph. At this string API there is no "malformed opt-in value" (the signal
    is one exact IRI, present-or-absent); the `default-graph-uri` **protocol-parameter**
    equivalent and its fail-closed validation are the Solid HTTP layer's responsibility
    (`solid-server-rs`), not sparq;
  - emulated at the **query layer** (the opt-in triggers the existing per-pattern `GRAPH`
    wrap; the engine view keeps its `Empty` default graph — no `UnionOfVisible` engine mode
    is added, design record §5.4(a)). Default public surface: `UNION_DEFAULT_GRAPH_IRI`
    (const) + `wrap_for_view_opt_in(sparql) -> Result<String, _>` (the spec-conformant
    read-path rewrite, the differential oracle for the enforcement path).
  - **security invariant (preserved by the flip):** empty-by-default is strictly MORE
    conservative than union-always — it can only ever REMOVE a bare pattern's default-graph
    solutions, never add graphs. Both variants evaluate over exactly the session's authorized
    named-graph set (the view enforces that at the graph-name layer before evaluation); the
    default-graph choice changes what the *default graph* sees, never *which* graphs are
    readable, so it cannot widen the readable set.
  - **legacy escape hatch:** the opt-in `legacy-union-default-graph` feature (OFF by default)
    reinstates the pre-flip union-always default (a bare pattern ranges over the authorized
    union) for downstream deployments that relied on that idiom and need to migrate on their
    own schedule; new code should use `GRAPH ?g` or the spec-minted opt-in.
  - **conformance:** the query-semantics conformance class of the spec is vendored + run in
    `crates/sparq-solid/tests/conformance_solid_sparql_query.rs` (upstream suite in
    `jeswr/solid-sparql-query:test-suite/query-semantics/`) and passes all 15 cases.
- `store.query_as_rewrite(&Session, Mode, sparql)` — the v1 **`FROM NAMED` rewrite**
  portability path: enforces the same policy on any standard SPARQL 1.1 engine (one
  deliberate semantic difference noted in-source: a caller `FROM <g>` can only restrict
  the view, never widen it). **NB the spec empty-default flip applies to the view path
  (`query_as`) only** — this v1 portability path keeps its union-always default-graph
  emulation (it predates the draft and is a standard-SPARQL-portability oracle, not the
  spec-conformant surface).
- `store.update_as(&Session, sparql)` / `store.update_as_acp(...)` — **write-path
  gating**: check every graph an update could mutate *before* applying, and
  auto-re-materialize on `.acl`/`.acr` writes.
- `store.update_as_with_budget(&Session, sparql, &QueryBudget)` /
  `store.update_as_acp_with_budget(...)` — the same write path under a cooperative
  `QueryBudget`, for a caller obliged to bound **every SPARQL evaluation** it issues (an
  agent tool surface, an HTTP handler). [FABLE-5] sq-yhlf0. The budget reaches both places an
  update evaluates SPARQL — the authorization check's `GRAPH ?var` binding SELECT (an
  exhausted budget there is a **deny**, nothing mutated) and the apply's
  `DELETE`/`INSERT … WHERE`. It does **not** bound the remaining operations, and a
  request-size cap does not cover all of them: `INSERT`/`DELETE DATA` carry their triples
  inline (a text cap *does* bound those; `CREATE` adds one empty-graph entry), but
  `CLEAR`/`DROP` cost whatever
  the targeted graphs already hold — their bound is authorization, since `NAMED`/`ALL`
  escalates to the all-graphs wildcard and a targeted `CLEAR GRAPH <g>` needs write on `g` —
  and `LOAD` reads a file whose size is independent of the update text (refused entirely
  unless an allowlisted base is installed via `sparq_engine::with_load_base`). A caller
  wanting a hard ceiling on those must add it itself. A budget can only add a failure mode,
  never widen authorization; an unlimited budget is byte-for-byte `update_as`. Failure
  semantics are `update_as`'s, including its engine-inherited **non-atomicity on error**
  across a `;`-separated multi-operation request (a single operation's WHERE is fully
  evaluated before any of its delta lands, so the common over-budget case mutates
  nothing).
- `store.put_acl(acl_iri, content, format) -> AclWriteOutcome` /
  `store.delete_acl(acl_iri)` (+ `…_acp` variants) — **authoritative ACL write-through**
  ([OPUS-4.8] issue #992 FR-3, sq-snopa.5): the LDP `PUT`/`DELETE /resource.acl` STORAGE
  primitive. Replaces (or removes) the `.acl`/`.acr` GRAPH and re-materializes
  `<urn:sparq:auth>` as ONE **atomic, fail-closed** unit, so SPARQ stays the source of
  truth (the auth view is always a pure function of the present `.acl` graphs — no stale
  grant survives a rule change). `content` is parsed FIRST (malformed ⇒ `Err`, nothing
  mutated); a re-materialization failure (e.g. a reserved-encoding principal) ROLLS BACK to
  the prior content + prior auth view (all-or-nothing). A non-`.acl`/`.acr` target IRI is
  rejected (write content graphs through `update_as`). It performs **NO session
  authorization** — the server authorizes the request itself (e.g. `decide(.., Mode::Control)`)
  and `update_as` remains the session-checked write path. Always-present API (no cargo gate;
  mirrors `update_as`/`decide` — adds no dependency).
  **Scale ([OPUS-4.8] issue #1571, sq-b7k7u):** an atomic single-`.acl`/`.acr` write still
  re-materializes the whole auth view (the incremental materializer is deferred), but the
  session-cache invalidation is **diff-based** ([SONNET-4.6] sq-b7k7u fix): `reindex_with`
  diffs old vs new `AuthIndex` per-origin and invalidates exactly the origins whose (allow,
  deny, cond) buckets changed — so a write to one pod does **not** cold-start every other
  pod's cached view, and cross-origin dependencies (WAC `acl:agentGroup` membership hosted on
  a different pod, foreign-subject grant triples, ACP cross-document policy/matcher
  indirection) are caught automatically: if a write at origin A changes B's effective grants,
  B's new index buckets differ → B is invalidated too. If the matcher maps change (not
  origin-bucketed), it falls back to a full cache clear. A differential property test asserts
  the scoped-cache state equals a from-scratch rebuild after every write (incl. the
  fail-open-critical revoke direction and cross-origin agentGroup revoke).
  The same origin-partitioned index makes `decide` cost one pod's grants, not the whole store.
- `store.accessible(&Session, Mode) -> Arc<Vec<NamedNode>>` /
  `store.accessible_set(...)` / `store.view_for(...) -> DatasetView` /
  `store.auth() -> &AuthIndex` — inspect the authorized graph set or the materialized
  index directly.
- **Concurrent reads — the read side is `&self`** ([FABLE-5] sq-cnuqd, issue #1569). Every
  read entry point (`accessible`, `accessible_set`, `view_for`, `query_as`, `query_json_as`,
  `ask_as`, `query_as_rewrite`, `wac_allow`, `decide`) takes `&self`, so **N threads sharing
  one `Arc<PodStore>` query the same materialized generation at once** — a per-request
  Solid/SPARQL server no longer serialises every reader through one exclusive borrow. The
  session cache behind these is **sharded + bounded**: `session_cache::SessionCache` stripes
  the memoized per-session graph sets across independent `RwLock` shards keyed by the session
  hash, so a warm hit is a shard *read* lock (+ two `Arc` clones) and only a miss/re-derive
  takes that one shard's write lock; each shard is **LRU-bounded** (`SHARD_CAP`, with a
  compacted recency queue) so a server keying per-request `now` timestamps cannot grow the
  cache without bound. **Writes stay `&mut self`** (`materialize_*`, `put_acl`/`delete_acl`,
  `update_as`) — they need exclusive access, so a server interleaves reads and writes by
  wrapping the store in an `RwLock<PodStore>`/arc-swap (readers take the read lock, a write
  takes the write lock). **Snapshot consistency (fail-closed, no torn set):** each read pins
  ONE `Arc<AuthIndex>` snapshot for the whole set computation, and because writes are
  `&mut self` (exclusive), a read observes either the whole pre-write generation or the whole
  post-write one, never a half-and-half mix (generation pinning composes with #1584);
  invalidation (`SessionCache::clear` / `invalidate_origin`, the diff-based #1585 path) visits
  every shard so it reaches a session regardless of which shard it hashed into, and an
  evicted/dropped set is re-derived on the current auth view — never assumed still authorized.
  Covered by `tests/concurrent_readers.rs` (16 threads on one `Arc<PodStore>`; a revoke under
  concurrent read observes only the two atomic snapshots and converges on the revoked view).
- `store.wac_allow(&Session, &NamedNode) -> String` — build the public
  [`WAC-Allow`](https://solidproject.org/TR/wac#wac-allow) response-header value
  (`user="…",public="…"`) advertising the modes the session (and the public) hold on a
  resource ([OPUS-4.8] sq-i7k08); fail-closed, only the modes actually held. See the
  request-pipeline section below.
- `store.decide(&Session, resource: &str, Mode) -> WacDecision` /
  `decide_batch(&Session, &[(&str, Mode)]) -> Vec<WacDecision>` — **per-REQUEST WAC
  decision** ([OPUS-4.8] issue #992 FR-1, sq-snopa.1): the point-query *"may principal X do
  mode M on resource R?"* an LDP resource server asks (NOT graph filtering). Returns
  `WacDecision { allow, granted_modes: Vec<Mode>, governing_acl: Option<NamedNode>, scope:
  Option<AclScope>, status: AclStatus }`. The verdict reuses the SAME fail-closed
  `AuthIndex::accessible` oracle, so a `decide` allow is never wider than `query_as` would
  grant; `granted_modes` carries the full mode set in one decision (build a `WAC-Allow`
  body without four sweeps). `decide_batch` builds the structural ACL index ONCE for a page
  of resources. Always-present API (no cargo gate; mirrors `wac_allow`). **API tier-1
  (proposed-stable)** — the proposed semver-stable per-resource decision surface (freeze
  pending maintainer ratification, #1346 / #1248; see
  [`docs/api-stability.md`](../../docs/api-stability.md)).
- `store.decide_create(&Session, container: &str, child_name: &str, Mode) -> WacDecision` /
  `is_control_document_name(name: &str) -> bool` — the **CREATE decision + its
  control-document guard** ([OPUS-5] sq-gg0qq.5, issue #2571). Minting a container child
  needs only `acl:Append`, but an access-control document is governed by `acl:Control`, so
  an Append-only principal who POSTs `Slug: secret.acl` passes the container's mode check
  and still ends up authoring `<container>/secret.acl` — the document the ACL resolver
  afterwards consults as `<container>/secret`'s governing ACL. The escalation is carried by
  the NAME, so a mode-only `decide` cannot see it. `decide_create` runs the name through
  `is_control_document_name` BEFORE the mode question and refuses `.acl`/`.acr` for **every**
  principal, `Control` holders included (the legitimate route to author an ACL is a
  `Control`-gated write of the governed resource's own ACL, never a child mint — so the
  create path stays uniformly closed). The guard is deliberately broader than the exact-case
  suffix the resolver derives: case variants (`secret.ACL`), the container-child trailing
  slash (`secret.acl/`), the bare `.acl`/`.acr`, and percent-encoded spellings (`secret%2Eacl`,
  `secret.ac%6C`, double-encoded, or a smuggled `%2F`) are all refused; a mid-path `.acl`
  (`x.acl/child`) and a name that merely contains `acl` are not. Unusable child names (empty,
  `.`/`..`, any decoded path separator) and a non-slash-terminated `container` are refused
  the same way. **The refusal is DEFINITIVE (`Resolved` ⇒ 403), never retryable** — it does
  not depend on the ACL state, so a transient/`Unloaded` condition can never downgrade it to
  "try again". When the name is benign the decision is exactly `decide(container, mode)`,
  fail-closed contract unchanged. `mode` is what the create verb requires on the CONTAINER
  (`Append` for POST, `Write` for a PUT-create); this crate's rules do not materialize WAC's
  `Write` ⇒ `Append` subsumption, so ask for the mode you mean. `is_control_document_name`
  is public so a resource server applies the SAME predicate at its own mint chokepoint and
  the two cannot drift.
- `WacDecision::acl_link_header() -> Option<String>` / `AclScope::as_acl_predicate() ->
  &'static str` — **effective-ACL provenance surface** (FR-5, sq-snopa.4): the `<acl-iri>;
  rel="acl"` RFC-8288 [`Link`](https://solidproject.org/TR/protocol#acl-resource) value a
  server emits alongside `wac_allow` so a client can discover/edit the governing ACL, plus
  the WAC predicate IRI (`acl:accessTo`/`acl:default`) for the scope in structured logs.
  Built straight from the `governing_acl`/`scope` FR-1 already puts on the decision (so it
  can never point at a document discovery did not find). **Fail-closed:** `None` when no
  governing ACL was discovered (`NoAcl`/`Transient`) — nothing to advertise; `Some` on
  `Unloaded` (the ACL *is* discovered, even on a retryable 503). It surfaces *where* the ACL
  is, never a verdict, so it is safe to emit on a deny.
- `AclStatus` — the **typed fail-closed load/error contract** (FR-6, sq-snopa.2): a server
  maps `Resolved` (authoritative — `allow` is the real verdict; a deny is **403**), `NoAcl`
  (no governing ACL anywhere up the chain — definitive **403**; `AclStatus` carries no
  authentication state, so the shipped `solid-authz` shell emits that 403 for anonymous
  requesters too. On an LDP resource-serving path that is a **known non-conformance**, not a
  permitted stricter choice: Solid requires **401** + `WWW-Authenticate` when a request lacks
  the credentials a protected resource needs, so a resource server must add that lane from
  its own authentication state), `Unloaded` (a governing
  ACL exists but `materialize_*` was never run — a **retryable 503**), and `Transient`
  (a typed transient error, e.g. a malformed resource IRI — a **retryable 503**).
  `AclStatus::is_retryable()` separates the 503s from the definitive denies. **It never
  fails OPEN:** `allow == false` for every non-`Resolved` status, independent of whether the
  caller inspects `status` — absent ⇒ deny, present-but-unloaded ⇒ deny, transient ⇒ deny.
- `store.resolve_acl(resource: &str) -> Option<EffectiveAcl>` — **ACL-discovery walk in ONE
  indexed call** (FR-7, sq-snopa.3): the up-the-container-chain `.acl`/`.acr` discovery +
  `acl:default` inheritance, returning `EffectiveAcl { acl: NamedNode, scope: AclScope }` —
  `AccessTo` when the resource has its OWN ACL, `Default` when it inherits from the nearest
  ancestor container — instead of N round-trips. The walk is exactly the loader's
  slash-semantics `parent_iri` chain, so per-resource discovery and the materialize-time
  inheritance can never disagree; `None` ⇒ un-protected ⇒ fail closed. Feeds `decide`'s
  `governing_acl`/`scope`, which `WacDecision::acl_link_header()` turns into the FR-5
  `Link: rel="acl"` surface (above). **Honest scope:** Phase-1 implements
  the WAC `accessTo` + container-`default` subset of [issue #992](https://github.com/sparq-org/sparq/issues/992)
  (FR-1/6/7) — the per-resource decision + fail-closed contract + ACL walk. The **HTTP shell**
  over this library surface (FR-4, sq-snopa.6) has LANDED as `sparq-server`'s opt-in
  `solid-authz` feature — `POST /authz/decide`+`/wac-allow`+`/query`, a fail-closed thin wrapper
  that maps these very verdicts onto HTTP status codes (an `AclStatus` `403`/`503` split). The
  **ODRL lane on `/authz/*`** (sq-lrtc3.1 + sq-3mu76) has LANDED as `sparq-server`'s opt-in
  `odrl-authz` feature — dataset-carried ODRL policies are fed through `materialize_odrl_policy`
  (deny-overrides, fail-closed) before the access-controlled query runs, so an ODRL policy gates a
  SPARQL query end-to-end over HTTP; since sq-3mu76 the advisory `/authz/decide` +
  `/authz/wac-allow` run the same lane (read-scoped advertisement), so they never report an allow
  the query lane would refuse to honour. The
  **FR-5 `Link: <acl-iri>; rel="acl"` response header** (sq-snopa.7) is now wired: `/authz/decide`
  and `/authz/wac-allow` emit it from `acl_link_header()` / `resolve_acl()` when a governing ACL
  was discovered; `None` ⇒ no header (fail-closed). The **STATEFUL lane** (sq-snopa.8) has also
  LANDED: `"source":"server"` authorises over the server's OWN loaded store instead of a body
  dataset, materialising this same `PodStore` once per ring generation (so an `.acl` write
  re-materialises by construction) and refusing — never silently un-enforcing — an ODRL-carrying
  store or a `trust` block. See the `http-server` skill. That HTTP layer
  does NOT authenticate — it takes an already-resolved session, exactly as this library does. The
  `decide` `scope` is the ACL-*document* discovery scope, while whether a grant within that ACL
  applies is the verdict the oracle computes.
- `store.materialize_odrl_permission(&Policy, &Request) -> BridgeOutcome` — **opt-in**
  (`odrl-bridge` feature, OFF by default; [OPUS-4.8] sq-h3uk): run the `sparq-policy` ODRL
  evaluator and, on a *definite Permit*, materialize the equivalent `principal auth:<mode>
  graph` grant into the auth view, then reindex — so this same enforcement path applies it.
  Fail-closed (Deny / unmapped action / partyless / targetless → no grant). See the
  [`usage-control-policy`](../usage-control-policy/SKILL.md) skill for the action→mode mapping.
- `store.materialize_odrl_prohibition(&Policy, &Request)` / `materialize_odrl_policy(...)` —
  same opt-in feature ([OPUS-4.8] sq-w693): a matched ODRL **Prohibition** materializes the dual
  `principal auth:deny<Mode> graph` triple, honoured by this enforcement under **deny-overrides**
  (`∪ allow ∖ ∪ deny` — a deny beats any allow for the same principal+target+mode). `…_policy`
  does both sides at once. Same fail-closed rules; no new enforcement engine.
- **Loud refusal of an unimplementable `odrl:conflict` strategy** ([OPUS-4.8] sq-ihqbl): the bridge
  implements only `odrl:conflict odrl:prohibit` (deny-overrides). A policy declaring `odrl:perm`,
  `odrl:invalid` **with** a detected conflict, or an unknown strategy IRI is **REFUSED** — every
  `materialize_odrl_*` entry materializes nothing and returns `BridgeOutcome { refused: true, .. }`
  with a `REFUSED (odrl:conflict): …` reason, rather than silently coercing it into deny-overrides.
  An unset `odrl:conflict` operates under `prohibit` (not refused) — sparq's own fail-closed default, a
  **deliberate divergence** from the ODRL 2.2 IM default of `invalid`, not a spec default the bridge is
  following (the ODRL Formal Semantics CG report supplies no conflict default either — its
  conflict-resolution machinery is explicitly pending). See the `usage-control-policy` skill.
- `store.materialize_odrl_permission_conditional(&Policy, &Request) -> BridgeOutcome` —
  **opt-in** (`odrl-bridge`; [OPUS-4.8] sq-hiz4): persists a *faithfully-mappable* ODRL
  constraint as a re-checked ACP `auth:ConditionalGrant` (agent matcher) instead of a
  one-shot allow — so the granted agent is verified **per session**, not frozen to the
  materializing party. `odrl:recipient`/`odrl:assignee`
  (`eq`/`isA`/`isPartOf`/`isAnyOf`/`neq`/`isNoneOf` — the set operators one head /
  exception per member, [FABLE-5] sq-5fkpp) maps
  faithfully (recipient-of-data = session agent); an `odrl:dateTime` **inclusive** bound
  (`lteq` → `auth:notAfter`, `gteq` → `auth:notBefore`) maps to a **live-clock window**
  re-checked against `Session::now` per request ([OPUS-4.8] sq-0q7n — a lapsed window denies
  immediately, no `refresh_odrl_grant` needed); `odrl:purpose`/`count`/a *strict* `dateTime`
  bound, **and any compound `odrl:LogicalConstraint`** (`odrl:and`/`odrl:or`/`odrl:xone` —
  no faithful single-head analogue; [OPUS-4.8] sq-izzak) have no faithful analogue and STAY
  one-shot; a rule mixing mappable + unmappable
  constraints falls back **entirely** to one-shot (fail-safe — never drops a bound). A
  dateTime window is mapped only on an **allow** (a lapsed *deny* would fail open). A bare
  `odrl:assignee` **rule PROPERTY** (with zero constraints) scopes the grant head to that ONE
  assignee — **not** `auth:Public` ([OPUS-4.8] sq-9n1q4; the deny dual likewise scopes to the
  assignee, never an over-broad public deny). Only a rule with **no** recipient constraint AND
  **no** assignee grants `auth:Public`. Mapping
  table in the [`usage-control-policy`](../usage-control-policy/SKILL.md) skill.
- `odrl_bridge::materialize_odrl_n3(&mut Graph, policy_ttl: &str, &Request) -> Result<BridgeOutcome, String>`
  — **opt-in** (`odrl-bridge`; [SONNET-4.6] sq-zgbso.2, [OPUS-5] sq-zgbso.2): an alternative
  materialization path that runs the stateless ODRL core as **five stratified `reason_n3` calls**
  (`rules/odrl-{a0,a,b,c,d}.n3`), mirroring the WAC/ACP N3-stratification pattern instead of the
  Rust evaluator. The relationship to `materialize_policy` is two claims, both checked over a
  GENERATED corpus in `tests/odrl_n3_differential.rs` (a hand-picked sample previously missed two
  fail-open defects, so the scope statement below is deliberately explicit):
  - **Never more permissive — everywhere.** For every policy and request, either the call returns
    `Err` (refusing the policy outright; the caller materializes nothing and must fail closed), or
    every allow it emits is also emitted by the Rust path AND every deny the Rust path emits is
    also emitted by it. Under `∪ allow ∖ ∪ deny` that is exactly "never widens access".
  - **Decision-equivalent — within a NARROWED scope**: permissions AND prohibitions over
    `odrl:dateTime` `lteq`/`gteq` and `odrl:recipient` `eq`/`neq`, combined by ≤1
    `odrl:or`/`odrl:and` logical constraint per rule, **each constraint node carrying exactly one
    operand tuple and at most one combinator**, under an **admissible `odrl:conflict` strategy**,
    for a **concrete mapped action** (the `odrl:use` umbrella is excluded — the Rust evaluator lets
    a `use` permission permit a `read` request while the strata match action IRIs exactly).
  **Fail-closed & injection-safe**: `policy_ttl` is parsed
  **strictly as Turtle** (N3 `=>` rules and every Turtle-extension are a syntax `Err`) and only the
  validated ground terms are re-serialized into the reasoner input, so a crafted policy cannot smuggle
  rules or derive `auth:*` directly; a triple mentioning a reserved/engine-owned IRI term, a
  non-canonical `xsd:dateTime` lexical form (only `YYYY-MM-DDTHH:MM:SSZ` compares correctly
  lexically), an **ambiguous constraint node** (several operand triples in one position, or several
  combinators — satisfaction is keyed on the constraint NODE, so such a node would be decided from
  one cross-product combination: "some operand satisfied" where "every operand satisfied" is
  required), an **`odrl:conflict` strategy the bridge cannot honour** (`odrl:perm`, an unknown
  strategy IRI, or `odrl:invalid` with a detected conflict — the same
  `sparq_policy::conflict_admissibility` verdict the Rust path refuses on), an out-of-scope
  **prohibition** construct (which the Rust evaluator might satisfy —
  silently ignoring it would drop a deny and WIDEN access), or an invalid request-field IRI all return
  `Err` and materialize **nothing**. Grants/denies land in `<urn:sparq:auth>` +
  `<urn:sparq:auth-bridged>` exactly as the Rust bridge writes them.
- `store.refresh_odrl_grant(&Policy, &Request, BridgeKind)` / `refresh_odrl_grants()` —
  **opt-in** (`odrl-bridge`; [OPUS-4.8] sq-dpk4): re-evaluate **bridged** ODRL grants when
  the policy changes and **retract** the ones that no longer hold (a withdrawn permission, a
  lapsed time window, a re-evaluation that now Denies) while preserving static WAC/ACP grants
  and still-valid bridged grants. Bridged triples are tracked in a ledger and mirrored into a
  provenance graph `<urn:sparq:auth-bridged>` (`AUTH_BRIDGED_GRAPH`) so they are structurally
  distinguishable from static grants; refresh rebuilds the view as `static_baseline ∪
  replay(valid bridged entries)`. **Fail-closed**: any ambiguous re-eval of an *allow grant*
  retracts (access never left stale); a static grant is never re-evaluated or dropped. A
  wholesale static re-materialization (`materialize_wac`/`materialize_acp`) auto-reconciles —
  valid bridged grants are replayed back on top. A bridged **deny** ([OPUS-4.8] sq-2pcf) is
  the dual and uses the OPPOSITE fail-closed rule: a materialized `auth:deny*` is retracted
  only when the ODRL Prohibition is **definitely** withdrawn/lapsed (`prohibition_status ==
  Withdrawn`); an *ambiguous* re-eval **keeps** the deny (never restore access on missing
  evidence). A retracted deny may re-expose an allow grant — correct, since the prohibition
  is genuinely gone.
- `Session { agent: Option<&str>, client: Option<&str>, issuer: Option<&str>, now: Option<&str> }`
  (agent/client/issuer caller-asserted: WebID + `acl:origin`/`acp:client` + the OIDC
  `acp:issuer`; `None` = anonymous / any client / any issuer respectively). [OPUS-4.8]
  sq-3jtd.6: the `issuer` field is the third matcher dimension (ACP only — WAC ignores it)
  and is a STRING MATCH on a caller-asserted issuer, **not** an authentication step (see
  "What this is — and is NOT"). [OPUS-4.8] sq-0q7n: `now` is the request clock (an
  `xsd:dateTime` lexical string), consulted **only** by time-windowed conditional grants —
  a windowed grant with `now == None` fails closed; set it with `Session::at(now)`.
  `Mode::{Read, Write, Append, Control}`; `wac_fixture()` / `acp_fixture()` (bundled demo
  pods).
- `triple_principal(agent, client, issuer) -> String` / `pair_principal(agent, client) ->
  String` — the deterministic minted principal IRIs (`urn:sparq:triple?…` /
  `urn:sparq:pair?…`) that the ACP/WAC N3 rules emit for an issuer- / client-constrained
  grant, kept in sync with the rules. Both use RFC 3986 percent-encoding and are
  INJECTIVE, so no agent/client/issuer value can smuggle a `&client=`/`&issuer=`
  delimiter into another principal's term. Exposed for inspection/round-trip tests.
- `ANY_ISSUER` / `ANY_CLIENT` / `PUBLIC` / `AUTHENTICATED` — the principal-lattice top
  IRIs (`https://sparq.dev/ns/auth#AnyIssuer` etc.). [OPUS-4.8] sq-3jtd.6: an ACP grant
  with no `acp:issuer` matcher is issuer-unconstrained ⇒ `ANY_ISSUER` (the issuer-dimension
  top); a session with `issuer: None` matches it.

The materialized view is itself just triples — *"who can read G?"* is one SPARQL
pattern: `GRAPH <urn:sparq:auth> { ?who <https://sparq.dev/ns/auth#read> ?doc }`.

## Request-pipeline integration for a Solid server (incl. WAC-Allow headers) — issue #1126

`examples/quickstart.rs` shows the load → `materialize_wac` → `query_as`/`accessible`
loop. A Solid **request handler** wraps that with three steps: (1) build a `Session`
from the authenticated request, (2) gate the request with `accessible(...)` /
`query_as(...)`, (3) emit a [`WAC-Allow`](https://solidproject.org/TR/wac#wac-allow)
response header advertising the modes the agent (and the public) hold on the target
resource. Step (3) is the public helper
[`PodStore::wac_allow(&Session, &NamedNode) -> String`](https://docs.rs/sparq-solid)
([OPUS-4.8] sq-i7k08) — it builds the RFC-style `user="…",public="…"` value over the
existing `accessible` API (the `user` list is the authenticated session's modes; the
`public` list is an anonymous `Session::default()`'s), fail-closed and with only the
modes actually held:

```rust
use sparq_solid::{Mode, PodStore, Session};
use oxrdf::NamedNode;

// One `Session` per request; `client`/`issuer` come from the auth layer (None = any).
fn session_from_request<'a>(webid: Option<&'a str>, origin: Option<&'a str>) -> Session<'a> {
    Session { agent: webid, client: origin, issuer: None, now: None }
}

// Does this session have `mode` on the graph backing `resource`? (fail-closed)
// [FABLE-5] sq-cnuqd: `&PodStore` (shared) — many request handlers call this at once.
fn may(store: &PodStore, s: &Session, mode: Mode, resource: &NamedNode) -> bool {
    store.accessible(s, mode).iter().any(|g| g == resource)
}

// Build the WAC-Allow header value for the request, e.g. `user="read write",public="read"`.
fn wac_allow_header(store: &PodStore, s: &Session, resource: &NamedNode) -> String {
    store.wac_allow(s, resource)
}
```

Per request: `session_from_request(...)` → `may(&store, &s, Mode::Read, &resource)`
to allow/deny (a deny is `403`/`404` per your fail-closed policy) → set the response's
`WAC-Allow` header to `store.wac_allow(&s, &resource)`. `wac_allow` runs four
`accessible` sweeps that share the per-session cache (each an O(1) hash check over the
materialized index), so it is cheap. Because these read helpers take **`&self`** (sq-cnuqd),
a concurrent server shares ONE `Arc<PodStore>` (or `Arc<RwLock<PodStore>>` if it also serves
writes) across all request workers — they read in parallel through the sharded session cache;
only a `put_acl`/materialize write needs the exclusive (`&mut`/write-lock) side. To pair the `WAC-Allow` advertisement with the WAC
**ACL-discovery** header, run `store.decide(&s, "<resource-iri>", Mode::Read)` and emit its
`acl_link_header()` (`Some("<acl-iri>; rel=\"acl\"")`) as the response `Link` header value
when present (FR-5) — fail-closed: it is `None` when no governing ACL was discovered, so
nothing leaks on an un-protected resource. **Scope caveat:** sparq-solid is a
*library-level* authoriser — there is no HTTP surface here (no `.well-known`, no served
`acl:` resource); `acl_link_header()` builds the `Link` header *value* from the discovered
governing ACL, but emitting it (and mapping a **request path to its named graph**, and
authenticating the WebID) is the server's job (see `research/sparq-solid-scope.md` §4).
`wac_allow`/`acl_link_header` build header *values* from the authorization verdict +
discovery; neither is itself a conformance-tested HTTP layer.

## Capability notes (WAC + ACP)

- **WAC** (`.acl`) — `acl:agent` (WebID), `acl:agentClass foaf:Agent` (public) /
  `acl:AuthenticatedAgent`, `acl:agentGroup`, `acl:default` inheritance, the
  `acl:Read/Write/Append/Control` modes.
- **ACP** (`.acr`) — policies / matchers with the `allOf` / `anyOf` / `noneOf`
  combinators and normative **deny-overrides**. [OPUS-4.8] sq-3jtd.6: matchers may now
  also constrain on `acp:issuer` (the caller-asserted OIDC issuer), the third principal
  dimension. [OPUS-4.8] sq-3jtd.5: matchers may also use `acp:agent acp:CreatorAgent` /
  `acp:OwnerAgent` — the context agent must be the resource's creator / owner, resolved
  against the TRUSTED per-resource provenance supplied via `materialize_acp_with` (above),
  resource-scoped and fail-closed. [SONNET-4.6] sq-ysv3u: matchers may also constrain on
  **`acp:vc`** — the context agent must hold a verified credential satisfying the named
  requirement, resolved against the TRUSTED holdings supplied via
  `materialize_acp_with_credentials` (above), and fail-closed with none supplied.
  [GPT-6] A matcher with no supported attribute (`acp:agent`, `acp:client`,
  `acp:issuer`, or `acp:vc`) is unsatisfied, as required by
  [ACP §6.5](https://solidproject.org/TR/acp#satisfied-matcher). Removing its final
  attribute and rematerializing revokes grants requiring that matcher, including
  cached query access. An empty `noneOf` matcher cannot block an otherwise
  satisfied policy. A policy still requires a positive `allOf` or `anyOf` matcher
  ([ACP §6.4](https://solidproject.org/TR/acp#satisfied-policy)).
- Principal lattice — three independent dimensions (agent, client, issuer):
  `Public ⊒ Authenticated ⊒ concrete-WebID`, `AnyClient ⊒ concrete-client`, and
  ([OPUS-4.8] sq-3jtd.6) `AnyIssuer ⊒ concrete-issuer`. A session expands to the agent
  principals, plus — when a client is given — the minted `(agent, client)` pair, plus —
  when an issuer is given — the minted `(agent, client, issuer)` triple (with
  `AnyClient` as the client component when no client was supplied), for ≤12 grant lookups.
  Only an issuer-CONSTRAINED grant mints a triple principal; an issuer-unconstrained grant
  (`AnyIssuer`) reuses the agent / pair term, so issuer-blind pods are unaffected.

## ACP conformance harness — [OPUS-4.8] sq-3jtd.9

`sparq_solid::conformance` is a **library-level ACP conformance harness**: a table-driven
scenario runner over this crate's own ACP engine (`materialize_acp` +
`AuthIndex::accessible`). A scenario is declared as **data** — an ACR-document corpus plus a
table of expected `(agent, client, mode, resource) → Allow | Deny` decisions — and the
harness asserts the engine reproduces every expected decision, reporting **all** mismatches
at once. The module is **always compiled** (no feature gate; it depends only on the
always-present ACP path). The scenario corpus is a single reusable source in
`crates/sparq-solid/tests/common/` (`common::acp_corpus()`, 13 scenarios / 45 decisions),
consumed by `crates/sparq-solid/tests/conformance_acp.rs` (the parity test, plus a negative
control asserting a wrong expectation is *reported*, not panicked) so a second test target —
the differential oracle (`sq-t58w.7`) — can run the IDENTICAL scenarios without copy-paste.

What it is — and is NOT (honest scope, mirrors the module/README/`research/sparq-solid-scope.md` §4):

- It is the **realistic, achievable** conformance signal for an authorization *oracle* —
  library-level **decision parity** against the [Solid ACP spec](https://solidproject.org/TR/acp)
  semantics. It is **NOT a normative-CTH pass** and makes no such claim.
- **CTH-over-HTTP** (the Solid Conformance Test Harness / `solid/specification-tests`, which
  drives a *server* and asserts on HTTP outputs) is **out-of-scope** — this crate has no HTTP
  surface; that belongs to a Solid server, conformance-tested *through* it.
- A **JS-reference differential oracle** (Community Solid Server / Inrupt ESS over the same
  corpus) is the credible *second* oracle but is research-open (JS-toolchain cost) and is
  **not** built here — captured as a follow-up.
- It is a **test/oracle harness over the existing engine** — it does not change authorization
  behaviour. It is complementary to `tests/acp.rs` (a hand-derived access matrix over one
  realistic pod): this harness gives spec-construct coverage with small, independently-failing
  cases.
- The **WAC** conformance harness (`sq-3jtd.8`, `sparq_solid::wac_conformance`) mirrors this
  one — the same harness shape with a WAC `.acl` corpus builder; see its section below. The
  two share the `conformance::{Decision, Expect, ScenarioReport}` vocabulary.

Public surface (`pub mod conformance`, re-exported from the crate root for the entry types):

- `AcrBuilder` — build the ACR-document corpus as N-Quads. `acr.access_control(resource,
  |p| …)` / `acr.member_access_control(resource, |p| …)` attach a policy (`acp:accessControl`
  / `acp:memberAccessControl` for cumulative ancestor inheritance); `acr.document(resource)`
  declares the protected resource graph; `acr.into_nquads()` emits the corpus.
- `PolicyBuilder<'a>` (the `|p| …` closure argument) — `allow(Mode)` / `deny(Mode)`,
  `any_of_agent` / `all_of_agent` / `none_of_agent`, `any_of_client` / `all_of_client`,
  `all_of_pair(agent, client)` — the `acp:allOf` / `acp:anyOf` / `acp:noneOf` combinators over
  `acp:agent` / `acp:client` matchers, with `PUBLIC_AGENT` / `AUTHENTICATED_AGENT` consts for
  the `acp:PublicAgent` / `acp:AuthenticatedAgent` matcher IRIs.
- `AcpScenario` — `new(name)`, `.acr(AcrBuilder)` / `.nquads(&str)` (the corpus),
  `.expect(Expect)` / `.expect_all(iter)` (the decision table), `.run() ->
  Result<ScenarioReport, String>` (materializes + checks). `run_corpus(&[AcpScenario])` runs
  a whole corpus. Read-only accessors `.name()`, `.nquads_str() -> &str` (the raw N-Quad
  corpus the engine loads) and `.expects() -> &[Expect]` (the decision table) let a SECOND
  consumer — the differential oracle — feed the IDENTICAL corpus to an independent decider.
- `Expect` — one expected decision, built fluently: `Expect::agent(webid)` /
  `Expect::anonymous()` / `Expect::pair(agent, client)`, then `.read/.write/.append/.control(
  resource)`, then `.is(Decision::Allow | Decision::Deny)`.
- `ScenarioReport` — `.passed()`, `.checked()`, `.mismatches()` (the failing
  `DecisionResult`s), `.name()`, and a readable mismatch `Display`. `Decision::{Allow, Deny}`
  is the binary the harness compares against the engine verdict (graph in the session's
  accessible set ⇒ `Allow`).

## WAC conformance harness — [OPUS-4.8] sq-3jtd.8

`sparq_solid::wac_conformance` is the **WAC sibling** of the ACP harness above: the same
table-driven scenario runner, but over this crate's **WAC** engine (`materialize_wac` +
`AuthIndex::accessible`) against the [Solid WAC spec](https://solidproject.org/TR/wac)
semantics. A scenario is an `.acl`-document corpus (`AclBuilder`) plus a table of expected
`(agent, client, mode, resource) → Allow | Deny` decisions; the harness asserts the engine
reproduces every one, reporting all mismatches at once. Always compiled (no feature gate).
The corpus is a single reusable source in `crates/sparq-solid/tests/common/`
(`common::wac_corpus()`, 13 scenarios / 51 decisions), consumed by
`crates/sparq-solid/tests/conformance_wac.rs` (the parity test, a `run_via_podstore` parity
test over the full `PodStore` method-form path, and a negative control) and reusable by a
second test target — the differential oracle (`sq-t58w.7`) — without copy-paste. The
honest-scope caveats are **identical** to the ACP harness's (library
decision parity, NOT a normative-CTH pass; CTH-over-HTTP out-of-scope; CSS differential oracle
research-open).

Public surface (`pub mod wac_conformance`, re-exported from the crate root for the entry types;
the decision/expectation/report types are the **shared** `conformance::{Decision, Expect,
ScenarioReport}`):

- `AclBuilder` — build the `.acl` corpus as N-Quads. `acl.access_to(resource, |a| …)` /
  `acl.default_for(resource, |a| …)` / `acl.access_to_and_default(resource, |a| …)` attach an
  `acl:Authorization` (`acl:accessTo` on the resource's own ACL / `acl:default` for
  **nearest-ancestor** inheritance over members / both); `acl.document(resource)` declares the
  protected resource graph + inheritance anchor; `acl.into_nquads()` emits the corpus.
- `AuthBuilder<'a>` (the `|a| …` closure argument) — `mode(Mode)`, `agent(webid)`,
  `public()` / `authenticated()` / `agent_class(class)` (`acl:agentClass foaf:Agent` /
  `acl:AuthenticatedAgent`), `agent_group(group, &[members])` (emits the `vcard:hasMember`
  group document), `origin(client)` (the `acl:origin` (user, app) pair). `PUBLIC_AGENT`
  (`foaf:Agent`) / `AUTHENTICATED_AGENT` consts name the two recognised classes.
- `WacScenario` — `new(name)`, `.acl(AclBuilder)` / `.nquads(&str)`, `.expect(Expect)` /
  `.expect_all(iter)`, `.run() -> Result<ScenarioReport, String>` (materializes via the free
  `AuthIndex` path + checks), and `.run_via_podstore()` (the same check through the full
  `PodStore` method form — proves the path a PSS integration uses). `run_corpus(&[WacScenario])`
  runs a whole corpus. Read-only accessors `.name()`, `.nquads_str() -> &str` and `.expects()
  -> &[Expect]` expose the raw corpus + decision table to the differential oracle (below).
- The `Expect` builder, `Decision`, and `ScenarioReport` are the same shared types as the ACP
  harness — `Expect::agent(webid)` / `Expect::anonymous()` / `Expect::pair(agent, client)`,
  `.read/.write/.append/.control(resource)`, `.is(Decision::Allow | Deny)`.

## Differential oracle — [OPUS-4.8] sq-t58w.7

`crates/sparq-solid/tests/differential_oracle.rs` is a **three-way agreement check** that
runs the shared parity corpus (`common::wac_corpus()` / `common::acp_corpus()`) through
THREE deciders for every `(agent, client, mode, resource)` request and asserts they all
agree:

1. **the engine** — `materialize_{wac,acp}` + `AuthIndex::accessible` (the **N3-rules**
   paradigm — `rules/*.n3` run by `sparq-reason`);
2. **an independent reference evaluator** — `crates/sparq-solid/tests/reference/{wac,acp}.rs`,
   a from-scratch **PROCEDURAL** reading of the WAC/ACP spec over a hand-parsed model. Its
   whole value is being a **different paradigm**: it shares **no** code with `materialize.rs`
   / `loader.rs` / `rules/*.n3`, and parses the corpus with its own tiny N-Quad reader (not
   `Graph::load_dataset`), so a shared bug cannot hide in both deciders;
3. **the hand `Expect` table** — the human-authored expected decision in each scenario.

A **divergence** is recorded whenever any two disagree; an unclassifiable/erroring request
(failed load/materialize, or a reference parse error) counts as a divergence — **fail-closed**.
The test asserts `divergences == 0` and prints, in the SHACL/geo runner shape (so a CI ratchet
can grep it):

```text
WAC differential pairs <N> / divergences 0 (floor 0)
ACP differential pairs <N> / divergences 0 (floor 0)
```

It is a **correctness oracle over the parity corpus, not a security audit**, and the reference
evaluator implements exactly the constructs the corpus exercises (anything it does not
recognise fails closed → a divergence). Pure in-crate Rust — no JS toolchain, no network, no
clock, no Docker. A JS-reference differential twin (vs. `@solidlab/policy-engine` /
`@solid/acl-check`) is a separate research-open follow-up (`sq-t58w.9`), NOT built here.

## Security posture — fail-closed

Absence of a grant means a graph is **invisible**, and a non-authorized graph is
indistinguishable from an absent one. The reasoner is fed only ACL/ACR + structural
facts — **never pod content** — so no writable document can grant itself access; the
reserved `urn:sparq:` namespace is rejected on input and any forged `<urn:sparq:auth>`
graph is stripped at load. A session whose `agent`/`client`/`issuer` falls inside the
reserved `urn:sparq:` encoding (or carries the `&client=`/`&issuer=` delimiter) fails
CLOSED — it cannot impersonate a minted pair/triple principal. (Again: this is the
*authorization* boundary — authenticating the WebID **and verifying the issuer↔WebID
binding** are the relying application's job.)

## Trust-graph authorisation PoC — [OPUS-4.8] sq-pfae (issue #940, opt-in `trust-graph`)

The **`sparq-trust`** crate (`crates/sparq-trust`) is a **research proof-of-concept** that adds
an **admission stratum** ahead of the WAC/ACP derivation stratum: *"is this externally-attested
fact from a source I trust for this statement-type?"* On success it injects the issuer-tagged
fact so the shipped N3 reasoner merges it with the `.acr` rules to derive access — the age>18
worked example (a trusted-government VC `<Jesse> age 25` + a trust policy + the `.acr` rule
`{ ?x age ?y . ?y math:greaterThan 18 } => { ?x auth:read R }` ⇒ `<Jesse>` gains read).

It is wired into `sparq-solid` behind the **default-OFF `trust-graph` cargo feature**. With the
feature off, `sparq-solid` is byte-identical to WAC/ACP today (**strict additivity**); with it
on, `PodStore` gains two install methods. The admission gate is the §6.0 algorithm:
RDFC-1.0 canonicalise (`sparq-canon`) → a **checked** issuer signature over the commitment
(`sparq-zk`, never a self-asserted triple) → statement-type scoping via a real SHACL shape
(`sparq-shacl`) → freshness (a per-request check) → the clear-WebID holder binding.

**Two equivalent policy-authoring forms (`parse_policy` / `resolve_rule_keys` accept both;
`sq-pfae.4`).** A Control-gated trust policy may name a rule either way, and both desugar to the
SAME `Vec<TrustRule>` the gate consumes:
- **Reified** — a `trust:TrustRule` node grouping `trust:source` + `trust:forShape`/`forPredicate`
  + `trust:scope` + `trust:freshWithin` (+ key). Keeps scope/freshness **per-rule**.
- **Claim-level relational** (`trust:trustsSourceFor`, the foundational primitive, design §2.3.1)
  — a `trust:Source` node carries `trust:trustsSourceFor <shape-or-predicate>` directly, alongside
  its shared key + `trust:scope` + `trust:freshWithin`; EACH `trustsSourceFor` statement is one
  rule. This is the compact *per-(source, statement-type)* form that offers claim-level trust in
  place of ACP's type-only `acp:vc` matcher. (`acp:vc` itself is now implemented — [SONNET-4.6]
  sq-ysv3u, above — as exact-IRI requirement matching over the trusted `VerifiedCredentials`
  channel; this trust-graph form differs in reasoning over a credential's *claims* directly.)
  The object is a `sh:NodeShape` node
  (carried like `forShape`) or a predicate IRI (desugared like `forPredicate`); a blank-node source
  is rejected fail-closed. It composes with the opt-in `did` key binding (`trust:issuerDid` on the
  source node). Every soundness side-condition (checked signature, type-scope, holder binding,
  reserved-predicate guard) is identical to the reified form — the claim-level form is NOT a weaker
  admission path.

- **`admit_trust_credential_static(credential, rules, target, abac_rule_n3)`** — the
  **materialise-time** path (the `sq-xc4y` static/dynamic split). It runs only the
  session-INDEPENDENT class (signature, type-scope, scope) and installs each derived grant as an
  `auth:ConditionalGrant` whose holder (`auth:agent`) and freshness (`auth:notAfter`) are
  re-checked **per request** by the shipped sq-0q7n `cond_applies` path — so holder/freshness are
  never frozen into the materialise-once view (a stale or wrong-holder request is denied at query
  time). **Use this for a long-lived view.**
- **`admit_trust_credential_with_rule(credential, rules, session, target, abac_rule_n3)`** — the
  single-request **snapshot** path: it runs the COMBINED gate against one live `Session` and
  installs an UNCONDITIONAL grant valid for that request only (do not use it to populate a
  long-lived view).

Both install on top of the unchanged auth view (the ODRL-bridge precedent). The
public surface is `sparq_trust::{vocab, policy, admit, wire, delegation}` (+ the opt-in
`delegation_prov` / `did` / `store` / `secprop` / `framework_vocab` modules) — see
[`crates/sparq-trust/README.md`](../../crates/sparq-trust/README.md) and the design record
`research/solid-trust-graph-authz-design.md` §6.0 (tracked in
[issue #940](https://github.com/sparq-org/sparq/issues/940); landing via design PR
<https://github.com/sparq-org/sparq/pull/951>).

**Trust-document storage / authoring model** (the **opt-in `store` feature**, `sq-pfae.5`,
design §3.2). `sparq_trust::store::TrustStore` decides *where* trust rules live and *which
version* is in force: a **server-wide default** (the ceiling) plus zero-or-more **per-`.acr`
documents** (`TrustLayer::{ServerWide, Container}`), where a per-`.acr` document may only
**NARROW, never broaden** the server default — it can tighten a freshness window or restrict
scope, but can NOT trust a `(source, statement-type)` the server default did not
(`TrustStore::effective_rules`, the load-bearing soundness property). Documents are
Control-gated (same `TrustDocument::new`-requires-`ControlGate` channel as `.acl`/`.acr`),
**monotonically versioned** (a stale `put` is rejected), and revoked by removal. The
`AdmissionCacheKey` = (evidence hash + policy version) composes with the sparq-solid epoch
cache: a re-materialise bumps the epoch, a trust-document edit/revoke bumps the
`policy_version` for affected targets — so a cached admission verdict is never served stale.
Pure-Rust, no new dependency; OFF in the default build.

**Honest scope (read first).** This is a RESEARCH prototype, **NOT a security guarantee**. It
does **not** provide privacy, unlinkability, or anonymity: the credential is admitted in the
clear and the holder binding authenticates the WebID in the clear (the non-anonymous degraded
path, `sq-wvne`). The ZK estate it composes with is externally **unaudited**
(`sq-qhy4`, pending external accredited-cryptographer sign-off). Issuer keys are
operator-asserted by default (`trust:issuerKey` hex, the live forgery vector D′); the **opt-in
`did` feature** (`sq-pfae.3`) lets a rule bind its key from a `trust:issuerDid` instead —
`sparq_trust::{DidKeyResolver, DidWebResolver, resolve_rule_keys}`, `did:key` offline self-cert +
a pluggable `did:web` fetcher (no HTTP client forced on the default build). This **narrows** D′ but
is no absolute anchor: `did:key` is self-certifying, `did:web` only as strong as host/TLS. Open
problems are respected as documented limitations, never solved: `sq-tu4e` (no in-reasoner NAF over
derived facts; `revoked` is input-only; no deny-on-disagreement) and `sq-wvne` (ZK privacy) are
out of PoC scope. `sq-xc4y` (per-request holder/freshness admission vs the materialise-once view)
is **RESOLVED** by the static/dynamic split: see `admit_trust_credential_static` above and design
§3.3 A′. `sq-l5og` (the delegation invocation-binding gate, `sparq_trust::delegation`) IS
implemented: it binds each hop's `delegate_key` into the delegator-signed preimage, defeating the
key-substitution stolen-chain replay — but it does **not** claim full non-replayability, because
the delegator's own key is still operator-asserted (`sq-pfae.3`); see the crate README *Honest scope*.

**Delegation PROV-O audit for human + AI agents** (the **opt-in `delegation-prov` feature**,
`sq-pfae.6`, design §4.2/§4.3). `sparq_trust::delegation_prov::audit_invocation(chain, effective,
config)` renders an invocation-bound delegation chain + its surviving `EffectiveCapability` as a
minimal **W3C PROV-O** audit graph: each hop's `delegate prov:actedOnBehalfOf delegator` (the
on-behalf-of lineage `sparq-prov` lacks), the invocation as a `prov:Activity` the terminal delegate
`prov:wasAssociatedWith`, and the effective grant as a `prov:Entity` `wasGeneratedBy`/`wasAttributedTo`
the delegate carrying the `auth:*` actions over its target — the §4.3 value-add of rendering the
delegated authority as RDF that reasons with the SAME `.acl` rules. **Human vs AI** is an **attested
attribute** on the delegate (§4.2: an AI agent is a distinct principal holding an attenuated child of
its human's authority) — recorded `trust:HumanPrincipal` / `trust:AiAgent` via `PrincipalClassification`.
Two honesty boundaries: (i) it is an **audit record, never an authority source** — produced *after* a
successful `invoke`, it can only ever describe authority that already passed the gate, and it never
renders a grant broader than the gate conferred (when `effective_against_current` narrows the chain to
the CURRENT delegator grant, the audit drops the revoked actions); (ii) it adds **no security property**
and no privacy — principals are named in the clear. Pure `oxrdf` (**no `sparq-prov` dependency** — that
would drag `sparq-engine` onto the crate's dep graph); OFF in the default build.

### LWS-server admission seam — opt-in `trust-graph` on `sparq-lws-core` ([OPUS-5] sq-hed3q)

The pod-side wiring above lives in `sparq-solid`'s `PodStore`. The **native LWS server**
(`sparq-lws-core`, `skills/solid-lws-server/SKILL.md`) decides access with the flat-`.acl` WAC
authorizer, which has no notion of an issuer-signed credential. `sparq_lws_core::authz::trust_admit`
(behind that crate's **own default-OFF `trust-graph` feature**, distinct from the `sparq-solid`
feature of the same name) is the adapter between the two: it runs the `sparq_trust::admit` gate and
re-expresses its output in this crate's `AccessMode`/`Decision` vocabulary.

```toml
sparq-lws-core = { path = "crates/sparq-lws-core", features = ["trust-graph"] }
```

```rust
use sparq_lws_core::authz::trust_admit::{trust_admit_verdict, union_trust_grants};
use sparq_lws_core::authz::trust_admit::{PresentedCredential, TrustRule, TrustSession};
use sparq_trust::policy::{parse_policy, ControlGate};

// Build rules ONLY from the Control-gated authoring channel. `parse_policy` demands a
// `ControlGate`, whose sole constructor `assert_control_gated()` is the caller ASSERTING it has
// verified the policy graph was authored behind `acl:Control` — a loudly-named assertion, not an
// enforced proof. `TrustRule`'s fields are public, so a caller can also build one by hand; doing
// so from anything but the Control-gated `.acr` channel is the one mistake that re-opens the
// trust root of the whole pipeline.
let rules: Vec<TrustRule> = parse_policy(&policy_triples, ControlGate::assert_control_gated())?;

let session = TrustSession { agent: requester_webid, now_unix_secs: now };
let verdict = trust_admit_verdict(&credential, &rules, &session, &target, abac_rule_n3);

// The only way a grant reaches a decision: an ADDITIVE union onto whatever WAC decided, and only
// for the (requester, resource) pair the grant was issued for — pass the CURRENT request's
// authenticated WebID (`None` when anonymous) and requested resource, which are re-checked against
// the grant's stored holder/target.
let decision = union_trust_grants(wac_decision, verdict.as_ref(), Some(&session.agent), &target);
```

`abac_rule_n3` is the controller-authored rule carried in the same Control-gated `.acr` channel,
e.g. `{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <https://pod.ex/resourceX> } .`

Load-bearing behaviour, each adversarially tested in
`crates/sparq-lws-core/tests/lws_trust_admit.rs`:

- **NOT handler-wired — enabling the feature changes no request's outcome.** `trust_admit_verdict`
  is a pure function; `ldp::handler`'s `authorize_read`/`authorize_mode` call sites are untouched,
  so nothing consults it unless *you* call it. Hot-path wiring is a separate, soundness-sensitive
  follow-up. With the feature OFF the module is not compiled and neither `sparq-solid` nor
  `sparq-trust` enters the dependency graph (strict additivity, as for `sparq-solid`'s G6).
- **Allow-only union.** `Decision::Allow(m)` gains modes and never loses any; `Decision::Forbidden`
  becomes `Allow(granted)` — the authenticated-but-unauthorized case a credential-gated resource
  exists for, and the ONLY route by which a WAC denial becomes an allow. A `TrustGrantSet` has no
  public constructor, so that route is unforgeable from outside the module.
- **Bound to the pair it was issued for.** A verdict is `Clone` and a caller may hold it across
  requests, so `union_onto`/`union_trust_grants` take the CURRENT authenticated WebID and requested
  resource and re-check them against the grant's stored `holder()`/`target()`; the raw union is
  private. On a mismatch — another resource, another authenticated agent, or none — the WAC decision
  is returned **unchanged**, so a grant minted for Jesse/`resourceX` cannot lift the denial on
  Jesse/`resourceY` or on Mallory/`resourceX`.
- **Unauthenticated is NEVER lifted.** `Decision::Unauthenticated` is returned unchanged even for a
  `Control` grant — a deliberate fail-closed narrowing: that variant means no verified WebID, so the
  credential's holder binding has nothing to bind against and a caller could otherwise name any
  `session.agent` it liked. Authenticate first (`skills/solid-lws-server/SKILL.md` § *Authenticate
  requests*), then the seam can apply.
- **Fail-closed, `None` never an error.** A forged or malformed issuer signature, a credential whose
  subject is not the authenticated requester, one outside the rule's freshness window or
  `trust:scope`, a revoked one, an unparseable `.acr` rule, or a grant derived for a different
  requester or resource each admit nothing. A rule deriving an `auth:deny*` for this requester and
  resource refuses the **whole** verdict — an allow-only union cannot express the denial, so
  honouring only its sibling allows would invert the controller's intent.
- **The combined (not static) half of the `sq-xc4y` split.** The seam is consulted per request, so it
  uses `sparq_trust::admit`, which evaluates the session-dependent class (holder binding, freshness)
  live rather than freezing it into a long-lived `<urn:sparq:auth>` view.

**Honest scope.** RESEARCH prototype, the **clear path** — the credential and the holder binding are
both in the clear. No privacy, unlinkability, anonymity, or ZK-soundness claim; the ZK estate whose
commitment/signature primitives it composes with is externally **UNAUDITED** (`sq-qhy4`, external
accredited-cryptographer sign-off pending) and the issuer key is operator-asserted. Do not read an
admitted grant as a security guarantee.

### Security-properties vocabulary — opt-in `secprop-vocab` ([OPUS-4.8] sq-5oru9, epic sq-0dksu)

The `sparq_trust::secprop` module (behind the **default-OFF `secprop-vocab`** cargo feature) is the
sparq **`sec-prop:` extension** vocabulary: the orthogonal proof-system dimensions (ZK-type,
soundness, completeness, hiding/binding, anonymity, setup, interactivity, selective disclosure,
single-use) plus the machine-reasonable **assurance / audit-status axis** that the vendored
`sec-prop:` ontology (the ISWC 2025 ZKP-SPARQL paper's vocabulary,
`crates/sparq-trust/ontologies/zkp-sparql/`) lacks — **under the same
`https://w3id.org/zkp-sparql/sec-prop#` namespace** (extend, do not fork; design
`research/security-properties-ontology-design.md` §4.1). It is **data + Rust constants only** (a
`const &str` registry + the canonical [`secprop-ext.ttl`](../../crates/sparq-secprop-vocab/ontologies/secprop-ext.ttl),
pinned together by a drift test) — it is **not** an ODRL profile or a per-method annotation graph
(those are the downstream Phase 3/4 beads `sq-bevd3`/`sq-uor3g`). Since
[#3705](https://github.com/sparq-org/sparq/issues/3705) the registry, the Turtle and that one drift
test live in the **dependency-free `sparq-secprop-vocab` leaf crate**, and `sparq_trust::secprop`
re-exports it — so the import path above is unchanged, and `sparq_policy::secprop`'s `DIM_*` and
`sparq_zk::secprop`'s `secx:` terms are now aliases of the SAME constants rather than three copies
kept in step by cross-package `include_str!` reads. The Phase 2 **N3 admissibility
reasoner** over this vocabulary now ships — see below.

The registry also names the three **vendored** (not minted) dimension IRIs the estate uses as
`secx:property` values — `SEC_PROP_POST_QUANTUM_FORGERY`, `SEC_PROP_POST_QUANTUM_SNOOPING`,
`SEC_PROP_SIGNATURE_TYPE_LEAKAGE` (paper §7.7 #3/#4/#5) — which are members of `ALL_SECPROP_IRIS`
and are re-asserted in `secprop-ext.ttl` as `sec-prop:SecurityProperty` subjects with the vendored
IRI, label and type verbatim (issue #3441). That re-assertion adds **no** type the vendored source
lacks — in particular they are **not** typed `owl:ObjectProperty` as the *minted* `secx:` dimensions
are — so the extension still mints and refines nothing on his terms (extend, do not fork). Their
purpose is to make the file self-contained for the `secx:property` range and to let the
dimension-IRI drift guards (`sq-mgxz8`) check subject presence directly. Since #3705 those guards
are ordinary crate-edge checks against `sparq_secprop_vocab::ALL_SECPROP_IRIS` — in
`sparq_policy::secprop` for the ODRL profile's dimensions, and in `sparq_zk::secprop` for the
annotation graph's — so neither reads another package's files, and the `VENDORED_SEC_PROP_DIMS`
exemption list #3441 made redundant is retired.

The **assurance axis is the honesty mechanism**: `Proven ⊐ Claimed ⊐ Conjectured`, one axis
orthogonal to every property. The **default** assurance for a sparq-asserted ZK property is
`secx:Claimed` (decision #1001 Option A) with audit status `secx:ExternalSignOffPending` — and **no
sparq ZK method may be labelled `secx:Proven` while the external accredited-cryptographer audit
(`sq-qhy4`) is open.** The vocabulary **records** a claim and its epistemic basis; it is **NOT** a
proof of any property. DPV alignment is *Light* (#1002 Option 2): `skos:closeMatch` cross-refs to
W3C DPV `CryptographicMethods` where a near-match exists, not a full regulation→requirement chain.

### `trustx:` certification-scope vocabulary — opt-in `framework-vocab` ([FABLE-5] sq-6syab.2, issue #1592)

The `sparq_trust::framework_vocab` module (behind the **default-OFF `framework-vocab`** cargo feature)
is the trust-expression program's **certification-scope layer** (design record
`research/trust-expression-spec.md` §3.4 / D4): the vocabulary a verifier uses to ask *"prove X can be
attested by unrevoked attributes issued by parties [X, Y, Z] OR by certified issuers WITHIN the eIDAS /
DIATF framework, that have only issued what they are certified to issue"* (#1592). It **extends** the
`trust:` vocabulary (shared `https://sparq.dev/ns/trust#` base — `trustx:` is a prose sub-prefix, not a
new namespace) and **`rdfs:seeAlso`s** the vendored `sec-req:` eIDAS 2.0 / UK-DVS individuals rather than
duplicating them (extend, do not fork; the design's D5). Like `secprop`, it is **data + Rust constants
only** — a `const &str` registry (`ALL_TRUSTX_IRIS`) pinned to the canonical
[`trust-framework.ttl`](../../crates/sparq-trust/ontologies/trust/trust-framework.ttl) by a byte-drift
test — **not** an evaluator (the holder-side contract evaluation is the later, `sq-3kd2g`-gated
`sq-6syab.4`).

The three surfaces the layer names: (1) the **trust requirements** (`trustx:TrustRequirements` + `question` /
`trustsIssuer` / `trustsFramework` / `requiresScopeConformance` / `requiresValidStatusAt`) — the
verifier→holder contract carrier; the trust conditions live in the requirements graph, **not** in the query (no
new query syntax); (2) the **two trust modes** — enumerated parties (`trustsIssuer`) OR
framework-certified issuers (`trustsFramework`), composing by plain OR; (3) the **certification-scope
terms** (`trustx:Certification` + `certifies` / `underFramework` / `scope` / `validFrom` / `validUntil`),
where `scope` ranges from service-level (`trustx:AnyServiceScope`, the honest DIATF granularity — DIATF
certifies a *service*, not an attribute list) down to a predicate set / SHACL shape reusing the existing
`trust:forShape` idiom. Non-revocation is a **positive**, time-windowed `trustx:StatusAttestation` —
existence of a covering window at `requiresValidStatusAt`, never evidence-of-absence (OWA/monotonicity).

**Honesty (load-bearing):** framework-anchored trust is **anchored, not proven** — a
`trustx:Certification` bottoms out in the framework operator's signed trusted-list / register artifacts
(a trust anchor, not cryptography); the scope-conformance check constrains what the *verifier accepts*
and, via the published certification, what the issuer was *authorised* to issue, but cannot retroactively
prove an issuer never mis-issued elsewhere. **No term asserts a settled cryptographic soundness or
privacy guarantee** (`sq-qhy4` external audit open; `sparq-mpc` semi-honest only). All `trustx:` IRIs are
**NON-STANDARD** placeholders a WG would rehome.

### N3 proof-admissibility ruleset — opt-in `secprop-admissibility` ([OPUS-4.8] sq-ufsi9, Phase 2)

The `sparq_trust::admissibility` module (behind the **default-OFF `secprop-admissibility`** feature,
which enables `secprop-vocab`) is the §4.3 **ODRL → admissible-proof-set reduction** run as a
genuinely **runnable Notation3 ruleset** on `sparq-reason`'s N3 engine
([`reason_n3_terms`](../inference/SKILL.md)) — not pseudocode. Two parts (design §4.3.2):

1. a **level-order fact base** — each dimension's `⊐` order as `secx:strongerThan` facts,
   transitively closed by one N3 rule, then lifted to the reflexive `secx:atLeast` partial order
   (`LEVEL_ORDERS` + `CLOSURE_RULES`);
2. a **discharge rule** — method `M` satisfies a `gteq` constraint `C` iff `M`'s asserted level for
   `C`'s dimension is `secx:atLeast` `C`'s required level (`DISCHARGE_RULE`); the leftOperand→dimension
   map is one `secx:overDimension` fact per leftOperand.

```rust
use sparq_trust::admissible;                              // --features secprop-admissibility
let a = admissible(method_iri, &constraint_iris, policy_n3, method_annotations_n3)?;
assert!(!a.admissible);  a.unsatisfied;                   // the constraints M fails (honest "why not")
```

The reasoner is **monotone** (it only derives `secx:satisfies`); the **"satisfies every constraint"**
universal (default-deny) is a **Rust side-condition** over the materialised facts (`admissible`), *not*
in-reasoner negation — the same discipline as the `admit` gate's freshness/holder side-conditions, and
for the same soundness reason. It reasons over **annotations, not crypto**: `requiresAssurance gteq
secx:Proven` mechanically **empties** the admissible set for the whole sparq ZK estate today (`sq-qhy4`
open), and `requiresPostQuantumForgery gteq …` removes the DL-signature methods — the value is a
**principled refusal** ("no admissible proof") over silently serving a non-conforming one. The §4.3.3
worked example is the golden test: empty under Alice's strict preference, non-empty under the relaxed one.

### Property-admissibility PRE-CHECK in the admission gate — opt-in `secprop-precheck` ([OPUS-4.8] sq-dt5hv Phase 5 + sq-nrwqs Phase 5.1, design §5b)

`sparq_trust::admit_with_precheck(cred, rules, session, target, preference)` (behind the **default-OFF
`secprop-precheck`** feature, which enables `secprop-admissibility` **and** `sparq-zk/secprop-annotations`)
wires the admissibility reduction above into the REAL admission path as an **optional pre-admission check**.
The `preference: Option<&AdmissibilityPreference>` carries the requester's machine-reasonable **ODRL privacy
preference** — the presented proof's `method_iri` (the `zk:scheme`/`zk:cryptosuite` the registry records)
and a `constraints: Vec<AdmissibilityConstraint>`, each a **structured** `gteq` constraint of two validated
IRIs (`left_operand` = a `secx:requires…` operand, `right_operand` = the required level).

**Phase 5.1 (sq-nrwqs) — bundled annotations + structured policy = tamper-resistant.** The caller supplies
**no raw N3 at all**: the method's `secprop` property annotations are resolved from the bundled, drift-pinned
`sparq-zk` `ontologies/secprop-methods.ttl` (the canonical source of truth) keyed on `method_iri`, and the
policy triples are **synthesised internally** from the structured constraints (each operand a validated
`NamedNode` emitted only as an `odrl:` object). Because there is no caller-supplied raw-N3 channel — neither
for the annotations nor for the policy — a caller **cannot** inject a `secx:hasProperty` / `secx:atLeast` /
`secx:satisfies` triple (or an N3 rule) to widen a method's recorded posture. (The Phase-5 `annotations_n3` /
`policy_n3` raw-string fields, which raw-concatenated caller text into the reasoning document, are gone — that
closed a real widening channel, not merely renamed it.) **`sq-ddbm8` defence in depth:** the operand fields are
`pub` and `NamedNode::new_unchecked` bypasses `NamedNode::new`'s RFC-3987 validation, so the synthesiser
**re-validates every operand as a well-formed IRIREF at the emission site** and **fails closed**
(`PrecheckOutcome::MalformedConstraint { operand }`) on any operand carrying a term-breaking character
(`>` / whitespace / `< " { } | ^` `` ` `` `\`) that could otherwise break out of its `<…>` object term — so the
escape channel is structurally absent even off the `NamedNode::new` path. An **unknown method** (no bundled block) fails closed
with `PrecheckOutcome::UnknownMethod { method }` — regardless of the (possibly empty) constraint set. Only
`secx:QueryProofLayer` assertions are used (the §5a non-transfer rule); the method-wide `secx:AssuranceLevel`
the `requiresAssurance` dimension reads is DERIVED as the **weakest** assurance across the method's positive
query-proof claims (so every sparq method is `Claimed` while `sq-qhy4` is open, and settled-NEGATIVE Proven
levels like `PQForgeable` never inflate it). This hardens the input trust boundary; it makes **no** new
soundness/privacy claim.

Before the existing signature / freshness / holder checks, the gate synthesises the policy from the structured
constraints and calls `admissible` over the method IRI, that policy, and the **bundled** annotation graph, and
**fails closed** when the method does not satisfy every constraint — returning an EMPTY admitted set + a
`PrecheckOutcome::{Admitted, UnknownMethod { method }, Denied { unsatisfied }, ReductionError { error },
MalformedConstraint { operand }}` (an unknown method / an unsatisfied constraint / a reduction error / a
non-IRIREF-safe operand are ALL fail-closed denials — a pre-check that cannot be evaluated never admits;
`Denied.unsatisfied` lists the failed constraints' `left_operand` dimensions; `MalformedConstraint` is the
`sq-ddbm8` input-boundary guard above). **OPT-IN strict additivity:** with `preference == None` it is **byte-identical** to
`admit` (no reasoning runs); the pre-check can only ever **DENY**, never broaden, and never weakens a
downstream crypto check (`admit` still verifies the checked issuer signature, scope, freshness, and holder
binding). The golden e2e invariant (`tests/secprop_precheck_e2e.rs`): a perfectly valid credential is
admitted with `None`, under a relaxed preference, and under a `requiresSoundness gteq secx:KnowledgeSound`
preference satisfied purely from the bundled graph — but `requiresAssurance gteq secx:Proven` (Alice's
strict preference, every sparq ZK method is `Claimed`-only while `sq-qhy4` is open) **admits nothing and
derives no grant** — the principled refusal, in the data flow, not just the prose. Research-grade, externally
**unaudited** (`sq-qhy4`); it reasons over recorded ANNOTATIONS, not cryptography.

### Live status / revocation + minimal denial justification — opt-in `status-list` ([OPUS-4.8] sq-pfae.7, design §6.1 P6)

The `sparq_trust::status_list` module (behind the **default-OFF `status-list`** cargo feature) gates
admission on a **live W3C Bitstring Status List** instead of the PoC's per-credential `revoked: bool`
flag. A credential's `StatusListEntry` (`{ status_list_credential, status_list_index, purpose }`) is
checked by a `LiveStatusCheck::new(resolver, decoder, max_age_secs)` — a **pluggable**
`StatusListResolver` (the network seam; the crate ships no HTTP client) + a **pluggable** `GzipDecoder`
(the compression seam; the multibase base64url/base58btc decode is hand-rolled and dependency-free, the
built-in `Flate2GzipDecoder` lives behind the nested `status-list-flate2` feature that pulls `flate2`).
`LiveStatusCheck::check(&entry, now)` returns a `LiveStatus`, and `LiveStatus::admits()` is `true` for
`LiveStatus::Live` **only** — **fail-closed on `Set` (revoked), `Unknown` (unresolvable / undecodable /
out-of-range), and `Stale` (snapshot older than `max_age_secs`)**, exactly as the bead requires. The
bitstring is MSB-first per the spec (`StatusBitstring`), distinct from the LSB-first
`sparq-zk-compose::StatusListSnapshot` ZK mirror — this is the **clear-index** gate, not the hidden-index
ZK proof. `sparq_trust::admit_with_status(cred, rules, session, target, &entry, &check)` wires the gate
into the REAL admission path: it runs the live-status check FIRST (deny the whole credential on a
non-admitting status) and otherwise delegates to the unchanged `admit` — strictly additive (it can only
NARROW). `justify_status_decision(grant, &entry, status, at_time)` renders a **minimal PROV-O
justification** for the allow OR the deny (a `prov:Activity` typed `trust:StatusCheck`, `prov:generated`
the grant, with `trust:statusDecision` = the reason token + the checked index/purpose) — the bead's
*minimal denial justification*. Revocation propagates by **full re-materialise** (the §4.4 stale window
is *bounded* by `max_age_secs`, not closed — no in-reasoner incremental retraction; see the
`StatusDelta` paragraph below for the bounded incremental *selection* step). Pure-Rust base layer
(no new default dep); OFF in the default build. One honesty boundary remains: it adds **no privacy** (the
index + list are clear; the resolver learns which credential is checked).

**Incremental status delta — re-check only the affected grants** (`StatusDelta`, same `status-list`
feature, [OPUS-5] `sq-pfae.14`). Retraction is still re-materialisation, but an **epoch bump** no longer
has to re-run the gate over every grant. `StatusDelta::between(list_iri, &prev, &next)` (or
`between_with_limit(.., max_changed_indices)`; the default cap is `DEFAULT_MAX_CHANGED_INDICES`) diffs two
`StatusBitstring` snapshots of ONE list and names the slots that moved: `changed_indices()`,
`requires_full_recheck()`, `unbounded_reason()` (a `DeltaUnbounded`), `successor_as_of_unix_secs()`, and
the two predicates that form the **skip contract** — skip an entry's re-check only when
`delta.valid_at(now, max_age_secs) && !delta.affects(&entry)`. `affects` is the per-slot bit selection;
`valid_at` is the **whole-list** freshness precondition, and it is load-bearing rather than decorative:
staleness is a property of the *snapshot*, not of a slot, so a future-dated successor moves an unchanged
slot from `Live` to `Stale` without `affects` ever seeing it. Under that contract a skipped entry
satisfies `verdict(prev).admits() ⇒ verdict(next).admits()`, i.e. skipping can never leave a grant
admitted that the full re-run would deny (the stronger `verdict(prev) == verdict(next)` holds when both
snapshots are fresh). **Why this does not break stratification:** it is a *selection* computed from two **input**
snapshots — it derives no verdict, reads no derived fact, and retracts nothing in the reasoner, so the
input-stratified / one-side-bound seeding discipline (`sq-tu4e`: no NAF over derived facts) is untouched.
**Fail-closed:** a successor that is not strictly newer (`NotNewer`), a change in the list's bit coverage
(`CoverageChanged` — indices moving in/out of range flip to/from `Unknown`), or more changes than the
limit (`LimitExceeded`) all yield `requires_full_recheck()`, i.e. the pre-`sq-pfae.14` full re-run, and
`affects` then reports **every** entry on the list as affected. **Honest residue:** the delta does NOT
close the §4.4 stale-authority window — an entry whose bit did not change still ages into
`LiveStatus::Stale` and `affects` will say `false` for it, so `valid_at` (and the caller's own `max_age_secs`-driven
re-check) is still required; a delta binds exactly one `statusListCredential`; and deep-chain delegation
revocation is still not incremental. The skip contract is pinned by an exhaustive differential test over
every single-byte snapshot pair × a `now` sweep covering both-fresh, prev-aged-out and future-successor
instants (`status_list::tests::unaffected_entries_are_safe_to_skip_across_the_bump`), plus
`valid_at_rejects_a_future_dated_successor_that_affects_cannot_see`.

**Verified status-list issuer signature** (`VerifyingLiveStatusCheck`, same `status-list` feature,
[OPUS-4.8] `sq-pfae.13`). The base `LiveStatusCheck` trusts the list AS FETCHED. `VerifyingLiveStatusCheck`
closes that gap: it resolves the status-list VC as a **signed graph** (a `SignedStatusList` `{ graph,
issuer_signature_hex, salt }` over a pluggable `VerifiedStatusListResolver`), and **before** trusting any
bit, verifies the list's OWN issuer signature over its RDFC-1.0 commitment — the SAME
`commit_triples → commitment_message → verify` Schnorr-over-RDFC-1.0 path `admit` uses — against a
**trusted status-authority key**. Only on a valid signature does it read the `status:encodedList` from the
*verified* graph and run the identical freshness + bit logic. **Fail-closed**: an unsigned / bad-signature /
wrong-key / unresolvable-issuer list VC, or a verified graph with no `encodedList`, all yield
`LiveStatus::Unknown` (deny) — never trusted. The trusted key is caller-supplied, or bound from a
status-authority `did:key`/`did:web` issuer DID via `VerifyingLiveStatusCheck::with_did_issuer(.., did_resolver,
authority_did, ..)` (the `did` feature, same binding the admission gate uses). Research-grade, externally
**UNAUDITED** (`sq-qhy4`): a verified issuer signature, NOT a privacy/unlinkability guarantee.

## Pattern-scoped masking spike — [FABLE-5] sq-lrtc3.3 (opt-in `pattern-scope`)

Sub-named-graph (triple-pattern / row-level) result masking: an ODRL-style target as a
set of **allow/deny triple patterns** over a source graph, so a session can be granted
*"the contacts graph except phone numbers"*. Design record (options analysis, ODRL
vocabulary, production caching path, follow-up beads):
`research/odrl-pattern-scoped-targets-2026-07.md`. Measured envelope:
`bench/pattern-scope/` (work-box JSONs, non-canonical).

**Mechanism — masked-subgraph materialization, NOT per-scan filtering.** A scoped graph
is decoded, filtered, and rebuilt as a physical replica; the engine evaluates a dataset
in which masked triples are **physically absent**, so equivalence with an oracle store
(the source with those triples deleted) under SELECT / `OPTIONAL` / `EXISTS` / `MINUS` /
aggregates / `COUNT` / `ASK` / `GRAPH ?g` holds **by construction** (differential
battery: `crates/sparq-solid/tests/pattern_scope.rs`). Zero engine/core changes; the
graph-granular enforcement walk is reused unchanged.

- `ScopePattern::new(s, p, o)` / `::any()` — per-triple predicate, `None` = wildcard,
  **term-identity** matching (no join variables, no value equality).
- `GraphScope::allow_only(patterns)` (permission shape) / `::deny_within(patterns)`
  (prohibition carving a hole); a triple is visible iff it matches ≥1 allow AND 0 deny
  (**deny overrides allow**; empty allow grants nothing — fail-closed).
- `masked_graph(&Graph, &GraphScope) -> Graph`; `masked_dataset(&Graph, &decisions)`
  (explicit-decision assembly: absent from the map = absent from the dataset; a fully
  masked graph is **omitted** — indistinguishable from absent).
- `store.scoped_dataset(&Session, Mode, &scopes) -> ScopedDataset` — **refinement-only**
  (a scope can only shrink the session's graph-level accessible set, never widen), then
  `ScopedDataset::{query, query_json, ask, view}` on the same `wrap_read`/empty-default
  path as `query_as`. Build once per (session × scopes), query many; rebuild after any
  store mutation. READ path only (UPDATE stays graph-granular, record §2.4).

## Related skills

- [`http-server`](../http-server/SKILL.md) — the sparq SPARQL HTTP server has **no**
  per-user authz of its own; front it with `sparq-solid` (or a gateway) for that.
- [`usage-control-policy`](../usage-control-policy/SKILL.md) — `sparq-policy` (W3C ODRL
  2.2) is the **usage-control** layer *above* this access-control layer: where
  `sparq-solid` answers "may this agent read graph G?", `sparq-policy` answers "may this
  party *use* this asset for purpose P, with obligation O, until time T?".
- [`inference`](../inference/SKILL.md) — `sparq-reason` runs the N3 rules that
  materialize the auth view.

_(status: RESEARCH / architecture track. The crate is `shipped` per the design record
(`research/solid-access-control-design.md`) — the zero-copy `DatasetView` query path and
the write-gating path are implemented and tested (`tests/update.rs` + the crate suite) —
but the SCOPE is the graph-level authorization layer ONLY: it does NOT authenticate (the
WebID, client, and `acp:issuer` are all caller-asserted — the issuer dimension matches a
presented credential issuer, it does NOT add WebID-OIDC authentication) and it is NOT a
Solid Pod HTTP server (no HTTP/LDP/OIDC). Verified against
`crates/sparq-solid/src/{lib,authindex,fixture}.rs` + README on branch
`feat/sq-3jtd-acp-issuer` (2026-06-16; the (agent, client, issuer) three-dimension
principal model, sq-3jtd.6); workspace v0.1.0, opt-in (depends on nothing in
the workspace), native-side. Conservative WAC/ACP sub-cases are noted in design §4.4; perf
is measured by `cargo run -p sparq-solid --example bench` and NOT baked into docs. Code
carries [OPUS-4.8] review markers pending re-review.)_
