# Host measurement boundaries

`paper_sweep` writes `sparq.zk-paper-sweep.v3` JSONL. Records without `schema`
are the legacy schema; readers also accept `sparq.zk-paper-sweep.v1`. The audit
loader validates v2 and v3 records and keeps reading legacy archives. V3 adds
bincode companion sizes; the existing JSON size fields and encoding identifiers
retain their previous meanings.
All `*_ns` values are wall-clock nanoseconds and local samples, not canonical evidence.

| Field | Boundary |
| --- | --- |
| `issuer_sign_ns[]` | Inclusive host issuance boundary: key construction, canonicalization, hashing, preparation, tree work where applicable, primitive signing and witness-form assembly. One entry per credential. |
| `issuer_signature_primitive_ns[]` | Only `SigningKey::sign` on the prepared message. Key construction, signature byte conversion, salt framing, canonicalization, hashing and message preparation are outside. |
| `issuer_merkle_tree_ns[]` | For S4, typed leaf encoding/hashing over parsed canonical quads, leaf sorting, leaf-row allocation and root computation. Canonicalization, parsing, proof-config hashing, signed-message hashing, document reassembly and signing are outside. Entries are null for RDF-C. |
| `credential_bytes[]` | Existing document plus proof-configuration UTF-8 bytes; unchanged. |
| `credential_encoded_bytes[]` | Minified `serde_json` number-array encoding of the complete `SignedCredential`, before revealed signatures are removed. Includes escaped document and proof configuration, raw proofValue bytes as a JSON byte array, Merkle salt where applicable, field names and framing. |
| `credential_bincode_bytes[]` | Bincode encoding of the same complete `SignedCredential`, including proof bytes and salt, before signature removal. |
| `proofs[].presentation_bincode_bytes` | Bincode encoding of the same complete `AnswerPresentation`, including receipt, metadata, journal, optional signatures, lengths and enum tags. |
| `proofs[].presentation_bytes` | Existing receipt JSON bytes plus raw detached-signature bytes; unchanged. |
| `proofs[].presentation_encoded_bytes` | Minified `serde_json` number-array encoding of `AnswerPresentation`: the complete nested `Presentation` (receipt, journal and receipt metadata) and detached signatures in journal order, or null in hidden mode. |
| `proofs[].verify_ns` | Aggregate production verification of that decoded envelope, including timing bookkeeping. Encoding, decoding and native-oracle comparison are outside. |
| `proofs[].verifier_receipt_ns` | Succinct-kind check, receipt verification with dev mode disabled and independently approved image identity, then journal decoding. |
| `proofs[].verifier_signature_ns` | Strict revealed-signature checks, once, after journal binding. Null in hidden mode. |
| `proofs[].verifier_request_ns` | Sum of disjoint intervals for entry-point mode guard, query/profile and request validation, journal binding, then nonce consumption. Preserves the original ordering around the other stages. |
| `proofs[].verifier_stages_ns` | Sum of the three work timers from that one verification. Timers never nest; timer bookkeeping and function return overhead remain in the aggregate only. |

The JSON carrier fields use serde_json number-array encoding: byte vectors and
receipt words serialize as arrays of JSON numbers. Minifying whitespace does not
make this a size-efficient binary carrier. The JSON size fields are useful for
that specific encoding; they are not a compact proof-size convention.

`credential_compact_encoding` and `presentation_compact_encoding` identify the
companion `bincode-1.3.3-fixint-le-v1` encoding. It uses the pinned SDK client's
existing bincode codec: fixed-width little-endian integers, binary byte vectors,
length prefixes, option discriminants and enum tags. Each companion serializes
the same complete typed value rather than summing component lengths. Tests decode
both carriers and compare their full JSON values. Bincode is a local companion,
without a claim of standardized interoperability or transport compression.
The JSON carrier remains the sweep's decoded verification input; the bincode
companion gives the paper a reproducible compact carrier convention.

For S4, `issuer_sign_ns` is a host re-implementation boundary. Schema v1 delegated
to `merkle_suite::issue`; v2 and later use the instrumented host issuer, whose
allocation, parsing strategy and timer overhead differ. The field name
is retained, but schema v1 and v2 archives are not strictly comparable. `run()`
checks byte equality with `merkle_suite::issue` after the timers stop and aborts
on drift, before native admission. This equality establishes issued bytes, not
equality of execution cost.

Timed inputs and outputs pass through `std::hint::black_box`. No warm-up is run
and no sample is automatically discarded. The first issuer, native evaluation,
standalone signature check and proof/verifier sample in a process can be cold;
later configurations share process state. Campaign analysis must declare its
warm-up/drop policy and retain that first-sample distinction. The reference S4
issuance check runs after the sampled issuance and can warm subsequent work.

`credential_encoding` and `presentation_encoding` identify the JSON encodings.
The host decodes the measured answer bytes into the same envelope that it verifies.
The independent verifier request is not holder-sent answer data. This encoding is
local to the low-level API, without HTTP/TLS framing or a claim of standardized
interoperability. The optional VCQ adapter supports only hidden RDF-C and therefore
cannot supply the common encoding for this grid. The credential encoding is the
complete host credential carrier, rather than JSON-LD or an RDF serialization with
an embedded proof graph.

Native issuance and credential sizing need no receipt or executor. `--execute`
and `--phases` add executor work without proving. `--prove N` alone enables real
proof generation and the answer-size/verifier split samples. Without a receipt,
`verifier_split_status` is `not_measured_no_receipt` and `proofs` is absent; it is
`measured` when proof samples are present. A rejected configuration retains its
issuance and credential-size samples. No receipt or answer size is estimated.

The sweep uses a fresh in-memory `CountingNonces` store for each proof sample;
request timing includes its `consume` call.

The existing top-level `verifier_signature_check_ns` is a separate native-journal
sample retained for compatibility; it is not part of the new verifier split.
Existing paper keys in `zkexec` describe executor phases and keep their meanings;
no host timer is substituted for them. `--phases` continues to measure journal
construction, without isolating the guest's final commit.

For an already retained answer, `paper_verify REQUEST.json ANSWER.json` reads the
independent expected request and verifies the answer using the same stages, with
no proof generation or executor. `paper_verify --size-only ANSWER.json` measures
the complete carrier without checking a receipt and emits `verified: false` with
no timing fields. The sibling emits `sparq.zk-paper-verification.v2` records with the bincode
companion size; earlier sibling v1 records contain only JSON sizes.
`presentation_received_bytes` counts the actual file bytes, including whitespace;
`presentation_encoded_bytes` counts minified serde_json number-array re-encoding.
`presentation_bincode_bytes` counts the complete companion encoding. Its fresh in-memory
single-use nonce store is local to one invocation, not persistent verifier state.
The ordinary sweep still emits one record per configuration.

Peak memory belongs to the campaign's per-configuration `/usr/bin/time -v` process
boundary. The host records no RSS samples. The guest and accepted image are
unchanged by these host measurements. This remains experimental and externally
unaudited.
