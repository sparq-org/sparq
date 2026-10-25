// [OPUS-4.8] sq-pn2: full-family prove/verify cost-curve harness.
//!
//! Drives every member of the `zk/compose/` circuit family through a single
//! real `bb prove` + `bb verify` and reports the (k, n, r, d) cost curve:
//! prove wall-clock, verify wall-clock, and proof size. The scan family sweeps
//! (k, n, r) — disclosed-attribute / row / slot dimensions — and the filter
//! family sweeps d (digit count). Emits a human-readable table to stderr and a
//! machine-readable JSON line per member to stdout (redirect to a JSON file).
//!
//! This is a STANDALONE timing harness (not criterion: each prove is ~1-2s of
//! bb, so criterion's repeated sampling would be hours). It is NOT gated in CI.
//! See README.md for how to run it.
//!
//! Usage:
//!   cd bench/zk-compose/family_curve
//!   cargo run --release > ../family_cost_curve.json
//!   # optional: REPEATS=3 averages each member over N proves (default 1).

use std::path::PathBuf;
use std::time::Instant;

use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_zk::commit::commit_triples;
use sparq_zk::encode::salt_from_bytes;
use sparq_zk::field::Fr;
use sparq_zk_compose::build::{
    build_filter_f64, build_filter_int, build_scan, encode_double_literal, encode_int_literal,
    Pattern, Slot,
};
use sparq_zk_compose::driver::CircuitProver;
use sparq_zk_compose::manifest::{CircuitId, FieldHex, FilterOp};
use sparq_zk_compose::toml::prover_toml_for;

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const CHALLENGE: &str = "0x2a";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}
fn int_lit(v: u64) -> Term {
    Term::Literal(Literal::new_typed_literal(v.to_string(), iri(XSD_INT)))
}

/// A graph of `n_triples` triples sharing one subject, predicate `<ex/pK>` per
/// triple (distinct predicates), the query predicate `<ex/age>` placed on the
/// FIRST `n_matches` triples so the disclosed row count is exactly `n_matches`.
fn graph(subject: &str, n_triples: usize, n_matches: usize) -> Vec<Triple> {
    let s = NamedOrBlankNode::NamedNode(iri(subject));
    let mut out = Vec::with_capacity(n_triples);
    for i in 0..n_triples {
        let pred = if i < n_matches { "http://ex/age".to_string() } else { format!("http://ex/p{i}") };
        out.push(Triple::new(s.clone(), iri(&pred), int_lit((i as u64) % 1000)));
    }
    out
}

fn age_pattern() -> Pattern {
    Pattern { s: Slot::Var, p: Slot::Const(Term::NamedNode(iri("http://ex/age"))), o: Slot::Var }
}

