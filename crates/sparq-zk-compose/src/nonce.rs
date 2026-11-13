//! Shared challenge and in-memory nonce vocabulary.

use crate::manifest::FieldHex;
use sparq_zk::field::{field_from_hex_str, field_from_hex_str_strict, field_to_hex, Fr};
use std::collections::BTreeSet;

/// A verifier-issued freshness nonce (audit #4) — a fresh BN254 field element the
/// relying party mints out of band and hands to the prover BEFORE proving. The
/// prover MUST incorporate it as the circuit's `challenge` public input (field 0)
/// at prove time; [`crate::verifier::verify_manifest`] then reconstructs the public-input vector
/// using THIS nonce (never `manifest.binding`), so the existing audit-#1
/// byte-compare rejects any proof whose committed challenge ≠ the verifier's
/// nonce. A captured manifest re-presented under a fresh nonce therefore fails
/// the byte-compare (replay defence), and the [`SeenNonces`] store below rejects
/// a nonce presented twice (single-use defence).
///
/// Stored as a normalized field element so `0x`-padding differences cannot make
/// two presentations of the same nonce look distinct to the single-use store.
// audit #4: verifier-issued freshness nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifierNonce(pub(crate) Fr);

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
    pub(crate) fn canonical_key(&self) -> String {
        field_to_hex(&self.0)
    }
}

/// Single-use nonce store (audit #4): records every verifier nonce a manifest has
/// already been accepted (or attempted) under, so a captured (nonce, manifest)
/// pair cannot be replayed. [`crate::verifier::verify_manifest`] calls [`SeenNonces::record_fresh`]
/// BEFORE the cryptographic gate; a nonce already present REJECTS
/// ([`crate::verifier::CheckError::NonceReplay`]).
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
/// - [`crate::verifier::FileSeenNonces`] (audit #4 durability fix, sq-aih) — DURABLE. Backed by an
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
/// the same restart-surviving, atomic semantics as [`crate::verifier::FileSeenNonces`].
// audit #4: single-use nonce store. sq-aih: durability is mandatory —
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
/// only. A relying party MUST use [`crate::verifier::FileSeenNonces`] (durable: survives restart, is
/// the sq-aih fix) or another durable backing (a DB row with a UNIQUE constraint /
/// a KV store with compare-and-set).
// audit #4 / sq-aih: in-memory store is NON-DURABLE, test-only. The
// durable impl is FileSeenNonces — single-use survives a restart there.
#[derive(Debug, Default)]
pub struct InMemorySeenNonces {
    seen: std::sync::Mutex<BTreeSet<String>>,
}

impl InMemorySeenNonces {
    pub fn new() -> Self {
        InMemorySeenNonces {
            seen: std::sync::Mutex::new(BTreeSet::new()),
        }
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

/// Normalize field hex by value, retaining case normalization for invalid text.
pub(crate) fn normalize_hex(h: &str) -> String {
    field_from_hex_str_strict(h)
        .map(|f| field_to_hex(&f))
        .unwrap_or_else(|| h.to_ascii_lowercase())
}
