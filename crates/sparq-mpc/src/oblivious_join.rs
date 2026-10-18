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
//!   multiplication CHAIN, so it is buildable on the single-product backend —
//!   exactly like `crate::join::HiddenValueJoin::secure_equal`, which needs no
//!   reduction to keep MULTIPLYING. Each product is nonetheless re-randomised
//!   through [`crate::shamir::degree_reduce_through`] before the shuffle: the
//!   reduction is there for what the OPEN would otherwise reveal, not to feed a
//!   further multiplication. So the shuffled column is degree `t`. The shuffle is
//!   the sound substrate; the
//!   padded-prefix reveal is a degree-`t` open of `B` slots. This
//!   is the structural obliviousness the bead asks for, and it holds at the
//!   honest-majority Shamir layer.
//! - **The DISCLOSED-key candidate path** — [`oblivious_set_output`] /
//!   [`oblivious_join_output`] from a candidate whose match bit was decided by a
//!   **public / disclosed-key** equi-join (computed OUTSIDE the crypto core,
//!   convention #4). The bit is public so we lift it to a trivial sharing with NO
//!   secure comparison; the transform then hides *which* candidates matched —
//!   output position is independent of input position and a dummy slot is
//!   indistinguishable from a non-match. **What it reveals depends on the output
//!   mode:** on the all-party path ([`oblivious_set_output`] /
//!   [`oblivious_join_output`]) the tags are opened by every compute party, so
//!   each of them also learns the exact count; on the recipient-only path
//!   ([`oblivious_set_output_to_recipient`]) the compute parties open nothing and
//!   the count reaches the designated recipient alone. Either way the revealed
//!   OUTPUT SIZE is `B`. This is fully sound and shippable now.
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
//! - **To the PARTIES / the protocol transcript:** the slot tags are OPENED to the
//!   compute parties, so they learn the exact number of real rows — the true match
//!   count — alongside the public bound `B`. What they do NOT learn is which input
//!   pair produced which output row, or which candidate a given slot came from:
//!   the shuffle destroyed position→candidate linkage, and a dummy slot's opened
//!   payload is the pad whether it came from a non-match or from padding. So L2 is
//!   destroyed and L1 is bounded to `B` in the OUTPUT SIZE, but the count itself is
//!   in the transcript. **That is the ALL-PARTY output mode.** The recipient-only
//!   mode ([`oblivious_set_output_to_recipient`]) runs the identical protocol and
//!   then has the compute parties open NOTHING, so no opened tag and no count is in
//!   their transcript; the designated recipient reconstructs alone. Which mode a
//!   caller is in decides which of these two bullets applies to it.
//! - **To the authorized RESULT RECIPIENT** (who must filter the tagged dummies to
//!   use the result): the true count IS learned, in BOTH output modes — it equals
//!   the number of non-dummy rows after filtering. This is inherent to a padded-bound scheme: the
//!   consumer of a result set learns its size. Hiding the count from the *recipient*
//!   too is exactly the job of the DP cardinality mode (sq-shk5), which adds noise
//!   + extra dummies on top of this transform; it is out of scope here.
//! - **`B` itself is public and caller-agreed.** Choosing `B` too tight FAILS
//!   CLOSED (we never silently drop matches — a candidate count `> B` is an error,
//!   never a truncation that would corrupt the result); choosing it loose costs `B`
//!   opens. `B` leaks an upper bound on the result size by construction.
//! - **What a REFUSAL discloses.** The refusals on this module's open/decode path
//!   carry no opened VALUE — not the opened tag, not a presence byte, a decoded
//!   term, a length prefix, a limb value or a byte count. What they do carry is
//!   WHERE decoding stopped: the failing column index and the failing data limb's
//!   index. Those two positions are themselves functions of the opened block — each
//!   says the earlier columns or limbs decoded and this one did not — so a refusal
//!   localises the malformation without describing it. The rest of what a refusal
//!   carries is public by construction and independent of the block: the two
//!   permitted tag constants, the payload's declared column arity, the block's
//!   uniform byte width, and `BYTES_PER_LIMB`. Where even the count of offending bytes
//!   is a function of the block, the refusal carries no number at all.
//! - **L3 (input cardinalities `|L|`, `|R|`) and the candidate count** are public
//!   (standard MPC assumption; research record §4.1). We do not claim to hide them.
//!   The padding step is where the candidate count shows: it allocates `B − C`
//!   dummy slots for `C` candidates, so the schedule and allocation reveal `C` to
//!   anyone observing the run, whatever a producer's bound check consulted.
//! - **The packed payload WIDTH is public.** Every slot is packed to one uniform
//!   width, and that width is the longest candidate encoding, rounded up to whole
//!   limbs, so it is visible to the parties: it is the per-slot limb count in the
//!   opens, in [`ObliviousOutputCost::payload_select_mults`] and in the shuffle's
//!   switch count. The maximum is taken over ALL candidates, the rejected ones
//!   included, so the width is an upper bound on the length of a candidate
//!   encoding and does not say which candidate reached it. The width is now
//!   BOUNDED, failing closed above its cap — see the width-cap section below — but
//!   bounding it does not hide it: a public width is still public.
//! - Malicious security is unchanged from the Shamir layer. The opens themselves
//!   carry the degree-`t` sharing's own RS redundancy: `n` points on a degree-`t`
//!   polynomial leave `n - t - 1` redundant shares, which at `n = 2t + 1` is `t`,
//!   so up to `⌊t/2⌋` corrupted shares are CORRECTED (at `n = 3`, `t = 1`, there is
//!   no correction budget at all and the decoder is detect-only — see
//!   [`crate::shamir::ShamirBackend::malicious_security`]). **Correcting is what
//!   costs the detection claim.** This is bounded-distance decoding, so a
//!   COORDINATED set of more than `⌊t/2⌋` corrupted shares can force a wrong
//!   value: the received vector lands within the correction budget of a DIFFERENT
//!   valid codeword and the open returns that codeword's value with no
//!   [`MpcError::Tampered`] at all. At `n = 5`, `t = 2`, two parties adding the
//!   evaluations of `δ(x) = (x - 4)(x - 5)` (`+12` at `x = 1`, `+6` at `x = 2`)
//!   get back `s + δ(0) = s + 20`, unflagged; the test
//!   `two_coordinated_share_errors_can_force_a_wrong_open_at_n5` pins it. So
//!   unconditional detection of up to `t` corrupted shares holds for a DETECT-ONLY
//!   decoder; with correction on, only INDEPENDENT errors are caught with the
//!   overwhelming probability [`crate::robust`]'s residual miscorrection note
//!   quantifies.
//! - What has no verification at all is everything BEFORE the open: nothing checks
//!   that a selector is a sharing of 0 or 1, that a select multiplication was
//!   computed honestly, that a degree reduction re-shared the value it was handed,
//!   or that the shuffle permuted anything. A deviation there produces a
//!   well-formed degree-`t` sharing of a different value that no open can flag, so
//!   the protocol stays **semi-honest only** (research record §4.1 D×A; bead
//!   sq-6d6g IT-MAC is the named fix).
//!
//! ## The output-stage WIDTH caps — what is now bounded, and what is not
//!
//! The padded bound `B` has been capped ([`MAX_OUTPUT_BOUND`]) since this stage
//! landed. The per-slot WIDTHS were not, so the following are now capped too, and
//! every one of them is checked **before any sharing is drawn and before any slot
//! is allocated**:
//!
//! - **Payload arity** — [`MAX_PAYLOAD_ARITY`]. Closed form on a public number,
//!   ahead of everything, including the all-unbound dummy row of that arity.
//! - **Per-term serialized byte length** — [`MAX_PAYLOAD_TERM_BYTES`], applied to
//!   each term as it is serialized, so at most the one over-cap term is serialized
//!   before the refusal. A term's length cannot be known without serializing it;
//!   that is the exact ordering claim, no stronger.
//! - **Uniform packed limb width** — [`MAX_SLOT_LIMBS`], on the width that every
//!   slot pays a sharing, a select, a degree reduction and an open per limb for.
//! - **Aggregate slot work `B × arity × limbs`** — [`MAX_SLOT_WORK`], in checked
//!   `u64` arithmetic. Three individually-legal factors still multiply past what
//!   the stage is built to run, so the PRODUCT is bounded in its own right and an
//!   overflowing product is a refusal, not a wrapped number under the budget.
//!
//! Each refusal names PUBLIC quantities only: the cap, the public factors (`B`,
//! the arity, the width) and, for the per-term cap, the public COLUMN position.
//! No term text, no term length and no candidate index appears in any of them.
//!
//! **What is still NOT bounded here.** The uniform width is still decided by the
//! longest candidate rather than agreed in advance, so a caller cannot pin the
//! width its plan will pay before it assembles its candidates; and every slot is
//! still padded to that width, so a single long candidate still widens the whole
//! column (bounding the width does not make it adaptive). And the aggregate budget
//! is charged from the uniform width the candidates HAPPEN to produce, so a caller
//! cannot be told in advance whether its plan will fit it — only refused once the
//! candidates are in hand.
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
//! through the shuffle as secret limbs (Copilot review #93) — plus one degree
//! reduction per selected element, and `B · (1 + limb_capacity)` degree-`t`
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

/// The byte [`bytes_to_limbs`] zero-fills a final partial limb with, and the byte a
/// `PAD_LIMB` limb unpacks to. [`limbs_to_bytes`] requires every byte past the
/// length prefix to be this value, so a limb block carries exactly one byte stream.
const PAD_BYTE: u8 = 0;

/// One disclosed payload row — a column vector of nullable `oxrdf` terms, the same
/// row shape `sparq_engine::QueryResult` / [`PartialResult`] use (`None` = unbound).
type PayloadRow = Vec<Option<Term>>;

/// Internal result of [`shuffle_with_payloads`]: the co-permuted tag column and the
/// payload-limb SHARINGS (one block of `limb_capacity` degree-`t` sharings per slot,
/// in shuffled order — opening and decoding are the caller's step, after it has
/// classified the slot by its tag), the PUBLIC dummy limb block every slot was
/// selected against, the shuffle's modelled cost, the number of payload-limb select
/// multiplications and the number of payload-limb degree reductions the stage
/// performed (one of each per limb per slot).
type ShuffledColumn = (
    SecretColumn,
    Vec<Vec<Vec<Share>>>,
    Vec<Fp>,
    ShuffleCost,
    usize,
    usize,
);

/// A single candidate row entering the oblivious output path: the disclosed
/// payload terms that would be emitted **on a match**, plus the match decision.
///
/// The payload is cleartext `oxrdf` terms (the disclosure-minimisation rule keeps
/// only the *key* hidden — convention #4; the payload columns are disclosed on a
/// match). Whether this candidate matched is carried as a [`MatchBit`], which is
/// either a public bit (disclosed-key regime, sound today) or a secret-shared bit
/// (hidden-key regime, gated on secure-compare — see module docs).
/// `Debug` is hand-written: the payload terms are the row content, and they are
/// carried for EVERY candidate — the rejected ones included — so a derived `Debug`
/// would print rows that no match ever disclosed. The rendering keeps the column
/// count, which is the payload arity the output stage already treats as public,
/// and redacts the terms.
#[derive(Clone)]
pub struct Candidate {
    /// The payload terms emitted if this candidate matches (the join's disclosed
    /// output columns for this `(i, j)` pair).
    pub payload: Vec<Option<Term>>,
    /// Whether this candidate is a match — see [`MatchBit`].
    pub matched: MatchBit,
}

impl std::fmt::Debug for Candidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Candidate")
            .field("payload", &"<redacted>")
            .field("columns", &self.payload.len())
            .field("matched", &self.matched)
            .finish()
    }
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

/// Maximum padded output bound `B` the oblivious output stage accepts before
/// refusing with an [`MpcError::Protocol`] error.
///
/// `B` is the number of slots the stage MATERIALISES: each one allocates a tag
/// sharing and `limb_capacity` payload-limb sharings of `n` [`Share`]s each, costs
/// a select multiplication per limb plus an open, and enters the oblivious
/// shuffle's Waksman/Beneš network. `B` is a PUBLIC plan parameter, so the
/// guard is a closed-form check on a public number — the bound is validated before
/// any allocation or draw, so an over-cap `B` (including `usize::MAX`) is a
/// controlled refusal rather than an allocation the stage cannot satisfy.
///
/// The value is set to cover the in-tree producers of candidates for this stage: it
/// matches [`crate::hidden_path::MAX_CHAIN_TUPLES`], the chain-tuple ceiling of the
/// hidden path operator, and sits well above
/// [`crate::hidden_distinct::MAX_DISTINCT_ROWS`]. It is NOT derived from those
/// ceilings, though, and must not be read as "every producer is guaranteed to fit":
/// a reflexive form such as `{0,k}` presents the chain tuples PLUS up to `|V|`
/// identity pairs, so a producer's candidate count can exceed its own tuple
/// ceiling. What is checked is the ACTUAL candidate count, here and in
/// `check_output_bound`, never a producer's assumed maximum. A protocol guard,
/// not a tuning knob: `B` beyond it is not a workload this stage is built to run.
pub const MAX_OUTPUT_BOUND: usize = 4096;

/// Maximum payload ARITY (number of payload columns per slot) the oblivious output
/// stage accepts before refusing with an [`MpcError::Protocol`] error.
///
/// The arity is a PUBLIC schema width: every slot carries exactly this many
/// nullable terms, the all-unbound dummy row of this arity is materialised once as
/// the public pad every slot is selected against, and the decode walks exactly this
/// many columns. So the guard is a closed-form check on a public number and it runs
/// before the dummy row is built — which is what keeps a `payload_arity` of
/// `usize::MAX` a controlled refusal instead of a `vec![None; usize::MAX]` the
/// allocator cannot satisfy.
///
/// A protocol guard, not a tuning knob: the in-tree producers of candidates for
/// this stage emit two payload columns (the endpoint pair), and a result row
/// dozens of columns wide is already past what this stage is built to run.
pub const MAX_PAYLOAD_ARITY: usize = 64;

/// Largest per-term byte width a caller may DECLARE in a [`PayloadWidths`], and
/// so the largest serialized payload term the stage will carry.
///
/// A payload term is carried through the shuffle as its canonical N-Triples form
/// (see `encode_payload_row`), and every slot is padded to the DECLARED width, so
/// the declared per-term width — not any term's actual length — is what fixes the
/// packed limb width of every slot. Each limb of each slot pays a sharing, a
/// select multiplication, a degree reduction and an open, so a declaration is a
/// cost commitment and is bounded here.
///
/// The declared width, not this cap, is what a term is measured against: this is
/// the ceiling on the declaration itself.
pub const MAX_PAYLOAD_TERM_BYTES: usize = 4096;

/// The per-term width a bare arity declares (`PayloadWidths::from(arity)`).
///
/// A caller that does not state a width gets this one. It is a public number like
/// any other declaration, chosen to carry the short IRIs and literals the in-tree
/// producers emit; a caller whose schema is wider declares it explicitly with
/// [`PayloadWidths::declare`] and pays the wider padding.
pub const DEFAULT_PAYLOAD_TERM_BYTES: usize = 128;

/// Maximum uniform packed limb width (limbs per slot) the oblivious output stage
/// accepts before refusing with an [`MpcError::Protocol`] error.
///
/// The width is `1 + ⌈row_bytes / 7⌉` over the DECLARED row width (see
/// [`PayloadWidths::limb_capacity`]) and is applied to every slot, real or dummy.
/// Each limb of each slot costs one sharing, one select multiplication, one degree
/// reduction (itself `2t + 1` re-sharings through the randomness seam), one
/// position in the shuffle network's entry width and one degree-`t` open — so the
/// width is the per-slot work multiplier and is bounded on its own, not only
/// through the arity and per-term widths it follows from.
pub const MAX_SLOT_LIMBS: usize = 2048;

/// Maximum compute-party count the output stage's work budget accepts.
///
/// Every sharing the stage draws is `parties` field elements, and the degree
/// reduction behind each select re-shares `2t + 1 <= parties` times, so the party
/// count multiplies the work twice over. It is a public plan parameter of the
/// backend, checked in closed form. This bounds THIS stage's budget; it is not a
/// crate-wide limit on how many parties a `ShamirBackend` may have.
pub const MAX_OUTPUT_PARTIES: usize = 7;

/// Maximum aggregate slot work the oblivious output stage accepts before refusing
/// with an [`MpcError::Protocol`] error, counted in MODELLED FIELD ELEMENTS.
///
/// The figure is `B × (1 + limbs) × parties × (2t + 1)`:
///
/// - `B` slots, each a tag element plus `limbs` packed limbs of the declared
///   width — the `1 +` is why a zero-width run is still charged for its per-slot
///   tag sharing, select, degree reduction and open;
/// - `parties`, because one sharing is one field element per party; and
/// - `2t + 1`, the re-sharings of the degree reduction behind every select.
///
/// The payload ARITY is deliberately NOT a factor: the declared arity is already
/// inside `limbs` ([`PayloadWidths::limb_capacity`] packs every column into the
/// row), so multiplying by it again would count each column twice and leave the
/// budget binding only at arities no caller uses.
///
/// Capping the FACTORS does not cap their PRODUCT: a `B`, a width and a party
/// count that are each individually legal multiply to an amount of sharing,
/// selecting, reducing and opening work no combination of the individual caps
/// alone would exclude. The product is what the stage performs, so it is bounded
/// in its own right, with checked arithmetic in `u64` (so the budget means the
/// same on every target width) — an overflowing product is a refusal, never a
/// wrapped number that would slip under the budget.
///
/// Every factor is a PUBLIC plan quantity: `B`, the arity and the per-term width
/// are the caller's declaration and `(n, t)` are the backend's, so the check and
/// its refusal are closed form over public numbers and no candidate is read to
/// decide them.
///
/// The unit matters when comparing this to the path-form budget:
/// [`crate::bounded_path::MAX_PATH_FORM_WORK`] counts STATIC UNROLL STEPS of a
/// property-path plan, while this counts field elements the output stage shares,
/// selects, reduces and opens. They are different quantities and their values are
/// not comparable.
pub const MAX_SLOT_WORK: u64 = 1 << 28; // 268_435_456 modelled field elements

/// The PUBLIC payload widths one output-stage run is padded to: the column count
/// and the per-term byte width.
///
/// ## Why the caller declares this
///
/// Every slot the stage emits is padded to ONE uniform width, so that width is a
/// property of the whole run and must be the same number on every party. It used
/// to be computed as the maximum over the candidates' own encodings, which made it
/// a function of the data: two runs over the same plan could pad to different
/// widths, and the stage had to serialize and hold every candidate before it could
/// decide anything. Both are gone. The width is declared up front from public
/// numbers, checked against the caps before a candidate is read, and a term that
/// does not fit the declaration is refused.
///
/// A width cannot be learned publicly from a candidate set — a term's serialized
/// length is only knowable by serializing it — and padding every run to
/// [`MAX_PAYLOAD_TERM_BYTES`] would make every run pay the widest legal schema. So
/// the caller declares, [`DEFAULT_PAYLOAD_TERM_BYTES`] covers a caller that states
/// only an arity, and the declaration itself is capped.
///
/// What this does NOT do: the declared width is public, so it is visible to every
/// party and in the cost model, exactly as the data-derived width was. Declaring
/// it makes the width independent of the data and the same on every party; it does
/// not hide it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PayloadWidths {
    arity: usize,
    term_bytes: usize,
}

impl PayloadWidths {
    /// Declare `arity` payload columns of up to `term_bytes` serialized bytes each,
    /// checking the arity cap, the per-term declaration cap and the uniform limb
    /// width that follows from them — all closed form on public numbers.
    pub fn declare(arity: usize, term_bytes: usize) -> Result<Self, MpcError> {
        let widths = PayloadWidths { arity, term_bytes };
        widths.check_shape()?;
        Ok(widths)
    }