fn salt(b: u8) -> Fr {
    salt_from_bytes(&[b; 32])
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq_zk_family_curve_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A measured row of the cost curve.
struct Row {
    member: String,
    k: u32,
    n: u32,
    r: u32,
    d: u32,
    prove_secs: f64,
    verify_secs: f64,
    proof_bytes: usize,
    vk_bytes: usize,
}

impl Row {
    fn to_json(&self) -> String {
        format!(
            "{{\"member\":\"{}\",\"k\":{},\"n\":{},\"r\":{},\"d\":{},\"prove_secs\":{:.3},\"verify_secs\":{:.3},\"proof_bytes\":{},\"vk_bytes\":{}}}",
            self.member, self.k, self.n, self.r, self.d, self.prove_secs, self.verify_secs, self.proof_bytes, self.vk_bytes
        )
    }
}

fn repeats() -> usize {
    std::env::var("REPEATS").ok().and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(1)
}

/// Prove `id` from `toml` `repeats` times (timed), then verify once (timed,
/// averaged over `repeats`). Returns (mean prove secs, mean verify secs, proof
/// bytes, vk bytes).
fn time_member(
    prover: &CircuitProver,
    id: &CircuitId,
    toml: &str,
    tag: &str,
) -> (f64, f64, usize, usize) {
    let reps = repeats();
    let out = scratch(tag);
    // Warm the ACIR cache so the first prove doesn't pay compile cost.
    prover.compile(id).expect("compiles");

    let mut prove_total = 0.0;
    let mut last_art = None;
    for i in 0..reps {
        let t = Instant::now();
        let art = prover.prove_in(id, toml, &out, &format!("{tag}_{i}")).expect("prove");
        prove_total += t.elapsed().as_secs_f64();
        last_art = Some(art);
    }
    let art = last_art.unwrap();

    let mut verify_total = 0.0;
    for i in 0..reps {
        let t = Instant::now();
        let ok = prover.verify(&art, &out.join(format!("verify_{i}"))).expect("verify runs");
        verify_total += t.elapsed().as_secs_f64();
        assert!(ok, "honest proof must verify");
    }
    let _ = std::fs::remove_dir_all(&out);
    (prove_total / reps as f64, verify_total / reps as f64, art.proof.len(), art.vk.len())
}

/// Build + time a scan member sized to land on (k, n_bucket, r_bucket).
fn bench_scan(prover: &CircuitProver, k: u32, graph_triples: usize, matches: usize, tag: &str) -> Row {
    let commits: Vec<_> = (0..k)
        .map(|g| {
            // Per-graph distinct subject + salt so commitments differ; the first
            // `matches` triples carry <ex/age> so each graph contributes the same
            // disclosed rows (kept within the r bucket via `matches`).
            commit_triples(&graph(&format!("http://ex/s{g}"), graph_triples, matches), salt(10 + g as u8)).unwrap()
        })
        .collect();
    let scan = build_scan(&commits, &age_pattern()).expect("scan in family");
    let (id, n, r) = match scan.inputs.circuit_id() {
        // This curve uses build_scan's opaque string-canonical witness only.
        // V2 component witnesses belong to the dedicated V2 proof tests.
        CircuitId::Scan { n, r, .. } => (scan.inputs.circuit_id().clone(), *n, *r),
        _ => unreachable!(),
    };
    let (cid, toml) = prover_toml_for(
        &scan.inputs,
        &FieldHex(CHALLENGE.into()),
        &scan.witness.counts,
        &scan.witness.enc,
        &[],
        None,
        None,
    )
    .expect("scan prover toml");
    assert_eq!(cid, id);
    let (prove_secs, verify_secs, proof_bytes, vk_bytes) = time_member(prover, &cid, &toml, tag);
    Row {
        member: cid.package().unwrap(),
        k,
        n,
        r,
        d: 0,
        prove_secs,
        verify_secs,
        proof_bytes,
        vk_bytes,
    }
}

/// Build + time a filter_int member for a `d`-digit operand.
fn bench_filter_int(prover: &CircuitProver, value: u64, tag: &str) -> Row {
    let operand_enc = encode_int_literal(value);
    let (filter, digits) = build_filter_int(operand_enc, value, FilterOp::Lt, value + 1, true).expect("filter in family");
    let d = match filter.circuit_id() {
        CircuitId::FilterInt { d } => *d,
        _ => unreachable!(),
    };
    let (cid, toml) =
        prover_toml_for(&filter, &FieldHex(CHALLENGE.into()), &[], &[], &digits, None, None)
            .expect("filter_int prover toml");
    let (prove_secs, verify_secs, proof_bytes, vk_bytes) = time_member(prover, &cid, &toml, tag);
    Row { member: cid.package().unwrap(), k: 0, n: 0, r: 0, d, prove_secs, verify_secs, proof_bytes, vk_bytes }
}

/// Build + time a MANIFEST-COMPOSABLE filter_f64 member for a `d`-digit
/// integer-valued xsd:double operand ([OPUS-4.8] sq-q7e / sq-tat / sq-kep2).
/// Mirrors [`bench_filter_int`]: `filter_f64` is now manifest-composable (it
/// carries `d` in its `CircuitId`, has a real `ProofInputs::FilterF64`, and
/// renders its Prover.toml via [`prover_toml_for`] from the canonical decimal
/// digit witness) rather than the old raw hand-written-Prover.toml building
/// block. Proves `value < value + 1` (true) over the integer-valued double.
fn bench_filter_f64(prover: &CircuitProver, value: u64, tag: &str) -> Row {
    let operand_enc = encode_double_literal(value);
    // [OPUS-4.8] sq-kep2: `value + 1` would wrap silently in release builds at
    // u64::MAX, so widen before adding. f64 is only exact for integers up to
    // 2^53; assert the bound stays inside that range so any out-of-family input
    // fails loudly and deterministically rather than silently losing precision.
    let bound_u = u128::from(value) + 1;
    debug_assert!(
        bound_u <= (1u128 << 53),
        "filter_f64 bound {bound_u} exceeds the 2^53 f64-exactness limit"
    );
    let bound = bound_u as f64;
    let (filter, digits) = build_filter_f64(operand_enc, value, FilterOp::Lt, bound, true)
        .expect("filter_f64 in family");
    let d = match filter.circuit_id() {
        CircuitId::FilterF64 { d } => *d,
        _ => unreachable!(),
    };
    let (cid, toml) =
        prover_toml_for(&filter, &FieldHex(CHALLENGE.into()), &[], &[], &digits, None, None)
            .expect("filter_f64 prover toml");
    let (prove_secs, verify_secs, proof_bytes, vk_bytes) = time_member(prover, &cid, &toml, tag);
    Row { member: cid.package().unwrap(), k: 0, n: 0, r: 0, d, prove_secs, verify_secs, proof_bytes, vk_bytes }
}

fn main() {
    // Toolchain check.
    let have = |t: &str| {
        std::process::Command::new(t).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
    };
    if !(have("nargo") && have("bb")) {
        eprintln!("nargo + bb required on PATH; aborting.");
        std::process::exit(1);
    }

    let prover = CircuitProver::from_crate_root();
    eprintln!("== zk-compose full-family prove/verify cost curve (REPEATS={}) ==", repeats());

    // Each `bench_*` runs a real prove/verify in sequence (left-to-right);
    // collected into the curve. Scan sizes are chosen to land on each compiled
    // member: scan_k1_n16_r4 (k=1, graph<=16, matches<=4), scan_k2_n16_r8 (k=2,
    // graph<=16, matches<=8), scan_k2_n64_r8 (k=2, graph in (16,64], matches<=8).
    // The filter_int sweep covers d in {1,2,4}; the composable filter_f64 sweep
    // mirrors it over the same d in {1,2,4} ([OPUS-4.8] sq-kep2).
    let rows = vec![
        bench_scan(&prover, 1, 8, 2, "scan_k1"),
        bench_scan(&prover, 2, 12, 4, "scan_k2_n16"),
        bench_scan(&prover, 2, 40, 4, "scan_k2_n64"),
        bench_filter_int(&prover, 5, "fi_d1"),    // 1 digit
        bench_filter_int(&prover, 42, "fi_d2"),   // 2 digits
        bench_filter_int(&prover, 7000, "fi_d4"), // 4 digits
        bench_filter_f64(&prover, 5, "ff64_d1"),    // 1 digit
        bench_filter_f64(&prover, 42, "ff64_d2"),   // 2 digits
        bench_filter_f64(&prover, 7000, "ff64_d4"), // 4 digits
    ];

    // Human-readable table to stderr.
    eprintln!(
        "\n{:<18} {:>2} {:>3} {:>2} {:>2} {:>10} {:>11} {:>11} {:>8}",
        "member", "k", "n", "r", "d", "prove (s)", "verify (s)", "proof (B)", "vk (B)"
    );
    for r in &rows {
        eprintln!(
            "{:<18} {:>2} {:>3} {:>2} {:>2} {:>10.3} {:>11.3} {:>11} {:>8}",
            r.member, r.k, r.n, r.r, r.d, r.prove_secs, r.verify_secs, r.proof_bytes, r.vk_bytes
        );
    }

    // Machine-readable JSON to stdout.
    println!("{{");
    println!("  \"tool\": \"bench/zk-compose/family_curve (sq-pn2 standalone harness)\",");
    println!("  \"repeats\": {},", repeats());
    println!("  \"rows\": [");
    for (i, r) in rows.iter().enumerate() {
        let comma = if i + 1 < rows.len() { "," } else { "" };
        println!("    {}{}", r.to_json(), comma);
    }
    println!("  ]");
    println!("}}");
}
