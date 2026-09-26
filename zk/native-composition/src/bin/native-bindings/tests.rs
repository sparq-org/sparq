// [GPT-6] Actual preparation/proof controls; no expected-outcome-driven acceptance.
use super::*;

fn job(tier: &str) -> Value {
    json!({"schema":"sparq.proof-binding-job.v1", "id":"1".repeat(64),
        "case_id":"native-adapter-test", "case_sha256":"2".repeat(64),
        "backend":"native_rdf", "tier":tier, "operation":"binding",
        "query":"SELECT DISTINCT ?s ?o WHERE { ?s <urn:p> ?o }",
        "triples":[["<urn:a>","<urn:p>","<urn:b>"]],
        "variables":["s","o"], "rows":[["<urn:a>","<urn:b>"]],
        "expected_accept":true, "nonce_id":1, "policy_overrides":{}})
}

fn run(job: &Value) -> Execution {
    execute(&serde_json::to_vec(job).unwrap()).unwrap()
}

#[test]
fn native_preparation_and_absent_candidates_ignore_expectation() {
    let mut input = job("native");
    input["expected_accept"] = json!(false);
    let valid = run(&input);
    assert_eq!(valid.outcome["observed"], "accepted");
    assert_eq!(valid.outcome["proof_count"], 0);
    assert_eq!(valid.outcome["verified_count"], 0);
    input["rows"][0][1] = json!("<urn:absent>");
    input["expected_accept"] = json!(true);
    let absent = run(&input);
    assert_eq!(absent.outcome["observed"], "rejected");
    assert_eq!(absent.outcome["stage"], "support");
    assert_eq!(absent.outcome["error_class"], "Support");
}

#[test]
fn real_proof_independent_verification_replay_and_absence() {
    let mut input = job("real");
    let actual = run(&input);
    assert_eq!(actual.outcome["observed"], "accepted");
    assert_eq!(actual.outcome["stage"], "proof_verifier");
    assert_eq!(actual.outcome["proof_count"], 1);
    assert_eq!(actual.outcome["verified_count"], 1);
    assert_eq!(actual.outcome["controls"][0]["error_class"], "Replay");
    assert!(actual
        .artifacts
        .iter()
        .any(|(name, bytes)| *name == "proof.bin" && bytes.starts_with(b"SPNRDF01")));
    input["rows"][0][0] = json!("<urn:absent>");
    let absent = run(&input);
    assert_eq!(absent.outcome["observed"], "rejected");
    assert_eq!(absent.outcome["stage"], "proof_verifier");
    assert_eq!(absent.outcome["error_class"], "Verification");
    assert_eq!(absent.outcome["proof_count"], 1);
    assert_eq!(absent.outcome["verified_count"], 0);
    assert_eq!(absent.outcome["controls"][0]["error_class"], "Support");
    assert_eq!(absent.outcome["controls"][1]["verified_count"], 1);
    assert_eq!(absent.outcome["controls"][2]["error_class"], "Verification");
    assert!(absent
        .artifacts
        .iter()
        .any(|(name, bytes)| *name == "weaker-proof.bin" && bytes.starts_with(b"SPNRDF01")));
    let public: Value = serde_json::from_slice(
        &absent
            .artifacts
            .iter()
            .find(|(name, _)| *name == "public.json")
            .unwrap()
            .1,
    )
    .unwrap();
    assert_eq!(public["context"]["query"], input["query"]);
    assert_eq!(public["context"]["rows"][0]["s"], "<urn:absent>");
    assert_eq!(
        public["nonce"],
        json!(nonce(&serde_json::to_vec(&input).unwrap()))
    );
}

#[test]
fn empty_graph_is_only_an_existing_admission_refusal() {
    let mut input = job("real");
    input["triples"] = json!([]);
    let actual = run(&input);
    assert_eq!(actual.outcome["observed"], "rejected");
    assert_eq!(actual.outcome["stage"], "admission");
    assert_eq!(actual.outcome["error_class"], "Capacity");
    assert!(actual.outcome["unsupported_reason"].is_string());
    assert_eq!(actual.outcome["proof_count"], 0);
}

#[test]
fn profile_boundaries_are_not_forgery_rejections() {
    for (field, value) in [
        ("query", json!("SELECT ?s ?o WHERE { ?s <urn:p> ?o }")),
        ("rows", json!([["<urn:a>", null]])),
        ("operation", json!("attack")),
        ("tier", json!("constraint")),
        ("authority", json!("holder_declared")),
        ("policy_overrides", json!({"max_rows":1})),
        ("triples", json!([["_:blank", "<urn:p>", "<urn:b>"]])),
    ] {
        let mut input = job("native");
        input[field] = value;
        let actual = run(&input);
        assert_eq!(actual.outcome["observed"], "unsupported", "{field}");
        assert_eq!(actual.outcome["proof_count"], 0, "{field}");
    }
}

#[test]
fn full_input_bytes_bind_nonce_and_hashes() {
    let original = serde_json::to_vec(&job("native")).unwrap();
    for field in ["query", "id", "case_sha256", "nonce_id", "rows", "triples"] {
        let mut changed = job("native");
        changed[field] = json!("changed");
        assert_ne!(
            nonce(&original),
            nonce(&serde_json::to_vec(&changed).unwrap())
        );
    }
    assert_eq!(
        hash(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn alternate_dataset_cannot_change_or_hide_graph_membership() {
    let mut input = job("native");
    let document = "<urn:a> <urn:p> <urn:b> .\n";
    input["dataset"] = json!({"ntriples":document,"nquads":document,"named_graphs":[]});
    assert_eq!(run(&input).outcome["observed"], "accepted");
    input["dataset"]["nquads"] = json!("<urn:a> <urn:p> <urn:other> .\n");
    assert!(execute(&serde_json::to_vec(&input).unwrap()).is_err());
    input["dataset"]["nquads"] = json!(document);
    input["dataset"]["named_graphs"] = json!(["urn:empty"]);
    assert_eq!(run(&input).outcome["observed"], "unsupported");
    input["dataset"]["named_graphs"] = json!([]);
    input["dataset"]["nquads"] = json!("<urn:a> <urn:p> <urn:b> <urn:graph> .\n");
    assert_eq!(run(&input).outcome["observed"], "unsupported");
}

#[test]
fn malformed_jobs_do_not_become_successful_negatives() {
    for (field, value) in [
        ("backend", json!("missing_backend")),
        ("schema", json!("wrong")),
        ("expected_accept", Value::Null),
        ("nonce_id", json!(-1)),
        ("id", json!("not-a-hash")),
        ("variables", json!(["s", "s"])),
        ("rows", json!([])),
        ("triples", json!([["<urn:a>", "<urn:p>"]])),
    ] {
        let mut input = job("real");
        input[field] = value;
        assert!(
            execute(&serde_json::to_vec(&input).unwrap()).is_err(),
            "{field}"
        );
    }
}
