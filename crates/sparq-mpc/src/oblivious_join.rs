// sq-jnkm: oblivious result-size protection + match-bit aggregation
// output path for SET-returning hidden joins. Closes leaks L1 (true result
// cardinality) and L2 (the per-pair match graph / key fan-out) that the per-pair
// `HiddenValueJoin` open exposes, building on the oblivious shuffle substrate
// (sq-18lk, `crate::oblivious::shuffle`). Native-only; in-process multi-party
// simulation; communication cost MODELLED not measured. Authored under the Opus
// 4.8 fallback model — re-review when Fable returns.
//! Oblivious output path for set-returning hidden joins — **result-size
//! protection + match-bit aggregation** (bead sq-jnkm).
//!
//! ## The leak this closes, and why the per-pair join needs it
//!
//! [`crate::join::HiddenValueJoin`] hides the join KEY values but its output path
//! leaks two things RQ2a wants hidden (research record §4.1):
//!
//! - **L1 — final result cardinality.** It emits exactly the true number of
//!   matches; `canonicalize_rows` sorts but never pads. An observer of the
//!   protocol learns `|result|` exactly.
//! - **L2 — per-pair match graph / key fan-out.** `secure_equal` *opens* the
//!   masked product `m = d·r` for every `(i, j)` candidate pair; that open IS the
//!   match bit at `(i, j)`. The set of matching pairs is the bipartite match graph,
//!   which fingerprints the (hidden) key fan-out / multiplicity distribution.
//!
//! The standard oblivious-join output path (research record §4.3, "Private-key
//! join returning a SET") closes both with the substrate this module consumes:
//!
//! 1. compute a per-candidate **match bit** kept SECRET-SHARED (never opened
//!    per-pair → L2 destroyed at source);
//! 2. **oblivious-select** each candidate slot to `bit · real_tag + (1 − bit) · 0`
//!    so a non-match becomes an indistinguishable dummy;
//! 3. **oblivious-shuffle** ([`crate::oblivious::shuffle`]) the slot column so the
//!    output position reveals nothing about which input pair produced it (the
//!    residual access-pattern half of L2);
//! 4. reveal **exactly a public padded bound `B` of slots** — never the true match
//!    count (→ L1 bounded to `B`, not exact).
//!
//! ## What is SOUND TODAY vs what is honestly GATED
//!
//! The empirical-honesty line is the same one the substrate draws between its
//! sound shuffle and its degree-reduction-gated secret comparator.
//!
//! ### Sound today (shipped here)
//!
//! - **The oblivious output TRANSFORM** — steps 2–4 above — given a secret-shared
//!   match bit per candidate. The per-slot select `bit·real_tag` is a **single**
//!   Shamir multiplication ([`crate::shamir::mul_shares_raw`]) plus local adds: no
//!   multiplication CHAIN, so it needs **no degree reduction** and is buildable on
//!   the single-product backend — exactly like
//!   `crate::join::HiddenValueJoin::secure_equal`. The shuffle is the sound
//!   substrate; the padded-prefix reveal is a degree-`2t` open of `B` slots. This
//!   is the structural obliviousness the bead asks for, and it holds at the
//!   honest-majority Shamir layer.
//! - **The DISCLOSED-key candidate path** — [`oblivious_set_output`] /
//!   [`oblivious_join_output`] from a candidate whose match bit was decided by a
//!   **public / disclosed-key** equi-join (computed OUTSIDE the crypto core,
//!   convention #4). The bit is public so we lift it to a trivial sharing with NO
//!   secure comparison; the transform then hides, from the parties, *which*
//!   candidates matched and the exact count (only `B` is revealed). This is fully
//!   sound and shippable now.
//!
//! ### sq-xhaw — the HIDDEN-key match bit is now REALISED (gate landed)
//!
//! - **Deriving the secret match bit from genuinely-secret keys WITHOUT opening
//!   it.** Feeding step 1 in the HIDDEN-key regime needs a secure
//!   equality-to-SHARED-bit. That was previously gated on the BGW/DN
//!   **degree-reduction** round (bead **sq-dvuc**) and the secure-comparison
//!   machinery on it (bead **sq-rrz4**) — the same gate the substrate's secret-key
//!   sort sat behind. **Those have landed**, and
//!   [`crate::compare::secure_equal_to_bit`] now produces `[1{a == b}]` as a fresh
//!   degree-`t` secret-shared bit that is NEVER opened (a bit-decomposition + an
//!   AND-tree of secure multiplications). So the hidden-key entry point
//!   ([`oblivious_set_output_hidden_keys`]) and the row-carrying production path
//!   ([`crate::join::HiddenValueJoin::fully_oblivious_batched_join`]) feed that
//!   shared bit as a [`MatchBit::SecretShared`] selector — no per-pair open, the
//!   L2 match-graph leak is closed at the DECISION (not just at the output). This
//!   is honest-majority / semi-honest only; external soundness sign-off is still
//!   pending (sq-qhy4).
//!
//! ## Residual leakage (state it loudly — empirical-honesty rule)
//!
//! This is the **padded-bound** (exact-obliviousness) output, NOT differential
//! privacy (DP cardinality / ε-budget is the SEPARATE follow-up bead **sq-shk5**;
//! deliberately not done here).
//!
//! - **To the PARTIES / the protocol transcript:** the output reveals only the
//!   PUBLIC bound `B` and that the true count is `≤ B`. It does NOT reveal the true
//!   count, which slots matched, or which input pair produced which output row
//!   (the shuffle destroyed position→candidate linkage). L1 is bounded to `B`; L2
//!   is destroyed.
//! - **To the authorized RESULT RECIPIENT** (who must filter the tagged dummies to
//!   use the result): the true count IS learned — it equals the number of
//!   non-dummy rows after filtering. This is inherent to a padded-bound scheme: the
//!   consumer of a result set learns its size. Hiding the count from the *recipient*
//!   too is exactly the job of the DP cardinality mode (sq-shk5), which adds noise
//!   + extra dummies on top of this transform; it is out of scope here.
//! - **`B` itself is public and caller-agreed.** Choosing `B` too tight FAILS
//!   CLOSED (we never silently drop matches — a candidate count `> B` is an error,
//!   never a truncation that would corrupt the result); choosing it loose costs `B`
//!   opens. `B` leaks an upper bound on the result size by construction.
//! - **L3 (input cardinalities `|L|`, `|R|`) and the candidate count** are public
//!   (standard MPC assumption; research record §4.1). We do not claim to hide them.
//! - Malicious security is unchanged from the Shamir layer: at `n = 2t + 1` the
//!   degree-`2t` opens have zero RS redundancy (research record §4.1 D×A; bead
//!   sq-6d6g IT-MAC is the named fix). Semi-honest only.
//!
//! ## Communication / round cost (MODELLED — counted, not measured)
//!
//! [`ObliviousOutputCost`] reports what a real deployment would pay: `B` tag
//! select multiplications plus `B · limb_capacity` payload-limb select
//! multiplications — the payload is selected against the dummy encoding limb by
//! limb, so a slot costs `1 + limb_capacity` multiplications, all still in ONE
//! parallel round — the shuffle's switch count +
//! depth ([`crate::oblivious::ShuffleCost`]) — with the switch count scaled by the
//! packed-entry WIDTH (tag + payload limbs), since a real secret-control shuffle
//! pays a conditional swap per field element per switch and the payloads ride
//! through the shuffle as secret limbs (Copilot review #93) — and `B` degree-`2t`
//! opens. No hard-coded numbers: derived from `B`, the payload width, and the
//! built network (AGENTS.md).

use crate::field::Fp;
use crate::oblivious::{SecretColumn, ShuffleCost};
use crate::partial::{HolderId, MpcError, PartialResult};
use crate::randomness::{
    checked_oblivious_shuffle, checked_vss_own_input, require_source_matches, DistributedRandomness,
};
use crate::shamir::{self, ShamirBackend, Share};
use oxrdf::{Term, Variable};
use std::str::FromStr;

/// Bytes packed per `Fp` limb when a payload term is encoded into the secret
/// domain to be carried THROUGH the oblivious shuffle (Copilot review #93,
/// thread 1). `Fp` is the Mersenne prime `2^61 − 1` ([`crate::field::P`]), so a
/// field element holds any 60-bit value; we pack 7 bytes (56 bits) per limb,
/// well under the modulus, so every limb is a canonical residue with no reduction
/// and the byte→limb→byte round-trip is exact.
const BYTES_PER_LIMB: usize = 7;

/// The fixed value packed into padding limbs (the limbs that bring a short
/// entry up to the uniform width). The exact byte length is carried in an explicit
/// LENGTH-PREFIX limb (see [`bytes_to_limbs`]), so padding content is irrelevant to
/// decoding — `0` is used purely for determinism.
const PAD_LIMB: u64 = 0;

/// One disclosed payload row — a column vector of nullable `oxrdf` terms, the same
/// row shape `sparq_engine::QueryResult` / [`PartialResult`] use (`None` = unbound).
type PayloadRow = Vec<Option<Term>>;

/// Internal result of [`shuffle_with_payloads`]: the co-permuted tag column and
/// OPENED payload-limb blocks (one block per slot, in shuffled order — decoding is
/// the caller's step, after it has classified the slot by its tag), the shuffle's
/// modelled cost, and the number of payload-limb select multiplications the stage
/// performed (one per limb per slot).
type ShuffledColumn = (SecretColumn, Vec<Vec<Fp>>, ShuffleCost, usize);

/// A single candidate row entering the oblivious output path: the disclosed
/// payload terms that would be emitted **on a match**, plus the match decision.
///
/// The payload is cleartext `oxrdf` terms (the disclosure-minimisation rule keeps
/// only the *key* hidden — convention #4; the payload columns are disclosed on a
/// match). Whether this candidate matched is carried as a [`MatchBit`], which is
/// either a public bit (disclosed-key regime, sound today) or a secret-shared bit
/// (hidden-key regime, gated on secure-compare — see module docs).
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The payload terms emitted if this candidate matches (the join's disclosed
    /// output columns for this `(i, j)` pair).
    pub payload: Vec<Option<Term>>,
    /// Whether this candidate is a match — see [`MatchBit`].
    pub matched: MatchBit,
}

/// The match decision for a [`Candidate`].
///
/// - [`MatchBit::Public`] — the bit was decided OUTSIDE the crypto core (a
///   disclosed-key / public equi-join, convention #4). Sound today: we lift it to
///   a trivial degree-`t` sharing with no secure comparison.
/// - [`MatchBit::SecretShared`] — a genuinely-secret 0/1 sharing produced by a
///   secure equality-to-shared-bit. Consuming it is sound, and **producing** it
///   from secret keys without opening it is now realised by
///   [`crate::compare::secure_equal_to_bit`] (sq-xhaw; the sq-rrz4/sq-dvuc gate it
///   needed has landed) — used by [`oblivious_set_output_hidden_keys`] and
///   [`crate::join::HiddenValueJoin::fully_oblivious_batched_join`]. (The older
///   `secure_equal` only *opens* the bit, which is the L2 leak this path closes, so
///   it is NOT used here.)
#[derive(Debug, Clone)]
pub enum MatchBit {
    /// Match bit decided in the clear (disclosed-key regime).
    Public(bool),
    /// Match bit as a secret-shared 0/1 field element (`Fp::one()` / `Fp::zero()`),
    /// one [`Share`] per party. Must be a sharing of exactly 0 or 1; this is the
    /// caller's obligation (a secure equality-to-bit guarantees it by construction).
    SecretShared(Vec<Share>),
}

