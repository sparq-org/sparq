// sq-18lk: oblivious shuffle (Waksman/Benes net) + oblivious sort
// substrate over Shamir F_p. Keystone hidden-regime primitive (ORQ SOSP'25): DISTINCT,
// ORDER BY, GROUP BY-over-hidden, MIN/MAX, OPTIONAL/MINUS, the set-returning
// oblivious-join output path, and ~linear joins all reduce to it. Native-only;
// in-process multi-party simulation; communication cost MODELLED not measured.
//! Oblivious shuffle + sort over secret-shared `F_p` vectors — the keystone
//! hidden-regime primitive (bead sq-18lk).
//!
//! ## Why this is the single highest-leverage primitive
//!
//! The unifying insight from relational MPC (ORQ SOSP'25 eprint 2025/1657,
//! Secrecy NSDI'23, Conclave, Senate, SMCQL — see
//! `research/mpc-security-models-and-benchmarks.md` §3) is that secure relational
//! evaluation is dominated by ONE primitive family: **oblivious shuffle + sort**.
//! Every set / dedup / order / group / outer operator reduces to a
//! *sort-then-linear-scan* so the pipeline costs `O(n log n)` work, `O(log n)`
//! rounds, `O(n)` memory (`n` = total input rows). This substrate is what unlocks,
//! against secret-shared (HIDDEN) operands:
//!
//! - **ORDER BY** — oblivious sort.
//! - **DISTINCT / dedup** — oblivious sort + adjacent-equality scan + compaction.
//! - **GROUP BY (hidden key)** — sort-on-key + segmented prefix scan.
//! - **MIN / MAX** — sort + extreme (or a comparison tournament).
//! - **OPTIONAL / MINUS / UNION** — the same sort-merge, different gating bits.
//! - **The set-returning oblivious-join OUTPUT path** — oblivious-SHUFFLE the
//!   matched rows then reveal only a padded prefix, so the per-pair match graph
//!   (leak L2, §4.1) is destroyed before anything opens.
//! - **~linear joins** — ORQ-style sort-merge join-aggregation (bead sq-ujz8),
//!   replacing the current `O(|L|·|R|)` all-pairs [`crate::join::HiddenValueJoin`].
//!
//! Those downstream operators are tracked as their own beads — this module is the
//! *substrate* they build on, not the operators themselves: see **sq-ujz8**
//! (ORQ-style oblivious sort-merge join-aggregation) and **sq-jnkm** (oblivious
//! result-size protection + match-bit aggregation, which consumes [`shuffle`]).
//!
//! ## What this module IS (sound today) vs what it scopes out (stated, not faked)
//!
//! ### Oblivious shuffle — REAL and sound at the honest-majority Shamir layer
//!
//! [`shuffle`] permutes a [`SecretColumn`] (a vector of full Shamir sharings) by a
//! **uniformly-random secret permutation** drawn from the masking CSPRNG
//! ([`crate::rng`]), routed through a **Waksman/Beneš permutation network**
//! ([`WaksmanNetwork`]). The output order is information-theoretically independent
//! of the input order to any party seeing `≤ t` shares, because:
//!
//! - the network *topology* (which wire pairs each switch acts on, at each layer)
//!   is **fixed by `n` alone — data-independent**; and
//! - the per-switch control bits realize a uniform random permutation and stay
//!   secret (in the simulation the routing layer holds them; in a real deployment
//!   they are jointly generated via PRSS — eprint 2021/1223 — and never opened).
//!
//! Crucially, with **cleartext** control bits — which is what THIS in-process
//! simulation uses (the routing layer / dealer holds the sampled permutation) —
//! a switch over two *sharings* is a **swap of two share-vectors**: pure local
//! relabeling, **zero multiplications, stays degree `t`, needs no degree
//! reduction**. That is exactly why the shuffle is buildable today on the
//! current Shamir machinery (which has only a single non-reducing product —
//! [`crate::shamir::mul_shares_raw`]) whereas a generic secure comparator is not.
//! (A deployed shuffle keeps the control bits **secret**, replacing each local
//! swap by the arithmetic conditional swap `x' = x + b·(y − x)` — 1 multiplication
//! per switch — at the SAME obliviousness; that modelled cost is [`ShuffleCost`].
//! "Zero multiplications" here is the simulation's cleartext-routing cost, not a
//! claim that the deployed protocol is multiplication-free — see the
//! communication-complexity note below and [`WaksmanNetwork::apply`].)
//! This is the standard oblivious shuffle (Waks-On/Waks-Off CCS'23, eprint
//! 2023/1236; we use the **full-support Waksman/Beneš network — NOT a network
//! that biases the realizable permutation distribution** (the "Benes — biased"
//! caveat the bead/design record §3 flag); full support is tested for small `n`).
//!
//! ### Oblivious sort — the SUBSTRATE (data-independent access pattern) is real;
//! ### the secret-key comparator is HONESTLY out of scope on this backend
//!
//! [`sort_by`] runs a **Batcher odd-even mergesort network** ([`SortingNetwork`]):
//! a `O(n log² n)` sequence of *compare-exchange* gates whose `(i, j)` index
//! sequence is **fixed by `n` alone** — completely independent of the secret data
//! (the load-bearing obliviousness property; the access pattern is asserted
//! data-independent in the tests). This network — the data-oblivious access
//! pattern — is the substrate the bead asks for, and it is real.
//!
//! Each compare-exchange needs a **secure comparison** `a < b` over two *secret*
//! `F_p` values plus a conditional swap. A secure comparison (Rabbit eprint
//! 2021/119 / edaBits Crypto'20) is a bit-decomposition that consumes a
//! **multiplication CHAIN**, which needs DN07/ATLAS **degree reduction** — and the
//! current backend has only a *single* non-reducing product (`mul_shares_raw`).
//! So a comparator over genuinely-secret values is **NOT buildable on this
//! backend yet** — it is gated on the BGW/DN degree-reduction round (bead
//! **sq-dvuc**) and the secure greater-than primitive built on it (bead
//! **sq-rrz4**); tracked, not faked. We therefore expose the sort against a
//! [`Comparator`] seam and ship two honest instantiations:
//!
//! - [`sort_with_keys`] — sorts rows by a **disclosed / public** sort key. The
//!   *values* stay secret-shared and are obliviously permuted; only the public
//!   keys drive the comparisons. This is the common DISCLOSED-regime case (§3: the
//!   verifier-recompute path) and is fully sound today.
//! - `SimulatedSecretComparator` — for the in-process simulation / differential
//!   tests ONLY (`#[cfg(any(test, feature = "insecure-test-rng"))]`): it
//!   reconstructs the two values to compare. **This LEAKS the comparison result
//!   and is NOT secure** — it stands in for the unbuilt secure comparator purely
//!   so the sort *network* and its obliviousness (the substrate) can be tested
//!   end-to-end. Its name and gate are the warning; no security is claimed.
//!
//! This is the empirical-honesty line the design record demands: the **shuffle**
//! and the **sort network / access-pattern obliviousness** are delivered for real;
//! the **secret-key comparator** is truthfully gated on degree reduction rather
//! than faked.
//!
//! ## Communication / round complexity (MODELLED — the crate is an in-process
//! ## simulation; these are counted, NOT measured)
//!
//! [`ShuffleCost`] / [`SortCost`] report the cost a REAL deployment would pay, per
//! the design record §5.2 metric triple (rounds, bytes/party, switch/compare
//! count). The simulation runs in one process so wall-clock here is NOT an MPC
//! latency; only these counts are load-bearing:
//!
//! - **Shuffle (Waksman/Beneš, `n` items):** `O(n log n)` conditional swaps;
//!   network **depth** `2·⌈log₂ n⌉ − 1` layers → that many communication rounds
//!   when control bits are secret-shared. **Two distinct cost regimes — do not
//!   conflate them:**
//!   - *This in-process SIMULATION* routes via **cleartext** control bits (the
//!     dealer holds the sampled permutation), so [`WaksmanNetwork::apply`] does a
//!     local relabel/swap of two whole sharings — **0 multiplications, 0 rounds**.
//!     That is the only reason the shuffle is buildable on the single-product
//!     backend, and what "0 multiplications" elsewhere refers to.
//!   - *The DEPLOYED protocol* uses **secret** control bits (PRSS-generated, never
//!     revealed), so each switch is the arithmetic conditional swap
//!     `x' = x + b·(y − x)` — **1 multiplication per switch**, the cost
//!     [`ShuffleCost`] models. The obliviousness is identical; only the
//!     simulation's bookkeeping of the routing differs.
//!
//!   Comm scales with the share width crossing the party boundary.
//! - **Sort (Batcher odd-even mergesort, `n` items):** `O(n log² n)`
//!   compare-exchange gates in `O(log² n)` layers (rounds), each gate one secure
//!   comparison + one conditional swap.
//!
//! No hard-coded performance numbers live in markdown (AGENTS.md): these are the
//! analytic complexity, recomputed from `n` by [`ShuffleCost::model`] /
//! [`SortCost`], not a measured benchmark figure.

