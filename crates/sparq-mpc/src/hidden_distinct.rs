// sq-py8h.4 — hidden-key endpoint DISTINCT (the gated sub-piece of the
// bounded property-path operator): collapse duplicate *secret* endpoint pairs.
//! Hidden-key endpoint **DISTINCT**: collapse duplicate *secret* endpoint pairs
//! `(?a, ?b)` — where BOTH endpoints are private secret-shared join keys — into a
//! set, **without ever opening a key** (design
//! `research/mpc-bounded-property-path-design.md` §2.5, §3 "the one genuinely gated
//! sub-piece", §6 step 4).
//!
//! ## What this is — and how it differs from the OR-fold dedup of sq-py8h.3
//!
//! The bounded property-path operator returns a *set* of endpoint pairs. Two
//! distinct dedup obligations arise, and they are NOT the same problem:
//!
//! 1. **Cross-length / cross-branch multiplicity** — one endpoint pair reached by a
//!    2-hop AND a 4-hop chain (or by two alternation branches). [`crate::hidden_path`]
//!    already collapses this by **OR-folding** the per-chain connected-bits per
//!    *disclosed* endpoint key (`endpoint_key` over the disclosed `Term`s). That is
//!    sound and shippable today and is the headline federation case (the endpoints
//!    are the disclosed result).
//!
//! 2. **Duplicate *secret* endpoint pairs** — the SAME pair arriving on several
//!    input rows where the endpoints are themselves PRIVATE keys (`?a`, `?b` are not
//!    disclosed global IRIs but secret-shared `F_p` values). Grouping by a disclosed
//!    term is impossible here — there is no disclosed key to group on — so collapsing
//!    the duplicates requires deciding key-equality **in MPC**. This module is that
//!    gated piece (it was blocked on the secure comparator `sq-rrz4` → degree
//!    reduction `sq-dvuc`, both now landed).
//!
//! ## The construction (design §2.5 / §3, realised against landed primitives)
//!
//! Given `N` candidate rows, each carrying a secret-shared composite key
//! `(a_key, b_key)` (both `< 2^`[`crate::compare::COMPARE_BITS`]) and the
//! disclosed-on-output endpoint terms `(a_term, b_term)`:
//!
//! 1. **Oblivious sort by the secret composite key.** Run the data-independent
//!    [`crate::oblivious::SortingNetwork`] (Batcher odd-even mergesort — its
//!    compare-exchange access pattern is a function of `N` only, the obliviousness
//!    substrate). At each compare-exchange `(i, j)` we decide the swap with a
//!    **secret-shared** comparator: `swap = lex_greater((a_i,b_i), (a_j,b_j))` built
//!    from [`crate::compare::secure_greater_than`] +
//!    [`crate::compare::secure_equal_to_bit`] (lexicographic on `a` then `b`) — a
//!    fresh degree-`t` 0/1 sharing that is **NEVER opened**. The two rows' key limbs
//!    are then swapped by the arithmetic **conditional swap** `x' = x + swap·(y − x)`,
//!    so the *order* of the comparison verdicts is never revealed. The disclosed
//!    payload terms ride co-indexed through the same swap (the simulation holds them
//!    in the clear exactly as [`crate::oblivious_join::oblivious_set_output`] does;
//!    the permutation is hidden because every swap bit is secret and the FINAL output
//!    re-shuffles before any reveal — so the sorted intermediate order is never
//!    opened).
//!
//! 2. **Adjacent-equality scan → secret keep-bit.** After the sort, equal-key rows
//!    are contiguous. For each row `r > 0` we compute `dup_r = [key_r == key_{r−1}]`
//!    (one [`crate::compare::secure_equal_to_bit`] per key limb, ANDed) — a
//!    secret-shared bit, never opened — and set `keep_r = 1 − dup_r` (the first
//!    occurrence of each distinct key has `keep = 1`; every later duplicate has
//!    `keep = 0`). Row 0 always keeps. This is the classic *adjacent-equality* dedup
//!    a sorted run admits, lifted into MPC.
//!
//! 3. **Oblivious compaction.** Each sorted row becomes a
//!    [`crate::oblivious_join::Candidate`] whose disclosed payload is `(a_term,
//!    b_term)` and whose [`crate::oblivious_join::MatchBit::SecretShared`] selector
//!    is `keep_r`. Feeding them to [`crate::oblivious_join::oblivious_set_output`]
//!    with a public padded bound `B` performs the oblivious select (`tag = keep ·
//!    real_tag`, one product, no open), re-shuffles the slots, and reveals exactly
//!    `B` shuffled slots: a duplicate row (`keep = 0`) opens to a dummy and is
//!    filtered, so each distinct secret pair survives **once**.
//!
//! ## What stays hidden / what is disclosed (state it precisely)
//!
//! - **Hidden:** every endpoint join KEY (`a_key`, `b_key` — never reconstructed);
//!   every per-comparison swap bit and every adjacent-equality `dup` bit
//!   (secret-shared, never opened); WHICH rows were duplicates of which (the
//!   multiplicity structure); the input→output position linkage (the final shuffle
//!   destroys it).
//! - **Disclosed:** the row count `N` and the public padded bound `B` (the sort
//!   network width and the output slot count are public, the standard MPC
//!   assumption); the endpoint TERMS of the surviving distinct pairs (the result the
//!   recipient asked for). The true DISTINCT COUNT is bounded by `B`, and whether it
//!   is also disclosed depends on the output mode: this stage uses the ALL-PARTY
//!   mode, where every compute party opens the `B` tags and therefore learns the
//!   exact count; the recipient-only mode
//!   ([`crate::oblivious_join::oblivious_set_output_to_recipient`]) keeps it out of
//!   their transcript, and hiding it from the recipient too is the separate
//!   DP-cardinality mode (bead sq-shk5).
//!
//! ## Security tier (HONESTY — privacy-claims gate, cite sq-qhy4)
//!
//! Honest-majority, semi-honest ONLY — inherits the [`crate::shamir::ShamirBackend`]
//! / [`crate::compare`] / [`crate::oblivious_join`] model and adds **no new
//! assumption**. Every comparison, equality and conditional swap routes through
//! [`crate::shamir::degree_reduce_through`], whose reshare has no in-protocol
//! check that a deviating party reshared honestly, so this is **NOT** maliciously
//! secure (the same degree-`2t`-at-`n=2t+1` residual the rest of the backend
//! documents; sq-6d6g / sq-km34 IT-MAC is the named fix). The confidentiality this
//! closes (collapsing secret duplicates without opening keys) is a CONFIDENTIALITY
//! axis, orthogonal to malicious security; external soundness sign-off is still
//! pending (sq-qhy4). No guarantee beyond the documented semi-honest model is
//! claimed.
//!
//! ## Cost (MODELLED — counted, not measured)
//!
//! The sort is `O(N log² N)` compare-exchanges, each a lexicographic secure compare
//! (a bit-decomposition + AND-tree, the cost [`crate::compare::secure_greater_than`]
//! pays) plus a constant-limb conditional swap; the adjacent scan is `N − 1` secure
//! equalities; the compaction is the `B`-slot [`crate::oblivious_join`] cost. The
//! returned [`DistinctCost`] reports these counts (no hard-coded wall-clock).
//!
//! ## Randomness (sq-yyro, issue #6117)
//!
//! The sort-and-scan stage draws every sharing through the
//! [`crate::randomness::DistributedRandomness`] seam: the key sharings (`a_key`
//! then `b_key`, row by row) via `vss_own_input`, every conditional swap and AND
//! via [`crate::shamir::degree_reduce_through`], and every comparison / equality
//! via the [`crate::compare`] `*_through` chain. The single-dealer simulation's
//! sort-and-scan draws are unchanged from the pre-seam code (a seeded prefix pin
//! test); a source that cannot share a holder's own input (PRSS today) makes any
//! DISTINCT fail closed. The
//! final oblivious output stage draws from the SAME source
//! (`oblivious_join::oblivious_set_output_through`), so one continuous
//! source covers the whole operator — and because that stage always shares the
//! real-row tag and the payload limbs, even an EMPTY DISTINCT now fails closed
//! under PRSS.

