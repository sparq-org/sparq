<!-- [OPUS-5] sq-gg0qq.10: the durable in-repo home for the imported LWS server's
     design/decision estate. Reconstructed from in-tree code, not copied. -->
# LWS server design records — the migrated `decisions/` + `docs/design/` estate (sq-gg0qq.10)

> **Status: DESIGN RECORD (migration + reconstruction).** `crates/sparq-lws-core` was
> imported whole from [jeswr/solid-server-rs](https://github.com/jeswr/solid-server-rs)
> at rev `1e555b10` under `sq-gg0qq.2`. That import deliberately left the source repo's
> `docs/design/` and `decisions/` trees behind, so ~70 doc-comments in the imported code
> cited paths that **do not exist in this checkout**. This record is the durable in-repo
> home those citations resolve to; #4970 re-pointed them at the sections below.
>
> Bead: **sq-gg0qq.10** · Issue: **#2742** · Parent: **#2572** / `sq-gg0qq`.
> Siblings: `.1` supply-chain pre-flight, `.2` the crate import, `.3` the in-workspace
> embedded-engine binding, `.4` the 3-crate split decision package
> ([`lws-3-crate-split.md`](./lws-3-crate-split.md)), `.5` the WAC-bypass fix,
> `.7` the conformance lane.
>
> **What this record is, precisely.** The original ADR prose is in another repository and
> was **not** available to this pass — nothing here is a verbatim copy of it. Every
> decision below is **reconstructed from the in-tree code that implements it**, and every
> claim carries a `file:line` citation you can check. Where the code does not state a
> rationale, this record says so rather than inventing one. Treat the code as normative
> and this record as the map; where the two disagree, **the code wins and this record is
> the bug**.
>
> **No timings appear in this document.** Three of the migrated source documents are
> throughput/latency design notes; §7 explains why they are pointer-only rather than
> reconstructed, and the repo's no-hard-coded-perf-numbers rule is why.

---

## 1. The reference map — source-repo path → in-repo home

Every path in the left column was cited by an imported doc-comment (pre-#4970) and **does
not exist in this repository**. The right column is where the load-bearing content actually
lives now, and is what those doc-comments cite today.

| Cited path (source repo) | Namespace | Subject | Durable in-repo home |
|---|---|---|---|
| `decisions/0001-embed-sparq-in-process.md` | RSS | in-process engine backend | §3 below · `crates/sparq-lws-core/src/store/embedded.rs` |
| `decisions/0002` | RSS | pre-crypto public-read skip | §5 below · `crates/sparq-lws-core/src/ldp/public_read_skip.rs` |
| `decisions/0003` | RSS | existence-non-disclosure V2/V4/V5/V6 | §6 below · `crates/sparq-lws-core/src/ldp/handler.rs` |
| `docs/design/webid-outside-pod.md` | RSS | provider WebIDs on an identity host | §4 below · `crates/sparq-lws-core/src/identity.rs` |
| `docs/design/backend-read-path.md` | RSS | read-path round-trip budget | §7 · `src/store/counting.rs`, `tests/read_path_counters.rs` |
| `docs/design/beyond-50k-throughput.md` | RSS | transport-level throughput work | §7 · `src/tls.rs`, `tests/tcp_nodelay.rs` |
| `docs/design/high-throughput-pop-auth.md` | RSS | tiered proof-of-possession | §7 · `src/pop/` module docs |
| `docs/design/throughput-hard-cases.md` | RSS | blob body cache | §7 · `src/store/body_cache.rs` |
| `decisions/0020-webid-outside-pod.md` | **PSS** | upstream WebID-host convention | upstream; adapted by §4 |
| `decisions/0001-foundational-architecture.md` | **PSS** | upstream architecture | upstream; cited by [`concurrent-serving.md`](./concurrent-serving.md) |
| `decisions/0003-qlever-live-update.md` | **PSS** | upstream live-update study | upstream; cited by [`concurrent-serving.md`](./concurrent-serving.md) |
| `decisions/0012` | **PSS** | upstream serving contract | upstream; cited by the `sq-kb` ingest corpus |

## 2. The two ADR namespaces — a real collision, read the number with its repo

The imported code cites **two different, independently-numbered `decisions/` trees**, and
the numbers overlap:

- **RSS** = `jeswr/solid-server-rs`, the Rust server this crate was imported from. Its
  `decisions/0001` is *embed sparq in-process*.
- **PSS** = `prod-solid-server`, the upstream TypeScript Solid server. Its `decisions/0001`
  is *foundational architecture*, and its `decisions/0020` is the WebID-host convention
  that RSS adapted.

So `decisions/0001` means two unrelated things depending on which repo the citing comment
belongs to. Since **#4970** the crate's doc-comments cite *this record's* section numbers
instead, and the ones that still name a source-repo path carry an explicit `RSS`/`PSS`
prefix — e.g. `src/identity.rs:4-6`, `src/lib.rs:86-87`, `src/main.rs:174-175`,
`src/ldp/public_read_skip.rs:32`, `src/tls.rs:57`, and the `decisions/0003` defining sites
in `src/ldp/handler.rs`. So no bare, ambiguous `decisions/NNNN` remains under
`crates/sparq-lws-core/**`. A citation inside `research/concurrent-serving.md` is PSS.

**Rule for future edits:** when you touch one of these citations, write the namespace into
it (`RSS decisions/0003`, `PSS decisions/0020`). Bare `decisions/NNNN` is ambiguous.

## 3. ADR — embed SPARQ in-process (RSS `decisions/0001-embed-sparq-in-process.md`)

**Decision.** The authoritative-RDF seam (`SparqClient`) is implemented by calling the
`sparq-engine` query/update entry points **directly against an in-process `Graph`**
(`sparq-core`), rather than over SPARQ's HTTP service. It is selected at boot by
`PSS_SPARQ_BACKEND=embedded`, compiled behind the `embedded-sparq` cargo feature, which is
**on by default** since `sq-gg0qq.3` (`src/store/embedded.rs:2-12`; `Cargo.toml:356-368`).
`--no-default-features` builds the engine-free profile.

**Backends and their gates** (`src/main.rs:214-226`):

| `PSS_SPARQ_BACKEND` | Cargo feature | Default? | Durability |
|---|---|---|---|
| `memory` | none | the **boot** default | ephemeral in-memory double |
| `embedded` | `embedded-sparq` | feature on by default; **not** the boot default | see below |
| `http` | `http-sparq` | off | the external service's |

Note the deliberate split between *compiled in* and *selected*: `embedded-sparq` is a
default-**on** build feature, but runtime selection "stays EXPLICIT via this variable
(fail-safe: an unconfigured boot serves the ephemeral in-memory double, never a store an
operator did not choose)" (`src/main.rs:220-226`; `Cargo.toml:362-366`). A default build
that is never configured therefore serves the in-memory double, not the engine.

**Rationale, as stated in the code** (`src/store/embedded.rs:14-30`) — three points, none
of them a performance claim:

1. **Same queries, different transport.** Every query/update is built by the *same*
   injection-safe builders in `store::sparql` that the HTTP client uses, "VERBATIM, no new
   query strings", so conformance-equivalence to the HTTP and in-memory impls is trivial:
   identical SPARQL, identical named-graph model, only the execution path differs.
2. **Named-graph isolation is real here.** The engine's `query`/`update_in_place` support
   `GRAPH <g> { … }` over a single `Graph` holding the default graph plus named graphs
   (graph IRI == resource IRI — the WAC-design model). The code records that the live HTTP
   `sparq-server` "today folds named graphs into one default graph", which the comment
   labels the HTTP path's `DEVIATION-1`; embedding sidesteps it.
3. **No marker/follow-up-ASK atomicity dance.** Over HTTP a SPARQL UPDATE cannot return
   rows, so the outcome must be probed by a follow-up ASK against per-operation markers —
   racing a concurrent mutation. In-process the whole operation "runs as ONE indivisible
   actor job on the `Graph`'s owning thread", so `create_child`/`delete_meta_if_empty`
   return their outcome directly (check-then-act with no interleaving).

**Durability — the honest posture.** `embedded` is durable only with a persistence
directory. `SOLID_SERVER_SPARQ_DIR` set ⇒ a directory-backed graph (durable);
unset ⇒ a fresh in-memory graph (ephemeral) (`src/main.rs:234-256`;
`src/store/embedded.rs:67-74`). Separately, the native binary's **blob** backend is
in-memory, so a durable RDF index does not by itself make resource bodies durable — the
crate README and `skills/solid-lws-server/SKILL.md` both say this, and it must stay said.

## 4. ADR — provider WebIDs outside the pod (RSS `docs/design/webid-outside-pod.md`)

**Decision.** A provider-issued WebID is served from a separate **identity host**, never as
an in-pod LDP resource. The WebID form is `https://<identity-host>/<handle>#me`, with the
document at `https://<identity-host>/<handle>`; the default host is derived as
`id.<base authority>` rather than hard-coded, so the server stays deployment-agnostic
(`src/identity.rs:11-12`, `78-102`).

**Rationale — quoted verbatim** (`src/identity.rs:5-9`):

> The WebID document is the Solid-OIDC identity trust root: every resource server on the
> web dereferences it to learn which issuers may mint tokens for that WebID
> (`solid:oidcIssuer`). Hosting it INSIDE the pod — a WAC-governed, owner-writable
> resource — leaves it one over-broad `acl:default` grant away from ecosystem-wide identity
> takeover.

**Mechanism.** Id-docs are stored under the reserved internal namespace
`<base>/.identity/<handle>`, written with no containment edge, so they appear in no
`ldp:contains` listing and are addressable only by the id-host route (`src/seed.rs:59-76`).
The LDP surface refuses `/.identity/**` outright — `404` for every method, `%`-decoded too,
**regardless of the identity feature flag** — so no `.acl` can ever exist for the namespace
and no WAC grant can ever apply to an id-doc (`src/ldp/target.rs:45-51`). That refusal is
enforced twice on purpose: the identity gate middleware refuses it first (outermost), and
`parse_target` is the belt-and-braces chokepoint covering every handler
(`src/identity.rs:238-250`). The seed comment states the consequence plainly — no `.acl` is
written for an id-doc because none *can* exist, and that is "the security property, not an
omission" (`src/seed.rs:59-76`).

**"LOCKED" id-doc.** The served document carries exactly the provider-controlled
statements: the Person type, the locked `solid:oidcIssuer`, `pim:storage` → the pod root,
the `<pod> solid:owner <webid>` back-link, and `rdfs:seeAlso` → the in-pod card
(`src/seed.rs:239-244`, `272-275`). The in-pod `/{u}/profile/card` is correspondingly
demoted to a user-editable extended profile carrying **neither** `solid:oidcIssuer` **nor**
`pim:storage` (`src/seed.rs:64-72`).

**Default posture.** `SOLID_SERVER_IDENTITY_ENABLE` is **default OFF**
(`src/main.rs:174-182`). Turning it on makes the identity gate serve id-docs on the id host
(`GET`/`HEAD` only, Turtle/JSON-LD, no WAC, no `.acl` Link) and makes the conformance seed
mint id-host WebIDs instead of in-pod cards. The unconditional LDP refusal of
`/.identity/**` holds either way, so that pre-seeded documents can never become
LDP-addressable — and thus `.acl`-able — when the flag is later turned on.

## 5. ADR — the pre-crypto public-read skip (RSS `decisions/0002`, "opt 3")

**Decision.** For a read whose response is fully identity-independent, a thin middleware
runs the existing anonymous WAC predicate *before* the auth layer and serves the read
directly (`src/ldp/public_read_skip.rs:2-8`). It is wired unconditionally into the router
layer stack (`src/app.rs:288-303`) — there is no feature flag.

**The hard scope limit, quoted verbatim** (`src/ldp/public_read_skip.rs:10-12`) — this is
the load-bearing, security-critical part:

> The skip fires **only for a GET/HEAD that carries NEITHER an `Authorization` NOR a `DPoP`
> header.** A request that carries credentials (or a `DPoP` proof header) is NEVER
> short-circuited — it falls through to the full auth path.

Two independent reasons are recorded, both pinned by the WAC-Allow conformance suite
(`src/ldp/public_read_skip.rs:18-28`): `WAC-Allow user=` is identity-**dependent**, so
serving an authenticated owner's public read as anonymous would under-report their access —
a wrong, observable response; and a forged proof is indistinguishable from a legitimate
owner's without verifying it, so the only way to serve forged proofs as harmless-anonymous
would be to serve *every* proof-carrying public read as anonymous, which is exactly the
first error. `tests/public_read_skip.rs:5-6` restates the limit as the property under test.

**Do not oversell this one.** The module docstring is explicit that because there is no
proof to verify on such a request, the win is "a marginal one (one fewer public-token
construction + an earlier WAC pass), **NOT a crypto saving**"
(`src/ldp/public_read_skip.rs:6-8`). The nickname "skip-crypto" overstates it; the code
already corrects itself, and any doc that repeats the nickname should carry the correction.

**Not reconstructable.** The source ADR also recorded options 1 and 2, and why applying
option 3 to *credentialed* reads is unsafe in full. The code names them
(`src/ldp/public_read_skip.rs:30-31`) but does not restate them, and they are **not stated
in the code** — so they are not reconstructed here.

## 6. ADR — existence-non-disclosure (RSS `decisions/0003`, variants V2/V4/V5/V6)

A family of closures over one invariant: **a requester who lacks the read-mode on a target
must not be able to distinguish "exists but forbidden" from "does not exist"** — the read-
mode being `acl:Control` for an `.acl` target and `acl:Read` otherwise. The exhaustive
byte-identical matrix is the top-level pin: `404` is served only to a requester who holds
the operation's required mode; every other requester gets their own denial code (`401`
anonymous / `403` authenticated) for **both** the forbidden-existing and the not-found case,
byte-identically (`src/ldp/handler.rs:3780`).

