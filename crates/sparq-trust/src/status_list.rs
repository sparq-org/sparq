//! # `status_list.rs` — live credential status / revocation + a minimal denial justification (`sq-pfae.7`)
//!
//! The **P6 status stratum** (design `research/solid-trust-graph-authz-design.md` §6.1
//! item 6; issue #940 / gh-940): replace the PoC's per-request `revoked: bool` input flag
//! (the `sq-tu4e` input-only guard) with a check against a **live W3C Bitstring Status
//! List** ([VC-BITSTRING-STATUS-LIST](https://www.w3.org/TR/vc-bitstring-status-list/)),
//! and render a **minimal PROV-O justification** for the resulting allow OR deny.
//!
//! It is **OPT-IN**: the whole module sits behind the default-OFF `status-list` cargo
//! feature, so the lean default `sparq-trust` build (and the byte-identical default
//! `sparq-solid` build) gain nothing — strict additivity (design §2.2 G6) is preserved.
//!
//! ## What a credential's status entry says
//!
//! A `BitstringStatusListEntry` (the credential's `credentialStatus`) names:
//! - a `statusListCredential` IRI (where the published list lives),
//! - a `statusListIndex` (this credential's slot in that list), and
//! - a `statusPurpose` (`revocation` / `suspension`).
//!
//! The published list is itself a Verifiable Credential whose `credentialSubject` carries
//! an `encodedList`: a **multibase** string (`u` = base64url-no-pad per the current spec,
//! or the legacy `z`-base58btc StatusList2021 form) over the **GZIP-compressed** status
//! bitstring. Bit `i` is `bits[i >> 3] & (0x80 >> (i & 7))` — **MSB-first within each byte**,
//! the Bitstring-Status-List §3.2 "leftmost bit is index 0" convention (this is the
//! *opposite* of the legacy StatusList2021 LSB-first layout the ZK
//! `sparq-zk-compose::StatusListSnapshot` mirror uses; that mirror is for the hidden-index
//! ZK proof, NOT this clear-index gate — see *Why a separate decoder*).
//!
//! ## Fail-closed by construction (`sq-pfae.7`: "fail-closed on unknown/stale")
//!
//! The check returns a [`LiveStatus`] that is **`Live` only** when EVERY condition holds:
//! the list resolved, decoded, the index is in range, the bit is **unset**, and the
//! snapshot is **fresh** (its `as_of_unix_secs` is within the caller's `max_age_secs`).
//! Every other outcome **denies**:
//! - [`LiveStatus::Set`] — the status bit is set (revoked/suspended): deny.
//! - [`LiveStatus::Unknown`] — no resolver wired, the list did not resolve, did not
//!   decode, or the index is out of range: deny (**fail-closed on unknown**, NOT
//!   fail-open). An out-of-range index is treated as not-covered (Unknown), never Live.
//! - [`LiveStatus::Stale`] — the snapshot is older than `max_age_secs`: deny
//!   (**fail-closed on stale**).
//!
//! [`LiveStatus::admits`] is the single fail-closed predicate: `true` for `Live` ONLY.
//! Wire it into [`crate::admit::admit`] by mapping `!status.admits()` onto the existing
//! `cred.revoked` guard — a revoked / unknown / stale credential is dropped exactly where
//! the `revoked` flag dropped it, with NO change to the rest of the gate.
//!
//! ## Re-materialise on change — with a bounded incremental *selection* step (design §3.3 / G5)
//!
//! Retraction is still **re-materialisation**: a status change invalidates the admission
//! verdict and the caller re-runs the UNCHANGED gate; there is **no** in-reasoner
//! incremental retraction (`sq-tu4e` forbids NAF over derived facts, so nothing here
//! retracts a derived grant in place).
//!
//! What [`StatusDelta`] (`sq-pfae.14`) adds is the cheap half: on a **status-list epoch
//! bump** (a strictly newer snapshot of the SAME list) it diffs the two snapshots and names
//! the bit indices that actually changed, so the caller re-runs the gate over **only the
//! affected grants** instead of all of them. It is a **selection** step over two *input*
//! snapshots — it computes no verdict, reads no derived fact, and changes no admission
//! rule, so the input-stratified / one-side-bound seeding discipline is untouched (the
//! re-check is still [`LiveStatusCheck::check`], bit for bit).
//!
//! It is **fail-closed**: anything the bit-diff cannot bound — a successor that is not
//! newer, a change in the list's coverage (a longer/shorter bitstring moves indices in or
//! out of range), or more changes than the caller's limit — yields
//! [`StatusDelta::requires_full_recheck`], i.e. *re-check everything on this list*.
//!
//! The skip contract has **two** halves and both are required:
//! `delta.valid_at(now, max_age_secs) && !delta.affects(entry)`. [`StatusDelta::affects`] is
//! the per-slot bit selection; [`StatusDelta::valid_at`] is the whole-list freshness
//! precondition, because staleness is a property of the *snapshot*, not of a slot — most
//! sharply when the successor is future-dated, where an unchanged slot moves from `Live`
//! under the old snapshot to [`LiveStatus::Stale`] under the new one.
//!
//! **Honest residue (`sq-pfae.14`), what this does NOT do:**
//! - It does **not** close the §4.4 stale-authority window. The window is still bounded by
//!   `max_age_secs`, not by the delta: a grant whose bit did **not** change still ages into
//!   [`LiveStatus::Stale`], and [`StatusDelta::affects`] will say `false` for it — which is
//!   exactly what [`StatusDelta::valid_at`] exists to force the caller to confront. The
//!   delta makes an epoch bump cheaper; it does not replace the caller's freshness-driven
//!   re-check.
//! - It is **per list**. A delta binds one `statusListCredential`; grants on other lists are
//!   reported unaffected by construction, so a caller holding grants across N lists must
//!   apply N deltas.
//! - It gives **no incremental retraction inside the reasoner**, and no deep-chain
//!   delegation revocation (§4.4) — the affected grants are re-derived, not patched.
//!
//! ## Why a separate decoder (NOT the ZK `StatusListSnapshot`)
//!
//! `sparq-zk-compose::StatusListSnapshot` is the **host-side mirror for the hidden-index
//! revocation ZK proof** — it is LSB-first (legacy StatusList2021), lives in the heavy ZK
//! estate, and exists to commit a Merkle root the circuit binds. Pulling it here would drag
//! `sparq-zk-compose` (Noir/circuit tooling) onto this crate and break lean-core. This
//! module is the **clear-index** gate: a dependency-light, MSB-first Bitstring-Status-List
//! decoder. The two are deliberately distinct (clear-index gate vs hidden-index proof).
//!
//! ## Network + GZIP are seams — the default build pulls NEITHER
//!
//! Like the `did:web` fetcher, the network fetch is a **pluggable** [`StatusListResolver`]
//! (the crate ships no HTTP client). GZIP decompression is likewise a pluggable
//! [`GzipDecoder`]: the base64url/base58btc multibase decode is hand-rolled and
//! dependency-free, but inflate needs a codec. A built-in `Flate2GzipDecoder` is offered
//! ONLY behind the nested default-OFF `status-list-flate2` feature (it pulls `flate2`); the
//! plain `status-list` feature is dependency-free and decode-only over a caller-supplied
//! decoder. An [`IdentityGzipDecoder`] (no compression) is always available for
//! already-inflated lists and tests.
//!
//! ## Honest scope — what this does and does NOT do
//!
//! - **No privacy / unlinkability.** The status check is over the **clear** index and the
//!   **clear** list; the resolver learns which credential is being checked. It does NOT
//!   deliver the hidden-index ZK revocation proof (`sq-3e5`/`sq-h2v`, the ZK estate — still
//!   externally **UNAUDITED**, `sq-qhy4`). This is the in-the-clear, fail-closed gate.
//! - **The list VC's OWN issuer signature is verifiable (`sq-pfae.13`).** The base
//!   [`LiveStatusCheck`] trusts the list as fetched (the resolver is the trust seam — a
//!   TLS-fetched or pre-validated list). The opt-in [`VerifyingLiveStatusCheck`] CLOSES
//!   that gap: it resolves the status-list VC as a **signed graph** (a [`SignedStatusList`]
//!   over a [`VerifiedStatusListResolver`]) and verifies the list's OWN issuer signature
//!   over its RDFC-1.0 commitment — the SAME `sparq-zk` Schnorr-over-RDFC-1.0 path
//!   [`crate::admit::admit`] uses — against a **trusted status-authority key** BEFORE
//!   trusting its `encodedList`. **Fail-closed**: an unsigned, bad-signature,
//!   wrong-key, or unresolvable-issuer status-list VC is rejected (treated as
//!   [`LiveStatus::Unknown`] — deny), **never** trusted. With the `did` feature the
//!   authority key can be bound from a `did:key`/`did:web` issuer DID
//!   ([`VerifyingLiveStatusCheck::with_did_issuer`]) instead of a raw key. This is
//!   research-grade and **externally UNAUDITED** (`sq-qhy4`) — a verified issuer
//!   signature, NOT a privacy/unlinkability guarantee.
//! - **The justification is an AUDIT record, never an authority.** Like
//!   [`crate::delegation_prov`], the PROV-O graph is emitted *after* the verdict — feeding
//!   it back as input grants nothing (no admission rule trusts a `prov:` triple; the §2.4
//!   reserved-predicate guard is unchanged).
//!
//! [OPUS-4.8] sq-pfae.7 (issue #940 / gh-940, epic sq-pfae P6). Written while Fable
//! unavailable — flag for re-review when Fable returns. 🤖 SPARQ agent — trust-graph
//! live-status / revocation gate.

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};

/// The W3C PROV-O namespace base (the justification uses the standard terms so the graph
/// round-trips through any PROV-O consumer — same posture as [`crate::delegation_prov`]).
const PROV_NS: &str = "http://www.w3.org/ns/prov#";

/// The W3C Bitstring-Status-List vocabulary namespace (the status-entry terms).
const STATUS_NS: &str = "https://www.w3.org/ns/credentials/status#";

/// The `trust:` justification terms reuse the crate vocab namespace.
const TRUST_NS: &str = crate::vocab::TRUST_NS;

/// `prov:` term IRI. Every `local` here is a valid IRI suffix, so `new_unchecked` is safe.
fn prov(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{}{}", PROV_NS, local))
}

