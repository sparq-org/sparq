// Measurement harness for the ZK SPARQL paper: V5 issuer, holder and verifier
// costs per cryptosuite, signature mode, credential count and size, and query.
//! Runs the paper's query sets over synthetic credentials and writes one JSON
//! object per configuration to stdout.
//!
//! ```text
//! paper_sweep [--set main|sweep] [--execute] [--phases] [--prove N]
//!             [--only ID] [--suite eddsa|merkle] [--mode hidden|revealed]
//!             [--n N] [--size Q]
//! ```
//!
//! - Native evaluation always runs and records admission, the result size, and
//!   issuer and revealed-mode verifier timings.
//! - `--execute` runs the production V5 image in the executor (`RISC0_SERVER_PATH`)
//!   and records user and padded cycles and segments. These are deterministic.
//! - `--phases` (needs the `phase-cycles` feature) also runs the measurement image
//!   and records the cycles of each [`auth::Phase`].
//! - `--prove N` proves N times with dev mode off, verifies each receipt, and
//!   records proving and verification times and receipt and presentation sizes.
//!
//! Synthetic test material only: every credential is signed with the public
//! RFC 8032 section 7.1 TEST 1 key.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::authenticated_rdf::{
    check_query_profile, prove_with_artifact, verify_revealed_with_artifact, verify_with_artifact,
};
use sparq_proved_evaluator::{AcceptedGuest, Error, Nonces, embedded_authrdf_artifact, embedded_authrdf_pin};
use sparq_proved_evaluator_model::DatasetAuthority;
use sparq_proved_evaluator_model::authenticated_rdf::{
    self as auth, AuthorizedKey, Cryptosuite, Policy, PrivateCredentials, Request, SignatureMode, SignedCredential,
    Witness,
};
use sparq_proved_evaluator_model::merkle_suite;
use std::path::{Path, PathBuf};
use std::time::Instant;

const TEST1_SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const ISSUER: &str = "https://issuer.example/sweep";
const VM: &str = "https://issuer.example/sweep#key-1";
const PAYMENT_ISSUER: &str = "https://bank.example/issuers/1";
const PAYMENT_VM: &str =
    "did:key:z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw#z6MktwupdmLXVVqTzCw4i46r4uGyosGXRnR3XjN4Zq7oMMsw";
const SESSION_LIMIT: u64 = 1 << 25;
/// The V5 guest's abort text as the pinned SDK renders it.
const GUEST_ABORT: &str = "Guest panicked: bounded authenticated-RDF relation rejected";
const SEGMENT_LIMIT_PO2: u32 = 20;
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const CRED: &str = "https://www.w3.org/2018/credentials#";

struct Args {
    set: String,
    execute: bool,
    phases: bool,
    prove: usize,
    only: Option<String>,
    suite: Option<String>,
    mode: Option<String>,
    n: Option<usize>,
    size: Option<usize>,
    blank_free: bool,
}

fn args() -> Args {
    let mut args = Args {
        set: "main".into(),
        execute: false,
        phases: false,
        prove: 0,
        only: None,
        suite: None,
        mode: None,
        n: None,
        size: None,
        blank_free: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--set" => args.set = value(),
            "--execute" => args.execute = true,
            "--phases" => args.phases = true,
            "--prove" => args.prove = value().parse().expect("--prove N"),
            "--only" => args.only = Some(value()),
            "--suite" => args.suite = Some(value()),
            "--mode" => args.mode = Some(value()),
            "--n" => args.n = Some(value().parse().expect("--n N")),
            "--size" => args.size = Some(value().parse().expect("--size Q")),
            "--blank-free" => args.blank_free = true,
            other => panic!("unknown argument {other}"),
        }
    }
    assert!(!args.phases || cfg!(feature = "phase-cycles"), "--phases needs the phase-cycles feature");
    args
}

fn hex<const N: usize>(text: &str) -> [u8; N] {
    std::array::from_fn(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&hex(TEST1_SECRET))
}