/// MODELLED communication / round cost of one [`oblivious_set_output`] over `bound`
/// output slots. Counted, never measured — see the module cost note. No hard-coded
/// numbers: derived from `bound` and the built shuffle network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObliviousOutputCost {
    /// The public padded bound `B` (number of slots revealed).
    pub bound: usize,
    /// Tag select multiplications: one per slot (`bit·real_tag`), all in a single
    /// parallel round.
    pub select_mults: usize,
    /// Payload-limb select multiplications: one per packed payload limb per slot
    /// (`PAD + sel·(limb − PAD)`), all in the same parallel round as the tag
    /// select. So a slot costs `1 + limb_capacity` multiplications in total, where
    /// `limb_capacity` is the public uniform packed-payload width — the payload is
    /// selected against the dummy encoding limb by limb, not carried through
    /// unselected.
    pub payload_select_mults: usize,
    /// Degree-`2t` opens of the slot TAGS performed at reveal (one per slot). The
    /// payload limbs are opened too (also at degree `2t` now that they are
    /// selected products), inside the shuffle step; this field counts the tag
    /// opens only, as it always has.
    pub opens: usize,
    /// The shuffle's cost over `B` slots (switch count + network depth). The
    /// `switches` count is scaled by the packed-entry WIDTH (tag + payload limbs):
    /// a real secret-control shuffle pays a conditional swap per field element per
    /// switch, and the payloads now ride through the shuffle as secret limbs, so a
    /// single-element switch count would understate the work (Copilot review #93,
    /// thread 2).
    pub shuffle: ShuffleCost,
}

/// One revealed output slot: either a real joined row or a tagged dummy.
///
/// After the oblivious shuffle the parties open exactly `B` of these IN SHUFFLED
/// ORDER. A [`OutputSlot::Dummy`] is the padding that hides the true match count
/// from the parties; the authorized recipient filters dummies to obtain the result
/// (and only then learns the true count — see the module's residual-leakage note).
#[derive(Debug, Clone, PartialEq)]
pub enum OutputSlot {
    /// A real joined payload row (this candidate matched).
    Row(Vec<Option<Term>>),
    /// A dummy slot (a non-match or pure padding) — filtered by the recipient.
    Dummy,
}

/// The result of [`oblivious_set_output`]: the `B` revealed slots (in shuffled
/// order) and the modelled cost.
pub type ObliviousOutput = (Vec<OutputSlot>, ObliviousOutputCost);

/// **The oblivious set-returning output path** (bead sq-jnkm).
///
/// Given the full candidate set of a hidden join and a PUBLIC padded bound `B`,
/// produce `B` revealed [`OutputSlot`]s in oblivious-shuffled order such that the
/// parties learn neither which candidates matched (L2) nor the exact match count
/// beyond `B` (L1). See the module docs for the sound-vs-gated split and the
/// residual-leakage statement.
///
/// ## Protocol
///
/// 1. **Resolve each candidate's match bit to a secret-shared selector.** A
///    [`MatchBit::Public`] bit is lifted to a trivial sharing (sound, no secure
///    compare). A [`MatchBit::SecretShared`] sharing is used directly. (Producing
///    the latter from secret keys is the gated step — see
///    [`oblivious_set_output_hidden_keys`] / module docs.)
/// 2. **Build `B` slots.** Each of the `|candidates|` candidates becomes one slot;
///    if `|candidates| < B` we append pure-dummy slots (selector = shared 0) so the
///    revealed count is exactly `B` regardless of input.
/// 3. **Oblivious-select** each slot's selector against a real-tag sharing:
///    `tag_out = sel · real_tag` (one multiplication; a non-match has `sel = 0` so
///    the tag opens to the dummy sentinel `0`). The payload is encoded into
///    secret-shared `Fp` limbs that travel THROUGH the shuffle locked to the tag
///    (so the permutation is never opened — Copilot review #93), and **each limb
///    is selected against the DUMMY encoding by the same selector**:
///    `limb_out = PAD + sel · (limb − PAD)`. A slot with `sel = 0` therefore
///    carries the dummy limb block, so its opened payload is the dummy encoding
///    and decodes to the same all-unbound row a pure padding slot decodes to — the
///    tag and the payload agree on what the slot is.
/// 4. **Oblivious-shuffle** the (tag, payload-limbs) entry together so output
///    position is independent of input position (substrate `shuffle`).
/// 5. **Open exactly `B` tags** at degree `2t` (and the payload limbs, also at
///    degree `2t`, in the same shuffled order); emit [`OutputSlot::Row`] where the
///    tag is the real sentinel, [`OutputSlot::Dummy`] where it is `0`, and FAIL
///    CLOSED on any other value.
///
/// ## Fail-closed bound contract
///
/// `B` must be `>= candidates.len()` so every candidate gets a slot — we NEVER
/// silently truncate candidates (that could drop a true match and corrupt the
/// result). A `B < candidates.len()` is a [`MpcError::Protocol`] error. (The
/// caller sizes `B` from the public candidate count `|L|·|R|`, optionally padded;
/// the true MATCH count stays hidden.)
pub fn oblivious_set_output(
    backend: &ShamirBackend,
    candidates: &[Candidate],
    payload_arity: usize,
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    // Mint ONE fresh dealer for this run (production: a fresh OS-seeded CSPRNG,
    // sq-1vt) and draw every sharing and the shuffle through the seam on it.
    oblivious_set_output_through(
        &mut backend.dealer(),
        backend,
        candidates,
        payload_arity,
        bound,
    )
}

/// [`oblivious_set_output`] with every sharing and the shuffle drawn through
/// `source` — the selector sharings, the real-tag sharings, the pure-dummy
/// selectors, the payload-limb sharings and the oblivious shuffle itself, in that
/// order.
///
/// Refuses a source whose `(n, t)` differs from `backend`'s
/// ([`require_source_matches`]) **before any draw**, since every sharing is later
/// combined with `backend`-built products and opened at `backend`'s degrees.
/// Every drawn sharing is shape-checked where it is consumed
/// ([`checked_vss_own_input`]) and the shuffled column's shape is checked on
/// return ([`checked_oblivious_shuffle`]). Shape checks only — no VSS /
/// consistency guarantee, and no verification that the source actually permuted
/// anything. A source that cannot share a value it was handed (PRSS today) makes
/// this fail closed with that source's own error.
///
/// ## The tag column is degree `2t` — what that means for a recombining source
///
/// Step 3's `tag_out = sel · real_tag` is a [`shamir::mul_shares_raw`] product and
/// each `limb_out = PAD + sel · (limb − PAD)` is a product of the same selector
/// with a locally-shifted limb sharing, so EVERY element of a packed entry — the
/// tag and all its payload limbs — is a degree-`2t` sharing. (Adding the public
/// `PAD` constant back is local and does not change the degree.) That is fine for
/// a switch-network source, which only moves whole entries and never reads a share
/// ([`DistributedRandomness::oblivious_shuffle`] documents the distinction), and
/// it is why the dealer path opens the tags AND the limbs at `2 * t` in step 5.
/// Both opens need `n >= 2t + 1`, which this function already requires for the
/// select multiplication.
///
/// A source that shuffles by RECOMBINING — as a dealer-less one must — requires
/// degree-`<= t` elements and refuses this column (the tag and every limb)
/// ([`crate::prss::PrssRandomness`]). Routing the output stage through such a
/// source therefore needs the tags degree-reduced through the seam
/// ([`shamir::degree_reduce_through`]) before the shuffle, with step 5's open
/// moved to degree `t` to match. That path is **unreachable today** — the only
/// dealer-less source refuses `vss_own_input`, so this function already fails
/// closed on the selector sharing long before the shuffle — and it is a
/// prerequisite of the dealer-less VSS step (#6117), not work this stage can do
/// on its own. The dealer path is deliberately left un-reduced: adding a
/// reduction there would change what the simulation draws.
pub(crate) fn oblivious_set_output_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    payload_arity: usize,
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    let (decoded, cost) =
        oblivious_set_output_decoded_through(source, backend, candidates, payload_arity, bound)?;
    // A slot whose tag is the dummy sentinel carries the dummy limb block (the
    // per-limb select against the dummy encoding put it there — see
    // `shuffle_with_payloads`), so its decode is the all-unbound row. We CHECK that
    // rather than assume it: the tag and the payload must agree on what the slot
    // is, and a disagreement is a refusal, not a quietly-emitted `Dummy`.
    let dummy_row: PayloadRow = vec![None; payload_arity];
    let mut out: Vec<OutputSlot> = Vec::with_capacity(bound);
    for (is_real, row) in decoded {
        if is_real {
            out.push(OutputSlot::Row(row));
        } else if row == dummy_row {
            out.push(OutputSlot::Dummy);
        } else {
            return Err(MpcError::Tampered {
                detail: "oblivious_set_output: a slot tagged as a dummy did not carry the dummy \
                         payload encoding — the per-limb select against the dummy encoding did \
                         not hold; refusing rather than emitting the slot"
                    .to_string(),
                cheaters: Vec::new(),
            });
        }
    }
    Ok((out, cost))
}

/// One opened slot of the output stage before it is narrowed to an
/// [`OutputSlot`]: whether its tag was the real-row sentinel, and the payload row
/// its OPENED limbs decoded to. Every slot's payload is decoded, including the
/// dummy-tagged ones whose row [`OutputSlot::Dummy`] drops.
type DecodedSlot = (bool, PayloadRow);

