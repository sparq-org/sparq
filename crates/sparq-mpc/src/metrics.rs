// sq-sxm — deterministic MPC communication / round / multiplication
// counter for the IN-PROCESS counting tier (Tier 1). New module; instruments at
// call sites added by the benchmark harness, NOT by rewriting backend.rs /
// shamir.rs / oblivious.rs internals (those are owned by sibling beads sq-a6p1 /
// sq-jnkm). The counter is analytic: it records the communication a REAL
// deployment of the same protocol structure would pay, derived from (n, t, sizes)
// — exactly the "counted, not timed" metric the design record prescribes
// (research/mpc-security-models-and-benchmarks.md §5.2).
//! Deterministic communication / round / multiplication accounting for the
//! in-process MPC simulation.
//!
//! ## Why a counter and not a timer
//!
//! `sparq-mpc` is a single-process multi-party *simulation*: every "party" is a
//! function call, so there is NO network, NO round latency, NO bytes on the wire.
//! A single-process wall-clock therefore does NOT measure MPC cost and must never
//! masquerade as one (design record §5, the critical-honesty constraint). The
//! load-bearing, cross-`N`-meaningful metric is the **modelled communication**:
//! how many field elements cross the abstract party boundary, in how many logical
//! rounds, paying how many secure multiplications.
//!
//! These quantities are *deterministic functions of `(n, t, sizes)`* — the same
//! input always yields the same counts (load-robust; unlike a quiet-box-sensitive
//! timing). This module is the accumulator; [`crate::bench`] is the harness that
//! drives the representative operations across the (security model × N × query
//! class) matrix and reads these counters off.
//!
//! ## What is counted (the metric triple + multiplications)
//!
//! Per the design record's metric definitions (§5.2):
//!
//! - **`field_elements_opened`** — each field element reconstructed (opened) across
//!   the parties. The aggregate opens 1 (the sum); each `secure_equal` opens 1 (the
//!   masked product `d·r`). This is the dominant on-wire term.
//! - **`field_elements_shared`** — each field element distributed when a secret is
//!   dealt (one share per party leaves the dealer). Sharing one secret across `n`
//!   parties sends `n` field elements (`n − 1` to the other parties; we count the
//!   full `n` as the abstract boundary-crossings, then derive a per-party figure).
//! - **`multiplications`** — secure (degree-raising) multiplications. Linear ops
//!   (add/sub/scale, the aggregate's cumulative sum) are LOCAL and free → 0. Each
//!   `secure_equal` pays exactly 1; the oblivious shuffle/sort cost is taken from
//!   the crate's own [`crate::oblivious::ShuffleCost`] / [`crate::oblivious::SortCost`].
//! - **`mult_rounds`** / **`open_rounds`** — logical communication rounds. A linear
//!   aggregate is `0` mult rounds `+ 1` open. Each `secure_equal` is `1` mult round
//!   `+ 1` open. An all-pairs join is `|L|·|R|` such tests. (Independent tests can
//!   batch into the same round on a real network — the harness records BOTH the
//!   sequential round count and a batched lower bound so neither over- nor
//!   under-states the round structure.)
//!
//! ## Bytes-per-party derivation
//!
//! A canonical field element is `Fp` over `p = 2^61 − 1`, i.e. it serialises to
//! **8 bytes** ([`FIELD_BYTES`]; one `u64`-width limb). The design record fixes
//! "each `F_p` opened = 8 bytes/party" (§5.2). So `bytes_per_party` for an open is
//! `field_elements_opened × 8` (each party broadcasts its one share to recombine);
//! for a deal it is `8` per secret per recipient party. See [`CommCounter::bytes_per_party`].
//!
//! ## Checked arithmetic: every recorder and total is fallible
//!
//! Each counter is a `u64` accumulating a product of caller-supplied sizes, so a
//! total can leave `u64`. A wrapped counter reports a SMALLER cost than the real
//! one, which is the one direction a cost model must never round, and in a release
//! build a `+=` would have wrapped silently. So every recorder
//! ([`CommCounter::record_deal`], [`CommCounter::record_mult`],
//! [`CommCounter::record_open`], their closed-form batch twins,
//! [`CommCounter::record_shuffle`], [`CommCounter::record_sort`]) and every derived
//! total ([`CommCounter::total_rounds`], [`CommCounter::batched_rounds`],
//! [`CommCounter::bytes_per_party`], [`CommCounter::total_bytes`]) uses checked
//! arithmetic and returns a `Result`.
//!
//! [`CommCounter::record_independent_equalities`] is checked too. It used to
//! saturate, pinning each counter it touches at the `u64` ceiling, which was
//! argued to be safe because a ceiling is not a smaller number. It is not safe as
//! a cost model: a pinned counter is indistinguishable from a real total at the
//! ceiling, and every later checked recorder and derived total on the same
//! `CommCounter` then refuses, so one absurd `k` turns the whole instrument into
//! refusals with no record of which call did it. It returns a `Result` like the
//! rest.
//!
//! ## Every recorder commits ALL its fields or NONE
//!
//! A recorder that touches several counters computes every new value first and
//! assigns only once they all succeeded. Assigning as it went left the counter
//! part-updated behind an `Err` — some fields advanced, some not — which is a
//! state no sequence of successful calls can produce, so a caller that reported
//! the error and carried on would read a total that never existed.
//! `a_refused_recorder_leaves_every_counter_untouched` asserts it for each of the
//! five recorders that touch more than one counter.
//!
//! ## Closed form, so the instrument does not cost what it measures
//!
//! Every counter a single-step recorder touches is LINEAR in its multiplicity, so
//! each has a closed-form batch twin — [`CommCounter::record_deals`],
//! [`CommCounter::record_mults`], [`CommCounter::record_opens`] — that folds `k`
//! applications in arithmetically. The harness ([`crate::bench`]) and the
//! bounded-path planner use those instead of looping, which otherwise made the
//! cost MODEL `O(k)` in the very quantity it was modelling.
//! `the_closed_form_batches_equal_k_single_steps` pins the two against the
//! documented per-op increments, not only against each other.

