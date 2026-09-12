// [OPUS-4.8] Honest-majority Shamir secret-sharing backend (M3, first concrete MpcBackend).
//! Shamir `t`-of-`n` secret sharing — the first concrete [`MpcBackend`] (M3).
//!
//! Architecture refs: §3.1 (secret-sharing families), §4.2 (trust model:
//! **honest-majority** is the stated first target), §4.3 step 4 (the secure
//! computation over secret-shared per-source values). Skill `mpc-protocols`:
//! "honest-majority … semi-honest among *cooperating* holders … is the viable
//! first target".
//!
//! ## Scheme chosen: Shamir `t`-of-`n` over `F_p` — and WHY (vs replicated 3PC)
//!
//! The driving use case is the **four flatmates** (`research/…architecture.md`
//! §2; Wright CEUR Vol-4085): N *cooperating* holders, each with a private
//! payslip value, jointly proving a cumulative aggregate without revealing
//! individual values. Two honest-majority candidates from the skill:
//!
//! - **Replicated 3PC secret sharing** is the *performance* sweet spot
//!   (1 element / mult-gate) **but is fixed at n = 3**: each of three parties
//!   holds two of three additive shares, assuming ≥2-of-3 non-colluding compute
//!   parties. It does **not** generalise to "four flatmates", or five, or any N.
//! - **Shamir `t`-of-`n`** uses a degree-`t` random polynomial whose constant
//!   term is the secret; honest-majority means `t < n/2`. It **generalises to
//!   any N** (the flatmate count is a deployment parameter, not a constant baked
//!   into the protocol), additions/linear combinations are **local / non-
//!   interactive** (free — no rounds), and reconstruction is exact Lagrange
//!   interpolation. The cumulative-salary aggregate is a *pure linear function*
//!   of the per-holder inputs, so under Shamir it costs **zero communication
//!   rounds** for the arithmetic itself — the dominant honest-majority win for
//!   THIS computation.
//!
//! **Decision (resolving Q2 for v1): Shamir `t`-of-`n`, honest-majority,
//! semi-honest.** It matches the "any number of cooperating flatmates" shape
//! that replicated-3PC cannot, and the aggregate we secure first is linear, so
//! Shamir's free-addition property is exactly the right cost profile. Replicated
//! 3PC would be the choice only if N were pinned at 3 and multiplication-heavy.
//!
//! ## Security model — stated explicitly (do not paper over)
//!
//! - **Honest-majority, semi-honest (honest-but-curious).** Privacy holds as
//!   long as **fewer than `t+1`** parties pool their shares. With threshold `t`,
//!   any set of `≤ t` shares is information-theoretically independent of the
//!   secret (Shamir 1979): `t` points leave one degree of freedom, so every
//!   candidate secret is equally consistent. The honest-majority instantiation
//!   sets `t = ⌊(n-1)/2⌋`, so a strict majority of honest parties cannot have
//!   their shares completed into the secret by the minority.
//! - **What each party learns:** ONLY its own share(s) of each input and of the
//!   result, plus the disclosed reconstructed output. It learns nothing about
//!   another holder's private input (confidentiality, guarantee (A)).
//! - **NOT in scope for v1:** active/malicious deviation (guarantee (D)). A
//!   semi-honest party follows the protocol; a malicious one could feed
//!   inconsistent shares. Malicious honest-majority (≈2× cost, e.g. with
//!   information-theoretic MACs / verifiable secret sharing) is a *future*
//!   hardening behind the SAME [`MpcBackend`] trait — see [`crate::backend`] doc
//!   and PLAN M3/M4. We do NOT claim malicious security here.
//!
//! ## Randomness (sq-1vt: CSPRNG, resolved) [OPUS-4.8]
//!
//! The masking polynomial coefficients and the equality-test mask need
//! randomness, and that randomness is **security-critical**: if it is
//! predictable, shares and masks become predictable and confidentiality
//! collapses (sq-1vt). The randomness therefore comes through the [`crate::rng`]
//! seam:
//!
//! - The **production / default** path ([`ShamirBackend::new`] →
//!   [`ShamirBackend::dealer`]) mints a fresh [`crate::rng::SecureRng`] — a
//!   ChaCha20 CSPRNG seeded from OS entropy — for each dealing session. Field
//!   elements are drawn uniformly via rejection sampling (no modulo bias).
//! - A **deterministic, seedable** path (`ShamirBackend::new_seeded`, gated
//!   behind `#[cfg(any(test, feature = "insecure-test-rng"))]` — so not an
//!   intra-doc link in default builds) drives the
//!   in-process multi-party SIMULATION and its differential/stress tests
//!   reproducibly. It is feature-gated out of normal builds: the real masking
//!   path cannot reach the deterministic RNG. No security is claimed for it.
//!
//! Crucially the live RNG state lives on a short-lived [`ShamirDealer`], not on
//! the `Clone`-able [`ShamirBackend`] config — so cloning a backend never
//! duplicates (and thus reuses) a CSPRNG keystream. Each `dealer()` call gets
//! independent randomness.

use crate::backend::{
    BackendInfo, MaliciousSecurity, MpcBackend, OperatorClass, SecurityDescriptor,
};
use crate::field::Fp;
use crate::holder::Holder;
use crate::partial::{HolderId, MpcError, PartialResult};
use crate::rng::{MpcRng, SecureRng};

/// A single Shamir share: the polynomial evaluated at a party's nonzero point.
/// `x` is the party index (1-based; the secret sits at `x = 0`), `y = f(x)`.
///
/// [OPUS-4.8] sq-u8a8: derives [`zeroize::Zeroize`] so secret share material
/// (`y = f(x)`, and for the MAC sharing `[α]`/`[α·x]`, every share `y`) can be
/// scrubbed by the secret containers that own it. `Share` is `Copy`, so it cannot
/// itself implement `Drop`; the zeroize-on-drop lives on the owning secret type
/// (e.g. `MacKey`). Hygiene only — `Zeroize` adds a scrub method, it changes no
/// sharing/reconstruction arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, zeroize::Zeroize)]
pub struct Share {
    /// The evaluation point (party index, always `>= 1`). `x = 0` is reserved
    /// for the secret and is never handed to a party.
    pub x: u64,
    /// The share value `f(x)` in `F_p`.
    pub y: Fp,
}

/// How a [`ShamirBackend`] seeds the masking RNG for each dealing session.
///
/// This is a *descriptor*, not live RNG state — it is cheaply `Clone`/`Copy` and
/// carries no keystream, so cloning a backend can never duplicate (and reuse) a
/// CSPRNG. The live randomness is minted per session by [`ShamirBackend::dealer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RngSource {
    /// **Production / default.** Each dealing session gets a fresh
    /// [`SecureRng`] (ChaCha20 CSPRNG) seeded from OS entropy. This is the only
    /// variant a default-feature build can construct (sq-1vt).
    Os,
    /// **Test/benchmark only.** Deterministic, seedable masking RNG for
    /// reproducible simulation. Gated out of normal builds.
    #[cfg(any(test, feature = "insecure-test-rng"))]
    InsecureSeed(u64),
}

/// Honest-majority Shamir `t`-of-`n` secret-sharing backend (the immutable
/// *configuration*).
///
/// `n` = number of compute parties (= holders in the flatmate model); the
/// privacy threshold is `t = ⌊(n-1)/2⌋` (honest majority). The backend holds NO
/// live RNG state — only `n`, `t`, and the `RngSource` descriptor (private) — so it is
/// freely `Clone`-able without ever cloning a CSPRNG keystream. The masking
/// randomness lives on the short-lived [`ShamirDealer`] minted by [`Self::dealer`].
///
/// The backend is the *coordinator-free* description of the scheme; the
/// in-process multi-party simulation that runs the parties is driven by
/// [`MpcBackend::run_secure`] + the helpers below, which is what the differential
/// tests exercise.
#[derive(Debug, Clone)]
pub struct ShamirBackend {
    n: usize,
    t: usize,
    rng_source: RngSource,
}

impl ShamirBackend {
    /// Build a **production** honest-majority backend for `n` parties (`n >= 2`).
    /// The privacy threshold is the honest-majority maximum `t = ⌊(n-1)/2⌋`: any
    /// `<= t` colluding parties learn nothing; reconstruction needs `>= t+1`.
    ///
    /// Masking randomness comes from a fresh OS-seeded ChaCha20 CSPRNG per
    /// dealing session — the only RNG this constructor wires in (sq-1vt). There
    /// is no seed parameter: a real masking RNG must not be predictable. For a
    /// reproducible test simulation use `Self::new_seeded` (test-gated, so not an
    /// intra-doc link in default builds).
    pub fn new(n: usize) -> Result<Self, MpcError> {
        Self::with_source(n, RngSource::Os)
    }

    /// Build a backend whose masking RNG is a **deterministic, seedable**
    /// SplitMix64 — **for reproducible tests / benchmarks ONLY**. Gated behind
    /// `#[cfg(any(test, feature = "insecure-test-rng"))]` so the real protocol
    /// cannot construct it. The masks it produces are predictable; never use it
    /// for a real deployment (that is the sq-1vt weakness).
    #[cfg(any(test, feature = "insecure-test-rng"))]
    pub fn new_seeded(n: usize, seed: u64) -> Result<Self, MpcError> {
        Self::with_source(n, RngSource::InsecureSeed(seed))
    }

    fn with_source(n: usize, rng_source: RngSource) -> Result<Self, MpcError> {
        if n < 2 {
            return Err(MpcError::Protocol(
                "Shamir honest-majority backend needs n >= 2 parties".into(),
            ));
        }
        let t = (n - 1) / 2;
        Ok(ShamirBackend { n, t, rng_source })
    }

    /// **Test-only escape hatch** that builds a backend with an EXPLICIT threshold
    /// `t`, bypassing the honest-majority `t = ⌊(n−1)/2⌋` rule, so deficient
    /// configurations (`n < 2t+1`, which the public constructors can never build)
    /// can be handed to the fail-closed precondition checks (e.g. the secure
    /// comparison's `n >= 2t+1` guard, sq-rrz4). It uses the seedable insecure RNG.
    /// NEVER a production path — it deliberately violates the security invariant the
    /// real constructors enforce. `[OPUS-4.8]`
    #[cfg(test)]
    pub(crate) fn with_unchecked_threshold(n: usize, t: usize) -> Self {
        ShamirBackend {
            n,
            t,
            rng_source: RngSource::InsecureSeed(0xDEAD_BEEF),
        }
    }

    /// Number of compute parties.
    pub fn parties(&self) -> usize {
        self.n
    }

    /// The privacy threshold `t`: any subset of `<= t` parties' shares is
    /// independent of the secret.
    pub fn threshold(&self) -> usize {
        self.t
    }

    /// The active-security guarantee this `(n, t)` configuration delivers at the
    /// degree-`t` reconstruction (WI-1, parent bead sq-uu0u), derived purely from
    /// the RS redundancy `n − (t + 1)`. See [`MaliciousSecurity`] and
    /// [`crate::robust::reconstruct_robust`] for the bound; surfaced via
    /// [`MpcBackend::info`]. `[OPUS-4.8]`
    pub fn malicious_security(&self) -> MaliciousSecurity {
        // No redundancy at exactly `t+1` shares: tampering is information-
        // theoretically undetectable, so claim nothing.
        if self.n <= self.t + 1 {
            return MaliciousSecurity::SemiHonestOnly;
        }
        // RS / Berlekamp–Welch correction budget at degree `t`.
        let max_cheaters = (self.n - self.t - 1) / 2;
        if max_cheaters == 0 {
            // One redundant share lets us cross-check and detect, but not correct.
            MaliciousSecurity::HonestMajorityAbort
        } else {
            MaliciousSecurity::HonestMajorityRobust { max_cheaters }
        }
    }

    /// [OPUS-4.8] sq-mq8q — the RS/Berlekamp–Welch correction budget
    /// `e = ⌊(n − degree − 1)/2⌋` at a given reconstruction `degree`, or `None`
    /// when there is NO redundancy (`n <= degree + 1`, so tampering is
    /// information-theoretically undetectable). Shared by the per-operator
    /// reporting below: the linear aggregate opens at `degree = t`, the equality
    /// join at `degree = 2t`.
    fn rs_correction_budget(&self, degree: usize) -> Option<usize> {
        if self.n <= degree + 1 {
            None
        } else {
            Some((self.n - degree - 1) / 2)
        }
    }

    /// [OPUS-4.8] sq-mq8q — the three-axis [`SecurityDescriptor`] for one
    /// [`OperatorClass`] at THIS `(n, t)`. Guarantees genuinely differ per
    /// operator (the degree-`t` aggregate carries RS redundancy for every valid
    /// honest-majority `(n, t)` and is robust; the degree-`2t` equality open has
    /// ZERO redundancy at `n = 2t+1` and is semi-honest-only there), so one
    /// backend-level bit would lie. Surfaced via [`MpcBackend::operator_security`].
    pub fn operator_descriptor(&self, operator: OperatorClass) -> SecurityDescriptor {
        match operator {
            // Linear aggregate: degree-`t` open. Has redundancy for every valid
            // honest-majority `t = ⌊(n−1)/2⌋`, so in practice it never hits the
            // no-redundancy branch — but we still match `None` explicitly rather
            // than collapsing it into `e = 0`. `unwrap_or(0)` would map the
            // no-redundancy case (tampering information-theoretically undetectable)
            // onto the SAME `e = 0` detect-and-abort descriptor as the
            // one-redundant-share case, over-claiming detection where there is
            // none. Mirror the `EqualityJoin` arm: `None` → `semi_honest_only`.
            // `[OPUS-4.8]` (Copilot review #87).
            OperatorClass::LinearAggregate => match self.rs_correction_budget(self.t) {
                None => SecurityDescriptor::semi_honest_only(self.n, self.t),
                Some(e) => SecurityDescriptor::shamir_degree_recon(self.n, self.t, e),
            },
            // Equality / hidden-value join: degree-`2t` open. No redundancy at
            // `n = 2t+1` (odd-`n` honest majority) → semi-honest-only; otherwise
            // the RS budget at degree `2t` decides detect-and-abort vs robust.
            OperatorClass::EqualityJoin => match self.rs_correction_budget(2 * self.t) {
                None => SecurityDescriptor::semi_honest_only(self.n, self.t),
                Some(e) => SecurityDescriptor::shamir_degree_recon(self.n, self.t, e),
            },
            // Comparison (`<`,`≤`,`>`, threshold) IS realized in-crypto now
            // (sq-rrz4, [`crate::compare`]): bit-decomposition MSB-first comparison
            // that chains multiplications through `degree_reduce` and opens ONLY the
            // verdict bit. Its security is honest-majority **semi-honest-only**, NOT
            // malicious: each step's degree reduction re-shares with no in-protocol
            // check that a deviating party re-shared honestly (the same boundary as
            // the degree-`2t` equality open / `degree_reduce` itself). The final
            // verdict opens at degree `t` (so the WI-1 RS checker still detects a
            // tampered FINAL open where `n > t+1`), but a cheat INSIDE a re-sharing
            // round is undetected — so we report the honest semi-honest-only
            // baseline for the WHOLE chain rather than over-claim the degree-`t`
            // open's detection. Malicious hardening (IT-MACs / verifiable resharing)
            // is the same deferred seam as the rest of the backend
            // (research/mpc-security-models-and-benchmarks.md §3, §8 steps 5–6).
            OperatorClass::Comparison => SecurityDescriptor::semi_honest_only(self.n, self.t),
        }
    }

    /// Mint a fresh [`ShamirDealer`] with **independent** masking randomness for
    /// one dealing session. In production this seeds a brand-new OS-seeded
    /// ChaCha20 CSPRNG — so two dealers from the same (or a cloned) backend draw
    /// independent, unpredictable masks, never a reused keystream.
    pub fn dealer(&self) -> ShamirDealer {
        let rng: Box<dyn MpcRng> = match self.rng_source {
            RngSource::Os => Box::new(SecureRng::from_os()),
            #[cfg(any(test, feature = "insecure-test-rng"))]
            RngSource::InsecureSeed(seed) => Box::new(crate::rng::InsecureTestRng::new(seed)),
        };
        ShamirDealer {
            n: self.n,
            t: self.t,
            rng,
            mults: 0,
            opens: 0,
        }
    }

    /// Reconstruct the secret `f(0)` from `>= t+1` shares. Fewer than `t+1`
    /// shares is a protocol error (the whole point of the threshold). Shares must
    /// have distinct `x`. RNG-free.
    ///
    /// **Consistency-checked / robust (sq-m34i, WI-1).** When redundancy is
    /// present (`n > t+1`) this routes through [`crate::robust::reconstruct_robust`]:
    /// it verifies all points lie on one degree-`t` polynomial, CORRECTS up to
    /// `e = ⌊(n−t−1)/2⌋` tampered shares (returning the true secret), and aborts
    /// with [`MpcError::Tampered`] on any inconsistency it cannot repair —
    /// closing the malicious-security gap (D) at the Shamir layer (parent bead
    /// sq-uu0u). On clean input it returns exactly the same value as plain
    /// Lagrange (no behaviour change). At exactly `t+1` shares (no redundancy)
    /// tampering is information-theoretically undetectable, so it falls back to
    /// plain Lagrange and makes NO detection claim.
    pub fn reconstruct(&self, shares: &[Share]) -> Result<Fp, MpcError> {
        crate::robust::reconstruct_robust(shares, self.t)
    }
}

/// A short-lived dealer holding **live masking randomness** for one sharing
/// session. It owns the CSPRNG (production) / deterministic PRNG (test) and is
/// the only thing that draws masking field elements. Created by
/// [`ShamirBackend::dealer`]; not `Clone` (its RNG state must not be duplicated).
pub struct ShamirDealer {
    n: usize,
    t: usize,
    rng: Box<dyn MpcRng>,
    /// Monotone count of successful [`Self::degree_reduce`] rounds — see
    /// [`Self::mult_count`].
    mults: usize,
    /// Monotone count of interactive opens noted by dealer-driven sub-protocols —
    /// see [`Self::open_count`] / [`Self::note_open`].
    opens: usize,
}

