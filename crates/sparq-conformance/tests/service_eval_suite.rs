//! [OPUS-4.8] sq-ddpgx (epic sq-my8wd) — the W3C SPARQL 1.1 `sparql11/service`
//! EVALUATION conformance lane, wired as a RATCHETED gate that mirrors the SPARQL /
//! SHACL / GeoSPARQL / Solid / JSON-LD / D-entailment ratchets in this crate
//! (crate-local `cargo test` + a pinned pass-count FLOOR that may only RISE,
//! registered in the central `scoreboard::SUITES` and guarded by
//! `tests/scoreboard_floors.rs`).
//!
//! ## What is gated
//!
//! Every `mf:QueryEvaluationTest` of the W3C `sparql11/service` manifest, run through
//! the merged in-process loopback harness (sq-ushvx, #1291). For each test the runner
//! stands up a REAL [`sparq_server::serve`] endpoint per `qt:serviceData` block on a
//! fresh ephemeral `127.0.0.1:0` loopback port (serving the block's remote RDF),
//! rewrites the well-known endpoint IRIs to the bound loopback URLs, runs the federated
//! `SERVICE` query end-to-end through the engine's REAL `ureq` transport, and compares
//! the solutions to the suite's `.srx` oracle. This GRADUATES the deferred
//! SERVICE/federation tests that the SPARQL binary skips (`unsupported_feature =
//! "SERVICE / federation"`).
//!
//! ## SILENT vs non-SILENT semantics (the load-bearing answer-safety invariant)
//!
//! Two manifest tests (`service6` / `service7`) carry a
//! `SERVICE SILENT <http://invalid.endpoint.org/sparql>` to an endpoint that is never
//! stood up and never allowlisted — so the call really hits a fail-closed refusal and
//! the oracle requires `SILENT` to SWALLOW it (empty remote contribution, the query
//! continues). The complementary non-SILENT direction is pinned directly here against a
//! CLOSED port: a non-SILENT `SERVICE` to a dead endpoint must PROPAGATE the error
//! (`silent_swallows_vs_non_silent_propagates_against_closed_port`), and the live-endpoint
//! control proves the same loopback endpoint, when reachable, returns the row — so the
//! SILENT path is genuinely swallowing a real failure, not masking a no-op.
//!
//! ## Honest floor + the tracked-not-asserted remainder
//!
//! `SERVICE_EVAL_FLOOR` is the MEASURED pass count at the pinned rdf-tests revision,
//! NOT an aspirational target; MIRRORED in `scoreboard::SUITES` and read textually by
//! `tests/scoreboard_floors.rs`. It may only RISE. Any test below the floor is an honest
//! engine/federation divergence, tracked-not-asserted (a child of sq-my8wd) and never
//! skip-laundered to inflate the count; a FAILING test always fails the gate.
//!
//! ## Feature gating (both states)
//!
//! The lane is behind this crate's opt-in `service` feature (forwards to
//! `service-loopback` → tokio + axum via `sparq-server/service` AND ureq via
//! `sparq-engine/service`). With the feature OFF this file compiles to a single
//! self-SKIP `#[test]` (no tokio/axum/ureq linked here — the lean opt-in posture). The
//! rdf-tests fixtures are fetched by `scripts/fetch-conformance.sh` into the gitignored
//! `tests/w3c/rdf-tests/`; when absent the runner SKIPS so a fresh offline checkout
//! stays green.
//!
//!   cargo test -p sparq-conformance --features service --test service_eval_suite

#[cfg(not(feature = "service"))]
#[test]
fn service_eval_suite_skipped_without_feature() {
    eprintln!(
        "SKIP: the W3C sparql11/service EVALUATION lane is OFF — build with \
         `--features service` (and run scripts/fetch-conformance.sh) to run it."
    );
}

#[cfg(feature = "service")]
mod gated {
    use sparq_conformance::service_eval;
    use std::path::PathBuf;

    /// The service-evaluation pass FLOOR (the RATCHET). MEASURED pass count at the
    /// pinned rdf-tests revision; MIRRORED in the central scoreboard
    /// (`scoreboard::SUITES`) and read textually by the guard test
    /// `tests/scoreboard_floors.rs`. It may only RISE — never lower it. This is the
    /// ACTUAL current pass count, not an aspirational target.
    pub const SERVICE_EVAL_FLOOR: usize = 6; // [SONNET-4.6] sq-my8wd.1: +service3 (nested non-SILENT SERVICE now handled)

    fn rdf_tests_root() -> PathBuf {
        // CARGO_MANIFEST_DIR is crates/sparq-conformance; the data lives at the
        // workspace-root `tests/w3c/rdf-tests` (the same clone the SPARQL harness
        // uses, fetched by scripts/fetch-conformance.sh).
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdf-tests")
    }