| Variant | Channel closed | Mechanism | Guard / pin |
|---|---|---|---|
| **V2** | the minted `Location` header on POST | the mint **always** appends the opaque suffix, so the shape is collision-independent — it never varies on whether the slug was free — while still *containing* the slug so `post-uri-assignment-slug` conformance holds; a slug-less POST falls back to an identically-shaped default stem | `mint_child_iri` (`handler.rs:2200-2216`) |
| **V4** | conditional requests carrying a **concrete** entity-tag validator (`If-Match: "x"` / `If-None-Match: "x"`) | such a validator is content- or membership-derived, so a non-reader carrying one gets their denial code **instead of** the precondition being evaluated — decided *before* the existence probe, so neither the `412`-vs-`2xx` outcome nor any `ETag` is observable. A bare **`*`** (`If-None-Match: *` safe-create / `If-Match: *` lost-update) is deliberately **EXEMPT**: it carries no ETag fingerprint and tests only existence, which a holder of the operation's required mode already learns from the unconditional `201`-vs-`204` write split — so Read-gating it broke the standard `PUT … If-None-Match: *` create pattern for a Write-without-Read holder at zero non-disclosure gain | `guard_conditional_requires_read` (`handler.rs:602-657`), wildcard split via `conditional_carries_etag`, rationale at `handler.rs:631-640`; called from PUT (`:1125`), DELETE (`:1443`), PATCH (`:1615`) |
| **V5** | the container `ETag` | the membership-derived container `ETag` shifts on every child add/remove, so it is a listing oracle; it is emitted only on the `acl:Read`-gated GET/HEAD path, and its conditional-request sibling is closed by V4 | structural — no separate guard (`handler.rs:894-901`) |
| **V6** | the POST `404`-vs-`405` existence branch | an agent holding `acl:default acl:Append` but no `acl:Read` could otherwise probe descendant existence; a non-reader gets their denial code instead of the branch, again *before* the existence probe | `guard_post_existence_requires_read` (`handler.rs:659-679`, `701-720`), called at POST (`:1261`, `:1285`, `:1291`) |

