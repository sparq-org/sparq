// [OPUS-4.8] sq-ujz8: ORQ-style oblivious sort-merge join over SECRET keys —
// replace the O(|L|·|R|) all-pairs `HiddenValueJoin` candidate enumeration with an
// O(n log² n) oblivious sort (n = |L|+|R|) + a linear segmented merge scan. Native-
// only; in-process multi-party simulation; communication cost MODELLED not measured.
// Authored under the Opus 4.8 fallback model — re-review when Fable returns.
//! Oblivious **sort-merge** join over private keys (bead sq-ujz8).
//!
//! ## Why this exists — the quadratic cost centre ORQ/Secrecy engineer away
//!
//! [`crate::join::HiddenValueJoin`] decides a hidden-key join by testing ONE
//! secret-shared equality **per candidate pair** — `O(|L|·|R|)` secure equalities
//! (`join.rs`, all three of `join` / `batched_join` / `fully_oblivious_batched_join`
//! enumerate every `(i, j)`). That all-pairs structure is exactly the quadratic
//! cost centre the relational-MPC line (Secrecy NSDI'23, ORQ SOSP'25 — see
//! `research/mpc-security-models-and-benchmarks.md` §2, `research/mpc-sparql-
//! capability-matrix.md` P6) replaces with a **sort-merge**: concatenate the two
//! sides into one relation of `n = |L|+|R|` rows, **obliviously sort by the join
//! key**, then a LINEAR scan over the sorted sequence decides the join — because
//! equal keys are adjacent after sorting, a row's matches are found among its
//! neighbours in its key-group, not by scanning the whole other side. Cost drops
//! from `O(|L|·|R|)` secure equalities to `O(n log² n)` compare-exchanges (the
//! Batcher network) + `O(n)` segmented-scan multiplications.
//!
//! This is distinct from the circuit-PSI approach (beads sq-h99 / sq-t21) per the
//! research synthesis — sort-merge is the operator that COMPOSES for multi-pattern
//! BGPs / sub-SELECTs, where circuit-PSI does not compose for free (matrix §2.5).
//!
//! ## What this module delivers (sound today) vs the honestly-scoped remainder
//!
//! Every primitive the ORQ operator needs has landed: the data-independent sort
//! NETWORK (`crate::oblivious::SortingNetwork`, sq-18lk), the degree-reduction round
//! (`ShamirDealer::degree_reduce`, sq-dvuc), the secure comparator
//! (`crate::compare::secure_greater_than`, sq-rrz4), and the oblivious shuffle
//! (`crate::oblivious::shuffle`, sq-18lk). The remaining integration step the
//! capability matrix names — *"the wired secret-key sort path"* — is built here:
//!
//! ### Delivered (sound, honest-majority / semi-honest)
//!
//! - [`oblivious_sort_by_secret_key`] — the **secret-key oblivious sort** substrate:
//!   the Batcher odd-even mergesort access pattern (data-independent, sq-18lk) driven
//!   by the landed P5 comparator ([`crate::compare::secure_greater_than_shared`], a
//!   secret verdict that is NEVER opened) and a **secret-bit-gated arithmetic
//!   conditional swap** `x' = x + b·(y−x)` (one secure multiplication per carried
//!   field element per switch). The only openings during the sort are the comparator's
//!   per-compare masked Rabbit decompositions (`c = x + r mod p`, near-uniform /
//!   statistically hiding at `≤ 2^{-61}` each — see the leakage statement); the verdict
//!   and the payload columns ride through the swaps as never-opened secret shares. This is the P6 sort the
//!   HIDDEN-key regime needs (the disclosed-key sort `sort_with_keys` was already
//!   sound — sq-18lk).
//! - [`sort_merge_semi_anti`] — the **semi-join and anti-join** (`MINUS` / `EXISTS` /
//!   `NOT EXISTS` family) via the sort-merge: sort the tagged union, a two-pass
//!   segmented scan computes each row's secret *"a row from the OTHER side shares my
//!   key"* bit, and the kept L rows are revealed through an oblivious shuffle +
//!   padded reveal (so which L rows matched is disclosed — that IS the result — but
//!   the keys, the R identities, and the sorted-key ORDER are not). Differential-
//!   tested against a plaintext semi-/anti-join oracle (the `minus_bindings`
//!   semantics for anti).
//!
//! ### Scoped remainder (NOT built here — honest, filed as follow-ups, not faked)
//!
//! - **Inner-join MATERIALISATION with many-to-many fan-out** and **left-outer with
//!   the matched R payload joined on** need the ORQ *expansion* step (an oblivious
//!   distribution of one side's payload across a key-group), which is a separate,
//!   larger protocol. Until it lands, inner-join materialisation is still served by
//!   the existing all-pairs [`crate::join::HiddenValueJoin::fully_oblivious_batched_join`]
//!   (correct, `O(|L|·|R|)`). This module deliberately does the fan-out-FREE family
//!   (semi / anti / existence-mark), which is the `O(n log n)` core.
//! - **Eager numeric aggregation** (per-key `COUNT`/`SUM` over the sorted groups via
//!   a segmented prefix-sum) reuses this module's sort + scan and is a natural
//!   follow-up on the same substrate.
//!
//! ## Privacy / leakage statement (empirical-honesty rule — state it loudly)
//!
//! - **The join KEY is never reconstructed — but the hiding is STATISTICAL, not
//!   information-theoretic.** The comparator and the adjacency-equality keep their
//!   VERDICT secret-shared (never opened) and the conditional swap opens nothing —
//!   HOWEVER each comparison/equality bit-decomposes its operands via the Rabbit path
//!   ([`crate::compare`]'s `secure_bit_decompose_rabbit`), whose ONE opening is a
//!   masked `c = (x + r) mod p` under a full-field mask `r` no party knows. That `c`
//!   is *near-uniform*, not perfectly uniform: it sits a statistical distance
//!   `≤ 2^{-61}` from uniform (the `p = 2^61 − 1` field-size floor — see the "Hiding"
//!   note on `secure_bit_decompose_rabbit`), so each opening leaks at most a `2^{-61}`
//!   advantage about its operand — NOT the perfect per-key independence a categorical
//!   "Shamir hiding" would assert. Each comparison / equality does this per operand
//!   TWICE: the masked decomposition open `c = x + r mod p`, and the in-protocol
//!   Rabbit range proof's masked zero-test open (`verify_value_in_range_rabbit`), each
//!   a `≤ 2^{-61}`-hiding open. **Composition:** one `sort_merge_semi_anti` therefore
//!   performs `Θ(|compare-exchanges| + n)` masked openings — a small constant per sort
//!   compare-exchange (`|compare-exchanges| = O(n log² n)`) and per adjacency equality
//!   (`n − 1` of them) — so by the standard hybrid / union-bound argument the total
//!   advantage to any `≤ t`-collusion composes to `≤ O(n log² n) · 2^{-61}`,
//!   cryptographically negligible for every realistic `n` but a *statistical* bound
//!   that must be composed, not a per-opening independence. (The Shamir SHARINGS of
//!   the keys are themselves perfectly `t`-private; the residual floor is the masked
//!   OPENINGS the comparator performs, not the sharings.)
//! - **The sorted-key ORDER is never revealed.** The final result is emitted through
//!   an oblivious [`crate::oblivious::shuffle`], so the position a revealed row came
//!   from — hence the key order — is destroyed before anything opens.
//! - **What IS revealed** (the result — plus the negligible masked-opening statistical
//!   floor noted above): for each kept L row, its
//!   *payload id* (which L row it is) is opened — that is the semi-/anti-join output
//!   the authorised recipient is entitled to. The number of kept rows (the result
//!   size) is learned by the recipient after filtering dummies, exactly as every
//!   other oblivious-output path in the crate (`oblivious_join`; the padded-bound
//!   scheme, NOT differential privacy — DP cardinality is the separate bead sq-shk5).
//!   `|L|`, `|R|` and `n` are public (standard MPC assumption; matrix §4.1).
//! - **Malicious security is unchanged** from the Shamir/`compare` layer: every
//!   multiplication routes through `degree_reduce`, which has no in-protocol check
//!   that a deviating party re-shared honestly — semi-honest, honest-majority only.
//!   External soundness sign-off is pending (sq-qhy4).
//!
//! ## Communication / round cost (MODELLED — counted, not measured)
//!
//! [`SortMergeCost`] is an interactive-OPERATION count of the modelled protocol,
//! derived from `n` (never a hard-coded number, AGENTS.md) — NOT a complete
//! deployment communication bill. What is counted, and at what granularity:
//!
//! - **Sort**: the `O(n log² n)` compare-exchanges and the network depth, at GATE
//!   granularity — each gate is one secure comparison + one conditional swap, and
//!   its interior Rabbit circuit is NOT expanded into per-gate
//!   multiplication/open counts here.
//! - **Segmented scan**: counted EXACTLY from the dealer's counters — BOTH the
//!   secure multiplications ([`SortMergeCost::scan_mults`]: each of the `n − 1`
//!   adjacency equalities contributes its full bit-decomposition circuit, not one
//!   multiplication per call) AND the interactive opens its Rabbit machinery
//!   performs ([`SortMergeCost::scan_opens`]: the square-protocol mask-bit opens,
//!   the masked decomposition opens, the range-proof zero-test opens).
//! - **Shuffle**: switch count/depth; **padded reveal**: its degree-`t` opens
//!   ([`SortMergeCost::opens`]).
//!
//! NOT modelled: the share-distribution traffic of dealing / re-sharing itself
//! (the in-process dealer deals each sharing locally; a real deployment pays
//! per-party messages for every sharing and degree-reduction round). The
//! in-process simulation's wall-clock is NOT an MPC latency.

