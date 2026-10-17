// sq-yyro follow-on (issue #3531): the PRSS correlated-randomness
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
//! ## The dealer-less SHUFFLE on these seeds (`oblivious_shuffle`)
//!
//! The seam's [`DistributedRandomness::oblivious_shuffle`] is the one method here
//! that permutes a secret-shared column rather than generating a value, and this
//! module implements it dealer-lessly as a **composed resharing shuffle**: one
//! round per maximal unqualified set `A`, in which the group `S_A = parties \ A`
//! derives `pi_A` from the seed `k_A` (domain-separated PRF stream, unbiased
//! Fisher-Yates), converts each element to an additive sharing among its `t+1`
//! lowest members via the public Lagrange-at-0 weights, permutes those additive
//! shares by `pi_A`, and reshares them as fresh degree-`t` sharings whose masking
//! randomness comes from each member's **own private key** — not from a
//! replicated seed. The composed permutation is `pi = pi_{A_k} ∘ … ∘ pi_{A_1}`.
//!
//! - **Semi-honest, honest-majority (`n >= 2t+1`), at most `t` corrupted.** For
//!   the real corrupt set `C`, extend it to a `t`-subset `A ⊇ C`; `k_A` is held by
//!   exactly the parties outside `A`, so no party in `C` can evaluate it, `pi_A`
//!   is uniform and hidden from the adversary, and the composed `pi` is uniform
//!   from its view. Public coins never choose the permutation and `pi` is never
//!   opened. Computational, as everything else here (it rests on the PRF).
//! - **NOT malicious-secure.** Nothing verifies that a group actually applied a
//!   permutation; a deviating member can corrupt or reorder outputs undetected.
//!   The consumer-side shape check is a SHAPE check only.
//! - **Cost:** `C(n, t)` rounds of resharing — the same combinatorial blow-up as
//!   the seed count, fine for the small `n` this module targets and a further
//!   reason it is a small-`n` tool.
//! - **Degree-`<= t` inputs only**, checked and refused otherwise: the conversion
//!   recombines from `t+1` points, so a degree-`2t` element (e.g. an
//!   un-re-randomised `selector * real_tag` product) would come back as a
//!   well-formed degree-`t` sharing of a DIFFERENT secret. A caller with such a
//!   column must degree-reduce it first — the oblivious output stage does, for
//!   every selected element, before it shuffles.
//! - **Fresh output sharings**, so the output is unlinkable to the input; no
//!   caller may match entries by share equality.
//! - **Ordered delivery is a TRANSPORT obligation, not enforced here.** Over real
//!   private channels each round's resharing batches must be emitted in
//!   output-position order; a source index, a per-element counter or
//!   input-order timing would leak `pi`. In-process there are no channels, so
//!   nothing enforces it yet.
//! - A **per-party private key** is part of the simulated setup for this. A real
//!   party generates it locally from its own entropy (there is nothing to
//!   distribute); it is drawn here only because this process plays all `n`
//!   parties.
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
use crate::oblivious::{SecretColumn, ShuffleCost};
use crate::partial::MpcError;
use crate::randomness::{DistributedRandomness, RandomnessModel};
use crate::rng::MpcRng;
use crate::shamir::{eval_poly, lagrange_weights_at, lagrange_zero_weights, Share};
use sha2::{Digest, Sha512};
use zeroize::{Zeroize, Zeroizing};

/// Domain-separation tag for the PRSS PRF key derivation. Versioned so a future
/// PRF change is a distinct, non-colliding derivation rather than a silent
/// re-interpretation of the same seeds.
const PRF_DOMAIN: &[u8] = b"sparq-mpc/prss/v1/prf";

/// Domain-separation tag for the shuffle's **permutation** stream, derived from a
/// replicated seed `k_A`. Disjoint from [`PRF_DOMAIN`], so the same `(k_A, ctr)`
/// never yields both a mask summand and a permutation.
const PRF_SHUFFLE_PERM_DOMAIN: &[u8] = b"sparq-mpc/prss/v1/shuffle-perm";