use crate::field::Fp;
use crate::partial::MpcError;
use crate::shamir::{ShamirDealer, Share};

/// A vector of secret-shared field elements — the unit the shuffle/sort operate
/// over. Each entry is a full Shamir sharing (`Vec<Share>` = one value's `n`
/// per-party points, the trait's [`crate::shamir::ShareVec`]). Permuting /
/// sorting this column rearranges the *sharings*, never reconstructing a value
/// (except where an explicitly-insecure simulated comparator does so for tests).
pub type SecretColumn = Vec<Vec<Share>>;

// =====================================================================
// AS-Waksman permutation network (arbitrary-size; the unbiased shuffle net)
// =====================================================================

/// One switch (2×2 swap gate) in a permutation network, acting on the two wire
/// indices `(a, b)` with `a < b`. A switch is `pass` (identity) or `swap`; the
/// realized choice is the switch's *control bit* (secret in the shuffle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switch {
    /// The lower wire index the switch acts on.
    pub a: usize,
    /// The upper wire index the switch acts on (`b > a`).
    pub b: usize,
}

/// An **arbitrary-size Waksman/Beneš** permutation network over `n` wires: the
/// data-INDEPENDENT topology (a fixed list of [`Switch`]es) that can route ANY of
/// the `n!` permutations by choosing per-switch control bits.
///
/// This recursive switching network is chosen over a plain power-of-two Beneš
/// network for two reasons stated in the design record (§3, §4.3): (1) it handles
/// **arbitrary `n`** (no padding to a power of two), and (2) — load-bearing for a
/// *shuffle* — it has **full support**: every one of the `n!` permutations is
/// realizable by some control-bit setting (tested exhaustively for small `n`), so
/// a uniformly-random permutation routed through it gives an **unbiased** shuffle.
/// That is the property "Benes — biased" in the bead refers to: a network that
/// cannot reach all permutations (or reaches some far more often) leaks order
/// information. The switch count is `O(n log n)`.
///
/// We build the **AS-Waksman** (Beauquier–Darrot, "On arbitrary-size Waksman
/// networks") redundancy-removal form: `⌊m/2⌋` input switches and `⌊(m−1)/2⌋`
/// output switches per level (one fewer output switch than the plain Beneš form on
/// **even** `m`), giving the optimal arbitrary-size count `W(n) = W(⌈n/2⌉) +
/// W(⌊n/2⌋) + (n−1)`. The saving comes from **fixing one output pair** on even
/// levels: the last output pair `(m−2, m−1)` is routed straight (output `m−2` from
/// the TOP sub-network, output `m−1` from the BOTTOM), so it needs no switch — the
/// alternating-path solver seeds that pin and derives the rest, so the fixed pair
/// is a *consequence* of the routing rather than an over-constraint. On **odd** `m`
/// no extra output pair is pinned (the odd-trailing output already carries no
/// switch), which is exactly how the corner-case clash the bead flagged — output 1
/// pinned BOTTOM vs. the odd-trailing input pinned TOP — is avoided. Full support
/// (every `n!` permutation reachable) and exhaustive routing are tested for small
/// `n`; see `solve_waksman_level`.
///
/// The topology depends ONLY on `n`. The same network is reused for every shuffle
/// of the same width; only the control bits (the secret permutation) change.
///
/// ## Construction
///
/// The network is realized by *recording the switch operations* of the recursive
/// routing run on a concrete permutation: building and routing share one recursion
/// (`route_recording`), so the switch list and the control-bit list are produced
/// in lockstep and are guaranteed consistent (a separate "build then route against
/// it" pair is the classic source of off-by-one bugs). The topology (`switches()`)
/// is data-independent because the recursion's *structure* — which wire pairs
/// become switches — depends only on `n`, never on which permutation is routed
/// (asserted in `waksman_topology_is_permutation_independent`).
#[derive(Debug, Clone)]
pub struct WaksmanNetwork {
    n: usize,
    switches: Vec<Switch>,
}

impl WaksmanNetwork {
    /// Build the Waksman/Beneš network for `n` wires (`n >= 1`). `n <= 1` is the
    /// trivial empty network (nothing to route).
    pub fn new(n: usize) -> Self {
        // Route the identity permutation to discover the switch topology. The set
        // of switch wire-pairs is permutation-independent (it is the recursion
        // structure), so the identity yields the canonical topology; control bits
        // are discarded here.
        let identity: Vec<usize> = (0..n).collect();
        let (switches, _bits) = route_recording(&identity);
        WaksmanNetwork { n, switches }
    }

    /// Number of wires.
    pub fn width(&self) -> usize {
        self.n
    }

    /// The switches in application order. This sequence is **data-independent**
    /// (a function of `n` only) — the obliviousness invariant of the shuffle: the
    /// set of wire-pairs switched reveals nothing about the routed permutation.
    pub fn switches(&self) -> &[Switch] {
        &self.switches
    }

    /// The number of switches (= conditional swaps / multiplications a real
    /// shuffle pays).
    pub fn switch_count(&self) -> usize {
        self.switches.len()
    }

    /// Compute the per-switch control bits that route the network so that input
    /// position `i` ends at output position `perm[i]` (output `perm[i]` receives
    /// input `i`). `perm` must be a permutation of `0..n`. Returns one bool per
    /// switch (in `switches()` order): `true` = swap, `false` = pass.
    ///
    /// In a real deployment the resulting control bits are **secret** (PRSS-
    /// generated, never opened); here they are computed in cleartext by the routing
    /// layer because this process plays all parties — exactly the dealer role.
    pub fn route(&self, perm: &[usize]) -> Result<Vec<bool>, MpcError> {
        if perm.len() != self.n {
            return Err(MpcError::Protocol(format!(
                "route: permutation length {} != network width {}",
                perm.len(),
                self.n
            )));
        }
        let mut seen = vec![false; self.n];
        for &p in perm {
            if p >= self.n || std::mem::replace(&mut seen[p], true) {
                return Err(MpcError::Protocol(
                    "route: argument is not a permutation of 0..n".into(),
                ));
            }
        }
        let (switches, bits) = route_recording(perm);
        // The switch list must match the canonical topology (it does, by the
        // build==route recursion); guard it so a future refactor can't desync.
        debug_assert_eq!(
            switches, self.switches,
            "route topology desynced from build"
        );
        Ok(bits)
    }

    /// Apply the network to a [`SecretColumn`] given per-switch control bits (in
    /// `switches()` order). A `swap` bit exchanges the two share-vectors at the
    /// switch's wires; a `pass` bit leaves them. **No field multiplication** — a
    /// swap of two sharings is local relabeling (degree `t` preserved), which is
    /// why this is buildable on the single-product backend.
    ///
    /// (A real shuffle with *secret* control bits replaces each cleartext swap by
    /// the arithmetic conditional swap `x' = x + b·(y−x)`, one multiplication per
    /// switch — see the module cost note. The simulation's local swap models the
    /// same routing at zero extra rounds, with the cost surfaced via
    /// [`ShuffleCost`].)
    pub fn apply(&self, col: &mut SecretColumn, bits: &[bool]) -> Result<(), MpcError> {
        if col.len() != self.n {
            return Err(MpcError::Protocol(format!(
                "apply: column length {} != network width {}",
                col.len(),
                self.n
            )));
        }
        if bits.len() != self.switches.len() {
            return Err(MpcError::Protocol(format!(
                "apply: {} control bits != {} switches",
                bits.len(),
                self.switches.len()
            )));
        }
        for (sw, &bit) in self.switches.iter().zip(bits) {
            if bit {
                col.swap(sw.a, sw.b);
            }
        }
        Ok(())
    }
}

