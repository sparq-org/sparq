// [OPUS-5] sq-yyro follow-on (issue #3531): the PRSS correlated-randomness
// SOURCE behind `crate::randomness::DistributedRandomness` — replicated-PRF seed
// setup + the `Σ_A PRF(k_A, ctr)·f_A` degree-`t` generator. Small-`n`
// honest-majority only; fails closed where the `C(n, t)` seed count is
// impractical. Design: research/mpc-distributed-randomness-design.md §2.1 / §5.1.
//! **PRSS** — pseudo-random secret sharing from replicated PRF seeds.
//!
//! The first dealer-less generator behind the [`crate::randomness`] seam: it
//! produces a fresh degree-`t` sharing of a value that **no `≤ t` parties can
//! compute**, with **zero online communication**, from a one-time replicated-seed
//! setup. It is the design record's *primary* choice for the small-`n`
//! honest-majority deployment target (the four flatmates) —
//! `research/mpc-distributed-randomness-design.md` §2.1 / §2.3 / §5 item 1.
//!
//! ## The construction (Cramer–Damgård–Ishai TCC'05; eprint 2021/1223)
//!
//! Parties sit on the canonical Shamir points `x = 1..=n`; the honest-majority
//! threshold is `t = ⌊(n−1)/2⌋`, matching [`crate::shamir::ShamirBackend`].
//!
//! 1. **Setup (one time).** For every *maximal unqualified set* `A ⊆ {1..n}` with
//!    `|A| = t`, a PRF seed `k_A` is distributed to **exactly the parties NOT in
//!    `A`**. There are `C(n, t)` such sets, so each party holds `C(n−1, t)` seeds.
//! 2. **Generation (per element, non-interactive).** For counter `ctr`, party `i`
//!    locally computes
//!
//!    ```text
//!    share_i  =  Σ_{A : i ∉ A}  PRF(k_A, ctr) · f_A(x_i)
//!    ```
//!
//!    where `f_A` is the **public** degree-`t` polynomial that is `1` at `0` and
//!    `0` at every point in `A`:  `f_A(z) = Π_{j∈A} (x_j − z)/x_j`.
//!
//! Because `f_A(x_i) = 0` exactly when `i ∈ A`, the term a party *cannot* compute
//! (it does not hold `k_A`) is precisely the term that *vanishes* from its share —
//! so the sum over "seeds I hold" and the sum over "all seeds" agree. The result
//! is the evaluation at `x_i` of the single degree-`≤ t` polynomial
//! `F(z) = Σ_A PRF(k_A, ctr)·f_A(z)`, i.e. a well-formed degree-`t` sharing of
//!
//! ```text
//! r  =  F(0)  =  Σ_A PRF(k_A, ctr)
//! ```
//!
//! **Why no `≤ t` parties know `r`.** Take any set `T` of `t` parties. `T` is
//! itself one of the maximal unqualified sets, and `k_T` is held by exactly the
//! parties *outside* `T` — so no member of `T` can evaluate `PRF(k_T, ctr)`, which
//! is one independent (pseudo)uniform summand of `r`. Their own shares carry no
//! information about it either, since `f_T(x_i) = 0` for every `i ∈ T`. Hence `r`
//! is pseudorandom given the whole view of `T`. This is the standard PRSS
//! argument; it is **computational** (it rests on the PRF), a step down from the
//! information-theoretic Shamir core — an honest caveat the design record states
//! (§2.1 cons 2), and the same tier the ChaCha20 masking RNG already sits at
//! (sq-1vt).
//!
//! ## What this module does and does NOT establish (read before claiming anything)
//!
//! - **Online generation is genuinely non-interactive and structurally
//!   seed-local.** Each party's share is produced by a *party-local* generator
//!   (`PartyView::share_at`) over a view that **carries only that party's own
//!   key material** — the key of a seed the party does not hold is simply absent
//!   from the structure, so that generator has nothing to evaluate, and the
//!   access instrumentation fires **at** each `PRF(k_A, ctr)` call site rather
//!   than after a global evaluation. `shared_mask` runs exactly these `n`
//!   party-local generators and evaluates no other key. The test
//!   `no_share_uses_a_seed_its_party_does_not_hold` records that access set,
//!   requires it to be exactly the complement of `A`, and cross-checks the
//!   resulting shares against the global `Σ_A PRF(k_A, ctr)·f_A` polynomial —
//!   evaluated **independently**, never fed into the shares. Its non-vacuity is
//!   shown by splicing a non-held seed into one party view with its *real*
//!   coefficient and re-running the same routine: the emitted number is
//!   unchanged (that coefficient is `0`), and only the access instrumentation
//!   goes red. So locality is a *checked* property of the real generator, not a
//!   comment — though see the in-process caveat below: what is checked is which
//!   key material a share's generator can reach, never process isolation.
//! - **The SETUP here is a SIMULATED one-time trusted setup, NOT dealer-less
//!   key agreement.** [`PrssRandomness::with_simulated_setup`] draws every `k_A`
//!   locally from OS entropy, standing in for the real replicated-seed
//!   distribution the design record leaves as an open question (§6 Q3). A
//!   deployment needs that distribution to be dealer-less (or an accepted one-time
//!   trusted setup); this module does not provide it.
//! - **The crate is an in-process SIMULATION.** All `n` shares are computed in one
//!   process, which therefore holds every seed. What is demonstrated is the
//!   *structure* of the protocol (which seed may enter which share, and that the
//!   result is a correct degree-`t` sharing), never process/network isolation.
//! - **Honest-majority, semi-honest only.** With an honest setup a party cannot
//!   *bias* `r` (it is fixed by the seeds), but nothing here detects a party that
//!   emits a wrong share, and the `r ≠ 0` guarantee below leans on a simulation
//!   artefact. No malicious-tier property is claimed.
//! - **Not deployable, and this module does not change that.**
//!   [`RandomnessModel::Prss`] still reports
//!   [`deployable() == false`](RandomnessModel::deployable), so
//!   [`DistributedRandomness::require_deployable`] still refuses this source
//!   fail-closed. Acceptance must be tied to a *validated* construction behind the
//!   boundary of design-record §5 item 5, and the crate remains research-grade and
//!   **externally unaudited (`sq-qhy4` pending)**.
//!
//! ## `r ≠ 0` for the equality mask — the honest gap
//!
//! [`DistributedRandomness::shared_nonzero_mask`]'s structural contract is
//! `r ≠ 0` (a forced `r = 0` makes `secure_equal` report "equal" for every pair —
//! see the [`crate::randomness`] module note). PRSS gives a uniform `r`, so
//! `r = 0` happens with probability `1/p ≈ 2^−61`; the honest fix is to detect it
//! and redraw at a fresh counter. But **no party can evaluate `r`** (that is the
//! whole point), so a real deployment must run a *distributed zero-test* on `[r]`
//! — design record §1 part 1 / §5 item 4, still a follow-on bead. This module
//! therefore does the rejection **centrally**, using the simulation's access to
//! every seed. That check is a **SIMULATION ARTEFACT**: it is exactly the step
//! that does not carry over to a real federation, and it is why
//! `shared_nonzero_mask` stays semi-honest-only here.
//!
//! ## Small-`n` only — the seed-count guard (fails closed)
//!
//! The setup cost is `C(n, t)` seeds, which blows up combinatorially: `n = 4` → 4
//! seeds, `n = 9` → 126, `n = 10` → 210, `n = 11` → 462. PRSS is the right tool
//! only for small `n` (design §2.1 cons 1, §6 Q1). [`MAX_PRSS_SEEDS`] caps the
//! total; past it the constructor **refuses** with [`MpcError::Protocol`] naming
//! the distributed-coin-toss fallback (design §2.2 / §5 item 2) rather than
//! silently building an impractical setup.