/// [`oblivious_set_output_through`]'s whole protocol, stopping one step short: the
/// `B` opened slots as `(is_real, decoded_row)` rather than [`OutputSlot`]s.
///
/// This is where the stage's steps 1–5 live. Keeping the dummy slots' decoded rows
/// (instead of dropping them with the public `Dummy`) is what lets the tests see
/// what a non-matching candidate's payload actually opened to — the pad, not its
/// row — on the real path rather than on a re-implementation of it.
fn oblivious_set_output_decoded_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    payload_arity: usize,
    bound: usize,
) -> Result<(Vec<DecodedSlot>, ObliviousOutputCost), MpcError> {
    // Up front, before anything is drawn: a source whose (n, t) disagrees with the
    // backend would deal sharings that do not compose with the backend's products
    // and opens.
    require_source_matches(source, backend)?;
    // Fail closed: the bound must cover every candidate so no match is ever
    // silently dropped (see the contract above).
    if bound < candidates.len() {
        return Err(MpcError::Protocol(format!(
            "oblivious_set_output: padded bound B={bound} < candidate count {} — \
             would truncate candidates and could drop a true match",
            candidates.len()
        )));
    }
    // Validate payload arity up front (the recipient relies on a uniform schema).
    for (k, c) in candidates.iter().enumerate() {
        if c.payload.len() != payload_arity {
            return Err(MpcError::Protocol(format!(
                "oblivious_set_output: candidate {k} has {} payload terms, expected arity {payload_arity}",
                c.payload.len()
            )));
        }
    }

    let (n, t) = (backend.parties(), backend.threshold());
    // One multiplication (the select) needs honest-majority headroom n >= 2t+1.
    if n < 2 * t + 1 {
        return Err(MpcError::Protocol(format!(
            "oblivious_set_output needs n >= 2t+1 for the select multiplication (n={}, t={})",
            n, t
        )));
    }

    // The real-row sentinel tag. A matched slot's tag opens to REAL_TAG; a
    // non-match / dummy opens to Fp::zero(). REAL_TAG != 0 so the two are
    // distinguishable AFTER the shuffle (and only after — position is destroyed).
    let real_tag = Fp::one();

    // --- Step 1–3: build per-slot (tag-sharing, payload) selecting on the bit. ---
    // tag_share is a degree-`2t` sharing (product of the degree-`t` selector and a
    // degree-`t` real_tag sharing). The selector is kept alongside so the SAME bit
    // can select the payload limbs against the dummy encoding in step 3's payload
    // half (`shuffle_with_payloads`); the payload itself travels co-indexed in
    // cleartext only until it is encoded there.
    let mut tag_col: SecretColumn = Vec::with_capacity(bound);
    let mut payloads: Vec<Vec<Option<Term>>> = Vec::with_capacity(bound);
    let mut selectors: Vec<Vec<Share>> = Vec::with_capacity(bound);

    for c in candidates {
        let sel = selector_sharing(source, n, &c.matched)?;
        // tag_out = sel * real_tag  (degree 2t). sel=1 → REAL_TAG; sel=0 → 0.
        let real_tag_share = checked_vss_own_input(source, n, real_tag)?;
        let tag_out = shamir::mul_shares_raw(&sel, &real_tag_share)?;
        tag_col.push(tag_out);
        payloads.push(c.payload.clone());
        selectors.push(sel);
    }
    // Pure-dummy padding slots: selector = shared 0 → tag opens to 0 → Dummy, and
    // the same shared 0 selects the payload limbs to the dummy encoding.
    let dummy_payload: Vec<Option<Term>> = vec![None; payload_arity];
    for _ in candidates.len()..bound {
        let zero_sel = checked_vss_own_input(source, n, Fp::zero())?;
        let real_tag_share = checked_vss_own_input(source, n, real_tag)?;
        let tag_out = shamir::mul_shares_raw(&zero_sel, &real_tag_share)?;
        tag_col.push(tag_out);
        payloads.push(dummy_payload.clone());
        selectors.push(zero_sel);
    }

    // --- Step 4: oblivious-shuffle the tag column AND co-permute the payloads. ---
    // Payloads ride THROUGH the shuffle as secret shares (the permutation is never
    // opened) — see `shuffle_with_payloads` (Copilot review #93, thread 1), which
    // also applies the per-limb select against the dummy encoding.
    let (shuffled_tags, shuffled_limbs, shuffle_cost, payload_select_mults) =
        shuffle_with_payloads(source, n, t, tag_col, payloads, &selectors, payload_arity)?;

    // --- Step 5: open exactly `B` tags at degree 2t; classify each slot. ---
    // FAIL CLOSED on an unexpected tag (Copilot review #93, thread 3): the only
    // legitimate opened values are `REAL_TAG` (a match) and `0` (a non-match /
    // dummy). Any other value means a malformed selector (a `MatchBit` that was not
    // a sharing of exactly 0/1) or share corruption that slipped past the robust
    // open's detection budget — silently treating it as a dummy could drop a real
    // match without a word. The tag is already opened here, so refusing costs
    // nothing and leaks nothing extra; we surface it as a typed `Tampered` error.
    // The tag is classified BEFORE the payload limbs are decoded, so an out-of-set
    // tag is refused rather than surfacing as a decode error on limbs the malformed
    // selector scaled.
    let mut out: Vec<DecodedSlot> = Vec::with_capacity(bound);
    for (tag_share, limbs) in shuffled_tags.iter().zip(shuffled_limbs) {
        let tag = shamir::reconstruct_degree(tag_share, 2 * t)?;
        let is_real = if tag == real_tag {
            true
        } else if tag == Fp::zero() {
            false
        } else {
            return Err(MpcError::Tampered {
                detail: format!(
                    "oblivious_set_output: opened slot tag {} is neither REAL_TAG ({}) nor 0 — \
                     malformed selector or corrupted share; refusing rather than dropping a match",
                    tag.value(),
                    real_tag.value()
                ),
                cheaters: Vec::new(),
            });
        };
        let bytes = limbs_to_bytes(&limbs)?;
        out.push((is_real, decode_payload_row(&bytes, payload_arity)?));
    }

    let cost = ObliviousOutputCost {
        bound,
        select_mults: bound,
        payload_select_mults,
        opens: bound,
        shuffle: shuffle_cost,
    };
    Ok((out, cost))
}

/// Test-only instrumentation on the real output stage: the `B` opened slots as
/// `(is_real, decoded_row)`, keeping the payload row that every slot's OPENED
/// limbs decoded to — including the dummy-tagged slots, whose row the public
/// [`OutputSlot::Dummy`] drops.
///
/// This is the same [`oblivious_set_output_decoded_through`] the public path runs;
/// nothing is re-implemented and nothing is bypassed. It exists so a test can
/// compare the candidates' own payload rows against what each slot actually
/// opened to, which the public return type cannot express.
#[cfg(test)]
pub(crate) fn opened_slot_rows_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    payload_arity: usize,
    bound: usize,
) -> Result<Vec<DecodedSlot>, MpcError> {
    oblivious_set_output_decoded_through(source, backend, candidates, payload_arity, bound)
        .map(|(slots, _cost)| slots)
}

/// Resolve a [`MatchBit`] to a degree-`t` secret-shared selector `{0,1}`.
///
/// [`MatchBit::Public`] is lifted with a fresh sharing (sound, no secure compare —
/// the disclosed-key regime). [`MatchBit::SecretShared`] is used as-is; its
/// well-formedness (a sharing of exactly 0 or 1) is the producer's obligation.
fn selector_sharing(
    source: &mut dyn DistributedRandomness,
    n: usize,
    bit: &MatchBit,
) -> Result<Vec<Share>, MpcError> {
    match bit {
        MatchBit::Public(b) => {
            let v = if *b { Fp::one() } else { Fp::zero() };
            checked_vss_own_input(source, n, v)
        }
        MatchBit::SecretShared(s) => {
            if s.len() != n {
                return Err(MpcError::Protocol(format!(
                    "secret-shared match bit has {} shares, expected n={}",
                    s.len(),
                    n
                )));
            }
            Ok(s.clone())
        }
    }
}

