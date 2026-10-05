// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! Off-circuit W3C Verifiable-Credential ingest bridge (sq-9c5e, design
//! `research/zk-configurable-commitment-design.md` §5).
//!
//! # What this is
//! A **host-side, off-circuit** bridge that brings a W3C Data-Integrity VC into
//! sparq's commitment pipeline:
//!
//! ```text
//! VC (issuer-signed under eddsa-rdfc-2022 / ecdsa-rdfc-2019)
//!    │
//!    ├─ verify the VC's Data-Integrity proof OFF-circuit (host), under the named
//!    │   cryptosuite, against the issuer's published key.            [fail closed]
//!    ├─ RDFC10-canonicalise the credential graph (sparq already does — `canon`).
//!    ├─ commit under the sparq commitment pipeline (`commit::commit_triples`).
//!    └─ record provenance:
//!         zk:cryptosuite       = the sparq scheme   (what the query proof checks)
//!         zk:sourceCryptosuite = the VC's W3C suite  (provenance / back-compat)
//! ```
//!
//! It exists so the maintainer's requested **per-cryptosuite performance
//! comparison** (cost of ingest+re-commit per W3C suite) and the
//! **backwards-compatibility discussion** (which suites round-trip) can happen on
//! real data, WITHOUT claiming sparq verifies the VC's cryptographic proof inside
//! a query circuit.
//!
//! # The two distinct signatures (design §5.1) — DO NOT conflate
//! 1. **The VC's own Data-Integrity proof** — Ed25519 / ECDSA under a W3C
//!    cryptosuite. Checked HERE, off-circuit, at ingest **only**.
//! 2. **sparq's commitment signature** — `Poseidon2SchnorrV1` over `C(G)`
//!    ([`crate::sig`]) — what the in-circuit / verifier-side query proof binds to.
//!
//! These are **not the same** and the query proof does **NOT** re-verify the VC's
//! Ed25519/ECDSA proof in-circuit (§5.3). `zk:sourceCryptosuite` is **provenance,
//! not a re-verifiable in-proof property**.
//!
//! # Honest scope boundary (load-bearing)
//! - **In scope:** the off-circuit DI verification for the *whole-credential*
//!   `rdfc` suites `eddsa-rdfc-2022` (Ed25519) + `ecdsa-rdfc-2019` (ECDSA, **both**
//!   published profiles — P-256/SHA-256 and P-384/SHA-384), then re-commit +
//!   record provenance.
//! - **NOT in scope:** `bbs-2023` / `ecdsa-sd-2023` selective disclosure (the
//!   natural per-leaf match, but a real BBS verifier is not in-repo). Those suites
//!   are still rejected by every function in THIS module; they are handled by the
//!   DELEGATING seam [`crate::vc_bridge_sd`] (sq-u5y1f, design §5.3), which asserts
//!   no selective-disclosure soundness of its own. Also NOT in scope: **in-circuit**
//!   VC-proof verification (explicitly excluded — would be an overclaim). The host
//!   **does not fetch** the issuer key from a `did:`/URL — the caller supplies the
//!   resolved key bytes.
//! - **This module is RDF-native**: it operates over the credential's RDF graph —
//!   its canonical N-Quads, the form the DI suites hash. Turning a JSON-LD VC
//!   document into that RDF is the ADDITIVE, layered-on-top job of
//!   [`crate::vc_bridge_json`] (sq-txg1y); nothing here parses JSON.
//!
//! # The two `ecdsa-rdfc-2019` curve profiles (sq-txg1y) [OPUS-5]
//! W3C *Data Integrity ECDSA Cryptosuites v1.0* gives the **one** cryptosuite
//! token `ecdsa-rdfc-2019` **two** profiles, selected by the issuer key's curve —
//! and the hash changes with it (§3.1 *ECDSA Algorithms*):
//!
//! | issuer key | DI hash | `hashData` width | signature |
//! | --- | --- | --- | --- |
//! | P-256 (SEC1 33B/65B) | SHA-256 | 32+32 = 64 B | 64 B `r‖s` |
//! | P-384 (SEC1 49B/97B) | SHA-384 | 48+48 = 96 B | 96 B `r‖s` |
//!
//! Because the token does not name the curve, the profile is resolved from the
//! **key** ([`EcdsaProfile::from_sec1_key`]) BEFORE the hash is taken — a P-384
//! key hashed under SHA-256 would produce a `hashData` no conforming issuer ever
//! signed, so the two must not be mixed. A key length matching neither profile is
//! [`VcBridgeError::MalformedPublicKey`]; a length matching a curve OUTSIDE the
//! suite's two profiles (P-521) is [`VcBridgeError::UnsupportedKeyCurve`].
//!
//! # Soundness posture
//! NOT externally audited (sq-qhy4). This module asserts **no** in-circuit or
//! query-soundness property; it is an *ingest-time* host verifier of the DI
//! hashing. A `false`/`Err` from any path is fail-closed — malformed key /
//! signature / cryptosuite bytes reject, never panic.
//!
//! OPT-IN: the whole module is behind the OFF-by-default `vc-bridge` cargo
//! feature, so the default build pulls no Ed25519/ECDSA/SHA-256 dependency.

use crate::commit::{commit_triples, CommitError, GraphCommitment};
use crate::field::Fr;
use crate::registry::RegistryEntry;
use oxrdf::{NamedNode, Triple};
use sha2::{Digest, Sha256, Sha384};

/// The W3C Data-Integrity cryptosuites this bridge verifies off-circuit **itself**.
/// Only the whole-credential `rdfc` suites are in scope (design §5.4); the
/// selective-disclosure suites (`bbs-2023`, `ecdsa-sd-2023`) are delegated to a
/// host verifier through [`crate::vc_bridge_sd::SdCryptosuite`] — a deliberately
/// separate type, so "sparq verified this" and "a host verifier verified this"
/// cannot be confused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcCryptosuite {
    /// `eddsa-rdfc-2022` — Ed25519 over the RDFC10 canonical credential, SHA-256
    /// hash (W3C *Data Integrity EdDSA Cryptosuites v1.0*).
    EddsaRdfc2022,
    /// `ecdsa-rdfc-2019` — ECDSA over the RDFC10 canonical credential (W3C *Data
    /// Integrity ECDSA Cryptosuites v1.0*). The **curve profile is not part of the
    /// token**: a P-256 key selects SHA-256 and a P-384 key selects SHA-384, and
    /// the bridge resolves which from the issuer key ([`EcdsaProfile`]).
    EcdsaRdfc2019,
}