use crate::chacha::ChaCha20Csprng;
use crate::field::Fp;
use crate::partial::MpcError;
use crate::randomness::{DistributedRandomness, RandomnessModel};
use crate::rng::MpcRng;
use crate::shamir::Share;
use sha2::{Digest, Sha512};
use zeroize::{Zeroize, Zeroizing};

/// Domain-separation tag for the PRSS PRF key derivation. Versioned so a future
/// PRF change is a distinct, non-colliding derivation rather than a silent
/// re-interpretation of the same seeds.
const PRF_DOMAIN: &[u8] = b"sparq-mpc/prss/v1/prf";

/// The maximum number of replicated seeds (`C(n, t)`) a [`PrssRandomness`] setup
/// will build before refusing fail-closed.
///
/// `128` admits every honest-majority `n ≤ 9` (`C(9, 4) = 126` — the design
/// record's own "borderline" point, §6 Q1) and refuses `n ≥ 10`
/// (`C(10, 4) = 210`). Past this ceiling the correct tool is the distributed
/// **coin-toss** (design §2.2: no combinatorial setup, `O(n)` per party, at the
/// cost of one interactive round per batch), which is a separate follow-on bead —
/// so this constructor refuses rather than build a setup nobody can run.
pub const MAX_PRSS_SEEDS: usize = 128;

/// One replicated PRF seed, together with the maximal unqualified set `A` that
/// defines who does *not* hold it and the precomputed public evaluations of `f_A`.
struct PrssSeed {
    /// The maximal unqualified set `A` (party points, ascending, `|A| = t`). The
    /// seed is held by exactly the parties **not** listed here.
    set: Vec<u64>,
    /// `f_A(x_i)` for `i = 1..=n`, indexed by `i − 1`. **Public** (it depends only
    /// on the party points), and `0` exactly at the parties in [`Self::set`] — the
    /// structural reason a party never needs a seed it does not hold.
    f_at: Vec<Fp>,
    /// The seed `k_A`. Secret: it (with the counter) determines one summand of
    /// every generated value. Held in a [`Zeroizing`] buffer so it is scrubbed
    /// when the setup is dropped.
    key: Zeroizing<[u8; 32]>,
}

/// A **PRSS** correlated-randomness source: replicated-PRF seeds plus the
/// `Σ_A PRF(k_A, ctr)·f_A` degree-`t` generator, reporting
/// [`RandomnessModel::Prss`].
///
/// Generation is **non-interactive** — a mask costs `C(n, t)` local PRF
/// evaluations and no rounds. See the module docs for the construction, the
/// secrecy argument, and — importantly — the three things this does NOT establish
/// (dealer-less *setup*, process isolation, any malicious-tier property). This
/// source is still refused by
/// [`require_deployable`](DistributedRandomness::require_deployable): the crate is
/// research-grade and externally unaudited (`sq-qhy4` pending).
///
/// Not `Clone`: cloning would duplicate the counter and hand out the *same* mask
/// twice under two apparently-independent sources.
pub struct PrssRandomness {
    n: usize,
    t: usize,
    /// One entry per maximal unqualified set, in lexicographic order of `set`.
    seeds: Vec<PrssSeed>,
    /// The PRF input for the NEXT generated element. Strictly monotone: a reused
    /// counter would reproduce a previously-issued mask, so it is never rewound
    /// and never wrapped (see [`Self::next_counter`]).
    counter: u64,
}

impl std::fmt::Debug for PrssRandomness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print seed material.
        f.debug_struct("PrssRandomness")
            .field("n", &self.n)
            .field("t", &self.t)
            .field("seeds", &format_args!("{} × <replicated seed>", self.seeds.len()))
            .field("counter", &self.counter)
            .finish()
    }
}

