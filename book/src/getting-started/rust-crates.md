<!-- Ported from #6148 (issue #6144) and cut to the crates.io publish set. This manifest-derived
table is a narrow exception to the book's include-only prose rule:
scripts/tests/test_publish_strip.py checks its rows against every workspace crate without
`publish = false`. -->

# Rust crates

These crates are published to crates.io, in lockstep at the workspace version. The first
crates.io publication is still pending; until then, build from source.

| Crate | What it is | Usage guide |
|---|---|---|
| [`sparq-core`](https://crates.io/crates/sparq-core) | Store, dictionary, indexes and RDF parsers | [SPARQL query](https://github.com/sparq-org/sparq/blob/main/skills/sparql-query/SKILL.md) |
| [`sparq-engine`](https://crates.io/crates/sparq-engine) | SPARQL 1.1/1.2 planner and execution | [SPARQL query](https://github.com/sparq-org/sparq/blob/main/skills/sparql-query/SKILL.md) |
| [`sparq-engine-serialize`](https://crates.io/crates/sparq-engine-serialize) | Result and RDF serializers used by the engine | [SPARQL query](https://github.com/sparq-org/sparq/blob/main/skills/sparql-query/SKILL.md) |
| [`sparq-engine-service`](https://crates.io/crates/sparq-engine-service) | `SERVICE` federation client used by the engine | [Federated planning](https://github.com/sparq-org/sparq/blob/main/skills/federated-planning/SKILL.md) |
| [`sparq-substrate`](https://crates.io/crates/sparq-substrate) | Shared evaluation substrate for engine and reasoner | [Substrate](https://github.com/sparq-org/sparq/blob/main/skills/substrate/SKILL.md) |
| [`sparq-jsonld`](https://crates.io/crates/sparq-jsonld) | JSON-LD processing | [JSON-LD](https://github.com/sparq-org/sparq/blob/main/skills/jsonld/SKILL.md) |
| [`sparq-reason`](https://crates.io/crates/sparq-reason) | RDFS / OWL RL / N3 inference | [Inference](https://github.com/sparq-org/sparq/blob/main/skills/inference/SKILL.md) |
| [`sparq-shacl`](https://crates.io/crates/sparq-shacl) | SHACL Core + SPARQL validation | [SHACL validation](https://github.com/sparq-org/sparq/blob/main/skills/shacl-validation/SKILL.md) |
| [`sparq-hdt`](https://crates.io/crates/sparq-hdt) | HDT archive loading | [Data formats](https://github.com/sparq-org/sparq/blob/main/skills/data-formats/SKILL.md) |
| [`sparq-serve`](https://crates.io/crates/sparq-serve) | Serving layer under the HTTP server | [HTTP server](https://github.com/sparq-org/sparq/blob/main/skills/http-server/SKILL.md) |
| [`sparq-server`](https://crates.io/crates/sparq-server) | SPARQL 1.1 Protocol / Graph Store HTTP server | [HTTP server](https://github.com/sparq-org/sparq/blob/main/skills/http-server/SKILL.md) |
| [`sparq-cli`](https://crates.io/crates/sparq-cli) | The `sparq` command-line tool | [CLI](https://github.com/sparq-org/sparq/blob/main/skills/cli/SKILL.md) |

The SPARQL parser ships as [`sparq-spargebra`](https://crates.io/crates/sparq-spargebra),
oxigraph's `spargebra` with sparq's conformance and hardening patches.

Every other capability (full-text, GeoSPARQL, vectors, Solid access control, ZK proofs,
policy, HTTP/3, ...) lives in crates that are built from the repository rather than from
crates.io, and the published `sparq-server` and `sparq-cli` omit the features that need them.
Build from source with the feature enabled to use one; see the
[release runbook](https://github.com/sparq-org/sparq/blob/main/docs/release.md#cratesio-publish-set).
