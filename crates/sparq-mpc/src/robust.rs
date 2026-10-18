// sq-m34i (MPC WI-1): Reed-Solomon consistency-checked + robust
// (Berlekamp-Welch) reconstruction over Fp. Addresses malicious-security gap (D)
// for share OPENS at the Shamir layer (honest-majority abort/robust regime);
// it does not make the protocols above it malicious-secure. Written while
// Fable unavailable — re-review on return.
//! Reed–Solomon error-detection / error-correction for Shamir reconstruction.
//!
//! ## The codeword view (why this works with NO new dependency)
//!
//! A Shamir sharing of a degree-`t` secret polynomial `Q` evaluated at `n`
//! distinct nonzero points `x_1..x_n` is **exactly** an `[n, t+1]` Reed–Solomon
//! codeword over the existing field `F_p` ([`crate::field`]). The message is the
//! `t+1` low coefficients of `Q` (the secret is `Q(0)`); the codeword is the `n`
//! evaluations `y_i = Q(x_i)`. Plain Lagrange interpolation
//! (`shamir::reconstruct_at_zero`, now `#[cfg(test)]` — no production caller after
//! WI-2, so not an intra-doc link) is RS *decoding under the assumption
//! that every symbol is correct* — it has no consistency check, so one tampered
//! `y_i` silently yields the wrong `Q(0)` even when redundancy is present
//! (pinned by `adversarial_tests::tamper`; bead sq-uu0u).
//!
//! With redundancy (`n > t+1`) the same codeword view gives us, for FREE over the
//! same `F_p` (no Feldman/Pedersen DLOG group, no SPDZ MAC preprocessing, no
//! bigint — see sq-uu0u "REJECTED for now"):
//!
//! - **Detect** any tampering, for any number of errors, by checking all `n`
//!   points lie on a single degree-`t` polynomial.
//! - **Correct** up to `e = ⌊(n − t − 1)/2⌋` tampered shares (the RS / BW bound
//!   `n ≥ t + 2e + 1`) and return the TRUE secret — *robust* reconstruction.
//!
//! ## Threat model (honest-majority, information-theoretic)
//!
//! This adds detect-and-abort (and, where the bound allows, robust correction)
//! against ACTIVE tampering of share *values* at reconstruction — a step toward
//! guarantee (D) at the Shamir layer, not malicious security. It needs no PKI, broadcast, or preprocessing. It does NOT defend:
//!
//! - **Exactly `t+1` shares (no redundancy).** Any `t+1` arbitrary points define
//!   *some* degree-`t` polynomial, so a single forged share just selects a
//!   different (wrong) secret with no inconsistency to detect. No scheme can
//!   detect this; we preserve the existing behaviour and say so explicitly
//!   ([`reconstruct_robust`] returns the interpolation, never a false
//!   `Tampered`). This is pinned by a test.
//! - **More than the correctable budget with sub-detectable structure.** When
//!   `#errors > e` the BW system can fail to yield a consistent codeword; we
//!   ABORT ([`MpcError::Tampered`]) rather than guess. (RS *miscorrection* — a
//!   second valid codeword within distance `e` — needs `> e` coordinated errors
//!   and is the standard RS limitation, not a regression over plain Lagrange,
//!   which silently miscorrects on a SINGLE error.)
//!
//!   **Residual miscorrection window (the adversarial caveat, sq-6u6b (3)).** The
//!   one case where robust reconstruction returns a WRONG secret WITHOUT aborting
//!   is when `> e` tampered shares happen to land the corrupted vector within
//!   Hamming distance `e` of a *different* valid degree-`degree` codeword `Q'`.
//!   BW then "corrects" to `Q' ≠ Q` and the all-points re-check passes (the
//!   tampered points now look like the `≤ e` errors of `Q'`), so we return
//!   `Q'(0)` and name the honest minority as the corrected shares. This requires
//!   the adversary to (a) control more than `e` shares AND (b) choose their
//!   deltas so the result is near a second codeword — which over a random-looking
//!   field `F_p = 2^61 − 1` means hitting a `(degree+1)`-coordinate algebraic
//!   coincidence with the remaining honest shares. For independent/random deltas
//!   the probability is `≈ |codewords within distance e| / p^{redundancy}`, i.e.
//!   astronomically small (`~2^{-61}` per honest constraint). It is nonetheless a
//!   REAL malicious caveat, not a numerical artefact: it is the generic RS
//!   bounded-distance-decoding limit, and the information-theoretic MAC layer
//!   (WI-4, bead sq-6d6g) — whose soundness comes from the secret `α`, not from
//!   codeword distance — is what closes it. Detection of `≤ e` errors is, by
//!   contrast, unconditional.
//! - **The degree-`2t` equality/mult open at `n = 2t+1`** (`join::secure_equal`).
//!   WI-2 (bead sq-7q9i) routes that open through THIS primitive at degree `2t`
//!   ([`crate::shamir::reconstruct_degree`]): it detects/corrects when
//!   `n > 2t+1`, but at exactly `n = 2t+1` degree `2t` has NO RS redundancy, so
//!   the same no-redundancy non-detection above applies — a forged product share
//!   is undetectable there. A true fix at `n = 2t+1` needs an information-
//!   theoretic MAC (deferred WI-4, bead sq-6d6g), not RS redundancy.
//!
//! ## Algorithm — Berlekamp–Welch via Gaussian elimination over `F_p`
//!
//! For target error budget `e`, find an **error-locator** `E(x)` of degree `e`
//! (monic: leading coeff 1) and `N(x)` of degree `t + e` such that
//! `N(x_i) = y_i · E(x_i)` for every share `i`, where (by construction)
//! `N(x) = E(x)·Q(x)`. Writing the unknown coefficients of `E` (the `e` below the
//! monic top) and of `N` (`t+e+1` of them) gives a linear system in `t + 2e + 1`
//! unknowns over `n ≥ t + 2e + 1` equations:
//!
//! ```text
//!   N(x_i) − y_i·E(x_i) = 0
//!   ⟺  Σ_{k≤t+e} N_k x_i^k  −  y_i·(Σ_{k<e} E_k x_i^k + x_i^e) = 0
//!   ⟺  Σ_{k≤t+e} N_k x_i^k  −  Σ_{k<e} (y_i x_i^k) E_k  =  y_i x_i^e
//! ```
//!
//! We solve it by Gaussian elimination over `F_p` (uses [`Fp::inv`]/`mul`),
//! recover `Q = N / E` by exact polynomial division (the remainder MUST be zero
//! — if it is not, the candidate is rejected), and return `Q(0)`. Finally we
//! re-verify that *all* `n` original points satisfy the recovered `Q`; only then
//! is the result trusted. `O(n^3)` Gaussian elimination is fine at the intended
//! party counts (`n` = 3..9).

