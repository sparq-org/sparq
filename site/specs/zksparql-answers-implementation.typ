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

The data model reserves these trust requirement types (its §6.3):

#table(
  columns: (1.2fr, 3fr),
  align: (left, left),
  table.header[Type][Trust source],
  [`TrustedList`], [Issuers listed for a service type in an ETSI trusted list
    (TS 119 612, as the eIDAS national lists are #cite("ETSI-TS-119-612")) or list of trusted
    entities (TS 119 602, as for EU PID providers #cite("ETSI-TS-119-602")), given by the list's
    IRI and the service type IRI. Corresponds to the OpenID4VP `trusted_authorities` type
    `etsi_tl` #cite("OID4VP").],
  [`FederationTrustAnchor`], [Issuers with a trust chain to an OpenID Federation trust anchor,
    optionally holding a trust mark of a given type #cite("OPENID-FEDERATION"). Corresponds to
    `openid_federation`, which has no trust mark condition.],
  [`RecognitionCredential`], [Issuers that a recognition credential, signed by an authority
    the verifier trusts, recognizes for a purpose #cite("VC-RECOGNITION"). This is the one
    source here that is already RDF.],
  [`TrustFramework`], [Issuers certified under a governance framework, such as the UK digital
    identity and attributes trust framework, with the scope of the certification checked as
    the sparq trust-expression proposal describes #cite("SPARQ-TRUST-EXPRESSION"). The UK
    register names certified organisations and services but publishes no keys, so it cannot
    yet be resolved to a key set.],
)

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

= Draft: Noir query credential statement method <sec-impl-noir-draft>

#strong[Draft — public policy integration. Not an available proof method.]
The proposed identifier is `urn:sparq:vcq:method:noir-query:v1`. The statement amendment is
@sec-impl-envelope-draft. The envelope is registered in the real core package alongside the suite and compiler
operators. Its fixtures include hand-built cells and a complete suite-message opening.
Authenticated metadata remains explicitly marked; stored-policy wiring awaits compiler integration. It does not
produce an answers-data-model presentation. This method is research grade, not externally
audited; its description states a proposed contract, not established assurance.

== Evidence, query scope and trust

The proposed evidence is a publicly verifiable UltraHonk zero-knowledge proof over one Noir
program generated from the verifier's query and compiled capacities. The method targets complete
bounded SELECT, ASK (including false) and CONSTRUCT answers over the credential named graphs and
their default merge, under simple entailment. Absence is relative to the selected input dataset;
the data model's closed-world qualifications apply even for an agreed commitment. Agreement
authenticates which dataset was selected, not world completeness.

The target excludes SERVICE, FROM, FROM NAMED, DESCRIBE, evaluation-time and environment
functions (NOW, RAND, UUID, STRUUID and argument-free BNODE), RDF triple terms, and REGEX or
REPLACE forms outside the admitted bounded expression profile. Property-path closure must fit
its compiled depth. Numeric and lexical operations must fit the exact admitted value and byte
capacities. An overflow, unsupported datatype, expression, cardinality or time precision is a
refusal, never rounding, truncation or substitution. This is the target fragment; this step
provides no executed query coverage or generated-program integration. Admission must follow the
actual coverage manifest as subsequent lanes supply it.

The verifier must trust its issuer-to-key authorization data, authenticated and
freshness-curated status snapshots, clock and durable one-time challenge store; the suite's term
and signature encodings; the correctness of lowering, operator gadgets and generated program;
the pinned compiler and backend; and the proof system's assumptions and reference string
provenance. Both parties derive the plan and verification key from the request. The holder
refuses an entry whose key differs from its independently derived key. The verifier uses its
stored query and domain, never a query or key chosen by the presentation. Trust in these
components is required in addition to any issuer claims about the world.

== Statement, layout and disclosures

The target public statement binds the complete request digest, dataset commitment, chosen
signature mode and canonical released result. The current authenticated compiler layout is
request digest bytes, input commitment bytes, signature-mode byte, credential/quad/row bounds,
term parameters, index parameters, released result, issuer root, ordered credential quad bounds
and statement binding, as defined in `statement::Layout` and `ops_core_statement`.

The agreed envelope extension replaces that single issuer-root slot with the following group,
after the result and before `credential_quad_bounds`, with exactly this declaration order:

```noir
issuer_root: Field
issuer_depth: u32
holder_required: bool
holder_issuers_present: bool
holder_issuer_count: u32
holder_issuers: [Field; 4]
status_required: bool
allow_missing_status: bool
snapshot_root: Field
snapshot_depth: u32
validity_required: bool
now: u64
freshness_required: bool
earliest_created: u64
```

The holder issuer list holds unique suite IRI encodings in unsigned big-endian field order, with
zero padding. Its capacity is four entries; a request above that capacity is refused before
deduplication. The group flattens to seventeen canonical big-endian field words. Rust's
`envelope_policy` module derives these words and both minimal set depths from typed
stored-policy input. Disabled flags have zero subordinate words, which the circuit asserts. An
absent holder list has zero count and issuer words. Roots are computed by the verifier, never by
a generated circuit. Compilation may provide larger path capacity, but the circuit hashes only
levels below the public depth, bounds that depth, and checks unused siblings and index bits are
zero.

There is no request or policy derivation inside the circuit. The existing Rust request module
computes the RDFC stored-request digest; it enters as thirty-two public bytes. The holder
message hashes its two exact unsigned big-endian halves under the holder domain. The verifier
reconstructs every policy word independently from that same stored request and verifies against
that entire public sequence, rather than accepting policy words from a presentation. A proof
with evidence for policy B cannot verify under public policy A. Query constants, capacities and
the result remain bound by the shared statement. A plan hash identifies the generated structure
and encoding version.

This partial stage supplies the policy helper and envelope gadgets. The existing compiler's
key-only issuer membership and fixed depth must be changed together with its public
declarations, reconstruction and statement binding by the compiler lane before the extension is
admitted. This stage does not edit those call sites or claim that generated circuits already run
the complete envelope. The method registry remains unchanged until admission.

Hidden mode keeps credential messages, issuer selections, holder keys and signatures, status
indices and paths, selected snapshots, individual bound-presence flags and all intermediate
tables private. Each selected issuer must still satisfy the authorization-set relation. The
query can disclose an issuer or a credential field, and a singleton trust set can identify an
issuer without projecting it. Revealed mode would additionally disclose issuer keys, signatures,
messages and the number of credentials; its integration belongs to step two.

The verification key and plan identify the compiled credential, quad, intermediate-row,
lexical-byte, numeric and path-depth buckets. These are public upper bounds, chosen from request
bounds, not assertions of actual private cardinalities. The released result exposes its size and
duplicates, and any query-selected identifiers. A public key-set or snapshot-set digest can
distinguish verifier policies. Fresh presentation-local graph names avoid a persistent graph
identifier; stable selected values can still link answers. Canonical term order fixes choices
left open by SPARQL, including unordered slices, SAMPLE and GROUP_CONCAT ordering in admitted
profiles.

The holder service needs a consent and query-history decision before repeated or related
requests, across domains when practical. Comparing results can reveal facts neither answer alone
states. This step does not implement that service decision. Agreed commitments and reused
presentation salts are linkable; revealed signatures remain stable. Hidden mode and fresh salts
do not eliminate linkage through the result, policy, bucket or query choices.

== Draft encodings and present boundary

The proposed `verificationKey` is the exact backend verification-key byte file, carried as the
data model's multibase base64url byte string without padding. It is opaque bytes, not a sequence
of host-endian words, and its byte identity is checked against independent plan compilation
under the pinned toolchain. An encoding change gets a new method identifier.

`parameters` is JCS JSON with required `planHash`, `encodingProfile`, `capacities` and
`toolchain` members. `planHash` is a multibase SHA-256 digest of the canonical plan bytes;
`encodingProfile` names the final suite/term, statement and snapshot profiles. `capacities`
names all the public buckets above. `toolchain` pins the compiler and backend versions. There
are no implicit capacity or profile defaults. The accepted request carries the trust and
snapshot digests and requirement values as defined in the amendment. The draft context defines
the new request terms and the amendment fixes policy-root ordering, padding and minimal depth.
Admission still requires the integrated plan and digest encodings; the local stand-ins are not
these encodings and cannot be sent as a method descriptor.

The draft `proofValue` encoding is a length-prefixed evidence frame: an unsigned big-endian
length followed by the backend proof bytes, then an unsigned big-endian length followed by the
backend public-input bytes. Both lengths are four-byte unsigned integers, with the public-input
payload consisting of canonical big-endian field elements. Declared lengths must equal the
remaining framed payloads and fit the request bound. The verifier requires exact consumption
with no trailing bytes, enforces the presentation bound before decoding, and compares the entire
public-input sequence against its reconstruction. No key, source program or private diagnostic
is carried in that frame. The frame and statement byte vectors must be checked against the
generated ABI in step two; there is no interoperable proof encoding in this step.

Dataset commitment and revealed signature integration are reserved for step two. No cross-method
equality with the zkVM commitment is claimed here. Equality requires identical signed-message
bytes, ordering, presentation salt and commitment algorithm on both methods; the revised suite
and final adapter determine whether those premises hold. A verifier must not reuse an agreed
commitment across methods before that comparison is fixed and tested. The shared `QueryMethod`
admission and stored-request/challenge-store adapter are also step two work under the programme
issues. This draft adds no alternate nonce store.

The method registry's `entry_status` enumeration has no draft value. This step therefore does
not register an executable or proposed adapter in `research/vc-query-methods.json`.

= Draft amendment: implementation boundary <sec-impl-envelope-draft>

The answers-data-model amendment proposes credential named graphs, a method-asserted
`vcq:verifiedIssuer` triple, `holderBinding` with an optional `issuer` collection,
`earliestCreated`, `statusPolicy` with accepted snapshot digests and an explicit missing-status
option, and `validityTime`. Existing methods retain their declared semantics until they opt into
this profile. The proposal vocabulary and context now include the draft `verifiedIssuer`,
`holderBinding` and `earliestCreated` terms. The transport adapter is unchanged.

`zk/noir-query/core/src/envelope.nr` uses complete credential quad arrays with the design's term
cells and padding. Property reads count every matching quad; a presence flag is computed, not
supplied. All credential-root status entries must be covered. Unsupported shapes and multiple
values where the profile requires a singleton are refused. The graph builder emits the complete
named graph and default copies plus the verified-issuer assertion, and the envelope set-merge
helper collapses duplicate default triples while preserving every credential named graph.
Compiler query integration remains separate.

The envelope adapts lane A's authenticated term observations to lane B's distinct table
representation. Its IRI constructor and vocabulary use the suite encoding. The complete suite
opening recomputes the salted signed message before table conversion. Created instant, Multikey,
status purpose/index and scoped graph-name metadata still await their authenticated openers.
Hand-built fixtures continue to use marked metadata transports; they are not admitted credential
witnesses. The request-digest and policy-derivation stand-ins have been removed. Rust derives
the digest and policy words from the stored request; the circuit receives them as public inputs
and performs no request normalization. The batch entry applies the public policy to every
credential. Holder evidence is deduplicated by full key coordinates and each required key has
one signature over the public request digest. The combined fixture covers four credentials and
their default/named dataset merge, with controls changing every condition on its final
credential. Status binds list identifier, purpose, length, compiled tree depth and root in an
accepted snapshot digest and proves the credential's actual index clear. Status leaves hash the
status-leaf domain tag, the boolean bit and a zero field; two-input internal nodes keep their
distinct arity. The snapshot digest hashes its domain tag followed by list, purpose, bit length,
depth and root, in that order. The status-bit tree depth remains inside the snapshot digest and
is distinct from the public snapshot-set membership depth. Validity endpoints are inclusive,
with an absent individual bound contributing no comparison and reversed bounds refused. When
both bounds are absent, the creation instant from suite metadata is the inclusive start;
unavailable creation metadata refuses a validity request. Freshness compares the authenticated
creation instant with inclusive `earliestCreated`; unknown creation refuses only a required
check, and absence imposes no freshness condition. This does not replace the validity fallback
above. The verifier computes the bound. The temporal fixture uses exact whole-second offset
keys; finer or wider times need the suite's checked temporal opening, not a cast or rounding
operation.

The accepted-snapshot and trust roots follow the amendment exactly: sort unique canonical
unsigned field values as thirty-two big-endian bytes, refuse zero members and empty enabled
sets, pad with zero leaves to the least power of two and reduce adjacent pairs with the
two-input Poseidon2 hash. A singleton has depth zero. The Rust helper returns roots, minimal
depths and sorted unique members; a separate Noir native test helper supplies fixed
cross-language vectors. The native helper lives only in the test package, so envelope entry
points cannot call it. Capacity does not extend the tree. The default-graph `verifiedIssuer`
assertion names the same presentation-local graph blank node used by `GRAPH`; its issuer object
is an IRI with exact trust-member and credential spelling. No IRI normalization is performed,
and credentials containing the reserved predicate are refused.

Vocabulary was checked against the VC Data Model, Bitstring Status List and VC Confidence Method
editor sources on 2026-11-13, using the exact source URLs recorded in the amendment.
`confidenceMethod` on a credential subject and the cryptographic-key profile provide the holder
relation; its Schnorr encoding is specific to this draft. The current VC Data Model editor
source identifies a later draft than the cited v2.0 Recommendation. The draft retains v2.0
validity semantics, including the indefinite-period note, and adopts no new requirements from
the later version without saying so.

The exact editor documents used, retrieved 2026-11-13, are
#link("https://raw.githubusercontent.com/w3c/vc-data-model/main/index.html")[VC Data Model],
#link("https://raw.githubusercontent.com/w3c/vc-bitstring-status-list/main/index.html")[Bitstring Status List], and
#link("https://raw.githubusercontent.com/w3c/vc-confidence-method/main/index.html")[VC Confidence Method].

= References <sec-refs>

#references((
  ("ETSI-TS-119-612", [ETSI. #emph[Electronic Signatures and Trust Infrastructures (ESI);
    Trusted Lists]. ETSI TS 119 612.]),
  ("ETSI-TS-119-602", [ETSI. #emph[Electronic Signatures and Trust Infrastructures (ESI);
    Lists of Trusted Entities; Data model]. ETSI TS 119 602 V1.1.1, November 2025.]),
  ("OID4VP", [Terbu, O.; Lodderstedt, T.; Yasuda, K.; Fett, D.; Heenan, J. #emph[OpenID for
    Verifiable Presentations 1.0]. OpenID Foundation, Final Specification, 9 July 2025.
    https://openid.net/specs/openid-4-verifiable-presentations-1_0.html.]),
  ("OPENID-FEDERATION", [Hedberg, R.; Jones, M. B.; Solberg, A. Å.; Bradley, J.; De Marco, G.;
    Dzhuvinov, V. #emph[OpenID Federation 1.0]. OpenID Foundation, Final Specification, February
    2026.]),
  ("VC-RECOGNITION", [W3C Credentials Community Group. #emph[Verifiable Recognition Credentials
    v0.9]. Final Community Group Report, 20 March 2026.]),
  ("SPARQ-TRUST-EXPRESSION", [The sparq project. #emph[Trust Expression: A Verifier-Holder
    Contract for Framework-Anchored Attestation Queries]. Unofficial Proposal Draft, 2026.
    Published on this site as `trust-expression`.]),
))
