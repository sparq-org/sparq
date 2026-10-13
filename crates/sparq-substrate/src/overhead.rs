//! The **zero-overhead delta** harness: each shared substrate kernel measured
//! against a hand-rolled, pre-extraction equivalent, per the systems paper's §8
//! protocol (`site/papers/sparq-engine-systems.typ` §8, evidence key
//! `substrate.overhead_<kernel>`).
//!
//! # [FABLE-5] sq-atjue — the title-level zero-overhead claim's producing harness
//!
//! The paper's central claim is that "carrying many standards costs no *measured*
//! marginal overhead over the engine's own hand-tuned evaluation". The word
//! *measured* is load-bearing: §8 fixes that the substrate half of the claim is a
//! micro-benchmark of "the shared kernel versus the engine's pre-extraction
//! hand-tuned loops, per kernel, on the canonical host". This module is that
//! micro-benchmark.
//!
//! ## What it measures — and what the number MEANS
//!
//! For each kernel the harness times two implementations over the *same* input:
//!
//! - the **substrate** kernel exactly as the engine (and a future reasoner) call
//!   it — the monomorphic-over-`Id=u32`, no-`Box<dyn>`, `#[inline]` free function
//!   from [`crate::join`] / [`crate::numeric`] / [`crate::compare`], driven through
//!   its generalisation surface: the `JoinKeys` column descriptor, the generic
//!   `Budget` cooperative-cancel type parameter, and the `CompareTerm` trait; and
//! - a **hand-specialised** equivalent written inline right here — the SAME
//!   algorithm and the SAME data structures, but with the generalisation removed:
//!   the key column and combine layout hard-coded to the concrete case, the
//!   `Budget` poll dropped, the `CompareTerm` trait replaced by direct field
//!   access. This is the engine's pre-extraction hand-tuned loop the paper's §5
//!   contrasts against — "whether the *generalised* kernel costs any wall-clock
//!   over the engine's hand-specialised original".
//!
//! The critical methodology rule: the hand-specialised loop runs the **identical
//! algorithm over the identical data structure** as the substrate kernel — a merge
//! join against a merge join, a single-hash `raw_entry` probe against the same
//! probe, a leapfrog intersection against a leapfrog intersection, the `Num` value
//! tower against the `Num` value tower. It removes ONLY the extraction's
//! generalisation layer, never the algorithm. Comparing a kernel against a
//! *different, cheaper* algorithm (e.g. a WCOJ against a nested-loop count) would
//! measure an algorithm gap, not the extraction overhead, and would be dishonest —
//! so this harness deliberately does not do that.
//!
//! The reported [`Kernel::overhead_ratio`] is the fractional wall-clock delta of
//! the shared kernel over the hand-rolled one:
//!
//! ```text
//! overhead_ratio = (substrate_ns - handrolled_ns) / handrolled_ns
//! ```
//!
//! The zero-overhead contract (`research/shared-eval-substrate.md` §2.3, §4) is
//! that the extraction is **shared by source, monomorphised per call site, no
//! vtable between the probe and the comparison** — so the shared kernel and the
//! hand-rolled loop should compile to the same hot loop and the ratio should sit
//! at zero within measurement noise. This harness does **not** assert that; it
//! *measures* it. If a kernel shows a non-negligible positive overhead, the honest
//! disposition — fixed in the paper before the measurement — is to report it as
//! measured and re-scope the "zero-overhead" language to the measured bound, never
//! to bend the number to the claim.
//!
//! ## Honesty boundaries
//!
//! - A wall-clock ratio is only meaningful on a **quiet, dedicated host**. The
//!   emitted envelope carries an `environment` field; a headline number is only
//!   valid when `environment == "canonical"` (a dedicated quiet EC2 box, one
//!   process, min-of-K). A work-box run is `environment == "indicative"` and can
//!   never be cited as the paper headline.
//! - The overhead of a *single* kernel is a ratio of two nanosecond timings, both
//!   subject to the same per-iteration overhead (clock read, black-box fence), so a
//!   tiny per-op kernel's ratio is noisier than a bulk kernel's. The harness reports
//!   the raw substrate/hand-rolled nanoseconds alongside every ratio so a reader can
//!   judge the signal, and it takes the **min of K reps** (the standard robust
//!   estimator for the fastest uncontended path).
//! - The harness is **behaviour-checked, not just timed**: each kernel's two
//!   implementations must produce the *same* result (row count / accumulated value
//!   / order), asserted every rep. A fast-but-wrong hand-rolled loop cannot flatter
//!   the ratio — see [`Kernel::agree`].

use crate::compare::{compare_terms, CompareTerm, LiteralKind, TermClass};
use crate::join::{build_table, hash_probe_serial, key_hash, merge_join, JoinKeys, NoBudget};
use crate::numeric::{ArithOp, Num};
use crate::rows::{Key, Row};
use std::cmp::Ordering;
use std::time::Instant;

