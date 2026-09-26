// [OPUS-5.5] Native engine-replay bridge contracts; no guest execution or receipts.
#![cfg(feature = "graph-results")]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::replay::{
    self, Cell, Disposition, ExpectedManifest, HolderManifest, HolderSalt, ManifestError, Oracle,
    OracleKind, OriginalDigests, Originals, ReplayError, RetainedFile, SourceError,
    VerifierManifest,
};
use sparq_proved_evaluator_model::{
    BudgetExceeded, DatasetAuthority, EvaluationCapacity, EvaluationError, ProofContract,
    Provenance, Rejected, RowOrder, v3,
};

/// Published salt; every input in this file is explicitly synthetic test data.
const SALT: [u8; 32] = [41; 32];
const QUERY: &str = "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?o }";
const DATA: &str = "@prefix ex: <http://ex/> .\nex:a ex:p ex:b , \"x\"@en .\nex:c ex:p 7 .\n";

struct Retained {
    record: Vec<u8>,
    query: Vec<u8>,
    data: Vec<u8>,
}

impl Retained {
    fn new(query: &str, data: &str) -> Self {
        Self::with(query, data, |_| {})
    }

    fn with(query: &str, data: &str, edit: impl FnOnce(&mut Value)) -> Self {
        let mut record = json!({
            "schema": replay::RECORD_SCHEMA, "seed": 4695, "category": "bgp", "storage": "dense",
            "input": {"query": query, "dataset": data, "format": "turtle", "named_graph_catalog": []},
            // Deliberately different observations: the bridge must never read them.
            "observed_ntriples": ["<http://ex/observed> <http://ex/p> <http://ex/row> ."],
            "native_result": {"kind": "ask", "value": true},
            "reference_result": {"kind": "ask", "value": false},
            "native_error": null, "reference_error": null,
            "comparison": {"status": "agreement"},
            "divergence_registry": {"source": "bench/differential-divergences.json"},
            "proof_count": 0, "verified_proof_count": 0,
            "proof_bridge": {"status": "not-prepared", "backend": null, "authority": null,
                "public_statement_sha256": null, "private_witness_sha256": null,
                "reuse_allowed": false},
        });
        edit(&mut record);
        Self {
            record: serde_json::to_vec(&record).unwrap(),
            query: query.as_bytes().to_vec(),
            data: data.as_bytes().to_vec(),
        }
    }