/// Route a permutation through the AS-Waksman recursion, RECORDING both the switch
/// wire-pairs (the topology) and their control bits. Returns `(switches, bits)` in
/// application order. This single function both *builds* (topology) and *routes*
/// (bits); [`WaksmanNetwork::new`] discards the bits and [`WaksmanNetwork::route`]
/// uses them, so the two can never disagree.
///
/// `perm[i] = o` means input `i` must arrive at output `o`.
fn route_recording(perm: &[usize]) -> (Vec<Switch>, Vec<bool>) {
    let n = perm.len();
    let mut switches = Vec::new();
    let mut bits = Vec::new();
    let wires: Vec<usize> = (0..n).collect();
    waksman_rec(&wires, perm, &mut switches, &mut bits);
    (switches, bits)
}

/// Recursive AS-Waksman build+route over `wires` (the GLOBAL wire indices this
/// sub-network spans, in order) realizing the local permutation `perm`
/// (`perm[local_in] = local_out`, on LOCAL indices `0..m`). Emits switches on the
/// global wire indices and their control bits, in application order.
///
/// Topology (data-independent — depends only on `m`): an input column pairs local
/// inputs `(2k, 2k+1)`; one output of each input switch feeds a TOP sub-network,
/// the other a BOTTOM sub-network; an output column recombines them. For odd `m`
/// the trailing input/output passes straight into the top sub-network (no input
/// switch, no output switch).
///
/// We use the **AS-Waksman recursive network** (Beauquier–Darrot): `⌊m/2⌋` input
/// switches and `⌊(m−1)/2⌋` output switches per level. On **even** `m` that is one
/// FEWER output switch than the plain Beneš form — the last output pair `(m−2,
/// m−1)` is routed straight (no switch) by fixing output `m−2` to the TOP
/// sub-network and `m−1` to the BOTTOM. On **odd** `m` all `⌊m/2⌋` output switches
/// are kept (no extra pair is pinned), so the odd-trailing-input / fixed-pair
/// clash the bead flagged cannot occur. This yields the optimal arbitrary-size
/// count `W(n) = W(⌈n/2⌉) + W(⌊n/2⌋) + (n−1)` (still `O(n log n)`) and routes EVERY
/// permutation (verified exhaustively up to `n = 9`). Crucially for a *shuffle* it
/// is **NOT Benes-biased**: it reaches all `n!` permutations with full support
/// (tested for `n = 3, 4`).
///
/// Routing (data-dependent — the control BITS, not the topology): solved by the
/// standard alternating-path / 2-coloring argument over the input/output pair
/// constraints; each cycle is seeded at an undecided input.
fn waksman_rec(wires: &[usize], perm: &[usize], switches: &mut Vec<Switch>, bits: &mut Vec<bool>) {
    let m = wires.len();
    debug_assert_eq!(perm.len(), m);
    if m <= 1 {
        return;
    }
    if m == 2 {
        switches.push(Switch {
            a: wires[0],
            b: wires[1],
        });
        bits.push(perm[0] == 1); // swap iff input 0 must reach output 1
        return;
    }

    let half = m / 2; // number of input switches
    let odd = m % 2 == 1;
    let top_len = m - half; // = ⌈m/2⌉
    let bot_len = half; // = ⌊m/2⌋

    // AS-Waksman: ⌊(m−1)/2⌋ output switches. On even m the LAST output pair
    // (m−2, m−1) is fixed (no switch), shaving one switch this level; on odd m all
    // `half` output switches are kept.
    let out_switches = (m - 1) / 2;

    let solved = solve_waksman_level(m, half, odd, out_switches, perm);

    // --- emit in BUILD/application order: input column, top sub, bottom sub, output column.

    // Input column: switch k acts on local (2k, 2k+1) → global (wires[2k], wires[2k+1]).
    for k in 0..half {
        switches.push(Switch {
            a: wires[2 * k],
            b: wires[2 * k + 1],
        });
        bits.push(solved.in_swap[k]);
    }

    // Sub-network global wires. Top sub-network spans the "top output" of each
    // input switch (top sub-network input slot k = global wire 2k) plus the odd
    // trailing wire; bottom spans the "bottom output" (slot k = global wire 2k+1).
    let mut top_wires = Vec::with_capacity(top_len);
    let mut bot_wires = Vec::with_capacity(bot_len);
    for k in 0..half {
        top_wires.push(wires[2 * k]);
        bot_wires.push(wires[2 * k + 1]);
    }
    if odd {
        top_wires.push(wires[m - 1]);
    }

    waksman_rec(&top_wires, &solved.top_perm, switches, bits);
    waksman_rec(&bot_wires, &solved.bot_perm, switches, bits);

    // Output column: AS-Waksman emits only the first `out_switches = ⌊(m−1)/2⌋`
    // output switches. On even m the last output pair (m−2, m−1) is fixed (output
    // m−2 from TOP, m−1 from BOTTOM) and carries NO switch — that is the −1/level
    // saving; on odd m `out_switches == half` so all are emitted.
    for k in 0..out_switches {
        switches.push(Switch {
            a: wires[2 * k],
            b: wires[2 * k + 1],
        });
        bits.push(solved.out_swap[k]);
    }
}

/// The solved routing of one AS-Waksman level.
struct WaksmanLevel {
    in_swap: Vec<bool>,   // len half (per input switch)
    out_swap: Vec<bool>,  // len ⌊(m−1)/2⌋ (the last even-m output pair is fixed)
    top_perm: Vec<usize>, // permutation routed by the top sub-network (len ⌈m/2⌉)
    bot_perm: Vec<usize>, // permutation routed by the bottom sub-network (len ⌊m/2⌋)
}