use crate::partial::MpcError;

/// The one refusal shape every counter in this module uses when a count would
/// leave `u64`. The message names the operation and the quantities, all of them
/// public modelling numbers — a counter holds no secret-shared data.
fn overflowed(what: &str) -> MpcError {
    MpcError::Protocol(format!(
        "CommCounter: {} would overflow u64 — refusing (a cost model that wrapped \
         would report a SMALLER cost than the real one, which is the one direction \
         it must never round)",
        what
    ))
}

/// Serialised width of one canonical [`crate::field::Fp`] element, in bytes. `Fp`
/// is over the Mersenne prime `p = 2^61 − 1`, whose canonical representative fits one `u64`
/// limb, so the on-wire encoding is **8 bytes** — the per-party-per-open cost the
/// design record fixes (§5.2: "each `F_p` opened = 8 bytes/party"). Deriving it
/// from `size_of::<u64>()` keeps the figure tied to the representation rather than
/// a hard-coded literal.
pub const FIELD_BYTES: usize = {
    // Compile-time assertion that the field value fits one u64 limb (so the
    // 8-byte wire width is correct). `Fp::value()` returns `u64`.
    assert!(crate::field::P < u64::MAX);
    std::mem::size_of::<u64>()
};

/// A deterministic accumulator of the communication a REAL deployment of the
/// simulated protocol would pay. All fields are monotone counters; the same
/// protocol over the same `(n, t, sizes)` always produces the same values.
///
/// This is threaded through the harness's instrumented wrappers (see
/// [`crate::bench`]); it deliberately lives OUTSIDE the protocol primitives so the
/// counting tier adds no invasive edits to `shamir.rs` / `backend.rs` /
/// `oblivious.rs` (sibling-bead ownership).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommCounter {
    /// Number of compute parties `n` this accounting is for (so a per-party
    /// figure can be derived). `0` until a protocol is recorded against it.
    pub parties: usize,
    /// Total field elements OPENED (reconstructed across the parties). Each open
    /// is one logical broadcast-and-recombine of one secret value.
    pub field_elements_opened: u64,
    /// Total field elements DEALT (one share per party when a secret is shared).
    /// Counts the abstract boundary-crossings of the dealing step.
    pub field_elements_shared: u64,
    /// Secure (degree-raising) multiplications. Local linear ops are free (0).
    pub multiplications: u64,
    /// Logical multiplication rounds, counted SEQUENTIALLY (one per dependent
    /// multiplication step). The upper bound on rounds.
    pub mult_rounds: u64,
    /// Logical open rounds, counted sequentially (one per dependent open step).
    pub open_rounds: u64,
    /// Independent operations that COULD be batched into a single network round on
    /// a real deployment (e.g. the `|L|·|R|` equality tests of an all-pairs join
    /// are mutually independent). Used to derive a batched round LOWER bound so the
    /// round structure is not over-stated.
    pub batchable_ops: u64,
}

