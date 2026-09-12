# Extracting the transport DoS-hardening layer into a standalone crate (design record)

> 🤖 SPARQ agent [OPUS-5] — design-for-review record for issue **#3702** (`[new-lib]` front of the
> maintainability program). It grounds the commissioning premise against the real tree, corrects it
> in four places, designs an acceptor-agnostic seam, and cuts the work into disjoint child beads.
> **No implementation is proposed for this PR** — the change spans four crates plus two upstream
> repos and cannot land as one scoped edit. **No measurements were taken**; every number quoted
> below is a *configuration default* read out of the source, not a benchmark result.

## 1. Scope and method

The commission: `crates/sparq-lws-core/src/transport.rs` is protocol-generic hyper/tokio hardening
that three HTTP servers in this workspace need, so lift it into a standalone (crates.io-candidate)
crate, make the seam acceptor-agnostic, delete the duplicates, and offer the two knobs hyper omits
upstream to `hyper-util`.

Ground truth was established by reading the actual sources, not the issue text:
`crates/sparq-lws-core/src/transport.rs` (2615 LOC, 35 tests), `crates/sparq-lws-core/src/main.rs`
(the only wiring site), `crates/sparq-lws-core/src/rate_limit.rs`, `crates/sparq-lws-core/src/pop/conn.rs`,
`crates/sparq-server/src/http.rs` (11763 LOC), `crates/sparq-http3/src/server.rs` (413 LOC),
the two manifests, `Cargo.lock`, `scripts/gate-new-crate.py`, and
[`AGENTS.md`](../AGENTS.md) § *Upstream blockers* / § *Post-batch re-evaluation checklist*.

Pinned versions (from `Cargo.lock`): `hyper` 1.10.1, `hyper-util` 0.1.20, `h2` 0.4.15,
`axum` 0.8.9, `axum-server` 0.8.0.

### 1.1 Four corrections to the commissioning premise

**(a) `transport.rs` is not "zero sparq types".** It carries three in-crate couplings:

| Coupling | Site | Nature |
|---|---|---|
| `use crate::rate_limit::is_internal_ip` | `transport.rs:105` | a pure `IpAddr` classifier — no sparq types, but it lives in another module |
| `impl crate::pop::conn::{PeerCertDer, TlsExporter} for PermittedStream<Io>` | `transport.rs:802`, `:810` | forwards the mTLS/DPoP-SK reads through the permit wrapper |
| `impl crate::pop::conn::{PeerCertDer, TlsExporter} for IdleTimeoutStream<Io>` | `transport.rs:822`, `:831` | same, through the idle wrapper |

None is fatal, and §3.3 gives each a clean resolution — but the extraction is **not** a file move,
and the issue's "zero sparq types" framing should not be carried into the bead as an estimate.

**(b) `sparq-server` does not re-implement "the overlapping subset" — the drift is asymmetric.**
The *only* genuine overlap is the HTTP/1 header-read timeout. Verified inventory:

| Capability | `sparq-lws-core` transport.rs | `sparq-server` http.rs | `sparq-http3` server.rs |
|---|---|---|---|
| h1 header-read (slowloris) timeout | yes (`TransportConfig::header_read_timeout`) | yes — `ServerConfig::header_read_timeout`, default 15s (`http.rs:838`), wired at `:5063` (h1 builder) and `:5297` (auto builder) | n/a (QUIC) |
| h2 `max_concurrent_streams` / rapid-reset cap | yes (explicit, env-tunable) | **absent** | n/a |
| global concurrent-connection cap | yes (`ConnectionLimiter`) | **absent** | yes (local re-impl, `server.rs:215`) |
| per-source (per-IP) connection cap | yes | **absent** | yes (local re-impl) |
| TLS-handshake timeout | yes | **absent** | n/a (QUIC handshake) |
| idle-keepalive IO timeout | yes (`IdleTimeoutStream`, default-off) | **absent** | n/a (QUIC has its own idle timeout) |
| max-requests-per-connection | yes (`MaxRequestsService`, default-off) | **absent** | yes (`max_requests_per_connection` arg) |
| slow-**body** read/idle deadline | **absent by deliberate design** (module docs argue the request timeout + body cap already bound it) | yes — `tower_http::RequestBodyTimeoutLayer` (`http.rs:5004`, `sq-lodb`) | n/a |

