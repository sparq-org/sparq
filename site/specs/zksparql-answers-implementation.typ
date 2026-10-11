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

= Implementation status <sec-impl>

This document is informative. It compares the data model with the code in the sparq
repository on 2026-10-10.

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
