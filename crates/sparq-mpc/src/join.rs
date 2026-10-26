// GlobalJoin trait + disclosed-key equi-join over GLOBAL IRIs (M2).
//! The cross-holder join protocol.
//!
//! Architecture refs: §2 convention #6 (GLOBAL IRIs as cross-credential join
//! keys — the distinguishing feature vs all prior graph-MPC), §4.3 step 4 (the
//! join model: key-on-key → (circuit-)PSI; non-key → oblivious join with
//! bounded intermediate size), §3.1 (PSI does NOT compose for free into a
//! multi-pattern BGP), and OPEN QUESTION **§5.2 Q3** (BGP-join obliviousness
//! cost).
//!
//! ## Why this is its own boundary
//!
//! GOOSE/SMPG/PPMQ all join on *node-local* (Cypher) identifiers, which
//! disqualifies them for federation: a node id is meaningful only inside one
//! database. The contribution here is joining on **global IRIs** that mean the
//! same thing across independent holders (architecture §2 convention #6). That
//! makes the join key a public, dereferenceable identifier — which interacts
//! directly with the no-proof-of-revealed-properties rule (#4): where the join
//! KEY is disclosed, the join can be checked OUTSIDE the cryptographic core (a
//! plaintext check over disclosed IRIs); only where joined VALUES must stay
//! hidden does the join go into the MPC. Q3 is exactly how much of the
//! per-pattern obliviousness padding that out-of-circuit handling collapses,
//! and for which SPARQL fragment (RQ2b).
//!
//! ## Status — M2: the disclosed-key path is REAL
//!
//! [`DisclosedKeyJoin`] implements [`GlobalJoin`] for the
//! `JoinPlan::key_disclosed == true` regime: a crypto-free equi-join over the
//! disclosed global-IRI key. Because a global IRI is a stable, public
//! cross-holder identifier, joining on it needs NO MPC primitive — it is a
//! plaintext check OUTSIDE the cryptographic core (convention #4), and is
//! *invariant to both design forks* (Q1/Q2). This is the PLAN's first M2 target.
//!
//! The **hidden-value** path (`key_disclosed == false`, where the join values
//! are PRIVATE and must enter a circuit-PSI / oblivious join) stays an honest
//! [`MpcError::NotYetImplemented`]: it is gated on a chosen
//! [`crate::backend::MpcBackend`] (M3) and on the Q2 trust-model fork + the Q3
//! BGP-join obliviousness-cost analysis (RQ2b). No fake crypto here.

use crate::batched::{BatchedShares, RowBinding};
use crate::compare::secure_equal_to_bit_through;
use crate::field::Fp;
use crate::oblivious_join::{self, Candidate, MatchBit, ObliviousOutputCost};
use crate::partial::{HolderId, MpcError, PartialResult};
use crate::randomness::{
    checked_shared_nonzero_mask, checked_vss_own_input, require_source_matches,
    DistributedRandomness,
};
use crate::shamir::{self, ShamirBackend, ShamirDealer, Share};
use oxrdf::{Term, Variable};
use std::collections::BTreeMap;

/// Describes ONE cross-holder join: the variable to join on and whether its
/// bound values are disclosed (so the join can be checked in the clear) or
/// hidden (so it must run inside the MPC). This is pure data — produced by the
/// (untrusted, §4.1) planner and consumed by a [`GlobalJoin`] impl. The planner
/// is untrusted: a [`GlobalJoin`] must not rely on this plan for *soundness*,
/// only as a hint for *which* protocol to run.
#[derive(Debug, Clone)]
pub struct JoinPlan {
    /// The shared variable the holders' partials are joined on.
    pub join_var: Variable,
    /// `true` if `join_var`'s bound values are disclosed global IRIs (key-on-key
    /// equi-join checkable in the clear, §4.3 step 4); `false` if they are
    /// hidden values requiring an oblivious / PSI join inside the MPC.
    pub key_disclosed: bool,
}

/// The protocol that joins multiple holders' [`PartialResult`]s on a global IRI.
///
/// Architecture §4.3 step 4: cross-source joins on global IRIs are the heart of
/// federated evaluation. Two regimes the eventual impl must distinguish:
///
/// - **Disclosed-key join** (`JoinPlan::key_disclosed == true`): a plaintext
///   equi-join over the disclosed IRIs, computed OUTSIDE the cryptographic core
///   per convention #4. This sub-case is *invariant to Q1/Q2* (no secret data)
///   and is the natural first target within M2.
/// - **Hidden-value join** (`key_disclosed == false`): requires circuit-PSI /
///   oblivious join inside a [`crate::backend::MpcBackend`]; gated on Q2/Q3 and
///   far heavier.
///
/// ## Status — M2
/// [`DisclosedKeyJoin`] implements this for the disclosed-key regime (crypto-
/// free, the PLAN's first M2 target). The hidden path returns
/// [`MpcError::NotYetImplemented`] until the backend (M3) and Q2/Q3 land.
pub trait GlobalJoin {
    /// Join holders' partials according to `plan`, returning the combined
    /// disclosed result.
    ///
    /// A `GlobalJoin` must not rely on the (untrusted, §4.1) `plan` for
    /// *soundness* — only as a hint for *which* protocol to run. The disclosed-
    /// key implementation therefore independently checks that the named
    /// `join_var` actually appears in every partial it is asked to join, and
    /// performs full SPARQL compatible-mapping semantics over *all* shared
    /// columns (not just the planner-named key), so a malicious planner cannot
    /// induce a result that disagrees with PAG evaluation over the union.
    /// Validate each partial's row arity and unique projected variables before
    /// indexing any row, including when only one partial is supplied.
    fn join(&self, partials: &[PartialResult], plan: &JoinPlan) -> Result<PartialResult, MpcError>;
}

/// The disclosed-key cross-holder equi-join (M2; crypto-free).
///
/// Architecture §4.3 step 4 "key-on-key": each holder has already disclosed a
/// [`PartialResult`] (via [`crate::holder::Holder::evaluate_local`]) carrying
/// only the join key + the columns its fragment projects — minimise data
/// sharing (§4.2). Because the join key is a GLOBAL IRI (convention #6) and is
/// disclosed, joining is a plain equi-join over plaintext IRIs, done OUTSIDE
/// any cryptographic core (convention #4 no-proof-of-revealed-properties). No
/// MPC primitive is needed; the result is *invariant* to the Q1/Q2 forks.
///
/// ## Semantics — a faithful SPARQL join, not a naive key-merge
///
/// The result is the natural inner join of the partials under SPARQL
/// *compatible-mappings* semantics (Pérez–Arenas–Gutiérrez): two rows combine
/// iff they agree on EVERY shared variable they both bind. The planner names
/// one `join_var` (the global-IRI key it expects to be present and disclosed),
/// but soundness does not trust that name — the join still enforces agreement
/// on any *other* variable two partials happen to share, exactly as evaluating
/// the whole BGP over the union of the holders' graphs would. This is what
/// makes the differential test (join == union-store evaluation) hold.
///
/// Join is folded left-to-right over `partials`; SPARQL join is associative and
/// commutative up to row order, and this impl sorts its output canonically so
/// the result is order-independent (the disclosed multiset is what convention
/// #4 lets the verifier recompute aggregates/ordering over).
#[derive(Debug, Default, Clone, Copy)]
pub struct DisclosedKeyJoin;

impl DisclosedKeyJoin {
    pub fn new() -> Self {
        DisclosedKeyJoin
    }
}

impl GlobalJoin for DisclosedKeyJoin {
    fn join(&self, partials: &[PartialResult], plan: &JoinPlan) -> Result<PartialResult, MpcError> {
        // The hidden-value path is the deferred, fork-gated one: PRIVATE join
        // values that must enter a circuit-PSI / oblivious join. No fake crypto.
        if !plan.key_disclosed {
            return Err(MpcError::not_yet(
                "hidden-value (private-key) cross-holder join — circuit-PSI / oblivious join",
                "M3 MpcBackend + Q2 (trust model) + Q3 (BGP-join obliviousness cost, RQ2b)",
            ));
        }

        // --- Preconditions (real, M2-checkable; NOT trusting the planner). ---
        if partials.is_empty() {
            return Err(MpcError::Protocol(
                "disclosed-key join needs at least one partial".into(),
            ));
        }
        // Soundness obligation (§4.1): independently verify the named key is
        // actually projected by every partial — a planner that names a key a
        // holder did not disclose must FAIL, not silently yield an empty join.
        for p in partials {
            p.check_shape()?;
            if !p.vars.contains(&plan.join_var) {
                return Err(MpcError::Protocol(format!(
                    "holder {} did not disclose the join key ?{} (vars: {})",
                    p.holder,
                    plan.join_var.as_str(),
                    p.vars
                        .iter()
                        .map(|v| format!("?{}", v.as_str()))
                        .collect::<Vec<_>>()
                        .join(", "),
                )));
            }
        }

        // --- Fold the equi-join left-to-right. ---
        // The accumulator is itself a PartialResult; the federated result is
        // attributed to a synthetic "federation" holder (it is no single
        // holder's data — it is the disclosed cross-holder result).
        let mut acc = partials[0].clone();
        for next in &partials[1..] {
            acc = join_two(&acc, next)?;
        }

        // Canonical row order so the disclosed multiset is independent of holder
        // / row ordering (convention #4: the verifier recomputes ordering).
        canonicalize_rows(&mut acc.rows);
        acc.holder = HolderId::new("federation");
        Ok(acc)
    }
}

/// Join two partials under SPARQL compatible-mapping semantics. The output
/// schema is the union of the two variable lists (shared vars appear once, in
/// left-then-new order); two rows combine iff they agree on every shared var
/// they both bind (SPARQL treats an unbound var as compatible with anything).
fn join_two(left: &PartialResult, right: &PartialResult) -> Result<PartialResult, MpcError> {
    // Output schema: left vars, then right vars not already present.
    let mut out_vars = left.vars.clone();
    for v in &right.vars {
        if !out_vars.contains(v) {
            out_vars.push(v.clone());
        }
    }

    // Positions of each shared variable in left and right rows.
    // (var, left_idx, right_idx) for every var both sides project.
    let shared: Vec<(usize, usize)> = left
        .vars
        .iter()
        .enumerate()
        .filter_map(|(li, v)| right.vars.iter().position(|rv| rv == v).map(|ri| (li, ri)))
        .collect();

    // Index the right side by its tuple of shared-var values so the join is
    // O(|left|+|right|) hash-join rather than O(|left|·|right|). `oxrdf::Term`
    // is `Eq + Hash` but NOT `Ord`, so we key on a deterministic string render
    // of the shared-column terms (the same canonical render used to sort the
    // output). None = unbound, which SPARQL treats as compatible with anything —
    // handled below by also scanning right rows with an unbound shared slot and
    // by the explicit compatibility check. For the disclosed-key regime the
    // global-IRI key is always bound, so the common path is a clean hash hit.
    let mut right_by_key: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (ri, row) in right.rows.iter().enumerate() {
        let key = render_key(row, &shared.iter().map(|&(_, rj)| rj).collect::<Vec<_>>());
        right_by_key.entry(key).or_default().push(ri);
    }
    // Rows of the right side whose shared key has at least one unbound slot must
    // be considered against every left row (unbound is compatible with all), so
    // collect them once.
    let right_has_unbound_key: Vec<usize> = right
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| shared.iter().any(|&(_, rj)| row[rj].is_none()))
        .map(|(ri, _)| ri)
        .collect();

    let mut out_rows: Vec<Vec<Option<Term>>> = Vec::new();
    for lrow in &left.rows {
        let left_shared_idx: Vec<usize> = shared.iter().map(|&(lj, _)| lj).collect();
        let left_has_unbound = left_shared_idx.iter().any(|&lj| lrow[lj].is_none());
        let lkey = render_key(lrow, &left_shared_idx);

        // Candidate right rows: exact-key matches, plus any right row with an
        // unbound shared slot, plus (if the LEFT key has an unbound slot) every
        // right row. The explicit compatibility check below is authoritative;
        // these are just the candidate set.
        let mut candidates: Vec<usize> = right_by_key.get(&lkey).cloned().unwrap_or_default();
        candidates.extend(right_has_unbound_key.iter().copied());
        if left_has_unbound {
            candidates = (0..right.rows.len()).collect();
        }
        candidates.sort_unstable();
        candidates.dedup();

        for ri in candidates {
            let rrow = &right.rows[ri];
            if !compatible(lrow, rrow, &shared) {
                continue;
            }
            // Merge: left row, then the right columns not already in the schema.
            let mut merged = lrow.clone();
            for (rj, rv) in right.vars.iter().enumerate() {
                if !left.vars.contains(rv) {
                    merged.push(rrow[rj].clone());
                } else if let Some(slot) = out_vars.iter().position(|v| v == rv) {
                    // Shared var: if the left side had it unbound but the right
                    // side bound it, take the bound value (SPARQL merge).
                    if merged[slot].is_none() {
                        merged[slot] = rrow[rj].clone();
                    }
                }
            }
            out_rows.push(merged);
        }
    }

    Ok(PartialResult {
        // Provisional holder; the top-level join() overwrites it with the
        // synthetic federation id once the whole fold completes.
        holder: left.holder.clone(),
        vars: out_vars,
        rows: out_rows,
    })
}

/// Deterministic string render of a row's terms at the given column indices,
/// used as a hash-join key (oxrdf `Term` is `Eq + Hash` but not `Ord`, so we
/// cannot key a `BTreeMap` on `Vec<Option<Term>>` directly). `{:?}` is stable
/// within a run and distinguishes terms that differ in lexical form, datatype,
/// or language tag — adequate for an equi-join key. The authoritative match is
/// still term `==` via [`compatible`]; this only buckets candidates.
fn render_key(row: &[Option<Term>], idxs: &[usize]) -> String {
    let mut s = String::new();
    for &i in idxs {
        match &row[i] {
            Some(t) => {
                s.push('\u{1}'); // bound marker
                s.push_str(&format!("{t:?}"));
            }
            None => s.push('\u{0}'), // unbound marker (distinct from any term)
        }
        s.push('\u{1f}'); // column separator
    }
    s
}

/// SPARQL compatibility: two rows are compatible iff for every shared variable
/// they BOTH bind, the bound terms are equal. An unbound slot on either side is
/// compatible with anything.
fn compatible(lrow: &[Option<Term>], rrow: &[Option<Term>], shared: &[(usize, usize)]) -> bool {
    shared.iter().all(|&(li, ri)| match (&lrow[li], &rrow[ri]) {
        (Some(a), Some(b)) => a == b,
        _ => true, // one side unbound → compatible
    })
}

/// Sort rows into a canonical order (by their `{:?}` term tuple) so the
/// disclosed result multiset is independent of holder/row ordering — the
/// verifier recomputes ORDER BY over the disclosed multiset (convention #4).
fn canonicalize_rows(rows: &mut [Vec<Option<Term>>]) {
    rows.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
}

// =====================================================================
// M3 — the HIDDEN-VALUE join (the path M2 deferred).
// =====================================================================

/// A holder's contribution to a hidden-value join: rows whose JOIN KEY is
/// PRIVATE (already encoded as a field element — see [`HiddenValueJoin`] doc on
/// the encoding contract) plus the DISCLOSED payload columns that may be opened
/// on a match.
///
/// The key is `Fp` because the secret-shared equality primitive operates over
/// the field; the holder encodes its private key term into `Fp` via the
/// documented, collision-resistant [`crate::term_encode::encode_term`] (a
/// domain-separated SHA-512 folded into the field, with a stated birthday bound),
/// or — to GUARANTEE injectivity for a concrete key set rather than rely on the
/// bound — through [`crate::term_encode::KeyEncoder`], which fails closed on a
/// false-match collision before any key is shared. The in-circuit proof that the
/// opened key equals `encode_term(term)` for the holder's real term stays the M4
/// collaborative-proof job (the encoder is its on-ramp).
/// `Debug` is hand-written: `rows` pairs each PRIVATE join key with that row's
/// payload terms, so a derived `Debug` would route the keys to `Fp`'s rendering and
/// print the payload beside them. What is kept is the public shape — the holder,
/// the disclosed payload column names, and the row count, which this join already
/// treats as public (input cardinality, L3).
#[derive(Clone)]
pub struct HiddenKeyedRows {
    /// The holder that owns these rows.
    pub holder: HolderId,
    /// Disclosed payload column names (NOT including the hidden key).
    pub payload_vars: Vec<Variable>,
    /// One entry per row: `(private_key_fp, disclosed_payload_terms)`.
    pub rows: Vec<(Fp, Vec<Option<Term>>)>,
}

impl std::fmt::Debug for HiddenKeyedRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HiddenKeyedRows")
            .field("holder", &self.holder)
            .field("payload_vars", &self.payload_vars)
            .field("rows", &"<redacted>")
            .field("row_count", &self.rows.len())
            .finish()
    }
}

/// The hidden-value cross-holder join over PRIVATE join keys (M3).
///
/// This is the capability the M2 [`DisclosedKeyJoin`] could NOT provide: joining
/// two holders on a key WITHOUT revealing the key values, disclosing only the
/// result payload columns of the matching rows. It is built on the honest-
/// majority [`ShamirBackend`] (M3) via a **secret-shared equality test** — the
/// core of circuit-PSI (`mpc-protocols` skill: "PSI for private joins").
///
/// ## What it computes securely (vs what is scaffolded)
///
/// SECURELY (real, in-process multi-party simulation):
/// - Each holder's private key is **secret-shared** across the `n` Shamir
///   parties; the cleartext key never leaves the holder.
/// - For each candidate row pair `(i, j)` the parties compute the secret-shared
///   difference `d = key_Li - key_Rj` (local), multiply by a fresh jointly-random
///   nonzero mask `r` (one Shamir multiplication), and **open only** `m = d·r`.
///   `m == 0 ⇔ keys equal`; for unequal keys `m` is a uniform-random nonzero
///   field element, so opening it reveals ONLY the match bit — never either key
///   value, never their difference. This is the standard MPC equality-to-zero
///   test ([`shamir::mul_shares_raw`] + [`shamir::reconstruct_degree`]).
/// - On a match, the DISCLOSED payload columns are emitted (they are disclosed
///   by the disclosure-minimisation rule, convention #4 — only the *key* is
///   hidden). The keys are NEVER reconstructed.
///
/// SCAFFOLDED / out of scope (honestly stated, not faked):
/// - The `O(|L|·|R|)` all-pairs comparison is the naive oblivious join. A real
///   circuit-PSI uses oblivious hashing / cuckoo bins to cut this to ~linear;
///   that optimisation (Q3, RQ2b — BGP-join obliviousness cost) is NOT done. The
///   all-pairs version is correct but not the SOTA cost profile.
/// - The field encoding of the key now has a documented, collision-resistant
///   construction — [`crate::term_encode::encode_term`] (domain-separated SHA-512
///   folded into the field) with a stated birthday bound, plus
///   [`crate::term_encode::KeyEncoder`] for a fail-closed exact-injectivity check
///   over a concrete key set (sq-dl81). Equality in `Fp` is then a SOUND stand-in
///   for term equality except on a birthday collision (`≈ q²/2^62`, negligible in
///   the ≤10⁴-row regime), which `KeyEncoder` detects. What is STILL deferred to
///   the collaborative proof (M4) is the in-circuit guarantee that the opened key
///   actually equals `encode_term` of the holder's real term — the encoding-
///   *correctness* proof, for which this encoder is the on-ramp.
/// - Malicious security (a party feeding inconsistent shares) is NOT provided —
///   semi-honest only, exactly as [`ShamirBackend`] states.
///
/// ## Feasibility envelope (honest, per the skill — minutes, not seconds)
///
/// Communication: ONE multiplication round PER candidate pair → `|L|·|R|` secure
/// equality tests, each an open of one field element across the parties. On a
/// real network that all-pairs structure is the cost center the skill flags
/// ("joins are the cost center; obliviousness forces worst-case padding"). For
/// the viable regime (≤10³–10⁴ rows/holder, LAN) this is the
/// minutes-to-tens-of-minutes envelope (ORQ SOSP'25), NOT sub-second. We do not
/// extrapolate beyond it.
#[derive(Debug, Clone)]
pub struct HiddenValueJoin {
    backend: ShamirBackend,
}