use crate::compare::{
    check_party_count, secure_equal_to_bit_shared, secure_greater_than_shared,
    verify_shared_operand_in_range, COMPARE_MAX_EXCLUSIVE,
};
use crate::field::Fp;
use crate::oblivious::{shuffle, SortingNetwork};
use crate::partial::{HolderId, MpcError, PartialResult};
use crate::shamir::{self, ShamirBackend, ShamirDealer, Share};
use oxrdf::{Term, Variable};

/// Which fan-out-free sort-merge join this operator computes over the L side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMergeJoinKind {
    /// **Semi-join**: keep each L row that has AT LEAST ONE R row with the same key
    /// (`FILTER EXISTS` / the presence half of an inner join). The R payload is not
    /// joined on — only L rows are emitted (fan-out-free).
    Semi,
    /// **Anti-join**: keep each L row that has NO R row with the same key (`MINUS` /
    /// `FILTER NOT EXISTS`; the `minus_bindings` semantics on a single shared key).
    Anti,
}

/// MODELLED communication / round cost of one [`sort_merge_semi_anti`]. Counted,
/// never measured — and NOT a complete deployment bill: see the module cost note
/// for what each field counts, at what granularity, and what is not modelled at
/// all (share-distribution traffic; the sort's interior per-gate circuit). No
/// hard-coded numbers: derived from the union size and the built networks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortMergeCost {
    /// Union size `n = |L| + |R|` (public).
    pub union_size: usize,
    /// Compare-exchange gates in the oblivious sort (Batcher odd-even mergesort over
    /// `n`) — each is one secure comparison + one conditional swap.
    pub sort_compare_exchanges: usize,
    /// Sort network depth in comparator layers (≈ communication rounds).
    pub sort_depth_rounds: usize,
    /// Secure multiplications (interactive [`ShamirDealer::degree_reduce`] rounds)
    /// spent in the segmented merge scan: the adjacency equalities + the
    /// forward/backward OR passes + the per-row selector. [OPUS-5] COUNTED from the
    /// dealer's multiplication counter, not hand-tallied — each adjacency equality
    /// contributes its FULL circuit (two Rabbit bit-decompositions with their
    /// in-protocol range proofs, the per-bit equalities, and the balanced AND-tree
    /// — orders of magnitude more than one multiplication per call, and the exact
    /// count varies slightly per call with the public bits of the masked Rabbit
    /// open). Masked-product opens inside the Rabbit machinery (the solved-bits
    /// square protocol, the zero-tests) reconstruct a degree-`2t` product directly
    /// instead of re-sharing it: they are opens, not secure multiplications, and
    /// are counted separately in [`SortMergeCost::scan_opens`].
    pub scan_mults: usize,
    /// [OPUS-5] Interactive opens performed inside the segmented merge scan's
    /// Rabbit machinery — COUNTED from the dealer's open counter
    /// ([`crate::shamir::ShamirDealer`]), the same delta discipline as
    /// [`SortMergeCost::scan_mults`], whose count deliberately excludes them. Per
    /// adjacency equality this is one square-protocol `c = a²` open behind every
    /// jointly-random mask bit, the masked open `c = (x + r) mod p` of each of the
    /// two bit-decompositions, and the zero-test product open `m = v·r` of each
    /// in-protocol range proof. Reported separately so the scan's interactive work
    /// is fully accounted (multiplications AND opens), not silently dropped.
    pub scan_opens: usize,
    /// Degree-`t` opens performed at the padded reveal ONLY (one selector per
    /// revealed slot, plus the payload id of each kept row). The scan's interior
    /// Rabbit opens are in [`SortMergeCost::scan_opens`]; the sort's interior
    /// opens are not counted (gate-granularity model — module cost note).
    pub opens: usize,
    /// The final oblivious shuffle's cost over the `n` reveal slots.
    pub shuffle: crate::oblivious::ShuffleCost,
}

/// The result of a sort-merge semi-/anti-join: the oblivious [`PartialResult`] (the
/// kept L rows only, dummies filtered, federation-attributed, canonical order) plus
/// the MODELLED [`SortMergeCost`].
pub type SortMergeOutput = (PartialResult, SortMergeCost);

/// A test-only fault injected at ONE named decision point of the REAL secret-shared
/// [`sort_merge_semi_anti`] fast path, so a mutation test can prove the production
/// oracle differential is non-vacuous by corrupting the production wiring itself
/// (not merely a cleartext model of it). In non-test builds only [`ProdFault::None`]
/// is ever constructed — the public entry point hard-codes it — so the other variants
/// are dead there (annotated), and every branch below folds to the honest path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
enum ProdFault {
    /// No corruption — the honest production path.
    None,
    /// Gate the keep-selector on `is_r` instead of `is_l` (emit R rows, not L rows).
    Selector,
    /// Force every adjacency-equality bit to a secret 0 (equal keys never seen as a run).
    AdjacencyEquality,
    /// Drop the backward OR pass (an L row before its R match is no longer seen).
    ScanDirection,
    /// Force every oblivious-sort swap bit to a secret 0 (keys never become adjacent).
    SwapDecision,
}

/// A secret-shared union row as it travels through the oblivious sort: the join
/// KEY, a `1{is L-side}` flag, and the L-side PAYLOAD ID (which L row it is), each a
/// full degree-`t` sharing. All three ride through the conditional swaps together so
/// a row's `(key, side, id)` stay locked as it is permuted.
struct Row {
    key: Vec<Share>,
    is_l: Vec<Share>,
    /// The original L-row index for an L row; irrelevant (never revealed) for an R
    /// row, which always has `is_l = 0` so its selector is 0.
    pid: Vec<Share>,
}

/// **The oblivious sort-merge semi-/anti-join** (bead sq-ujz8).
///
/// Join the L side's private keys against the R side's private keys WITHOUT
/// revealing any key: keep each L row whose key is present ([`SortMergeJoinKind::Semi`])
/// or absent ([`SortMergeJoinKind::Anti`]) in the R key multiset. The output is the
/// kept L rows' disclosed payloads (schema `out_vars`), attributed to the synthetic
/// `federation` holder, in canonical order.
///
/// `left` is `(private_key, disclosed_payload)` per L row; `right_keys` is the R
/// side's private keys (R payloads are not emitted by a semi/anti-join, so R needs
/// only its keys). See the module docs for the delivered-vs-scoped split, the
/// leakage statement, and the honest-majority / semi-honest security model.
///
/// ## Fail-closed contracts
///
/// - Every key (both sides) must be `< 2^60` (`COMPARE_MAX_EXCLUSIVE`) so the
///   in-MPC bit-decomposition comparator is injective — a larger key is a
///   [`MpcError::Protocol`] error, never a silently-wrapped comparison.
/// - `n >= 2t + 1` for the secure multiplications (fail-closed inside the primitives).
/// - Every L payload must have arity `out_vars.len()` (uniform schema for the
///   recipient) — a short row is a malformed input, not a silent truncation.
pub fn sort_merge_semi_anti(
    backend: &ShamirBackend,
    left: &[(Fp, Vec<Option<Term>>)],
    right_keys: &[Fp],
    out_vars: Vec<Variable>,
    kind: SortMergeJoinKind,
) -> Result<SortMergeOutput, MpcError> {
    // The public entry point is always the honest path; the fault-injected variant is
    // exercised ONLY by the in-crate mutation test that proves this differential is
    // non-vacuous against the REAL secret-shared wiring.
    sort_merge_semi_anti_faulted(backend, left, right_keys, out_vars, kind, ProdFault::None)
}

