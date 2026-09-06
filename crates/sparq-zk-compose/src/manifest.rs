// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! The proof manifest: the public, serializable description of a query-result
//! proof (plan v3 §S2.5 / §S4.E module (iii)).
//!
//! A manifest travels with the bb proof bytes and is everything a verifier
//! needs (besides the proof) to re-derive the circuit-family id, re-check the
//! sparq-zk obligations, and reconstruct the public-input vector the circuit
//! was proved against. It deliberately carries NO witnesses — only public
//! inputs and the metadata that binds them to issuers, an entailment regime,
//! and a freshness challenge.
//!
//! Credential model (named-graph): the proven content lives in a named graph;
//! the manifest is the proof's metadata graph (plan §S4.E). `did:key` issuer
//! refs and the status-list placeholder mirror the W3C VC data model so a
//! manifest can later be lifted into a `DataIntegrityProof`-shaped credential
//! without a schema change.

use serde::{Deserialize, Serialize};
use sparq_zk::field::{field_from_hex_str, field_to_hex, Fr};
// [OPUS-4.8] sq-h8rg (HolderPoP T2): the holder-key digest from the T1 message
// family (`commitment_message_with_holder`) is computed via `holder_key_digest`;
// the attested-holder-binding schema below wires it through the manifest.
use sparq_zk::sig::{holder_key_digest, public_key_from_hex, HolderKeyError, PublicKey};

/// Field element rendered as `0x`-prefixed 64-nibble hex — the same
/// representation `<urn:sparq:zk>` registry literals use, so commitments and
/// term encodings round-trip through the manifest verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldHex(pub String);

impl FieldHex {
    pub fn from_field(f: &Fr) -> Self {
        FieldHex(field_to_hex(f))
    }
    /// Parse back to a field element. `None` if the hex is malformed.
    pub fn to_field(&self) -> Option<Fr> {
        field_from_hex_str(&self.0)
    }
}

/// SPARQL numeric comparison operator selector — mirrors the circuit globals
/// `OP_*` in `sparq_zk_compose_core::filter_int` (value = the `op` public
/// input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl FilterOp {
    /// The `op` public-input value the circuit expects.
    pub fn code(self) -> u32 {
        match self {
            FilterOp::Lt => 0,
            FilterOp::Le => 1,
            FilterOp::Gt => 2,
            FilterOp::Ge => 3,
            FilterOp::Eq => 4,
            FilterOp::Ne => 5,
        }
    }
}

/// Entailment regime under which the proof was produced (plan §S2.5
/// with/without-inference). The verifier records which regime it is checking;
/// in v1 only `Simple` is proved (no in-circuit reasoning) — `Rdfs`/`Owl` are
/// placeholders so the field is stable across the inference deliverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntailmentRegime {
    /// No inference: the commitment holds exactly the asserted triples.
    Simple,
    /// RDFS entailment (deferred — placeholder).
    Rdfs,
    /// OWL RL entailment (deferred — placeholder).
    Owl,
}

/// How the proof is bound against replay / holder (plan §S2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum BindingMode {
    /// Verifier-supplied fresh nonce, carried as the circuit's `challenge`
    /// public input (the v1 default).
    Challenge { challenge: FieldHex },
    /// Holder proof-of-possession (sq-cwq): the holder demonstrates possession of
    /// its holder secret key by signing the verifier's `challenge` (its nonce).
    ///
    /// # Verifier contract (fail-closed) — see `crate::verifier::bind_holder_pop`
    /// The verifier requires (a) `holder` to be a member of an EXTERNAL
    /// relying-party holder registry ([`crate::verifier::HolderRegistry`], the
    /// trust anchor — mirrors the issuer key-set `K`), and (b) `pop` to be a valid
    /// signature under `holder`'s key over
    /// [`sparq_zk::sig::holder_pop_message`]`(challenge)`. An absent registry, an
    /// untrusted `holder`, a malformed/unverifiable `pop`, or an unknown
    /// `cryptosuite` all REJECT — there is NO silent-accept path for an
    /// unimplemented/absent PoP (the previous placeholder was accepted as a bare
    /// challenge, which silently waived the holder check).
    ///
    /// # Scope (honest deferral)
    /// This proves possession of a TRUSTED holder key, freshly over the verifier's
    /// nonce (so a captured manifest cannot be replayed by a non-holder). It does
    /// NOT yet bind that key to a SPECIFIC credential — an issuer-attested holder
    /// binding (the issuer signing the holder key into the credential) is deferred;
    /// see the verifier docs. Until then the relying party's holder registry is the
    /// who-may-present anchor.
    // [OPUS-4.8] sq-cwq: holder PoP — implemented (challenge-bound Schnorr),
    // fail-closed; issuer→holder credential binding documented-deferred.
    HolderPop {
        challenge: FieldHex,
        /// The holder's verification key (compressed Baby-JubJub point, hex). Must
        /// be a member of the relying party's [`crate::verifier::HolderRegistry`].
        holder: String,
        /// The holder's signature over
        /// [`sparq_zk::sig::holder_pop_message`]`(challenge)`, hex
        /// (`compressed(R) ‖ s`). Proves possession of the holder secret.
        pop: String,
        /// The PoP signature scheme's `zk:cryptosuite` IRI (`poseidon2-schnorr-v1`
        /// in v1). An unknown cryptosuite is unverifiable => REJECT (fail closed).
        #[serde(default = "default_holder_cryptosuite")]
        cryptosuite: String,
    },
}

fn default_holder_cryptosuite() -> String {
    // The v1 Schnorr-over-Baby-JubJub suite (mirrors the issuer attestation
    // default); kept as a fn so the field is stable across schema versions.
    "https://sparq.dev/ns/zk#poseidon2-schnorr-v1".to_string()
}

impl BindingMode {
    pub fn challenge(&self) -> &FieldHex {
        match self {
            BindingMode::Challenge { challenge } => challenge,
            BindingMode::HolderPop { challenge, .. } => challenge,
        }
    }

    /// The disclosed holder verification key of a [`BindingMode::HolderPop`]
    /// binding, parsed (compressed Baby-JubJub point). `None` for a
    /// [`BindingMode::Challenge`] binding or a malformed `holder` hex.
    // [OPUS-4.8] sq-h8rg (HolderPoP T2): expose the presented holder key.
    pub fn holder_key(&self) -> Option<PublicKey> {
        match self {
            BindingMode::HolderPop { holder, .. } => public_key_from_hex(holder),
            BindingMode::Challenge { .. } => None,
        }
    }

    /// The [`sparq_zk::sig::holder_key_digest`] of the disclosed
    /// [`BindingMode::HolderPop`] holder key — the T1 digest wired through the
    /// binding path so a verifier (T3/sq-z8s7) can cross-check the PRESENTED key
    /// against a [`CommitmentAttestation`]'s issuer-attested
    /// [`AttestedHolderBinding::holder_pk_digest`]. `None` for a
    /// [`BindingMode::Challenge`] binding, a malformed key, or the identity key
    /// (fail-closed — [`HolderKeyError::IdentityKey`]).
    ///
    /// This is wiring only; the actual fail-closed equality gate lives in T3.
    // [OPUS-4.8] sq-h8rg (HolderPoP T2): digest of the presented holder key.
    pub fn holder_key_digest(&self) -> Option<Fr> {
        self.holder_key()
            .as_ref()
            .and_then(|pk| holder_key_digest(pk).ok())
    }
}

/// An issuer attestation over one per-graph commitment `C(G)` (audit #3): the
/// commitment value, the issuer's public key, and the issuer's signature over
/// the domain-separated commitment message. The verifier checks (a) the
/// signature is valid under `issuer_public_key`, and (b) `issuer_public_key` is
/// a member of the manifest's disclosed `key_set` K — so `commitments[]` is no
/// longer an unsigned prover-chosen public input. Every scan sub-proof's
/// commitment must have a matching, in-`K` attestation, else the manifest is
/// rejected.
///
/// Privacy note (interim): this discloses WHICH issuer signed each graph (the
/// key is checked in the clear). The full-privacy upgrade verifies the same
/// signature IN-CIRCUIT with an undisclosed signing key + a set-membership
/// gadget over K, revealing only "signed by SOME key in K". See
/// `sparq_zk::sig` module docs.
// [OPUS-4.8] audit #3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitmentAttestation {
    /// `C(G)` — must match a scan sub-proof's `commitments[g]` (and is itself
    /// byte-bound into the bb public inputs by the audit #1 reconstruction).
    pub commitment: FieldHex,
    /// The issuer verification key (compressed Baby-JubJub point, hex). Must be
    /// a member of `ProofManifest::key_set` (audit #3 key-set membership).
    pub issuer_public_key: String,
    /// The issuer's signature, hex. When `salt` is present (audit #9) the signed
    /// message is `commitment_message_with_salt(C(G), salt_G)`; otherwise it is
    /// the bare `commitment_message(C(G))` (audit #3, salt-unbound legacy).
    pub signature: String,
    /// The signature scheme's `zk:cryptosuite` IRI (`poseidon2-schnorr-v1` in
    /// v1). An unknown cryptosuite is unverifiable => the attestation is
    /// rejected (fail closed).
    pub cryptosuite: String,
    /// The per-graph RDFC10 bnode salt this commitment was committed under
    /// (audit #9), hex. When present, the issuer signature is verified over the
    /// SALT-BOUND message so the salt is issuer-attested (a salt-reusing
    /// ingester cannot present a graph under a salt the issuer did not sign), and
    /// the verifier additionally rejects two DISTINCT commitments sharing a salt
    /// (the Q6 cross-graph bnode-correlation channel). Absent => salt-unbound
    /// legacy attestation (audit #3 only); the salt-separation guarantee then
    /// rests on the honest-ingest convention, which the verifier cannot enforce.
    // [OPUS-4.8] audit #9: issuer-attested per-graph salt.
    #[serde(default)]
    pub salt: Option<FieldHex>,
    /// The credential's STATUS-LIST REFERENCE as the issuer signed it (audit
    /// #12): the index + version that, together with `H(status_list)` from the
    /// manifest's [`RevocationStatus`], form the issuer-bound
    /// [`sparq_zk::sig::status_ref_digest`]. When present the issuer signature is
    /// verified over the STATUS-BOUND message
    /// ([`sparq_zk::sig::commitment_message_with_status`]), so the reference is
    /// unforgeable and un-omittable: the verifier recomputes the digest from this
    /// field + the disclosed `RevocationStatus` and the bare signature check
    /// fails if they disagree. A scan-covering attestation MUST carry this
    /// (mandatory / fail-closed — mirrors the codex-2221 salt-mandatory
    /// precedent; an omitted status ref is rejected, never silently un-checked).
    /// Absent => status-unbound (audit #3/#9 only) attestation, NOT accepted on
    /// the scan-verify path.
    // [OPUS-4.8] audit #12: issuer-attested status-list reference.
    #[serde(default)]
    pub status: Option<AttestedStatusRef>,
    /// The ISSUER-ATTESTED holder binding (sq-h8rg, HolderPoP T2): when present,
    /// the issuer signed the HOLDER-bound message variant
    /// ([`sparq_zk::sig::commitment_message_with_holder`], the distinct `ZKSIG_C4`
    /// domain tag) instead of the status-only message — so the credential carries
    /// a cryptographic fact tying THIS commitment to a SPECIFIC holder key `H`
    /// (the [`AttestedHolderBinding::holder_pk_digest`]). This is the strict
    /// analogue of [`Self::status`] (audit #12): exactly one signed-message shape
    /// per attestation, here folding one more field — the holder digest — into the
    /// same Schnorr-signed object. It closes the trusted-holder gap that the
    /// nonce-only [`BindingMode::HolderPop`] left open (it bound the *nonce*, not
    /// the *credential*; see `research/zk-holder-pop-design.md` §0).
    ///
    /// Absent => a NON-holder-bound (bearer / status-only audit #3/#9/#12)
    /// attestation — still valid; this field is PURELY ADDITIVE.
    ///
    /// # Verifier enforcement is T3 (sq-z8s7), NOT here
    /// This T2 deliverable adds the SCHEMA and wires the digest through
    /// construction/parse. The fail-closed verifier gates — requiring a
    /// `holder-pop` presentation's disclosed key to match this attested digest,
    /// requiring the issuer signature to verify over `commitment_message_with_holder`,
    /// and refusing to honour a holder-pop claim over a bearer attestation
    /// (`HolderBindingMissing`/`HolderKeyMismatch`) — are sq-z8s7 (T3). Until then
    /// the presence of this field changes no verifier decision.
    // [OPUS-4.8] sq-h8rg (HolderPoP T2): issuer-attested holder binding (schema +
    // digest wiring; verifier enforcement deferred to T3/sq-z8s7).
    #[serde(default)]
    pub holder: Option<AttestedHolderBinding>,
}

/// The index + version of a credential's status-list reference as bound under
/// the issuer signature (audit #12). The list IRI is the manifest's
/// [`RevocationStatus::status_list`]; the verifier hashes it
/// ([`sparq_zk::sig::status_list_id_to_field`]) and folds it with these into
/// [`sparq_zk::sig::status_ref_digest`] to recompute the signed message. Carried
/// in the attestation (not just `RevocationStatus`) so the issuer-signed values
/// and the disclosed reference are cross-checked for equality — a prover that
/// disclosed a different index/version than the issuer signed is rejected.
// [OPUS-4.8] audit #12.
// [OPUS-4.8] sq-ayv: `index` is now OPTIONAL — a committed-index attestation
// (the index-hiding path) signs an `index_commitment` instead of the clear index,
// so the clear index is absent from the signed object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestedStatusRef {
    /// The credential's CLEAR index into the status list (as issuer-signed) — the
    /// audit-#12 clear-index path. `None` when the attestation uses the sq-ayv
    /// COMMITTED-index path (`index_commitment` is `Some` instead), which signs a
    /// hiding commitment to the index rather than the clear index. Exactly one of
    /// `index` / `index_commitment` is `Some`; both-None or both-Some is rejected
    /// fail-closed by the verifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u64>,
    /// The status-list version (as issuer-signed). `None` ONLY on the sq-kndw
    /// FULLY-HIDDEN path, where the version is folded into `ref_commitment` and is
    /// therefore absent from the signed object as well as from the manifest. On the
    /// clear-index and committed-index paths this is MANDATORY — a `None` version
    /// there is rejected fail-closed ([`crate::verifier::CheckError::RevocationReferenceModeInvalid`]),
    /// never silently defaulted to 0.
    // [OPUS-5] sq-kndw: version withholdable on the fully-hidden path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// [OPUS-4.8] sq-ayv: the hiding COMMITMENT to the index the issuer signed (in
    /// place of the clear `index`), hex. When `Some`, the issuer signed
    /// `status_ref_commit_digest(H(list), index_commitment, version)` and the clear
    /// index is withheld; the hidden-index revocation proof cross-binds this
    /// commitment to the proven-unset index in-circuit. `None` for the clear-index
    /// (audit #12) path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_commitment: Option<FieldHex>,
    /// [OPUS-5] sq-kndw: the hiding COMMITMENT to the `(list IRI, version)` PAIR the
    /// issuer signed, hex — [`sparq_zk::sig::status_ref_commitment`]`(H(list),
    /// version, ref_blinding)`. When `Some`, the issuer signed the FULLY-COMMITTED
    /// digest [`sparq_zk::sig::status_ref_fully_committed_digest`]`(ref_commitment,
    /// index_commitment)`, which folds NEITHER the clear list id NOR the clear
    /// version — so the signed object discloses nothing about which list or which
    /// publication epoch the credential belongs to. `None` for the clear-index and
    /// committed-index paths.
    ///
    /// Requires `index_commitment` to be `Some` and `index` / `version` to be
    /// `None`; any other combination is a malformed mode and is rejected
    /// fail-closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_commitment: Option<FieldHex>,
}

impl AttestedStatusRef {
    /// The CLEAR-index attested reference (audit #12). The issuer signs
    /// [`sparq_zk::sig::status_ref_digest`]`(H(list), index, version)`.
    pub fn clear(index: u64, version: u64) -> Self {
        AttestedStatusRef {
            index: Some(index),
            version: Some(version),
            index_commitment: None,
            ref_commitment: None,
        }
    }

    /// The COMMITTED-index attested reference (sq-ayv). The issuer signs
    /// [`sparq_zk::sig::status_ref_commit_digest`]`(H(list), index_commitment,
    /// version)`; `index_commitment` must be
    /// [`sparq_zk::sig::status_index_commitment`]`(index, blinding)`.
    pub fn committed(index_commitment: &sparq_zk::Fr, version: u64) -> Self {
        AttestedStatusRef {
            index: None,
            version: Some(version),
            index_commitment: Some(FieldHex::from_field(index_commitment)),
            ref_commitment: None,
        }
    }

    /// [OPUS-5] sq-kndw: the FULLY-HIDDEN attested reference. The issuer signs
    /// [`sparq_zk::sig::status_ref_fully_committed_digest`]`(ref_commitment,
    /// index_commitment)` — no clear list id, no clear version. Build the inputs
    /// with [`sparq_zk::sig::status_ref_commitment`]`(H(list), version,
    /// ref_blinding)` and [`sparq_zk::sig::status_index_commitment`]`(index,
    /// blinding)`.
    ///
    /// ⚠️ Both blindings MUST be freshly sampled per presentation and the
    /// credential re-signed; a reused pair is a cross-presentation correlation
    /// handle (see [`FullyHiddenRevocation`]).
    pub fn fully_hidden(ref_commitment: &sparq_zk::Fr, index_commitment: &sparq_zk::Fr) -> Self {
        AttestedStatusRef {
            index: None,
            version: None,
            index_commitment: Some(FieldHex::from_field(index_commitment)),
            ref_commitment: Some(FieldHex::from_field(ref_commitment)),
        }
    }
}

/// The ISSUER-ATTESTED holder binding carried by a [`CommitmentAttestation`]
/// (sq-h8rg, HolderPoP T2). It records the holder public key the issuer bound
/// into THIS credential, as the issuer signed it: the
/// [`Self::holder_pk_digest`] = [`sparq_zk::sig::holder_key_digest`]`(hpk)` that
/// the issuer folded into [`sparq_zk::sig::commitment_message_with_holder`] (the
/// `ZKSIG_C4` message variant). It is the strict analogue of
/// [`AttestedStatusRef`] (audit #12): the manifest carries the issuer-signed
/// value so the verifier can recompute the holder-bound signed message and
/// cross-check the presented holder key against it.
///
/// # Construction (digest wiring from T1)
/// Build it from a holder [`PublicKey`] with [`Self::from_holder_key`], which
/// computes the digest via [`sparq_zk::sig::holder_key_digest`] (the SINGLE
/// source of truth, shared with the issuer's
/// [`sparq_zk::sig::SecretKey::sign_commitment_with_holder`] and, in T3, the
/// verifier) — so the digest the manifest carries is byte-for-byte the one the
/// issuer signed. The identity holder key is rejected fail-closed
/// ([`HolderKeyError::IdentityKey`]), exactly as `holder_key_digest` does.
///
/// # Clear vs hidden tier
/// [`Self::holder_pk_digest`] is the only MANDATORY field — it is sufficient for
/// the hidden-key tier (design §2.B-B2), where the clear `hpk` is never
/// disclosed and only the digest is public. The optional [`Self::holder_public_key`]
/// carries the clear `hpk` hex for the clear-key tier (design §2.B-B1), where the
/// verifier recomputes the digest from the disclosed key and cross-checks it.
/// When `holder_public_key` is present, [`Self::from_holder_key`] guarantees it
/// is consistent with `holder_pk_digest`.
///
/// # Verifier enforcement is T3 (sq-z8s7)
/// This struct is the SCHEMA only. The fail-closed cross-checks (disclosed key
/// vs digest; issuer signature over `commitment_message_with_holder`) are T3.
/// See `research/zk-holder-pop-design.md` §3.2 / §6 step 2 (this) and step 3 (T3).
// [OPUS-4.8] sq-h8rg (HolderPoP T2): attested-holder-binding schema + digest wiring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestedHolderBinding {
    /// The issuer-attested holder-key DIGEST,
    /// [`sparq_zk::sig::holder_key_digest`]`(hpk) = Poseidon2([ZKSIG_HK, hpk.x, hpk.y])`,
    /// hex. This is the value the issuer folded into
    /// [`sparq_zk::sig::commitment_message_with_holder`]; the verifier (T3) binds
    /// it (clear-tier cross-check, or hidden-tier public input). The mandatory
    /// field of the binding — present in BOTH the clear and the hidden tier.
    pub holder_pk_digest: FieldHex,
    /// OPTIONAL clear holder verification key (compressed Baby-JubJub point, hex)
    /// for the clear-key tier (design §2.B-B1). When present, the verifier (T3)
    /// recomputes [`sparq_zk::sig::holder_key_digest`] from it and requires it to
    /// equal [`Self::holder_pk_digest`]; it is also the key the
    /// [`BindingMode::HolderPop`] PoP is checked under. Absent => the hidden-key
    /// tier (design §2.B-B2): only the digest is public, the clear key never
    /// disclosed (no linkability channel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_public_key: Option<String>,
}

impl AttestedHolderBinding {
    /// Build a binding from a holder [`PublicKey`], wiring the T1
    /// [`sparq_zk::sig::holder_key_digest`] as the single source of the digest.
    /// `disclose_key` selects the tier: `true` records the clear `hpk` hex
    /// ([`Self::holder_public_key`]) for the clear-key tier (design §2.B-B1);
    /// `false` carries only the digest for the hidden-key tier (§2.B-B2).
    ///
    /// Returns [`HolderKeyError::IdentityKey`] for the identity key (fail-closed —
    /// it has no usable digest and is never a valid holder key), exactly as
    /// [`sparq_zk::sig::holder_key_digest`].
    // [OPUS-4.8] sq-h8rg (HolderPoP T2): digest wiring from the holder key.
    pub fn from_holder_key(hpk: &PublicKey, disclose_key: bool) -> Result<Self, HolderKeyError> {
        let digest = holder_key_digest(hpk)?;
        Ok(AttestedHolderBinding {
            holder_pk_digest: FieldHex::from_field(&digest),
            holder_public_key: disclose_key.then(|| sparq_zk::sig::public_key_to_hex(hpk)),
        })
    }

