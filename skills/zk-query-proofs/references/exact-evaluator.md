# Exact dataset evaluation

<!-- [GPT-6] zkp-10.1, experimental and not externally audited. -->

Use the detached [`zk/sparql-evaluator`](../../../zk/sparql-evaluator/README.md)
workspace when experimenting with exact evaluation of a bounded default graph.
This is distinct from `sparq_zk_compose::result` selected-answer support.

Construct a verifier-owned `Request` with `ExactDataset`, an independently
accepted `VerifierAgreed` commitment, the exact query, resource policy and fresh
challenge. Supply complete private N-Triples bytes and a cryptographically random
salt through `PrivateDataset`. The verifier should accept a snapshot boundary
based on its application policy; a holder's proposed hash does not by itself
establish that boundary or wallet completeness.

`prove(&Witness, &Path)` invokes an explicitly named local RISC Zero server.
`verify(&Presentation, &Request, &mut impl Nonces)` verifies its real receipt,
locally compiled method ID, independent expected request, and atomic nonce use.
`Nonces` must use durable storage in applications. Do not use `model::evaluate`
or `model::bind_journal` as substitutes for receipt verification.

`HolderDeclared` is supported only as an explicit weaker authority mode. Keep the
returned `HolderDeclaredOnly` provenance visible to downstream policy decisions.
No issuer signatures or credential status are checked by this first adapter.
For separate deployments, export a reviewed guest with the `export_guest` example,
independently pin its artifact digest and execution ID, and load `AcceptedGuest`
on both peers. Use `prove_with_artifact` and `verify_with_artifact`; locally rebuilt
constants are not assumed portable across checkout paths. Reject pins received
only from the proof sender. The byte digest is checked before ELF processing.
See the workspace README for limits, exact bag/sequence/unbound encoding, rejected
features, proof-mode leakage caveats and the host-versus-guest evidence distinction.
EXISTS/NOT EXISTS bodies in this first profile admit only BGP/join/UNION and pure
FILTER, including fixed sequences lowered to BGP. Nested EXISTS, residual paths,
OPTIONAL/MINUS, binding operators, subqueries and modifiers inside those bodies
are rejected pending explicit published-SPARQL-1.1 substitution support. Their
admission elsewhere is unchanged; these rejections do not count as conformance.

This experimental backend also inherits the upstream [privacy-assurance limitation](https://dev.risczero.com/api/security-model#zero-knowledge-proving):
the perfect-zero-knowledge target has no written mathematical argument yet, and
[GHSA-5xgj-pmjj-gw49](https://github.com/risc0/risc0/security/advisories/GHSA-5xgj-pmjj-gw49)
still lists all versions. Treat application tests, receipt checks and individual
upstream audits as evidence only for their stated scopes, not a settled privacy
claim or proof that the advisory is resolved.
