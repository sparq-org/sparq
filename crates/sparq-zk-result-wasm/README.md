# sparq-zk-result-wasm

Private workspace host adapter for successful-result proofs. Research grade and
not externally audited. It concerns sparq commitment-signed credentials, not
standard VCs. There is no answer or wallet completeness, absence proof or holder
identity. Issuer identities, capacities and result size are observable.

## 🚀 Usage

Build the library for wasm32 and generate the Node bindings with wasm-bindgen.
Consumers use the prebuilt [Node package](../../js/zk-result/README.md).

The JSON functions are `testIssuerKeygen(seedHex)`,
`issue(issuerJson, graphNQuads, statusRefJson)`,
`prepare(queryText, credentialsJson, policyJson, nonceHex, optionsJson)`,
`reconstructPublicInputs(expectedQueryText, presentationJson, policyJson, nonceHex)`
and `canonicalVkHash(member)`. Successful calls return JSON strings, except the
key-hash function, which returns a digest string. Errors throw strings in JS.
The issuer helper is deterministic test issuance only.

Pass `{}` for default preparation options. Optional `hideInTwo`, `hideInU64` and
`firstSuccess` booleans default to false; `maxSearchSteps` defaults to the shared
planner budget. These choices cannot relax the verifier policy.

## ✨ Scope

The relation lives in `sparq-zk-compose` with `successful-results`. The native
subprocess driver and file-backed nonce store are excluded on wasm32. Preparation
returns a private circuit ABI map, canonical member and public skeleton; sensitive
witness maps must remain in the prover process. Reconstruction shares the native
verifier's statement checks and performs no cryptographic proof check itself.

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