    /// The issuer-attested holder-key digest as a field element. `None` if the
    /// stored hex is malformed (fail-closed — the verifier in T3 treats an
    /// unparseable digest as no valid binding).
    // [OPUS-4.8] sq-h8rg (HolderPoP T2).
    pub fn digest(&self) -> Option<Fr> {
        self.holder_pk_digest.to_field()
    }

    /// The disclosed clear holder key (clear-key tier), parsed. `None` when the
    /// binding is hidden-tier (no clear key) OR the hex is malformed. The T3
    /// verifier cross-checks `holder_key_digest(this) == digest()`.
    // [OPUS-4.8] sq-h8rg (HolderPoP T2).
    pub fn holder_key(&self) -> Option<PublicKey> {
        self.holder_public_key
            .as_deref()
            .and_then(public_key_from_hex)
    }
}

/// A credential's revocation reference (plan §S2.6, Bitstring/StatusList2021
/// shape): which status list tracks the credential's liveness, the credential's
/// index into that list, and the list VERSION the prover is asserting against.
///
/// # Issuer-bound (audit #12, leverages #3)
/// This reference is NOT trusted as a prover claim. The issuer signature (which
/// already binds `C(G)` + salt, audit #3/#9) ALSO binds
/// [`sparq_zk::sig::status_ref_digest`]`(H(status_list), index, version)`
/// ([`CommitmentAttestation::status`]). The verifier recomputes that digest from
/// THIS disclosed reference and requires it to match the issuer-signed value —
/// so a prover cannot omit, forge, or swap the reference (an omitted/forged
/// reference yields no valid issuer signature, fail-closed). The verifier then
/// checks the disclosed status-list snapshot's bit at `index` is UNSET and the
/// `version` is within its freshness window.
///
/// # The three disclosure MODES (exactly one is well-formed)
/// | mode | `status_list` | `index` | `version` | `index_commitment` | `ref_commitment` |
/// |---|---|---|---|---|---|
/// | CLEAR (audit #12)          | `Some` | `Some` | `Some` | `None` | `None` |
/// | COMMITTED-index (sq-ayv)   | `Some` | `None` | `Some` | `Some` | `None` |
/// | FULLY-HIDDEN (sq-kndw)     | `None` | `None` | `None` | `Some` | `Some` |
///
/// Any other combination is a malformed mode and is rejected FAIL-CLOSED by
/// [`crate::verifier`] (`RevocationReferenceModeInvalid`) — the mode is resolved
/// at ONE chokepoint (`resolve_status_ref`) so a new mode cannot bypass the
/// issuer-binding cross-check.
///
/// # Privacy (per mode, honest)
/// - CLEAR: `index` is disclosed (a linkability channel — a relying party can
///   correlate two presentations of the same credential by its list slot).
/// - COMMITTED-index (sq-ayv): index + liveness bit hidden; the status-list IRI
///   and the `version` are STILL disclosed (a coarser correlation channel — which
///   list, which publication epoch).
/// - FULLY-HIDDEN (sq-kndw / sq-6qe): IRI + version hidden too. Nothing
///   holder-identifying is disclosed; the statement reduces to "some accepted
///   `(list, version)` in the relying party's committed set, at or above its
///   public epoch floor, has my hidden index unset". Requires a
///   [`FullyHiddenRevocation`] proof. The residual disclosures are policy-side,
///   not holder-side: the accepted-set root (the RP's own policy fingerprint), the
///   public `min_version` floor, and the member depths `(D, A)` via the vk.
///
///   ⚠️ `ref_commitment` + `index_commitment` are HIDING but STABLE per issuance,
///   so REUSING them across presentations reinstates full linkability and voids
///   the guarantee. See [`FullyHiddenRevocation`] for the enforcement.
///
/// NOT externally audited (sq-qhy4); no soundness / privacy property is asserted
/// as achieved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationStatus {
    /// IRI of the status-list credential (bound under the issuer signature via
    /// [`sparq_zk::sig::status_list_id_to_field`]). `None` ONLY on the sq-kndw
    /// FULLY-HIDDEN path, where the IRI is folded into `ref_commitment` and hence
    /// absent from both the signed object and the manifest. MANDATORY on the clear
    /// and committed-index paths — a `None` IRI there is a malformed mode and is
    /// rejected fail-closed.
    // [OPUS-5] sq-kndw: list IRI withholdable (fully-hidden path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_list: Option<String>,
    /// The CLEAR index into the list (the audit-#12 clear-index path). `None` when
    /// the credential uses the sq-ayv COMMITTED-index path — the clear index is
    /// WITHHELD and `index_commitment` carries a hiding commitment instead, so the
    /// linkability channel is closed. `#[serde(default)]` so a committed-index
    /// manifest simply omits this field.
    ///
    /// # Privacy
    /// When `Some`, `index` is disclosed in the clear (a linkability channel — a
    /// relying party can correlate two presentations by index). The sq-ayv
    /// committed-index path (`index = None`, `index_commitment = Some`) closes
    /// this: revocation is checked via the hidden-index proof against the
    /// authoritative root, and the issuer binds only a hiding commitment to the
    /// index — so neither the index NOR the liveness bit is disclosed.
    // [OPUS-4.8] sq-ayv: clear index withholdable (committed-index path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u64>,
    /// The status-list version the credential asserts against (a monotone
    /// freshness counter — the issuer's status-list publication sequence /
    /// `validFrom` epoch). Bound under the issuer signature and freshness-window
    /// checked by the verifier (audit #12). `#[serde(default)]` keeps old
    /// version-less manifests parseable, but the verifier's status check is
    /// mandatory and a version-0 reference still must match the issuer-signed
    /// digest and a fresh snapshot, so the default does not bypass the gate.
    ///
    /// [OPUS-5] sq-kndw: now `Option<u64>` — `None` on the FULLY-HIDDEN path,
    /// where the version is committed inside `ref_commitment` and proven
    /// `>= min_version` IN-CIRCUIT instead of being disclosed. On the clear and
    /// committed-index paths a `None` version is a malformed mode and is rejected
    /// fail-closed (it is NOT defaulted to 0 — that would let a manifest silently
    /// drop the freshness anchor).
    // [OPUS-4.8] audit #12: issuer-bound, freshness-checked version.
    // [OPUS-5] sq-kndw: version withholdable (fully-hidden path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// [OPUS-4.8] sq-ayv: the hiding COMMITMENT to the index (in place of the clear
    /// `index`), hex. When `Some`, the issuer signed
    /// `status_ref_commit_digest(H(list), index_commitment, version)` and the
    /// verifier (a) recomputes that digest from THIS field to check the signature,
    /// and (b) requires the hidden-index revocation proof's PUBLIC index commitment
    /// to byte-equal it (the cross-binding). `None` for the clear-index path. A
    /// committed-index reference REQUIRES a hidden-revocation proof (revocation is
    /// then checked there, never skipped — fail-closed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_commitment: Option<FieldHex>,
    /// [OPUS-5] sq-kndw: the hiding COMMITMENT to the `(list IRI, version)` pair,
    /// hex — [`sparq_zk::sig::status_ref_commitment`]`(H(list), version,
    /// ref_blinding)`. When `Some` (the FULLY-HIDDEN mode) the verifier
    /// (a) recomputes the issuer-signed digest
    /// [`sparq_zk::sig::status_ref_fully_committed_digest`]`(ref_commitment,
    /// index_commitment)` from THIS field to check the signature, and (b) requires
    /// the fully-hidden revocation proof's PUBLIC `ref_commitment` to byte-equal
    /// it — the cross-binding that ties the in-circuit private `(list, version)` to
    /// the reference the ISSUER signed. Without (b) the in-circuit "ref open"
    /// relation would constrain nothing.
    ///
    /// A fully-hidden reference REQUIRES a [`ProofManifest::fully_hidden_revocation`]
    /// proof (revocation is then checked there, never skipped — fail-closed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_commitment: Option<FieldHex>,
}

impl RevocationStatus {
    /// The CLEAR-index disclosed reference (audit #12) — index and version in the
    /// clear. Pairs with [`AttestedStatusRef::clear`].
    pub fn clear(status_list: impl Into<String>, index: u64, version: u64) -> Self {
        RevocationStatus {
            status_list: Some(status_list.into()),
            index: Some(index),
            version: Some(version),
            index_commitment: None,
            ref_commitment: None,
        }
    }

    /// The COMMITTED-index disclosed reference (sq-ayv) — the clear index is
    /// withheld; the list IRI and version are still disclosed. Pairs with
    /// [`AttestedStatusRef::committed`], and REQUIRES a
    /// [`ProofManifest::hidden_revocation`] proof.
    pub fn committed(
        status_list: impl Into<String>,
        index_commitment: &sparq_zk::Fr,
        version: u64,
    ) -> Self {
        RevocationStatus {
            status_list: Some(status_list.into()),
            index: None,
            version: Some(version),
            index_commitment: Some(FieldHex::from_field(index_commitment)),
            ref_commitment: None,
        }
    }

    /// [OPUS-5] sq-kndw: the FULLY-HIDDEN disclosed reference — no list IRI, no
    /// index, no version; only the two hiding commitments the issuer signed. Pairs
    /// with [`AttestedStatusRef::fully_hidden`], and REQUIRES a
    /// [`ProofManifest::fully_hidden_revocation`] proof (liveness is decided there,
    /// never skipped).
    pub fn fully_hidden(ref_commitment: &sparq_zk::Fr, index_commitment: &sparq_zk::Fr) -> Self {
        RevocationStatus {
            status_list: None,
            index: None,
            version: None,
            index_commitment: Some(FieldHex::from_field(index_commitment)),
            ref_commitment: Some(FieldHex::from_field(ref_commitment)),
        }
    }
}

/// A snapshot of a Bitstring/StatusList2021-style status list (audit #12): the
/// list IRI, its version, and its status bitstring. The credential's liveness is
/// `bit[index] == 0` (unset = active; set = revoked/suspended).
///
/// # Two roles — and which one is authoritative (re-audit Option B)
/// This type is used in TWO places:
/// 1. The relying party's AUTHORITATIVE snapshot, carried externally in
///    `verifier::RevocationPolicy` (resolved + authenticated by the relying party
///    out of band, like the trusted key-set `K`). The liveness BIT decision reads
///    from HERE.
/// 2. The prover's `ProofManifest::status_snapshots` — UNAUTHENTICATED
///    prover-supplied bytes. The issuer signature binds the status-list REFERENCE
///    (`status_ref_digest(H(list IRI), index, version)`) but NOT the bit VALUES,
///    so the prover's bitstring is NOT trusted for the bit decision. If a prover
///    snapshot is present for the referenced `(list, version)` the verifier only
///    requires it to byte-equal the authoritative one (a tamper tripwire —
///    `CheckError::StatusSnapshotTampered`); otherwise it is ignored. Reading the
///    bit from the prover's snapshot was the re-audit hole: a genuine reference +
///    a forged all-zero snapshot reverified a REVOKED credential.
///
/// The verifier checks, against the (issuer-bound) [`RevocationStatus`] and its
/// AUTHORITATIVE snapshot: (i) the reference `version` is within the verifier's
/// freshness window, (ii) the AUTHORITATIVE `bit[index]` is UNSET, and (iii) any
/// prover snapshot for `(list, version)` agrees byte-for-byte. A revoked
/// authoritative bit, a stale version, a missing authoritative snapshot, or a
/// disagreeing prover snapshot all REJECT (fail-closed).
///
/// `bits` is the raw status bitstring, LSB-first within each byte (bit `i` is
/// `bits[i / 8] >> (i % 8) & 1`) — the StatusList2021 convention.
// [OPUS-4.8] audit #12: status-list snapshot (authoritative copy lives in the policy).
// [OPUS-4.8] audit #12 re-audit: prover's manifest copy is NOT the bit-decision source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusListSnapshot {
    /// IRI of the status list this snapshot is for (must equal the credential's
    /// `RevocationStatus::status_list`).
    pub status_list: String,
    /// The snapshot's version (must equal the credential's
    /// `RevocationStatus::version` and be within the verifier freshness window).
    pub version: u64,
    /// The raw status bitstring, LSB-first within each byte.
    pub bits: Vec<u8>,
}

impl StatusListSnapshot {
    /// The status bit at `index` (LSB-first within each byte). An out-of-range
    /// index reads as SET (revoked) — fail-closed: a credential whose index
    /// falls outside the disclosed snapshot is treated as not-proven-live.
    // [OPUS-4.8] audit #12: out-of-range reads as revoked (fail closed).
    pub fn bit(&self, index: u64) -> bool {
        let byte = (index / 8) as usize;
        let off = (index % 8) as u32;
        match self.bits.get(byte) {
            Some(b) => (b >> off) & 1 == 1,
            None => true,
        }
    }
}