/// The curve profile an [`VcCryptosuite::EcdsaRdfc2019`] proof was created under,
/// resolved from the issuer key because the cryptosuite token does not name it
/// (sq-txg1y). W3C *Data Integrity ECDSA Cryptosuites v1.0* §3.1 publishes exactly
/// these two; the DI hash function changes with the curve, so this MUST be settled
/// before `hashData` is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcdsaProfile {
    /// P-256 keys: SHA-256 DI hash, 64-byte `hashData`, 64-byte `r‖s` signature.
    P256,
    /// P-384 keys: SHA-384 DI hash, 96-byte `hashData`, 96-byte `r‖s` signature.
    P384,
}

impl EcdsaProfile {
    /// Resolve the profile from the SEC1 encoding of the issuer verification key.
    ///
    /// SEC1 point encodings are curve-length-determined, so the byte length alone
    /// identifies the curve: compressed is `1 + ⌈log256 p⌉`, uncompressed is
    /// `1 + 2⌈log256 p⌉`. Fail-closed and total —
    ///
    /// | length | outcome |
    /// | --- | --- |
    /// | 33, 65 | [`EcdsaProfile::P256`] |
    /// | 49, 97 | [`EcdsaProfile::P384`] |
    /// | 67, 133 | [`VcBridgeError::UnsupportedKeyCurve`] (P-521 — a real curve, but not one of the suite's two profiles) |
    /// | anything else | [`VcBridgeError::MalformedPublicKey`] |
    ///
    /// Length is a *necessary* condition, not a sufficient one: it selects which
    /// curve's parser runs, and that parser then rejects a point that is not on
    /// the curve. A `secp256k1` key is indistinguishable from P-256 **by length**
    /// and is caught one step later, when `p256`'s `from_sec1_bytes` finds the
    /// point off the P-256 curve.
    pub fn from_sec1_key(pk_bytes: &[u8]) -> Result<EcdsaProfile, VcBridgeError> {
        match pk_bytes.len() {
            33 | 65 => Ok(EcdsaProfile::P256),
            49 | 97 => Ok(EcdsaProfile::P384),
            // P-521 SEC1 (compressed 67B / uncompressed 133B). vc-di-ecdsa
            // publishes no P-521 profile, so this is a KNOWN curve that is
            // deliberately out of scope — reported as such rather than as
            // "malformed", which would misdescribe well-formed bytes.
            67 | 133 => Err(VcBridgeError::UnsupportedKeyCurve),
            _ => Err(VcBridgeError::MalformedPublicKey),
        }
    }
}

impl VcCryptosuite {
    /// The verbatim W3C `proof.cryptosuite` token (what `zk:sourceCryptosuite`
    /// records and what appears in a VC's `proof`).
    pub const fn token(self) -> &'static str {
        match self {
            VcCryptosuite::EddsaRdfc2022 => "eddsa-rdfc-2022",
            VcCryptosuite::EcdsaRdfc2019 => "ecdsa-rdfc-2019",
        }
    }

    /// Parse a W3C cryptosuite token. Fail-closed: an unknown / out-of-scope suite
    /// returns `None`, never a default. The selective-disclosure tokens
    /// (`bbs-2023`, `ecdsa-sd-2023`) are out of scope HERE and stay so — they
    /// resolve only through [`crate::vc_bridge_sd::SdCryptosuite::from_token`], so
    /// an SD credential can never reach this module's Ed25519/ECDSA paths.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "eddsa-rdfc-2022" => Some(VcCryptosuite::EddsaRdfc2022),
            "ecdsa-rdfc-2019" => Some(VcCryptosuite::EcdsaRdfc2019),
            _ => None,
        }
    }
}

/// Off-circuit VC-bridge failures. Every variant is a **fail-closed reject** — a
/// relying/ingesting party feeds issuer-/prover-controlled bytes, so the bridge
/// never panics on malformed input.
#[derive(Debug)]
pub enum VcBridgeError {
    /// The `proof.cryptosuite` token is unknown or out of the bridge's scope
    /// (e.g. `bbs-2023`). Carries the offending token.
    UnsupportedCryptosuite(String),
    /// The issuer public key is for a **known** elliptic curve that
    /// `ecdsa-rdfc-2019` publishes no profile for — currently only P-521 (SEC1
    /// 67B/133B). Both profiles the suite *does* define, P-256/SHA-256 and
    /// P-384/SHA-384, are implemented (sq-txg1y); bytes matching neither any
    /// profile nor a recognised out-of-scope curve are
    /// [`VcBridgeError::MalformedPublicKey`], not this.
    UnsupportedKeyCurve,
    /// The issuer public-key bytes do not parse as a valid key for the suite.
    MalformedPublicKey,
    /// The proof's signature bytes do not parse as a valid signature for the suite.
    MalformedSignature,
    /// The Data-Integrity proof did **not** verify against the canonical credential
    /// + proof-config under the issuer key (the off-circuit verification failed).
    VerificationFailed,
    /// Canonicalizing / committing the credential graph failed.
    Commit(CommitError),
    /// The credential graph is empty (nothing to commit).
    EmptyCredential,
    /// [OPUS-5] sq-txg1y — the JSON envelope layer ([`crate::vc_bridge_json`])
    /// could not decompose the document into a DI-secured VC: bad JSON, a missing
    /// or non-object `proof`, a proof set/chain, a missing `cryptosuite` /
    /// `verificationMethod` / `proofValue`, or a `proofValue` that is not a
    /// decodable `z` (base58-btc) multibase. Carries the specific reason.
    MalformedVcJson(String),
    /// [OPUS-5] sq-txg1y — expanding the JSON-LD document to RDF failed. The
    /// commonest cause by far is a remote `@context` the caller did not supply:
    /// the bridge performs NO network access, so an unlisted context URL is
    /// refused (by name, in this message) rather than fetched.
    JsonLdExpansion(String),
    /// [OPUS-5] sq-u5y1f — a **selective-disclosure** suite (`bbs-2023` /
    /// `ecdsa-sd-2023`) was presented to [`crate::vc_bridge_sd`] but no host
    /// verifier able to check it was supplied (none at all, or one whose
    /// [`crate::vc_bridge_sd::SelectiveDisclosureVerifier::supports`] said no).
    /// sparq implements NO selective-disclosure verifier, so this is the DEFAULT
    /// outcome for those suites. Carries the offending token.
    SelectiveDisclosureUnavailable(String),
    /// [OPUS-5] sq-u5y1f — the host-supplied selective-disclosure verifier
    /// REJECTED the derived proof. Carries that verifier's verbatim reason; the
    /// decision is the host's, not sparq's (which is why it is a distinct variant
    /// from [`VcBridgeError::VerificationFailed`], the in-repo `rdfc` outcome).
    SelectiveDisclosureRejected(String),
    /// [OPUS-5] sq-txg1y — the JSON-LD expansion produced quads outside the
    /// default graph. Refused rather than flattened, because flattening would
    /// change the canonical N-Quads the Data-Integrity hash covers.
    NamedGraphUnsupported,
}