impl std::fmt::Debug for ShamirDealer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShamirDealer")
            .field("n", &self.n)
            .field("t", &self.t)
            .field("rng", &"<live masking RNG>")
            .finish()
    }
}

impl ShamirDealer {
    /// Number of compute parties (mirrors the backend's).
    pub fn parties(&self) -> usize {
        self.n
    }

    /// The privacy threshold `t` (mirrors the backend's).
    pub fn threshold(&self) -> usize {
        self.t
    }

    /// Number of secure multiplications this dealer has performed so far — a
    /// monotone count of successful [`Self::degree_reduce`] rounds, the BGW
    /// re-sharing round every chained secret-shared product pays. [OPUS-5]
    ///
    /// Cost reporters take DELTAS of this counter (e.g. `SortMergeCost::scan_mults`
    /// in `sort_merge_join`) so a modelled cost counts what the protocol actually
    /// executed instead of a hand-kept tally that drifts when a primitive's
    /// internal circuit changes. Masked-product OPENS — paths that reconstruct a
    /// degree-`2t` product directly (the square-protocol bit dealing, the
    /// zero-tests) rather than re-sharing it — are opens, not degree reductions,
    /// and are deliberately not counted here: they have their own twin counter,
    /// [`Self::open_count`].
    pub(crate) fn mult_count(&self) -> usize {
        self.mults
    }

    /// Number of interactive OPENS the dealer-driven comparison sub-protocols have
    /// performed so far — the twin of [`Self::mult_count`] for the reconstructions
    /// that [`Self::mult_count`] deliberately excludes: the square-protocol
    /// `c = a²` open behind every jointly-random mask bit, the masked
    /// bit-decomposition open (`c = (x + r) mod p` on the Rabbit path), and the
    /// zero-test product open `m = v·r` of the in-protocol range proofs. [OPUS-5]
    ///
    /// A monotone count incremented via [`Self::note_open`] at each such open site
    /// (the reconstruct routines themselves are dealer-free, so the sites note the
    /// open explicitly). Cost reporters take DELTAS (e.g.
    /// `SortMergeCost::scan_opens` in `sort_merge_join`) so the modelled
    /// communication includes the opens as well as the multiplications. Verdict /
    /// padded-reveal opens performed directly by an operator through the backend
    /// are NOT noted here — operators count those themselves.
    pub(crate) fn open_count(&self) -> usize {
        self.opens
    }

    /// Record one interactive open performed by a dealer-driven sub-protocol —
    /// see [`Self::open_count`].
    pub(crate) fn note_open(&mut self) {
        self.opens += 1;
    }

    /// Draw one uniform field element from the masking RNG (advances state).
    /// Used by the equality-test mask. Rejection-sampled for exact uniformity
    /// (see [`crate::rng`]).
    pub fn draw_fp(&mut self) -> Fp {
        self.rng.next_fp()
    }

    /// Draw a uniform **nonzero** field element. The equality mask must be
    /// nonzero (a zero mask would open `m = 0` even for unequal keys).
    pub fn draw_nonzero_fp(&mut self) -> Fp {
        self.rng.next_nonzero_fp()
    }

    /// [OPUS-4.8] sq-g7t5 — draw one **exactly-unbiased** uniform random bit
    /// (`0` or `1`) directly from the masking RNG. Takes the LSB of a raw
    /// `next_u64()` draw, which is uniform because the CSPRNG output is uniform
    /// over all `2^64` words (so its low bit is `Pr[0] = Pr[1] = 1/2`). This is
    /// the source the bit-decomposition mask needs; it deliberately does NOT go
    /// through [`draw_fp`](Self::draw_fp), whose `[0, p)` output (`p = 2^61−1`
    /// odd) has a `~2^{-61}`-biased LSB — negligible, but the bit-mask soundness
    /// argument is cleaner when the source is *exactly* uniform.
    pub fn draw_bit(&mut self) -> u64 {
        self.rng.next_u64() & 1
    }

    /// Secret-share one field element into `n` shares on a fresh degree-`t`
    /// polynomial `f` with `f(0) = secret`. Free coefficients are uniform random
    /// from the masking RNG (a CSPRNG in production — sq-1vt).
    pub fn share(&mut self, secret: Fp) -> Vec<Share> {
        // f(z) = secret + c_1 z + ... + c_t z^t.
        let mut coeffs = Vec::with_capacity(self.t + 1);
        coeffs.push(secret);
        for _ in 0..self.t {
            coeffs.push(self.rng.next_fp());
        }
        (1..=self.n as u64)
            .map(|x| Share {
                x,
                y: eval_poly(&coeffs, Fp::new(x)),
            })
            .collect()
    }

    /// [OPUS-4.8] sq-dwb5 — **batched / vector secret sharing.** Share a whole
    /// `Vec<Fp>` element-wise: each `values[i]` gets its OWN fresh, INDEPENDENT
    /// degree-`t` masking polynomial, yielding one sharing (`ShareVec`) per
    /// element. The returned `Vec<ShareVec>` is *positionally* row-bound: output
    /// index `i` is the sharing of `values[i]` — the documented row-binding
    /// contract (see [`crate::batched::BatchedShares`]).
    ///
    /// **Why one fresh polynomial per element (NOT one polynomial carrying all
    /// values).** A single masking polynomial whose *coefficients* are the secrets
    /// would couple the elements: opening one element's evaluation would constrain
    /// the others, and `≤ t` parties' views would no longer be independent of the
    /// vector. Independent per-element polynomials preserve the exact single-scalar
    /// privacy guarantee for EVERY element simultaneously — any `≤ t` parties'
    /// shares of the whole batch are jointly independent of all the values (each
    /// element's masking coefficients are fresh, so the joint view is a product of
    /// independent uniform sharings). This is the standard batched-Shamir layout;
    /// it trades share count (`n·k`) for the unchanged per-element security.
    ///
    /// Linear ops ([`add_shares`] / [`scale`] / …) compose element-wise across two
    /// equally-sized batches, so a per-row secure aggregate is the element-wise
    /// fold of the per-holder batches — still zero communication rounds.
    pub fn share_batch(&mut self, values: &[Fp]) -> Vec<ShareVec> {
        values.iter().map(|&v| self.share(v)).collect()
    }

    /// **BGW/GRR degree-reduction round (sq-dvuc).** Reduce a degree-`2t` product
    /// sharing back to a fresh degree-`t` sharing of the SAME secret, so that
    /// secret-shared multiplications can **chain** (`a·b·c`, secure comparison /
    /// threshold, conjunctive hidden-pattern joins) instead of being limited to a
    /// single non-reducing product ([`mul_shares_raw`]). `[OPUS-4.8]`
    ///
    /// This is the standard **BGW** reduction with a **public recombination
    /// vector** (Gennaro–Rabin–Rabin'98 simplification of BGW'88), run over the
    /// in-process party simulation:
    ///
    /// 1. The component-wise product `h_i = f(x_i)·g(x_i)` of two degree-`t`
    ///    sharings lies on a degree-`2t` polynomial `H` with `H(0) = a·b` (the
    ///    secret product). Reconstructing `H(0)` from its first `2t+1` evaluation
    ///    points is a FIXED public linear map — the Lagrange-at-0 weights
    ///    `λ_1..λ_{2t+1}`: `H(0) = Σ_i λ_i · h_i`.
    /// 2. Each simulated party `i` (`i = 1..2t+1`) **re-shares** its own degree-`2t`
    ///    share value `h_i` under a FRESH, INDEPENDENT degree-`t` polynomial
    ///    (`[h_i]_t`), using this dealer's masking RNG — a fresh OS-seeded ChaCha20
    ///    CSPRNG in production (sq-1vt). This is the one (simulated) communication
    ///    round.
    /// 3. Every party then locally applies the SAME public recombination vector to
    ///    the sub-shares it received: `[a·b]_t = Σ_i λ_i · [h_i]_t`. Because the
    ///    `λ_i` are public scalars and the `[h_i]_t` are degree-`t` sharings, the
    ///    result is a degree-`t` sharing whose secret is `Σ_i λ_i·H(x_i) = H(0) =
    ///    a·b` — a fresh degree-`t` sharing of the original product.
    ///
    /// **Precondition (fail-closed):** `degree_reduce` requires `n >= 2t+1` (so a
    /// degree-`2t` polynomial is determined by the `n` party points and the `2t+1`
    /// recombination points exist). The honest-majority constructor already fixes
    /// `t = ⌊(n−1)/2⌋`, so every backend `Self` builds satisfies this; the check is
    /// here to fail with a descriptive [`MpcError::Protocol`] rather than panic if a
    /// short / mis-built share vector is ever passed. The input must be the full
    /// `n`-party degree-`2t` sharing on the canonical points `x = 1..n`, **in
    /// order** — this is now ENFORCED (sq-dvuc / Copilot #119): the reduction is a
    /// FIXED public linear map that pairs the recombination weights and the fresh
    /// sub-sharings with the input shares *by position*, so a permuted or
    /// non-canonical input is rejected with [`MpcError::Protocol`] (fail-closed in
    /// BOTH debug and release builds) rather than silently mis-reduced. `[OPUS-4.8]`
    ///
    /// **Security model — UNCHANGED honest-majority / semi-honest (do not
    /// over-claim).** The reduction is the BGW reduction step under the SAME trust
    /// assumptions as the rest of this backend (module docs §"Security model"): it
    /// is correct and confidentiality-preserving when every party follows the
    /// protocol and at most `t` collude. It is **NOT** maliciously secure: a
    /// deviating party can feed an inconsistent re-sharing, and — exactly as for the
    /// degree-`2t` equality open at `n = 2t+1` — there is no in-protocol check here
    /// that detects it. Each fresh re-sharing draws its own random masking
    /// coefficients, so any `≤ t` parties' view of the sub-shares is independent of
    /// the reduced secret (the standard BGW privacy argument).
    /// See `research/mpc-security-models-and-benchmarks.md` §3 (the
    /// "general Shamir multiplication needs degree reduction" gap) and §6 step 6.
    ///
    /// **This primitive is not hardened — the AUTHENTICATED caller is (sq-km34.3).**
    /// The wrong-re-sharing deviation above ("Hole 2") is not detectable *here*, and
    /// this method makes no attempt to detect it: a party that re-shares `h_i + δ`
    /// emits a perfectly consistent degree-`t` codeword, so there is nothing for
    /// Reed–Solomon to flag even at over-provisioned `n`. It is COVERED one level up,
    /// on the authenticated path: [`MacSession::auth_mul`] calls this reduce TWICE
    /// over independent inputs (once for `[z]`, once for `[α·z] = reduce([α·x]·[y])`),
    /// so a `δ` injected into either reduce breaks the `m_z = α·z` relation and
    /// [`MacSession::mac_check`] aborts. A caller that uses this reduce directly,
    /// WITHOUT the MAC session, gets the semi-honest guarantee only.
    ///
    /// Returns a fresh degree-`t` sharing on the canonical points `x = 1..n`.
    pub fn degree_reduce(&mut self, shares_2t: &[Share]) -> Result<Vec<Share>, MpcError> {
        // Fail-closed precondition: we need n >= 2t+1 distinct party points so the
        // degree-2t polynomial is over-determined and 2t+1 recombination points
        // exist. (Equivalently: the supplied sharing must cover the full party set.)
        if shares_2t.len() < 2 * self.t + 1 {
            return Err(MpcError::Protocol(format!(
                "degree_reduce: need a degree-2t product sharing on n >= 2t+1 = {} parties, \
                 got {} shares (honest-majority needs n >= 2t+1 to reduce a degree-2t product)",
                2 * self.t + 1,
                shares_2t.len()
            )));
        }
        if shares_2t.len() != self.n {
            return Err(MpcError::Protocol(format!(
                "degree_reduce: expected the full {}-party sharing, got {} shares",
                self.n,
                shares_2t.len()
            )));
        }
        // `[OPUS-4.8]` Strict canonical-point check (Copilot #119, sq-dvuc). The
        // whole reduction is a FIXED public linear map that ASSUMES the input is
        // the full n-party degree-2t sharing on the canonical points `x = 1..=n`
        // *in order*: the recombination λ-weights below are derived from the first
        // 2t+1 x-coords and then paired BY POSITION with the fresh sub-sharings
        // (which `share()` always emits on `x = 1..=n` in order). A permuted or
        // non-canonical input would silently produce a WRONG reduced sharing, so we
        // fail closed here rather than mis-reduce. (We choose the strict-canonical
        // contract — matching the docs — over deriving weights from arbitrary x:
        // it is simplest and the only ordering any in-process backend ever builds.)
        for (i, s) in shares_2t.iter().enumerate() {
            let expected_x = i as u64 + 1;
            if s.x != expected_x {
                return Err(MpcError::Protocol(format!(
                    "degree_reduce: input must be the canonical n-party sharing on x = 1..={}, \
                     in order; share[{i}] is on x = {} (expected {expected_x})",
                    self.n, s.x
                )));
            }
        }

        // Public recombination vector: the Lagrange-at-0 weights for the first
        // 2t+1 evaluation points. H(0) = Σ_{i=1}^{2t+1} λ_i · H(x_i), a FIXED
        // public linear map (depends only on the party points, never on secrets).
        // After the canonical-point check above, these are exactly `x = 1..=2t+1`.
        let recomb_points: Vec<u64> = shares_2t[..2 * self.t + 1].iter().map(|s| s.x).collect();
        let lambdas = lagrange_zero_weights(&recomb_points);

        // Step 2: each of the 2t+1 recombination parties re-shares its degree-2t
        // share h_i under a FRESH, independent degree-t polynomial. Each re-sharing
        // is an n-vector of sub-shares on the canonical points x = 1..n.
        let mut resharings: Vec<Vec<Share>> = Vec::with_capacity(2 * self.t + 1);
        for s in &shares_2t[..2 * self.t + 1] {
            resharings.push(self.share(s.y));
        }

        // Step 3: every party j locally forms Σ_i λ_i · (sub-share i held by j).
        // The λ_i are public scalars, the sub-sharings are degree-t, so the result
        // is a degree-t sharing of Σ_i λ_i·h_i = H(0) = a·b.
        let mut reduced: Vec<Share> = shares_2t
            .iter()
            .map(|s| Share {
                x: s.x,
                y: Fp::zero(),
            })
            .collect();
        for (i, sub) in resharings.iter().enumerate() {
            let lambda = lambdas[i];
            for (out, sub_share) in reduced.iter_mut().zip(sub.iter()) {
                // `[OPUS-4.8]` Real fail-closed x-alignment check (Copilot #119,
                // sq-dvuc). The accumulation pairs each output party with the
                // sub-share at the SAME position, which is only correct when both
                // run over the canonical points `x = 1..=n` in order. This was a
                // `debug_assert_eq!`, which is COMPILED OUT in release — so a
                // position/point mismatch could silently mis-accumulate in release
                // builds. A real `if` guarantees it in both profiles. (Given the
                // canonical-point check above and that `share()` always emits
                // `x = 1..=n`, this is unreachable in practice; it is a defence-in-
                // depth invariant, not a redundant assertion.)
                if out.x != sub_share.x {
                    return Err(MpcError::Protocol(format!(
                        "degree_reduce: resharing point misalignment — output party x = {} \
                         paired with sub-share x = {} (resharings must use the canonical \
                         points x = 1..={} in order)",
                        out.x, sub_share.x, self.n
                    )));
                }
                out.y = out.y.add(lambda.mul(sub_share.y));
            }
        }
        self.mults += 1; // one completed secure multiplication (see mult_count)
        Ok(reduced)
    }

    /// [OPUS-4.8] sq-81gd — **test-only INSTRUMENTED degree-reduce that injects a
    /// deviation `δ` INSIDE the BGW re-sharing step.** This reproduces
    /// [`Self::degree_reduce`] EXACTLY, except the `reshare_party`-th recombination
    /// party re-shares `h_i + δ` instead of its true sub-share `h_i` — the genuine
    /// "Hole 2" malicious deviation that a plain post-hoc share tamper cannot model.
    ///
    /// ## Why this is a DIFFERENT, harder attack than tampering the output sharing
    ///
    /// The honest reduce re-shares each `h_i` under a FRESH degree-`t` polynomial and
    /// recombines `Σ_i λ_i·[h_i]`. A deviating party who instead re-shares `h_i + δ`
    /// emits a **perfectly internally-consistent** degree-`t` codeword (`share()`
    /// always produces a valid sharing) — so the recombined output `[z]` is a *valid*
    /// degree-`t` sharing whose secret is `z + λ_i·δ`. There is NO off-curve point,
    /// NO degree inflation, NOTHING for Reed–Solomon / the robust open to detect:
    /// every party's share lies exactly on a consistent degree-`t` polynomial. This
    /// is structurally INDISTINGUISHABLE from an honest reduction of a different
    /// product. A post-hoc tamper that adds a uniform `δ` to every OUTPUT share's `y`
    /// (e.g. `auth_compare::tests::tamper_value`) reaches the same *observable* `[z]`,
    /// but it models tampering the RESULT, not a party deviating INSIDE the reduce;
    /// crucially it cannot be wired to drive a SOUND-vs-UNSOUND `auth_mul`
    /// discrimination (see [`MacSession::auth_mul_with_value_reduce_tamper_for_test`]),
    /// which is the property bead sq-81gd pins.
    ///
    /// The `secret_shift` out-param receives `λ_{reshare_party}·δ`, the exact amount
    /// the reduced secret was shifted — the caller uses it to construct the UNSOUND
    /// (`[z]·[α]`) MAC that would *track* the tampered value, so the regression test
    /// can show the unsound design fails to catch what the sound one catches.
    ///
    /// Only the recombination parties `0..2t+1` re-share, so `reshare_party` must be
    /// in that range. NOT part of any production path (`#[cfg(test)]`). `[OPUS-4.8]`
    #[cfg(test)]
    pub(crate) fn degree_reduce_tamper_inside_reshare_for_test(
        &mut self,
        shares_2t: &[Share],
        reshare_party: usize,
        delta: Fp,
        secret_shift: &mut Fp,
    ) -> Result<Vec<Share>, MpcError> {
        assert!(
            reshare_party < 2 * self.t + 1,
            "only the first 2t+1 recombination parties re-share"
        );
        assert_eq!(shares_2t.len(), self.n, "expected the full n-party sharing");

        let recomb_points: Vec<u64> = shares_2t[..2 * self.t + 1].iter().map(|s| s.x).collect();
        let lambdas = lagrange_zero_weights(&recomb_points);

        // The deviation: the `reshare_party`-th party re-shares `h_i + δ` instead of
        // `h_i`. `share()` still emits a perfectly consistent degree-t codeword — the
        // tamper is invisible to any consistency / RS check, exactly the Hole-2 point.
        let mut resharings: Vec<Vec<Share>> = Vec::with_capacity(2 * self.t + 1);
        for (i, s) in shares_2t[..2 * self.t + 1].iter().enumerate() {
            let to_share = if i == reshare_party {
                s.y.add(delta)
            } else {
                s.y
            };
            resharings.push(self.share(to_share));
        }

        // The reduced secret is shifted by exactly λ_{reshare_party}·δ.
        *secret_shift = lambdas[reshare_party].mul(delta);

        let mut reduced: Vec<Share> = shares_2t
            .iter()
            .map(|s| Share {
                x: s.x,
                y: Fp::zero(),
            })
            .collect();
        for (i, sub) in resharings.iter().enumerate() {
            let lambda = lambdas[i];
            for (out, sub_share) in reduced.iter_mut().zip(sub.iter()) {
                out.y = out.y.add(lambda.mul(sub_share.y));
            }
        }
        Ok(reduced)
    }