/// [`sort_merge_semi_anti`] with a test-only [`ProdFault`] injected at one named
/// production decision point. `ProdFault::None` is the honest path — identical to what
/// the public wrapper computes; the non-`None` variants corrupt the REAL secret-shared
/// sort/scan/selector so a mutation test can assert the production output diverges from
/// the plaintext oracle (see `production_fast_path_mutations_diverge_from_oracle`).
fn sort_merge_semi_anti_faulted(
    backend: &ShamirBackend,
    left: &[(Fp, Vec<Option<Term>>)],
    right_keys: &[Fp],
    out_vars: Vec<Variable>,
    kind: SortMergeJoinKind,
    fault: ProdFault,
) -> Result<SortMergeOutput, MpcError> {
    let payload_arity = out_vars.len();
    // Validate payload arity + key range up front (fail closed before any protocol
    // work, so a malformed input never enters the crypto core).
    for (k, (key, pay)) in left.iter().enumerate() {
        if pay.len() != payload_arity {
            return Err(MpcError::Protocol(format!(
                "sort_merge_semi_anti: L row {k} has {} payload terms, expected arity {payload_arity}",
                pay.len()
            )));
        }
        check_key_range("L", k, *key)?;
    }
    for (k, key) in right_keys.iter().enumerate() {
        check_key_range("R", k, *key)?;
    }

    let ll = left.len();
    let rr = right_keys.len();
    let n = ll + rr;

    let mut dealer = backend.dealer();
    let t = dealer.threshold();
    let parties = dealer.parties();
    if parties < 2 * t + 1 {
        return Err(MpcError::Protocol(format!(
            "sort_merge_semi_anti needs n >= 2t+1 for the secure multiplications (n={parties}, t={t})"
        )));
    }

    // --- Build the tagged secret union in original order. `is_l`/`pid` start as
    // PUBLIC facts (position), shared as constants; the conditional swaps then mix
    // them into genuine secret combinations aligned to the (secret) sorted order. ---
    let mut rows: Vec<Row> = Vec::with_capacity(n);
    for (i, (key, _pay)) in left.iter().enumerate() {
        rows.push(Row {
            key: dealer.share(*key),
            is_l: dealer.share(Fp::one()),
            pid: dealer.share(Fp::new(i as u64)),
        });
    }
    for key in right_keys {
        rows.push(Row {
            key: dealer.share(*key),
            is_l: dealer.share(Fp::zero()),
            pid: dealer.share(Fp::zero()),
        });
    }

    // --- (1) Oblivious sort by the SECRET key (ascending). ---
    let net = SortingNetwork::new(n);
    for &(i, j) in net.compare_exchanges() {
        // Swap iff key_i > key_j (sort ascending) — a NEVER-opened secret verdict.
        let (lo, hi) = (i.min(j), i.max(j));
        let mut swap_bit =
            secure_greater_than_shared(&mut dealer, backend, &rows[lo].key, &rows[hi].key)?;
        if fault == ProdFault::SwapDecision {
            // Corrupt the swap decision to a stuck secret 0: no rows ever swap, so equal
            // keys across the L/R boundary never become adjacent (the sort is defeated).
            swap_bit = dealer.share(Fp::zero());
        }
        conditional_swap(&mut dealer, &mut rows, lo, hi, &swap_bit)?;
    }

    // --- (2) Segmented merge scan: per sorted position, the secret bit "a row from
    //         the OTHER side (an R row) shares my key-group". Equal keys are now
    //         adjacent, so a group is a maximal run of equal keys; we broadcast
    //         "the run contains an R row" to every member with a forward + backward
    //         OR pass gated by the adjacent-equal bit. ---
    // [OPUS-5] scan_mults / scan_opens are DELTAS of the dealer's counters, not
    // hand-kept tallies: each adjacency equality below is a full bit-decomposed
    // circuit (two Rabbit decompositions + range proofs + per-bit equalities + the
    // AND-tree), so tallying it as one multiplication per call — as this code once
    // did — understates the modelled cost by orders of magnitude; and its interior
    // Rabbit opens (mask bits, masked decompositions, zero-tests) are interactive
    // communication too, reported in scan_opens rather than dropped.
    let scan_mults_before = dealer.mult_count();
    let scan_opens_before = dealer.open_count();
    // Adjacent-key equalities eq[k] = [key_k == key_{k+1}] (secret), for k in 0..n-1.
    let mut eq: Vec<Vec<Share>> = Vec::with_capacity(n.saturating_sub(1));
    for k in 0..n.saturating_sub(1) {
        let mut e = secure_equal_to_bit_shared(&mut dealer, backend, &rows[k].key, &rows[k + 1].key)?;
        if fault == ProdFault::AdjacencyEquality {
            // Corrupt the adjacency-equality bit to a stuck secret 0: equal keys are
            // never recognised as a run, so no L row ever sees an R in its group.
            e = dealer.share(Fp::zero());
        }
        eq.push(e);
    }
    // is_r[k] = 1 - is_l[k] (local).
    let is_r: Vec<Vec<Share>> = rows
        .iter()
        .map(|r| shamir::add_constant(&shamir::scale(&r.is_l, Fp::one().neg()), Fp::one()))
        .collect();

    // Backward: b_r[k] = is_r[k] OR (eq[k] AND b_r[k+1]) — "R at or after k in run".
    let mut b_r: Vec<Vec<Share>> = vec![Vec::new(); n];
    if n > 0 {
        b_r[n - 1] = is_r[n - 1].clone();
        for k in (0..n - 1).rev() {
            let carried = secure_mul(&mut dealer, &eq[k], &b_r[k + 1])?;
            b_r[k] = secure_or(&mut dealer, &is_r[k], &carried)?;
        }
    }
    // Forward: f_r[k] = is_r[k] OR (eq[k-1] AND f_r[k-1]) — "R at or before k in run".
    let mut f_r: Vec<Vec<Share>> = vec![Vec::new(); n];
    if n > 0 {
        f_r[0] = is_r[0].clone();
        for k in 1..n {
            let carried = secure_mul(&mut dealer, &eq[k - 1], &f_r[k - 1])?;
            f_r[k] = secure_or(&mut dealer, &is_r[k], &carried)?;
        }
    }

    // --- (3) Per-position selector: keep this row iff it is an L row and the
    //         semi/anti condition on "any R in my run" holds. ---
    let mut selectors: Vec<Vec<Share>> = Vec::with_capacity(n);
    for k in 0..n {
        // any_r[k] = b_r[k] OR f_r[k].
        let any_r_full = secure_or(&mut dealer, &b_r[k], &f_r[k])?;
        let any_r = if fault == ProdFault::ScanDirection {
            // Drop the backward pass: an L row positioned before its matching R in the
            // sorted run no longer sees that R.
            f_r[k].clone()
        } else {
            any_r_full
        };
        let cond = match kind {
            SortMergeJoinKind::Semi => any_r,
            // NOT any_r = 1 - any_r (local).
            SortMergeJoinKind::Anti => {
                shamir::add_constant(&shamir::scale(&any_r, Fp::one().neg()), Fp::one())
            }
        };
        // selector = is_l AND cond (one secure mult) — corrupted to gate on is_r under
        // the Selector fault (emit R rows instead of L rows).
        let gate = if fault == ProdFault::Selector {
            &is_r[k]
        } else {
            &rows[k].is_l
        };
        selectors.push(secure_mul(&mut dealer, gate, &cond)?);
    }
    // The scan (adjacency equalities + OR passes + selectors) ends here; the shuffle
    // below reports its own cost, so snapshot the counter deltas now.
    let scan_mults = dealer.mult_count() - scan_mults_before;
    let scan_opens = dealer.open_count() - scan_opens_before;

    // --- (4) Oblivious reveal: pack (selector, pid) per position, oblivious-shuffle
    //         so the revealed order hides the sorted-key order, open the selector
    //         (0/1) per slot, and for a kept row open its pid → the L payload. ---
    let mut packed: Vec<Vec<Share>> = Vec::with_capacity(n);
    for (sel, row) in selectors.iter().zip(rows.iter()) {
        let mut entry = sel.clone();
        entry.extend_from_slice(&row.pid);
        packed.push(entry);
    }
    let (shuffled, shuffle_cost) = shuffle(&mut dealer, packed)?;

    let mut rows_out: Vec<Vec<Option<Term>>> = Vec::new();
    let mut opens = 0usize;
    for entry in &shuffled {
        // entry = [selector shares (parties) | pid shares (parties)].
        let (sel_part, pid_part) = entry.split_at(parties);
        let sel = backend.reconstruct(sel_part)?;
        opens += 1;
        if sel == Fp::one() {
            // A kept L row — reveal its payload id (this row IS in the result, so
            // disclosing which L row it is discloses only the result the recipient
            // gets). Map back to the L payload.
            let pid = backend.reconstruct(pid_part)?;
            opens += 1;
            let idx = pid.value() as usize;
            let payload = left
                .get(idx)
                .map(|(_, p)| p.clone())
                .ok_or_else(|| {
                    MpcError::Protocol(format!(
                        "sort_merge_semi_anti: revealed payload id {idx} out of range (|L|={ll}) \
                         — corrupted share slipped past the open"
                    ))
                })?;
            rows_out.push(payload);
        } else if sel == Fp::zero() {
            // A dropped L row or any R row — an indistinguishable dummy.
        } else {
            // A correct run opens a selector to exactly 0 or 1. Anything else means a
            // tampered/inconsistent share survived the open — refuse rather than
            // silently drop or emit a corrupted row.
            return Err(MpcError::Tampered {
                detail: format!(
                    "sort_merge_semi_anti: opened selector {} is neither 0 nor 1 — malformed \
                     scan or corrupted share; refusing rather than dropping/forging a row",
                    sel.value()
                ),
                cheaters: Vec::new(),
            });
        }
    }
    canonicalize_rows(&mut rows_out);

    let cost = SortMergeCost {
        union_size: n,
        sort_compare_exchanges: net.gate_count(),
        sort_depth_rounds: net.depth(),
        scan_mults,
        scan_opens,
        opens,
        shuffle: shuffle_cost,
    };
    Ok((
        PartialResult {
            holder: HolderId::new("federation"),
            vars: out_vars,
            rows: rows_out,
        },
        cost,
    ))
}