/// One measured kernel: the shared-substrate implementation vs the hand-rolled
/// pre-extraction equivalent, timed over identical input.
#[derive(Clone, Debug)]
pub struct Kernel {
    /// Stable slug used in the evidence key `substrate.overhead_<name>`.
    pub name: &'static str,
    /// One-line description of the workload (what the two loops compute).
    pub workload: String,
    /// Problem size (rows / values / result tuples) — records the scale the ratio
    /// was measured at, so the number is not scale-ambiguous.
    pub n: u64,
    /// Repetitions timed; the reported nanoseconds are the min over these reps.
    pub reps: u32,
    /// Min-over-reps nanoseconds for the **substrate** kernel.
    pub substrate_ns: u64,
    /// Min-over-reps nanoseconds for the **hand-rolled** equivalent.
    pub handrolled_ns: u64,
    /// Whether the two implementations produced the identical observable result on
    /// every rep. A `false` here invalidates the ratio (the loops are not
    /// equivalent) and MUST be surfaced, never hidden.
    pub agree: bool,
    /// The honest root cause of a NON-negligible overhead, if any — recorded in the
    /// envelope so a reader (and the paper) sees WHY a kernel is not ~0, per the
    /// "report it as measured, never bend the number to the claim" rule. `None` when
    /// the kernel sits at zero within noise (nothing to explain).
    pub root_cause: Option<&'static str>,
}

impl Kernel {
    /// The fractional wall-clock overhead of the shared kernel over the hand-rolled
    /// one: `(substrate_ns - handrolled_ns) / handrolled_ns`. Positive means the
    /// shared kernel is slower; ~0 (within noise) is the zero-overhead property;
    /// negative means the shared kernel is *faster* (e.g. the single-hash probe).
    ///
    /// Returns `f64::NAN` if the hand-rolled baseline timed as zero (too-small a
    /// workload to time) — a reader must treat a NaN ratio as "unmeasurable at this
    /// scale", not zero.
    #[must_use]
    pub fn overhead_ratio(&self) -> f64 {
        if self.handrolled_ns == 0 {
            return f64::NAN;
        }
        let s = self.substrate_ns as f64;
        let h = self.handrolled_ns as f64;
        (s - h) / h
    }

    /// The evidence key this kernel emits: `substrate.overhead_<name>`.
    #[must_use]
    pub fn evidence_key(&self) -> String {
        format!("substrate.overhead_{}", self.name)
    }
}

/// The whole zero-overhead measurement: every kernel plus the run's provenance.
#[derive(Clone, Debug)]
pub struct OverheadReport {
    /// The kernels measured, in registration order.
    pub kernels: Vec<Kernel>,
    /// `"canonical"` (dedicated quiet host — headline-eligible) or `"indicative"`
    /// (work box — PR body only, never a paper headline).
    pub environment: String,
    /// Free-text host note (instance type / core count) for the envelope.
    pub host_note: String,
    /// Repetitions each kernel was timed over.
    pub reps: u32,
}

impl OverheadReport {
    /// Run the full kernel suite at the given per-kernel `reps` (min-over-reps),
    /// tagging the run with an `environment` and a `host_note`.
    ///
    /// `environment` MUST be `"canonical"` only on a dedicated quiet host; pass
    /// `"indicative"` on a shared/dev box so the emitted envelope cannot be cited
    /// as a paper headline.
    #[must_use]
    pub fn run(reps: u32, environment: &str, host_note: &str) -> OverheadReport {
        let reps = reps.max(1);
        // NOTE (honesty): the leapfrog trie-join (WCOJ) kernel is DELIBERATELY absent
        // from this delta. LFTJ is net-new in the substrate — it has no pre-extraction
        // hand-specialised predecessor in the engine's history to form an honest
        // "generalised vs hand-tuned original" delta against (the reasoners used a
        // *different* algorithm, FxHashMap adjacency, not a specialised leapfrog).
        // Measuring the WCOJ against a nested-loop enumeration would compare two
        // ALGORITHMS, not the extraction's generalisation overhead, and would be
        // dishonest. Its only removable generalisation surface — the generic `Budget`
        // type param and the sorted-slice/`parts_at_level` descriptor — is already
        // measured by `merge_join` and `hash_probe`. (`TrieIter`'s navigation methods
        // are private, so a faithful hand-specialised leapfrog cannot even be written
        // from outside the kernel without reimplementing the algorithm.)
        let kernels = vec![
            bench_merge_join(reps),
            bench_hash_probe(reps),
            bench_num_int_add(reps),
            bench_num_double_add(reps),
            bench_compare_terms(reps),
        ];
        OverheadReport {
            kernels,
            environment: environment.to_string(),
            host_note: host_note.to_string(),
            reps,
        }
    }

    /// Whether every kernel's two implementations agreed on their result. A `false`
    /// invalidates the whole report and the envelope records it loudly.
    #[must_use]
    pub fn all_agree(&self) -> bool {
        self.kernels.iter().all(|k| k.agree)
    }

