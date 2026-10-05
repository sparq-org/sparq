# Splitting `sparq-lws-core` into three crates — decision package (sq-gg0qq.4) [OPUS-5]

> **Status: DESIGN RECORD / DECISION PACKAGE — not an implementation, and not a
> ratified plan.** It exists because the handover partition this bead was written
> against does **not** survive contact with the code: §1 records that correction, §2
> the measurements behind it, §4 the three ways out, §5 a recommendation, §6 the
> phased bead plan, §7 what genuinely needs the maintainer.
>
> Bead: **sq-gg0qq.4** · Issue: **#2747** · Parent: **#2572** / `sq-gg0qq`.
> Sibling beads in the program: `.1` supply-chain pre-flight, `.2` the crate import,
> `.3` the in-workspace embedded-engine binding, `.5` the WAC-bypass fix.
>
> Author: Claude Opus 5. Every count in §2 was taken from
> `crates/sparq-lws-core` at commit `7f0cfbce`; no figure here is quoted from the
> handover or from a prior record. **No timings appear in this document** — the split
> question is structural, and the perf question it raises (§4, option A) is stated as a
> proof obligation, not a measurement.

---

## 1. The premise correction

The bead's acceptance criterion is:

> all pre-split tests still green, **relocated not rewritten**; workspace builds with
> server crates NOT in any default feature of core crates.

and the handover's §3 partition is:

- `sparq-lws-core` keeps: `Store` trait + embedded binding, blob backend, LDP plumbing,
  N3-PATCH, conditional/Range/conneg, SPARQL builder, **auth SEAM**, overload/rate-limit/TLS,
  notifications transport.
- `sparq-solid-server` gets: Solid-OIDC wiring, `authz/` (WAC), `identity.rs`, `seed.rs`,
  the `solid-server` bin.
- `sparq-lws` starts as a thin profile shell.

**Those two statements are mutually unsatisfiable against the current code.** The
modules the handover assigns to `sparq-solid-server` are not leaves — three of them are
*called by* modules the handover keeps in core:

| Moving module | Called from (core-retained) | Nature of the call |
| --- | --- | --- |
| `authz::wac` | `ldp/handler.rs`, `sparql_endpoint.rs` | `LdpState` **constructs** `WacAuthorizer` and decides on it |
| `identity` | `ldp/target.rs`, `app.rs` | reserved-namespace refusal + the router's gate layer |
| Solid-OIDC types | `app.rs`, `auth.rs`, `auth_cache.rs`, `pop/**`, `redis_replay.rs` | `AppState` is **generic over** `JwksProvider`/`ReplayStore` |

Only `seed.rs` and `main.rs` are genuine leaves that relocate untouched.

So the split as specified is a **refactor with a new trait seam**, not a file move. A
bead executed literally — `git mv` the modules, add two `Cargo.toml`s — cannot compile.
This record's job is to say so before someone spends a day discovering it, and to offer
the partitions that *are* reachable.

## 2. What the code actually looks like

Measured at `7f0cfbce` (`crates/sparq-lws-core`): **61 source files / 41 910 lines**,
**37 integration tests / 17 797 lines**, 8 examples.

### 2.1 The mass that would move

`authz/**` + `identity.rs` + `seed.rs` + `main.rs` = **7 859 lines**, ~19% of the crate.
The residue (~34 000 lines) stays in core. `ldp/handler.rs` alone is 6 226 lines and is
the single hottest coupling point.

### 2.2 The WAC coupling, precisely

`ldp/handler.rs` carries **42 lines** referencing `crate::authz`; `sparql_endpoint.rs`
carries **4**. The bindings imported are not incidental — they are the decision
vocabulary itself:

```rust
// crates/sparq-lws-core/src/ldp/handler.rs:42-44
use crate::authz::wac::{Decision, ReadDecision, WacAuthorizer};
use crate::authz::wac_allow::wac_allow_header;
use crate::authz::{mode_for_operation, AccessMode};
```

and there are **three construction sites in core-retained code** where the LDP layer
builds the authorizer itself out of its own store and ACL cache:

- `src/ldp/handler.rs:393` — `WacAuthorizer::with_cache(&self.store, &self.base_url, &self.acl_cache)`
- `src/ldp/handler.rs:544` — same
- `src/sparql_endpoint.rs:288` — same, off `state`

`WacAuthorizer` is `pub struct WacAuthorizer<'a, S: Store>` — a borrow-carrying,
`Store`-generic struct, not an object-safe trait. Additionally `is_acl_resource` /
`is_acl_auxiliary_suffix` (from `authz::mode`) are consulted at **twelve** points in the
handler for `.acl`-resource special-casing, and `acl_cache.rs` and
`ldp/public_read_skip.rs` — both core-retained under the handover — exist *only* to serve
the WAC path.

