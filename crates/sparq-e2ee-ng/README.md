<!-- [OPUS-4.8] sq-tag1q.9: internal-stub README for a publish=false crate; full posture + public-API surface live in skills/e2ee-ng/SKILL.md and the design record. -->
# sparq-e2ee-ng

Opt-in **E2EE-NG profile primitives** for sparq: the *capability / envelope /
epoch* layer of the NextGraph-style E2EE-queryable design — deterministic
capability encoding with strict read/publish/admin separation, recipient-wrapped
secrets, randomized padded block/commit envelopes, a Merkle-linked object
chunker, a domain-separated key schedule, Ed25519 signatures, epoch transitions,
a fail-closed deterministic-CBOR codec with explicit parser limits, and golden
test vectors. The public-API surface and honest disclosure ledger live in
[`skills/e2ee-ng/SKILL.md`](../../skills/e2ee-ng/SKILL.md); the design is
[`research/e2ee-nextgraph-variant-gpt56-2026-07.md`](../../research/e2ee-nextgraph-variant-gpt56-2026-07.md).

> **Internal crate — not published** to crates.io (`publish = false`).
> **No security guarantee — research-grade and externally unaudited (`sq-qhy4`).**
> Every confidentiality / integrity / authorization / revocation property is
> **designed/intended, not proven**; the v0 suite name is a placeholder pending
> external review; sync / broker / CRDT / materialize are NOT in this crate.
> <!-- privacy-claims-allow: NEGATIVE/scoped — explicitly denies any proven soundness/privacy claim; sq-qhy4 pending -->
> Encryption + key material live ONLY behind this crate: `sparq-core` /
> `sparq-engine` / `sparq-substrate` do not depend on it, and the default + wasm
> builds are byte-identical with or without it.

Design: [`research/e2ee-nextgraph-variant-gpt56-2026-07.md`](../../research/e2ee-nextgraph-variant-gpt56-2026-07.md).
How-to: [`skills/e2ee-ng/SKILL.md`](../../skills/e2ee-ng/SKILL.md).
Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