impl CommCounter {
    /// A fresh counter for an `n`-party protocol.
    pub fn new(parties: usize) -> Self {
        CommCounter {
            parties,
            ..Default::default()
        }
    }

    /// Record dealing ONE secret across all `n` parties: `n` field elements leave
    /// the dealer (one share per party). No round cost is attributed here — the
    /// dealing latency is folded into the operation (mult/open) that consumes the
    /// sharing, matching how the metric triple is defined per-operation in §5.2.
    pub fn record_deal(&mut self) -> Result<(), MpcError> {
        self.record_deals(1)
    }

    /// Record dealing `k` secrets across all `n` parties — [`Self::record_deal`]
    /// `k` times, in CLOSED FORM. Every counter it touches is linear in `k`, so a
    /// loop would make the *instrument* `O(k)` and let it dominate a harness at the
    /// larger scales; the totals are identical.
    pub fn record_deals(&mut self, k: u64) -> Result<(), MpcError> {
        let dealt = k
            .checked_mul(self.parties as u64)
            .ok_or_else(|| overflowed("k deals x parties shares"))?;
        let shared = self
            .field_elements_shared
            .checked_add(dealt)
            .ok_or_else(|| overflowed("the dealt-element total"))?;
        self.field_elements_shared = shared;
        Ok(())
    }

    /// Record ONE secure multiplication of two sharings: 1 multiplication, in 1
    /// sequential mult round. In a real DN07/BGW protocol this is one re-share +
    /// recombine round; the crate's single-product primitive opens the *result*
    /// directly, so the open is recorded by the caller's [`Self::record_open`].
    /// Here we count only the multiplication and its round; the operation is
    /// independently batchable.
    pub fn record_mult(&mut self) -> Result<(), MpcError> {
        self.record_mults(1)
    }

    /// Record `k` INDEPENDENT secure multiplications — [`Self::record_mult`] `k`
    /// times, in CLOSED FORM (every counter it touches is linear in `k`).
    pub fn record_mults(&mut self, k: u64) -> Result<(), MpcError> {
        // Compute all three, then commit: a mid-way `?` would leave the counter
        // part-updated.
        let mults = self
            .multiplications
            .checked_add(k)
            .ok_or_else(|| overflowed("the multiplication total"))?;
        let rounds = self
            .mult_rounds
            .checked_add(k)
            .ok_or_else(|| overflowed("the mult-round total"))?;
        let batchable = self
            .batchable_ops
            .checked_add(k)
            .ok_or_else(|| overflowed("the batchable-op total"))?;
        self.multiplications = mults;
        self.mult_rounds = rounds;
        self.batchable_ops = batchable;
        Ok(())
    }

    /// Record opening (reconstructing) ONE field element across the parties: each
    /// party broadcasts its share, so `n` field elements cross the boundary; it is
    /// one logical open round.
    pub fn record_open(&mut self) -> Result<(), MpcError> {
        self.record_opens(1)
    }

    /// Record `k` opens — [`Self::record_open`] `k` times, in CLOSED FORM (every
    /// counter it touches is linear in `k`).
    pub fn record_opens(&mut self, k: u64) -> Result<(), MpcError> {
        let opened = self
            .field_elements_opened
            .checked_add(k)
            .ok_or_else(|| overflowed("the opened-element total"))?;
        let rounds = self
            .open_rounds
            .checked_add(k)
            .ok_or_else(|| overflowed("the open-round total"))?;
        self.field_elements_opened = opened;
        self.open_rounds = rounds;
        Ok(())
    }

