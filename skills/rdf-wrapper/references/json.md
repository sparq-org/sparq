# JSON extension

Implemented behind the default-off `proposed-json` feature in
`proposed::json`.

`JsonProjection::project(&node)` returns the focus node and its outgoing
reachable subgraph as one compact JSON string. Cycles terminate: a repeated
node becomes `{"@ref": "<term>"}` instead of recursing, chosen by the explicit
`RepeatedFocus` policy (`OnCycle`, the default, references only ancestors of
the node being written; `OnRepeat` references every node already expanded).
`with_max_depth` bounds recursion depth. Output is deterministic — predicates
sorted by IRI, objects by N-Triples form — so the same store projects
byte-identically twice. Literals are value objects carrying `@value` plus
`@type`, or `@language` (and `@direction` for an RDF 1.2 directional literal),
so no datatype or language metadata is discarded. Source: rdfjs/wrapper open
PR #23. <!-- [SONNET-4.6] sq-1rg2q.11 -->