    #[test]
    fn service_evaluation_ratchet() {
        let root = rdf_tests_root();
        if !root.join("sparql/sparql11/service/manifest.ttl").exists() {
            eprintln!(
                "SKIP: rdf-tests `sparql11/service` not present under {} — run \
                 scripts/fetch-conformance.sh",
                root.display()
            );
            return;
        }

        let out = service_eval::run_service_suite(&root)
            .unwrap_or_else(|e| panic!("service evaluation suite error: {e}"));

        // [OPUS-4.8] The CI ratchet greps this exact `TOTAL service` line (pass count
        // in field $3), mirroring the JSON-LD / d-entail lanes' `TOTAL <cat>` shape.
        // Positional `format!`/`println!` args (not inline `{pass}`) per the CodeQL
        // `rust/unused-variable` false-positive guard in the shared agent contract.
        println!("\nW3C SPARQL 1.1 sparql11/service evaluation conformance (pinned rdf-tests)");
        println!(
            "TOTAL service {} {} {} (floor {})",
            out.pass, out.fail, out.skip, SERVICE_EVAL_FLOOR
        );
        if !out.failures.is_empty() {
            println!("service evaluation FAILS:");
            for (name, why) in &out.failures {
                println!("  - {}: {}", name, why);
            }
        }

        assert_eq!(
            out.fail, 0,
            "service evaluation lane has failing tests: {:?}",
            out.failures
        );
        assert!(
            out.pass >= SERVICE_EVAL_FLOOR,
            "service evaluation pass count {} regressed below the ratchet floor {}",
            out.pass,
            SERVICE_EVAL_FLOOR
        );
    }

    /// The answer-safety invariant: `SERVICE SILENT` to a CLOSED (never stood-up)
    /// endpoint must SWALLOW the failure and continue with an empty remote
    /// contribution, while the SAME query WITHOUT `SILENT` must PROPAGATE the error.
    /// Both are checked against a real refused call (a loopback port that is not
    /// allowlisted under the default-deny-private egress policy), and a live control
    /// proves the endpoint, when reachable, actually returns the row — so SILENT is
    /// genuinely masking a real failure, not a no-op.
    #[test]
    fn silent_swallows_vs_non_silent_propagates_against_closed_port() {
        // A REAL reachable loopback endpoint so the live control returns one row.
        let remote = "<http://ex/a> <http://ex/p> \"v\" .\n";
        let live_template = "SELECT ?o WHERE { SERVICE <{EP}> { <http://ex/a> <http://ex/p> ?o } }";
        let live_rows = service_eval::run_against_live_endpoint(remote, live_template)
            .expect("live endpoint returns the row");
        assert_eq!(live_rows, 1, "the reachable endpoint binds ?o");

        // A loopback endpoint that exists but is NOT allowlisted by the call below:
        // the default-deny-private egress policy refuses the dial (a fail-closed
        // refusal — the same shape as a closed port to the engine's transport).
        // SILENT must swallow it (the local row survives, no remote contribution);
        // the non-SILENT form must propagate it as an error.
        use sparq_conformance::service_loopback::LoopbackEndpoint;
        let dead = sparq_core::Graph::load_str(remote, "ntriples").expect("remote graph");
        let endpoint = LoopbackEndpoint::serve(dead);
        let unreachable = endpoint.sparql_url(); // bound, but we will NOT allowlist its host

        let local =
            sparq_core::Graph::load_str("<http://ex/s> <http://ex/q> \"L\" .\n", "ntriples")
                .expect("local graph");

        // Run WITHOUT allowlisting the endpoint host => the dial is refused.
        let silent_q = format!(
            "SELECT ?l ?o WHERE {{ <http://ex/s> <http://ex/q> ?l . \
             SERVICE SILENT <{unreachable}> {{ <http://ex/a> <http://ex/p> ?o }} }}"
        );
        let silent_res = sparq_engine::query(&local, &silent_q);
        assert!(
            silent_res.is_ok(),
            "SERVICE SILENT must swallow the refused/closed-endpoint call: {silent_res:?}"
        );
        let silent_res = silent_res.unwrap();
        assert_eq!(
            silent_res.rows.len(),
            1,
            "SILENT keeps the local row with the remote contribution dropped"
        );

        let non_silent_q = format!(
            "SELECT ?l ?o WHERE {{ <http://ex/s> <http://ex/q> ?l . \
             SERVICE <{unreachable}> {{ <http://ex/a> <http://ex/p> ?o }} }}"
        );
        let non_silent_res = sparq_engine::query(&local, &non_silent_q);
        assert!(
            non_silent_res.is_err(),
            "a NON-SILENT SERVICE to the refused/closed endpoint must propagate the error \
             (got {non_silent_res:?})"
        );
    }
}