/// Encode one payload row (`payload_arity` nullable terms) into a byte stream that
/// round-trips back to the SAME row via [`decode_payload_row`]. Layout, per column:
/// a 1-byte presence flag (`0` = unbound `None`, `1` = bound `Some`), and for a
/// bound column the term's canonical N-Triples serialization (`Term::to_string`)
/// as UTF-8, NUL-terminated. The N-Triples form escapes control characters
/// (including NUL → `\u0000`), so a raw `0x00` never appears inside a term and is
/// an unambiguous field terminator.
fn encode_payload_row(row: &[Option<Term>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for cell in row {
        match cell {
            None => bytes.push(0u8),
            Some(term) => {
                bytes.push(1u8);
                bytes.extend_from_slice(term.to_string().as_bytes());
                bytes.push(0u8); // NUL terminator (never occurs inside the term)
            }
        }
    }
    bytes
}

/// Inverse of [`encode_payload_row`]: parse a byte stream back into `arity`
/// nullable terms. Fails closed ([`MpcError::Protocol`]) on any malformed encoding
/// (truncated stream, non-UTF-8, unparseable term) — a corrupted shuffle entry must
/// not silently yield a wrong or partial row.
fn decode_payload_row(bytes: &[u8], arity: usize) -> Result<Vec<Option<Term>>, MpcError> {
    let mut out = Vec::with_capacity(arity);
    let mut pos = 0usize;
    for col in 0..arity {
        let flag = *bytes.get(pos).ok_or_else(|| {
            MpcError::Protocol(format!(
                "decode_payload_row: truncated stream at column {col} (missing presence flag)"
            ))
        })?;
        pos += 1;
        match flag {
            0 => out.push(None),
            1 => {
                let end = bytes[pos..]
                    .iter()
                    .position(|&b| b == 0u8)
                    .map(|rel| pos + rel)
                    .ok_or_else(|| {
                        MpcError::Protocol(format!(
                            "decode_payload_row: column {col} term not NUL-terminated (corrupt entry)"
                        ))
                    })?;
                let s = std::str::from_utf8(&bytes[pos..end]).map_err(|e| {
                    MpcError::Protocol(format!(
                        "decode_payload_row: column {col} term is not valid UTF-8: {e}"
                    ))
                })?;
                let term = Term::from_str(s).map_err(|e| {
                    MpcError::Protocol(format!(
                        "decode_payload_row: column {col} term {s:?} did not re-parse: {e}"
                    ))
                })?;
                out.push(Some(term));
                pos = end + 1; // skip the NUL
            }
            other => {
                return Err(MpcError::Protocol(format!(
                    "decode_payload_row: column {col} has invalid presence flag {other} (corrupt entry)"
                )))
            }
        }
    }
    Ok(out)
}

/// Pack a byte stream into `Fp` limbs with an explicit LENGTH PREFIX so the decode
/// is exact regardless of where the last chunk falls. Layout: `limb[0]` = the byte
/// length, then `⌈len / 7⌉` data limbs of [`BYTES_PER_LIMB`] bytes each (little-
/// endian within a limb), each `< 2^56`. The length prefix removes the trailing-
/// zero ambiguity a sentinel/implicit-length scheme has (a final partial limb is
/// zero-padded to 7 bytes; the prefix tells the decoder exactly how many to keep).
fn bytes_to_limbs(bytes: &[u8]) -> Vec<Fp> {
    let mut limbs = Vec::with_capacity(1 + bytes.len().div_ceil(BYTES_PER_LIMB));
    limbs.push(Fp::new(bytes.len() as u64));
    for chunk in bytes.chunks(BYTES_PER_LIMB) {
        let mut v = 0u64;
        for (k, &b) in chunk.iter().enumerate() {
            v |= (b as u64) << (8 * k);
        }
        limbs.push(Fp::new(v));
    }
    limbs
}

/// The number of limbs [`bytes_to_limbs`] produces for a `byte_len`-byte stream:
/// one length-prefix limb plus `⌈byte_len / 7⌉` data limbs.
fn limb_count_for(byte_len: usize) -> usize {
    1 + byte_len.div_ceil(BYTES_PER_LIMB)
}

/// Inverse of [`bytes_to_limbs`]: reassemble the original bytes from a limb block,
/// reading the byte length from `limbs[0]` and keeping exactly that many bytes.
/// Fails closed ([`MpcError::Protocol`]) if the length prefix is missing, if it
/// claims more bytes than the data limbs can supply, or if a data limb's value
/// exceeds the 7-byte range — any of which signals a corrupted shuffle entry rather
/// than a recoverable payload.
fn limbs_to_bytes(limbs: &[Fp]) -> Result<Vec<u8>, MpcError> {
    let len_limb = limbs.first().ok_or_else(|| {
        MpcError::Protocol("limbs_to_bytes: empty limb block (missing length prefix)".to_string())
    })?;
    let byte_len = len_limb.value() as usize;
    let available = (limbs.len() - 1) * BYTES_PER_LIMB;
    if byte_len > available {
        return Err(MpcError::Protocol(format!(
            "limbs_to_bytes: length prefix {byte_len} exceeds available {available} bytes \
             (corrupt shuffle entry)"
        )));
    }
    let mut bytes = Vec::with_capacity(byte_len);
    for limb in &limbs[1..] {
        let v = limb.value();
        if v >= (1u64 << (8 * BYTES_PER_LIMB)) {
            return Err(MpcError::Protocol(format!(
                "limbs_to_bytes: data limb {v} exceeds the 7-byte range (corrupt shuffle entry)"
            )));
        }
        for k in 0..BYTES_PER_LIMB {
            bytes.push(((v >> (8 * k)) & 0xff) as u8);
        }
    }
    bytes.truncate(byte_len);
    Ok(bytes)
}

/// Oblivious-shuffle the tag column AND its payloads by the SAME secret
/// permutation — carrying the payloads THROUGH the shuffle as secret shares, so the
/// permutation is NEVER opened (Copilot review #93, thread 1).
///
/// ## Why the old "open the indices" trick leaked (and this does not)
///
/// The previous implementation attached a secret-shared *position index* `i` to
/// each slot, shuffled, then OPENED the shuffled indices to learn the permutation
/// and reorder cleartext payloads. That reveals the permutation π to the protocol
/// transcript; composed with the step-5 per-slot tag opens (which are
/// position-aligned to the SAME shuffled order), an observer maps each opened tag
/// back to its originating candidate `π(k)` and recovers the per-candidate match
/// pattern — exactly the L2 match-graph / fan-out leak this module exists to close.
/// Cleartext payloads cannot be co-permuted by a secret permutation without either
/// revealing the permutation or carrying the payload INSIDE the secret domain;
/// this function does the latter.
///
/// ## Mechanism
///
/// Each payload row is serialized ([`encode_payload_row`]) and packed into a
/// FIXED number of `Fp` limbs (`limb_capacity`, the max over all rows so every
/// entry has identical width — required by the shuffle network and revealing only
/// the public max payload length), with `PAD_LIMB` padding. Each limb is
/// secret-shared, **selected against the dummy encoding** (below) and packed into
/// the shuffle entry AFTER the tag shares:
/// `[tag shares | limb₁ shares | … | limb_C shares]`. The substrate [`shuffle`]
/// permutes whole entries by a secret permutation; the limbs ride along locked to
/// their tag. At reveal, the caller opens the tag AND the payload limbs IN
/// SHUFFLED ORDER (never the permutation), then decodes the limbs back to the
/// row — so the only thing the transcript learns is the shuffled multiset of
/// (tag, payload), with no position→candidate linkage.
///
/// ## The per-limb select against the dummy encoding
///
/// A slot's tag is already selected by its secret bit, so a non-matching candidate
/// opens to the dummy tag. Its PAYLOAD limbs are selected by the same bit, with the
/// same [`shamir::mul_shares_raw`] product and the same degree handling, against the
/// DUMMY limb block — the limbs of `encode_payload_row` applied to the all-unbound
/// row of this arity, padded to `limb_capacity` with `PAD_LIMB`:
///
/// ```text
/// limb_out = PAD + sel · (limb − PAD)
/// ```
///
/// where `PAD` is that limb's dummy value (a PUBLIC constant — the dummy encoding
/// depends only on the public arity, so subtracting and re-adding it is local and
/// draws nothing). `sel = 1` reproduces `limb` exactly, so real rows' payloads are
/// unchanged; `sel = 0` yields the dummy limb block, so the opened payload of a
/// non-matching candidate is the pad, not its row, and decodes to exactly what a
/// pure padding slot decodes to. Dummy slots are therefore identical in both their
/// tag and their payload whether they arose from a non-match or from padding.
///
/// Both the tag and every limb are degree-`2t` products, so the limb opens move to
/// `reconstruct_degree(.., 2 * t)` — the same `n >= 2t + 1` the tag select already
/// requires.
fn shuffle_with_payloads(
    source: &mut dyn DistributedRandomness,
    nparties: usize,
    t: usize,
    tag_col: SecretColumn,
    payloads: Vec<PayloadRow>,
    selectors: &[Vec<Share>],
    payload_arity: usize,
) -> Result<ShuffledColumn, MpcError> {
    let n = tag_col.len();
    debug_assert_eq!(payloads.len(), n);
    debug_assert_eq!(selectors.len(), n);

    // Serialize every payload row and size a UNIFORM limb capacity = the max limb
    // count (length prefix + data limbs) over all rows. Uniform width is required
    // by the shuffle network and leaks only the public maximum payload length (all
    // candidate payloads are cleartext to the protocol builder anyway).
    let encoded: Vec<Vec<u8>> = payloads.iter().map(|p| encode_payload_row(p)).collect();
    // The dummy limb block every slot is selected against: the encoding of the
    // all-unbound row of this arity. It depends only on the PUBLIC arity.
    let dummy_limbs = bytes_to_limbs(&encode_payload_row(&vec![None; payload_arity]));
    let mut limb_capacity = encoded
        .iter()
        .map(|b| limb_count_for(b.len()))
        .max()
        .unwrap_or(0);
    if !encoded.is_empty() {
        // Every real encoding is at least `payload_arity` bytes (one presence flag
        // per column), so this is already the max; take it explicitly so the dummy
        // block always fits the uniform width.
        limb_capacity = limb_capacity.max(dummy_limbs.len());
    }
    // The dummy block padded out to the uniform width, so limb `i` has a public
    // pad value for every `i < limb_capacity`.
    let mut dummy_block = dummy_limbs;
    dummy_block.resize(limb_capacity, Fp::new(PAD_LIMB));

    // Pack each entry: [tag shares | limb shares...]. Each limb is shared at
    // degree `t`, then selected against its dummy value — a single multiplication
    // by the slot's selector — so every packed element is degree `2t`. Short rows
    // are padded to `limb_capacity` with shared PAD_LIMB before the select, so all
    // entries are identical width (the explicit length prefix makes the padding
    // content of a REAL row irrelevant to its decode).
    let mut packed: SecretColumn = Vec::with_capacity(n);
    let mut payload_select_mults = 0usize;
    for ((tag, enc), sel) in tag_col.into_iter().zip(&encoded).zip(selectors) {
        let mut entry = tag; // tag shares first
        let limbs = bytes_to_limbs(enc);
        // `dummy_block` is exactly `limb_capacity` long, so iterating it walks the
        // uniform width and supplies each limb's public pad value.
        for (i, &pad) in dummy_block.iter().enumerate() {
            // Draw order is unchanged: the row's own limbs first, then PAD_LIMB up
            // to the uniform width.
            let value = limbs.get(i).copied().unwrap_or(Fp::new(PAD_LIMB));
            let limb_share = checked_vss_own_input(source, nparties, value)?;
            // limb_out = pad + sel·(limb − pad): local shift, one multiplication,
            // local shift back. Degree `2t`, like the tag.
            let shifted = shamir::add_constant(&limb_share, pad.neg());
            let selected = shamir::mul_shares_raw(sel, &shifted)?;
            payload_select_mults += 1;
            entry.extend(shamir::add_constant(&selected, pad));
        }
        packed.push(entry);
    }

    // One oblivious shuffle over the packed entries (the substrate primitive). It
    // permutes whole entries by a SECRET permutation that is never revealed.
    let (shuffled, base_cost) = checked_oblivious_shuffle(source, packed)?;

    // The real secret-control shuffle pays a conditional swap per FIELD ELEMENT per
    // switch; the packed entry is `entry_width` field elements wide (tag + limbs),
    // so the modelled switch count must scale by the width (Copilot review #93,
    // thread 2). `ShuffleCost::model` counts one element per item, so widen it here.
    let entry_width = 1 + limb_capacity; // tag + payload limbs (per secret value)
    let cost = ShuffleCost {
        items: base_cost.items,
        switches: base_cost.switches.saturating_mul(entry_width),
        depth_rounds: base_cost.depth_rounds,
    };

    // Split each shuffled entry into (tag shares, limb shares...) and OPEN the
    // limbs — in shuffled order, never opening the permutation. Decoding is left to
    // the caller, which classifies the slot by its tag first (so a slot whose tag
    // is out of `{0, REAL_TAG}` is refused before any byte is parsed). Every entry
    // has the same width = nparties·(1 + limb_capacity).
    let expect_width = nparties * (1 + limb_capacity);
    let mut shuffled_tags: SecretColumn = Vec::with_capacity(n);
    let mut shuffled_limbs: Vec<Vec<Fp>> = Vec::with_capacity(n);
    for entry in shuffled {
        if entry.len() != expect_width {
            return Err(MpcError::Protocol(format!(
                "shuffle_with_payloads: packed entry width {} != n_parties*(1+limbs) {expect_width}",
                entry.len(),
            )));
        }
        let (tag_part, limb_part) = entry.split_at(nparties);
        shuffled_tags.push(tag_part.to_vec());

        // Open each limb sharing IN SHUFFLED ORDER. The limbs are selected
        // products, so they open at degree `2t` like the tag.
        let mut limbs: Vec<Fp> = Vec::with_capacity(limb_capacity);
        for limb_shares in limb_part.chunks(nparties) {
            limbs.push(shamir::reconstruct_degree(limb_shares, 2 * t)?);
        }
        shuffled_limbs.push(limbs);
    }

    Ok((shuffled_tags, shuffled_limbs, cost, payload_select_mults))
}

/// Run a hidden-value join with the oblivious set-returning output path, from
/// candidate pairs whose match was decided by a **disclosed / public** equi-join
/// (convention #4) — the SOUND-TODAY end-to-end entry point.
///
/// This wraps [`oblivious_set_output`] into the [`PartialResult`] shape the rest of
/// the crate uses, so a federation can run a private-key SET join whose OUTPUT is
/// oblivious (L1 bounded to `B`, L2 destroyed) without touching
/// [`crate::join::HiddenValueJoin`]'s leaky per-pair open path. The output schema
/// is `out_vars`; the disclosed result is the [`OutputSlot::Row`]s with dummies
/// filtered, attributed to the synthetic `federation` holder, in canonical order.
///
/// HONESTY: this is the sound path because the match bits are public (decided
/// outside the crypto core). The HIDDEN-key variant — deriving the match bits from
/// secret keys WITHOUT opening them — is [`oblivious_set_output_hidden_keys`],
/// which is gated on secure-compare (sq-rrz4/sq-dvuc).
pub fn oblivious_join_output(
    backend: &ShamirBackend,
    candidates: &[Candidate],
    out_vars: Vec<Variable>,
    bound: usize,
) -> Result<(PartialResult, ObliviousOutputCost), MpcError> {
    // One fresh dealer for this run; every draw and the shuffle go through the seam.
    oblivious_join_output_through(&mut backend.dealer(), backend, candidates, out_vars, bound)
}

/// [`oblivious_join_output`] with the whole output stage drawn through `source` —
/// see [`oblivious_set_output_through`] for the draw order and the refusals.
pub(crate) fn oblivious_join_output_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    out_vars: Vec<Variable>,
    bound: usize,
) -> Result<(PartialResult, ObliviousOutputCost), MpcError> {
    let payload_arity = out_vars.len();
    let (slots, cost) =
        oblivious_set_output_through(source, backend, candidates, payload_arity, bound)?;
    Ok((partial_result_from_slots(slots, out_vars), cost))
}

/// Narrow the `B` revealed slots to the disclosed [`PartialResult`]: filter the
/// dummies and put the surviving rows in canonical order.
///
/// Filtering dummies is what the authorized recipient does — it sees the real
/// result multiset and, by construction, learns its true size (see the
/// residual-leakage note). The canonical (order-independent) row order makes the
/// disclosed multiset independent of the shuffle's permutation; the recipient
/// recomputes any ORDER BY over it (convention #4).
///
/// `pub(crate)` so every operator that already has the slots
/// ([`crate::hidden_path`]) reaches its `PartialResult` through this one function
/// rather than a second route into the output stage.
pub(crate) fn partial_result_from_slots(
    slots: Vec<OutputSlot>,
    out_vars: Vec<Variable>,
) -> PartialResult {
    let mut rows: Vec<Vec<Option<Term>>> = slots
        .into_iter()
        .filter_map(|s| match s {
            OutputSlot::Row(r) => Some(r),
            OutputSlot::Dummy => None,
        })
        .collect();
    canonicalize_rows(&mut rows);
    PartialResult {
        holder: HolderId::new("federation"),
        vars: out_vars,
        rows,
    }
}

/// Sort rows into a canonical order (by their `{:?}` term tuple) so the disclosed
/// result multiset is independent of the shuffle / row ordering. Local to this
/// module so it does not couple to `join.rs` (sibling-PR file-lane discipline).
fn canonicalize_rows(rows: &mut [Vec<Option<Term>>]) {
    rows.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
}

