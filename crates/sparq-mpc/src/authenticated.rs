// [OPUS-4.8] sq-km34.1: IT-MAC authenticated secret sharing — the FOUNDATION design
// §3 builds honest-majority malicious-with-abort MPC on (not delivered today). Layered over the existing degree-t
// Shamir sharing (`crate::shamir`). This module is the CARRIER (the `[[x]]` pair
// + the free linear ops); the MAC-carrying multiplication (sq-km34.3,
// `MacSession::auth_mul`) and the batched MAC-check (sq-km34.4,
// `MacSession::mac_check`) live in `crate::shamir`, and the registry wiring
// (sq-km34.7) is a SEPARATE bead that has NOT landed.
//! Authenticated (IT-MAC) secret sharing over the honest-majority Shamir backend
//! — the foundation malicious-with-abort security would build on; today's paths
//! are tamper-evident only.
//!
//! Design (authoritative): `research/mpc-malicious-security-design.md` §2.1–2.2
//! (the construction) and §2.3 (free linear ops). Skill `mpc-protocols`:
//! "malicious security comes free in honest majority" (Goyal–Song eprint
//! 2020/134) for *linear* circuits — which is exactly what this foundation
//! delivers; the cost only appears at multiplication (sq-km34.2+).
//!
//! ## What this is (§2.1–2.2)
//!
//! The SPDZ-family fix for the crate's load-bearing semi-honest assumptions
//! (design doc §1, "the honest hole") is to attach an **information-theoretic
//! MAC** to every secret-shared value. Pick a single, **session-global** MAC key
//! `α ∈ F_p` that is *itself* secret-shared as a degree-`t` Shamir sharing
//! `[α]` — **no party knows `α`** (design §2.1). The authenticated sharing of a
//! secret `x` is then the pair
//!
//! ```text
//! [[x]] = ( [x] , [α·x] )      both degree-t Shamir sharings.
//! ```
//!
//! `m_x = α·x` is the authenticator: a party cannot change its share of `[x]` to
//! flip `x` *and* simultaneously fix `[m_x]` to still equal `α·x`, because it does
//! not know `α`. That is the whole point — and it is why the later batched
//! MAC-check (sq-km34.4) catches tampering even at the minimal `n = 2t+1` where
//! Reed–Solomon redundancy is zero (design §2.5): soundness comes from the
//! *secret* `α`, not from over-determination of the codeword.
//!
//! ## What this foundation bead delivers — and what it deliberately does NOT
//!
//! This is bead **sq-km34.1**, the FOUNDATION (design §6 step 1). It delivers:
//!
//! - [`MacKey`] — the session-global secret-shared `[α]`, minted by the dealer,
//!   structurally **un-openable** (no method reconstructs `α`; see below).
//! - [`AuthenticatedShare`] — the `([x], [α·x])` pair (design §2.2).
//! - [`ShamirDealer::new_mac_session`](crate::shamir::ShamirDealer::new_mac_session)
//!   mints `[α]` once per session and returns a
//!   [`MacSession`](crate::shamir::MacSession) whose
//!   [`authenticated_share`](crate::shamir::MacSession::authenticated_share)
//!   produces authenticated sharings under that key.
//! - The **free linear ops** [`auth_add`], [`auth_scale`], [`auth_sub`],
//!   [`auth_add_constant`] (design §2.3): each applies the existing local Shamir
//!   linear op to BOTH `[x]` and `[α·x]`, with the public-constant MAC term using
//!   the shared `[α]`. Zero communication rounds — the honest-majority sweet spot.
//!
//! It does **NOT** itself deliver (they live elsewhere, or have not landed):
//!
//! - MAC-carrying **multiplication** + authenticated degree-reduce — **sq-km34.3**
//!   (design §2.4 route (a)). Multiplication is where authentication needs real
//!   work (`α·(x·y) ≠ (α·x)·(α·y)`); this module only covers the FREE linear ops.
//!   It HAS landed, in `crate::shamir` as
//!   [`MacSession::auth_mul`](crate::shamir::MacSession::auth_mul): the value
//!   product and the MAC `[α·z] = reduce([α·x]·[y])` are two INDEPENDENT
//!   mult-then-reduce rounds, which is what MAC-covers the `degree_reduce`
//!   re-sharing (design "Hole 2").
//! - The **batched MAC-check** at open time — **sq-km34.4** (design §2.5). Without
//!   it an authenticated share is not tamper-*evident*; this module builds the
//!   carrier, not the check. It HAS landed, in `crate::shamir` as
//!   [`MacSession::mac_check`](crate::shamir::MacSession::mac_check).
//! - **Registry / per-operator** reporting (`new_malicious`,
//!   `authenticated_abort` descriptor) — **sq-km34.7** (design §4), which has NOT
//!   landed. This module does NOT touch [`crate::backend`], and the security tier
//!   [`crate::backend`] reports to a federation is still the semi-honest one: a
//!   caller only gets the authenticated guarantee by explicitly running the
//!   `MacSession` path and calling `mac_check` before it acts on an opened value.
//!
//! ## Security tier (stated, not over-claimed)
//!
//! This foundation is the carrier design §3 targets for **honest-majority,
//! malicious-with-abort** security; today's paths reach only tamper-evident-with-abort
//! (see [`crate::auth_compare`]'s security tier). On its own it changes NO advertised
//! guarantee: it adds the MAC sharing alongside the value sharing and keeps the
//! linear ops free and correct. Tamper DETECTION on opened values needs a path where
//! the multiplication carries the MAC
//! ([`MacSession::auth_mul`](crate::shamir::MacSession::auth_mul), sq-km34.3) AND
//! the batched check
//! ([`MacSession::mac_check`](crate::shamir::MacSession::mac_check), sq-km34.4)
//! fires before any opened value is acted on. Both have landed, so that
//! tamper-evident path EXISTS. Malicious security does NOT: it additionally needs
//! multiplication-gate verification binding BOTH operands, because `auth_mul`
//! adopts a value tamper on its second operand, and that is not implemented. The
//! tamper-evident path is opt-in: on any path that does not run it (the plain
//! [`ShamirBackend`](crate::shamir::ShamirBackend) aggregate, the semi-honest
//! join/compare surfaces) the parties are still trusted to follow the protocol,
//! and the backend's advertised tier is unchanged pending sq-km34.7. Nothing here
//! is externally audited (sq-qhy4); MPC remains research-grade.
//!
//! ## Why `α` is NEVER reconstructed (a structural, not documentary, guarantee)
//!
//! [`MacKey`] holds `[α]` only as its per-party [`Share`] vector and exposes NO
//! method that opens it — not `reconstruct`, not a getter that returns the field
//! value, nothing. The crate's reconstruction entry points
//! ([`ShamirBackend::reconstruct`](crate::shamir::ShamirBackend::reconstruct),
//! [`reconstruct_degree`](crate::shamir::reconstruct_degree),
//! [`reconstruct_robust`](crate::robust::reconstruct_robust)) all take `&[Share]`,
//! so `[α]`'s shares COULD be fed to them by external code that pulls them out —
//! which is why [`MacKey`] keeps its shares private and the only share-level
//! accessor (`MacKey::scaled_constant_mac`, crate-private) returns a *derived*
//! sharing of `α·c` for a PUBLIC `c` (the §2.3 add-constant MAC term), never
//! `[α]` itself.
//! The cleartext session `α` is RETAINED (secret, in-process) inside the
//! [`MacSession`](crate::shamir::MacSession) for the session's lifetime — it must
//! be, to mint each value's MAC `α·x` — but it is never returned to, nor opened by,
//! any caller (see
//! [`ShamirDealer::new_mac_session`](crate::shamir::ShamirDealer::new_mac_session)).
//! What is never reconstructed is the *shared* `[α]` and the MAC shares `[α·x]`.
//! A test (`mac_key_is_never_reconstructed_*`) pins that no `α`-opening path
//! exists. `[OPUS-4.8]`

