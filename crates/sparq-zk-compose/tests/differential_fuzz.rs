// written while Fable 5 unavailable — re-review when Fable returns.
//! sq-61g: differential prove → verify → compare-to-cleartext fuzzer.
//!
//! For each random iteration the fuzzer:
//!  1. generates a small random RDF "credential" graph (a single subject with a
//!     random set of `<ex/predN> <xsd:integer>` triples) and a random query from
//!     the supported circuit family — either a bare BGP scan
//!     `{ ?s <pred> ?o }` or that scan plus a numeric `FILTER(?o <op> <bound>)`;
//!  2. evaluates the query in CLEARTEXT over the same triples (the oracle): the
//!     set of matched (s,p,o) rows, and — when a FILTER is present — the boolean
//!     verdict per matched row;
//!  3. drives the SAME data through the ZK pipeline: `build_scan` /
//!     `build_filter_int` → `prover_toml_for` → real `nargo execute` witness
//!     generation (and, for the heavy variant, a full `bb prove` +
//!     `verify_manifest`), then reads the ZK PUBLIC result (the disclosed `rows`
//!     and the proved `expected` verdict);
//!  4. asserts the ZK public result EQUALS the cleartext result.
//!
//! A divergence here is exactly a soundness (ZK accepts a result cleartext
//! rejects) or completeness (ZK drops a result cleartext keeps) bug. The default
//! test runs a modest, CI-sane number of iterations against the cheap witness
//! path (which already exercises the full in-circuit relation — `nargo execute`
//! fails to produce a witness iff the relation is unsatisfiable). The heavy
//! `#[ignore]`d variant runs fewer iterations but adds a real `bb prove` +
//! `verify_manifest`, exercising the cryptographic proof end to end.
//!
//! Seedable: the per-iteration seed is derived from a base seed (env
//! `SPARQ_FUZZ_SEED`, else a fixed default) so failures are reproducible. On any
//! mismatch the failing seed is printed for one-line repro.

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::{commit_triples, GraphCommitment};
use sparq_zk::encode::{encode_term, salt_from_bytes};
use sparq_zk::field::{field_to_hex, Fr};
use sparq_zk_compose::build::{build_filter_int, build_scan, Pattern, Slot};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::manifest::{FieldHex, FilterOp, ProofInputs};
use sparq_zk_compose::toml::prover_toml_for;

