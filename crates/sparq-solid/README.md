<!-- [OPUS-4.8] sq-inzv: README brought to template. -->
# sparq-solid

<p>
  <a href="https://crates.io/crates/sparq-solid"><img src="https://img.shields.io/crates/v/sparq-solid.svg" alt="crates.io"></a>
  <a href="https://docs.rs/sparq-solid"><img src="https://docs.rs/sparq-solid/badge.svg" alt="docs.rs"></a>
  <a href="../../LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

**Solid Pod access control** over the [sparq](../../README.md) engine. Pods are stored as **named graph
per document**; their WAC (`.acl`) / ACP (`.acr`) docs stay as plain, queryable triples, and their
semantics are encoded as **N3 rules** (run by `sparq-reason`) that materialize a queryable authorization
view in `<urn:sparq:auth>`. Every SPARQL query is then filtered per `(WebID, client)` session to the
authorized graph set — **fail-closed**, with zero Solid-specific code in the engine (this crate is
depended on by nothing else in the workspace).

## 🚀 Quickstart

```rust
# // [OPUS-4.8] hidden main returns Result<(), String>: engine API errors are `String` (no Error impl).
# fn main() -> Result<(), String> {
use sparq_core::Graph;
use sparq_solid::{Mode, PodStore, Session};
// A pod is a dataset, one named graph per document (here the bundled fixture).
let graph = Graph::load_dataset(&sparq_solid::wac_fixture(), "nquads")?;
let mut store = PodStore::new(graph);
store.materialize_wac()?; // run the N3 rules → install <urn:sparq:auth>
// The SAME query, different sessions, different results — fail-closed.
let q = "SELECT ?title WHERE { ?s <https://ex.dev/ns#title> ?title }";
let alice = Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
let _authorized = store.query_as(&alice, Mode::Read, q)?.rows.len();
let _public_only = store.query_as(&Session::default(), Mode::Read, q)?.rows.len();
# Ok(()) }
```

## ✨ Features

- **WAC + ACP** — Web Access Control (`.acl`) and Access Control Policy (`.acr`): inheritance, agent
  classes, groups, `allOf`/`anyOf`/`noneOf`, the ACP `(agent, client, issuer)` principal,
  `acp:CreatorAgent`/`acp:OwnerAgent`, normative deny-overrides.
- **Trusted caller-asserted channels — creator/owner and verified credentials** — `acp:CreatorAgent`/`acp:OwnerAgent` resolve only against per-resource WebIDs the storage layer supplies through the trusted `AccessProvenance` channel, and `acp:vc <requirement>` (`sq-ysv3u`) only against holdings supplied through the trusted `VerifiedCredentials` channel — **never** graph content (the loader hard-rejects `solidx:` triples, so a writer cannot self-grant). `acp:vc` matches its requirement by exact IRI, conjunctively with `acp:agent`/`acp:client`/`acp:issuer`, and is **fail-closed**: with no credential supplied it accepts nobody (before `sq-ysv3u`, `acp:vc` was an unrecognized attribute, so a credential-gated matcher looked agent-unconstrained and granted everyone, anonymous included).
  Verification never runs in the reasoner — the opt-in `acp-vc` feature adds the [`sparq-vc`](../sparq-vc) **trust-the-issuer** backend that populates the channel (`VcRequirement` + `VerifiedCredentials::admit_data_integrity`: W3C Data Integrity `eddsa-rdfc-2022`, checking issuer, credential type and exact-match claims). Authenticity/integrity only — **no** privacy, unlinkability or selective disclosure, and no revocation or expiry check; a zero-knowledge backend would need the ZK estate, whose external crypto audit is pending (`sq-qhy4`).