So the workspace is not "paying for this logic twice" in the way the issue states. `sparq-server`
is **missing six of the eight transport guards outright** — including any concurrent-connection cap
at all (`grep -n "Semaphore\|max_connections\|ConnectionLimit" crates/sparq-server/src/http.rs`
returns nothing). That reframes the payoff: the prize is **closing a capability gap in the flagship
server**, and de-duplication is the smaller, secondary benefit. It also means the acceptance
criterion "duplicated sites are deleted, not deprecated" only bites in two places (§6, P2/P3),
not across the board.

Corollary worth writing down: `sparq-server` reached for `tower_http::RequestBodyTimeoutLayer`
rather than hand-rolling. The extracted crate should do the same wherever an upstream layer already
exists, and should **not** absorb the slow-body deadline — that one is already solved upstream.

**(c) The cited line numbers have drifted, and there is more than one serve loop.** `http.rs:834`
is now `:838`; the "hand-rolled per-connection serve loops at 4825–4900" are now ≈`:4986–5108`.
More importantly there are **four** serve entry points, not one — `serve` (HTTP/1-only, uses
`hyper::server::conn::http1::Builder`), the `http2`-feature variant, the rustls TLS variant
(`tokio_rustls::TlsAcceptor`, `http.rs:5165`), and `serve_h3`. An acceptor-agnostic seam must
therefore cover the **h1-only builder** as well as `hyper_util`'s `auto::Builder`; the issue's
design only names the latter.

**(d) `sparq-http3`'s limiter is QUIC, so only the *policy* is shareable.** `IdleTimeoutStream`,
`MaxRequestsService` and every hyper builder knob are inapplicable to a quinn connection (QUIC
carries its own idle timeout, and h3 has no `Connection: close`). What *is* shareable is the
admission policy — semaphore + per-IP live-connection map + internal-IP exemption + the defaults
(`DEFAULT_MAX_CONNECTIONS = 10_000`, `DEFAULT_MAX_CONNECTIONS_PER_IP = 512`, declared identically
at `sparq-http3/src/server.rs:25,28` and `transport.rs:225,245`). `is_internal_ip` is **triplicated**:
`rate_limit.rs:301`, `sparq-http3/src/server.rs:296`, and (by import) `transport.rs`. Collapsing
that triple is the single highest-confidence, lowest-risk win in this whole issue.

## 2. What is actually extractable

`transport.rs` decomposes into four tiers with very different portability:

| Tier | Items | Deps | Portable? |
|---|---|---|---|
| **T1 — policy** | `ConnectionLimiter`, `IpConnGuard`, `PerIpConnMap`, `is_internal_ip`, the `DEFAULT_*` consts, the `parse_*` helpers | `tokio/sync` only | yes, cleanly. Usable by hyper *and* quinn *and* anything else |
| **T2 — IO adapter** | `IdleTimeoutStream`, `PermittedStream`, `InFlightGuard`, `PeerAddr` | `tokio/io` | yes; sparq-specific trait impls must move out (§3.3) |
| **T3 — service adapter** | `MaxRequestsService`, `MaxRequestsFuture`, `TrackedBody`, `is_upgrade_response` | `tower`, `http`, `http-body` | yes |
| **T4 — glue** | `TransportConfig` (+ `apply_to_builder`), `ConnectionLimitAcceptor` | `hyper-util`, `axum-server` | `apply_to_builder` yes (behind a `hyper` feature); `ConnectionLimitAcceptor` only behind an `axum-server` feature |