    /// The declared column count.
    pub fn arity(&self) -> usize {
        self.arity
    }

    /// The declared per-term byte width.
    pub fn term_bytes(&self) -> usize {
        self.term_bytes
    }

    /// The declared row width in bytes: per column a presence flag, the term and
    /// its NUL terminator. `None` when the declaration overflows `usize`, which the
    /// arity and per-term caps refuse.
    pub fn row_bytes(&self) -> Option<usize> {
        self.term_bytes
            .checked_add(2)
            .and_then(|per_col| self.arity.checked_mul(per_col))
    }

    /// The uniform packed limb width every slot is padded to — a closed form of
    /// the declaration, with no candidate read. `None` on the same overflow
    /// `row_bytes` reports.
    pub fn limb_capacity(&self) -> Option<usize> {
        self.row_bytes().map(limb_count_for)
    }

    /// Check the declaration against [`MAX_PAYLOAD_ARITY`],
    /// [`MAX_PAYLOAD_TERM_BYTES`] and [`MAX_SLOT_LIMBS`]. Public numbers only.
    pub(crate) fn check_shape(&self) -> Result<(), MpcError> {
        check_payload_arity_cap(self.arity)?;
        if self.term_bytes > MAX_PAYLOAD_TERM_BYTES {
            return Err(MpcError::Protocol(format!(
                "oblivious output stage: declared per-term width {} bytes exceeds the \
                 MAX_PAYLOAD_TERM_BYTES cap of {} — refusing on the declaration alone, \
                 before any candidate is read",
                self.term_bytes, MAX_PAYLOAD_TERM_BYTES
            )));
        }
        let limbs = self.limb_capacity().ok_or_else(|| {
            MpcError::Protocol(format!(
                "oblivious output stage: declared arity {} at {} bytes per term overflows a \
                 usize row width — refusing on the declaration alone",
                self.arity, self.term_bytes
            ))
        })?;
        check_slot_limbs_cap(limbs)
    }
}

/// A bare arity declares that arity at [`DEFAULT_PAYLOAD_TERM_BYTES`].
///
/// Infallible by construction, so it does no checking: the stage checks the
/// declaration it is handed before it reads anything. [`PayloadWidths::declare`]
/// is the checked constructor.
impl From<usize> for PayloadWidths {
    fn from(arity: usize) -> Self {
        PayloadWidths {
            arity,
            term_bytes: DEFAULT_PAYLOAD_TERM_BYTES,
        }
    }
}

/// The payload-arity cap, callable before a candidate set or a dummy row exists.
/// See [`MAX_PAYLOAD_ARITY`].
pub(crate) fn check_payload_arity_cap(payload_arity: usize) -> Result<(), MpcError> {
    if payload_arity > MAX_PAYLOAD_ARITY {
        return Err(MpcError::Protocol(format!(
            "oblivious output stage: payload arity {} exceeds the MAX_PAYLOAD_ARITY cap of {} \
             — refusing (the arity is the public per-slot column count and the all-unbound \
             dummy row of that arity is materialised, so it is bounded before anything is \
             allocated or drawn)",
            payload_arity, MAX_PAYLOAD_ARITY
        )));
    }
    Ok(())
}

/// The uniform packed-limb-width cap. See [`MAX_SLOT_LIMBS`].
pub(crate) fn check_slot_limbs_cap(limb_capacity: usize) -> Result<(), MpcError> {
    if limb_capacity > MAX_SLOT_LIMBS {
        return Err(MpcError::Protocol(format!(
            "oblivious output stage: packed payload width {} limbs exceeds the MAX_SLOT_LIMBS \
             cap of {} — refusing before any sharing or draw (the width is uniform over the \
             column, so every slot pays a sharing, a select, a degree reduction and an open \
             per limb)",
            limb_capacity, MAX_SLOT_LIMBS
        )));
    }
    Ok(())
}

/// The aggregate slot-work budget, computed with checked arithmetic.
/// See [`MAX_SLOT_WORK`] for the figure, its unit and why each factor is in it.
pub(crate) fn check_slot_work_budget(
    bound: usize,
    limb_capacity: usize,
    parties: usize,
    threshold: usize,
) -> Result<(), MpcError> {
    // The tag element is charged alongside the limbs, so a zero-width run is still
    // charged for the per-slot tag sharing, select, reduction and open.
    let per_slot = (limb_capacity as u64).checked_add(1);
    // `2t + 1` re-sharings per degree reduction, which honest majority keeps <= n.
    let reshares = (threshold as u64)
        .checked_mul(2)
        .and_then(|v| v.checked_add(1));
    let work = per_slot
        .and_then(|w| w.checked_mul(bound as u64))
        .and_then(|v| v.checked_mul(parties as u64))
        .and_then(|v| reshares.and_then(|r| v.checked_mul(r)));
    match work {
        Some(w) if w <= MAX_SLOT_WORK => Ok(()),
        other => Err(MpcError::Protocol(format!(
            "oblivious output stage: aggregate slot work B={} x (1 + limbs={}) x parties={} \
             x (2t+1={}) is {} modelled field element(s), exceeding the MAX_SLOT_WORK budget \
             of {} — refusing before any candidate is read (each factor is within its own \
             cap; their product is the work the stage performs and is bounded in its own \
             right)",
            bound,
            limb_capacity,
            parties,
            reshares
                .map(|r| r.to_string())
                .unwrap_or_else(|| "overflow".to_string()),
            other
                .map(|w| w.to_string())
                .unwrap_or_else(|| "an amount that overflows u64".to_string()),
            MAX_SLOT_WORK
        ))),
    }
}

/// The compute-party cap for this stage's work budget. See [`MAX_OUTPUT_PARTIES`].
pub(crate) fn check_output_parties_cap(parties: usize) -> Result<(), MpcError> {
    if parties > MAX_OUTPUT_PARTIES {
        return Err(MpcError::Protocol(format!(
            "oblivious output stage: {} compute parties exceeds the MAX_OUTPUT_PARTIES cap \
             of {} — refusing before any candidate is read (every sharing spans one field \
             element per party and every select's degree reduction re-shares up to that \
             many times, so the party count multiplies the per-limb work twice over)",
            parties, MAX_OUTPUT_PARTIES
        )));
    }
    Ok(())
}

/// The cap half of the fail-closed bound contract, callable before a candidate set
/// exists — so a public entry that BUILDS its candidates (and draws while doing so)
/// can validate its bound first. See [`MAX_OUTPUT_BOUND`].
pub(crate) fn check_output_bound_cap(bound: usize) -> Result<(), MpcError> {
    if bound > MAX_OUTPUT_BOUND {
        return Err(MpcError::Protocol(format!(
            "oblivious output stage: padded bound B={} exceeds the MAX_OUTPUT_BOUND cap of {} — \
             refusing (B slots are materialised, shuffled and opened, so B is bounded before \
             anything is allocated or drawn)",
            bound, MAX_OUTPUT_BOUND
        )));
    }
    Ok(())
}

/// The bound contract, decided from the PUBLIC input sizes FIRST and from the
/// exact candidate count only if it has to be.
///
/// `public_bound` is an upper bound on the candidate count derived from public
/// sizes alone (an edge count, a hop count) — never a statistic over the caller's
/// data. `exact_count` computes the real count, and is called ONLY on the branch
/// where it is needed.
///
/// - `B >= public_bound`: accepted on public sizes alone. `exact_count` is not
///   called, so the DECISION consults no statistic over the data, and no `B` the
///   data would have allowed is ever refused.
/// - `B < public_bound`: the caller has chosen a tighter `B` than the public
///   worst case, so coverage can only be settled against the real count. It is
///   computed here, before any draw, and a `B` that cannot cover it is refused.
///   Success-or-refusal therefore discloses whether the candidate count is at most
///   `B`; and since a refusal draws nothing, repeated calls with different `B`
///   recover the count exactly by binary search — up to [`MAX_OUTPUT_BOUND`], past
///   which every probe is refused by the cap and the caller learns only that the
///   count is larger. That is the caller's own trade for the tighter bound.
///
/// Either way the message names `B` and `public_bound`, both already public, and
/// never the candidate count. **This is a check contract, not a hiding mechanism:**
/// it keeps the candidate count out of the decision and the error text, and says
/// nothing about what the producer itself discloses. The hidden-path producers
/// group on every enumerated tuple's endpoint pair, read in the clear, and multiply
/// once more per repeat, so for them the fold POSITIONS — and hence that count —
/// are disclosed on BOTH branches of this contract to the grouping evaluator here,
/// and to every evaluating party in a distributed implementation that supplies them
/// with the ordered plaintext endpoint terms or with that construction's
/// data-dependent accumulator wiring. This crate implements a single-process
/// simulation and does not establish a per-party input or wiring-distribution
/// boundary. See [`crate::hidden_path`] for who learns what, and for the limits of
/// that account. A count above [`MAX_OUTPUT_BOUND`] needs no branch of
/// its own: `B` is capped first, so a count that large cannot be covered and falls
/// out of the same refusal.
pub(crate) fn check_bound_public_first(
    what: &str,
    bound: usize,
    public_bound: usize,
    exact_count: impl FnOnce() -> Result<usize, MpcError>,
) -> Result<(), MpcError> {
    check_output_bound_cap(bound)?;
    if bound >= public_bound {
        return Ok(());
    }
    if bound < exact_count()? {
        return Err(MpcError::Protocol(format!(
            "{}: padded bound B={} is below the public candidate bound {} AND does not cover \
             the candidate set — refusing before any draw (a B at or above the public bound is \
             decided from the public input sizes alone)",
            what, bound, public_bound
        )));
    }
    Ok(())
}

/// The whole fail-closed bound contract: `B` within [`MAX_OUTPUT_BOUND`] (checked
/// first, since it is the one that would otherwise reach an allocation), the
/// candidate COUNT within the same cap, and `B` covering every candidate so no
/// match is ever silently truncated.
///
/// Its messages name the candidate count, so call it only where that count is
/// something the CALLER already holds — it owns the candidate slice, or the count
/// is a product of public input lengths. Where the count is a statistic over
/// hidden or unmatched data, use `check_bound_public_first` instead: it decides
/// from the public sizes whenever `B` allows, keeps the count out of the message
/// either way, and leaves the one-bit disclosure to callers that ask for a
/// tighter `B`.
pub(crate) fn check_output_bound(bound: usize, candidates: usize) -> Result<(), MpcError> {
    check_output_bound_cap(bound)?;
    if candidates > MAX_OUTPUT_BOUND {
        // A count above the cap can never be served: it would need a `B` at least
        // that large, which the cap refuses. Saying so names the cap rather than
        // leaving the caller with a `B < candidates` message it cannot act on.
        return Err(MpcError::Protocol(format!(
            "oblivious output stage: candidate count {} exceeds the MAX_OUTPUT_BOUND cap \
             of {} — refusing (every candidate becomes one materialised slot, so no \
             legal B could cover this many)",
            candidates, MAX_OUTPUT_BOUND
        )));
    }
    if bound < candidates {
        return Err(MpcError::Protocol(format!(
            "oblivious_set_output: padded bound B={} < candidate count {} — would truncate \
             candidates and could drop a true match",
            bound, candidates
        )));
    }
    Ok(())
}

/// The number of candidates an ALL-PAIRS builder would materialise, computed with
/// CHECKED arithmetic and capped — callable before the candidate set exists.
///
/// `left · right` is a product of two caller-supplied lengths, so it can overflow
/// `usize` (and an overflowed `Vec::with_capacity` is an allocation nobody asked
/// for) and it can be enormous while still fitting. Both are refused here, so a
/// builder can preflight the cardinality it is ABOUT to enumerate before it
/// allocates a slot or draws a sharing. The cap is [`MAX_OUTPUT_BOUND`], the same
/// ceiling the stage's own `B` has: each candidate becomes one materialised slot.
pub(crate) fn checked_candidate_pairs(left: usize, right: usize) -> Result<usize, MpcError> {
    let count = left.checked_mul(right).ok_or_else(|| {
        MpcError::Protocol(format!(
            "oblivious output stage: candidate count {} x {} overflows usize — refusing \
             before any allocation or draw (each candidate pair becomes one \
             materialised slot)",
            left, right
        ))
    })?;
    if count > MAX_OUTPUT_BOUND {
        return Err(MpcError::Protocol(format!(
            "oblivious output stage: candidate count {} (= {} x {}) exceeds the \
             MAX_OUTPUT_BOUND cap of {} — refusing before any allocation or draw \
             (each candidate is materialised, selected, shuffled and opened)",
            count, left, right, MAX_OUTPUT_BOUND
        )));
    }
    Ok(count)
}

/// [`checked_candidate_pairs`] plus the caller's own bound contract: the whole
/// preflight an all-pairs builder needs before it enumerates anything.
pub(crate) fn check_candidate_pairs_within_bound(
    bound: usize,
    left: usize,
    right: usize,
) -> Result<usize, MpcError> {
    // The bound's own cap first, so the ordering is the same at every entry: no
    // caller-sized work of any kind happens ahead of it.
    check_output_bound_cap(bound)?;
    let count = checked_candidate_pairs(left, right)?;
    check_output_bound(bound, count)?;
    Ok(count)
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
    /// unselected. This field therefore reflects that width, which is the longest
    /// candidate encoding rounded up to whole limbs and is public to the parties
    /// (see the module's residual-leakage section).
    pub payload_select_mults: usize,
    /// Degree reductions: one per SELECTED element — the slot tag and each of its
    /// payload limbs — so `bound · (1 + limb_capacity)` in all. This counts the
    /// REDUCTIONS, not rounds: each one is a [`shamir::degree_reduce_through`] that
    /// costs `2t + 1` re-sharings through the seam, and they are independent, so
    /// all of them run in ONE round. Every selected
    /// element is a product sharing, and a product sharing opened without
    /// re-randomisation reveals more than its value, so this is not optional work.
    pub degree_reductions: usize,
    /// Degree-`t` opens of the slot TAGS performed at reveal (one per slot). The
    /// payload limbs are opened too, also at degree `t` — every selected element is
    /// degree-reduced before the shuffle. This field counts the tag opens only, as
    /// it always has.
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
/// ORDER. A [`OutputSlot::Dummy`] is the padding that makes the revealed SLOT
/// COUNT exactly `B` whatever the true match count is, and that hides which
/// candidate a slot came from. The tags are opened, so the number of real slots is
/// in the transcript — see the module's residual-leakage note. The authorized
/// recipient filters dummies to obtain the result.
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
/// 5. **Open and classify all `B` tags FIRST**, then the payload limbs — every
///    open at degree `t`, since every selected element is degree-reduced before
///    the shuffle. A tag outside `{0, REAL_TAG}` FAILS CLOSED before a single
///    limb of any slot is opened. Then each slot's limbs are opened in the same
///    shuffled order, and a slot the tag called a dummy must carry the pad block
///    EXACTLY — the comparison is on the opened limb VECTOR, not on the decoded
///    row — or the call refuses. Decoding is strict. Emit [`OutputSlot::Row`]
///    where the tag was the real sentinel and [`OutputSlot::Dummy`] where it was
///    `0`.
///
/// ## Who opens, and what that reveals
///
/// On THIS path the opens are the compute parties': every compute party opens all
/// `B` tags, so every one of them learns which slots are real and therefore the
/// exact match count. That is unchanged behaviour, stated plainly. For the
/// arrangement where the compute parties open nothing and one designated recipient
/// reconstructs alone, use [`oblivious_set_output_to_recipient`].
///
/// ## Fail-closed bound contract
///
/// `B` must be `>= candidates.len()` so every candidate gets a slot — we NEVER
/// silently truncate candidates (that could drop a true match and corrupt the
/// result). A `B < candidates.len()` is a [`MpcError::Protocol`] error. (The
/// caller sizes `B` from the public candidate count `|L|·|R|`, optionally padded,
/// so `B` never tracks the true MATCH count.)
///
/// `B` must also be `<= `[`MAX_OUTPUT_BOUND`]. Both halves are checked **before any
/// allocation or draw** — the stage materialises `B` slots, so the bound is
/// validated first and an over-cap `B` is a [`MpcError::Protocol`] error.
pub fn oblivious_set_output(
    backend: &ShamirBackend,
    candidates: &[Candidate],
    widths: impl Into<PayloadWidths>,
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    // Preflight the bound BEFORE a dealer exists: `output_stage_shuffled_through`
    // checks it again as its first statement, so this only keeps a refused run from
    // creating keystream state.
    check_output_bound(bound, candidates.len())?;
    // Mint ONE fresh dealer for this run (production: a fresh OS-seeded CSPRNG,
    // sq-1vt) and draw every sharing and the shuffle through the seam on it.
    oblivious_set_output_through(
        &mut backend.dealer(),
        backend,
        candidates,
        widths.into(),
        bound,
    )
}

/// [`oblivious_set_output`] with every sharing and the shuffle drawn through
/// `source`, in this order: per candidate its selector sharing, its real-tag
/// sharing and then the `2t + 1` re-sharings of the degree reduction that
/// re-randomises its tag select; then the same triple for each pure-dummy padding
/// slot; then, per slot and per packed limb, the limb sharing followed by the
/// `2t + 1` re-sharings of that limb's reduction; and last the oblivious shuffle
/// itself.
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
/// ## Every selected element is degree-reduced before the shuffle
///
/// Step 3's `tag_out = sel · real_tag` is a [`shamir::mul_shares_raw`] product and
/// each `limb_out = PAD + sel · (limb − PAD)` is a product of the same selector
/// with a locally-shifted limb sharing, so EVERY element of a packed entry — the
/// tag and all its payload limbs — leaves the select as a degree-`2t` product
/// sharing. A product sharing opened without re-randomisation reveals more than
/// its value: its share vector still carries the factor polynomials, so the
/// opened shares of a non-matching candidate's slot would not match those of a
/// padding slot. Each selected element is therefore re-randomised through the
/// seam with [`shamir::degree_reduce_through`] before the shuffle, which replaces
/// it with a FRESH degree-`t` sharing of the same value whose non-constant
/// coefficients are independent of the factors. Adding the public `PAD` constant
/// back is local and does not change the degree.
///
/// Step 5 therefore opens the tags AND the limbs at degree `t`. The select
/// multiplication still requires `n >= 2t + 1`.
///
/// Because the shuffled column is degree `t`, it also meets the precondition of a
/// source that shuffles by RECOMBINING, as a dealer-less one must
/// ([`crate::prss::PrssRandomness`] refuses an element above degree `t`). That
/// path stays unreachable today for a different reason: the only dealer-less
/// source refuses `vss_own_input`, so this function fails closed on the selector
/// sharing long before the shuffle, and the dealer-less VSS step (#6117) is not
/// work this stage can do on its own.
pub(crate) fn oblivious_set_output_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    widths: impl Into<PayloadWidths>,
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    let widths: PayloadWidths = widths.into();
    let (decoded, cost) =
        oblivious_set_output_decoded_through(source, backend, candidates, widths, bound)?;
    Ok((narrow_decoded_slots(decoded, widths.arity())?, cost))
}