**The residual disclosure is documented, not closed — do not describe this family as
total.** The code records an explicit WAC-inherent exception: a requester holding `acl:Read`
through inheritance who is separately denied on one existing child by *that child's own*
restrictive `.acl` can still distinguish that child (`403`) from a missing one (`404`),
"this is WAC-inherent (a per-child `.acl` legitimately overrides inheritance) and documented
in decisions/0003" (`handler.rs:697-700`). Any doc summarising V2/V4/V5/V6 must carry that
caveat rather than claim existence is never disclosed.

**The lock-step note is load-bearing.** V6 computes the read-mode "EXACTLY [as]
`guard_conditional_requires_read` computes [it], kept in lock-step so the two
existence-disclosure gates cannot drift" (`handler.rs:677-678`). V5 carries the matching
forward-looking caveat: it is enforced by *placement* rather than by a guard, so "if a
future change emits a container's representation ETag outside a Read-gated path, that gate
must be re-established there too" (`handler.rs:894-901`). Both are the kind of invariant
that survives only if it is written down — which is the reason this section exists.

## 7. The four `docs/design/*.md` performance records — pointer-only, on purpose

`backend-read-path.md`, `beyond-50k-throughput.md`, `high-throughput-pop-auth.md`, and
`throughput-hard-cases.md` are throughput/latency design notes. They are **not**
reconstructed here, for two reasons, and the omission is deliberate rather than an
oversight:

1. **The repo forbids hard-coded performance numbers in markdown** (`AGENTS.md`;
   `scripts/check-no-perf-numbers.py`). The canonical homes for figures are the bench
   registry and the generated dashboard. A reconstruction of a throughput design note that
   *omitted* its numbers would be a shell; one that included them would be a gate
   violation, and — worse — an unreproducible number quoted from a document this pass
   never read.
2. **Their load-bearing output is already in-tree as executable pins, not prose.** The
   round-trip budgets are asserted by counting decorators and deterministic counter tests
   (`src/store/counting.rs`, `tests/read_path_counters.rs`, `tests/write_path_counters.rs`,
   `tests/embedded_read_counters.rs`); the transport knobs and their opt-in/default-off
   posture are documented on the items themselves (`src/tls.rs`); the tiered
   proof-of-possession design is documented across the `src/pop/` module docs; the blob body
   cache is documented on `src/store/body_cache.rs`. A test that fails when the count
   changes is a stronger record than a paragraph asserting the count.

If a figure from one of those documents is ever needed, it must be **re-measured in this
repo** through the bench harness — not quoted from the source repo.

## 8. The maintainer spec estate — the specs are the contract

Several behaviours of this crate are not sparq's to define: they implement an external
normative specification, and the spec — not this implementation — is the contract. When the
two disagree, **the spec is right and the code is the bug**; when the spec moves, the pinned
reference moves with it in the same change.