/// The HIDDEN-key oblivious set output: derive each candidate's match bit from
/// genuinely-SECRET keys WITHOUT opening it, then run the oblivious output
/// transform. This is the configuration that closes L2 at its source for a
/// truly-private join key (the disclosed-key path closes L2 too, but only because
/// the key was public to begin with).
///
/// ## sq-xhaw — UNGATED: the secure equality-to-shared-bit landed
///
/// This path was previously an honest [`MpcError::NotYetImplemented`] gated on a
/// secure equality-to-shared-bit (the prior `secure_equal` only *opened* the bit —
/// the L2 leak). That gate — the BGW/DN degree-reduction round (bead **sq-dvuc**)
/// and the secure-comparison machinery on it (bead **sq-rrz4**) — has since landed,
/// and [`crate::compare::secure_equal_to_bit`] now produces `[1{a == b}]` as a fresh
/// degree-`t` **secret-shared** bit that is NEVER opened. So this is realised for
/// real: each candidate pair's match bit is computed shared and fed as a
/// [`MatchBit::SecretShared`] selector into the (already sound, sq-jnkm) oblivious
/// output transform.
///
/// All-pairs: every `left_keys[i]` is compared against every `right_keys[j]`, so
/// the candidate count is `|left|·|right|`; a single payload column carries the
/// matched right key's index as a literal so the result is checkable (the payload
/// schema is the caller's concern — this raw-key entry point exists mainly for the
/// transform's hidden-key test surface; the row-carrying production path is
/// [`crate::join::HiddenValueJoin::fully_oblivious_batched_join`]). `bound` must
/// cover `|left|·|right|` (fail-closed, never truncates a candidate).
///
/// HONESTY (privacy-claims gate is live; cite sq-qhy4). Semi-honest, honest-majority
/// only — the per-pair match bit is never opened (the L2-at-decision leak is closed)
/// but malicious security is unchanged from the Shamir/compare layer, and external
/// soundness sign-off is still pending. No security guarantee beyond the documented
/// semi-honest model is claimed.
pub fn oblivious_set_output_hidden_keys(
    backend: &ShamirBackend,
    left_keys: &[Fp],
    right_keys: &[Fp],
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    // ONE fresh dealer for this run (production: a fresh OS-seeded CSPRNG, sq-1vt)
    // covering BOTH the per-pair equality bits and the output stage.
    oblivious_set_output_hidden_keys_through(
        &mut backend.dealer(),
        backend,
        left_keys,
        right_keys,
        bound,
    )
}