/// The circuit-family id: which compiled member of the `zk/compose/` family
/// the proof was produced by. Prover and verifier BOTH derive this from the
/// manifest shape (the (k, n) lattice id), so a proof can only verify against
/// the member its public inputs fit (plan Q5 / brief: "derive the circuit-
/// family id from the proof manifest").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CircuitId {
    /// `scan_k{k}_n{n}_r{r}` — a BGP triple-pattern scan over `k` graphs,
    /// `n` slots/graph, `r` disclosed rows.
    Scan { k: u32, n: u32, r: u32 },
    /// `filter_int_d{d}` — hidden xsd:integer FILTER with `d` decimal digits.
    FilterInt { d: u32 },
    /// `filter_f64_d{d}` — MANIFEST-COMPOSABLE hidden xsd:double FILTER with `d`
    /// decimal digits, over the INTEGER-VALUED double fragment ([OPUS-4.8] sq-q7e
    /// / sq-tat). The hidden operand is bound to the committed literal via the
    /// canonical `"<digits>"^^<…#double>` token (blake3, the same mechanism as
    /// `filter_int`), and the IEEE bits are DERIVED in-circuit from the bound
    /// value (`f64::from(value)`), so there is no prover-free `a_bits`. The raw
    /// `filter_f64` building block (free bits) remains for non-composed use; this
    /// `{d}`-carrying id is the composable member. Fragment scope: plain
    /// integer-valued doubles (`"42"^^xsd:double`); fractional/scientific forms
    /// are deferred (the in-circuit decimal→IEEE parser is unbudgeted). See
    /// `sparq_zk_compose_core::filter_float::filter_f64_composable_check`.
    // [OPUS-4.8] sq-q7e + sq-tat: FilterF64 is now manifest-composable (carries d).
    FilterF64 { d: u32 },
    /// `filter_signed_int_d{md}` — MANIFEST-COMPOSABLE hidden SIGNED xsd:integer
    /// FILTER with `md` MAGNITUDE digits (the optional leading `-` is bound into the
    /// canonical token, not counted in `md`) ([OPUS-4.8] sq-7lrq, the sq-1q9h
    /// member). Extends [`Self::FilterInt`] (canonical NON-negative only) to negative
    /// coordinates / amounts: the hidden operand is bound to the committed literal
    /// via the canonical `"[-]?<digits>"^^<…#integer>` token (blake3, the same
    /// mechanism as `filter_int`), and the sign-aware comparison runs over the `u64`
    /// magnitude (`sparq_zk_compose_core::filter_signed::filter_signed_int_check`).
    /// `md` selects the compiled member exactly as `filter_int`'s `d` does (the
    /// `mag_digits: [u8; MD]` witness pins the operand's magnitude-digit count to
    /// `MD`), so a `md`-magnitude-digit operand is provable ONLY by the `MD == md`
    /// member — the same EXACT-match discipline (sq-wto) the filter_int / filter_f64
    /// families use.
    // [OPUS-4.8] sq-7lrq: signed xsd:integer FILTER is now manifest-composable.
    FilterSignedInt { md: u32 },
    /// `filter_decimal_i{id}_f{fd}` — MANIFEST-COMPOSABLE hidden xsd:decimal FILTER
    /// with `id` integer-part digits and `fd` fraction digits (e.g. `"123.45"` =>
    /// `id=3`, `fd=2`) ([OPUS-4.8] sq-7lrq, the sq-1q9h member). The hidden operand
    /// is bound to the committed literal via the canonical
    /// `"[-]?<int>.<frac>"^^<…#decimal>` token (blake3, the same mechanism as
    /// `filter_int`), and the comparison is fixed-point at `fd` places against a
    /// HOST-PRESCALED constant (`bound_scaled = round(|bound| * 10^fd)` carried as a
    /// public input) — `sparq_zk_compose_core::filter_signed::filter_decimal_check`.
    /// `(id, fd)` selects the compiled member: the `int_digits: [u8; ID]` /
    /// `frac_digits: [u8; FD]` witnesses pin the operand's integer-digit AND
    /// fraction-digit counts to `(ID, FD)` exactly, so an operand is provable ONLY by
    /// the member whose `(ID, FD)` equals its `(id, fd)` (EXACT-match discipline,
    /// sq-wto). The general fractional/scientific xsd:double fragment (an in-circuit
    /// decimal→IEEE RNE parser over an arbitrary lexical form) remains DEFERRED — see
    /// `filter_float.nr` and the README "sparq-zk API gaps"; this member is the
    /// fixed-point decimal fragment, not that parser.
    // [OPUS-4.8] sq-7lrq: xsd:decimal FILTER is now manifest-composable.
    FilterDecimal { id: u32, fd: u32 },
    /// `filter_value_dl_int` — DUAL-LEAF value-lane FILTER over a committed
    /// NON-NEGATIVE `xsd:integer` ([OPUS-4.8] sq-xojl). Unlike the blake3-bound
    /// `FilterInt` family (which re-hashes the canonical token in-circuit), this
    /// member binds the operand to the DUAL-LEAF commitment via two Poseidon2
    /// permutations over the witnessed `VALUE_HOOK` and carries
    /// `lexical_component` as a FREE witness — NO in-circuit blake3 (the measured
    /// gate win: 3033 vs 17416, `gate_count_snapshot.json`). It is DIGIT-COUNT-FREE
    /// (no `[u8; D]` witness), so the per-`d` family collapses to ONE member per
    /// datatype class and the member selection no longer leaks `ceil(log10(value))`.
    ///
    /// LEGAL ONLY against the `DualLeafV1` commitment method (and the
    /// feature-gated `ValueOnlyV1` research dial) — a graph committed
    /// `string-canonical` has no `value_component`, so this member is unprovable
    /// against it. The `(method, circuit)` legality is enforced FAIL-CLOSED by
    /// [`crate::dispatch`] (sq-cfmv).
    ///
    /// DOCUMENTED RISK: this member carries the INV-VL downgrade — value↔lexical
    /// agreement on the value-FILTER lane is TRUSTED-ISSUER-HONESTY, not
    /// machine-enforced (#769 accepted at research grade; gap CR-G8 / sq-qhy4).
    /// The whole ZK estate is NOT externally audited; no soundness / privacy claim.
    // [OPUS-4.8] sq-xojl: dual-leaf value-lane FILTER member. Opt-in (`dual-leaf`
    // feature), research-grade, NOT externally audited.
    #[cfg(feature = "dual-leaf")]
    FilterValueDl,
    /// `filter_value_dl_f64` — DUAL-LEAF value-lane FILTER over a committed
    /// `xsd:double` ([OPUS-4.8] sq-2ezsx, the double sibling of [`Self::FilterValueDl`]).
    /// Like the integer member it binds the operand via two Poseidon2 permutations
    /// over the witnessed `VALUE_HOOK` (here the IEEE-754 bit pattern) with NO
    /// in-circuit blake3, but — because `xsd:double` is MANY-TO-ONE on the term
    /// (`-0.0`/`+0.0`, NaN payloads) — it instantiates B4 IN-CIRCUIT: it
    /// CANONICALISES the IEEE bits (`-0.0` → `+0.0`; any NaN → the canonical qNaN)
    /// before forming `value_component`, so two bit-distinct but
    /// SPARQL-numerically-equal terms collapse to ONE value handle. DIGIT-COUNT-FREE
    /// (one member per datatype class). LEGAL ONLY against `DualLeafV1` / the
    /// `ValueOnlyV1` research dial — fail-closed via `crate::dispatch`.
    ///
    /// DOCUMENTED RISK: carries the INV-VL downgrade (value↔lexical agreement is
    /// trusted-issuer-honesty, not machine-enforced; #769 accepted, CR-G8 / sq-qhy4).
    /// NOT externally audited; no soundness / privacy claim.
    // [OPUS-4.8] sq-2ezsx: dual-leaf value-lane double FILTER member. Opt-in
    // (`dual-leaf`), research-grade, NOT externally audited.
    #[cfg(feature = "dual-leaf")]
    FilterValueDlF64,
    /// `filter_value_dl_decimal` — DUAL-LEAF value-lane FILTER over a committed
    /// `xsd:decimal` at a fixed canonical scale ([OPUS-4.8] sq-2ezsx, the decimal
    /// sibling of [`Self::FilterValueDl`]). Binds the operand via two Poseidon2
    /// permutations over the witnessed `VALUE_HOOK` (here the SIGNED scaled
    /// magnitude) with NO in-circuit blake3. Because `xsd:decimal` is MANY-TO-ONE on
    /// the term at a fixed scale (`"5.0"` == `"5.00"`), B4 is the canonical-SCALE
    /// bind: the value handle is the magnitude at exactly the canonical fraction
    /// width, and that scale is folded into the PUBLIC `datatype_const`
    /// (`sparq_zk::dual_leaf::decimal_datatype_const`) so a value at one scale can
    /// never collide a value at another. DIGIT-COUNT-FREE AND scale-agnostic (ONE
    /// compiled member serves every scale — the scale lives in the public input),
    /// unlike the blake3 `FilterDecimal { id, fd }` family. LEGAL ONLY against
    /// `DualLeafV1` / `ValueOnlyV1` — fail-closed via `crate::dispatch`.
    ///
    /// DOCUMENTED RISK: carries the INV-VL downgrade (CR-G8 / sq-qhy4). NOT
    /// externally audited; no soundness / privacy claim.
    // [OPUS-4.8] sq-2ezsx: dual-leaf value-lane decimal FILTER member. Opt-in
    // (`dual-leaf`), research-grade, NOT externally audited.
    #[cfg(feature = "dual-leaf")]
    FilterValueDlDecimal,
    /// `filter_value_dl_datetime` — DUAL-LEAF value-lane FILTER over a committed
    /// `xsd:dateTime` OR `xsd:date` ([OPUS-5] sq-wz99x, the dateTime/date sibling
    /// of [`Self::FilterValueDl`]; `research/zk-field-native-encoding.md` §13.5).
    /// Structurally [`Self::FilterValueDlDecimal`] with a different value handle:
    /// the `VALUE_HOOK` is the SIGNED SCALED EPOCH (milliseconds from
    /// `1970-01-01T00:00:00Z` on the XSD proleptic-Gregorian `timeOnTimeline`, at
    /// the lane-fixed sub-second scale `FS = 3`), so it reuses the member's
    /// UNCHANGED signed fixed-point verdict with NO in-circuit blake3.
    ///
    /// ONE compiled member serves BOTH datatype classes. `xsd:date` is its own
    /// lane only by its PUBLIC `datatype_const`
    /// (`sparq_zk::dual_leaf_datetime::{datetime_datatype_const,
    /// date_datatype_const}` — `blake3(IRI ‖ "@epochscale=3")`), which is folded
    /// into `value_component`; a date's hook is the scaled epoch of its STARTING
    /// instant and is therefore numerically EQUAL to the dateTime hook of that same
    /// instant, so the lane constant is the only thing keeping the two apart (an
    /// honest date witness rebinds to a different leaf under the dateTime constant
    /// and fails the member's leaf assert). That is a BINDING argument resting on
    /// Poseidon2 preimage resistance, NOT an audited soundness claim.
    ///
    /// The hookable DOMAIN (strict XSD-canonical `Z`-timezoned lexicals only) is
    /// the host's §13.4 fail-closed predicate, not an in-circuit check — the same
    /// division of labour the decimal member uses for its canonical fraction width.
    /// LEGAL ONLY against `DualLeafV1` / `ValueOnlyV1` — fail-closed via
    /// `crate::dispatch`.
    ///
    /// DOCUMENTED RISK: carries the INV-VL downgrade, and the whole §13 rule set is
    /// itself an OPEN external-audit obligation (CR-G8 / sq-qhy4). NOT externally
    /// audited; no soundness / privacy claim.
    // [OPUS-5] sq-wz99x: dual-leaf value-lane dateTime/date FILTER member. Opt-in
    // (`dual-leaf`), research-grade, NOT externally audited.
    #[cfg(feature = "dual-leaf")]
    FilterValueDlDateTime,
    /// `revoke_unset_d{depth}` — hidden-index status-list inclusion + bit-unset
    /// proof over a depth-`depth` Poseidon2 Merkle tree (sq-3e5 / sq-h2v). The
    /// proof's PUBLIC inputs are `challenge` + the status-list Merkle `root`; the
    /// holder's index, the leaf bit, and the authentication path are PRIVATE, so
    /// the proof proves "the bit at my hidden index is unset" without disclosing
    /// the index (the clear-index linkability channel the verifier-side
    /// [`RevocationStatus`] check leaked). Supports lists up to `2^depth` indices.
    // [OPUS-4.8] sq-3e5 + sq-h2v: hidden-index revocation circuit member.
    RevokeUnset { depth: u32 },
    /// `revoke_hidden_ref_d{depth}_a{set_depth}` — the FULLY-HIDDEN revocation
    /// member (sq-kndw, the deferred remainder of sq-6qe;
    /// `research/zk-statuslist-hide-iri-version.md` §3 sub-option A). The privacy
    /// upgrade over [`Self::RevokeUnset`]: it hides the status-list IRI and the
    /// VERSION on top of the index and the liveness bit.
    ///
    /// The proof's PUBLIC inputs are `challenge`, `ref_commitment`,
    /// `index_commitment`, `accepted_set_root` and `min_version`. The list id, the
    /// version, both blindings, the status-list Merkle root, the accepted-set slot
    /// and path, the holder's index, the leaf bit and the status-list path are all
    /// PRIVATE. The relying party's `status_list_root` is bound PRIVATELY, inside
    /// the accepted-set leaf `Poseidon2([ZKSIG_AL, list_id, version,
    /// status_list_root])`, so it never has to name the snapshot to check the fold.
    ///
    /// `depth` is the status-list tree depth (`≤ 2^depth` indices, as
    /// [`Self::RevokeUnset`]); `set_depth` is the accepted-set tree depth
    /// (`≤ 2^set_depth` accepted `(list, version)` pairs). Both are disclosed by
    /// the member name / vk — a cardinality bound, inherent to fixed-depth Merkle.
    /// [`crate::build::derive_revoke_hidden_ref_id`] is the single source of the
    /// compiled family list; a `(depth, set_depth)` outside it derives `None`
    /// (fail-closed, no wrong-bucket fallback).
    ///
    /// Opt-in and research-grade: NOT externally audited (sq-qhy4); no soundness /
    /// ZK-privacy property is asserted as achieved.
    // [OPUS-5] sq-kndw: fully-hidden revocation circuit member.
    RevokeHiddenRef { depth: u32, set_depth: u32 },
    /// `hidden_issuer_d{depth}` — in-circuit Schnorr-over-Baby-JubJub signature
    /// verification + hidden-key set membership over a depth-`depth` Poseidon2
    /// Merkle tree of the issuer key set K (sq-z9l). The proof's PUBLIC inputs are
    /// `challenge` + the commitment message `m` + the key-set Merkle `key_set_root`;
    /// the issuer public key, the signature `(R, s)`, the challenge-reduction
    /// witness, and the membership index/path are PRIVATE, so the proof proves
    /// "this commitment was signed by SOME key in K" without disclosing WHICH
    /// issuer. The privacy upgrade over the clear-key
    /// `crate::verifier::bind_issuer_attestations` check.
    // [OPUS-4.8] sq-z9l: hidden-issuer-attestation circuit member.
    HiddenIssuer { depth: u32 },
    /// `holder_pok` — in-circuit holder Proof-of-Possession (sq-xqfg, HolderPoP
    /// T5, the B2 hidden-key tier). The proof's PUBLIC inputs are `challenge` +
    /// the issuer-attested `holder_pk_digest`; the holder secret `hsk` and the
    /// holder public key `(hpk_x, hpk_y)` are PRIVATE, so the proof proves
    /// "I possess the holder secret whose key the issuer bound into this
    /// credential" without disclosing the secret OR the key. The relation is
    /// `hpk = hsk·G` (Baby-JubJub, ONE scalar-mul — cheaper than the
    /// `HiddenIssuer` Schnorr's two) AND
    /// `Poseidon2([ZKSIG_HK, hpk.x, hpk.y]) == holder_pk_digest`, reusing
    /// `issuer.nr`'s scalar-mul / on-curve / `< L` gadgets verbatim. A single
    /// depth-free member (no Merkle parameterisation — the clear-digest tier).
    /// The verifier gate that binds `holder_pk_digest` to the issuer attestation
    /// is `bind_holder_pok` (T6/sq-i1dt), SEPARATE from this member registration.
    // [OPUS-4.8] sq-xqfg (HolderPoP T5): in-circuit holder-PoK circuit member.
    HolderPok,
    /// `holder_set_d{depth}` — in-circuit hidden-holder SET membership (sq-3c00,
    /// the HolderPoP hidden-holder-SET anonymity tier). The proof's PUBLIC inputs
    /// are `challenge` + the holder-set Merkle `holder_set_root`; the holder secret
    /// `hsk`, the holder public key `(hpk_x, hpk_y)`, the membership `index`, and
    /// the authentication path are PRIVATE, so the proof proves "I possess the
    /// holder secret of SOME holder in the set" without disclosing the secret, the
    /// key, OR which holder. The relation is `hpk = hsk·G` (Baby-JubJub, ONE
    /// scalar-mul, plus on-curve / identity / `< L` guards) AND a depth-`depth`
    /// Poseidon2 Merkle membership of the holder-key DIGEST
    /// `Poseidon2([ZKSIG_HK, hpk.x, hpk.y])`, reusing `holder.nr`'s `holder_pok`
    /// gadgets + `issuer.nr`'s Merkle-fold pattern verbatim. The hidden-holder
    /// analogue of `HiddenIssuer` (which hides WHICH issuer); the privacy upgrade
    /// over the clear-digest `HolderPok` member (which makes `holder_pk_digest`
    /// public). The verifier gate that binds `holder_set_root` to the relying
    /// party's authoritative holder registry is `bind_holder_set`, SEPARATE from
    /// this member registration.
    ///
    /// NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2); opt-in. No
    /// soundness / ZK-privacy property is asserted as achieved.
    // [OPUS-4.8] sq-3c00 (HolderPoP hidden-holder-SET tier): circuit member. Opt-in,
    // NOT-yet-sound.
    HolderSet { depth: u32 },
    /// `join_eq_na{n_a}_nb{n_b}` — hidden cross-credential JOIN
    /// (sq-bwwl / sq-fi03, `research/zk-hidden-join-design.md` §2.2/§3.1). Proves
    /// two scan rows share a value at chosen slots — `row_a[slot_a] ==
    /// row_b[slot_b]` — WITHOUT disclosing the joined term encoding. The two graph
    /// size buckets `n_a`/`n_b` select the compiled member (the `N_A`/`N_B` const
    /// generics of `join_eq_check`), exactly as `Scan { k, n, r }` selects a scan
    /// member, so a proof verifies only against the member its witnesses fit. The
    /// proof's PUBLIC inputs are `[challenge, commit_a, commit_b, join_commitment,
    /// slot_a, slot_b]` (see [`ProofInputs::JoinEq`]); the join VALUE, both graphs'
    /// contents, the two joined rows, and the blinder are PRIVATE.
    ///
    /// This is SCHEMA ONLY (sq-fi03, step 3). The verifier gate `bind_joins`
    /// (public-input reconstruction + canonical-vk + the `UnboundJoin` query
    /// binding) is step 4 (sq-sfsi) and is NOT wired here.
    // [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): hidden cross-credential JOIN member.
    JoinEq { n_a: u32, n_b: u32 },
    /// `path_reach_d{d}_k{k}_n{n}` — BOUNDED-DEPTH property-path reachability
    /// member (sq-3kd2g.6, the compose-side of the sq-3kd2g.2 circuit family).
    /// Proves the EXISTENCE-ONLY statement of
    /// `research/zksparql-fragment-extension.md` §4: "a chain of `1..=d`
    /// (`0..=d` when the zero-length case is admitted) committed triples, each
    /// carrying the disclosed path predicate, chained object-to-subject, connects
    /// the disclosed source to the disclosed destination in the union of `k`
    /// committed graphs".
    ///
    /// # The three bucket parameters (they name the compiled member)
    /// - `d` = the compile-time DEPTH BOUND (the design record's normative "path
    ///   depth `k`" of §4 requirement 1 — renamed `d` HERE only to avoid colliding
    ///   with `k` = graph count, and to match the `path_reach_d{d}_…` package
    ///   directory). `d` is ALSO surfaced as the circuit's public `depth_bound`
    ///   input, constant-constrained to `D` in-circuit, so a manifest cannot
    ///   disclose a different bound than the member it binds (soundness req 1).
    /// - `k` = number of committed graphs (the `[Field; K]` commitments arity,
    ///   exactly like `Scan { k, .. }`).
    /// - `n` = triple slots per graph (the `N` membership-probe width, like
    ///   `Scan { n, .. }`).
    ///
    /// The four compiled members today are `(d,k,n)` in {(2,1,16), (4,1,16),
    /// (4,2,16), (8,1,16)} — [`crate::build::derive_path_reach_id`] is the single
    /// source of that family list; an `(d,k,n)` outside it derives `None`
    /// (fail-closed, no wrong-bucket fallback).
    ///
    /// # SOUNDNESS (load-bearing, NOT a security claim)
    /// A bounded path proof is EXISTENCE-ONLY and MONOTONE — it never asserts a
    /// longer path does not exist nor that the reachable set is complete (req 2).
    /// This member registers the id + public-input serialization + fail-closed
    /// dispatch routing; it does NOT make the composition verifier sound. The
    /// verifier is internally re-audited but NOT externally audited (sq-qhy4
    /// pending); no soundness / ZK-privacy property is asserted as achieved.
    // [OPUS-4.8] sq-3kd2g.6: bounded-depth path-reachability member. Opt-in
    // (`extended-fragment` feature), research-grade, NOT externally audited.
    #[cfg(feature = "extended-fragment")]
    PathReach { d: u32, k: u32, n: u32 },
}

impl CircuitId {
    /// The on-disk package directory name under `zk/compose/`.
    pub fn package(&self) -> String {
        match self {
            CircuitId::Scan { k, n, r } => format!("scan_k{k}_n{n}_r{r}"),
            CircuitId::FilterInt { d } => format!("filter_int_d{d}"),
            CircuitId::FilterF64 { d } => format!("filter_f64_d{d}"),
            // [OPUS-4.8] sq-7lrq: the `md` magnitude-digit count names the compiled
            // signed-int member, e.g. `filter_signed_int_d2`.
            CircuitId::FilterSignedInt { md } => format!("filter_signed_int_d{md}"),
            // [OPUS-4.8] sq-7lrq: the `(id, fd)` integer/fraction digit counts name
            // the compiled decimal member, e.g. `filter_decimal_i3_f2`.
            CircuitId::FilterDecimal { id, fd } => format!("filter_decimal_i{id}_f{fd}"),
            // [OPUS-4.8] sq-xojl: digit-count-free dual-leaf value member (one
            // package per datatype class; the integer class).
            #[cfg(feature = "dual-leaf")]
            CircuitId::FilterValueDl => "filter_value_dl_int".to_string(),
            // [OPUS-4.8] sq-2ezsx: the double + decimal datatype-class siblings.
            #[cfg(feature = "dual-leaf")]
            CircuitId::FilterValueDlF64 => "filter_value_dl_f64".to_string(),
            #[cfg(feature = "dual-leaf")]
            CircuitId::FilterValueDlDecimal => "filter_value_dl_decimal".to_string(),
            // [OPUS-5] sq-wz99x: the dateTime/date datatype-class sibling. ONE
            // package for BOTH lanes — the lane lives in the public
            // `datatype_const`, not the member name.
            #[cfg(feature = "dual-leaf")]
            CircuitId::FilterValueDlDateTime => "filter_value_dl_datetime".to_string(),
            CircuitId::RevokeUnset { depth } => format!("revoke_unset_d{depth}"),
            // [OPUS-5] sq-kndw: the (status-list depth, accepted-set depth) pair
            // names the compiled fully-hidden member, e.g. revoke_hidden_ref_d10_a4.
            CircuitId::RevokeHiddenRef { depth, set_depth } => {
                format!("revoke_hidden_ref_d{depth}_a{set_depth}")
            }
            CircuitId::HiddenIssuer { depth } => format!("hidden_issuer_d{depth}"),
            // [OPUS-4.8] sq-xqfg (HolderPoP T5): depth-free single member.
            CircuitId::HolderPok => "holder_pok".to_string(),
            // [OPUS-4.8] sq-3c00 (HolderPoP hidden-holder-SET tier): the `depth`
            // names the compiled set-membership member, e.g. `holder_set_d4`.
            CircuitId::HolderSet { depth } => format!("holder_set_d{depth}"),
            // [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): the `n_a`/`n_b` graph-size
            // buckets name the compiled join member, e.g. `join_eq_na16_nb16`.
            CircuitId::JoinEq { n_a, n_b } => format!("join_eq_na{n_a}_nb{n_b}"),
            // [OPUS-4.8] sq-3kd2g.6: the `(d, k, n)` buckets name the compiled
            // bounded-depth path member, e.g. `path_reach_d4_k2_n16`.
            #[cfg(feature = "extended-fragment")]
            CircuitId::PathReach { d, k, n } => format!("path_reach_d{d}_k{k}_n{n}"),
        }
    }
}