fn proof_config(method: &str, suite: Cryptosuite) -> String {
    let sec = "https://w3id.org/security#";
    format!(
        "_:proof <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{sec}DataIntegrityProof> .\n\
         _:proof <{sec}cryptosuite> \"{}\"^^<{sec}cryptosuiteString> .\n\
         _:proof <{sec}verificationMethod> <{method}> .\n\
         _:proof <{sec}proofPurpose> <{sec}assertionMethod> .\n\
         _:proof <http://purl.org/dc/terms/created> \"2026-09-01T00:00:00Z\"^^<{XSD}dateTime> .\n",
        suite.name()
    )
}

/// Synthetic sweep credential `i` with `size` statements (at least the 26 core ones).
/// A sweep credential of `size` statements; `blank_free` names the holder with an
/// IRI instead of a blank node, the only blank node in the credential data.
fn sweep_document(i: usize, size: usize, blank_free: bool) -> String {
    let vc = format!("<urn:vc:sweep{i}>");
    let p = format!("<did:example:person{i}>");
    let holder = if blank_free { format!("<did:example:holder{i}>") } else { "_:holder".to_owned() };
    let mut lines = vec![
        format!("{vc} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{CRED}VerifiableCredential> ."),
        format!("{vc} <{CRED}issuer> <{ISSUER}> ."),
        format!("{vc} <{CRED}credentialSubject> {p} ."),
        format!("{vc} <{CRED}credentialSubject> {holder} ."),
        format!("{holder} <http://ex/nickname> \"Ali{i}\" ."),
        format!("{p} <http://ex/name> \"Alice{i}\" ."),
        format!("{p} <http://ex/label> \"Bonjour\"@fr ."),
        format!("{p} <http://ex/age> \"{}\"^^<{XSD}integer> .", 30 + i),
        format!("{p} <http://ex/balance> \"{}.50\"^^<{XSD}decimal> .", 1250 + i),
        format!("{p} <http://ex/score> \"0.75\"^^<{XSD}double> ."),
        format!("{p} <http://ex/since> \"2020-01-0{}T00:00:00Z\"^^<{XSD}dateTime> .", 1 + i % 9),
        format!("{p} <http://ex/active> \"true\"^^<{XSD}boolean> ."),
        format!("{p} <http://ex/knows> <did:example:friend{i}_0> ."),
    ];
    for k in 0..6 {
        lines.push(format!("<did:example:friend{i}_{k}> <http://ex/knows> <did:example:friend{i}_{}> .", k + 1));
    }
    for k in 0..7 {
        lines.push(format!("<did:example:friend{i}_{k}> <http://ex/name> \"Friend{i}_{k}\" ."));
    }
    let mut tag = 0;
    while lines.len() < size {
        lines.push(format!("{p} <http://ex/tag> \"t{tag}\" ."));
        tag += 1;
    }
    lines.join("\n") + "\n"
}

/// The payment credential behind Q1–Q5 for subject `i`; `i = 0` is the recorded fixture.
fn payment_document(i: usize) -> String {
    let subject = if i == 0 { "did:example:abcdefgh".to_owned() } else { format!("did:example:customer{i}") };
    let vc = if i == 0 { "urn:uuid:7d1c2b4e-6f0a-4c39-9a51-2b8e3f4d5a60".to_owned() } else { format!("urn:vc:payments{i}") };
    let bank = "https://bank.example/vocab#";
    let mut lines = Vec::new();
    for (month, amount) in [("06", "1250.00"), ("07", "1250.00"), ("08", "1310.50")] {
        let payment = if i == 0 {
            format!("https://bank.example/payments/2026-{month}")
        } else {
            format!("https://bank.example/payments/{i}/2026-{month}")
        };
        lines.push(format!("<{subject}> <{bank}payment> <{payment}> ."));
        lines.push(format!("<{payment}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{bank}Payment> ."));
        lines.push(format!("<{payment}> <{bank}amount> \"{amount}\"^^<{XSD}decimal> ."));
        lines.push(format!("<{payment}> <{bank}paymentStatus> <{bank}Settled> ."));
    }
    lines.push(format!("<{vc}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{bank}PaymentHistoryCredential> ."));
    lines.push(format!("<{vc}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{CRED}VerifiableCredential> ."));
    lines.push(format!("<{vc}> <{CRED}credentialSubject> <{subject}> ."));
    lines.push(format!("<{vc}> <{CRED}issuer> <{PAYMENT_ISSUER}> ."));
    lines.push(format!("<{vc}> <{CRED}validFrom> \"2026-09-01T00:00:00Z\"^^<{XSD}dateTime> ."));
    lines.join("\n") + "\n"
}