impl PrssRandomness {
    /// Build a PRSS source for `n` parties with a **SIMULATED one-time setup**:
    /// the `C(n, t)` replicated seeds are drawn here, locally, from OS entropy.
    ///
    /// The name is the honesty: a real federation must obtain those seeds from a
    /// dealer-less key-agreement (or an explicitly accepted one-time trusted
    /// setup) — the design record's open question §6 Q3 — and this constructor is
    /// **not** that. What it does give is the real *online* construction: from
    /// here on every generated sharing is produced by the genuine
    /// `Σ_A PRF(k_A, ctr)·f_A` rule with no communication and with each party's
    /// share touching only the seeds that party holds.
    ///
    /// The threshold is the honest-majority `t = ⌊(n−1)/2⌋`, identical to
    /// [`crate::shamir::ShamirBackend`], so the sharings this produces reconstruct
    /// through that backend unchanged.
    ///
    /// # Errors
    ///
    /// - [`MpcError::Protocol`] if `n < 2` (matching `ShamirBackend::new`).
    /// - [`MpcError::Protocol`] if the setup would need more than
    ///   [`MAX_PRSS_SEEDS`] seeds — PRSS is a small-`n` tool; the refusal names
    ///   the distributed-coin-toss fallback instead of building an impractical
    ///   setup.
    ///
    /// Note the degenerate `n = 2` case: `t = 0`, so the single unqualified set is
    /// empty, every party holds the one seed, and every share equals `r`. That is
    /// the correct `t = 0` behaviour (a threshold of `0` promises secrecy against
    /// no one) and mirrors `ShamirDealer::share` at `t = 0`; it is not a privacy
    /// regime anyone should deploy.
    pub fn with_simulated_setup(n: usize) -> Result<Self, MpcError> {
        Self::from_setup_rng(n, &mut crate::rng::SecureRng::from_os())
    }

    /// Build a PRSS source whose **setup seeds** come from the deterministic,
    /// seedable test PRNG — **for reproducible tests / benchmarks ONLY**, gated
    /// behind `#[cfg(any(test, feature = "insecure-test-rng"))]` exactly like
    /// `ShamirBackend::new_seeded`. The replicated seeds it produces are
    /// predictable, so every secrecy property of the module docs is void; the
    /// online generator itself is unchanged.
    #[cfg(any(test, feature = "insecure-test-rng"))]
    pub fn with_seeded_setup(n: usize, seed: u64) -> Result<Self, MpcError> {
        Self::from_setup_rng(n, &mut crate::rng::InsecureTestRng::new(seed))
    }

    /// Shared setup: validate `(n, t)`, enumerate the maximal unqualified sets,
    /// precompute the public `f_A` evaluations, and draw one seed per set from
    /// `setup_rng`.
    fn from_setup_rng(n: usize, setup_rng: &mut dyn MpcRng) -> Result<Self, MpcError> {
        if n < 2 {
            return Err(MpcError::Protocol(
                "PRSS needs n >= 2 parties (honest-majority Shamir party set)".into(),
            ));
        }
        let t = (n - 1) / 2;
        let count = seed_count(n, t).ok_or_else(|| {
            MpcError::Protocol(format!(
                "PRSS setup for n = {n} (t = {t}) needs C({n}, {t}) > {MAX_PRSS_SEEDS} replicated \
                 seeds — impractical. PRSS is the SMALL-n tool; use the distributed coin-toss \
                 fallback for larger federations (research/mpc-distributed-randomness-design.md \
                 §2.2 / §5 item 2)"
            ))
        })?;

        let sets = unqualified_sets(n, t);
        debug_assert_eq!(sets.len(), count, "enumeration must match C(n, t)");

        let seeds = sets
            .into_iter()
            .map(|set| {
                // f_A(x_i) for every party point — public, and zero exactly on `A`.
                let f_at = (1..=n as u64)
                    .map(|x| unqualified_poly_at(&set, Fp::new(x)))
                    .collect();
                let mut key = Zeroizing::new([0u8; 32]);
                for chunk in key.chunks_exact_mut(8) {
                    chunk.copy_from_slice(&setup_rng.next_u64().to_le_bytes());
                }
                PrssSeed { set, f_at, key }
            })
            .collect();

        Ok(PrssRandomness {
            n,
            t,
            seeds,
            counter: 0,
        })
    }

    /// Number of parties `n`.
    pub fn parties(&self) -> usize {
        self.n
    }

    /// The honest-majority privacy threshold `t = ⌊(n−1)/2⌋`.
    pub fn threshold(&self) -> usize {
        self.t
    }

    /// Total replicated seeds in the setup, `C(n, t)` — bounded by
    /// [`MAX_PRSS_SEEDS`]. This is the setup cost the small-`n` restriction is
    /// about, and also the number of PRF evaluations one generated element costs.
    pub fn seed_count(&self) -> usize {
        self.seeds.len()
    }

    /// How many of those seeds any single party holds, `C(n−1, t)` — the per-party
    /// storage figure (every party holds the seed of every set it is not in, and
    /// by symmetry that count is the same for all parties).
    pub fn seeds_per_party(&self) -> usize {
        self.view_of(1).count()
    }

