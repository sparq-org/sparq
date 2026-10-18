//! `<urn:sparq:zk>` registry plumbing (plan §2.1, Q13).
//!
//! One reserved synthesized graph `<urn:sparq:zk>` (vocabulary
//! `zk:` = `https://sparq.dev/ns/zk#`, mirroring `auth:`) holds the registry
//! the prover needs, one entry per credential document:
//!
//! ```turtle
//! <https://dmv.example/vc/lic-123>
//!     zk:commitment      "0x1a2b…"^^zk:field ;
//!     zk:scheme          zk:poseidon2-rdfc10-v1 ;
//!     zk:cryptosuite     zk:poseidon2-schnorr-v1 ;
//!     zk:issuerKey       <did:example:dmv#key-1> ;
//!     zk:signatureGraph  <https://dmv.example/vc/lic-123?proof> ;
//!     zk:statusList      <https://dmv.example/status/3#94> ;
//!     zk:rdfc10Salt      "0x9f3c…"^^zk:field ;
//!     zk:status          zk:active ;
//!     zk:ingested        "2026-06-12T00:00:00Z"^^xsd:dateTime .
//! ```
//!
//! Conventions mirrored from `sparq-solid`'s `<urn:sparq:auth>`
//! (`install_auth_view`): the registry graph lives in `Graph::named` under
//! the reserved name, is rebuilt wholesale by its installer (never merged
//! with pre-existing content — pre-existing copies are an attack, plan
//! §2.1's loader-stripping rule), and reading is fail-closed (malformed
//! entries are dropped, an absent graph yields no entries). The proof graph
//! convention `<D + "?proof">` (Q13) is provided by [`proof_graph_name`].

use crate::field::{field_from_hex_str, field_to_hex, Fr};
use oxrdf::{NamedNode, Term};
use sparq_core::dict::Dict;
use sparq_core::Graph;

/// The reserved registry graph name.
pub const ZK_GRAPH: &str = "urn:sparq:zk";
/// The reserved IRI space (same hardening domain as `sparq-solid`).
pub const RESERVED_PREFIX: &str = "urn:sparq:";
/// The zk vocabulary namespace.
pub const ZK_NS: &str = "https://sparq.dev/ns/zk#";

pub const ZK_COMMITMENT: &str = "https://sparq.dev/ns/zk#commitment";
pub const ZK_SCHEME: &str = "https://sparq.dev/ns/zk#scheme";
pub const ZK_CRYPTOSUITE: &str = "https://sparq.dev/ns/zk#cryptosuite";
/// **Provenance** predicate (sq-9c5e): the W3C Data-Integrity cryptosuite the
/// *source* VC was signed under, recorded verbatim when a credential is brought
/// in through the off-circuit VC ingest bridge (`crate::vc_bridge`, design
/// `research/zk-configurable-commitment-design.md` §5.2 / §10 Q5 default).
///
/// This is **distinct** from [`ZK_CRYPTOSUITE`]: `zk:cryptosuite` names the
/// *sparq* scheme the in-circuit / verifier-side query proof binds to
/// (`poseidon2-schnorr-v1`); `zk:sourceCryptosuite` is *back-compat / perf-
/// comparison provenance only* — it records which W3C suite (`eddsa-rdfc-2022`,
/// `ecdsa-rdfc-2019`, …) the credential carried before it was off-circuit
/// verified and re-committed. It is **NOT** a re-verifiable in-proof property
/// (§5.3): the query proof does not re-check the VC's Data-Integrity signature.
/// Recorded as a plain literal (the verbatim `proof.cryptosuite` value, which is
/// a bare suite token in a W3C VC, not an absolute IRI).
// sq-9c5e.
pub const ZK_SOURCE_CRYPTOSUITE: &str = "https://sparq.dev/ns/zk#sourceCryptosuite";
pub const ZK_ISSUER_KEY: &str = "https://sparq.dev/ns/zk#issuerKey";
/// The issuer's actual verification-key material (compressed Baby-JubJub point,
/// hex) — the bytes the verifier resolves to check the commitment signature
/// (audit #3). Distinct from `zk:issuerKey`, which is a `did:`/verification-
/// method REFERENCE; this is the resolvable key the signature is verified
/// under. (`zk:issuerKey` carries provenance; `zk:issuerPublicKey` carries the
/// cryptographic key. A production resolver would dereference the former to the
/// latter; v1 records the latter directly so the proof is self-contained.)
// audit #3: real signing key, not just a did: IRI.
pub const ZK_ISSUER_PUBLIC_KEY: &str = "https://sparq.dev/ns/zk#issuerPublicKey";
/// The issuer's signature over `C(G)` (hex), under `zk:cryptosuite`. This is
/// what makes the commitment attested rather than prover-chosen (audit #3).
// audit #3.
pub const ZK_COMMITMENT_SIGNATURE: &str = "https://sparq.dev/ns/zk#commitmentSignature";
pub const ZK_SIGNATURE_GRAPH: &str = "https://sparq.dev/ns/zk#signatureGraph";
pub const ZK_STATUS_LIST: &str = "https://sparq.dev/ns/zk#statusList";
/// The credential's index into its status list, and the status-list version the
/// signature is bound to (audit #12). Both are folded — with `H(zk:statusList)`
/// — into the issuer-signed status reference digest, so the reference is
/// unforgeable/un-omittable on the verify path.
// audit #12: issuer-bound status-list index + version.
pub const ZK_STATUS_LIST_INDEX: &str = "https://sparq.dev/ns/zk#statusListIndex";
pub const ZK_STATUS_LIST_VERSION: &str = "https://sparq.dev/ns/zk#statusListVersion";
pub const ZK_RDFC10_SALT: &str = "https://sparq.dev/ns/zk#rdfc10Salt";
pub const ZK_STATUS: &str = "https://sparq.dev/ns/zk#status";
pub const ZK_INGESTED: &str = "https://sparq.dev/ns/zk#ingested";
/// Datatype of commitment/salt literals.
pub const ZK_FIELD: &str = "https://sparq.dev/ns/zk#field";
/// The v1 commitment scheme id (this crate's pipeline) — the `string-canonical`
/// commitment method ([`crate::commit::CommitmentMethod::StringCanonicalV1`]),
/// the byte-unchanged default. Retained verbatim for back-compat.
pub const ZK_SCHEME_POSEIDON2_RDFC10_V1: &str = "https://sparq.dev/ns/zk#poseidon2-rdfc10-v1";
/// The `dual-leaf` commitment scheme id (value handle AND lexical-identity hash)
/// — [`crate::commit::CommitmentMethod::DualLeafV1`]. Selecting it records the
/// #769-accepted INV-VL downgrade on the entry (config only here; the leaf
/// encoding is the audit-gated `sq-j506` — NOT implemented in this crate yet).
// sq-zzxt: configurable-commitment scheme IRIs (design
// `research/zk-configurable-commitment-design.md` §2.1).
pub const ZK_SCHEME_POSEIDON2_DUALLEAF_V1: &str = "https://sparq.dev/ns/zk#poseidon2-dualleaf-v1";
/// The `dual-leaf` **v2** commitment scheme id —
/// [`crate::commit::CommitmentMethod::DualLeafV2`]. The versioned dual-leaf
/// scheme: value-lane constants derived in BLAKE3's key-derivation mode over
/// length-framed `(datatype, params)`, an out-of-band no-value sentinel, and a
/// bit-domain boolean member. This is the dual-leaf method a new issuance
/// selects; `DualLeafV1` stays recognised and verifiable but is no longer
/// production-selectable.
pub const ZK_SCHEME_POSEIDON2_DUALLEAF_V2: &str = "https://sparq.dev/ns/zk#poseidon2-dualleaf-v2";
/// The `value-only` commitment scheme id (value handle only, lexical hash
/// dropped) — `CommitmentMethod::ValueOnlyV1`, a
/// **benchmark / research dial** (§10 default (2)). The IRI const is always
/// present, but the method that selects it is compiled out unless the
/// OFF-by-default `commitment-value-only` feature is enabled, so a default build
/// cannot select it.
// sq-zzxt.
pub const ZK_SCHEME_POSEIDON2_VALUEHOOK_V1: &str = "https://sparq.dev/ns/zk#poseidon2-valuehook-v1";
pub const ZK_STATUS_ACTIVE: &str = "https://sparq.dev/ns/zk#active";
pub const ZK_STATUS_REVOKED: &str = "https://sparq.dev/ns/zk#revoked";
const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

