//! sq-1uuxz (epic sq-my8wd) — the SPARQL 1.1 **Service-Description** (SD) +
//! **Graph-Store Protocol** (GSP) conformance lane, wired as a RATCHETED gate that mirrors the
//! SPARQL / SHACL / GeoSPARQL / Solid / JSON-LD / sparql11-service / http-protocol ratchets in
//! this crate (crate-local `cargo test` + a pinned pass-count FLOOR that may only RISE, registered
//! in the central `scoreboard::SUITES` and guarded by `tests/scoreboard_floors.rs`).
//!
//! ## What is gated
//!
//! Against the merged in-process loopback server (sq-ushvx, #1291) — stood up with the server's
//! `federation-descriptors` runtime flag ON so the `GET /sparql` (no query) Service-Description
//! endpoint is live — over RAW HTTP:
//!
//! * **(A) SERVICE DESCRIPTION** — the `sd:Service` document advertises exactly the result/input
//!   formats (SRJ/SRX/CSV/TSV + Turtle/N-Triples/RDF-XML), query/update languages
//!   (`sd:SPARQL11Query`/`Update` + the version-agnostic `sd:SPARQLQuery`/`Update`), SPARQL versions
//!   (`sd:supportedVersion` 1.0/1.1/1.2) and `sd:BasicFederatedQuery` that the server GENUINELY
//!   implements — checked positively (each advertised result format is cross-checked against a real
//!   SELECT request) AND negatively (JSON-LD, which this build does NOT serve, must NOT appear).
//! * **(B) GRAPH STORE PROTOCOL** — a GET/PUT/POST/DELETE round-trip on a named graph (indirect
//!   `?graph=<iri>` + direct `/graphs/<path>`) and the default graph (`?default`), VERIFYING store
//!   state after every op (PUT→GET-back-equal; PUT replaces; POST merges; DELETE removes;
//!   200/201/204/400/404/405/415).
//!
//! See [`sparq_conformance::sd_gsp`] for the per-assertion catalogue.
//!
//! ## Honest floor + the documented-divergence remainder
//!
//! `SD_GSP_FLOOR` is the MEASURED count of PASS assertions, NOT an aspirational target; MIRRORED in
//! `scoreboard::SUITES` and read textually by `tests/scoreboard_floors.rs`. It may only RISE. The
//! SD/GSP features sparq does NOT implement (a GSP read of an absent named graph is 200+empty, not
//! 404) are recorded as DOCUMENTED DIVERGENCES and are NOT summed into the floor, so a documented
//! gap can never inflate the conformance number; a genuine FAILURE always fails the gate.
//!
//! ## Feature gating (both states)
//!
//! Behind this crate's opt-in `federation-descriptors` feature (forwards to `service-loopback` →
//! tokio + axum via `sparq-server/server`, AND turns on `sparq-server/federation-descriptors`; NO
//! new third-party dep — the raw HTTP client is a std-only `TcpStream` helper). With the feature OFF
//! this file compiles to a single self-SKIP `#[test]` (no tokio/axum linked here — the lean opt-in
//! posture).
//!
//!   cargo test -p sparq-conformance --features federation-descriptors --test sd_gsp_suite

#[cfg(not(feature = "federation-descriptors"))]
#[test]
fn sd_gsp_suite_skipped_without_feature() {
    eprintln!(
        "SKIP: the SPARQL 1.1 Service-Description + Graph-Store-Protocol conformance lane is OFF — \
         build with `--features federation-descriptors` to run it."
    );
}

#[cfg(feature = "federation-descriptors")]
mod gated {
    use sparq_conformance::http_protocol::Outcome;
    use sparq_conformance::sd_gsp;

    /// The SD/GSP PASS FLOOR (the RATCHET). MEASURED count of PASS assertions the suite makes
    /// against the real in-process loopback server (with `federation-descriptors` on); MIRRORED in
    /// the central scoreboard (`scoreboard::SUITES`) and read textually by the guard test
    /// `tests/scoreboard_floors.rs`. It may only RISE — never lower it. This is the ACTUAL current
    /// pass count, not an aspirational target. Documented SD/GSP DIVERGENCES (e.g. a GSP read of an
    /// absent graph returning 200+empty rather than 404) are reported separately and NOT counted here.
    pub const SD_GSP_FLOOR: usize = 39;

    #[test]
    fn sd_gsp_ratchet() {
        let report = sd_gsp::run_sd_gsp_suite();
        let pass = report.pass();
        let div = report.divergences();
        let failures = report.failures();

        // The CI ratchet greps this exact `TOTAL sd-gsp` line (pass count in field $3),
        // mirroring the service / http-protocol / JSON-LD lanes' `TOTAL <cat>` shape. Positional
        // `format!`/`println!` args (not inline `{pass}`) per the CodeQL `rust/unused-variable`
        // false-positive guard in the shared agent contract.
        println!("\nSPARQL 1.1 Service-Description + Graph-Store-Protocol conformance (in-process loopback server)");
        println!(
            "TOTAL sd-gsp {} {} {} (floor {})",
            pass,
            failures.len(),
            div,
            SD_GSP_FLOOR
        );
        println!("documented SD/GSP divergences (NOT summed into the floor):");
        for (label, outcome) in &report.outcomes {
            if let Outcome::Divergence(why) = outcome {
                println!("  ~ {}: {}", label, why);
            }
        }
        if !failures.is_empty() {
            println!("sd-gsp FAILS:");
            for (label, why) in &failures {
                println!("  - {}: {}", label, why);
            }
        }

        assert!(
            failures.is_empty(),
            "the SD/GSP lane has failing assertions: {:?}",
            failures
        );
        assert!(
            pass >= SD_GSP_FLOOR,
            "SD/GSP pass count {} regressed below the ratchet floor {}",
            pass,
            SD_GSP_FLOOR
        );
    }
}
