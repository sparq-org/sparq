---
name: verifiable-credentials
description: Verify and sign RDF graphs with W3C Data Integrity (eddsa-rdfc-2022 — standard Ed25519 over RDFC-1.0 + SHA-256) and resolve did:key/did:web to a verification key, with the opt-in sparq-vc crate. Use when you trust the SIGNER's key but need tamper-evidence / non-repudiation over an RDF answer (audit trails, data marketplaces, journalistic provenance, agent-to-agent exchange) — the standards-interop, vetted-crypto complement to the in-circuit ZK estate (sparq-zk). Covers sign/verify (sign_graph/verify_graph), ProofConfig binding, DidKeyResolver, and the pluggable did:web DidDocumentFetcher.
license: MIT
metadata:
  version: "0.1.0"
  homepage: https://github.com/sparq-org/sparq
---

# sparq verifiable-credentials (Data Integrity)

**`sparq-vc`** implements the W3C [Data Integrity](https://www.w3.org/TR/vc-data-integrity/)
[`eddsa-rdfc-2022`](https://www.w3.org/TR/vc-di-eddsa/) cryptosuite: **standard Ed25519**
(RFC 8032) signatures over the [RDFC-1.0](https://www.w3.org/TR/rdf-canon/) canonical form
sparq already has (`sparq-canon`), with `did:key`/`did:web` resolution.

This is the **trust-the-signer** model — the ~90% case where the consumer trusts the
signing key but needs **tamper-evidence and non-repudiation**. It is the
**standards-interoperable, vetted-crypto** complement to the ZK estate (`sparq-zk`),
whose Schnorr-over-Baby-JubJub signatures are deliberately non-interoperable.

> **Honest boundary (load-bearing).** `eddsa-rdfc-2022` is **authenticity + integrity +
> non-repudiation only** — **NOT** confidentiality, **NOT** zero-knowledge, **NOT**
> selective disclosure. It proves *who signed* and *that nothing changed*, and reveals
> the full signed content to the verifier. This surface makes **no privacy claim**.
> Unlinkable / selective-disclosure presentation is the ZK estate's job (or a later
> `bbs-2023` phase). `did:key` is self-certifying (the DID *is* the key) — a stable
> identifier, not a trust anchor; `did:web`'s root of trust is whoever controls the host.

## Prerequisites

`sparq-vc` is an **opt-in, `publish = false`** workspace member — nothing in sparq's
default build depends on it. Add it as a path dependency:

<!-- [OPUS-5] #6132: GENERATED region — do not hand-edit. The canonical copy is the
     `path-dep` anchor in crates/sparq-vc/README.md; regenerate with
     `python3 scripts/gen-doc-inject.py` (docs-quality.yml runs `--check`). -->
<!-- BEGIN-INJECT: crates/sparq-vc/README.md#path-dep -->
```toml
sparq-vc = { path = "crates/sparq-vc" }                       # did:key + verify/sign
sparq-vc = { path = "crates/sparq-vc", features = ["did-web"] } # + did:web resolution
```
<!-- END-INJECT -->

No external toolchain is required (unlike the ZK estate's `nargo`/`bb`): Ed25519 is the
vetted [`ed25519-dalek`](https://crates.io/crates/ed25519-dalek) crate.

## Quickstart — sign then verify a graph

```rust
use sparq_vc::{sign, verify, ProofConfig, SigningKey, did::DidKeyResolver};
use oxrdf::{Triple, NamedNode, NamedOrBlankNode, Term, Literal};

// 1. A signer key. In production load the seed from a secret store, not generate().
let key = SigningKey::generate();
let did = key.did_key();                       // did:key:z6Mk… (standard Ed25519)
let vm  = format!("{did}#{}", did.strip_prefix("did:key:").unwrap());

// 2. The graph to attest.
let triples = vec![Triple::new(
    NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
    NamedNode::new("http://ex/p").unwrap(),
    Term::Literal(Literal::new_simple_literal("v")),
)];

// 3. Sign → a DataIntegrityProof bound to the RDFC-1.0 canonical form. Bind extra
//    proof options (verifier replay defence, audience) via the builder:
let cfg = ProofConfig::new(vm)
    .with_created("2026-06-22T12:00:00Z")
    .with_challenge("nonce-from-verifier");
let proof = sign(&triples, &key, &cfg).unwrap();

// 4. Verify with the offline did:key resolver — checks the Ed25519 signature AND that
//    the resolved key matches the proof's verificationMethod, fail-closed otherwise.
let verified = verify(&triples, &proof, &DidKeyResolver).unwrap();
assert_eq!(verified.verification_method, cfg.verification_method);
```

## Signing a stored graph / store

`sign_graph` / `verify_graph` take a `sparq_core::Graph` directly (materialize a named
graph or a whole store, then attest it):

```rust
use sparq_core::Graph;
use sparq_vc::{sign_graph, verify_graph, ProofConfig, SigningKey, did::DidKeyResolver};

let g = Graph::load_str("<http://ex/s> <http://ex/p> \"v\" .", "ntriples").unwrap();
let key = SigningKey::generate();
let did = key.did_key();
let cfg = ProofConfig::new(format!("{did}#{}", did.strip_prefix("did:key:").unwrap()));
let proof = sign_graph(&g, &key, &cfg).unwrap();
verify_graph(&g, &proof, &DidKeyResolver).unwrap();
```

A SELECT result-set is signed the same way: materialize it as a CONSTRUCT graph (or any
canonical RDF encoding) and `sign` the triples.

## What binds, and what fails closed

The signature covers `SHA-256(canon(proofConfig)) ‖ SHA-256(canon(document))` (proof
config first, per the spec). Consequences you can rely on:

- **Isomorphism-stable.** A blank-node relabelling or triple reorder of the same graph
  canonicalizes identically, so the proof still verifies — order/labels do not matter.
- **Tamper-evident.** Any content change (one literal, one triple) ⇒ `SignatureInvalid`.
- **Config-bound.** The `ProofConfig` (verificationMethod, purpose, created, domain,
  challenge) is hashed in too, so a verifier expecting a different `challenge`/`domain`
  than was signed also gets `SignatureInvalid` — replay / audience binding.
- **Key-bound.** A signature from a key other than the one the `verificationMethod`
  resolves to fails closed.

## DID resolution

- **`did:key`** (`DidKeyResolver`, default) — offline, self-certifying. The Ed25519 key
  is encoded in the DID (`z`-base58btc over the `0xed01` multicodec). `did_key_for(&vk)`
  mints it; the resolver decodes it. **Only standard Ed25519 is accepted** — a published
  W3C `did:key` example round-trips byte-identically (regression-tested).
- **`did:web`** (`DidWebResolver`, `did-web` feature) — document-fetched, host-rooted.
  Maps the DID to its `https://…/did.json` URL and reads a verification method's
  `publicKeyMultibase`. The HTTP fetch is a **pluggable `DidDocumentFetcher`** — this
  crate ships no HTTP client, so it forces no `reqwest`/`ureq` on anyone and stays
  offline-testable (an in-memory map is a valid fetcher).

> **Not** to be confused with `sparq-trust::did`, which resolves to a *sparq-private
> Baby-JubJub* key for the ZK scheme and rejects the standard Ed25519 `z6Mk…` form. That
> resolver is for the custom ZK estate; **this** one is the W3C-interoperable resolver.

## Scope (v1, honest)

- **Cryptosuite:** `eddsa-rdfc-2022` only. `bbs-2023` (selective disclosure) is out of
  scope — it is the ZK estate's selective-disclosure story.
- **Inputs are RDF datasets.** `sparq-vc` operates over the RDF-dataset form of a
  credential + its proof config (exactly what RDFC-1.0 canonicalizes). Transforming a
  JSON-LD credential to RDF (context expansion) is the caller's job — e.g. via the
  engine's JSON-LD parser — to keep the lean build free of a JSON-LD context processor.

## See also

- [`zk-query-proofs`](../zk-query-proofs/SKILL.md) — the zero-knowledge / selective-disclosure complement (in-circuit, `nargo`/`bb`).
- [`rdf-canon`](../rdf-canon/SKILL.md) — the RDFC-1.0 canonical unit the proof binds to.
- [`prov-lineage`](../prov-lineage/SKILL.md) — W3C PROV-O lineage for derived graphs.
