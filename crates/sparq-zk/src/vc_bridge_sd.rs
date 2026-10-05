// [OPUS-5] sq-u5y1f (issue #3235) — follow-up to sq-9c5e / sq-txg1y.
//! The **selective-disclosure ingest seam** for `bbs-2023` / `ecdsa-sd-2023`
//! (design `research/zk-configurable-commitment-design.md` §5.3).
//!
//! # What this is — a seam, NOT a verifier
//! [`crate::vc_bridge`] verifies the *whole-credential* `rdfc` suites off-circuit
//! because their primitives (Ed25519, ECDSA P-256/P-384) are in-repo. The
//! selective-disclosure suites are the natural match for sparq's **per-leaf**
//! disclosure model — a `bbs-2023` holder derives a proof over a *subset* of the
//! issuer-signed statements, which is exactly the shape sparq commits to — but
//! **a real BBS (or `ecdsa-sd-2023`) verifier is not in this repo, and this module
//! does not implement one.**
//!
//! So this module is the **delegation boundary**: it names the two suites, gives a
//! host a place to plug its own audited verifier in
//! ([`SelectiveDisclosureVerifier`]), does the RDF-side work sparq *can* honestly
//! do (RDFC10-canonicalise the disclosed subset, re-commit it, record provenance),
//! and **fails closed to a reject when no verifier is supplied**
//! ([`UnavailableSdVerifier`], the default).
//!
//! ```text
//! derived (disclosed-subset) VC under bbs-2023 / ecdsa-sd-2023
//!    │
//!    ├─ resolve the suite token                                     [fail closed]
//!    ├─ DELEGATE the derived-proof check to the host's verifier      [fail closed]
//!    │     └─ no verifier supplied  ->  SelectiveDisclosureUnavailable  (NEVER Ok)
//!    ├─ RDFC10-canonicalise + commit the DISCLOSED subset (`commit::commit_triples`)
//!    └─ record provenance:
//!         zk:sourceCryptosuite = the SD suite the HOST's verifier checked
//! ```
//!
//! # Honest scope boundary (load-bearing — read before claiming anything)
//! - **sparq asserts NO selective-disclosure soundness.** It performs no BBS pairing
//!   check, no `ecdsa-sd-2023` base/derived-proof check, and no verification that
//!   the disclosed subset is genuinely a subset of an issuer-signed statement set.
//!   That property comes **entirely** from the host verifier plugged into
//!   [`SelectiveDisclosureVerifier`]; an [`IngestedSdCredential`] is evidence that
//!   *that verifier* returned success, and nothing more. It carries
//!   [`IngestedSdCredential::verifier_id`] precisely so the attribution is not lost.
//! - **Unlinkability / holder privacy is NOT claimed** either. The SD suites'
//!   unlinkability is a property of the derived proof, which sparq never inspects;
//!   re-committing the disclosed subset under `C(G)` neither preserves nor
//!   establishes it.
//! - **Fail-closed by construction.** With the default [`UnavailableSdVerifier`] —
//!   or any verifier that does not [`SelectiveDisclosureVerifier::supports`] the
//!   presented suite — [`ingest_disclosed_vc`] returns `Err` and **no commitment is
//!   produced**. There is no path through this module that accepts an SD credential
//!   without an external verifier saying so.
//! - **The `rdfc` and SD token spaces stay disjoint.**
//!   [`crate::vc_bridge::VcCryptosuite::from_token`] still rejects `bbs-2023` /
//!   `ecdsa-sd-2023` (they never reach an Ed25519/ECDSA path), and
//!   [`SdCryptosuite::from_token`] rejects the `rdfc` tokens (they never reach a
//!   delegated path). Neither seam can be entered through the other.
//! - **RDF-native, like [`crate::vc_bridge`].** The derived proof value arrives as
//!   raw bytes; this module does not parse the SD suites' `u`/base64url multibase
//!   envelope, and [`crate::vc_bridge_json`] is unchanged (still `z`/base58-btc
//!   only, so a real derived-proof document does not round-trip through it yet).
//! - **NOT externally audited** (sq-qhy4). Research-grade.
//!
//! OPT-IN: behind the same OFF-by-default `vc-bridge` cargo feature as the rest of
//! the bridge. LEAN: this module adds **no** dependency — the delegation boundary
//! is a trait, so the BBS/SD crypto stays outside sparq entirely.

