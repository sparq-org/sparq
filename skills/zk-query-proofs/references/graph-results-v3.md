# Blank nodes and graph results: V3 implementation design

[GPT-6] This is work in progress after the separate V2 named-dataset relation.
It does not establish V3 guest execution, receipts or full SPARQL coverage.

The intended V3 schema will preserve V1/V2 request and commitment meanings.
It will bind source blank-node identity within the complete N-Quads document,
support query blank nodes as existential variables, and publish result-local
canonical labels. Source labels must not become public correlators. The complete
IRI-named graph catalog remains explicit; blank-node graph names remain excluded
from this pinned SPARQL 1.1 profile.

CONSTRUCT must instantiate a template once per solution occurrence, allocate
fresh blank nodes per occurrence, preserve repeated labels within that occurrence,
avoid collisions with bound source nodes, omit unbound/illegal triples and retain
graph set semantics. DESCRIBE will explicitly bind the implementation's outgoing
blank-node closure over the active default graph, with cycle detection; it will
not imply that SPARQL defines a unique DESCRIBE graph. FROM merges must standardize
source graphs apart, while ordinary GRAPH access preserves root dataset identity.

Graph results will use standard RDFC-1.0. SELECT canonicalization must encode the
entire result table: distinguish value blank nodes from row nodes, preserve bag
occurrence counts and cross-row identity, and bind indices for ordered results.
Per-row relabeling cannot satisfy this contract. The new bounded standard APIs in
`sparq-canon` provide explicit input/output and HNDQ/permutation preflight limits;
the V3 policy and table encoding are still to be implemented.

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
still require standardization apart before combination. Graph merge identity,
complete V3 APIs and actual guest validation remain outside this prerequisite.

Sources: [SPARQL 1.1 CONSTRUCT](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#construct),
[DESCRIBE](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#describe),
[RDFC-1.0](https://www.w3.org/TR/rdf-canon/).
