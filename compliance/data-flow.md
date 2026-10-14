<!-- [OPUS-4.8] sq-toze — SHARED data-flow doc (owned by the Privacy framework engineer).
     Every place the binary can touch data + the operator/engine responsibility split.
     Referenced by privacy, iso27001, cra, soc2. Re-review when Fable returns. -->

# Data-flow — what the sparq binary can touch

A **shared** compliance artifact (owned by the Privacy framework, cross-referenced by ISO 27001,
CRA, and SOC 2). It answers one question honestly: **everywhere a personal datum could flow
through the sparq binary**, and for each, **whether handling it is the engine's job or the
operator's**. It deliberately maps *what the code can touch*, not an idealised deployment.

> **Frame.** sparq is a data *engine*. It has **no notion of "personal data"** — every RDF triple
> is handled identically. So this doc maps *data in general*; whether any given triple is personal
> data, and the lawful basis for processing it, is the **operator's** determination. The STRIDE
> security model for the same surfaces lives at [`../research/threat-model.md`](../research/threat-model.md)
> (referenced, not forked); this doc is the *privacy/data-handling* view.

## 0 — Responsibility split (the load-bearing table)

| Concern | sparq (engine) | Operator (controller/deployer) |
|---|---|---|
| **What data is loaded** | — (loads whatever it is given) | **Decides** the dataset, source, lawful basis, purpose |
| **In-memory processing** | Parses → Dict/index → query → results | Runs the process in a trusted environment |
| **At-rest persistence** | Optional plaintext WAL (`--persist`), **off by default** | **Owns** disk/volume encryption, FS permissions, backup retention |
| **In-transit** | Serves plaintext HTTP | **Owns** TLS termination (gateway/reverse proxy) |
| **Who may query** | Optional Bearer auth; `sparq-solid` graph-level authz (fail-closed) | **Configures** auth; owns identity/session if no `sparq-solid` |
| **Logging / audit** | OFF by default; opt-in `--verbose` request trace; aggregate-only metrics | **Decides** whether to log; owns log retention/redaction |
| **Erasure / retention** | Provides SPARQL `DELETE`/`DROP`/`CLEAR` + WAL (must rotate) | **Owns** the retention policy + runs erasure + rotates WAL |
| **Egress (federation)** | `SERVICE` deny-all by default; explicit allowlist | **Decides** the allowlist; owns the trust in remote endpoints |
| **Cross-border transfer / DPA / consent / RoPA / breach notice** | — (no such concept in the engine) | **Wholly owns** |

## 1 — Data sources (INGRESS — where data enters the binary)

| Source | Surface | Crate / entry point | Personal-data risk | Owner |
|---|---|---|---|---|
| **RDF file / stdin / HTTP body** | parse → in-memory graph | `sparq_core::Graph::load_str` / `load_dataset` / `load_reader` (`crates/sparq-core/src/lib.rs:632,767,994`); CLI `open_reader` (`crates/sparq-cli/src/main.rs:775`) | The dataset is whatever the operator ingests — **may be entirely personal data** | **Operator** chooses; engine parses |
| **Compressed RDF (gzip/zstd/bzip2)** | fused decompress → parse | `sparq-core` fused-decompress ingest | Same as above (just compressed on the wire/disk) | **Operator** |
| **SPARQL query text** | HTTP `/sparql` (GET/POST), CLI, WASM | `sparq-server` `http.rs`; `spargebra` parse | Query *predicates/filters* can embed personal data (e.g. an email literal in a `FILTER`) | Caller supplies; engine evaluates |
| **SPARQL UPDATE (`INSERT`/`DELETE`/`LOAD`)** | HTTP `/sparql` write, CLI | `sparq-engine/src/update.rs` | Writes personal data into the store; `LOAD <url>` pulls a remote document | **Operator** gates writes (auth/solid) |
| **`SERVICE` federation fetch** | outbound to a remote endpoint | `sparq-engine-service/src/service.rs` (feature-gated) | Could *exfiltrate* a sub-query / pull remote personal data — **deny-all by default** (SSRF/exfil guard) | **Operator** sets the allowlist |
| **On-disk index (mmap)** | load a persisted store | `sparq-core` mmap loader (boundary B5) | The persisted dataset re-enters memory; a *tampered* file is a memory-safety risk (covered by `compliance/memsafety/`) | **Operator** controls the file |

## 2 — Data at rest (STORAGE)

| State | Where | Default | Encryption | Erasure | Owner |
|---|---|---|---|---|---|
| **In-memory graph (Dict/index)** | process RAM | always | — (RAM) | gone on process exit | engine holds; operator runs the process |
| **`--persist DIR` write-ahead log** | operator filesystem | **OFF** (in-memory only) | **none (plaintext)** — operator's disk encryption | `DELETE`/`DROP` + **must rotate/compact the WAL** (see PR-G3) | **Operator** (encryption, FS perms, rotation) |
| **Compressed `.spq` / HDT archive** | operator filesystem | — | none | re-generate / delete the file | **Operator** |

