# sparq-zk-query-wasm

Research grade and not externally audited. This `publish = false` crate adapts the
query compiler and Merkle suite to JSON for the Node package. It shares lowering,
capacity selection, interpretation, code generation, request RDF, public-input
reconstruction, key derivation and issuance with the native crates.

## 🚀 Usage

The package build generates bindings from source; no WebAssembly binary is tracked.
`hostCall` handles JSON preparation operations. `verifyWith` takes JSON plus
synchronous proof, atomic challenge-store and clock callbacks. Rust checks the
stored request, reconstructs the statement, checks the catalogue key, calls the
proof verifier, checks validity again and consumes the original challenge last.
The callbacks carry proof-system and storage operations; they do not evaluate queries.

The native reference bridge accepts JSON on standard input:
`cargo run -p sparq-zk-query-wasm --example host_json`.
Witness outputs contain private credential material.

The Node facade defaults to the pinned local native witness/prover backend.
Its optional WebAssembly backend proves admitted small profiles; verification
runs in Node. Native executables use explicit locations, version and digest checks,
private copies and private witness files. Catalogue keys and programs are addressed
by the Rust-derived plan hash and checked against pinned digests. Unknown plans
and mismatched keys refuse.

## ✨ Features

Portable preparation and verification ordering share the native method's host logic.

## 📚 Contract

The package's [public-input layout](../../js/zk-query/README.md#public-input-layout)
is generated from the query crate's declarations and audited ABI; its drift test
fails when the documentation diverges.

See the [package API](../../js/zk-query/README.md) and
[usage skill](../../skills/zk-query-javascript/SKILL.md).
Completeness is relative to the presented signed roots. The holder chooses those
roots unless a verifier-agreed commitment applies. Replay protection lives in
the caller's store. Enabled credential-envelope policies remain refused by the
current query primitive.

## License

MIT.
