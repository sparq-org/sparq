<!-- [OPUS-5.5] zkp-16.2: usage reference for the optional vcq adapter over the exact V3 relation. -->
# vcq adapter for the exact RISC Zero V3 evaluator

`sparq_proved_evaluator::vcq` (detached crate `zk/sparql-evaluator/host`, cargo feature `vcq`,
**off by default**) implements the `sparq-query-protocol` `QueryMethod` trait over the existing
V3 relation. It is experimental and not externally audited (sq-qhy4). It authenticates no
source credential, checks no credential status and no holder key. The research
[method registry](../../../research/vc-query-methods.json) still lists `method:risc0-exact` with
`adapter_available: false`; the adapter's local `Capabilities` declaration is not a registry
entry. The existing V1/V2/V3 host APIs and the engine replay bridge are unchanged.

```sh
cargo test --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator --features vcq
cargo test --manifest-path zk/sparql-evaluator/Cargo.toml -p sparq-proved-evaluator-model \
  --features graph-results --test vcq_request_shape
```

## API

| Item | Purpose |
|---|---|
| `Risc0ExactV3::new(&ArtifactPin, AcceptedGuest)` | Method from an independently approved pin and guest; rejects a guest that does not match the pin |
| `.with_r0vm(PathBuf)` | Local `r0vm` used by `prove` |
| `.with_verifier(Identifier, fn() -> u64)` | Verifier audience and clock used by the trait `verify` |
| `verify_at(request, audience, now_unix, admission, presentation, store)` | Verification with explicitly supplied audience and Unix time |
| `descriptor(&ArtifactPin)` | The exact `MethodDescriptor` a verifier lists in its request |
| `derive_nonce`, `stored_request_digest`, `descriptor_digest`, `statement_digest`, `parameter_digest` | Documented digests below |
| `expected_v3_request` | The V3 request both sides derive from a `StoredRequest` |
| `VcqPresentation` | Descriptor digest plus serialized receipt bytes |
| `V3Output`, `ReleasedResult` | Claim output: result, journaled provenance, commitment, V3 request digest |

`QueryMethod` associated types: `Request = StoredRequest`, `PrivateInputs = v3::PrivateDataset`,
`ChallengeStore = dyn ChallengeStore`. The witness is opaque, not `Clone`, and redacted in `Debug`.

## Descriptor and capabilities

- Descriptor: `urn:sparq:vcq:method:risc0-exact` version 3, parameter set
  `urn:sparq:vcq:params:risc0-exact:v3-default-policy` with `parameter_digest()`,
  `ZkvmGuest { artifact_digest: pin.sha256, image_id }` (each image-id word little-endian), backend
  `urn:sparq:vcq:backend:risc0-zkvm:3.0.6:succinct`.
- Six whole tuples: `SelectBag`, `AskBoolean`, `GraphRdfc10` (CONSTRUCT only), each under
  `VerifierAgreedAnchor` and `HolderDeclared`; `ExactBounded`; profile
  `urn:sparq:vcq:dialect:sparq-sparql11-graph-results:v3` / `urn:sparq:vcq:fragment:v3-static-admission`;
  `ExactSourceCatalog`; source evidence `None`, status `NotRequested`, holder `BearerAccepted`;
  mapping `urn:sparq:vcq:map:exact-source-bytes` (V3 via the descriptor version); linking
  `urn:sparq:vcq:link:committed-dataset-evaluation`.
- Enforcers: mapping, linking and query by the guest relation; the anchor by the public host
  check `urn:sparq:vcq:host:anchor-equality`, owed only under anchor authority. Authenticity,
  status and holder binding are `Absent`, so a claim reports them `NotEstablished`.
- Challenge: owner `Method`, `ConsumeOnSuccess`.
- Fixed default V3 policy only: dataset bytes 65,536, source quads 256, rows 4,096, named graphs
  16; canonicalization quads 32,768, input and output bytes 1 MiB, HNDQ calls 64, permutation
  steps 1,048,576; query bytes 8,192. Ceilings: 4,096 released rows, 16 MiB presentation bytes.

Rejected: SELECT sequences (outer ORDER BY), DESCRIBE, an explicit base IRI, set/true-only
contracts, and any authenticated, status or holder-binding request.

## Encodings

All digests are SHA-256; `local-struct-v1` is the protocol crate's local encoding.

- `parameter_digest`: `sparq:vcq:risc0-exact:parameters:v3\0`, then big-endian `u32` V3 version,
  the four dataset bounds, the five canonicalization bounds (declaration order), one DESCRIBE byte
  `0x01`, and the query bound as big-endian `u64`.
- `derive_nonce`: `sparq:vcq:risc0-exact:v3-nonce:local-struct-v1\0` ‖
  SHA-256(stored-request bytes) ‖ SHA-256(descriptor bytes). Every stored-request field,
  including the original challenge, audience and window, reaches the journaled V3 request digest.
- `statement_digest`: `sparq:vcq:risc0-exact:statement:v3\0` ‖ stored-request digest ‖
  descriptor digest ‖ SHA-256(exact verified receipt journal bytes).
- The V3 request digest keeps its existing pinned JSON format. Receipt JSON inside
  `VcqPresentation` is transport only and is never hashed as protocol data.

## Verification order

`prepare` and `verify` both recompute admission (a caller admission that differs is rejected)
and check the parsed query with the host-only `v3::query_shape` helper, so a DESCRIBE query
labelled CONSTRUCT, a SELECT labelled ASK or an ordered SELECT fails before any proof work or
challenge use. `verify` then checks audience, `not_before <= now < not_after`, the descriptor
digest, and the presentation-byte bound before decoding the receipt. The existing checked V3
path verifies the Succinct receipt with dev mode disabled, decodes the journal and binds the
request. The adapter then checks the anchor or provenance and the exact result form (bag rows of
unique variables and uniform arity, sorted, within the row bound; ASK boolean; canonical
N-Triples lines within the row bound) and builds the claim. Only after all of that does the V3
nonce call reach a private bridge. The bridge accepts one call with the expected derived nonce
and consumes the ORIGINAL challenge once through the shared store. Replay and store failure are
classified from the typed store outcome.

## Evidence status

Native tests (`host/tests/vcq_adapter.rs`, unit tests in `host/src/vcq.rs`,
`model/tests/vcq_request_shape.rs`) create no proof. The only receipt they use is a fake one,
which must be rejected without touching the store. No genuine receipt has been produced or
verified through this adapter yet.