use crate::field::Fp;
use crate::partial::MpcError;
use crate::shamir::Share;

/// Robust / consistency-checked reconstruction of a degree-`degree` sharing's
/// value at `x = 0`, using the Reed–Solomon structure of the shares.
///
/// `degree` is the polynomial degree (`t` for a normal sharing). Let `n` be the
/// number of (distinct-`x`) shares supplied. Behaviour by redundancy:
///
/// - `n < degree + 1` → [`MpcError::Protocol`] (below threshold — cannot
///   reconstruct at all; same precondition as plain Lagrange).
/// - `n == degree + 1` (NO redundancy) → plain Lagrange interpolation. Tampering
///   is **undetectable** here and we do NOT claim otherwise (see module docs):
///   never returns [`MpcError::Tampered`].
/// - `n > degree + 1` (redundancy) → Berlekamp–Welch with the maximal
///   correctable budget `e = ⌊(n − degree − 1)/2⌋`:
///   - `≤ e` tampered shares → corrects them, returns the TRUE secret.
///   - `> e` tampered shares but still inconsistent → [`MpcError::Tampered`].
///   - all consistent → the secret (no behaviour change on honest input).
///
/// The returned secret is `Q(0)` of the recovered/agreed degree-`degree`
/// polynomial. On any inconsistency that cannot be corrected, returns
/// [`MpcError::Tampered`] with the identified cheater points where possible.
///
/// Callers that need the identity of the cheaters on the SUCCESS path (the shares
/// Berlekamp–Welch corrected — exact attribution, not a heuristic) should use
/// [`reconstruct_robust_attributed`] instead, which returns the same secret plus
/// that list.
pub fn reconstruct_robust(shares: &[Share], degree: usize) -> Result<Fp, MpcError> {
    reconstruct_robust_attributed(shares, degree).map(|r| r.secret)
}

/// Outcome of a successful robust reconstruction with cheater attribution.
///
/// `secret` is `Q(0)` of the recovered degree-`degree` polynomial. `cheaters`
/// holds the evaluation points (`Share::x`) of the shares that were CORRECTED —
/// i.e. whose supplied `y` disagrees with the recovered codeword. On the success
/// path (Berlekamp–Welch correction, or the pure `e = 0` consistency check) this
/// list is **exact**: the agreeing degree-`degree` majority uniquely pins the
/// codeword, so every named point really did carry a tampered value. It is empty
/// when all shares were already consistent (clean input, or the no-redundancy
/// `n == degree + 1` path where attribution is impossible).
///
/// Contrast with the abort path: when correction is impossible the honest set is
/// unknown, so [`MpcError::Tampered`]'s `cheaters` is a best-effort *vote*, not an
/// exact set. Here, because correction SUCCEEDED, attribution is sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobustReconstruction {
    /// The recovered secret `Q(0)`.
    pub secret: Fp,
    /// Evaluation points (`Share::x`) of the corrected (tampered) shares, sorted
    /// ascending. Exact on the success path; see the struct docs.
    pub cheaters: Vec<u64>,
}

/// Robust reconstruction that ALSO returns the exactly-identified cheaters on the
/// success path (bead sq-6u6b follow-up (1)).
///
/// Behaves exactly like [`reconstruct_robust`] for the secret value and for the
/// abort/precondition errors, but on success additionally surfaces which shares
/// Berlekamp–Welch had to correct. When correction succeeds the recovered `Q` is
/// the unique degree-`degree` codeword agreeing with the honest majority, so the
/// off-curve points are precisely the tampered shares — sound attribution, not a
/// heuristic. Callers that need to *act* on a cheater (e.g. exclude a party from
/// a re-run) should prefer this entry point over the value-only one.
pub fn reconstruct_robust_attributed(
    shares: &[Share],
    degree: usize,
) -> Result<RobustReconstruction, MpcError> {
    let n = shares.len();
    // `degree + 1` is the share count a degree-`degree` codeword needs. `degree` is
    // caller-supplied, so at `usize::MAX` the addition would wrap to 0 and the
    // comparison would ACCEPT a share set that cannot determine the polynomial.
    // Refuse instead of wrapping.
    let needed = degree.checked_add(1).ok_or_else(|| {
        MpcError::Protocol(format!(
            "robust reconstruction: degree {} + 1 overflows usize; no share count \
             could satisfy it",
            degree
        ))
    })?;
    if n < needed {
        return Err(MpcError::Protocol(format!(
            "robust reconstruction needs >= {} shares, got {}",
            needed, n
        )));
    }
    // Distinct evaluation points are required for RS decoding (a repeated x is a
    // structural protocol violation, not a value tamper).
    check_distinct_points(shares)?;

    // No redundancy: t+1 points always define SOME degree-t polynomial. There is
    // nothing to cross-check, so we preserve plain-Lagrange behaviour and make NO
    // detection claim (returning Tampered here would be a false positive). With no
    // redundancy we also cannot attribute anything, so cheaters is empty.
    if n == degree + 1 {
        return Ok(RobustReconstruction {
            secret: lagrange_at_zero(shares)?,
            cheaters: Vec::new(),
        });
    }

    // Redundant: maximal Berlekamp–Welch error budget for this (n, degree).
    // n >= degree + 2e + 1  ⟺  e <= (n - degree - 1)/2.
    let e_max = (n - degree - 1) / 2;

    // Try increasing error budgets 0..=e_max. e = 0 is the pure consistency
    // check (interpolate degree+1 points, verify the rest); larger e attempts
    // correction. The first budget that yields a polynomial consistent with all
    // points within that error budget wins.
    for e in 0..=e_max {
        if let Some(q) = berlekamp_welch(shares, degree, e) {
            // Re-verify against ALL points: the recovered Q must disagree with at
            // most e of them; the agreeing majority pins the true codeword. Those
            // off-curve points ARE exactly the corrected shares — sound attribution.
            let bad = disagreeing_points(shares, &q);
            if bad.len() <= e {
                let mut cheaters: Vec<u64> = bad.into_iter().map(|i| shares[i].x).collect();
                cheaters.sort_unstable();
                return Ok(RobustReconstruction {
                    secret: eval_poly(&q, Fp::zero()),
                    cheaters,
                });
            }
        }
    }

    // No degree-`degree` polynomial fits within the correctable budget: the
    // shares are mutually inconsistent beyond what redundancy can repair. Abort
    // (detect-and-abort), naming the cheaters we can pin by a voting scheme.
    Err(tampered_error(shares, degree, e_max))
}

