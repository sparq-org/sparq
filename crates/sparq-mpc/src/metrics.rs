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
    pub fn record_deal(&mut self) {
        self.field_elements_shared += self.parties as u64;
    }

    /// Record ONE secure multiplication of two sharings: 1 multiplication, in 1
    /// sequential mult round. In a real DN07/BGW protocol this is one re-share +
    /// recombine round; the crate's single-product primitive opens the *result*
    /// directly, so the open is recorded by the caller's [`Self::record_open`].
    /// Here we count only the multiplication and its round; the operation is
    /// independently batchable.
    pub fn record_mult(&mut self) {
        self.multiplications += 1;
        self.mult_rounds += 1;
        self.batchable_ops += 1;
    }

    /// Record opening (reconstructing) ONE field element across the parties: each
    /// party broadcasts its share, so `n` field elements cross the boundary; it is
    /// one logical open round.
    pub fn record_open(&mut self) {
        self.field_elements_opened += 1;
        self.open_rounds += 1;
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
    /// applications of those primitives.
    pub fn record_independent_equalities(&mut self, k: u64) {
        // Per equality: 3 deals (key_L, key_R, mask r) + 1 mult + 1 open. Each
        // deal sends `parties` shares; opens/mults are 1 logical op each. Fold the
        // whole batch in arithmetically (== `k` × the per-op increments above).
        self.field_elements_shared += 3 * k * self.parties as u64;
        self.multiplications += k;
        self.mult_rounds += k;
        self.batchable_ops += k;
        self.field_elements_opened += k;
        self.open_rounds += k;
    }

    /// Fold in the modelled cost of one oblivious **shuffle** (from the crate's own
    /// [`crate::oblivious::ShuffleCost`]): a real secret-control shuffle pays 1
    /// multiplication per switch and `depth_rounds` communication rounds. The
    /// switches are NOT mutually independent (network depth), so the sequential
    /// rounds grow by `depth_rounds`, not by the switch count.
    pub fn record_shuffle(&mut self, cost: crate::oblivious::ShuffleCost) {
        self.multiplications += cost.switches as u64;
        self.mult_rounds += cost.depth_rounds as u64;
        // Each switch's conditional swap opens nothing on its own (it stays
        // secret-shared); the cost is the multiplications + the depth rounds.
    }

    /// Fold in the modelled cost of one oblivious **sort** (from the crate's own
    /// [`crate::oblivious::SortCost`]): each compare-exchange is a secure
    /// comparison + conditional swap, and the network has `depth_rounds` layers.
    pub fn record_sort(&mut self, cost: crate::oblivious::SortCost) {
        self.multiplications += cost.compare_exchanges as u64;
        self.mult_rounds += cost.depth_rounds as u64;
    }

    /// Total logical communication rounds, SEQUENTIAL (upper bound): every
    /// dependent mult/open step counted separately.
    pub fn total_rounds(&self) -> u64 {
        self.mult_rounds + self.open_rounds
    }

    /// Lower-bound round count assuming all mutually-independent operations batch
    /// into ONE round each (the optimistic real-network figure). When every op is
    /// independent (the all-pairs join), this collapses to a small constant; when
    /// ops are dependency-chained (a sort network's layers), it equals the
    /// sequential count. Reported alongside [`Self::total_rounds`] so neither bound
    /// is mistaken for the other.
    pub fn batched_rounds(&self) -> u64 {
        let sequential = self.total_rounds();
        if self.batchable_ops == 0 {
            return sequential;
        }
        // Each batchable op contributed 1 mult round + 1 open round to the
        // sequential count; those collapse into 2 rounds (one mult, one open) when
        // batched. Any rounds NOT attributable to batchable ops remain.
        let batchable_rounds = 2 * self.batchable_ops;
        let non_batchable = sequential.saturating_sub(batchable_rounds);
        non_batchable + 2
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
    fn wire_field_elements(&self) -> u64 {
        self.field_elements_opened * self.parties as u64 + self.field_elements_shared
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
    pub fn bytes_per_party(&self) -> u64 {
        if self.parties == 0 {
            return 0;
        }
        let total_bytes = self.wire_field_elements() * FIELD_BYTES as u64;
        total_bytes / self.parties as u64
    }

    /// Total modelled bytes across ALL parties (the aggregate on-wire volume),
    /// before the per-party division — useful for an O(N) vs O(N²) scaling check.
    /// Both opens (`n` broadcast shares each) and deals (`n` shares each) scale
    /// with `N`, so this aggregate grows with the party count as intended.
    pub fn total_bytes(&self) -> u64 {
        self.wire_field_elements() * FIELD_BYTES as u64
    }
}

#[cfg(test)]
mod tests {
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
            c.record_deal();
        }
        c.record_open();
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
        c.record_independent_equalities(1);
        assert_eq!(c.multiplications, 1);
        assert_eq!(c.field_elements_opened, 1);
        // 3 deals (key_L, key_R, mask) × 3 parties = 9 field elements shared.
        assert_eq!(c.field_elements_shared, 9);
        assert_eq!(c.total_rounds(), 2); // 1 mult + 1 open
    }

    #[test]
    fn all_pairs_join_opens_grow_as_product() {
        // |L| = |R| = s ⇒ s² equality tests ⇒ s² opens (the round-count explosion
        // the literature flags) and s² multiplications.
        for s in [2u64, 4, 8] {
            let mut c = CommCounter::new(3);
            c.record_independent_equalities(s * s);
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
        c.record_independent_equalities(k);
        assert_eq!(c.total_rounds(), 2 * k); // sequential upper bound
        assert_eq!(c.batched_rounds(), 2); // batched lower bound
        assert!(c.batched_rounds() < c.total_rounds());
    }

    #[test]
    fn bytes_per_party_scales_with_opens() {
        let mut c = CommCounter::new(3);
        c.record_open();
        // 1 open broadcasts n=3 shares (each party sends its 8-byte share to
        // recombine) ⇒ 3 wire field elements = 24 total bytes; per party = 8 bytes,
        // the "8 bytes/party per open" figure the design record fixes (§5.2).
        assert_eq!(c.total_bytes(), 3 * 8);
        assert_eq!(c.bytes_per_party(), 8);
    }

    #[test]
    fn open_bytes_scale_with_party_count() {
        // total_bytes for a single open grows with N (an open is an n-broadcast),
        // and bytes_per_party stays a flat 8 (one 8-byte share per party). This is
        // the symmetry with deals the byte model must preserve.
        for n in [2usize, 3, 5, 7] {
            let mut c = CommCounter::new(n);
            c.record_open();
            assert_eq!(c.total_bytes(), (n as u64) * FIELD_BYTES as u64);
            assert_eq!(c.bytes_per_party(), FIELD_BYTES as u64);
        }
    }

    #[test]
    fn shuffle_cost_folds_in_switches_and_depth() {
        let cost = crate::oblivious::ShuffleCost::model(8, 17);
        let mut c = CommCounter::new(3);
        c.record_shuffle(cost);
        assert_eq!(c.multiplications, 17);
        assert_eq!(c.mult_rounds, cost.depth_rounds as u64);
    }

    #[test]
    fn deterministic_same_input_same_counts() {
        let run = || {
            let mut c = CommCounter::new(5);
            for _ in 0..5 {
                c.record_deal();
            }
            c.record_open();
            c.record_independent_equalities(9);
            c
        };
        assert_eq!(
            run(),
            run(),
            "same protocol → identical counts (load-robust)"
        );
    }
}