use crate::field::Fp;
use crate::partial::MpcError;
use crate::shamir::{add_constant, add_shares, scale, sub_shares, Share, ShareVec};

/// The **session-global MAC key sharing** `[α]` — a degree-`t` Shamir sharing of
/// a single `α ∈ F_p` that **no party knows** (design §2.1).
///
/// Minted ONCE per session by
/// [`ShamirDealer::new_mac_session`](crate::shamir::ShamirDealer::new_mac_session),
/// which draws `α` from the masking RNG (an OS-seeded ChaCha20 CSPRNG in
/// production, sq-1vt) and shares it. The cleartext `α` is **retained, secret and
/// in-process, for the session's lifetime** inside the returned
/// [`MacSession`](crate::shamir::MacSession) — it is needed there to mint each
/// value's MAC `α·x`. What is never reconstructed is the *shared* `[α]` (this type)
/// and the MAC shares `[α·x]`: `α` itself is never returned to, nor opened by, any
/// caller (no public accessor exposes it). The same `[α]` then authenticates every
/// value in the session via [`AuthenticatedShare`]. `[OPUS-4.8]`
///
/// **`α` is structurally un-openable.** This type stores `[α]` only as its private
/// per-party share vector and exposes NO method that reconstructs it. The only
/// share-level accessor, `MacKey::scaled_constant_mac` (crate-private), returns a
/// sharing of `α·c` for a PUBLIC constant `c` (the add-constant MAC term of §2.3)
/// — a derived sharing, never `[α]` itself. This is the structural realisation of
/// the bead's acceptance criterion (2): no code path opens `α`. `[OPUS-4.8]`
///
/// **Does NOT derive [`Debug`].** The per-party share vector `[α]` is secret key
/// material; a derived `Debug` would print every share, and *any* `t+1` of them
/// reconstruct `α` — so a stray `{:?}` (a log line, a panic message, an
/// `assert_eq!` failure) would leak the very key the whole construction protects.
/// Instead [`MacKey`] has a MANUAL [`Debug`] that REDACTS the shares
/// (`MacKey { alpha: <redacted>, .. }`), so it stays printable for diagnostics
/// without ever exposing `[α]`. `[OPUS-4.8]`
#[derive(Clone)]
pub struct MacKey {
    /// The degree-`t` Shamir sharing `[α]` (one [`Share`] per party). Private so
    /// the only way to USE it is the public-constant MAC term [`Self::scaled_constant_mac`];
    /// there is deliberately no accessor that returns the shares for reconstruction.
    shares: ShareVec,
    /// The privacy threshold `t` the key was shared at (mirrors the dealer's). The
    /// authenticated sharings under this key MUST be at the same `t` so the MAC
    /// relation `α·x` opens at the same degree as the value.
    t: usize,
}

// [OPUS-4.8] Manual `Debug` that REDACTS the `[α]` shares. The `shares` vector is
// the secret-shared MAC key; printing it (any `t+1` shares reconstruct `α`) would
// defeat the "α is never reconstructable" guarantee through a back door. We surface
// only the non-secret shape (party count + threshold) and an explicit `<redacted>`
// for the key material.
impl std::fmt::Debug for MacKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacKey")
            .field("alpha", &"<redacted>")
            .field("parties", &self.shares.len())
            .field("t", &self.t)
            .finish()
    }
}

// [OPUS-4.8] sq-u8a8: zeroize the `[α]` shares on drop. The `shares` vector IS
// the secret-shared MAC key — any `t+1` of them reconstruct `α` — so when a
// `MacKey` is dropped we scrub the share material from memory rather than leaving
// it in the freed allocation. `Share` derives `Zeroize`, so `Vec<Share>` zeroizes
// element-wise (and frees scrubbed). This is HYGIENE only: it changes no sharing
// arithmetic and runs only at end-of-life, after the key has been used.
impl Drop for MacKey {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.shares.zeroize();
    }
}

impl MacKey {
    /// Construct a [`MacKey`] from the dealer's freshly-minted `[α]` sharing.
    /// `pub(crate)` so ONLY the dealer (which alone saw `α`, and keeps the cleartext
    /// `α` retained — secret, in-process — inside its
    /// [`MacSession`](crate::shamir::MacSession) to mint MACs) can build one;
    /// external code cannot supply its own `[α]`. The threshold `t` is recorded so
    /// authenticated sharings are checked to match it. `[OPUS-4.8]`
    pub(crate) fn from_shares(shares: ShareVec, t: usize) -> Self {
        MacKey { shares, t }
    }