Verified in-tree members of that estate:

| Spec / dependency | Pinned reference | What depends on it | Evidence |
|---|---|---|---|
| **DPoP-SK** — the sender-key DPoP profile (<https://jeswr.github.io/dpop-sk-spec/>) | cited by section; the spec's Appendix-A worked example is executed as a test vector | `src/pop/sk/**` (HKDF/HMAC DPoP-SK attestation) | `src/pop/sk/mod.rs:4-5`; `src/pop/sk/derive.rs:5`, `:162` |
| **solid-oidc-verifier** (<https://github.com/jeswr/solid-oidc-verifier>) | git dependency at rev `89c896249a726398b78302fd2f65eef0a82af681`, `network` feature | baseline (cache-miss) Solid-OIDC access-token + DPoP proof verification; `JwksProvider` / `ReplayStore` | `Cargo.toml:230`; `src/auth.rs:5`; `src/lib.rs:15` |
| **solid-oidc-verifier**, cache-hit path | same pin, used through the verifier's *public* primitives (`verify_proof_with_embedded_jwk`, `Jwk::thumbprint_sha256`, `proof_has_ath`, `peek_claims`) + the SHARED `ReplayStore` | on a verified-token-cache hit the cached token is reused, and `src/auth_cache.rs` re-verifies the fresh proof itself: signature, `htm`/`htu`/`iat`, `ath == H(token)`, the `jti` replay mark, and the `cnf.jkt` binding | `src/auth_cache.rs:13-31`, `:55-57` |
| **lws-acp** (<https://github.com/jeswr/lws-acp/tree/main/docs>) | consulted as prior art | the ACP/trust-graph authorization design | [`solid-trust-graph-authz-design.md`](./solid-trust-graph-authz-design.md) §; [`security-properties-ontology-design.md`](./security-properties-ontology-design.md) |

The DPoP-SK row is the pattern worth copying: `src/pop/sk/derive.rs:162` runs the *spec's
own* execution-verified Appendix-A worked example as a unit-test vector, so a drift between
the spec's arithmetic and this implementation's fails a test rather than sitting silent.
That is what "the specs are the contract" should mean mechanically — a pinned reference plus
an executed vector, not a prose citation.

**Unresolved, stated honestly.** The `sq-gg0qq.10` bead also names **`lws-spec`** and
**`lws-ucs`** as members of this estate. Neither name appears anywhere in this
repository — not in code, docs, `research/`, or `.beads/issues.jsonl` — so this pass has
**no in-tree evidence of their location or content** and does not link them rather than
guess a URL. Resolving them (and, if they are normative for shipped behaviour, pinning them
the way DPoP-SK is pinned) is follow-up work, not something this record can assert.

## 9. What this record does not do

- **It is not a verbatim migration.** The source ADRs were not read. Everything above is
  reconstructed from code, and §5 flags the one place where the source ADR provably holds
  content (options 1 and 2) that the code does not restate.
- **It is not itself the reason the citations resolve.** The ~70 imported doc-comments that
  cited `docs/design/…` / `decisions/…` were re-pointed at this record's sections under
  **#4970**, a separate change; §1 remains the source-path → in-repo map, and §2 the
  namespace rule any future citation must follow.
- **It claims no capability.** Every behaviour above is stated with its default posture
  (`embedded-sparq` on, `SOLID_SERVER_IDENTITY_ENABLE` off, `http-sparq` off) and its
  durability caveat. `sparq-lws-core` remains **EXPERIMENTAL**, is `publish = false`, and
  does not replace the TypeScript prod-solid-server.