/// Solve one AS-Waksman level: choose input-switch bits, output-switch bits, and
/// the two sub-network permutations realizing `perm` (local indices `0..m`).
///
/// ## Topology semantics (local indices)
///
/// - **Input switch `k`** (`k < half`) carries inputs `2k` (low) and `2k+1`
///   (high). Its two outputs are TOP slot `k` and BOTTOM slot `k`. When it PASSES,
///   low→TOP[k], high→BOTTOM[k]; when it SWAPS, low→BOTTOM[k], high→TOP[k].
/// - **Odd trailing input** `m-1` (only when `odd`) has no switch: it is TOP slot
///   `half` (= TOP slot `top_len-1`).
/// - **Output switch `k`** (`0 <= k < half`) serves outputs `2k` (low) and `2k+1`
///   (high), fed by TOP slot `k` and BOTTOM slot `k`. When it PASSES, TOP[k]→low,
///   BOTTOM[k]→high; when it SWAPS, the reverse.
/// - **Odd trailing output** `m-1` (only when `odd`) is fed by TOP slot `half`.
///
/// ## Algorithm (alternating-path / "looping")
///
/// Build the constraint that for each input/output PAIR the two endpoints land in
/// DIFFERENT sub-networks, then 2-color by walking alternating cycles. Seeding
/// each cycle at an undecided input switch (with the fixed output pair already
/// pinned) yields a consistent assignment; this is the classic Waksman routing.
///
/// AS-Waksman optimization: on **even** `m` the last output pair `(m−2, m−1)` is
/// pinned BEFORE the alternating-path propagation (output `m−2` from TOP, `m−1`
/// from BOTTOM), so it needs no output switch — the pin is just one more seed the
/// constraint propagation honours, making the fixed pair a *consequence* of a
/// consistent routing rather than an axiom that over-constrains it. `out_switches`
/// = `⌊(m−1)/2⌋` switch bits are returned (the pinned even-`m` pair excluded). On
/// **odd** `m` nothing extra is pinned (the odd-trailing output already carries no
/// switch), avoiding the corner-case clash between a pinned output pair and the
/// odd-trailing input that is forced TOP.
fn solve_waksman_level(
    m: usize,
    half: usize,
    odd: bool,
    out_switches: usize,
    perm: &[usize],
) -> WaksmanLevel {
    let top_len = m - half;
    // inv[o] = the input that must reach output o.
    let mut inv = vec![0usize; m];
    for (i, &o) in perm.iter().enumerate() {
        inv[o] = i;
    }

    // Decisions. `in_top[i]` = does input i go to the TOP sub-network?
    // `out_from_top[o]` = is output o fed FROM the top sub-network?
    let mut in_top = vec![Option::<bool>::None; m];
    let mut out_from_top = vec![Option::<bool>::None; m];

    // Pin the odd trailing input/output (forced TOP).
    if odd {
        in_top[m - 1] = Some(true);
        out_from_top[m - 1] = Some(true);
    } else {
        // AS-Waksman (even m): fix the LAST output pair (m−2, m−1) so it needs no
        // switch — output m−2 fed from TOP, m−1 from BOTTOM. This is the −1/level
        // saving. We pin it as a seed; the alternating-path propagation below makes
        // the rest consistent, so the fixed pair is a consequence of the routing.
        // (m ≥ 4 here: m == 2 returns early above, m == 3 is odd.)
        out_from_top[m - 2] = Some(true);
        out_from_top[m - 1] = Some(false);
    }

    // Helper: given an input i is decided, the OTHER input of its switch must go to
    // the opposite sub-network. Returns the partner input index (or None for the
    // odd trailing input which has no switch).
    let input_partner = |i: usize| -> Option<usize> {
        if odd && i == m - 1 {
            return None;
        }
        Some(if i.is_multiple_of(2) { i + 1 } else { i - 1 })
    };
    // Output partner within an output pair (its two endpoints are in different
    // sub-networks). The odd trailing output has no partner.
    let output_partner = |o: usize| -> Option<usize> {
        if odd && o == m - 1 {
            return None;
        }
        Some(if o.is_multiple_of(2) { o + 1 } else { o - 1 })
    };

    // (Plain Beneš/Waksman: no fixed output pair — every output switch is free,
    // which avoids the odd-`m` over-constraint the AS-Waksman fixed pair causes.)

    // Propagate decisions through the alternating constraints to a fixpoint:
    // setting an input/output's sub-network forces its pair-partner to the
    // opposite, and an output's source equals the sub-network of the input feeding
    // it (perm/inv linkage), and vice versa.
    //
    // We iterate: pick any undecided input, assign it TOP, then close the chain.
    loop {
        // First, flush all forced implications to a fixpoint.
        let mut changed = true;
        while changed {
            changed = false;
            // input decided -> output it feeds has same source; pair-partner opposite.
            for i in 0..m {
                if let Some(t) = in_top[i] {
                    let o = perm[i];
                    if out_from_top[o].is_none() {
                        out_from_top[o] = Some(t);
                        changed = true;
                    } else {
                        debug_assert_eq!(out_from_top[o], Some(t), "in→out source conflict");
                    }
                    if let Some(p) = input_partner(i) {
                        if in_top[p].is_none() {
                            in_top[p] = Some(!t);
                            changed = true;
                        } else {
                            debug_assert_eq!(in_top[p], Some(!t), "input pair conflict at {i}");
                        }
                    }
                }
            }
            // output decided -> input feeding it has same source; pair-partner opposite.
            for o in 0..m {
                if let Some(t) = out_from_top[o] {
                    let i = inv[o];
                    if in_top[i].is_none() {
                        in_top[i] = Some(t);
                        changed = true;
                    } else {
                        debug_assert_eq!(in_top[i], Some(t), "out→in source conflict");
                    }
                    if let Some(p) = output_partner(o) {
                        if out_from_top[p].is_none() {
                            out_from_top[p] = Some(!t);
                            changed = true;
                        } else {
                            debug_assert_eq!(
                                out_from_top[p],
                                Some(!t),
                                "output pair conflict at {o}"
                            );
                        }
                    }
                }
            }
        }
        // Seed a new chain at the first undecided input.
        match (0..m).find(|&i| in_top[i].is_none()) {
            Some(i) => in_top[i] = Some(true),
            None => break,
        }
    }

    // Now translate sub-network membership into switch bits and sub-network perms.
    // Input switch k passes iff input 2k goes TOP (low→top is the pass wiring).
    let mut in_swap = vec![false; half];
    for k in 0..half {
        let low_top = in_top[2 * k].unwrap();
        in_swap[k] = !low_top; // pass when low_top; swap otherwise
    }
    // Output switch k passes iff output 2k is fed from TOP (top→low is the pass
    // wiring). Only the first `out_switches` output pairs carry a switch; on even
    // m the last pair (k = half−1, outputs m−2/m−1) was pinned (TOP/BOTTOM) and has
    // no switch — that is the AS-Waksman −1/level saving.
    let mut out_swap = vec![false; out_switches];
    for k in 0..out_switches {
        let low_from_top = out_from_top[2 * k].unwrap();
        out_swap[k] = !low_from_top;
    }

    // Sub-network slot assignment.
    // TOP slot for input i: if i is the odd trailing input → top_len-1; else its
    // switch index i/2 (the input that went TOP occupies its switch's top slot).
    // BOTTOM slot for input i: its switch index i/2.
    // The sub-network permutation maps an input SLOT to the output SLOT it must
    // reach. Output o's TOP/BOTTOM slot: trailing output → top_len-1; else o/2.
    let input_top_slot = |i: usize| -> usize {
        if odd && i == m - 1 {
            top_len - 1
        } else {
            i / 2
        }
    };
    let input_bot_slot = |i: usize| -> usize { i / 2 };
    let output_top_slot = |o: usize| -> usize {
        if odd && o == m - 1 {
            top_len - 1
        } else {
            o / 2
        }
    };
    let output_bot_slot = |o: usize| -> usize { o / 2 };

    let mut top_perm = vec![0usize; top_len];
    let mut bot_perm = vec![0usize; half];
    for i in 0..m {
        let o = perm[i];
        if in_top[i].unwrap() {
            top_perm[input_top_slot(i)] = output_top_slot(o);
        } else {
            bot_perm[input_bot_slot(i)] = output_bot_slot(o);
        }
    }

    WaksmanLevel {
        in_swap,
        out_swap,
        top_perm,
        bot_perm,
    }
}

// =====================================================================
// Oblivious shuffle (the sound, real primitive)
// =====================================================================

/// MODELLED communication / round cost of one [`shuffle`] over `n` items, for a
/// REAL deployment (the in-process simulation does not pay it). Counted, never
/// measured — see the module cost note. No hard-coded numbers: derived from `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShuffleCost {
    /// Items shuffled.
    pub items: usize,
    /// AS-Waksman switch count (= conditional swaps / multiplications a real
    /// secret-control shuffle pays).
    pub switches: usize,
    /// Network depth in switch layers (≈ communication rounds with secret control
    /// bits; 0 extra rounds in the cleartext-control simulation).
    pub depth_rounds: usize,
}

impl ShuffleCost {
    /// Analytic cost model for `n` items (`n` ≥ 0). `switches` is the actual count
    /// from the built network; `depth_rounds = 2·⌈log₂ n⌉ − 1` for `n ≥ 2`.
    pub fn model(n: usize, switches: usize) -> Self {
        let depth_rounds = if n >= 2 {
            let l = ceil_log2(n).max(1);
            2 * l - 1
        } else {
            0
        };
        ShuffleCost {
            items: n,
            switches,
            depth_rounds,
        }
    }
}

/// `⌈log₂ n⌉` for `n >= 1` (0 for `n <= 1`).
fn ceil_log2(n: usize) -> usize {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as usize
    }
}

/// **Oblivious shuffle** of a secret-shared column by a uniformly-random secret
/// permutation, via the AS-Waksman network. Returns the shuffled column AND the
/// modelled cost.
///
/// Security (sound today): the output is a uniformly-random permutation of the
/// input multiset, and the permutation is independent of anything a `≤ t`-collusion
/// learns (the network topology is data-independent; the control bits are secret,
/// PRSS-generated in a real deployment). This is the primitive the set-returning
/// oblivious-join output path (bead sq-jnkm) and ~linear sort-merge join (bead
/// sq-ujz8) consume to destroy the per-pair match-graph leak (L2) before opening.
///
/// The random permutation is drawn from the `dealer`'s masking RNG (a CSPRNG in
/// production — sq-1vt; the seedable PRNG only under the test feature). The
/// cleartext permutation never leaves the routing layer.
pub fn shuffle(
    dealer: &mut ShamirDealer,
    mut col: SecretColumn,
) -> Result<(SecretColumn, ShuffleCost), MpcError> {
    let n = col.len();
    let net = WaksmanNetwork::new(n);
    if n <= 1 {
        return Ok((col, ShuffleCost::model(n, net.switch_count())));
    }
    let perm = random_permutation(dealer, n)?;
    let bits = net.route(&perm)?;
    net.apply(&mut col, &bits)?;
    Ok((col, ShuffleCost::model(n, net.switch_count())))
}

/// Public wrapper over the private `random_permutation` sampler so the
/// **network tier** ([`crate::transport`]) can draw the SAME uniformly-random
/// secret permutation the in-process [`shuffle`] uses, then route it through the
/// [`WaksmanNetwork`] to obtain the per-switch control bits it drives over the
/// wire. Same unbiased Fisher–Yates over the dealer's masking RNG, same
/// fail-closed `n > P` guard — the network tier must not re-implement a second
/// sampler. `` sq-bdbv.
pub fn sample_random_permutation(
    dealer: &mut ShamirDealer,
    n: usize,
) -> Result<Vec<usize>, MpcError> {
    random_permutation(dealer, n)
}