impl std::fmt::Display for VcBridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // [OPUS-4.8] positional format args (CodeQL rust/unused-variable).
            VcBridgeError::UnsupportedCryptosuite(t) => {
                write!(f, "unsupported source cryptosuite: {}", t)
            }
            VcBridgeError::UnsupportedKeyCurve => {
                write!(
                    f,
                    "unsupported issuer key curve (in scope: Ed25519, ECDSA P-256, ECDSA P-384)"
                )
            }
            VcBridgeError::MalformedPublicKey => write!(f, "malformed issuer public key"),
            VcBridgeError::MalformedSignature => write!(f, "malformed proof signature"),
            VcBridgeError::VerificationFailed => {
                write!(f, "source VC Data-Integrity proof did not verify")
            }
            VcBridgeError::Commit(e) => write!(f, "re-commit failed: {}", e),
            VcBridgeError::EmptyCredential => write!(f, "credential graph is empty"),
            VcBridgeError::MalformedVcJson(why) => {
                write!(f, "malformed VC JSON envelope: {}", why)
            }
            VcBridgeError::JsonLdExpansion(why) => {
                write!(f, "JSON-LD expansion to RDF failed: {}", why)
            }
            VcBridgeError::SelectiveDisclosureUnavailable(t) => write!(
                f,
                "no selective-disclosure verifier available for cryptosuite {}; sparq implements \
                 none and delegates it (supply one via SelectiveDisclosureVerifier)",
                t
            ),
            VcBridgeError::SelectiveDisclosureRejected(why) => write!(
                f,
                "the host selective-disclosure verifier rejected the derived proof: {}",
                why
            ),
            VcBridgeError::NamedGraphUnsupported => write!(
                f,
                "the VC expanded to quads outside the default graph; the bridge hashes a \
                 single default graph and will not flatten a named one"
            ),
        }
    }
}

impl std::error::Error for VcBridgeError {}

/// The domain-separated **hashData** the W3C `rdfc` cryptosuites sign: the SHA-256
/// of the RDFC10-canonical proof config, concatenated with the SHA-256 of the
/// RDFC10-canonical credential document (design §5.1; W3C DI *Create/Verify
/// Proof* algorithm). `proofConfigHash || transformedDocumentHash`.
///
/// Both the proof config and the credential are taken as already-RDFC10-canonical
/// **N-Quads** byte strings (the form sparq's `canon` layer produces — see
/// [`hash_data_from_triples`] for the triples-in convenience). Returns the 64-byte
/// concatenation that the Ed25519 / ECDSA signature is over.
fn hash_data(proof_config_canonical: &[u8], credential_canonical: &[u8]) -> [u8; 64] {
    let proof_hash = Sha256::digest(proof_config_canonical);
    let doc_hash = Sha256::digest(credential_canonical);
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&proof_hash);
    out[32..].copy_from_slice(&doc_hash);
    out
}

/// The SHA-384 twin of [`hash_data`] — the `ecdsa-rdfc-2019` **P-384** profile
/// (sq-txg1y). Identical construction, wider digest:
/// `SHA-384(proofConfig) || SHA-384(document)`, 96 bytes.
fn hash_data_384(proof_config_canonical: &[u8], credential_canonical: &[u8]) -> [u8; 96] {
    let proof_hash = Sha384::digest(proof_config_canonical);
    let doc_hash = Sha384::digest(credential_canonical);
    let mut out = [0u8; 96];
    out[..48].copy_from_slice(&proof_hash);
    out[48..].copy_from_slice(&doc_hash);
    out
}

/// RDFC10-canonicalise the credential + proof-config triples and return their
/// canonical N-Quads as `(proof_config, credential)` — the two byte strings the DI
/// hashing step digests, in the order it concatenates them. Shared by both the
/// SHA-256 and SHA-384 `hashData` derivations so the canonicalization step (and
/// its fail-closed error mapping) exists exactly once.
///
/// `pub(crate)` so the selective-disclosure seam ([`crate::vc_bridge_sd`]) can
/// offer the SAME canonicalization to a host verifier — the SD suites canonicalise
/// identically, and two RDFC10 implementations disagreeing would silently change
/// the bytes a proof covers.
pub(crate) fn canonical_nquads(
    credential: &[Triple],
    proof_config: &[Triple],
) -> Result<(String, String), VcBridgeError> {
    let cred_canon = crate::canon::canonicalize_triples(credential)
        .map_err(|e| VcBridgeError::Commit(CommitError::Canon(e)))?;
    let proof_canon = crate::canon::canonicalize_triples(proof_config)
        .map_err(|e| VcBridgeError::Commit(CommitError::Canon(e)))?;
    Ok((proof_canon.to_nquads(), cred_canon.to_nquads()))
}