use crate::compare::{
    secure_equal_to_bit_through, secure_greater_than_through, COMPARE_MAX_EXCLUSIVE,
};
use crate::field::Fp;
use crate::oblivious::SortingNetwork;
use crate::oblivious_join::{
    oblivious_set_output_through, Candidate, MatchBit, ObliviousOutput, OutputSlot, PayloadWidths,
};
use crate::partial::{HolderId, MpcError, PartialResult};
use crate::randomness::{checked_vss_own_input, require_source_matches, DistributedRandomness};
use crate::shamir::{self, ShamirBackend, Share};
use oxrdf::{Term, Variable};

/// Reserved subject-end / object-end variable names the operator projects onto
/// (matching [`crate::hidden_path`]).
const SUBJECT_VAR: &str = "__pp_a";
const OBJECT_VAR: &str = "__pp_b";

/// Maximum number of input rows the DISTINCT will accept before refusing with a
/// [`MpcError::Protocol`]. The oblivious sort is `O(N log² N)` secure comparisons,
/// each a bit-decomposition + AND-tree, so an unbounded `N` is a CPU/round
/// denial-of-service on this surface. `N` is PUBLIC (the sort-network width is
/// public), so the guard is a closed-form check before any crypto runs. A protocol
/// guard, not a tuning knob.
pub const MAX_DISTINCT_ROWS: usize = 1 << 12; // 4096

