// sq-dwb5: batched / vector secret sharing — generalise the
// single-scalar `share_private_input` to a holder sharing a `Vec<Fp>` (per-row
// hidden values / a salary vector / per-graph commitments) under a DOCUMENTED
// row-binding, so the secure aggregate and the hidden-value join can range over
// more than one value per source.
//! Batched (vector) secret sharing + its row-binding contract.
//!
//! Architecture refs: §4.3 step 4 (the secure computation over secret-shared
//! per-source values), §4.2 (minimise inter-source disclosure). Skill
//! `mpc-protocols`: batched secret sharing is the standard way to secure a
//! *column* of private values under one MPC session.
//!
//! ## The gap this closes
//!
//! [`crate::backend::MpcBackend::share_private_input`] caps a holder's secure
//! contribution to ONE scalar (one row, one column — its single salary). The
//! four-flatmates aggregate works because each flatmate has exactly one private
//! salary; but the moment a holder has a *column* of private values (per-row
//! hidden join keys, a salary history, a per-graph commitment vector) the single
//! scalar is not enough. This module is the batched primitive: a holder shares a
//! `Vec<Fp>`, and the result carries an EXPLICIT row-binding so downstream
//! operators correlate elements across holders correctly.
//!
//! ## Row-binding contract (READ THIS — it is load-bearing for correctness)
//!
//! A [`BatchedShares`] is an ordered `Vec` of per-element Shamir sharings. The
//! binding of element `i` to a logical row is one of two documented modes,
//! recorded in [`RowBinding`]:
//!
//! - [`RowBinding::Positional`] — element `i` is row `i`. Two holders' batches
//!   correlate by INDEX. This is sound only when both holders impose the SAME
//!   deterministic order on their rows; [`crate::backend::MpcBackend::share_private_inputs`]
//!   enforces this by value-sorting before sharing, so positional batches from
//!   different holders line up regardless of local row order. Use for a per-row
//!   secure aggregate where row `i` of holder A pairs with row `i` of holder B
//!   (e.g. element-wise cumulative sum of two equal-length salary vectors).
//! - [`RowBinding::Keyed`] — element `i` is bound to the DISCLOSED row key
//!   `keys[i]` (a public global IRI / row id, convention #6). Correlation across
//!   holders is by KEY, not index, so the orders need not match. The key column
//!   is disclosed (convention #4) and travels alongside the shares; the VALUE
//!   stays hidden. Use when rows are addressed by a public id rather than by
//!   position.
//!
//! Both preserve the single-scalar privacy guarantee element-wise: each element
//! has its own fresh degree-`t` masking polynomial (see
//! [`crate::shamir::ShamirDealer::share_batch`]), so any `≤ t` parties' shares of
//! the WHOLE batch are jointly independent of all the values.
//!
//! ## What is wired end-to-end here
//!
//! - [`BatchedShares`] / [`BatchedAuthShares`] — the carrier + binding metadata.
//! - [`BatchedShares::reconstruct`] — element-wise open (the inverse of sharing).
//! - [`per_row_sum`] — the multi-row secure aggregate: element-wise cumulative
//!   sum across several holders' positional batches, the zero-round Shamir
//!   linear fold lifted to a vector. This is the demonstrated end-to-end
//!   multi-row path (tested against the plaintext per-row sum).
//!
//! The full batched HIDDEN-VALUE join (a [`crate::join::HiddenValueJoin`] ranging
//! over many rows per holder via these batches, with the oblivious output
//! transform of [`crate::oblivious_join`]) is a larger wiring job tracked
//! separately — see the crate's bead tracker (follow-up to sq-dwb5). The
//! primitive + binding it needs is exactly what this module lands.

use crate::field::Fp;
use crate::partial::MpcError;
use crate::shamir::{add_shares, ShamirBackend, ShareVec};

/// How element `i` of a [`BatchedShares`] binds to a logical row — the documented
/// correlation contract downstream operators MUST honour (see module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowBinding {
    /// Element `i` is row `i`; holders correlate by INDEX. Sound only when every
    /// holder imposes the same deterministic row order (the value-sort in
    /// [`crate::backend::MpcBackend::share_private_inputs`] guarantees this).
    Positional,
    /// Element `i` is bound to the DISCLOSED public row key `keys[i]` (a global
    /// IRI / row id rendered to a `String`); holders correlate by KEY. The key
    /// column is disclosed (convention #4); the value stays hidden. The keys MUST
    /// be the same length as the batch.
    Keyed(Vec<String>),
}