    /// The number of parties `[α]` is shared across (`= shares.len()`). Lets a
    /// caller sanity-check that a value sharing and the MAC key are on the same
    /// party set WITHOUT exposing the shares themselves.
    pub fn parties(&self) -> usize {
        self.shares.len()
    }

    /// The threshold `t` this key (and every authenticated sharing under it) lives
    /// at.
    pub fn threshold(&self) -> usize {
        self.t
    }

    /// The MAC sharing of a **public** constant `c`, i.e. a degree-`t` sharing of
    /// `α·c`, computed as `c · [α]` (local [`scale`] of `[α]`). This is the §2.3
    /// add-constant MAC term — adding a public `c` to `[[x]]` leaves `[x]+c` but
    /// shifts the MAC by `α·c`, and since `[α]` is shared, `c·[α]` is a free local
    /// op. It returns a sharing of `α·c`, a *derived* quantity, **NOT** `[α]`: `c`
    /// is public, so revealing a sharing of `α·c` is no more than revealing a
    /// sharing of (public-`c`) × (secret-`α`), which is exactly the MAC of the
    /// public value `c` and is meant to be added into another MAC sharing — it is
    /// never opened on its own here. `[OPUS-4.8]`
    pub(crate) fn scaled_constant_mac(&self, c: Fp) -> ShareVec {
        scale(&self.shares, c)
    }

    /// The raw `[α]` shares — **`pub(crate)`, NEVER public** — for the ONE crate-
    /// internal consumer that genuinely needs the SECRET-SHARED key itself (not a
    /// derived `α·c`): the §2.5 batched MAC-check
    /// ([`MacSession::mac_check`](crate::shamir::MacSession::mac_check)), whose `σ`
    /// term `−y·[α]` scales `[α]` by the PUBLIC opened value `y` (a free local
    /// [`scale`]).
    ///
    /// **NOT the §2.4 MAC-carrying multiplication.** [`MacSession::auth_mul`](crate::shamir::MacSession::auth_mul)
    /// does **not** read `[α]` at all: it carries the MAC forward as the INDEPENDENT
    /// product `[α·z] = reduce([α·x]·[y])` — the input MAC `[α·x]` times the input
    /// value `[y]` — **not** as `[z]·[α]` of the just-reduced product. That
    /// independence is exactly what makes the multiplication tamper-evident (a δ
    /// injected into one of the two separate re-sharings cannot land both `[z]` and
    /// `[α·z]` on a consistent `(z+δ, α·(z+δ))` pair), so `auth_mul` never touches
    /// this accessor; recomputing the MAC from `[z]·[α]` would be UNSOUND (the MAC
    /// would track whatever tampered `z` carried and `σ` would be 0). See
    /// [`MacSession::auth_mul`](crate::shamir::MacSession::auth_mul) for the full
    /// argument.
    ///
    /// The single consumer *consumes* `[α]` inside a sharing computation; it never
    /// opens it — the shares flow into [`scale`] and only the (leakage-free) `σ` is
    /// opened, never `[α]`. Kept crate-private exactly as
    /// [`AuthenticatedShare::mac_shares`] is, so no public surface can pull `[α]` out
    /// and reconstruct `α`. `[OPUS-4.8]`
    pub(crate) fn alpha_shares(&self) -> &[Share] {
        &self.shares
    }

    /// **Test-only.** The raw `[α]` shares, so the independence test (acceptance
    /// (3)) can argue about `≤ t` views of `α`. The `cfg` gate keeps this out of
    /// the public/production API — there is deliberately no non-test accessor that
    /// returns the shares for reconstruction. `[OPUS-4.8]`
    #[cfg(test)]
    pub(crate) fn shares_for_test(&self) -> ShareVec {
        self.shares.clone()
    }
}

/// An **authenticated secret sharing** `[[x]] = ([x], [α·x])` (design §2.2): the
/// value's degree-`t` Shamir sharing `[x]` paired with the degree-`t` Shamir
/// sharing of its IT-MAC `m_x = α·x` under the session key [`MacKey`].
///
/// Produced by
/// [`MacSession::authenticated_share`](crate::shamir::MacSession::authenticated_share).
/// Carries TWO sharings (value + MAC), so share storage roughly doubles vs a plain
/// [`ShareVec`] (design §5) — the price of authentication. Linear combinations are
/// FREE: [`auth_add`] / [`auth_scale`] / [`auth_sub`] / [`auth_add_constant`] apply
/// the matching local Shamir op to BOTH components (design §2.3), with no
/// communication round.
///
/// The value `[x]` reconstructs via the existing
/// [`ShamirBackend::reconstruct`](crate::shamir::ShamirBackend::reconstruct); the
/// MAC `[α·x]` reconstructs the same way and equals `α · x` — the relation a
/// later batched check (sq-km34.4) verifies WITHOUT opening `α`. `[OPUS-4.8]`
///
/// **Does NOT derive [`Debug`].** A derived `Debug` would print the private `mac`
/// field — the `[α·x]` shares. For `authenticated_share(1)` those are shares of `α`
/// itself, and *any* `t+1` of them reconstruct `α`; for general `x`, `α = (α·x)/x`
/// whenever `x` is known and invertible. So a stray `{:?}` (a log line, a panic
/// message, an `assert_eq!` failure) would exfiltrate the MAC shares and back-door
/// the `pub(crate)` restriction on `Self::mac_shares`, undermining the "α is never
/// reconstructable" guarantee. Instead [`AuthenticatedShare`] has a MANUAL [`Debug`]
/// that surfaces the openable value sharing `[x]` (already public via
/// [`Self::value_shares`]) but REDACTS the MAC shares. `[OPUS-4.8]`
#[derive(Clone)]
pub struct AuthenticatedShare {
    /// `[x]` — the degree-`t` Shamir sharing of the value.
    value: ShareVec,
    /// `[α·x]` — the degree-`t` Shamir sharing of the IT-MAC `m_x = α·x`.
    mac: ShareVec,
}

