// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! Issuer signatures over per-graph commitments (audit #3, the M1 attestation
//! foundation).
//!
//! # The hole this closes
//! Before this module, `C(G)` (`crate::commit`) was a prover-supplied public
//! input with NO signature: the scan circuit only proves
//! `commit_fold(witnessed_leaves, count) == commitments[g]` for a
//! PROVER-CHOSEN `commitments[g]`, and the verifier never resolved an issuer
//! key or checked a signature. So the prover was effectively the issuer of
//! every fact it proved — it could invent any triple set, or drop a
//! suspension/revocation triple and recommit over the truncated leaves. This
//! module makes every commitment carry an issuer signature the verifier checks
//! against a DISCLOSED key-set `K`: an unsigned/prover-invented commitment, a
//! truncated-leaf suppression, and a key-not-in-`K` signature all fail.
//!
//! # Scheme — Schnorr over Baby-JubJub (the BN254 embedded curve), Poseidon2
//! challenge hash (`zk:poseidon2-schnorr-v1`).
//!
//! Baby-JubJub's *base* field is exactly BN254's scalar field — i.e. point
//! coordinates live in [`crate::field::Fr`] (= Noir's `Field`). That is the
//! whole point of picking this curve over Ed25519: the challenge hash is a
//! Poseidon2 sponge over field coordinates, so this signature is EXACTLY the
//! one an in-circuit verifier would later check — the privacy-preserving
//! upgrade ("prove a commitment was signed by SOME key in `K`, without
//! revealing which") is *move this verification in-circuit + a set-membership
//! gadget over `K`*, not a scheme swap.
//!
//! ## v1 placement: VERIFIER-SIDE (the sound interim)
//! The full-privacy choice the goal wants — in-circuit signature + set
//! membership over an UNDISCLOSED signing key — is expensive (in-circuit
//! Baby-JubJub scalar mul is a heavy black box; see `noir-optimisation` cost
//! table) and needs new circuit members + a vk recompute. v1 verifies the
//! signature VERIFIER-SIDE over the already-public-input-bound `commitments[g]`
//! (audit #1's reconstruction byte-binds `C(G)` into the proof), which closes
//! the unsigned-commitment hole NOW. **Privacy gap of the interim (documented
//! loudly):** the verifier checks `pk_i` in the clear, so it reveals WHICH
//! issuer signed each graph, not merely "some key in `K`". The in-circuit
//! undisclosed-key upgrade removes that leak.
//!
//! ## Modularity (per Jesse's modular-commitment/signature design)
//! [`SignatureScheme`] tags the scheme so BBS+ / SD-JWT-VC / a post-quantum
//! candidate can ship as parallel options for the paper's per-signature
//! performance + security table. v1 ships `poseidon2-schnorr-v1` only.
//!
//! ## Constant-time posture (CR-G5 / sq-8jv7 / sq-j3b9) [OPUS-5]
//! Honest statement, do NOT over-read: **this signing path is still NOT
//! asserted constant-time.** What has changed, in two steps, is how much of the
//! secret-dependent *shape* lives in code sparq owns.
//!
//! * **sq-8jv7** made `derive_nonce`'s degenerate-`k` guard branchless (always
//!   computes the re-fold candidate and `subtle`-selects it), so the nonce
//!   derivation emits no secret-dependent control flow.
//! * **sq-j3b9** replaced the secret-scalar multiplications — `pk = sk·G` in
//!   [`SecretKey::public_key`] and `R = G·k` in [`sign_deterministic`] — with
//!   `crate::ct::mul_ct`, a fixed-width double-and-ALWAYS-add ladder whose trip
//!   count and per-iteration work do not vary with the scalar. The `arkworks`
//!   generic `Group::mul_bigint` these replace branched on every secret scalar
//!   bit (`if b { res += self }`) *and* short-circuited leading zeros, i.e. it
//!   carried the textbook square-and-multiply side channel. The ladder is
//!   value-identical (differential test in `crate::ct`), so no signature, key,
//!   or wire format changes.
//!
//! **What remains open (the part sq-j3b9 did NOT close).** Every operation
//! above still bottoms out in `arkworks` (`ark-ed-on-bn254` / `ark-ff`) field
//! arithmetic, which makes **no default constant-time guarantee** and which
//! sparq does not — and cannot, without replacing the field/curve
//! implementation — assert is constant-time. The scalar arithmetic
//! `s = k + e·sk` is likewise untouched `arkworks` (though, unlike a scalar
//! multiplication, it is a fixed two-operation sequence with no loop and no
//! branch, so it carries no square-and-multiply structure). This is the
//! residual recorded in `compliance/cryptoreview/side-channel-analysis.md`
//! §2.2 / §6 and gap **CR-G5**; closing it is the curve/dep swap sq-j3b9
//! scoped as its ceiling and deliberately did not take. **No instrumented
//! `dudect`/`ctgrind` measurement has been run** — this is a source-level shape
//! argument, and a clean reading is not a timing-channel proof.
//!
//! **Why the residual is rated LOW (placement, not primitives):** the secret
//! key is used **only at ISSUANCE** (signing), which v1 places in a trusted
//! issuance environment; the relying party only ever calls [`verify`], which
//! is over **public** data (commitment + public key) and carries no secret —
//! and which therefore deliberately keeps the faster variable-time `arkworks`
//! multiplication. The rating is unchanged by sq-j3b9: hardening the shape of
//! the ladder does not upgrade the *claim*. The crate remains **research-grade
//! and externally unaudited** (CR-G1, `sq-qhy4`).

use crate::field::Fr;
use crate::poseidon2;
use ark_ec::{twisted_edwards::Affine, AffineRepr, CurveGroup, PrimeGroup};
use ark_ed_on_bn254::{EdwardsConfig, EdwardsProjective, Fr as JjScalar};
use ark_ff::{BigInteger, PrimeField, UniformRand, Zero};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};

/// Domain separator folded into the signed message (so an issuer signature
/// over `C(G)` can never be replayed as a signature over some other field
/// element with a different meaning — domain-separation discipline,
/// `verifiable-credentials-zk` skill). Distinct from the leaf/commitment IVs.
const SIG_DOMAIN_COMMITMENT: u64 = 0x5a4b_5349_475f_4331; // "ZKSIG_C1"

/// Which signature scheme an issuer key/signature uses. v1 ships Schnorr over
/// Baby-JubJub only; the enum is the modularity swap-point (BBS+, SD-JWT-VC,
/// post-quantum candidates ship as parallel variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureScheme {
    /// Schnorr over Baby-JubJub with a Poseidon2 challenge (`zk:poseidon2-schnorr-v1`).
    Poseidon2SchnorrV1,
}

impl SignatureScheme {
    /// The `zk:cryptosuite` IRI for this scheme.
    pub const POSEIDON2_SCHNORR_V1_IRI: &'static str =
        "https://sparq.dev/ns/zk#poseidon2-schnorr-v1";

    pub fn cryptosuite_iri(self) -> &'static str {
        match self {
            SignatureScheme::Poseidon2SchnorrV1 => Self::POSEIDON2_SCHNORR_V1_IRI,
        }
    }

    pub fn from_cryptosuite_iri(iri: &str) -> Option<Self> {
        match iri {
            Self::POSEIDON2_SCHNORR_V1_IRI => Some(SignatureScheme::Poseidon2SchnorrV1),
            _ => None,
        }
    }
}

// --- pluggable issuer-signature seam (sq-1hsl) ----------------------------
// [OPUS-4.8] sq-1hsl: the OFF-circuit signature seam as an OPEN trait, per the
// maintainer-greenlit (#769) configurable-commitment build-out
// (`research/zk-configurable-commitment-design.md` §4, §10 Q4 default = open
// trait). This is the SIGNATURE-VERIFICATION axis, distinct from the
// commitment-method axis ([`crate::commit::CommitmentMethod`], a CLOSED
// fail-closed enum because the method is IN-CIRCUIT-affecting). Signature
// verification here is OFF-circuit (verifier-side, public data only), so an open
// trait is the right extensibility point: a second verifier-side scheme (an
// EdDSA/ECDSA-over-a-VC bridge, §5) can be added without touching the
// registry/verifier call sites or any circuit.
//
// SCOPE + HONESTY (load-bearing): this is the trait BOUNDARY plus the existing
// Schnorr-over-Baby-JubJub scheme moved BEHIND it, byte-for-byte unchanged. It
// adds NO new production scheme, NO circuit, and NO dependency. The closed
// [`SignatureScheme`] enum tag is RETAINED unchanged (the registry, the manifest
// verifier in `sparq-zk-compose`, and already-issued credentials reference it),
// so this is purely additive and back-compatible. The whole ZK estate is
// remediated + internally re-audited but NOT externally audited (sq-qhy4, P0);
// nothing here asserts a soundness or privacy property.

/// Whether (and how) an issuer-signature scheme can be verified IN-CIRCUIT — the
/// honest discriminator the configurability design (§4.2) surfaces, because it is
/// the property that actually matters for this estate: a scheme over BN254's
/// embedded curve has a native in-circuit verifier; a generic VC signature over a
/// non-embedded curve does not (it is verifiable verifier-side only, which
/// sacrifices the hidden-issuer privacy upgrade).
///
/// This is a LOCAL descriptor, NOT a `sparq-zk-compose::CircuitId` — `sparq-zk`
/// is the lower crate and does not depend on the circuit crate (the in-circuit
/// gadget for the v1 scheme lives in `sparq-zk-compose`'s `hidden_issuer_d{depth}`
/// member; this only NAMES that such a member exists). Fail-closed by default:
/// a scheme with no in-circuit verifier returns [`Self::None`].
// [OPUS-4.8] sq-1hsl.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InCircuitVerifier {
    /// The scheme has NO in-circuit verifier — it is checkable verifier-side only
    /// (e.g. an EdDSA/ECDSA-over-a-VC bridge over a non-embedded curve, §4.3). The
    /// hidden-issuer ("signed by SOME key in `K`") privacy upgrade is NOT reachable.
    None,
    /// The scheme has a native in-circuit verifier (over BN254's embedded curve),
    /// reachable as a `sparq-zk-compose` hidden-issuer circuit member. The hidden-key
    /// set-membership privacy upgrade is reachable. `member_hint` names the member
    /// family for documentation/dispatch (`hidden_issuer`), NOT a circuit handle.
    Native {
        /// The in-circuit member-family name (e.g. `"hidden_issuer"`). A stable
        /// string, NOT a typed `CircuitId` (that lives in the higher crate).
        member_hint: &'static str,
    },
}

/// An OFF-circuit issuer-signature scheme — the pluggable, verifier-side
/// signature seam (sq-1hsl, design §4.2). Object-safe (`&dyn
/// IssuerSignatureScheme`) so a resolver can hold a registry of schemes and
/// dispatch by cryptosuite IRI.
///
/// All methods operate on the SERIALIZED hex forms the registry / manifest carry
/// (`zk:issuerPublicKey`, `zk:commitmentSignature`), so the trait abstracts over
/// schemes whose key/signature byte layouts differ. The signed MESSAGE is a
/// scheme-independent domain-separated field element (built by the
/// `commitment_message*` free functions — that is message construction, not the
/// signature scheme), passed in as `&Fr`.
///
/// Fail-closed: every method must return `false` / reject on any malformed input
/// (a relying party feeds prover-controlled bytes), never panic.
///
/// # Not a guarantee
/// Implementing this trait asserts NO cryptographic property. The estate is NOT
/// externally audited (sq-qhy4). The trait only standardizes the verifier-side
/// dispatch boundary so a second scheme can be wired in without touching call
/// sites.
// [OPUS-4.8] sq-1hsl: open OFF-circuit signature-scheme trait.
pub trait IssuerSignatureScheme {
    /// The `zk:cryptosuite` IRI this scheme records on a registry entry /
    /// attestation. The resolver ([`resolve_signature_scheme`]) matches on this.
    fn cryptosuite_iri(&self) -> &str;

    /// Verify a signature (hex) over the domain-separated message field element
    /// `m`, under the issuer public key (hex). Verifier-side, PUBLIC data only —
    /// the relying-party path. Fail-closed on malformed key/signature hex.
    fn verify_message(&self, pk_hex: &str, m: &Fr, sig_hex: &str) -> bool;

    /// Whether this scheme has an in-circuit verifier (and which member family) —
    /// the honest discriminator of §4.2. Defaults to [`InCircuitVerifier::None`]
    /// (fail-closed: a scheme is assumed verifier-side-only unless it states a
    /// native in-circuit member).
    fn in_circuit(&self) -> InCircuitVerifier {
        InCircuitVerifier::None
    }
}

/// The default issuer-signature scheme: Schnorr over Baby-JubJub with a Poseidon2
/// challenge (`zk:poseidon2-schnorr-v1`) — the v1 scheme, now expressed BEHIND the
/// [`IssuerSignatureScheme`] trait. Its [`Self::verify_message`] is byte-for-byte
/// the existing verifier-side path (parse via [`public_key_from_hex`] and
/// [`signature_from_hex`], then [`verify`]), so wrapping it in the trait changes no
/// behaviour — the same key, message, and signature produce the same accept/reject
/// as a direct [`verify`] call (pinned by the seam tests).
///
/// It is the one scheme with a NATIVE in-circuit verifier (Baby-JubJub's base
/// field IS BN254's scalar field), so [`Self::in_circuit`] reports the
/// `hidden_issuer` member family — the embedded-curve property that made this the
/// v1 choice over Ed25519 (see the module-level scheme note).
// [OPUS-4.8] sq-1hsl: the Schnorr-over-Baby-JubJub scheme as the first trait impl.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SchnorrBjjScheme;

impl IssuerSignatureScheme for SchnorrBjjScheme {
    fn cryptosuite_iri(&self) -> &str {
        SignatureScheme::POSEIDON2_SCHNORR_V1_IRI
    }

    fn verify_message(&self, pk_hex: &str, m: &Fr, sig_hex: &str) -> bool {
        // Byte-identical to the existing verifier-side path. Fail-closed on hex
        // that does not parse to a usable key / signature.
        let (Some(pk), Some(sig)) = (public_key_from_hex(pk_hex), signature_from_hex(sig_hex))
        else {
            return false;
        };
        verify(&pk, m, &sig)
    }

    fn in_circuit(&self) -> InCircuitVerifier {
        // Embedded-curve scheme => a native hidden-issuer in-circuit member exists
        // (`sparq-zk-compose`'s `hidden_issuer_d{depth}`).
        InCircuitVerifier::Native { member_hint: "hidden_issuer" }
    }
}

/// Resolve the OFF-circuit issuer-signature scheme for a `zk:cryptosuite` IRI, or
/// `None` if no in-tree scheme verifies it (fail-closed — a verifier requiring a
/// known scheme MUST reject an unresolved cryptosuite, never assume a default).
///
/// Today only [`SchnorrBjjScheme`] is implemented, so this resolves exactly the
/// IRIs [`SignatureScheme::from_cryptosuite_iri`] accepts — the two stay in lock
/// step (a seam test pins the equivalence). The resolver is the additive
/// extension point: a future verifier-side scheme (an EdDSA/ECDSA-over-a-VC
/// bridge, §5) is wired in by adding its IRI arm here, with no change to the
/// registry or `sparq-zk-compose` call sites.
///
/// Returns a boxed `dyn` so the caller can dispatch uniformly across schemes
/// whose key/signature layouts differ.
// [OPUS-4.8] sq-1hsl: cryptosuite-IRI -> scheme resolver (the open seam's registry).
pub fn resolve_signature_scheme(cryptosuite_iri: &str) -> Option<Box<dyn IssuerSignatureScheme>> {
    if cryptosuite_iri == SignatureScheme::POSEIDON2_SCHNORR_V1_IRI {
        return Some(Box::new(SchnorrBjjScheme));
    }
    None
}

/// Verify a commitment signature using the scheme selected by `cryptosuite_iri`,
/// composing the OFF-circuit signature seam with the rest of the verify path. The
/// signed `m` is the domain-separated message the caller built (e.g.
/// [`commitment_message_with_status`]). Fail-closed: an unresolved cryptosuite
/// (no in-tree scheme) returns `false`, exactly like the existing
/// `RegistryEntry::verify_commitment_signature_with_status` guard — it does not
/// fall back to a default scheme.
///
/// This is the seam's convenience entry point; it composes naturally with the
/// commitment-method registry (a [`crate::registry::RegistryEntry`] carries BOTH
/// its `zk:scheme` commitment method and its `zk:cryptosuite` signature scheme, so
/// a caller resolves the method via [`crate::commit::CommitmentMethod::from_scheme_iri`]
/// and the signature scheme via this function — the two independent axes the
/// design separates).
// [OPUS-4.8] sq-1hsl.
pub fn verify_commitment_with_scheme(
    cryptosuite_iri: &str,
    pk_hex: &str,
    m: &Fr,
    sig_hex: &str,
) -> bool {
    match resolve_signature_scheme(cryptosuite_iri) {
        Some(scheme) => scheme.verify_message(pk_hex, m, sig_hex),
        None => false,
    }
}