    /// Reconstruct (RNG-free). Like [`ShamirBackend::reconstruct`], this uses the
    /// consistency-checked / robust path (sq-m34i) when redundancy is present.
    pub fn reconstruct(&self, shares: &[Share]) -> Result<Fp, MpcError> {
        crate::robust::reconstruct_robust(shares, self.t)
    }

    /// **Begin an IT-MAC authenticated-sharing session (sq-km34.1).** Mints ONE
    /// session-global MAC key `α ∈ F_p` from this dealer's masking RNG (an
    /// OS-seeded ChaCha20 CSPRNG in production, sq-1vt), secret-shares it as a
    /// degree-`t` Shamir sharing `[α]`, and returns a [`MacSession`] that produces
    /// authenticated sharings `[[x]] = ([x], [α·x])` under that single key.
    ///
    /// **`α` is never returned and never reconstructed.** The cleartext `α` is
    /// drawn, immediately consumed to compute the MAC of each shared value, and
    /// kept ONLY inside the returned [`MacSession`] (which exposes no opening path);
    /// `[α]` lives behind the un-openable [`crate::authenticated::MacKey`]. This is
    /// the foundation for honest-majority malicious-with-abort security (design
    /// `research/mpc-malicious-security-design.md` §2.1–2.2); it is NOT yet
    /// malicious-secure on its own — the MAC-carrying multiplication (sq-km34.2)
    /// and the batched MAC-check at open time (sq-km34.4) are separate beads.
    ///
    /// Consumes a fresh `α` per session: call [`ShamirBackend::dealer`] +
    /// `new_mac_session` once per protocol run so two sessions never reuse a key.
    /// `[OPUS-4.8]`
    pub fn new_mac_session(&mut self) -> MacSession<'_> {
        // Draw the session-global MAC key α from the (CS)PRNG and share it. The
        // value `alpha` is held only inside the MacSession (no opening accessor);
        // `[α]` is wrapped in the un-openable MacKey.
        let alpha = self.rng.next_fp();
        let alpha_shares = self.share(alpha);
        let key = crate::authenticated::MacKey::from_shares(alpha_shares, self.t);
        MacSession {
            dealer: self,
            alpha,
            key,
            #[cfg(test)]
            sigma_opens: 0,
        }
    }
}

/// [OPUS-4.8] sq-81gd — **test-only** selector for how
/// [`MacSession::auth_mul_with_value_reduce_tamper_for_test`] carries the output MAC.
/// It exists ONLY so the regression suite can run the SAME in-reduce VALUE tamper
/// against BOTH the sound and the (rejected) unsound MAC-carry and PROVE they
/// diverge — the production [`MacSession::auth_mul`] is hard-wired to the sound route
/// and offers no such toggle. `#[cfg(test)]`.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MacCarry {
    /// PRODUCTION route: `[α·z] = reduce([α·x]·[y])` — an INDEPENDENT reduce over the
    /// input MAC that never saw the VALUE-reduce tamper, so it holds the TRUE `α·z`
    /// and the batched check fires on a tampered value.
    SoundIndependentReduce,
    /// REJECTED route (design §2.4 "REJECTED variant"): `[α·z] = reduce([z]·[α])` from the
    /// just-reduced (tampered) value times `[α]`, so the MAC TRACKS the tampered
    /// value and the batched check is fooled (`σ = 0`). Used only to demonstrate the
    /// false-negative the sound route avoids.
    UnsoundFromValueTimesAlpha,
}

/// An **IT-MAC authenticated-sharing session** bound to a single session-global,
/// secret-shared MAC key `[α]` (design §2.1). Created by
/// [`ShamirDealer::new_mac_session`]; borrows the dealer so it can draw fresh
/// degree-`t` masking polynomials for each value AND each value's MAC sharing from
/// the same CSPRNG stream.
///
/// **`α` is structurally un-openable through this type.** The cleartext `α` is a
/// PRIVATE field, used only to compute `α·x` before sharing the MAC; there is no
/// public accessor that returns it or reconstructs `[α]`. The only key material a
/// caller can reach is the [`crate::authenticated::MacKey`] from [`Self::mac_key`],
/// which itself exposes no opening path (bead sq-km34.1 acceptance (2)).
/// `[OPUS-4.8]`
pub struct MacSession<'a> {
    /// The borrowed dealer: source of the fresh masking randomness for every
    /// value and MAC sharing produced in this session.
    dealer: &'a mut ShamirDealer,
    /// The session-global MAC key `α` in the clear — PRIVATE. Used only to compute
    /// `α·x` before sharing; never returned by any public method (acceptance (2)).
    alpha: Fp,
    /// The secret-shared, un-openable `[α]` handed to callers via [`Self::mac_key`]
    /// and consumed by the §2.3 add-constant MAC term.
    key: crate::authenticated::MacKey,
    /// **Test-only.** Count of `σ` opens spent by [`Self::mac_check_and_open`] — the
    /// instrument behind the §2.5/§5 amortisation test. Not present in a production
    /// build, so it costs the session nothing.
    #[cfg(test)]
    sigma_opens: u64,
}

impl std::fmt::Debug for MacSession<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the cleartext session MAC key α.
        f.debug_struct("MacSession")
            .field("dealer", &self.dealer)
            .field("alpha", &"<secret session MAC key α>")
            .field("key", &self.key)
            .finish()
    }
}

// [OPUS-4.8] sq-u8a8: zeroize the cleartext session MAC key α on drop. `alpha`
// is the most sensitive secret in the MPC estate — it is the SPDZ-family
// authenticator that must never be opened (acceptance (2)) — so when the session
// ends we scrub it from memory rather than leaving it in freed stack/heap. The
// secret-shared `[α]` inside `key` (a `MacKey`) is scrubbed by `MacKey`'s own
// `Drop`. This is HYGIENE: it runs only at end-of-life, after every `α·x` MAC has
// already been minted, so it changes no protocol behaviour.
impl Drop for MacSession<'_> {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.alpha.zeroize();
    }
}