/// **The secret-key oblivious sort substrate** (the P6 sort the matrix names as the
/// remaining integration step). Sort a column of secret-shared field values ascending
/// via the data-independent Batcher odd-even mergesort access pattern
/// ([`SortingNetwork`], sq-18lk), each compare-exchange decided by the landed secure
/// comparator ([`secure_greater_than_shared`], a NEVER-opened secret verdict) and
/// applied by a **secret-bit-gated arithmetic conditional swap**. Returns the sorted
/// sharings — the sorted VALUES are never opened; the only openings are the
/// comparator's per-compare masked Rabbit decompositions (`c = x + r mod p`,
/// near-uniform / statistically hiding at `≤ 2^{-61}` each, composing over the
/// `O(n log² n)` gates — see the module leakage statement), so the caller holds a
/// re-ordered secret column whose order is STATISTICALLY hidden (not
/// information-theoretically independent) from any `≤ t` collusion.
///
/// This is exposed for the composed operators (and their differential tests) that
/// need only the sort — DISTINCT-over-hidden, ORDER BY, MIN/MAX, the sort-merge
/// join's own union sort.
///
/// ## Fail-closed contracts — enforced UP FRONT, independent of the gate count
///
/// - **`n >= 2t+1`** for the secure multiplications the comparator and the
///   conditional swap perform. Checked before any protocol work.
/// - **Every column value must be `< 2^60`** (`COMPARE_MAX_EXCLUSIVE`), so the
///   comparator's bit-decomposition is injective and cannot return a silently-wrapped
///   verdict. The values are SHARINGS, so this cannot be checked in cleartext: it is
///   PROVED in-protocol, once per element, by the same Rabbit range proof the
///   comparator runs (`compare::verify_shared_operand_in_range`) — the value is never
///   reconstructed.
///
/// [OPUS-5] Both were previously enforced only *inside* `secure_greater_than_shared`,
/// i.e. once per compare-exchange gate. `SortingNetwork::new(n)` emits ZERO gates for
/// `n <= 1`, so for a 0- or 1-element column neither guard ran and the contract above
/// was skipped — the identical out-of-range sharing was rejected at `n = 2` and
/// accepted at `n = 1`. Checking up front makes the contract hold at every length
/// (regression tests `oblivious_sort_rejects_out_of_range_value_at_every_column_length`
/// / `oblivious_sort_rejects_deficient_party_count_at_gateless_lengths`). The added
/// cost is `O(n)` decompositions against the sort's own `O(n log² n)`, and it is
/// strictly stronger than the per-gate check: every element is proved in range BEFORE
/// any comparison opens anything.
///
/// Honest-majority, semi-honest; external soundness sign-off pending (sq-qhy4).
pub fn oblivious_sort_by_secret_key(
    backend: &ShamirBackend,
    dealer: &mut ShamirDealer,
    mut col: Vec<Vec<Share>>,
) -> Result<Vec<Vec<Share>>, MpcError> {
    // --- Fail-closed contract enforcement, BEFORE any protocol work and independent
    //     of how many compare-exchange gates the network happens to emit. ---
    check_party_count(dealer.parties(), dealer.threshold()).map_err(|e| match e {
        MpcError::Protocol(m) => MpcError::Protocol(format!(
            "oblivious_sort_by_secret_key: refusing to sort on a deficient backend — {m}"
        )),
        other => other,
    })?;
    for (k, entry) in col.iter().enumerate() {
        verify_shared_operand_in_range(dealer, backend, entry).map_err(|e| match e {
            MpcError::Protocol(m) => MpcError::Protocol(format!(
                "oblivious_sort_by_secret_key: column entry #{k} failed the in-protocol range \
                 check (every value must be < 2^60 = {}, the comparator's injective window) — \
                 refusing fail-closed rather than sorting on a wrapped comparison: {m}",
                COMPARE_MAX_EXCLUSIVE
            )),
            other => other,
        })?;
    }

    let n = col.len();
    let net = SortingNetwork::new(n);
    for &(i, j) in net.compare_exchanges() {
        let (lo, hi) = (i.min(j), i.max(j));
        let swap_bit = secure_greater_than_shared(dealer, backend, &col[lo], &col[hi])?;
        // x_lo' = x_lo + b·(x_hi − x_lo); x_hi' = x_hi − b·(x_hi − x_lo).
        let delta = shamir::sub_shares(&col[hi], &col[lo])?;
        let step = secure_mul(dealer, &swap_bit, &delta)?;
        col[lo] = shamir::add_shares(&col[lo], &step)?;
        col[hi] = shamir::sub_shares(&col[hi], &step)?;
    }
    Ok(col)
}

/// Range-check a key against the comparator's injective window (`< 2^60`), failing
/// closed so an out-of-range key never yields a silently-wrapped comparison.
fn check_key_range(side: &str, idx: usize, key: Fp) -> Result<(), MpcError> {
    if key.value() >= COMPARE_MAX_EXCLUSIVE {
        return Err(MpcError::Protocol(format!(
            "sort_merge_semi_anti: {side} key #{idx} = {} is out of range (must be < 2^60 = {} so \
             the in-MPC bit-decomposition comparator is injective — use a <=60-bit key encoding)",
            key.value(),
            COMPARE_MAX_EXCLUSIVE
        )));
    }
    Ok(())
}

/// General secure multiply of two degree-`t` sharings: one [`shamir::mul_shares_raw`]
/// (→ degree `2t`) + one [`ShamirDealer::degree_reduce`] back to a fresh degree-`t`
/// sharing (the mult-chaining step, sq-dvuc). Used for logical AND (`a·b` on 0/1
/// operands) AND for the conditional swap's `b·(x_j − x_i)` (`b` a 0/1 bit, the
/// delta a general field value) — a product is a product.
fn secure_mul(
    dealer: &mut ShamirDealer,
    a: &[Share],
    b: &[Share],
) -> Result<Vec<Share>, MpcError> {
    let prod_2t = shamir::mul_shares_raw(a, b)?;
    dealer.degree_reduce(&prod_2t)
}

/// Secret OR of two 0/1 sharings: `a OR b = a + b − a·b` (one secure multiplication,
/// the rest local). Correct for 0/1 operands.
fn secure_or(
    dealer: &mut ShamirDealer,
    a: &[Share],
    b: &[Share],
) -> Result<Vec<Share>, MpcError> {
    let and = secure_mul(dealer, a, b)?;
    let sum = shamir::add_shares(a, b)?;
    shamir::sub_shares(&sum, &and)
}

/// Secret-bit-gated conditional swap of two whole [`Row`]s: for each carried field
/// `x_i, x_j`, `x_i' = x_i + b·(x_j − x_i)` and `x_j' = x_j − b·(x_j − x_i)`. With a
/// secret bit `b ∈ {0,1}` this is a swap when `b = 1` and identity when `b = 0`,
/// opening nothing and conserving the pair (`x_i' + x_j' = x_i + x_j`). One secure
/// multiplication per carried field (here 3: key, is_l, pid).
fn conditional_swap(
    dealer: &mut ShamirDealer,
    rows: &mut [Row],
    i: usize,
    j: usize,
    b: &[Share],
) -> Result<(), MpcError> {
    // Field-by-field so we borrow one column pair at a time.
    swap_field(dealer, b, |r| &r.key, |r, v| r.key = v, rows, i, j)?;
    swap_field(dealer, b, |r| &r.is_l, |r, v| r.is_l = v, rows, i, j)?;
    swap_field(dealer, b, |r| &r.pid, |r, v| r.pid = v, rows, i, j)?;
    Ok(())
}