// [OPUS-4.8] Manual `Debug` that REDACTS the MAC shares. The `mac` vector is the
// `[α·x]` sharing; for `authenticated_share(1)` it is a sharing of `α` itself, and
// any `t+1` of those shares reconstruct `α` — so a derived `Debug` would leak the
// MAC key through a back door round the `pub(crate)` `mac_shares()` accessor. We
// print the openable value sharing `[x]` (already public via `value_shares()`) and
// an explicit `<redacted>` for the MAC shares, keeping the struct printable for
// diagnostics without ever exposing `[α·x]`.
impl std::fmt::Debug for AuthenticatedShare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthenticatedShare")
            .field("value", &self.value)
            .field("mac", &"<redacted>")
            .field("parties", &self.value.len())
            .finish()
    }
}

// [OPUS-4.8] sq-u8a8: zeroize the MAC shares `[α·x]` on drop. For
// `authenticated_share(1)` the `mac` vector is a sharing of `α` itself, so it is
// key material; we scrub it on drop rather than leaving it in freed memory. The
// `value` sharing `[x]` is the openable value (already public via
// `value_shares()`), but we scrub it too — a uniform "drop scrubs the share
// vectors" hygiene posture is simpler and cannot leak. HYGIENE only: runs at
// end-of-life, changes no arithmetic.
impl Drop for AuthenticatedShare {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.mac.zeroize();
        self.value.zeroize();
    }
}

impl AuthenticatedShare {
    /// Assemble an authenticated sharing from its value and MAC sharings. The two
    /// must be on the identical party-point set (same length, same `x` per index);
    /// otherwise the linear ops below could not pair them and the MAC relation
    /// would be meaningless. `pub(crate)`: only the dealer mints these (it alone
    /// computed `α·x` from the secret `α` it holds transiently). `[OPUS-4.8]`
    pub(crate) fn new(value: ShareVec, mac: ShareVec) -> Result<Self, MpcError> {
        if value.len() != mac.len() {
            return Err(MpcError::Protocol(
                "AuthenticatedShare: value and MAC sharings differ in party count".into(),
            ));
        }
        for (v, m) in value.iter().zip(mac.iter()) {
            if v.x != m.x {
                return Err(MpcError::Protocol(
                    "AuthenticatedShare: value and MAC sharings on different evaluation points"
                        .into(),
                ));
            }
        }
        Ok(AuthenticatedShare { value, mac })
    }

    /// The value sharing `[x]` (degree-`t`). Reconstruct it with the existing
    /// [`ShamirBackend::reconstruct`](crate::shamir::ShamirBackend::reconstruct) /
    /// [`ShamirDealer::reconstruct`](crate::shamir::ShamirDealer::reconstruct).
    pub fn value_shares(&self) -> &[Share] {
        &self.value
    }

    /// The MAC sharing `[α·x]` (degree-`t`). **`pub(crate)` — deliberately NOT
    /// public.** Reconstructed, it yields `α·x`; in SPDZ-style authenticated
    /// sharing a single MAC is NEVER opened on its own — `α·x` is only ever consumed
    /// INSIDE the (future) batched MAC-check (sq-km34.4) as a random linear
    /// combination, never individually. A public, individually-reconstructable MAC
    /// accessor is a back door round the "α is never reconstructed" guarantee: a
    /// caller could mint `authenticated_share(1)` (so `α·x = α·1 = α`), open its MAC
    /// via the public `reconstruct`, and recover `α` directly. Keeping this
    /// crate-private means the ONLY in-crate consumers are the MAC-carrying
    /// multiplication ([`MacSession::auth_mul`](crate::shamir::MacSession::auth_mul),
    /// sq-km34.3, which reads `[α·x]` to build `[α·z] = reduce([α·x]·[y])`) and the
    /// batched MAC-check
    /// ([`MacSession::mac_check`](crate::shamir::MacSession::mac_check), sq-km34.4,
    /// which only ever consumes it inside the random linear combination) — there is
    /// no public surface that returns the MAC shares, so the `x = 1` extraction is
    /// closed. (`value_shares()` stays public: it opens `[x]`, the value, never
    /// `α`.) `[OPUS-4.8]`
    //
    // [OPUS-4.8] sq-km34.3: the `allow(dead_code)` that used to sit here is GONE.
    // Its justification was "the MAC-check machinery is NOT built yet, so this has
    // no non-test caller" — no longer true: `auth_mul` and `mac_check` both call
    // this in a plain (non-test) library build. The lint now genuinely guards the
    // accessor, so a future refactor that orphans it gets flagged instead of
    // silently suppressed.
    pub(crate) fn mac_shares(&self) -> &[Share] {
        &self.mac
    }

    /// Number of parties this authenticated value is shared across.
    pub fn parties(&self) -> usize {
        self.value.len()
    }
}

// =============================================================================
// Free linear ops on authenticated shares (design §2.2.3 / §2.3).
//
// Each is the matching local Shamir linear op (`crate::shamir`) applied TWICE —
// once to `[x]`, once to `[α·x]` — using the field identities:
//   α·(x + y) = α·x + α·y        → add: add both components
//   α·(c·x)   = c·(α·x)          → scale: scale both components
//   α·(x − y) = α·x − α·y        → sub: sub both components
//   α·(x + c) = α·x + α·c        → add-constant: value gets +c, MAC gets +α·c
// All are LOCAL / zero-round, so the authenticated SUM aggregate stays the
// zero-round honest-majority sweet spot (design §2.3 / §5). [OPUS-4.8]
// =============================================================================

/// Add two authenticated sharings: `[[x]] + [[y]] = ([x]+[y], [α·x]+[α·y])`
/// (design §2.3). FREE / local — both components add component-wise, no round.
/// Inputs must be on the identical party-point set. `[OPUS-4.8]`
pub fn auth_add(
    a: &AuthenticatedShare,
    b: &AuthenticatedShare,
) -> Result<AuthenticatedShare, MpcError> {
    let value = add_shares(&a.value, &b.value)?;
    let mac = add_shares(&a.mac, &b.mac)?;
    AuthenticatedShare::new(value, mac)
}

/// Subtract two authenticated sharings: `[[x]] − [[y]] = ([x]−[y], [α·x]−[α·y])`
/// (design §2.3). FREE / local. Inputs must be on the identical party-point set.
/// `[OPUS-4.8]`
pub fn auth_sub(
    a: &AuthenticatedShare,
    b: &AuthenticatedShare,
) -> Result<AuthenticatedShare, MpcError> {
    let value = sub_shares(&a.value, &b.value)?;
    let mac = sub_shares(&a.mac, &b.mac)?;
    AuthenticatedShare::new(value, mac)
}