/// An issuer's public verification key: a Baby-JubJub point `pk = sk·G`. Its
/// affine coordinates `(x, y)` are field elements (= Noir `Field`), so the key
/// itself is in the same arena as the commitment it signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicKey(pub Affine<EdwardsConfig>);

impl PublicKey {
    /// True iff this is a USABLE issuer/binding key: on the Baby-JubJub curve, in
    /// the prime-order (order-`L`) subgroup, and not the identity.
    ///
    /// [OPUS-4.8] sq-l15mi (audit H-1): Baby-JubJub has cofactor 8, so on-curve
    /// points can have small order (2/4/8). Because [`PublicKey`]'s inner field is
    /// `pub`, a caller can construct a key around such a torsion point (or an
    /// off-curve point) DIRECTLY, bypassing `public_key_from_hex`. A torsion issuer
    /// key breaks Schnorr soundness on the hidden-issuer path — it admits a
    /// no-secret forgery (the in-circuit `issuer.nr` gadget mirrors this guard as
    /// `[L]·pk == O`). Every host path that could bind a key into a key-set `K` or a
    /// circuit witness therefore gates on this predicate; `verify` enforces the same
    /// on the clear-issuer path. This is exactly ark's
    /// `is_in_correct_subgroup_assuming_on_curve` (`[MODULUS]·P == 0`), plus the
    /// on-curve and non-identity guards it assumes.
    pub fn is_prime_order(&self) -> bool {
        !self.0.is_zero()
            && self.0.is_on_curve()
            && self.0.is_in_correct_subgroup_assuming_on_curve()
    }
}

/// An issuer secret key (a Baby-JubJub scalar). Test/issuance-side only — a
/// relying party never sees it.
///
/// # Secret-memory hygiene (sq-u8a8, from the side-channel analysis CR-G5)
/// [OPUS-4.8] The scalar is **zeroized on drop** so the key bytes do not linger
/// in freed stack/heap memory after the `SecretKey` goes out of scope. arkworks'
/// `JjScalar` (`ark-ed-on-bn254::Fr`) does not implement [`zeroize::Zeroize`], so
/// we cannot derive it; instead a manual [`Drop`] overwrites the scalar's
/// canonical little-endian bytes (the only secret-bearing representation we can
/// reach) and resets the field element to zero. This is **memory HYGIENE ONLY**:
/// it changes neither the signing algorithm nor any value the key produces while
/// alive (every signing/verify test is byte-identical). Because a type that
/// implements [`Drop`] cannot also be `Copy`, [`SecretKey`] is no longer `Copy`
/// (it is still [`Clone`]); all callers pass it by `&` reference, so this is not a
/// behavioural change.
#[derive(Clone)]
pub struct SecretKey(pub JjScalar);

// [OPUS-4.8] sq-u8a8: zeroize the secret scalar on drop. `JjScalar` is not
// `Zeroize`, so we best-effort scrub its canonical byte image and overwrite the
// field element with zero. This is hygiene, not a protocol change — the key
// behaves identically for its whole lifetime; only its drop is scrubbed.
impl zeroize::Zeroize for SecretKey {
    fn zeroize(&mut self) {
        // Scrub the canonical little-endian byte image of the scalar, then reset
        // the live field element to the additive identity so nothing recoverable
        // remains in the `SecretKey`'s own storage.
        let mut bytes = self.0.into_bigint().to_bytes_le();
        bytes.zeroize();
        self.0 = JjScalar::zero();
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.zeroize();
    }
}

// [OPUS-4.8] sq-u8a8: manual `Debug` that REDACTS the secret scalar. A derived
// `Debug` would print the raw key; a stray `{:?}` (log line, panic, failed
// `assert_eq!`) would then leak it. (The old derive was `Debug` only because the
// type was a simple newtype; redacting it is pure hygiene — no caller relies on
// the scalar appearing in the debug output.)
impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SecretKey").field(&"<redacted>").finish()
    }
}

impl SecretKey {
    /// [OPUS-4.8] sq-u8a8: constant-time equality of two secret keys, comparing
    /// their canonical little-endian scalar bytes via [`subtle::ConstantTimeEq`]
    /// so the comparison time does not depend on WHERE two keys first differ.
    ///
    /// This is a **defensive PRIMITIVE**, not a replacement for any existing
    /// comparison: the side-channel analysis (CR-G5) found sparq performs no
    /// secret-vs-secret key equality on any live path (the relying party only ever
    /// calls [`verify`] over PUBLIC data), so nothing in the protocol calls this
    /// today. It exists so any FUTURE secret-key comparison is constant-time by
    /// construction rather than by a plain `==` on `JjScalar` (whose `PartialEq`
    /// is not asserted constant-time). Behaviour is identical to a value `==`.
    pub fn ct_eq(&self, other: &SecretKey) -> subtle::Choice {
        use subtle::ConstantTimeEq;
        let a = self.0.into_bigint().to_bytes_le();
        let b = other.0.into_bigint().to_bytes_le();
        a.ct_eq(&b)
    }
}

/// A Schnorr signature `(R, s)` over a message field element: `R = k·G`,
/// `s = k + e·sk` with `e = Poseidon2(DOMAIN, R.x, R.y, pk.x, pk.y, m)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature {
    /// The nonce commitment `R = k·G`.
    pub r: Affine<EdwardsConfig>,
    /// `s = k + e·sk` in the curve's scalar field.
    pub s: JjScalar,
}

impl SecretKey {
    /// Derive the public key `pk = sk·G`.
    // [OPUS-5] sq-j3b9: `sk` is the long-term secret, so the multiplication goes
    // through the scalar-independent ladder (`crate::ct::mul_ct`) rather than
    // arkworks' `if bit { add }` double-and-add. Value-identical; see `crate::ct`
    // for the bounded claim (this is NOT "constant-time signing").
    pub fn public_key(&self) -> PublicKey {
        let g = EdwardsProjective::generator();
        PublicKey(crate::ct::mul_ct(&g, &self.0).into_affine())
    }

    /// A deterministic secret key from a `u64` seed — issuance-tooling / test
    /// convenience so callers in crates without an `ark` dependency can mint a
    /// key without touching the curve types. NOT for production key generation
    /// (use OS entropy). Distinct seeds give distinct keys.
    // [OPUS-4.8] audit #3 test/tooling helper.
    pub fn from_seed(seed: u64) -> Self {
        use ark_std::rand::SeedableRng;
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(seed);
        SecretKey(JjScalar::rand(&mut rng))
    }

    /// Sign a per-graph commitment under this key, returning the signature hex
    /// (`compressed(R) ‖ s`). The signed message is the domain-separated
    /// commitment message, matching [`verify`] on the verifier side.
    ///
    /// # Nonce derivation (audit #3 codex finding #4 fix)
    /// The Schnorr nonce `k` is derived DETERMINISTICALLY from (secret key,
    /// message) via the Poseidon2 sponge (an RFC6979-style derivation in the
    /// signature's native field), NOT from any caller-supplied seed. The old
    /// `sign_commitment_seeded(.., seed: u64)` was an unsafe public API: a caller
    /// reusing a `seed` across two distinct messages produced two signatures with
    /// the SAME `R = k·G` but different `s`, from which `sk = (s1 - s2)/(e1 - e2)`
    /// is trivially recovered. Deriving `k` from `(sk, m)` removes the seed
    /// entirely — the same message always yields the same (safe) signature, and
    /// distinct messages get distinct nonces, with no way for a caller to force a
    /// nonce collision. Production keys must still be generated from OS entropy
    /// (see [`SecretKey::from_seed`]'s warning); deterministic nonces only remove
    /// the *signing-time* entropy requirement, exactly as RFC6979 does for ECDSA.
    // [OPUS-4.8] audit #3 codex #4: deterministic (sk, m) nonce, no caller seed.
    pub fn sign_commitment(&self, commitment: &Fr) -> String {
        let sig = sign_deterministic(self, &commitment_message(commitment));
        signature_to_hex(&sig)
    }

    /// Sign a per-graph commitment AND its salt (audit #9), returning the
    /// signature hex. The signed message is [`commitment_message_with_salt`], so
    /// the salt under which the graph was committed is issuer-attested. Same
    /// deterministic `(sk, m)` nonce discipline as [`Self::sign_commitment`].
    // [OPUS-4.8] audit #9: salt-bound issuance.
    pub fn sign_commitment_with_salt(&self, commitment: &Fr, salt: &Fr) -> String {
        let sig = sign_deterministic(self, &commitment_message_with_salt(commitment, salt));
        signature_to_hex(&sig)
    }

    /// Sign a per-graph commitment, its salt, AND its status-list reference
    /// digest (audit #12), returning the signature hex. The signed message is
    /// [`commitment_message_with_status`], so the credential's revocation
    /// reference (list id + index + version) is issuer-attested and cannot be
    /// omitted or forged by the prover. Same deterministic `(sk, m)` nonce
    /// discipline as [`Self::sign_commitment`].
    // [OPUS-4.8] audit #12: status-bound issuance.
    pub fn sign_commitment_with_status(
        &self,
        commitment: &Fr,
        salt: &Fr,
        status_ref: &Fr,
    ) -> String {
        let sig = sign_deterministic(
            self,
            &commitment_message_with_status(commitment, salt, status_ref),
        );
        signature_to_hex(&sig)
    }

    /// Sign a per-graph commitment, its salt, its status-list reference digest,
    /// AND a HOLDER-public-key digest (sq-y464, HolderPoP T1), returning the
    /// signature hex. The signed message is [`commitment_message_with_holder`], so
    /// the holder key the credential is bound to becomes ISSUER-ATTESTED for that
    /// credential (closing the trusted-holder gap — see
    /// [`commitment_message_with_holder`]). `holder_pk_digest` is
    /// [`holder_key_digest`]`(hpk)`. Same deterministic `(sk, m)` nonce discipline
    /// as [`Self::sign_commitment`].
    ///
    /// Purely additive — it does not change [`Self::sign_commitment_with_status`]
    /// or any existing signature; bearer credentials remain valid. The verifier-side
    /// fail-closed enforcement has LANDED (T3/sq-z8s7): given the disclosed holder
    /// key, `sparq_zk_compose::verifier::bind_holder_pop` recovers this signed
    /// `holder_pk_digest` from the credential's attestation and rejects a presenter
    /// whose key digest disagrees (`HolderKeyMismatch`); a relying party opting in
    /// with `HolderBindingPolicy::require_binding()` additionally rejects a bearer
    /// credential (`HolderBindingMissing`). Research-grade, NOT externally audited
    /// (sq-qhy4) — no soundness or privacy property is claimed as achieved.
    // [OPUS-4.8] sq-y464 (HolderPoP T1): holder-bound issuance.
    // [OPUS-5] sq-sg37 (HolderPoP T8): deferral note retired — T3/sq-z8s7 landed.
    pub fn sign_commitment_with_holder(
        &self,
        commitment: &Fr,
        salt: &Fr,
        status_ref: &Fr,
        holder_pk_digest: &Fr,
    ) -> String {
        let sig = sign_deterministic(
            self,
            &commitment_message_with_holder(commitment, salt, status_ref, holder_pk_digest),
        );
        signature_to_hex(&sig)
    }
}

/// Derive a deterministic Schnorr nonce `k` from the secret key and the message
/// (RFC6979-style, in the curve's native field). The secret key's scalar is
/// mapped into the base field [`Fr`] (its big-endian bytes, reduced) and folded
/// with the message and a nonce-specific domain tag through the Poseidon2 sponge;
/// the resulting base-field digest is reduced into the scalar field. This is a
/// PRF over `(sk, m)`: it never repeats a nonce for distinct messages and never
/// leaks `sk`, so no signing-time entropy is needed and seed-reuse is impossible
/// by construction (there is no seed). Both this and the in-circuit-recomputable
/// challenge live in the same base field, keeping the scheme circuit-friendly.
// [OPUS-4.8] audit #3 codex #4.
fn derive_nonce(sk: &SecretKey, m: &Fr) -> JjScalar {
    use subtle::{ConditionallySelectable, ConstantTimeEq};
    // A distinct domain tag from the challenge so the nonce-PRF output can never
    // collide with / be mistaken for a challenge value.
    const SIG_DOMAIN_NONCE: u64 = 0x5a4b_5349_475f_4e31; // "ZKSIG_N1"
    // Map the secret scalar into the base field via its big-endian bytes. The
    // base field (BN254 scalar field) is larger than the Baby-JubJub scalar
    // field, so this reduction is injective on canonical sk encodings.
    let sk_base = Fr::from_be_bytes_mod_order(&sk.0.into_bigint().to_bytes_be());
    let k_base: Fr = poseidon2::hash(&[Fr::from(SIG_DOMAIN_NONCE), sk_base, *m]);
    let k = JjScalar::from_be_bytes_mod_order(&k_base.into_bigint().to_bytes_be());

    // [OPUS-4.8] sq-8jv7: guard the degenerate `k == 0` (would make `R` the
    // identity) WITHOUT a secret-dependent branch in our own code. The old
    // `if k.is_zero() { .. } else { .. }` selected on the secret nonce; here we
    // ALWAYS compute the re-fold candidate `k2` and then `subtle`-select it only
    // when `k == 0`, so the control flow our code emits is data-independent of
    // the secret nonce. (The `k == 0` event is itself negligibly rare — ~2^-251
    // for a 251-bit scalar field — so this is defence-in-depth, not a measured
    // leak. [OPUS-5] sq-j3b9 has since moved the scalar mul `G·k` onto the
    // fixed-width `crate::ct::mul_ct` ladder; the remaining residual is the
    // arkworks scalar arithmetic `e·sk.0` and the underlying field ops, which
    // sparq does NOT assert constant-time — see the module CONSTANT-TIME POSTURE
    // note and `compliance/cryptoreview/side-channel-analysis.md` §2.2.)
    let k2_base: Fr = poseidon2::hash(&[Fr::from(SIG_DOMAIN_NONCE), k_base, *m]);
    let k2 = JjScalar::from_be_bytes_mod_order(&k2_base.into_bigint().to_bytes_be());
    // Constant-time "is k zero?" over the canonical little-endian scalar bytes,
    // then a branchless byte-wise select of `k` vs `k2` keyed on that Choice.
    let k_le = k.into_bigint().to_bytes_le();
    let zero_le = JjScalar::zero().into_bigint().to_bytes_le();
    let k_is_zero = k_le.ct_eq(&zero_le);
    let mut sel = k_le;
    let k2_le = k2.into_bigint().to_bytes_le();
    debug_assert_eq!(sel.len(), k2_le.len());
    for (dst, src) in sel.iter_mut().zip(k2_le.iter()) {
        *dst = u8::conditional_select(dst, src, k_is_zero);
    }
    JjScalar::from_le_bytes_mod_order(&sel)
}

/// Sign `m` with `sk` using a DETERMINISTIC nonce derived from `(sk, m)` (no
/// entropy source, no caller seed — see `derive_nonce`). This is the
/// issuance-side path used by [`SecretKey::sign_commitment`]; a relying party
/// only ever calls [`verify`]. Equivalent in shape to `sign` but with the
/// nonce pinned, so it is replay-stable and seed-reuse-proof.
//
// # Constant-time posture (CR-G5 / sq-8jv7 / sq-j3b9) [OPUS-5]
// Still NOT asserted constant-time. The secret-dependent SHAPE is now sparq's:
// [`derive_nonce`]'s `k == 0` guard is branchless (sq-8jv7) and `G·k` goes
// through the fixed-width `crate::ct::mul_ct` ladder rather than arkworks'
// bit-branching double-and-add (sq-j3b9). The scalar arithmetic `k + e·sk.0`
// and every underlying field operation remain arkworks ops sparq does not
// assert are constant-time — the residual (see the module CONSTANT-TIME POSTURE
// note), LOW today by issuance-side placement.
// [OPUS-4.8] audit #3 codex #4.
pub fn sign_deterministic(sk: &SecretKey, m: &Fr) -> Signature {
    let g = EdwardsProjective::generator();
    let k = derive_nonce(sk, m);
    // [OPUS-5] sq-j3b9: `k` is the secret nonce — recovering even a few of its
    // bits over several signatures recovers `sk` — so `R = G·k` goes through the
    // scalar-independent ladder, not arkworks' bit-branching double-and-add.
    let r_pt = crate::ct::mul_ct(&g, &k).into_affine();
    let pk = sk.public_key().0;
    let e = challenge(&r_pt, &pk, m);
    let s = k + e * sk.0;
    Signature { r: r_pt, s }
}