/// The output stage's result as the compute parties leave it: the `B` slots'
/// degree-`t` SHARINGS, addressed to one designated recipient, with nothing opened.
///
/// ## What this is for
///
/// On the all-party path ([`oblivious_set_output`]) every compute party opens the
/// `B` slot tags, so every compute party learns which slots are real and therefore
/// the exact match count. This type is the other arrangement: the compute parties
/// run the whole stage — selectors, selects, degree reductions, the oblivious
/// shuffle — and then **open nothing**. Each party sends its own share of each
/// slot to the designated recipient, who reconstructs alone with
/// [`Self::reconstruct`]. The compute parties' transcript contains no opened tag
/// and no opened payload limb, so the match count is not in it.
///
/// ## Honest boundary: what this does and does not change
///
/// - The **recipient** learns exactly what the all-party path's recipient learned:
///   the `B` slots, hence the true count. Hiding the count from the recipient too
///   is the separate DP-cardinality mode (bead **sq-shk5**), not this.
/// - This crate is an **in-process simulation**: one process plays every party, so
///   "each party sends its share" is this one value carrying all `n` shares. A real
///   deployment sends `n` messages, one per party, each holding only that party's
///   share — the per-party split is a transport concern and there is no wire format
///   for it yet, so it is recorded here rather than faked. Nothing in this type
///   enforces that a party sent only its own share.
/// - Honest-majority, **semi-honest only**, unchanged (external soundness sign-off
///   is still pending, sq-qhy4). A recipient that reconstructs alone has no second
///   opinion on the shares it was sent beyond the degree-`t` sharing's own RS
///   redundancy, which is what [`crate::robust`] already gives the all-party path.
/// - The [`std::fmt::Debug`] rendering is **redacting**: it prints the recipient,
///   the bound and the payload arity, and no share values. A derived `Debug` would
///   print every share this value carries, and a debug print is where
///   secret-shared data escapes a type by accident — a log line, a panic message,
///   an `assert_eq!` failure.
#[derive(Clone)]
pub struct RecipientOutputShares {
    recipient: HolderId,
    bound: usize,
    payload_arity: usize,
    threshold: usize,
    /// Per slot: the tag sharing and its payload-limb sharings, in shuffled order.
    slots: Vec<(Vec<Share>, Vec<Vec<Share>>)>,
    /// The PUBLIC dummy limb block every slot was selected against. Public by
    /// construction — it depends only on the payload arity — so carrying it to the
    /// recipient reveals nothing about the rows.
    dummy_block: Vec<Fp>,
}

/// Redacting: the public shape only, never a share value. See the type's docs.
impl std::fmt::Debug for RecipientOutputShares {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecipientOutputShares")
            .field("recipient", &self.recipient)
            .field("bound", &self.bound)
            .field("payload_arity", &self.payload_arity)
            .field("slots", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl RecipientOutputShares {
    /// The holder the shares are addressed to.
    pub fn recipient(&self) -> &HolderId {
        &self.recipient
    }

    /// The public padded bound `B` — the number of slots carried.
    pub fn bound(&self) -> usize {
        self.bound
    }

    /// The payload arity the slots decode to.
    pub fn payload_arity(&self) -> usize {
        self.payload_arity
    }

    /// Reconstruct the `B` slots. **The recipient runs this; no compute party
    /// does.**
    ///
    /// Applies exactly what the all-party path applies at step 5: all `B` tags are
    /// opened and classified before any payload limb is opened, a tag outside
    /// `{0, REAL_TAG}` is a [`MpcError::Tampered`] refusal, a dummy-tagged slot's
    /// opened limb vector must equal the dummy block exactly, and decoding is
    /// strict. Nothing is weakened by moving the open to one party.
    pub fn reconstruct(&self) -> Result<Vec<OutputSlot>, MpcError> {
        let tags: Vec<Vec<Share>> = self.slots.iter().map(|(t, _)| t.clone()).collect();
        let limbs: Vec<Vec<Vec<Share>>> = self.slots.iter().map(|(_, l)| l.clone()).collect();
        let decoded = open_and_decode_slots(
            &tags,
            &limbs,
            &self.dummy_block,
            self.payload_arity,
            self.threshold,
        )?;
        narrow_decoded_slots(decoded, self.payload_arity)
    }

    /// [`Self::reconstruct`] assembled into a [`PartialResult`]: dummies filtered,
    /// rows canonicalised, attributed to the `federation` holder — the same shape
    /// [`oblivious_join_output`] returns on the all-party path.
    ///
    /// Fails closed ([`MpcError::Protocol`]) unless `out_vars.len()` equals the
    /// [`Self::payload_arity`] the slots were built with, **before reconstructing
    /// anything**. On the all-party path the arity cannot disagree because
    /// [`oblivious_join_output`] DERIVES it from `out_vars`; here the arity was
    /// fixed when the compute parties ran the stage, so a caller can hand in a
    /// different variable list. Accepting one would return a `PartialResult` whose
    /// `vars` and row widths disagree — a result the consumer would read with the
    /// columns misaligned, or short.
    pub fn reconstruct_partial_result(
        &self,
        out_vars: Vec<Variable>,
    ) -> Result<PartialResult, MpcError> {
        if out_vars.len() != self.payload_arity {
            return Err(MpcError::Protocol(format!(
                "RecipientOutputShares::reconstruct_partial_result: {} output variable(s) \
                 for slots of payload arity {}; the variable list must match the arity the \
                 compute parties packed",
                out_vars.len(),
                self.payload_arity
            )));
        }
        Ok(partial_result_from_slots(self.reconstruct()?, out_vars))
    }
}

/// **Recipient-only output reconstruction.** Runs the whole oblivious output stage
/// and hands back the `B` slots' degree-`t` sharings WITHOUT opening anything, for
/// the designated `recipient` to reconstruct alone
/// ([`RecipientOutputShares::reconstruct`]).
///
/// This is [`oblivious_set_output`] with step 5 moved off the compute parties. The
/// protocol up to and including the shuffle is byte-for-byte the same; what changes
/// is who opens. The compute parties open nothing, so the reported
/// [`ObliviousOutputCost::opens`] is `0` for this path — the `B` tag opens are the
/// recipient's, not theirs — and the match count is not in their transcript.
///
/// Both paths ship: the all-party [`oblivious_set_output`] is what every in-crate
/// operator calls today and is unchanged, and its docs say plainly that it reveals
/// the match count to the compute parties. See [`RecipientOutputShares`] for the
/// honest boundary — in particular that this crate is an in-process simulation, so
/// the per-party message split is a transport concern that does not exist yet.
pub fn oblivious_set_output_to_recipient(
    backend: &ShamirBackend,
    candidates: &[Candidate],
    widths: impl Into<PayloadWidths>,
    bound: usize,
    recipient: HolderId,
) -> Result<(RecipientOutputShares, ObliviousOutputCost), MpcError> {
    // Same pre-dealer preflight as the all-party path: `B` sizes every per-slot
    // allocation and the shuffle network, and the recipient-only mode runs the very
    // same stage, so an over-cap or short `B` is refused here and again inside it.
    check_output_bound(bound, candidates.len())?;
    oblivious_set_output_to_recipient_through(
        &mut backend.dealer(),
        backend,
        candidates,
        widths.into(),
        bound,
        recipient,
    )
}

/// [`oblivious_set_output_to_recipient`] with every sharing and the shuffle drawn
/// through a caller-supplied [`DistributedRandomness`] source, so one source covers
/// an operator end to end (#6117 step 6).
pub(crate) fn oblivious_set_output_to_recipient_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    widths: impl Into<PayloadWidths>,
    bound: usize,
    recipient: HolderId,
) -> Result<(RecipientOutputShares, ObliviousOutputCost), MpcError> {
    let widths: PayloadWidths = widths.into();
    let (tags, limb_blocks, dummy_block, mut cost) =
        output_stage_shuffled_through(source, backend, candidates, widths, bound)?;
    // The compute parties opened nothing: the `B` tag opens belong to the recipient.
    cost.opens = 0;
    Ok((
        RecipientOutputShares {
            recipient,
            bound,
            payload_arity: widths.arity(),
            threshold: backend.threshold(),
            slots: tags.into_iter().zip(limb_blocks).collect(),
            dummy_block,
        },
        cost,
    ))
}

/// Narrow the opened slots to the public [`OutputSlot`] shape.
///
/// A slot whose tag is the dummy sentinel carries the dummy limb block (the
/// per-limb select against the dummy encoding put it there — see
/// `shuffle_with_payloads`), so its decode is the all-unbound row. We CHECK that
/// rather than assume it: the tag and the payload must agree on what the slot is,
/// and a disagreement is a refusal, not a quietly-emitted `Dummy`.
///
/// Shared by the all-party path and by the recipient's own reconstruction
/// ([`RecipientOutputShares::reconstruct`]), so both apply the same refusal.
fn narrow_decoded_slots(
    decoded: Vec<DecodedSlot>,
    payload_arity: usize,
) -> Result<Vec<OutputSlot>, MpcError> {
    let dummy_row: PayloadRow = vec![None; payload_arity];
    let mut out: Vec<OutputSlot> = Vec::with_capacity(decoded.len());
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
    Ok(out)
}

/// One opened slot of the output stage before it is narrowed to an
/// [`OutputSlot`]: whether its tag was the real-row sentinel, and the payload row
/// its OPENED limbs decoded to. Every slot's payload is decoded, including the
/// dummy-tagged ones whose row [`OutputSlot::Dummy`] drops.
type DecodedSlot = (bool, PayloadRow);

/// What [`output_stage_shuffled_through`] hands back: the shuffled tag sharings,
/// per slot its payload-limb sharings, the PUBLIC dummy limb block each slot was
/// selected against, and the modelled cost.
type ShuffledStage = (
    SecretColumn,
    Vec<Vec<Vec<Share>>>,
    Vec<Fp>,
    ObliviousOutputCost,
);

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
    widths: impl Into<PayloadWidths>,
    bound: usize,
) -> Result<(Vec<DecodedSlot>, ObliviousOutputCost), MpcError> {
    let widths: PayloadWidths = widths.into();
    let (tags, limb_blocks, dummy_block, cost) =
        output_stage_shuffled_through(source, backend, candidates, widths, bound)?;
    let decoded = open_and_decode_slots(
        &tags,
        &limb_blocks,
        &dummy_block,
        widths.arity(),
        backend.threshold(),
    )?;
    Ok((decoded, cost))
}

/// Open the `B` slots' sharings and decode them — the stage's step 5, as its own
/// function because TWO parties may run it: every compute party on the all-party
/// path, or the designated recipient alone on the recipient-only path
/// ([`RecipientOutputShares::reconstruct`]). Whoever runs it applies the same
/// classification, the same exact dummy-block comparison and the same strict
/// decode.
fn open_and_decode_slots(
    tags: &[Vec<Share>],
    limb_blocks: &[Vec<Vec<Share>>],
    dummy_block: &[Fp],
    payload_arity: usize,
    t: usize,
) -> Result<Vec<DecodedSlot>, MpcError> {
    let bound = tags.len();

    // --- Step 5a: open ALL `B` tags at degree `t` and classify every slot, before
    // a single payload limb is opened. ---
    // FAIL CLOSED on an unexpected tag (Copilot review #93, thread 3): the only
    // legitimate opened values are `REAL_TAG` (a match) and `0` (a non-match /
    // dummy). Any other value means a malformed selector (a `MatchBit` that was not
    // a sharing of exactly 0/1) or share corruption that slipped past the robust
    // open's detection budget — silently treating it as a dummy could drop a real
    // match without a word.
    //
    // Classifying the WHOLE column first is deliberate: if any slot's tag is out of
    // set the call refuses here, so no payload limb of any slot is opened at all.
    // Opening the limbs slot by slot as the tags were checked would have opened the
    // limbs of every slot ahead of the bad one before refusing.
    let mut classes: Vec<bool> = Vec::with_capacity(bound);
    for tag_share in tags {
        let tag = open_tag(tag_share, t)?;
        if tag == Fp::one() {
            classes.push(true);
        } else if tag == Fp::zero() {
            classes.push(false);
        } else {
            return Err(MpcError::Tampered {
                // The opened tag is a reconstructed quantity, so the refusal
                // names the two values it was allowed to be — both public
                // constants — and not the one it actually was.
                detail: format!(
                    "oblivious_set_output: an opened slot tag is neither REAL_TAG ({}) nor 0 — \
                     malformed selector or corrupted share; refusing rather than dropping a match",
                    Fp::one().value()
                ),
                cheaters: Vec::new(),
            });
        }
    }

    // --- Step 5b: open the payload limbs and decode. ---
    // A slot whose tag is the dummy sentinel must carry the dummy limb block
    // EXACTLY — the per-limb select against the dummy encoding put it there. The
    // check is on the opened LIMB VECTOR, not on the decoded row: several ways of
    // selecting the wrong subset of limbs still decode to the all-unbound row (a
    // wrong length prefix with the right data limbs, for instance), so comparing
    // decoded rows would accept them. A mismatch is a `Tampered` refusal rather
    // than a quietly-emitted `Dummy`.
    let mut out: Vec<DecodedSlot> = Vec::with_capacity(bound);
    for (&is_real, limbs) in classes.iter().zip(limb_blocks) {
        let values = open_limb_block(limbs, t)?;
        if !is_real && values != dummy_block {
            return Err(MpcError::Tampered {
                detail: "oblivious_set_output: a slot tagged as a dummy did not carry the dummy \
                         limb block — the per-limb select against the dummy encoding did not \
                         hold; refusing rather than emitting the slot"
                    .to_string(),
                cheaters: Vec::new(),
            });
        }
        let bytes = limbs_to_bytes(&values)?;
        out.push((is_real, decode_payload_row(&bytes, payload_arity)?));
    }
    Ok(out)
}

/// Open one slot's TAG at degree `t`.
///
/// Its own function so the test-only tally below can observe every tag open and a
/// test can state that a path performed NONE — which is the whole point of the
/// recipient-only path.
fn open_tag(tag: &[Share], t: usize) -> Result<Fp, MpcError> {
    #[cfg(test)]
    tag_opens::bump();
    shamir::reconstruct_degree(tag, t)
}

/// A test-only tally of slot-TAG opens, the same shape as [`limb_opens`]: the
/// counter is `#[cfg(test)]` and write-only from [`open_tag`], so it observes the
/// open and changes nothing about it.
#[cfg(test)]
pub(crate) mod tag_opens {
    use std::cell::Cell;

    thread_local! {
        static OPENS: Cell<usize> = const { Cell::new(0) };
    }

    /// Record one tag open. Called by [`super::open_tag`].
    pub(crate) fn bump() {
        OPENS.with(|c| c.set(c.get() + 1));
    }

    /// Clear the tally for this thread.
    pub(crate) fn reset() {
        OPENS.with(|c| c.set(0));
    }

    /// The tally for this thread.
    pub(crate) fn count() -> usize {
        OPENS.with(|c| c.get())
    }
}

/// Open one slot's payload-limb block: every limb at degree `t`, since every
/// selected element is degree-reduced before the shuffle.
fn open_limb_block(limbs: &[Vec<Share>], t: usize) -> Result<Vec<Fp>, MpcError> {
    limbs
        .iter()
        .map(|limb| {
            #[cfg(test)]
            limb_opens::bump();
            shamir::reconstruct_degree(limb, t)
        })
        .collect()
}

/// A test-only tally of PAYLOAD-LIMB opens, so a test can assert that a refusal
/// happened before any limb was opened. Same shape as `shamir::dealer_mints`: the
/// counter is `#[cfg(test)]` and write-only from [`open_limb_block`], so it
/// observes the open and changes nothing about it. Each `#[test]` runs on its own
/// thread, so a test owns its own tally; `reset()` then `count()` brackets a run.
#[cfg(test)]
pub(crate) mod limb_opens {
    use std::cell::Cell;

    thread_local! {
        static OPENS: Cell<usize> = const { Cell::new(0) };
    }

    /// Record one payload-limb open. Called by [`super::open_limb_block`].
    pub(crate) fn bump() {
        OPENS.with(|c| c.set(c.get() + 1));
    }

    /// Zero this thread's tally, so the next [`count`] covers only what follows.
    pub(crate) fn reset() {
        OPENS.with(|c| c.set(0));
    }

    /// Payload-limb opens recorded on this thread since the last [`reset`].
    pub(crate) fn count() -> usize {
        OPENS.with(Cell::get)
    }
}

/// Steps 1–4 of the output stage: resolve the selectors, select the tag and every
/// payload limb against their dummy values, DEGREE-REDUCE every selected element,
/// and oblivious-shuffle the packed entries. Returns the shuffled tag sharings and,
/// per slot, its payload-limb sharings — all degree `t`, none yet opened.
///
/// Split out so the share VECTORS the stage is about to open are reachable without
/// opening them, which is what the share-level tests inspect.
///
/// The returned dummy limb block is PUBLIC — it depends only on the payload arity
/// and the uniform packed width — and is what the caller compares a dummy-tagged
/// slot's opened limbs against, limb for limb.
fn output_stage_shuffled_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    widths: PayloadWidths,
    bound: usize,
) -> Result<ShuffledStage, MpcError> {
    let payload_arity = widths.arity();
    // FIRST, before anything is drawn or allocated: the bound is validated. Every
    // `Vec::with_capacity(bound)` below and the shuffle network are sized from it,
    // so the cap check precedes even the source check (see the contract above).
    check_output_bound(bound, candidates.len())?;
    // Then the DECLARATION's own shape — the arity cap, the per-term declaration
    // cap and the uniform limb width that follows from them — also closed form and
    // also ahead of the source check: the all-unbound dummy row of this arity is
    // materialised below, so an `usize::MAX` arity must refuse before anything
    // tries to allocate it.
    widths.check_shape()?;
    // And the party count, which multiplies the per-limb work twice over.
    check_output_parties_cap(backend.parties())?;
    // A source whose (n, t) disagrees with the backend would deal sharings that do
    // not compose with the backend's products and opens.
    require_source_matches(source, backend)?;
    // Validate payload arity up front (the recipient relies on a uniform schema).
    for (k, c) in candidates.iter().enumerate() {
        if c.payload.len() != payload_arity {
            return Err(MpcError::Protocol(format!(
                "oblivious_set_output: candidate {k} has {} payload terms, expected arity {payload_arity}",
                c.payload.len()
            )));
        }
    }

    // Decide the PUBLIC packing plan — the uniform limb width from the DECLARED
    // widths, the public dummy block and the aggregate work budget — and then
    // measure the candidates' terms against the declaration, all BEFORE the first
    // sharing is drawn below. The width was previously the maximum over the
    // candidates' own encodings, decided inside `shuffle_with_payloads` after the
    // per-candidate selector, real-tag and degree-reduction draws; it is now a
    // closed form of public numbers, settled here, and an over-width run refuses
    // with zero draws.
    let plan = SlotPlan::decide(candidates, widths, bound, backend.parties(), backend.threshold())?;

    let (n, t) = (backend.parties(), backend.threshold());
    // The select multiplication AND the degree reduction that re-randomises it both
    // need honest-majority headroom n >= 2t+1.
    if n < 2 * t + 1 {
        return Err(MpcError::Protocol(format!(
            "oblivious_set_output needs n >= 2t+1 for the select multiplication and its \
             degree reduction (n={}, t={})",
            n, t
        )));
    }

    // The real-row sentinel tag. A matched slot's tag opens to REAL_TAG; a
    // non-match / dummy opens to Fp::zero(). REAL_TAG != 0 so the two are
    // distinguishable AFTER the shuffle (and only after — position is destroyed).
    let real_tag = Fp::one();

    // --- Step 1–3: build per-slot (tag-sharing, payload) selecting on the bit. ---
    // `tag_out = sel · real_tag` is a degree-`2t` product, which is then
    // DEGREE-REDUCED to a fresh degree-`t` sharing of the same value before it goes
    // anywhere. The selector is kept alongside so the SAME bit can select the
    // payload limbs against the dummy encoding in step 3's payload half
    // (`shuffle_with_payloads`); the payload's own bytes are already in `plan`.
    let mut tag_col: SecretColumn = Vec::with_capacity(bound);
    let mut selectors: Vec<Vec<Share>> = Vec::with_capacity(bound);
    let mut degree_reductions = 0usize;

    for c in candidates {
        let sel = selector_sharing(source, n, &c.matched)?;
        // tag_out = sel * real_tag  (degree 2t). sel=1 → REAL_TAG; sel=0 → 0.
        let real_tag_share = checked_vss_own_input(source, n, real_tag)?;
        let tag_out = shamir::mul_shares_raw(&sel, &real_tag_share)?;
        tag_col.push(shamir::degree_reduce_through(source, &tag_out)?);
        degree_reductions += 1;
        selectors.push(sel);
    }
    // Pure-dummy padding slots: selector = shared 0 → tag opens to 0 → Dummy, and
    // the same shared 0 selects the payload limbs to the dummy encoding.
    for _ in candidates.len()..bound {
        let zero_sel = checked_vss_own_input(source, n, Fp::zero())?;
        let real_tag_share = checked_vss_own_input(source, n, real_tag)?;
        let tag_out = shamir::mul_shares_raw(&zero_sel, &real_tag_share)?;
        tag_col.push(shamir::degree_reduce_through(source, &tag_out)?);
        degree_reductions += 1;
        selectors.push(zero_sel);
    }

    // --- Step 4: oblivious-shuffle the tag column AND co-permute the payloads. ---
    // Payloads ride THROUGH the shuffle as secret shares (the permutation is never
    // opened) — see `shuffle_with_payloads` (Copilot review #93, thread 1), which
    // also applies the per-limb select against the dummy encoding and reduces every
    // limb to degree `t`.
    let (
        shuffled_tags,
        shuffled_limbs,
        dummy_block,
        shuffle_cost,
        payload_select_mults,
        limb_reductions,
    ) = shuffle_with_payloads(source, n, tag_col, &plan, &selectors)?;
    degree_reductions += limb_reductions;

    let cost = ObliviousOutputCost {
        bound,
        select_mults: bound,
        payload_select_mults,
        degree_reductions,
        opens: bound,
        shuffle: shuffle_cost,
    };
    Ok((shuffled_tags, shuffled_limbs, dummy_block, cost))
}

