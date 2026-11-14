#[path = "support/fixture.rs"]
mod fixture_support;
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod support;

#[path = "support/authenticated.rs"]
mod authenticated;
use authenticated::{canonical_fixture as fixture, canonical_policy};
use fixture_support::{integer, iri};
use sparq_query_protocol::*;
use sparq_zk_query::credential::SuiteEncoder as FixtureEncoder;
use sparq_zk_query::{
    answer::Answer,
    capacity::Bounds,
    method::{Compiler, Prepared},
    request::{Request, PROFILE},
    Error,
};
use std::{collections::HashSet, sync::Mutex};

const B: Bounds = Bounds {
    credentials: 1,
    quads_per_credential: 8,
    rows_per_table: 4,
};
const NOW: u64 = 1_794_576_000;
fn id(s: &str) -> Identifier {
    Identifier::new(s).unwrap()
}
fn stored(
    prepared: &Prepared,
    query: &str,
    challenge: u8,
    audience: &str,
    anchor: Option<[u8; 32]>,
) -> StoredRequest {
    StoredRequest::new(StoredRequestSpec {
        requirements: QueryRequirements::new(RequirementsSpec {
            contract: if matches!(prepared.plan.form(), sparq_zk_query::plan::ResultForm::Ask) {
                ResultContract::AskBoolean
            } else {
                ResultContract::SelectSequence
            },
            mode: EvaluationMode::ExactBounded,
            authority: if anchor.is_some() {
                ScopeAuthority::VerifierAgreedAnchor
            } else {
                ScopeAuthority::HolderDeclared
            },
            anchor: anchor.map(|b| Digest32::new(b).unwrap()),
            source_evidence: Some(SourceEvidence::None),
            status: Some(StatusPolicy::NotRequested),
            holder: Some(HolderPolicy::BearerAccepted),
            assembly: DatasetAssembly::UnionDefaultGraph,
            query_profile: QueryProfile {
                dialect: id("urn:sparq:sparql11"),
                fragment: id(PROFILE),
            },
            accepted_suites: vec![],
            accepted_mappings: vec![id(PROFILE)],
            accepted_linking: vec![id(PROFILE)],
            resources: ResourceBounds::new(prepared.plan.bounds().rows_per_table, 1 << 24).unwrap(),
            methods: vec![prepared.descriptor.clone()],
        })
        .unwrap(),
        query: query.into(),
        challenge: Challenge32::new([challenge; 32]).unwrap(),
        audience: id(audience),
        not_before: NOW - 60,
        not_after: NOW + 3600,
        form: if matches!(prepared.plan.form(), sparq_zk_query::plan::ResultForm::Ask) {
            QueryForm::Ask
        } else {
            QueryForm::Select
        },
        base_iri: None,
        describe_policy: None,
    })
    .unwrap()
}
fn authenticated_request(
    stored: StoredRequest,
    bounds: Bounds,
    key: Vec<u8>,
) -> Result<Request, Error> {
    let mut spec = request_spec(&stored);
    let mut requirements = requirements_spec(&spec.requirements);
    requirements.source_evidence = Some(SourceEvidence::IssuerAuthenticated);
    requirements.accepted_suites = vec![id(sparq_zk_query::request::SUITE)];
    spec.requirements = QueryRequirements::new(requirements).unwrap();
    Request::new_authenticated_with_envelope(
        StoredRequest::new(spec).unwrap(),
        bounds,
        key,
        canonical_policy(),
        None,
    )
}