    /// The indices into [`Self::seeds`] of the seeds party `party` holds — the
    /// index-level description of that party's cryptographic view of the setup.
    ///
    /// A seed `k_A` is held by exactly the parties **not** in `A`, so this is the
    /// complement of the sets containing `party`. This returns *indices*, not key
    /// material; [`Self::party_view`] is the structure generation actually runs
    /// on, and it carries only the keys named here.
    fn view_of(&self, party: u64) -> impl Iterator<Item = usize> + '_ {
        self.seeds
            .iter()
            .enumerate()
            .filter(move |(_, seed)| !seed.set.contains(&party))
            .map(|(i, _)| i)
    }

    /// Party `party`'s **entire** cryptographic view of the setup, as the
    /// party-local generator sees it: the key material of exactly the seeds it
    /// holds, each already paired with the public coefficient `f_A(x_party)`.
    ///
    /// This is the boundary that makes seed-locality structural rather than
    /// disciplinary — a key the party does not hold is *absent* from the returned
    /// [`PartyView`], so [`PartyView::share_at`] cannot evaluate one. In a real
    /// federation this is the whole of what party `party` would have been given.
    fn party_view(&self, party: u64) -> PartyView<'_> {
        PartyView {
            party,
            held: self
                .view_of(party)
                .map(|idx| HeldSeed {
                    idx,
                    key: &self.seeds[idx].key,
                    coeff: self.seeds[idx].f_at[party as usize - 1],
                })
                .collect(),
        }
    }

    /// Reserve the next PRF counter, refusing fail-closed at exhaustion.
    ///
    /// Counters MUST be strictly monotone: repeating one reproduces a
    /// previously-issued mask exactly (every seed is fixed), which would silently
    /// reuse a blinding factor. `u64` exhaustion is unreachable in practice
    /// (`2^64` elements), but wrapping would be a silent soundness break, so it is
    /// an error instead.
    fn next_counter(&mut self) -> Result<u64, MpcError> {
        let advanced = self.counter.checked_add(1).ok_or_else(|| {
            MpcError::Protocol(
                "PRSS counter exhausted (2^64 elements) — refusing to wrap, since a reused \
                 counter would re-issue a previously-generated mask. Re-run the setup with \
                 fresh replicated seeds."
                    .into(),
            )
        })?;
        let ctr = self.counter;
        self.counter = advanced;
        Ok(ctr)
    }

    /// The full degree-`t` sharing at PRF counter `ctr`: `n` **independent
    /// party-local** runs of [`PartyView::share_at`], one per party.
    ///
    /// This is the whole of the online protocol. No seed is evaluated outside the
    /// view of a party that holds it, because each run can only reach the keys in
    /// its own view.
    ///
    /// The simulation therefore pays what the `n` parties collectively pay —
    /// `n · C(n−1, t)` PRF evaluations, since every seed is re-derived by each of
    /// its holders — rather than the `C(n, t)` a central evaluator would. That is
    /// the cost of the party-local boundary being real instead of asserted, and
    /// it is per-party `C(n−1, t)` for anyone actually running the protocol.
    fn shares_at(&self, ctr: u64) -> Vec<Share> {
        self.shares_at_observed(ctr, |_, _| {})
    }

    /// [`Self::shares_at`] with an instrumentation hook threaded into the
    /// party-local generators: `observe(party, seed_idx)` fires at the
    /// `PRF(k_A, ctr)` call site, for **every key party `party` actually
    /// evaluates**.
    ///
    /// This is what makes seed-locality a checked property of the *generator*
    /// rather than of a re-derivation of the coefficients:
    /// `no_share_uses_a_seed_its_party_does_not_hold` records the access set on
    /// this exact path and requires it to equal that party's view — so a
    /// generator that reached for a key its party does not hold goes red even
    /// though the arithmetic would still come out right (`f_A(x_i) = 0` there).
    fn shares_at_observed<F: FnMut(u64, usize)>(&self, ctr: u64, mut observe: F) -> Vec<Share> {
        (1..=self.n as u64)
            .map(|party| self.party_view(party).share_at(ctr, &mut observe))
            .collect()
    }

    /// **SIMULATION-ONLY oracle:** the shared value `r = F(0) = Σ_A PRF(k_A, ctr)`.
    ///
    /// This is exactly what no `≤ t` parties can compute; it is available here
    /// only because this in-process simulation holds every seed. It is computed
    /// **independently of the shares** — over `self.seeds` directly, feeding
    /// nothing into any [`PartyView`] — so it is never evidence about the
    /// party-local path. It backs the `r ≠ 0` rejection (see the module note on
    /// that gap) and the tests, and is deliberately not reachable from the public
    /// API.
    fn central_value_at(&self, ctr: u64) -> Fp {
        self.seeds
            .iter()
            .fold(Fp::zero(), |acc, seed| acc.add(prf_fp(&seed.key, ctr)))
    }

    /// The sharing at `ctr` together with the simulation-only oracle for its
    /// value. The two halves are computed by separate, independent passes —
    /// [`Self::shares_at`] over the per-party views, [`Self::central_value_at`]
    /// over the whole setup.
    fn sharing_at(&self, ctr: u64) -> (Fp, Vec<Share>) {
        (self.central_value_at(ctr), self.shares_at(ctr))
    }
}

/// One entry of a [`PartyView`]: a key the party **holds**, with the public
/// coefficient `f_A(x_party)` that key's PRF output is scaled by.
struct HeldSeed<'a> {
    /// Index into [`PrssRandomness::seeds`]. A label for instrumentation and
    /// tests only — nothing in generation dereferences it.
    idx: usize,
    /// `k_A`, borrowed from the setup. Only held keys are ever borrowed into a
    /// view, which is what makes a non-held evaluation unrepresentable.
    key: &'a [u8; 32],
    /// `f_A(x_party)`, public and (for a held seed) nonzero.
    coeff: Fp,
}

/// A single party's key material — the **only** thing that party's share
/// generator can reach.
///
/// In a real federation this is what the one-time replicated-seed setup would
/// have delivered to that party and nothing more; here it is a borrowed
/// projection of the in-process setup, assembled by
/// [`PrssRandomness::party_view`]. The
/// distinction the module docs draw between the party-local path and the
/// simulation-only oracle is this type: [`Self::share_at`] has no access to
/// `PrssRandomness`, so it cannot evaluate a key its party does not hold.
struct PartyView<'a> {
    /// The Shamir point `x_i` this view belongs to.
    party: u64,
    /// Exactly the seeds this party holds — the complement of the unqualified
    /// sets containing [`Self::party`].
    held: Vec<HeldSeed<'a>>,
}

impl PartyView<'_> {
    /// The party's share at counter `ctr`:
    /// `Σ_{A : i ∉ A} PRF(k_A, ctr) · f_A(x_i)`, over **this view alone**.
    ///
    /// Non-interactive: `C(n−1, t)` local PRF evaluations, no rounds, no access
    /// to any other party's state. `observe(party, seed_idx)` fires immediately
    /// before each PRF evaluation, so the recorded set is the set of keys this
    /// generator genuinely used — not a set of indices into values something else
    /// already computed.
    fn share_at(&self, ctr: u64, observe: &mut dyn FnMut(u64, usize)) -> Share {
        let mut y = Fp::zero();
        for seed in &self.held {
            observe(self.party, seed.idx);
            y = y.add(prf_fp(seed.key, ctr).mul(seed.coeff));
        }
        Share { x: self.party, y }
    }
}

/// The `r ≠ 0` acceptance test of [`PrssRandomness::shared_nonzero_mask`],
/// factored out so the rejection branch — which a live draw hits with probability
/// `1/p ≈ 2^−61` — is directly testable.
///
/// Returns the sharing iff the shared value is nonzero; `None` means "redraw at a
/// fresh counter". See the module note: evaluating `value` at all is the
/// simulation artefact that a distributed zero-test must replace.
fn accept_if_nonzero(value: Fp, shares: Vec<Share>) -> Option<Vec<Share>> {
    if value == Fp::zero() {
        None
    } else {
        Some(shares)
    }
}

