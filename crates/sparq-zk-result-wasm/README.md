# sparq-zk-result-wasm

Private workspace host adapter for successful-result proofs. Research grade and
not externally audited. It concerns sparq commitment-signed credentials, not
standard VCs. There is no answer or wallet completeness, absence proof or holder
identity. Issuer identities, capacities and result size are observable.

## 🚀 Usage

Build the library for wasm32 and generate the Node bindings with wasm-bindgen.
Consumers use the prebuilt [Node package](../../js/zk-result/README.md).

The JSON functions are `issuerKeygen()`, `testIssuerKeygen(seedHex)`,
`issue(issuerJson, graphNQuads, statusRefJson)`, `testIssue(inputJson)`,
`prepare(queryText, credentialsJson, policyJson, nonceHex, optionsJson)`,
`reconstructPublicInputs(expectedQueryText, presentationJson, policyJson, nonceHex)`
and `canonicalVkHash(member)`. Successful calls return JSON strings, except the
key-hash function, which returns a digest string. Errors throw strings in JS.
`issuerKeygen()` returns `{ secretKeyHex, publicKey }` from platform CSPRNG
entropy. The caller must store and protect the secret; the package never persists
it. Keys remain research grade, not externally audited. `testIssuerKeygen`
retains its deterministic `{ seedHex, publicKey }` test-only format. `issue`
accepts either format, checks the public key, and rejects mixed fields.

The Node API documents `issue({ issuer, graphNQuads, status })` and retains its
older positional form. Both call the CSPRNG-backed host `issue`; neither can
select deterministic issuance. `testIssue` takes JSON
`{ issuer, graphNQuads, status, seedHex }`, accepts only a seed-based test issuer
and refuses production secrets, including mixed objects. A predictable salt
lets anyone who can guess the graph test a guess against the commitment.
Both seeds are hexadecimal unsigned `u64` values with optional `0x`; case and
leading zeros do not change their values. The Rust salt derivation uses the
issuance seed, canonical issuer public key and exact graph UTF-8 bytes, including
whitespace, under a separate SHA-256 domain and the existing hash-to-field helper.
Status does not affect the salt; it remains signature-bound. See the
[exact derivation](../../js/zk-result/README.md#reproducible-test-issuance).
Repeated inputs give byte-identical credentials usable by the ordinary relation.

Pass `{}` for default preparation options. Optional `hideInTwo`, `hideInU64` and
`firstSuccess` booleans default to false; `maxSearchSteps` defaults to the shared
planner budget. These choices cannot relax the verifier policy.

## ✨ Scope

The relation lives in `sparq-zk-compose` with `successful-results`. The native
subprocess driver and file-backed nonce store are excluded on wasm32. Preparation
returns a private circuit ABI map, canonical member and public skeleton; sensitive
witness maps must remain in the prover process. Reconstruction shares the native
verifier's statement checks and performs no cryptographic proof check itself.
Its `report` is candidate evidence only. The Node verifier releases it after
proof acceptance and synchronous nonce consumption: `rule` holds the receiver's
query and selected member, `issuers` resolves rebuilt public coordinates to
canonical trusted key hex in signature-slot order (including repeats), `subject`
is empty, `fresh` holds the consumed canonical nonce and `complete` is false.
Every slot authenticates a private credential, but a slot may be padding or unused
by the solutions. This does not identify which issuer signed which fact or count
distinct credentials. There is no holder binding or proof that no solution is
missing. Refusal returns no report. The
[public-input layout](../../js/zk-result/README.md#rebuilt-public-input-layout)
is checked against reconstruction and every bundled ABI by the offline suite.

Target-specific dependency features enable the documented JavaScript entropy
backends for the locked getrandom generations. The workspace target configuration
selects `getrandom_backend="wasm_js"` only for `wasm32-unknown-unknown`. Entropy
comes from the platform CSPRNG, including `crypto.getRandomValues` in Node. Native
features and entropy behavior remain unchanged. No stub entropy is registered.

## 📚 References

- [Successful-result contract](../../skills/zk-query-proofs/references/successful-results.md)
- [Node API](../../skills/zk-result-javascript/SKILL.md)

## License

MIT, as in the workspace.