/// A batched (vector) secret sharing: one per-element Shamir sharing
/// ([`ShareVec`]) per row, plus the [`RowBinding`] that says which row each
/// element is. Index `i` is the sharing of value `i` under the binding.
///
/// This is the batched generalisation of a single trait-`Share`
/// ([`ShareVec`]); the existing single-scalar path is the `len() == 1`
/// [`RowBinding::Positional`] special case.
#[derive(Debug, Clone)]
pub struct BatchedShares {
    /// One sharing per element; `elements[i]` is the degree-`t` Shamir sharing of
    /// value `i`. Each is on the canonical `n`-party points `x = 1..=n`.
    elements: Vec<ShareVec>,
    /// How `elements[i]` binds to a logical row (see [`RowBinding`]).
    binding: RowBinding,
}

impl BatchedShares {
    /// Assemble a batched sharing from its per-element sharings and a binding.
    /// A [`RowBinding::Keyed`] whose key count differs from the element count is
    /// a protocol error (the binding would be ambiguous).
    pub fn new(elements: Vec<ShareVec>, binding: RowBinding) -> Result<Self, MpcError> {
        if let RowBinding::Keyed(keys) = &binding {
            if keys.len() != elements.len() {
                return Err(MpcError::Protocol(format!(
                    "BatchedShares: keyed binding has {} keys but {} elements",
                    keys.len(),
                    elements.len()
                )));
            }
        }
        Ok(BatchedShares { elements, binding })
    }

    /// Number of shared rows in the batch.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether the batch is empty (a holder with no private rows).
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// The per-element sharings (index `i` is the sharing of value `i`).
    pub fn elements(&self) -> &[ShareVec] {
        &self.elements
    }

    /// The row-binding contract for this batch.
    pub fn binding(&self) -> &RowBinding {
        &self.binding
    }

    /// Element-wise reconstruct the whole batch to the plaintext value vector —
    /// the inverse of batched sharing. Each element opens via the backend's
    /// consistency-checked degree-`t` reconstruction. Used to open a DISCLOSED
    /// batched result (never to open a value that must stay hidden).
    pub fn reconstruct(&self, backend: &ShamirBackend) -> Result<Vec<Fp>, MpcError> {
        self.elements
            .iter()
            .map(|e| backend.reconstruct(e))
            .collect()
    }
}

/// sq-dwb5 — the **multi-row secure aggregate**: element-wise
/// cumulative sum across several holders' POSITIONAL batches. This is the Shamir
/// zero-round linear fold ([`add_shares`]) lifted to a vector — row `i` of the
/// result is the sum of row `i` across every holder's batch — so the secure
/// aggregate now ranges over more than one value per source.
///
/// **Preconditions (fail-closed):** at least one batch; every batch the same
/// length `k` (the positional row-binding requires aligned columns); every batch
/// [`RowBinding::Positional`]. A length / binding mismatch is an
/// [`MpcError::Protocol`], never a silent wrong answer. The result carries the
/// `k`-row positional binding. Zero communication rounds (pure local addition),
/// the honest-majority Shamir sweet spot.
pub fn per_row_sum(batches: &[BatchedShares]) -> Result<BatchedShares, MpcError> {
    let first = batches
        .first()
        .ok_or_else(|| MpcError::Protocol("per_row_sum: no input batches".into()))?;
    let k = first.len();
    for (h, b) in batches.iter().enumerate() {
        if b.binding != RowBinding::Positional {
            return Err(MpcError::Protocol(format!(
                "per_row_sum: batch {h} is not positionally bound — \
                 element-wise summation requires the positional row-binding"
            )));
        }
        if b.len() != k {
            return Err(MpcError::Protocol(format!(
                "per_row_sum: batch {h} has {} rows but the first batch has {k} — \
                 a per-row aggregate needs equal-length positional columns",
                b.len()
            )));
        }
    }

    // Element-wise fold: row i of the result = Σ_holder (row i of that holder).
    let mut acc: Vec<ShareVec> = first.elements.clone();
    for b in &batches[1..] {
        for (a, next) in acc.iter_mut().zip(b.elements.iter()) {
            *a = add_shares(a, next)?;
        }
    }
    BatchedShares::new(acc, RowBinding::Positional)
}