The honest reading: **WAC is not a module the LDP layer consults, it is part of how the
LDP layer is written.**

### 2.3 The auth coupling

```rust
// crates/sparq-lws-core/src/app.rs
pub struct AppState<J: JwksProvider, R: ReplayStore, S: Store> { … }
```

`JwksProvider` and `ReplayStore` come from `solid-oidc-verifier`. The handover asks core
to keep "the auth SEAM" while `sparq-solid-server` takes "Solid-OIDC wiring" — but core's
router type is *parameterised by Solid-OIDC traits*, and `pop/sk/**` (the DPoP-SK
proof-of-possession tier) and `redis_replay.rs` are written against
`solid_oidc_verifier::replay::*` directly. Core cannot drop the `solid-oidc-verifier`
dependency without first defining its own token/replay traits and adapting.

Encouragingly, the *volume* is small: 24 `solid_oidc_verifier::` references across 7
files, and the distinct surface is ~10 paths (`verifier::{Verifier, VerifiedToken}`,
`config::JwksProvider`, `replay::{ReplayStore, MarkResult, …}`, `SIGNING_ALGS`). This is
the tractable half of the split.

### 2.4 The tests do not partition either

**25 of 37** integration tests reference a moving surface; **23 of 37** go through
`tests/common/mod.rs`, which itself imports `solid_oidc_verifier::config::StaticJwksProvider`
and mints DPoP/cert-bound tokens. The harness boots the whole app, so almost every test
— including ones that are nominally about transport, rate-limiting, or counters — is a
`sparq-solid-server` test after the split.

That has a consequence the bead does not mention: **core would lose most of its
integration coverage to the new crate**, which the per-crate coverage ratchet will read
as a regression on `sparq-lws-core`, not a relocation. Whatever partition is chosen, the
ratchet baselines have to be re-derived in the same change.

### 2.5 There is already a working precedent for the seam

`authz::odrl` is *exactly* the shape the WAC seam would need, and it already ships:

```rust
// crates/sparq-lws-core/src/authz/odrl.rs
pub trait OdrlGate: Send + Sync {
    fn decide_read(&self, target_graph: &str, web_id: Option<&str>) -> OdrlVerdict;
}
```

held by `LdpState` as `Option<Arc<dyn OdrlGate>>` and attached at router assembly via
`LdpState::set_odrl_gate`, behind an opt-in `odrl-authz` feature that compiles to nothing
when off. `authz::trust_admit` follows the same pure-adapter discipline (and is
deliberately *not* wired into the handler). So the crate has already answered "can an
authorization decision live behind an object-safe, per-request, feature-gated seam here?"
— yes, twice.

## 3. Blast radius outside the crate

The bead scope reads as one crate; the change is not confined to one crate.

- **Container + registry.** `crates/sparq-lws-core/Dockerfile` copies and runs
  `/usr/local/bin/sparq-lws-core` (the implicit bin from `src/main.rs`; there is no
  `[[bin]]` stanza). `.github/workflows/lws-container.yml` builds it, smoke-tests it via
  `crates/sparq-lws-core/tests/container-smoke.sh`, Trivy-scans it, and publishes
  `ghcr.io/sparq-org/sparq-lws-core`. Renaming the binary to `solid-server` and moving it
  to a new crate changes the build context, the entrypoint, and the published image name.
- **Deploy surface.** `deploy/demo/sparq-lws-demo.yaml`, `deploy/gcp/sparq-lws.yaml`,
  `deploy/paas/lws/{Dockerfile,fly.toml}`, `deploy/terraform/main.tf`,
  `deploy/demo/compat/{docker-compose.yml,smoke.sh}` all reference the current names, and
  `.github/workflows/deploy-lint.yml:154` asserts the literal string
  `image: "ghcr.io/sparq-org/sparq-lws-core:latest"` in the rendered manifest.
- **Feature matrix.** `feature-matrix.yml` has a dedicated
  `sparq-lws-core --no-default-features` leg that asserts, via `cargo tree`, that
  `async-lock` / `spargebra` / `sparq-core` / `sparq-engine` stay out of the graph. Two new
  crates need the equivalent assertions or the "core stays lean" property is unpoliced.
- **Wasm.** `sparq-lws-wasm` depends on `sparq-lws-core` with `default-features = false,
  features = ["wasm"]`, and `authz` / `identity` / `seed` are declared **unconditionally**
  in `lib.rs` (no `cfg(not(wasm32))`). `tests/wasm_dependency_boundary.rs` polices that
  boundary. Moving `authz` out of core therefore changes what the wasm pod can decide, and
  `sparq-lws-wasm` seeds an owner ACL itself (`lib.rs:384 seed_owner_acl`).