/// Domain-separation tag for a party's **own** resharing randomness inside the
/// shuffle, derived from that party's private key (never a replicated seed).
const PRF_SHUFFLE_RESHARE_DOMAIN: &[u8] = b"sparq-mpc/prss/v1/shuffle-reshare";

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
    /// Party `i`'s **own private** PRF key, at index `i − 1` — held by that party
    /// ALONE, never replicated to anyone. Indexed by party point, so
    /// `private_keys[i − 1]` is party `i`'s.
    ///
    /// This is a different kind of key material from [`Self::seeds`]: a
    /// replicated seed is shared with a group, and every one of them is unknown to
    /// exactly one maximal unqualified set — whereas a party's private key is
    /// unknown to *everyone else*. The resharing shuffle
    /// ([`PrssRandomness::resharing_shuffle`]) needs the latter: its fresh
    /// resharing randomness must not come from a seed the corrupt set holds, or
    /// the adversary could recompute the honest parties' resharing polynomials.
    ///
    /// **Simulation note.** A real party generates this itself, locally, from its
    /// own OS entropy — there is nothing to distribute, so unlike the replicated
    /// seeds this needs no setup protocol. It is drawn here from the setup RNG only
    /// because this in-process simulation plays all `n` parties, and the whole
    /// process therefore holds every party's private key (the same in-process
    /// caveat as the seeds — see the module docs).
    private_keys: Vec<Zeroizing<[u8; 32]>>,
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

        // Each party's OWN private key — in a deployment generated locally by that
        // party and never distributed; drawn here because the simulation plays
        // every party.
        let private_keys = (0..n)
            .map(|_| {
                let mut key = Zeroizing::new([0u8; 32]);
                for chunk in key.chunks_exact_mut(8) {
                    chunk.copy_from_slice(&setup_rng.next_u64().to_le_bytes());
                }
                key
            })
            .collect();

        Ok(PrssRandomness {
            n,
            t,
            seeds,
            private_keys,
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

    /// The **composed resharing shuffle** behind
    /// [`DistributedRandomness::oblivious_shuffle`] — see that impl's docs for the
    /// construction, the security model and the cost.
    ///
    /// # Errors
    ///
    /// [`MpcError::Protocol`] if `col` is not the layout this construction can
    /// interpret: every entry must have the same width, that width must be a
    /// multiple of `n`, each `n`-wide group must sit on the canonical points
    /// `x = 1..=n` in order, and every such group must be a consistent
    /// **degree-`<= t`** sharing (`require_degree_t_elements`). Unlike a
    /// switch-network shuffle — which only swaps whole entries and so never reads
    /// a share — this one converts each *element* of an entry between sharings, so
    /// it must know where each party's share of each element is and at what
    /// degree. A column it cannot parse, or whose degree it cannot honour, is
    /// refused rather than re-interpreted.
    ///
    /// Every refusal happens **before** `pi` or any resharing randomness is drawn,
    /// so a refused call consumes no PRF counter and leaves the source exactly as
    /// it was.
    fn resharing_shuffle(
        &mut self,
        col: SecretColumn,
    ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
        let m = col.len();
        let width = col.first().map(|e| e.len()).unwrap_or(0);
        for (j, entry) in col.iter().enumerate() {
            if entry.len() != width {
                return Err(MpcError::Protocol(format!(
                    "PRSS resharing shuffle: entry {j} is {} shares wide but entry 0 is {} — \
                     every entry must have the same width",
                    entry.len(),
                    width
                )));
            }
            for (idx, sh) in entry.iter().enumerate() {
                let expected = (idx % self.n) as u64 + 1;
                if sh.x != expected {
                    return Err(MpcError::Protocol(format!(
                        "PRSS resharing shuffle: entry {j} share {idx} is on x = {} (expected \
                         {expected}); an entry must be whole {}-party sharings on x = 1..={}, \
                         in order",
                        sh.x, self.n, self.n
                    )));
                }
            }
        }
        if !width.is_multiple_of(self.n) {
            return Err(MpcError::Protocol(format!(
                "PRSS resharing shuffle: entry width {width} is not a multiple of n = {}; \
                 an entry must be a whole number of {}-party sharings",
                self.n, self.n
            )));
        }
        let elements = width / self.n;
        // The conversion below interpolates each element from t+1 points, which is
        // only faithful for a degree-<= t sharing — so the degree is required, not
        // assumed. Checked before anything is drawn.
        self.require_degree_t_elements(&col, elements)?;
        // Nothing to permute: no resharing round runs, so none is charged.
        if m <= 1 {
            return Ok((
                col,
                ShuffleCost {
                    items: m,
                    switches: 0,
                    depth_rounds: 0,
                    // No resharing round runs, so the input share vectors come
                    // straight back. With at most one entry there is no position
                    // for them to be paired with.
                    fresh_sharings: 0,
                },
            ));
        }
        let rounds = self.seeds.len();
        let mut cur = col;
        for a in 0..rounds {
            cur = self.resharing_round(a, m, elements, &cur)?;
        }
        Ok((
            cur,
            ShuffleCost {
                items: m,
                // No switch network is routed: this construction permutes by
                // resharing, not by conditional swaps.
                switches: 0,
                // One resharing round per maximal unqualified set.
                depth_rounds: rounds,
                // Every element of every entry comes back as a fresh sharing: the
                // last resharing round masks it with randomness from each group
                // member's own private key. At `t = 0` a resharing draws no
                // masking coefficient, so the share vectors are only reordered
                // and nothing is counted, matching the dealer path.
                fresh_sharings: if self.t == 0 {
                    0
                } else {
                    m.saturating_mul(elements)
                },
            },
        ))
    }

    /// Fail closed unless EVERY element of EVERY entry is a consistent
    /// degree-`<= t` sharing over the `n` party points.
    ///
    /// ## Why this is required, not merely assumed
    ///
    /// The resharing conversion reads an element's shares at only `t+1` points and
    /// treats them as determining the secret (the Lagrange-at-0 weights for those
    /// points). For a sharing of degree `> t` that is simply a different secret:
    /// the output would be a well-formed degree-`t` sharing of the WRONG value, and
    /// because it is well-formed no later reconstruction could notice. The caller
    /// case to watch for is a select product packed without re-randomisation: a
    /// degree-`2t` `selector * real_tag` alongside degree-`t` payload limbs, a
    /// MIXED-degree entry. The oblivious output stage degree-reduces every
    /// selected element before it shuffles (see
    /// [`crate::oblivious_join::oblivious_set_output`]), so it presents a
    /// degree-`t` column; the degree is still checked here, and an unsupported
    /// sharing from any caller is refused.
    ///
    /// ## What the check is, exactly
    ///
    /// Interpolate the degree-`t` polynomial through the first `t+1` party points
    /// and require every remaining point (`n - t - 1` of them) to lie on it. The
    /// shares ARE the data, so this is **exact and complete** on the given points:
    /// if all `n` points lie on a degree-`<= t` polynomial then that polynomial is
    /// the one the conversion will use, and if they do not, they are rejected.
    /// There is no probabilistic gap to state.
    ///
    /// ## What it is NOT
    ///
    /// A **simulation-side contract check.** It reads all `n` shares of every
    /// element, which only this in-process simulation can do — a real party holds
    /// one share. A deployment must either guarantee degree `t` *by construction*
    /// (degree-reduce before shuffling) or carry a degree proof; it cannot run
    /// this. And it is **not a malicious-security check**: it says nothing about
    /// whether the shares are the ones an honest party would have sent, and
    /// nothing downstream verifies that this shuffle applied a permutation at all.
    fn require_degree_t_elements(
        &self,
        col: &SecretColumn,
        elements: usize,
    ) -> Result<(), MpcError> {
        // The t+1 points the conversion will actually interpolate from, and the
        // public extrapolation weights for every remaining point.
        let base: Vec<u64> = (1..=self.t as u64 + 1).collect();
        let checks: Vec<(usize, Vec<Fp>)> = ((self.t + 2)..=self.n)
            .map(|x| (x, lagrange_weights_at(&base, Fp::new(x as u64))))
            .collect();
        if checks.is_empty() {
            // No redundant point: with n = t+1 every point set lies on SOME
            // degree-t polynomial, so there is nothing to reject. The
            // honest-majority constructors never build such a source.
            return Ok(());
        }
        for (j, entry) in col.iter().enumerate() {
            for e in 0..elements {
                let ys = &entry[e * self.n..(e + 1) * self.n];
                for (x, weights) in &checks {
                    let extrapolated = weights
                        .iter()
                        .zip(ys)
                        .fold(Fp::zero(), |acc, (w, sh)| acc.add(w.mul(sh.y)));
                    if extrapolated != ys[*x - 1].y {
                        // A note for any future PRSS integration of the oblivious
                        // output stage: EVERY selected element must be
                        // degree-reduced before the shuffle — the slot tags AND all
                        // of their payload limbs, not the tags alone. Reducing only
                        // the tags leaves the limbs as degree-`2t` products, which
                        // this check refuses, and which an open would reveal more
                        // of than their values.
                        //
                        // Name the POSITION only. The two values compared here are
                        // a share and a function of t+1 shares, so printing either
                        // leaks secret-shared data into logs: at n = 3, t = 1 one
                        // of them plus one party's own share reconstructs the
                        // secret. The caller needs to know which element is wrong,
                        // not what it holds.
                        return Err(MpcError::Protocol(format!(
                            "PRSS resharing shuffle: entry {j} element {e} is not a consistent \
                             degree-<= t sharing (t = {}): its share at x = {x} is off the \
                             polynomial through x = 1..={} (share values withheld). This \
                             construction recombines from t+1 points, so a higher-degree \
                             sharing (e.g. an un-re-randomised degree-2t selector-tag \
                             product) would be silently re-interpreted as a different secret - \
                             degree-reduce it first",
                            self.t,
                            self.t + 1
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// One resharing round: the group `S_A = parties \ A` permutes the column by
    /// `pi_A`, derived from the seed `k_A` that **no party in `A`** holds.
    ///
    /// The `t+1` lowest-numbered members of `S_A` (`n − t >= t + 1` under honest
    /// majority, so they exist) convert each element's degree-`t` sharing to an
    /// additive sharing via the public Lagrange-at-0 weights for their own points,
    /// permute their additive shares by `pi_A`, and each reshares its additive
    /// share as a fresh degree-`t` Shamir sharing to all `n` parties from its OWN
    /// private randomness. Summing the `t+1` resharings componentwise gives the new
    /// degree-`t` sharing of the permuted element.
    fn resharing_round(
        &mut self,
        a: usize,
        m: usize,
        elements: usize,
        col: &SecretColumn,
    ) -> Result<SecretColumn, MpcError> {
        // The permutation comes from `k_A` alone — every party in `A` is missing
        // that key, so `pi_A` is pseudorandom and hidden from all of them.
        let perm_ctr = self.next_counter()?;
        let perm = {
            let mut stream = prf_stream(PRF_SHUFFLE_PERM_DOMAIN, &self.seeds[a].key, perm_ctr);
            fisher_yates(&mut stream, m)?
        };

        // S_A's t+1 lowest members as the recombination set, with the PUBLIC
        // Lagrange-at-0 weights for exactly those points.
        let recomb = self.recombination_group(a);
        let lambdas = lagrange_zero_weights(&recomb);

        let mut out: SecretColumn = (0..m)
            .map(|_| {
                (0..elements * self.n)
                    .map(|idx| Share {
                        x: (idx % self.n) as u64 + 1,
                        y: Fp::zero(),
                    })
                    .collect()
            })
            .collect();

        for (j, entry) in col.iter().enumerate() {
            // Each member of `recomb` holds its additive share of entry `j` and
            // places it at `pi_A(j)`; resharing from there is the same thing as
            // permuting first and resharing in place.
            let dest = perm[j];
            for e in 0..elements {
                let base = e * self.n;
                // One counter per reshared element: every member of `recomb` draws
                // at this counter under its OWN key, so their polynomials are
                // independent of each other and of every other element.
                let ctr = self.next_counter()?;
                for (ri, &party) in recomb.iter().enumerate() {
                    let additive = lambdas[ri].mul(entry[base + party as usize - 1].y);
                    let sub = self.reshare_from_private_randomness(party, ctr, additive);
                    for (p, y) in sub.into_iter().enumerate() {
                        out[dest][base + p].y = out[dest][base + p].y.add(y);
                    }
                }
            }
        }
        Ok(out)
    }

    /// The recombination set of round `a`: the `t+1` LOWEST-numbered members of
    /// `S_A = parties \ A`.
    ///
    /// Excluding `A` is not a detail — it is the whole security argument. A party
    /// in `A` does not hold `k_A`, so it cannot know `pi_A`; letting it
    /// contribute to the round would mean the permutation was applied by someone
    /// who can see where its own additive share went. Honest majority gives
    /// `|S_A| = n − t >= t + 1`, so the `t+1` members always exist; taking the
    /// lowest is the canonical choice, and for even `n` (`|S_A| = t + 2`) it
    /// matters that the Lagrange weights are computed for exactly these `t+1`
    /// points and not for the whole of `S_A`.
    fn recombination_group(&self, a: usize) -> Vec<u64> {
        let group: Vec<u64> = (1..=self.n as u64)
            .filter(|p| !self.seeds[a].set.contains(p))
            .collect();
        debug_assert!(group.len() > self.t, "honest majority gives n - t >= t + 1");
        group[..self.t + 1].to_vec()
    }

    /// Party `party`'s fresh degree-`t` Shamir sharing of `value`, as the `n`
    /// `y`-values on the canonical points, with the masking coefficients drawn
    /// from THAT PARTY'S OWN private key ([`Self::private_keys`]) — never from a
    /// replicated seed, which the corrupt set may hold.
    fn reshare_from_private_randomness(&self, party: u64, ctr: u64, value: Fp) -> Vec<Fp> {
        let mut stream = prf_stream(
            PRF_SHUFFLE_RESHARE_DOMAIN,
            &self.private_keys[party as usize - 1],
            ctr,
        );
        let mut coeffs = Vec::with_capacity(self.t + 1);
        coeffs.push(value);
        for _ in 0..self.t {
            coeffs.push(stream.next_fp());
        }
        (1..=self.n as u64)
            .map(|x| eval_poly(&coeffs, Fp::new(x)))
            .collect()
    }
}

/// An **unbiased** uniform permutation of `0..n` by Fisher-Yates over `stream`,
/// each index chosen uniformly from the remaining range by
/// [`uniform_below_stream`]'s rejection sampling (no modulo bias).
///
/// Fails closed for `n` above the field modulus `P`, which the rejection sampler
/// cannot serve unbiasedly — unreachable for a real column, guarded anyway.
fn fisher_yates(stream: &mut PrfStream, n: usize) -> Result<Vec<usize>, MpcError> {
    let p = crate::field::P;
    if n as u128 > p as u128 {
        return Err(MpcError::Protocol(format!(
            "PRSS resharing shuffle: column length {n} exceeds field modulus P = {p}; \
             cannot sample an unbiased permutation"
        )));
    }
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = uniform_below_stream(stream, (i + 1) as u64)? as usize;
        perm.swap(i, j);
    }
    Ok(perm)
}

/// A uniform integer in `[0, bound)` from `stream`, rejection-sampled so the
/// modular reduction is unbiased. Requires `1 <= bound <= P`: the sampler draws
/// uniform field elements in `[0, P)`, so a larger `bound` would make the
/// rejection limit `0` and the loop would never terminate — it is refused instead.
fn uniform_below_stream(stream: &mut PrfStream, bound: u64) -> Result<u64, MpcError> {
    let p = crate::field::P;
    if bound == 0 || bound > p {
        return Err(MpcError::Protocol(format!(
            "PRSS resharing shuffle: bound {bound} is not in 1..={p}; cannot \
             rejection-sample without bias"
        )));
    }
    if bound == 1 {
        return Ok(0);
    }
    let limit = p - (p % bound);
    loop {
        let v = stream.next_fp().value();
        if v < limit {
            return Ok(v % bound);
        }
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

/// `PRF(key, ctr)` as a uniform field element — one element off the
/// [`PRF_DOMAIN`] stream, uniform over `F_p` with no modulo bias (the rejection
/// sampler in [`MpcRng::next_fp`]). See [`prf_stream`] for the derivation.
fn prf_fp(key: &[u8; 32], ctr: u64) -> Fp {
    prf_stream(PRF_DOMAIN, key, ctr).next_fp()
}

/// The keystream `PRF(domain, key, ctr)` — the shared mechanism behind
/// [`prf_fp`] and the shuffle's two derivations.
///
/// A domain-separated SHA-512 **key derivation** — `SHA-512(domain ‖ key ‖ ctr)`,
/// with `key` and `ctr` fixed-length so no two distinct `(key, ctr)` pairs share
/// a preimage — keys a fresh ChaCha20 stream ([`crate::chacha`], the same vetted
/// generator the masking CSPRNG uses), from which uniform field elements are
/// drawn by [`MpcRng::next_fp`]'s rejection sampler (uniform over `F_p`, no
/// modulo bias).
///
/// `domain` separates the uses, so the same `(key, ctr)` never produces both a
/// mask summand and a permutation. Deriving a fresh stream per `(domain, key,
/// ctr)` keeps every counter's output independent by construction, so a
/// rejection-sampling redraw can never bleed into the next counter's value. The
/// derived key is never exposed, and both it and the digest are scrubbed before
/// this returns.
fn prf_stream(domain: &[u8], key: &[u8; 32], ctr: u64) -> PrfStream {
    let mut hasher = Sha512::new();
    hasher.update(domain);
    hasher.update(key);
    hasher.update(ctr.to_le_bytes());
    let mut digest = hasher.finalize();

    let mut stream_key = Zeroizing::new([0u8; 32]);
    stream_key.copy_from_slice(&digest[..32]);
    // The digest is derived secret material; scrub the buffer we own.
    digest[..].zeroize();

    PrfStream {
        inner: ChaCha20Csprng::from_seed(*stream_key),
    }
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

    fn parties(&self) -> usize {
        PrssRandomness::parties(self)
    }

    fn threshold(&self) -> usize {
        PrssRandomness::threshold(self)
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

    /// A **dealer-less composed resharing shuffle** — the one thing in this module
    /// that permutes rather than generates, and the only `DistributedRandomness`
    /// method here that needs no central sampler.
    ///
    /// ## The construction
    ///
    /// For every maximal unqualified set `A` — the `t`-subsets of the party
    /// points, in the same canonical lexicographic order as the seeds themselves —
    /// one resharing round runs:
    ///
    /// 1. The group `S_A = parties \ A` derives a permutation `pi_A` of the
    ///    column's positions from the seed `k_A` — a domain-separated PRF stream
    ///    (`PRF_SHUFFLE_PERM_DOMAIN`), consumed by Fisher-Yates with rejection
    ///    sampling, so `pi_A` is uniform over all `m!` permutations and unbiased.
    /// 2. The `t+1` lowest-numbered members of `S_A` (honest majority gives
    ///    `|S_A| = n − t >= t + 1`) convert each element's degree-`t` sharing to an
    ///    **additive** sharing among themselves, by the public Lagrange-at-0
    ///    weights for their own `t+1` points.
    /// 3. Each of them permutes its additive shares by `pi_A`.
    /// 4. Each **reshares** its permuted additive share as a fresh degree-`t`
    ///    Shamir sharing to all `n` parties, with masking coefficients drawn from
    ///    **its own private key** (`private_keys`) — never from a
    ///    replicated seed, which the corrupt set may hold.
    /// 5. The `t+1` resharings are summed componentwise, giving the new degree-`t`
    ///    sharing of the permuted element.
    ///
    /// The composed permutation is `pi = pi_{A_k} ∘ … ∘ pi_{A_1}` (round `A_1`
    /// first).
    ///
    /// ## Security model (read this before relying on it)
    ///
    /// **Semi-honest, honest majority (`n >= 2t+1`), at most `t` corrupted.** For
    /// the actual corrupt set `C` with `|C| <= t`, extend `C` to a maximal
    /// unqualified set `A ⊇ C`. By the replicated-seed rule `k_A` is held by
    /// exactly the parties *outside* `A`, so **no party in `A` — and therefore no
    /// party in `C` — can evaluate `k_A`'s PRF**. Hence `pi_A` is pseudorandom and
    /// uniform from the adversary's view, and the composed `pi` is uniform from
    /// that view too (composing a uniform, view-independent permutation with
    /// anything the adversary may know leaves the result uniform). Public coins are
    /// never used to choose the permutation; nothing about `pi` is ever opened.
    /// Like the rest of this module the argument is **computational** (it rests on
    /// the PRF), one step down from the information-theoretic Shamir core.
    ///
    /// **NOT malicious-secure.** Nothing verifies that a group applied a
    /// permutation at all: a deviating member can reshare a value it did not hold,
    /// or place an additive share at the wrong position, and corrupt or reorder the
    /// outputs undetected. The `checked_oblivious_shuffle` shape check on the
    /// consumer side is a SHAPE check only. Verified shuffling needs a proof of
    /// permutation (or IT-MAC-authenticated resharing) and is not built here.
    ///
    /// **Ordered delivery is a transport obligation.** Each round's resharings are
    /// produced here by a plain loop over the input entries; in-process that is
    /// invisible, but over real private channels the batches MUST go out in
    /// **output-position order**. Attaching a source index or a per-element
    /// counter to a batch, or emitting the batches in input order so their timing
    /// tracks it, would hand the adversary `pi` directly — the one thing the
    /// construction hides. There is no wire format for this shuffle yet, so
    /// nothing enforces it today; it is an obligation on the network / transport
    /// layer when that lands.
    ///
    /// **The returned sharings are FRESH** whenever a round actually runs
    /// (`m >= 2`). Step 4 re-randomises every element, so an output share will not
    /// equal an input share even though each entry reconstructs to the same tuple
    /// — that is what makes the output unlinkable to the input. A column with
    /// `m <= 1` has nothing to permute and is returned UNCHANGED, shares included.
    /// Either way a caller must not match entries by share equality (the seam
    /// documents this as part of the method's contract).
    ///
    /// **Still not deployable.** [`RandomnessModel::Prss`] remains
    /// `deployable() == false`: the replicated-seed SETUP is a simulated one-time
    /// trusted setup, the whole simulation runs in one process and therefore holds
    /// every seed and every private key, and the crate is externally unaudited
    /// (`sq-qhy4` pending).
    ///
    /// ## Cost
    ///
    /// `C(n, t)` resharing rounds — one per maximal unqualified set — reported as
    /// [`ShuffleCost::depth_rounds`]. Each round reshares every element of every
    /// entry from `t+1` parties, so the work is `C(n, t) · m · elements · (t+1)`
    /// resharings. No switch network is routed, so
    /// [`ShuffleCost::switches`] is `0`. That round count is the same
    /// combinatorial blow-up as the seed count, which is why this module is a
    /// SMALL-`n` tool and why [`MAX_PRSS_SEEDS`] caps the setup at all.
    ///
    /// # Errors
    ///
    /// [`MpcError::Protocol`] if the column is not whole `n`-party sharings on the
    /// canonical points, or if ANY element of ANY entry is not a consistent
    /// degree-`<= t` sharing (a degree-`2t` element must be degree-reduced first —
    /// see the degree section above). Both refusals land before `pi` or any
    /// resharing randomness is drawn, so a refused call consumes no PRF counter
    /// and returns no partial output. See `resharing_shuffle`.
    fn oblivious_shuffle(
        &mut self,
        col: SecretColumn,
    ) -> Result<(SecretColumn, ShuffleCost), MpcError> {
        self.resharing_shuffle(col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shamir::ShamirBackend;
    use std::collections::{BTreeMap, BTreeSet};

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

    // ---- The dealer-less composed resharing shuffle (#6117 step 5) ----

    /// Build a column of `m` entries, each packing `elements` secret values as
    /// consecutive whole `n`-party degree-`t` sharings — the layout
    /// `crate::oblivious_join` packs and the only one the resharing shuffle can
    /// interpret.
    fn shared_column(backend: &ShamirBackend, rows: &[Vec<u64>]) -> crate::oblivious::SecretColumn {
        let mut dealer = backend.dealer();
        rows.iter()
            .map(|row| {
                let mut entry = Vec::new();
                for v in row {
                    entry.extend(dealer.share(Fp::new(*v)));
                }
                entry
            })
            .collect()
    }

    /// **Exact** consistency check with NO error correction: interpolate through
    /// the first `degree+1` shares and require EVERY share to lie on that
    /// polynomial; return `P(0)`.
    ///
    /// This pins degree **<= `degree`**, not *exactly* `degree`: a lower-degree
    /// sharing lies on its own degree-`degree` interpolant and is accepted. To pin
    /// FULL degree `d`, pair `exact_open(_, d).is_ok()` with
    /// `exact_open(_, d - 1).is_err()`, as `open_column` does.
    ///
    /// Deliberately NOT `ShamirBackend::reconstruct`, which is the robust open: it
    /// CORRECTS up to `e` tampered shares, so it would pass a tampered or
    /// wrong-degree sharing this crate's own rounds must never produce.
    fn exact_open(shares: &[Share], degree: usize) -> Result<Fp, String> {
        let base: Vec<u64> = shares[..degree + 1].iter().map(|sh| sh.x).collect();
        let at = |z: Fp| -> Fp {
            lagrange_weights_at(&base, z)
                .iter()
                .zip(shares)
                .fold(Fp::zero(), |acc, (w, sh)| acc.add(w.mul(sh.y)))
        };
        for sh in shares {
            if at(Fp::new(sh.x)) != sh.y {
                return Err(format!(
                    "share at x = {} is off the degree-{degree} polynomial",
                    sh.x
                ));
            }
        }
        Ok(at(Fp::zero()))
    }

    /// Reconstruct a column back to its cleartext rows, asserting on the way that
    /// every element is a consistent sharing of degree exactly `t` — consistent at
    /// `t` via [`exact_open`], and NOT consistent at `t - 1`, which is what rules
    /// a degree-collapsing round out. A round that produced a wrong-degree,
    /// lower-degree or inconsistent sharing fails here rather than returning a
    /// plausible number.
    fn open_column(
        backend: &ShamirBackend,
        col: &crate::oblivious::SecretColumn,
        elements: usize,
    ) -> Vec<Vec<u64>> {
        let n = backend.parties();
        let t = backend.threshold();
        col.iter()
            .enumerate()
            .map(|(j, entry)| {
                (0..elements)
                    .map(|e| {
                        let el = &entry[e * n..(e + 1) * n];
                        let v = exact_open(el, t)
                            .unwrap_or_else(|why| panic!("entry {j} element {e}: {why}"))
                            .value();
                        // Consistent at t is only half of "degree t": a sharing of
                        // degree BELOW t is consistent at t too, and a round that
                        // collapsed the degree would slip through.
                        assert!(
                            t == 0 || exact_open(el, t - 1).is_err(),
                            "entry {j} element {e}: degree is below t = {t}, not exactly t"
                        );
                        v
                    })
                    .collect()
            })
            .collect()
    }

    /// Every `k`-subset of the party points `1..=n`, as a corrupt-set enumeration.
    fn subsets(n: usize, k: usize) -> Vec<Vec<u64>> {
        (0u32..(1 << n))
            .filter(|mask| mask.count_ones() as usize == k)
            .map(|mask| {
                (1..=n as u64)
                    .filter(|i| mask & (1 << (i - 1)) != 0)
                    .collect()
            })
            .collect()
    }

    /// An INDEPENDENT Fisher-Yates over the PRF stream — re-implemented here on
    /// purpose rather than calling [`fisher_yates`], so the composition pin below
    /// does not merely mirror whatever the implementation does.
    fn expected_permutation(key: &[u8; 32], ctr: u64, m: usize) -> Vec<usize> {
        let mut stream = prf_stream(PRF_SHUFFLE_PERM_DOMAIN, key, ctr);
        let p = crate::field::P;
        let mut perm: Vec<usize> = (0..m).collect();
        for i in (1..m).rev() {
            let bound = (i + 1) as u64;
            let limit = p - p % bound;
            let j = loop {
                let v = stream.next_fp().value();
                if v < limit {
                    break (v % bound) as usize;
                }
            };
            perm.swap(i, j);
        }
        perm
    }

    /// The opened single-element order a source produces for `rows`, with the
    /// adversary's whole view pinned to `reference`: every seed the corrupt set
    /// `corrupt` HOLDS (a seed `k_B` with some member of `C` outside `B`) and
    /// every party's private key are copied from `reference`, so only the seeds
    /// `k_A` with `A` containing `C` — the ones no member of `C` can evaluate —
    /// are left varying with `seed`.
    fn order_under_pinned_adversary_view(
        backend: &ShamirBackend,
        reference: &PrssRandomness,
        corrupt: &[u64],
        seed: u64,
        rows: &[Vec<u64>],
    ) -> Vec<u64> {
        let n = backend.parties();
        let mut source = PrssRandomness::with_seeded_setup(n, seed).unwrap();
        for (i, entry) in source.seeds.iter_mut().enumerate() {
            if corrupt.iter().any(|c| !entry.set.contains(c)) {
                entry.key = reference.seeds[i].key.clone();
            }
        }
        source.private_keys = reference.private_keys.clone();
        let (out, _) = source
            .oblivious_shuffle(shared_column(backend, rows))
            .unwrap();
        open_column(backend, &out, 1)
            .into_iter()
            .map(|row| row[0])
            .collect()
    }

    fn sorted(mut rows: Vec<Vec<u64>>) -> Vec<Vec<u64>> {
        rows.sort();
        rows
    }

    /// The load-bearing correctness invariant: the shuffled column reconstructs to
    /// a PERMUTATION of the input multiset — no value altered, dropped or
    /// duplicated by the `C(n, t)` rounds of Lagrange conversion + resharing.
    /// Checked over several setup seeds and both honest-majority shapes the design
    /// record names, `(n, t) = (3, 1)` and `(5, 2)`.
    #[test]
    fn the_resharing_shuffle_output_is_a_permutation_of_the_input_multiset() {
        for n in [3usize, 5] {
            let backend = ShamirBackend::new_seeded(n, 0xA11CE).unwrap();
            let rows: Vec<Vec<u64>> = (0..6).map(|k| vec![100 + k, 900 - k]).collect();
            for seed in [1u64, 2, 3, 0xFEED, 0xC0FFEE] {
                let mut source = PrssRandomness::with_seeded_setup(n, seed).unwrap();
                assert_eq!(source.threshold(), (n - 1) / 2);
                let col = shared_column(&backend, &rows);
                let (shuffled, cost) = source.oblivious_shuffle(col).unwrap();
                assert_eq!(shuffled.len(), rows.len());
                assert_eq!(cost.items, rows.len());
                // No switch network is routed; the cost is the resharing rounds,
                // one per maximal unqualified set.
                assert_eq!(cost.switches, 0);
                assert_eq!(cost.depth_rounds, source.seed_count());
                assert_eq!(
                    sorted(open_column(&backend, &shuffled, 2)),
                    sorted(rows.clone()),
                    "n = {n}, seed = {seed}"
                );
            }
        }
    }

    /// A packed entry must stay together: the elements of one entry are permuted
    /// as a UNIT, never independently. Each row carries the binding
    /// `second == first * 1000 + 7`, which every output row must still satisfy —
    /// a per-element permutation would scramble the pairs and break it.
    #[test]
    fn a_packed_entry_keeps_its_element_to_element_binding() {
        for n in [3usize, 5] {
            let backend = ShamirBackend::new_seeded(n, 0xB0B).unwrap();
            let rows: Vec<Vec<u64>> = (1..=5u64)
                .map(|k| vec![k, k * 1000 + 7, k * 1_000_000 + 11])
                .collect();
            for seed in [7u64, 8, 9] {
                let mut source = PrssRandomness::with_seeded_setup(n, seed).unwrap();
                let (shuffled, _) = source
                    .oblivious_shuffle(shared_column(&backend, &rows))
                    .unwrap();
                for row in open_column(&backend, &shuffled, 3) {
                    assert_eq!(row[1], row[0] * 1000 + 7, "n = {n}, seed = {seed}: {row:?}");
                    assert_eq!(
                        row[2],
                        row[0] * 1_000_000 + 11,
                        "n = {n}, seed = {seed}: {row:?}"
                    );
                }
            }
        }
    }

    /// The shuffle must actually REORDER: over many setup seeds a small column
    /// must come out in more than one order. A construction that returned the
    /// input (or applied one fixed permutation) would hide nothing.
    #[test]
    fn the_resharing_shuffle_does_not_produce_one_fixed_order() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0D1).unwrap();
        let rows: Vec<Vec<u64>> = (0..4).map(|k| vec![10 + k]).collect();
        let mut orders: BTreeSet<Vec<Vec<u64>>> = BTreeSet::new();
        for seed in 0..24u64 {
            let mut source = PrssRandomness::with_seeded_setup(n, seed).unwrap();
            let (shuffled, _) = source
                .oblivious_shuffle(shared_column(&backend, &rows))
                .unwrap();
            orders.insert(open_column(&backend, &shuffled, 1));
        }
        assert!(
            orders.len() > 1,
            "24 setups produced one single order: {orders:?}"
        );
    }

    /// Two successive shuffles from the SAME source must not reuse a permutation:
    /// each round draws a fresh counter, so the second shuffle is independent of
    /// the first.
    #[test]
    fn successive_shuffles_from_one_source_do_not_repeat_the_permutation() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0D2).unwrap();
        let rows: Vec<Vec<u64>> = (0..8).map(|k| vec![20 + k]).collect();
        let mut distinct: BTreeSet<Vec<Vec<u64>>> = BTreeSet::new();
        for seed in 0..6u64 {
            let mut source = PrssRandomness::with_seeded_setup(n, seed).unwrap();
            for _ in 0..2 {
                let (shuffled, _) = source
                    .oblivious_shuffle(shared_column(&backend, &rows))
                    .unwrap();
                distinct.insert(open_column(&backend, &shuffled, 1));
            }
        }
        assert!(distinct.len() > 1, "every shuffle produced the same order");
    }

    /// **The security property, exercised.** Give the adversary every seed it
    /// actually holds and vary ONLY the seeds it does not: `pi` must still move.
    ///
    /// The corrupt set `C` holds `k_B` exactly when some member of `C` is outside
    /// `B` — i.e. for every `B` that does NOT contain `C`. The seeds it cannot
    /// evaluate are precisely those `A ⊇ C`. So we build variants that agree on
    /// every `B ⊉ C` (the adversary's whole seed view) and on every party's
    /// private key, and differ only on the `A ⊇ C` seeds. The output ORDER over
    /// those variants is not constant, so the adversary's knowledge does not fix
    /// `pi`: it depends on the `r_A` for some `A ⊇ C`.
    #[test]
    fn the_corrupt_sets_own_seeds_do_not_fix_the_permutation() {
        let n = 5;
        let t = 2;
        // A corrupt set of the maximum tolerated size.
        let corrupt: Vec<u64> = vec![2, 4];
        assert_eq!(corrupt.len(), t);
        let backend = ShamirBackend::new_seeded(n, 0x0D3).unwrap();
        let rows: Vec<Vec<u64>> = (0..5).map(|k| vec![30 + k]).collect();

        // The adversary's fixed view: one reference setup supplies every seed it
        // holds, plus every party's private key.
        let reference = PrssRandomness::with_seeded_setup(n, 0xADDE45).unwrap();
        let mut orders: BTreeSet<Vec<Vec<u64>>> = BTreeSet::new();
        let mut varied_any = false;
        for seed in 0..16u64 {
            let mut source = PrssRandomness::with_seeded_setup(n, seed).unwrap();
            assert_eq!(source.threshold(), t);
            for (i, seed_entry) in source.seeds.iter_mut().enumerate() {
                let known_to_corrupt = corrupt.iter().any(|c| !seed_entry.set.contains(c));
                if known_to_corrupt {
                    // Pin it to the adversary's view.
                    seed_entry.key = reference.seeds[i].key.clone();
                } else {
                    varied_any = true;
                }
            }
            // The honest parties' own resharing randomness is held fixed too, so
            // any change in the ORDER can only come from a `pi_A` with `A ⊇ C`.
            source.private_keys = reference.private_keys.clone();
            let (shuffled, _) = source
                .oblivious_shuffle(shared_column(&backend, &rows))
                .unwrap();
            orders.insert(open_column(&backend, &shuffled, 1));
        }
        assert!(
            varied_any,
            "the test must actually vary a seed the corrupt set cannot evaluate"
        );
        assert!(
            orders.len() > 1,
            "pi was fixed by the corrupt set's own view: {orders:?}"
        );
    }

    /// The shuffle reads individual shares (unlike a switch network, which only
    /// swaps whole entries), so a column it cannot parse as whole `n`-party
    /// sharings on the canonical points is REFUSED rather than re-interpreted.
    #[test]
    fn a_column_that_is_not_whole_canonical_sharings_is_refused() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0D4).unwrap();
        let rows: Vec<Vec<u64>> = (0..4).map(|k| vec![40 + k, 50 + k]).collect();

        // Ragged widths.
        let mut ragged = shared_column(&backend, &rows);
        ragged[1].pop();
        let mut source = PrssRandomness::with_seeded_setup(n, 1).unwrap();
        assert!(matches!(
            source.oblivious_shuffle(ragged),
            Err(MpcError::Protocol(_))
        ));

        // A width that is not a multiple of n (every entry equally short).
        let mut short = shared_column(&backend, &rows);
        for entry in short.iter_mut() {
            entry.truncate(n + 1);
        }
        let mut source = PrssRandomness::with_seeded_setup(n, 1).unwrap();
        assert!(matches!(
            source.oblivious_shuffle(short),
            Err(MpcError::Protocol(_))
        ));

        // Right width, wrong points: a permuted sharing would be read as a
        // different party's share of each element.
        let mut mispointed = shared_column(&backend, &rows);
        mispointed[0].swap(0, 1);
        let mut source = PrssRandomness::with_seeded_setup(n, 1).unwrap();
        assert!(matches!(
            source.oblivious_shuffle(mispointed),
            Err(MpcError::Protocol(_))
        ));
    }

    /// A column packed the way a select would pack one WITHOUT re-randomising the
    /// product: a degree-`2t` tag element (the `selector * real_tag` product)
    /// followed by degree-`t` payload limbs, in one entry. This is a fixture for
    /// the degree check, not what the oblivious output stage builds — that stage
    /// degree-reduces every selected element first.
    fn mixed_degree_column(
        backend: &ShamirBackend,
        rows: &[(u64, Vec<u64>)],
    ) -> crate::oblivious::SecretColumn {
        let mut dealer = backend.dealer();
        rows.iter()
            .map(|(sel, limbs)| {
                let selector = dealer.share(Fp::new(*sel));
                let real_tag = dealer.share(Fp::one());
                // A raw product, left un-reduced on purpose: degree 2t.
                let mut entry = crate::shamir::mul_shares_raw(&selector, &real_tag).unwrap();
                for limb in limbs {
                    entry.extend(dealer.share(Fp::new(*limb)));
                }
                entry
            })
            .collect()
    }

    /// A degree-`2t` element recombined from `t+1` points would come back as a
    /// well-formed degree-`t` sharing of a DIFFERENT secret, undetectable
    /// downstream. A mixed-degree packed entry is the shape that case arrives in,
    /// so that is what must be refused.
    #[test]
    fn a_mixed_degree_packed_column_is_refused() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0D7).unwrap();
        let rows = vec![
            (1u64, vec![11u64, 12]),
            (0, vec![21, 22]),
            (1, vec![31, 32]),
            (0, vec![41, 42]),
        ];
        let col = mixed_degree_column(&backend, &rows);
        // Sanity: the tag element really is degree 2t — it opens at 2t and NOT at t.
        let tag = &col[0][..n];
        assert_eq!(
            crate::shamir::reconstruct_degree(tag, 2 * backend.threshold())
                .unwrap()
                .value(),
            1,
            "the fixture's tag element must be the degree-2t product"
        );
        assert!(
            exact_open(tag, backend.threshold()).is_err(),
            "...and must NOT be a consistent degree-t sharing"
        );
        let mut source = PrssRandomness::with_seeded_setup(n, 1).unwrap();
        match source.oblivious_shuffle(col) {
            Err(MpcError::Protocol(m)) => assert!(
                m.contains("not a consistent") && m.contains("degree"),
                "wrong refusal: {m}"
            ),
            other => panic!("must fail closed, got {other:?}"),
        }
    }

    /// The same refusal for a column whose ONLY element is degree `2t` — the
    /// mixed layout is not what triggers it, the degree is.
    #[test]
    fn a_degree_2t_only_column_is_refused() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0D8).unwrap();
        let rows: Vec<(u64, Vec<u64>)> = (0..4).map(|k| (k % 2, Vec::new())).collect();
        let col = mixed_degree_column(&backend, &rows);
        assert_eq!(col[0].len(), n, "one element per entry");
        let mut source = PrssRandomness::with_seeded_setup(n, 1).unwrap();
        match source.oblivious_shuffle(col) {
            Err(MpcError::Protocol(msg)) => assert!(
                msg.contains("not a consistent") && msg.contains("degree"),
                "it must be the DEGREE refusal, not some other shape check: {msg}"
            ),
            other => panic!("must fail closed, got {other:?}"),
        }
    }

    /// Every refusal lands before `pi` or any resharing randomness is drawn, so a
    /// refused call must consume NO PRF counter: a source that refused one column
    /// and then shuffled a valid one must produce exactly what a fresh source with
    /// the same setup produces from the valid column alone.
    #[test]
    fn a_refused_shuffle_consumes_no_randomness() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0D9).unwrap();
        let rows: Vec<Vec<u64>> = (0..6).map(|k| vec![70 + k]).collect();
        let bad_rows: Vec<(u64, Vec<u64>)> = (0..6).map(|k| (k % 2, vec![9])).collect();

        let mut refused_then_valid = PrssRandomness::with_seeded_setup(n, 21).unwrap();
        for malformed in [
            mixed_degree_column(&backend, &bad_rows),
            // A shape refusal must be free too.
            {
                let mut ragged = shared_column(&backend, &rows);
                ragged[2].pop();
                ragged
            },
        ] {
            assert!(refused_then_valid.oblivious_shuffle(malformed).is_err());
        }
        let (after_refusals, _) = refused_then_valid
            .oblivious_shuffle(shared_column(&backend, &rows))
            .unwrap();

        let mut fresh = PrssRandomness::with_seeded_setup(n, 21).unwrap();
        let (clean, _) = fresh
            .oblivious_shuffle(shared_column(&backend, &rows))
            .unwrap();

        let ys = |col: &crate::oblivious::SecretColumn| -> Vec<u64> {
            col.iter().flatten().map(|sh| sh.y.value()).collect()
        };
        assert_eq!(
            ys(&after_refusals),
            ys(&clean),
            "a refused call must leave the counter (and so every later draw) untouched"
        );
    }

    /// The dealer's privacy shuffle refuses the SAME mixed-degree packed column the
    /// PRSS source refuses, and refuses it before any draw.
    ///
    /// The dealer path used to accept it, because a switch only moves whole entries
    /// and never interpolates one. It cannot accept it any more: the output is
    /// re-randomised with a degree-`t` zero-sharing, which refreshes the
    /// coefficients `x^1 .. x^t` and leaves anything above `x^t` as the input had
    /// it, so a degree-`2t` element would come back carrying a value-independent
    /// fingerprint of its input element. Refusing is the fail-closed answer; a
    /// caller with a product sharing degree-reduces it first, which the oblivious
    /// output stage does for every selected element. Both sources therefore now
    /// agree on what they accept.
    #[test]
    fn the_dealer_shuffle_refuses_the_mixed_degree_layout() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0DA).unwrap();
        let rows = vec![
            (1u64, vec![11u64, 12]),
            (0, vec![21, 22]),
            (1, vec![31, 32]),
            (0, vec![41, 42]),
        ];
        let col = mixed_degree_column(&backend, &rows);
        let mut dealer = backend.dealer();
        // A witness draw taken BEFORE and AFTER the refused call: a refusal must
        // not consume a field element, or the keystream would depend on a rejected
        // input.
        let mut probe = backend.dealer();
        let expected_next = probe.draw_fp();
        match dealer.oblivious_shuffle(col) {
            Err(MpcError::Protocol(m)) => assert!(
                m.contains("not a consistent") && m.contains("degree"),
                "wrong refusal: {m}"
            ),
            other => panic!("the dealer path must fail closed, got {other:?}"),
        }
        assert_eq!(
            dealer.draw_fp(),
            expected_next,
            "the refused call must have drawn nothing"
        );
        // The same column goes through once every element is degree-`t`: a plain
        // degree-`t` column of the same shape is accepted.
        let mut ok_dealer = backend.dealer();
        let plain: crate::oblivious::SecretColumn = rows
            .iter()
            .map(|(tag, limbs)| {
                let mut e = ok_dealer.share(Fp::new(*tag));
                for l in limbs {
                    e.extend(ok_dealer.share(Fp::new(*l)));
                }
                e
            })
            .collect();
        let (shuffled, cost) = ok_dealer.oblivious_shuffle(plain).unwrap();
        assert_eq!(shuffled.len(), rows.len());
        assert_eq!(cost.fresh_sharings, rows.len() * 3);
    }

    /// The PRSS construction's output is unlinkable to its input for the same
    /// reason, by a different mechanism: it permutes by RESHARING, so every output
    /// element is a fresh sharing already. A coalition of `t` parties looking for
    /// its own share values again in the output finds none of them.
    #[test]
    fn a_t_coalition_cannot_pair_prss_output_elements_with_input_elements() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0DB).unwrap();
        let coalition = [1u64, 2];
        assert_eq!(backend.threshold(), coalition.len());
        let view = |col: &crate::oblivious::SecretColumn| -> BTreeSet<Vec<u64>> {
            col.iter()
                .map(|e| {
                    e.iter()
                        .filter(|sh| coalition.contains(&sh.x))
                        .map(|sh| sh.y.value())
                        .collect()
                })
                .collect()
        };
        for seed in 0..8u64 {
            let mut dealer = backend.dealer();
            let col: crate::oblivious::SecretColumn =
                (0..4u64).map(|k| dealer.share(Fp::new(1000 + k))).collect();
            let before = view(&col);
            let mut source = PrssRandomness::with_seeded_setup(n, 0x0DC + seed).unwrap();
            let (after, cost) = source.oblivious_shuffle(col).unwrap();
            assert_eq!(cost.fresh_sharings, 4, "one fresh sharing per element");
            assert!(
                view(&after).is_disjoint(&before),
                "seed {seed}: an output element carried an input element's coalition shares"
            );
        }
    }

    #[test]
    fn at_threshold_zero_the_prss_shuffle_counts_no_fresh_sharings() {
        // n = 2 gives t = 0: a resharing draws no masking coefficient, so the
        // shuffle only reorders share vectors and must not report them as fresh.
        let n = 2;
        let backend = ShamirBackend::new_seeded(n, 0x0E1).unwrap();
        assert_eq!(backend.threshold(), 0);
        let mut dealer = backend.dealer();
        let col: crate::oblivious::SecretColumn =
            (0..4u64).map(|k| dealer.share(Fp::new(2000 + k))).collect();
        let mut source = PrssRandomness::with_seeded_setup(n, 0x0E2).unwrap();
        let (after, cost) = source.oblivious_shuffle(col).unwrap();
        assert_eq!(after.len(), 4);
        assert_eq!(cost.fresh_sharings, 0, "t = 0 refreshes nothing");
    }

    // ---- The shuffle's SECURITY properties, each pinned against a weakening
    // ---- that the correctness tests above let through (#7079 round-3 review).

    /// Every round must use ITS OWN seed `k_A`, so that for **every** maximal
    /// corrupt set `C` the permutation still varies when only the seeds `C` cannot
    /// evaluate change. Pinning the adversary's whole view — every seed it holds
    /// plus every party's private key — must leave `pi` moving, for every
    /// `t`-subset `C`, at odd AND even `n`.
    ///
    /// A source that drew every round's permutation from one fixed seed (or from a
    /// public value) fails here as soon as `C` is a set that holds that seed.
    #[test]
    fn every_maximal_corrupt_set_leaves_the_permutation_varying() {
        for n in 3usize..=7 {
            let t = (n - 1) / 2;
            let backend = ShamirBackend::new_seeded(n, 0x5E01).unwrap();
            let reference = PrssRandomness::with_seeded_setup(n, 0xFEE1).unwrap();
            let rows: Vec<Vec<u64>> = (0..4).map(|k| vec![k]).collect();
            for corrupt in subsets(n, t) {
                let mut seen: BTreeSet<Vec<u64>> = BTreeSet::new();
                for seed in 0..12u64 {
                    seen.insert(order_under_pinned_adversary_view(
                        &backend,
                        &reference,
                        &corrupt,
                        0x1_0000 + seed,
                        &rows,
                    ));
                }
                assert!(
                    seen.len() > 1,
                    "n = {n}, C = {corrupt:?}: pi is fixed by the corrupt set's own view"
                );
            }
        }
    }

    /// Two calls on ONE source must use FRESH permutations — a per-round counter
    /// that is not advanced (e.g. the round index) would replay the same `pi` on
    /// every call. Asserted pairwise over several setups and both parities of `n`,
    /// on `m = 8` entries (`8! = 40320` orders), so a chance coincidence is
    /// negligible.
    #[test]
    fn two_calls_on_one_source_use_fresh_permutations() {
        for n in [3usize, 4, 5] {
            let backend = ShamirBackend::new_seeded(n, 0x5E02).unwrap();
            let rows: Vec<Vec<u64>> = (0..8).map(|k| vec![k]).collect();
            for seed in 0..4u64 {
                let mut source = PrssRandomness::with_seeded_setup(n, seed).unwrap();
                let (first, _) = source
                    .oblivious_shuffle(shared_column(&backend, &rows))
                    .unwrap();
                let (second, _) = source
                    .oblivious_shuffle(shared_column(&backend, &rows))
                    .unwrap();
                assert_ne!(
                    open_column(&backend, &first, 1),
                    open_column(&backend, &second, 1),
                    "n = {n}, seed = {seed}: the same pi was applied twice"
                );
            }
        }
    }

    /// No resharing masking polynomial may EVER be reused — not across the
    /// elements of one entry, not across entries, not across rounds, not across
    /// calls. Each output element's mask vector (`share - secret` at every point)
    /// must be distinct; a shared counter anywhere would collide them.
    #[test]
    fn no_resharing_mask_is_ever_reused() {
        for n in [3usize, 4, 5] {
            let t = (n - 1) / 2;
            let backend = ShamirBackend::new_seeded(n, 0x5E03).unwrap();
            let rows: Vec<Vec<u64>> = (0..5).map(|k| vec![k, 100 + k]).collect();
            let mut source = PrssRandomness::with_seeded_setup(n, 3).unwrap();
            let mut masks: BTreeSet<Vec<u64>> = BTreeSet::new();
            let mut total = 0usize;
            for _call in 0..2 {
                let (out, _) = source
                    .oblivious_shuffle(shared_column(&backend, &rows))
                    .unwrap();
                for entry in &out {
                    for element in entry.chunks(n) {
                        let secret = exact_open(element, t).expect("degree t");
                        masks.insert(element.iter().map(|sh| sh.y.sub(secret).value()).collect());
                        total += 1;
                    }
                }
            }
            assert_eq!(
                masks.len(),
                total,
                "n = {n}: a resharing masking polynomial was reused across {total} elements"
            );
        }
    }

    /// The counter layout and the composed permutation, pinned together: each
    /// round spends ONE counter on `pi_A` then one per reshared element, and the
    /// column ends up permuted by `pi_{A_k} o ... o pi_{A_1}` with each `pi_A`
    /// drawn from `k_A` at that round's own counter.
    ///
    /// The expected permutation is recomputed by an INDEPENDENT Fisher-Yates
    /// ([`expected_permutation`]), so dropping a round, reusing a counter, drawing
    /// from the wrong seed, or an off-by-one in the sampler all show up here. A
    /// few mask draws run first so the counter is not `0` at the start.
    #[test]
    fn the_counter_layout_and_composition_order_are_pinned() {
        for n in [3usize, 4, 5] {
            let backend = ShamirBackend::new_seeded(n, 0x5E04).unwrap();
            let (m, elements) = (6usize, 2usize);
            let rows: Vec<Vec<u64>> = (0..m)
                .map(|k| vec![10 + k as u64, 500 + k as u64])
                .collect();
            let mut source = PrssRandomness::with_seeded_setup(n, 0xC0 + n as u64).unwrap();
            for _ in 0..3 {
                source.shared_mask().unwrap();
            }
            for call in 0..2 {
                let before = source.counter;
                let (out, _) = source
                    .oblivious_shuffle(shared_column(&backend, &rows))
                    .unwrap();
                let rounds = source.seeds.len();
                let per_round = 1 + (m * elements) as u64;
                assert_eq!(
                    source.counter,
                    before + rounds as u64 * per_round,
                    "n = {n}, call = {call}: counter layout"
                );
                // Where each input entry should have ended up.
                let mut position: Vec<usize> = (0..m).collect();
                for a in 0..rounds {
                    let pi = expected_permutation(
                        &source.seeds[a].key,
                        before + a as u64 * per_round,
                        m,
                    );
                    for p in position.iter_mut() {
                        *p = pi[*p];
                    }
                }
                let opened = open_column(&backend, &out, elements);
                for (j, row) in rows.iter().enumerate() {
                    assert_eq!(
                        &opened[position[j]], row,
                        "n = {n}, call = {call}: input {j} is not at pi_k o .. o pi_1 (j)"
                    );
                }
            }
        }
    }

    /// The sampler must be a true Fisher-Yates, reaching **every** permutation of
    /// `m` items with roughly equal frequency. An off-by-one (Sattolo's algorithm,
    /// `uniform_below(i)` instead of `uniform_below(i + 1)`) yields only full
    /// cycles — it never returns the identity — so the reachable-count assertion
    /// below goes red on it, and a biased sampler moves the chi-square.
    #[test]
    fn fisher_yates_reaches_every_permutation_about_uniformly() {
        for (m, trials, bound) in [(2usize, 4000u64, 25.0f64), (3, 6000, 30.0)] {
            let mut histogram: BTreeMap<Vec<usize>, u64> = BTreeMap::new();
            for i in 0..trials {
                let mut key = [0u8; 32];
                key[..8].copy_from_slice(&i.to_le_bytes());
                let mut stream = prf_stream(PRF_SHUFFLE_PERM_DOMAIN, &key, 3);
                *histogram
                    .entry(fisher_yates(&mut stream, m).unwrap())
                    .or_insert(0) += 1;
            }
            let orders: usize = (1..=m).product();
            assert_eq!(
                histogram.len(),
                orders,
                "m = {m}: only {} of {orders} permutations are reachable (an off-by-one \
                 sampler yields cycles only)",
                histogram.len()
            );
            let expected = trials as f64 / orders as f64;
            let chi2: f64 = histogram
                .values()
                .map(|&o| (o as f64 - expected).powi(2) / expected)
                .sum();
            assert!(chi2 < bound, "m = {m}: non-uniform, chi2 = {chi2:.1}");
        }
    }

    /// The recombination set of every round must be exactly the `t+1`
    /// LOWEST-numbered members of `S_A = parties \ A` — it must EXCLUDE `A`
    /// (a party in `A` does not hold `k_A`, so it must not help apply `pi_A`) and
    /// it must be `t+1` points, not `t` and not the whole of `S_A` (which for even
    /// `n` is `t+2`, and whose Lagrange weights would be the wrong linear map).
    #[test]
    fn the_recombination_group_excludes_a_and_is_the_t_plus_1_lowest_of_s_a() {
        for n in 3usize..=8 {
            let t = (n - 1) / 2;
            let source = PrssRandomness::with_seeded_setup(n, 0x5E05).unwrap();
            for a in 0..source.seeds.len() {
                let set = &source.seeds[a].set;
                let expected: Vec<u64> = (1..=n as u64)
                    .filter(|p| !set.contains(p))
                    .take(t + 1)
                    .collect();
                let got = source.recombination_group(a);
                assert_eq!(got.len(), t + 1, "n = {n}, A = {set:?}: wrong group size");
                assert!(
                    got.iter().all(|p| !set.contains(p)),
                    "n = {n}, A = {set:?}: the recombination group {got:?} includes a member of A"
                );
                assert_eq!(got, expected, "n = {n}, A = {set:?}");
            }
        }
    }

    /// Correctness at EVEN `n`, where `|S_A| = t + 2 > t + 1`: the Lagrange
    /// weights must be the ones for the `t+1` recombination points, not for the
    /// whole of `S_A`. With an odd `n` the two sets coincide, so only even `n`
    /// separates them — and getting it wrong silently changes every secret.
    #[test]
    fn even_n_recombines_over_exactly_the_t_plus_1_points() {
        for n in [4usize, 6, 8] {
            let backend = ShamirBackend::new_seeded(n, 0x5E06).unwrap();
            let rows: Vec<Vec<u64>> = (0..5).map(|k| vec![10 + k, 20 + k]).collect();
            let mut source = PrssRandomness::with_seeded_setup(n, 6).unwrap();
            assert_eq!(
                source.recombination_group(0).len() + 1,
                (1..=n as u64)
                    .filter(|p| !source.seeds[0].set.contains(p))
                    .count(),
                "n = {n}: even n must have one more group member than recombination point"
            );
            let (out, _) = source
                .oblivious_shuffle(shared_column(&backend, &rows))
                .unwrap();
            assert_eq!(
                sorted(open_column(&backend, &out, 2)),
                sorted(rows.clone()),
                "n = {n}"
            );
        }
    }

    /// Output sharings must be EXACTLY degree `t`, never lower: a degree-`t-1`
    /// resharing would let `t` parties reconstruct, collapsing the privacy
    /// threshold. Checked by exact interpolation in both directions — degree `t`
    /// must fit every point, degree `t-1` must NOT.
    ///
    /// This deliberately avoids `ShamirBackend::reconstruct`, whose robust decoder
    /// corrects errors and accepts a lower degree, which is exactly the weakening
    /// under test.
    #[test]
    fn output_sharings_have_full_degree_t() {
        for n in [3usize, 4, 5, 7] {
            let t = (n - 1) / 2;
            let backend = ShamirBackend::new_seeded(n, 0x5E07).unwrap();
            let rows: Vec<Vec<u64>> = (0..4).map(|k| vec![k, 50 + k]).collect();
            let mut source = PrssRandomness::with_seeded_setup(n, 8).unwrap();
            let (out, _) = source
                .oblivious_shuffle(shared_column(&backend, &rows))
                .unwrap();
            for (j, entry) in out.iter().enumerate() {
                for (e, element) in entry.chunks(n).enumerate() {
                    assert!(
                        exact_open(element, t).is_ok(),
                        "n = {n}, entry {j} element {e}: not a degree-<= t sharing"
                    );
                    assert!(
                        exact_open(element, t - 1).is_err(),
                        "n = {n}, entry {j} element {e}: output sharing has degree < t, so t \
                         parties could reconstruct it"
                    );
                }
            }
        }
    }

    /// The degree check must cover EVERY element of EVERY entry, not just the
    /// first of either: a bad element is placed at each `(entry, element)`
    /// position in turn and must be refused, with no counter consumed.
    #[test]
    fn the_degree_check_covers_every_entry_and_every_element() {
        for n in [3usize, 5] {
            let t = (n - 1) / 2;
            let backend = ShamirBackend::new_seeded(n, 0x5E08).unwrap();
            let (m, elements) = (3usize, 3usize);
            let clean: Vec<Vec<u64>> = (0..m)
                .map(|k| {
                    (0..elements)
                        .map(|e| 1000 * k as u64 + 7 * e as u64 + 1)
                        .collect()
                })
                .collect();
            for bad_entry in 0..m {
                for bad_element in 0..elements {
                    let mut col = shared_column(&backend, &clean);
                    // Replace exactly one element with a degree-2t sharing.
                    let mut dealer = backend.dealer();
                    let a = dealer.share(Fp::new(3));
                    let b = dealer.share(Fp::new(5));
                    let product = crate::shamir::mul_shares_raw(&a, &b).unwrap();
                    assert!(
                        exact_open(&product, t).is_err(),
                        "fixture must be degree > t"
                    );
                    col[bad_entry][bad_element * n..(bad_element + 1) * n]
                        .clone_from_slice(&product);

                    let mut source = PrssRandomness::with_seeded_setup(n, 4).unwrap();
                    source.shared_mask().unwrap();
                    let before = source.counter;
                    match source.oblivious_shuffle(col) {
                        Err(MpcError::Protocol(msg)) => assert!(
                            msg.contains("not a consistent"),
                            "n = {n} ({bad_entry}, {bad_element}): wrong refusal: {msg}"
                        ),
                        other => panic!(
                            "n = {n} ({bad_entry}, {bad_element}): must fail closed, got {other:?}"
                        ),
                    }
                    assert_eq!(
                        source.counter, before,
                        "n = {n} ({bad_entry}, {bad_element}): the refusal consumed a counter"
                    );
                }
            }
        }
    }

    /// The degree check must test EVERY redundant point, not just the first.
    ///
    /// `H(x) = c + prod_{k=1..2t} (x - k)` has degree `2t` yet equals `c` at
    /// `x = 1..2t`, so the degree-`t` interpolant through `x = 1..t+1` is the
    /// constant `c` and extrapolates CORRECTLY to every point except the last,
    /// `x = n = 2t+1`. A check that stopped after one redundant point would accept
    /// it and silently reshare `c` in place of `H(0)`.
    #[test]
    fn the_degree_check_uses_every_redundant_point() {
        for n in [3usize, 5, 7, 9] {
            let t = (n - 1) / 2;
            let c = Fp::new(42);
            let h = |x: u64| -> Fp {
                (1..=(2 * t) as u64)
                    .fold(Fp::one(), |acc, k| acc.mul(Fp::new(x).sub(Fp::new(k))))
                    .add(c)
            };
            let bad: Vec<Share> = (1..=n as u64).map(|x| Share { x, y: h(x) }).collect();
            assert!(
                bad[..n - 1].iter().all(|sh| sh.y == c),
                "n = {n}: the fixture must differ from the interpolant ONLY at x = n"
            );
            assert_ne!(h(0), c, "n = {n}: ...and must hide a different secret");
            let backend = ShamirBackend::new_seeded(n, 0x5E09).unwrap();
            let good = backend.dealer().share(Fp::new(7));
            let mut source = PrssRandomness::with_seeded_setup(n, 2).unwrap();
            let before = source.counter;
            match source.oblivious_shuffle(vec![good, bad]) {
                Err(MpcError::Protocol(msg)) => {
                    assert!(msg.contains("not a consistent"), "n = {n}: {msg}")
                }
                other => panic!("n = {n}: crafted degree-2t sharing accepted: {other:?}"),
            }
            assert_eq!(
                source.counter, before,
                "n = {n}: the refusal consumed a counter"
            );
        }
    }

    /// The degree refusal must name the POSITION and nothing else. Two kinds of
    /// number are forbidden: any SHARE of the column, and the INTERPOLANT's value
    /// at any redundant point (the other side of the comparison that fails). Both
    /// are secret-shared data; at `n = 3`, `t = 1` either one plus a single
    /// party's own share reconstructs the secret.
    #[test]
    fn the_degree_refusal_leaks_no_share_value() {
        for n in [3usize, 5, 7] {
            let t = (n - 1) / 2;
            let secret_a = 123_456_789u64;
            let secret_b = 987_654_321u64;
            let backend = ShamirBackend::new_seeded(n, 0x5E0A).unwrap();
            let mut dealer = backend.dealer();
            let a = dealer.share(Fp::new(secret_a));
            let b = dealer.share(Fp::new(secret_b));
            // Degree 2t <= n - 1, so it IS a consistent sharing on the n points -
            // just of the wrong degree, which is exactly the hazard.
            let product = crate::shamir::mul_shares_raw(&a, &b).unwrap();
            let good = dealer.share(Fp::new(7));
            let mut forbidden: BTreeSet<u64> = product
                .iter()
                .chain(&a)
                .chain(&b)
                .chain(&good)
                .map(|sh| sh.y.value())
                .chain([secret_a, secret_b, secret_a.wrapping_mul(secret_b)])
                .collect();
            // The interpolant through the first t+1 shares, evaluated at every
            // redundant point: these are the values the check extrapolates and
            // compares against, and printing one of them leaks just as much.
            let base: Vec<u64> = (1..=t as u64 + 1).collect();
            for x in (t + 2)..=n {
                let weights = lagrange_weights_at(&base, Fp::new(x as u64));
                let extrapolated = weights
                    .iter()
                    .zip(&product)
                    .fold(Fp::zero(), |acc, (w, sh)| acc.add(w.mul(sh.y)));
                forbidden.insert(extrapolated.value());
            }
            // Every forbidden number is wide, so `contains` cannot match it by
            // accident inside an index or a threshold the message legitimately
            // carries (the fixtures are seeded, so this is deterministic).
            assert!(
                forbidden.iter().all(|v| *v > 1_000_000),
                "n = {n}: a forbidden value is too small to test by substring"
            );

            let mut source = PrssRandomness::with_seeded_setup(n, 5).unwrap();
            let Err(MpcError::Protocol(msg)) = source.oblivious_shuffle(vec![good, product]) else {
                panic!("n = {n}: the degree-2t column must be refused");
            };
            // The message names the position, which the caller needs.
            assert!(msg.contains("entry 1"), "n = {n}: {msg}");
            assert!(msg.contains("element 0"), "n = {n}: {msg}");
            for value in forbidden {
                assert!(
                    !msg.contains(&value.to_string()),
                    "n = {n}: the refusal leaked a secret value ({value}): {msg}"
                );
                assert!(
                    !msg.contains(&format!("{:x}", value)),
                    "n = {n}: the refusal leaked a secret value in hex ({:x}): {msg}",
                    value
                );
            }
        }
    }

    /// No member of `A` may take part in round `a`: the recombiners are `S_A`'s
    /// `t+1` lowest members, and a party in `A` (which does not hold `k_A`) must
    /// contribute nothing. Checked PER ROUND against the private keys, because the
    /// final column only reflects the last round's recombiners - so a round that
    /// recombines over the wrong parties is invisible end-to-end.
    ///
    /// Flipping a bit of a party's own resharing key must change round `a`'s output
    /// exactly when that party is a recombiner. This sees a round that bypasses
    /// `recombination_group`, which the group helper's own unit test cannot.
    #[test]
    fn no_member_of_a_takes_part_in_its_own_round() {
        for n in [3usize, 4, 5, 6] {
            let backend = ShamirBackend::new_seeded(n, 0x5E0B).unwrap();
            let rows: Vec<Vec<u64>> = (0..4).map(|k| vec![100 + k]).collect();
            let reference = PrssRandomness::with_seeded_setup(n, 9).unwrap();
            for a in 0..reference.seeds.len() {
                let set = reference.seeds[a].set.clone();
                // Round `a` alone, with one party's private resharing key flipped.
                // The RAW shares, not the opened values: resharing randomness
                // cancels on reconstruction, so only the share vectors can show
                // whose key was used.
                let round_with = |flip: Option<usize>| -> Vec<u64> {
                    let mut source = PrssRandomness::with_seeded_setup(n, 9).unwrap();
                    if let Some(party) = flip {
                        source.private_keys[party][0] ^= 1;
                    }
                    let col = shared_column(&backend, &rows);
                    let out = source
                        .resharing_round(a, rows.len(), 1, &col)
                        .expect("one round over a clean degree-t column");
                    // The round must still deliver the values it was given.
                    assert_eq!(
                        sorted(open_column(&backend, &out, 1)),
                        sorted(rows.clone()),
                        "n = {n}: a single round must permute, not alter, the values"
                    );
                    out.iter().flatten().map(|sh| sh.y.value()).collect()
                };
                let base = round_with(None);
                let recombiners = reference.recombination_group(a);
                for party in 1..=n as u64 {
                    let flipped = round_with(Some(party as usize - 1));
                    if set.contains(&party) {
                        assert_eq!(
                            flipped, base,
                            "n = {n}, A = {set:?}: party {party} is in A but took part in round A"
                        );
                    }
                    if recombiners.contains(&party) {
                        assert_ne!(
                            flipped, base,
                            "n = {n}, A = {set:?}: recombiner {party} did not mask with its OWN key"
                        );
                    }
                }
                // ...and the round really did depend on somebody outside A.
                assert!(
                    (1..=n as u64)
                        .filter(|p| !set.contains(p))
                        .any(|p| round_with(Some(p as usize - 1)) != base),
                    "n = {n}, A = {set:?}: no party outside A took part"
                );
            }
        }
    }

    /// **The reviewer's counterexample, pinned** (#7079 review). `n = 3`, `t = 1`.
    /// For the unqualified set `A = {1}` the recombination group is `{2, 3}`, whose
    /// Lagrange-at-0 weights are `(3, −2)`. Take `H(X) = (1 + X)^2`, shared at the
    /// party points as `H(1) = 4`, `H(2) = 9`, `H(3) = 16`: the secret is
    /// `H(0) = 1`, but `3·H(2) − 2·H(3) = 27 − 32 = −5`. A degree-`2t` sharing is
    /// not a degree-`t` one, and recombining it from `t+1` points yields a
    /// well-formed degree-`t` sharing of `−5` that nothing downstream could flag.
    /// The shuffle must refuse it, before drawing anything.
    #[test]
    fn the_degree_2t_counterexample_is_refused_without_drawing() {
        let n = 3;
        let t = 1;
        // The hazard, stated in the open: the group's own recombination of this
        // sharing is NOT the secret.
        let weights = lagrange_zero_weights(&[2, 3]);
        let naive = weights[0].mul(Fp::new(9)).add(weights[1].mul(Fp::new(16)));
        assert_eq!(weights, vec![Fp::new(3), Fp::new(2).neg()]);
        assert_eq!(naive, Fp::new(5).neg(), "3*H(2) - 2*H(3) = -5");
        assert_ne!(naive, Fp::one(), "...while the secret H(0) is 1");

        let bad: Vec<Share> = [(1u64, 4u64), (2, 9), (3, 16)]
            .into_iter()
            .map(|(x, y)| Share { x, y: Fp::new(y) })
            .collect();
        // A valid degree-t entry alongside it, so the refusal is about the degree
        // and not about the column being odd.
        let backend = ShamirBackend::new_seeded(n, 0x0DB).unwrap();
        let good = backend.dealer().share(Fp::new(42));
        let col: crate::oblivious::SecretColumn = vec![bad, good];

        let mut source = PrssRandomness::with_seeded_setup(n, 1).unwrap();
        assert_eq!(source.threshold(), t);
        let counter_before = source.counter;
        match source.oblivious_shuffle(col) {
            Err(MpcError::Protocol(m)) => assert!(
                m.contains("not a consistent") && m.contains("degree"),
                "wrong refusal: {m}"
            ),
            other => panic!("must fail closed, got {other:?}"),
        }
        assert_eq!(
            source.counter, counter_before,
            "a refused column must not have advanced the PRF counter"
        );
    }

    /// The resharing randomness must come from each member's OWN private key, not
    /// from a replicated seed the corrupt set may hold. Two setups with IDENTICAL
    /// seeds but different private keys therefore apply the SAME permutation (`pi`
    /// is derived from the seeds alone) while producing DIFFERENT share values —
    /// which is exactly the split the security argument relies on.
    #[test]
    fn the_resharing_randomness_comes_from_each_partys_own_private_key() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0D6).unwrap();
        let rows: Vec<Vec<u64>> = (0..6).map(|k| vec![60 + k]).collect();
        let mut a = PrssRandomness::with_seeded_setup(n, 11).unwrap();
        let mut b = PrssRandomness::with_seeded_setup(n, 12).unwrap();
        // Same replicated seeds, different private keys.
        b.seeds
            .iter_mut()
            .zip(&a.seeds)
            .for_each(|(dst, src)| dst.key = src.key.clone());
        assert!(
            b.private_keys != a.private_keys,
            "the two setups must differ in their private keys"
        );
        let (sa, _) = a.oblivious_shuffle(shared_column(&backend, &rows)).unwrap();
        let (sb, _) = b.oblivious_shuffle(shared_column(&backend, &rows)).unwrap();
        // Same permutation: the opened rows come out in the same ORDER.
        assert_eq!(
            open_column(&backend, &sa, 1),
            open_column(&backend, &sb, 1),
            "pi must be derived from the replicated seeds alone"
        );
        // Different masking: the raw shares are not the same.
        let ys = |col: &crate::oblivious::SecretColumn| -> Vec<u64> {
            col.iter().flatten().map(|sh| sh.y.value()).collect()
        };
        assert_ne!(
            ys(&sa),
            ys(&sb),
            "the resharing polynomials must come from the per-party private keys"
        );
    }

    /// Degenerate columns: nothing to permute, so no resharing round runs and none
    /// is charged.
    #[test]
    fn empty_and_singleton_columns_pass_through_uncharged() {
        let n = 5;
        let backend = ShamirBackend::new_seeded(n, 0x0D5).unwrap();
        let mut source = PrssRandomness::with_seeded_setup(n, 1).unwrap();
        let (out, cost) = source.oblivious_shuffle(Vec::new()).unwrap();
        assert!(out.is_empty());
        assert_eq!((cost.items, cost.switches, cost.depth_rounds), (0, 0, 0));

        let col = shared_column(&backend, &[vec![77]]);
        let (out, cost) = source.oblivious_shuffle(col).unwrap();
        assert_eq!(open_column(&backend, &out, 1), vec![vec![77]]);
        assert_eq!((cost.items, cost.switches, cost.depth_rounds), (1, 0, 0));
    }

    /// The oblivious output stage needs a holder's own input shared
    /// (`vss_own_input`), which PRSS refuses — so composing the shuffle into the
    /// full stage must FAIL CLOSED with that `NotYetImplemented`, never panic and
    /// never silently produce an output. The shuffle itself is exercised directly
    /// by the tests above.
    #[test]
    fn the_full_output_stage_on_prss_still_fails_closed_not_panics() {
        let backend = ShamirBackend::new(5).unwrap();
        let mut source = PrssRandomness::with_simulated_setup(5).unwrap();
        let candidates = [crate::oblivious_join::Candidate {
            payload: vec![None],
            matched: crate::oblivious_join::MatchBit::Public(true),
        }];
        assert!(matches!(
            crate::oblivious_join::oblivious_set_output_through(
                &mut source,
                &backend,
                &candidates,
                1,
                2
            ),
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