/// Draw a uniformly-random permutation of `0..n` via Fisher–Yates over the
/// dealer's masking RNG (CSPRNG in production). Unbiased: each index is chosen
/// uniformly from the remaining range with rejection sampling to kill modulo bias.
///
/// **Fails closed for oversized `n`.** The unbiased sampler can only draw a
/// uniform integer below the field modulus `P = 2^61 − 1` (it rejection-samples
/// field elements via `uniform_below`), so a column whose length exceeds `P` is
/// rejected with a typed [`MpcError`] rather than producing a biased permutation
/// or hanging. `n` is a row count, so `n > P` is unreachable in any real
/// deployment; this guards the boundary explicitly anyway. ``
fn random_permutation(dealer: &mut ShamirDealer, n: usize) -> Result<Vec<usize>, MpcError> {
    // Reject up front so the `as u64` cast below cannot lose information and the
    // sampler is never asked for a `bound > P` it cannot serve unbiasedly.
    let p = crate::field::P;
    if n as u128 > p as u128 {
        return Err(MpcError::Protocol(format!(
            "random_permutation: column length {n} exceeds field modulus P = {p}; \
             cannot sample an unbiased permutation"
        )));
    }
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        // `i + 1 <= n <= P` (checked above), so `(i + 1) as u64` is exact and
        // `uniform_below` always terminates.
        let j = uniform_below(dealer, (i + 1) as u64)? as usize; // j in [0, i]
        perm.swap(i, j);
    }
    Ok(perm)
}

/// Uniform integer in `[0, bound)` from the dealer's masking RNG, rejection-
/// sampled to remove modulo bias. Requires `1 <= bound <= P` (the field modulus):
/// the sampler draws uniform field elements in `[0, P)`, so a `bound > P` cannot
/// be served — the rejection `limit = P − (P % bound)` would be `0` and the loop
/// would never terminate. Such a `bound` is rejected with a typed [`MpcError`]
/// instead of hanging. ``
fn uniform_below(dealer: &mut ShamirDealer, bound: u64) -> Result<u64, MpcError> {
    debug_assert!(bound >= 1);
    let p = crate::field::P;
    if bound > p {
        return Err(MpcError::Protocol(format!(
            "uniform_below: bound {bound} exceeds field modulus P = {p}; \
             cannot rejection-sample without bias"
        )));
    }
    if bound == 1 {
        return Ok(0);
    }
    // Draw uniform field elements (already uniform in [0, P)) and reject the tail
    // so the modular reduction is unbiased. `bound <= P` guarantees `limit >= 1`
    // (the constant-term residue `P % bound < bound <= P`), so the loop makes
    // progress and terminates with probability 1.
    let limit = p - (p % bound);
    loop {
        let v = dealer.draw_fp().value();
        if v < limit {
            return Ok(v % bound);
        }
    }
}

// =====================================================================
// Oblivious sort (the data-independent SORTING NETWORK substrate)
// =====================================================================

/// MODELLED cost of one [`sort_by`] over `n` items (Batcher odd-even mergesort).
/// Counted, not measured. No hard-coded numbers: derived from `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortCost {
    /// Items sorted.
    pub items: usize,
    /// Compare-exchange gate count (= secure comparisons + conditional swaps).
    pub compare_exchanges: usize,
    /// Network depth in comparator layers (≈ communication rounds).
    pub depth_rounds: usize,
}

/// The compare-exchange **access pattern** a sort network executes: the ordered
/// list of `(i, j)` index pairs (`i < j`) it compares/swaps. A function of the
/// item count ONLY — the load-bearing obliviousness substrate (the sequence
/// reveals nothing about the secret data; asserted data-independent in the tests).
pub type AccessPattern = Vec<(usize, usize)>;

/// Result of [`sort_by`]: the sorted column, the [`AccessPattern`] executed, and
/// the modelled [`SortCost`].
pub type SortByResult = (SecretColumn, AccessPattern, SortCost);

/// Result of [`sort_with_keys`]: the sorted column, the keys co-permuted with it,
/// the [`AccessPattern`], and the modelled [`SortCost`].
pub type SortWithKeysResult = (SecretColumn, Vec<Fp>, AccessPattern, SortCost);

/// A 2-input *comparator* for the oblivious sort network: decides whether the
/// pair at positions `(i, j)` (`i < j`) must be exchanged so the column ends up
/// ascending. The sort network calls this on a **data-independent** sequence of
/// `(i, j)` index pairs (the obliviousness substrate); the comparator decides the
/// swap. Different impls have different security: the disclosed-key path is
/// [`sort_with_keys`] (sound); the test-only `SimulatedSecretComparator` (insecure)
/// stands in for the unbuilt secure comparator.
pub trait Comparator {
    /// `true` iff the sharings at `i` and `j` must be swapped to sort ascending
    /// (i.e. the value at `i` is greater than the value at `j`). Implementations
    /// must NOT depend on the index positions for the swap *decision* — only the
    /// keys' order may drive the result, so the *number* and *positions* of
    /// comparisons stay input-independent (the obliviousness invariant).
    fn must_swap(&self, col: &SecretColumn, i: usize, j: usize) -> Result<bool, MpcError>;
}

/// Sort a secret-shared column ascending via a **Batcher odd-even mergesort
/// network** and the given [`Comparator`]. Returns the sorted column, the
/// compare-exchange access pattern actually executed (the `(i, j)` sequence — a
/// function of `n` only, used by tests to assert data-independence), and the
/// modelled cost.
///
/// The access pattern is the obliviousness substrate the bead asks for: the
/// sequence of compared/swapped index pairs is **independent of the secret data**.
/// Whether the *comparator* is secure is the comparator's responsibility (and is
/// gated on degree reduction for genuinely-secret keys — see the module note).
pub fn sort_by<C: Comparator>(mut col: SecretColumn, cmp: &C) -> Result<SortByResult, MpcError> {
    let n = col.len();
    let net = SortingNetwork::new(n);
    for &(i, j) in net.compare_exchanges() {
        if cmp.must_swap(&col, i, j)? {
            col.swap(i, j);
        }
    }
    let cost = SortCost {
        items: n,
        compare_exchanges: net.gate_count(),
        depth_rounds: net.depth(),
    };
    Ok((col, net.compare_exchanges().to_vec(), cost))
}

/// A Batcher **odd-even mergesort** network: a data-INDEPENDENT sequence of
/// compare-exchange gates `(i, j)` with `i < j`, ascending. The gate sequence
/// depends ONLY on `n` (the obliviousness substrate). `O(n log² n)` gates in
/// `O(log² n)` layers — a sorting network (not a comparison sort), so the access
/// pattern is fully data-oblivious. (Bitonic is the power-of-two alternative;
/// odd-even mergesort handles arbitrary `n` in the same complexity class — the
/// design record §3 names both, vs the asymptotically-better Hamada radix sort
/// which needs the unbuilt bit-decomposition.)
#[derive(Debug, Clone)]
pub struct SortingNetwork {
    n: usize,
    gates: Vec<(usize, usize)>,
    depth: usize,
}

impl SortingNetwork {
    /// Build the odd-even mergesort network for `n` items (`n >= 0`).
    pub fn new(n: usize) -> Self {
        let mut gates = Vec::new();
        if n >= 2 {
            oddeven_mergesort(0, n, &mut gates);
        }
        let depth = layer_depth(&gates);
        SortingNetwork { n, gates, depth }
    }

    /// Width.
    pub fn width(&self) -> usize {
        self.n
    }

    /// The compare-exchange `(i, j)` gates in execution order. Data-independent.
    pub fn compare_exchanges(&self) -> &[(usize, usize)] {
        &self.gates
    }

    /// Number of compare-exchange gates.
    pub fn gate_count(&self) -> usize {
        self.gates.len()
    }

    /// Network depth (parallel comparator layers ≈ communication rounds).
    pub fn depth(&self) -> usize {
        self.depth
    }
}

