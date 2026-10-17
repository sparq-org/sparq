// [OPUS-4.8] sq-sxm — the (security model × N × query class) benchmark MATRIX,
// in-process counting tier (Tier 1). New module: the harness that drives the
// representative MPC operations across N and the security models and emits, per
// cell, the modelled communication (bytes/party, rounds, multiplications) PLUS
// the per-cell security guarantee read off the real backend's per-operator
// descriptor — never a requested model, the ACTUAL one (design record §5.1).
//
// This is the in-process COUNTING tier. The real network transport + tc/netem
// LAN/WAN emulation is a SEPARATE bead (sq-tg6b, Tier 2/3); the EC2 scale-out is
// sq-hoaj. Per the critical-honesty constraint (design record §5): a single-
// process wall-clock is NOT an MPC latency — the load-bearing metric here is the
// DETERMINISTIC modelled communication ([`crate::metrics::CommCounter`]), and the
// emitted schema is labelled accordingly.
//! The (security model × N × query class) MPC benchmark matrix — in-process
//! counting tier.
//!
//! ## What this measures (and what it deliberately does not)
//!
//! The crate is a single-process multi-party SIMULATION, so wall-clock is not a
//! meaningful cross-`N` MPC cost (design record §5). The harness instead emits the
//! **deterministic modelled communication** per matrix cell:
//! `bytes_per_party`, `round_count` (sequential upper bound + batched lower
//! bound), and `multiplications` — counted analytically from the protocol
//! structure via [`crate::metrics::CommCounter`], reproducible run-to-run.
//!
//! Every cost cell co-runs the REAL primitive (the cumulative-sum aggregate, the
//! hidden-value equality join, the oblivious shuffle/sort) so a fast-but-wrong
//! cell is caught — correctness gates cost (design record §5.4). The cost itself
//! is the modelled count, NOT the simulation's wall-clock.
//!
//! ## The matrix axes (design record §5.1)
//!
//! - **AXIS 1 — security model:** the ACTUAL guarantee reachable at this `(n, t)`
//!   for this operator class, read from
//!   [`crate::shamir::ShamirBackend::operator_descriptor`] — never a requested
//!   one. The degree-`t` aggregate carries RS redundancy (robust for most `n`);
//!   the degree-`2t` equality/join open is **semi-honest-only at `n = 2t+1`** (odd
//!   `n`, zero redundancy). So the security axis is a *function of `(n, primitive,
//!   t)`*, recorded per cell — see [`CellSecurity`].
//! - **AXIS 2 — number of parties `N`:** `{2, 3, 5, 7}` (the bead's set), with
//!   `t = ⌊(n−1)/2⌋`. Even `N` (here 2) exercises the abort-vs-robust boundary at
//!   degree `2t`; odd `N` the semi-honest-only equality boundary.
//! - **AXIS 3 — query class:** the representative operations —
//!   [`QueryClass::CumulativeSumAggregate`] (the £100k case — zero-round linear
//!   baseline), [`QueryClass::HiddenValueEquiJoin`] (the cost center —
//!   `|L|·|R|` equalities), and the oblivious
//!   [`QueryClass::ObliviousShuffle`] / [`QueryClass::ObliviousSort`] substrate.
//!
//! ## Output schema
//!
//! [`MatrixResults`] is the structured result the harness emits (one
//! [`MatrixCell`] per axis combination). It serialises to a stable JSON shape
//! (hand-written, no `serde` dependency — the crate stays dependency-minimal) via
//! [`MatrixResults::to_json`], so downstream tooling consumes the generated data
//! rather than any hard-coded figure. The example binary
//! (`examples/mpc_bench_matrix.rs`) prints both the JSON and a human table.

use crate::backend::{MaliciousSecurity, MpcBackend, OperatorClass, PreprocessingCost, TrustModel};
use crate::field::Fp;
use crate::join::{HiddenKeyedRows, HiddenValueJoin};
use crate::metrics::CommCounter;
use crate::oblivious::{shuffle, sort_with_keys, SecretColumn};
use crate::partial::{HolderId, MpcError};
use crate::shamir::{ShamirBackend, ShamirDealer};

/// The representative SPARQL/MPC operation a matrix row benchmarks (AXIS 3).
/// Each maps to a real primitive in the crate so correctness co-gates the cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryClass {
    /// **Cumulative-SUM aggregate** (the £100k case): each holder secret-shares
    /// one private integer; the secure sum is a FREE local share-addition
    /// (zero-round linear), then ONE open. The zero-multiplication baseline.
    /// Operator class [`OperatorClass::LinearAggregate`] (degree-`t` open).
    CumulativeSumAggregate,
    /// **Hidden-value equi-join** on a PRIVATE key: the all-pairs `|L|·|R|`
    /// secret-shared equality tests — the cost center (`|L|·|R|` opens + mults).
    /// Operator class [`OperatorClass::EqualityJoin`] (degree-`2t` open).
    HiddenValueEquiJoin,
    /// **Oblivious shuffle** (AS-Waksman network): the keystone hidden-regime
    /// primitive; 1 multiplication per switch (secret control bits) in
    /// `O(log n)` depth. No degree-`2t` open ⇒ reported under the shuffle's own
    /// soundness, not the equality class.
    ObliviousShuffle,
    /// **Oblivious sort** (Batcher odd-even mergesort): the obliviousness
    /// substrate; `O(n log² n)` compare-exchanges in `O(log² n)` depth.
    ObliviousSort,
    /// **Hidden bounded property path** (`sq-py8h`): the secret-graph `(p){m,k}`
    /// operator. Its cost is the `k×`-join secure equalities + the `O(k)` AND/OR
    /// degree-reduction rounds that chain the per-tuple hop bits + the `B`-slot
    /// oblivious output — modelled by
    /// [`crate::hidden_path::planner::BoundedPathPlan::comm_counter`] (sq-py8h.5).
    /// Operator class [`OperatorClass::EqualityJoin`] (the secret-shared hop equality
    /// opens at degree `2t`). `[OPUS-4.8]`
    HiddenBoundedPath,
}