/// `trust:` term IRI for a justification predicate (constructed from the crate namespace).
fn trust(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{}{}", TRUST_NS, local))
}

/// `status:` term IRI (the W3C Bitstring-Status-List vocabulary).
fn status(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{}{}", STATUS_NS, local))
}

/// The status purpose a list serves — the W3C `statusPurpose` (a list slot is purpose-
/// specific; a "revocation" list and a "suspension" list are distinct bitstrings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusPurpose {
    /// `revocation` — a permanent, irreversible status change.
    Revocation,
    /// `suspension` — a reversible status change.
    Suspension,
}

impl StatusPurpose {
    /// The spec string for this purpose (`"revocation"` / `"suspension"`).
    pub fn as_str(self) -> &'static str {
        match self {
            StatusPurpose::Revocation => "revocation",
            StatusPurpose::Suspension => "suspension",
        }
    }
}

/// A credential's `BitstringStatusListEntry` — the `credentialStatus` that points at the
/// credential's slot in a published status list (W3C Bitstring-Status-List §2).
///
/// This is the per-credential input the gate consults; it carries NO bit of its own (the
/// authoritative status lives in the resolved list, never in a credential-asserted flag —
/// the `sq-tu4e` "input-only, never self-asserted" discipline, now over a LIVE list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusListEntry {
    /// `statusListCredential` — the IRI of the published list VC to resolve.
    pub status_list_credential: NamedNode,
    /// `statusListIndex` — this credential's slot (bit index) in that list.
    pub status_list_index: u64,
    /// `statusPurpose` — the status this slot encodes.
    pub purpose: StatusPurpose,
}

/// A decoded status bitstring: the inflated bytes + a freshness timestamp.
///
/// Bit `i` is **MSB-first within each byte** (Bitstring-Status-List §3.2: the leftmost bit
/// of the first byte is index 0). An index past the end of the bytes reads as **set**
/// (fail-closed padding — an out-of-range slot is treated as revoked at the bit level; the
/// higher-level [`LiveStatusCheck`] reports such an index as `Unknown`, never `Live`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBitstring {
    /// The inflated status bits (MSB-first within each byte).
    bits: Vec<u8>,
    /// When this snapshot was produced/observed (Unix seconds) — the freshness anchor the
    /// staleness check compares against `now - max_age_secs`.
    as_of_unix_secs: i64,
}

impl StatusBitstring {
    /// Build a snapshot from already-inflated MSB-first bytes observed at `as_of_unix_secs`.
    pub fn from_bits(bits: Vec<u8>, as_of_unix_secs: i64) -> Self {
        StatusBitstring {
            bits,
            as_of_unix_secs,
        }
    }

    /// The bit at `index` (MSB-first within each byte). An out-of-range index reads as
    /// **set** (`true`) — fail-closed padding, an unknown slot is treated as revoked.
    pub fn is_set(&self, index: u64) -> bool {
        let byte = (index >> 3) as usize;
        let bit_in_byte = (index & 7) as u8; // 0 = MSB
        match self.bits.get(byte) {
            // MSB-first: leftmost bit is index 0 within the byte.
            Some(b) => (b >> (7 - bit_in_byte)) & 1 == 1,
            None => true, // out of range ⇒ fail-closed (treated as set/revoked)
        }
    }

    /// When this snapshot was observed (Unix seconds).
    pub fn as_of_unix_secs(&self) -> i64 {
        self.as_of_unix_secs
    }

    /// The number of bytes in the bitstring.
    pub fn len_bytes(&self) -> usize {
        self.bits.len()
    }
}

/// A pluggable GZIP decoder for the `encodedList`'s compressed payload. The multibase
/// decode (base64url / base58btc) is hand-rolled and dependency-free, but inflate needs a
/// codec — this is the **compression seam** that keeps the default build dependency-free.
pub trait GzipDecoder {
    /// Inflate `gzipped` (RFC 1952 GZIP) to the raw status bytes, or a human-readable error.
    fn inflate(&self, gzipped: &[u8]) -> Result<Vec<u8>, String>;
}

/// A no-compression decoder: returns the input unchanged. For already-inflated lists and
/// tests. Always available (the spec mandates GZIP, so this is NOT spec-conformant on a
/// real `encodedList` — it is the test/seam fallback).
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityGzipDecoder;

impl GzipDecoder for IdentityGzipDecoder {
    fn inflate(&self, gzipped: &[u8]) -> Result<Vec<u8>, String> {
        Ok(gzipped.to_vec())
    }
}

/// The built-in GZIP decoder over `flate2` — ONLY behind the nested default-OFF
/// `status-list-flate2` feature (it pulls the `flate2` dependency). The plain `status-list`
/// feature is dependency-free and uses a caller-supplied [`GzipDecoder`].
#[cfg(feature = "status-list-flate2")]
#[cfg_attr(docsrs, doc(cfg(feature = "status-list-flate2")))]
#[derive(Debug, Clone, Copy, Default)]
pub struct Flate2GzipDecoder;

#[cfg(feature = "status-list-flate2")]
impl GzipDecoder for Flate2GzipDecoder {
    fn inflate(&self, gzipped: &[u8]) -> Result<Vec<u8>, String> {
        use std::io::Read as _;
        let mut decoder = flate2::read::GzDecoder::new(gzipped);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| format!("gzip inflate failed: {}", e))?;
        Ok(out)
    }
}

/// The multibase prefix for base64url-no-pad (`u`), the current Bitstring-Status-List form.
const MULTIBASE_BASE64URL: char = 'u';
/// The multibase prefix for base58btc (`z`), the legacy StatusList2021 form.
const MULTIBASE_BASE58BTC: char = 'z';

/// Decode a multibase `encodedList` string + inflate it into a [`StatusBitstring`].
///
/// Accepts the current base64url-no-pad (`u`) form and the legacy base58btc (`z`) form.
/// Fails closed (returns `None`) on an unknown multibase prefix, malformed base, or a
/// decoder inflate error. `as_of_unix_secs` stamps the resulting snapshot's freshness.
pub fn decode_encoded_list<D: GzipDecoder>(
    encoded_list: &str,
    decoder: &D,
    as_of_unix_secs: i64,
) -> Option<StatusBitstring> {
    let prefix = encoded_list.chars().next()?;
    let body = &encoded_list[prefix.len_utf8()..];
    let compressed = match prefix {
        MULTIBASE_BASE64URL => base64url_decode(body)?,
        MULTIBASE_BASE58BTC => base58btc_decode(body)?,
        _ => return None, // unknown multibase prefix ⇒ fail closed
    };
    let bits = decoder.inflate(&compressed).ok()?;
    Some(StatusBitstring::from_bits(bits, as_of_unix_secs))
}

/// A pluggable resolver for a published status-list credential. Hands the gate the
/// `encodedList` string + the snapshot's `as_of` timestamp, or signals the list could not
/// be obtained (mapped to a fail-closed [`LiveStatus::Unknown`]).
///
/// This is the **network seam** — the crate ships no HTTP client, so the default build
/// forces no `reqwest`/`ureq` onto anyone and the resolver stays offline-testable (an
/// in-memory map is a perfectly valid resolver for tests / a pre-fetched registry).
pub trait StatusListResolver {
    /// Resolve the list at `status_list_credential`. Return `(encoded_list, as_of_unix_secs)`
    /// — the multibase `encodedList` string and when the snapshot was observed — or `None`
    /// if the list could not be obtained (⇒ fail-closed `Unknown`).
    fn resolve(&self, status_list_credential: &NamedNode) -> Option<(String, i64)>;
}

/// The verdict of a live status check — fail-closed by construction: ONLY [`LiveStatus::Live`]
/// admits ([`LiveStatus::admits`]). Every other variant denies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStatus {
    /// The list resolved + decoded, the index is in range, the bit is **unset**, and the
    /// snapshot is **fresh**. The ONLY variant that admits.
    Live,
    /// The status bit is **set** (revoked / suspended): deny.
    Set,
    /// The list did not resolve / decode, no resolver is wired, or the index is out of
    /// range (not covered by the list): deny (**fail-closed on unknown**, never fail-open).
    Unknown,
    /// The resolved snapshot is older than the caller's `max_age_secs` (or from the
    /// future): deny (**fail-closed on stale**).
    Stale,
}

impl LiveStatus {
    /// The single fail-closed admission predicate: `true` for [`LiveStatus::Live`] ONLY.
    pub fn admits(self) -> bool {
        matches!(self, LiveStatus::Live)
    }

    /// A short machine-readable reason token for the denial justification (or `"live"`).
    pub fn reason(self) -> &'static str {
        match self {
            LiveStatus::Live => "live",
            LiveStatus::Set => "status-set",
            LiveStatus::Unknown => "status-unknown",
            LiveStatus::Stale => "status-stale",
        }
    }
}

/// A live status check: a pluggable [`StatusListResolver`] + a [`GzipDecoder`] + a freshness
/// budget. Consult it with [`Self::check`] for a credential's [`StatusListEntry`] at `now`.
///
/// Fail-closed by construction: a missing resolver, an unresolvable / undecodable list, an
/// out-of-range index, a set bit, or a snapshot older than `max_age_secs` ALL deny.
pub struct LiveStatusCheck<R: StatusListResolver, D: GzipDecoder> {
    resolver: R,
    decoder: D,
    /// The maximum age (seconds) a resolved snapshot may have before it is treated as
    /// stale (fail-closed). Bounds the §4.4 stale-authority window.
    max_age_secs: i64,
}

impl<R: StatusListResolver, D: GzipDecoder> LiveStatusCheck<R, D> {
    /// Build a check over the given resolver + decoder with a `max_age_secs` freshness
    /// budget. A non-positive `max_age_secs` means "no snapshot is ever fresh enough" —
    /// every check is `Stale` (the maximally fail-closed setting).
    pub fn new(resolver: R, decoder: D, max_age_secs: i64) -> Self {
        LiveStatusCheck {
            resolver,
            decoder,
            max_age_secs,
        }
    }

