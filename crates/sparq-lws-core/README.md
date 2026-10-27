<!-- sq-gg0qq.2: imported from jeswr/solid-server-rs@1e555b10 (see epic sq-gg0qq). -->
# sparq-lws-core

> **EXPERIMENTAL** — a parallel-track Rust implementation of a Solid/LDP (Linked Web
> Storage) server. It does **not** replace the TypeScript prod-solid-server. Imported
> whole from [jeswr/solid-server-rs](https://github.com/jeswr/solid-server-rs)
> (rev `1e555b10`); the 3-way lws-core / lws / solid-server split is a later bead.

A Solid/LDP server core: SPARQ-authoritative for RDF and access control (WAC),
`object_store`/S3 backup-only for bytes. Solid-OIDC + DPoP auth (via the pinned
[solid-oidc-verifier](https://github.com/jeswr/solid-oidc-verifier)), tiered
proof-of-possession (mTLS cert-bound tokens, DPoP-SK), notifications
(WebSocketChannel2023), and a DoS-hardened hyper/axum transport.

## 🚀 Quickstart

```bash
# Build + run the server binary (in-memory store, plain TCP)
cargo run -p sparq-lws-core
# Full test suite (default features — includes the in-process engine backend)
cargo test -p sparq-lws-core
# Core Solid tier: engine-free and `/sparql` compiled out
cargo test -p sparq-lws-core --no-default-features
# Opt in to one periodic orphan-blob sweep task (interval in seconds)
SOLID_SERVER_RECONCILE_INTERVAL_SECS=3600 cargo run -p sparq-lws-core
```

The binary is configured entirely by `SOLID_SERVER_*` / `PSS_*` env variables (bind address, TLS PEM paths, backend selection, seeding) — see the module docs on `src/main.rs`. `SOLID_SERVER_RECONCILE_INTERVAL_SECS` is unset by default; a positive integer enables one periodic orphan sweep (unchanged one-hour grace period); invalid or zero fails boot.

**`SOLID_SERVER_SEED_DEMO`** (default OFF, `1|true|TRUE|True` only) opt-in-seeds the `research/lws-demo-architecture.md` §3.2 public demo: `/playground/` writable by any *authenticated* agent, anonymous read-only, `acl:Control` granted to nobody — so visitors are not isolated and can overwrite/delete each other's resources (disclosed in the seeded `/README`). It normally requires `PSS_SPARQ_BACKEND=memory`; the escape hatch `SOLID_SERVER_ALLOW_SEED_NONMEMORY=1` permits it on **any** non-memory backend, intended for an *ephemeral* embedded test instance — the server does **not** verify ephemerality, so it is not memory-only in the enforced sense. API break: `seed::DEMO_USER` + `DemoFixtures::{pod, welcome_doc, owner}` removed, `DemoFixtures::playground` moved `/demo/playground/` → `/playground/`, `DemoFixtures::readme` added.

## Native storage lifecycle

`SOLID_SERVER_DATA_DIR` (default unset) implies `PSS_SPARQ_BACKEND=embedded`, opens a durable index at `<data>/index` and `FsBlobStore` at `<data>/blobs`, and conflicts with `SOLID_SERVER_SPARQ_DIR` or another explicit backend. Both `embedded-sparq` and `blob-fs` are required (default-on); builds missing either refuse boot. The legacy directory setting keeps existing-snapshot behavior and its durability guard; a directory-backed index opened through that path also compacts on graceful shutdown.

Boot atomically publishes and syncs `format.json` (`schema=1`, `index.major=1`, `blobs.major=1`) before creating either plane; content without a marker or an incompatible marker refuses boot. The index opens before blob construction; its `index.lock` + `index/sparq.lock` guard the data root. `SOLID_SERVER_WAIT_FOR_LOCK_SECS` (default `0`, non-negative integer seconds) retries index contention with backoff, then names the lock path; other errors fail immediately.

Durable blobs with an ephemeral index refuse boot unless `SOLID_SERVER_ALLOW_INCONSISTENT_DURABILITY=1` (default unset); a durable index with ephemeral bytes always refuses. SIGTERM/Ctrl-C drains listeners and stops the reconciler before actor compaction; failure logs and exits non-zero with recovery data retained. Final client drop joins the actor and releases the lock. See [native storage](../../skills/solid-lws-server/references/native-storage.md) for the complete boot and library contract.

## 📦 Native container image

<!-- sq-lmz40: native image contract; distinct from the wasm/npm development host. -->

Release tags publish `ghcr.io/sparq-org/sparq-lws-core` for `linux/amd64` + `linux/arm64`; pin an immutable `X.Y.Z` tag (also `X.Y` / `latest`).
It binds `0.0.0.0:3000` via `SOLID_SERVER_BIND`, runs as uid/gid **65532**, serves unauthenticated `GET /livez` + `GET /readyz`, and enables no auth bypass or dev escape hatch; other `SOLID_SERVER_*` / `PSS_*` settings pass through.

```bash
VERSION=0.1.0   # the release to deploy
docker run --rm --name sparq-lws-core -p 127.0.0.1:3000:3000 \
  -e SOLID_SERVER_BASE_URL=https://solid.example \
  -e SOLID_SERVER_TRUSTED_ISSUER=https://idp.example/realms/solid \
  "ghcr.io/sparq-org/sparq-lws-core:${VERSION}"
```

**Required production configuration:**

- **Base URL:** `SOLID_SERVER_BASE_URL` = the public `https://` origin (set `SOLID_SERVER_AUDIENCE` only if the token audience differs).
- **TLS:** terminate at a trusted proxy (keep the container port private), or mount PEMs
  and set both `SOLID_SERVER_TLS_CERT` + `SOLID_SERVER_TLS_KEY` (setting one fails boot;
  uid 65532 must read them).
- **Auth:** `SOLID_SERVER_TRUSTED_ISSUER` = the production Solid-OIDC issuer. DPoP,
  HTTPS-only WebIDs, anonymous-mutation rejection, and strict WebID↔issuer checks are on
  by default — never set `SOLID_SERVER_ALLOW_LOOPBACK`, the seed vars, or
  `SOLID_SERVER_BIDIRECTIONAL=off`; access stays WAC-controlled.
- **Storage:** the default `memory` (and `embedded` without a directory) is ephemeral. Set `SOLID_SERVER_DATA_DIR` to a mounted data root to persist both the index and resource bytes; see the storage lifecycle above. The `http` and `redis-replay` backends require opt-in features.

## ✨ Features

- **LDP surface** — containers + RDF/non-RDF resources, conditional requests,
  `Content-Range` reads, Turtle / JSON-LD conneg (oxrdf/oxttl/oxjsonld) honouring the
  JSON-LD `profile` parameter (expanded / compacted forms echoed in `Content-Type`;
  compaction is local and context-free — nothing fetched).
- **Access control** — WAC (`acl:`) evaluated against the SPARQ-authoritative
  store, with an ACL decision cache; public-read fast path.
- **WAC-scoped query endpoint** — default-on, query-only
  `GET`/`POST /sparql` for SELECT, ASK, and CONSTRUCT. Each request rebuilds a
  named-graph-per-readable-resource dataset; the default graph is empty and
  unreadable or uncertain resources never reach the query engine.
- **Auth** — Solid-OIDC access tokens + mandatory DPoP, verified-token cache,
  tiered PoP: RFC 8705 mTLS cert-bound tokens and HKDF/HMAC DPoP-SK attestation.
- **Storage seams** — `Store` / `SparqClient` / `BlobStore` traits: the default-enabled in-process engine (`PSS_SPARQ_BACKEND=embedded` or `SOLID_SERVER_DATA_DIR`), opt-in live SPARQ HTTP client, and filesystem blobs with immutable opaque keys. An unconfigured boot uses the in-memory double.
- **Notification observability** — process-wide backlog-overflow totals
  are available through `notifications::ws::NotificationMetrics::snapshot()`. Subscribe and
  receive are WAC-gated per resource (re-checked before every frame; see the SKILL).
- **Transport hardening** — HTTP/2 rapid-reset and HTTP/1 slowloris guards (explicit
  header-count, aggregate-byte, and slow-header timeout bounds); request timeouts, body
  limits, per-connection max-requests, rate limiting, and overload shedding.
- **LWS 1.0 mode** — `SOLID_SERVER_PROTOCOL=lws` serves W3C Linked Web Storage (`src/lws/`, skill § *Run the LWS 1.0 protocol*, [`conformance/lws/`](./conformance/lws)).
- Cargo features:
  - `blob-object` (**default-off**, native only) — library adapter for object-store bytes; server wiring pending.
  - `test-support` (**default-off**) — reusable validated, create-only blob backend contract.
  - `blob-fs` (**default-on**, native only) — async adapter for [`sparq-lws-blob-fs`](../sparq-lws-blob-fs); hold the data-directory lock before open and until drop.
  - `embedded-sparq` (**default-on**, sq-gg0qq.3) — the first-class in-process SPARQ engine backend (in-workspace path deps on `sparq-core`/`sparq-engine`); `--no-default-features` builds the engine-free profile.
  - `sparql-endpoint` (**default-on**, sq-r1ei8) — query-only endpoints: `/sparql`
    under WAC in Solid mode and `/.lws/sparql` under LWS Read rules; internal Store methods remain independent.
  - `http-sparq` (off) — the remote SPARQL-over-HTTP backend
    (`PSS_SPARQ_BACKEND=http`) for a shared-service deployment.
  - `http3` (off, sq-oprna.2) — with the TLS PEM variables configured, also serve
    the same hardened LDP router over HTTP/3 on UDP at the resolved `SOLID_SERVER_BIND`
    address+port; TCP stays HTTP/2 + HTTP/1.1 (and WS).
  - `redis-replay` (off) — a shared Redis-backed DPoP `jti` replay store for horizontally-scaled deployments.
  - `authn-ssi` + `authn-openid` (**default-on**) and `authn-saml` (off) — the LWS 1.0 authorization server's authentication suites, each its own `sparq-lws-authn-*` crate implementing `sparq_lws_authn::Suite`; `SOLID_SERVER_LWS_AUTHN` picks among those compiled in.
  - `odrl-authz` (off, sq-elg47) — the native ODRL policy gate seam on the
    read/query path (`authz::odrl`, via `LdpState::set_odrl_gate`; deny-overrides /
    permit-extends over the WAC decision, fail-closed).
  - `trust-graph` (off, sq-hed3q) — the LIBRARY-only trust-graph admission
    seam (`authz::trust_admit`); NOT handler-wired. Research prototype (sq-qhy4).

## 📚 Learn more

- Usage: [`skills/solid-lws-server/SKILL.md`](../../skills/solid-lws-server/SKILL.md). Authorized dataset rewrites retain query VERSION announcements. Unknown or incompatible labels return a bad-request error before evaluation; see the [EBV dialect contract](../../skills/sparql-query/ebv-dialects.md).
- Design records: [`research/lws-design-records.md`](../../research/lws-design-records.md) — the in-repo home for this crate's migrated `decisions/` + `docs/design/` estate (sq-gg0qq.10), reconstructed from the code. Doc-comments here cite that record by section; where a source-repo path is still named it carries its `RSS`/`PSS` namespace (§2), and §1 maps every source path to its in-repo home. `bench/` stays in the source repo.
- Specification estate (what this crate is pinned to, and what is still UNRESOLVED — issue #4971): the crate-level rustdoc, `cargo doc -p sparq-lws-core --open`.
- Normative specs — the spec is the contract, not this implementation: [DPoP-SK](https://jeswr.github.io/dpop-sk-spec/), implemented here in `src/pop/sk/` against that profile (its Appendix-A worked example runs as a test vector); and the pinned [solid-oidc-verifier](https://github.com/jeswr/solid-oidc-verifier), which owns baseline (cache-miss) Solid-OIDC token + DPoP proof verification. On a verified-token-cache hit `src/auth_cache.rs` re-verifies the fresh proof locally — signature, `htm`/`htu`/`iat`, `ath`, `jti` replay, `cnf.jkt` binding — built from the verifier's own public primitives and its shared replay store, so that path is security-sensitive code to audit here.
- Solid CTH conformance: [`conformance/`](./conformance) — opt-in lane; the score is generated + ratcheted, never committed prose (sq-gg0qq.7).
- Related crates: [`sparq-solid`](../sparq-solid) (Solid protocol pieces), [`sparq-server`](../sparq-server) (the SPARQL endpoint it can delegate to).

## License

MIT OR Apache-2.0 — see LICENSE-MIT and LICENSE-APACHE in this directory.