/// Batcher odd-even mergesort over the half-open range `[lo, lo+n)`, emitting
/// ascending compare-exchange gates `(i, j)` with `i < j`. Handles arbitrary `n`.
///
/// We build the classic **power-of-two** odd-even mergesort network over the next
/// power of two `N >= n`, then **drop every comparator that touches an index
/// `>= n`**. This is the standard, provably-correct construction for arbitrary `n`
/// (the "truncated sorting network" result): conceptually the wires `[n, N)` hold
/// `+∞` sentinels that never move, so every dropped comparator was a no-op on the
/// real data — the surviving comparators sort the `n` real items. Validated by the
/// 0-1-principle test for every `n` and every binary input. The stride form below
/// is the textbook power-of-two odd-even merge (Batcher; Wikipedia "Batcher
/// odd–even mergesort"), correct precisely because `N` is a power of two.
fn oddeven_mergesort(lo: usize, n: usize, gates: &mut Vec<(usize, usize)>) {
    if n <= 1 {
        return;
    }
    let pow2 = n.next_power_of_two();
    let mut raw: Vec<(usize, usize)> = Vec::new();
    oddeven_pow2(pow2, &mut raw);
    // Translate to global wire indices and drop comparators touching a sentinel
    // wire (index >= n). Then offset by `lo`.
    for (i, j) in raw {
        if i < n && j < n {
            gates.push((lo + i.min(j), lo + i.max(j)));
        }
    }
}

/// The classic power-of-two Batcher odd-even mergesort network over `n` wires
/// `0..n` (`n` MUST be a power of two), emitting ascending `(i, j)` comparators.
fn oddeven_pow2(n: usize, gates: &mut Vec<(usize, usize)>) {
    debug_assert!(n.is_power_of_two());
    // Outer loop: `p` is the block size being merged (1, 2, 4, …, n/2).
    let mut p = 1;
    while p < n {
        let mut k = p;
        while k >= 1 {
            // Compare-exchange across stride `k` within the odd-even structure.
            for j in (k % p..n - k).step_by(2 * k) {
                for i in 0..k {
                    if (i + j) / (2 * p) == (i + j + k) / (2 * p) {
                        gates.push((i + j, i + j + k));
                    }
                }
            }
            k /= 2;
        }
        p *= 2;
    }
}

/// Greedy layering: assign each gate to the earliest layer in which neither of its
/// wires is already used, returning the number of layers (a parallel-round count).
fn layer_depth(gates: &[(usize, usize)]) -> usize {
    use std::collections::HashMap;
    let mut wire_layer: HashMap<usize, usize> = HashMap::new();
    let mut max_layer = 0usize;
    let mut any = false;
    for &(i, j) in gates {
        any = true;
        let li = wire_layer.get(&i).copied();
        let lj = wire_layer.get(&j).copied();
        let layer = match (li, lj) {
            (None, None) => 0,
            (Some(a), None) => a + 1,
            (None, Some(b)) => b + 1,
            (Some(a), Some(b)) => a.max(b) + 1,
        };
        wire_layer.insert(i, layer);
        wire_layer.insert(j, layer);
        max_layer = max_layer.max(layer);
    }
    if any {
        max_layer + 1
    } else {
        0
    }
}

// =====================================================================
// Disclosed-key sort (the sound ORDER BY path) + the secret-comparator seam
// =====================================================================

/// Sort a secret-shared column by **disclosed / public** sort keys, keeping each
/// key aligned with its sharing through every swap. Returns the sorted column, the
/// sorted keys, the compare-exchange access pattern (data-independent), and the
/// modelled cost.
///
/// This is the sound, shippable DISCLOSED-regime ORDER BY / GROUP BY path (design
/// record §3: many operators have a zero-extra-crypto realization when the sort
/// KEY is disclosed — the verifier-recompute regime): the secret VALUES stay
/// secret-shared and are obliviously permuted; only the public keys drive the
/// comparisons, so no secure comparator is needed. Stable: equal keys keep input
/// order (swap only on strict greater-than). Comparisons over public keys are NOT
/// data-oblivious *in the key values* (the keys are public by assumption) but the
/// access PATTERN over the sharings is still the fixed network — the payload
/// permutation reveals nothing the public keys did not already.
///
/// For a *hidden*-key sort (keys secret), drive [`sort_by`] with a secure
/// [`Comparator`] — which needs the degree-reduction-gated secure comparison (bead
/// sq-rrz4, itself gated on the degree-reduction round sq-dvuc); the only in-crate
/// `Comparator` over secret values today is the test-only insecure
/// `SimulatedSecretComparator`.
pub fn sort_with_keys(col: SecretColumn, keys: Vec<Fp>) -> Result<SortWithKeysResult, MpcError> {
    if col.len() != keys.len() {
        return Err(MpcError::Protocol(format!(
            "sort_with_keys: {} column entries != {} keys",
            col.len(),
            keys.len()
        )));
    }
    let n = col.len();
    let net = SortingNetwork::new(n);
    let mut col = col;
    let mut keys = keys;
    for &(i, j) in net.compare_exchanges() {
        if keys[i].value() > keys[j].value() {
            col.swap(i, j);
            keys.swap(i, j);
        }
    }
    let cost = SortCost {
        items: n,
        compare_exchanges: net.gate_count(),
        depth_rounds: net.depth(),
    };
    Ok((col, keys, net.compare_exchanges().to_vec(), cost))
}

/// **INSECURE, simulation/test-only** comparator that reconstructs the two values
/// to compare. Gated behind `#[cfg(any(test, feature = "insecure-test-rng"))]`.
///
/// It stands in for the unbuilt secure comparator (Rabbit / edaBits bit-
/// decomposition, which needs DN07/ATLAS degree reduction the backend lacks) so
/// the sort NETWORK and its data-independent access pattern (the substrate) can be
/// exercised end-to-end on genuinely secret-shared values. **It LEAKS the
/// comparison outcome and reconstructs intermediate values — NO security is
/// claimed.** Its name and gate are the warning; it must NEVER drive a real sort.
#[cfg(any(test, feature = "insecure-test-rng"))]
#[derive(Debug, Clone)]
pub struct SimulatedSecretComparator {
    threshold: usize,
}

#[cfg(any(test, feature = "insecure-test-rng"))]
impl SimulatedSecretComparator {
    /// Build a simulated (INSECURE) comparator for a column shared at threshold
    /// `t`. Test/benchmark only.
    pub fn new(threshold: usize) -> Self {
        SimulatedSecretComparator { threshold }
    }
}