/// Build the signed **hashData** from the credential + proof-config **triples**:
/// RDFC10-canonicalise each (the DI suites canonicalise before hashing), then
/// `proofConfigHash || documentHash`. This is the **SHA-256** derivation, shared by
/// `eddsa-rdfc-2022` and the `ecdsa-rdfc-2019` **P-256** profile — the suites
/// differ only in the SIGNATURE check over this 64-byte hashData.
///
/// The `ecdsa-rdfc-2019` **P-384** profile hashes with SHA-384 instead — see
/// [`hash_data_from_triples_sha384`]. Feeding a P-384 proof the 64-byte hashData
/// from here cannot verify (the issuer signed 96 different bytes), which is why
/// [`verify_source_proof`] resolves the curve profile before choosing.
pub fn hash_data_from_triples(
    credential: &[Triple],
    proof_config: &[Triple],
) -> Result<[u8; 64], VcBridgeError> {
    let (proof_nq, cred_nq) = canonical_nquads(credential, proof_config)?;
    Ok(hash_data(proof_nq.as_bytes(), cred_nq.as_bytes()))
}

/// The **SHA-384** twin of [`hash_data_from_triples`] — the `ecdsa-rdfc-2019`
/// **P-384** profile's 96-byte `hashData` (`SHA-384(proofConfig) ||
/// SHA-384(document)`, W3C vc-di-ecdsa §3.1 / §A.3). Same canonicalization, same
/// proof-config-first concatenation order; only the digest widens. [OPUS-5] sq-txg1y.
pub fn hash_data_from_triples_sha384(
    credential: &[Triple],
    proof_config: &[Triple],
) -> Result<[u8; 96], VcBridgeError> {
    let (proof_nq, cred_nq) = canonical_nquads(credential, proof_config)?;
    Ok(hash_data_384(proof_nq.as_bytes(), cred_nq.as_bytes()))
}

/// Verify an Ed25519 (`eddsa-rdfc-2022`) signature over `hash_data`. Fail-closed:
/// a 32-byte public key and 64-byte signature are required; any malformed length
/// / point / scalar rejects. PUBLIC data only — no secret here.
fn verify_eddsa(pk_bytes: &[u8], sig_bytes: &[u8], hash_data: &[u8]) -> Result<(), VcBridgeError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let pk_arr: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| VcBridgeError::MalformedPublicKey)?;
    let vk = VerifyingKey::from_bytes(&pk_arr).map_err(|_| VcBridgeError::MalformedPublicKey)?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| VcBridgeError::MalformedSignature)?;
    let sig = Signature::from_bytes(&sig_arr);
    vk.verify(hash_data, &sig)
        .map_err(|_| VcBridgeError::VerificationFailed)
}

/// Verify an ECDSA-P256 (`ecdsa-rdfc-2019`, P-256 / SHA-256 profile) signature
/// over `hash_data`. Fail-closed: an SEC1 public key (compressed 33B or
/// uncompressed 65B) and a 64-byte fixed-width `(r,s)` signature are required.
/// PUBLIC data only.
///
/// NOTE: the W3C ECDSA suite's `hashData` is itself already the
/// `proofConfigHash || documentHash` concatenation; the ECDSA primitive then
/// hashes THAT with SHA-256 before the curve op (`Verifier::verify` over the raw
/// message does this digest internally), matching the suite's "hash the hashData"
/// step for the P-256 profile.
fn verify_ecdsa_p256(
    pk_bytes: &[u8],
    sig_bytes: &[u8],
    hash_data: &[u8],
) -> Result<(), VcBridgeError> {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    let vk =
        VerifyingKey::from_sec1_bytes(pk_bytes).map_err(|_| VcBridgeError::MalformedPublicKey)?;
    // Fixed-width `(r || s)` 64-byte signature (the DI ECDSA proofValue form).
    let sig = Signature::from_slice(sig_bytes).map_err(|_| VcBridgeError::MalformedSignature)?;
    vk.verify(hash_data, &sig)
        .map_err(|_| VcBridgeError::VerificationFailed)
}

/// Verify an ECDSA-P384 (`ecdsa-rdfc-2019`, **P-384 / SHA-384** profile — W3C
/// vc-di-ecdsa §A.3) signature over the 96-byte `hash_data`. Fail-closed: an SEC1
/// public key (compressed 49B or uncompressed 97B) and a 96-byte fixed-width
/// `(r,s)` signature are required. PUBLIC data only. [OPUS-5] sq-txg1y.
///
/// NOTE (the profile's whole point): `p384`'s `Verifier::verify` digests the raw
/// message with **SHA-384** before the curve operation, matching the suite's
/// "hash the hashData" step for this profile — exactly as the P-256 path above
/// digests with SHA-256. The `hash_data` handed in must therefore be the SHA-384
/// concatenation ([`hash_data_from_triples_sha384`]), not the SHA-256 one.
fn verify_ecdsa_p384(
    pk_bytes: &[u8],
    sig_bytes: &[u8],
    hash_data: &[u8],
) -> Result<(), VcBridgeError> {
    use p384::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    let vk =
        VerifyingKey::from_sec1_bytes(pk_bytes).map_err(|_| VcBridgeError::MalformedPublicKey)?;
    // Fixed-width `(r || s)` 96-byte signature (the DI ECDSA proofValue form).
    let sig = Signature::from_slice(sig_bytes).map_err(|_| VcBridgeError::MalformedSignature)?;
    vk.verify(hash_data, &sig)
        .map_err(|_| VcBridgeError::VerificationFailed)
}