/// One per-property proof's public inputs, by circuit kind. These are exactly
/// the `pub` parameters of the corresponding `main` (in declaration order) —
/// the prover serializes them into `Prover.toml`, the verifier re-derives the
/// public-input vector from them. `binding`'s challenge is prepended as the
/// first `challenge: pub Field` of every member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "circuit")]
pub enum ProofInputs {
    /// scan_k{k}_n{n}_r{r}: commitment recompute + scan completeness.
    #[serde(rename = "scan")]
    Scan {
        id: CircuitId,
        /// Per-graph flat Poseidon2 commitments (length k).
        commitments: Vec<FieldHex>,
        /// BGP slot constancy (s, p, o).
        pattern_is_const: [bool; 3],
        /// Term encodings of constant slots (0 = variable).
        pattern_const_enc: [FieldHex; 3],
        /// Disclosed matched rows (length r, padded with zero rows).
        rows: Vec<[FieldHex; 3]>,
        /// Active row count (<= r).
        row_count: u32,
        /// Per-graph source attribution (length k): `attribution[g]` is true
        /// iff this pattern's match set draws a triple from committed graph `g`.
        /// Constrained in-circuit (`scan.nr` step 4, audit #8) and byte-bound
        /// into the bb public inputs by `crate::verifier::reconstruct_public_inputs`,
        /// so it is PROOF-BOUND, not a prover-controlled claim. The verifier
        /// cross-checks it against `manifest.attributions` for the pattern this
        /// scan answers (closes the `[[0],[0]]` collapse-two-graphs forge).
        // [OPUS-4.8] audit #8: in-circuit per-graph source attribution.
        #[serde(default)]
        attribution: Vec<bool>,
    },
    /// filter_int_d{d}: hidden-operand numeric FILTER over an xsd:integer.
    #[serde(rename = "filter_int")]
    FilterInt {
        id: CircuitId,
        /// The hidden column's term encoding (the scan-proof anchor).
        operand_enc: FieldHex,
        op: FilterOp,
        /// The FILTER's constant operand.
        bound: u64,
        /// The disclosed verdict.
        expected: bool,
    },
    /// filter_f64_d{d}: MANIFEST-COMPOSABLE hidden-operand numeric FILTER over an
    /// xsd:double (integer-valued fragment) ([OPUS-4.8] sq-q7e / sq-tat). Public
    /// inputs mirror the member `main`: challenge (prepended), operand_enc, op,
    /// b_bits, expected. The hidden operand is bound to the committed literal by
    /// `operand_enc` (the scan-proof anchor, same as `filter_int`), and `b_bits`
    /// is the FILTER's constant double as an IEEE-754 bit pattern.
    #[serde(rename = "filter_f64")]
    FilterF64 {
        id: CircuitId,
        /// The hidden column's term encoding (the scan-proof anchor) — bound
        /// in-circuit to the committed xsd:double literal via its canonical token.
        operand_enc: FieldHex,
        op: FilterOp,
        /// The FILTER's constant operand as an IEEE-754 double bit pattern.
        b_bits: u64,
        /// The disclosed verdict.
        expected: bool,
    },
    /// filter_signed_int_d{md}: MANIFEST-COMPOSABLE hidden-operand numeric FILTER
    /// over a SIGNED xsd:integer ([OPUS-4.8] sq-7lrq). Public inputs mirror the
    /// member `main` (`zk/compose/filter_signed_int_d{md}/src/main.nr`), in
    /// declaration order AFTER the prepended `challenge`: operand_enc, op, bound_neg,
    /// bound, expected. The hidden operand is bound to the committed literal by
    /// `operand_enc` (the scan-proof anchor, same as `filter_int`); the FILTER's
    /// constant is carried sign-split as `(bound_neg, bound)` (`bound` = the `u64`
    /// magnitude), so the in-circuit signed comparison sees the full signed constant.
    /// The operand's SIGN and magnitude digits are PRIVATE witnesses.
    #[serde(rename = "filter_signed_int")]
    FilterSignedInt {
        id: CircuitId,
        /// The hidden column's term encoding (the scan-proof anchor) — bound
        /// in-circuit to the committed signed xsd:integer literal via its canonical
        /// `"[-]?<digits>"^^<…#integer>` token.
        operand_enc: FieldHex,
        op: FilterOp,
        /// Sign of the FILTER's constant operand (`true` = negative).
        bound_neg: bool,
        /// `|FILTER constant operand|` — the unsigned magnitude.
        bound: u64,
        /// The disclosed verdict.
        expected: bool,
    },
    /// filter_decimal_i{id}_f{fd}: MANIFEST-COMPOSABLE hidden-operand numeric FILTER
    /// over an xsd:decimal ([OPUS-4.8] sq-7lrq). Public inputs mirror the member
    /// `main` (`zk/compose/filter_decimal_i{id}_f{fd}/src/main.nr`), in declaration
    /// order AFTER the prepended `challenge`: operand_enc, op, bound_neg,
    /// bound_scaled, expected. The hidden operand is bound to the committed literal
    /// by `operand_enc`; the FILTER's constant is carried as the sign +
    /// HOST-PRESCALED magnitude `bound_scaled = round(|bound| * 10^fd)` (so the
    /// fixed-point comparison at `fd` places stays in the integer domain). The
    /// operand's SIGN, integer-part digits, and fraction digits are PRIVATE witnesses.
    #[serde(rename = "filter_decimal")]
    FilterDecimal {
        id: CircuitId,
        /// The hidden column's term encoding (the scan-proof anchor) — bound
        /// in-circuit to the committed xsd:decimal literal via its canonical
        /// `"[-]?<int>.<frac>"^^<…#decimal>` token.
        operand_enc: FieldHex,
        op: FilterOp,
        /// Sign of the FILTER's constant operand (`true` = negative).
        bound_neg: bool,
        /// `round(|FILTER constant operand| * 10^fd)` — the host-prescaled magnitude
        /// the in-circuit fixed-point comparison uses (`fd` = the member's fraction
        /// digit count).
        bound_scaled: u64,
        /// The disclosed verdict.
        expected: bool,
    },
    /// filter_value_dl_int: DUAL-LEAF value-lane FILTER over a committed
    /// NON-NEGATIVE xsd:integer ([OPUS-4.8] sq-xojl). These fields are EXACTLY the
    /// `pub` parameters of the `filter_value_dl_int` member `main`
    /// (`zk/compose/filter_value_dl_int/src/main.nr`), in declaration order AFTER
    /// the prepended `challenge`:
    ///
    /// ```text
    /// [challenge, operand_enc, op, bound, datatype_const, expected]
    /// ```
    ///
    /// The hidden operand is bound to the committed literal by `operand_enc` (the
    /// scan-proof anchor, same edge as `filter_int`) — but `operand_enc` here is
    /// the DUAL-LEAF `Enc = h3(h3(VALUE_HOOK, datatype_const, LANG_NONE),
    /// lexical_component, TYPE_CODE_LITERAL)` (`sparq_zk::dual_leaf`), bound via two
    /// Poseidon2 permutations over the witnessed `VALUE_HOOK` with NO in-circuit
    /// blake3. The `VALUE_HOOK` and `lexical_component` are PRIVATE witnesses; the
    /// `datatype_const = blake3(datatype IRI)` is PUBLIC (it folds the datatype so
    /// a cross-datatype value collision cannot occur).
    ///
    /// The `xsd:boolean` VALUE LANE SHARES THIS VARIANT (sq-5xdlk): the boolean
    /// hooks `{0 = false, 1 = true}` lie inside this member's `u64` comparison
    /// domain, so no new Noir member exists — the boolean lane simply carries
    /// `datatype_const = `[`boolean_datatype_const`]`()` and `bound ∈ {0, 1}`, and
    /// the lanes are separated by that public constant alone (it is folded into
    /// `value_component`, so an integer leaf's honest witness cannot satisfy a
    /// boolean member call, and vice versa). Build one with
    /// [`crate::build::build_filter_value_dl_boolean`].
    ///
    /// DOCUMENTED RISK: this carries the INV-VL downgrade (value↔lexical agreement
    /// is trusted-issuer-honesty, not machine-enforced; #769 accepted, CR-G8 /
    /// sq-qhy4). NOT externally audited; no soundness / privacy claim.
    // [OPUS-4.8] sq-xojl: dual-leaf value-lane FILTER inputs. Opt-in, NOT-yet-sound.
    // [OPUS-5] sq-5xdlk: + the xsd:boolean lane, by datatype_const alone.
    #[cfg(feature = "dual-leaf")]
    #[serde(rename = "filter_value_dl")]
    FilterValueDl {
        id: CircuitId,
        /// The hidden column's DUAL-LEAF term encoding (the scan-proof anchor).
        operand_enc: FieldHex,
        op: FilterOp,
        /// The FILTER's constant operand — a non-negative integer, or (on the
        /// `xsd:boolean` lane, sq-5xdlk) the boolean hook `0` = `false` / `1` =
        /// `true`.
        bound: u64,
        /// `blake3(datatype IRI)` as a field — the public `DATATYPE_CONST` folded
        /// into `value_component`. This is what SELECTS the datatype lane:
        /// `blake3(xsd:integer)` for the integer lane,
        /// [`boolean_datatype_const`]`()` for the boolean one.
        datatype_const: FieldHex,
        /// The disclosed verdict.
        expected: bool,
    },
    /// filter_value_dl_f64: DUAL-LEAF value-lane FILTER over a committed
    /// `xsd:double` ([OPUS-4.8] sq-2ezsx). These fields are EXACTLY the `pub`
    /// parameters of the `filter_value_dl_f64` member `main`
    /// (`zk/compose/filter_value_dl_f64/src/main.nr`), in declaration order AFTER
    /// the prepended `challenge`:
    ///
    /// ```text
    /// [challenge, operand_enc, op, b_bits, datatype_const, expected]
    /// ```
    ///
    /// `operand_enc` is the DUAL-LEAF `Enc` over the CANONICAL IEEE bits (the
    /// member canonicalises `-0.0`/`+0.0` and NaN payloads before binding); the
    /// `VALUE_HOOK` (IEEE bits) and `lexical_component` are PRIVATE; `b_bits` is the
    /// FILTER's constant double as an IEEE-754 bit pattern (PUBLIC), and
    /// `datatype_const = blake3(xsd:double IRI)` is PUBLIC. DOCUMENTED RISK: INV-VL
    /// downgrade (CR-G8 / sq-qhy4); NOT externally audited; no soundness claim.
    // [OPUS-4.8] sq-2ezsx: dual-leaf double value-lane FILTER inputs. Opt-in.
    #[cfg(feature = "dual-leaf")]
    #[serde(rename = "filter_value_dl_f64")]
    FilterValueDlF64 {
        id: CircuitId,
        /// The hidden column's DUAL-LEAF double term encoding (the scan-proof anchor).
        operand_enc: FieldHex,
        op: FilterOp,
        /// The FILTER's constant double as an IEEE-754 bit pattern.
        b_bits: u64,
        /// `blake3(xsd:double IRI)` as a field — the public `DATATYPE_CONST`.
        datatype_const: FieldHex,
        /// The disclosed verdict.
        expected: bool,
    },
    /// filter_value_dl_decimal: DUAL-LEAF value-lane FILTER over a committed
    /// `xsd:decimal` at a fixed canonical scale ([OPUS-4.8] sq-2ezsx). These fields
    /// are EXACTLY the `pub` parameters of the `filter_value_dl_decimal` member
    /// `main` (`zk/compose/filter_value_dl_decimal/src/main.nr`), in declaration
    /// order AFTER the prepended `challenge`:
    ///
    /// ```text
    /// [challenge, operand_enc, op, bound_neg, bound_scaled, datatype_const, expected]
    /// ```
    ///
    /// `operand_enc` is the DUAL-LEAF `Enc` over the SIGNED scaled magnitude; the
    /// `VALUE_HOOK` (sign + scaled magnitude) and `lexical_component` are PRIVATE;
    /// the FILTER's constant is carried sign-split + host-prescaled as
    /// `(bound_neg, bound_scaled = round(|bound| * 10^fd))`, and
    /// `datatype_const = blake3(xsd:decimal IRI ‖ "@scale=fd")` is PUBLIC — it folds
    /// BOTH the datatype AND the canonical scale (the B4 scale bind). DOCUMENTED
    /// RISK: INV-VL downgrade (CR-G8 / sq-qhy4); NOT externally audited.
    // [OPUS-4.8] sq-2ezsx: dual-leaf decimal value-lane FILTER inputs. Opt-in.
    #[cfg(feature = "dual-leaf")]
    #[serde(rename = "filter_value_dl_decimal")]
    FilterValueDlDecimal {
        id: CircuitId,
        /// The hidden column's DUAL-LEAF decimal term encoding (the scan-proof anchor).
        operand_enc: FieldHex,
        op: FilterOp,
        /// Sign of the FILTER's constant operand (`true` = negative).
        bound_neg: bool,
        /// `round(|FILTER constant operand| * 10^fd)` — the host-prescaled magnitude.
        bound_scaled: u64,
        /// `blake3(xsd:decimal IRI ‖ "@scale=fd")` as a field — folds the datatype
        /// AND the canonical scale into the public `DATATYPE_CONST` (the B4 bind).
        datatype_const: FieldHex,
        /// The disclosed verdict.
        expected: bool,
    },
    /// filter_value_dl_datetime: DUAL-LEAF value-lane FILTER over a committed
    /// `xsd:dateTime` OR `xsd:date` ([OPUS-5] sq-wz99x). These fields are EXACTLY
    /// the `pub` parameters of the `filter_value_dl_datetime` member `main`
    /// (`zk/compose/filter_value_dl_datetime/src/main.nr`), in declaration order
    /// AFTER the prepended `challenge`:
    ///
    /// ```text
    /// [challenge, operand_enc, op, bound_neg, bound_scaled_epoch, datatype_const, expected]
    /// ```
    ///
    /// `operand_enc` is the DUAL-LEAF `Enc` over the SIGNED SCALED EPOCH; the
    /// `VALUE_HOOK` (sign + scaled epoch) and `lexical_component` are PRIVATE; the
    /// FILTER's constant instant is carried sign-split + host-converted as
    /// `(bound_neg, bound_scaled_epoch = |T_bound|` in milliseconds`)`, and
    /// `datatype_const = blake3(<xsd:dateTime|xsd:date IRI> ‖ "@epochscale=3")` is
    /// PUBLIC — it SELECTS the lane and folds the sub-second scale `FS` (the B4
    /// bind). The dateTime and date lanes share this ONE member and are separated
    /// by that constant alone. DOCUMENTED RISK: INV-VL downgrade, and the §13 rule
    /// set is an OPEN external-audit obligation (CR-G8 / sq-qhy4); NOT externally
    /// audited.
    // [OPUS-5] sq-wz99x: dual-leaf dateTime/date value-lane FILTER inputs. Opt-in.
    #[cfg(feature = "dual-leaf")]
    #[serde(rename = "filter_value_dl_datetime")]
    FilterValueDlDateTime {
        id: CircuitId,
        /// The hidden column's DUAL-LEAF dateTime/date term encoding (the scan-proof
        /// anchor).
        operand_enc: FieldHex,
        op: FilterOp,
        /// Sign of the FILTER's constant instant (`true` = pre-epoch).
        bound_neg: bool,
        /// `|T_bound|` in milliseconds — the FILTER constant on the same scaled-epoch
        /// timeline as the value handle.
        bound_scaled_epoch: u64,
        /// `blake3(<datatype IRI> ‖ "@epochscale=3")` as a field — SELECTS the
        /// dateTime lane ([`datetime_datatype_const`]) or the date lane
        /// ([`date_datatype_const`]) and folds the scale.
        datatype_const: FieldHex,
        /// The disclosed verdict.
        expected: bool,
    },
    /// `join_eq_na{n_a}_nb{n_b}`: hidden cross-credential JOIN
    /// (sq-bwwl / sq-fi03, `research/zk-hidden-join-design.md` §2.2/§3.2). These
    /// fields are EXACTLY the `pub` parameters of the `join_eq` member `main`, in
    /// declaration order AFTER the prepended `challenge` (which `binding` carries,
    /// like every member). The verifier's public-input reconstruction (step 4,
    /// sq-sfsi) MUST emit them in this order:
    ///
    /// ```text
    /// [challenge, commit_a, commit_b, join_commitment, slot_a, slot_b]
    /// ```
    ///
    /// Cross-reference — the Noir source this layout MUST match (do not reorder;
    /// the verifier rebuilds the vector in declaration order, audit-#1 discipline):
    /// `zk/compose/join_eq_na16_nb16/src/main.nr` `fn main(challenge, commit_a,
    /// commit_b, join_commitment, slot_a, slot_b, /* private */ …)`.
    ///
    /// The join VALUE is PRIVATE (never a public input — the headline privacy win);
    /// only the two graph commitments, the HIDING `join_commitment`, and the two
    /// query-bound join slots are public. The slots are public BY DESIGN (§4.4):
    /// the query already reveals which column a shared variable occupies, so
    /// disclosing the slot is not new leakage — only the join term stays hidden.
    // [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): hidden cross-credential JOIN inputs.
    #[serde(rename = "join_eq")]
    JoinEq {
        id: CircuitId,
        /// Graph-A commitment `C(G_a)`. Byte-bound (step 4, sq-sfsi) to equal the
        /// scan-A sub-proof's `commitments[g_a]` — the anti-row-swap binding (A2).
        commit_a: FieldHex,
        /// Graph-B commitment `C(G_b)`. Byte-bound to scan-B's `commitments[g_b]`.
        commit_b: FieldHex,
        /// The HIDING commitment to the join value: `h3(SIG_DOMAIN_JOIN, value,
        /// blinding)` (`sparq_zk::sig::join_value_commitment`). Per-presentation
        /// blinded, so two presentations of the same join value are unlinkable and
        /// a low-entropy key is not dictionary-attackable (design §1.4 R4 / §2.4).
        join_commitment: FieldHex,
        /// Graph-A join slot in `{0,1,2}` (s/p/o). PUBLIC and query-bound: the
        /// verifier (step 4) requires it to equal the query-derived slot the shared
        /// variable occupies in pattern A (§4.4 slot binding).
        slot_a: u32,
        /// Graph-B join slot in `{0,1,2}` — query-bound to pattern B's slot.
        slot_b: u32,
    },
    /// `path_reach_d{d}_k{k}_n{n}`: BOUNDED-DEPTH property-path reachability
    /// public inputs (sq-3kd2g.6). These fields are EXACTLY the `pub` parameters
    /// of the member `main` (`zk/compose/path_reach_d{d}_k{k}_n{n}/src/main.nr`),
    /// in declaration order AFTER the prepended `challenge` (which `binding`
    /// carries for every member):
    ///
    /// ```text
    /// [challenge, commitments[k], pred_enc, src_enc, dst_enc, allow_zero,
    ///  depth_bound, attribution[k]]
    /// ```
    ///
    /// The path chain nodes, the actual (hidden) chain length `l <= d`, and the
    /// per-graph triple encodings are PRIVATE witnesses. `depth_bound` is the
    /// PUBLIC, constant-constrained depth bound the manifest discloses (soundness
    /// req 1): the verifier's `reconstruct_public_inputs` emits it and
    /// `dispatch_fragment` requires it to equal the member's compiled `d`, so a
    /// consumer always sees the bound the proof was produced at.
    ///
    /// # SOUNDNESS (load-bearing, NOT a security claim)
    /// EXISTENCE-ONLY: a passing proof witnesses one bounded chain; it says
    /// NOTHING about longer paths or reachable-set completeness. NOT externally
    /// audited (sq-qhy4); no soundness / privacy claim.
    // [OPUS-4.8] sq-3kd2g.6: bounded-depth path-reachability inputs. Opt-in
    // (`extended-fragment`), research-grade, NOT-yet-sound.
    #[cfg(feature = "extended-fragment")]
    #[serde(rename = "path_reach")]
    PathReach {
        id: CircuitId,
        /// Per-graph flat Poseidon2 commitments (length `k`) — the committed
        /// graph union the chain is drawn from (same role as `Scan.commitments`).
        commitments: Vec<FieldHex>,
        /// Term encoding of the single path predicate every chain triple carries
        /// (a constant NamedNode, salt-independent).
        pred_enc: FieldHex,
        /// Term encoding of the disclosed chain SOURCE `μ(s)`.
        src_enc: FieldHex,
        /// Term encoding of the disclosed chain DESTINATION `μ(o)`.
        dst_enc: FieldHex,
        /// Whether the operator admits the ZERO-LENGTH path (`p*` / `p?`). `true`
        /// for `*`/`?`, `false` for `+`. `dispatch_fragment` cross-checks this
        /// against the query-re-derived closure (`false` iff `closure.min_len ==
        /// 1`) so a manifest cannot silently upgrade `+` to `*`.
        allow_zero: bool,
        /// The PUBLIC depth bound (`= d`, constant-constrained in-circuit). The
        /// verifier surfaces it and rejects a manifest whose `depth_bound` differs
        /// from the member's compiled `d` (soundness req 1 — depth-overflow /
        /// mismatch is fail-closed).
        depth_bound: u32,
        /// Per-graph source attribution (length `k`): `attribution[g]` is true iff
        /// the chain draws a triple from committed graph `g` (chain-relative,
        /// constrained in-circuit — same proof-bound provenance as `Scan.attribution`).
        attribution: Vec<bool>,
    },
}

impl ProofInputs {
    pub fn circuit_id(&self) -> &CircuitId {
        match self {
            ProofInputs::Scan { id, .. } => id,
            ProofInputs::FilterInt { id, .. } => id,
            ProofInputs::FilterF64 { id, .. } => id,
            // [OPUS-4.8] sq-7lrq: signed xsd:integer / xsd:decimal composable FILTERs.
            ProofInputs::FilterSignedInt { id, .. } => id,
            ProofInputs::FilterDecimal { id, .. } => id,
            // [OPUS-4.8] sq-xojl: dual-leaf value-lane FILTER.
            #[cfg(feature = "dual-leaf")]
            ProofInputs::FilterValueDl { id, .. } => id,
            // [OPUS-4.8] sq-2ezsx: dual-leaf double + decimal value-lane FILTERs.
            #[cfg(feature = "dual-leaf")]
            ProofInputs::FilterValueDlF64 { id, .. } => id,
            #[cfg(feature = "dual-leaf")]
            ProofInputs::FilterValueDlDecimal { id, .. } => id,
            // [OPUS-5] sq-wz99x: dual-leaf dateTime/date value-lane FILTER.
            #[cfg(feature = "dual-leaf")]
            ProofInputs::FilterValueDlDateTime { id, .. } => id,
            // [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): hidden cross-credential JOIN.
            ProofInputs::JoinEq { id, .. } => id,
            // [OPUS-4.8] sq-3kd2g.6: bounded-depth path reachability.
            #[cfg(feature = "extended-fragment")]
            ProofInputs::PathReach { id, .. } => id,
        }
    }
}

/// The PUBLIC `datatype_const` of the DUAL-LEAF `xsd:boolean` value lane
/// (sq-5xdlk) — `blake3_field(xsd:boolean IRI)`, exactly the constant
/// `sparq_zk::dual_leaf_boolean::encode_boolean` (the host half, sq-hh7a4) folds
/// into the committed leaf's `value_component`.
///
/// # The boolean lane adds NO new Noir member
///
/// It REUSES [`CircuitId::FilterValueDl`] (`filter_value_dl_int`): that member's
/// `datatype_const` is already a PUBLIC input, and its `u64` comparison domain
/// covers the boolean value hooks `{0 = false, 1 = true}` — so the XSD boolean
/// order `false < true` (and hence the degenerate `LT`/`LE`/`GT`/`GE` results
/// alongside `EQ`/`NE`) falls out of the integer comparison unchanged. The lane
/// is therefore pure WIRING: host and verifier pick THIS constant instead of
/// `blake3(xsd:integer)`.
///
/// # Lane separation is the public `datatype_const`, and only that
///
/// `datatype_const` is folded into `value_component = h3(VALUE_HOOK,
/// DATATYPE_CONST, LANG_NONE)`, and `blake3(xsd:boolean) != blake3(xsd:integer)`.
/// So the honest witness for a committed `"1"^^xsd:integer` leaf recomputes a
/// DIFFERENT leaf under this constant and fails the member's
/// `assert_eq(leaf, operand_enc)` binding — and symmetrically for a boolean leaf
/// under the integer constant. That is a BINDING argument resting on Poseidon2
/// preimage resistance (a prover free to search preimages is out of scope of this
/// statement), NOT an audited soundness claim.
///
/// DOCUMENTED RISK: the boolean lane inherits the value lane's INV-VL downgrade
/// — value↔lexical agreement is TRUSTED-ISSUER-HONESTY, not machine-enforced
/// (#769 accepted; gap CR-G8 / sq-qhy4). The ZK estate is internally re-audited
/// but NOT externally audited; nothing here is a soundness or privacy guarantee.
// [OPUS-5] sq-5xdlk: boolean value-lane wiring. Opt-in (`dual-leaf`), NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
pub fn boolean_datatype_const() -> FieldHex {
    FieldHex(field_to_hex(&sparq_zk::dual_leaf::datatype_const(
        sparq_zk::dual_leaf_boolean::XSD_BOOLEAN,
    )))
}

/// The PUBLIC `datatype_const` of the DUAL-LEAF `xsd:dateTime` value lane
/// (sq-wz99x) — `blake3_field("<xsd:dateTime IRI>@epochscale=3")`, exactly the
/// constant `sparq_zk::dual_leaf_datetime::encode_datetime` (the host half,
/// sq-we9vs) folds into the committed leaf's `value_component`. It carries the
/// lane-fixed sub-second scale `FS`, so a hook at one scale can never collide a
/// hook at another (the B4 bind, the `@scale=` construction of the decimal lane).
///
/// Pair it with [`CircuitId::FilterValueDlDateTime`] /
/// [`ProofInputs::FilterValueDlDateTime`]; build one with
/// [`crate::build::build_filter_value_dl_datetime`].
///
/// DOCUMENTED RISK: inherits the value lane's INV-VL downgrade (#769 accepted,
/// CR-G8 / sq-qhy4), and the §13 rule set is itself an OPEN external-audit
/// obligation. NOT externally audited; no soundness / privacy claim.
// [OPUS-5] sq-wz99x: dateTime lane constant. Opt-in (`dual-leaf`), NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
pub fn datetime_datatype_const() -> FieldHex {
    FieldHex(field_to_hex(
        &sparq_zk::dual_leaf_datetime::datetime_datatype_const(),
    ))
}

/// The PUBLIC `datatype_const` of the DUAL-LEAF `xsd:date` value lane (sq-wz99x)
/// — `blake3_field("<xsd:date IRI>@epochscale=3")`, the constant
/// `sparq_zk::dual_leaf_datetime::encode_date` folds into the committed leaf.
///
/// # The date lane adds NO second Noir member
///
/// It shares [`CircuitId::FilterValueDlDateTime`] with the dateTime lane: that
/// member's `datatype_const` is a PUBLIC input, and a date's `VALUE_HOOK` is the
/// scaled epoch of the date's STARTING instant (midnight UTC — XSD orders dates by
/// their starting moment), which lives in the SAME signed-`u64` domain the member
/// already compares. So the lane is pure WIRING: host and verifier pick THIS
/// constant instead of [`datetime_datatype_const`]`()`.
///
/// # Lane separation is the public `datatype_const`, and only that
///
/// A date's hook is NUMERICALLY EQUAL to the dateTime hook of the same starting
/// instant (`"1970-01-02Z"` and `"1970-01-02T00:00:00Z"` both hook `86_400_000`),
/// so the constant is the ONLY thing keeping the two terms apart. Because it is
/// folded into `value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE)` and
/// the two constants differ, an honest date witness recomputes a DIFFERENT leaf
/// under the dateTime constant and fails the member's
/// `assert_eq(leaf, operand_enc)` binding — and symmetrically. That is a BINDING
/// argument resting on Poseidon2 preimage resistance, NOT an audited soundness
/// claim.
///
/// DOCUMENTED RISK: as [`datetime_datatype_const`] (CR-G8 / sq-qhy4). NOT
/// externally audited; no soundness / privacy claim.
// [OPUS-5] sq-wz99x: date lane constant. Opt-in (`dual-leaf`), NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
pub fn date_datatype_const() -> FieldHex {
    FieldHex(field_to_hex(
        &sparq_zk::dual_leaf_datetime::date_datatype_const(),
    ))
}

/// One composed sub-proof: the circuit member, its public inputs, and the
/// raw bb proof bytes (hex). Composition is the verifier checking each
/// sub-proof AND the binding-consistency edges between them (a shared
/// `operand_enc` appearing in both a scan proof's rows and a filter proof's
/// public inputs is a plain public-input equality — the "modular per-property
/// proof" pattern, sparql_noir reference architecture).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubProof {
    pub inputs: ProofInputs,
    /// bb proof bytes, hex-encoded (no `0x`). Empty in witness-only manifests.
    #[serde(default)]
    pub proof_hex: String,
}