/// Q13 convention: the proof graph of document `<D>` is `<D + "?proof">`.
pub fn proof_graph_name(document: &NamedNode) -> NamedNode {
    NamedNode::new_unchecked(format!("{}?proof", document.as_str()))
}

/// Credential status in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Revoked,
}

/// One `<urn:sparq:zk>` registry entry.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryEntry {
    /// Credential document IRI (= its content graph name).
    pub document: NamedNode,
    /// `C(G)`, the per-graph commitment.
    pub commitment: Fr,
    /// Commitment scheme id (`zk:poseidon2-rdfc10-v1` for this pipeline).
    ///
    /// PRIVATE and fixed at construction: read it with [`Self::scheme`] or
    /// [`Self::method`]. A signed entry's signature covers it (see
    /// [`Self::verify_commitment_signature`]), so it is set once by the
    /// constructor that also signs, and nothing writes it afterwards.
    scheme: NamedNode,
    /// Issuer cryptosuite id, when known.
    pub cryptosuite: Option<NamedNode>,
    /// **Provenance** (sq-9c5e): the W3C Data-Integrity cryptosuite the *source*
    /// VC was signed under (verbatim `proof.cryptosuite`, e.g. `eddsa-rdfc-2022`),
    /// set when the entry was produced by the off-circuit VC ingest bridge
    /// (`crate::vc_bridge`). Back-compat / perf-comparison provenance only — NOT a
    /// re-verifiable in-proof property; distinct from [`Self::cryptosuite`] (the
    /// sparq scheme the query proof binds to). `None` for non-VC-bridge entries.
    // sq-9c5e.
    pub source_cryptosuite: Option<String>,
    /// Issuer verification-method reference (a `did:` URL).
    pub issuer_key: Option<NamedNode>,
    /// The issuer's actual verification key (compressed Baby-JubJub point, hex)
    /// — the key the verifier checks the commitment signature under (audit #3).
    //
    pub issuer_public_key: Option<String>,
    /// The issuer's signature over `C(G)` (hex), under `cryptosuite`. Absent =>
    /// the commitment is UNATTESTED and a verifier requiring attestation must
    /// reject it (audit #3).
    //
    pub commitment_signature: Option<String>,
    /// The credential's proof graph (`<D>?proof` by convention).
    pub signature_graph: Option<NamedNode>,
    /// Status-list reference (plan §2.6).
    pub status_list: Option<NamedNode>,
    /// The credential's index into its status list (audit #12). Bound — with
    /// `status_list_version` and `H(status_list)` — under the issuer signature
    /// when the entry is issued via [`Self::issued_with_status`].
    // audit #12.
    pub status_list_index: Option<u64>,
    /// The status-list version this entry's signature is bound to (audit #12).
    // audit #12.
    pub status_list_version: Option<u64>,
    /// The per-graph RDFC10 bnode salt (Q6).
    pub salt: Fr,
    pub status: Status,
    /// Ingest timestamp (`xsd:dateTime` lexical form).
    pub ingested: Option<String>,
}

impl RegistryEntry {
    /// Minimal UNSIGNED entry for a freshly committed graph, recording the
    /// default `string-canonical` method
    /// ([`CommitmentMethod::StringCanonicalV1`](crate::commit::CommitmentMethod::StringCanonicalV1)).
    /// An entry under another method is built by [`Self::new_for_issuance`],
    /// [`Self::new_already_issued`], [`Self::issued`] or
    /// [`Self::issued_with_status`] — the method is chosen at construction and
    /// is not writable afterwards.
    pub fn new(document: NamedNode, commitment: Fr, salt: Fr) -> Self {
        Self::with_scheme(
            document,
            commitment,
            salt,
            NamedNode::new_unchecked(ZK_SCHEME_POSEIDON2_RDFC10_V1),
        )
    }

    /// The one place an entry's `scheme` is written.
    fn with_scheme(document: NamedNode, commitment: Fr, salt: Fr, scheme: NamedNode) -> Self {
        RegistryEntry {
            document,
            commitment,
            scheme,
            cryptosuite: None,
            source_cryptosuite: None,
            issuer_key: None,
            issuer_public_key: None,
            commitment_signature: None,
            signature_graph: None,
            status_list: None,
            status_list_index: None,
            status_list_version: None,
            salt,
            status: Status::Active,
            ingested: None,
        }
    }

    /// The recorded `zk:scheme` IRI. Read-only: the field is fixed when the
    /// entry is constructed (or decoded by [`read_registry`]).
    pub fn scheme(&self) -> &NamedNode {
        &self.scheme
    }

    /// The [`CommitmentMethod`](crate::commit::CommitmentMethod) this entry's
    /// `zk:scheme` IRI selects, or `None` if the recorded scheme is one this build
    /// does not understand.
    ///
    /// **Fail-closed** (sq-zzxt, design §10 default (1)): an unknown/unrecognized
    /// `zk:scheme` — including the `value-only` research-dial IRI when the
    /// `commitment-value-only` feature is OFF — returns `None`, never a default. A
    /// verifier that requires a known method MUST reject an entry whose
    /// [`Self::method`] is `None` rather than assume `string-canonical`; the
    /// signature checks here already do. The method is **in-circuit-affecting**,
    /// so the `(method, circuit)` dispatch matrix (sq-cfmv) consumes it to refuse
    /// an illegal pairing.
    // sq-zzxt: commitment-method selection over the `zk:scheme` slot.
    pub fn method(&self) -> Option<crate::commit::CommitmentMethod> {
        crate::commit::CommitmentMethod::from_scheme_iri(self.scheme.as_str())
    }

    /// An UNSIGNED entry recording `method` **as-is**, for a graph committed
    /// EARLIER — decoding, rebuilding an entry for an already-issued graph, and
    /// tests. It accepts a method that is no longer selectable for new issuance.
    ///
    /// "Already issued" is a CALLER ASSERTION: nothing here reads a signature, a
    /// timestamp or any other evidence of issuance history. What the name buys is
    /// that legacy acceptance is written down at the call site; a NEW entry uses
    /// [`Self::new_for_issuance`] (unsigned) or [`Self::issued`] (signed), which
    /// refuse a non-selectable method.
    ///
    /// This is **config only**: it does NOT re-commit the graph under a different
    /// leaf shape; it records the method so it is an explicit property of the
    /// entry rather than a silent default.
    // sq-zzxt.
    pub fn new_already_issued(
        document: NamedNode,
        commitment: Fr,
        salt: Fr,
        method: crate::commit::CommitmentMethod,
    ) -> Self {
        Self::with_scheme(document, commitment, salt, method.scheme_node())
    }

