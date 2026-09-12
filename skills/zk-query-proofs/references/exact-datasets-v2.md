# Complete named-dataset evaluation

<!-- [GPT-6] zkp-10.4; experimental relation, not externally audited. -->

The detached evaluator's `model::v2` and `host::v2` APIs evaluate a complete bounded
RDF dataset with a default graph and an explicit catalog of IRI-named graphs.
The catalog includes empty graphs, which N-Quads source cannot represent. This
experimental path is not externally audited and makes no complete SPARQL
conformance, credential-authentication or general privacy claim.

Use `sparq_proved_evaluator_model::v2::{Request, Policy, PrivateDataset, Witness}`.
Set `version` to `v2::VERSION`, `contract` to `ProofContract::ExactDataset`, and
`dialect` to `v2::Dialect::SparqSparql11DatasetV2`. `PrivateDataset` contains the
complete `nquads` source, every `named_graphs` IRI, and a cryptographic `salt`.
`Policy` bounds their total byte size, source quads before deduplication, output
rows and named-graph count. These capacities participate in both commitments.
The named-graph capacity bounds the input catalog; dataset-clause references have
a separate static admission bound in the pinned program.

`v2::dataset_commitment` sorts the exact catalog IRI strings, then commits to the
versioned domain, policy, salt, each length-prefixed IRI and the exact N-Quads
bytes. Catalog ordering is irrelevant; adding, removing or renaming an empty
graph changes the commitment. This is exact source-byte binding, not RDF dataset
canonicalization. Equivalent N-Quads serializations can have different anchors.
The guest validates absolute IRI syntax, unique catalog membership, and every
quad's graph name. A source quad naming an undeclared graph is rejected even in
holder-declared mode. Blank-node graph names and source blank nodes are excluded.

Both `DatasetAuthority` modes retain their V1 trust meaning. `VerifierAgreed`
checks an independently accepted complete input commitment. `HolderDeclared`
reports exact evaluation of the holder's declared input, with
`Provenance::HolderDeclaredOnly`. Neither mode authenticates issuer signatures,
credential status or the contents of a holder's wallet. An accepted commitment
is an application expectation, not independent evidence of source authenticity.

Call `sparq_proved_evaluator::v2::prove_with_artifact` with the typed witness,
explicit local `r0vm` path and an independently approved `AcceptedGuest`. Verify
using `v2::verify_with_artifact` with the verifier-owned expected request, the
same approved program and persistent `Nonces`. The common artifact pin includes
both the complete artifact-byte hash and its method ID; it must come from trusted
deployment configuration, never from the presentation. Receipt-kind and replay
checks match V1. Returned SELECT bags retain duplicates and unbound cells; outer
ORDER BY results retain sequence order. ASK includes false results.

GRAPH changes the active graph while retaining the active dataset catalog.
Nested constant and variable GRAPH therefore resolve against the same catalog,
including empty graphs. FROM merges the selected local snapshot graphs into an
active default graph; FROM NAMED determines the active named catalog. Default
and named graph data remain distinct unless the query explicitly requests a
merge. Duplicate source triples are RDF graph set members, while different graph
matches can produce duplicate result mappings.

Dataset IRIs are resolved only within the committed local snapshot. There is no
network retrieval. Under the existing engine's snapshot-resolution policy, an
absent FROM source contributes an empty graph, and an explicitly selected absent
FROM NAMED source contributes an empty named graph in the derived active dataset.
Consequently `FROM NAMED <absent> { GRAPH <absent> {} }` does not establish that
the input catalog contained that graph. The complete input catalog and the
query-derived active catalog are distinct objects.
To test membership in the accepted input catalog, verifiers should omit dataset
clauses and query GRAPH directly against that input catalog.

V2 supports SELECT and ASK with the shared bounded deterministic profile. SERVICE,
LATERAL, graph-producing query forms, updates, volatile functions and the existing
complex-EXISTS/nullable-composition exclusions remain rejected. GRAPH inside an
EXISTS body is still outside that conservative profile. SPARQL 1.2 is not enabled
as an advertised dialect.

V1 and V2 use separate types, request-digest domains and dataset-commitment domains.
The guest dispatches on the request's existing first version word, preserving the
V1 witness wire layout. It bounds raw input before deserialization and requires
the exact typed SDK encoding, rejecting trailing words and noncanonical padding.
Old V1-only artifacts remain usable through V1 APIs and
their independently accepted pins; they cannot satisfy V2 requests. Verification
rejects cross-version journals even when the result happens to be identical.

Native contracts and graph semantics are tested in `model/tests/datasets.rs` and
the V2 model unit tests. The original default-graph conformance goldens also run
through V2 without changing their expected mappings. These are semantic evidence,
not cryptographic receipts.
`model/tests/datasets.rs` and the actual guest negatives share
`fixtures/v2-admission-rejections.json`; the native test also accepts the exact
dataset-reference capacity boundary. `host/tests/real_datasets.rs` defines the two
V2 receipt fixtures, catalog/framing negatives and V1/V2 wire checks. All V1 host
tests remain required on the changed dual-version guest image.
The [evaluator README](../../../zk/sparql-evaluator/README.md) describes pinned
toolchain execution, receipt evidence and deployment limitations.
