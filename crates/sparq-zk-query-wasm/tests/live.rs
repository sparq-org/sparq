//! Research grade and not externally audited. Live package contracts.
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod support;
#[test]
fn node_split_prover_and_operator_agreement() {
    let Some(gate) = support::gate(
        "node_split_prover_and_operator_agreement",
        &["nargo", "bb", "node"],
    ) else {
        return;
    };
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../js/zk-query/scripts/live-tests.mjs");
    let directory = std::env::var_os("SPARQ_QUERY_TEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("sparq-query-live-evidence"));
    let output = support::live_tool(&gate, "node")
        .arg("--test")
        .arg(script)
        .env("SPARQ_QUERY_TEST_DIR", directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    support::complete(gate);
}