/// `f_A(z) = Π_{j∈A} (x_j − z)/x_j` — the public degree-`|A|` polynomial that is
/// `1` at `z = 0` and `0` at every party point in `A`.
///
/// Public in every argument (it depends only on the party points), so the
/// variable-time `Fp::inv` it uses touches nothing secret. Party points are
/// `1..=n`, never `0`, so the inverse always exists.
fn unqualified_poly_at(set: &[u64], z: Fp) -> Fp {
    let mut acc = Fp::one();
    for &xj in set {
        let x = Fp::new(xj);
        acc = acc.mul(x.sub(z)).mul(x.inv());
    }
    acc
}

/// `PRF(key, ctr)` as a uniform field element.
///
/// A domain-separated SHA-512 **key derivation** — `SHA-512(DOMAIN ‖ k_A ‖ ctr)`,
/// all inputs fixed-length so no two distinct `(k_A, ctr)` pairs share a preimage
/// — keys a fresh ChaCha20 stream ([`crate::chacha`], the same vetted generator
/// the masking CSPRNG uses), from which one field element is drawn by the
/// rejection sampler in [`MpcRng::next_fp`] (uniform over `F_p`, no modulo bias).
///
/// Deriving a fresh stream per `(seed, counter)` — rather than seeking a single
/// per-seed stream — keeps every counter's output independent by construction,
/// so a rejection-sampling redraw can never bleed into the next counter's value.
/// The derived key is never exposed, and both it and the digest are scrubbed
/// before this returns.
fn prf_fp(key: &[u8; 32], ctr: u64) -> Fp {
    let mut hasher = Sha512::new();
    hasher.update(PRF_DOMAIN);
    hasher.update(key);
    hasher.update(ctr.to_le_bytes());
    let mut digest = hasher.finalize();

    let mut stream_key = Zeroizing::new([0u8; 32]);
    stream_key.copy_from_slice(&digest[..32]);
    // The digest is derived secret material; scrub the buffer we own.
    digest[..].zeroize();

    let mut stream = PrfStream {
        inner: ChaCha20Csprng::from_seed(*stream_key),
    };
    stream.next_fp()
}

/// One PRF invocation's keystream, wrapped so the crate's uniform rejection
/// sampler ([`MpcRng::next_fp`]) applies unchanged. The generator's key schedule
/// is scrubbed when this is dropped (`ChaCha20Csprng` is `ZeroizeOnDrop`).
struct PrfStream {
    inner: ChaCha20Csprng,
}

impl MpcRng for PrfStream {
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }
}

/// `C(n, k)`, or `None` if it exceeds [`MAX_PRSS_SEEDS`].
///
/// Computed multiplicatively with an early abort, so an absurd `n` can never
/// overflow or drive an allocation: the accumulator is bounded by
/// `MAX_PRSS_SEEDS` on entry to every step.
fn seed_count(n: usize, k: usize) -> Option<usize> {
    // C(n, k) == C(n, n−k); iterate over the smaller one.
    let k = k.min(n - k);
    let mut acc: u128 = 1;
    for i in 0..k {
        // Exact at every step: a product of `i+1` consecutive integers is
        // divisible by `(i+1)!`.
        acc = acc * (n - i) as u128 / (i as u128 + 1);
        if acc > MAX_PRSS_SEEDS as u128 {
            return None;
        }
    }
    Some(acc as usize)
}

