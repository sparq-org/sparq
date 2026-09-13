# Blank nodes and graph results: V3 implementation design

[GPT-6] This is work in progress after the separate V2 named-dataset relation.
Execution status is recorded in the linked structured checks; full SPARQL coverage is not claimed.

The V3 model schema preserves V1/V2 request and commitment meanings.
It binds source blank-node identity within the complete N-Quads document,
supports query blank nodes as existential variables, and publishes result-local
canonical labels instead of copying source labels as identifiers. The complete
IRI-named graph catalog remains explicit; blank-node graph names remain excluded
from this pinned SPARQL 1.1 profile.

CONSTRUCT instantiates a template once per solution occurrence, allocates
fresh blank nodes per occurrence, preserves repeated labels within that occurrence,
avoids collisions with bound source nodes, omits unbound/illegal triples and retains
graph set semantics. DESCRIBE explicitly binds the implementation's outgoing
blank-node closure over the active default graph, with cycle detection; it will
not imply that SPARQL defines a unique DESCRIBE graph. FROM merges standardize
source graphs apart, while ordinary GRAPH access preserves root dataset identity.

Graph results use standard RDFC-1.0. SELECT canonicalization encodes the
entire result table: it distinguishes value blank nodes from row nodes, preserves
bag occurrence counts and cross-row identity, and binds ordered-result indices.
Per-row relabeling cannot satisfy this contract. The new bounded standard APIs in
`sparq-canon` provide explicit input/output and HNDQ/permutation preflight limits;
the V3 policy binds those limits and the explicit DESCRIBE closure policy.
Blank-free tables skip isomorphism work. Other tables use distinct typed row and
value nodes, retain each duplicate row occurrence, and encode sequence indices.
Unbound cells have no value edge; columns retain their ordered variable names.
Result serialization has a separate byte limit, including JSON escaping.

The dependency assessment used rdf-canon 0.15.3's `counter.rs` and
`canon.rs::hash_n_degree_quads`: default `None` selects 4000 calls, and
`call_counter.add` precedes each function body. Its sole `permutations` loop may
skip candidates before another counted call. If each node has at most `d` related
occurrences, each call has at most `d` groups of at most `d!` candidates, so
`call_limit × d × d!` bounds total candidates. Occurrences include repetitions;
checked overflow or excessive bounds reject before canonicalization. The bound
is conservative and can reject an easy graph; it never changes canonical output.
Size limits separately bound encoding, sorting, issuer maps and per-candidate work.

The native `bounded_canonicalization` tests target standard-output parity,
relabeling, each capacity, actual HNDQ exhaustion and repeated-neighbour counting.
They are not guest or proof evidence. The default-off engine feature
`deterministic-blank-nodes` provides anonymous parser labels and result-local
template allocation, with native freshness, collision and output-budget tests.
Source namespaces are checked across the active dataset. Independent result graphs
still require standardization apart before combination. V3 guest validation and receipts have separate test entry points; their source
presence does not establish successful execution.

[GPT-6] Native `dataset_blank_nodes` now exercises read-query RDF merge separation,
preserved named-graph identity, collision avoidance, repeated-IRI acquisition,
dataset views, graph production and recursive triple-term handling. Each distinct
FROM IRI supplies one snapshot; default-copy nodes are disjoint from preserved
named graphs. This is an explicit acquisition policy, not a claim that §13.2.3
requires a particular identity for repeated references. Update USING remains a
separate execution boundary. These tests do not expand V1/V2 admission or establish
V3 guest execution.

## Native model API

Enable the detached model's `graph-results` feature explicitly. It includes
`evaluate`, `sparq-canon` without its parallel default, and the engine's
`deterministic-blank-nodes` feature. The ordinary engine never depends on this
model or the canonicalization crate.

`v3::Witness` carries `v3::Request` and the same exact-source/catalog/salt container
as V2. `v3::Policy` nests the dataset capacities plus `CanonicalizationPolicy`
and `DescribePolicy::OutgoingBlankNodeClosure`. First compute
`v3::dataset_commitment(&dataset, &policy)` for an independently accepted snapshot,
or explicitly select `HolderDeclared`; then use `v3::evaluate(&witness)`.
The source commitment wraps the V2 byte-commitment component under a separate V3
domain and full V3 policy. Equivalent source relabelings can have different
commitments while yielding the same canonical result.

`v3::CanonicalResult` returns SELECT variables/order/rows, ASK truth, or canonical
graph N-Triples. `v3::bind_journal` checks independently expected version, request
and authority **after receipt verification**; calling it alone verifies no proof.
The typed host `v3::{prove_with_artifact, verify_with_artifact}` APIs and guest
version dispatch define the V3 relation. Native and actual-guest checks are recorded in
[the integrated checks](../../../zk/sparql-evaluator/graph-results-bound-integration-checks.json);
genuine receipts require the separate immutable artifact campaign.

```sh
cargo test --locked --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator-model --features graph-results --test graph_results
```

The native tests exercise whole-table identity and relabeling, bag duplicates,
sequence order, unbound cells, CONSTRUCT freshness/omission, the explicit DESCRIBE
closure, FROM separation, both authority modes, capacity rejection and unchanged
V2 exclusions. BNODE(), nondeterminism, SERVICE, triple terms and blank-node graph names remain
rejected. V3 admits the earlier positive EXISTS/NOT EXISTS profile only when every
parsed default/named source graph is blank-free; any source blank node rejects
correlation, including in an unselected named graph. Structural `admit` cannot
establish that private-source condition; `evaluate` checks it inside the guest.
Query blank nodes are existential variables, no admitted expression creates blank
terms, and CONSTRUCT template nodes arise after WHERE evaluation. This avoids
the disputed SPARQL 1.1 captured-blank substitution semantics described in the
[EXISTS community report](https://w3c.github.io/sparql-exists/docs/sparql-exists.html);
this version declines the blank-containing case instead of assigning a practical
repair to the published Recommendation. All versions reject potentially captured
BOUND inside EXISTS; local BOUND is retained, with versioned actual-guest tests.

## Guest and receipt validation

`host/tests/actual_graph_results.rs` executes SELECT identity/order, query blank
nodes, FROM separation, CONSTRUCT and DESCRIBE in the actual guest, then checks
exact relation-panic rejection for capacity, domain and admission failures.
`real_graph_results.rs` requires three genuine Succinct receipts: a holder-declared
bag, verifier-agreed CONSTRUCT and holder-declared DESCRIBE. Independent request,
nonce, authority, policy, journal and cross-version substitutions must reject.
Changes to `sparq-canon` trigger the mandatory actual guest job. The helper retains
every V1/V2 receipt and requires all three V3 exports,
with the same independently checked artifact throughout. Use its evidence record
to determine execution status; these test definitions alone establish no receipt.

Sources: [SPARQL 1.1 CONSTRUCT](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#construct),
[DESCRIBE](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#describe),
[RDFC-1.0](https://www.w3.org/TR/rdf-canon/).
