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
- **Exported group closure**: U1, U2, and U4 expose `group_memberships`, a sorted map
  from group IRIs to resolved agent sets (including nested membership). ODRL consumers
  map these to compiled collection IRIs and supply explicit request evidence. U3 only
  names advisory groups; it does not generate membership rolls.
- **Zero dependency on `sparq-core` / `sparq-engine`**: opt-in crate architecture.

## 📚 Learn more

- Design record: [`research/ac-query-benchmark.md`](../../research/ac-query-benchmark.md)
- Epic: `sq-i6du2` — issue [#1613](https://github.com/sparq-org/sparq/issues/1613)
- Generator beads: `sq-i6du2.2` (U1), `.3` (U2), `.4` (U3), `.5` (U4)
- Workload engine (W1–W4): `sq-i6du2.6` (`src/workload.rs` / `src/oracle.rs`)
- Benchmark registration: `sq-i6du2.7` (`bench/ac/`)

## License

[MIT](../../LICENSE).
