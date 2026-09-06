# sparq-acbench

Parameterised deterministic generator and by-construction oracle for the WAC/ACP/ODRL
access-controlled-query benchmark (epic `sq-i6du2`, issue
[#1613](https://github.com/sparq-org/sparq/issues/1613)).

**Internal tooling — not published** (`publish = false`): nothing in the shipping
graph depends on it. Design authority:
[`research/ac-query-benchmark.md`](../../research/ac-query-benchmark.md).

## 🚀 Quickstart

```rust
use sparq_acbench::{GenParams, oracle_wac, AccessMode, Request, Decision};

// Same seed → byte-identical corpus, forever.
let params = GenParams::smoke();
params.validate().unwrap();

// Fail-closed: empty intents → Deny.
let request = Request {
    agent: "https://alice.example/".to_string(),
    client: None,
    resource: "https://alice.example/docs/notes.ttl".to_string(),
    mode: AccessMode::read_only(),
};
assert_eq!(oracle_wac(&request, &[]), Decision::Deny);
```

## ✨ Features

- **Seeded determinism**: `SplitMix64` throughout — same `GenParams` → byte-identical
  N-Quads, intent tables, and expected decisions on every run and platform.
- **Intent-table IR**: model-agnostic `(audience, scope, mode, condition, effect)` rows
  with three per-model compilers (WAC / ACP / ODRL) and an expressibility matrix.
- **By-construction oracle**: WAC, ACP, and ODRL evaluators structurally independent of
  sparq's N3 rule engine and `AclIndex` (cannot launder sparq bugs).
- **Fail-closed harness**: `Decision::Deny` is the default; any mismatch → nonzero exit.
- **Four use-case generators** (beads `sq-i6du2.2`–`.5`): personal data storage (U1),
  commercial project management (U2), financial services (U3), research consortium (U4).
- **Independent many-Pod generator** (`deployment`): varies Pod and document counts,
  triples per document, hierarchy depth, own-ACL placement, audience mix, URI topology,
  and social/health graph shape without coupling them through one scale factor. It also
  supplies an independent physical-readability oracle and eight SPARQL query families.
- **Zero dependency on `sparq-core` / `sparq-engine`**: opt-in crate architecture.

## Streaming personal-service populations

<!-- [GPT-6] -->

`population::write_pod(&config, pod_id, model, writer)` writes one populated Pod to
any `std::io::Write` without building or scanning a server-wide deployment. Its
`PodSummary` counts actual serialized records, quads, graphs, and bytes. The config
and summaries support `serde_json`; malformed or unknown config fields fail closed.
`PopulationConfig::smoke()` is a controlled fixture. `service_history()` is the
central scenario with a partially calibrated service history. Neither constructor
implies an empirically representative population of future Solid users.

The services cover communication, contacts, calendar, transactions, activity
summaries, episodic location, media metadata and retained media ratings. Binary
media payloads are excluded. Message excerpts, not full retained-message body-size
distributions, are modeled. Record counts share an explicitly assumed activity
factor, except the empirical ratings marginal. Population generation is independent
of Pod order and Pod count, making prefixes and random-access regeneration identical.

`PolicyModel::{Wac,Acp}` produces equivalent intended content rights.
`write_readable_content` emits physically filtered record graphs for query-result
comparison. Query templates use explicit `GRAPH` clauses and deterministic ordering
for bounded results. `expected_record_count` and `can_read` provide a by-construction oracle that never
invokes the system under test. `benchmark_queries` supplies eleven query families.
The generated WAC group expands to named ACP recipients; direct private exceptions
use WAC inheritance shadowing or ACP deny overrides. Group expansion, policy size,
and policy setup work must remain visible in benchmark comparisons.

See the [usage skill](../../skills/ac-benchmark/SKILL.md) and
[calibration manifest](../../bench/ac/million/corpus-calibration.json). The earlier
`deployment` generator and its frozen study remain separate.

## 📚 Learn more

- Design record: [`research/ac-query-benchmark.md`](../../research/ac-query-benchmark.md)
- Epic: `sq-i6du2` — issue [#1613](https://github.com/sparq-org/sparq/issues/1613)
- Generator beads: `sq-i6du2.2` (U1), `.3` (U2), `.4` (U3), `.5` (U4)
- Workload engine (W1–W4): `sq-i6du2.6` (`src/workload.rs` / `src/oracle.rs`)
- Benchmark registration: `sq-i6du2.7` (`bench/ac/`)
- Many-Pod scaling artefact:
  [`bench/ac/scaling/README.md`](../../bench/ac/scaling/README.md)

## License

[MIT](../../LICENSE).
