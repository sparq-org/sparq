# sparq-vc

W3C **[Data Integrity]** (`eddsa-rdfc-2022`) **verify + sign** over the [RDFC-1.0]
canonical form sparq already has — the **vetted, standards-interop** half of
verifiable RDF, and the complement to the ZK estate.

Where [`sparq-zk`](../sparq-zk)'s Schnorr-over-Baby-JubJub signatures are
deliberately *non-interoperable* (they live in-circuit for the SNARK pipeline),
this crate implements the **interoperable** cryptosuite: standard **Ed25519**
(RFC 8032) over RDFC-1.0 + SHA-256, with `did:key`/`did:web` resolution.

[Data Integrity]: https://www.w3.org/TR/vc-data-integrity/
[RDFC-1.0]: https://www.w3.org/TR/rdf-canon/

> Model: Opus 4.8 (Fable unavailable — flag for re-review when Fable returns).
> Bead sq-ylbrq / issue #908.

## 🚀 Quickstart

```rust
use sparq_vc::{sign, verify, ProofConfig, SigningKey, did::DidKeyResolver};
use oxrdf::{Triple, NamedNode, NamedOrBlankNode, Term, Literal};

let key = SigningKey::generate();          // vetted Ed25519 keypair
let did = key.did_key();                   // did:key:z6Mk… (standard Ed25519)

let triples = vec![Triple::new(
    NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
    NamedNode::new("http://ex/p").unwrap(),
    Term::Literal(Literal::new_simple_literal("v")),
)];

// Sign → a DataIntegrityProof bound to the RDFC-1.0 canonical form.
let vm = format!("{did}#{}", did.strip_prefix("did:key:").unwrap());
let proof = sign(&triples, &key, &ProofConfig::new(vm)).unwrap();

// Verify with the offline did:key resolver.
verify(&triples, &proof, &DidKeyResolver).unwrap();
```

## ✨ Features

- **Verify** an `eddsa-rdfc-2022` `DataIntegrityProof` over a slice of triples or a
  `sparq_core::Graph` (`verify` / `verify_graph`), resolving the proof's
  `verificationMethod` `did:key`/`did:web` to an Ed25519 key.
- **Sign** a named graph, a store, or a materialized SELECT result-set
  (`sign` / `sign_graph`) → a portable *"this endpoint asserted this answer"*
  proof any W3C-conformant verifier can check.
- **Isomorphism-stable + tamper-evident.** The signature binds to the RDFC-1.0
  canonical N-Quads, so a blank-node relabelling / reorder verifies identically and
  any content change fails closed (`SignatureInvalid`). The `ProofConfig`
  (verificationMethod, purpose, time, domain, challenge) is hashed in too, so a
  swapped option also fails closed — replay/non-repudiation binding.
- **`did:key`** offline self-certifying resolution (default); **`did:web`**
  document-fetched resolution behind the `did-web` feature over a pluggable
  `DidDocumentFetcher` (this crate ships no HTTP client).

### Honest boundary

`eddsa-rdfc-2022` is **authenticity + integrity + non-repudiation only** — **NOT**
confidentiality, **NOT** zero-knowledge, **NOT** selective disclosure. It proves
*who signed* and *that nothing changed*; it reveals the full signed content to the
verifier. This crate makes **no privacy claim**. Unlinkable / selective-disclosure
presentation is the ZK estate's job (or a later `bbs-2023` phase).

`did:key` is self-certifying (the DID *is* the key) — a stable identifier, **not** a
trust anchor. `did:web`'s root of trust is whoever controls the host + its TLS.

### Scope of v1

Operates over the **RDF dataset** form of a credential + its proof config (exactly
what RDFC-1.0 canonicalizes). Transforming a JSON-LD credential to RDF (context
expansion) is the caller's job — doing it here would force a JSON-LD context
processor onto the lean build. `bbs-2023` is out of scope.

## Opt-in by construction

Nothing in sparq's default build or the wasm artifact depends on this crate —
`sparq-core`/`sparq-engine` stay lean. Pull it in explicitly only when you need W3C
Data Integrity verify/sign. It reuses [`sparq-canon`](../sparq-canon)'s single
RDFC-1.0 bridge, so the canonical seam stays single-sourced. `publish = false`.

<!-- [OPUS-5] #6132: this crate README is the CANONICAL copy of the path-dependency
     stanza. skills/verifiable-credentials/SKILL.md materialises it into a BEGIN-INJECT
     region sourced from `crates/sparq-vc/README.md#path-dep` (scripts/gen-doc-inject.py;
     research/docs-site-single-sourcing-anti-drift.md §5), because neither renderer can
     link to the other — a crate README is read on GitHub/crates.io/docs.rs, a SKILL.md
     by an agent. Keep the anchored block LINK-FREE (an injected repo-relative link
     would resolve from the consumer's directory, not this one), and keep the
     ANCHOR / ANCHOR_END marker lines one-per-line. Do not nest a comment terminator in
     this comment: HTML comments do not nest, so the rest would render as visible text. -->
<!-- ANCHOR: path-dep -->
```toml
sparq-vc = { path = "crates/sparq-vc" }                       # did:key + verify/sign
sparq-vc = { path = "crates/sparq-vc", features = ["did-web"] } # + did:web resolution
```
<!-- ANCHOR_END: path-dep -->

## 📚 Learn more

- [W3C VC Data Integrity](https://www.w3.org/TR/vc-data-integrity/) ·
  [eddsa-rdfc-2022](https://www.w3.org/TR/vc-di-eddsa/) ·
  [did:key](https://w3c-ccg.github.io/did-method-key/)
- [`skills/verifiable-credentials/SKILL.md`](../../skills/verifiable-credentials/SKILL.md)
  — how to use this surface.
- [`sparq-canon`](../sparq-canon) — the RDFC-1.0 canonical unit the proof binds to.

## License

MIT.