    /// Check a credential's status entry against the live list at instant `now_unix_secs`.
    /// Returns the fail-closed [`LiveStatus`].
    pub fn check(&self, entry: &StatusListEntry, now_unix_secs: i64) -> LiveStatus {
        // Resolve the published list (network seam). No list ⇒ fail-closed Unknown.
        let Some((encoded, as_of)) = self.resolver.resolve(&entry.status_list_credential) else {
            return LiveStatus::Unknown;
        };
        // Decode + inflate. A bad multibase / inflate error ⇒ fail-closed Unknown.
        let Some(snapshot) = decode_encoded_list(&encoded, &self.decoder, as_of) else {
            return LiveStatus::Unknown;
        };
        // Freshness + bit logic (shared with the verifying gate — an out-of-range index is
        // `Unknown`, NOT a positive revocation, but still fail-closed; a stale snapshot
        // denies).
        status_from_snapshot(&snapshot, entry, self.max_age_secs, now_unix_secs)
    }
}

// --- verified status-list VC (the list's OWN issuer signature — sq-pfae.13) ---------------

/// The W3C VC `encodedList` term — the multibase status bitstring on the published
/// status-list credential's `credentialSubject`. The verified path reads the
/// `encodedList` from the **signed** VC graph, so the bits it gates on are exactly the
/// bits the status authority signed.
fn encoded_list_predicate() -> NamedNode {
    status("encodedList")
}

/// A published status-list credential as a **signed graph** — the verifiable unit the
/// verified gate consumes (`sq-pfae.13`). It is the SAME signed-graph shape
/// [`crate::admit::PresentedCredential`] uses: the VC claim graph, the issuer Schnorr
/// signature over its RDFC-1.0 commitment, and the per-graph salt the commitment was
/// minted under.
///
/// The graph MUST carry the `status:encodedList` triple (the multibase status bitstring
/// on the credential subject); the verified gate extracts the `encodedList` from THIS
/// graph **after** the signature verifies, so the bits it trusts are exactly the bits the
/// status authority signed. A graph with no (or a non-string) `encodedList` fails closed.
#[derive(Debug, Clone)]
pub struct SignedStatusList {
    /// The status-list VC claim graph (must contain a `status:encodedList` literal).
    pub graph: Vec<Triple>,
    /// The status authority's Schnorr signature over `commitment_message(C(graph))`,
    /// hex-encoded exactly as `sparq_zk::sig::SecretKey::sign_commitment` produces. An
    /// absent / forged signature fails closed (the list is treated as `Unknown`).
    pub issuer_signature_hex: String,
    /// The 32-byte per-graph salt the RDFC-1.0 commitment was minted under. The verifier
    /// re-commits under the same salt; a mismatched salt yields a different `C(graph)` and
    /// fails the signature check.
    pub salt: [u8; 32],
}

impl SignedStatusList {
    /// Extract the multibase `encodedList` string from the (already verified) VC graph.
    /// Returns `None` if the graph carries no `status:encodedList` literal — fail-closed:
    /// a verified VC that does not actually publish a list cannot admit anything.
    fn encoded_list(&self) -> Option<&str> {
        let pred = encoded_list_predicate();
        self.graph.iter().find_map(|t| {
            if t.predicate == pred {
                if let Term::Literal(l) = &t.object {
                    return Some(l.value());
                }
            }
            None
        })
    }
}

/// A pluggable resolver for a published status-list credential **as a signed graph**.
/// Unlike [`StatusListResolver`] (which hands back the bare `encodedList` string, trusted
/// as fetched), this returns a [`SignedStatusList`] whose OWN issuer signature the
/// [`VerifyingLiveStatusCheck`] verifies against a trusted status authority before
/// trusting any bit (`sq-pfae.13`).
///
/// Like [`StatusListResolver`] this is the **network seam** — the crate ships no HTTP
/// client, so an in-memory map (a pre-fetched signed registry) is a valid resolver and
/// the default build forces no `reqwest`/`ureq` onto anyone.
pub trait VerifiedStatusListResolver {
    /// Resolve the list at `status_list_credential` to its signed VC + the snapshot's
    /// `as_of_unix_secs`, or `None` if it could not be obtained (⇒ fail-closed `Unknown`).
    fn resolve_signed(&self, status_list_credential: &NamedNode)
        -> Option<(SignedStatusList, i64)>;
}

/// A live status check that **verifies the status-list VC's OWN issuer signature**
/// (`sq-pfae.13`) before trusting its bits.
///
/// It resolves the published list as a [`SignedStatusList`] (a signed graph) via a
/// [`VerifiedStatusListResolver`], then — **before** decoding the `encodedList` —
/// verifies the list's issuer Schnorr signature over its RDFC-1.0 commitment against a
/// **trusted status-authority key**, reusing the EXACT `sparq-zk` Schnorr-over-RDFC-1.0
/// path [`crate::admit::admit`] uses. Only on a valid signature does it read the
/// `encodedList` from the verified graph and run the SAME freshness + bit logic as
/// [`LiveStatusCheck`].
///
/// **Fail-closed by construction** (`sq-pfae.13`): an unsigned, malformed-signature,
/// wrong-key, or unresolvable-issuer status-list VC, or a verified graph with no
/// `encodedList`, ALL yield [`LiveStatus::Unknown`] (deny) — the bits are **never**
/// trusted. Set-bit / stale / out-of-range outcomes are unchanged from [`LiveStatusCheck`].
///
/// The trusted key is supplied by the caller. With the `did` feature it can be bound from
/// a status-authority issuer DID via [`Self::with_did_issuer`] (the same `did:key`/`did:web`
/// binding the admission gate uses). Research-grade, externally **UNAUDITED** (`sq-qhy4`):
/// a verified issuer signature, NOT a privacy/unlinkability guarantee.
pub struct VerifyingLiveStatusCheck<R: VerifiedStatusListResolver, D: GzipDecoder> {
    resolver: R,
    decoder: D,
    /// The trusted status-authority verifying key. A list VC whose signature does not
    /// verify against THIS key fails closed (`Unknown`).
    authority_key: sparq_zk::sig::PublicKey,
    /// The maximum age (seconds) a resolved snapshot may have before it is treated as
    /// stale (fail-closed). Bounds the §4.4 stale-authority window.
    max_age_secs: i64,
}

impl<R: VerifiedStatusListResolver, D: GzipDecoder> VerifyingLiveStatusCheck<R, D> {
    /// Build a verifying check over a [`VerifiedStatusListResolver`] + a [`GzipDecoder`],
    /// a trusted status-authority `authority_key`, and a `max_age_secs` freshness budget.
    /// A non-positive `max_age_secs` means "no snapshot is ever fresh enough" — every check
    /// is `Stale` (the maximally fail-closed setting), exactly as [`LiveStatusCheck::new`].
    pub fn new(
        resolver: R,
        decoder: D,
        authority_key: sparq_zk::sig::PublicKey,
        max_age_secs: i64,
    ) -> Self {
        VerifyingLiveStatusCheck {
            resolver,
            decoder,
            authority_key,
            max_age_secs,
        }
    }

    /// Build a verifying check whose trusted status-authority key is resolved from a
    /// status-authority **DID** (`did:key` offline self-cert or a pluggable `did:web`),
    /// using the SAME [`crate::did::DidResolver`] binding the admission gate uses. Fails
    /// closed (returns the [`crate::did::DidError`]) if the DID does not resolve to a key —
    /// so a status authority named by an unresolvable DID can verify nothing.
    ///
    /// `did` feature only.
    #[cfg(feature = "did")]
    #[cfg_attr(docsrs, doc(cfg(feature = "did")))]
    pub fn with_did_issuer<Res: crate::did::DidResolver>(
        resolver: R,
        decoder: D,
        did_resolver: &Res,
        authority_did: &str,
        max_age_secs: i64,
    ) -> Result<Self, crate::did::DidError> {
        let authority_key = did_resolver.resolve_str(authority_did)?;
        Ok(Self::new(resolver, decoder, authority_key, max_age_secs))
    }

    /// Check a credential's status entry against the **verified** live list at instant
    /// `now_unix_secs`. Returns the fail-closed [`LiveStatus`]. The status-list VC's own
    /// issuer signature is verified against the trusted authority key BEFORE any bit is
    /// trusted; a non-verifying VC is [`LiveStatus::Unknown`] (deny).
    pub fn check(&self, entry: &StatusListEntry, now_unix_secs: i64) -> LiveStatus {
        // Resolve the published list AS A SIGNED GRAPH (network seam). No list ⇒ Unknown.
        let Some((signed, as_of)) = self.resolver.resolve_signed(&entry.status_list_credential)
        else {
            return LiveStatus::Unknown;
        };
        // Verify the list VC's OWN issuer signature over its RDFC-1.0 commitment against the
        // trusted status authority — the SAME Schnorr-over-RDFC-1.0 path the admit gate uses.
        // A bad / absent / wrong-key signature ⇒ fail-closed Unknown (NEVER trust the bits).
        if !verify_status_list_signature(&signed, &self.authority_key) {
            return LiveStatus::Unknown;
        }
        // Read the encodedList from the VERIFIED graph (so the bits are exactly those the
        // authority signed). A verified VC with no encodedList ⇒ fail-closed Unknown.
        let Some(encoded) = signed.encoded_list() else {
            return LiveStatus::Unknown;
        };
        // Decode + inflate. A bad multibase / inflate error ⇒ fail-closed Unknown.
        let Some(snapshot) = decode_encoded_list(encoded, &self.decoder, as_of) else {
            return LiveStatus::Unknown;
        };
        // Freshness + bit logic is identical to the unverified gate.
        status_from_snapshot(&snapshot, entry, self.max_age_secs, now_unix_secs)
    }
}

/// Verify a [`SignedStatusList`]'s issuer signature over its RDFC-1.0 commitment against
/// the trusted status-authority `authority_key`. Returns `false` (fail-closed) on an
/// uncommittable graph, a malformed signature hex, or a signature that does not verify —
/// the SAME `commit_triples` → `commitment_message` → `verify` path [`crate::admit::admit`]
/// runs over a presented credential.
fn verify_status_list_signature(
    signed: &SignedStatusList,
    authority_key: &sparq_zk::sig::PublicKey,
) -> bool {
    use sparq_zk::commit::commit_triples;
    use sparq_zk::encode::salt_from_bytes;
    use sparq_zk::sig::{commitment_message, signature_from_hex, verify};

    let salt = salt_from_bytes(&signed.salt);
    let Ok(commitment) = commit_triples(&signed.graph, salt) else {
        return false; // uncommittable graph ⇒ fail closed
    };
    let Some(signature) = signature_from_hex(&signed.issuer_signature_hex) else {
        return false; // malformed signature hex ⇒ fail closed
    };
    let msg = commitment_message(&commitment.commitment);
    verify(authority_key, &msg, &signature)
}

