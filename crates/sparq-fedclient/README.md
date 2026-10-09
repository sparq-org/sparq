<!-- sq-inzv: internal-stub README for a publish=false crate; full design lives in research/federation-client-design.md. -->
# sparq-fedclient

A streaming federation **client** over heterogeneous remote RDF sources (full
SPARQL endpoints, bindings-restricted brTPF servers, plain TPF servers, and the
local sparq engine) — the query *consumer* half of federation. It discovers each
source's capability, reuses the `sparq-fedplan` cost-based planner, pushes the most
precise sub-query each source can answer (FedX exclusive groups; an exact
common-variable FILTER-pushdown safety check), and streams results through
non-blocking operators behind a default-deny SSRF egress guard. The architecture
(layered model + per-module map, the reuse seams, honest risks, and the load-bearing
dependency boundary that keeps it out of `sparq-core`/`sparq-engine` and the wasm
artifact) lives in
[`research/federation-client-design.md`](../../research/federation-client-design.md).

> **Internal crate — not published** to crates.io (`publish = false`). Opt-in and
> OFF by default (`fedclient` / `fedclient-adaptive` / `pattern_probe` cargo features), native-only
> HTTP transport (never enters the wasm bundle). Early/research: no performance
> claim is asserted here — any "better than Comunica" prediction in the design
> record must be validated head-to-head before being stated as fact.

Architecture: [`research/federation-client-design.md`](../../research/federation-client-design.md).
Planner it reuses: [`skills/federated-planning/SKILL.md`](../../skills/federated-planning/SKILL.md).
Contributing: [`AGENTS.md`](../../AGENTS.md).

Correctness suite under `tests/` (gated on the `fedclient` feature; the default build compiles it to nothing) — run on the REAL path, with local `sparq-engine` evaluation as the canonical answer: result-equivalence (planner / streaming / multi-source UNION / adaptive vs. static — incl. the union-arm ADAPTIVE loop with live per-arm latency observations vs. the merged-graph oracle, sq-xw8zz — and end-to-end over a REAL in-process `sparq-server` loopback on `127.0.0.1:0`, not just the in-memory `Transport` seam), fail-closed wire & error paths (SRJ decode, brTPF binary codec, interpreter), discovery + source-adapter error paths (incl. the load-bearing SSRF egress guard: an egress attempt outside the per-endpoint allowlist scope is refused), and the one-way `sparq-core`/`sparq-engine` dependency boundary. Test QUALITY is ratcheted by `cargo-mutants` (nightly, features-on): the suite pins exact rendered sub-queries, error-variant strings, SRJ/SD parse outputs, SSRF boundary tables and native-transport observables so a mutated return value is caught, not just executed. With `pattern_probe`, `PatternProbeSession` caches bounded ASK/capped-SELECT observations per query; only exact ASK `false` prunes, while any timeout/error/budget exhaustion keeps the source and uniform fallback. Full usage is in the federated-planning skill linked above. sq-fx5id.

## License

[MIT](../../LICENSE).