/// Apply the conditional swap to ONE field of the two rows, selected by `get`/`set`.
fn swap_field(
    dealer: &mut ShamirDealer,
    b: &[Share],
    get: impl Fn(&Row) -> &Vec<Share>,
    set: impl Fn(&mut Row, Vec<Share>),
    rows: &mut [Row],
    i: usize,
    j: usize,
) -> Result<(), MpcError> {
    let delta = shamir::sub_shares(get(&rows[j]), get(&rows[i]))?; // x_j − x_i
    let step = secure_mul(dealer, b, &delta)?; // b·(x_j − x_i)
    let new_i = shamir::add_shares(get(&rows[i]), &step)?;
    let new_j = shamir::sub_shares(get(&rows[j]), &step)?;
    set(&mut rows[i], new_i);
    set(&mut rows[j], new_j);
    Ok(())
}

/// Sort rows into a canonical order (by their `{:?}` term tuple) so the disclosed
/// result multiset is independent of the shuffle / row ordering — the recipient
/// recomputes any ORDER BY over the disclosed multiset (convention #4).
fn canonicalize_rows(rows: &mut [Vec<Option<Term>>]) {
    rows.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
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

    /// The real (non-dummy) rows of a result, as an order-independent multiset of
    /// debug strings.
    fn multiset(rows: &[Vec<Option<Term>>]) -> Vec<String> {
        let mut m: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
        m.sort();
        m
    }

    /// Plaintext oracle: the semi-/anti-join of L against R's key set (the
    /// `minus_bindings` semantics for anti on a single shared key).
    fn plaintext_expected(
        left: &[(Fp, Vec<Option<Term>>)],
        right_keys: &[Fp],
        kind: SortMergeJoinKind,
    ) -> Vec<String> {
        let rset: std::collections::HashSet<u64> = right_keys.iter().map(|k| k.value()).collect();
        let mut m: Vec<String> = left
            .iter()
            .filter(|(k, _)| match kind {
                SortMergeJoinKind::Semi => rset.contains(&k.value()),
                SortMergeJoinKind::Anti => !rset.contains(&k.value()),
            })
            .map(|(_, p)| format!("{p:?}"))
            .collect();
        m.sort();
        m
    }

    // ---- oblivious sort substrate: reconstruct-after-sort == plaintext sort -------

    #[test]
    fn oblivious_secret_key_sort_matches_plaintext_sort() {
        for seed in 0..8u64 {
            let backend = ShamirBackend::new_seeded(3, 100 + seed).unwrap();
            let mut dealer = backend.dealer();
            let vals: Vec<u64> = vec![7, 2, 9, 2, 5, 0, 5, 3];
            let col: Vec<Vec<Share>> = vals.iter().map(|&v| dealer.share(Fp::new(v))).collect();
            let sorted = oblivious_sort_by_secret_key(&backend, &mut dealer, col).unwrap();
            let got: Vec<u64> = sorted
                .iter()
                .map(|s| backend.reconstruct(s).unwrap().value())
                .collect();
            let mut expected = vals.clone();
            expected.sort_unstable();
            assert_eq!(got, expected, "seed {seed}: secret-key sort wrong");
        }
    }

    /// The sort ACCESS PATTERN is data-independent (a function of `n` only) — the
    /// obliviousness substrate. Two different data columns of the same length drive
    /// the IDENTICAL compare-exchange sequence.
    #[test]
    fn sort_access_pattern_is_data_independent() {
        let a = SortingNetwork::new(8);
        let b = SortingNetwork::new(8);
        assert_eq!(a.compare_exchanges(), b.compare_exchanges());
    }

    // ---- differential: semi-join == plaintext semi-join --------------------------

    #[test]
    fn semi_join_matches_plaintext() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        // L keys: 1,2,3,4 ; R keys: 3,1,1 → present {1,3}. Semi keeps rows 1 and 3.
        let left = vec![
            (Fp::new(1), vec![lit("a")]),
            (Fp::new(2), vec![lit("b")]),
            (Fp::new(3), vec![lit("c")]),
            (Fp::new(4), vec![lit("d")]),
        ];
        let right = [Fp::new(3), Fp::new(1), Fp::new(1)];
        let (res, cost) =
            sort_merge_semi_anti(&backend, &left, &right, vec![var("x")], SortMergeJoinKind::Semi)
                .unwrap();
        assert_eq!(res.holder, HolderId::new("federation"));
        assert_eq!(
            multiset(&res.rows),
            plaintext_expected(&left, &right, SortMergeJoinKind::Semi)
        );
        assert_eq!(multiset(&res.rows), vec![format!("{:?}", vec![lit("a")]), format!("{:?}", vec![lit("c")])]);
        assert_eq!(cost.union_size, 7);
    }

    // ---- differential: anti-join == plaintext anti-join (MINUS semantics) ---------

    #[test]
    fn anti_join_matches_plaintext_minus() {
        let backend = ShamirBackend::new_seeded(3, 2).unwrap();
        let left = vec![
            (Fp::new(1), vec![lit("a")]),
            (Fp::new(2), vec![lit("b")]),
            (Fp::new(3), vec![lit("c")]),
            (Fp::new(4), vec![lit("d")]),
        ];
        let right = [Fp::new(3), Fp::new(1), Fp::new(1)];
        // Anti keeps the L rows whose key is NOT in R: keys 2 and 4 → rows b, d.
        let (res, _) =
            sort_merge_semi_anti(&backend, &left, &right, vec![var("x")], SortMergeJoinKind::Anti)
                .unwrap();
        assert_eq!(
            multiset(&res.rows),
            plaintext_expected(&left, &right, SortMergeJoinKind::Anti)
        );
        assert_eq!(multiset(&res.rows), vec![format!("{:?}", vec![lit("b")]), format!("{:?}", vec![lit("d")])]);
    }

    /// Randomised differential across many key layouts, party counts and seeds —
    /// the sort-merge result must equal the plaintext oracle for BOTH kinds,
    /// including duplicate keys (fan-out groups) and disjoint / identical sides.
    #[test]
    fn randomised_differential_semi_and_anti() {
        // A few hand-varied layouts (no RNG in tests — vary by index, per harness).
        let layouts: Vec<(Vec<u64>, Vec<u64>)> = vec![
            (vec![1, 2, 3], vec![]),                 // empty R: semi=∅, anti=all
            (vec![], vec![1, 2]),                    // empty L: both ∅
            (vec![5, 5, 5], vec![5]),                // all L share one key present in R
            (vec![5, 5, 5], vec![9]),                // group absent from R
            (vec![1, 2, 2, 3, 4], vec![2, 4, 4, 6]), // mixed duplicates both sides
            (vec![10, 20, 30], vec![30, 20, 10]),    // identical sets, permuted
        ];
        for (li, (lk, rk)) in layouts.iter().enumerate() {
            for parties in [3usize, 5] {
                let backend = ShamirBackend::new_seeded(parties, 500 + li as u64).unwrap();
                let left: Vec<(Fp, Vec<Option<Term>>)> = lk
                    .iter()
                    .enumerate()
                    .map(|(i, &k)| (Fp::new(k), vec![lit(&format!("L{i}"))]))
                    .collect();
                let right: Vec<Fp> = rk.iter().map(|&k| Fp::new(k)).collect();
                for kind in [SortMergeJoinKind::Semi, SortMergeJoinKind::Anti] {
                    let (res, _) = sort_merge_semi_anti(
                        &backend,
                        &left,
                        &right,
                        vec![var("x")],
                        kind,
                    )
                    .unwrap();
                    assert_eq!(
                        multiset(&res.rows),
                        plaintext_expected(&left, &right, kind),
                        "layout {li} parties {parties} kind {kind:?}: sort-merge != plaintext"
                    );
                }
            }
        }
    }

    /// The revealed result MULTISET is invariant to the oblivious shuffle / seed —
    /// correctness holds no matter which permutation the shuffle drew (the shuffle's
    /// order-hiding obliviousness itself is the substrate property tested in
    /// `oblivious.rs`; here the operator canonicalises its output, so we assert the
    /// seed-invariant multiset, not an order).
    #[test]
    fn result_multiset_is_shuffle_invariant() {
        let left = vec![
            (Fp::new(1), vec![lit("p0")]),
            (Fp::new(2), vec![lit("p1")]),
            (Fp::new(3), vec![lit("p2")]),
            (Fp::new(4), vec![lit("p3")]),
        ];
        let right = [Fp::new(1), Fp::new(2), Fp::new(3), Fp::new(4)]; // all present
        let expected = plaintext_expected(&left, &right, SortMergeJoinKind::Semi);
        for seed in 0..8u64 {
            let backend = ShamirBackend::new_seeded(3, 700 + seed).unwrap();
            let (res, _) = sort_merge_semi_anti(
                &backend,
                &left,
                &right,
                vec![var("x")],
                SortMergeJoinKind::Semi,
            )
            .unwrap();
            assert_eq!(multiset(&res.rows), expected, "seed {seed}: multiset changed");
        }
    }

    // ---- real randomized differential + mutation (non-vacuity) check -------------

    /// Deterministic SplitMix64 — a per-index-seeded PRNG so the differential is
    /// *generated* (many lengths / seeds / key multisets) yet fully reproducible, with
    /// no `Math::random`-style nondeterminism (harness convention: vary by seed index).
    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Generate one L/R key-multiset layout for `seed`, deliberately spanning the
    /// coverage the hand-picked layouts miss: arbitrary lengths (incl. empty sides),
    /// heavy duplicate/collision domains (small key space → fan-out groups + shared
    /// keys), AND keys near the supported upper bound `2^60 − 1` (max Rabbit width).
    fn generate_layout(seed: u64) -> (Vec<u64>, Vec<u64>) {
        let mut s = seed.wrapping_mul(0x2545_F491_4F6C_DD1D).wrapping_add(1);
        let ll = (splitmix64(&mut s) % 7) as usize; // 0..=6 (empty L reachable)
        let rr = (splitmix64(&mut s) % 7) as usize; // 0..=6 (empty R reachable)
        let key = |s: &mut u64| -> u64 {
            let r = splitmix64(s);
            match r % 5 {
                // Tiny domain → many duplicates / shared keys across sides.
                0 | 1 => r % 4,
                2 => r % 12,
                // Near the supported upper bound (exercises full 60-bit Rabbit width).
                3 => (COMPARE_MAX_EXCLUSIVE - 1) - (r % 3),
                _ => r % 40,
            }
        };
        let lk: Vec<u64> = (0..ll).map(|_| key(&mut s)).collect();
        let rk: Vec<u64> = (0..rr).map(|_| key(&mut s)).collect();
        (lk, rk)
    }

    /// A REAL generated randomized differential: for many generated layouts (varying
    /// lengths, seeds, duplicate/collision patterns, empty sides, and upper-bound
    /// values), the actual `sort_merge_semi_anti` output MUST equal the plaintext
    /// oracle for BOTH kinds and several party counts. This is the coverage the six
    /// fixed layouts do not provide.
    #[test]
    fn real_randomised_differential_generated() {
        let mut ran = 0usize;
        for seed in 0..24u64 {
            let (lk, rk) = generate_layout(seed);
            let left: Vec<(Fp, Vec<Option<Term>>)> = lk
                .iter()
                .enumerate()
                .map(|(i, &k)| (Fp::new(k), vec![lit(&format!("L{i}"))]))
                .collect();
            let right: Vec<Fp> = rk.iter().map(|&k| Fp::new(k)).collect();
            // Party count varies with the seed (both honest-majority sizes exercised).
            let parties = if seed % 4 == 0 { 5 } else { 3 };
            let backend = ShamirBackend::new_seeded(parties, 900 + seed).unwrap();
            for kind in [SortMergeJoinKind::Semi, SortMergeJoinKind::Anti] {
                let (res, _) =
                    sort_merge_semi_anti(&backend, &left, &right, vec![var("x")], kind).unwrap();
                assert_eq!(
                    multiset(&res.rows),
                    plaintext_expected(&left, &right, kind),
                    "seed {seed} parties {parties} kind {kind:?} \
                     (L={lk:?} R={rk:?}): sort-merge != plaintext oracle"
                );
                ran += 1;
            }
        }
        assert!(ran >= 40, "expected a broad generated sweep, ran only {ran}");
    }

    /// The named corruption points of the novel sort/scan, injected into a cleartext
    /// MODEL of the exact pipeline so a mutation test can prove the oracle differential
    /// is NON-VACUOUS (it goes red on a wrong answer) without threading `cfg(test)`
    /// fault hooks through the secret-shared crypto core.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Mutation {
        /// No corruption — must reproduce the production output and the oracle.
        None,
        /// Emit R rows instead of L rows (the `is_l AND cond` selector gated wrong).
        Selector,
        /// The adjacency-equality bit is stuck at 0 (equal keys never seen as a run).
        AdjacencyEquality,
        /// Only the forward OR pass runs (the backward scan direction is dropped).
        ScanDirection,
        /// The oblivious sort's compare-exchanges never fire (swap decision corrupted),
        /// so equal keys across the L/R boundary are no longer adjacent.
        SwapDecision,
    }

    /// Cleartext model of the EXACT sort-merge pipeline (tagged union → oblivious sort
    /// by key → adjacency equalities → forward/backward OR scan → `is_l AND cond`
    /// selector), returning the kept-L-payload multiset. `Mutation::None` mirrors the
    /// secret-shared implementation step-for-step; the other variants corrupt one named
    /// step. Validated equal to the production output on every generated input by
    /// [`model_matches_production`], so it is a faithful stand-in for the mutation test.
    fn sort_merge_model(
        left: &[(Fp, Vec<Option<Term>>)],
        right_keys: &[Fp],
        kind: SortMergeJoinKind,
        mutation: Mutation,
    ) -> Vec<String> {
        // Tagged union: (key, is_l, pid).
        let mut rows: Vec<(u64, bool, usize)> = Vec::new();
        for (i, (k, _)) in left.iter().enumerate() {
            rows.push((k.value(), true, i));
        }
        for k in right_keys {
            rows.push((k.value(), false, 0));
        }
        // (1) oblivious sort by key ascending — unless the swap decisions are corrupted.
        if mutation != Mutation::SwapDecision {
            rows.sort_by_key(|&(k, _, _)| k);
        }
        let n = rows.len();
        // (2) adjacency equalities eq[k] = key_k == key_{k+1}.
        let eq: Vec<bool> = (0..n.saturating_sub(1))
            .map(|k| match mutation {
                Mutation::AdjacencyEquality => false,
                _ => rows[k].0 == rows[k + 1].0,
            })
            .collect();
        let is_r: Vec<bool> = rows.iter().map(|&(_, is_l, _)| !is_l).collect();
        // (3) backward + forward OR passes (broadcast "an R shares my run").
        let mut b_r = vec![false; n];
        let mut f_r = vec![false; n];
        if n > 0 {
            b_r[n - 1] = is_r[n - 1];
            for k in (0..n - 1).rev() {
                b_r[k] = is_r[k] || (eq[k] && b_r[k + 1]);
            }
            f_r[0] = is_r[0];
            for k in 1..n {
                f_r[k] = is_r[k] || (eq[k - 1] && f_r[k - 1]);
            }
        }
        // (4) selector: keep iff is_l AND the semi/anti condition on "any R in my run".
        let mut out: Vec<String> = Vec::new();
        for k in 0..n {
            let (_, is_l, pid) = rows[k];
            let any_r = match mutation {
                // Drop the backward pass: an L before its R match is no longer seen.
                Mutation::ScanDirection => f_r[k],
                _ => b_r[k] || f_r[k],
            };
            let cond = match kind {
                SortMergeJoinKind::Semi => any_r,
                SortMergeJoinKind::Anti => !any_r,
            };
            // The `is_l AND cond` gate — corrupted to gate on is_r under Selector.
            let gate = match mutation {
                Mutation::Selector => !is_l,
                _ => is_l,
            };
            if gate && cond {
                out.push(format!("{:?}", left[pid].1));
            }
        }
        out.sort();
        out
    }

    /// The cleartext model is faithful: with NO mutation it equals BOTH the plaintext
    /// oracle AND the real secret-shared `sort_merge_semi_anti` output on every
    /// generated input — so mutating the model is a valid proxy for corrupting the
    /// production sort/scan.
    #[test]
    fn model_matches_production() {
        for seed in 0..16u64 {
            let (lk, rk) = generate_layout(seed);
            let left: Vec<(Fp, Vec<Option<Term>>)> = lk
                .iter()
                .enumerate()
                .map(|(i, &k)| (Fp::new(k), vec![lit(&format!("L{i}"))]))
                .collect();
            let right: Vec<Fp> = rk.iter().map(|&k| Fp::new(k)).collect();
            let backend = ShamirBackend::new_seeded(3, 800 + seed).unwrap();
            for kind in [SortMergeJoinKind::Semi, SortMergeJoinKind::Anti] {
                let oracle = plaintext_expected(&left, &right, kind);
                assert_eq!(
                    sort_merge_model(&left, &right, kind, Mutation::None),
                    oracle,
                    "seed {seed} kind {kind:?}: model(None) != oracle"
                );
                let (res, _) =
                    sort_merge_semi_anti(&backend, &left, &right, vec![var("x")], kind).unwrap();
                assert_eq!(
                    multiset(&res.rows),
                    oracle,
                    "seed {seed} kind {kind:?}: production != oracle"
                );
            }
        }
    }

    /// PRIMARY non-vacuity check — mutates the REAL secret-shared fast path.
    ///
    /// Each named corruption is injected into `sort_merge_semi_anti` ITSELF (via the
    /// test-only [`ProdFault`] seam wired through the production sort/scan/selector),
    /// then the ACTUAL secret-shared output is run against the plaintext oracle. Every
    /// fault MUST make the production differential go red — diverge from the oracle (or
    /// trip a fail-closed error) — on at least one generated witness input, for at least
    /// one party count and kind. This proves the harness catches a deliberately wrong
    /// answer from the production MPC path, not merely from a cleartext model of it
    /// (the concern the model-only [`mutations_turn_the_oracle_differential_red`] check
    /// alone does not close). The unmutated production path is asserted to still match
    /// the oracle on the same input, so the divergence is due to the fault, not the data.
    #[test]
    fn production_fast_path_mutations_diverge_from_oracle() {
        let faults = [
            ProdFault::Selector,
            ProdFault::AdjacencyEquality,
            ProdFault::ScanDirection,
            ProdFault::SwapDecision,
        ];
        for &fault in &faults {
            let mut caught = false;
            'search: for seed in 0..64u64 {
                let (lk, rk) = generate_layout(seed);
                let left: Vec<(Fp, Vec<Option<Term>>)> = lk
                    .iter()
                    .enumerate()
                    .map(|(i, &k)| (Fp::new(k), vec![lit(&format!("L{i}"))]))
                    .collect();
                let right: Vec<Fp> = rk.iter().map(|&k| Fp::new(k)).collect();
                for parties in [3usize, 5] {
                    let backend = ShamirBackend::new_seeded(parties, 1300 + seed).unwrap();
                    for kind in [SortMergeJoinKind::Semi, SortMergeJoinKind::Anti] {
                        let oracle = plaintext_expected(&left, &right, kind);
                        // Control: the UNMUTATED production path still matches the oracle
                        // on this exact input, so any divergence below is the fault's doing.
                        let (honest, _) =
                            sort_merge_semi_anti(&backend, &left, &right, vec![var("x")], kind)
                                .unwrap();
                        assert_eq!(
                            multiset(&honest.rows),
                            oracle,
                            "unmutated production diverged — bad control for {fault:?}"
                        );
                        // The fault-injected REAL path must diverge (wrong multiset) or
                        // fail closed (a corrupted intermediate trips a guard) — either is
                        // the differential going red on a deliberately wrong production run.
                        let diverged = match sort_merge_semi_anti_faulted(
                            &backend,
                            &left,
                            &right,
                            vec![var("x")],
                            kind,
                            fault,
                        ) {
                            Ok((res, _)) => multiset(&res.rows) != oracle,
                            Err(_) => true,
                        };
                        if diverged {
                            caught = true;
                            break 'search;
                        }
                    }
                }
            }
            assert!(
                caught,
                "{fault:?}: the PRODUCTION fast path did not diverge from the oracle on any \
                 generated input — the differential would be vacuous against this real corruption"
            );
        }
    }

    /// Supplemental non-vacuity check on the cleartext MODEL (kept alongside the
    /// primary [`production_fast_path_mutations_diverge_from_oracle`], which mutates the
    /// real secret-shared path): each named corruption of the sort/scan (a wrong
    /// selector, a stuck adjacency equality, a dropped scan direction, a corrupted
    /// swap decision) MUST make the model pipeline disagree with the plaintext oracle on
    /// at least one generated input. Because the unmutated model is validated equal to
    /// the production output ([`model_matches_production`]), this documents the same
    /// mutations at the readable model level; it does NOT substitute for mutating the
    /// production path.
    #[test]
    fn mutations_turn_the_oracle_differential_red() {
        let mutations = [
            Mutation::Selector,
            Mutation::AdjacencyEquality,
            Mutation::ScanDirection,
            Mutation::SwapDecision,
        ];
        for &mutation in &mutations {
            let mut caught = false;
            'search: for seed in 0..64u64 {
                let (lk, rk) = generate_layout(seed);
                let left: Vec<(Fp, Vec<Option<Term>>)> = lk
                    .iter()
                    .enumerate()
                    .map(|(i, &k)| (Fp::new(k), vec![lit(&format!("L{i}"))]))
                    .collect();
                let right: Vec<Fp> = rk.iter().map(|&k| Fp::new(k)).collect();
                for kind in [SortMergeJoinKind::Semi, SortMergeJoinKind::Anti] {
                    let oracle = plaintext_expected(&left, &right, kind);
                    // Sanity: the SAME harness with no mutation still matches (so the
                    // divergence below is due to the mutation, not the input).
                    assert_eq!(
                        sort_merge_model(&left, &right, kind, Mutation::None),
                        oracle,
                        "unmutated model diverged — bad control for {mutation:?}"
                    );
                    if sort_merge_model(&left, &right, kind, mutation) != oracle {
                        caught = true;
                        break 'search;
                    }
                }
            }
            assert!(
                caught,
                "{mutation:?}: oracle differential did NOT go red on any generated \
                 input — the test would be vacuous against this corruption"
            );
        }
    }

    // ---- fail-closed contracts ---------------------------------------------------

    #[test]
    fn out_of_range_key_fails_closed() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let big = Fp::new(COMPARE_MAX_EXCLUSIVE); // == 2^60, out of range
        let left = vec![(big, vec![lit("a")])];
        let right = [Fp::new(1)];
        let err = sort_merge_semi_anti(&backend, &left, &right, vec![var("x")], SortMergeJoinKind::Semi)
            .unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("out of range")));
    }

    #[test]
    fn payload_arity_mismatch_fails_closed() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let left = vec![(Fp::new(1), vec![lit("a"), lit("b")])]; // arity 2
        let right = [Fp::new(1)];
        let err = sort_merge_semi_anti(&backend, &left, &right, vec![var("x")], SortMergeJoinKind::Semi)
            .unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("arity")));
    }

    #[test]
    fn empty_sides_are_handled() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        // Empty L → empty result for both kinds.
        let (res, cost) =
            sort_merge_semi_anti(&backend, &[], &[Fp::new(1)], vec![var("x")], SortMergeJoinKind::Anti)
                .unwrap();
        assert!(res.rows.is_empty());
        assert_eq!(cost.union_size, 1);
    }

    // ---- fail-closed contracts of the SORT SUBSTRATE, independent of gate count ---
    //
    // [OPUS-5] Review round 3 (sq-ujz8). `oblivious_sort_by_secret_key` documents a
    // fail-closed range + party-count contract, but the guards used to live ONLY
    // inside `secure_greater_than_shared` — which the sort calls once per
    // compare-exchange gate. `SortingNetwork::new(n)` emits ZERO gates for `n <= 1`,
    // so for a 0- or 1-element column NEITHER guard ever ran and the documented
    // contract was silently skipped: the SAME out-of-range sharing was rejected at
    // `n = 2` and accepted at `n = 1`. The tests below pin the contract at exactly
    // the gate-free sizes; they are RED without the up-front enforcement in
    // `oblivious_sort_by_secret_key`.

    /// An out-of-range value must be refused at EVERY column length — including the
    /// gate-free `n = 1`, where the comparator (and therefore its in-protocol range
    /// proof) is never invoked. Also pins the `n = 2` case so the two agree: the
    /// contract must not depend on how many compare-exchanges the network happens to
    /// emit.
    #[test]
    fn oblivious_sort_rejects_out_of_range_value_at_every_column_length() {
        let backend = ShamirBackend::new_seeded(3, 4242).unwrap();
        let mut dealer = backend.dealer();
        // == 2^60: the first value ABOVE the comparator's injective window, so its
        // bit-decomposition would be truncated and the comparison silently wrong.
        let out_of_range = Fp::new(COMPARE_MAX_EXCLUSIVE);

        // n = 1 — ZERO compare-exchange gates. This is the case that was accepted.
        let singleton = vec![dealer.share(out_of_range)];
        let err = oblivious_sort_by_secret_key(&backend, &mut dealer, singleton).unwrap_err();
        assert!(
            matches!(&err, MpcError::Protocol(m)
                if m.contains("oblivious_sort_by_secret_key") && m.contains("range")),
            "singleton out-of-range column must fail closed with a range error, got {err:?}"
        );

        // n = 2 — one gate; rejected before AND after, and now by the same up-front
        // check, so the diagnostic is identical in shape.
        let pair = vec![dealer.share(out_of_range), dealer.share(Fp::new(1))];
        let err = oblivious_sort_by_secret_key(&backend, &mut dealer, pair).unwrap_err();
        assert!(
            matches!(&err, MpcError::Protocol(m)
                if m.contains("oblivious_sort_by_secret_key") && m.contains("range")),
            "n = 2 out-of-range column must fail closed with a range error, got {err:?}"
        );

        // An out-of-range value anywhere in a LONGER column is refused too — the
        // check is per element, not per gate.
        let mut mixed: Vec<Vec<Share>> = (0..4u64).map(|v| dealer.share(Fp::new(v))).collect();
        mixed.push(dealer.share(out_of_range));
        let err = oblivious_sort_by_secret_key(&backend, &mut dealer, mixed).unwrap_err();
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("column entry #4")),
            "the offending element must be named, got {err:?}"
        );
    }

    /// An in-range column of ANY length still sorts — the fail-closed check must not
    /// have become a blanket refusal. Covers the two gate-free lengths (0 and 1) plus
    /// a value at the top of the legal window.
    #[test]
    fn oblivious_sort_accepts_in_range_columns_including_gateless_lengths() {
        let backend = ShamirBackend::new_seeded(3, 4243).unwrap();
        let mut dealer = backend.dealer();

        // n = 0.
        let empty = oblivious_sort_by_secret_key(&backend, &mut dealer, Vec::new()).unwrap();
        assert!(empty.is_empty());

        // n = 1, at the largest LEGAL value (2^60 − 1) — the boundary below the bound.
        let top = Fp::new(COMPARE_MAX_EXCLUSIVE - 1);
        let top_col = vec![dealer.share(top)];
        let one = oblivious_sort_by_secret_key(&backend, &mut dealer, top_col)
            .expect("the top in-range value must be accepted");
        assert_eq!(backend.reconstruct(&one[0]).unwrap(), top);
    }

    /// The `n >= 2t+1` party-count contract must hold at the gate-free lengths too:
    /// with a deficient backend the sort's own secure multiplications (and the
    /// comparator's) are unsound, so it must refuse BEFORE doing any protocol work —
    /// even for a column so short that no comparison would ever run.
    #[test]
    fn oblivious_sort_rejects_deficient_party_count_at_gateless_lengths() {
        // 3 parties with t = 2 violates n >= 2t+1 (3 < 5). The public constructors
        // can never build this; the test-only escape hatch can.
        let backend = ShamirBackend::with_unchecked_threshold(3, 2);
        let mut dealer = backend.dealer();

        // n = 0 — zero gates AND zero elements: only an up-front check can catch it.
        let err = oblivious_sort_by_secret_key(&backend, &mut dealer, Vec::new()).unwrap_err();
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("2t+1")),
            "empty column on a deficient backend must fail closed, got {err:?}"
        );

        // n = 1 — zero gates.
        let col = vec![dealer.share(Fp::new(7))];
        let err = oblivious_sort_by_secret_key(&backend, &mut dealer, col).unwrap_err();
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("2t+1")),
            "singleton column on a deficient backend must fail closed, got {err:?}"
        );
    }

    // ---- cost model: scan_mults reports the REAL multiplication count -------------

    /// [OPUS-5] Review round 4 (PR #3585). `scan_mults` claims to report the secure
    /// multiplications the segmented scan spends, but the scan used to tally each
    /// adjacency equality as ONE multiplication — while `secure_equal_to_bit_shared`
    /// alone performs `RABBIT_VALUE_BITS` per-bit equalities + a balanced AND-tree,
    /// on top of two Rabbit bit-decompositions with their range proofs: well over a
    /// hundred multiplications per call. `scan_mults` is now a delta of the dealer's
    /// `degree_reduce` counter, so it counts what actually ran.
    ///
    /// The exact per-equality count varies slightly per call (the Rabbit LTBits
    /// chain spends one extra multiplication per zero bit of the random masked open
    /// `c`), so the derived pin is a FLOOR from the circuit's data-independent
    /// parts — which the old one-per-call tally cannot reach on any input.
    #[test]
    fn scan_mults_counts_the_full_adjacency_equality_circuit() {
        use crate::compare::RABBIT_VALUE_BITS;
        let backend = ShamirBackend::new_seeded(3, 4244).unwrap();

        // Independently measure ONE equality through the same counter the operator
        // reads: at least the equality fold alone (RABBIT_VALUE_BITS per-bit
        // equalities + RABBIT_VALUE_BITS − 1 AND-tree multiplications).
        let eq_floor = 2 * RABBIT_VALUE_BITS - 1;
        let mut dealer = backend.dealer();
        let a = dealer.share(Fp::new(5));
        let b = dealer.share(Fp::new(9));
        let before = dealer.mult_count();
        secure_equal_to_bit_shared(&mut dealer, &backend, &a, &b).unwrap();
        let one_eq = dealer.mult_count() - before;
        assert!(
            one_eq >= eq_floor,
            "one secure_equal_to_bit_shared performed {one_eq} counted secure \
             multiplications, below the equality-fold floor {eq_floor}"
        );

        // A small union: |L| = 2, |R| = 2 → n = 4, so n − 1 = 3 adjacency
        // equalities. Scan structure: (n − 1) equality circuits, 2 multiplications
        // per backward-pass step and per forward-pass step (n − 1 each: the eq·carry
        // AND + the OR), and 2 per row for the selector (the b∨f OR + the is_l·cond
        // AND).
        let left = vec![(Fp::new(1), vec![lit("a")]), (Fp::new(2), vec![lit("b")])];
        let right = [Fp::new(2), Fp::new(7)];
        let (_, cost) = sort_merge_semi_anti(
            &backend,
            &left,
            &right,
            vec![var("x")],
            SortMergeJoinKind::Semi,
        )
        .unwrap();
        let n = cost.union_size;
        assert_eq!(n, 4);
        let derived_floor = (n - 1) * eq_floor + 4 * (n - 1) + 2 * n;
        assert!(
            cost.scan_mults >= derived_floor,
            "scan_mults = {} but the scan performs at least {derived_floor} secure \
             multiplications for a union of {n} — an equality is being undercounted",
            cost.scan_mults
        );
        // The exact value the old accounting reported for this input — each equality
        // counted as one multiplication — must be impossible now.
        let one_per_equality_tally = (n - 1) + 4 * (n - 1) + 2 * n;
        assert!(
            cost.scan_mults > one_per_equality_tally,
            "scan_mults = {} matches the one-multiplication-per-equality undercount \
             ({one_per_equality_tally})",
            cost.scan_mults
        );
    }

    /// [OPUS-5] Review round 5 (PR #3585). `scan_mults` counts only the dealer's
    /// degree-reduction rounds; the scan's Rabbit machinery ALSO performs
    /// interactive opens — the square-protocol `c = a²` open behind every
    /// jointly-random mask bit, the masked open of each bit-decomposition, and the
    /// zero-test product open of each range proof — which the cost report used to
    /// drop entirely. `scan_opens` now reports them, counted from the dealer's
    /// open counter.
    ///
    /// Unlike the multiplication count (data-dependent via the public bits of the
    /// masked open), the OPEN count is data-independent — a retry costs an extra
    /// open only on the ~`2^{-61}` `a == 0` draw — so the pin is EXACT: each
    /// `secure_equal_to_bit_shared` performs `2·(RABBIT_MASK_BITS + 2)` opens (per
    /// operand: one open per mask bit, one masked decomposition open, one
    /// range-proof zero-test open), and the scan's other steps (OR passes,
    /// selectors) open nothing.
    #[test]
    fn scan_opens_reports_the_rabbit_interactive_opens_exactly() {
        use crate::compare::RABBIT_MASK_BITS;
        let backend = ShamirBackend::new_seeded(3, 4245).unwrap();

        // Derive the per-equality open count and cross-check it against ONE
        // independently-measured call through the same counter the operator reads.
        let per_equality = 2 * (RABBIT_MASK_BITS + 2);
        let mut dealer = backend.dealer();
        let a = dealer.share(Fp::new(5));
        let b = dealer.share(Fp::new(9));
        let before = dealer.open_count();
        secure_equal_to_bit_shared(&mut dealer, &backend, &a, &b).unwrap();
        let one_eq_opens = dealer.open_count() - before;
        assert_eq!(
            one_eq_opens, per_equality,
            "one secure_equal_to_bit_shared performed {one_eq_opens} interactive \
             opens, not the derived 2 masked decompositions + 2 zero-tests + \
             2 * {RABBIT_MASK_BITS} mask-bit opens = {per_equality}"
        );

        // A union of n = 4 → 3 adjacency equalities; nothing else in the scan
        // (the OR passes, the selectors) performs an open.
        let left = vec![(Fp::new(1), vec![lit("a")]), (Fp::new(2), vec![lit("b")])];
        let right = [Fp::new(2), Fp::new(7)];
        let (_, cost) = sort_merge_semi_anti(
            &backend,
            &left,
            &right,
            vec![var("x")],
            SortMergeJoinKind::Semi,
        )
        .unwrap();
        let n = cost.union_size;
        assert_eq!(n, 4);
        assert_eq!(
            cost.scan_opens,
            (n - 1) * per_equality,
            "scan_opens = {} must report every interactive Rabbit open the scan's \
             {} adjacency equalities perform",
            cost.scan_opens,
            n - 1
        );
        // The padded reveal stays a SEPARATE counter: n selector opens plus one
        // payload-id open for the single kept row (L key 2 matches R key 2) — the
        // scan's interior opens must not leak into it.
        assert_eq!(cost.opens, n + 1);
    }
}