/// One slot's share VECTORS as the stage is about to open them: the tag sharing and
/// its `limb_capacity` payload-limb sharings, in shuffled order.
#[cfg(test)]
pub(crate) type SlotShares = (Vec<Share>, Vec<Vec<Share>>);

/// Test-only instrumentation: the `B` slots' SHARE VECTORS, stopping one step short
/// of the opens.
///
/// This is [`output_stage_shuffled_through`] — the real steps 1–4, selectors,
/// selects, degree reductions and shuffle — handing back exactly the sharings the
/// public path then opens, plus the PUBLIC dummy limb block they were selected
/// against. The share-level tests inspect these to check what a dummy-tagged slot's
/// sharings reveal, which no opened VALUE can show.
#[cfg(test)]
pub(crate) fn opened_slot_shares_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    widths: impl Into<PayloadWidths>,
    bound: usize,
) -> Result<(Vec<SlotShares>, Vec<Fp>), MpcError> {
    let widths: PayloadWidths = widths.into();
    let (tags, limbs, dummy_block, _cost) =
        output_stage_shuffled_through(source, backend, candidates, widths, bound)?;
    Ok((tags.into_iter().zip(limbs).collect(), dummy_block))
}

/// Test-only instrumentation on the real output stage: the `B` opened slots as
/// `(is_real, decoded_row)`, keeping the payload row that every slot's OPENED
/// limbs decoded to — including the dummy-tagged slots, whose row the public
/// [`OutputSlot::Dummy`] drops.
///
/// It runs the real stage ([`output_stage_shuffled_through`]) and then the real
/// opens and decode ([`shamir::reconstruct_degree`] at degree `t`, `limbs_to_bytes`,
/// `decode_payload_row`) on its sharings. It deliberately OMITS one step the public
/// path takes: the refusal of a dummy-tagged slot that does not carry the dummy
/// encoding. Omitting it is the point — it is what lets a test SEE what a
/// non-matching candidate's payload opened to instead of only seeing the refusal.
/// That refusal has its own tests.
#[cfg(test)]
pub(crate) fn opened_slot_rows_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    candidates: &[Candidate],
    widths: impl Into<PayloadWidths>,
    bound: usize,
) -> Result<Vec<DecodedSlot>, MpcError> {
    let widths: PayloadWidths = widths.into();
    let t = backend.threshold();
    let (slots, _dummy_block) =
        opened_slot_shares_through(source, backend, candidates, widths, bound)?;
    let mut out = Vec::with_capacity(slots.len());
    for (tag_share, limbs) in slots {
        let tag = shamir::reconstruct_degree(&tag_share, t)?;
        let values = open_limb_block(&limbs, t)?;
        let bytes = limbs_to_bytes(&values)?;
        out.push((tag == Fp::one(), decode_payload_row(&bytes, widths.arity())?));
    }
    Ok(out)
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

/// [`encode_payload_row`] with every bound term measured against the DECLARED
/// per-term width ([`PayloadWidths::term_bytes`]).
///
/// A term's serialized length is only knowable by serializing it, so each bound
/// column is serialized once and measured immediately: an over-width row refuses
/// having serialized exactly one over-width term and nothing else. The declared
/// width is the same public number on every party, so every party refuses the same
/// row at the same point.
///
/// The refusal carries PUBLIC values only — the declared width and the declared
/// arity. It names no column, no candidate position, no term text and no length:
/// which column of which row was too wide is a property of the data, and a
/// refusal is exactly where it would otherwise be reported.
fn encode_payload_row_checked(
    row: &[Option<Term>],
    widths: &PayloadWidths,
) -> Result<Vec<u8>, MpcError> {
    let mut bytes = Vec::new();
    for cell in row {
        match cell {
            None => bytes.push(0u8),
            Some(term) => {
                let text = term.to_string();
                if text.len() > widths.term_bytes() {
                    return Err(over_declared_width(widths));
                }
                bytes.push(1u8);
                bytes.extend_from_slice(text.as_bytes());
                bytes.push(0u8); // NUL terminator (never occurs inside the term)
            }
        }
    }
    Ok(bytes)
}

/// The one refusal for a term that does not fit the declared width. Public values
/// The output stage's PUBLIC-SHAPE checks, in the order they must run: the
/// declaration's own shape (arity cap, per-term declaration cap, uniform limb
/// cap), the party cap, then the aggregate work budget.
///
/// None of them reads a candidate, so an operator can run them at its OWN entry,
/// before it builds candidates and draws randomness, and get the same answer the
/// output stage would give. `SlotPlan::decide` runs them again at the stage entry;
/// they are closed forms on public numbers, so running them twice costs nothing
/// and the stage stays correct for a caller that did not preflight.
pub(crate) fn check_output_public_shape(
    widths: &PayloadWidths,
    bound: usize,
    parties: usize,
    threshold: usize,
) -> Result<(), MpcError> {
    widths.check_shape()?;
    check_output_parties_cap(parties)?;
    // With no slot at all nothing is packed and the width is 0.
    let limb_capacity = if bound == 0 {
        0
    } else {
        widths.limb_capacity().unwrap_or(0)
    };
    check_slot_work_budget(bound, limb_capacity, parties, threshold)
}

/// only; see [`encode_payload_row_checked`].
fn over_declared_width(widths: &PayloadWidths) -> MpcError {
    MpcError::Protocol(format!(
        "oblivious output stage: a payload term does not fit the declared per-term width \
         of {} bytes (declared arity {}) — refusing before any sharing or draw. The \
         declared width is what every slot is padded to, so a term that does not fit \
         cannot be carried; declare a wider schema (up to the MAX_PAYLOAD_TERM_BYTES cap \
         of {}) if it should be",
        widths.term_bytes(),
        widths.arity(),
        MAX_PAYLOAD_TERM_BYTES
    ))
}

/// Measure every candidate's bound terms against the declared per-term width,
/// BEFORE the stage draws anything.
///
/// Serializes one term at a time and drops it, so the pass costs one term's bytes
/// at a time whatever the candidate set holds — it does not build or retain an
/// encoding per candidate. The encodings are built again, one slot at a time, in
/// the packing loop; serializing twice is the price of not holding the whole
/// candidate set encoded at once.
fn check_candidate_term_widths(
    candidates: &[Candidate],
    widths: &PayloadWidths,
) -> Result<(), MpcError> {
    for c in candidates {
        for cell in c.payload.iter().flatten() {
            if cell.to_string().len() > widths.term_bytes() {
                return Err(over_declared_width(widths));
            }
        }
    }
    Ok(())
}

/// The PUBLIC packing plan for one output-stage run: the declared widths, the
/// public dummy limb block and the slot count. Every number in it comes from the
/// caller's declaration and the backend's party count, so it is decided before a
/// candidate is read and is identical on every party.
///
/// It holds no candidate encoding. Each slot's bytes are produced on demand in the
/// packing loop ([`Self::slot_limbs`]), so the stage's peak is one slot's encoding
/// and not one per candidate.
struct SlotPlan<'a> {
    /// The candidates, borrowed — slot `i` below the candidate count is this
    /// candidate's row, and any slot above it is a pure-dummy padding slot.
    candidates: &'a [Candidate],
    /// The caller's declaration.
    widths: PayloadWidths,
    /// The all-unbound row of the declared arity, encoded once — the byte stream
    /// every pure-dummy padding slot carries, and the row the public pad block is
    /// built from.
    dummy_encoded: Vec<u8>,
    /// The public dummy limb block, padded out to [`Self::limb_capacity`], that
    /// every slot's limbs are selected against.
    dummy_block: Vec<Fp>,
    /// The slot count — the padded bound `B`.
    slots: usize,
}

impl<'a> SlotPlan<'a> {
    /// The uniform packed limb width — the declared width, or `0` when there is no
    /// slot at all.
    fn limb_capacity(&self) -> usize {
        self.dummy_block.len()
    }

    /// Slot `i`'s limbs, encoded on demand: its candidate's row, or the all-unbound
    /// dummy row for a pure-dummy padding slot. Shorter than the uniform width; the
    /// packing loop pads it.
    fn slot_limbs(&self, i: usize) -> Result<Vec<Fp>, MpcError> {
        match self.candidates.get(i) {
            Some(c) => Ok(bytes_to_limbs(&encode_payload_row_checked(
                &c.payload,
                &self.widths,
            )?)),
            None => Ok(bytes_to_limbs(&self.dummy_encoded)),
        }
    }