### 2.1 The 35 tests, inventoried by tier

The 35 `#[test]`/`#[tokio::test]` functions in `transport.rs`'s `mod tests` (`:1481–2615`) were read
and each assigned to the tier of its unit under test. **They do not all belong to T1**, so no single
phase can migrate all 35 — the split below is what makes §6 disjoint:

| Tier → phase | Count | Tests |
|---|---|---|
| **T1 → P0** — parsers | 10 | `h2_max_concurrent_streams_default_on_absent_empty_invalid_or_zero`, `h2_max_concurrent_streams_explicit_positive_is_honoured`, `h2_pending_reset_keeps_hyper_default_unless_positive_override`, `max_connections_default_on_absent_empty_invalid_or_zero`, `max_connections_explicit_positive_is_honoured`, `optional_secs_rules`, `max_connections_per_ip_parse_rules`, `conn_exempt_internal_parse_rules`, `max_requests_per_conn_disabled_on_absent_empty_invalid_or_zero`, `idle_timeout_is_default_off_and_zero_disables` |
| **T1 → P0** — limiter/per-IP policy | 8 | `connection_limiter_clamps_to_at_least_one_permit`, `connection_limiter_bounds_in_flight_then_recovers`, `connection_limiter_clamps_huge_value_below_semaphore_max`, `per_ip_cap_bounds_a_single_source_and_recovers_on_drop`, `per_ip_cap_isolates_sources`, `per_ip_cap_exempts_internal_ips_by_default`, `per_ip_cap_disabled_admits_everything`, `per_ip_cap_zero_is_clamped_to_one_never_refuses_all` |
| **T2/T3 → P2** — IO + service | 13 | the five `idle_timeout_stream_*` tests, the six `max_requests_service_*` tests, `tcpstream_peer_ip_is_extracted_for_the_per_source_cap`, `is_upgrade_response_exempts_only_101_not_advisory_upgrade_headers` |
| **T4 → P3** — builder + acceptor | 3 | `apply_to_builder_does_not_panic_with_timeout_set`, `apply_to_builder_with_disabled_timeouts_does_not_panic`, `acceptor_releases_permit_when_handshake_stalls` |
| **stays in `sparq-lws-core`** | 1 | `from_env_with_unset_vars_uses_documented_defaults` — asserts the `SOLID_SERVER_*` env wiring, which §3.4 deliberately leaves in the consumer |

Note the parser tests migrate at P0 even for knobs whose *implementation* is T3/T4 (`idle_timeout`,
`max_requests_per_conn`, the h2 caps): the `parse_*` helpers are `Option<String>`-in, value-out and
carry no tier dependency. The per-IP tests share a local `pub_ip` helper (`:1797`) that must move
with them.

The `ENV_*` constants are **not** portable as written: all ten are `SOLID_SERVER_`-prefixed
(`transport.rs:117–190`), whereas `sparq-server` uses `SPARQ_*`. A general crate must not bake a
prefix in. See §3.4.

## 3. Seam design

### 3.1 Layering

A single crate with additive, default-off features, so a quinn consumer never compiles hyper:

```text
default          = []                      # T1 only: policy + parsers, tokio/sync
feature "io"     = T2                      # + tokio/io
feature "service"= T3                      # + tower/http/http-body
feature "hyper"  = ["io","service"] + T4a  # + hyper-util: apply_to_builder for auto::Builder AND http1::Builder
feature "axum-server" = ["hyper"] + T4b    # + ConnectionLimitAcceptor
```

Consumers: `sparq-http3` takes the default (T1 only). `sparq-server` takes `hyper` (it drives
hyper builders directly; it has no `axum-server` dep). `sparq-lws-core` takes `axum-server`.
This is the standard opt-in-feature discipline the workspace already enforces, and it is what
makes "one crate, three servers" honest rather than a lowest-common-denominator compromise.