/// One candidate endpoint pair whose endpoints are **secret** join keys. The keys
/// are compared in MPC and never reconstructed; the terms are the disclosed-on-
/// output payload (convention #4 — the path result is the disclosed binding). Both
/// keys must be `< 2^`[`crate::compare::COMPARE_BITS`] (fail-closed, so the secure
/// comparator's bit-decomposition is injective).
/// `Debug` is hand-written: both keys are private, and both terms are disclosed
/// only if the pair survives the DISTINCT — a `Debug` rendering is not that
/// condition. All four fields are redacted; they stay public for the callers that
/// legitimately read them.
#[derive(Clone)]
pub struct SecretEndpointPair {
    /// The subject endpoint's private join key (compared, never reconstructed).
    pub a_key: Fp,
    /// The object endpoint's private join key (compared, never reconstructed).
    pub b_key: Fp,
    /// The subject endpoint's disclosed term (emitted iff this pair survives).
    pub a_term: Term,
    /// The object endpoint's disclosed term (emitted iff this pair survives).
    pub b_term: Term,
}

impl std::fmt::Debug for SecretEndpointPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretEndpointPair")
            .field("a_key", &"<redacted>")
            .field("b_key", &"<redacted>")
            .field("a_term", &"<redacted>")
            .field("b_term", &"<redacted>")
            .finish()
    }
}

/// MODELLED cost of one hidden-key DISTINCT (counted, never measured — the crate is
/// an in-process simulation). No hard-coded numbers: derived from `N` and `B`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DistinctCost {
    /// Input rows `N`.
    pub rows: usize,
    /// Sort-network compare-exchange gates (= secure lexicographic comparisons +
    /// conditional swaps the sort pays).
    pub sort_compare_exchanges: usize,
    /// Adjacent-equality secure equalities (`N − 1`).
    pub adjacent_equalities: usize,
    /// The padded output bound `B` (slots revealed by the compaction).
    pub output_bound: usize,
}

/// A secret-shared row carried through the oblivious sort: the two secret key limbs
/// (degree-`t` sharings) plus the disclosed payload terms co-indexed. The payload
/// rides the conditional swap in the clear (the simulation plays all parties), the
/// keys ride as sharings; the swap BIT is always secret, so the permutation is never
/// revealed (mirrors the cleartext-payload co-permutation of
/// [`crate::oblivious_join::oblivious_set_output`]).
struct SortRow {
    a_key: Vec<Share>,
    b_key: Vec<Share>,
    a_term: Term,
    b_term: Term,
}

/// Conditional swap of two sharings under a secret 0/1 control bit: returns
/// `(x', y')` where `x' = x + bit·(y − x)` and `y' = y − bit·(y − x)`. When
/// `bit = 0` the pair is unchanged; when `bit = 1` it is swapped. One
/// [`shamir::mul_shares_raw`] + [`shamir::degree_reduce_through`] for the shared
/// `bit·(y − x)` term, reused for both sides; nothing is opened.
fn conditional_swap(
    source: &mut dyn DistributedRandomness,
    bit: &[Share],
    x: &[Share],
    y: &[Share],
) -> Result<(Vec<Share>, Vec<Share>), MpcError> {
    let y_minus_x = shamir::sub_shares(y, x)?;
    let delta_2t = shamir::mul_shares_raw(bit, &y_minus_x)?; // bit·(y − x), degree 2t
    let delta = shamir::degree_reduce_through(source, &delta_2t)?; // back to degree t
    let x_new = shamir::add_shares(x, &delta)?; // x + bit·(y − x)
    let y_new = shamir::sub_shares(y, &delta)?; // y − bit·(y − x)
    Ok((x_new, y_new))
}