/// A binding-consistency edge: the term encoding at `from_proof`'s row/slot
/// must equal the `operand_enc` of `to_proof` (a numeric FILTER applied to a
/// scanned column). Verifier checks this as a field equality over the
/// already-verified public inputs.
///
/// # Canonical ordering ([OPUS-4.8] sq-y2wy)
/// `binding_edges` is canonicalised by [`ProofManifest::canonicalize`] (applied
/// before hashing/serialisation) to ascending `(from_proof, from_row, from_slot,
/// to_proof)` — the field-declaration tuple, which is a TOTAL order over edges.
/// Each edge is SELF-CONTAINED (it carries its own scan/row/slot/filter indices),
/// so reordering the edge VECTOR never invalidates those references: the verifier
/// resolves every edge against `sub_proofs` by the edge's own indices, not by the
/// edge's position. So two manifests that differ only in binding-edge order
/// canonicalise to the same vector, hence the same serialisation/hash. The
/// `#[derive(Ord)]` tuple ordering is exactly that field tuple (declaration order).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BindingEdge {
    /// Index into `ProofManifest::sub_proofs` of the scan proof.
    pub from_proof: usize,
    /// Which disclosed row of the scan proof carries the operand.
    pub from_row: usize,
    /// Which slot (0=s,1=p,2=o) of that row is the operand column.
    pub from_slot: usize,
    /// Index into `sub_proofs` of the consuming filter proof.
    pub to_proof: usize,
}

/// A hidden cross-credential JOIN edge (sq-bwwl / sq-fi03,
/// `research/zk-hidden-join-design.md` §3.2): the hidden-key analogue of
/// [`BindingEdge`]. Where a `BindingEdge` ties a **disclosed** scan slot to a
/// filter operand, a `JoinEdge` ties **two scan sub-proofs' graph commitments**
/// to a `join_eq` sub-proof — disclosing the *graph linkage* (which two
/// credentials are joined, at which slots) but NOT the joined term value.
///
/// The edge names the two scan sub-proofs and which committed graph index within
/// each (`commitments[graph_a]` / `commitments[graph_b]`) the join binds, plus
/// the index of the `join_eq` sub-proof. The verifier gate `bind_joins` (step 4,
/// sq-sfsi) resolves these, requires the `join_eq` proof's public
/// `commit_a`/`commit_b` to byte-equal the two scans' `commitments[graph_*]` (the
/// anti-A2 binding), and requires the proof's public `slot_a`/`slot_b` to equal
/// the query-derived slots for the shared variable (the §4.4 slot binding). Those
/// slots live on the `join_eq` proof's public inputs, NOT on this edge.
///
/// # Canonical ordering ([OPUS-4.8] sq-y2wy — was deferred at sq-fi03)
/// `join_edges` is canonicalised by [`ProofManifest::canonicalize`] (applied
/// before hashing/serialisation) to ascending `(scan_a, graph_a, scan_b, graph_b,
/// join_proof)` — the field-declaration tuple, a TOTAL order over edges (it
/// extends the previously-proposed `(scan_a, graph_a, scan_b, graph_b)` key with
/// `join_proof` for a strict total order even when two edges share all four scan
/// refs). Each edge is SELF-CONTAINED (it carries its own scan/graph/join-proof
/// indices), so sorting the edge VECTOR never invalidates those references: the
/// `bind_joins` gate resolves every edge against `sub_proofs` by the edge's own
/// indices, not by its position. So two manifests that differ only in join-edge
/// order canonicalise to the same vector, hence the same serialisation/hash. The
/// `#[derive(Ord)]` tuple ordering is exactly that field tuple. `join_eq` itself
/// is value-symmetric, so the proven equality holds regardless of edge order.
///
/// The `bind_joins` gate that consumes this is step 4 (sq-sfsi).
// [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): hidden cross-credential JOIN edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct JoinEdge {
    /// Index into [`ProofManifest::sub_proofs`] of the scan sub-proof for graph A.
    pub scan_a: usize,
    /// Which committed graph of `scan_a` (index into its `commitments`) the join
    /// binds — the `commitments[graph_a]` whose value must equal the `join_eq`
    /// proof's public `commit_a`.
    pub graph_a: usize,
    /// Index into `sub_proofs` of the scan sub-proof for graph B.
    pub scan_b: usize,
    /// Which committed graph of `scan_b` the join binds (`commitments[graph_b]`).
    pub graph_b: usize,
    /// Index into `sub_proofs` of the `join_eq` sub-proof
    /// ([`ProofInputs::JoinEq`]) that proves the hidden equality.
    pub join_proof: usize,
}

/// One DISCLOSED term of an extended-fragment solution (sq-1zf94): an IRI or a
/// literal the relying party reads off the presented solution. Blank nodes are
/// NOT expressible (they are existential in the committed model, so a disclosed
/// solution never names one) — an endpoint that binds to a blank node stays
/// existential/undisclosed and is not term-bound by this layer.
///
/// The verifier RE-ENCODES the term itself (`sparq_zk::encode::encode_term`,
/// salt-independent for IRIs/literals) and byte-matches the recomputed encoding
/// against the proof-bound `PathReach` `src_enc`/`dst_enc` / the query's `VALUES`
/// cell — it never trusts a prover-supplied encoding. So this carries the
/// PREIMAGE the relying party reads, and the gate proves that preimage is the one
/// the proof attests.
// [OPUS-4.8] sq-1zf94: disclosed-solution term. Opt-in (`extended-fragment`),
// research-grade, NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisclosedTerm {
    /// An IRI (a `NamedNode`). `value` is the raw IRI (no `<>`).
    Iri {
        /// The IRI string.
        value: String,
    },
    /// A literal. Exactly one shape is well-formed: a plain literal (`value`
    /// only), a language-tagged literal (`value` + `language`, datatype implicitly
    /// `rdf:langString`), or a typed literal (`value` + `datatype`). A literal
    /// carrying BOTH a `language` and a non-`rdf:langString` `datatype` is
    /// malformed and rejected fail-closed (`to_term` returns `None`).
    Literal {
        /// The lexical value.
        value: String,
        /// The datatype IRI (typed literals; mutually exclusive with `language`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        datatype: Option<String>,
        /// The BCP-47 language tag (language-tagged literals).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
}

#[cfg(feature = "extended-fragment")]
impl DisclosedTerm {
    /// Rebuild the `oxrdf::Term` (verifier-side) so the encoding can be
    /// recomputed. Returns `None` (fail-closed) on an unparseable IRI / datatype
    /// / language tag, or a literal that carries both a language and a
    /// non-`rdf:langString` datatype.
    pub fn to_term(&self) -> Option<oxrdf::Term> {
        const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
        match self {
            DisclosedTerm::Iri { value } => {
                oxrdf::NamedNode::new(value).ok().map(oxrdf::Term::NamedNode)
            }
            DisclosedTerm::Literal { value, datatype, language } => {
                let lit = match (datatype.as_deref(), language.as_deref()) {
                    (Some(dt), Some(_)) if dt != RDF_LANG_STRING => return None,
                    (_, Some(lang)) => {
                        oxrdf::Literal::new_language_tagged_literal(value, lang).ok()?
                    }
                    (Some(dt), None) => {
                        oxrdf::Literal::new_typed_literal(value, oxrdf::NamedNode::new(dt).ok()?)
                    }
                    (None, None) => oxrdf::Literal::new_simple_literal(value),
                };
                Some(oxrdf::Term::Literal(lit))
            }
        }
    }
}

/// One disclosed variable binding of an extended-fragment solution (sq-1zf94):
/// `var` (a query variable name, no leading `?`) is bound to the disclosed
/// [`DisclosedTerm`]. The verifier binds this to the proof-bound term encodings
/// (see [`crate::verifier::bind_fragment_solution`]).
// [OPUS-4.8] sq-1zf94: disclosed-solution binding. Opt-in (`extended-fragment`).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolutionBinding {
    /// The query variable name (no leading `?`).
    pub var: String,
    /// The disclosed term the variable is bound to in this solution.
    pub term: DisclosedTerm,
}

/// The PER-SOLUTION UNION branch attribution + obligation binding (sq-3kd2g.6).
///
/// `UNION` semantics under the extended fragment is PER-SOLUTION branch
/// attribution (`research/zksparql-fragment-extension.md` §3.2): each disclosed
/// solution is attributed to exactly ONE `UNION` branch, and the verifier checks
/// THAT branch's obligations. A `BranchWitness` records, for one disclosed
/// solution, (a) which branch it witnesses and (b) the `sub_proofs` indices that
/// discharge the branch's obligations, in the branch's obligation order.
///
/// The [`crate::verifier::dispatch_fragment`] gate re-derives the branches from
/// the query text alone (via `sparq_zk::verify::fragment_query` — never trusting
/// the manifest) and checks, FAIL-CLOSED, that:
/// - `branch` indexes a real branch (out-of-range => rejected: wrong branch);
/// - `scan_proofs` / `path_proofs` / `values_rows` have EXACTLY the branch's
///   obligation arity;
/// - each `scan_proofs[i]` is a bound BGP-scan sub-proof, and each
///   `path_proofs[i]` is a bound [`ProofInputs::PathReach`] sub-proof of the
///   member the closure requires (with a matching disclosed `depth_bound` /
///   `allow_zero`) — a path obligation with no bound path sub-proof of the right
///   member is rejected;
/// - each `values_rows[i]` indexes a real re-derived VALUES row of block `i`.
///
/// A query WITHOUT `UNION` has exactly one branch, so a plain path/VALUES
/// manifest carries a single `BranchWitness { branch: 0, .. }` per disclosed
/// solution.
///
/// # Honest scope (the disclosed-solution term binding — sq-1zf94)
/// `dispatch_fragment` is the STRUCTURAL ROUTING layer: it binds each construct to
/// a bound sub-proof of the correct circuit member (and surfaces the depth bound).
/// The TERM-encoding binding of a path's `pred_enc`/`src_enc`/`dst_enc` and a
/// `VALUES` row's cell terms to the disclosed SOLUTION bindings (the [`solution`]
/// field) — the composition analogue of the flat scan-slot binding /
/// `bind_joins` commitment binding — is done by
/// [`crate::verifier::bind_fragment_solution`] (run by
/// [`crate::verifier::verify_fragment_manifest`]). The BGP-scan-slot binding of a
/// disclosed solution's variables to a scan sub-proof's disclosed rows (a scan
/// discloses `r` rows; the per-solution row-selection model is the [`scan_rows`]
/// field) is done by [`crate::verifier::bind_fragment_scans`] (sq-qyfth, also run
/// by [`crate::verifier::verify_fragment_manifest`]). The flat cross-graph Q6
/// non-bnode obligation per branch AND the existential coherence of a variable
/// shared between a scan slot and a `PathReach` endpoint (`src_enc`/`dst_enc`) are
/// enforced by [`crate::verifier::bind_fragment_join_coherence`] (sq-ygk6x, also run
/// by [`crate::verifier::verify_fragment_manifest`]). What those gates STILL DEFER is
/// explicit: the salt-uniqueness gate covers only SCAN-referenced committed graphs
/// (so a cross-graph join through a single-graph PATH graph is an agreement check
/// pending path-graph salt coverage; a multi-graph path is refused fail-closed), and
/// an EXISTENTIAL (non-projected) path endpoint's VALUE (hidden by design). Like
/// every gate here, they assert NO soundness / privacy property (sq-qhy4).
///
/// [`solution`]: BranchWitness::solution
/// [`scan_rows`]: BranchWitness::scan_rows
// [OPUS-4.8] sq-3kd2g.6: per-solution UNION branch attribution + obligation
// binding schema. Opt-in (`extended-fragment`), NOT-yet-sound.
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchWitness {
    /// Which `UNION` branch (index into the query-re-derived
    /// `fragment_query.branches`) this disclosed solution witnesses. Out-of-range
    /// => fail-closed (the "wrong branch" rejection).
    pub branch: usize,
    /// The `sub_proofs` indices discharging this branch's BGP-scan obligations,
    /// one per `branch.patterns`, in query-text order. Each must be a bound
    /// [`ProofInputs::Scan`] sub-proof.
    #[serde(default)]
    pub scan_proofs: Vec<usize>,
    /// The `sub_proofs` indices discharging this branch's bounded-path
    /// obligations, one per `branch.path_reach`, in query-text order. Each must be
    /// a bound [`ProofInputs::PathReach`] sub-proof of the member the closure
    /// requires (matching `depth_bound` / `allow_zero`).
    #[serde(default)]
    pub path_proofs: Vec<usize>,
    /// The chosen row index into each VALUES block, one per `branch.values`, in
    /// query-text order. Each must index a real re-derived row of that block
    /// (out-of-range => fail-closed).
    #[serde(default)]
    pub values_rows: Vec<usize>,
    /// The PER-SOLUTION BGP-scan ROW SELECTION (sq-qyfth): for each BGP-scan
    /// obligation (one per `branch.patterns`, parallel to [`scan_proofs`]), the
    /// index of the DISCLOSED matched row of that scan sub-proof
    /// ([`ProofInputs::Scan::rows`]) that supports THIS solution. The verifier
    /// ([`crate::verifier::bind_fragment_scans`]) binds each solution variable
    /// occurring in the scan pattern to the selected row's slot value (re-derived
    /// from the disclosed solution + query text, never a prover encoding) and
    /// checks join coherence across atoms sharing a variable. Empty (or shorter
    /// than [`scan_proofs`]) is back-compatible for a branch whose scan patterns
    /// are all-constant; a scan pattern that carries ANY variable with no selected
    /// row is refused fail-closed. An index outside the scan's ACTIVE disclosed
    /// rows is refused fail-closed.
    ///
    /// [`scan_proofs`]: BranchWitness::scan_proofs
    // [OPUS-4.8] sq-qyfth: per-solution BGP scan-slot row selection.
    #[serde(default)]
    pub scan_rows: Vec<usize>,
    /// The DISCLOSED solution bindings (sq-1zf94): the variable→term assignment
    /// the relying party reads for THIS solution. The verifier re-encodes each
    /// term itself and binds it to the proof-bound `PathReach`
    /// `src_enc`/`dst_enc` and the query's `VALUES` cells
    /// ([`crate::verifier::bind_fragment_solution`]) — so an accepted proof's
    /// disclosed path endpoints / VALUES-constrained variables are tied to the
    /// specific terms here (fail-closed on any mismatch). Every PROJECTED path
    /// endpoint variable MUST appear here; an EXISTENTIAL (non-projected) endpoint
    /// stays hidden and is not term-bound. Empty => no disclosed-term binding
    /// (only the query-CONSTANT path predicate/endpoints are bound).
    // [OPUS-4.8] sq-1zf94: disclosed-solution term binding.
    #[serde(default)]
    pub solution: Vec<SolutionBinding>,
}