### 3.2 Acceptor-agnostic entry points

`ConnectionLimiter`'s admission *logic* is already acceptor-agnostic — it works in `IpAddr`s and
guards and knows nothing about `Accept`. The coupling is entirely in `ConnectionLimitAcceptor`.
So the seam is: **make the guard-returning API the primary public surface**, and ship
`ConnectionLimitAcceptor` as one *adapter over* it (feature-gated), with a second documented
adapter shape for a hand-rolled accept loop:

```rust
// hand-rolled loop (sparq-server's four serve fns), inside the accept loop.
// NB: this is the DESIGN TARGET, not an API that exists today — see the delta below.
let Some(admit) = limiter.try_admit(peer_addr.map(|a| a.ip())) else { continue }; // shed, don't queue
let io = PermittedStream::new(IdleTimeoutStream::new(tls_or_tcp, idle, in_flight), admit);
```

**The API delta is larger than the commissioning premise implies, and it is a real design decision
that P0 owes, not an incidental `pub` flip.** Verified visibility in `transport.rs` today:

| Item | Site | Today | Needed by P1 |
|---|---|---|---|
| `ConnectionLimiter::try_acquire` | `:517` | **private** | yes — global admission |
| `ConnectionLimiter::try_acquire_ip` | `:482` | **private** | yes — per-source admission |
| `IpConnGuard::disabled` | `:602` | **private** | yes — the unknown-peer fail-open guard |
| `PermittedStream::new` | `:789` | **private** | yes — to hold both guards for the connection |
| `ConnectionLimiter::try_acquire_ip_for_test` | `:525` | `pub`, but `#[doc(hidden)]` | no — a test shim, not a supported cross-crate API |
| `IdleTimeoutStream::new` | `:971` | `pub` | already fine |
| `MaxRequestsService::new` | `:1237` | `pub` | already fine |

So **four** private items sit on the P1 path, and today's only public acquisition entry point is a
`#[doc(hidden)]` test shim. Two shapes are available:

- **(i) One atomic admission method — recommended.** `try_admit(peer: Option<IpAddr>) -> Option<ConnAdmission>`
  returning both guards in one value. This *encodes* the two load-bearing behaviours named just
  below (global permit first; unknown peer fails open) instead of documenting them and hoping
  four hand-rolled call sites get the order right. It also keeps `IpConnGuard::disabled` private.
- **(ii) Expose the two methods plus a public guard constructor.** More flexible, but it publishes
  the ordering contract as prose and makes "acquire per-IP before global" a compilable mistake.

Either way this is a **new public API that has never been reviewed**, so P0 must specify it and
carry its tests (§6), rather than treating it as a mechanical re-export.

Two behaviours the extraction must preserve verbatim, because they are load-bearing and easy to
lose in a port: the global permit is acquired **fail-fast, outside the async block** (an over-cap
connection is refused, never parked as a queued task — `transport.rs:702`), and an **unknown peer
IP fails open** to the global cap rather than being refused (`transport.rs:728`).

### 3.3 Resolving the three sparq couplings

- **`is_internal_ip`** moves *into* the new crate (T1). `sparq-lws-core::rate_limit` and
  `sparq-http3` then both import it and delete their copies. This is a pure move: the function
  is `IpAddr`-only, and the two existing bodies are already semantically equivalent (loopback /
  RFC 1918 / link-local / IPv4-mapped v6 / ULA / v6 link-local).
- **`PeerCertDer` / `TlsExporter` impls** are *not* an orphan-rule problem, because the trait is
  local to `sparq-lws-core` and the type becomes foreign — `impl LocalTrait for ForeignType` is
  allowed. So the four impls simply **move to `crates/sparq-lws-core/src/pop/conn.rs`**, where
  they arguably belonged anyway (they are PoP concerns, not transport concerns). No newtype, no
  blanket-impl gymnastics, no API change. This is the single most important thing to get into the
  bead spec, because it is the one place an implementer would otherwise burn hours.