impl HiddenValueJoin {
    /// Build a hidden-value join driven by an honest-majority Shamir backend.
    pub fn new(backend: ShamirBackend) -> Self {
        HiddenValueJoin { backend }
    }

    /// Securely test whether two secret-shared field values are equal, opening
    /// ONLY the match bit. This is the in-process multi-party simulation of the
    /// MPC equality-to-zero test (one multiplication + one open). The cleartext
    /// `a`/`b` are passed here ONLY because this function plays ALL parties in
    /// one process; it secret-shares them internally and never uses the
    /// cleartext after sharing except to deal the shares — exactly what a dealer
    /// does. It returns the boolean match WITHOUT reconstructing `a` or `b`.
    ///
    /// ## Leakage — the L2 match-graph leak (characterised, sq-4vgx)
    ///
    /// The match bit IS opened (the `m == 0?` test reconstructs `m`). For ONE pair
    /// that single bit is the minimum a join must compute. Calling the primitive
    /// on every `(i, j)` lets the party driving the opens observe each pair and
    /// learn the **entire bipartite match graph** — exactly which left row matched
    /// which right row — even though the keys themselves stay hidden (`m` is a
    /// uniform nonzero for unequal keys, so neither key nor their difference leaks).
    /// That match graph reveals the join-key fan-out / multiplicity distribution (a
    /// strong fingerprint of the hidden key distribution): leak **L2** in the
    /// leakage taxonomy (`research/mpc-sparql-capability-matrix.md` §5, "L2 per-pair
    /// match graph / fan-out"). This is the CHEAP-but-LEAKY tier — it leaks strictly
    /// more than the output multiset alone. [`Self::batched_join`]'s oblivious
    /// output bounds the result-set leaks (L1/L2 of the *output*), and
    /// [`Self::fully_oblivious_batched_join`] closes L2 at the DECISION by keeping
    /// the per-pair bit secret-shared via [`secure_equal_to_bit`] — never opened.
    /// The leak is PINNED by `secure_equal_leaks_full_bipartite_match_graph` so this
    /// boundary is a regression-guarded property, not just prose.
    ///
    /// **Consistency-checked open (sq-7q9i, WI-2).** The product `m = d·r` is a
    /// degree-`2t` Reed–Solomon codeword over the `n` party points; the open
    /// routes through [`shamir::reconstruct_degree`] → the WI-1 RS checker at
    /// degree `2t`. When `n > 2t + 1` a tampered product share is detected (abort
    /// with [`MpcError::Tampered`]) or corrected, so a forged share can no longer
    /// silently flip the match verdict. HONESTY: the honest-majority instantiation
    /// `t = ⌊(n−1)/2⌋` gives `n = 2t + 1` for odd `n`, where degree-`2t` has ZERO
    /// RS redundancy — tampering is information-theoretically undetectable and is
    /// NOT claimed otherwise (pinned by a boundary test; a true fix needs a MAC,
    /// deferred WI-4 / bead sq-6d6g). Detection at `n > 2t + 1` thus requires
    /// running the equality test on MORE than the minimal party count.
    ///
    /// **Randomness routed through the seam (sq-yyro, issue #6117).** Both key
    /// sharings are dealt with [`DistributedRandomness::vss_own_input`] rather than
    /// the dealer's inherent `share`, so a dealer-less source can be swapped in
    /// behind the same trait. For [`ShamirDealer`] that method *is* `share`, so the
    /// draws — and therefore the output — are unchanged; a source that cannot
    /// verifiably share a holder's own input (PRSS today) fails closed here.
    #[cfg(test)]
    fn secure_equal(
        &self,
        source: &mut dyn DistributedRandomness,
        a: Fp,
        b: Fp,
    ) -> Result<bool, MpcError> {
        // Secret-share both keys, then run the shared-input core. The scalar path
        // deals its own shares; the batched path deals a whole key COLUMN up front
        // (a `BatchedShares`) and passes the per-row sharings straight in.
        let (n, t) = Self::equality_params(source)?;
        let sa = checked_vss_own_input(source, n, a)?;
        let sb = checked_vss_own_input(source, n, b)?;
        Self::secure_equal_core(source, n, t, &sa, &sb)
    }

    /// Read the source's `(n, t)` ONCE and refuse metadata outside the
    /// honest-majority headroom one multiplication needs (`2t+1 <= n`, checked so
    /// a misreported `t` cannot wrap past the guard). Every later use of `n` / `t`
    /// in the equality path is this validated pair, never a re-read.
    fn equality_params(source: &dyn DistributedRandomness) -> Result<(usize, usize), MpcError> {
        let (n, t) = (source.parties(), source.threshold());
        if t.checked_mul(2)
            .and_then(|v| v.checked_add(1))
            .is_none_or(|r| r > n)
        {
            return Err(MpcError::Protocol(format!(
                "secure_equal needs n >= 2t+1 (n={n}, t={t})"
            )));
        }
        Ok((n, t))
    }

    /// The match-bit core of the scalar `secure_equal`, over keys that are ALREADY
    /// secret-shared. This is the entry point the batched join uses so the
    /// `BatchedShares` dealt up front by [`BatchedHiddenInput::shared_keys`] are the
    /// actual sharings compared — the keys are dealt ONCE, not re-shared per
    /// candidate. `sa` / `sb` must be degree-`t` sharings on the dealer's `n`-party
    /// points (exactly what `share` / `share_batch` produce).
    ///
    /// `m = (a - b)·r` is opened at degree `2t`; `m == 0 ⇔ a == b`, and for unequal
    /// keys `m` is uniform nonzero — so ONLY the match bit is revealed, never either
    /// key or their difference.
    ///
    /// **Integrity envelope — DETECT-only, never CORRECT (bead sq-ji5f; decision
    /// = the honest envelope).** The degree-`2t` open routes through
    /// [`shamir::reconstruct_degree`] → the WI-1 Reed–Solomon checker, but the
    /// honest-majority constructor fixes `t = ⌊(n−1)/2⌋`, so the equality open's
    /// correction budget at degree `2t` is `e_max = ⌊(n − (2t+1))/2⌋ = 0` for
    /// EVERY party count the constructor builds:
    ///
    /// - **odd `n`** (`n = 2t+1`): zero RS redundancy at degree `2t` ⇒ tampering
    ///   is information-theoretically undetectable (a MAC is the deferred WI-4
    ///   fix, bead sq-6d6g) — NOT claimed otherwise;
    /// - **even `n`** (`n = 2t+2`): exactly one redundant share ⇒ a tampered
    ///   product share is DETECTED and the open aborts with [`MpcError::Tampered`].
    ///
    /// So this primitive can at best **detect-and-abort** under an honest majority
    /// and NEVER auto-corrects a cheater. Robust correction (`e_max ≥ 1`) at
    /// degree `2t` would need `n ≥ 2t+3`, which is deliberately NOT provisioned
    /// here: the bead weighed (a) over-provisioning the party set, (b) a
    /// BGW/DN degree-reduction round before the open, and (c) leaving
    /// detect-and-abort as the honest envelope, and chose (c) — robustness is out
    /// of scope for the honest-majority equality test and would change the trust
    /// model. The arithmetic and behaviour are pinned by the `sq-ji5f` properties
    /// in `adversarial_tests` (`honest_majority_equality_open_*`).
    ///
    /// **Randomness routed through the seam (sq-yyro, issue #6117).** The nonzero
    /// equality mask `[r]` now comes from
    /// [`DistributedRandomness::shared_nonzero_mask`] instead of the dealer's own
    /// `draw_nonzero_fp` + `share`. For [`ShamirDealer`] that method is exactly
    /// those two calls in that order, so the drawn values — and the opened `m` —
    /// are unchanged; `(n, t)` are likewise read off the source. The `r ≠ 0`
    /// structural contract that this verdict rests on is the seam method's, and the
    /// ACTIVE-adversary `r = 0` defense remains a follow-on bead (see the
    /// [`crate::randomness`] module note) — nothing stronger is claimed here.
    fn secure_equal_shared(
        &self,
        source: &mut dyn DistributedRandomness,
        sa: &[Share],
        sb: &[Share],
    ) -> Result<bool, MpcError> {
        let (n, t) = Self::equality_params(source)?;
        Self::secure_equal_core(source, n, t, sa, sb)
    }

    /// The masked-product equality core shared by the scalar `secure_equal` and
    /// [`Self::secure_equal_shared`], over the source's already-validated
    /// `(n, t)` ([`Self::equality_params`]).
    fn secure_equal_core(
        source: &mut dyn DistributedRandomness,
        n: usize,
        t: usize,
        sa: &[Share],
        sb: &[Share],
    ) -> Result<bool, MpcError> {
        // Fresh nonzero mask r through the randomness seam (the single-dealer
        // simulation draws it from its masking CSPRNG — sq-1vt). Nonzero is
        // load-bearing and part of the seam method's structural contract: masking
        // by zero would make m=0 even for unequal keys (a false match). Its shape is
        // checked against the validated `n` before it is paired with `d`.
        let r = checked_shared_nonzero_mask(source, n)?;
        // d = a - b (local, degree t).
        let d = shamir::sub_shares(sa, sb)?;
        // m = d * r (one multiplication, degree 2t).
        let m_shares = shamir::mul_shares_raw(&d, &r)?;
        // Open m at degree 2t. m == 0  <=>  d == 0  <=>  a == b.
        let m = shamir::reconstruct_degree(&m_shares, 2 * t)?;
        Ok(m == Fp::zero())
    }
}

// =====================================================================
// sq-khf9 — the BATCHED hidden-value join: wire the
// `BatchedShares` / `RowBinding` primitive (sq-dwb5) + the oblivious output
// transform (`oblivious_join`, sq-jnkm) into the hidden join so it ranges over
// a COLUMN of rows per holder under a documented row-binding, with the output
// cardinality / ordering hidden.
// =====================================================================

/// A holder's BATCHED contribution to a hidden-value join: a COLUMN of rows
/// addressed under an explicit [`RowBinding`] (sq-dwb5). The single-scalar
/// [`HiddenKeyedRows`] caps a holder to an unstructured bag of rows; this carries
/// the same per-row `(private_key_fp, disclosed_payload)` pairs **plus the
/// row-binding contract** that says how rows correlate across holders.
///
/// Why a dedicated type (vs reusing [`HiddenKeyedRows`]). The row-binding is
/// load-bearing for correctness: it decides which `(i, j)` pairs are even
/// *candidates* (every pair under `Positional` aligned columns vs only same-key
/// pairs under `Keyed`). It must travel WITH the rows so the join cannot silently
/// correlate them the wrong way (the `batched.rs` contract).
///
/// ## Privacy of the key column (the sq-dwb5 primitive, actually used)
///
/// [`Self::shared_keys`] runs the holder's private key column through
/// [`DistributedRandomness::vss_own_input`], one row at a time (for the
/// single-dealer simulation, exactly [`crate::shamir::ShamirDealer::share_batch`])
/// → a [`BatchedShares`], so each
/// row's key gets its OWN fresh degree-`t` masking polynomial and any `≤ t`
/// parties' shares of the whole key column are jointly independent of every key
/// (the batched-Shamir hiding property). This is the structural reason the keys
/// never leak: the cleartext keys are used ONLY as the local inputs to Shamir
/// sharing and to the secure-equality test (`HiddenValueJoin::secure_equal_shared`),
/// and are NEVER reconstructed from shares — exactly as the four-flatmates aggregate
/// uses its salary column. The DISCLOSED payload columns ride alongside in the clear
/// (disclosure-minimisation, convention #4 — only the *key* is hidden).
/// `Debug` is hand-written for the same reason as [`HiddenKeyedRows`]: `rows`
/// carries the private key of each row next to its payload. The binding IS printed
/// — a [`RowBinding::Keyed`] holds the DISCLOSED public row keys (convention #4).
#[derive(Clone)]
pub struct BatchedHiddenInput {
    /// The holder that owns this column.
    pub holder: HolderId,
    /// Disclosed payload column names (NOT including the hidden key).
    pub payload_vars: Vec<Variable>,
    /// One entry per row: `(private_key_fp, disclosed_payload_terms)`, in this
    /// holder's local row order.
    pub rows: Vec<(Fp, Vec<Option<Term>>)>,
    /// How row `i` correlates across holders (see [`RowBinding`]).
    pub binding: RowBinding,
}

impl std::fmt::Debug for BatchedHiddenInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchedHiddenInput")
            .field("holder", &self.holder)
            .field("payload_vars", &self.payload_vars)
            .field("rows", &"<redacted>")
            .field("row_count", &self.rows.len())
            .field("binding", &self.binding)
            .finish()
    }
}

impl BatchedHiddenInput {
    /// Assemble a batched hidden input, validating the [`RowBinding::Keyed`] key
    /// count against the row count (a mismatch is an ambiguous binding → a protocol
    /// error, mirroring [`BatchedShares::new`]).
    pub fn new(
        holder: HolderId,
        payload_vars: Vec<Variable>,
        rows: Vec<(Fp, Vec<Option<Term>>)>,
        binding: RowBinding,
    ) -> Result<Self, MpcError> {
        if let RowBinding::Keyed(keys) = &binding {
            if keys.len() != rows.len() {
                return Err(MpcError::Protocol(format!(
                    "BatchedHiddenInput: keyed binding has {} keys but {} rows",
                    keys.len(),
                    rows.len()
                )));
            }
        }
        Ok(BatchedHiddenInput {
            holder,
            payload_vars,
            rows,
            binding,
        })
    }

    /// Number of rows in this batch.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the batch is empty (a holder with no private rows).
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Secret-share this holder's private KEY column as a [`BatchedShares`] under
    /// the same [`RowBinding`] — the sq-dwb5 batched primitive, actually used. Each
    /// key gets a fresh degree-`t` masking polynomial; the cleartext column is
    /// consumed only to deal the batch.
    ///
    /// Each key is drawn through the [`DistributedRandomness`] seam
    /// ([`DistributedRandomness::vss_own_input`]), in row order — for
    /// [`ShamirDealer`] exactly the [`ShamirDealer::share_batch`] it always was, so
    /// the shares are unchanged.
    pub fn shared_keys(&self, dealer: &mut ShamirDealer) -> Result<BatchedShares, MpcError> {
        let n = dealer.parties();
        self.shared_keys_through(dealer, n)
    }

    /// [`Self::shared_keys`] over any randomness source: one
    /// [`DistributedRandomness::vss_own_input`] per row, in row order, each refused
    /// unless it is the canonical sharing for the caller's (already validated)
    /// party count `n`. A source that cannot share a holder's own input (PRSS
    /// today) fails closed with its own error. Shape checks only.
    pub(crate) fn shared_keys_through(
        &self,
        source: &mut dyn DistributedRandomness,
        n: usize,
    ) -> Result<BatchedShares, MpcError> {
        let shares = self
            .rows
            .iter()
            .map(|(k, _)| checked_vss_own_input(source, n, *k))
            .collect::<Result<Vec<_>, _>>()?;
        BatchedShares::new(shares, self.binding.clone())
    }
}

/// The result of a batched hidden join: the oblivious [`PartialResult`] (real rows
/// only, dummies filtered, federation-attributed, canonical order) plus the
/// MODELLED cost of the oblivious output transform.
pub type BatchedJoinOutput = (PartialResult, ObliviousOutputCost);

impl HiddenValueJoin {
    /// The bound contract every candidate-materialising entry runs FIRST: refuse an
    /// over-cap `B`, then count the candidates the binding permits without
    /// enumerating them, then require `B` to cover that count.
    ///
    /// Each candidate costs one equality sharing on the way to the output stage and
    /// one materialised slot inside it, so leaving the cardinality to the output
    /// stage's own check means the work is already done by the time it refuses.
    ///
    /// The `B` cap comes FIRST and on its own, because counting is not free: a
    /// `Keyed` binding counts the fan-out through a map keyed by the right-hand
    /// rows, so an over-cap `B` would otherwise allocate that map before being
    /// refused. Once the cap has passed, the map is sized by the caller's own row
    /// count — work they asked for — and the coverage check follows the count.
    fn preflight_candidate_bound(
        &self,
        left: &BatchedHiddenInput,
        right: &BatchedHiddenInput,
        bound: usize,
    ) -> Result<usize, MpcError> {
        oblivious_join::check_output_bound_cap(bound)?;
        let count = candidate_pair_count(&left.binding, &right.binding, left.len(), right.len())?;
        oblivious_join::check_output_bound(bound, count)?;
        Ok(count)
    }

    /// **The batched hidden-value join (sq-khf9).** Join two holders' BATCHED
    /// row-columns on a PRIVATE key under an explicit candidate binding and public
    /// bound, and route the result through the
    /// **oblivious output transform** ([`crate::oblivious_join`]) so the output
    /// cardinality (bounded to the public `bound` `B`) and ordering reveal nothing
    /// about WHICH rows matched.
    ///
    /// ## How the batched inputs + [`RowBinding`] wire in
    ///
    /// 1. Each holder's private key column is secret-shared as a [`BatchedShares`]
    ///    via [`BatchedHiddenInput::shared_keys`] (the sq-dwb5 primitive): the
    ///    cleartext keys never leave, only the batched shares, and any `≤ t` parties'
    ///    shares of the whole column are jointly independent of every key.
    /// 2. The [`RowBinding`] decides the CANDIDATE pairs (the row-dimension lift of
    ///    the secure-equal):
    ///    - [`RowBinding::Positional`] — row `i` of L pairs ONLY with row `i` of R
    ///      (index-correlated). Both batches must be the same length (aligned
    ///      columns); a mismatch fails closed. `k` candidate pairs.
    ///    - [`RowBinding::Keyed`] — rows correlate by the DISCLOSED public row key:
    ///      row `i` of L pairs with row `j` of R iff `lkeys[i] == rkeys[j]`. The keys
    ///      are disclosed (convention #4); the VALUE stays hidden. Correlation is by
    ///      key, not index, so the orders need not match.
    /// 3. For each candidate pair the match is decided by the existing
    ///    `Self::secure_equal` over the secret-shared keys — the join key/value is
    ///    NEVER opened; only the match bit is (see the honesty note).
    /// 4. The candidates feed
    ///    [`crate::oblivious_join::oblivious_join_output`] (drawing from the same
    ///    randomness source as steps 1–3) with
    ///    [`MatchBit::Public`] bits and the public bound `B`: non-matches become
    ///    indistinguishable dummies, the slots are oblivious-shuffled, and exactly
    ///    `B` are revealed — so the OUTPUT does not leak which input pair produced
    ///    which row, and the revealed output size is `B`. The true count depends on
    ///    the OUTPUT MODE: this path takes the ALL-PARTY one, where every compute
    ///    party opens the `B` tags and so learns the exact count; the recipient-only
    ///    mode ([`crate::oblivious_join::oblivious_set_output_to_recipient`]) leaves
    ///    the count to the designated recipient alone.
    ///
    /// ## Privacy property enforced (state it precisely — empirical-honesty rule)
    ///
    /// - **The join KEY/VALUE is never reconstructed.** Only `secure_equal`'s masked
    ///   product `m = d·r` is opened per candidate; `m = 0 ⇔ keys equal`, and for
    ///   unequal keys `m` is uniform nonzero — it reveals ONLY the match bit, never
    ///   either key or their difference (the sq-dwb5 batched-Shamir hiding lifts this
    ///   to the whole column: ≤ t parties' views are independent of every key).
    /// - **The OUTPUT ordering is oblivious** to the protocol/parties: shuffled so
    ///   position → candidate linkage is destroyed (`oblivious_join` L2-destroyed at
    ///   the OUTPUT), and the cardinality is bounded to the public `B` (L1-bounded).
    ///   The exact count is NOT hidden from the compute parties on this path: it is
    ///   the all-party output mode, whose tag opens are in every compute party's
    ///   transcript. Keeping the count out of their transcript is the recipient-only
    ///   mode ([`crate::oblivious_join::oblivious_set_output_to_recipient`]); hiding
    ///   it from the recipient as well is the separate DP-cardinality mode
    ///   (bead sq-shk5).
    ///
    /// HONESTY (the gated half — NOT faked). The per-candidate match BIT is still
    /// *opened* by `secure_equal_shared`: deriving a SECRET-shared match bit WITHOUT
    /// opening it needs a secure equality-to-shared-bit (bead sq-rrz4 on the
    /// degree-reduction round sq-dvuc), the gate `oblivious_join`'s hidden-key entry
    /// point names. So this closes the OUTPUT leaks (L1/L2 of the result set) but not
    /// the decision-time match-graph leak; the fully-oblivious upgrade is a follow-up
    /// bead. We do NOT claim the match graph is hidden from the parties. Semi-honest
    /// only, unchanged from the [`ShamirBackend`] layer.
    ///
    /// ## Fail-closed contracts
    ///
    /// - `Positional`: both batches must be the same length (aligned columns) — else
    ///   [`MpcError::Protocol`]. The candidate set is the `k` index-aligned pairs.
    /// - `Keyed`: both batches must be `Keyed`; mixing bindings is a protocol error
    ///   (the correlation rule would be ambiguous). Keys are matched in the clear.
    /// - Mixed bindings (one `Positional`, one `Keyed`) → protocol error.
    /// - `bound` must cover the candidate count (`oblivious_join_output` fails closed
    ///   otherwise — never truncating a candidate, which could drop a true match).
    /// - Both holders must agree on the disclosed payload arity per side; the output
    ///   schema is `left.payload_vars ++ right.payload_vars`.
    pub fn batched_join(
        &self,
        left: &BatchedHiddenInput,
        right: &BatchedHiddenInput,
        bound: usize,
    ) -> Result<BatchedJoinOutput, MpcError> {
        // Preflight before a dealer exists: minting one is cheap, but it is still
        // keystream state created for a run that is about to be refused.
        self.preflight_candidate_bound(left, right, bound)?;
        // Mint ONE fresh dealer for this protocol run (production: a fresh OS-seeded
        // CSPRNG, sq-1vt) — independent masks, never a reused keystream across joins
        // — and one source for the whole run: the key sharings, the equality masks
        // AND the oblivious output stage draw from it in sequence, where before the
        // output stage minted a second dealer of its own.
        self.batched_join_through(&mut self.backend.dealer(), left, right, bound)
    }