> **Erasure caveat (privacy-critical).** The `--persist` WAL is **append/replay**. A `DELETE`d
> triple may remain in earlier WAL segments until the operator **compacts/rotates** the directory.
> A complete GDPR Art. 17 erasure of the *persisted* store therefore requires both the SPARQL
> `DELETE`/`DROP` **and** a WAL rotation. Tracked: `compliance/privacy/gap-register.md` PR-G3
> (bead **sq-toze.33**, a retention/erasure operator runbook).

## 3 — Data in transit (TRANSPORT)

| Path | Encryption | Auth | Owner |
|---|---|---|---|
| **Client ↔ `sparq-server` (HTTP)** | **plaintext** — operator terminates TLS at a gateway/reverse proxy | optional Bearer (`--auth-token[-read]`, constant-time) | **Operator** owns TLS |
| **WebSocket / SSE subscriptions** | plaintext (wss via the operator's proxy) | gated by `--auth-token-read` (header or `Sec-WebSocket-Protocol: bearer.<token>`) | **Operator** owns TLS |
| **`SERVICE` egress** | per the remote endpoint | **deny-all** unless allowlisted | **Operator** sets allowlist |
| **WASM/JS client** | **none leaves the client** — engine runs in the browser tab | the embedding app's concern | **Embedding app** |

## 4 — Data observability (LOGS / METRICS — the minimisation surface)

| Sink | Default | Content | Personal-data risk |
|---|---|---|---|
| **Request log (`TraceLayer`)** | **OFF** (opt-in `--verbose`) | full request lines incl. **SPARQL query text** when on | **Query text can embed personal data** when enabled — redaction gap PR-G4 (bead sq-toze.34) |
| **Startup banner (`eprintln!`)** | on | triple counts, persist/auth/bind posture | **no per-request data, no PII** |
| **Prometheus `/metrics`** | on (scrape) | **aggregate counters + latency histogram + gauges only** | **no query text, rows, IPs, or identifiers** — static route labels only |
| **Error responses** | on | **generic structured JSON** ("not found", "internal server error", budget limits) | **no data echoed** — one residual: UPDATE *parse* errors echo a query fragment (PR-G1, bead sq-toze.32) |

**Net minimisation posture:** *off by default.* Absent operator opt-in, sparq logs **no**
per-request content and **no** personal data; metrics are aggregate-only; errors are generic. See
`compliance/privacy/evidence.md` §P-2.

## 5 — Data egress (where data leaves the binary)

| Egress | When | Guard | Owner |
|---|---|---|---|
| **Query results** | every read | the operator's auth/solid authz decides who receives them | operator |
| **`SERVICE` outbound** | feature on + allowlisted | **deny-all default** allowlist (SSRF/exfil) | operator |
| **`LOAD <url>`** | UPDATE write | a remote fetch — operator gates writes | operator |
| **Telemetry / analytics** | **never** | the engine has **no** phone-home / analytics egress | — |

## 6 — ZK/MPC privacy story (explicitly NOT a guarantee)

<!-- [OPUS-4.8] reconciled with post-remediation re-audit (sq-gbp4); see ZK-verdict cross-ref sweep -->
If sound, `sparq-zk`/`sparq-zk-compose` (zero-knowledge query proofs) and `sparq-mpc`
(multi-party computation over data split across distrusting parties) would let an operator prove a
query result *without revealing the data* / compute a federated result *without pooling* the
inputs — a strong data-minimisation control. The v1 ZK verifier was originally found **unsound**
(`research/zk-soundness-audit.md`); `sq-1s2` landed the binding layer and the **internal,
single-model** re-audit (`research/zk-verifier-reaudit.md`, `sq-gbp4`) found all findings closed
("sound as landed for the assumed threat model"), but it is **NOT externally audited** (`SECURITY.md`;
`sq-qhy4`, P0, pending) and carries **no production guarantee**; `sparq-mpc` is semi-honest-only and
gives no confidentiality/correctness guarantee. **No data-flow protection in this
document relies on them**; they appear only to be explicitly excluded. Gated: bead **sq-toze.35**.

## 7 — Summary

The binary can touch personal data at exactly five points — **ingest, in-memory query, optional
persist WAL, optional request log, and result/`SERVICE` egress** — and at every point the
*engine's* job is a **conservative default** (in-memory only, no logging, deny-all egress, generic
errors, fail-closed authz when `sparq-solid` is used) while the *operator* owns the
deployment-level decisions (what to load, TLS, at-rest encryption, retention policy, lawful basis).
This split is the spine of the DPIA ([`dpia.md`](./dpia.md)) and the privacy control table
([`privacy/controls.md`](./privacy/controls.md)).