impl QueryClass {
    /// Stable identifier for the JSON schema / table.
    pub fn id(self) -> &'static str {
        match self {
            QueryClass::CumulativeSumAggregate => "cumulative_sum_aggregate",
            QueryClass::HiddenValueEquiJoin => "hidden_value_equi_join",
            QueryClass::ObliviousShuffle => "oblivious_shuffle",
            QueryClass::ObliviousSort => "oblivious_sort",
            QueryClass::HiddenBoundedPath => "hidden_bounded_path",
        }
    }

    /// The [`OperatorClass`] whose per-operator security descriptor governs this
    /// query class's reconstruction guarantee. Shuffle/sort do not perform a
    /// degree-`2t` equality open, but their conditional-swap / comparator products
    /// are degree-raising like the equality join, so they share its operator class
    /// for the security-axis read-off (the honest reduction the design record
    /// makes: the secret-control swap and the secure comparator are the same
    /// degree-`2t` regime as the equality test).
    pub fn operator_class(self) -> OperatorClass {
        match self {
            QueryClass::CumulativeSumAggregate => OperatorClass::LinearAggregate,
            QueryClass::HiddenValueEquiJoin
            | QueryClass::ObliviousShuffle
            | QueryClass::ObliviousSort
            | QueryClass::HiddenBoundedPath => OperatorClass::EqualityJoin,
        }
    }
}

/// The per-cell security guarantee, read from the REAL backend's per-operator
/// descriptor at this `(n, t)` — the ACTUAL guarantee, never a requested one
/// (design record §5.1). This is what makes the "security model" axis honest:
/// e.g. the degree-`2t` equality open is [`MaliciousSecurity::SemiHonestOnly`] at
/// `n = 2t+1` even though the degree-`t` aggregate is robust at the same `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSecurity {
    /// The trust regime (honest- vs dishonest-majority) at this `(n, t)`.
    pub trust_model: TrustModel,
    /// The malicious-security guarantee for THIS operator at this `(n, t)`. The
    /// load-bearing per-cell honesty field.
    pub malicious_security: MaliciousSecurity,
}

impl CellSecurity {
    /// Read the actual guarantee for `operator` off `backend` at its `(n, t)`.
    pub fn read(backend: &ShamirBackend, operator: OperatorClass) -> Self {
        let desc = backend.operator_descriptor(operator);
        // The RS correction budget at this operator's reconstruction degree is what
        // distinguishes robust-{e} from abort from semi-honest-only; recover it the
        // same way `BackendInfo::new` does so the projection is faithful.
        let e = match operator {
            OperatorClass::LinearAggregate => rs_budget(backend.parties(), backend.threshold()),
            OperatorClass::EqualityJoin => rs_budget(backend.parties(), 2 * backend.threshold()),
            OperatorClass::Comparison => 0,
        };
        CellSecurity {
            trust_model: desc.trust_model(),
            malicious_security: desc.malicious_security(e),
        }
    }

    /// Stable string for the JSON schema / table.
    pub fn label(self) -> String {
        let trust = match self.trust_model {
            TrustModel::HonestMajority => "honest-majority",
            TrustModel::DishonestMajority => "dishonest-majority",
        };
        let mal = match self.malicious_security {
            MaliciousSecurity::SemiHonestOnly => "semi-honest-only".to_string(),
            MaliciousSecurity::HonestMajorityAbort => "abort".to_string(),
            MaliciousSecurity::HonestMajorityRobust { max_cheaters } => {
                format!("robust(e={max_cheaters})")
            }
        };
        format!("{trust}/{mal}")
    }
}

/// RS/Berlekamp–Welch correction budget `e = ⌊(n − degree − 1)/2⌋` at a given
/// reconstruction `degree`, or `0` when there is no redundancy (`n <= degree+1`).
/// Mirrors `ShamirBackend::rs_correction_budget` (which is private), kept local so
/// the harness does not reach into the backend's internals. `[OPUS-4.8]`
fn rs_budget(n: usize, degree: usize) -> usize {
    if n <= degree + 1 {
        0
    } else {
        (n - degree - 1) / 2
    }
}

/// One matrix cell: (query class × N × actual security) with its modelled cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixCell {
    /// AXIS 3.
    pub query_class: QueryClass,
    /// AXIS 2: number of compute parties.
    pub parties: usize,
    /// `t = ⌊(n−1)/2⌋`.
    pub threshold: usize,
    /// The data scale this cell ran (rows/holder for joins; party count for the
    /// aggregate, where `N` itself is the scale). `1` for the aggregate.
    pub scale: usize,
    /// AXIS 1: the ACTUAL per-operator security at this `(n, t)`.
    pub security: CellSecurity,
    /// The deterministic modelled communication for this cell.
    pub comm: CommCounter,
    /// Backend-declared offline cost, evaluated against this cell's modeled operation counts.
    pub declared_preprocessing: PreprocessingCost,
}