    /// [`Self::batched_join`] with the key sharings, the equality masks AND the
    /// whole oblivious output stage (selector / real-tag / payload-limb sharings
    /// and the shuffle) drawn through ONE `source`, in that order — where the
    /// output stage previously minted a second dealer from the backend and
    /// restarted the keystream. Refuses a source whose `(n, t)` differs from the
    /// backend's before any draw; every drawn sharing is shape-checked where it is
    /// consumed, and the shuffled column's shape on return. Shape checks only.
    pub(crate) fn batched_join_through(
        &self,
        source: &mut dyn DistributedRandomness,
        left: &BatchedHiddenInput,
        right: &BatchedHiddenInput,
        bound: usize,
    ) -> Result<BatchedJoinOutput, MpcError> {
        // Validated before any draw or allocation: the candidates this entry builds
        // below cost a draw each, and the output stage sizes its allocations and its
        // shuffle network from `bound`. So the candidate CARDINALITY is counted
        // without materialising the pairs, and `B` must be within the cap and cover
        // it (see `candidate_pair_count`).
        self.preflight_candidate_bound(left, right, bound)?;
        require_source_matches(source, &self.backend)?;
        let n = self.backend.parties();

        // Output schema: left payload vars then right payload vars (the key is NOT
        // projected — it stays hidden).
        let mut out_vars = left.payload_vars.clone();
        out_vars.extend(right.payload_vars.iter().cloned());
        let payload_arity = out_vars.len();

        // (1) Secret-share each holder's private key column as a BatchedShares (the
        // sq-dwb5 primitive): each row's key gets its OWN fresh degree-`t` masking
        // polynomial, the cleartext column is consumed ONCE into the batch, and the
        // keyed-binding key counts are validated. These are the SAME sharings the
        // comparisons below consume — the keys are dealt once, never re-shared per
        // candidate.
        let l_shared = left.shared_keys_through(source, n)?;
        let r_shared = right.shared_keys_through(source, n)?;

        // (2) Enumerate the candidate row pairs the RowBinding permits.
        let pairs = candidate_pairs(&left.binding, &right.binding, left.len(), right.len())?;

        // (3) Decide each candidate's match with the secure-equal primitive lifted to
        // the row dimension — over the BATCHED shares dealt in (1), NOT a per-row
        // re-share. The join KEY/VALUE is never opened — only the match bit.
        let mut candidates: Vec<Candidate> = Vec::with_capacity(pairs.len());
        for (li, rj) in pairs {
            let (_lkey, lpay) = &left.rows[li];
            let (_rkey, rpay) = &right.rows[rj];
            let matched = self.secure_equal_shared(
                source,
                &l_shared.elements()[li],
                &r_shared.elements()[rj],
            )?;
            let mut payload = lpay.clone();
            payload.extend(rpay.iter().cloned());
            // Pad / validate to the uniform output arity so the oblivious output's
            // recipient sees one schema (a short row is a malformed input, not a
            // silent truncation).
            if payload.len() != payload_arity {
                return Err(MpcError::Protocol(format!(
                    "batched_join: candidate payload arity {} != schema arity {payload_arity} \
                     (left {} + right {} payload vars)",
                    payload.len(),
                    left.payload_vars.len(),
                    right.payload_vars.len()
                )));
            }
            candidates.push(Candidate {
                payload,
                matched: MatchBit::Public(matched),
            });
        }

        // (4) Route through the oblivious output transform: hide which candidates
        // matched (shuffle) and bound the revealed cardinality to the public `B`.
        // It draws from the SAME `source` the key sharings and equality masks came
        // from, so one continuous source covers the whole join.
        oblivious_join::oblivious_join_output_through(
            source,
            &self.backend,
            &candidates,
            out_vars,
            bound,
        )
    }

    /// **The fully-oblivious batched hidden-value join (sq-xhaw).** The upgrade
    /// [`Self::batched_join`] explicitly deferred: decide each candidate pair's
    /// match as a **secret-shared bit that is NEVER opened per pair**, so the join
    /// leaks nothing per pair — only the final oblivious output (bounded to the
    /// public `B`, shuffled) is revealed.
    ///
    /// ## What changes vs [`Self::batched_join`] (and why it matters)
    ///
    /// [`Self::batched_join`] decides each match with `secure_equal`, which OPENS
    /// the masked product `m = (a−b)·r` per candidate. That open IS the match bit at
    /// `(i, j)` — and the set of true `(i, j)` is the **bipartite match graph / key
    /// fan-out** (leak L2 at the DECISION; `research/mpc-sparql-capability-matrix.md`
    /// §4.2). `batched_join` therefore closes the OUTPUT leaks (L1/L2 of the result
    /// set, via the oblivious shuffle + padded reveal) but NOT the decision-time
    /// match-graph leak — it states this honestly in its own doc.
    ///
    /// This entry point closes that last leak: it computes each match bit with
    /// [`crate::compare::secure_equal_to_bit`] — a bit-decomposition equality whose
    /// 0/1 verdict stays a fresh degree-`t` **secret-shared** sharing — and feeds it
    /// as a [`MatchBit::SecretShared`] selector into the SAME landed oblivious output
    /// transform ([`crate::oblivious_join::oblivious_join_output`], sq-jnkm). The
    /// per-candidate match bit is therefore never reconstructed anywhere in the
    /// protocol; the only thing ever opened is the `B` shuffled output tags (which
    /// classify a slot as a real row or a dummy AFTER the permutation destroyed
    /// position→candidate linkage). No per-pair open ⇒ no per-pair leak.
    ///
    /// ## Cost (honest — this is the expensive path)
    ///
    /// `secure_equal_to_bit` is a bit-decomposition + AND-tree of secure
    /// multiplications ([`crate::compare::COMPARE_BITS`] equalities + `COMPARE_BITS
    /// − 1` ANDs, each one [`shamir::mul_shares_raw`] + one
    /// [`crate::shamir::ShamirDealer::degree_reduce`] round), per candidate pair —
    /// **vastly** more multiplication rounds than `batched_join`'s single masked-open
    /// per pair. The reward is the per-pair confidentiality. This is the LAN,
    /// `O(L)`-round profile the comparison module documents; the constant-round
    /// (Rabbit/edaBits) speedup is the same future seam `compare` defers. We do not
    /// extrapolate beyond the `secure_equal`/comparison feasibility envelope.
    ///
    /// ## Privacy property enforced (state it precisely — empirical-honesty rule)
    ///
    /// - **Per pair: nothing is opened.** The match bit is a secret-shared 0/1; the
    ///   keys/values are never reconstructed (bit-decomposition deals only the bit
    ///   shares, exactly like [`crate::compare::secure_greater_than`]). The L2
    ///   match-graph leak `batched_join` has is **eliminated at the decision**.
    /// - **Output: oblivious.** Cardinality bounded to the public `B`, shuffled so
    ///   position→candidate linkage is destroyed (sq-jnkm transform, unchanged).
    /// - **Operand range.** Both holders' private keys must be `< 2^COMPARE_BITS`
    ///   (`secure_equal_to_bit` fails closed otherwise so the bit-decomposition is
    ///   injective and field-equality ⇔ recovered-bit-equality).
    ///
    /// HONESTY (NOT an overclaim; privacy-claims gate is live, cite sq-qhy4).
    /// Semi-honest, honest-majority ONLY — unchanged from the [`ShamirBackend`] /
    /// `compare` layer: every multiplication routes through `degree_reduce`, which
    /// has no in-protocol check that a deviating party re-shared honestly, so this is
    /// NOT maliciously secure. This closes a *confidentiality* axis (the per-pair
    /// match bit), which is orthogonal to malicious security; the external soundness
    /// sign-off is still pending (sq-qhy4). No soundness/security guarantee beyond
    /// the documented semi-honest model is claimed.
    ///
    /// ## Fail-closed contracts (same as [`Self::batched_join`])
    ///
    /// - `Positional`: both batches must be equal length; `Keyed`: both `Keyed`;
    ///   mixed bindings → [`MpcError::Protocol`].
    /// - `bound` must cover the candidate count (`oblivious_join_output` fails closed
    ///   otherwise — never truncating a candidate, which could drop a true match).
    /// - Output schema is `left.payload_vars ++ right.payload_vars` (the key is NOT
    ///   projected — it stays hidden).
    pub fn fully_oblivious_batched_join(
        &self,
        left: &BatchedHiddenInput,
        right: &BatchedHiddenInput,
        bound: usize,
    ) -> Result<BatchedJoinOutput, MpcError> {
        // Preflight before a dealer exists, as in `batched_join`.
        self.preflight_candidate_bound(left, right, bound)?;
        // ONE fresh dealer for this protocol run (production: fresh OS-seeded
        // CSPRNG, sq-1vt) — independent masks, never a reused keystream across
        // joins — and now one source for the whole run: the equality bits AND the
        // oblivious output stage draw from it in sequence, where before the output
        // stage minted a second dealer of its own.
        self.fully_oblivious_batched_join_through(&mut self.backend.dealer(), left, right, bound)
    }

    /// [`Self::fully_oblivious_batched_join`] with the per-pair equality bits AND
    /// the whole oblivious output stage (selector / tag / payload-limb sharings and
    /// the shuffle) drawn through ONE `source`, in that order. Refuses a source
    /// whose `(n, t)` differs from the backend's before any draw; every drawn
    /// sharing is shape-checked where it is consumed, and the shuffled column's
    /// shape on return. Shape checks only.
    pub(crate) fn fully_oblivious_batched_join_through(
        &self,
        source: &mut dyn DistributedRandomness,
        left: &BatchedHiddenInput,
        right: &BatchedHiddenInput,
        bound: usize,
    ) -> Result<BatchedJoinOutput, MpcError> {
        // Validated before any draw or allocation, as in `batched_join_through`.
        self.preflight_candidate_bound(left, right, bound)?;
        require_source_matches(source, &self.backend)?;

        // Output schema: left payload vars then right payload vars (key NOT projected).
        let mut out_vars = left.payload_vars.clone();
        out_vars.extend(right.payload_vars.iter().cloned());
        let candidates = self.fully_oblivious_candidates(source, left, right)?;

        // (3) Route through the oblivious output transform with the SECRET-SHARED
        // selectors: the oblivious select consumes the shared bit directly (one
        // `mul_shares_raw` per slot, no open), the slots are shuffled, and exactly
        // `B` tags are revealed. Nothing per-pair is opened anywhere.
        oblivious_join::oblivious_join_output_through(
            source,
            &self.backend,
            &candidates,
            out_vars,
            bound,
        )
    }

    /// Steps (1) and (2) of [`Self::fully_oblivious_batched_join_through`]: the
    /// candidate row pairs the row-binding permits, each carrying its joined
    /// payload row and a SECRET-SHARED match bit that is never opened. Split out so
    /// the candidates (and the payload rows they carry) are reachable without
    /// re-running the output stage.
    fn fully_oblivious_candidates(
        &self,
        source: &mut dyn DistributedRandomness,
        left: &BatchedHiddenInput,
        right: &BatchedHiddenInput,
    ) -> Result<Vec<Candidate>, MpcError> {
        let payload_arity = left.payload_vars.len() + right.payload_vars.len();

        // (1) Enumerate the candidate row pairs the RowBinding permits — the SAME
        // public candidate structure as `batched_join` (the candidate COUNT is a
        // public MPC assumption; only the per-pair match VERDICT is hidden here).
        let pairs = candidate_pairs(&left.binding, &right.binding, left.len(), right.len())?;

        // (2) Decide each candidate's match as a SECRET-SHARED bit — NEVER opened.
        // `secure_equal_to_bit` bit-decomposes both private keys and returns a fresh
        // degree-`t` sharing of `1{lkey == rkey}`; the keys are never reconstructed
        // and the verdict is never opened (the L2-at-decision leak `batched_join`
        // has is closed here). The bit becomes a `MatchBit::SecretShared` selector.
        let mut candidates: Vec<Candidate> = Vec::with_capacity(pairs.len());
        for (li, rj) in pairs {
            let (lkey, lpay) = &left.rows[li];
            let (rkey, rpay) = &right.rows[rj];
            let match_bit = secure_equal_to_bit_through(source, *lkey, *rkey)?;
            let mut payload = lpay.clone();
            payload.extend(rpay.iter().cloned());
            // Uniform output arity for the recipient (a short row is a malformed
            // input, never a silent truncation).
            if payload.len() != payload_arity {
                return Err(MpcError::Protocol(format!(
                    "fully_oblivious_batched_join: candidate payload arity {} != schema arity \
                     {payload_arity} (left {} + right {} payload vars)",
                    payload.len(),
                    left.payload_vars.len(),
                    right.payload_vars.len()
                )));
            }
            candidates.push(Candidate {
                payload,
                matched: MatchBit::SecretShared(match_bit),
            });
        }
        Ok(candidates)
    }

    /// Test-only instrumentation: the candidates
    /// [`Self::fully_oblivious_batched_join_through`] builds, together with the
    /// `(is_real, decoded_row)` of every slot the REAL output stage opens for them.
    /// Same candidate construction and same output stage as the public path — it
    /// just keeps the dummy slots' decoded payload rows, which `PartialResult`
    /// cannot express.
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    pub(crate) fn fully_oblivious_candidates_and_opened_rows(
        &self,
        source: &mut dyn DistributedRandomness,
        left: &BatchedHiddenInput,
        right: &BatchedHiddenInput,
        bound: usize,
    ) -> Result<(Vec<Candidate>, Vec<(bool, Vec<Option<Term>>)>), MpcError> {
        self.preflight_candidate_bound(left, right, bound)?;
        require_source_matches(source, &self.backend)?;
        let payload_arity = left.payload_vars.len() + right.payload_vars.len();
        let candidates = self.fully_oblivious_candidates(source, left, right)?;
        let slots = oblivious_join::opened_slot_rows_through(
            source,
            &self.backend,
            &candidates,
            payload_arity,
            bound,
        )?;
        Ok((candidates, slots))
    }
}

/// Enumerate the candidate `(left_row, right_row)` index pairs the row-binding
/// permits — the row-dimension lift of which pairs the secure-equal even tests.
///
/// - [`RowBinding::Positional`] (both sides): index-correlated. The two columns
///   must be the same length (aligned positional columns, the `batched.rs`
///   contract); the candidates are the `k` pairs `(i, i)`.
/// - [`RowBinding::Keyed`] (both sides): correlated by the DISCLOSED public row
///   key. Candidate `(i, j)` iff `lkeys[i] == rkeys[j]` — a clear-text bucketed
///   join over the disclosed keys (convention #4); the VALUE stays hidden and is
///   still decided by `secure_equal`.
/// - Mixed bindings are a protocol error (the correlation rule is ambiguous).
fn candidate_pairs(
    left: &RowBinding,
    right: &RowBinding,
    llen: usize,
    rlen: usize,
) -> Result<Vec<(usize, usize)>, MpcError> {
    match (left, right) {
        (RowBinding::Positional, RowBinding::Positional) => {
            if llen != rlen {
                return Err(MpcError::Protocol(format!(
                    "batched_join (Positional): left has {llen} rows but right has {rlen} — \
                     index-correlated columns must be equal length"
                )));
            }
            Ok((0..llen).map(|i| (i, i)).collect())
        }
        (RowBinding::Keyed(lkeys), RowBinding::Keyed(rkeys)) => {
            // Bucket the right rows by disclosed key, then emit (i, j) for every
            // left row whose key hits a bucket — a clear-text equi-join over the
            // DISCLOSED keys (the value stays hidden, decided by secure_equal).
            let mut right_by_key: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
            for (j, k) in rkeys.iter().enumerate() {
                right_by_key.entry(k.as_str()).or_default().push(j);
            }
            let mut pairs = Vec::new();
            for (i, k) in lkeys.iter().enumerate() {
                if let Some(js) = right_by_key.get(k.as_str()) {
                    for &j in js {
                        pairs.push((i, j));
                    }
                }
            }
            Ok(pairs)
        }
        _ => Err(MpcError::Protocol(
            "batched_join: mixed row-bindings (one Positional, one Keyed) — \
             both holders must share the same correlation contract"
                .into(),
        )),
    }
}