use crate::commit::{commit_triples, GraphCommitment};
use crate::field::Fr;
use crate::registry::RegistryEntry;
use crate::vc_bridge::VcBridgeError;
use oxrdf::{NamedNode, Triple};

/// The W3C **selective-disclosure** Data-Integrity cryptosuites this seam
/// delegates. Deliberately a *separate* type from
/// [`crate::vc_bridge::VcCryptosuite`]: that enum means "sparq verifies this
/// off-circuit itself", this one means "sparq hands this to a host verifier". The
/// two token spaces do not overlap, and neither `from_token` accepts the other's
/// tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdCryptosuite {
    /// `bbs-2023` — BBS+ selective disclosure over the RDFC10-canonical credential
    /// (W3C *Data Integrity BBS Cryptosuites v1.0*). Selective-disclosure-native:
    /// the issuer's base proof is reused to derive a proof over a disclosed subset.
    Bbs2023,
    /// `ecdsa-sd-2023` — ECDSA selective disclosure (W3C *Data Integrity ECDSA
    /// Cryptosuites v1.0* §3.4), built from per-statement ECDSA signatures under an
    /// ephemeral key rather than from a BBS proof.
    EcdsaSd2023,
}

impl SdCryptosuite {
    /// The verbatim W3C `proof.cryptosuite` token (what `zk:sourceCryptosuite`
    /// records for a delegated ingest).
    pub const fn token(self) -> &'static str {
        match self {
            SdCryptosuite::Bbs2023 => "bbs-2023",
            SdCryptosuite::EcdsaSd2023 => "ecdsa-sd-2023",
        }
    }

    /// Parse a W3C selective-disclosure cryptosuite token. Fail-closed: an unknown
    /// token returns `None`, never a default — and so do the **whole-credential**
    /// `rdfc` tokens, which belong to [`crate::vc_bridge::VcCryptosuite`] and must
    /// not be routed through a delegated SD path.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "bbs-2023" => Some(SdCryptosuite::Bbs2023),
            "ecdsa-sd-2023" => Some(SdCryptosuite::EcdsaSd2023),
            _ => None,
        }
    }
}

/// A **derived** (disclosed-subset) selective-disclosure credential, as presented
/// to a verifier. Every field is holder-/issuer-controlled input: nothing here has
/// been checked yet.
///
/// The two triple slices are the RDF the SD suites canonicalise — the disclosed
/// subset of the credential, and the proof configuration (the proof node without
/// its `proofValue`) — mirroring [`crate::vc_bridge::verify_source_proof`]'s
/// arguments. `derived_proof_value` and `issuer_pk` are **opaque** to sparq: they
/// are forwarded to the host verifier byte-for-byte and never interpreted here.
#[derive(Debug, Clone, Copy)]
pub struct SdPresentation<'a> {
    /// The selective-disclosure suite the derived proof was produced under.
    pub suite: SdCryptosuite,
    /// The **disclosed subset** of the credential's triples — the statements the
    /// holder chose to reveal, and the graph that gets re-committed on success.
    pub disclosed_credential: &'a [Triple],
    /// The proof configuration: the proof's triples WITHOUT `proofValue`.
    pub proof_config: &'a [Triple],
    /// The derived proof's raw bytes (the decoded `proofValue`). Opaque to sparq —
    /// this module neither parses nor validates the SD suites' proof envelope.
    pub derived_proof_value: &'a [u8],
    /// The resolved issuer verification-key bytes. Opaque to sparq; as everywhere
    /// in the bridge, the host does NOT dereference a `did:`/URL — supply the bytes.
    pub issuer_pk: &'a [u8],
}

impl SdPresentation<'_> {
    /// RDFC10-canonicalise the presentation's two graphs and return their canonical
    /// N-Quads as `(proof_config, disclosed_credential)` — the same pair, in the
    /// same order, that [`crate::vc_bridge`] digests for the `rdfc` suites, because
    /// the SD suites canonicalise identically before their own hashing.
    ///
    /// Offered so a host verifier can reuse sparq's RDFC10 implementation instead
    /// of bringing a second one (a mismatch between the two would silently change
    /// the bytes the proof covers). This is the **canonicalization step only** —
    /// it performs, and implies, no selective-disclosure cryptography.
    pub fn canonical_nquads(&self) -> Result<(String, String), VcBridgeError> {
        crate::vc_bridge::canonical_nquads(self.disclosed_credential, self.proof_config)
    }
}

