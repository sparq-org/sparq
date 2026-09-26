# Issuer-authenticated RDF: native V5 model

[OPUS-5.5] **Native model only.** No guest adapter, host API, receipt or real
proof exists for this relation yet. It makes no conformance or performance claim,
is not a complete Data Integrity processor, and is not externally audited. Treat
it as research-grade and not yet sound.
<!-- privacy-claims-allow: native model only; explicitly not audited, no proof exists -->

The detached model crate's `authenticated-rdf` feature (off by default) adds
`sparq_proved_evaluator_model::authenticated_rdf`, relation version 5. It is
separate from V1–V3 and from historical V4 work, and it leaves their requests,
encodings and APIs unchanged. It enables `graph-results` and an optional exact
`ed25519-dalek = 2.2.0` dependency (`default-features = false`, `alloc`).

## Relation

`Request` binds the V3 query, `v3::Policy` (as `Policy::evaluation`), a
`DatasetAuthority` and a nonce. It also binds a verifier-owned table of 1–16
`AuthorizedKey { issuer, verification_method, public_key }` entries and the fixed
`Cryptosuite::EddsaRdfc2022` and `Mapping::ScopedCanonicalUnion` profile.
`Policy::new(table)` builds the fixed profile with default V3 capacities. Table
IRIs must be absolute and at most 512 bytes. Verification methods must be unique,
and keys must decompress and must not have small order.

`Witness` holds only `PrivateCredentials`: 1–4 `SignedCredential { document,
proof_config, signature }` values and a salt. It has no separately supplied query
dataset. Byte and statement bounds (`MAX_*` constants) reject before parsing.

For each credential, the model parses both N-Quads inputs. It admits only the
default graph and rejects triple terms and directional literals. It then
canonicalizes both inputs with bounded RDFC-1.0/SHA-256 and reads every check from
those exact canonical bytes:

- **Proof configuration.** One proof node carries exactly:
  - `rdf:type sec:DataIntegrityProof`;
  - `sec:cryptosuite "eddsa-rdfc-2022"^^sec:cryptosuiteString` (a plain literal
    rejects);
  - one IRI `sec:verificationMethod`;
  - `sec:proofPurpose sec:assertionMethod`;
  - optionally one `dcterms:created` typed `xsd:dateTime`.

  Any other option rejects, including challenge, expiration, `previousProof` and
  `proofValue`.
- **Document.** It has no `sec:proof` or `sec:proofValue`, exactly one
  `VerifiableCredential` node, and exactly one IRI issuer.
- **Authorization.** The table must contain the verification method, and the
  document issuer must equal that entry's pinned issuer.
- **Signature.** Strict Ed25519 over `SHA-256(config) || SHA-256(document)`.

A valid signature alone is therefore insufficient.

Verified credentials are sorted by document hash, and duplicate canonical
documents reject. The model builds the V3 dataset internally as a default-graph
union of the hashed canonical document quads. Each credential's blank nodes get
their own scope, so they never join across credentials. Signed literal lexical
forms are kept, for example `"018"^^xsd:integer`. V3 evaluates under an internal
`HolderDeclared` authority, and only its result is kept.

## Commitment and journal

`dataset_commitment(&PrivateCredentials, &Policy)` runs the full authentication.
It then hashes, under a version-5 domain:

- a policy digest that frames the suite and mapping profiles, every fixed bound,
  the serialized V3 policy and the method-sorted table;
- the salt;
- a `u32` credential count;
- per sorted credential, the 32-byte canonical document and proof-configuration
  hashes that were signed.

A commitment accepted under one policy cannot anchor another. Salt reuse is
publicly linkable.

`evaluate(&Witness) -> Journal { version, request_digest, dataset_commitment,
provenance, result }`. The journal does not publish a credential count or issuer
list. `request_digest` frames every request and policy field. After receipt
verification (not yet available), `bind_journal(&journal, &expected)` checks the
version and request digest. For `VerifierAgreed`, it also requires the new
authenticated commitment and `Provenance::VerifierAgreedAuthenticated`. For
`HolderDeclared`, it requires `Provenance::HolderSelectedAuthenticated`. Calling
it verifies no proof.

## Not established

Neither provenance implies wallet or world completeness: a holder may omit
credentials. The model does not check:

- credential status;
- holder binding;
- validity periods;
- `created` against a clock;
- DID or controller resolution.

The table is the verifier's only trust input.

`created` accepts only a restricted lexical profile: positive four-digit years
from `0001`, whole seconds and `Z`. Other valid XML Schema 1.1 `dateTimeStamp`
values, such as year `0000`, fractions, offsets and `24:00:00`, reject as
unsupported. Malformed values reject separately. The existing temporal parser
uses XSD 1.0 year rules, so it is not reused here. Full typed proof-option
handling is left for the native VC API integration.

## Tests

The native tests use the published W3C `vc-di-eddsa` `eddsa-rdfc-2022` vector
(canonical bytes, hashes, key and signature) plus deterministic synthetic keys.

```sh
cargo test --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator-model --features authenticated-rdf --test authenticated_rdf
cargo test --manifest-path zk/sparql-evaluator/Cargo.toml \
  -p sparq-proved-evaluator-model --features authenticated-rdf --lib authenticated_rdf
```

Add `--locked` once the detached `zk/sparql-evaluator/Cargo.lock` has been
regenerated to include `ed25519-dalek`.