/// The candidate count [`candidate_pairs`] would enumerate, computed WITHOUT
/// materialising the pairs — the preflight every builder runs before it allocates
/// a candidate or draws a sharing.
///
/// A `Keyed` binding fans out: the pair count is `Σ_i |bucket(key_i)|`, which can
/// reach `llen · rlen`. Materialising the pairs to learn that count is the DoS
/// itself, and the output stage's own `B >= candidates` check comes too late — one
/// equality sharing is drawn per pair on the way there. So this counts with
/// [`usize::checked_add`] and refuses on overflow, touching no holder data and
/// allocating only the key buckets a clear-text equi-join needs anyway.
///
/// Those buckets ARE an allocation, sized by the caller's own row count, so the
/// caller runs the `B` cap before calling this — see `preflight_candidate_bound`.
///
/// It mirrors [`candidate_pairs`] exactly, and
/// `candidate_pair_count_matches_candidate_pairs` pins that it does.
fn candidate_pair_count(
    left: &RowBinding,
    right: &RowBinding,
    llen: usize,
    rlen: usize,
) -> Result<usize, MpcError> {
    match (left, right) {
        (RowBinding::Positional, RowBinding::Positional) => {
            if llen != rlen {
                return Err(MpcError::Protocol(format!(
                    "batched_join (Positional): left has {} rows but right has {} — \
                     index-correlated columns must be equal length",
                    llen, rlen
                )));
            }
            Ok(llen)
        }
        (RowBinding::Keyed(lkeys), RowBinding::Keyed(rkeys)) => {
            let mut right_counts: BTreeMap<&str, usize> = BTreeMap::new();
            for k in rkeys.iter() {
                *right_counts.entry(k.as_str()).or_default() += 1;
            }
            let mut total: usize = 0;
            for k in lkeys.iter() {
                if let Some(c) = right_counts.get(k.as_str()) {
                    total = total.checked_add(*c).ok_or_else(|| {
                        MpcError::Protocol(
                            "batched_join (Keyed): the candidate fan-out overflows usize \
                             — refusing before any candidate is materialised or any \
                             sharing drawn"
                                .to_string(),
                        )
                    })?;
                }
            }
            Ok(total)
        }
        _ => Err(MpcError::Protocol(
            "batched_join: mixed row-bindings (one Positional, one Keyed) — \
             both holders must share the same correlation contract"
                .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    //! M2 tests: the disclosed-key global-IRI equi-join. The load-bearing one is
    //! `differential_*`: the federated join of per-holder partials must equal
    //! evaluating the WHOLE query over the UNION of the holders' graphs in a
    //! single `sparq-engine` store. This is what makes the crypto-free join a
    //! faithful stand-in for PAG evaluation over `D = ⋃ holder graphs`.
    use super::*;
    use crate::holder::Holder;
    use sparq_core::Graph;
    use sparq_engine::query;

    const PFX: &str = "@prefix ex: <http://ex/> .\n";

    fn var(n: &str) -> Variable {
        Variable::new_unchecked(n)
    }

    /// Render a (vars, rows) result as a canonical, order-independent multiset of
    /// rows where each row is a sorted list of `(?var, term-debug)` pairs. Two
    /// results are equal as SPARQL solution multisets iff this is equal. Used to
    /// compare the federated join against the union-store evaluation regardless
    /// of column order or row order.
    fn canonical_multiset(
        vars: &[Variable],
        rows: &[Vec<Option<Term>>],
    ) -> Vec<Vec<(String, String)>> {
        let mut out: Vec<Vec<(String, String)>> = rows
            .iter()
            .map(|row| {
                let mut pairs: Vec<(String, String)> = vars
                    .iter()
                    .zip(row.iter())
                    .map(|(v, t)| {
                        let val = match t {
                            Some(t) => format!("{t:?}"),
                            None => "<UNBOUND>".to_string(),
                        };
                        (v.as_str().to_string(), val)
                    })
                    .collect();
                pairs.sort();
                pairs
            })
            .collect();
        out.sort();
        out
    }

    /// Build a single union store from several turtle documents (the "evaluate
    /// the whole query over D = ⋃ holder graphs" side of the differential).
    fn union_graph(docs: &[&str]) -> Graph {
        let mut combined = String::from(PFX);
        for d in docs {
            combined.push_str(d);
            combined.push('\n');
        }
        Graph::load_str(&combined, "turtle").expect("union graph parses")
    }

    /// THE differential test (two holders): Holder A has `?p ex:knows ?x`,
    /// Holder B has `?x ex:name ?n`. Each evaluates its OWN fragment locally;
    /// the disclosed-key join on the shared global IRI `?x` must equal the whole
    /// query `{ ?p ex:knows ?x . ?x ex:name ?n }` over the UNION store.
    #[test]
    fn differential_two_holder_join_equals_union_eval() {
        // Holder A: a "knows" graph. ?x values are GLOBAL IRIs shared with B.
        let a_doc = "ex:p1 ex:knows ex:x1 . ex:p1 ex:knows ex:x2 . ex:p2 ex:knows ex:x2 .";
        // Holder B: a "name" graph. Note ex:x3 has a name but nobody knows it,
        // and ex:x1/ex:x2 are the shared join keys.
        let b_doc = "ex:x1 ex:name \"Xena\" . ex:x2 ex:name \"Yuri\" . ex:x3 ex:name \"Zed\" .";

        let a = Holder::from_rdf("a", &format!("{PFX}{a_doc}"), "turtle").unwrap();
        let b = Holder::from_rdf("b", &format!("{PFX}{b_doc}"), "turtle").unwrap();

        let pa = a
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }")
            .unwrap();
        let pb = b
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?x ?n WHERE { ?x ex:name ?n }")
            .unwrap();

        let plan = JoinPlan {
            join_var: var("x"),
            key_disclosed: true,
        };
        let joined = DisclosedKeyJoin
            .join(&[pa, pb], &plan)
            .expect("disclosed-key join ok");

        // Federated result is attributed to the synthetic federation holder, not
        // to any single source.
        assert_eq!(joined.holder, HolderId::new("federation"));

        // Union-store evaluation of the whole query.
        let u = union_graph(&[a_doc, b_doc]);
        let expected = query(
            &u,
            "PREFIX ex: <http://ex/> SELECT ?p ?x ?n WHERE { ?p ex:knows ?x . ?x ex:name ?n }",
        )
        .unwrap();

        // DIFFERENTIAL: identical solution multisets.
        assert_eq!(
            canonical_multiset(&joined.vars, &joined.rows),
            canonical_multiset(&expected.vars, &expected.rows),
            "federated join must equal union-store evaluation"
        );
        // And concretely: p1-x1-Xena, p1-x2-Yuri, p2-x2-Yuri (x3 dropped: nobody
        // knows it; so 3 rows).
        assert_eq!(joined.rows.len(), 3);
    }

    /// Empty-result holder: if one holder discloses NO rows for its fragment, the
    /// inner join is empty — and that equals the union-store evaluation (an inner
    /// BGP join with an unsatisfied pattern yields nothing).
    #[test]
    fn empty_holder_yields_empty_join_matching_union() {
        let a_doc = "ex:p1 ex:knows ex:x1 .";
        let b_doc = "ex:p1 ex:age 30 ."; // no ex:name triples at all

        let a = Holder::from_rdf("a", &format!("{PFX}{a_doc}"), "turtle").unwrap();
        let b = Holder::from_rdf("b", &format!("{PFX}{b_doc}"), "turtle").unwrap();

        let pa = a
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }")
            .unwrap();
        let pb = b
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?x ?n WHERE { ?x ex:name ?n }")
            .unwrap();
        assert!(pb.is_empty(), "Holder B discloses no rows for its fragment");

        let plan = JoinPlan {
            join_var: var("x"),
            key_disclosed: true,
        };
        let joined = DisclosedKeyJoin.join(&[pa, pb], &plan).unwrap();
        assert!(joined.is_empty(), "join with an empty partial is empty");

        let u = union_graph(&[a_doc, b_doc]);
        let expected = query(
            &u,
            "PREFIX ex: <http://ex/> SELECT ?p ?x ?n WHERE { ?p ex:knows ?x . ?x ex:name ?n }",
        )
        .unwrap();
        assert!(expected.is_empty());
        assert_eq!(
            canonical_multiset(&joined.vars, &joined.rows),
            canonical_multiset(&expected.vars, &expected.rows),
        );
    }

    /// Multi-row fan-out: one join key shared by many rows on both sides exercises
    /// the cartesian product within a key bucket; still must equal the union eval.
    #[test]
    fn multi_row_fanout_join_equals_union() {
        // Two people both know x1; x1 has two names (multi-valued). Expect 2×2=4
        // joined rows on the x1 key.
        let a_doc = "ex:p1 ex:knows ex:x1 . ex:p2 ex:knows ex:x1 .";
        let b_doc = "ex:x1 ex:name \"A\" . ex:x1 ex:name \"B\" .";

        let a = Holder::from_rdf("a", &format!("{PFX}{a_doc}"), "turtle").unwrap();
        let b = Holder::from_rdf("b", &format!("{PFX}{b_doc}"), "turtle").unwrap();
        let pa = a
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }")
            .unwrap();
        let pb = b
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?x ?n WHERE { ?x ex:name ?n }")
            .unwrap();

        let plan = JoinPlan {
            join_var: var("x"),
            key_disclosed: true,
        };
        let joined = DisclosedKeyJoin.join(&[pa, pb], &plan).unwrap();
        assert_eq!(joined.rows.len(), 4, "2 knowers × 2 names on the x1 key");

        let u = union_graph(&[a_doc, b_doc]);
        let expected = query(
            &u,
            "PREFIX ex: <http://ex/> SELECT ?p ?x ?n WHERE { ?p ex:knows ?x . ?x ex:name ?n }",
        )
        .unwrap();
        assert_eq!(
            canonical_multiset(&joined.vars, &joined.rows),
            canonical_multiset(&expected.vars, &expected.rows),
        );
    }

    /// A 3-holder chain: A `?p knows ?x`, B `?x supervisedBy ?y`, C `?y name ?n`.
    /// Folded join on the chain must equal the 3-pattern BGP over the union of
    /// all three holders' graphs. (Each fold step joins on the shared var.)
    #[test]
    fn differential_three_holder_chain_equals_union() {
        let a_doc = "ex:p1 ex:knows ex:x1 . ex:p2 ex:knows ex:x2 .";
        let b_doc = "ex:x1 ex:supervisedBy ex:y1 . ex:x2 ex:supervisedBy ex:y1 . ex:x9 ex:supervisedBy ex:y2 .";
        let c_doc = "ex:y1 ex:name \"Boss\" . ex:y2 ex:name \"Other\" .";

        let a = Holder::from_rdf("a", &format!("{PFX}{a_doc}"), "turtle").unwrap();
        let b = Holder::from_rdf("b", &format!("{PFX}{b_doc}"), "turtle").unwrap();
        let c = Holder::from_rdf("c", &format!("{PFX}{c_doc}"), "turtle").unwrap();

        let pa = a
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }")
            .unwrap();
        let pb = b
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?x ?y WHERE { ?x ex:supervisedBy ?y }")
            .unwrap();
        let pc = c
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?y ?n WHERE { ?y ex:name ?n }")
            .unwrap();

        // The fold: A⋈B on ?x, then (A⋈B)⋈C on ?y. The JoinPlan names the key the
        // planner expects to be present in *all* partials; here we fold pairwise,
        // each step naming its shared var. (A single JoinPlan models one join;
        // the chain is two joins.)
        let ab = DisclosedKeyJoin
            .join(
                &[pa, pb],
                &JoinPlan {
                    join_var: var("x"),
                    key_disclosed: true,
                },
            )
            .unwrap();
        let abc = DisclosedKeyJoin
            .join(
                &[ab, pc],
                &JoinPlan {
                    join_var: var("y"),
                    key_disclosed: true,
                },
            )
            .unwrap();

        let u = union_graph(&[a_doc, b_doc, c_doc]);
        let expected = query(
            &u,
            "PREFIX ex: <http://ex/> SELECT ?p ?x ?y ?n WHERE { \
                 ?p ex:knows ?x . ?x ex:supervisedBy ?y . ?y ex:name ?n }",
        )
        .unwrap();
        assert_eq!(
            canonical_multiset(&abc.vars, &abc.rows),
            canonical_multiset(&expected.vars, &expected.rows),
        );
        // p1→x1→y1→Boss and p2→x2→y1→Boss; x9/y2 dropped (no knower). 2 rows.
        assert_eq!(abc.rows.len(), 2);
    }

    /// SOUNDNESS: the join must NOT trust the untrusted planner. If the planner
    /// names a join key a holder did not actually disclose, the join FAILS with a
    /// Protocol error — it does not silently produce an empty (or wrong) result a
    /// later proof layer could not back (§4.1).
    #[test]
    fn join_key_absent_from_a_partial_is_a_protocol_error() {
        let a = Holder::from_rdf("a", &format!("{PFX}ex:p1 ex:knows ex:x1 ."), "turtle").unwrap();
        let b = Holder::from_rdf("b", &format!("{PFX}ex:x1 ex:name \"X\" ."), "turtle").unwrap();
        let pa = a
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }")
            .unwrap();
        let pb = b
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?x ?n WHERE { ?x ex:name ?n }")
            .unwrap();
        // Planner names ?wrong — present in NEITHER partial.
        let plan = JoinPlan {
            join_var: var("wrong"),
            key_disclosed: true,
        };
        let err = DisclosedKeyJoin.join(&[pa, pb], &plan).unwrap_err();
        match err {
            MpcError::Protocol(m) => assert!(m.contains("did not disclose the join key")),
            other => panic!("expected Protocol error, got {other:?}"),
        }
    }

    /// An empty federation is a Protocol precondition violation, not a panic.
    #[test]
    fn empty_federation_is_a_protocol_error() {
        let plan = JoinPlan {
            join_var: var("x"),
            key_disclosed: true,
        };
        let err = DisclosedKeyJoin.join(&[], &plan).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(_)));
    }

    /// A single-holder "join" is the identity (its own disclosed partial,
    /// re-attributed to the federation) — and equals evaluating that fragment
    /// over the (single-holder) union. Edge case: the fold over one partial.
    #[test]
    fn single_holder_join_is_identity() {
        let a_doc = "ex:p1 ex:knows ex:x1 . ex:p2 ex:knows ex:x2 .";
        let a = Holder::from_rdf("a", &format!("{PFX}{a_doc}"), "turtle").unwrap();
        let pa = a
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }")
            .unwrap();
        let plan = JoinPlan {
            join_var: var("x"),
            key_disclosed: true,
        };
        let joined = DisclosedKeyJoin
            .join(std::slice::from_ref(&pa), &plan)
            .unwrap();
        assert_eq!(joined.rows.len(), pa.rows.len());
        assert_eq!(
            canonical_multiset(&joined.vars, &joined.rows),
            canonical_multiset(&pa.vars, &pa.rows),
        );
    }

    /// The crypto-free [`DisclosedKeyJoin`] still routes the hidden-value regime
    /// AWAY (it is the M2 disclosed-key path). The hidden-value capability now
    /// lives in the dedicated [`HiddenValueJoin`] (M3, tested below); asking the
    /// disclosed-key join to handle private keys must still be an honest,
    /// gate-naming `NotYetImplemented` — it does NOT fake a private join.
    #[test]
    fn disclosed_key_join_still_routes_hidden_path_away() {
        let a = Holder::from_rdf("a", &format!("{PFX}ex:p1 ex:knows ex:x1 ."), "turtle").unwrap();
        let pa = a
            .evaluate_local("PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows ?x }")
            .unwrap();
        // key_disclosed == false → the private-value regime.
        let plan = JoinPlan {
            join_var: var("x"),
            key_disclosed: false,
        };
        let err = DisclosedKeyJoin.join(&[pa], &plan).unwrap_err();
        match err {
            MpcError::NotYetImplemented { gated_on, what } => {
                assert!(what.contains("hidden-value") || what.contains("PSI"));
                assert!(gated_on.contains("M3"), "must cite the backend gate M3");
                assert!(gated_on.contains("Q2") && gated_on.contains("Q3"));
            }
            other => panic!("expected NotYetImplemented, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod hidden_value_tests {
    //! M3 tests for the HIDDEN-VALUE join (`HiddenValueJoin`): the path M2 could
    //! not provide. The load-bearing ones are `differential_*`: the secure join
    //! over PRIVATE keys must yield the SAME disclosed-payload multiset as the
    //! plaintext inner join over the union of the two holders' rows. The secure
    //! computation (secret-shared equality, keys never reconstructed) is run
    //! in-process across the simulated Shamir parties.
    use super::*;
    use crate::shamir::ShamirBackend;

    fn var(n: &str) -> Variable {
        Variable::new_unchecked(n)
    }

    /// A plaintext literal payload term, for building expected results / rows.
    fn lit(s: &str) -> Option<Term> {
        Some(Term::Literal(oxrdf::Literal::new_simple_literal(s)))
    }

    /// Build a holder's hidden-keyed rows. The key is encoded as a field element
    /// via an INJECTIVE small-integer map over the join's key domain (so the
    /// differential is sound: field equality ⇔ key equality, no collisions). In
    /// production this encoding is a collision-resistant hash proven in-circuit;
    /// here we control it to make the test exact (see `HiddenValueJoin` doc).
    fn keyed(
        holder: &str,
        payload_vars: &[&str],
        rows: &[(u64, Vec<Option<Term>>)],
    ) -> BatchedHiddenInput {
        // A common public group admits every pair without disclosing the
        // private equality key; the caller still supplies the candidate bound.
        BatchedHiddenInput::new(
            HolderId::new(holder),
            payload_vars.iter().map(|v| var(v)).collect(),
            rows.iter().map(|(k, p)| (Fp::new(*k), p.clone())).collect(),
            RowBinding::Keyed(vec!["candidate-group".to_string(); rows.len()]),
        )
        .unwrap()
    }

    /// The reference PLAINTEXT inner join over the union: combine `(lpay ++ rpay)`
    /// for every pair whose integer keys are equal. This is what the secure join
    /// must match as a multiset.
    fn plaintext_join(
        left: &[(u64, Vec<Option<Term>>)],
        right: &[(u64, Vec<Option<Term>>)],
    ) -> Vec<Vec<Option<Term>>> {
        let mut out = Vec::new();
        for (lk, lp) in left {
            for (rk, rp) in right {
                if lk == rk {
                    let mut row = lp.clone();
                    row.extend(rp.iter().cloned());
                    out.push(row);
                }
            }
        }
        out
    }

    fn multiset(rows: &[Vec<Option<Term>>]) -> Vec<String> {
        let mut m: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
        m.sort();
        m
    }

    /// THE M3 differential: a secure hidden-key join equals the plaintext inner
    /// join over the union — keys are PRIVATE and never reconstructed. Holder L
    /// has `(key, name)`, Holder R has `(key, city)`; the join discloses
    /// `(name, city)` for matching keys WITHOUT revealing the key.
    #[test]
    fn differential_hidden_join_equals_plaintext_join() {
        // Keys 100, 200, 300 are private identifiers (e.g. a hashed national-ID).
        let l_rows = vec![
            (100u64, vec![lit("Alice")]),
            (200u64, vec![lit("Bob")]),
            (300u64, vec![lit("Carol")]),
        ];
        // R shares keys 200 and 300 (Alice's 100 has no match on the right).
        let r_rows = vec![
            (200u64, vec![lit("Leeds")]),
            (300u64, vec![lit("York")]),
            (999u64, vec![lit("Hull")]), // no left match
        ];
        let left = keyed("L", &["name"], &l_rows);
        let right = keyed("R", &["city"], &r_rows);

        // n=3, t=1 → supports the single equality multiplication (2t+1 = 3 = n).
        // Seeded test backend for reproducibility; production uses the CSPRNG.
        let backend = ShamirBackend::new_seeded(3, 0xBEEF).unwrap();
        let join = HiddenValueJoin::new(backend);
        let (got, _) = join.batched_join(&left, &right, 9).unwrap();

        // Disclosed schema is (name, city) — the key is NOT projected.
        assert_eq!(got.vars, vec![var("name"), var("city")]);
        assert_eq!(got.holder, HolderId::new("federation"));

        let expected = plaintext_join(&l_rows, &r_rows);
        assert_eq!(
            multiset(&got.rows),
            multiset(&expected),
            "hidden join must equal plaintext join"
        );
        // Concretely: Bob-Leeds, Carol-York. 2 rows.
        assert_eq!(got.rows.len(), 2);
    }

    /// No-overlap: disjoint private key sets → empty join, matching plaintext.
    #[test]
    fn differential_no_overlap_is_empty() {
        let l_rows = vec![(1u64, vec![lit("a")]), (2, vec![lit("b")])];
        let r_rows = vec![(3u64, vec![lit("x")]), (4, vec![lit("y")])];
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 7).unwrap());
        let got = join
            .batched_join(
                &keyed("L", &["l"], &l_rows),
                &keyed("R", &["r"], &r_rows),
                4,
            )
            .unwrap()
            .0;
        assert!(got.rows.is_empty());
        assert_eq!(
            multiset(&got.rows),
            multiset(&plaintext_join(&l_rows, &r_rows))
        );
    }

    /// Multi-match fan-out: a private key shared by several rows on each side
    /// produces the full cartesian product within that key — must equal plaintext.
    #[test]
    fn differential_multi_match_fanout() {
        // Key 42 appears twice on the left and twice on the right → 4 joined rows.
        let l_rows = vec![
            (42u64, vec![lit("L1")]),
            (42, vec![lit("L2")]),
            (7, vec![lit("Lonely")]),
        ];
        let r_rows = vec![(42u64, vec![lit("R1")]), (42, vec![lit("R2")])];
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 123).unwrap());
        let got = join
            .batched_join(
                &keyed("L", &["l"], &l_rows),
                &keyed("R", &["r"], &r_rows),
                6,
            )
            .unwrap()
            .0;
        assert_eq!(got.rows.len(), 4, "2x2 cartesian on key 42");
        assert_eq!(
            multiset(&got.rows),
            multiset(&plaintext_join(&l_rows, &r_rows))
        );
    }

    /// Empty side → empty join (an empty holder contributes nothing).
    #[test]
    fn differential_empty_side_is_empty() {
        let l_rows = vec![(1u64, vec![lit("a")])];
        let r_rows: Vec<(u64, Vec<Option<Term>>)> = vec![];
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 1).unwrap());
        let got = join
            .batched_join(
                &keyed("L", &["l"], &l_rows),
                &keyed("R", &["r"], &r_rows),
                0,
            )
            .unwrap()
            .0;
        assert!(got.rows.is_empty());
    }

    /// The secure equality primitive in isolation: equal keys → match, unequal
    /// keys → no match, WITHOUT reconstructing the keys. This is the core
    /// circuit-PSI primitive. Larger n (n=5,t=2) exercises 2t+1=5 reconstruction.
    #[test]
    fn secure_equal_primitive_is_correct() {
        let backend = ShamirBackend::new_seeded(5, 0xD00D).unwrap();
        let join = HiddenValueJoin::new(backend);
        let mut dealer = join.backend.dealer();
        assert!(join
            .secure_equal(&mut dealer, Fp::new(12345), Fp::new(12345))
            .unwrap());
        assert!(!join
            .secure_equal(&mut dealer, Fp::new(12345), Fp::new(12346))
            .unwrap());
        // Zero is a valid key value and equality with zero still works.
        assert!(join
            .secure_equal(&mut dealer, Fp::zero(), Fp::zero())
            .unwrap());
        assert!(!join
            .secure_equal(&mut dealer, Fp::zero(), Fp::new(1))
            .unwrap());
    }

    /// The honest-majority constructor never under-provisions for the single
    /// equality multiplication (it picks `t = (n-1)/2`, so `2t+1 <= n`); assert
    /// the happy path holds so the guard never fires a false error.
    #[test]
    fn secure_equal_does_not_falsely_guard_party_count() {
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 0).unwrap());
        let mut dealer = join.backend.dealer();
        assert!(join
            .secure_equal(&mut dealer, Fp::new(5), Fp::new(5))
            .is_ok());
    }

    /// STRESS / adversarial: the secure equality test must have ZERO false
    /// matches AND zero false non-matches across many random key pairs and
    /// several honest-majority party counts. This guards against a masking bug
    /// (e.g. a zero mask slipping through → false match) or a reconstruction-
    /// degree error. Deterministic LCG so the test is reproducible.
    #[test]
    fn secure_equal_no_false_results_across_random_pairs() {
        let mut state: u64 = 0x1234_5678;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % 50 // small range → keys collide often, exercising both arms
        };
        for n in [3usize, 5, 7] {
            for trial in 0..300u64 {
                let a = next();
                let b = next();
                let join = HiddenValueJoin::new(
                    ShamirBackend::new_seeded(n, trial.wrapping_mul(31).wrapping_add(1)).unwrap(),
                );
                let mut dealer = join.backend.dealer();
                let got = join
                    .secure_equal(&mut dealer, Fp::new(a), Fp::new(b))
                    .unwrap();
                assert_eq!(
                    got,
                    a == b,
                    "n={n} a={a} b={b}: secure_equal disagreed with plaintext"
                );
            }
        }
    }

    // ===================== sq-4vgx: PIN the L2 match-graph leak =====================
    //
    // The spike's deliverable: pin, with a test, exactly what the CHEAP-but-LEAKY
    // scalar per-pair `secure_equal` calls disclose,
    // BEFORE any fix — so the privacy boundary between the leaky tier and the
    // oblivious tier (`fully_oblivious_batched_join`, sq-xhaw) is a regression-
    // guarded property, not just prose. These tests assert the leak EXISTS in the
    // scalar path (it is the documented cheap tier) and is ABSENT (per-pair) in the
    // fully-oblivious path. If a future refactor silently routed the leaky path as
    // though it were oblivious, the first test would still pass (the leak is real)
    // but `fully_oblivious_*` already pins the no-open property — together they
    // fence the boundary.

    /// PIN (sq-4vgx): the scalar all-pairs `secure_equal` driver learns the EXACT
    /// bipartite match graph. We play the role of the party driving the opens:
    /// call `secure_equal` on every `(i, j)` and collect each opened verdict. The
    /// recovered incidence matrix must equal the TRUE bipartite match graph — i.e.
    /// the open path leaks not just the match COUNT but precisely which left row
    /// matched which right row (leak L2). The hidden KEY values are still never
    /// recovered (only the per-pair equality bit), which we also assert.
    #[test]
    fn secure_equal_leaks_full_bipartite_match_graph() {
        // Two left rows and three right rows over a small private-key domain. The
        // keys themselves are secret; the match GRAPH is what leaks.
        let lkeys = [7u64, 9];
        let rkeys = [7u64, 9, 7]; // r0,r2 share L's 7; r1 shares L's 9
        let true_graph: Vec<Vec<bool>> = lkeys
            .iter()
            .map(|&lk| rkeys.iter().map(|&rk| lk == rk).collect())
            .collect();

        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(5, 0x4F60).unwrap());
        let mut dealer = join.backend.dealer();

        // Call the scalar primitive per pair and record each opened verdict.
        let observed: Vec<Vec<bool>> = lkeys
            .iter()
            .map(|&lk| {
                rkeys
                    .iter()
                    .map(|&rk| {
                        join.secure_equal(&mut dealer, Fp::new(lk), Fp::new(rk))
                            .unwrap()
                    })
                    .collect()
            })
            .collect();

        // THE LEAK: the driver reconstructs the full bipartite match graph, not
        // merely the match count. (This is L2 — the cheap-tier leak the oblivious
        // path closes.)
        assert_eq!(
            observed, true_graph,
            "scalar secure_equal driver must reveal the exact per-pair match graph (L2 leak pinned)"
        );

        // The match graph fingerprints the key fan-out: L's key 7 has fan-out 2 on
        // the right, key 9 fan-out 1 — directly readable from the observed rows.
        let fanout: Vec<usize> = observed
            .iter()
            .map(|row| row.iter().filter(|&&b| b).count())
            .collect();
        assert_eq!(
            fanout,
            vec![2, 1],
            "per-key fan-out is readable from the leak"
        );
    }

    /// PIN (sq-4vgx, companion): two key configurations that yield the SAME output
    /// multiset SIZE (and the same total match count) but DIFFERENT bipartite match
    /// graphs are DISTINGUISHABLE through the scalar open path — which is precisely
    /// the privacy loss the oblivious tier removes. Config A: a 2×2 block on one key
    /// (fan-out [2,2] across two left rows → 4 matches won't help; use disjoint).
    /// We pick two graphs with the SAME number of true cells but different shapes.
    #[test]
    fn scalar_match_graph_distinguishes_equal_size_different_shape() {
        // Both configs: 2 left rows × 2 right rows, with exactly 2 matching pairs —
        // identical match COUNT, so the count alone cannot tell them apart.
        // Config A — diagonal: (l0,r0) and (l1,r1) match.
        let a_l = [1u64, 2];
        let a_r = [1u64, 2];
        // Config B — one left row matches BOTH right rows (key fan-out 2), the other
        // matches neither: (l0,r0) and (l0,r1) match. Same 2 matches, different graph.
        let b_l = [5u64, 8];
        let b_r = [5u64, 5];

        let graph_of = |lk: &[u64], rk: &[u64]| -> Vec<Vec<bool>> {
            let join = HiddenValueJoin::new(ShamirBackend::new_seeded(5, 0x4F61).unwrap());
            let mut dealer = join.backend.dealer();
            lk.iter()
                .map(|&l| {
                    rk.iter()
                        .map(|&r| {
                            join.secure_equal(&mut dealer, Fp::new(l), Fp::new(r))
                                .unwrap()
                        })
                        .collect()
                })
                .collect()
        };

        let ga = graph_of(&a_l, &a_r);
        let gb = graph_of(&b_l, &b_r);

        let count = |g: &Vec<Vec<bool>>| g.iter().flatten().filter(|&&b| b).count();
        assert_eq!(
            count(&ga),
            count(&gb),
            "both configs have the same match count"
        );
        // Yet the observed match GRAPHS differ — the scalar path leaks the SHAPE,
        // not just the size. This is exactly the L2 information the oblivious
        // output + secret-shared-bit decision (sq-jnkm / sq-xhaw) hide.
        assert_ne!(
            ga, gb,
            "equal-count configs must be distinguishable via the scalar match-graph leak (L2)"
        );
    }
}