    /// An UNSIGNED entry recording `method` for a **new issuance**, refusing one
    /// that is not selectable
    /// ([`CommitmentMethod::checked_for_issuance`](crate::commit::CommitmentMethod::checked_for_issuance)):
    /// a superseded method (`dual-leaf` v1) or a research-only dial
    /// (`value-only`) is a
    /// [`MethodNotSelectable`](crate::commit::MethodNotSelectable) error rather
    /// than a silently recorded scheme. Decoding and verification stay ungated —
    /// [`Self::method`] still reads a recorded `dual-leaf` v1 entry, and the
    /// `(method x circuit)` dispatch matrix still admits its lanes — so this
    /// refusal narrows what sparq ISSUES, never what it can verify.
    ///
    /// Honest scope: selectability means "not superseded and not a research dial".
    /// It asserts NO cryptographic property; the estate is NOT externally audited
    /// (CR-G8 / sq-qhy4).
    pub fn new_for_issuance(
        document: NamedNode,
        commitment: Fr,
        salt: Fr,
        method: crate::commit::CommitmentMethod,
    ) -> Result<Self, crate::commit::MethodNotSelectable> {
        Ok(Self::new_already_issued(
            document,
            commitment,
            salt,
            method.checked_for_issuance()?,
        ))
    }

    /// The message an issuer signs for this entry — salt-bound, and status-bound
    /// when `status_ref` is given — with the recorded METHOD in it, for EVERY
    /// method including `string-canonical`:
    /// [`crate::sig::commitment_message_with_scheme`] /
    /// [`crate::sig::commitment_message_with_status_and_scheme`] over the
    /// [`crate::sig::scheme_iri_to_field`] digest of the scheme IRI. So a
    /// signature verifies only under the method it was issued with. `None` for a
    /// scheme this build does not recognise.
    fn method_bound_message(&self, status_ref: Option<&Fr>) -> Option<Fr> {
        let scheme = crate::sig::scheme_iri_to_field(self.method()?.scheme_iri());
        Some(match status_ref {
            None => {
                crate::sig::commitment_message_with_scheme(&self.commitment, &self.salt, &scheme)
            }
            Some(r) => crate::sig::commitment_message_with_status_and_scheme(
                &self.commitment,
                &self.salt,
                r,
                &scheme,
            ),
        })
    }

    /// The status-reference digest this entry's status-bound signature covers,
    /// or `None` when any of the three status fields is absent.
    fn status_ref(&self) -> Option<Fr> {
        let (Some(sl), Some(idx), Some(ver)) =
            (&self.status_list, self.status_list_index, self.status_list_version)
        else {
            return None;
        };
        let list_id = crate::sig::status_list_id_to_field(sl.as_str());
        Some(crate::sig::status_ref_digest(&list_id, idx, ver))
    }

    /// Records the **source** W3C Data-Integrity cryptosuite (`zk:sourceCryptosuite`)
    /// — provenance for a credential brought in through the off-circuit VC ingest
    /// bridge (sq-9c5e). `suite` is the verbatim `proof.cryptosuite` token of the
    /// source VC (e.g. `eddsa-rdfc-2022`). This is back-compat / perf-comparison
    /// provenance ONLY; it does not change [`Self::cryptosuite`] (the sparq scheme
    /// the query proof binds to) and is not a re-verifiable in-proof property.
    // sq-9c5e.
    #[must_use]
    pub fn with_source_cryptosuite(mut self, suite: impl Into<String>) -> Self {
        self.source_cryptosuite = Some(suite.into());
        self
    }

    /// An attested entry: a freshly committed graph signed by an issuer
    /// (audit #3). Records the issuer's public key + a Schnorr signature over
    /// the domain-separated commitment message, plus the `zk:cryptosuite` id.
    /// This is the issuance-side path (an issuer tool / tests); a relying party
    /// only ever VERIFIES via [`Self::verify_commitment_signature`].
    ///
    /// # Nonce safety (codex job 2216 MEDIUM)
    /// Signing routes through [`crate::sig::SecretKey::sign_commitment`], whose
    /// Schnorr nonce is derived DETERMINISTICALLY from `(sk, commitment_message)`
    /// (RFC6979-style; see `sig::derive_nonce`). No caller-supplied RNG/seed is
    /// taken, so a cloned/reused RNG state can never reuse a Schnorr nonce across
    /// commitments and leak the issuer secret key. (The earlier deterministic-
    /// signing fix added `sign_commitment` but issuance still took a caller RNG;
    /// this routes issuance through it.) The random-nonce `sig::sign` is now
    /// test-only — no public API takes a caller RNG.
    ///
    /// # The method is chosen here and signed
    /// `method` is checked with
    /// [`CommitmentMethod::checked_for_issuance`](crate::commit::CommitmentMethod::checked_for_issuance)
    /// first (a superseded or research-only method is a
    /// [`MethodNotSelectable`](crate::commit::MethodNotSelectable) error and
    /// nothing is signed), then recorded on the entry and carried in the signed
    /// message (see [`Self::verify_commitment_signature`]), so the signature is
    /// over the method as well as the commitment and salt.
    // audit #3; codex 2216 MEDIUM: deterministic-nonce issuance, no caller RNG.
    pub fn issued(
        document: NamedNode,
        commitment: Fr,
        salt: Fr,
        method: crate::commit::CommitmentMethod,
        sk: &crate::sig::SecretKey,
    ) -> Result<Self, crate::commit::MethodNotSelectable> {
        let mut e = RegistryEntry::new_for_issuance(document, commitment, salt, method)?;
        // audit #9: the per-graph salt is in the signed object, so it is
        // issuer-attested and salt reuse is detectable verifier-side; the method
        // is in it too. A recognised method always yields a message.
        let m = e
            .method_bound_message(None)
            .expect("a method that passed checked_for_issuance is recognised");
        e.sign_with(&m, sk);
        Ok(e)
    }

    /// Record the cryptosuite, issuer key and a deterministic-nonce signature
    /// over `m` — the shared tail of [`Self::issued`] and
    /// [`Self::issued_with_status`].
    fn sign_with(&mut self, m: &Fr, sk: &crate::sig::SecretKey) {
        // Deterministic (sk, m) nonce — see `sig::sign_deterministic`. No RNG
        // state crosses the API boundary.
        let sig = crate::sig::sign_deterministic(sk, m);
        self.cryptosuite = Some(NamedNode::new_unchecked(
            crate::sig::SignatureScheme::Poseidon2SchnorrV1.cryptosuite_iri(),
        ));
        self.issuer_public_key = Some(crate::sig::public_key_to_hex(&sk.public_key()));
        self.commitment_signature = Some(crate::sig::signature_to_hex(&sig));
    }

    /// An attested entry whose signature ALSO binds the credential's status-list
    /// reference (audit #12): the issuer signs the METHOD-BOUND
    /// [`crate::sig::commitment_message_with_status_and_scheme`] (`ZKSIG_M3`)
    /// over `(C(G), salt, status_ref_digest, scheme_digest)`, where
    /// `status_ref_digest = status_ref_digest(H(status_list), index, version)`
    /// and `scheme_digest` is the [`crate::sig::scheme_iri_to_field`] digest of
    /// the recorded method's scheme IRI — NOT the method-unbound
    /// `commitment_message_with_status` (`ZKSIG_C3`), which a registry entry is
    /// never verified over.
    /// So the reference (which list, which index, which version) is
    /// issuer-attested and cannot be omitted or forged by the prover on the
    /// verify path. Same deterministic `(sk, m)` nonce discipline as
    /// [`Self::issued`], and the same method handling: `method` is checked for
    /// issuance first and is carried in the signed message.
    // audit #12: status-bound issuance.
    // clippy: each arg is a distinct cryptographic input to a signing constructor;
    // bundling into a params struct would only obscure the call sites.
    #[allow(clippy::too_many_arguments)]
    pub fn issued_with_status(
        document: NamedNode,
        commitment: Fr,
        salt: Fr,
        method: crate::commit::CommitmentMethod,
        status_list: NamedNode,
        index: u64,
        version: u64,
        status: Status,
        sk: &crate::sig::SecretKey,
    ) -> Result<Self, crate::commit::MethodNotSelectable> {
        let mut e = RegistryEntry::new_for_issuance(document, commitment, salt, method)?;
        e.status_list = Some(status_list);
        e.status_list_index = Some(index);
        e.status_list_version = Some(version);
        e.status = status;
        let status_ref = e.status_ref().expect("the three status fields were just set");
        let m = e
            .method_bound_message(Some(&status_ref))
            .expect("a method that passed checked_for_issuance is recognised");
        e.sign_with(&m, sk);
        Ok(e)
    }