impl MacSession<'_> {
    /// The session's MAC key sharing `[α]` (un-openable). Clones the wrapper, which
    /// carries only the per-party shares of `α` — no cleartext key — so a caller
    /// can use it for the §2.3 add-constant MAC term
    /// ([`crate::authenticated::auth_add_constant`]) without any way to open `α`.
    pub fn mac_key(&self) -> crate::authenticated::MacKey {
        self.key.clone()
    }

    /// Produce an **authenticated sharing** `[[x]] = ([x], [α·x])` of `x` under
    /// this session's key (design §2.2). Draws TWO fresh degree-`t` masking
    /// polynomials (one for `[x]`, one for `[α·x]`) from the session RNG, so any
    /// `≤ t` parties' views of EITHER sharing are independent of the secret.
    /// `[OPUS-4.8]`
    pub fn authenticated_share(&mut self, x: Fp) -> crate::authenticated::AuthenticatedShare {
        let value = self.dealer.share(x);
        // The MAC is α·x, shared on a FRESH independent degree-t polynomial.
        let mac = self.dealer.share(self.alpha.mul(x));
        // value and mac are both n-party canonical sharings (x = 1..=n), so the
        // point-alignment invariant in `AuthenticatedShare::new` always holds.
        crate::authenticated::AuthenticatedShare::new(value, mac)
            .expect("share() emits matched canonical party points for value and MAC")
    }

    /// [OPUS-4.8] sq-dwb5 — **batched / vector AUTHENTICATED sharing.** Produce one
    /// [`crate::authenticated::AuthenticatedShare`] `[[values[i]]] = ([values[i]],
    /// [α·values[i]])` per element, ALL under THIS session's single MAC key `α`.
    /// The returned vector is *positionally* row-bound: index `i` is the
    /// authenticated sharing of `values[i]` (see
    /// [`crate::batched::BatchedAuthShares`]).
    ///
    /// Every element draws TWO fresh independent degree-`t` masking polynomials
    /// (value + MAC), so any `≤ t` parties' views of the whole batch are jointly
    /// independent of all the values AND of `α`. Sharing the WHOLE batch under one
    /// `α` is exactly what makes the later batched MAC-check (sq-km34.4) able to
    /// authenticate a vector with a single random linear combination — the
    /// vectorised analogue of the single-scalar MAC. `α` is never reconstructed.
    pub fn authenticated_share_batch(
        &mut self,
        values: &[Fp],
    ) -> Vec<crate::authenticated::AuthenticatedShare> {
        values
            .iter()
            .map(|&v| self.authenticated_share(v))
            .collect()
    }

    /// [OPUS-4.8] sq-ka8m / sq-km34.3 — **MAC-carrying secret×secret multiplication**
    /// (design §2.4 route (a)). Given authenticated `[[x]] = ([x],[α·x])` and
    /// `[[y]] = ([y],[α·y])`, produce the authenticated product `[[z]] = ([z],[α·z])`
    /// with `z = x·y`, so the chain can keep multiplying while staying tamper-evident
    /// under the batched MAC-check (§2.5, [`Self::mac_check`]).
    ///
    /// ## How (two INDEPENDENT BGW mult-then-reduce rounds — the Chida-et-al. shape)
    ///
    /// 1. The VALUE product `[z] = reduce([x]·[y])` — one degree-`2t`
    ///    [`mul_shares_raw`] then one BGW [`ShamirDealer::degree_reduce`].
    /// 2. The MAC `[α·z] = reduce([α·x]·[y])` — computed from the INPUT MAC `[α·x]`
    ///    times the input value `[y]`, since `(α·x)·y = α·(x·y) = α·z`. This is a
    ///    SECOND, INDEPENDENT product + reduce over DIFFERENT input shares than step
    ///    1 (`[α·x]·[y]`, not `[z]·[α]`). Carrying the MAC forward through the input
    ///    MAC — rather than recomputing it from the just-reduced value — is exactly
    ///    what makes the multiplication TAMPER-EVIDENT (next paragraph), and is the
    ///    standard honest-majority malicious-multiplication shape (Chida et al.
    ///    "Fast Large-Scale Honest-Majority MPC for Malicious Adversaries",
    ///    CRYPTO'18; the design's route (a)).
    ///
    /// ## Why this MAC-COVERS the `degree_reduce` re-sharing (Hole 2)
    ///
    /// `degree_reduce`'s re-sharing step (`shamir.rs`) is the one place a deviating
    /// party can re-share `h_i + δ` and produce a *perfectly consistent* degree-`t`
    /// codeword of a WRONG product — undetectable by Reed–Solomon even at
    /// over-provisioned `n`, because the tampering is on the value being shared, not
    /// on the open. The value reduce (step 1) and the MAC reduce (step 2) are
    /// SEPARATE re-sharings over DIFFERENT input shares, so a party that injects `δ`
    /// into ONE of them cannot land both on a consistent `(z+δ, α·(z+δ))` pair:
    /// - tamper the value reduce → `[z]` opens to `z+δ` while `[α·z]` still holds the
    ///   true `α·z` (computed from `[α·x]·[y]`) → `σ = α·z − (z+δ)·α = −δ·α ≠ 0`;
    /// - tamper the MAC reduce → `[α·z]` holds `α·z+δ` while `[z]` opens to the true
    ///   `z` → `σ = δ ≠ 0`;
    /// - tamper BOTH, coordinated (net shifts `δ_v`, `δ_m`) → `σ = δ_m − α·δ_v`, zero
    ///   only if the adversary picks `δ_m = α·δ_v`, i.e. only by guessing the secret `α`
    ///   (pinned by `auth_compare::tests::coordinated_tamper_in_both_reduces_is_caught`).
    ///
    /// Every way the §2.5 batched check fires (`≈ 1 − 2^{−61}`): the adversary would
    /// have to fix the OTHER side without knowing the secret `α`, which it cannot. The
    /// reduction is therefore no longer *trusted* — its correctness is *checked*.
    /// (Recomputing `[α·z]` from `[z]·[α]` instead would NOT have this property — the
    /// MAC would just track whatever tampered value `[z]` carried, and `σ` would be 0.)
    ///
    /// HONESTY (cost): route (a) roughly DOUBLES the per-multiplication work — two
    /// `mul_shares_raw` + two `degree_reduce` rounds instead of one (design §5). This
    /// is the documented price of authentication; it does NOT enter a preprocessing
    /// phase (that is route (b) / the dishonest-majority continuation, sq-j5ok).
    /// Honest-majority, malicious-with-abort (the abort comes from
    /// [`Self::mac_check`], not from this primitive alone). `[OPUS-4.8]`
    pub fn auth_mul(
        &mut self,
        x: &crate::authenticated::AuthenticatedShare,
        y: &crate::authenticated::AuthenticatedShare,
    ) -> Result<crate::authenticated::AuthenticatedShare, MpcError> {
        // 1. Value product z = x·y: degree-2t product, then BGW reduce to degree-t.
        let z_2t = mul_shares_raw(x.value_shares(), y.value_shares())?;
        let z = self.dealer.degree_reduce(&z_2t)?;
        // 2. MAC [α·z] = reduce([α·x]·[y]) — computed from the INPUT MAC `[α·x]`
        //    times the input value `[y]`, NOT from the just-reduced `[z]` times
        //    `[α]`. This independence is what makes the multiplication TAMPER-EVIDENT
        //    (Hole 2): the value reduction (step 1) and the MAC reduction (here) are
        //    SEPARATE BGW re-sharings over DIFFERENT input shares, so a deviating
        //    party that injects δ into ONE of them cannot make both land on a
        //    consistent `(z+δ, α·(z+δ))` pair. If the value reduce is tampered, `[z]`
        //    opens to `z+δ` while `[α·z]` still holds `α·z` → σ = α·z − (z+δ)·α =
        //    −δ·α ≠ 0; if the MAC reduce is tampered, `[α·z]` holds `α·z+δ` while
        //    `[z]` opens to `z` → σ = δ ≠ 0. Either way the §2.5 batched check fires
        //    (the adversary cannot fix the other side without knowing the secret α).
        //    Recomputing `[α·z]` from `[z]·[α]` would NOT have this property — the MAC
        //    would just track whatever (tampered) value `[z]` carried. This is the
        //    Chida-et-al. honest-majority malicious multiplication shape (the MAC is
        //    carried forward via the input MAC, then checked at output).
        // [OPUS-4.8] The genuine regression test that DISTINGUISHES this sound carry
        //    from the unsound `[z]·[α]` route must inject δ INSIDE this `degree_reduce`
        //    over the independent input shares (a post-hoc output tamper cannot tell the
        //    two routes apart — both leave the honest MAC opening to `α·z`). Tracked as
        //    bead sq-81gd.
        let mac_2t = mul_shares_raw(x.mac_shares(), y.value_shares())?;
        let mac = self.dealer.degree_reduce(&mac_2t)?;
        crate::authenticated::AuthenticatedShare::new(z, mac)
    }

    /// [OPUS-4.8] sq-81gd — **test-only `auth_mul` whose VALUE degree-reduce is
    /// tampered INSIDE the re-sharing step (the genuine Hole-2 deviation), with a
    /// SOUND-vs-UNSOUND toggle for how the MAC is carried.** This is the harness the
    /// production tamper helpers cannot express: it lets the regression test inject
    /// `δ` into the BGW re-sharing of the VALUE product
    /// ([`ShamirDealer::degree_reduce_tamper_inside_reshare_for_test`], so `[z]` is a
    /// *perfectly consistent* degree-`t` sharing of `z + λ·δ`, undetectable by RS),
    /// then compute the output MAC by one of two routes:
    ///
    /// - **`MacCarry::SoundIndependentReduce`** — the PRODUCTION route: `[α·z] =
    ///   reduce([α·x]·[y])`, an INDEPENDENT reduce over the input MAC `[α·x]` that
    ///   never saw `δ`. The MAC therefore holds the TRUE `α·z`, so `σ = α·z −
    ///   (z+λδ)·α = −λδ·α ≠ 0` and [`Self::mac_check`] ABORTS.
    /// - **`MacCarry::UnsoundFromValueTimesAlpha`** — the REJECTED route the design
    ///   doc warns against: `[α·z] = reduce([z]·[α])` from the JUST-REDUCED (tampered)
    ///   value times `[α]`. The MAC then tracks the tampered `z + λδ`, so `σ = 0` and
    ///   [`Self::mac_check`] PASSES — a SILENT WRONG result.
    ///
    /// The two routes diverging on the SAME in-reduce tamper is exactly what the
    /// current harness cannot show (a post-hoc value tamper happens AFTER the unsound
    /// `[z]·[α]` recompute too, so even the unsound design would catch it). `[OPUS-4.8]`
    #[cfg(test)]
    pub(crate) fn auth_mul_with_value_reduce_tamper_for_test(
        &mut self,
        x: &crate::authenticated::AuthenticatedShare,
        y: &crate::authenticated::AuthenticatedShare,
        reshare_party: usize,
        delta: Fp,
        mac_carry: MacCarry,
    ) -> Result<crate::authenticated::AuthenticatedShare, MpcError> {
        // 1. VALUE product reduced with a δ injected INSIDE the re-sharing: [z] is a
        //    consistent degree-t sharing of (true z) + λ_{reshare_party}·δ.
        let z_2t = mul_shares_raw(x.value_shares(), y.value_shares())?;
        let mut secret_shift = Fp::zero();
        let z = self.dealer.degree_reduce_tamper_inside_reshare_for_test(
            &z_2t,
            reshare_party,
            delta,
            &mut secret_shift,
        )?;

        let mac = match mac_carry {
            // SOUND: independent reduce of [α·x]·[y]; never saw δ → holds true α·z.
            MacCarry::SoundIndependentReduce => {
                let mac_2t = mul_shares_raw(x.mac_shares(), y.value_shares())?;
                self.dealer.degree_reduce(&mac_2t)?
            }
            // UNSOUND: [α·z] = reduce([z]·[α]) — tracks whatever (tampered) z carried.
            MacCarry::UnsoundFromValueTimesAlpha => {
                let mac_2t = mul_shares_raw(&z, self.key.alpha_shares())?;
                self.dealer.degree_reduce(&mac_2t)?
            }
        };
        crate::authenticated::AuthenticatedShare::new(z, mac)
    }

    /// [SONNET-4.6] **test-only `auth_mul` that injects an in-re-sharing deviation into
    /// BOTH degree-reduces at once** — the COORDINATED Hole-2 attack the design's
    /// adversary model allows (up to `t` corrupted parties may deviate in both reduces
    /// of the same multiplication, choosing the two deviations jointly).
    ///
    /// [`Self::auth_mul_with_value_reduce_tamper_for_test`] only deviates in the VALUE
    /// reduce, so on its own it does not exercise the design's claim that the MAC reduce
    /// is covered too, nor the joint case. This helper takes an independent `δ` for each
    /// reduce (either may be zero) and otherwise runs the PRODUCTION sound carry
    /// (`[α·z] = reduce([α·x]·[y])` over the input MAC).
    ///
    /// With net secret shifts `δ_v` (value) and `δ_m` (MAC), the pair the adversary lands
    /// on is `(z + δ_v, α·z + δ_m)`, so the §2.5 check computes
    /// `σ = δ_m − α·δ_v`, which is zero only if `δ_m = α·δ_v` — i.e. only by guessing the
    /// secret `α` (probability `1/p`). `#[cfg(test)]`; NOT on any production path.
    #[cfg(test)]
    pub(crate) fn auth_mul_with_both_reduces_tampered_for_test(
        &mut self,
        x: &crate::authenticated::AuthenticatedShare,
        y: &crate::authenticated::AuthenticatedShare,
        reshare_party: usize,
        value_delta: Fp,
        mac_delta: Fp,
    ) -> Result<crate::authenticated::AuthenticatedShare, MpcError> {
        let mut scratch = Fp::zero();
        // VALUE: [z] = reduce([x]·[y]) with δ_v injected inside the re-sharing.
        let z_2t = mul_shares_raw(x.value_shares(), y.value_shares())?;
        let z = self.dealer.degree_reduce_tamper_inside_reshare_for_test(
            &z_2t,
            reshare_party,
            value_delta,
            &mut scratch,
        )?;
        // MAC: the PRODUCTION independent carry [α·z] = reduce([α·x]·[y]), with δ_m
        // injected inside ITS re-sharing. Different input shares from the value reduce —
        // that independence is the property under test.
        let mac_2t = mul_shares_raw(x.mac_shares(), y.value_shares())?;
        let mac = self.dealer.degree_reduce_tamper_inside_reshare_for_test(
            &mac_2t,
            reshare_party,
            mac_delta,
            &mut scratch,
        )?;
        crate::authenticated::AuthenticatedShare::new(z, mac)
    }

    /// [OPUS-4.8] sq-ka8m — a degree-`t` AUTHENTICATED sharing of a PUBLIC constant
    /// `c`: value sharing `[c]` (the trivial constant sharing on `x = 1..=n`) paired
    /// with the MAC `[α·c]` (= `c·[α]` via `crate::authenticated::MacKey::scaled_constant_mac`).
    /// This is the authenticated analogue of `const_sharing` — it lets the
    /// comparison chain seed its `[0]`/`[1]` accumulators (`gt`, `eq`) as authenticated
    /// values so the WHOLE chain is MAC-carried. `α·c` is the MAC of a PUBLIC value
    /// (no secret leaks); the constant sharing is degree-`0 ⊆ t`, consistent with the
    /// degree-`t` MAC. `[OPUS-4.8]`
    pub fn auth_const_sharing(&self, c: Fp) -> crate::authenticated::AuthenticatedShare {
        let value: Vec<Share> = (1..=self.dealer.parties() as u64)
            .map(|x| Share { x, y: c })
            .collect();
        let mac = self.key.scaled_constant_mac(c);
        crate::authenticated::AuthenticatedShare::new(value, mac)
            .expect("const value sharing and scaled-constant MAC share the canonical party points")
    }

    /// [OPUS-4.8] sq-6fv7 — **authenticate an EXISTING degree-`t` value sharing**
    /// `[v]` into `[[v]] = ([v], [α·v])` under this session's key, WITHOUT
    /// reconstructing `v`. This is the entry the malicious-secure
    /// [`disclose_threshold_verdict`](crate::compare::disclose_threshold_verdict)
    /// path needs: the federation hands it a plain Shamir sharing of an aggregate
    /// (e.g. the cumulative SUM from [`ShamirBackend::run_secure`]), and to carry it
    /// through the MAC-checked decompose+compare chain it must first gain a MAC.
    ///
    /// The MAC `[α·v] = reduce([α]·[v])` is computed by ONE BGW mult-then-reduce
    /// over the SECRET-shared `[α]` and the input `[v]` — the SAME shape as the MAC
    /// half of [`Self::auth_mul`] — so `α·v` is never reconstructed and `[α]` is
    /// never opened. The value sharing `[v]` is reused VERBATIM (the caller's
    /// sharing, not re-dealt). The MAC reduction is itself covered by the later
    /// batched [`Self::mac_check`]: a deviating party that re-shares `[α·v] + δ` here
    /// makes `σ = δ ≠ 0` at the check, exactly as a tampered `auth_mul` MAC reduce
    /// does. Honest-majority; tamper-evident under the §2.5 check. `[OPUS-4.8]`
    pub fn authenticate_existing(
        &mut self,
        value_shares: &[Share],
    ) -> Result<crate::authenticated::AuthenticatedShare, MpcError> {
        // [α·v] = reduce([α] · [v]): degree-2t product of the session's secret-shared
        // MAC key and the value, then BGW-reduce back to degree t. No reconstruction
        // of v, no opening of α — and the reduction is checked at output time.
        let mac_2t = mul_shares_raw(self.key.alpha_shares(), value_shares)?;
        let mac = self.dealer.degree_reduce(&mac_2t)?;
        crate::authenticated::AuthenticatedShare::new(value_shares.to_vec(), mac)
    }

    /// [OPUS-4.8] sq-6fv7 — draw a fresh nonzero `F_p` from the session's masking
    /// RNG (the SAME CSPRNG stream the value/MAC sharings are dealt from). The
    /// authenticated square protocol ([`crate::auth_disclose`]) uses it for the
    /// jointly-random `[[a]]` whose `a²` is opened (no party knows `a`). `pub(crate)`
    /// — an internal protocol accessor, not part of the malicious-secure public API.
    pub(crate) fn draw_nonzero_fp(&mut self) -> Fp {
        self.dealer.draw_nonzero_fp()
    }

    // [OPUS-4.8] sq-m4zi/sq-e7ma — `dealer_mut()` (sq-6fv7) was removed: the malicious
    // disclose path's range proof no longer borrows the bare dealer to run the
    // SEMI-HONEST `verify_sum_in_range`. It now runs the MAC-checked
    // `auth_disclose::auth_verify_sum_in_range` over AUTHENTICATED shares, which uses
    // only the existing session API (`auth_mul` / `mac_check` / `authenticated_share`).

    /// [OPUS-4.8] sq-ka8m / sq-km34.4 — **the batched random-challenge IT-MAC check**
    /// (design §2.5), run ONCE just before any value on an authenticated path is
    /// opened. This is the step that turns "the MACs were carried" into "tampering is
    /// CAUGHT" — the catch-everything check that closes Holes 1, 2 and (via the
    /// comparison chain) 4, **at the minimal `n = 2t+1`** where Reed–Solomon
    /// redundancy is zero.
    ///
    /// Given the authenticated values `[[y_1]]..[[y_k]]` whose VALUES are about to be
    /// opened (the verdict bit, the intermediate opens of a chain — every value whose
    /// integrity the verdict depends on):
    ///
    /// 1. Derive the challenge coefficients `χ_1..χ_k ∈ F_p` by **Fiat–Shamir over the
    ///    ALREADY-FIXED share vectors** (`mac_check_challenges`, crate-private — the
    ///    coin is an implementation detail, not a public surface). CRITICAL ORDER: the
    ///    challenges are a deterministic function of the `[[y_j]]` being checked, so
    ///    "drawn after the shares are fixed" is a *structural* fact rather than a
    ///    call-ordering convention. **Scope:** the transcript is the full share vectors,
    ///    which only the trusted-dealer simulation has — this is NOT a coin `n` real
    ///    parties could jointly derive; see `mac_check_challenges` for the trust model
    ///    and for the grinding bound that binding the coin to the shares implies.
    /// 2. Open the values `y_j` and form the public `y = Σ_j χ_j·y_j`.
    /// 3. Each (simulated) party locally forms its share of
    ///    `[σ] = Σ_j χ_j·[m_{y_j}] − y·[α]` (all LINEAR → free), then open `σ`.
    /// 4. **Accept iff `σ == 0`.** For honest values `σ = Σχ_j(α·y_j) − (Σχ_jy_j)α = 0`;
    ///    any tamper that changed a `y_j` without a consistent matching change to its
    ///    MAC (impossible without `α`) makes `σ ≠ 0` with probability `≥ 1 − 1/p ≈
    ///    1 − 2^{−61}` — against a deviation fixed BEFORE `χ` is known; an adversary
    ///    that can re-choose its deviation and re-derive the coin faces the `≈ 2^61`
    ///    grinding bound documented on `mac_check_challenges` instead. On `σ ≠ 0` this
    ///    returns [`MpcError::MacCheckFailed`] (ABORT).
    ///
    /// **One `σ` open per BATCH, not per value** — this is the amortisation the design
    /// (§5, "Output: one batched MAC-check") banks on: an all-pairs hidden join's
    /// `|L|·|R|` equality opens are authenticated by a *single* `σ` open, so the
    /// marginal round cost of the malicious upgrade is `O(1)` per opened batch, not
    /// `O(1)` per opened value. Pass the WHOLE batch in one call; calling this once
    /// per value forfeits exactly that amortisation.
    ///
    /// `σ` is **leakage-free**: it is identically `0` for honest executions and is a
    /// random-coefficient combination of MACs minus the same of values otherwise — it
    /// reveals nothing about the inputs (design §2.5 confidentiality note; the
    /// coZK-2025/1026 mitigation is that the check runs BEFORE the result is acted on).
    /// `α` is never reconstructed: only `σ` (which is `0`) and the public `y_j` are
    /// opened.
    ///
    /// Returns the **opened, MAC-verified** values `y_1..y_k`, positionally aligned
    /// with `values` — so the caller acts on exactly the values the check covered, and
    /// on a failing check gets an `Err` and NO value at all (the abort-before-acting
    /// discipline). Prefer this over [`Self::mac_check`] + a separate open: the latter
    /// re-opens each value a second time and leaves the "checked" and "acted on"
    /// quantities bound only by convention. `[OPUS-4.8]`
    pub fn mac_check_and_open(
        &mut self,
        values: &[crate::authenticated::AuthenticatedShare],
    ) -> Result<Vec<Fp>, MpcError> {
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let t = self.dealer.threshold();
        let n = self.dealer.parties();
        // 1. Challenge coefficients χ_j, bound by Fiat–Shamir to the shares being
        //    checked (so they are unavoidably derived AFTER those are fixed). The
        //    transcript is simulator-private state — see `mac_check_challenges` for why
        //    that is NOT a distributed public coin.
        let chis = mac_check_challenges(n, t, values);

        // 2./3. Build [σ] = Σ_j χ_j·[m_{y_j}] − (Σ_j χ_j·y_j)·[α] as a free local
        //       linear combination, and accumulate the public y = Σ_j χ_j·y_j.
        let mut sigma_shares: Vec<Share> =
            (1..=n as u64).map(|x| Share { x, y: Fp::zero() }).collect();
        let mut y_pub = Fp::zero();
        let mut opened = Vec::with_capacity(values.len());
        for (chi, av) in chis.iter().zip(values.iter()) {
            // Open the value y_j (public on this path) and accumulate χ_j·y_j.
            let y_j = self.dealer.reconstruct(av.value_shares())?;
            opened.push(y_j);
            y_pub = y_pub.add(chi.mul(y_j));
            // Add χ_j·[m_{y_j}] into [σ].
            let weighted_mac = scale(av.mac_shares(), *chi);
            sigma_shares = add_shares(&sigma_shares, &weighted_mac)?;
        }
        // Subtract y·[α]: scale the session's secret-shared [α] by the PUBLIC y.
        let y_alpha = scale(self.key.alpha_shares(), y_pub);
        sigma_shares = sub_shares(&sigma_shares, &y_alpha)?;

        // 4. Open σ — leakage-free (identically 0 when honest) — and check it is 0.
        //    σ is a degree-`t` sharing (a linear combination of degree-`t` MAC
        //    sharings and [α]), so we open it via the backend's robust degree-`t`
        //    path: at the minimal n = 2t+1 (zero RS redundancy) this is plain
        //    Lagrange and the σ == 0 check below is the SOLE detector (soundness from
        //    the secret α); with redundancy (n > 2t+1) the robust path additionally
        //    flags a degree-`t` inconsistency. Either way a tamper is caught.
        //    This is the ONE σ open the whole batch shares (the §5 amortisation).
        #[cfg(test)]
        {
            self.sigma_opens += 1;
        }
        let sigma = self.dealer.reconstruct(&sigma_shares)?;
        if sigma != Fp::zero() {
            return Err(MpcError::MacCheckFailed {
                detail: format!(
                    "batched IT-MAC check over {} value(s) opened σ = {} != 0 — a value, share, \
                     or degree-reduce re-sharing on the authenticated path was tampered (caught at \
                     n = {n}, t = {t}; soundness ≈ 1 − 2^-61 from the secret α, not RS redundancy)",
                    values.len(),
                    sigma.value()
                ),
            });
        }
        Ok(opened)
    }

    /// The §2.5 batched MAC-check as a pure GATE — [`Self::mac_check_and_open`] with the
    /// opened values discarded. `Ok(())` means every `[[y_j]]` in the batch is
    /// MAC-consistent; the caller then opens what it needs.
    ///
    /// Prefer [`Self::mac_check_and_open`] on any path that goes on to open one of the
    /// checked values: it hands back the values the check actually covered, so the
    /// value acted on cannot drift from the value verified, and it avoids opening each
    /// `y_j` a second time. Use this form only when the batch is checked for its own
    /// sake (nothing is opened afterwards). `[OPUS-4.8]`
    pub fn mac_check(
        &mut self,
        values: &[crate::authenticated::AuthenticatedShare],
    ) -> Result<(), MpcError> {
        self.mac_check_and_open(values).map(|_| ())
    }

    /// **Test-only.** How many `σ` opens this session has spent on batched MAC-checks
    /// — one per [`Self::mac_check_and_open`] call over a NON-empty batch, regardless
    /// of the batch size. It exists so the amortisation acceptance (a single batched
    /// check covers `|L|·|R|` equality opens) is MEASURED against the real code path
    /// rather than asserted in prose.
    #[cfg(test)]
    pub(crate) fn sigma_opens_for_test(&self) -> u64 {
        self.sigma_opens
    }

    /// **Test-only.** The cleartext session MAC key `α`, so tests can verify the
    /// MAC relation `reconstruct([α·x]) == α·x`. NOT part of the public API — the
    /// `cfg` gate guarantees production code physically cannot read `α` (acceptance
    /// (2): no `α`-opening path in normal builds).
    #[cfg(test)]
    pub(crate) fn alpha_for_test(&self) -> Fp {
        self.alpha
    }

    /// **Test-only.** The raw `[α]` shares, so the independence test (acceptance
    /// (3)) can exhibit a second key `α'` consistent with any `t` of them. NOT in
    /// the public API (the production [`crate::authenticated::MacKey`] never
    /// surfaces its shares for reconstruction).
    #[cfg(test)]
    pub(crate) fn alpha_shares_for_test(&self) -> Vec<Share> {
        self.key.shares_for_test()
    }
}

/// Domain-separation tag for the §2.5 batched MAC-check challenge. Versioned and
/// fixed: changing it changes every derived challenge (which is harmless — the coin is
/// re-derived from scratch on each check — but it must never collide with another
/// hash use in the crate, e.g. [`crate::term_encode::DOMAIN_TAG`]).
pub(crate) const MAC_CHECK_DOMAIN_TAG: &[u8] = b"sparq-mpc/mac-check-challenge/v1\0";