    /// Record a batch of `k` MUTUALLY-INDEPENDENT secure equality tests (the
    /// all-pairs join's inner loop). Each is one deal of two keys + one mask, one
    /// multiplication, and one open — but the `k` of them can collapse into a
    /// SINGLE network round on a real deployment (they share no data dependency).
    /// So this adds `k` multiplications / opens / deals to the on-wire totals; the
    /// sequential round counters grow by `k` (the upper bound), while
    /// `batchable_ops` grows by `k` so [`Self::batched_rounds`] can report the
    /// round LOWER bound.
    ///
    /// Every counter is LINEAR in `k`, so this is computed in CLOSED FORM (`O(1)`),
    /// not by looping `record_deal`/`record_mult`/`record_open` `k` times — the
    /// loop would make the *instrument* `O(k)` and let it dominate the harness at
    /// the larger `|L|·|R|` join scales. The totals are identical to `k`
    /// applications of those primitives, including at the `u64` ceiling: this
    /// recorder is CHECKED and fallible like every other one on the type, and
    /// computes all six new values before committing any of them. It used to
    /// saturate, pinning each counter at the ceiling, on the argument that a
    /// ceiling over-states rather than under-states the cost. A pinned counter is
    /// not distinguishable from a real total at the ceiling, and it makes every
    /// later checked recorder and derived total on the same counter refuse, so the
    /// refusal now names the batch instead.
    pub fn record_independent_equalities(&mut self, k: u64) -> Result<(), MpcError> {
        // Per equality: 3 deals (key_L, key_R, mask r) + 1 mult + 1 open. Each
        // deal sends `parties` shares; opens/mults are 1 logical op each. Fold the
        // whole batch in arithmetically (== `k` × the per-op increments above).
        let dealt = k
            .checked_mul(3)
            .and_then(|v| v.checked_mul(self.parties as u64))
            .ok_or_else(|| overflowed("k equalities x 3 deals x parties shares"))?;
        let shared = self
            .field_elements_shared
            .checked_add(dealt)
            .ok_or_else(|| overflowed("the dealt-element total"))?;
        let mults = self
            .multiplications
            .checked_add(k)
            .ok_or_else(|| overflowed("the multiplication total"))?;
        let mult_rounds = self
            .mult_rounds
            .checked_add(k)
            .ok_or_else(|| overflowed("the mult-round total"))?;
        let batchable = self
            .batchable_ops
            .checked_add(k)
            .ok_or_else(|| overflowed("the batchable-op total"))?;
        let opened = self
            .field_elements_opened
            .checked_add(k)
            .ok_or_else(|| overflowed("the opened-element total"))?;
        let open_rounds = self
            .open_rounds
            .checked_add(k)
            .ok_or_else(|| overflowed("the open-round total"))?;
        // Every value computed; commit them together.
        self.field_elements_shared = shared;
        self.multiplications = mults;
        self.mult_rounds = mult_rounds;
        self.batchable_ops = batchable;
        self.field_elements_opened = opened;
        self.open_rounds = open_rounds;
        Ok(())
    }

    /// Fold in the modelled cost of one oblivious **shuffle** (from the crate's own
    /// [`crate::oblivious::ShuffleCost`]): a real secret-control shuffle pays 1
    /// multiplication per switch and `depth_rounds` communication rounds. The
    /// switches are NOT mutually independent (network depth), so the sequential
    /// rounds grow by `depth_rounds`, not by the switch count.
    pub fn record_shuffle(&mut self, cost: crate::oblivious::ShuffleCost) -> Result<(), MpcError> {
        let mults = self
            .multiplications
            .checked_add(cost.switches as u64)
            .ok_or_else(|| overflowed("the multiplication total"))?;
        let rounds = self
            .mult_rounds
            .checked_add(cost.depth_rounds as u64)
            .ok_or_else(|| overflowed("the mult-round total"))?;
        self.multiplications = mults;
        self.mult_rounds = rounds;
        // Each switch's conditional swap opens nothing on its own (it stays
        // secret-shared); the cost is the multiplications + the depth rounds.
        Ok(())
    }

