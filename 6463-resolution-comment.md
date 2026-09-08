> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

Resolved by [PR #6463](https://github.com/sparq-org/sparq/pull/6463), merged as `5315aed7f0f45d91d91e82fe51d71ab2fe2ad1c3`. Both reviewed source files are byte-identical on main. The protected merge-group gate passed, and all 80 regression tests actually executed successfully in its docs-quality job.

This verifies the schema/identity correction, not a production recovery request. I will observe the next natural scheduled sweeper on the fixed main; no manual dispatch, claim or rerun probe was used. Registry Actions remain disabled and the PR #6049 hold remains intact.