/// Issues one credential and returns it with the issuer's signing time in nanoseconds.
fn issue(document: &str, config: &str, suite: Cryptosuite, salt: [u8; 32]) -> (SignedCredential, u128) {
    let start = Instant::now();
    let credential = match suite {
        Cryptosuite::EddsaRdfc2022 => {
            let config_hash = Sha256::digest(sparq_canon::canonicalize_nquads(config).unwrap());
            let document_hash = Sha256::digest(sparq_canon::canonicalize_nquads(document).unwrap());
            let message = [config_hash.as_slice(), document_hash.as_slice()].concat();
            SignedCredential {
                document: document.into(),
                proof_config: config.into(),
                signature: key().sign(&message).to_bytes().to_vec(),
            }
        }
        Cryptosuite::EddsaSha256Merkle2026 => merkle_suite::issue(document, config, &key(), salt).unwrap(),
    };
    (credential, start.elapsed().as_nanos())
}

struct Config {
    suite: Cryptosuite,
    mode: SignatureMode,
    n: usize,
    size: usize,
}

fn suite_name(suite: Cryptosuite) -> &'static str {
    suite.name()
}

#[derive(Default)]
struct CountingNonces(Vec<[u8; 32]>);

impl Nonces for CountingNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        if self.0.contains(&nonce) {
            return Ok(false);
        }
        self.0.push(nonce);
        Ok(true)
    }
}

fn executor() -> ExternalProver {
    let r0vm = std::env::var_os("RISC0_SERVER_PATH").expect("RISC0_SERVER_PATH must name r0vm");
    ExternalProver::new("paper-sweep", PathBuf::from(r0vm))
}

fn input(witness: &Witness) -> Vec<u8> {
    risc0_zkvm::serde::to_vec(witness).unwrap().into_iter().flat_map(u32::to_le_bytes).collect()
}

fn execute(witness: &Witness, artifact: &[u8], stdout: Option<&mut Vec<u8>>) -> Result<Value, String> {
    let bytes = input(witness);
    let mut builder = ExecutorEnv::builder();
    builder.session_limit(Some(SESSION_LIMIT)).segment_limit_po2(SEGMENT_LIMIT_PO2).write_slice(&bytes);
    if let Some(out) = stdout {
        builder.stdout(out);
    }
    let env = builder.build().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let session = executor().execute(env, artifact).map_err(|e| format!("{e:#}"))?;
    let elapsed = start.elapsed().as_nanos();
    Ok(json!({
        "user_cycles": session.cycles(),
        "padded_cycles": session.segments.iter().map(|s| 1u64 << s.po2).sum::<u64>(),
        "segments": session.segments.len(),
        "journal_bytes": session.journal.bytes.len(),
        "witness_bytes": bytes.len(),
        "execute_ns": elapsed,
    }))
}

#[cfg(feature = "phase-cycles")]
fn phases(witness: &Witness) -> Result<Value, String> {
    let mut out = Vec::new();
    execute(witness, sparq_proved_evaluator_methods::SPARQ_AUTHRDF_PHASES_GUEST_ELF, Some(&mut out))?;
    let words: Vec<u32> = out.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let marks: Vec<(Option<auth::Phase>, u64)> = risc0_zkvm::serde::from_slice(&words).map_err(|e| e.to_string())?;
    let mut totals = serde_json::Map::new();
    let mut previous = marks.first().map(|m| m.1).unwrap_or(0);
    totals.insert("input".into(), json!(previous));
    for (phase, at) in marks.iter().skip(1) {
        let name = format!("{:?}", phase.expect("phase"));
        let entry = totals.entry(name).or_insert(json!(0));
        *entry = json!(entry.as_u64().unwrap() + (at - previous));
        previous = *at;
    }
    Ok(Value::Object(totals))
}

#[cfg(not(feature = "phase-cycles"))]
fn phases(_: &Witness) -> Result<Value, String> {
    unreachable!("checked in args()")
}

