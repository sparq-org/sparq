// sq-7agop — headless wasm32 smoke test of the materializer.
//
// `materialize_wac` / `materialize_acp` used to call `std::time::Instant::now()`
// unconditionally for `stats.millis`. `Instant` is unusable on
// `wasm32-unknown-unknown` (no monotonic clock — `Instant::now()` PANICS), so the
// crate compiled to wasm32 but TRAPPED at runtime the moment either function ran.
//
// The timing is now `cfg`-gated off on wasm32 (`stats.millis == 0.0` there). This
// module is the regression guard: it drives the REAL materialize path in a genuine
// wasm32 runtime via `wasm-pack test --node` and asserts it returns instead of
// trapping. The native `#[cfg(test)]` suite never catches this — `Instant` is fine
// natively — so the bug is only observable in an actual wasm runtime.
//
// Run with: `wasm-pack test --node -p sparq-solid`. Node is the default executor, so
// no `wasm_bindgen_test_configure!(run_in_browser)` directive is needed.

#![cfg(target_arch = "wasm32")]

use sparq_solid::{materialize_acp, materialize_wac, AUTH_GRAPH};
use wasm_bindgen_test::*;

// A single .acl that grants alice Read on the pod root (mirrors the `materialize_wac`
// doctest fixture).
const WAC_NQUADS: &str = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
<https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
"#;

// One ACR granting alice Read on all members of the pod root (mirrors the
// `materialize_acp` doctest fixture).
const ACP_NQUADS: &str = r#"
<https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
<https://pod.ex/.acr> <http://www.w3.org/ns/solid/acp#memberAccessControl> <https://pod.ex/.acr#c> <https://pod.ex/.acr> .
<https://pod.ex/.acr#c> <http://www.w3.org/ns/solid/acp#apply> <https://pod.ex/.acr#pol> <https://pod.ex/.acr> .
<https://pod.ex/.acr#pol> <http://www.w3.org/ns/solid/acp#allow> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acr> .
<https://pod.ex/.acr#pol> <http://www.w3.org/ns/solid/acp#allOf> <https://pod.ex/.acr#m> <https://pod.ex/.acr> .
<https://pod.ex/.acr#m> <http://www.w3.org/ns/solid/acp#agent> <https://alice.ex/card#me> <https://pod.ex/.acr> .
"#;

/// WAC materialize runs to completion in wasm32 (it used to trap on `Instant::now()`),
/// installs the auth view, and reports `millis == 0.0` (timing is cfg-gated off here).
#[wasm_bindgen_test]
fn wac_materialize_does_not_trap_on_wasm32() {
    let mut graph =
        sparq_core::Graph::load_dataset(WAC_NQUADS, "nquads").expect("nquads must load in wasm");
    let stats = materialize_wac(&mut graph).expect("materialize_wac must not trap on wasm32");
    assert!(stats.auth_triples > 0, "auth view must be non-empty");
    assert_eq!(stats.strata_facts.len(), 1, "WAC is a single stratum");
    // Timing is unavailable on wasm32 (Instant cfg-gated off) — reported as 0.0.
    assert_eq!(stats.millis, 0.0, "wasm32 reports no wall-clock timing");
    // The view is installed as an ordinary named graph.
    assert!(
        graph.named().iter().any(|(name, _)| matches!(
            name, oxrdf::Term::NamedNode(n) if n.as_str() == AUTH_GRAPH)),
        "auth graph must be installed"
    );
}

/// ACP materialize (three stratified reasoning calls) also runs to completion in wasm32
/// and reports `millis == 0.0`.
#[wasm_bindgen_test]
fn acp_materialize_does_not_trap_on_wasm32() {
    let mut graph =
        sparq_core::Graph::load_dataset(ACP_NQUADS, "nquads").expect("nquads must load in wasm");
    let stats = materialize_acp(&mut graph).expect("materialize_acp must not trap on wasm32");
    assert!(stats.auth_triples > 0, "auth view must be non-empty");
    assert_eq!(stats.strata_facts.len(), 3, "ACP runs three strata");
    assert_eq!(stats.millis, 0.0, "wasm32 reports no wall-clock timing");
}
