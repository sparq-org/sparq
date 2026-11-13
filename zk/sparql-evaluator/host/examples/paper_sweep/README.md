# Host measurement boundaries

`paper_sweep` writes `sparq.zk-paper-sweep.v2` JSONL. Records without `schema`
are the legacy schema; readers also accept `sparq.zk-paper-sweep.v1`. The audit
loader validates the additive fields for v2 and keeps reading legacy archives.
All wall-clock values are local samples, not canonical evidence.

| Field | Boundary |
| --- | --- |
| `issuer_sign_ns[]` | Existing inclusive issuance boundary: key construction, canonicalization, hashing, preparation, tree work where applicable, primitive signing and witness-form assembly. One entry per credential. |
| `issuer_signature_primitive_ns[]` | Only `SigningKey::sign` on the prepared message. Key construction, signature byte conversion, salt framing, canonicalization, hashing and message preparation are outside. |
| `issuer_merkle_tree_ns[]` | For S4, typed leaf encoding/hashing over parsed canonical quads, leaf sorting, leaf-row allocation and root computation. Canonicalization, parsing, proof-config hashing, signed-message hashing, document reassembly and signing are outside. Entries are null for RDF-C. |
| `credential_bytes[]` | Existing document plus proof-configuration UTF-8 bytes; unchanged. |
| `credential_encoded_bytes[]` | Compact `serde_json` encoding of the complete `SignedCredential`, before revealed signatures are removed. Includes escaped document and proof configuration, raw proofValue bytes as a JSON byte array, Merkle salt where applicable, field names and framing. |
| `proofs[].presentation_bytes` | Existing receipt JSON bytes plus raw detached-signature bytes; unchanged. |
| `proofs[].presentation_encoded_bytes` | Compact JSON encoding of `AnswerPresentation`: the complete nested `Presentation` (receipt, journal and receipt metadata) and detached signatures in journal order, or null in hidden mode. |
| `proofs[].verify_ns` | Aggregate production verification of that decoded envelope, including timing bookkeeping. Encoding, decoding and native-oracle comparison are outside. |
| `proofs[].verifier_receipt_ns` | Succinct-kind check, receipt verification with dev mode disabled and independently approved image identity, then journal decoding. |
| `proofs[].verifier_signature_ns` | Strict revealed-signature checks, once, after journal binding. Null in hidden mode. |
| `proofs[].verifier_request_ns` | Sum of disjoint intervals for entry-point mode guard, query/profile and request validation, journal binding, then nonce consumption. Preserves the original ordering around the other stages. |
| `proofs[].verifier_stages_ns` | Sum of the three work timers from that one verification. Timers never nest; timer bookkeeping and function return overhead remain in the aggregate only. |

`credential_encoding` and `presentation_encoding` identify these local encodings.
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

The existing top-level `verifier_signature_check_ns` is a separate native-journal
sample retained for compatibility; it is not part of the new verifier split.
Existing paper keys in `zkexec` describe executor phases and keep their meanings;
no host timer is substituted for them. `--phases` continues to measure journal
construction, without isolating the guest's final commit.

For an already retained answer, `paper_verify REQUEST.json ANSWER.json` reads the
independent expected request and verifies the answer using the same stages, with
no proof generation or executor. `paper_verify --size-only ANSWER.json` measures
the complete carrier without checking a receipt and emits `verified: false` with
no timing fields. The sibling emits `sparq.zk-paper-verification.v1` records.
`presentation_received_bytes` counts the actual file bytes, including whitespace;
`presentation_encoded_bytes` counts compact re-encoding. Its fresh in-memory
single-use nonce store is local to one invocation, not persistent verifier state.
The ordinary sweep still emits one record per configuration.

Peak memory belongs to the campaign's per-configuration `/usr/bin/time -v` process
boundary. The host records no RSS samples. The guest and accepted image are
unchanged by these host measurements. This remains experimental and externally
unaudited.