- **Per-crate gate cost.** Each new crate owes: a ≤120-line README with the
  `🚀`/`✨`/`📚` + License sections (`scripts/check-readme-template.py`, hard gate),
  direct unit tests per public fn against the coverage ratchet, and a clean rustdoc
  all-features build.

None of this is a blocker. All of it is work the bead as written does not budget for, and
it is why §6 sequences the binary/container move **last**.

## 4. Options

### Option A — invert the dependency: `Authorizer` trait in core, WAC impl in `sparq-solid-server`

Define an object-safe `Authorizer` in core mirroring `OdrlGate`; `LdpState` holds
`Arc<dyn Authorizer>` instead of building `WacAuthorizer`; move `authz/{acl,wac,wac_allow}`
into `sparq-solid-server` as the impl. `authz::mode` (`AccessMode`, `is_acl_resource`,
`mode_for_operation`) stays in core as vocabulary, since the handler's twelve
`.acl`-special-casing sites are LDP naming rules, not WAC policy.

- (+) Delivers the handover's intent honestly: core becomes profile-agnostic, WAC becomes
  a pluggable profile decision, and a future ACP or SPARQ-native oracle drops in the same way.
- (+) Precedent-backed — `OdrlGate` proves the shape works in this crate (§2.5).
- (−) **This is a rewrite of the LDP authorization path**, so the bead's
  "relocated not rewritten" criterion is not met and must be renegotiated.
- (−) A dyn boundary lands on the request-authorization path. It is *per-request*, not
  per-row, so `scripts/check-no-dyn-dispatch.py` (scoped to the substrate hot loops) will
  not fire and the #1303 invariant is not literally engaged — but the crate ships
  `examples/{auth_hotpath_microbench,acl_cache_bench,read_response_alloc_microbench}.rs`
  precisely because this path is measured. **Perf-neutrality is a proof obligation of this
  option, not an assumption.** The borrow-carrying `WacAuthorizer<'a, S>` also has to be
  restructured to be object-safe, which is where the allocation risk sits.
- (−) Largest single change; hardest to review; touches the WAC path while `sq-gg0qq.5`
  (the WAC-bypass P1, which carries mandatory adversarial review) is still open.

### Option B — narrow the partition: WAC stays in core, only the genuinely-Solid layer moves

`sparq-solid-server` takes Solid-OIDC wiring, `identity.rs`, `seed.rs`, and the bin.
`authz/` stays where the LDP handler already uses it.

- (+) Nearly all of it *is* relocation, so the bead's own acceptance criterion holds.
- (+) Zero hot-path exposure; zero interaction with the open `sq-gg0qq.5` WAC work.
- (+) Still buys the real prize: core stops depending on `solid-oidc-verifier`, so a
  non-Solid LWS profile becomes expressible.
- (−) Does not match the handover's §3 partition. Core keeps WAC, so "protocol-agnostic
  substrate" is aspirational rather than achieved.
- (−) `identity` still needs a small seam: `ldp/target.rs` calls
  `identity::is_reserved_identity_path`. Cheapest fix is to keep the *reserved-namespace
  predicate* in core (it is an LDP path rule, and its security property — the namespace is
  refused whether or not id-hosting is on — is a core invariant) and move only the id-doc
  serving. Note the handover requires the id-host convention stay **byte-identical to PSS
  decisions/0020**, so nothing about the convention itself may change in the move.

### Option C — move `ldp/handler.rs` with `authz/` into `sparq-solid-server`

- (+) No new seam, no dyn, genuinely relocation.
- (−) Guts core: the handler is 6 226 lines and is what "LDP plumbing" *means*. Core
  would be a `Store` trait plus transport. Contradicts the handover directly and leaves
  `sparq-lws` with nothing to be a profile *of*. Recorded for completeness; not recommended.

## 5. Recommendation

**Do B first, then A as its own bead — and do not do either as one commit.**

The two are not competitors; they are phases. Option B is the part of the handover that
is real relocation, is independently valuable (core sheds `solid-oidc-verifier`), is
reviewable, and is safe to land while `sq-gg0qq.5` is open. Option A is a genuine
architectural change to the authorization path that deserves its own bead, its own
adversarial review, and its own perf evidence — and it is strictly easier to do *after*
B, because by then the crate boundary and the new test homes already exist.

Concretely: **this bead (`sq-gg0qq.4`) should be re-scoped to option B**, and its
acceptance criterion amended from "relocated not rewritten" to "relocated not rewritten,
**except** the `identity` reserved-path predicate and the auth trait seam, which are
enumerated in advance." A bead whose acceptance criterion is known to be unachievable
will either be failed honestly or met dishonestly; neither is a good outcome.