/// A batched AUTHENTICATED (IT-MAC) sharing: one
/// [`crate::authenticated::AuthenticatedShare`] per row, all under ONE session
/// MAC key `α`, plus the [`RowBinding`]. The vector analogue of a single
/// authenticated share — minted by
/// [`crate::shamir::MacSession::authenticated_share_batch`].
///
/// Linear ops on authenticated shares ([`crate::authenticated::auth_add`] …)
/// compose element-wise, so an authenticated per-row aggregate stays free and
/// keeps the MAC relation on every element. `α` is never reconstructed (the same
/// structural guarantee as the single-scalar [`crate::authenticated::AuthenticatedShare`]).
#[derive(Debug, Clone)]
pub struct BatchedAuthShares {
    elements: Vec<crate::authenticated::AuthenticatedShare>,
    binding: RowBinding,
}

impl BatchedAuthShares {
    /// Assemble a batched authenticated sharing. A [`RowBinding::Keyed`] with a
    /// mismatched key count is a protocol error.
    pub fn new(
        elements: Vec<crate::authenticated::AuthenticatedShare>,
        binding: RowBinding,
    ) -> Result<Self, MpcError> {
        if let RowBinding::Keyed(keys) = &binding {
            if keys.len() != elements.len() {
                return Err(MpcError::Protocol(format!(
                    "BatchedAuthShares: keyed binding has {} keys but {} elements",
                    keys.len(),
                    elements.len()
                )));
            }
        }
        Ok(BatchedAuthShares { elements, binding })
    }

    /// Number of authenticated rows.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether empty.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// The per-element authenticated sharings (index `i` is value `i`).
    pub fn elements(&self) -> &[crate::authenticated::AuthenticatedShare] {
        &self.elements
    }

    /// The row-binding contract.
    pub fn binding(&self) -> &RowBinding {
        &self.binding
    }

    /// The openable VALUE sharing of every element (index `i` → `[value_i]`). The
    /// MAC stays crate-private exactly as for a single authenticated share — there
    /// is no batched MAC opener here (a back door round the "α never reconstructed"
    /// guarantee); the value sharings open via the backend's reconstruct.
    pub fn value_shares(&self) -> Vec<ShareVec> {
        self.elements
            .iter()
            .map(|e| e.value_shares().to_vec())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    //! sq-dwb5 batched-sharing acceptance suite. The load-bearing tests:
    //! - `batched_round_trip_*`: a `Vec<Fp>` shares + reconstructs element-wise
    //!   across n ∈ {3,5,7}.
    //! - `t_shares_of_batch_are_independent_of_values`: ≤ t parties' shares of the
    //!   WHOLE batch are independent of the values (vector privacy).
    //! - `authenticated_batch_*`: the batched shares carry MACs; the MAC relation
    //!   holds element-wise; α is never reconstructable through the batched API.
    //! - `per_row_sum_equals_plaintext`: the multi-row secure aggregate matches
    //!   the plaintext per-row sum (the row-binding correctness differential).
    use super::*;
    use crate::field::P;
    use crate::shamir::{reconstruct_at_zero, Share};

    /// ACCEPTANCE: batched share → element-wise reconstruct round-trips across the
    /// honest-majority party counts n ∈ {3,5,7}.
    #[test]
    fn batched_round_trip_across_party_counts() {
        let values: Vec<Fp> = [0u64, 1, 42, 100_000, P - 1]
            .into_iter()
            .map(Fp::new)
            .collect();
        for n in [3usize, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0xBA7C + n as u64).unwrap();
            let mut dealer = backend.dealer();
            let elements = dealer.share_batch(&values);
            assert_eq!(elements.len(), values.len());
            let batch = BatchedShares::new(elements, RowBinding::Positional).unwrap();
            let opened = batch.reconstruct(&backend).unwrap();
            assert_eq!(opened, values, "n={n}: batch must reconstruct element-wise");
        }
    }

