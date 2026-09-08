> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

Resolved by [PR #6464](https://github.com/sparq-org/sparq/pull/6464), merged as `cf19a52c6880c496cefaca24046174902bc30958`. The six implementation/test/fixture files are byte-identical to the independently reviewed head; docs-quality matches the separately verified composition with preceding main changes. All 47 nightly-freshness tests actually executed and passed in merge-group CI. Both the protected merge-group gate and main's aggregate gate succeeded.

The next natural heavy nightly execution will validate operational behavior. No new measurement, manual dispatch or rerun was used as a substitute for the existing evidence. Registry dispatch remains paused, API-budget failures remain fail-closed, and releases remain unapproved.