/// The shared freshness + bit decision over an already-decoded [`StatusBitstring`] — the
/// tail both [`LiveStatusCheck::check`] and [`VerifyingLiveStatusCheck::check`] run once
/// they hold a snapshot, so the two gates stay bit-for-bit identical from this point on.
fn status_from_snapshot(
    snapshot: &StatusBitstring,
    entry: &StatusListEntry,
    max_age_secs: i64,
    now_unix_secs: i64,
) -> LiveStatus {
    // Freshness: a snapshot older than max_age_secs (or from the future) is stale.
    let age = now_unix_secs.saturating_sub(snapshot.as_of_unix_secs());
    if max_age_secs <= 0 || age < 0 || age > max_age_secs {
        return LiveStatus::Stale;
    }
    // An out-of-range index is `Unknown` (the slot is not covered), NOT a positive
    // revocation — but still fail-closed (deny).
    let byte = (entry.status_list_index >> 3) as usize;
    if byte >= snapshot.len_bytes() {
        return LiveStatus::Unknown;
    }
    if snapshot.is_set(entry.status_list_index) {
        LiveStatus::Set
    } else {
        LiveStatus::Live
    }
}

// --- incremental status delta: re-check only the affected grants (sq-pfae.14) ------------

/// The default cap on how many changed indices a [`StatusDelta`] will enumerate before it
/// gives up and demands a full re-check. Past this point the delta has stopped being a
/// saving (and an unbounded `Vec<u64>` is an attacker-chosen allocation), so falling back
/// is both cheaper and fail-closed.
pub const DEFAULT_MAX_CHANGED_INDICES: usize = 4096;

/// Why an epoch bump could **not** be reduced to a bounded set of affected indices — the
/// caller must re-check every grant on the list (fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaUnbounded {
    /// The successor snapshot is not strictly newer than the predecessor (equal or older
    /// `as_of`): this is not an epoch bump, so the diff is not a meaningful delta.
    NotNewer,
    /// The two snapshots cover a different number of bits. A longer/shorter bitstring moves
    /// indices in or out of range — a `Live`/`Set` ⇄ `Unknown` verdict change a bit-diff
    /// over the common prefix cannot see.
    CoverageChanged,
    /// More indices changed than the caller's limit allows.
    LimitExceeded,
}

impl DeltaUnbounded {
    /// A short machine-readable reason token (for logs / an audit record).
    pub fn reason(self) -> &'static str {
        match self {
            DeltaUnbounded::NotNewer => "delta-not-newer",
            DeltaUnbounded::CoverageChanged => "delta-coverage-changed",
            DeltaUnbounded::LimitExceeded => "delta-limit-exceeded",
        }
    }
}

/// Either the bounded set of changed indices, or the reason we could not bound it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeltaBody {
    /// Exactly these bit indices differ between the two snapshots (ascending).
    Changed(Vec<u64>),
    /// Unbounded ⇒ every grant on this list must be re-checked.
    Unbounded(DeltaUnbounded),
}

/// The change between two snapshots of **one** status list across an epoch bump
/// (`sq-pfae.14`): which slots moved, so the caller re-runs the admission gate over only
/// the grants bound to those slots.
///
/// This is a **selection** step, not a verdict: it is computed from two *input* snapshots
/// and hands back indices. The verdict for an affected grant still comes from the unchanged
/// [`LiveStatusCheck::check`] / [`VerifyingLiveStatusCheck::check`], so the input-stratified
/// seeding discipline is preserved and nothing is retracted inside the reasoner
/// (`sq-tu4e`).
///
/// **The skip contract — both halves are required.** A caller may skip an entry's re-check at
/// `now` **only** when `delta.valid_at(now, max_age_secs) && !delta.affects(entry)`:
/// - [`Self::valid_at`] is the **whole-list** precondition (the successor snapshot is itself
///   fresh at `now`). If it is `false`, every grant on the list is [`LiveStatus::Stale`] and
///   the per-slot selection says nothing — re-check them all.
/// - [`Self::affects`] is the **per-slot** selection over the bits.
///
/// **Soundness obligation it meets.** Under that contract, for a skipped entry
/// `verdict(prev).admits() ⇒ verdict(next).admits()` — so skipping can never leave admitted a
/// grant the full re-run would have denied. (The stronger `verdict(prev) == verdict(next)`
/// holds when *both* snapshots are fresh at `now`; when only the successor is, a skipped entry
/// can only have been *denied* under `prev`, so the skip stays fail-closed.) Every case the
/// bit-diff cannot establish this for — not-newer, coverage change, over-limit — collapses to
/// [`Self::requires_full_recheck`].
///
/// **What it does NOT do** — see the module docs: it does not close the §4.4 stale-authority
/// window (an unchanged bit still ages into [`LiveStatus::Stale`], which is exactly what
/// [`Self::valid_at`] forces the caller to confront), and it binds exactly one list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDelta {
    status_list_credential: NamedNode,
    /// The successor snapshot's `as_of` — the anchor [`StatusDelta::valid_at`] tests, so the
    /// caller cannot use a per-slot selection at an instant where the whole list is stale.
    successor_as_of_unix_secs: i64,
    body: DeltaBody,
}

impl StatusDelta {
    /// Diff two snapshots of the list at `status_list_credential`, enumerating at most
    /// [`DEFAULT_MAX_CHANGED_INDICES`] changed indices. See [`Self::between_with_limit`].
    pub fn between(
        status_list_credential: NamedNode,
        prev: &StatusBitstring,
        next: &StatusBitstring,
    ) -> StatusDelta {
        StatusDelta::between_with_limit(
            status_list_credential,
            prev,
            next,
            DEFAULT_MAX_CHANGED_INDICES,
        )
    }

    /// Diff two snapshots of the list at `status_list_credential`, enumerating at most
    /// `max_changed_indices` changed indices before falling back to
    /// [`DeltaUnbounded::LimitExceeded`].
    ///
    /// `prev` is the snapshot the caller's current verdicts rest on and `next` the freshly
    /// resolved one. Fails closed (an unbounded delta ⇒ re-check everything on the list)
    /// when `next` is not strictly newer than `prev`, when the two cover a different number
    /// of bits, or when more than `max_changed_indices` slots moved.
    pub fn between_with_limit(
        status_list_credential: NamedNode,
        prev: &StatusBitstring,
        next: &StatusBitstring,
        max_changed_indices: usize,
    ) -> StatusDelta {
        let unbounded = |reason| StatusDelta {
            status_list_credential: status_list_credential.clone(),
            successor_as_of_unix_secs: next.as_of_unix_secs(),
            body: DeltaBody::Unbounded(reason),
        };
        // An epoch bump is a strictly NEWER snapshot of the same list. Anything else is not
        // a bump we can reason about (a replayed or reordered snapshot) ⇒ fail closed.
        if next.as_of_unix_secs() <= prev.as_of_unix_secs() {
            return unbounded(DeltaUnbounded::NotNewer);
        }
        // Coverage must be identical: an index that moves in or out of range flips between
        // a bit verdict and `Unknown`, which a diff over the common prefix cannot see.
        if prev.len_bytes() != next.len_bytes() {
            return unbounded(DeltaUnbounded::CoverageChanged);
        }
        let mut changed: Vec<u64> = Vec::new();
        for (byte_index, (a, b)) in prev.bits.iter().zip(next.bits.iter()).enumerate() {
            let mut diff = a ^ b;
            while diff != 0 {
                // MSB-first within the byte (§3.2), so the leading-zero count IS the bit's
                // index within the byte — the same convention `StatusBitstring::is_set` reads.
                let bit_in_byte = diff.leading_zeros() as u64;
                changed.push(((byte_index as u64) << 3) | bit_in_byte);
                if changed.len() > max_changed_indices {
                    return unbounded(DeltaUnbounded::LimitExceeded);
                }
                diff &= !(0x80u8 >> bit_in_byte);
            }
        }
        StatusDelta {
            status_list_credential,
            successor_as_of_unix_secs: next.as_of_unix_secs(),
            body: DeltaBody::Changed(changed),
        }
    }

    /// The list this delta binds. A delta says nothing about any other list.
    pub fn status_list_credential(&self) -> &NamedNode {
        &self.status_list_credential
    }

    /// When the successor snapshot was observed (Unix seconds) — the freshness anchor
    /// [`Self::valid_at`] tests.
    pub fn successor_as_of_unix_secs(&self) -> i64 {
        self.successor_as_of_unix_secs
    }

    /// The **whole-list precondition** on skipping any re-check: is the successor snapshot
    /// itself fresh at `now_unix_secs` under `max_age_secs`?
    ///
    /// This is the SAME freshness test [`LiveStatusCheck::check`] applies (a non-positive
    /// `max_age_secs`, an aged-out snapshot, or a future-dated one all fail it). It is
    /// separate from [`Self::affects`] because staleness is a property of the **snapshot**,
    /// not of a slot: when this is `false` EVERY grant on the list is [`LiveStatus::Stale`]
    /// and no per-slot skip is sound — most sharply when the successor is *future*-dated,
    /// where an unchanged slot moves from `Live` under `prev` to `Stale` under `next`.
    ///
    /// Skip an entry's re-check only when `valid_at(now, max_age) && !affects(entry)`.
    pub fn valid_at(&self, now_unix_secs: i64, max_age_secs: i64) -> bool {
        let age = now_unix_secs.saturating_sub(self.successor_as_of_unix_secs);
        max_age_secs > 0 && age >= 0 && age <= max_age_secs
    }

    /// The changed bit indices in ascending order, or `None` when the delta is unbounded
    /// (⇒ [`Self::requires_full_recheck`]).
    pub fn changed_indices(&self) -> Option<&[u64]> {
        match &self.body {
            DeltaBody::Changed(ix) => Some(ix),
            DeltaBody::Unbounded(_) => None,
        }
    }

    /// Why the delta is unbounded, or `None` when it is bounded.
    pub fn unbounded_reason(&self) -> Option<DeltaUnbounded> {
        match &self.body {
            DeltaBody::Changed(_) => None,
            DeltaBody::Unbounded(r) => Some(*r),
        }
    }