impl MatrixCell {
    /// Modelled bytes per party (the cross-`N`-comparable on-wire cost).
    pub fn bytes_per_party(self) -> u64 {
        self.comm.bytes_per_party()
    }
    /// Sequential round count (upper bound).
    pub fn rounds_sequential(self) -> u64 {
        self.comm.total_rounds()
    }
    /// Batched round count (lower bound, independent ops collapsed).
    pub fn rounds_batched(self) -> u64 {
        self.comm.batched_rounds()
    }
    /// Secure multiplications.
    pub fn multiplications(self) -> u64 {
        self.comm.multiplications
    }

    /// Modeled ordering comparisons; equality tests already count as multiplications.
    pub fn comparisons(self) -> u64 {
        match self.query_class {
            // The sort counter records one multiplication per compare-exchange.
            QueryClass::ObliviousSort => self.comm.multiplications,
            _ => 0,
        }
    }
}

/// The full matrix result: every cell, plus the axis definitions. Serialises to a
/// stable JSON shape (no `serde`; hand-written) so downstream tooling reads the
/// GENERATED numbers — never a hard-coded figure (AGENTS.md: no baked-in perf).
#[derive(Debug, Clone)]
pub struct MatrixResults {
    /// The party counts (AXIS 2) the matrix swept.
    pub party_counts: Vec<usize>,
    /// One cell per (query class × N × scale) combination.
    pub cells: Vec<MatrixCell>,
}

impl MatrixResults {
    /// Serialise the matrix to a stable, dependency-free JSON string. The schema:
    /// ```json
    /// {
    ///   "tier": "in-process-counting",
    ///   "note": "single-process simulation; network cost MODELLED not measured",
    ///   "field_bytes": 8,
    ///   "party_counts": [2,3,5,7],
    ///   "cells": [
    ///     {"query_class": "...", "parties": N, "threshold": t, "scale": s,
    ///      "security": "honest-majority/semi-honest-only",
    ///      "trust_model": "...", "malicious_security": "...",
    ///      "bytes_per_party": B, "total_bytes": TB,
    ///      "rounds_sequential": R, "rounds_batched": Rb,
    ///      "multiplications": M, "field_elements_opened": O,
    ///      "field_elements_shared": S, "offline_bytes": OB,
    ///      "declared_preprocessing": {"basis": "declared-model",
    ///        "items": I, "bytes_per_party": PB}}
    ///   ]
    /// }
    /// ```
    /// `declared_preprocessing` estimates the backend's declared offline budget
    /// using the cell's modeled multiplications and ordering comparisons. It is
    /// not a measurement. No preprocessing gives explicit zeros; unknown byte
    /// costs or overflowing estimates give JSON `null`. `offline_bytes` is the
    /// legacy total-byte projection (declared bytes per party times parties),
    /// also `null` when unknown or overflowing.
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        s.push_str("{\n");
        s.push_str("  \"tier\": \"in-process-counting\",\n");
        s.push_str(
            "  \"note\": \"single-process simulation; network cost MODELLED not measured\",\n",
        );
        s.push_str(&format!(
            "  \"field_bytes\": {},\n",
            crate::metrics::FIELD_BYTES
        ));
        s.push_str("  \"party_counts\": [");
        for (i, n) in self.party_counts.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&n.to_string());
        }
        s.push_str("],\n");
        s.push_str("  \"cells\": [\n");
        for (i, c) in self.cells.iter().enumerate() {
            let comma = if i + 1 < self.cells.len() { "," } else { "" };
            let items = c
                .declared_preprocessing
                .estimate_items(c.multiplications(), c.comparisons());
            let bytes_per_party = c
                .declared_preprocessing
                .estimate_bytes_per_party(c.multiplications(), c.comparisons());
            let offline_bytes = bytes_per_party.and_then(|b| b.checked_mul(c.parties as u64));
            s.push_str(&format!(
                "    {{\"query_class\": \"{}\", \"parties\": {}, \"threshold\": {}, \
                 \"scale\": {}, \"security\": \"{}\", \"trust_model\": \"{}\", \
                 \"malicious_security\": \"{}\", \"bytes_per_party\": {}, \
                 \"total_bytes\": {}, \"rounds_sequential\": {}, \"rounds_batched\": {}, \
                 \"multiplications\": {}, \"field_elements_opened\": {}, \
                 \"field_elements_shared\": {}, \"offline_bytes\": {}, \
                 \"declared_preprocessing\": {{\"basis\": \"declared-model\", \
                 \"items\": {}, \"bytes_per_party\": {}}}}}{}\n",
                c.query_class.id(),
                c.parties,
                c.threshold,
                c.scale,
                c.security.label(),
                trust_id(c.security.trust_model),
                mal_id(c.security.malicious_security),
                c.bytes_per_party(),
                c.comm.total_bytes(),
                c.rounds_sequential(),
                c.rounds_batched(),
                c.multiplications(),
                c.comm.field_elements_opened,
                c.comm.field_elements_shared,
                json_estimate(offline_bytes),
                json_estimate(items),
                json_estimate(bytes_per_party),
                comma,
            ));
        }
        s.push_str("  ]\n}\n");
        s
    }

    /// A compact human-readable table for the example binary's stdout.
    pub fn to_table(&self) -> String {
        let mut s = String::new();
        s.push_str(
            "TIER 1 (in-process counting) — single-process simulation; \
             network cost MODELLED not measured.\n\n",
        );
        s.push_str(&format!(
            "{:<26} {:>3} {:>3} {:>7} {:>10} {:>9} {:>9} {:>7}  {}\n",
            "query_class",
            "N",
            "t",
            "scale",
            "B/party",
            "rounds",
            "rnd(bat)",
            "mults",
            "security (actual, per-operator)"
        ));
        s.push_str(&"-".repeat(110));
        s.push('\n');
        for c in &self.cells {
            s.push_str(&format!(
                "{:<26} {:>3} {:>3} {:>7} {:>10} {:>9} {:>9} {:>7}  {}\n",
                c.query_class.id(),
                c.parties,
                c.threshold,
                c.scale,
                c.bytes_per_party(),
                c.rounds_sequential(),
                c.rounds_batched(),
                c.multiplications(),
                c.security.label(),
            ));
        }
        s
    }
}