#[cfg(test)]
mod batched_hidden_value_tests {
    //! sq-khf9 — the BATCHED hidden-value join over a COLUMN of rows per holder,
    //! wiring the sq-dwb5 `BatchedShares`/`RowBinding` primitive + the sq-jnkm
    //! oblivious output transform into `HiddenValueJoin::batched_join`. The
    //! load-bearing tests:
    //! - `differential_*`: the batched join's REAL (dummy-filtered) output multiset
    //!   equals the plaintext join over the union of the two holders' row columns,
    //!   under each row-binding regime — keys PRIVATE, never reconstructed.
    //! - row-binding correctness (Positional + Keyed) across n ∈ {3,5,7}.
    //! - privacy: the join VALUE is never opened (only the oblivious output is) and
    //!   the revealed output cardinality is the public bound B, not the true count.
    //! - a forged-match soundness case (a fabricated key cannot induce a match,
    //!   consistent with the existing secure-equal soundness).
    use super::*;
    use crate::oblivious_join::OutputSlot;
    use crate::shamir::ShamirBackend;

    fn var(n: &str) -> Variable {
        Variable::new_unchecked(n)
    }

    fn lit(s: &str) -> Option<Term> {
        Some(Term::Literal(oxrdf::Literal::new_simple_literal(s)))
    }