    /// Decide the plan from PUBLIC numbers only, in this order: the declaration's
    /// own shape (arity cap, per-term declaration cap, uniform limb cap), the party
    /// cap, and the aggregate work budget — none of which reads a candidate. Then,
    /// and still before any sharing or draw, every candidate's terms are measured
    /// against the declared width.
    ///
    /// `bound` and the per-candidate arity agreement are the caller's checks and
    /// have already run.
    fn decide(
        candidates: &'a [Candidate],
        widths: PayloadWidths,
        bound: usize,
        parties: usize,
        threshold: usize,
    ) -> Result<Self, MpcError> {
        check_output_public_shape(&widths, bound, parties, threshold)?;
        // With no slot at all nothing is packed and the width is 0.
        let limb_capacity = if bound == 0 {
            0
        } else {
            // `check_shape` has already refused an overflowing declaration.
            widths.limb_capacity().unwrap_or(0)
        };

        // Only now is anything candidate-shaped touched, and only to measure it.
        check_candidate_term_widths(candidates, &widths)?;

        // The all-unbound row of the declared arity: the public dummy encoding.
        let dummy_encoded = encode_payload_row(&vec![None; widths.arity()]);
        let mut dummy_block = bytes_to_limbs(&dummy_encoded);
        dummy_block.resize(limb_capacity, Fp::new(PAD_LIMB));
        Ok(SlotPlan {
            candidates,
            widths,
            dummy_encoded,
            dummy_block,
            slots: bound,
        })
    }
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
                // The decoded bytes are a joined row's PAYLOAD. Neither the text
                // nor the underlying parse error (which quotes it) may appear in
                // the refusal — the column position is the diagnosis.
                let s = std::str::from_utf8(&bytes[pos..end]).map_err(|_| {
                    MpcError::Protocol(format!(
                        "decode_payload_row: column {} term is not valid UTF-8",
                        col
                    ))
                })?;
                let term = Term::from_str(s).map_err(|_| {
                    MpcError::Protocol(format!(
                        "decode_payload_row: column {} term did not re-parse",
                        col
                    ))
                })?;
                out.push(Some(term));
                pos = end + 1; // skip the NUL
            }
            _ => {
                // The flag byte came out of an opened limb block, so it is not
                // named. The column index is not public either: which column fails
                // first is a function of the opened block, and reporting it says
                // the earlier columns decoded. The error class plus that boundary
                // is the whole diagnosis.
                return Err(MpcError::Protocol(format!(
                    "decode_payload_row: column {} has an invalid presence flag (corrupt entry)",
                    col
                )));
            }
        }
    }
    // STRICT: the stream must be exactly the `arity` columns and nothing more.
    // Trailing bytes mean the stream is not one `encode_payload_row` produced, so two
    // different streams could decode to the same row; refuse instead.
    if pos != bytes.len() {
        // How MANY bytes trail is a function of the opened block, so it is not
        // named. The column arity IS public — it is the payload's declared width,
        // the same for every slot — and the fact that anything trails it is the
        // whole diagnosis.
        return Err(MpcError::Protocol(format!(
            "decode_payload_row: trailing byte(s) after {} column(s) (corrupt entry)",
            arity
        )));
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
        // The prefix itself is an opened limb, so it is not named. The capacity it
        // overran is the block's uniform width — public, a function of the slot
        // layout rather than of this block's contents — so that is what the message
        // names.
        return Err(MpcError::Protocol(format!(
            "limbs_to_bytes: the length prefix exceeds the block's {} available byte(s) \
             (corrupt shuffle entry)",
            available
        )));
    }
    let mut bytes = Vec::with_capacity(byte_len);
    for (index, limb) in limbs[1..].iter().enumerate() {
        let v = limb.value();
        if v >= (1u64 << (8 * BYTES_PER_LIMB)) {
            // `v` is an opened limb, so it is not named. The index is NOT public
            // either: which limb fails first is a function of the opened block, and
            // saying so reports that the earlier limbs were in range and this one
            // was not. `BYTES_PER_LIMB` is a compile-time constant.
            return Err(MpcError::Protocol(format!(
                "limbs_to_bytes: data limb at index {} exceeds the {}-byte range (corrupt \
                 shuffle entry)",
                index, BYTES_PER_LIMB
            )));
        }
        for k in 0..BYTES_PER_LIMB {
            bytes.push(((v >> (8 * k)) & 0xff) as u8);
        }
    }
    // STRICT: every byte past the length prefix must be the pad byte. `bytes_to_limbs`
    // zero-fills the final partial limb and pads the rest of the uniform width with
    // `PAD_LIMB`, so anything else there is content the length prefix does not
    // account for — a limb block that is not one this encoder could have produced.
    // Accepting it would let two different limb blocks decode to the same row.
    if bytes[byte_len..].iter().any(|&b| b != PAD_BYTE) {
        // Both the count of offending bytes and the prefix they sit past are
        // functions of the opened block; neither is named.
        return Err(MpcError::Protocol(
            "limbs_to_bytes: byte(s) past the length prefix are not the pad byte (corrupt \
             shuffle entry)"
                .to_string(),
        ));
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
/// Both the tag and every limb leave the select as degree-`2t` products, and each
/// is re-randomised with [`shamir::degree_reduce_through`] before it is packed, so
/// the shuffled entries are degree `t` and the limb opens are at degree `t`. The
/// select multiplication itself still requires `n >= 2t + 1`.
fn shuffle_with_payloads(
    source: &mut dyn DistributedRandomness,
    nparties: usize,
    tag_col: SecretColumn,
    plan: &SlotPlan<'_>,
    selectors: &[Vec<Share>],
) -> Result<ShuffledColumn, MpcError> {
    let n = tag_col.len();
    debug_assert_eq!(plan.slots, n);
    debug_assert_eq!(selectors.len(), n);

    // The UNIFORM limb capacity and the public dummy block come from the plan the
    // caller decided and capped before any sharing was drawn (`SlotPlan::decide`).
    // The width is a closed form of the caller's DECLARED arity and per-term width,
    // so it is the same number on every party and on every run of the same plan,
    // and no candidate was read to arrive at it. It remains public to the parties
    // and visible in the cost model, which the module docs state.
    let limb_capacity = plan.limb_capacity();
    let dummy_block = plan.dummy_block.clone();

    // Pack each entry: [tag shares | limb shares...]. Each limb is shared at
    // degree `t`, then selected against its dummy value — a single multiplication
    // by the slot's selector, so the product is degree `2t` — and then
    // degree-reduced back to a fresh degree-`t` sharing, so every PACKED element is
    // degree `t`. Short rows
    // are padded to `limb_capacity` with shared PAD_LIMB before the select, so all
    // entries are identical width (the explicit length prefix makes the padding
    // content of a REAL row irrelevant to its decode).
    let mut packed: SecretColumn = Vec::with_capacity(n);
    let mut payload_select_mults = 0usize;
    let mut limb_reductions = 0usize;
    for ((slot, tag), sel) in tag_col.into_iter().enumerate().zip(selectors) {
        let mut entry = tag; // tag shares first (already degree-reduced)
        // Encoded on demand, one slot at a time: the plan holds no per-candidate
        // encoding, so the peak is one slot's bytes.
        let limbs = plan.slot_limbs(slot)?;
        // `dummy_block` is exactly `limb_capacity` long, so iterating it walks the
        // uniform width and supplies each limb's public pad value.
        for (i, &pad) in dummy_block.iter().enumerate() {
            // Draw order is unchanged: the row's own limbs first, then PAD_LIMB up
            // to the uniform width.
            let value = limbs.get(i).copied().unwrap_or(Fp::new(PAD_LIMB));
            let limb_share = checked_vss_own_input(source, nparties, value)?;
            // limb_out = pad + sel·(limb − pad): local shift, one multiplication,
            // local shift back. The product is degree `2t`, like the tag's.
            let shifted = shamir::add_constant(&limb_share, pad.neg());
            let selected = shamir::mul_shares_raw(sel, &shifted)?;
            payload_select_mults += 1;
            // Re-randomise before the public constant goes back on: the reduction
            // replaces the product sharing with a FRESH degree-`t` sharing of the
            // same value, independent of the factor polynomials. Adding `pad` after
            // the reduction is local and keeps the element degree `t`.
            let reduced = shamir::degree_reduce_through(source, &selected)?;
            limb_reductions += 1;
            entry.extend(shamir::add_constant(&reduced, pad));
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
        // The shuffle already counted one fresh sharing per packed ELEMENT, so
        // this is its own count, not a widened one.
        fresh_sharings: base_cost.fresh_sharings,
    };

    // Split each shuffled entry into (tag shares, limb shares...) and OPEN the
    // limbs — in shuffled order, never opening the permutation. Decoding is left to
    // the caller, which classifies the slot by its tag first (so a slot whose tag
    // is out of `{0, REAL_TAG}` is refused before any byte is parsed). Every entry
    // has the same width = nparties·(1 + limb_capacity).
    let expect_width = nparties * (1 + limb_capacity);
    let mut shuffled_tags: SecretColumn = Vec::with_capacity(n);
    let mut shuffled_limbs: Vec<Vec<Vec<Share>>> = Vec::with_capacity(n);
    for entry in shuffled {
        if entry.len() != expect_width {
            return Err(MpcError::Protocol(format!(
                "shuffle_with_payloads: packed entry width {} != n_parties*(1+limbs) {expect_width}",
                entry.len(),
            )));
        }
        let (tag_part, limb_part) = entry.split_at(nparties);
        shuffled_tags.push(tag_part.to_vec());

        // Split out each limb's sharing IN SHUFFLED ORDER. Nothing is opened here —
        // the caller opens tag and limbs together, at degree `t`, once it knows what
        // the slot is.
        let mut limbs: Vec<Vec<Share>> = Vec::with_capacity(limb_capacity);
        for limb_shares in limb_part.chunks(nparties) {
            limbs.push(limb_shares.to_vec());
        }
        shuffled_limbs.push(limbs);
    }

    Ok((
        shuffled_tags,
        shuffled_limbs,
        dummy_block,
        cost,
        payload_select_mults,
        limb_reductions,
    ))
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
    // Preflight the bound before the dealer exists; the stage re-checks it first.
    check_output_bound(bound, candidates.len())?;
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
        oblivious_set_output_through(source, backend, candidates, PayloadWidths::from(payload_arity), bound)?;
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
    // Preflight the bound and the all-pairs cardinality BEFORE a dealer exists —
    // minting one is cheap but it is still keystream state created for a run that
    // is about to be refused.
    check_candidate_pairs_within_bound(bound, left_keys.len(), right_keys.len())?;
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
    // Up front, before anything is drawn OR allocated: `B` within the cap, the
    // all-pairs cardinality computed with checked arithmetic, and `B` covering it.
    // The equality chain below draws once per pair and the candidate vector is
    // sized from the product, so neither may be reached on an over-cap or
    // overflowing input.
    let pairs = check_candidate_pairs_within_bound(bound, left_keys.len(), right_keys.len())?;
    require_source_matches(source, backend)?;
    // All-pairs candidates: each (i, j) becomes one slot whose match bit is the
    // SECRET-SHARED equality of the two private keys — never opened.
    let mut candidates: Vec<Candidate> = Vec::with_capacity(pairs);
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
    oblivious_set_output_through(source, backend, &candidates, PayloadWidths::from(1), bound)
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

    // ---- Recipient-only output reconstruction -----------------------------

    /// The fixture for the recipient-only tests: two matches and two non-matches
    /// at a bound that also adds padding slots.
    fn recipient_candidates() -> Vec<Candidate> {
        public_candidates(&[
            (true, vec![lit("kept-a")]),
            (false, vec![lit("rejected-b")]),
            (true, vec![lit("kept-c")]),
            (false, vec![lit("rejected-d")]),
        ])
    }

    /// **THE recipient-only guard.** The compute parties open NOTHING: running the
    /// recipient-only path performs zero tag opens and zero payload-limb opens, so
    /// no opened tag value — and therefore no match count — is in their view. The
    /// `B` opens happen only when the RECIPIENT reconstructs.
    ///
    /// The all-party path is measured in the same test with the same counters, so
    /// the comparison is between two real runs, not against a remembered number.
    #[test]
    fn the_compute_parties_open_nothing_on_the_recipient_only_path() {
        let backend = ShamirBackend::new_seeded(5, 0x8E01).unwrap();
        let cands = recipient_candidates();

        // The all-party path: every compute party opens every slot.
        tag_opens::reset();
        limb_opens::reset();
        let (all_party, all_cost) = oblivious_set_output(&backend, &cands, 1, 6).unwrap();
        let (all_tags, all_limbs) = (tag_opens::count(), limb_opens::count());
        assert_eq!(all_tags, 6, "the all-party path opens one tag per slot");
        assert!(all_limbs > 0, "the all-party path opens the payload limbs");
        assert_eq!(all_cost.opens, 6);

        // The recipient-only path: the compute phase opens nothing at all.
        tag_opens::reset();
        limb_opens::reset();
        let (shares, cost) =
            oblivious_set_output_to_recipient(&backend, &cands, 1, 6, HolderId::new("recipient"))
                .unwrap();
        assert_eq!(
            (tag_opens::count(), limb_opens::count()),
            (0, 0),
            "the compute parties opened something on the recipient-only path"
        );
        assert_eq!(
            cost.opens, 0,
            "the reported opens must be the compute parties', and they made none"
        );
        assert_eq!(shares.bound(), 6);
        assert_eq!(shares.payload_arity(), 1);
        assert_eq!(shares.recipient(), &HolderId::new("recipient"));
        // Every other modelled cost is the all-party path's: the protocol up to
        // and including the shuffle is the same.
        assert_eq!(
            (
                cost.bound,
                cost.select_mults,
                cost.payload_select_mults,
                cost.degree_reductions,
                cost.shuffle
            ),
            (
                all_cost.bound,
                all_cost.select_mults,
                all_cost.payload_select_mults,
                all_cost.degree_reductions,
                all_cost.shuffle
            )
        );

        // Now the RECIPIENT reconstructs, alone — and that is where the opens are.
        let slots = shares.reconstruct().unwrap();
        assert_eq!(
            tag_opens::count(),
            6,
            "the recipient opens one tag per slot"
        );
        assert_eq!(limb_opens::count(), all_limbs);
        // The same result the all-party path reveals: two real rows, the rest dummy.
        assert_eq!(slots.len(), 6);
        assert_eq!(
            real_multiset(&slots),
            vec![
                format!("{:?}", vec![lit("kept-a")]),
                format!("{:?}", vec![lit("kept-c")])
            ]
        );
        assert_eq!(real_multiset(&slots), real_multiset(&all_party));
    }

    /// The shares handed to the recipient are not the match count in disguise: the
    /// struct carries exactly `B` slots whatever the true count is, and the compute
    /// parties' cost report is identical for two candidate sets with DIFFERENT true
    /// counts.
    #[test]
    fn the_recipient_shares_carry_b_slots_whatever_the_true_count() {
        let backend = ShamirBackend::new_seeded(5, 0x8E02).unwrap();
        let one = public_candidates(&[
            (true, vec![lit("a")]),
            (false, vec![lit("b")]),
            (false, vec![lit("c")]),
        ]);
        let three = public_candidates(&[
            (true, vec![lit("a")]),
            (true, vec![lit("b")]),
            (true, vec![lit("c")]),
        ]);
        let r = |c: &[Candidate]| {
            oblivious_set_output_to_recipient(&backend, c, 1, 5, HolderId::new("recipient"))
                .unwrap()
        };
        let (s1, c1) = r(&one);
        let (s3, c3) = r(&three);
        assert_eq!((s1.bound(), s3.bound()), (5, 5));
        assert_eq!(c1, c3, "the compute parties' cost must not track the count");
        // And the recipient, who may reconstruct, does see the difference.
        assert_eq!(real_multiset(&s1.reconstruct().unwrap()).len(), 1);
        assert_eq!(real_multiset(&s3.reconstruct().unwrap()).len(), 3);
    }

    /// The recipient-only entry carries the SAME bound guard as the all-party one,
    /// and it refuses before any draw or allocation: `B` sizes every per-slot
    /// vector and the shuffle network, so a `usize::MAX` or over-cap `B` must not
    /// reach them. Covered on both the `_through` entry (where the count is
    /// observable) and the public wrapper.
    #[test]
    fn the_recipient_entry_refuses_an_over_cap_bound_before_any_draw() {
        use crate::randomness::test_sources::DrawCountingSource;
        let backend = ShamirBackend::new_seeded(5, 0x8E0B).unwrap();
        let cands = recipient_candidates();
        assert!(
            !cands.is_empty(),
            "a non-empty input, so draws are possible"
        );

        for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
            let mut dealer = backend.dealer();
            let mut counted = DrawCountingSource::new(&mut dealer);
            let err = oblivious_set_output_to_recipient_through(
                &mut counted,
                &backend,
                &cands,
                1,
                bound,
                HolderId::new("recipient"),
            )
            .expect_err("an over-cap bound must be refused");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                "B={}: {:?}",
                bound,
                err
            );
            assert_eq!(counted.draws(), 0, "B={}: refused before any draw", bound);

            let err = oblivious_set_output_to_recipient(
                &backend,
                &cands,
                1,
                bound,
                HolderId::new("recipient"),
            )
            .expect_err("the public wrapper must refuse it too");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                "wrapper B={}: {:?}",
                bound,
                err
            );
        }

        // A short `B` is the other half of the contract: never truncate a match.
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = oblivious_set_output_to_recipient_through(
            &mut counted,
            &backend,
            &cands,
            1,
            cands.len() - 1,
            HolderId::new("recipient"),
        )
        .expect_err("a bound below the candidate count must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("bound")),
            "short bound: {:?}",
            err
        );
        assert_eq!(counted.draws(), 0, "the short bound precedes every draw");

        // And a legal bound over the same input still runs, so the guard is not
        // refusing everything.
        oblivious_set_output_to_recipient(
            &backend,
            &cands,
            1,
            cands.len(),
            HolderId::new("recipient"),
        )
        .expect("a legal bound runs");
    }

    /// The recipient's reconstruction is not a weaker open: it applies the same
    /// refusals. A dummy-tagged slot whose limbs were tampered with after the
    /// compute phase is refused, and the refusal comes from the recipient's own
    /// check.
    #[test]
    fn the_recipients_reconstruction_refuses_a_tampered_dummy_slot() {
        let backend = ShamirBackend::new_seeded(5, 0x8E03).unwrap();
        let cands = recipient_candidates();
        let (mut shares, _) =
            oblivious_set_output_to_recipient(&backend, &cands, 1, 6, HolderId::new("recipient"))
                .unwrap();
        shares.reconstruct().expect("the untouched shares decode");
        // Find a dummy-tagged slot and move one of its limbs off the pad block.
        let t = backend.threshold();
        let k = shares
            .slots
            .iter()
            .position(|(tag, _)| shamir::reconstruct_degree(tag, t).unwrap() == Fp::zero())
            .expect("the fixture has dummy slots");
        let bumped = shamir::add_constant(&shares.slots[k].1[0], Fp::one());
        shares.slots[k].1[0] = bumped;
        match shares.reconstruct() {
            Err(MpcError::Tampered { detail, .. }) => assert!(
                detail.contains("did not carry the dummy"),
                "wrong refusal: {}",
                detail
            ),
            other => panic!("the recipient must fail closed, got {:?}", other),
        }
    }

    /// The recipient can take the result in `PartialResult` shape, which is what a
    /// federated caller consumes — same rows as the all-party wrapper.
    #[test]
    fn the_recipient_can_reconstruct_a_partial_result() {
        let backend = ShamirBackend::new_seeded(5, 0x8E04).unwrap();
        let cands = recipient_candidates();
        let vars = vec![var("x")];
        let (public, _) = oblivious_join_output(&backend, &cands, vars.clone(), 6).unwrap();
        let (shares, _) =
            oblivious_set_output_to_recipient(&backend, &cands, 1, 6, HolderId::new("recipient"))
                .unwrap();
        let mine = shares.reconstruct_partial_result(vars).unwrap();
        assert_eq!(mine.rows, public.rows);
        assert_eq!(mine.vars, public.vars);
        assert_eq!(mine.holder, public.holder);
    }

    /// Corrupting payload-limb shares exercises the degree-`t` open's RS
    /// redundancy, which is what pins the opens to degree `t`.
    ///
    /// At `n = 5`, `t = 2`, a degree-`t` sharing has `n - t - 1 = 2` redundant
    /// shares, so Berlekamp–Welch CORRECTS one corrupted share and DETECTS two.
    /// Both halves are asserted on the real path:
    ///
    /// - ONE corrupted share: the open corrects it and the revealed slots are
    ///   byte-identical to the untouched run. That is the load-bearing half for the
    ///   degree: opened at degree `2t = 4` the sharing would have `n - 2t - 1 = 0`
    ///   redundant shares, the corrupted share would be absorbed into a DIFFERENT
    ///   value, and the slots could not come back unchanged.
    /// - TWO corrupted shares: past the correction budget, so the call refuses
    ///   rather than emitting a slot built from a value nobody shared. **This half
    ///   holds for the deltas used here, not for every pair of errors.** Two
    ///   *coordinated* errors can instead land the vector within correcting
    ///   distance of a different valid codeword, and the open then returns that
    ///   codeword's value with no refusal at all —
    ///   `two_coordinated_share_errors_can_force_a_wrong_open_at_n5` pins exactly
    ///   that window.
    #[test]
    fn corrupted_limb_shares_are_corrected_then_refused_at_n5() {
        let backend = ShamirBackend::new_seeded(5, 0x8E05).unwrap();
        assert_eq!(backend.threshold(), 2);
        let cands = recipient_candidates();
        let fresh = || {
            oblivious_set_output_to_recipient(&backend, &cands, 1, 6, HolderId::new("recipient"))
                .unwrap()
                .0
        };
        let clean = fresh().reconstruct().expect("the untouched sharings open");

        // One corrupted share: corrected, same result.
        let mut one = fresh();
        one.slots[0].1[0][2].y = one.slots[0].1[0][2].y.add(Fp::one());
        assert_eq!(
            one.reconstruct()
                .expect("one corrupted share must be corrected"),
            clean,
            "the degree-t open must correct one corrupted share to the same value"
        );

        // Two corrupted shares: past the budget, so the call must fail closed.
        let mut two = fresh();
        two.slots[0].1[0][2].y = two.slots[0].1[0][2].y.add(Fp::one());
        two.slots[0].1[0][3].y = two.slots[0].1[0][3].y.add(Fp::new(7));
        match two.reconstruct() {
            Err(MpcError::Tampered { .. }) | Err(MpcError::Protocol(_)) => {}
            other => panic!("two corrupted limb shares must be refused, got {:?}", other),
        }
    }

    /// A debug print of the recipient's shares must not put share values in a log
    /// line or a panic message: the rendering carries the PUBLIC shape only.
    #[test]
    fn the_recipient_shares_debug_prints_no_share_values() {
        let backend = ShamirBackend::new_seeded(5, 0x8E07).unwrap();
        let cands = recipient_candidates();
        let (shares, _) =
            oblivious_set_output_to_recipient(&backend, &cands, 1, 6, HolderId::new("recipient"))
                .unwrap();
        let rendered = format!("{:?}", shares);
        assert!(
            rendered.contains("recipient") && rendered.contains('6') && rendered.contains("1"),
            "the public shape must still be there: {}",
            rendered
        );
        for (tag, limbs) in &shares.slots {
            for sh in tag.iter().chain(limbs.iter().flatten()) {
                assert!(
                    !rendered.contains(&sh.y.value().to_string()),
                    "a share value reached the debug rendering: {}",
                    rendered
                );
            }
        }
    }

    /// **A documented LIMITATION, not a guarantee.** The opens decode with
    /// correction, and bounded-distance decoding has a miscorrection window: a
    /// coordinated set of errors larger than the correctable budget can place the
    /// received vector within that budget of a DIFFERENT valid codeword, which the
    /// open then returns as a wrong value with no [`MpcError::Tampered`].
    ///
    /// The concrete instance at `n = 5`, `t = 2`: two parties, at `x = 1` and
    /// `x = 2`, add the evaluations of `δ(x) = (x - 4)(x - 5)`, i.e. `+12` and
    /// `+6`. The received vector then agrees with the valid degree-`2` codeword
    /// `f + δ` at four of the five points, so it is one correction away from it and
    /// the open hands back `f(0) + δ(0) = s + 20` — a value nobody shared, flagged
    /// as nothing. Two corrupted shares out of five is within the DETECTION budget
    /// (`t = 2`), so this is the cost of correcting at all; detection of up to `t`
    /// errors is unconditional only in a detect-only decoder.
    ///
    /// Asserting `Ok(wrong)` is the point: it is what makes the module doc's
    /// qualification of the RS claim load-bearing rather than prose. If a future
    /// decoder change (detect-only, or verified opens — bead sq-6d6g) closes this
    /// window, this test goes red and is the place to record that it did.
    #[test]
    fn two_coordinated_share_errors_can_force_a_wrong_open_at_n5() {
        let t = 2usize;
        let (s, a, b) = (Fp::new(1_234_567), Fp::new(77), Fp::new(9));
        let f = |x: Fp| s.add(a.mul(x)).add(b.mul(x.mul(x)));
        let honest: Vec<Share> = (1..=5u64)
            .map(|x| Share {
                x,
                y: f(Fp::new(x)),
            })
            .collect();
        assert_eq!(
            shamir::reconstruct_degree(&honest, t).unwrap(),
            s,
            "the honest sharing must open to the secret"
        );

        // delta(x) = (x - 4)(x - 5): delta(1) = 12, delta(2) = 6, delta(0) = 20.
        let mut forged = honest.clone();
        forged[0].y = forged[0].y.add(Fp::new(12));
        forged[1].y = forged[1].y.add(Fp::new(6));
        let got = shamir::reconstruct_degree(&forged, t)
            .expect("the miscorrection window: this open does NOT refuse");
        assert_eq!(
            got,
            s.add(Fp::new(20)),
            "the open must return the other codeword's value, s + delta(0)"
        );
        assert_ne!(got, s, "and that value is not the secret");
    }

    /// A variable list that does not match the slots' payload arity is refused
    /// before anything is reconstructed — a `PartialResult` whose `vars` and row
    /// widths disagree would be read with its columns misaligned.
    #[test]
    fn reconstruct_partial_result_refuses_a_mismatched_output_arity() {
        let backend = ShamirBackend::new_seeded(5, 0x8E06).unwrap();
        let cands = recipient_candidates();
        let (shares, _) =
            oblivious_set_output_to_recipient(&backend, &cands, 1, 6, HolderId::new("recipient"))
                .unwrap();
        assert_eq!(shares.payload_arity(), 1);
        for vars in [vec![], vec![var("x"), var("y")]] {
            tag_opens::reset();
            let wanted = vars.len();
            match shares.reconstruct_partial_result(vars) {
                Err(MpcError::Protocol(m)) => {
                    assert!(
                        m.contains("payload arity") && m.contains(&wanted.to_string()),
                        "wrong refusal: {}",
                        m
                    );
                    assert_eq!(
                        tag_opens::count(),
                        0,
                        "the refusal must land before anything is reconstructed"
                    );
                }
                other => panic!("must fail closed for {} var(s), got {:?}", wanted, other),
            }
        }
        // The matching arity still works.
        assert_eq!(
            shares
                .reconstruct_partial_result(vec![var("x")])
                .unwrap()
                .vars,
            vec![var("x")]
        );
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

    // ---- L1: the revealed SLOT COUNT is the bound, not the true count ----------

    /// L1: the number of REVEALED slots is exactly `B` for ANY true match count, so
    /// the OUTPUT SIZE is `B` rather than the true cardinality. Two candidate sets
    /// with DIFFERENT true match counts must produce the IDENTICAL revealed-slot
    /// count. (The opened tags still distinguish real slots from dummies, so the
    /// count itself is in the transcript — see the module's residual-leakage note.)
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
        // Same revealed slot count (= B) regardless of true match count → the
        // output SIZE is B, not the true cardinality.
        assert_eq!(s1.len(), s3.len());
        assert_eq!(s1.len(), 5);
        // The slot count is identical across the two runs; the real-row multisets
        // differ (1 vs 3), which is what filtering the dummies yields.
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

    /// The all-pairs hidden-keys builder refuses before ANY draw on a NON-EMPTY key
    /// pair: an over-cap `B`, an over-cap `|L|·|R|`, and a product that overflows
    /// `usize`. One equality sharing per pair would otherwise be drawn, and the
    /// candidate vector is reserved from the product.
    #[test]
    fn the_hidden_keys_builder_refuses_before_any_draw() {
        use crate::randomness::test_sources::DrawCountingSource;
        let backend = ShamirBackend::new_seeded(5, 0x8F01).unwrap();
        let left = [Fp::new(1), Fp::new(2)];
        let right = [Fp::new(2), Fp::new(3)];

        for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
            let mut dealer = backend.dealer();
            let mut counted = DrawCountingSource::new(&mut dealer);
            let err = oblivious_set_output_hidden_keys_through(
                &mut counted,
                &backend,
                &left,
                &right,
                bound,
            )
            .expect_err("an over-cap bound must be refused");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                "B={}: {:?}",
                bound,
                err
            );
            assert_eq!(counted.draws(), 0, "B={}: refused before any draw", bound);
        }

        // An over-cap all-pairs cardinality, with `B` itself inside the cap.
        let big: Vec<Fp> = vec![Fp::new(5); 1024];
        let also_big: Vec<Fp> = vec![Fp::new(5); 1024];
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = oblivious_set_output_hidden_keys_through(
            &mut counted,
            &backend,
            &big,
            &also_big,
            MAX_OUTPUT_BOUND,
        )
        .expect_err("an over-cap pair count must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
            "pair count: {:?}",
            err
        );
        assert_eq!(counted.draws(), 0, "the pair count precedes every draw");

        // And an OVERFLOWING product is a controlled refusal, not a wrapped length
        // or an allocation nobody asked for. The lengths come from slice lengths, so
        // the overflow case is exercised on the validator directly.
        let err = checked_candidate_pairs(usize::MAX, 2).expect_err("must refuse on overflow");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("overflows usize")),
            "overflow: {:?}",
            err
        );
        // The cap is a cap, not a refusal of everything: the largest product it
        // admits is exactly `MAX_OUTPUT_BOUND` pairs.
        assert!(checked_candidate_pairs(64, 64).is_ok(), "the cap is a cap");
        assert_eq!(checked_candidate_pairs(64, 64).unwrap(), MAX_OUTPUT_BOUND);
        assert!(checked_candidate_pairs(64, 65).is_err(), "one pair past it");

        // The combined preflight the builders call: it refuses the overflow, the
        // over-cap product and a `B` that cannot cover the product, and hands back
        // the count on the way through.
        assert!(
            check_candidate_pairs_within_bound(MAX_OUTPUT_BOUND, usize::MAX, 2).is_err(),
            "the preflight must refuse an overflowing product"
        );
        assert!(
            check_candidate_pairs_within_bound(MAX_OUTPUT_BOUND, 1024, 1024).is_err(),
            "the preflight must refuse an over-cap product"
        );
        assert!(
            check_candidate_pairs_within_bound(3, 2, 2).is_err(),
            "the preflight must refuse a bound that cannot cover the product"
        );
        assert_eq!(
            check_candidate_pairs_within_bound(4, 2, 2).unwrap(),
            4,
            "and it hands back the count it checked"
        );
    }

    /// A bound past [`MAX_OUTPUT_BOUND`] — including `usize::MAX`, which would
    /// otherwise reach the stage's `Vec::with_capacity(bound)` — is a Protocol
    /// error, not a panic. The bound is validated before allocation.
    #[test]
    fn bound_above_cap_is_protocol_error_not_a_panic() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let cands = public_candidates(&[(true, vec![lit("a")])]);
        for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                oblivious_set_output(&backend, &cands, 1, bound)
            }));
            let err = outcome
                .unwrap_or_else(|_| panic!("B={} panicked instead of returning Err", bound))
                .expect_err("an over-cap bound must be refused");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                "B={}: expected the cap refusal, got {:?}",
                bound,
                err
            );
        }
        // The cap itself is still accepted as a parameter — it is a cap, not an
        // off-by-one. Checked on the validator so the test costs no `B`-slot run.
        assert!(check_output_bound(MAX_OUTPUT_BOUND, 1).is_ok());
    }

    /// The over-cap bound is refused before ANYTHING is drawn — on the dealer
    /// source and on PRSS alike, so the refusal does not depend on which source is
    /// driving the stage nor on that source's own refusals.
    #[test]
    fn bound_above_cap_is_refused_before_any_draw() {
        use crate::randomness::test_sources::DrawCountingSource;
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let cands = public_candidates(&[(true, vec![lit("a")])]);

        let mut dealer = backend.dealer();
        let mut prss = crate::prss::PrssRandomness::with_simulated_setup(3).unwrap();
        let mut sources: Vec<(&str, &mut dyn DistributedRandomness)> =
            vec![("dealer", &mut dealer), ("prss", &mut prss)];
        for (name, inner) in sources.iter_mut() {
            for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
                let mut counted = DrawCountingSource::new(*inner);
                let err = oblivious_set_output_through(&mut counted, &backend, &cands, 1, bound)
                    .expect_err("an over-cap bound must be refused");
                assert!(
                    matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                    "{} B={}: {:?}",
                    name,
                    bound,
                    err
                );
                assert_eq!(
                    counted.draws(),
                    0,
                    "{} B={}: the bound must be validated before any draw",
                    name,
                    bound
                );
            }
        }
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
        // The width is the DECLARED one, so the two runs differ in their
        // declaration and not in their data: the same candidate is carried at a
        // narrow and at a wide declared schema.
        let cands = public_candidates(&[(true, vec![lit("x")])]);
        let narrow_w = PayloadWidths::declare(1, 32).expect("a narrow declaration");
        let wide_w = PayloadWidths::declare(1, 512).expect("a wide declaration");
        let (_, c_narrow) = oblivious_set_output(&backend, &cands, narrow_w, 4).unwrap();
        let (_, c_wide) = oblivious_set_output(&backend, &cands, wide_w, 4).unwrap();
        assert_eq!(
            c_narrow.shuffle.items, c_wide.shuffle.items,
            "same slot count"
        );
        assert!(
            c_wide.shuffle.switches > c_narrow.shuffle.switches,
            "a wider declared payload must cost more switches (width-scaled): wide={}, narrow={}",
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

    /// THE row-level guard for the output stage's payload select: across ALL `B`
    /// slots, the set of opened payload rows is `{the matched candidates' rows} ∪
    /// {the dummy encoding}` — a rejected candidate's row is never among them.
    /// Asserted by comparing the rejected candidates' OWN payload rows against every
    /// slot's opened row, on the real path via `opened_slot_rows_through`.
    ///
    /// The LIMB-level counterpart is
    /// [`a_dummy_slots_opened_limbs_are_exactly_the_pad_block`], which is the
    /// stronger check: a wrong subset of selected limbs can still decode to the
    /// all-unbound row, so a row-level comparison alone would accept it.
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
            // And at the LIMB level: a dummy slot's opened limb vector is exactly
            // the pad block, not merely a vector that decodes to the dummy row.
            let t = b.threshold();
            let (shares, pad) =
                opened_slot_shares_through(&mut b.dealer(), &b, &cands, 2, 8).unwrap();
            for (tag, limbs) in &shares {
                if shamir::reconstruct_degree(tag, t).unwrap() == Fp::zero() {
                    let values: Vec<Fp> = limbs
                        .iter()
                        .map(|l| shamir::reconstruct_degree(l, t).unwrap())
                        .collect();
                    assert_eq!(values, pad, "seed {seed}: dummy slot limb vector");
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
        // B = 5: two rejected candidates and three pure padding slots. The wider
        // of the two rejected terms is 200 bytes, so the run declares a schema that
        // carries it.
        let widths = PayloadWidths::declare(1, 256).expect("one 256-byte column");
        let slots = opened_slot_rows_through(&mut b.dealer(), &b, &cands, widths, 5).unwrap();
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
        // Narrow and wide DECLARATIONS over the same candidate: the limb count is
        // a function of the declared width, not of the term that happens to be in
        // the row.
        let cands = public_candidates(&[(true, vec![lit("x")])]);
        let narrow_w = PayloadWidths::declare(1, 32).expect("a narrow declaration");
        let wide_w = PayloadWidths::declare(1, 512).expect("a wide declaration");
        let (_, c_narrow) = oblivious_set_output(&b, &cands, narrow_w, 4).unwrap();
        let (_, c_wide) = oblivious_set_output(&b, &cands, wide_w, 4).unwrap();
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
            "a wider declared payload must cost more limb selects: wide={}, narrow={}",
            c_wide.payload_select_mults,
            c_narrow.payload_select_mults
        );
    }

    // ---- Share-level: what a dummy slot's OPENED SHARINGS reveal ----------------

    /// Interpolate the unique polynomial through a sharing's `n` points and return
    /// its coefficients, lowest degree first. `n` distinct points determine a
    /// degree-`n−1` polynomial, so this recovers EVERYTHING the opened share vector
    /// determines — not just the value at 0. Built by expanding the Lagrange basis,
    /// so it needs no matrix solve.
    fn interpolate_coeffs(shares: &[Share]) -> Vec<Fp> {
        let n = shares.len();
        let mut coeffs = vec![Fp::zero(); n];
        for (i, si) in shares.iter().enumerate() {
            // Numerator polynomial Π_{j≠i} (x − x_j), and the scalar denominator
            // Π_{j≠i} (x_i − x_j).
            let mut num = vec![Fp::one()];
            let mut den = Fp::one();
            for (j, sj) in shares.iter().enumerate() {
                if i == j {
                    continue;
                }
                // Multiply `num` by (x − x_j).
                let mut next = vec![Fp::zero(); num.len() + 1];
                for (k, c) in num.iter().enumerate() {
                    next[k + 1] = next[k + 1].add(*c);
                    next[k] = next[k].sub(c.mul(Fp::new(sj.x)));
                }
                num = next;
                den = den.mul(Fp::new(si.x).sub(Fp::new(sj.x)));
            }
            let scale = si.y.mul(den.inv());
            for (k, c) in num.iter().enumerate() {
                coeffs[k] = coeffs[k].add(c.mul(scale));
            }
        }
        coeffs
    }

    /// The degree of an interpolated sharing: the highest index with a nonzero
    /// coefficient (0 for the all-zero polynomial).
    fn interpolated_degree(shares: &[Share]) -> usize {
        let c = interpolate_coeffs(shares);
        c.iter().rposition(|v| *v != Fp::zero()).unwrap_or(0)
    }

    /// The `x^1` coefficient of an interpolated sharing — the quantity the
    /// share-level recovery below divides.
    fn x_coeff(shares: &[Share]) -> Fp {
        interpolate_coeffs(shares)
            .get(1)
            .copied()
            .unwrap_or(Fp::zero())
    }

    /// The fixture for the share-level tests: two rejected candidates with distinct
    /// payloads and one match, run at a bound that also adds pure padding slots.
    fn share_level_candidates() -> Vec<Candidate> {
        public_candidates(&[
            (true, vec![lit("kept-row")]),
            (false, vec![lit("rejected-one")]),
            (false, vec![lit("rejected-two-is-longer")]),
        ])
    }

    /// **THE share-level guard.** Every element the stage opens — each slot's tag
    /// and each of its payload limbs — interpolates to a polynomial of degree at
    /// most `t`.
    ///
    /// This is the property that makes a dummy slot's opened share vector carry
    /// nothing but its value. A select is a product of the selector polynomial `f`
    /// with another polynomial `g`, and an UNREDUCED product sharing interpolates to
    /// degree `2t`: its share vector determines `f·g`, so for `sel = 0` (where
    /// `f(0) = 0`) the `x` coefficients give `[x](f·g_i) / [x](f·g_tag) = g_i(0)`,
    /// which is the candidate's own limb minus the pad. Degree-reducing every
    /// selected element replaces it with a FRESH degree-`t` sharing of the same
    /// value, whose non-constant coefficients are uniform and independent of `f`
    /// and `g`. So `degree <= t` is the exact, deterministic discriminator, and it
    /// is checked at every `n` the honest-majority constructor allows.
    #[test]
    fn every_opened_element_interpolates_to_degree_at_most_t() {
        let cands = share_level_candidates();
        for n in [3usize, 5, 7] {
            let b = ShamirBackend::new_seeded(n, 0xE0_0000 + n as u64).unwrap();
            let t = b.threshold();
            assert!(t >= 1, "n={n}: the fixture needs a real threshold");
            let (slots, _pad) =
                opened_slot_shares_through(&mut b.dealer(), &b, &cands, 1, 6).unwrap();
            assert_eq!(slots.len(), 6);
            for (k, (tag, limbs)) in slots.iter().enumerate() {
                assert_eq!(tag.len(), n, "n={n}: tag sharing width");
                assert!(
                    interpolated_degree(tag) <= t,
                    "n={n}, slot {k}: tag interpolates to degree {} > t={t} — an \
                     unreduced product sharing",
                    interpolated_degree(tag)
                );
                for (i, limb) in limbs.iter().enumerate() {
                    assert_eq!(limb.len(), n, "n={n}: limb sharing width");
                    assert!(
                        interpolated_degree(limb) <= t,
                        "n={n}, slot {k}, limb {i}: interpolates to degree {} > t={t} — an \
                         unreduced product sharing",
                        interpolated_degree(limb)
                    );
                }
            }
        }
    }

    /// **THE share-level recovery guard.** The `x`-coefficient-ratio recovery that
    /// an unreduced product sharing admits —
    /// `limb_i = PAD_i + [x]h_i / [x]h_tag` — must not return a rejected
    /// candidate's limbs.
    ///
    /// Run on the real path's opened sharings at `n = 3, 5, 7`, with both PUBLIC
    /// and SECRET-SHARED selectors, over several seeds. After the per-element
    /// degree reduction the two `x` coefficients are independent uniform values, so
    /// the ratio is noise: the test asserts the recovered block is never the
    /// rejected candidate's limb block, and that it VARIES across seeds while the
    /// rejected candidate's value is fixed — i.e. it carries no information about
    /// the row. On an unreduced product sharing the same formula returns the
    /// rejected candidate's limbs exactly.
    #[test]
    fn the_share_level_recovery_never_returns_a_rejected_row() {
        let public = share_level_candidates();
        let rejected_blocks = |cands: &[Candidate], pad: &[Fp]| -> Vec<Vec<Fp>> {
            // What each candidate's limb block WOULD be, padded to the uniform
            // width — the thing the recovery must never produce for a dummy slot.
            cands
                .iter()
                .map(|c| {
                    let mut l = bytes_to_limbs(&encode_payload_row(&c.payload));
                    l.resize(pad.len(), Fp::new(PAD_LIMB));
                    l
                })
                .collect()
        };
        for n in [3usize, 5, 7] {
            // The same fixture with SECRET-SHARED selectors, so both regimes are
            // covered at every party count.
            let b0 = ShamirBackend::new_seeded(n, 7).unwrap();
            let mut d0 = b0.dealer();
            let (one, zero) = (d0.share(Fp::one()), d0.share(Fp::zero()));
            let secret = vec![
                Candidate {
                    payload: vec![lit("kept-row")],
                    matched: MatchBit::SecretShared(one),
                },
                Candidate {
                    payload: vec![lit("rejected-one")],
                    matched: MatchBit::SecretShared(zero.clone()),
                },
                Candidate {
                    payload: vec![lit("rejected-two-is-longer")],
                    matched: MatchBit::SecretShared(zero),
                },
            ];
            for (label, cands) in [("public", &public), ("secret-shared", &secret)] {
                let mut recovered_seen: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                for seed in 0..8u64 {
                    let b = ShamirBackend::new_seeded(n, 0xE1_0000 + seed).unwrap();
                    let (slots, pad) =
                        opened_slot_shares_through(&mut b.dealer(), &b, cands, 1, 6).unwrap();
                    let forbidden = rejected_blocks(cands, &pad);
                    for (k, (tag, limbs)) in slots.iter().enumerate() {
                        let xt = x_coeff(tag);
                        if xt == Fp::zero() {
                            // The formula needs a nonzero divisor. After the
                            // reduction `[x]h_tag` is uniform, so this is possible;
                            // it is the formula being inapplicable, not a recovery.
                            continue;
                        }
                        let inv = xt.inv();
                        let rec: Vec<Fp> = limbs
                            .iter()
                            .zip(&pad)
                            .map(|(l, p)| p.add(x_coeff(l).mul(inv)))
                            .collect();
                        for (c, block) in forbidden.iter().enumerate() {
                            assert_ne!(
                                &rec, block,
                                "n={n}, {label}, seed {seed}, slot {k}: the share-level \
                                 recovery returned candidate {c}'s limb block"
                            );
                        }
                        recovered_seen.insert(format!("{:?}", rec));
                    }
                }
                // The recovered block is noise, not a fixed function of the row: it
                // takes many different values across seeds although the candidates
                // (and so their limb blocks) never change.
                assert!(
                    recovered_seen.len() > 8,
                    "n={n}, {label}: the share-level recovery produced only {} distinct \
                     results across 8 seeds × 6 slots — it is not independent of the rows",
                    recovered_seen.len()
                );
            }
        }
    }

    /// The same algebra at `n = 3`, `t = 1` over many trials, plus a POSITIVE
    /// CONTROL that the formula itself works.
    ///
    /// `n = 3` is the smallest honest-majority party count and the one where the
    /// opened vector is shortest, so it is where a reduction would be easiest to
    /// miss. The control builds an UNREDUCED product sharing by hand — a sharing of
    /// `sel = 0` times a sharing of `limb − PAD`, exactly what the select produces
    /// before the reduction — and shows the recovery
    /// `PAD + [x](sel·(limb−PAD)) / [x](sel·real_tag)` returns `limb` on it. The
    /// same formula run over 64 trials of the real path returns the pad or noise,
    /// never the rejected candidate's limb.
    #[test]
    fn the_share_level_recovery_at_n3_over_many_trials() {
        let n = 3usize;
        // Positive control: on an unreduced product the formula recovers the limb.
        {
            let b = ShamirBackend::new_seeded(n, 0xE3_0001).unwrap();
            let t = b.threshold();
            assert_eq!(t, 1, "n=3 must give t=1");
            let mut d = b.dealer();
            let sel = d.share(Fp::zero());
            let real_tag = d.share(Fp::one());
            let limb = Fp::new(1234);
            let pad = Fp::new(PAD_LIMB);
            let shifted = shamir::add_constant(&d.share(limb), pad.neg());
            let h_tag = shamir::mul_shares_raw(&sel, &real_tag).unwrap();
            let h_limb = shamir::mul_shares_raw(&sel, &shifted).unwrap();
            assert!(
                interpolated_degree(&h_tag) > t,
                "the control's product sharing must be above degree t"
            );
            let recovered = pad.add(x_coeff(&h_limb).mul(x_coeff(&h_tag).inv()));
            assert_eq!(
                recovered, limb,
                "the recovery formula must work on an unreduced product sharing \
                 — otherwise the assertions below prove nothing"
            );
        }
        // The real path: 64 trials, the same formula, never the rejected row.
        let cands = share_level_candidates();
        let mut recovered_seen: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let mut applied = 0usize;
        for seed in 0..64u64 {
            let b = ShamirBackend::new_seeded(n, 0xE3_1000 + seed).unwrap();
            let (slots, pad) =
                opened_slot_shares_through(&mut b.dealer(), &b, &cands, 1, 6).unwrap();
            let forbidden: Vec<Vec<Fp>> = cands
                .iter()
                .map(|c| {
                    let mut l = bytes_to_limbs(&encode_payload_row(&c.payload));
                    l.resize(pad.len(), Fp::new(PAD_LIMB));
                    l
                })
                .collect();
            for (k, (tag, limbs)) in slots.iter().enumerate() {
                let xt = x_coeff(tag);
                if xt == Fp::zero() {
                    continue;
                }
                applied += 1;
                let inv = xt.inv();
                let rec: Vec<Fp> = limbs
                    .iter()
                    .zip(&pad)
                    .map(|(l, p)| p.add(x_coeff(l).mul(inv)))
                    .collect();
                for (c, block) in forbidden.iter().enumerate() {
                    assert_ne!(
                        &rec, block,
                        "n=3, seed {seed}, slot {k}: the recovery returned candidate {c}'s limbs"
                    );
                }
                recovered_seen.insert(format!("{:?}", rec));
            }
        }
        assert!(
            applied > 300,
            "the formula must have been applicable on most of the 64 x 6 slots, was {}",
            applied
        );
        assert!(
            recovered_seen.len() > 64,
            "n=3: only {} distinct recoveries over 64 trials — not independent of the rows",
            recovered_seen.len()
        );
    }

    /// A dummy slot's OPENED LIMB VECTOR is exactly the pad block — not merely a
    /// vector that happens to decode to the all-unbound row. Several ways of
    /// selecting the wrong subset of limbs still decode to that row, so the check
    /// that matters is on the limbs.
    #[test]
    fn a_dummy_slots_opened_limbs_are_exactly_the_pad_block() {
        let cands = share_level_candidates();
        for n in [3usize, 5, 7] {
            let b = ShamirBackend::new_seeded(n, 0xE2_0000 + n as u64).unwrap();
            let t = b.threshold();
            let (slots, pad) =
                opened_slot_shares_through(&mut b.dealer(), &b, &cands, 1, 6).unwrap();
            let mut dummies = 0usize;
            for (tag, limbs) in &slots {
                let values = limbs
                    .iter()
                    .map(|l| shamir::reconstruct_degree(l, t).unwrap())
                    .collect::<Vec<Fp>>();
                if shamir::reconstruct_degree(tag, t).unwrap() == Fp::zero() {
                    dummies += 1;
                    assert_eq!(
                        values.iter().map(|v| v.value()).collect::<Vec<u64>>(),
                        pad.iter().map(|v| v.value()).collect::<Vec<u64>>(),
                        "n={n}: a dummy slot's opened limbs are not the pad block"
                    );
                }
            }
            // Two rejected candidates and three pure padding slots.
            assert_eq!(dummies, 5, "n={n}: dummy slot count");
        }
    }

    /// The pad block IS the encoding of the all-unbound row of this arity, padded to
    /// the uniform width — so a dummy slot and a pure padding slot are the same
    /// thing limb for limb, and the pad block decodes to the dummy row.
    #[test]
    fn the_pad_block_is_the_dummy_encoding_at_the_uniform_width() {
        for arity in [1usize, 2, 3] {
            let b = ShamirBackend::new_seeded(3, 0xE3).unwrap();
            let cands: Vec<Candidate> = vec![Candidate {
                payload: vec![lit("x"); arity],
                matched: MatchBit::Public(false),
            }];
            let (_slots, pad) =
                opened_slot_shares_through(&mut b.dealer(), &b, &cands, arity, 3).unwrap();
            let mut expect = bytes_to_limbs(&encode_payload_row(&vec![None; arity]));
            assert!(
                pad.len() >= expect.len(),
                "arity {arity}: pad narrower than dummy"
            );
            expect.resize(pad.len(), Fp::new(PAD_LIMB));
            assert_eq!(pad, expect, "arity {arity}: pad block");
            let bytes = limbs_to_bytes(&pad).unwrap();
            assert_eq!(
                decode_payload_row(&bytes, arity).unwrap(),
                vec![None; arity],
                "arity {arity}: the pad block must decode to the all-unbound row"
            );
        }
    }

    // ---- Tag classification happens before any payload limb is opened (F3) ------

    /// An out-of-set tag anywhere in the column refuses BEFORE a single payload limb
    /// is opened: all `B` tags are opened and classified first. Asserted with the
    /// limb-open tally, not inferred.
    #[test]
    fn an_out_of_set_tag_refuses_before_any_limb_is_opened() {
        let b = ShamirBackend::new_seeded(5, 0xE4).unwrap();
        let mut dealer = b.dealer();
        // A malformed selector: a sharing of 2, so this slot's tag opens to 2.
        let two = dealer.share(Fp::new(2));
        let cands = vec![
            Candidate {
                payload: vec![lit("fine")],
                matched: MatchBit::Public(true),
            },
            Candidate {
                payload: vec![lit("malformed-selector")],
                matched: MatchBit::SecretShared(two),
            },
            Candidate {
                payload: vec![lit("also-fine")],
                matched: MatchBit::Public(false),
            },
        ];
        limb_opens::reset();
        let err = oblivious_set_output_through(&mut dealer, &b, &cands, 1, 5).unwrap_err();
        assert_eq!(
            limb_opens::count(),
            0,
            "a payload limb was opened before the tags were all classified"
        );
        match err {
            MpcError::Tampered { detail, .. } => assert!(
                detail.contains("neither REAL_TAG"),
                "must name the out-of-set tag: {detail}"
            ),
            other => panic!("expected Tampered, got {other:?}"),
        }
    }

    /// The counterpart: a well-formed run DOES open every slot's limbs, so the
    /// zero-open assertion above is about the refusal and not about the tally never
    /// moving.
    #[test]
    fn a_well_formed_run_opens_every_slots_limbs() {
        let b = ShamirBackend::new_seeded(5, 0xE5).unwrap();
        let cands = share_level_candidates();
        limb_opens::reset();
        let (slots, cost) = oblivious_set_output(&b, &cands, 1, 6).unwrap();
        assert_eq!(slots.len(), 6);
        // One open per packed limb per slot, which is what the cost model counts as
        // one select multiplication each.
        assert_eq!(limb_opens::count(), cost.payload_select_mults);
        assert!(limb_opens::count() >= 6, "at least one limb per slot");
    }

    // ---- The dummy-limb check is reachable and fires (F4) -----------------------

    /// A shuffle source that keeps the column's SHAPE but rewrites every payload
    /// limb to a consistent sharing of a different value reaches the dummy-limb
    /// check: the tags still open to the dummy sentinel, every limb still opens
    /// cleanly at degree `t`, and the only thing that can notice is the comparison
    /// against the pad block. It must be a `Tampered` refusal.
    #[test]
    fn a_tampered_payload_on_a_dummy_slot_is_refused() {
        use crate::randomness::test_sources::{ShuffleTampering, TamperingShuffleSource};
        let b = ShamirBackend::new_seeded(5, 0xE6).unwrap();
        // No matches, so EVERY slot is a dummy and the first slot checked trips it.
        let cands = public_candidates(&[(false, vec![lit("a")]), (false, vec![lit("b")])]);
        let mut src = TamperingShuffleSource::new(
            b.dealer(),
            // Keep each entry's tag element, rewrite all its payload limbs.
            ShuffleTampering::ResharePast {
                keep_prefix: 1,
                value: 123,
            },
        );
        let err = oblivious_set_output_through(&mut src, &b, &cands, 1, 4).unwrap_err();
        assert!(src.fired(), "the shuffle never happened");
        match err {
            MpcError::Tampered { detail, .. } => assert!(
                detail.contains("did not carry the dummy limb block"),
                "must name the dummy-limb check: {detail}"
            ),
            other => panic!("expected Tampered on a tampered dummy payload, got {other:?}"),
        }
    }

    // ---- Strict decoding (F4) --------------------------------------------------

    /// `limbs_to_bytes` refuses a limb block whose bytes past the length prefix are
    /// not the pad byte, so one limb block carries exactly one byte stream. Without
    /// this, a block with the right length prefix but different trailing content
    /// would decode to the same row.
    #[test]
    fn limbs_to_bytes_refuses_content_past_the_length_prefix() {
        // A 1-byte stream packed into two limbs: length prefix 1, data limb 0x00.
        let good = bytes_to_limbs(&[0u8]);
        assert_eq!(limbs_to_bytes(&good).unwrap(), vec![0u8]);
        // The same length prefix, but a data limb carrying a second byte.
        let bad = vec![Fp::new(1), Fp::new(0xAB00)];
        let err = limbs_to_bytes(&bad).unwrap_err();
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("past the length prefix")),
            "got {err:?}"
        );
        // And a pad limb appended past the data is still fine (that is the padding
        // `bytes_to_limbs` itself produces).
        let mut padded = good.clone();
        padded.push(Fp::new(PAD_LIMB));
        assert_eq!(limbs_to_bytes(&padded).unwrap(), vec![0u8]);
    }

    /// `decode_payload_row` refuses trailing bytes after the `arity` columns, so one
    /// byte stream decodes to exactly one row.
    #[test]
    fn decode_payload_row_refuses_trailing_bytes() {
        let row = vec![lit("v")];
        let mut bytes = encode_payload_row(&row);
        assert_eq!(decode_payload_row(&bytes, 1).unwrap(), row);
        bytes.push(0u8); // one trailing byte: a fourth column's worth of nothing
        let err = decode_payload_row(&bytes, 1).unwrap_err();
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("trailing byte")),
            "got {err:?}"
        );
    }

    // ---- Decode refusals name no opened value -----------------------------------

    /// Check one decode refusal in BOTH renderings: `Display` and `Debug` (the
    /// variant's payload is the same string, and a caller may log either).
    fn assert_decode_refusal(what: &str, err: &MpcError, forbidden: &[&str], expected: &[&str]) {
        for (how, rendered) in [
            ("Display", err.to_string()),
            ("Debug", format!("{:?}", err)),
        ] {
            for bad in forbidden {
                assert!(
                    !rendered.contains(bad),
                    "{} ({}) renders the opened value {}: {}",
                    what,
                    how,
                    bad,
                    rendered
                );
            }
            for good in expected {
                assert!(
                    rendered.contains(good),
                    "{} ({}) dropped the public detail {}: {}",
                    what,
                    how,
                    good,
                    rendered
                );
            }
        }
    }

    /// The bytes and limbs these two decoders inspect come out of an OPENED limb
    /// block, so every refusal must describe the error class and the position it
    /// occurred at — the public column index, limb index or block width — and never
    /// the opened byte, limb value or byte count itself.
    #[test]
    fn the_decode_refusals_name_the_position_but_no_opened_value() {
        // 1. An out-of-set presence flag: the flag byte is opened, the column is public.
        let err = decode_payload_row(&[91u8], 1).unwrap_err();
        assert_decode_refusal(
            "an invalid presence flag",
            &err,
            &["91"],
            &["presence flag", "column 0"],
        );

        // 2. Trailing bytes: HOW MANY trail is opened, the column arity is public.
        let mut bytes = encode_payload_row(&[lit("v")]);
        bytes.extend(std::iter::repeat_n(0u8, 37));
        let err = decode_payload_row(&bytes, 1).unwrap_err();
        assert_decode_refusal(
            "a trailing-byte stream",
            &err,
            &["37"],
            &["trailing byte", "1 column"],
        );

        // 3. An over-long length prefix: the prefix is opened, the block's uniform
        //    width (one data limb here, so 7 bytes) is public.
        let err = limbs_to_bytes(&[Fp::new(93), Fp::new(0)]).unwrap_err();
        assert_decode_refusal(
            "an over-long length prefix",
            &err,
            &["93"],
            &["length prefix", "7 available"],
        );

        // 4. An out-of-range data limb: the limb VALUE is opened, its index is public.
        let wide = (1u64 << (8 * BYTES_PER_LIMB)) | 0x1234;
        let err = limbs_to_bytes(&[Fp::new(1), Fp::new(0), Fp::new(wide)]).unwrap_err();
        assert_decode_refusal(
            "an out-of-range data limb",
            &err,
            &[&format!("{}", wide)],
            &["data limb at index 1", "7-byte range"],
        );

        // 5. Content past the length prefix: both the count of offending bytes and
        //    the prefix they sit past are opened, so this refusal carries no number
        //    at all.
        let err = limbs_to_bytes(&[Fp::new(1), Fp::new(0xAB_CD_EF_00)]).unwrap_err();
        assert_decode_refusal(
            "content past the length prefix",
            &err,
            &[],
            &["past the length prefix", "not the pad byte"],
        );
        for (how, rendered) in [
            ("Display", err.to_string()),
            ("Debug", format!("{:?}", err)),
        ] {
            assert!(
                !rendered.chars().any(|c| c.is_ascii_digit()),
                "content past the length prefix ({}) renders a number: {}",
                how,
                rendered
            );
        }
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
    // ---- Output-stage WIDTH caps (arity / term width / limbs / parties / work) ----

    /// Every cap is pinned BY VALUE, so a change to one is a change to this test
    /// and not a silent relaxation of a protocol guard.
    #[test]
    fn the_width_caps_are_pinned_by_value() {
        assert_eq!(MAX_OUTPUT_BOUND, 4096);
        assert_eq!(MAX_PAYLOAD_ARITY, 64);
        assert_eq!(MAX_PAYLOAD_TERM_BYTES, 4096);
        assert_eq!(DEFAULT_PAYLOAD_TERM_BYTES, 128);
        assert_eq!(MAX_SLOT_LIMBS, 2048);
        assert_eq!(MAX_OUTPUT_PARTIES, 7);
        assert_eq!(MAX_SLOT_WORK, 1 << 28);
    }

    /// The padded width is a closed form of the DECLARATION — it does not depend
    /// on the candidates' own term lengths.
    ///
    /// Two runs of the same declaration over candidate sets whose terms differ by
    /// orders of magnitude pay the same per-slot width, and that width is the one
    /// [`PayloadWidths::limb_capacity`] states from public numbers alone. This is
    /// the property the old data-derived width did not have: there, the longest
    /// candidate set the stage to a wider schedule.
    #[test]
    fn the_padded_width_is_a_closed_form_of_the_declaration_not_of_the_data() {
        let backend = ShamirBackend::new_seeded(5, 0x7A10).unwrap();
        let widths = PayloadWidths::declare(1, 64).expect("a legal declaration");
        let declared = widths.limb_capacity().expect("a legal declaration has a width");

        // One short term, one term 50x longer; both inside the declared width.
        let short = public_candidates(&[(true, vec![lit("x")])]);
        let long = public_candidates(&[(true, vec![lit(&"x".repeat(50))])]);
        let bound = 4;
        let mut observed = Vec::new();
        for cands in [&short, &long] {
            let (slots, cost) =
                oblivious_set_output(&backend, cands, widths, bound).expect("both runs are legal");
            assert_eq!(slots.len(), bound);
            // One select multiplication per limb per slot, so the per-slot width is
            // the quotient.
            assert_eq!(cost.payload_select_mults % bound, 0);
            observed.push(cost.payload_select_mults / bound);
        }
        assert_eq!(
            observed[0], observed[1],
            "the width must not follow the data: {:?}",
            observed
        );
        assert_eq!(
            observed[0], declared,
            "and it must be the width the declaration states"
        );
    }

    /// A row whose payload arity is one past [`MAX_PAYLOAD_ARITY`] is refused with
    /// ZERO draws, and an arity exactly AT the cap runs. The over-cap side also
    /// covers `usize::MAX`, which used to reach `vec![None; usize::MAX]`.
    #[test]
    fn the_payload_arity_cap_refuses_just_over_and_runs_at_the_cap() {
        use crate::randomness::test_sources::DrawCountingSource;
        let backend = ShamirBackend::new_seeded(5, 0x7A11).unwrap();

        // The closed-form guard itself, exactly at and past the cap.
        assert!(check_payload_arity_cap(MAX_PAYLOAD_ARITY).is_ok());
        let err = check_payload_arity_cap(MAX_PAYLOAD_ARITY + 1)
            .expect_err("one column past the cap is refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PAYLOAD_ARITY")),
            "{:?}",
            err
        );

        for arity in [MAX_PAYLOAD_ARITY + 1, usize::MAX] {
            let mut dealer = backend.dealer();
            let mut counted = DrawCountingSource::new(&mut dealer);
            // No candidate, so nothing but the declaration itself can be refused on.
            let err = oblivious_set_output_through(&mut counted, &backend, &[], arity, 1)
                .expect_err("an over-cap payload arity must be refused");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PAYLOAD_ARITY")),
                "arity={}: {:?}",
                arity,
                err
            );
            assert_eq!(
                counted.draws(),
                0,
                "arity={}: refused before any draw",
                arity
            );
            assert!(oblivious_set_output(&backend, &[], arity, 1).is_err());
        }

        // AT the cap the stage still runs: one all-unbound row of the widest legal
        // arity, revealed at B = 1.
        let cands = public_candidates(&[(true, vec![None; MAX_PAYLOAD_ARITY])]);
        let (slots, cost) = oblivious_set_output(&backend, &cands, MAX_PAYLOAD_ARITY, 1)
            .expect("the widest legal arity runs");
        assert_eq!(slots.len(), 1);
        assert!(cost.payload_select_mults > 0);
    }

    /// A term wider than the DECLARED per-term width is refused with ZERO draws,
    /// and the refusal carries public values only — no column, no candidate
    /// position, no text, no length. A term exactly at the declared width runs and
    /// round-trips.
    #[test]
    fn a_term_wider_than_the_declaration_is_refused_with_public_values_only() {
        use crate::randomness::test_sources::DrawCountingSource;
        let backend = ShamirBackend::new_seeded(5, 0x7A12).unwrap();
        let declared = 64usize;
        let widths = PayloadWidths::declare(2, declared).expect("a legal declaration");

        // A simple literal serialises as `"<text>"`, so the N-Triples form is two
        // bytes longer than its text.
        let at_width = lit(&"x".repeat(declared - 2));
        let over_width = lit(&"x".repeat(declared - 1));
        assert_eq!(at_width.as_ref().unwrap().to_string().len(), declared);

        // The over-width term sits in the SECOND column of the second candidate, so
        // a position-naming refusal would say so.
        let cands = public_candidates(&[
            (true, vec![at_width.clone(), at_width.clone()]),
            (true, vec![at_width.clone(), over_width]),
        ]);
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = oblivious_set_output_through(&mut counted, &backend, &cands, widths, 4)
            .expect_err("a term wider than the declaration must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("declared per-term width")),
            "{:?}",
            err
        );
        assert_eq!(counted.draws(), 0, "refused before any draw");
        let text = format!("{}", err);
        for forbidden in ["column", "candidate 1", "xxxx"] {
            assert!(
                !text.contains(forbidden),
                "the refusal must not carry {}: {}",
                forbidden,
                text
            );
        }
        assert!(text.contains(&declared.to_string()), "{}", text);

        // AT the declared width it runs, and the row survives the round-trip.
        let cands = public_candidates(&[(true, vec![at_width.clone(), at_width.clone()])]);
        let (slots, _cost) = oblivious_set_output(&backend, &cands, widths, 2)
            .expect("the widest declared term runs");
        let rows: Vec<&Vec<Option<Term>>> = slots
            .iter()
            .filter_map(|s| match s {
                OutputSlot::Row(r) => Some(r),
                OutputSlot::Dummy => None,
            })
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], at_width);
    }

    /// The declaration's own per-term ceiling: a width past
    /// [`MAX_PAYLOAD_TERM_BYTES`] is refused on the declaration alone, with no
    /// candidate read and no draw.
    #[test]
    fn a_declaration_past_the_per_term_ceiling_is_refused() {
        use crate::randomness::test_sources::DrawCountingSource;
        let backend = ShamirBackend::new_seeded(5, 0x7A15).unwrap();
        let err = PayloadWidths::declare(1, MAX_PAYLOAD_TERM_BYTES + 1)
            .expect_err("a declaration past the ceiling is refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PAYLOAD_TERM_BYTES")),
            "{:?}",
            err
        );

        // And through the stage, with no candidate at all.
        let widths = PayloadWidths {
            arity: 1,
            term_bytes: MAX_PAYLOAD_TERM_BYTES + 1,
        };
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = oblivious_set_output_through(&mut counted, &backend, &[], widths, 1)
            .expect_err("the stage refuses the declaration too");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_PAYLOAD_TERM_BYTES")),
            "{:?}",
            err
        );
        assert_eq!(counted.draws(), 0, "refused before any draw");
    }

    /// The uniform packed limb width is capped in its own right: a declaration
    /// whose arity and per-term width are each legal but whose ROW does not fit
    /// [`MAX_SLOT_LIMBS`] is refused, on the declaration alone.
    #[test]
    fn the_slot_limb_cap_refuses_a_declaration_that_is_too_wide() {
        use crate::randomness::test_sources::DrawCountingSource;
        let backend = ShamirBackend::new_seeded(5, 0x7A13).unwrap();

        assert!(check_slot_limbs_cap(MAX_SLOT_LIMBS).is_ok());
        let err =
            check_slot_limbs_cap(MAX_SLOT_LIMBS + 1).expect_err("one limb past the cap is refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_SLOT_LIMBS")),
            "{:?}",
            err
        );

        // The widest legal arity at the widest legal per-term width: both factors
        // are inside their own caps, the row they describe is not.
        let err = PayloadWidths::declare(MAX_PAYLOAD_ARITY, MAX_PAYLOAD_TERM_BYTES)
            .expect_err("an over-width row is refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_SLOT_LIMBS")),
            "{:?}",
            err
        );
        // An overflowing declaration is a refusal, not a wrapped width.
        let err = PayloadWidths::declare(usize::MAX, usize::MAX)
            .expect_err("an overflowing declaration is refused");
        assert!(matches!(&err, MpcError::Protocol(_)), "{:?}", err);

        // And through the stage, with no candidate at all.
        let widths = PayloadWidths {
            arity: MAX_PAYLOAD_ARITY,
            term_bytes: MAX_PAYLOAD_TERM_BYTES,
        };
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = oblivious_set_output_through(&mut counted, &backend, &[], widths, 1)
            .expect_err("an over-width declaration must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_SLOT_LIMBS")),
            "{:?}",
            err
        );
        assert_eq!(counted.draws(), 0, "refused before any draw");
    }

    /// The party count is a factor of the work and is capped in its own right: a
    /// backend with more compute parties than [`MAX_OUTPUT_PARTIES`] is refused
    /// before any candidate is read.
    #[test]
    fn the_output_parties_cap_refuses_a_wider_federation() {
        use crate::randomness::test_sources::DrawCountingSource;
        assert!(check_output_parties_cap(MAX_OUTPUT_PARTIES).is_ok());
        let err = check_output_parties_cap(MAX_OUTPUT_PARTIES + 1)
            .expect_err("one party past the cap is refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_PARTIES")),
            "{:?}",
            err
        );

        let wide = ShamirBackend::new_seeded(MAX_OUTPUT_PARTIES + 2, 0x7A16).unwrap();
        let cands = public_candidates(&[(true, vec![lit("x")])]);
        let mut dealer = wide.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = oblivious_set_output_through(&mut counted, &wide, &cands, 1, 2)
            .expect_err("a federation wider than the budget models must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_PARTIES")),
            "{:?}",
            err
        );
        assert_eq!(counted.draws(), 0, "refused before any draw");

        // At the cap it runs.
        let at_cap = ShamirBackend::new_seeded(MAX_OUTPUT_PARTIES, 0x7A17).unwrap();
        assert!(oblivious_set_output(&at_cap, &cands, 1, 2).is_ok());
    }

    /// The aggregate work budget bounds the PRODUCT, not just the factors: a `B`,
    /// a declared width, a party count and a threshold that are each individually
    /// legal are refused when they multiply past [`MAX_SLOT_WORK`]. The per-slot
    /// tag element is charged alongside the limbs, so a zero-width run is not
    /// waved through, and an overflowing product is a refusal rather than a
    /// wrapped number.
    ///
    /// The product is `B × (1 + limbs) × n × (2t+1)`: one tag element plus the
    /// slot's limbs, at every party, through the degree reduction's `2t+1`
    /// re-sharings. Arity is NOT a factor — `limbs` is derived from the declared
    /// row, which already spans every column — so multiplying it in again charged
    /// the columns twice.
    #[test]
    fn the_slot_work_budget_bounds_the_product_not_just_the_factors() {
        use crate::randomness::test_sources::DrawCountingSource;
        // (n=7, t=3): B x (1 + limbs) x 7 x 7. At a 2047-limb slot, B = 2674 is
        // just inside the budget and one slot more is not.
        assert!(check_slot_work_budget(2674, 2047, 7, 3).is_ok());
        assert!(check_slot_work_budget(2675, 2047, 7, 3).is_err());
        let err = check_slot_work_budget(2675, 2047, 7, 3).expect_err("just past the budget");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_SLOT_WORK")),
            "{:?}",
            err
        );
        // The PARTY count alone moves the same plan across the budget, and so does
        // the THRESHOLD through the degree reduction's 2t+1 re-sharings — both are
        // factors of the product, not bystanders.
        assert!(check_slot_work_budget(2674, 2047, 3, 3).is_ok());
        assert!(check_slot_work_budget(2674, 2047, 7, 3).is_ok());
        assert!(check_slot_work_budget(2674, 2047, 7, 4).is_err());
        // Each cap on its own is legal, and their CORNER is not: the widest `B` at
        // the widest uniform slot is past the budget at a legal federation size.
        assert!(check_slot_work_budget(MAX_OUTPUT_BOUND, MAX_SLOT_LIMBS, 7, 3).is_err());
        // A ZERO-width run is still charged for its per-slot tag work, so the
        // product is not zeroed by a width of 0.
        assert!(check_slot_work_budget(usize::MAX, 0, 7, 3).is_err());
        // An overflowing product refuses rather than wrapping under the budget.
        let err = check_slot_work_budget(usize::MAX, usize::MAX, usize::MAX, usize::MAX)
            .expect_err("an overflowing product is refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("overflows u64")),
            "{:?}",
            err
        );

        // And on the real stage: the widest legal `B` with a declared width whose
        // limb count is inside `MAX_SLOT_LIMBS`. Every factor is inside its own
        // cap, so only their product can refuse this.
        let backend = ShamirBackend::new_seeded(7, 0x7A14).unwrap();
        let widths = PayloadWidths::declare(3, MAX_PAYLOAD_TERM_BYTES)
            .expect("3 columns at the per-term ceiling is a legal declaration");
        assert!(
            widths.limb_capacity().expect("the declared row fits") <= MAX_SLOT_LIMBS,
            "the fixture's own shape must be legal, so the product is what refuses"
        );
        let cands = public_candidates(&[(true, vec![None; 3])]);
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err =
            oblivious_set_output_through(&mut counted, &backend, &cands, widths, MAX_OUTPUT_BOUND)
                .expect_err("the product of legal factors must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_SLOT_WORK")),
            "{:?}",
            err
        );
        assert_eq!(counted.draws(), 0, "refused before any draw");
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
        fingerprint, prefix_before_first_shuffle, MalformedShuffleSource, MalformedSource,
        MisreportedThresholdSource, RecordingSource, SeamMethod, ALL_MALFORMATIONS,
        ALL_SHUFFLE_MALFORMATIONS, SHUFFLE_SHAPE_REFUSAL,
    };

    /// The refusal [`crate::randomness::require_source_matches`] raises.
    const MISMATCH_REFUSAL: &str = "its sharings would not compose with the backend's";

    /// The common substring of the TWO shape refusals a malformed draw can hit:
    /// the stage's own `checked_vss_own_input`
    /// ([`crate::randomness::test_sources::SHAPE_REFUSAL`], "returned a sharing
    /// that is not the canonical …") and `shamir::degree_reduce_through`'s
    /// `require_canonical_sharing` on a re-sharing ("returned a re-sharing that is
    /// not the canonical …"). Both are the shape check at the consumption site;
    /// which one fires depends only on whether the corrupted draw was the stage's
    /// or one of the reduction's.
    const ANY_SHAPE_REFUSAL: &str = "sharing that is not the canonical";

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

    /// The fixture's seam transcript is `VSS_DRAWS` sharings then one shuffle, at
    /// `n = 5`, `t = 2` (so each degree reduction re-shares through `2t + 1 = 5`
    /// parties). Per slot the tag costs a selector (a zero-selector, for a padding
    /// slot) + a real-tag sharing + 5 reduction re-sharings = 7; then each payload
    /// limb costs the limb sharing + 5 reduction re-sharings = 6. The limb count is
    /// the DECLARED width's, not the data's: arity 1 at
    /// `DEFAULT_PAYLOAD_TERM_BYTES` is a 130-byte row, so
    /// `limb_count_for(130) = 20` limbs per slot whatever the term in it is. Five
    /// slots × (7 + 20 × 6) = 635, and then the oblivious shuffle over the five
    /// packed entries. It was 95 while the width was derived from the widest term
    /// present — two limbs for these one-character literals — which is exactly the
    /// data dependence the declared width removes: the transcript's length is now a
    /// function of public numbers only.
    const VSS_DRAWS: usize = 635;

    fn rendered(slots: &[OutputSlot]) -> Vec<String> {
        slots
            .iter()
            .map(|s| match s {
                OutputSlot::Row(r) => format!("{:?}", r),
                OutputSlot::Dummy => "Dummy".to_string(),
            })
            .collect()
    }

    /// The revealed slot ORDER is the strong pin: it is the shuffle's permutation,
    /// which moves if the stage consumes more, fewer or differently ordered draws
    /// before the shuffle samples it. The WITNESS draw after the run pins the total
    /// draw count.
    ///
    /// Re-captured on the current code: the per-element degree reduction draws
    /// `2t + 1` re-sharings per selected element, and the padded width is now the
    /// DECLARED width rather than the widest term present, so each slot packs 20
    /// limbs instead of 2. The shuffle therefore samples a later point of the
    /// keystream and the order and witness moved. The REVEALED MULTISET is
    /// unchanged — the same two real rows and three padding slots — while the
    /// width-scaled costs (limb selects, switches, degree reductions) grew with the
    /// declaration.
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
        // The switch count scales with the entry width `1 + limbs = 21`, so it is
        // the 8-switch network over five items at that width.
        assert_eq!(cost.shuffle.switches, 8 * 21);
        // Twenty payload limbs per slot, from the DECLARED 130-byte row: the tag
        // select is still one per slot, the limb select adds twenty.
        assert_eq!((cost.select_mults, cost.payload_select_mults), (5, 100));
        // One degree reduction per selected element: the tag plus its twenty limbs,
        // over five slots.
        assert_eq!(cost.degree_reductions, 105);
        assert_eq!(dealer.draw_fp().value(), 2_629_004_675_748_555);

        // The public entry point mints its own dealer from the same seeded
        // backend, so it reproduces the same run.
        let (public, _) = oblivious_set_output(&b, &cands, 1, 5).unwrap();
        assert_eq!(rendered(&public), rendered(&slots));
    }

    /// The COMPLETE seam transcript of the output stage — every sharing in
    /// consumption order, then the shuffled column's shares — folded into the
    /// order-sensitive fingerprint.
    ///
    /// This fixture is the output stage ALONE, so it has no pre-output-stage prefix
    /// to hold fixed: the per-element degree reduction re-shares through the seam,
    /// so the stage's own draw count and every draw after the first selector moved.
    /// The method list is still all `vss_own_input` then one `oblivious_shuffle`
    /// (the reduction draws through `vss_own_input` too), and `VSS_DRAWS` pins the
    /// exact count. The operator-level fixtures in `hidden_path`, `hidden_distinct`
    /// and `join` are where the prefix BEFORE the output stage is pinned unchanged.
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
        // Both fingerprints were re-captured when the padded width became the
        // DECLARED width: every draw's value is unchanged, but there are 20 payload
        // limbs per slot instead of the 2 the widest term present used to give, so
        // the prefix is longer and hashes differently. What the pin is for is
        // unchanged — a transcript that moves without a declared reason is a
        // regression.
        assert_eq!(
            fingerprint(prefix_before_first_shuffle(&rec.log)),
            0x7d24_d21b_50c1_7493
        );
        assert_eq!(fingerprint(&rec.log), 0xa3a6_7cb6_7305_af61);
    }

    /// Every sharing the output stage draws — selector, real tag, pure-dummy
    /// selector, payload limb, and every degree-reduction re-sharing — is
    /// shape-checked where it is consumed: corrupting exactly ONE draw must be
    /// refused by a shape check itself, either the stage's own
    /// `checked_vss_own_input` or the reduction's `require_canonical_sharing` on a
    /// re-sharing ([`ANY_SHAPE_REFUSAL`] covers both wordings).
    #[test]
    fn malformed_draws_are_refused_at_every_output_consumption_site() {
        let b = ShamirBackend::new_seeded(5, 0x0B17).unwrap();
        let cands = padded_candidates();
        for skip in 0..VSS_DRAWS {
            for how in ALL_MALFORMATIONS {
                let mut src = MalformedSource::new(b.dealer(), SeamMethod::VssOwnInput, skip, how);
                match oblivious_set_output_through(&mut src, &b, &cands, 1, 5) {
                    Err(MpcError::Protocol(m)) => assert!(
                        m.contains(ANY_SHAPE_REFUSAL),
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
            (9, 9, 9, 9, 21 * 21, 7)
        );
        // Each slot's payload packs the DECLARED row's 20 limbs, so the limb select
        // adds 20 multiplications per slot on top of the tag select, and the switch
        // count above is the 21-switch network at an entry width of `1 + 20`.
        assert_eq!(cost.payload_select_mults, 9 * 20);
        // One degree reduction per selected element: the tag plus its twenty limbs,
        // over nine slots.
        assert_eq!(cost.degree_reductions, 9 * 21);
        let (public, public_cost) = oblivious_set_output_hidden_keys(&b, &left, &right, 9).unwrap();
        assert_eq!(rendered(&public), rendered(&slots));
        assert_eq!(public_cost.shuffle.switches, cost.shuffle.switches);
    }

    /// The COMPLETE seam transcript of the hidden-key stage, folded into the
    /// order-sensitive fingerprint.
    ///
    /// **This is the #6117-step-6 guard for this path.** The log's tail is the
    /// output stage's 756 `vss_own_input` draws and its `oblivious_shuffle`. At
    /// `n = 3`, `t = 1` each degree reduction re-shares through `2t + 1 = 3`
    /// parties, so per slot the tag costs a real-tag sharing + 3 reduction
    /// re-sharings = 4 (the selector is the SHARED equality bit, which costs no
    /// draw) and each payload limb costs the limb sharing + 3 re-sharings = 4. The
    /// limb count is the DECLARED width's: arity 1 at `DEFAULT_PAYLOAD_TERM_BYTES`
    /// is a 130-byte row, so 20 limbs per slot whatever the term in it is, giving
    /// 84 per slot over nine slots. It was 12 per slot while the width came from
    /// the widest term present. If the stage went back to minting its own dealer
    /// for the output, the recorded log would stop at the equality chain and both
    /// the length and the tail assertion would fail.
    ///
    /// The 4293-entry PREFIX is the equality chain, which this change does not
    /// touch: it is pinned unchanged, so the whole move is in the output suffix.
    #[test]
    fn seeded_hidden_keys_transcript_is_pinned() {
        let (b, left, right) = hidden_key_fixture();
        let mut rec = RecordingSource::new(b.dealer());
        oblivious_set_output_hidden_keys_through(&mut rec, &b, &left, &right, 9).unwrap();
        assert_eq!(rec.log.len(), 4293 + 756 + 1);
        // The equality-chain prefix is untouched by the output stage's change, and
        // is pinned equal to its pre-change value: the declared width moved the
        // output suffix only.
        assert_eq!(fingerprint(&rec.log[..4293]), 0x94ae_8e7b_302d_0377);
        // The two whole-transcript fingerprints were re-captured with the declared
        // width: the draws' values are unchanged, there are simply 20 payload limbs
        // per slot instead of 2.
        assert_eq!(
            fingerprint(prefix_before_first_shuffle(&rec.log)),
            0x81ca_7a32_6544_9d4e
        );
        assert_eq!(fingerprint(&rec.log), 0x87ea_2485_ea80_16fd);
        let tail: Vec<&str> = rec.log[4293..].iter().map(|(m, _)| *m).collect();
        assert_eq!(
            tail,
            [["vss_own_input"; 756].as_slice(), &["oblivious_shuffle"]].concat()
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

    /// The output-stage refusals carry no opened or decoded value.
    ///
    /// Two sites: the opened slot TAG (a reconstructed quantity) and the payload
    /// decode (the joined row's own content). Both use a distinctive value and
    /// assert it appears in neither `Display` nor `Debug` of the error.
    #[test]
    fn output_stage_refusals_render_no_opened_or_decoded_value() {
        // (a) The opened tag. A selector sharing of a distinctive non-{0,1} value
        // makes this slot's tag open to it.
        let b = ShamirBackend::new_seeded(5, 0xE6).unwrap();
        let mut dealer = b.dealer();
        let odd_tag = dealer.share(Fp::new(313_131));
        let cands = vec![
            Candidate {
                payload: vec![lit("fine")],
                matched: MatchBit::Public(true),
            },
            Candidate {
                payload: vec![lit("malformed-selector")],
                matched: MatchBit::SecretShared(odd_tag),
            },
        ];
        let err = oblivious_set_output_through(&mut dealer, &b, &cands, 1, 5).unwrap_err();
        let text = format!("{} || {:?}", err, err);
        assert!(
            !text.contains("313131"),
            "the opened tag must not be rendered: {}",
            text
        );
        assert!(
            text.contains("neither REAL_TAG"),
            "the two permitted PUBLIC constants are still named: {}",
            text
        );

        // (b) The payload decode. `flag = 1` then a NUL-terminated string that is
        // not a term: the string IS the payload, so it must not be echoed.
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(b"PATIENT-575757");
        bytes.push(0);
        let err = decode_payload_row(&bytes, 1).unwrap_err();
        let text = format!("{} || {:?}", err, err);
        assert!(
            !text.contains("575757") && !text.contains("PATIENT"),
            "the undecodable payload must not be rendered: {}",
            text
        );
        assert!(
            text.contains("column 0") && text.contains("did not re-parse"),
            "the column position is still named: {}",
            text
        );

        // The non-UTF-8 branch of the same column, likewise.
        let err = decode_payload_row(&[1u8, 0xff, 0xfe, 0x00], 1).unwrap_err();
        let text = format!("{} || {:?}", err, err);
        assert!(
            !text.contains("255") && !text.contains("xff") && !text.contains("\\xff"),
            "the offending bytes must not be rendered: {}",
            text
        );
        assert!(text.contains("not valid UTF-8"), "{}", text);
    }

    /// The bound contract tries the PUBLIC half first, and the data-dependent count
    /// is not even computed when the public half settles it.
    ///
    /// The closure stands in for the caller's exact projection: it panics if
    /// called, so "accepted without computing the count" is asserted by the test
    /// completing rather than by inspecting anything after the fact.
    #[test]
    fn a_bound_at_or_above_the_public_bound_never_computes_the_exact_count() {
        use std::cell::Cell;

        let called = Cell::new(0usize);
        let counting = |n: usize| {
            let called = &called;
            move || {
                called.set(called.get() + 1);
                Ok(n)
            }
        };

        // B above, and B exactly at, the public bound: accepted, closure untouched.
        for bound in [8usize, 7] {
            check_bound_public_first("t", bound, 7, || {
                panic!("the exact count must not be computed when B covers the public bound")
            })
            .expect("a B at or above the public bound is accepted");
        }

        // B below the public bound: the count decides, and is computed exactly once.
        check_bound_public_first("t", 5, 7, counting(5))
            .expect("a tighter B is still accepted when the count fits");
        assert_eq!(
            called.get(),
            1,
            "the count is computed once, on this branch only"
        );

        let err = check_bound_public_first("t", 5, 7, counting(6))
            .expect_err("a tighter B that cannot cover the count is refused");
        assert_eq!(called.get(), 2);
        let text = format!("{} || {:?}", err, err);
        assert!(
            text.contains("B=5") && text.contains("public candidate bound 7"),
            "{}",
            text
        );
        // The count that decided it is NOT in the message.
        assert!(
            !text.contains("count 6") && !text.contains(" 6 "),
            "{}",
            text
        );

        // The cap is still checked first, before either branch.
        assert!(check_bound_public_first("t", MAX_OUTPUT_BOUND + 1, 1, || Ok(0)).is_err());

        // A count above the cap needs no branch of its own: B is capped, so it
        // cannot cover such a count and falls out of the same refusal.
        assert!(
            check_bound_public_first("t", MAX_OUTPUT_BOUND, MAX_OUTPUT_BOUND + 1, || Ok(
                MAX_OUTPUT_BOUND + 1
            ))
            .is_err(),
            "a count above the cap is refused"
        );
    }
}