    /// `true` when the delta could not be bounded and the caller must re-check **every**
    /// grant on this list (the fail-closed fallback — the pre-`sq-pfae.14` behaviour).
    pub fn requires_full_recheck(&self) -> bool {
        matches!(self.body, DeltaBody::Unbounded(_))
    }

    /// Whether `entry`'s status verdict may have changed across the epoch bump.
    ///
    /// `true` ⇒ re-run the gate for the grants bound to this entry. `false` ⇒ the entry's
    /// verdict against the new snapshot is identical to its verdict against the old one, so
    /// the re-check can be skipped.
    ///
    /// An entry on a **different** list is reported unaffected — a delta binds exactly one
    /// `statusListCredential`. And `false` speaks only to the **bits**: an entry whose bit
    /// did not change can still have aged into [`LiveStatus::Stale`], which is the caller's
    /// freshness-driven re-check, not this delta's job (the §4.4 residue).
    pub fn affects(&self, entry: &StatusListEntry) -> bool {
        if entry.status_list_credential != self.status_list_credential {
            return false;
        }
        match &self.body {
            DeltaBody::Unbounded(_) => true,
            // `changed` is built in ascending index order, so a binary search is exact.
            DeltaBody::Changed(ix) => ix.binary_search(&entry.status_list_index).is_ok(),
        }
    }
}

// --- minimal PROV-O justification (the "minimal justification" half of sq-pfae.7) --------

/// A minimal PROV-O justification for a status DECISION over a single grant — an **audit
/// record, never an authority** (same posture as [`crate::delegation_prov`]: emitted AFTER
/// the verdict; feeding it back grants nothing).
///
/// For an allow it records the grant as live; for a deny (the bead's *minimal denial
/// justification*) it records WHY — the [`LiveStatus::reason`] token + the status entry that
/// was checked — so an auditor can replay the decision without re-fetching the list.
#[derive(Debug, Clone)]
pub struct StatusJustification {
    triples: Vec<Triple>,
}

impl StatusJustification {
    /// The justification triples (a small fixed set, like the delegation audit).
    pub fn triples(&self) -> &[Triple] {
        &self.triples
    }

    /// Consume the justification, returning the owned triples.
    pub fn into_triples(self) -> Vec<Triple> {
        self.triples
    }
}

/// Render a **minimal PROV-O justification** for a status decision over `grant`.
///
/// The activity (a fresh blank node) is a `prov:Activity` (also `trust:StatusCheck`) that
/// `prov:generated` the grant entity; the activity `prov:used` the named
/// `statusListCredential` entity and carries the checked index, the purpose, the decision
/// (`trust:statusDecision` = `live` / the deny reason), and (when supplied) `prov:atTime`.
/// For a DENY the decision token is the [`LiveStatus::reason`] — the *minimal denial
/// justification* the bead names.
///
/// `grant` is the `auth:*` (or any) grant triple the decision gates; its subject is recorded
/// as `trust:aboutSubject` and its predicate as `trust:grantPredicate`. The activity is the
/// SAME PROV-O shape the delegation audit uses, so the two compose in one audit graph.
pub fn justify_status_decision(
    grant: &Triple,
    entry: &StatusListEntry,
    decision: LiveStatus,
    at_time_unix_secs: Option<i64>,
) -> StatusJustification {
    let mut triples = Vec::new();
    let activity = NamedOrBlankNode::BlankNode(oxrdf::BlankNode::default());
    let grant_entity = NamedOrBlankNode::BlankNode(oxrdf::BlankNode::default());

    // The activity is a prov:Activity, typed trust:StatusCheck so an auditor can find it.
    triples.push(Triple::new(
        activity.clone(),
        NamedNode::from(rdf::TYPE),
        Term::NamedNode(prov("Activity")),
    ));
    triples.push(Triple::new(
        activity.clone(),
        NamedNode::from(rdf::TYPE),
        Term::NamedNode(trust("StatusCheck")),
    ));
    // It used the named status list credential (a prov:Entity input).
    let list_entity = entry.status_list_credential.clone();
    triples.push(Triple::new(
        activity.clone(),
        prov("used"),
        Term::NamedNode(list_entity.clone()),
    ));
    triples.push(Triple::new(
        NamedOrBlankNode::NamedNode(list_entity),
        NamedNode::from(rdf::TYPE),
        Term::NamedNode(prov("Entity")),
    ));
    // The checked index + purpose (so the audit binds the exact slot).
    triples.push(Triple::new(
        activity.clone(),
        status("statusListIndex"),
        Term::Literal(Literal::new_typed_literal(
            entry.status_list_index.to_string(),
            xsd::NON_NEGATIVE_INTEGER,
        )),
    ));
    triples.push(Triple::new(
        activity.clone(),
        status("statusPurpose"),
        Term::Literal(Literal::new_simple_literal(entry.purpose.as_str())),
    ));
    // The decision token (live / the deny reason — the minimal justification).
    triples.push(Triple::new(
        activity.clone(),
        trust("statusDecision"),
        Term::Literal(Literal::new_simple_literal(decision.reason())),
    ));
    // Whether the decision admits (a boolean an auditor can act on directly).
    triples.push(Triple::new(
        activity.clone(),
        trust("statusAdmits"),
        Term::Literal(Literal::new_typed_literal(
            if decision.admits() { "true" } else { "false" },
            xsd::BOOLEAN,
        )),
    ));
    // The grant entity the activity generated, and the grant triple it is about.
    triples.push(Triple::new(
        activity.clone(),
        prov("generated"),
        Term::from(grant_entity.clone()),
    ));
    triples.push(Triple::new(
        grant_entity.clone(),
        NamedNode::from(rdf::TYPE),
        Term::NamedNode(prov("Entity")),
    ));
    triples.push(Triple::new(
        grant_entity.clone(),
        trust("aboutSubject"),
        subject_term(&grant.subject),
    ));
    triples.push(Triple::new(
        grant_entity,
        trust("grantPredicate"),
        Term::NamedNode(grant.predicate.clone()),
    ));
    // Optional activity timestamp.
    if let Some(t) = at_time_unix_secs {
        triples.push(Triple::new(
            activity,
            prov("atTime"),
            Term::Literal(Literal::new_typed_literal(t.to_string(), xsd::INTEGER)),
        ));
    }
    StatusJustification { triples }
}

/// Lift a triple subject into an object term (named node or blank node).
fn subject_term(s: &NamedOrBlankNode) -> Term {
    match s {
        NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
    }
}

// --- dependency-free base decoders (hand-rolled, like did.rs's base58btc) ----------------