/// [OPUS-4.8] sq-km34.4 — the challenge coefficients `χ_1..χ_k ∈ F_p` for the §2.5
/// batched MAC-check, derived by Fiat–Shamir from a domain-separated transcript of the
/// share vectors being checked.
///
/// ## SCOPE: a SIMULATED coin under the trusted dealer, not a distributed public coin
///
/// The design (§2.5 step 2) wants the `χ_j` to be a public coin derived AFTER the shares
/// are fixed, so a deviating party cannot adapt its tampering to the challenge. Hashing
/// the (already-fixed) `[[y_j]]` delivers the *after* half **structurally**: the coin is a
/// deterministic function of exactly the shares under check, so it cannot precede them and
/// any tamper re-randomises it — where a draw from the dealer's private masking CSPRNG
/// achieved that only by call ORDERING. That is the whole of what this function buys.
///
/// It does **not** deliver the *public* half, and no such claim is made here. The
/// transcript is the COMPLETE value and MAC share vectors, which exist in one place only
/// because [`ShamirDealer`] is an in-process simulation of all `n` parties. In a real
/// deployment party `i` holds its own `(x_i, y_i)` and nothing else: the full vectors are
/// dealer-private state, not public transcript data, and broadcasting them would
/// reconstruct the opened values and expose MAC-sharing material. So this coin is **not
/// something `n` real parties could jointly derive, and it does not remove the trusted
/// dealer** — the trusted-dealer simulation is precisely the trust model it is scoped to,
/// and the tests below exercise it with one [`MacSession`] holding every share.
///
/// Making the coin genuinely public needs a protocol this crate does not implement: each
/// party broadcasts a binding commitment to its own fixed shares (or the parties run a
/// commit-then-reveal coin toss), then `χ = H(commitments)`, with specified broadcast,
/// verification and abort behaviour. That is a DIFFERENT transcript carrying its own
/// soundness argument — not a re-labelling of this one.
///
/// ## Grinding: what the bound actually is
///
/// For deviations `δ_{y,j}` / `δ_{m,j}` on the value and MAC sharings the check quantity
/// is `σ = Σ_j χ_j·(δ_{m,j} − α·δ_{y,j})`. It is **not** true that testing a candidate
/// `χ` always requires the secret `α`: for the value-only deviation class (`δ_m = 0` —
/// the Hole-2 tamper, and exactly what `tamper_value` injects in the tests below) this
/// collapses to `σ = −α·Σ_j χ_j·δ_{y,j}`, so `σ = 0` iff `Σ_j χ_j·δ_{y,j} = 0`. That is a
/// condition on the `χ` and the attacker's OWN `δ`, testable with no knowledge of `α`.
///
/// So `≥ 1 − 1/p ≈ 1 − 2^{−61}` is the bound against a deviation fixed BEFORE `χ` is
/// known. Binding `χ` to the shares makes the coin and the deviation co-determined, so an
/// adversary free to re-choose `δ` and re-evaluate `H` is grinding: each trial costs one
/// hash plus `k` multiplications and cancels with probability `≈ 1/p`, i.e. `≈ p ≈ 2^61`
/// expected trials. In this 61-bit field that is a COMPUTATIONAL bound, not the
/// information-theoretic one §2.5 states, and `2^61` is not a comfortable margin.
///
/// It does not bite as used here — the simulation has no party that could grind; test
/// deviations are injected non-adaptively. But it is a live constraint on the distributed
/// protocol sketched above, which must make each party's contribution BINDING before `χ`
/// is derived (what commit-then-challenge is for) rather than inherit this transcript.
/// Nothing here is externally audited — sq-qhy4; the MPC estate is research-grade and
/// semi-honest outside this authenticated path.
///
/// The `χ_j` are uniform over ALL of `F_p`, zero included: a `χ_j = 0` drops value `j`
/// from that check, which is exactly the `1/p` term already inside the soundness bound
/// (forcing them nonzero would skew the distribution the bound is stated over).
pub(crate) fn mac_check_challenges(
    n: usize,
    t: usize,
    values: &[crate::authenticated::AuthenticatedShare],
) -> Vec<Fp> {
    use sha2::{Digest, Sha512};

    /// Absorb one share vector: its length, then each `(x, y)` as fixed-width
    /// big-endian words. Length-prefixing keeps the transcript unambiguous (two
    /// different share layouts can never serialise to the same bytes).
    fn absorb(hasher: &mut Sha512, shares: &[Share]) {
        hasher.update((shares.len() as u64).to_be_bytes());
        for s in shares {
            hasher.update(s.x.to_be_bytes());
            hasher.update(s.y.value().to_be_bytes());
        }
    }

    // The transcript root commits to the whole batch: the protocol parameters, the
    // batch size, and BOTH halves ([y_j] and [α·y_j]) of every authenticated sharing.
    let mut transcript = Sha512::new();
    transcript.update(MAC_CHECK_DOMAIN_TAG);
    transcript.update((n as u64).to_be_bytes());
    transcript.update((t as u64).to_be_bytes());
    transcript.update((values.len() as u64).to_be_bytes());
    for av in values {
        absorb(&mut transcript, av.value_shares());
        absorb(&mut transcript, av.mac_shares());
    }
    let root = transcript.finalize();

    // Expand the root into one challenge per value. Each χ_j depends on the WHOLE
    // batch (via the root), so tampering with any single [[y_j]] re-randomises every
    // coefficient — an adversary cannot hold the other challenges fixed while it
    // searches for a favourable one.
    (0..values.len())
        .map(|j| {
            let mut h = Sha512::new();
            h.update(MAC_CHECK_DOMAIN_TAG);
            h.update(&root[..]);
            h.update((j as u64).to_be_bytes());
            let digest = h.finalize();
            // 128 bits reduced mod the 61-bit p: statistically uniform over F_p, the
            // same fold `crate::term_encode` uses.
            let mut be = [0u8; 16];
            be.copy_from_slice(&digest[..16]);
            Fp::new((u128::from_be_bytes(be) % (crate::field::P as u128)) as u64)
        })
        .collect()
}

/// Evaluate a polynomial (coeffs low-to-high) at `x` by Horner's method.
fn eval_poly(coeffs: &[Fp], x: Fp) -> Fp {
    let mut acc = Fp::zero();
    for &c in coeffs.iter().rev() {
        acc = acc.mul(x).add(c);
    }
    acc
}

/// The public **Lagrange-at-0 recombination weights** `λ_i` for evaluation points
/// `xs`: the unique scalars with `P(0) = Σ_i λ_i · P(x_i)` for any polynomial `P`
/// of degree `< xs.len()` sampled at `xs`. `λ_i = Π_{j≠i} (0 − x_j)/(x_i − x_j)`.
///
/// This is the SAME basis-weight computation as Lagrange interpolation at zero,
/// factored out so [`ShamirDealer::degree_reduce`] can reuse it as a FIXED public
/// linear map (sq-dvuc). The points must be distinct and nonzero; the caller
/// guarantees this (party indices are `1..=n`). RNG-free / public. `[OPUS-4.8]`
fn lagrange_zero_weights(xs: &[u64]) -> Vec<Fp> {
    xs.iter()
        .enumerate()
        .map(|(i, &xi_raw)| {
            let xi = Fp::new(xi_raw);
            let mut num = Fp::one();
            let mut den = Fp::one();
            for (j, &xj_raw) in xs.iter().enumerate() {
                if i == j {
                    continue;
                }
                let xj = Fp::new(xj_raw);
                num = num.mul(xj.neg()); // (0 − x_j)
                den = den.mul(xi.sub(xj)); // (x_i − x_j)
            }
            num.mul(den.inv())
        })
        .collect()
}

/// Lagrange-interpolate the shares' polynomial and evaluate it at `x = 0` (the
/// secret). Requires `>= t+1` distinct points.
///
/// This is the unchecked RS-decode-under-no-errors primitive — it does NOT
/// detect tampering. Every PRODUCTION reconstruction path now routes through the
/// consistency-checked / robust entry point [`crate::robust::reconstruct_robust`]
/// (wired into [`ShamirBackend::reconstruct`] AND, since WI-2 / sq-7q9i, into the
/// degree-`2t` [`reconstruct_degree`] open). This unchecked Lagrange-at-0 helper
/// therefore has NO production caller left; it is kept `#[cfg(test)]` purely as
/// the differential REFERENCE the adversarial suite compares the robust path
/// against (robust-vs-robust would be vacuous — see `robust.rs` tests).
#[cfg(test)]
pub(crate) fn reconstruct_at_zero(shares: &[Share], t: usize) -> Result<Fp, MpcError> {
    if shares.len() < t + 1 {
        return Err(MpcError::Protocol(format!(
            "Shamir reconstruction needs >= {} shares, got {}",
            t + 1,
            shares.len()
        )));
    }
    // Lagrange at 0: sum_i y_i * prod_{j != i} (0 - x_j)/(x_i - x_j).
    let mut secret = Fp::zero();
    for (i, si) in shares.iter().enumerate() {
        let xi = Fp::new(si.x);
        let mut num = Fp::one();
        let mut den = Fp::one();
        for (j, sj) in shares.iter().enumerate() {
            if i == j {
                continue;
            }
            let xj = Fp::new(sj.x);
            num = num.mul(xj.neg()); // (0 - x_j)
            den = den.mul(xi.sub(xj)); // (x_i - x_j)
        }
        secret = secret.add(si.y.mul(num.mul(den.inv())));
    }
    Ok(secret)
}

/// Add two share-vectors component-wise (parties at the same `x` add their
/// shares). This is the **local, non-interactive** linear operation that makes
/// the cumulative-sum aggregate free under Shamir: `Share(a) + Share(b)` is a
/// valid sharing of `a + b` on the SAME points, with no communication. Both
/// inputs must be sharings on the identical party-point set.
pub fn add_shares(a: &[Share], b: &[Share]) -> Result<Vec<Share>, MpcError> {
    if a.len() != b.len() {
        return Err(MpcError::Protocol(
            "add_shares: share vectors differ in length (different party sets)".into(),
        ));
    }
    a.iter()
        .zip(b.iter())
        .map(|(sa, sb)| {
            if sa.x != sb.x {
                return Err(MpcError::Protocol(
                    "add_shares: shares are on different evaluation points".into(),
                ));
            }
            Ok(Share {
                x: sa.x,
                y: sa.y.add(sb.y),
            })
        })
        .collect()
}

/// Add a public field constant to a sharing (local, non-interactive). Adding `c`
/// to every share replaces `f(x)` by `f(x) + c`, whose value at `x = 0` is
/// `secret + c`, still a valid degree-`t` sharing.
pub fn add_constant(a: &[Share], c: Fp) -> Vec<Share> {
    a.iter()
        .map(|s| Share {
            x: s.x,
            y: s.y.add(c),
        })
        .collect()
}

/// Multiply a sharing by a public field constant (local, non-interactive): scale
/// every share. `c * f(x)` interpolates to `c * secret` at `x = 0` and stays
/// degree `t`.
pub fn scale(a: &[Share], c: Fp) -> Vec<Share> {
    a.iter()
        .map(|s| Share {
            x: s.x,
            y: s.y.mul(c),
        })
        .collect()
}

/// Subtract two sharings component-wise (local). `Share(a) - Share(b)` is a
/// valid degree-`t` sharing of `a - b`.
pub fn sub_shares(a: &[Share], b: &[Share]) -> Result<Vec<Share>, MpcError> {
    if a.len() != b.len() {
        return Err(MpcError::Protocol("sub_shares: length mismatch".into()));
    }
    a.iter()
        .zip(b.iter())
        .map(|(sa, sb)| {
            if sa.x != sb.x {
                return Err(MpcError::Protocol("sub_shares: point mismatch".into()));
            }
            Ok(Share {
                x: sa.x,
                y: sa.y.sub(sb.y),
            })
        })
        .collect()
}

/// **Local share-products before degree reduction.** Multiplying two degree-`t`
/// sharings component-wise yields, at each party point, `f(x)·g(x)` — a sharing
/// of `a·b` but on a polynomial of degree `2t`. Reconstructing it therefore
/// needs `2t+1` points (honest-majority gives `2t+1 <= n`). For a single product
/// that is opened immediately (the equality test opens its masked product) this
/// degree-`2t` sharing is reconstructed directly via [`reconstruct_degree`] — no
/// further work is needed.
///
/// To CHAIN multiplications (`a·b·c`, secure comparison/threshold, conjunctive
/// hidden-pattern joins) the degree-`2t` product must be brought back to degree
/// `t` first: feed the result of `mul_shares_raw` into
/// [`ShamirDealer::degree_reduce`] (the BGW reshare-and-recombine round, sq-dvuc)
/// before the next multiplication.
///
/// HONESTY: the round/degree cost is real and stated — one multiplication is one
/// interaction round and consumes the `n >= 2t+1` headroom; the degree-reduction
/// round (sq-dvuc) is a SECOND (simulated) round that restores degree `t` so the
/// next product fits, under the SAME honest-majority / semi-honest model.
pub fn mul_shares_raw(a: &[Share], b: &[Share]) -> Result<Vec<Share>, MpcError> {
    if a.len() != b.len() {
        return Err(MpcError::Protocol("mul_shares_raw: length mismatch".into()));
    }
    a.iter()
        .zip(b.iter())
        .map(|(sa, sb)| {
            if sa.x != sb.x {
                return Err(MpcError::Protocol("mul_shares_raw: point mismatch".into()));
            }
            Ok(Share {
                x: sa.x,
                y: sa.y.mul(sb.y),
            })
        })
        .collect()
}

/// Reconstruct the secret of a sharing of a known polynomial `degree`, requiring
/// `degree + 1` points. Used to open the degree-`2t` product of the equality
/// test (`degree = 2t`).
///
/// **Consistency-checked / robust at degree `degree` (sq-7q9i, WI-2).** This
/// routes the open through [`crate::robust::reconstruct_robust`] at the GIVEN
/// `degree` — the SAME Reed–Solomon checker WI-1 wired into the degree-`t`
/// reconstruction, just instantiated at the product's degree. The codeword view
/// is identical: a sharing of a degree-`degree` polynomial at `n` distinct points
/// is an `[n, degree+1]` RS codeword, so the checker:
///
/// - `n == degree + 1` (**NO redundancy**) → plain Lagrange; tampering is
///   information-theoretically undetectable and is NOT claimed otherwise.
/// - `n > degree + 1` (**redundancy**) → Berlekamp–Welch: detect any tampering
///   and abort with [`MpcError::Tampered`], correcting up to
///   `e = ⌊(n − degree − 1)/2⌋` tampered shares first.
///
/// HONESTY (the degree-`2t` boundary; bead sq-7q9i / parent sq-uu0u): for the
/// equality/mult open `degree = 2t`, redundancy exists ONLY when `n > 2t + 1`.
/// The honest-majority constructor fixes `t = ⌊(n−1)/2⌋`, so `n = 2t + 1` for odd
/// `n` (e.g. n=3,5,7,9): there is **zero** RS redundancy at degree `2t` and
/// tampering one product share is undetectable — pinned by a boundary test. A
/// true fix at `n = 2t + 1` needs an information-theoretic MAC (the deferred WI-4
/// seam, bead sq-6d6g), not RS redundancy. Even `n` (n=4,6,8) yields exactly one
/// redundant share at degree `2t` (`e_max = 0`), so tampering is DETECT-only
/// there; correction (`e_max ≥ 1`) at degree `2t` needs `n ≥ 2t + 3`.
pub fn reconstruct_degree(shares: &[Share], degree: usize) -> Result<Fp, MpcError> {
    // Same RS-consistency-checked entry point as ShamirBackend::reconstruct,
    // instantiated at the product's `degree` (here `2t`) rather than `t`. At
    // `n == degree+1` it falls back to plain Lagrange (no detection claim); with
    // redundancy it detects/corrects. See the doc above for the degree-2t bound.
    crate::robust::reconstruct_robust(shares, degree)
}

/// The Shamir `Share` representation surfaced through the trait. A holder's
/// private contribution is shared into one [`Share`] *per party*; we carry the
/// whole per-party vector as the trait's opaque `Share` so the rest of the crate
/// need not know the scheme's internals (see [`MpcBackend::Share`]).
pub type ShareVec = Vec<Share>;

impl MpcBackend for ShamirBackend {
    /// One trait-`Share` is the full per-party share-vector of a single secret.
    type Share = ShareVec;

    fn info(&self) -> BackendInfo {
        // [OPUS-4.8] sq-mq8q — the backend-level descriptor describes the PRIMARY
        // (degree-`t` linear-aggregate) reconstruction path. `BackendInfo::new`
        // derives the back-compat `trust_model` / `malicious_security` projection
        // from it, threading the degree-`t` RS correction budget `e` so the old
        // enum's `max_cheaters` stays faithful (this PRESERVES the exact prior
        // `info().malicious_security` value — `self.malicious_security()`). The
        // weaker degree-`2t` equality-open guarantee is NOT smuggled into the
        // backend-level bit; it is reported per-operator via `operator_security`.
        // Shamir shares are an [n, t+1] RS codeword, so `reconstruct` (degree `t`):
        //   - n == t+1 (NO redundancy)  → no detection possible → SemiHonestOnly.
        //   - n  > t+1, e == 0          → detect-and-abort       → Abort.
        //   - n  > t+1, e >= 1          → robust up to e cheaters → Robust{e}.
        // NB: with the honest-majority t = ⌊(n−1)/2⌋, every valid n >= 2 has
        // n > t+1, so the no-redundancy branch is UNREACHABLE for the aggregate
        // (it is real only for the degree-`2t` equality open / a stub backend).
        let e_t = self.rs_correction_budget(self.t).unwrap_or(0);
        BackendInfo::new(
            "shamir-honest-majority",
            self.operator_descriptor(OperatorClass::LinearAggregate),
            e_t,
        )
    }

    /// [OPUS-4.8] sq-mq8q — per-operator security: the degree-`t` aggregate is
    /// robust while the degree-`2t` equality open is semi-honest-only at
    /// `n = 2t+1`, so this reports each [`OperatorClass`] precisely rather than
    /// letting one backend-level bit lie. Delegates to [`Self::operator_descriptor`].
    fn operator_security(&self, operator: OperatorClass) -> SecurityDescriptor {
        self.operator_descriptor(operator)
    }