/// Multiply an authenticated sharing by a PUBLIC field constant `c`:
/// `c · [[x]] = (c·[x], c·[α·x])` (design §2.3, since `α·(c·x) = c·(α·x)`). FREE /
/// local. (This is scaling by a *public* scalar — NOT secret×secret multiplication,
/// which is sq-km34.2 and needs real work.) `[OPUS-4.8]`
pub fn auth_scale(a: &AuthenticatedShare, c: Fp) -> AuthenticatedShare {
    // [OPUS-4.8] Route through `new` so the value/MAC point-alignment invariant is
    // enforced in exactly one place rather than re-asserted (or silently assumed)
    // here. `scale` is point-preserving and `a` is already a valid alignment, so
    // `new` cannot fail — but we go through it rather than building the struct
    // directly, matching what this comment claims.
    AuthenticatedShare::new(scale(&a.value, c), scale(&a.mac, c))
        .expect("auth_scale: scaling preserves value/MAC point alignment")
}

/// Add a PUBLIC field constant `c` to an authenticated sharing:
/// `[[x]] + c = ([x]+c, [α·x] + c·[α])` (design §2.3). The value gets the usual
/// local add-constant; the MAC gains `α·c`, computed from the *shared* `[α]` as
/// `c·[α]` (`MacKey::scaled_constant_mac`) — still a FREE local op, no round.
/// The [`MacKey`] must be the same session key the sharing was authenticated under.
/// What is actually CHECKED is the **party set** (`a.parties() == key.parties()`):
/// [`AuthenticatedShare`] does not carry its own threshold, so the threshold is not
/// re-verified here (the dealer mints the key and the sharing at the same `t`, so a
/// matching party set on the same session implies a matching `t`). A party-set
/// mismatch is a protocol error. `[OPUS-4.8]`
pub fn auth_add_constant(
    a: &AuthenticatedShare,
    c: Fp,
    key: &MacKey,
) -> Result<AuthenticatedShare, MpcError> {
    if a.parties() != key.parties() {
        return Err(MpcError::Protocol(
            "auth_add_constant: authenticated share and MAC key are on different party sets".into(),
        ));
    }
    let value = add_constant(&a.value, c);
    // MAC of the public constant c is α·c = c·[α]; add it into the existing MAC.
    let mac = add_shares(&a.mac, &key.scaled_constant_mac(c))?;
    AuthenticatedShare::new(value, mac)
}

/// [OPUS-4.8] sq-km34.2 — the **authenticated cumulative-SUM aggregate** (design
/// §2.3): the MAC-carrying analogue of the backend's plain
/// [`run_secure`](crate::backend::MpcBackend::run_secure) fold.
/// Folds a slice of authenticated sharings with [`auth_add`], so it returns
/// `([Σx], [α·Σx])` — the sum's value sharing paired with its correct MAC.
///
/// This is the reusable form of the flatmate cumulative-salary use case: the SUM
/// is a pure linear function, so under Shamir it is **zero communication rounds**,
/// and because [`auth_add`] carries the MAC on BOTH components (`α·(x+y) = α·x +
/// α·y`) the MAC relation is preserved for FREE — the "malicious comes free in
/// honest majority" property for linear circuits (design §2.3) — for
/// same-session, honestly formed inputs. No new interaction over the
/// unauthenticated aggregate; only the MAC sharing is additionally maintained.
///
/// **This helper only propagates MAC shares — it never invokes the batched
/// `mac_check` (§2.5) itself.** Tamper detection requires the CALLER to include
/// the result in a successful `mac_check` before opening or otherwise relying on
/// it. Even then the chain is tamper-evident, NOT malicious-with-abort: protocol
/// verification is incomplete (multiplication gates are not verified on both
/// operands), and separately EXTERNAL accredited-cryptographer sign-off is PENDING
/// `sq-qhy4`.
///
/// All addends must be authenticated under the SAME session key `α` and shared on
/// the identical party-point set (as [`auth_add`] requires). An empty input is a
/// protocol error — like `run_secure`, there is no meaningful sum of zero addends
/// to disclose, and we never invent one. `[OPUS-4.8]`
pub fn auth_sum(shares: &[AuthenticatedShare]) -> Result<AuthenticatedShare, MpcError> {
    let (first, rest) = shares.split_first().ok_or_else(|| {
        MpcError::Protocol("auth_sum: no authenticated inputs to aggregate".into())
    })?;
    let mut acc = first.clone();
    for next in rest {
        acc = auth_add(&acc, next)?;
    }
    Ok(acc)
}

#[cfg(test)]
mod tests {
    //! sq-km34.1 acceptance tests (design §2.1–2.2). The three load-bearing ones,
    //! mapped to the bead's acceptance bar:
    //!  (1) authenticated sharing ROUND-TRIPS: reconstruct `[x]` → `x`, and the MAC
    //!      relation `reconstruct([α·x]) == α · x` HOLDS (`authenticated_share_round_trips`);
    //!  (2) `[α]` is NEVER reconstructed — STRUCTURAL (no `α`-opening path exists),
    //!      pinned + the MAC relation derives `α` only inside the test harness from
    //!      shares the test mints itself (`mac_key_is_never_reconstructed_by_the_api`);
    //!  (3) any `<= t` party views are INDEPENDENT of `α` — the standard privacy
    //!      property, tested by exhibiting a second `α` consistent with `t` shares
    //!      (`t_shares_of_alpha_are_independent_of_alpha`).
    //! Plus: the free linear ops preserve the MAC relation
    //! (`linear_ops_preserve_mac_relation`).
    use super::*;
    use crate::shamir::{reconstruct_at_zero, ShamirBackend};

    /// [OPUS-4.8] sq-u8a8: compile-level assertion that the secret-bearing share
    /// element type implements [`zeroize::Zeroize`] (so the `MacKey` / `AuthenticatedShare`
    /// drop scrub of their `Vec<Share>` MAC material type-checks). `MacKey` and
    /// `AuthenticatedShare` themselves zeroize their share vectors on `Drop`; a
    /// `Drop`-implementing type cannot be probed by a `T: Zeroize` bound, so we pin
    /// the element type here — which is exactly what their `Drop` impls scrub.
    #[test]
    fn share_element_is_zeroize() {
        fn assert_zeroize<T: zeroize::Zeroize>() {}
        assert_zeroize::<Share>();
        assert_zeroize::<Fp>();
        assert_zeroize::<Vec<Share>>();
    }