/// The challenge `e = Poseidon2([DOMAIN, R.x, R.y, pk.x, pk.y, m])`, reduced
/// into the curve's scalar field. Computed over the *base* field (= `Fr`) so it
/// is in-circuit-recomputable, then reduced via its big-endian bytes (the
/// standard Schnorr "hash-to-scalar" step). Both signer and verifier call this,
/// so a drift in the challenge derivation can never make a forged signature
/// verify (single source of truth, mirroring the `verify_*_relation` discipline
/// in the noir-optimisation skill).
fn challenge(r: &Affine<EdwardsConfig>, pk: &Affine<EdwardsConfig>, m: &Fr) -> JjScalar {
    // [OPUS-4.8] codex #2 (false positive, confirmed): in ark-ec 0.5
    // `AffineRepr::xy()` returns `Option<(Self::BaseField, Self::BaseField)>` —
    // OWNED `BaseField` values, not references (see ark-ec-0.5.0
    // models/twisted_edwards/affine.rs:164). `BaseField == Fr` here, so the
    // `unwrap_or` with owned `Fr` zeros is type-correct and compiles. (The
    // identity is the only point with `xy() == None`; it is rejected in `verify`
    // and `public_key_from_hex` before reaching here — codex #3.)
    let (rx, ry) = r.xy().unwrap_or((Fr::from(0u64), Fr::from(0u64)));
    let (px, py) = pk.xy().unwrap_or((Fr::from(0u64), Fr::from(0u64)));
    let e_base: Fr = poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_COMMITMENT),
        rx,
        ry,
        px,
        py,
        *m,
    ]);
    // Reduce the base-field challenge into the scalar field (big-endian bytes,
    // mod the scalar order). Deterministic and recomputable in-circuit.
    JjScalar::from_be_bytes_mod_order(&e_base.into_bigint().to_bytes_be())
}

/// The signed message for a per-graph commitment: a domain-separated binding of
/// `C(G)` to the issuer. The signature is over THIS value, never the raw
/// commitment — so an issuer's commitment signature is not interchangeable with
/// any other use of the same field element.
pub fn commitment_message(commitment: &Fr) -> Fr {
    poseidon2::hash(&[Fr::from(SIG_DOMAIN_COMMITMENT), *commitment])
}

/// The signed message for a per-graph commitment that ALSO binds the per-graph
/// RDFC10 bnode salt (audit #9). The issuer signs `(C(G), salt_G)` under a
/// distinct domain tag, so the salt under which a graph was committed becomes
/// ISSUER-ATTESTED: a salt-reusing ingester cannot present a graph under a salt
/// the trusted issuer did not sign, and the verifier — which sees `salt_G` in
/// the attestation — can detect salt reuse across distinct commitments (the Q6
/// cross-graph bnode-correlation channel).
///
/// # Why bind the salt at all (the audit #9 fix)
/// The Q6 "bnodes from different graphs are distinct by construction" guarantee
/// rests on each graph having a globally-unique salt (so a recurring canonical
/// bnode label `c14n0` encodes to DIFFERENT field elements in different graphs).
/// The salt never enters any circuit and the bare `commitment_message` does not
/// bind it, so a salt-reusing ingester could correlate bnodes across graphs. By
/// folding `salt_G` into the signed object, the property is anchored on the
/// TRUSTED ISSUER (who draws a fresh OS-random salt per graph at mint, see
/// `encode::salt_from_bytes`) rather than on an unenforced honest-ingest
/// convention. A forger who reuses a salt cannot obtain a valid issuer signature
/// over the reused `(C(G), salt)` pair.
// [OPUS-4.8] audit #9: salt-bound commitment message.
const SIG_DOMAIN_COMMITMENT_SALT: u64 = 0x5a4b_5349_475f_4332; // "ZKSIG_C2"
pub fn commitment_message_with_salt(commitment: &Fr, salt: &Fr) -> Fr {
    poseidon2::hash(&[Fr::from(SIG_DOMAIN_COMMITMENT_SALT), *commitment, *salt])
}

/// A domain-separated digest of a credential's STATUS-LIST REFERENCE (audit
/// #12): the status list the credential's liveness is tracked in, the
/// credential's index into that list, and the list VERSION (a monotone
/// freshness counter, e.g. the issuer's status-list `validFrom` epoch or
/// publication sequence number). Folded into the issuer signature by
/// [`commitment_message_with_status`] so the reference cannot be omitted,
/// forged, or swapped by the prover.
///
/// `list_id` is the caller-hashed status-list IRI (a field element — the
/// verifier hashes the disclosed `manifest.revocation.status_list` IRI the same
/// way and asserts equality, so the in-the-clear IRI is bound). The index and
/// version enter as field elements directly.
// [OPUS-4.8] audit #12: status-list reference digest.
pub fn status_ref_digest(list_id: &Fr, index: u64, version: u64) -> Fr {
    const SIG_DOMAIN_STATUS_REF: u64 = 0x5a4b_5349_475f_5352; // "ZKSIG_SR"
    poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_STATUS_REF),
        *list_id,
        Fr::from(index),
        Fr::from(version),
    ])
}

/// A hiding COMMITMENT to a credential's status-list index (sq-ayv): the
/// index-hiding analogue of the clear `index` in [`status_ref_digest`]. The
/// issuer signs a commitment to the index (via [`status_ref_commit_digest`]),
/// NOT the clear index, so a hidden-revocation presentation can withhold the
/// index entirely. `blinding` is a per-credential random field element (the
/// holder's secret) that makes the commitment hiding: two credentials at the
/// same index commit to different values, closing the linkability channel the
/// clear index opened.
///
/// # Cross-binding (load-bearing for soundness)
/// This commitment is recomputed IN-CIRCUIT by the `revoke_unset_d{depth}` member
/// from the SAME private `index` the Merkle bit-unset fold uses (sq-ayv) and a
/// private `blinding`, and exposed as a PUBLIC input. The verifier then requires
/// the proof's public commitment to byte-equal the ISSUER-SIGNED commitment (this
/// value, folded into the signed message). So the index proven UNSET is provably
/// the index the issuer committed to — a holder cannot obtain a signature over a
/// commitment to its REVOKED index and prove bit-unset for some OTHER index.
///
/// Domain-separated and binding under the Poseidon2 collision/hiding assumption
/// (the same hash the commitment + Merkle layers use). Issuer and verifier (and
/// the circuit) all compute it identically, so a drift cannot make a wrong
/// commitment verify (single source of truth).
// [OPUS-4.8] sq-ayv: hiding index commitment (issuer-signed in place of the clear index).
pub fn status_index_commitment(index: u64, blinding: &Fr) -> Fr {
    const SIG_DOMAIN_STATUS_IDX_COMMIT: u64 = 0x5a4b_5349_475f_4943; // "ZKSIG_IC"
    poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_STATUS_IDX_COMMIT),
        Fr::from(index),
        *blinding,
    ])
}

/// `SIG_DOMAIN_JOIN` — domain-separation tag for the hidden cross-credential
/// join value commitment (`research/zk-hidden-join-design.md` §2.4). Distinct
/// from every other `SIG_DOMAIN_*` tag so a join commitment can never be
/// cross-substituted for a status/index/holder/credential commitment. (The
/// Noir-side circuit exposes the identical byte tag under the name
/// `JOIN_DOMAIN`.)
// [OPUS-4.8] sq-bwwl / sq-sco0: hidden-join value-commitment domain tag.
pub const SIG_DOMAIN_JOIN: u64 = 0x5a4b_5349_475f_4a4e; // "ZKSIG_JN"

/// A HIDING commitment to a hidden cross-credential join VALUE
/// (`research/zk-hidden-join-design.md` §2.4 / bead `sq-bwwl`). The single-prover
/// analogue of [`status_index_commitment`]: where that hides a status-list index,
/// this hides the JOINED TERM ENCODING of a cross-credential join (the `?p`
/// shared across two issuer credentials), so the privacy-sensitive joined entity
/// never enters any public input.
///
/// `value` is the Poseidon2 term encoding of the join key. `blinding` is a
/// per-PRESENTATION random field element. Two presentations of the same join
/// value produce UNLINKABLE commitments, and — load-bearing — a verifier cannot
/// dictionary-attack a low-entropy join key by enumerating candidate encodings
/// (design §1.4 R4): a bare deterministic encoding would be brute-forceable, the
/// blinded commitment is hiding.
///
/// # Cross-binding (load-bearing for soundness)
/// This commitment is recomputed IN-CIRCUIT by the `join_eq` member from the SAME
/// private join value the in-circuit equality (`a_val == b_val`) uses (design
/// §2.2 step 5) and a private `blinding`, and exposed as a PUBLIC input. For a
/// multi-way (N-way) join the same `join_commitment` is shared across each
/// pairwise `join_eq`, composing pairwise equalities into an N-way join without
/// ever disclosing the value. Issuer/verifier/circuit all compute it identically
/// (single source of truth), so a drift cannot make a wrong commitment verify.
// [OPUS-4.8] sq-bwwl / sq-sco0: hidden-join value commitment (mirrors the Noir
// `join::join_value_commitment` gadget — h3(SIG_DOMAIN_JOIN, value, blinding)).
pub fn join_value_commitment(value: &Fr, blinding: &Fr) -> Fr {
    poseidon2::hash(&[Fr::from(SIG_DOMAIN_JOIN), *value, *blinding])
}

/// A domain-separated digest of a credential's status-list reference that binds a
/// COMMITMENT to the index (sq-ayv) instead of the clear index — the
/// index-hiding analogue of [`status_ref_digest`]. The issuer folds
/// `(list_id, index_commitment, version)` so the credential's revocation
/// reference is issuer-attested WITHOUT the clear index ever entering the signed
/// object. A DISTINCT domain tag from [`status_ref_digest`] so a clear-index
/// digest can never be substituted for a committed-index one (and vice versa).
///
/// `index_commitment` is [`status_index_commitment`]`(index, blinding)`. The
/// verifier recomputes this digest from the disclosed `index_commitment` +
/// version + hashed list IRI and checks the issuer signature over it; the
/// hidden-revocation proof then cross-binds `index_commitment` to the proven-unset
/// index in-circuit (see [`status_index_commitment`]). So the index is disclosed
/// in NEITHER the signed object NOR any clear field, yet revocation is still
/// checked against the authoritative root.
// [OPUS-4.8] sq-ayv: committed-index status-reference digest.
pub fn status_ref_commit_digest(list_id: &Fr, index_commitment: &Fr, version: u64) -> Fr {
    const SIG_DOMAIN_STATUS_REF_COMMIT: u64 = 0x5a4b_5349_475f_5343; // "ZKSIG_SC"
    poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_STATUS_REF_COMMIT),
        *list_id,
        *index_commitment,
        Fr::from(version),
    ])
}

/// [OPUS-4.8] sq-6qe: a hiding COMMITMENT to a credential's status-list
/// `(list_id, version)` reference — the IRI/version-hiding analogue of the clear
/// `list_id` + `version` that [`status_ref_commit_digest`] still folds in the
/// clear. The fully-hidden committed-index revocation path (sq-6qe) signs over
/// THIS commitment instead of the clear list/version, so a hidden-revocation
/// presentation discloses neither the list IRI nor the publication epoch.
///
/// `ref_blinding` is a per-credential random field element (the holder's secret)
/// that makes the commitment hiding: two credentials on the same list+version
/// commit to different values, closing the linkability channel the clear IRI +
/// version open. Domain-separated and binding under the Poseidon2
/// collision/hiding assumption.
///
/// # In-circuit use today (sq-kndw, see `research/zk-statuslist-hide-iri-version.md`)
/// The COMPILED fully-hidden member `revoke_hidden_ref_d10_a4` recomputes this
/// commitment IN-CIRCUIT from the SAME private `(list_id, version)` it (a) proves
/// member of the relying party's accepted `(list, version, status_list_root)` set
/// and (b) freshness-checks (`version >= public min_version`), exposing it as a
/// PUBLIC input so the verifier byte-matches it against the ISSUER-SIGNED value —
/// exactly the cross-binding discipline [`status_index_commitment`] uses for the
/// index. So `list_id` and `version` are disclosed in NEITHER the signed object NOR
/// any clear field, yet the proof is bound to a list/version the issuer attested and
/// the relying party accepts. The relying-party side is the
/// `sparq_zk_compose::verifier` `bind_fully_hidden_revocation` stage (run by
/// `verify_manifest`), which derives the accepted-set root and the epoch floor from
/// its OWN curated snapshots and rebuilds the public inputs from them, so the prover
/// chooses neither anchor. Research-grade, NOT externally audited (sq-qhy4) — no
/// soundness or privacy property is claimed as achieved.
///
/// Maps to a Noir `Poseidon2::hash([DOMAIN, list_id, version, ref_blinding], 4)`.
// [OPUS-4.8] sq-6qe: hiding (list, version) reference commitment.
// [OPUS-5] sq-kndw: deferral note retired — the fully-hidden member is compiled.
pub fn status_ref_commitment(list_id: &Fr, version: u64, ref_blinding: &Fr) -> Fr {
    const SIG_DOMAIN_STATUS_REF_COMMITMENT: u64 = 0x5a4b_5349_475f_5243; // "ZKSIG_RC"
    poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_STATUS_REF_COMMITMENT),
        *list_id,
        Fr::from(version),
        *ref_blinding,
    ])
}

/// [OPUS-4.8] sq-6qe: a domain-separated digest of a credential's status-list
/// reference that binds a COMMITMENT to BOTH the `(list, version)` reference AND
/// the index — the fully-hidden analogue of [`status_ref_commit_digest`] (which
/// still folds the clear `list_id` + `version`). The issuer folds
/// `(ref_commitment, index_commitment)`, so NEITHER the list IRI, the version, NOR
/// the index ever enters the signed object. A DISTINCT domain tag from both
/// [`status_ref_digest`] (clear index) and [`status_ref_commit_digest`]
/// (committed index, clear list/version), so no cross-substitution between the
/// three disclosure modes is possible.
///
/// `ref_commitment` is [`status_ref_commitment`]`(list_id, version, ref_blinding)`
/// and `index_commitment` is [`status_index_commitment`]`(index, blinding)`. On the
/// fully-hidden path (sq-kndw, landed) the verifier recomputes this digest from the
/// disclosed commitments to check the issuer signature; the compiled
/// `revoke_hidden_ref_d10_a4` proof then cross-binds BOTH commitments in-circuit (the
/// proven-unset index to `index_commitment`, and the membership-resolved
/// `(list, version)` to `ref_commitment`), so the disclosure floor is "some accepted
/// (list, version) in the RP's committed set, version >= public min_version, my
/// hidden index unset". That check is the `sparq_zk_compose::verifier`
/// `bind_fully_hidden_revocation` stage, which also enforces single-use of the
/// `(ref_commitment, index_commitment)` pair — the pair is stable per ISSUANCE, so
/// the issuer must re-blind and RE-SIGN per presentation or the pair itself becomes a
/// cross-presentation linkage handle. Research-grade, NOT externally audited
/// (sq-qhy4) — no soundness or privacy property is claimed as achieved.
///
/// Maps to a Noir `Poseidon2::hash([DOMAIN, ref_commitment, index_commitment], 3)`.
// [OPUS-4.8] sq-6qe: fully-committed (list+version+index hidden) status-reference digest.
// [OPUS-5] sq-kndw: deferral note retired — the fully-hidden path is landed.
pub fn status_ref_fully_committed_digest(ref_commitment: &Fr, index_commitment: &Fr) -> Fr {
    const SIG_DOMAIN_STATUS_REF_FULL_COMMIT: u64 = 0x5a4b_5349_475f_4643; // "ZKSIG_FC"
    poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_STATUS_REF_FULL_COMMIT),
        *ref_commitment,
        *index_commitment,
    ])
}

/// [OPUS-4.8] sq-6qe: the relying party's ACCEPTED-SET Merkle LEAF for a
/// `(list_id, version, status_list_root)` triple (sub-option A of the sq-6qe
/// design — see `research/zk-statuslist-hide-iri-version.md`). The fully-hidden
/// revocation circuit proves membership of the credential's hidden `(list_id,
/// version)` in the relying party's accepted set committed as a Poseidon2 Merkle
/// root over these leaves — which ALSO privately binds the corresponding
/// `status_list_root` the bit-unset fold then runs against, so the verifier
/// publishes only the accepted-SET root and never learns which list/version/root.
///
/// Binding all three in ONE leaf is load-bearing: a prover cannot pair list₁'s
/// identity with list₂'s root, because the leaf hash commits the triple atomically
/// and the membership fold must recompute the public accepted-set root (the same
/// atomic-binding discipline [`crate::sig::status_index_commitment`] +
/// `revoke_unset_check_committed` use). The relying party builds this set from its
/// OWN authoritative, freshness-curated snapshots (the audit-#12 trust anchor,
/// moved behind a commitment — no new trust assumption).
///
/// Maps to the in-circuit leaf `Poseidon2::hash([DOMAIN, list_id, version,
/// status_list_root], 4)` (the accepted-set analogue of `issuer.nr`'s `key_leaf`).
// [OPUS-4.8] sq-6qe: accepted (list, version, root)-set Merkle leaf.
pub fn accepted_status_leaf(list_id: &Fr, version: u64, status_list_root: &Fr) -> Fr {
    const SIG_DOMAIN_ACCEPTED_STATUS_LEAF: u64 = 0x5a4b_5349_475f_414c; // "ZKSIG_AL"
    poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_ACCEPTED_STATUS_LEAF),
        *list_id,
        Fr::from(version),
        *status_list_root,
    ])
}

