// Informative companion to zksparql-answers.typ: how the code compares with the data model.

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "Zero-Knowledge SPARQL Answers: Implementation Status")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

#spec-head()

#intro-section("abstract", "Abstract")[
  How the code in the sparq repository compares with #emph[Zero-Knowledge SPARQL Answers: Request and Presentation Data Model] (https://sparq.jeswr.org/specs/zksparql-answers): what matches, and what is different or not built yet. This document is informative; the data model is the normative text.
]

#sotd()

This document is informative.

= Proof methods <sec-impl-methods>

The proof methods currently in the sparq repository, named as §9 of the data model defines,
are below. The table records what the code does, not what is assured: none of them has had
an external cryptographic audit.

#table(
  columns: (1.5fr, 1.3fr, 1.2fr),
  align: (left, left, left),
  table.header[IRI][Results][Issuer checking],
  [`urn:sparq:vcq:method:` \ `risc0-exact:v3` (RISC Zero program)], [The full result of
    SELECT, ASK and CONSTRUCT queries in its fragment], [none: usable only with a request that
    has no `trustedIssuers`],
  [`urn:sparq:vcq:method:` \ `risc0-authenticated-rdf:v5` (RISC Zero program)], [The full
    result of SELECT, ASK and CONSTRUCT queries in its fragment], [`eddsa-rdfc-2022` against
    `IssuerKeys`, inside the proof],
  [`urn:sparq:vcq:method:` \ `noir-selected-support-unsigned:v2` (Noir circuits)], [Only that
    returned solutions are solutions: ASK when `true`, and SELECT over basic graph patterns
    with integer FILTERs; not usable with the data model yet (@sec-impl)], [Schnorr
    signatures over the sparq commitment format, inside the proof],
  [`urn:sparq:vcq:method:` \ `disclosed-reevaluation:v1` (`proof-methods/disclosed`)],
    [The full result; the holder sends the signed credentials and the salt, and the verifier
    checks the signatures and evaluates the query itself. Its dataset commitment is the one
    `risc0-authenticated-rdf:v5` computes, so one agreed commitment serves both. A baseline
    that hides nothing.], [`eddsa-rdfc-2022` against `IssuerKeys`, by the verifier],
  [`urn:sparq:vcq:method:` \ `tee-attestation:v1` (`proof-methods/tee`)], [The full result,
    attested by a program in an AWS Nitro Enclave whose attestation document carries the
    statement digest. Only the `aws-nitro` platform is built.], [`eddsa-rdfc-2022`, inside the
    enclave; hidden mode only],
)

A prototype of `vole-designated-verifier:v1`, an interactive proof based on vector oblivious
linear evaluation (VOLE) as in QuickSilver, proves the ACIR of the Noir circuits
(`proof-methods/vole`); it is not yet a proof method in the sense of the data model. The
following methods are proposed and not built. Their IRIs use the same prefix and end in
`:v1`.

#table(
  columns: (1.5fr, 1fr, 2fr),
  align: (left, left, left),
  table.header[Name][Evidence kind][What it is],
  [`selective-disclosure-reevaluation`], [disclosed credentials, selectively], [The holder
    discloses, with `bbs-2023` or `ecdsa-sd-2023`, the claims each returned solution uses,
    together with any claims the issuer made mandatory to disclose and the structure the
    cryptosuite needs; the verifier checks them and evaluates the query over them. It can
    show only that returned solutions are solutions, so it supports the same results as the
    Noir method above.],
  [`vole-designated-verifier`], [zero-knowledge proof, interactive, designated-verifier],
  [An interactive proof based on VOLE. It convinces only the verifier that took part.],
  [`tee-attestation` on other platforms], [attestation], [The `parameters` name the platform:
    `intel-tdx`, `amd-sev-snp` or `nvidia-cc`.],
)

= Implementation status <sec-impl>

This section compares the data model with the code in the sparq repository on 2026-10-10.