/// The full query-result proof manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofManifest {
    /// Schema marker (URN registry convention, mirrors `<urn:sparq:zk>`).
    #[serde(default = "default_type")]
    pub r#type: String,
    /// The SPARQL query text the proof attests a result for. The verifier
    /// re-parses this (sparq-zk `verify::recheck`) — it is NOT trusted.
    pub query: String,
    /// did:key issuer references for the committed graphs (length = number of
    /// distinct issuers; informational provenance — the cryptographic check is
    /// `key_set` + `commitment_attestations` below).
    #[serde(default)]
    pub issuers: Vec<String>,
    /// The prover's DECLARED key-set (audit #3): the issuer public keys the
    /// prover claims its commitments draw on, as hex (compressed Baby-JubJub
    /// points). This is informational / a narrowing claim ONLY — it is NOT the
    /// trust anchor.
    ///
    /// # Codex #1 soundness fix
    /// The verifier's trust anchor `K` is an EXTERNAL relying-party input
    /// ([`crate::verifier::KeySet`]), passed into
    /// [`crate::verifier::verify_manifest`], NOT this field. Trusting this
    /// prover-supplied field as the anchor was a soundness hole: a prover signs a
    /// forged commitment with its own key and self-lists it here. The verifier
    /// now (a) checks every attestation key against the EXTERNAL `K`, and (b)
    /// requires this declared `key_set` to be a SUBSET of the external `K` (a
    /// prover may narrow but never widen the trust set). An empty external `K`
    /// trusts no issuer — any scan carrying commitments is then rejected.
    // [OPUS-4.8] audit #3 / codex #1: prover-declared, NOT the trust anchor.
    #[serde(default)]
    pub key_set: Vec<String>,
    /// Issuer attestations over the per-graph commitments (audit #3): one per
    /// distinct `commitments[g]` value that any scan sub-proof carries. The
    /// verifier requires every scan commitment to have a matching attestation
    /// whose signature is valid and whose key is in `key_set`.
    // [OPUS-4.8] audit #3.
    #[serde(default)]
    pub commitment_attestations: Vec<CommitmentAttestation>,
    /// Per-pattern graph attribution sets (which committed graph indices each
    /// BGP pattern may draw from) — fed to `verify::recheck` for the Q6
    /// cross-graph bnode-join guard.
    pub attributions: Vec<Vec<usize>>,
    /// [OPUS-5] sq-q9r5e follow-up: the EXPLICIT per-pattern answering-scan
    /// declaration — `pattern_scans[pi]` is the set of `sub_proofs` indices of
    /// the SCAN sub-proofs that answer query BGP pattern `pi`. Indexed per query
    /// pattern in query order, exactly like [`Self::attributions`].
    ///
    /// # It carries NO verification weight — do not read it as one
    /// The verifier resolves pattern→scan by constant MEMBERSHIP
    /// (`crate::verifier`'s `scan_matches_pattern`) for EVERY obligation it
    /// derives: the FILTER slot gate (`bind_query_correctness`), the cross-graph
    /// attribution gate (`bind_attributions`), the Q6 namespace
    /// (`global_attributions`) and `bind_joins` all ignore this field. A
    /// declaration therefore CANNOT shrink what the verifier demands — a manifest
    /// carrying one is never accepted where the same manifest without one is
    /// rejected; it can only fail ADDITIONALLY, on the well-formedness checks
    /// below.
    ///
    /// # Why it does not narrow (the residual this field is a placeholder for)
    /// The reason to declare a mapping is the same-constant-layout over-demand:
    /// two query patterns sharing a constant layout — `{ ?x <age> ?v . ?x <age>
    /// ?c }`, both `(?, <age>, ?)` — are BOTH matched by BOTH scan sub-proofs, so
    /// under `FILTER(?v >= 18)` the fail-closed sq-q9r5e / audit-L-1 rule demands
    /// a true-verdict `?v >= 18` proof over the `5` of a genuine solution
    /// `(?x = alice, ?v = 25, ?c = 5)`, which no honest prover can supply. Letting
    /// the declaration narrow that obligation would fix the over-demand and open a
    /// hole: SPARQL evaluates each pattern over EVERY compatible committed row and
    /// the query text authorises no prover-chosen partition of the data, so the
    /// prover could drop a constant-compatible scan's rows out of a pattern's
    /// FILTER and attribution obligations by fiat while still disclosing them. The
    /// well-formedness checks below pin only that the declaration is a TOTAL map
    /// of scans to patterns; they establish nothing about whether an excluded scan
    /// contributes to the claimed result. Narrowing needs what the flat manifest
    /// cannot yet express — a claimed result row bound to the selected scan rows
    /// with all shared-variable joins enforced — so it is NOT done, and the
    /// over-demand stands (`crate::verifier`'s `check_pattern_scans` carries the
    /// full argument).
    ///
    /// # It is DECLARED, not trusted — the verifier re-checks it
    /// `crate::verifier`'s `check_pattern_scans` gate rejects a declaration
    /// that is mis-sized (not one entry per query pattern), leaves a pattern
    /// unanswered (empty entry), names a sub-proof that is out of range / not a
    /// scan / whose bb-bound `pattern_is_const`/`pattern_const_enc` do NOT match
    /// the pattern's constants, or leaves a scan sub-proof DANGLING (declared for
    /// no pattern at all). Those are ADDITIONAL rejections, never a relaxation.
    ///
    /// # EMPTY = not declared
    /// An empty vector means "no declaration" and skips the checks above; the
    /// obligations are identical either way, so omitting the field neither weakens
    /// nor strengthens any gate.
    ///
    /// Checked by the FLAT stage-1 gates only. The `extended-fragment` regime
    /// defers all query-text term binding (see `verify_fragment_manifest`), so a
    /// declaration is neither validated nor used there.
    // [OPUS-5] sq-q9r5e follow-up: explicit pattern→scan mapping. Research-grade,
    // NOT externally audited (sq-qhy4).
    #[serde(default)]
    pub pattern_scans: Vec<Vec<usize>>,
    /// Declared non-bnode join obligations (manifest side of the layer-3
    /// gate). `(variable, pattern_i, pattern_j)`.
    #[serde(default)]
    pub join_obligations: Vec<(String, usize, usize)>,
    pub entailment_regime: EntailmentRegime,
    /// The recorded inference steps that justify any DERIVED triples under a
    /// non-`Simple` `entailment_regime` (sq-314). EMPTY for `Simple` (no
    /// inference). For `Rdfs`/`Owl` the verifier (`crate::verifier::bind_entailment`)
    /// re-checks every step is a well-formed, regime-admitted rule instance whose
    /// antecedents are GROUNDED (chain to an earlier step or to a disclosed scan
    /// row) — so the regime claim is enforced, not free metadata. A non-`Simple`
    /// regime with NO grounded steps is rejected (fail-closed). See the
    /// `derivation` module for the honest scope (disclosed-base re-check; the
    /// in-circuit closure proof is deferred).
    // [OPUS-4.8] sq-314: derivation steps for entailment-regime enforcement.
    #[serde(default)]
    pub derivation_steps: Vec<crate::derivation::DerivationStep>,
    pub binding: BindingMode,
    /// The credential's revocation reference (audit #12): which status list,
    /// index, and version. Issuer-bound (see [`RevocationStatus`]). When ANY
    /// scan-covering attestation carries an issuer-bound status reference
    /// ([`CommitmentAttestation::status`]) this MUST be present and match it —
    /// an omitted `revocation` for a status-bound credential is REJECTED
    /// (fail-closed; the prover cannot drop the reference to skip the check).
    ///
    /// # SCALAR by design — ONE reference per presentation (sq-cuvmj)
    /// This field, [`Self::hidden_revocation`] and [`Self::fully_hidden_revocation`]
    /// are all scalar `Option`s: a manifest carries exactly ONE status reference and
    /// at most one liveness proof over it. `crate::verifier::resolve_status_ref`
    /// requires EVERY scan-covering commitment's issuer-signed
    /// [`CommitmentAttestation::status`] to resolve to this ONE reference, so a
    /// presentation carrying two credentials with DISTINCT `(list, index, version)`
    /// references is structurally REJECTED
    /// (`crate::verifier::CheckError::RevocationReferenceMismatch`).
    ///
    /// That rejection is FAIL-CLOSED, not a false-accept: a revoked second
    /// credential cannot be smuggled past the liveness check by pointing
    /// `revocation` at the live one (the review's attempt-5 construction —
    /// `research/zk-bind-composition-review.md` §Finding B). It is, however, an
    /// over-restriction: it limits hidden cross-credential JOINs
    /// (`crate::verifier::bind_joins`) to credentials sharing an IDENTICAL
    /// issuer-signed status slot — in practice intra-credential multi-graph joins.
    /// See that gate's `# Cross-credential scope constraint` section.
    ///
    /// # Pre-registered obligations for any future `Vec` migration (sq-cuvmj)
    /// Promoting these fields to `Vec` to support genuine multi-credential
    /// presentations is a SOUNDNESS-CRITICAL change: four verifier sites currently
    /// read the single reference and would each silently cover only ONE credential.
    /// A migration owes a PER-COMMITMENT re-derivation at every one of them —
    /// dropping any leaves a second credential's liveness UNCHECKED:
    ///
    /// 1. `crate::verifier::bind_issuer_attestations` / `resolve_status_ref` — must
    ///    resolve the reference belonging to THE COMMITMENT under inspection, and
    ///    must still reject a commitment whose attested status matches NO reference
    ///    (an unmatched commitment must never fall through as "no status bound").
    /// 2. `crate::verifier::bind_revocation` — must run the authoritative-snapshot,
    ///    freshness and `bit[index] == 0` checks once PER reference, and reject if
    ///    any referenced credential is revoked or stale (not "some reference is
    ///    live").
    /// 3. `crate::verifier::bind_hidden_revocation` /
    ///    `bind_fully_hidden_revocation` — each proof must be bound to the
    ///    ISSUER-SIGNED `index_commitment` / `ref_commitment` of ITS OWN reference,
    ///    and every reference in a hidden mode must have a matching proof (the
    ///    existing fail-closed `HiddenRevocationRequired` /
    ///    `FullyHiddenRevocationRequired` rule, applied per reference rather than
    ///    once).
    /// 4. `crate::verifier::scan_referenced_messages` and
    ///    `verify_holder_attestation_signature` — both recompute an issuer-signed
    ///    MESSAGE that folds the status digest, so each must fold the digest of the
    ///    reference for THAT commitment; using a global/first reference would make
    ///    the hidden-issuer and holder-binding messages unverifiable (or, worse,
    ///    verifiable under the wrong credential's status).
    ///
    /// The current single-reference invariant is pinned by
    /// `crate::verifier::tests::two_credentials_with_distinct_status_refs_are_rejected`,
    /// which a `Vec` migration must consciously revisit. Not externally audited
    /// (sq-qhy4).
    #[serde(default)]
    pub revocation: Option<RevocationStatus>,
    /// Disclosed status-list snapshots (audit #12): the bitstrings the verifier
    /// checks the credential's status bit against. Keyed (by the verifier) on
    /// `(status_list, version)`. The snapshot matching the credential's
    /// (issuer-bound) `revocation` reference must show `bit[index] == 0` and a
    /// version within the verifier's freshness window. A missing matching
    /// snapshot REJECTS.
    // [OPUS-4.8] audit #12.
    #[serde(default)]
    pub status_snapshots: Vec<StatusListSnapshot>,
    /// The composed sub-proofs (per-property circuit instances).
    pub sub_proofs: Vec<SubProof>,
    /// Binding-consistency edges between sub-proofs.
    #[serde(default)]
    pub binding_edges: Vec<BindingEdge>,
    /// Hidden cross-credential JOIN edges (sq-bwwl / sq-fi03): each ties two scan
    /// sub-proofs' graph commitments to a `join_eq` sub-proof that proves the two
    /// rows share a value at the named slots WITHOUT disclosing it. Empty for a
    /// manifest with no hidden joins (defaults so legacy manifests parse). The
    /// `bind_joins` verifier gate that enforces these is step 4 (sq-sfsi); this
    /// field is the schema it consumes.
    // [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): hidden cross-credential JOIN edges.
    #[serde(default)]
    pub join_edges: Vec<JoinEdge>,
    /// OPTIONAL hidden-index revocation proof (sq-3e5 / sq-h2v): a zero-knowledge
    /// proof that the credential's status bit at its (HIDDEN) index in the
    /// committed status list is UNSET, disclosing neither the index nor the other
    /// bits. The privacy upgrade over the clear-index [`RevocationStatus`] check.
    ///
    /// When present, the verifier checks the proof's PUBLIC Merkle root equals the
    /// root it derives from its OWN AUTHORITATIVE snapshot (the audit-#12 re-audit
    /// trust anchor is preserved) and runs `bb verify` — so the holder's list slot
    /// is never disclosed. The clear `RevocationStatus.index`/snapshot path remains
    /// the interim check; this field is the additive privacy layer. See
    /// `crate::verifier::bind_hidden_revocation`.
    ///
    /// SCALAR for the same reason [`Self::revocation`] is — one hidden-index proof
    /// per presentation, bound to the one issuer-signed reference. The
    /// multi-credential consequence and the obligations any `Vec` migration owes
    /// are pre-registered on [`Self::revocation`] (sq-cuvmj).
    // [OPUS-4.8] sq-3e5 + sq-h2v: hidden-index revocation proof (privacy upgrade).
    #[serde(default)]
    pub hidden_revocation: Option<HiddenIndexRevocation>,
    /// [OPUS-5] sq-kndw: the FULLY-HIDDEN revocation proof — the privacy upgrade
    /// over [`Self::hidden_revocation`], hiding the status-list IRI and version on
    /// top of the index and the liveness bit. Present exactly when
    /// `revocation` is in the FULLY-HIDDEN mode (`status_list`/`index`/`version`
    /// all `None`, `ref_commitment` + `index_commitment` `Some`); a fully-hidden
    /// reference WITHOUT this proof is rejected fail-closed
    /// (`FullyHiddenRevocationRequired` — revocation is never skipped), and this
    /// proof without a fully-hidden reference is likewise rejected (there would be
    /// no issuer-signed commitments to bind it to).
    ///
    /// Deliberately a SEPARATE field from `hidden_revocation` rather than more
    /// `Option`s inside it: the two modes have disjoint public-input vectors and
    /// disjoint trust anchors (an authoritative status-list root vs an accepted-set
    /// root), so keeping them apart makes the illegal mixed state unrepresentable
    /// and leaves the audited committed-index gate's code path untouched.
    ///
    /// Gated by an opt-in [`crate::verifier::RevocationPolicy`] accepted-set depth
    /// (`with_accepted_set_depth`) — with no accepted-set anchor the relying party
    /// has no root to bind the proof to and rejects. NOT externally audited
    /// (sq-qhy4).
    ///
    /// SCALAR for the same reason [`Self::revocation`] is; the `Vec`-migration
    /// obligations are pre-registered on that field (sq-cuvmj).
    // [OPUS-5] sq-kndw: fully-hidden revocation proof. Opt-in, research-grade.
    #[serde(default)]
    pub fully_hidden_revocation: Option<FullyHiddenRevocation>,
    /// OPTIONAL hidden-issuer attestation proofs (sq-z9l): zero-knowledge proofs
    /// that scan-covering commitments were each signed by SOME issuer whose key is
    /// in the committed key set K, WITHOUT disclosing which issuer. The privacy
    /// upgrade over the clear-key `crate::verifier::bind_issuer_attestations`
    /// check. When the policy enables the path (`KeySet::with_hidden_issuer_depth`)
    /// and an entry is present for a commitment, the verifier checks the proof's
    /// PUBLIC `key_set_root` equals the root it derives from its OWN authoritative
    /// KeySet and `message` equals the recomputed issuer-signed message, then runs
    /// `bb verify` — so WHICH authority vouched for the holder is never disclosed.
    /// The clear-key path remains the interim/always-on check; this is the additive
    /// privacy layer. See `crate::verifier::bind_hidden_issuer_attestations`.
    // [OPUS-4.8] sq-z9l: hidden-issuer attestation proofs (privacy upgrade).
    #[serde(default)]
    pub hidden_issuer_attestations: Vec<HiddenIssuerAttestation>,
    /// OPTIONAL in-circuit holder Proof-of-Possession proofs (sq-c2ql, HolderPoP
    /// T6 / B2 — the HIDDEN-key tier). Each proves, in zero knowledge, knowledge of
    /// the holder secret whose public key hashes to the issuer-attested
    /// `holder_pk_digest` of the credential covering [`HolderPokProof::commitment`]
    /// — WITHOUT disclosing the holder key. The verifier
    /// (`crate::verifier::bind_holder_pok`) binds the proof's public digest to the
    /// ISSUER-SIGNED digest (the binding edge: the digest must verify under the
    /// external trusted `K` over [`sparq_zk::sig::commitment_message_with_holder`]),
    /// reconstructs the public inputs from its own nonce + that digest, and `bb
    /// verify`s. The hidden-key analogue of the clear-key
    /// [`BindingMode::HolderPop`]+[`AttestedHolderBinding`] gate (T3/sq-z8s7 B1,
    /// `bind_holder_binding`), which remains the clear-tier holder gate; this is the
    /// additive privacy layer. Empty for a manifest with no in-circuit PoK (defaults
    /// so legacy manifests parse). Gated by an opt-in
    /// [`crate::verifier::HolderBindingPolicy`].
    ///
    /// NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2): this wires the
    /// binding edge, it does NOT make the verifier sound. No soundness/ZK-privacy
    /// claim. See [`HolderPokProof`].
    // [OPUS-4.8] sq-c2ql (HolderPoP T6 / B2): in-circuit holder PoK proofs. Opt-in,
    // NOT-yet-sound.
    #[serde(default)]
    pub holder_pok_proofs: Vec<HolderPokProof>,
    /// OPTIONAL in-circuit hidden-holder SET-membership proofs (sq-3c00, the
    /// HolderPoP hidden-holder-SET anonymity tier). Each proves, in zero knowledge,
    /// knowledge of a holder secret whose public key's digest is a member of a
    /// holder SET committed as the PUBLIC `holder_set_root` — WITHOUT disclosing the
    /// holder key OR which holder. The hidden-holder analogue of the clear-digest
    /// [`HolderPokProof`] (which makes `holder_pk_digest` public), the holder twin
    /// of [`HiddenIssuerAttestation`] (which hides WHICH issuer). The verifier
    /// (`crate::verifier::bind_holder_set`) binds the proof's PUBLIC
    /// `holder_set_root` to the root it derives from its OWN authoritative holder
    /// registry (the trust anchor; WHICH holder is hidden, the trust source is
    /// not), reconstructs the public inputs from its own nonce + that root, and `bb
    /// verify`s. Empty for a manifest with no set-membership proof (defaults so
    /// legacy manifests parse). Gated by an opt-in
    /// [`crate::verifier::HolderRegistry`] depth (`with_hidden_holder_set_depth`).
    ///
    /// NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2): this wires the
    /// membership gate, it does NOT make the verifier sound. No soundness /
    /// ZK-privacy property is asserted as achieved. See [`HolderSetProof`].
    // [OPUS-4.8] sq-3c00 (HolderPoP hidden-holder-SET tier): in-circuit
    // set-membership proofs. Opt-in, NOT-yet-sound.
    #[serde(default)]
    pub holder_set_proofs: Vec<HolderSetProof>,
}

/// A hidden-index revocation (bit-unset) proof (sq-3e5 / sq-h2v): the bb proof
/// produced by the `revoke_unset_d{depth}` circuit, together with the PUBLIC
/// status-list Merkle `root` it was proved against.
///
/// # Trust anchor (preserves audit #12 re-audit)
/// `root` is a PUBLIC input the prover commits, but it is NOT trusted as a prover
/// claim: the verifier recomputes the authoritative root from its OWN
/// [`StatusListSnapshot`] (carried in [`crate::verifier::RevocationPolicy`], the
/// external relying-party trust anchor) and rejects unless the proof's public
/// `root` byte-equals it. So the liveness fact is bound to the relying party's own
/// authenticated status data, exactly as the clear-index path is — the only thing
/// hidden is WHICH index (the linkability channel), never the trust source.
///
/// `depth` selects the circuit member (`revoke_unset_d{depth}`) and MUST equal the
/// depth the relying party uses to derive its authoritative root, so the trees (and
/// roots) are over the same leaf layout.
// [OPUS-4.8] sq-3e5 + sq-h2v.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiddenIndexRevocation {
    /// The Merkle-tree depth (`revoke_unset_d{depth}` member; supports `2^depth`
    /// indices). MUST match the depth the relying party derives its root with.
    pub depth: u32,
    /// The status-list Merkle root the proof was produced against (the proof's
    /// PUBLIC input). Checked byte-equal to the relying party's authoritative root.
    pub root: FieldHex,
    /// [OPUS-4.8] sq-ayv: the hiding index COMMITMENT the proof was produced against
    /// (the proof's SECOND public input, after `root`), hex. The circuit recomputes
    /// it in-circuit from the same private `index` it proves bit-unset for; the
    /// verifier byte-matches it against the ISSUER-SIGNED commitment in
    /// [`RevocationStatus::index_commitment`], cross-binding the proven-unset index
    /// to the index the issuer committed to. `None` for a legacy (sq-3e5) proof
    /// that predates the cross-binding — but a committed-index `RevocationStatus`
    /// REQUIRES this to be `Some` (fail-closed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_commitment: Option<FieldHex>,
    /// The bb proof blob (hex), in the same `len|proof|len|pi|vk` layout as a
    /// [`SubProof::proof_hex`] (see [`crate::verifier::encode_artifacts`]).
    pub proof_hex: String,
}

/// [OPUS-5] sq-kndw: a FULLY-HIDDEN revocation proof — the bb proof produced by
/// the `revoke_hidden_ref_d{depth}_a{set_depth}` circuit
/// ([`CircuitId::RevokeHiddenRef`]), together with the public inputs it commits.
/// The privacy upgrade over [`HiddenIndexRevocation`]: the status-list IRI and the
/// VERSION are hidden as well as the index and the liveness bit.
///
/// # Trust anchor (the audit-#12 anchor, moved behind a commitment)
/// `accepted_set_root` and `min_version` are PUBLIC inputs the prover commits, but
/// NEITHER is trusted as a prover claim. The verifier derives both from its OWN
/// [`crate::verifier::RevocationPolicy`] — the accepted-set root over its
/// freshness-curated `(list, version, status_list_root)` entries, and its own
/// epoch floor — and rejects unless the declared values byte-equal them, then
/// reconstructs the public-input vector from ITS OWN values before `bb verify`.
/// So the liveness fact is still bound to the relying party's own authenticated
/// status bytes; the only new thing hidden is WHICH of its accepted lists/epochs
/// the credential belongs to. No new trust assumption is introduced.
///
/// Because membership is restricted to the freshness-curated window, a stale or
/// future-dated version is not a leaf at all and no proof can be built against it
/// — the audit-#12 freshness gate SURVIVES the move behind the commitment. The
/// in-circuit `version >= min_version` is defence-in-depth on top of that.
///
/// # ⚠️ The re-blinding requirement (the guarantee depends on it)
/// `ref_commitment` and `index_commitment` are HIDING but STABLE per issuance. A
/// holder that presents the SAME pair twice hands the relying party a perfect
/// cross-presentation correlation handle and voids the entire privacy guarantee
/// — this is the single most important operational requirement of the design
/// (`research/zk-statuslist-hide-iri-version.md` §4). The verifier therefore
/// enforces SINGLE-USE of the pair through the same durable
/// [`crate::verifier::SeenNonces`] store the nonce replay defence uses
/// (`FullyHiddenRevocationLinkageReplay`). Honest limit: single-use enforcement
/// protects the holder only against an HONEST relying party — a malicious one can
/// simply not run it, and by then it has already observed the pair. The real fix
/// is upstream: the ISSUER must mint a fresh `(ref_blinding, blinding)` pair and
/// re-sign per presentation (a re-randomisable commitment + signature scheme,
/// which sparq does NOT implement, would remove the round trip).
///
/// NOT externally audited (sq-qhy4). Research-grade; no soundness / ZK-privacy
/// property is asserted as achieved.
// [OPUS-5] sq-kndw: fully-hidden revocation proof (deferred remainder of sq-6qe).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullyHiddenRevocation {
    /// The status-list Merkle depth (the `d{depth}` half of the member name).
    /// MUST equal the depth the relying party derives its per-entry status-list
    /// roots with ([`crate::verifier::RevocationPolicy::with_hidden_index_depth`]).
    pub depth: u32,
    /// The accepted-set Merkle depth (the `a{set_depth}` half of the member name).
    /// MUST equal [`crate::verifier::RevocationPolicy::with_accepted_set_depth`].
    pub set_depth: u32,
    /// The hiding `(list, version)` reference commitment the proof was produced
    /// against (public input 1). Byte-matched against the ISSUER-SIGNED
    /// [`RevocationStatus::ref_commitment`] — the cross-binding that ties the
    /// in-circuit private `(list, version)` to the issuer's reference.
    pub ref_commitment: FieldHex,
    /// The hiding index commitment the proof was produced against (public input 2).
    /// Byte-matched against the ISSUER-SIGNED [`RevocationStatus::index_commitment`]
    /// exactly as on the committed-index path (sq-ayv).
    pub index_commitment: FieldHex,
    /// The accepted-set Merkle root the proof was produced against (public input
    /// 3). Checked byte-equal to
    /// [`crate::verifier::RevocationPolicy::accepted_set_root`].
    pub accepted_set_root: FieldHex,
    /// The public epoch FLOOR the proof was produced against (public input 4).
    /// Checked equal to [`crate::verifier::RevocationPolicy::min_version`].
    pub min_version: u64,
    /// The bb proof blob (hex), in the same `len|proof|len|pi|vk` layout as a
    /// [`SubProof::proof_hex`] (see [`crate::verifier::encode_artifacts`]).
    pub proof_hex: String,
}

/// A hidden-issuer attestation (sq-z9l): a bb proof produced by the
/// `hidden_issuer_d{depth}` circuit that the commitment message `m` was signed by
/// SOME issuer whose public key is a member of the key set K committed as the
/// PUBLIC Poseidon2 Merkle `key_set_root` — WITHOUT disclosing which issuer.
///
/// # Trust anchor (preserves the audit #3 external-K anchor — load-bearing)
/// `key_set_root` is a PUBLIC input the prover commits, but it is NOT trusted as a
/// prover claim: the verifier recomputes the authoritative root from its OWN
/// [`crate::verifier::KeySet`] (canonical order) at `depth` and rejects unless the
/// proof's public `key_set_root` byte-equals it. So the "in K" fact is bound to
/// the relying party's own trust anchor, exactly as the clear-key path is — the
/// only thing hidden is WHICH key (the deanonymising channel), never the trust
/// source. The privacy upgrade over the clear-key
/// `crate::verifier::bind_issuer_attestations` check.
///
/// `m` is the issuer-signed commitment message the proof binds. In v1 this is the
/// status-bound `commitment_message_with_status(C(G), salt, status_ref)` (the
/// SAME message the clear path binds), so the verifier recomputes `m` from the
/// disclosed commitment/salt/reference and requires the proof's PUBLIC `m` to
/// match — tying the hidden-issuer proof to a specific committed graph.
///
/// `depth` selects the circuit member (`hidden_issuer_d{depth}`) and MUST equal
/// the depth the relying party derives its root with.
// [OPUS-4.8] sq-z9l: hidden-issuer attestation (privacy upgrade).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiddenIssuerAttestation {
    /// The commitment this attestation covers — must match a scan sub-proof's
    /// `commitments[g]` (so the verifier can recompute the signed message `m`).
    pub commitment: FieldHex,
    /// The Merkle-tree depth (`hidden_issuer_d{depth}` member; supports `2^depth`
    /// issuers). MUST match the depth the relying party derives its root with.
    pub depth: u32,
    /// The key-set Merkle root the proof was produced against (the proof's PUBLIC
    /// input). Checked byte-equal to the relying party's authoritative root.
    pub key_set_root: FieldHex,
    /// The issuer-signed commitment message `m` the proof binds (the proof's
    /// PUBLIC input). Checked equal to the message the verifier recomputes from the
    /// disclosed commitment + salt + status reference.
    pub message: FieldHex,
    /// The per-graph RDFC10 bnode salt this commitment was committed under (audit
    /// #9), hex. Carried HERE so the verifier can recompute the issuer-signed
    /// message `m = commitment_message_with_status(C(G), salt, status_ref)` for a
    /// HIDDEN-ONLY commitment — one with NO clear [`CommitmentAttestation`] from
    /// which to read the salt (sq-xxg). The salt is NOT the privacy target (only
    /// WHICH issuer signed is hidden); disclosing it is consistent with the clear
    /// path, and it still participates in the salt-uniqueness guarantee (audit #9).
    ///
    /// When `None`, the verifier falls back to the salt from the clear attestation
    /// over the same commitment (the additive mode, where a clear attestation also
    /// exists — the original sq-z9l behaviour). A hidden-ONLY commitment (no clear
    /// attestation) MUST carry the salt here, or the verifier cannot recompute `m`
    /// and rejects the entry as unreferenced (fail-closed).
    ///
    /// # Why the salt is not withheld (sq-93h, assessed — do not re-litigate)
    /// Disclosing it adds NO cross-presentation linkability, because [`Self::commitment`]
    /// — the SAME graph's `C(G)` — is disclosed in the clear on this very entry and is
    /// byte-bound into every scan sub-proof's bb public inputs. Given the audit-#9
    /// ISSUANCE discipline that a salt is never reused for two distinct graphs (an
    /// issuance-side assumption — the verifier machine-checks only the within-manifest
    /// instance of it, `SaltReused`), the `graph -> salt` partition REFINES the
    /// `graph -> C(G)` one, so the salt is a dominated correlator: a coalition that can
    /// link two presentations by salt can already link them by `C(G)`. Moving `m`-reconstruction
    /// behind an in-circuit salt-commitment (the sq-ayv index-commitment analogue) would
    /// therefore buy zero unlinkability for a new circuit member and VK. Full analysis,
    /// including the separate (non-linkability) guess-confirmation residual and the
    /// trip-wire that fires if `C(G)` ever stops being disclosed:
    /// `research/zk-hidden-path-salt-disclosure.md`.
    // [OPUS-4.8] sq-xxg: salt for hidden-only `m` reconstruction.
    // [OPUS-5] sq-93h: assessed NO-BUILD — dominated by the clear `commitment`.
    #[serde(default)]
    pub salt: Option<FieldHex>,
    /// The bb proof blob (hex), same `len|proof|len|pi|vk` layout as a
    /// [`SubProof::proof_hex`].
    pub proof_hex: String,
}