    fn originals(&self) -> Originals<'_> {
        Originals {
            record: &self.record,
            query: &self.query,
            data: &self.data,
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn holder() -> HolderManifest {
    HolderManifest {
        schema: replay::HOLDER_SCHEMA.into(),
        synthetic_inputs: true,
        salt: HolderSalt::SyntheticFixed(SALT),
    }
}

fn verifier(retained: &Retained, authority: DatasetAuthority) -> VerifierManifest {
    VerifierManifest {
        schema: replay::VERIFIER_SCHEMA.into(),
        cell: Cell {
            profile: "shipped".into(),
            category: "bgp".into(),
            seed: 4695,
            storage: "dense".into(),
        },
        originals: OriginalDigests {
            record_sha256: hex(&retained.record),
            query_sha256: hex(&retained.query),
            data_sha256: hex(&retained.data),
        },
        request: v3::Request {
            version: v3::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v3::Dialect::SparqSparql11GraphResultsV3,
            query: String::from_utf8(retained.query.clone()).unwrap(),
            authority,
            policy: v3::Policy::default(),
            nonce: [53; 32],
        },
    }
}

/// Verifier-side test setup: the anchor comes from the retained original and the
/// declared synthetic salt, never from a holder-supplied witness.
fn agreed_anchor(data: &str) -> DatasetAuthority {
    let policy = v3::Policy::default();
    let dataset = replay::convert_turtle(data, &[], &policy)
        .unwrap()
        .into_dataset(SALT);
    DatasetAuthority::VerifierAgreed {
        commitment: v3::dataset_commitment(&dataset, &policy).unwrap(),
    }
}

fn expected(retained: &Retained, result: v3::CanonicalResult) -> ExpectedManifest {
    ExpectedManifest {
        schema: replay::EXPECTED_SCHEMA.into(),
        query_sha256: hex(&retained.query),
        data_sha256: hex(&retained.data),
        oracle: Oracle {
            kind: OracleKind::ReviewedDerivation,
            source: "hand-derived from the synthetic Turtle and SPARQL 1.1 BGP semantics".into(),
            reviewer: "engine_replay native contract test".into(),
        },
        result,
    }
}

/// Hand-derived from `DATA` and `QUERY`; never produced by the evaluator.
fn bgp_expected() -> v3::CanonicalResult {
    let row = |s: &str, o: &str| vec![Some(s.to_owned()), Some(o.to_owned())];
    v3::CanonicalResult::Select {
        variables: vec!["s".into(), "o".into()],
        order: RowOrder::Bag,
        rows: vec![
            row(
                "<http://ex/c>",
                "\"7\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            ),
            row("<http://ex/a>", "\"x\"@en"),
            row("<http://ex/a>", "<http://ex/b>"),
        ],
    }
}

fn holder_prepared(retained: &Retained) -> replay::Prepared {
    replay::prepare(
        retained.originals(),
        &verifier(retained, DatasetAuthority::HolderDeclared),
        &holder(),
    )
    .unwrap()
}

fn drop_first_statement(witness: &mut v3::Witness) {
    let end = witness.dataset.nquads.find('\n').unwrap() + 1;
    witness.dataset.nquads.replace_range(..end, "");
}

#[test]
fn original_turtle_converts_to_exact_nquads_without_lexical_normalization() {
    let turtle = r#"@base <http://example.org/base/> .
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:s a ex:C ;
  ex:int 01 ;
  ex:dec -1.50 ;
  ex:dbl 1E3 ;
  ex:bool true ;
  ex:date "2024-01-01"^^xsd:date ;
  ex:lang "chat"@en ;
  ex:str 'plain' , "say \"hi\"" .
<relative> ex:p _:x .
_:x ex:q _:y .
_:y ex:q [] .
"#;
    let converted = replay::convert_turtle(turtle, &[], &v3::Policy::default()).unwrap();
    assert_eq!(
        converted.nquads(),
        r#"<http://ex/s> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/C> .
<http://ex/s> <http://ex/int> "01"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://ex/s> <http://ex/dec> "-1.50"^^<http://www.w3.org/2001/XMLSchema#decimal> .
<http://ex/s> <http://ex/dbl> "1E3"^^<http://www.w3.org/2001/XMLSchema#double> .
<http://ex/s> <http://ex/bool> "true"^^<http://www.w3.org/2001/XMLSchema#boolean> .
<http://ex/s> <http://ex/date> "2024-01-01"^^<http://www.w3.org/2001/XMLSchema#date> .
<http://ex/s> <http://ex/lang> "chat"@en .
<http://ex/s> <http://ex/str> "plain" .
<http://ex/s> <http://ex/str> "say \"hi\"" .
<http://example.org/base/relative> <http://ex/p> _:b0 .
_:b0 <http://ex/q> _:b1 .
_:b1 <http://ex/q> _:b2 .
"#
    );
    assert_eq!(converted.statements(), 12);
    assert_eq!(converted.blank_nodes(), 3);
    assert!(converted.named_graphs().is_empty());
    // Duplicate statements are source statements, retained before V3 deduplication.
    let duplicated = "<http://ex/s> <http://ex/p> <http://ex/o> .\n".repeat(2);
    let converted = replay::convert_turtle(&duplicated, &[], &v3::Policy::default()).unwrap();
    assert_eq!(converted.nquads(), duplicated);
    assert_eq!(converted.statements(), 2);
}

#[test]
fn conversion_is_deterministic_and_independent_of_source_blank_labels() {
    let policy = v3::Policy::default();
    let labelled = "@prefix ex: <http://ex/> .\n_:x ex:p _:y .\n_:y ex:p _:x .\n";
    let renamed = labelled.replace("_:x", "_:first").replace("_:y", "_:second");
    let a = replay::convert_turtle(labelled, &[], &policy).unwrap();
    let b = replay::convert_turtle(&renamed, &[], &policy).unwrap();
    assert_eq!(a.nquads(), b.nquads());
    assert!(!b.nquads().contains("first"));
    // Anonymous nodes and collections receive parser labels; relabeling makes
    // repeated conversions identical and never leaks those labels.
    let nested = "@prefix ex: <http://ex/> .\nex:a ex:p [ ex:q [ ex:r ( 1 2 ) ] ] .\n";
    let first = replay::convert_turtle(nested, &[], &policy).unwrap();
    let second = replay::convert_turtle(nested, &[], &policy).unwrap();
    assert_eq!(first.nquads(), second.nquads());
    assert_eq!(first.statements(), 7);
    assert_eq!(first.blank_nodes(), 4);
    for token in first.nquads().split_whitespace() {
        if let Some(label) = token.strip_prefix("_:") {
            assert!(
                label.strip_prefix('b').is_some_and(|n| n.parse::<u32>().is_ok()),
                "unexpected blank label {token}"
            );
        }
    }
}

#[test]
fn malformed_unknown_base_and_blank_graph_names_are_distinct_source_errors() {
    let policy = v3::Policy::default();
    let convert = |turtle: &str, named: &[String]| {
        replay::convert_turtle(turtle, named, &policy).unwrap_err()
    };
    assert_eq!(convert("<http://ex/s> <http://ex/p> .", &[]), SourceError::Syntax);
    assert_eq!(
        convert("<s> <http://ex/p> <http://ex/o> .", &[]),
        SourceError::RelativeIriWithoutBase
    );
    assert_eq!(
        convert("@base <relative/> .\n<s> <http://ex/p> <http://ex/o> .", &[]),
        SourceError::RelativeIriWithoutBase
    );
    // A later syntax error keeps the document malformed, not merely base-less.
    assert_eq!(
        convert("<s> <http://ex/p> <http://ex/o> .\n<http://ex/s> <http://ex/p> .", &[]),
        SourceError::Syntax
    );
    assert_eq!(convert("", &["_:g".into()]), SourceError::BlankGraphName);
    // Other catalog rules remain V3's own; the catalog is carried unchanged.
    let converted = replay::convert_turtle("", &["http://ex/empty".into()], &policy).unwrap();
    assert_eq!(converted.named_graphs(), ["http://ex/empty".to_owned()]);
    for (error, disposition) in [
        (SourceError::Syntax, Disposition::Malformed),
        (SourceError::RelativeIriWithoutBase, Disposition::Malformed),
        (SourceError::BlankGraphName, Disposition::Unsupported),
        (SourceError::Capacity, Disposition::Capacity),
    ] {
        assert_eq!(ReplayError::Source(error).disposition(), disposition);
        assert_eq!(ReplayError::Source(error).stage(), "source");
    }
}

#[test]
fn converted_bytes_are_bounded_by_the_request_policy_before_expansion_grows() {
    let line = "<http://ex/s> <http://ex/p> <http://ex/o> .\n";
    let mut policy = v3::Policy::default();
    policy.dataset.max_dataset_bytes = u32::try_from(line.len()).unwrap();
    let converted = replay::convert_turtle(line, &[], &policy).unwrap();
    assert_eq!(converted.nquads(), line);
    // Catalog IRI bytes share the same V3 byte accounting.
    assert_eq!(
        replay::convert_turtle(line, &["http://ex/g".into()], &policy).unwrap_err(),
        SourceError::Capacity
    );
    // A short prefixed document expanding beyond the policy stops early.
    let expanding = format!(
        "@prefix x: <http://ex/{}> .\nx:a x:b x:c , x:d .\n",
        "l".repeat(20_000)
    );
    assert_eq!(
        replay::convert_turtle(&expanding, &[], &v3::Policy::default()).unwrap_err(),
        SourceError::Capacity
    );
}

#[test]
fn both_authorities_prepare_the_original_and_match_the_hand_derived_oracle() {
    let retained = Retained::new(QUERY, DATA);
    let oracle = expected(&retained, bgp_expected());
    for (authority, provenance) in [
        (DatasetAuthority::HolderDeclared, Provenance::HolderDeclaredOnly),
        (agreed_anchor(DATA), Provenance::VerifierAcceptedCommitment),
    ] {
        let verifier = verifier(&retained, authority);
        let prepared = replay::prepare(retained.originals(), &verifier, &holder()).unwrap();
        assert_eq!(prepared.witness().request, verifier.request, "request unchanged");
        assert_eq!(prepared.witness().dataset.salt, SALT);
        assert!(prepared.witness().dataset.named_graphs.is_empty());
        assert_eq!(prepared.cell(), &verifier.cell);
        assert_eq!(prepared.originals(), &verifier.originals);
        assert_eq!(prepared.native_comparison(), "agreement");
        let journal = prepared.evaluate_against(&oracle).unwrap();
        assert_eq!(journal.provenance, provenance);
        assert_eq!(journal.dataset_commitment, prepared.dataset_commitment());
        v3::bind_journal(&journal, &verifier.request).unwrap();
        let summary = prepared.private_summary();
        assert_eq!(
            summary.nquads_sha256,
            hex(prepared.witness().dataset.nquads.as_bytes())
        );
        assert_eq!((summary.statements, summary.blank_nodes), (3, 0));
    }
    // Storage and profile copies with byte-identical originals share one
    // expectation; the verifier cell still identifies each retained record.
    let copy = Retained::with(QUERY, DATA, |record| record["storage"] = json!("mmap"));
    let mut manifest = verifier(&copy, DatasetAuthority::HolderDeclared);
    manifest.cell.storage = "mmap".into();
    let prepared = replay::prepare(copy.originals(), &manifest, &holder()).unwrap();
    prepared.evaluate_against(&oracle).unwrap();
}

#[test]
fn observations_and_record_fields_never_become_input_or_expectations() {
    let retained = Retained::with(QUERY, DATA, |record| {
        record["request"] = json!({"query": "ASK {}", "nonce": [1; 32]});
        record["expected_result"] = json!({"Ask": true});
        record["input"]["authority"] = json!("HolderDeclared");
    });
    let verifier = verifier(&retained, agreed_anchor(DATA));
    let prepared = replay::prepare(retained.originals(), &verifier, &holder()).unwrap();
    let converted = replay::convert_turtle(DATA, &[], &v3::Policy::default()).unwrap();
    assert_eq!(prepared.witness().dataset.nquads, converted.nquads());
    assert!(!prepared.witness().dataset.nquads.contains("observed"));
    assert_eq!(prepared.witness().request, verifier.request);
    prepared
        .evaluate_against(&expected(&retained, bgp_expected()))
        .unwrap();
    // Debug output redacts the dataset, salt and witness digests.
    for rendered in [
        format!("{prepared:?}"),
        format!("{:?}", prepared.private_summary()),
        format!("{:?}", holder()),
        format!("{:?}", retained.originals()),
        format!("{converted:?}"),
    ] {
        assert!(
            !rendered.contains("http://ex/a") && !rendered.contains("41, 41"),
            "{rendered}"
        );
        assert!(!rendered.contains(&prepared.private_summary().nquads_sha256));
    }
}

#[test]
fn retained_record_and_file_identity_mismatches_fail_closed() {
    let retained = Retained::new(QUERY, DATA);
    let prepare = |retained: &Retained, verifier: &VerifierManifest| {
        replay::prepare(retained.originals(), verifier, &holder()).unwrap_err()
    };
    let base = verifier(&retained, DatasetAuthority::HolderDeclared);
    for file in [RetainedFile::Record, RetainedFile::Query, RetainedFile::Data] {
        let mut changed = base.clone();
        let digest = match file {
            RetainedFile::Record => &mut changed.originals.record_sha256,
            RetainedFile::Query => &mut changed.originals.query_sha256,
            RetainedFile::Data => &mut changed.originals.data_sha256,
        };
        *digest = hex(b"another retained file");
        assert_eq!(prepare(&retained, &changed), ReplayError::OriginalDigest(file));
    }
    // Verifier digests match the files, but the record's copies do not.
    for (edited, file) in [
        (
            Retained::with(QUERY, DATA, |r| r["input"]["query"] = json!(format!("{QUERY} "))),
            RetainedFile::Query,
        ),
        (
            Retained::with(QUERY, DATA, |r| r["input"]["dataset"] = json!(format!("{DATA}\n"))),
            RetainedFile::Data,
        ),
    ] {
        let manifest = verifier(&edited, DatasetAuthority::HolderDeclared);
        assert_eq!(
            prepare(&edited, &manifest),
            ReplayError::RecordFileMismatch(file)
        );
    }
    let mut changed = base.clone();
    changed.cell.storage = "mmap".into();
    assert_eq!(prepare(&retained, &changed), ReplayError::CellMismatch);
    let reseeded = Retained::with(QUERY, DATA, |r| r["seed"] = json!(0));
    let manifest = verifier(&reseeded, DatasetAuthority::HolderDeclared);
    assert_eq!(prepare(&reseeded, &manifest), ReplayError::CellMismatch);
    let trig = Retained::with(QUERY, DATA, |r| r["input"]["format"] = json!("trig"));
    let manifest = verifier(&trig, DatasetAuthority::HolderDeclared);
    assert_eq!(prepare(&trig, &manifest), ReplayError::UnsupportedFormat);
    let schema = Retained::with(QUERY, DATA, |r| {
        r["schema"] = json!("sparq.engine-seed-replay.v2")
    });
    let manifest = verifier(&schema, DatasetAuthority::HolderDeclared);
    assert_eq!(prepare(&schema, &manifest), ReplayError::RecordSchema);
    let mut truncated = Retained::new(QUERY, DATA);
    truncated.record.truncate(10);
    let manifest = verifier(&truncated, DatasetAuthority::HolderDeclared);
    assert_eq!(prepare(&truncated, &manifest), ReplayError::RecordSyntax);
    for error in [
        ReplayError::OriginalDigest(RetainedFile::Data),
        ReplayError::RecordFileMismatch(RetainedFile::Query),
        ReplayError::CellMismatch,
    ] {
        assert_eq!(error.disposition(), Disposition::ContractViolation);
    }
}

#[test]
fn verifier_request_query_and_anchor_mismatches_fail_closed() {
    let retained = Retained::new(QUERY, DATA);
    let prepare = |verifier: &VerifierManifest, holder: &HolderManifest| {
        replay::prepare(retained.originals(), verifier, holder).unwrap_err()
    };
    let mut changed = verifier(&retained, DatasetAuthority::HolderDeclared);
    changed.request.query.push(' ');
    assert_eq!(prepare(&changed, &holder()), ReplayError::RequestQuery);
    let mut changed = verifier(&retained, DatasetAuthority::HolderDeclared);
    changed.request.nonce = [0; 32];
    assert_eq!(
        prepare(&changed, &holder()),
        ReplayError::Request(Rejected("V3 query size or challenge rejected"))
    );
    let mut changed = verifier(&retained, DatasetAuthority::HolderDeclared);
    changed.request.version = 2;
    assert_eq!(
        prepare(&changed, &holder()),
        ReplayError::Request(Rejected("unsupported V3 proof contract or version"))
    );
    // An agreed commitment is compared, never replaced by the holder's value.
    let wrong = verifier(
        &retained,
        DatasetAuthority::VerifierAgreed {
            commitment: [9; 32],
        },
    );
    assert_eq!(prepare(&wrong, &holder()), ReplayError::AnchorMismatch);
    let mut other_salt = holder();
    other_salt.salt = HolderSalt::SyntheticFixed([42; 32]);
    let agreed = verifier(&retained, agreed_anchor(DATA));
    assert_eq!(prepare(&agreed, &other_salt), ReplayError::AnchorMismatch);
    let mut stricter = agreed.clone();
    stricter.request.policy.dataset.max_rows -= 1;
    assert_eq!(prepare(&stricter, &holder()), ReplayError::AnchorMismatch);
    assert_eq!(
        ReplayError::AnchorMismatch.disposition(),
        Disposition::ContractViolation
    );
    assert_eq!(ReplayError::AnchorMismatch.stage(), "commitment");
}

#[test]
fn independent_expected_result_mismatch_is_not_agreement() {
    let retained = Retained::new(QUERY, DATA);
    let prepared = holder_prepared(&retained);
    let mut missing = bgp_expected();
    let v3::CanonicalResult::Select { rows, .. } = &mut missing else {
        unreachable!()
    };
    rows.pop();
    assert_eq!(
        prepared
            .evaluate_against(&expected(&retained, missing))
            .unwrap_err(),
        ReplayError::ExpectedMismatch
    );
    // A numerically equal but lexically different term is a different RDF term.
    let mut normalized = bgp_expected();
    let v3::CanonicalResult::Select { rows, .. } = &mut normalized else {
        unreachable!()
    };
    rows[0][1] = Some("\"7.0\"^^<http://www.w3.org/2001/XMLSchema#decimal>".into());
    assert_eq!(
        prepared
            .evaluate_against(&expected(&retained, normalized))
            .unwrap_err(),
        ReplayError::ExpectedMismatch
    );
    let mut ordered = bgp_expected();
    let v3::CanonicalResult::Select { order, rows, .. } = &mut ordered else {
        unreachable!()
    };
    *order = RowOrder::Sequence;
    rows.reverse();
    assert_eq!(
        prepared
            .evaluate_against(&expected(&retained, ordered))
            .unwrap_err(),
        ReplayError::ExpectedMismatch
    );
    // Bag expectations are multisets: listing order is not significant.
    let mut reordered = bgp_expected();
    let v3::CanonicalResult::Select { rows, .. } = &mut reordered else {
        unreachable!()
    };
    rows.rotate_left(1);
    prepared
        .evaluate_against(&expected(&retained, reordered))
        .unwrap();
    let mut other = expected(&retained, bgp_expected());
    other.data_sha256 = hex(b"another dataset");
    assert_eq!(
        prepared.evaluate_against(&other).unwrap_err(),
        ReplayError::ExpectedBinding
    );
    assert_eq!(
        ReplayError::ExpectedMismatch.disposition(),
        Disposition::ContractViolation
    );
}

#[test]
fn blank_node_expectations_use_one_global_bijection() {
    let retained = Retained::new(QUERY, DATA);
    let table = |rows: &[(&str, &str)]| v3::CanonicalResult::Select {
        variables: vec!["s".into(), "o".into()],
        order: RowOrder::Bag,
        rows: rows
            .iter()
            .map(|(s, o)| vec![Some((*s).to_owned()), Some((*o).to_owned())])
            .collect(),
    };
    let actual = table(&[
        ("_:c14n0", "<http://ex/a>"),
        ("_:c14n0", "<http://ex/b>"),
        ("_:c14n1", "<http://ex/a>"),
    ]);
    let oracle = |rows: &[(&str, &str)]| expected(&retained, table(rows));
    let same = oracle(&[
        ("_:y", "<http://ex/a>"),
        ("_:x", "<http://ex/a>"),
        ("_:x", "<http://ex/b>"),
    ]);
    assert!(same.matches(&actual).unwrap());
    let swapped = oracle(&[
        ("_:x", "<http://ex/a>"),
        ("_:y", "<http://ex/a>"),
        ("_:y", "<http://ex/a>"),
    ]);
    assert!(!swapped.matches(&actual).unwrap());
    let per_row = oracle(&[
        ("_:x", "<http://ex/a>"),
        ("_:y", "<http://ex/b>"),
        ("_:z", "<http://ex/a>"),
    ]);
    assert!(!per_row.matches(&actual).unwrap(), "per-row relabeling loses identity");
    let labels: Vec<String> = (0..12).map(|n| format!("_:n{n}")).collect();
    let many: Vec<(&str, &str)> = labels
        .iter()
        .map(|label| (label.as_str(), "<http://ex/a>"))
        .collect();
    assert_eq!(
        oracle(&many).matches(&table(&many)).unwrap_err(),
        ReplayError::OracleCapacity
    );
    assert_eq!(
        ReplayError::OracleCapacity.disposition(),
        Disposition::Capacity
    );
    // ASK and graph forms compare exactly.
    let ask = expected(&retained, v3::CanonicalResult::Ask(false));
    assert!(ask.matches(&v3::CanonicalResult::Ask(false)).unwrap());
    assert!(!ask.matches(&v3::CanonicalResult::Ask(true)).unwrap());
    assert!(!ask.matches(&actual).unwrap());
}

#[test]
fn unsupported_profile_refusal_is_distinct_from_unexpected_failure() {
    // A registered V3 admission refusal is the only exclusion class.
    let rand = Retained::new("SELECT (RAND() AS ?x) WHERE {}", DATA);
    let error = holder_prepared(&rand)
        .evaluate_against(&expected(&rand, v3::CanonicalResult::Ask(true)))
        .unwrap_err();
    assert_eq!(
        error,
        ReplayError::Admission(Rejected(
            "nondeterministic or RDF 1.2 function is not admitted"
        ))
    );
    assert_eq!((error.stage(), error.disposition()), ("admission", Disposition::Unsupported));
    // A V3 diagnostic absent from the reviewed registry is never an exclusion.
    let exists = Retained::new(
        "PREFIX ex: <http://ex/> ASK { ?s ex:p ?o FILTER EXISTS { ?s ex:p ?x } }",
        "@prefix ex: <http://ex/> .\nex:a ex:p _:b .\n",
    );
    let error = holder_prepared(&exists)
        .evaluate_against(&expected(&exists, v3::CanonicalResult::Ask(true)))
        .unwrap_err();
    assert_eq!(
        error,
        ReplayError::Evaluation(EvaluationError::Rejected(Rejected(
            "V3 EXISTS blank-node correlation is not admitted"
        )))
    );
    assert_eq!((error.stage(), error.disposition()), ("evaluation", Disposition::Unexpected));
    // Malformed original data is not a profile exclusion either.
    let malformed = Retained::new(QUERY, "<http://ex/s> <http://ex/p> .");
    let manifest = verifier(&malformed, DatasetAuthority::HolderDeclared);
    let error = replay::prepare(malformed.originals(), &manifest, &holder()).unwrap_err();
    assert_eq!(error, ReplayError::Source(SourceError::Syntax));
    assert_eq!(error.disposition(), Disposition::Malformed);
    // Typed engine causes follow the registry; only row/byte/domain limits are capacity.
    for (cause, disposition) in [
        (EvaluationError::Budget(BudgetExceeded::Rows), Disposition::Capacity),
        (EvaluationError::Budget(BudgetExceeded::Bytes), Disposition::Capacity),
        (
            EvaluationError::Capacity(EvaluationCapacity::NumericRepresentation),
            Disposition::Capacity,
        ),
        (
            EvaluationError::Capacity(EvaluationCapacity::TemporalYear),
            Disposition::Capacity,
        ),
        (EvaluationError::Budget(BudgetExceeded::Deadline), Disposition::Unexpected),
        (EvaluationError::Budget(BudgetExceeded::Cancelled), Disposition::Unexpected),
        (EvaluationError::Execution, Disposition::Unexpected),
        (
            EvaluationError::Rejected(Rejected("query evaluation or resource budget rejected")),
            Disposition::Unexpected,
        ),
    ] {
        assert_eq!(ReplayError::Evaluation(cause).disposition(), disposition);
    }
    assert_eq!(
        ReplayError::Commitment(Rejected("V2 dataset byte capacity rejected")).disposition(),
        Disposition::Capacity
    );
    assert_eq!(
        ReplayError::Admission(Rejected("SPARQL parse rejected")).disposition(),
        Disposition::Malformed
    );
}

#[test]
fn agreed_omission_fails_its_anchor_but_holder_omission_is_only_an_oracle_mismatch() {
    let retained = Retained::new(QUERY, DATA);
    let oracle = expected(&retained, bgp_expected());
    // Malicious witness under a verifier-agreed anchor: the fixed anchor rejects it.
    let agreed = verifier(&retained, agreed_anchor(DATA));
    let prepared = replay::prepare(retained.originals(), &agreed, &holder()).unwrap();
    let mut omitted = prepared.witness().clone();
    drop_first_statement(&mut omitted);
    assert_eq!(
        v3::evaluate_detailed(&omitted).unwrap_err(),
        EvaluationError::Rejected(Rejected("V3 complete dataset anchor mismatch"))
    );
    // Holder-declared scope: the same omission is a valid computation over the
    // holder's chosen bytes. Verification binding accepts it; this is not a
    // rejected wallet-incompleteness check. Only the independent oracle differs.
    let prepared = holder_prepared(&retained);
    let mut omitted = prepared.witness().clone();
    drop_first_statement(&mut omitted);
    let journal = v3::evaluate(&omitted).unwrap();
    v3::bind_journal(&journal, &prepared.witness().request).unwrap();
    assert_eq!(journal.provenance, Provenance::HolderDeclaredOnly);
    assert_ne!(journal.dataset_commitment, prepared.dataset_commitment());
    assert!(!oracle.matches(&journal.result).unwrap());
}

#[test]
fn manifests_reject_unknown_fields_and_other_protocol_versions() {
    let retained = Retained::new(QUERY, DATA);
    let base = verifier(&retained, DatasetAuthority::HolderDeclared);
    let prepare = |verifier: &VerifierManifest, holder: &HolderManifest| {
        replay::prepare(retained.originals(), verifier, holder).unwrap_err()
    };
    let mut smuggled = serde_json::to_value(&base).unwrap();
    smuggled["expected_result"] = json!({"Ask": true});
    let error = serde_json::from_value::<VerifierManifest>(smuggled).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
    let mut unknown_salt = serde_json::to_value(holder()).unwrap();
    unknown_salt["salt"] = json!({"OperatingSystem": null});
    let error = serde_json::from_value::<HolderManifest>(unknown_salt).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
    let mut changed = base.clone();
    changed.schema = "sparq.engine-replay-proof.verifier.v2".into();
    assert_eq!(
        prepare(&changed, &holder()),
        ReplayError::Manifest(ManifestError::VerifierSchema)
    );
    let mut changed = base.clone();
    changed.cell.profile = "../shipped".into();
    assert_eq!(
        prepare(&changed, &holder()),
        ReplayError::Manifest(ManifestError::CellIdentity)
    );
    let mut changed = base.clone();
    changed.originals.data_sha256 = changed.originals.data_sha256.to_uppercase();
    assert_eq!(
        prepare(&changed, &holder()),
        ReplayError::Manifest(ManifestError::DigestFormat)
    );
    let mut other = holder();
    other.schema = "sparq.engine-replay-proof.holder.v0".into();
    assert_eq!(
        prepare(&base, &other),
        ReplayError::Manifest(ManifestError::HolderSchema)
    );
    let mut undeclared = holder();
    undeclared.synthetic_inputs = false;
    assert_eq!(
        prepare(&base, &undeclared),
        ReplayError::Manifest(ManifestError::FixedSaltWithoutSyntheticInputs)
    );
    let mut zero = holder();
    zero.salt = HolderSalt::SyntheticFixed([0; 32]);
    assert_eq!(
        prepare(&base, &zero),
        ReplayError::Manifest(ManifestError::ZeroSalt)
    );
    let prepared = holder_prepared(&retained);
    let mut schema = expected(&retained, bgp_expected());
    schema.schema = "sparq.engine-replay-proof.expected.v2".into();
    assert_eq!(
        prepared.evaluate_against(&schema).unwrap_err(),
        ReplayError::Manifest(ManifestError::ExpectedSchema)
    );
    let mut unreviewed = expected(&retained, bgp_expected());
    unreviewed.oracle.reviewer = " ".into();
    assert_eq!(
        prepared.evaluate_against(&unreviewed).unwrap_err(),
        ReplayError::Manifest(ManifestError::OracleProvenance)
    );
    assert_eq!(
        ReplayError::Manifest(ManifestError::ZeroSalt).stage(),
        "manifest"
    );
}