/// Decode base64url-no-pad (RFC 4648 §5, no `=` padding) — the current Bitstring-Status-List
/// multibase `u` form. Returns `None` on any non-alphabet character or a non-canonical
/// (non-zero) final-sextet remainder.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None, // including '=' (no-pad form) and base64-std '+'/'/'
        }
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut nbits: u32 = 0;
    for &b in bytes {
        let v = val(b)? as u32;
        acc = (acc << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    // Any leftover bits beyond a partial final sextet must be zero (canonical no-pad).
    if nbits > 0 && (acc & ((1u32 << nbits) - 1)) != 0 {
        return None;
    }
    Some(out)
}

/// Decode base58btc (Bitcoin alphabet) — the legacy StatusList2021 multibase `z` form.
/// Hand-rolled (dependency-free), matching the `did.rs` base58 decoder's semantics.
fn base58btc_decode(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    // Leading '1's encode leading zero bytes.
    let mut leading_zeros = 0usize;
    for &c in input.as_bytes() {
        if c == b'1' {
            leading_zeros += 1;
        } else {
            break;
        }
    }
    let mut bytes: Vec<u8> = Vec::new();
    for &c in input.as_bytes() {
        let digit = ALPHABET.iter().position(|&a| a == c)? as u32;
        let mut carry = digit;
        for byte in bytes.iter_mut() {
            carry += (*byte as u32) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // `bytes` is little-endian; reverse and prepend the leading zeros.
    let mut out = vec![0u8; leading_zeros];
    out.extend(bytes.iter().rev());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: u64) -> StatusListEntry {
        StatusListEntry {
            status_list_credential: NamedNode::new_unchecked("https://issuer.ex/status/list#list"),
            status_list_index: index,
            purpose: StatusPurpose::Revocation,
        }
    }

    /// Encode bytes as base64url-no-pad (test helper — the encoder side of the decoder).
    fn base64url_encode(input: &[u8]) -> String {
        const ALPHA: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        let mut acc: u32 = 0;
        let mut nbits: u32 = 0;
        for &b in input {
            acc = (acc << 8) | b as u32;
            nbits += 8;
            while nbits >= 6 {
                nbits -= 6;
                out.push(ALPHA[((acc >> nbits) & 0x3f) as usize] as char);
            }
        }
        if nbits > 0 {
            out.push(ALPHA[((acc << (6 - nbits)) & 0x3f) as usize] as char);
        }
        out
    }

    #[test]
    fn base64url_round_trips() {
        for case in [
            &b""[..],
            &b"f"[..],
            &b"fo"[..],
            &b"foo"[..],
            &b"foob"[..],
            &b"fooba"[..],
            &b"foobar"[..],
            &[0u8, 1, 2, 3, 0xff, 0x80][..],
        ] {
            let enc = base64url_encode(case);
            let dec = base64url_decode(&enc).expect("decodes");
            assert_eq!(dec, case, "round-trip for {:?}", case);
        }
    }

    #[test]
    fn base64url_rejects_non_alphabet_and_padding() {
        assert!(base64url_decode("ab=c").is_none(), "'=' padding rejected");
        assert!(base64url_decode("ab cd").is_none(), "space rejected");
        assert!(
            base64url_decode("ab+cd").is_none(),
            "'+' (base64 std) rejected"
        );
    }

    #[test]
    fn bitstring_is_msb_first_and_fail_closed_past_end() {
        // 0b1000_0000 → index 0 set (MSB-first), indices 1..=7 unset.
        let bs = StatusBitstring::from_bits(vec![0x80], 1000);
        assert!(bs.is_set(0), "MSB is index 0");
        for i in 1..8 {
            assert!(!bs.is_set(i), "index {} unset", i);
        }
        // Out of range ⇒ fail-closed set (at the bit level).
        assert!(bs.is_set(8), "out-of-range index reads as set");
        assert!(bs.is_set(1_000_000), "far out-of-range reads as set");
    }

    #[test]
    fn decode_encoded_list_u_prefix_identity() {
        // 0x80 = index-0-set; base64url-no-pad with the 'u' multibase prefix; identity codec.
        let encoded = format!("{}{}", MULTIBASE_BASE64URL, base64url_encode(&[0x80]));
        let bs = decode_encoded_list(&encoded, &IdentityGzipDecoder, 500).expect("decodes");
        assert!(bs.is_set(0));
        assert!(!bs.is_set(1));
        assert_eq!(bs.as_of_unix_secs(), 500);
    }

    #[test]
    fn decode_encoded_list_rejects_unknown_multibase() {
        // 'm' is base64-standard multibase, which this decoder does not accept.
        assert!(decode_encoded_list("mgA", &IdentityGzipDecoder, 0).is_none());
        assert!(decode_encoded_list("", &IdentityGzipDecoder, 0).is_none());
    }

    /// An in-memory resolver mapping the one list IRI to an encoded list + as_of.
    struct MapResolver {
        encoded: Option<(String, i64)>,
    }
    impl StatusListResolver for MapResolver {
        fn resolve(&self, _iri: &NamedNode) -> Option<(String, i64)> {
            self.encoded.clone()
        }
    }

    fn check_with(bits: &[u8], as_of: i64, max_age: i64, now: i64, index: u64) -> LiveStatus {
        let encoded = format!("{}{}", MULTIBASE_BASE64URL, base64url_encode(bits));
        let resolver = MapResolver {
            encoded: Some((encoded, as_of)),
        };
        let check = LiveStatusCheck::new(resolver, IdentityGzipDecoder, max_age);
        check.check(&entry(index), now)
    }

    #[test]
    fn live_when_unset_fresh_in_range() {
        // index 1 unset (byte 0x80 = only index 0 set), fresh snapshot.
        assert_eq!(check_with(&[0x80], 1000, 3600, 1500, 1), LiveStatus::Live);
    }

    #[test]
    fn set_bit_denies() {
        // index 0 set ⇒ Set (deny).
        let s = check_with(&[0x80], 1000, 3600, 1500, 0);
        assert_eq!(s, LiveStatus::Set);
        assert!(!s.admits());
    }

    #[test]
    fn stale_snapshot_denies_fail_closed() {
        // snapshot at 1000, now 99999, max_age 3600 ⇒ Stale.
        let s = check_with(&[0x00], 1000, 3600, 99999, 1);
        assert_eq!(s, LiveStatus::Stale);
        assert!(!s.admits());
    }

    #[test]
    fn future_snapshot_denies_fail_closed() {
        // snapshot timestamped in the future (now < as_of) ⇒ Stale (negative age).
        let s = check_with(&[0x00], 5000, 3600, 1000, 1);
        assert_eq!(s, LiveStatus::Stale);
    }

    #[test]
    fn non_positive_max_age_is_maximally_fail_closed() {
        // max_age 0 ⇒ nothing is ever fresh enough ⇒ Stale even at as_of == now.
        let s = check_with(&[0x00], 1000, 0, 1000, 1);
        assert_eq!(s, LiveStatus::Stale);
    }

    #[test]
    fn unresolvable_list_is_unknown_fail_closed() {
        let resolver = MapResolver { encoded: None };
        let check = LiveStatusCheck::new(resolver, IdentityGzipDecoder, 3600);
        let s = check.check(&entry(0), 1000);
        assert_eq!(s, LiveStatus::Unknown);
        assert!(!s.admits());
    }

    #[test]
    fn out_of_range_index_is_unknown_not_set() {
        // 1-byte list covers indices 0..=7; index 8 is not covered ⇒ Unknown, not Set.
        let s = check_with(&[0x00], 1000, 3600, 1500, 8);
        assert_eq!(s, LiveStatus::Unknown);
        assert!(!s.admits());
    }

    #[test]
    fn undecodable_list_is_unknown_fail_closed() {
        // A bad multibase prefix ('m') ⇒ the decoder returns None ⇒ Unknown.
        let resolver = MapResolver {
            encoded: Some(("mZZZ".to_string(), 1000)),
        };
        let check = LiveStatusCheck::new(resolver, IdentityGzipDecoder, 3600);
        assert_eq!(check.check(&entry(0), 1500), LiveStatus::Unknown);
    }

    #[test]
    fn only_live_admits() {
        assert!(LiveStatus::Live.admits());
        assert!(!LiveStatus::Set.admits());
        assert!(!LiveStatus::Unknown.admits());
        assert!(!LiveStatus::Stale.admits());
    }

    #[test]
    fn justification_records_the_decision_and_reason() {
        let grant = Triple::new(
            NamedNode::new_unchecked("https://jesse.ex/card#me"),
            NamedNode::new_unchecked("https://sparq.dev/ns/auth#read"),
            Term::NamedNode(NamedNode::new_unchecked("https://pod.ex/resourceX")),
        );
        // Deny case: a revoked status ⇒ the minimal denial justification.
        let j = justify_status_decision(&grant, &entry(7), LiveStatus::Set, Some(1500));
        let joined: String = j
            .triples()
            .iter()
            .map(|t| format!("{} {} {}\n", t.subject, t.predicate, t.object))
            .collect();
        // The decision token is the deny reason (minimal justification).
        assert!(
            joined.contains("statusDecision") && joined.contains("status-set"),
            "records the deny reason:\n{}",
            joined
        );
        // It records statusAdmits=false.
        assert!(
            joined.contains("statusAdmits") && joined.contains("false"),
            "records admits=false:\n{}",
            joined
        );
        // It binds the checked index (7) + purpose.
        assert!(joined.contains("statusListIndex"), "binds the index");
        assert!(joined.contains("revocation"), "binds the purpose");
        // PROV-O shape: a prov:Activity that generated a grant entity.
        assert!(joined.contains("prov#Activity"), "is a prov:Activity");
        assert!(joined.contains("prov#generated"), "generated the grant");
    }

    #[test]
    fn justification_allow_records_live() {
        let grant = Triple::new(
            NamedNode::new_unchecked("https://jesse.ex/card#me"),
            NamedNode::new_unchecked("https://sparq.dev/ns/auth#read"),
            Term::NamedNode(NamedNode::new_unchecked("https://pod.ex/resourceX")),
        );
        let j = justify_status_decision(&grant, &entry(1), LiveStatus::Live, None);
        let joined: String = j
            .triples()
            .iter()
            .map(|t| format!("{} {} {}\n", t.subject, t.predicate, t.object))
            .collect();
        assert!(!joined.contains("status-set"), "not a deny token");
        assert!(joined.contains("statusDecision"));
        assert!(joined.contains("statusAdmits") && joined.contains("true"));
    }

    #[test]
    fn base58btc_decodes_leading_zero() {
        // A single '1' decodes to one zero byte (leading-zero convention).
        let decoded = base58btc_decode("1").expect("single '1' decodes to one zero byte");
        assert_eq!(decoded, vec![0u8]);
    }

    /// The built-in `flate2` GZIP decoder inflates a REAL gzip-compressed `encodedList`
    /// (the spec-mandated codec) — exercised ONLY under the `status-list-flate2` feature.
    #[cfg(feature = "status-list-flate2")]
    #[test]
    fn flate2_decoder_inflates_a_real_gzip_encoded_list() {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write as _;

        // 0x80 = index-0-set, 0x00 = indices 8..=15 unset.
        let raw = [0x80u8, 0x00];
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&raw).unwrap();
        let gzipped = enc.finish().unwrap();
        // Multibase `u` (base64url) over the GZIP bytes — the real `encodedList` shape.
        let encoded = format!("{}{}", MULTIBASE_BASE64URL, base64url_encode(&gzipped));
        let bs = decode_encoded_list(&encoded, &Flate2GzipDecoder, 1000)
            .expect("decodes + inflates the real gzip list");
        assert!(bs.is_set(0), "MSB of byte 0 is index 0 (set)");
        assert!(!bs.is_set(1));
        assert!(!bs.is_set(8), "byte 1 is 0x00 ⇒ index 8 unset");
        assert_eq!(bs.len_bytes(), 2);
    }

    // --- incremental status delta (sq-pfae.14) -------------------------------------------

    /// The list IRI the delta tests bind (the same one `entry()` uses).
    fn list_iri() -> NamedNode {
        NamedNode::new_unchecked("https://issuer.ex/status/list#list")
    }

    #[test]
    fn delta_enumerates_the_changed_indices_msb_first() {
        // 0x80 → 0x81: index 0 unchanged (both set), index 7 flipped 0→1.
        let prev = StatusBitstring::from_bits(vec![0x80, 0x00], 1000);
        let next = StatusBitstring::from_bits(vec![0x81, 0x40], 1100);
        let d = StatusDelta::between(list_iri(), &prev, &next);
        // byte 0 bit 7 = index 7; byte 1 bit 1 = index 9.
        assert_eq!(d.changed_indices(), Some(&[7u64, 9][..]));
        assert!(!d.requires_full_recheck());
        assert_eq!(d.unbounded_reason(), None);
        assert_eq!(d.status_list_credential(), &list_iri());
    }

    #[test]
    fn delta_affects_only_the_changed_slots() {
        let prev = StatusBitstring::from_bits(vec![0x00], 1000);
        let next = StatusBitstring::from_bits(vec![0x01], 1100); // index 7 revoked
        let d = StatusDelta::between(list_iri(), &prev, &next);
        assert!(d.affects(&entry(7)), "the revoked slot must be re-checked");
        for i in 0..7 {
            assert!(!d.affects(&entry(i)), "index {} is untouched", i);
        }
    }

    #[test]
    fn delta_affects_a_reinstatement_too() {
        // 1→0 (a lifted suspension) WIDENS: it must be re-checked just like a revocation,
        // else a reinstated grant stays denied forever.
        let prev = StatusBitstring::from_bits(vec![0xff], 1000);
        let next = StatusBitstring::from_bits(vec![0xfe], 1100); // index 7 cleared
        let d = StatusDelta::between(list_iri(), &prev, &next);
        assert_eq!(d.changed_indices(), Some(&[7u64][..]));
        assert!(d.affects(&entry(7)));
    }

    #[test]
    fn delta_binds_one_list_only() {
        let prev = StatusBitstring::from_bits(vec![0x00], 1000);
        let next = StatusBitstring::from_bits(vec![0x80], 1100);
        let d = StatusDelta::between(list_iri(), &prev, &next);
        let other = StatusListEntry {
            status_list_credential: NamedNode::new_unchecked("https://other.ex/status#l"),
            status_list_index: 0,
            purpose: StatusPurpose::Revocation,
        };
        assert!(d.affects(&entry(0)), "same list, changed slot");
        assert!(!d.affects(&other), "a delta says nothing about another list");
    }

    #[test]
    fn delta_not_newer_snapshot_demands_full_recheck() {
        let prev = StatusBitstring::from_bits(vec![0x00], 1000);
        // Same as_of ⇒ not an epoch bump ⇒ fail closed.
        let same = StatusBitstring::from_bits(vec![0xff], 1000);
        let d = StatusDelta::between(list_iri(), &prev, &same);
        assert!(d.requires_full_recheck());
        assert_eq!(d.unbounded_reason(), Some(DeltaUnbounded::NotNewer));
        assert_eq!(d.changed_indices(), None);
        // Unbounded ⇒ EVERY entry on the list is affected (never silently skipped).
        assert!(d.affects(&entry(3)));
        // An older successor likewise.
        let older = StatusBitstring::from_bits(vec![0x00], 900);
        assert_eq!(
            StatusDelta::between(list_iri(), &prev, &older).unbounded_reason(),
            Some(DeltaUnbounded::NotNewer)
        );
    }

    #[test]
    fn delta_coverage_change_demands_full_recheck() {
        // A grown list moves indices 8..=15 from Unknown into range — a verdict change a
        // common-prefix diff cannot see ⇒ fail closed.
        let prev = StatusBitstring::from_bits(vec![0x00], 1000);
        let next = StatusBitstring::from_bits(vec![0x00, 0x00], 1100);
        let d = StatusDelta::between(list_iri(), &prev, &next);
        assert!(d.requires_full_recheck());
        assert_eq!(d.unbounded_reason(), Some(DeltaUnbounded::CoverageChanged));
        assert!(d.affects(&entry(9)));
        // A shrunk list is equally unbounded.
        let shrunk = StatusBitstring::from_bits(vec![0x00], 1200);
        assert_eq!(
            StatusDelta::between(list_iri(), &next, &shrunk).unbounded_reason(),
            Some(DeltaUnbounded::CoverageChanged)
        );
    }

    #[test]
    fn delta_over_the_limit_demands_full_recheck() {
        let prev = StatusBitstring::from_bits(vec![0x00], 1000);
        let next = StatusBitstring::from_bits(vec![0xff], 1100); // 8 changes
        // A limit of 7 is exceeded by 8 changes ⇒ fail closed rather than enumerate.
        let d = StatusDelta::between_with_limit(list_iri(), &prev, &next, 7);
        assert!(d.requires_full_recheck());
        assert_eq!(d.unbounded_reason(), Some(DeltaUnbounded::LimitExceeded));
        // Exactly at the limit still enumerates.
        let at = StatusDelta::between_with_limit(list_iri(), &prev, &next, 8);
        assert_eq!(at.changed_indices().map(<[u64]>::len), Some(8));
        // The default limit comfortably covers this case (checked at compile time — the
        // operand is a `const`, so a runtime `assert!` is a constant assertion).
        const { assert!(DEFAULT_MAX_CHANGED_INDICES >= 8) };
    }

    #[test]
    fn delta_of_an_unchanged_list_is_empty() {
        let prev = StatusBitstring::from_bits(vec![0xa5, 0x5a], 1000);
        let next = StatusBitstring::from_bits(vec![0xa5, 0x5a], 1100);
        let d = StatusDelta::between(list_iri(), &prev, &next);
        assert_eq!(d.changed_indices(), Some(&[][..]));
        for i in 0..16 {
            assert!(!d.affects(&entry(i)), "index {} unaffected", i);
        }
    }

    #[test]
    fn delta_unbounded_reason_tokens_are_distinct() {
        assert_eq!(DeltaUnbounded::NotNewer.reason(), "delta-not-newer");
        assert_eq!(
            DeltaUnbounded::CoverageChanged.reason(),
            "delta-coverage-changed"
        );
        assert_eq!(DeltaUnbounded::LimitExceeded.reason(), "delta-limit-exceeded");
    }

    /// **The load-bearing soundness guard.** Exhaustively over every pair of single-byte
    /// snapshots, every in-range index, and a `now` sweep that covers both-fresh,
    /// prev-aged-out, and future-successor instants, assert the documented skip contract:
    ///
    /// - under `valid_at(now, max_age) && !affects(entry)`, `verdict(prev).admits()` implies
    ///   `verdict(next).admits()` — skipping never leaves a grant admitted that the full
    ///   re-run would deny (the property that actually matters);
    /// - when BOTH snapshots are fresh, the stronger equality `verdict(prev) == verdict(next)`
    ///   holds;
    /// - a flagged slot really did move, so the selection is not a trivially-sound
    ///   over-approximation.
    #[test]
    fn unaffected_entries_are_safe_to_skip_across_the_bump() {
        const MAX_AGE: i64 = 3600;
        const PREV_AS_OF: i64 = 1000;
        const NEXT_AS_OF: i64 = 1100;
        // 1500: both fresh. 5000: prev aged out, successor still fresh. 900: successor is
        // FUTURE-dated (and prev fresh) — the case that breaks a naive `same verdict` claim.
        // 1_000_000: both aged out.
        for now in [1500i64, 5000, 900, 1_000_000] {
            for a in 0u16..=0xff {
                for b in 0u16..=0xff {
                    let prev = StatusBitstring::from_bits(vec![a as u8], PREV_AS_OF);
                    let next = StatusBitstring::from_bits(vec![b as u8], NEXT_AS_OF);
                    let d = StatusDelta::between(list_iri(), &prev, &next);
                    assert!(!d.requires_full_recheck(), "same-length newer snapshot");
                    let prev_fresh = (now - PREV_AS_OF) >= 0 && (now - PREV_AS_OF) <= MAX_AGE;
                    for index in 0..8u64 {
                        let e = entry(index);
                        let before = status_from_snapshot(&prev, &e, MAX_AGE, now);
                        let after = status_from_snapshot(&next, &e, MAX_AGE, now);
                        if d.affects(&e) {
                            // A flagged slot really did move (at an instant where the bits
                            // are what decides — i.e. both snapshots fresh).
                            if prev_fresh && d.valid_at(now, MAX_AGE) {
                                assert_ne!(
                                    before, after,
                                    "index {} flagged but unchanged for {:#04x}→{:#04x}",
                                    index, a, b
                                );
                            }
                            continue;
                        }
                        // Not flagged. The skip is only claimed under `valid_at`.
                        if !d.valid_at(now, MAX_AGE) {
                            // Whole list is stale — nothing may be skipped, and indeed the
                            // successor verdict is Stale for every slot.
                            assert_eq!(after, LiveStatus::Stale, "stale successor at {}", now);
                            continue;
                        }
                        assert!(
                            !before.admits() || after.admits(),
                            "skipping index {} at now={} would keep {:#04x}→{:#04x} admitted \
                             while the full re-run denies it",
                            index,
                            now,
                            a,
                            b
                        );
                        if prev_fresh {
                            assert_eq!(
                                before, after,
                                "skipped index {} changed verdict for {:#04x}→{:#04x}",
                                index, a, b
                            );
                        }
                    }
                }
            }
        }
    }

    /// The precise case that makes [`StatusDelta::valid_at`] load-bearing rather than
    /// decorative: a FUTURE-dated successor. No bit moved, so `affects` is `false` for every
    /// slot — yet the verdict flips `Live` → `Stale`. `valid_at` is what stops a caller
    /// skipping the re-check and keeping a now-stale grant admitted.
    #[test]
    fn valid_at_rejects_a_future_dated_successor_that_affects_cannot_see() {
        let prev = StatusBitstring::from_bits(vec![0x00], 1000);
        let next = StatusBitstring::from_bits(vec![0x00], 2000); // future relative to `now`
        let d = StatusDelta::between(list_iri(), &prev, &next);
        let e = entry(1);
        assert!(!d.affects(&e), "no bit moved, so the per-slot selection is silent");
        // Against `prev` the grant is Live; against `next` it is Stale — a skip would be wrong.
        assert_eq!(status_from_snapshot(&prev, &e, 3600, 1500), LiveStatus::Live);
        assert_eq!(status_from_snapshot(&next, &e, 3600, 1500), LiveStatus::Stale);
        // The whole-list precondition catches it: no skip is permitted at this instant.
        assert!(!d.valid_at(1500, 3600), "future successor is not valid to skip against");
        assert_eq!(d.successor_as_of_unix_secs(), 2000);
        // …and once `now` catches up, the delta becomes usable again.
        assert!(d.valid_at(2500, 3600));
        // A non-positive max_age is never valid (matching the gate's maximal fail-closed).
        assert!(!d.valid_at(2500, 0));
        // An aged-out successor is likewise not valid to skip against.
        assert!(!d.valid_at(1_000_000, 3600));
    }

    /// The delta is a SELECTION, not a verdict: `affects` cannot see staleness. An unchanged
    /// list whose snapshot has aged out flags NOTHING, yet every verdict flips to `Stale` —
    /// the documented §4.4 residue, pinned so it cannot be quietly overclaimed as closed.
    #[test]
    fn delta_does_not_capture_staleness_the_documented_residue() {
        let prev = StatusBitstring::from_bits(vec![0x00], 1000);
        let next = StatusBitstring::from_bits(vec![0x00], 1100);
        let d = StatusDelta::between(list_iri(), &prev, &next);
        assert_eq!(d.changed_indices(), Some(&[][..]), "no bit moved");
        assert!(!d.affects(&entry(1)));
        // `affects` stays silent even at an instant where everything is stale — which is
        // precisely why `valid_at` is the required other half of the skip contract.
        assert!(!d.affects(&entry(1)));
        assert!(!d.valid_at(1_000_000, 3600));
        // …yet at a far-future `now` the same entry is Stale against BOTH snapshots.
        assert_eq!(
            status_from_snapshot(&next, &entry(1), 3600, 1_000_000),
            LiveStatus::Stale
        );
        assert_eq!(
            status_from_snapshot(&next, &entry(1), 3600, 1500),
            LiveStatus::Live
        );
    }

    /// The delta composes with the REAL gate: an epoch bump that revokes one slot flags
    /// exactly that slot, and re-running the unchanged `LiveStatusCheck` over it denies.
    #[test]
    fn delta_selects_the_grant_the_real_gate_then_denies() {
        let prev = StatusBitstring::from_bits(vec![0x00], 1000);
        let next_bits = [0x40u8]; // index 1 revoked
        let next = StatusBitstring::from_bits(next_bits.to_vec(), 1100);
        let d = StatusDelta::between(list_iri(), &prev, &next);
        assert!(d.affects(&entry(1)));
        assert!(!d.affects(&entry(2)));
        // Re-check ONLY the affected entry through the unchanged gate ⇒ Set (deny).
        assert_eq!(check_with(&next_bits, 1100, 3600, 1500, 1), LiveStatus::Set);
        // The skipped entry is still Live, exactly as a full re-run would have found it.
        assert_eq!(check_with(&next_bits, 1100, 3600, 1500, 2), LiveStatus::Live);
    }

    // --- verified status-list VC: the list's OWN issuer signature (sq-pfae.13) ------------

    use sparq_zk::sig::{PublicKey, SecretKey};

    /// The status-authority key the verified tests sign / verify against.
    fn authority_key() -> (SecretKey, PublicKey) {
        let sk = SecretKey::from_seed(0x0bad_c0de_dead_beef);
        let pk = sk.public_key();
        (sk, pk)
    }

    /// Build the status-list VC claim graph carrying a multibase `encodedList` over `bits`.
    fn status_list_vc_graph(bits: &[u8]) -> Vec<Triple> {
        let encoded = format!("{}{}", MULTIBASE_BASE64URL, base64url_encode(bits));
        let subject = NamedNode::new_unchecked("https://issuer.ex/status/list#list");
        vec![
            Triple::new(
                subject.clone(),
                NamedNode::from(rdf::TYPE),
                Term::NamedNode(status("BitstringStatusList")),
            ),
            Triple::new(
                subject,
                status("encodedList"),
                Term::Literal(Literal::new_simple_literal(encoded)),
            ),
        ]
    }

    /// Sign the VC graph's RDFC-1.0 commitment with `sk`, returning a [`SignedStatusList`]
    /// — the SAME commit→sign path the admit gate uses on a presented credential.
    fn sign_status_list(graph: Vec<Triple>, sk: &SecretKey) -> SignedStatusList {
        use sparq_zk::commit::commit_triples;
        use sparq_zk::encode::salt_from_bytes;
        let salt_bytes = [7u8; 32];
        let commitment = commit_triples(&graph, salt_from_bytes(&salt_bytes)).expect("commits");
        let sig_hex = sk.sign_commitment(&commitment.commitment);
        SignedStatusList {
            graph,
            issuer_signature_hex: sig_hex,
            salt: salt_bytes,
        }
    }

    /// An in-memory resolver returning a fixed signed VC + as_of (or `None`).
    struct SignedMapResolver {
        signed: Option<(SignedStatusList, i64)>,
    }
    impl VerifiedStatusListResolver for SignedMapResolver {
        fn resolve_signed(&self, _iri: &NamedNode) -> Option<(SignedStatusList, i64)> {
            self.signed.clone()
        }
    }

    fn verifying_check_with(
        signed: Option<(SignedStatusList, i64)>,
        key: PublicKey,
        max_age: i64,
    ) -> VerifyingLiveStatusCheck<SignedMapResolver, IdentityGzipDecoder> {
        VerifyingLiveStatusCheck::new(
            SignedMapResolver { signed },
            IdentityGzipDecoder,
            key,
            max_age,
        )
    }

    #[test]
    fn verified_validly_signed_status_list_is_accepted() {
        let (sk, pk) = authority_key();
        // index 1 unset (byte 0x80 ⇒ only index 0 set), fresh snapshot.
        let signed = sign_status_list(status_list_vc_graph(&[0x80]), &sk);
        let check = verifying_check_with(Some((signed, 1000)), pk, 3600);
        // A validly-signed list is trusted: index 1 is Live, index 0 (set) denies.
        assert_eq!(check.check(&entry(1), 1500), LiveStatus::Live);
        assert_eq!(check.check(&entry(0), 1500), LiveStatus::Set);
    }

    #[test]
    fn verified_unsigned_status_list_is_rejected_fail_closed() {
        let (_sk, pk) = authority_key();
        // An empty signature hex never parses ⇒ fail-closed Unknown (NEVER trusted).
        let unsigned = SignedStatusList {
            graph: status_list_vc_graph(&[0x00]),
            issuer_signature_hex: String::new(),
            salt: [7u8; 32],
        };
        let check = verifying_check_with(Some((unsigned, 1000)), pk, 3600);
        // index 1 is unset in the bits, but the VC is unsigned ⇒ Unknown, never Live.
        let s = check.check(&entry(1), 1500);
        assert_eq!(s, LiveStatus::Unknown);
        assert!(
            !s.admits(),
            "an unsigned status-list VC must not be trusted"
        );
    }

    #[test]
    fn verified_tampered_bits_are_rejected_fail_closed() {
        let (sk, pk) = authority_key();
        // Sign the all-unset list, then TAMPER: swap in an all-set encodedList AFTER signing.
        let mut signed = sign_status_list(status_list_vc_graph(&[0x00]), &sk);
        let tampered_encoded = format!("{}{}", MULTIBASE_BASE64URL, base64url_encode(&[0xff]));
        for t in signed.graph.iter_mut() {
            if t.predicate == status("encodedList") {
                *t = Triple::new(
                    t.subject.clone(),
                    t.predicate.clone(),
                    Term::Literal(Literal::new_simple_literal(tampered_encoded.clone())),
                );
            }
        }
        let check = verifying_check_with(Some((signed, 1000)), pk, 3600);
        // The signature was over the ORIGINAL graph; the tampered graph no longer verifies
        // ⇒ Unknown (the attacker-supplied bits are rejected, never trusted as Live/Set).
        let s = check.check(&entry(1), 1500);
        assert_eq!(s, LiveStatus::Unknown);
        assert!(!s.admits());
    }

    #[test]
    fn verified_wrong_authority_key_is_rejected_fail_closed() {
        let (sk, _pk) = authority_key();
        let signed = sign_status_list(status_list_vc_graph(&[0x00]), &sk);
        // Verify against a DIFFERENT key than the one that signed ⇒ fail-closed Unknown.
        let other_pk = SecretKey::from_seed(0x1111_2222_3333_4444).public_key();
        let check = verifying_check_with(Some((signed, 1000)), other_pk, 3600);
        assert_eq!(check.check(&entry(1), 1500), LiveStatus::Unknown);
    }

    #[test]
    fn verified_unresolvable_list_is_unknown_fail_closed() {
        let (_sk, pk) = authority_key();
        let check = verifying_check_with(None, pk, 3600);
        let s = check.check(&entry(0), 1500);
        assert_eq!(s, LiveStatus::Unknown);
        assert!(!s.admits());
    }

    #[test]
    fn verified_graph_without_encoded_list_is_unknown_fail_closed() {
        let (sk, pk) = authority_key();
        // A VALIDLY-signed graph that carries no encodedList ⇒ fail-closed Unknown (a
        // verified VC that publishes no list cannot admit anything).
        let subject = NamedNode::new_unchecked("https://issuer.ex/status/list#list");
        let graph = vec![Triple::new(
            subject,
            NamedNode::from(rdf::TYPE),
            Term::NamedNode(status("BitstringStatusList")),
        )];
        let signed = sign_status_list(graph, &sk);
        let check = verifying_check_with(Some((signed, 1000)), pk, 3600);
        assert_eq!(check.check(&entry(0), 1500), LiveStatus::Unknown);
    }

    #[test]
    fn verified_stale_signed_snapshot_denies_fail_closed() {
        let (sk, pk) = authority_key();
        // Validly signed + unset bit, but the snapshot is older than max_age ⇒ Stale.
        let signed = sign_status_list(status_list_vc_graph(&[0x00]), &sk);
        let check = verifying_check_with(Some((signed, 1000)), pk, 3600);
        let s = check.check(&entry(1), 99999);
        assert_eq!(s, LiveStatus::Stale);
        assert!(!s.admits());
    }

    /// The `did`-bound authority-key constructor resolves the status authority's key from a
    /// `did:key` and verifies a list signed by the matching key — exercised under `did`.
    #[cfg(feature = "did")]
    #[test]
    fn verified_with_did_issuer_resolves_authority_key() {
        use crate::did::{did_key_for, DidKeyResolver};
        let (sk, pk) = authority_key();
        let authority_did = did_key_for(&pk);
        let signed = sign_status_list(status_list_vc_graph(&[0x80]), &sk);
        let check = VerifyingLiveStatusCheck::with_did_issuer(
            SignedMapResolver {
                signed: Some((signed, 1000)),
            },
            IdentityGzipDecoder,
            &DidKeyResolver,
            &authority_did,
            3600,
        )
        .expect("did:key resolves the authority key");
        // index 1 unset ⇒ Live (the DID-bound key verifies the signature).
        assert_eq!(check.check(&entry(1), 1500), LiveStatus::Live);
        assert_eq!(check.check(&entry(0), 1500), LiveStatus::Set);
    }

    /// A `did:key` authority whose DID does NOT match the signing key rejects the list
    /// (fail-closed) — the DID binding is enforced, not cosmetic.
    #[cfg(feature = "did")]
    #[test]
    fn verified_with_did_issuer_wrong_did_is_rejected() {
        use crate::did::{did_key_for, DidKeyResolver};
        let (sk, _pk) = authority_key();
        let signed = sign_status_list(status_list_vc_graph(&[0x00]), &sk);
        // A DID for a DIFFERENT key than signed the list.
        let other_pk = SecretKey::from_seed(0xaaaa_bbbb_cccc_dddd).public_key();
        let wrong_did = did_key_for(&other_pk);
        let check = VerifyingLiveStatusCheck::with_did_issuer(
            SignedMapResolver {
                signed: Some((signed, 1000)),
            },
            IdentityGzipDecoder,
            &DidKeyResolver,
            &wrong_did,
            3600,
        )
        .expect("did:key parses");
        assert_eq!(check.check(&entry(1), 1500), LiveStatus::Unknown);
    }
}