/// Hash a status-list IRI string to a field element (domain-separated), so the
/// in-the-clear `manifest.revocation.status_list` IRI can be bound under the
/// issuer signature. Issuer and verifier both call this, so a drift cannot make
/// a wrong IRI verify (single source of truth).
// [OPUS-4.8] audit #12.
pub fn status_list_id_to_field(list_iri: &str) -> Fr {
    const SIG_DOMAIN_STATUS_LIST_IRI: u64 = 0x5a4b_5349_475f_4c49; // "ZKSIG_LI"
    // Fold the UTF-8 bytes 31 at a time (each chunk fits a BN254 field element
    // headroom-safe) through the sponge, prefixed by the domain tag and the
    // byte length so two IRIs differing only by trailing padding cannot collide.
    let bytes = list_iri.as_bytes();
    let mut acc: Vec<Fr> = Vec::with_capacity(2 + bytes.len() / 31 + 1);
    acc.push(Fr::from(SIG_DOMAIN_STATUS_LIST_IRI));
    acc.push(Fr::from(bytes.len() as u64));
    for chunk in bytes.chunks(31) {
        acc.push(Fr::from_be_bytes_mod_order(chunk));
    }
    poseidon2::hash(&acc)
}

/// The signed message for a per-graph commitment that binds BOTH the per-graph
/// RDFC10 salt (audit #9) AND the credential's status-list reference (audit
/// #12). The issuer signs `(C(G), salt_G, status_ref_digest)` under a distinct
/// domain tag, so the status reference (which list, which index, which version)
/// becomes ISSUER-ATTESTED.
///
/// # Why bind the status reference at all (the audit #12 fix)
/// Before this, revocation was entirely unchecked: a prover could OMIT
/// `manifest.revocation` (or point it at an empty/attacker list) and a
/// revoked/suspended credential would still verify, because nothing tied the
/// status reference to the credential. By folding the status-reference digest
/// into the SAME issuer signature that already binds `C(G)` and the salt, the
/// reference is mandatory and unforgeable: an omitted/forged/swapped reference
/// yields a different signed message and so has no valid issuer signature
/// (fail-closed — exactly the optional-field bypass that bit #3/#8/#9/#4). The
/// verifier then (a) recomputes this message from the disclosed reference and
/// checks the signature, (b) checks the disclosed status-list snapshot's
/// bit[index] is UNSET, and (c) checks the snapshot version is within a
/// freshness window. See `sparq_zk_compose::verifier`.
// [OPUS-4.8] audit #12: status-bound commitment message.
const SIG_DOMAIN_COMMITMENT_STATUS: u64 = 0x5a4b_5349_475f_4333; // "ZKSIG_C3"
pub fn commitment_message_with_status(commitment: &Fr, salt: &Fr, status_ref: &Fr) -> Fr {
    poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_COMMITMENT_STATUS),
        *commitment,
        *salt,
        *status_ref,
    ])
}

/// A holder key that cannot be digested into a binding (sq-y464, HolderPoP T1).
/// Fails closed — mirrors the [`verify`] / [`public_key_from_hex`] discipline that
/// the curve identity is never an admissible key.
// [OPUS-4.8] sq-y464 (HolderPoP T1): holder-key digest error (fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HolderKeyError {
    /// The holder public key is the curve IDENTITY (neutral element): it has no
    /// affine coordinates, so it is not a valid Baby-JubJub binding key. Because
    /// [`PublicKey`]'s tuple field is `pub`, an identity key can be constructed
    /// externally; digesting it would silently fold `(0, 0)`, masking accidental
    /// or malicious identity-key use — so it is rejected rather than digested.
    IdentityKey,
}

impl std::fmt::Display for HolderKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HolderKeyError::IdentityKey => write!(
                f,
                "holder public key is the curve identity (no affine coordinates); not a valid binding key"
            ),
        }
    }
}

impl std::error::Error for HolderKeyError {}

/// A domain-separated digest of a HOLDER's public key (sq-y464, HolderPoP T1):
/// `Poseidon2([ZKSIG_HK, hpk.x, hpk.y])` over the holder key's Baby-JubJub affine
/// coordinates (= base-field [`Fr`] = Noir `Field`). This is the value the issuer
/// folds into [`commitment_message_with_holder`], so a credential's bound holder
/// key is ISSUER-ATTESTED rather than a free relying-party allow-list entry. A
/// distinct domain tag ([`SIG_DOMAIN_HOLDER_KEY`] = `ZKSIG_HK`, NOT the
/// [`key_set_leaf`] `Poseidon2([x, y])` shape) so a holder-key digest can never be
/// confused with the issuer key-set Merkle leaf computed over the same coordinates.
///
/// # Mirrors the in-circuit digest (single source of truth)
/// The B2 in-circuit holder PoK has LANDED (circuit T5/sq-xqfg, verifier
/// T6/sq-i1dt): `zk/compose/compose_core/src/holder.nr`'s `holder_key_digest`,
/// driven by the `holder_pok` member, recomputes the SAME
/// `Poseidon2([ZKSIG_HK, hpk.x, hpk.y])` from a PRIVATE `hpk` and asserts it equals
/// the PUBLIC `holder_pk_digest` the verifier folded into the issuer-signed message
/// (design §2.B/B2). So this host helper and that gadget must agree bit-for-bit; a
/// drift could not make a wrong holder key verify (the established
/// issuer/verifier/circuit single-source discipline). Build the prover-side inputs
/// with [`in_circuit_holder_witness`].
///
/// # Identity key (fail-closed)
/// The identity point has no affine coordinates and is not a valid binding key
/// (it is rejected at parse/verify too — see [`public_key_from_hex`], [`verify`]).
/// Because [`PublicKey`]'s field is `pub`, an identity key can be constructed
/// EXTERNALLY and reach this PUBLIC helper directly; rather than silently folding
/// `(0, 0)` (which would mask accidental/malicious identity-key use), this returns
/// [`HolderKeyError::IdentityKey`]. This is the SINGLE point where a holder
/// [`PublicKey`] becomes a digest, so rejecting it here is sufficient:
/// [`commitment_message_with_holder`] and [`SecretKey::sign_commitment_with_holder`]
/// take the already-validated `&Fr` digest, so the whole holder-binding path is
/// fail-closed on an identity key without any infallible `(0, 0)` back-door.
// [OPUS-4.8] sq-y464 (HolderPoP T1): domain-separated holder-public-key digest.
const SIG_DOMAIN_HOLDER_KEY: u64 = 0x5a4b_5349_475f_484b; // "ZKSIG_HK"
pub fn holder_key_digest(hpk: &PublicKey) -> Result<Fr, HolderKeyError> {
    // Reject the curve identity (no affine coordinates). Do NOT fold (0, 0): an
    // identity holder key is not a valid binding key, and silently digesting it
    // would conceal accidental/malicious use of an externally-constructed identity
    // PublicKey (its tuple field is `pub`). Fail closed.
    let (x, y) = hpk.coords().ok_or(HolderKeyError::IdentityKey)?;
    Ok(poseidon2::hash(&[Fr::from(SIG_DOMAIN_HOLDER_KEY), x, y]))
}

/// Domain separator for the SINGLE-USE NULLIFIER (`sq-rsd3v.1`): `"ZKSIG_NF"`.
/// Distinct from every other `SIG_DOMAIN_*` tag — in particular from the
/// [`SIG_DOMAIN_HOLDER_KEY`] `"ZKSIG_HK"` digest computed over the SAME holder and
/// from the nonce-PRF `"ZKSIG_N1"` tag in [`derive_nonce`] — so a nullifier can
/// never be confused with a holder-key digest, a commitment challenge, or a signing
/// nonce. The in-circuit gadget (`sparq_zk_compose_core::nullifier`) folds the
/// identical byte tag, so the host and circuit nullifiers are bit-identical.
// [OPUS-4.8] sq-rsd3v.1: nullifier domain tag.
pub const SIG_DOMAIN_NULLIFIER: u64 = 0x5a4b_5349_475f_4e46; // "ZKSIG_NF"

/// The SINGLE-USE NULLIFIER for a holder key + epoch (`sq-rsd3v.1`, design
/// `research/zk-inference-and-credentials.md` §6.3):
/// `nf = Poseidon2([ZKSIG_NF, hsk, epoch])`.
///
/// `hsk` is the holder secret scalar EMBEDDED IN THE BASE FIELD (the same value the
/// in-circuit gadget witnesses — [`InCircuitHolderPokWitness::hsk`], i.e.
/// [`jj_scalar_to_base`] of the Baby-JubJub scalar). `epoch` is a
/// verifier-published rate-limit window (a field element). This host helper is the
/// SINGLE SOURCE OF TRUTH with the in-circuit `nullifier` gadget: the circuit
/// recomputes the SAME `Poseidon2([ZKSIG_NF, hsk, epoch])` from the WITNESSED `hsk`
/// and asserts it equals the PUBLIC `nf`, so a drift here would break that
/// in-circuit equality (pinned by the `nullifier_cross_vector` Noir test).
///
/// # Soundness (NOT-yet-sound, `sq-qhy4`) — rides TWO assumptions (§6.3(b))
/// The nullifier is only meaningful when the SAME `hsk` is DL-bound to a credential
/// by the holder-PoK (`hpk = hsk·G`, `holder_key_digest(hpk) == holder_pk_digest`);
/// the in-circuit `nullifier` gadget is composed WITH [`holder_key_digest`]-based
/// possession (see `sparq_zk_compose_core`'s `tests.nr`) so a prover cannot witness
/// an arbitrary `hsk`. Its unforgeability then rides (1) that holder-PoK DL-binding
/// (itself
/// NOT-yet-sound, `sq-qhy4`) AND (2) Poseidon2 domain-separated collision
/// resistance over `(ZKSIG_NF, hsk, epoch)`. The whole ZK estate is internally
/// re-audited but NOT externally audited (`sq-qhy4`); this asserts NO soundness
/// property as achieved.
///
/// # Granularity — per-holder-per-epoch, NOT per-presentation (§6.3(c))
/// `nf` binds `hsk` and `epoch` but NOT the credential/commitment. So ONE holder
/// key reused across DISTINCT credentials in the SAME epoch produces the SAME `nf`
/// (they COLLIDE) — this is a per-holder-per-epoch RATE-LIMIT, usable as a feature,
/// but it is NOT a per-presentation single-use token. A per-presentation nullifier
/// additionally folds the commitment into the hash; that is a SEPARATE, larger
/// obligation (see the module + design §6.3) and is deliberately NOT what this
/// helper computes.
// [OPUS-4.8] sq-rsd3v.1: in-circuit-mirrored single-use nullifier primitive.
pub fn nullifier(hsk: &Fr, epoch: &Fr) -> Fr {
    poseidon2::hash(&[Fr::from(SIG_DOMAIN_NULLIFIER), *hsk, *epoch])
}

/// The signed message for a per-graph commitment that binds the per-graph RDFC10
/// salt (audit #9), the credential's status-list reference (audit #12), AND a
/// HOLDER-public-key digest (sq-y464, HolderPoP T1). The issuer signs
/// `(C(G), salt_G, status_ref, holder_pk_digest)` under a NEW domain tag
/// `ZKSIG_C4` ([`SIG_DOMAIN_COMMITMENT_HOLDER`], distinct from the existing
/// [`SIG_DOMAIN_COMMITMENT`] `ZKSIG_C1`, [`SIG_DOMAIN_COMMITMENT_SALT`] `ZKSIG_C2`,
/// and [`SIG_DOMAIN_COMMITMENT_STATUS`] `ZKSIG_C3` tags), so the holder key a
/// credential is bound to becomes ISSUER-ATTESTED for that specific credential.
/// `holder_pk_digest` is [`holder_key_digest`]`(hpk)`.
///
/// # The trusted-holder gap this closes (design §1/§2.B)
/// The audit-#12 [`commitment_message_with_status`] signed object never includes
/// any holder key, so the issuer-attested credential is a pure BEARER credential:
/// anyone presenting the manifest is treated as the holder, and the
/// proof-of-possession ([`holder_pop_message`]) can only be checked against an
/// EXTERNAL holder registry the issuer never bound to this credential (the G1 gap
/// in `bind_holder_pop`). By folding `holder_pk_digest` into the SAME Schnorr
/// signature that already binds `C(G)`, the salt, and the status reference, the
/// holder key is bound BY THE ISSUER AT MINT — a presenter cannot substitute its
/// own key without invalidating the issuer signature.
///
/// # Purely additive — back-compatible
/// This adds a NEW message shape; it does not change [`commitment_message_with_status`]
/// or any existing signature. A credential issued without holder binding (signed
/// over the audit-#12 status message) remains a valid bearer credential — its
/// signature still verifies under [`verify`] over that older message. Credentials
/// minted before holder binding existed carry no attested digest, so the verifier
/// treats them as BEARER — accepted on the sq-cwq registry + nonce-PoP path under
/// the back-compatible default policy, rejected outright once the relying party
/// opts into `HolderBindingPolicy::require_binding()`. Re-issuance is the migration
/// path; an absent binding is never read as a satisfied one.
///
/// # End-to-end today (the enforcement side, for orientation)
/// The fail-CLOSED enforcement this message enables now exists in
/// `sparq-zk-compose`: `manifest::AttestedHolderBinding` carries the attested
/// digest (T2/sq-h8rg), `verifier::bind_holder_pop` cross-checks a DISCLOSED holder
/// key against it (B1, T3/sq-z8s7 — `HolderKeyMismatch` / `HolderBindingMissing`
/// under `HolderBindingPolicy::require_binding()`), and `verifier::bind_holder_pok`
/// binds an in-circuit `holder_pok` proof to the same digest so the key stays
/// HIDDEN (B2, T5/sq-xqfg + T6/sq-i1dt, opt in with
/// `HolderBindingPolicy::require_in_circuit_pok()`). Research-grade, NOT externally
/// audited (sq-qhy4) — no soundness or privacy property is claimed as achieved.
///
/// Issuer, verifier, and the B2 circuit all recompute this message identically from
/// the disclosed `(C(G), salt, status_ref, holder_pk_digest)`, so a drift cannot
/// make a wrong holder binding verify (single source of truth, matching the
/// audit-#12 message family).
// [OPUS-4.8] sq-y464 (HolderPoP T1): holder-bound commitment message (new ZKSIG_C4 tag).
// [OPUS-5] sq-sg37 (HolderPoP T8): T2/T3/T5/T6 deferral notes retired — all landed.
const SIG_DOMAIN_COMMITMENT_HOLDER: u64 = 0x5a4b_5349_475f_4334; // "ZKSIG_C4"
pub fn commitment_message_with_holder(
    commitment: &Fr,
    salt: &Fr,
    status_ref: &Fr,
    holder_pk_digest: &Fr,
) -> Fr {
    poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_COMMITMENT_HOLDER),
        *commitment,
        *salt,
        *status_ref,
        *holder_pk_digest,
    ])
}

/// The signed message for a HOLDER proof-of-possession (sq-cwq): a
/// domain-separated binding of the verifier's freshness `challenge` (its nonce,
/// the same field element that is public-input field 0 of every circuit member).
/// The HOLDER signs THIS value with its holder secret key; the verifier checks the
/// signature under the disclosed holder public key, proving the presenter
/// POSSESSES the holder secret AND did so freshly over the verifier's nonce (so a
/// captured manifest cannot be replayed by a party that does not hold the key —
/// the PoP is bound to the relying party's fresh challenge).
///
/// # Scope (honest)
/// This message binds possession of the holder KEY to the verifier's challenge. It
/// does NOT, on its own, bind that key to a SPECIFIC credential — that is the job of
/// the issuer-attested holder binding ([`commitment_message_with_holder`] /
/// [`SecretKey::sign_commitment_with_holder`], sq-y464), which is no longer a
/// deferral: `sparq_zk_compose::verifier::bind_holder_pop` runs BOTH checks, so the
/// combined `HolderPop` gate proves "possession, freshly, of the key THIS issuer
/// bound to THIS credential" and rejects trusted holder A presenting trusted holder
/// B's credential. The relying party still anchors the holder key in an EXTERNAL
/// holder registry (mirroring the issuer key-set `K`), so an absent/untrusted/forged
/// PoP fails closed.
// [OPUS-4.8] sq-cwq: holder proof-of-possession message (challenge-bound).
// [OPUS-5] sq-sg37 (HolderPoP T8): deferral note retired — the binding is enforced.
pub fn holder_pop_message(challenge: &Fr) -> Fr {
    const SIG_DOMAIN_HOLDER_POP: u64 = 0x5a4b_5349_475f_4850; // "ZKSIG_HP"
    poseidon2::hash(&[Fr::from(SIG_DOMAIN_HOLDER_POP), *challenge])
}

