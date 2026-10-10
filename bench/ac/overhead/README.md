<!-- internal-stub -->
# ac-bench-overhead

> `publish = false` — dev-only overhead driver for the AC-query benchmark (bead sq-hmd7l.44).

Measures what ODRL enforcement **costs**: the sibling `bench/ac/` (oracle) + `bench/ac/live/`
(live WAC/ACP) drivers are correctness/decision-agreement; this driver compares **gated vs
unguarded latency**, fail-closed on any correctness disagreement while timing.

- **Lane A** — policy-materialization cost sweep (kind × count: bare permission,
  permission+prohibition, conditional recipient, counted `odrl:count`). Only the bare
  permission is stored; the other kinds are refused (a constraint or prohibition can change
  while a stored grant stands), so their rows time the refusal and assert no access.
- **Lane B** — steady-state per-query overhead: `PodStore::query_as` over an ODRL-materialized
  `<urn:sparq:auth>` vs the SAME query unguarded (`sparq_engine::query`) on a data-only twin
  where the permitted subset is physically the whole store — identical result sets asserted
  per query (honest apples-to-apples), stranger anti-vacuity probe before timing. Resource
  universes: the `sparq-acbench` U1–U4 intent tables at ≥2 scale factors.
- **Lane C** — churn: `refresh_odrl_grants` (no-op) + `refresh_odrl_grant` (revocation) cost vs
  ledger size, asserting the retraction removes access and preserves survivors.

**Competitor**: an explicit honest NOT-COMPARABLE verdict is recorded in the JSON envelope
(CSS enforces WAC/ACP at HTTP level; ODRE is PDP decision throughput, not result-set
filtering); the HTTP-level same-box lane composes with sq-lrtc3.1.

Run: `bench/ac/run.sh --overhead --smoke`, or `cargo run --release -- --smoke [--out FILE]`
from this directory. Work-box numbers are advisory + NON-CANONICAL (`bench/CATALOG.md`
QUIET-BOX); canonical envelopes are EC2-gated. No number is committed to markdown.

## License

MIT — see the workspace root `LICENSE` file.
