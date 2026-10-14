<!-- sq-inzv: internal-stub README for a publish=false crate; full posture lives in skills/mpc/SKILL.md + PLAN.md. -->
# sparq-mpc

Honest-majority Shamir MPC over (federated) SPARQL (research question RQ2): the
secret-sharing + secure-computation substrate (prime field `F_p`, Shamir sharing,
authenticated MAC shares, the hidden-value join, robust reconstruction, transports)
that lets mutually-distrusting holders jointly answer one query while minimising
disclosure. The detailed per-operator security posture and the public-API surface
live in [`skills/mpc/SKILL.md`](../../skills/mpc/SKILL.md); the build plan and the
deferred malicious-security seams live in [`PLAN.md`](./PLAN.md).

> **Internal crate — not published** to crates.io (`publish = false`).
> **No security guarantee — research-grade and externally unaudited (`sq-qhy4`).**
> Honest-majority, **semi-honest only** — NOT malicious security, NOT
> dishonest-majority. <!-- privacy-claims-allow: NEGATIVE/scoped — denies malicious + dishonest-majority security; sq-qhy4 pending -->
> The ZK verifier it composes with is not externally signed off (`sq-qhy4`
> pending), and the collaborative ZK proof is a stub (`NotYetImplemented`). Nothing
> here is a production security claim.

Per-protocol trust-model annotations (opt-in `secprop-annotations`):
[`ontologies/secprop-mpc-methods.ttl`](./ontologies/secprop-mpc-methods.ttl).

Backend selection: `SecurityRequirement::max_preprocessing` defaults to `None`,
preserving security-only ranking. Set a `PreprocessingBudget` with multiplication
and ordering-comparison counts plus optional inclusive `max_items` and
`max_bytes_per_party` ceilings. Both `BackendRegistry::select` and
`select_for_operator` use the existing `PreprocessingCost` estimates: excessive,
unknown or overflowing estimates fail a set ceiling. `PreprocessingBudgetRefused`
carries the workload, ceilings and rejected candidate estimates; `NoBackendSatisfies`
remains the error when no backend meets the security floor. Costs are declared,
not measured runtime spending limits; update workload counts when the plan changes.

Plan: [`PLAN.md`](./PLAN.md).
How-to: [`skills/mpc/SKILL.md`](../../skills/mpc/SKILL.md).
Design: [`research/mpc-zkp-research-and-architecture.md`](../../research/mpc-zkp-research-and-architecture.md).
Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