/// Best-effort cheater attribution for the ABORT path (bead sq-6u6b follow-up
/// (2)): name the off-curve points of the MOST self-consistent reference subset.
///
/// When `> e` shares are tampered the honest set is unknowable, so no attribution
/// can be sound (the bead acknowledges this — detection is sound, blame is not).
/// The OLD code fixed the reference to the FIRST `degree+1` points; if a cheater
/// happened to sit in that arbitrary window the recovered polynomial was wrong and
/// it blamed the honest majority. We instead try **every** `degree+1`-point
/// reference subset, interpolate each, and keep the fit with the FEWEST off-curve
/// points across all `n` shares. The intuition: a fully-honest reference subset
/// recovers the true polynomial and disagrees only with the `≈ #cheaters` tampered
/// shares, whereas a cheater-containing subset recovers a spurious polynomial that
/// generically misses almost everything — so "fewest disagreements" is a strong
/// signal for "this reference subset was (probably) all honest", and its off-curve
/// set is then the best single-subset cheater estimate. Ties (and the pathological
/// all-miss case) fall back to the first such minimum, which is no worse than the
/// old fixed-subset behaviour.
///
/// **Measured guarantee and its limit.** In the FIRST abort band — exactly
/// `e_max + 1` tampered shares (one over the correctable budget) — a fully-honest
/// reference subset is the *unique* minimum and this names ONLY genuinely-tampered
/// points (no honest party framed; pinned by `abort_first_band_names_only_real_
/// cheaters` across `n = 5..9`). That is precisely the case the old fixed-first-
/// subset code could get wrong. As the error count grows further past the budget
/// the honest set becomes genuinely unidentifiable and the minimum-disagreement
/// subset can itself contain a cheater, so attribution degrades to a true best-
/// effort guess that MAY name an honest party. This is therefore NOT a blanket
/// soundness claim: [`MpcError::Tampered::cheaters`] stays documented as best-
/// effort and callers MUST NOT treat an abort-path name as proof of guilt.
/// Detection itself is always sound; only blame is heuristic. The success path
/// ([`reconstruct_robust_attributed`]) is where attribution is exact.
fn tampered_error(shares: &[Share], degree: usize, e_max: usize) -> MpcError {
    let n = shares.len();
    let suspects = best_effort_suspects(shares, degree);
    MpcError::Tampered {
        detail: format!(
            "RS consistency check failed: {n} shares of a degree-{degree} secret are not all on \
             one polynomial and the inconsistency exceeds the correctable budget e={e_max} \
             (need n >= degree + 2e + 1 to correct e errors); cheater attribution is best-effort \
             on this abort path"
        ),
        cheaters: suspects,
    }
}

/// Off-curve evaluation points of the most self-consistent `degree+1`-point
/// reference fit (sorted ascending). See [`tampered_error`] for the rationale.
fn best_effort_suspects(shares: &[Share], degree: usize) -> Vec<u64> {
    let mut best: Option<Vec<usize>> = None;
    // Enumerate all C(n, degree+1) reference subsets. At the intended party counts
    // (n = 3..9, degree+1 ≤ 5) this is ≤ C(9,5)=126 fits, each O(n^2) — negligible.
    for_each_combination(shares.len(), degree + 1, &mut |idxs| {
        let subset: Vec<Share> = idxs.iter().map(|&i| shares[i]).collect();
        let Ok(q) = lagrange_at_zero_poly(&subset) else {
            return;
        };
        let bad = disagreeing_points(shares, &q);
        // Keep the fit with the fewest disagreements (most likely all-honest).
        if best.as_ref().is_none_or(|b| bad.len() < b.len()) {
            best = Some(bad);
        }
    });
    match best {
        Some(bad) => {
            let mut xs: Vec<u64> = bad.into_iter().map(|i| shares[i].x).collect();
            xs.sort_unstable();
            xs
        }
        None => Vec::new(),
    }
}

