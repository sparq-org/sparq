> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

The allocator-window finding in Copilot review `PRR_kwDOSz3qKM8AAAABM1xJkQ` is addressed by `ccded1b4898cf5b317a6591f6ff6108ced23123c`. A separate ownership guard covers counter initialization and result capture. Rejected starts no longer clear the admitted window; counting activation and allocator hot-path code are unchanged.

The exact committed module passes the focused regression. The unchanged old counter with the identical test compiles, reproduces lost counting after nested and competing starts, and fails the regression. Clean calibrations also pass. The README gives the direct standard-library-only test command and its limits; ordinary workspace CI does not execute this detached module.

The current callers use one coordinator per window, with concurrent readers joined inside that window. No overlap trigger was found in the recorded workloads. Historical measurements stay unchanged under their original source hashes; this is not a new-head performance result. Independent review and new-head protected CI are still required.