### 3.4 Configuration and the env-prefix problem

Keep the crate's `TransportConfig` **env-free**: a plain struct with `Default` plus the ten
`parse_*` functions taking `Option<String>` (they already have exactly that signature —
`transport.rs:1369–1479` — so they are already testable without touching the process environment).
Each consumer keeps its own `from_env` that supplies its own prefix. That preserves
`SOLID_SERVER_*` for existing lws operators (no breaking rename) while letting `sparq-server` use
`SPARQ_*`, and it keeps `std::env` out of a library — which is the right shape for a crates.io
crate regardless.

## 4. Options considered

| Option | What | Trade-off | Verdict |
|---|---|---|---|
| **A. Publish a standalone crates.io crate now** | new `crates/<name>`, `publish` on, full public API | Maximum reuse story, but locks a public API that has *never* been exercised by a second consumer, and immediately pulls the full publish tax: a registered `bench/benchmarks.toml` entry, a `skills/<surface>/SKILL.md`, README-template compliance, `cargo-vet`/`cargo-deny`, and semver liability (`scripts/gate-new-crate.py` § RULE G1) | **not first** |
| **B. In-workspace `publish = false` crate, publish later** | same code, stub-exempt from the bench + SKILL requirements (README still required) | Lets the API be *proved* against three consumers before it is frozen; the publish step later is then a manifest flip plus the artifacts, with a known-good surface | **recommended** |
| **C. No new crate — `sparq-server` depends on `sparq-lws-core`** | reuse in place | Drags an entire Solid/LDP server (axum-server, rustls, object_store, the LDP/WAC stack) into `sparq-server`'s graph for eight small guards. Directly contradicts the keep-the-core-lean rule | **rejected** |
| **D. Narrow fix only** | give `sparq-server` a connection cap; leave the rest | Closes the real security gap fastest with the least risk, but re-triplicates the policy — the exact divergence this issue exists to stop | **rejected as an endpoint**, but P1 below is deliberately shaped so it is also the useful bail-out point |

**Recommendation: B, staged as §6.** Land the crate `publish = false`, migrate the three consumers
in separate PRs, then decide on publication once the API has survived contact with a quinn consumer
and a hand-rolled-hyper consumer. Publishing is a one-way door (a crates.io version cannot be
unpublished); nothing here is time-critical enough to justify walking through it early.

## 5. The two upstream knobs

**Verify before filing.** The claim that hyper 1.x exposes neither a per-connection idle-keepalive
timeout nor a max-requests-per-connection cap is, in this record, sourced *only* to the in-tree pin
(hyper 1.10.1 / hyper-util 0.1.20) and to `transport.rs`'s own module docs. This environment has no
network access, so I could not check current `hyper`/`hyper-util` HEAD, open PRs, or the issue
tracker. **P6 must begin by re-verifying against upstream HEAD** — if either knob has since landed,
the correct move is to adopt it and delete our copy, not to file a PR.

On the merits, and stated as an expectation rather than a prediction of acceptance:

- **Max-requests-per-connection.** `MaxRequestsService` is a plain `tower::Service` wrapper that
  sets `Connection: close` after N exchanges. It needs nothing from hyper's internals, which is
  precisely the argument a maintainer will use to decline it — "this is a tower layer, write it in
  your app." That argument is correct. The stronger case is that it is a *widely re-implemented*
  layer with a subtle correctness trap our version already handles (`is_upgrade_response`,
  `transport.rs:1320` — never clobber a 101 with `Connection: close`), which is exactly the kind of
  thing that belongs in a shared crate. **`tower-http` is the better upstream home than
  `hyper-util`**; propose it there first.