/// An in-circuit holder Proof-of-Possession (sq-c2ql, HolderPoP T6 / B2 — the
/// HIDDEN-key tier): a bb proof produced by the `holder_pok` circuit member
/// ([`crate::CircuitId::HolderPok`]) that the prover knows a holder secret `hsk`
/// whose public key `hpk = hsk·G` hashes to `Poseidon2([ZKSIG_HK, hpk.x, hpk.y]) =
/// holder_pk_digest` — WITHOUT disclosing `hsk` OR `hpk` (only the digest is
/// public). The hidden-key analogue of the clear-key
/// [`BindingMode::HolderPop`]+[`AttestedHolderBinding`] path
/// (`crate::verifier::bind_holder_binding`, T3/sq-z8s7 B1): there the presenter
/// discloses `hpk` and the verifier recomputes the digest host-side; here `hpk`
/// stays private and possession is proved in zero knowledge.
///
/// # The binding edge (the load-bearing tie — sq-c2ql)
/// `holder_pk_digest` is the proof's PUBLIC input, but it is NOT trusted as a
/// prover claim. The verifier (`crate::verifier::bind_holder_pok`) reads the
/// digest from the ISSUER-ATTESTED [`AttestedHolderBinding::holder_pk_digest`] on
/// the attestation COVERING this credential's scan-referenced `commitment`, and —
/// crucially — anchors that digest in the issuer's Schnorr signature (it must
/// verify over [`sparq_zk::sig::commitment_message_with_holder`], the same
/// ZKSIG_C4 anchor T3/B1 uses, under the EXTERNAL trusted `K`). The verifier then
/// reconstructs the proof's public inputs from the verifier nonce + THAT
/// issuer-signed digest and requires the proof's public inputs to byte-equal them.
/// So the proven holder key is cryptographically bound to the issuer-attested
/// credential: a malicious holder A who does NOT hold `hsk_B` cannot produce a
/// satisfying `holder_pok` witness for B's issuer-signed digest (DL-hardness on
/// Baby-JubJub + proof soundness), and cannot substitute its own digest without
/// invalidating the issuer's EUF-CMA signature.
///
/// `challenge` is the proof's OTHER public input (the verifier's fresh nonce, the
/// audit-#4 replay binding shared by the whole circuit family); the verifier feeds
/// its own nonce, never the prover's declared bytes.
///
/// Absent => no in-circuit holder PoK is presented; the clear-key
/// [`BindingMode::HolderPop`] path remains the holder gate (additive, like
/// [`HiddenIssuerAttestation`] over the clear-key attestation path). Gated by an
/// opt-in relying-party policy ([`crate::verifier::HolderBindingPolicy`]).
///
/// # SOUNDNESS (load-bearing, NOT a security claim)
/// This wires the binding edge; it does NOT make the composition verifier sound.
/// The verifier is NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2) and
/// this member inherits that — a passing proof is NOT a guarantee under an
/// adversarial prover, and there is NO external accredited-cryptographer sign-off
/// (sq-qhy4 pending). Research-grade, opt-in. No soundness/ZK-privacy claim is made.
// [OPUS-4.8] sq-c2ql (HolderPoP T6 / B2): in-circuit holder PoK + issuer-attested
// credential binding edge. NOT-yet-sound (sq-qhy4); opt-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderPokProof {
    /// The commitment this holder PoK covers — must match a scan sub-proof's
    /// `commitments[g]`, so the verifier can resolve the COVERING issuer
    /// attestation (and thus the issuer-signed `holder_pk_digest` the proof's
    /// public input must equal). A PoK over a commitment no verified scan
    /// references is a dangling proof, rejected fail-closed.
    pub commitment: FieldHex,
    /// The bb proof blob (hex), same `len|proof|len|pi|vk` layout as a
    /// [`SubProof::proof_hex`] (see [`crate::verifier::encode_artifacts`]). Its
    /// public inputs are `[challenge, holder_pk_digest]` in `holder_pok` main's
    /// declaration order; the verifier reconstructs them from its own nonce + the
    /// issuer-attested digest and byte-compares.
    pub proof_hex: String,
}

/// An in-circuit hidden-holder SET-membership proof (sq-3c00, the HolderPoP
/// hidden-holder-SET anonymity tier): a bb proof produced by the
/// `holder_set_d{depth}` circuit member ([`crate::CircuitId::HolderSet`]) that the
/// prover knows a holder secret `hsk` whose public key `hpk = hsk·G` has a
/// holder-key digest `Poseidon2([ZKSIG_HK, hpk.x, hpk.y])` that is a MEMBER of the
/// holder SET committed as the PUBLIC Poseidon2 Merkle `holder_set_root` — WITHOUT
/// disclosing `hsk`, `hpk`, OR which holder. The hidden-holder analogue of the
/// clear-digest [`HolderPokProof`] (which makes `holder_pk_digest` public, so the
/// verifier learns the holder is the SPECIFIC hidden-key party bound to one
/// credential); the holder twin of [`HiddenIssuerAttestation`] (which hides WHICH
/// issuer signed).
///
/// # Trust anchor (mirrors the hidden-issuer external-K anchor — load-bearing)
/// `holder_set_root` is a PUBLIC input the prover commits, but it is NOT trusted as
/// a prover claim: the verifier (`crate::verifier::bind_holder_set`) recomputes
/// the authoritative root from its OWN [`crate::verifier::HolderRegistry`]
/// (canonical order) at `depth` and rejects unless the proof's public
/// `holder_set_root` byte-equals it. So the "in the set" fact is bound to the
/// relying party's own holder registry, exactly as the clear-key holder path is —
/// the only thing hidden is WHICH holder (the deanonymising channel), never the
/// trust source.
///
/// `depth` selects the circuit member (`holder_set_d{depth}`) and MUST equal the
/// depth the relying party derives its root with.
///
/// # SOUNDNESS (load-bearing, NOT a security claim)
/// This wires the membership gate; it does NOT make the composition verifier sound.
/// The verifier is NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2) and
/// this member inherits that — a passing proof is NOT a guarantee under an
/// adversarial prover, and there is NO external accredited-cryptographer sign-off
/// (sq-qhy4 pending). Research-grade, opt-in. No soundness / ZK-privacy property is
/// asserted as achieved.
// [OPUS-4.8] sq-3c00 (HolderPoP hidden-holder-SET tier): in-circuit set-membership
// proof. Opt-in, NOT-yet-sound (sq-qhy4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderSetProof {
    /// The commitment this set-membership proof covers — must match a scan
    /// sub-proof's `commitments[g]`, so the proof is tied to a credential the
    /// presentation actually uses. A proof over a commitment no verified scan
    /// references is a dangling proof, rejected fail-closed.
    pub commitment: FieldHex,
    /// The Merkle-tree depth (`holder_set_d{depth}` member; supports `2^depth`
    /// holders). MUST match the depth the relying party derives its root with.
    pub depth: u32,
    /// The holder-set Merkle root the proof was produced against (the proof's
    /// PUBLIC input). Checked byte-equal to the relying party's authoritative root.
    pub holder_set_root: FieldHex,
    /// The bb proof blob (hex), same `len|proof|len|pi|vk` layout as a
    /// [`SubProof::proof_hex`] (see [`crate::verifier::encode_artifacts`]). Its
    /// public inputs are `[challenge, holder_set_root]` in `holder_set_d{depth}`
    /// main's declaration order; the verifier reconstructs them from its own nonce
    /// + the authoritative root and byte-compares.
    pub proof_hex: String,
}

fn default_type() -> String {
    "urn:sparq:zk:ProofManifest".to_string()
}

impl ProofManifest {
    /// Canonicalise the edge vectors to a deterministic order ([OPUS-4.8]
    /// sq-y2wy). Sorts `binding_edges` ascending by `(from_proof, from_row,
    /// from_slot, to_proof)` and `join_edges` ascending by `(scan_a, graph_a,
    /// scan_b, graph_b, join_proof)` — each the struct's field-declaration tuple
    /// (the derived [`Ord`]), which is a TOTAL order over edges, so the result is
    /// independent of insertion order.
    ///
    /// # Reference validity is preserved
    /// Each edge is SELF-CONTAINED: it carries its own `sub_proofs` indices
    /// (`from_proof`/`to_proof`; `scan_a`/`scan_b`/`join_proof`). The verifier
    /// (`binding_edges` stage 2 / `bind_joins` stage 2g) resolves every edge by
    /// THOSE indices, never by the edge's position in the vector — so reordering
    /// the vector cannot invalidate any reference, and verification still accepts
    /// a canonicalised manifest.
    ///
    /// # Determinism contract
    /// Two manifests that differ ONLY in edge order produce IDENTICAL canonical
    /// forms — hence the same `to_json` bytes and the same hash. [`Self::to_json`]
    /// canonicalises before serialising, so a manifest's on-the-wire/hashable form
    /// is always canonical regardless of construction order. Call this directly
    /// when a canonical in-memory manifest is needed (e.g. before equality checks
    /// on the struct itself).
    // [OPUS-4.8] sq-y2wy: canonical edge ordering for binding_edges + join_edges.
    pub fn canonicalize(&mut self) {
        // [OPUS-4.8] sq-y2wy: edges are distinct under derived total `Ord`
        // (no equal elements), so an unstable sort is deterministic here and
        // avoids the auxiliary allocation a stable sort may perform.
        self.binding_edges.sort_unstable();
        self.join_edges.sort_unstable();
    }

    /// Serialise to canonical pretty JSON. The edge vectors are canonicalised
    /// ([`Self::canonicalize`]) on a CLONE first, so the serialised form is
    /// deterministic in edge order WITHOUT mutating `self`: two manifests that
    /// differ only in edge order produce byte-identical JSON (and thus the same
    /// hash over those bytes).
    // [OPUS-4.8] sq-y2wy: canonicalise edge order before serialisation.
    pub fn to_json(&self) -> String {
        let mut canonical = self.clone();
        canonical.canonicalize();
        serde_json::to_string_pretty(&canonical).expect("manifest is serializable")
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

/// A WAVE-1 EXTENDED-FRAGMENT presentation (sq-3kd2g.6): a stage-1
/// [`ProofManifest`] PLUS the per-solution UNION branch attribution
/// ([`BranchWitness`]) the extended fragment (property paths / `UNION` / `VALUES`
/// / subquery) needs.
///
/// It is a distinct WRAPPER — not a new [`ProofManifest`] field — so the stage-1
/// manifest schema and its (many) construction sites are byte-unchanged, and the
/// whole extended surface stays behind the opt-in `extended-fragment` feature
/// (the default verifier surface is untouched). A relying party verifying an
/// extended-fragment presentation runs [`crate::verifier::dispatch_fragment`]
/// over this wrapper (the fail-closed routing gate) AND
/// [`crate::verifier::verify_manifest`] over the embedded [`Self::manifest`] (the
/// crypto binding of each sub-proof).
///
/// # Honest scope
/// `dispatch_fragment` is the STRUCTURAL routing gate. Its integration into the
/// `verify_manifest` crypto flow for an end-to-end path/`UNION`/`VALUES` accept
/// (which additionally needs the disclosed-solution term binding) is a documented
/// follow-up bead. NOT externally audited (sq-qhy4); no soundness/privacy claim.
// [OPUS-4.8] sq-3kd2g.6: extended-fragment presentation wrapper. Opt-in
// (`extended-fragment`), research-grade, NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentManifest {
    /// Schema marker (URN registry convention, mirrors `ProofManifest`'s `type`).
    #[serde(default = "default_fragment_type")]
    pub r#type: String,
    /// The embedded stage-1 proof manifest (sub-proofs, attestations, binding,
    /// revocation — everything [`crate::verifier::verify_manifest`] consumes).
    pub manifest: ProofManifest,
    /// One [`BranchWitness`] per disclosed solution, attributing it to a `UNION`
    /// branch and naming the embedded manifest's `sub_proofs` that discharge that
    /// branch's obligations. Empty => no extended-fragment attribution (the gate
    /// then only re-derives the query fragment and checks the embedded sub-proofs
    /// carry known circuit ids).
    #[serde(default)]
    pub branch_witnesses: Vec<BranchWitness>,
}

#[cfg(feature = "extended-fragment")]
fn default_fragment_type() -> String {
    "urn:sparq:zk:FragmentManifest".to_string()
}

#[cfg(feature = "extended-fragment")]
impl FragmentManifest {
    /// Wrap a stage-1 [`ProofManifest`] with its branch attribution.
    pub fn new(manifest: ProofManifest, branch_witnesses: Vec<BranchWitness>) -> Self {
        FragmentManifest {
            r#type: default_fragment_type(),
            manifest,
            branch_witnesses,
        }
    }

    /// Serialise to pretty JSON (the embedded manifest is canonicalised via
    /// [`ProofManifest::to_json`]'s edge-ordering discipline on serialise).
    pub fn to_json(&self) -> String {
        // Canonicalise the embedded manifest's edge vectors, mirroring
        // `ProofManifest::to_json`, without mutating `self`.
        let mut canonical = self.clone();
        canonical.manifest.canonicalize();
        serde_json::to_string_pretty(&canonical).expect("fragment manifest is serializable")
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

// [OPUS-4.8] sq-h8rg (HolderPoP T2): AttestedHolderBinding schema + digest-wiring tests.
#[cfg(test)]
mod holder_binding_tests {
    use super::*;
    use sparq_zk::sig::{holder_key_digest, public_key_to_hex, SecretKey};

    fn holder_pk(seed: u64) -> PublicKey {
        SecretKey::from_seed(seed).public_key()
    }

    /// A non-holder-bound (bearer) `CommitmentAttestation` — the existing
    /// audit-#3/#9/#12 shape, with `holder: None`. Used to assert additivity.
    fn bearer_attestation() -> CommitmentAttestation {
        let sk = SecretKey::from_seed(11);
        let commitment = Fr::from(7u64);
        CommitmentAttestation {
            commitment: FieldHex::from_field(&commitment),
            issuer_public_key: public_key_to_hex(&sk.public_key()),
            signature: sk.sign_commitment(&commitment),
            cryptosuite: "https://sparq.dev/ns/zk#poseidon2-schnorr-v1".to_string(),
            salt: None,
            status: None,
            holder: None,
        }
    }

    /// The new schema round-trips through serde (both tiers), and the wired
    /// digest equals T1's `holder_key_digest` for the given holder key.
    #[test]
    fn attested_holder_binding_round_trips_and_wires_t1_digest() {
        let hpk = holder_pk(42);
        let expected = holder_key_digest(&hpk).expect("non-identity holder key");

        // Clear-key tier (discloses hpk).
        let clear = AttestedHolderBinding::from_holder_key(&hpk, true)
            .expect("non-identity holder key builds");
        assert_eq!(
            clear.digest(),
            Some(expected),
            "wired digest must equal T1 holder_key_digest"
        );
        assert_eq!(
            clear.holder_key(),
            Some(hpk),
            "clear tier exposes the disclosed holder key"
        );
        let json = serde_json::to_string(&clear).expect("serializes");
        let back: AttestedHolderBinding = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, clear, "clear-tier binding round-trips");

        // Hidden-key tier (digest only; no clear hpk).
        let hidden = AttestedHolderBinding::from_holder_key(&hpk, false)
            .expect("non-identity holder key builds");
        assert_eq!(
            hidden.digest(),
            Some(expected),
            "hidden tier carries the same digest"
        );
        assert_eq!(
            hidden.holder_public_key, None,
            "hidden tier discloses no clear key"
        );
        assert_eq!(
            hidden.holder_key(),
            None,
            "hidden tier exposes no clear key"
        );
        let json = serde_json::to_string(&hidden).expect("serializes");
        // The clear key is omitted from the JSON entirely (skip_serializing_if).
        assert!(
            !json.contains("holder_public_key"),
            "hidden tier omits the clear-key field"
        );
        let back: AttestedHolderBinding = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, hidden, "hidden-tier binding round-trips");
    }

    /// `from_holder_key` propagates T1's identity-key rejection fail-closed. The
    /// identity `PublicKey` itself cannot be CONSTRUCTED in this crate (no `ark`
    /// dep, and `public_key_from_hex` rejects it at parse — codex #3), and T1
    /// already proves `holder_key_digest(identity) == Err(IdentityKey)`
    /// (`sparq_zk::sig` `identity_holder_key_digest_rejected`); the binding builder
    /// is a thin `?`-propagating wrapper over it, so the rejection holds by
    /// construction. Asserted here only at the type level (the builder returns a
    /// `Result<_, HolderKeyError>`, never an infallible value).
    #[test]
    fn from_holder_key_is_fallible_on_the_holder_key_digest() {
        // A non-identity key is the success path; the only error variant the
        // wrapper can yield is the identity rejection forwarded from T1.
        let ok: Result<AttestedHolderBinding, HolderKeyError> =
            AttestedHolderBinding::from_holder_key(&holder_pk(7), false);
        assert!(ok.is_ok(), "a real holder key builds a binding");
    }

    /// A full `ProofManifest` carrying a holder-bound `CommitmentAttestation`
    /// round-trips through `to_json`/`from_json` and exposes BOTH the issuer
    /// signature (the attestation's `signature`) and the holder digest.
    #[test]
    fn manifest_with_attested_holder_binding_round_trips_and_exposes_digest_and_signature() {
        let issuer = SecretKey::from_seed(3);
        let hpk = holder_pk(99);
        let commitment = Fr::from(123u64);
        let salt = Fr::from(456u64);
        let list_id = sparq_zk::sig::status_list_id_to_field("urn:sparq:status:list:1");
        let status_ref = sparq_zk::sig::status_ref_digest(&list_id, 5, 1);
        let holder_digest = holder_key_digest(&hpk).expect("non-identity holder key");

        // The issuer signs the T1 holder-bound message variant (ZKSIG_C4).
        let signature =
            issuer.sign_commitment_with_holder(&commitment, &salt, &status_ref, &holder_digest);

        let att = CommitmentAttestation {
            commitment: FieldHex::from_field(&commitment),
            issuer_public_key: public_key_to_hex(&issuer.public_key()),
            signature: signature.clone(),
            cryptosuite: "https://sparq.dev/ns/zk#poseidon2-schnorr-v1".to_string(),
            salt: Some(FieldHex::from_field(&salt)),
            status: Some(AttestedStatusRef {
                index: Some(5),
                version: Some(1),
                index_commitment: None,
                ref_commitment: None,
            }),
            holder: AttestedHolderBinding::from_holder_key(&hpk, true).ok(),
        };

        let manifest = ProofManifest {
            r#type: default_type(),
            query: "ASK {}".to_string(),
            issuers: vec![],
            key_set: vec![public_key_to_hex(&issuer.public_key())],
            commitment_attestations: vec![att],
            attributions: vec![],
            pattern_scans: vec![],
            join_obligations: vec![],
            entailment_regime: EntailmentRegime::Simple,
            derivation_steps: vec![],
            binding: BindingMode::HolderPop {
                challenge: FieldHex::from_field(&Fr::from(0x2au64)),
                holder: public_key_to_hex(&hpk),
                pop: SecretKey::from_seed(99).sign_holder_pop(&Fr::from(0x2au64)),
                cryptosuite: default_holder_cryptosuite(),
            },
            revocation: None,
            status_snapshots: vec![],
            sub_proofs: vec![],
            binding_edges: vec![],
            join_edges: vec![],
            hidden_revocation: None,
            hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
            fully_hidden_revocation: None,
        };

        let json = manifest.to_json();
        let back = ProofManifest::from_json(&json).expect("manifest round-trips");
        assert_eq!(back, manifest, "manifest with holder binding round-trips");

        // The parsed manifest exposes the issuer signature and the holder digest.
        let binding = back.commitment_attestations[0]
            .holder
            .as_ref()
            .expect("attestation carries an attested holder binding");
        assert_eq!(
            binding.digest(),
            Some(holder_digest),
            "parsed binding exposes the issuer-attested holder digest"
        );
        assert_eq!(
            back.commitment_attestations[0].signature, signature,
            "parsed attestation exposes the (holder-bound) issuer signature"
        );

        // The disclosed holder key in the binding-mode PoP wires through to the
        // SAME T1 digest the attestation carries (the T3 cross-check anchor).
        assert_eq!(
            back.binding.holder_key_digest(),
            Some(holder_digest),
            "presented holder key digest matches the issuer-attested digest"
        );
        assert_eq!(back.binding.holder_key(), Some(hpk));
    }

