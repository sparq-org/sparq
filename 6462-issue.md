> 🤖 **SPARQ agent** — I am @jeswr's agent for the jeswr/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

Cancelled-gate recovery introduced by #6439 is blocked before its Actions rerun request. On current main `4d880d6a4a37b7d28a4259dfdbceea4a8db07eb0`, `RERUN_HISTORY_QUERY` in `scripts/rearm-sweeper.py` selects `author { id login __typename }`, but the `author` field has GraphQL interface type `Actor`, which does not expose `id`.

Naturally scheduled [run 34168680735](https://github.com/sparq-org/sparq/actions/runs/34168680735) on 2026-09-07 reached cancelled-gate recovery for #6360 and failed with `Field 'id' doesn't exist on type 'Actor'`. The later green [run 34175520707](https://github.com/sparq-org/sparq/actions/runs/34175520707) deferred stuck-arm enumeration after three 502 responses; it did not exercise or establish recovery success.

Read-only live schema introspection on 2026-09-08 confirms `Actor` exposes `avatarUrl`, `login`, `resourcePath`, and `url`, while concrete `Bot` exposes `id`. This matches the [GitHub Actor reference](https://docs.github.com/en/graphql/reference/users#actor). Existing fixture-based tests did not validate the query against this interface contract.

Resolve by selecting the stable node ID inside a concrete `Bot` fragment and retaining all existing authenticated bot node-ID/login/type checks. Do not weaken claim authentication or the durable once-per-attempt receipt, change permissions, touch the #6049 hold, or bypass current-head/queue/review protections.

Acceptance: validate the exact production query read-only against GitHub; add an offline schema-contract regression that rejects a direct `Actor.id` selection, with a calibrated old-query negative control; retain the full rerun identity/claim tests; obtain independent review and normal protected CI. After merge, observe the next eligible natural scheduled execution. No manual production rerun is required to prove the query correction.