`sparq-lws` should be created empty-shell in the same change **only if** it has a
compiling reason to exist. On the evidence, it does not yet — conformance classes are a
later bead by the handover's own account, so an empty crate would be a placeholder that
the README/coverage/rustdoc gates all still charge rent for. Prefer to create it in the
phase that gives it content (§6, phase 5).

## 6. Phased plan (each phase = one future bead)

1. **Auth seam in core.** Define core-owned token/replay traits; adapt
   `solid-oidc-verifier` behind them; make `AppState` generic over the core traits rather
   than the vendor ones. Core still *depends* on the verifier at this point — the point is
   that nothing in core names it outside one adapter module. No crate created.
   *Acceptance:* `solid_oidc_verifier::` appears in exactly one core module; all 37 tests green.
2. **Identity predicate split.** Keep `is_reserved_identity_path` + `RESERVED_HANDLES` in
   core as an LDP path rule (preserving the flag-independent refusal property); isolate the
   id-doc *serving* surface behind one module boundary. Convention stays byte-identical to
   PSS `decisions/0020`. No crate created.
3. **Create `sparq-solid-server`.** Move the verifier adapter, id-doc serving, `seed.rs`,
   and the `main.rs` bin. Move the 25 affected integration tests and `tests/common/`.
   Re-derive both crates' coverage floors in the same change. Core must build with
   `--no-default-features` *without* `solid-oidc-verifier` in `cargo tree`; add that
   assertion to the `feature-matrix.yml` leg alongside the existing one.
4. **Container/deploy cutover.** Rename the bin to `solid-server`, move the `Dockerfile`,
   update the six deploy manifests and the `deploy-lint.yml` literal-string assertion, and
   decide the published image name. Deliberately last, and deliberately its own bead —
   this is the only phase that can break a running demo.
5. **Create `sparq-lws`** when the conformance-class work that gives it content is ready
   (the handover's own sequencing). Not before.
6. **Option A: `Authorizer` seam** (`ldp/handler.rs` → `Arc<dyn Authorizer>`, WAC impl into
   `sparq-solid-server`). Gated on `sq-gg0qq.5` being closed, and carrying an explicit
   perf-neutrality obligation on the authorization path against the crate's existing
   microbench examples. Adversarial review mandatory — this edits the WAC decision path.

Phases 1, 2 are prerequisites for 3; 4 depends on 3; 5 and 6 are independent of each other
and both depend on 3.

## 7. Open questions for the maintainer

1. **Is the handover's §3 partition binding, or was it indicative?** If binding, option A
   is forced and this bead's acceptance criterion must change. If indicative, option B is
   the better first cut. *This is the decision that unblocks everything else.*
2. **Published image name.** Does `ghcr.io/sparq-org/sparq-lws-core` stay (repointed at the
   new crate's Dockerfile), or become `ghcr.io/sparq-org/sparq-solid-server`? The second
   breaks any pinned consumer of the demo.
3. **Does the wasm pod keep WAC?** `sparq-lws-wasm` seeds an owner ACL and `authz` is
   unconditional in `lib.rs`. Under option A the wasm profile needs an authorizer impl from
   somewhere, and `sparq-solid-server` is a native server crate.
4. **Coverage ratchet.** Confirm that re-deriving `sparq-lws-core`'s floor downward, when
   the drop is provably a relocation of tests into a sibling crate, is acceptable — or
   whether core must grow direct unit tests to hold its floor in the same change.

## 8. What this record does not know

- **No timings.** Option A's perf risk is argued structurally (a dyn boundary on a path the
  crate bothers to microbenchmark) and is explicitly left as an obligation for phase 6, not
  a measured claim. Nothing here asserts a regression exists.
- **The handover document itself is not in this repo** (repo hygiene forbids `HANDOVER*.md`
  — AGENTS.md §repository hygiene). §1 quotes the partition as it reaches this bead via
  issue #2747; if the source document says something more specific, that supersedes §1's
  reading.
- **Cross-crate compile-time effect is unmeasured.** The `sparq-engine` split RFC found the
  compile-time argument weaker than hoped; no analogous measurement was taken here, and none
  of the recommendation rests on one.
- **`authz::trust_admit` is an opt-in, deliberately unwired research adapter** (`trust-graph`
  feature, off by default) and is treated here purely as mass that moves with `authz/`. No
  claim is made about its soundness; the standing caveat (sq-qhy4) applies.

---

*Related records:* [`solid-access-control-design.md`](./solid-access-control-design.md)
(WAC/ACP model and threat model), [`sparq-solid-scope.md`](./sparq-solid-scope.md)
(the PSS boundary — security-critical paths stay in PSS),
[`engine-split-rfc.md`](./engine-split-rfc.md) (the house method for a crate-split
decision package, and the #1303 perf-neutrality precedent).