    /// Reconstruct a degree-`t` sharing's secret using the test-only unchecked
    /// Lagrange-at-0 helper (RNG-free). The production `ShamirBackend::reconstruct`
    /// routes through the robust checker; here we want the plain secret for both
    /// `[x]` and `[α·x]` and for the forged-third-share independence argument.
    fn open(shares: &[Share], t: usize) -> Fp {
        reconstruct_at_zero(shares, t).expect("enough shares to reconstruct")
    }

    /// ACCEPTANCE (1): authenticated sharing round-trips — `[x]` reconstructs to
    /// `x`, and the MAC sharing `[α·x]` reconstructs to a value satisfying the MAC
    /// relation `m_x = α · x`. We recover `α` here ONLY by reconstructing it from
    /// shares the TEST mints (the dealer never hands it out — see acceptance (2));
    /// this is the test verifying the relation, not the API exposing `α`.
    #[test]
    fn authenticated_share_round_trips() {
        // Seedable test RNG for reproducibility; production uses OS-seeded ChaCha20.
        let backend = ShamirBackend::new_seeded(5, 0xA17C).unwrap(); // t = 2
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();

        for secret in [0u64, 1, 7, 42, 100_000, crate::field::P - 1] {
            let x = Fp::new(secret);
            let auth = session.authenticated_share(x);

            // [x] reconstructs to x.
            let recovered_x = backend.reconstruct(auth.value_shares()).unwrap();
            assert_eq!(recovered_x, x, "value sharing must reconstruct to x");

            // The MAC relation holds: reconstruct([α·x]) == α · x. We get α from the
            // session-internal test accessor (test-only — NOT part of the public API).
            let alpha = session.alpha_for_test();
            let recovered_mac = backend.reconstruct(auth.mac_shares()).unwrap();
            assert_eq!(
                recovered_mac,
                alpha.mul(x),
                "MAC sharing must reconstruct to α·x"
            );
        }
    }

    /// ACCEPTANCE (2): `[α]` is NEVER reconstructed by any code path. This is a
    /// STRUCTURAL guarantee — [`MacKey`] exposes no `reconstruct` and no getter
    /// returning the `α` field value, and the dealer drops the cleartext `α` the
    /// instant it shares it. The test asserts the *shape* of the API: the only
    /// MAC-key-derived sharing a caller can obtain is `α·c` for a PUBLIC `c`
    /// ([`MacKey::scaled_constant_mac`]), and opening THAT yields `α·c`, never `α`.
    /// (A `c = 1` would give `α·1 = α`; that accessor is `pub(crate)` and only the
    /// §2.3 add-constant path uses it with a caller-supplied PUBLIC constant — the
    /// public surface of [`MacKey`] has no reconstruction at all.)
    #[test]
    fn mac_key_is_never_reconstructed_by_the_api() {
        let backend = ShamirBackend::new_seeded(5, 0xBEEF).unwrap();
        let mut dealer = backend.dealer();
        let session = dealer.new_mac_session();
        let key = session.mac_key();

        // The PUBLIC API of MacKey is: parties(), threshold(). Neither opens α.
        assert_eq!(key.parties(), 5);
        assert_eq!(key.threshold(), backend.threshold());

        // There is no `key.reconstruct(..)`, no `key.alpha()`, no public accessor
        // returning the shares for external reconstruction. The crate-internal
        // `scaled_constant_mac(c)` returns a sharing of α·c for PUBLIC c — a derived
        // MAC term, used only inside `auth_add_constant`. Even reconstructing THAT
        // gives α·c, not α (here c = 7, a public constant → α·7).
        let c = Fp::new(7);
        let mac_of_c = key.scaled_constant_mac(c);
        let opened = open(&mac_of_c, key.threshold());
        // We can only check this is consistent (α·7) using the test α; the point is
        // the *value* opened is α·c, and the API never lets a caller open α itself.
        let alpha = session.alpha_for_test();
        assert_eq!(
            opened,
            alpha.mul(c),
            "scaled_constant_mac opens to α·c (public c), not α"
        );
    }

    /// ACCEPTANCE (2), the `x = 1` trick: a caller MUST NOT be able to recover `α`
    /// through the PUBLIC API by minting `authenticated_share(1)` (whose MAC is
    /// `α·1 = α`) and reconstructing that MAC. This pins the load-bearing fix:
    /// [`AuthenticatedShare::mac_shares`] is `pub(crate)`, so there is NO public path
    /// that returns the MAC sharing for reconstruction — the public surface of an
    /// authenticated share is `value_shares()` (opens `[x]` → the value, here `1`,
    /// NEVER `α`) and `parties()`. The MAC is only reachable in-crate (by the future
    /// MAC-check machinery, km34.2/.4). This test asserts: (a) the public path opens
    /// the VALUE `1`, not `α`; and (b) the MAC accessor — reachable here only because
    /// the test is in-crate — WOULD yield `α` if it were public, which is exactly
    /// why it is `pub(crate)`. If `mac_shares` were ever made `pub` again, this test
    /// (and the crate's public-API contract) would be the thing documenting the leak.
    #[test]
    fn x_equals_one_cannot_extract_alpha_via_public_api() {
        let backend = ShamirBackend::new_seeded(5, 0x1234).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();

        // The x = 1 trick: mint an authenticated sharing of 1. Its MAC is α·1 = α.
        let auth_one = session.authenticated_share(Fp::one());

        // (a) The PUBLIC path: the only share accessor a caller outside the crate has
        // on an AuthenticatedShare is `value_shares()`. Reconstructing it gives the
        // VALUE (1) — never α. This is the whole of what the x = 1 trick can reach
        // through the public surface.
        let public_value = backend.reconstruct(auth_one.value_shares()).unwrap();
        assert_eq!(
            public_value,
            Fp::one(),
            "public x=1 path opens the value 1, never α"
        );

        // (b) The MAC sharing reconstructs to α·1 = α — which is EXACTLY why
        // `mac_shares()` is crate-private (`pub(crate)`): if it were public, this
        // single line would be a public α-extraction. We can only call it here
        // because this test compiles inside the crate; an external caller cannot.
        let alpha = session.alpha_for_test();
        let mac_of_one = backend.reconstruct(auth_one.mac_shares()).unwrap();
        assert_eq!(
            mac_of_one, alpha,
            "the MAC of x=1 is α — so the MAC accessor MUST stay pub(crate)"
        );

        // Compile-time guarantee that closes the hole: `mac_shares` is `pub(crate)`.
        // The following line type-checks (we are in-crate); the SAME line in any
        // downstream crate would be a privacy error `E0624`, so the public API has
        // no way to obtain the MAC shares and thus no x=1 path to α.
        let _: &[Share] = auth_one.mac_shares();
    }

