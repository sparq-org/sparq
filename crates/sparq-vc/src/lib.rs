//! # sparq-vc — W3C Data Integrity (`eddsa-rdfc-2022`) over RDFC-1.0
//!
//! The **vetted, standards-interop** half of verifiable RDF, and the complement
//! to sparq's ZK estate. Where [`sparq-zk`](https://docs.rs/sparq-zk)'s
//! Schnorr-over-Baby-JubJub signatures are deliberately *non-interoperable*
//! (they live in-circuit for the SNARK pipeline), this crate implements the
//! **interoperable** [W3C Data Integrity] `eddsa-rdfc-2022` cryptosuite: standard
//! **Ed25519** (RFC 8032) signatures over the [RDFC-1.0] canonical form sparq
//! already has in [`sparq_canon`].
//!
//! [W3C Data Integrity]: https://www.w3.org/TR/vc-data-integrity/
//! [RDFC-1.0]: https://www.w3.org/TR/rdf-canon/
//!
//! ## What it does — and what it deliberately does NOT
//!
//! `eddsa-rdfc-2022` is **authenticity + integrity + non-repudiation** only:
//!
//! - **VERIFY** that an RDF graph/dataset carries a valid [`DataIntegrityProof`]
//!   ([`verify`]) — i.e. that *this signer asserted exactly these triples,
//!   unmodified* — resolving the proof's `verificationMethod` `did:key`/`did:web`
//!   to an Ed25519 key.
//! - **SIGN** a named graph, a whole store, or a materialized SELECT result-set
//!   ([`sign`]), producing a **portable "this endpoint asserted this answer"
//!   proof** any W3C-conformant verifier can check.
//!
//! It is the **trust-the-signer** model — the ~90% case where the consumer
//! trusts the signing key but needs tamper-evidence and non-repudiation (audit
//! trails, data marketplaces, journalistic provenance, agent-to-agent exchange).
//! "Prove this came from `data.gov` unmodified" needs no SNARK.
//!
//! **Honest boundary (load-bearing):** this is **NOT confidentiality**, **NOT
//! zero-knowledge**, and **NOT selective disclosure**. It reveals the full signed
//! content to the verifier; it proves *who signed* and *that nothing changed*,
//! nothing more. Unlinkable / selective-disclosure presentation is the ZK
//! estate's job (or a later `bbs-2023` phase). This crate makes **no** privacy
//! claim.
//!
//! ## The canonical unit — single-sourced through `sparq-canon`
//!
//! `eddsa-rdfc-2022` signs the SHA-256 hash of the [RDFC-1.0] canonical N-Quads
//! of the document. This crate reuses [`sparq_canon`]'s single RDFC-1.0 bridge
//! for that step, so the canonical seam every sparq consumer shares stays in one
//! place. Two RDF-isomorphic graphs therefore produce the same signature input —
//! blank-node relabelling and triple order do not affect a verify.
//!
//! ## Scope of v1 (honest)
//!
//! - **Cryptosuite:** `eddsa-rdfc-2022` (Ed25519 + RDFC-1.0 + SHA-256). The
//!   `bbs-2023` selective-disclosure suite is out of scope (it is the ZK estate's
//!   selective-disclosure story); a later phase may add it behind its own feature.
//! - **Inputs are RDF datasets.** This crate operates over the **RDF dataset**
//!   form of a credential and its proof config — exactly the form RDFC-1.0
//!   canonicalizes and the proof binds to. Transforming a JSON-LD credential to
//!   RDF (context expansion) is the caller's responsibility (e.g. via the
//!   engine's JSON-LD parser); doing it inside this crate would force a JSON-LD
//!   context processor (and remote-context fetching) onto the lean build. See
//!   [`ProofConfig`] for how the proof options are supplied.
//! - **Proof-config RDF matches the W3C vectors.** [OPUS-5.5] `created` maps to
//!   `dcterms:created` and `cryptosuite` is typed `sec:cryptosuiteString`, as in
//!   the published [vc-di-eddsa test vectors]; the published `proofValue` is
//!   regression-tested. Proofs from earlier releases (`sec:created`, plain
//!   `cryptosuite` literal) no longer verify — no legacy fallback; re-sign them.
//! - **Proof options are validated first.** [OPUS-5.5] zkp-14.3: [`sign`],
//!   [`verify`] and the graph wrappers run [`ProofConfig::validate`] before any
//!   graph materialization, canonicalization, signing or DID resolution, failing
//!   with [`VcError::InvalidProofOption`]: `verificationMethod` must be an
//!   absolute IRI, `proofPurpose` one of [`SUPPORTED_PURPOSE_TERMS`] or an
//!   absolute IRI (hashed verbatim), and `created` an XSD 1.1 `xsd:dateTime`
//!   (hashed exactly as given). A config that passes and uses a compact purpose
//!   term hashes exactly as before, so its signature bytes are unchanged.
//!   **Incompatibility:** earlier releases appended *any* purpose to `sec:`, so
//!   proofs they made with an absolute-IRI purpose no longer verify, and
//!   other bare purpose terms are now rejected. This is lexical validation only.
//! - **Not checked by this signature-only API:** issuer/controller key
//!   authorization, the expected purpose, `domain`, `challenge` or `created`
//!   window, and credential status — enforce them on [`VerifiedProof::config`].
//!   Only [`ProofConfig`]'s typed fields are representable; a caller mapping a
//!   JSON-LD `proof` node must itself reject proof options and `@context`
//!   mappings outside that subset.
//!
//! [vc-di-eddsa test vectors]: https://www.w3.org/TR/vc-di-eddsa/#test-vectors
//! - **DID methods:** `did:key` (offline, self-certifying) by default; `did:web`
//!   (document-fetched, host-rooted) behind the opt-in `did-web` feature, over a
//!   pluggable `did::DidDocumentFetcher` (a `did-web`-feature item) so this crate
//!   ships no HTTP client.
//!
//! ## Opt-in by construction
//!
//! This is a **standalone, opt-in** crate. Nothing in sparq's default build or
//! the wasm artifact depends on it — `sparq-core`/`sparq-engine` stay lean. Pull
//! it in explicitly only when you need W3C Data Integrity verify/sign.
//!
//! [OPUS-4.8] sq-ylbrq (issue #908) — written while Fable is unavailable; flag for
//! re-review when Fable returns.
//!
//! ## Quickstart
//!
//! ```
//! use sparq_vc::{sign, verify, ProofConfig, SigningKey};
//! use oxrdf::{Triple, NamedNode, NamedOrBlankNode, Term, Literal};
//!
//! // A signer key (here generated; in practice loaded from a secret store).
//! let key = SigningKey::generate();
//! let did = key.did_key();               // did:key:z6Mk… (Ed25519)
//!
//! // The graph to attest.
//! let triples = vec![Triple::new(
//!     NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
//!     NamedNode::new("http://ex/p").unwrap(),
//!     Term::Literal(Literal::new_simple_literal("v")),
//! )];
//!
//! // Sign → a DataIntegrityProof bound to the RDFC-1.0 canonical form.
//! let cfg = ProofConfig::new(format!("{did}#{}", did.strip_prefix("did:key:").unwrap()));
//! let proof = sign(&triples, &key, &cfg).unwrap();
//!
//! // Verify with the offline did:key resolver — checks the signature AND that the
//! // resolved key matches the proof's verificationMethod.
//! let resolver = sparq_vc::did::DidKeyResolver;
//! let verified = verify(&triples, &proof, &resolver).unwrap();
//! assert_eq!(verified.verification_method, proof.verification_method());
//! ```

#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod did;
mod proof_options;
mod suite;

pub use proof_options::{ProofOptionError, SUPPORTED_PURPOSE_TERMS};
pub use suite::{
    CRYPTOSUITE, DataIntegrityProof, PROOF_TYPE, ProofConfig, SigningKey, VcError, VerifiedProof,
    VerifyingKey, sign, sign_graph, verify, verify_graph,
};
