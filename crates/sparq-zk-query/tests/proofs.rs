#[path = "support/mod.rs"]
mod fixture_support;
#[path = "../../sparq-zk-compose/tests/support/mod.rs"]
mod support;

use fixture_support::{fixture, integer, iri, FixtureEncoder};
use sparq_query_protocol::*;
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
    quads_per_credential: 4,
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
            resources: ResourceBounds::new(4, 1 << 24).unwrap(),
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
    let query = "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1 && ?x < 3)}";
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
    let request = Request::new(
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
    changed.signature_mode = 1;
    assert_eq!(
        check(&changed).unwrap_err(),
        Error::Unsupported("credential suite opening".into())
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
    let wrong_key = Request::new(
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
    let changed_query = "SELECT ?s WHERE {?s <urn:p> ?x FILTER(?x >= 1 && ?x < 2)}";
    let wrong_request = Request::new(
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
    let other_challenge = Request::new(
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
    let anchor_request = Request::new(
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
    let small = Request::new(
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

#[test]
#[ignore = "requires the pinned native query toolchain"]
fn n0_native_proofs_match_the_engine_and_refuse_altered_answers() {
    let Some(gate) = support::gate(
        "n0_native_proofs_match_the_engine_and_refuse_altered_answers",
        &["nargo", "bb"],
    ) else {
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let zk = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../zk");
    let compiler = Compiler::new(temp.path(), zk)
        .unwrap()
        .with_crs_path(std::env::temp_dir().join("sparq_noir_query_crs"));
    let bounds = Bounds {
        credentials: 2,
        ..B
    };
    let store = Store::default();
    for (index, case) in fixture_support::n0_cases().into_iter().enumerate() {
        let start = std::time::Instant::now();
        let prepared = compiler.prepare(case.query, bounds).unwrap();
        let prepare_seconds = start.elapsed().as_secs_f64();
        let mut data = vec![];
        let mut vocabulary = vec![];
        for (i, source) in case.sources.iter().enumerate() {
            let (mut cells, terms) = fixture_support::dataset(source);
            for q in &mut cells[0].quads {
                for c in q {
                    if c.kind == 2 {
                        c.scope = i as u32 + 1;
                    }
                }
            }
            data.extend(cells);
            vocabulary.extend(terms);
        }
        let request = Request::new(
            stored(&prepared, case.query, index as u8 + 1, "urn:verifier", None),
            bounds,
            prepared.verification_key.clone(),
        )
        .unwrap();
        let trace =
            sparq_zk_query::interp::evaluate(&prepared.plan, &data, &FixtureEncoder).unwrap();
        // Slicing uses the method's canonical order; the graph oracle compares
        // the unsliced relation because SPARQL permits other unspecified orders.
        if case.name != "slice" {
            fixture_support::assert_engine(case.query, &case.sources.join("\n"), &trace.result);
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
    assert_eq!(entries(), 2);
    support::complete(gate);
}