    /// Every candidate-materialising entry refuses an over-cap `B` with ZERO draws,
    /// on a NON-EMPTY input — the point is that the refusal precedes the work, not
    /// that an empty input does nothing. One equality sharing per candidate would
    /// otherwise be drawn before the output stage's own check ran.
    ///
    /// Also pinned: an over-cap CANDIDATE count (not just an over-cap `B`) is
    /// refused the same way, and so is a `Keyed` fan-out that overflows. The three
    /// entries are `batched_join_through`, `fully_oblivious_batched_join_through`
    /// and the test-only `fully_oblivious_candidates_and_opened_rows`.
    /// The `B` cap runs before `candidate_pair_count` is CALLED, not just before
    /// the draws — and the only way to observe "before the call" from outside is
    /// the refusal's identity. So each case below is invalid in TWO ways at once:
    /// the bound is over the cap, and the bindings are something the counter itself
    /// would reject. A cap-first preflight names `MAX_OUTPUT_BOUND`; a
    /// count-first one names the binding instead. That distinction is what pins the
    /// ordering, and with it the fact that the counter's key-bucket map — the one
    /// allocation in the preflight, sized by the caller's rows — is never built for
    /// a run that is already refused.
    #[test]
    fn the_bound_cap_precedes_the_candidate_count_and_its_allocation() {
        use crate::oblivious_join::MAX_OUTPUT_BOUND;
        use crate::randomness::test_sources::DrawCountingSource;

        let backend = ShamirBackend::new_seeded(5, 0xB0C1).unwrap();
        let join = HiddenValueJoin::new(backend.clone());

        // (a) Positional columns of DIFFERENT length: the counter's own refusal.
        let short = positional("l", &["x"], &[(7, vec![lit("a")])]);
        let long = positional(
            "r",
            &["y"],
            &[
                (7, vec![lit("c")]),
                (9, vec![lit("d")]),
                (11, vec![lit("e")]),
            ],
        );
        // (b) MIXED bindings: also the counter's own refusal.
        let as_keyed = keyed("r", &["y"], &[("k", 7, vec![lit("c")])]);

        for (label, l, r) in [
            ("positional length mismatch", &short, &long),
            ("mixed bindings", &short, &as_keyed),
        ] {
            // The counter really does refuse these on its own, with its own words —
            // otherwise the discriminator below would prove nothing.
            let legal = join
                .batched_join_through(&mut DrawCountingSource::new(&mut backend.dealer()), l, r, 4)
                .expect_err("a legal bound still leaves the binding invalid");
            assert!(
                matches!(&legal, MpcError::Protocol(m) if !m.contains("MAX_OUTPUT_BOUND")),
                "{}: the binding refusal should not mention the cap: {:?}",
                label,
                legal
            );

            for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
                let mut dealer = backend.dealer();
                let mut counted = DrawCountingSource::new(&mut dealer);
                let err = join
                    .batched_join_through(&mut counted, l, r, bound)
                    .expect_err("an over-cap bound must be refused");
                assert!(
                    matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                    "{} B={}: the cap must refuse BEFORE the count, got {:?}",
                    label,
                    bound,
                    err
                );
                assert_eq!(
                    counted.draws(),
                    0,
                    "{} B={}: refused before any draw",
                    label,
                    bound
                );
            }
        }
    }

    #[test]
    fn every_batched_entry_refuses_an_over_cap_bound_before_any_draw() {
        use crate::oblivious_join::MAX_OUTPUT_BOUND;
        use crate::randomness::test_sources::DrawCountingSource;

        let backend = ShamirBackend::new_seeded(5, 0xB0FF).unwrap();
        let join = HiddenValueJoin::new(backend.clone());
        let left = positional("l", &["x"], &[(7, vec![lit("a")]), (8, vec![lit("b")])]);
        let right = positional("r", &["y"], &[(7, vec![lit("c")]), (9, vec![lit("d")])]);

        for bound in [usize::MAX, MAX_OUTPUT_BOUND + 1] {
            let mut dealer = backend.dealer();
            let mut counted = DrawCountingSource::new(&mut dealer);
            let err = join
                .batched_join_through(&mut counted, &left, &right, bound)
                .expect_err("batched_join_through must refuse an over-cap bound");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                "batched_join_through B={}: {:?}",
                bound,
                err
            );
            assert_eq!(counted.draws(), 0, "batched_join_through B={}", bound);

            let mut dealer = backend.dealer();
            let mut counted = DrawCountingSource::new(&mut dealer);
            let err = join
                .fully_oblivious_batched_join_through(&mut counted, &left, &right, bound)
                .expect_err("fully_oblivious_batched_join_through must refuse it too");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                "fully_oblivious B={}: {:?}",
                bound,
                err
            );
            assert_eq!(counted.draws(), 0, "fully_oblivious B={}", bound);

            let mut dealer = backend.dealer();
            let mut counted = DrawCountingSource::new(&mut dealer);
            let err = join
                .fully_oblivious_candidates_and_opened_rows(&mut counted, &left, &right, bound)
                .expect_err("the instrumented entry must refuse it too");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
                "candidates_and_opened_rows B={}: {:?}",
                bound,
                err
            );
            assert_eq!(counted.draws(), 0, "candidates_and_opened_rows B={}", bound);
        }

        // An over-cap CANDIDATE count, with a `B` that is itself within the cap: the
        // Keyed fan-out is counted without enumerating the pairs, so this refuses
        // before the pair vector exists.
        let many = (MAX_OUTPUT_BOUND / 8) + 1;
        let lrows: Vec<(&str, u64, Vec<Option<Term>>)> =
            (0..8).map(|_| ("k", 1u64, vec![lit("a")])).collect();
        let rrows: Vec<(&str, u64, Vec<Option<Term>>)> =
            (0..many).map(|_| ("k", 1u64, vec![lit("c")])).collect();
        let lk = keyed("l", &["x"], &lrows);
        let rk = keyed("r", &["y"], &rrows);
        let mut dealer = backend.dealer();
        let mut counted = DrawCountingSource::new(&mut dealer);
        let err = join
            .batched_join_through(&mut counted, &lk, &rk, MAX_OUTPUT_BOUND)
            .expect_err("an over-cap candidate count must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("MAX_OUTPUT_BOUND")),
            "over-cap candidate count: {:?}",
            err
        );
        assert_eq!(
            counted.draws(),
            0,
            "the candidate count must be refused before any draw"
        );

        // The same shapes at a legal bound still run, so the guard is not refusing
        // everything.
        assert!(join.batched_join(&left, &right, 4).is_ok());
    }

    /// The non-materialising counter must agree with the enumeration it stands in
    /// for, or the preflight would guard a different number than the builders use.
    #[test]
    fn candidate_pair_count_matches_candidate_pairs() {
        let pos = RowBinding::Positional;
        let k_l = RowBinding::Keyed(vec!["a".into(), "b".into(), "a".into()]);
        let k_r = RowBinding::Keyed(vec!["a".into(), "a".into(), "c".into(), "b".into()]);
        let cases: Vec<(&RowBinding, &RowBinding, usize, usize)> =
            vec![(&pos, &pos, 3, 3), (&k_l, &k_r, 3, 4), (&k_r, &k_l, 4, 3)];
        for (l, r, llen, rlen) in cases {
            let counted = candidate_pair_count(l, r, llen, rlen).unwrap();
            let enumerated = candidate_pairs(l, r, llen, rlen).unwrap().len();
            assert_eq!(
                counted, enumerated,
                "the counter and the enumeration disagree for {} x {} rows",
                llen, rlen
            );
        }
        // And the refusals agree too.
        assert!(candidate_pair_count(&pos, &pos, 2, 3).is_err());
        assert!(candidate_pair_count(&pos, &k_r, 1, 1).is_err());
    }

    /// Build a POSITIONAL batched input from `(key, payload)` rows.
    fn positional(
        holder: &str,
        payload_vars: &[&str],
        rows: &[(u64, Vec<Option<Term>>)],
    ) -> BatchedHiddenInput {
        BatchedHiddenInput::new(
            HolderId::new(holder),
            payload_vars.iter().map(|v| var(v)).collect(),
            rows.iter().map(|(k, p)| (Fp::new(*k), p.clone())).collect(),
            RowBinding::Positional,
        )
        .unwrap()
    }

    /// Build a KEYED batched input from `(disclosed_key, hidden_key, payload)` rows.
    /// The DISCLOSED key is the public row id (correlation), the HIDDEN key is the
    /// `Fp` value matched under secure-equal.
    fn keyed(
        holder: &str,
        payload_vars: &[&str],
        rows: &[(&str, u64, Vec<Option<Term>>)],
    ) -> BatchedHiddenInput {
        let disclosed: Vec<String> = rows.iter().map(|(k, _, _)| (*k).to_string()).collect();
        BatchedHiddenInput::new(
            HolderId::new(holder),
            payload_vars.iter().map(|v| var(v)).collect(),
            rows.iter()
                .map(|(_, h, p)| (Fp::new(*h), p.clone()))
                .collect(),
            RowBinding::Keyed(disclosed),
        )
        .unwrap()
    }

    /// The REAL (non-dummy) rows of a join output, as an order-independent multiset.
    fn real_multiset(result: &PartialResult) -> Vec<String> {
        let mut m: Vec<String> = result.rows.iter().map(|r| format!("{r:?}")).collect();
        m.sort();
        m
    }

    fn expect_multiset(rows: &[Vec<Option<Term>>]) -> Vec<String> {
        let mut m: Vec<String> = rows.iter().map(|r| format!("{r:?}")).collect();
        m.sort();
        m
    }

    // ============================ POSITIONAL regime ============================

    /// THE Positional differential across n ∈ {3,5,7}: two equal-length columns,
    /// row `i` of L paired with row `i` of R; the batched join's real output equals
    /// the index-aligned plaintext join — keys PRIVATE, never reconstructed.
    #[test]
    fn positional_differential_equals_plaintext_across_party_counts() {
        // Row i matches iff lkeys[i] == rkeys[i]. Rows 0 and 2 match (10, 30); row 1
        // does not (20 vs 99).
        let l = [
            (10u64, vec![lit("L0")]),
            (20, vec![lit("L1")]),
            (30, vec![lit("L2")]),
        ];
        let r = [
            (10u64, vec![lit("R0")]),
            (99, vec![lit("R1")]),
            (30, vec![lit("R2")]),
        ];
        // Plaintext index-aligned join.
        let expected: Vec<Vec<Option<Term>>> = (0..l.len())
            .filter(|&i| l[i].0 == r[i].0)
            .map(|i| {
                let mut row = l[i].1.clone();
                row.extend(r[i].1.iter().cloned());
                row
            })
            .collect();

        for n in [3usize, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0xB47C + n as u64).unwrap();
            let join = HiddenValueJoin::new(backend);
            let li = positional("L", &["lname"], &l);
            let ri = positional("R", &["rname"], &r);
            // Bound B = candidate count (3); the revealed slot count is B, not the
            // true 2 matches.
            let (result, cost) = join.batched_join(&li, &ri, 3).unwrap();
            assert_eq!(result.holder, HolderId::new("federation"));
            assert_eq!(result.vars, vec![var("lname"), var("rname")]);
            assert_eq!(cost.bound, 3, "n={n}: revealed slot bound is B");
            assert_eq!(
                real_multiset(&result),
                expect_multiset(&expected),
                "n={n}: batched positional join != plaintext index-aligned join"
            );
            assert_eq!(result.rows.len(), 2, "n={n}: exactly the 2 matching rows");
        }
    }

    /// Positional fails closed on mis-aligned (unequal-length) columns — never a
    /// silent wrong answer.
    #[test]
    fn positional_unequal_lengths_fails_closed() {
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 1).unwrap());
        let l = positional("L", &["a"], &[(1u64, vec![lit("x")]), (2, vec![lit("y")])]);
        let r = positional("R", &["b"], &[(1u64, vec![lit("p")])]);
        let err = join.batched_join(&l, &r, 4).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("equal length")));
    }

    // ============================== KEYED regime ==============================

    /// THE Keyed differential across n ∈ {3,5,7}: rows correlate by the DISCLOSED
    /// public key (orders need NOT match); within a shared disclosed key the HIDDEN
    /// value still gates the match via secure-equal. Real output == plaintext join.
    #[test]
    fn keyed_differential_equals_plaintext_across_party_counts() {
        // Disclosed keys correlate rows; hidden values gate the actual match.
        // ("r1", hidden 100) on both → match. ("r2", hidden 200 vs 999) → key
        // correlates but hidden values differ → NO match. ("r3") only on left.
        let l = [
            ("r1", 100u64, vec![lit("LA")]),
            ("r2", 200, vec![lit("LB")]),
            ("r3", 300, vec![lit("LC")]),
        ];
        // R in a DIFFERENT order to prove key-correlation (not index).
        let r = [
            ("r2", 999u64, vec![lit("RB")]),
            ("r1", 100, vec![lit("RA")]),
        ];
        // Plaintext keyed join: same disclosed key AND same hidden value.
        let mut expected: Vec<Vec<Option<Term>>> = Vec::new();
        for (lk, lh, lp) in &l {
            for (rk, rh, rp) in &r {
                if lk == rk && lh == rh {
                    let mut row = lp.clone();
                    row.extend(rp.iter().cloned());
                    expected.push(row);
                }
            }
        }

        for n in [3usize, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0x6EE7 + n as u64).unwrap();
            let join = HiddenValueJoin::new(backend);
            let li = keyed("L", &["lname"], &l);
            let ri = keyed("R", &["rname"], &r);
            // Candidate count = key-bucket matches: r1 (1×1) + r2 (1×1) = 2.
            let (result, cost) = join.batched_join(&li, &ri, 2).unwrap();
            assert_eq!(cost.bound, 2, "n={n}");
            assert_eq!(
                real_multiset(&result),
                expect_multiset(&expected),
                "n={n}: batched keyed join != plaintext keyed join"
            );
            // Only ("r1", 100) actually matches on the hidden value.
            assert_eq!(result.rows.len(), 1, "n={n}: only the r1/100 row matches");
            assert_eq!(result.rows[0], vec![lit("LA"), lit("RA")]);
        }
    }

    /// Keyed multi-row fan-out within a shared disclosed key: 2×2 hidden-value
    /// matches inside one key bucket → cartesian, equal to plaintext.
    #[test]
    fn keyed_fanout_within_a_key_bucket() {
        // All under disclosed key "k", hidden value 7 shared by 2 rows each side.
        let l = [
            ("k", 7u64, vec![lit("L1")]),
            ("k", 7, vec![lit("L2")]),
            ("k", 8, vec![lit("L3")]), // hidden 8 → no right match
        ];
        let r = [("k", 7u64, vec![lit("R1")]), ("k", 7, vec![lit("R2")])];
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(5, 0xFA00).unwrap());
        let li = keyed("L", &["l"], &l);
        let ri = keyed("R", &["r"], &r);
        // Candidate pairs in the "k" bucket: 3 left × 2 right = 6; B must cover them.
        let (result, _) = join.batched_join(&li, &ri, 6).unwrap();
        // Hidden-value matches: the two 7-rows on each side → 2×2 = 4.
        assert_eq!(result.rows.len(), 4, "2×2 cartesian on hidden value 7");
    }

    /// Disjoint disclosed keys → zero candidates → empty real output.
    #[test]
    fn keyed_disjoint_keys_is_empty() {
        let l = [("a", 1u64, vec![lit("x")])];
        let r = [("b", 1u64, vec![lit("y")])];
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 2).unwrap());
        let (result, _) = join
            .batched_join(&keyed("L", &["l"], &l), &keyed("R", &["r"], &r), 0)
            .unwrap();
        assert!(result.rows.is_empty());
    }

    /// Mixed bindings (one Positional, one Keyed) is a protocol error — the
    /// correlation rule would be ambiguous.
    #[test]
    fn mixed_bindings_is_a_protocol_error() {
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 1).unwrap());
        let l = positional("L", &["a"], &[(1u64, vec![lit("x")])]);
        let r = keyed("R", &["b"], &[("k", 1u64, vec![lit("y")])]);
        let err = join.batched_join(&l, &r, 1).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("mixed row-bindings")));
    }

    // ============================== Privacy ==============================

    /// PRIVACY (L1 at the OUTPUT — substantive, not the bound echo): two scenarios
    /// with DIFFERENT true match counts (1 vs 3) but the SAME candidate count + bound
    /// reveal the IDENTICAL number of output SLOTS — exactly `B`, ALL of which are
    /// observed as a flat slot vector whose LENGTH is `B` regardless of the true
    /// count. We go through the lower-level `oblivious_set_output` (which exposes the
    /// SLOTS that `batched_join` filters) so the obliviousness is asserted on the
    /// actual revealed structure, not just the `cost.bound` the input echoes. We also
    /// assert the hidden join KEY never appears in any revealed slot.
    #[test]
    fn output_cardinality_is_bounded_to_b_not_true_count() {
        use crate::oblivious_join::oblivious_set_output;

        // A SECRET join key the parties must never see in the output, used in the
        // matching rows of both scenarios.
        let secret_key = 0x5EC5_E700u64;
        // Build the candidates a batched positional join would produce, deciding each
        // match with the SAME secure-equal core, for `n_matches` of 3 rows.
        let candidates_for = |n_matches: usize, backend: &ShamirBackend| -> Vec<Candidate> {
            let join = HiddenValueJoin::new(backend.clone());
            let mut dealer = backend.dealer();
            // Left column: 3 rows, all keyed on the secret key.
            let left = [secret_key, secret_key, secret_key];
            // Right column: the first `n_matches` rows share the secret key (a true
            // match); the rest carry distinct non-matching keys.
            let right: Vec<u64> = (0..3)
                .map(|i| {
                    if i < n_matches {
                        secret_key
                    } else {
                        0xD15 + i as u64
                    }
                })
                .collect();
            (0..3)
                .map(|i| {
                    let matched = join
                        .secure_equal(&mut dealer, Fp::new(left[i]), Fp::new(right[i]))
                        .unwrap();
                    Candidate {
                        // Payload discloses only the row index, NEVER the key.
                        payload: vec![lit(&format!("L{i}")), lit(&format!("R{i}"))],
                        matched: MatchBit::Public(matched),
                    }
                })
                .collect()
        };

        let b = 5usize; // public bound padded above the 3 candidates
        let backend = ShamirBackend::new_seeded(3, 0x5217).unwrap();
        let (slots1, c1) =
            oblivious_set_output(&backend, &candidates_for(1, &backend), 2, b).unwrap();
        let (slots3, c3) =
            oblivious_set_output(&backend, &candidates_for(3, &backend), 2, b).unwrap();

        // (a) The REVEALED SLOT VECTOR has length exactly B in BOTH cases — the
        // parties observe the same output cardinality whether 1 or 3 rows truly
        // matched. This is the substantive obliviousness, not the bound echo.
        assert_eq!(slots1.len(), b, "1-match: parties see exactly B slots");
        assert_eq!(slots3.len(), b, "3-match: parties see exactly B slots");
        assert_eq!(
            slots1.len(),
            slots3.len(),
            "revealed slot count is independent of the true match count"
        );
        // The modelled cost (slot count / selects) is likewise B-driven, not count-driven.
        assert_eq!(c1.bound, b);
        assert_eq!((c1.bound, c1.select_mults), (c3.bound, c3.select_mults));

        // (b) The hidden join KEY never appears in ANY revealed slot of either run —
        // only the disclosed payload is ever materialised.
        for slots in [&slots1, &slots3] {
            let rendered = format!("{slots:?}");
            assert!(
                !rendered.contains(&secret_key.to_string()),
                "the hidden join key {secret_key} must NOT appear in any revealed slot: {rendered}"
            );
        }
    }

    /// PRIVACY (key never opened): the batched join discloses ONLY the payload
    /// columns; the hidden key column is never projected into the output schema,
    /// and the output rows contain only the payload terms — never the key value.
    #[test]
    fn key_value_is_never_in_the_output() {
        let secret_key = 0xDEAD_BEEFu64;
        let l = positional("L", &["lname"], &[(secret_key, vec![lit("Alice")])]);
        let r = positional("R", &["rname"], &[(secret_key, vec![lit("Leeds")])]);
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 0xABCD).unwrap());
        let (result, _) = join.batched_join(&l, &r, 1).unwrap();
        // Schema is payload-only — the key var is absent.
        assert_eq!(result.vars, vec![var("lname"), var("rname")]);
        assert_eq!(result.rows.len(), 1);
        // No output term encodes the secret key (it is hidden; only payload shows).
        let rendered = format!("{:?}", result.rows);
        assert!(
            !rendered.contains(&secret_key.to_string()),
            "the hidden key {secret_key} must NOT appear in the disclosed output: {rendered}"
        );
        assert_eq!(result.rows[0], vec![lit("Alice"), lit("Leeds")]);
    }

    /// PRIVACY (output order is shuffled): across several seeds the Row/Dummy
    /// classification of the OUTPUT SLOTS varies — the oblivious shuffle destroyed
    /// the position→candidate linkage. We observe via the lower-level set output so
    /// we can see slots (batched_join filters dummies). This mirrors the
    /// `oblivious_join` L2 test, exercised through the batched candidate path.
    #[test]
    fn output_order_is_oblivious_across_seeds() {
        use crate::oblivious_join::oblivious_set_output;
        // Build candidates exactly as batched_join would for a positional column
        // with matches at fixed input positions 0,2,4.
        let l = [
            (1u64, vec![lit("p0")]),
            (2, vec![lit("p1")]),
            (3, vec![lit("p2")]),
            (4, vec![lit("p3")]),
            (5, vec![lit("p4")]),
        ];
        let r = [
            (1u64, vec![lit("q0")]),
            (99, vec![lit("q1")]),
            (3, vec![lit("q2")]),
            (98, vec![lit("q3")]),
            (5, vec![lit("q4")]),
        ];
        let mut patterns = std::collections::HashSet::new();
        for seed in 0..32u64 {
            let backend = ShamirBackend::new_seeded(3, 2000 + seed).unwrap();
            let join = HiddenValueJoin::new(backend.clone());
            let mut dealer = backend.dealer();
            // Reproduce candidate decisions (positional, index-aligned).
            let mut cands = Vec::new();
            for i in 0..l.len() {
                let matched = join
                    .secure_equal(&mut dealer, Fp::new(l[i].0), Fp::new(r[i].0))
                    .unwrap();
                let mut payload = l[i].1.clone();
                payload.extend(r[i].1.iter().cloned());
                cands.push(Candidate {
                    payload,
                    matched: MatchBit::Public(matched),
                });
            }
            let (slots, _) = oblivious_set_output(&backend, &cands, 2, 5).unwrap();
            let pat: Vec<bool> = slots
                .iter()
                .map(|s| matches!(s, OutputSlot::Row(_)))
                .collect();
            patterns.insert(pat);
            assert_eq!(
                slots
                    .iter()
                    .filter(|s| matches!(s, OutputSlot::Row(_)))
                    .count(),
                3
            );
        }
        assert!(
            patterns.len() > 1,
            "output match positions never moved — not oblivious"
        );
    }

    // ============================== Soundness ==============================

    /// SOUNDNESS / negative: a forged key cannot manufacture a match. A left row
    /// whose key differs from every right key yields NO match through secure-equal
    /// (the masked product opens to a uniform nonzero, never 0), so it is filtered
    /// to a dummy and never appears in the disclosed output — consistent with the
    /// existing secure-equal soundness. Run across n ∈ {3,5,7}.
    #[test]
    fn forged_match_attempt_fails() {
        for n in [3usize, 5, 7] {
            let join =
                HiddenValueJoin::new(ShamirBackend::new_seeded(n, 0xF0F0 + n as u64).unwrap());
            // Control: both sides claim key 42 — a GENUINELY equal key DOES match.
            let l = positional("L", &["a"], &[(42u64, vec![lit("forged")])]);
            let r = positional("R", &["b"], &[(42u64, vec![lit("real")])]);
            let (ok, _) = join.batched_join(&l, &r, 1).unwrap();
            assert_eq!(
                ok.rows.len(),
                1,
                "n={n}: a genuine equal key matches (control)"
            );

            // Now the forge: distinct keys must NOT match.
            let l_bad = positional("L", &["a"], &[(42u64, vec![lit("forged")])]);
            let r_bad = positional("R", &["b"], &[(43u64, vec![lit("real")])]);
            let (bad, _) = join.batched_join(&l_bad, &r_bad, 1).unwrap();
            assert!(
                bad.rows.is_empty(),
                "n={n}: distinct keys must not produce a match (no forged match)"
            );
        }
    }

    /// Bound below the candidate count fails closed (the oblivious transform never
    /// truncates a candidate, which could drop a true match).
    #[test]
    fn bound_below_candidate_count_fails_closed() {
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 1).unwrap());
        // 3 positional candidates but B = 1.
        let l = positional(
            "L",
            &["a"],
            &[
                (1u64, vec![lit("x")]),
                (2, vec![lit("y")]),
                (3, vec![lit("z")]),
            ],
        );
        let r = positional(
            "R",
            &["b"],
            &[
                (1u64, vec![lit("p")]),
                (2, vec![lit("q")]),
                (3, vec![lit("r")]),
            ],
        );
        let err = join.batched_join(&l, &r, 1).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("truncate")));
    }

    /// A keyed binding with a mismatched disclosed-key count is rejected at
    /// construction (ambiguous binding), mirroring `BatchedShares::new`.
    #[test]
    fn keyed_input_key_count_must_match_rows() {
        let err = BatchedHiddenInput::new(
            HolderId::new("L"),
            vec![var("a")],
            vec![(Fp::new(1), vec![lit("x")]), (Fp::new(2), vec![lit("y")])],
            RowBinding::Keyed(vec!["only-one".into()]),
        )
        .unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("keyed binding")));
    }

    /// The sq-dwb5 batched primitive is actually exercised: `shared_keys` produces a
    /// `BatchedShares` whose length matches the row count and whose binding is
    /// preserved (the documented row-binding travels with the shares).
    #[test]
    fn shared_keys_uses_the_batched_primitive() {
        let backend = ShamirBackend::new_seeded(5, 0x5151).unwrap();
        let input = positional(
            "L",
            &["a"],
            &[
                (11u64, vec![lit("x")]),
                (22, vec![lit("y")]),
                (33, vec![lit("z")]),
            ],
        );
        let mut dealer = backend.dealer();
        let shared = input.shared_keys(&mut dealer).unwrap();
        assert_eq!(shared.len(), 3, "one batched sharing per row");
        assert_eq!(*shared.binding(), RowBinding::Positional);
    }

    // =========== sq-xhaw: FULLY-OBLIVIOUS batched hidden join ==========
    //
    // The match bit per candidate is SECRET-SHARED and NEVER opened per pair (vs
    // `batched_join`, which opens `secure_equal`'s masked product per pair — the L2
    // match-graph leak). The load-bearing test is the RESULT differential: the
    // revealed output equals the plaintext reference join, while the only per-pair
    // primitive used is `secure_equal_to_bit` (no open). The L1/L2 tests pin the
    // obliviousness the secret-shared bit makes possible.

    /// Plaintext index-aligned (Positional) reference join.
    fn plaintext_positional(
        l: &[(u64, Vec<Option<Term>>)],
        r: &[(u64, Vec<Option<Term>>)],
    ) -> Vec<Vec<Option<Term>>> {
        (0..l.len().min(r.len()))
            .filter(|&i| l[i].0 == r[i].0)
            .map(|i| {
                let mut row = l[i].1.clone();
                row.extend(r[i].1.iter().cloned());
                row
            })
            .collect()
    }

    /// THE sq-xhaw differential (Positional, n ∈ {3,5,7}): the fully-oblivious
    /// join's RESULT equals the plaintext index-aligned join — and the per-pair
    /// match bit is SECRET-SHARED, never opened (the join path uses only
    /// `secure_equal_to_bit`, whose verdict stays shared; nothing per-pair is
    /// reconstructed). The revealed slot count is the public bound `B`, NOT the true
    /// match count.
    #[test]
    fn fully_oblivious_positional_differential_equals_plaintext() {
        let l = [
            (10u64, vec![lit("L0")]),
            (20, vec![lit("L1")]),
            (30, vec![lit("L2")]),
        ];
        let r = [
            (10u64, vec![lit("R0")]),
            (99, vec![lit("R1")]),
            (30, vec![lit("R2")]),
        ];
        let expected = plaintext_positional(&l, &r);
        for n in [3usize, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0x0BBE + n as u64).unwrap();
            let join = HiddenValueJoin::new(backend);
            let li = positional("L", &["lname"], &l);
            let ri = positional("R", &["rname"], &r);
            let (result, cost) = join.fully_oblivious_batched_join(&li, &ri, 3).unwrap();
            assert_eq!(result.holder, HolderId::new("federation"));
            assert_eq!(result.vars, vec![var("lname"), var("rname")]);
            assert_eq!(cost.bound, 3, "n={n}: revealed slot bound is B");
            assert_eq!(
                real_multiset(&result),
                expect_multiset(&expected),
                "n={n}: fully-oblivious positional join != plaintext"
            );
            assert_eq!(result.rows.len(), 2, "n={n}: exactly the 2 matching rows");
        }
    }

    /// The fully-oblivious join agrees with the leaky-per-pair `batched_join` on the
    /// RESULT multiset (same correctness), differing only in WHAT leaks: the
    /// fully-oblivious path never opens a per-pair match bit. Cross-check across
    /// several seeds so the (independent) shuffles cannot mask a discrepancy.
    #[test]
    fn fully_oblivious_matches_leaky_batched_result() {
        let l = [
            (1u64, vec![lit("a")]),
            (2, vec![lit("b")]),
            (3, vec![lit("c")]),
            (4, vec![lit("d")]),
        ];
        let r = [
            (1u64, vec![lit("p")]),
            (9, vec![lit("q")]),
            (3, vec![lit("r")]),
            (8, vec![lit("s")]),
        ];
        for seed in 0..8u64 {
            let join = HiddenValueJoin::new(ShamirBackend::new_seeded(5, 200 + seed).unwrap());
            let li = positional("L", &["x"], &l);
            let ri = positional("R", &["y"], &r);
            let (oblivious, _) = join.fully_oblivious_batched_join(&li, &ri, 4).unwrap();
            let (leaky, _) = join.batched_join(&li, &ri, 4).unwrap();
            assert_eq!(
                real_multiset(&oblivious),
                real_multiset(&leaky),
                "seed {seed}: fully-oblivious result differs from leaky batched_join"
            );
        }
    }

    /// THE Keyed differential (n ∈ {3,5,7}): rows correlate by the DISCLOSED public
    /// key; within a shared disclosed key the HIDDEN value gates the match via the
    /// secret-shared equality bit. Result equals the plaintext join.
    #[test]
    fn fully_oblivious_keyed_differential_equals_plaintext() {
        // ("r1", hidden 100) on both → match. ("r2", 200 vs 999) → no match.
        // ("r3", 300) on both → match. Orders deliberately differ across sides.
        let l = [
            ("r1", 100u64, vec![lit("La")]),
            ("r2", 200, vec![lit("Lb")]),
            ("r3", 300, vec![lit("Lc")]),
        ];
        let r = [
            ("r3", 300u64, vec![lit("Rc")]),
            ("r1", 100, vec![lit("Ra")]),
            ("r2", 999, vec![lit("Rb")]),
        ];
        // Plaintext: candidate pairs are equal-disclosed-key rows; match iff hidden
        // values also equal. r1: 100==100 ✓ (La,Ra). r3: 300==300 ✓ (Lc,Rc). r2:
        // 200!=999 ✗.
        let expected: Vec<Vec<Option<Term>>> =
            vec![vec![lit("La"), lit("Ra")], vec![lit("Lc"), lit("Rc")]];
        for n in [3usize, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0xC0DE + n as u64).unwrap();
            let join = HiddenValueJoin::new(backend);
            let li = keyed("L", &["ln"], &l);
            let ri = keyed("R", &["rn"], &r);
            // 3 disclosed-key candidate pairs (r1, r2, r3 each correlate one-to-one);
            // r2's hidden values differ so it is a candidate that does NOT match. B=3.
            let (result, cost) = join.fully_oblivious_batched_join(&li, &ri, 3).unwrap();
            assert_eq!(cost.bound, 3, "n={n}");
            assert_eq!(
                real_multiset(&result),
                expect_multiset(&expected),
                "n={n}: fully-oblivious keyed join != plaintext"
            );
        }
    }

    /// L1 obliviousness: two candidate sets with DIFFERENT true match counts produce
    /// the IDENTICAL revealed slot count (= B). This is only possible BECAUSE the
    /// per-pair match bit is never opened — if it were, the parties would learn the
    /// true count from the per-pair verdicts. (The recipient who filters dummies
    /// learns the true result size; the protocol transcript sees only B.)
    #[test]
    fn fully_oblivious_revealed_count_is_bound_not_true_cardinality() {
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 0xA11).unwrap());
        // one match (only row 0).
        let l1 = positional(
            "L",
            &["x"],
            &[
                (1u64, vec![lit("a")]),
                (2, vec![lit("b")]),
                (3, vec![lit("c")]),
            ],
        );
        let r1 = positional(
            "R",
            &["y"],
            &[
                (1u64, vec![lit("p")]),
                (8, vec![lit("q")]),
                (9, vec![lit("r")]),
            ],
        );
        // three matches (all rows).
        let l3 = positional(
            "L",
            &["x"],
            &[
                (1u64, vec![lit("a")]),
                (2, vec![lit("b")]),
                (3, vec![lit("c")]),
            ],
        );
        let r3 = positional(
            "R",
            &["y"],
            &[
                (1u64, vec![lit("p")]),
                (2, vec![lit("q")]),
                (3, vec![lit("r")]),
            ],
        );
        let (out1, _) = join.fully_oblivious_batched_join(&l1, &r1, 5).unwrap();
        let (out3, _) = join.fully_oblivious_batched_join(&l3, &r3, 5).unwrap();
        // Different true counts (1 vs 3), identical disclosed-multiset SIZE only
        // after dummy-filter; the transcript-visible slot count is B=5 in both runs.
        assert_eq!(out1.rows.len(), 1, "true match count 1");
        assert_eq!(out3.rows.len(), 3, "true match count 3");
        // The protocol revealed exactly B=5 slots either way (cost.bound), proving
        // the count is bounded to B not the true cardinality — checked via cost.
        let (_, c1) = join.fully_oblivious_batched_join(&l1, &r1, 5).unwrap();
        let (_, c3) = join.fully_oblivious_batched_join(&l3, &r3, 5).unwrap();
        assert_eq!(
            c1.bound, c3.bound,
            "revealed slot bound identical regardless of true count"
        );
        assert_eq!(c1.bound, 5);
    }

    /// THE guard for the output stage's payload select in the batched join: with
    /// non-matching rows present, no rejected candidate's joined payload row appears
    /// among the opened payloads of the `B` slots. The opened set is exactly the
    /// matched rows plus the dummy encoding (two unbound cells).
    ///
    /// Asserted by comparing the candidates' OWN payload rows against what each slot
    /// actually opened to, through `fully_oblivious_candidates_and_opened_rows` — the
    /// same candidate construction and the same output stage the public path runs.
    #[test]
    fn rejected_batched_join_rows_payloads_open_to_the_dummy_encoding() {
        // Keys 1 and 3 overlap positionally; rows 1 and 3 do not match.
        let l = [
            (1u64, vec![lit("a")]),
            (2, vec![lit("b")]),
            (3, vec![lit("c")]),
            (4, vec![lit("d")]),
        ];
        let r = [
            (1u64, vec![lit("p")]),
            (9, vec![lit("q")]),
            (3, vec![lit("r")]),
            (8, vec![lit("s")]),
        ];
        let li = positional("L", &["x"], &l);
        let ri = positional("R", &["y"], &r);
        let render = |row: &[Option<Term>]| format!("{row:?}");
        let matched: Vec<Vec<Option<Term>>> =
            vec![vec![lit("a"), lit("p")], vec![lit("c"), lit("r")]];
        let pad: Vec<Option<Term>> = vec![None, None];
        for seed in 0..8u64 {
            let backend = ShamirBackend::new_seeded(5, 0xB0_0000 + seed).unwrap();
            let join = HiddenValueJoin::new(backend.clone());
            let (candidates, slots) = join
                .fully_oblivious_candidates_and_opened_rows(&mut backend.dealer(), &li, &ri, 6)
                .unwrap();
            assert_eq!(slots.len(), 6, "seed {seed}: exactly B slots");
            let opened: Vec<String> = slots.iter().map(|(_, r)| render(r)).collect();
            let mut rejected_seen = 0usize;
            for c in &candidates {
                if !matched.contains(&c.payload) {
                    rejected_seen += 1;
                    assert!(
                        !opened.contains(&render(&c.payload)),
                        "seed {seed}: a rejected row's payload was opened: {}",
                        render(&c.payload)
                    );
                }
            }
            assert!(
                rejected_seen > 0,
                "seed {seed}: fixture must contain rejected candidates"
            );
            let distinct: std::collections::BTreeSet<String> = opened.into_iter().collect();
            let mut want: std::collections::BTreeSet<String> =
                matched.iter().map(|r| render(r)).collect();
            want.insert(render(&pad));
            assert_eq!(distinct, want, "seed {seed}: opened payload set");
            for (is_real, row) in &slots {
                if *is_real {
                    assert!(matched.contains(row), "seed {seed}");
                } else {
                    assert_eq!(row, &pad, "seed {seed}: dummy slot payload");
                }
            }
        }
    }

    /// Fail-closed: a `bound` below the candidate count is rejected (never truncates
    /// a candidate, which could drop a true match) — same contract as `batched_join`.
    #[test]
    fn fully_oblivious_bound_below_candidate_count_fails_closed() {
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 1).unwrap());
        let l = positional("L", &["x"], &[(1u64, vec![lit("a")]), (2, vec![lit("b")])]);
        let r = positional("R", &["y"], &[(1u64, vec![lit("p")]), (2, vec![lit("q")])]);
        let err = join.fully_oblivious_batched_join(&l, &r, 1).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("truncate")));
    }

    /// No-overlap → empty result, matching plaintext; the output is all dummies up
    /// to B (no match bit opened to reveal the emptiness per pair).
    #[test]
    fn fully_oblivious_no_overlap_is_empty() {
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(5, 77).unwrap());
        let l = positional("L", &["x"], &[(1u64, vec![lit("a")]), (2, vec![lit("b")])]);
        let r = positional("R", &["y"], &[(7u64, vec![lit("p")]), (8, vec![lit("q")])]);
        let (result, cost) = join.fully_oblivious_batched_join(&l, &r, 4).unwrap();
        assert!(result.rows.is_empty(), "disjoint keys → empty join");
        assert_eq!(cost.bound, 4, "still reveals B slots (all dummies)");
    }

    /// Mixed bindings (one Positional, one Keyed) fail closed — same ambiguity guard
    /// as `batched_join`.
    #[test]
    fn fully_oblivious_mixed_bindings_fail_closed() {
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 1).unwrap());
        let l = positional("L", &["x"], &[(1u64, vec![lit("a")])]);
        let r = keyed("R", &["y"], &[("k", 1u64, vec![lit("p")])]);
        let err = join.fully_oblivious_batched_join(&l, &r, 4).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(m) if m.contains("row-binding")));
    }
}