- **Per-connection idle-keepalive timeout.** `IdleTimeoutStream` is an `AsyncRead`/`AsyncWrite`
  adapter, not a hyper concern either — and by the same logic **`tokio-util` is a plausible home**
  for a generic `IdleTimeout<S>` IO wrapper. The hyper-util `auto::Builder` framing in the issue is
  only right if the knob is expressed as a *builder* option, which would require hyper to plumb it
  into the connection state machine (a much larger, much more likely-declined change). Recommend
  offering the IO adapter, not the builder knob.

Whichever home is chosen, [`AGENTS.md`](../AGENTS.md) § *Upstream contributions — how to open the PR*
governs: **tag @jeswr first**, open as a **draft**, carry the explicit *"NOT yet ready for maintainer
review"* note, lead with **Why** (why sparq needs it) before What/How, and record the URL in the bead
and in [`docs/upstream-proposals.md`](../docs/upstream-proposals.md). Never mark it ready — that is
@jeswr's call.

## 6. Phased plan (each phase is one future bead)

Ordered; each is single-crate or single-seam and disjoint from its siblings.

1. **P0 — collapse `is_internal_ip` + land the T1 crate (crate: new + `sparq-lws-core` + `sparq-http3`).**
   Create the crate with T1 only (`publish = false`, README, no `std::env`): `is_internal_ip`, the
   `DEFAULT_MAX_CONNECTIONS` / `DEFAULT_MAX_CONNECTIONS_PER_IP` consts, `ConnectionLimiter`,
   `IpConnGuard`, the `parse_*` helpers, and **the 18 T1 tests of §2.1** (10 parser + 8
   limiter/per-IP) — the other 17 stay put until their tier moves, because they will not compile
   against a T1-only crate. P0 also **specifies and reviews the public admission API** of §3.2
   (shape (i) or (ii)); the four items private today are the point of the bead, not a footnote.
   Both existing `is_internal_ip` copies are **deleted**, not deprecated. *Acceptance:* both
   feature states green; the 18 migrated tests pass unchanged; a test asserts the two servers' caps
   resolve from the same consts; and the new public admission API has a direct unit test for each
   of — fail-fast global admission (over-cap ⇒ `None`, never awaited), per-IP refusal at the cap,
   unknown-peer (`None` IP) fail-open, and release of both counters on guard drop.
2. **P1 — give `sparq-server` the connection cap (crate: `sparq-server`).** Wire T1 into all four
   serve entry points via the public admission API P0 shipped. This is the security fix, and it is
   deliberately ordered before the IO/service tiers so the gap closes even if the rest stalls.
   *Acceptance:* an over-cap connection is shed (not queued); unknown peer IP fails open; a
   per-source-capped IP is refused while a second IP is still admitted; defaults chosen so the
   conformance harness cannot trip them.
3. **P2 — move T2/T3 (crate: new + `sparq-lws-core`).** `IdleTimeoutStream`, `PermittedStream`,
   `InFlightGuard`, `PeerAddr`, `MaxRequestsService`, `TrackedBody` move behind the `io` /
   `service` features, **with the 13 T2/T3 tests of §2.1**; `new()` constructors become `pub`; the
   four `PeerCertDer`/`TlsExporter` impls move to `pop/conn.rs` per §3.3. *Acceptance:* those 13
   tests pass in the new crate under `--features io,service`; the mTLS Tier-1b and DPoP-SK paths
   still read cert/exporter through both wrappers (existing tests must cover this — if they do not,
   adding that test is part of this bead).
4. **P3 — move T4 and re-point `sparq-lws-core` (crate: new + `sparq-lws-core`).**
   `TransportConfig` (env-free) + `apply_to_builder` behind `hyper`, `ConnectionLimitAcceptor`
   behind `axum-server`; `sparq-lws-core::transport` becomes a thin re-export shim holding only
   its `SOLID_SERVER_*` `from_env`. *Acceptance:* zero behaviour change — the startup TRANSPORT
   log lines in `main.rs:604–642` are byte-identical.
