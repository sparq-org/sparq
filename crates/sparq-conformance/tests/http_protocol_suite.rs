//! sq-jaj38 (epic sq-my8wd) — the W3C SPARQL 1.1 **Protocol** (HTTP layer)
//! conformance lane, wired as a RATCHETED gate that mirrors the SPARQL / SHACL / GeoSPARQL /
//! Solid / JSON-LD / D-entailment / sparql11-service ratchets in this crate (crate-local
//! `cargo test` + a pinned pass-count FLOOR that may only RISE, registered in the central
//! `scoreboard::SUITES` and guarded by `tests/scoreboard_floors.rs`).
//!
//! ## What is gated
//!
//! The SPARQL 1.1 **Protocol** operations exercised against the merged in-process loopback
//! server (sq-ushvx, #1291) over RAW HTTP — query via GET / POST-urlencoded / POST-direct, the
//! HTTP `QUERY` method (#1304), update via POST, the `default-graph-uri` / `named-graph-uri`
//! dataset overrides, result-format content negotiation for SELECT/ASK
//! (`application/sparql-results+json`, `+xml`, `text/csv`, `text/tab-separated-values`) and the
//! documented status codes (200/400/405/415). Each assertion is a real HTTP request to the
//! bound ephemeral `127.0.0.1:0` port, checking the status / `Content-Type` / payload shape.
//! See [`sparq_conformance::http_protocol`] for the per-assertion catalogue.
//!
//! ## Honest floor + the documented-divergence remainder
//!
//! `HTTP_PROTOCOL_FLOOR` is the MEASURED count of PASS assertions, NOT an aspirational target;
//! MIRRORED in `scoreboard::SUITES` and read textually by `tests/scoreboard_floors.rs`. It may
//! only RISE. sq-406acc: a present-but-unsatisfiable `Accept` now genuinely returns
//! 406 (Oxigraph parity) — a PASS, raising the floor 20→21. The remaining documented divergences
//! (an absent / `*/*` Accept defaults to JSON per the W3C-permitted default; an ASK boolean in
//! CSV/TSV falls back to a JSON boolean) are recorded as DOCUMENTED DIVERGENCES and are NOT summed
//! into the floor, so a documented gap can never inflate the conformance number; a genuine FAILURE
//! always fails the gate.
//!
//! ## Feature gating (both states)
//!
//! Behind this crate's opt-in `http-protocol` feature (forwards to `service-loopback` → tokio +
//! axum via `sparq-server/server`; NO new third-party dep — the raw HTTP client is a std-only
//! `TcpStream` helper). With the feature OFF this file compiles to a single self-SKIP `#[test]`
//! (no tokio/axum linked here — the lean opt-in posture).
//!
//!   cargo test -p sparq-conformance --features http-protocol --test http_protocol_suite

#[cfg(not(feature = "http-protocol"))]
#[test]
fn http_protocol_suite_skipped_without_feature() {
    eprintln!(
        "SKIP: the W3C SPARQL 1.1 Protocol (HTTP layer) conformance lane is OFF — build with \
         `--features http-protocol` to run it."
    );
}

#[cfg(feature = "http-protocol")]
mod gated {
    use sparq_conformance::http_protocol::{self, Outcome};

    /// The SPARQL-Protocol PASS FLOOR (the RATCHET). MEASURED count of PASS assertions the
    /// suite makes against the real in-process loopback server; MIRRORED in the central
    /// scoreboard (`scoreboard::SUITES`) and read textually by the guard test
    /// `tests/scoreboard_floors.rs`. It may only RISE — never lower it. This is the ACTUAL
    /// current pass count, not an aspirational target. Documented protocol DIVERGENCES (e.g.
    /// sparq's absent-Accept JSON default) are reported separately and NOT counted here.
    /// sq-406acc: raised 20→21 when the present-but-unsatisfiable-Accept 406 (Oxigraph
    /// parity, w3c/sparql-protocol#40) flipped from a documented divergence to a genuine PASS.
    pub const HTTP_PROTOCOL_FLOOR: usize = 21;

    #[test]
    fn http_protocol_ratchet() {
        let report = http_protocol::run_protocol_suite();
        let pass = report.pass();
        let div = report.divergences();
        let failures = report.failures();

        // The CI ratchet greps this exact `TOTAL http-protocol` line (pass count in
        // field $3), mirroring the service / JSON-LD / d-entail lanes' `TOTAL <cat>` shape.
        // Positional `format!`/`println!` args (not inline `{pass}`) per the CodeQL
        // `rust/unused-variable` false-positive guard in the shared agent contract.
        println!("\nW3C SPARQL 1.1 Protocol (HTTP layer) conformance (in-process loopback server)");
        println!(
            "TOTAL http-protocol {} {} {} (floor {})",
            pass,
            failures.len(),
            div,
            HTTP_PROTOCOL_FLOOR
        );
        println!("documented protocol divergences (NOT summed into the floor):");
        for (label, outcome) in &report.outcomes {
            if let Outcome::Divergence(why) = outcome {
                println!("  ~ {}: {}", label, why);
            }
        }
        if !failures.is_empty() {
            println!("http-protocol FAILS:");
            for (label, why) in &failures {
                println!("  - {}: {}", label, why);
            }
        }

        assert!(
            failures.is_empty(),
            "the SPARQL-Protocol lane has failing assertions: {:?}",
            failures
        );
        assert!(
            pass >= HTTP_PROTOCOL_FLOOR,
            "SPARQL-Protocol pass count {} regressed below the ratchet floor {}",
            pass,
            HTTP_PROTOCOL_FLOOR
        );
    }
}