/// **Off-circuit** verification of a source VC's W3C Data-Integrity proof
/// (design §5.2). RDFC10-canonicalises the credential + proof-config triples,
/// derives the suite's `hashData`, and verifies the signature under the
/// caller-resolved issuer public key for the named suite.
///
/// - `credential` — the credential graph's triples (the `proof` triple(s)
///   excluded, per the DI transform; the caller separates them).
/// - `proof_config` — the proof's triples WITHOUT `proofValue` (the proof
///   options the suite canonicalises).
/// - `suite` — the W3C cryptosuite.
/// - `issuer_pk` — the resolved issuer verification key bytes (Ed25519 32B; ECDSA
///   SEC1 — P-256 33B/65B or P-384 49B/97B). The host does NOT dereference a
///   `did:`/URL — supply the bytes.
/// - `signature` — the proof's signature bytes (`proofValue`, decoded to raw
///   bytes: Ed25519 64B; ECDSA-P256 64B, ECDSA-P384 96B, both fixed-width `r‖s`).
///
/// For `ecdsa-rdfc-2019` the **curve profile is resolved from `issuer_pk` first**
/// ([`EcdsaProfile::from_sec1_key`]), because the DI hash depends on it: P-256
/// hashes with SHA-256 (64-byte `hashData`), P-384 with SHA-384 (96-byte). Only
/// then is the matching `hashData` derived — deriving it before the profile is
/// known would hand a P-384 proof the wrong bytes and fail every valid signature.
///
/// Fail-closed: returns `Err` on any malformed input or a non-verifying proof;
/// `Ok(())` ONLY on a cryptographically valid DI proof. Asserts no in-circuit /
/// query-soundness property (sq-qhy4).
pub fn verify_source_proof(
    credential: &[Triple],
    proof_config: &[Triple],
    suite: VcCryptosuite,
    issuer_pk: &[u8],
    signature: &[u8],
) -> Result<(), VcBridgeError> {
    match suite {
        VcCryptosuite::EddsaRdfc2022 => {
            let hd = hash_data_from_triples(credential, proof_config)?;
            verify_eddsa(issuer_pk, signature, &hd)
        }
        // Resolve the curve profile BEFORE hashing — see the module docs' profile
        // table. A rejected key never reaches the (comparatively expensive)
        // canonicalization step either, which is a happy side effect, not the point.
        VcCryptosuite::EcdsaRdfc2019 => match EcdsaProfile::from_sec1_key(issuer_pk)? {
            EcdsaProfile::P256 => {
                let hd = hash_data_from_triples(credential, proof_config)?;
                verify_ecdsa_p256(issuer_pk, signature, &hd)
            }
            EcdsaProfile::P384 => {
                let hd = hash_data_from_triples_sha384(credential, proof_config)?;
                verify_ecdsa_p384(issuer_pk, signature, &hd)
            }
        },
    }
}

/// As [`verify_source_proof`] but the cryptosuite arrives as its W3C **token**
/// (the verbatim `proof.cryptosuite` value, e.g. `"eddsa-rdfc-2022"`), resolved
/// fail-closed. An unknown / out-of-scope token (incl. `bbs-2023`,
/// `ecdsa-sd-2023`) returns [`VcBridgeError::UnsupportedCryptosuite`] WITHOUT
/// touching the key/signature — the caller never has to pre-resolve the enum.
pub fn verify_source_proof_by_token(
    credential: &[Triple],
    proof_config: &[Triple],
    cryptosuite_token: &str,
    issuer_pk: &[u8],
    signature: &[u8],
) -> Result<(), VcBridgeError> {
    let suite = VcCryptosuite::from_token(cryptosuite_token)
        .ok_or_else(|| VcBridgeError::UnsupportedCryptosuite(cryptosuite_token.to_string()))?;
    verify_source_proof(credential, proof_config, suite, issuer_pk, signature)
}

/// A credential brought in through the bridge: its re-commitment under sparq's
/// pipeline, the source W3C cryptosuite it was verified under, and the document
/// IRI. Produced by [`ingest_verified_vc`] ONLY after the source DI proof verified
/// off-circuit, so an [`IngestedCredential`] is evidence the VC's proof checked at
/// ingest (it is NOT evidence of any in-circuit property).
#[derive(Debug, Clone)]
pub struct IngestedCredential {
    /// The credential document IRI (= its content-graph name in the registry).
    pub document: NamedNode,
    /// sparq's per-graph commitment `C(G)` over the RDFC10-canonical credential.
    pub commitment: GraphCommitment,
    /// The W3C cryptosuite the source VC's proof was verified under (provenance).
    pub source_cryptosuite: VcCryptosuite,
}

impl IngestedCredential {
    /// The (unattested) `<urn:sparq:zk>` registry entry for this bridged
    /// credential: it records `C(G)`, the per-graph salt, AND the
    /// `zk:sourceCryptosuite` provenance (the source W3C suite). The sparq issuer
    /// signature (`zk:commitmentSignature`) is added by the attestation path
    /// ([`RegistryEntry::issued`]), not here — the bridge attests the *source*
    /// proof, not sparq's commitment.
    pub fn registry_entry(&self) -> RegistryEntry {
        RegistryEntry::new(
            self.document.clone(),
            self.commitment.commitment,
            self.commitment.salt,
        )
        .with_source_cryptosuite(self.source_cryptosuite.token())
    }
}