fn json_estimate(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_string(), |v| v.to_string())
}

fn trust_id(t: TrustModel) -> &'static str {
    match t {
        TrustModel::HonestMajority => "honest-majority",
        TrustModel::DishonestMajority => "dishonest-majority",
    }
}

fn mal_id(m: MaliciousSecurity) -> &'static str {
    match m {
        MaliciousSecurity::SemiHonestOnly => "semi-honest-only",
        MaliciousSecurity::HonestMajorityAbort => "honest-majority-abort",
        MaliciousSecurity::HonestMajorityRobust { .. } => "honest-majority-robust",
    }
}

// =============================================================================
// The cost MODEL per query class (analytic, derived from the protocol structure)
// =============================================================================

/// Model the cumulative-SUM aggregate over `n` holders: each holder deals one
/// private integer (`n` deals); the secure sum is a free LOCAL share-addition
/// (zero multiplications, zero rounds); then ONE open of the result. This is the
/// zero-round linear baseline — `O(n)` shared field elements, `0` mults, `1` open.
fn model_aggregate(n: usize) -> CommCounter {
    let mut c = CommCounter::new(n);
    for _ in 0..n {
        c.record_deal();
    }
    c.record_open();
    c
}

/// Model the hidden-value all-pairs equi-join over `|L| = |R| = scale` rows/holder:
/// `scale²` mutually-independent secret-shared equality tests, each = 3 deals
/// (key_L, key_R, mask) + 1 multiplication + 1 open. This is the cost center: the
/// opens / mults grow as the PRODUCT of the row counts (the round-count explosion
/// the literature flags).
/// `scale²` is a product of a caller-chosen number with itself, so it is counted
/// with checked arithmetic and capped: an over-cap or overflowing scale is a
/// controlled refusal, not a debug-build overflow panic on a public entry.
fn model_hidden_join(n: usize, scale: usize) -> Result<CommCounter, MpcError> {
    let pairs = crate::oblivious_join::checked_candidate_pairs(scale, scale)?;
    let mut c = CommCounter::new(n);
    c.record_independent_equalities(pairs as u64);
    Ok(c)
}

/// Model an oblivious shuffle of `scale` rows: fold in the crate's own
/// [`crate::oblivious::ShuffleCost`] (1 mult/switch, `O(log n)` depth) — no
/// degree-`2t` open. We also deal the `scale` column values once (they enter
/// secret-shared).
fn model_shuffle(n: usize, scale: usize) -> CommCounter {
    let mut c = CommCounter::new(n);
    for _ in 0..scale {
        c.record_deal();
    }
    let net = crate::oblivious::WaksmanNetwork::new(scale);
    c.record_shuffle(crate::oblivious::ShuffleCost::model(
        scale,
        net.switch_count(),
    ));
    c
}

/// Model an oblivious sort of `scale` rows: fold in the crate's own
/// [`crate::oblivious::SortCost`] (one secure comparison + conditional swap per
/// compare-exchange, `O(log² n)` depth).
fn model_sort(n: usize, scale: usize) -> CommCounter {
    let mut c = CommCounter::new(n);
    for _ in 0..scale {
        c.record_deal();
    }
    let net = crate::oblivious::SortingNetwork::new(scale);
    c.record_sort(crate::oblivious::SortCost {
        items: scale,
        compare_exchanges: net.gate_count(),
        depth_rounds: net.depth(),
    });
    c
}

/// Model the hidden bounded property path `(p){1,k}` over `scale` secret edges with
/// a public padded output bound `B = scale`. Routes through the operator's OWN
/// statically-projected cost descriptor
/// ([`crate::hidden_path::planner::BoundedPathPlan::comm_counter`], sq-py8h.5) so the
/// matrix cell reports the SAME modelled communication the operator pays — the
/// `k×`-join secure equalities + the `O(k)` reduction rounds + the `B`-slot oblivious
/// output — never a fabricated wall-clock. `k` is the fixed public bound
/// [`BOUNDED_PATH_K`]; `B = scale` covers up to `scale` distinct endpoint pairs.
/// `[OPUS-4.8]`
fn model_bounded_path(n: usize, scale: usize) -> Result<CommCounter, MpcError> {
    use crate::hidden_path::planner::{plan_hidden_bounded_path, HiddenPathRequest};
    // `(p){1,k}` over a single predicate, projected against |E| = scale public edges.
    let req = HiddenPathRequest::range("p", 1, BOUNDED_PATH_K);
    let plan = plan_hidden_bounded_path(&req, scale)?;
    // B = scale (cover up to `scale` distinct endpoint pairs); at least 1 slot.
    plan.comm_counter(n, scale.max(1))
}