    /// Fold in the modelled cost of one oblivious **sort** (from the crate's own
    /// [`crate::oblivious::SortCost`]): each compare-exchange is a secure
    /// comparison + conditional swap, and the network has `depth_rounds` layers.
    pub fn record_sort(&mut self, cost: crate::oblivious::SortCost) -> Result<(), MpcError> {
        let mults = self
            .multiplications
            .checked_add(cost.compare_exchanges as u64)
            .ok_or_else(|| overflowed("the multiplication total"))?;
        let rounds = self
            .mult_rounds
            .checked_add(cost.depth_rounds as u64)
            .ok_or_else(|| overflowed("the mult-round total"))?;
        self.multiplications = mults;
        self.mult_rounds = rounds;
        Ok(())
    }

    /// Total logical communication rounds, SEQUENTIAL (upper bound): every
    /// dependent mult/open step counted separately.
    pub fn total_rounds(&self) -> Result<u64, MpcError> {
        self.mult_rounds
            .checked_add(self.open_rounds)
            .ok_or_else(|| overflowed("the sequential round total"))
    }

    /// Lower-bound round count assuming all mutually-independent operations batch
    /// into ONE round each (the optimistic real-network figure). When every op is
    /// independent (the all-pairs join), this collapses to a small constant; when
    /// ops are dependency-chained (a sort network's layers), it equals the
    /// sequential count. Reported alongside [`Self::total_rounds`] so neither bound
    /// is mistaken for the other.
    pub fn batched_rounds(&self) -> Result<u64, MpcError> {
        let sequential = self.total_rounds()?;
        if self.batchable_ops == 0 {
            return Ok(sequential);
        }
        // Each batchable op contributed 1 mult round + 1 open round to the
        // sequential count; those collapse into 2 rounds (one mult, one open) when
        // batched. Any rounds NOT attributable to batchable ops remain.
        let batchable_rounds = self
            .batchable_ops
            .checked_mul(2)
            .ok_or_else(|| overflowed("the batchable-round total"))?;
        let non_batchable = sequential.saturating_sub(batchable_rounds);
        non_batchable
            .checked_add(2)
            .ok_or_else(|| overflowed("the batched round total"))
    }

    /// Total field elements that cross the abstract party boundary on the wire.
    ///
    /// An OPEN broadcasts `n` shares (every party sends its one share to
    /// recombine), so each opened *value* is `parties` wire field elements — the
    /// same `n`-fan-out a deal already pays (`record_deal` adds `parties` to
    /// `field_elements_shared`, so that counter is ALREADY in wire units). Keeping
    /// `field_elements_opened` as the logical count (1 per opened value — what the
    /// JSON/table report and the round counters use) and applying the `× parties`
    /// fan-out only here keeps the on-wire model symmetric with deals and makes
    /// `total_bytes` scale with `N` (design record §5.2: "8 bytes/party per open"
    /// over `n` broadcasters ⇒ `8·n` total bytes per open).
    fn wire_field_elements(&self) -> Result<u64, MpcError> {
        self.field_elements_opened
            .checked_mul(self.parties as u64)
            .and_then(|opened| opened.checked_add(self.field_elements_shared))
            .ok_or_else(|| overflowed("the on-wire field-element total"))
    }

    /// **Modelled bytes per party.** Each party broadcasts its one share (8 bytes)
    /// to recombine an open, and receives its one share (8 bytes) per deal. We
    /// total the field elements that cross the boundary and divide by the party
    /// count to get a symmetric per-party figure — the cross-`N`-comparable cost.
    ///
    /// The on-wire traffic is `Self::wire_field_elements` × [`FIELD_BYTES`]; the
    /// per-party share is that divided by `parties` (each party sends/receives an
    /// equal slice in a symmetric protocol). For one open at `n` parties this is
    /// `(1·n)·8 / n = 8` — the per-open-per-party figure the design record fixes
    /// (§5.2), no longer undercounted. Returns `0` when no party count is set
    /// (nothing recorded).
    pub fn bytes_per_party(&self) -> Result<u64, MpcError> {
        if self.parties == 0 {
            return Ok(0);
        }
        Ok(self.total_bytes()? / self.parties as u64)
    }

    /// Total modelled bytes across ALL parties (the aggregate on-wire volume),
    /// before the per-party division — useful for an O(N) vs O(N²) scaling check.
    /// Both opens (`n` broadcast shares each) and deals (`n` shares each) scale
    /// with `N`, so this aggregate grows with the party count as intended.
    pub fn total_bytes(&self) -> Result<u64, MpcError> {
        self.wire_field_elements()?
            .checked_mul(FIELD_BYTES as u64)
            .ok_or_else(|| overflowed("the total-bytes figure"))
    }
}