    /// ACCEPTANCE (privacy): any ≤ t parties' shares of the WHOLE batch are
    /// independent of the values — the single-scalar hiding property lifted to the
    /// vector. We witness it per element: t shares of EACH element are consistent
    /// with a DIFFERENT value, so the joint t-party view pins down nothing.
    #[test]
    fn t_shares_of_batch_are_independent_of_values() {
        let backend = ShamirBackend::new_seeded(7, 0x5EED).unwrap(); // t = 3
        let t = backend.threshold();
        assert_eq!(t, 3);
        let mut dealer = backend.dealer();
        let values: Vec<Fp> = [11u64, 22, 33].into_iter().map(Fp::new).collect();
        let batch =
            BatchedShares::new(dealer.share_batch(&values), RowBinding::Positional).unwrap();

        for (i, element) in batch.elements().iter().enumerate() {
            let t_views = &element[..t];
            // For a DIFFERENT target value, forge a (t+1)-th share so the t views
            // interpolate to it. Its existence is the information-theoretic hiding.
            let target = values[i].add(Fp::new(7777 + i as u64));
            let forged = forge_next_share(t_views, target, t);
            let mut combined = t_views.to_vec();
            combined.push(forged);
            assert_eq!(
                reconstruct_at_zero(&combined, t).unwrap(),
                target,
                "element {i}: t shares are consistent with a different value → they hide it"
            );
        }
    }

    /// ACCEPTANCE (authenticated): the batched shares carry IT-MACs; the MAC
    /// relation `reconstruct([α·xᵢ]) == α·xᵢ` holds for EVERY element, and the
    /// value sharings round-trip. Mirrors the single-scalar
    /// `authenticated_share_round_trips`, extended to the vector.
    #[test]
    fn authenticated_batch_round_trips_and_stays_authenticated() {
        let backend = ShamirBackend::new_seeded(5, 0xA17C).unwrap();
        let t = backend.threshold();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();
        let alpha = session.alpha_for_test();

        let values: Vec<Fp> = [30_000u64, 45_000, 0, 1, P - 1]
            .into_iter()
            .map(Fp::new)
            .collect();
        let auth = session.authenticated_share_batch(&values);
        let batch = BatchedAuthShares::new(auth, RowBinding::Positional).unwrap();

        for (i, element) in batch.elements().iter().enumerate() {
            let v = backend.reconstruct(element.value_shares()).unwrap();
            assert_eq!(v, values[i], "element {i} value round-trips");
            // The MAC relation holds element-wise (MAC opened only inside the test).
            let m = reconstruct_at_zero(element.mac_shares(), t).unwrap();
            assert_eq!(m, alpha.mul(values[i]), "element {i} MAC == α·value");
        }
    }

    /// ACCEPTANCE (α-leak): the batched authenticated API exposes NO MAC opener —
    /// `value_shares()` opens only the values; there is no path that reconstructs
    /// `α` from the batch (mirrors the single-scalar `x = 1` α-leak guard). The
    /// worst case x = 1 (whose MAC is α) is reachable only via the crate-private
    /// `mac_shares()`, never through the public batched surface.
    #[test]
    fn batched_authenticated_api_never_opens_alpha() {
        let backend = ShamirBackend::new_seeded(5, 0x1234).unwrap();
        let mut dealer = backend.dealer();
        let mut session = dealer.new_mac_session();

        // Mint a batch containing x = 1 (its MAC is α·1 = α — the worst case).
        let auth = session.authenticated_share_batch(&[Fp::one(), Fp::new(2)]);
        let batch = BatchedAuthShares::new(auth, RowBinding::Positional).unwrap();

        // The PUBLIC batched surface yields only the VALUE sharings; opening them
        // gives the values (1, 2), never α.
        let value_sharings = batch.value_shares();
        assert_eq!(backend.reconstruct(&value_sharings[0]).unwrap(), Fp::one());
        assert_eq!(backend.reconstruct(&value_sharings[1]).unwrap(), Fp::new(2));
        // There is deliberately no `batch.mac_shares()` / `batch.reconstruct_mac()`:
        // the MAC opener stays crate-private on each AuthenticatedShare.
    }