/// Secret AND of two 0/1 sharings: `[a ∧ b] = [a]·[b]` (one mul + degree reduce).
/// Local mirror of `compare.rs`'s `secret_and` (file-lane discipline).
fn secret_and(
    source: &mut dyn DistributedRandomness,
    a: &[Share],
    b: &[Share],
) -> Result<Vec<Share>, MpcError> {
    let prod_2t = shamir::mul_shares_raw(a, b)?;
    shamir::degree_reduce_through(source, &prod_2t)
}

/// Secret-shared lexicographic strict-greater on the composite key `(a, b)`:
/// `(a_i, b_i) > (a_j, b_j)` iff `a_i > a_j`, OR `a_i == a_j` AND `b_i > b_j`. Built
/// from [`crate::compare::secure_greater_than`] +
/// [`crate::compare::secure_equal_to_bit`] (their seam-drawing `*_through` forms);
/// the result is a fresh degree-`t` 0/1 sharing that is **never opened** (it drives
/// the conditional swap arithmetically).
fn lex_greater_bit(
    source: &mut dyn DistributedRandomness,
    a_i: Fp,
    b_i: Fp,
    a_j: Fp,
    b_j: Fp,
) -> Result<Vec<Share>, MpcError> {
    let a_gt = secure_greater_than_through(source, a_i, a_j)?; // [a_i > a_j]
    let a_eq = secure_equal_to_bit_through(source, a_i, a_j)?; // [a_i == a_j]
    let b_gt = secure_greater_than_through(source, b_i, b_j)?; // [b_i > b_j]

    // tie_break = [a_i == a_j] ∧ [b_i > b_j]
    let tie_break = secret_and(source, &a_eq, &b_gt)?;
    // `a_gt` and `tie_break` are mutually exclusive (`tie_break ⇒ a_i == a_j ⇒
    // ¬(a_i > a_j)`), so their disjunction is their SUM (no overlap to subtract);
    // both are 0/1 so the sum is 0/1.
    shamir::add_shares(&a_gt, &tie_break)
}

/// Secret-shared equality on the composite key `(a, b)`: `(a_i, b_i) == (a_j, b_j)`
/// iff `a_i == a_j` AND `b_i == b_j`. Two [`crate::compare::secure_equal_to_bit`]
/// (seam-drawing form) + one [`secret_and`]; never opened.
fn lex_equal_bit(
    source: &mut dyn DistributedRandomness,
    a_i: Fp,
    b_i: Fp,
    a_j: Fp,
    b_j: Fp,
) -> Result<Vec<Share>, MpcError> {
    let a_eq = secure_equal_to_bit_through(source, a_i, a_j)?;
    let b_eq = secure_equal_to_bit_through(source, b_i, b_j)?;
    secret_and(source, &a_eq, &b_eq)
}

/// Reconstruct the plaintext composite key of a [`SortRow`] for the secret
/// comparator's bit-decomposition inputs. The simulation plays ALL parties, so it
/// holds the cleartext exactly as [`crate::shamir::ShamirDealer::share`] and the
/// secure comparator already do — nothing is revealed OUTSIDE the simulation (the
/// verdict it produces is a sharing that is never opened). Returns `(a, b)` as
/// field elements.
fn row_key(backend: &ShamirBackend, row: &SortRow) -> Result<(Fp, Fp), MpcError> {
    Ok((
        backend.reconstruct(&row.a_key)?,
        backend.reconstruct(&row.b_key)?,
    ))
}

/// `1 − x` for a 0/1 sharing (local affine NOT): negate then add the constant 1.
fn sub_from_one(x: &[Share]) -> Vec<Share> {
    let neg = shamir::scale(x, Fp::zero().sub(Fp::one()));
    shamir::add_constant(&neg, Fp::one())
}

/// Degree-`t` sharing of a PUBLIC constant `c` on points `1..=n` (the constant
/// polynomial `f(x) = c`). Local mirror of `compare.rs`'s `const_sharing`.
fn const_sharing(n: usize, c: Fp) -> Vec<Share> {
    (1..=n as u64).map(|x| Share { x, y: c }).collect()
}