/// The **seam**: a host-supplied verifier for the selective-disclosure suites.
///
/// sparq ships **no** implementation of this trait that can return success — the
/// only one in-repo is [`UnavailableSdVerifier`], which always rejects. A host that
/// wants `bbs-2023` / `ecdsa-sd-2023` ingest brings its own audited verifier and
/// owns the resulting soundness claim; sparq's part is the RDF canonicalization,
/// the re-commitment, and the provenance record.
///
/// Object-safe on purpose: hosts keep these in a registry keyed by suite and pass
/// `&dyn SelectiveDisclosureVerifier` into [`ingest_disclosed_vc`].
pub trait SelectiveDisclosureVerifier {
    /// A short, stable identifier for this verifier (crate name + version, a
    /// deployment id — whatever the host can attribute a decision to). Recorded on
    /// the [`IngestedSdCredential`] so a successful ingest names *who* verified it.
    fn id(&self) -> &str;

    /// Whether this verifier can check `suite`. Consulted **before**
    /// [`SelectiveDisclosureVerifier::verify_derived_proof`], so an implementation
    /// never has to defend against a suite it does not implement: a `false` here is
    /// a fail-closed [`VcBridgeError::SelectiveDisclosureUnavailable`].
    fn supports(&self, suite: SdCryptosuite) -> bool;

    /// Check the derived proof in `presentation`. `Ok(())` means **this verifier**
    /// asserts the disclosed subset is authentic under the suite; sparq re-commits
    /// on that basis and on no other.
    ///
    /// Returns the verifier's own verbatim reason on rejection, which the seam
    /// surfaces as [`VcBridgeError::SelectiveDisclosureRejected`]. The reason is a
    /// plain `String` rather than a [`VcBridgeError`] so a delegated rejection can
    /// never be mistaken for one of sparq's own in-repo verification outcomes.
    fn verify_derived_proof(&self, presentation: &SdPresentation<'_>) -> Result<(), String>;
}

/// The **default, always-rejecting** verifier — sparq's honest answer when no host
/// verifier is plugged in.
///
/// [`SelectiveDisclosureVerifier::supports`] is `false` for every suite and
/// [`SelectiveDisclosureVerifier::verify_derived_proof`] always returns `Err`, so
/// an SD credential ingested against this verifier is rejected twice over and no
/// commitment is produced. Use it wherever a `&dyn SelectiveDisclosureVerifier` is
/// required but the deployment has no SD support: the result is a clear
/// [`VcBridgeError::SelectiveDisclosureUnavailable`] rather than a silent accept.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableSdVerifier;

impl SelectiveDisclosureVerifier for UnavailableSdVerifier {
    fn id(&self) -> &str {
        "unavailable"
    }

    fn supports(&self, _suite: SdCryptosuite) -> bool {
        false
    }