    /// The house JSON envelope with one `substrate.overhead_<kernel>` record per
    /// kernel, mirroring `site/src/data/paper-evidence.json`'s record shape.
    ///
    /// Hand-rolled JSON (no `serde` dependency in the lean substrate crate): the
    /// value set is small and fixed, and adding a serialiser dep to a leaf crate for
    /// a bench envelope violates the lean-core discipline.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        s.push_str("{\n");
        s.push_str("  \"gather\": \"substrate-zero-overhead-delta\",\n");
        s.push_str("  \"bead\": \"sq-atjue\",\n");
        s.push_str(&format!(
            "  \"canonical\": {},\n",
            self.environment == "canonical"
        ));
        s.push_str(&format!(
            "  \"environment\": {},\n",
            json_str(&self.environment)
        ));
        s.push_str(&format!("  \"host_note\": {},\n", json_str(&self.host_note)));
        s.push_str(&format!("  \"reps_min_of\": {},\n", self.reps));
        s.push_str(&format!("  \"all_kernels_agree\": {},\n", self.all_agree()));
        s.push_str(
            "  \"method\": \"per-kernel wall-clock: (substrate_ns - handrolled_ns) / handrolled_ns, \
min over reps. The hand-rolled loop is the engine's pre-extraction hand-SPECIALISED original: the \
SAME algorithm over the SAME data structure with only the extraction's generalisation removed \
(JoinKeys descriptor / generic Budget / CompareTerm trait replaced by hard-coded concrete access). \
Every kernel's two impls are asserted equal each rep (agree); a non-zero delta carries a \
root_cause. A headline requires environment=canonical (dedicated quiet host). The leapfrog \
trie-join (WCOJ) is deliberately excluded: it has no hand-specialised pre-extraction predecessor \
to form an honest delta against.\",\n",
        );
        s.push_str("  \"records\": {\n");
        for (i, k) in self.kernels.iter().enumerate() {
            let ratio = k.overhead_ratio();
            s.push_str(&format!("    {}: {{\n", json_str(&k.evidence_key())));
            s.push_str(&format!("      \"value\": {},\n", json_f64(ratio)));
            s.push_str("      \"unit\": \"overhead_ratio\",\n");
            s.push_str(&format!(
                "      \"environment\": {},\n",
                json_str(&self.environment)
            ));
            s.push_str("      \"kind\": \"substrate-overhead-delta\",\n");
            s.push_str(&format!("      \"kernel\": {},\n", json_str(k.name)));
            s.push_str(&format!("      \"workload\": {},\n", json_str(&k.workload)));
            s.push_str(&format!("      \"n\": {},\n", k.n));
            s.push_str(&format!("      \"reps\": {},\n", k.reps));
            s.push_str(&format!("      \"substrate_ns\": {},\n", k.substrate_ns));
            s.push_str(&format!("      \"handrolled_ns\": {},\n", k.handrolled_ns));
            s.push_str(&format!("      \"agree\": {},\n", k.agree));
            match k.root_cause {
                Some(rc) => s.push_str(&format!("      \"root_cause\": {},\n", json_str(rc))),
                None => s.push_str("      \"root_cause\": null,\n"),
            }
            s.push_str(
                "      \"note\": \"shared substrate kernel vs hand-specialised pre-extraction loop \
(SAME algorithm + data structure, generalisation removed); overhead_ratio ~0 within noise is the \
zero-overhead property (research/shared-eval-substrate.md Section 2.3, Section 4); positive = \
shared kernel slower (see root_cause), negative = faster.\"\n",
            );
            let comma = if i + 1 < self.kernels.len() { "," } else { "" };
            s.push_str(&format!("    }}{}\n", comma));
        }
        s.push_str("  }\n");
        s.push_str("}\n");
        s
    }

    /// A compact human table for the console (the envelope is the machine record).
    #[must_use]
    pub fn to_table(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "substrate zero-overhead delta (environment={}, min-of-{} reps)\n",
            self.environment, self.reps
        ));
        s.push_str(&format!("  host: {}\n", self.host_note));
        s.push_str(
            "  kernel                    n         substrate_ns  handrolled_ns  overhead   agree\n",
        );
        for k in &self.kernels {
            s.push_str(&format!(
                "  {:<24}  {:>8}  {:>12}  {:>13}  {:>+8.2}%  {}\n",
                k.name,
                k.n,
                k.substrate_ns,
                k.handrolled_ns,
                k.overhead_ratio() * 100.0,
                if k.agree { "yes" } else { "NO!" }
            ));
        }
        for k in &self.kernels {
            if let Some(rc) = k.root_cause {
                s.push_str(&format!("  root_cause[{}]: {}\n", k.name, rc));
            }
        }
        if !self.all_agree() {
            s.push_str(
                "  WARNING: a kernel's substrate and hand-rolled results DISAGREE — the ratio is \
INVALID for that kernel.\n",
            );
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Minimal JSON helpers (no serde dep in the lean substrate crate — the value set
// is small and fixed).
// ---------------------------------------------------------------------------

/// Emit a JSON string literal.
fn json_str(s: &str) -> String {
    let mut out = String::new();
    sparq_core::json::push_string(&mut out, s);
    out
}

/// Emit a JSON number, encoding a non-finite ratio (NaN from an unmeasurable
/// too-small baseline) as `null` so the record stays valid JSON and a reader treats
/// it as "unmeasurable at this scale", not zero.
fn json_f64(v: f64) -> String {
    if v.is_finite() {
        format!("{}", v)
    } else {
        "null".to_string()
    }
}

// ---------------------------------------------------------------------------
// Timing core: min-over-reps nanoseconds for a closure returning an observable
// result, plus an equality check between the two implementations.
// ---------------------------------------------------------------------------

/// Time `f` over `reps` reps, returning `(min_ns, last_result)`. `f` is passed a
/// fresh call each rep; its return value is fed through [`std::hint::black_box`] so
/// the optimiser cannot elide the work.
fn time_min<R, F: FnMut() -> R>(reps: u32, mut f: F) -> (u64, R) {
    // Warm-up rep (not timed) to page in code/data and settle the branch predictor.
    let mut last = std::hint::black_box(f());
    let mut best = u64::MAX;
    for _ in 0..reps {
        let t0 = Instant::now();
        let r = std::hint::black_box(f());
        let ns = t0.elapsed().as_nanos() as u64;
        best = best.min(ns);
        last = r;
    }
    (best, last)
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Two sorted `Row` slices of `n` rows each, joined on column 0 (unique keys, so a
/// 1:1 output of exactly `n` rows). Matches the criterion `substrate.rs` fixture.
fn sorted_rows(n: u32) -> (Vec<Row>, Vec<Row>) {
    let left: Vec<Row> = (0..n)
        .map(|i| {
            let mut r = Row::new();
            r.extend_from_slice(&[i, i + 1000]);
            r
        })
        .collect();
    let right: Vec<Row> = (0..n)
        .map(|i| {
            let mut r = Row::new();
            r.extend_from_slice(&[i, i + 2000]);
            r
        })
        .collect();
    (left, right)
}

fn key_col0() -> JoinKeys {
    JoinKeys {
        key_cols: vec![(0, 0)],
        right_only: vec![1],
    }
}

// ---------------------------------------------------------------------------
// Kernel: merge join
// ---------------------------------------------------------------------------

fn bench_merge_join(reps: u32) -> Kernel {
    let n: u32 = 20_000;
    let (left, right) = sorted_rows(n);

    // Substrate kernel: the shared merge_join with the generic Budget hook.
    let mut out_sub: Vec<Row> = Vec::with_capacity(n as usize);
    let (substrate_ns, sub_len) = time_min(reps, || {
        out_sub.clear();
        merge_join(&left, 0, &right, 0, &[], &[1], &NoBudget, &mut out_sub);
        out_sub.len()
    });

    // Hand-rolled equivalent: the merge loop a pre-extraction engine would write
    // directly against its two sorted row slices, single key column 0, appending
    // right column 1 — no JoinKeys, no Budget type parameter.
    let mut out_hand: Vec<Row> = Vec::with_capacity(n as usize);
    let (handrolled_ns, hand_len) = time_min(reps, || {
        out_hand.clear();
        handrolled_merge_join(&left, &right, &mut out_hand);
        out_hand.len()
    });

    Kernel {
        name: "merge_join",
        workload: format!("sorted merge join, single key column, 1:1 on {} rows", n),
        n: n as u64,
        reps,
        substrate_ns,
        handrolled_ns,
        agree: sub_len == hand_len && out_sub == out_hand,
        root_cause: None,
    }
}

/// The pre-extraction merge join: two sorted slices, key = column 0, output =
/// left row extended with right column 1. Written inline, no descriptor.
#[inline]
fn handrolled_merge_join(left: &[Row], right: &[Row], out: &mut Vec<Row>) {
    let (mut i, mut j) = (0usize, 0usize);
    while i < left.len() && j < right.len() {
        let lk = left[i][0];
        let rk = right[j][0];
        match lk.cmp(&rk) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                // group of equal keys on both sides
                let li_start = i;
                while i < left.len() && left[i][0] == lk {
                    i += 1;
                }
                let rj_start = j;
                while j < right.len() && right[j][0] == rk {
                    j += 1;
                }
                for l in &left[li_start..i] {
                    for r in &right[rj_start..j] {
                        let mut row = l.clone();
                        row.push(r[1]);
                        out.push(row);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Kernel: hash probe (serial)
// ---------------------------------------------------------------------------

fn bench_hash_probe(reps: u32) -> Kernel {
    let n: u32 = 20_000;
    let (left, right) = sorted_rows(n);
    let keys = key_col0();
    let table = build_table(&left, &keys);
    let tables = std::slice::from_ref(&table);

    let mut out_sub: Vec<Row> = Vec::with_capacity(n as usize);
    let (substrate_ns, sub_len) = time_min(reps, || {
        out_sub.clear();
        hash_probe_serial(&right, &keys, &left, tables, &[1], &NoBudget, &mut out_sub);
        out_sub.len()
    });

    // Hand-specialised: probe the SAME substrate hashbrown `JoinTable` (identical
    // data structure) with the SAME single-hash `raw_entry().from_hash` lookup — but
    // with the generalisation stripped: key hard-coded to probe column 0 (no
    // `JoinKeys::right_key` projection), the appended probe column hard-coded to 1
    // (no `probe_only` slice loop), and no `Budget` poll. This is exactly what a
    // pre-extraction engine wrote inline for a single-key equi-join; the only delta
    // from the substrate probe is the removed generalisation layer.
    let mut out_hand: Vec<Row> = Vec::with_capacity(n as usize);
    let (handrolled_ns, hand_len) = time_min(reps, || {
        out_hand.clear();
        handrolled_hash_probe(&right, &left, tables, &mut out_hand);
        out_hand.len()
    });

    // Result-set equality is order-independent here (both emit exactly the 1:1 set);
    // sort both by (key, appended col) before comparing so a hash-order difference
    // does not spuriously fail agreement.
    let agree = sub_len == hand_len && sorted_multiset(&out_sub) == sorted_multiset(&out_hand);

    Kernel {
        name: "hash_probe",
        workload: format!(
            "serial single-hash raw_entry probe, single key column, 1:1 on {} rows",
            n
        ),
        n: n as u64,
        reps,
        substrate_ns,
        handrolled_ns,
        agree,
        // [OPUS-4.8] sq-4r8uy: the descriptor's per-row single-column key projection —
        // previously `key_cols.iter().map(...).collect()` over a heap `Vec<(usize,usize)>`,
        // the whole measured delta in the #1810 canonical run — is now FAST-PATHED. For the
        // dominant `key_cols.len() == 1` case `JoinKeys::right_key` special-cases to a direct
        // one-element `Key::push` (`single_key`), matching the hand-specialised probe's key
        // derivation, so the substrate probe and the hand-rolled loop now do the SAME key
        // work. On the work box the substrate probe's wall-clock roughly halved after this
        // change; whether the residual sits at zero within noise is the CANONICAL re-measure's
        // to decide (a dedicated quiet host, min-of-K), so the honest disposition keeps a
        // root_cause naming the only remaining structural difference rather than asserting
        // ~0 from a noisy work-box run. Set to `None` once the canonical envelope confirms the
        // residual is within noise (the bead's acceptance gate).
        root_cause: Some(
            "single-column JoinKeys key projection now fast-pathed (single_key: a direct \
             one-element Key::push) so it matches the hand-specialised probe; any residual is \
             the descriptor call + Budget type-param plumbing, pending the canonical re-measure",
        ),
    }
}

fn sorted_multiset(rows: &[Row]) -> Vec<Row> {
    let mut v = rows.to_vec();
    v.sort_unstable_by(|a, b| a.as_slice().cmp(b.as_slice()));
    v
}

/// The pre-extraction single-key hash probe: for each probe row hash its column-0
/// key ONCE (via the same [`key_hash`]), select the partition, `raw_entry` lookup on
/// the SAME substrate table, and emit `build_row + probe[1]` per match. Identical
/// algorithm + data structure to `probe_emit`; only the `JoinKeys`/`probe_only`/
/// `Budget` generalisation is removed. `tables` is the substrate's serial
/// [`crate::join::JoinTable`] (one element) built by `build_table`.
#[inline]
fn handrolled_hash_probe(
    probe: &[Row],
    build: &[Row],
    tables: &[crate::join::JoinTable],
    out: &mut Vec<Row>,
) {
    let n_parts = tables.len();
    for pr in probe {
        // Single-key: the key is just column 0.
        let mut key = Key::new();
        key.push(pr[0]);
        let h = key_hash(&key);
        let table = if n_parts == 1 {
            &tables[0]
        } else {
            &tables[(h % n_parts as u64) as usize]
        };
        if let Some((_, matches)) = table.raw_entry().from_hash(h, |k| *k == key) {
            out.reserve(matches.len());
            for &bi in matches {
                let mut combined = build[bi].clone();
                combined.push(pr[1]);
                out.push(combined);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Kernel: Num integer arithmetic (exact tier)
// ---------------------------------------------------------------------------

fn bench_num_int_add(reps: u32) -> Kernel {
    let n: usize = 100_000;
    let vals: Vec<(i64, i64)> = (0..n as i64).map(|i| (i, i + 1)).collect();

    let (substrate_ns, sub_acc) = time_min(reps, || {
        let mut acc: i64 = 0;
        for &(a, b) in &vals {
            if let Some(Num::Int(v)) = Num::Int(a).binop(Num::Int(b), ArithOp::Add) {
                acc = acc.wrapping_add(v);
            }
        }
        acc
    });

    // Hand-specialised: the SAME `Num::binop` rank-0 Add tier logic written inline on
    // `Num` values WITHOUT going through the extracted `binop` method call — the exact
    // pre-extraction body (rank/max dispatch, the `to_dec` div-guard, the rank-0
    // `checked_add` with overflow-to-double). It removes ONLY the method-call
    // indirection of the extracted kernel, not the value-tower dispatch — so the delta
    // measures extraction/monomorphization neutrality, NOT "value tower vs raw i64"
    // (which is a different question the paper does not claim is zero: the tower's
    // typed-dispatch cost is intrinsic and identical before and after the move).
    let (handrolled_ns, hand_acc) = time_min(reps, || {
        let mut acc: i64 = 0;
        for &(a, b) in &vals {
            if let Some(Num::Int(v)) = inline_num_binop(Num::Int(a), Num::Int(b), ArithOp::Add) {
                acc = acc.wrapping_add(v);
            }
        }
        acc
    });

    Kernel {
        name: "num_int_add",
        workload: format!(
            "xsd:integer addition through the Num value tower (extracted call vs inline body), \
             {} pairs",
            n
        ),
        n: n as u64,
        reps,
        substrate_ns,
        handrolled_ns,
        agree: sub_acc == hand_acc,
        root_cause: None,
    }
}

/// The full `Num::binop` tier body, replicated inline here (a byte-for-byte copy of
/// the pre-extraction engine `binop`), so the numeric delta compares the extracted
/// `Num::binop` method against the identical logic NOT behind that method — the true
/// "generalised kernel vs hand-tuned original" test for the value tower. Both run the
/// same tier dispatch + `to_dec` div-guard + checked-arith-with-overflow-to-double,
/// so the delta isolates the extraction cost (expected ~0), not the tower's intrinsic
/// typed-dispatch cost over machine arithmetic.
#[inline]
fn inline_num_binop(x: Num, y: Num, op: ArithOp) -> Option<Num> {
    fn rank(n: Num) -> u8 {
        match n {
            Num::Int(_) => 0,
            Num::Dec(_) => 1,
            Num::Float(_) => 2,
            Num::Double(_) => 3,
        }
    }
    fn f64_of(n: Num) -> f64 {
        n.f64()
    }
    // [OPUS-5] issue #3796: the float tier promotes through the SHARED `Num::f32` helper —
    // one correctly-rounded conversion — exactly as `Num::binop` does. The pre-fix body
    // here was a verbatim copy of `numeric.rs`'s `f64() as f32` double rounding, so the
    // defect lived in TWO places; both now route through the one helper and cannot drift.
    fn f32_of(n: Num) -> f32 {
        n.f32()
    }
    fn to_dec(n: Num) -> Option<crate::numeric::Dec> {
        n.to_dec()
    }
    fn apply(a: f64, b: f64, op: ArithOp) -> f64 {
        match op {
            ArithOp::Add => a + b,
            ArithOp::Sub => a - b,
            ArithOp::Mul => a * b,
            ArithOp::Div => a / b,
        }
    }
    fn apply32(a: f32, b: f32, op: ArithOp) -> f32 {
        match op {
            ArithOp::Add => a + b,
            ArithOp::Sub => a - b,
            ArithOp::Mul => a * b,
            ArithOp::Div => a / b,
        }
    }
    let r = rank(x).max(rank(y));
    if r == 3 {
        return Some(Num::Double(apply(f64_of(x), f64_of(y), op)));
    }
    if r == 2 {
        return Some(Num::Float(apply32(f32_of(x), f32_of(y), op)));
    }
    let (a, b) = (to_dec(x)?, to_dec(y)?);
    if op == ArithOp::Div {
        if b.mant == 0 {
            return None;
        }
        return match a.checked_div(b) {
            Some(d) => Some(Num::Dec(d)),
            None => Some(Num::Double(f64_of(x) / f64_of(y))),
        };
    }
    if r == 0 {
        let (xi, yi) = (
            match x {
                Num::Int(i) => i,
                _ => unreachable!(),
            },
            match y {
                Num::Int(i) => i,
                _ => unreachable!(),
            },
        );
        let rr = match op {
            ArithOp::Add => xi.checked_add(yi),
            ArithOp::Sub => xi.checked_sub(yi),
            ArithOp::Mul => xi.checked_mul(yi),
            ArithOp::Div => unreachable!(),
        };
        return Some(match rr {
            Some(i) => Num::Int(i),
            None => Num::Double(apply(xi as f64, yi as f64, op)),
        });
    }
    let rr = match op {
        ArithOp::Add => a.checked_add(b),
        ArithOp::Sub => a.checked_sub(b),
        ArithOp::Mul => a.checked_mul(b),
        ArithOp::Div => unreachable!(),
    };
    Some(match rr {
        Some(d) => Num::Dec(d),
        None => Num::Double(apply(f64_of(x), f64_of(y), op)),
    })
}

// ---------------------------------------------------------------------------
// Kernel: Num double arithmetic (inexact tier)
// ---------------------------------------------------------------------------

fn bench_num_double_add(reps: u32) -> Kernel {
    let n: usize = 100_000;
    let vals: Vec<(f64, f64)> = (0..n).map(|i| (i as f64 * 0.1, i as f64 * 0.01)).collect();

    let (substrate_ns, sub_acc) = time_min(reps, || {
        let mut acc = 0.0f64;
        for &(a, b) in &vals {
            if let Some(Num::Double(v)) = Num::Double(a).binop(Num::Double(b), ArithOp::Add) {
                acc += v;
            }
        }
        acc.to_bits()
    });

    // Hand-specialised: the SAME `Num::binop` rank-3 (Double) tier path, inline (no
    // extracted method call). Isolates the extraction cost, not double-vs-raw-f64.
    let (handrolled_ns, hand_acc) = time_min(reps, || {
        let mut acc = 0.0f64;
        for &(a, b) in &vals {
            if let Some(Num::Double(v)) =
                inline_num_binop(Num::Double(a), Num::Double(b), ArithOp::Add)
            {
                acc += v;
            }
        }
        acc.to_bits()
    });

    Kernel {
        name: "num_double_add",
        workload: format!(
            "xsd:double addition through the Num value tower (extracted call vs inline body), \
             {} pairs",
            n
        ),
        n: n as u64,
        reps,
        substrate_ns,
        handrolled_ns,
        agree: sub_acc == hand_acc,
        root_cause: None,
    }
}

// ---------------------------------------------------------------------------
// Kernel: compare_terms (SPARQL total order) over a numeric-literal workload
// ---------------------------------------------------------------------------

/// A minimal `CompareTerm` model for the benchmark: an integer literal, a double
/// literal, or a plain string. Mirrors the shape the engine's `Value` presents to
/// `compare_terms`, restricted to the families the delta workload exercises. Kept
/// deliberately small so the trait dispatch (which monomorphises) is the thing
/// under measurement, not term construction.
#[derive(Clone, Debug, PartialEq)]
enum BenchTerm {
    Int(i64),
    Dbl(f64),
    Str(String),
}

impl CompareTerm for BenchTerm {
    fn term_class(&self) -> TermClass {
        // All three families are RDF literals.
        TermClass::Literal
    }
    fn literal_kind(&self) -> LiteralKind {
        match self {
            BenchTerm::Int(_) | BenchTerm::Dbl(_) => LiteralKind::Numeric,
            BenchTerm::Str(_) => LiteralKind::String,
        }
    }
    fn value_str(&self) -> Option<String> {
        match self {
            BenchTerm::Int(i) => Some(i.to_string()),
            BenchTerm::Dbl(d) => Some(d.to_string()),
            BenchTerm::Str(s) => Some(s.clone()),
        }
    }
    fn as_f64(&self) -> Option<f64> {
        match self {
            BenchTerm::Int(i) => Some(*i as f64),
            BenchTerm::Dbl(d) => Some(*d),
            BenchTerm::Str(_) => None,
        }
    }
    fn exact_cmp(&self, other: &Self) -> Option<Ordering> {
        // Exact-rational recheck only fires on an f64 tie; for this workload's
        // distinct integers the tie is rare, but implement it exactly: integers by
        // i128, mixed via f64 (the values here never collapse at 2^53).
        match (self, other) {
            (BenchTerm::Int(a), BenchTerm::Int(b)) => Some((*a as i128).cmp(&(*b as i128))),
            _ => self.as_f64().partial_cmp(&other.as_f64()),
        }
    }
    fn strict_cmp(&self, _other: &Self) -> Option<Ordering> {
        // No strict typed family in this model; caller falls back to lexical.
        None
    }
    fn triple_parts(&self) -> Option<[Self; 3]> {
        None
    }
}

fn bench_compare_terms(reps: u32) -> Kernel {
    let n: usize = 50_000;
    // Alternating int / double / string terms so the class + kind dispatch runs.
    let terms: Vec<BenchTerm> = (0..n)
        .map(|i| match i % 3 {
            0 => BenchTerm::Int(i as i64),
            1 => BenchTerm::Dbl(i as f64 * 1.5),
            _ => BenchTerm::Str(format!("s{:06}", i)),
        })
        .collect();

    // Substrate: run the shared compare_terms over consecutive pairs, folding the
    // ordering into an accumulator so nothing is elided.
    let (substrate_ns, sub_acc) = time_min(reps, || {
        let mut acc: i64 = 0;
        for w in terms.windows(2) {
            acc += ordering_code(compare_terms(&w[0], &w[1]));
        }
        acc
    });

    // Hand-rolled: the total-order comparison a pre-extraction engine would write
    // inline for these three families — kind-first, numeric by f64 then exact
    // recheck on tie, string lexically — with no CompareTerm trait indirection.
    let (handrolled_ns, hand_acc) = time_min(reps, || {
        let mut acc: i64 = 0;
        for w in terms.windows(2) {
            acc += ordering_code(handrolled_compare(&w[0], &w[1]));
        }
        acc
    });

    Kernel {
        name: "compare_terms",
        workload: format!(
            "SPARQL total order over a mixed int/double/string literal stream, {} comparisons",
            n - 1
        ),
        n: (n - 1) as u64,
        reps,
        substrate_ns,
        handrolled_ns,
        agree: sub_acc == hand_acc,
        root_cause: None,
    }
}

/// The pre-extraction inline total order for the three benchmark families:
/// kind-first (Numeric < String), numeric by f64 then i128 recheck on tie, string
/// lexical. No `CompareTerm` trait — the loop a pre-extraction engine wrote directly.
#[inline]
fn handrolled_compare(x: &BenchTerm, y: &BenchTerm) -> Option<Ordering> {
    fn kind_rank(t: &BenchTerm) -> u8 {
        match t {
            BenchTerm::Int(_) | BenchTerm::Dbl(_) => 0, // Numeric
            BenchTerm::Str(_) => 4,                     // String
        }
    }
    let (kx, ky) = (kind_rank(x), kind_rank(y));
    if kx != ky {
        return Some(kx.cmp(&ky));
    }
    match (x, y) {
        (BenchTerm::Str(a), BenchTerm::Str(b)) => Some(a.cmp(b)),
        _ => {
            let a = num_f64(x);
            let b = num_f64(y);
            match a.partial_cmp(&b) {
                Some(Ordering::Equal) | None => match (x, y) {
                    (BenchTerm::Int(ai), BenchTerm::Int(bi)) => {
                        Some((*ai as i128).cmp(&(*bi as i128)))
                    }
                    _ => a.partial_cmp(&b),
                },
                ord => ord,
            }
        }
    }
}

#[inline]
fn num_f64(t: &BenchTerm) -> f64 {
    match t {
        BenchTerm::Int(i) => *i as f64,
        BenchTerm::Dbl(d) => *d,
        BenchTerm::Str(_) => f64::NAN,
    }
}

/// A stable integer code for an `Option<Ordering>` so the fold observes the result.
#[inline]
fn ordering_code(o: Option<Ordering>) -> i64 {
    match o {
        Some(Ordering::Less) => -1,
        Some(Ordering::Equal) => 0,
        Some(Ordering::Greater) => 1,
        None => 2,
    }
}

// ---------------------------------------------------------------------------
// Tests — the REAL path: the harness runs, every kernel's two impls AGREE, and
// each evidence key is well-formed. (Timing is non-deterministic and NOT asserted;
// the invariant under test is result-equivalence + envelope shape.)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kernel_substrate_and_handrolled_agree() {
        // A small rep count keeps the test fast; agreement is rep-independent.
        let report = OverheadReport::run(2, "indicative", "unit-test");
        for k in &report.kernels {
            assert!(
                k.agree,
                "kernel `{}` substrate and hand-rolled results disagree — the overhead ratio is \
                 invalid",
                k.name
            );
        }
        assert!(report.all_agree());
    }

    #[test]
    fn evidence_keys_are_well_formed() {
        let report = OverheadReport::run(1, "indicative", "unit-test");
        // Exactly the five kernels this delta measures (merge_join, hash_probe,
        // num_int_add, num_double_add, compare_terms). LFTJ is deliberately excluded —
        // it has no hand-specialised pre-extraction predecessor (see `run`).
        let keys: Vec<String> = report.kernels.iter().map(|k| k.evidence_key()).collect();
        assert_eq!(keys.len(), 5);
        for key in &keys {
            assert!(
                key.starts_with("substrate.overhead_"),
                "evidence key `{}` must start with substrate.overhead_",
                key
            );
        }
        // No duplicate kernel slugs.
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "duplicate kernel slug");
    }

    #[test]
    fn envelope_marks_indicative_as_non_canonical() {
        let report = OverheadReport::run(1, "indicative", "unit-test");
        let json = report.to_json();
        assert!(json.contains("\"canonical\": false"));
        assert!(json.contains("\"environment\": \"indicative\""));
        // Every record carries its evidence key + the honest agree flag.
        assert!(json.contains("substrate.overhead_merge_join"));
        assert!(json.contains("\"agree\":"));
    }

    #[test]
    fn canonical_environment_sets_canonical_true() {
        let report = OverheadReport::run(1, "canonical", "test-canonical-host");
        assert!(report.to_json().contains("\"canonical\": true"));
    }

    /// The inline `Num::binop` replica must agree with the substrate method on the
    /// `xsd:float` PROMOTION boundary, not merely on small exactly-representable values.
    ///
    /// This kernel is a deliberate byte-for-byte copy of the pre-extraction body, and the
    /// copy carried its own instance of the `f64() as f32` double rounding (issue #3796):
    /// fixing `numeric.rs` alone left the defect alive here, and the timing harness's
    /// `agree` flag could not see it because it feeds small integers. The fixtures below
    /// are the exact-rational-verified midpoint witnesses; the assertion is on
    /// `f32::to_bits()` because the two candidates are ADJACENT floats. [OPUS-5]
    #[test]
    fn inline_num_binop_float_promotion_matches_substrate_bit_exactly() {
        // `H +/- 1` around an f32 midpoint that is exactly representable in f64.
        const UP: i64 = 4_611_686_293_305_294_849;
        const DOWN: i64 = 4_611_686_843_061_108_735;
        const UP_CORRECT: u32 = 0x5E80_0001;
        const DOWN_CORRECT: u32 = 0x5E80_0001;
        for (n, want) in [(UP, UP_CORRECT), (DOWN, DOWN_CORRECT)] {
            for op in [ArithOp::Add, ArithOp::Sub] {
                let hand = inline_num_binop(Num::Int(n), Num::Float(0.0), op);
                let sub = Num::Int(n).binop(Num::Float(0.0), op);
                let (hb, sb) = match (hand, sub) {
                    (Some(Num::Float(h)), Some(Num::Float(s))) => (h.to_bits(), s.to_bits()),
                    other => panic!("float tier expected, got {:?}", other),
                };
                assert_eq!(hb, sb, "inline replica diverged from Num::binop for n={}", n);
                assert_eq!(hb, want, "inline replica double-rounded n={}", n);
            }
        }
        // And the scaled-decimal promotion, which also routes through `Num::f32`.
        let dec = Num::Dec(crate::numeric::Dec { mant: 46_116_862_933_052_948_481, scale: 1 });
        let hand = inline_num_binop(dec, Num::Float(0.0), ArithOp::Add);
        let sub = dec.binop(Num::Float(0.0), ArithOp::Add);
        match (hand, sub) {
            (Some(Num::Float(h)), Some(Num::Float(s))) => {
                assert_eq!(h.to_bits(), s.to_bits());
                assert_eq!(h.to_bits(), UP_CORRECT);
            }
            other => panic!("float tier expected, got {:?}", other),
        }
    }

    #[test]
    fn handrolled_merge_join_matches_substrate_exactly() {
        // Direct unit coverage of the hand-rolled merge loop against the substrate
        // kernel on a small, hand-checkable input (a mutation of the loop that broke
        // equivalence would fail here, not just skew a timing).
        let (left, right) = sorted_rows(8);
        let mut out_sub = Vec::new();
        merge_join(&left, 0, &right, 0, &[], &[1], &NoBudget, &mut out_sub);
        let mut out_hand = Vec::new();
        handrolled_merge_join(&left, &right, &mut out_hand);
        assert_eq!(out_sub, out_hand);
        assert_eq!(out_sub.len(), 8);
    }
}