    /// Verify that this entry's recorded signature is a valid STATUS-BOUND issuer
    /// signature (audit #12): over `(commitment, salt, status_ref_digest)` where
    /// the digest folds `H(status_list)`, `status_list_index`, and
    /// `status_list_version`, and over the recorded METHOD exactly as
    /// [`Self::verify_commitment_signature`] describes. Fail-closed: a missing
    /// key/signature/cryptosuite/status-list/index/version, an unrecognised
    /// `zk:scheme`, an unknown cryptosuite, or malformed hex all return `false`.
    /// The caller must ALSO check the key is in the disclosed key-set `K` and
    /// validate liveness (status bit + freshness) separately.
    // audit #12.
    pub fn verify_commitment_signature_with_status(&self) -> bool {
        let (Some(pk_hex), Some(sig_hex), Some(cs), Some(status_ref)) = (
            &self.issuer_public_key,
            &self.commitment_signature,
            &self.cryptosuite,
            self.status_ref(),
        ) else {
            return false;
        };
        let Some(msg) = self.method_bound_message(Some(&status_ref)) else {
            return false;
        };
        // sq-1hsl: route through the pluggable OFF-circuit signature
        // seam. `verify_commitment_with_scheme` resolves the cryptosuite to its
        // `IssuerSignatureScheme` and fails closed on an unknown cryptosuite or
        // malformed key/signature hex — byte-identical accept/reject to the prior
        // inline `from_cryptosuite_iri` + parse + `verify` block.
        crate::sig::verify_commitment_with_scheme(cs.as_str(), pk_hex, &msg, sig_hex)
    }

    /// Verify that this entry's recorded signature is a valid issuer signature
    /// over its commitment under its recorded public key + cryptosuite — the
    /// verifier-side attestation gate (audit #3). Fail-closed: a missing key,
    /// missing signature, unrecognised `zk:scheme`, unknown cryptosuite, or
    /// malformed hex all return `false`. NB this checks the signature is
    /// INTERNALLY valid; the caller must ALSO check the key is a member of the
    /// disclosed key-set `K`.
    ///
    /// The recorded METHOD is part of what is verified: the message is
    /// recomputed from the entry's own `zk:scheme`, so an entry whose scheme
    /// differs from the one it was signed under does not verify. The message is
    /// the method-bound one for every method, `string-canonical` included; the
    /// method-independent salt-bound message is never accepted here.
    // audit #3.
    pub fn verify_commitment_signature(&self) -> bool {
        let (Some(pk_hex), Some(sig_hex), Some(cs)) =
            (&self.issuer_public_key, &self.commitment_signature, &self.cryptosuite)
        else {
            return false;
        };
        // audit #9: issuance via `issued` signs the SALT-BOUND message (with
        // the method in it), so verification recomputes it from the entry.
        let Some(msg) = self.method_bound_message(None) else {
            return false;
        };
        // sq-1hsl: route through the pluggable OFF-circuit signature
        // seam. Only the v1 scheme is verifiable in-tree; an unknown cryptosuite
        // (no resolved `IssuerSignatureScheme`) or malformed key/signature hex
        // fails closed — byte-identical to the prior inline `from_cryptosuite_iri`
        // + parse + `verify` block.
        crate::sig::verify_commitment_with_scheme(cs.as_str(), pk_hex, &msg, sig_hex)
    }

    /// Serializes the entry as triples of the registry graph.
    pub fn to_triples(&self) -> Vec<[Term; 3]> {
        let s = Term::NamedNode(self.document.clone());
        let field_dt = NamedNode::new_unchecked(ZK_FIELD);
        let pred = |p: &str| Term::NamedNode(NamedNode::new_unchecked(p));
        let mut out = vec![
            [
                s.clone(),
                pred(ZK_COMMITMENT),
                Term::Literal(oxrdf::Literal::new_typed_literal(
                    field_to_hex(&self.commitment),
                    field_dt.clone(),
                )),
            ],
            [s.clone(), pred(ZK_SCHEME), Term::NamedNode(self.scheme.clone())],
            [
                s.clone(),
                pred(ZK_RDFC10_SALT),
                Term::Literal(oxrdf::Literal::new_typed_literal(
                    field_to_hex(&self.salt),
                    field_dt,
                )),
            ],
            [
                s.clone(),
                pred(ZK_STATUS),
                Term::NamedNode(NamedNode::new_unchecked(match self.status {
                    Status::Active => ZK_STATUS_ACTIVE,
                    Status::Revoked => ZK_STATUS_REVOKED,
                })),
            ],
        ];
        if let Some(cs) = &self.cryptosuite {
            out.push([s.clone(), pred(ZK_CRYPTOSUITE), Term::NamedNode(cs.clone())]);
        }
        // sq-9c5e: source-VC cryptosuite provenance (plain literal,
        // the verbatim W3C `proof.cryptosuite` token).
        if let Some(scs) = &self.source_cryptosuite {
            out.push([
                s.clone(),
                pred(ZK_SOURCE_CRYPTOSUITE),
                Term::Literal(oxrdf::Literal::new_simple_literal(scs.clone())),
            ]);
        }
        if let Some(k) = &self.issuer_key {
            out.push([s.clone(), pred(ZK_ISSUER_KEY), Term::NamedNode(k.clone())]);
        }
        // audit #3: the cryptographic key + the commitment signature.
        if let Some(pk) = &self.issuer_public_key {
            out.push([
                s.clone(),
                pred(ZK_ISSUER_PUBLIC_KEY),
                Term::Literal(oxrdf::Literal::new_simple_literal(pk.clone())),
            ]);
        }
        if let Some(sig) = &self.commitment_signature {
            out.push([
                s.clone(),
                pred(ZK_COMMITMENT_SIGNATURE),
                Term::Literal(oxrdf::Literal::new_simple_literal(sig.clone())),
            ]);
        }
        if let Some(g) = &self.signature_graph {
            out.push([s.clone(), pred(ZK_SIGNATURE_GRAPH), Term::NamedNode(g.clone())]);
        }
        if let Some(sl) = &self.status_list {
            out.push([s.clone(), pred(ZK_STATUS_LIST), Term::NamedNode(sl.clone())]);
        }
        // audit #12: the issuer-bound status-list index + version.
        let xsd_int = NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#nonNegativeInteger");
        if let Some(idx) = self.status_list_index {
            out.push([
                s.clone(),
                pred(ZK_STATUS_LIST_INDEX),
                Term::Literal(oxrdf::Literal::new_typed_literal(idx.to_string(), xsd_int.clone())),
            ]);
        }
        if let Some(ver) = self.status_list_version {
            out.push([
                s.clone(),
                pred(ZK_STATUS_LIST_VERSION),
                Term::Literal(oxrdf::Literal::new_typed_literal(ver.to_string(), xsd_int)),
            ]);
        }
        if let Some(ts) = &self.ingested {
            out.push([
                s.clone(),
                pred(ZK_INGESTED),
                Term::Literal(oxrdf::Literal::new_typed_literal(
                    ts.clone(),
                    NamedNode::new_unchecked(XSD_DATETIME),
                )),
            ]);
        }
        out
    }
}