#[derive(Default)]
struct Store(Mutex<HashSet<[u8; 32]>>);
impl ChallengeStore for Store {
    fn consume(&self, challenge: &Challenge32) -> Result<ChallengeOutcome, ChallengeStoreError> {
        Ok(if self.0.lock().unwrap().insert(*challenge.as_bytes()) {
            ChallengeOutcome::Fresh
        } else {
            ChallengeOutcome::AlreadyConsumed
        })
    }
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn native_proof_reconstructs_inputs_and_consumes_only_after_acceptance() {
    let Some(gate) = support::gate(
        "native_proof_reconstructs_inputs_and_consumes_only_after_acceptance",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let zk = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../zk");
    let compiler = Compiler::new(tmp.path(), &zk)
        .unwrap()
        .with_crs_path(std::env::temp_dir().join("sparq_noir_query_crs"));
    let query = "SELECT ?s WHERE {?s <urn:p> ?x}";
    let prepared = compiler.prepare(query, B).unwrap();
    let prepared_again = compiler.prepare(query, B).unwrap();
    assert_eq!(prepared.verification_key, prepared_again.verification_key);
    let separate = tempfile::tempdir().unwrap();
    let separate_compiler = Compiler::new(separate.path(), &zk)
        .unwrap()
        .with_crs_path(std::env::temp_dir().join("sparq_noir_query_crs"));
    let independently_compiled = separate_compiler.prepare(query, B).unwrap();
    assert_eq!(
        prepared.verification_key,
        independently_compiled.verification_key
    );
    assert_eq!(
        prepared.artifact["bytecode"],
        independently_compiled.artifact["bytecode"]
    );
    let request = authenticated_request(
        stored(&prepared, query, 7, "urn:verifier", None),
        B,
        prepared.verification_key.clone(),
    )
    .unwrap();
    let vocabulary = vec![
        iri("urn:a"),
        iri("urn:b"),
        iri("urn:p"),
        integer(1),
        integer(2),
    ];
    let presentation = compiler
        .prove(
            &request,
            &fixture(),
            &vocabulary,
            &FixtureEncoder,
            [3; 32],
            || NOW,
        )
        .unwrap();
    let store = Store::default();
    let audience = id("urn:verifier");
    let check = |p: &sparq_zk_query::method::Presentation| {
        compiler.verify(&request, p, &FixtureEncoder, &audience, &store, || NOW)
    };
    let mut changed = presentation.clone();
    changed.request_digest[31] ^= 1;
    assert_eq!(
        check(&changed).unwrap_err(),
        Error::Invalid("request digest".into())
    );
    let mut changed = presentation.clone();
    changed.input_commitment[31] ^= 1;
    assert!(check(&changed).is_err());
    let mut changed = presentation.clone();
    changed.signature_mode = 0;
    assert_eq!(
        check(&changed).unwrap_err(),
        Error::Unsupported("hidden suite signature mode".into())
    );
    let mut changed = presentation.clone();
    changed.proof[0] ^= 1;
    assert!(check(&changed).is_err());
    let mut changed = presentation.clone();
    if let Answer::Select { rows, .. } = &mut changed.result {
        rows.pop();
    }
    assert!(check(&changed).is_err());
    assert!(compiler
        .verify(
            &request,
            &presentation,
            &FixtureEncoder,
            &id("urn:other"),
            &store,
            || NOW
        )
        .is_err());
    assert!(compiler
        .verify(
            &request,
            &presentation,
            &FixtureEncoder,
            &audience,
            &store,
            || NOW + 3600
        )
        .is_err());
    // This clock is expired before verification starts and valid afterwards.
    // It isolates the initial window check from the final window check.
    assert!(compiler
        .verify(
            &request,
            &presentation,
            &FixtureEncoder,
            &audience,
            &store,
            || {
                let verifying = std::fs::read_dir(tmp.path())
                    .unwrap()
                    .map(Result::unwrap)
                    .any(|entry| entry.file_name().to_string_lossy().starts_with("verify_"));
                if verifying {
                    NOW
                } else {
                    NOW + 3600
                }
            }
        )
        .is_err());
    let late = std::cell::Cell::new(false);
    assert!(compiler
        .verify(
            &request,
            &presentation,
            &FixtureEncoder,
            &audience,
            &store,
            || {
                if late.replace(true) {
                    NOW + 3600
                } else {
                    NOW
                }
            }
        )
        .is_err());
    let mut wrong_prepared = prepared.clone();
    wrong_prepared.verification_key[0] ^= 1;
    wrong_prepared.descriptor =
        sparq_zk_query::request::descriptor(&wrong_prepared.plan, &wrong_prepared.verification_key)
            .unwrap();
    let wrong_key = authenticated_request(
        stored(&wrong_prepared, query, 7, "urn:verifier", None),
        B,
        wrong_prepared.verification_key.clone(),
    )
    .unwrap();
    assert_eq!(
        compiler
            .prove(&wrong_key, &[], &[], &FixtureEncoder, [3; 32], || NOW)
            .unwrap_err(),
        Error::Invalid("verification key mismatch".into())
    );
    assert_eq!(
        compiler
            .verify(
                &wrong_key,
                &sparq_zk_query::method::Presentation {
                    request_digest: wrong_key.digest(),
                    ..presentation.clone()
                },
                &FixtureEncoder,
                &audience,
                &store,
                || NOW
            )
            .unwrap_err(),
        Error::Invalid("verification key mismatch".into())
    );
    let changed_query = "SELECT ?s WHERE {?s <urn:q> ?x}";
    let wrong_request = authenticated_request(
        stored(&prepared, changed_query, 7, "urn:verifier", None),
        B,
        prepared.verification_key.clone(),
    )
    .unwrap();
    let mut redirected = presentation.clone();
    redirected.request_digest = wrong_request.digest();
    assert!(compiler
        .verify(
            &wrong_request,
            &redirected,
            &FixtureEncoder,
            &audience,
            &store,
            || NOW
        )
        .is_err());
    let other_challenge = authenticated_request(
        stored(&prepared, query, 8, "urn:verifier", None),
        B,
        prepared.verification_key.clone(),
    )
    .unwrap();
    let mut redirected = presentation.clone();
    redirected.request_digest = other_challenge.digest();
    assert!(compiler
        .verify(
            &other_challenge,
            &redirected,
            &FixtureEncoder,
            &audience,
            &store,
            || NOW
        )
        .is_err());
    let mut oversized = presentation.clone();
    if let Answer::Select { rows, .. } = &mut oversized.result {
        rows.resize(5, vec![Some(iri("urn:a"))]);
    }
    assert_eq!(
        check(&oversized).unwrap_err(),
        Error::Capacity("released result".into())
    );
    let anchor_request = authenticated_request(
        stored(&prepared, query, 7, "urn:verifier", Some([5; 32])),
        B,
        prepared.verification_key.clone(),
    )
    .unwrap();
    let mut anchored = presentation.clone();
    anchored.request_digest = anchor_request.digest();
    assert_eq!(
        compiler
            .verify(
                &anchor_request,
                &anchored,
                &FixtureEncoder,
                &audience,
                &store,
                || NOW
            )
            .unwrap_err(),
        Error::Invalid("dataset commitment agreement".into())
    );
    let mut spec = request_spec(request.stored());
    let mut requirements = requirements_spec(&spec.requirements);
    requirements.resources = ResourceBounds::new(4, 1).unwrap();
    spec.requirements = QueryRequirements::new(requirements).unwrap();
    let small = authenticated_request(
        StoredRequest::new(spec).unwrap(),
        B,
        prepared.verification_key.clone(),
    )
    .unwrap();
    let mut changed = presentation.clone();
    changed.request_digest = small.digest();
    assert_eq!(
        compiler
            .verify(&small, &changed, &FixtureEncoder, &audience, &store, || NOW)
            .unwrap_err(),
        Error::Capacity("presentation bytes".into())
    );
    assert!(store.0.lock().unwrap().is_empty());
    assert_eq!(check(&presentation).unwrap(), presentation.result);
    assert!(
        matches!(check(&presentation),Err(Error::Protocol(e)) if e.code()==ErrorCode::ChallengeReplayed)
    );
    assert_eq!(*store.0.lock().unwrap(), HashSet::from([[7; 32]]));
    support::complete(gate);
}

fn native_proof_cases(profile: Option<&str>) -> Result<Vec<fixture_support::N0Case>, &'static str> {
    match profile {
        None | Some("full") => Ok(fixture_support::n0_cases()),
        Some("smoke") => Ok(vec![fixture_support::N0Case {
            name: "authenticated-ask-smoke",
            query: "ASK {?s <urn:p> ?o}",
            sources: vec!["<urn:s> <urn:p> <urn:o> .".into()],
        }]),
        _ => Err("invalid native proof profile"),
    }
}

#[test]
fn native_proof_profiles_preserve_the_acceptance_sweep_and_refuse_unknown_profiles() {
    let expected = fixture_support::n0_cases()
        .into_iter()
        .map(|c| c.name)
        .collect::<Vec<_>>();
    let default = native_proof_cases(None).unwrap();
    assert_eq!(default.iter().map(|c| c.name).collect::<Vec<_>>(), expected);
    let full = native_proof_cases(Some("full")).unwrap();
    assert_eq!(full.iter().map(|c| c.name).collect::<Vec<_>>(), expected);
    let smoke = native_proof_cases(Some("smoke")).unwrap();
    assert_eq!(
        smoke.iter().map(|c| c.name).collect::<Vec<_>>(),
        ["authenticated-ask-smoke"]
    );
    assert_eq!(smoke[0].sources, ["<urn:s> <urn:p> <urn:o> ."]);
    assert!(native_proof_cases(Some("other")).is_err());
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn n0_native_proof_profile_matches_the_engine_and_refuses_altered_answers() {
    let Some(gate) = support::gate(
        "n0_native_proof_profile_matches_the_engine_and_refuses_altered_answers",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let zk = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../zk");
    let compiler = Compiler::new(temp.path(), zk)
        .unwrap()
        .with_crs_path(std::env::temp_dir().join("sparq_noir_query_crs"));
    let profile = match std::env::var("SPARQ_NOIR_QUERY_PROOF_PROFILE") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => panic!("invalid native proof profile"),
    };
    let cases =
        native_proof_cases(profile.as_deref()).expect("native proof profile is full or smoke");
    eprintln!(
        "Native proof profile: {}",
        profile.as_deref().unwrap_or("full")
    );
    let store = Store::default();
    for (index, case) in cases.into_iter().enumerate() {
        let bounds = Bounds {
            credentials: case.sources.len() as u32,
            rows_per_table: if matches!(case.name, "projection-bag" | "distinct") {
                8
            } else {
                B.rows_per_table
            },
            ..B
        };
        let start = std::time::Instant::now();
        let prepared = compiler.prepare(case.query, bounds).unwrap();
        let prepare_seconds = start.elapsed().as_secs_f64();
        let mut data = vec![];
        let mut vocabulary = vec![];
        for (i, source) in case.sources.iter().enumerate() {
            let (_, terms) = fixture_support::dataset(source);
            data.push(authenticated::canonical_sign(source, i as u64));
            vocabulary.extend(terms);
        }
        let request = authenticated_request(
            stored(&prepared, case.query, index as u8 + 1, "urn:verifier", None),
            bounds,
            prepared.verification_key.clone(),
        )
        .unwrap();
        let dataset = sparq_zk_query::credential::assemble_issuer_dataset(
            &data,
            &canonical_policy().trust_members,
            request.digest(),
        )
        .unwrap();
        let trace =
            sparq_zk_query::interp::evaluate_dataset(&prepared.plan, &dataset, &FixtureEncoder)
                .unwrap();
        // The oracle observes the complete default merge, including signed metadata
        // and the one authenticated issuer assertion per presentation graph.
        let mut source = String::new();
        for (i, credential) in data.iter().enumerate() {
            source.push_str(&credential.credential.document);
            source.push_str(&format!(
                "_:graph{i} <{}> <urn:issuer> .\n",
                sparq_zk_query::credential::VERIFIED_ISSUER
            ));
        }
        if case.name != "slice" {
            fixture_support::assert_engine_with_encoder(
                case.query,
                &source,
                &trace.result,
                &FixtureEncoder,
            );
        }
        let start = std::time::Instant::now();
        let presentation = compiler
            .prove(
                &request,
                &data,
                &vocabulary,
                &FixtureEncoder,
                [3; 32],
                || NOW,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        let prove_seconds = start.elapsed().as_secs_f64();
        let mut changed = presentation.clone();
        match &mut changed.result {
            Answer::Ask(value) => *value = !*value,
            Answer::Select { rows, .. } => {
                rows.pop();
            }
        }
        assert!(
            compiler
                .verify(
                    &request,
                    &changed,
                    &FixtureEncoder,
                    &id("urn:verifier"),
                    &store,
                    || NOW
                )
                .is_err(),
            "{}",
            case.name
        );
        let start = std::time::Instant::now();
        assert_eq!(
            compiler
                .verify(
                    &request,
                    &presentation,
                    &FixtureEncoder,
                    &id("urn:verifier"),
                    &store,
                    || NOW
                )
                .unwrap(),
            presentation.result
        );
        let verify_seconds = start.elapsed().as_secs_f64();
        let artifact_path = temp.path().join("gates.json");
        std::fs::write(
            &artifact_path,
            serde_json::to_vec(&prepared.artifact).unwrap(),
        )
        .unwrap();
        let output = support::live_tool(&gate, "bb")
            .args(["gates", "-s", "ultra_honk", "-t", "noir-recursive", "-b"])
            .arg(&artifact_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let gates: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(gates["functions"][0]["circuit_size"].as_u64().unwrap() > 0);
        if let Some(path) = std::env::var_os("SPARQ_NOIR_QUERY_EVIDENCE") {
            use std::io::Write;
            let record = serde_json::json!({"case":case.name,"query":case.query,"plan":format!("{:#?}",prepared.plan),"prepare_seconds":prepare_seconds,"prove_seconds":prove_seconds,"verify_seconds":verify_seconds,"proof_bytes":presentation.proof.len(),"gates":gates});
            writeln!(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .unwrap(),
                "{record}"
            )
            .unwrap();
        }
    }
    support::complete(gate);
}

#[test]
fn stored_rdf_digest_covers_the_exact_query_key_challenge_and_bounds() {
    let query = "ASK {}";
    let plan = sparq_zk_query::lower::lower(query, B).unwrap();
    let key = vec![1, 2, 3];
    let prepared = Prepared {
        descriptor: sparq_zk_query::request::descriptor(&plan, &key).unwrap(),
        plan,
        verification_key: key.clone(),
        artifact: serde_json::Value::Null,
    };
    let request = |p: &Prepared, q: &str, c: u8, a: &str, b: Bounds| {
        Request::new(stored(p, q, c, a, None), b, p.verification_key.clone()).unwrap()
    };
    let base = request(&prepared, query, 7, "urn:verifier", B);
    assert_eq!(
        base.canonical_rdf(),
        sparq_canon::canonicalize_nquads(base.canonical_rdf()).unwrap()
    );
    use sha2::Digest;
    assert_eq!(
        base.digest(),
        <[u8; 32]>::from(sha2::Sha256::digest(base.canonical_rdf().as_bytes()))
    );
    for other in [
        request(&prepared, "ASK { }", 7, "urn:verifier", B),
        request(&prepared, query, 8, "urn:verifier", B),
        request(&prepared, query, 7, "urn:other", B),
    ] {
        assert_ne!(base.digest(), other.digest());
    }
    let mut different = prepared.clone();
    different.verification_key[0] ^= 1;
    different.descriptor =
        sparq_zk_query::request::descriptor(&different.plan, &different.verification_key).unwrap();
    assert_ne!(
        base.digest(),
        request(&different, query, 7, "urn:verifier", B).digest()
    );
    let bounds = Bounds {
        rows_per_table: 3,
        ..B
    };
    let mut different = prepared.clone();
    different.plan = sparq_zk_query::lower::lower(query, bounds).unwrap();
    different.descriptor =
        sparq_zk_query::request::descriptor(&different.plan, &different.verification_key).unwrap();
    assert_ne!(
        base.digest(),
        request(&different, query, 7, "urn:verifier", bounds).digest()
    );
    assert!(base
        .canonical_rdf()
        .contains("https://w3id.org/sparq/vcq#verificationKey"));
    assert!(base.canonical_rdf().contains("storedRequestEncoding"));
    let mut found = false;
    for quad in oxttl::NQuadsParser::new().for_reader(base.canonical_rdf().as_bytes()) {
        let quad = quad.unwrap();
        if quad.predicate.as_str() == "https://w3id.org/sparq/vcq#parameters" {
            let oxrdf::Term::Literal(literal) = quad.object else {
                panic!("JSON literal")
            };
            let positions: Vec<_> = [
                "bounds",
                "planHash",
                "profile",
                "storedRequest",
                "storedRequestEncoding",
            ]
            .iter()
            .map(|key| literal.value().find(&format!("\"{key}\":")).unwrap())
            .collect();
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
            found = true;
        }
    }
    assert!(found);
}

fn request_spec(request: &StoredRequest) -> StoredRequestSpec {
    StoredRequestSpec {
        requirements: request.requirements().clone(),
        query: request.query().into(),
        challenge: *request.challenge(),
        audience: request.audience().clone(),
        not_before: request.not_before(),
        not_after: request.not_after(),
        form: request.form(),
        base_iri: request.base_iri().cloned(),
        describe_policy: request.describe_policy().cloned(),
    }
}
fn requirements_spec(r: &QueryRequirements) -> RequirementsSpec {
    RequirementsSpec {
        contract: r.contract(),
        mode: r.mode(),
        authority: r.authority(),
        anchor: r.anchor().copied(),
        source_evidence: Some(r.source_evidence()),
        status: Some(r.status()),
        holder: Some(r.holder()),
        assembly: r.assembly(),
        query_profile: r.query_profile().clone(),
        accepted_suites: r.accepted_suites().to_vec(),
        accepted_mappings: r.accepted_mappings().to_vec(),
        accepted_linking: r.accepted_linking().to_vec(),
        resources: r.resources(),
        methods: r.methods().to_vec(),
    }
}
#[test]
fn draft_profile_refuses_unimplemented_request_obligations_by_name() {
    let plan = sparq_zk_query::lower::lower("ASK {}", B).unwrap();
    let key = vec![1, 2, 3];
    let prepared = Prepared {
        descriptor: sparq_zk_query::request::descriptor(&plan, &key).unwrap(),
        plan,
        verification_key: key.clone(),
        artifact: serde_json::Value::Null,
    };
    let stored = stored(&prepared, "ASK {}", 7, "urn:verifier", None);
    for (name, change) in [
        ("credential suite opening", 0),
        ("credential status envelope", 1),
        ("holder envelope", 2),
        ("dataset assembly", 3),
        ("query mapping and linking profile", 4),
    ] {
        let mut requirements = requirements_spec(stored.requirements());
        match change {
            0 => {
                requirements.source_evidence = Some(SourceEvidence::IssuerAuthenticated);
                requirements.accepted_suites = vec![id("urn:suite")];
            }
            1 => requirements.status = Some(StatusPolicy::Required),
            2 => requirements.holder = Some(HolderPolicy::Required),
            3 => requirements.assembly = DatasetAssembly::CredentialNamedGraphs,
            4 => requirements.query_profile.fragment = id("urn:other"),
            _ => unreachable!(),
        }
        let mut spec = request_spec(&stored);
        spec.requirements = QueryRequirements::new(requirements).unwrap();
        assert_eq!(
            Request::new(StoredRequest::new(spec).unwrap(), B, key.clone()).unwrap_err(),
            Error::Unsupported(name.into())
        );
    }
    let mut spec = request_spec(&stored);
    spec.base_iri = Some(BaseIri::new("urn:base").unwrap());
    assert_eq!(
        Request::new(StoredRequest::new(spec).unwrap(), B, key.clone()).unwrap_err(),
        Error::Excluded("external base IRI".into())
    );
    let mut spec = request_spec(&stored);
    spec.form = QueryForm::Select;
    let mut requirements = requirements_spec(stored.requirements());
    requirements.contract = ResultContract::SelectSequence;
    spec.requirements = QueryRequirements::new(requirements).unwrap();
    assert_eq!(
        Request::new(StoredRequest::new(spec).unwrap(), B, key.clone()).unwrap_err(),
        Error::Invalid("query form and result contract".into())
    );
    let mut spec = request_spec(&stored);
    spec.not_before = u64::MAX - 2;
    spec.not_after = u64::MAX - 1;
    assert_eq!(
        Request::new(StoredRequest::new(spec).unwrap(), B, key.clone()).unwrap_err(),
        Error::Invalid("request validity date".into())
    );
    assert_eq!(
        Request::new(stored, B, vec![]).unwrap_err(),
        Error::Invalid("verification key".into())
    );
}

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn shape_cache_reuses_constants_and_invalidates_library_revisions() {
    let Some(gate) = support::gate(
        "shape_cache_reuses_constants_and_invalidates_library_revisions",
        &["nargo", "bb"],
    ) else {
        return;
    };
    fn copy_sources(source: &std::path::Path, destination: &std::path::Path) {
        std::fs::create_dir_all(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap().map(Result::unwrap) {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() && entry.file_name() != "target" {
                copy_sources(&path, &destination.join(entry.file_name()));
            } else if path.extension().is_some_and(|e| e == "nr")
                || entry.file_name() == "Nargo.toml"
            {
                std::fs::copy(path, destination.join(entry.file_name())).unwrap();
            }
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let zk = temp.path().join("zk");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../zk");
    for directory in ["noir-query/core", "compose/compose_core", "dual-leaf-v2"] {
        copy_sources(&source.join(directory), &zk.join(directory));
    }
    let cache = temp.path().join("cache");
    let compiler = Compiler::new(&cache, &zk)
        .unwrap()
        .with_crs_path(std::env::temp_dir().join("sparq_noir_query_crs"));
    let entries = || {
        std::fs::read_dir(&cache)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().starts_with("noir_query_"))
            .count()
    };
    let original = compiler.prepare("ASK {?s <urn:p> ?x}", B).unwrap();
    assert_eq!(entries(), 1);
    let other_constants = compiler.prepare("ASK {?v <urn:q> ?y}", B).unwrap();
    assert_eq!(original.verification_key, other_constants.verification_key);
    assert_eq!(entries(), 1);
    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(zk.join("noir-query/core/src/table.nr"))
            .unwrap(),
        "\n// Library revision fixture."
    )
    .unwrap();
    let revised = compiler.prepare("ASK {?s <urn:p> ?x}", B).unwrap();
    assert_eq!(original.verification_key, revised.verification_key);
    let original_request = authenticated_request(
        stored(&original, "ASK {?s <urn:p> ?x}", 7, "urn:verifier", None),
        B,
        original.verification_key.clone(),
    )
    .unwrap();
    let path = zk.join("noir-query/core/src/ops_core.nr");
    let source = std::fs::read_to_string(&path).unwrap();
    let old = "assert(result == (input.len > 0), \"ASK result\");";
    let new = "assert(result != (input.len > 0), \"ASK result\");";
    assert_eq!(source.matches(old).count(), 1);
    std::fs::write(path, source.replace(old, new)).unwrap();
    let changed = compiler.prepare("ASK {?s <urn:p> ?x}", B).unwrap();
    assert_ne!(original.verification_key, changed.verification_key);
    assert_eq!(entries(), 3);
    assert_eq!(
        compiler
            .prove(
                &original_request,
                &[],
                &[],
                &FixtureEncoder,
                [3; 32],
                || NOW
            )
            .unwrap_err(),
        Error::Invalid("verification key mismatch".into())
    );
    support::complete(gate);
}

#[test]
fn authenticated_issuer_policy_is_part_of_the_canonical_request() {
    let query = "ASK {}";
    let plan = sparq_zk_query::lower::lower(query, B).unwrap();
    let key = vec![1, 2, 3];
    let prepared = Prepared {
        descriptor: sparq_zk_query::request::descriptor(&plan, &key).unwrap(),
        plan,
        verification_key: key.clone(),
        artifact: serde_json::Value::Null,
    };
    let request = authenticated_request(
        stored(&prepared, query, 7, "urn:verifier", None),
        B,
        key.clone(),
    )
    .unwrap();
    let mut other_root = request.issuer_root().unwrap();
    other_root[31] ^= 1;
    let other = Request::new_authenticated(
        StoredRequest::new(request_spec(request.stored())).unwrap(),
        B,
        key,
        other_root,
    )
    .unwrap();
    assert_ne!(request.digest(), other.digest());
    assert!(request.canonical_rdf().contains("issuerRoot"));
    assert_eq!(
        request.issuer_root().unwrap(),
        sparq_zk_query::envelope_policy::derive(&canonical_policy())
            .unwrap()
            .issuer_root
    );
}

#[test]
fn stored_request_reconstructs_the_verifier_selected_opening_profile() {
    use sparq_zk_query::capacity::OpeningProfile;
    let query = "ASK {}";
    let plan = sparq_zk_query::lower::lower(query, B)
        .unwrap()
        .with_opening_profile(OpeningProfile::Extended);
    let key = vec![37; 8];
    let prepared = Prepared {
        descriptor: sparq_zk_query::request::descriptor(&plan, &key).unwrap(),
        plan,
        verification_key: key.clone(),
        artifact: serde_json::json!({}),
    };
    let mut spec = request_spec(&stored(&prepared, query, 9, "urn:verifier", None));
    let mut requirements = requirements_spec(&spec.requirements);
    requirements.source_evidence = Some(SourceEvidence::IssuerAuthenticated);
    requirements.accepted_suites = vec![id(sparq_zk_query::request::SUITE)];
    spec.requirements = QueryRequirements::new(requirements).unwrap();
    let stored = StoredRequest::new(spec).unwrap();
    let request = Request::new_authenticated_with_opening_profile(
        stored.clone(),
        B,
        key.clone(),
        authenticated::issuer_root(),
        None,
        OpeningProfile::Extended,
    )
    .unwrap();
    assert_eq!(request.opening_profile(), OpeningProfile::Extended);
    assert_eq!(request.plan().unwrap().hash(), prepared.plan.hash());
    assert!(request.canonical_rdf().contains("openingProfile"));
    let policy = sparq_zk_query::envelope_policy::PolicyInput {
        trust_members: vec![sparq_zk_query::envelope_policy::TrustMember {
            issuer: oxrdf::NamedNode::new_unchecked("urn:issuer"),
            verification_method: oxrdf::NamedNode::new_unchecked("urn:issuer#key"),
            key: sparq_zk::sig::SecretKey::from_seed(19).public_key(),
        }],
        validity_time: Some(1_794_614_400_123),
        ..Default::default()
    };
    let typed_plan =
        prepared
            .plan
            .clone()
            .with_envelope_features(sparq_zk_query::plan::EnvelopeFeatures {
                validity: true,
                ..Default::default()
            });
    let mut typed_spec = request_spec(&stored);
    let mut typed_requirements = requirements_spec(&typed_spec.requirements);
    typed_requirements.methods =
        vec![sparq_zk_query::request::descriptor(&typed_plan, &key).unwrap()];
    typed_spec.requirements = QueryRequirements::new(typed_requirements).unwrap();
    let typed = Request::new_authenticated_with_envelope_and_profile(
        StoredRequest::new(typed_spec).unwrap(),
        B,
        key.clone(),
        policy,
        None,
        OpeningProfile::Extended,
    )
    .unwrap();
    assert_eq!(typed.plan().unwrap().hash(), typed_plan.hash());
    assert_ne!(typed.plan().unwrap().hash(), prepared.plan.hash());
    assert_eq!(typed.opening_profile(), OpeningProfile::Extended);
    assert_eq!(typed.envelope_inputs().unwrap().now, 1_794_614_400_123);
    assert_ne!(request.digest(), typed.digest());
    assert_eq!(
        Request::new_authenticated(stored, B, key, authenticated::issuer_root()).unwrap_err(),
        Error::Invalid("method descriptor".into()),
    );
}

#[test]
fn authenticated_request_admits_and_binds_typed_envelope_properties() {
    use sparq_zk_query::statement::EnvelopeInputs;
    let plan = sparq_zk_query::lower::lower("ASK {}", B)
        .unwrap()
        .with_envelope_features(sparq_zk_query::plan::EnvelopeFeatures::ALL);
    let key = vec![1, 2, 3];
    let prepared = Prepared {
        descriptor: sparq_zk_query::request::descriptor(&plan, &key).unwrap(),
        plan,
        verification_key: key.clone(),
        artifact: serde_json::Value::Null,
    };
    let base = stored(&prepared, "ASK {}", 7, "urn:verifier", None);
    let mut requirements = requirements_spec(base.requirements());
    requirements.source_evidence = Some(SourceEvidence::IssuerAuthenticated);
    requirements.accepted_suites = vec![id(sparq_zk_query::request::SUITE)];
    requirements.holder = Some(HolderPolicy::Required);
    requirements.status = Some(StatusPolicy::Required);
    let mut spec = request_spec(&base);
    spec.requirements = QueryRequirements::new(requirements).unwrap();
    let stored = StoredRequest::new(spec).unwrap();
    let mut policy = EnvelopeInputs::issuer_only(authenticated::issuer_root());
    policy.holder_required = true;
    policy.status_required = true;
    policy.validity_required = true;
    policy.now = 1u64 << 40;
    policy.freshness_required = true;
    policy.earliest_created = 1u64 << 39;
    policy.snapshot_root[31] = 7;
    let request = Request::new_authenticated_with_policy(
        stored.clone(),
        B,
        key.clone(),
        policy.clone(),
        None,
    )
    .unwrap();
    assert_eq!(request.envelope_inputs().unwrap(), policy);
    assert_eq!(
        request.plan().unwrap().canonical_bytes(),
        prepared.plan.canonical_bytes()
    );
    let mut altered = policy.clone();
    altered.earliest_created += 1;
    let other =
        Request::new_authenticated_with_policy(stored.clone(), B, key.clone(), altered, None)
            .unwrap();
    assert_ne!(request.digest(), other.digest());
    assert!(request.canonical_rdf().contains("envelopeFeatures"));
    assert_eq!(
        request.envelope_features(),
        sparq_zk_query::plan::EnvelopeFeatures::ALL
    );
    for consumer in ["validity", "freshness"] {
        let mut dropped = policy.clone();
        if consumer == "validity" {
            dropped.validity_required = false;
            dropped.now = 0;
        } else {
            dropped.freshness_required = false;
            dropped.earliest_created = 0;
        }
        assert_eq!(
            Request::new_authenticated_with_policy(stored.clone(), B, key.clone(), dropped, None)
                .unwrap_err(),
            Error::Invalid("method descriptor".into())
        );
    }
    let mut altered = policy;
    altered.holder_required = false;
    assert_eq!(
        Request::new_authenticated_with_policy(stored, B, key, altered, None).unwrap_err(),
        Error::Invalid("request and envelope policy".into())
    );
}

#[test]
fn request_admission_refuses_a_present_empty_holder_issuer_list() {
    let plan = sparq_zk_query::lower::lower("ASK {}", B)
        .unwrap()
        .with_envelope_features(sparq_zk_query::plan::EnvelopeFeatures {
            holder: true,
            ..Default::default()
        });
    let key = b"request-admission-fixture".to_vec();
    let prepared = Prepared {
        descriptor: sparq_zk_query::request::descriptor(&plan, &key).unwrap(),
        plan,
        verification_key: key.clone(),
        artifact: serde_json::Value::Null,
    };
    let mut spec = request_spec(&stored(&prepared, "ASK {}", 1, "urn:receiver", None));
    let mut requirements = requirements_spec(&spec.requirements);
    requirements.source_evidence = Some(SourceEvidence::IssuerAuthenticated);
    requirements.holder = Some(HolderPolicy::Required);
    requirements.accepted_suites = vec![id(sparq_zk_query::request::SUITE)];
    spec.requirements = QueryRequirements::new(requirements).unwrap();
    let stored = StoredRequest::new(spec).unwrap();
    let mut policy = sparq_zk_query::statement::EnvelopeInputs::issuer_only([7; 32]);
    policy.holder_required = true;
    assert!(Request::new_authenticated_with_policy(
        stored.clone(),
        B,
        key.clone(),
        policy.clone(),
        None
    )
    .is_ok());
    policy.holder_issuers_present = true;
    assert_eq!(
        Request::new_authenticated_with_policy(stored, B, key, policy, None).unwrap_err(),
        Error::Invalid("empty holder issuer list".into())
    );
}

#[test]
fn request_derives_canonical_policy_roots_and_exact_millisecond_words() {
    use oxrdf::NamedNode;
    use sparq_zk_query::envelope_policy::{
        self, AcceptedSnapshots, HolderBinding, PolicyInput, TrustMember,
    };
    let plan = sparq_zk_query::lower::lower("ASK {}", B)
        .unwrap()
        .with_envelope_features(sparq_zk_query::plan::EnvelopeFeatures::ALL);
    let key = b"typed-envelope-fixture".to_vec();
    let prepared = Prepared {
        descriptor: sparq_zk_query::request::descriptor(&plan, &key).unwrap(),
        plan,
        verification_key: key.clone(),
        artifact: serde_json::Value::Null,
    };
    let mut spec = request_spec(&stored(&prepared, "ASK {}", 1, "urn:receiver", None));
    let mut requirements = requirements_spec(&spec.requirements);
    requirements.source_evidence = Some(SourceEvidence::IssuerAuthenticated);
    requirements.holder = Some(HolderPolicy::Required);
    requirements.status = Some(StatusPolicy::Required);
    requirements.accepted_suites = vec![id(sparq_zk_query::request::SUITE)];
    spec.requirements = QueryRequirements::new(requirements).unwrap();
    let stored = StoredRequest::new(spec).unwrap();
    let issuer = NamedNode::new("urn:issuer").unwrap();
    let mut policy = PolicyInput {
        trust_members: vec![TrustMember {
            issuer: issuer.clone(),
            verification_method: NamedNode::new("urn:issuer#key").unwrap(),
            key: sparq_zk::sig::SecretKey::from_seed(19).public_key(),
        }],
        holder_binding: Some(HolderBinding {
            issuers: Some(vec![issuer.clone()]),
        }),
        accepted_snapshots: Some(AcceptedSnapshots {
            digests: vec![sparq_zk_query::statement::number(7)],
            allow_missing: false,
        }),
        validity_time: Some(1_794_614_400_123),
        earliest_created: Some(1_794_528_000_456),
    };
    let request = Request::new_authenticated_with_envelope(
        stored.clone(),
        B,
        key.clone(),
        policy.clone(),
        None,
    )
    .unwrap();
    let expected = envelope_policy::public_words(&policy).unwrap();
    let words_request = Request::new_authenticated_with_policy(
        stored.clone(),
        B,
        key.clone(),
        envelope_policy::derive(&policy).unwrap().into(),
        None,
    )
    .unwrap();
    assert_eq!(request.canonical_rdf(), words_request.canonical_rdf());
    assert_eq!(request.digest(), words_request.digest());
    assert_eq!(
        request.envelope_inputs().unwrap().words().unwrap(),
        expected
    );
    assert_eq!(request.envelope_inputs().unwrap().now, 1_794_614_400_123);
    assert_eq!(
        request.envelope_inputs().unwrap().earliest_created,
        1_794_528_000_456
    );
    assert_eq!(request.envelope_inputs().unwrap().issuer_depth, 0);
    policy.trust_members.push(policy.trust_members[0].clone());
    policy
        .holder_binding
        .as_mut()
        .unwrap()
        .issuers
        .as_mut()
        .unwrap()
        .push(issuer);
    let canonical = Request::new_authenticated_with_envelope(
        stored.clone(),
        B,
        key.clone(),
        policy.clone(),
        None,
    )
    .unwrap();
    assert_eq!(request.digest(), canonical.digest());
    policy.validity_time = policy.validity_time.map(|v| v + 1);
    let altered = Request::new_authenticated_with_envelope(
        stored.clone(),
        B,
        key.clone(),
        policy.clone(),
        None,
    )
    .unwrap();
    assert_ne!(request.digest(), altered.digest());
    assert_eq!(
        request.envelope_inputs().unwrap().words().unwrap(),
        expected
    );
    policy.holder_binding.as_mut().unwrap().issuers = Some(vec![]);
    assert_eq!(
        Request::new_authenticated_with_envelope(stored, B, key, policy, None).unwrap_err(),
        Error::Invalid("empty holder issuer list".into())
    );
}

#[test]
fn canonical_policy_is_wired_to_the_native_program() {
    use oxrdf::NamedNode;
    use sparq_zk_query::envelope_policy::{PolicyInput, TrustMember};
    let plan = sparq_zk_query::lower::lower("ASK {}", B).unwrap();
    let key = b"canonical-policy-refusal-fixture".to_vec();
    let prepared = Prepared {
        descriptor: sparq_zk_query::request::descriptor(&plan, &key).unwrap(),
        plan,
        verification_key: key.clone(),
        artifact: serde_json::Value::Null,
    };
    let base = authenticated_request(
        stored(&prepared, "ASK {}", 7, "urn:verifier", None),
        B,
        key.clone(),
    )
    .unwrap();
    let policy = PolicyInput {
        trust_members: (0..(1 << sparq_zk_query::credential::ISSUER_DEPTH))
            .map(|i| TrustMember {
                issuer: NamedNode::new(format!("urn:issuer:{i}")).unwrap(),
                verification_method: NamedNode::new(format!("urn:issuer:{i}#key")).unwrap(),
                key: sparq_zk::sig::SecretKey::from_seed(19).public_key(),
            })
            .collect(),
        ..PolicyInput::default()
    };
    let request =
        Request::new_authenticated_with_envelope(base.stored().clone(), B, key, policy, None)
            .unwrap();
    assert_eq!(
        request.envelope_inputs().unwrap().issuer_depth,
        sparq_zk_query::credential::ISSUER_DEPTH
    );
    let package = sparq_zk_query::codegen::generate(&request.plan().unwrap()).unwrap();
    let source = &package.files[&format!("{}/src/main.nr", package.name)];
    assert!(source.contains("core::ops_core_envelope::select_policy::<0,0,0,0>"));
    assert!(source.contains("core::ops_core_envelope::open_slot"));
    assert!(source.contains("core::ops_core_envelope::check_selected_holders::<0,1>"));
    assert!(!source.contains("open_profile_mode_one"));
}

#[test]
fn native_acceptance_cases_use_the_complete_method_dataset() {
    for case in native_proof_cases(None).unwrap() {
        let bounds = Bounds {
            credentials: case.sources.len() as u32,
            rows_per_table: if matches!(case.name, "projection-bag" | "distinct") {
                8
            } else {
                B.rows_per_table
            },
            ..B
        };
        let plan = sparq_zk_query::lower::lower(case.query, bounds).unwrap();
        let data = case
            .sources
            .iter()
            .enumerate()
            .map(|(i, s)| authenticated::canonical_sign(s, i as u64))
            .collect::<Vec<_>>();
        let dataset = sparq_zk_query::credential::assemble_issuer_dataset(
            &data,
            &canonical_policy().trust_members,
            [7; 32],
        )
        .unwrap();
        let trace =
            sparq_zk_query::interp::evaluate_dataset(&plan, &dataset, &FixtureEncoder).unwrap();
        let mut source = String::new();
        for (i, credential) in data.iter().enumerate() {
            source.push_str(&credential.credential.document);
            source.push_str(&format!(
                "_:graph{i} <{}> <urn:issuer> .\n",
                sparq_zk_query::credential::VERIFIED_ISSUER
            ));
        }
        if case.name != "slice" {
            fixture_support::assert_engine_with_encoder(
                case.query,
                &source,
                &trace.result,
                &FixtureEncoder,
            );
        }
    }
}