#[cfg(any(test, feature = "insecure-test-rng"))]
impl Comparator for SimulatedSecretComparator {
    fn must_swap(&self, col: &SecretColumn, i: usize, j: usize) -> Result<bool, MpcError> {
        // INSECURE: reconstruct both values and compare in the clear. The explicit
        // stand-in for the unbuilt secure comparator (gated on degree reduction).
        let a = crate::robust::reconstruct_robust(&col[i], self.threshold)?;
        let b = crate::robust::reconstruct_robust(&col[j], self.threshold)?;
        Ok(a.value() > b.value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shamir::ShamirBackend;

    // ---- helpers --------------------------------------------------------

    fn share_col(dealer: &mut ShamirDealer, vals: &[u64]) -> SecretColumn {
        vals.iter().map(|&v| dealer.share(Fp::new(v))).collect()
    }

    fn open_col(t: usize, col: &SecretColumn) -> Vec<u64> {
        col.iter()
            .map(|s| crate::robust::reconstruct_robust(s, t).unwrap().value())
            .collect()
    }

    fn sorted_asc(mut v: Vec<u64>) -> Vec<u64> {
        v.sort_unstable();
        v
    }

    fn multiset(mut v: Vec<u64>) -> Vec<u64> {
        v.sort_unstable();
        v
    }

    /// Visit every permutation of `p` (recursive generation).
    fn permute_each<F: FnMut(&[usize])>(p: &mut Vec<usize>, k: usize, f: &mut F) {
        if k == p.len() {
            f(p);
            return;
        }
        for i in k..p.len() {
            p.swap(k, i);
            permute_each(p, k + 1, f);
            p.swap(k, i);
        }
    }

    // ---- AS-Waksman network: topology is data-independent ----------------

    #[test]
    fn waksman_topology_depends_only_on_n() {
        let a = WaksmanNetwork::new(8);
        let b = WaksmanNetwork::new(8);
        assert_eq!(a.switches(), b.switches());
        assert!(a.switch_count() > 0);
    }

    /// The switch list a network exposes must equal the switch list produced when
    /// routing ANY permutation (topology is permutation-independent — the
    /// obliviousness invariant of the shuffle). If routing two different perms
    /// produced different switch SETS, the access pattern would leak the perm.
    #[test]
    fn waksman_topology_is_permutation_independent() {
        for n in 1..=8usize {
            let net = WaksmanNetwork::new(n);
            let mut perm: Vec<usize> = (0..n).collect();
            permute_each(&mut perm, 0, &mut |p| {
                let (switches, bits) = route_recording(p);
                assert_eq!(
                    switches,
                    *net.switches(),
                    "n={n} perm={p:?}: topology depends on data"
                );
                assert_eq!(bits.len(), switches.len());
            });
        }
    }

    #[test]
    fn waksman_switch_count_is_log_linear() {
        // The AS-Waksman form is O(n log n); assert it stays within a loose
        // 2·n·⌈log₂ n⌉ bound and is non-trivial for n>1.
        for n in [2usize, 3, 4, 5, 7, 8, 16, 17, 31, 32] {
            let net = WaksmanNetwork::new(n);
            let l = ceil_log2(n);
            let upper = 2 * n * l; // loose O(n log n) upper bound
            assert!(
                net.switch_count() <= upper,
                "n={n}: {} > {upper}",
                net.switch_count()
            );
            assert!(net.switch_count() >= 1, "n={n}");
        }
    }

    /// AS-Waksman optimal switch count (bead sq-hny9): the network realizes the
    /// canonical arbitrary-size recurrence `W(n) = W(⌈n/2⌉) + W(⌊n/2⌋) + (n−1)`
    /// (Beauquier–Darrot), shaving one switch per even-`m` recursion level vs. the
    /// plain Beneš form. This is the `~n·⌈log₂ n⌉ − n + 1` optimal count.
    #[test]
    fn waksman_switch_count_is_as_waksman_optimal() {
        fn canonical(n: usize) -> usize {
            match n {
                0 | 1 => 0,
                2 => 1,
                _ => canonical(n.div_ceil(2)) + canonical(n / 2) + (n - 1),
            }
        }
        for n in 0..=64usize {
            assert_eq!(
                WaksmanNetwork::new(n).switch_count(),
                canonical(n),
                "n={n}: switch count is not the AS-Waksman optimum"
            );
        }
        // Spot-check known optimal values from the literature.
        assert_eq!(WaksmanNetwork::new(4).switch_count(), 5);
        assert_eq!(WaksmanNetwork::new(8).switch_count(), 17);
        assert_eq!(WaksmanNetwork::new(16).switch_count(), 49);
    }

    /// THE shuffle correctness + routing test: for every permutation of small n,
    /// routing it and applying the network must realize EXACTLY that permutation.
    /// Full support (the anti-Benes-bias property the bead requires verified): for
    /// small `n`, EVERY one of the `n!` permutations is realized by routing it
    /// through the network — none is unreachable. Combined with
    /// `waksman_routes_every_permutation_correctly` (which checks each routes to
    /// EXACTLY itself) this proves the realized-permutation map is a bijection onto
    /// `S_n`, so a uniform random `perm` gives an unbiased shuffle even after the
    /// AS-Waksman switch-count reduction.
    #[test]
    fn waksman_has_full_support_after_optimization() {
        use std::collections::HashSet;
        for n in 1..=7usize {
            let net = WaksmanNetwork::new(n);
            let mut realized: HashSet<Vec<usize>> = HashSet::new();
            let mut perm: Vec<usize> = (0..n).collect();
            permute_each(&mut perm, 0, &mut |p| {
                let bits = net.route(p).expect("route ok");
                let mut labels: Vec<usize> = (0..n).collect();
                for (sw, &bit) in net.switches().iter().zip(&bits) {
                    if bit {
                        labels.swap(sw.a, sw.b);
                    }
                }
                // labels[o] = input landing at output o; record the realized perm.
                realized.insert(labels);
            });
            let factorial: usize = (1..=n).product();
            assert_eq!(
                realized.len(),
                factorial,
                "n={n}: AS-Waksman net does not reach all {factorial} permutations (biased)"
            );
        }
    }

    #[test]
    fn waksman_routes_every_permutation_correctly() {
        for n in 1..=8usize {
            let net = WaksmanNetwork::new(n);
            let mut perm: Vec<usize> = (0..n).collect();
            permute_each(&mut perm, 0, &mut |p| {
                let bits = net.route(p).expect("route ok");
                // Apply to identity-labeled wires; labels[o] = input that landed at
                // output o. We require input i to land at output p[i].
                let mut labels: Vec<usize> = (0..n).collect();
                for (sw, &bit) in net.switches().iter().zip(&bits) {
                    if bit {
                        labels.swap(sw.a, sw.b);
                    }
                }
                for (i, &pi) in p.iter().enumerate() {
                    assert_eq!(
                        labels[pi], i,
                        "n={n} perm={p:?}: input {i} should land at {pi}"
                    );
                }
            });
        }
    }

    // ---- Oblivious shuffle: valid permutation of the multiset -------------

    #[test]
    fn shuffle_preserves_multiset() {
        let backend = ShamirBackend::new_seeded(5, 0xABCD).unwrap();
        let mut dealer = backend.dealer();
        let t = dealer.threshold();
        let vals = vec![10u64, 20, 30, 40, 50, 60, 70];
        let col = share_col(&mut dealer, &vals);
        let (shuffled, cost) = shuffle(&mut dealer, col).unwrap();
        assert_eq!(multiset(open_col(t, &shuffled)), multiset(vals.clone()));
        assert_eq!(cost.items, vals.len());
        assert!(cost.switches > 0);
    }

    #[test]
    fn shuffle_actually_reorders_sometimes() {
        // Over several shuffles of a distinct-valued column, at least one differs
        // from the input order. P(all identity) across 8 trials is negligible.
        let t = ShamirBackend::new_seeded(5, 1).unwrap().threshold();
        let vals = vec![1u64, 2, 3, 4, 5, 6, 7];
        let mut any_reordered = false;
        for seed in 0..8u64 {
            let b = ShamirBackend::new_seeded(5, 100 + seed).unwrap();
            let mut dealer = b.dealer();
            let col = share_col(&mut dealer, &vals);
            let (sh, _) = shuffle(&mut dealer, col).unwrap();
            if open_col(t, &sh) != vals {
                any_reordered = true;
                break;
            }
        }
        assert!(
            any_reordered,
            "shuffle never reordered across 8 trials — suspicious"
        );
    }

    #[test]
    fn shuffle_edge_cases_empty_and_singleton() {
        let backend = ShamirBackend::new_seeded(3, 7).unwrap();
        let mut dealer = backend.dealer();
        let (e, c0) = shuffle(&mut dealer, Vec::new()).unwrap();
        assert!(e.is_empty());
        assert_eq!(c0.switches, 0);
        let one = share_col(&mut dealer, &[42]);
        let t = backend.threshold();
        let (s1, c1) = shuffle(&mut dealer, one).unwrap();
        assert_eq!(open_col(t, &s1), vec![42]);
        assert_eq!(c1.switches, 0);
    }

    /// Distribution sanity (not a strict uniformity proof): over many shuffles of
    /// [0,1,2] every one of the 6 permutations must appear (no permutation is
    /// unreachable — the Benes-bias failure mode the AS-Waksman choice avoids).
    #[test]
    fn shuffle_reaches_all_permutations_of_three() {
        use std::collections::HashSet;
        let mut seen: HashSet<Vec<u64>> = HashSet::new();
        let vals = vec![0u64, 1, 2];
        for seed in 0..5000u64 {
            let b = ShamirBackend::new_seeded(3, seed).unwrap();
            let t = b.threshold();
            let mut dealer = b.dealer();
            let col = share_col(&mut dealer, &vals);
            let (sh, _) = shuffle(&mut dealer, col).unwrap();
            seen.insert(open_col(t, &sh));
            if seen.len() == 6 {
                break;
            }
        }
        assert_eq!(
            seen.len(),
            6,
            "AS-Waksman shuffle did not reach all 3! perms: {seen:?}"
        );
    }

    /// A 4-item shuffle must also reach all 24 permutations (exercises the odd/even
    /// recursive split with an output column).
    #[test]
    fn shuffle_reaches_all_permutations_of_four() {
        use std::collections::HashSet;
        let mut seen: HashSet<Vec<u64>> = HashSet::new();
        let vals = vec![0u64, 1, 2, 3];
        for seed in 0..200_000u64 {
            let b = ShamirBackend::new_seeded(3, seed).unwrap();
            let t = b.threshold();
            let mut dealer = b.dealer();
            let col = share_col(&mut dealer, &vals);
            let (sh, _) = shuffle(&mut dealer, col).unwrap();
            seen.insert(open_col(t, &sh));
            if seen.len() == 24 {
                break;
            }
        }
        assert_eq!(
            seen.len(),
            24,
            "shuffle did not reach all 4! perms (saw {})",
            seen.len()
        );
    }

    // ---- Sorting network: data-independent access pattern -----------------

    #[test]
    fn sort_network_access_pattern_is_data_independent() {
        // THE obliviousness assertion: the compare-exchange (i,j) sequence is a
        // function of n ONLY — two different secret payloads of the same length
        // produce the IDENTICAL access pattern.
        let backend = ShamirBackend::new_seeded(5, 9).unwrap();
        let t = backend.threshold();
        let mut dealer = backend.dealer();
        let col_a = share_col(&mut dealer, &[5u64, 1, 9, 3, 7, 2, 8, 4]);
        let col_b = share_col(&mut dealer, &[8u64, 8, 8, 8, 1, 1, 1, 1]);
        let cmp = SimulatedSecretComparator::new(t);
        let (_sa, ap_a, _) = sort_by(col_a, &cmp).unwrap();
        let (_sb, ap_b, _) = sort_by(col_b, &cmp).unwrap();
        assert_eq!(
            ap_a, ap_b,
            "sort access pattern leaked data (differed across payloads)"
        );
        assert_eq!(ap_a, SortingNetwork::new(8).compare_exchanges());
    }

    #[test]
    fn oddeven_mergesort_network_actually_sorts_all_small_inputs() {
        // 0-1 principle: a comparison network that sorts all binary inputs sorts
        // all inputs. Validate every binary input for small n.
        for n in 0..=9usize {
            let net = SortingNetwork::new(n);
            for mask in 0u32..(1u32 << n) {
                let mut v: Vec<u32> = (0..n).map(|i| (mask >> i) & 1).collect();
                for &(i, j) in net.compare_exchanges() {
                    if v[i] > v[j] {
                        v.swap(i, j);
                    }
                }
                assert!(
                    v.windows(2).all(|w| w[0] <= w[1]),
                    "n={n} mask={mask:b} not sorted: {v:?}"
                );
            }
        }
    }

    #[test]
    fn sort_by_secret_column_sorts_and_preserves_multiset() {
        let backend = ShamirBackend::new_seeded(5, 0xF00D).unwrap();
        let t = backend.threshold();
        let mut dealer = backend.dealer();
        for vals in [
            vec![3u64, 1, 2],
            vec![9u64, 8, 7, 6, 5, 4, 3, 2, 1],
            vec![5u64, 5, 1, 9, 5, 1],
            vec![100u64],
            vec![],
        ] {
            let col = share_col(&mut dealer, &vals);
            let cmp = SimulatedSecretComparator::new(t);
            let (sorted, _ap, cost) = sort_by(col, &cmp).unwrap();
            assert_eq!(
                open_col(t, &sorted),
                sorted_asc(vals.clone()),
                "sort wrong for {vals:?}"
            );
            assert_eq!(cost.items, vals.len());
        }
    }

    #[test]
    fn sort_with_disclosed_keys_sound_path() {
        let backend = ShamirBackend::new_seeded(5, 0x1234).unwrap();
        let t = backend.threshold();
        let mut dealer = backend.dealer();
        let payload = vec![111u64, 222, 333, 444];
        let keys = vec![Fp::new(40), Fp::new(10), Fp::new(30), Fp::new(20)];
        let col = share_col(&mut dealer, &payload);
        let (sorted_col, sorted_keys, _ap, _cost) = sort_with_keys(col, keys).unwrap();
        let ks: Vec<u64> = sorted_keys.iter().map(|k| k.value()).collect();
        assert_eq!(ks, vec![10, 20, 30, 40]);
        // key 10→222, 20→444, 30→333, 40→111.
        assert_eq!(open_col(t, &sorted_col), vec![222, 444, 333, 111]);
    }

    #[test]
    fn sort_is_stable_on_equal_public_keys() {
        let backend = ShamirBackend::new_seeded(5, 5).unwrap();
        let t = backend.threshold();
        let mut dealer = backend.dealer();
        let payload = vec![1u64, 2, 3, 4];
        let keys = vec![Fp::new(7); 4]; // all equal
        let col = share_col(&mut dealer, &payload);
        let (sorted_col, _k, _ap, _c) = sort_with_keys(col, keys).unwrap();
        assert_eq!(
            open_col(t, &sorted_col),
            vec![1, 2, 3, 4],
            "equal keys must keep input order"
        );
    }

    // ---- Cost model ------------------------------------------------------

    #[test]
    fn shuffle_cost_model_is_derived_from_n() {
        let net = WaksmanNetwork::new(16);
        let c = ShuffleCost::model(16, net.switch_count());
        assert_eq!(c.items, 16);
        assert_eq!(c.switches, net.switch_count());
        // depth = 2*ceil(log2 16) - 1 = 2*4 - 1 = 7.
        assert_eq!(c.depth_rounds, 7);
    }

    #[test]
    fn sort_cost_model_matches_network() {
        let net = SortingNetwork::new(16);
        let (_c, _ap, cost) = {
            let backend = ShamirBackend::new_seeded(3, 1).unwrap();
            let mut dealer = backend.dealer();
            let col = share_col(&mut dealer, &[0u64; 16]);
            let t = backend.threshold();
            sort_by(col, &SimulatedSecretComparator::new(t)).unwrap()
        };
        assert_eq!(cost.compare_exchanges, net.gate_count());
        assert_eq!(cost.depth_rounds, net.depth());
        assert!(cost.depth_rounds > 0);
    }

    // ---- Validation / error paths ----------------------------------------

    #[test]
    fn route_rejects_non_permutation() {
        let net = WaksmanNetwork::new(4);
        assert!(net.route(&[0, 1, 2, 2]).is_err()); // duplicate
        assert!(net.route(&[0, 1, 2]).is_err()); // wrong length
        assert!(net.route(&[0, 1, 2, 9]).is_err()); // out of range
    }

    #[test]
    fn apply_rejects_mismatched_lengths() {
        let net = WaksmanNetwork::new(4);
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let mut dealer = backend.dealer();
        let mut col = share_col(&mut dealer, &[1, 2, 3, 4]);
        assert!(net.apply(&mut col, &[true]).is_err()); // too few bits
        let mut short = share_col(&mut dealer, &[1, 2]);
        let bits = vec![false; net.switch_count()];
        assert!(net.apply(&mut short, &bits).is_err()); // wrong column len
    }

    #[test]
    fn sort_with_keys_rejects_length_mismatch() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let mut dealer = backend.dealer();
        let col = share_col(&mut dealer, &[1, 2, 3]);
        assert!(sort_with_keys(col, vec![Fp::new(1)]).is_err());
    }

    // FAIL-CLOSED: `uniform_below` must reject a `bound > P` with a typed error
    // instead of spinning forever (the `limit = P − (P % bound) = 0` non-
    // termination Copilot flagged). For `1 <= bound <= P` it must terminate and
    // return a value in range. `` (Copilot review #88).
    #[test]
    fn uniform_below_rejects_bound_above_modulus_and_terminates_below() {
        let backend = ShamirBackend::new_seeded(5, 0xBEEF).unwrap();
        let mut dealer = backend.dealer();
        let p = crate::field::P;
        // bound > P → typed error, NOT a hang.
        assert!(uniform_below(&mut dealer, p + 1).is_err());
        assert!(uniform_below(&mut dealer, u64::MAX).is_err());
        // bound == P is the largest serviceable bound: terminates, in range.
        let v = uniform_below(&mut dealer, p).expect("bound == P is serviceable");
        assert!(v < p);
        // Small bounds terminate and stay in range.
        for bound in [1u64, 2, 3, 7, 1000] {
            let v = uniform_below(&mut dealer, bound).expect("small bound terminates");
            assert!(v < bound, "uniform_below({bound}) = {v} out of range");
        }
    }

    // FAIL-CLOSED: `random_permutation` rejects a column length > P (would force
    // an unserviceable `uniform_below` bound). Reachable sizes produce a valid
    // permutation of `0..n`. `` (Copilot review #88).
    #[test]
    fn random_permutation_rejects_oversized_and_is_valid_below() {
        let backend = ShamirBackend::new_seeded(5, 0x1234).unwrap();
        let mut dealer = backend.dealer();
        let p = crate::field::P as usize;
        // Length > P → typed error (cannot construct such a Vec, but the guard is
        // checked from the count before any allocation of indices beyond it).
        assert!(random_permutation(&mut dealer, p.saturating_add(1)).is_err());
        // Reachable n → a valid permutation of 0..n.
        for n in [0usize, 1, 2, 5, 16] {
            let perm = random_permutation(&mut dealer, n).expect("valid n");
            let mut sorted = perm.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                (0..n).collect::<Vec<_>>(),
                "n={n}: not a permutation"
            );
        }
    }
}