impl SecretKey {
    /// Produce a holder proof-of-possession over the verifier's `challenge`
    /// (sq-cwq), returning the signature hex (`compressed(R) ‖ s`). The signed
    /// message is [`holder_pop_message`], matching the verifier-side
    /// `bind_holder_pop` check. Same deterministic `(sk, m)` nonce discipline as
    /// [`Self::sign_commitment`] (no signing-time entropy, seed-reuse-proof).
    // [OPUS-4.8] sq-cwq: holder-side PoP signing.
    pub fn sign_holder_pop(&self, challenge: &Fr) -> String {
        let sig = sign_deterministic(self, &holder_pop_message(challenge));
        signature_to_hex(&sig)
    }
}

/// Sign a message field element `m` with `sk`, drawing the nonce `k` from `rng`.
///
/// # Test-only (codex job 2216 MEDIUM)
/// This random-nonce path is `#[cfg(test)]` and `pub(crate)`: no public API takes
/// a caller-supplied RNG/seed. A reused/cloned RNG state would reuse a Schnorr
/// nonce across two distinct messages, yielding two signatures with the same
/// `R = k·G`, from which `sk = (s1 - s2)/(e1 - e2)` is trivially recovered. All
/// issuance now goes through the deterministic `(sk, m)`-nonce path
/// ([`sign_deterministic`] / [`SecretKey::sign_commitment`]); this remains only
/// to exercise [`verify`] against an independently-nonce'd honest signature in
/// this crate's own tests.
// [OPUS-4.8] codex 2216 MEDIUM: random-nonce signer is test-only; no public caller-RNG API.
#[cfg(test)]
pub(crate) fn sign<R: ark_std::rand::RngCore + ark_std::rand::CryptoRng>(
    sk: &SecretKey,
    m: &Fr,
    rng: &mut R,
) -> Signature {
    let g = EdwardsProjective::generator();
    let k = JjScalar::rand(rng);
    // [OPUS-5] sq-j3b9: this test-only signer deliberately KEEPS the arkworks
    // multiplication. `sk.public_key()` below now goes through the ladder, so
    // every `sign` → `verify` test in this module cross-checks a ladder-derived
    // public key against an arkworks-derived `R` — an independent-path check the
    // suite would lose if both sides used the same implementation.
    let r_pt = (g * k).into_affine();
    let pk = sk.public_key().0;
    let e = challenge(&r_pt, &pk, m);
    let s = k + e * sk.0;
    Signature { r: r_pt, s }
}

/// Verify a Schnorr signature: `s·G == R + e·pk`. Returns `false` (never
/// panics) on any malformed input. This is the verifier-side gate audit #3
/// requires — the relying party resolves `pk` from the disclosed key-set `K`,
/// recomputes `e`, and checks this equation over the commitment message.
pub fn verify(pk: &PublicKey, m: &Fr, sig: &Signature) -> bool {
    // [OPUS-4.8] codex #3: reject the IDENTITY public key. The identity point is
    // on-curve and in-subgroup, but `e·pk = 0` for it, so the verification
    // equation collapses to `s·G == R`; a forger picks any `s`, sets `R = s·G`,
    // and a signature verifies for the identity key under ANY message. Rejecting
    // `pk.0.is_zero()` here (and in `public_key_from_hex`, fail-closed at parse
    // time) closes that universal forgery.
    if pk.0.is_zero() {
        return false;
    }
    // Reject the off-curve / non-prime-order points defensively: `R` and `pk`
    // must be on the curve and in the prime-order subgroup, else a small-subgroup
    // point could let a forger pass the equation.
    if !sig.r.is_on_curve()
        || !pk.0.is_on_curve()
        || !sig.r.is_in_correct_subgroup_assuming_on_curve()
        || !pk.0.is_in_correct_subgroup_assuming_on_curve()
    {
        return false;
    }
    let g = EdwardsProjective::generator();
    let e = challenge(&sig.r, &pk.0, m);
    // lhs = s·G ; rhs = R + e·pk
    let lhs = g * sig.s;
    let rhs = sig.r.into_group() + pk.0 * e;
    lhs == rhs
}

// --- in-circuit witness bridge (sq-z9l) -----------------------------------
// [OPUS-4.8] sq-z9l: the witness an in-circuit Schnorr verifier needs. The
// in-circuit gadget (`zk/compose/compose_core/src/issuer.nr`) verifies
// `s*G == R + e*pk` over Baby-JubJub with point coordinates in the BASE field
// (= Noir `Field` = `crate::field::Fr`) and the SCALAR `e` reduced mod the
// curve's scalar order `L`. This bridge produces exactly those field-element
// witnesses from a `(PublicKey, Signature, message)` triple so the host can
// render the circuit's `Prover.toml`.

impl PublicKey {
    /// The issuer key's affine coordinates as base-field [`Fr`] elements (= Noir
    /// `Field`). `None` for the identity (no affine coordinates) — which is never
    /// a usable key (rejected at parse/verify, see [`verify`]). The in-circuit
    /// gadget and the key-set Merkle leaf both work over these coordinates.
    // [OPUS-4.8] sq-z9l.
    pub fn coords(&self) -> Option<(Fr, Fr)> {
        self.0.xy()
    }
}

/// The key-set Merkle leaf for an issuer public key: `Poseidon2([pk.x, pk.y])`.
/// Mirrors `issuer.nr`'s `key_leaf` (`h2(pk.x, pk.y)`) bit-for-bit. `None` for
/// the identity key (no coordinates). The relying party commits its authoritative
/// KeySet K with this leaf, and the in-circuit membership proof folds it to the
/// public root.
// [OPUS-4.8] sq-z9l: key-set Merkle leaf (host mirror of issuer.nr::key_leaf).
pub fn key_set_leaf(pk: &PublicKey) -> Option<Fr> {
    // [OPUS-4.8] sq-l15mi (audit H-1): never commit a torsion / low-order /
    // off-curve key into the key set `K`. `PublicKey`'s inner field is `pub`, so a
    // key can be constructed around such a point directly (bypassing
    // `public_key_from_hex`); binding it into `K` would let the hidden-issuer
    // circuit accept a forgeable torsion key. Fail closed unless prime-order.
    if !pk.is_prime_order() {
        return None;
    }
    let (x, y) = pk.coords()?;
    Some(poseidon2::hash(&[x, y]))
}

/// The Baby-JubJub scalar-field order `L` as a base-field element (`< q_base`,
/// so the embedding is injective). Mirrors `issuer.nr`'s `BJJ_L`.
fn scalar_order_in_base() -> Fr {
    // `JjScalar::MODULUS` big-endian, reduced into the base field. `L < q_base`,
    // so this is the exact value, not a reduction.
    Fr::from_be_bytes_mod_order(&JjScalar::MODULUS.to_bytes_be())
}

/// Map a Baby-JubJub scalar into the base field via its canonical big-endian
/// bytes. `L < q_base`, so a canonical scalar (`< L`) embeds injectively — the
/// resulting `Fr` is the same integer, suitable as a circuit `Field` witness.
fn jj_scalar_to_base(s: &JjScalar) -> Fr {
    Fr::from_be_bytes_mod_order(&s.into_bigint().to_bytes_be())
}

/// The in-circuit witness for one hidden issuer attestation: the affine point
/// coordinates of the issuer key and the signature's `R`, the signature scalar
/// `s`, and the challenge reduction `(e, e_k)` such that
/// `e_base = e + e_k * L` with `e < L` and `e_k < 8` (the soundness binding the
/// circuit re-checks). All as base-field [`Fr`] elements (= Noir `Field`).
///
/// [OPUS-4.8] sq-ru0yx (internal re-audit M-1): the circuit's reduction binding
/// ALSO enforces a no-wrap bound (`issuer.nr::challenge_scalar` step 4) — when
/// `e_k == 7` it requires `e < q_base - 7*L`, closing the field-wrap alternative
/// `(e_base + q_base - 7*L, 7)` that the bare `e + e_k*L == e_base` identity would
/// otherwise admit. This honest witness (`e = e_base mod L`, `e_k = floor(e_base /
/// L)`) always satisfies it, so the host helper is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InCircuitSchnorrWitness {
    /// Issuer public-key affine coordinates.
    pub pk_x: Fr,
    pub pk_y: Fr,
    /// Signature nonce-commitment `R` affine coordinates.
    pub r_x: Fr,
    pub r_y: Fr,
    /// Signature scalar `s` (canonical, `< L`), embedded in the base field.
    pub s: Fr,
    /// Reduced challenge scalar `e = e_base mod L` (`< L`).
    pub e: Fr,
    /// Reduction quotient `k` such that `e_base = e + k * L` (`< 8`).
    pub e_k: Fr,
}

/// Build the [`InCircuitSchnorrWitness`] for `(pk, sig)` over the signed message
/// field element `m` (e.g. [`commitment_message_with_status`]'s output — the
/// SAME `Fr` the issuer signed and the circuit recomputes the challenge over).
///
/// Returns `None` if the inputs are degenerate (an identity key or a point with
/// no affine coordinates), which the in-circuit gadget would reject anyway. This
/// is an ISSUANCE/PROVER-SIDE helper: it does not itself verify the signature
/// (the circuit does), it only lays out the witness fields. Callers that want a
/// sanity check should also call [`verify`].
pub fn in_circuit_witness(pk: &PublicKey, m: &Fr, sig: &Signature) -> Option<InCircuitSchnorrWitness> {
    // [OPUS-4.8] sq-l15mi (audit H-1): reject a torsion / low-order / off-curve key
    // before laying out a circuit witness. The in-circuit `schnorr_verify` now
    // enforces `[L]·pk == O`, so a torsion-key witness would be unprovable; refuse
    // to emit it here (fail-closed; subsumes the earlier identity-only `is_zero`
    // guard). (`R` is checked in-circuit; an honest `R = k·G` is already in-subgroup.)
    if !pk.is_prime_order() {
        return None;
    }
    let (pk_x, pk_y) = pk.0.xy()?;
    let (r_x, r_y) = sig.r.xy()?;
    // The base-field Poseidon2 challenge digest, BEFORE the scalar-field
    // reduction (the in-circuit `e_base`). Recompute it from the SAME inputs
    // `challenge` uses, but keep the base-field value (challenge() returns the
    // reduced JjScalar; here we need both halves of the reduction).
    let e_base: Fr = poseidon2::hash(&[
        Fr::from(SIG_DOMAIN_COMMITMENT),
        r_x,
        r_y,
        pk_x,
        pk_y,
        *m,
    ]);
    // Reduce e_base into the scalar field, then re-embed the reduced value and
    // derive the quotient k = (e_base - e) / L (exact in the integers).
    let e_scalar = JjScalar::from_be_bytes_mod_order(&e_base.into_bigint().to_bytes_be());
    let e = jj_scalar_to_base(&e_scalar);
    let l = scalar_order_in_base();
    // e_k = (e_base - e) / L. e_base, e, L are all canonical base-field values
    // representing the true integers (e_base, e < q_base; e < L; e_base = e + kL
    // with k < 8), so the base-field subtraction/division gives the exact small
    // integer quotient (no wraparound: e_base >= e since e = e_base mod L).
    let e_k = (e_base - e) / l;
    Some(InCircuitSchnorrWitness {
        pk_x,
        pk_y,
        r_x,
        r_y,
        s: jj_scalar_to_base(&sig.s),
        e,
        e_k,
    })
}

/// The in-circuit witness for a HOLDER Proof-of-Possession (sq-xqfg, HolderPoP
/// T5 / design §2.B/B2): the holder secret `hsk` mapped into the base field, the
/// holder public key `hpk = hsk·G` affine coordinates, and the issuer-attested
/// holder-key digest `holder_pk_digest = Poseidon2([ZKSIG_HK, hpk.x, hpk.y])`
/// ([`holder_key_digest`]). These are exactly the inputs the `holder_pok` circuit
/// member proves over: `hsk` and `(hpk_x, hpk_y)` are its PRIVATE witness, and
/// `holder_pk_digest` is the PUBLIC input the verifier (T6/sq-i1dt) binds to the
/// issuer attestation. All as base-field [`Fr`] elements (= Noir `Field`).
///
/// The `hsk` embedding mirrors the issuer-witness scalar embedding
/// ([`in_circuit_witness`]'s `s`/`e`): a canonical Baby-JubJub scalar (`< L`)
/// maps injectively into the larger base field via its big-endian bytes, so the
/// in-circuit `hsk·G` is computed over the same integer the holder signs with.
// [OPUS-4.8] sq-xqfg (HolderPoP T5): in-circuit holder-PoK witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InCircuitHolderPokWitness {
    /// Holder secret scalar `hsk` (canonical, `< L`), embedded in the base field.
    pub hsk: Fr,
    /// Holder public-key `hpk = hsk·G` affine coordinates.
    pub hpk_x: Fr,
    pub hpk_y: Fr,
    /// The holder-key digest `Poseidon2([ZKSIG_HK, hpk.x, hpk.y])` — the PUBLIC
    /// input the verifier binds to the issuer attestation (single source of truth
    /// with [`holder_key_digest`] and the in-circuit `holder::holder_key_digest`).
    pub holder_pk_digest: Fr,
}

/// Build the [`InCircuitHolderPokWitness`] for the holder key pair `hsk` (and its
/// derived `hpk = hsk·G`). Returns `None` if the public key is the identity (no
/// affine coordinates) — which the in-circuit gadget rejects anyway (fail-closed).
///
/// ISSUANCE/PROVER-SIDE helper: it lays out the witness fields the `holder_pok`
/// member proves over; it does not itself prove anything (the circuit does). The
/// digest it returns is exactly [`holder_key_digest`]`(hsk.public_key())`, so the
/// host and circuit agree bit-for-bit (the cross-check T6 relies on).
// [OPUS-4.8] sq-xqfg (HolderPoP T5).
pub fn in_circuit_holder_witness(hsk: &SecretKey) -> Option<InCircuitHolderPokWitness> {
    let hpk = hsk.public_key();
    let (hpk_x, hpk_y) = hpk.coords()?;
    let digest = holder_key_digest(&hpk).ok()?;
    Some(InCircuitHolderPokWitness {
        hsk: jj_scalar_to_base(&hsk.0),
        hpk_x,
        hpk_y,
        holder_pk_digest: digest,
    })
}

// --- serialization (registry literals + manifest fields) ------------------

/// Serialize a public key to lowercase hex (compressed Baby-JubJub point). The
/// `zk:issuerKey` registry literal and the manifest key-set carry this.
pub fn public_key_to_hex(pk: &PublicKey) -> String {
    let mut bytes = Vec::new();
    pk.0
        .serialize_compressed(&mut bytes)
        .expect("affine point serializes");
    to_hex(&bytes)
}

/// Parse a public key from hex. `None` on malformed/odd hex or a point that is
/// not a valid compressed Baby-JubJub point (fail-closed).
pub fn public_key_from_hex(s: &str) -> Option<PublicKey> {
    let bytes = from_hex(s)?;
    let pt = Affine::<EdwardsConfig>::deserialize_compressed(&bytes[..]).ok()?;
    let pk = PublicKey(pt);
    // [OPUS-4.8] codex #3 + sq-l15mi (audit H-1): fail-closed unless the key is a
    // genuine prime-order, non-identity, on-curve point. A signature is universally
    // forgeable for the identity key (e·pk = 0), and a torsion / low-order key
    // (Baby-JubJub cofactor 8) admits a no-secret forgery on the hidden-issuer path
    // — neither may ever enter a key-set K or an attestation. ark's
    // `deserialize_compressed` (`Validate::Yes`) already rejects off-curve AND
    // non-prime-order-subgroup bytes, so a torsion key cannot reach here via hex
    // today; this EXPLICIT gate does not rely on that ark default (a future switch
    // to `deserialize_compressed_unchecked` / `Validate::No` for speed would
    // silently reintroduce the hole). It subsumes the earlier identity-only reject.
    if !pk.is_prime_order() {
        return None;
    }
    Some(pk)
}

/// Serialize a signature to hex: `compressed(R) ‖ scalar(s)` (each
/// canonical-serialized, concatenated).
pub fn signature_to_hex(sig: &Signature) -> String {
    let mut bytes = Vec::new();
    sig.r
        .serialize_compressed(&mut bytes)
        .expect("R serializes");
    sig.s
        .serialize_compressed(&mut bytes)
        .expect("s serializes");
    to_hex(&bytes)
}

