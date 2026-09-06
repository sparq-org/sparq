# JSON proposal

Implementation: `src/proposed/json.rs` (gated behind the default-off
`proposed-json` feature).
Feature: `proposed-json` (default off).

Cycle-safe JSON projection of a wrapped focus node: `JsonProjection::project`
renders the focus and its outgoing reachable subgraph as one compact JSON
document. It is total on cyclic graphs — a repeated node is emitted as the
reference `{"@ref": "<term>"}` under an explicit `RepeatedFocus` policy
(`OnCycle` breaks cycles only, `OnRepeat` expands each node at most once), with
`with_max_depth` bounding recursion — and deterministic, because predicates are
ordered by IRI and objects by N-Triples form. Literals keep their datatype IRI,
language tag, and RDF 1.2 base direction rather than collapsing to bare JSON
scalars. Source: rdfjs/wrapper open PR #23. <!-- [SONNET-4.6] sq-1rg2q.11 -->