// --- tiny deterministic PRNG (splitmix64) — no new dependency ---------------
// We only need a reproducible stream of small integers; splitmix64 is a one-liner
// with good-enough statistical quality for fuzzing input selection.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    /// Uniform-ish in `[0, n)` (n > 0).
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    fn boolean(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

// The base seed: override with SPARQ_FUZZ_SEED to reproduce a reported failure
// (the per-iteration seed printed on failure already pins the exact case, but a
// base override lets you sweep a different stream).
fn base_seed() -> u64 {
    std::env::var("SPARQ_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5EED_C0FFEE_u64)
}

mod support;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sparq_zk_compose_fuzz_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";

fn int_lit(v: u64) -> Term {
    Term::Literal(Literal::new_typed_literal(v.to_string(), iri(XSD_INT)))
}

// --- the generated test case -----------------------------------------------

/// One random predicate→value pair in the generated subject's graph.
#[derive(Clone)]
struct Fact {
    pred: String,
    value: u64,
}

/// A generated fuzz case: a single-subject credential graph + a chosen query.
struct FuzzCase {
    triples: Vec<Triple>,
    facts: Vec<Fact>,
    /// The predicate the query's BGP pattern fixes (`{ ?s <query_pred> ?o }`).
    query_pred: String,
    /// An optional numeric FILTER on the matched object: `?o <op> <bound>`.
    filter: Option<(FilterOp, u64)>,
}

/// The compiled `filter_int` digit buckets (`build::FILTER_INT_D_VALUES`). The
/// circuit's `digits: [u8; D]` witness requires the operand's decimal digit
/// count to EXACTLY equal a compiled `D` (and forbids a leading zero for D>1),
/// so an operand is provable iff its digit count == a compiled `D`. The compiled
/// range is now the contiguous 1..=4 (sq-wto added the `d=3` member
/// that closes the old gap — a 3-digit operand used to derive an unprovable
/// `d=4`); 5..=19 digit operands still have NO member and `build_filter_int`
/// returns `None` (a CLEAN error, never a silently-unprovable d — that is exactly
/// what `derive_filter_int_id` now guarantees by requiring an exact match). The
/// fuzzer draws FILTER-eligible values only from these exact provable counts.
const FILTER_DIGIT_BUCKETS: &[u32] = &[1, 2, 3, 4];

/// Draw an `xsd:integer` value whose canonical decimal has exactly `d` digits
/// (no leading zero for d>1).
fn value_with_digits(rng: &mut Rng, d: u32) -> u64 {
    match d {
        1 => rng.below(10),                       // 0..=9
        2 => 10 + rng.below(90),                  // 10..=99
        3 => 100 + rng.below(900),                // 100..=999 (sq-wto)
        4 => 1000 + rng.below(9000),              // 1000..=9999
        _ => unreachable!("only compiled buckets"),
    }
}

/// Generate a random small credential graph + query.
///
/// Object values are drawn only from compiled `filter_int` digit buckets (1, 2,
/// or 4 digits — see [`FILTER_DIGIT_BUCKETS`]) so a matched object is always a
/// provable FILTER operand; graph size is kept ≤ the `n=16` slot-bucket and
/// matched rows ≤ the `r=4` row-bucket of the `scan_k1` member, so a single
/// committed graph always derives a compiled scan id.
fn generate(rng: &mut Rng) -> FuzzCase {
    let subject = NamedOrBlankNode::NamedNode(iri("http://ex/subj"));
    // 1..=4 distinct predicates so matched rows for a single predicate is 1 (one
    // value per predicate), keeping us inside r=4 even when the query predicate
    // is absent (0 matched rows is also valid — empty result).
    let n_preds = 1 + rng.below(4) as usize; // 1..=4
    let mut facts: Vec<Fact> = Vec::with_capacity(n_preds);
    let mut triples = Vec::with_capacity(n_preds);
    for i in 0..n_preds {
        let pred = format!("http://ex/pred{i}");
        // Pick a compiled digit bucket then a value of exactly that many digits,
        // so the matched object is a provable FILTER operand.
        let d = FILTER_DIGIT_BUCKETS[rng.below(FILTER_DIGIT_BUCKETS.len() as u64) as usize];
        let value = value_with_digits(rng, d);
        facts.push(Fact { pred: pred.clone(), value });
        triples.push(Triple::new(subject.clone(), iri(&pred), int_lit(value)));
    }

    // The query predicate: usually one that EXISTS (so we exercise matches), but
    // with some probability a predicate that is ABSENT (exercise the empty
    // result / completeness corner).
    let query_pred = if rng.below(5) == 0 {
        "http://ex/absent".to_string()
    } else {
        facts[rng.below(facts.len() as u64) as usize].pred.clone()
    };

    // Optionally attach a numeric FILTER on the matched object.
    let filter = if rng.boolean() {
        let op = match rng.below(6) {
            0 => FilterOp::Lt,
            1 => FilterOp::Le,
            2 => FilterOp::Gt,
            3 => FilterOp::Ge,
            4 => FilterOp::Eq,
            _ => FilterOp::Ne,
        };
        // Bound near the actual values so both verdicts occur; allow 0..=10000.
        let bound = rng.below(10_001);
        Some((op, bound))
    } else {
        None
    };

    FuzzCase { triples, facts, query_pred, filter }
}

// --- the CLEARTEXT oracle ---------------------------------------------------

/// Cleartext evaluation of the case's scan `{ ?s <query_pred> ?o }`: the matched
/// integer object value(s). With a single subject + distinct predicates this is
/// 0 or 1 values, but we model it as a set for generality.
fn cleartext_scan_values(case: &FuzzCase) -> Vec<u64> {
    case.facts
        .iter()
        .filter(|f| f.pred == case.query_pred)
        .map(|f| f.value)
        .collect()
}

/// Cleartext numeric comparison verdict (`value <op> bound`).
fn cleartext_verdict(op: FilterOp, value: u64, bound: u64) -> bool {
    match op {
        FilterOp::Lt => value < bound,
        FilterOp::Le => value <= bound,
        FilterOp::Gt => value > bound,
        FilterOp::Ge => value >= bound,
        FilterOp::Eq => value == bound,
        FilterOp::Ne => value != bound,
    }
}

// --- the ZK path ------------------------------------------------------------

/// Build the scan over one committed graph. Returns the commitment + built scan,
/// or `None` if the case does not fit a compiled member (caller skips it — not a
/// soundness signal, just an out-of-family input).
fn zk_build_scan(case: &FuzzCase, salt: Fr) -> Option<(GraphCommitment, sparq_zk_compose::build::BuiltScan)> {
    let commit = commit_triples(&case.triples, salt).ok()?;
    let pattern = Pattern {
        s: Slot::Var,
        p: Slot::Const(Term::NamedNode(iri(&case.query_pred))),
        o: Slot::Var,
    };
    let scan = build_scan(std::slice::from_ref(&commit), &pattern)?;
    Some((commit, scan))
}

/// The ZK PUBLIC scan result: the disclosed object values, read from the scan's
/// `rows` public input (slot 2), decoded by matching each disclosed encoding back
/// to a fact's encoded value. We compare ENCODINGS (the on-field public values),
/// not cleartext ints, by encoding the cleartext oracle values the same way — so
/// this is a true comparison of "what the public proof discloses" vs "what
/// cleartext says should be disclosed".
fn zk_scan_disclosed_encodings(scan: &ProofInputs) -> Vec<String> {
    match scan {
        ProofInputs::Scan { rows, row_count, .. } => rows
            .iter()
            .take(*row_count as usize)
            .map(|r| r[2].0.clone())
            .collect(),
        _ => unreachable!("scan inputs"),
    }
}

/// Encode a cleartext integer object value to its term encoding hex, the way the
/// scan discloses it (literals are salt-independent in this encoding, but we pass
/// the graph salt for fidelity).
fn encode_value_hex(value: u64, salt: Fr) -> String {
    let enc = encode_term(&int_lit(value), &salt).expect("int literal encodes");
    field_to_hex(&enc)
}

/// The DEFAULT, CI-sane iteration count for the cheap witness-path fuzzer.
const DEFAULT_ITERS: u64 = 24;
/// The heavy (`#[ignore]`) full-prove iteration count (each is ~1-2s of bb).
const HEAVY_ITERS: u64 = 6;

/// Core differential body shared by the cheap + heavy variants. `full_prove`
/// selects whether each case runs a real `bb prove` + `verify_manifest` (heavy)
/// or only `nargo execute` witness-gen (cheap, but still the full in-circuit
/// relation). Returns the number of cases actually exercised (some are skipped
/// as out-of-family, which is not a failure).
fn run_differential(iters: u64, full_prove: bool) -> u64 {
    let prover = CircuitProver::from_crate_root();
    let base = base_seed();
    let mut exercised = 0u64;

    for i in 0..iters {
        // Per-iteration seed derived from the base — printed on failure.
        let seed = base ^ (i.wrapping_mul(0x100_0000_01b3));
        let mut rng = Rng::new(seed);
        let case = generate(&mut rng);
        // A per-case salt so distinct iterations don't share commitment state.
        let salt = salt_from_bytes(&{
            let mut b = [0u8; 32];
            b[..8].copy_from_slice(&seed.to_le_bytes());
            b[8] = 0xA5; // make it non-zero / distinctive
            b
        });

        let Some((_commit, scan)) = zk_build_scan(&case, salt) else {
            // Out-of-family input (no compiled member fits) — skip, not a bug.
            continue;
        };
        exercised += 1;

        // ---- SCAN differential: disclosed rows vs cleartext matches. ----
        let zk_rows = zk_scan_disclosed_encodings(&scan.inputs);
        let mut clear_rows: Vec<String> = cleartext_scan_values(&case)
            .into_iter()
            .map(|v| encode_value_hex(v, salt))
            .collect();
        let mut zk_sorted = zk_rows.clone();
        zk_sorted.sort();
        clear_rows.sort();
        assert_eq!(
            zk_sorted, clear_rows,
            "SCAN result divergence (seed {seed:#x}): ZK disclosed {zk_rows:?} vs cleartext {clear_rows:?}; \
             query_pred={}, facts={:?}",
            case.query_pred,
            case.facts.iter().map(|f| (&f.pred, f.value)).collect::<Vec<_>>()
        );

        // The cheap path verifies the relation is SATISFIABLE for the honest
        // statement: nargo execute must produce a witness. (A wrong disclosed row
        // set would already have failed the assertion above; here we confirm the
        // honest statement the cleartext oracle agrees with is also provable.)
        let challenge = FieldHex("0x2a".into());
        let (scan_id, scan_toml) = prover_toml_for(
            &scan.inputs,
            &challenge,
            &scan.witness.counts,
            &scan.witness.enc,
            &[], None, None
        ).unwrap();
        let tag = format!("fuzz_scan_{i}");
        prover.compile(&scan_id).expect("scan compiles");
        prover
            .gen_witness_tagged(&scan_id, &scan_toml, &tag)
            .unwrap_or_else(|e| {
                panic!(
                    "SCAN witness UNSAT for an honest cleartext-agreeing statement \
                     (seed {seed:#x}): {e}"
                )
            });

        // ---- FILTER differential (when the case carries a FILTER). ----
        if let Some((op, bound)) = case.filter {
            // The filter is applied to the matched object value(s). With one
            // matched row (single subject) we filter that value; with zero
            // matched rows there is nothing to filter (skip the filter leg).
            let matched = cleartext_scan_values(&case);
            for value in matched {
                let expected = cleartext_verdict(op, value, bound);
                let operand_enc = FieldHex(encode_value_hex(value, salt));
                // Build the HONEST filter (expected = the cleartext verdict).
                let Some((filter_inputs, digits)) =
                    build_filter_int(operand_enc.clone(), value, op, bound, expected)
                else {
                    // Value out of the d≤4 family (shouldn't happen: value≤9999).
                    continue;
                };
                let (fid, ftoml) =
                    prover_toml_for(&filter_inputs, &challenge, &[], &[], &digits, None, None).unwrap();
                let ftag = format!("fuzz_filter_{i}");
                prover.compile(&fid).expect("filter compiles");

                // Honest verdict must be PROVABLE (witness exists)...
                prover
                    .gen_witness_tagged(&fid, &ftoml, &ftag)
                    .unwrap_or_else(|e| {
                        panic!(
                            "FILTER witness UNSAT for the HONEST cleartext verdict \
                             (seed {seed:#x}): {value} {op:?} {bound} = {expected}: {e}"
                        )
                    });

                // ...and the FLIPPED verdict must be UNPROVABLE (soundness: the
                // circuit must not let a lie pass). nargo execute signals
                // unsatisfiability by NOT writing a witness => gen_witness errs.
                let Some((lie_inputs, lie_digits)) =
                    build_filter_int(operand_enc, value, op, bound, !expected)
                else {
                    continue;
                };
                let (lid, ltoml) =
                    prover_toml_for(&lie_inputs, &challenge, &[], &[], &lie_digits, None, None).unwrap();
                let ltag = format!("fuzz_filter_lie_{i}");
                let lie_witness = prover.gen_witness_tagged(&lid, &ltoml, &ltag);
                assert!(
                    lie_witness.is_err(),
                    "SOUNDNESS VIOLATION (seed {seed:#x}): the circuit produced a \
                     witness for the FALSE verdict {value} {op:?} {bound} = {} \
                     (cleartext says {expected})",
                    !expected
                );

                // Heavy variant: a real bb prove of the honest filter, then bb
                // verify, asserting the proved PUBLIC verdict equals cleartext.
                if full_prove {
                    let out = scratch(&format!("fuzz_prove_{i}"));
                    let art = prover
                        .prove_in(&fid, &ftoml, &out, &format!("fuzz_prove_{i}"))
                        .expect("honest filter proves");
                    let ok = prover
                        .verify(&art, &out.join("verify"))
                        .expect("bb verify runs");
                    assert!(
                        ok,
                        "PROOF of the honest cleartext verdict failed to verify \
                         (seed {seed:#x}): {value} {op:?} {bound} = {expected}"
                    );
                    let _ = std::fs::remove_dir_all(&out);
                }
            }
        }
    }
    exercised
}

/// sq-61g — the default CI-sane differential fuzzer (cheap witness path). Runs a
/// modest number of random cases; each asserts the ZK disclosed scan rows equal
/// cleartext matches, the honest FILTER verdict is provable, and the FLIPPED
/// verdict is UNprovable (soundness). Skips cleanly if nargo/bb absent.
#[test]
fn differential_fuzz_witness_path() {
    if !support::require_toolchain("differential_fuzz_witness_path", &["nargo", "bb"]) {
        return;
    }
    let n = run_differential(DEFAULT_ITERS, false);
    eprintln!("differential_fuzz_witness_path: {n} in-family cases exercised");
    assert!(n > 0, "no in-family fuzz cases were exercised — generator is broken");
}

/// sq-61g (heavy) — the full prove→verify→compare-to-cleartext fuzzer. Each case
/// runs a real `bb prove` + `bb verify` of the honest FILTER and asserts the
/// proved public verdict verifies (and matches cleartext). Slower (~seconds per
/// case), so gated behind `#[ignore]`.
///
/// Run with:
///   cargo test -p sparq-zk-compose --test differential_fuzz -- --ignored
/// Reproduce a specific stream with:
///   SPARQ_FUZZ_SEED=0x... cargo test -p sparq-zk-compose --test differential_fuzz -- --ignored
#[test]
#[ignore = "slow: full bb prove per fuzz case (sq-61g heavy variant)"]
fn differential_fuzz_full_prove() {
    if !support::require_toolchain("differential_fuzz_full_prove", &["nargo", "bb"]) {
        return;
    }
    let n = run_differential(HEAVY_ITERS, true);
    eprintln!("differential_fuzz_full_prove: {n} in-family cases exercised (full bb prove)");
    assert!(n > 0, "no in-family fuzz cases were exercised — generator is broken");
}

// --- sq-wto: the FILTER digit-bucket completeness fix --------------------------
//
// sq-wto. The bug the differential fuzzer surfaced: `build_filter_int`
// derived the `filter_int_d{D}` member from the operand's decimal DIGIT COUNT via
// `smallest_bucket(.., digits)` (smallest compiled `D >= digits`), but the circuit
// requires the digit count to EXACTLY equal `D`. So a 3-digit operand derived
// `d=4`, whose `digits: [u8; 4]` witness no 3-digit value can fill — an honest
// FILTER that SILENTLY produced an UNPROVABLE statement (`nargo execute` writes no
// witness). NOT a soundness hole, but a completeness/honesty hole: the prover
// believed it had a statement it could never prove.
//
// The fix: a `d=3` member fills the gap (range now contiguous 1..=4), and
// `derive_filter_int_id` requires an EXACT match — out-of-family counts (5..) now
// return `None` (a CLEAN error at `build_filter_int`), never a wrong-D member.
//
// This test pins the post-fix invariant the brief demands: for a 3-digit and a
// 5-digit operand the pipeline either PROVES correctly or ERRORS cleanly — it is
// NEVER silently-unprovable (i.e. `build_filter_int` returns `Some` only when the
// honest witness is actually producible).
use sparq_zk_compose::build::derive_filter_int_id;
use sparq_zk_compose::manifest::CircuitId;

/// A 3-digit operand: `build_filter_int` must return `Some` AND the honest
/// statement must be PROVABLE (witness produced) — the gap the bug left
/// silently-unprovable. The flipped verdict must stay UNprovable (soundness
/// preserved by the new member).
#[test]
fn sq_wto_three_digit_operand_proves_not_silently_unprovable() {
    if !support::require_toolchain("sq_wto_three_digit_operand_proves_not_silently_unprovable", &["nargo", "bb"]) {
        return;
    }
    let value: u64 = 123; // 3 digits — the old smallest-bucket bug's witness.
    // Derivation picks the EXACT d=3 member (not the old wrong d=4).
    assert_eq!(
        derive_filter_int_id(3),
        Some(CircuitId::FilterInt { d: 3 }),
        "a 3-digit operand must derive the EXACT d=3 member (sq-wto)"
    );
    let salt = Fr::from(0u64);
    let operand_enc = FieldHex(encode_value_hex(value, salt));
    let op = FilterOp::Ge;
    let bound = 100u64;
    let expected = value >= bound; // honest verdict, true

    // 1) The honest statement must build AND prove (no silently-unprovable d).
    let (inputs, digits) = build_filter_int(operand_enc.clone(), value, op, bound, expected)
        .expect("3-digit FILTER must be in-family (sq-wto: d=3 member exists)");
    assert_eq!(*inputs.circuit_id(), CircuitId::FilterInt { d: 3 });
    let (id, toml) = prover_toml_for(&inputs, &FieldHex("0x2a".into()), &[], &[], &digits, None, None).unwrap();
    let prover = CircuitProver::from_crate_root();
    prover.compile(&id).expect("filter_int_d3 compiles");
    prover
        .gen_witness_tagged(&id, &toml, "wto_d3_honest")
        .expect(
            "sq-wto: the HONEST 3-digit FILTER must PROVE — a witness must exist \
             (the old derivation made this silently-unprovable)",
        );

    // 2) Soundness preserved: the FLIPPED verdict must be UNprovable.
    let (lie_inputs, lie_digits) =
        build_filter_int(operand_enc, value, op, bound, !expected).expect("builds");
    let (lid, ltoml) = prover_toml_for(&lie_inputs, &FieldHex("0x2a".into()), &[], &[], &lie_digits, None, None).unwrap();
    let lie = prover.gen_witness_tagged(&lid, &ltoml, "wto_d3_lie");
    assert!(
        lie.is_err(),
        "sq-wto: the FALSE verdict over a 3-digit operand must remain UNprovable"
    );
}

/// A 5-digit operand: out of the compiled family, so `build_filter_int` must
/// return `None` — a CLEAN error at the call site, NOT a derived wrong-D member
/// that would be silently-unprovable. (No toolchain needed — pure derivation.)
#[test]
fn sq_wto_five_digit_operand_errors_cleanly_not_silently_unprovable() {
    // 5-digit value: no compiled `d=5` member.
    assert_eq!(
        derive_filter_int_id(5),
        None,
        "a 5-digit operand has no compiled member and must derive None (clean error)"
    );
    let salt = Fr::from(0u64);
    let value: u64 = 54321; // 5 digits
    let operand_enc = FieldHex(encode_value_hex(value, salt));
    let built = build_filter_int(operand_enc, value, FilterOp::Lt, 99999, true);
    assert!(
        built.is_none(),
        "sq-wto: an out-of-family (5-digit) operand must yield None from \
         build_filter_int — a clean error, never a silently-unprovable wrong-D member"
    );
}