/// [`oblivious_set_output_hidden_keys`] with the per-pair equality bits AND the
/// whole output stage (selector / real-tag / pure-dummy / payload-limb sharings and
/// the shuffle) drawn through ONE `source`, in that order.
///
/// Refuses a source whose `(n, t)` differs from `backend`'s
/// ([`require_source_matches`]) **before any draw** — the equality chain draws
/// first, so the check cannot be left to [`oblivious_set_output_through`]. Every
/// drawn sharing is shape-checked where it is consumed and the shuffled column's
/// shape on return. Shape checks only — no VSS / consistency guarantee. A source
/// that cannot share a value it was handed (PRSS today) makes this fail closed with
/// that source's own error — inside the first `secure_equal_to_bit_through` when
/// there is a candidate pair, and otherwise (empty key lists) inside the output
/// stage, which shares the real-row tag and the payload limbs for each of its `B`
/// pure-dummy slots. With empty key lists AND `bound = 0` nothing is drawn at all.
pub(crate) fn oblivious_set_output_hidden_keys_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    left_keys: &[Fp],
    right_keys: &[Fp],
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    // Up front, before anything is drawn (the equality chain below draws before
    // the output stage's own check would run).
    require_source_matches(source, backend)?;
    // All-pairs candidates: each (i, j) becomes one slot whose match bit is the
    // SECRET-SHARED equality of the two private keys — never opened.
    let mut candidates: Vec<Candidate> = Vec::with_capacity(left_keys.len() * right_keys.len());
    for (i, &lk) in left_keys.iter().enumerate() {
        for (j, &rk) in right_keys.iter().enumerate() {
            let match_bit = crate::compare::secure_equal_to_bit_through(source, lk, rk)?;
            // A single disclosed payload column tagging the matched right index, so
            // the test surface can identify which pair survived (the key stays
            // hidden; this is a disclosed payload column, convention #4).
            let payload = vec![Some(Term::Literal(oxrdf::Literal::new_simple_literal(
                format!("{i}-{j}"),
            )))];
            candidates.push(Candidate {
                payload,
                matched: MatchBit::SecretShared(match_bit),
            });
        }
    }
    oblivious_set_output_through(source, backend, &candidates, 1, bound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shamir::ShamirBackend;

    fn var(n: &str) -> Variable {
        Variable::new_unchecked(n)
    }

    fn lit(s: &str) -> Option<Term> {
        Some(Term::Literal(oxrdf::Literal::new_simple_literal(s)))
    }

    /// Build candidate set from (matched_bool, payload) tuples in the disclosed-key
    /// (public match bit) regime.
    fn public_candidates(rows: &[(bool, Vec<Option<Term>>)]) -> Vec<Candidate> {
        rows.iter()
            .map(|(m, p)| Candidate {
                payload: p.clone(),
                matched: MatchBit::Public(*m),
            })
            .collect()
    }

    /// The real (non-dummy) rows of an output, as an order-independent multiset.
    fn real_multiset(slots: &[OutputSlot]) -> Vec<String> {
        let mut m: Vec<String> = slots
            .iter()
            .filter_map(|s| match s {
                OutputSlot::Row(r) => Some(format!("{r:?}")),
                OutputSlot::Dummy => None,
            })
            .collect();
        m.sort();
        m
    }

    // ---- Correctness: the real result multiset is right under reconstruction ----

    /// THE correctness test: the matched rows survive, the non-matches become
    /// dummies, and the real multiset equals the plaintext expectation — regardless
    /// of the (oblivious) order they come out in.
    #[test]
    fn matched_rows_survive_nonmatches_become_dummies() {
        let backend = ShamirBackend::new_seeded(3, 0xBEEF).unwrap();
        let cands = public_candidates(&[
            (true, vec![lit("Alice"), lit("Leeds")]),
            (false, vec![lit("Bob"), lit("York")]),
            (true, vec![lit("Carol"), lit("Hull")]),
            (false, vec![lit("Dan"), lit("Ely")]),
        ]);
        let (slots, cost) = oblivious_set_output(&backend, &cands, 2, 6).unwrap();
        assert_eq!(slots.len(), 6, "exactly B=6 slots revealed");
        // Two real matches; the rest dummies (2 non-matches + 2 pure padding).
        let reals = real_multiset(&slots);
        assert_eq!(reals.len(), 2);
        let mut expected = vec![
            format!("{:?}", vec![lit("Alice"), lit("Leeds")]),
            format!("{:?}", vec![lit("Carol"), lit("Hull")]),
        ];
        expected.sort();
        assert_eq!(
            reals, expected,
            "real rows must be exactly the matched payloads"
        );
        assert_eq!(cost.bound, 6);
        assert_eq!(cost.select_mults, 6);
        assert_eq!(cost.opens, 6);
        assert_eq!(cost.shuffle.items, 6);
    }

    /// Padding to a bound LARGER than the candidate count works (pure-dummy slots).
    #[test]
    fn pads_with_pure_dummies_up_to_bound() {
        let backend = ShamirBackend::new_seeded(5, 7).unwrap();
        let cands = public_candidates(&[(true, vec![lit("X")])]);
        let (slots, cost) = oblivious_set_output(&backend, &cands, 1, 10).unwrap();
        assert_eq!(slots.len(), 10);
        assert_eq!(real_multiset(&slots).len(), 1);
        assert_eq!(
            slots
                .iter()
                .filter(|s| matches!(s, OutputSlot::Dummy))
                .count(),
            9
        );
        assert_eq!(cost.bound, 10);
    }

    /// All-match and no-match boundaries.
    #[test]
    fn all_match_and_no_match_boundaries() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let all = public_candidates(&[(true, vec![lit("a")]), (true, vec![lit("b")])]);
        let (slots, _) = oblivious_set_output(&backend, &all, 1, 2).unwrap();
        assert_eq!(
            real_multiset(&slots).len(),
            2,
            "B == match count: all real, no dummies"
        );
        assert!(slots.iter().all(|s| matches!(s, OutputSlot::Row(_))));

        let none = public_candidates(&[(false, vec![lit("a")]), (false, vec![lit("b")])]);
        let (slots, _) = oblivious_set_output(&backend, &none, 1, 4).unwrap();
        assert_eq!(real_multiset(&slots).len(), 0, "no matches: all dummies");
        assert!(slots.iter().all(|s| matches!(s, OutputSlot::Dummy)));
    }

    // ---- L1 obliviousness: revealed count is the BOUND, not the true count ------

    /// L1: the number of REVEALED slots is exactly `B` for ANY true match count —
    /// the parties see `B`, not the true cardinality. Two candidate sets with
    /// DIFFERENT true match counts must produce the IDENTICAL revealed-slot count.
    #[test]
    fn revealed_count_is_bound_not_true_cardinality() {
        let backend = ShamirBackend::new_seeded(3, 42).unwrap();
        let one_match = public_candidates(&[
            (true, vec![lit("a")]),
            (false, vec![lit("b")]),
            (false, vec![lit("c")]),
        ]);
        let three_match = public_candidates(&[
            (true, vec![lit("a")]),
            (true, vec![lit("b")]),
            (true, vec![lit("c")]),
        ]);
        let (s1, _) = oblivious_set_output(&backend, &one_match, 1, 5).unwrap();
        let (s3, _) = oblivious_set_output(&backend, &three_match, 1, 5).unwrap();
        // Same revealed slot count (= B) regardless of true match count → L1
        // bounded to B, not exact, from the slot count the parties observe.
        assert_eq!(s1.len(), s3.len());
        assert_eq!(s1.len(), 5);
        // The transcript-visible thing (slot count) is identical; only the
        // recipient who filters dummies learns the true counts differ (1 vs 3).
        assert_eq!(real_multiset(&s1).len(), 1);
        assert_eq!(real_multiset(&s3).len(), 3);
    }

    // ---- L2 obliviousness: output order hides which candidate produced each row -

    /// L2 (access-pattern half): the OUTPUT ORDER is not the input order — the
    /// shuffle destroyed position→candidate linkage. Over several runs the slot
    /// CLASSIFICATION (which positions are Row vs Dummy) must VARY — proving the
    /// match positions are not fixed by input position.
    #[test]
    fn output_order_is_shuffled_hiding_match_positions() {
        // Distinct matched payloads at fixed input positions; if the output never
        // reordered, the matched rows would land at the same output slots every run.
        let cands = public_candidates(&[
            (true, vec![lit("p0")]),
            (false, vec![lit("p1")]),
            (true, vec![lit("p2")]),
            (false, vec![lit("p3")]),
            (true, vec![lit("p4")]),
        ]);
        let mut classification_patterns = std::collections::HashSet::new();
        for seed in 0..32u64 {
            let backend = ShamirBackend::new_seeded(3, 1000 + seed).unwrap();
            let (slots, _) = oblivious_set_output(&backend, &cands, 1, 5).unwrap();
            // Record the Row/Dummy pattern (positions of the 3 reals among 5 slots).
            let pat: Vec<bool> = slots
                .iter()
                .map(|s| matches!(s, OutputSlot::Row(_)))
                .collect();
            classification_patterns.insert(pat);
            // Always exactly 3 reals regardless of where they land.
            assert_eq!(real_multiset(&slots).len(), 3);
        }
        // If the output were NOT shuffled, the match positions would be fixed
        // (reals always at input slots 0,2,4) → exactly ONE pattern. The shuffle
        // must produce several distinct patterns.
        assert!(
            classification_patterns.len() > 1,
            "match positions never moved across 32 runs — output is not oblivious"
        );
    }

    /// The real multiset is INVARIANT to the shuffle (correctness under
    /// reconstruction holds no matter which permutation the shuffle drew).
    #[test]
    fn real_multiset_is_shuffle_invariant() {
        let cands = public_candidates(&[
            (true, vec![lit("A"), lit("1")]),
            (false, vec![lit("B"), lit("2")]),
            (true, vec![lit("C"), lit("3")]),
        ]);
        let mut expected = vec![
            format!("{:?}", vec![lit("A"), lit("1")]),
            format!("{:?}", vec![lit("C"), lit("3")]),
        ];
        expected.sort();
        for seed in 0..16u64 {
            let backend = ShamirBackend::new_seeded(5, seed).unwrap();
            let (slots, _) = oblivious_set_output(&backend, &cands, 2, 4).unwrap();
            assert_eq!(
                real_multiset(&slots),
                expected,
                "seed {seed}: real multiset changed"
            );
        }
    }

    // ---- Secret-shared match bit path (sound transform given a shared bit) -------

    /// A SECRET-SHARED match bit (sharing of 0/1) drives the select correctly — the
    /// transform is sound given the bit; only PRODUCING the bit from secret keys is
    /// gated. Here we hand it well-formed 0/1 sharings directly.
    #[test]
    fn secret_shared_match_bit_select_is_correct() {
        let backend = ShamirBackend::new_seeded(3, 99).unwrap();
        let mut dealer = backend.dealer();
        let one = dealer.share(Fp::one());
        let zero = dealer.share(Fp::zero());
        let cands = vec![
            Candidate {
                payload: vec![lit("match")],
                matched: MatchBit::SecretShared(one),
            },
            Candidate {
                payload: vec![lit("nomatch")],
                matched: MatchBit::SecretShared(zero),
            },
        ];
        let (slots, _) = oblivious_set_output(&backend, &cands, 1, 3).unwrap();
        assert_eq!(
            real_multiset(&slots),
            vec![format!("{:?}", vec![lit("match")])]
        );
    }

    // ---- Fail-closed contracts --------------------------------------------------

    /// Bound smaller than the candidate count FAILS CLOSED (never truncates).
    #[test]
    fn bound_below_candidate_count_is_protocol_error() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let cands = public_candidates(&[(true, vec![lit("a")]), (true, vec![lit("b")])]);
        let err = oblivious_set_output(&backend, &cands, 1, 1).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("truncate")));
    }

    /// Wrong payload arity is rejected (uniform schema for the recipient).
    #[test]
    fn payload_arity_mismatch_is_protocol_error() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let cands = public_candidates(&[(true, vec![lit("a"), lit("b")])]);
        let err = oblivious_set_output(&backend, &cands, 1, 4).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("arity")));
    }

    /// A malformed secret-shared bit (wrong share count) is rejected, not silently
    /// mis-selected.
    #[test]
    fn malformed_secret_bit_is_rejected() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let cands = vec![Candidate {
            payload: vec![lit("a")],
            matched: MatchBit::SecretShared(vec![]), // 0 shares != n
        }];
        let err = oblivious_set_output(&backend, &cands, 1, 2).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("match bit")));
    }

    // ---- The full PartialResult wrapper + the gated hidden-key path -------------

    /// The end-to-end disclosed-key wrapper yields the right PartialResult (real
    /// rows only, federation-attributed, canonical order).
    #[test]
    fn oblivious_join_output_yields_partial_result() {
        let backend = ShamirBackend::new_seeded(3, 5).unwrap();
        let cands = public_candidates(&[
            (true, vec![lit("Bob"), lit("Leeds")]),
            (false, vec![lit("x"), lit("y")]),
            (true, vec![lit("Carol"), lit("York")]),
        ]);
        let out_vars = vec![var("name"), var("city")];
        let (result, cost) = oblivious_join_output(&backend, &cands, out_vars.clone(), 5).unwrap();
        assert_eq!(result.holder, HolderId::new("federation"));
        assert_eq!(result.vars, out_vars);
        assert_eq!(
            result.rows.len(),
            2,
            "only the 2 matched rows, dummies filtered"
        );
        assert_eq!(cost.bound, 5);
        // Canonical (order-independent) check of the disclosed multiset.
        let got: Vec<String> = result.rows.iter().map(|r| format!("{r:?}")).collect();
        let mut exp = vec![
            format!("{:?}", vec![lit("Bob"), lit("Leeds")]),
            format!("{:?}", vec![lit("Carol"), lit("York")]),
        ];
        exp.sort();
        assert_eq!(got, exp, "disclosed multiset wrong (canonical order)");
    }

    /// sq-xhaw — the HIDDEN-key path is now REALISED (the secure
    /// equality-to-shared-bit gate landed): the per-pair match bit is secret-shared
    /// and never opened, and the revealed result matches a plaintext all-pairs
    /// equi-join. Keys 1 and 3 are shared between the two sides → 2 matches; the
    /// revealed slot count is the bound B, not the true match count.
    #[test]
    fn hidden_key_path_matches_plaintext_all_pairs() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let left = [Fp::new(1), Fp::new(2), Fp::new(3)];
        let right = [Fp::new(3), Fp::new(5), Fp::new(1)];
        // Plaintext all-pairs equality: (i,j) matches iff left[i]==right[j].
        // left[0]=1 == right[2]=1 → "0-2"; left[2]=3 == right[0]=3 → "2-0".
        let mut expected: Vec<String> = vec!["0-2".to_string(), "2-0".to_string()]
            .into_iter()
            .map(|s| {
                format!(
                    "{:?}",
                    vec![Some(Term::Literal(oxrdf::Literal::new_simple_literal(s)))]
                )
            })
            .collect();
        expected.sort();
        // 9 candidate pairs; bound B=9.
        let (slots, cost) = oblivious_set_output_hidden_keys(&backend, &left, &right, 9).unwrap();
        assert_eq!(
            cost.bound, 9,
            "revealed slot count is B, not the 2 true matches"
        );
        assert_eq!(
            real_multiset(&slots),
            expected,
            "hidden-key result != plaintext all-pairs join"
        );
    }

    /// Empty candidate set with a bound → all dummies, no panic.
    #[test]
    fn empty_candidates_all_dummies() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let (slots, cost) = oblivious_set_output(&backend, &[], 2, 3).unwrap();
        assert_eq!(slots.len(), 3);
        assert!(slots.iter().all(|s| matches!(s, OutputSlot::Dummy)));
        assert_eq!(cost.bound, 3);
    }

    /// Bound == 0 with no candidates is the trivial empty output (shuffle of 0).
    #[test]
    fn zero_bound_no_candidates_is_empty() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let (slots, cost) = oblivious_set_output(&backend, &[], 0, 0).unwrap();
        assert!(slots.is_empty());
        assert_eq!(cost.bound, 0);
        assert_eq!(cost.select_mults, 0);
    }

    // ---- Payload-through-shuffle round-trip (Copilot #93, thread 1) --------------

    fn iri(s: &str) -> Option<Term> {
        Some(Term::NamedNode(oxrdf::NamedNode::new(s).unwrap()))
    }

    fn typed(value: &str, dt: &str) -> Option<Term> {
        Some(Term::Literal(oxrdf::Literal::new_typed_literal(
            value,
            oxrdf::NamedNode::new(dt).unwrap(),
        )))
    }

    /// The encode→limbs→bytes→decode pipeline round-trips a mix of IRIs, simple
    /// and typed literals, special characters, and unbound `None` cells exactly.
    #[test]
    fn payload_row_encode_decode_round_trips() {
        let rows: Vec<Vec<Option<Term>>> = vec![
            vec![iri("http://ex/x"), lit("Alice")],
            vec![
                None,
                typed("42", "http://www.w3.org/2001/XMLSchema#integer"),
            ],
            vec![lit("comma, and \"quote\" and \\backslash"), None],
            vec![lit(""), lit("")], // empty literals
        ];
        for row in &rows {
            let bytes = encode_payload_row(row);
            let limbs = bytes_to_limbs(&bytes);
            let back = limbs_to_bytes(&limbs).unwrap();
            assert_eq!(back, bytes, "byte<->limb round-trip changed bytes");
            let decoded = decode_payload_row(&back, row.len()).unwrap();
            assert_eq!(&decoded, row, "term round-trip changed the row");
        }
    }

    /// End-to-end: IRIs, typed literals, and unbound cells survive the FULL
    /// oblivious output path (carried through the shuffle as secret limbs), and the
    /// real multiset is shuffle-invariant across seeds. This is the regression test
    /// for the L2-leak fix — payloads no longer ride as cleartext reordered by an
    /// opened permutation.
    #[test]
    fn rich_payloads_survive_shuffle_without_opening_permutation() {
        let cands = public_candidates(&[
            (
                true,
                vec![
                    iri("http://ex/alice"),
                    typed("30", "http://www.w3.org/2001/XMLSchema#integer"),
                ],
            ),
            (
                false,
                vec![
                    iri("http://ex/bob"),
                    typed("40", "http://www.w3.org/2001/XMLSchema#integer"),
                ],
            ),
            (true, vec![lit("Carol \"C\""), None]),
        ]);
        let mut expected = vec![
            format!(
                "{:?}",
                vec![
                    iri("http://ex/alice"),
                    typed("30", "http://www.w3.org/2001/XMLSchema#integer")
                ]
            ),
            format!("{:?}", vec![lit("Carol \"C\""), None]),
        ];
        expected.sort();
        for seed in 0..16u64 {
            let backend = ShamirBackend::new_seeded(5, seed).unwrap();
            let (slots, _) = oblivious_set_output(&backend, &cands, 2, 5).unwrap();
            assert_eq!(
                real_multiset(&slots),
                expected,
                "seed {seed}: rich payload changed"
            );
        }
    }

    /// The modelled shuffle switch count scales with the packed-entry width
    /// (tag + payload limbs), not one element per item (Copilot #93, thread 2).
    /// A wider payload (more limbs) yields a strictly larger switch count than a
    /// narrow one at the same slot count.
    #[test]
    fn shuffle_cost_scales_with_payload_width() {
        let backend = ShamirBackend::new_seeded(3, 5).unwrap();
        // Narrow: one short literal. Wide: one long literal (many limbs).
        let narrow = public_candidates(&[(true, vec![lit("x")])]);
        let wide = public_candidates(&[(true, vec![lit(&"y".repeat(200))])]);
        let (_, c_narrow) = oblivious_set_output(&backend, &narrow, 1, 4).unwrap();
        let (_, c_wide) = oblivious_set_output(&backend, &wide, 1, 4).unwrap();
        assert_eq!(
            c_narrow.shuffle.items, c_wide.shuffle.items,
            "same slot count"
        );
        assert!(
            c_wide.shuffle.switches > c_narrow.shuffle.switches,
            "wider payload must cost more switches (width-scaled): wide={}, narrow={}",
            c_wide.shuffle.switches,
            c_narrow.shuffle.switches
        );
    }

    // ---- What a dummy slot's payload opens to -----------------------------------

    /// The encoding every dummy slot's opened limbs must decode to: the all-unbound
    /// row of this arity, which is exactly what a PURE PADDING slot carries.
    fn dummy_row(arity: usize) -> Vec<Option<Term>> {
        vec![None; arity]
    }

    /// THE guard for the output stage's payload select: across ALL `B` slots, the
    /// set of opened payload rows is `{the matched candidates' rows} ∪ {the dummy
    /// encoding}` — a rejected candidate's row is never among them. Asserted by
    /// comparing the rejected candidates' OWN payload rows against every slot's
    /// opened row, on the real path via `opened_slot_rows_through`.
    #[test]
    fn rejected_candidates_payloads_open_to_the_dummy_encoding() {
        let rows: Vec<(bool, Vec<Option<Term>>)> = vec![
            (true, vec![lit("Alice"), lit("Leeds")]),
            (false, vec![lit("Bob"), lit("York")]),
            (true, vec![lit("Carol"), lit("Hull")]),
            (false, vec![lit("Dan"), lit("Ely")]),
            (false, vec![iri("http://ex/eve"), None]),
        ];
        let cands = public_candidates(&rows);
        let kept: Vec<String> = rows
            .iter()
            .filter(|(m, _)| *m)
            .map(|(_, p)| format!("{p:?}"))
            .collect();
        let rejected: Vec<String> = rows
            .iter()
            .filter(|(m, _)| !*m)
            .map(|(_, p)| format!("{p:?}"))
            .collect();
        let pad = format!("{:?}", dummy_row(2));
        for seed in 0..16u64 {
            let b = ShamirBackend::new_seeded(5, 0xD0_0000 + seed).unwrap();
            let slots = opened_slot_rows_through(&mut b.dealer(), &b, &cands, 2, 8).unwrap();
            assert_eq!(slots.len(), 8, "seed {seed}: exactly B slots");
            let opened: Vec<String> = slots.iter().map(|(_, r)| format!("{r:?}")).collect();
            // No rejected candidate's row is among the opened payloads.
            for r in &rejected {
                assert!(
                    !opened.contains(r),
                    "seed {seed}: a rejected candidate's payload row was opened: {r}"
                );
            }
            // The opened set is exactly the matched rows plus the dummy encoding.
            let mut got: Vec<String> = opened.clone();
            got.sort();
            got.dedup();
            let mut want: Vec<String> = kept.iter().cloned().chain([pad.clone()]).collect();
            want.sort();
            want.dedup();
            assert_eq!(got, want, "seed {seed}: opened payload set");
            // Every dummy-tagged slot opened to the pad; every real one to its row.
            for (is_real, row) in &slots {
                if *is_real {
                    assert!(kept.contains(&format!("{row:?}")), "seed {seed}");
                } else {
                    assert_eq!(row, &dummy_row(2), "seed {seed}: dummy slot payload");
                }
            }
        }
    }

    /// The same guard in the SECRET-SHARED selector regime: a candidate whose
    /// shared bit is 0 opens to the dummy encoding, not to its row.
    #[test]
    fn rejected_secret_shared_candidates_open_to_the_dummy_encoding() {
        let b = ShamirBackend::new_seeded(5, 0xD1).unwrap();
        let mut dealer = b.dealer();
        let one = dealer.share(Fp::one());
        let zero = dealer.share(Fp::zero());
        let cands = vec![
            Candidate {
                payload: vec![lit("kept"), lit("row")],
                matched: MatchBit::SecretShared(one),
            },
            Candidate {
                payload: vec![lit("rejected"), lit("row")],
                matched: MatchBit::SecretShared(zero),
            },
        ];
        let slots = opened_slot_rows_through(&mut dealer, &b, &cands, 2, 4).unwrap();
        let opened: Vec<String> = slots.iter().map(|(_, r)| format!("{r:?}")).collect();
        assert!(
            !opened.contains(&format!("{:?}", vec![lit("rejected"), lit("row")])),
            "the rejected candidate's row was opened: {opened:?}"
        );
        for (is_real, row) in &slots {
            if !*is_real {
                assert_eq!(row, &dummy_row(2));
            }
        }
    }

    /// A rejected candidate's opened payload is byte-for-byte the encoding a PURE
    /// PADDING slot carries, so the two kinds of dummy slot are not distinguishable
    /// by their payload. Run with a bound larger than the candidate count so both
    /// kinds are present.
    #[test]
    fn a_rejected_candidate_and_a_padding_slot_open_to_the_same_payload() {
        let b = ShamirBackend::new_seeded(3, 0xD2).unwrap();
        let cands = public_candidates(&[
            (false, vec![lit("rejected-a")]),
            (false, vec![lit(&"rejected-b".repeat(20))]),
        ]);
        // B = 5: two rejected candidates and three pure padding slots.
        let slots = opened_slot_rows_through(&mut b.dealer(), &b, &cands, 1, 5).unwrap();
        assert_eq!(slots.len(), 5);
        assert!(slots.iter().all(|(is_real, _)| !*is_real));
        let distinct: std::collections::BTreeSet<String> =
            slots.iter().map(|(_, r)| format!("{r:?}")).collect();
        assert_eq!(
            distinct.len(),
            1,
            "a rejected candidate and a padding slot must open to the same payload: {distinct:?}"
        );
        assert_eq!(slots[0].1, dummy_row(1));
    }

    /// The payload-limb select is counted in the modelled cost: one multiplication
    /// per packed limb per slot, on top of the one tag select per slot. A wider
    /// payload costs strictly more.
    #[test]
    fn payload_select_mults_are_counted_per_limb_per_slot() {
        let b = ShamirBackend::new_seeded(3, 0xD3).unwrap();
        let narrow = public_candidates(&[(true, vec![lit("x")])]);
        let wide = public_candidates(&[(true, vec![lit(&"y".repeat(200))])]);
        let (_, c_narrow) = oblivious_set_output(&b, &narrow, 1, 4).unwrap();
        let (_, c_wide) = oblivious_set_output(&b, &wide, 1, 4).unwrap();
        // The tag select is still one per slot.
        assert_eq!((c_narrow.select_mults, c_wide.select_mults), (4, 4));
        // The entry width the shuffle cost already scales by is `1 + limbs`, so the
        // limb count is recoverable from it and must match the per-slot mult count.
        for c in [c_narrow, c_wide] {
            assert_eq!(c.payload_select_mults % c.bound, 0);
            let limbs = c.payload_select_mults / c.bound;
            assert!(limbs > 0, "a payload always packs at least a length limb");
            assert_eq!(c.shuffle.switches % (1 + limbs), 0);
        }
        assert!(
            c_wide.payload_select_mults > c_narrow.payload_select_mults,
            "a wider payload must cost more limb selects: wide={}, narrow={}",
            c_wide.payload_select_mults,
            c_narrow.payload_select_mults
        );
    }

    // ---- Fail-closed on an out-of-set opened tag (Copilot #93, thread 3) ---------

    /// A `MatchBit::SecretShared` selector that is NOT a sharing of 0/1 makes the
    /// opened tag fall outside `{0, REAL_TAG}`; rather than silently dropping it as
    /// a dummy, the path FAILS CLOSED with a typed `Tampered` error.
    #[test]
    fn out_of_set_tag_fails_closed() {
        let backend = ShamirBackend::new_seeded(3, 7).unwrap();
        let mut dealer = backend.dealer();
        // A malformed selector: a sharing of 5 (not 0 or 1). tag = 5*1 = 5.
        let five = dealer.share(Fp::new(5));
        let cands = vec![Candidate {
            payload: vec![lit("x")],
            matched: MatchBit::SecretShared(five),
        }];
        let err = oblivious_set_output(&backend, &cands, 1, 2).unwrap_err();
        match err {
            MpcError::Tampered { detail, .. } => {
                assert!(
                    detail.contains("neither REAL_TAG") || detail.contains("malformed selector"),
                    "must name the out-of-set tag: {detail}"
                );
            }
            other => panic!("expected Tampered on an out-of-set tag, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod output_randomness_seam_tests {
    //! Issue #6117 — the oblivious output stage draws its selector / real-tag /
    //! pure-dummy / payload-limb sharings through
    //! [`DistributedRandomness::vss_own_input`] and its shuffle through
    //! [`DistributedRandomness::oblivious_shuffle`], instead of minting its own
    //! dealer and calling `share` / [`crate::oblivious::shuffle`] directly. These
    //! tests pin that the single-dealer simulation draws exactly what it drew
    //! before the routing, that every drawn sharing and the shuffled column's
    //! shape are checked where they are consumed, that a mismatched source is
    //! refused before any draw, and that PRSS fails closed.
    use super::*;
    use crate::prss::PrssRandomness;
    use crate::randomness::test_sources::{
        fingerprint, MalformedShuffleSource, MalformedSource, MisreportedThresholdSource,
        RecordingSource, SeamMethod, ALL_MALFORMATIONS, ALL_SHUFFLE_MALFORMATIONS, SHAPE_REFUSAL,
        SHUFFLE_SHAPE_REFUSAL,
    };

    /// The refusal [`crate::randomness::require_source_matches`] raises.
    const MISMATCH_REFUSAL: &str = "its sharings would not compose with the backend's";

    fn lit(s: &str) -> Option<Term> {
        Some(Term::Literal(oxrdf::Literal::new_simple_literal(s)))
    }

    /// The pinned fixture: three candidates matching / not / matching, payload
    /// arity 1, run at `bound = 5` so TWO pure-dummy padding slots are added.
    fn padded_candidates() -> Vec<Candidate> {
        vec![
            Candidate {
                payload: vec![lit("a")],
                matched: MatchBit::Public(true),
            },
            Candidate {
                payload: vec![lit("b")],
                matched: MatchBit::Public(false),
            },
            Candidate {
                payload: vec![lit("c")],
                matched: MatchBit::Public(true),
            },
        ]
    }

    /// The fixture's seam transcript is 20 sharings then one shuffle: per
    /// candidate a selector + a real-tag sharing (6), per padding slot a
    /// zero-selector + a real-tag sharing (4), then two payload limbs per slot
    /// (10) — and finally the oblivious shuffle over the five packed entries.
    const VSS_DRAWS: usize = 20;

    fn rendered(slots: &[OutputSlot]) -> Vec<String> {
        slots
            .iter()
            .map(|s| match s {
                OutputSlot::Row(r) => format!("{:?}", r),
                OutputSlot::Dummy => "Dummy".to_string(),
            })
            .collect()
    }

    /// Pinned from the PRE-ROUTING code (`backend.dealer()` + inline
    /// `dealer.share` + `oblivious::shuffle(dealer, …)`), captured by threading a
    /// draw log through the unmodified `oblivious_set_output` on `origin/main`.
    /// The revealed slot ORDER is the strong pin: it is the shuffle's
    /// permutation, which moves if the stage consumes more, fewer or differently
    /// ordered draws before the shuffle samples it. The WITNESS draw after the run
    /// pins the total draw count.
    #[test]
    fn seeded_output_stage_draws_exactly_what_the_dealer_drew_before_the_seam() {
        let b = ShamirBackend::new_seeded(5, 0x0B15).unwrap();
        let cands = padded_candidates();
        let mut dealer = b.dealer();
        let (slots, cost) = oblivious_set_output_through(&mut dealer, &b, &cands, 1, 5).unwrap();
        assert_eq!(
            rendered(&slots),
            [
                "Dummy",
                "[Some(Literal(Literal(String(\"c\"))))]",
                "[Some(Literal(Literal(String(\"a\"))))]",
                "Dummy",
                "Dummy",
            ]
        );
        assert_eq!(cost.bound, 5);
        assert_eq!(cost.opens, 5);
        assert_eq!(cost.shuffle.items, 5);
        assert_eq!(cost.shuffle.switches, 24);
        // Two payload limbs per slot (length prefix + one data limb): the tag
        // select is still one per slot, the limb select adds two.
        assert_eq!((cost.select_mults, cost.payload_select_mults), (5, 10));
        assert_eq!(dealer.draw_fp().value(), 929230798186952631);

        // The public entry point mints its own dealer from the same seeded
        // backend, so it reproduces the same run.
        let (public, _) = oblivious_set_output(&b, &cands, 1, 5).unwrap();
        assert_eq!(rendered(&public), rendered(&slots));
    }

    /// The COMPLETE seam transcript of the output stage — the 20 sharings in
    /// consumption order, then the shuffled column's shares — folded into the
    /// order-sensitive fingerprint. Captured on the pre-routing code by mapping
    /// each logged `dealer.share` to `vss_own_input` and the shuffle's returned
    /// column to `oblivious_shuffle`.
    #[test]
    fn seeded_output_stage_transcript_is_pinned() {
        let b = ShamirBackend::new_seeded(5, 0x0B15).unwrap();
        let mut rec = RecordingSource::new(b.dealer());
        oblivious_set_output_through(&mut rec, &b, &padded_candidates(), 1, 5).unwrap();
        let methods: Vec<&str> = rec.log.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            methods,
            [
                ["vss_own_input"; VSS_DRAWS].as_slice(),
                &["oblivious_shuffle"]
            ]
            .concat()
        );
        // The 20 sharings are unchanged (same values, same order); only the
        // shuffled column the source returns differs, so the DRAW prefix is pinned
        // equal to its pre-change value and the full-log fingerprint is the new pin.
        assert_eq!(fingerprint(&rec.log[..VSS_DRAWS]), 0x1833_380f_332a_d752);
        assert_eq!(fingerprint(&rec.log), 0x6630_2a41_847f_32c4);
    }

    /// Every sharing the output stage draws — selector, real tag, pure-dummy
    /// selector, payload limb — is shape-checked where it is consumed: corrupting
    /// exactly ONE draw must be refused by the shape check itself.
    #[test]
    fn malformed_draws_are_refused_at_every_output_consumption_site() {
        let b = ShamirBackend::new_seeded(5, 0x0B17).unwrap();
        let cands = padded_candidates();
        for skip in 0..VSS_DRAWS {
            for how in ALL_MALFORMATIONS {
                let mut src = MalformedSource::new(b.dealer(), SeamMethod::VssOwnInput, skip, how);
                match oblivious_set_output_through(&mut src, &b, &cands, 1, 5) {
                    Err(MpcError::Protocol(m)) => assert!(
                        m.contains(SHAPE_REFUSAL),
                        "draw #{skip}, {how:?}: wrong check: {m}"
                    ),
                    other => panic!("draw #{skip}, {how:?}: must fail closed, got {other:?}"),
                }
                assert!(
                    src.fired(),
                    "draw #{skip}: the targeted draw never happened"
                );
            }
        }
    }

    /// The shuffled column comes back from a caller-supplied source, so its SHAPE
    /// is checked, not trusted: a dropped / extra entry, or an entry that lost or
    /// gained a share, would mis-split the packed `[tag | limbs]` layout. Each
    /// corruption must be refused by the shuffle shape check itself.
    #[test]
    fn a_malformed_shuffled_column_is_refused() {
        let b = ShamirBackend::new_seeded(5, 0x0B18).unwrap();
        let cands = padded_candidates();
        for how in ALL_SHUFFLE_MALFORMATIONS {
            let mut src = MalformedShuffleSource::new(b.dealer(), how);
            match oblivious_set_output_through(&mut src, &b, &cands, 1, 5) {
                Err(MpcError::Protocol(m)) => {
                    assert!(
                        m.contains(SHUFFLE_SHAPE_REFUSAL),
                        "{how:?}: wrong check: {m}"
                    )
                }
                other => panic!("{how:?}: must fail closed, got {other:?}"),
            }
            assert!(src.fired(), "{how:?}: the shuffle never happened");
        }
    }

    /// A source whose `(n, t)` differs from the backend's is refused before any
    /// draw: its sharings would not compose with the backend's products or open at
    /// its degrees. A misreported threshold yields canonically SHAPED sharings, so
    /// only the source-match check catches it.
    #[test]
    fn output_stage_refuses_a_mismatched_source_before_any_draw() {
        let b = ShamirBackend::new_seeded(5, 0x0B19).unwrap();
        let cands = padded_candidates();
        for t in [0, 1] {
            let mut src = MisreportedThresholdSource {
                inner: b.dealer(),
                t,
            };
            match oblivious_set_output_through(&mut src, &b, &cands, 1, 5) {
                Err(MpcError::Protocol(m)) => {
                    assert!(m.contains(MISMATCH_REFUSAL), "t={t}: wrong check: {m}")
                }
                other => panic!("t={t}: must fail closed, got {other:?}"),
            }
        }
        let mut rec = RecordingSource::new(ShamirBackend::new_seeded(3, 0x0B19).unwrap().dealer());
        let outcome = oblivious_set_output_through(&mut rec, &b, &cands, 1, 5);
        assert!(
            matches!(&outcome, Err(MpcError::Protocol(m)) if m.contains(MISMATCH_REFUSAL)),
            "got {outcome:?}"
        );
        assert!(rec.log.is_empty(), "drew before refusing");
    }

    // ---- The hidden-key stage's equality bits + output stage (#6117 step 6) ----

    /// The pinned hidden-key fixture: `n = 3`, keys `[1, 2, 3]` against
    /// `[3, 5, 1]`, all nine pairs revealed at `bound = 9` (so no padding slot).
    /// Two pairs match (`0-2` and `2-0`).
    fn hidden_key_fixture() -> (ShamirBackend, [Fp; 3], [Fp; 3]) {
        (
            ShamirBackend::new_seeded(3, 1).unwrap(),
            [Fp::new(1), Fp::new(2), Fp::new(3)],
            [Fp::new(3), Fp::new(5), Fp::new(1)],
        )
    }

    /// The whole hidden-key stage — the per-pair equality bits AND the output
    /// stage — now draws from ONE source, where the output stage previously minted
    /// a second dealer. The REVEALED multiset and every modelled cost are
    /// byte-identical to the pre-routing capture (the revealed ORDER is the
    /// shuffle's secret permutation and carries no result meaning), and the public
    /// entry point reproduces the `_through` run exactly.
    #[test]
    fn hidden_keys_reveal_the_same_multiset_and_cost_through_one_source() {
        let (b, left, right) = hidden_key_fixture();
        let (slots, cost) =
            oblivious_set_output_hidden_keys_through(&mut b.dealer(), &b, &left, &right, 9)
                .unwrap();
        let mut got = rendered(&slots);
        got.sort();
        assert_eq!(
            got,
            [
                "Dummy",
                "Dummy",
                "Dummy",
                "Dummy",
                "Dummy",
                "Dummy",
                "Dummy",
                "[Some(Literal(Literal(String(\"0-2\"))))]",
                "[Some(Literal(Literal(String(\"2-0\"))))]",
            ]
        );
        assert_eq!(
            (
                cost.bound,
                cost.select_mults,
                cost.opens,
                cost.shuffle.items,
                cost.shuffle.switches,
                cost.shuffle.depth_rounds
            ),
            (9, 9, 9, 9, 63, 7)
        );
        // Each slot's payload packs two limbs (length prefix + one data limb), so
        // the limb select adds 2 multiplications per slot on top of the tag select.
        assert_eq!(cost.payload_select_mults, 18);
        let (public, public_cost) = oblivious_set_output_hidden_keys(&b, &left, &right, 9).unwrap();
        assert_eq!(rendered(&public), rendered(&slots));
        assert_eq!(public_cost.shuffle.switches, cost.shuffle.switches);
    }

    /// The COMPLETE seam transcript of the hidden-key stage, folded into the
    /// order-sensitive fingerprint.
    ///
    /// **This is the #6117-step-6 guard for this path.** The log's tail is the
    /// output stage's 27 `vss_own_input` draws (a real-tag sharing per pair — the
    /// selector is the SHARED equality bit, which costs no draw — then two payload
    /// limbs per slot) and its `oblivious_shuffle`. If the stage went back to
    /// minting its own dealer for the output, the recorded log would stop at the
    /// equality chain and both the length and the tail assertion would fail.
    #[test]
    fn seeded_hidden_keys_transcript_is_pinned() {
        let (b, left, right) = hidden_key_fixture();
        let mut rec = RecordingSource::new(b.dealer());
        oblivious_set_output_hidden_keys_through(&mut rec, &b, &left, &right, 9).unwrap();
        assert_eq!(rec.log.len(), 4321);
        // The draws are unchanged (same values, same order); only the shuffled
        // column the source returns differs, so the prefix up to the shuffle is
        // pinned equal to its pre-change value.
        assert_eq!(
            fingerprint(&rec.log[..rec.log.len() - 1]),
            0x108b_b56f_8b48_8a95
        );
        assert_eq!(fingerprint(&rec.log), 0x19a3_e182_ecb3_9943);
        let tail: Vec<&str> = rec.log[rec.log.len() - 28..]
            .iter()
            .map(|(m, _)| *m)
            .collect();
        assert_eq!(
            tail,
            [["vss_own_input"; 27].as_slice(), &["oblivious_shuffle"]].concat()
        );
    }

    /// A source whose `(n, t)` differs from the backend's is refused BEFORE any
    /// draw. The hidden-key stage's equality chain draws first, so the check cannot
    /// be left to [`oblivious_set_output_through`] — it is made up front in
    /// [`oblivious_set_output_hidden_keys_through`] itself.
    #[test]
    fn hidden_keys_refuse_a_mismatched_source_before_any_draw() {
        let (b, left, right) = hidden_key_fixture();
        for t in [0, 2] {
            let mut src = MisreportedThresholdSource {
                inner: b.dealer(),
                t,
            };
            match oblivious_set_output_hidden_keys_through(&mut src, &b, &left, &right, 9) {
                Err(MpcError::Protocol(m)) => {
                    assert!(m.contains(MISMATCH_REFUSAL), "t={t}: wrong check: {m}")
                }
                other => panic!("t={t}: must fail closed, got {other:?}"),
            }
        }
        let mut rec = RecordingSource::new(ShamirBackend::new_seeded(5, 1).unwrap().dealer());
        let outcome = oblivious_set_output_hidden_keys_through(&mut rec, &b, &left, &right, 9);
        assert!(
            matches!(&outcome, Err(MpcError::Protocol(m)) if m.contains(MISMATCH_REFUSAL)),
            "got {outcome:?}"
        );
        assert!(rec.log.is_empty(), "drew before refusing");
    }

    /// The hidden-key stage must share a holder's own input — first inside the
    /// per-pair equality chain, then again in the output stage — which PRSS
    /// refuses. The refusal must surface as PRSS's own `NotYetImplemented`, never a
    /// panic or a silently different output.
    #[test]
    fn prss_fails_closed_in_the_hidden_keys_stage() {
        let b = ShamirBackend::new(3).unwrap();
        let mut prss = PrssRandomness::with_simulated_setup(3).unwrap();
        let left = [Fp::new(1), Fp::new(2)];
        let right = [Fp::new(2), Fp::new(3)];
        assert!(matches!(
            oblivious_set_output_hidden_keys_through(&mut prss, &b, &left, &right, 4),
            Err(MpcError::NotYetImplemented { .. })
        ));
        // And with no pairs at all, where the output stage is the only draw site.
        assert!(matches!(
            oblivious_set_output_hidden_keys_through(&mut prss, &b, &[], &[], 2),
            Err(MpcError::NotYetImplemented { .. })
        ));
    }

    /// The output stage must share the selector and the real tag, which PRSS
    /// refuses (it generates correlated randomness, it does not share a value it
    /// was handed) — the refusal must surface as PRSS's own `NotYetImplemented`,
    /// never a panic or a silently different output.
    #[test]
    fn prss_fails_closed_in_the_output_stage() {
        let b = ShamirBackend::new(5).unwrap();
        let mut prss = PrssRandomness::with_simulated_setup(5).unwrap();
        assert!(matches!(
            oblivious_set_output_through(&mut prss, &b, &padded_candidates(), 1, 5),
            Err(MpcError::NotYetImplemented { .. })
        ));
    }
}