    fn verify_derived_proof(&self, _presentation: &SdPresentation<'_>) -> Result<(), String> {
        Err("sparq implements no selective-disclosure verifier (sq-u5y1f); supply one".to_string())
    }
}

/// A selective-disclosure credential brought in through the seam: the
/// re-commitment of its **disclosed subset**, the SD suite, and the id of the host
/// verifier that accepted the derived proof.
///
/// Produced by [`ingest_disclosed_vc`] ONLY after that verifier returned success.
/// It is therefore evidence of **a delegated decision**, not of any property sparq
/// checked: sparq performed the RDFC10 canonicalization and the commitment, and
/// nothing cryptographic about the selective disclosure itself (sq-qhy4).
#[derive(Debug, Clone)]
pub struct IngestedSdCredential {
    /// The credential document IRI (= its content-graph name in the registry).
    pub document: NamedNode,
    /// sparq's per-graph commitment `C(G)` over the RDFC10-canonical **disclosed
    /// subset** — the revealed statements only, not the issuer's full credential.
    pub commitment: GraphCommitment,
    /// The W3C selective-disclosure suite the host verifier checked (provenance).
    pub source_cryptosuite: SdCryptosuite,
    /// [`SelectiveDisclosureVerifier::id`] of the verifier that accepted the
    /// derived proof — the party the soundness claim actually belongs to.
    pub verifier_id: String,
}

impl IngestedSdCredential {
    /// The (unattested) `<urn:sparq:zk>` registry entry for this bridged disclosed
    /// subset: `C(G)`, the per-graph salt, and the `zk:sourceCryptosuite`
    /// provenance token.
    ///
    /// The token records **which suite the host's verifier checked**, exactly as
    /// the `rdfc` path records which suite sparq itself checked — in both cases it
    /// is provenance, **not a re-verifiable in-proof property** (design §5.3): the
    /// query proof binds to sparq's `Poseidon2SchnorrV1` commitment signature and
    /// re-checks no VC proof. [`IngestedSdCredential::verifier_id`] is deliberately
    /// **not** written into the graph — the registry has no slot for it, and
    /// inventing one would put an unattested third-party attribution into
    /// verifier-visible data.
    pub fn registry_entry(&self) -> RegistryEntry {
        RegistryEntry::new(
            self.document.clone(),
            self.commitment.commitment,
            self.commitment.salt,
        )
        .with_source_cryptosuite(self.source_cryptosuite.token())
    }
}

/// Delegate a derived selective-disclosure proof to `verifier`, fail-closed.
///
/// Two gates, in this order:
/// 1. [`SelectiveDisclosureVerifier::supports`] must accept the presented suite —
///    otherwise [`VcBridgeError::SelectiveDisclosureUnavailable`], and
///    `verify_derived_proof` is **not** called.
/// 2. [`SelectiveDisclosureVerifier::verify_derived_proof`] must return `Ok` —
///    otherwise [`VcBridgeError::SelectiveDisclosureRejected`] carrying its reason.
///
/// `Ok(())` asserts only that `verifier` accepted; sparq checks nothing about the
/// derived proof itself (sq-qhy4).
pub fn verify_disclosed_proof(
    presentation: &SdPresentation<'_>,
    verifier: &dyn SelectiveDisclosureVerifier,
) -> Result<(), VcBridgeError> {
    if !verifier.supports(presentation.suite) {
        return Err(VcBridgeError::SelectiveDisclosureUnavailable(
            presentation.suite.token().to_string(),
        ));
    }
    verifier
        .verify_derived_proof(presentation)
        .map_err(VcBridgeError::SelectiveDisclosureRejected)
}

/// The full selective-disclosure bridge: **delegate** the derived-proof check to
/// `verifier`, then re-commit the **disclosed subset** under sparq's pipeline and
/// record the SD suite as provenance. The single fail-closed entry point of the
/// seam, and the mirror of [`crate::vc_bridge::ingest_verified_vc`].
///
/// - Delegation runs FIRST: if the host verifier does not accept (or none supports
///   the suite), NO commitment is produced (`Err`). There is no path here that
///   commits an unverified disclosed subset.
/// - An empty disclosed subset is rejected **before** delegation
///   ([`VcBridgeError::EmptyCredential`]) — there is nothing to commit, so nothing
///   to ask a verifier about.
/// - `salt` is the per-graph RDFC10 bnode salt the disclosed subset is committed
///   under (mint it via [`crate::ingest::SaltMint`] for global uniqueness).
///
/// The commitment is byte-identical to committing the same disclosed triples
/// through the ordinary pipeline — the seam changes *who verified*, never *what is
/// committed*.
///
/// Asserts **no** selective-disclosure soundness, unlinkability, or in-circuit /
/// query-soundness property; those belong to `verifier` (sq-qhy4).
pub fn ingest_disclosed_vc(
    document: NamedNode,
    presentation: &SdPresentation<'_>,
    verifier: &dyn SelectiveDisclosureVerifier,
    salt: Fr,
) -> Result<IngestedSdCredential, VcBridgeError> {
    if presentation.disclosed_credential.is_empty() {
        return Err(VcBridgeError::EmptyCredential);
    }
    // Delegate FIRST — fail closed before committing anything.
    verify_disclosed_proof(presentation, verifier)?;
    // Only now re-commit the disclosed subset under sparq's pipeline.
    let commitment =
        commit_triples(presentation.disclosed_credential, salt).map_err(VcBridgeError::Commit)?;
    Ok(IngestedSdCredential {
        document,
        commitment,
        source_cryptosuite: presentation.suite,
        verifier_id: verifier.id().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::salt_from_bytes;
    use crate::vc_bridge::VcCryptosuite;
    use oxrdf::{Literal, Term};
    use std::cell::RefCell;

    fn disclosed() -> Vec<Triple> {
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

    fn proof_config(suite: SdCryptosuite) -> Vec<Triple> {
        vec![Triple::new(
            oxrdf::BlankNode::new("proof").unwrap(),
            NamedNode::new("https://w3id.org/security#cryptosuite").unwrap(),
            Term::Literal(Literal::new_simple_literal(suite.token())),
        )]
    }

    /// What the seam actually forwarded, captured owned so the test can assert on
    /// it after the borrow ends.
    #[derive(Debug, PartialEq, Eq)]
    struct SeenPresentation {
        suite: SdCryptosuite,
        disclosed_len: usize,
        proof_config_len: usize,
        derived_proof_value: Vec<u8>,
        issuer_pk: Vec<u8>,
    }

    /// A stub host verifier. It performs NO cryptography — it exists to exercise
    /// the seam's control flow (supported / accepted / rejected) and to capture
    /// exactly what the seam forwarded.
    struct StubVerifier {
        supported: Vec<SdCryptosuite>,
        accept: bool,
        /// Every presentation the seam actually handed over.
        seen: RefCell<Vec<SeenPresentation>>,
    }

    impl StubVerifier {
        fn new(supported: &[SdCryptosuite], accept: bool) -> Self {
            StubVerifier {
                supported: supported.to_vec(),
                accept,
                seen: RefCell::new(Vec::new()),
            }
        }
        fn calls(&self) -> usize {
            self.seen.borrow().len()
        }
    }

    impl SelectiveDisclosureVerifier for StubVerifier {
        fn id(&self) -> &str {
            "stub-verifier/1"
        }
        fn supports(&self, suite: SdCryptosuite) -> bool {
            self.supported.contains(&suite)
        }
        fn verify_derived_proof(&self, p: &SdPresentation<'_>) -> Result<(), String> {
            self.seen.borrow_mut().push(SeenPresentation {
                suite: p.suite,
                disclosed_len: p.disclosed_credential.len(),
                proof_config_len: p.proof_config.len(),
                derived_proof_value: p.derived_proof_value.to_vec(),
                issuer_pk: p.issuer_pk.to_vec(),
            });
            if self.accept {
                Ok(())
            } else {
                Err("stub rejected the derived proof".to_string())
            }
        }
    }

    fn presentation<'a>(
        suite: SdCryptosuite,
        cred: &'a [Triple],
        cfg: &'a [Triple],
        proof: &'a [u8],
        pk: &'a [u8],
    ) -> SdPresentation<'a> {
        SdPresentation {
            suite,
            disclosed_credential: cred,
            proof_config: cfg,
            derived_proof_value: proof,
            issuer_pk: pk,
        }
    }

    // --- token spaces are disjoint and fail-closed --------------------------------

    #[test]
    fn sd_cryptosuite_token_round_trips() {
        for suite in [SdCryptosuite::Bbs2023, SdCryptosuite::EcdsaSd2023] {
            assert_eq!(SdCryptosuite::from_token(suite.token()), Some(suite));
        }
    }

    /// The two token spaces must not leak into each other: an SD token must never
    /// reach the in-repo Ed25519/ECDSA path, and an `rdfc` token must never reach
    /// the delegated path (where a permissive host verifier could accept it).
    #[test]
    fn sd_and_rdfc_token_spaces_are_disjoint() {
        for t in ["bbs-2023", "ecdsa-sd-2023"] {
            assert_eq!(VcCryptosuite::from_token(t), None, "{} must stay delegated", t);
        }
        for t in ["eddsa-rdfc-2022", "ecdsa-rdfc-2019"] {
            assert_eq!(
                SdCryptosuite::from_token(t),
                None,
                "{} must stay in-repo, not delegated",
                t
            );
        }
        for t in ["bbs", "bbs-2022", "ecdsa-sd", "nonsense", ""] {
            assert_eq!(SdCryptosuite::from_token(t), None, "{} must not resolve", t);
        }
    }

    // --- fail-closed: no verifier, no commitment ----------------------------------

    /// THE load-bearing invariant of this seam: with sparq's own (only) in-repo
    /// verifier, an SD credential CANNOT be ingested. If `UnavailableSdVerifier`
    /// ever started accepting — or if `ingest_disclosed_vc` committed before
    /// delegating — this goes red.
    #[test]
    fn without_a_host_verifier_nothing_is_ingested() {
        let cred = disclosed();
        let cfg = proof_config(SdCryptosuite::Bbs2023);
        let p = presentation(SdCryptosuite::Bbs2023, &cred, &cfg, &[1u8; 80], &[2u8; 96]);

        assert!(matches!(
            verify_disclosed_proof(&p, &UnavailableSdVerifier),
            Err(VcBridgeError::SelectiveDisclosureUnavailable(t)) if t == "bbs-2023"
        ));
        assert!(matches!(
            ingest_disclosed_vc(
                NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
                &p,
                &UnavailableSdVerifier,
                salt_from_bytes(&[1u8; 32]),
            ),
            Err(VcBridgeError::SelectiveDisclosureUnavailable(_))
        ));
    }

    /// A verifier that does not support the presented suite is never asked about
    /// it — the `supports` gate runs first, so an implementation cannot be handed
    /// a suite it did not opt into.
    #[test]
    fn unsupported_suite_fails_closed_without_calling_the_verifier() {
        let cred = disclosed();
        let cfg = proof_config(SdCryptosuite::EcdsaSd2023);
        let v = StubVerifier::new(&[SdCryptosuite::Bbs2023], true);
        let p = presentation(SdCryptosuite::EcdsaSd2023, &cred, &cfg, &[3u8; 8], &[4u8; 8]);
        assert!(matches!(
            ingest_disclosed_vc(
                NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
                &p,
                &v,
                salt_from_bytes(&[1u8; 32]),
            ),
            Err(VcBridgeError::SelectiveDisclosureUnavailable(t)) if t == "ecdsa-sd-2023"
        ));
        assert_eq!(v.calls(), 0, "an unsupported suite must not reach the verifier");
    }

    /// A rejecting verifier produces NO commitment, and its verbatim reason is
    /// surfaced (not flattened into one of sparq's own in-repo outcomes).
    #[test]
    fn a_rejecting_verifier_produces_no_commitment() {
        let cred = disclosed();
        let cfg = proof_config(SdCryptosuite::Bbs2023);
        let v = StubVerifier::new(&[SdCryptosuite::Bbs2023], false);
        let p = presentation(SdCryptosuite::Bbs2023, &cred, &cfg, &[5u8; 16], &[6u8; 16]);
        let err = ingest_disclosed_vc(
            NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
            &p,
            &v,
            salt_from_bytes(&[1u8; 32]),
        )
        .expect_err("a rejected derived proof must not ingest");
        let VcBridgeError::SelectiveDisclosureRejected(why) = &err else {
            panic!("expected a delegated rejection, got {:?}", err);
        };
        assert_eq!(why, "stub rejected the derived proof");
        assert_eq!(v.calls(), 1);
    }

    /// An empty disclosed subset is rejected before delegation — there is nothing
    /// to commit, so the verifier is never troubled with it.
    #[test]
    fn empty_disclosed_subset_fails_closed_before_delegating() {
        let cfg = proof_config(SdCryptosuite::Bbs2023);
        let v = StubVerifier::new(&[SdCryptosuite::Bbs2023], true);
        let p = presentation(SdCryptosuite::Bbs2023, &[], &cfg, &[0u8; 4], &[0u8; 4]);
        assert!(matches!(
            ingest_disclosed_vc(
                NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
                &p,
                &v,
                salt_from_bytes(&[1u8; 32]),
            ),
            Err(VcBridgeError::EmptyCredential)
        ));
        assert_eq!(v.calls(), 0);
    }

    // --- the accepting path: what sparq actually does -----------------------------

    /// On acceptance the seam commits the DISCLOSED subset — byte-identical to the
    /// ordinary pipeline over the same triples — and records the suite + the
    /// verifier's id. If the seam ever committed something else (the proof config,
    /// a merged graph), the equality here goes red.
    #[test]
    fn an_accepting_verifier_ingests_the_disclosed_subset_and_records_provenance() {
        let cred = disclosed();
        let cfg = proof_config(SdCryptosuite::Bbs2023);
        let v = StubVerifier::new(&[SdCryptosuite::Bbs2023], true);
        let salt = salt_from_bytes(&[7u8; 32]);
        let doc = NamedNode::new("https://dmv.example/vc/lic-7").unwrap();
        let p = presentation(SdCryptosuite::Bbs2023, &cred, &cfg, &[8u8; 32], &[9u8; 48]);

        let ing = ingest_disclosed_vc(doc.clone(), &p, &v, salt).expect("accepted proof ingests");
        assert_eq!(ing.source_cryptosuite, SdCryptosuite::Bbs2023);
        assert_eq!(ing.verifier_id, "stub-verifier/1");
        assert_eq!(ing.commitment.salt, salt);
        // Same commitment as the ordinary pipeline over the same disclosed triples.
        let direct = commit_triples(&cred, salt).unwrap();
        assert_eq!(ing.commitment.commitment, direct.commitment);

        let entry = ing.registry_entry();
        assert_eq!(entry.document, doc);
        assert_eq!(entry.commitment, ing.commitment.commitment);
        assert_eq!(entry.source_cryptosuite.as_deref(), Some("bbs-2023"));
        // The verifier attribution stays OUT of the registry graph.
        assert_eq!(entry.commitment_signature, None);
    }

    /// The seam forwards the presentation verbatim: the verifier sees the suite,
    /// both graphs, and the opaque proof/key bytes exactly as supplied.
    #[test]
    fn the_presentation_reaches_the_verifier_unmodified() {
        let cred = disclosed();
        let cfg = proof_config(SdCryptosuite::EcdsaSd2023);
        let v = StubVerifier::new(&[SdCryptosuite::EcdsaSd2023], true);
        let proof_bytes: Vec<u8> = (0u8..37).collect();
        let pk_bytes: Vec<u8> = (100u8..133).collect();
        let p = presentation(
            SdCryptosuite::EcdsaSd2023,
            &cred,
            &cfg,
            &proof_bytes,
            &pk_bytes,
        );
        ingest_disclosed_vc(
            NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
            &p,
            &v,
            salt_from_bytes(&[3u8; 32]),
        )
        .expect("accepted proof ingests");
        let seen = v.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0],
            SeenPresentation {
                suite: SdCryptosuite::EcdsaSd2023,
                disclosed_len: cred.len(),
                proof_config_len: cfg.len(),
                derived_proof_value: proof_bytes.clone(),
                issuer_pk: pk_bytes.clone(),
            }
        );
    }

    // --- the canonicalization helper ---------------------------------------------

    /// The helper is RDFC10 — invariant under input triple order (RDF is a set) —
    /// and returns `(proof_config, credential)` in that order, matching what the
    /// `rdfc` path digests.
    #[test]
    fn canonical_nquads_is_order_invariant_and_proof_config_first() {
        let cred = disclosed();
        let cfg = proof_config(SdCryptosuite::Bbs2023);
        let p = presentation(SdCryptosuite::Bbs2023, &cred, &cfg, &[], &[]);
        let (cfg_nq, cred_nq) = p.canonical_nquads().unwrap();

        let mut reversed = disclosed();
        reversed.reverse();
        let p2 = presentation(SdCryptosuite::Bbs2023, &reversed, &cfg, &[], &[]);
        let (cfg_nq2, cred_nq2) = p2.canonical_nquads().unwrap();
        assert_eq!((&cfg_nq, &cred_nq), (&cfg_nq2, &cred_nq2));

        // First element is the PROOF CONFIG, not the credential.
        assert!(cfg_nq.contains("cryptosuite"));
        assert!(cred_nq.contains("birthDate"));
    }
}
