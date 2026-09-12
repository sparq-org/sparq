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
See the workspace README for limits, exact bag/sequence/unbound encoding, rejected
features, proof-mode leakage caveats and the host-versus-guest evidence distinction.