#[cfg(test)]
mod randomness_seam_tests {
    //! sq-yyro / issue #6117 — the hidden-value equality path draws its key
    //! sharings and its nonzero equality mask through
    //! [`DistributedRandomness`] instead of the dealer's inherent `share` /
    //! `draw_nonzero_fp`. These tests pin that the single-dealer simulation draws
    //! exactly what it drew before the routing, and that a PRSS source drives the
    //! mask-only core correctly while failing closed on the input-sharing path.
    use super::*;
    use crate::prss::PrssRandomness;
    use crate::shamir::ShamirBackend;

    /// Pinned from the PRE-ROUTING code (`dealer.share` + `draw_nonzero_fp` +
    /// `share` inline). `secure_equal` returns only a bit, so the WITNESS draw
    /// after the calls is what pins the draw count: it is the next `draw_fp()`
    /// from the same dealer, and it moves if the equality path consumes more or
    /// fewer draws. It does not pin the order of the two key sharings, since
    /// swapping them changes neither the bit nor the count.
    #[test]
    fn seeded_equality_path_draws_exactly_what_the_dealer_drew_before_the_seam() {
        let backend = ShamirBackend::new_seeded(5, 0x5EED7).unwrap();
        let join = HiddenValueJoin::new(backend.clone());
        let mut dealer = backend.dealer();
        assert!(join
            .secure_equal(&mut dealer, Fp::new(42), Fp::new(42))
            .unwrap());
        assert!(!join
            .secure_equal(&mut dealer, Fp::new(42), Fp::new(43))
            .unwrap());
        assert_eq!(dealer.draw_fp().value(), 13364450358920826);

        let sa = dealer.share(Fp::new(9));
        let sb = dealer.share(Fp::new(9));
        assert!(join.secure_equal_shared(&mut dealer, &sa, &sb).unwrap());
        assert_eq!(dealer.draw_fp().value(), 1388396751701323652);
    }

    /// `secure_equal_shared` needs ONLY `shared_nonzero_mask` — which PRSS
    /// genuinely implements, dealer-lessly — so a PRSS source must decide the
    /// match CORRECTLY over keys that were already shared. PRSS draws from OS
    /// entropy, so this asserts the verdict invariant, not pinned values.
    #[test]
    fn prss_decides_the_match_bit_correctly_over_already_shared_keys() {
        let backend = ShamirBackend::new(5).unwrap();
        let join = HiddenValueJoin::new(backend.clone());
        let mut prss = PrssRandomness::with_simulated_setup(5).unwrap();
        let mut dealer = backend.dealer();
        for (a, b, expected) in [
            (7u64, 7u64, true),
            (7, 8, false),
            (0, 0, true),
            (0, 1, false),
        ] {
            let sa = dealer.share(Fp::new(a));
            let sb = dealer.share(Fp::new(b));
            assert_eq!(
                join.secure_equal_shared(&mut prss, &sa, &sb).unwrap(),
                expected,
                "PRSS-masked equality disagreed with the plaintext for ({a}, {b})"
            );
        }
    }

    /// Fail-closed: the scalar `secure_equal` must first share both keys, which
    /// PRSS refuses (it generates correlated randomness; it does not verifiably
    /// share a holder's own input). The refusal must surface as PRSS's own
    /// `NotYetImplemented`, not an unverified sharing.
    #[test]
    fn prss_fails_closed_when_the_equality_path_must_share_the_keys() {
        let backend = ShamirBackend::new(5).unwrap();
        let join = HiddenValueJoin::new(backend);
        let mut prss = PrssRandomness::with_simulated_setup(5).unwrap();
        let outcome = join.secure_equal(&mut prss, Fp::new(1), Fp::new(1));
        assert!(
            matches!(outcome, Err(MpcError::NotYetImplemented { .. })),
            "a PRSS source must refuse to share the join keys, got {:?}",
            outcome
        );
    }