- **Triples-native + zero-copy enforcement** — pods, ACL/ACR docs, and the auth view are all ordinary
  named graphs ("who can read G?" is one SPARQL pattern); the default path evaluates through the engine's
  zero-copy dataset view, with a v1 `FROM NAMED` rewrite as a standard-SPARQL portability path. The default-graph semantics are **spec-compliant by default** ([solid-sparql-query](https://github.com/jeswr/solid-sparql-query) Editor's Draft, issue #1546): the standing default graph is **empty** and a bare `{ ?s ?p ?o }` opts into the authorized union only via `FROM <…#union-default-graph>`; the opt-in `legacy-union-default-graph` feature reinstates the old union-always default — see the SKILL.
- **Write-path gating, WAC-Allow, per-resource decision + ACL write-through (#992 FR-1/3/5/6/7)** — `update_as` gates writes (`update_as_with_budget`/`update_as_acp_with_budget` are the twins that run the same gate under a cooperative `QueryBudget`, for a caller that must bound every SPARQL *evaluation* it issues — the budget does not reach `CLEAR`/`DROP`/`LOAD`, whose cost is set by the store and by loaded files rather than by the request; see the SKILL — `sq-yhlf0`); `put_acl`/`delete_acl` are the authoritative LDP-`PUT`/`DELETE`-on-`.acl` write-through (atomic graph replace + re-materialize, fail-closed rollback; a single-`.acl` write uses **diff-based** invalidation — `reindex_with` diffs old vs new `AuthIndex` per-origin and invalidates exactly the origins whose buckets changed, so other pods stay warm and cross-origin dependencies (WAC agentGroup, foreign-subject grants, ACP cross-doc indirection) are caught automatically; sound by construction, `sq-b7k7u`); `wac_allow` builds the header; `decide`/`decide_batch`/`resolve_acl` answer the per-request *"may X do M on R?"* with the
  governing-ACL + `accessTo`/`default` scope (FR-5: `WacDecision::acl_link_header()` → the `Link: rel="acl"` value) and a typed fail-closed `AclStatus` (absent/unloaded/transient ⇒ deny, never open — for a server's 403-vs-503 mapping; the shipped `solid-authz` shell maps every definitive deny to 403 — an LDP resource server owes an *anonymous* requester the Solid-required 401 + `WWW-Authenticate` challenge and must add that lane from its own authentication state, a known limitation documented on `AclStatus`). `decide_create` adds the **create** decision: a container child needs only `acl:Append` but an access-control document is governed by `acl:Control`, so the child NAME goes through `is_control_document_name` (case variants, container-child trailing slash, percent-encoded spellings) BEFORE the mode question and `.acl`/`.acr` is refused for every principal — `Control` holders included — as a definitive, non-retryable deny; the legitimate route to author an ACL stays the `Control`-gated write of the governed resource's own ACL.
- **ODRL bridge (opt-in `odrl-bridge`, research-track — not a production cutover)** — runs the
  [`sparq-policy`](../sparq-policy) ODRL evaluator and materializes the equivalent WAC/ACP grant (or dual
  `auth:deny*`) into the auth view — no new enforcement engine (zero ODRL code by default; see below).
- **Trust-graph admission PoC (opt-in `trust-graph`, research — NOT a security guarantee)** — a [`sparq-trust`](../sparq-trust)
  admission stratum injects an issuer-signed, trusted-source-scoped credential fact ahead of the materialiser; OFF = byte-identical WAC/ACP. No privacy/ZK (`sq-qhy4` unaudited).
- **Pattern-scoped masking (opt-in `pattern-scope`, spike `sq-lrtc3.3`)** — an ODRL target as allow/deny **triple patterns** over a
  graph, enforced by **materializing the masked sub-graph replica** (masked triples physically absent ⇒ oracle-equivalent under `OPTIONAL`/`EXISTS`/`MINUS`/aggregates/`COUNT` by construction). Design: `research/odrl-pattern-scoped-targets-2026-07.md`; envelope: `bench/pattern-scope/`.
- **Concurrent reads (`&self`, no feature flag)** — every read entry point (`accessible`, `view_for`,
  `query_as`/`query_json_as`/`ask_as`, `wac_allow`) takes `&self`, so N threads sharing one `Arc<PodStore>` query at once via a **sharded + bounded** session cache (interior `RwLock` stripes, LRU eviction); writes stay `&mut self`.

## ODRL → AUTH_GRAPH bridge (opt-in `odrl-bridge`)

A matched ODRL `Permission` becomes a concrete `principal auth:<mode> graph` triple **appended** to
`<urn:sparq:auth>`; the *request* action maps conservatively to the narrowest WAC mode (`odrl:use`
deliberately **unmapped → no grant**), and a Prohibition maps to the dual `auth:deny*`. **Fail-closed:**
a grant materializes **only** on a definite Permit + mappable action + concrete party + target (a deny
**only** on a genuine prohibition match); a Deny, unsatisfied constraint, undischarged duty, unmapped
action, or partyless/targetless request materializes **nothing**.

**SPARQL-query action contract ([SONNET-4.6] sq-lrtc3.2).** Represent a query request with standard `odrl:read`, which maps exactly to `Mode::Read`; sparq does not mint a profile-specific query action IRI. A request carrying only the `odrl:use` umbrella remains unmapped and grants no `query_as` visibility. Although ODRL defines `read` below `use`, hierarchy matching is the policy evaluator's concern: the bridge maps the concrete request action and will not guess that the broader `use` request meant read rather than one of the mutation actions it also covers.

**Conditions, refresh & revocation.** A *faithfully-mappable* constraint (`materialize_odrl_permission_conditional`) persists as a per-session-rechecked ACP `auth:ConditionalGrant`
(recipient/assignee matchers; an inclusive `odrl:dateTime` window vs `Session::now`, **fail-closed with no
clock**); `purpose`/`count`/strict bounds have no stateless analogue and stay **one-shot** (any unmappable
constraint falls the rule back to one-shot). Bridged grants are ledger-tracked; `refresh_odrl_grant(s)`
rebuilds the view (static baseline + replay of valid entries), retracting lapsed ones. **Deny retraction is asymmetric (fail-OPEN risk):** an `auth:deny*` is retracted **only** on a *definite* `Withdrawn` verdict. Full detail in the SKILL.

## WASM support

`sparq-solid` (with its `sparq-reason` dep, `default-features = false`) **compiles for AND runs on
`wasm32-unknown-unknown`** (no native-only deps — rayon off, `sparq-reason`'s parallel path gated). Its
transitive `oxrdf 0.3.3 → rand → getrandom` needs the host bundle to select a wasm RNG backend as
`sparq-wasm` does (`getrandom`'s `wasm_js` feature + the `getrandom_backend` cfg; see the
[migration guide](../../docs/migrating-from-oxigraph.md#wasm-compilation)). **Timing on wasm32:**
`std::time::Instant` panics (no monotonic clock), so `materialize_wac`/`materialize_acp` `cfg`-gate the
wall-clock plumbing off and report `stats.millis == 0.0` (rest of `MaterializeStats` unchanged); a wasm32
smoke test (`tests/wasm_materialize.rs`, `wasm-pack test --node`) guards it. Deno-wasm / Workers run-feasible.

## Conformance, security & containment

- **WAC + ACP conformance harnesses** (`sparq_solid::wac_conformance` / `conformance`) assert the
  engine against the [WAC](https://solidproject.org/TR/wac) / [ACP](https://solidproject.org/TR/acp)
  specs at the *library* level (data-declared `(agent, client, mode, resource) → allow|deny`). **Scope
  (honest):** the realistic library-level oracle, **not** the Solid CTH-over-HTTP (no HTTP surface here)
  — see `research/sparq-solid-scope.md` §4.
- **In-repo differential oracle** (`tests/differential_oracle.rs`) runs the shared corpus through THREE
  deciders — the engine (N3 rules), an **independent procedural reference evaluator** (`tests/reference/`,
  no shared code) and the hand `Expect` table — asserting **zero divergence**. A correctness oracle,
  **not** a security audit.
- **Security posture — fail-closed.** Absence of a grant makes a graph **invisible**. The reasoner is fed
  only ACL/ACR + structural facts — **never pod content** — so no writable document can grant itself
  access; the reserved `urn:sparq:` namespace is rejected on input and forged `<urn:sparq:auth>` graphs
  are stripped at load. `ldp:contains` is PSS-written opaque content, never derived from IRI structure or
  read into the reasoner; containment *ancestry* drives ACL inheritance only (`tests/containment_view_ownership.rs`).

## 📚 Learn more

- **How-to** — [`skills/access-control/SKILL.md`](../../skills/access-control/SKILL.md) (public API,
  WAC/ACP notes, conformance harnesses + the differential oracle, the **request-pipeline / WAC-Allow
  example**, ODRL-bridge mapping detail).
- **Design + threat model** —
  [`research/solid-access-control-design.md`](../../research/solid-access-control-design.md) (model,
  matrix, strata, boundaries) + [scope](../../research/sparq-solid-scope.md).
- **API reference** — [docs.rs/sparq-solid](https://docs.rs/sparq-solid); walk-through `cargo run -p
  sparq-solid --example quickstart --release`. Migrating from Oxigraph?
  [`docs/migrating-from-oxigraph.md`](../../docs/migrating-from-oxigraph.md).
- **Performance / Contribute** — [benchmarks dashboard](https://sparq.jeswr.org/dev/bench)
  (`--example bench`); [`AGENTS.md`](../../AGENTS.md), [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

## License

[MIT](../../LICENSE).