What matches: the two RISC Zero proof methods (`zk/sparql-evaluator`) prove the full result
of a query over an input dataset and bind a request digest, the dataset commitment, whether
the commitment was agreed, and the canonical result in their public output (the
#emph[journal]). The `sparq-query-protocol` crate holds the verifier-side request, the
admission check against declared proof-method capabilities and the one-time challenge store.
Through the adapters in `zk/sparql-evaluator/host` (features `vcq` and `vcq-authenticated`),
the challenge, domain and validity period reach the proof through a nonce derived from the
request.

What differs or is missing:

+ No code produces or parses the JSON-LD documents of the data model (its §4 and §5), and its
  context and vocabulary are not yet published at their `w3id.org` IRIs. The
  adapters pass Rust values, and the presentation is a descriptor digest plus the serialized
  receipt.
+ The request digest is computed over a project-specific binary encoding of the Rust values,
  not over RDFC-1.0. Two implementations could not yet agree on a digest.
+ The code still carries an answer kind (Supported or Exact), which the data model no longer
  has. The RISC Zero methods implement what is here the only kind; the Noir method implements
  only the "returned solutions are solutions" results of §9 of the data model.
+ Results are in the proof's own canonical form (N-Triples term strings), not SPARQL Query
  Results JSON or a JSON-LD graph; the conversion of §5.1 of the data model is not written.
+ The evaluator implements SPARQL 1.1 and has no triple terms. The adapters also reject
  SELECT queries with ORDER BY, and the evaluator does not support SERVICE, DESCRIBE, FROM or
  nested EXISTS. Each method's published list of excluded features has still to be written.
+ The version 5 RISC Zero method implements the hidden and revealed signature modes for
  `eddsa-rdfc-2022`, and the zkVM relation implements `eddsa-sha256-merkle-2026`; the other
  Merkle-root cryptosuites are specified only. The Noir circuits support both modes, but the
  Noir method (`noir-selected-support-unsigned:v2`) cannot yet produce a presentation for the
  data model:
  it keeps graph roots and salts private, so it has no dataset commitment to publish, and it
  requires a credential-status root that the data model has no property for.
+ The version 5 policy (issuer keys and capacity bounds) has no RDF encoding yet, so the
  mapping from `trustedIssuers` and `parameters` to the adapter's parameter digest is not
  defined.
+ The code names methods by an identifier without the `:vN` suffix plus a separate version
  number; the data model folds the version into the IRI.
+ The methods in `proof-methods/` work on the internal version 5 request and statement, which
  do not bind the domain, the validity period, the accepted methods or the signature mode,
  and they define no canonical statement or proof encoding yet.
+ The RISC Zero methods' image ID is eight 32-bit words; the code encodes it as the words in
  order, each little-endian.
+ Only one end-to-end proof of the version 5 adapter has been made and independently checked
  (a SELECT over an agreed input); its other combinations have run only in tests without
  proving.

The older zkSPARQL proposal on this site describes the Noir proof manifest in detail. The data
model replaces neither it nor the code; it fixes the data model that both sides should
converge on.

= Defining the reserved trust requirements <sec-impl-trust>

The types the data model reserves (its §6.3) follow the trust sources verifiers already use. Defining one needs three
things the data model does not yet fix: which entries of the list count as issuers of
credentials; how holder and verifier agree on the same version of the list, for example by
the verifier resolving it at a stated time and putting a digest of the sorted key set in the
request; and how a proof shows that each credential's key is in that set, for example by a
Merkle membership proof against that digest, which also keeps hidden which listed issuer
signed. Until then a verifier that trusts the issuers on a list can resolve the list itself
and send the result as `IssuerKeys` entries. A vocabulary for verifier trust policies is
also planned by the W3C Verifiable Credentials Working Group ("Verifiable Issuers and
Verifiers"); a later version of the data model should reuse it.