/// Registry installation / parse errors.
#[derive(Debug)]
pub enum RegistryError {
    /// A document IRI may not live in the reserved `urn:sparq:` space.
    ReservedDocumentIri(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::ReservedDocumentIri(iri) => {
                write!(f, "registry entry document IRI is in the reserved space: {iri}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Installs (replaces wholesale) the `<urn:sparq:zk>` registry graph on a
/// store, mirroring `sparq-solid::install_auth_view`: a fresh sub-graph
/// dictionary, `Graph::from_parts`, and replacement of any existing slot —
/// pre-existing registry content never survives (only the installer writes
/// this graph).
pub fn install_registry(graph: &mut Graph, entries: &[RegistryEntry]) -> Result<usize, RegistryError> {
    for e in entries {
        if e.document.as_str().starts_with(RESERVED_PREFIX) {
            return Err(RegistryError::ReservedDocumentIri(e.document.as_str().to_string()));
        }
    }
    let mut dict = Dict::new();
    let mut ids: Vec<[sparq_core::dict::Id; 3]> = Vec::new();
    for e in entries {
        for [s, p, o] in e.to_triples() {
            ids.push([dict.intern(&s), dict.intern(&p), dict.intern(&o)]);
        }
    }
    let n = ids.len();
    let reg = Graph::from_parts(dict, ids);
    let name = Term::NamedNode(NamedNode::new_unchecked(ZK_GRAPH));
    if let Some(slot) = graph.named.iter_mut().find(|(gname, _)| *gname == name) {
        slot.1 = reg;
    } else {
        graph.named.push((name, reg));
    }
    Ok(n)
}

/// Reads the registry back from a store. Fail-closed: an absent registry
/// graph yields no entries; entries missing a parseable commitment or salt
/// are dropped.
pub fn read_registry(graph: &Graph) -> Vec<RegistryEntry> {
    let name = Term::NamedNode(NamedNode::new_unchecked(ZK_GRAPH));
    let Some((_, reg)) = graph.named.iter().find(|(gname, _)| *gname == name) else {
        return Vec::new();
    };

    // Group predicate/object pairs by subject, preserving first-seen order.
    let mut order: Vec<NamedNode> = Vec::new();
    let mut by_subject: std::collections::HashMap<String, Vec<(String, Term)>> =
        std::collections::HashMap::new();
    for t in reg.iter_ids() {
        let s = reg.dict().term(t[0]);
        let p = reg.dict().term(t[1]);
        let o = reg.dict().term(t[2]);
        let (Term::NamedNode(s), Term::NamedNode(p)) = (s, p) else { continue };
        if !by_subject.contains_key(s.as_str()) {
            order.push(s.clone());
        }
        by_subject
            .entry(s.as_str().to_string())
            .or_default()
            .push((p.as_str().to_string(), o));
    }

    let mut out = Vec::new();
    for doc in order {
        let props = &by_subject[doc.as_str()];
        let field_of = |pred: &str| -> Option<Fr> {
            props.iter().find_map(|(p, o)| {
                if p != pred {
                    return None;
                }
                match o {
                    Term::Literal(l) if l.datatype().as_str() == ZK_FIELD => {
                        field_from_hex_str(l.value())
                    }
                    _ => None,
                }
            })
        };
        let iri_of = |pred: &str| -> Option<NamedNode> {
            props.iter().find_map(|(p, o)| {
                if p != pred {
                    return None;
                }
                match o {
                    Term::NamedNode(n) => Some(n.clone()),
                    _ => None,
                }
            })
        };
        let (Some(commitment), Some(salt)) = (field_of(ZK_COMMITMENT), field_of(ZK_RDFC10_SALT))
        else {
            continue; // fail closed on malformed entries
        };
        let Some(scheme) = iri_of(ZK_SCHEME) else { continue };
        let status = match iri_of(ZK_STATUS) {
            Some(s) if s.as_str() == ZK_STATUS_REVOKED => Status::Revoked,
            Some(s) if s.as_str() == ZK_STATUS_ACTIVE => Status::Active,
            _ => continue, // unknown status: fail closed
        };
        let ingested = props.iter().find_map(|(p, o)| match o {
            Term::Literal(l) if p == ZK_INGESTED => Some(l.value().to_string()),
            _ => None,
        });
        // audit #3: plain-literal key + signature hex.
        let str_of = |pred: &str| -> Option<String> {
            props.iter().find_map(|(p, o)| match o {
                Term::Literal(l) if p == pred => Some(l.value().to_string()),
                _ => None,
            })
        };
        // audit #12: integer literal (status-list index / version).
        let u64_of = |pred: &str| -> Option<u64> {
            props.iter().find_map(|(p, o)| match o {
                Term::Literal(l) if p == pred => l.value().parse::<u64>().ok(),
                _ => None,
            })
        };
        out.push(RegistryEntry {
            document: doc,
            commitment,
            scheme,
            cryptosuite: iri_of(ZK_CRYPTOSUITE),
            // sq-9c5e: source-VC cryptosuite provenance (plain literal).
            source_cryptosuite: str_of(ZK_SOURCE_CRYPTOSUITE),
            issuer_key: iri_of(ZK_ISSUER_KEY),
            issuer_public_key: str_of(ZK_ISSUER_PUBLIC_KEY),
            commitment_signature: str_of(ZK_COMMITMENT_SIGNATURE),
            signature_graph: iri_of(ZK_SIGNATURE_GRAPH),
            status_list: iri_of(ZK_STATUS_LIST),
            status_list_index: u64_of(ZK_STATUS_LIST_INDEX),
            status_list_version: u64_of(ZK_STATUS_LIST_VERSION),
            salt,
            status,
            ingested,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::salt_from_bytes;

    fn entry(doc: &str) -> RegistryEntry {
        let mut e = RegistryEntry::new(
            NamedNode::new(doc).unwrap(),
            Fr::from(42u64),
            salt_from_bytes(&[9u8; 32]),
        );
        e.cryptosuite = Some(NamedNode::new_unchecked("https://sparq.dev/ns/zk#poseidon2-schnorr-v1"));
        e.issuer_key = Some(NamedNode::new_unchecked("did:example:dmv#key-1"));
        e.signature_graph = Some(proof_graph_name(&e.document));
        e.status_list = Some(NamedNode::new_unchecked("https://dmv.example/status/3#94"));
        e.ingested = Some("2026-06-12T00:00:00Z".to_string());
        e
    }

    // sq-9c5e: the `zk:sourceCryptosuite` provenance slot round-trips
    // through the store unchanged (the real serialize/parse path), independent of
    // the sparq `zk:cryptosuite`. An entry without it parses back as `None`.
    #[test]
    fn source_cryptosuite_round_trips_through_store() {
        let e = RegistryEntry::new(
            NamedNode::new("https://dmv.example/vc/lic-9").unwrap(),
            Fr::from(7u64),
            salt_from_bytes(&[2u8; 32]),
        )
        .with_source_cryptosuite("eddsa-rdfc-2022");
        assert_eq!(e.source_cryptosuite.as_deref(), Some("eddsa-rdfc-2022"));
        let mut g = Graph::load_str("", "turtle").unwrap();
        install_registry(&mut g, std::slice::from_ref(&e)).unwrap();
        let back = read_registry(&g);
        assert_eq!(back, vec![e]);
        assert_eq!(back[0].source_cryptosuite.as_deref(), Some("eddsa-rdfc-2022"));
        // A plain entry has no source-cryptosuite (the slot is optional provenance).
        let plain =
            RegistryEntry::new(NamedNode::new("https://x/y").unwrap(), Fr::from(1u64), Fr::from(2u64));
        assert_eq!(plain.source_cryptosuite, None);
        let mut g2 = Graph::load_str("", "turtle").unwrap();
        install_registry(&mut g2, std::slice::from_ref(&plain)).unwrap();
        assert_eq!(read_registry(&g2)[0].source_cryptosuite, None);
    }

    // audit #3: an attested entry, its signature, and tamper cases.
    #[test]
    fn issued_entry_signature_round_trips_through_store_and_verifies() {
        use crate::sig::SecretKey;
        let sk = SecretKey::from_seed(11);
        let commitment = Fr::from(0xc0ffeeu64);
        let salt = salt_from_bytes(&[3u8; 32]);
        let e = RegistryEntry::issued(
            NamedNode::new("https://dmv.example/vc/lic-7").unwrap(),
            commitment,
            salt,
            crate::commit::CommitmentMethod::StringCanonicalV1,
            &sk,
        )
        .unwrap();
        assert!(e.verify_commitment_signature(), "fresh issued entry verifies");
        // Round-trips through the store unchanged (key + sig survive).
        let mut g = Graph::load_str("", "turtle").unwrap();
        install_registry(&mut g, std::slice::from_ref(&e)).unwrap();
        let back = read_registry(&g);
        assert_eq!(back, vec![e]);
        assert!(back[0].verify_commitment_signature(), "read-back entry verifies");
    }

    // audit #12: a status-bound issued entry verifies, round-trips
    // through the store (index + version survive), and rejects a tampered
    // status-list reference (index/version/list) — the property that makes an
    // omitted/forged status reference unverifiable on the verify path.
    #[test]
    fn issued_with_status_round_trips_and_binds_the_reference() {
        use crate::sig::SecretKey;
        let sk = SecretKey::from_seed(31);
        let commitment = Fr::from(0xbeefu64);
        let salt = salt_from_bytes(&[5u8; 32]);
        let sl = NamedNode::new("https://dmv.example/status/3").unwrap();
        let e = RegistryEntry::issued_with_status(
            NamedNode::new("https://dmv.example/vc/lic-st").unwrap(),
            commitment,
            salt,
            crate::commit::CommitmentMethod::StringCanonicalV1,
            sl.clone(),
            94,
            7,
            Status::Active,
            &sk,
        )
        .unwrap();
        assert!(e.verify_commitment_signature_with_status(), "fresh status-bound entry verifies");
        // Round-trips through the store.
        let mut g = Graph::load_str("", "turtle").unwrap();
        install_registry(&mut g, std::slice::from_ref(&e)).unwrap();
        let back = read_registry(&g);
        assert_eq!(back, vec![e.clone()]);
        assert!(back[0].verify_commitment_signature_with_status(), "read-back verifies");

        // Tamper the index/version/list => the recomputed digest diverges => fail.
        let mut tampered = e.clone();
        tampered.status_list_index = Some(95);
        assert!(!tampered.verify_commitment_signature_with_status(), "wrong index must fail");
        let mut tampered = e.clone();
        tampered.status_list_version = Some(8);
        assert!(!tampered.verify_commitment_signature_with_status(), "wrong version must fail");
        let mut tampered = e;
        tampered.status_list = Some(NamedNode::new("https://attacker.example/empty").unwrap());
        assert!(!tampered.verify_commitment_signature_with_status(), "wrong list must fail");
    }

    #[test]
    fn issued_entry_rejects_commitment_tamper() {
        // The drop-a-triple-and-recommit shape: change the commitment after
        // signing => the signature no longer verifies.
        use crate::sig::SecretKey;
        let sk = SecretKey::from_seed(12);
        let mut e = RegistryEntry::issued(
            NamedNode::new("https://dmv.example/vc/lic-8").unwrap(),
            Fr::from(1000u64),
            salt_from_bytes(&[4u8; 32]),
            crate::commit::CommitmentMethod::StringCanonicalV1,
            &sk,
        )
        .unwrap();
        e.commitment = Fr::from(1001u64); // recommit over a different (truncated) graph
        assert!(!e.verify_commitment_signature(), "tampered commitment must not verify");
    }

    // codex 2216 MEDIUM: issuance is deterministic + nonce-safe. The
    // signature comes from the `(sk, m)`-derived-nonce path, so (a) re-issuing
    // the SAME (sk, commitment) yields byte-identical signatures (no caller RNG),
    // and (b) two DISTINCT commitments under the same key get DISTINCT nonces `R`
    // — the property that makes the seed/nonce-reuse `sk`-extraction attack
    // impossible by construction (there is no caller RNG to clone/reuse).
    #[test]
    fn issued_is_deterministic_and_nonce_safe() {
        use crate::sig::{signature_from_hex, SecretKey};
        let sk = SecretKey::from_seed(99);
        let salt = salt_from_bytes(&[7u8; 32]);
        let doc = NamedNode::new("https://dmv.example/vc/lic-det").unwrap();

        // (a) Determinism: two issuances of the same (sk, commitment) match.
        let c = Fr::from(0xabc123u64);
        let sc = crate::commit::CommitmentMethod::StringCanonicalV1;
        let e1 = RegistryEntry::issued(doc.clone(), c, salt, sc, &sk).unwrap();
        let e2 = RegistryEntry::issued(doc.clone(), c, salt, sc, &sk).unwrap();
        assert_eq!(
            e1.commitment_signature, e2.commitment_signature,
            "issuance must be deterministic (no caller RNG => byte-stable signature)"
        );
        assert!(e1.verify_commitment_signature());

        // (b) Nonce safety: distinct commitments => distinct nonces R. If R were
        // reused, sk would be recoverable; deterministic (sk, m) nonces forbid it.
        let other = RegistryEntry::issued(doc, c + Fr::from(1u64), salt, sc, &sk).unwrap();
        let sig1 = signature_from_hex(e1.commitment_signature.as_ref().unwrap()).unwrap();
        let sig_other = signature_from_hex(other.commitment_signature.as_ref().unwrap()).unwrap();
        assert_ne!(
            sig1.r, sig_other.r,
            "distinct commitments must use distinct Schnorr nonces R (no nonce reuse)"
        );
    }

    #[test]
    fn unsigned_entry_does_not_verify() {
        // A plain (unsigned) entry has no commitment signature => unattested.
        let e = entry("https://dmv.example/vc/unsigned");
        // `entry` sets a cryptosuite IRI but no public key / signature.
        assert!(!e.verify_commitment_signature(), "unsigned entry is unattested");
    }

    #[test]
    fn round_trip_through_store() {
        let mut g = Graph::load_str("", "turtle").unwrap();
        let entries = vec![entry("https://dmv.example/vc/lic-123"), entry("https://rail.example/vc/t-9")];
        let n = install_registry(&mut g, &entries).unwrap();
        assert!(n >= entries.len() * 4);
        let back = read_registry(&g);
        assert_eq!(back, entries);
    }

    #[test]
    fn reinstall_replaces_wholesale() {
        let mut g = Graph::load_str("", "turtle").unwrap();
        install_registry(&mut g, &[entry("https://dmv.example/vc/a")]).unwrap();
        install_registry(&mut g, &[entry("https://dmv.example/vc/b")]).unwrap();
        let back = read_registry(&g);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].document.as_str(), "https://dmv.example/vc/b");
    }

    #[test]
    fn reserved_document_iri_rejected() {
        let mut g = Graph::load_str("", "turtle").unwrap();
        let e = RegistryEntry::new(
            NamedNode::new_unchecked("urn:sparq:zk-fake"),
            Fr::from(1u64),
            Fr::from(2u64),
        );
        assert!(matches!(
            install_registry(&mut g, &[e]),
            Err(RegistryError::ReservedDocumentIri(_))
        ));
    }

    #[test]
    fn read_is_fail_closed() {
        // Absent graph.
        let g = Graph::load_str("", "turtle").unwrap();
        assert!(read_registry(&g).is_empty());
        // Malformed entry (no commitment): dropped.
        let mut g = Graph::load_str("", "turtle").unwrap();
        let mut dict = Dict::new();
        let s = Term::NamedNode(NamedNode::new_unchecked("https://x.example/vc/1"));
        let p = Term::NamedNode(NamedNode::new_unchecked(ZK_STATUS));
        let o = Term::NamedNode(NamedNode::new_unchecked(ZK_STATUS_ACTIVE));
        let ids = vec![[dict.intern(&s), dict.intern(&p), dict.intern(&o)]];
        let reg = Graph::from_parts(dict, ids);
        g.named.push((Term::NamedNode(NamedNode::new_unchecked(ZK_GRAPH)), reg));
        assert!(read_registry(&g).is_empty());
    }

    #[test]
    fn proof_graph_convention() {
        let d = NamedNode::new("https://dmv.example/vc/lic-123").unwrap();
        assert_eq!(proof_graph_name(&d).as_str(), "https://dmv.example/vc/lic-123?proof");
    }

    // sq-zzxt: commitment-method selection over the `zk:scheme` slot.

    #[test]
    fn fresh_entry_defaults_to_string_canonical_method() {
        use crate::commit::CommitmentMethod;
        let e = RegistryEntry::new(
            NamedNode::new("https://dmv.example/vc/m-default").unwrap(),
            Fr::from(1u64),
            salt_from_bytes(&[1u8; 32]),
        );
        // The byte-unchanged default: a fresh entry records string-canonical.
        assert_eq!(e.scheme().as_str(), ZK_SCHEME_POSEIDON2_RDFC10_V1);
        assert_eq!(e.method(), Some(CommitmentMethod::StringCanonicalV1));
    }

    #[test]
    fn new_already_issued_records_and_round_trips_through_store() {
        use crate::commit::CommitmentMethod;
        let e = RegistryEntry::new_already_issued(
            NamedNode::new("https://dmv.example/vc/m-dual").unwrap(),
            Fr::from(7u64),
            salt_from_bytes(&[2u8; 32]),
            CommitmentMethod::DualLeafV1,
        );
        assert_eq!(e.scheme().as_str(), ZK_SCHEME_POSEIDON2_DUALLEAF_V1);
        assert_eq!(e.method(), Some(CommitmentMethod::DualLeafV1));

        // The method survives a round-trip through the registry graph (the scheme
        // IRI is serialized + re-read unchanged).
        let mut g = Graph::load_str("", "turtle").unwrap();
        install_registry(&mut g, std::slice::from_ref(&e)).unwrap();
        let back = read_registry(&g);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].method(), Some(CommitmentMethod::DualLeafV1));
    }

    /// The ISSUANCE boundary is checked, not advisory: `new_for_issuance`
    /// refuses a superseded or research-only method, while `new_already_issued`
    /// still records one for decoding / legacy entries — and an entry so recorded
    /// is still READ back by `method()`, so verification of an already-issued
    /// graph is unaffected.
    #[test]
    fn new_for_issuance_refuses_a_non_selectable_method() {
        use crate::commit::CommitmentMethod;
        let doc = || NamedNode::new("https://dmv.example/vc/m-issuance").unwrap();
        let fresh = |m| RegistryEntry::new_for_issuance(doc(), Fr::from(9u64), salt_from_bytes(&[3u8; 32]), m);
        let legacy = |m| {
            RegistryEntry::new_already_issued(doc(), Fr::from(9u64), salt_from_bytes(&[3u8; 32]), m)
        };

        // Selectable: recorded exactly as `new_already_issued` would.
        let v2 = fresh(CommitmentMethod::DualLeafV2).expect("V2 is selectable for new issuance");
        assert_eq!(v2.method(), Some(CommitmentMethod::DualLeafV2));
        assert_eq!(v2, legacy(CommitmentMethod::DualLeafV2));
        assert!(fresh(CommitmentMethod::StringCanonicalV1).is_ok());

        // Superseded: refused, and the error names the IRI it did not write.
        let err = fresh(CommitmentMethod::DualLeafV1).expect_err("V1 must not be selectable for new issuance");
        assert_eq!(err.method(), CommitmentMethod::DualLeafV1);
        assert_eq!(err.scheme_iri(), ZK_SCHEME_POSEIDON2_DUALLEAF_V1);

        // ...but recording V1 as-is still works and still READS BACK, which is
        // what keeps an already-issued V1 graph verifiable.
        let v1 = legacy(CommitmentMethod::DualLeafV1);
        assert_eq!(v1.scheme().as_str(), ZK_SCHEME_POSEIDON2_DUALLEAF_V1);
        assert_eq!(v1.method(), Some(CommitmentMethod::DualLeafV1));

        // The research dial is refused too, when it is compiled in at all.
        #[cfg(feature = "commitment-value-only")]
        assert!(fresh(CommitmentMethod::ValueOnlyV1).is_err());
    }

    /// The signing constructors take the method and run the same issuance
    /// check: a non-selectable method is refused and nothing is signed.
    #[test]
    fn issued_constructors_refuse_a_non_selectable_method() {
        use crate::commit::CommitmentMethod;
        use crate::sig::SecretKey;
        let sk = SecretKey::from_seed(41);
        let doc = NamedNode::new("https://dmv.example/vc/m-refused").unwrap();
        let salt = salt_from_bytes(&[6u8; 32]);
        let err = RegistryEntry::issued(doc.clone(), Fr::from(5u64), salt, CommitmentMethod::DualLeafV1, &sk)
            .expect_err("a superseded method must not be signed for a new entry");
        assert_eq!(err.method(), CommitmentMethod::DualLeafV1);
        let err = RegistryEntry::issued_with_status(
            doc.clone(),
            Fr::from(5u64),
            salt,
            CommitmentMethod::DualLeafV1,
            NamedNode::new("https://dmv.example/status/1").unwrap(),
            1,
            1,
            Status::Active,
            &sk,
        )
        .expect_err("a superseded method must not be signed for a new status-bound entry");
        assert_eq!(err.method(), CommitmentMethod::DualLeafV1);
        #[cfg(feature = "commitment-value-only")]
        assert!(RegistryEntry::issued(doc, Fr::from(5u64), salt, CommitmentMethod::ValueOnlyV1, &sk).is_err());
    }

    /// Rewrite an entry's `zk:scheme` triple in the store and read it back — the
    /// only way to change a recorded method from outside this module.
    fn with_scheme_rewritten_in_store(e: &RegistryEntry, scheme_iri: &str) -> RegistryEntry {
        let mut dict = Dict::new();
        let mut ids = Vec::new();
        for [s, p, o] in e.to_triples() {
            let o = match &p {
                Term::NamedNode(n) if n.as_str() == ZK_SCHEME => {
                    Term::NamedNode(NamedNode::new_unchecked(scheme_iri))
                }
                _ => o,
            };
            ids.push([dict.intern(&s), dict.intern(&p), dict.intern(&o)]);
        }
        let mut g = Graph::load_str("", "turtle").unwrap();
        g.named.push((
            Term::NamedNode(NamedNode::new_unchecked(ZK_GRAPH)),
            Graph::from_parts(dict, ids),
        ));
        let mut back = read_registry(&g);
        assert_eq!(back.len(), 1);
        let back = back.remove(0);
        assert_eq!(back.scheme().as_str(), scheme_iri);
        back
    }

    fn every_scheme_iri() -> Vec<&'static str> {
        vec![
            ZK_SCHEME_POSEIDON2_RDFC10_V1,
            ZK_SCHEME_POSEIDON2_DUALLEAF_V1,
            ZK_SCHEME_POSEIDON2_DUALLEAF_V2,
            ZK_SCHEME_POSEIDON2_VALUEHOOK_V1,
            "https://sparq.dev/ns/zk#poseidon2-bogus-v9",
        ]
    }

    /// The signature binds the method: for every method an entry can be issued
    /// under, rewriting the recorded `zk:scheme` to ANY other IRI makes both
    /// signature checks fail, and leaving it alone keeps them passing.
    #[test]
    fn rewriting_the_recorded_scheme_invalidates_the_signature() {
        use crate::commit::CommitmentMethod;
        use crate::sig::SecretKey;
        let sk = SecretKey::from_seed(77);
        let salt = salt_from_bytes(&[8u8; 32]);
        let list = NamedNode::new("https://dmv.example/status/7").unwrap();
        for method in [CommitmentMethod::StringCanonicalV1, CommitmentMethod::DualLeafV2] {
            let plain = RegistryEntry::issued(
                NamedNode::new("https://dmv.example/vc/m-bind").unwrap(),
                Fr::from(0x5eedu64),
                salt,
                method,
                &sk,
            )
            .unwrap();
            let bound = RegistryEntry::issued_with_status(
                NamedNode::new("https://dmv.example/vc/m-bind-st").unwrap(),
                Fr::from(0x5eedu64),
                salt,
                method,
                list.clone(),
                3,
                4,
                Status::Active,
                &sk,
            )
            .unwrap();
            assert_eq!(plain.method(), Some(method));
            for iri in every_scheme_iri() {
                let same = iri == method.scheme_iri();
                assert_eq!(
                    with_scheme_rewritten_in_store(&plain, iri).verify_commitment_signature(),
                    same,
                    "{} signed, {} recorded",
                    method.scheme_iri(),
                    iri
                );
                assert_eq!(
                    with_scheme_rewritten_in_store(&bound, iri)
                        .verify_commitment_signature_with_status(),
                    same,
                    "{} signed (status-bound), {} recorded",
                    method.scheme_iri(),
                    iri
                );
            }
        }
    }

    /// Every method, `string-canonical` included, signs the method-bound
    /// message: a fresh signature verifies over it and not over the
    /// method-independent salt-bound / status-bound message.
    #[test]
    fn every_method_signs_the_method_bound_message() {
        use crate::commit::CommitmentMethod;
        use crate::sig::{
            commitment_message_with_salt, commitment_message_with_scheme,
            commitment_message_with_status, commitment_message_with_status_and_scheme,
            scheme_iri_to_field, SecretKey,
        };
        let sk = SecretKey::from_seed(78);
        let (c, salt) = (Fr::from(0xfeedu64), salt_from_bytes(&[9u8; 32]));
        let doc = NamedNode::new("https://dmv.example/vc/m-compat").unwrap();
        let list = NamedNode::new("https://dmv.example/status/9").unwrap();
        let verifies_over = |e: &RegistryEntry, m: &Fr| {
            crate::sig::verify_commitment_with_scheme(
                e.cryptosuite.as_ref().unwrap().as_str(),
                e.issuer_public_key.as_ref().unwrap(),
                m,
                e.commitment_signature.as_ref().unwrap(),
            )
        };
        for method in [CommitmentMethod::StringCanonicalV1, CommitmentMethod::DualLeafV2] {
            let scheme = scheme_iri_to_field(method.scheme_iri());
            let plain = RegistryEntry::issued(doc.clone(), c, salt, method, &sk).unwrap();
            let bound = RegistryEntry::issued_with_status(
                doc.clone(),
                c,
                salt,
                method,
                list.clone(),
                1,
                2,
                Status::Active,
                &sk,
            )
            .unwrap();
            let r = bound.status_ref().unwrap();
            assert!(verifies_over(&plain, &commitment_message_with_scheme(&c, &salt, &scheme)));
            assert!(verifies_over(
                &bound,
                &commitment_message_with_status_and_scheme(&c, &salt, &r, &scheme)
            ));
            assert!(!verifies_over(&plain, &commitment_message_with_salt(&c, &salt)));
            assert!(!verifies_over(&bound, &commitment_message_with_status(&c, &salt, &r)));
        }
    }

    const LEGACY_FIXTURE: &str =
        include_str!("../tests/fixtures/legacy_registry/base-5e7e008e.nq");

    /// The entries of the base-issuance fixture, by the `# name` line heading
    /// each block, decoded through [`read_registry`].
    fn legacy_fixture() -> Vec<(String, RegistryEntry)> {
        let mut out = Vec::new();
        for block in LEGACY_FIXTURE.split("# ").filter(|b| !b.trim().is_empty()) {
            let (name, quads) = block.split_once('\n').unwrap();
            let g = Graph::load_dataset(quads, "nquads").unwrap();
            let mut entries = read_registry(&g);
            assert_eq!(entries.len(), 1, "{name}");
            out.push((name.trim().to_string(), entries.remove(0)));
        }
        assert_eq!(out.len(), 4);
        out
    }

    fn legacy_fixture_entry(name: &str) -> RegistryEntry {
        legacy_fixture().into_iter().find(|(n, _)| n == name).unwrap().1
    }

    /// Every entry signed by the issuance path before the signature covered
    /// the method fails verification: the `dual-leaf` v1 entries as issued and
    /// relabelled `string-canonical`, and the honest `string-canonical` entries
    /// under every scheme. Each fixture signature is still a valid signature over
    /// the method-independent message, so it is the refusal of that message, not
    /// a corrupt fixture, that makes them fail.
    #[test]
    fn every_entry_signed_before_the_method_binding_fails_verification() {
        use crate::sig::{commitment_message_with_salt, commitment_message_with_status};
        let fixture = legacy_fixture();
        for (name, e) in &fixture {
            let m = match e.status_ref() {
                None => commitment_message_with_salt(&e.commitment, &e.salt),
                Some(r) => commitment_message_with_status(&e.commitment, &e.salt, &r),
            };
            assert!(
                crate::sig::verify_commitment_with_scheme(
                    e.cryptosuite.as_ref().unwrap().as_str(),
                    e.issuer_public_key.as_ref().unwrap(),
                    &m,
                    e.commitment_signature.as_ref().unwrap(),
                ),
                "{name}: fixture signature is valid over the legacy message"
            );
            for iri in every_scheme_iri() {
                let r = with_scheme_rewritten_in_store(e, iri);
                let ok = if e.status_ref().is_some() {
                    r.verify_commitment_signature_with_status()
                } else {
                    r.verify_commitment_signature()
                };
                assert!(!ok, "{name} recorded as {iri} must not verify");
            }
        }
        // The cases named explicitly, on the entries exactly as decoded.
        assert!(!legacy_fixture_entry("dual-leaf-v1-plain").verify_commitment_signature());
        assert!(!legacy_fixture_entry("dual-leaf-v1-status")
            .verify_commitment_signature_with_status());
        assert!(!legacy_fixture_entry("string-canonical-plain").verify_commitment_signature());
        assert!(!legacy_fixture_entry("string-canonical-status")
            .verify_commitment_signature_with_status());
        assert!(!with_scheme_rewritten_in_store(
            &legacy_fixture_entry("dual-leaf-v1-plain"),
            ZK_SCHEME_POSEIDON2_RDFC10_V1
        )
        .verify_commitment_signature());
    }

    #[test]
    fn unknown_scheme_iri_yields_no_method_fail_closed() {
        // An entry recording a scheme this build does not understand maps to None
        // (fail-closed) — a verifier must reject it, not assume a default.
        let mut e = RegistryEntry::new(
            NamedNode::new("https://dmv.example/vc/m-bogus").unwrap(),
            Fr::from(3u64),
            salt_from_bytes(&[4u8; 32]),
        );
        e.scheme = NamedNode::new_unchecked("https://sparq.dev/ns/zk#poseidon2-bogus-v9");
        assert_eq!(e.method(), None);
        // ...and an entry under an unrecognised scheme does not verify, even with
        // an otherwise valid signature: the signed message cannot be recomputed.
        let sk = crate::sig::SecretKey::from_seed(13);
        let mut signed = RegistryEntry::issued(
            NamedNode::new("https://dmv.example/vc/m-bogus-signed").unwrap(),
            Fr::from(3u64),
            salt_from_bytes(&[4u8; 32]),
            crate::commit::CommitmentMethod::StringCanonicalV1,
            &sk,
        )
        .unwrap();
        assert!(signed.verify_commitment_signature());
        signed.scheme = NamedNode::new_unchecked("https://sparq.dev/ns/zk#poseidon2-bogus-v9");
        assert!(!signed.verify_commitment_signature());
    }
}
