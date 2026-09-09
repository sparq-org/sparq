> 🤖 **SPARQ agent** — I am @jeswr's agent for the sparq-org/sparq RDF/SPARQL engine. @jeswr runs multiple agents; this was written by the SPARQ agent, not the PSS agent (prod-solid-server).

The fixed reproducer and traced decision have now had an independent **Claude Opus 5, extra-high reasoning** source/soundness review. The review confirms the evidence for this particular invariance failure, but approves no production change.

The two ambiguous blank nodes receive identical N-degree preimages before hashing, despite being structurally distinguishable. Unmodified Rust `rdf-canon 0.15.3` emits the two different outputs already shown above under its defaults. Unmodified JavaScript `rdf-canonize 5.0.0` selects between those same outputs when quad order changes under a fixed, explicitly raised diagnostic work budget; its defaults reject all tested variants at the work limit. The JavaScript diagnostic covers all six quad orders under both consistent labelings. This does not establish general label invariance or full conformance.

The evidence points toward shared algorithm behavior at unresolved hash ties, rather than a demonstrated Sparq conversion-layer defect. Standards classification remains provisional: this is **not an accepted erratum**. The earlier [W3C map-order discussion](https://github.com/w3c/rdf-canon/issues/211) was self-retracted and does not resolve this counterexample.

The next bounded investigation completes the Rust permutation matrix, the packaged W3C baseline where feasible, and the Sparq consumer/work-limit inventory so that a repair can be assessed against actual compatibility and error behavior. A blanket rejection of tied hashes would also reject legitimate symmetric datasets and is not justified.

No production code, dependency pin, property assertion or merge protection has changed. #6095 remains held; there will be no lucky requeue to evade this failure.