/// Every maximal unqualified set — the `t`-subsets of the party points `1..=n` —
/// in lexicographic order.
fn unqualified_sets(n: usize, t: usize) -> Vec<Vec<u64>> {
    if t == 0 {
        // The single empty set: its seed is held by every party and `f_∅ ≡ 1`.
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    // Lexicographic t-combination enumeration over 1..=n.
    let mut idx: Vec<u64> = (1..=t as u64).collect();
    loop {
        out.push(idx.clone());
        // Find the rightmost position that can still be advanced.
        let mut i = t;
        while i > 0 && idx[i - 1] == (n - t + i) as u64 {
            i -= 1;
        }
        if i == 0 {
            return out;
        }
        idx[i - 1] += 1;
        for j in i..t {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

/// PRSS behind the dealer-less seam.
///
/// Reports [`RandomnessModel::Prss`] — an honest *description* of the regime, and
/// still **not** a deployment credential: `Prss.deployable()` remains `false`, so
/// [`require_deployable`](DistributedRandomness::require_deployable) refuses this
/// source exactly as it refuses the single-dealer simulation. Acceptance is tied
/// to a validated construction behind the boundary of
/// `research/mpc-distributed-randomness-design.md` §5 item 5, and the crate is
/// research-grade and externally unaudited (`sq-qhy4` pending).
impl DistributedRandomness for PrssRandomness {
    fn randomness_model(&self) -> RandomnessModel {
        RandomnessModel::Prss
    }

    /// A fresh degree-`t` sharing of the PRSS value at the next counter — each
    /// party's `C(n−1, t)` local PRF evaluations over its own view, **zero
    /// rounds**. The simulation-only value oracle is not evaluated on this path.
    fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError> {
        let ctr = self.next_counter()?;
        Ok(self.shares_at(ctr))
    }

    /// As [`shared_mask`](Self::shared_mask), redrawing at a fresh counter on the
    /// `1/p ≈ 2^−61` chance that the shared value is `0` (the equality mask must
    /// be nonzero — [`crate::randomness`] module note).
    ///
    /// **The rejection test is a SIMULATION ARTEFACT.** It evaluates
    /// `r = Σ_A PRF(k_A, ctr)` centrally, which is possible only because this
    /// in-process simulation holds every seed; no party can do it. A real
    /// deployment must replace it with a distributed zero-test on `[r]` whose
    /// opens are authenticated (design record §1 part 1, §5 item 4). Until then
    /// this method — like every `shared_nonzero_mask` — is **semi-honest only**
    /// and makes no active-adversary claim.
    fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
        loop {
            let ctr = self.next_counter()?;
            let (value, shares) = self.sharing_at(ctr);
            if let Some(accepted) = accept_if_nonzero(value, shares) {
                return Ok(accepted);
            }
        }
    }

    /// **Honestly refused.** PRSS is a *correlated-randomness generator*, not an
    /// input-sharing protocol: it produces a sharing of a value nobody chose,
    /// which is precisely the wrong shape for "a holder shares the secret it
    /// already has". Dealer-less VSS is a distinct construction (pairwise
    /// consistency / Feldman, composing with [`crate::robust`] on the output side)
    /// and a distinct follow-on bead — design record §3 / §5 item 3. Faking it
    /// with an unverified plain sharing would silently drop the verification that
    /// is the entire point, so this returns
    /// [`MpcError::NotYetImplemented`] instead.
    fn vss_own_input(&mut self, _secret: Fp) -> Result<Vec<Share>, MpcError> {
        Err(MpcError::not_yet(
            "dealer-less VSS input sharing (PRSS generates correlated randomness, \
             it does not verifiably share a holder's own input)",
            "sq-yyro follow-on: research/mpc-distributed-randomness-design.md §3 / §5 item 3",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shamir::ShamirBackend;
    use std::collections::BTreeSet;

    fn prss(n: usize) -> PrssRandomness {
        PrssRandomness::with_seeded_setup(n, 0xC0FFEE).expect("valid small-n setup")
    }

    #[test]
    fn reports_the_prss_model_but_is_still_not_deployable() {
        let source = prss(5);
        let model = source.randomness_model();
        assert_eq!(model, RandomnessModel::Prss);
        assert!(model.is_dealer_less(), "PRSS is dealer-less BY DESIGN");
        // The honesty line: a real generator landing does NOT by itself make the
        // source deployment-acceptable. The label is descriptive; acceptance is
        // tied to a validated construction (design §5 item 5) and sq-qhy4 is
        // still pending, so the fail-closed gate must still refuse.
        assert!(!model.deployable());
        assert!(matches!(
            source.require_deployable(),
            Err(MpcError::NoBackendSatisfies { .. })
        ));
    }

    #[test]
    fn setup_matches_the_honest_majority_threshold_and_seed_counts() {
        // C(n, t) for t = ⌊(n−1)/2⌋, and the per-party count C(n−1, t).
        for (n, t, total, per_party) in [
            (2usize, 0usize, 1usize, 1usize), // C(2,0)=1, C(1,0)=1
            (3, 1, 3, 2),                     // C(3,1)=3, C(2,1)=2
            (4, 1, 4, 3),                     // C(4,1)=4, C(3,1)=3
            (5, 2, 10, 6),                    // C(5,2)=10, C(4,2)=6
            (9, 4, 126, 70),                  // C(9,4)=126, C(8,4)=70 — the ceiling case
        ] {
            let source = prss(n);
            assert_eq!(source.parties(), n);
            assert_eq!(source.threshold(), t, "n = {n}");
            assert_eq!(source.seed_count(), total, "C({n}, {t})");
            assert_eq!(source.seeds_per_party(), per_party, "C({}, {t})", n - 1);
        }
        // Cross-check the per-party accessor against the symmetry it relies on:
        // EVERY party holds the same number of seeds, so counting the sets that
        // exclude party 1 must equal counting those that exclude any other party.
        let source = prss(7);
        for party in 1..=7u64 {
            let held = source
                .seeds
                .iter()
                .filter(|s| !s.set.contains(&party))
                .count();
            assert_eq!(held, source.seeds_per_party(), "party {party}");
        }
    }

    #[test]
    fn seed_count_guard_refuses_impractical_n() {
        // n = 9 → C(9,4) = 126 <= 128: the ceiling case still builds.
        assert!(PrssRandomness::with_seeded_setup(9, 1).is_ok());
        // n = 10 → C(10,4) = 210 > 128: refused fail-closed, naming the fallback.
        let err = PrssRandomness::with_seeded_setup(10, 1).expect_err("must refuse n = 10");
        match err {
            MpcError::Protocol(m) => {
                assert!(m.contains("coin-toss"), "refusal must name the fallback: {m}")
            }
            other => panic!("expected a Protocol refusal, got {other:?}"),
        }
        // And an absurd n must be refused by the same guard, not overflow.
        assert!(PrssRandomness::with_seeded_setup(1_000, 1).is_err());
        // n < 2 is refused like ShamirBackend::new.
        assert!(PrssRandomness::with_seeded_setup(1, 1).is_err());
    }

    #[test]
    fn seed_count_is_exact_below_the_cap_and_none_above() {
        assert_eq!(seed_count(9, 4), Some(126));
        assert_eq!(seed_count(8, 3), Some(56));
        assert_eq!(seed_count(2, 0), Some(1));
        assert_eq!(seed_count(10, 4), None);
        assert_eq!(seed_count(usize::MAX, 3), None);
    }

    #[test]
    fn unqualified_sets_are_the_t_subsets_in_order() {
        assert_eq!(unqualified_sets(4, 1), vec![vec![1], vec![2], vec![3], vec![4]]);
        assert_eq!(
            unqualified_sets(4, 2),
            vec![
                vec![1, 2],
                vec![1, 3],
                vec![1, 4],
                vec![2, 3],
                vec![2, 4],
                vec![3, 4]
            ]
        );
        // t = 0 → the single empty set (held by everyone).
        assert_eq!(unqualified_sets(2, 0), vec![Vec::<u64>::new()]);
        // Count matches C(n, t) for every admissible n.
        for n in 2..=9usize {
            let t = (n - 1) / 2;
            assert_eq!(unqualified_sets(n, t).len(), seed_count(n, t).unwrap());
        }
    }

    #[test]
    fn f_a_is_one_at_the_secret_and_zero_on_its_own_set() {
        for n in 2..=9usize {
            let t = (n - 1) / 2;
            for set in unqualified_sets(n, t) {
                // f_A(0) = 1 — this is what makes F(0) the SUM of the PRF outputs.
                assert_eq!(
                    unqualified_poly_at(&set, Fp::zero()),
                    Fp::one(),
                    "f_A(0) must be 1 for A = {set:?}"
                );
                // f_A(x_j) = 0 for every j in A — the structural reason a party
                // never needs a seed it does not hold.
                for &xj in &set {
                    assert_eq!(
                        unqualified_poly_at(&set, Fp::new(xj)),
                        Fp::zero(),
                        "f_A must vanish at {xj} for A = {set:?}"
                    );
                }
                // ...and nonzero off `A`, so the seed genuinely contributes to
                // every holder's share (a vanishing f_A off A would make the
                // sharing degenerate).
                for x in 1..=n as u64 {
                    if !set.contains(&x) {
                        assert_ne!(unqualified_poly_at(&set, Fp::new(x)), Fp::zero());
                    }
                }
            }
        }
    }

    /// **The load-bearing structural invariant.** A share may only evaluate the
    /// PRF on seeds its party actually holds; PRSS achieves that because
    /// `f_A(x_i) = 0` exactly when `i ∈ A`.
    ///
    /// Checked on the REAL generation path (the one `shared_mask` runs), via the
    /// [`PrssRandomness::shares_at_observed`] hook, which fires **at** each
    /// `prf_fp` call site inside the party-local [`PartyView::share_at`] — not on
    /// a re-derivation of the coefficients, and not after a global evaluation of
    /// every key, either of which would leave a generator that reached for a
    /// non-held seed undetected (its coefficient is zero, so the emitted numbers
    /// would be unchanged).
    #[test]
    fn no_share_uses_a_seed_its_party_does_not_hold() {
        const CTR: u64 = 9;
        for n in 2..=9usize {
            let source = prss(n);
            // (a) The coefficient structure that MAKES seed-locality possible.
            for seed in &source.seeds {
                for (i, &f) in seed.f_at.iter().enumerate() {
                    let party = i as u64 + 1;
                    if seed.set.contains(&party) {
                        assert_eq!(
                            f,
                            Fp::zero(),
                            "party {party} does not hold the seed for A = {:?}, so that seed's \
                             coefficient in its share MUST be zero",
                            seed.set
                        );
                    } else {
                        assert_ne!(f, Fp::zero());
                    }
                }
            }

            // (b) The generation path itself: every key each party's generator
            // actually evaluated, recorded at the `prf_fp` call site.
            let mut accessed: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
            let shares = source.shares_at_observed(CTR, |party, idx| {
                accessed[party as usize - 1].insert(idx);
            });
            for party in 1..=n as u64 {
                let held: BTreeSet<usize> = source.view_of(party).collect();
                // The view is exactly the complement of the sets containing the
                // party — i.e. the seeds it is a non-holder of are forbidden.
                for (idx, seed) in source.seeds.iter().enumerate() {
                    assert_eq!(
                        held.contains(&idx),
                        !seed.set.contains(&party),
                        "party {party} holds the seed for A = {:?} iff it is not in A",
                        seed.set
                    );
                }
                assert_eq!(held.len(), source.seeds_per_party());
                // The party-local view carries key material for EXACTLY those
                // seeds — a forbidden key is not merely unused, it is absent.
                let carried: BTreeSet<usize> =
                    source.party_view(party).held.iter().map(|h| h.idx).collect();
                assert_eq!(
                    carried, held,
                    "party {party} (n = {n}) must be handed EXACTLY its own key material"
                );
                assert_eq!(
                    accessed[party as usize - 1], held,
                    "party {party} (n = {n}) must generate from EXACTLY its own seed view"
                );
            }

            // (c) Those seed-local shares agree with the GLOBAL construction
            // `F(x_i) = Σ_A PRF(k_A, ctr)·f_A(x_i)` summed over ALL seeds — the
            // sum no single party can compute.
            for (i, share) in shares.iter().enumerate() {
                let global = source.seeds.iter().fold(Fp::zero(), |acc, seed| {
                    acc.add(prf_fp(&seed.key, CTR).mul(seed.f_at[i]))
                });
                assert_eq!(share.y, global, "party {} (n = {n})", i + 1);
            }

            // (d) Mutation guard, so (b) is not vacuous — run through the SAME
            // party-local interface, not around it.
            if source.t >= 1 {
                let party = 1u64;
                let forbidden = source
                    .seeds
                    .iter()
                    .position(|s| s.set.contains(&party))
                    .expect("t >= 1 ⇒ some unqualified set contains party 1");
                assert!(!accessed[0].contains(&forbidden));

                // Splice the forbidden key into party 1's view with its REAL
                // coefficient — the shape an actual seed-locality bug takes — and
                // re-run `share_at`. That coefficient is `f_A(x_1) = 0`, so the
                // emitted share is IDENTICAL: no value-level check could catch
                // this. Only the access instrumentation does, which is precisely
                // why (b) asserts on the access set and not on the numbers.
                let mut tampered = source.party_view(party);
                let real_coeff = source.seeds[forbidden].f_at[party as usize - 1];
                assert_eq!(real_coeff, Fp::zero(), "a non-held seed's coefficient is 0");
                tampered.held.push(HeldSeed {
                    idx: forbidden,
                    key: &source.seeds[forbidden].key,
                    coeff: real_coeff,
                });
                let mut leaked_access = BTreeSet::new();
                let leaked = tampered.share_at(CTR, &mut |_, idx| {
                    leaked_access.insert(idx);
                });
                assert_eq!(
                    leaked.y, shares[0].y,
                    "the real coefficient is 0, so a leak is INVISIBLE in the share value"
                );
                assert!(leaked_access.contains(&forbidden));
                assert_ne!(
                    leaked_access, accessed[0],
                    "a forbidden key evaluated by the party-local generator MUST show up \
                     in the access set — otherwise (b) is vacuous"
                );
            }
        }
    }

    #[test]
    fn generated_sharing_is_degree_t_and_reconstructs_to_the_prf_sum() {
        for n in 2..=9usize {
            let source = prss(n);
            let backend = ShamirBackend::new(n).expect("n >= 2");
            assert_eq!(backend.threshold(), source.threshold());
            for ctr in 0..8u64 {
                let (value, shares) = source.sharing_at(ctr);
                // Full n-party sharing on the canonical points, in order.
                assert_eq!(shares.len(), n);
                for (i, s) in shares.iter().enumerate() {
                    assert_eq!(s.x, i as u64 + 1);
                }
                // Reconstruction (which for n > t+1 also RS-checks that all n
                // points lie on ONE degree-t polynomial) yields Σ_A PRF(k_A, ctr).
                assert_eq!(
                    backend.reconstruct(&shares).expect("consistent degree-t sharing"),
                    value,
                    "n = {n}, ctr = {ctr}"
                );
            }
        }
    }

    #[test]
    fn any_t_plus_one_shares_reconstruct_the_same_value() {
        // Degree-`t`-ness restated as the property that matters: every quorum
        // agrees. A generator that emitted a higher-degree polynomial would let
        // different quorums reconstruct different values.
        let n = 7;
        let source = prss(n);
        let backend = ShamirBackend::new(n).expect("n >= 2");
        let t = source.threshold();
        let (value, shares) = source.sharing_at(42);
        // Every contiguous window of t+1 shares, plus a strided quorum.
        for start in 0..=(n - (t + 1)) {
            let quorum = &shares[start..start + t + 1];
            assert_eq!(backend.reconstruct(quorum).expect("quorum"), value);
        }
        let strided: Vec<Share> = shares.iter().step_by(2).copied().take(t + 1).collect();
        assert_eq!(strided.len(), t + 1);
        assert_eq!(backend.reconstruct(&strided).expect("strided quorum"), value);
    }

    #[test]
    fn each_draw_advances_the_counter_and_yields_a_fresh_mask() {
        let mut source = prss(5);
        let backend = ShamirBackend::new(5).expect("n >= 2");
        let mut seen = Vec::new();
        for expected_ctr in 0..32u64 {
            assert_eq!(source.counter, expected_ctr);
            let shares = source.shared_mask().expect("mask");
            seen.push(backend.reconstruct(&shares).expect("reconstruct"));
        }
        assert_eq!(source.counter, 32);
        // Distinct counters must give distinct masks (a counter that failed to
        // advance would silently re-issue the same blinding factor).
        let mut sorted = seen.clone();
        sorted.sort_by_key(|v| v.value());
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "masks must not repeat");
    }

    #[test]
    fn counter_exhaustion_is_refused_rather_than_wrapped() {
        let mut source = prss(4);
        source.counter = u64::MAX;
        assert!(matches!(source.shared_mask(), Err(MpcError::Protocol(_))));
        assert!(matches!(
            source.shared_nonzero_mask(),
            Err(MpcError::Protocol(_))
        ));
        // The counter must not have wrapped to 0 (which would re-issue mask #0).
        assert_eq!(source.counter, u64::MAX);
    }

    #[test]
    fn shared_nonzero_mask_never_shares_zero() {
        let n = 5;
        let mut source = prss(n);
        let backend = ShamirBackend::new(n).expect("n >= 2");
        for _ in 0..1_000 {
            let shares = source.shared_nonzero_mask().expect("nonzero mask");
            assert_ne!(
                backend.reconstruct(&shares).expect("reconstruct"),
                Fp::zero(),
                "the equality mask must never be r = 0"
            );
        }
    }

    #[test]
    fn nonzero_acceptance_rejects_exactly_the_zero_value() {
        // The `r = 0` branch is hit by a live draw with probability ~2^-61, so
        // the predicate is tested directly. Flipping its sense must go red here.
        let shares = vec![Share {
            x: 1,
            y: Fp::new(7),
        }];
        assert!(accept_if_nonzero(Fp::zero(), shares.clone()).is_none());
        assert_eq!(
            accept_if_nonzero(Fp::one(), shares.clone()),
            Some(shares.clone())
        );
        assert_eq!(accept_if_nonzero(Fp::new(12345), shares.clone()), Some(shares));
    }

    #[test]
    fn setup_seeds_determine_the_stream() {
        // Same setup seed → same replicated seeds → identical generated sharings
        // (the generator is deterministic in (seeds, counter), which is exactly
        // why the counter must never repeat).
        let a = prss(5);
        let b = prss(5);
        assert_eq!(a.sharing_at(3), b.sharing_at(3));
        // A different setup → a different stream.
        let c = PrssRandomness::with_seeded_setup(5, 0xBEEF).expect("setup");
        assert_ne!(a.sharing_at(3).0, c.sharing_at(3).0);
    }

    #[test]
    fn os_setup_produces_independent_sources() {
        // The production setup path must not be a fixed sequence.
        let a = PrssRandomness::with_simulated_setup(4).expect("setup");
        let b = PrssRandomness::with_simulated_setup(4).expect("setup");
        assert_ne!(a.sharing_at(0).0, b.sharing_at(0).0);
    }

    #[test]
    fn prf_is_domain_separated_by_seed_and_counter() {
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        assert_ne!(prf_fp(&k1, 0), prf_fp(&k2, 0), "distinct seeds must differ");
        assert_ne!(prf_fp(&k1, 0), prf_fp(&k1, 1), "distinct counters must differ");
        // Deterministic in (seed, counter) — the whole construction depends on
        // every holder of `k_A` deriving the SAME PRF output locally.
        assert_eq!(prf_fp(&k1, 7), prf_fp(&k1, 7));
    }

    #[test]
    fn vss_own_input_is_honestly_refused() {
        // PRSS is not an input-sharing protocol; faking it with an unverified
        // plain sharing would drop the verification that is its whole point.
        let mut source = prss(5);
        assert!(matches!(
            source.vss_own_input(Fp::new(4085)),
            Err(MpcError::NotYetImplemented { .. })
        ));
    }

    #[test]
    fn debug_never_prints_seed_material() {
        let source = prss(4);
        let rendered = format!("{source:?}");
        assert!(rendered.contains("<replicated seed>"));
        for seed in &source.seeds {
            let leaked = format!("{:?}", &seed.key[..4]);
            assert!(
                !rendered.contains(&leaked),
                "Debug must not render seed bytes"
            );
        }
    }
}