// =============================================================================
// The CORRECTNESS check per query class (runs the REAL primitive so a fast-but-
// wrong cost cell is caught). Needs a seeded backend (caller supplies it).
// =============================================================================

/// Run the REAL cumulative-sum aggregate over `n` holders' private values and
/// assert it reconstructs the plaintext sum — the correctness gate for the
/// aggregate cost cell (design record §5.4: differential correctness alongside
/// every cost cell). Returns the reconstructed sum.
pub fn check_aggregate(backend: &ShamirBackend, values: &[u64]) -> Result<u64, MpcError> {
    use crate::backend::MpcBackend;
    let mut dealer = backend.dealer();
    // Share each value, sum the sharings (free local add), open.
    let shares: Vec<Vec<crate::shamir::Share>> =
        values.iter().map(|&v| dealer.share(Fp::new(v))).collect();
    let summed = backend.run_secure(&shares)?;
    let result = backend.reconstruct_disclosed(&summed)?;
    // The disclosed partial carries the sum as an integer literal in row 0, col 0.
    let term = result.rows[0][0]
        .as_ref()
        .ok_or_else(|| MpcError::Protocol("aggregate result missing".into()))?;
    match term {
        oxrdf::Term::Literal(l) => l
            .value()
            .parse::<u64>()
            .map_err(|e| MpcError::Protocol(format!("aggregate not an integer: {e}"))),
        _ => Err(MpcError::Protocol("aggregate not a literal".into())),
    }
}

/// Run the REAL hidden-value join over two holders' `scale`-row tables (keys
/// `0..scale` on both sides, so exactly `scale` matches) and return the number of
/// joined rows — the correctness gate for the hidden-join cost cell.
pub fn check_hidden_join(backend: &ShamirBackend, scale: usize) -> Result<usize, MpcError> {
    let join = HiddenValueJoin::new(backend.clone());
    let left = synthetic_table(HolderId::new("left"), scale);
    let right = synthetic_table(HolderId::new("right"), scale);
    let out = join.join(&left, &right)?;
    Ok(out.rows.len())
}

/// Build a `scale`-row hidden-keyed table with keys `0..scale` and a single
/// disclosed payload column carrying the row index, for the join correctness gate.
fn synthetic_table(holder: HolderId, scale: usize) -> HiddenKeyedRows {
    use oxrdf::{Literal, Term, Variable};
    let payload_vars = vec![Variable::new_unchecked("v")];
    let rows = (0..scale)
        .map(|i| {
            let key = Fp::new(i as u64);
            let payload = vec![Some(Term::Literal(Literal::new_simple_literal(
                i.to_string(),
            )))];
            (key, payload)
        })
        .collect();
    HiddenKeyedRows {
        holder,
        payload_vars,
        rows,
    }
}

/// Run the REAL oblivious shuffle over `scale` secret-shared rows and assert it
/// preserves the multiset — the correctness gate for the shuffle cost cell.
pub fn check_shuffle(dealer: &mut ShamirDealer, t: usize, scale: usize) -> Result<bool, MpcError> {
    let values: Vec<Fp> = (0..scale).map(|i| Fp::new((i * 7 + 1) as u64)).collect();
    let col = secret_column(dealer, &values);
    let (shuffled, _cost) = shuffle(dealer, col)?;
    let opened = open_column(t, &shuffled);
    Ok(multiset_eq(&values, &opened))
}

/// Run the REAL oblivious sort over `scale` secret-shared rows and assert the
/// opened result is ascending and a permutation of the input — the correctness
/// gate for the sort cost cell.
pub fn check_sort(dealer: &mut ShamirDealer, t: usize, scale: usize) -> Result<bool, MpcError> {
    // Reverse-sorted input so the sort has work to do.
    let values: Vec<Fp> = (0..scale).map(|i| Fp::new((scale - i) as u64)).collect();
    let col = secret_column(dealer, &values);
    let keys: Vec<Fp> = values.clone();
    let (sorted, _keys_out, _ap, _cost) = sort_with_keys(col, keys)?;
    let opened = open_column(t, &sorted);
    let ascending = opened.windows(2).all(|w| w[0].value() <= w[1].value());
    Ok(ascending && multiset_eq(&values, &opened))
}

/// The fixed PUBLIC bound `k` the hidden-bounded-path cost cell models: `(p){1,k}`.
/// A small finite bound keeps the `Σ_ℓ |E|^ℓ` enumeration within the in-process
/// counting tier while still exercising the multi-length union + reduction chain.
/// `[OPUS-4.8]`
pub const BOUNDED_PATH_K: usize = 3;