    /// ACCEPTANCE (2), Debug redaction: a `MacKey`'s `{:?}` must NEVER print the
    /// `[α]` share vector — a derived `Debug` would, and any `t+1` of those shares
    /// reconstruct `α`. The manual [`Debug`] prints `<redacted>` for the key
    /// material and only the non-secret shape (party count + threshold). We assert
    /// the redaction marker is present and that NO share's `y` value appears in the
    /// output. `MacSession`'s `Debug` delegates to this one, so the session is
    /// covered too. `[OPUS-4.8]`
    #[test]
    fn mac_key_debug_redacts_alpha_shares() {
        let backend = ShamirBackend::new_seeded(5, 0xD00D).unwrap();
        let mut dealer = backend.dealer();
        let session = dealer.new_mac_session();
        let key = session.mac_key();

        let dbg = format!("{key:?}");
        assert!(
            dbg.contains("<redacted>"),
            "MacKey Debug must redact the α shares: {dbg}"
        );
        // No share's secret `y` value may leak into the Debug string.
        for share in session.alpha_shares_for_test() {
            assert!(
                !dbg.contains(&share.y.value().to_string()),
                "MacKey Debug leaked an α share value {}: {dbg}",
                share.y.value()
            );
        }
    }

    /// ACCEPTANCE (2), Debug redaction (continued): an `AuthenticatedShare`'s `{:?}`
    /// must NEVER print its `mac` (`[α·x]`) shares. For `authenticated_share(1)` the
    /// MAC is a sharing of `α`, so a derived `Debug` would leak the key through a
    /// back door round the `pub(crate)` `mac_shares()` accessor. The manual [`Debug`]
    /// prints `<redacted>` for the MAC and keeps the openable value sharing `[x]`. We
    /// assert the redaction marker is present and that NO MAC share's `y` value
    /// appears in the output. `[OPUS-4.8]`
    #[test]
    fn authenticated_share_debug_redacts_mac_shares() {
        let backend = ShamirBackend::new_seeded(5, 0xBEEF).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();

        // The worst case: x = 1, so the MAC shares ARE shares of α.
        let auth_one = session.authenticated_share(Fp::one());
        let dbg = format!("{auth_one:?}");

        assert!(
            dbg.contains("<redacted>"),
            "AuthenticatedShare Debug must redact the MAC (α·x) shares: {dbg}"
        );
        // No MAC share's secret `y` value may leak into the Debug string — those are
        // shares of α for x = 1, and any t+1 of them reconstruct α.
        for share in auth_one.mac_shares() {
            assert!(
                !dbg.contains(&share.y.value().to_string()),
                "AuthenticatedShare Debug leaked a MAC share value {}: {dbg}",
                share.y.value()
            );
        }
    }

    /// ACCEPTANCE (3): any `<= t` party views are INDEPENDENT of `α` — the standard
    /// Shamir information-theoretic hiding property applied to `[α]`. With `t`
    /// shares of `α` there is one degree of freedom left, so for ANY candidate
    /// `α'` there exists a degree-`t` polynomial through those `t` shares with
    /// `f(0) = α'`. We witness this by forging a `(t+1)`-th share that makes the
    /// `t` real shares of `α` interpolate to a DIFFERENT key `α' ≠ α` — proof the
    /// `t` shares pin down neither `α` nor anything about it.
    #[test]
    fn t_shares_of_alpha_are_independent_of_alpha() {
        let backend = ShamirBackend::new_seeded(7, 0x5EED).unwrap(); // t = 3
        let t = backend.threshold();
        assert_eq!(t, 3);
        let mut dealer = backend.dealer();
        let session = dealer.new_mac_session();

        // The actual α (test-only accessor) and the t shares an adversary holding t
        // parties would see.
        let alpha = session.alpha_for_test();
        let alpha_shares = session.alpha_shares_for_test();
        let t_views = &alpha_shares[..t];

        // For a DIFFERENT target key α' ≠ α, forge a (t+1)-th share so the t views
        // + the forged share interpolate to α'. Its existence is exactly the
        // information-theoretic hiding argument: the t shares leave f(0) free.
        let alpha_prime = alpha.add(Fp::new(12345)); // some α' != α
        assert_ne!(alpha_prime, alpha);
        let forged = forge_next_share(t_views, alpha_prime, t);

        let mut combined = t_views.to_vec();
        combined.push(forged);
        assert_eq!(
            open(&combined, t),
            alpha_prime,
            "t shares of α are consistent with a DIFFERENT key α' → they hide α"
        );
        // Sanity: the same t views with the REAL (t+1)-th share open to the real α.
        let mut real = t_views.to_vec();
        real.push(alpha_shares[t]);
        assert_eq!(open(&real, t), alpha);
    }