/// Fail-closed parameter checks: `n >= 2t+1` (the conditional swap / AND / compare
/// all need the degree-reduction headroom), `N` within [`MAX_DISTINCT_ROWS`],
/// `bound >= N` (never truncate a distinct pair), and every key `< 2^COMPARE_BITS`.
/// All PUBLIC / statically checkable before any crypto runs.
fn check_params(
    backend: &ShamirBackend,
    pairs: &[SecretEndpointPair],
    bound: usize,
) -> Result<(), MpcError> {
    let t = backend.threshold();
    if backend.parties() < 2 * t + 1 {
        return Err(MpcError::Protocol(format!(
            "hidden-key DISTINCT needs n >= 2t+1 for the conditional-swap / compare degree \
             reduction (n={}, t={t})",
            backend.parties()
        )));
    }
    if pairs.len() > MAX_DISTINCT_ROWS {
        return Err(MpcError::Protocol(format!(
            "hidden-key DISTINCT over N={} rows exceeds the MAX_DISTINCT_ROWS cap of {} — \
             refusing (the O(N log² N) oblivious sort would be a CPU/round denial-of-service)",
            pairs.len(),
            MAX_DISTINCT_ROWS
        )));
    }
    // The guarantee is: BEFORE ANY DRAW OR ALLOCATION, not "first". The checks
    // above it are parameter and shape refusals that draw nothing and allocate
    // nothing, so they keep precedence deliberately; what matters is that no draw
    // and no `B`-sized allocation can precede this. The sort-and-scan draws per
    // comparison and the output stage materialises `B` slots.
    crate::oblivious_join::check_output_bound_cap(bound)?;
    if bound < pairs.len() {
        return Err(MpcError::Protocol(format!(
            "hidden-key DISTINCT: padded bound B={bound} < row count {} — would truncate \
             candidates and could drop a distinct pair",
            pairs.len()
        )));
    }
    for (k, p) in pairs.iter().enumerate() {
        if p.a_key.value() >= COMPARE_MAX_EXCLUSIVE || p.b_key.value() >= COMPARE_MAX_EXCLUSIVE {
            return Err(MpcError::Protocol(format!(
                "hidden-key DISTINCT: row {k} has a key >= 2^COMPARE_BITS ({COMPARE_MAX_EXCLUSIVE}) \
                 — out of the secure comparator's injective range",
            )));
        }
    }
    Ok(())
}