/// Run the REAL hidden bounded property path `(p){1,k}` over a `scale`-edge linear
/// chain `n_0 →p n_1 →p … →p n_scale` and assert the disclosed endpoint pairs match
/// the plaintext bounded-path oracle over that chain — the correctness gate for the
/// hidden-bounded-path cost cell (design record §5.4: differential correctness
/// alongside every cost cell). Returns the number of disclosed endpoint pairs.
///
/// On a length-`scale` chain, `(p){1,k}` connects every `(n_i, n_j)` with
/// `1 <= j − i <= k`, so the oracle count is closed-form — no separate engine
/// needed. `[OPUS-4.8]`
pub fn check_bounded_path(backend: &ShamirBackend, scale: usize) -> Result<usize, MpcError> {
    use crate::hidden_path::{
        eval_bounded_path_hidden, HiddenBoundedPath, HiddenNode, PredicatedEdge, PredicatedEdges,
    };
    use oxrdf::{NamedNode, Term};
    if scale == 0 {
        return Ok(0);
    }
    // Build the chain n_0 →p n_1 →p … →p n_scale (scale edges, scale+1 nodes).
    let node = |i: usize| -> HiddenNode {
        HiddenNode {
            key: Fp::new((i + 1) as u64),
            term: Term::NamedNode(NamedNode::new_unchecked(format!("urn:n:{i}"))),
        }
    };
    let edges: Vec<PredicatedEdge> = (0..scale)
        .map(|i| PredicatedEdge {
            predicate: "p".to_string(),
            subject: node(i),
            object: node(i + 1),
        })
        .collect();
    let edges = PredicatedEdges::new(edges);
    let form = HiddenBoundedPath::range("p", 1, BOUNDED_PATH_K);
    // Plaintext oracle: pairs (i, j) on the chain with 1 <= j−i <= k.
    let k = BOUNDED_PATH_K;
    let expected: usize = (0..=scale)
        .map(|i| {
            let hi = (i + k).min(scale);
            hi.saturating_sub(i)
        })
        .sum();
    // B must cover the (pre-dedup) candidate enumeration, never just the deduped
    // result — `oblivious_set_output` fails closed on `B < candidates`. The safe
    // public upper bound is the projected tuple enumeration Σ_ℓ |E|^ℓ.
    let bound: usize = (1..=k)
        .map(|len| scale.saturating_pow(len as u32))
        .sum::<usize>()
        .max(1);
    let result = eval_bounded_path_hidden(backend, &edges, &form, bound)?;
    if result.rows.len() != expected {
        return Err(MpcError::Protocol(format!(
            "hidden bounded path correctness gate: got {} endpoint pairs, oracle expected {expected}",
            result.rows.len()
        )));
    }
    Ok(result.rows.len())
}

/// Build a secret-shared column from cleartext field values (the dealer shares
/// each; the harness plays all parties, exactly as the oblivious tests do). A
/// [`SecretColumn`] is a `Vec` of full Shamir sharings (one per row).
fn secret_column(dealer: &mut ShamirDealer, values: &[Fp]) -> SecretColumn {
    values.iter().map(|&v| dealer.share(v)).collect()
}

/// Open a secret-shared column to its cleartext field values (reconstruct each
/// row at degree `t`). Used only by the correctness gates.
fn open_column(t: usize, col: &SecretColumn) -> Vec<Fp> {
    col.iter()
        .map(|row| crate::shamir::reconstruct_degree(row, t).expect("reconstruct"))
        .collect()
}