/// Invoke `f` on each strictly-increasing combination of `k` indices drawn from
/// `0..n` (the lexicographic `C(n, k)` subsets). A reused index buffer means no
/// per-subset allocation. `k == 0` yields the single empty combination; `k > n`
/// yields none.
fn for_each_combination(n: usize, k: usize, f: &mut impl FnMut(&[usize])) {
    if k > n {
        return;
    }
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        f(&idx);
        if k == 0 {
            return; // the single empty combination
        }
        // Find the rightmost index that can be incremented. idx[i] may rise to at
        // most (n - k + i); past that the slot is "maxed out".
        let mut i = k - 1;
        while idx[i] == n - k + i {
            if i == 0 {
                return; // all slots maxed → last combination consumed
            }
            i -= 1;
        }
        idx[i] += 1;
        for j in (i + 1)..k {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

/// Indices of shares whose `y` does NOT equal `Q(x)` for the candidate poly `q`.
fn disagreeing_points(shares: &[Share], q: &[Fp]) -> Vec<usize> {
    shares
        .iter()
        .enumerate()
        .filter(|(_, s)| eval_poly(q, Fp::new(s.x)) != s.y)
        .map(|(i, _)| i)
        .collect()
}

/// Reject repeated evaluation points (RS decoding assumes distinct `x_i`).
fn check_distinct_points(shares: &[Share]) -> Result<(), MpcError> {
    for i in 0..shares.len() {
        for j in (i + 1)..shares.len() {
            if shares[i].x == shares[j].x {
                return Err(MpcError::Protocol(format!(
                    "robust reconstruction: repeated evaluation point x={}",
                    shares[i].x
                )));
            }
        }
    }
    Ok(())
}

/// Berlekamp–Welch decode for `n` shares of a degree-`degree` polynomial,
/// targeting exactly `e` correctable errors. Returns the recovered polynomial
/// `Q` (coeffs low→high, length `degree+1`) if the key-equation system is
/// solvable and `E | N` exactly; otherwise `None`.
///
/// Unknowns: `E_0..E_{e-1}` (the `e` non-leading error-locator coeffs; `E_e = 1`
/// is fixed monic) and `N_0..N_{degree+e}` (`degree+e+1` numerator coeffs). The
/// equation per share `i` is `N(x_i) − y_i·E(x_i) = 0`, rearranged so the known
/// `y_i·x_i^e` (from the fixed monic top of `E`) is the RHS.
fn berlekamp_welch(shares: &[Share], degree: usize, e: usize) -> Option<Vec<Fp>> {
    let n = shares.len();
    let num_n = degree + e + 1; // coeffs of N: N_0..N_{degree+e}
    let num_e = e; // unknown coeffs of E: E_0..E_{e-1}
    let cols = num_n + num_e;

    // We need at least as many equations as unknowns. With n >= degree+2e+1,
    // n >= num_n + num_e holds; use exactly the available n rows.
    if n < cols {
        return None;
    }

    // Build the augmented matrix [A | b] with n rows, `cols` unknown columns.
    // Column layout: [N_0..N_{degree+e}, E_0..E_{e-1}].
    let mut aug: Vec<Vec<Fp>> = Vec::with_capacity(n);
    for s in shares {
        let x = Fp::new(s.x);
        let y = s.y;
        let mut row = vec![Fp::zero(); cols + 1];
        // N coefficients: + x^k.
        let mut xpow = Fp::one();
        for col in row.iter_mut().take(num_n) {
            *col = xpow;
            xpow = xpow.mul(x);
        }
        // E coefficients (unknown, k = 0..e-1): − y·x^k.
        let mut xpow = Fp::one();
        for k in 0..num_e {
            row[num_n + k] = y.mul(xpow).neg();
            xpow = xpow.mul(x);
        }
        // RHS: the fixed monic top E_e = 1 contributes y·x^e to N(x_i)=y·E(x_i),
        // i.e. moves to the RHS as + y·x^e.
        // sq-7ltf: `e` is the PUBLIC error-correction parameter and `x`
        // a PUBLIC evaluation point, so the variable-time exponentiation is sound
        // here (no secret in the exponent or base).
        row[cols] = y.mul(x.pow_vartime(e as u64));
        aug.push(row);
    }

    let sol = solve_linear_system(aug, cols)?;
    let n_coeffs = &sol[..num_n]; // N_0..N_{degree+e}
                                  // Rebuild E with the fixed monic leading coefficient.
    let mut e_coeffs: Vec<Fp> = sol[num_n..].to_vec(); // E_0..E_{e-1}
    e_coeffs.push(Fp::one()); // E_e = 1

    // Q = N / E exactly (remainder must vanish, else this (E,N) is spurious).
    let (q, rem) = poly_divmod(n_coeffs, &e_coeffs)?;
    if !rem.iter().all(|c| *c == Fp::zero()) {
        return None;
    }
    // Q must have degree <= `degree`. Trim and length-check.
    let q = trim_trailing_zeros(q);
    if q.len() > degree + 1 {
        return None;
    }
    let mut q = q;
    q.resize(degree + 1, Fp::zero());
    Some(q)
}

/// Solve a (possibly over-determined) linear system `A·z = b` over `F_p` for
/// exactly `cols` unknowns, given an augmented matrix with `>= cols` rows
/// (last entry of each row is the RHS). Returns the unique solution, or `None`
/// if the system is inconsistent or rank-deficient (no unique solution). Plain
/// Gaussian elimination with partial (nonzero) pivoting; `O(rows · cols^2)`.
fn solve_linear_system(mut aug: Vec<Vec<Fp>>, cols: usize) -> Option<Vec<Fp>> {
    let rows = aug.len();
    let mut pivot_row = 0usize;
    let mut where_pivot = vec![usize::MAX; cols]; // pivot row for each column

    for col in 0..cols {
        if pivot_row >= rows {
            break;
        }
        // Find a row at/below pivot_row with a nonzero entry in this column.
        let sel = (pivot_row..rows).find(|&r| aug[r][col] != Fp::zero());
        let Some(sel) = sel else { continue };
        aug.swap(pivot_row, sel);

        // Normalize the pivot row so the pivot is 1.
        let inv = aug[pivot_row][col].inv();
        for entry in aug[pivot_row].iter_mut().skip(col) {
            *entry = entry.mul(inv);
        }
        // Eliminate this column from every other row. Snapshot the (now-borrowed-
        // immutably) pivot row so we can mutate the other rows in place.
        let pivot_snapshot = aug[pivot_row].clone();
        for (r, row) in aug.iter_mut().enumerate() {
            if r != pivot_row && row[col] != Fp::zero() {
                let factor = row[col];
                for (entry, &p) in row.iter_mut().zip(pivot_snapshot.iter()).skip(col) {
                    *entry = entry.sub(factor.mul(p));
                }
            }
        }
        where_pivot[col] = pivot_row;
        pivot_row += 1;
    }

    // Every unknown must be pivoted (unique solution), else underdetermined.
    if where_pivot.contains(&usize::MAX) {
        return None;
    }
    // Consistency: any all-zero coefficient row must have zero RHS.
    for row in aug.iter() {
        if row[..cols].iter().all(|c| *c == Fp::zero()) && row[cols] != Fp::zero() {
            return None;
        }
    }
    let mut sol = vec![Fp::zero(); cols];
    for (col, &pr) in where_pivot.iter().enumerate() {
        sol[col] = aug[pr][cols];
    }
    Some(sol)
}

/// Polynomial division `num / den` over `F_p` (coeffs low→high). Returns
/// `(quotient, remainder)`. `None` if `den` is the zero polynomial.
fn poly_divmod(num: &[Fp], den: &[Fp]) -> Option<(Vec<Fp>, Vec<Fp>)> {
    let den = trim_trailing_zeros(den.to_vec());
    if den.is_empty() {
        return None; // division by zero polynomial
    }
    let mut rem = trim_trailing_zeros(num.to_vec());
    let den_deg = den.len() - 1;
    let den_lead_inv = den[den_deg].inv();

    if rem.len() < den.len() {
        return Some((vec![Fp::zero()], rem));
    }
    let mut quot = vec![Fp::zero(); rem.len() - den_deg];
    while rem.len() >= den.len() && !rem.is_empty() {
        let rem_deg = rem.len() - 1;
        let shift = rem_deg - den_deg;
        let coeff = rem[rem_deg].mul(den_lead_inv);
        quot[shift] = coeff;
        // rem -= coeff · x^shift · den.
        for (k, &d) in den.iter().enumerate() {
            let idx = shift + k;
            rem[idx] = rem[idx].sub(coeff.mul(d));
        }
        rem = trim_trailing_zeros(rem);
    }
    Some((trim_trailing_zeros(quot), rem))
}

/// Drop trailing zero (high-degree) coefficients; an all-zero poly becomes `[]`.
fn trim_trailing_zeros(mut v: Vec<Fp>) -> Vec<Fp> {
    while v.last() == Some(&Fp::zero()) {
        v.pop();
    }
    v
}

/// Evaluate a polynomial (coeffs low→high) at `x` by Horner's method.
fn eval_poly(coeffs: &[Fp], x: Fp) -> Fp {
    let mut acc = Fp::zero();
    for &c in coeffs.iter().rev() {
        acc = acc.mul(x).add(c);
    }
    acc
}

/// Plain Lagrange interpolation at `x = 0` (the secret), no consistency check.
/// Used on the no-redundancy path where detection is impossible.
fn lagrange_at_zero(shares: &[Share]) -> Result<Fp, MpcError> {
    Ok(eval_poly(&lagrange_at_zero_poly(shares)?, Fp::zero()))
}

/// Full Lagrange interpolation: recover the polynomial (coeffs low→high) through
/// the given points. Used both to evaluate at 0 and to verify other points.
fn lagrange_at_zero_poly(shares: &[Share]) -> Result<Vec<Fp>, MpcError> {
    if shares.is_empty() {
        return Err(MpcError::Protocol("interpolation needs >= 1 point".into()));
    }
    // Build P(x) = Σ_i y_i · Π_{j≠i} (x − x_j)/(x_i − x_j) by accumulating the
    // weighted basis polynomials. O(n^2) over small n.
    let n = shares.len();
    let mut result = vec![Fp::zero(); n];
    for (i, si) in shares.iter().enumerate() {
        let xi = Fp::new(si.x);
        // Numerator polynomial Π_{j≠i} (x − x_j) and the scalar denominator.
        let mut basis = vec![Fp::one()]; // start with the constant poly 1
        let mut den = Fp::one();
        for (j, sj) in shares.iter().enumerate() {
            if i == j {
                continue;
            }
            let xj = Fp::new(sj.x);
            basis = poly_mul_linear(&basis, xj.neg()); // multiply by (x − x_j)
            den = den.mul(xi.sub(xj));
        }
        let scale = si.y.mul(den.inv());
        for (k, b) in basis.iter().enumerate() {
            result[k] = result[k].add(b.mul(scale));
        }
    }
    Ok(result)
}

/// Multiply a polynomial (coeffs low→high) by the linear factor `(x + c)`.
fn poly_mul_linear(poly: &[Fp], c: Fp) -> Vec<Fp> {
    let mut out = vec![Fp::zero(); poly.len() + 1];
    for (k, &p) in poly.iter().enumerate() {
        out[k] = out[k].add(p.mul(c)); // p · c  → x^k term
        out[k + 1] = out[k + 1].add(p); // p · x  → x^{k+1} term
    }
    out
}

#[cfg(test)]
mod tests {
    /// `degree + 1` is the share count a degree-`degree` codeword needs, and
    /// `degree` is caller-supplied. At `usize::MAX` the addition used to wrap to 0,
    /// which made `n < 0` false and ACCEPTED a share set that cannot determine the
    /// polynomial. It is now a typed refusal.
    #[test]
    fn an_overflowing_degree_is_refused_rather_than_wrapped() {
        use super::*;
        let shares = vec![
            Share {
                x: 1,
                y: crate::field::Fp::new(1),
            },
            Share {
                x: 2,
                y: crate::field::Fp::new(2),
            },
        ];
        for degree in [usize::MAX, usize::MAX - 1] {
            let err = reconstruct_robust_attributed(&shares, degree)
                .expect_err("an overflowing degree must be refused");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("overflows usize")
                    || m.contains("needs >=")),
                "degree={}: {:?}",
                degree,
                err
            );
        }
        // And a degree one above the share count is still the ordinary refusal, so
        // the overflow arm has not swallowed the normal one.
        let err =
            reconstruct_robust_attributed(&shares, 2).expect_err("too few shares must be refused");
        assert!(
            matches!(&err, MpcError::Protocol(m) if m.contains("needs >=")),
            "{:?}",
            err
        );
    }

    use super::*;
    use crate::shamir::ShamirBackend;

    fn fp(v: u64) -> Fp {
        Fp::new(v)
    }

    fn corrupt(shares: &mut [Share], idx: usize, delta: u64) {
        shares[idx].y = shares[idx].y.add(fp(delta));
    }

    #[test]
    fn untampered_full_set_reconstructs() {
        // Genuine differential vs the UNCHECKED Lagrange primitive on
        // clean inputs: no regression. We must NOT compare against
        // `ShamirBackend::reconstruct`, which now routes through
        // `reconstruct_robust` itself (robust-vs-robust is vacuous). Compare the
        // robust path against `reconstruct_at_zero` (the plain Lagrange-at-0
        // building block, `pub(crate)` for exactly this suite) so the assertion
        // genuinely pins robust == honest-Lagrange with no errors present.
        let b = ShamirBackend::new_seeded(7, 0xC0FFEE).unwrap();
        let t = b.threshold();
        for secret in [0u64, 1, 42, 100_000, crate::field::P - 1] {
            let shares = b.dealer().share(fp(secret));
            let robust = reconstruct_robust(&shares, t).unwrap();
            let honest = crate::shamir::reconstruct_at_zero(&shares, t).unwrap();
            assert_eq!(
                robust, honest,
                "robust must match unchecked Lagrange on clean input"
            );
            assert_eq!(robust, fp(secret));
        }
    }

    #[test]
    fn detects_single_tamper_with_full_redundancy_when_e_zero_not_enough() {
        // n=4, t=1 → e_max = floor((4-1-1)/2) = 1: a single error is CORRECTABLE.
        // n=3, t=1 → e_max = 0: a single error is only DETECTABLE (abort).
        let b = ShamirBackend::new_seeded(3, 1).unwrap(); // n=3, t=1
        let t = b.threshold();
        let mut shares = b.dealer().share(fp(2024));
        // Honest robust reconstruct is fine.
        assert_eq!(reconstruct_robust(&shares, t).unwrap(), fp(2024));
        // Tamper one share: redundancy present (n=3 > t+1=2) but e_max=0 ⇒ abort.
        corrupt(&mut shares, 1, 5);
        let err = reconstruct_robust(&shares, t).unwrap_err();
        assert!(matches!(err, MpcError::Tampered { .. }), "got {err:?}");
    }

    #[test]
    fn corrects_single_tamper_when_budget_allows() {
        // n=4, t=1 → e_max = floor((4-1-1)/2) = 1: correct ONE error, return truth.
        let b = ShamirBackend::new_seeded(4, 7).unwrap();
        let t = b.threshold();
        assert_eq!(t, 1);
        let secret = fp(98_765);
        let mut shares = b.dealer().share(secret);
        corrupt(&mut shares, 2, 42); // one tampered share, within budget
        let got = reconstruct_robust(&shares, t).unwrap();
        assert_eq!(
            got, secret,
            "BW must correct one error and return the truth"
        );
    }

    #[test]
    fn corrects_two_tampers_at_n9() {
        // The backend fixes t = floor((n-1)/2), so for n=9, t=4 (NOT
        // t=2). With t=4 the BW correction budget is e_max = floor((n-t-1)/2) =
        // floor((9-4-1)/2) = 2: this test corrects exactly e_max = 2 errors and
        // then asserts that a 3rd error (over budget) aborts.
        let b = ShamirBackend::new_seeded(9, 0x9999).unwrap();
        let t = b.threshold();
        assert_eq!(t, 4); // floor((9-1)/2) = 4
                          // t=4 ⇒ e_max = floor((9-4-1)/2) = 2: correct up to TWO errors.
        let secret = fp(555_555);
        let mut shares = b.dealer().share(secret);
        corrupt(&mut shares, 0, 11);
        corrupt(&mut shares, 5, 22);
        let got = reconstruct_robust(&shares, t).unwrap();
        assert_eq!(got, secret, "BW must correct two errors at n=9,t=4");
        // A THIRD error exceeds e_max=2 ⇒ abort.
        let mut three = b.dealer().share(secret);
        corrupt(&mut three, 0, 11);
        corrupt(&mut three, 4, 22);
        corrupt(&mut three, 7, 33);
        let err = reconstruct_robust(&three, t).unwrap_err();
        assert!(matches!(err, MpcError::Tampered { .. }), "got {err:?}");
    }

    #[test]
    fn no_redundancy_does_not_claim_detection() {
        // Exactly t+1 shares: tampering selects a different valid secret, NOT a
        // detectable inconsistency. Must NOT return Tampered (no false positive).
        let b = ShamirBackend::new_seeded(5, 0xABCD).unwrap();
        let t = b.threshold(); // 2
        let secret = fp(123_456);
        let shares = b.dealer().share(secret);
        let mut min_set = shares[..t + 1].to_vec();
        corrupt(&mut min_set, 0, 9_999);
        let got = reconstruct_robust(&min_set, t).expect("t+1 points always interpolate");
        assert_ne!(got, secret, "t+1: tamper changes the secret (no detection)");
    }

    // ---------------------------------------------------------------------
    // sq-6u6b: cheater attribution.
    // ---------------------------------------------------------------------

    #[test]
    fn attributed_success_pins_exact_cheaters() {
        // (1) On the SUCCESS path the corrected shares are EXACTLY the tampered
        // ones — `cheaters` lists their evaluation points (Share::x), sorted.
        let b = ShamirBackend::new_seeded(9, 0x9999).unwrap();
        let t = b.threshold();
        assert_eq!(t, 4); // e_max = 2
        let secret = fp(555_555);
        let mut shares = b.dealer().share(secret);
        corrupt(&mut shares, 0, 11);
        corrupt(&mut shares, 5, 22);
        let r = reconstruct_robust_attributed(&shares, t).unwrap();
        assert_eq!(r.secret, secret, "still corrects to the true secret");
        let mut expected = vec![shares[0].x, shares[5].x];
        expected.sort_unstable();
        assert_eq!(
            r.cheaters, expected,
            "corrected shares must be exactly the tampered evaluation points"
        );
    }

    #[test]
    fn attributed_clean_input_names_no_cheaters() {
        // No tamper ⇒ empty cheater list on the success path.
        let b = ShamirBackend::new_seeded(7, 0xC0FFEE).unwrap();
        let t = b.threshold();
        let shares = b.dealer().share(fp(42));
        let r = reconstruct_robust_attributed(&shares, t).unwrap();
        assert_eq!(r.secret, fp(42));
        assert!(r.cheaters.is_empty(), "clean input has no cheaters");
    }

    #[test]
    fn attributed_no_redundancy_empty_cheaters() {
        // At exactly t+1 shares attribution is impossible; cheaters is empty and
        // we never claim Tampered (matches reconstruct_robust behaviour).
        let b = ShamirBackend::new_seeded(5, 0xABCD).unwrap();
        let t = b.threshold();
        let shares = b.dealer().share(fp(123_456));
        let mut min_set = shares[..t + 1].to_vec();
        corrupt(&mut min_set, 0, 9_999);
        let r = reconstruct_robust_attributed(&min_set, t).unwrap();
        assert!(r.cheaters.is_empty());
    }

    #[test]
    fn abort_first_band_names_only_real_cheaters() {
        // (2) The improvement that is genuinely demonstrable: in the FIRST abort
        // band — exactly `e_max + 1` tampered shares (one over the correctable
        // budget) — a fully-honest `degree+1`-reference subset is the UNIQUE
        // minimum-disagreement fit (it disagrees with only the `e_max+1` cheaters;
        // any cheater-containing subset recovers a spurious poly that misses many
        // more points). So the min-disagreement heuristic names ONLY genuinely-
        // tampered points: no honest party is framed. This is the regime the old
        // fixed-first-subset attribution could get wrong (its arbitrary window may
        // contain a cheater). Beyond this band attribution is provably impossible
        // (documented best-effort), so we do NOT assert soundness there.
        let mut rng = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for &(n, degree) in &[(5usize, 1usize), (6, 1), (7, 1), (7, 2), (9, 1), (9, 2)] {
            let e_max = (n - degree - 1) / 2;
            let num_err = e_max + 1; // first abort band
            for _ in 0..300 {
                let secret = fp(next() % crate::field::P);
                let pts: Vec<u64> = (1..=n as u64).collect();
                let mut shares = deal_at(&mut next, secret, degree, &pts);
                // Tamper `num_err` distinct shares.
                let mut idxs: Vec<usize> = (0..n).collect();
                for i in (1..n).rev() {
                    let j = next() as usize % (i + 1);
                    idxs.swap(i, j);
                }
                let mut tampered_xs = Vec::new();
                for &i in &idxs[..num_err] {
                    let mut d = next() % crate::field::P;
                    if d == 0 {
                        d = 1;
                    }
                    shares[i].y = shares[i].y.add(fp(d));
                    tampered_xs.push(shares[i].x);
                }
                tampered_xs.sort_unstable();
                let err = reconstruct_robust(&shares, degree).unwrap_err();
                let MpcError::Tampered { cheaters, .. } = err else {
                    panic!("n={n} degree={degree}: expected Tampered, got {err:?}");
                };
                assert!(!cheaters.is_empty(), "must name at least one suspect");
                for x in &cheaters {
                    assert!(
                        tampered_xs.contains(x),
                        "n={n} degree={degree} num_err={num_err}: framed honest x={x} \
                         (tampered {tampered_xs:?}, named {cheaters:?})"
                    );
                }
            }
        }
    }

    /// Deal a degree-`degree` Shamir sharing of `secret` at the given evaluation
    /// points, with random non-constant coefficients drawn from `next`.
    fn deal_at(
        next: &mut impl FnMut() -> u64,
        secret: Fp,
        degree: usize,
        pts: &[u64],
    ) -> Vec<Share> {
        let mut coeffs = vec![secret];
        for _ in 0..degree {
            coeffs.push(fp(next() % crate::field::P));
        }
        pts.iter()
            .map(|&x| Share {
                x,
                y: eval_poly(&coeffs, Fp::new(x)),
            })
            .collect()
    }

    // =====================================================================
    // sq-qcnn.26 — DIRECT unit tests on the private polynomial /
    // linear-algebra helpers of the robust (Berlekamp–Welch) reconstruction
    // path. Each asserts EXACT VALUES (not just "no panic") so the mutation
    // ratchet's semantic mutant classes — arithmetic swaps (`+`/`-`/`*`/`/`),
    // comparison flips (`<`/`==`/`>=`), `&&`/`||` guard flips, and default-body
    // replacements — are NOTICED in the helper that actually computes the
    // reconstruction, not only end-to-end where a wrong intermediate can be
    // masked. Semi-honest scope only; NO protocol logic changed, NO security
    // claim (sq-qhy4 gates that).
    // =====================================================================

    #[test]
    fn eval_poly_horner_exact_values() {
        // Q(x) = 6 + 5x + x^2 ; Q(2) = 6 + 10 + 4 = 20.
        assert_eq!(eval_poly(&[fp(6), fp(5), fp(1)], fp(2)), fp(20));
        // The empty polynomial is identically 0.
        assert_eq!(eval_poly(&[], fp(9)), fp(0));
        // A constant polynomial ignores x.
        assert_eq!(eval_poly(&[fp(3)], fp(100)), fp(3));
        // Q(x) = 2 + 3x^2 ; Q(2) = 2 + 12 = 14 (pins the x^2 coefficient path).
        assert_eq!(eval_poly(&[fp(2), fp(0), fp(3)], fp(2)), fp(14));
    }

    #[test]
    fn trim_trailing_zeros_drops_only_high_zeros() {
        assert_eq!(
            trim_trailing_zeros(vec![fp(1), fp(2), fp(0), fp(0)]),
            vec![fp(1), fp(2)]
        );
        // Interior zeros are preserved — only TRAILING zeros are dropped.
        assert_eq!(
            trim_trailing_zeros(vec![fp(1), fp(0), fp(3)]),
            vec![fp(1), fp(0), fp(3)]
        );
        // An all-zero polynomial collapses to the empty vec.
        assert_eq!(trim_trailing_zeros(vec![fp(0), fp(0)]), Vec::<Fp>::new());
        assert_eq!(trim_trailing_zeros(vec![fp(0)]), Vec::<Fp>::new());
        assert_eq!(trim_trailing_zeros(Vec::<Fp>::new()), Vec::<Fp>::new());
        // No trailing zero → unchanged.
        assert_eq!(trim_trailing_zeros(vec![fp(7)]), vec![fp(7)]);
    }

    #[test]
    fn poly_mul_linear_multiplies_by_x_plus_c() {
        // (1 + x)·(x + 2) = 2 + 3x + x^2.
        assert_eq!(
            poly_mul_linear(&[fp(1), fp(1)], fp(2)),
            vec![fp(2), fp(3), fp(1)]
        );
        // 3·(x + 5) = 15 + 3x (the constant-poly case; pins BOTH the `p·c` and
        // the `p·x` shift terms).
        assert_eq!(poly_mul_linear(&[fp(3)], fp(5)), vec![fp(15), fp(3)]);
    }

    #[test]
    fn poly_divmod_exact_quotient_and_remainder() {
        // (x+2)(x+3) = x^2 + 5x + 6, divided by (x+2), is exactly (x+3) rem 0.
        let (q, r) = poly_divmod(&[fp(6), fp(5), fp(1)], &[fp(2), fp(1)]).unwrap();
        assert_eq!(q, vec![fp(3), fp(1)], "quotient must be x+3");
        assert_eq!(
            r,
            Vec::<Fp>::new(),
            "remainder must vanish (exact division)"
        );
    }

    #[test]
    fn poly_divmod_nonzero_remainder_and_low_degree_dividend() {
        // x^2 + 1 divided by (x+1): quotient x-1 = x + (p-1), remainder 2.
        // (x+1)(x-1) = x^2 - 1, so x^2 + 1 = (x+1)(x-1) + 2.
        let (q, r) = poly_divmod(&[fp(1), fp(0), fp(1)], &[fp(1), fp(1)]).unwrap();
        assert_eq!(q, vec![fp(1).neg(), fp(1)], "quotient must be x - 1");
        assert_eq!(r, vec![fp(2)], "remainder must be the constant 2");

        // Dividend of LOWER degree than the divisor → the early-return arm:
        // quotient is the zero polynomial [0] and the whole dividend is the
        // remainder. (Pins the `rem.len() < den.len()` comparison + return.)
        let (q2, r2) = poly_divmod(&[fp(5)], &[fp(2), fp(1)]).unwrap();
        assert_eq!(q2, vec![fp(0)]);
        assert_eq!(r2, vec![fp(5)]);
    }

    #[test]
    fn poly_divmod_by_zero_polynomial_is_none() {
        assert!(poly_divmod(&[fp(1), fp(2)], &[fp(0)]).is_none());
        assert!(poly_divmod(&[fp(1), fp(2)], &[]).is_none());
    }

    #[test]
    fn lagrange_recovers_polynomial_and_secret() {
        // Q(x) = 10 + 4x + 7x^2. Sample at x = 1,2,3 and interpolate back.
        let q = [fp(10), fp(4), fp(7)];
        let shares: Vec<Share> = [1u64, 2, 3]
            .iter()
            .map(|&x| Share {
                x,
                y: eval_poly(&q, fp(x)),
            })
            .collect();
        // The full polynomial is recovered coefficient-for-coefficient.
        assert_eq!(lagrange_at_zero_poly(&shares).unwrap(), q.to_vec());
        // The secret (value at x = 0) is the constant term.
        assert_eq!(lagrange_at_zero(&shares).unwrap(), fp(10));
        // No points → a protocol error, never a bogus zero.
        assert!(matches!(
            lagrange_at_zero_poly(&[]),
            Err(MpcError::Protocol(_))
        ));
    }

    #[test]
    fn solve_linear_system_unique_inconsistent_and_underdetermined() {
        // Unique: 2x + 3y = 8 ; x + 2y = 5  ⇒  x = 1, y = 2.
        let unique = vec![vec![fp(2), fp(3), fp(8)], vec![fp(1), fp(2), fp(5)]];
        assert_eq!(solve_linear_system(unique, 2), Some(vec![fp(1), fp(2)]));

        // Inconsistent: x = 1 and x = 2 has no solution.
        let inconsistent = vec![vec![fp(1), fp(1)], vec![fp(1), fp(2)]];
        assert_eq!(solve_linear_system(inconsistent, 1), None);

        // Under-determined: one equation, two unknowns ⇒ no UNIQUE solution.
        let under = vec![vec![fp(1), fp(0), fp(3)]];
        assert_eq!(solve_linear_system(under, 2), None);
    }

    #[test]
    fn for_each_combination_enumerates_exact_subsets() {
        let mut got: Vec<Vec<usize>> = Vec::new();
        for_each_combination(4, 2, &mut |idx| got.push(idx.to_vec()));
        assert_eq!(
            got,
            vec![
                vec![0, 1],
                vec![0, 2],
                vec![0, 3],
                vec![1, 2],
                vec![1, 3],
                vec![2, 3],
            ],
            "C(4,2) in lexicographic order"
        );

        // k == 0 yields the single empty combination.
        let mut zero: Vec<Vec<usize>> = Vec::new();
        for_each_combination(3, 0, &mut |idx| zero.push(idx.to_vec()));
        assert_eq!(zero, vec![Vec::<usize>::new()]);

        // k > n yields NO combination.
        let mut none: Vec<Vec<usize>> = Vec::new();
        for_each_combination(2, 3, &mut |idx| none.push(idx.to_vec()));
        assert!(none.is_empty());

        // k == n yields exactly the one full combination.
        let mut full: Vec<Vec<usize>> = Vec::new();
        for_each_combination(3, 3, &mut |idx| full.push(idx.to_vec()));
        assert_eq!(full, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn check_distinct_points_rejects_repeats() {
        let ok = [Share { x: 1, y: fp(1) }, Share { x: 2, y: fp(2) }];
        assert!(check_distinct_points(&ok).is_ok());
        let dup = [
            Share { x: 1, y: fp(1) },
            Share { x: 2, y: fp(2) },
            Share { x: 2, y: fp(9) },
        ];
        assert!(matches!(
            check_distinct_points(&dup),
            Err(MpcError::Protocol(_))
        ));
    }

    #[test]
    fn disagreeing_points_flags_exactly_the_off_curve_shares() {
        // Q(x) = 10 + 4x + 7x^2 sampled at x = 1..4; then tamper index 1.
        let q = [fp(10), fp(4), fp(7)];
        let mut shares: Vec<Share> = [1u64, 2, 3, 4]
            .iter()
            .map(|&x| Share {
                x,
                y: eval_poly(&q, fp(x)),
            })
            .collect();
        assert!(disagreeing_points(&shares, &q).is_empty(), "clean → none");
        shares[1].y = shares[1].y.add(fp(1)); // off-curve by 1
        assert_eq!(
            disagreeing_points(&shares, &q),
            vec![1],
            "exactly the tampered index is off-curve"
        );
    }

    #[test]
    fn berlekamp_welch_corrects_one_error_directly() {
        // Q(x) = 5 + 3x sampled at x = 1..4, one share tampered; e = 1 recovers Q.
        let q = [fp(5), fp(3)];
        let mut shares: Vec<Share> = [1u64, 2, 3, 4]
            .iter()
            .map(|&x| Share {
                x,
                y: eval_poly(&q, fp(x)),
            })
            .collect();
        shares[2].y = shares[2].y.add(fp(99)); // one off-curve share
        let recovered = berlekamp_welch(&shares, 1, 1).expect("e=1 must decode");
        assert_eq!(recovered, vec![fp(5), fp(3)], "BW recovers Q = 5 + 3x");
        assert_eq!(eval_poly(&recovered, fp(0)), fp(5), "secret Q(0) = 5");
        // e = 0 cannot absorb the tampered share (the points are not all on one
        // degree-1 line) → the key-equation system is inconsistent → None.
        assert!(berlekamp_welch(&shares, 1, 0).is_none());
    }
}