#[cfg(test)]
mod tests {
    /// The batch fold is CHECKED, not saturating and not wrapping. `k` is a
    /// candidate count the caller supplies, and `3 * k * parties` can exceed
    /// `u64`. A wrapped counter would report a smaller cost than the real one;
    /// a saturated one would report the ceiling as if it were a real total and
    /// make every later checked recorder on the same counter refuse. The refusal
    /// names the batch instead.
    #[test]
    fn an_enormous_equality_batch_is_refused_instead_of_pinned() {
        use super::*;
        let mut c = CommCounter::new(5);
        assert!(
            c.record_independent_equalities(u64::MAX).is_err(),
            "a batch whose dealt elements leave u64 must be refused"
        );

        // And an ordinary batch is still exact: 3 deals x parties shares, one
        // mult/open each, so the closed form must equal the per-op increments.
        let mut small = CommCounter::new(5);
        small
            .record_independent_equalities(4)
            .expect("an ordinary batch is recorded");
        assert_eq!(small.field_elements_shared, 3 * 4 * 5);
        assert_eq!(small.multiplications, 4);
        assert_eq!(small.open_rounds, 4);
    }

    /// A REFUSED recorder leaves every counter exactly as it was.
    ///
    /// Each of these touches several counters, and assigning as it went left the
    /// counter part-updated behind the `Err` — a state no sequence of successful
    /// calls can produce. The check is per recorder: drive each one to its own
    /// overflow from a counter whose fields are at known values, and compare the
    /// whole snapshot.
    #[test]
    fn a_refused_recorder_leaves_every_counter_untouched() {
        use super::*;
        // A counter whose round fields are already at the ceiling, so the SECOND
        // field each recorder touches is the one that overflows — the case that
        // used to commit the first.
        let primed = || {
            let mut c = CommCounter::new(5);
            c.multiplications = 7;
            c.field_elements_shared = 11;
            c.field_elements_opened = 13;
            c.mult_rounds = u64::MAX;
            c.open_rounds = u64::MAX;
            c
        };
        let before = primed();

        let mut c = primed();
        assert!(c.record_mults(1).is_err());
        assert_eq!(c, before, "record_mults must commit all fields or none");

        let mut c = primed();
        assert!(c.record_opens(1).is_err());
        assert_eq!(c, before, "record_opens must commit all fields or none");

        let mut c = primed();
        assert!(c.record_independent_equalities(1).is_err());
        assert_eq!(
            c, before,
            "record_independent_equalities must commit all fields or none"
        );

        let mut c = primed();
        assert!(c
            .record_shuffle(crate::oblivious::ShuffleCost {
                items: 2,
                switches: 1,
                depth_rounds: 1,
                fresh_sharings: 2,
            })
            .is_err());
        assert_eq!(c, before, "record_shuffle must commit all fields or none");

        let mut c = primed();
        assert!(c
            .record_sort(crate::oblivious::SortCost {
                items: 2,
                compare_exchanges: 1,
                depth_rounds: 1,
            })
            .is_err());
        assert_eq!(c, before, "record_sort must commit all fields or none");
    }

    use super::*;
    use crate::field::Fp;

    #[test]
    fn field_bytes_is_eight() {
        // The whole bytes model rests on Fp being one u64 limb.
        assert_eq!(FIELD_BYTES, 8);
        assert_eq!(std::mem::size_of_val(&Fp::zero().value()), FIELD_BYTES);
    }

    #[test]
    fn aggregate_is_zero_mult_one_open() {
        // The cumulative-SUM aggregate over n holders: n deals (each holder shares
        // its private value) + a free local sum + ONE open. Zero multiplications.
        let n = 5;
        let mut c = CommCounter::new(n);
        for _ in 0..n {
            c.record_deal().unwrap();
        }
        c.record_open().unwrap();
        assert_eq!(
            c.multiplications, 0,
            "linear aggregate pays no multiplications"
        );
        assert_eq!(c.mult_rounds, 0);
        assert_eq!(c.open_rounds, 1);
        assert_eq!(c.field_elements_opened, 1);
        assert_eq!(c.field_elements_shared, (n * n) as u64);
    }