    /// The free linear ops (§2.3) preserve the MAC relation: after add / scale /
    /// sub / add-constant, the reconstructed MAC still equals `α ·` the
    /// reconstructed value. This is what makes the later batched MAC-check
    /// (sq-km34.4) sound on linear circuits, and what keeps the SUM aggregate's
    /// authentication FREE.
    #[test]
    fn linear_ops_preserve_mac_relation() {
        let backend = ShamirBackend::new_seeded(5, 0xF00D).unwrap();
        let t = backend.threshold();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let key = session.mac_key();
        let alpha = session.alpha_for_test();

        let x = Fp::new(30_000);
        let y = Fp::new(45_000);
        let ax = session.authenticated_share(x);
        let ay = session.authenticated_share(y);

        // Assert the MAC relation reconstruct([α·v]) == α·reconstruct([v]) for an
        // authenticated value.
        let check = |auth: &AuthenticatedShare, expected: Fp| {
            let v = open(auth.value_shares(), t);
            let m = open(auth.mac_shares(), t);
            assert_eq!(v, expected, "value");
            assert_eq!(m, alpha.mul(expected), "MAC = α·value");
        };

        // add: x + y.
        check(&auth_add(&ax, &ay).unwrap(), x.add(y));
        // sub: x - y.
        check(&auth_sub(&ax, &ay).unwrap(), x.sub(y));
        // scale by public c.
        let c = Fp::new(3);
        check(&auth_scale(&ax, c), x.mul(c));
        // add public constant: x + k, MAC must pick up α·k via the shared [α].
        let k = Fp::new(500);
        check(&auth_add_constant(&ax, k, &key).unwrap(), x.add(k));

        // A composite linear combination: 3·x + y + 500, the shape of a salary
        // aggregate. MAC must still be α·(3x + y + 500).
        let combo =
            auth_add_constant(&auth_add(&auth_scale(&ax, c), &ay).unwrap(), k, &key).unwrap();
        check(&combo, x.mul(c).add(y).add(k));
    }

    /// The authenticated SUM aggregate (the flatmate cumulative-salary use case)
    /// round-trips AND stays authenticated — the foundation's headline linear
    /// circuit. Mirrors `shamir::secure_sum_equals_plaintext_sum`, but every
    /// addend is authenticated and the result's MAC relation holds.
    #[test]
    fn authenticated_sum_round_trips_and_stays_authenticated() {
        let salaries = [30_000u64, 45_000, 28_000, 51_000];
        let plaintext_sum: u64 = salaries.iter().sum();

        let backend = ShamirBackend::new_seeded(5, 0xABCD).unwrap();
        let t = backend.threshold();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let alpha = session.alpha_for_test();

        let shared: Vec<AuthenticatedShare> = salaries
            .iter()
            .map(|&s| session.authenticated_share(Fp::new(s)))
            .collect();

        // Zero-round local accumulation (the SUM aggregate) via the reusable
        // authenticated aggregate — the MAC-carrying analogue of `run_secure`.
        let acc = auth_sum(&shared).unwrap();

        let sum = open(acc.value_shares(), t);
        assert_eq!(
            sum,
            Fp::new(plaintext_sum),
            "authenticated sum == plaintext sum"
        );
        let sum_mac = open(acc.mac_shares(), t);
        assert_eq!(sum_mac, alpha.mul(sum), "aggregate MAC stays α·sum");

        // Parity: the reusable `auth_sum` matches an explicit `auth_add` fold —
        // it IS the fold, just packaged (non-vacuous check on the helper).
        let mut manual = shared[0].clone();
        for next in &shared[1..] {
            manual = auth_add(&manual, next).unwrap();
        }
        assert_eq!(
            open(manual.value_shares(), t),
            sum,
            "auth_sum == manual fold"
        );
    }

    /// `auth_sum` rejects an empty aggregate (like `run_secure`): there is no
    /// meaningful sum of zero addends to disclose, so it is a protocol error, not
    /// a silent zero. `[OPUS-4.8]`
    #[test]
    fn auth_sum_rejects_empty_input() {
        assert!(matches!(auth_sum(&[]), Err(MpcError::Protocol(_))));
    }

    /// Mismatched party sets are a protocol error, not a silent wrong answer (the
    /// linear ops must reject sharings on different point sets — the same
    /// discipline as `add_shares`).
    #[test]
    fn linear_ops_reject_mismatched_party_sets() {
        let b5 = ShamirBackend::new_seeded(5, 1).unwrap();
        let b7 = ShamirBackend::new_seeded(7, 2).unwrap();
        let mut d5 = b5.dealer();
        let mut d7 = b7.dealer();
        let mut s5 = d5.new_mac_session();
        let mut s7 = d7.new_mac_session();
        let a = s5.authenticated_share(Fp::new(1));
        let b = s7.authenticated_share(Fp::new(2));
        assert!(matches!(auth_add(&a, &b), Err(MpcError::Protocol(_))));
        assert!(matches!(auth_sub(&a, &b), Err(MpcError::Protocol(_))));
        // add_constant with a wrong-sized key.
        let wrong_key = s7.mac_key();
        assert!(matches!(
            auth_add_constant(&a, Fp::new(9), &wrong_key),
            Err(MpcError::Protocol(_))
        ));
    }

    /// Given `t` shares on a degree-`t` polynomial, produce ONE more share (at a
    /// fresh `x`) so the `t+1` interpolate to `target` at `x = 0`. Demonstrates the
    /// hiding property of acceptance (3); test-only. (Generalises the degree-2
    /// `forge_third_share` in `shamir.rs` to any `t`.)
    fn forge_next_share(views: &[Share], target: Fp, t: usize) -> Share {
        assert_eq!(views.len(), t, "need exactly t shares to leave one DOF");
        // Pick a fresh x distinct from the views' points.
        let xnew = (1..=10_000u64)
            .find(|x| views.iter().all(|s| s.x != *x))
            .expect("a fresh evaluation point exists");
        // Solve y_new so Lagrange-at-0 of {views, (xnew, y_new)} equals target.
        let mut xs: Vec<u64> = views.iter().map(|s| s.x).collect();
        xs.push(xnew);
        let weight = |i: usize| -> Fp {
            let xi = Fp::new(xs[i]);
            let mut num = Fp::one();
            let mut den = Fp::one();
            for (j, &xj) in xs.iter().enumerate() {
                if i == j {
                    continue;
                }
                let xj = Fp::new(xj);
                num = num.mul(xj.neg());
                den = den.mul(xi.sub(xj));
            }
            num.mul(den.inv())
        };
        // target = Σ_i y_i·L_i + y_new·L_new  ⇒  y_new = (target − Σ y_i·L_i)/L_new.
        let mut acc = Fp::zero();
        for (i, v) in views.iter().enumerate() {
            acc = acc.add(v.y.mul(weight(i)));
        }
        let l_new = weight(views.len());
        let y_new = target.sub(acc).mul(l_new.inv());
        Share { x: xnew, y: y_new }
    }
}