    /// Back-compat: an OLD manifest JSON whose attestation has NO `holder` field
    /// (and a manifest with no `holder` key anywhere) still parses, with the new
    /// field defaulting to `None`. The schema addition is purely additive.
    #[test]
    fn old_manifest_without_holder_field_still_parses() {
        // Attestation-level back-compat: serialize a bearer attestation, confirm
        // it carries no `holder` JSON key when None is omitted on deserialize...
        let att = bearer_attestation();
        let json = serde_json::to_string(&att).expect("serializes");
        // `holder: None` serializes (no skip on the field) but a JSON missing it
        // must still parse via #[serde(default)].
        //
        // [OPUS-4.8] Drop the `holder` key STRUCTURALLY (parse → remove key on the
        // attestation object → re-serialize) rather than via brittle raw-string
        // `.replace()`, so this back-compat test survives field-order, whitespace,
        // and future-field changes to the serialized form.
        let mut value: serde_json::Value =
            serde_json::from_str(&json).expect("attestation JSON is a value");
        assert!(
            value
                .as_object_mut()
                .expect("attestation serializes as a JSON object")
                .remove("holder")
                .is_some(),
            "the serialized attestation carries a `holder` key to remove"
        );
        let stripped = serde_json::to_string(&value).expect("re-serializes");
        assert!(
            !stripped.contains("\"holder\""),
            "stripped the holder field"
        );
        let back: CommitmentAttestation =
            serde_json::from_str(&stripped).expect("old attestation (no holder field) parses");
        assert_eq!(back.holder, None, "absent holder field defaults to None");
        assert_eq!(back, att, "back-compat parse equals the bearer attestation");

        // Manifest-level back-compat: a hand-written legacy manifest with no
        // holder field anywhere parses, and the attestation's holder is None.
        let legacy = r#"{
            "type": "urn:sparq:zk:ProofManifest",
            "query": "ASK {}",
            "commitment_attestations": [{
                "commitment": "0x0000000000000000000000000000000000000000000000000000000000000007",
                "issuer_public_key": "00",
                "signature": "00",
                "cryptosuite": "https://sparq.dev/ns/zk#poseidon2-schnorr-v1"
            }],
            "attributions": [],
            "entailment_regime": "simple",
            "binding": { "mode": "challenge", "challenge": "0x2a" },
            "sub_proofs": []
        }"#;
        let parsed = ProofManifest::from_json(legacy).expect("legacy manifest parses");
        assert_eq!(
            parsed.commitment_attestations[0].holder, None,
            "legacy attestation has no holder binding"
        );
        assert!(
            matches!(parsed.binding, BindingMode::Challenge { .. }),
            "legacy bearer/challenge binding remains valid"
        );
    }
}

/// [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): schema tests for the hidden
/// cross-credential JOIN manifest types — `CircuitId::JoinEq`,
/// `ProofInputs::JoinEq`, and `JoinEdge`. SCHEMA ONLY: the `bind_joins` verifier
/// gate (commitment binding + `UnboundJoin` query binding) is step 4 (sq-sfsi).
#[cfg(test)]
mod join_schema_tests {
    use super::*;

    /// The PUBLIC-input field ordering the `join_eq` circuit's `main` declares,
    /// AFTER the prepended `challenge` (which `binding` carries for every member).
    /// This MUST stay byte-for-byte in step with
    /// `zk/compose/join_eq_na16_nb16/src/main.nr` — the verifier's
    /// `reconstruct_public_inputs` (audit-#1) emits the vector in exactly this
    /// declaration order. If the Noir `main` reorders, this constant and the
    /// reconstruction arm must move together.
    const JOIN_EQ_PUBLIC_INPUT_LAYOUT: &[&str] = &[
        "challenge",        // field 0 — every member's first `pub` (verifier nonce)
        "commit_a",         // graph-A commitment C(G_a)
        "commit_b",         // graph-B commitment C(G_b)
        "join_commitment",  // HIDING commitment to the join value (design §2.4)
        "slot_a",           // graph-A join slot in {0,1,2} (query-bound)
        "slot_b",           // graph-B join slot in {0,1,2} (query-bound)
    ];

    fn join_inputs() -> ProofInputs {
        ProofInputs::JoinEq {
            id: CircuitId::JoinEq { n_a: 16, n_b: 16 },
            commit_a: FieldHex("0x0a".to_string()),
            commit_b: FieldHex("0x0b".to_string()),
            join_commitment: FieldHex("0x0c".to_string()),
            slot_a: 0,
            slot_b: 2,
        }
    }

    /// `CircuitId::JoinEq` enumerates and names its compiled member exactly like
    /// the other members (`scan_k…`, `filter_int_d…`): the `(n_a, n_b)` buckets
    /// drive the package directory `join_eq_na{n_a}_nb{n_b}`.
    #[test]
    fn circuit_id_join_eq_packages_like_other_members() {
        assert_eq!(
            CircuitId::JoinEq { n_a: 16, n_b: 16 }.package(),
            "join_eq_na16_nb16",
            "matches the landed Noir member directory (PR #170)"
        );
        // Asymmetric buckets name distinctly (forward-looking; v1 compiles 16x16).
        assert_eq!(
            CircuitId::JoinEq { n_a: 16, n_b: 64 }.package(),
            "join_eq_na16_nb64"
        );
        // The id round-trips through serde with its `kind` tag like every variant.
        let id = CircuitId::JoinEq { n_a: 16, n_b: 16 };
        let json = serde_json::to_string(&id).expect("serializes");
        assert!(json.contains("join_eq"), "snake_case `kind` tag is `join_eq`");
        let back: CircuitId = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, id, "CircuitId::JoinEq round-trips");
    }

    /// `ProofInputs::JoinEq` round-trips through serde and exposes its id via the
    /// `circuit_id()` accessor (the new exhaustive-match arm).
    #[test]
    fn proof_inputs_join_eq_round_trips_and_exposes_id() {
        let inputs = join_inputs();
        assert_eq!(
            inputs.circuit_id(),
            &CircuitId::JoinEq { n_a: 16, n_b: 16 },
            "circuit_id() returns the JoinEq id"
        );
        let json = serde_json::to_string(&inputs).expect("serializes");
        // The serde `circuit` tag (mirrors scan/filter) is `join_eq`.
        assert!(json.contains("\"circuit\":\"join_eq\""), "tagged `join_eq`");
        // The HIDDEN join value never appears as a field — only the public inputs.
        assert!(
            !json.contains("\"value\"") && !json.contains("\"blinding\""),
            "join value + blinder are PRIVATE — never serialized in ProofInputs"
        );
        let back: ProofInputs = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, inputs, "ProofInputs::JoinEq round-trips");
    }

    /// `JoinEdge` round-trips through serde, and an empty `join_edges` default
    /// keeps legacy (pre-join) manifests parseable (additive schema).
    #[test]
    fn join_edge_round_trips_and_is_additive() {
        let edge = JoinEdge {
            scan_a: 0,
            graph_a: 0,
            scan_b: 1,
            graph_b: 0,
            join_proof: 2,
        };
        let json = serde_json::to_string(&edge).expect("serializes");
        let back: JoinEdge = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, edge, "JoinEdge round-trips");

        // A manifest JSON with NO `join_edges` key parses (serde default), so the
        // schema addition does not break existing manifests.
        let no_joins = r#"{
            "query": "SELECT * WHERE { ?s ?p ?o }",
            "attributions": [[0]],
            "entailment_regime": "simple",
            "binding": { "mode": "challenge", "challenge": "0x1" },
            "sub_proofs": []
        }"#;
        let m: ProofManifest = serde_json::from_str(no_joins).expect("legacy parses");
        assert!(m.join_edges.is_empty(), "join_edges defaults to empty");

        // And a manifest carrying join_edges round-trips through the full type.
        let mut m2 = m.clone();
        m2.join_edges = vec![edge.clone()];
        let json = serde_json::to_string(&m2).expect("serializes");
        let back: ProofManifest = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.join_edges, vec![edge], "manifest join_edges round-trip");
    }

    /// The `ProofInputs::JoinEq` field set is EXACTLY the public inputs the Noir
    /// member exposes (minus the `binding`-carried `challenge`), in the same
    /// order. Pins the schema↔circuit contract: if a field is added/removed/
    /// reordered on either side without updating the other, this fails.
    #[test]
    fn join_eq_field_ordering_matches_circuit_layout() {
        // The struct fields, in declaration order, that are PUBLIC inputs.
        let struct_pub_fields = ["commit_a", "commit_b", "join_commitment", "slot_a", "slot_b"];
        // The circuit layout minus field 0 (`challenge`, carried by `binding`).
        let circuit_after_challenge = &JOIN_EQ_PUBLIC_INPUT_LAYOUT[1..];
        assert_eq!(
            struct_pub_fields.as_slice(),
            circuit_after_challenge,
            "ProofInputs::JoinEq public fields match join_eq main's order \
             (zk/compose/join_eq_na16_nb16/src/main.nr)"
        );
        assert_eq!(
            JOIN_EQ_PUBLIC_INPUT_LAYOUT[0], "challenge",
            "field 0 is the verifier nonce, as every member"
        );
        assert_eq!(
            JOIN_EQ_PUBLIC_INPUT_LAYOUT.len(),
            6,
            "join_eq exposes exactly 6 public inputs"
        );
    }
}

/// [OPUS-4.8] sq-y2wy: canonical edge-ordering tests for `binding_edges` +
/// `join_edges`. PR #178 (sq-fi03) documented (then corrected to "deferred")
/// edge canonicalisation; this module pins the now-real implementation —
/// `ProofManifest::canonicalize` / `to_json` sort both edge vectors to a total,
/// deterministic order so manifests differing only in edge order are identical.
#[cfg(test)]
mod canonical_edge_tests {
    use super::*;

    /// A minimal manifest with the two edge vectors set to the given contents.
    /// Everything else is the smallest legal shape (no sub-proofs — the edges'
    /// references are NOT resolved by `canonicalize`/`to_json`, only sorted; the
    /// verification-still-accepts path is exercised by the bind_joins / binding
    /// tests in `verifier.rs`, which already build resolvable sub-proofs).
    fn manifest_with_edges(binding: Vec<BindingEdge>, joins: Vec<JoinEdge>) -> ProofManifest {
        ProofManifest {
            r#type: default_type(),
            query: "SELECT * WHERE { ?s ?p ?o }".to_string(),
            issuers: vec![],
            key_set: vec![],
            commitment_attestations: vec![],
            attributions: vec![],
            pattern_scans: vec![],
            join_obligations: vec![],
            entailment_regime: EntailmentRegime::Simple,
            derivation_steps: vec![],
            binding: BindingMode::Challenge {
                challenge: FieldHex::from_field(&Fr::from(1u64)),
            },
            revocation: None,
            status_snapshots: vec![],
            sub_proofs: vec![],
            binding_edges: binding,
            join_edges: joins,
            hidden_revocation: None,
            hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
            fully_hidden_revocation: None,
        }
    }

    fn binding_edge(from_proof: usize, from_row: usize, from_slot: usize, to_proof: usize) -> BindingEdge {
        BindingEdge { from_proof, from_row, from_slot, to_proof }
    }

    fn join_edge(scan_a: usize, graph_a: usize, scan_b: usize, graph_b: usize, join_proof: usize) -> JoinEdge {
        JoinEdge { scan_a, graph_a, scan_b, graph_b, join_proof }
    }

    /// `canonicalize` sorts `binding_edges` to ascending
    /// `(from_proof, from_row, from_slot, to_proof)` regardless of insertion order,
    /// and is idempotent.
    #[test]
    fn binding_edges_canonicalise_to_total_order() {
        // Edges chosen so EACH tuple component is the tie-breaker for some pair
        // (so the test would fail if any component were dropped from the key).
        let order_x = vec![
            binding_edge(2, 0, 0, 9),
            binding_edge(0, 1, 0, 4),
            binding_edge(0, 0, 2, 5),
            binding_edge(0, 0, 1, 7),
            binding_edge(0, 0, 1, 3),
        ];
        // The SAME set, shuffled.
        let order_y = vec![
            binding_edge(0, 0, 1, 7),
            binding_edge(0, 0, 2, 5),
            binding_edge(2, 0, 0, 9),
            binding_edge(0, 0, 1, 3),
            binding_edge(0, 1, 0, 4),
        ];

        let expected = vec![
            binding_edge(0, 0, 1, 3),
            binding_edge(0, 0, 1, 7),
            binding_edge(0, 0, 2, 5),
            binding_edge(0, 1, 0, 4),
            binding_edge(2, 0, 0, 9),
        ];

        let mut a = manifest_with_edges(order_x, vec![]);
        let mut b = manifest_with_edges(order_y, vec![]);
        a.canonicalize();
        b.canonicalize();
        assert_eq!(a.binding_edges, expected, "binding_edges sort to the tuple order");
        assert_eq!(a.binding_edges, b.binding_edges, "order-independent canonical form");

        // Idempotent: a second canonicalise is a no-op.
        let once = a.binding_edges.clone();
        a.canonicalize();
        assert_eq!(a.binding_edges, once, "canonicalize is idempotent");
    }

    /// `canonicalize` sorts `join_edges` to ascending
    /// `(scan_a, graph_a, scan_b, graph_b, join_proof)` regardless of insertion
    /// order — including using `join_proof` as the final tie-breaker.
    #[test]
    fn join_edges_canonicalise_to_total_order() {
        let order_x = vec![
            join_edge(1, 0, 2, 0, 5),
            join_edge(0, 0, 1, 0, 4),
            join_edge(0, 0, 1, 0, 3), // ties first 4 with the previous; join_proof breaks it
            join_edge(0, 1, 1, 0, 6),
        ];
        let order_y = vec![
            join_edge(0, 1, 1, 0, 6),
            join_edge(0, 0, 1, 0, 3),
            join_edge(1, 0, 2, 0, 5),
            join_edge(0, 0, 1, 0, 4),
        ];

        let expected = vec![
            join_edge(0, 0, 1, 0, 3),
            join_edge(0, 0, 1, 0, 4),
            join_edge(0, 1, 1, 0, 6),
            join_edge(1, 0, 2, 0, 5),
        ];

        let mut a = manifest_with_edges(vec![], order_x);
        let mut b = manifest_with_edges(vec![], order_y);
        a.canonicalize();
        b.canonicalize();
        assert_eq!(a.join_edges, expected, "join_edges sort to the tuple order");
        assert_eq!(a.join_edges, b.join_edges, "order-independent canonical form");
    }

    /// `to_json` canonicalises BOTH edge vectors before serialising, WITHOUT
    /// mutating `self`: a multi-edge manifest built with edges in order X and one
    /// with the same edges in order Y produce byte-identical JSON (and thus the
    /// same hash over those bytes).
    #[test]
    fn to_json_is_byte_identical_regardless_of_edge_insertion_order() {
        let binding_x = vec![
            binding_edge(1, 0, 0, 5),
            binding_edge(0, 0, 0, 3),
        ];
        let joins_x = vec![
            join_edge(1, 0, 2, 0, 6),
            join_edge(0, 0, 1, 0, 4),
        ];
        // Reverse BOTH vectors.
        let binding_y: Vec<_> = binding_x.iter().rev().cloned().collect();
        let joins_y: Vec<_> = joins_x.iter().rev().cloned().collect();

        let m_x = manifest_with_edges(binding_x, joins_x);
        let m_y = manifest_with_edges(binding_y, joins_y);

        let json_x = m_x.to_json();
        let json_y = m_y.to_json();
        assert_eq!(json_x, json_y, "edge-order-only difference => identical canonical JSON");

        // to_json does NOT mutate self (canonicalises a clone): the in-memory
        // manifest keeps its original (unsorted) edge order.
        assert_eq!(
            m_x.binding_edges,
            vec![binding_edge(1, 0, 0, 5), binding_edge(0, 0, 0, 3)],
            "to_json leaves self's binding_edges untouched"
        );

        // The canonical JSON round-trips back to a manifest whose edges ARE sorted.
        let back = ProofManifest::from_json(&json_x).expect("canonical JSON parses");
        assert_eq!(
            back.binding_edges,
            vec![binding_edge(0, 0, 0, 3), binding_edge(1, 0, 0, 5)],
            "parsed canonical manifest carries sorted binding_edges"
        );
        assert_eq!(
            back.join_edges,
            vec![join_edge(0, 0, 1, 0, 4), join_edge(1, 0, 2, 0, 6)],
            "parsed canonical manifest carries sorted join_edges"
        );
    }
}

// [OPUS-4.8] sq-3kd2g.6: schema tests for the bounded-depth path member id,
// the PathReach public inputs, and the extended-fragment FragmentManifest wrapper.
#[cfg(all(test, feature = "extended-fragment"))]
mod path_schema_tests {
    use super::*;

    fn fh(s: &str) -> FieldHex {
        FieldHex(s.to_string())
    }

    fn path_inputs() -> ProofInputs {
        ProofInputs::PathReach {
            id: CircuitId::PathReach { d: 4, k: 2, n: 16 },
            commitments: vec![fh("0x1"), fh("0x2")],
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero: true,
            depth_bound: 4,
            attribution: vec![true, false],
        }
    }

    #[test]
    fn path_reach_circuit_id_packages_like_the_other_members() {
        assert_eq!(
            CircuitId::PathReach { d: 4, k: 2, n: 16 }.package(),
            "path_reach_d4_k2_n16"
        );
        assert_eq!(
            CircuitId::PathReach { d: 8, k: 1, n: 16 }.package(),
            "path_reach_d8_k1_n16"
        );
    }

    #[test]
    fn path_reach_inputs_round_trip_and_expose_the_id() {
        let inputs = path_inputs();
        assert_eq!(inputs.circuit_id(), &CircuitId::PathReach { d: 4, k: 2, n: 16 });
        let json = serde_json::to_string(&inputs).expect("serializes");
        let back: ProofInputs = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, inputs, "ProofInputs::PathReach round-trips");
        assert!(json.contains("\"circuit\":\"path_reach\""));
    }

    #[test]
    fn branch_witness_round_trips() {
        let bw = BranchWitness {
            branch: 1,
            scan_proofs: vec![0, 2],
            path_proofs: vec![1],
            values_rows: vec![3],
            scan_rows: vec![0, 1],
            solution: vec![SolutionBinding {
                var: "o".to_string(),
                term: DisclosedTerm::Iri { value: "http://ex/b".to_string() },
            }],
        };
        let json = serde_json::to_string(&bw).expect("serializes");
        let back: BranchWitness = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, bw);
    }

    #[test]
    fn branch_witness_scan_rows_is_serde_default_back_compatible() {
        // [OPUS-4.8] sq-qyfth: a pre-scan_rows manifest (no `scan_rows` key) still
        // deserializes, defaulting the row-selection to empty — additive back-compat.
        let json = r#"{"branch":0,"scan_proofs":[0],"path_proofs":[],"values_rows":[]}"#;
        let back: BranchWitness = serde_json::from_str(json).expect("deserializes");
        assert!(back.scan_rows.is_empty());
        assert_eq!(back.scan_proofs, vec![0]);
    }

    #[test]
    fn disclosed_term_to_term_builds_iris_and_literal_shapes() {
        // IRI.
        assert_eq!(
            DisclosedTerm::Iri { value: "http://ex/a".into() }.to_term(),
            Some(oxrdf::Term::NamedNode(oxrdf::NamedNode::new("http://ex/a").unwrap()))
        );
        // Plain literal.
        assert_eq!(
            DisclosedTerm::Literal { value: "x".into(), datatype: None, language: None }.to_term(),
            Some(oxrdf::Term::Literal(oxrdf::Literal::new_simple_literal("x")))
        );
        // Typed literal.
        assert_eq!(
            DisclosedTerm::Literal {
                value: "1".into(),
                datatype: Some("http://www.w3.org/2001/XMLSchema#integer".into()),
                language: None,
            }
            .to_term(),
            Some(oxrdf::Term::Literal(oxrdf::Literal::new_typed_literal(
                "1",
                oxrdf::NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
            )))
        );
        // Language-tagged literal.
        assert_eq!(
            DisclosedTerm::Literal { value: "x".into(), datatype: None, language: Some("en".into()) }
                .to_term(),
            Some(oxrdf::Term::Literal(
                oxrdf::Literal::new_language_tagged_literal("x", "en").unwrap()
            ))
        );
        // Fail-closed: an unparseable IRI, a bad datatype, a bad language tag, and
        // a language + non-langString datatype all return None.
        assert_eq!(DisclosedTerm::Iri { value: "not an iri".into() }.to_term(), None);
        assert_eq!(
            DisclosedTerm::Literal {
                value: "x".into(),
                datatype: Some("not an iri".into()),
                language: None,
            }
            .to_term(),
            None
        );
        assert_eq!(
            DisclosedTerm::Literal {
                value: "x".into(),
                datatype: Some("http://www.w3.org/2001/XMLSchema#string".into()),
                language: Some("en".into()),
            }
            .to_term(),
            None
        );
    }

    #[test]
    fn fragment_manifest_wraps_and_round_trips() {
        let manifest = ProofManifest {
            r#type: default_type(),
            query: "SELECT * WHERE { <http://ex/a> <http://ex/p>+ ?o }".to_string(),
            issuers: vec![],
            key_set: vec![],
            commitment_attestations: vec![],
            attributions: vec![],
            pattern_scans: vec![],
            join_obligations: vec![],
            entailment_regime: EntailmentRegime::Simple,
            derivation_steps: vec![],
            binding: BindingMode::Challenge { challenge: fh("0x2a") },
            revocation: None,
            status_snapshots: vec![],
            sub_proofs: vec![SubProof { inputs: path_inputs(), proof_hex: String::new() }],
            binding_edges: vec![],
            join_edges: vec![],
            hidden_revocation: None,
            hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
            fully_hidden_revocation: None,
        };
        let fm = FragmentManifest::new(
            manifest.clone(),
            vec![BranchWitness {
                branch: 0,
                scan_proofs: vec![],
                path_proofs: vec![0],
                values_rows: vec![],
                scan_rows: vec![],
                solution: vec![],
            }],
        );
        assert_eq!(fm.r#type, "urn:sparq:zk:FragmentManifest");
        let json = fm.to_json();
        let back = FragmentManifest::from_json(&json).expect("fragment manifest round-trips");
        assert_eq!(back, fm);
        assert_eq!(back.manifest, manifest);
        assert_eq!(back.branch_witnesses.len(), 1);
    }

    #[test]
    fn fragment_manifest_parses_without_a_type_marker() {
        let inner = r#"{
            "type": "urn:sparq:zk:ProofManifest",
            "query": "ASK {}",
            "attributions": [],
            "entailment_regime": "simple",
            "binding": { "mode": "challenge", "challenge": "0x2a" },
            "sub_proofs": []
        }"#;
        let json = format!("{{ \"manifest\": {}, \"branch_witnesses\": [] }}", inner);
        let fm = FragmentManifest::from_json(&json).expect("parses without a type marker");
        assert_eq!(fm.r#type, "urn:sparq:zk:FragmentManifest");
        assert!(fm.branch_witnesses.is_empty());
    }
}