    #[test]
    fn one_equality_is_one_mult_one_open() {
        let mut c = CommCounter::new(3);
        c.record_independent_equalities(1).unwrap();
        assert_eq!(c.multiplications, 1);
        assert_eq!(c.field_elements_opened, 1);
        // 3 deals (key_L, key_R, mask) × 3 parties = 9 field elements shared.
        assert_eq!(c.field_elements_shared, 9);
        assert_eq!(c.total_rounds().unwrap(), 2); // 1 mult + 1 open
    }

    #[test]
    fn all_pairs_join_opens_grow_as_product() {
        // |L| = |R| = s ⇒ s² equality tests ⇒ s² opens (the round-count explosion
        // the literature flags) and s² multiplications.
        for s in [2u64, 4, 8] {
            let mut c = CommCounter::new(3);
            c.record_independent_equalities(s * s).unwrap();
            assert_eq!(c.field_elements_opened, s * s);
            assert_eq!(c.multiplications, s * s);
        }
    }

    #[test]
    fn batched_rounds_collapses_independent_ops() {
        // All-pairs join: many independent equalities → sequential rounds grow as
        // 2·k, but the batched lower bound collapses to ~2 (one mult + one open
        // round) because the tests share no data dependency.
        let mut c = CommCounter::new(3);
        let k = 16;
        c.record_independent_equalities(k).unwrap();
        assert_eq!(c.total_rounds().unwrap(), 2 * k); // sequential upper bound
        assert_eq!(c.batched_rounds().unwrap(), 2); // batched lower bound
        assert!(c.batched_rounds().unwrap() < c.total_rounds().unwrap());
    }

    #[test]
    fn bytes_per_party_scales_with_opens() {
        let mut c = CommCounter::new(3);
        c.record_open().unwrap();
        // 1 open broadcasts n=3 shares (each party sends its 8-byte share to
        // recombine) ⇒ 3 wire field elements = 24 total bytes; per party = 8 bytes,
        // the "8 bytes/party per open" figure the design record fixes (§5.2).
        assert_eq!(c.total_bytes().unwrap(), 3 * 8);
        assert_eq!(c.bytes_per_party().unwrap(), 8);
    }

    #[test]
    fn open_bytes_scale_with_party_count() {
        // total_bytes for a single open grows with N (an open is an n-broadcast),
        // and bytes_per_party stays a flat 8 (one 8-byte share per party). This is
        // the symmetry with deals the byte model must preserve.
        for n in [2usize, 3, 5, 7] {
            let mut c = CommCounter::new(n);
            c.record_open().unwrap();
            assert_eq!(c.total_bytes().unwrap(), (n as u64) * FIELD_BYTES as u64);
            assert_eq!(c.bytes_per_party().unwrap(), FIELD_BYTES as u64);
        }
    }

    #[test]
    fn shuffle_cost_folds_in_switches_and_depth() {
        let cost = crate::oblivious::ShuffleCost::model(8, 17);
        let mut c = CommCounter::new(3);
        c.record_shuffle(cost).unwrap();
        assert_eq!(c.multiplications, 17);
        assert_eq!(c.mult_rounds, cost.depth_rounds as u64);
    }

    #[test]
    fn deterministic_same_input_same_counts() {
        let run = || {
            let mut c = CommCounter::new(5);
            for _ in 0..5 {
                c.record_deal().unwrap();
            }
            c.record_open().unwrap();
            c.record_independent_equalities(9).unwrap();
            c
        };
        assert_eq!(
            run(),
            run(),
            "same protocol → identical counts (load-robust)"
        );
    }

