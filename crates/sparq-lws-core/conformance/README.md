<!-- [SONNET-4.6] sq-gg0qq.7: Solid CTH wire-conformance harness for sparq-lws-core, ported from
     jeswr/solid-server-rs@1e555b10 (the repo this crate was imported from — sq-gg0qq.2). -->

# Solid CTH conformance — `sparq-lws-core`

Runs the official [Solid Conformance Test Harness](https://github.com/solid-contrib/conformance-test-harness)
(CTH) against the `sparq-lws-core` binary, booted with the **in-memory store doubles**
(`CompositeStore` over `InMemorySparqClient` + `InMemoryBlobStore` — no S3, no live SPARQ; S3 is
explicitly out of scope for this baseline) terminating TLS in-process and seeded with the test users.

This is **wire-level** conformance against a running server. It is distinct from the *library-level*
conformance harnesses in `crates/sparq-solid/tests/conformance_*.rs`, which pin scenario floors
without a network. Both exist; neither subsumes the other.

The committed config lives in `config/`; the TLS SAN definition in `tls/`. The Java harness, the
`solid-contrib/specification-tests` manifests, the generated TLS pair, and generated reports are
**not** committed.

## What is tested

The harness loads the Solid Protocol + WAC manifests from `solid-contrib/specification-tests` and runs
the scenarios `config/test-subjects.ttl` claims, after the three skip tags (`acp`,
`wac-agent-group`, `http-redirect` — each with its rationale in that file).

The TestSubject claims the LDP/Solid surface the server implements: LDP CRUD including
intermediate-container creation + `ldp:contains` listing, Turtle/JSON-LD content negotiation,
conditional + Range requests, CORS, OPTIONS/`Allow`/`Accept-*`, `text/n3` + `application/sparql-update`
PATCH, storage description + `/.well-known/solid` discovery, WebSocketChannel2023 notifications, and
the full Web Access Control suite (the in-Rust engine in `src/authz`, evaluating `.acl` documents
through the `Store` seam).

**The score is generated, not written down here** — see [`SCORE.md`](./SCORE.md) and
[`baseline.json`](./baseline.json).

## Prerequisites

All three are **external to this repo** and must be present before `run.sh` will proceed (it
pre-flights each and fails with an actionable message):

1. **The `solid` Keycloak realm** at `http://localhost:8080/realms/solid`, with the
   `conformance-alice` / `conformance-bob` DPoP service-account clients
   (`serviceAccountsEnabled` + `dpop.bound.access.tokens`). `run.sh` never modifies it.
2. **An `ath`-patched CTH docker image** (default `pss-cth:ath`, override `CTH_IMAGE`). The published
   harness does not send the RFC 9449 DPoP `ath` claim on resource requests, so it cannot
   authenticate against a server that enforces `ath` — which this server does, via the verifier. This
   is an upstream harness bug; the image must be built from a patched harness clone.
3. **Docker** (for the harness and the `socat` sidecar) and network egress.

`run.sh` handles the fourth input itself: the `solid-contrib/specification-tests` manifests are
shallow-cloned into `.cache/` on first run (override `SPEC_TESTS` to reuse an existing clone).

## Running

```sh
cargo build --release -p sparq-lws-core
./crates/sparq-lws-core/conformance/run.sh
```

Reports land in `conformance/reports/` (`report.html`, `report.ttl` EARL, plus the Karate
`target/karate-reports/` inside the report dir). The booted server's log is `reports/server.log`.

Override knobs (all optional): `CTH_IMAGE`, `SPEC_TESTS`, `SPEC_TESTS_REPO`, `SERVER_BIN`, `ENV_FILE`,
`CTH_BASELINE`, `CTH_ENFORCE_BASELINE`, `PSS_SPARQ_BACKEND`.

`PSS_SPARQ_BACKEND=embedded` runs the same suite against the in-process engine over an ephemeral
in-memory graph. Unlike the source repo, no extra `--features` flag is needed — `embedded-sparq` is
default-on in this workspace (sq-gg0qq.3).

## Auth wiring (Keycloak, the hard part)

The CTH logs its test users in to obtain access tokens. Identity is delegated to **Keycloak**; the
WebIDs are seeded by the server itself:

1. **Token path — client-credentials, DPoP-bound.** Each user (alice, bob) maps to a confidential
   Keycloak service-account client. The CTH exchanges `USERS_<U>_CLIENTID`/`CLIENTSECRET` for a token
   via the realm token endpoint and adds the DPoP proof itself (`Client.withDpopSupport`), so the
   token satisfies the verifier's DPoP requirement. The token is RFC-9068 `at+jwt`,
   `iss=http://localhost:8080/realms/solid`, `aud=["solid","https://localhost:3000"]`,
   `webid=https://localhost:3000/<u>/profile/card#me`, `cnf.jkt` present.
2. **WebID + container seeding (in lieu of a provisioner).** The server has no provisioner, so it
   seeds the test users at boot when `SOLID_SERVER_SEED_CONFORMANCE=1` (see `src/seed.rs`): the root
   container `/`, each user's `/{u}/`, `/{u}/profile/`, `/{u}/test/` containers, each WebID profile
   `/{u}/profile/card` whose `#me` subject carries `pim:storage` → the pod root and `solid:oidcIssuer`
   → the realm, plus the pod-root owner-default ACL and the public-read profile-card ACL so the
   harness can bootstrap under the enforced WAC engine. Profiles are built from `oxrdf` triples + the
   server's own Turtle serializer (never hand-concatenated).
3. **Trust + audience.** Booted with `SOLID_SERVER_TRUSTED_ISSUER` = the realm and
   `SOLID_SERVER_AUDIENCE=https://localhost:3000` (matching the token's mandatory `aud`).
   `SOLID_SERVER_BIDIRECTIONAL=off` skips the WebID↔issuer cross-check, keeping the in-memory
   baseline independent of WebID-fetch round-trips. `SOLID_SERVER_ALLOW_LOOPBACK=1` permits the
   `http:` localhost IdP (dev/IT escape hatch).

### Networking (load-bearing)

The token's `iss` is whatever host the harness used for OIDC discovery, because Keycloak echoes its
issuer from the request host. The verifier's SSRF guard permits an `http:` issuer **only if it
resolves to loopback**. So the token `iss` and the server's trusted issuer must BOTH be
`localhost:8080`. To make the harness mint a `localhost:8080`-issued token AND reach the server,
`run.sh`:

- runs the harness with **`--network host`** — on Docker Desktop this shares the Linux VM's network
  namespace, where Keycloak (a VM container) is reachable at `localhost:8080` and discovery returns
  `iss=http://localhost:8080/realms/solid`;
- **probes** whether a throwaway `--network host` container can already open a TCP connection to
  `localhost:3000`, and starts a **`--network host` `socat` sidecar** forwarding the VM's
  `localhost:3000` → the host's `:3000` (`host.docker.internal`) **only when it cannot**.

Net effect: harness, server, and Keycloak all agree on `localhost:3000` / `localhost:8080`, the DPoP
`htu` matches the server's reconstructed URL, and the http issuer resolves to loopback.

The probe is why `run.sh` is portable, and it replaced an unconditional sidecar that was wrong on one
of the two branches:

| engine | `--network host` means | forwarder |
| --- | --- | --- |
| Docker Desktop / colima (VM-backed) | the Linux VM's netns, which cannot reach a host-bound process via `localhost` | **started** — it is the load-bearing hop |
| native Linux | the literal host netns, which already reaches the server | **skipped** |

On native Linux the sidecar is *not* a harmless passthrough: the server holds `0.0.0.0:3000`, so
socat's `TCP-LISTEN:3000` fails with `EADDRINUSE` and the container dies immediately (`reuseaddr` is
`SO_REUSEADDR`, which does not permit a second listener on a live socket). Because `docker run -d`
exits 0 as soon as the container *starts*, that failure sailed past `set -e` and left a dead container
posing as the hop. When the forwarder *is* started, `run.sh` now re-probes and fails loudly (dumping
`docker logs`) if the server is still unreachable. `check-conformance-config.py` C7 keeps the start
gated on the probe.

### CORS

The CTH's CORS scenarios are why `src/ldp/cors.rs` is **hand-rolled** rather than `tower-http`'s
`CorsLayer`: the harness's `match header Vary contains 'Origin'` is case-**sensitive** and
`tower-http` emits a lowercased `vary`, and the harness also requires concrete (non-`*`)
expose-headers on the preflight. That middleware survived the crate import, and both properties are
pinned by the `#[cfg(test)] mod tests` in `cors.rs` — `preflight_returns_empty_204_with_reflected_origin_and_headers`
asserts a non-empty, non-`*` expose-header list and a capital-`Origin` `Vary` on the preflight, and
`merges_a_handler_set_vary_accept_with_vary_origin` pins that the middleware appends to the handler's
`Vary: Accept` instead of clobbering it. Do not swap it for the stock layer.

### TLS

The verifier requires the server reachable over **https** (the harness dereferences the https WebID).
The server terminates TLS in-process (rustls/aws-lc-rs). The source repo committed a self-signed
`server-cert.pem`/`server-key.pem` pair; **this port commits only `tls/san.cnf`** and generates the
pair on first run (both generated files are gitignored), so no long-lived private-key material enters
this repo and the SANs — `host.docker.internal`, `localhost`, `127.0.0.1` — stay single-sourced. The
harness trusts the result via `ALLOW_SELF_SIGNED_CERTS=true`.

## Upstream branch triage (sq-gg0qq.7)

The port task asked for a port-or-drop verdict on two upstream branches. **Verdict: nothing to port
and nothing to drop — both are already in the imported code**, established as follows.

| Branch | Tip | Ahead of `main` | Ancestor of `main@1e555b10` |
|---|---|---|---|
| `conformance/wire-cth-baseline` | `982cf7d` | 0 commits | yes |
| `conformance/protocol-fixes` | `f9ff60f` | 0 commits | yes |

The lineage is linear: `wire-cth-baseline` → `protocol-fixes` → `main@1e555b10`, and `1e555b10` is
exactly the commit `sparq-lws-core` was imported from (sq-gg0qq.2). So every commit on both branches
is in this crate's history by construction. The three commits `protocol-fixes` adds over
`wire-cth-baseline` are `f119f87` (Cluster-A protocol completeness), `1b3b0d9` (four review findings)
and `f9ff60f` (origin-exact public-read gate, an auth-bypass fix).

One honest caveat: *merged* is not the same as *present verbatim*. Roughly 140 further commits landed
on upstream `main` after `protocol-fixes`, and some of that code was refactored on the way — e.g.
`f9ff60f`'s `split_origin_and_path` / `origin_spoof_prefix_is_not_public_readable` no longer exist
under those names. The security property they added did survive, as `INV-6 ORIGIN-FAIL-CLOSED` in
`src/ldp/public_read_skip.rs`. What this crate carries is upstream `main`'s evolved version, which is
the correct thing to carry — but a reader should not expect a name-for-name match against the branch
diffs.

Related: `research/solid-cth-wire-conformance-feasibility.md` concludes that CTH wire conformance is
not worth building. That conclusion is scoped to **`sparq-server`**, which deliberately has no
accounts, pods or WAC enforcement, so the CTH grades the wrong subject there. It does not apply to
`sparq-lws-core`, which implements exactly the surface the CTH grades.

## CI

The [`lws-cth.yml`](../../../.github/workflows/lws-cth.yml) workflow is **opt-in**
(`workflow_dispatch` only) and declared advisory, because the harness needs Docker, network egress,
a Keycloak realm and a patched image that GitHub-hosted runners do not have. It never runs on
`pull_request` or `push`, so it adds no latency to the required gate and cannot contribute to CI
congestion. Its cheap `preflight` job (shellcheck + config/baseline validation) is hermetic and safe
to dispatch anywhere; the `harness` job needs a runner with the stack above.
