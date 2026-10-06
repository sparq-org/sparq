<!-- [OPUS-4.8] sq-4kr5: README for the library-internal serving core. It is
     PUBLISHABLE (no `publish = false`) only because the published `sparq-server`
     depends on it — a crates.io crate cannot depend on a `publish = false` crate;
     it has no standalone public API surface of its own. Keep this in sync with
     crates/sparq-serve/Cargo.toml. -->
<!-- [OPUS-4.8] sq-inzv: README brought to template. -->
# sparq-serve

The **concurrent-serving core** of [sparq](../../README.md): a lock-free
*generation ring* — an arc-swapped chain of immutable store snapshots with
bounded retention and per-pod epoch vectors — plus the single **sequenced
writer** with group-commit batching that publishes those snapshots.

Why it exists: readers load the current generation in tens of nanoseconds and **never block the
writer**, and the writer **never waits for readers** or reclaims in place — old generations are
freed by ordinary `Arc` drop. This replaced the double-buffered snapshot scheme whose
pinned-snapshot writer stalls and reclaim-poll degradation motivated the redesign.

The crate is **sync, runtime-agnostic, and library-first**: it exposes no HTTP
or async-runtime types (consumers such as `sparq-server` wrap it), and it must
never enter `sparq-wasm`'s dependency graph.

> **Mostly library-internal plumbing — with one deliberate public surface** (the
> `embed` seam below); otherwise wrapped by `sparq-server`. Publishable (not
> `publish = false`) because `sparq-server` depends on it.

## 🔌 `embed` — in-process embedding seam (#1248)

[OPUS-4.8] The `embed` module is a documented facade that lets an external host
(e.g. `solid-server-rs`) call the engine **in-process** instead of over HTTP — the
read/write/probe entry points (`query_json` / `ask` / `update_in_place` /
`apply_delta_nquads` / `exists`+`metadata`, thin wrappers, no new behaviour) plus a
re-export of the runtime-agnostic concurrency wrapper (`GenerationRing` +
`GraphApplier` / `Writer`, factored out of `sparq-server`'s axum/tokio glue so an
embedder reuses sparq's *tested* fork → update → publish + generation-pinning model).
It is the proposed **API tier-1 (proposed-stable)** embedding surface (the
[API stability policy](../../docs/api-stability.md)) — but **NOT yet frozen**: the
semver freeze is the maintainer's to ratify ([#1248](https://github.com/sparq-org/sparq/issues/1248)),
and until then a minor pre-`1.0` release MAY still change it.

## 🚀 Quickstart

This crate has no standalone surface — use it through
[`sparq-server`](../sparq-server/README.md), which wraps the generation ring and
sequenced writer behind the HTTP endpoint:

```sh
cargo run -p sparq-server -- --format turtle data.ttl
```

## ✨ Features

- **Lock-free generation ring** — readers pin the current immutable snapshot in
  tens of nanoseconds and never block the writer; old generations are freed by
  ordinary `Arc` drop (no in-place reclaim, no reclaim-poll degradation).
- **Single sequenced writer** — group-commit batching publishes each batch as one new immutable
  generation; serialisability is by construction. Out-of-band ops (`Writer::maintain` for durable
  WAL compaction; `Writer::restore` for a crash-safe restore-into-durable) are sequenced through
  the same queue, so they run strictly between batches — never racing a commit.
- **Sync, runtime-agnostic, library-first** — no HTTP or async-runtime types;
  consumers wrap it. It must never enter `sparq-wasm`'s dependency graph.
- **Online backup/restore** (opt-in `backup` feature, default OFF) — `backup::export` serialises
  an already-immutable pinned `Generation` to one self-describing artifact **while serving** (no
  stop-the-world); `backup::import` re-hydrates a `Graph` from one, **fail-closed** on a corrupt
  or mismatched artifact. `sparq-server` mounts `/admin/backup` + `/admin/restore`. At-rest
  encryption is out of scope. (Same feature: `backup_delta` incremental-delta / point-in-time
  recovery between **same-lineage** generations — see rustdoc.)
- **Durable change-data-capture stream** *(opt-in `change-stream`, OFF by default)* —
  `change_stream::ChangeLog` persists each commit as an ordered, monotonically-sequenced record to
  a segmented, fsync'd append-only log (Neptune-Streams shape), appended on the writer thread (one
  per generation) via `ChangeLog::into_commit_hook` + `Writer::spawn_with_commit_hook`.
  `poll(from_seq)` **replays after a restart**; retention (`apply_retention`) drops old segments (a
  trimmed poll **fails closed**); `rebase_to` resyncs a broken stream (honest gap). No HTTP/async.
- **External-broker sink seam** *(opt-in `change-sink`, OFF by default)* — a `ChangeSink` trait +
  resumable `BrokerRelay` pump over that log (durable watermark, at-least-once, run OFF the writer
  thread), with one std-only `NatsSink` (core NATS, plain TCP, no TLS). Kafka / TLS / SASL: impl
  the trait over your own client — no broker client or async runtime enters this crate.
- **Response-bytes result cache** *(opt-in `result-cache`, OFF by default)* — see below.
- **Prepared-update applier** *(opt-in `params`, OFF by default)* — `PreparedGraphApplier`
  applies a parsed-once, bound `PreparedUpdate`, avoiding `GraphApplier`'s per-commit re-parse.

## 🗃️ Result cache (opt-in, `result-cache` feature)

A serving-layer cache from a request *identity* to the complete pre-serialized
response body, so a repeated read returns bytes without re-executing. **OFF by
default**; the default build carries zero cache code and no extra dependency.

- **Key = (canonical-query × visibility-scope × per-pod epoch-vector)** (design
  §6.3). The query is cheaply canonicalized (whitespace, and opt-in variable
  renaming). The **visibility scope** is the identity of the *accessible graph set*
  a request runs under — derive a `ScopeKey` from
  `sparq_solid::AuthIndex::accessible(session, mode)`, **never** from the WebID
  (the Hasura lesson). Many WebIDs that share one public-read scope collapse to one
  key.
- **Access-control isolation is correctness, not privacy.** Bytes cached for one
  scope can never be served to a different scope (a different scope MUST miss —
  tested). This enforces the access-control boundary the auth layer defines; it is
  **not itself a confidentiality/privacy guarantee** (no cryptographic claim; it
  trusts a faithfully-derived scope key).
- **Invalidation = per-pod (per-named-graph) epoch bumps.** Each entry records the
  epoch of the graphs its query touched; a write to any of them makes it stale.
  Queries with an unbounded read footprint pin the global generation (invalidated
  by any write).
- **Single-flight leases** collapse a stampede on a hot uncached key into one
  execution + N waiters; **byte-budget LRU + admission** never cache oversize bodies.

The cache stores opaque `Arc<[u8]>` bodies + a caller-derived `ScopeKey`; it never
depends on `sparq-solid` and never parses a query, and is a **different layer** from
`sparq-engine`'s embedded `result-cache` (the in-engine algebra-keyed LRU).

## 📚 Learn more

- **Design** — [`research/concurrent-serving.md`](../../research/concurrent-serving.md) §6.
- **API reference** — [docs.rs/sparq-serve](https://docs.rs/sparq-serve).
- **Consumer** — [`sparq-server`](../sparq-server/README.md) (the HTTP wrapper).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