/// The full bridge (design §5.2): **verify** the source VC's Data-Integrity proof
/// off-circuit, then **re-commit** the credential graph under sparq's pipeline and
/// record the source cryptosuite. The single fail-closed entry point.
///
/// - Verification runs FIRST: if the source proof does not verify, NO commitment
///   is produced (`Err`). A credential only enters the pipeline after its DI proof
///   checked — the bridge never re-commits an unverified VC.
/// - `salt` is the per-graph RDFC10 bnode salt the credential is committed under
///   (mint it via [`crate::ingest::SaltMint`] for global uniqueness).
///
/// Returns an [`IngestedCredential`] carrying `C(G)` + the source-suite provenance.
/// Asserts no in-circuit / query-soundness property (sq-qhy4); it is an ingest-time
/// host verification of the DI hashing.
pub fn ingest_verified_vc(
    document: NamedNode,
    credential: &[Triple],
    proof_config: &[Triple],
    suite: VcCryptosuite,
    issuer_pk: &[u8],
    signature: &[u8],
    salt: Fr,
) -> Result<IngestedCredential, VcBridgeError> {
    if credential.is_empty() {
        return Err(VcBridgeError::EmptyCredential);
    }
    // Verify the source DI proof FIRST — fail closed before committing anything.
    verify_source_proof(credential, proof_config, suite, issuer_pk, signature)?;
    // Only now re-commit under sparq's pipeline.
    let commitment = commit_triples(credential, salt).map_err(VcBridgeError::Commit)?;
    Ok(IngestedCredential {
        document,
        commitment,
        source_cryptosuite: suite,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::salt_from_bytes;
    use oxrdf::{Literal, NamedNode, Term};

    // A small credential graph (subject/predicate/object triples).
    fn credential() -> Vec<Triple> {
        vec![
            Triple::new(
                NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
                NamedNode::new("https://example.org/holder").unwrap(),
                NamedNode::new("https://people.example/alice").unwrap(),
            ),
            Triple::new(
                NamedNode::new("https://people.example/alice").unwrap(),
                NamedNode::new("http://schema.org/birthDate").unwrap(),
                Term::Literal(Literal::new_simple_literal("1990-01-01")),
            ),
        ]
    }

    // The proof options (proof config) the suite canonicalises — WITHOUT proofValue.
    fn proof_config(suite: VcCryptosuite) -> Vec<Triple> {
        vec![Triple::new(
            oxrdf::BlankNode::new("proof").unwrap(),
            NamedNode::new("https://w3id.org/security#cryptosuite").unwrap(),
            Term::Literal(Literal::new_simple_literal(suite.token())),
        )]
    }

    // --- suite token round-trips -------------------------------------------------

    #[test]
    fn cryptosuite_token_round_trips() {
        for suite in [VcCryptosuite::EddsaRdfc2022, VcCryptosuite::EcdsaRdfc2019] {
            assert_eq!(VcCryptosuite::from_token(suite.token()), Some(suite));
        }
    }

    #[test]
    fn unknown_and_deferred_suites_fail_closed() {
        // Deferred selective-disclosure suites and unknown tokens are rejected.
        for t in [
            "bbs-2023",
            "ecdsa-sd-2023",
            "ecdsa-rdfc-2019-p384",
            "nonsense",
            "",
        ] {
            assert_eq!(VcCryptosuite::from_token(t), None, "{} must not resolve", t);
        }
    }

    // --- hashData is deterministic + RDF-isomorphism invariant -------------------

    #[test]
    fn hash_data_is_canonicalization_invariant() {
        // Re-ordering the triples (RDF is a SET) must not change hashData, because
        // it is over the RDFC10-canonical form.
        let mut cred_a = credential();
        let cfg = proof_config(VcCryptosuite::EddsaRdfc2022);
        let hd1 = hash_data_from_triples(&cred_a, &cfg).unwrap();
        cred_a.reverse();
        let hd2 = hash_data_from_triples(&cred_a, &cfg).unwrap();
        assert_eq!(
            hd1, hd2,
            "hashData must be invariant under input triple order"
        );
    }

    #[test]
    fn hash_data_changes_with_credential_content() {
        let cfg = proof_config(VcCryptosuite::EddsaRdfc2022);
        let hd1 = hash_data_from_triples(&credential(), &cfg).unwrap();
        let mut tampered = credential();
        tampered[1] = Triple::new(
            NamedNode::new("https://people.example/alice").unwrap(),
            NamedNode::new("http://schema.org/birthDate").unwrap(),
            // changed value -> different commitment -> different hashData
            Term::Literal(Literal::new_simple_literal("2010-01-01")),
        );
        let hd2 = hash_data_from_triples(&tampered, &cfg).unwrap();
        assert_ne!(hd1, hd2, "a content change must change hashData");
    }

    // --- Ed25519 (eddsa-rdfc-2022): the REAL verify path -------------------------

    #[test]
    fn eddsa_real_proof_verifies_and_ingests() {
        use ed25519_dalek::{Signer, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EddsaRdfc2022);
        let hd = hash_data_from_triples(&cred, &cfg).unwrap();

        // A real Ed25519 issuer key signs the REAL hashData (exercises the actual
        // verify path, not a mock).
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key();
        let sig = sk.sign(&hd);

        // verify_source_proof accepts the genuine proof.
        verify_source_proof(
            &cred,
            &cfg,
            VcCryptosuite::EddsaRdfc2022,
            pk.as_bytes(),
            &sig.to_bytes(),
        )
        .expect("genuine eddsa-rdfc-2022 proof must verify");

        // ...and the full bridge re-commits + records provenance.
        let doc = NamedNode::new("https://dmv.example/vc/lic-7").unwrap();
        let salt = salt_from_bytes(&[1u8; 32]);
        let ing = ingest_verified_vc(
            doc.clone(),
            &cred,
            &cfg,
            VcCryptosuite::EddsaRdfc2022,
            pk.as_bytes(),
            &sig.to_bytes(),
            salt,
        )
        .expect("verified VC must ingest");
        assert_eq!(ing.source_cryptosuite, VcCryptosuite::EddsaRdfc2022);
        assert_eq!(ing.commitment.salt, salt);

        // The registry entry carries the source-cryptosuite provenance verbatim.
        let entry = ing.registry_entry();
        assert_eq!(entry.source_cryptosuite.as_deref(), Some("eddsa-rdfc-2022"));
        assert_eq!(entry.document, doc);
        assert_eq!(entry.commitment, ing.commitment.commitment);
    }

    #[test]
    fn eddsa_tampered_credential_fails_closed() {
        use ed25519_dalek::{Signer, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EddsaRdfc2022);
        let hd = hash_data_from_triples(&cred, &cfg).unwrap();
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key();
        let sig = sk.sign(&hd);

        // Tamper with the credential AFTER signing: the proof must NOT verify, and
        // the bridge must refuse to ingest (no commitment leaks through).
        let mut tampered = credential();
        tampered[1] = Triple::new(
            NamedNode::new("https://people.example/alice").unwrap(),
            NamedNode::new("http://schema.org/birthDate").unwrap(),
            Term::Literal(Literal::new_simple_literal("1980-01-01")),
        );
        assert!(matches!(
            verify_source_proof(
                &tampered,
                &cfg,
                VcCryptosuite::EddsaRdfc2022,
                pk.as_bytes(),
                &sig.to_bytes(),
            ),
            Err(VcBridgeError::VerificationFailed)
        ));
        assert!(ingest_verified_vc(
            NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
            &tampered,
            &cfg,
            VcCryptosuite::EddsaRdfc2022,
            pk.as_bytes(),
            &sig.to_bytes(),
            salt_from_bytes(&[1u8; 32]),
        )
        .is_err());
    }

    #[test]
    fn eddsa_wrong_key_fails_closed() {
        use ed25519_dalek::{Signer, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EddsaRdfc2022);
        let hd = hash_data_from_triples(&cred, &cfg).unwrap();
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let sig = sk.sign(&hd);
        // A DIFFERENT issuer key must reject the signature.
        let other_pk = SigningKey::from_bytes(&[8u8; 32]).verifying_key();
        assert!(matches!(
            verify_source_proof(
                &cred,
                &cfg,
                VcCryptosuite::EddsaRdfc2022,
                other_pk.as_bytes(),
                &sig.to_bytes(),
            ),
            Err(VcBridgeError::VerificationFailed)
        ));
    }

    #[test]
    fn malformed_key_and_signature_fail_closed() {
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EddsaRdfc2022);
        // Wrong-length key.
        assert!(matches!(
            verify_source_proof(
                &cred,
                &cfg,
                VcCryptosuite::EddsaRdfc2022,
                &[0u8; 16],
                &[0u8; 64]
            ),
            Err(VcBridgeError::MalformedPublicKey)
        ));
        // Wrong-length signature (valid-length key bytes that parse, bad sig len).
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        assert!(matches!(
            verify_source_proof(
                &cred,
                &cfg,
                VcCryptosuite::EddsaRdfc2022,
                sk.verifying_key().as_bytes(),
                &[0u8; 10],
            ),
            Err(VcBridgeError::MalformedSignature)
        ));
    }

    // --- ECDSA P-256 (ecdsa-rdfc-2019): the REAL verify path ---------------------

    #[test]
    fn ecdsa_p256_real_proof_verifies_and_ingests() {
        use p256::ecdsa::{signature::Signer, Signature, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EcdsaRdfc2019);
        let hd = hash_data_from_triples(&cred, &cfg).unwrap();

        let sk = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let vk = sk.verifying_key();
        let sig: Signature = sk.sign(&hd);
        // SEC1 compressed public-key bytes (the DI ECDSA verification-key form).
        let pk_sec1 = vk.to_encoded_point(true).as_bytes().to_vec();

        verify_source_proof(
            &cred,
            &cfg,
            VcCryptosuite::EcdsaRdfc2019,
            &pk_sec1,
            &sig.to_bytes(),
        )
        .expect("genuine ecdsa-rdfc-2019 proof must verify");

        let ing = ingest_verified_vc(
            NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
            &cred,
            &cfg,
            VcCryptosuite::EcdsaRdfc2019,
            &pk_sec1,
            &sig.to_bytes(),
            salt_from_bytes(&[2u8; 32]),
        )
        .expect("verified P-256 VC must ingest");
        assert_eq!(
            ing.registry_entry().source_cryptosuite.as_deref(),
            Some("ecdsa-rdfc-2019")
        );
    }

    #[test]
    fn ecdsa_p256_tampered_fails_closed() {
        use p256::ecdsa::{signature::Signer, Signature, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EcdsaRdfc2019);
        let hd = hash_data_from_triples(&cred, &cfg).unwrap();
        let sk = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let vk = sk.verifying_key();
        let sig: Signature = sk.sign(&hd);

        let mut tampered = credential();
        tampered.truncate(1); // drop a triple -> different canonical form
        assert!(matches!(
            verify_source_proof(
                &tampered,
                &cfg,
                VcCryptosuite::EcdsaRdfc2019,
                vk.to_encoded_point(true).as_bytes(),
                &sig.to_bytes(),
            ),
            Err(VcBridgeError::VerificationFailed)
        ));
    }

    #[test]
    fn verify_by_token_resolves_and_rejects_unknown() {
        use ed25519_dalek::{Signer, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EddsaRdfc2022);
        let hd = hash_data_from_triples(&cred, &cfg).unwrap();
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let sig = sk.sign(&hd);
        // A genuine proof verifies through the token entry point.
        verify_source_proof_by_token(
            &cred,
            &cfg,
            "eddsa-rdfc-2022",
            sk.verifying_key().as_bytes(),
            &sig.to_bytes(),
        )
        .expect("token path must verify a genuine proof");
        // An out-of-scope / unknown token fails closed WITHOUT touching the key.
        assert!(matches!(
            verify_source_proof_by_token(&cred, &cfg, "bbs-2023", &[0u8; 32], &[0u8; 64]),
            Err(VcBridgeError::UnsupportedCryptosuite(t)) if t == "bbs-2023"
        ));
    }

    // --- ECDSA P-384 (ecdsa-rdfc-2019, SHA-384 profile): the REAL verify path ----
    // [OPUS-5] sq-txg1y.

    #[test]
    fn ecdsa_p384_real_proof_verifies_and_ingests() {
        use p384::ecdsa::{signature::Signer, Signature, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EcdsaRdfc2019);
        // The P-384 profile signs the SHA-384 hashData, NOT the SHA-256 one.
        let hd = hash_data_from_triples_sha384(&cred, &cfg).unwrap();
        assert_eq!(hd.len(), 96, "P-384 hashData is 48+48 bytes");

        let sk = SigningKey::from_slice(&[9u8; 48]).unwrap();
        let vk = sk.verifying_key();
        let sig: Signature = sk.sign(&hd);
        assert_eq!(sig.to_bytes().len(), 96, "P-384 r||s is 48+48 bytes");
        // SEC1 compressed public-key bytes (the DI ECDSA verification-key form).
        let pk_sec1 = vk.to_encoded_point(true).as_bytes().to_vec();
        assert_eq!(pk_sec1.len(), 49);

        verify_source_proof(
            &cred,
            &cfg,
            VcCryptosuite::EcdsaRdfc2019,
            &pk_sec1,
            &sig.to_bytes(),
        )
        .expect("genuine P-384 ecdsa-rdfc-2019 proof must verify");

        let ing = ingest_verified_vc(
            NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
            &cred,
            &cfg,
            VcCryptosuite::EcdsaRdfc2019,
            &pk_sec1,
            &sig.to_bytes(),
            salt_from_bytes(&[4u8; 32]),
        )
        .expect("verified P-384 VC must ingest");
        // The provenance token is the SAME for both curve profiles — the curve is
        // not part of the W3C cryptosuite identifier.
        assert_eq!(
            ing.registry_entry().source_cryptosuite.as_deref(),
            Some("ecdsa-rdfc-2019")
        );
    }

    /// The load-bearing mutation for the profile split: a P-384 key signing the
    /// **SHA-256** `hashData` must NOT verify. If the dispatch ever hashed P-384
    /// proofs with SHA-256, this would go green and
    /// `ecdsa_p384_real_proof_verifies_and_ingests` would go red — so the pair
    /// pins the profile to the curve in both directions.
    #[test]
    fn ecdsa_p384_rejects_the_sha256_hash_data() {
        use p384::ecdsa::{signature::Signer, Signature, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EcdsaRdfc2019);
        let wrong_hd = hash_data_from_triples(&cred, &cfg).unwrap(); // SHA-256, 64B
        let sk = SigningKey::from_slice(&[9u8; 48]).unwrap();
        let sig: Signature = sk.sign(&wrong_hd);
        assert!(matches!(
            verify_source_proof(
                &cred,
                &cfg,
                VcCryptosuite::EcdsaRdfc2019,
                sk.verifying_key().to_encoded_point(true).as_bytes(),
                &sig.to_bytes(),
            ),
            Err(VcBridgeError::VerificationFailed)
        ));
    }

    #[test]
    fn ecdsa_p384_tampered_fails_closed() {
        use p384::ecdsa::{signature::Signer, Signature, SigningKey};
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EcdsaRdfc2019);
        let hd = hash_data_from_triples_sha384(&cred, &cfg).unwrap();
        let sk = SigningKey::from_slice(&[9u8; 48]).unwrap();
        let sig: Signature = sk.sign(&hd);

        let mut tampered = credential();
        tampered.truncate(1); // drop a triple -> different canonical form
        assert!(matches!(
            verify_source_proof(
                &tampered,
                &cfg,
                VcCryptosuite::EcdsaRdfc2019,
                sk.verifying_key().to_encoded_point(true).as_bytes(),
                &sig.to_bytes(),
            ),
            Err(VcBridgeError::VerificationFailed)
        ));
        // A P-256-width (64B) signature under a P-384 key is a length mismatch.
        assert!(matches!(
            verify_source_proof(
                &cred,
                &cfg,
                VcCryptosuite::EcdsaRdfc2019,
                sk.verifying_key().to_encoded_point(true).as_bytes(),
                &[0u8; 64],
            ),
            Err(VcBridgeError::MalformedSignature)
        ));
    }

    /// The two profiles produce DIFFERENT `hashData` for the same documents — a
    /// 64-byte SHA-256 concatenation and a 96-byte SHA-384 one — so a
    /// SHA-256-derived prefix can never coincide with the SHA-384 derivation.
    #[test]
    fn sha256_and_sha384_hash_data_differ() {
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EcdsaRdfc2019);
        let hd256 = hash_data_from_triples(&cred, &cfg).unwrap();
        let hd384 = hash_data_from_triples_sha384(&cred, &cfg).unwrap();
        assert_ne!(&hd384[..64], &hd256[..], "profiles must not collide");
        // Same construction, wider digest: the proof-config half comes FIRST in
        // both, so a transposed SHA-384 concatenation would fail here.
        let cfg_nquads = crate::canon::canonicalize_triples(&cfg).unwrap().to_nquads();
        let cfg_hash = Sha384::digest(cfg_nquads.as_bytes());
        assert_eq!(&hd384[..48], &cfg_hash[..]);
    }

    #[test]
    fn ecdsa_profile_resolution_is_fail_closed() {
        // Length alone decides which curve's parser runs (the parser then rejects
        // an off-curve point), so this is a pure length classification.
        let key_of = |len: usize| vec![0u8; len];
        // The two published profiles, compressed and uncompressed.
        for len in [33usize, 65] {
            assert_eq!(
                EcdsaProfile::from_sec1_key(&key_of(len)).unwrap(),
                EcdsaProfile::P256
            );
        }
        for len in [49usize, 97] {
            assert_eq!(
                EcdsaProfile::from_sec1_key(&key_of(len)).unwrap(),
                EcdsaProfile::P384
            );
        }
        // P-521: a real curve, but vc-di-ecdsa publishes no profile for it.
        for len in [67usize, 133] {
            assert!(matches!(
                EcdsaProfile::from_sec1_key(&key_of(len)),
                Err(VcBridgeError::UnsupportedKeyCurve)
            ));
        }
        // Everything else is malformed, not "some other curve".
        for len in [0usize, 1, 32, 34, 48, 64, 96, 200] {
            assert!(
                matches!(
                    EcdsaProfile::from_sec1_key(&key_of(len)),
                    Err(VcBridgeError::MalformedPublicKey)
                ),
                "{}-byte key must be MalformedPublicKey",
                len
            );
        }
    }

    /// A P-521-length key reaches the caller as the honest "known curve, out of
    /// scope" reason rather than "malformed" — and never touches the signature.
    #[test]
    fn ecdsa_p521_key_size_is_unsupported_curve() {
        let cred = credential();
        let cfg = proof_config(VcCryptosuite::EcdsaRdfc2019);
        assert!(matches!(
            verify_source_proof(
                &cred,
                &cfg,
                VcCryptosuite::EcdsaRdfc2019,
                &[0x02u8; 67],
                &[0u8; 132],
            ),
            Err(VcBridgeError::UnsupportedKeyCurve)
        ));
    }

    #[test]
    fn empty_credential_fails_closed() {
        let cfg = proof_config(VcCryptosuite::EddsaRdfc2022);
        assert!(matches!(
            ingest_verified_vc(
                NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
                &[],
                &cfg,
                VcCryptosuite::EddsaRdfc2022,
                &[0u8; 32],
                &[0u8; 64],
                salt_from_bytes(&[1u8; 32]),
            ),
            Err(VcBridgeError::EmptyCredential)
        ));
    }
}
