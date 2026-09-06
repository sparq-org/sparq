// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! Manifest verifier (plan §S4.E module iii, layer 3).
//!
//! Verification stages, all of which must pass:
//! 1. **Manifest re-checks** (cheap, no proving): re-parse the query and run
//!    `sparq_zk::verify::recheck` — the bnode cross-graph join guard (Q6) plus
//!    attribution arity. Re-derive each sub-proof's circuit id from its public
//!    inputs and confirm it equals the declared id (a proof cannot be replayed
//!    against a different family member). For each scan sub-proof, also require
//!    the per-graph `attribution` to be exactly `CircuitId.k` bits AND the
//!    per-graph `commitments` to be STRICTLY INCREASING on the field
//!    representative ([OPUS-4.8] sq-vxq8 / plan S2.5): the latter is the host-side
//!    mirror of `scan_check` step 1b, rejecting a duplicate/out-of-order
//!    commitment (the duplicate-inclusion / COUNT-forgery class) before any bb
//!    proof. The in-circuit `<` is the authoritative gate; this structural check
//!    is defence in depth so a witness-only manifest cannot smuggle a duplicate
//!    past the fast stage.
//! 2. **Binding-consistency edges**: each declared edge's scan-proof row/slot
//!    encoding must equal the consuming filter proof's `operand_enc` (a plain
//!    field equality over public inputs — the modular composition seam).
//!
//! 2d. **Issuer-signature / key-set binding (audit #3, codex #1).** Every scan
//!    sub-proof's `commitments[g]` must carry an issuer attestation
//!    ([`crate::manifest::CommitmentAttestation`]) whose Schnorr signature
//!    verifies under its declared issuer key and whose key is a member of the
//!    EXTERNAL trusted key-set `K` — the relying party's [`KeySet`] argument,
//!    NOT the prover-supplied `manifest.key_set` (trusting the latter as the
//!    anchor was the codex #1 soundness hole: a prover signs a forgery with its
//!    own key and self-lists it). `manifest.key_set` is only accepted as a
//!    SUBSET of the external `K`. `commitments[g]` is byte-bound into the bb
//!    public inputs by stage 3a, so this ties the attested commitment to the
//!    proved statement: an unsigned/prover-invented commitment, a
//!    drop-a-triple-and-recommit suppression (the truncated `C(G')` has no valid
//!    attestation), a key-not-in-external-`K` signature, and a prover key-set
//!    that widens `K` all REJECT. Interim privacy note: this reveals WHICH
//!    issuer signed; the in-circuit undisclosed-key upgrade (see
//!    `sparq_zk::sig`) removes that.
//! 3. **bb verification (the cryptographic gate)**, for each sub-proof, all of:
//!    a. **Public-input reconstruction (audit #1).** Independently rebuild the
//!   public-input field vector from the DECLARED [`ProofInputs`] (in `main`
//!   declaration order) using the verifier's own challenge, serialize to
//!   bb's byte layout (32-byte BE field elements, no header — see
//!   `reconstruct_public_inputs`), and assert byte-equality with the
//!   prover's `public_inputs` blob. This binds the JSON statement to the
//!   proof; without it stages 1-2 (JSON) and the proof (a detached crypto
//!   object) describe potentially different statements.
//!    b. **Canonical vk (audit #2).** Recompute the vk verifier-side from the
//!   compiled member named by the re-derived [`CircuitId`] (never the
//!   prover's `art.vk`), pinning the vk to the FULL CircuitId (subsumes the
//!   #11 n/d/r relabel).
//!    c. `bb verify` over (prover proof, reconstructed public inputs, canonical
//!   vk).
//!
//! 2f. **Revocation / freshness (audit #12, + re-audit Option B).** Leverages #3:
//!    the issuer signature ALSO binds the credential's STATUS-LIST REFERENCE
//!    (`status_ref_digest(H(list IRI), index, version)`), so a scan-covering
//!    attestation MUST carry an issuer-bound status reference (mandatory /
//!    fail-closed — a `status: None` attestation is rejected, and an omitted
//!    `manifest.revocation` leaves the status-bound signature uncheckable and is
//!    rejected). The verifier then recomputes the digest from the disclosed
//!    `manifest.revocation`, requires it to match the issuer-signed
//!    `AttestedStatusRef`, and — THE RE-AUDIT FIX — reads the credential's status
//!    BIT from the relying party's OWN AUTHORITATIVE [`crate::manifest::StatusListSnapshot`]
//!    for `(list, version)`, carried EXTERNALLY in [`RevocationPolicy`] (exactly
//!    as the trusted key-set `K` is external), NOT from the prover's
//!    `manifest.status_snapshots`. The issuer signature binds the REFERENCE but
//!    NOT the bit VALUES, so trusting the prover's bitstring let a genuine
//!    reference + a forged all-zero snapshot reverify a REVOKED credential; the
//!    bit is now sourced off prover-controlled bytes. It asserts (i) the
//!    reference version is within the relying party's freshness window, (ii) the
//!    AUTHORITATIVE snapshot's bit at `index` is UNSET, and (iii) any prover
//!    snapshot for the same `(list, version)` byte-equals the authoritative one
//!    (a tamper tripwire). A REVOKED bit, a STALE reference, a missing
//!    AUTHORITATIVE snapshot, a disagreeing prover snapshot, or an
//!    omitted/forged/mismatched reference all REJECT. [OPUS-4.8] sq-ayv: a
//!    credential may instead use the COMMITTED-index path — the issuer signs
//!    `status_ref_commit_digest(H(list), index_commitment, version)` (a hiding
//!    commitment to the index), the clear index is WITHHELD, and liveness is
//!    checked by the hidden-index proof cross-bound to that commitment (so neither
//!    the index nor the bit is disclosed). The clear-index path above is unchanged
//!    for clear references. See `bind_revocation` / `bind_hidden_revocation`.
//!
//! 4. **Freshness / single-use (audit #4).** [`verify_manifest`] takes a
//!    VERIFIER-ISSUED fresh [`VerifierNonce`] (minted out of band, handed to the
//!    prover before proving) and a single-use [`SeenNonces`] store. It (i)
//!    records the nonce single-use BEFORE the crypto gate (replay of the same
//!    nonce => [`CheckError::NonceReplay`]); (ii) requires `manifest.binding`'s
//!    declared challenge to equal the nonce (fail-closed); and (iii) feeds the
//!    nonce — NOT `manifest.binding` — as field 0 of the stage-3a reconstruction,
//!    so a proof committed under any OTHER challenge fails the byte-compare. A
//!    captured manifest re-presented under a fresh nonce is rejected by the
//!    byte-compare; the same manifest re-presented under its original nonce is
//!    rejected by the store. (Field 0 stays an unconstrained in-circuit tag — the
//!    binding is wholly verifier-side via the audit-#1 byte-compare, so no circuit
//!    change is needed.)
//!
//! Stage 1+2 run WITHOUT bb (the fast structural gate); stage 3 is the
//! cryptographic gate; stage 4 (freshness) is enforced by `verify_manifest`
//! around stage 3. [`prefilter_manifest_structure`] runs stages 1+2 ONLY and
//! is NOT a sound verifier (it binds nothing to a proof and enforces NO
//! freshness — see its docs); [`verify_manifest`] is the sound public entry
//! point: it runs the pre-filter then the freshness + bb verify +
//! public-input reconstruction that binds the JSON statement (incl. attribution
//! bits) and the verifier's nonce to the proofs.

use crate::build::{
    derive_filter_decimal_id, derive_filter_f64_id, derive_filter_int_id,
    derive_filter_signed_int_id, derive_join_eq_id, derive_scan_id,
};
use crate::driver::{CircuitProver, DriverError};
use crate::manifest::{
    BindingMode, CircuitId, EntailmentRegime, FieldHex, ProofInputs, ProofManifest,
    StatusListSnapshot,
};
// [OPUS-4.8] sq-314: derivation-step re-check for entailment-regime enforcement.
// [OPUS-5] sq-rsd3v.7: the two UNBUILT completeness halves the refusal message names.
use crate::derivation::{regime_admits, COMPLETENESS_UNDER_ENTAILMENT_UNBUILT};
// [OPUS-4.8] sq-3e5 + sq-h2v: hidden-index revocation root derivation.
use crate::revocation::merkle_root;
use sparq_zk::encode::encode_term;
use sparq_zk::field::{field_from_hex_str, field_to_be_bytes_32, field_to_hex, Fr};
// [OPUS-4.8] codex 2221 HIGH: only the SALT-BOUND `commitment_message_with_salt`
// is used on the scan-verify path; the bare salt-less `commitment_message` is no
// longer reachable here (a scan-covering attestation must be salt-bound).
// [OPUS-4.8] audit #12: the STATUS-BOUND `commitment_message_with_status` is the
// scan-verify path's signed message — a scan-covering attestation must bind the
// status reference (status_ref_digest over the disclosed list/index/version).
// [OPUS-4.8] sq-z8s7 (HolderPoP T3 / B1): the HOLDER-BOUND `commitment_message_with_holder`
// (ZKSIG_C4) is the signed message for a holder-bound attestation, and `holder_key_digest`
// recomputes the presented holder key's digest for the clear-key cross-check.
use sparq_zk::sig::{
    commitment_message_with_holder, commitment_message_with_status, holder_key_digest,
    holder_pop_message, public_key_from_hex, signature_from_hex, status_list_id_to_field,
    status_ref_commit_digest, status_ref_digest, status_ref_fully_committed_digest,
    verify as sig_verify, PublicKey, SignatureScheme,
};
use sparq_zk::verify::{
    fragment_filters, fragment_pattern_consts, fragment_patterns, recheck, variable_slots,
    FilterCmp, JoinEdge, QueryFilter, VerifyError,
};
// [OPUS-4.8] sq-3kd2g.6: the wave-1 extended-fragment query re-derivation the
// fail-closed `dispatch_fragment` routing gate consumes (never trusting the
// manifest). Opt-in (`extended-fragment`). [OPUS-4.8] sq-1zf94: `SlotPattern` is
// the shape of a re-derived path endpoint the disclosed-solution binding matches on.
// [OPUS-4.8] sq-ygk6x: `branch_obligations` re-derives one UNION branch's Q6
// cross-graph non-bnode obligations (join edges + multi-graph path links) from the
// query text + the manifest's proof-bound per-obligation attributions — the
// per-branch analogue of the flat `cross_graph_join_obligations` the compose
// verifier's `bind_fragment_join_coherence` gate enforces.
#[cfg(feature = "extended-fragment")]
use sparq_zk::verify::{branch_obligations, fragment_query, SlotPattern};
use std::collections::BTreeSet;
use std::path::Path;

/// The relying party's EXTERNALLY-anchored trusted issuer key-set `K` — the
/// soundness fix for audit #3 codex finding #1.
///
/// # Why this is a verifier input, not a manifest field
/// The manifest carries a `key_set` field, but it is PROVER-SUPPLIED: a
/// malicious prover signs a forged commitment with its OWN key and lists that
/// key in `manifest.key_set`, so a verifier that trusts `manifest.key_set` as
/// the anchor gives NO "authoritative source" guarantee — `#3` would be
/// vacuous. The trust anchor MUST come from outside the proof. A relying party
/// constructs a [`KeySet`] from the issuer keys IT decides to trust (its policy
/// / an issuer-key registry it resolves out of band) and passes it into
/// [`verify_manifest`] (the sound entry point) / [`prefilter_manifest_structure`]
/// (the structural pre-filter). The accept decision then
/// depends only on this external set; `manifest.key_set`, if present at all, is
/// only accepted when it is a SUBSET of this external set (checked, never
/// trusted as the anchor) — see `bind_issuer_attestations`.
///
/// Keys are stored in normalized hex form (no `0x`, lowercase) and validated as
/// parseable, non-identity Baby-JubJub points at construction (an unparseable or
/// identity key can never be a real issuer key, so it is dropped fail-closed).
// [OPUS-4.8] audit #3 codex #1: external trust anchor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeySet {
    keys: BTreeSet<String>,
    /// OPTIONAL Merkle-tree depth for the HIDDEN-ISSUER attestation proof
    /// (sq-z9l). When set, the verifier derives the authoritative key-set Merkle
    /// root from THIS set (canonical order) at this depth and accepts a
    /// `manifest.hidden_issuer_attestations` proof whose PUBLIC root byte-equals
    /// it — proving "signed by SOME key in K" without disclosing which. `None` =>
    /// the hidden-issuer path is disabled (the verifier only runs the clear-key
    /// `bind_issuer_attestations` check). MUST equal the `hidden_issuer_d{depth}`
    /// member the prover used.
    // [OPUS-4.8] sq-z9l: opt-in hidden-issuer verification depth.
    hidden_issuer_depth: Option<u32>,
}

impl KeySet {
    /// An empty trust anchor: trusts NO issuer. Any scan carrying commitments is
    /// then rejected (fail closed). Useful as the explicit "no source is
    /// authoritative" policy and as a test default.
    pub fn empty() -> Self {
        KeySet { keys: BTreeSet::new(), hidden_issuer_depth: None }
    }

    /// Build a trust anchor from the relying party's trusted issuer public keys
    /// (hex). Each key is validated as a parseable, non-identity Baby-JubJub
    /// point (`public_key_from_hex` already rejects the identity — codex #3) and
    /// stored in normalized hex; unparseable/identity entries are dropped (they
    /// can never match a real attestation key, so dropping them is fail-closed,
    /// not a silent widening of trust).
    pub fn from_hex_keys<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let keys = keys
            .into_iter()
            .filter_map(|h| public_key_from_hex(h.as_ref()).map(|_| normalize_hex(h.as_ref())))
            .collect();
        KeySet { keys, hidden_issuer_depth: None }
    }

    /// Enable the HIDDEN-ISSUER attestation path (sq-z9l) at Merkle depth `depth`
    /// (builder style). The verifier will then derive the authoritative key-set
    /// Merkle root from THIS set (canonical order) at `depth` and accept a
    /// `manifest.hidden_issuer_attestations` proof whose PUBLIC root matches —
    /// proving "signed by SOME key in K" without disclosing which. `depth` MUST
    /// equal the `hidden_issuer_d{depth}` member the prover used. The clear-key
    /// `bind_issuer_attestations` path is unaffected.
    // [OPUS-4.8] sq-z9l: opt-in hidden-issuer verification depth.
    pub fn with_hidden_issuer_depth(mut self, depth: u32) -> Self {
        self.hidden_issuer_depth = Some(depth);
        self
    }

    /// The hidden-issuer Merkle depth, if the relying party enabled the path.
    /// `None` => disabled (a `manifest.hidden_issuer_attestations` is not accepted).
    // [OPUS-4.8] sq-z9l.
    fn hidden_issuer_depth(&self) -> Option<u32> {
        self.hidden_issuer_depth
    }

    /// Whether `pk_hex` (any case, optional `0x`) is a member of the trusted set.
    fn contains_hex(&self, pk_hex: &str) -> bool {
        self.keys.contains(&normalize_hex(pk_hex))
    }

    /// The trusted set is empty (trusts no issuer).
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The trusted keys in the canonical CANONICAL leaf order for the hidden-issuer
    /// key-set Merkle tree (sq-z9l): the normalized-hex set's sorted order
    /// (`BTreeSet` iteration). Both the relying party (deriving the authoritative
    /// `key_set_root`) and the prover (building its membership path) commit K in
    /// THIS order, so the roots agree. Parsed back to [`PublicKey`]s (every stored
    /// key is already validated parseable at construction, so this never drops a
    /// member).
    // [OPUS-4.8] sq-z9l: canonical leaf order for the key-set commitment.
    fn ordered_keys(&self) -> Vec<sparq_zk::sig::PublicKey> {
        self.keys
            .iter()
            .filter_map(|h| public_key_from_hex(h))
            .collect()
    }

    /// The authoritative hidden-issuer key-set Merkle root (sq-z9l) over the
    /// trusted keys in canonical order, at depth `depth`. This is the TRUST ANCHOR
    /// the relying party derives from its OWN [`KeySet`] (exactly as
    /// `RevocationPolicy` derives the status root from its own snapshot), and which
    /// a `manifest.hidden_issuer_attestations` proof's PUBLIC `key_set_root` must
    /// byte-equal. `None` if the set overflows the tree or `depth` is implausible.
    ///
    /// [OPUS-4.8] sq-r6dq: uses the SPARSE builder
    /// ([`crate::issuer::key_set_root_sparse`], sq-8k3h) rather than the dense
    /// `key_set_root`, so this derivation is `O(n·depth)` in the number of trusted
    /// keys instead of `O(2^depth)`. The `depth` is a free policy parameter
    /// ([`KeySet::with_hidden_issuer_depth`]), so a relying party with a large or
    /// growing issuer registry can pick a deep tree without the verifier
    /// materialising all `2^depth` leaves. Mirrors `revocation::merkle_root`, whose
    /// sparse builder is likewise the default (sq-hwe). The matching deeper COMPILED
    /// circuit member (`hidden_issuer_d{depth}` beyond `d4`) is a follow-up requiring
    /// the nargo/bb toolchain lane to gate-baseline.
    ///
    /// # What the substitution guarantees (NOT "scaling-only")
    /// The sparse root is **value-equivalent to the dense root wherever the dense
    /// evaluation completes** — the trust anchor a prover's public root must
    /// byte-equal is unchanged at every depth the dense builder can still be run.
    /// It is NOT behaviour-preserving: dropping the dense builder's implicit
    /// `O(2^depth)` host cost means a deep policy depth that previously aborted or
    /// exhausted memory before yielding an anchor now yields one, so the verifier
    /// reaches **later states** (key parsing, leaf hashing, the 96-byte public-input
    /// byte-compare, the `canonical_vk` lookup). Those states are themselves
    /// fail-closed — an uncompiled `hidden_issuer_d{depth}` makes
    /// [`crate::driver::CircuitProver::canonical_vk`] error, with no fallback to
    /// `d4` and no prover-selected vk — but the reachable state space genuinely
    /// grew, and with it the work an unauthenticated submission can force before the
    /// verifier discovers the member is unavailable. That residual DoS surface is
    /// the RP's own `with_hidden_issuer_depth` choice and is tracked separately.
    ///
    /// # How far the equivalence is EVIDENCED (two different kinds of evidence)
    /// - **Dense cross-check against an independent oracle — depths ≤ 12 only:**
    ///   `issuer::tests::sparse_root_matches_dense_for_all_sizes` (all sizes,
    ///   depths 0–6), `sparse_witness_matches_dense_for_all_indices` (depths 0–5),
    ///   and part (a) of `hidden_issuer_root_uses_sparse_builder_and_scales`
    ///   (depths 4/8/12) compare against the separately-implemented dense builder.
    /// - **Self-consistency ONLY — deep trees (24, 28, 31):** the deep groups check
    ///   that a sparse witness re-folds to the sparse root. Root and witness both
    ///   come from `issuer::sparse_fold_leaves`, so a CORRELATED common-mode error
    ///   in that shared fold would pass both. There is no independent oracle at
    ///   those depths (the dense builder cannot materialise `2^24`+ leaves). Deep
    ///   equivalence therefore rests on the induction argument plus the ≤ 12
    ///   cross-checks, not on a deep-tree oracle.
    ///
    /// The one genuinely independent end-to-end oracle is `tests/e2e.rs`'s
    /// `prove_hidden_issuer`, which builds the prover's root/path with the DENSE
    /// builders and attaches this SPARSE anchor as the byte-compared public input,
    /// so `hidden_issuer_in_set_verifies_and_key_is_private` reddens on a
    /// single-bit divergence through a real `bb` proof. Do not "simplify" that test
    /// to one builder — the cross-builder asymmetry is what makes it an oracle.
    ///
    /// See `research/zk-membership-pok-reaudit.md` §2. Nothing here is an external
    /// soundness sign-off; the accredited-cryptographer audit `sq-qhy4` is PENDING.
    // [OPUS-4.8] sq-z9l; sparse builder sq-r6dq.
    pub fn hidden_issuer_root(&self, depth: u32) -> Option<Fr> {
        crate::issuer::key_set_root_sparse(&self.ordered_keys(), depth)
    }

    /// The 0-based index of `pk_hex` in the canonical leaf order, if it is a
    /// member — the slot the prover proves membership at. (Prover-side convenience;
    /// the verifier never needs the index, which stays private.)
    // [OPUS-4.8] sq-z9l.
    pub fn member_index(&self, pk_hex: &str) -> Option<usize> {
        let target = normalize_hex(pk_hex);
        self.keys.iter().position(|k| *k == target)
    }
}

/// The relying party's EXTERNALLY-anchored set of trusted HOLDER keys (sq-cwq) —
/// the trust anchor for the `HolderPop` binding's proof-of-possession.
///
/// # Why this is a verifier input, not a manifest field
/// Exactly the audit-#3 external-`K` precedent: the holder key the PoP is checked
/// against MUST come from outside the proof, or any party could mint a key, sign
/// the challenge with it, list it in the manifest, and pass — a PoP anchored on a
/// prover-chosen key proves nothing. A relying party constructs a `HolderRegistry`
/// from the holder keys IT authorises (its policy / a holder-key registry it
/// resolves out of band) and passes it into [`verify_manifest`]. The accept
/// decision for a `HolderPop` binding then depends only on this external set.
///
/// # Fail-closed
/// An EMPTY registry trusts no holder: a `HolderPop` binding presented against an
/// empty registry is REJECTED ([`CheckError::HolderRegistryEmpty`]). There is NO
/// path on which a `HolderPop` binding is accepted as a bare challenge (the
/// previous placeholder behaviour, which silently waived the holder check). A
/// relying party that does not use holder binding simply uses
/// [`BindingMode::Challenge`]; one that DOES must supply a non-empty registry.
///
/// # Scope (honest deferral)
/// Membership here means "this holder key is authorised to present" — it does NOT
/// bind the key to a SPECIFIC credential. Issuer-attested credential↔holder
/// binding (the issuer signing the holder key into the credential) is deferred;
/// see `bind_holder_pop`. Keys are stored normalized (no `0x`, lowercase) and
/// validated as parseable, non-identity Baby-JubJub points at construction
/// (unparseable/identity entries dropped fail-closed).
// [OPUS-4.8] sq-cwq: external holder trust anchor (mirrors KeySet).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HolderRegistry {
    holders: BTreeSet<String>,
    /// [OPUS-4.8] sq-3c00: when `Some(depth)`, the relying party has OPTED IN to the
    /// hidden-holder-SET tier — a `manifest.holder_set_proofs` entry is accepted and
    /// bound to the depth-`depth` holder-set Merkle root this registry derives. MUST
    /// equal the `holder_set_d{depth}` member the prover used. `None` => disabled (a
    /// `holder_set_proofs` entry is rejected `HolderSetNotEnabled`, fail-closed).
    hidden_holder_set_depth: Option<u32>,
}

impl HolderRegistry {
    /// An empty registry: trusts NO holder. A `HolderPop` binding is then rejected
    /// (`HolderRegistryEmpty`) — the explicit "holder binding not in use" anchor
    /// and the test default. (A `Challenge` binding is unaffected.)
    pub fn empty() -> Self {
        HolderRegistry { holders: BTreeSet::new(), hidden_holder_set_depth: None }
    }

    /// Build a registry from the relying party's authorised holder public keys
    /// (hex). Each is validated as a parseable, non-identity Baby-JubJub point and
    /// stored normalized; unparseable/identity entries are dropped (fail-closed —
    /// they can never match a real PoP key).
    pub fn from_hex_keys<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let holders = keys
            .into_iter()
            .filter_map(|h| public_key_from_hex(h.as_ref()).map(|_| normalize_hex(h.as_ref())))
            .collect();
        HolderRegistry { holders, hidden_holder_set_depth: None }
    }

    /// [OPUS-4.8] sq-3c00: OPT IN to the hidden-holder-SET anonymity tier at Merkle
    /// `depth` — a `manifest.holder_set_proofs` proof whose PUBLIC root matches the
    /// depth-`depth` root this registry derives is then accepted (the holder proves
    /// membership in this set WITHOUT disclosing which holder). MUST equal the
    /// `holder_set_d{depth}` member the prover used. The clear-key / clear-digest
    /// holder paths are unaffected (additive). Builder-style.
    ///
    /// NOT-yet-sound (sq-qhy4); opt-in. Enabling this only changes a decision when a
    /// `holder_set_proofs` entry is presented.
    pub fn with_hidden_holder_set_depth(mut self, depth: u32) -> Self {
        self.hidden_holder_set_depth = Some(depth);
        self
    }

    /// The opt-in hidden-holder-set depth, if enabled (sq-3c00). `None` => disabled
    /// (a `manifest.holder_set_proofs` entry is not accepted).
    fn hidden_holder_set_depth(&self) -> Option<u32> {
        self.hidden_holder_set_depth
    }

    /// The trusted holders in the canonical leaf order for the hidden-holder-set
    /// Merkle tree (sq-3c00): the normalized-hex set's sorted order (`BTreeSet`
    /// iteration), exactly mirroring [`KeySet::ordered_keys`]. Both the relying
    /// party (deriving the authoritative `holder_set_root`) and the prover (building
    /// its membership path) commit the set in THIS order, so the roots agree.
    // [OPUS-4.8] sq-3c00: canonical leaf order for the holder-set commitment.
    fn ordered_holders(&self) -> Vec<sparq_zk::sig::PublicKey> {
        self.holders
            .iter()
            .filter_map(|h| public_key_from_hex(h))
            .collect()
    }

    /// The authoritative hidden-holder-set Merkle root (sq-3c00) over the trusted
    /// holders in canonical order, at depth `depth`. This is the TRUST ANCHOR the
    /// relying party derives from its OWN registry (exactly as
    /// [`KeySet::hidden_issuer_root`] derives the key-set root), and which a
    /// `manifest.holder_set_proofs` proof's PUBLIC `holder_set_root` must byte-equal.
    /// `None` if the set overflows the tree, `depth` is implausible, or any holder
    /// key is the identity (fail-closed). Leaf = `holder_key_digest(hpk)`.
    // [OPUS-4.8] sq-3c00.
    pub fn hidden_holder_set_root(&self, depth: u32) -> Option<Fr> {
        crate::holder::holder_set_root(&self.ordered_holders(), depth)
    }

    /// The 0-based index of `holder_hex` in the canonical leaf order, if it is a
    /// member — the slot the prover proves membership at. (Prover-side convenience;
    /// the verifier never needs the index, which stays private.) Mirrors
    /// [`KeySet::member_index`].
    // [OPUS-4.8] sq-3c00.
    pub fn member_index(&self, holder_hex: &str) -> Option<usize> {
        let target = normalize_hex(holder_hex);
        self.holders.iter().position(|h| *h == target)
    }

    /// The registry trusts no holder.
    pub fn is_empty(&self) -> bool {
        self.holders.is_empty()
    }

    /// Whether `holder_hex` (any case, optional `0x`) is an authorised holder.
    fn contains_hex(&self, holder_hex: &str) -> bool {
        self.holders.contains(&normalize_hex(holder_hex))
    }
}

/// The relying party's HOLDER-BINDING policy (T3/sq-z8s7 B1): whether a
/// `HolderPop` presentation MUST carry an issuer-attested per-credential holder
/// binding ([`crate::manifest::AttestedHolderBinding`]), or whether a BEARER
/// credential (no holder binding) is still acceptable under `HolderPop` (the
/// back-compatible sq-cwq behaviour: registry membership + a fresh nonce-PoP
/// only).
///
/// # The trusted-holder gap this closes
/// The sq-cwq `HolderPop` check binds the presenter to the verifier's fresh
/// NONCE (proof of possession of *a* registry-trusted key) but NOT to the
/// CREDENTIAL the issuer issued. So trusted holder A could present trusted holder
/// B's credential: A holds *a* trusted key and can sign the nonce with it, while
/// the scan/filter sub-proofs attest B's credential. B1 closes this by
/// cross-checking the PRESENTED holder key against the issuer-attested
/// `holder_pk_digest` the issuer folded into THIS credential's signature (the
/// `ZKSIG_C4` [`sparq_zk::sig::commitment_message_with_holder`] message). When a
/// holder binding is present, the cross-check ALWAYS runs (fail-closed —
/// regardless of this policy). This policy governs only the BEARER case: whether
/// the ABSENCE of a binding is itself rejected.
///
/// # Default = back-compatible (binding NOT required)
/// [`HolderBindingPolicy::default`] / [`HolderBindingPolicy::allow_bearer`] does
/// NOT require a binding: a `HolderPop` over a bearer credential keeps its sq-cwq
/// behaviour (registry + nonce-PoP). This is the conservative default so existing
/// callers/tests are unaffected. A relying party that mandates per-credential
/// binding opts in with [`HolderBindingPolicy::require_binding`], after which a
/// bearer `HolderPop` presentation is rejected fail-closed
/// ([`CheckError::HolderBindingMissing`]) — the design's "bearer must be
/// rejectable" requirement (`research/zk-holder-pop-design.md` §1 honest scope /
/// §4.3 obligation 3).
///
/// # Tiers
/// - **B1 (clear-key, T3/sq-z8s7):** the presented holder key is DISCLOSED
///   ([`BindingMode::HolderPop`]) and the verifier recomputes its digest host-side
///   (`bind_holder_binding`), governed by [`Self::require_binding`].
/// - **B2 (hidden-key in-circuit PoK, T6/sq-c2ql):** only the issuer-attested
///   `holder_pk_digest` is public; the holder proves possession of the matching
///   secret IN ZERO KNOWLEDGE ([`crate::manifest::HolderPokProof`], verified by
///   `bind_holder_pok`). Opt in with [`Self::require_in_circuit_pok`]. This is the
///   NOT-yet-sound (sq-qhy4) hidden-holder tier — see `bind_holder_pok`.
// [OPUS-4.8] sq-z8s7 (HolderPoP T3 / B1): holder-binding policy (external,
// fail-closed bearer rejection; default back-compatible). Mirrors RevocationPolicy
// / EntailmentPolicy as a relying-party-supplied external policy object.
// [OPUS-4.8] sq-c2ql (HolderPoP T6 / B2): + opt-in in-circuit holder-PoK requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HolderBindingPolicy {
    require_binding: bool,
    /// [OPUS-4.8] sq-c2ql (B2): when set, a `HolderPop` presentation over a
    /// holder-bound credential MUST additionally carry a verifying in-circuit holder
    /// PoK ([`crate::manifest::HolderPokProof`]) for that credential's commitment.
    /// Opt-in (default off), so the clear-key (B1) path is unaffected.
    require_in_circuit_pok: bool,
}

impl HolderBindingPolicy {
    /// The back-compatible default: a `HolderPop` over a BEARER credential (no
    /// issuer-attested holder binding) is accepted on the sq-cwq registry +
    /// nonce-PoP path. A holder binding, if PRESENT, is still cross-checked
    /// fail-closed (B1) — this only governs the bearer-absent case.
    pub fn allow_bearer() -> Self {
        HolderBindingPolicy {
            require_binding: false,
            require_in_circuit_pok: false,
        }
    }

    /// REQUIRE an issuer-attested per-credential holder binding for every
    /// `HolderPop` presentation: a bearer credential (no
    /// [`crate::manifest::AttestedHolderBinding`]) is then rejected fail-closed
    /// ([`CheckError::HolderBindingMissing`]). This is the design's mandated-binding
    /// posture that fully closes the trusted-holder gap (no bearer fallback).
    pub fn require_binding() -> Self {
        HolderBindingPolicy {
            require_binding: true,
            require_in_circuit_pok: false,
        }
    }

    /// [OPUS-4.8] sq-c2ql (B2): additionally REQUIRE an in-circuit holder PoK
    /// ([`crate::manifest::HolderPokProof`]) for every holder-bound credential a
    /// `HolderPop` presentation uses — the HIDDEN-key tier. A holder-bound covering
    /// attestation with NO matching `HolderPokProof` is then rejected fail-closed
    /// ([`CheckError::HolderPokMissing`]). Builder-style on top of the current
    /// policy (so it composes with [`Self::require_binding`]).
    ///
    /// NOT-yet-sound (sq-qhy4); opt-in. Enabling this only changes a decision when a
    /// `HolderPokProof` is presented or required; the B1 clear-key gate is unchanged.
    pub fn require_in_circuit_pok(self) -> Self {
        HolderBindingPolicy {
            require_binding: self.require_binding,
            require_in_circuit_pok: true,
        }
    }

    /// Whether the relying party requires a holder binding (rejects bearer).
    fn requires_binding(&self) -> bool {
        self.require_binding
    }

    /// [OPUS-4.8] sq-c2ql (B2): whether the relying party requires an in-circuit
    /// holder PoK for each holder-bound credential.
    fn requires_in_circuit_pok(&self) -> bool {
        self.require_in_circuit_pok
    }
}

/// The relying party's ENTAILMENT-REGIME policy (sq-314): which entailment regimes
/// it will accept a manifest under. The verifier enforces this fail-closed
/// (`bind_entailment`) so `manifest.entailment_regime` is no longer free
/// metadata.
///
/// # Default = `Simple`-only (fail-closed)
/// [`EntailmentPolicy::default`] / [`EntailmentPolicy::simple_only`] accepts ONLY
/// `Simple` (no inference) — the conservative anchor, matching what v1 actually
/// proves cryptographically. A relying party that wants to accept inference opts
/// in explicitly with [`EntailmentPolicy::with_rdfs`] / [`EntailmentPolicy::with_owl`];
/// when it does, the verifier additionally requires the manifest's
/// `derivation_steps` to STRUCTURALLY ground every derived triple (see
/// `bind_entailment` + the `derivation` module). A regime the policy does not
/// accept REJECTS.
///
/// # Honest scope
/// Accepting `Rdfs`/`Owl` here means "I accept a derivation re-checked against the
/// disclosed base"; it does NOT (yet) mean an in-circuit closure proof. The
/// in-circuit single-step relation exists (`compose_core::entail`, sq-g91d,
/// research-grade / NOT-yet-sound sq-qhy4) but is not yet wired into this policy —
/// see the `derivation` module docs. A relying party that requires
/// cryptographic-strength inference keeps the `Simple`-only default. Accepting a
/// regime also says NOTHING about COMPLETENESS under that regime — the separate,
/// UNBUILT obligation `sq-rsd3v.7`; a relying party that needs it says so with
/// [`EntailmentPolicy::require_completeness_under_entailment`] and is REFUSED
/// rather than handed a soundness-only accept.
// [OPUS-4.8] sq-314: entailment-regime policy (external, fail-closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntailmentPolicy {
    accept_rdfs: bool,
    accept_owl: bool,
    // [OPUS-5] sq-rsd3v.7: the relying party demands completeness-under-entailment.
    require_completeness: bool,
}

impl Default for EntailmentPolicy {
    fn default() -> Self {
        EntailmentPolicy::simple_only()
    }
}

impl EntailmentPolicy {
    /// Accept ONLY `Simple` (no inference) — the fail-closed default.
    pub fn simple_only() -> Self {
        EntailmentPolicy { accept_rdfs: false, accept_owl: false, require_completeness: false }
    }

    /// Additionally accept `Rdfs` manifests (with grounded derivation steps).
    pub fn with_rdfs(mut self) -> Self {
        self.accept_rdfs = true;
        self
    }

    /// Additionally accept `Owl` (RDFS-RL/OWL-RL) manifests. `Owl` subsumes RDFS,
    /// so this also accepts `Rdfs`.
    pub fn with_owl(mut self) -> Self {
        self.accept_owl = true;
        self.accept_rdfs = true;
        self
    }

    /// Declare that this relying party requires **COMPLETENESS under entailment** —
    /// "no entailed answer is MISSING from the disclosed result".
    ///
    /// That property is **UNBUILT in sparq and NOT claimed** (`sq-rsd3v.7`, design
    /// `research/zk-inference-and-credentials.md` §3.7): it needs BOTH halves of
    /// [`crate::derivation::COMPLETENESS_UNDER_ENTAILMENT_UNBUILT`], and the
    /// fixpoint-saturation half exists nowhere in the estate. So setting this dial
    /// does not enable a check — it makes the verifier REFUSE, fail-closed, every
    /// non-`Simple` manifest with
    /// [`CheckError::CompletenessUnderEntailmentUnavailable`]. The point is that a
    /// relying party which needs completeness gets a MACHINE-CHECKABLE refusal
    /// naming the gap, instead of an accept it could misread as completeness — the
    /// soundness-of-derivation / completeness-under-entailment conflation this
    /// crate must never allow.
    ///
    /// # Precisely what the refusal does and does NOT assert
    /// It asserts only this: **no accepted proof under this policy rests on
    /// entailment whose completeness sparq cannot check.** A `Simple` manifest is
    /// NOT refused (there is no entailment closure for it to be complete over), but
    /// passing one is *not* an assertion that its answer set is complete — that
    /// rests on the `scan.nr` per-pattern sweep and the rest of the (not externally
    /// audited, `sq-qhy4`) verifier, not on this dial. And this dial CANNOT detect
    /// a closure materialised OFF-circuit and presented as `Simple` over the
    /// materialised graph (design §3.6(a) trusted-materialiser mode): there the
    /// regime field is honestly `Simple` and entailment is trusted to the
    /// materialiser's signature, a DIFFERENT trust model the relying party must
    /// evaluate itself.
    ///
    /// When the design's RE-ENTRY TRIGGER fires (a documented huge-closure case
    /// plus a verifier demanding full completeness — §3.6(c),
    /// `research/zkp-performance-landscape.md` §5 trigger 4), the unconditional
    /// refusal is what gets replaced by a real check; until then it is the honest
    /// answer.
    // [OPUS-5] sq-rsd3v.7: enforced deferral — a demand for completeness REFUSES.
    pub fn require_completeness_under_entailment(mut self) -> Self {
        self.require_completeness = true;
        self
    }

    /// Whether this relying party requires completeness under entailment
    /// (`sq-rsd3v.7`) — always a REFUSAL for a non-`Simple` regime, since the
    /// capability is unbuilt.
    fn requires_completeness(&self) -> bool {
        self.require_completeness
    }

    /// Whether this policy accepts `regime`.
    fn accepts(&self, regime: EntailmentRegime) -> bool {
        match regime {
            EntailmentRegime::Simple => true,
            EntailmentRegime::Rdfs => self.accept_rdfs,
            EntailmentRegime::Owl => self.accept_owl,
        }
    }
}

/// The relying party's revocation/freshness policy (audit #12). The verifier
/// requires every scan-covering credential to carry an issuer-bound status-list
/// reference, that the AUTHORITATIVE status snapshot (resolved by the relying
/// party, NOT taken from the prover — see below) show the credential's status
/// bit UNSET, and that the snapshot's version be within `[min_version, now]`
/// where `now` is the current/latest version the relying party accepts and
/// `now - min_version <= freshness_window` (a snapshot older than the window is
/// STALE and rejected).
///
/// # The authenticated-bits fix (audit #12 re-audit — the load-bearing change)
/// The issuer signature binds only the status-list REFERENCE
/// (`status_ref_digest(H(list IRI), index, version)`), NOT the bit VALUES. So the
/// status BITSTRING is unauthenticated: a prover that presents a genuine
/// issuer-signed reference can attach a FORGED all-zero
/// `manifest.status_snapshots` entry, and reading `snapshot.bit(index)` from THAT
/// would let a REVOKED credential verify — the liveness decision would rest on
/// prover-controlled bytes (the recurring "decision on an unauthenticated
/// prover-supplied input" pattern that bit #3/#8/#9/#4/#12-ref).
///
/// The fix (Option B — mirrors the audit-#3 external-trust-anchor `K` precedent):
/// the AUTHORITATIVE status-list snapshot is an EXTERNAL relying-party input
/// carried HERE, in the policy, exactly as the trusted key-set is external. The
/// verifier reads ITS OWN authoritative snapshot's `bit[index]` for the liveness
/// decision; the prover's `manifest.status_snapshots` is NEVER trusted for the
/// bit. The credential's issuer-signed `(list IRI, index, version)` ties it to a
/// specific list/slot/version; the verifier resolves the snapshot for THAT
/// `(list, version)` from its own store and reads the bit. A relying party
/// populates this store from the status-list credential(s) it fetches + verifies
/// out of band (the real W3C status-list object IS a separately-signed,
/// periodically-updated artifact; resolving it is the relying party's job, just
/// like resolving issuer keys for `K`).
///
/// If the prover ALSO discloses a snapshot for the referenced `(list, version)`,
/// it MUST byte-equal the authoritative one (else REJECT — a disagreeing
/// prover snapshot is a tamper signal); but the bit decision always uses the
/// authoritative bytes regardless. A reference whose `(list, version)` the
/// relying party has NO authoritative snapshot for is REJECTED fail-closed
/// (`StatusSnapshotMissing`): the verifier will not vouch for a liveness view it
/// cannot itself authenticate.
///
/// # Freshness model (version-as-counter)
/// `version` is a monotone counter the issuer increments each time it republishes
/// the status list (a publication sequence number / `validFrom` epoch). The
/// relying party tracks the latest version it has observed (`now`) and how far
/// back it will trust a snapshot (`freshness_window`). A snapshot at version `v`
/// is FRESH iff `min_version <= v <= now`, where `min_version = now.saturating_sub(window)`.
/// This is deliberately simple and transport-agnostic (no wall-clock in the
/// verifier core); a production relying party maps its real status-list refresh
/// cadence onto `(now, freshness_window)`.
///
/// # Fail-closed
/// There is no "no policy" opt-out: [`verify_manifest`] takes a `&RevocationPolicy`
/// mandatorily, and the status check runs unconditionally. A relying party that
/// does not (yet) track versions can use [`RevocationPolicy::accept_version`] to
/// pin the exact version it trusts (window 0), or [`RevocationPolicy::up_to`] for
/// a window. A policy with NO authoritative snapshot for a referenced
/// `(list, version)` rejects (fail-closed). There is no constructor that disables
/// the bit-unset check.
// [OPUS-4.8] audit #12: revocation / freshness policy.
// [OPUS-4.8] audit #12 re-audit (Option B): authoritative snapshot is EXTERNAL —
// the bit decision rests on relying-party-resolved bytes, never the prover's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationPolicy {
    /// The latest status-list version the relying party accepts as current.
    now: u64,
    /// How many versions back a snapshot may be and still be fresh.
    freshness_window: u64,
    /// The relying party's AUTHORITATIVE status-list snapshots, keyed by
    /// `(status_list IRI, version)`. The liveness bit decision reads from HERE,
    /// never from the prover's `manifest.status_snapshots`. Populated from the
    /// status-list credential(s) the relying party fetched + authenticated out of
    /// band (the external trust anchor for the bitstring, mirroring `K`).
    // [OPUS-4.8] audit #12 re-audit: external authoritative bitstrings.
    authoritative: std::collections::BTreeMap<(String, u64), StatusListSnapshot>,
    /// OPTIONAL Merkle-tree depth for the HIDDEN-INDEX revocation proof (sq-3e5 /
    /// sq-h2v). When set, the verifier derives the authoritative status-list
    /// Merkle root from its own [`StatusListSnapshot`] at this depth and checks a
    /// `manifest.hidden_revocation` proof's PUBLIC root byte-equals it (so the
    /// holder's index is never disclosed). `None` => no hidden-index proof is
    /// accepted (the relying party only runs the clear-index path). MUST equal the
    /// depth the prover used (the `revoke_unset_d{depth}` member).
    // [OPUS-4.8] sq-3e5 + sq-h2v: authoritative-root derivation depth.
    hidden_index_depth: Option<u32>,
    /// [OPUS-5] sq-6qe / sq-kndw: OPTIONAL Merkle depth for the ACCEPTED-SET
    /// commitment — the relying party's `(list, version, status_list_root)` trust
    /// anchor moved behind a single root, so a fully-hidden revocation proof can
    /// hide WHICH list/version it pertains to. `None` => the accepted-set anchor is
    /// not derived AND the fully-hidden path is DISABLED (a
    /// `manifest.fully_hidden_revocation` proof is rejected fail-closed — the
    /// verifier will not accept an anchor it cannot itself derive). Setting it is
    /// the relying party's opt-in; the clear-index and committed-index paths, which
    /// still disclose the IRI + version, are unaffected.
    // [OPUS-5] sq-kndw: accepted-set anchor depth = the fully-hidden opt-in.
    accepted_set_depth: Option<u32>,
    /// [OPUS-5] sq-kndw: an EXPLICIT epoch floor, overriding the one derived from
    /// `now - freshness_window`. `None` => the derived floor. See
    /// [`RevocationPolicy::with_min_version`] for why an explicit floor is
    /// consistent rather than a second, conflicting freshness rule.
    // [OPUS-5] sq-kndw: explicit public epoch floor.
    explicit_min_version: Option<u64>,
}

impl RevocationPolicy {
    /// Accept snapshots in `[now - window, now]` (a freshness window of `window`
    /// versions ending at `now`). Carries NO authoritative snapshots yet — attach
    /// them with [`Self::with_snapshot`] / [`Self::with_snapshots`]; a referenced
    /// `(list, version)` with no attached authoritative snapshot is rejected
    /// fail-closed.
    pub fn up_to(now: u64, freshness_window: u64) -> Self {
        RevocationPolicy {
            now,
            freshness_window,
            authoritative: std::collections::BTreeMap::new(),
            hidden_index_depth: None,
            accepted_set_depth: None,
            explicit_min_version: None,
        }
    }

    /// Accept EXACTLY version `v` (window 0) — the relying party pins the single
    /// status-list version it has resolved and trusts. Attach the authoritative
    /// snapshot(s) with [`Self::with_snapshot`] / [`Self::with_snapshots`].
    pub fn accept_version(v: u64) -> Self {
        RevocationPolicy {
            now: v,
            freshness_window: 0,
            authoritative: std::collections::BTreeMap::new(),
            hidden_index_depth: None,
            accepted_set_depth: None,
            explicit_min_version: None,
        }
    }

    /// Enable the HIDDEN-INDEX revocation path (sq-3e5 / sq-h2v) at Merkle depth
    /// `depth` (builder style). The verifier will then derive the authoritative
    /// status-list Merkle root from its own snapshot at this depth and accept a
    /// `manifest.hidden_revocation` proof whose PUBLIC root matches — without the
    /// holder disclosing its index. `depth` MUST equal the `revoke_unset_d{depth}`
    /// member the prover used. The clear-index path is unaffected.
    // [OPUS-4.8] sq-3e5 + sq-h2v: opt-in hidden-index verification depth.
    pub fn with_hidden_index_depth(mut self, depth: u32) -> Self {
        self.hidden_index_depth = Some(depth);
        self
    }

    /// Attach one AUTHORITATIVE status-list snapshot (builder style). The relying
    /// party resolves + authenticates this out of band (a verified W3C status-list
    /// credential); the verifier reads its `bit[index]` for the liveness decision,
    /// NEVER the prover's snapshot. Keyed by `(status_list, version)`; a later
    /// snapshot for the same key replaces an earlier one.
    // [OPUS-4.8] audit #12 re-audit: relying-party authoritative bitstring.
    pub fn with_snapshot(mut self, snapshot: StatusListSnapshot) -> Self {
        self.authoritative
            .insert((snapshot.status_list.clone(), snapshot.version), snapshot);
        self
    }

    /// Attach several authoritative snapshots at once (builder style).
    // [OPUS-4.8] audit #12 re-audit.
    pub fn with_snapshots<I>(mut self, snapshots: I) -> Self
    where
        I: IntoIterator<Item = StatusListSnapshot>,
    {
        for s in snapshots {
            self.authoritative.insert((s.status_list.clone(), s.version), s);
        }
        self
    }

    /// The relying party's authoritative snapshot for `(list, version)`, if it has
    /// resolved one. `None` => the verifier has no authenticated liveness view for
    /// this reference and rejects fail-closed.
    // [OPUS-4.8] audit #12 re-audit: authoritative lookup (never the prover's).
    fn authoritative_snapshot(&self, list: &str, version: u64) -> Option<&StatusListSnapshot> {
        self.authoritative.get(&(list.to_string(), version))
    }

    /// The hidden-index Merkle depth, if the relying party enabled the
    /// hidden-index revocation path. `None` => the path is disabled (a
    /// `manifest.hidden_revocation` is then not accepted as the liveness check).
    // [OPUS-4.8] sq-3e5 + sq-h2v.
    fn hidden_index_depth(&self) -> Option<u32> {
        self.hidden_index_depth
    }

    /// [OPUS-5] sq-6qe: derive the ACCEPTED-SET commitment (sub-option A of
    /// `research/zk-statuslist-hide-iri-version.md` §3) at Merkle depth `depth`
    /// (builder style) — the relying party's `(list, version, status_list_root)`
    /// trust anchor folded into ONE root, so a fully-hidden revocation proof can
    /// show "some accepted `(list, version)` has my hidden index unset" without
    /// disclosing which.
    ///
    /// # [OPUS-5] sq-kndw: this now ENABLES a verification path
    /// Setting it (together with [`Self::with_hidden_index_depth`]) is what OPTS
    /// THE RELYING PARTY IN to the fully-hidden revocation mode: it makes
    /// [`Self::accepted_set_root`] derivable, and `bind_fully_hidden_revocation`
    /// binds a `revoke_hidden_ref_d{depth}_a{set_depth}` proof to that anchor.
    /// Without it a `fully_hidden_revocation` proof is rejected
    /// (`FullyHiddenRevocationNotEnabled`) — the verifier will not accept an anchor
    /// it cannot itself derive. The clear-index and committed-index paths are
    /// unaffected. Not externally audited (sq-qhy4).
    ///
    /// `depth` must be wide enough for the freshness-curated accepted set
    /// (`2^depth >= |entries|`), else [`Self::accepted_set_root`] is `None`
    /// (fail-closed — never a truncated anchor). The accepted-set root also
    /// requires [`Self::with_hidden_index_depth`], since each entry carries the
    /// status-list Merkle root derived at THAT depth.
    // [OPUS-5] sq-6qe: opt-in accepted-set anchor depth.
    pub fn with_accepted_set_depth(mut self, depth: u32) -> Self {
        self.accepted_set_depth = Some(depth);
        self
    }

    /// [OPUS-5] sq-6qe: the relying party's accepted `(list, version,
    /// status_list_root)` entries in CANONICAL leaf order, FRESHNESS-CURATED.
    ///
    /// Order is the sorted `(status_list, version)` order of the authoritative
    /// snapshot map — the canonical order both the relying party (deriving the
    /// anchor) and the prover (building its membership path) must commit, exactly
    /// as [`KeySet::hidden_issuer_root`] fixes the key-set order.
    ///
    /// # Freshness curation is the soundness-relevant part
    /// Only snapshots whose version is INSIDE the policy's freshness window
    /// `[min_version, now]` become entries. Because the future circuit's liveness
    /// statement is membership in this set, a STALE (or future-dated) version is
    /// not a member at all and no proof can be built against it — the audit-#12
    /// freshness gate survives the move behind the commitment. The in-circuit
    /// `version >= min_version` comparison of the design record is then
    /// defence-in-depth on top of membership, not the only freshness check.
    ///
    /// `None` if the hidden-index depth is unset (there is no depth at which to
    /// derive each entry's status-list root) or any snapshot's root is
    /// underivable at that depth — fail-closed, never a partial anchor.
    // [OPUS-5] sq-6qe: canonical, freshness-curated accepted entries.
    pub fn accepted_entries(&self) -> Option<Vec<crate::revocation::AcceptedStatusEntry>> {
        let depth = self.hidden_index_depth?;
        let mut entries = Vec::new();
        for ((list, version), snapshot) in &self.authoritative {
            if !self.is_fresh(*version) {
                continue;
            }
            let root = crate::revocation::merkle_root(snapshot, depth)?;
            entries.push(crate::revocation::AcceptedStatusEntry {
                status_list: list.clone(),
                version: *version,
                status_list_root: root,
            });
        }
        Some(entries)
    }

    /// [OPUS-5] sq-6qe: the AUTHORITATIVE accepted-set Merkle root over
    /// [`Self::accepted_entries`] at the policy's accepted-set depth — the public
    /// input a future fully-hidden revocation proof would be bound to, derived
    /// from the relying party's OWN curated snapshots (never the prover's).
    ///
    /// `None` if [`Self::with_accepted_set_depth`] / [`Self::with_hidden_index_depth`]
    /// were not set, or the curated set overflows the depth (fail-closed).
    // [OPUS-5] sq-6qe: accepted-set trust anchor (host side only — no gate yet).
    pub fn accepted_set_root(&self) -> Option<Fr> {
        let set_depth = self.accepted_set_depth?;
        crate::revocation::accepted_set_root(&self.accepted_entries()?, set_depth)
    }

    /// [OPUS-5] sq-6qe: the 0-based slot of `(list, version)` in the canonical
    /// accepted-set leaf order — the index a prover proves membership at (private
    /// in-circuit; the verifier never needs it). `None` if the pair is not a
    /// freshness-curated member. Mirrors [`KeySet::member_index`].
    // [OPUS-5] sq-6qe: prover-side convenience.
    pub fn accepted_member_index(&self, list: &str, version: u64) -> Option<usize> {
        self.accepted_entries()?
            .iter()
            .position(|e| e.status_list == list && e.version == version)
    }

    /// The oldest version still considered fresh — the policy's epoch FLOOR.
    ///
    /// [OPUS-5] sq-6qe: this is also the public `min_version` input of the
    /// fully-hidden revocation member (sq-kndw). It discloses only the relying
    /// party's own policy floor, not the credential's epoch.
    ///
    /// [OPUS-5] sq-kndw: returns the EXPLICIT floor when [`Self::with_min_version`]
    /// set one, else the window-derived `now - freshness_window`.
    pub fn min_version(&self) -> u64 {
        self.explicit_min_version
            .unwrap_or_else(|| self.now.saturating_sub(self.freshness_window))
    }

    /// [OPUS-5] sq-kndw: pin an EXPLICIT public epoch floor (builder style),
    /// replacing the `now - freshness_window` one.
    ///
    /// This is ONE floor, not a second freshness rule: [`Self::min_version`] is the
    /// single definition of the floor and both the (private) freshness-window check
    /// and [`Self::accepted_entries`] read it, so the clear-path freshness window,
    /// the accepted-set curation, and the fully-hidden member's public
    /// `min_version` input can never disagree.
    /// Setting a floor BELOW the derived one deliberately widens what the relying
    /// party accepts (its own policy call); setting one ABOVE narrows it. A floor
    /// above `now` accepts nothing (the fresh window is then empty) — fail-closed.
    ///
    /// Useful when the floor is a published policy constant rather than a rolling
    /// window, since it is a PUBLIC input of every fully-hidden proof: a rolling
    /// floor changes the public input (and so the anchor a holder must prove
    /// against) on every tick.
    // [OPUS-5] sq-kndw: explicit epoch floor.
    pub fn with_min_version(mut self, min_version: u64) -> Self {
        self.explicit_min_version = Some(min_version);
        self
    }

    /// [OPUS-5] sq-kndw: the accepted-set Merkle depth, if the relying party
    /// enabled the fully-hidden path. `None` => the path is disabled (a
    /// `manifest.fully_hidden_revocation` proof is then not accepted).
    fn accepted_set_depth(&self) -> Option<u32> {
        self.accepted_set_depth
    }

    /// Whether `v` is within the freshness window `[min_version, now]`.
    fn is_fresh(&self, v: u64) -> bool {
        v >= self.min_version() && v <= self.now
    }
}

/// A verifier-issued freshness nonce (audit #4) — a fresh BN254 field element the
/// relying party mints out of band and hands to the prover BEFORE proving. The
/// prover MUST incorporate it as the circuit's `challenge` public input (field 0)
/// at prove time; [`verify_manifest`] then reconstructs the public-input vector
/// using THIS nonce (never `manifest.binding`), so the existing audit-#1
/// byte-compare rejects any proof whose committed challenge ≠ the verifier's
/// nonce. A captured manifest re-presented under a fresh nonce therefore fails
/// the byte-compare (replay defence), and the [`SeenNonces`] store below rejects
/// a nonce presented twice (single-use defence).
///
/// Stored as a normalized field element so `0x`-padding differences cannot make
/// two presentations of the same nonce look distinct to the single-use store.
// [OPUS-4.8] audit #4: verifier-issued freshness nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifierNonce(Fr);

impl VerifierNonce {
    /// Adopt a caller-chosen field element as the nonce (e.g. from an external
    /// CSPRNG mapped into the field, or a fresh per-session value). The relying
    /// party is responsible for unpredictability/uniqueness; this type only
    /// guarantees a canonical field representation for the binding + single-use
    /// machinery. `None` if `hex` is not a parseable field element.
    pub fn from_hex(hex: &str) -> Option<Self> {
        field_from_hex_str(hex).map(VerifierNonce)
    }

    /// Adopt a field element directly (e.g. one drawn by the relying party's RNG
    /// and reduced into the field).
    pub fn from_field(f: Fr) -> Self {
        VerifierNonce(f)
    }

    /// The nonce as the `FieldHex` the circuit challenge / reconstruction uses.
    pub fn as_field_hex(&self) -> FieldHex {
        FieldHex(field_to_hex(&self.0))
    }

    /// Canonical hex key for the single-use store (representation-insensitive).
    fn canonical_key(&self) -> String {
        field_to_hex(&self.0)
    }
}

/// Single-use nonce store (audit #4): records every verifier nonce a manifest has
/// already been accepted (or attempted) under, so a captured (nonce, manifest)
/// pair cannot be replayed. [`verify_manifest`] calls [`SeenNonces::record_fresh`]
/// BEFORE the cryptographic gate; a nonce already present REJECTS
/// ([`CheckError::NonceReplay`]).
///
/// # Fail-closed contract
/// `record_fresh` must (a) return `false` (already-seen) if the nonce was ever
/// previously recorded, and (b) atomically mark it recorded otherwise. The store
/// is consulted on EVERY `verify_manifest` call with NO opt-out — there is no
/// "skip when the store is absent" path (the parameter is mandatory), so the
/// single-use property cannot be bypassed by omitting a field.
///
/// # Persistence — pick the right impl, durability is NOT optional in production
/// This crate ships TWO implementations and the choice is load-bearing for the
/// audit-#4 single-use guarantee:
///
/// - [`FileSeenNonces`] (audit #4 durability fix, sq-aih) — DURABLE. Backed by an
///   append-only file with an `fsync` + an advisory `flock(LOCK_EX)` around every
///   check-and-append, so a recorded nonce SURVIVES A RESTART and the
///   check-and-insert is atomic across concurrent processes sharing the path. THIS
///   is what a relying party that can be restarted (i.e. every real deployment)
///   MUST use. See its docs for the exact durability/atomicity contract and the
///   honest "reference impl vs. a DB UNIQUE constraint" caveats.
/// - [`InMemorySeenNonces`] — NON-DURABLE, TEST / single-session ONLY. Process-local:
///   it enforces single-use only WITHIN one verifier process. A replayed proof is
///   accepted again after a restart because the seen-set is lost. It is deliberately
///   labelled test-only and MUST NOT be used by a relying party that can restart.
///
/// The trait boundary exists so the persistence choice is pluggable; a relying
/// party with an existing datastore can also back it with a database row carrying a
/// UNIQUE constraint on the nonce or a KV store with compare-and-set — either gives
/// the same restart-surviving, atomic semantics as [`FileSeenNonces`].
// [OPUS-4.8] audit #4: single-use nonce store. sq-aih: durability is mandatory —
// FileSeenNonces is the durable impl; InMemory is test-only.
pub trait SeenNonces {
    /// Record `nonce` as used and return `true` iff it was FRESH (not previously
    /// recorded). Returns `false` if the nonce was already seen — the verifier
    /// then rejects the manifest as a replay. Implementations MUST be atomic
    /// (check-and-insert) so concurrent verifiers cannot both observe the same
    /// nonce as fresh.
    fn record_fresh(&self, nonce: &VerifierNonce) -> bool;
}

/// Process-local, thread-safe [`SeenNonces`] (audit #4) — **NON-DURABLE, TEST /
/// single-session ONLY.**
///
/// # ⚠️ NOT durable across restarts — DO NOT use in a restartable relying party.
/// This enforces single-use only WITHIN one verifier process: the seen-set lives
/// in memory and is LOST on restart, so a captured (nonce, manifest) pair that was
/// already rejected once is ACCEPTED AGAIN after the process restarts. That defeats
/// the audit-#4 single-use guarantee for any deployment that can be restarted (i.e.
/// every real one). It exists for unit tests and ephemeral single-session tooling
/// only. A relying party MUST use [`FileSeenNonces`] (durable: survives restart, is
/// the sq-aih fix) or another durable backing (a DB row with a UNIQUE constraint /
/// a KV store with compare-and-set).
// [OPUS-4.8] audit #4 / sq-aih: in-memory store is NON-DURABLE, test-only. The
// durable impl is FileSeenNonces — single-use survives a restart there.
#[derive(Debug, Default)]
pub struct InMemorySeenNonces {
    seen: std::sync::Mutex<BTreeSet<String>>,
}

impl InMemorySeenNonces {
    pub fn new() -> Self {
        InMemorySeenNonces { seen: std::sync::Mutex::new(BTreeSet::new()) }
    }
}

impl SeenNonces for InMemorySeenNonces {
    fn record_fresh(&self, nonce: &VerifierNonce) -> bool {
        // `.insert` returns true iff the value was NOT already present — exactly
        // the "was fresh" semantics. A poisoned lock fails closed (treat as
        // already-seen / reject) rather than panicking on prover-triggerable
        // input: a poisoned mutex means another verify panicked mid-record, and
        // we must not optimistically accept a possibly-replayed nonce.
        match self.seen.lock() {
            Ok(mut set) => set.insert(nonce.canonical_key()),
            Err(_) => false,
        }
    }
}

/// DURABLE, restart-surviving, cross-process single-use [`SeenNonces`] — the
/// audit-#4 durability fix (sq-aih). Backed by an APPEND-ONLY file of canonical
/// nonce keys (one per line). Single-use SURVIVES A RESTART: the recorded set is
/// the file's contents, so reopening the same path re-loads every previously-seen
/// nonce and a replayed proof is still rejected.
///
/// # Atomicity / concurrency contract
/// Every [`record_fresh`](SeenNonces::record_fresh) does, under an advisory
/// exclusive `flock(LOCK_EX)` held on the file fd for the whole operation:
/// 1. seek to start, read the WHOLE file, parse it into the seen-set (the file —
///    not an in-RAM cache — is the source of truth, so a concurrent process that
///    appended between calls is observed);
/// 2. if the nonce is already present, release the lock and return `false`
///    (replay);
/// 3. otherwise append `key\n`, `flush` + `fsync` the file (durably on disk before
///    we report fresh), release the lock, return `true`.
///
/// `flock(LOCK_EX)` serialises step 1–3 across ALL processes that opened the SAME
/// path on the SAME machine, so two concurrent verifiers cannot both observe one
/// nonce as fresh (the check-and-append is atomic). An in-process `Mutex` also
/// guards the fd so threads in one process can't interleave the seek/read/append.
///
/// # Honest scope — reference impl, not a clustered DB
/// This is a single-file, single-host reference implementation chosen to give a
/// real restart-surviving guarantee with NO heavy dependency (just `libc::flock`):
/// - `flock` is ADVISORY and per-host. It does NOT coordinate across machines /
///   NFS mounts that don't honour `flock` / networked filesystems. A multi-HOST
///   relying party MUST use a shared transactional store instead (a DB row with a
///   `UNIQUE` constraint on the nonce, or a KV store with compare-and-set) —
///   exactly what the [`SeenNonces`] trait boundary is for.
/// - The file GROWS unbounded (one line per nonce ever seen). For a high-volume
///   deployment, rotate/compact behind the same path or use a DB. The set is also
///   re-read from disk on every call (O(file) per verify) — fine for a reference /
///   moderate-volume relying party, replace with an indexed store at scale.
/// - A poisoned in-process mutex, an I/O error, a lock failure, or a malformed line
///   all FAIL CLOSED (return `false` = treat as already-seen / reject), never
///   accept-on-error and never panic on prover-triggerable input.
// [OPUS-4.8] sq-aih (audit #4 durability): durable append-only + flock single-use
// store. Single-use survives a restart. Reference impl — see scope caveats above.
#[derive(Debug)]
pub struct FileSeenNonces {
    /// The append-only nonce log. Held open for the lifetime of the store so the
    /// advisory `flock` and the appends target a stable fd. Guarded by a process-
    /// local mutex so threads within one process serialise too.
    file: std::sync::Mutex<std::fs::File>,
}

impl FileSeenNonces {
    /// Open (creating if absent) the durable nonce log at `path`. Any nonces
    /// already recorded there are immediately in force — reopening the same path
    /// after a restart re-loads them, so single-use survives the restart.
    ///
    /// Returns an `io::Error` if the file cannot be opened/created; a relying party
    /// MUST treat that as fatal (it has no durable single-use store and must not
    /// fall back to the non-durable in-memory one silently).
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path.as_ref())?;
        Ok(FileSeenNonces { file: std::sync::Mutex::new(file) })
    }

    /// Whether `key` is already present in the (locked) file. Reads the WHOLE file
    /// from the start so a concurrent appender is observed. `Err` on an I/O failure
    /// (the caller fails closed). The fd MUST already hold the advisory lock.
    fn contains_key_locked(file: &mut std::fs::File, key: &str) -> std::io::Result<bool> {
        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(0))?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;
        // Each non-empty line is a canonical nonce key. Trailing/embedded blank
        // lines (e.g. a partial write) are ignored — they can never equal a
        // canonical key, so ignoring them cannot mask a real replay.
        Ok(buf.lines().any(|line| line == key))
    }
}

impl SeenNonces for FileSeenNonces {
    fn record_fresh(&self, nonce: &VerifierNonce) -> bool {
        use std::io::Write;
        use std::os::unix::io::AsRawFd;

        // Serialise threads in THIS process; cross-process serialisation is the
        // `flock` below. A poisoned mutex fails closed (another verify panicked
        // mid-record — do not optimistically accept a possibly-replayed nonce).
        let mut file = match self.file.lock() {
            Ok(f) => f,
            Err(_) => return false,
        };

        let fd = file.as_raw_fd();
        // Advisory EXCLUSIVE lock for the whole check-and-append: this is what
        // makes the operation atomic ACROSS PROCESSES sharing the path. Blocks
        // until acquired; an error fails closed.
        // SAFETY: `fd` is a valid open file descriptor owned by `file` for the
        // duration of this call (the MutexGuard keeps `file` alive).
        if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
            return false;
        }

        // Unlock helper — runs on every return path below so we never leak the
        // advisory lock to the next caller (a leaked lock would deadlock).
        let unlock = |fd: i32| {
            // SAFETY: same valid, locked fd; LOCK_UN releases our advisory lock.
            let _ = unsafe { libc::flock(fd, libc::LOCK_UN) };
        };

        let key = nonce.canonical_key();

        // Step 1: is it already recorded (in the durable file)? Fail closed on I/O
        // error — we must not report a nonce fresh if we couldn't read the log.
        match Self::contains_key_locked(&mut file, &key) {
            Ok(true) => {
                unlock(fd);
                return false; // replay
            }
            Ok(false) => {}
            Err(_) => {
                unlock(fd);
                return false; // fail closed
            }
        }

        // Step 2: durably append the new key, then fsync BEFORE reporting fresh —
        // so a crash after we return `true` cannot lose the record (which would
        // re-open the replay window). The file was opened in append mode, so the
        // write lands at EOF regardless of the seek done by the read above.
        let mut line = key;
        line.push('\n');
        if file.write_all(line.as_bytes()).is_err()
            || file.flush().is_err()
            || file.sync_all().is_err()
        {
            // The append may be partially written; a partial (non-newline-
            // terminated) line is ignored on the next read, and we report
            // not-fresh so this presentation is rejected fail-closed. The nonce is
            // NOT durably recorded, but rejecting here is the safe (non-accepting)
            // direction.
            unlock(fd);
            return false;
        }

        unlock(fd);
        true
    }
}

/// Why a manifest was rejected.
#[derive(Debug)]
pub enum CheckError {
    /// The sparq-zk layer-3 re-check failed (parse / arity / bnode obligation).
    Sparqzk(VerifyError),
    /// A sub-proof's declared circuit id does not match the id re-derived from
    /// its public inputs.
    CircuitIdMismatch { proof: usize, declared: CircuitId, derived: Option<CircuitId> },
    /// A binding edge references a non-existent proof / row / slot.
    DanglingEdge { edge: usize },
    /// A binding edge connects proofs whose kinds cannot be bound (e.g. the
    /// source is not a scan or the target is not a filter).
    EdgeKindMismatch { edge: usize },
    /// A binding edge's encodings disagree: the scanned column does not equal
    /// the filter's operand (the join the prover claimed is a lie).
    BindingInconsistent { edge: usize },
    /// bb rejected a sub-proof.
    ProofRejected { proof: usize },
    /// A sub-proof carried no bb proof bytes (structure-only manifest passed to
    /// the full verifier).
    MissingProof { proof: usize },
    /// A sub-proof's `proof_hex` blob is malformed (non-hex, odd length, a
    /// truncated/oversized length prefix) — rejected before any bb call rather
    /// than panicking (audit hardening: route prover-controlled bytes through
    /// the REJECT channel).
    MalformedProof { proof: usize },
    /// A declared `ProofInputs` field is not a parseable BN254 field element, so
    /// the public-input vector cannot be reconstructed (audit #1).
    MalformedField { proof: usize, what: &'static str },
    /// The public-input vector reconstructed from the declared `ProofInputs`
    /// (audit #1) does not byte-match the prover's `public_inputs` blob: the
    /// JSON statement and the cryptographic proof describe different statements.
    PublicInputMismatch { proof: usize },
    /// A query BGP pattern's constant slots have no scan sub-proof whose bound
    /// `pattern_is_const`/`pattern_const_enc` match them (audit #10): the
    /// disclosed scan does not actually answer the query's pattern (e.g. an age
    /// scan presented under a salary query — constant-swap).
    UnboundPattern { pattern: usize },
    /// A query FILTER has no bound `filter_int` sub-proof matching its operator
    /// and constant, reachable via a binding edge from the scan slot the FILTER
    /// variable binds to, with a true verdict (audit #5/#6/#10): the FILTER is
    /// unproven (FILTER-add, comparison-substitution, wrong-operand-slot, or an
    /// `expected==false` row presented as passing).
    UnboundFilter { variable: String },
    /// A query FILTER constrains a variable that does not bind to any scanned
    /// column of a BGP pattern (cannot be mapped to a `filter_int` operand).
    UnmappableFilterVar { variable: String },
    /// `manifest.pattern_scans` is DECLARED (non-empty) but does not carry
    /// exactly one entry per query BGP pattern (sq-q9r5e follow-up): the
    /// pattern→scan mapping is indexed in query order like `attributions`, so a
    /// mis-sized declaration cannot be interpreted and is rejected fail-closed.
    /// (The FILTER/attribution obligations themselves are unaffected by a
    /// declaration — see `check_pattern_scans`.)
    // [OPUS-5] sq-q9r5e follow-up: explicit pattern→scan mapping.
    PatternScanArityMismatch { patterns: usize, declared: usize },
    /// A DECLARED `manifest.pattern_scans[pattern]` is EMPTY: the prover declared
    /// the mapping but left this query BGP pattern with no answering scan. Every
    /// pattern must be answered (the declared analogue of `UnboundPattern`).
    // [OPUS-5] sq-q9r5e follow-up.
    PatternScanUnbound { pattern: usize },
    /// A DECLARED `manifest.pattern_scans[pattern]` names sub-proof `proof`,
    /// which is out of range, is not a SCAN, or whose bb-bound
    /// `pattern_is_const`/`pattern_const_enc` do NOT match the query pattern's
    /// constant slots (audit #10). A declaration must not claim a scan answers a
    /// pattern it provably does not.
    // [OPUS-5] sq-q9r5e follow-up.
    PatternScanMismatch { pattern: usize, proof: usize },
    /// `manifest.pattern_scans` is DECLARED but scan sub-proof `proof` is named
    /// by NO pattern: the manifest discloses that scan's rows while its own
    /// declared reading gives them no pattern, which is incoherent, so it is
    /// rejected fail-closed rather than recorded.
    // [OPUS-5] sq-q9r5e follow-up.
    PatternScanUndeclared { proof: usize },
    /// A query BGP pattern uses the SAME variable at two slots (`{ ?v <p> ?v }`)
    /// but a disclosed row of a scan that answers that pattern binds those two
    /// slots to DIFFERENT terms — so the row does not satisfy the pattern it is
    /// disclosed under, and a relying party would read `?v` as two terms at once.
    /// `slots` is the `(first, offending)` slot pair within the pattern.
    ///
    /// The cross-pattern analogues of this obligation are the disclosed-path
    /// `recheck` / `join_obligations` gate and the hidden-path `bind_joins` slot
    /// binding; both iterate pattern PAIRS over per-pattern variable SETS, so the
    /// WITHIN-pattern repetition is invisible to them. `scan_matches_pattern` only
    /// compares per-slot const-ness/encodings and the scan circuit binds only the
    /// CONSTANT slots to `pattern_const_enc`, so nothing else constrains it.
    // [OPUS-5] #5240 (raised while fixing audit L-1 / sq-q9r5e). Research-grade,
    // NOT externally audited (sq-qhy4).
    RepeatedSlotMismatch {
        pattern: usize,
        proof: usize,
        row: usize,
        variable: String,
        slots: (usize, usize),
    },
    /// A scan sub-proof's commitment has no issuer attestation in the manifest
    /// (audit #3): `commitments[g]` is unsigned / prover-invented, so the
    /// "credential issued by X" claim has no cryptographic backing. Closes the
    /// unsigned-commitment forgery and the drop-a-triple-and-recommit
    /// suppression (a truncated-leaf recommit yields a different `C(G)` with no
    /// valid attestation).
    // [OPUS-4.8] audit #3.
    UnattestedCommitment { proof: usize, commitment: String },
    /// An attestation's signature did not verify under its declared issuer key
    /// (audit #3): forged/tampered signature, or the signature is over a
    /// different commitment than declared.
    // [OPUS-4.8] audit #3.
    InvalidIssuerSignature { commitment: String },
    /// An attestation's issuer key is not a member of the EXTERNAL trusted
    /// key-set `K` (audit #3, codex #1): a signature by a key outside the
    /// relying-party-supplied trusted set is rejected. The trust anchor is the
    /// verifier's [`KeySet`] argument, NOT the prover's `manifest.key_set`.
    // [OPUS-4.8] audit #3 / codex #1.
    IssuerKeyNotInKeySet { commitment: String },
    /// The prover's `manifest.key_set` lists a key that is NOT in the external
    /// trusted key-set `K` (audit #3 codex #1): the prover tried to widen the
    /// trust anchor with a key the relying party never trusted. The manifest's
    /// declared key-set must be a SUBSET of the external `K`; a superset is
    /// rejected so the accept decision can never depend on a prover-chosen key.
    // [OPUS-4.8] audit #3 / codex #1.
    UntrustedDeclaredKey { key: String },
    /// The prover DECLARED a narrowed `manifest.key_set` (non-empty) but an
    /// accepted attestation's issuer key is not a member of it (audit #3 codex
    /// 2216 LOW): the declared narrowed set is inconsistent with the
    /// attestations actually proven. The accept decision stays anchored on the
    /// external trusted `K` (an attestation key is ALWAYS required to be in `K`);
    /// this additionally enforces internal consistency of a declared narrowing,
    /// so a prover cannot advertise a tighter issuer set than it actually used.
    // [OPUS-4.8] codex 2216 LOW.
    AttestationKeyNotInDeclaredSet { commitment: String, key: String },
    /// A query BGP pattern's declared `manifest.attributions[pattern]` does not
    /// cover the PROOF-BOUND set of graphs the answering scan sub-proof's
    /// in-circuit attribution shows it actually drew matched triples from (audit
    /// #8): the prover under-declared a contributing graph to collapse a genuine
    /// cross-graph join and drop its non-bnode obligation (the `[[0],[0]]`
    /// forge). `proof_graph` is a graph index the scan proved a contribution
    /// from that `manifest.attributions[pattern]` omits.
    // [OPUS-4.8] audit #8.
    AttributionUnderDeclared { pattern: usize, proof_graph: usize },
    /// A query BGP pattern matched no scan sub-proof carrying a proof-bound
    /// attribution to cross-check (audit #8): every BGP pattern must be answered
    /// by a scan (already enforced by stage 2b `UnboundPattern`), so this is the
    /// belt-and-braces fail-closed if attribution binding cannot find the
    /// answering scan.
    // [OPUS-4.8] audit #8.
    AttributionUnbound { pattern: usize },
    /// Two DISTINCT committed graphs disclosed the SAME per-graph bnode salt
    /// (audit #9): a reused salt makes a same-label canonical bnode encode
    /// identically across both graphs — the Q6 cross-graph correlation handle.
    /// Each graph must be committed under a globally-unique issuer-attested salt.
    // [OPUS-4.8] audit #9.
    SaltReused { salt: String },
    /// The attestation covering a scan sub-proof's commitment carries NO salt
    /// (`salt: None`) — fail-closed for audit #9 / codex 2221 HIGH. A salt-less
    /// (legacy) attestation does NOT bind the per-graph RDFC10 salt into the
    /// issuer signature and does NOT participate in the salt-uniqueness check, so
    /// accepting one for a scan-covering commitment would silently bypass the #9
    /// salt-separation guarantee (a salt-reusing ingester just omits the salt
    /// field). Every attestation that covers a verified scan commitment MUST carry
    /// a salt and verify via the salt-bound `commitment_message_with_salt` path.
    // [OPUS-4.8] codex 2221 HIGH: salt-bound attestation is mandatory for scans.
    ScanCommitmentSaltMissing { proof: usize, commitment: String },
    /// A scan sub-proof's `attribution` vector is absent/empty or the wrong
    /// length (codex 2221 MEDIUM, fail-closed for audit #8). The per-graph source
    /// attribution is a security-relevant, proof-bound quantity (`scan.nr` step 4)
    /// that the cross-graph obligation gate (stage 2e) cross-checks against
    /// `manifest.attributions`. `#[serde(default)]` lets a prover OMIT it (empty
    /// vec) or under-length it, and `bind_attributions` only checks the bits
    /// present — so an omitted attribution makes the #8 cross-check vacuous,
    /// resurrecting the `[[0],[0]]` collapse forge. It MUST be present and EXACTLY
    /// `CircuitId.k` bits (no default/pad-to-false). `expected` = k.
    // [OPUS-4.8] codex 2221 MEDIUM: attribution must be present + exactly k bits.
    AttributionMalformed { proof: usize, expected: usize, got: usize },
    /// A scan sub-proof's per-graph `commitments` are NOT strictly increasing on
    /// the field representative (plan S2.5, [OPUS-4.8]): two adjacent commitments
    /// are equal or out of order. `scan_check` step 1b enforces
    /// `commitments[0] < commitments[1] < ...` in-circuit to force the committed
    /// graphs pairwise DISTINCT — closing the duplicate-inclusion / COUNT-forgery
    /// class (the same credential committed twice repeats a commitment). This is
    /// the host-side, structural mirror of that gate: it rejects a non-increasing
    /// commitment vector BEFORE any bb proof (defence in depth), so a witness-only
    /// manifest cannot smuggle a duplicate past the structural stage either. The
    /// honest builder ([`crate::build::build_scan`]) emits commitments ascending,
    /// so a consistent honest manifest never trips this. `at` is the index `g`
    /// whose `commitments[g] <= commitments[g-1]`.
    // [OPUS-4.8] sq-vxq8: distinct-graph strict-ordering (duplicate-inclusion closure).
    ScanCommitmentsNotStrictlyIncreasing { proof: usize, at: usize },
    /// The verifier-issued nonce has already been seen by the single-use store
    /// (audit #4): a captured (nonce, manifest) pair re-presented to the SAME
    /// verifier session. Rejected before the cryptographic gate so a bearer proof
    /// cannot be replayed. The honest flow uses each verifier nonce exactly once.
    // [OPUS-4.8] audit #4: single-use.
    NonceReplay,
    /// The manifest's declared `binding` challenge does not equal the
    /// verifier-issued nonce (audit #4). The proof's committed challenge (field 0)
    /// is byte-bound to the nonce by the audit-#1 reconstruction; this additional
    /// check fails closed when the JSON `binding` advertises a DIFFERENT challenge
    /// than the verifier issued (a manifest minted for a different session/nonce).
    /// A consistent honest manifest sets `binding.challenge == nonce`.
    // [OPUS-4.8] audit #4: nonce/binding consistency, fail-closed.
    NonceBindingMismatch,
    /// A scan-covering attestation carries NO issuer-bound status reference
    /// (`status: None`) — fail-closed for audit #12. A status-unbound attestation
    /// does NOT bind the credential's revocation reference into the issuer
    /// signature, so accepting one for a scan-covering commitment would let a
    /// prover OMIT the status reference and present a revoked credential
    /// unchecked. Every attestation covering a verified scan commitment MUST bind
    /// a status reference and verify via the status-bound message.
    // [OPUS-4.8] audit #12: status-bound attestation is mandatory for scans.
    ScanCommitmentStatusMissing { proof: usize, commitment: String },
    /// The manifest carries an issuer-bound status reference for a scan
    /// commitment but NO `manifest.revocation` to recompute the signed digest
    /// from (audit #12): the prover dropped the disclosed reference. The
    /// status-bound issuer signature cannot be checked without it, so this is
    /// rejected (fail-closed — the omit-the-field bypass).
    // [OPUS-4.8] audit #12.
    RevocationReferenceMissing { proof: usize },
    /// The disclosed `manifest.revocation` (list id / index / version) does not
    /// match the issuer-signed status reference for a scan commitment (audit
    /// #12): the prover disclosed a different reference than the issuer signed
    /// (e.g. pointing at another list/index/version whose bit is unset). Detected
    /// because the recomputed status digest then fails the signature check, OR
    /// because the disclosed index/version differs from the attestation's signed
    /// `AttestedStatusRef`.
    // [OPUS-4.8] audit #12.
    RevocationReferenceMismatch { commitment: String },
    /// [OPUS-4.8] sq-ayv: the status reference's index-disclosure MODE is malformed
    /// — neither a clear `index` nor an `index_commitment` is present, or BOTH are
    /// (the attestation's signed reference and the disclosed reference must each be
    /// EXACTLY one of clear-index or committed-index, and the two must agree on the
    /// mode). Rejected fail-closed: an ambiguous reference mode could let a prover
    /// recompute the signed digest one way while the liveness check reads the other.
    RevocationReferenceModeInvalid { commitment: String },
    /// [OPUS-4.8] sq-ayv: a COMMITTED-index revocation reference (clear index
    /// withheld) was disclosed but NO `manifest.hidden_revocation` proof is present
    /// to check liveness against the authoritative root. The committed-index path
    /// MOVES the liveness decision to the hidden-index proof; without it the
    /// credential's liveness is unchecked, so this is rejected fail-closed
    /// (revocation is NEVER skipped — either the clear-index bit check or the
    /// hidden-index proof must run).
    HiddenRevocationRequired { proof: usize },
    /// [OPUS-4.8] sq-ayv: a `manifest.hidden_revocation` proof's PUBLIC index
    /// commitment does NOT equal the ISSUER-SIGNED index commitment in
    /// `manifest.revocation.index_commitment` (the cross-binding): the index proven
    /// unset is not provably the index the issuer committed to. Rejected — a holder
    /// could otherwise sign over a commitment to its REVOKED index and prove
    /// bit-unset for a different (active) index.
    HiddenRevocationIndexCommitmentMismatch,
    /// The relying party has NO AUTHORITATIVE status-list snapshot for the
    /// credential's (issuer-bound) revocation reference `(status_list, version)`
    /// (audit #12 / re-audit Option B): the verifier cannot AUTHENTICATE the
    /// credential's liveness view, so it rejects (fail-closed). The bit decision
    /// is sourced from the relying party's [`RevocationPolicy`] authoritative
    /// store — NOT the prover's `manifest.status_snapshots` — so an absent
    /// authoritative snapshot is the verifier's own missing trust input, never a
    /// prover-skippable field.
    // [OPUS-4.8] audit #12 / re-audit: missing AUTHORITATIVE (relying-party) snapshot.
    StatusSnapshotMissing { status_list: String, version: u64 },
    /// The prover disclosed a status-list snapshot for the referenced
    /// `(status_list, version)` whose bits DISAGREE with the relying party's
    /// AUTHORITATIVE snapshot (audit #12 re-audit, Option B): a tamper signal. The
    /// liveness bit decision does NOT depend on the prover's snapshot (it always
    /// reads the authoritative bytes), but a disagreeing prover snapshot — e.g. a
    /// FORGED all-zero bitstring presented alongside a genuine reference for a
    /// REVOKED credential — is surfaced explicitly as a forgery attempt rather
    /// than silently ignored.
    // [OPUS-4.8] audit #12 re-audit: prover snapshot ≠ authoritative snapshot.
    StatusSnapshotTampered { status_list: String, version: u64 },
    /// The credential's status bit is SET in the disclosed snapshot (audit #12):
    /// the credential is REVOKED / SUSPENDED. Rejected.
    // [OPUS-4.8] audit #12.
    CredentialRevoked { status_list: String, index: u64 },
    /// The disclosed status-list snapshot's version is OUTSIDE the verifier's
    /// freshness window (audit #12): the snapshot is STALE (or implausibly
    /// future-dated). A relying party will not trust a revocation view it cannot
    /// vouch is current, so a stale snapshot is rejected (a revoked-since-snapshot
    /// credential must not slip through on an old "active" view).
    // [OPUS-4.8] audit #12.
    StatusListStale { status_list: String, version: u64 },
    /// A `manifest.hidden_revocation` proof was presented but the relying party
    /// has NOT enabled the hidden-index path (no `hidden_index_depth` in the
    /// policy) (sq-3e5 / sq-h2v): the verifier cannot derive an authoritative
    /// Merkle root to bind the proof to, so it rejects fail-closed rather than
    /// accepting a root it did not itself compute.
    // [OPUS-4.8] sq-3e5 + sq-h2v.
    HiddenRevocationNotEnabled,
    /// The `manifest.hidden_revocation` declared a Merkle depth that does NOT
    /// match the relying party's policy depth (sq-3e5 / sq-h2v): the trees (and
    /// roots) would be over different leaf layouts, so the proof cannot be bound
    /// to the verifier's authoritative root. Rejected.
    // [OPUS-4.8] sq-3e5 + sq-h2v.
    HiddenRevocationDepthMismatch { declared: u32, policy: u32 },
    /// The relying party could not derive an authoritative Merkle root for the
    /// hidden-index revocation proof (sq-3e5 / sq-h2v): there is no authoritative
    /// snapshot for the referenced `(list, version)`, or the depth is implausible.
    /// Fail-closed — the verifier will not vouch for a liveness view it cannot
    /// itself anchor.
    // [OPUS-4.8] sq-3e5 + sq-h2v.
    HiddenRevocationRootUnavailable { status_list: String, version: u64 },
    /// The `manifest.hidden_revocation` proof's PUBLIC Merkle root does NOT equal
    /// the root the relying party derived from its OWN authoritative snapshot
    /// (sq-3e5 / sq-h2v): the proof was produced against a different (e.g.
    /// prover-forged all-zero) status list. Rejected — the liveness fact is not
    /// bound to the relying party's authenticated status data.
    // [OPUS-4.8] sq-3e5 + sq-h2v.
    HiddenRevocationRootMismatch,
    /// bb rejected the hidden-index revocation proof (sq-3e5 / sq-h2v): the
    /// zero-knowledge bit-unset/inclusion statement did not verify against the
    /// canonical `revoke_unset_d{depth}` vk and the reconstructed public inputs.
    // [OPUS-4.8] sq-3e5 + sq-h2v.
    HiddenRevocationProofRejected,
    /// The `manifest.hidden_revocation` proof blob is malformed (non-hex /
    /// truncated length prefix) (sq-3e5 / sq-h2v) — rejected before any bb call
    /// (audit hardening; prover-controlled bytes never panic).
    // [OPUS-4.8] sq-3e5 + sq-h2v.
    HiddenRevocationMalformedProof,
    /// [OPUS-5] sq-kndw: the revocation reference is in the FULLY-HIDDEN mode
    /// (`ref_commitment` present, no clear IRI / index / version) but the manifest
    /// carries NO `fully_hidden_revocation` proof. That mode moves the entire
    /// liveness decision into the proof, so accepting without it would leave
    /// revocation UNCHECKED — rejected (never skip revocation).
    FullyHiddenRevocationRequired,
    /// [OPUS-5] sq-kndw: a `fully_hidden_revocation` proof was presented WITHOUT a
    /// fully-hidden revocation reference. There are then no ISSUER-SIGNED
    /// `(ref_commitment, index_commitment)` values to cross-bind the proof to, so it
    /// would be a free-floating liveness claim over an unbound credential. Rejected.
    FullyHiddenRevocationUnbound,
    /// [OPUS-5] sq-kndw: a FULLY-HIDDEN presentation ALSO attached a
    /// `status_snapshots` entry. A snapshot names its `(status_list, version)`, so
    /// attaching one discloses in the clear exactly what the mode exists to hide —
    /// self-defeating, and never needed (the fully-hidden gate reads the relying
    /// party's own curated snapshots, never the prover's). Rejected so a buggy
    /// holder implementation cannot silently leak the credential's list + epoch.
    FullyHiddenRevocationSnapshotDisclosed,
    /// [OPUS-5] sq-kndw: a `fully_hidden_revocation` proof was presented but the
    /// relying party has NOT enabled the fully-hidden path (no accepted-set depth /
    /// hidden-index depth on the [`RevocationPolicy`], or the curated accepted set
    /// does not fit the configured depth). Without an accepted-set anchor there is
    /// no root to bind the proof to — rejected fail-closed.
    FullyHiddenRevocationNotEnabled,
    /// [OPUS-5] sq-kndw: the proof's declared `(depth, set_depth)` do not match the
    /// relying party's policy depths, or name no COMPILED
    /// `revoke_hidden_ref_d{depth}_a{set_depth}` member
    /// ([`crate::build::derive_revoke_hidden_ref_id`]). The trees would be over
    /// different layouts, so the proof cannot be bound to the verifier's anchors.
    FullyHiddenRevocationDepthMismatch {
        declared_depth: u32,
        declared_set_depth: u32,
        policy_depth: u32,
        policy_set_depth: u32,
    },
    /// [OPUS-5] sq-kndw: the proof's declared `accepted_set_root` or `min_version`
    /// does not equal the value the relying party derives from its OWN
    /// freshness-curated policy. The prover does not get to choose the trust anchor
    /// or the epoch floor.
    FullyHiddenRevocationAnchorMismatch,
    /// [OPUS-5] sq-kndw: the proof's declared `ref_commitment` / `index_commitment`
    /// do not byte-equal the ISSUER-SIGNED ones in `manifest.revocation`. The
    /// cross-binding that ties the in-circuit private `(list, version)` and index to
    /// the issuer's reference is broken — rejected.
    FullyHiddenRevocationCommitmentMismatch,
    /// [OPUS-5] sq-kndw: the `(ref_commitment, index_commitment)` pair has been
    /// presented BEFORE. The pair is a stable per-issuance handle, so re-presenting
    /// it is exactly the cross-presentation linkage the fully-hidden mode exists to
    /// prevent (`research/zk-statuslist-hide-iri-version.md` §4). Rejected so the
    /// holder is forced to re-blind. Honest limit: this protects the holder only
    /// against an HONEST relying party.
    FullyHiddenRevocationLinkageReplay,
    /// [OPUS-5] sq-kndw: the fully-hidden revocation proof did not verify — either
    /// bb rejected the zero-knowledge ref-open / accepted-set-membership /
    /// freshness / bit-unset statement against the canonical
    /// `revoke_hidden_ref_d{depth}_a{set_depth}` vk and the reconstructed public
    /// inputs, or the proof commits a DIFFERENT challenge than this verifier's
    /// nonce (a proof minted for another session — caught before the bb call by
    /// the public-input byte-compare).
    FullyHiddenRevocationProofRejected,
    /// [OPUS-5] sq-kndw: the `manifest.fully_hidden_revocation` proof blob is
    /// malformed (non-hex / truncated length prefix), or a declared field is not a
    /// parseable field element — rejected before any bb call (prover-controlled
    /// bytes never panic).
    FullyHiddenRevocationMalformedProof,
    /// A `manifest.hidden_issuer_attestations` entry was presented but the relying
    /// party has NOT enabled the hidden-issuer path (no `hidden_issuer_depth` in
    /// the KeySet) (sq-z9l): the verifier cannot derive an authoritative key-set
    /// Merkle root to bind the proof to, so it rejects fail-closed.
    // [OPUS-4.8] sq-z9l.
    HiddenIssuerNotEnabled,
    /// A `manifest.hidden_issuer_attestations` entry declared a Merkle depth that
    /// does NOT match the KeySet policy depth (sq-z9l): the trees (and roots) would
    /// be over different layouts, so the proof cannot be bound to the verifier's
    /// authoritative root. Rejected.
    // [OPUS-4.8] sq-z9l.
    HiddenIssuerDepthMismatch { declared: u32, policy: u32 },
    /// The relying party could not derive an authoritative key-set Merkle root for
    /// the hidden-issuer proof (sq-z9l): the trusted set overflows the tree at this
    /// depth, or the depth is implausible. Fail-closed.
    // [OPUS-4.8] sq-z9l.
    HiddenIssuerRootUnavailable,
    /// The `manifest.hidden_issuer_attestations` proof's PUBLIC key-set Merkle root
    /// does NOT equal the root the relying party derived from its OWN authoritative
    /// KeySet (sq-z9l): the proof was produced against a different (e.g.
    /// prover-chosen) key set. Rejected — the "in K" fact is not anchored on the
    /// relying party's trust.
    // [OPUS-4.8] sq-z9l.
    HiddenIssuerRootMismatch,
    /// The `manifest.hidden_issuer_attestations` proof's PUBLIC message `m` does
    /// NOT equal the issuer-signed message the verifier recomputed from the
    /// disclosed commitment + salt + status reference (sq-z9l): the hidden-issuer
    /// proof is not bound to this committed graph. Rejected.
    // [OPUS-4.8] sq-z9l.
    HiddenIssuerMessageMismatch { commitment: String },
    /// A `manifest.hidden_issuer_attestations` entry covers a commitment that no
    /// verified scan sub-proof references (sq-z9l): a dangling hidden attestation.
    /// Rejected (every hidden attestation must cover a scan-referenced commitment,
    /// mirroring the clear-key path's UnattestedCommitment discipline in reverse).
    // [OPUS-4.8] sq-z9l.
    HiddenIssuerUnreferencedCommitment { commitment: String },
    /// `bb verify` REJECTED the hidden-issuer proof against the canonical
    /// `hidden_issuer_d{depth}` vk + reconstructed public inputs (sq-z9l): the
    /// signature is invalid, the key is not in K, or the public inputs were
    /// tampered. Rejected.
    // [OPUS-4.8] sq-z9l.
    HiddenIssuerProofRejected,
    /// The `manifest.hidden_issuer_attestations` proof blob is malformed (non-hex /
    /// truncated length prefix) (sq-z9l) — rejected before any bb call.
    // [OPUS-4.8] sq-z9l.
    HiddenIssuerMalformedProof,
    /// [OPUS-4.8] sq-c2ql (HolderPoP T6 / B2): a [`crate::manifest::HolderPokProof`]
    /// covers a commitment that no verified scan sub-proof references — a dangling
    /// in-circuit holder PoK. Rejected fail-closed (every PoK must cover a
    /// scan-referenced credential, mirroring the hidden-issuer
    /// [`Self::HiddenIssuerUnreferencedCommitment`] discipline).
    HolderPokUnreferencedCommitment { commitment: String },
    /// [OPUS-4.8] sq-c2ql (B2): a [`crate::manifest::HolderPokProof`] covers a
    /// scan-referenced commitment whose COVERING issuer attestation carries NO
    /// holder binding ([`crate::manifest::AttestedHolderBinding`]). The in-circuit
    /// PoK has no issuer-attested `holder_pk_digest` to bind to — there is nothing
    /// for the binding edge to anchor on, so it is rejected fail-closed.
    HolderPokBindingMissing { commitment: String },
    /// [OPUS-4.8] sq-c2ql (B2): the relying party requires an in-circuit holder PoK
    /// ([`HolderBindingPolicy::require_in_circuit_pok`]) for a holder-bound
    /// credential, but the manifest carries NO matching
    /// [`crate::manifest::HolderPokProof`] for that credential's commitment.
    /// Rejected fail-closed — the hidden-key possession proof is mandated, never
    /// silently waived.
    HolderPokMissing { commitment: String },
    /// [OPUS-4.8] sq-c2ql (B2): a [`crate::manifest::HolderPokProof`]'s PUBLIC
    /// `holder_pk_digest` does NOT equal the ISSUER-ATTESTED digest the verifier
    /// recovered from the credential's [`crate::manifest::AttestedHolderBinding`]
    /// (signature-anchored under the external `K`). This is the binding edge: the
    /// proven holder key is not the one the issuer signed into THIS credential.
    /// Rejected fail-closed.
    HolderPokDigestMismatch { commitment: String },
    /// [OPUS-4.8] sq-c2ql (B2): `bb verify` REJECTED the in-circuit holder PoK
    /// against the canonical `holder_pok` vk + the reconstructed public inputs
    /// (verifier nonce + issuer-attested digest) — the prover does not know a
    /// holder secret whose public key hashes to the issuer-attested digest, or the
    /// public inputs were tampered. Rejected.
    HolderPokProofRejected { commitment: String },
    /// [OPUS-4.8] sq-c2ql (B2): a [`crate::manifest::HolderPokProof`] blob is
    /// malformed (non-hex / truncated length prefix), or its declared commitment /
    /// the recovered digest is not a field element — rejected before any bb call.
    HolderPokMalformedProof,
    /// [OPUS-4.8] sq-3c00 (hidden-holder-SET tier): a
    /// [`crate::manifest::HolderSetProof`] was presented but the relying party has
    /// NOT enabled the hidden-holder-set path (no
    /// [`HolderRegistry::with_hidden_holder_set_depth`]): the verifier cannot derive
    /// an authoritative holder-set root to bind the proof to. Rejected fail-closed.
    HolderSetNotEnabled,
    /// [OPUS-4.8] sq-3c00: a [`crate::manifest::HolderSetProof`] declared a Merkle
    /// depth that does NOT match the registry policy depth: the trees (and roots)
    /// would be over different leaf layouts. Rejected fail-closed.
    HolderSetDepthMismatch { declared: u32, policy: u32 },
    /// [OPUS-4.8] sq-3c00: the relying party enabled the hidden-holder-set path but
    /// the authoritative holder-set root could not be derived (the registry
    /// overflows the tree at the policy depth, the depth is implausible, or a
    /// holder key is the identity). Rejected fail-closed.
    HolderSetRootUnavailable,
    /// [OPUS-4.8] sq-3c00: a [`crate::manifest::HolderSetProof`]'s PUBLIC
    /// `holder_set_root` does NOT byte-equal the root the verifier derives from its
    /// OWN authoritative [`HolderRegistry`]: the proof was produced against a
    /// different (e.g. prover-forged) holder set. Rejected fail-closed (the trust
    /// anchor, mirroring [`Self::HiddenIssuerRootMismatch`]).
    HolderSetRootMismatch,
    /// [OPUS-4.8] sq-3c00: a [`crate::manifest::HolderSetProof`] covers a commitment
    /// that no verified scan sub-proof references — a dangling set-membership proof.
    /// Rejected fail-closed (mirrors [`Self::HolderPokUnreferencedCommitment`]).
    HolderSetUnreferencedCommitment { commitment: String },
    /// [OPUS-4.8] sq-3c00: `bb verify` REJECTED the hidden-holder set-membership
    /// proof against the canonical `holder_set_d{depth}` vk + the reconstructed
    /// public inputs (verifier nonce + authoritative root) — the prover does not
    /// know a holder secret whose key digest is a member of the committed set, or
    /// the public inputs were tampered. Rejected.
    HolderSetProofRejected { commitment: String },
    /// [OPUS-4.8] sq-3c00: a [`crate::manifest::HolderSetProof`] blob is malformed
    /// (non-hex / truncated length prefix), or its declared commitment / root is not
    /// a field element — rejected before any bb call.
    HolderSetMalformedProof,
    /// The manifest's binding is `HolderPop` but the relying party supplied NO
    /// holder registry (an empty [`HolderRegistry`]) (sq-cwq): the verifier has no
    /// trust anchor to check the holder key against, so it cannot accept a holder
    /// PoP. Rejected fail-closed — a HolderPop binding without a registry is NEVER
    /// silently accepted as a bare challenge (the previous placeholder behaviour).
    // [OPUS-4.8] sq-cwq.
    HolderRegistryEmpty,
    /// The `HolderPop` binding's `holder` key is not a member of the relying
    /// party's external [`HolderRegistry`] (sq-cwq): the presenter is not an
    /// authorised holder. Rejected.
    // [OPUS-4.8] sq-cwq.
    HolderNotTrusted { holder: String },
    /// The `HolderPop` binding's `cryptosuite` is unknown/unsupported, or its
    /// `holder` key / `pop` signature did not parse (sq-cwq): the PoP is
    /// unverifiable. Rejected fail-closed (prover-controlled bytes never panic).
    // [OPUS-4.8] sq-cwq.
    HolderPopMalformed,
    /// The `HolderPop` binding's `pop` signature did not verify under the `holder`
    /// key over the challenge-bound PoP message (sq-cwq): the presenter did NOT
    /// prove possession of the holder secret (a forged/replayed/absent PoP).
    /// Rejected.
    // [OPUS-4.8] sq-cwq.
    HolderPopInvalid { holder: String },
    /// The relying party REQUIRES an issuer-attested holder binding (T3/sq-z8s7
    /// B1, [`HolderBindingPolicy::require_binding`]) but the `HolderPop`
    /// presentation's credential carries NO [`crate::manifest::AttestedHolderBinding`]
    /// — i.e. a BEARER credential (no `holder` field on any
    /// [`crate::manifest::CommitmentAttestation`]) presented where a per-credential
    /// holder binding is mandated. Rejected fail-closed — there is NO silent bearer
    /// fallback (design `research/zk-holder-pop-design.md` §4.3 obligation 3, the
    /// audit-#12 `status:None ⇒ reject` precedent). Closes the trusted-holder gap's
    /// "present a bearer credential and skip the binding" bypass.
    // [OPUS-4.8] sq-z8s7 (HolderPoP T3 / B1): fail-closed bearer rejection.
    HolderBindingMissing,
    /// The PRESENTED holder key (the `HolderPop` binding's disclosed key, also the
    /// key the freshness PoP was signed under) does NOT hash (via
    /// [`sparq_zk::sig::holder_key_digest`]) to the issuer-attested
    /// [`crate::manifest::AttestedHolderBinding::holder_pk_digest`] (T3/sq-z8s7 B1):
    /// the presenter is binding a DIFFERENT key than the issuer signed into THIS
    /// credential. This is the load-bearing trusted-holder-gap closure — it rejects
    /// "trusted holder A presents trusted holder B's credential" (A's key digest ≠
    /// B's attested digest). Also covers the identity holder key (no usable digest,
    /// [`sparq_zk::sig::HolderKeyError::IdentityKey`]) and a clear holder key in the
    /// attestation that disagrees with the presented key. Rejected fail-closed.
    // [OPUS-4.8] sq-z8s7 (HolderPoP T3 / B1): presented-key vs attested-digest gate.
    HolderKeyMismatch,
    /// The manifest's `entailment_regime` is NOT accepted by the relying party's
    /// [`EntailmentPolicy`] (sq-314): e.g. an `Rdfs`/`Owl` manifest under a
    /// `Simple`-only policy. Rejected fail-closed — the regime is enforced, not
    /// free metadata (a relying party opts into inference explicitly).
    // [OPUS-4.8] sq-314.
    EntailmentRegimeNotAccepted { regime: &'static str },
    /// A `Simple` manifest carried derivation steps (sq-314): `Simple` means NO
    /// inference, so any `derivation_steps` are inconsistent with the declared
    /// regime. Rejected.
    // [OPUS-4.8] sq-314.
    UnexpectedDerivationSteps,
    /// A non-`Simple` manifest carried NO derivation steps (sq-314): an inference
    /// regime with nothing to justify the inference is a vacuous claim. Rejected
    /// fail-closed (a relying party that accepts `Rdfs`/`Owl` requires the
    /// derivation to be recorded + re-checkable).
    // [OPUS-4.8] sq-314.
    MissingDerivationSteps { regime: &'static str },
    /// A derivation step is not a well-formed instance of its rule, or its rule is
    /// not admitted by the declared regime (sq-314): the recorded inference does
    /// not match the rule shape / regime. Rejected.
    // [OPUS-4.8] sq-314.
    MalformedDerivationStep { step: usize },
    /// A derivation step's antecedent is UNGROUNDED (sq-314): it is neither an
    /// earlier step's derived triple nor a triple disclosed by a scan sub-proof
    /// (the asserted base). A derived triple cannot rest on an antecedent the proof
    /// does not establish, so this is rejected fail-closed. (The in-circuit closure
    /// proof that would ground antecedents not disclosed is deferred — see the
    /// `derivation` module; until then only the disclosed base grounds a step.)
    // [OPUS-4.8] sq-314.
    UngroundedDerivationAntecedent { step: usize, antecedent: usize },
    /// The relying party requires COMPLETENESS under entailment
    /// ([`EntailmentPolicy::require_completeness_under_entailment`]) but the
    /// manifest declares a non-`Simple` regime, and that property is **UNBUILT in
    /// sparq and NOT claimed** (`sq-rsd3v.7`): both halves of
    /// [`crate::derivation::COMPLETENESS_UNDER_ENTAILMENT_UNBUILT`] would be needed
    /// and the fixpoint-saturation half exists nowhere in the estate. Refused
    /// fail-closed — a soundness-of-derivation accept must never be handed to a
    /// relying party that asked for completeness (the conflation the design's §3.7
    /// forbids). This is a CAPABILITY refusal, not a defect in the manifest.
    // [OPUS-5] sq-rsd3v.7: enforced deferral of completeness-under-entailment.
    CompletenessUnderEntailmentUnavailable { regime: &'static str },
    /// A derivation step introduces or consumes an `owl:sameAs` fact
    /// (sq-rsd3v.6): the `owl:sameAs` encoding stands in a predicate slot of an
    /// antecedent or of the derived triple. The fixed-shape RDFS / OWL-RL-minus-
    /// sameAs path re-checks rules by term-encoding equality, which is a term
    /// IDENTITY test and is therefore the WRONG proxy under equality reasoning
    /// (`owl:sameAs` quotients the term universe). Equality reasoning needs the
    /// in-circuit union-find canonicalisation of the [`crate::sameas`] module,
    /// which is a SEPARATE, not-yet-composable member — so such a step is
    /// refused fail-closed rather than allowed to ride this path silently. See
    /// [`crate::derivation::DerivationStep::mentions_equality_predicate`].
    // [OPUS-5] sq-rsd3v.6 (#3265).
    EqualityReasoningUnsupported { step: usize },
    /// [OPUS-4.8] sq-sfsi (hidden JOIN, step 4): a `JoinEdge` references a
    /// non-existent sub-proof index (`scan_a`/`scan_b`/`join_proof`) or a
    /// committed-graph index (`graph_a`/`graph_b`) outside the referenced scan's
    /// `commitments`. The hidden-key analogue of [`Self::DanglingEdge`]. Rejected
    /// fail-closed — a join cannot bind a proof/graph the manifest does not carry.
    JoinDanglingEdge { edge: usize },
    /// [OPUS-4.8] sq-sfsi (hidden JOIN, step 4): a `JoinEdge`'s `scan_a`/`scan_b`
    /// do not point at `Scan` sub-proofs, or `join_proof` does not point at a
    /// `JoinEq` sub-proof. The hidden-key analogue of [`Self::EdgeKindMismatch`]:
    /// a join edge must tie two scans to a `join_eq` proof. Rejected fail-closed.
    JoinEdgeKindMismatch { edge: usize },
    /// [OPUS-4.8] sq-sfsi (hidden JOIN, step 4): the `join_eq` proof's PUBLIC
    /// `commit_a`/`commit_b` do NOT byte-equal the two referenced scans' bound
    /// `commitments[graph_a]`/`commitments[graph_b]` (design §2.3 / §4.2, the
    /// anti-A2 binding). The scan commitments are audit-#1 byte-bound into the scan
    /// proofs AND issuer-signed (audit #3), so this ties the join to two genuine,
    /// attested credentials. A `join_eq` pointed at a graph the scans do not attest
    /// (cross-scan forgery) is rejected here fail-closed.
    JoinCommitmentMismatch { edge: usize },
    /// [OPUS-4.8] sq-sfsi (hidden JOIN, step 4): the `join_eq` proof's PUBLIC
    /// `slot_a`/`slot_b` do NOT equal the query-derived slots the shared join
    /// variable occupies in the two patterns the referenced scans answer (design
    /// §4.4 slot binding — the one genuinely new soundness obligation). A prover
    /// that proved the equality over the wrong column (the salary-slot-for-age
    /// analogue, audit #6) is rejected fail-closed.
    JoinSlotMismatch { edge: usize },
    /// [OPUS-4.8] sq-r2s8 (hidden JOIN, N-way chain — design §2.4): two declared
    /// `JoinEdge`s join the SAME query variable (a multi-hop / N-way join chain),
    /// but their `join_eq` sub-proofs carry DIFFERENT `join_commitment`s. The N-way
    /// composition is sound only when every pairwise `join_eq` over the chained
    /// variable binds the SAME hiding commitment (the prover uses one blinder), so
    /// equality of the join VALUE composes transitively across the chain
    /// (`a_val == b_val` per hop + a shared commitment). Distinct commitments mean
    /// the hops proved equalities over potentially DIFFERENT values, so the claimed
    /// N-way join is unproven — rejected fail-closed. `edge` is the first chained
    /// edge whose commitment diverges from the chain's first edge.
    JoinCommitmentChainMismatch { edge: usize },
    /// Subprocess / io failure (not a verification verdict).
    Driver(DriverError),
    /// [OPUS-4.8] sq-h732x: the FAIL-CLOSED extended-fragment structural routing
    /// gate ([`dispatch_fragment`]) refused the presentation — the query is
    /// outside the wave-1 fragment, or a disclosed solution's branch witness does
    /// not route to a bound sub-proof of the correct circuit member. Carried into
    /// [`verify_fragment_manifest`] so an extended-fragment refusal surfaces as one
    /// structured error alongside the stage-1 gates. Opt-in (`extended-fragment`).
    #[cfg(feature = "extended-fragment")]
    FragmentDispatch(FragmentDispatchError),
    /// [OPUS-4.8] sq-1zf94: the FAIL-CLOSED extended-fragment DISCLOSED-SOLUTION
    /// term binding ([`bind_fragment_solution`]) refused the presentation — a
    /// `PathReach` `pred_enc`/`src_enc`/`dst_enc` or a `VALUES` cell does not equal
    /// the encoding the verifier re-derives from the disclosed solution + query
    /// text (never the manifest's encodings). Carried into
    /// [`verify_fragment_manifest`]. Opt-in (`extended-fragment`).
    #[cfg(feature = "extended-fragment")]
    FragmentSolution(FragmentSolutionError),
    /// [OPUS-4.8] sq-qyfth: the FAIL-CLOSED extended-fragment BGP SCAN-SLOT binding
    /// ([`bind_fragment_scans`]) refused the presentation — a BGP scan sub-proof
    /// does not answer the query pattern, no supporting row is selected for a scan
    /// pattern carrying a variable, the selected row is out of range, a selected
    /// row slot does not equal the disclosed solution's re-encoded term for a
    /// projected variable, or two atoms sharing a variable selected rows whose slot
    /// values disagree (join incoherence). Carried into
    /// [`verify_fragment_manifest`]. Opt-in (`extended-fragment`).
    #[cfg(feature = "extended-fragment")]
    FragmentScan(FragmentScanError),
    /// [OPUS-4.8] sq-ygk6x: the FAIL-CLOSED extended-fragment PER-BRANCH JOIN
    /// COHERENCE + cross-graph Q6 non-bnode gate ([`bind_fragment_join_coherence`])
    /// refused the presentation — an existential variable shared between two branch
    /// atoms (scan/scan, scan/path, or path/path) resolves to disagreeing disclosed
    /// values, a `PathReach` whose proof-bound attribution admits more than one
    /// committed graph (whose interior-chain non-bnode obligation the verifier cannot
    /// discharge from disclosed data), or a required cross-graph join obligation not
    /// covered by the disclosed data. Carried into [`verify_fragment_manifest`].
    /// Opt-in (`extended-fragment`).
    #[cfg(feature = "extended-fragment")]
    FragmentJoin(FragmentJoinError),
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckError::Sparqzk(e) => write!(f, "sparq-zk re-check failed: {e}"),
            CheckError::CircuitIdMismatch { proof, declared, derived } => write!(
                f,
                "sub-proof {proof}: declared circuit id {declared:?} but inputs derive {derived:?}"
            ),
            CheckError::DanglingEdge { edge } => {
                write!(f, "binding edge {edge} references a missing proof/row/slot")
            }
            CheckError::EdgeKindMismatch { edge } => {
                write!(f, "binding edge {edge} connects incompatible proof kinds")
            }
            CheckError::BindingInconsistent { edge } => write!(
                f,
                "binding edge {edge}: scanned column does not equal the filter operand"
            ),
            CheckError::ProofRejected { proof } => write!(f, "bb rejected sub-proof {proof}"),
            CheckError::MissingProof { proof } => {
                write!(f, "sub-proof {proof} has no proof bytes")
            }
            CheckError::MalformedProof { proof } => {
                write!(f, "sub-proof {proof} proof_hex blob is malformed")
            }
            CheckError::MalformedField { proof, what } => {
                write!(f, "sub-proof {proof}: declared field `{what}` is not a valid field element")
            }
            CheckError::PublicInputMismatch { proof } => write!(
                f,
                "sub-proof {proof}: reconstructed public inputs do not match the proof's public inputs (declared statement is not the proved statement)"
            ),
            CheckError::UnboundPattern { pattern } => write!(
                f,
                "query BGP pattern {pattern} has no scan sub-proof binding its constant slots (constant-swap / unproven pattern)"
            ),
            CheckError::UnboundFilter { variable } => write!(
                f,
                "query FILTER on ?{variable} has no matching slot-bound, true-verdict filter_int sub-proof (FILTER-add / comparison- or operand-substitution / false-verdict row)"
            ),
            CheckError::UnmappableFilterVar { variable } => write!(
                f,
                "query FILTER on ?{variable} does not bind to any scanned column"
            ),
            CheckError::PatternScanArityMismatch { patterns, declared } => write!(
                f,
                "manifest.pattern_scans declares {declared} entries for {patterns} query BGP patterns (the pattern→scan mapping is indexed per query pattern, like attributions)"
            ),
            CheckError::PatternScanUnbound { pattern } => write!(
                f,
                "manifest.pattern_scans[{pattern}] is empty: query BGP pattern {pattern} is declared to be answered by no scan sub-proof"
            ),
            CheckError::PatternScanMismatch { pattern, proof } => write!(
                f,
                "manifest.pattern_scans[{pattern}] names sub-proof {proof}, which is out of range, is not a scan, or whose bound pattern constants do not answer query BGP pattern {pattern} (audit #10: a declaration must not contradict the proof-bound constants)"
            ),
            CheckError::PatternScanUndeclared { proof } => write!(
                f,
                "scan sub-proof {proof} is named by no entry of the declared manifest.pattern_scans: the manifest discloses its rows but its own declared reading gives them no query BGP pattern (dangling scan)"
            ),
            CheckError::RepeatedSlotMismatch { pattern, proof, row, variable, slots } => write!(
                f,
                "query BGP pattern {pattern} binds ?{variable} at slots {} and {}, but disclosed row {row} of scan sub-proof {proof} (which answers that pattern) gives them different terms: the row does not satisfy the pattern it is disclosed under",
                slots.0, slots.1
            ),
            CheckError::UnattestedCommitment { proof, commitment } => write!(
                f,
                "sub-proof {proof}: commitment {commitment} has no issuer attestation (unsigned / prover-invented commitment — no credential provenance); applies to a BGP scan or an extended-fragment path sub-proof alike"
            ),
            CheckError::InvalidIssuerSignature { commitment } => write!(
                f,
                "commitment {commitment}: issuer signature does not verify under its declared key (forged/tampered signature or wrong commitment)"
            ),
            CheckError::IssuerKeyNotInKeySet { commitment } => write!(
                f,
                "commitment {commitment}: attestation key is not a member of the external trusted key-set K (untrusted issuer)"
            ),
            CheckError::UntrustedDeclaredKey { key } => write!(
                f,
                "manifest key_set declares key {key} which is not in the external trusted key-set K (prover may not widen the trust anchor)"
            ),
            CheckError::AttestationKeyNotInDeclaredSet { commitment, key } => write!(
                f,
                "commitment {commitment}: attestation key {key} is not in the prover's declared (non-empty) manifest key_set (declared narrowing is inconsistent with the proven attestations)"
            ),
            CheckError::AttributionUnderDeclared { pattern, proof_graph } => write!(
                f,
                "query BGP pattern {pattern}: manifest.attributions omits graph {proof_graph}, which the scan proof's in-circuit attribution shows it drew a matched triple from (under-declared attribution — the [[0],[0]] collapse-two-graphs forge that would drop a cross-graph bnode obligation)"
            ),
            CheckError::AttributionUnbound { pattern } => write!(
                f,
                "query BGP pattern {pattern} has no scan sub-proof carrying a proof-bound attribution to cross-check"
            ),
            CheckError::SaltReused { salt } => write!(
                f,
                "salt {salt} is reused across two distinct committed graphs (audit #9: cross-graph bnode-correlation channel — each graph needs a globally-unique issuer-attested salt)"
            ),
            CheckError::ScanCommitmentSaltMissing { proof, commitment } => write!(
                f,
                "sub-proof {proof}: the attestation covering commitment {commitment} carries no salt (audit #9 / codex 2221 HIGH: a scan- or path-covering attestation MUST be salt-bound — a salt-less legacy attestation bypasses salt-separation)"
            ),
            CheckError::AttributionMalformed { proof, expected, got } => write!(
                f,
                "scan sub-proof {proof}: attribution must be present and exactly {expected} bits (CircuitId.k), got {got} (audit #8 / codex 2221 MEDIUM: an omitted/short attribution makes the cross-graph under-declaration check vacuous)"
            ),
            CheckError::ScanCommitmentsNotStrictlyIncreasing { proof, at } => write!(
                f,
                "scan sub-proof {proof}: per-graph commitments are not strictly increasing at index {at} (commitments[{at}] <= commitments[{at_prev}]) (plan S2.5 / sq-vxq8: distinct-graph strict ordering — a duplicate or out-of-order commitment is the duplicate-inclusion / COUNT forgery, e.g. the same credential included twice to claim 'I hold >=2 tickets')",
                // [OPUS-4.8] `Display` must be panic-free for every constructible value
                // (Copilot PR #95): the sole constructor sets `at` from `1..len()`, so
                // `at >= 1` in practice, but `saturating_sub` avoids an `at - 1` underflow
                // (debug panic / release wrap) should `at == 0` ever reach here.
                at_prev = at.saturating_sub(1)
            ),
            CheckError::NonceReplay => write!(
                f,
                "verifier nonce already seen (audit #4: single-use — a captured (nonce, manifest) pair may not be replayed)"
            ),
            CheckError::NonceBindingMismatch => write!(
                f,
                "manifest binding challenge does not equal the verifier-issued nonce (audit #4: the manifest was minted for a different nonce/session)"
            ),
            CheckError::ScanCommitmentStatusMissing { proof, commitment } => write!(
                f,
                "sub-proof {proof}: the attestation covering commitment {commitment} carries no issuer-bound status reference (audit #12: a scan- or path-covering attestation MUST bind a status-list reference — a status-unbound attestation lets a revoked credential be presented unchecked)"
            ),
            CheckError::RevocationReferenceMissing { proof } => write!(
                f,
                "sub-proof {proof}: an issuer-bound status reference is present but manifest.revocation is absent (audit #12: the prover dropped the disclosed revocation reference needed to check the issuer-signed status digest)"
            ),
            CheckError::RevocationReferenceMismatch { commitment } => write!(
                f,
                "commitment {commitment}: the disclosed manifest.revocation does not match the issuer-signed status reference (audit #12: index/version/list mismatch — the prover disclosed a different reference than the issuer signed)"
            ),
            CheckError::RevocationReferenceModeInvalid { commitment } => write!(
                f,
                "commitment {commitment}: the status reference's index-disclosure mode is malformed (sq-ayv: must be EXACTLY one of clear index or index_commitment, and the disclosed reference must agree with the issuer-signed one — both-set, neither-set, or a clear/committed mode mismatch is rejected fail-closed)"
            ),
            CheckError::HiddenRevocationRequired { proof } => write!(
                f,
                "scan sub-proof {proof}: a committed-index revocation reference (clear index withheld) requires a manifest.hidden_revocation proof to check liveness against the authoritative root (sq-ayv: the committed-index path moves the liveness decision to the hidden-index proof; revocation is never skipped)"
            ),
            CheckError::HiddenRevocationIndexCommitmentMismatch => write!(
                f,
                "the hidden-revocation proof's public index commitment does not equal the issuer-signed index commitment (sq-ayv cross-binding: the index proven unset must be the index the issuer committed to)"
            ),
            CheckError::StatusSnapshotMissing { status_list, version } => write!(
                f,
                "the relying party has no AUTHORITATIVE status-list snapshot for the credential's reference (list {status_list}, version {version}) (audit #12 re-audit: the liveness bit is read from the relying party's own resolved snapshot, not the prover's — an unresolved reference fails closed)"
            ),
            CheckError::StatusSnapshotTampered { status_list, version } => write!(
                f,
                "the prover-disclosed status-list snapshot for (list {status_list}, version {version}) disagrees with the relying party's AUTHORITATIVE snapshot (audit #12 re-audit: forged/tampered bitstring — the bit decision uses the authoritative bytes, and a disagreeing prover snapshot is rejected)"
            ),
            CheckError::CredentialRevoked { status_list, index } => write!(
                f,
                "credential is REVOKED/SUSPENDED: status bit {index} is SET in list {status_list} (audit #12)"
            ),
            CheckError::StatusListStale { status_list, version } => write!(
                f,
                "status-list snapshot for {status_list} at version {version} is outside the verifier freshness window (audit #12: stale revocation view)"
            ),
            CheckError::HiddenRevocationNotEnabled => write!(
                f,
                "manifest carries a hidden-index revocation proof but the relying party's policy has not enabled the hidden-index path (no Merkle depth set) (sq-3e5/sq-h2v: the verifier cannot derive an authoritative root to bind the proof to)"
            ),
            CheckError::HiddenRevocationDepthMismatch { declared, policy } => write!(
                f,
                "hidden-index revocation proof declares Merkle depth {declared} but the relying party's policy depth is {policy} (sq-3e5/sq-h2v: trees over different leaf layouts cannot share a root)"
            ),
            CheckError::HiddenRevocationRootUnavailable { status_list, version } => write!(
                f,
                "the relying party could not derive an authoritative Merkle root for the hidden-index revocation proof (list {status_list}, version {version}) (sq-3e5/sq-h2v: no authoritative snapshot, or implausible depth -- fail-closed)"
            ),
            CheckError::HiddenRevocationRootMismatch => write!(
                f,
                "hidden-index revocation proof's public Merkle root does not equal the relying party's authoritative root (sq-3e5/sq-h2v: the proof was produced against a different/forged status list)"
            ),
            CheckError::HiddenRevocationProofRejected => write!(
                f,
                "bb rejected the hidden-index revocation proof (sq-3e5/sq-h2v: the zero-knowledge bit-unset/inclusion statement did not verify)"
            ),
            CheckError::HiddenRevocationMalformedProof => write!(
                f,
                "hidden-index revocation proof blob is malformed (sq-3e5/sq-h2v)"
            ),
            CheckError::FullyHiddenRevocationRequired => write!(
                f,
                "the revocation reference is FULLY HIDDEN (sq-kndw) but no fully_hidden_revocation proof is present — that mode moves the whole liveness decision into the proof, so accepting without it would leave revocation unchecked (fail-closed)"
            ),
            CheckError::FullyHiddenRevocationUnbound => write!(
                f,
                "a fully_hidden_revocation proof was presented without a fully-hidden revocation reference (sq-kndw): there are no issuer-signed (ref_commitment, index_commitment) values to cross-bind it to"
            ),
            CheckError::FullyHiddenRevocationSnapshotDisclosed => write!(
                f,
                "a FULLY-HIDDEN revocation presentation also attached a status-list snapshot (sq-kndw): a snapshot names its (status_list, version), which is exactly what this mode hides — drop it (the verifier reads its OWN curated snapshots)"
            ),
            CheckError::FullyHiddenRevocationNotEnabled => write!(
                f,
                "fully-hidden revocation proof present but the relying party has not enabled the path (needs RevocationPolicy::with_hidden_index_depth + with_accepted_set_depth, and a curated accepted set that fits) (sq-kndw)"
            ),
            CheckError::FullyHiddenRevocationDepthMismatch {
                declared_depth,
                declared_set_depth,
                policy_depth,
                policy_set_depth,
            } => write!(
                f,
                "fully-hidden revocation proof depths (d{declared_depth}, a{declared_set_depth}) do not match the policy depths (d{policy_depth}, a{policy_set_depth}) or name no compiled member (sq-kndw)"
            ),
            CheckError::FullyHiddenRevocationAnchorMismatch => write!(
                f,
                "fully-hidden revocation proof's declared accepted_set_root / min_version do not equal the values the relying party derives from its OWN freshness-curated policy (sq-kndw: the prover does not choose the trust anchor)"
            ),
            CheckError::FullyHiddenRevocationCommitmentMismatch => write!(
                f,
                "fully-hidden revocation proof's ref_commitment / index_commitment do not byte-equal the ISSUER-SIGNED ones (sq-kndw: the cross-binding to the issuer's reference is broken)"
            ),
            CheckError::FullyHiddenRevocationLinkageReplay => write!(
                f,
                "the fully-hidden revocation (ref_commitment, index_commitment) pair has been presented before (sq-kndw): reusing it is exactly the cross-presentation linkage this mode exists to prevent — the holder must re-blind and the issuer re-sign per presentation"
            ),
            CheckError::FullyHiddenRevocationProofRejected => write!(
                f,
                "the fully-hidden revocation proof did not verify (sq-kndw: bb rejected the ref-open / accepted-set-membership / freshness / bit-unset statement, or the proof commits a challenge other than this verifier's nonce)"
            ),
            CheckError::FullyHiddenRevocationMalformedProof => write!(
                f,
                "fully-hidden revocation proof blob or declared field is malformed (sq-kndw)"
            ),
            CheckError::HiddenIssuerNotEnabled => write!(
                f,
                "hidden-issuer attestation present but the relying party has not enabled the hidden-issuer path (no KeySet::with_hidden_issuer_depth) (sq-z9l)"
            ),
            CheckError::HiddenIssuerDepthMismatch { declared, policy } => write!(
                f,
                "hidden-issuer attestation depth {declared} does not match the KeySet policy depth {policy} (sq-z9l)"
            ),
            CheckError::HiddenIssuerRootUnavailable => write!(
                f,
                "relying party could not derive an authoritative key-set Merkle root for the hidden-issuer proof (set overflows the tree or implausible depth) (sq-z9l)"
            ),
            CheckError::HiddenIssuerRootMismatch => write!(
                f,
                "hidden-issuer proof's public key-set root does not equal the relying party's authoritative key-set root (sq-z9l: proved against a different key set)"
            ),
            CheckError::HiddenIssuerMessageMismatch { commitment } => write!(
                f,
                "hidden-issuer proof's public message does not equal the issuer-signed message recomputed for commitment {commitment} (sq-z9l: proof not bound to this committed graph)"
            ),
            CheckError::HiddenIssuerUnreferencedCommitment { commitment } => write!(
                f,
                "hidden-issuer attestation covers commitment {commitment} which no verified scan sub-proof references (sq-z9l: dangling attestation)"
            ),
            CheckError::HiddenIssuerProofRejected => write!(
                f,
                "bb rejected the hidden-issuer attestation proof (sq-z9l: the zero-knowledge signature-validity + key-set-membership statement did not verify)"
            ),
            CheckError::HiddenIssuerMalformedProof => write!(
                f,
                "hidden-issuer attestation proof blob is malformed (sq-z9l)"
            ),
            CheckError::HolderPokUnreferencedCommitment { commitment } => write!(
                f,
                "in-circuit holder PoK covers commitment {commitment} which no verified scan sub-proof references (sq-c2ql: dangling holder PoK)"
            ),
            CheckError::HolderPokBindingMissing { commitment } => write!(
                f,
                "in-circuit holder PoK over commitment {commitment} whose covering issuer attestation carries no holder binding (sq-c2ql: no issuer-attested holder_pk_digest for the binding edge to anchor on)"
            ),
            CheckError::HolderPokMissing { commitment } => write!(
                f,
                "relying party requires an in-circuit holder PoK for holder-bound commitment {commitment} but the manifest carries none (sq-c2ql: the hidden-key possession proof is mandated, fail-closed)"
            ),
            CheckError::HolderPokDigestMismatch { commitment } => write!(
                f,
                "in-circuit holder PoK's public holder_pk_digest does not equal the issuer-attested digest for commitment {commitment} (sq-c2ql binding edge: the proven holder key is not the one the issuer signed into this credential)"
            ),
            CheckError::HolderPokProofRejected { commitment } => write!(
                f,
                "bb rejected the in-circuit holder PoK for commitment {commitment} (sq-c2ql: the zero-knowledge holder-possession statement did not verify against the issuer-attested digest)"
            ),
            CheckError::HolderPokMalformedProof => write!(
                f,
                "in-circuit holder PoK proof blob is malformed (sq-c2ql)"
            ),
            CheckError::HolderSetNotEnabled => write!(
                f,
                "hidden-holder set-membership proof present but the relying party has not enabled the hidden-holder-set path (no HolderRegistry::with_hidden_holder_set_depth) (sq-3c00)"
            ),
            CheckError::HolderSetDepthMismatch { declared, policy } => write!(
                f,
                "hidden-holder set-membership proof depth {declared} does not match the registry policy depth {policy} (sq-3c00)"
            ),
            CheckError::HolderSetRootUnavailable => write!(
                f,
                "the relying party's authoritative holder-set root could not be derived (overflow / implausible depth / identity holder key) (sq-3c00)"
            ),
            CheckError::HolderSetRootMismatch => write!(
                f,
                "hidden-holder set-membership proof's public holder_set_root does not equal the relying party's authoritative root (sq-3c00: proved against a different holder set, fail-closed)"
            ),
            CheckError::HolderSetUnreferencedCommitment { commitment } => write!(
                f,
                "hidden-holder set-membership proof covers commitment {commitment} which no verified scan sub-proof references (sq-3c00: dangling set-membership proof)"
            ),
            CheckError::HolderSetProofRejected { commitment } => write!(
                f,
                "bb rejected the hidden-holder set-membership proof for commitment {commitment} (sq-3c00: the zero-knowledge holder-possession + set-membership statement did not verify)"
            ),
            CheckError::HolderSetMalformedProof => write!(
                f,
                "hidden-holder set-membership proof blob is malformed (sq-3c00)"
            ),
            CheckError::HolderRegistryEmpty => write!(
                f,
                "manifest binding is HolderPop but the relying party supplied no holder registry (sq-cwq: no trust anchor to check the holder key against — a holder PoP cannot be accepted, fail-closed)"
            ),
            CheckError::HolderNotTrusted { holder } => write!(
                f,
                "HolderPop binding holder key {holder} is not a member of the relying party's holder registry (sq-cwq: unauthorised holder)"
            ),
            CheckError::HolderPopMalformed => write!(
                f,
                "HolderPop binding is unverifiable: unknown cryptosuite, or the holder key / pop signature did not parse (sq-cwq: fail-closed, no silent accept)"
            ),
            CheckError::HolderPopInvalid { holder } => write!(
                f,
                "HolderPop binding pop signature does not verify under holder key {holder} over the challenge-bound message (sq-cwq: the presenter did not prove possession of the holder secret)"
            ),
            CheckError::HolderBindingMissing => write!(
                f,
                "HolderPop presentation requires an issuer-attested holder binding but the credential carries none — a BEARER credential presented where a per-credential holder binding is mandated (sq-z8s7 B1: fail-closed, no silent bearer fallback — closes the trusted-holder gap)"
            ),
            CheckError::HolderKeyMismatch => write!(
                f,
                "HolderPop presentation's holder key does not match the issuer-attested holder binding (sq-z8s7 B1: the presented key's holder_key_digest != the issuer-signed holder_pk_digest, the identity key, or a clear attested key disagreeing with the presented key — rejects trusted holder A presenting trusted holder B's credential)"
            ),
            CheckError::EntailmentRegimeNotAccepted { regime } => write!(
                f,
                "manifest entailment regime `{regime}` is not accepted by the relying party's EntailmentPolicy (sq-314: the regime is enforced, not free metadata — a relying party must opt into inference)"
            ),
            CheckError::UnexpectedDerivationSteps => write!(
                f,
                "a Simple-regime manifest carried derivation steps (sq-314: Simple means no inference — steps are inconsistent with the regime)"
            ),
            CheckError::MissingDerivationSteps { regime } => write!(
                f,
                "a non-Simple regime `{regime}` carried no derivation steps (sq-314: an inference regime must record the derivation it claims — fail-closed)"
            ),
            CheckError::MalformedDerivationStep { step } => write!(
                f,
                "derivation step {step} is not a well-formed instance of its rule, or its rule is not admitted by the declared regime (sq-314)"
            ),
            CheckError::UngroundedDerivationAntecedent { step, antecedent } => write!(
                f,
                "derivation step {step} antecedent {antecedent} is ungrounded (sq-314: it is neither an earlier step's derived triple nor a disclosed scan row — a derived triple cannot rest on an antecedent the proof does not establish)"
            ),
            CheckError::CompletenessUnderEntailmentUnavailable { regime } => write!(
                f,
                "the relying party requires completeness under entailment but regime `{regime}` cannot supply it (sq-rsd3v.7: UNBUILT and NOT claimed — it needs both an {} and a {}, and the saturation half exists nowhere in sparq; soundness of derivation is NOT completeness under entailment)",
                COMPLETENESS_UNDER_ENTAILMENT_UNBUILT[0], COMPLETENESS_UNDER_ENTAILMENT_UNBUILT[1]
            ),
            CheckError::EqualityReasoningUnsupported { step } => write!(
                f,
                "derivation step {step} introduces or consumes an owl:sameAs fact (sq-rsd3v.6: encoding-equality re-checks are the wrong proxy under equality reasoning — owl:sameAs needs the separate in-circuit canonicalisation member, so it is refused fail-closed here)"
            ),
            CheckError::JoinDanglingEdge { edge } => write!(
                f,
                "join edge {edge} references a non-existent sub-proof or committed-graph index (sq-sfsi: a hidden join cannot bind a proof/graph the manifest does not carry — fail-closed)"
            ),
            CheckError::JoinEdgeKindMismatch { edge } => write!(
                f,
                "join edge {edge} does not connect two scan sub-proofs to a join_eq sub-proof (sq-sfsi: scan_a/scan_b must be scans and join_proof a join_eq — fail-closed)"
            ),
            CheckError::JoinCommitmentMismatch { edge } => write!(
                f,
                "join edge {edge}: the join_eq proof's public commit_a/commit_b do not byte-equal the referenced scans' bound commitments[graph_a]/commitments[graph_b] (sq-sfsi §2.3/§4.2 anti-A2: the join is not bound to the two attested credentials — cross-scan forgery, fail-closed)"
            ),
            CheckError::JoinSlotMismatch { edge } => write!(
                f,
                "join edge {edge}: the join_eq proof's public slot_a/slot_b do not equal the query-derived slots the shared join variable occupies (sq-sfsi §4.4 slot binding: the equality was proved over the wrong column — fail-closed)"
            ),
            CheckError::JoinCommitmentChainMismatch { edge } => write!(
                f,
                "join edge {edge}: an N-way join chain over a shared variable carries differing join_commitments across its pairwise join_eq proofs (sq-r2s8 §2.4: every hop of a multi-way join must bind the SAME hiding commitment so the join value composes transitively — distinct commitments leave the N-way join unproven, fail-closed)"
            ),
            CheckError::Driver(e) => write!(f, "{e}"),
            #[cfg(feature = "extended-fragment")]
            CheckError::FragmentDispatch(e) => {
                write!(f, "extended-fragment dispatch rejected: {}", e)
            }
            #[cfg(feature = "extended-fragment")]
            CheckError::FragmentSolution(e) => {
                write!(f, "extended-fragment disclosed-solution binding rejected: {}", e)
            }
            #[cfg(feature = "extended-fragment")]
            CheckError::FragmentScan(e) => {
                write!(f, "extended-fragment BGP scan-slot binding rejected: {}", e)
            }
            #[cfg(feature = "extended-fragment")]
            CheckError::FragmentJoin(e) => {
                write!(f, "extended-fragment per-branch join coherence rejected: {}", e)
            }
        }
    }
}

impl std::error::Error for CheckError {}

impl From<VerifyError> for CheckError {
    fn from(e: VerifyError) -> Self {
        CheckError::Sparqzk(e)
    }
}

impl From<DriverError> for CheckError {
    fn from(e: DriverError) -> Self {
        CheckError::Driver(e)
    }
}

/// Re-derive a sub-proof's circuit id from its public inputs alone (the
/// verifier never trusts the declared `id`).
fn derive_id(inputs: &ProofInputs) -> Option<CircuitId> {
    match inputs {
        ProofInputs::Scan { commitments, rows, row_count, .. } => {
            let k = commitments.len() as u32;
            // The verifier knows r (= declared rows length) and row_count, but
            // not the private graph sizes; n is part of the declared id and
            // re-derived only for r/k. We recompute the id from k + rows.len()
            // (r bucket) + the declared n carried in the inputs' id.
            let r_bucket = rows.len() as u32;
            // n cannot be derived without the witness; trust the declared n in
            // the id but bind k and r.
            let declared_n = match inputs.circuit_id() {
                CircuitId::Scan { n, .. } => *n,
                _ => return None,
            };
            let _ = row_count;
            derive_scan_id(k, declared_n, r_bucket.max(*row_count))
        }
        ProofInputs::FilterInt { bound, .. } => {
            // digit count of the *bound* is not the operand's; the operand's
            // digit count is private. The declared d is re-checked against the
            // member family only (it must be a compiled d).
            let _ = bound;
            let d = match inputs.circuit_id() {
                CircuitId::FilterInt { d } => *d,
                _ => return None,
            };
            derive_filter_int_id(d)
        }
        // [OPUS-4.8] sq-q7e + sq-tat: composable xsd:double FILTER. As with
        // filter_int, the operand's digit count is PRIVATE; the declared `d` is
        // re-checked against the compiled f64 family only (it must be a compiled
        // d). The full CircuitId (incl. `d`) is what the audit-#1 public-input
        // reconstruction + canonical-vk recompute pin, so a wrong `d` cannot
        // byte-match a real member's proof.
        ProofInputs::FilterF64 { .. } => {
            let d = match inputs.circuit_id() {
                CircuitId::FilterF64 { d } => *d,
                _ => return None,
            };
            derive_filter_f64_id(d)
        }
        // [OPUS-4.8] sq-7lrq: composable SIGNED xsd:integer FILTER. As with
        // filter_int, the operand's MAGNITUDE-digit count is PRIVATE; the declared
        // `md` is re-checked against the compiled signed-int family only (it must be
        // a compiled MD). The full CircuitId is what the public-input reconstruction
        // + canonical-vk recompute pin, so a wrong `md` cannot byte-match a real
        // member's proof.
        ProofInputs::FilterSignedInt { .. } => {
            let md = match inputs.circuit_id() {
                CircuitId::FilterSignedInt { md } => *md,
                _ => return None,
            };
            derive_filter_signed_int_id(md)
        }
        // [OPUS-4.8] sq-7lrq: composable xsd:decimal FILTER. The operand's
        // integer-/fraction-digit counts are PRIVATE; the declared `(id, fd)` is
        // re-checked against the compiled decimal family only (it must be a compiled
        // shape). The full CircuitId is what the reconstruction + canonical-vk pin.
        ProofInputs::FilterDecimal { .. } => {
            let (id, fd) = match inputs.circuit_id() {
                CircuitId::FilterDecimal { id, fd } => (*id, *fd),
                _ => return None,
            };
            derive_filter_decimal_id(id, fd)
        }
        // [OPUS-4.8] sq-xojl: DUAL-LEAF value-lane FILTER. DIGIT-COUNT-FREE — the
        // member id carries no `d`/`md`/`(id,fd)` parameter (the per-digit family
        // collapses), so the derive only confirms the declared id is the single
        // `FilterValueDl` member. The full CircuitId is what the public-input
        // reconstruction + canonical-vk recompute pin. The fail-closed legality of
        // this member against the recorded COMMITMENT METHOD (it is LEGAL only for
        // `DualLeafV1` / the `ValueOnlyV1` research dial, never `string-canonical`)
        // is the dispatch matrix `crate::dispatch::resolve_circuit` (sq-cfmv) — the
        // host-encoding wiring that gives the verifier the method is sq-j506, gated
        // behind sq-qhy4. Here the derive only confirms the member identity.
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDl { .. } => match inputs.circuit_id() {
            CircuitId::FilterValueDl => Some(CircuitId::FilterValueDl),
            _ => None,
        },
        // [OPUS-4.8] sq-2ezsx: the DUAL-LEAF double + decimal value-lane FILTERs.
        // Like the integer member they are DIGIT-COUNT-FREE (the per-digit family
        // collapses; the decimal class is even scale-agnostic — the scale lives in
        // the public `datatype_const`, not the member id), so the derive only
        // confirms the declared id is the single member of its datatype class. The
        // full CircuitId is what the reconstruction + canonical-vk pin. The
        // fail-closed `(method × circuit)` legality is `crate::dispatch` (sq-cfmv).
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDlF64 { .. } => match inputs.circuit_id() {
            CircuitId::FilterValueDlF64 => Some(CircuitId::FilterValueDlF64),
            _ => None,
        },
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDlDecimal { .. } => match inputs.circuit_id() {
            CircuitId::FilterValueDlDecimal => Some(CircuitId::FilterValueDlDecimal),
            _ => None,
        },
        // [OPUS-5] sq-wz99x: the DUAL-LEAF dateTime/date value-lane FILTER. Also
        // DIGIT-COUNT-FREE, and additionally LANE-FREE: ONE member serves BOTH the
        // `xsd:dateTime` and `xsd:date` classes, because the lane (and the
        // sub-second scale `FS`) lives in the PUBLIC `datatype_const`, not the
        // member id. So the derive only confirms the declared id is that single
        // member; WHICH lane a proof is for is pinned by the public-input
        // reconstruction below — `datatype_const` is a public input, so a lane swap
        // changes the reconstructed vector and cannot byte-match the proof. The
        // fail-closed `(method × circuit)` legality is `crate::dispatch` (sq-cfmv).
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDlDateTime { .. } => match inputs.circuit_id() {
            CircuitId::FilterValueDlDateTime => Some(CircuitId::FilterValueDlDateTime),
            _ => None,
        },
        // [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): hidden cross-credential JOIN. The
        // two graph sizes are PRIVATE (the witnessed graph contents are not public
        // inputs — only the commitments are), so — exactly as scan trusts the
        // declared `n` and filter the declared `d` — the verifier re-derives the
        // member id from the declared `(n_a, n_b)` buckets carried in the inputs'
        // id. The full CircuitId is what the step-4 (sq-sfsi) public-input
        // reconstruction + canonical-vk recompute pin, so a wrong bucket cannot
        // byte-match a real member's proof.
        ProofInputs::JoinEq { .. } => {
            let (n_a, n_b) = match inputs.circuit_id() {
                CircuitId::JoinEq { n_a, n_b } => (*n_a, *n_b),
                _ => return None,
            };
            derive_join_eq_id(n_a, n_b)
        }
        // [OPUS-4.8] sq-3kd2g.6: bounded-depth path reachability. The declared id
        // carries the depth bound `d` and slot bucket `n`; `k` is re-derived from
        // `commitments.len()` (the arity a wrong-length attribution cannot fake —
        // it changes the reconstructed public-input vector's length). The disclosed
        // `depth_bound` MUST equal the member's `d` (it is constant-constrained to
        // `D` in-circuit, soundness req 1), so a manifest disclosing a different
        // bound derives NO id (fail-closed). `derive_path_reach_id` then requires
        // `(d, k, n)` to be an EXACTLY compiled member (a wrong `k` bucket => a
        // different / no derived id => `CircuitIdMismatch` upstream).
        #[cfg(feature = "extended-fragment")]
        ProofInputs::PathReach { commitments, depth_bound, .. } => {
            let (d, n) = match inputs.circuit_id() {
                CircuitId::PathReach { d, n, .. } => (*d, *n),
                _ => return None,
            };
            if *depth_bound != d {
                return None;
            }
            let k = commitments.len() as u32;
            crate::build::derive_path_reach_id(d, k, n)
        }
    }
}

/// Stage 1+2: the structural PRE-FILTER (no bb). Returns the required obligation
/// edges on success.
///
/// # ⚠️ THIS IS NOT A VERIFIER — it provides NO cryptographic or attribution soundness.
///
/// [OPUS-4.8] codex 2223 MEDIUM. This runs only the JSON/structural stages
/// (circuit-id re-derivation, binding-edge consistency, query-correctness
/// binding, the cross-graph attribution length+superset gate, and the
/// issuer-signature/salt checks). It does **NOT** run the bb proof verification
/// and does **NOT** run the public-input reconstruction byte-compare. The
/// attribution / pattern / filter / commitment bits it inspects are therefore
/// **NOT cryptographically bound to any proof here** — a manifest can pass this
/// pre-filter while its declared `ProofInputs` differ from what the bb proofs
/// actually attest. The only thing that binds the JSON statement to the proof is
/// [`verify_manifest`], whose stage 3 reconstructs the public-input vector from
/// the DECLARED inputs and byte-compares it against each proof's `public_inputs`
/// (and then runs `bb verify`). **A relying party MUST call [`verify_manifest`];
/// this function is an internal fast pre-filter / test seam only and accepting a
/// manifest on its result alone is unsound.**
///
/// The name was deliberately chosen so it cannot be mistaken for a verifier (it
/// was `verify_manifest_structure`, which read as a verification entry point).
///
/// `trusted_key_set` is the relying party's EXTERNALLY-anchored issuer trust set
/// `K` (audit #3 codex #1) — NOT read from the manifest. Every committed graph's
/// attestation key must be a member of THIS set, and the prover's
/// `manifest.key_set` (if any) must be a subset of it. See [`KeySet`].
///
/// `revocation_policy` is the relying party's freshness/revocation policy (audit
/// #12) — also external, NOT read from the manifest. The status check is
/// mandatory: a scan-covering credential MUST carry an issuer-bound status
/// reference, show a status bit UNSET in a disclosed snapshot, and that snapshot
/// MUST be within the policy's freshness window. See [`RevocationPolicy`].
pub fn prefilter_manifest_structure(
    manifest: &ProofManifest,
    trusted_key_set: &KeySet,
    revocation_policy: &RevocationPolicy,
) -> Result<Vec<JoinEdge>, CheckError> {
    // The public structural pre-filter is the FLAT stage-1 regime
    // (`skip_query_binding = false`): every stage-1 query-text gate runs, so its
    // behaviour is byte-identical with or without the `extended-fragment` feature.
    prefilter_manifest_structure_impl(manifest, trusted_key_set, revocation_policy, false)
}

/// Shared body of [`prefilter_manifest_structure`]. `skip_query_binding` selects
/// the stage-1 QUERY-TEXT binding regime; every query-INDEPENDENT gate runs
/// identically in both.
///
/// - `false` — the FLAT stage-1 fragment, the ONLY value the sound
///   [`verify_manifest`] path and the default build ever use. Stage 1a runs the
///   full [`recheck`] (`fragment_patterns` + the flat cross-graph Q6 obligation
///   binding), and the query-text term-binding gates `bind_query_correctness`,
///   `bind_attributions`, `bind_joins` all run.
/// - `true` — the EXTENDED-fragment regime, set ONLY by
///   `verify_fragment_manifest` AFTER `dispatch_fragment` has re-derived and
///   routed the query through `fragment_query` (sq-h732x). Stage 1a's
///   query-fragment ACCEPTANCE is routed through `fragment_query` (accepts the
///   wave-1 `UNION` / `VALUES` / property-path extensions, fail-closed on anything
///   outside), and the FLAT cross-graph obligation binding plus the per-branch
///   term binding (`bind_query_correctness` / `bind_attributions` / `bind_joins`)
///   are DEFERRED to the structural routing gate + bead sq-1zf94.
///
/// The query-INDEPENDENT gates — id hygiene (stage 1b), binding edges (stage 2),
/// issuer attestation (stage 2d), revocation (stage 2f) — are UNCHANGED in both
/// regimes. This is the honest scope of the extended-fragment routing: it lets an
/// accepted extended query's sub-proofs verify end-to-end WITHOUT claiming the
/// disclosed path/`VALUES` terms are bound to the proofs (that binding is
/// sq-1zf94). NOT externally audited (sq-qhy4).
// [OPUS-4.8] sq-h732x: mode-aware stage-1 pre-filter (flat vs extended fragment).
fn prefilter_manifest_structure_impl(
    manifest: &ProofManifest,
    trusted_key_set: &KeySet,
    revocation_policy: &RevocationPolicy,
    skip_query_binding: bool,
) -> Result<Vec<JoinEdge>, CheckError> {
    // --- Stage 1a: query-fragment gate + (flat stage-1 only) cross-graph
    // obligations. ---
    let required = if skip_query_binding {
        // Extended-fragment regime: route the query-fragment ACCEPTANCE through
        // `fragment_query` (fail-closed on anything OUTSIDE the wave-1 fragment)
        // instead of the stage-1-only `fragment_patterns`, so the extended query is
        // not rejected here. The flat cross-graph Q6 obligation binding is DEFERRED
        // to `dispatch_fragment` (structural routing) + sq-1zf94 (term binding), so
        // there are no flat obligations to return.
        #[cfg(feature = "extended-fragment")]
        fragment_query(&manifest.query).map_err(CheckError::Sparqzk)?;
        Vec::new()
    } else {
        // [OPUS-4.8] sq-en5dx (Finding A of the sq-1s2.6 composition review): feed
        // the Q6 cross-graph obligation gate a GLOBAL-namespace attribution vector,
        // NOT the raw scan-LOCAL indices. `manifest.attributions[pi]` indexes the
        // ANSWERING scan's OWN `commitments`, so two DISTINCT `k=1` scans both
        // declaring local index 0 (the `[[0],[0]]` cross-scan alias) would COLLAPSE
        // to one element and the non-bnode obligation would be DROPPED for the
        // cross-scan join. Keying the union on the committed-graph IDENTITY (via
        // `global_attributions`) makes two distinct graphs distinct so the
        // obligation is correctly required, while two scans over the SAME graph
        // still collapse (a same-graph bnode join is legitimate). See
        // `global_attributions` for the fail-closed edge cases.
        let attributions = global_attributions(manifest);
        let declared: Vec<JoinEdge> = manifest
            .join_obligations
            .iter()
            .map(|(variable, i, j)| JoinEdge {
                variable: variable.clone(),
                patterns: (*i, *j),
            })
            .collect();
        recheck(&manifest.query, &attributions, &declared)?
    };

    // --- Stage 1b: re-derive each circuit id from public inputs. ---
    for (i, sp) in manifest.sub_proofs.iter().enumerate() {
        let declared = sp.inputs.circuit_id().clone();
        let derived = derive_id(&sp.inputs);
        if derived.as_ref() != Some(&declared) {
            return Err(CheckError::CircuitIdMismatch {
                proof: i,
                declared,
                derived,
            });
        }
        // [OPUS-4.8] codex 2221 MEDIUM (fail-closed for audit #8): a scan's
        // per-graph source attribution is a security-relevant, proof-bound
        // quantity (`scan.nr` step 4) the cross-graph obligation gate (stage 2e)
        // cross-checks against `manifest.attributions`. `#[serde(default)]` lets a
        // prover OMIT it (empty vec) or under-length it; `bind_attributions` then
        // only inspects the bits PRESENT, so an omitted attribution makes the
        // under-declaration cross-check vacuous and resurrects the `[[0],[0]]`
        // collapse forge. Require it PRESENT and EXACTLY `CircuitId.k` bits — the
        // same k the (already-verified) circuit id declares and the audit #1
        // reconstruction byte-binds — with NO default and NO pad-to-false. A
        // missing/empty/short/long attribution is rejected here, before any gate
        // can silently skip the omitted graphs.
        if let ProofInputs::Scan { attribution, commitments, .. } = &sp.inputs {
            let k = match &declared {
                CircuitId::Scan { k, .. } => *k as usize,
                _ => unreachable!("Scan inputs always carry a Scan circuit id"),
            };
            if attribution.len() != k {
                return Err(CheckError::AttributionMalformed {
                    proof: i,
                    expected: k,
                    got: attribution.len(),
                });
            }
            // [OPUS-4.8] sq-vxq8 / plan S2.5: distinct-graph strict ordering.
            // `scan_check` step 1b enforces `commitments[0] < commitments[1] < ...`
            // in-circuit to force the K committed graphs pairwise distinct (closing
            // the duplicate-inclusion / COUNT-forgery class). Mirror it structurally
            // here so a witness-only manifest with a duplicate/out-of-order
            // commitment is rejected BEFORE any bb proof (defence in depth). Compare
            // on the SAME canonical big-endian bytes the circuit's `Field::lt` and
            // the audit-#1 public-input reconstruction use, so the host and circuit
            // orders agree exactly. A malformed commitment hex is left to the
            // reconstruction stage (`MalformedField`); skip it here.
            for g in 1..commitments.len() {
                let (Some(prev), Some(cur)) =
                    (commitments[g - 1].to_field(), commitments[g].to_field())
                else {
                    continue;
                };
                if field_to_be_bytes_32(&cur) <= field_to_be_bytes_32(&prev) {
                    return Err(CheckError::ScanCommitmentsNotStrictlyIncreasing {
                        proof: i,
                        at: g,
                    });
                }
            }
        }
    }

    // --- Stage 2: binding-consistency edges. ---
    for (e, edge) in manifest.binding_edges.iter().enumerate() {
        let from = manifest
            .sub_proofs
            .get(edge.from_proof)
            .ok_or(CheckError::DanglingEdge { edge: e })?;
        let to = manifest
            .sub_proofs
            .get(edge.to_proof)
            .ok_or(CheckError::DanglingEdge { edge: e })?;
        let scanned = match &from.inputs {
            ProofInputs::Scan { rows, .. } => rows
                .get(edge.from_row)
                .and_then(|r| r.get(edge.from_slot))
                .ok_or(CheckError::DanglingEdge { edge: e })?,
            _ => return Err(CheckError::EdgeKindMismatch { edge: e }),
        };
        // [OPUS-4.8] sq-q7e + sq-tat: a binding edge may consume the scanned
        // column into EITHER an xsd:integer FILTER (filter_int) or a composable
        // xsd:double FILTER (filter_f64) — both carry `operand_enc` as the
        // scan-proof anchor, bound to the committed literal in-circuit.
        // [OPUS-4.8] sq-7lrq: a binding edge may also consume the scanned column
        // into a SIGNED xsd:integer FILTER (filter_signed_int) or an xsd:decimal
        // FILTER (filter_decimal); both carry `operand_enc` as the scan-proof anchor,
        // bound to the committed literal in-circuit (same mechanism as filter_int).
        let operand = match &to.inputs {
            ProofInputs::FilterInt { operand_enc, .. } => operand_enc,
            ProofInputs::FilterF64 { operand_enc, .. } => operand_enc,
            ProofInputs::FilterSignedInt { operand_enc, .. } => operand_enc,
            ProofInputs::FilterDecimal { operand_enc, .. } => operand_enc,
            _ => return Err(CheckError::EdgeKindMismatch { edge: e }),
        };
        if scanned != operand {
            return Err(CheckError::BindingInconsistent { edge: e });
        }
    }

    // --- Stage 2b + 2c: query-correctness binding (audit #5/#6/#7/#10). ---
    // The bb public-input vector already cryptographically binds the scan
    // pattern constants and the FILTER op/bound/expected/operand_enc (audit
    // #1). This stage is the VERIFIER-SIDE check that those bound values match
    // the query the relying party reads in `manifest.query` — without it a
    // proof of one statement is presented under a different query.
    //
    // [OPUS-4.8] sq-h732x: this gate re-parses the query with the FLAT
    // `fragment_patterns` (rejecting `UNION`/`VALUES`/paths) and binds scan
    // constants per the flat BGP model. In the extended-fragment regime
    // (`skip_query_binding`) the per-branch term binding is DEFERRED to sq-1zf94,
    // so it is skipped here (documented unbound-terms limitation — the sub-proofs
    // still verify cryptographically, they are just not yet tied to the disclosed
    // solution terms).
    //
    // [OPUS-5] sq-q9r5e follow-up: stage 2a′ first — when the prover DECLARED a
    // `manifest.pattern_scans` mapping, re-check it here. It is an ADDITIONAL
    // fail-closed constraint only: the FILTER and attribution obligations below
    // still run over the full constant-MEMBERSHIP relation, so a declaration can
    // never shrink what the verifier demands (see `check_pattern_scans`).
    //
    // [OPUS-5] #5240: stage 2b′ last — a variable repeated WITHIN one BGP pattern
    // (`{ ?v <p> ?v }`) constrains the two disclosed columns to be EQUAL, which no
    // other gate enforced: `scan_matches_pattern` compares only const-ness and the
    // constant encodings, and every shared-variable gate (`recheck` /
    // `join_obligations` on the disclosed path, `bind_joins` on the hidden path)
    // works across pattern PAIRS and so cannot see a within-pattern repeat. Placed
    // AFTER the gates above so their (already-pinned) error precedence is unchanged.
    if !skip_query_binding {
        check_pattern_scans(manifest)?;
        bind_query_correctness(manifest)?;
        bind_repeated_pattern_slots(manifest)?;
    }

    // --- Stage 2e: cross-graph attribution binding (audit #8). ---
    // Bind manifest.attributions (the JSON sets fed to the Q6 obligation gate in
    // stage 1a) to the PROOF-BOUND per-graph attribution each scan sub-proof
    // carries (scan.nr step 4, byte-bound by the audit #1 reconstruction). A
    // prover whose pattern genuinely matches in two graphs can no longer declare
    // a collapsed `[[0],[0]]` to drop the cross-graph non-bnode obligation: the
    // declared attribution must be a SUPERSET of the proof-bound matched-graph
    // set, so under-declaring a contributing graph is rejected here.
    //
    // [OPUS-4.8] sq-h732x: re-parses via the flat `fragment_patterns` to map
    // patterns->scans, so the extended-fragment regime (`skip_query_binding`)
    // defers it to the per-branch routing (`dispatch_fragment`) + sq-1zf94.
    if !skip_query_binding {
        bind_attributions(manifest)?;
    }

    // --- Stage 2d: issuer-signature / key-set binding (audit #3 / codex #1). ---
    // Every scan sub-proof's commitments[g] must carry a valid issuer signature
    // whose key ∈ the EXTERNAL trusted K (the verifier's argument, NOT
    // manifest.key_set). commitments[g] is byte-bound into the bb public inputs
    // by the audit #1 reconstruction, so this verifier-side check ties the
    // attested commitment to the proved statement. The issuer signature ALSO
    // binds the credential's status-list reference (audit #12), so a scan-covering
    // attestation that omits/forges the reference is rejected here.
    //
    // [OPUS-4.8] sq-xxg: a scan commitment may be covered EITHER by a clear
    // attestation OR by a hidden-issuer proof (when the relying party enabled the
    // hidden path). Compute the STRUCTURALLY hidden-covered commitments so the
    // clear attestation is treated as NOT-REQUIRED for them; the fail-closed
    // either-clear-or-hidden / never-neither rule is enforced inside
    // `bind_issuer_attestations`. The hidden proof itself is cryptographically
    // verified by `bind_hidden_issuer_attestations` in the bb stage of
    // `verify_manifest` (which binds its public `key_set_root` to the relying
    // party's authoritative KeySet and its `m` to the issuer-signed message), so a
    // hidden-only commitment is no less attested than a clear one — only WHICH
    // issuer signed is hidden.
    let hidden_covered = hidden_issuer_covered_commitments(manifest, trusted_key_set);
    bind_issuer_attestations(manifest, trusted_key_set, &hidden_covered)?;

    // --- Stage 2f: revocation / freshness (audit #12). ---
    // The status reference is now issuer-bound (stage 2d); check the credential's
    // status bit is UNSET in the disclosed snapshot and the snapshot version is
    // within the relying party's freshness window. A revoked bit, a stale
    // snapshot, or a missing snapshot all REJECT (fail-closed).
    bind_revocation(manifest, revocation_policy)?;

    // --- Stage 2g: hidden cross-credential JOIN binding (sq-sfsi, step 4). ---
    // The hidden-key analogue of the binding-edge + query-correctness stages: for
    // each DECLARED `JoinEdge`, require the `join_eq` proof's public commit_a/commit_b
    // to byte-equal the two referenced scans' bound commitments[graph_*] (anti-A2,
    // §2.3/§4.2) and require its public slot_a/slot_b to equal the query-derived
    // slots a variable SHARED across the two answered patterns occupies (§4.4 slot
    // binding — which doubles as the anti-spurious-join check). A query cross-scan
    // shared variable WITHOUT a hidden JoinEdge is discharged by the disclosed-row
    // path (`recheck`/`join_obligations`, stage 1a), NOT demanded here — the hidden
    // join is the opt-in privacy alternative (see bind_joins' scope docs). The
    // `join_eq` proof itself is cryptographically
    // verified in `verify_manifest`'s per-sub-proof loop (canonical vk by re-derived
    // CircuitId::JoinEq, audit-#1 public-input byte-compare, bb verify) — this is the
    // structural gate that ties those bound public inputs to the attested scans and
    // the query. Placed AFTER bind_issuer_attestations so the commitments it
    // byte-matches are already known issuer-attested + in K (design §3.3 step 3).
    //
    // [OPUS-4.8] sq-h732x: `bind_joins` re-derives the query-shared join slots via
    // the flat `fragment_patterns`, so the extended-fragment regime
    // (`skip_query_binding`) defers it — the hidden-join slot binding over a
    // multi-branch query is part of the per-branch term binding (sq-1zf94).
    if !skip_query_binding {
        bind_joins(manifest)?;
    }

    Ok(required)
}

/// Stage 2d: bind every committed graph a verified sub-proof draws triples from to
/// an issuer signature whose key is in the EXTERNAL trusted key-set `K` (audit #3,
/// soundness fix for codex #1). For each `commitments[g]` of each `Scan` and — in
/// the `extended-fragment` regime — each bounded-path `PathReach` sub-proof
/// (sq-nlulr; a path graph is attested + salt-recorded on the same footing as a
/// scan graph, so the flat cross-graph non-bnode discipline extends to a
/// scan↔single-graph-path join):
/// - there MUST be a `commitment_attestations` entry over that commitment value
///   OR a hidden-issuer proof covering it (sq-xxg, see below),
/// - its signature MUST verify under its declared `issuer_public_key`,
/// - that key MUST be a member of the EXTERNAL `trusted_key_set` (the relying
///   party's argument — NEVER `manifest.key_set`),
/// - and, when the prover DECLARED a narrowed `manifest.key_set` (non-empty),
///   that key MUST ALSO be a member of it (codex 2216 LOW): a declared narrowing
///   must be internally consistent with the attestations actually proven. The
///   accept decision stays anchored on the external K (this consistency rule is
///   ADDED to, never substituted for, the external-K check).
///
/// # sq-xxg: clear attestation is OPTIONAL when a hidden-issuer proof covers it
/// `hidden_covered` is the set of commitment-hex keys for which the manifest
/// carries a `hidden_issuer_attestations` entry AND the relying party enabled the
/// hidden-issuer path. For a commitment in this set, the clear-key
/// `commitment_attestations` entry is NOT required: the hidden-issuer proof (whose
/// public `key_set_root` is bound to the relying party's authoritative KeySet, and
/// whose `m` is the issuer-signed message, both checked cryptographically by
/// [`bind_hidden_issuer_attestations`] in the bb stage) is the attestation. The
/// privacy gain is that WHICH trusted issuer signed is hidden; the soundness
/// guarantee ("signed by SOME key in the relying party's K over this committed
/// graph's status-bound message") is unchanged.
///
/// FAIL-CLOSED (either-clear-or-hidden, NEVER neither): a commitment with NEITHER
/// a clear attestation NOR a hidden-issuer entry is rejected
/// ([`CheckError::UnattestedCommitment`]). A commitment WITH a clear attestation
/// is still fully checked here (key ∈ K, signature, salt, status) regardless of
/// any hidden entry — the hidden path only RELAXES the *requirement* for a clear
/// entry, it never relaxes the checks applied to a clear entry that IS present.
/// So no commitment can go unattested, and a hidden-only commitment is gated by
/// the (mandatory, fail-closed) bb hidden-issuer verification.
///
/// # The codex #1 soundness fix
/// Previously the trust anchor was `manifest.key_set` — PROVER-supplied. A
/// malicious prover could sign a forged commitment with its own key, list that
/// key in `manifest.key_set`, and pass. That made `#3` vacuous. Now the anchor
/// is `trusted_key_set`, an external relying-party input. `manifest.key_set` is
/// only allowed as a SUBSET of the external `K` (a prover may narrow but never
/// widen the trust set); a `manifest.key_set` key NOT in the external `K` is
/// rejected ([`CheckError::UntrustedDeclaredKey`]) so the accept decision can
/// never depend on a prover-chosen key.
///
/// Closes: (a) an unsigned/prover-invented commitment (no matching attestation),
/// (b) drop-a-triple-and-recommit suppression (the truncated graph's `C(G')`
/// differs from the signed `C(G)`, so no valid attestation exists),
/// (c) a signature by a key not in the external K (incl. the prover's own key —
/// the forge the codex #1 test exercises). The commitment is no longer an
/// unsigned prover-chosen public input under a prover-chosen trust anchor.
fn bind_issuer_attestations(
    manifest: &ProofManifest,
    trusted_key_set: &KeySet,
    hidden_covered: &BTreeSet<String>,
) -> Result<(), CheckError> {
    // The prover's declared key_set may NARROW but never WIDEN the external trust
    // anchor: every key it lists must already be in the external K. (A
    // declared-but-untrusted key is the codex #1 forge: prover lists its own key.)
    for declared in &manifest.key_set {
        if !trusted_key_set.contains_hex(declared) {
            return Err(CheckError::UntrustedDeclaredKey {
                key: declared.clone(),
            });
        }
    }

    // [OPUS-4.8] codex 2223 LOW / sq-nlulr: the verified per-graph salt for every
    // commitment ACTUALLY REFERENCED by a verified SCAN or (extended-fragment) PATH
    // sub-proof. The salt-uniqueness check (step 3) runs ONLY over this referenced
    // set, not over every declared attestation: the #9 security property only
    // concerns committed graphs a verified sub-proof drew triples from, so an
    // unrelated extra attestation reusing a salt must NOT false-reject a valid
    // proof. Keyed by canonical commitment hex (so the same graph referenced by
    // several scans/paths records once); the value is the verified salt hex.
    // Populated only after the attestation over `c` has fully verified (key ∈ K,
    // signature valid, salt present + salt-bound), so a recorded salt is always
    // issuer-attested.
    let mut referenced_salt: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();

    for (pi, sp) in manifest.sub_proofs.iter().enumerate() {
        // [OPUS-4.8] sq-nlulr: the audit-#9 issuer-attestation + salt-uniqueness
        // requirement covers EVERY committed graph a VERIFIED sub-proof draws
        // triples from. A BGP `Scan` and, in the extended fragment, a bounded
        // property-path `PathReach` both expose `commitments` (each with a
        // proof-bound `attribution`), so a path commitment is attested and
        // salt-recorded on the SAME footing as a scan commitment. This is what
        // makes a cross-graph scan<->single-graph-PATH join's non-bnode COROLLARY
        // (`bind_fragment_join_coherence`) carry the flat path's distinct-salt
        // discipline: the path graph now contributes an issuer-attested,
        // distinctly-salted commitment to `referenced_salt`, so the salt-uniqueness
        // gate (step 3) rejects a path graph reusing a scan graph's salt. Flat
        // manifests carry NO `PathReach` sub-proof (that variant is
        // `extended-fragment`-gated), so the default build is byte-identical: the
        // match collapses to the original `Scan`-or-continue.
        let commitments: &[FieldHex] = match &sp.inputs {
            ProofInputs::Scan { commitments, .. } => commitments,
            #[cfg(feature = "extended-fragment")]
            ProofInputs::PathReach { commitments, .. } => commitments,
            _ => continue,
        };
        for c in commitments {
            // Find an attestation declared over this exact commitment value
            // (compare as field elements, so 0x-padding differences don't slip
            // an unattested commitment past).
            let c_field = c.to_field();
            let att = manifest.commitment_attestations.iter().find(|a| {
                a.commitment.to_field().is_some() && a.commitment.to_field() == c_field
            });
            let Some(att) = att else {
                // [OPUS-4.8] sq-xxg: no CLEAR attestation. This is acceptable ONLY
                // if a hidden-issuer proof covers this commitment (and the relying
                // party enabled the hidden path) — the fail-closed
                // either-clear-or-hidden / never-neither rule. The hidden proof is
                // cryptographically verified by `bind_hidden_issuer_attestations`
                // (bb stage), so a hidden-covered commitment is fully attested
                // there; here we only RELAX the clear-entry requirement. A
                // commitment covered by NEITHER is rejected as unattested.
                if let Some(c_fr) = c_field {
                    if hidden_covered.contains(&field_to_hex(&c_fr)) {
                        // The hidden-only commitment still participates in the
                        // audit-#9 salt-uniqueness check (the Q6 cross-graph
                        // bnode-correlation channel applies to ANY committed graph
                        // a verified scan drew from, hidden-attested or clear). Its
                        // salt is the one the verifier uses to recompute the
                        // issuer-signed `m` (resolve_commitment_salt). If no salt is
                        // disclosed for a hidden-only commitment, the message cannot
                        // be recomputed and the bb hidden-issuer gate rejects it as
                        // unreferenced (fail-closed) — so we record only a present,
                        // parseable salt here.
                        if let Some(salt_fr) = resolve_commitment_salt(manifest, &c_fr) {
                            referenced_salt
                                .insert(field_to_hex(&c_fr), field_to_hex(&salt_fr));
                        }
                        continue;
                    }
                }
                return Err(CheckError::UnattestedCommitment {
                    proof: pi,
                    commitment: c.0.clone(),
                });
            };
            // (1) The attestation key must be in the EXTERNAL trusted set K.
            // (Check membership BEFORE the signature so an untrusted issuer is
            // the reported reason even if its signature is internally valid.)
            // This is the codex #1 fix: K is the verifier's argument, never the
            // prover-supplied manifest.key_set.
            if !trusted_key_set.contains_hex(&att.issuer_public_key) {
                return Err(CheckError::IssuerKeyNotInKeySet {
                    commitment: c.0.clone(),
                });
            }
            // (1b) [OPUS-4.8] codex 2216 LOW: declared-key_set consistency. The
            // accept decision is ALREADY anchored on the external K above (never
            // weakened here). But if the prover DECLARED a narrowed key_set
            // (non-empty), every accepted attestation key must also be a member
            // of it — otherwise the advertised narrowing is inconsistent with the
            // attestations actually proven (the prover claims a tighter issuer
            // set than it used). An empty declared key_set means "no narrowing
            // declared", so this is skipped (external K alone governs).
            if !manifest.key_set.is_empty()
                && !manifest
                    .key_set
                    .iter()
                    .any(|k| normalize_hex(k) == normalize_hex(&att.issuer_public_key))
            {
                return Err(CheckError::AttestationKeyNotInDeclaredSet {
                    commitment: c.0.clone(),
                    key: att.issuer_public_key.clone(),
                });
            }
            // (2) The cryptosuite must be a known/verifiable scheme, the key +
            // signature must parse, and the signature must verify over the
            // SALT-BOUND domain-separated commitment message. Any failure =>
            // reject (fail closed; prover-controlled bytes never panic).
            let Some(commitment_fr) = c_field else {
                return Err(CheckError::InvalidIssuerSignature {
                    commitment: c.0.clone(),
                });
            };
            // [OPUS-4.8] codex 2221 HIGH (fail-closed for audit #9): an
            // attestation that COVERS a verified scan commitment MUST carry a salt
            // and be verified via the salt-bound `commitment_message_with_salt`
            // path. The salt-less (legacy) `commitment_message` path does NOT bind
            // the per-graph RDFC10 salt and does NOT participate in the
            // salt-uniqueness check below, so accepting it here would let a
            // salt-reusing ingester bypass the whole #9 salt-separation guarantee
            // simply by OMITTING the salt field. There is therefore NO `salt:
            // None` branch on the scan-covering path — `None` is rejected. (The
            // bare `commitment_message` remains a primitive used elsewhere; it is
            // not reachable from the default scan-verify path.) A salt that is
            // present but unparseable also fails closed.
            let salt_fr = match &att.salt {
                Some(salt_hex) => match salt_hex.to_field() {
                    Some(salt_fr) => salt_fr,
                    None => {
                        return Err(CheckError::InvalidIssuerSignature {
                            commitment: c.0.clone(),
                        })
                    }
                },
                None => {
                    return Err(CheckError::ScanCommitmentSaltMissing {
                        proof: pi,
                        commitment: c.0.clone(),
                    })
                }
            };
            // [OPUS-4.8] audit #12 (fail-closed, leverages #3): a scan-covering
            // attestation MUST bind the credential's STATUS-LIST REFERENCE. The
            // issuer signs `commitment_message_with_status(C(G), salt,
            // status_ref_digest)`, where `status_ref_digest` folds the disclosed
            // `manifest.revocation` (H(list IRI), index, version). So the signed
            // message can only be reconstructed from a disclosed reference that
            // MATCHES what the issuer signed — an omitted/forged/swapped reference
            // yields a different message and so no valid signature (the
            // omit-the-field bypass that bit #3/#8/#9/#4 is closed here). There is
            // NO `status: None` branch on the scan-covering path — `None` is
            // rejected (mirrors the salt-mandatory precedent above).
            let Some(att_status) = &att.status else {
                return Err(CheckError::ScanCommitmentStatusMissing {
                    proof: pi,
                    commitment: c.0.clone(),
                });
            };
            // The disclosed revocation reference is required to recompute the
            // signed digest (the prover cannot drop it to skip the check).
            let Some(rev) = &manifest.revocation else {
                return Err(CheckError::RevocationReferenceMissing { proof: pi });
            };
            // The disclosed reference MUST equal what the issuer signed (the
            // attestation's `AttestedStatusRef`), in the SAME disclosure mode. A
            // mismatch means the prover disclosed a different reference than was
            // signed (e.g. an index/commitment whose bit is unset); reject
            // explicitly. [OPUS-4.8] sq-ayv: `resolve_status_ref` handles BOTH the
            // clear-index (audit #12) and committed-index paths and recomputes the
            // issuer-signed `status_ref` over the disclosed value the issuer signed
            // (clear index OR index commitment).
            let (status_ref, _mode) = resolve_status_ref(rev, att_status, &c.0)?;
            // [OPUS-4.8] sq-z8s7 (HolderPoP T3 / B1): select the signed-message
            // variant from the attestation's OPTIONAL fields. A HOLDER-BOUND
            // attestation (the issuer folded a holder-key digest into the signature,
            // the distinct ZKSIG_C4 tag) signs `commitment_message_with_holder(C(G),
            // salt, status_ref, holder_pk_digest)`; a non-holder-bound one signs the
            // status-only `commitment_message_with_status` (ZKSIG_C3). Selecting the
            // right variant HERE is what makes a holder-bound credential pass the
            // main attestation gate AND anchors the attested `holder_pk_digest` in
            // the issuer signature (design §3.3 / §4.3 obligation 1) — the digest the
            // T3 cross-check (`bind_holder_binding`) compares the presented key
            // against is the one the ISSUER signed, never a free prover JSON field. A
            // holder-bound attestation whose `holder_pk_digest` hex is malformed has
            // no reconstructable message => InvalidIssuerSignature (fail-closed).
            let message = match att.holder.as_ref() {
                Some(binding) => {
                    let Some(holder_digest) = binding.digest() else {
                        return Err(CheckError::InvalidIssuerSignature {
                            commitment: c.0.clone(),
                        });
                    };
                    commitment_message_with_holder(
                        &commitment_fr,
                        &salt_fr,
                        &status_ref,
                        &holder_digest,
                    )
                }
                None => commitment_message_with_status(&commitment_fr, &salt_fr, &status_ref),
            };
            let ok = SignatureScheme::from_cryptosuite_iri(&att.cryptosuite).is_some()
                && match (
                    public_key_from_hex(&att.issuer_public_key),
                    signature_from_hex(&att.signature),
                ) {
                    (Some(pk), Some(sig)) => sig_verify(&pk, &message, &sig),
                    _ => false,
                };
            if !ok {
                return Err(CheckError::InvalidIssuerSignature {
                    commitment: c.0.clone(),
                });
            }
            // [OPUS-4.8] codex 2223 LOW / sq-nlulr: record the now-verified salt for
            // this SCAN- or PATH-REFERENCED commitment. Keyed by canonical commitment
            // hex so the same graph referenced by multiple scans/paths records a
            // single entry; the salt-uniqueness gate (step 3) iterates only this set,
            // so an unrelated extra attestation never participates.
            referenced_salt.insert(field_to_hex(&commitment_fr), field_to_hex(&salt_fr));
        }
    }

    // (3) Salt uniqueness (audit #9): no two DISTINCT committed graphs USED BY A
    // VERIFIED SCAN OR PATH sub-proof may share a salt. A reused salt is the Q6
    // cross-graph bnode-correlation channel — a same-label canonical bnode then
    // encodes identically across both graphs. [OPUS-4.8] codex 2223 LOW / sq-nlulr:
    // this check is scoped to `referenced_salt` (commitments an actually-verified
    // scan or path drew from) rather than every `manifest.commitment_attestations`
    // entry. The #9 property only concerns committed graphs a verified sub-proof
    // used, so an UNRELATED extra attestation that happens to reuse a salt must NOT
    // false-reject an otherwise valid proof. Each recorded salt is already
    // issuer-attested (recorded only after the signature verified above). A salt
    // reused across two distinct SCAN- or PATH-referenced commitments still REJECTS
    // — including a scan graph and a single-graph path graph, which is the
    // sq-nlulr cross-graph scan<->path corollary. (Two attestations over the SAME
    // commitment collapse to one entry by the commitment-keyed map and so are
    // fine: the same graph attested twice.)
    let mut salt_to_commitment: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for (commitment_key, salt_key) in &referenced_salt {
        match salt_to_commitment.get(salt_key) {
            Some(prev) if prev != commitment_key => {
                return Err(CheckError::SaltReused {
                    salt: salt_key.clone(),
                });
            }
            _ => {
                salt_to_commitment.insert(salt_key.clone(), commitment_key.clone());
            }
        }
    }
    Ok(())
}

/// The set of commitment-hex keys (canonical `field_to_hex`) for which a CLEAR
/// issuer attestation may be withheld because a HIDDEN-ISSUER proof covers them
/// (sq-xxg). A commitment is hidden-covered iff (a) the relying party ENABLED the
/// hidden-issuer path (`KeySet::with_hidden_issuer_depth`) AND (b) the manifest
/// carries a `hidden_issuer_attestations` entry for it.
///
/// This is a STRUCTURAL coverage set only — it does NOT verify the hidden proof
/// (that is the bb-stage [`bind_hidden_issuer_attestations`], which binds the
/// proof's public `key_set_root` to the authoritative KeySet and its `m` to the
/// issuer-signed message, and rejects a dangling/forged/unreferenced entry). So a
/// commitment in this set is treated as "clear attestation not required" by the
/// structural [`bind_issuer_attestations`], but is still gated by the mandatory,
/// fail-closed bb hidden-issuer verification before `verify_manifest` accepts.
///
/// When the relying party did NOT enable the hidden path, this returns the empty
/// set, so every commitment then requires a clear attestation exactly as before
/// (a manifest carrying hidden entries against a non-enabled policy is separately
/// rejected by `bind_hidden_issuer_attestations` with `HiddenIssuerNotEnabled`).
// [OPUS-4.8] sq-xxg: structural hidden-issuer coverage set (clear-attestation
// optionality). Soundness rests on the bb gate; this only relaxes the structural
// clear-entry requirement and never lets a commitment go uncovered (either-clear-
// or-hidden, never-neither — enforced in `bind_issuer_attestations`).
fn hidden_issuer_covered_commitments(
    manifest: &ProofManifest,
    trusted_key_set: &KeySet,
) -> BTreeSet<String> {
    if trusted_key_set.hidden_issuer_depth().is_none() {
        // The hidden path is disabled; no commitment may withhold its clear
        // attestation. (Any hidden entry present is rejected fail-closed by the
        // bb-stage gate with HiddenIssuerNotEnabled.)
        return BTreeSet::new();
    }
    manifest
        .hidden_issuer_attestations
        .iter()
        .filter_map(|hi| hi.commitment.to_field().map(|f| field_to_hex(&f)))
        .collect()
}

/// Whether a status reference uses the sq-ayv COMMITTED-index path (the clear
/// index is withheld and a hiding `index_commitment` is bound) vs the audit-#12
/// CLEAR-index path. Determined by the ATTESTATION's signed reference
/// (`AttestedStatusRef`), which the issuer controls — never a prover claim alone.
// [OPUS-4.8] sq-ayv.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusRefMode {
    /// Clear-index (audit #12): the issuer signed `status_ref_digest(list, index,
    /// version)`; liveness is the clear-index bit check.
    Clear,
    /// Committed-index (sq-ayv): the issuer signed
    /// `status_ref_commit_digest(list, index_commitment, version)`; liveness is the
    /// hidden-index proof cross-bound to that commitment.
    Committed,
    /// [OPUS-5] sq-kndw: FULLY-COMMITTED (fully-hidden). The issuer signed
    /// `status_ref_fully_committed_digest(ref_commitment, index_commitment)`, which
    /// folds NEITHER a clear list id NOR a clear version — both are committed
    /// inside `ref_commitment`. Liveness is the fully-hidden proof, cross-bound to
    /// BOTH commitments and to the relying party's accepted-set anchor.
    FullyCommitted,
}

/// Resolve the issuer-signed `status_ref` field element AND the disclosure mode
/// from the attestation's signed reference (`att_status`) cross-checked against
/// the disclosed `rev` reference (sq-ayv). Fail-closed:
/// - the ATTESTED reference must be EXACTLY one of clear `index` / committed
///   `index_commitment` (both-set or neither-set => `RevocationReferenceModeInvalid`);
/// - the DISCLOSED `rev` must agree on the same mode and value (a mismatch or a
///   mode disagreement => the named error). The clear-index path stays byte-for-byte
///   what audit #12 did; the committed path recomputes the digest over the disclosed
///   `index_commitment` (which the issuer signed, never the clear index).
///
/// [OPUS-5] sq-kndw: the fully-hidden mode is the THIRD arm. The ATTESTED
/// reference must be EXACTLY one of clear `index` / committed `index_commitment` /
/// fully-committed `ref_commitment + index_commitment`, and the DISCLOSED `rev`
/// must present the SAME mode with the SAME values. In the fully-hidden arm the
/// digest is [`status_ref_fully_committed_digest`], which folds neither the list id
/// nor the version — so `rev.status_list` / `rev.version` MUST be absent (a
/// disclosed IRI or version alongside a fully-hidden reference is a mode
/// disagreement, rejected: it would be a leak the signature does not cover).
///
/// Returns `(status_ref_fr, mode)`.
// [OPUS-4.8] sq-ayv: mode-aware status-reference resolution.
// [OPUS-5] sq-kndw: + the fully-hidden (IRI + version committed) mode.
fn resolve_status_ref(
    rev: &crate::manifest::RevocationStatus,
    att_status: &crate::manifest::AttestedStatusRef,
    commitment_hex: &str,
) -> Result<(Fr, StatusRefMode), CheckError> {
    let mode_invalid = || CheckError::RevocationReferenceModeInvalid {
        commitment: commitment_hex.to_string(),
    };
    let mismatch = || CheckError::RevocationReferenceMismatch {
        commitment: commitment_hex.to_string(),
    };

    let att_fully = att_status.ref_commitment.is_some();
    let att_committed = att_status.index_commitment.is_some() && !att_fully;
    let att_clear = att_status.index.is_some();
    // The ATTESTED reference must be EXACTLY one mode. (`att_fully` implies
    // `index_commitment` too, so it is checked as its own arm first.)
    if att_fully {
        if att_clear {
            return Err(mode_invalid());
        }
    } else if att_committed == att_clear {
        return Err(mode_invalid());
    }

    if att_fully {
        // [OPUS-5] sq-kndw: FULLY-HIDDEN. The issuer signed only the two
        // commitments; the list IRI and the version live inside `ref_commitment`.
        let att_rc_hex = att_status.ref_commitment.as_ref().expect("att_fully");
        // The attestation must carry BOTH commitments — the digest folds both, and
        // an index-commitment-less fully-hidden reference has nothing to bind the
        // proven-unset index to.
        let Some(att_ic_hex) = att_status.index_commitment.as_ref() else {
            return Err(mode_invalid());
        };
        // The DISCLOSED reference must be in the same mode: both commitments
        // present, and NOTHING clear (no IRI, no index, no version). Disclosing any
        // of those would defeat the mode and is not covered by the signature.
        let (Some(rev_rc), Some(rev_ic)) = (&rev.ref_commitment, &rev.index_commitment) else {
            return Err(mode_invalid());
        };
        if rev.index.is_some() || rev.status_list.is_some() || rev.version.is_some() {
            return Err(mode_invalid());
        }
        // The attestation must likewise withhold the version (it is not folded into
        // the digest, so a value there would be unbound metadata).
        if att_status.version.is_some() {
            return Err(mode_invalid());
        }
        // Compare as field elements so 0x-padding cannot slip a different value past.
        let (Some(att_rc_fr), Some(rev_rc_fr)) = (att_rc_hex.to_field(), rev_rc.to_field()) else {
            return Err(mode_invalid());
        };
        let (Some(att_ic_fr), Some(rev_ic_fr)) = (att_ic_hex.to_field(), rev_ic.to_field()) else {
            return Err(mode_invalid());
        };
        if att_rc_fr != rev_rc_fr || att_ic_fr != rev_ic_fr {
            return Err(mismatch());
        }
        let status_ref = status_ref_fully_committed_digest(&att_rc_fr, &att_ic_fr);
        Ok((status_ref, StatusRefMode::FullyCommitted))
    } else if att_committed {
        // Committed-index (sq-ayv). The disclosed reference must withhold the clear
        // index AND disclose a matching commitment + version.
        let att_ic_hex = att_status
            .index_commitment
            .as_ref()
            .expect("att_committed");
        let Some(rev_ic) = &rev.index_commitment else {
            return Err(mode_invalid());
        };
        // A disclosed clear index alongside a committed reference is a mode
        // disagreement (the index must be withheld on the committed path); so is a
        // disclosed ref_commitment (that is the fully-hidden mode's field).
        if rev.index.is_some() || rev.ref_commitment.is_some() {
            return Err(mode_invalid());
        }
        // The committed path still binds a CLEAR list IRI + version, so both must be
        // disclosed (a `None` version is NOT defaulted to 0 — that would silently
        // drop the freshness anchor).
        let (Some(list), Some(rev_version), Some(att_version)) =
            (&rev.status_list, rev.version, att_status.version)
        else {
            return Err(mode_invalid());
        };
        // The disclosed commitment must equal the issuer-signed one (compare as
        // field elements so 0x-padding cannot slip a different value past), and
        // the version must match.
        let (Some(att_ic_fr), Some(rev_ic_fr)) = (att_ic_hex.to_field(), rev_ic.to_field())
        else {
            return Err(mode_invalid());
        };
        if att_ic_fr != rev_ic_fr || rev_version != att_version {
            return Err(mismatch());
        }
        let list_id_fr = status_list_id_to_field(list);
        let status_ref = status_ref_commit_digest(&list_id_fr, &att_ic_fr, att_version);
        Ok((status_ref, StatusRefMode::Committed))
    } else {
        // Clear-index (audit #12), unchanged. The disclosed reference must carry the
        // same clear index + version and NOT carry a commitment.
        let att_index = att_status.index.expect("att_clear");
        let Some(rev_index) = rev.index else {
            return Err(mismatch());
        };
        if rev.index_commitment.is_some() || rev.ref_commitment.is_some() {
            return Err(mode_invalid());
        }
        let (Some(list), Some(rev_version), Some(att_version)) =
            (&rev.status_list, rev.version, att_status.version)
        else {
            return Err(mode_invalid());
        };
        if rev_index != att_index || rev_version != att_version {
            return Err(mismatch());
        }
        let list_id_fr = status_list_id_to_field(list);
        let status_ref = status_ref_digest(&list_id_fr, rev_index, att_version);
        Ok((status_ref, StatusRefMode::Clear))
    }
}

/// Stage 2f: revocation / freshness check (audit #12). PRECONDITION:
/// [`bind_issuer_attestations`] has already run and accepted the manifest, so —
/// for every scan-covering commitment — the disclosed `manifest.revocation`
/// reference has been cross-checked against, and bound under, the issuer
/// signature (its index/version equal the signed [`AttestedStatusRef`] and the
/// `status_ref_digest` over `(H(list IRI), index, version)` is the one the
/// issuer signed). So here `manifest.revocation` is the ISSUER'S reference, not
/// a prover claim.
///
/// This stage then validates the credential's LIVENESS against the AUTHORITATIVE
/// status-list snapshot (resolved by the relying party into [`RevocationPolicy`],
/// NOT taken from the prover) and the relying party's freshness policy:
/// - resolve the AUTHORITATIVE [`StatusListSnapshot`] matching the reference's
///   `(status_list, version)` from the policy — NO authoritative snapshot REJECTS
///   ([`CheckError::StatusSnapshotMissing`]);
/// - the version MUST be within the policy's freshness window — a stale
///   (or future-dated) reference REJECTS ([`CheckError::StatusListStale`]);
/// - if the prover ALSO disclosed a snapshot for the SAME `(list, version)` it
///   MUST byte-equal the authoritative one — a disagreeing prover snapshot is a
///   tamper signal and REJECTS ([`CheckError::StatusSnapshotTampered`]);
/// - the credential's status bit at `index` IN THE AUTHORITATIVE SNAPSHOT MUST be
///   UNSET — a set bit means the credential is REVOKED/SUSPENDED and REJECTS
///   ([`CheckError::CredentialRevoked`]).
///
/// # The authenticated-bits fix (Option B) — load-bearing
/// The bit decision reads `authoritative.bit(index)`, where `authoritative` is
/// the relying party's OWN snapshot for the reference's `(list, version)` — NEVER
/// `manifest.status_snapshots`. The issuer signature binds the reference (which
/// list / slot / version) but NOT the bit values; if the verifier read the
/// prover's snapshot bytes for the bit, a prover holding a genuine reference
/// could attach a forged all-zero snapshot and a REVOKED credential would verify
/// (the re-audit break). Sourcing the bitstring externally — exactly as the
/// trusted key-set `K` is external (audit #3) — moves the liveness decision off
/// prover-controlled bytes. The prover's snapshot, if present for the referenced
/// key, is only checked for byte-equality with the authoritative one (a tamper
/// tripwire); it is otherwise ignored.
///
/// # Mandatory / fail-closed
/// This runs whenever the manifest carries a `revocation` reference, which —
/// because a scan-covering attestation MUST bind one (audit #12 in
/// `bind_issuer_attestations`) — is EVERY manifest carrying a scan. A
/// status-bound credential cannot reach acceptance without passing this. (A
/// manifest with no scans and no revocation has nothing to revoke; the check is
/// vacuous, not skipped — there is no scan-covering commitment whose liveness is
/// in question.) A relying party that has resolved NO authoritative snapshot for
/// the referenced list/version rejects (it cannot authenticate the liveness view).
///
/// # Privacy — the hidden-index upgrade (sq-3e5 / sq-h2v) and its residual gap
/// `index` is matched against the authoritative bitstring in the clear HERE, so
/// this clear-index path reveals WHICH list slot the credential occupies (a
/// linkability handle). The IN-CIRCUIT hidden-index inclusion + bit-unset proof is
/// now IMPLEMENTED ([`bind_hidden_revocation`] + the `revoke_unset_d{depth}`
/// circuit): it proves "the bit at my HIDDEN index in the tree rooted at the
/// relying party's authoritative root is unset" in zero knowledge, disclosing
/// neither the index nor the other bits, and is bound to the verifier's OWN
/// authoritative Merkle root (so the trust anchor is preserved). When a manifest
/// carries `hidden_revocation` and the policy enables it, that proof is the
/// index-hiding liveness evidence.
///
/// # sq-ayv: the residual index leak is now CLOSED (committed-index path)
/// The index-leak gap is resolved. A credential may use the COMMITTED-index path:
/// the issuer signs `status_ref_commit_digest(H(list), index_commitment, version)`
/// — a HIDING commitment to the index, not the clear index — so the clear index is
/// withheld from every signed object AND disclosed field (`RevocationStatus.index`
/// is `None`). In that mode this function does NOT run the clear bit check (there
/// is no clear index); instead it REQUIRES a hidden-index proof
/// ([`CheckError::HiddenRevocationRequired`]) whose in-circuit cross-binding ties
/// the proven-unset index to the issuer-signed commitment ([`bind_hidden_revocation`]).
/// So a hidden-revocation presentation discloses neither the index nor the liveness
/// bit, while revocation is still checked against the authoritative root (never
/// skipped). The CLEAR-index path below remains unchanged for clear references (the
/// always-on soundness floor); a relying party that does not need index-hiding can
/// keep using it.
///
/// HONEST REMAINING DISCLOSURE ON *THIS* PATH: on the committed-index path the
/// status-list IRI and the `version` are still disclosed in the clear (both
/// issuer-bound); only the index + liveness bit are hidden.
/// [OPUS-5] sq-kndw: that leak is now CLOSED by the THIRD mode — a FULLY-HIDDEN
/// reference (`ref_commitment` present; `status_list` / `index` / `version` all
/// `None`) discloses none of them, and this function routes it to
/// [`bind_fully_hidden_revocation`] instead of resolving a snapshot by name. The
/// committed-index path below is UNCHANGED and remains available for relying
/// parties that do not need IRI/version hiding.
/// [OPUS-4.8] sq-hwe: the host Merkle builder is now SPARSE
/// ([`crate::revocation::merkle_root`]) — `O(set-bits * depth)`, independent of
/// `2^depth` — so it no longer bounds the list size; the remaining size bound is
/// the single COMPILED `revoke_unset_d10` circuit member (compiling a deeper member
/// is mechanical, the relation is depth-generic — tracked as follow-up).
// [OPUS-4.8] audit #12: verifier-side revocation / freshness check.
// [OPUS-4.8] audit #12 re-audit (Option B): the bit decision reads the
// AUTHORITATIVE (relying-party-resolved) snapshot, never the prover's bytes.
fn bind_revocation(
    manifest: &ProofManifest,
    policy: &RevocationPolicy,
) -> Result<(), CheckError> {
    let Some(rev) = &manifest.revocation else {
        // No revocation reference. `bind_issuer_attestations` guarantees that a
        // scan-covering commitment forces one to be present (and bound), so
        // reaching here means there is no scan-covering credential whose liveness
        // is in question — the check is vacuously satisfied.
        return Ok(());
    };
    // [OPUS-5] sq-kndw: FULLY-HIDDEN reference. There is no clear `(list, version)`
    // to resolve a snapshot by — that is the whole point of the mode — so the
    // snapshot-shaped checks below cannot run and MUST NOT be skipped silently.
    // Everything they would have established is instead established by
    // `bind_fully_hidden_revocation` (bb stage) against the relying party's
    // accepted-set anchor:
    //   * freshness  -> membership is restricted to the policy's freshness-curated
    //                   window (`accepted_entries`), plus the in-circuit
    //                   `version >= min_version` on the policy's own public floor;
    //   * authenticity of the liveness view -> the status-list root the fold runs
    //                   against is bound INSIDE the accepted-set leaf, and that
    //                   leaf's tree root is derived by the relying party from its
    //                   OWN snapshots;
    //   * bit-unset  -> the in-circuit `bit == 0` assertion.
    // So here we only enforce the fail-closed structural requirement: the proof
    // must be present. Its absence is never a silent skip.
    if rev.ref_commitment.is_some() {
        // A fully-hidden reference must not ALSO disclose the clear fields (the
        // signature does not cover them in this mode). `resolve_status_ref` already
        // enforces this for every scan-covering commitment; repeat it here so a
        // `revocation` with no scan covering it cannot smuggle a half-hidden shape.
        if rev.index.is_some() || rev.status_list.is_some() || rev.version.is_some() {
            return Err(CheckError::RevocationReferenceModeInvalid {
                commitment: String::new(),
            });
        }
        if rev.index_commitment.is_none() {
            return Err(CheckError::RevocationReferenceModeInvalid {
                commitment: String::new(),
            });
        }
        if manifest.fully_hidden_revocation.is_none() {
            return Err(CheckError::FullyHiddenRevocationRequired);
        }
        // DISCLOSURE FLOOR: a prover snapshot names its (list, version) in the
        // clear. On this mode it is both useless (the gate reads the relying
        // party's own curated snapshots) and self-defeating, so it is refused
        // rather than ignored — a silently-ignored leak is still a leak.
        if !manifest.status_snapshots.is_empty() {
            return Err(CheckError::FullyHiddenRevocationSnapshotDisclosed);
        }
        return Ok(());
    }
    // A fully-hidden PROOF without a fully-hidden REFERENCE has no issuer-signed
    // commitments to bind to — reject rather than leave it unchecked.
    if manifest.fully_hidden_revocation.is_some() {
        return Err(CheckError::FullyHiddenRevocationUnbound);
    }
    // The clear + committed paths both bind a CLEAR list IRI and version. A `None`
    // in either is a malformed mode — NOT a default-to-0/empty, which would drop
    // the freshness anchor or resolve the wrong snapshot.
    let (Some(list), Some(version)) = (rev.status_list.clone(), rev.version) else {
        return Err(CheckError::RevocationReferenceModeInvalid {
            commitment: String::new(),
        });
    };
    // Freshness FIRST, on the (issuer-bound) reference's version: a stale (or
    // future-dated) reference is rejected so a revoked-since-snapshot credential
    // cannot slip through on an old "active" view, regardless of whether the
    // relying party still holds that old version's snapshot.
    if !policy.is_fresh(version) {
        return Err(CheckError::StatusListStale { status_list: list, version });
    }
    // Resolve the AUTHORITATIVE snapshot from the relying party's policy (NOT the
    // prover). No authoritative snapshot for the referenced (list, version) =>
    // the verifier cannot authenticate the liveness view => REJECT fail-closed.
    let Some(authoritative) = policy.authoritative_snapshot(&list, version) else {
        return Err(CheckError::StatusSnapshotMissing { status_list: list, version });
    };
    // [OPUS-4.8] sq-ayv: the index-disclosure mode. A COMMITTED-index reference
    // (clear index withheld, `index_commitment` present) MOVES the liveness
    // decision to the hidden-index proof (`bind_hidden_revocation`, bb stage),
    // which proves bit-unset against THIS authoritative root and cross-binds the
    // index commitment. The clear bit-read below cannot run (there is no clear
    // index), so we REQUIRE a hidden-revocation proof here (fail-closed —
    // revocation is never skipped) and defer the bit decision to it.
    match (rev.index, &rev.index_commitment) {
        (None, Some(_)) => {
            // Committed-index path: a hidden-revocation proof MUST be present (its
            // cryptographic bit-unset + cross-binding verification is the liveness
            // gate, run in the bb stage). Without it, liveness is unchecked.
            if manifest.hidden_revocation.is_none() {
                return Err(CheckError::HiddenRevocationRequired { proof: 0 });
            }
            // The authoritative snapshot's existence + freshness are already
            // checked above; the bit-unset decision happens in bind_hidden_revocation
            // against the root derived from this same authoritative snapshot.
        }
        (Some(index), None) => {
            // Clear-index path (audit #12), unchanged. Liveness (THE security
            // decision): the credential's status bit must be UNSET IN THE
            // AUTHORITATIVE snapshot (out-of-range reads as SET — fail closed). This
            // reads the relying party's OWN authenticated bytes, so the verdict is
            // identical regardless of what snapshot the prover attached.
            if authoritative.bit(index) {
                return Err(CheckError::CredentialRevoked { status_list: list.clone(), index });
            }
        }
        _ => {
            // Neither or both disclosed: a malformed reference mode. (The
            // attestation-side check `resolve_status_ref` already enforces the mode
            // for status-bound scans; this guards a `revocation` with no scan
            // covering it, fail-closed.)
            return Err(CheckError::RevocationReferenceModeInvalid { commitment: list });
        }
    }
    // Tamper tripwire: the authoritative bit is UNSET, but if the prover ALSO
    // disclosed a snapshot for this exact (list, version) it MUST byte-equal the
    // authoritative one. The liveness verdict above did not depend on it; this
    // only surfaces a disagreeing prover snapshot as an explicit forgery signal
    // (e.g. the prover lied the OTHER way — claimed REVOKED — or doctored
    // unrelated bits) rather than silently accepting it.
    // [OPUS-4.8] audit #12 re-audit: prover snapshot is a tamper tripwire only.
    // roborev #2263: check EVERY snapshot matching (list, version), not just the first.
    // A prover can attach duplicate entries — a benign one that byte-equals the
    // authoritative snapshot followed by a forged one — and a `.find()` that stops at the
    // first match would never inspect the forgery. `any()` over all matches trips on ANY
    // disagreeing snapshot.
    if manifest
        .status_snapshots
        .iter()
        .any(|s| s.status_list == list && s.version == version && s.bits != authoritative.bits)
    {
        return Err(CheckError::StatusSnapshotTampered { status_list: list, version });
    }
    Ok(())
}

/// The HIDDEN-INDEX revocation check (sq-3e5 / sq-h2v) — the privacy upgrade
/// over [`bind_revocation`]'s clear-index path. Runs only the cryptographic gate
/// for `manifest.hidden_revocation`; the structural pre-conditions (the
/// issuer-bound reference, freshness) are still enforced by
/// [`bind_issuer_attestations`] + [`bind_revocation`] in the structural prefilter.
///
/// # What it proves and what stays hidden
/// The prover supplies a `revoke_unset_d{depth}` bb proof whose PUBLIC inputs are
/// the verifier's `challenge` and a status-list Merkle `root`; the holder's
/// INDEX, the leaf bit, and the authentication path are PRIVATE. The circuit
/// proves "the bit at my hidden index in the tree rooted at `root` is UNSET" — so
/// a relying party learns the credential is live WITHOUT learning which list slot
/// it occupies (closing the clear-index linkability channel).
///
/// # Trust anchor (preserves the audit-#12 re-audit fix — load-bearing)
/// `root` is a prover-committed public input, NOT trusted as a prover claim. The
/// verifier derives the AUTHORITATIVE root from its OWN [`StatusListSnapshot`] for
/// the credential's (issuer-bound, freshness-checked) `(list, version)` — exactly
/// the snapshot [`bind_revocation`] uses, resolved out of band like the trusted
/// key-set `K` — at the policy's `hidden_index_depth`, and REQUIRES the proof's
/// public root to byte-equal it. A prover that proves bit-unset against a FORGED
/// all-zero tree (its own root) fails this equality: the liveness fact is bound to
/// the relying party's authenticated status bytes, never the prover's. (A
/// genuinely REVOKED credential cannot even produce the proof — the in-circuit
/// `bit == 0` assertion is unsatisfiable for a set bit — and additionally the
/// authoritative root over the relying party's snapshot, whose bit IS set, would
/// differ from any all-zero forgery.)
///
/// # Fail-closed
/// - No `manifest.hidden_revocation` => nothing to check here (the clear-index
///   [`bind_revocation`] remains the liveness gate); returns `Ok`.
/// - A proof present but the policy has NOT enabled the hidden-index path
///   (`hidden_index_depth == None`) => REJECT ([`CheckError::HiddenRevocationNotEnabled`]):
///   the verifier will not accept a root it cannot itself derive.
/// - A `depth` mismatch, an unresolvable authoritative root, a root mismatch, a
///   malformed blob, or a bb rejection all REJECT.
///
/// PRECONDITION: `bind_revocation` has already validated the issuer-bound
/// reference + freshness for `manifest.revocation`, so the `(list, version)` used
/// to resolve the authoritative snapshot here is the issuer's, not a prover claim.
///
/// # Scope (honest, see [`crate::revocation`])
/// [OPUS-4.8] sq-hwe: the authoritative root is now derived with the SPARSE
/// [`merkle_root`] builder (`O(set-bits * depth)`, independent of `2^depth`, with
/// roots BIT-IDENTICAL to the old dense fold), so the host side scales to
/// production list sizes (2^17+ slots) WITHOUT materialising `2^depth` leaves. The
/// remaining size bound is that only the `revoke_unset_d10` member (depth 10, up to
/// 1024 indices) is COMPILED — the circuit relation is depth-generic, so compiling
/// a deeper member is mechanical (tracked as follow-up). The soundness of the
/// binding (root equality + bb verify + in-circuit bit-unset) holds at any supported
/// depth. The status-list IRI + version are still disclosed in the clear on THIS
/// (committed-index) path; [OPUS-5] sq-kndw hides them too on the fully-hidden path
/// ([`bind_fully_hidden_revocation`]).
// [OPUS-4.8] sq-3e5 + sq-h2v: hidden-index revocation cryptographic gate.
fn bind_hidden_revocation(
    manifest: &ProofManifest,
    policy: &RevocationPolicy,
    prover: &CircuitProver,
    work_dir: &Path,
    challenge: &FieldHex,
) -> Result<(), CheckError> {
    let Some(hidden) = &manifest.hidden_revocation else {
        // No hidden-index proof; the clear-index path (bind_revocation) is the
        // liveness gate. Nothing to verify here.
        return Ok(());
    };
    // The relying party must have OPTED IN to the hidden-index path; otherwise it
    // has no authoritative root to bind the proof to and rejects fail-closed.
    let Some(depth) = policy.hidden_index_depth() else {
        return Err(CheckError::HiddenRevocationNotEnabled);
    };
    if hidden.depth != depth {
        return Err(CheckError::HiddenRevocationDepthMismatch {
            declared: hidden.depth,
            policy: depth,
        });
    }
    // The issuer-bound reference (validated by bind_revocation upstream) names the
    // (list, version) whose AUTHORITATIVE snapshot we derive the root from. Without
    // a reference there is no credential whose liveness is in question.
    let Some(rev) = &manifest.revocation else {
        return Ok(());
    };
    // [OPUS-5] sq-kndw: this gate is the COMMITTED-index one and needs a clear
    // `(list, version)` to name the snapshot. A fully-hidden reference has neither
    // (by design) and is handled by `bind_fully_hidden_revocation`; `bind_revocation`
    // has already required that mode to carry its own proof, so refusing here is
    // fail-closed, not a skip. A `hidden_revocation` proof attached to a
    // fully-hidden reference has no clear root to bind to and is rejected.
    let (Some(list), Some(version)) = (rev.status_list.as_deref(), rev.version) else {
        return Err(CheckError::HiddenRevocationRootUnavailable {
            status_list: String::new(),
            version: 0,
        });
    };
    let Some(authoritative) = policy.authoritative_snapshot(list, version) else {
        return Err(CheckError::HiddenRevocationRootUnavailable {
            status_list: list.to_string(),
            version,
        });
    };
    // Derive the AUTHORITATIVE root from the relying party's OWN snapshot (the
    // trust anchor). This is what the proof's public root must equal.
    let Some(auth_root) = merkle_root(authoritative, depth) else {
        return Err(CheckError::HiddenRevocationRootUnavailable {
            status_list: list.to_string(),
            version,
        });
    };
    // The proof's declared public root must byte-equal the authoritative root. A
    // malformed declared root fails closed.
    let Some(declared_root) = hidden.root.to_field() else {
        return Err(CheckError::HiddenRevocationRootMismatch);
    };
    if declared_root != auth_root {
        return Err(CheckError::HiddenRevocationRootMismatch);
    }

    // [OPUS-4.8] sq-ayv: the index commitment the proof cross-binds. The TRUST
    // ANCHOR is the ISSUER-SIGNED commitment in `manifest.revocation.index_commitment`
    // (validated under the issuer signature in `bind_issuer_attestations` /
    // `resolve_status_ref`), NOT the prover's declared `hidden.index_commitment`.
    // The `revoke_unset_d{depth}` member ALWAYS exposes an index_commitment public
    // input now (sq-ayv), so a hidden-revocation proof REQUIRES an issuer-signed
    // index commitment to bind it to — a hidden proof without one is rejected
    // fail-closed (it would be a free-floating bit-unset over an unbound index).
    let Some(rev_ic_hex) = &rev.index_commitment else {
        return Err(CheckError::HiddenRevocationIndexCommitmentMismatch);
    };
    let Some(auth_index_commitment) = rev_ic_hex.to_field() else {
        return Err(CheckError::HiddenRevocationIndexCommitmentMismatch);
    };
    // The proof's DECLARED public index commitment must byte-equal the issuer-signed
    // one (the prover cannot prove against a commitment the issuer did not sign).
    let Some(declared_ic) = hidden
        .index_commitment
        .as_ref()
        .and_then(|h| h.to_field())
    else {
        return Err(CheckError::HiddenRevocationIndexCommitmentMismatch);
    };
    if declared_ic != auth_index_commitment {
        return Err(CheckError::HiddenRevocationIndexCommitmentMismatch);
    }

    // Cryptographic gate: reconstruct the public inputs (challenge, root,
    // index_commitment) from the AUTHORITATIVE root + the ISSUER-SIGNED index
    // commitment (NOT the prover's declared bytes — byte-equal above, but we feed
    // our own to be end-to-end authentic), recompute the canonical vk, and bb
    // verify. The prover's bundled vk / public_inputs are never trusted (audit #2
    // discipline, mirrored here). The in-circuit cross-binding asserts the proof's
    // index_commitment is a commitment to the SAME index proven bit-unset, so the
    // index the issuer committed to is the one shown active.
    let blob = hex_decode(&hidden.proof_hex).ok_or(CheckError::HiddenRevocationMalformedProof)?;
    let art = decode_artifacts(&blob).ok_or(CheckError::HiddenRevocationMalformedProof)?;

    // Public-input layout for revoke_unset_d{depth} main: challenge, root,
    // index_commitment (three 32-byte BE field words, declaration order).
    let mut reconstructed: Vec<u8> = Vec::with_capacity(96);
    let challenge_fr = challenge
        .to_field()
        .ok_or(CheckError::HiddenRevocationMalformedProof)?;
    reconstructed.extend_from_slice(&field_to_be_bytes_32(&challenge_fr));
    reconstructed.extend_from_slice(&field_to_be_bytes_32(&auth_root));
    reconstructed.extend_from_slice(&field_to_be_bytes_32(&auth_index_commitment));
    if reconstructed != art.public_inputs {
        // The proof's committed public inputs (challenge + root + index_commitment)
        // do not match the verifier's nonce + authoritative root + issuer-signed
        // commitment. Diagnose the index-commitment word distinctly.
        let pi = &art.public_inputs;
        if pi.len() == 96 && pi[64..96] != field_to_be_bytes_32(&auth_index_commitment) {
            return Err(CheckError::HiddenRevocationIndexCommitmentMismatch);
        }
        return Err(CheckError::HiddenRevocationRootMismatch);
    }

    let id = CircuitId::RevokeUnset { depth };
    let sub_work = work_dir.join("hidden_revocation");
    let canonical_vk = prover
        .canonical_vk(&id, &sub_work.join("vk"))
        .map_err(CheckError::Driver)?;
    let ok = prover
        .verify_with(&art.proof, &reconstructed, &canonical_vk, &sub_work.join("verify"))
        .map_err(CheckError::Driver)?;
    if !ok {
        return Err(CheckError::HiddenRevocationProofRejected);
    }
    Ok(())
}

/// [OPUS-5] sq-kndw: the FULLY-HIDDEN revocation cryptographic gate — the privacy
/// upgrade over [`bind_hidden_revocation`], closing the sq-6qe status-list
/// IRI + version disclosure. Implements
/// `research/zk-statuslist-hide-iri-version.md` §3 sub-option A, verifier side.
///
/// # What it proves and what stays hidden
/// The prover supplies a `revoke_hidden_ref_d{depth}_a{set_depth}` bb proof whose
/// PUBLIC inputs are the verifier's `challenge`, the issuer-signed
/// `ref_commitment` and `index_commitment`, the relying party's
/// `accepted_set_root`, and its public `min_version` floor. Everything else — the
/// list id, the version, both blindings, the status-list Merkle root, the
/// accepted-set slot and path, the holder's index, the leaf bit and the
/// status-list path — is PRIVATE. So the relying party learns only:
///
/// > "some `(list, version)` in MY committed accepted set, at or above MY public
/// > epoch floor, has the index the issuer committed to for this credential
/// > UNSET."
///
/// It learns neither WHICH list, WHICH publication epoch, WHICH slot, nor the
/// liveness bit of any other credential.
///
/// # Trust anchor (the audit-#12 anchor, moved behind a commitment — load-bearing)
/// `accepted_set_root` and `min_version` are prover-committed public inputs but
/// are NOT trusted as claims. Both are derived from the relying party's OWN
/// [`RevocationPolicy`]: the accepted-set root over its freshness-curated
/// `(list, version, status_list_root)` entries, and its own epoch floor. The
/// declared values are byte-matched, and then the public-input vector fed to `bb`
/// is rebuilt from the verifier's OWN values (never the prover's bytes) — the same
/// discipline the clear and committed-index gates use. A prover that proves
/// membership in its OWN forged accepted set fails the equality.
///
/// Crucially, each accepted-set leaf binds `(list_id, version, status_list_root)`
/// ATOMICALLY, so the `status_list_root` the in-circuit bit-unset fold runs against
/// is the one the RELYING PARTY published for that hidden `(list, version)` — a
/// prover cannot pair list₁'s identity with list₂'s (all-zero) root. That is what
/// lets the root stay private without losing the audit-#12 re-audit fix.
///
/// # Freshness survives the move behind the commitment
/// [`RevocationPolicy::accepted_entries`] only admits versions inside
/// `[min_version, now]`, so a stale or future-dated version is not a leaf and no
/// membership proof exists for it. The in-circuit `version >= min_version` is
/// defence-in-depth on top of that, not the only check.
///
/// # Re-blinding / linkage single-use (the privacy guarantee depends on it)
/// `(ref_commitment, index_commitment)` is a stable per-issuance pair. Presenting
/// it twice hands the relying party a perfect correlation handle and voids the
/// whole point of the mode (design §4). This gate therefore records a
/// DOMAIN-SEPARATED linkage tag `h3(ZKLINK, ref_commitment, index_commitment)` in
/// the same durable [`SeenNonces`] store the audit-#4 nonce replay defence uses,
/// and rejects a repeat with [`CheckError::FullyHiddenRevocationLinkageReplay`].
/// The tag is a Poseidon2 image under a tag distinct from every `ZKSIG_*` /
/// verifier-nonce value, so it can never collide with a real nonce.
///
/// HONEST LIMIT: single-use enforcement only helps against an HONEST relying
/// party — a malicious one simply skips it, and has already observed the pair. The
/// real fix is upstream (the issuer re-signs freshly-blinded commitments per
/// presentation, or a re-randomisable commitment+signature scheme sparq does not
/// implement). It is enforced anyway because it makes the requirement operational
/// rather than advisory, and turns silent linkability into a visible rejection.
///
/// # Fail-closed
/// - No `manifest.fully_hidden_revocation` => nothing to check here; the reference
///   modes that need it already required it in [`bind_revocation`]. `Ok`.
/// - A proof with no fully-hidden reference => `FullyHiddenRevocationUnbound`
///   (also caught structurally upstream).
/// - Path not enabled / no derivable anchor => `FullyHiddenRevocationNotEnabled`.
/// - Depth mismatch or an uncompiled member => `FullyHiddenRevocationDepthMismatch`.
/// - Anchor, commitment, linkage, blob, or bb failure => the named error.
///
/// NOT externally audited (sq-qhy4). Research-grade; no soundness / ZK-privacy
/// property is asserted as achieved.
// [OPUS-5] sq-kndw: fully-hidden revocation cryptographic gate.
fn bind_fully_hidden_revocation(
    manifest: &ProofManifest,
    policy: &RevocationPolicy,
    prover: &CircuitProver,
    work_dir: &Path,
    challenge: &FieldHex,
    seen: &dyn SeenNonces,
) -> Result<(), CheckError> {
    let Some(fh) = &manifest.fully_hidden_revocation else {
        return Ok(());
    };
    // The ISSUER-SIGNED commitments are the trust anchor for the cross-binding —
    // taken from `manifest.revocation` (validated under the issuer signature by
    // `bind_issuer_attestations` / `resolve_status_ref`), never the prover's
    // declared copies in `fh`.
    let Some(rev) = &manifest.revocation else {
        return Err(CheckError::FullyHiddenRevocationUnbound);
    };
    let (Some(rc_hex), Some(ic_hex)) = (&rev.ref_commitment, &rev.index_commitment) else {
        return Err(CheckError::FullyHiddenRevocationUnbound);
    };
    let (Some(auth_ref_commitment), Some(auth_index_commitment)) =
        (rc_hex.to_field(), ic_hex.to_field())
    else {
        return Err(CheckError::FullyHiddenRevocationMalformedProof);
    };

    // The relying party must have OPTED IN to both halves of the anchor: the
    // status-list depth (each accepted entry's root is derived at it) and the
    // accepted-set depth (the tree those entries are folded into).
    let (Some(depth), Some(set_depth)) = (policy.hidden_index_depth(), policy.accepted_set_depth())
    else {
        return Err(CheckError::FullyHiddenRevocationNotEnabled);
    };
    // The declared depths must match the policy AND name a COMPILED member —
    // `derive_revoke_hidden_ref_id` is the single source of the family list, so an
    // unbuilt (depth, set_depth) is a clean refusal, not a proof attempt against a
    // circuit that does not exist.
    let depth_mismatch = || CheckError::FullyHiddenRevocationDepthMismatch {
        declared_depth: fh.depth,
        declared_set_depth: fh.set_depth,
        policy_depth: depth,
        policy_set_depth: set_depth,
    };
    if fh.depth != depth || fh.set_depth != set_depth {
        return Err(depth_mismatch());
    }
    let Some(id) = crate::build::derive_revoke_hidden_ref_id(depth, set_depth) else {
        return Err(depth_mismatch());
    };

    // Derive BOTH public anchors from the relying party's own curated policy.
    let Some(auth_accepted_root) = policy.accepted_set_root() else {
        return Err(CheckError::FullyHiddenRevocationNotEnabled);
    };
    let auth_min_version = policy.min_version();
    // The declared values must byte-equal ours (field comparison so 0x-padding
    // cannot slip a different value past). We then feed OUR values to bb.
    let Some(declared_root) = fh.accepted_set_root.to_field() else {
        return Err(CheckError::FullyHiddenRevocationMalformedProof);
    };
    if declared_root != auth_accepted_root || fh.min_version != auth_min_version {
        return Err(CheckError::FullyHiddenRevocationAnchorMismatch);
    }
    // The declared commitments must byte-equal the ISSUER-SIGNED ones.
    let (Some(declared_rc), Some(declared_ic)) =
        (fh.ref_commitment.to_field(), fh.index_commitment.to_field())
    else {
        return Err(CheckError::FullyHiddenRevocationMalformedProof);
    };
    if declared_rc != auth_ref_commitment || declared_ic != auth_index_commitment {
        return Err(CheckError::FullyHiddenRevocationCommitmentMismatch);
    }

    // Re-blinding enforcement: the (ref_commitment, index_commitment) pair is
    // single-use. Recorded BEFORE the bb call, mirroring the audit-#4 nonce
    // burn-on-presentation policy — a rejected presentation must not be a free
    // retry that lets an attacker probe with the same linkage handle.
    let linkage = VerifierNonce::from_field(linkage_tag(
        &auth_ref_commitment,
        &auth_index_commitment,
    ));
    if !seen.record_fresh(&linkage) {
        return Err(CheckError::FullyHiddenRevocationLinkageReplay);
    }

    let blob =
        hex_decode(&fh.proof_hex).ok_or(CheckError::FullyHiddenRevocationMalformedProof)?;
    let art = decode_artifacts(&blob).ok_or(CheckError::FullyHiddenRevocationMalformedProof)?;

    // Public-input layout for revoke_hidden_ref_d{depth}_a{set_depth} main, in
    // DECLARATION order: challenge, ref_commitment, index_commitment,
    // accepted_set_root, min_version — five 32-byte BE field words (the `u64`
    // `min_version` is a field element in the ACIR public-input vector like every
    // other input; `full_manifest_fully_hidden_revocation` pins this empirically
    // against a real bb proof).
    let challenge_fr = challenge
        .to_field()
        .ok_or(CheckError::FullyHiddenRevocationMalformedProof)?;
    let mut reconstructed: Vec<u8> = Vec::with_capacity(160);
    reconstructed.extend_from_slice(&field_to_be_bytes_32(&challenge_fr));
    reconstructed.extend_from_slice(&field_to_be_bytes_32(&auth_ref_commitment));
    reconstructed.extend_from_slice(&field_to_be_bytes_32(&auth_index_commitment));
    reconstructed.extend_from_slice(&field_to_be_bytes_32(&auth_accepted_root));
    reconstructed.extend_from_slice(&field_to_be_bytes_32(&Fr::from(auth_min_version)));
    if reconstructed != art.public_inputs {
        // Diagnose WHICH word diverged, so a genuine misconfiguration is not
        // reported as a forgery (and vice versa). A wrong arity is malformed.
        let pi = &art.public_inputs;
        if pi.len() != 160 {
            return Err(CheckError::FullyHiddenRevocationMalformedProof);
        }
        if pi[0..32] != field_to_be_bytes_32(&challenge_fr) {
            // The proof commits a DIFFERENT challenge than this verifier's nonce
            // (the manifest's declared binding was already checked equal to the
            // nonce upstream) — a proof minted for another session.
            return Err(CheckError::FullyHiddenRevocationProofRejected);
        }
        if pi[32..64] != field_to_be_bytes_32(&auth_ref_commitment)
            || pi[64..96] != field_to_be_bytes_32(&auth_index_commitment)
        {
            return Err(CheckError::FullyHiddenRevocationCommitmentMismatch);
        }
        // Remaining: the accepted-set root and/or the epoch floor.
        return Err(CheckError::FullyHiddenRevocationAnchorMismatch);
    }

    let sub_work = work_dir.join("fully_hidden_revocation");
    let canonical_vk = prover
        .canonical_vk(&id, &sub_work.join("vk"))
        .map_err(CheckError::Driver)?;
    let ok = prover
        .verify_with(&art.proof, &reconstructed, &canonical_vk, &sub_work.join("verify"))
        .map_err(CheckError::Driver)?;
    if !ok {
        return Err(CheckError::FullyHiddenRevocationProofRejected);
    }
    Ok(())
}

/// [OPUS-5] sq-kndw: the DOMAIN-SEPARATED single-use tag for a fully-hidden
/// revocation presentation's `(ref_commitment, index_commitment)` linkage handle.
///
/// `Poseidon2([ZKLINK, ref_commitment, index_commitment])`. The tag
/// (`0x5a4b_4c49_4e4b_5f31` = `"ZKLINK_1"`) is distinct from every `ZKSIG_*`
/// domain in [`sparq_zk::sig`], and the value is a Poseidon2 IMAGE, so recording
/// it in the shared [`SeenNonces`] store cannot collide with (or burn) a real
/// verifier nonce except with negligible probability.
// [OPUS-5] sq-kndw: re-blinding / linkage single-use tag.
fn linkage_tag(ref_commitment: &Fr, index_commitment: &Fr) -> Fr {
    const DOMAIN_LINKAGE: u64 = 0x5a4b_4c49_4e4b_5f31; // "ZKLINK_1"
    sparq_zk::poseidon2::hash(&[Fr::from(DOMAIN_LINKAGE), *ref_commitment, *index_commitment])
}

/// Stage 3b (sq-z9l): the HIDDEN-ISSUER attestation cryptographic gate — the
/// privacy upgrade over [`bind_issuer_attestations`]'s clear-key check. Runs only
/// the cryptographic gate for `manifest.hidden_issuer_attestations`; the
/// structural pre-conditions (the issuer-bound reference + salt + clear-key
/// attestation) are still enforced by the prefilter.
///
/// # What it proves and what stays hidden
/// Each entry supplies a `hidden_issuer_d{depth}` bb proof whose PUBLIC inputs are
/// the verifier's `challenge`, the commitment message `m`, and the key-set Merkle
/// `key_set_root`; the issuer key, the signature `(R, s)`, the challenge-reduction
/// witness, and the membership index/path are PRIVATE. The circuit proves "this
/// `m` was signed by SOME issuer whose key is in the tree rooted at `key_set_root`"
/// — so the relying party learns a TRUSTED authority vouched for the commitment
/// WITHOUT learning WHICH (closing the clear-key deanonymisation channel).
///
/// # Trust anchor (preserves the audit #3 external-K anchor — load-bearing)
/// `key_set_root` is a prover-committed public input, NOT trusted as a claim. The
/// verifier derives the AUTHORITATIVE root from its OWN [`KeySet`] (canonical
/// order, the same external anchor `bind_issuer_attestations` checks membership
/// against) at the policy's `hidden_issuer_depth`, and REQUIRES the proof's public
/// root to byte-equal it. A prover that proves membership in its OWN (forged) key
/// set fails this equality. And `m` is recomputed from the disclosed
/// commitment + salt + status reference (the SAME issuer-signed message the clear
/// path binds), so the proof is tied to a specific committed graph the relying
/// party can name — not a free-floating "some signature exists".
///
/// # Fail-closed
/// - No entries => nothing to check (the clear-key [`bind_issuer_attestations`]
///   remains the attestation gate); returns `Ok`.
/// - An entry present but the KeySet has NOT enabled the hidden-issuer path
///   (`hidden_issuer_depth == None`) => REJECT ([`CheckError::HiddenIssuerNotEnabled`]).
/// - A depth mismatch, an unresolvable root, a root mismatch, a message mismatch,
///   an unreferenced commitment, a malformed blob, or a bb rejection all REJECT.
///
/// PRECONDITION: `bind_issuer_attestations` + `bind_revocation` have already run
/// in the prefilter, so `manifest.revocation` is the ISSUER's (bound) reference
/// and the per-commitment salt is the issuer-attested one — the message recomputed
/// here is therefore the genuine issuer-signed message.
// [OPUS-4.8] sq-z9l: hidden-issuer attestation cryptographic gate.
fn bind_hidden_issuer_attestations(
    manifest: &ProofManifest,
    trusted_key_set: &KeySet,
    prover: &CircuitProver,
    work_dir: &Path,
    challenge: &FieldHex,
) -> Result<(), CheckError> {
    if manifest.hidden_issuer_attestations.is_empty() {
        // No hidden-issuer proofs; the clear-key path is the attestation gate.
        return Ok(());
    }
    // The relying party must have OPTED IN; otherwise it has no authoritative
    // key-set root to bind the proof to and rejects fail-closed.
    let Some(depth) = trusted_key_set.hidden_issuer_depth() else {
        return Err(CheckError::HiddenIssuerNotEnabled);
    };
    // Derive the AUTHORITATIVE key-set root from the relying party's OWN KeySet
    // (canonical order) — the trust anchor every entry's public root must equal.
    let auth_root = trusted_key_set
        .hidden_issuer_root(depth)
        .ok_or(CheckError::HiddenIssuerRootUnavailable)?;

    let challenge_fr = challenge
        .to_field()
        .ok_or(CheckError::HiddenIssuerMalformedProof)?;

    // The set of commitments a VERIFIED scan sub-proof references, with the
    // issuer-signed message recomputed for each (commitment hex -> message Fr).
    // Mirrors bind_issuer_attestations' referenced-commitment discipline: the
    // message is `commitment_message_with_status(C(G), salt, status_ref)`, exactly
    // the clear path's signed message, recomputed from the disclosed (and, by the
    // prefilter, issuer-bound) reference + salt.
    let referenced = scan_referenced_messages(manifest)?;

    for (i, hi) in manifest.hidden_issuer_attestations.iter().enumerate() {
        if hi.depth != depth {
            return Err(CheckError::HiddenIssuerDepthMismatch {
                declared: hi.depth,
                policy: depth,
            });
        }
        // The covered commitment must be referenced by a verified scan, and we use
        // OUR recomputed message (never the prover's declared `hi.message`).
        let Some(c_fr) = hi.commitment.to_field() else {
            return Err(CheckError::HiddenIssuerUnreferencedCommitment {
                commitment: hi.commitment.0.clone(),
            });
        };
        let c_key = field_to_hex(&c_fr);
        let Some(expected_m) = referenced.get(&c_key) else {
            return Err(CheckError::HiddenIssuerUnreferencedCommitment {
                commitment: hi.commitment.0.clone(),
            });
        };

        let blob = hex_decode(&hi.proof_hex).ok_or(CheckError::HiddenIssuerMalformedProof)?;
        let art = decode_artifacts(&blob).ok_or(CheckError::HiddenIssuerMalformedProof)?;

        // Public-input layout for hidden_issuer_d{depth} main: challenge, m,
        // key_set_root (three 32-byte BE field words, declaration order). We feed
        // OUR own values (nonce, recomputed message, authoritative root) — never
        // the prover's declared bytes — so a different challenge, a message not
        // bound to a referenced commitment, or a non-authoritative key set all fail
        // the byte-compare.
        let mut reconstructed: Vec<u8> = Vec::with_capacity(96);
        reconstructed.extend_from_slice(&field_to_be_bytes_32(&challenge_fr));
        reconstructed.extend_from_slice(&field_to_be_bytes_32(expected_m));
        reconstructed.extend_from_slice(&field_to_be_bytes_32(&auth_root));
        if reconstructed != art.public_inputs {
            // Diagnose WHICH public input diverged for a precise reason. The
            // proof's first word is the challenge (already bound elsewhere); the
            // second is the message, the third the key-set root.
            let pi = &art.public_inputs;
            if pi.len() == 96 && pi[64..96] != field_to_be_bytes_32(&auth_root) {
                return Err(CheckError::HiddenIssuerRootMismatch);
            }
            if pi.len() == 96 && pi[32..64] != field_to_be_bytes_32(expected_m) {
                return Err(CheckError::HiddenIssuerMessageMismatch {
                    commitment: hi.commitment.0.clone(),
                });
            }
            // Otherwise the challenge word (or the blob length) diverged.
            return Err(CheckError::HiddenIssuerProofRejected);
        }

        let id = CircuitId::HiddenIssuer { depth };
        let sub_work = work_dir.join(format!("hidden_issuer_{i}"));
        let canonical_vk = prover
            .canonical_vk(&id, &sub_work.join("vk"))
            .map_err(CheckError::Driver)?;
        let ok = prover
            .verify_with(&art.proof, &reconstructed, &canonical_vk, &sub_work.join("verify"))
            .map_err(CheckError::Driver)?;
        if !ok {
            return Err(CheckError::HiddenIssuerProofRejected);
        }
    }
    Ok(())
}

/// [OPUS-4.8] sq-c2ql (HolderPoP T6 / B2): the in-circuit holder Proof-of-Possession
/// cryptographic gate — the HIDDEN-key analogue of the clear-key
/// [`bind_holder_binding`] (T3/sq-z8s7 B1) and the structural twin of
/// [`bind_hidden_issuer_attestations`] (sq-z9l).
///
/// # What it proves (the binding edge — sq-c2ql)
/// The clear-key B1 gate ([`bind_holder_binding`]) binds a DISCLOSED holder key to
/// the issuer-attested digest host-side. B2 does the same WITHOUT disclosing the
/// holder key: the prover supplies a [`crate::manifest::HolderPokProof`] — a bb
/// proof of the `holder_pok` relation (knowledge of `hsk` with `hpk = hsk·G` and
/// `Poseidon2([ZKSIG_HK, hpk.x, hpk.y]) == holder_pk_digest`, `hsk`/`hpk` private).
/// The PUBLIC `holder_pk_digest` is NOT trusted as a prover field: this gate reads
/// it from the ISSUER-ATTESTED [`crate::manifest::AttestedHolderBinding`] on the
/// attestation covering the PoK's scan-referenced commitment, anchored in the
/// issuer's Schnorr signature ([`verify_holder_attestation_signature`], the same
/// `commitment_message_with_holder` / external-`K` anchor B1 uses). It then
/// reconstructs the proof's public inputs from the verifier's fresh nonce + THAT
/// issuer-signed digest and requires the proof's public inputs to byte-equal them
/// (audit-#1 discipline), recomputes the canonical `holder_pok` vk verifier-side
/// (audit-#2 discipline, never the prover's vk), and `bb verify`s.
///
/// So the proven (hidden) holder key is cryptographically bound to the
/// issuer-attested credential — the binding edge: a holder A who does NOT hold
/// `hsk_B` cannot produce a satisfying witness for B's issuer-signed digest
/// (DL-hardness + proof soundness), and cannot swap in its own digest without
/// breaking the issuer's EUF-CMA signature.
///
/// # Fail-closed contract
/// - No `holder_pok_proofs` AND the policy does not require one => nothing to do
///   (the clear-key path is the holder gate); returns `Ok`.
/// - A PoK over a commitment no verified scan references =>
///   [`CheckError::HolderPokUnreferencedCommitment`].
/// - A PoK over a commitment whose covering attestation carries no holder binding
///   (no issuer-attested digest to anchor on) =>
///   [`CheckError::HolderPokBindingMissing`].
/// - A digest / nonce / public-input mismatch => [`CheckError::HolderPokDigestMismatch`];
///   a malformed blob => [`CheckError::HolderPokMalformedProof`]; a bb rejection =>
///   [`CheckError::HolderPokProofRejected`].
/// - Under [`HolderBindingPolicy::require_in_circuit_pok`], a holder-bound
///   scan-referenced credential with NO matching verified PoK =>
///   [`CheckError::HolderPokMissing`] (the hidden-key proof is mandated).
///
/// PRECONDITION: `bind_issuer_attestations` + `bind_revocation` have already run in
/// the prefilter, so the per-commitment salt + revocation reference are the
/// ISSUER-bound ones — the message [`verify_holder_attestation_signature`]
/// recomputes is therefore the genuine issuer-signed message.
///
/// # SOUNDNESS (load-bearing, NOT a security claim)
/// This wires the binding edge; it does NOT make the composition verifier sound.
/// The verifier is NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2) and
/// `holder_pok` inherits that — a passing PoK is NOT, under an adversarial prover, a
/// guarantee the holder relation holds, and there is NO external
/// accredited-cryptographer sign-off (sq-qhy4 pending). Research-grade, opt-in. No
/// soundness / ZK-privacy claim is made or implied.
// [OPUS-4.8] sq-c2ql (HolderPoP T6 / B2): in-circuit holder PoK + issuer-attested
// credential binding edge. Opt-in, NOT-yet-sound (sq-qhy4).
fn bind_holder_pok(
    manifest: &ProofManifest,
    trusted_key_set: &KeySet,
    holder_binding_policy: &HolderBindingPolicy,
    prover: &CircuitProver,
    work_dir: &Path,
    challenge: &FieldHex,
) -> Result<(), CheckError> {
    // Nothing presented AND nothing required => the clear-key path is the gate.
    if manifest.holder_pok_proofs.is_empty() && !holder_binding_policy.requires_in_circuit_pok() {
        return Ok(());
    }

    // The scan-referenced commitments and their COVERING issuer attestation — the
    // EXACT lookup `bind_holder_binding` / `bind_issuer_attestations` use (compare as
    // field elements so 0x-padding cannot slip a mismatch). A PoK / requirement is
    // only meaningful for a credential the presentation actually uses.
    let mut covering: std::collections::BTreeMap<
        String,
        Option<&crate::manifest::CommitmentAttestation>,
    > = std::collections::BTreeMap::new();
    for sp in &manifest.sub_proofs {
        let ProofInputs::Scan { commitments, .. } = &sp.inputs else {
            continue;
        };
        for c in commitments {
            let Some(c_fr) = c.to_field() else { continue };
            let att = manifest.commitment_attestations.iter().find(|a| {
                a.commitment.to_field().is_some() && a.commitment.to_field() == Some(c_fr)
            });
            covering.insert(field_to_hex(&c_fr), att);
        }
    }

    // The verifier nonce (audit #4) — public-input field 0, fed by us, never the
    // prover's declared bytes.
    let challenge_fr = challenge
        .to_field()
        .ok_or(CheckError::HolderPokMalformedProof)?;

    // Track which holder-bound commitments a VERIFIED PoK covered, so the
    // require_in_circuit_pok sweep below can flag any holder-bound credential left
    // without a possession proof.
    let mut verified_for: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for (i, pok) in manifest.holder_pok_proofs.iter().enumerate() {
        let Some(c_fr) = pok.commitment.to_field() else {
            return Err(CheckError::HolderPokMalformedProof);
        };
        let c_key = field_to_hex(&c_fr);

        // (1) The PoK must cover a commitment a verified scan references (no dangling
        // PoK — mirrors HiddenIssuerUnreferencedCommitment).
        let Some(covering_att) = covering.get(&c_key) else {
            return Err(CheckError::HolderPokUnreferencedCommitment {
                commitment: pok.commitment.0.clone(),
            });
        };
        // (2) Its covering attestation must carry a holder binding — that is the
        // issuer-attested digest the binding edge anchors on. A PoK over a bearer
        // credential has nothing to bind to (fail-closed).
        let Some(att) = covering_att.filter(|a| a.holder.is_some()) else {
            return Err(CheckError::HolderPokBindingMissing {
                commitment: pok.commitment.0.clone(),
            });
        };

        // (3) Anchor the issuer-attested digest in the ISSUER signature: the digest
        // must be the one the issuer folded into commitment_message_with_holder,
        // verified under the EXTERNAL trusted K (never a free prover JSON field —
        // the design §4.3 obligation-1 anchor, shared with B1). A holder-bound
        // attestation whose signature does not so verify is rejected here
        // (InvalidIssuerSignature / IssuerKeyNotInKeySet).
        verify_holder_attestation_signature(manifest, trusted_key_set, att)?;
        let binding = att
            .holder
            .as_ref()
            .expect("filtered for Some(holder) above");
        let Some(attested_digest) = binding.digest() else {
            return Err(CheckError::HolderPokMalformedProof);
        };

        // (4) Reconstruct the public-input vector for holder_pok main: challenge,
        // holder_pk_digest (two 32-byte BE field words, declaration order). We feed
        // OUR nonce + the ISSUER-ATTESTED digest — never the prover's declared bytes
        // — so a proof committed under a different challenge OR over a digest the
        // issuer did not sign cannot byte-match (the binding edge).
        let blob = hex_decode(&pok.proof_hex).ok_or(CheckError::HolderPokMalformedProof)?;
        let art = decode_artifacts(&blob).ok_or(CheckError::HolderPokMalformedProof)?;
        let mut reconstructed: Vec<u8> = Vec::with_capacity(64);
        reconstructed.extend_from_slice(&field_to_be_bytes_32(&challenge_fr));
        reconstructed.extend_from_slice(&field_to_be_bytes_32(&attested_digest));
        if reconstructed != art.public_inputs {
            // Diagnose the digest word distinctly (the binding edge): if the proof's
            // second public-input word is not the issuer-attested digest, it is a
            // digest mismatch; otherwise the challenge word (or the blob length)
            // diverged.
            let pi = &art.public_inputs;
            if pi.len() == 64 && pi[32..64] != field_to_be_bytes_32(&attested_digest) {
                return Err(CheckError::HolderPokDigestMismatch {
                    commitment: pok.commitment.0.clone(),
                });
            }
            return Err(CheckError::HolderPokProofRejected {
                commitment: pok.commitment.0.clone(),
            });
        }

        // (5) Recompute the canonical holder_pok vk verifier-side (audit #2) and bb
        // verify over OUR reconstructed public inputs.
        let id = CircuitId::HolderPok;
        let sub_work = work_dir.join(format!("holder_pok_{i}"));
        let canonical_vk = prover
            .canonical_vk(&id, &sub_work.join("vk"))
            .map_err(CheckError::Driver)?;
        let ok = prover
            .verify_with(&art.proof, &reconstructed, &canonical_vk, &sub_work.join("verify"))
            .map_err(CheckError::Driver)?;
        if !ok {
            return Err(CheckError::HolderPokProofRejected {
                commitment: pok.commitment.0.clone(),
            });
        }
        verified_for.insert(c_key);
    }

    // (6) [OPUS-4.8] sq-c2ql: under require_in_circuit_pok, EVERY holder-bound
    // scan-referenced credential a HolderPop presentation uses must carry a verified
    // PoK — a holder-bound covering attestation with no matching PoK is rejected
    // fail-closed (the hidden-key possession proof is mandated, never silently
    // waived). Scoped to a `HolderPop` binding: a plain `Challenge` binding presents
    // no holder, so there is no possession to prove (mirrors the B1 `bind_holder_pop`
    // scoping, which returns early for `Challenge`).
    if holder_binding_policy.requires_in_circuit_pok()
        && matches!(manifest.binding, BindingMode::HolderPop { .. })
    {
        for (c_key, att) in &covering {
            if att.is_some_and(|a| a.holder.is_some()) && !verified_for.contains(c_key) {
                return Err(CheckError::HolderPokMissing {
                    commitment: c_key.clone(),
                });
            }
        }
    }
    Ok(())
}

/// [OPUS-4.8] sq-3c00 (HolderPoP hidden-holder-SET tier): the hidden-holder
/// set-membership cryptographic gate — the structural twin of
/// [`bind_hidden_issuer_attestations`] (sq-z9l) on the HOLDER axis, and the
/// hidden-holder upgrade over the clear-digest [`bind_holder_pok`] (sq-c2ql).
///
/// # What it proves (and what it hides)
/// `bind_holder_pok` makes the issuer-attested `holder_pk_digest` PUBLIC, so a
/// verifier learns the holder is the SPECIFIC (hidden-key) party bound to one
/// credential. This gate instead hides WHICH holder: the prover supplies a
/// [`crate::manifest::HolderSetProof`] — a bb proof of the `hidden_holder_set`
/// relation (knowledge of `hsk` with `hpk = hsk·G`, on-curve / non-identity /
/// `< L`, AND `holder_key_digest(hpk)` a Merkle MEMBER of the committed holder
/// set, with `hsk`/`hpk`/index/path all PRIVATE). Only `holder_set_root` is public,
/// exactly as `hidden_issuer` publishes `key_set_root` instead of the issuer key.
///
/// # Trust anchor (mirrors the audit #3 external-K anchor — load-bearing)
/// `holder_set_root` is a prover-committed public input, NOT trusted as a claim:
/// the verifier derives the AUTHORITATIVE root from its OWN [`HolderRegistry`]
/// (canonical order) at the policy's `hidden_holder_set_depth`, and REQUIRES the
/// proof's public root to byte-equal it. A prover that proves membership in its OWN
/// (forged) holder set fails this equality. The proof is also tied to a
/// scan-referenced commitment (no dangling proof), so it is bound to a credential
/// the relying party can name.
///
/// # Fail-closed contract
/// - No `holder_set_proofs` => nothing to check (the clear holder paths remain the
///   holder gate); returns `Ok`.
/// - An entry present but the registry has NOT enabled the hidden-holder-set path
///   (`hidden_holder_set_depth == None`) => REJECT [`CheckError::HolderSetNotEnabled`].
/// - A depth mismatch, an unresolvable root, a root mismatch, an unreferenced
///   commitment, a malformed blob, or a bb rejection all REJECT.
///
/// # SOUNDNESS (load-bearing, NOT a security claim)
/// This wires the membership gate; it does NOT make the composition verifier sound.
/// The verifier is NOT-yet-sound (sq-qhy4 / sq-9hrn; remediation epic sq-1s2) and
/// `holder_set_d{depth}` inherits that — a passing proof is NOT, under an
/// adversarial prover, a guarantee the holder relation holds, and there is NO
/// external accredited-cryptographer sign-off (sq-qhy4 pending). Research-grade,
/// opt-in. No soundness / ZK-privacy property is asserted as achieved.
// [OPUS-4.8] sq-3c00 (HolderPoP hidden-holder-SET tier). Opt-in, NOT-yet-sound (sq-qhy4).
fn bind_holder_set(
    manifest: &ProofManifest,
    holder_registry: &HolderRegistry,
    prover: &CircuitProver,
    work_dir: &Path,
    challenge: &FieldHex,
) -> Result<(), CheckError> {
    if manifest.holder_set_proofs.is_empty() {
        // No set-membership proofs; the clear holder paths are the holder gate.
        return Ok(());
    }
    // The relying party must have OPTED IN; otherwise it has no authoritative
    // holder-set root to bind the proof to and rejects fail-closed.
    let Some(depth) = holder_registry.hidden_holder_set_depth() else {
        return Err(CheckError::HolderSetNotEnabled);
    };
    // Derive the AUTHORITATIVE holder-set root from the relying party's OWN registry
    // (canonical order) — the trust anchor every entry's public root must equal.
    let auth_root = holder_registry
        .hidden_holder_set_root(depth)
        .ok_or(CheckError::HolderSetRootUnavailable)?;

    let challenge_fr = challenge
        .to_field()
        .ok_or(CheckError::HolderSetMalformedProof)?;

    // The set of commitments a VERIFIED scan sub-proof references (as field
    // elements, so 0x-padding cannot slip a mismatch) — a set-membership proof is
    // only meaningful for a credential the presentation actually uses.
    let mut referenced: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for sp in &manifest.sub_proofs {
        let ProofInputs::Scan { commitments, .. } = &sp.inputs else {
            continue;
        };
        for c in commitments {
            if let Some(c_fr) = c.to_field() {
                referenced.insert(field_to_hex(&c_fr));
            }
        }
    }

    for (i, hs) in manifest.holder_set_proofs.iter().enumerate() {
        if hs.depth != depth {
            return Err(CheckError::HolderSetDepthMismatch {
                declared: hs.depth,
                policy: depth,
            });
        }
        // The covered commitment must be referenced by a verified scan.
        let Some(c_fr) = hs.commitment.to_field() else {
            return Err(CheckError::HolderSetUnreferencedCommitment {
                commitment: hs.commitment.0.clone(),
            });
        };
        if !referenced.contains(&field_to_hex(&c_fr)) {
            return Err(CheckError::HolderSetUnreferencedCommitment {
                commitment: hs.commitment.0.clone(),
            });
        }

        let blob = hex_decode(&hs.proof_hex).ok_or(CheckError::HolderSetMalformedProof)?;
        let art = decode_artifacts(&blob).ok_or(CheckError::HolderSetMalformedProof)?;

        // Public-input layout for holder_set_d{depth} main: challenge, holder_set_root
        // (two 32-byte BE field words, declaration order). We feed OUR nonce + the
        // AUTHORITATIVE root — never the prover's declared bytes — so a proof
        // committed under a different challenge OR over a non-authoritative holder
        // set cannot byte-match (the trust anchor).
        let mut reconstructed: Vec<u8> = Vec::with_capacity(64);
        reconstructed.extend_from_slice(&field_to_be_bytes_32(&challenge_fr));
        reconstructed.extend_from_slice(&field_to_be_bytes_32(&auth_root));
        if reconstructed != art.public_inputs {
            // Diagnose the root word distinctly (the trust anchor): if the proof's
            // second public-input word is not the authoritative root, it is a root
            // mismatch; otherwise the challenge word (or the blob length) diverged.
            let pi = &art.public_inputs;
            if pi.len() == 64 && pi[32..64] != field_to_be_bytes_32(&auth_root) {
                return Err(CheckError::HolderSetRootMismatch);
            }
            return Err(CheckError::HolderSetProofRejected {
                commitment: hs.commitment.0.clone(),
            });
        }

        // Recompute the canonical holder_set_d{depth} vk verifier-side (audit #2)
        // and bb verify over OUR reconstructed public inputs.
        let id = CircuitId::HolderSet { depth };
        let sub_work = work_dir.join(format!("holder_set_{i}"));
        let canonical_vk = prover
            .canonical_vk(&id, &sub_work.join("vk"))
            .map_err(CheckError::Driver)?;
        let ok = prover
            .verify_with(&art.proof, &reconstructed, &canonical_vk, &sub_work.join("verify"))
            .map_err(CheckError::Driver)?;
        if !ok {
            return Err(CheckError::HolderSetProofRejected {
                commitment: hs.commitment.0.clone(),
            });
        }
    }
    Ok(())
}

/// The per-graph salt the verifier uses to recompute a scan commitment's
/// issuer-signed message, resolved from EITHER a clear [`CommitmentAttestation`]
/// over `c_fr` OR — for a HIDDEN-ONLY commitment (sq-xxg) — the
/// [`HiddenIssuerAttestation`]'s own `salt`. The clear attestation's salt is
/// preferred when present (the additive mode); the hidden entry's salt is the
/// fallback so a commitment with no clear attestation can still have its `m`
/// recomputed. `None` if neither source supplies a parseable salt.
///
/// # Disclosure posture (sq-93h, assessed)
/// Every salt this can return belongs to a commitment the presentation ALREADY
/// discloses in the clear (a scan's `commitments[g]`, byte-bound into the bb public
/// inputs by [`reconstruct_public_inputs`]), so the salt is a DOMINATED correlator and
/// withholding it behind an in-circuit salt-commitment would buy no unlinkability. That
/// conclusion is conditional on TWO things: `C(G)` staying public — pinned on the real
/// paths by `tests::hidden_only_salt_disclosure_is_dominated_by_the_clear_commitment` —
/// and the audit-#9 ISSUANCE discipline that no salt is reused for two distinct graphs,
/// of which only the within-manifest instance (`SaltReused`) is machine-checked. Argued
/// in `research/zk-hidden-path-salt-disclosure.md`.
// [OPUS-4.8] sq-xxg: salt source for hidden-only message reconstruction.
// [OPUS-5] sq-93h: disclosure assessed NO-BUILD; the trip-wire guards the premise.
fn resolve_commitment_salt(manifest: &ProofManifest, c_fr: &Fr) -> Option<Fr> {
    // Prefer the clear attestation's salt (the original sq-z9l additive path).
    if let Some(att) = manifest.commitment_attestations.iter().find(|a| {
        a.commitment.to_field().is_some() && a.commitment.to_field() == Some(*c_fr)
    }) {
        if let Some(salt_hex) = &att.salt {
            if let Some(salt_fr) = salt_hex.to_field() {
                return Some(salt_fr);
            }
        }
    }
    // Fall back to a hidden-issuer entry's salt (the hidden-only case).
    manifest
        .hidden_issuer_attestations
        .iter()
        .find(|hi| hi.commitment.to_field() == Some(*c_fr))
        .and_then(|hi| hi.salt.as_ref())
        .and_then(|salt_hex| salt_hex.to_field())
}

/// The issuer-signed message for every commitment a VERIFIED scan sub-proof
/// references, keyed by canonical commitment hex (sq-z9l). The message is
/// `commitment_message_with_status(C(G), salt, status_ref_digest(H(list), index,
/// version))` — the SAME message [`bind_issuer_attestations`] verifies the clear
/// signature over, recomputed from the disclosed (prefilter-validated, so
/// issuer-bound) salt + revocation reference. The salt is resolved via
/// [`resolve_commitment_salt`], so a HIDDEN-ONLY commitment (no clear attestation,
/// salt carried on the hidden entry — sq-xxg) is also covered. Used by the
/// hidden-issuer gate to bind each proof's PUBLIC `m` to a specific committed
/// graph.
fn scan_referenced_messages(
    manifest: &ProofManifest,
) -> Result<std::collections::BTreeMap<String, Fr>, CheckError> {
    let mut out: std::collections::BTreeMap<String, Fr> = std::collections::BTreeMap::new();
    let Some(rev) = &manifest.revocation else {
        // No reference => no status-bound message can be formed. The hidden-issuer
        // path requires the same issuer-bound reference the clear path does.
        return Ok(out);
    };
    // [OPUS-4.8] sq-ayv: derive the status reference in the mode the credential
    // uses (clear index, committed index, or [OPUS-5] sq-kndw fully hidden),
    // matching the message `bind_issuer_attestations` verified the signature over.
    let Some(status_ref) = status_ref_from_revocation(rev) else {
        return Ok(out);
    };
    for sp in &manifest.sub_proofs {
        let ProofInputs::Scan { commitments, .. } = &sp.inputs else {
            continue;
        };
        for c in commitments {
            let Some(c_fr) = c.to_field() else { continue };
            let Some(salt_fr) = resolve_commitment_salt(manifest, &c_fr) else { continue };
            let m = commitment_message_with_status(&c_fr, &salt_fr, &status_ref);
            out.insert(field_to_hex(&c_fr), m);
        }
    }
    Ok(out)
}

/// Derive the issuer-signed `status_ref` field element from the DISCLOSED
/// revocation reference's index-disclosure mode (sq-ayv): clear index =>
/// [`status_ref_digest`]; committed index => [`status_ref_commit_digest`]. `None`
/// if the reference's mode is malformed (neither/both set, or an unparseable
/// commitment). The disclosed reference has ALREADY been cross-checked against the
/// issuer signature by `bind_issuer_attestations`, so deriving from it here yields
/// the genuine issuer-signed message.
///
/// [OPUS-5] sq-kndw: the FULLY-HIDDEN mode is the third arm —
/// [`status_ref_fully_committed_digest`] over the two commitments, folding neither
/// the list id nor the version. It is matched FIRST because a fully-hidden
/// reference also carries an `index_commitment`, and it hashes the list IRI itself
/// (there is none to hash on that path).
// [OPUS-4.8] sq-ayv.
// [OPUS-5] sq-kndw: + the fully-hidden arm.
fn status_ref_from_revocation(rev: &crate::manifest::RevocationStatus) -> Option<Fr> {
    // FULLY-HIDDEN first: no clear list/version, both commitments present.
    if let Some(rc_hex) = &rev.ref_commitment {
        if rev.index.is_some() || rev.status_list.is_some() || rev.version.is_some() {
            return None;
        }
        let rc = rc_hex.to_field()?;
        let ic = rev.index_commitment.as_ref()?.to_field()?;
        return Some(status_ref_fully_committed_digest(&rc, &ic));
    }
    // The clear + committed paths both bind a clear list IRI and version.
    let list_id_fr = status_list_id_to_field(rev.status_list.as_deref()?);
    let version = rev.version?;
    match (rev.index, &rev.index_commitment) {
        (Some(index), None) => Some(status_ref_digest(&list_id_fr, index, version)),
        (None, Some(ic_hex)) => ic_hex
            .to_field()
            .map(|ic| status_ref_commit_digest(&list_id_fr, &ic, version)),
        _ => None,
    }
}

/// Normalize a hex key for set membership: strip an optional `0x` prefix and
/// lowercase, so K-membership is representation-insensitive.
// [OPUS-5] sq-rsd3v.6: `pub(crate)` so `sameas`'s canon-table re-check
// normalizes encodings through the SAME function `bind_entailment` grounds
// derivation antecedents with (one source of truth, no drift).
pub(crate) fn normalize_hex(h: &str) -> String {
    h.strip_prefix("0x").unwrap_or(h).to_ascii_lowercase()
}

/// Encode a query pattern's constant term to its `pattern_const_enc` field-hex
/// (salt-independent for IRIs/literals — only bnodes use the salt, and query
/// constants are never bnodes; matches `build::encode_slot` / the in-circuit
/// `pattern_const_enc`). Variable slots encode as `0x0`.
fn encode_pattern_slot(c: &Option<oxrdf::Term>) -> Option<FieldHex> {
    match c {
        Some(t) => encode_term(t, &Fr::from(0u64)).map(|f| FieldHex(field_to_hex(&f))),
        None => Some(FieldHex("0x0".to_string())),
    }
}

/// Whether a scan sub-proof's bound pattern constancy/encoding matches a query
/// pattern's constant slots. This is the verifier-side equality over the
/// bb-bound `pattern_is_const`/`pattern_const_enc` (audit #10): a scan over
/// `<hasAge>` cannot answer a query pattern over `<hasSalary>` (constant-swap),
/// and a variable slot must be a variable on both sides.
fn scan_matches_pattern(inputs: &ProofInputs, consts: &[Option<oxrdf::Term>; 3]) -> bool {
    let (is_const, const_enc) = match inputs {
        ProofInputs::Scan { pattern_is_const, pattern_const_enc, .. } => {
            (pattern_is_const, pattern_const_enc)
        }
        _ => return false,
    };
    for slot in 0..3 {
        let q_is_const = consts[slot].is_some();
        if q_is_const != is_const[slot] {
            return false;
        }
        // For a constant slot, the bound encoding must equal the query
        // constant's encoding; for a variable slot both carry the `0x0` filler
        // (still checked, so a non-`0x0` variable slot mismatches).
        match encode_pattern_slot(&consts[slot]) {
            Some(enc) if enc == const_enc[slot] => {}
            _ => return false,
        }
    }
    true
}

/// Stage 2a′: re-check a DECLARED [`ProofManifest::pattern_scans`] pattern→scan
/// mapping. A no-op when the field is empty (the ordinary case).
///
/// # This gate only ADDS obligations — it never narrows one
/// The declaration is a prover-authored reading of which scan answers which query
/// BGP pattern. It is NOT consulted by [`bind_query_correctness`] or
/// [`bind_attributions`]: both still resolve pattern→scan by constant MEMBERSHIP
/// (`scan_matches_pattern`), so the sq-q9r5e / audit-L-1 rule stands unweakened —
/// the FILTER must be discharged at EVERY slot the filtered variable occupies
/// across EVERY pattern a scan matches by constants, whatever the prover declares.
/// Declaring a mapping can therefore only ever cause an ADDITIONAL rejection
/// here; it can never buy an acceptance the membership regime would refuse.
///
/// # Why it does not narrow (the round-2 review finding — READ THIS BEFORE WIRING IT IN)
/// The obvious use of the declaration is to demand only the DECLARED answering
/// scan's slots, removing the membership over-demand on same-constant-layout
/// queries (`{ ?x <age> ?v . ?x <age> ?c }` — both patterns `(?, <age>, ?)`; see
/// `research/zk-audit-gpt56-2026-07.md` L-1). That is UNSOUND as the manifest
/// stands. SPARQL evaluates each pattern over EVERY compatible committed row, and
/// the query text authorises no prover-chosen partition of the committed data, so
/// a prover free to exclude a constant-compatible scan from a pattern can drop
/// that scan's rows out of the pattern's FILTER and attribution obligations while
/// still disclosing them. The checks below (total assignment: no empty entry, no
/// dangling scan, no declared pair that contradicts the bb-bound constants) pin
/// only that the declaration is a TOTAL map of scans to labels — they establish
/// nothing about whether an excluded scan contributes to the claimed result.
///
/// Narrowing needs the missing piece: a claimed result row bound to the selected
/// scan rows, with all shared-variable joins enforced, so that "this scan does not
/// contribute" is a VERIFIED property rather than a prover assertion the consumer
/// is asked to take on faith. The flat `ProofManifest` carries no such claimed
/// result row, so that witness is NOT built here and
/// the flat verifier keeps full constant-membership obligations, including the
/// over-demand, and the honest same-layout manifest stays REJECTED
/// (`pattern_scans_do_not_narrow_the_filter_obligation`).
///
/// # What IS checked when a declaration is present
/// Exactly one entry per query pattern ([`CheckError::PatternScanArityMismatch`]);
/// no empty entry ([`CheckError::PatternScanUnbound`]); every named sub-proof in
/// range, a scan, and with bb-bound `pattern_is_const`/`pattern_const_enc` that
/// MATCH the pattern's constants ([`CheckError::PatternScanMismatch`], audit #10);
/// and no scan sub-proof left undeclared
/// ([`CheckError::PatternScanUndeclared`]). A declaration that survives all four
/// is recorded metadata, nothing more.
// [OPUS-5] sq-q9r5e follow-up: explicit pattern→scan mapping, validated but
// deliberately NOT load-bearing. Research-grade, NOT externally audited (sq-qhy4).
fn check_pattern_scans(manifest: &ProofManifest) -> Result<(), CheckError> {
    if manifest.pattern_scans.is_empty() {
        return Ok(());
    }

    let patterns = fragment_patterns(&manifest.query)?;
    let consts = fragment_pattern_consts(&patterns);

    if manifest.pattern_scans.len() != consts.len() {
        return Err(CheckError::PatternScanArityMismatch {
            patterns: consts.len(),
            declared: manifest.pattern_scans.len(),
        });
    }

    let mut declared_scans: BTreeSet<usize> = BTreeSet::new();
    for (pi, decl) in manifest.pattern_scans.iter().enumerate() {
        if decl.is_empty() {
            return Err(CheckError::PatternScanUnbound { pattern: pi });
        }
        for &spi in decl {
            // Out of range / not a scan / constants disagree all collapse to the
            // same rejection: the declaration must not contradict the bb-bound
            // pattern constants (audit #10).
            let answers = manifest
                .sub_proofs
                .get(spi)
                .is_some_and(|sp| scan_matches_pattern(&sp.inputs, &consts[pi]));
            if !answers {
                return Err(CheckError::PatternScanMismatch { pattern: pi, proof: spi });
            }
            declared_scans.insert(spi);
        }
    }

    // No DANGLING scan: a declaration that discloses a scan's rows while naming it
    // for no pattern is an incoherent reading, so it is rejected rather than
    // recorded.
    for (spi, sp) in manifest.sub_proofs.iter().enumerate() {
        if matches!(sp.inputs, ProofInputs::Scan { .. }) && !declared_scans.contains(&spi) {
            return Err(CheckError::PatternScanUndeclared { proof: spi });
        }
    }

    Ok(())
}

/// Stage 2b/2c: bind every query BGP pattern's constants to a scan sub-proof
/// (audit #10) and every query FILTER to a slot-bound, true-verdict
/// `filter_int` sub-proof reached via a binding edge (audit #5/#6/#7).
///
/// FILTER var→slot mapping: a query FILTER `?v op c` is satisfied iff there is
/// a binding edge `(from_proof=scan, from_row, from_slot, to_proof=filter)`
/// such that (1) the scan answers the query pattern in which `?v` binds (its
/// constants match, `scan_matches_pattern`); (2) `from_slot` is exactly the
/// slot `?v` occupies in that pattern (audit #6 — closes "point the operand at
/// the salary slot for an age filter"); (3) the filter's bound `(op, bound)`
/// EQUAL the query's `(op, c)` (audit #5 — closes the 17-vs-`>=18`
/// comparison-substitution); and (4) the filter's `expected == true` (audit
/// #5/#6 — the verdict gates row inclusion; an `expected==false` row may not be
/// presented as passing).
///
/// # EVERY slot, not the first (sq-q9r5e / audit L-1)
/// A scan is matched to a query pattern by constant MEMBERSHIP, so ONE scan can
/// answer SEVERAL patterns — and those patterns may place the filtered variable
/// at DIFFERENT slots. The obligation is therefore per `(scan, row, slot)` over
/// the FULL set of slots `?v` occupies across the patterns that scan answers,
/// not the first such slot. Gating only the first accepted a manifest whose
/// other disclosed ?v column was never proven against the FILTER (confirmed
/// reachable; witness `filter_reject_ungated_second_slot_within_scan`).
///
/// This is deliberately FAIL-CLOSED on the pattern→scan ambiguity: where two
/// patterns share a constant layout the verifier cannot tell which one a given
/// scan was meant to answer, so it demands the FILTER be discharged for every
/// slot that scan could be read at. A manifest that cannot supply those proofs
/// is REJECTED rather than accepted on the strength of one of them. A prover's
/// `manifest.pattern_scans` declaration does NOT relax this — see
/// [`check_pattern_scans`] for why narrowing needs a verified result witness the
/// flat manifest cannot yet express.
///
/// A FILTER with no such edge ⇒ REJECT (audit #10 FILTER-add / a `filter_int`
/// over the wrong operand). Stage 2 already enforced the edge's scanned-slot
/// encoding equals the filter `operand_enc` (so #7 operand substitution is
/// closed by that equality over the now-bb-bound values + this slot check).
fn bind_query_correctness(manifest: &ProofManifest) -> Result<(), CheckError> {
    let patterns = fragment_patterns(&manifest.query)?;
    let consts = fragment_pattern_consts(&patterns);
    let filters = fragment_filters(&manifest.query)?;
    let var_slots = variable_slots(&patterns);

    // (2b) every query BGP pattern's constants are bound by SOME scan sub-proof.
    for (pi, c) in consts.iter().enumerate() {
        let bound = manifest
            .sub_proofs
            .iter()
            .any(|sp| scan_matches_pattern(&sp.inputs, c));
        if !bound {
            return Err(CheckError::UnboundPattern { pattern: pi });
        }
    }

    // (2c) every query FILTER has a matching, slot-bound, true-verdict proof for
    // EVERY active disclosed row of EVERY scan that answers the FILTER's pattern.
    // Per-row gating is the load-bearing part of #5/#6: the disclosed result is
    // the scans' rows, so a FILTER row whose verdict is false must be EXCLUDED —
    // i.e. every active row of a filtered pattern must carry a true-verdict
    // filter_int sub-proof over that row's operand slot. A single missing/false
    // row makes the FILTER unproven for the disclosed set => REJECT.
    for QueryFilter { variable, op, bound } in &filters {
        // The (pattern, slot) positions ?variable binds to. A FILTER over a
        // variable that never binds to a scanned column is unmappable.
        let positions: Vec<(usize, usize)> = var_slots
            .iter()
            .filter(|(v, _, _)| v == variable)
            .map(|(_, p, s)| (*p, *s))
            .collect();
        if positions.is_empty() {
            return Err(CheckError::UnmappableFilterVar { variable: variable.clone() });
        }
        // The FILTER must constrain at least one disclosed row (else it is
        // vacuously "satisfied" by an empty result while the query carries a
        // FILTER the prover never proved). Track that some scan answered the
        // pattern AND every one of its active rows is gated true.
        let mut any_scan_answered = false;
        for (spi, sp) in manifest.sub_proofs.iter().enumerate() {
            let (rows, row_count) = match &sp.inputs {
                ProofInputs::Scan { rows, row_count, .. } => (rows, *row_count as usize),
                _ => continue,
            };
            // EVERY slot ?v sits at across EVERY query pattern this scan
            // answers — not just the first.
            //
            // [OPUS-5] sq-q9r5e (audit L-1, `research/zk-audit-gpt56-2026-07.md`):
            // this was a `find_map`, which took only the FIRST matching
            // (pattern, slot) pair. Pattern→scan is resolved by constant
            // MEMBERSHIP (`scan_matches_pattern`), not an explicit mapping, so
            // ONE scan can answer SEVERAL query patterns — and when those
            // patterns place the filtered variable at DIFFERENT slots (e.g. a
            // `(?, P, ?)` scan answering both `(?s P ?v)` and `(?v P ?o)`),
            // every one of those slots is a column the relying party reads ?v
            // off. Gating only the first left the others ungated, so a row whose
            // second-slot binding of ?v was never proven against the FILTER was
            // presented as satisfying it (CONFIRMED reachable: the structural
            // gate accepted such a manifest — witness
            // `filter_reject_ungated_second_slot_within_scan` in `tests/e2e.rs`).
            //
            // Collecting ALL matching slots is the FAIL-CLOSED direction and
            // matches the discipline `bind_attributions` already applies (it
            // checks EVERY scan matching a pattern, never a first match). A
            // BTreeSet dedups the case where two patterns place ?v at the same
            // slot, and gives a deterministic order for the error path.
            //
            // [OPUS-5] sq-q9r5e follow-up: a `manifest.pattern_scans` declaration
            // is deliberately NOT read here. Narrowing this set to the declared
            // answering scan would let the prover drop a constant-compatible
            // scan's rows out of the FILTER obligation on its own say-so; see
            // `check_pattern_scans`.
            let slots: BTreeSet<usize> = positions
                .iter()
                .filter(|(pi, _)| {
                    consts.get(*pi).is_some_and(|c| scan_matches_pattern(&sp.inputs, c))
                })
                .map(|(_, si)| *si)
                .collect();
            if slots.is_empty() {
                continue;
            }
            any_scan_answered = true;
            // Every ACTIVE disclosed row must have a true-verdict filter_int
            // edge at EACH such slot with matching (op, bound).
            for row in 0..row_count.min(rows.len()) {
                for &slot in &slots {
                    let gated = manifest.binding_edges.iter().any(|edge| {
                        edge.from_proof == spi
                            && edge.from_row == row
                            && edge.from_slot == slot
                            && filter_edge_true(manifest, edge.to_proof, *op, *bound)
                    });
                    if !gated {
                        return Err(CheckError::UnboundFilter { variable: variable.clone() });
                    }
                }
            }
        }
        if !any_scan_answered {
            // A FILTER whose variable binds in a pattern, but no scan answers
            // that pattern: the FILTER cannot be discharged (FILTER-add on a
            // manifest missing the filtered pattern's scan).
            return Err(CheckError::UnboundFilter { variable: variable.clone() });
        }
    }
    Ok(())
}

/// Stage 2b′: a variable repeated WITHIN a single query BGP pattern
/// (`{ ?v <p> ?v }`) must bind ONE term, so every disclosed row of every scan that
/// answers that pattern must carry EQUAL values at the repeated slots.
///
/// # Why this is not already covered (#5240)
/// SPARQL evaluates a BGP by matching each pattern against the data with a single
/// substitution, so a variable at two slots of one pattern constrains those two
/// columns to be equal. Nothing in the flat manifest regime enforced that:
///
/// - [`scan_matches_pattern`] compares only per-slot CONST-NESS and the constant
///   ENCODINGS. Both slots of `{ ?v <p> ?v }` are variables, so it accepts any
///   `(?, <p>, ?)` scan regardless of what the rows hold.
/// - The scan circuit binds each disclosed row's CONSTANT slots to
///   `pattern_const_enc`; a variable slot is unconstrained by construction (that
///   is what makes it a variable), so the in-circuit statement says nothing here.
/// - The shared-variable gates all work CROSS-pattern and are therefore blind to
///   it: `sparq_zk::verify::cross_graph_join_obligations` (the disclosed path,
///   via [`recheck`]) iterates pattern PAIRS `i < j` over per-pattern variable
///   SETS — a repeated variable collapses in the set and `i == j` is never
///   considered — and [`bind_joins`] (the hidden path) likewise requires the
///   shared variable to sit in two DISTINCT patterns (`pj != pi`).
///
/// Confirmed empirically reachable against [`prefilter_manifest_structure`]
/// (the method used for audit L-1): a manifest disclosing `(alice, knows, bob)`
/// under `SELECT ?v WHERE { ?v <knows> ?v }` was ACCEPTED, so a relying party
/// read a solution binding `?v` to two different terms at once. Witness:
/// `repeated_pattern_var_rejects_a_row_whose_slots_disagree` in `tests/e2e.rs`.
///
/// # Per-row, over every constant-matching scan (the sq-q9r5e regime)
/// This mirrors the FILTER gate exactly. The disclosed result IS the scans' rows,
/// so EVERY active disclosed row of a matching scan is presented as answering the
/// pattern and must satisfy it — one bad row makes the pattern unproven for the
/// disclosed set. And pattern→scan is resolved by constant MEMBERSHIP, not by the
/// prover's [`ProofManifest::pattern_scans`] declaration, so the obligation runs
/// over EVERY scan whose bound constants match. Where a query places a
/// repeated-variable pattern and a distinct-variable pattern at the SAME constant
/// layout (`{ ?v <p> ?v . ?a <p> ?b }`) that is an OVER-demand — the verifier
/// cannot tell which pattern a given scan answers, so it demands the equality for
/// both readings and REJECTS rather than accept on the prover's say-so. That is
/// the same deliberate fail-closed trade sq-q9r5e made for FILTERs; see
/// [`check_pattern_scans`] for why narrowing it needs a verified result witness
/// the flat manifest cannot yet express.
///
/// Comparison is on canonical big-endian field bytes ([`field_hex_eq`]), so hex
/// spelling/padding differences do not spuriously diverge and a malformed row hex
/// fails CLOSED (the reconstruction stage also rejects it as `MalformedField`).
///
/// The `extended-fragment` regime does not need this gate: `bind_fragment_scans`
/// already binds each variable slot of a selected row to the disclosed solution
/// (projected vars) or to a per-branch coherence map (existentials), both keyed by
/// variable NAME across slots, so a repeated variable's two slots are compared
/// there by construction.
// [OPUS-5] #5240: within-pattern slot equality. Research-grade, NOT externally
// audited (sq-qhy4).
fn bind_repeated_pattern_slots(manifest: &ProofManifest) -> Result<(), CheckError> {
    let patterns = fragment_patterns(&manifest.query)?;
    let consts = fragment_pattern_consts(&patterns);
    let var_slots = variable_slots(&patterns);

    for (pi, c) in consts.iter().enumerate() {
        // The repeated-slot obligations of THIS pattern: for a variable occupying
        // slots [s0, s1, ..], every later slot must equal the first (which chains
        // into full pairwise equality). A pattern with no repeat contributes none.
        let mut by_var: std::collections::BTreeMap<&str, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (v, p, s) in &var_slots {
            if *p == pi {
                by_var.entry(v.as_str()).or_default().push(*s);
            }
        }
        let obligations: Vec<(&str, usize, usize)> = by_var
            .iter()
            .filter(|(_, slots)| slots.len() > 1)
            .flat_map(|(v, slots)| slots[1..].iter().map(move |s| (*v, slots[0], *s)))
            .collect();
        if obligations.is_empty() {
            continue;
        }

        for (spi, sp) in manifest.sub_proofs.iter().enumerate() {
            let (rows, row_count) = match &sp.inputs {
                ProofInputs::Scan { rows, row_count, .. } => (rows, *row_count as usize),
                _ => continue,
            };
            if !scan_matches_pattern(&sp.inputs, c) {
                continue;
            }
            let active = row_count.min(rows.len());
            for (row, values) in rows.iter().take(active).enumerate() {
                for &(variable, first, other) in &obligations {
                    if !field_hex_eq(&values[first], &values[other]) {
                        return Err(CheckError::RepeatedSlotMismatch {
                            pattern: pi,
                            proof: spi,
                            row,
                            variable: variable.to_string(),
                            slots: (first, other),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// [OPUS-4.8] sq-sfsi (hidden cross-credential JOIN, step 4 of sq-bwwl): the
/// `bind_joins` verifier gate — the hidden-key analogue of `bind_query_correctness`
/// + the `binding_edges` consistency stage, for `ProofManifest::join_edges`.
///
/// `bind_joins` enforces THREE properties; a manifest whose join proof,
/// commitments, or query binding do not line up is REJECTED:
///
/// 1. **Commitment-matching (anti-A2, design §2.3/§4.2).** For each [`JoinEdge`],
///    the referenced `join_eq` sub-proof's PUBLIC `commit_a`/`commit_b` MUST
///    byte-equal the two referenced SCAN sub-proofs' bound
///    `commitments[graph_a]`/`commitments[graph_b]`. The scan commitments are
///    audit-#1 byte-bound into the scan proofs AND issuer-attested+in-`K` (audit
///    #3, `bind_issuer_attestations`, run in the same structural stage), so the
///    join is provably over two genuine, attested credentials. A `join_eq` pointed
///    at a commitment no referenced scan attests (cross-scan forgery) is rejected
///    ([`CheckError::JoinCommitmentMismatch`]). The byte-comparison is over the
///    SAME canonical big-endian field bytes the audit-#1 reconstruction uses (so
///    `0x`-padding spelling differences do not spuriously diverge).
///
/// 2. **Canonical VK (anti-A1, design §4.1) — enforced by the sub-proof loop.** The
///    `join_eq` proof itself is cryptographically verified in
///    [`verify_manifest`]'s per-sub-proof loop EXACTLY like every other member:
///    [`reconstruct_public_inputs`] rebuilds `[challenge, commit_a, commit_b,
///    join_commitment, slot_a, slot_b]` (verifier nonce as field 0) and
///    byte-equals it against the proof's public inputs; the vk is recomputed
///    verifier-side from the re-derived [`CircuitId::JoinEq`] (NEVER the prover's
///    vk, audit #2); and `bb verify` runs. So the equality is cryptographically
///    proved, not JSON-asserted, and a forged proof / attacker vk / mismatched
///    public inputs all reject there. `bind_joins` is the STRUCTURAL gate that ties
///    those already-bound public inputs to the scans and the query; it does not
///    re-run bb (the structural stage is reachable without the toolchain, exactly
///    like `bind_query_correctness`). Because the slots/commitments this gate reads
///    off the `join_eq` `ProofInputs` are the SAME values the sub-proof loop
///    byte-binds into the proof, a manifest that passes BOTH stages has its join
///    commitments + slots simultaneously (a) equal to the attested scans / query
///    and (b) bound into a valid `join_eq` proof.
///
/// 3. **UnboundJoin query binding — slot binding (design §3.3 step 5 / §4.4).**
///    The `join_eq` proof's PUBLIC `slot_a`/`slot_b` MUST equal the query-derived
///    slots a variable SHARED across the two patterns the referenced scans answer
///    occupies (`variable_slots`). A join proved over the wrong column (the §4.4 /
///    audit-#6 analogue) — OR a SPURIOUS join edge whose two scans share no query
///    variable at the declared slots at all — is rejected
///    ([`CheckError::JoinSlotMismatch`]): the slots are PUBLIC by design (the query
///    already reveals which column a shared variable occupies, §4.4), so this is a
///    plain public-input equality, and it doubles as the anti-spurious-join check
///    (a prover cannot inject a `join_eq` over an unrelated column pair). The
///    "patterns the referenced scans answer" is resolved by MEMBERSHIP against the
///    SPECIFIC `edge.scan_a`/`edge.scan_b` (not a first-match `position`), so a
///    query pattern legitimately answered by MORE THAN ONE scan (the same triple
///    pattern satisfied by two credentials) is handled correctly: the join binds
///    against whichever scan the edge names, neither spuriously rejected nor bound
///    to the wrong scan via `sub_proofs` ordering. [OPUS-4.8] sq-sfsi multi-scan.
///
/// # Scope / honesty boundary (the disclosed-vs-hidden distinction — load-bearing)
/// `bind_joins` rigorously validates every DECLARED hidden `JoinEdge`. It does NOT
/// *demand* a hidden join for every query cross-scan shared variable: a
/// cross-credential shared variable can ALSO be discharged by the DISCLOSED-row
/// path — the existing `join_obligations` / `verify::recheck` non-bnode obligation
/// gate (stage 1a) + the disclosed scan rows, which is the default mechanism and is
/// verified SEPARATELY. The hidden `JoinEdge` is the OPT-IN privacy alternative
/// (the joined value is hidden instead of disclosed in the rows). So a "dropped"
/// hidden join is NOT a soundness hole — it falls back to the disclosed path, which
/// `recheck` still enforces; demanding a hidden join would WRONGLY break disclosed
/// joins. The completeness obligation the design's `UnboundJoin` names is therefore
/// discharged by the disclosed-row gate for non-hidden joins; for a join the prover
/// CHOSE to make hidden, this gate forbids forging it (point 1) or binding it to
/// the wrong column / an unrelated scan pair (point 3).
///
/// This gate does NOT verify the join_commitment opening (the value stays hidden —
/// the privacy win) and does NOT itself prove `a_val == b_val` (that is the
/// in-circuit equality, verified by the sub-proof loop's `bb verify`, point 2).
/// Multi-way (N-way) join commitment-equality across a chain (design §2.4) is NOT
/// yet enforced here — a single pairwise join is the v1 scope; the join_eq PROVING
/// path + a FULL-bb accept test (sq-r2s8) and the forge-and-verify regression suite
/// (sq-hlul) are the follow-ups. What IS enforced is the security-critical
/// direction: a forged / cross-scan / wrong-slot / spurious hidden join is rejected.
///
/// # Cross-credential scope constraint (sq-cuvmj) — READ BEFORE CLAIMING THE USE CASE
/// The headline use case for a hidden `JoinEdge` is joining two genuinely DIFFERENT
/// credentials. In the current manifest schema that case only reaches this gate when
/// both credentials carry the SAME issuer-signed status reference, because
/// [`ProofManifest::revocation`] is SCALAR: [`resolve_status_ref`] (run earlier, in
/// [`bind_issuer_attestations`]) requires EVERY scan-covering commitment's attested
/// status to resolve to that ONE reference, so two credentials with distinct
/// `(list, index, version)` slots cannot both be attested and the manifest is
/// rejected upstream ([`CheckError::RevocationReferenceMismatch`]) before any join
/// is inspected.
///
/// This is FAIL-CLOSED — it is an over-restriction, not a hole. The construction it
/// blocks (present a live credential A alongside a REVOKED credential B, joined,
/// hoping B's liveness goes unchecked because there is only one `revocation` field)
/// has no false-accept: pointing `revocation` at A's slot makes B's attestation
/// mismatch, and pointing it at B's slot makes [`bind_revocation`] read B's SET
/// status bit and reject [`CheckError::CredentialRevoked`]
/// (`research/zk-bind-composition-review.md` §Finding B, attempt 5).
///
/// The practical consequence for this gate: what it validates today is hidden joins
/// ACROSS GRAPHS OF ONE CREDENTIAL (or across credentials sharing a status slot),
/// NOT arbitrary multi-credential joins. Do not describe `bind_joins` as enabling
/// arbitrary cross-credential joins until the manifest carries per-credential
/// revocation references; the obligations such a migration owes are pre-registered
/// on [`ProofManifest::revocation`].
// [OPUS-4.8] sq-sfsi: bind_joins gate (commitment-matching + query slot binding).
// [OPUS-5] sq-cuvmj: + the scalar-revocation cross-credential scope constraint.
fn bind_joins(manifest: &ProofManifest) -> Result<(), CheckError> {
    if manifest.join_edges.is_empty() {
        // No hidden joins declared: nothing for this gate to validate. A query
        // cross-scan shared variable WITHOUT a hidden JoinEdge is discharged by the
        // disclosed-row path (`recheck`/`join_obligations`, stage 1a) — not here.
        return Ok(());
    }

    let patterns = fragment_patterns(&manifest.query)?;
    let consts = fragment_pattern_consts(&patterns);
    let var_slots = variable_slots(&patterns);

    // Does the SPECIFIC scan sub-proof `scan_idx` answer query pattern `pi`?
    // A pattern is answered by a scan iff the scan's bound `pattern_const_enc`
    // matches the pattern's constants (audit #10). Crucially this is a
    // MEMBERSHIP test against the referenced scan — NOT a first-match
    // `position(..)` lookup. A query pattern may LEGITIMATELY be answered by
    // MORE THAN ONE scan (the same triple pattern satisfied by two different
    // credentials / graph commitments); the disclosed-row path
    // (`bind_query_correctness`, `.any(..)` + the per-scan FILTER loop) already
    // treats multi-scan-per-pattern as a first-class configuration. A
    // first-match `position` here would (a) REJECT a valid join whose edge
    // points at a non-first scan answering the pattern, and (b) — worse for
    // soundness — let a prover order `sub_proofs` so the slot binding validates
    // against a DIFFERENT (first-match) scan than the one the edge actually
    // references. Binding against the specific `edge.scan_a`/`edge.scan_b`
    // closes both. [OPUS-4.8] sq-sfsi multi-scan fix.
    let pattern_answered_by_scan = |pi: usize, scan_idx: usize| -> bool {
        consts
            .get(pi)
            .zip(manifest.sub_proofs.get(scan_idx))
            .is_some_and(|(c, sp)| scan_matches_pattern(&sp.inputs, c))
    };

    // For the N-way chain check (design §2.4): record, per edge that passes the
    // slot binding, the SHARED join VARIABLE it binds and the join_eq proof's
    // `join_commitment`. Two edges that join the SAME query variable form a multi-
    // hop chain and MUST carry byte-equal commitments (enforced after the loop).
    // [OPUS-4.8] sq-r2s8.
    let mut chain: Vec<(String, FieldHex, usize)> = Vec::new();

    // --- (1)+(3a): per-edge commitment-matching + slot binding. ---
    for (e, edge) in manifest.join_edges.iter().enumerate() {
        // Resolve the three referenced sub-proofs (dangling => reject).
        let scan_a = manifest
            .sub_proofs
            .get(edge.scan_a)
            .ok_or(CheckError::JoinDanglingEdge { edge: e })?;
        let scan_b = manifest
            .sub_proofs
            .get(edge.scan_b)
            .ok_or(CheckError::JoinDanglingEdge { edge: e })?;
        let join = manifest
            .sub_proofs
            .get(edge.join_proof)
            .ok_or(CheckError::JoinDanglingEdge { edge: e })?;

        // Kinds: scan_a/scan_b must be scans; join_proof must be a join_eq.
        let commit_a_scan = match &scan_a.inputs {
            ProofInputs::Scan { commitments, .. } => commitments
                .get(edge.graph_a)
                .ok_or(CheckError::JoinDanglingEdge { edge: e })?,
            _ => return Err(CheckError::JoinEdgeKindMismatch { edge: e }),
        };
        let commit_b_scan = match &scan_b.inputs {
            ProofInputs::Scan { commitments, .. } => commitments
                .get(edge.graph_b)
                .ok_or(CheckError::JoinDanglingEdge { edge: e })?,
            _ => return Err(CheckError::JoinEdgeKindMismatch { edge: e }),
        };
        let (commit_a_join, commit_b_join, join_commitment, slot_a, slot_b) = match &join.inputs {
            ProofInputs::JoinEq { commit_a, commit_b, join_commitment, slot_a, slot_b, .. } => {
                (commit_a, commit_b, join_commitment, *slot_a, *slot_b)
            }
            _ => return Err(CheckError::JoinEdgeKindMismatch { edge: e }),
        };

        // (1) Commitment-matching (anti-A2). Compare on the canonical big-endian
        // field bytes (so spelling/padding differences do not spuriously diverge);
        // a malformed commitment hex on EITHER side fails closed (the audit-#1
        // reconstruction also rejects it as MalformedField in the bb stage).
        if !field_hex_eq(commit_a_join, commit_a_scan)
            || !field_hex_eq(commit_b_join, commit_b_scan)
        {
            return Err(CheckError::JoinCommitmentMismatch { edge: e });
        }

        // (3a) Slot binding (§4.4). The edge's two scans answer two query
        // patterns; the shared join variable must occupy slot_a in pattern A and
        // slot_b in pattern B. We require, for the SPECIFIC scans `edge.scan_a` /
        // `edge.scan_b` the edge references (NOT a first-match scan that merely
        // happens to answer the pattern earlier in `sub_proofs`), that there
        // exist patterns pi (answered by scan_a) and pj (answered by scan_b)
        // carrying a SHARED variable at exactly the proof's public (slot_a,
        // slot_b). A join_eq whose public slots are not a real shared variable's
        // query-derived positions over the referenced scans is rejected. This is
        // multi-scan-safe: with two scans answering one pattern, the binding
        // validates against whichever scan the edge actually names, so it can
        // neither be spuriously rejected nor bound to the wrong scan.
        // The shared variable the edge binds, if the slot binding holds (its NAME
        // identifies the N-way chain this edge belongs to — design §2.4).
        let join_var = patterns.iter().enumerate().find_map(|(pi, _)| {
            // pattern pi is answered by THE REFERENCED scan_a, slot_a is a var.
            if !pattern_answered_by_scan(pi, edge.scan_a) {
                return None;
            }
            let var_a = var_at(&var_slots, pi, slot_a as usize)?;
            // The SAME variable occupies slot_b in some pattern answered by the
            // REFERENCED scan_b.
            let shared = patterns.iter().enumerate().any(|(pj, _)| {
                pj != pi
                    && pattern_answered_by_scan(pj, edge.scan_b)
                    && var_at(&var_slots, pj, slot_b as usize).as_deref() == Some(var_a.as_str())
            });
            shared.then_some(var_a)
        });
        let Some(join_var) = join_var else {
            return Err(CheckError::JoinSlotMismatch { edge: e });
        };
        chain.push((join_var, join_commitment.clone(), e));
    }

    // --- (4) N-way chain commitment-equality (design §2.4). ---
    // A query variable joined across MORE THAN TWO patterns is composed from
    // several pairwise `join_eq` sub-proofs. The composition is sound only if every
    // pairwise proof binds the SAME hiding `join_commitment` (the prover uses one
    // blinder), so `a_val == b_val` per hop + the shared commitment compose into a
    // transitive N-way equality WITHOUT disclosing the value. We group the bound
    // edges by their join VARIABLE and require all `join_commitment`s in a group to
    // byte-equal the group's first; a divergence means the hops proved equalities
    // over potentially different values, so the N-way join is unproven. A single
    // edge per variable (the 2-way case) trivially passes. [OPUS-4.8] sq-r2s8.
    for (i, (var_i, commit_i, _)) in chain.iter().enumerate() {
        // Compare against the FIRST edge sharing this variable (the chain anchor).
        if let Some((_, anchor_commit, _)) =
            chain.iter().take(i).find(|(v, _, _)| v == var_i)
        {
            if !field_hex_eq(commit_i, anchor_commit) {
                return Err(CheckError::JoinCommitmentChainMismatch { edge: chain[i].2 });
            }
        }
    }

    Ok(())
}

/// The variable occupying `(pattern, slot)` in the `variable_slots` table, if any
/// (a constant slot returns `None`). [OPUS-4.8] sq-sfsi.
fn var_at(var_slots: &[(String, usize, usize)], pattern: usize, slot: usize) -> Option<String> {
    var_slots
        .iter()
        .find(|(_, p, s)| *p == pattern && *s == slot)
        .map(|(v, _, _)| v.clone())
}

/// Byte-equality of two [`FieldHex`] commitment values over their canonical
/// big-endian field representation (so `0x`-padding / case differences do not
/// spuriously diverge). A malformed hex on either side returns `false`
/// (fail-closed — a non-parseable commitment is never "equal"). [OPUS-4.8] sq-sfsi.
fn field_hex_eq(a: &FieldHex, b: &FieldHex) -> bool {
    match (a.to_field(), b.to_field()) {
        (Some(fa), Some(fb)) => field_to_be_bytes_32(&fa) == field_to_be_bytes_32(&fb),
        _ => false,
    }
}

/// Reconcile the scan-LOCAL graph-index namespace of `manifest.attributions` into a
/// GLOBAL namespace keyed by the answering scan's committed-graph IDENTITY (the
/// canonical big-endian bytes of `commitments[g]`), so the Q6 gate
/// `sparq_zk::verify::cross_graph_join_obligations` computes the union
/// `|A_i ∪ A_j|` across patterns in a CONSISTENT namespace. [OPUS-4.8] sq-en5dx
/// (Finding A of the sq-1s2.6 composition review, `research/zk-bind-composition-review.md`).
///
/// # The bug this closes
/// `manifest.attributions[pi]` is a set of indices into the ANSWERING scan's OWN
/// `commitments` vector — a scan-LOCAL index (`bind_attributions` cross-checks it
/// against that scan's proof-bound `attribution` bits, and the audit-#1
/// reconstruction byte-binds it per scan). But the Q6 gate treated those integers
/// as globally-distinct graph identities (`attributions[i].union(&attributions[j]).count() > 1`).
/// Two DISTINCT single-commitment (`k=1`) scans each declaring local index 0 (the
/// Finding-A `[[0],[0]]` construction) therefore collapsed to a single element, so
/// `|A_0 ∪ A_1| = 1` and the non-bnode obligation was DROPPED for the cross-scan
/// join — the gate was inert, with commitment-salt separation the sole live
/// backstop. Keying the union on the commitment identity makes two distinct graphs
/// map to distinct global ids (obligation correctly required) while two scans over
/// the SAME committed graph still collapse (a same-graph bnode join is legitimate,
/// no spurious obligation).
///
/// # Consistency / fail-closed contract
/// The output has the SAME length as `manifest.attributions`, so `recheck`'s
/// `patterns.len() != attributions.len()` arity check still fires `ArityMismatch`
/// on a mis-sized vector, and an empty declared set still yields an empty global
/// set (fail-closed `EmptyAttribution`). A local index that resolves to NO
/// answering scan's commitment (out-of-range, malformed hex, or an unparsable
/// query) is mapped to a FRESH unique id: this can only WIDEN the union — demanding
/// MORE obligations, the conservative-safe direction — never collapse two graphs;
/// the malformed manifest is independently rejected by stage 1b
/// (`AttributionMalformed`), stage 2b (`UnboundPattern`), or the reconstruction
/// stage. A pattern legitimately answered by MORE THAN ONE scan (multi-scan) unions
/// every matching scan's identity (widening = safe), mirroring the
/// `scan_matches_pattern` membership test `bind_attributions`/`bind_joins` use.
fn global_attributions(manifest: &ProofManifest) -> Vec<BTreeSet<usize>> {
    // The query's per-pattern constant slots identify which scan answers each
    // pattern (the same mapping `bind_attributions` uses). A parse failure leaves
    // `consts` empty; every local index then falls through to a fresh unique id and
    // `recheck` re-reports the parse error itself (it re-parses the query).
    let consts = fragment_patterns(&manifest.query)
        .map(|patterns| fragment_pattern_consts(&patterns))
        .unwrap_or_default();

    // Precompute, ONCE per scan sub-proof, the 32-byte committed-graph identity of
    // each of its commitments (parsing each `FieldHex` exactly once). Non-scan
    // sub-proofs (and commitments that fail to parse) get an empty / `None` slot.
    // This turns the hot loop below into a slice lookup instead of a re-parse +
    // convert per `(pattern, index, scan)` candidate, bounding verifier CPU / DoS
    // surface. [OPUS-4.8] sq-en5dx (Copilot review): precompute commitment ids.
    let scan_commit_ids: Vec<Vec<Option<[u8; 32]>>> = manifest
        .sub_proofs
        .iter()
        .map(|sp| match &sp.inputs {
            ProofInputs::Scan { commitments, .. } => commitments
                .iter()
                .map(|c| c.to_field().map(|f| field_to_be_bytes_32(&f)))
                .collect(),
            _ => Vec::new(),
        })
        .collect();

    // Precompute, ONCE per pattern, the sub-proof indices whose scan answers it
    // (the `scan_matches_pattern` membership test) so the hot loop iterates a small
    // precomputed list instead of re-scanning every sub-proof for every local
    // index. A pattern with no `consts` entry (parse failure / arity slip) matches
    // nothing → every index falls through to a fresh id (fail-closed, widening).
    // [OPUS-4.8] sq-en5dx (Copilot review): precompute pattern → matching scans.
    let pattern_scans: Vec<Vec<usize>> = manifest
        .attributions
        .iter()
        .enumerate()
        .map(|(pi, _)| {
            let Some(c) = consts.get(pi) else {
                return Vec::new();
            };
            manifest
                .sub_proofs
                .iter()
                .enumerate()
                .filter(|(_, sp)| {
                    matches!(sp.inputs, ProofInputs::Scan { .. })
                        && scan_matches_pattern(&sp.inputs, c)
                })
                .map(|(idx, _)| idx)
                .collect()
        })
        .collect();

    // A SINGLE monotonic id generator shared by BOTH interned commitment identities
    // and fresh unresolved-index ids: `intern` maps each distinct committed-graph
    // identity to a stable id, and an unresolved index simply draws the next id
    // too. Every id is allocated at most once (the only reuse is the intentional
    // interning of a repeated identity), so ids are pairwise-distinct with NO
    // reliance on wraparound — removing the descending `usize::MAX` underflow
    // footgun. [OPUS-4.8] sq-en5dx (Copilot review): monotonic next_id.
    let mut intern: std::collections::BTreeMap<[u8; 32], usize> =
        std::collections::BTreeMap::new();
    let mut next_id: usize = 0;

    manifest
        .attributions
        .iter()
        .enumerate()
        .map(|(pi, local)| {
            let mut out = BTreeSet::new();
            for &g in local {
                let mut resolved = false;
                for &sp_idx in &pattern_scans[pi] {
                    if let Some(Some(key)) = scan_commit_ids[sp_idx].get(g) {
                        let id = *intern.entry(*key).or_insert_with(|| {
                            let v = next_id;
                            next_id += 1;
                            v
                        });
                        out.insert(id);
                        resolved = true;
                    }
                }
                if !resolved {
                    out.insert(next_id);
                    next_id += 1;
                }
            }
            out
        })
        .collect()
}

/// Stage 2e: bind the prover's `manifest.attributions` (which drives the Q6
/// cross-graph-bnode-join obligation gate in stage 1a) to the PROOF-BOUND
/// per-graph attribution each scan sub-proof carries (audit #8).
///
/// For each query BGP pattern `pi`, find the scan sub-proof that answers it
/// (constants match, `scan_matches_pattern` — a prover's `manifest.pattern_scans`
/// declaration deliberately does NOT narrow this, see [`check_pattern_scans`])
/// and require `manifest.attributions[pi]` to be a SUPERSET of that scan's
/// proof-bound matched-graph set (`attribution[g] == true`). Soundness:
/// - **Under-declaring** (the `[[0],[0]]` forge): a graph the scan proved a
///   contribution from but `manifest.attributions[pi]` omits is rejected. This
///   is the load-bearing #8 fix — the prover can no longer shrink the
///   attribution set below the truth to drop a cross-graph obligation.
/// - **Over-declaring** is conservative-safe: extra graphs in
///   `manifest.attributions[pi]` only widen `|A_i ∪ A_j|`, demanding MORE
///   non-bnode obligations (the coarser-is-safe direction, per `verify.rs`
///   module docs). So a superset relation, not equality, is the correct gate —
///   it preserves the legitimate "this pattern MAY draw from these graphs"
///   semantics while forbidding the dishonest narrowing.
///
/// The proof-bound attribution is byte-checked against the bb public inputs in
/// stage 3 (audit #1 reconstruction). Here in the structural stage we cross-check
/// the SAME declared `attribution` field that the reconstruction will bind, so a
/// structure-only verify already rejects the forge and the full verify rejects it
/// twice (structurally + cryptographically).
// [OPUS-4.8] audit #8.
fn bind_attributions(manifest: &ProofManifest) -> Result<(), CheckError> {
    let patterns = fragment_patterns(&manifest.query)?;
    let consts = fragment_pattern_consts(&patterns);

    for (pi, c) in consts.iter().enumerate() {
        let declared: BTreeSet<usize> = manifest
            .attributions
            .get(pi)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();

        // Find a scan sub-proof that answers this pattern and read its
        // proof-bound attribution. (Stage 2b already guarantees one exists.)
        let mut matched_a_scan = false;
        for sp in &manifest.sub_proofs {
            let ProofInputs::Scan { attribution, .. } = &sp.inputs else {
                continue;
            };
            if !scan_matches_pattern(&sp.inputs, c) {
                continue;
            }
            matched_a_scan = true;
            // Every graph the scan PROVED a contribution from must be declared.
            for (g, &bit) in attribution.iter().enumerate() {
                if bit && !declared.contains(&g) {
                    return Err(CheckError::AttributionUnderDeclared {
                        pattern: pi,
                        proof_graph: g,
                    });
                }
            }
        }
        if !matched_a_scan {
            return Err(CheckError::AttributionUnbound { pattern: pi });
        }
    }
    Ok(())
}

/// Whether sub-proof `to_proof` is a `filter_int` whose bound `(op, bound)`
/// match the query FILTER and whose disclosed verdict is TRUE (audit #5/#6:
/// op/bound substitution + verdict gating). The operand-slot equality (the edge
/// references the FILTER variable's scanned column) is enforced by the caller;
/// the edge's scanned-slot encoding == the filter `operand_enc` is enforced by
/// stage 2 over the now-bb-bound values (audit #7).
fn filter_edge_true(
    manifest: &ProofManifest,
    to_proof: usize,
    op: FilterCmp,
    bound: u64,
) -> bool {
    match manifest.sub_proofs.get(to_proof).map(|sp| &sp.inputs) {
        Some(ProofInputs::FilterInt { op: f_op, bound: f_bound, expected, .. }) => {
            f_op.code() == op.code() && *f_bound == bound && *expected
        }
        _ => false,
    }
}

/// sq-cwq: verify the holder proof-of-possession when the binding is `HolderPop`.
/// Runs ONLY for a `HolderPop` binding (a `Challenge` binding returns `Ok(())`
/// immediately — no PoP is required). `challenge` is the VERIFIER'S nonce (the
/// same field element bound as public-input field 0 of every sub-proof), so the
/// PoP is fresh: a captured manifest re-presented under a new nonce cannot reuse
/// an old PoP, and the holder must sign THIS verifier's challenge.
///
/// Fail-closed contract (the sq-cwq fix — no silent accept of an absent PoP):
/// 1. an EMPTY `holder_registry` => `HolderRegistryEmpty` (no trust anchor — a
///    holder PoP cannot be accepted; the relying party must supply authorised
///    holder keys to use holder binding);
/// 2. `holder` not a member of the registry => `HolderNotTrusted`;
/// 3. an unknown `cryptosuite`, or a `holder`/`pop` that does not parse =>
///    `HolderPopMalformed` (prover-controlled bytes never panic);
/// 4. `pop` not a valid signature under `holder` over
///    `holder_pop_message(challenge)` => `HolderPopInvalid`.
///
/// The previous placeholder accepted a `HolderPop` binding by simply reading its
/// `challenge` (exactly like a bare `Challenge`), silently waiving (1)-(4) — an
/// absent/forged PoP passed. That silent-accept path is removed.
///
/// # T3/sq-z8s7 B1: issuer-attested credential↔holder binding (the gap closed)
/// The sq-cwq checks above prove the presenter possesses a holder key the relying
/// party trusts AND signed the verifier's fresh nonce with it — but they do NOT
/// bind that key to the SPECIFIC credential the scan/filter sub-proofs attest. So
/// trusted holder A could present trusted holder B's credential (A holds *a*
/// trusted key, signs the nonce, while the proofs attest B's credential). B1
/// closes this: after the registry+PoP checks, [`bind_holder_binding`]
/// cross-checks the PRESENTED holder key against the issuer-attested
/// `holder_pk_digest` the issuer folded into THIS credential's signature (the
/// `ZKSIG_C4` [`sparq_zk::sig::commitment_message_with_holder`] message, recovered
/// from the credential's [`crate::manifest::CommitmentAttestation`] and verified
/// under the EXTERNAL trusted `K`). On any mismatch / required-but-absent binding
/// / identity key it fails closed (`HolderKeyMismatch` / `HolderBindingMissing`).
///
/// # Scope (B2 deferred)
/// This is the CLEAR-KEY tier: the presented holder key is disclosed and the
/// verifier recomputes its digest host-side. The in-circuit HIDDEN-key PoK (B2,
/// sq-i1dt), where only the digest is public, is a separate deliverable — NOT
/// enforced here. See `research/zk-holder-pop-design.md` §2.B / §3.3.
// [OPUS-4.8] sq-cwq: holder PoP implemented (challenge-bound Schnorr) + fail-closed.
// [OPUS-4.8] sq-z8s7 (HolderPoP T3 / B1): + issuer-attested credential↔holder binding.
fn bind_holder_pop(
    manifest: &ProofManifest,
    holder_registry: &HolderRegistry,
    trusted_key_set: &KeySet,
    holder_binding_policy: &HolderBindingPolicy,
    challenge: &FieldHex,
) -> Result<(), CheckError> {
    let (holder, pop, cryptosuite) = match &manifest.binding {
        // A plain challenge binding requires no holder PoP.
        BindingMode::Challenge { .. } => return Ok(()),
        BindingMode::HolderPop { holder, pop, cryptosuite, .. } => (holder, pop, cryptosuite),
    };

    // (1) No trust anchor => cannot accept a holder PoP (fail-closed). This is the
    // load-bearing replacement for the old silent-accept: a HolderPop binding is
    // NEVER waived just because no registry was supplied.
    if holder_registry.is_empty() {
        return Err(CheckError::HolderRegistryEmpty);
    }
    // (2) The holder key must be authorised by the relying party's external set.
    if !holder_registry.contains_hex(holder) {
        return Err(CheckError::HolderNotTrusted { holder: holder.clone() });
    }
    // (3) Known cryptosuite + parseable key/signature (fail-closed on bad bytes).
    if SignatureScheme::from_cryptosuite_iri(cryptosuite).is_none() {
        return Err(CheckError::HolderPopMalformed);
    }
    let (Some(pk), Some(sig)) = (public_key_from_hex(holder), signature_from_hex(pop)) else {
        return Err(CheckError::HolderPopMalformed);
    };
    // The challenge must parse to a field element (it equals the verifier nonce,
    // already validated in verify_manifest, but check here for a standalone caller).
    let Some(challenge_fr) = challenge.to_field() else {
        return Err(CheckError::HolderPopMalformed);
    };
    // (4) The PoP signature must verify under the holder key over the
    // challenge-bound, domain-separated PoP message — proving possession of the
    // holder secret, freshly over the verifier's nonce.
    let message = holder_pop_message(&challenge_fr);
    if !sig_verify(&pk, &message, &sig) {
        return Err(CheckError::HolderPopInvalid { holder: holder.clone() });
    }

    // (5) [OPUS-4.8] sq-z8s7 (T3 / B1): bind THIS presented holder key to the
    // ISSUER-ATTESTED holder binding of THIS credential. `pk` is the SAME key the
    // PoP (4) was verified under, so the digest cross-check + the nonce-PoP
    // together bind THIS holder to THIS credential (closing the trusted-holder
    // gap: A's key digest != B's attested digest). Fail-closed.
    bind_holder_binding(manifest, trusted_key_set, holder_binding_policy, &pk)?;

    Ok(())
}

/// T3/sq-z8s7 B1 (clear-key tier): cross-check the PRESENTED holder key against
/// the credential's ISSUER-ATTESTED holder binding, closing the trusted-holder gap
/// `bind_holder_pop`'s nonce-only PoP left open. `presented_pk` is the holder key
/// the `HolderPop` PoP was verified under (so the digest check below + the existing
/// nonce-PoP together bind THIS holder to THIS credential).
///
/// # What it checks (design `research/zk-holder-pop-design.md` §3.3 B1 / §4.1)
/// 1. **Scope to the COVERING attestation, never any attestation (sq-z8s7 Copilot
///    scoping fix).** The binding is checked on the attestation that COVERS a
///    SCAN-REFERENCED commitment — i.e. the credential the presentation actually
///    uses — reusing the EXACT attestation→commitment mapping
///    [`bind_issuer_attestations`] uses (`a.commitment.to_field() == c_field` over
///    the per-graph `commitments` of every scan sub-proof). The earlier "ANY
///    `manifest.commitment_attestations` entry has `holder: Some(_)`" shortcut was
///    a SECURITY hole: a holder binding on an UNRELATED attestation (one covering
///    no scan-referenced commitment) could satisfy the check while the credential
///    genuinely presented was bearer/mismatched — the A-presents-B closure this
///    function exists to enforce would silently lapse. We now iterate the
///    scan-referenced commitments and look up THEIR covering attestation. (If
///    several scans reference the same commitment, it is checked once; a credential
///    has at most one covering attestation per commitment.)
/// 2. **Bearer policy (fail-closed, no silent fallback), per covering attestation.**
///    A scan-referenced commitment whose COVERING attestation carries no holder
///    binding (or that has no clear covering attestation at all — hidden-issuer or
///    unattested) is BEARER for this credential. Under
///    [`HolderBindingPolicy::require_binding`] that is rejected
///    ([`CheckError::HolderBindingMissing`] — bearer-where-binding-required);
///    under the back-compatible default it is accepted (the sq-cwq registry +
///    nonce-PoP guarantee stands). When NO holder binding covers ANY presented
///    credential the whole presentation is bearer (same policy split).
/// 3. **Anchor the digest in the issuer signature (NEVER the manifest alone).**
///    For a holder-bound attestation, the issuer signed
///    [`sparq_zk::sig::commitment_message_with_holder`]`(C(G), salt, status_ref, holder_pk_digest)`
///    (the distinct `ZKSIG_C4` tag). The verifier RECOMPUTES that message from the
///    disclosed `(C(G), salt, status_ref)` + the attested `holder_pk_digest` and
///    requires the issuer signature to verify under a key in the EXTERNAL trusted
///    `K` — so the digest the verifier cross-checks is the one the ISSUER bound,
///    not a free prover JSON field (design §4.3 obligation 1). A holder-bound
///    attestation whose signature does not so verify is rejected
///    ([`CheckError::InvalidIssuerSignature`]) — A cannot swap in its own digest
///    without invalidating the issuer's EUF-CMA signature.
/// 4. **Presented-key digest == attested digest.** `holder_key_digest(presented_pk)`
///    (T1, Poseidon2-collision-resistant) MUST equal the attested
///    `holder_pk_digest`. A's key digest != B's attested digest =>
///    [`CheckError::HolderKeyMismatch`] (the core trusted-holder-gap closure). The
///    identity holder key has no usable digest ([`sparq_zk::sig::HolderKeyError`])
///    => `HolderKeyMismatch`.
/// 5. **Clear attested key (if disclosed) == presented key.** When the attestation
///    discloses the clear `holder_public_key`, it MUST equal `presented_pk` (a
///    belt-and-braces check: the digest equality already implies key equality under
///    collision-resistance, but a disclosed clear key that disagrees is a malformed
///    binding => `HolderKeyMismatch`).
// [OPUS-4.8] sq-z8s7 (HolderPoP T3 / B1): verifier-side issuer-attested clear-key
// holder binding, fail-closed. B2 (hidden-key in-circuit PoK) is sq-i1dt.
fn bind_holder_binding(
    manifest: &ProofManifest,
    trusted_key_set: &KeySet,
    holder_binding_policy: &HolderBindingPolicy,
    presented_pk: &PublicKey,
) -> Result<(), CheckError> {
    // (1) [OPUS-4.8] sq-z8s7 (Copilot scoping fix): collect the attestations that
    // COVER a SCAN-REFERENCED commitment — the credential(s) the presentation
    // actually uses — using the EXACT attestation→commitment mapping
    // `bind_issuer_attestations` uses (`a.commitment.to_field() == c_field` over
    // the per-graph `commitments` of every scan sub-proof). This REPLACES the
    // earlier "ANY attestation has `holder: Some(_)`" shortcut, which was a
    // SECURITY hole: a holder binding on an UNRELATED attestation could satisfy the
    // check while the genuinely-presented credential was bearer/mismatched. We
    // build the covering set so a credential's bearer/holder-bound status is judged
    // on ITS OWN attestation, not on a stray sibling. Keyed by canonical commitment
    // hex so a commitment referenced by several scans is checked once; the value is
    // whether THAT commitment's covering attestation carries a holder binding (and,
    // if so, the binding itself for the cross-check below).
    let mut covering: std::collections::BTreeMap<
        String,
        Option<&crate::manifest::CommitmentAttestation>,
    > = std::collections::BTreeMap::new();
    for sp in &manifest.sub_proofs {
        let ProofInputs::Scan { commitments, .. } = &sp.inputs else {
            continue;
        };
        for c in commitments {
            let Some(c_field) = c.to_field() else {
                // A non-field commitment can carry no valid attestation. It is
                // already rejected upstream by `bind_issuer_attestations`
                // (`bind_holder_binding` runs after the issuer gate via
                // `verify_manifest`); skip it here so a malformed commitment cannot
                // mask a real scoping decision.
                continue;
            };
            // The covering attestation = the same lookup `bind_issuer_attestations`
            // uses (compare as field elements so 0x-padding cannot slip a mismatch).
            let att = manifest.commitment_attestations.iter().find(|a| {
                a.commitment.to_field().is_some() && a.commitment.to_field() == Some(c_field)
            });
            covering.insert(field_to_hex(&c_field), att);
        }
    }

    // The set of covering attestations that DO carry a holder binding — the only
    // attestations whose binding this credential's presentation must satisfy.
    let bound: Vec<&crate::manifest::CommitmentAttestation> = covering
        .values()
        .filter_map(|a| a.filter(|att| att.holder.is_some()))
        .collect();

    // (2) Bearer policy, scoped to the COVERING attestations. A presentation is
    // bearer iff NO scan-referenced commitment's covering attestation carries a
    // holder binding (a holder binding on an unrelated attestation no longer
    // counts). Under `require_binding` that is rejected fail-closed
    // (`HolderBindingMissing` — bearer-where-binding-required); under the
    // back-compatible default it is accepted (sq-cwq registry + nonce-PoP stands).
    if bound.is_empty() {
        if holder_binding_policy.requires_binding() {
            return Err(CheckError::HolderBindingMissing);
        }
        return Ok(());
    }

    // [OPUS-4.8] sq-z8s7 (Copilot scoping fix): under `require_binding`, EVERY
    // presented credential must be holder-bound — a scan-referenced commitment whose
    // covering attestation lacks a binding (or has no clear covering attestation at
    // all) is bearer-where-binding-required, rejected even though a SIBLING
    // commitment is bound. (Under the back-compatible default a mix is allowed: the
    // bound credentials are still cross-checked below; the bearer ones rely on the
    // registry + nonce-PoP.)
    if holder_binding_policy.requires_binding()
        && covering
            .values()
            .any(|a| a.is_none_or(|att| att.holder.is_none()))
    {
        return Err(CheckError::HolderBindingMissing);
    }

    // The presented key's digest (T1). The identity key has no usable digest and is
    // rejected fail-closed (it can never equal a real issuer-attested digest).
    let Ok(presented_digest) = holder_key_digest(presented_pk) else {
        return Err(CheckError::HolderKeyMismatch);
    };

    for att in bound {
        let binding = att
            .holder
            .as_ref()
            .expect("filtered for Some(holder) above");

        // (3) Anchor the attested digest in the ISSUER signature: recompute the
        // holder-bound (ZKSIG_C4) message and require the issuer signature to
        // verify under a key in the EXTERNAL trusted K. This is what makes the
        // attested `holder_pk_digest` trustworthy (design §4.3 obligation 1) —
        // without it the digest would be an unauthenticated prover JSON field.
        verify_holder_attestation_signature(manifest, trusted_key_set, att)?;

        // The attested digest as a field element (fail-closed on malformed hex).
        let Some(attested_digest) = binding.digest() else {
            return Err(CheckError::HolderKeyMismatch);
        };

        // (4) The presented key MUST hash to the issuer-attested digest. This is
        // the load-bearing trusted-holder-gap closure: trusted holder A presenting
        // trusted holder B's credential has A's digest != B's attested digest.
        if presented_digest != attested_digest {
            return Err(CheckError::HolderKeyMismatch);
        }

        // (5) If the attestation ALSO discloses the clear holder key (clear-key
        // tier), it must equal the presented key. Digest equality already implies
        // this under collision-resistance; a disclosed clear key that disagrees is
        // a malformed binding, rejected fail-closed.
        if let Some(clear) = binding.holder_key() {
            if clear != *presented_pk {
                return Err(CheckError::HolderKeyMismatch);
            }
        }
    }

    Ok(())
}

/// Verify that the issuer signature on a HOLDER-BOUND attestation verifies over
/// the `ZKSIG_C4` [`sparq_zk::sig::commitment_message_with_holder`] message under
/// a key in the EXTERNAL trusted `K` (T3/sq-z8s7 B1). This anchors the attested
/// `holder_pk_digest` in the issuer signature: the message folds
/// `(C(G), salt, status_ref, holder_pk_digest)`, so a forged/swapped digest yields
/// a different message and no valid signature (EUF-CMA). Mirrors the salt/status
/// recompute in [`bind_issuer_attestations`], but over the holder-bound message
/// variant (the status path verifies the status-only `ZKSIG_C3` message, which a
/// holder-bound attestation does NOT use).
///
/// Fail-closed: an issuer key not in `K` => [`CheckError::IssuerKeyNotInKeySet`];
/// an unknown cryptosuite / unparseable key or signature / malformed salt or
/// commitment / a missing or non-verifying signature => [`CheckError::InvalidIssuerSignature`];
/// an absent revocation reference (needed to recompute `status_ref`) =>
/// [`CheckError::RevocationReferenceMissing`].
// [OPUS-4.8] sq-z8s7 (HolderPoP T3 / B1): issuer-signature anchor for the holder digest.
fn verify_holder_attestation_signature(
    manifest: &ProofManifest,
    trusted_key_set: &KeySet,
    att: &crate::manifest::CommitmentAttestation,
) -> Result<(), CheckError> {
    let commitment_hex = att.commitment.0.clone();
    // The issuer key MUST be in the EXTERNAL trusted K (never a prover-chosen key —
    // the audit-#3 codex-#1 anchor; reported BEFORE the signature check so an
    // untrusted issuer is the stated reason).
    if !trusted_key_set.contains_hex(&att.issuer_public_key) {
        return Err(CheckError::IssuerKeyNotInKeySet {
            commitment: commitment_hex,
        });
    }

    let binding = att
        .holder
        .as_ref()
        .expect("caller passes a holder-bound attestation");
    let (Some(commitment_fr), Some(holder_digest)) = (att.commitment.to_field(), binding.digest())
    else {
        return Err(CheckError::InvalidIssuerSignature {
            commitment: commitment_hex,
        });
    };
    // A holder-bound (scan-covering) attestation carries a salt exactly as a
    // status-bound one does; an absent/malformed salt fails closed.
    let Some(salt_fr) = att.salt.as_ref().and_then(|s| s.to_field()) else {
        return Err(CheckError::InvalidIssuerSignature {
            commitment: commitment_hex,
        });
    };
    // Recompute the issuer-signed status reference from the disclosed revocation
    // reference (same source `bind_issuer_attestations` uses) — it is folded into
    // the ZKSIG_C4 message alongside the holder digest.
    let Some(rev) = &manifest.revocation else {
        return Err(CheckError::RevocationReferenceMissing { proof: 0 });
    };
    let Some(status_ref) = status_ref_from_revocation(rev) else {
        return Err(CheckError::InvalidIssuerSignature {
            commitment: commitment_hex,
        });
    };

    let message =
        commitment_message_with_holder(&commitment_fr, &salt_fr, &status_ref, &holder_digest);
    let ok = SignatureScheme::from_cryptosuite_iri(&att.cryptosuite).is_some()
        && match (
            public_key_from_hex(&att.issuer_public_key),
            signature_from_hex(&att.signature),
        ) {
            (Some(pk), Some(sig)) => sig_verify(&pk, &message, &sig),
            _ => false,
        };
    if !ok {
        return Err(CheckError::InvalidIssuerSignature {
            commitment: commitment_hex,
        });
    }
    Ok(())
}

/// Encode an IRI to its salt-independent term encoding `FieldHex` (the same
/// encoding scans disclose), `None` if it does not encode. Used by
/// [`bind_entailment`] to compute the RDFS schema-vocabulary encodings.
// [OPUS-4.8] sq-314.
fn encode_iri_hex(iri: &str) -> Option<FieldHex> {
    let nn = oxrdf::NamedNode::new(iri).ok()?;
    let enc = encode_term(&oxrdf::Term::NamedNode(nn), &Fr::from(0u64))?;
    Some(FieldHex(field_to_hex(&enc)))
}

/// sq-314: enforce the manifest's entailment regime end-to-end. This makes
/// `entailment_regime` a CHECKED claim rather than free metadata.
///
/// Fail-closed contract:
/// 0. if the relying party requires COMPLETENESS under entailment
///    ([`EntailmentPolicy::require_completeness_under_entailment`]), every
///    non-`Simple` manifest is REFUSED first — the capability is unbuilt
///    (`sq-rsd3v.7`) — else `CompletenessUnderEntailmentUnavailable`;
/// 1. the regime MUST be accepted by the relying party's [`EntailmentPolicy`]
///    (`Simple` always; `Rdfs`/`Owl` only on explicit opt-in) — else
///    `EntailmentRegimeNotAccepted`;
/// 2. a `Simple` manifest MUST carry NO derivation steps (no inference) — else
///    `UnexpectedDerivationSteps`;
/// 3. a non-`Simple` manifest MUST carry derivation steps — else
///    `MissingDerivationSteps`;
/// 4. every derivation step MUST be a well-formed, regime-admitted rule instance
///    (`MalformedDerivationStep`), and every antecedent MUST be GROUNDED: equal to
///    an EARLIER step's derived triple or to a triple disclosed by a scan
///    sub-proof (the asserted base) — else `UngroundedDerivationAntecedent`.
///
/// Grounding is by TERM-ENCODING equality (the disclosed scan rows and the step
/// triples are both `FieldHex` encodings), so an antecedent that equals a
/// disclosed asserted triple is soundly grounded. Step ordering is significant:
/// each step may only ground on STRICTLY EARLIER steps (a forward-chained
/// derivation), so there are no cyclic self-justifications.
///
/// # Honest scope (what is NOT proved HERE — this host re-check)
/// Grounding ties antecedents to the DISCLOSED base; THIS host path does NOT
/// prove in zero-knowledge that an undisclosed antecedent is in the committed
/// graph's closure. An antecedent that is neither disclosed nor chained to a
/// disclosed triple is REJECTED (not assumed). This stage makes the regime claim
/// non-vacuous and auditable over the disclosed-base fragment, fail-closed.
///
/// The in-circuit single-step privacy upgrade now exists as a research-grade Noir
/// relation (`zk/compose/compose_core::entail`, sq-g91d — undisclosed antecedents
/// proven members of the committed graph); it is NOT-yet-sound (sq-qhy4) and NOT
/// yet wired into this verifier (no compiled member / manifest variant / dispatch
/// arm), so until that follow-up lands this path stays disclosed-base only. See
/// the `crate::derivation` module docs.
///
/// Everything above is SOUNDNESS of derivation ("every derived triple IS
/// entailed"). It is NOT completeness under entailment ("no entailed answer is
/// MISSING") — the distinct, UNBUILT obligation `sq-rsd3v.7`. Gate (0) below is
/// the enforced deferral: a relying party that demands completeness is REFUSED
/// before any other check, so the two obligations can never be conflated by
/// reading an accept.
// [OPUS-4.8] sq-314: entailment regime + derivation steps, end-to-end.
fn bind_entailment(
    manifest: &ProofManifest,
    policy: &EntailmentPolicy,
) -> Result<(), CheckError> {
    let regime = manifest.entailment_regime;
    let regime_name = match regime {
        EntailmentRegime::Simple => "simple",
        EntailmentRegime::Rdfs => "rdfs",
        EntailmentRegime::Owl => "owl",
    };
    // (0) sq-rsd3v.7: the relying party demands COMPLETENESS under entailment. The
    // capability is UNBUILT (no in-circuit closure-sweep, no fixpoint-saturation
    // proof), so any manifest that RESTS on entailment is refused here rather than
    // accepted on soundness-of-derivation grounds the relying party could misread
    // as completeness. Checked FIRST so the diagnostic names the real gap (the
    // unbuilt obligation) rather than the incidental one (regime not accepted).
    // `Simple` is not refused: it carries no entailment for completeness to range
    // over — see `require_completeness_under_entailment` for what that does and
    // does NOT assert.
    if policy.requires_completeness() && regime != EntailmentRegime::Simple {
        return Err(CheckError::CompletenessUnderEntailmentUnavailable { regime: regime_name });
    }
    // (1) The regime must be accepted by the relying party.
    if !policy.accepts(regime) {
        return Err(CheckError::EntailmentRegimeNotAccepted { regime: regime_name });
    }
    let steps = &manifest.derivation_steps;
    // (2) Simple => no inference steps.
    if regime == EntailmentRegime::Simple {
        if !steps.is_empty() {
            return Err(CheckError::UnexpectedDerivationSteps);
        }
        return Ok(());
    }
    // (3) Non-Simple => steps required.
    if steps.is_empty() {
        return Err(CheckError::MissingDerivationSteps { regime: regime_name });
    }

    // The asserted base: every triple disclosed by a scan sub-proof (active rows),
    // as [s, p, o] encodings — the ground set a step may chain from.
    let mut disclosed: BTreeSet<[String; 3]> = BTreeSet::new();
    for sp in &manifest.sub_proofs {
        if let ProofInputs::Scan { rows, row_count, .. } = &sp.inputs {
            for row in rows.iter().take(*row_count as usize) {
                disclosed.insert([
                    normalize_hex(&row[0].0),
                    normalize_hex(&row[1].0),
                    normalize_hex(&row[2].0),
                ]);
            }
        }
    }

    // RDFS schema-vocabulary encodings (salt-independent IRIs). If any fails to
    // encode the whole check fails closed (it cannot happen for these constants).
    // [OPUS-5] sq-rsd3v.6: `owl:sameAs` joins the list — not as a rule term, but
    // so the equality guard below can recognise (and refuse) an equality fact.
    let (
        Some(rdf_type),
        Some(rdfs_subclassof),
        Some(rdfs_subpropertyof),
        Some(rdfs_domain),
        Some(rdfs_range),
        Some(owl_sameas),
    ) = (
        encode_iri_hex("http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
        encode_iri_hex("http://www.w3.org/2000/01/rdf-schema#subClassOf"),
        encode_iri_hex("http://www.w3.org/2000/01/rdf-schema#subPropertyOf"),
        encode_iri_hex("http://www.w3.org/2000/01/rdf-schema#domain"),
        encode_iri_hex("http://www.w3.org/2000/01/rdf-schema#range"),
        encode_iri_hex(crate::sameas::OWL_SAME_AS),
    ) else {
        return Err(CheckError::MalformedDerivationStep { step: 0 });
    };

    // (4) Re-check each step. `derived_so_far` accumulates the derived triples of
    // EARLIER steps (forward chaining only — no cyclic self-grounding).
    let mut derived_so_far: BTreeSet<[String; 3]> = BTreeSet::new();
    for (si, step) in steps.iter().enumerate() {
        // 4a-0. EQUALITY GUARD (sq-rsd3v.6): `owl:sameAs` must never ride the
        // fixed-shape path. Checked BEFORE the shape check, because the shapes
        // that matter here are shape-VALID (an `rdfs7` whose sub-property is
        // `owl:sameAs` consumes an equality; one whose super-property is
        // `owl:sameAs` introduces one), so shape alone would let them through.
        if step.mentions_equality_predicate(&owl_sameas) {
            return Err(CheckError::EqualityReasoningUnsupported { step: si });
        }
        // 4a. Well-formed rule instance AND the regime admits the rule.
        if !regime_admits(regime, step.rule)
            || !step.is_well_formed(
                &rdf_type,
                &rdfs_subclassof,
                &rdfs_subpropertyof,
                &rdfs_domain,
                &rdfs_range,
            )
        {
            return Err(CheckError::MalformedDerivationStep { step: si });
        }
        // 4b. Every antecedent grounded (disclosed base OR an earlier derived).
        for (ai, ant) in step.antecedents.iter().enumerate() {
            let key = [
                normalize_hex(&ant[0].0),
                normalize_hex(&ant[1].0),
                normalize_hex(&ant[2].0),
            ];
            if !disclosed.contains(&key) && !derived_so_far.contains(&key) {
                return Err(CheckError::UngroundedDerivationAntecedent {
                    step: si,
                    antecedent: ai,
                });
            }
        }
        // This step's derived triple is now available to ground later steps.
        derived_so_far.insert([
            normalize_hex(&step.derived[0].0),
            normalize_hex(&step.derived[1].0),
            normalize_hex(&step.derived[2].0),
        ]);
    }
    Ok(())
}

/// Full verification: structure (stage 1+2) then the cryptographic gate
/// (stage 3). `prover` points at the `zk/compose/` workspace; `work_dir` is
/// scratch for bb artifacts; `trusted_key_set` is the relying party's EXTERNAL
/// issuer trust anchor `K` (audit #3 codex #1 — never the prover's
/// `manifest.key_set`); `revocation_policy` is the relying party's EXTERNAL
/// freshness/revocation policy (audit #12 — the status check is mandatory: a
/// revoked credential, a stale status snapshot, or an omitted/forged
/// issuer-bound status reference all REJECT); `holder_registry` is the relying
/// party's EXTERNAL set of authorised holder keys (sq-cwq) — consulted ONLY when
/// `manifest.binding` is `HolderPop`, in which case the holder MUST prove
/// possession of a registry-member key by signing the verifier nonce (an empty
/// registry, an untrusted holder, or a malformed/invalid PoP all REJECT; a
/// `Challenge` binding ignores the registry). Pass `&HolderRegistry::empty()`
/// when holder binding is not in use. `entailment_policy` is the relying party's
/// EXTERNAL entailment-regime policy (sq-314): which regimes it accepts. The
/// regime is enforced fail-closed — a regime the policy rejects, or a non-`Simple`
/// regime whose `derivation_steps` do not structurally ground every derived triple
/// to the disclosed base, REJECTS. Pass `&EntailmentPolicy::simple_only()` (the
/// default) to accept only non-inference proofs.
///
/// # Freshness / single-use (audit #4) — the challenge-response flow
/// `nonce` is the relying party's OWN fresh value (a [`VerifierNonce`]), minted
/// out of band and handed to the prover BEFORE proving. The honest flow is a
/// three-step challenge-response:
/// 1. **Verifier → prover:** the relying party mints a fresh `nonce` and sends
///    it. (Unpredictability/uniqueness is the relying party's responsibility; a
///    CSPRNG value reduced into the field is the expected source.)
/// 2. **Prover:** proves with `nonce` as the circuit `challenge` public input
///    (field 0) — the prove path threads it through `toml.rs` / `build.rs`, so
///    NO circuit change is needed (field 0 is an unconstrained in-circuit tag;
///    the binding is verifier-side).
/// 3. **Verifier:** this function (a) records the nonce single-use via `seen`
///    BEFORE the crypto gate (a second presentation of the same nonce =>
///    [`CheckError::NonceReplay`]); (b) asserts `manifest.binding`'s declared
///    challenge equals `nonce` (fail-closed JSON consistency); and (c)
///    reconstructs the public-input vector using `nonce` (NOT `manifest.binding`)
///    as field 0, so the audit-#1 byte-compare rejects any proof whose committed
///    challenge ≠ the verifier's nonce. A CAPTURED manifest re-presented under a
///    NEW verifier nonce therefore fails the byte-compare (its proof committed
///    the OLD nonce); the same manifest re-presented under its ORIGINAL nonce
///    fails the single-use store. Both replay vectors are closed fail-closed —
///    there is no "skip when a binding field is absent" path.
///
/// Stage 3, per sub-proof, binds the declared statement to the proof (audit
/// #1/#2): (a) reconstruct the public-input byte vector from the DECLARED
/// `ProofInputs` using the VERIFIER'S NONCE as field 0 and assert byte-equality
/// with the proof's `public_inputs`; (b) recompute the CANONICAL member vk
/// verifier-side; (c) `bb verify` over (prover proof, reconstructed public
/// inputs, canonical vk). The prover-supplied vk and public-input bytes from
/// the blob are NEVER trusted.
// [OPUS-4.8] audit #4: verifier-issued nonce + single-use.
// [OPUS-4.8] sq-cwq: + holder_registry (external holder trust anchor). Each
// argument is a DISTINCT external trust input the relying party supplies (issuer
// key-set K, revocation policy, holder registry, fresh nonce, single-use store) —
// deliberately separate, not bundled, to keep each anchor explicit at the call
// site; the count exceeds clippy's default heuristic but every arg is load-bearing.
// [OPUS-4.8] sq-z8s7 (T3 / B1): + holder_binding_policy (external; whether a bearer
// credential is rejected under HolderPop — `HolderBindingPolicy::require_binding()`).
#[allow(clippy::too_many_arguments)]
pub fn verify_manifest(
    manifest: &ProofManifest,
    prover: &CircuitProver,
    work_dir: &Path,
    trusted_key_set: &KeySet,
    revocation_policy: &RevocationPolicy,
    holder_registry: &HolderRegistry,
    holder_binding_policy: &HolderBindingPolicy,
    entailment_policy: &EntailmentPolicy,
    nonce: &VerifierNonce,
    seen: &dyn SeenNonces,
) -> Result<(), CheckError> {
    // The sound stage-1 entry point: the FLAT-fragment regime
    // (`skip_query_binding = false`) keeps EVERY stage-1 query-text gate live, so
    // this path is byte-identical with or without the `extended-fragment` feature.
    verify_manifest_impl(
        manifest,
        prover,
        work_dir,
        trusted_key_set,
        revocation_policy,
        holder_registry,
        holder_binding_policy,
        entailment_policy,
        nonce,
        seen,
        false,
    )
}

/// Shared crypto-verification body of [`verify_manifest`] (and, under
/// `extended-fragment`, `verify_fragment_manifest`). `skip_query_binding` is
/// threaded straight into `prefilter_manifest_structure_impl` and selects the
/// stage-1 query-text binding regime (flat vs extended fragment); it changes
/// NOTHING else. Every other gate — the entailment regime, the verifier-nonce
/// single-use + challenge binding, holder PoP, the per-sub-proof public-input
/// reconstruction + canonical-vk + `bb verify`, and the hidden revocation /
/// issuer / holder gates — runs identically in both regimes.
// [OPUS-4.8] sq-h732x: mode-aware shared body (flat vs extended fragment).
#[allow(clippy::too_many_arguments)]
fn verify_manifest_impl(
    manifest: &ProofManifest,
    prover: &CircuitProver,
    work_dir: &Path,
    trusted_key_set: &KeySet,
    revocation_policy: &RevocationPolicy,
    holder_registry: &HolderRegistry,
    holder_binding_policy: &HolderBindingPolicy,
    entailment_policy: &EntailmentPolicy,
    nonce: &VerifierNonce,
    seen: &dyn SeenNonces,
    skip_query_binding: bool,
) -> Result<(), CheckError> {
    prefilter_manifest_structure_impl(
        manifest,
        trusted_key_set,
        revocation_policy,
        skip_query_binding,
    )?;

    // --- sq-314: entailment regime + derivation steps (fail-closed). ---
    // Enforce `manifest.entailment_regime` against the relying party's policy so
    // it is a CHECKED claim, not free metadata: a regime the policy rejects, a
    // Simple manifest carrying inference steps, a non-Simple manifest with no
    // steps, or a derivation step that is malformed / ungrounded all REJECT. This
    // is a pure-JSON structural check (no bb); placed before the nonce burn since
    // it is independent of the crypto gate.
    bind_entailment(manifest, entailment_policy)?;

    // --- Audit #4: single-use (fail-closed, BEFORE the crypto gate). ---
    // Record the verifier's nonce as used; reject if it was already seen. Doing
    // this first means a replayed (nonce, manifest) pair is rejected without
    // even running bb. The store is consulted unconditionally — there is no
    // opt-out path that could bypass single-use (the parameter is mandatory).
    //
    // [OPUS-4.8] sq-3v2 — BURN-ON-MISMATCH is intentional. Because this records
    // FIRST, the verifier nonce is CONSUMED even when the manifest is subsequently
    // rejected (NonceBindingMismatch, MalformedProof, a bb failure, …). That is the
    // chosen freshness/replay policy: a verifier-issued nonce is spent the moment it
    // is PRESENTED, so a rejection is never a free retry — an attacker who captured
    // a nonce cannot use ANY rejection as an oracle to probe-and-retry the SAME
    // nonce (a re-presentation is a flat NonceReplay). It cannot harm an honest
    // prover (whose binding == nonce, so it never takes the mismatch path; and a
    // fresh session always mints a new nonce). The cost — a transient/buggy
    // submission spends the nonce — is accepted in exchange for the strict
    // no-retry-after-rejection property; the relying party simply issues a new
    // nonce. The `nonce_binding_mismatch_rejected` e2e test asserts this exact
    // policy (a re-presentation under the burnt nonce returns NonceReplay).
    if !seen.record_fresh(nonce) {
        return Err(CheckError::NonceReplay);
    }

    // --- Audit #4: nonce/binding consistency (fail-closed). ---
    // The challenge that MUST appear as public-input field 0 of every member is
    // the VERIFIER'S nonce, NOT the prover-written `manifest.binding`. We still
    // require the declared `binding` challenge to EQUAL the nonce so an honest
    // manifest is internally consistent and a manifest minted for a different
    // nonce/session is rejected explicitly (rather than only failing the
    // byte-compare further down). The load-bearing freshness anchor, though, is
    // the nonce fed into `reconstruct_public_inputs` below — the JSON `binding`
    // is no longer trusted as the challenge source (closing the audit-#4
    // single-JSON-substitution rebind).
    let challenge = nonce.as_field_hex();
    let declared_binding_challenge = match &manifest.binding {
        BindingMode::Challenge { challenge } => challenge,
        BindingMode::HolderPop { challenge, .. } => challenge,
    };
    // Compare as field elements so 0x-padding differences don't spuriously
    // diverge; a malformed declared binding challenge fails closed.
    let declared_fr = declared_binding_challenge.to_field();
    if declared_fr.is_none() || declared_fr != challenge.to_field() {
        return Err(CheckError::NonceBindingMismatch);
    }

    // --- sq-cwq: holder proof-of-possession (fail-closed for HolderPop). ---
    // When the binding is `HolderPop`, the holder MUST prove possession of a
    // relying-party-trusted holder key by signing the VERIFIER'S nonce (above);
    // an absent registry, an untrusted holder, or a malformed/invalid PoP all
    // REJECT here — there is no silent-accept of a HolderPop as a bare challenge.
    // A `Challenge` binding requires no PoP (this returns Ok immediately). The
    // nonce is recorded single-use BEFORE this, so a rejected PoP still burns the
    // nonce (consistent with the burn-on-mismatch policy above).
    //
    // [OPUS-4.8] sq-z8s7 (T3 / B1): this ALSO cross-checks the presented holder key
    // against the credential's ISSUER-ATTESTED holder binding (the digest the issuer
    // folded into commitment_message_with_holder, verified under the external K),
    // and — under `holder_binding_policy.require_binding()` — rejects a bearer
    // credential presented under HolderPop. The trusted-holder gap is closed at the
    // clear-key tier here.
    bind_holder_pop(
        manifest,
        holder_registry,
        trusted_key_set,
        holder_binding_policy,
        &challenge,
    )?;

    for (i, sp) in manifest.sub_proofs.iter().enumerate() {
        if sp.proof_hex.is_empty() {
            return Err(CheckError::MissingProof { proof: i });
        }
        // Hardening: prover-controlled bytes are rejected, never panicked on.
        let blob = hex_decode(&sp.proof_hex)
            .ok_or(CheckError::MalformedProof { proof: i })?;
        let art = decode_artifacts(&blob).ok_or(CheckError::MalformedProof { proof: i })?;

        // (a) Reconstruct public inputs from the DECLARED statement (audit #1)
        // using the VERIFIER'S NONCE as field 0 (audit #4) and assert
        // byte-equality with the proof's public_inputs. This is the single
        // load-bearing binding: stages 1-2 check JSON, the proof is a detached
        // crypto object, and THIS ties them to the same statement AND to the
        // verifier's fresh nonce (a proof committed under a different challenge
        // cannot byte-match — closing replay).
        let reconstructed = reconstruct_public_inputs(&sp.inputs, &challenge, i)?;
        if reconstructed != art.public_inputs {
            return Err(CheckError::PublicInputMismatch { proof: i });
        }

        // (b) Recompute the canonical vk verifier-side from the member named by
        // the re-derived circuit id — NEVER the prover's art.vk (audit #2).
        // derive_id already passed in stage 1b, so the declared id is sound to
        // select the member by; recompute against it (full CircuitId pins the
        // n/d/r bucket, subsuming audit #11).
        let id = sp.inputs.circuit_id();
        let sub_work = work_dir.join(format!("sub{i}"));
        let canonical_vk = prover
            .canonical_vk(id, &sub_work.join("vk"))
            .map_err(CheckError::Driver)?;

        // (c) bb verify over (prover proof, OUR reconstructed public inputs, OUR
        // canonical vk). We pass the reconstructed public inputs (byte-equal to
        // the proof's, asserted above) so a single authentic vector is used end
        // to end.
        let ok = prover
            .verify_with(&art.proof, &reconstructed, &canonical_vk, &sub_work.join("verify"))
            .map_err(CheckError::Driver)?;
        if !ok {
            return Err(CheckError::ProofRejected { proof: i });
        }
    }

    // --- sq-3e5 / sq-h2v: hidden-index revocation cryptographic gate. ---
    // If the manifest carries a hidden-index revocation proof, verify it against
    // the relying party's OWN authoritative Merkle root (derived from its
    // authoritative snapshot) and the verifier's nonce. The clear-index liveness
    // gate (bind_revocation, in the prefilter above) is UNCHANGED and still runs;
    // this is the additive privacy upgrade that lets the holder NOT disclose its
    // index. The challenge fed here is the verifier's nonce (audit #4), identical
    // to the sub-proof loop's binding.
    bind_hidden_revocation(manifest, revocation_policy, prover, work_dir, &challenge)?;

    // --- sq-kndw: FULLY-HIDDEN revocation cryptographic gate. ---
    // If the manifest carries a fully-hidden revocation proof, verify it against the
    // relying party's OWN accepted-set root + epoch floor (derived from its curated
    // authoritative snapshots) and the verifier's nonce, and enforce single-use of
    // the (ref_commitment, index_commitment) linkage pair through the same durable
    // store the nonce replay defence uses. The clear-index and committed-index
    // liveness gates are UNCHANGED and still run for their own modes; this is the
    // additive privacy upgrade that also hides the status-list IRI and the version.
    bind_fully_hidden_revocation(
        manifest,
        revocation_policy,
        prover,
        work_dir,
        &challenge,
        seen,
    )?;

    // --- sq-z9l: hidden-issuer attestation cryptographic gate. ---
    // If the manifest carries hidden-issuer attestation proofs, verify each
    // against the relying party's OWN authoritative key-set Merkle root (derived
    // from its trusted KeySet) and the verifier's nonce. The clear-key
    // attestation gate (bind_issuer_attestations, in the prefilter above) is
    // UNCHANGED and still runs; this is the additive privacy upgrade that lets the
    // holder NOT disclose WHICH issuer signed. The challenge fed here is the
    // verifier's nonce (audit #4), identical to the sub-proof loop's binding.
    bind_hidden_issuer_attestations(manifest, trusted_key_set, prover, work_dir, &challenge)?;

    // --- sq-c2ql: in-circuit holder Proof-of-Possession cryptographic gate (B2). ---
    // If the manifest carries in-circuit holder PoK proofs (or the policy mandates
    // them), verify each against the ISSUER-ATTESTED holder digest of the covering
    // credential (the binding edge: the proven hidden holder key is the one the
    // issuer signed into THIS credential) and the verifier's nonce, then bb verify.
    // The clear-key holder gate (bind_holder_pop, above) is UNCHANGED and still runs;
    // this is the additive HIDDEN-key tier. NOT-yet-sound (sq-qhy4); opt-in. The
    // challenge fed here is the verifier's nonce (audit #4), identical to the
    // sub-proof loop's binding.
    bind_holder_pok(
        manifest,
        trusted_key_set,
        holder_binding_policy,
        prover,
        work_dir,
        &challenge,
    )?;

    // --- sq-3c00: hidden-holder SET-membership cryptographic gate. ---
    // If the manifest carries hidden-holder set-membership proofs, verify each
    // against the relying party's OWN authoritative holder-set Merkle root (derived
    // from its HolderRegistry) and the verifier's nonce, then bb verify. The
    // clear-key / clear-digest holder gates (bind_holder_pop, bind_holder_pok,
    // above) are UNCHANGED and still run; this is the additive hidden-holder
    // anonymity tier (hides WHICH holder). NOT-yet-sound (sq-qhy4); opt-in. The
    // challenge fed here is the verifier's nonce (audit #4), identical to the
    // sub-proof loop's binding.
    bind_holder_set(manifest, holder_registry, prover, work_dir, &challenge)?;

    Ok(())
}

/// Why a wave-1 extended-fragment manifest was REJECTED by [`dispatch_fragment`]
/// (sq-3kd2g.6). Every variant is a FAIL-CLOSED refusal — the gate never falls
/// through to a default / silent accept for anything outside the proven-sound
/// fragment.
// [OPUS-4.8] sq-3kd2g.6. Opt-in (`extended-fragment`), NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentDispatchError {
    /// The query is OUTSIDE the wave-1 extended fragment: `fragment_query`
    /// (re-derived from the query text alone, never the manifest) rejected it
    /// (e.g. `OPTIONAL` / `MINUS` / `GRAPH` / `SERVICE` / a non-atomic closure).
    /// Carries the underlying reason.
    OutsideFragment(String),
    /// A [`crate::manifest::BranchWitness`] attributes a disclosed solution to a
    /// `UNION` branch index that does not exist (the "wrong branch" rejection).
    BranchOutOfRange { witness: usize, branch: usize, branches: usize },
    /// A branch witness's per-obligation arity (`scan_proofs` / `path_proofs` /
    /// `values_rows`) does not match the re-derived branch's obligation count.
    ObligationArityMismatch { witness: usize, what: &'static str, expected: usize, got: usize },
    /// A named `sub_proofs` index is out of range of the embedded manifest.
    DanglingProof { witness: usize, proof: usize },
    /// A sub-proof carries an unknown / uncompiled circuit id (`derive_id` =>
    /// None) — e.g. a `PathReach` whose disclosed `depth_bound` is not a compiled
    /// member's bound.
    UnknownCircuit { proof: usize },
    /// A sub-proof's DECLARED circuit id does not equal the id re-derived from its
    /// public inputs — the "k (or n/d) mismatch between claim and circuit member"
    /// rejection (e.g. a `PathReach` declaring `k = 2` but carrying one
    /// commitment). Same invariant `prefilter_manifest_structure` stage-1b
    /// enforces, checked here too so the routing gate is self-contained.
    CircuitIdMismatch { proof: usize, declared: CircuitId, derived: CircuitId },
    /// A BGP-scan obligation's named sub-proof is not a [`ProofInputs::Scan`].
    NotAScanProof { witness: usize, obligation: usize, proof: usize },
    /// A bounded-path obligation has NO bound [`ProofInputs::PathReach`] sub-proof
    /// of the right member at its named index (the "path claimed without a bound
    /// sub-proof" rejection).
    ///
    /// The depth-overflow / mismatch case — a `PathReach` whose disclosed
    /// `depth_bound` is not the member's compiled `d` (soundness req 1) — is
    /// rejected EARLIER, by the id-hygiene check, as
    /// [`FragmentDispatchError::UnknownCircuit`] (`derive_id` returns `None` when
    /// `depth_bound != d`), so it never reaches this per-obligation stage.
    PathReachMissing { witness: usize, obligation: usize, proof: usize },
    /// A path sub-proof's `allow_zero` disagrees with the query-re-derived
    /// closure (`p+` cannot be presented as `p*`/`p?` or vice versa).
    PathClosureMismatch { witness: usize, obligation: usize },
    /// A FIXED-depth closure (`p?`, whose bound is pinned to 1) is bound to a
    /// member whose depth `d` exceeds that fixed bound.
    PathDepthExceedsClosure { witness: usize, obligation: usize, member_d: u32, fixed: usize },
    /// A VALUES-row index is out of range of the re-derived block's rows.
    ValuesRowOutOfRange { witness: usize, block: usize, row: usize, rows: usize },
}

#[cfg(feature = "extended-fragment")]
impl std::fmt::Display for FragmentDispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FragmentDispatchError::OutsideFragment(why) => {
                write!(f, "query is outside the wave-1 extended fragment: {}", why)
            }
            FragmentDispatchError::BranchOutOfRange { witness, branch, branches } => write!(
                f,
                "branch witness {} attributes a solution to branch {} but the query \
                 re-derives only {} branch(es) (fail-closed: wrong branch)",
                witness, branch, branches
            ),
            FragmentDispatchError::ObligationArityMismatch { witness, what, expected, got } => write!(
                f,
                "branch witness {} has {} {} proof-refs but the branch has {} {} obligation(s)",
                witness, got, what, expected, what
            ),
            FragmentDispatchError::DanglingProof { witness, proof } => write!(
                f,
                "branch witness {} names sub-proof {} which is out of range",
                witness, proof
            ),
            FragmentDispatchError::UnknownCircuit { proof } => write!(
                f,
                "sub-proof {} carries an unknown / uncompiled circuit id (fail-closed)",
                proof
            ),
            FragmentDispatchError::CircuitIdMismatch { proof, declared, derived } => write!(
                f,
                "sub-proof {}: declared circuit id {:?} but its public inputs re-derive {:?} \
                 (fail-closed: claim / circuit-member mismatch)",
                proof, declared, derived
            ),
            FragmentDispatchError::NotAScanProof { witness, obligation, proof } => write!(
                f,
                "branch witness {} BGP obligation {} names sub-proof {}, which is not a scan proof",
                witness, obligation, proof
            ),
            FragmentDispatchError::PathReachMissing { witness, obligation, proof } => write!(
                f,
                "branch witness {} path obligation {} names sub-proof {}, which is not a \
                 bound path_reach proof (fail-closed: path claimed without a bound sub-proof)",
                witness, obligation, proof
            ),
            FragmentDispatchError::PathClosureMismatch { witness, obligation } => write!(
                f,
                "branch witness {} path obligation {}: allow_zero disagrees with the query closure",
                witness, obligation
            ),
            FragmentDispatchError::PathDepthExceedsClosure { witness, obligation, member_d, fixed } => {
                write!(
                    f,
                    "branch witness {} path obligation {}: member depth {} exceeds the closure's fixed \
                     bound {} (fail-closed)",
                    witness, obligation, member_d, fixed
                )
            }
            FragmentDispatchError::ValuesRowOutOfRange { witness, block, row, rows } => write!(
                f,
                "branch witness {} VALUES block {}: row index {} out of range ({} row(s))",
                witness, block, row, rows
            ),
        }
    }
}

#[cfg(feature = "extended-fragment")]
impl std::error::Error for FragmentDispatchError {}

/// FAIL-CLOSED wave-1 extended-fragment DISPATCH (sq-3kd2g.6): route a
/// [`crate::manifest::FragmentManifest`] to the circuit members the query's
/// property-path / `UNION` / `VALUES` constructs require, REFUSING anything
/// outside the proven-sound fragment — never a silent fallback.
///
/// The gate re-derives the query's branch structure from the query TEXT alone
/// (`sparq_zk::verify::fragment_query`, never trusting the manifest) and enforces,
/// fail-closed, the load-bearing routing invariant (design record §3–§4):
///
/// 1. the query is IN the wave-1 fragment ([`FragmentDispatchError::OutsideFragment`]
///    otherwise — everything the record excludes still rejects);
/// 2. EVERY embedded sub-proof carries a KNOWN, compiled circuit id (an unknown /
///    uncompiled id => [`FragmentDispatchError::UnknownCircuit`]);
/// 3. each disclosed solution's [`crate::manifest::BranchWitness`] attributes it to
///    a REAL branch ([`FragmentDispatchError::BranchOutOfRange`] — the "wrong
///    branch" rejection) and names EXACTLY one bound sub-proof per branch
///    obligation, of the RIGHT member: a BGP obligation => a [`ProofInputs::Scan`],
///    a bounded-path obligation => a [`ProofInputs::PathReach`] of a member whose
///    disclosed `depth_bound` equals its compiled `d` (soundness req 1) and whose
///    `allow_zero` matches the closure — a path claimed WITHOUT a bound path
///    sub-proof, a `k`/depth mismatch, or a closure mismatch all REJECT;
/// 4. each disclosed VALUES row-index is in range of the re-derived block.
///
/// # Honest scope (load-bearing)
/// This is the STRUCTURAL ROUTING gate. It binds each construct to a bound
/// sub-proof of the correct member and surfaces the depth bound; it is the
/// composition analogue of `verify::branch_obligations`. It does NOT itself run
/// the bb proof verification (that is the [`verify_manifest`] loop, which
/// `reconstruct_public_inputs` serializes `PathReach` inputs for) NOR the disclosed
/// TERM binding of a path's `pred_enc`/`src_enc`/`dst_enc` or a VALUES row's cells
/// — that disclosed-solution term binding is [`bind_fragment_solution`] (sq-1zf94),
/// and both run together in [`verify_fragment_manifest`] for an end-to-end
/// path/`UNION`/`VALUES` accept. The verifier stack is internally re-audited but
/// NOT externally audited (sq-qhy4 pending); NO soundness / privacy property is
/// asserted as achieved.
// [OPUS-4.8] sq-3kd2g.6: fail-closed fragment dispatch routing gate.
#[cfg(feature = "extended-fragment")]
pub fn dispatch_fragment(
    fm: &crate::manifest::FragmentManifest,
) -> Result<(), FragmentDispatchError> {
    let manifest = &fm.manifest;
    // (1) Re-derive the fragment from the query text alone — fail-closed on
    // anything outside the wave-1 extended fragment.
    let fq = fragment_query(&manifest.query)
        .map_err(|e| FragmentDispatchError::OutsideFragment(e.to_string()))?;

    // (2) Every embedded sub-proof MUST carry a known, compiled circuit id whose
    // DECLARED value equals the id re-derived from its public inputs — no unknown /
    // uncompiled member, and no claim/member mismatch (e.g. a wrong `k` bucket),
    // reaches the routing (fail-closed hygiene, independent of branch attribution).
    for (i, sp) in manifest.sub_proofs.iter().enumerate() {
        match derive_id(&sp.inputs) {
            None => return Err(FragmentDispatchError::UnknownCircuit { proof: i }),
            Some(derived) if &derived != sp.inputs.circuit_id() => {
                return Err(FragmentDispatchError::CircuitIdMismatch {
                    proof: i,
                    declared: sp.inputs.circuit_id().clone(),
                    derived,
                })
            }
            Some(_) => {}
        }
    }

    // A manifest with no branch attribution carries no extended-fragment claim to
    // route (a stage-1 presentation): the query-hygiene + id checks above are all
    // that apply. This keeps the gate a strict no-op on stage-1 manifests.
    if fm.branch_witnesses.is_empty() {
        return Ok(());
    }

    for (wi, bw) in fm.branch_witnesses.iter().enumerate() {
        let branch =
            fq.branches
                .get(bw.branch)
                .ok_or(FragmentDispatchError::BranchOutOfRange {
                    witness: wi,
                    branch: bw.branch,
                    branches: fq.branches.len(),
                })?;

        // (3) Per-obligation arity: EXACTLY one bound sub-proof per BGP-scan and
        // per bounded-path obligation, and one chosen row per VALUES block.
        if bw.scan_proofs.len() != branch.patterns.len() {
            return Err(FragmentDispatchError::ObligationArityMismatch {
                witness: wi,
                what: "scan",
                expected: branch.patterns.len(),
                got: bw.scan_proofs.len(),
            });
        }
        if bw.path_proofs.len() != branch.path_reach.len() {
            return Err(FragmentDispatchError::ObligationArityMismatch {
                witness: wi,
                what: "path",
                expected: branch.path_reach.len(),
                got: bw.path_proofs.len(),
            });
        }
        if bw.values_rows.len() != branch.values.len() {
            return Err(FragmentDispatchError::ObligationArityMismatch {
                witness: wi,
                what: "values",
                expected: branch.values.len(),
                got: bw.values_rows.len(),
            });
        }

        // Each BGP obligation must name a bound scan sub-proof.
        for (oi, &pi) in bw.scan_proofs.iter().enumerate() {
            let sp = manifest
                .sub_proofs
                .get(pi)
                .ok_or(FragmentDispatchError::DanglingProof { witness: wi, proof: pi })?;
            if !matches!(sp.inputs, ProofInputs::Scan { .. }) {
                return Err(FragmentDispatchError::NotAScanProof {
                    witness: wi,
                    obligation: oi,
                    proof: pi,
                });
            }
        }

        // Each bounded-path obligation must name a bound PathReach sub-proof of the
        // member the closure requires (depth surfaced + matching, allow_zero
        // consistent, fixed-depth closures pinned).
        for (oi, &pi) in bw.path_proofs.iter().enumerate() {
            let obligation = &branch.path_reach[oi];
            let sp = manifest
                .sub_proofs
                .get(pi)
                .ok_or(FragmentDispatchError::DanglingProof { witness: wi, proof: pi })?;
            let ProofInputs::PathReach { allow_zero, id, .. } = &sp.inputs else {
                return Err(FragmentDispatchError::PathReachMissing {
                    witness: wi,
                    obligation: oi,
                    proof: pi,
                });
            };
            let CircuitId::PathReach { d, .. } = id else {
                return Err(FragmentDispatchError::PathReachMissing {
                    witness: wi,
                    obligation: oi,
                    proof: pi,
                });
            };
            // The sub-proof's disclosed `depth_bound` is already pinned to this
            // member's `d` by the id-hygiene check above (`derive_id` => None when
            // `depth_bound != d`) — the depth-surfacing / anti-overflow gate
            // (soundness req 1). `d` is re-used here only for the fixed-closure check.
            // Closure <-> allow_zero: the zero-length case is admitted iff the
            // closure's minimum length is 0 (`*`/`?`), never for `+`.
            let expect_allow_zero = obligation.closure.min_len() == 0;
            if *allow_zero != expect_allow_zero {
                return Err(FragmentDispatchError::PathClosureMismatch { witness: wi, obligation: oi });
            }
            // A fixed-depth closure (`p?`, bound pinned to 1) must not bind a
            // deeper member — that would let a 2-step chain masquerade as `p?`.
            if let Some(fixed) = obligation.closure.fixed_k() {
                if *d as usize != fixed {
                    return Err(FragmentDispatchError::PathDepthExceedsClosure {
                        witness: wi,
                        obligation: oi,
                        member_d: *d,
                        fixed,
                    });
                }
            }
        }

        // (4) Each disclosed VALUES row-index must be in range of the re-derived
        // block (the rows are public constants of the query text).
        for (bi, &row) in bw.values_rows.iter().enumerate() {
            let rows = branch.values[bi].rows.len();
            if row >= rows {
                return Err(FragmentDispatchError::ValuesRowOutOfRange {
                    witness: wi,
                    block: bi,
                    row,
                    rows,
                });
            }
        }
    }

    Ok(())
}

/// Which endpoint of a `PathReach` obligation a disclosed-solution binding refers
/// to (`src_enc` vs `dst_enc`). Used by [`FragmentSolutionError`] (sq-1zf94).
// [OPUS-4.8] sq-1zf94. Opt-in (`extended-fragment`).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathEndpoint {
    /// The chain SOURCE (`src_enc`, the path subject).
    Src,
    /// The chain DESTINATION (`dst_enc`, the path object).
    Dst,
}

#[cfg(feature = "extended-fragment")]
impl std::fmt::Display for PathEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathEndpoint::Src => write!(f, "src"),
            PathEndpoint::Dst => write!(f, "dst"),
        }
    }
}

/// Why the FAIL-CLOSED extended-fragment DISCLOSED-SOLUTION term binding
/// ([`bind_fragment_solution`], sq-1zf94) REJECTED a presentation. Every variant
/// is a fail-closed refusal: the verifier RE-ENCODES the query predicate / query
/// constant / disclosed solution term ITSELF and demands byte-equality with the
/// proof-bound `PathReach` `pred_enc`/`src_enc`/`dst_enc` and the query's inline
/// `VALUES` cells — a mismatch never silently accepts.
// [OPUS-4.8] sq-1zf94. Opt-in (`extended-fragment`), NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentSolutionError {
    /// A disclosed [`crate::manifest::SolutionBinding`] term is malformed (an
    /// unparseable IRI / datatype / language tag, a language + non-`rdf:langString`
    /// datatype, or a term the encoder rejects) — the verifier cannot re-derive its
    /// encoding, so it refuses fail-closed.
    MalformedSolutionTerm { witness: usize, var: String },
    /// A `PathReach` sub-proof's `pred_enc` does not equal the encoding of the
    /// query-text path predicate: a chain proved over the WRONG predicate.
    PathPredMismatch { witness: usize, obligation: usize },
    /// A `PathReach` sub-proof's endpoint encoding (`src_enc` / `dst_enc`) does not
    /// equal the encoding the verifier re-derives from the query-CONSTANT endpoint
    /// or the disclosed solution's binding for the endpoint VARIABLE — the proof's
    /// disclosed endpoint is not the term the presented solution claims.
    PathEndpointMismatch { witness: usize, obligation: usize, endpoint: PathEndpoint },
    /// A PROJECTED path endpoint VARIABLE is absent from the disclosed solution.
    /// The relying party MUST disclose every projected endpoint so it can be bound;
    /// omitting it to dodge the binding is fail-closed.
    UnboundProjectedEndpoint {
        witness: usize,
        obligation: usize,
        endpoint: PathEndpoint,
        var: String,
    },
    /// A `VALUES` cell for a PROJECTED variable does not match the disclosed
    /// solution's binding for that variable — a wrong disclosed row, or a solution
    /// term inconsistent with the query's inline `VALUES` data.
    ValuesCellMismatch { witness: usize, block: usize, column: usize, var: String },
    /// A `VALUES` cell for a PROJECTED variable has NO disclosed-solution binding
    /// (fail-closed — a projected VALUES-constrained variable must be disclosed).
    UnboundProjectedValuesVar { witness: usize, block: usize, column: usize, var: String },
    /// A path endpoint slot was an unnamed wildcard (the property-path fragment
    /// never produces one; refused for totality).
    WildcardEndpoint { witness: usize, obligation: usize, endpoint: PathEndpoint },
    /// A structural inconsistency the routing gate normally rules out first (a
    /// branch / obligation / row index out of range, or a path obligation naming a
    /// non-`PathReach` sub-proof). Kept so this gate is SELF-CONTAINED and never
    /// panics on an index — a fail-closed refusal even if reached directly.
    Structure { witness: usize, what: &'static str },
}

#[cfg(feature = "extended-fragment")]
impl std::fmt::Display for FragmentSolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FragmentSolutionError::MalformedSolutionTerm { witness, var } => write!(
                f,
                "branch witness {} discloses a malformed solution term for ?{} (fail-closed)",
                witness, var
            ),
            FragmentSolutionError::PathPredMismatch { witness, obligation } => write!(
                f,
                "branch witness {} path obligation {}: pred_enc does not match the query-text path predicate (path proved over the wrong predicate)",
                witness, obligation
            ),
            FragmentSolutionError::PathEndpointMismatch { witness, obligation, endpoint } => write!(
                f,
                "branch witness {} path obligation {}: {}_enc does not match the disclosed solution / query-constant endpoint",
                witness, obligation, endpoint
            ),
            FragmentSolutionError::UnboundProjectedEndpoint { witness, obligation, endpoint, var } => {
                write!(
                    f,
                    "branch witness {} path obligation {}: projected {} endpoint ?{} is not disclosed in the solution (fail-closed)",
                    witness, obligation, endpoint, var
                )
            }
            FragmentSolutionError::ValuesCellMismatch { witness, block, column, var } => write!(
                f,
                "branch witness {} VALUES block {} column {} (?{}): the disclosed solution does not match the chosen row's cell (wrong disclosed row / inconsistent term)",
                witness, block, column, var
            ),
            FragmentSolutionError::UnboundProjectedValuesVar { witness, block, column, var } => {
                write!(
                    f,
                    "branch witness {} VALUES block {} column {} (?{}): projected VALUES variable is not disclosed in the solution (fail-closed)",
                    witness, block, column, var
                )
            }
            FragmentSolutionError::WildcardEndpoint { witness, obligation, endpoint } => write!(
                f,
                "branch witness {} path obligation {}: {} endpoint is an unnamed wildcard (fail-closed)",
                witness, obligation, endpoint
            ),
            FragmentSolutionError::Structure { witness, what } => write!(
                f,
                "branch witness {} disclosed-solution structure error: {} out of range (fail-closed)",
                witness, what
            ),
        }
    }
}

#[cfg(feature = "extended-fragment")]
impl std::error::Error for FragmentSolutionError {}

/// Whether a proof-bound `FieldHex` parses to EXACTLY the expected field element
/// `want`. A malformed hex is treated as a mismatch (fail-closed) — the crypto
/// stage additionally rejects it as [`CheckError::MalformedField`].
// [OPUS-4.8] sq-1zf94.
#[cfg(feature = "extended-fragment")]
fn field_hex_is(h: &FieldHex, want: &Fr) -> bool {
    h.to_field().map(|f| f == *want).unwrap_or(false)
}

/// Bind ONE `PathReach` endpoint (`src_enc` or `dst_enc`) to the query-re-derived
/// endpoint slot: a query CONSTANT binds to its own encoding; a PROJECTED query
/// VARIABLE binds to the disclosed solution's encoding for it; a non-projected
/// (existential) variable stays hidden and is NOT term-bound here (documented
/// residual). Fail-closed on any mismatch / missing projected binding / wildcard.
// [OPUS-4.8] sq-1zf94.
#[cfg(feature = "extended-fragment")]
#[allow(clippy::too_many_arguments)]
fn bind_path_endpoint(
    witness: usize,
    obligation: usize,
    endpoint: PathEndpoint,
    slot: &SlotPattern,
    enc: &FieldHex,
    mu: &std::collections::BTreeMap<String, Fr>,
    projected: &BTreeSet<String>,
    salt: &Fr,
) -> Result<(), FragmentSolutionError> {
    match slot {
        SlotPattern::Term(t) => {
            let want = encode_term(t, salt).ok_or(FragmentSolutionError::PathEndpointMismatch {
                witness,
                obligation,
                endpoint,
            })?;
            if field_hex_is(enc, &want) {
                Ok(())
            } else {
                Err(FragmentSolutionError::PathEndpointMismatch { witness, obligation, endpoint })
            }
        }
        SlotPattern::Var(v) => {
            // A non-projected endpoint is existential (hidden by design) — the
            // disclosed solution says nothing about it, so it is not term-bound.
            if !projected.contains(v) {
                return Ok(());
            }
            match mu.get(v) {
                None => Err(FragmentSolutionError::UnboundProjectedEndpoint {
                    witness,
                    obligation,
                    endpoint,
                    var: v.clone(),
                }),
                Some(want) if field_hex_is(enc, want) => Ok(()),
                Some(_) => {
                    Err(FragmentSolutionError::PathEndpointMismatch { witness, obligation, endpoint })
                }
            }
        }
        SlotPattern::Wildcard => {
            Err(FragmentSolutionError::WildcardEndpoint { witness, obligation, endpoint })
        }
    }
}

/// FAIL-CLOSED wave-1 extended-fragment DISCLOSED-SOLUTION term binding
/// (sq-1zf94): the composition analogue of the flat query-text term binding
/// (`bind_query_correctness`'s scan-const check + the `bind_joins` slot binding),
/// for the extended constructs `dispatch_fragment` routes but does NOT term-bind.
///
/// For each disclosed solution (one [`crate::manifest::BranchWitness`], attributed
/// to a branch the routing gate already validated), the verifier RE-DERIVES the
/// branch structure from the query TEXT alone ([`fragment_query`], never the
/// manifest), re-encodes each disclosed [`crate::manifest::SolutionBinding`] term
/// ITSELF, and demands, fail-closed:
///
/// 1. **Path predicate.** Each bound `PathReach`'s `pred_enc` equals the encoding
///    of the query-text path predicate — a chain proved over a DIFFERENT predicate
///    than the query names is refused.
/// 2. **Path endpoints.** Each `src_enc` / `dst_enc` equals the encoding the
///    verifier re-derives from the query-CONSTANT endpoint, or (for a PROJECTED
///    endpoint variable) from the disclosed solution's binding for that variable —
///    a proof whose disclosed endpoint is not the claimed term is refused, and a
///    projected endpoint omitted from the solution is refused.
/// 3. **VALUES cells.** For each disclosed `VALUES` row, every PROJECTED
///    variable's disclosed solution binding equals the encoding of that row's cell
///    — a "wrong disclosed row" (`values_rows` pointing at a row whose cell does
///    not match the disclosed term) is refused.
///
/// Because `PathReach.pred_enc`/`src_enc`/`dst_enc` are byte-bound into the bb
/// public inputs by `reconstruct_public_inputs` (audit #1), a solution that
/// passes THIS structural gate AND the crypto stage has its disclosed path
/// endpoints / VALUES-constrained variables both equal to the query + disclosed
/// terms and bound into a valid sub-proof, so the disclosed terms are genuinely
/// tied to the proofs.
///
/// # Honest scope — the RESIDUAL still deferred (load-bearing)
/// This binds the PATH and `VALUES` disclosed terms. The BGP-SCAN-slot binding of a
/// disclosed solution's variables to a scan sub-proof's disclosed rows (a scan
/// discloses `r` rows; the per-solution row-selection model) is done by the
/// SIBLING gate [`bind_fragment_scans`] (sq-qyfth), which
/// [`verify_fragment_manifest`] runs immediately after this one. Still NOT bound by
/// either gate: (a) the flat cross-graph Q6 non-bnode obligation per branch (and
/// the existential scan↔path join coherence — sq-ygk6x), and (b) an EXISTENTIAL
/// (non-projected) path endpoint's value (hidden by design — disclosed only as an
/// opaque encoding). The whole verifier stack is internally re-audited but NOT
/// externally audited (sq-qhy4). NO soundness / privacy property is asserted as
/// achieved.
// [OPUS-4.8] sq-1zf94: disclosed-solution term binding. Opt-in
// (`extended-fragment`), research-grade, NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
pub fn bind_fragment_solution(
    fm: &crate::manifest::FragmentManifest,
) -> Result<(), FragmentSolutionError> {
    let manifest = &fm.manifest;
    // Re-derive the fragment from the query TEXT alone (never the manifest). An
    // outside-fragment / unparseable query is normally caught first by
    // `dispatch_fragment`; refuse fail-closed if we somehow reach here.
    let fq = fragment_query(&manifest.query)
        .map_err(|_| FragmentSolutionError::Structure { witness: 0, what: "query" })?;
    let projected: BTreeSet<String> = fq.projected.iter().cloned().collect();
    // IRIs / literals (the only disclosable term kinds, and every query constant /
    // VALUES cell) are salt-INDEPENDENT, so salt 0 matches the prover's encoding
    // (`build_path_reach` uses the graph salt, identical for non-bnode terms).
    let salt = Fr::from(0u64);

    for (wi, bw) in fm.branch_witnesses.iter().enumerate() {
        let Some(branch) = fq.branches.get(bw.branch) else {
            return Err(FragmentSolutionError::Structure { witness: wi, what: "branch" });
        };

        // Re-encode the disclosed solution: var -> Fr, VERIFIER-recomputed (never a
        // prover-supplied encoding).
        let mut mu: std::collections::BTreeMap<String, Fr> = std::collections::BTreeMap::new();
        for sb in &bw.solution {
            let term = sb.term.to_term().ok_or_else(|| {
                FragmentSolutionError::MalformedSolutionTerm { witness: wi, var: sb.var.clone() }
            })?;
            let enc = encode_term(&term, &salt).ok_or_else(|| {
                FragmentSolutionError::MalformedSolutionTerm { witness: wi, var: sb.var.clone() }
            })?;
            mu.insert(sb.var.clone(), enc);
        }

        // (1)+(2) Path predicate + endpoint binding.
        for (oi, &pi) in bw.path_proofs.iter().enumerate() {
            let Some(obl) = branch.path_reach.get(oi) else {
                return Err(FragmentSolutionError::Structure { witness: wi, what: "path obligation" });
            };
            let sp = manifest.sub_proofs.get(pi).ok_or(FragmentSolutionError::Structure {
                witness: wi,
                what: "path proof index",
            })?;
            let ProofInputs::PathReach { pred_enc, src_enc, dst_enc, .. } = &sp.inputs else {
                return Err(FragmentSolutionError::Structure { witness: wi, what: "path proof kind" });
            };
            let want_pred = encode_term(&oxrdf::Term::NamedNode(obl.predicate.clone()), &salt)
                .ok_or(FragmentSolutionError::PathPredMismatch { witness: wi, obligation: oi })?;
            if !field_hex_is(pred_enc, &want_pred) {
                return Err(FragmentSolutionError::PathPredMismatch { witness: wi, obligation: oi });
            }
            bind_path_endpoint(wi, oi, PathEndpoint::Src, &obl.subject, src_enc, &mu, &projected, &salt)?;
            bind_path_endpoint(wi, oi, PathEndpoint::Dst, &obl.object, dst_enc, &mu, &projected, &salt)?;
        }

        // (3) VALUES cell binding — the disclosed solution must agree with the
        // CHOSEN row's inline query constants for every PROJECTED variable.
        for (bi, &row) in bw.values_rows.iter().enumerate() {
            let Some(block) = branch.values.get(bi) else {
                return Err(FragmentSolutionError::Structure { witness: wi, what: "values block" });
            };
            let Some(cells) = block.rows.get(row) else {
                return Err(FragmentSolutionError::Structure { witness: wi, what: "values row" });
            };
            for (col, var) in block.variables.iter().enumerate() {
                // A non-projected VALUES variable is existential here (not term-bound).
                if !projected.contains(var) {
                    continue;
                }
                // `None` (UNDEF) cell => no constraint on this variable.
                let Some(Some(term)) = cells.get(col) else {
                    continue;
                };
                let want = encode_term(term, &salt).ok_or_else(|| {
                    FragmentSolutionError::ValuesCellMismatch {
                        witness: wi,
                        block: bi,
                        column: col,
                        var: var.clone(),
                    }
                })?;
                match mu.get(var) {
                    None => {
                        return Err(FragmentSolutionError::UnboundProjectedValuesVar {
                            witness: wi,
                            block: bi,
                            column: col,
                            var: var.clone(),
                        })
                    }
                    Some(sol) if *sol == want => {}
                    Some(_) => {
                        return Err(FragmentSolutionError::ValuesCellMismatch {
                            witness: wi,
                            block: bi,
                            column: col,
                            var: var.clone(),
                        })
                    }
                }
            }
        }
    }

    Ok(())
}

/// Why the FAIL-CLOSED extended-fragment BGP SCAN-SLOT binding
/// ([`bind_fragment_scans`], sq-qyfth) REJECTED a presentation. Every variant is a
/// fail-closed refusal: the verifier RE-DERIVES the query BGP pattern + the
/// disclosed solution term ITSELF and demands byte-equality with the proof-bound
/// scan `pattern_const_enc` / disclosed-row slot values (never a prover encoding).
// [OPUS-4.8] sq-qyfth. Opt-in (`extended-fragment`), NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentScanError {
    /// A disclosed [`crate::manifest::SolutionBinding`] term is malformed (the
    /// verifier cannot re-derive its encoding) — refused fail-closed.
    MalformedSolutionTerm { witness: usize, var: String },
    /// A BGP obligation names a sub-proof that is not a [`ProofInputs::Scan`] (the
    /// routing gate normally rules this out; kept so this gate is self-contained).
    NotAScanProof { witness: usize, obligation: usize, proof: usize },
    /// A structural inconsistency the routing gate normally rules out first (a
    /// branch / obligation / proof index out of range). Kept so this gate never
    /// panics on an index — a fail-closed refusal even if reached directly.
    Structure { witness: usize, what: &'static str },
    /// The scan sub-proof's bound `pattern_is_const`/`pattern_const_enc` does not
    /// answer the query BGP pattern the obligation stands for (a scan over the
    /// WRONG predicate/constant, or a constant/variable slot-shape mismatch) — the
    /// composition analogue of the flat `scan_matches_pattern` gate.
    ScanPatternMismatch { witness: usize, obligation: usize },
    /// The scan pattern carries at least one VARIABLE slot, but the manifest
    /// selected NO supporting disclosed row for this scan (`scan_rows` has no entry
    /// for the obligation) — a solution variable in the scan pattern cannot be
    /// bound without a row, so it is refused fail-closed.
    MissingRowSelection { witness: usize, obligation: usize },
    /// The selected supporting row index is outside the scan's ACTIVE disclosed
    /// rows (`min(row_count, rows.len())`).
    RowOutOfRange { witness: usize, obligation: usize, row: usize, active: usize },
    /// A PROJECTED (disclosed) variable occupies a scan slot but is absent from the
    /// disclosed solution — omitting it to dodge the binding is refused fail-closed.
    UnboundProjectedScanVar { witness: usize, obligation: usize, slot: usize, var: String },
    /// The selected row's slot value does not equal the encoding the verifier
    /// re-derives from the disclosed solution's binding for that PROJECTED variable
    /// (the row does not support the claimed solution) — refused fail-closed.
    ScanSlotMismatch { witness: usize, obligation: usize, slot: usize, var: String },
    /// Two atoms in the SAME branch that share an EXISTENTIAL (non-projected)
    /// variable selected rows whose slot values for it disagree (join incoherence —
    /// there is no single witness for the shared variable), mirroring the flat
    /// `bind_joins` / disclosed-row join gate. Refused fail-closed.
    JoinIncoherent { witness: usize, obligation: usize, slot: usize, var: String },
    /// A scan pattern slot is an unnamed wildcard (the BGP fragment never produces
    /// one; refused for totality).
    WildcardSlot { witness: usize, obligation: usize, slot: usize },
}

#[cfg(feature = "extended-fragment")]
impl std::fmt::Display for FragmentScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FragmentScanError::MalformedSolutionTerm { witness, var } => write!(
                f,
                "branch witness {} discloses a malformed solution term for ?{} (fail-closed)",
                witness, var
            ),
            FragmentScanError::NotAScanProof { witness, obligation, proof } => write!(
                f,
                "branch witness {} BGP obligation {}: sub-proof {} is not a scan (fail-closed)",
                witness, obligation, proof
            ),
            FragmentScanError::Structure { witness, what } => write!(
                f,
                "branch witness {} scan-slot structure error: {} out of range (fail-closed)",
                witness, what
            ),
            FragmentScanError::ScanPatternMismatch { witness, obligation } => write!(
                f,
                "branch witness {} BGP obligation {}: the scan does not answer the query pattern (wrong predicate/constant or slot-shape mismatch)",
                witness, obligation
            ),
            FragmentScanError::MissingRowSelection { witness, obligation } => write!(
                f,
                "branch witness {} BGP obligation {}: the scan pattern has a variable but no supporting row was selected (fail-closed)",
                witness, obligation
            ),
            FragmentScanError::RowOutOfRange { witness, obligation, row, active } => write!(
                f,
                "branch witness {} BGP obligation {}: selected row {} is outside the {} active disclosed rows (fail-closed)",
                witness, obligation, row, active
            ),
            FragmentScanError::UnboundProjectedScanVar { witness, obligation, slot, var } => write!(
                f,
                "branch witness {} BGP obligation {} slot {}: projected scan variable ?{} is not disclosed in the solution (fail-closed)",
                witness, obligation, slot, var
            ),
            FragmentScanError::ScanSlotMismatch { witness, obligation, slot, var } => write!(
                f,
                "branch witness {} BGP obligation {} slot {} (?{}): the selected row's slot does not match the disclosed solution (wrong supporting row / inconsistent term)",
                witness, obligation, slot, var
            ),
            FragmentScanError::JoinIncoherent { witness, obligation, slot, var } => write!(
                f,
                "branch witness {} BGP obligation {} slot {} (?{}): rows selected for atoms sharing this existential variable disagree (join incoherent)",
                witness, obligation, slot, var
            ),
            FragmentScanError::WildcardSlot { witness, obligation, slot } => write!(
                f,
                "branch witness {} BGP obligation {} slot {}: unnamed wildcard slot (fail-closed)",
                witness, obligation, slot
            ),
        }
    }
}

#[cfg(feature = "extended-fragment")]
impl std::error::Error for FragmentScanError {}

/// The query BGP pattern's constant slots as `Option<oxrdf::Term>` (the input
/// `scan_matches_pattern` expects): a `Term` slot is a constant, a `Var` /
/// `Wildcard` slot is `None`.
// [OPUS-4.8] sq-qyfth.
#[cfg(feature = "extended-fragment")]
fn pattern_slot_consts(slots: &[SlotPattern; 3]) -> [Option<oxrdf::Term>; 3] {
    let c = |s: &SlotPattern| match s {
        SlotPattern::Term(t) => Some(t.clone()),
        _ => None,
    };
    [c(&slots[0]), c(&slots[1]), c(&slots[2])]
}

/// FAIL-CLOSED wave-1 extended-fragment BGP SCAN-SLOT binding (sq-qyfth): the
/// composition analogue of the flat `bind_query_correctness` scan-const check +
/// the disclosed-row / `bind_joins` slot binding, for the BGP scans a branch
/// carries. This is the LARGEST unbound surface [`bind_fragment_solution`] (#1673)
/// left: a disclosed solution variable that occurs ONLY in a BGP scan (not as a
/// `PathReach` endpoint or `VALUES` cell) was routed but not term-bound to the
/// scan's disclosed rows.
///
/// For each disclosed solution (one [`crate::manifest::BranchWitness`], attributed
/// to a branch the routing gate already validated), the verifier RE-DERIVES the
/// branch's BGP patterns from the query TEXT alone ([`fragment_query`], never the
/// manifest), re-encodes each disclosed [`crate::manifest::SolutionBinding`] term
/// ITSELF, and for every BGP obligation demands, fail-closed:
///
/// 1. **Pattern binding.** The scan sub-proof's bound `pattern_is_const` /
///    `pattern_const_enc` answer the query BGP pattern (`scan_matches_pattern`) —
///    a scan over the wrong predicate/constant is refused. Because those inputs are
///    byte-bound into the bb public inputs and the scan circuit binds every
///    disclosed row's constant slots to `pattern_const_enc`, the selected row's
///    constant slots are transitively equal to the query constants.
/// 2. **Row selection.** A scan pattern that carries ANY variable needs a selected
///    supporting row (`BranchWitness::scan_rows`); a missing selection, or an index
///    outside the scan's ACTIVE disclosed rows, is refused.
/// 3. **Projected-variable binding.** For each VARIABLE slot bound to a PROJECTED
///    (disclosed) query variable, the selected row's slot value must equal the
///    encoding the verifier re-derives from the disclosed solution's binding for
///    that variable — a row that does not support the claimed solution, or a
///    projected scan variable omitted from the solution, is refused.
/// 4. **Join coherence.** For an EXISTENTIAL (non-projected) variable shared by two
///    atoms in the branch, the rows selected for those atoms must agree on its slot
///    value (mirroring the flat `bind_joins` / disclosed-row join gate) — an
///    incoherent row selection is refused.
///
/// Because the scan `rows` and `pattern_const_enc` are byte-bound into the bb
/// public inputs by `reconstruct_public_inputs` (audit #1), a solution that passes
/// THIS structural gate AND the crypto stage has its disclosed scan-bound variables
/// genuinely tied to the proofs.
///
/// # Honest scope — where the remaining obligations live (load-bearing)
/// This gate binds the BGP-scan-slot residual [`bind_fragment_solution`] named and
/// checks scan↔scan join coherence. The COMPLEMENTARY obligations — the flat
/// cross-graph Q6 non-bnode obligation PER BRANCH, and the existential coherence of
/// a variable shared between a scan slot and a `PathReach` endpoint (`src_enc` /
/// `dst_enc`) — are enforced by [`bind_fragment_join_coherence`] (sq-ygk6x), run as
/// the next layer of [`verify_fragment_manifest`]. What still remains after BOTH
/// gates (documented on [`bind_fragment_join_coherence`]): since sq-nlulr the
/// salt-uniqueness gate covers PATH-referenced committed graphs too, so a cross-graph
/// join through a single-graph PATH graph carries the same distinct-salt non-bnode
/// discipline as a scan↔scan join (a multi-graph path is still refused fail-closed);
/// the only remaining BY-DESIGN item is that an EXISTENTIAL (non-projected) path
/// endpoint's value stays hidden. The whole verifier stack is internally re-audited
/// but NOT externally audited (sq-qhy4). NO soundness / privacy property is asserted
/// as achieved.
// [OPUS-4.8] sq-qyfth: BGP scan-slot binding. Opt-in (`extended-fragment`),
// research-grade, NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
pub fn bind_fragment_scans(
    fm: &crate::manifest::FragmentManifest,
) -> Result<(), FragmentScanError> {
    let manifest = &fm.manifest;
    // Re-derive the fragment from the query TEXT alone (never the manifest). An
    // outside-fragment / unparseable query is normally caught first by
    // `dispatch_fragment`; refuse fail-closed if we somehow reach here.
    let fq = fragment_query(&manifest.query)
        .map_err(|_| FragmentScanError::Structure { witness: 0, what: "query" })?;
    let projected: BTreeSet<String> = fq.projected.iter().cloned().collect();
    // IRIs / literals (the only disclosable term kinds) are salt-INDEPENDENT, so
    // salt 0 matches the prover's encoding for a projected variable's term. The
    // existential coherence check compares two disclosed row slot values DIRECTLY
    // (never re-derived), so it is salt-agnostic within a graph.
    let salt = Fr::from(0u64);

    for (wi, bw) in fm.branch_witnesses.iter().enumerate() {
        let Some(branch) = fq.branches.get(bw.branch) else {
            return Err(FragmentScanError::Structure { witness: wi, what: "branch" });
        };

        // Re-encode the disclosed solution: var -> Fr, VERIFIER-recomputed.
        let mut mu: std::collections::BTreeMap<String, Fr> = std::collections::BTreeMap::new();
        for sb in &bw.solution {
            let term = sb.term.to_term().ok_or_else(|| {
                FragmentScanError::MalformedSolutionTerm { witness: wi, var: sb.var.clone() }
            })?;
            let enc = encode_term(&term, &salt).ok_or_else(|| {
                FragmentScanError::MalformedSolutionTerm { witness: wi, var: sb.var.clone() }
            })?;
            mu.insert(sb.var.clone(), enc);
        }

        // Per-branch coherence map for EXISTENTIAL (non-projected) variables shared
        // across the branch's scans (the disclosed-row join analogue of bind_joins).
        let mut existential: std::collections::BTreeMap<String, Fr> =
            std::collections::BTreeMap::new();

        for (oi, &pi) in bw.scan_proofs.iter().enumerate() {
            let Some(pattern) = branch.patterns.get(oi) else {
                return Err(FragmentScanError::Structure { witness: wi, what: "scan obligation" });
            };
            let sp = manifest.sub_proofs.get(pi).ok_or(FragmentScanError::Structure {
                witness: wi,
                what: "scan proof index",
            })?;
            let ProofInputs::Scan { rows, row_count, .. } = &sp.inputs else {
                return Err(FragmentScanError::NotAScanProof { witness: wi, obligation: oi, proof: pi });
            };

            // A wildcard slot never comes from a parsed BGP; refuse for totality.
            if let Some(slot) =
                pattern.slots.iter().position(|s| matches!(s, SlotPattern::Wildcard))
            {
                return Err(FragmentScanError::WildcardSlot { witness: wi, obligation: oi, slot });
            }

            // (1) The scan must answer this query BGP pattern (constant/shape
            // binding — the flat `scan_matches_pattern` gate). This binds the
            // pattern constants; the scan circuit binds each disclosed row's
            // constant slots to those, so we only bind the VARIABLE slots below.
            let consts = pattern_slot_consts(&pattern.slots);
            if !scan_matches_pattern(&sp.inputs, &consts) {
                return Err(FragmentScanError::ScanPatternMismatch { witness: wi, obligation: oi });
            }

            // A pattern with no variable slot is fully bound by (1); no row needed.
            let has_var = pattern.slots.iter().any(|s| matches!(s, SlotPattern::Var(_)));
            if !has_var {
                continue;
            }

            // (2) A variable pattern needs a selected supporting disclosed row that
            // lies within the scan's ACTIVE rows.
            let Some(&row) = bw.scan_rows.get(oi) else {
                return Err(FragmentScanError::MissingRowSelection { witness: wi, obligation: oi });
            };
            let active = (*row_count as usize).min(rows.len());
            if row >= active {
                return Err(FragmentScanError::RowOutOfRange {
                    witness: wi,
                    obligation: oi,
                    row,
                    active,
                });
            }
            let selected = &rows[row];

            // (3)+(4) Bind each variable slot to the disclosed solution (projected)
            // or the per-branch existential coherence map.
            for (slot, sp_slot) in pattern.slots.iter().enumerate() {
                let SlotPattern::Var(v) = sp_slot else {
                    // Term slots are bound by (1); wildcards were refused above.
                    continue;
                };
                // A malformed row hex is a mismatch fail-closed (the crypto stage
                // also rejects it as MalformedField).
                let Some(row_enc) = selected[slot].to_field() else {
                    return Err(FragmentScanError::ScanSlotMismatch {
                        witness: wi,
                        obligation: oi,
                        slot,
                        var: v.clone(),
                    });
                };
                if projected.contains(v) {
                    match mu.get(v) {
                        None => {
                            return Err(FragmentScanError::UnboundProjectedScanVar {
                                witness: wi,
                                obligation: oi,
                                slot,
                                var: v.clone(),
                            })
                        }
                        Some(want) if *want == row_enc => {}
                        Some(_) => {
                            return Err(FragmentScanError::ScanSlotMismatch {
                                witness: wi,
                                obligation: oi,
                                slot,
                                var: v.clone(),
                            })
                        }
                    }
                } else {
                    match existential.get(v) {
                        None => {
                            existential.insert(v.clone(), row_enc);
                        }
                        Some(prev) if *prev == row_enc => {}
                        Some(_) => {
                            return Err(FragmentScanError::JoinIncoherent {
                                witness: wi,
                                obligation: oi,
                                slot,
                                var: v.clone(),
                            })
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Why the FAIL-CLOSED extended-fragment PER-BRANCH JOIN COHERENCE + cross-graph
/// Q6 non-bnode gate ([`bind_fragment_join_coherence`], sq-ygk6x) REJECTED a
/// presentation. Every variant is a fail-closed refusal: the verifier RE-DERIVES
/// the branch's join structure + Q6 obligations from the query TEXT + the
/// proof-bound disclosed data (rows / `src_enc` / `dst_enc` / attributions), never
/// a prover-supplied claim.
// [OPUS-4.8] sq-ygk6x. Opt-in (`extended-fragment`), NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentJoinError {
    /// A structural inconsistency the routing / scan-slot gates normally rule out
    /// first (a branch / obligation / proof / row index out of range, a wrong
    /// sub-proof kind, a malformed proof-bound attribution, or a variable scan with
    /// no selected supporting row). Kept so this gate never panics on an index — a
    /// fail-closed refusal even if reached directly.
    Structure { witness: usize, what: &'static str },
    /// A proof-bound endpoint encoding / disclosed row slot (`src_enc` / `dst_enc` /
    /// a scan row slot) is not a valid field element — refused fail-closed (the
    /// crypto stage additionally rejects it as [`CheckError::MalformedField`]).
    MalformedField { witness: usize, obligation: usize, what: &'static str },
    /// Two atoms in the SAME branch that share a variable resolve it to DISAGREEING
    /// disclosed values — the join has no single witness. `obligation` is the atom
    /// (combined index: `0..patterns.len()` are BGP scans, `patterns.len()..` are
    /// path obligations) whose value disagreed with the one first seen for `var`.
    /// This is the sq-ygk6x join coherence spanning scan↔scan (mirroring
    /// [`bind_fragment_scans`]) AND the NEW scan↔path / path↔path edges: a path
    /// claiming a different node than its supporting scan row (or another path
    /// endpoint) supports is refused, both directions.
    Incoherent { witness: usize, obligation: usize, var: String },
    /// A [`ProofInputs::PathReach`] sub-proof whose proof-bound attribution admits
    /// MORE THAN ONE committed graph (`obligation` is its combined index). The path's
    /// interior chain nodes then cross graph boundaries; the flat cross-graph
    /// non-bnode obligation would demand every chain-equated interior node
    /// IRI/literal-typed, but those nodes are hidden IN-CIRCUIT — the verifier cannot
    /// discharge that obligation from disclosed data — so a multi-graph path is
    /// refused fail-closed. A single-graph path (interior within one graph) is
    /// accepted.
    MultiGraphPath { witness: usize, obligation: usize },
    /// A required cross-graph join obligation (re-derived per branch by
    /// [`branch_obligations`]) whose shared variable is not covered by a disclosed
    /// value in one of its two atoms — the disclosed data cannot discharge the
    /// obligation, so it is refused fail-closed (the branch-local analogue of the
    /// flat `recheck` MissingObligation rejection).
    UncoveredCrossGraphJoin { witness: usize, variable: String },
}

#[cfg(feature = "extended-fragment")]
impl std::fmt::Display for FragmentJoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FragmentJoinError::Structure { witness, what } => write!(
                f,
                "branch witness {} join-coherence structure error: {} out of range (fail-closed)",
                witness, what
            ),
            FragmentJoinError::MalformedField { witness, obligation, what } => write!(
                f,
                "branch witness {} obligation {}: proof-bound `{}` is not a valid field element (fail-closed)",
                witness, obligation, what
            ),
            FragmentJoinError::Incoherent { witness, obligation, var } => write!(
                f,
                "branch witness {} obligation {} (?{}): atoms sharing this variable resolve it to disagreeing disclosed values (join incoherent — a scan↔path / scan↔scan / path↔path mismatch)",
                witness, obligation, var
            ),
            FragmentJoinError::MultiGraphPath { witness, obligation } => write!(
                f,
                "branch witness {} obligation {}: a path whose proof-bound attribution admits more than one committed graph is refused (its interior-chain non-bnode obligation is not verifier-dischargeable from disclosed data)",
                witness, obligation
            ),
            FragmentJoinError::UncoveredCrossGraphJoin { witness, variable } => write!(
                f,
                "branch witness {}: required cross-graph join on ?{} is not covered by the disclosed data (fail-closed)",
                witness, variable
            ),
        }
    }
}

#[cfg(feature = "extended-fragment")]
impl std::error::Error for FragmentJoinError {}

/// FAIL-CLOSED wave-1 extended-fragment PER-BRANCH JOIN COHERENCE + cross-graph Q6
/// non-bnode gate (sq-ygk6x): the branch-local analogue of the flat cross-graph
/// non-bnode obligation (`sparq_zk::verify::cross_graph_join_obligations` /
/// `recheck`), extended to the path-rewritten obligations of one `UNION` branch.
///
/// [`bind_fragment_scans`] (#1678) bound each disclosed solution variable in a BGP
/// SCAN to its selected row and checked scan↔scan join coherence, but explicitly
/// left TWO residuals to THIS gate: (a) an existential variable shared between a
/// BGP scan atom and a [`ProofInputs::PathReach`] ENDPOINT was not bound (the path
/// `src_enc` / `dst_enc` are public `FieldHex`, directly comparable to a selected
/// scan row slot), and (b) the flat cross-graph Q6 non-bnode obligation was not
/// enforced PER BRANCH.
///
/// For each disclosed solution (one [`crate::manifest::BranchWitness`], attributed
/// to a branch the routing gate already validated), the verifier re-derives the
/// branch from the query TEXT alone ([`fragment_query`], never the manifest) and:
///
/// 1. **Join coherence (item 2).** Builds a per-branch value map from every atom's
///    disclosed data — each BGP-scan variable slot from its SELECTED disclosed row
///    ([`crate::manifest::BranchWitness::scan_rows`]) and each `PathReach` variable
///    ENDPOINT from the proof-bound `src_enc` / `dst_enc` — and requires every
///    occurrence of a shared variable to resolve to the SAME encoding. A path
///    claiming a different node than its supporting scan row (or another path
///    endpoint) supports is refused ([`FragmentJoinError::Incoherent`]), both
///    directions. Because `src_enc` / `dst_enc` and the scan `rows` are byte-bound
///    into the `bb` public inputs (audit #1), a coherent selection genuinely ties
///    the shared existential value across the scan and path sub-proofs.
/// 2. **Cross-graph Q6 non-bnode obligation (item 1).** Re-derives the branch's Q6
///    obligations with [`branch_obligations`] over the PROOF-BOUND per-obligation
///    graph attributions (each scan / path sub-proof's `attribution` bits over its
///    `commitments`, interned to a per-branch committed-graph identity so two atoms
///    over the SAME graph collapse and two over DISTINCT graphs stay distinct — the
///    same safe-coarser discipline as the flat `global_attributions`). Every
///    required cross-graph join edge's shared variable MUST be covered by the
///    disclosed value map ([`FragmentJoinError::UncoveredCrossGraphJoin`] otherwise);
///    the coherence check (1) already forces its two atoms to agree. Combined with
///    the ALWAYS-ACTIVE salt-uniqueness gate (audit #9 — a same-label canonical
///    blank node encodes DIFFERENTLY across two distinctly-salted graphs), an equal
///    encoding across a cross-graph edge cannot be a blank node, so the flat
///    non-bnode obligation holds branch-locally. Since sq-nlulr the salt-uniqueness
///    gate (`bind_issuer_attestations`) records PATH-referenced committed graphs
///    too, so this holds for scan↔scan, scan↔single-graph-path, AND
///    path↔single-graph-path edges alike (a single-graph path graph is now
///    issuer-attested + distinctly salted on the same footing as a scan graph).
/// 3. **Multi-graph paths (fail-closed).** A `PathReach` whose proof-bound
///    attribution admits more than one committed graph
///    ([`branch_obligations`]'s `path_link_non_bnode`) has interior chain nodes that
///    cross graph boundaries. The flat obligation would demand those IRI/literal-
///    typed, but they are hidden in-circuit — the verifier cannot discharge it from
///    disclosed data — so such a path is refused fail-closed
///    ([`FragmentJoinError::MultiGraphPath`]).
///
/// # Honest scope (load-bearing)
/// After this gate, the extended regime enforces the SAME cross-graph non-bnode
/// obligation as the flat path — branch-locally, via the coherence enc-equality +
/// the active salt-uniqueness gate (the identical mechanism) — for BGP scan↔scan,
/// scan↔single-graph-path, AND path↔single-graph-path joins, and additionally binds
/// every existential scan/path endpoint for coherence. sq-nlulr CLOSED the residual
/// that #1684 enumerated here: the salt-uniqueness gate (audit #9,
/// `bind_issuer_attestations`) now records PATH-referenced committed graphs on the
/// SAME footing as SCAN-referenced ones (each path commitment carries the issuer-
/// attestation requirement AND participates in the distinct-salt record), so a
/// cross-graph scan↔single-graph-path join's enc-equality is no longer a bare
/// AGREEMENT check: its non-bnode COROLLARY is discharged by the two graphs being
/// distinctly salted, exactly as for a scan↔scan join. An unattested or
/// salt-colliding path commitment is refused fail-closed
/// ([`CheckError::UnattestedCommitment`] / [`CheckError::SaltReused`]) before any
/// `bb` sub-proof runs. A MULTI-graph path is still refused fail-closed (item 3:
/// its interior-chain non-bnode obligation is not verifier-dischargeable from
/// disclosed data). What remains BY DESIGN — not a non-bnode gap: an EXISTENTIAL
/// (non-projected) path endpoint's VALUE stays hidden (a privacy choice). The whole
/// verifier stack is internally re-audited but NOT externally audited (sq-qhy4). NO
/// soundness / privacy property is asserted as achieved.
// [OPUS-4.8] sq-ygk6x: per-branch join coherence + cross-graph Q6. Opt-in
// (`extended-fragment`), research-grade, NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
pub fn bind_fragment_join_coherence(
    fm: &crate::manifest::FragmentManifest,
) -> Result<(), FragmentJoinError> {
    let manifest = &fm.manifest;
    // Re-derive the fragment from the query TEXT alone (never the manifest).
    let fq = fragment_query(&manifest.query)
        .map_err(|_| FragmentJoinError::Structure { witness: 0, what: "query" })?;

    for (wi, bw) in fm.branch_witnesses.iter().enumerate() {
        let Some(branch) = fq.branches.get(bw.branch) else {
            return Err(FragmentJoinError::Structure { witness: wi, what: "branch" });
        };
        let n_scans = branch.patterns.len();

        // Per-obligation disclosed value maps (combined index space: 0..n_scans are
        // BGP scans, n_scans.. are path obligations — the SAME order `branch_obligations`
        // uses) plus per-obligation proof-bound global attribution sets. A per-branch
        // committed-graph identity interning (distinct graphs -> distinct ids, same
        // graph -> one id) keys the attributions the SAME safe-coarser way as the flat
        // `global_attributions`; it never touches the disclosed values.
        let mut per_obl: Vec<std::collections::BTreeMap<String, Fr>> = Vec::new();
        let mut scan_attrs: Vec<BTreeSet<usize>> = Vec::new();
        let mut path_attrs: Vec<BTreeSet<usize>> = Vec::new();
        let mut intern: std::collections::BTreeMap<[u8; 32], usize> =
            std::collections::BTreeMap::new();
        let mut next_id = 0usize;

        // --- BGP-scan obligations. ---
        for oi in 0..n_scans {
            let pattern = &branch.patterns[oi];
            let &pi = bw.scan_proofs.get(oi).ok_or(FragmentJoinError::Structure {
                witness: wi,
                what: "scan obligation",
            })?;
            let sp = manifest.sub_proofs.get(pi).ok_or(FragmentJoinError::Structure {
                witness: wi,
                what: "scan proof index",
            })?;
            let ProofInputs::Scan { rows, row_count, commitments, attribution, .. } = &sp.inputs
            else {
                return Err(FragmentJoinError::Structure { witness: wi, what: "scan proof kind" });
            };
            scan_attrs.push(intern_attribution(commitments, attribution, &mut intern, &mut next_id)
                .ok_or(FragmentJoinError::Structure { witness: wi, what: "scan attribution" })?);

            // Disclosed value map for THIS scan's variable slots (from the selected
            // supporting row). A constant-only pattern needs no row.
            let mut map = std::collections::BTreeMap::new();
            if pattern.slots.iter().any(|s| matches!(s, SlotPattern::Var(_))) {
                let &row = bw.scan_rows.get(oi).ok_or(FragmentJoinError::Structure {
                    witness: wi,
                    what: "scan row selection",
                })?;
                let active = (*row_count as usize).min(rows.len());
                if row >= active {
                    return Err(FragmentJoinError::Structure { witness: wi, what: "scan row range" });
                }
                for (slot, s) in pattern.slots.iter().enumerate() {
                    if let SlotPattern::Var(v) = s {
                        let val = rows[row][slot].to_field().ok_or(
                            FragmentJoinError::MalformedField {
                                witness: wi,
                                obligation: oi,
                                what: "scan row slot",
                            },
                        )?;
                        map.insert(v.clone(), val);
                    }
                }
            }
            per_obl.push(map);
        }

        // --- Bounded-path obligations. ---
        for oi in 0..branch.path_reach.len() {
            let obligation = &branch.path_reach[oi];
            let &pi = bw.path_proofs.get(oi).ok_or(FragmentJoinError::Structure {
                witness: wi,
                what: "path obligation",
            })?;
            let sp = manifest.sub_proofs.get(pi).ok_or(FragmentJoinError::Structure {
                witness: wi,
                what: "path proof index",
            })?;
            let ProofInputs::PathReach { src_enc, dst_enc, commitments, attribution, .. } =
                &sp.inputs
            else {
                return Err(FragmentJoinError::Structure { witness: wi, what: "path proof kind" });
            };
            path_attrs.push(intern_attribution(commitments, attribution, &mut intern, &mut next_id)
                .ok_or(FragmentJoinError::Structure { witness: wi, what: "path attribution" })?);

            // A VARIABLE path endpoint's value IS the public `src_enc`/`dst_enc`
            // (directly comparable to a scan row slot — the #1678 clean-extension
            // observation). A constant endpoint is bound by `bind_fragment_solution`.
            let combined = n_scans + oi;
            let mut map = std::collections::BTreeMap::new();
            if let SlotPattern::Var(v) = &obligation.subject {
                let val = src_enc.to_field().ok_or(FragmentJoinError::MalformedField {
                    witness: wi,
                    obligation: combined,
                    what: "path src_enc",
                })?;
                map.insert(v.clone(), val);
            }
            if let SlotPattern::Var(v) = &obligation.object {
                let val = dst_enc.to_field().ok_or(FragmentJoinError::MalformedField {
                    witness: wi,
                    obligation: combined,
                    what: "path dst_enc",
                })?;
                map.insert(v.clone(), val);
            }
            per_obl.push(map);
        }

        // (1) JOIN COHERENCE (item 2): every shared variable resolves to ONE value
        // across all atoms (scan↔scan, scan↔path, path↔path). The safe-coarser
        // direction — it can only reject more.
        let mut seen: std::collections::BTreeMap<String, Fr> = std::collections::BTreeMap::new();
        for (idx, map) in per_obl.iter().enumerate() {
            for (v, val) in map {
                match seen.get(v) {
                    None => {
                        seen.insert(v.clone(), *val);
                    }
                    Some(prev) if *prev == *val => {}
                    Some(_) => {
                        return Err(FragmentJoinError::Incoherent {
                            witness: wi,
                            obligation: idx,
                            var: v.clone(),
                        })
                    }
                }
            }
        }

        // (2)+(3) CROSS-GRAPH Q6 obligation (item 1): re-derive the branch's Q6
        // obligations from the PROOF-BOUND attributions (the per-branch analogue of
        // the flat `recheck`). Arity is already validated by `dispatch_fragment`; a
        // mismatch here is refused fail-closed.
        let obligations = branch_obligations(branch, &scan_attrs, &path_attrs)
            .map_err(|_| FragmentJoinError::Structure { witness: wi, what: "branch obligations" })?;

        // (3) A multi-graph path's interior-chain non-bnode obligation is not
        // verifier-dischargeable from disclosed data — refuse fail-closed.
        if let Some(&p) = obligations.path_link_non_bnode.first() {
            return Err(FragmentJoinError::MultiGraphPath { witness: wi, obligation: n_scans + p });
        }

        // (2) Every required cross-graph join edge's shared variable MUST be covered
        // by the disclosed value map of BOTH its atoms (the coherence check above has
        // already forced them equal). A required cross-graph obligation the disclosed
        // data does not cover is refused fail-closed (branch-local `recheck`).
        for edge in &obligations.join_edges {
            let (i, j) = edge.patterns;
            let covered = per_obl.get(i).is_some_and(|m| m.contains_key(&edge.variable))
                && per_obl.get(j).is_some_and(|m| m.contains_key(&edge.variable));
            if !covered {
                return Err(FragmentJoinError::UncoveredCrossGraphJoin {
                    witness: wi,
                    variable: edge.variable.clone(),
                });
            }
        }
    }

    Ok(())
}

/// The per-branch interned committed-graph identity set a sub-proof's PROOF-BOUND
/// `attribution` bits select over its `commitments` (sq-ygk6x). `intern` maps each
/// distinct 32-byte committed-graph identity to a stable per-branch id (distinct
/// graphs -> distinct ids, the same graph -> one id — the flat `global_attributions`
/// safe-coarser discipline). Returns `None` fail-closed on a malformed proof-bound
/// attribution (length != `commitments` — stage 1b `AttributionMalformed` also
/// rejects it) or a commitment that is not a valid field element; under-counting the
/// contributing graphs would WEAKEN the obligation, so it is refused rather than
/// padded.
// [OPUS-4.8] sq-ygk6x.
#[cfg(feature = "extended-fragment")]
fn intern_attribution(
    commitments: &[FieldHex],
    attribution: &[bool],
    intern: &mut std::collections::BTreeMap<[u8; 32], usize>,
    next_id: &mut usize,
) -> Option<BTreeSet<usize>> {
    if attribution.len() != commitments.len() {
        return None;
    }
    let mut out = BTreeSet::new();
    for (g, &bit) in attribution.iter().enumerate() {
        if !bit {
            continue;
        }
        let key = field_to_be_bytes_32(&commitments[g].to_field()?);
        let id = *intern.entry(key).or_insert_with(|| {
            let v = *next_id;
            *next_id += 1;
            v
        });
        out.insert(id);
    }
    Some(out)
}

/// FAIL-CLOSED wave-1 extended-fragment END-TO-END verification (sq-h732x):
/// route a [`crate::manifest::FragmentManifest`]'s property-path / `UNION` /
/// `VALUES` presentation all the way through the cryptographic gate, so an
/// accepted extended query's `bb` sub-proofs actually VERIFY end-to-end — which
/// [`verify_manifest`] alone CANNOT do, because its stage-1 `recheck` rejects
/// every extended query at `fragment_patterns` before any sub-proof runs.
///
/// Five layers, all fail-closed:
/// 1. [`dispatch_fragment`] re-derives the query fragment from the query TEXT
///    alone (`fragment_query`, never the manifest), REFUSES anything outside the
///    wave-1 fragment, and routes each disclosed solution's branch witness to a
///    bound sub-proof of the correct circuit member — mapped into
///    [`CheckError::FragmentDispatch`]. It runs FIRST, before the verifier nonce
///    is burnt or any `bb` subprocess starts, so an outside-fragment / mis-routed
///    presentation is refused without side effects.
/// 2. [`bind_fragment_solution`] (sq-1zf94) binds each bound path's
///    `pred_enc`/`src_enc`/`dst_enc` and each disclosed `VALUES` cell to the
///    encoding the verifier RE-DERIVES from the query text + the disclosed solution
///    — mapped into [`CheckError::FragmentSolution`]. Also before the nonce is
///    burnt, so a proof whose disclosed endpoints are not the claimed terms is
///    refused with no side effects.
/// 3. [`bind_fragment_scans`] (sq-qyfth) binds each solution variable occurring in
///    a BGP scan pattern to the selected disclosed row's slot value (again
///    re-derived from the query text + disclosed solution), enforces the scan
///    answers the query pattern, and checks join coherence across SCAN atoms sharing
///    a variable — mapped into [`CheckError::FragmentScan`]. Also before the nonce
///    is burnt.
/// 4. [`bind_fragment_join_coherence`] (sq-ygk6x) re-derives each branch's Q6
///    obligations from the proof-bound attributions, binds every existential
///    variable shared between two atoms — INCLUDING a scan slot and a `PathReach`
///    endpoint — to one value (enc-equality), enforces the flat cross-graph
///    non-bnode obligation branch-locally for BGP scans, and refuses a multi-graph
///    path — mapped into [`CheckError::FragmentJoin`]. Also before the nonce is
///    burnt.
/// 5. the embedded [`crate::manifest::FragmentManifest::manifest`] is then run
///    through the SAME crypto stage as [`verify_manifest`] (the per-sub-proof
///    public-input reconstruction + canonical-vk + `bb verify`, the
///    verifier-nonce single-use + challenge binding, holder PoP, issuer
///    attestation, revocation, and the hidden gates), EXCEPT that stage-1a's
///    query-fragment acceptance is routed through `fragment_query` (so the
///    extended query is not rejected) and the FLAT per-branch term-binding gates
///    (`bind_query_correctness`/`bind_attributions`/`bind_joins`) are replaced by
///    layers 2–4's extended analogue.
///
/// # Honest scope (LOAD-BEARING — read before relying on this)
/// Layers 2–4 bind the disclosed path predicate/endpoints, `VALUES` cells, and a
/// disclosed solution's BGP-scan variables (all via the audit-#1 byte binding of
/// those public inputs), and layer 4 (sq-ygk6x) additionally enforces the flat
/// cross-graph Q6 non-bnode obligation PER BRANCH for BGP scans and binds every
/// existential variable shared across scan↔scan / scan↔path / path↔path atoms. So an
/// accepted proof's disclosed path endpoints, VALUES-constrained variables, and
/// BGP-scan-bound variables ARE tied to the specific disclosed terms, and its
/// per-branch cross-graph scan AND single-graph-path joins carry the same non-bnode
/// obligation as the flat path. sq-nlulr CLOSED the #1684 residual: the
/// salt-uniqueness gate (audit #9, `bind_issuer_attestations`) now covers
/// PATH-referenced committed graphs as well as SCAN-referenced ones, so a cross-graph
/// join between a scan graph and a single-graph PATH graph is no longer a bare
/// AGREEMENT check — the path graph carries the issuer-attestation requirement AND
/// the distinct-salt record, so its non-bnode corollary is discharged exactly as for
/// a scan↔scan join, and an unattested / salt-colliding path commitment is refused
/// fail-closed. A MULTI-graph path is still refused fail-closed. What remains BY
/// DESIGN (not a gap): an EXISTENTIAL (non-projected) path endpoint's value stays
/// hidden. So the extended regime now carries the SAME attestation + salt discipline
/// as the flat path for scan↔scan / scan↔path / path↔path cross-graph joins. The
/// whole verifier stack is internally re-audited but **NOT externally audited**
/// (sq-qhy4). NO soundness / privacy property is asserted as achieved.
// [OPUS-4.8] sq-h732x: extended-fragment end-to-end routing. Opt-in
// (`extended-fragment`), research-grade, NOT-yet-sound (sq-qhy4).
#[cfg(feature = "extended-fragment")]
#[allow(clippy::too_many_arguments)]
pub fn verify_fragment_manifest(
    fm: &crate::manifest::FragmentManifest,
    prover: &CircuitProver,
    work_dir: &Path,
    trusted_key_set: &KeySet,
    revocation_policy: &RevocationPolicy,
    holder_registry: &HolderRegistry,
    holder_binding_policy: &HolderBindingPolicy,
    entailment_policy: &EntailmentPolicy,
    nonce: &VerifierNonce,
    seen: &dyn SeenNonces,
) -> Result<(), CheckError> {
    // (1) FAIL-CLOSED structural routing FIRST — before the nonce is burnt or any
    // bb subprocess runs. Re-derive the fragment from the query text, refuse
    // anything outside it, and route each branch witness to a bound sub-proof of
    // the correct member.
    dispatch_fragment(fm).map_err(CheckError::FragmentDispatch)?;

    // (2) FAIL-CLOSED DISCLOSED-SOLUTION term binding (sq-1zf94): bind each bound
    // path's pred_enc/src_enc/dst_enc and each disclosed VALUES cell to the encoding
    // the verifier re-derives from the query text + the disclosed solution (never a
    // prover-supplied encoding). Also before the nonce is burnt or any bb runs, so a
    // proof whose disclosed endpoints are not the claimed terms is refused with no
    // side effects. The residual still-deferred surface is documented on
    // `bind_fragment_solution`.
    bind_fragment_solution(fm).map_err(CheckError::FragmentSolution)?;

    // (2b) FAIL-CLOSED BGP SCAN-SLOT binding (sq-qyfth): bind each solution
    // variable occurring in a BGP scan pattern to the selected disclosed row's slot
    // value (re-derived from the query text + disclosed solution), and check join
    // coherence across scan atoms sharing a variable. Also before the nonce is burnt
    // or any bb runs.
    bind_fragment_scans(fm).map_err(CheckError::FragmentScan)?;

    // (2c) FAIL-CLOSED PER-BRANCH JOIN COHERENCE + cross-graph Q6 non-bnode
    // (sq-ygk6x): re-derive each branch's Q6 obligations from the proof-bound
    // per-obligation attributions and bind every existential variable shared between
    // atoms — INCLUDING a scan slot and a `PathReach` endpoint (`src_enc`/`dst_enc`)
    // — to one value (enc-equality). Enforces the flat cross-graph non-bnode
    // obligation branch-locally for BGP scans (via the enc-equality + the active
    // salt-uniqueness gate) and refuses a multi-graph path whose interior-chain
    // non-bnode obligation the verifier cannot discharge. Also before the nonce is
    // burnt or any bb runs. The remaining residual is documented on
    // `bind_fragment_join_coherence`.
    bind_fragment_join_coherence(fm).map_err(CheckError::FragmentJoin)?;

    // (3) Crypto stage over the EMBEDDED manifest, with stage-1a routed through
    // `fragment_query` (`skip_query_binding = true`) so the extended query is not
    // rejected. Every other gate is identical to `verify_manifest` (see
    // `verify_manifest_impl` / `prefilter_manifest_structure_impl`). The deferred
    // per-branch term binding is the documented sq-1zf94 / sq-qyfth limitation.
    verify_manifest_impl(
        &fm.manifest,
        prover,
        work_dir,
        trusted_key_set,
        revocation_policy,
        holder_registry,
        holder_binding_policy,
        entailment_policy,
        nonce,
        seen,
        true,
    )
}

/// Reconstruct the bb `public_inputs` byte vector from the DECLARED
/// `ProofInputs` (audit #1), in each member's `main` declaration order, using
/// the verifier's `challenge` as public-input field 0. bb's layout (determined
/// empirically against bb 5.0.0-nightly): each public input is one 32-byte
/// big-endian field element, structs/arrays flattened in index order
/// (row-major), `bool` -> {0,1}, `u32`/`u64` -> the integer value; no header,
/// no length prefix. The result is byte-compared against the proof's
/// `public_inputs`.
///
/// Declaration order — the single source of truth is each
/// `zk/compose/<member>/src/main.nr` (mirrored 1:1 by `toml.rs`):
/// - scan_k{k}_n{n}_r{r}: challenge, commitments[k], pattern_is_const[3],
///   pattern_const_enc[3], rows[r][3] (row-major), row_count, attribution[k]
///   (audit #8 — `bool` -> {0,1}, one word per committed graph).
/// - filter_int_d{d}: challenge, operand_enc, op, bound, expected.
///
/// Crucially the layout is sized by the DECLARED `CircuitId`'s `r`/`k` (rows
/// padded to `r`, commitments are exactly `k`), so a relabel that disagrees
/// with the actual member also fails (#11): a wrong `r` produces a vector of
/// the wrong length, which cannot byte-match the proof of the real member.
// [OPUS-4.8] public-input reconstruction (audit #1, subsumes #7/#11 with vk pin).
fn reconstruct_public_inputs(
    inputs: &ProofInputs,
    challenge: &FieldHex,
    proof: usize,
) -> Result<Vec<u8>, CheckError> {
    let mut out: Vec<u8> = Vec::new();
    // Append a field element parsed from a FieldHex, rejecting malformed hex.
    fn push_field(
        out: &mut Vec<u8>,
        h: &FieldHex,
        proof: usize,
        what: &'static str,
    ) -> Result<(), CheckError> {
        let f = h
            .to_field()
            .ok_or(CheckError::MalformedField { proof, what })?;
        out.extend_from_slice(&field_to_be_bytes_32(&f));
        Ok(())
    }
    // Append a small integer (u32/u64/bool/op-code) as a 32-byte BE word.
    fn push_uint(out: &mut Vec<u8>, v: u64) {
        let mut word = [0u8; 32];
        word[24..].copy_from_slice(&v.to_be_bytes());
        out.extend_from_slice(&word);
    }

    // Field 0 is always the verifier's challenge (every member's first `pub`).
    push_field(&mut out, challenge, proof, "challenge")?;

    match inputs {
        ProofInputs::Scan {
            commitments,
            pattern_is_const,
            pattern_const_enc,
            rows,
            row_count,
            attribution,
            ..
        } => {
            // commitments[k] — exactly k words (k is the declared CircuitId.k,
            // re-derived from commitments.len() in stage 1b).
            for c in commitments {
                push_field(&mut out, c, proof, "commitments")?;
            }
            // pattern_is_const[3] (bool -> {0,1}).
            for b in pattern_is_const {
                push_uint(&mut out, u64::from(*b));
            }
            // pattern_const_enc[3] (variable slots carry 0x0).
            for e in pattern_const_enc {
                push_field(&mut out, e, proof, "pattern_const_enc")?;
            }
            // rows[r][3], row-major, PADDED to the declared CircuitId.r with
            // zero rows (matching the prover's pad_rows / the circuit's R).
            let r = match inputs.circuit_id() {
                CircuitId::Scan { r, .. } => *r as usize,
                _ => return Err(CheckError::MalformedField { proof, what: "scan id" }),
            };
            for j in 0..r {
                match rows.get(j) {
                    Some(row) => {
                        for slot in row {
                            push_field(&mut out, slot, proof, "rows")?;
                        }
                    }
                    // Padding row: three zero words.
                    None => out.extend_from_slice(&[0u8; 96]),
                }
            }
            // row_count: u32.
            push_uint(&mut out, u64::from(*row_count));
            // attribution[k]: bool -> {0,1}, one word per committed graph (audit
            // #8). Bound to the proof so the verifier-side cross-check against
            // manifest.attributions operates over a PROOF-BOUND quantity. The
            // declared CircuitId.k is the authority for the length (re-derived
            // from commitments.len() in stage 1b); a wrong-length attribution
            // therefore yields a wrong-length vector that cannot byte-match.
            //
            // [OPUS-4.8] codex 2221 MEDIUM: `verify_manifest` runs
            // `prefilter_manifest_structure` (stage 1b) FIRST, which already rejects
            // any scan whose `attribution.len() != k` (`AttributionMalformed`), so
            // on this path `attribution` is exactly k bits. The `unwrap_or(false)`
            // below is therefore belt-and-braces only (never the pad-to-false
            // bypass — that is rejected upstream); it cannot widen the accepted
            // shape because the structural gate runs unconditionally before here.
            let k = match inputs.circuit_id() {
                CircuitId::Scan { k, .. } => *k as usize,
                _ => return Err(CheckError::MalformedField { proof, what: "scan id" }),
            };
            for g in 0..k {
                let bit = attribution.get(g).copied().unwrap_or(false);
                push_uint(&mut out, u64::from(bit));
            }
        }
        ProofInputs::FilterInt { operand_enc, op, bound, expected, .. } => {
            push_field(&mut out, operand_enc, proof, "operand_enc")?;
            push_uint(&mut out, u64::from(op.code()));
            push_uint(&mut out, *bound);
            push_uint(&mut out, u64::from(*expected));
        }
        // [OPUS-4.8] sq-q7e + sq-tat: filter_f64_d{d} public inputs, in
        // `main` declaration order: challenge (pushed above), operand_enc, op,
        // b_bits (the constant double's IEEE-754 bit pattern as a u64 word),
        // expected.
        ProofInputs::FilterF64 { operand_enc, op, b_bits, expected, .. } => {
            push_field(&mut out, operand_enc, proof, "operand_enc")?;
            push_uint(&mut out, u64::from(op.code()));
            push_uint(&mut out, *b_bits);
            push_uint(&mut out, u64::from(*expected));
        }
        // [OPUS-4.8] sq-7lrq: filter_signed_int_d{md} public inputs, in `main`
        // declaration order: challenge (pushed above), operand_enc, op, bound_neg
        // (bool -> {0,1}), bound (the constant's u64 magnitude), expected.
        // Cross-reference `zk/compose/filter_signed_int_d{md}/src/main.nr`.
        ProofInputs::FilterSignedInt { operand_enc, op, bound_neg, bound, expected, .. } => {
            push_field(&mut out, operand_enc, proof, "operand_enc")?;
            push_uint(&mut out, u64::from(op.code()));
            push_uint(&mut out, u64::from(*bound_neg));
            push_uint(&mut out, *bound);
            push_uint(&mut out, u64::from(*expected));
        }
        // [OPUS-4.8] sq-7lrq: filter_decimal_i{id}_f{fd} public inputs, in `main`
        // declaration order: challenge (pushed above), operand_enc, op, bound_neg
        // (bool -> {0,1}), bound_scaled (the host-prescaled constant magnitude),
        // expected. Cross-reference `zk/compose/filter_decimal_i{id}_f{fd}/src/main.nr`.
        ProofInputs::FilterDecimal { operand_enc, op, bound_neg, bound_scaled, expected, .. } => {
            push_field(&mut out, operand_enc, proof, "operand_enc")?;
            push_uint(&mut out, u64::from(op.code()));
            push_uint(&mut out, u64::from(*bound_neg));
            push_uint(&mut out, *bound_scaled);
            push_uint(&mut out, u64::from(*expected));
        }
        // [OPUS-4.8] sq-xojl: filter_value_dl_int (DUAL-LEAF value lane) public
        // inputs, in `main` declaration order: challenge (pushed above),
        // operand_enc, op, bound, datatype_const, expected. Cross-reference
        // `zk/compose/filter_value_dl_int/src/main.nr`. `operand_enc` is the
        // DUAL-LEAF leaf `h3(h3(VALUE_HOOK, datatype_const, LANG_NONE),
        // lexical_component, TYPE_CODE_LITERAL)`; the verifier reconstructs the
        // SAME public-input vector the prover serialized, so a `filter_value_dl_int`
        // sub-proof's bb verification runs. (Like the other FILTER members, this is
        // the public-input SERIALIZATION; the `(method, circuit)` legality + the
        // scan-binding gate are the dispatch matrix `crate::dispatch` (sq-cfmv) and
        // the host-encoding wiring (sq-j506), gated behind sq-qhy4.)
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDl { operand_enc, op, bound, datatype_const, expected, .. } => {
            push_field(&mut out, operand_enc, proof, "operand_enc")?;
            push_uint(&mut out, u64::from(op.code()));
            push_uint(&mut out, *bound);
            push_field(&mut out, datatype_const, proof, "datatype_const")?;
            push_uint(&mut out, u64::from(*expected));
        }
        // [OPUS-4.8] sq-2ezsx: filter_value_dl_f64 (DUAL-LEAF double value lane)
        // public inputs, in `main` declaration order: challenge (pushed above),
        // operand_enc, op, b_bits (the FILTER constant as IEEE-754 bits),
        // datatype_const, expected. Cross-reference
        // `zk/compose/filter_value_dl_f64/src/main.nr`. `operand_enc` is the
        // DUAL-LEAF leaf over the CANONICAL IEEE bits.
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDlF64 { operand_enc, op, b_bits, datatype_const, expected, .. } => {
            push_field(&mut out, operand_enc, proof, "operand_enc")?;
            push_uint(&mut out, u64::from(op.code()));
            push_uint(&mut out, *b_bits);
            push_field(&mut out, datatype_const, proof, "datatype_const")?;
            push_uint(&mut out, u64::from(*expected));
        }
        // [OPUS-4.8] sq-2ezsx: filter_value_dl_decimal (DUAL-LEAF decimal value
        // lane) public inputs, in `main` declaration order: challenge (pushed
        // above), operand_enc, op, bound_neg (bool -> {0,1}), bound_scaled (the
        // host-prescaled constant magnitude at the canonical scale), datatype_const
        // (folds the datatype AND the scale), expected. Cross-reference
        // `zk/compose/filter_value_dl_decimal/src/main.nr`.
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDlDecimal {
            operand_enc,
            op,
            bound_neg,
            bound_scaled,
            datatype_const,
            expected,
            ..
        } => {
            push_field(&mut out, operand_enc, proof, "operand_enc")?;
            push_uint(&mut out, u64::from(op.code()));
            push_uint(&mut out, u64::from(*bound_neg));
            push_uint(&mut out, *bound_scaled);
            push_field(&mut out, datatype_const, proof, "datatype_const")?;
            push_uint(&mut out, u64::from(*expected));
        }
        // [OPUS-5] sq-wz99x: filter_value_dl_datetime (DUAL-LEAF dateTime/date
        // value lane) public inputs, in `main` declaration order: challenge (pushed
        // above), operand_enc, op, bound_neg (bool -> {0,1}), bound_scaled_epoch
        // (the FILTER constant instant as |T| in milliseconds on the XSD
        // timeOnTimeline), datatype_const (SELECTS the dateTime or date lane AND
        // folds the scale FS), expected. Cross-reference
        // `zk/compose/filter_value_dl_datetime/src/main.nr`. The layout is the
        // decimal member's with `bound_scaled` renamed — ONE member, and here ONE
        // reconstruction, serves both lanes, because the lane is carried by the
        // `datatype_const` public input (so a lane swap changes THIS vector).
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDlDateTime {
            operand_enc,
            op,
            bound_neg,
            bound_scaled_epoch,
            datatype_const,
            expected,
            ..
        } => {
            push_field(&mut out, operand_enc, proof, "operand_enc")?;
            push_uint(&mut out, u64::from(op.code()));
            push_uint(&mut out, u64::from(*bound_neg));
            push_uint(&mut out, *bound_scaled_epoch);
            push_field(&mut out, datatype_const, proof, "datatype_const")?;
            push_uint(&mut out, u64::from(*expected));
        }
        // [OPUS-4.8] sq-bwwl / sq-fi03 (step 3): hidden cross-credential JOIN
        // public inputs, in the `join_eq` member's `main` declaration order:
        // challenge (pushed above), commit_a, commit_b, join_commitment, slot_a,
        // slot_b. Cross-reference `zk/compose/join_eq_na16_nb16/src/main.nr`. This
        // is the pure public-input SERIALIZATION (the audit-#1 byte-layout) — it
        // does NOT perform the `bind_joins` gate (commitment-equality to the scan
        // proofs, the `UnboundJoin` query binding, the slot binding), which is
        // step 4 (sq-sfsi). Reconstructing the vector here only lets a `join_eq`
        // sub-proof's bb verification run; it bypasses no check, because no check
        // beyond plain bb verification is wired for joins until sq-sfsi.
        ProofInputs::JoinEq { commit_a, commit_b, join_commitment, slot_a, slot_b, .. } => {
            push_field(&mut out, commit_a, proof, "commit_a")?;
            push_field(&mut out, commit_b, proof, "commit_b")?;
            push_field(&mut out, join_commitment, proof, "join_commitment")?;
            push_uint(&mut out, u64::from(*slot_a));
            push_uint(&mut out, u64::from(*slot_b));
        }
        // [OPUS-4.8] sq-3kd2g.6: bounded-depth path reachability public inputs, in
        // the `path_reach_d{d}_k{k}_n{n}` member's `main` declaration order:
        // challenge (pushed above), commitments[k], pred_enc, src_enc, dst_enc,
        // allow_zero (bool -> {0,1}), depth_bound (u32), attribution[k] (bool ->
        // {0,1}). Cross-reference `zk/compose/path_reach_d{d}_k{k}_n{n}/src/main.nr`
        // — do not reorder. The `commitments` / `attribution` length is the declared
        // `CircuitId.k` (re-derived from `commitments.len()` in stage 1b), so a
        // wrong-length attribution yields a wrong-length vector that cannot
        // byte-match the real member's proof. `depth_bound` is the PUBLIC bound the
        // consumer sees (soundness req 1); it is also constant-constrained to the
        // member's `D` in-circuit.
        #[cfg(feature = "extended-fragment")]
        ProofInputs::PathReach {
            commitments,
            pred_enc,
            src_enc,
            dst_enc,
            allow_zero,
            depth_bound,
            attribution,
            ..
        } => {
            for c in commitments {
                push_field(&mut out, c, proof, "commitments")?;
            }
            push_field(&mut out, pred_enc, proof, "pred_enc")?;
            push_field(&mut out, src_enc, proof, "src_enc")?;
            push_field(&mut out, dst_enc, proof, "dst_enc")?;
            push_uint(&mut out, u64::from(*allow_zero));
            push_uint(&mut out, u64::from(*depth_bound));
            let k = match inputs.circuit_id() {
                CircuitId::PathReach { k, .. } => *k as usize,
                _ => return Err(CheckError::MalformedField { proof, what: "path id" }),
            };
            for g in 0..k {
                let bit = attribution.get(g).copied().unwrap_or(false);
                push_uint(&mut out, u64::from(bit));
            }
        }
    }
    Ok(out)
}

/// SubProof `proof_hex` blob layout: `len(proof) | proof | len(pi) | pi | vk`,
/// each length a 4-byte big-endian u32. Keeps the three bb artifacts together
/// in one manifest field.
pub fn encode_artifacts(art: &crate::driver::ProofArtifacts) -> String {
    let mut blob = Vec::new();
    push_lp(&mut blob, &art.proof);
    push_lp(&mut blob, &art.public_inputs);
    blob.extend_from_slice(&art.vk);
    hex_encode(&blob)
}

/// Decoded bb artifacts split out of the `proof_hex` blob. `vk` is the
/// PROVER-supplied vk and is NOT trusted by `verify_manifest` (audit #2); it is
/// retained only for tooling / round-trip tests.
struct DecodedArtifacts {
    proof: Vec<u8>,
    public_inputs: Vec<u8>,
    #[allow(dead_code)]
    vk: Vec<u8>,
}

/// Decode the `proof_hex` blob. Returns `None` on any malformed length prefix /
/// truncation (the caller rejects via `CheckError::MalformedProof` rather than
/// panicking — audit hardening; prover-controlled bytes reach here before bb).
fn decode_artifacts(blob: &[u8]) -> Option<DecodedArtifacts> {
    let (proof, rest) = take_lp(blob)?;
    let (public_inputs, vk) = take_lp(rest)?;
    Some(DecodedArtifacts {
        proof: proof.to_vec(),
        public_inputs: public_inputs.to_vec(),
        vk: vk.to_vec(),
    })
}

fn push_lp(buf: &mut Vec<u8>, data: &[u8]) {
    buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
    buf.extend_from_slice(data);
}

/// Read a 4-byte-big-endian-length-prefixed segment, returning `(body, rest)`.
/// `None` if the buffer is shorter than the prefix or the prefix overruns it
/// (no panic on prover-controlled bytes — audit hardening).
fn take_lp(buf: &[u8]) -> Option<(&[u8], &[u8])> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    let end = 4usize.checked_add(len)?;
    if end > buf.len() {
        return None;
    }
    Some((&buf[4..end], &buf[end..]))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Decode a hex string to bytes. `None` on odd length or any non-hex nibble —
/// the caller rejects via `CheckError::MalformedProof` rather than panicking
/// (audit hardening: `proof_hex` is prover-controlled and reaches the verifier
/// before any bb call; the old `.expect("valid hex")` + OOB slice aborted the
/// process under the release `panic="abort"` profile).
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    // [OPUS-4.8] sq-hbg7: stable-1.96 clippy `manual_is_multiple_of`.
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

// [OPUS-4.8] sq-nlulr: shared test fixtures for the audit-#9 issuer-attestation +
// salt-uniqueness gate. Kept at parent scope (compiled only under `#[cfg(test)]`)
// so BOTH the flat `mod tests` and the `extended-fragment` `fragment_dispatch_tests`
// build an attestation of the IDENTICAL shape — the scan- and path-covering paths
// exercise the same attestation discipline. `commitment` is an arbitrary field
// element (the gate verifies the signature over the given commitment value; it does
// not recompute it from triples), so tests can pick distinct commitments/salts.
#[cfg(test)]
const TEST_STATUS_LIST: &str = "http://ex/status/1";
#[cfg(test)]
const TEST_STATUS_INDEX: u64 = 3;
#[cfg(test)]
const TEST_STATUS_VERSION: u64 = 1;

/// A valid salt- AND status-bound issuer attestation over `commitment` (the
/// scan-/path-verify path requires a salt+status-bound attestation).
#[cfg(test)]
fn test_attestation(
    commitment: Fr,
    salt: Fr,
    sk: &sparq_zk::sig::SecretKey,
) -> crate::manifest::CommitmentAttestation {
    test_attestation_at_index(commitment, salt, sk, TEST_STATUS_INDEX)
}

/// As [`test_attestation`], but over a CHOSEN status-list `index` — the signature is
/// formed over that index's `status_ref_digest`, so the attestation is internally
/// valid and the manifest reaches the reference-resolution step. Lets a test build a
/// presentation whose two credentials occupy DISTINCT status slots (sq-cuvmj).
#[cfg(test)]
fn test_attestation_at_index(
    commitment: Fr,
    salt: Fr,
    sk: &sparq_zk::sig::SecretKey,
    index: u64,
) -> crate::manifest::CommitmentAttestation {
    let list_id = sparq_zk::sig::status_list_id_to_field(TEST_STATUS_LIST);
    let status_ref = sparq_zk::sig::status_ref_digest(&list_id, index, TEST_STATUS_VERSION);
    crate::manifest::CommitmentAttestation {
        commitment: FieldHex::from_field(&commitment),
        issuer_public_key: sparq_zk::sig::public_key_to_hex(&sk.public_key()),
        signature: sk.sign_commitment_with_status(&commitment, &salt, &status_ref),
        cryptosuite: sparq_zk::sig::SignatureScheme::Poseidon2SchnorrV1
            .cryptosuite_iri()
            .to_string(),
        salt: Some(FieldHex::from_field(&salt)),
        status: Some(crate::manifest::AttestedStatusRef {
            index: Some(index),
            version: Some(TEST_STATUS_VERSION),
            index_commitment: None,
            ref_commitment: None,
        }),
        holder: None,
    }
}

/// The disclosed revocation reference matching [`test_attestation`]'s signed status
/// reference (required so the issuer gate can recompute the signed status digest).
#[cfg(test)]
fn test_revocation() -> crate::manifest::RevocationStatus {
    crate::manifest::RevocationStatus {
        status_list: Some(TEST_STATUS_LIST.to_string()),
        index: Some(TEST_STATUS_INDEX),
        version: Some(TEST_STATUS_VERSION),
        index_commitment: None,
        ref_commitment: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::FilterOp;

    fn fh(s: &str) -> FieldHex {
        FieldHex(s.to_string())
    }

    // [OPUS-4.8] sq-r6dq: the hidden-issuer trust-anchor derivation
    // ([`KeySet::hidden_issuer_root`]) now uses the SPARSE key-set builder
    // (sq-8k3h), so a relying party with a large/growing issuer registry can pick a
    // deep tree without the verifier materialising `2^depth` leaves. Pin (a) the
    // switch is VALUE-PRESERVING — the (sparse) anchor is bit-identical to the dense
    // root at depths the dense builder can still compute — and (b) it is FEASIBLE
    // AND SELF-CONSISTENT at a deep tree the dense builder could not materialise: a
    // trusted member's sparse authentication path must re-fold (LSB-first index bits,
    // the circuit's fold) to exactly the anchor the verifier derives.
    //
    // The two parts are NOT the same strength of evidence and must not be quoted as
    // one. (a) is a CROSS-CHECK against an independently implemented oracle (the
    // dense builder). (b) is SELF-CONSISTENCY ONLY: `hidden_issuer_root` and
    // `key_membership_witness_sparse` both go through
    // `issuer::sparse_fold_leaves`, so (b) catches an ISOLATED wrong anchor or
    // wrong sibling but a CORRELATED common-mode error inside that shared fold
    // would pass it. No dense oracle exists at depth 24 (2^24 leaves is not
    // materialisable), which is exactly why (a) stops at depth 12. The independent
    // deep oracle is `tests/e2e.rs::prove_hidden_issuer` (dense prover root vs
    // sparse verifier anchor through a real bb proof), not this test.
    #[test]
    fn hidden_issuer_root_uses_sparse_builder_and_scales() {
        use sparq_zk::sig::{key_set_leaf, public_key_to_hex, SecretKey};
        let sks: Vec<SecretKey> = (0u64..5).map(|s| SecretKey::from_seed(9000 + s)).collect();
        let hexes: Vec<String> = sks.iter().map(|sk| public_key_to_hex(&sk.public_key())).collect();
        let k = KeySet::from_hex_keys(hexes).with_hidden_issuer_depth(8);
        let ordered = k.ordered_keys();
        assert_eq!(ordered.len(), 5, "all five real keys are trusted");

        // (a) VALUE-PRESERVING: the sparse trust anchor equals the dense root over
        // the SAME canonical key order, at depths the dense builder can compute.
        for depth in [4u32, 8, 12] {
            assert_eq!(
                k.hidden_issuer_root(depth),
                crate::issuer::key_set_root(&ordered, depth),
                "sparse trust anchor must equal the dense root (depth {depth})"
            );
        }

        // (b) FEASIBLE + CORRECT at a deep tree: 2^24 = 16.7M dense slots — only the
        // sparse builder can derive this anchor. Every trusted member's sparse path
        // must re-fold to exactly the anchor the verifier derives — the Merkle
        // root/path consistency this test pins. (Only that narrow equality is
        // demonstrated here; the hidden-issuer ZK construction overall remains
        // research-grade, external cryptographer audit pending sq-qhy4.)
        let deep = 24u32;
        let anchor = k.hidden_issuer_root(deep).expect("deep-tree anchor");
        for index in 0..ordered.len() as u64 {
            let sibs = crate::issuer::key_membership_witness_sparse(&ordered, deep, index)
                .expect("deep sparse path");
            assert_eq!(sibs.len(), deep as usize);
            let mut node = key_set_leaf(&ordered[index as usize]).unwrap();
            let mut pos = index;
            for sib in &sibs {
                let is_right = pos & 1 == 1;
                node = if is_right {
                    sparq_zk::poseidon2::hash(&[*sib, node])
                } else {
                    sparq_zk::poseidon2::hash(&[node, *sib])
                };
                pos /= 2;
            }
            assert_eq!(
                node, anchor,
                "member {index}'s sparse path re-folds to the verifier's deep anchor"
            );
        }

        // fail-closed parity retained: an implausible depth is still None.
        assert_eq!(k.hidden_issuer_root(32), None);
    }

    /// [OPUS-5] sq-r6dq review round 3 (gpt-5.6-sol finding 7): the depth-mismatch
    /// gate `hi.depth != depth` in [`bind_hidden_issuer_attestations`] had NO
    /// functional test — only a `Display`-string test in `tests/verifier_errors.rs`,
    /// which formats a hand-built `CheckError` and never runs the comparison. So
    /// deleting the gate, or flipping `!=` to `==`, reddened nothing.
    ///
    /// This drives the REAL gate: an attestation declaring a depth other than the
    /// policy's must be rejected with `HiddenIssuerDepthMismatch{declared, policy}`
    /// carrying BOTH values, and the matching declared depth must get PAST the gate
    /// (it then fails later on the deliberately unreferenced commitment, which is
    /// how we know the depth check did not swallow it).
    ///
    /// Why the verifier's own VK selection is unaffected: `CircuitId::HiddenIssuer`
    /// is built from `depth` — the POLICY depth — never from `hi.depth`. This gate
    /// exists so a prover cannot silently present a proof for a different-depth
    /// member; the vk it is verified against was never the attacker's to choose.
    #[test]
    fn hidden_issuer_declared_depth_must_equal_policy_depth() {
        let mk = |declared: u32| crate::manifest::HiddenIssuerAttestation {
            commitment: fh("0x7"),
            depth: declared,
            key_set_root: fh("0x8"),
            message: fh("0x9"),
            salt: None,
            proof_hex: "00".into(),
        };
        // `revocation: None` => `scan_referenced_messages` returns an empty map
        // WITHOUT error, so the depth gate is the first thing the loop evaluates.
        let mut m = minimal_manifest("SELECT * WHERE { ?s <http://ex/p> ?o }");
        let prover = CircuitProver::from_crate_root();
        let work = std::env::temp_dir().join("sq_r6dq_hi_depth_gate");
        let policy_depth = 4u32;
        let k = KeySet::empty().with_hidden_issuer_depth(policy_depth);
        assert!(
            k.hidden_issuer_root(policy_depth).is_some(),
            "an empty KeySet still derives an all-padding anchor, so the gate under \
             test is reached rather than short-circuited by RootUnavailable"
        );

        // MISMATCH (both directions: shallower and deeper than the policy) => reject,
        // with the declared/policy pair reported.
        for declared in [3u32, 5, 0, 31] {
            m.hidden_issuer_attestations = vec![mk(declared)];
            match bind_hidden_issuer_attestations(&m, &k, &prover, &work, &fh("0x2a")) {
                Err(CheckError::HiddenIssuerDepthMismatch { declared: d, policy: p }) => {
                    assert_eq!(d, declared, "the REJECTED declared depth is reported");
                    assert_eq!(p, policy_depth, "the POLICY depth is reported");
                }
                other => panic!(
                    "a hidden-issuer attestation declaring depth {declared} under a \
                     depth-{policy_depth} policy must be HiddenIssuerDepthMismatch, got {other:?}"
                ),
            }
        }

        // MATCH => the depth gate passes and the entry proceeds to the referenced-
        // commitment check, which rejects for a DIFFERENT (non-depth) reason. This is
        // the half that fails if the gate is inverted (`==` instead of `!=`).
        m.hidden_issuer_attestations = vec![mk(policy_depth)];
        match bind_hidden_issuer_attestations(&m, &k, &prover, &work, &fh("0x2a")) {
            Err(CheckError::HiddenIssuerUnreferencedCommitment { .. }) => {}
            other => panic!(
                "a MATCHING declared depth must pass the depth gate and fail later on \
                 the unreferenced commitment, got {other:?}"
            ),
        }

        // The gate fires BEFORE any per-entry proof work, and on the FIRST offending
        // entry: a good entry followed by a bad one still rejects on depth.
        m.hidden_issuer_attestations = vec![mk(policy_depth), mk(policy_depth + 1)];
        assert!(
            matches!(
                bind_hidden_issuer_attestations(&m, &k, &prover, &work, &fh("0x2a")),
                Err(CheckError::HiddenIssuerUnreferencedCommitment { .. })
                    | Err(CheckError::HiddenIssuerDepthMismatch { .. })
            ),
            "a mixed batch must still be rejected fail-closed"
        );
    }

    // [OPUS-4.8] EMPIRICAL anchor for audit #1: the reconstruction must equal
    // the REAL bb `public_inputs` blobs captured by `bb prove --write_vk -t
    // noir-recursive` on the compiled members (bb 5.0.0-nightly.20260324). If a
    // toolchain bump changes the serialization these byte-vectors break loudly
    // — exactly the gate the binding layer needs. The hex below is the verbatim
    // probe output (see STATUS.md DESIGN); no nargo/bb needed to run this test.

    /// `filter_int_d1` over: challenge=0x2a, operand_enc=0x0831…943b, op=Lt(0),
    /// bound=10, expected=true. 5 fields * 32 = 160 bytes.
    #[test]
    fn reconstruct_filter_int_matches_real_bb_public_inputs() {
        let bb = hex_decode(concat!(
            "000000000000000000000000000000000000000000000000000000000000002a",
            "0831327030a1bd8f46862134e0d6273c75d1d33f4b3334fca37d9f7ed1a7943b",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000a",
            "0000000000000000000000000000000000000000000000000000000000000001",
        ))
        .unwrap();
        let inputs = ProofInputs::FilterInt {
            id: CircuitId::FilterInt { d: 1 },
            operand_enc: fh("0x0831327030a1bd8f46862134e0d6273c75d1d33f4b3334fca37d9f7ed1a7943b"),
            op: FilterOp::Lt,
            bound: 10,
            expected: true,
        };
        let got = reconstruct_public_inputs(&inputs, &fh("0x2a"), 0).unwrap();
        assert_eq!(got.len(), 160);
        assert_eq!(got, bb, "filter_int reconstruction must byte-match bb");
    }

    /// `scan_k1_n16_r4` over the probe values. 22 fields * 32 = 704 bytes; the
    /// single active row plus 3 zero-padded rows exercise the row-major
    /// flattening and the pad-to-`r` path, and the trailing `attribution[1]`
    /// word (audit #8) exercises the per-graph source-attribution binding.
    ///
    /// [OPUS-4.8] audit #8: the trailing `...01` word is the in-circuit
    /// attribution for the single committed graph (the pattern matches in it).
    /// Regenerated by the `probe_scan_public_inputs_hex` test (e2e.rs, ignored)
    /// against a real `bb prove` of the new scan_k1_n16_r4 — the empirical anchor.
    #[test]
    fn reconstruct_scan_matches_real_bb_public_inputs() {
        let bb = hex_decode(concat!(
            // challenge
            "000000000000000000000000000000000000000000000000000000000000002a",
            // commitments[0]
            "000c6024571cb9fc261106500dbacab6fbd7fdafd50566f881c68b9fde9cffe1",
            // pattern_is_const [false,true,false]
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000000",
            // pattern_const_enc [0, 0x0579…5b61, 0]
            "0000000000000000000000000000000000000000000000000000000000000000",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "0000000000000000000000000000000000000000000000000000000000000000",
            // rows[0] = active match
            "067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "2b5caeb2bbd290ab32434a9109030784c7faebadee7a9908d24dccb847910d1d",
            // rows[1..4] = zero rows (9 zero words)
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            // row_count = 1
            "0000000000000000000000000000000000000000000000000000000000000001",
            // attribution[0] = true (the single graph matches the pattern)
            "0000000000000000000000000000000000000000000000000000000000000001",
        ))
        .unwrap();
        let z = || fh("0x0");
        let inputs = ProofInputs::Scan {
            id: CircuitId::Scan { k: 1, n: 16, r: 4 },
            commitments: vec![fh(
                "0x000c6024571cb9fc261106500dbacab6fbd7fdafd50566f881c68b9fde9cffe1",
            )],
            pattern_is_const: [false, true, false],
            pattern_const_enc: [
                z(),
                fh("0x057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61"),
                z(),
            ],
            // Only the active row is declared; reconstruction pads to r=4.
            rows: vec![[
                fh("0x067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713"),
                fh("0x057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61"),
                fh("0x2b5caeb2bbd290ab32434a9109030784c7faebadee7a9908d24dccb847910d1d"),
            ]],
            row_count: 1,
            // The single graph contributes => attribution[0] = true (audit #8).
            attribution: vec![true],
        };
        let got = reconstruct_public_inputs(&inputs, &fh("0x2a"), 0).unwrap();
        assert_eq!(got.len(), 704);
        assert_eq!(got, bb, "scan reconstruction must byte-match bb");
    }

    /// [OPUS-4.8] sq-f9tl (re-audit NEW-1): EMPIRICAL anchor for the
    /// `filter_f64_d{d}` family — the re-audit flagged it as relying on layout
    /// reasoning with NO captured golden vector. `filter_f64_d2` over:
    /// challenge=0x2a, operand_enc=0x1a9c…c990 ("25"^^xsd:double), op=Ge(3),
    /// b_bits=0x4032000000000000 (18.0 IEEE-754), expected=true. 5 fields * 32 =
    /// 160 bytes. Captured verbatim by `probe_filter_f64_public_inputs_hex`
    /// (e2e.rs, ignored) from a real `bb prove`; a toolchain bump that changes
    /// the f64 serialization breaks this test loudly.
    #[test]
    fn reconstruct_filter_f64_matches_real_bb_public_inputs() {
        let bb = hex_decode(concat!(
            // challenge
            "000000000000000000000000000000000000000000000000000000000000002a",
            // operand_enc ("25"^^xsd:double)
            "1a9cfccd1a2354f0e79fefda14e00216260fce60b75bd34cace5606856c3c990",
            // op = Ge (3)
            "0000000000000000000000000000000000000000000000000000000000000003",
            // b_bits = 18.0_f64 = 0x4032000000000000
            "0000000000000000000000000000000000000000000000004032000000000000",
            // expected = true
            "0000000000000000000000000000000000000000000000000000000000000001",
        ))
        .unwrap();
        let inputs = ProofInputs::FilterF64 {
            id: CircuitId::FilterF64 { d: 2 },
            operand_enc: fh("0x1a9cfccd1a2354f0e79fefda14e00216260fce60b75bd34cace5606856c3c990"),
            op: FilterOp::Ge,
            b_bits: 18.0_f64.to_bits(),
            expected: true,
        };
        let got = reconstruct_public_inputs(&inputs, &fh("0x2a"), 0).unwrap();
        assert_eq!(got.len(), 160);
        assert_eq!(got, bb, "filter_f64 reconstruction must byte-match bb");
    }

    /// [OPUS-4.8] sq-7lrq: EMPIRICAL anchor for the composable `filter_signed_int_d{md}`
    /// family. `filter_signed_int_d2` over: challenge=0x2a, operand_enc=0x25f9…a120
    /// ("-42"^^xsd:integer), op=Lt(0), bound_neg=false, bound=1, expected=true. 6
    /// fields * 32 = 192 bytes. Captured verbatim by
    /// `probe_filter_signed_int_public_inputs_hex` (e2e.rs, ignored) from a real
    /// `bb prove`; a toolchain bump that changes the serialization breaks this
    /// loudly. The two extra words over `filter_int` are the sign-split bound
    /// (`bound_neg`, `bound` magnitude).
    #[test]
    fn reconstruct_filter_signed_int_matches_real_bb_public_inputs() {
        let bb = hex_decode(concat!(
            // challenge
            "000000000000000000000000000000000000000000000000000000000000002a",
            // operand_enc ("-42"^^xsd:integer)
            "25f95edbf033080613232d81e9851bafcb0addf47bcffcb02e388298dec5a120",
            // op = Lt (0)
            "0000000000000000000000000000000000000000000000000000000000000000",
            // bound_neg = false
            "0000000000000000000000000000000000000000000000000000000000000000",
            // bound = 1 (the +1 magnitude)
            "0000000000000000000000000000000000000000000000000000000000000001",
            // expected = true
            "0000000000000000000000000000000000000000000000000000000000000001",
        ))
        .unwrap();
        let inputs = ProofInputs::FilterSignedInt {
            id: CircuitId::FilterSignedInt { md: 2 },
            operand_enc: fh("0x25f95edbf033080613232d81e9851bafcb0addf47bcffcb02e388298dec5a120"),
            op: FilterOp::Lt,
            bound_neg: false,
            bound: 1,
            expected: true,
        };
        let got = reconstruct_public_inputs(&inputs, &fh("0x2a"), 0).unwrap();
        assert_eq!(got.len(), 192);
        assert_eq!(got, bb, "filter_signed_int reconstruction must byte-match bb");
    }

    /// [OPUS-4.8] sq-7lrq: EMPIRICAL anchor for the composable
    /// `filter_decimal_i{id}_f{fd}` family. `filter_decimal_i3_f2` over:
    /// challenge=0x2a, operand_enc=0x2711…8ddd ("123.45"^^xsd:decimal), op=Gt(2),
    /// bound_neg=false, bound_scaled=12340 (0x3034 = round(123.40*100)),
    /// expected=true. 6 fields * 32 = 192 bytes. Captured verbatim by
    /// `probe_filter_decimal_public_inputs_hex` (e2e.rs, ignored) from a real
    /// `bb prove`. The host-prescaled `bound_scaled` is the only layout difference
    /// from the signed-int member.
    #[test]
    fn reconstruct_filter_decimal_matches_real_bb_public_inputs() {
        let bb = hex_decode(concat!(
            // challenge
            "000000000000000000000000000000000000000000000000000000000000002a",
            // operand_enc ("123.45"^^xsd:decimal)
            "271130972d0065afdc11c7ac94fd97f113e2cc1d8a6f8771d8d4116446138ddd",
            // op = Gt (2)
            "0000000000000000000000000000000000000000000000000000000000000002",
            // bound_neg = false
            "0000000000000000000000000000000000000000000000000000000000000000",
            // bound_scaled = 12340 (0x3034)
            "0000000000000000000000000000000000000000000000000000000000003034",
            // expected = true
            "0000000000000000000000000000000000000000000000000000000000000001",
        ))
        .unwrap();
        let inputs = ProofInputs::FilterDecimal {
            id: CircuitId::FilterDecimal { id: 3, fd: 2 },
            operand_enc: fh("0x271130972d0065afdc11c7ac94fd97f113e2cc1d8a6f8771d8d4116446138ddd"),
            op: FilterOp::Gt,
            bound_neg: false,
            bound_scaled: 12340,
            expected: true,
        };
        let got = reconstruct_public_inputs(&inputs, &fh("0x2a"), 0).unwrap();
        assert_eq!(got.len(), 192);
        assert_eq!(got, bb, "filter_decimal reconstruction must byte-match bb");
    }

    /// [OPUS-4.8] sq-f9tl (re-audit NEW-1): EMPIRICAL anchor for the k=2 scan
    /// family (`scan_k2_n16_r8`) — the other un-anchored member the re-audit
    /// flagged. Two named-graph credentials, each with 3 matching `ex:age`
    /// triples (6 active rows, padded to r=8) and BOTH contributing (audit #8
    /// attribution = [true, true]). 36 fields * 32 = 1152 bytes; this exercises
    /// the k=2 commitment array, the trailing two-bit attribution word group, and
    /// the r=8 pad path that the k=1 anchor cannot. Captured verbatim by
    /// `probe_scan_k2_public_inputs_hex` (e2e.rs, ignored) from a real `bb prove`.
    #[test]
    fn reconstruct_scan_k2_matches_real_bb_public_inputs() {
        let bb = hex_decode(concat!(
            // challenge
            "000000000000000000000000000000000000000000000000000000000000002a",
            // commitments[0], commitments[1]
            "3009ad0a7a02313686bfb8e9224a7a9a5d2f90653b3905243ffc826f8b1c4baf",
            "1e182eccb67380e0a29dfbdd337b27daee9b2bc0c6d35e9dbc55be67b2acfa90",
            // pattern_is_const [false,true,false]
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000000",
            // pattern_const_enc [0, ex:age, 0]
            "0000000000000000000000000000000000000000000000000000000000000000",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "0000000000000000000000000000000000000000000000000000000000000000",
            // rows[0..6] = the 6 active matched rows (row-major s,p,o)
            "067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "08a387a1d4e98da1fcf28c25bc413f88d5ff771682c18f432f0f03971fad9602",
            "067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "1aff50a8f430c8288c8a386adefff02a20c75db111a6ef0dcb71e3d63c36e39e",
            "067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "1e76b7d5b462137832e8c482bf0e84cd27acff5af29ab19f584b7e4e5279c3c6",
            "067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "132fa587351bf3f12fd3cbed64d5526f28791099d1d40870f94595873c78fa72",
            "067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "30202cbdd89765b9370d4790b6f7f073a95ef2ffbd731b53ce49cd2eb86614f0",
            "067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713",
            "057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61",
            "16cd94467389dbde075372360eadb589f417d9e8c79507a34293ef3478b2d68b",
            // rows[6..8] = 2 zero rows (6 zero words) padding to r=8
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            // row_count = 6
            "0000000000000000000000000000000000000000000000000000000000000006",
            // attribution[0], attribution[1] = both true (audit #8)
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000001",
        ))
        .unwrap();
        // [OPUS-4.8] sq-f9tl (Copilot review): each closure is named for the (s,p,o)
        // COLUMN it fills, so the anchor rows read in triple order — `subj` is the
        // subject (ex:alice), `age` the predicate (ex:age), `z` a zero field.
        let z = || fh("0x0");
        let subj = || fh("0x067d8d75b405117a4cce58d59db1cbe420dabf0ff8d4c0fa50d80ccf0ed4a713");
        let age = || fh("0x057914fc592ade7b970f92e4992958ea8a6a265caeb012b5b63903257eef5b61");
        let inputs = ProofInputs::Scan {
            id: CircuitId::Scan { k: 2, n: 16, r: 8 },
            commitments: vec![
                fh("0x3009ad0a7a02313686bfb8e9224a7a9a5d2f90653b3905243ffc826f8b1c4baf"),
                fh("0x1e182eccb67380e0a29dfbdd337b27daee9b2bc0c6d35e9dbc55be67b2acfa90"),
            ],
            pattern_is_const: [false, true, false],
            pattern_const_enc: [z(), age(), z()],
            rows: vec![
                [subj(), age(), fh("0x08a387a1d4e98da1fcf28c25bc413f88d5ff771682c18f432f0f03971fad9602")],
                [subj(), age(), fh("0x1aff50a8f430c8288c8a386adefff02a20c75db111a6ef0dcb71e3d63c36e39e")],
                [subj(), age(), fh("0x1e76b7d5b462137832e8c482bf0e84cd27acff5af29ab19f584b7e4e5279c3c6")],
                [subj(), age(), fh("0x132fa587351bf3f12fd3cbed64d5526f28791099d1d40870f94595873c78fa72")],
                [subj(), age(), fh("0x30202cbdd89765b9370d4790b6f7f073a95ef2ffbd731b53ce49cd2eb86614f0")],
                [subj(), age(), fh("0x16cd94467389dbde075372360eadb589f417d9e8c79507a34293ef3478b2d68b")],
            ],
            row_count: 6,
            attribution: vec![true, true],
        };
        let got = reconstruct_public_inputs(&inputs, &fh("0x2a"), 0).unwrap();
        assert_eq!(got.len(), 1152);
        assert_eq!(got, bb, "scan_k2 reconstruction must byte-match bb");
    }

    /// A different declared statement (a single mutated field) must produce a
    /// different vector — the property the byte-compare relies on (audit #1).
    #[test]
    fn reconstruct_is_statement_sensitive() {
        let base = ProofInputs::FilterInt {
            id: CircuitId::FilterInt { d: 1 },
            operand_enc: fh("0x05"),
            op: FilterOp::Ge,
            bound: 18,
            expected: true,
        };
        let v0 = reconstruct_public_inputs(&base, &fh("0x2a"), 0).unwrap();
        // Flip the bound 18 -> 17.
        let mut m = base.clone();
        if let ProofInputs::FilterInt { bound, .. } = &mut m {
            *bound = 17;
        }
        assert_ne!(v0, reconstruct_public_inputs(&m, &fh("0x2a"), 0).unwrap());
        // Flip the challenge.
        assert_ne!(v0, reconstruct_public_inputs(&base, &fh("0x2b"), 0).unwrap());
    }

    /// Malformed `proof_hex` is rejected, never panics (audit hardening).
    #[test]
    fn hex_decode_and_take_lp_reject_bad_bytes() {
        assert!(hex_decode("zz").is_none()); // non-hex
        assert!(hex_decode("abc").is_none()); // odd length
        assert!(hex_decode("").unwrap().is_empty());
        assert!(take_lp(&[0, 0, 0]).is_none()); // < 4-byte prefix
        assert!(take_lp(&[0, 0, 0, 255, 1, 2]).is_none()); // oversized length
        assert!(decode_artifacts(&[0, 0, 0, 1, 9]).is_none()); // pi prefix missing
    }

    /// Audit #4: the in-memory single-use store records a nonce on first sight
    /// (fresh => true) and rejects it on every subsequent sight (=> false).
    #[test]
    fn in_memory_seen_nonces_is_single_use() {
        let store = InMemorySeenNonces::new();
        let n = VerifierNonce::from_hex("0x2a").unwrap();
        // First presentation is fresh; second (and third) are not.
        assert!(store.record_fresh(&n), "first sight must be fresh");
        assert!(!store.record_fresh(&n), "replay must be rejected");
        assert!(!store.record_fresh(&n), "replay stays rejected");
        // A DIFFERENT nonce is independently fresh.
        let n2 = VerifierNonce::from_hex("0x2b").unwrap();
        assert!(store.record_fresh(&n2));
    }

    /// A unique temp path for a durable-store test (cleaned by the test).
    fn tmp_nonce_log(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sparq_zk_compose_seen_nonces_{tag}_{}_{:?}.log",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    /// sq-aih (audit #4 durability): the DURABLE store records a nonce, and a
    /// FRESH store REOPENED FROM THE SAME PATH (modelling a verifier restart —
    /// the in-memory state is gone) STILL REJECTS the reused nonce. This is the
    /// load-bearing property the in-memory store lacks.
    #[test]
    fn file_seen_nonces_single_use_survives_restart() {
        let path = tmp_nonce_log("restart");
        let _ = std::fs::remove_file(&path);
        let n = VerifierNonce::from_hex("0x2a").unwrap();

        // Session 1: first sight is fresh, replay rejected. Then DROP the store
        // (its in-memory fd/state is gone — a restart).
        {
            let store = FileSeenNonces::open(&path).expect("open durable store");
            assert!(store.record_fresh(&n), "first sight must be fresh");
            assert!(!store.record_fresh(&n), "replay rejected within the session");
        }

        // Session 2: a COMPLETELY FRESH store reopened from the SAME path. The
        // nonce must still be seen — single-use survived the restart.
        {
            let store = FileSeenNonces::open(&path).expect("reopen durable store");
            assert!(
                !store.record_fresh(&n),
                "a nonce recorded before restart must STILL be rejected after reopening the store"
            );
            // A different nonce is independently fresh in the reopened store.
            let n2 = VerifierNonce::from_hex("0x2b").unwrap();
            assert!(store.record_fresh(&n2), "an unseen nonce is fresh after restart");
        }

        // Session 3: confirm the second nonce is now ALSO durable.
        {
            let store = FileSeenNonces::open(&path).expect("reopen durable store");
            let n2 = VerifierNonce::from_hex("0x2b").unwrap();
            assert!(!store.record_fresh(&n2), "the post-restart nonce is durable too");
        }

        let _ = std::fs::remove_file(&path);
    }

    /// sq-aih: the durable store keys by CANONICAL field value too — a re-padded
    /// hex spelling of an already-recorded nonce is still rejected after a restart
    /// (mirrors the in-memory `seen_nonces_key_is_representation_insensitive`).
    #[test]
    fn file_seen_nonces_key_is_representation_insensitive_across_restart() {
        let path = tmp_nonce_log("repr");
        let _ = std::fs::remove_file(&path);
        let padded = VerifierNonce::from_hex(
            "0x000000000000000000000000000000000000000000000000000000000000002a",
        )
        .unwrap();
        let bare = VerifierNonce::from_hex("0x2a").unwrap();
        {
            let store = FileSeenNonces::open(&path).expect("open");
            assert!(store.record_fresh(&padded), "first sight fresh");
        }
        {
            // Restart, then present the SAME field under a different spelling.
            let store = FileSeenNonces::open(&path).expect("reopen");
            assert!(
                !store.record_fresh(&bare),
                "same field, different spelling => replay even across a restart"
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Audit #4: a nonce is keyed by its CANONICAL field value, so two hex
    /// spellings of the same field element collapse to one single-use entry (a
    /// prover cannot re-present by re-padding the hex).
    #[test]
    fn seen_nonces_key_is_representation_insensitive() {
        let store = InMemorySeenNonces::new();
        let padded = VerifierNonce::from_hex(
            "0x000000000000000000000000000000000000000000000000000000000000002a",
        )
        .unwrap();
        let bare = VerifierNonce::from_hex("0x2a").unwrap();
        assert!(store.record_fresh(&padded), "first sight fresh");
        assert!(!store.record_fresh(&bare), "same field, different spelling => replay");
    }

    /// Audit #4: the nonce round-trips to the FieldHex the reconstruction binds
    /// as field 0 (canonical 0x-prefixed 64-nibble hex).
    #[test]
    fn verifier_nonce_round_trips_to_field_hex() {
        let n = VerifierNonce::from_hex("0x2a").unwrap();
        assert_eq!(
            n.as_field_hex(),
            FieldHex(
                "0x000000000000000000000000000000000000000000000000000000000000002a".to_string()
            )
        );
        // A reconstruction with the nonce as challenge equals one with the
        // equivalent FieldHex challenge (the nonce IS the challenge anchor).
        let inputs = ProofInputs::FilterInt {
            id: CircuitId::FilterInt { d: 1 },
            operand_enc: fh("0x05"),
            op: FilterOp::Ge,
            bound: 18,
            expected: true,
        };
        let via_nonce =
            reconstruct_public_inputs(&inputs, &n.as_field_hex(), 0).unwrap();
        let via_hex = reconstruct_public_inputs(&inputs, &fh("0x2a"), 0).unwrap();
        assert_eq!(via_nonce, via_hex);
    }

    /// Audit #4: a non-field nonce hex is rejected at construction (fail-closed —
    /// a relying party cannot mint an unparseable nonce).
    #[test]
    fn verifier_nonce_rejects_malformed_hex() {
        assert!(VerifierNonce::from_hex("0xzz").is_none());
        assert!(VerifierNonce::from_hex("").is_none());
    }

    /// A non-field hex string in a declared slot is rejected (no panic).
    #[test]
    fn reconstruct_rejects_malformed_field() {
        let inputs = ProofInputs::FilterInt {
            id: CircuitId::FilterInt { d: 1 },
            operand_enc: fh("0xnot-a-field"),
            op: FilterOp::Ge,
            bound: 18,
            expected: true,
        };
        assert!(matches!(
            reconstruct_public_inputs(&inputs, &fh("0x2a"), 0),
            Err(CheckError::MalformedField { proof: 0, what: "operand_enc" })
        ));
    }

    // --- sq-h732x: stage-1 NON-REGRESSION (runs in BOTH feature states) -----

    /// A ProofManifest carrying only the required fields — enough to drive the
    /// flat stage-1 pre-filter for the query-fragment non-regression checks.
    fn minimal_manifest(query: &str) -> ProofManifest {
        ProofManifest {
            r#type: "urn:sparq:zk:ProofManifest".to_string(),
            query: query.to_string(),
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
            sub_proofs: vec![],
            binding_edges: vec![],
            join_edges: vec![],
            hidden_revocation: None,
            hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
            fully_hidden_revocation: None,
        }
    }

    #[test]
    fn stage1_prefilter_still_rejects_union_fragment() {
        // [OPUS-4.8] sq-h732x: the FLAT stage-1 pre-filter still rejects an
        // extended (UNION) query at `recheck` with the SAME structured error as
        // before the extended-fragment routing landed — byte-identical whether or
        // not the `extended-fragment` feature is compiled in.
        let m = minimal_manifest(
            "SELECT * WHERE { { ?s <http://ex/p> ?o } UNION { ?o <http://ex/q> ?s } }",
        );
        let err = prefilter_manifest_structure(
            &m,
            &KeySet::empty(),
            &RevocationPolicy::accept_version(1),
        )
        .unwrap_err();
        match err {
            CheckError::Sparqzk(VerifyError::UnsupportedFragment(msg)) => {
                assert!(msg.contains("UNION"), "expected UNION rejection, got {msg}");
            }
            other => panic!("flat stage-1 must still reject UNION, got {other:?}"),
        }
    }

    /// [OPUS-4.8] sq-nlulr: FLAT-PATH INVARIANCE. The audit-#9 issuer-attestation +
    /// salt-uniqueness gate over a FLAT (scan-only) manifest must behave IDENTICALLY
    /// whether or not `extended-fragment` is compiled in — the new `PathReach` match
    /// arm is cfg-gated and a flat manifest never reaches it (no `PathReach`
    /// sub-proof exists in the default build). This test lives in the
    /// always-compiled `mod tests`, so it runs in BOTH `cargo test` and `cargo test
    /// --features extended-fragment` and must give the SAME verdict in each.
    #[test]
    fn bind_issuer_attestations_flat_scan_is_feature_state_invariant() {
        let sk = sparq_zk::sig::SecretKey::from_seed(1);
        let k = KeySet::from_hex_keys([sparq_zk::sig::public_key_to_hex(&sk.public_key())]);
        let commit = Fr::from(100u64);
        let scan = ProofInputs::Scan {
            id: CircuitId::Scan { k: 1, n: 16, r: 4 },
            commitments: vec![FieldHex::from_field(&commit)],
            pattern_is_const: [true, true, false],
            pattern_const_enc: [fh("0x1"), fh("0x2"), fh("0x0")],
            rows: vec![],
            row_count: 0,
            attribution: vec![false],
        };
        // Attested + salt-bound => the flat scan passes the issuer + salt gate.
        let mut ok = minimal_manifest("SELECT * WHERE { ?s <http://ex/p> ?o }");
        ok.sub_proofs =
            vec![crate::manifest::SubProof { inputs: scan, proof_hex: String::new() }];
        ok.commitment_attestations = vec![test_attestation(commit, Fr::from(7u64), &sk)];
        ok.revocation = Some(test_revocation());
        assert!(
            bind_issuer_attestations(&ok, &k, &std::collections::BTreeSet::new()).is_ok(),
            "a flat attested scan must pass the issuer gate identically in both feature states"
        );
        // Drop the attestation => the flat scan is refused (deterministic, both states).
        let mut bad = ok.clone();
        bad.commitment_attestations.clear();
        assert!(
            matches!(
                bind_issuer_attestations(&bad, &k, &std::collections::BTreeSet::new()),
                Err(CheckError::UnattestedCommitment { proof: 0, .. })
            ),
            "an unattested flat scan must be refused identically in both feature states"
        );
    }

    /// A single-graph BGP `Scan` over a CHOSEN committed graph (the issuer gate
    /// verifies the signature over the given commitment value; it never recomputes
    /// it from triples, so a test may pick the commitment freely).
    fn flat_scan_with_commit(commit: Fr) -> ProofInputs {
        ProofInputs::Scan {
            id: CircuitId::Scan { k: 1, n: 16, r: 4 },
            commitments: vec![FieldHex::from_field(&commit)],
            pattern_is_const: [true, true, false],
            pattern_const_enc: [fh("0x1"), fh("0x2"), fh("0x0")],
            rows: vec![],
            row_count: 0,
            attribution: vec![false],
        }
    }

    /// [OPUS-5] sq-cuvmj: THE SCALAR-`revocation` TRIPWIRE.
    ///
    /// Pins the single-reference invariant `ProofManifest::revocation` documents: a
    /// presentation carrying TWO credentials whose issuer-signed status references
    /// occupy DISTINCT slots is structurally REJECTED, because every scan-covering
    /// commitment must resolve to the ONE disclosed reference. Both attestations
    /// here are internally VALID (key in K, signature verifies over each one's own
    /// `status_ref_digest`, distinct salts), so the manifest reaches
    /// `resolve_status_ref` and the rejection is the reference comparison itself —
    /// not an incidental signature or salt failure.
    ///
    /// This is FAIL-CLOSED, not a false-accept (§Finding B of
    /// `research/zk-bind-composition-review.md`): the second credential's liveness is
    /// never skipped, it simply cannot be presented. The cost is that hidden
    /// cross-credential joins ([`bind_joins`]) are restricted to credentials sharing
    /// a status slot.
    ///
    /// TRIPWIRE: a future `Vec` migration of `revocation`/`hidden_revocation` WILL
    /// turn this test red — that is the point. Before changing it, discharge the
    /// per-commitment obligations pre-registered on `ProofManifest::revocation`;
    /// flipping the expectation to "accepted" without them is exactly the
    /// unchecked-second-credential regression this pins.
    #[test]
    fn two_credentials_with_distinct_status_refs_are_rejected() {
        let sk = sparq_zk::sig::SecretKey::from_seed(1);
        let k = KeySet::from_hex_keys([sparq_zk::sig::public_key_to_hex(&sk.public_key())]);
        let c_a = Fr::from(100u64); // credential A, status index TEST_STATUS_INDEX
        let c_b = Fr::from(200u64); // credential B, a DIFFERENT slot on the same list
        let other_index = TEST_STATUS_INDEX + 6;

        let mut m = minimal_manifest("SELECT * WHERE { ?s <http://ex/p> ?o }");
        m.sub_proofs = vec![
            crate::manifest::SubProof {
                inputs: flat_scan_with_commit(c_a),
                proof_hex: String::new(),
            },
            crate::manifest::SubProof {
                inputs: flat_scan_with_commit(c_b),
                proof_hex: String::new(),
            },
        ];
        m.commitment_attestations = vec![
            test_attestation(c_a, Fr::from(7u64), &sk),
            test_attestation_at_index(c_b, Fr::from(9u64), &sk, other_index),
        ];
        // The ONE disclosed reference — A's slot. B's issuer-signed reference cannot
        // also match it.
        m.revocation = Some(test_revocation());

        let err = bind_issuer_attestations(&m, &k, &std::collections::BTreeSet::new())
            .unwrap_err();
        assert!(
            matches!(err, CheckError::RevocationReferenceMismatch { .. }),
            "two credentials on distinct status slots must be refused at the reference gate (sq-cuvmj fail-closed), got {err:?}"
        );

        // CONTROL: the SAME manifest with both credentials on the SAME slot is
        // accepted — so the rejection above is the distinct-reference constraint,
        // not a vacuous failure of the two-scan fixture itself.
        let mut same_slot = m.clone();
        same_slot.commitment_attestations = vec![
            test_attestation(c_a, Fr::from(7u64), &sk),
            test_attestation(c_b, Fr::from(9u64), &sk),
        ];
        assert!(
            bind_issuer_attestations(&same_slot, &k, &std::collections::BTreeSet::new()).is_ok(),
            "two credentials sharing one status slot must pass — the fixture is otherwise valid"
        );
    }

    /// [OPUS-5] sq-93h: THE SALT-DISCLOSURE DOMINATION TRIP-WIRE.
    ///
    /// sq-93h asked whether the per-graph salt that `HiddenIssuerAttestation` carries
    /// on the sq-xxg HIDDEN-ONLY path is a residual cross-presentation linkability
    /// channel. Assessment (`research/zk-hidden-path-salt-disclosure.md`): NO — it is
    /// DOMINATED, because the same graph's `C(G)` is disclosed in the clear on the very
    /// same entry (and byte-bound into the scan sub-proof's bb public inputs). Any two
    /// presentations linkable by salt are already linkable by `C(G)`, so hiding the salt
    /// behind an in-circuit salt-commitment would buy zero unlinkability.
    ///
    /// That verdict rests on ONE premise — `C(G)` stays public. This test pins it on the
    /// REAL paths, never on a test-local notion of "disclosed":
    /// - the hidden-only salt fallback actually resolves (red if `resolve_commitment_salt`
    ///   loses its hidden-entry arm);
    /// - `reconstruct_public_inputs` — the function whose output the verifier
    ///   BYTE-COMPARES against each proof's `public_inputs` — emits `C(G)` as scan
    ///   public-input word 1, salt or no salt; and
    /// - on the WIRE (serde round-trip of the salt-withheld manifest) `C(G)` survives
    ///   while the salt is gone, and the round-tripped manifest still reconstructs the
    ///   same `C(G)`-bearing public inputs.
    ///
    /// TRIP-WIRE: a future hidden / re-randomised-commitment tier stops emitting the
    /// cleartext `C(G)` word and turns this red. That is the intended signal — at that
    /// point the salt becomes the finest remaining correlator and sq-93h must be
    /// RE-OPENED, not the assertion relaxed.
    ///
    /// SCOPE (do not over-read): this pins premise (D1) — `C(G)` disclosure — only.
    /// Domination additionally assumes the ISSUANCE discipline that a salt is never
    /// reused for two distinct graphs; the verifier machine-checks only the
    /// within-manifest instance of that (`SaltReused`), so no test here can establish
    /// it across presentations. See §3 of the research record.
    #[test]
    fn hidden_only_salt_disclosure_is_dominated_by_the_clear_commitment() {
        let c = Fr::from(4242u64);
        let salt = Fr::from(1357u64);

        // A HIDDEN-ONLY presentation: one scan over `c`, one hidden-issuer entry over
        // `c` carrying the salt, and NO clear attestation to read the salt from.
        let mut with_salt = minimal_manifest("SELECT * WHERE { ?s <http://ex/p> ?o }");
        with_salt.sub_proofs = vec![crate::manifest::SubProof {
            inputs: flat_scan_with_commit(c),
            proof_hex: String::new(),
        }];
        with_salt.hidden_issuer_attestations = vec![crate::manifest::HiddenIssuerAttestation {
            commitment: FieldHex::from_field(&c),
            depth: 4,
            key_set_root: fh("0x8"),
            message: fh("0x9"),
            salt: Some(FieldHex::from_field(&salt)),
            proof_hex: String::new(),
        }];
        assert!(
            with_salt.commitment_attestations.is_empty(),
            "the fixture must be HIDDEN-ONLY — a clear attestation would supply the salt \
             by the preferred path and the hidden fallback would go untested"
        );

        // (a) The hidden-only fallback resolves the salt (the sq-xxg behaviour).
        assert_eq!(
            resolve_commitment_salt(&with_salt, &c),
            Some(salt),
            "a hidden-only commitment must resolve its salt from the hidden entry"
        );

        // The counterfactual: the SAME presentation with the salt WITHHELD, i.e. what an
        // in-circuit salt-commitment would achieve on the disclosure surface.
        let mut without_salt = with_salt.clone();
        without_salt.hidden_issuer_attestations[0].salt = None;
        assert_eq!(
            resolve_commitment_salt(&without_salt, &c),
            None,
            "withholding the salt must actually remove it from the disclosure surface — \
             otherwise the comparison below is vacuous"
        );

        // (b) Premise (D1) on the REAL verification path: `C(G)` is not merely a JSON
        // field, it is BYTE-BOUND into the scan sub-proof's bb public inputs — the blob
        // stage 3a byte-compares against the prover's proof. Reconstruct it exactly as
        // the verifier does (challenge = word 0, `commitments[k]` next).
        let challenge = match &without_salt.binding {
            BindingMode::Challenge { challenge } => challenge.clone(),
            other => panic!("fixture must use the challenge binding, got {:?}", other),
        };
        let c_word = field_to_be_bytes_32(&c);
        let pi = reconstruct_public_inputs(&without_salt.sub_proofs[0].inputs, &challenge, 0)
            .expect("the scan sub-proof's public inputs must reconstruct");
        assert_eq!(
            pi.get(32..64),
            Some(&c_word[..]),
            "premise (D1): the committed graph's C(G) is emitted in the CLEAR as scan \
             public-input word 1 even on the hidden-only path, so it cannot be withheld \
             without redesigning the scan member — if this fails, sq-93h must be RE-OPENED"
        );
        assert_eq!(
            pi,
            reconstruct_public_inputs(&with_salt.sub_proofs[0].inputs, &challenge, 0)
                .expect("the scan sub-proof's public inputs must reconstruct"),
            "withholding the salt changes NOT ONE BYTE of the scan's public inputs — the \
             salt is not among them, C(G) is"
        );

        // (c) DOMINATION on the wire: serialize the salt-withheld presentation (what an
        // in-circuit salt-commitment would achieve on the disclosure surface) and confirm
        // the salt is really gone while `C(G)` — the correlator a colluding verifier pair
        // would use — survives, and still reconstructs the same public inputs.
        let salt_hex = field_to_hex(&salt);
        let with_json = serde_json::to_string(&with_salt).expect("manifest serializes");
        let without_json = serde_json::to_string(&without_salt).expect("manifest serializes");
        assert!(
            with_json.contains(&salt_hex),
            "the fixture must actually disclose the salt on the wire, else the \
             counterfactual below is vacuous"
        );
        assert!(
            !without_json.contains(&salt_hex),
            "withholding must actually remove the salt from the wire form"
        );
        assert!(
            without_json.contains(&field_to_hex(&c)),
            "premise (D1) on the wire: C(G) survives the salt being withheld — hiding the \
             salt buys no cross-presentation unlinkability (sq-93h NO-BUILD)"
        );
        let round: ProofManifest =
            serde_json::from_str(&without_json).expect("salt-withheld manifest round-trips");
        assert_eq!(
            reconstruct_public_inputs(&round.sub_proofs[0].inputs, &challenge, 0)
                .expect("the round-tripped scan's public inputs must reconstruct"),
            pi,
            "a verifier that only ever sees the salt-withheld wire form still reconstructs \
             the SAME C(G)-bearing public inputs"
        );
    }

    // ---------------------------------------------------------------
    // [OPUS-5] sq-6qe: the ACCEPTED-SET trust anchor (host side).
    // ---------------------------------------------------------------

    fn status_snapshot(list: &str, version: u64, bits: Vec<u8>) -> StatusListSnapshot {
        StatusListSnapshot { status_list: list.to_string(), version, bits }
    }

    /// A policy accepting versions 8..=10 with three attached snapshots — one
    /// STALE (v7), two FRESH (v9, v10) — at hidden-index depth 4 and accepted-set
    /// depth 3.
    fn accepted_set_policy() -> RevocationPolicy {
        RevocationPolicy::up_to(10, 2)
            .with_snapshots([
                status_snapshot("http://ex/a", 7, vec![0u8, 0]),
                status_snapshot("http://ex/a", 9, vec![0u8, 0]),
                status_snapshot("http://ex/b", 10, vec![0b0000_0100u8, 0]),
            ])
            .with_hidden_index_depth(4)
            .with_accepted_set_depth(3)
    }

    // [OPUS-5] sq-6qe: the SOUNDNESS-relevant property of the accepted set — it is
    // FRESHNESS-CURATED. Because the designed fully-hidden statement is membership
    // in this set, a stale (or future-dated) version must not be a member at all,
    // or moving the audit-#12 freshness gate behind the commitment would silently
    // drop it. Also pins the CANONICAL leaf order (sorted by (list, version)) that
    // the relying party and the prover must both commit.
    #[test]
    fn accepted_entries_are_freshness_curated_and_canonically_ordered() {
        let policy = accepted_set_policy();
        let entries = policy.accepted_entries().expect("entries derivable");
        let names: Vec<(String, u64)> = entries
            .iter()
            .map(|e| (e.status_list.clone(), e.version))
            .collect();
        assert_eq!(
            names,
            vec![("http://ex/a".to_string(), 9), ("http://ex/b".to_string(), 10)],
            "stale v7 excluded; the rest in sorted (list, version) order"
        );
        assert_eq!(policy.min_version(), 8, "now=10, window=2");
        assert_eq!(policy.accepted_member_index("http://ex/a", 9), Some(0));
        assert_eq!(policy.accepted_member_index("http://ex/b", 10), Some(1));
        assert_eq!(
            policy.accepted_member_index("http://ex/a", 7),
            None,
            "a stale (list, version) is not a member — no proof can be built for it"
        );

        // A FUTURE-dated snapshot (beyond `now`) is likewise excluded.
        let with_future = accepted_set_policy()
            .with_snapshot(status_snapshot("http://ex/c", 11, vec![0u8, 0]));
        assert_eq!(
            with_future.accepted_member_index("http://ex/c", 11),
            None,
            "a future-dated version is outside [min_version, now] and not accepted"
        );
        assert_eq!(
            with_future.accepted_set_root(),
            accepted_set_policy().accepted_set_root(),
            "an out-of-window snapshot must not move the anchor"
        );
    }

    // [OPUS-5] sq-6qe: the anchor is derived from the relying party's OWN
    // authoritative bitstrings — each entry's `status_list_root` is exactly
    // `revocation::merkle_root` of its snapshot at the hidden-index depth, and the
    // policy root is the accepted-set fold over those entries. So a change to the
    // authoritative bits (a newly REVOKED credential) moves the anchor, which is
    // what keeps the future in-circuit bit-unset fold bound to real liveness data.
    #[test]
    fn accepted_set_root_folds_the_policys_own_authoritative_roots() {
        let policy = accepted_set_policy();
        let entries = policy.accepted_entries().unwrap();
        assert_eq!(
            entries[0].status_list_root,
            crate::revocation::merkle_root(&status_snapshot("http://ex/a", 9, vec![0u8, 0]), 4)
                .unwrap(),
            "entry root is merkle_root of the RP's own snapshot at hidden_index_depth"
        );
        assert_eq!(
            policy.accepted_set_root(),
            crate::revocation::accepted_set_root(&entries, 3),
            "the policy anchor is the accepted-set fold over those entries"
        );

        // Revoking a bit in an authoritative snapshot changes that entry's root and
        // therefore the whole anchor.
        let revoked = RevocationPolicy::up_to(10, 2)
            .with_snapshots([
                status_snapshot("http://ex/a", 9, vec![0b0000_0001u8, 0]),
                status_snapshot("http://ex/b", 10, vec![0b0000_0100u8, 0]),
            ])
            .with_hidden_index_depth(4)
            .with_accepted_set_depth(3);
        assert_ne!(
            revoked.accepted_set_root(),
            policy.accepted_set_root(),
            "a revoked authoritative bit must move the accepted-set anchor"
        );
    }

    // [OPUS-5] sq-6qe: fail-closed derivation. Without an opted-in accepted-set
    // depth, or without the hidden-index depth each entry's status-list root is
    // derived at, or when the curated set overflows the tree, the anchor is `None`
    // — never a partial or truncated trust anchor.
    #[test]
    fn accepted_set_root_is_fail_closed_when_underspecified_or_overflowing() {
        let base = RevocationPolicy::up_to(10, 2).with_snapshots([
            status_snapshot("http://ex/a", 9, vec![0u8, 0]),
            status_snapshot("http://ex/b", 10, vec![0u8, 0]),
        ]);
        assert_eq!(
            base.clone().with_hidden_index_depth(4).accepted_set_root(),
            None,
            "no accepted-set depth opted in => no anchor"
        );
        assert_eq!(
            base.clone().with_accepted_set_depth(3).accepted_set_root(),
            None,
            "no hidden-index depth => entry roots are underivable => no anchor"
        );
        assert_eq!(
            base.clone()
                .with_hidden_index_depth(4)
                .with_accepted_set_depth(0)
                .accepted_set_root(),
            None,
            "2 curated entries do not fit a 2^0 tree => fail closed, not truncated"
        );
        assert!(
            base.with_hidden_index_depth(4)
                .with_accepted_set_depth(1)
                .accepted_set_root()
                .is_some(),
            "2 curated entries fit a 2^1 tree"
        );
    }
}

// [OPUS-4.8] sq-3kd2g.6: FAIL-CLOSED wave-1 fragment DISPATCH tests — the
// acceptance-set negatives (a path claimed without a bound sub-proof; a k / member
// mismatch; branch attribution at the wrong branch) plus the accept + closure +
// VALUES routing. Structural (no bb): the crypto binding is the verify_manifest
// loop; the term binding to disclosed solutions is a documented follow-up.
#[cfg(all(test, feature = "extended-fragment"))]
mod fragment_dispatch_tests {
    use super::*;
    use crate::manifest::{BranchWitness, FragmentManifest, SubProof};

    fn fh(s: &str) -> FieldHex {
        FieldHex(s.to_string())
    }

    /// A `PathReach` input whose id is EXACTLY `(d, k, n)` and whose
    /// `commitments`/`attribution` have length `k` and `depth_bound == d` (a
    /// consistent, id-hygiene-passing member).
    fn path_ok(d: u32, k: u32, n: u32, allow_zero: bool) -> ProofInputs {
        ProofInputs::PathReach {
            id: CircuitId::PathReach { d, k, n },
            commitments: (0..k).map(|i| fh(&format!("0x{:x}", i + 1))).collect(),
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero,
            depth_bound: d,
            attribution: vec![true; k as usize],
        }
    }

    fn scan_min() -> ProofInputs {
        ProofInputs::Scan {
            id: CircuitId::Scan { k: 1, n: 16, r: 4 },
            commitments: vec![fh("0x1")],
            pattern_is_const: [true, true, false],
            pattern_const_enc: [fh("0x1"), fh("0x2"), fh("0x0")],
            rows: vec![],
            row_count: 0,
            attribution: vec![false],
        }
    }

    fn sub(inputs: ProofInputs) -> SubProof {
        SubProof { inputs, proof_hex: String::new() }
    }

    fn base_manifest(query: &str, sub_proofs: Vec<SubProof>) -> ProofManifest {
        ProofManifest {
            r#type: "urn:sparq:zk:ProofManifest".to_string(),
            query: query.to_string(),
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
            sub_proofs,
            binding_edges: vec![],
            join_edges: vec![],
            hidden_revocation: None,
            hidden_issuer_attestations: vec![],
            holder_pok_proofs: vec![],
            holder_set_proofs: vec![],
            fully_hidden_revocation: None,
        }
    }

    fn fm(query: &str, sub_proofs: Vec<SubProof>, witnesses: Vec<BranchWitness>) -> FragmentManifest {
        FragmentManifest::new(base_manifest(query, sub_proofs), witnesses)
    }

    const PLUS: &str = "SELECT * WHERE { <http://ex/a> <http://ex/p>+ ?o }";
    const STAR: &str = "SELECT * WHERE { <http://ex/a> <http://ex/p>* ?o }";
    const OPT: &str = "SELECT * WHERE { <http://ex/a> <http://ex/p>? ?o }";
    const UNION: &str =
        "SELECT * WHERE { { ?s <http://ex/p> ?o } UNION { ?o <http://ex/q> ?s } }";
    const VALUES: &str =
        "SELECT * WHERE { ?s <http://ex/p> ?o VALUES ?o { <http://ex/x> <http://ex/y> } }";

    fn bw(branch: usize, scan: Vec<usize>, path: Vec<usize>, values: Vec<usize>) -> BranchWitness {
        BranchWitness {
            branch,
            scan_proofs: scan,
            path_proofs: path,
            values_rows: values,
            scan_rows: vec![],
            solution: vec![],
        }
    }

    // --- ACCEPT paths (a bound path member of the right member routes) -------

    #[test]
    fn accepts_bounded_plus_path_with_a_bound_member() {
        // `p+` => allow_zero false; a d=4,k=1,n=16 member is a valid bound.
        let m = fm(PLUS, vec![sub(path_ok(4, 1, 16, false))], vec![bw(0, vec![], vec![0], vec![])]);
        assert_eq!(dispatch_fragment(&m), Ok(()));
    }

    #[test]
    fn accepts_star_path_zero_length_admitted() {
        // `p*` => allow_zero true.
        let m = fm(STAR, vec![sub(path_ok(4, 1, 16, true))], vec![bw(0, vec![], vec![0], vec![])]);
        assert_eq!(dispatch_fragment(&m), Ok(()));
    }

    #[test]
    fn accepts_union_with_per_branch_solution_attribution() {
        // Two solutions, one per branch; each branch is a single BGP scan.
        let m = fm(
            UNION,
            vec![sub(scan_min()), sub(scan_min())],
            vec![bw(0, vec![0], vec![], vec![]), bw(1, vec![1], vec![], vec![])],
        );
        assert_eq!(dispatch_fragment(&m), Ok(()));
    }

    #[test]
    fn accepts_values_row_in_range() {
        // One BGP scan + a VALUES block of 2 rows; the solution picks row 1.
        let m = fm(
            VALUES,
            vec![sub(scan_min())],
            vec![bw(0, vec![0], vec![], vec![1])],
        );
        assert_eq!(dispatch_fragment(&m), Ok(()));
    }

    #[test]
    fn empty_branch_witnesses_is_a_noop_on_a_fragment_query() {
        // No attribution => the gate only re-derives the fragment + checks the
        // sub-proof ids; a stage-1-style presentation is a pass-through.
        let m = fm(PLUS, vec![], vec![]);
        assert_eq!(dispatch_fragment(&m), Ok(()));
    }

    // --- ACCEPTANCE NEGATIVE 1: PathReach claimed without a bound sub-proof ---

    #[test]
    fn rejects_path_obligation_pointing_at_a_scan_proof() {
        // The path obligation names sub-proof 0, which is a SCAN (not a bound
        // path_reach) => PathReachMissing (fail-closed).
        let m = fm(PLUS, vec![sub(scan_min())], vec![bw(0, vec![], vec![0], vec![])]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::PathReachMissing { witness: 0, obligation: 0, proof: 0 })
        ));
    }

    #[test]
    fn rejects_path_obligation_naming_an_out_of_range_proof() {
        // No sub-proofs at all, but the path obligation names index 0 => dangling.
        let m = fm(PLUS, vec![], vec![bw(0, vec![], vec![0], vec![])]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::DanglingProof { witness: 0, proof: 0 })
        ));
    }

    // --- ACCEPTANCE NEGATIVE 2: k / member mismatch between claim and circuit --

    #[test]
    fn rejects_path_member_k_mismatch() {
        // Declared id says k=2 but only ONE commitment is carried => the id
        // re-derives PathReach{4,1,16} != the declared {4,2,16} => CircuitIdMismatch.
        let inputs = ProofInputs::PathReach {
            id: CircuitId::PathReach { d: 4, k: 2, n: 16 },
            commitments: vec![fh("0x1")], // len 1, not 2
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero: false,
            depth_bound: 4,
            attribution: vec![true],
        };
        let m = fm(PLUS, vec![sub(inputs)], vec![]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::CircuitIdMismatch {
                proof: 0,
                declared: CircuitId::PathReach { d: 4, k: 2, n: 16 },
                derived: CircuitId::PathReach { d: 4, k: 1, n: 16 },
            })
        ));
    }

    #[test]
    fn rejects_path_depth_bound_not_a_compiled_member() {
        // depth_bound (5) != the member's d (4) => derive_id returns None =>
        // UnknownCircuit (the depth-overflow / mismatch rejection, req 1).
        let inputs = ProofInputs::PathReach {
            id: CircuitId::PathReach { d: 4, k: 1, n: 16 },
            commitments: vec![fh("0x1")],
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero: false,
            depth_bound: 5, // != d
            attribution: vec![true],
        };
        let m = fm(PLUS, vec![sub(inputs)], vec![]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::UnknownCircuit { proof: 0 })
        ));
    }

    // --- ACCEPTANCE NEGATIVE 3: branch attribution pointing at the wrong branch -

    #[test]
    fn rejects_branch_attribution_out_of_range() {
        // The UNION query has 2 branches; attributing a solution to branch 5 =>
        // BranchOutOfRange (the "wrong branch" rejection).
        let m = fm(UNION, vec![sub(scan_min())], vec![bw(5, vec![0], vec![], vec![])]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::BranchOutOfRange { witness: 0, branch: 5, branches: 2 })
        ));
    }

    #[test]
    fn rejects_wrong_branch_obligation_shape() {
        // Attributing to branch 0 (a single BGP scan) but naming a PATH proof for
        // it => the branch has 0 path obligations, so the path arity mismatches.
        let m = fm(
            UNION,
            vec![sub(path_ok(4, 1, 16, false))],
            vec![bw(0, vec![], vec![0], vec![])],
        );
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::ObligationArityMismatch {
                witness: 0,
                what: "scan",
                expected: 1,
                got: 0
            })
        ));
    }

    // --- closure / allow_zero + fixed-depth + VALUES range rejections ---------

    #[test]
    fn rejects_plus_presented_as_star() {
        // A `p+` obligation (allow_zero expected false) bound to a path proof with
        // allow_zero = true => PathClosureMismatch.
        let m = fm(PLUS, vec![sub(path_ok(4, 1, 16, true))], vec![bw(0, vec![], vec![0], vec![])]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::PathClosureMismatch { witness: 0, obligation: 0 })
        ));
    }

    #[test]
    fn rejects_zero_or_one_bound_to_a_deeper_member() {
        // `p?` pins the depth bound to 1, but the smallest compiled member is d=2:
        // binding a d=4 member (allow_zero true, matching the ? closure) still
        // rejects because d != the fixed bound => PathDepthExceedsClosure.
        let m = fm(OPT, vec![sub(path_ok(4, 1, 16, true))], vec![bw(0, vec![], vec![0], vec![])]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::PathDepthExceedsClosure {
                witness: 0,
                obligation: 0,
                member_d: 4,
                fixed: 1
            })
        ));
    }

    #[test]
    fn rejects_scan_obligation_pointing_at_a_path_proof() {
        // The VALUES query's single BGP obligation names sub-proof 0, which is a
        // PATH proof (not a scan) => NotAScanProof.
        let m = fm(
            VALUES,
            vec![sub(path_ok(4, 1, 16, false))],
            vec![bw(0, vec![0], vec![], vec![0])],
        );
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::NotAScanProof { witness: 0, obligation: 0, proof: 0 })
        ));
    }

    #[test]
    fn rejects_values_row_out_of_range() {
        // The VALUES block has 2 rows (indices 0,1); claiming row 2 => out of range.
        let m = fm(VALUES, vec![sub(scan_min())], vec![bw(0, vec![0], vec![], vec![2])]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::ValuesRowOutOfRange {
                witness: 0,
                block: 0,
                row: 2,
                rows: 2
            })
        ));
    }

    #[test]
    fn rejects_query_outside_the_fragment() {
        // OPTIONAL is non-monotone => outside the wave-1 fragment, fail-closed.
        let m = fm(
            "SELECT * WHERE { ?s <http://ex/p> ?o OPTIONAL { ?o <http://ex/q> ?x } }",
            vec![sub(scan_min())],
            vec![],
        );
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::OutsideFragment(_))
        ));
    }

    #[test]
    fn rejects_unknown_circuit_id_in_a_sub_proof() {
        // A scan id outside the compiled family (k=3 is not a SCAN_K_VALUES member)
        // => derive_id None => UnknownCircuit, independent of branch attribution.
        let inputs = ProofInputs::Scan {
            id: CircuitId::Scan { k: 3, n: 16, r: 4 },
            commitments: vec![fh("0x1"), fh("0x2"), fh("0x3")],
            pattern_is_const: [true, true, false],
            pattern_const_enc: [fh("0x1"), fh("0x2"), fh("0x0")],
            rows: vec![],
            row_count: 0,
            attribution: vec![false, false, false],
        };
        let m = fm(PLUS, vec![sub(inputs)], vec![]);
        assert!(matches!(
            dispatch_fragment(&m),
            Err(FragmentDispatchError::UnknownCircuit { proof: 0 })
        ));
    }

    #[test]
    fn reconstruct_path_reach_public_inputs_layout() {
        // Declaration order after challenge: commitments[k], pred_enc, src_enc,
        // dst_enc, allow_zero, depth_bound, attribution[k]. k=2 =>
        // 1 (challenge) + 2 (commit) + 3 (enc) + 1 (allow_zero) + 1 (depth) + 2
        // (attr) = 10 words * 32 bytes.
        let inputs = ProofInputs::PathReach {
            id: CircuitId::PathReach { d: 4, k: 2, n: 16 },
            commitments: vec![fh("0x1"), fh("0x2")],
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero: true,
            depth_bound: 4,
            attribution: vec![true, false],
        };
        let bytes = reconstruct_public_inputs(&inputs, &fh("0x2a"), 0).unwrap();
        assert_eq!(bytes.len(), 10 * 32, "1 challenge + 2 commit + 3 enc + allow_zero + depth + 2 attr");
        // allow_zero (word index 6) is 1, depth_bound (word 7) is 4.
        assert_eq!(bytes[6 * 32 + 31], 1, "allow_zero = true -> 1");
        assert_eq!(bytes[7 * 32 + 31], 4, "depth_bound word = 4");
        // attribution[0]=true (word 8), attribution[1]=false (word 9).
        assert_eq!(bytes[8 * 32 + 31], 1);
        assert_eq!(bytes[9 * 32 + 31], 0);
    }

    #[test]
    fn derive_id_path_reach_binds_k_and_depth() {
        // A consistent member re-derives itself.
        assert_eq!(
            derive_id(&path_ok(4, 1, 16, false)),
            Some(CircuitId::PathReach { d: 4, k: 1, n: 16 })
        );
        // depth_bound != d => None (the depth-overflow / mismatch rejection).
        let bad_depth = ProofInputs::PathReach {
            id: CircuitId::PathReach { d: 4, k: 1, n: 16 },
            commitments: vec![fh("0x1")],
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero: false,
            depth_bound: 7,
            attribution: vec![true],
        };
        assert_eq!(derive_id(&bad_depth), None);
    }

    #[test]
    fn error_display_is_non_empty_for_each_variant() {
        // Cheap Display coverage (the error is `pub` + `impl Error`).
        let errs = [
            FragmentDispatchError::OutsideFragment("x".into()),
            FragmentDispatchError::BranchOutOfRange { witness: 0, branch: 1, branches: 1 },
            FragmentDispatchError::ObligationArityMismatch {
                witness: 0,
                what: "path",
                expected: 1,
                got: 0,
            },
            FragmentDispatchError::DanglingProof { witness: 0, proof: 9 },
            FragmentDispatchError::UnknownCircuit { proof: 0 },
            FragmentDispatchError::CircuitIdMismatch {
                proof: 0,
                declared: CircuitId::PathReach { d: 4, k: 2, n: 16 },
                derived: CircuitId::PathReach { d: 4, k: 1, n: 16 },
            },
            FragmentDispatchError::NotAScanProof { witness: 0, obligation: 0, proof: 0 },
            FragmentDispatchError::PathReachMissing { witness: 0, obligation: 0, proof: 0 },
            FragmentDispatchError::PathClosureMismatch { witness: 0, obligation: 0 },
            FragmentDispatchError::PathDepthExceedsClosure {
                witness: 0,
                obligation: 0,
                member_d: 4,
                fixed: 1,
            },
            FragmentDispatchError::ValuesRowOutOfRange { witness: 0, block: 0, row: 2, rows: 2 },
        ];
        for e in &errs {
            assert!(!format!("{}", e).is_empty());
        }
    }

    // --- sq-h732x: end-to-end verify_fragment_manifest routing -------------

    /// A query OUTSIDE the wave-1 fragment (`OPTIONAL`).
    const OUTSIDE: &str =
        "SELECT * WHERE { ?s <http://ex/p> ?o OPTIONAL { ?o <http://ex/q> ?r } }";

    fn dummy_prover() -> CircuitProver {
        // Never invoked in these tests — every case rejects before the sub-proof
        // bb loop, so no nargo/bb toolchain is needed.
        CircuitProver::new(std::env::temp_dir())
    }

    /// The full external-input set for `verify_fragment_manifest` /
    /// `verify_manifest`, wired so the reject happens BEFORE the crypto gate.
    #[allow(clippy::type_complexity)]
    fn empty_verify_env() -> (
        CircuitProver,
        std::path::PathBuf,
        KeySet,
        RevocationPolicy,
        HolderRegistry,
        HolderBindingPolicy,
        EntailmentPolicy,
        VerifierNonce,
        InMemorySeenNonces,
    ) {
        (
            dummy_prover(),
            std::env::temp_dir(),
            KeySet::empty(),
            RevocationPolicy::accept_version(1),
            HolderRegistry::empty(),
            HolderBindingPolicy::allow_bearer(),
            EntailmentPolicy::simple_only(),
            VerifierNonce::from_hex("0x2a").unwrap(),
            InMemorySeenNonces::new(),
        )
    }

    #[test]
    fn verify_fragment_manifest_refuses_outside_fragment_before_bb() {
        // `dispatch_fragment` runs FIRST (before the nonce is burnt or any bb
        // subprocess starts): an OPTIONAL query fails closed as a structured
        // CheckError::FragmentDispatch, never a silent fallthrough.
        let m = fm(OUTSIDE, vec![], vec![]);
        let (p, w, ks, rp, hr, hbp, ep, n, seen) = empty_verify_env();
        let err = verify_fragment_manifest(&m, &p, &w, &ks, &rp, &hr, &hbp, &ep, &n, &seen)
            .unwrap_err();
        assert!(
            matches!(err, CheckError::FragmentDispatch(FragmentDispatchError::OutsideFragment(_))),
            "outside-fragment must fail-closed as FragmentDispatch, got {err:?}"
        );
    }

    #[test]
    fn verify_fragment_manifest_routes_union_past_stage1_recheck() {
        // The load-bearing end-to-end routing invariant: a UNION query that the
        // FLAT `verify_manifest` rejects at its stage-1 `recheck` now routes PAST
        // stage-1 through the fragment path and reaches the crypto/attestation
        // gate. With an EMPTY trust anchor the scan commitments are unattested, so
        // it fails at the issuer gate (`UnattestedCommitment`) — NOT at a stage-1
        // fragment rejection. No bb: both reject before the sub-proof loop.
        // Each branch's BGP scan answers its pattern with a row consistent with the
        // disclosed solution (so the sq-qyfth scan-slot binding also passes).
        let sc0 = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s0"), iri("http://ex/p"), iri("http://ex/o0")]],
        );
        let sc1 = scan_real(
            [None, Some("http://ex/q"), None],
            vec![[iri("http://ex/o1"), iri("http://ex/q"), iri("http://ex/s1")]],
        );
        let m = fm(
            UNION,
            vec![sub(sc0), sub(sc1)],
            vec![
                bw_scan(0, vec![0], vec![0], vec![], sol(&[("s", "http://ex/s0"), ("o", "http://ex/o0")])),
                bw_scan(1, vec![1], vec![0], vec![], sol(&[("o", "http://ex/o1"), ("s", "http://ex/s1")])),
            ],
        );
        // Fragment path: routes past stage-1a; fails at the (empty-K) issuer gate.
        let (p, w, ks, rp, hr, hbp, ep, n, seen) = empty_verify_env();
        let frag_err =
            verify_fragment_manifest(&m, &p, &w, &ks, &rp, &hr, &hbp, &ep, &n, &seen).unwrap_err();
        assert!(
            matches!(frag_err, CheckError::UnattestedCommitment { .. }),
            "fragment path must route past stage-1 into the attestation gate, got {frag_err:?}"
        );
        // NON-REGRESSION: the FLAT entry point still rejects the same query at
        // stage-1 with the same structured error as before this bead.
        let (p2, w2, ks2, rp2, hr2, hbp2, ep2, n2, seen2) = empty_verify_env();
        let flat_err = verify_manifest(
            &m.manifest, &p2, &w2, &ks2, &rp2, &hr2, &hbp2, &ep2, &n2, &seen2,
        )
        .unwrap_err();
        assert!(
            matches!(flat_err, CheckError::Sparqzk(VerifyError::UnsupportedFragment(_))),
            "flat verify_manifest must still reject UNION at stage-1, got {flat_err:?}"
        );
    }

    #[test]
    fn prefilter_impl_routes_union_where_flat_rejects() {
        // Direct unit test of the mode-aware stage-1a routing (no scans): the flat
        // regime rejects UNION at `recheck`; the extended regime routes the
        // acceptance through `fragment_query` and passes (obligations deferred).
        let m = base_manifest(UNION, vec![]);
        let flat = prefilter_manifest_structure_impl(
            &m,
            &KeySet::empty(),
            &RevocationPolicy::accept_version(1),
            false,
        );
        assert!(
            matches!(flat, Err(CheckError::Sparqzk(VerifyError::UnsupportedFragment(_)))),
            "flat regime rejects UNION at stage-1a, got {flat:?}"
        );
        let ext = prefilter_manifest_structure_impl(
            &m,
            &KeySet::empty(),
            &RevocationPolicy::accept_version(1),
            true,
        );
        assert!(
            matches!(ext, Ok(ref v) if v.is_empty()),
            "extended regime routes the UNION acceptance through fragment_query, got {ext:?}"
        );
    }

    #[test]
    fn prefilter_impl_extended_regime_still_enforces_id_hygiene() {
        // Stage-1b (id hygiene) runs in BOTH regimes: a PathReach whose declared
        // id (k=2) disagrees with the id re-derived from its single commitment
        // (k=1) is rejected fail-closed even in the extended regime.
        let bad = ProofInputs::PathReach {
            id: CircuitId::PathReach { d: 4, k: 2, n: 16 },
            commitments: vec![fh("0x1")],
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero: false,
            depth_bound: 4,
            attribution: vec![true],
        };
        let m = base_manifest(PLUS, vec![sub(bad)]);
        let err = prefilter_manifest_structure_impl(
            &m,
            &KeySet::empty(),
            &RevocationPolicy::accept_version(1),
            true,
        )
        .unwrap_err();
        assert!(
            matches!(err, CheckError::CircuitIdMismatch { proof: 0, .. }),
            "extended regime must still run stage-1b id hygiene, got {err:?}"
        );
    }

    // --- sq-1zf94: DISCLOSED-SOLUTION term binding (bind_fragment_solution) ----
    //
    // Non-vacuous, no nargo/bb: the disclosed path pred/endpoints + VALUES cells
    // are bound to encodings the verifier re-derives from the query text + the
    // disclosed solution. A mismatch REFUSES before the sub-proof loop; a
    // consistent solution reaches the SAME downstream gate as the #1665 routing.

    use crate::manifest::{DisclosedTerm, SolutionBinding};

    /// A path with variable endpoints (both projected).
    const VPATH: &str = "SELECT * WHERE { ?s <http://ex/p>+ ?o }";

    fn iri(s: &str) -> oxrdf::Term {
        oxrdf::Term::NamedNode(oxrdf::NamedNode::new(s).unwrap())
    }

    /// The canonical `FieldHex` encoding of a term (salt 0 — IRIs/literals are
    /// salt-independent), exactly as the verifier re-derives it.
    fn enc_hex(t: &oxrdf::Term) -> FieldHex {
        FieldHex(field_to_hex(&encode_term(t, &Fr::from(0u64)).unwrap()))
    }

    /// A `PathReach` input with REAL pred/src/dst encodings (over IRIs).
    fn path_real(d: u32, k: u32, n: u32, allow_zero: bool, pred: &str, src: &str, dst: &str) -> ProofInputs {
        ProofInputs::PathReach {
            id: CircuitId::PathReach { d, k, n },
            commitments: (0..k).map(|i| fh(&format!("0x{:x}", i + 1))).collect(),
            pred_enc: enc_hex(&iri(pred)),
            src_enc: enc_hex(&iri(src)),
            dst_enc: enc_hex(&iri(dst)),
            allow_zero,
            depth_bound: d,
            attribution: vec![true; k as usize],
        }
    }

    /// Disclosed IRI solution bindings from `(var, iri)` pairs.
    fn sol(pairs: &[(&str, &str)]) -> Vec<SolutionBinding> {
        pairs
            .iter()
            .map(|(v, i)| SolutionBinding {
                var: v.to_string(),
                term: DisclosedTerm::Iri { value: i.to_string() },
            })
            .collect()
    }

    fn bw_sol(
        branch: usize,
        scan: Vec<usize>,
        path: Vec<usize>,
        values: Vec<usize>,
        solution: Vec<SolutionBinding>,
    ) -> BranchWitness {
        BranchWitness {
            branch,
            scan_proofs: scan,
            path_proofs: path,
            values_rows: values,
            scan_rows: vec![],
            solution,
        }
    }

    /// [OPUS-4.8] sq-qyfth: a branch witness that also carries a per-scan
    /// `scan_rows` row-selection (for the BGP scan-slot binding tests).
    fn bw_scan(
        branch: usize,
        scan: Vec<usize>,
        scan_rows: Vec<usize>,
        values: Vec<usize>,
        solution: Vec<SolutionBinding>,
    ) -> BranchWitness {
        BranchWitness {
            branch,
            scan_proofs: scan,
            path_proofs: vec![],
            values_rows: values,
            scan_rows,
            solution,
        }
    }

    /// [OPUS-4.8] sq-qyfth: a `Scan` input over a query BGP pattern with REAL
    /// slot encodings. `consts` gives the 3 slots as `Some(iri)` for a constant or
    /// `None` for a variable (`pattern_is_const`/`pattern_const_enc` are derived to
    /// match, so `scan_matches_pattern` accepts it); `rows` are the disclosed
    /// matched rows as `[Term; 3]` (already the right slot encodings).
    fn scan_real(consts: [Option<&str>; 3], rows: Vec<[oxrdf::Term; 3]>) -> ProofInputs {
        let z = fh("0x0");
        let is_const = [consts[0].is_some(), consts[1].is_some(), consts[2].is_some()];
        let const_enc = [
            consts[0].map(|s| enc_hex(&iri(s))).unwrap_or_else(|| z.clone()),
            consts[1].map(|s| enc_hex(&iri(s))).unwrap_or_else(|| z.clone()),
            consts[2].map(|s| enc_hex(&iri(s))).unwrap_or_else(|| z.clone()),
        ];
        let enc_rows: Vec<[FieldHex; 3]> = rows
            .iter()
            .map(|r| [enc_hex(&r[0]), enc_hex(&r[1]), enc_hex(&r[2])])
            .collect();
        let row_count = enc_rows.len() as u32;
        // Derive the id EXACTLY as `derive_id` re-derives it (so the dispatch
        // id-hygiene check passes end-to-end): k=1, n=16 bucket, r bucketed from the
        // active row count.
        let id = crate::build::derive_scan_id(1, 16, row_count).expect("scan id in family");
        ProofInputs::Scan {
            id,
            commitments: vec![fh("0x1")],
            pattern_is_const: is_const,
            pattern_const_enc: const_enc,
            rows: enc_rows,
            row_count,
            attribution: vec![true],
        }
    }

    #[test]
    fn bind_solution_accepts_a_consistent_constant_source_path() {
        // PLUS: subject <http://ex/a> (constant), object ?o (projected) = <b>.
        // pred_enc = enc(<p>), src_enc = enc(<a>), dst_enc = enc(<b>) all consistent.
        let m = fm(
            PLUS,
            vec![sub(path_real(4, 1, 16, false, "http://ex/p", "http://ex/a", "http://ex/b"))],
            vec![bw_sol(0, vec![], vec![0], vec![], sol(&[("o", "http://ex/b")]))],
        );
        assert_eq!(bind_fragment_solution(&m), Ok(()));
    }

    #[test]
    fn bind_solution_rejects_a_wrong_path_predicate() {
        // pred_enc encodes <q>, but the query names <p> => PathPredMismatch.
        let m = fm(
            PLUS,
            vec![sub(path_real(4, 1, 16, false, "http://ex/q", "http://ex/a", "http://ex/b"))],
            vec![bw_sol(0, vec![], vec![0], vec![], sol(&[("o", "http://ex/b")]))],
        );
        assert_eq!(
            bind_fragment_solution(&m),
            Err(FragmentSolutionError::PathPredMismatch { witness: 0, obligation: 0 })
        );
    }

    #[test]
    fn bind_solution_rejects_src_enc_not_matching_the_disclosed_solution() {
        // VPATH: subject ?s (projected) disclosed = <a>, but src_enc encodes <zzz>
        // => PathEndpointMismatch{Src}. This is the load-bearing sq-1zf94 negative.
        let m = fm(
            VPATH,
            vec![sub(path_real(4, 1, 16, false, "http://ex/p", "http://ex/zzz", "http://ex/b"))],
            vec![bw_sol(0, vec![], vec![0], vec![], sol(&[("s", "http://ex/a"), ("o", "http://ex/b")]))],
        );
        assert_eq!(
            bind_fragment_solution(&m),
            Err(FragmentSolutionError::PathEndpointMismatch {
                witness: 0,
                obligation: 0,
                endpoint: PathEndpoint::Src,
            })
        );
    }

    #[test]
    fn bind_solution_rejects_dst_enc_not_matching_the_disclosed_solution() {
        // PLUS: object ?o disclosed = <b>, but dst_enc encodes <other> => Dst mismatch.
        let m = fm(
            PLUS,
            vec![sub(path_real(4, 1, 16, false, "http://ex/p", "http://ex/a", "http://ex/other"))],
            vec![bw_sol(0, vec![], vec![0], vec![], sol(&[("o", "http://ex/b")]))],
        );
        assert_eq!(
            bind_fragment_solution(&m),
            Err(FragmentSolutionError::PathEndpointMismatch {
                witness: 0,
                obligation: 0,
                endpoint: PathEndpoint::Dst,
            })
        );
    }

    #[test]
    fn bind_solution_rejects_a_projected_endpoint_omitted_from_the_solution() {
        // VPATH: ?s is projected but the solution omits it => UnboundProjectedEndpoint.
        let m = fm(
            VPATH,
            vec![sub(path_real(4, 1, 16, false, "http://ex/p", "http://ex/a", "http://ex/b"))],
            vec![bw_sol(0, vec![], vec![0], vec![], sol(&[("o", "http://ex/b")]))],
        );
        assert_eq!(
            bind_fragment_solution(&m),
            Err(FragmentSolutionError::UnboundProjectedEndpoint {
                witness: 0,
                obligation: 0,
                endpoint: PathEndpoint::Src,
                var: "s".to_string(),
            })
        );
    }

    #[test]
    fn bind_solution_accepts_a_values_row_consistent_with_the_solution() {
        // VALUES ?o { <x> <y> }; row 1 = <y>, solution ?o = <y> => consistent.
        let m = fm(
            VALUES,
            vec![sub(scan_min())],
            vec![bw_sol(0, vec![0], vec![], vec![1], sol(&[("o", "http://ex/y")]))],
        );
        assert_eq!(bind_fragment_solution(&m), Ok(()));
    }

    #[test]
    fn bind_solution_rejects_a_values_cell_mismatch() {
        // Row 1 = <y>, but the disclosed solution claims ?o = <z> => ValuesCellMismatch.
        let m = fm(
            VALUES,
            vec![sub(scan_min())],
            vec![bw_sol(0, vec![0], vec![], vec![1], sol(&[("o", "http://ex/z")]))],
        );
        assert_eq!(
            bind_fragment_solution(&m),
            Err(FragmentSolutionError::ValuesCellMismatch {
                witness: 0,
                block: 0,
                column: 0,
                var: "o".to_string(),
            })
        );
    }

    #[test]
    fn bind_solution_rejects_pointing_at_the_wrong_disclosed_row() {
        // The solution ?o = <y>, but values_rows picks row 0 (<x>) => the "wrong
        // disclosed row" rejection (ValuesCellMismatch).
        let m = fm(
            VALUES,
            vec![sub(scan_min())],
            vec![bw_sol(0, vec![0], vec![], vec![0], sol(&[("o", "http://ex/y")]))],
        );
        assert_eq!(
            bind_fragment_solution(&m),
            Err(FragmentSolutionError::ValuesCellMismatch {
                witness: 0,
                block: 0,
                column: 0,
                var: "o".to_string(),
            })
        );
    }

    #[test]
    fn bind_solution_rejects_a_projected_values_var_with_no_disclosure() {
        // ?o is a projected VALUES variable but the solution is empty => fail-closed.
        let m = fm(VALUES, vec![sub(scan_min())], vec![bw_sol(0, vec![0], vec![], vec![1], vec![])]);
        assert_eq!(
            bind_fragment_solution(&m),
            Err(FragmentSolutionError::UnboundProjectedValuesVar {
                witness: 0,
                block: 0,
                column: 0,
                var: "o".to_string(),
            })
        );
    }

    #[test]
    fn bind_solution_rejects_a_malformed_disclosed_term() {
        // A disclosed IRI that does not parse => MalformedSolutionTerm (fail-closed).
        let m = fm(
            VPATH,
            vec![sub(path_real(4, 1, 16, false, "http://ex/p", "http://ex/a", "http://ex/b"))],
            vec![bw_sol(0, vec![], vec![0], vec![], sol(&[("s", "not an iri"), ("o", "http://ex/b")]))],
        );
        assert_eq!(
            bind_fragment_solution(&m),
            Err(FragmentSolutionError::MalformedSolutionTerm { witness: 0, var: "s".to_string() })
        );
    }

    #[test]
    fn bind_solution_does_not_bind_an_existential_endpoint() {
        // ASK projects nothing, so both endpoints are existential (hidden) — the
        // src/dst encodings are NOT term-bound, only the query-constant predicate is.
        // A garbage src/dst therefore still passes the disclosed-solution gate.
        let ask = "ASK { ?s <http://ex/p>+ ?o }";
        let m = fm(
            ask,
            vec![sub(path_real(4, 1, 16, false, "http://ex/p", "http://ex/whatever", "http://ex/junk"))],
            vec![bw_sol(0, vec![], vec![0], vec![], vec![])],
        );
        assert_eq!(bind_fragment_solution(&m), Ok(()));
    }

    #[test]
    fn verify_fragment_manifest_refuses_inconsistent_solution_before_bb() {
        // End-to-end: a VALUES cell that disagrees with the disclosed solution is
        // refused as a structured CheckError::FragmentSolution — BEFORE the nonce is
        // burnt or any bb subprocess runs.
        let m = fm(
            VALUES,
            vec![sub(scan_min())],
            vec![bw_sol(0, vec![0], vec![], vec![1], sol(&[("o", "http://ex/z")]))],
        );
        let (p, w, ks, rp, hr, hbp, ep, n, seen) = empty_verify_env();
        let err = verify_fragment_manifest(&m, &p, &w, &ks, &rp, &hr, &hbp, &ep, &n, &seen)
            .unwrap_err();
        assert!(
            matches!(
                err,
                CheckError::FragmentSolution(FragmentSolutionError::ValuesCellMismatch { .. })
            ),
            "an inconsistent disclosed solution must fail-closed as FragmentSolution, got {err:?}"
        );
    }

    #[test]
    fn verify_fragment_manifest_consistent_solution_reaches_the_crypto_gate() {
        // The accept path: a CONSISTENT disclosed solution passes the routing, the
        // disclosed-solution binding AND the sq-qyfth BGP scan-slot binding, then
        // reaches the SAME downstream crypto/attestation gate as the #1665 routing
        // test (empty K => UnattestedCommitment) — NOT a fragment-stage rejection.
        // No bb. The VALUES query's BGP pattern `?s <p> ?o` is answered by a scan
        // whose row (?s=<s1>, ?o=<y>) matches the disclosed solution; ?o=<y> also
        // matches VALUES row 1.
        let sc = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/y")]],
        );
        let m = fm(
            VALUES,
            vec![sub(sc)],
            vec![bw_scan(
                0,
                vec![0],
                vec![0],
                vec![1],
                sol(&[("s", "http://ex/s1"), ("o", "http://ex/y")]),
            )],
        );
        let (p, w, ks, rp, hr, hbp, ep, n, seen) = empty_verify_env();
        let err = verify_fragment_manifest(&m, &p, &w, &ks, &rp, &hr, &hbp, &ep, &n, &seen)
            .unwrap_err();
        assert!(
            matches!(err, CheckError::UnattestedCommitment { .. }),
            "a consistent solution must route past the bindings into the attestation gate, got {err:?}"
        );
    }

    #[test]
    fn fragment_solution_error_display_is_non_empty_for_each_variant() {
        // Cheap Display coverage (the error is `pub` + `impl Error`).
        let errs = [
            FragmentSolutionError::MalformedSolutionTerm { witness: 0, var: "s".into() },
            FragmentSolutionError::PathPredMismatch { witness: 0, obligation: 0 },
            FragmentSolutionError::PathEndpointMismatch {
                witness: 0,
                obligation: 0,
                endpoint: PathEndpoint::Src,
            },
            FragmentSolutionError::UnboundProjectedEndpoint {
                witness: 0,
                obligation: 0,
                endpoint: PathEndpoint::Dst,
                var: "o".into(),
            },
            FragmentSolutionError::ValuesCellMismatch {
                witness: 0,
                block: 0,
                column: 0,
                var: "o".into(),
            },
            FragmentSolutionError::UnboundProjectedValuesVar {
                witness: 0,
                block: 0,
                column: 0,
                var: "o".into(),
            },
            FragmentSolutionError::WildcardEndpoint {
                witness: 0,
                obligation: 0,
                endpoint: PathEndpoint::Src,
            },
            FragmentSolutionError::Structure { witness: 0, what: "branch" },
        ];
        for e in &errs {
            assert!(!format!("{}", e).is_empty());
        }
    }

    // --- sq-qyfth: BGP SCAN-SLOT binding (bind_fragment_scans) -----------------
    //
    // Non-vacuous, no nargo/bb: a disclosed solution variable occurring in a BGP
    // scan is bound to the SELECTED disclosed row's slot value (re-derived from the
    // query text + the disclosed solution). A wrong / out-of-range / missing /
    // join-incoherent row selection REFUSES before the sub-proof loop; a consistent
    // selection reaches the same downstream gate as the #1673 accept path.

    /// A single-scan BGP (both endpoints projected).
    const SCANQ: &str = "SELECT * WHERE { ?s <http://ex/p> ?o }";
    /// A two-scan BGP joined on an EXISTENTIAL variable ?x (projected: ?s ?o).
    const JOINQ: &str =
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?x . ?x <http://ex/q> ?o }";

    /// A scan answering `?s <http://ex/p> ?o` with one row `(<s1>, <p>, <o1>)`.
    fn scan_sp_o(o: &str) -> ProofInputs {
        scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri(o)]],
        )
    }

    #[test]
    fn bind_scans_accepts_a_row_consistent_with_the_disclosed_solution() {
        let m = fm(
            SCANQ,
            vec![sub(scan_sp_o("http://ex/o1"))],
            vec![bw_scan(0, vec![0], vec![0], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(bind_fragment_scans(&m), Ok(()));
    }

    #[test]
    fn bind_scans_rejects_a_slot_not_matching_the_disclosed_term() {
        // The row's object slot is <other>, but the solution claims ?o=<o1>.
        let m = fm(
            SCANQ,
            vec![sub(scan_sp_o("http://ex/other"))],
            vec![bw_scan(0, vec![0], vec![0], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::ScanSlotMismatch { witness: 0, obligation: 0, slot: 2, var: "o".to_string() })
        );
    }

    #[test]
    fn bind_scans_rejects_pointing_at_the_wrong_disclosed_row() {
        // Two rows; the solution matches row 0, but scan_rows selects row 1.
        let sc = scan_real(
            [None, Some("http://ex/p"), None],
            vec![
                [iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/o1")],
                [iri("http://ex/s2"), iri("http://ex/p"), iri("http://ex/o2")],
            ],
        );
        let m = fm(
            SCANQ,
            vec![sub(sc)],
            vec![bw_scan(0, vec![0], vec![1], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        // Row 1 slot 0 (?s = <s2>) disagrees with the disclosed ?s = <s1>.
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::ScanSlotMismatch { witness: 0, obligation: 0, slot: 0, var: "s".to_string() })
        );
    }

    #[test]
    fn bind_scans_rejects_an_out_of_range_row() {
        let m = fm(
            SCANQ,
            vec![sub(scan_sp_o("http://ex/o1"))],
            vec![bw_scan(0, vec![0], vec![3], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::RowOutOfRange { witness: 0, obligation: 0, row: 3, active: 1 })
        );
    }

    #[test]
    fn bind_scans_rejects_a_missing_row_selection_for_a_variable_scan() {
        // The pattern has variables ?s ?o but no scan_rows entry => MissingRowSelection.
        let m = fm(
            SCANQ,
            vec![sub(scan_sp_o("http://ex/o1"))],
            vec![bw_scan(0, vec![0], vec![], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::MissingRowSelection { witness: 0, obligation: 0 })
        );
    }

    #[test]
    fn bind_scans_rejects_a_projected_scan_var_omitted_from_the_solution() {
        // ?s is projected but the solution omits it => UnboundProjectedScanVar.
        let m = fm(
            SCANQ,
            vec![sub(scan_sp_o("http://ex/o1"))],
            vec![bw_scan(0, vec![0], vec![0], vec![], sol(&[("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::UnboundProjectedScanVar { witness: 0, obligation: 0, slot: 0, var: "s".to_string() })
        );
    }

    #[test]
    fn bind_scans_rejects_a_scan_over_the_wrong_predicate() {
        // The scan answers <q>, but the query pattern names <p> => ScanPatternMismatch.
        let sc = scan_real(
            [None, Some("http://ex/q"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/q"), iri("http://ex/o1")]],
        );
        let m = fm(
            SCANQ,
            vec![sub(sc)],
            vec![bw_scan(0, vec![0], vec![0], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::ScanPatternMismatch { witness: 0, obligation: 0 })
        );
    }

    #[test]
    fn bind_scans_rejects_a_non_scan_proof_for_a_bgp_obligation() {
        // The BGP obligation names sub-proof 0, which is a PATH proof.
        let m = fm(
            SCANQ,
            vec![sub(path_ok(4, 1, 16, false))],
            vec![bw_scan(0, vec![0], vec![0], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::NotAScanProof { witness: 0, obligation: 0, proof: 0 })
        );
    }

    #[test]
    fn bind_scans_rejects_a_malformed_disclosed_term() {
        let m = fm(
            SCANQ,
            vec![sub(scan_sp_o("http://ex/o1"))],
            vec![bw_scan(0, vec![0], vec![0], vec![], sol(&[("s", "not an iri"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::MalformedSolutionTerm { witness: 0, var: "s".to_string() })
        );
    }

    #[test]
    fn bind_scans_accepts_a_coherent_existential_join() {
        // ?x existential, shared across both atoms; the two selected rows agree on it.
        let sc0 = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
        );
        let sc1 = scan_real(
            [None, Some("http://ex/q"), None],
            vec![[iri("http://ex/x1"), iri("http://ex/q"), iri("http://ex/o1")]],
        );
        let m = fm(
            JOINQ,
            vec![sub(sc0), sub(sc1)],
            vec![bw_scan(0, vec![0, 1], vec![0, 0], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(bind_fragment_scans(&m), Ok(()));
    }

    #[test]
    fn bind_scans_rejects_an_incoherent_existential_join() {
        // The two rows disagree on the shared existential ?x (x1 vs x2) => JoinIncoherent.
        let sc0 = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
        );
        let sc1 = scan_real(
            [None, Some("http://ex/q"), None],
            vec![[iri("http://ex/x2"), iri("http://ex/q"), iri("http://ex/o1")]],
        );
        let m = fm(
            JOINQ,
            vec![sub(sc0), sub(sc1)],
            vec![bw_scan(0, vec![0, 1], vec![0, 0], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_scans(&m),
            Err(FragmentScanError::JoinIncoherent { witness: 0, obligation: 1, slot: 0, var: "x".to_string() })
        );
    }

    #[test]
    fn bind_scans_accepts_an_all_constant_pattern_with_no_row() {
        // A fully-ground BGP pattern needs no disclosed row (bound by scan_matches_pattern).
        const GROUND: &str = "ASK { <http://ex/a> <http://ex/p> <http://ex/b> }";
        let sc = scan_real(
            [Some("http://ex/a"), Some("http://ex/p"), Some("http://ex/b")],
            vec![[iri("http://ex/a"), iri("http://ex/p"), iri("http://ex/b")]],
        );
        let m = fm(GROUND, vec![sub(sc)], vec![bw_scan(0, vec![0], vec![], vec![], vec![])]);
        assert_eq!(bind_fragment_scans(&m), Ok(()));
    }

    #[test]
    fn verify_fragment_manifest_refuses_wrong_scan_row_before_bb() {
        // End-to-end: a scan row inconsistent with the disclosed solution is refused
        // as a structured CheckError::FragmentScan BEFORE the nonce is burnt or any
        // bb subprocess runs.
        let m = fm(
            SCANQ,
            vec![sub(scan_sp_o("http://ex/other"))],
            vec![bw_scan(0, vec![0], vec![0], vec![], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        let (p, w, ks, rp, hr, hbp, ep, n, seen) = empty_verify_env();
        let err = verify_fragment_manifest(&m, &p, &w, &ks, &rp, &hr, &hbp, &ep, &n, &seen)
            .unwrap_err();
        assert!(
            matches!(err, CheckError::FragmentScan(FragmentScanError::ScanSlotMismatch { .. })),
            "an inconsistent scan row must fail-closed as FragmentScan, got {err:?}"
        );
    }

    #[test]
    fn fragment_scan_error_display_is_non_empty_for_each_variant() {
        let errs = [
            FragmentScanError::MalformedSolutionTerm { witness: 0, var: "s".into() },
            FragmentScanError::NotAScanProof { witness: 0, obligation: 0, proof: 0 },
            FragmentScanError::Structure { witness: 0, what: "branch" },
            FragmentScanError::ScanPatternMismatch { witness: 0, obligation: 0 },
            FragmentScanError::MissingRowSelection { witness: 0, obligation: 0 },
            FragmentScanError::RowOutOfRange { witness: 0, obligation: 0, row: 3, active: 1 },
            FragmentScanError::UnboundProjectedScanVar { witness: 0, obligation: 0, slot: 0, var: "s".into() },
            FragmentScanError::ScanSlotMismatch { witness: 0, obligation: 0, slot: 2, var: "o".into() },
            FragmentScanError::JoinIncoherent { witness: 0, obligation: 1, slot: 0, var: "x".into() },
            FragmentScanError::WildcardSlot { witness: 0, obligation: 0, slot: 0 },
        ];
        for e in &errs {
            assert!(!format!("{}", e).is_empty());
        }
    }

    // --- sq-ygk6x: PER-BRANCH JOIN COHERENCE + cross-graph Q6 (bind_fragment_join_coherence)
    //
    // Non-vacuous, no nargo/bb: an existential variable shared between a BGP scan
    // slot and a PathReach endpoint (or two scans across graphs) is bound to ONE
    // disclosed value; a mismatch REFUSES before the sub-proof loop. A multi-graph
    // path (interior chain non-bnode not verifier-dischargeable) REFUSES. A coherent
    // solution reaches the same downstream gate as the #1678 accept path.

    /// A BGP scan joined to a `+` path on an EXISTENTIAL ?x (the path SUBJECT).
    /// Projected ?s ?o; ?x is not projected.
    const SCANPATH_SRC: &str =
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?x . ?x <http://ex/q>+ ?o }";
    /// A BGP scan joined to a `+` path on an EXISTENTIAL ?x (the path OBJECT).
    const SCANPATH_DST: &str =
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?x . ?o <http://ex/q>+ ?x }";

    /// A branch witness carrying scan proofs, path proofs, VALUES rows, per-scan row
    /// selection, and the disclosed solution together (sq-ygk6x tests).
    #[allow(clippy::too_many_arguments)]
    fn bw_full(
        branch: usize,
        scan_proofs: Vec<usize>,
        path_proofs: Vec<usize>,
        values_rows: Vec<usize>,
        scan_rows: Vec<usize>,
        solution: Vec<SolutionBinding>,
    ) -> BranchWitness {
        BranchWitness { branch, scan_proofs, path_proofs, values_rows, scan_rows, solution }
    }

    /// [`scan_real`] over a SPECIFIC committed graph (so two scans can be attributed
    /// to DISTINCT graphs for the cross-graph tests).
    fn scan_real_c(consts: [Option<&str>; 3], rows: Vec<[oxrdf::Term; 3]>, commit: &str) -> ProofInputs {
        match scan_real(consts, rows) {
            ProofInputs::Scan {
                id,
                pattern_is_const,
                pattern_const_enc,
                rows,
                row_count,
                attribution,
                ..
            } => ProofInputs::Scan {
                id,
                commitments: vec![fh(commit)],
                pattern_is_const,
                pattern_const_enc,
                rows,
                row_count,
                attribution,
            },
            _ => unreachable!("scan_real always builds a Scan"),
        }
    }

    #[test]
    fn bind_join_coherence_accepts_a_coherent_scan_path() {
        // scan row ?x = <x1>; path src_enc = enc(<x1>) agrees => coherent.
        let sc = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
        );
        let pa = path_real(4, 1, 16, false, "http://ex/q", "http://ex/x1", "http://ex/o1");
        let m = fm(
            SCANPATH_SRC,
            vec![sub(sc), sub(pa)],
            vec![bw_full(0, vec![0], vec![1], vec![], vec![0], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(bind_fragment_join_coherence(&m), Ok(()));
    }

    #[test]
    fn bind_join_coherence_rejects_scan_path_src_mismatch() {
        // scan row ?x = <x1>; path src_enc = enc(<zzz>) disagrees => Incoherent at
        // the path obligation (combined index 1 = n_scans(1) + 0).
        let sc = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
        );
        let pa = path_real(4, 1, 16, false, "http://ex/q", "http://ex/zzz", "http://ex/o1");
        let m = fm(
            SCANPATH_SRC,
            vec![sub(sc), sub(pa)],
            vec![bw_full(0, vec![0], vec![1], vec![], vec![0], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_join_coherence(&m),
            Err(FragmentJoinError::Incoherent { witness: 0, obligation: 1, var: "x".to_string() })
        );
    }

    #[test]
    fn bind_join_coherence_rejects_scan_path_dst_mismatch() {
        // ?x is the path OBJECT (dst_enc). scan ?x = <x1>; path dst_enc = enc(<zzz>)
        // => Incoherent, the OTHER direction of the scan↔path endpoint join.
        let sc = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
        );
        let pa = path_real(4, 1, 16, false, "http://ex/q", "http://ex/o1", "http://ex/zzz");
        let m = fm(
            SCANPATH_DST,
            vec![sub(sc), sub(pa)],
            vec![bw_full(0, vec![0], vec![1], vec![], vec![0], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_join_coherence(&m),
            Err(FragmentJoinError::Incoherent { witness: 0, obligation: 1, var: "x".to_string() })
        );
    }

    #[test]
    fn bind_join_coherence_rejects_a_multi_graph_path() {
        // A k=2 path (attribution [true, true]) admits a cross-graph interior chain
        // whose non-bnode obligation the verifier cannot discharge => MultiGraphPath.
        let pa = path_real(4, 2, 16, false, "http://ex/p", "http://ex/a", "http://ex/b");
        let m = fm(
            PLUS,
            vec![sub(pa)],
            vec![bw_full(0, vec![], vec![0], vec![], vec![], sol(&[("o", "http://ex/b")]))],
        );
        assert_eq!(
            bind_fragment_join_coherence(&m),
            Err(FragmentJoinError::MultiGraphPath { witness: 0, obligation: 0 })
        );
    }

    #[test]
    fn bind_join_coherence_refuses_cross_graph_disagreeing_join() {
        // Two scans over DISTINCT committed graphs (0x1, 0x2) sharing existential ?x,
        // whose selected rows DISAGREE — the encoding a blank node would take across
        // two distinctly-salted graphs. Refused (join incoherent) at obligation 1.
        let sc0 = scan_real_c(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
            "0x1",
        );
        let sc1 = scan_real_c(
            [None, Some("http://ex/q"), None],
            vec![[iri("http://ex/x2"), iri("http://ex/q"), iri("http://ex/o1")]],
            "0x2",
        );
        let m = fm(
            JOINQ,
            vec![sub(sc0), sub(sc1)],
            vec![bw_full(0, vec![0, 1], vec![], vec![], vec![0, 0], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(
            bind_fragment_join_coherence(&m),
            Err(FragmentJoinError::Incoherent { witness: 0, obligation: 1, var: "x".to_string() })
        );
    }

    #[test]
    fn bind_join_coherence_accepts_cross_graph_agreeing_join() {
        // Two scans over DISTINCT committed graphs sharing existential ?x whose rows
        // AGREE (a non-bnode IRI encodes identically across graphs) => Ok, and the
        // REQUIRED cross-graph obligation `branch_obligations` derives is covered.
        let sc0 = scan_real_c(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
            "0x1",
        );
        let sc1 = scan_real_c(
            [None, Some("http://ex/q"), None],
            vec![[iri("http://ex/x1"), iri("http://ex/q"), iri("http://ex/o1")]],
            "0x2",
        );
        let m = fm(
            JOINQ,
            vec![sub(sc0), sub(sc1)],
            vec![bw_full(0, vec![0, 1], vec![], vec![], vec![0, 0], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        assert_eq!(bind_fragment_join_coherence(&m), Ok(()));
    }

    #[test]
    fn verify_fragment_manifest_refuses_scan_path_incoherence_before_bb() {
        // End-to-end: a path endpoint inconsistent with its supporting scan row is
        // refused as CheckError::FragmentJoin BEFORE the nonce is burnt or any bb runs.
        // ?x is existential (bind_fragment_solution ignores its src); ?o projected =>
        // dst_enc = enc(o1) must match. The scan↔path ?x mismatch surfaces in layer 2c.
        let sc = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
        );
        let pa = path_real(4, 1, 16, false, "http://ex/q", "http://ex/zzz", "http://ex/o1");
        let m = fm(
            SCANPATH_SRC,
            vec![sub(sc), sub(pa)],
            vec![bw_full(0, vec![0], vec![1], vec![], vec![0], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        let (p, w, ks, rp, hr, hbp, ep, n, seen) = empty_verify_env();
        let err = verify_fragment_manifest(&m, &p, &w, &ks, &rp, &hr, &hbp, &ep, &n, &seen)
            .unwrap_err();
        assert!(
            matches!(err, CheckError::FragmentJoin(FragmentJoinError::Incoherent { .. })),
            "an incoherent scan↔path join must fail-closed as FragmentJoin, got {err:?}"
        );
    }

    #[test]
    fn verify_fragment_manifest_coherent_scan_path_reaches_the_crypto_gate() {
        // The accept path: a coherent scan↔path solution passes routing + all three
        // binding layers, then reaches the SAME downstream attestation gate (empty K
        // => UnattestedCommitment). No bb.
        let sc = scan_real(
            [None, Some("http://ex/p"), None],
            vec![[iri("http://ex/s1"), iri("http://ex/p"), iri("http://ex/x1")]],
        );
        let pa = path_real(4, 1, 16, false, "http://ex/q", "http://ex/x1", "http://ex/o1");
        let m = fm(
            SCANPATH_SRC,
            vec![sub(sc), sub(pa)],
            vec![bw_full(0, vec![0], vec![1], vec![], vec![0], sol(&[("s", "http://ex/s1"), ("o", "http://ex/o1")]))],
        );
        let (p, w, ks, rp, hr, hbp, ep, n, seen) = empty_verify_env();
        let err = verify_fragment_manifest(&m, &p, &w, &ks, &rp, &hr, &hbp, &ep, &n, &seen)
            .unwrap_err();
        assert!(
            matches!(err, CheckError::UnattestedCommitment { .. }),
            "a coherent scan↔path solution must route past the bindings into the attestation gate, got {err:?}"
        );
    }

    #[test]
    fn fragment_join_error_display_is_non_empty_for_each_variant() {
        let errs = [
            FragmentJoinError::Structure { witness: 0, what: "branch" },
            FragmentJoinError::MalformedField { witness: 0, obligation: 1, what: "path src_enc" },
            FragmentJoinError::Incoherent { witness: 0, obligation: 1, var: "x".into() },
            FragmentJoinError::MultiGraphPath { witness: 0, obligation: 0 },
            FragmentJoinError::UncoveredCrossGraphJoin { witness: 0, variable: "x".into() },
        ];
        for e in &errs {
            assert!(!format!("{}", e).is_empty());
        }
    }

    // --- sq-nlulr: issuer-attestation + salt-uniqueness (audit #9) over PATH
    // commitments (bind_issuer_attestations). Non-vacuous, no nargo/bb: these call
    // the REAL issuer gate directly over a manifest carrying a `PathReach`
    // sub-proof — the same gate `verify_manifest`/`verify_fragment_manifest` run.

    /// A single-graph `PathReach` sub-proof over a CHOSEN committed graph (so the
    /// attestation / salt tests can attest or collide specific commitments; the
    /// gate verifies the signature over the commitment value, never recomputes it).
    fn path_with_commit(commit: Fr) -> ProofInputs {
        ProofInputs::PathReach {
            id: CircuitId::PathReach { d: 4, k: 1, n: 16 },
            commitments: vec![FieldHex::from_field(&commit)],
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero: false,
            depth_bound: 4,
            attribution: vec![true],
        }
    }

    /// A single-graph BGP `Scan` sub-proof over a CHOSEN committed graph.
    fn scan_with_commit(commit: Fr) -> ProofInputs {
        ProofInputs::Scan {
            id: CircuitId::Scan { k: 1, n: 16, r: 4 },
            commitments: vec![FieldHex::from_field(&commit)],
            pattern_is_const: [true, true, false],
            pattern_const_enc: [fh("0x1"), fh("0x2"), fh("0x0")],
            rows: vec![],
            row_count: 0,
            attribution: vec![false],
        }
    }

    #[test]
    fn bind_issuer_attestations_refuses_an_unattested_path_commitment() {
        // A PathReach commitment with NO issuer attestation is refused on the SAME
        // footing as an unattested scan commitment. Before sq-nlulr the issuer gate
        // silently skipped path sub-proofs, so this passed unattested.
        let sk = sparq_zk::sig::SecretKey::from_seed(1);
        let k = KeySet::from_hex_keys([sparq_zk::sig::public_key_to_hex(&sk.public_key())]);
        let mut m = base_manifest(PLUS, vec![sub(path_with_commit(Fr::from(100u64)))]);
        m.revocation = Some(test_revocation());
        let err = bind_issuer_attestations(&m, &k, &std::collections::BTreeSet::new())
            .unwrap_err();
        assert!(
            matches!(err, CheckError::UnattestedCommitment { proof: 0, .. }),
            "an unattested PATH commitment must be refused at the issuer gate, got {err:?}"
        );
    }

    #[test]
    fn bind_issuer_attestations_refuses_a_scan_path_salt_collision() {
        // A scan graph and a single-graph PATH graph, distinctly committed but
        // SHARING a salt, are the audit-#9 cross-graph bnode-correlation channel.
        // Both attestations verify, so the gate reaches the salt-uniqueness step and
        // REFUSES SaltReused — the load-bearing sq-nlulr corollary.
        let sk = sparq_zk::sig::SecretKey::from_seed(1);
        let k = KeySet::from_hex_keys([sparq_zk::sig::public_key_to_hex(&sk.public_key())]);
        let salt = Fr::from(7u64);
        let c_scan = Fr::from(100u64);
        let c_path = Fr::from(200u64); // distinct commitment, SAME salt.
        let mut m = base_manifest(
            SCANPATH_SRC,
            vec![sub(scan_with_commit(c_scan)), sub(path_with_commit(c_path))],
        );
        m.commitment_attestations =
            vec![test_attestation(c_scan, salt, &sk), test_attestation(c_path, salt, &sk)];
        m.revocation = Some(test_revocation());
        let err = bind_issuer_attestations(&m, &k, &std::collections::BTreeSet::new())
            .unwrap_err();
        assert!(
            matches!(err, CheckError::SaltReused { .. }),
            "a scan-graph <-> single-graph-path-graph salt collision must be refused (audit #9), got {err:?}"
        );
    }

    #[test]
    fn bind_issuer_attestations_accepts_an_attested_distinctly_salted_scan_path() {
        // With DISTINCT salts the attested scan AND path both record their salt and
        // the gate passes to the downstream stage — proving the path commitment is
        // attested + salt-recorded (not merely ignored) and only COLLISIONS refuse.
        let sk = sparq_zk::sig::SecretKey::from_seed(1);
        let k = KeySet::from_hex_keys([sparq_zk::sig::public_key_to_hex(&sk.public_key())]);
        let c_scan = Fr::from(100u64);
        let c_path = Fr::from(200u64);
        let mut m = base_manifest(
            SCANPATH_SRC,
            vec![sub(scan_with_commit(c_scan)), sub(path_with_commit(c_path))],
        );
        m.commitment_attestations = vec![
            test_attestation(c_scan, Fr::from(7u64), &sk),
            test_attestation(c_path, Fr::from(9u64), &sk),
        ];
        m.revocation = Some(test_revocation());
        assert!(
            bind_issuer_attestations(&m, &k, &std::collections::BTreeSet::new()).is_ok(),
            "an attested, distinctly-salted scan+path must pass the issuer + salt gate"
        );
    }
}