5. **P4 — `sparq-server` opts into T2/T3 (crate: `sparq-server`).** Idle-keepalive + max-requests
   made available (default-off, matching lws) alongside the existing `sq-lodb` body deadline,
   which stays on `tower_http`. *Acceptance:* WebSocket upgrade (`/subscriptions`) unaffected;
   `.with_upgrades()` path untouched.
6. **P5 — the re-divergence tripwire.** See §7 — scope it honestly, it is smaller than it sounds.
7. **P6 — upstream (needs:user for the filing).** Re-verify upstream HEAD first (§5). If still
   missing, open the two draft PRs at the homes argued in §5, @jeswr tagged, and record them in
   `docs/upstream-proposals.md`.

## 7. Guarding against re-divergence — what is actually achievable

The acceptance criterion asks for "a mutation tripwire or test". Be honest about what works:

- **Achievable and worth it:** a single cross-crate test asserting all three servers resolve
  `max_connections` / `max_connections_per_ip` from the *same* consts. Once the consts have one
  home, a fourth copy of the number is a compile-visible choice, not an accident.
- **Achievable and cheap:** a `scripts/` grep gate that fails if a new `fn is_internal_ip` is
  defined anywhere outside the new crate. Narrow, deterministic, no false positives.
- **Not achievable:** a general "someone re-implemented a semaphore connection cap" detector.
  Any such pattern-match would be either trivially evaded or noisy. Do not put it in a bead.

## 8. Open questions for the maintainer

1. **Crate name and publication.** The recommendation is `publish = false` first (§4). If the
   crates.io story is the *point* of the issue, say so and P0 changes shape (it then owes a
   registered bench + a `skills/<surface>/SKILL.md` up front, per G1).
2. **`sparq-server` cap defaults.** Should P1's connection cap ship **on** by default? Turning it
   on is a behaviour change for every existing deployment; leaving it off means the gap stays open
   for anyone who does not read the changelog. The lws defaults (10_000 global / 512 per-IP) are
   lenient enough that on-by-default looks safe, but this is a policy call, not an engineering one.
3. **Env-var naming.** §3.4 keeps `SOLID_SERVER_*` for lws and `SPARQ_*` for the server, i.e. two
   prefixes for one config struct. Acceptable, or should lws take a deprecation cycle onto a
   single prefix?
4. **Upstream homes.** §5 argues `tower-http` and `tokio-util` over `hyper-util`. Confirm before
   P6 files anything.
5. **`sq-gg0qq.4` (the "pending lws-core 3-crate split").** The issue asks to coordinate the
   boundary with it, but that bead is **not discoverable in this checkout** — grep finds only
   `sq-gg0qq.2` (the import) and `sq-gg0qq.5` (WAC fail-closed) in
   `compliance/`, `orchestration/start-here.toml`, and `fuzz/`. If the split is live, P3's
   boundary should be reviewed against it; if it is stale, the issue's coordination clause can be
   dropped.

## 9. Uncertainties

- **Upstream API state is unverified** (§5) — no network access in this environment. Treat the
  "two missing knobs" claim as sourced to the in-tree pin only.
- **The test split is inventoried but not compiled.** All 35 test functions were read and assigned
  to a tier by their unit under test (§2.1), and none depends on a `sparq-lws-core` domain fixture —
  they use `parse_*`, `ConnectionLimiter`, the wrappers, and tokio/http/tower directly. What is
  *not* verified is that each subset compiles once split: the module is one `mod tests { use
  super::*; }` block, so shared `use` items and helpers (e.g. `pub_ip`, `:1797`) have to be
  partitioned along with the tests. Expect per-phase import churn, not per-phase rewrites.
- **No behaviour of this design was executed.** Nothing here has been compiled; the seam is argued
  from the source, not demonstrated. The first bead to touch code should expect at least one
  surprise in the `Accept` trait bounds (`transport.rs:680–688` carries a five-parameter
  `where` clause that the plain-IO entry point does not need but the adapter still must satisfy).