fn run(args: &Args, set: &str, id: &str, query: &str, config: &Config, guest: &AcceptedGuest) -> Value {
    let (issuer, vm) = if set == "main" { (PAYMENT_ISSUER, PAYMENT_VM) } else { (ISSUER, VM) };
    let config_text = proof_config(vm, config.suite);
    let mut credentials = Vec::new();
    let mut issue_ns = Vec::new();
    for i in 0..config.n {
        let document = if set == "main" { payment_document(i) } else { sweep_document(i, config.size, args.blank_free) };
        let (credential, ns) = issue(&document, &config_text, config.suite, [0x40 + i as u8; 32]);
        credentials.push(credential);
        issue_ns.push(ns);
    }
    let counts: Vec<usize> =
        credentials.iter().map(|c| c.document.lines().filter(|line| !line.trim().is_empty()).count()).collect();
    assert!(counts.windows(2).all(|pair| pair[0] == pair[1]), "credentials differ in size");
    let statements = counts[0];
    let credential_bytes: Vec<usize> = credentials.iter().map(|c| c.document.len() + c.proof_config.len()).collect();
    let mut presented = Vec::new();
    if config.mode == SignatureMode::Revealed {
        for credential in &mut credentials {
            presented.push(credential.signature.drain(..64).collect::<Vec<u8>>());
        }
    }
    let public = VerifyingKey::from(&key()).to_bytes();
    let policy = Policy::new(vec![AuthorizedKey {
        issuer: issuer.into(),
        verification_method: vm.into(),
        public_key: public,
    }])
    .with_cryptosuite(config.suite)
    .with_signature_mode(config.mode);
    let nonce: [u8; 32] = Sha256::digest(format!("{id}:{}:{:?}:{}:{}", suite_name(config.suite), config.mode, config.n, config.size)).into();
    let request = Request {
        version: auth::VERSION,
        query: query.into(),
        authority: DatasetAuthority::HolderDeclared,
        policy,
        nonce,
    };
    let witness = Witness {
        request: request.clone(),
        dataset: PrivateCredentials { credentials, salt: [0x5a; 32] },
    };
    let mut record = json!({
        "set": set,
        "id": id,
        "suite": suite_name(config.suite),
        "mode": config.mode.as_str(),
        "n": config.n,
        "statements_per_credential": statements,
        "credential_blank_nodes": set != "main" && !args.blank_free,
        "issuer_sign_ns": issue_ns,
        "credential_bytes": credential_bytes,
        "signature_bytes": 64,
    });
    // The host query profile check that proving and verification apply; the guest
    // is still executed below so the record also shows what the image itself does.
    let profile = check_query_profile(&request);
    record["query_profile"] = json!(profile.as_ref().map_or_else(|e| e.0, |()| "accepted"));
    let start = Instant::now();
    let native = auth::evaluate(&witness);
    record["native_evaluate_ns"] = json!(start.elapsed().as_nanos());
    let journal = match native {
        Ok(journal) => journal,
        Err(rejected) => {
            record["admitted"] = json!(false);
            record["rejection"] = json!(rejected.0);
            if args.execute {
                // Guest-level evidence: the production image must abort on it too.
                let outcome = execute(&witness, guest_artifact(), None);
                record["guest_aborted"] = json!(outcome.as_ref().is_err_and(|e| e.contains(GUEST_ABORT)));
                if let Ok(execution) = outcome {
                    record["execution"] = execution;
                }
            }
            return record;
        }
    };
    record["admitted"] = json!(true);
    record["result_bytes"] = json!(serde_json::to_vec(&journal.result).unwrap().len());
    if config.mode == SignatureMode::Revealed {
        // The holder presents signatures in the journal's credential order.
        let key = VerifyingKey::from_bytes(&public).unwrap();
        presented = journal
            .signed_messages
            .iter()
            .map(|entry| {
                presented
                    .iter()
                    .find(|s| key.verify(&entry.message, &ed25519_dalek::Signature::from_slice(s).unwrap()).is_ok())
                    .expect("one presented signature per signed message")
                    .clone()
            })
            .collect();
        let start = Instant::now();
        auth::check_revealed_signatures(&journal, &request, &presented).expect("revealed signatures verify");
        record["verifier_signature_check_ns"] = json!(start.elapsed().as_nanos());
        // The same check through the dalek API, as a verifier library would run it.
        let key = VerifyingKey::from_bytes(&public).unwrap();
        for (entry, signature) in journal.signed_messages.iter().zip(&presented) {
            let signature = ed25519_dalek::Signature::from_slice(signature).unwrap();
            key.verify(&entry.message, &signature).unwrap();
        }
    }
    if args.execute {
        record["execution"] = execute(&witness, guest_artifact(), None).unwrap_or_else(|e| json!({"error": e}));
    }
    if args.phases {
        record["phase_cycles"] = phases(&witness).unwrap_or_else(|e| json!({"error": e}));
    }
    if args.prove > 0 && profile.is_ok() {
        let r0vm = PathBuf::from(std::env::var_os("RISC0_SERVER_PATH").expect("RISC0_SERVER_PATH"));
        let mut runs = Vec::new();
        for attempt in 0..args.prove {
            let mut witness = witness.clone();
            witness.request.nonce[0] ^= attempt as u8 + 1;
            let start = Instant::now();
            let presentation = prove_with_artifact(&witness, Path::new(&r0vm), guest).expect("prove");
            let prove_ns = start.elapsed().as_nanos();
            let receipt_bytes = bincode_len(&presentation);
            let mut nonces = CountingNonces::default();
            let start = Instant::now();
            match config.mode {
                SignatureMode::Hidden => {
                    verify_with_artifact(&presentation, &witness.request, &mut nonces, guest).expect("verify");
                }
                SignatureMode::Revealed => {
                    verify_revealed_with_artifact(&presentation, &witness.request, &presented, &mut nonces, guest)
                        .expect("verify");
                }
            }
            runs.push(json!({
                "prove_ns": prove_ns,
                "verify_ns": start.elapsed().as_nanos(),
                "presentation_bytes": receipt_bytes + presented.iter().map(Vec::len).sum::<usize>(),
                "receipt_kind": "succinct",
            }));
        }
        record["proofs"] = json!(runs);
    }
    record
}

