// [GPT-6] Synthetic, NONcanonical stage-13 experiment; not external security assurance.
//! Run with a strict experiment JSON and a new output directory. No real wallet input.
use oxrdf::{Literal, NamedNode, Term, Triple};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sparq_zk::{
    commit::commit_triples,
    field::{field_to_hex, Fr},
    sig::{self, SecretKey},
};
use sparq_zk_compose::{
    driver::CircuitProver,
    manifest::{DisclosedTerm, StatusListSnapshot},
    result::{
        prepare_result_with_options, verify_result, PrivateIntegerCapacity, ResultCredential,
        ResultError, ResultOptions, ResultPolicy, ResultPresentation, ResultWork, WitnessSelection,
    },
    verifier::{InMemorySeenNonces, VerifierNonce},
};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::Instant,
};

type Fallible<T> = Result<T, Box<dyn Error>>;
const QUERY: &str =
    "SELECT DISTINCT ?name WHERE { ?s <urn:name> ?name . ?s <urn:age> ?age . FILTER(?age >= 18) }";
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Contract {
    meaning: String,
    exact_dataset_scope: String,
    query: String,
    released_terms: Vec<BTreeMap<String, String>>,
    authority: String,
    signature_suite: String,
    status_regime: String,
    disclosure_regime: String,
    capacity_policy: String,
}
impl Contract {
    fn synthetic() -> Self {
        Self {
            meaning: "selected_successful_support_unsigned_v1".into(),
            exact_dataset_scope: "not_applicable_no_completeness_claim".into(),
            query: QUERY.into(),
            released_terms: vec![BTreeMap::from([("name".into(), "\"Alice\"".into())])],
            authority: "synthetic_babyjubjub_issuer_seed_1".into(),
            signature_suite: "sparq_schnorr_babyjubjub_poseidon2".into(),
            status_regime: "verifier_owned_snapshot_epoch_7_hidden_reference".into(),
            capacity_policy: "k1_or_k2_smallest_p3_r4_n16_tiny_integer_status_d10".into(),
            disclosure_regime: "public_query_result_issuer_capacity_private_roots_and_operands"
                .into(),
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Planner {
    FirstSuccess,
    Optimize,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Experiment {
    schema_version: u32,
    fixture: String,
    contract: Contract,
    planners: Vec<Planner>,
    warmup_runs_per_planner: u32,
    measured_runs_per_planner: u32,
}
impl Experiment {
    fn validate(&self) -> Fallible<()> {
        if self.schema_version != 1
            || self.fixture != "synthetic_shared_alternative_v1"
            || self.contract != Contract::synthetic()
        {
            return Err(
                "unsupported fixture, contract or signature/status/disclosure regime".into(),
            );
        }
        if self.planners.is_empty()
            || self.planners.len() > 2
            || (self.planners.len() == 2 && self.planners[0] == self.planners[1])
            || self.warmup_runs_per_planner > 2
            || !(1..=5).contains(&self.measured_runs_per_planner)
        {
            return Err("invalid planner inventory or run budget".into());
        }
        Ok(())
    }
}

struct Fixture {
    credentials: Vec<ResultCredential>,
    policy: ResultPolicy,
    rows: Vec<BTreeMap<String, Term>>,
}
fn fixture() -> Fallible<Fixture> {
    let iri = |s: &str| NamedNode::new(s.to_owned());
    let mut names = Vec::new();
    let mut ages = Vec::new();
    // Carla is another successful answer deliberately withheld: selected support is not completeness.
    for (person, name, age) in [
        ("urn:alice", "Alice", 42),
        ("urn:bob", "Bob", 12),
        ("urn:carla", "Carla", 50),
    ] {
        names.push(Triple::new(
            iri(person)?,
            iri("urn:name")?,
            Literal::new_simple_literal(name),
        ));
        ages.push(Triple::new(
            iri(person)?,
            iri("urn:age")?,
            Literal::new_typed_literal(
                age.to_string(),
                iri("http://www.w3.org/2001/XMLSchema#integer")?,
            ),
        ));
    }
    let all = names.iter().chain(&ages).cloned().collect();
    let sk = SecretKey::from_seed(1);
    let mut credentials = Vec::new();
    for (i, triples) in [names, ages, all].into_iter().enumerate() {
        let graph = commit_triples(&triples, Fr::from(100 + i as u64))?;
        let status = sig::status_ref_digest(
            &sig::status_list_id_to_field("urn:synthetic:status"),
            i as u64,
            7,
        );
        let msg = sig::commitment_message_with_status(&graph.commitment, &graph.salt, &status);
        credentials.push(ResultCredential {
            graph,
            issuer: sk.public_key(),
            signature: sig::sign_deterministic(&sk, &msg),
            status_list: "urn:synthetic:status".into(),
            status_version: 7,
            status_index: i as u64,
        });
    }
    Ok(Fixture {
        credentials,
        policy: ResultPolicy {
            trusted_issuers: vec![sk.public_key()],
            snapshots: vec![StatusListSnapshot {
                status_list: "urn:synthetic:status".into(),
                version: 7,
                bits: vec![0; 128],
            }],
            min_version: 7,
            max_version: 7,
        },
        rows: vec![BTreeMap::from([(
            "name".into(),
            Term::Literal(Literal::new_simple_literal("Alice")),
        )])],
    })
}
fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
fn json_hash(value: &Value) -> Fallible<String> {
    Ok(hash(&serde_json::to_vec(value)?))
}
fn fixture_binding(f: &Fixture) -> Value {
    json!({"wallet": f.credentials.iter().map(|c| json!({
        "canonical_rdf": c.graph.canonical.lines, "commitment": field_to_hex(&c.graph.commitment),
        "salt": field_to_hex(&c.graph.salt), "issuer": sig::public_key_to_hex(&c.issuer),
        "signature": sig::signature_to_hex(&c.signature), "status_list": c.status_list,
        "status_version": c.status_version, "status_index": c.status_index
    })).collect::<Vec<_>>(), "policy": {"issuers": f.policy.trusted_issuers.iter().map(sig::public_key_to_hex).collect::<Vec<_>>(),
        "snapshots": f.policy.snapshots, "min_version": f.policy.min_version, "max_version": f.policy.max_version},
        "released_terms": f.rows.iter().map(|r| r.iter().map(|(k,v)|(k.clone(),v.to_string())).collect::<BTreeMap<_,_>>()).collect::<Vec<_>>()})
}
fn work(w: &ResultWork) -> Value {
    json!({"selected_credentials":w.selected_credentials,"shared_memberships":w.shared_memberships,
        "witness_uses":w.witness_uses,"public_predicates":w.public_predicates,"private_predicates":w.private_predicates,
        "signature_checks":w.signature_checks,"optimization":w.optimization.map(|v|format!("{v:?}"))})
}
fn command(tool: &str, args: &[&str], cwd: &Path) -> Fallible<String> {
    let out = Command::new(tool).args(args).current_dir(cwd).output()?;
    if !out.status.success() {
        return Err(format!("{tool} failed: {}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}
fn pinned_version(tool: &str, output: &str, expected: &str) -> bool {
    match tool {
        "nargo" => output
            .lines()
            .next()
            .is_some_and(|line| line == format!("nargo version = {expected}")),
        "bb" => output.trim() == expected,
        _ => false,
    }
}
fn tool_identity(tool: &str, expected: &str, root: &Path) -> Fallible<Value> {
    // This driver is Unix-only. Match executable lookup before hashing the same file.
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let found = std::env::split_paths(&std::env::var_os("PATH").ok_or("missing PATH")?)
        .map(|p| p.join(tool))
        .find(|p| {
            #[cfg(unix)]
            {
                fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            }
            #[cfg(not(unix))]
            {
                let _ = p;
                false
            }
        })
        .ok_or("executable tool not found on PATH or unsupported platform")?
        .canonicalize()?;
    let version = command(
        found.to_str().ok_or("non-UTF-8 tool path")?,
        &["--version"],
        root,
    )?;
    // Match the tool's actual version field; a later diagnostic cannot rescue a near match.
    if !pinned_version(tool, &version, expected) {
        return Err(format!("unsupported {tool} version: {version}").into());
    }
    Ok(json!({"version_output":version,"binary_blake3":hash(&fs::read(found)?)}))
}
fn source_identity(root: &Path) -> Fallible<Value> {
    let head = command("git", &["rev-parse", "HEAD"], root)?;
    let status = command(
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
        root,
    )?;
    if !status.is_empty() {
        return Err(
            "benchmark source must be committed and clean; put outputs outside source".into(),
        );
    }
    Ok(
        json!({"git_commit":head,"tree":command("git", &["rev-parse","HEAD^{tree}"],root)?,
        "cargo_lock_blake3":hash(&fs::read(root.join("Cargo.lock"))?),
        "rustc":command("rustc", &["-Vv"],root)?,
        "adapter_binary_blake3":hash(&fs::read(std::env::current_exe()?)?),
        "build_profile":if cfg!(debug_assertions){"debug_noncanonical"}else{"release_noncanonical"},
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "available_parallelism":std::thread::available_parallelism()?.get()}),
    )
}
fn stage<T>(record: &mut Value, name: &str, run: impl FnOnce() -> Fallible<T>) -> Fallible<T> {
    eprintln!("stage: {name}");
    let start = Instant::now();
    let result = run();
    record["stages"][name] = json!({"seconds":start.elapsed().as_secs_f64(),"status":if result.is_ok(){"success"}else{"failure"},
        "error":result.as_ref().err().map(ToString::to_string)});
    result
}
fn rejection(
    result: Result<sparq_zk_compose::result::VerifiedResult, ResultError>,
    expected: &str,
) -> Fallible<Value> {
    match result {
        Err(ResultError::Rejected(s)) if s == expected => {
            Ok(json!({"status":"rejected_as_expected","reason":s}))
        }
        Err(e) => Err(format!("tamper test had unrelated error: {e}").into()),
        Ok(_) => Err("tampered presentation was accepted".into()),
    }
}
fn controls(
    p: &ResultPresentation,
    fixture: &Fixture,
    nonce: &VerifierNonce,
    prover: &CircuitProver,
    out: &Path,
) -> Fallible<Value> {
    let verify = |p: &ResultPresentation, n: &VerifierNonce| {
        verify_result(
            QUERY,
            p,
            &fixture.policy,
            n,
            &InMemorySeenNonces::new(),
            prover,
            out,
        )
    };
    let mut proof = p.clone();
    proof.proof[0] ^= 1;
    let proof = rejection(verify(&proof, nonce), "cryptographic proof rejected")?;
    let mut rows = p.clone();
    rows.rows[0].insert(
        "name".into(),
        DisclosedTerm::Literal {
            value: "Mallory".into(),
            datatype: None,
            language: None,
        },
    );
    let rows = rejection(verify(&rows, nonce), "cryptographic proof rejected")?;
    // Change BOTH sides of the challenge, so a host equality guard cannot satisfy this control.
    let n = VerifierNonce::from_field(Fr::from(999999u64));
    let mut challenge = p.clone();
    challenge.challenge = n.as_field_hex();
    let challenge = rejection(verify(&challenge, &n), "cryptographic proof rejected")?;
    let mut query = p.clone();
    query.query = QUERY.replace(">= 18", ">= 19");
    let query = rejection(
        verify(&query, nonce),
        "query differs from relying-party request",
    )?;
    let mut policy = fixture.policy.clone();
    policy.snapshots[0].bits.fill(255);
    let status = rejection(
        verify_result(
            QUERY,
            p,
            &policy,
            nonce,
            &InMemorySeenNonces::new(),
            prover,
            out,
        ),
        "cryptographic proof rejected",
    )?;
    Ok(
        json!({"proof_bytes":proof,"released_term":rows,"challenge_both_sides":challenge,"requested_query":query,"accepted_status_snapshot":status}),
    )
}
fn run_one(
    planner: Planner,
    f: &Fixture,
    nonce_id: u64,
    prover: &CircuitProver,
    out: &Path,
    controls_required: bool,
    record: &mut Value,
) -> Fallible<()> {
    let nonce = VerifierNonce::from_field(Fr::from(nonce_id));
    let options = ResultOptions {
        witness_selection: match planner {
            Planner::FirstSuccess => WitnessSelection::FirstSuccess,
            Planner::Optimize => WitnessSelection::Optimize,
        },
        ..ResultOptions::default()
    };
    let prepared = stage(record, "prepare_api", || {
        Ok(prepare_result_with_options(
            QUERY,
            &f.credentials,
            &f.rows,
            &f.policy,
            &nonce,
            options,
        )?)
    })?;
    record["work"] = work(prepared.work());
    let p = driver_stage(record, "prove_api_inclusive", prover, || {
        Ok(prepared.prove(prover, out, "experiment")?)
    })?;
    require_driver_inventory(
        record,
        "prove_api_inclusive",
        &[
            "nargo_cache_lock",
            "nargo_compile",
            "acir_private_copy",
            "nargo_cache_lock",
            "nargo_execute",
            "bb_prove_and_write_vk",
        ],
    )?;
    if p.version != 1 || p.integer_capacity != PrivateIntegerCapacity::TwoDigits {
        return Err("result numeric contract/capacity drift".into());
    }
    if p.proof.is_empty() {
        return Err("empty proof".into());
    }
    let seen = InMemorySeenNonces::new();
    let verified = driver_stage(record, "verify_api_inclusive", prover, || {
        Ok(verify_result(
            QUERY, &p, &f.policy, &nonce, &seen, prover, out,
        )?)
    })?;
    require_driver_inventory(
        record,
        "verify_api_inclusive",
        &[
            "nargo_cache_lock",
            "nargo_compile",
            "acir_private_copy",
            "bb_write_vk",
            "bb_verify",
        ],
    )?;
    if verified.rows != f.rows || verified.query != QUERY {
        return Err("accepted result/contract drift".into());
    }
    let replay = driver_stage(
        record,
        "replay_control_excluded_from_timings",
        prover,
        || {
            rejection(
                verify_result(QUERY, &p, &f.policy, &nonce, &seen, prover, out),
                "challenge already consumed",
            )
        },
    )?;
    let mut public = serde_json::to_value(&p)?;
    public
        .as_object_mut()
        .ok_or("presentation object")?
        .remove("proof");
    let public_bytes = serde_json::to_vec(&public)?;
    let wire = serde_json::to_vec(&p)?;
    fs::write(out.join("proof.bin"), &p.proof)?;
    fs::write(out.join("public.json"), &public_bytes)?;
    fs::write(out.join("presentation.json"), &wire)?;
    record["artifacts"] = json!({"cryptographic_proof_bytes":p.proof.len(),"proof_blake3":hash(&p.proof),
        "public_disclosure_json_bytes":public_bytes.len(),"public_disclosure_blake3":hash(&public_bytes),
        "presentation_json_bytes":wire.len(),"presentation_blake3":hash(&wire),"public_transcript":public,
        "verification_key":{"value":null,"reason":"independently derived by verifier API; not exported"}});
    record["replay_control"] = replay;
    if controls_required {
        record["tamper_controls"] = driver_stage(
            record,
            "tamper_controls_excluded_from_timings",
            prover,
            || controls(&p, f, &nonce, prover, out),
        )?;
    }
    record["status"] = json!("success");
    Ok(())
}

// [GPT-6] Preserve measurements on failures too. Driver events are nested inside
// an inclusive API span, so adding the two levels would double count work.
fn driver_stage<T>(
    record: &mut Value,
    name: &str,
    prover: &CircuitProver,
    run: impl FnOnce() -> Fallible<T>,
) -> Fallible<T> {
    let result = stage(record, name, run);
    let metrics = prover
        .take_stage_metrics()
        .ok_or("driver collection not enabled")?;
    let complete = metrics.complete;
    record["stages"][name]["driver"] = serde_json::to_value(metrics)?;
    let value = result?;
    if !complete {
        return Err("driver measurements incomplete; cannot record successful measurement".into());
    }
    Ok(value)
}

fn require_driver_inventory(record: &Value, scope: &str, expected: &[&str]) -> Fallible<()> {
    let events = record["stages"][scope]["driver"]["events"]
        .as_array()
        .ok_or("driver event array missing")?;
    if events.len() != expected.len()
        || events.iter().zip(expected).any(|(event, stage)| {
            event["stage"] != *stage
                || event["outcome"] != "success"
                || !event["acir_cache"].is_null()
                || !event["elapsed_seconds"]
                    .as_f64()
                    .is_some_and(|s| s.is_finite() && s >= 0.0)
        })
    {
        return Err("driver stage inventory differs from this fixed adapter".into());
    }
    Ok(())
}
fn read_manifest(path: &Path) -> Fallible<Experiment> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("manifest exceeds byte limit".into());
    }
    let exp: Experiment = serde_json::from_slice(&bytes)?;
    exp.validate()?;
    Ok(exp)
}
fn execute(exp: &Experiment, out: &Path) -> Fallible<bool> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let source = source_identity(&root)?;
    let parent = out
        .parent()
        .ok_or("output needs a parent directory")?
        .canonicalize()?;
    if parent.starts_with(&root) {
        return Err("output must be outside the source checkout".into());
    }
    fs::create_dir(out)?; // Refuse overwriting or merging prior measurements.
    let mut report = json!({"schema_version":2,"canonical":false,"status":"failure","experiment":exp,"source":source,
        "backend":{"name":"barretenberg","target":"noir-recursive","zk_mode_requested":true},
        "signature_suites_unavailable":["BBS+","ECDSA","EdDSA"],
        "cache_contract":{"os_cache":"uncontrolled","nargo_dependency_cache":"uncontrolled","rust_build":"outside timing",
            "warmup_definition":"completed full prepare/prove/verify runs per planner; no OS cold-cache assertion",
            "nargo_internal_cache_hit":{"value":null,"reason":"Nargo does not expose reliable per-invocation cache-hit telemetry"},
            "nonce_convention":"distinct deterministic test nonce per attempted run; excluded only from semantic equivalence digest and retained in each transcript"},
        "timing_contract":"driver events are measured child spans of inclusive API timers, not additive extra stages; uninstrumented host I/O/setup remains in inclusive timers",
        "runs":[],"unavailable_stages":{
            "backend_prove_excluding_key_seconds":{"value":null,"reason":"bb prove --write_vk is measured as one subprocess; internal proof/key split unavailable"},
            "peak_rss_bytes":{"value":null,"reason":"host/subprocess RSS not instrumented"}}});
    let outcome = (|| -> Fallible<()> {
        report["backend"]["nargo"] = tool_identity("nargo", "1.0.0-beta.21", &root)?;
        report["backend"]["bb"] = tool_identity("bb", "5.0.0-nightly.20260324", &root)?;
        let start = Instant::now();
        let f = fixture()?;
        report["fixture_setup_seconds"] = json!(start.elapsed().as_secs_f64());
        let binding = fixture_binding(&f);
        report["synthetic_input_binding"] = binding.clone();
        let contract = json!({"acceptance":exp.contract,"synthetic_inputs":binding});
        let digest = json_hash(&contract)?;
        report["acceptance_contract_blake3"] = json!(digest);
        let prover = CircuitProver::new(root.join("zk/compose")).with_stage_metrics();
        let mut ordinal = 0u64;
        for &planner in &exp.planners {
            let mut completed_warmups = 0;
            for i in 0..exp.warmup_runs_per_planner + exp.measured_runs_per_planner {
                ordinal += 1;
                let warmup = i < exp.warmup_runs_per_planner;
                let dir = out.join(format!("run-{ordinal}"));
                fs::create_dir(&dir)?;
                let mut r = json!({"ordinal":ordinal,"planner":planner,"role":if warmup{"warmup"}else{"measurement"},
                    "completed_warmups":completed_warmups,"prior_runs_in_process":ordinal-1,"status":"failure",
                    "acceptance_contract_blake3":digest,"stages":{}});
                let result = run_one(
                    planner,
                    &f,
                    420000 + ordinal,
                    &prover,
                    &dir,
                    !warmup && i == exp.warmup_runs_per_planner,
                    &mut r,
                );
                if let Err(ref e) = result {
                    r["error"] = json!(e.to_string());
                }
                report["runs"].as_array_mut().ok_or("run array")?.push(r);
                result?;
                // A supposedly controlled ablation cannot mutate its inputs or acceptance policy.
                if json_hash(
                    &json!({"acceptance":exp.contract,"synthetic_inputs":fixture_binding(&f)}),
                )? != digest
                {
                    return Err("ablation changed acceptance contract".into());
                }
                if warmup {
                    completed_warmups += 1;
                }
            }
        }
        report["status"] = json!("success");
        Ok(())
    })();
    if let Err(e) = outcome.as_ref() {
        report["error"] = json!(e.to_string());
    }
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    Ok(outcome.is_ok())
}
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = (|| -> Fallible<bool> {
        if args.len() != 2 {
            return Err("usage: result_experiment <experiment.json> <new-output-directory>".into());
        }
        execute(
            &read_manifest(Path::new(&args[0]))?,
            &PathBuf::from(&args[1]),
        )
    })();
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("experiment rejected: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_failed_driver_events_cannot_be_successful_measurements() {
        let mut record = json!({"stages":{"verify":{"driver":{"events":[{
            "stage":"bb_verify","outcome":"success","acir_cache":null,"elapsed_seconds":0.001
        }]}}}});
        require_driver_inventory(&record, "verify", &["bb_verify"]).unwrap();
        record["stages"]["verify"]["driver"]["events"][0]["outcome"] = json!("failed");
        assert!(require_driver_inventory(&record, "verify", &["bb_verify"]).is_err());
        record["stages"]["verify"]["driver"]["events"] = json!([]);
        assert!(require_driver_inventory(&record, "verify", &["bb_verify"]).is_err());
    }
    #[test]
    fn version_fields_reject_near_matches_and_diagnostic_rescue() {
        let n = "1.0.0-beta.21";
        let b = "5.0.0-nightly.20260324";
        assert!(pinned_version(
            "nargo",
            &format!("nargo version = {n}\nnoirc version = {n}+hash"),
            n
        ));
        assert!(pinned_version("bb", b, b));
        for suffix in ["0", "-other", "+other"] {
            assert!(!pinned_version("bb", &format!("{b}{suffix}"), b));
            assert!(!pinned_version(
                "nargo",
                &format!("nargo version = {n}{suffix}\n{n}"),
                n
            ));
        }
        assert!(!pinned_version("bb", &format!("bad\n{b}"), b));
    }
    fn config() -> Experiment {
        serde_json::from_str(include_str!(
            "../../../bench/zk-compose/experiments/synthetic-selected-support.json"
        ))
        .unwrap()
    }
    #[test]
    fn contract_mutations_cannot_masquerade_as_ablations() {
        let valid = config();
        valid.validate().unwrap();
        for field in [
            "meaning",
            "exact_dataset_scope",
            "query",
            "released_terms",
            "authority",
            "signature_suite",
            "status_regime",
            "disclosure_regime",
            "capacity_policy",
        ] {
            let mut value = serde_json::to_value(&valid).unwrap();
            if field == "released_terms" {
                value["contract"][field] = json!([]);
            } else {
                value["contract"][field] = json!("changed");
            }
            assert!(
                serde_json::from_value::<Experiment>(value)
                    .unwrap()
                    .validate()
                    .is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn manifest_is_strict_and_runs_are_bounded() {
        let mut value = serde_json::to_value(config()).unwrap();
        value["extra"] = json!(1);
        assert!(serde_json::from_value::<Experiment>(value).is_err());
        let mut e = config();
        e.measured_runs_per_planner = 0;
        assert!(e.validate().is_err());
        e = config();
        e.planners = vec![Planner::Optimize, Planner::Optimize];
        assert!(e.validate().is_err());
    }
    #[test]
    fn actual_planners_select_different_work_for_identical_released_support() {
        let f = fixture().unwrap();
        let n = VerifierNonce::from_field(Fr::from(42u64));
        let before = fixture_binding(&f);
        let mut opts = ResultOptions {
            witness_selection: WitnessSelection::FirstSuccess,
            ..ResultOptions::default()
        };
        let first =
            prepare_result_with_options(QUERY, &f.credentials, &f.rows, &f.policy, &n, opts)
                .unwrap();
        opts.witness_selection = WitnessSelection::Optimize;
        let optimized =
            prepare_result_with_options(QUERY, &f.credentials, &f.rows, &f.policy, &n, opts)
                .unwrap();
        assert_eq!(first.work().signature_checks, 2);
        assert_eq!(optimized.work().signature_checks, 1);
        assert_eq!(before, fixture_binding(&f));
    }
    #[test]
    fn backend_errors_do_not_count_as_rejected_tampers() {
        assert!(rejection(
            Err(ResultError::SearchExhausted),
            "cryptographic proof rejected"
        )
        .is_err());
        assert!(rejection(
            Err(ResultError::Rejected("unrelated".into())),
            "cryptographic proof rejected"
        )
        .is_err());
    }
}
