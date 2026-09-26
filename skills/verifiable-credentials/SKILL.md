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

```toml
sparq-vc = { path = "crates/sparq-vc" }                          # did:key + verify/sign
sparq-vc = { path = "crates/sparq-vc", features = ["did-web"] }  # + did:web resolution
```

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
- **W3C proof-config mapping (breaking).** [OPUS-5.5] `created` is hashed as
  `dcterms:created` and `cryptosuite` as a `sec:cryptosuiteString`-typed literal, per
  the published [vc-di-eddsa test vectors](https://www.w3.org/TR/vc-di-eddsa/#test-vectors)
  (the published `proofValue` verifies in `crates/sparq-vc/tests/w3c_eddsa_rdfc.rs`).
  Proofs signed by earlier releases (`sec:created`, plain `cryptosuite` literal) **no
  longer verify** — there is no legacy fallback; re-sign them.
- **Typed subset only.** `ProofConfig` carries type, cryptosuite, verificationMethod,
  proofPurpose, created, domain and challenge. Unknown JSON-LD proof options and
  `@context` mappings are not representable, so a caller mapping a JSON-LD `proof`
  node must reject what this subset cannot carry — nothing here validates JSON.

## Proof-option validation (runs first)

[OPUS-5.5] `sign`, `sign_graph`, `verify` and `verify_graph` all run
`ProofConfig::validate` **before** graph materialization, RDFC-1.0 canonicalization,
signing or DID resolution, and fail with `VcError::InvalidProofOption(ProofOptionError)`:

- **`verification_method`** — must be an absolute IRI (oxrdf/RFC 3987); hashed verbatim.
- **`proof_purpose`** — one of `SUPPORTED_PURPOSE_TERMS`, hashed as the `@id` the
  `proofPurpose` scoped context of the [VC v2 `@context`](https://www.w3.org/ns/credentials/v2)
  gives it; or, if it contains `:`, an absolute IRI hashed verbatim (never appended
  to `sec:`). Other bare terms are rejected; compact IRIs like `sec:assertionMethod`
  are not expanded — pass the full IRI from this table, which hashes like its term:

  | Term | Hashed IRI (`https://w3id.org/security#…`) |
  |---|---|
  | `assertionMethod` | `assertionMethod` |
  | `authentication` | `authenticationMethod` |
  | `capabilityDelegation` | `capabilityDelegationMethod` |
  | `capabilityInvocation` | `capabilityInvocationMethod` |
  | `keyAgreement` | `keyAgreementMethod` |

  **Breaking:** earlier releases hashed every term as `sec:<term>`. Only
  `assertionMethod` (the published W3C vector's purpose) is unaffected. Proofs made
  with the other four compact terms, or with an absolute-IRI purpose, no longer
  verify — no fallback; re-sign them. Only the `assertionMethod` configuration is
  checked against a published W3C vector; the other four rows are unit-tested
  against the `@context` text, not against published vectors.
- **`created`** — an XSD 1.1 `xsd:dateTime` (vc-di-eddsa §3.3.5 requires rejecting
  invalid values): year `0000` (leap) and negative years, `24:00:00` with a zero
  fraction, any number of fraction digits, optional timezone within `±14:00`, real
  calendar dates. Signed exactly as written — never normalized, no whitespace trimming.
  The Data Integrity core's `dateTimeStamp` (timezone-required) profile is **not** enforced.

```rust
use sparq_vc::{ProofConfig, ProofOptionError};

let vm = "did:key:z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2";
assert!(ProofConfig::new(vm).with_created("-0004-02-29T24:00:00Z").validate().is_ok());
let bad = ProofConfig::new(vm).with_created("2023-02-29T00:00:00Z");
assert!(matches!(bad.validate(), Err(ProofOptionError::Created { .. })));
```

Validation is **lexical only**. A successful `verify` is a signature check: it does
not establish that the key is authorized for the purpose (issuer/controller
authorization), nor that purpose, `domain`, `challenge` or `created` match what you
expect, nor credential status. Check those yourself on `VerifiedProof::config`.

## See also

- [`zk-query-proofs`](../zk-query-proofs/SKILL.md) — the zero-knowledge / selective-disclosure complement (in-circuit, `nargo`/`bb`).
- [`rdf-canon`](../rdf-canon/SKILL.md) — the RDFC-1.0 canonical unit the proof binds to.
- [`prov-lineage`](../prov-lineage/SKILL.md) — W3C PROV-O lineage for derived graphs.