fn bincode_len(presentation: &sparq_proved_evaluator::Presentation) -> usize {
    serde_json::to_vec(&presentation.receipt).map(|b| b.len()).unwrap_or(0)
}

fn guest_artifact() -> &'static [u8] {
    embedded_authrdf_artifact()
}

fn main() {
    let args = args();
    let guest = AcceptedGuest::from_artifact(embedded_authrdf_artifact().to_vec(), &embedded_authrdf_pin())
        .expect("embedded V5 guest matches its pin");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/paper");
    let cases: Vec<(String, String)> = if args.set == "main" {
        let mut names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".rq"))
            .collect();
        names.sort();
        names
            .into_iter()
            .map(|n| (n.trim_end_matches(".rq").to_owned(), std::fs::read_to_string(root.join(&n)).unwrap()))
            .collect()
    } else {
        let sweep: Value = serde_json::from_str(&std::fs::read_to_string(root.join("sweep.json")).unwrap()).unwrap();
        sweep["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["id"].as_str().unwrap().to_owned(), c["query"].as_str().unwrap().to_owned()))
            .collect()
    };
    let suites = [("eddsa", Cryptosuite::EddsaRdfc2022), ("merkle", Cryptosuite::EddsaSha256Merkle2026)];
    let modes = [("hidden", SignatureMode::Hidden), ("revealed", SignatureMode::Revealed)];
    let (counts, sizes): (Vec<usize>, Vec<usize>) =
        if args.set == "main" { (vec![1, 4], vec![16]) } else { (vec![1, 4], vec![32, 64]) };
    for (id, query) in &cases {
        if args.only.as_ref().is_some_and(|only| only != id) {
            continue;
        }
        for (suite_key, suite) in suites {
            if args.suite.as_ref().is_some_and(|s| s != suite_key) {
                continue;
            }
            for (mode_key, mode) in modes {
                if args.mode.as_ref().is_some_and(|m| m != mode_key) {
                    continue;
                }
                for &n in &counts {
                    if args.n.is_some_and(|only| only != n) {
                        continue;
                    }
                    for &size in &sizes {
                        if args.size.is_some_and(|only| only != size) {
                            continue;
                        }
                        let record = run(&args, &args.set, id, query, &Config { suite, mode, n, size }, &guest);
                        println!("{record}");
                    }
                }
            }
        }
    }
}