/// Parse a signature from hex (`compressed(R) ‖ scalar(s)`). `None` on any
/// malformed input (fail-closed — prover-controlled bytes never panic).
pub fn signature_from_hex(s: &str) -> Option<Signature> {
    let bytes = from_hex(s)?;
    // The compressed Edwards point is 32 bytes; the scalar is the remainder.
    if bytes.len() < 32 {
        return None;
    }
    let (r_bytes, s_bytes) = bytes.split_at(32);
    let r = Affine::<EdwardsConfig>::deserialize_compressed(r_bytes).ok()?;
    let s_scalar = JjScalar::deserialize_compressed(s_bytes).ok()?;
    Some(Signature { r, s: s_scalar })
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    // [OPUS-4.8] sq-hbg7: stable-1.96 clippy `manual_is_multiple_of`.
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_std::rand::SeedableRng;

    fn rng() -> ark_std::rand::rngs::StdRng {
        ark_std::rand::rngs::StdRng::seed_from_u64(42)
    }

    fn keypair(seed: u64) -> (SecretKey, PublicKey) {
        let mut r = ark_std::rand::rngs::StdRng::seed_from_u64(seed);
        let sk = SecretKey(JjScalar::rand(&mut r));
        let pk = sk.public_key();
        (sk, pk)
    }

    #[test]
    fn sign_verify_round_trip() {
        let (sk, pk) = keypair(1);
        let c = Fr::from(0x1234u64);
        let m = commitment_message(&c);
        let sig = sign(&sk, &m, &mut rng());
        assert!(verify(&pk, &m, &sig), "honest signature must verify");
    }

    // [OPUS-4.8] sq-u8a8: the constant-time secret-key equality returns the SAME
    // boolean as a value equality on the underlying scalars — for an equal pair
    // (same seed) and an unequal pair. This is the "CT-eq agrees with the prior
    // `==`" gate; nothing in the protocol calls `ct_eq` today (the verifier path
    // is over public data), so this is the only exercise of the defensive
    // primitive.
    #[test]
    fn secret_key_ct_eq_agrees_with_scalar_eq() {
        let (sk_a, _) = keypair(7);
        let (sk_a2, _) = keypair(7); // same seed → same scalar
        let (sk_b, _) = keypair(8); // different seed → different scalar
        let eq_same: bool = sk_a.ct_eq(&sk_a2).into();
        let eq_diff: bool = sk_a.ct_eq(&sk_b).into();
        assert!(
            eq_same,
            "ct_eq must be true for two keys with the same scalar"
        );
        assert!(
            !eq_diff,
            "ct_eq must be false for keys with different scalars"
        );
        // Cross-check against the scalar value equality (the prior-`==` semantics).
        assert_eq!(eq_same, sk_a.0 == sk_a2.0);
        assert_eq!(eq_diff, sk_a.0 == sk_b.0);
    }

    // [OPUS-5] sq-j3b9: routing the two SECRET-scalar multiplications through the
    // fixed-width `crate::ct::mul_ct` ladder must be a pure timing-shape change —
    // every derived key and every signature has to be BYTE-IDENTICAL to what the
    // arkworks variable-time multiplication produced. This pins both halves
    // against the reference implementation directly (the `crate::ct` differential
    // covers the ladder in isolation; this covers the call sites), so a drift
    // that silently re-keyed an issuer or invalidated already-issued credentials
    // fails here.
    #[test]
    fn ct_ladder_call_sites_are_value_identical_to_arkworks() {
        let g = EdwardsProjective::generator();
        for seed in [0u64, 1, 7, 4242] {
            let (sk, pk) = keypair(seed);
            // pk = sk·G must match the arkworks reference exactly.
            assert_eq!(
                pk.0,
                (g * sk.0).into_affine(),
                "ladder-derived public key must equal the arkworks reference"
            );

            // R = G·k must match too, for the real deterministic nonce.
            let m = commitment_message(&Fr::from(seed));
            let sig = sign_deterministic(&sk, &m);
            let k = derive_nonce(&sk, &m);
            assert_eq!(
                sig.r,
                (g * k).into_affine(),
                "ladder-derived nonce commitment must equal the arkworks reference"
            );
            // ...and the signature the ladder produced still verifies.
            assert!(verify(&pk, &m, &sig), "ladder-signed message must verify");
        }
    }

    // [OPUS-4.8] sq-u8a8: compile + behaviour check that `SecretKey` implements
    // `Zeroize` and that scrubbing resets the scalar to zero. (A `SecretKey` also
    // zeroizes on `Drop`; this asserts the scrub itself, which `Drop` calls.)
    #[test]
    fn secret_key_zeroizes() {
        use ark_ff::Zero;
        use zeroize::Zeroize;
        let (mut sk, _) = keypair(9);
        assert!(!sk.0.is_zero(), "fresh key is non-zero");
        sk.zeroize();
        assert!(sk.0.is_zero(), "zeroized SecretKey scalar must be zero");
    }

    #[test]
    fn wrong_message_rejected() {
        // The truncated-leaf-suppression shape: a signature over C(G) must NOT
        // verify against C(G') for a different (truncated) graph commitment.
        let (sk, pk) = keypair(2);
        let m = commitment_message(&Fr::from(100u64));
        let sig = sign(&sk, &m, &mut rng());
        let m_other = commitment_message(&Fr::from(101u64));
        assert!(!verify(&pk, &m_other, &sig), "sig over a different commitment must fail");
    }

    #[test]
    fn wrong_key_rejected() {
        // The key-not-in-K shape: a signature by issuer A must not verify under
        // issuer B's key.
        let (sk_a, _pk_a) = keypair(3);
        let (_sk_b, pk_b) = keypair(4);
        let m = commitment_message(&Fr::from(7u64));
        let sig = sign(&sk_a, &m, &mut rng());
        assert!(!verify(&pk_b, &m, &sig), "sig under a different key must fail");
    }

    #[test]
    fn tampered_signature_rejected() {
        let (sk, pk) = keypair(5);
        let m = commitment_message(&Fr::from(9u64));
        let mut sig = sign(&sk, &m, &mut rng());
        sig.s += JjScalar::from(1u64); // tamper s
        assert!(!verify(&pk, &m, &sig), "tampered s must fail");
    }

    #[test]
    fn pubkey_hex_round_trip() {
        let (_sk, pk) = keypair(6);
        let h = public_key_to_hex(&pk);
        assert_eq!(public_key_from_hex(&h), Some(pk));
        assert_eq!(public_key_from_hex(&format!("0x{h}")), Some(pk));
        assert!(public_key_from_hex("zz").is_none());
        assert!(public_key_from_hex("abc").is_none()); // odd length
    }

    #[test]
    fn signature_hex_round_trip() {
        let (sk, pk) = keypair(7);
        let m = commitment_message(&Fr::from(55u64));
        let sig = sign(&sk, &m, &mut rng());
        let h = signature_to_hex(&sig);
        let back = signature_from_hex(&h).expect("round-trips");
        assert!(verify(&pk, &m, &back), "deserialized sig verifies");
        assert!(signature_from_hex("zz").is_none());
        assert!(signature_from_hex("00").is_none()); // too short for R
    }

    #[test]
    fn cryptosuite_iri_round_trip() {
        let s = SignatureScheme::Poseidon2SchnorrV1;
        assert_eq!(
            SignatureScheme::from_cryptosuite_iri(s.cryptosuite_iri()),
            Some(s)
        );
        assert_eq!(SignatureScheme::from_cryptosuite_iri("urn:other"), None);
    }

    // --- audit #3 codex #3: identity-key forgery rejection ----------------

    /// [OPUS-4.8] codex #3: a signature is universally forgeable for the IDENTITY
    /// public key (`e·pk = 0` ⇒ the equation is `s·G == R`, satisfied by any `s`
    /// with `R = s·G`). Such a forgery — valid for ANY message — MUST be rejected
    /// by `verify`. Before the fix this returned `true`.
    #[test]
    fn identity_key_forgery_rejected() {
        // The identity / neutral point of Baby-JubJub.
        let id_pk = PublicKey(Affine::<EdwardsConfig>::zero());
        assert!(id_pk.0.is_zero(), "constructed the identity point");
        // Forge: pick any s, set R = s·G. Then s·G == R + e·0 holds for all m.
        let s = JjScalar::from(123456u64);
        let r = (EdwardsProjective::generator() * s).into_affine();
        let forged = Signature { r, s };
        let m = commitment_message(&Fr::from(42u64));
        assert!(
            !verify(&id_pk, &m, &forged),
            "identity-key signature must be rejected (universal forgery)"
        );
        // And it must never be admissible from hex either (fail-closed at parse).
        let id_hex = {
            let mut b = Vec::new();
            Affine::<EdwardsConfig>::zero()
                .serialize_compressed(&mut b)
                .unwrap();
            to_hex(&b)
        };
        assert!(
            public_key_from_hex(&id_hex).is_none(),
            "identity key must not parse into a usable PublicKey"
        );
    }

    /// [OPUS-4.8] sq-l15mi (audit H-1): a torsion / low-order Baby-JubJub key
    /// (cofactor 8) must be rejected at every host boundary a key could enter a
    /// key-set `K` or a circuit witness through. The order-2 point `(0, -1)` is
    /// on-curve, non-identity, and NOT in the prime-order subgroup, so it slips past
    /// the on-curve + identity guards — only the prime-order-subgroup check catches
    /// it. Before this fix, `key_set_leaf` / `in_circuit_witness` returned `Some`
    /// for it (a torsion key could be committed into `K`). The paired accept case
    /// pins that a genuine `sk·G` key is unaffected (non-vacuity in both directions).
    #[test]
    fn torsion_key_rejected_from_key_set_and_witness() {
        use ark_ff::One;
        // Baby-JubJub order-2 torsion point (0, -1): on-curve, non-identity, NOT in
        // the prime-order subgroup. Constructed directly (the `pub` inner-field
        // vector `public_key_from_hex` cannot reach, since ark's Validate::Yes
        // rejects it at deserialize).
        let torsion_pt = Affine::<EdwardsConfig>::new_unchecked(Fr::zero(), -Fr::one());
        assert!(torsion_pt.is_on_curve(), "(0,-1) is on the curve");
        assert!(!torsion_pt.is_zero(), "(0,-1) is not the identity");
        assert!(
            !torsion_pt.is_in_correct_subgroup_assuming_on_curve(),
            "(0,-1) is an order-2 torsion point, not in the prime-order subgroup"
        );
        let torsion = PublicKey(torsion_pt);
        assert!(!torsion.is_prime_order(), "torsion key must not be prime-order");

        // Soundness gates: a torsion key cannot enter K or a circuit witness.
        assert!(
            key_set_leaf(&torsion).is_none(),
            "torsion key must not commit a key-set leaf"
        );
        let c = Fr::from(0xc0ffeeu64);
        let m = commitment_message(&c);
        let sig = signature_from_hex(&SecretKey::from_seed(9).sign_commitment(&c))
            .expect("signature round-trips");
        assert!(
            in_circuit_witness(&torsion, &m, &sig).is_none(),
            "no in-circuit witness may be laid out for a torsion key"
        );
        // Boundary: the hex form is fail-closed too (ark Validate::Yes at deserialize
        // + the explicit is_prime_order gate — belt and suspenders).
        let torsion_hex = public_key_to_hex(&torsion);
        assert!(
            public_key_from_hex(&torsion_hex).is_none(),
            "torsion key must not parse into a usable PublicKey"
        );

        // Non-vacuity (accept direction): a genuine prime-order key is unaffected.
        let good = SecretKey::from_seed(9).public_key();
        assert!(good.is_prime_order(), "a genuine sk·G key is prime-order");
        assert!(
            key_set_leaf(&good).is_some(),
            "a valid key must still commit a key-set leaf"
        );
        assert!(
            in_circuit_witness(&good, &m, &sig).is_some(),
            "a valid key must still yield an in-circuit witness"
        );
    }

    // --- audit #3 codex #4: deterministic nonce (no caller seed) ----------

    /// [OPUS-4.8] codex #4: `sign_commitment` derives the nonce from `(sk, m)`,
    /// so it is deterministic (replay-stable) and seed-reuse is impossible by
    /// construction. The signature verifies, and re-signing the same commitment
    /// yields byte-identical output.
    #[test]
    fn deterministic_sign_commitment_round_trip_and_stable() {
        let sk = SecretKey::from_seed(11);
        let pk = sk.public_key();
        let c = Fr::from(0xc0ffeeu64);
        let h1 = sk.sign_commitment(&c);
        let h2 = sk.sign_commitment(&c);
        assert_eq!(h1, h2, "deterministic signing must be replay-stable");
        let sig = signature_from_hex(&h1).expect("round-trips");
        assert!(
            verify(&pk, &commitment_message(&c), &sig),
            "deterministic signature must verify"
        );
    }

    /// [OPUS-4.8] sq-8jv7: the branchless degenerate-`k` guard in `derive_nonce`
    /// is BEHAVIOURALLY IDENTICAL to the old `if k.is_zero()` select. In the
    /// non-degenerate case (overwhelmingly the only reachable case — `k == 0`
    /// has probability ~2^-251) the nonce equals the first PRF fold, so the
    /// signature is byte-stable and verifies. This pins that the constant-time
    /// rewrite changed no value the signer produces (the protocol is identical;
    /// only our emitted control flow is now data-independent of the secret).
    #[test]
    fn branchless_nonce_matches_first_fold_and_signs_stably() {
        const SIG_DOMAIN_NONCE: u64 = 0x5a4b_5349_475f_4e31; // mirror derive_nonce
        let sk = SecretKey::from_seed(77);
        let m = commitment_message(&Fr::from(0xabcdu64));
        // Recompute the first PRF fold exactly as derive_nonce does.
        let sk_base = Fr::from_be_bytes_mod_order(&sk.0.into_bigint().to_bytes_be());
        let k_base: Fr = poseidon2::hash(&[Fr::from(SIG_DOMAIN_NONCE), sk_base, m]);
        let k_first = JjScalar::from_be_bytes_mod_order(&k_base.into_bigint().to_bytes_be());
        assert!(!k_first.is_zero(), "first fold is non-degenerate for this seed");
        assert_eq!(
            super::derive_nonce(&sk, &m),
            k_first,
            "branchless guard must return the first fold when k != 0"
        );
        // And the produced signature is replay-stable + verifies (no value drift).
        let pk = sk.public_key();
        let h1 = sk.sign_commitment(&Fr::from(0xabcdu64));
        let h2 = sk.sign_commitment(&Fr::from(0xabcdu64));
        assert_eq!(h1, h2, "deterministic signing remains replay-stable");
        let sig = signature_from_hex(&h1).expect("round-trips");
        assert!(verify(&pk, &m, &sig), "signature still verifies after CT rewrite");
    }

    /// [OPUS-4.8] codex #4: distinct messages get DISTINCT nonces `R` — the
    /// property that makes seed-reuse key-extraction impossible (the old seeded
    /// API let a caller force `R1 == R2` across messages, leaking `sk`).
    #[test]
    fn deterministic_nonce_differs_per_message() {
        let sk = SecretKey::from_seed(12);
        let r1 = sign_deterministic(&sk, &commitment_message(&Fr::from(1u64))).r;
        let r2 = sign_deterministic(&sk, &commitment_message(&Fr::from(2u64))).r;
        assert_ne!(r1, r2, "distinct messages must use distinct nonces R");
    }

    // --- audit #12: status-bound commitment message ----------------------

    /// [OPUS-4.8] audit #12: a status-bound signature verifies over the SAME
    /// `(C(G), salt, status_ref)` and is REJECTED if any of the three differ —
    /// the property that makes an omitted/forged/swapped status reference (a
    /// different version, index, or list id) unverifiable.
    #[test]
    fn status_bound_signature_binds_the_reference() {
        let sk = SecretKey::from_seed(21);
        let pk = sk.public_key();
        let c = Fr::from(0xc0ffeeu64);
        let salt = Fr::from(0x5a17u64);
        let list_id = status_list_id_to_field("https://dmv.example/status/3");
        let sref = status_ref_digest(&list_id, 94, 7);
        let hex = sk.sign_commitment_with_status(&c, &salt, &sref);
        let sig = signature_from_hex(&hex).expect("round-trips");
        let msg = commitment_message_with_status(&c, &salt, &sref);
        assert!(verify(&pk, &msg, &sig), "honest status-bound signature verifies");

        // A DIFFERENT version (freshness rollover) ⇒ different message ⇒ fails.
        let sref_v8 = status_ref_digest(&list_id, 94, 8);
        let msg_v8 = commitment_message_with_status(&c, &salt, &sref_v8);
        assert!(!verify(&pk, &msg_v8, &sig), "wrong version must not verify");
        // A DIFFERENT index ⇒ fails (cannot point at another credential's bit).
        let sref_idx = status_ref_digest(&list_id, 95, 7);
        let msg_idx = commitment_message_with_status(&c, &salt, &sref_idx);
        assert!(!verify(&pk, &msg_idx, &sig), "wrong index must not verify");
        // A DIFFERENT list id ⇒ fails (cannot swap to an attacker-controlled list).
        let other_list = status_list_id_to_field("https://attacker.example/empty");
        let sref_other = status_ref_digest(&other_list, 94, 7);
        let msg_other = commitment_message_with_status(&c, &salt, &sref_other);
        assert!(!verify(&pk, &msg_other, &sig), "wrong list id must not verify");
        // The bare salt-only message (the prover OMITTING the status ref) ⇒
        // fails: a status-bound credential cannot be presented as salt-only.
        let salt_only = commitment_message_with_salt(&c, &salt);
        assert!(
            !verify(&pk, &salt_only, &sig),
            "salt-only (status-omitted) message must not verify against a status-bound sig"
        );
    }

    /// [OPUS-4.8] sq-ayv: `status_index_commitment` is bit-identical to the
    /// in-circuit `h3(SIG_DOMAIN_STATUS_IDX_COMMIT, index, blinding)` the
    /// `revoke_unset_d{depth}` member recomputes. The cross-binding (the index
    /// proven unset == the index the issuer committed to) rests on both computing
    /// the SAME value, so this pins the cross-vector against the nargo output
    /// (compose_core/tests.nr `revoke_idx_commit_cross_vector`).
    #[test]
    fn status_index_commitment_matches_circuit_cross_vector() {
        let got = status_index_commitment(0, &Fr::from(0xb1u64));
        assert_eq!(
            crate::field::field_to_hex(&got),
            "0x204fe7eb53d68b4f2c7b981619ed4fbd5f3b99879e0f387761bf489a21456118",
            "host index commitment must equal the in-circuit h3(DOMAIN, 0, 0xb1)"
        );
    }

    /// [OPUS-4.8] sq-ayv: a COMMITTED-index status-bound signature binds the index
    /// COMMITMENT (not the clear index), is hiding (a different blinding => a
    /// different commitment for the same index), and the committed-digest domain
    /// is distinct from the clear-index digest (no substitution).
    #[test]
    fn committed_index_status_signature_binds_the_commitment() {
        let sk = SecretKey::from_seed(31);
        let pk = sk.public_key();
        let c = Fr::from(0xc0ffeeu64);
        let salt = Fr::from(0x5a17u64);
        let list_id = status_list_id_to_field("https://dmv.example/status/3");
        let blinding = Fr::from(0xbeefu64);
        let idx_commit = status_index_commitment(94, &blinding);
        let sref = status_ref_commit_digest(&list_id, &idx_commit, 7);
        let hex = sk.sign_commitment_with_status(&c, &salt, &sref);
        let sig = signature_from_hex(&hex).expect("round-trips");
        let msg = commitment_message_with_status(&c, &salt, &sref);
        assert!(verify(&pk, &msg, &sig), "honest committed-index signature verifies");

        // Hiding: the SAME index under a DIFFERENT blinding commits differently,
        // so two presentations of the same credential are not linkable by the
        // commitment (the clear-index linkability channel is closed).
        let idx_commit2 = status_index_commitment(94, &Fr::from(0x1234u64));
        assert_ne!(idx_commit, idx_commit2, "different blinding => different commitment");

        // A DIFFERENT committed index => different digest => signature fails.
        let other_commit = status_index_commitment(95, &blinding);
        let sref_other = status_ref_commit_digest(&list_id, &other_commit, 7);
        let msg_other = commitment_message_with_status(&c, &salt, &sref_other);
        assert!(!verify(&pk, &msg_other, &sig), "a different index commitment must not verify");

        // The CLEAR-index digest (audit #12) is domain-separated from the
        // committed-index one: even with a numerically-equal field input it
        // cannot be substituted (distinct domain tags).
        let clear = status_ref_digest(&list_id, 94, 7);
        let committed = status_ref_commit_digest(&list_id, &idx_commit, 7);
        assert_ne!(clear, committed, "clear-index and committed-index digests are domain-separated");
    }

    /// [OPUS-4.8] sq-6qe: the `(list, version)` reference commitment is HIDING
    /// (a different blinding => a different commitment for the same list+version,
    /// so two presentations are unlinkable on the reference) and BINDING (a
    /// different list OR a different version => a different commitment).
    #[test]
    fn status_ref_commitment_is_hiding_and_binding() {
        let list_a = status_list_id_to_field("https://dmv.example/status/3");
        let list_b = status_list_id_to_field("https://dmv.example/status/4");
        let rc = status_ref_commitment(&list_a, 7, &Fr::from(0xb11du64));

        // Hiding: same (list, version) under different blinding commits differently.
        let rc_other_blinding = status_ref_commitment(&list_a, 7, &Fr::from(0x1234u64));
        assert_ne!(rc, rc_other_blinding, "different blinding => different commitment");

        // Binding: a different version OR a different list changes the commitment.
        let rc_other_version = status_ref_commitment(&list_a, 8, &Fr::from(0xb11du64));
        assert_ne!(rc, rc_other_version, "different version => different commitment");
        let rc_other_list = status_ref_commitment(&list_b, 7, &Fr::from(0xb11du64));
        assert_ne!(rc, rc_other_list, "different list => different commitment");
    }

    /// [OPUS-4.8] sq-6qe: a FULLY-HIDDEN status-bound signature binds BOTH the
    /// reference commitment and the index commitment (neither list, version, nor
    /// index in the clear), and its digest domain is distinct from BOTH the
    /// clear-index (audit #12) and committed-index-clear-list (sq-ayv) digests, so
    /// no disclosure-mode substitution is possible.
    #[test]
    fn fully_committed_status_signature_binds_both_commitments() {
        let sk = SecretKey::from_seed(41);
        let pk = sk.public_key();
        let c = Fr::from(0xc0ffeeu64);
        let salt = Fr::from(0x5a17u64);
        let list_id = status_list_id_to_field("https://dmv.example/status/3");
        let ref_commit = status_ref_commitment(&list_id, 7, &Fr::from(0x5efb10u64));
        let idx_commit = status_index_commitment(94, &Fr::from(0xbeefu64));
        let sref = status_ref_fully_committed_digest(&ref_commit, &idx_commit);
        let hex = sk.sign_commitment_with_status(&c, &salt, &sref);
        let sig = signature_from_hex(&hex).expect("round-trips");
        let msg = commitment_message_with_status(&c, &salt, &sref);
        assert!(verify(&pk, &msg, &sig), "honest fully-hidden signature verifies");

        // A different reference commitment (e.g. a different list/version/blinding)
        // => different digest => signature fails.
        let ref_commit2 = status_ref_commitment(&list_id, 8, &Fr::from(0x5efb10u64));
        let sref2 = status_ref_fully_committed_digest(&ref_commit2, &idx_commit);
        let msg2 = commitment_message_with_status(&c, &salt, &sref2);
        assert!(!verify(&pk, &msg2, &sig), "a different reference commitment must not verify");

        // A different index commitment => different digest => signature fails.
        let idx_commit2 = status_index_commitment(95, &Fr::from(0xbeefu64));
        let sref3 = status_ref_fully_committed_digest(&ref_commit, &idx_commit2);
        let msg3 = commitment_message_with_status(&c, &salt, &sref3);
        assert!(!verify(&pk, &msg3, &sig), "a different index commitment must not verify");

        // Domain separation from the two other disclosure modes: even with
        // numerically-equal inputs the three digests differ (distinct domain tags).
        let clear = status_ref_digest(&list_id, 94, 7);
        let committed_clear_list = status_ref_commit_digest(&list_id, &idx_commit, 7);
        assert_ne!(sref, clear, "fully-hidden vs clear-index digests are domain-separated");
        assert_ne!(
            sref, committed_clear_list,
            "fully-hidden vs committed-index-clear-list digests are domain-separated"
        );
    }

    /// [OPUS-4.8] sq-6qe: the accepted `(list, version, status_list_root)`-set leaf
    /// binds all three atomically — any one differing changes the leaf, so a prover
    /// cannot pair one list's identity with another list's root (the sub-option-A
    /// soundness property). Also distinct from the reference commitment's domain.
    #[test]
    fn accepted_status_leaf_binds_triple_atomically() {
        let list_id = status_list_id_to_field("https://dmv.example/status/3");
        let root = Fr::from(0x12345678u64);
        let leaf = accepted_status_leaf(&list_id, 7, &root);

        let other_list = status_list_id_to_field("https://dmv.example/status/4");
        assert_ne!(leaf, accepted_status_leaf(&other_list, 7, &root), "list binds");
        assert_ne!(leaf, accepted_status_leaf(&list_id, 8, &root), "version binds");
        assert_ne!(
            leaf,
            accepted_status_leaf(&list_id, 7, &Fr::from(0x87654321u64)),
            "root binds"
        );

        // Distinct domain from the reference commitment (no leaf/commitment confusion).
        let rc = status_ref_commitment(&list_id, 7, &root); // same inputs, different role
        assert_ne!(leaf, rc, "accepted-set leaf and ref commitment are domain-separated");
    }

    /// [OPUS-4.8] audit #12: the list-id hash is collision-resistant on distinct
    /// IRIs and length-separated (a prefix cannot collide with a longer IRI).
    #[test]
    fn status_list_id_is_distinct_per_iri() {
        let a = status_list_id_to_field("https://dmv.example/status/3");
        let b = status_list_id_to_field("https://dmv.example/status/4");
        let c = status_list_id_to_field("https://dmv.example/status/3x");
        assert_ne!(a, b, "distinct lists hash distinctly");
        assert_ne!(a, c, "a prefix must not collide with a longer IRI");
    }

    // --- sq-z9l: in-circuit witness bridge --------------------------------

    use crate::field::field_to_hex;

    /// [OPUS-4.8] sq-z9l: the in-circuit witness reproduces the verify equation
    /// AND the challenge-reduction binding the circuit re-checks. This is the
    /// host/circuit contract: `s*G == R + e*pk` (in base-field coordinates) and
    /// `e_base == e + e_k*L` with `e < L`, `e_k < 8`.
    #[test]
    fn in_circuit_witness_is_internally_consistent() {
        let sk = SecretKey::from_seed(99);
        let pk = sk.public_key();
        let c = Fr::from(0xdecafu64);
        let m = commitment_message(&c);
        let sig = sign_deterministic(&sk, &m);
        assert!(verify(&pk, &m, &sig), "honest sig verifies (control)");

        let w = in_circuit_witness(&pk, &m, &sig).expect("witness");
        // The reduction binding the circuit asserts: e_base == e + e_k*L.
        let e_base: Fr = poseidon2::hash(&[
            Fr::from(SIG_DOMAIN_COMMITMENT),
            w.r_x,
            w.r_y,
            w.pk_x,
            w.pk_y,
            m,
        ]);
        let l = scalar_order_in_base();
        assert_eq!(w.e + w.e_k * l, e_base, "e_base == e + e_k*L");
        // e < L and e_k < 8 (3-bit).
        assert!(w.e < l, "e < L");
        assert!(w.e_k < Fr::from(8u64), "e_k < 8");

        // The witnessed e equals sig::challenge's reduced scalar, re-embedded.
        let e_ref = jj_scalar_to_base(&challenge(&sig.r, &pk.0, &m));
        assert_eq!(w.e, e_ref, "witnessed e == reduced challenge");

        // s*G == R + e*pk recomputed in the curve group from the witness scalars.
        // (Sanity that the base-field witness round-trips back to a valid eqn.)
        let g = EdwardsProjective::generator();
        let lhs = (g * sig.s).into_affine();
        let rhs = (sig.r.into_group() + pk.0 * challenge(&sig.r, &pk.0, &m)).into_affine();
        assert_eq!(lhs, rhs, "verify equation holds in-group (control)");
        assert_eq!(field_to_hex(&w.pk_x).len(), 66, "pk_x is a field hex");
    }

    /// [OPUS-4.8] sq-z9l: the witness bridge declines the identity key (the
    /// circuit rejects it; the bridge returns None rather than emitting a
    /// degenerate witness).
    #[test]
    fn in_circuit_witness_rejects_identity_key() {
        let id_pk = PublicKey(Affine::<EdwardsConfig>::zero());
        let m = commitment_message(&Fr::from(1u64));
        let s = JjScalar::from(7u64);
        let r = (EdwardsProjective::generator() * s).into_affine();
        let sig = Signature { r, s };
        assert!(in_circuit_witness(&id_pk, &m, &sig).is_none());
    }

    // --- sq-y464 (HolderPoP T1): holder-bound signed-message family ----------

    /// [OPUS-4.8] sq-y464 (HolderPoP T1): `commitment_message_with_holder` is
    /// DETERMINISTIC (same inputs => same message), DOMAIN-SEPARATED from the
    /// audit-#12 status message over the same `(C(G), salt, status_ref)` (a distinct
    /// `ZKSIG_C4` tag, so a holder-bound attestation can never be cross-substituted
    /// for a non-holder-bound one), and BINDS the holder digest (changing
    /// `holder_pk_digest` changes the message).
    #[test]
    fn holder_bound_message_is_deterministic_and_domain_separated() {
        let c = Fr::from(0xc0ffeeu64);
        let salt = Fr::from(0x5a17u64);
        let list_id = status_list_id_to_field("https://dmv.example/status/3");
        let sref = status_ref_digest(&list_id, 94, 7);
        let hpk = SecretKey::from_seed(201).public_key();
        let hdig = holder_key_digest(&hpk).expect("non-identity holder key digests");

        // Deterministic: same inputs => byte-identical message.
        let m1 = commitment_message_with_holder(&c, &salt, &sref, &hdig);
        let m2 = commitment_message_with_holder(&c, &salt, &sref, &hdig);
        assert_eq!(m1, m2, "holder-bound message must be deterministic");

        // Domain-separated from the status (audit #12) message over the SAME
        // (C(G), salt, status_ref): the new ZKSIG_C4 tag (vs ZKSIG_C3) makes them
        // distinct, so a holder-bound and a non-holder-bound attestation can never
        // be confused even with identical C(G)/salt/status.
        let status_msg = commitment_message_with_status(&c, &salt, &sref);
        assert_ne!(
            m1, status_msg,
            "holder-bound (ZKSIG_C4) must differ from status-only (ZKSIG_C3) over the same inputs"
        );

        // Binds the holder digest: a DIFFERENT holder key (=> different digest) =>
        // a different signed message.
        let hpk_other = SecretKey::from_seed(202).public_key();
        let hdig_other = holder_key_digest(&hpk_other).expect("non-identity holder key digests");
        assert_ne!(hdig, hdig_other, "distinct holder keys digest distinctly");
        let m_other = commitment_message_with_holder(&c, &salt, &sref, &hdig_other);
        assert_ne!(
            m1, m_other,
            "changing holder_pk_digest must change the message"
        );
    }

    /// [OPUS-4.8] sq-y464 (HolderPoP T1): `holder_key_digest` is DETERMINISTIC and
    /// distinguishes distinct holder keys, and is DOMAIN-SEPARATED from the issuer
    /// key-set Merkle leaf (`Poseidon2([x, y])`) over the same coordinates — so a
    /// holder-key digest and a key-set leaf can never be cross-substituted.
    #[test]
    fn holder_key_digest_is_deterministic_and_key_distinguishing() {
        let (_sk_a, pk_a) = keypair(211);
        let (_sk_b, pk_b) = keypair(212);

        let dig_a = holder_key_digest(&pk_a).expect("non-identity holder key digests");
        let dig_b = holder_key_digest(&pk_b).expect("non-identity holder key digests");
        // Deterministic.
        assert_eq!(
            dig_a,
            holder_key_digest(&pk_a).expect("non-identity holder key digests"),
            "holder_key_digest must be deterministic"
        );
        // Distinguishes distinct keys.
        assert_ne!(dig_a, dig_b, "distinct holder keys must digest distinctly");
        // Domain-separated from the key-set Merkle leaf (h2(x, y)) over the SAME
        // coordinates: the ZKSIG_HK tag prevents holder-digest / key-set-leaf
        // confusion.
        assert_ne!(
            dig_a,
            key_set_leaf(&pk_a).expect("non-identity key has a leaf"),
            "holder_key_digest (ZKSIG_HK) must differ from the key-set leaf (h2(x,y))"
        );
    }

    /// [OPUS-4.8] sq-rsd3v.1: the single-use nullifier primitive. It is
    /// deterministic, per-holder + per-epoch (the two granularity axes of §6.3(c)),
    /// domain-separated from the holder-key digest over the same holder, and
    /// bit-identical to the pinned in-circuit cross-vector (seed-102 hsk, epoch 7).
    #[test]
    fn nullifier_is_deterministic_per_holder_per_epoch() {
        let sk_a = SecretKey::from_seed(102);
        let sk_b = SecretKey::from_seed(200);
        let hsk_a = jj_scalar_to_base(&sk_a.0);
        let hsk_b = jj_scalar_to_base(&sk_b.0);
        let e7 = Fr::from(7u64);
        let e8 = Fr::from(8u64);

        // Deterministic.
        assert_eq!(nullifier(&hsk_a, &e7), nullifier(&hsk_a, &e7));
        // Per-epoch: same holder, different epoch => different nf.
        assert_ne!(nullifier(&hsk_a, &e7), nullifier(&hsk_a, &e8));
        // Per-holder: different holder, same epoch => different nf.
        assert_ne!(nullifier(&hsk_a, &e7), nullifier(&hsk_b, &e7));
        // Domain-separated from the holder-key digest over the same holder (ZKSIG_NF
        // vs ZKSIG_HK) — a nullifier can never be confused with a holder-key digest.
        assert_ne!(
            nullifier(&hsk_a, &e7),
            holder_key_digest(&sk_a.public_key()).expect("non-identity holder key digests"),
        );
        // CROSS-VECTOR PIN: bit-identical to the in-circuit `nullifier_cross_vector`
        // (tests.nr) and the host `nullifier_matches_noir_cross_vector`
        // (sparq-zk-compose). Drift here breaks the in-circuit `nf == public nf`.
        assert_eq!(
            crate::field::field_to_hex(&nullifier(&hsk_a, &e7)),
            "0x27113b53c9dd70eaf8705b017290442911e46676758fd901c1446286940c7d7e"
        );
    }

    /// [OPUS-4.8] sq-y464 (HolderPoP T1): an issuer Schnorr-signs a holder-bound
    /// message and the EXISTING `verify` accepts it (round-trip via the additive
    /// `sign_commitment_with_holder` path), AND the binding is REAL — a signature
    /// over the holder-bound message does NOT verify against the non-holder
    /// (status-only) message for the same `(C(G), salt, status_ref)`, and vice
    /// versa, nor against a different holder digest. Back-compat: the older
    /// status-only signature still verifies over its own message.
    #[test]
    fn holder_bound_signature_round_trips_and_binds() {
        let sk = SecretKey::from_seed(221);
        let pk = sk.public_key();
        let c = Fr::from(0xc0ffeeu64);
        let salt = Fr::from(0x5a17u64);
        let list_id = status_list_id_to_field("https://dmv.example/status/3");
        let sref = status_ref_digest(&list_id, 94, 7);
        let hpk = SecretKey::from_seed(222).public_key();
        let hdig = holder_key_digest(&hpk).expect("non-identity holder key digests");

        // Round-trip: issuer signs the holder-bound message, existing verify accepts.
        let hex = sk.sign_commitment_with_holder(&c, &salt, &sref, &hdig);
        let sig = signature_from_hex(&hex).expect("round-trips");
        let holder_msg = commitment_message_with_holder(&c, &salt, &sref, &hdig);
        assert!(
            verify(&pk, &holder_msg, &sig),
            "honest holder-bound signature must verify"
        );

        // Binding is real: the holder-bound signature must NOT verify against the
        // non-holder (status-only) message (the prover cannot strip the holder
        // binding by presenting the credential as a bearer status-only credential).
        let status_msg = commitment_message_with_status(&c, &salt, &sref);
        assert!(
            !verify(&pk, &status_msg, &sig),
            "holder-bound sig must NOT verify against the status-only (non-holder) message"
        );

        // And a DIFFERENT holder digest => different message => fails (a presenter
        // cannot substitute its own key for the issuer-bound one).
        let hdig_other = holder_key_digest(&SecretKey::from_seed(223).public_key())
            .expect("non-identity holder key digests");
        let msg_other = commitment_message_with_holder(&c, &salt, &sref, &hdig_other);
        assert!(
            !verify(&pk, &msg_other, &sig),
            "a different holder digest must not verify (key substitution rejected)"
        );

        // Symmetric direction + back-compat: a status-only signature still verifies
        // over its own message (bearer credentials unchanged) and does NOT verify
        // against the holder-bound message.
        let status_hex = sk.sign_commitment_with_status(&c, &salt, &sref);
        let status_sig = signature_from_hex(&status_hex).expect("round-trips");
        assert!(
            verify(&pk, &status_msg, &status_sig),
            "back-compat: a status-only (bearer) signature still verifies over its own message"
        );
        assert!(
            !verify(&pk, &holder_msg, &status_sig),
            "a status-only sig must NOT verify against the holder-bound message (no upgrade-by-substitution)"
        );
    }

    /// [OPUS-4.8] sq-y464 (HolderPoP T1) — Copilot review on #124: the IDENTITY /
    /// neutral holder key MUST be rejected by `holder_key_digest`, not silently
    /// digested as `(0, 0)`. `PublicKey`'s tuple field is `pub`, so an identity key
    /// is externally constructible and could reach this PUBLIC helper directly; a
    /// holder key that is the curve identity is not a valid binding key, and
    /// digesting it would mask accidental/malicious identity-key use. It must fail
    /// closed with [`HolderKeyError::IdentityKey`] (matching the [`verify`] /
    /// [`public_key_from_hex`] identity-rejection discipline). `coords()` returning
    /// `None` is exactly the identity case.
    #[test]
    fn identity_holder_key_digest_rejected() {
        let id_pk = PublicKey(Affine::<EdwardsConfig>::zero());
        assert!(id_pk.0.is_zero(), "constructed the identity point");
        assert!(
            id_pk.coords().is_none(),
            "identity key has no affine coordinates (the None case)"
        );
        // It must NOT be silently digested as (0, 0): fail closed with a descriptive
        // error instead of returning Poseidon2([ZKSIG_HK, 0, 0]).
        assert_eq!(
            holder_key_digest(&id_pk),
            Err(HolderKeyError::IdentityKey),
            "an identity/None holder key must be rejected, not digested as (0, 0)"
        );
        // Sanity: a real (non-identity) key still digests Ok — the rejection is
        // specific to the identity, not a blanket failure.
        let real = SecretKey::from_seed(231).public_key();
        assert!(
            holder_key_digest(&real).is_ok(),
            "a non-identity holder key must still digest successfully"
        );
    }

    // [OPUS-4.8] sq-bwwl / sq-sco0: hidden-join value commitment.
    #[test]
    fn join_value_commitment_is_deterministic_and_domain_separated() {
        let value = Fr::from(0x1234u64);
        let blinding = Fr::from(0xb1u64);
        // Determinism / single-source-of-truth: matches the raw Poseidon2 the Noir
        // `join::join_value_commitment` gadget computes (same tag, same preimage
        // order) — the host half of the cross-vector the Noir test
        // `join_value_commitment_matches_host` pins.
        let c = join_value_commitment(&value, &blinding);
        assert_eq!(
            c,
            poseidon2::hash(&[Fr::from(SIG_DOMAIN_JOIN), value, blinding]),
            "join_value_commitment must be h3(SIG_DOMAIN_JOIN, value, blinding)"
        );
        // The tag bytes are "ZKSIG_JN" (matches the Noir global JOIN_DOMAIN).
        assert_eq!(SIG_DOMAIN_JOIN, u64::from_be_bytes(*b"ZKSIG_JN"));
        // The blinder is HIDING: same value, different blinder => unlinkable.
        let c2 = join_value_commitment(&value, &Fr::from(0xb2u64));
        assert_ne!(c, c2, "different blinders must yield unlinkable commitments");
        // Domain separation: a join commitment must never collide with the
        // status-index commitment over the same (value-as-index, blinding) — the
        // distinct tag prevents cross-substitution.
        let idx_c = status_index_commitment(0x1234u64, &blinding);
        assert_ne!(c, idx_c, "join commitment must be domain-separated from the index commitment");
    }

    // --- sq-1hsl: the pluggable OFF-circuit signature seam ------------------
    // [OPUS-4.8]

    /// A second, MOCK `IssuerSignatureScheme` that exercises the open seam
    /// WITHOUT any real cryptography: it "accepts" iff the signature hex equals a
    /// fixed sentinel, and reports no in-circuit verifier. It exists only to prove
    /// the trait dispatches a non-default scheme distinctly from the Schnorr
    /// default (the trait boundary is real, not a single-impl façade); it is NOT a
    /// signature scheme and is test-only.
    struct MockAcceptScheme;
    const MOCK_CRYPTOSUITE_IRI: &str = "urn:test:mock-accept-v1";
    const MOCK_ACCEPT_SIG: &str = "deadbeef";
    impl IssuerSignatureScheme for MockAcceptScheme {
        fn cryptosuite_iri(&self) -> &str {
            MOCK_CRYPTOSUITE_IRI
        }
        fn verify_message(&self, _pk_hex: &str, _m: &Fr, sig_hex: &str) -> bool {
            sig_hex == MOCK_ACCEPT_SIG
        }
        // in_circuit() defaults to None (verifier-side-only) — exercises the
        // default-method path of the trait too.
    }

    /// The default scheme behind the trait reproduces the EXACT prior Schnorr
    /// verifier path: for the same (key, message, signature) the trait's
    /// `verify_message` accepts iff a direct `verify` accepts — byte-for-byte the
    /// same decision, both for an honest signature and for tampered inputs. This
    /// is the back-compat invariant (the v1 scheme moved behind the trait changed
    /// nothing).
    #[test]
    fn schnorr_scheme_matches_direct_verify_byte_for_byte() {
        let sk = SecretKey::from_seed(101);
        let pk = sk.public_key();
        let c = Fr::from(0xc0ffeeu64);
        let salt = Fr::from(0x5a17u64);
        // The issued-entry path signs the SALT-BOUND message; build that here.
        let sig_hex = sk.sign_commitment_with_salt(&c, &salt);
        let m = commitment_message_with_salt(&c, &salt);
        let pk_hex = public_key_to_hex(&pk);

        let scheme = SchnorrBjjScheme;
        // The trait reports the v1 cryptosuite IRI (single source of truth with
        // the closed enum tag).
        assert_eq!(scheme.cryptosuite_iri(), SignatureScheme::POSEIDON2_SCHNORR_V1_IRI);

        // Honest signature: trait accept == direct verify accept (both true).
        let direct = {
            let sig = signature_from_hex(&sig_hex).unwrap();
            verify(&pk, &m, &sig)
        };
        let via_trait = scheme.verify_message(&pk_hex, &m, &sig_hex);
        assert!(direct && via_trait, "honest signature must verify both ways");
        assert_eq!(direct, via_trait, "trait must mirror direct verify exactly");

        // Tampered message: both reject.
        let m_bad = commitment_message_with_salt(&(c + Fr::from(1u64)), &salt);
        let direct_bad = {
            let sig = signature_from_hex(&sig_hex).unwrap();
            verify(&pk, &m_bad, &sig)
        };
        assert_eq!(scheme.verify_message(&pk_hex, &m_bad, &sig_hex), direct_bad);
        assert!(!scheme.verify_message(&pk_hex, &m_bad, &sig_hex), "tampered msg fails");

        // Malformed key / signature hex: fail-closed, never panic.
        assert!(!scheme.verify_message("zz", &m, &sig_hex), "bad key hex => false");
        assert!(!scheme.verify_message(&pk_hex, &m, "zz"), "bad sig hex => false");

        // And the scheme reports its NATIVE in-circuit member (embedded curve).
        assert_eq!(
            scheme.in_circuit(),
            InCircuitVerifier::Native { member_hint: "hidden_issuer" }
        );
    }

    /// The resolver maps the v1 cryptosuite IRI to the Schnorr scheme and fails
    /// closed (None) on any other IRI — in lock step with the closed
    /// `SignatureScheme::from_cryptosuite_iri` enum tag (the two must agree on
    /// exactly which IRIs are verifiable, so a verifier cannot be tricked).
    #[test]
    fn resolver_is_fail_closed_and_agrees_with_enum_tag() {
        // v1 IRI resolves AND parses to the enum tag.
        let iri = SignatureScheme::POSEIDON2_SCHNORR_V1_IRI;
        assert!(resolve_signature_scheme(iri).is_some());
        assert!(SignatureScheme::from_cryptosuite_iri(iri).is_some());
        assert_eq!(resolve_signature_scheme(iri).unwrap().cryptosuite_iri(), iri);

        // Unknown IRIs: resolver None AND enum None (lock step).
        for bogus in ["", "urn:other", "https://sparq.dev/ns/zk#poseidon2-schnorr-v2"] {
            assert!(resolve_signature_scheme(bogus).is_none(), "resolver fail-closed");
            assert_eq!(
                resolve_signature_scheme(bogus).is_none(),
                SignatureScheme::from_cryptosuite_iri(bogus).is_none(),
                "resolver and enum tag must agree on {}",
                bogus
            );
        }
    }

    /// `verify_commitment_with_scheme` (the seam's convenience entry point)
    /// composes resolve + verify: it accepts an honest v1 signature, rejects a
    /// tampered one, and fails closed on an unknown cryptosuite (no default
    /// fallback) — the property the registry verify path relies on.
    #[test]
    fn verify_commitment_with_scheme_composes_and_fails_closed() {
        let sk = SecretKey::from_seed(202);
        let pk_hex = public_key_to_hex(&sk.public_key());
        let c = Fr::from(0xabcdu64);
        let salt = Fr::from(0x99u64);
        let sig_hex = sk.sign_commitment_with_salt(&c, &salt);
        let m = commitment_message_with_salt(&c, &salt);
        let iri = SignatureScheme::POSEIDON2_SCHNORR_V1_IRI;

        assert!(
            verify_commitment_with_scheme(iri, &pk_hex, &m, &sig_hex),
            "honest v1 signature verifies through the scheme seam"
        );
        // Unknown cryptosuite => fail closed (no default scheme).
        assert!(
            !verify_commitment_with_scheme("urn:unknown", &pk_hex, &m, &sig_hex),
            "unknown cryptosuite must NOT fall back to a default scheme"
        );
        // Tampered signature => reject.
        let mut sig = signature_from_hex(&sig_hex).unwrap();
        sig.s += JjScalar::from(1u64);
        let tampered_hex = signature_to_hex(&sig);
        assert!(
            !verify_commitment_with_scheme(iri, &pk_hex, &m, &tampered_hex),
            "tampered signature must not verify"
        );
    }

    /// The trait dispatches a SECOND, non-default scheme distinctly from the
    /// Schnorr default — through a `&dyn IssuerSignatureScheme`, proving the
    /// boundary is a real open seam (object-safe dynamic dispatch), not a
    /// single-impl façade. The mock and the default disagree on the SAME inputs,
    /// which a single hard-wired scheme could not do.
    #[test]
    fn trait_dispatches_a_second_scheme_dynamically() {
        let schemes: [&dyn IssuerSignatureScheme; 2] = [&SchnorrBjjScheme, &MockAcceptScheme];
        // Distinct cryptosuite IRIs (distinct schemes).
        assert_ne!(schemes[0].cryptosuite_iri(), schemes[1].cryptosuite_iri());

        // The mock "accepts" its sentinel signature and rejects anything else,
        // over ANY key/message — behaviour the real Schnorr scheme would reject.
        let m = Fr::from(7u64);
        assert!(
            schemes[1].verify_message("ignored", &m, MOCK_ACCEPT_SIG),
            "mock accepts its sentinel"
        );
        assert!(
            !schemes[1].verify_message("ignored", &m, "00"),
            "mock rejects a non-sentinel signature"
        );
        // The Schnorr scheme would reject the mock's sentinel (it is not a valid
        // Baby-JubJub signature), so the two impls give DIFFERENT answers on the
        // same input — the dispatch is real.
        assert!(!schemes[0].verify_message("ignored", &m, MOCK_ACCEPT_SIG));

        // The in-circuit discriminator differs per scheme (Native vs None).
        assert_eq!(
            schemes[0].in_circuit(),
            InCircuitVerifier::Native { member_hint: "hidden_issuer" }
        );
        assert_eq!(schemes[1].in_circuit(), InCircuitVerifier::None);
    }

    /// The signature-scheme axis composes with — and is INDEPENDENT of — the
    /// commitment-method axis: a registry entry carries BOTH a `zk:scheme`
    /// (commitment method) and a `zk:cryptosuite` (signature scheme), resolved by
    /// two separate functions. This pins that the two seams are orthogonal (the
    /// design's separation of an open signature trait from the closed commitment
    /// enum).
    #[test]
    fn signature_scheme_and_commitment_method_axes_are_independent() {
        use crate::commit::CommitmentMethod;
        // The signature cryptosuite IRI is NOT a commitment-method scheme IRI, and
        // vice versa — the two axes never cross-resolve.
        let sig_iri = SignatureScheme::POSEIDON2_SCHNORR_V1_IRI;
        let method_iri = crate::registry::ZK_SCHEME_POSEIDON2_RDFC10_V1;
        assert!(resolve_signature_scheme(sig_iri).is_some());
        assert_eq!(
            CommitmentMethod::from_scheme_iri(sig_iri),
            None,
            "a signature cryptosuite IRI is NOT a commitment method"
        );
        assert!(resolve_signature_scheme(method_iri).is_none());
        assert_eq!(
            CommitmentMethod::from_scheme_iri(method_iri),
            Some(CommitmentMethod::StringCanonicalV1),
            "a commitment-method IRI is NOT a signature cryptosuite"
        );
    }
}