    /// A source whose plain `shared_mask` deals ZERO and whose
    /// `shared_nonzero_mask` deals a real nonzero value — the `r = 0` threat from
    /// the [`crate::randomness`] module note, made deterministic.
    ///
    /// This is what pins the CHOICE of seam method at the equality call site: with
    /// `r = 0` the opened `m = (a−b)·r` is `0` for every pair, so a call site that
    /// drew the plain mask would report "equal" for UNEQUAL keys — a false match on
    /// every row. The two methods consume the same number of RNG draws in the
    /// single-dealer simulation, so the pinned-value test above cannot tell them
    /// apart; this one can.
    struct ZeroPlainMaskSource {
        inner: ShamirDealer,
    }
    impl DistributedRandomness for ZeroPlainMaskSource {
        fn randomness_model(&self) -> crate::randomness::RandomnessModel {
            self.inner.randomness_model()
        }
        fn parties(&self) -> usize {
            DistributedRandomness::parties(&self.inner)
        }
        fn threshold(&self) -> usize {
            DistributedRandomness::threshold(&self.inner)
        }
        fn shared_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            Ok(self.inner.share(Fp::zero()))
        }
        fn shared_nonzero_mask(&mut self) -> Result<Vec<Share>, MpcError> {
            self.inner.shared_nonzero_mask()
        }
        fn vss_own_input(&mut self, secret: Fp) -> Result<Vec<Share>, MpcError> {
            self.inner.vss_own_input(secret)
        }
        fn oblivious_shuffle(
            &mut self,
            col: crate::oblivious::SecretColumn,
        ) -> Result<
            (
                crate::oblivious::SecretColumn,
                crate::oblivious::ShuffleCost,
            ),
            MpcError,
        > {
            self.inner.oblivious_shuffle(col)
        }
    }

    /// Both key sharings and the equality mask are shape-checked where they are
    /// consumed. Each case corrupts exactly ONE draw and must be refused by the
    /// shape check itself.
    #[test]
    fn malformed_draws_are_refused_at_every_equality_consumption_site() {
        use crate::randomness::test_sources::{
            MalformedSource, SeamMethod, ALL_MALFORMATIONS, SHAPE_REFUSAL,
        };
        let backend = ShamirBackend::new_seeded(5, 0xE9A5).unwrap();
        let join = HiddenValueJoin::new(backend.clone());
        let sa = backend.dealer().share(Fp::new(11));
        let sb = backend.dealer().share(Fp::new(12));
        for how in ALL_MALFORMATIONS {
            for (label, method, skip, scalar) in [
                ("key a", SeamMethod::VssOwnInput, 0, true),
                ("key b", SeamMethod::VssOwnInput, 1, true),
                ("mask (scalar path)", SeamMethod::SharedNonzeroMask, 0, true),
                (
                    "mask (shared path)",
                    SeamMethod::SharedNonzeroMask,
                    0,
                    false,
                ),
            ] {
                let mut src = MalformedSource::new(backend.dealer(), method, skip, how);
                let outcome = if scalar {
                    join.secure_equal(&mut src, Fp::new(11), Fp::new(12))
                } else {
                    join.secure_equal_shared(&mut src, &sa, &sb)
                };
                match outcome {
                    Err(MpcError::Protocol(m)) => {
                        assert!(
                            m.contains(SHAPE_REFUSAL),
                            "{label}, {how:?}: wrong check: {m}"
                        )
                    }
                    other => panic!("{label}, {how:?}: must fail closed, got {other:?}"),
                }
                assert!(src.fired(), "{label}: the targeted draw never happened");
            }
        }
    }

    #[test]
    fn the_equality_mask_comes_from_the_nonzero_seam_method() {
        let backend = ShamirBackend::new_seeded(5, 0xE9A1).unwrap();
        let join = HiddenValueJoin::new(backend.clone());
        let mut dealer = backend.dealer();
        let sa = dealer.share(Fp::new(11));
        let sb = dealer.share(Fp::new(12));
        let mut source = ZeroPlainMaskSource {
            inner: backend.dealer(),
        };
        // Drawing the NONZERO mask keeps the verdict honest; drawing the plain
        // (here zero) mask would report a false match.
        assert!(
            !join.secure_equal_shared(&mut source, &sa, &sb).unwrap(),
            "unequal keys must not match — the equality mask must be the nonzero one"
        );
        let sc = dealer.share(Fp::new(11));
        assert!(join.secure_equal_shared(&mut source, &sa, &sc).unwrap());
    }

    // ---- Input dealing: `shared_keys` / `batched_join` (#6117 step 4) ----

    fn lit(s: &str) -> Option<Term> {
        Some(Term::Literal(oxrdf::Literal::new_simple_literal(s)))
    }

    fn input(
        holder: &str,
        var: &str,
        rows: &[(u64, &str)],
        binding: RowBinding,
    ) -> BatchedHiddenInput {
        BatchedHiddenInput::new(
            HolderId::new(holder),
            vec![Variable::new_unchecked(var)],
            rows.iter()
                .map(|(k, p)| (Fp::new(*k), vec![lit(p)]))
                .collect(),
            binding,
        )
        .unwrap()
    }

    fn ys(shares: &[Share]) -> Vec<u64> {
        shares.iter().map(|s| s.y.value()).collect()
    }

    /// The positional fixture: keys `[5, 7, 9]` vs `[5, 8, 9]`, so rows 0 and 2 match.
    fn positional_pair() -> (BatchedHiddenInput, BatchedHiddenInput) {
        (
            input(
                "L",
                "l",
                &[(5, "l0"), (7, "l1"), (9, "l2")],
                RowBinding::Positional,
            ),
            input(
                "R",
                "r",
                &[(5, "r0"), (8, "r1"), (9, "r2")],
                RowBinding::Positional,
            ),
        )
    }

    /// The keyed fixture (`n = 3`): disclosed keys `a`, `b` on both sides, in
    /// different orders; only `a` carries equal hidden keys.
    fn keyed_pair() -> (BatchedHiddenInput, BatchedHiddenInput) {
        (
            input(
                "L",
                "l",
                &[(1, "la"), (2, "lb")],
                RowBinding::Keyed(vec!["a".into(), "b".into()]),
            ),
            input(
                "R",
                "r",
                &[(3, "rb"), (1, "ra")],
                RowBinding::Keyed(vec!["b".into(), "a".into()]),
            ),
        )
    }

    fn rows_of(result: &PartialResult) -> Vec<Vec<Option<Term>>> {
        result.rows.clone()
    }

    /// `shared_keys` pins its three returned sharings and a direct WITNESS draw,
    /// both unchanged since the pre-routing code (`dealer.share_batch`).
    ///
    /// The two batched joins pin their output and a WITNESS draw from the join's
    /// own dealer; the witness moves if the keys, the masks or the output stage
    /// consume more or fewer draws. **Both witnesses moved in #6117 step 6**
    /// (`1791168067962045003` → `459721877472897114` positional,
    /// `604506784193922450` → `949282158979249119` keyed) because the oblivious
    /// output stage now draws from the SAME source instead of minting a second
    /// dealer: the join consumes 16 more draws on one keystream. The REVEALED
    /// result and every modelled cost below are byte-identical to the pre-routing
    /// values, and `seeded_batched_join_transcripts_are_pinned` shows the
    /// key-and-mask PREFIX of the transcript is unchanged — only the output-stage
    /// suffix is new.
    #[test]
    fn seeded_key_dealing_draws_exactly_what_the_dealer_drew_before_the_seam() {
        let b = ShamirBackend::new_seeded(5, 0x5EED8).unwrap();
        let l = input(
            "L",
            "a",
            &[(11, "x"), (22, "y"), (33, "z")],
            RowBinding::Positional,
        );
        let mut dealer = b.dealer();
        let shared = l.shared_keys(&mut dealer).unwrap();
        assert_eq!(
            shared.elements().iter().map(|s| ys(s)).collect::<Vec<_>>(),
            [
                [
                    1019715604101880969,
                    1282736186249256194,
                    789061746442125686,
                    1844535293894183396,
                    2143313819391735373
                ],
                [
                    413971500113348580,
                    1415338825238769910,
                    698258966162570061,
                    568574932098442984,
                    1026286723046388679
                ],
                [
                    1865313469515181088,
                    469639919993982597,
                    424665369863792462,
                    1730389819124610683,
                    2080970258562743309
                ]
            ]
        );
        assert_eq!(dealer.draw_fp().value(), 1086533885347419370);

        let b = ShamirBackend::new_seeded(5, 0xBA7C1).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        let (public, _) = join.batched_join(&l, &r, 3).unwrap();
        let mut dealer = b.dealer();
        let (through, cost) = join.batched_join_through(&mut dealer, &l, &r, 3).unwrap();
        assert_eq!(
            rows_of(&through),
            vec![vec![lit("l0"), lit("r0")], vec![lit("l2"), lit("r2")]]
        );
        assert_eq!(rows_of(&public), rows_of(&through));
        // Every modelled cost is identical to the pre-routing capture.
        assert_eq!(
            (
                cost.bound,
                cost.select_mults,
                cost.opens,
                cost.shuffle.items,
                cost.shuffle.switches,
                cost.shuffle.depth_rounds
            ),
            (3, 3, 3, 3, 12, 3)
        );
        assert_eq!(dealer.draw_fp().value(), 1985712154807401802);

        let b = ShamirBackend::new_seeded(3, 0xBA7C2).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = keyed_pair();
        let mut dealer = b.dealer();
        let (through, cost) = join.batched_join_through(&mut dealer, &l, &r, 2).unwrap();
        assert_eq!(rows_of(&through), vec![vec![lit("la"), lit("ra")]]);
        assert_eq!(
            (cost.bound, cost.shuffle.items, cost.shuffle.switches),
            (2, 2, 4)
        );
        assert_eq!(dealer.draw_fp().value(), 2292455923086310036);
    }

    /// The COMPLETE seam transcript of a batched join — both key columns (left
    /// then right, each in row order), one equality mask per candidate, then the
    /// OUTPUT STAGE's sharings and its shuffle — folded into the order-sensitive
    /// fingerprint. Swapping the two key columns, or re-ordering rows, changes it.
    ///
    /// **This is the #6117-step-6 guard.** The method list ends in the output
    /// stage's `vss_own_input` run and its `oblivious_shuffle`: if
    /// `batched_join_through` went back to minting its own dealer for the output
    /// stage, the recorded log would stop at the equality masks and this
    /// `assert_eq!` on `methods` would fail.
    ///
    /// The PREFIX fingerprints are the pre-routing whole-log pins
    /// (`0x3928_3c3c_85e4_183f` / `0xbd36_3090_bbad_ff3c`, captured on the
    /// pre-routing code by mapping each logged `share` to `vss_own_input` and each
    /// `draw_nonzero_fp` + `share` pair to `shared_nonzero_mask`), asserted here
    /// unchanged: the routing appended an output-stage SUFFIX and moved nothing in
    /// the key/mask part. The full-log fingerprints are the new pins.
    ///
    /// The third case (`B = 5`, padding two pure-dummy slots onto the same three
    /// candidates) is a NEW fixture: its method list and full-log fingerprint were
    /// captured on the current code, so they pin today's behaviour forward and
    /// make no pre-routing claim. Its PREFIX, however, is asserted against the
    /// same pre-routing pin as the `B = 3` case.
    #[test]
    fn seeded_batched_join_transcripts_are_pinned() {
        use crate::randomness::test_sources::{
            fingerprint, prefix_before_first_shuffle, RecordingSource,
        };
        let b = ShamirBackend::new_seeded(5, 0xBA7C1).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        let mut rec = RecordingSource::new(b.dealer());
        join.batched_join_through(&mut rec, &l, &r, 3).unwrap();
        let methods: Vec<&str> = rec.log.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            methods,
            [
                ["vss_own_input"; 6].as_slice(),
                &["shared_nonzero_mask"; 3],
                // The output stage: per slot a selector (a PUBLIC bit, lifted
                // with its own sharing) + a real-tag sharing + `2t + 1 = 5`
                // tag-reduction re-sharings, then per payload limb the limb
                // sharing + 5 re-sharings — 25 per slot over three slots — and
                // finally the shuffle over the three packed entries.
                &["vss_own_input"; 75],
                &["oblivious_shuffle"],
            ]
            .concat()
        );
        // Prefix unchanged from the pre-routing pin; suffix is the new output stage.
        assert_eq!(fingerprint(&rec.log[..9]), 0x3928_3c3c_85e4_183f);
        // The draws BEFORE the shuffle are byte-for-byte unchanged: this prefix
        // fingerprint is the value the pre-re-randomisation code produced, so only
        // the shuffle's own output and the draws after it moved.
        assert_eq!(
            fingerprint(prefix_before_first_shuffle(&rec.log)),
            0x35e4_8905_57b6_816d
        );
        assert_eq!(fingerprint(&rec.log), 0xf563_03af_2cc8_0641);

        let b = ShamirBackend::new_seeded(3, 0xBA7C2).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = keyed_pair();
        let mut rec = RecordingSource::new(b.dealer());
        join.batched_join_through(&mut rec, &l, &r, 2).unwrap();
        let methods: Vec<&str> = rec.log.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            methods,
            [
                ["vss_own_input"; 4].as_slice(),
                &["shared_nonzero_mask"; 2],
                // At `n = 3`, `t = 1` each reduction re-shares through 3 parties:
                // per slot the tag costs 2 + 3 = 5 and each of the three limbs
                // 1 + 3 = 4, so 17 per slot over two slots.
                &["vss_own_input"; 34],
                &["oblivious_shuffle"],
            ]
            .concat()
        );
        assert_eq!(fingerprint(&rec.log[..6]), 0xbd36_3090_bbad_ff3c);
        // The draws BEFORE the shuffle are byte-for-byte unchanged: this prefix
        // fingerprint is the value the pre-re-randomisation code produced, so only
        // the shuffle's own output and the draws after it moved.
        assert_eq!(
            fingerprint(prefix_before_first_shuffle(&rec.log)),
            0x1ac1_763a_ba2c_84a9
        );
        assert_eq!(fingerprint(&rec.log), 0x4d93_31c7_2de2_08e0);

        // `B = 5` over the SAME three candidates: the output stage pads with two
        // PURE-DUMMY slots on top of the three real ones. That padding branch is
        // where a regression can hide — a `bound > candidates.len()` special case
        // that fell back to minting a dealer for the output stage would leave this
        // log with only the key/mask prefix. The prefix fingerprint is the SAME pin
        // as the `B = 3` case above (same backend seed, same first nine draws):
        // only the output suffix differs with `B`.
        let b = ShamirBackend::new_seeded(5, 0xBA7C1).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        let mut rec = RecordingSource::new(b.dealer());
        join.batched_join_through(&mut rec, &l, &r, 5).unwrap();
        let methods: Vec<&str> = rec.log.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            methods,
            [
                ["vss_own_input"; 6].as_slice(),
                &["shared_nonzero_mask"; 3],
                // Five slots now, at the same 25 sharings per slot as the
                // `B = 3` case (75 for three), then one shuffle over all five.
                &["vss_own_input"; 125],
                &["oblivious_shuffle"],
            ]
            .concat()
        );
        // Same prefix pin as `B = 3`: `B` changes only the output suffix.
        assert_eq!(fingerprint(&rec.log[..9]), 0x3928_3c3c_85e4_183f);
        // The draws BEFORE the shuffle are byte-for-byte unchanged: this prefix
        // fingerprint is the value the pre-re-randomisation code produced, so only
        // the shuffle's own output and the draws after it moved.
        assert_eq!(
            fingerprint(prefix_before_first_shuffle(&rec.log)),
            0x33be_42c1_760b_6dbe
        );
        assert_eq!(fingerprint(&rec.log), 0xebc2_32c7_b6b7_cb63);
    }

    /// Every key sharing (any row of either column), every equality mask AND every
    /// sharing the OUTPUT stage draws (real tag, pure-dummy selector, payload limb)
    /// in a batched join is shape-checked where it is consumed: corrupting exactly
    /// ONE draw must be refused by a shape check itself — either the stage's own
    /// `checked_vss_own_input` or `shamir::degree_reduce_through`'s
    /// `require_canonical_sharing` on a reduction re-sharing; both name the
    /// canonical sharing, so the assertion below matches either wording. The 81
    /// `vss_own_input` sites are the 6 key sharings plus the output stage's 75: per
    /// slot a selector + a real-tag sharing + `2t + 1 = 5` tag-reduction
    /// re-sharings, then per payload limb the limb sharing + 5 re-sharings.
    #[test]
    fn malformed_draws_are_refused_at_every_batched_join_consumption_site() {
        use crate::randomness::test_sources::{MalformedSource, SeamMethod, ALL_MALFORMATIONS};
        /// Either shape refusal's common substring (the stage's own, or the
        /// reduction's "re-sharing" wording).
        const ANY_SHAPE_REFUSAL: &str = "sharing that is not the canonical";
        let b = ShamirBackend::new_seeded(5, 0xBA7C3).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        let sites = (0..81)
            .map(|k| (SeamMethod::VssOwnInput, k))
            .chain((0..3).map(|k| (SeamMethod::SharedNonzeroMask, k)));
        for (method, skip) in sites {
            for how in ALL_MALFORMATIONS {
                let mut src = MalformedSource::new(b.dealer(), method, skip, how);
                match join.batched_join_through(&mut src, &l, &r, 3) {
                    Err(MpcError::Protocol(m)) => assert!(
                        m.contains(ANY_SHAPE_REFUSAL),
                        "{method:?} #{skip}, {how:?}: wrong check: {m}"
                    ),
                    other => {
                        panic!("{method:?} #{skip}, {how:?}: must fail closed, got {other:?}")
                    }
                }
                assert!(
                    src.fired(),
                    "{method:?} #{skip}: the targeted draw never happened"
                );
            }
        }
    }

    /// A source whose `(n, t)` differs from the backend's is refused before any
    /// key is dealt. A misreported threshold yields canonically SHAPED sharings,
    /// so only the source-match check catches it.
    #[test]
    fn batched_join_refuses_a_mismatched_source_before_any_draw() {
        use crate::randomness::test_sources::{MisreportedThresholdSource, RecordingSource};
        const MISMATCH_REFUSAL: &str = "its sharings would not compose with the backend's";
        let b = ShamirBackend::new_seeded(5, 0xBA7C4).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        for t in [0, 1] {
            let mut src = MisreportedThresholdSource {
                inner: b.dealer(),
                t,
            };
            match join.batched_join_through(&mut src, &l, &r, 3) {
                Err(MpcError::Protocol(m)) => {
                    assert!(m.contains(MISMATCH_REFUSAL), "t={t}: wrong check: {m}")
                }
                other => panic!("t={t}: must fail closed, got {other:?}"),
            }
        }
        let mut rec = RecordingSource::new(ShamirBackend::new_seeded(3, 0xBA7C4).unwrap().dealer());
        let outcome = join.batched_join_through(&mut rec, &l, &r, 3);
        assert!(
            matches!(&outcome, Err(MpcError::Protocol(m)) if m.contains(MISMATCH_REFUSAL)),
            "got {outcome:?}"
        );
        assert!(rec.log.is_empty(), "drew before refusing");
    }

    /// The batched join's output stage now shuffles through the caller's source,
    /// so the column it gets back is SHAPE-checked rather than trusted: each
    /// corruption must be refused by the shuffle shape check itself (#6117 step 6).
    #[test]
    fn a_malformed_shuffled_column_is_refused_by_the_batched_join() {
        use crate::randomness::test_sources::{
            MalformedShuffleSource, ALL_SHUFFLE_MALFORMATIONS, SHUFFLE_SHAPE_REFUSAL,
        };
        let b = ShamirBackend::new_seeded(5, 0xBA7C5).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        for how in ALL_SHUFFLE_MALFORMATIONS {
            let mut src = MalformedShuffleSource::new(b.dealer(), how);
            match join.batched_join_through(&mut src, &l, &r, 3) {
                Err(MpcError::Protocol(m)) => assert!(
                    m.contains(SHUFFLE_SHAPE_REFUSAL),
                    "{how:?}: wrong check: {m}"
                ),
                other => panic!("{how:?}: must fail closed, got {other:?}"),
            }
            assert!(src.fired(), "{how:?}: the shuffle never happened");
        }
    }

    // ---- The fully-oblivious join's output stage (#6117 step 5) ----

    /// The fully-oblivious fixture: the positional pair at `bound = 4`, so the
    /// three candidates get one pure-dummy padding slot. At `n = 5`, `t = 2` each
    /// degree reduction re-shares through `2t + 1 = 5` parties, so the output stage
    /// draws 97 sharings — per candidate a real-tag sharing + 5 reduction
    /// re-sharings (the selector is the SHARED equality bit, which costs no draw),
    /// for the padding slot a zero-selector + a real-tag sharing + 5 re-sharings,
    /// then for each of the three payload limbs per slot the limb sharing + 5
    /// re-sharings — and then one shuffle.
    const OUTPUT_STAGE_DRAWS: usize = 98;

    /// The rows are the canonicalised result — unchanged — and the WITNESS draw
    /// after the run pins the total draw count of the whole join. The witness and
    /// the modelled costs other than `degree_reductions` were pinned against the
    /// pre-routing code; the witness was re-captured when the per-element degree
    /// reduction added its `2t + 1` re-sharings per selected element.
    #[test]
    fn seeded_fully_oblivious_join_draws_exactly_what_the_dealer_drew_before_the_seam() {
        let b = ShamirBackend::new_seeded(5, 0x0B16).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        let mut dealer = b.dealer();
        let (through, cost) = join
            .fully_oblivious_batched_join_through(&mut dealer, &l, &r, 4)
            .unwrap();
        assert_eq!(
            rows_of(&through),
            vec![vec![lit("l0"), lit("r0")], vec![lit("l2"), lit("r2")]]
        );
        assert_eq!(cost.bound, 4);
        assert_eq!(cost.shuffle.items, 4);
        assert_eq!(cost.shuffle.switches, 20);
        // One degree reduction per selected element: the tag plus its three limbs,
        // over four slots.
        assert_eq!(cost.degree_reductions, 16);
        assert_eq!(dealer.draw_fp().value(), 982498321419405726);

        // The public entry point mints its own dealer from the same seeded
        // backend, so it reproduces the same run.
        let (public, _) = join.fully_oblivious_batched_join(&l, &r, 4).unwrap();
        assert_eq!(rows_of(&public), rows_of(&through));
    }

    /// The OUTPUT-STAGE suffix of the fully-oblivious join's seam transcript —
    /// every sharing in consumption order then the shuffled column — folded into
    /// the order-sensitive fingerprint. Only the suffix: the long prefix is the
    /// per-pair `secure_equal_to_bit` chain, whose own transcript the `compare` seam
    /// tests already pin, and which this change does not touch. The suffix pin was
    /// re-captured when the per-element degree reduction grew the stage's draws.
    #[test]
    fn seeded_fully_oblivious_output_stage_transcript_is_pinned() {
        use crate::randomness::test_sources::{fingerprint, RecordingSource};
        let b = ShamirBackend::new_seeded(5, 0x0B16).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        let mut rec = RecordingSource::new(b.dealer());
        join.fully_oblivious_batched_join_through(&mut rec, &l, &r, 4)
            .unwrap();
        assert!(
            rec.log.len() > OUTPUT_STAGE_DRAWS,
            "the equality chain must have drawn through the same source first"
        );
        let suffix = &rec.log[rec.log.len() - OUTPUT_STAGE_DRAWS..];
        let methods: Vec<&str> = suffix.iter().map(|(m, _)| *m).collect();
        assert_eq!(
            methods,
            [
                ["vss_own_input"; OUTPUT_STAGE_DRAWS - 1].as_slice(),
                &["oblivious_shuffle"]
            ]
            .concat()
        );
        assert_eq!(fingerprint(suffix), 0xefb1_95a0_08bb_d36e);
    }

    /// A source whose `(n, t)` differs from the backend's is refused before the
    /// fully-oblivious join draws anything at all.
    #[test]
    fn fully_oblivious_join_refuses_a_mismatched_source_before_any_draw() {
        use crate::randomness::test_sources::{MisreportedThresholdSource, RecordingSource};
        const MISMATCH_REFUSAL: &str = "its sharings would not compose with the backend's";
        let b = ShamirBackend::new_seeded(5, 0x0B1A).unwrap();
        let join = HiddenValueJoin::new(b.clone());
        let (l, r) = positional_pair();
        for t in [0, 1] {
            let mut src = MisreportedThresholdSource {
                inner: b.dealer(),
                t,
            };
            match join.fully_oblivious_batched_join_through(&mut src, &l, &r, 4) {
                Err(MpcError::Protocol(m)) => {
                    assert!(m.contains(MISMATCH_REFUSAL), "t={t}: wrong check: {m}")
                }
                other => panic!("t={t}: must fail closed, got {other:?}"),
            }
        }
        let mut rec = RecordingSource::new(ShamirBackend::new_seeded(3, 0x0B1A).unwrap().dealer());
        let outcome = join.fully_oblivious_batched_join_through(&mut rec, &l, &r, 4);
        assert!(
            matches!(&outcome, Err(MpcError::Protocol(m)) if m.contains(MISMATCH_REFUSAL)),
            "got {outcome:?}"
        );
        assert!(rec.log.is_empty(), "drew before refusing");
    }

    /// The batched join must first share both key columns, which PRSS refuses —
    /// the refusal must surface as PRSS's own `NotYetImplemented`, for both the
    /// key dealing alone and the whole join.
    #[test]
    fn prss_fails_closed_when_the_batched_join_must_share_the_keys() {
        let backend = ShamirBackend::new(5).unwrap();
        let join = HiddenValueJoin::new(backend);
        let mut prss = PrssRandomness::with_simulated_setup(5).unwrap();
        let (l, r) = positional_pair();
        assert!(matches!(
            l.shared_keys_through(&mut prss, 5),
            Err(MpcError::NotYetImplemented { .. })
        ));
        assert!(matches!(
            join.batched_join_through(&mut prss, &l, &r, 3),
            Err(MpcError::NotYetImplemented { .. })
        ));
    }

    /// EMPTY inputs on both sides: no key to share and no candidate to compare,
    /// so the key/equality stages draw nothing at all. The join must still fail
    /// closed under PRSS, because the OUTPUT stage draws through the same source
    /// for its `B = 2` pure-dummy slots (#6117 step 6). Before that routing this
    /// shape ran to completion under PRSS, since the output stage minted its own
    /// dealer and never touched the caller's source — so this is the shape that
    /// pins the routing rather than the key dealing.
    #[test]
    fn prss_fails_closed_on_an_empty_batched_join_whose_only_draws_are_the_output_stage() {
        let backend = ShamirBackend::new(5).unwrap();
        let join = HiddenValueJoin::new(backend);
        let empty = |holder: &str, var: &str| input(holder, var, &[], RowBinding::Positional);
        let (l, r) = (empty("L", "l"), empty("R", "r"));
        // The key dealing itself is draw-free here: an empty column shares nothing,
        // so PRSS has nothing to refuse and succeeds.
        let mut prss = PrssRandomness::with_simulated_setup(5).unwrap();
        assert!(l.shared_keys_through(&mut prss, 5).is_ok());
        // The whole join nonetheless fails closed — the refusal comes from the
        // output stage, which is only reachable because it draws from `prss`.
        let outcome = join.batched_join_through(&mut prss, &l, &r, 2);
        assert!(
            matches!(outcome, Err(MpcError::NotYetImplemented { .. })),
            "got {:?}",
            outcome
        );
    }
}
