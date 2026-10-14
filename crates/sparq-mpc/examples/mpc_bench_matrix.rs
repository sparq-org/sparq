// sq-sxm — runnable harness for the (security model × N × query class)
// MPC benchmark matrix, IN-PROCESS counting tier (Tier 1). Emits the deterministic
// modelled communication (bytes/party, rounds, multiplications) + the per-cell
// ACTUAL security guarantee, as both a human table and the structured JSON schema.
//
// Run (the seeded RNG for the correctness gates needs the insecure-test-rng
// feature — OFF by default; it only makes the SIMULATION reproducible, never a
// production claim):
//
//   cargo run -p sparq-mpc --example mpc_bench_matrix --features insecure-test-rng
//   cargo run -p sparq-mpc --example mpc_bench_matrix --features insecure-test-rng -- --json
//
// HONESTY: this is the single-process counting tier. The numbers are MODELLED
// communication, not measured network cost; a single-process wall-clock is NOT an
// MPC latency. The real multi-process transport + tc/netem LAN/WAN emulation is a
// SEPARATE bead (sq-tg6b); EC2 scale-out is sq-hoaj.
//
// The feature gate means a default build of this example only prints the cost
// matrix (which needs no RNG) and SKIPS the correctness gates with a clear note,
// so it still compiles + runs without the feature.

use sparq_mpc::bench::{run_matrix, DEFAULT_PARTIES};

fn main() {
    // Cap the scales SMALL — this is the in-process counting tier (watch df; the
    // heavy scale-out is the EC2 bead sq-hoaj). The hidden join is scale² pairs.
    let join_scale = 6; // 36 equality tests/cell at the largest
    let shuffle_scale = 16; // oblivious networks over 16 rows

    let results = run_matrix(DEFAULT_PARTIES, join_scale, shuffle_scale)
        .expect("matrix runs over the default party counts");

    let json_only = std::env::args().any(|a| a == "--json");
    if json_only {
        print!("{}", results.to_json());
        return;
    }

    print!("{}", results.to_table());
    println!();

    // The correctness gates RUN THE REAL PRIMITIVES so a fast-but-wrong cost cell
    // is caught (design record §5.4). They need the seedable simulation RNG.
    #[cfg(feature = "insecure-test-rng")]
    run_correctness_gates(join_scale, shuffle_scale);
    #[cfg(not(feature = "insecure-test-rng"))]
    println!(
        "(correctness gates skipped: re-run with --features insecure-test-rng to \
         co-verify the real primitives reproducibly)"
    );

    println!();
    println!("--- JSON schema (structured results; consume this, not the table) ---");
    print!("{}", results.to_json());
}

#[cfg(feature = "insecure-test-rng")]
fn run_correctness_gates(join_scale: usize, shuffle_scale: usize) {
    use sparq_mpc::bench::{check_aggregate, check_hidden_join, check_shuffle, check_sort};
    use sparq_mpc::ShamirBackend;

    println!("--- correctness gates (real primitives co-run; deterministic seed) ---");
    for &n in DEFAULT_PARTIES {
        // Fixed seed ⇒ reproducible simulation masks (NEVER a production claim;
        // that is exactly what the insecure-test-rng feature name warns about).
        let backend = ShamirBackend::new_seeded(n, 0x5347_5851_4D50_4331).expect("seeded backend");
        // Aggregate: sum of n distinct values reconstructs to the plaintext sum.
        let values: Vec<u64> = (0..n as u64).map(|i| 1000 * (i + 1)).collect();
        let expected: u64 = values.iter().sum();
        let got = check_aggregate(&backend, &values).expect("aggregate runs");
        assert_eq!(got, expected, "n={n}: secure sum must equal plaintext sum");

        // Hidden join: scale matching rows on both sides ⇒ scale joined rows.
        let joined = check_hidden_join(&backend, join_scale).expect("join runs");
        assert_eq!(
            joined, join_scale,
            "n={n}: hidden join must find all matches"
        );

        // Oblivious shuffle/sort over the same backend's dealer.
        let mut dealer = backend.dealer();
        let t = backend.threshold();
        assert!(
            check_shuffle(&mut dealer, t, shuffle_scale).expect("shuffle runs"),
            "n={n}: shuffle must preserve the multiset"
        );
        assert!(
            check_sort(&mut dealer, t, shuffle_scale).expect("sort runs"),
            "n={n}: sort must produce an ascending permutation"
        );
        println!("  n={n}: aggregate=OK join={joined}/{join_scale} shuffle=OK sort=OK");
    }
    println!("all correctness gates passed.");
}