    /// Secret-share a holder's *private* contribution. For the cumulative-
    /// aggregate sub-case the holder's contribution is one private integer (its
    /// salary). We extract it from the holder's local single-row, single-column
    /// partial and share it across the `n` parties. The cleartext value NEVER
    /// leaves as cleartext — only its `n` shares do, and any `<= t` of them are
    /// independent of it.
    ///
    /// (The holder is queried for the private value via the SAME local-eval path
    /// as the disclosed flow — `evaluate_local` — but the value is shared rather
    /// than disclosed. The fragment must project exactly one integer-valued
    /// column in one row; otherwise it is a protocol error, not a guess.)
    fn share_private_input(&self, holder: &Holder) -> Result<Vec<Self::Share>, MpcError> {
        // The private contribution is named by a convention fragment: a single
        // integer the holder agrees to *secret-share* (not disclose). We use the
        // dedicated private-salary fragment.
        let private = holder.evaluate_local(PRIVATE_SALARY_FRAGMENT)?;
        let v = extract_single_integer(&private)?;
        // A fresh sharing requires fresh, INDEPENDENT randomness. Mint a fresh
        // dealer (production: a fresh OS-seeded CSPRNG — sq-1vt) rather than
        // cloning RNG state, so per-input sharings never reuse a keystream. The
        // trait stays `&self`; the dealer's randomness is per-input by design.
        let mut dealer = self.dealer();
        Ok(vec![dealer.share(Fp::new(v))])
    }

    /// [OPUS-4.8] sq-dwb5 — **batched / vector** secret-sharing of a holder's
    /// private contribution. The holder evaluates `fragment` (which MUST project
    /// exactly one integer-valued column) over its OWN data and we share EVERY row
    /// it returns — generalising [`Self::share_private_input`] (the single-row
    /// special case using the fixed `PRIVATE_SALARY_FRAGMENT`) to a per-row
    /// hidden-value / salary vector / per-graph commitment column.
    ///
    /// **Row-binding (POSITIONAL, value-sorted).** The values are extracted and
    /// then sorted ascending (`extract_integer_vector` returns them in the
    /// holder's local row order; we sort here) so two holders running the same
    /// fragment produce batches whose index `i` refers to the same logical row
    /// position regardless of each holder's local row ordering — exactly the
    /// contract a per-row secure aggregate / hidden join relies on. The cleartext
    /// values NEVER leave; only their `n·k` shares do, and any `≤ t` parties'
    /// shares of the whole batch are jointly independent of all the values.
    ///
    /// One fresh dealer (fresh OS-seeded CSPRNG in production, sq-1vt) shares the
    /// whole batch, so every row gets an independent masking polynomial without
    /// reusing a keystream across rows.
    fn share_private_inputs(
        &self,
        holder: &Holder,
        fragment: &str,
    ) -> Result<Vec<Self::Share>, MpcError> {
        let private = holder.evaluate_local(fragment)?;
        let mut values = extract_integer_vector(&private)?;
        // Positional row-binding: a deterministic order so two holders' batches
        // line up by index regardless of local row order (the documented contract).
        values.sort_unstable();
        let fps: Vec<Fp> = values.into_iter().map(Fp::new).collect();
        let mut dealer = self.dealer();
        Ok(dealer.share_batch(&fps))
    }

    /// Run the secure computation over the shared inputs. For the v1 aggregate
    /// this is the **cumulative sum** of the holders' private values: a pure
    /// linear function, so it is the local component-wise addition of the
    /// sharings ([`add_shares`]) — zero communication rounds, the honest-
    /// majority Shamir sweet spot. Returns the sharing of the sum.
    fn run_secure(&self, shares: &[Self::Share]) -> Result<Vec<Self::Share>, MpcError> {
        if shares.is_empty() {
            return Err(MpcError::Protocol("run_secure: no shared inputs".into()));
        }
        let mut acc = shares[0].clone();
        for next in &shares[1..] {
            acc = add_shares(&acc, next)?;
        }
        Ok(vec![acc])
    }

    /// Reconstruct ONLY the disclosed output (the aggregate sum). Per convention
    /// #4 the disclosed value is the minimal answer; here we reconstruct the
    /// summed sharing to a single integer and surface it as a one-row partial.
    /// Any disclosed-property post-processing (e.g. the boolean `sum > £100k`)
    /// is recomputed by the verifier OUTSIDE the crypto core (M5), not here.
    fn reconstruct_disclosed(
        &self,
        result_shares: &[Self::Share],
    ) -> Result<PartialResult, MpcError> {
        if result_shares.len() != 1 {
            return Err(MpcError::Protocol(
                "reconstruct_disclosed: expected exactly one result sharing".into(),
            ));
        }
        let sum = self.reconstruct(&result_shares[0])?;
        Ok(PartialResult {
            holder: HolderId::new("federation"),
            vars: vec![oxrdf::Variable::new_unchecked("cumulative")],
            rows: vec![vec![Some(oxrdf::Term::Literal(
                oxrdf::Literal::new_typed_literal(
                    sum.value().to_string(),
                    oxrdf::vocab::xsd::INTEGER,
                ),
            ))]],
        })
    }
}

/// The fragment a holder evaluates to surface its single private salary as a
/// value to be SECRET-SHARED (not disclosed). Kept distinct from any disclosed
/// fragment to make the disclose-vs-hide boundary explicit at the call site.
const PRIVATE_SALARY_FRAGMENT: &str =
    "PREFIX ex: <http://ex/> SELECT ?salary WHERE { ?p ex:salary ?salary }";

/// Pull a single non-negative integer out of a one-row / one-column partial.
/// Anything else (no rows, many rows, non-integer, multiple columns) is a
/// protocol error — we never guess a private input.
fn extract_single_integer(p: &PartialResult) -> Result<u64, MpcError> {
    if p.rows.len() != 1 {
        return Err(MpcError::Protocol(format!(
            "private input must be exactly one row, got {}",
            p.rows.len()
        )));
    }
    if p.vars.len() != 1 || p.rows[0].len() != 1 {
        return Err(MpcError::Protocol(
            "private input must be exactly one column".into(),
        ));
    }
    match &p.rows[0][0] {
        Some(oxrdf::Term::Literal(l)) => l
            .value()
            .parse::<u64>()
            .map_err(|e| MpcError::Protocol(format!("private input not a u64 integer: {e}"))),
        other => Err(MpcError::Protocol(format!(
            "private input must be an integer literal, got {other:?}"
        ))),
    }
}