/// Oblivious-sort the rows by their secret composite key (secret-control
/// compare-exchange — the swap bit is never opened), then run the adjacent-equality
/// scan so each distinct key's FIRST occurrence has `keep = 1` and every later
/// duplicate has `keep = 0`. Returns one [`Candidate`] per sorted row (payload
/// `(a_term, b_term)`, selector the secret keep-bit) plus the modelled gate counts.
///
/// Every sharing is drawn from `source`, which the caller has matched to
/// `backend` ([`require_source_matches`]).
fn build_distinct_candidates(
    backend: &ShamirBackend,
    source: &mut dyn DistributedRandomness,
    pairs: &[SecretEndpointPair],
) -> Result<(Vec<Candidate>, usize, usize), MpcError> {
    // The sort network's width BEFORE any key is shared: `MAX_DISTINCT_ROWS` is
    // below `MAX_NETWORK_WIDTH`, so a caller that came through `check_params`
    // cannot trip this — but this function takes `pairs` directly, and leaving the
    // width check until after the per-row sharings would mean a refusal that had
    // already drawn `2N` sharings. Checked here, it draws nothing.
    crate::oblivious::check_network_width("hidden-key DISTINCT sort", pairs.len())?;
    // Share each pair's keys (a_key then b_key, row by row — the holder's own
    // inputs, shape-checked); carry the disclosed terms co-indexed.
    let n = backend.parties();
    let mut rows: Vec<SortRow> = Vec::with_capacity(pairs.len());
    for p in pairs {
        let a_key = checked_vss_own_input(source, n, p.a_key)?;
        let b_key = checked_vss_own_input(source, n, p.b_key)?;
        rows.push(SortRow {
            a_key,
            b_key,
            a_term: p.a_term.clone(),
            b_term: p.b_term.clone(),
        });
    }

    // --- Oblivious sort by the secret composite key. ---
    // Width already refused above, before any sharing was drawn; `try_new` is kept
    // as the only public constructor, and here it cannot fail.
    let net = SortingNetwork::try_new(rows.len())?;
    let mut compare_exchanges = 0usize;
    for &(i, j) in net.compare_exchanges() {
        // Plaintext composite keys (held by the simulation as the dealer; the secure
        // comparator below shares them and never opens its verdict).
        let (ai, bi) = row_key(backend, &rows[i])?;
        let (aj, bj) = row_key(backend, &rows[j])?;

        // Secret swap bit: swap iff row[i] > row[j] lexicographically (so the column
        // ends ASCENDING). A fresh degree-`t` 0/1 sharing — NEVER opened; it drives
        // the arithmetic conditional swap of the key limbs below.
        let swap = lex_greater_bit(source, ai, bi, aj, bj)?;
        let (a_i_new, a_j_new) = conditional_swap(source, &swap, &rows[i].a_key, &rows[j].a_key)?;
        let (b_i_new, b_j_new) = conditional_swap(source, &swap, &rows[i].b_key, &rows[j].b_key)?;
        rows[i].a_key = a_i_new;
        rows[j].a_key = a_j_new;
        rows[i].b_key = b_i_new;
        rows[j].b_key = b_j_new;

        // Co-permute the disclosed payload terms by the SAME swap. In a real
        // deployment the terms would be secret-shared limbs swapped by the secret
        // `swap` arithmetically; in this in-process simulation they are cleartext and
        // relabelled when the swap fires. The swap value equals the lexicographic
        // predicate the secure comparator decided, recomputed locally from the
        // cleartext the simulation already holds — the secret verdict is NOT opened.
        if (ai.value(), bi.value()) > (aj.value(), bj.value()) {
            let (ta, tb) = (rows[i].a_term.clone(), rows[i].b_term.clone());
            rows[i].a_term = rows[j].a_term.clone();
            rows[i].b_term = rows[j].b_term.clone();
            rows[j].a_term = ta;
            rows[j].b_term = tb;
        }
        compare_exchanges += 1;
    }

    // --- Adjacent-equality scan → secret keep-bit. ---
    let mut adjacent_equalities = 0usize;
    let mut candidates: Vec<Candidate> = Vec::with_capacity(rows.len());
    for r in 0..rows.len() {
        let keep = if r == 0 {
            // Row 0 always keeps: the constant shared 1.
            const_sharing(n, Fp::one())
        } else {
            let (a_r, b_r) = row_key(backend, &rows[r])?;
            let (a_p, b_p) = row_key(backend, &rows[r - 1])?;
            let dup = lex_equal_bit(source, a_r, b_r, a_p, b_p)?;
            adjacent_equalities += 1;
            sub_from_one(&dup) // keep = 1 − dup
        };
        candidates.push(Candidate {
            payload: vec![Some(rows[r].a_term.clone()), Some(rows[r].b_term.clone())],
            matched: MatchBit::SecretShared(keep),
        });
    }

    Ok((candidates, compare_exchanges, adjacent_equalities))
}

/// Evaluate the hidden-key endpoint **DISTINCT** over `pairs` and return the
/// disclosed distinct endpoint terms as a [`PartialResult`] with
/// `vars == [?__pp_a, ?__pp_b]` (real rows only, dummies filtered, in canonical
/// order — the SET result is order-independent). Duplicate *secret* pairs are
/// collapsed without opening any key; `bound` is the public padded `B` (must cover
/// `pairs.len()` — fail-closed).
///
/// See the module docs for the full construction and the (semi-honest,
/// honest-majority) security tier.
pub fn distinct_hidden_pairs(
    backend: &ShamirBackend,
    pairs: &[SecretEndpointPair],
    bound: usize,
) -> Result<PartialResult, MpcError> {
    let (slots, _cost) = distinct_hidden_pairs_slots(backend, pairs, bound)?;
    let mut rows: Vec<Vec<Option<Term>>> = slots
        .into_iter()
        .filter_map(|s| match s {
            OutputSlot::Row(p) => Some(p),
            OutputSlot::Dummy => None,
        })
        .collect();
    // Canonicalise the disclosed multiset so the result is order-independent (the
    // shuffle order is not load-bearing for the SET result).
    rows.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    Ok(PartialResult {
        holder: HolderId::new("federation"),
        vars: vec![
            Variable::new_unchecked(SUBJECT_VAR),
            Variable::new_unchecked(OBJECT_VAR),
        ],
        rows,
    })
}

