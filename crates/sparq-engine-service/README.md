# sparq-engine-service

The SPARQL 1.1 **federated-query (`SERVICE`) client** for [`sparq-engine`] — the HTTP
transport seam, streaming SPARQL-Results JSON/XML parsers, bound-join `VALUES` batching,
and the SSRF egress-policy allowlist.

> **Internal, unstable crate.** This is seam A2 of the staged `sparq-engine` facade split
> (RFC `research/engine-split-rfc.md` §4 Option A / §7 Phase A2, bead `sq-6vshe.4`). It is
> published to crates.io only to satisfy `sparq-engine`'s dependency closure and has
> **no stability guarantee of its own**. Depend on
> **`sparq-engine`** — its `service` feature forwards here and its public
> `with_service_egress_allow` / `SERVICE_EGRESS_REFUSED_MARKER` / `allowlist_entry_permits`
> / … re-exports are unchanged by the split.

[`sparq-engine`]: ../sparq-engine

## 🚀 Quickstart

```rust
// Consume it through the facade — never depend on this crate directly.
// (sparq-engine, features = ["service"])
use sparq_engine::{query, with_service_egress_policy};
use sparq_core::Graph;

let local = Graph::default();
// Strict mode: only the listed host is dialled; every other host is refused.
// (`with_service_egress_allow` only blocks private addresses, so it is not enough
// for untrusted queries.)
let out = with_service_egress_policy(true, ["sparql.example.org".to_string()], || {
    query(&local, "SELECT * WHERE { SERVICE <https://sparql.example.org/> { ?s ?p ?o } }")
});
// (`out` is a transport error here — there is no live endpoint in the doctest.)
let _ = out;
```

## ✨ Features

- **`SERVICE` evaluation** — wraps the inner algebra as `SELECT * WHERE { <inner> }` (via
  spargebra `Display`), POSTs it to the remote endpoint, and streams the parsed rows back
  into the surrounding query. `SILENT` yields the join identity on any failure.
- **Streaming result parse** — SPARQL-Results-**JSON** (`serde` `DeserializeSeed`, one
  binding at a time — never a whole-document DOM) and SPARQL-Results-**XML** (`quick-xml`,
  event-driven), content-sniffed by the first byte. Bounded by a body-byte cap so an
  adversarial endpoint cannot exhaust memory. The `ReaderTransport` seam
  (`eval_remote_into_read`) feeds the parser directly from a `Read` stream so the HTTP
  response body is never materialised into a `String` — peak memory stays below the
  body size regardless of result cardinality (sq-my8wd.5).
- **Bound join (`VALUES` pushdown)** — when the SERVICE is the right side of a join whose
  join variables are already bound, a *block* of those bindings is pushed as a `VALUES`
  clause so the remote returns only rows that can survive the local join (brTPF/FedX bound
  join). Result-preserving; falls back to the bare pattern when it does not apply.
- **SSRF egress policy** — a deny-by-default allowlist enforced on the **resolved** IP
  (DNS-rebinding-safe, via a ureq `Resolver` wrapper) and mirrored as a pure
  `allowlist_entry_permits` predicate the server + federation client reuse.
- **`service`** *(feature, off by default)* — gates the whole client. When off, the crate
  compiles **empty** and pulls in **zero** dependencies, so the default and wasm builds of
  `sparq-engine` are byte-identical to before the split.

### Opt-in by construction

Nothing in sparq's default native build or the wasm artifact compiles this crate:
`sparq-engine` pulls it in only behind its off-by-default `service` feature, as an optional
dependency, and the ureq HTTP client is additionally `cfg(not(wasm32))`-gated so no HTTP/TLS
stack ever enters the browser bundle. The crate is `#![forbid(unsafe_code)]`.

## 📚 Learn more

- Facade + public surface: [`sparq-engine`] (`with_service_egress_allow`, `…`).
- Split design + seam map: `research/engine-split-rfc.md` (bead `sq-6vshe.4`).
- Federated-query surface: `skills/federated-planning/SKILL.md`.
- SPARQL 1.1 Federated Query: <https://www.w3.org/TR/sparql11-federated-query/>

## License

MIT © the sparq authors.
