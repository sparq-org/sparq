# `sparq-mcp`: adopting the official Rust MCP SDK (`rmcp`) vs the hand-rolled JSON-RPC framing

**Status:** assessment record, 2026-07-28. Bead `sq-95zda`, gh #3219. Answers the two
preconditions the bead names — *vendored-policy-friendly* and *cargo-vet auditable* — with
measured evidence, and records a verdict plus the triggers that should reopen it.

**Verdict: DECLINE for now. Keep the hand-rolled framing.** Not because `rmcp` is bad — it
is the official SDK, it tracks the spec, and its transport layer is genuinely valuable —
but because at today's scope the trade is **146 lines of framing code for 45–52 extra
crates and an async runtime** in a crate whose defining property is that it has neither,
and **neither precondition in the bead title is met today**. The re-evaluation triggers are
in [Reopen this when](#reopen-this-when) — the strongest one is the HTTP/SSE transport,
where the calculus flips.

> 🤖 This record was written by a SPARQ agent.

## What is actually being compared

`crates/sparq-mcp/src` is **4,536 lines**. The part `rmcp` would replace is only:

| File | Lines | What it is |
| --- | --- | --- |
| `src/jsonrpc.rs` | 93 | `Request` / `Response` / `RpcError` + the standard error codes |
| `src/transport.rs` | 53 | the line-delimited stdio serve loop over `handle_message` |
| **total** | **146** | **3.2% of the crate** |

The other ~4,390 lines — `server.rs` dispatch, `tools.rs` schemas, `shapes.rs`, `nlq.rs`,
`solid.rs`, `notifications.rs`, `cli.rs` — are sparq-specific and are **not** replaced by
any SDK. So this is not "adopt the SDK instead of writing an MCP server"; it is "adopt the
SDK instead of ~146 lines of JSON-RPC struct definitions and a `for line in reader.lines()`
loop".

The framing is also not a conformance gap. `SUPPORTED_PROTOCOL_VERSIONS` in
`crates/sparq-mcp/src/server.rs:42` is `["2025-11-25", "2025-06-18", "2025-03-26",
"2024-11-05"]`; `rmcp-2.2.0/src/model.rs:163-166` defines exactly the same four. There is no
protocol revision sparq-mcp declines that adopting `rmcp` would win.

## Cost: the dependency delta (measured)

Measured against `rmcp` **2.2.0** (the current `max_stable_version`), by resolving a scratch
crate per feature tier and diffing package names against both `sparq-mcp`'s own closure and
the workspace `Cargo.lock`. Reproduce with the commands in [Reproduction](#reproduction).

`sparq-mcp`'s **current** default closure is **62 packages** (including itself), and
contains **no async runtime,
no `futures`, no `chrono`, no `tracing`, no `async-trait`**.

| `rmcp` feature tier | closure | new crates in `sparq-mcp`'s closure | new crates in the workspace lock |
| --- | --- | --- | --- |
| `default-features = false`, no features | 60 | **+45** | +1 (`rmcp`) |
| `["server", "transport-io"]` | 67 | **+52** | +8 |
| default (`base64`, `macros`, `server`) | 74 | **+59** | +13 |

Two things matter more than the raw counts:

1. **There is no runtime-free tier.** `tokio` (features `sync`/`macros`/`rt`/`time`),
   `futures`, `async-trait`, `chrono`, `tokio-util`, `tracing` and `pin-project-lite` are
   **unconditional** dependencies of `rmcp` — no feature flag removes them, and none of
   those seven is in `sparq-mcp`'s closure today. (`rmcp`'s other unconditional deps,
   `serde` / `serde_json` / `thiserror`, are already there and cost nothing.)
   Even the "just take the protocol types" tier drags a full async runtime plus a timezone
   database (`chrono` → `iana-time-zone` → `android_system_properties`,
   `core-foundation-sys`, `windows-*`, `js-sys`/`wasm-bindgen`) into a crate that today is
   pure `serde_json` over `String`s.
2. **The workspace-level delta looks small and is misleading.** Most of those 45 crates are
   already *somewhere* in the workspace lock (`sparq-server` and friends use `tokio`), so
   only 1–13 names are new to the lock. But `sparq-mcp` is an **opt-in leaf crate that
   nothing in the workspace depends on** — its whole design claim, stated in
   `crates/sparq-mcp/Cargo.toml:1-6`, is that it "pulls no heavy dependency". The number
   that measures that claim is the +45/+52, not the +1/+8.

## Precondition 1: cargo-vet auditable? — **No, not today**

The gating `supply-chain.yml` `vet` job requires every crate to be audited, covered by an
imported trusted audit set (Mozilla / Google / Bytecode-Alliance / ISRG / Embark / Zcash), or
hold an explicit `[[exemptions.*]]` entry. Status of the 8 crates new to the lock at the
`["server", "transport-io"]` tier, checked against the live contents of all six imported
audit URLs:

| crate | resolved | coverage found | verdict |
| --- | --- | --- | --- |
| `rmcp` | 2.2.0 | **none** — no `[[audits]]`, no `[[trusted]]`, in any of the six sets | needs exemption or first-party audit |
| `serde_derive_internals` | 0.30.0 | **none** | needs exemption or first-party audit |
| `schemars` | 1.2.2 | Embark 0.8.12 + Zcash deltas reaching 1.1.0 | chain stops short of 1.2.2 |
| `schemars_derive` | 1.2.2 | Embark 0.8.12 only | large gap |
| `pastey` | 0.2.3 | Bytecode-Alliance 0.2.1 | delta gap |
| `dyn-clone` | 1.0.20 | Zcash delta 1.0.19 → 1.0.20, `safe-to-deploy` | **covered** |
| `ref-cast` | 1.0.26 | Mozilla `[[trusted]]`, user-id 3618, `safe-to-deploy` | covered, but the entry's `end` is **2026-08-19** |
| `ref-cast-impl` | 1.0.26 | Mozilla `[[trusted]]`, user-id 3618, `safe-to-deploy` | covered, but the entry's `end` is **2026-08-19** |

So: **1 of 8 cleanly covered by an audit, 2 by a publisher-trust entry that expires within
about three weeks of this record, and 5 would require new exemptions or first-party audits** —
including `rmcp` itself, which is the whole point of the exercise. Adopting the SDK today
means adding the SDK to the bootstrap exemption list, which is precisely the ratchet the
gate exists to prevent.

**Honesty caveat on this table.** `rmcp` is not in the dependency graph, so `cargo vet`
cannot be run against it. The table was produced by fetching the six imported `audits.toml`
files and matching `[[audits.<crate>]]` / `[[trusted.<crate>]]` blocks against the versions
`cargo` actually resolved. That is a static approximation of what `cargo vet` computes — in
particular it does not fully model how delta chains compose, and it does not evaluate
whether Google's `ub-risk-2` criteria (the criteria on its `dyn-clone`/`ref-cast` audits) map
onto this repo's `safe-to-deploy` policy. The two `[[trusted]]`-based rows are the least
certain: `supply-chain/imports.lock` currently contains **zero** `[[trusted]]` entries, so
publisher trust is a mechanism this repo has never actually exercised. Treat the table as
"clearly not clean today", not as an exact exemption count. Any adoption PR must run the
real gate.

## Precondition 2: vendored-policy-friendly? — **Poorly suited**

`AGENTS.md` §*Upstream blockers — roll your own, then contribute back* says: when upstream
blocks us, vendor a local copy under `vendor/`
(or fork via `[patch.crates-io]`), ship it, and keep an upstream PR live for the delta. The
only crate carried that way today is `vendor/spargebra`.

- **License.** `rmcp` 2.2.0 is **Apache-2.0 only**. The workspace is MIT; the one existing
  vendored crate, `vendor/spargebra`, is `MIT OR Apache-2.0`. Apache-2.0 is compatible for
  distribution, but vendoring `rmcp` would introduce a second, non-dual-licensed license
  into the tree — a first for this repo, and a question for the maintainer rather than an
  agent.
- **Rebase cost.** `rmcp` has published **54 versions since 2025-03-16**, and **three major
  versions in five months**: 1.0.0 on 2026-03-03, 2.0.0 on 2026-06-29, and 3.0.0-beta.1 on
  2026-07-23 — with `3.0.0-beta.4` landing on the day this record was written. Carrying a
  vendored fork of a crate rebasing at that rate is a standing tax, and the vendoring policy
  is designed for occasional unblocking deltas, not for continuous tracking of a
  fast-moving upstream.
- **Which cuts the other way too.** That same cadence is an argument *for* letting the SDK
  absorb spec churn rather than hand-maintaining framing — but only once the framing
  surface is big enough to be worth it. At 146 lines it is not (see [Reopen this
  when](#reopen-this-when)).

## Architectural cost: the sync seam would have to go

`McpServer::handle_message(&str) -> Option<String>` is the crate's public seam and its
headline testability property: the dispatch core is a **synchronous, I/O-free data
transform**, which is why the round-trip tests exercise the real path with zero feature flags
and without spawning a process. There are **31 call sites** across `src/` and the test
suite (`tests/roundtrip.rs`, `tests/solid.rs`, `tests/dispatch_edges.rs`,
`tests/templates_text.rs`, `tests/describe_form.rs`, `tests/shacl.rs`, `tests/ask.rs`, and
`examples/mcp_roundtrip_bench.rs`).

`rmcp`'s server surface is an **async `ServerHandler` driven by a tokio service**. Adopting
it wholesale replaces a string-in/string-out function with a runtime-bound service — a
breaking change to the crate's main embedder seam, a rewrite of the test suite's driving
pattern, and the loss of the "no runtime needed to test protocol dispatch" property.

## MSRV: not a blocker, but unpinned upstream

`rmcp` 2.2.0 is **edition 2024** and declares **no `rust-version`**. The workspace floor is
`rust-version = "1.88"`. Measured directly: `rmcp` 2.2.0 with `["server", "transport-io"]`
**compiles cleanly on rustc 1.88.0**, so it would not break the `msrv` CI job today. But
because upstream publishes no MSRV, nothing constrains a future patch release from raising
it — which would red the `msrv` lane on a dependency bump rather than on a sparq change.

## What `rmcp` genuinely buys (recorded fairly)

Declining is not "the SDK has nothing". It has:

- **Transports sparq-mcp does not have** — streamable HTTP (client and server side), SSE,
  child-process client transport, Unix-socket variants. sparq-mcp ships only stdio plus the
  embeddable `handle_message`. This is where the real value is concentrated.
- **OAuth2 client auth** (`auth` / `auth-client-credentials-jwt` features).
- **Client-side types**, if sparq ever needs to *be* an MCP client rather than a server.
- **`schemars`-derived tool input schemas** instead of the hand-written JSON in `tools.rs`.
- **Spec-churn absorption** — someone else tracks the revisions.

None of these are needed by the current tool surface, and the first is already tracked
separately (the SSE/HTTP transport follow-up recorded in
`research/mcp-grounding-tools-decision.md`).

## Reopen this when

Re-evaluate when **any** of these becomes true:

1. **An HTTP/SSE MCP transport is actually required.** This is the decisive trigger. A
   hand-rolled streamable-HTTP transport with session resumption is a far larger surface
   than 146 lines, and at that point "+45 crates" buys something real instead of replacing
   a `for line in reader.lines()` loop.
   **FIRED and re-assessed, 2026-07-29** — `research/mcp-streamable-http-transport-design.md`
   (sq-2c0f0, gh #3221). The verdict **held**, for a reason this record did not measure: the
   workspace already contains a vetted axum SSE server (`sparq-server`'s
   `subscriptions::sse`), so hand-rolling the transport costs **+29 crates in `sparq-mcp`'s
   closure and 0 crates new to `Cargo.lock`** — no new exemption or audit — against `rmcp`'s
   +52/+8. Triggers 2–4 below are untouched.
2. **`rmcp` lands in an imported audit set**, or a first-party `safe-to-deploy` audit of it
   becomes affordable.
3. **`rmcp` declares a `rust-version` and stabilises** — no new major for at least two
   release cycles.
4. **sparq needs an MCP client**, not just a server.

## Recommended shape, if it is ever adopted

Do **not** replace `src/jsonrpc.rs` or invert the sync seam. Add a **new opt-in cargo
feature** (e.g. `rmcp-transport`, OFF by default) that wires an `rmcp` transport to the
**existing synchronous dispatch core**, keeping `handle_message` as the public seam and the
default build free of `rmcp` and tokio entirely. That is the same opt-in pattern already
used for `nlq`, `solid`, `text`, `shacl` and `templates`, it keeps every current embedder
working, and it confines the audit/exemption delta to a feature nobody has to enable.

## Reproduction

Every number above is re-derivable. The dependency deltas:

```sh
# current sparq-mcp closure (62)
cargo tree -p sparq-mcp --no-default-features -e normal --prefix none | sed 's/ (.*//' | sort -u

# per-tier rmcp closure: scratch crate + `cargo generate-lockfile`, then diff the
# `name = "..."` sets in the generated Cargo.lock against sparq-mcp's closure and
# against the workspace Cargo.lock.
```

The audit coverage table: fetch the six `url =` entries under `[imports.*]` in
`supply-chain/config.toml` and match `[[audits.<crate>]]` / `[[trusted.<crate>]]` blocks
against the resolved versions. The MSRV result: `cargo +1.88.0 build` on the
`["server", "transport-io"]` scratch crate. The release cadence:
`https://crates.io/api/v1/crates/rmcp/versions`.

All measurements are as of 2026-07-28 against `rmcp` 2.2.0; the audit sets and the crate's
version list both move, so re-run before acting on this record.

## Follow-ups (beads/issues, not TODOs)

- If the HTTP/SSE transport work is scheduled, re-run this comparison **first** — trigger 1
  above is the one that changes the answer, and the decision should be made before the
  transport is hand-rolled, not after.
- The Mozilla `[[trusted]]` entries for `ref-cast` / `ref-cast-impl` carry `end =
  "2026-08-19"`. Those crates are not in the tree today, so nothing is currently at risk —
  but any future dependency that pulls them in inherits an expiring trust window.