/// Like [`distinct_hidden_pairs`] but returns the raw oblivious-output slots
/// (`B` shuffled slots) + the modelled [`DistinctCost`] — the transcript-level view
/// (witnesses that exactly `B` slots are revealed, L1 bounded to `B`).
pub fn distinct_hidden_pairs_slots(
    backend: &ShamirBackend,
    pairs: &[SecretEndpointPair],
    bound: usize,
) -> Result<(Vec<OutputSlot>, DistinctCost), MpcError> {
    distinct_hidden_pairs_slots_through(&mut backend.dealer(), backend, pairs, bound)
}

/// [`distinct_hidden_pairs_slots`] with the sort-and-scan stage's randomness drawn
/// through the [`DistributedRandomness`] seam (sq-yyro, issue #6117), and — since
/// #6117 step 6 — the output stage's too. Under the single-dealer simulation the
/// REVEALED multiset and the [`DistinctCost`] are unchanged from the pre-seam code,
/// and the sort-and-scan prefix of the draw transcript is pinned unchanged; the
/// revealed slot ORDER moved, since the shuffle now samples a different point of
/// one continuous keystream rather than a second dealer's fresh one.
/// A source whose `(n, t)` differs from `backend`'s is
/// refused ([`require_source_matches`]); a source that cannot share a holder's own
/// input (PRSS today) makes the DISTINCT fail closed with that source's error.
/// The oblivious output stage draws from the SAME `source`
/// ([`oblivious_set_output_through`]) — one continuous source for the whole
/// operator, where it previously minted a second dealer from `backend` and
/// restarted the keystream.
pub(crate) fn distinct_hidden_pairs_slots_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    pairs: &[SecretEndpointPair],
    bound: usize,
) -> Result<(Vec<OutputSlot>, DistinctCost), MpcError> {
    require_source_matches(source, backend)?;
    check_params(backend, pairs, bound)?;
    // The output stage's public-shape caps (declared widths, party count, aggregate
    // slot work) at THIS operator's entry, before a candidate is built or a share
    // drawn. They are closed forms on public numbers, so the answer is the same one
    // the stage gives; running them here is what makes "before any draw" true of the
    // operator and not only of the stage. The stage re-checks.
    let widths = PayloadWidths::from(2);
    // Resolve the widths over the INPUT pairs' terms — a superset of the terms the
    // candidates will carry — in one streaming pass that retains nothing, so the
    // width is fixed (derived mode) or the declaration is enforced (declared mode)
    // before the first draw.
    let resolved = widths.resolve(pairs.iter().flat_map(|p| [&p.a_term, &p.b_term]))?;
    crate::oblivious_join::check_output_public_shape(
        &resolved,
        bound,
        backend.parties(),
        backend.threshold(),
    )?;
    let (candidates, sort_ce, adj_eq) = build_distinct_candidates(backend, source, pairs)?;
    let (slots, _out_cost) =
        oblivious_set_output_through(source, backend, &candidates, widths, bound)?;
    let cost = DistinctCost {
        rows: pairs.len(),
        sort_compare_exchanges: sort_ce,
        adjacent_equalities: adj_eq,
        output_bound: bound,
    };
    Ok((slots, cost))
}

/// The raw oblivious output (`ObliviousOutput`-shaped: `B` slots + output cost), for
/// callers wiring this into the oblivious-output cost model.
///
/// Draws the sort-and-scan stage AND the output stage from ONE fresh dealer, so it
/// reveals exactly what [`distinct_hidden_pairs_slots`] reveals.
pub fn distinct_hidden_pairs_oblivious(
    backend: &ShamirBackend,
    pairs: &[SecretEndpointPair],
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    check_params(backend, pairs, bound)?;
    // The stage's public-shape caps before the build and before any draw; the stage
    // re-checks.
    let widths = PayloadWidths::from(2);
    // The same width pass over the input pairs' terms, before the dealer is minted.
    let resolved = widths.resolve(pairs.iter().flat_map(|p| [&p.a_term, &p.b_term]))?;
    crate::oblivious_join::check_output_public_shape(
        &resolved,
        bound,
        backend.parties(),
        backend.threshold(),
    )?;
    let mut dealer = backend.dealer();
    let (candidates, _ce, _eq) = build_distinct_candidates(backend, &mut dealer, pairs)?;
    oblivious_set_output_through(&mut dealer, backend, &candidates, widths, bound)
}

#[cfg(test)]
mod tests;