fn multiset_eq(a: &[Fp], b: &[Fp]) -> bool {
    let mut a: Vec<u64> = a.iter().map(|x| x.value()).collect();
    let mut b: Vec<u64> = b.iter().map(|x| x.value()).collect();
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

// =============================================================================
// The MATRIX RUNNER — assemble the cells (cost analytic; correctness gated)
// =============================================================================

/// The party counts the bead asks for (AXIS 2): `{2, 3, 5, 7}`.
pub const DEFAULT_PARTIES: &[usize] = &[2, 3, 5, 7];

/// Build ONE matrix cell: read the actual security off the backend, model the
/// cost analytically, and (for the aggregate / join) record nothing that needs
/// RNG — the cost is pure structure. The seeded backend is used by the SEPARATE
/// correctness gates ([`check_aggregate`] etc.), not here.
pub fn cell(query_class: QueryClass, n: usize, scale: usize) -> Result<MatrixCell, MpcError> {
    let t = (n - 1) / 2;
    // Read the ACTUAL per-operator security at this (n, t) off a fresh production
    // backend (no RNG drawn — `operator_descriptor` is pure). This is the honest
    // security axis: never a requested model.
    let backend = ShamirBackend::new(n)?;
    let security = CellSecurity::read(&backend, query_class.operator_class());
    let comm = match query_class {
        QueryClass::CumulativeSumAggregate => model_aggregate(n),
        QueryClass::HiddenValueEquiJoin => model_hidden_join(n, scale)?,
        QueryClass::ObliviousShuffle => model_shuffle(n, scale),
        QueryClass::ObliviousSort => model_sort(n, scale),
        QueryClass::HiddenBoundedPath => model_bounded_path(n, scale)?,
    };
    Ok(MatrixCell {
        query_class,
        parties: n,
        threshold: t,
        scale,
        security,
        comm,
        declared_preprocessing: backend.info().requires_preprocessing,
    })
}

/// Run the full matrix over the given party counts and per-class scales, emitting
/// every cell. Scales are CAPPED small (the in-process counting tier; the heavy
/// scale-out is the EC2 bead sq-hoaj). The hidden join uses `scale²` equalities,
/// so its scale is deliberately the smallest.
///
/// The aggregate takes NO scale parameter — `N` itself is its scale dimension
/// (each party deals one secret), so it is always recorded at `scale = 1`.
/// `join_scale` caps the hidden-value join's rows/holder and `shuffle_scale` caps
/// the oblivious shuffle/sort cells.
pub fn run_matrix(
    party_counts: &[usize],
    join_scale: usize,
    shuffle_scale: usize,
) -> Result<MatrixResults, MpcError> {
    let mut cells = Vec::new();
    for &n in party_counts {
        // The aggregate runs for every N (N is its scale dimension → scale=1).
        cells.push(cell(QueryClass::CumulativeSumAggregate, n, 1)?);
        // The hidden-value join needs n >= 2t+1 for the equality test — always
        // true at t = ⌊(n−1)/2⌋, so every N is covered.
        cells.push(cell(QueryClass::HiddenValueEquiJoin, n, join_scale)?);
        cells.push(cell(QueryClass::ObliviousShuffle, n, shuffle_scale)?);
        cells.push(cell(QueryClass::ObliviousSort, n, shuffle_scale)?);
        // The hidden bounded property path (sq-py8h.5): models |E| = shuffle_scale
        // secret edges at the fixed public bound k = BOUNDED_PATH_K, B = scale.
        cells.push(cell(QueryClass::HiddenBoundedPath, n, shuffle_scale)?);
    }
    Ok(MatrixResults {
        party_counts: party_counts.to_vec(),
        cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- The matrix builds and is deterministic. ---------------------------
    #[test]
    fn matrix_builds_over_all_party_counts() {
        let r = run_matrix(DEFAULT_PARTIES, 4, 8).unwrap();
        // 5 query classes (aggregate, hidden join, shuffle, sort, bounded path)
        // × 4 party counts = 20 cells.
        assert_eq!(r.cells.len(), DEFAULT_PARTIES.len() * 5);
        // Every party count present.
        for &n in DEFAULT_PARTIES {
            assert!(r.cells.iter().any(|c| c.parties == n));
        }
    }

    #[test]
    fn matrix_is_deterministic() {
        let a = run_matrix(DEFAULT_PARTIES, 4, 8).unwrap();
        let b = run_matrix(DEFAULT_PARTIES, 4, 8).unwrap();
        assert_eq!(a.cells, b.cells, "same matrix inputs → identical cells");
        assert_eq!(a.to_json(), b.to_json());
    }

    // --- AXIS-1 honesty: the security guarantee is the ACTUAL per-operator one. ---
    #[test]
    fn equality_join_is_semi_honest_only_at_n_eq_2t_plus_1() {
        // Odd N (n = 2t+1) → degree-2t open has ZERO RS redundancy → semi-honest-only.
        for &n in &[3usize, 5, 7] {
            let c = cell(QueryClass::HiddenValueEquiJoin, n, 2).unwrap();
            assert_eq!(
                c.security.malicious_security,
                MaliciousSecurity::SemiHonestOnly,
                "n={n}: equality open at n=2t+1 must be semi-honest-only"
            );
            assert_eq!(c.security.trust_model, TrustModel::HonestMajority);
        }
    }

    #[test]
    fn aggregate_is_at_least_abort_for_odd_n_ge_3() {
        // The degree-t aggregate carries RS redundancy for n >= 3 (odd) → not
        // semi-honest-only (abort or robust).
        for &n in &[3usize, 5, 7] {
            let c = cell(QueryClass::CumulativeSumAggregate, n, 1).unwrap();
            assert_ne!(
                c.security.malicious_security,
                MaliciousSecurity::SemiHonestOnly,
                "n={n}: degree-t aggregate has redundancy → at least abort"
            );
        }
    }

    // --- Cost SHAPE assertions (the scaling the protocol predicts). ---------
    #[test]
    fn aggregate_pays_zero_multiplications_and_one_open() {
        for &n in DEFAULT_PARTIES {
            let c = cell(QueryClass::CumulativeSumAggregate, n, 1).unwrap();
            assert_eq!(
                c.multiplications(),
                0,
                "linear aggregate is multiplication-free"
            );
            assert_eq!(c.comm.field_elements_opened, 1, "one open: the sum");
            // n deals × n parties = n² shared field elements.
            assert_eq!(c.comm.field_elements_shared, (n * n) as u64);
        }
    }

    #[test]
    fn hidden_join_opens_grow_as_scale_squared() {
        // For fixed N, doubling the rows/holder quadruples the equality opens
        // (the |L|·|R| all-pairs cost center).
        let n = 5;
        let c2 = cell(QueryClass::HiddenValueEquiJoin, n, 2).unwrap();
        let c4 = cell(QueryClass::HiddenValueEquiJoin, n, 4).unwrap();
        assert_eq!(c2.comm.field_elements_opened, 4); // 2² = 4
        assert_eq!(c4.comm.field_elements_opened, 16); // 4² = 16
        assert_eq!(c4.multiplications(), 4 * c2.multiplications());
    }

    #[test]
    fn aggregate_bytes_per_party_grows_with_n() {
        // The aggregate's wire field elements = n² shared (n deals × n parties) +
        // the single open broadcast as n shares = n² + n, so bytes-per-party =
        // (n² + n)·8 / n = (n + 1)·8 — grows LINEARLY with N, exactly as the
        // protocol predicts (each added holder deals one more secret).
        let mut prev = 0u64;
        for &n in DEFAULT_PARTIES {
            let c = cell(QueryClass::CumulativeSumAggregate, n, 1).unwrap();
            let bpp = c.bytes_per_party();
            assert!(
                bpp > prev,
                "n={n}: bytes/party must grow with N (got {bpp}, prev {prev})"
            );
            // bytes/party = (n + 1)·FIELD_BYTES: (n² shared + n open) / n parties × 8.
            assert_eq!(bpp, (n as u64 + 1) * crate::metrics::FIELD_BYTES as u64);
            prev = bpp;
        }
    }

    #[test]
    fn hidden_join_batched_rounds_below_sequential() {
        // The all-pairs equalities are independent → the batched lower bound is far
        // below the sequential upper bound (the design record's round-batching point).
        let c = cell(QueryClass::HiddenValueEquiJoin, 5, 4).unwrap();
        assert!(
            c.rounds_batched() < c.rounds_sequential(),
            "independent equality tests must batch below the sequential round count"
        );
    }

    #[test]
    fn oblivious_sort_costs_more_rounds_than_shuffle_at_same_scale() {
        // Sort is O(log² n) depth vs shuffle's O(log n) — sort has more rounds.
        let n = 3;
        let scale = 8;
        let shuf = cell(QueryClass::ObliviousShuffle, n, scale).unwrap();
        let sort = cell(QueryClass::ObliviousSort, n, scale).unwrap();
        assert!(
            sort.comm.mult_rounds >= shuf.comm.mult_rounds,
            "Batcher sort depth >= Waksman shuffle depth at the same scale"
        );
    }

    // --- JSON schema is well-formed and carries the honest tier label. -----
    #[test]
    fn json_carries_honest_tier_label_and_cells() {
        let r = run_matrix(&[3], 2, 4).unwrap();
        let json = r.to_json();
        assert!(json.contains("in-process-counting"));
        assert!(json.contains("network cost MODELLED not measured"));
        assert!(json.contains("\"offline_bytes\": 0"));
        assert!(json.contains("hidden_value_equi_join"));
        assert!(json.contains("\"field_bytes\": 8"));
    }

    #[test]
    fn matrix_rows_declare_zero_preprocessing_for_current_backends() {
        // Shamir is currently the only concrete backend; cover every matrix cell.
        let r = run_matrix(DEFAULT_PARTIES, 4, 8).unwrap();
        for c in &r.cells {
            assert_eq!(c.declared_preprocessing, PreprocessingCost::None);
        }
        let json = r.to_json();
        let rows: Vec<_> = json
            .lines()
            .filter(|line| line.contains("\"query_class\""))
            .collect();
        assert_eq!(rows.len(), 20);
        for row in rows {
            assert!(row.contains("\"declared_preprocessing\": {\"basis\": \"declared-model\", \"items\": 0, \"bytes_per_party\": 0}"));
            assert!(row.contains("\"offline_bytes\": 0,"));
        }
    }

    #[test]
    fn declared_preprocessing_uses_cell_multiplications_and_comparisons() {
        let cost = PreprocessingCost::PerOperation {
            items_per_multiplication: 2,
            items_per_comparison: 3,
            bytes_per_item_per_party: Some(7),
        };
        // Four join equalities use four multiplications; a four-row sort has
        // five compare-exchanges, each modeled as a multiplication + comparison.
        for (query, scale, expected) in [
            (QueryClass::CumulativeSumAggregate, 1, (0, 0, 0)),
            (QueryClass::HiddenValueEquiJoin, 2, (8, 56, 168)),
            (QueryClass::ObliviousSort, 4, (25, 175, 525)),
        ] {
            let mut c = cell(query, 3, scale).unwrap();
            c.declared_preprocessing = cost;
            let json = MatrixResults {
                party_counts: vec![3],
                cells: vec![c],
            }
            .to_json();
            assert!(json.contains(&format!(
                "\"declared_preprocessing\": {{\"basis\": \"declared-model\", \"items\": {}, \"bytes_per_party\": {}}}",
                expected.0, expected.1,
            )), "{json}");
            assert!(json.contains(&format!("\"offline_bytes\": {},", expected.2)));
        }
    }

    #[test]
    fn declared_preprocessing_unknown_bytes_serialize_as_null_even_without_items() {
        for (query, scale, items) in [
            (QueryClass::CumulativeSumAggregate, 1, 0),
            (QueryClass::ObliviousSort, 4, 25),
        ] {
            let mut c = cell(query, 3, scale).unwrap();
            c.declared_preprocessing = PreprocessingCost::PerOperation {
                items_per_multiplication: 2,
                items_per_comparison: 3,
                bytes_per_item_per_party: None,
            };
            let json = MatrixResults {
                party_counts: vec![3],
                cells: vec![c],
            }
            .to_json();
            assert!(json.contains(&format!(
                "\"declared_preprocessing\": {{\"basis\": \"declared-model\", \"items\": {items}, \"bytes_per_party\": null}}",
            )), "{json}");
            assert!(json.contains("\"offline_bytes\": null,"));
        }
    }

    #[test]
    fn declared_preprocessing_overflow_is_unknown() {
        let cost = PreprocessingCost::PerOperation {
            items_per_multiplication: 2,
            items_per_comparison: 3,
            bytes_per_item_per_party: Some(u64::MAX),
        };
        assert_eq!(cost.estimate_items(u64::MAX, 0), None);
        assert_eq!(cost.estimate_items(0, u64::MAX), None);
        assert_eq!(cost.estimate_items(u64::MAX / 2, 1), None);
        assert_eq!(cost.estimate_bytes_per_party(1, 0), None);
        assert_eq!(
            PreprocessingCost::None.estimate_items(u64::MAX, u64::MAX),
            Some(0)
        );
        assert_eq!(
            PreprocessingCost::None.estimate_bytes_per_party(u64::MAX, u64::MAX),
            Some(0)
        );
    }
}
