# sparq-vc

Experimental W3C [Data Integrity](https://www.w3.org/TR/vc-data-integrity/) `eddsa-rdfc-2022`
sign/verify over RDF triples: Ed25519 over the [RDFC-1.0](https://www.w3.org/TR/rdf-canon/)
canonical form, resolving `did:key` (`did:web` behind the `did-web` feature). Opt-in, `publish = false`. [OPUS-5.5]

## 🚀 Quickstart

```rust
use sparq_vc::{sign, verify, ProofConfig, SigningKey, did::DidKeyResolver};
let key = SigningKey::generate();
let did = key.did_key();
let vm = format!("{did}#{}", did.strip_prefix("did:key:").unwrap());
let proof = sign(&triples, &key, &ProofConfig::new(vm))?; // triples: &[oxrdf::Triple]
verify(&triples, &proof, &DidKeyResolver)?;
```

## ✨ Features

- **Status:** research-grade, not externally audited; only a typed subset of proof options is supported.
- A passing `verify` shows integrity under the resolved key — **not** issuer key authorization,
  freshness, credential status, confidentiality or selective disclosure. Those checks are the caller's.

## 📚 Learn more

Proof options, validation rules, compatibility notes and boundaries: [`skills/verifiable-credentials/SKILL.md`](../../skills/verifiable-credentials/SKILL.md) and the crate rustdoc (`cargo doc -p sparq-vc --open`).

## License

MIT.
