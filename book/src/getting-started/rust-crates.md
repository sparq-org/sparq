<!-- [GPT-5] issue #6144 — one-off published-crate reconciliation. The package list is
derived from crates/*/Cargo.toml: a package is publishable unless `publish = false`.
The website destination agrees with PUBLISHED_CRATE_SURFACE in site/src/data/surfaces.ts.
Supporting packages deliberately share their parent capability page. -->
<!-- This manifest-derived catalogue is the narrow exception to the book's include-only prose
rule. scripts/tests/test_published_crate_catalogue.py gates every row against its source data. -->

# Rust crate catalogue

Every workspace package eligible for crates.io publication is represented below. A capability
may use several packages: implementation and support crates deliberately share one website page
and usage guide rather than presenting the same feature as separate products.

Publication is currently manual and the initial crates.io bootstrap is still pending; these
names become installable registry links only after the leaf-first procedure in
[the release runbook](https://github.com/jeswr/sparq/blob/main/docs/release.md#4-cratesio-publication).

| Publishable crate | Website capability | Usage guide |
|---|---|---|
| [`sparq-algos`](https://crates.io/crates/sparq-algos) | [Graph analytics](https://sparq.jeswr.org/capabilities/#query-data) | [Graph analytics](https://github.com/jeswr/sparq/blob/main/skills/graph-analytics/SKILL.md) |
| [`sparq-arrow`](https://crates.io/crates/sparq-arrow) | [Arrow export](https://sparq.jeswr.org/capabilities/#query-data) | [Arrow columnar](https://github.com/jeswr/sparq/blob/main/skills/arrow-columnar/SKILL.md) |
| [`sparq-canon`](https://crates.io/crates/sparq-canon) | [RDF canonicalization](https://sparq.jeswr.org/capabilities/#query-data) | [RDF canonicalization](https://github.com/jeswr/sparq/blob/main/skills/rdf-canon/SKILL.md) |
| [`sparq-cli`](https://crates.io/crates/sparq-cli) | [CLI](https://sparq.jeswr.org/capabilities/#serve-embed) | [CLI](https://github.com/jeswr/sparq/blob/main/skills/cli/SKILL.md) |
| [`sparq-core`](https://crates.io/crates/sparq-core) | [SPARQL](https://sparq.jeswr.org/surface/sparql/) | [SPARQL query](https://github.com/jeswr/sparq/blob/main/skills/sparql-query/SKILL.md) |
| [`sparq-engine`](https://crates.io/crates/sparq-engine) | [SPARQL](https://sparq.jeswr.org/surface/sparql/) | [SPARQL query](https://github.com/jeswr/sparq/blob/main/skills/sparql-query/SKILL.md) |
| [`sparq-engine-serialize`](https://crates.io/crates/sparq-engine-serialize) | [SPARQL](https://sparq.jeswr.org/surface/sparql/) | [SPARQL query](https://github.com/jeswr/sparq/blob/main/skills/sparql-query/SKILL.md) |
| [`sparq-engine-service`](https://crates.io/crates/sparq-engine-service) | [Federation](https://sparq.jeswr.org/capabilities/#serve-embed) | [Federated planning](https://github.com/jeswr/sparq/blob/main/skills/federated-planning/SKILL.md) |
| [`sparq-fedplan`](https://crates.io/crates/sparq-fedplan) | [Federation](https://sparq.jeswr.org/capabilities/#serve-embed) | [Federated planning](https://github.com/jeswr/sparq/blob/main/skills/federated-planning/SKILL.md) |
| [`sparq-forms`](https://crates.io/crates/sparq-forms) | [SHACL](https://sparq.jeswr.org/surface/shacl/) | [SHACL forms](https://github.com/jeswr/sparq/blob/main/skills/shacl-forms/SKILL.md) |
| [`sparq-geo`](https://crates.io/crates/sparq-geo) | [GeoSPARQL](https://sparq.jeswr.org/capabilities/#query-data) | [GeoSPARQL](https://github.com/jeswr/sparq/blob/main/skills/geosparql/SKILL.md) |
| [`sparq-hdt`](https://crates.io/crates/sparq-hdt) | [Data formats](https://sparq.jeswr.org/surface/data-formats/) | [Data formats](https://github.com/jeswr/sparq/blob/main/skills/data-formats/SKILL.md) |
| [`sparq-http3`](https://crates.io/crates/sparq-http3) | [HTTP server](https://sparq.jeswr.org/capabilities/#serve-embed) | [HTTP/3 server](https://github.com/jeswr/sparq/blob/main/skills/http3-server/SKILL.md) |
| [`sparq-introspect`](https://crates.io/crates/sparq-introspect) | [GenAI / NLQ](https://sparq.jeswr.org/capabilities/#search-genai) | [GenAI retrieval](https://github.com/jeswr/sparq/blob/main/skills/genai-retrieval/SKILL.md) |
| [`sparq-jsonld`](https://crates.io/crates/sparq-jsonld) | [Data formats](https://sparq.jeswr.org/surface/data-formats/) | [JSON-LD](https://github.com/jeswr/sparq/blob/main/skills/jsonld/SKILL.md) |
| [`sparq-mcp`](https://crates.io/crates/sparq-mcp) | [MCP server](https://sparq.jeswr.org/capabilities/#serve-embed) | [Agent tools](https://github.com/jeswr/sparq/blob/main/skills/agent-tools/SKILL.md) |
| [`sparq-nlq`](https://crates.io/crates/sparq-nlq) | [GenAI / NLQ](https://sparq.jeswr.org/capabilities/#search-genai) | [GenAI retrieval](https://github.com/jeswr/sparq/blob/main/skills/genai-retrieval/SKILL.md) |
| [`sparq-policy`](https://crates.io/crates/sparq-policy) | [Policy](https://sparq.jeswr.org/capabilities/#trust-governance) | [Usage-control policy](https://github.com/jeswr/sparq/blob/main/skills/usage-control-policy/SKILL.md) |
| [`sparq-reason`](https://crates.io/crates/sparq-reason) | [Inference](https://sparq.jeswr.org/surface/inference/) | [Inference](https://github.com/jeswr/sparq/blob/main/skills/inference/SKILL.md) |
| [`sparq-reason-el`](https://crates.io/crates/sparq-reason-el) | [Inference](https://sparq.jeswr.org/surface/inference/) | [Inference](https://github.com/jeswr/sparq/blob/main/skills/inference/SKILL.md) |
| [`sparq-reason-ql`](https://crates.io/crates/sparq-reason-ql) | [Inference](https://sparq.jeswr.org/surface/inference/) | [Inference](https://github.com/jeswr/sparq/blob/main/skills/inference/SKILL.md) |
| [`sparq-rsp`](https://crates.io/crates/sparq-rsp) | [Streaming RSP](https://sparq.jeswr.org/capabilities/#serve-embed) | [Streaming RSP](https://github.com/jeswr/sparq/blob/main/skills/streaming-rsp/SKILL.md) |
| [`sparq-secprop-vocab`](https://crates.io/crates/sparq-secprop-vocab) | [Zero-knowledge](https://sparq.jeswr.org/capabilities/#trust-governance) | [ZK query proofs](https://github.com/jeswr/sparq/blob/main/skills/zk-query-proofs/SKILL.md) |
| [`sparq-serve`](https://crates.io/crates/sparq-serve) | [HTTP server](https://sparq.jeswr.org/capabilities/#serve-embed) | [HTTP server](https://github.com/jeswr/sparq/blob/main/skills/http-server/SKILL.md) |
| [`sparq-server`](https://crates.io/crates/sparq-server) | [HTTP server](https://sparq.jeswr.org/capabilities/#serve-embed) | [HTTP server](https://github.com/jeswr/sparq/blob/main/skills/http-server/SKILL.md) |
| [`sparq-shacl`](https://crates.io/crates/sparq-shacl) | [SHACL](https://sparq.jeswr.org/surface/shacl/) | [SHACL validation](https://github.com/jeswr/sparq/blob/main/skills/shacl-validation/SKILL.md) |
| [`sparq-shaclc`](https://crates.io/crates/sparq-shaclc) | [SHACL](https://sparq.jeswr.org/surface/shacl/) | [SHACL Compact Syntax](https://github.com/jeswr/sparq/blob/main/skills/shacl-compact-syntax/SKILL.md) |
| [`sparq-sim`](https://crates.io/crates/sparq-sim) | [GenAI / NLQ](https://sparq.jeswr.org/capabilities/#search-genai) | [Structural similarity](https://github.com/jeswr/sparq/blob/main/skills/structural-similarity/SKILL.md) |
| [`sparq-solid`](https://crates.io/crates/sparq-solid) | [Solid access control](https://sparq.jeswr.org/showcase/solid-pairs/) | [Access control](https://github.com/jeswr/sparq/blob/main/skills/access-control/SKILL.md) |
| [`sparq-substrate`](https://crates.io/crates/sparq-substrate) | [SPARQL](https://sparq.jeswr.org/surface/sparql/) | [Substrate](https://github.com/jeswr/sparq/blob/main/skills/substrate/SKILL.md) |
| [`sparq-terse`](https://crates.io/crates/sparq-terse) | [GenAI / NLQ](https://sparq.jeswr.org/capabilities/#search-genai) | [GenAI retrieval](https://github.com/jeswr/sparq/blob/main/skills/genai-retrieval/SKILL.md) |
| [`sparq-text`](https://crates.io/crates/sparq-text) | [Full-text](https://sparq.jeswr.org/capabilities/#search-genai) | [Full-text](https://github.com/jeswr/sparq/blob/main/skills/full-text-search/SKILL.md) |
| [`sparq-trust`](https://crates.io/crates/sparq-trust) | [Solid access control](https://sparq.jeswr.org/showcase/solid-pairs/) | [Trust graph](https://github.com/jeswr/sparq/blob/main/skills/trust-graph/SKILL.md) |
| [`sparq-vc`](https://crates.io/crates/sparq-vc) | [Zero-knowledge](https://sparq.jeswr.org/capabilities/#trust-governance) | [Verifiable credentials](https://github.com/jeswr/sparq/blob/main/skills/verifiable-credentials/SKILL.md) |
| [`sparq-vectors`](https://crates.io/crates/sparq-vectors) | [Vector](https://sparq.jeswr.org/capabilities/#search-genai) | [Vector search](https://github.com/jeswr/sparq/blob/main/skills/vector-search/SKILL.md) |
| [`sparq-wrapper`](https://crates.io/crates/sparq-wrapper) | [SPARQL](https://sparq.jeswr.org/surface/sparql/) | [RDF wrapper](https://github.com/jeswr/sparq/blob/main/skills/rdf-wrapper/SKILL.md) |
| [`sparq-zk`](https://crates.io/crates/sparq-zk) | [Zero-knowledge](https://sparq.jeswr.org/capabilities/#trust-governance) | [ZK query proofs](https://github.com/jeswr/sparq/blob/main/skills/zk-query-proofs/SKILL.md) |

The inventory is reproducible from the workspace manifests: inspect each `crates/*/Cargo.toml`
package and exclude only packages whose `publish` field is `false`. The site catalogue records
the same association in `PUBLISHED_CRATE_SURFACE`.