    /// ACCEPTANCE (row-binding correctness): the multi-row secure aggregate over
    /// several holders' positional batches equals the plaintext per-row sum. This
    /// is the differential that proves the row-binding correlates elements
    /// correctly across holders — row i of the secure result = Σ row i.
    #[test]
    fn per_row_sum_equals_plaintext() {
        let backend = ShamirBackend::new_seeded(5, 0xC0DE).unwrap();
        let mut dealer = backend.dealer();

        // Three holders, each a 4-row salary vector (the multi-row generalisation
        // of the four-flatmates single salary).
        let holders: [[u64; 4]; 3] = [
            [30_000, 12_000, 0, 5],
            [45_000, 8_000, 1, 0],
            [28_000, 0, 2, 100_000],
        ];
        let batches: Vec<BatchedShares> = holders
            .iter()
            .map(|rows| {
                let fps: Vec<Fp> = rows.iter().map(|&v| Fp::new(v)).collect();
                BatchedShares::new(dealer.share_batch(&fps), RowBinding::Positional).unwrap()
            })
            .collect();

        let summed = per_row_sum(&batches).unwrap();
        let got = summed.reconstruct(&backend).unwrap();

        // Plaintext per-row sum.
        let expected: Vec<Fp> = (0..4)
            .map(|i| Fp::new(holders.iter().map(|h| h[i]).sum::<u64>()))
            .collect();
        assert_eq!(
            got, expected,
            "per-row secure sum must equal plaintext per-row sum"
        );
    }

    /// per_row_sum fails closed on a length mismatch (mis-aligned positional
    /// columns) and on a non-positional binding — never a silent wrong answer.
    #[test]
    fn per_row_sum_rejects_misaligned_or_keyed_batches() {
        let backend = ShamirBackend::new_seeded(3, 1).unwrap();
        let mut dealer = backend.dealer();
        let a = BatchedShares::new(
            dealer.share_batch(&[Fp::new(1), Fp::new(2)]),
            RowBinding::Positional,
        )
        .unwrap();
        let b_short =
            BatchedShares::new(dealer.share_batch(&[Fp::new(3)]), RowBinding::Positional).unwrap();
        assert!(matches!(
            per_row_sum(&[a.clone(), b_short]),
            Err(MpcError::Protocol(_))
        ));

        let keyed = BatchedShares::new(
            dealer.share_batch(&[Fp::new(3), Fp::new(4)]),
            RowBinding::Keyed(vec!["r0".into(), "r1".into()]),
        )
        .unwrap();
        assert!(matches!(
            per_row_sum(&[a, keyed]),
            Err(MpcError::Protocol(_))
        ));
    }

    /// A keyed binding with a mismatched key count is rejected at construction.
    #[test]
    fn keyed_binding_key_count_must_match() {
        let backend = ShamirBackend::new_seeded(3, 2).unwrap();
        let mut dealer = backend.dealer();
        let elements = dealer.share_batch(&[Fp::new(1), Fp::new(2)]);
        assert!(matches!(
            BatchedShares::new(elements, RowBinding::Keyed(vec!["only-one-key".into()])),
            Err(MpcError::Protocol(_))
        ));
    }

    /// An empty batch reconstructs to an empty vector (a holder with no private
    /// rows contributes nothing) — the OWA-safe empty case.
    #[test]
    fn empty_batch_round_trips() {
        let backend = ShamirBackend::new_seeded(3, 3).unwrap();
        let mut dealer = backend.dealer();
        let batch = BatchedShares::new(dealer.share_batch(&[]), RowBinding::Positional).unwrap();
        assert!(batch.is_empty());
        assert_eq!(batch.reconstruct(&backend).unwrap(), Vec::<Fp>::new());
    }

    /// Given `t` shares on a degree-`t` polynomial, produce ONE more share so the
    /// `t+1` interpolate to `target` at `x = 0` (the hiding witness for the
    /// independence test). Mirrors `authenticated::tests::forge_next_share`.
    fn forge_next_share(views: &[Share], target: Fp, t: usize) -> Share {
        assert_eq!(views.len(), t, "need exactly t shares to leave one DOF");
        let xnew = (1..=10_000u64)
            .find(|x| views.iter().all(|s| s.x != *x))
            .expect("a fresh evaluation point exists");
        let mut xs: Vec<u64> = views.iter().map(|s| s.x).collect();
        xs.push(xnew);
        let weight = |i: usize| -> Fp {
            let xi = Fp::new(xs[i]);
            let mut num = Fp::one();
            let mut den = Fp::one();
            for (j, &xj) in xs.iter().enumerate() {
                if i == j {
                    continue;
                }
                let xj = Fp::new(xj);
                num = num.mul(xj.neg());
                den = den.mul(xi.sub(xj));
            }
            num.mul(den.inv())
        };
        let mut acc = Fp::zero();
        for (i, v) in views.iter().enumerate() {
            acc = acc.add(v.y.mul(weight(i)));
        }
        let l_new = weight(views.len());
        let y_new = target.sub(acc).mul(l_new.inv());
        Share { x: xnew, y: y_new }
    }
}