    /// Every CHECKED recorder and every total refuses at the `u64` ceiling instead
    /// of wrapping. A wrapped counter reports a SMALLER cost than the real one,
    /// which is the one direction a cost model must never round, and a silent wrap
    /// is exactly what a release build would have done here.
    #[test]
    fn every_checked_recorder_and_total_refuses_at_the_ceiling() {
        // Deals: `k x parties` and then the running total.
        let mut c = CommCounter::new(5);
        assert!(
            c.record_deals(u64::MAX).is_err(),
            "k x parties must refuse, not wrap"
        );
        let mut c = CommCounter {
            parties: 2,
            field_elements_shared: u64::MAX,
            ..Default::default()
        };
        assert!(c.record_deal().is_err(), "the dealt total must refuse");

        // Multiplications / rounds / batchable ops.
        let mut c = CommCounter {
            parties: 3,
            multiplications: u64::MAX,
            ..Default::default()
        };
        assert!(c.record_mult().is_err());
        let mut c = CommCounter {
            parties: 3,
            mult_rounds: u64::MAX,
            ..Default::default()
        };
        assert!(c.record_mult().is_err());
        let mut c = CommCounter {
            parties: 3,
            batchable_ops: u64::MAX,
            ..Default::default()
        };
        assert!(c.record_mult().is_err());

        // Opens.
        let mut c = CommCounter {
            parties: 3,
            field_elements_opened: u64::MAX,
            ..Default::default()
        };
        assert!(c.record_open().is_err());
        let mut c = CommCounter {
            parties: 3,
            open_rounds: u64::MAX,
            ..Default::default()
        };
        assert!(c.record_open().is_err());

        // The network cost folds.
        let mut c = CommCounter {
            parties: 3,
            multiplications: u64::MAX,
            ..Default::default()
        };
        assert!(c
            .record_shuffle(crate::oblivious::ShuffleCost::model(8, 17))
            .is_err());
        let mut c = CommCounter {
            parties: 3,
            multiplications: u64::MAX,
            ..Default::default()
        };
        assert!(c
            .record_sort(crate::oblivious::SortCost {
                items: 8,
                compare_exchanges: 1,
                depth_rounds: 1,
            })
            .is_err());

        // The totals.
        let full = CommCounter {
            parties: 3,
            mult_rounds: u64::MAX,
            open_rounds: 1,
            ..Default::default()
        };
        assert!(full.total_rounds().is_err());
        assert!(full.batched_rounds().is_err());
        let wide = CommCounter {
            parties: 3,
            field_elements_opened: u64::MAX,
            ..Default::default()
        };
        assert!(wide.total_bytes().is_err(), "opened x parties must refuse");
        assert!(wide.bytes_per_party().is_err());
        let bytes = CommCounter {
            parties: 1,
            field_elements_shared: u64::MAX,
            ..Default::default()
        };
        assert!(
            bytes.total_bytes().is_err(),
            "elements x FIELD_BYTES must refuse"
        );

        // And the refusals name the module, so an overflow is diagnosable.
        match CommCounter::new(5).record_deals(u64::MAX) {
            Err(MpcError::Protocol(m)) => {
                assert!(m.contains("CommCounter") && m.contains("overflow"), "{m}")
            }
            other => panic!("expected a Protocol refusal, got {other:?}"),
        }
    }

    /// The CLOSED-FORM batch recorders are exactly `k` applications of their
    /// single-step twin — that is what makes replacing a harness loop with one call
    /// a refactor and not a change to the model.
    #[test]
    fn the_closed_form_batches_equal_k_single_steps() {
        for k in [0u64, 1, 2, 7, 33] {
            for parties in [1usize, 3, 5] {
                let mut looped = CommCounter::new(parties);
                for _ in 0..k {
                    looped.record_deal().unwrap();
                    looped.record_mult().unwrap();
                    looped.record_open().unwrap();
                }
                let mut closed = CommCounter::new(parties);
                closed.record_deals(k).unwrap();
                closed.record_mults(k).unwrap();
                closed.record_opens(k).unwrap();
                assert_eq!(
                    closed, looped,
                    "k={}, parties={}: the closed form must equal k single steps",
                    k, parties
                );
                // Asserted against the DOCUMENTED per-op increments too, not only
                // against the loop: `record_mult` delegates to `record_mults`, so a
                // dropped counter would move both sides of that comparison
                // together and the comparison alone would pass.
                assert_eq!(closed.field_elements_shared, k * parties as u64);
                assert_eq!(closed.multiplications, k);
                assert_eq!(closed.mult_rounds, k);
                assert_eq!(closed.batchable_ops, k);
                assert_eq!(closed.field_elements_opened, k);
                assert_eq!(closed.open_rounds, k);
            }
        }
    }
}