/// [OPUS-4.8] sq-dwb5 — pull a VECTOR of non-negative integers out of a
/// single-column (any number of rows) partial — the batched generalisation of
/// [`extract_single_integer`]. Each row must bind exactly one integer literal in
/// the one projected column; an empty result is a valid empty vector (a holder
/// with no private rows contributes nothing). A multi-column partial, a missing
/// binding, or a non-integer literal is a protocol error — we never guess.
/// Values are returned in the holder's local row order; the caller imposes the
/// row-binding order (see [`ShamirBackend::share_private_inputs`]).
fn extract_integer_vector(p: &PartialResult) -> Result<Vec<u64>, MpcError> {
    if p.vars.len() != 1 {
        return Err(MpcError::Protocol(format!(
            "batched private input must project exactly one column, got {}",
            p.vars.len()
        )));
    }
    p.rows
        .iter()
        .map(|row| {
            // Fail closed on a malformed row shape: the column COUNT is checked
            // once via `p.vars.len()`, but a hand-built / adversarial
            // `PartialResult` can desync the per-row arity from `vars`. Require
            // each row to carry exactly one cell so a multi-column row can't
            // silently mis-extract by ignoring the trailing columns.
            if row.len() != 1 {
                return Err(MpcError::Protocol(format!(
                    "batched private input row must have exactly one column, got {}",
                    row.len()
                )));
            }
            match row.first() {
                Some(Some(oxrdf::Term::Literal(l))) => l.value().parse::<u64>().map_err(|e| {
                    MpcError::Protocol(format!("batched private input not a u64 integer: {e}"))
                }),
                other => Err(MpcError::Protocol(format!(
                    "batched private input must be an integer literal in each row, got {other:?}"
                ))),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! M3 Shamir tests. The load-bearing ones are:
    //! - `reconstruct_*`: the secret-sharing LAYER is correct (round-trips, and
    //!   the threshold actually hides the secret below `t+1` shares).
    //! - `secure_sum_*`: `run_secure` computes the cumulative aggregate over
    //!   shares so that reconstructing it equals the PLAINTEXT sum — the M3
    //!   differential for the secure computation itself.
    use super::*;

    #[test]
    fn share_then_reconstruct_roundtrips() {
        // Use the seedable test RNG so the simulation is reproducible; the
        // PRODUCTION path uses the OS-seeded CSPRNG (see `production_csprng_*`).
        let b = ShamirBackend::new_seeded(4, 0xC0FFEE).unwrap();
        let mut dealer = b.dealer();
        for secret in [0u64, 1, 42, 100_000, crate::field::P - 1] {
            let shares = dealer.share(Fp::new(secret));
            assert_eq!(shares.len(), 4);
            // Full set reconstructs.
            assert_eq!(b.reconstruct(&shares).unwrap(), Fp::new(secret));
            // Any t+1 subset reconstructs (threshold t = (4-1)/2 = 1 → 2 shares).
            assert_eq!(b.threshold(), 1);
            assert_eq!(b.reconstruct(&shares[..2]).unwrap(), Fp::new(secret));
        }
    }

    #[test]
    fn fewer_than_threshold_plus_one_shares_cannot_reconstruct() {
        // t = 1, so a SINGLE share is below threshold: reconstruction must error,
        // not silently return a wrong/honest-looking value.
        let b = ShamirBackend::new_seeded(4, 1).unwrap();
        let shares = b.dealer().share(Fp::new(73));
        let err = b.reconstruct(&shares[..1]).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(_)));
    }

    #[test]
    fn production_csprng_share_reconstruct_roundtrips() {
        // The PRODUCTION path (OS-seeded ChaCha20 CSPRNG, sq-1vt) must still
        // produce correct sharings: round-trip the secret through fresh dealers.
        let b = ShamirBackend::new(4).unwrap();
        for secret in [0u64, 1, 42, 100_000, crate::field::P - 1] {
            let shares = b.dealer().share(Fp::new(secret));
            assert_eq!(b.reconstruct(&shares).unwrap(), Fp::new(secret));
        }
    }

    #[test]
    fn production_csprng_two_dealers_use_independent_randomness() {
        // sq-1vt unpredictability witness: two dealers minted from the SAME
        // (production) backend must NOT produce identical shares for the same
        // secret — each `dealer()` mints a fresh OS-seeded CSPRNG, so the masking
        // polynomials differ. (Collision is cryptographically negligible.)
        let b = ShamirBackend::new(5).unwrap(); // t = 2 → random coeffs present
        let s1 = b.dealer().share(Fp::new(42));
        let s2 = b.dealer().share(Fp::new(42));
        // Both reconstruct to 42 ...
        assert_eq!(b.reconstruct(&s1).unwrap(), Fp::new(42));
        assert_eq!(b.reconstruct(&s2).unwrap(), Fp::new(42));
        // ... but the share vectors differ (different random masking polynomials).
        assert_ne!(
            s1, s2,
            "two production dealers reused the same masking randomness"
        );
    }

    #[test]
    fn threshold_hides_secret_information_theoretically() {
        // CONFIDENTIALITY witness: with t = 2 (n = 5), any 2 shares are
        // consistent with EVERY candidate secret. Concretely: there exists a
        // valid polynomial through the 2 shares for any chosen f(0). We show the
        // 2 shares do not determine the secret by exhibiting two different
        // secrets whose sharings agree on the same 2 points.
        let b = ShamirBackend::new_seeded(5, 7).unwrap();
        assert_eq!(b.threshold(), 2);
        let shares_a = b.dealer().share(Fp::new(1000));
        // For ANY two of A's shares, a degree-2 poly through them + a third
        // chosen point reconstructs a DIFFERENT secret — i.e. 2 points underdet.
        let two = &shares_a[..2];
        // Forge a third share that, with the two, interpolates to secret 2000.
        // (Existence of such a forge is exactly the information-theoretic hiding
        // argument: 2 points leave the constant term free.)
        let target = Fp::new(2000);
        let forged_third = forge_third_share(two, target);
        let mut combined = two.to_vec();
        combined.push(forged_third);
        assert_eq!(
            reconstruct_at_zero(&combined, 2).unwrap(),
            target,
            "2 shares are consistent with a different secret → they hide it"
        );
    }

    /// Given 2 shares on a degree-2 polynomial, produce a 3rd share (at a fresh
    /// x) such that the 3 interpolate to `target` at x=0. Demonstrates the
    /// hiding property; test-only.
    fn forge_third_share(two: &[Share], target: Fp) -> Share {
        // Pick x3 distinct from the two.
        let x3 = (1..=1000u64)
            .find(|x| two.iter().all(|s| s.x != *x))
            .unwrap();
        // We need f(0)=target with f through (x1,y1),(x2,y2),(x3,y3). Solve y3 so
        // the Lagrange-at-0 of the three equals target.
        // target = y1 L1 + y2 L2 + y3 L3  where Li are the Lagrange weights at 0.
        let xs = [two[0].x, two[1].x, x3];
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
        let l1 = weight(0);
        let l2 = weight(1);
        let l3 = weight(2);
        // y3 = (target - y1 L1 - y2 L2) / L3.
        let rhs = target.sub(two[0].y.mul(l1)).sub(two[1].y.mul(l2));
        let y3 = rhs.mul(l3.inv());
        Share { x: x3, y: y3 }
    }

    #[test]
    fn secure_sum_equals_plaintext_sum() {
        // THE M3 differential for run_secure: secret-share four private salaries,
        // run the secure cumulative sum, reconstruct — must equal the plaintext
        // sum. This is the flatmate cumulative-salary aggregate.
        let salaries = [30_000u64, 45_000, 28_000, 51_000];
        let plaintext_sum: u64 = salaries.iter().sum();

        let mut dealer = ShamirBackend::new_seeded(4, 0xABCD).unwrap().dealer();
        let shared: Vec<ShareVec> = salaries.iter().map(|&s| dealer.share(Fp::new(s))).collect();

        let backend = ShamirBackend::new(4).unwrap(); // reconstruction is RNG-free
        let summed = backend.run_secure(&shared).unwrap();
        let out = backend.reconstruct_disclosed(&summed).unwrap();

        // Disclosed partial carries exactly the cumulative integer.
        assert_eq!(out.rows.len(), 1);
        let got = match &out.rows[0][0] {
            Some(oxrdf::Term::Literal(l)) => l.value().parse::<u64>().unwrap(),
            other => panic!("expected integer literal, got {other:?}"),
        };
        assert_eq!(got, plaintext_sum, "secure sum must equal plaintext sum");
    }

    #[test]
    fn secure_sum_zero_and_single() {
        // Edge: a single holder's "sum" is its own value; reconstructs to it.
        let mut dealer = ShamirBackend::new_seeded(3, 5).unwrap().dealer();
        let one = vec![dealer.share(Fp::new(12345))];
        let backend = ShamirBackend::new(3).unwrap();
        let summed = backend.run_secure(&one).unwrap();
        let got = backend.reconstruct(&summed[0]).unwrap();
        assert_eq!(got, Fp::new(12345));
    }

    #[test]
    fn n_too_small_is_a_protocol_error() {
        assert!(matches!(ShamirBackend::new(1), Err(MpcError::Protocol(_))));
    }

    // ---- BGW degree-reduction round (sq-dvuc) [OPUS-4.8] ------------------

    #[test]
    fn degree_reduce_round_trips_single_product() {
        // Acceptance #1: degree_reduce(shares_2t) round-trips — reconstruct after
        // reduction == reconstruct before == plaintext product. Use n=5 (t=2) so
        // the degree-2t open at degree 2t=4 has the full 5 points, AND the reduced
        // degree-t sharing genuinely has t=2 < 5 (real reduction, not a no-op).
        let b = ShamirBackend::new_seeded(5, 0xD7C).unwrap();
        let t = b.threshold();
        assert_eq!(t, 2);
        let mut dealer = b.dealer();
        for (av, bv) in [
            (0u64, 7u64),
            (1, 1),
            (6, 9),
            (123_456, 654_321),
            (crate::field::P - 1, crate::field::P - 1),
            (crate::field::P - 1, 2),
        ] {
            let sa = dealer.share(Fp::new(av));
            let sb = dealer.share(Fp::new(bv));
            let prod_2t = mul_shares_raw(&sa, &sb).unwrap();
            let expected = Fp::new(av).mul(Fp::new(bv));

            // Reconstruct BEFORE reduction at degree 2t.
            let before = reconstruct_degree(&prod_2t, 2 * t).unwrap();
            assert_eq!(before, expected, "degree-2t product must equal a·b");

            // Reduce, then reconstruct AFTER at degree t.
            let reduced = dealer.degree_reduce(&prod_2t).unwrap();
            assert_eq!(reduced.len(), 5);
            let after = b.reconstruct(&reduced).unwrap();
            assert_eq!(after, expected, "reduced degree-t sharing must equal a·b");
            assert_eq!(before, after, "reduction must preserve the secret");
        }
    }

    #[test]
    fn two_multiplication_chain_a_b_c() {
        // Acceptance #2: a TWO-multiplication chain a·b·c — share a,b,c;
        // mul→degree-reduce→mul→degree-reduce; reconstruct; assert == plaintext
        // a·b·c. Several field values incl. edge values (0, 1, large). This is THE
        // load-bearing test: it proves multiplications now CHAIN.
        let b = ShamirBackend::new_seeded(5, 0xC4A1_5EED).unwrap();
        assert_eq!(b.threshold(), 2); // n=5 → t=2; chain stays well within 2t<n
        let mut dealer = b.dealer();
        let cases = [
            (0u64, 5u64, 9u64),          // zero short-circuits
            (1, 1, 1),                   // identities
            (1, 0, 12345),               // zero in the middle
            (2, 3, 4),                   // small
            (7, 11, 13),                 // small primes
            (100_000, 7, 9),             // mixed magnitude
            (crate::field::P - 1, 2, 3), // large × small × small (wraps mod P)
            (
                crate::field::P - 1,
                crate::field::P - 1,
                crate::field::P - 1,
            ), // all large
        ];
        for (av, bv, cv) in cases {
            let (fa, fb, fc) = (Fp::new(av), Fp::new(bv), Fp::new(cv));
            let expected = fa.mul(fb).mul(fc);

            let sa = dealer.share(fa);
            let sb = dealer.share(fb);
            let sc = dealer.share(fc);

            // mul → degree-reduce → mul → degree-reduce.
            let ab_2t = mul_shares_raw(&sa, &sb).unwrap();
            let ab_t = dealer.degree_reduce(&ab_2t).unwrap();
            let abc_2t = mul_shares_raw(&ab_t, &sc).unwrap();
            let abc_t = dealer.degree_reduce(&abc_2t).unwrap();

            let got = b.reconstruct(&abc_t).unwrap();
            assert_eq!(got, expected, "a·b·c chain failed for ({av}, {bv}, {cv})");
        }
    }

    /// [OPUS-5] The dealer's secure-multiplication counter: 0 on a fresh dealer,
    /// +1 per SUCCESSFUL degree_reduce, unchanged by a failed (fail-closed) one
    /// and by non-multiplication work (share / draw). Cost reporters take deltas
    /// of it, so its exactness is load-bearing for the modelled cost honesty.
    #[test]
    fn mult_count_tracks_successful_degree_reductions_exactly() {
        let b = ShamirBackend::new_seeded(5, 0xC0_0A7).unwrap();
        let mut dealer = b.dealer();
        assert_eq!(dealer.mult_count(), 0, "fresh dealer must start at zero");

        // Non-multiplication work does not count.
        let sa = dealer.share(Fp::new(3));
        let sb = dealer.share(Fp::new(4));
        let _ = dealer.draw_fp();
        assert_eq!(dealer.mult_count(), 0, "share/draw are not multiplications");

        // Each successful reduction counts exactly once.
        let prod_2t = mul_shares_raw(&sa, &sb).unwrap();
        let reduced = dealer.degree_reduce(&prod_2t).unwrap();
        assert_eq!(dealer.mult_count(), 1);
        let prod2_2t = mul_shares_raw(&reduced, &sa).unwrap();
        dealer.degree_reduce(&prod2_2t).unwrap();
        assert_eq!(dealer.mult_count(), 2);

        // A failed (fail-closed) reduction performs no multiplication.
        let too_few = &prod_2t[..prod_2t.len() - 1];
        dealer.degree_reduce(too_few).unwrap_err();
        assert_eq!(dealer.mult_count(), 2, "a refused reduction must not count");
    }

    #[test]
    fn degree_reduce_precondition_fails_closed() {
        // Acceptance #3: with n < 2t+1, degree_reduce returns the descriptive
        // error (no panic). We cannot build an honest-majority backend with
        // n < 2t+1 (the constructor fixes t = ⌊(n−1)/2⌋), so we simulate the
        // failure by handing degree_reduce a TRUNCATED share vector (fewer than
        // the 2t+1 = n points it needs) — the fail-closed precondition must catch
        // it with MpcError::Protocol, never a panic / wrong answer.
        let b = ShamirBackend::new_seeded(5, 1).unwrap();
        let t = b.threshold(); // 2 → needs 2t+1 = 5 shares
        let mut dealer = b.dealer();
        let prod = {
            let sa = dealer.share(Fp::new(3));
            let sb = dealer.share(Fp::new(4));
            mul_shares_raw(&sa, &sb).unwrap()
        };
        // Too few points to determine the degree-2t = 4 polynomial.
        let too_few = &prod[..2 * t]; // 4 < 2t+1 = 5
        let err = dealer.degree_reduce(too_few).unwrap_err();
        assert!(
            matches!(err, MpcError::Protocol(_)),
            "n < 2t+1 must be a descriptive protocol error, got {err:?}"
        );
        // A vector that has 2t+1 points but is NOT the full n-party set is also a
        // protocol error (degree_reduce expects the canonical full sharing).
        let mut wrong_n = prod.clone();
        wrong_n.push(Share {
            x: 99,
            y: Fp::new(7),
        });
        let err2 = dealer.degree_reduce(&wrong_n).unwrap_err();
        assert!(matches!(err2, MpcError::Protocol(_)), "got {err2:?}");
    }

    #[test]
    fn degree_reduce_rejects_permuted_or_noncanonical_input() {
        // `[OPUS-4.8]` Copilot #119 (sq-dvuc): the canonical-point precondition is
        // enforced by a REAL runtime check (a fail-closed `if`, NOT a
        // `debug_assert!` that compiles out). The reduction is a fixed public
        // linear map that pairs recombination weights / fresh sub-sharings with the
        // input BY POSITION, so a permuted-but-otherwise-valid sharing — or one on
        // non-canonical x-coords — would silently mis-reduce if accepted. Assert it
        // is rejected with `MpcError::Protocol`. Because the guard is a runtime
        // `if`, this test holds identically in debug AND release (`--release`),
        // which is the whole point of the fix.
        let b = ShamirBackend::new_seeded(5, 0x9E2B).unwrap();
        let t = b.threshold();
        assert_eq!(t, 2); // n = 5, needs the full canonical x = 1..=5 in order
        let mut dealer = b.dealer();
        let prod_2t =
            mul_shares_raw(&dealer.share(Fp::new(11)), &dealer.share(Fp::new(13))).unwrap();
        // Sanity: the genuine canonical sharing still reduces correctly.
        let expected = Fp::new(11).mul(Fp::new(13));
        let ok = dealer.degree_reduce(&prod_2t).unwrap();
        assert_eq!(b.reconstruct(&ok).unwrap(), expected);

        // (a) PERMUTED: same shares, swapped order (x no longer 1,2,3,4,5).
        let mut permuted = prod_2t.clone();
        permuted.swap(0, 4); // x = 5,2,3,4,1
        let err_perm = dealer.degree_reduce(&permuted).unwrap_err();
        assert!(
            matches!(err_perm, MpcError::Protocol(_)),
            "permuted input must be a protocol error, got {err_perm:?}"
        );

        // (b) NON-CANONICAL x: full count, in order, but one point shifted off the
        // canonical 1..=n lattice (here party 5 relabelled to x = 6).
        let mut noncanon = prod_2t.clone();
        noncanon[4].x = 6; // x = 1,2,3,4,6
        let err_nc = dealer.degree_reduce(&noncanon).unwrap_err();
        assert!(
            matches!(err_nc, MpcError::Protocol(_)),
            "non-canonical x must be a protocol error, got {err_nc:?}"
        );

        // (c) Off-by-one start (x = 0 reserved for the secret, never a party).
        let mut zero_start = prod_2t;
        zero_start[0].x = 0; // x = 0,2,3,4,5
        let err_zs = dealer.degree_reduce(&zero_start).unwrap_err();
        assert!(
            matches!(err_zs, MpcError::Protocol(_)),
            "x starting at 0 must be a protocol error, got {err_zs:?}"
        );
    }

    #[test]
    fn reduced_sharing_is_genuinely_degree_t() {
        // Acceptance #4: the reduced sharing is genuinely degree-t — any t+1 of
        // the new shares reconstruct the same secret, and a DIFFERENT t+1 subset
        // agrees. (If the reduction left the sharing at degree 2t, a t+1 subset
        // would interpolate to a WRONG value.) Use n=7 (t=3) so there are several
        // distinct t+1 = 4 subsets to compare and ample headroom over 2t=6.
        let b = ShamirBackend::new_seeded(7, 0x7E57).unwrap();
        let t = b.threshold();
        assert_eq!(t, 3);
        let mut dealer = b.dealer();
        let (av, bv) = (4321u64, 8765u64);
        let expected = Fp::new(av).mul(Fp::new(bv));
        let prod_2t =
            mul_shares_raw(&dealer.share(Fp::new(av)), &dealer.share(Fp::new(bv))).unwrap();
        let reduced = dealer.degree_reduce(&prod_2t).unwrap();
        assert_eq!(reduced.len(), 7);

        // First t+1 shares reconstruct the secret (use reconstruct_at_zero so we
        // interpolate EXACTLY t+1 points at degree t — no robustness fallback).
        let lo = reconstruct_at_zero(&reduced[..t + 1], t).unwrap();
        assert_eq!(lo, expected, "first t+1 reduced shares must give a·b");
        // A DIFFERENT, disjoint-ish t+1 subset agrees → consistent degree-t poly.
        let hi = reconstruct_at_zero(&reduced[reduced.len() - (t + 1)..], t).unwrap();
        assert_eq!(hi, expected, "last t+1 reduced shares must also give a·b");
        assert_eq!(lo, hi, "two t+1 subsets must agree (degree exactly t)");

        // Negative control: the ORIGINAL degree-2t product is NOT degree-t — any
        // t+1 of its shares interpolated AT DEGREE t give the WRONG value (it
        // takes 2t+1 of them). This pins that the reduction actually did work.
        let wrong = reconstruct_at_zero(&prod_2t[..t + 1], t).unwrap();
        assert_ne!(
            wrong, expected,
            "t+1 points of the degree-2t product must NOT give a·b (sanity: reduction was real)"
        );
    }

    #[test]
    fn share_private_input_from_holder_roundtrips() {
        // End-to-end: a holder's PRIVATE salary is shared (not disclosed), summed
        // with another holder's, and reconstructed to the plaintext total.
        const PFX: &str =
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";
        let alice = Holder::from_rdf(
            "alice",
            &format!("{PFX} ex:alice ex:salary \"30000\"^^xsd:integer ."),
            "turtle",
        )
        .unwrap();
        let bob = Holder::from_rdf(
            "bob",
            &format!("{PFX} ex:bob ex:salary \"45000\"^^xsd:integer ."),
            "turtle",
        )
        .unwrap();

        // Production backend (OS-seeded CSPRNG) — the round-trip is seed-agnostic.
        let backend = ShamirBackend::new(2).unwrap();
        let mut sa = backend.share_private_input(&alice).unwrap();
        let sb = backend.share_private_input(&bob).unwrap();
        sa.extend(sb);
        let summed = backend.run_secure(&sa).unwrap();
        let got = backend.reconstruct(&summed[0]).unwrap();
        assert_eq!(got, Fp::new(75_000));
    }

    /// [OPUS-4.8] sq-dwb5 — BACKWARD-COMPAT: the single-scalar `share_private_input`
    /// path is unchanged, AND the default `share_private_inputs` over the fixed
    /// single-salary fragment yields the SAME length-1 batch (the k=1 special case
    /// the generalisation must preserve).
    #[test]
    fn share_private_inputs_subsumes_single_scalar_path() {
        const PFX: &str =
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";
        let alice = Holder::from_rdf(
            "alice",
            &format!("{PFX} ex:alice ex:salary \"30000\"^^xsd:integer ."),
            "turtle",
        )
        .unwrap();
        let backend = ShamirBackend::new(2).unwrap();
        // The original single path still works.
        let single = backend.share_private_input(&alice).unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(backend.reconstruct(&single[0]).unwrap(), Fp::new(30_000));
        // The batched path over the SAME single-salary fragment yields one row.
        let batched = backend
            .share_private_inputs(&alice, PRIVATE_SALARY_FRAGMENT)
            .unwrap();
        assert_eq!(batched.len(), 1, "k=1 special case is a length-1 batch");
        assert_eq!(backend.reconstruct(&batched[0]).unwrap(), Fp::new(30_000));
    }

    /// [OPUS-4.8] sq-dwb5 — ROW-BINDING end-to-end: two holders each have a
    /// MULTI-ROW private salary column; the batched share + the multi-row secure
    /// aggregate (`batched::per_row_sum`) over their positional batches equals the
    /// plaintext per-row sum. The value-sort row-binding makes the two holders'
    /// rows line up by position regardless of local triple order. This is the
    /// multi-row path the single-scalar `share_private_input` could not express.
    #[test]
    fn batched_multi_row_per_holder_aggregate_matches_plaintext() {
        use crate::batched::{per_row_sum, BatchedShares, RowBinding};
        const PFX: &str =
            "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";
        // Each holder has a 3-row salary column (e.g. monthly payslips). The triples
        // are written in DIFFERENT orders per holder to exercise the value-sort.
        let alice = Holder::from_rdf(
            "alice",
            &format!(
                "{PFX} ex:a1 ex:salary \"3000\"^^xsd:integer . \
                 ex:a2 ex:salary \"1000\"^^xsd:integer . \
                 ex:a3 ex:salary \"2000\"^^xsd:integer ."
            ),
            "turtle",
        )
        .unwrap();
        let bob = Holder::from_rdf(
            "bob",
            &format!(
                "{PFX} ex:b1 ex:salary \"500\"^^xsd:integer . \
                 ex:b2 ex:salary \"1500\"^^xsd:integer . \
                 ex:b3 ex:salary \"2500\"^^xsd:integer ."
            ),
            "turtle",
        )
        .unwrap();

        let frag = PRIVATE_SALARY_FRAGMENT; // SELECT ?salary WHERE { ?p ex:salary ?salary }
        let backend = ShamirBackend::new_seeded(5, 0xBA7C).unwrap();
        let a = BatchedShares::new(
            backend.share_private_inputs(&alice, frag).unwrap(),
            RowBinding::Positional,
        )
        .unwrap();
        let b = BatchedShares::new(
            backend.share_private_inputs(&bob, frag).unwrap(),
            RowBinding::Positional,
        )
        .unwrap();
        assert_eq!(a.len(), 3);
        assert_eq!(b.len(), 3);

        let summed = per_row_sum(&[a, b]).unwrap();
        let got = summed.reconstruct(&backend).unwrap();
        // Value-sorted: alice = [1000,2000,3000], bob = [500,1500,2500] → per-row
        // sums [1500, 3500, 5500].
        assert_eq!(got, vec![Fp::new(1500), Fp::new(3500), Fp::new(5500)]);
    }

    /// [OPUS-4.8] sq-dwb5 — `extract_integer_vector` must fail CLOSED on a row
    /// whose arity desyncs from `vars`: a malformed/adversarial `PartialResult`
    /// can claim one column in `vars` yet carry rows with extra cells. We must
    /// reject (not silently use the first column and drop the rest).
    #[test]
    fn extract_integer_vector_rejects_multi_column_row() {
        use crate::partial::PartialResult;
        use oxrdf::{Literal, Term, Variable};
        let int = |n: u64| Some(Term::Literal(Literal::from(n as i64)));
        let p = PartialResult {
            holder: crate::partial::HolderId::new("mallory"),
            // `vars` claims a single column...
            vars: vec![Variable::new("salary").unwrap()],
            // ...but a row sneaks in a second cell.
            rows: vec![vec![int(1000)], vec![int(2000), int(9999)]],
        };
        let err = extract_integer_vector(&p).unwrap_err();
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("exactly one column")),
            "expected per-row arity rejection, got {err:?}"
        );

        // A well-formed single-column partial still extracts in row order.
        let ok = PartialResult {
            holder: crate::partial::HolderId::new("alice"),
            vars: vec![Variable::new("salary").unwrap()],
            rows: vec![vec![int(1000)], vec![int(2000)]],
        };
        assert_eq!(extract_integer_vector(&ok).unwrap(), vec![1000, 2000]);
    }

    // =====================================================================
    // [OPUS-4.8] sq-qcnn.26 — DIRECT mutation-directed tests for the Shamir
    // share/reconstruct algebra. VALUE assertions on the dealer accessors, the
    // fail-closed degree-reduce precondition, the per-operator security
    // boundary, and the secret-key Debug redaction — surface the round-trip
    // tests do not pin tightly enough for the mutation ratchet. Semi-honest
    // scope only; NO protocol logic changed, NO security claim (sq-qhy4).
    // =====================================================================

    #[test]
    fn dealer_parties_and_threshold_mirror_backend() {
        // A constant-return mutant on either accessor would desync it from the
        // backend silently; pin both to the exact honest-majority values.
        for n in [2usize, 3, 4, 5, 7] {
            let b = ShamirBackend::new_seeded(n, 0x1234 + n as u64).unwrap();
            let dealer = b.dealer();
            assert_eq!(dealer.parties(), n, "dealer.parties() must equal n");
            assert_eq!(
                dealer.threshold(),
                (n - 1) / 2,
                "dealer.threshold() = floor((n-1)/2)"
            );
            assert_eq!(dealer.threshold(), b.threshold());
        }
    }

    #[test]
    fn degree_reduce_deficient_backend_fails_closed_not_panics() {
        // A DEFICIENT config n < 2t+1 (unbuildable by the honest-majority
        // constructor) is what the `with_unchecked_threshold` escape hatch
        // exists to exercise: degree_reduce over the FULL n-party sharing must
        // return a descriptive MpcError::Protocol, NEVER panic / mis-reduce.
        // This pins the fail-closed `shares_2t.len() < 2*self.t + 1` precondition
        // in BOTH the comparison direction AND the `2*self.t + 1` arithmetic: a
        // flip that raises the guard's threshold or reverses it lets the
        // too-short sharing through to the `[..2*self.t + 1]` slice, which is
        // out of range on the deficient n-party vector → PANIC (a caught mutant)
        // rather than the original's clean Protocol. The spread of (n, t) puts n
        // at several offsets from 2t+1 so each arithmetic variant (2*t → 2/t,
        // 2+t; 2*t+1 → 2*t-1, 2*t*1) diverges on at least one config.
        for &(n, t) in &[(3usize, 2usize), (4, 2), (5, 3), (6, 3)] {
            let b = ShamirBackend::with_unchecked_threshold(n, t); // n < 2t+1
            let mut dealer = b.dealer();
            let shares = dealer.share(Fp::new(42)); // the full n-party sharing
            assert_eq!(shares.len(), n);
            let err = dealer.degree_reduce(&shares).unwrap_err();
            assert!(
                matches!(err, MpcError::Protocol(_)),
                "deficient n={n} < 2t+1 (t={t}) must fail closed with Protocol, got {err:?}"
            );
        }
    }

    #[test]
    fn equality_open_semi_honest_only_at_minimal_odd_n() {
        // The degree-2t equality / hidden-value open carries ZERO Reed–Solomon
        // redundancy at the minimal honest-majority n = 2t+1 (odd n), so its
        // per-operator security is SEMI-HONEST-ONLY there — the load-bearing
        // no-detection boundary (contrast the degree-t aggregate, which stays
        // robust). Pinned at n=5 (t=2) and n=7 (t=3): there 2*t differs from a
        // mutated 2+t / 2/t and yields a DIFFERENT descriptor, so the exact-value
        // assertion kills the arithmetic mutant on the degree-2t computation.
        for &(n, t) in &[(5usize, 2usize), (7, 3)] {
            let b = ShamirBackend::new(n).unwrap();
            assert_eq!(b.threshold(), t);
            let eq = b.operator_security(OperatorClass::EqualityJoin);
            assert_eq!(
                eq.malicious_security(0),
                MaliciousSecurity::SemiHonestOnly,
                "degree-2t equality open at n=2t+1={n} has zero RS redundancy → semi-honest-only"
            );
        }
        // Even n gains exactly one redundant share at degree 2t → detect-and-abort
        // (NOT semi-honest-only): a genuinely different descriptor at the same t.
        let b6 = ShamirBackend::new(6).unwrap(); // t=2, degree 2t=4, n=6 > 2t+1=5
        assert_eq!(
            b6.operator_security(OperatorClass::EqualityJoin)
                .malicious_security(0),
            MaliciousSecurity::HonestMajorityAbort,
            "n=6 degree-2t open has one redundant share → detect-and-abort"
        );
    }

    #[test]
    fn malicious_security_no_redundancy_branch_is_semi_honest_only() {
        // The `n <= t+1` NO-REDUNDANCY arm of malicious_security() returns
        // SemiHonestOnly (tampering is information-theoretically undetectable at
        // exactly t+1 shares). It is UNREACHABLE via the honest-majority
        // constructor (which fixes t=floor((n-1)/2), so every valid n has n>t+1),
        // hence untested end-to-end — the with_unchecked_threshold escape hatch
        // exercises it. Pinned at the boundary n == t+1, where the guard's `t+1`
        // arithmetic is load-bearing: a `t+1`->`t` (or `t-1`) mutation moves n out
        // of the no-redundancy arm and mis-reports HonestMajorityAbort instead.
        let b = ShamirBackend::with_unchecked_threshold(3, 2); // n = 3 = t+1
        assert_eq!(
            b.malicious_security(),
            MaliciousSecurity::SemiHonestOnly,
            "n == t+1 carries no RS redundancy → semi-honest-only (no detection claim)"
        );
        // One share more (n = t+2) DOES carry redundancy → detect-and-abort, so the
        // arm boundary is genuinely at t+1, not below it.
        let b2 = ShamirBackend::with_unchecked_threshold(4, 2); // n = 4 = t+2
        assert_eq!(
            b2.malicious_security(),
            MaliciousSecurity::HonestMajorityAbort,
            "n == t+2 has one redundant share → detect-and-abort"
        );
    }

    #[test]
    fn dealer_debug_labels_struct_and_redacts_rng() {
        // The ShamirDealer Debug must stay a non-empty, labelled struct that
        // redacts the live masking RNG (a fmt body replaced by an empty `Ok(())`
        // would drop the redaction and the label). The `contains` checks are
        // absent in an empty-output mutant, so this kills the fmt default-body.
        let b = ShamirBackend::new_seeded(5, 0x0DDD).unwrap();
        let dealer = b.dealer();
        let dbg = format!("{dealer:?}");
        assert!(
            dbg.contains("ShamirDealer"),
            "Debug must name the struct: {dbg}"
        );
        assert!(
            dbg.contains("<live masking RNG>"),
            "Debug must redact the live RNG state: {dbg}"
        );
    }

    #[test]
    fn mac_session_debug_labels_struct_and_redacts_alpha() {
        // The MacSession Debug must stay a non-empty, labelled struct that
        // REDACTS the cleartext session key α (a Debug body replaced by an empty
        // `Ok(())` would silently drop the redaction). The `contains` checks are
        // absent in an empty-output mutant, so this kills the fmt default-body.
        let b = ShamirBackend::new_seeded(5, 0xD00D).unwrap();
        let mut dealer = b.dealer();
        let session = dealer.new_mac_session();
        let dbg = format!("{session:?}");
        assert!(
            dbg.contains("MacSession"),
            "Debug must name the struct: {dbg}"
        );
        assert!(
            dbg.contains("<secret session MAC key"),
            "Debug must carry the α redaction label: {dbg}"
        );
        // The POINT of the redaction is that the cleartext α VALUE is absent, not merely
        // that a label is present: a future Debug impl could print α *and* the label and
        // still pass a label-only check. Capture α's own Debug rendering and assert it is
        // NOT a substring of the session Debug output, so the redaction property is pinned.
        let alpha_dbg = format!("{:?}", session.alpha_for_test());
        assert!(
            !dbg.contains(&alpha_dbg),
            "Debug must NOT leak the cleartext α value {}: {}",
            alpha_dbg,
            dbg
        );
    }

    #[test]
    fn extract_single_integer_shape_and_value_guards() {
        use crate::partial::{HolderId, PartialResult};
        use oxrdf::{Literal, Term, Variable};
        let int = |n: u64| Some(Term::Literal(Literal::from(n as i64)));
        let one_var = || vec![Variable::new("salary").unwrap()];
        let mk = |vars: Vec<Variable>, rows: Vec<Vec<Option<Term>>>| PartialResult {
            holder: HolderId::new("h"),
            vars,
            rows,
        };

        // Happy path: exactly one row, one column, one integer literal → its value.
        // (Pins a non-{0,1} value so a constant-return mutant is caught.)
        assert_eq!(
            extract_single_integer(&mk(one_var(), vec![vec![int(30_000)]])).unwrap(),
            30_000
        );

        // Zero rows and many rows are BOTH protocol errors (the `rows.len() != 1`
        // guard — a `!=`→`==` flip would reject the valid single-row case above).
        assert!(matches!(
            extract_single_integer(&mk(one_var(), vec![])),
            Err(MpcError::Protocol(_))
        ));
        assert!(matches!(
            extract_single_integer(&mk(one_var(), vec![vec![int(1)], vec![int(2)]])),
            Err(MpcError::Protocol(_))
        ));

        // Multi-COLUMN via `vars.len() != 1` alone (single well-formed row) → error.
        assert!(matches!(
            extract_single_integer(&mk(
                vec![Variable::new("a").unwrap(), Variable::new("b").unwrap()],
                vec![vec![int(5), int(6)]]
            )),
            Err(MpcError::Protocol(_))
        ));

        // Multi-COLUMN via the ROW arity alone: vars claims ONE column but the row
        // carries two cells. This is the case the `||` guard (`vars.len() != 1 ||
        // rows[0].len() != 1`) must reject on the RIGHT operand — an `||`→`&&`
        // flip would accept it and silently use only the first cell.
        assert!(matches!(
            extract_single_integer(&mk(one_var(), vec![vec![int(5), int(6)]])),
            Err(MpcError::Protocol(_))
        ));

        // A non-integer literal in the single cell → error, never a guessed value.
        assert!(matches!(
            extract_single_integer(&mk(
                one_var(),
                vec![vec![Some(Term::Literal(Literal::new_simple_literal("hi")))]]
            )),
            Err(MpcError::Protocol(_))
        ));
    }

    // ---- sq-km34.4: the batched MAC-check at open (design §2.5) ----------------
    //
    // The acceptance for this bead is twofold and both halves are MEASURED here:
    //   (a) a SINGLE batched check amortises `|L|·|R|` equality opens — pinned by the
    //       σ-open counter, not by prose;
    //   (b) a tamper anywhere in that batch ABORTS, and aborts *before* any opened
    //       value reaches the caller.
    // Plus the binding property: the challenges are derived by Fiat–Shamir from the
    // already-fixed shares. (These are centralized-simulation tests — one MacSession
    // holds every share — so they say nothing about a distributed public coin.)

    /// Build the `|L|·|R|` authenticated masked products an all-pairs hidden-value join
    /// opens — `[[m_ij]] = [[l_i − r_j]] · [[mask_ij]]`, whose opened `m_ij` is `0`
    /// exactly when the keys match. This is the batch §2.5 has to authenticate in ONE
    /// check; returns the authenticated products alongside the expected match graph.
    fn all_pairs_equality_batch(
        session: &mut MacSession,
        left: &[u64],
        right: &[u64],
    ) -> (Vec<crate::authenticated::AuthenticatedShare>, Vec<bool>) {
        let mut products = Vec::with_capacity(left.len() * right.len());
        let mut expected_match = Vec::with_capacity(left.len() * right.len());
        for &l in left {
            for &r in right {
                let auth_l = session.authenticated_share(Fp::new(l));
                let auth_r = session.authenticated_share(Fp::new(r));
                let diff = crate::authenticated::auth_sub(&auth_l, &auth_r).unwrap();
                let mask_value = session.draw_nonzero_fp();
                let auth_mask = session.authenticated_share(mask_value);
                products.push(session.auth_mul(&diff, &auth_mask).unwrap());
                expected_match.push(l == r);
            }
        }
        (products, expected_match)
    }

    /// Corrupt the VALUE sharing into a CONSISTENT degree-`t` sharing of a different
    /// value (shift every share's `y` by δ) while leaving the MAC untouched — the
    /// Hole-2 deviation Reed–Solomon cannot see, which only the IT-MAC catches.
    fn tamper_value(
        av: &crate::authenticated::AuthenticatedShare,
        delta: u64,
    ) -> crate::authenticated::AuthenticatedShare {
        let shifted: Vec<Share> = av
            .value_shares()
            .iter()
            .map(|s| Share {
                x: s.x,
                y: s.y.add(Fp::new(delta)),
            })
            .collect();
        crate::authenticated::AuthenticatedShare::new(shifted, av.mac_shares().to_vec()).unwrap()
    }

    /// **Acceptance (a).** One batched check authenticates all `|L|·|R|` equality opens
    /// with ONE `σ` open, and returns every opened value correctly — while checking the
    /// same values one-at-a-time costs `|L|·|R|` σ opens. That ratio IS the design's
    /// "output: +1 batched check, `O(1)` per opened batch, not per opened value".
    #[test]
    fn batched_mac_check_amortises_all_pairs_equality_opens() {
        let backend = ShamirBackend::new_seeded(5, 0x5121).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();

        // |L| = 6, |R| = 7 ⇒ 42 equality opens; rows 3 and 5 of L match rows in R.
        let left = [10u64, 20, 30, 40, 50, 60];
        let right = [30u64, 31, 32, 33, 34, 35, 50];
        let (products, expected_match) = all_pairs_equality_batch(&mut session, &left, &right);
        assert_eq!(products.len(), left.len() * right.len());
        assert_eq!(session.sigma_opens_for_test(), 0, "no check run yet");

        // ONE call over the WHOLE batch.
        let opened = session.mac_check_and_open(&products).unwrap();
        assert_eq!(
            session.sigma_opens_for_test(),
            1,
            "the whole |L|·|R| batch must share ONE σ open — that is the §2.5/§5 \
             amortisation this bead delivers"
        );

        // The check hands back exactly the values it authenticated, and they carry the
        // real match graph (m_ij == 0 ⇔ l_i == r_j), so the batch was not vacuous.
        assert_eq!(opened.len(), products.len());
        for (idx, (m, &matches)) in opened.iter().zip(expected_match.iter()).enumerate() {
            assert_eq!(
                *m == Fp::zero(),
                matches,
                "pair {idx}: opened masked product must be zero exactly on a key match"
            );
            assert_eq!(
                *m,
                backend.reconstruct(products[idx].value_shares()).unwrap(),
                "pair {idx}: the returned value must be the opened value sharing"
            );
        }

        // The un-amortised alternative: one check per value costs one σ open EACH.
        for p in &products {
            session.mac_check(std::slice::from_ref(p)).unwrap();
        }
        assert_eq!(
            session.sigma_opens_for_test(),
            1 + products.len() as u64,
            "checking the same values one-at-a-time spends |L|·|R| σ opens — the cost \
             the single batched check replaces"
        );
    }

    /// **Acceptance (b).** A tamper on ANY single pair of the `|L|·|R|` batch aborts the
    /// one batched check, and the caller receives NO opened value — the abort happens
    /// before the inconsistent result can be acted on (coZK-2025/1026 discipline).
    #[test]
    fn tamper_anywhere_in_the_batch_aborts_before_any_value_is_returned() {
        let left = [7u64, 8, 9, 10];
        let right = [9u64, 11, 12, 13, 14];
        // Tamper the first, a middle, and the last pair in turn.
        for bad_index in [0usize, 9, left.len() * right.len() - 1] {
            let backend = ShamirBackend::new_seeded(5, 0x7000 + bad_index as u64).unwrap();
            let mut dealer = backend.dealer();
            let mut session = dealer.new_mac_session();
            let (mut products, _) = all_pairs_equality_batch(&mut session, &left, &right);

            // Honest batch passes and yields every value.
            let clean = session.mac_check_and_open(&products).unwrap();
            assert_eq!(clean.len(), products.len());

            products[bad_index] = tamper_value(&products[bad_index], 12345);
            let err = session
                .mac_check_and_open(&products)
                .expect_err("a tampered value in the batch must abort the batched check");
            assert!(
                matches!(err, MpcError::MacCheckFailed { .. }),
                "expected MacCheckFailed at batch index {bad_index}, got {err:?}"
            );
        }
    }

    /// The challenge is **Fiat–Shamir over the fixed shares**: deterministic given the
    /// batch, and re-randomised by any tamper (so the challenge cannot precede the shares
    /// it authenticates). That BINDING is all this pins — it is derived from the whole
    /// share vectors, which only the trusted-dealer simulation holds, so it deliberately
    /// asserts NOTHING about `n` real parties agreeing on `χ`; that would need the
    /// commit-then-challenge protocol `mac_check_challenges` scopes out.
    #[test]
    fn mac_check_challenges_are_bound_to_the_fixed_shares() {
        let backend = ShamirBackend::new_seeded(5, 0xC0DE).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let batch: Vec<_> = [3u64, 5, 8]
            .iter()
            .map(|&v| session.authenticated_share(Fp::new(v)))
            .collect();

        let chis = mac_check_challenges(5, 2, &batch);
        assert_eq!(chis.len(), batch.len(), "one challenge per value");
        assert_eq!(
            chis,
            mac_check_challenges(5, 2, &batch),
            "the coin must be a deterministic function of the fixed shares"
        );

        // Any tamper re-randomises the WHOLE challenge vector — an adversary cannot
        // hold the other coefficients fixed while searching for a favourable one.
        let mut tampered = batch.clone();
        tampered[1] = tamper_value(&tampered[1], 1);
        let chis_tampered = mac_check_challenges(5, 2, &tampered);
        assert_ne!(
            chis, chis_tampered,
            "changing a share must change the Fiat–Shamir challenges"
        );
        assert_ne!(
            chis[0], chis_tampered[0],
            "the challenges are derived from a root over the whole batch, so even an \
             untouched value's coefficient moves"
        );

        // Domain/parameter separation: the same shares under different (n, t) give a
        // different coin, so a transcript cannot be replayed across parameterisations.
        assert_ne!(chis, mac_check_challenges(5, 1, &batch));
        assert_ne!(chis, mac_check_challenges(7, 2, &batch));
    }

    /// **The random challenge has to be REAL.** Two tampers that cancel — pair `i`
    /// shifted by `+δ` and pair `j` by `−δ` — contribute `−α·(χ_i·δ − χ_j·δ)` to `σ`.
    /// Under an unweighted sum (`χ ≡ 1`) that is identically `0` and the batched check
    /// is FOOLED; under the §2.5 random challenges it is `−α·δ·(χ_i − χ_j) ≠ 0` except
    /// with probability `≈ 1/p`. This is the test that pins `mac_check_and_open` to the
    /// actual [`mac_check_challenges`] coin rather than to any constant weighting.
    #[test]
    fn cancelling_tampers_are_caught_because_the_challenges_are_random() {
        let left = [4u64, 5, 6];
        let right = [6u64, 7, 8, 9];
        for delta in [1u64, 999, 1 << 40] {
            let backend = ShamirBackend::new_seeded(5, 0xCA11 + delta).unwrap();
            let mut dealer = backend.dealer();
            let mut session = dealer.new_mac_session();
            let (mut products, _) = all_pairs_equality_batch(&mut session, &left, &right);

            // Equal-and-opposite value shifts on two different pairs: their MACs are
            // untouched, so the per-value MAC deficits are exactly +δ·α and −δ·α, which
            // an unweighted check would sum to zero.
            products[2] = tamper_value(&products[2], delta);
            products[7] = tamper_value(&products[7], crate::field::P - delta);

            let err = session
                .mac_check_and_open(&products)
                .expect_err("cancelling tampers must still abort under random challenges");
            assert!(
                matches!(err, MpcError::MacCheckFailed { .. }),
                "expected MacCheckFailed for δ = {delta}, got {err:?}"
            );
        }
    }

    /// The empty batch opens nothing and spends no `σ` open — checking "nothing about
    /// to be opened" must not cost a round, and must not fail closed either.
    #[test]
    fn empty_batch_opens_nothing_and_spends_no_sigma_open() {
        let backend = ShamirBackend::new_seeded(3, 0x0E0E).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        assert!(session.mac_check_and_open(&[]).unwrap().is_empty());
        assert_eq!(session.sigma_opens_for_test(), 0);
    }
}
