// [OPUS-5.5] Synthetic experiment setup for the engine replay proof bridge.
//! Derive holder and verifier manifests for one explicitly synthetic replay cell.
//!
//! See `bench/zk-bindings/engine-proof-replay.md` for usage and trust boundary.
//! One host process plays verifier and holder for a synthetic experiment, so the
//! verifier-agreed anchor is test-setup trust input, not source authentication.
//! It creates no proof, guest artifact or pin. Experimental; not externally audited.
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::replay::{
    self, Cell, ExpectedManifest, HolderManifest, HolderSalt, OriginalDigests, Originals,
    ReplayError, VerifierManifest,
};
use sparq_proved_evaluator_model::{DatasetAuthority, ProofContract, v3};
use std::{
    error::Error as StdError,
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

type Result<T> = std::result::Result<T, Box<dyn StdError>>;

const HELP: &str = "Usage:
  engine_replay_setup synthetic REPLAY_DIR PROFILE EXPECTED.json RUN_ID NEW_OUTPUT_DIR
Synthetic test inputs only; there is no production mode. REPLAY_DIR is one
retained native cell with record.json, query.rq and data.ttl. EXPECTED.json is an
independently authored expectation and is copied unchanged. RUN_ID is a fresh
caller-chosen experiment label (1-64 of A-Z a-z 0-9 - _), never a production
challenge source. NEW_OUTPUT_DIR must not exist and must be outside the source
checkout and the replay directory. Creates no proof, guest or pin.
Exit 0: manifests written after both authorities matched EXPECTED.json natively.
Exit 1: typed setup refusal; nothing written. Exit 2: usage or infrastructure failure.";

const SETUP_SCHEMA: &str = "sparq.engine-replay-proof.setup.v1";
/// Published synthetic salt; reproducible by design and never production blinding.
const SYNTHETIC_SALT: [u8; 32] = *b"sparq-engine-replay-synthetic-01";
/// Domain of the synthetic challenge derivation; bump the suffix if inputs change.
const NONCE_DOMAIN: &[u8] = b"sparq:engine-replay-setup:synthetic-nonce:v1\0";
const HOLDER_DECLARED: &str = "holder_declared";
const VERIFIER_AGREED: &str = "verifier_agreed";
/// Keeps RUN_ID a short reviewable label, matching the replay cell identifier bound.
const MAX_RUN_ID_BYTES: usize = 64;
/// Same retained-file and manifest bounds as `engine_replay_proof`.
const MAX_RETAINED_BYTES: usize = 16 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

/// Holder and verifier manifests, each natively checked against the expectation.
#[derive(Debug)]
struct Manifests {
    holder: HolderManifest,
    holder_declared: VerifierManifest,
    verifier_agreed: VerifierManifest,
}

/// Setup refusal; no output is written for any of these.
#[derive(Debug, PartialEq)]
enum SetupError {
    /// RUN_ID is empty, too long or not a safe label.
    RunId,
    /// A retained file lacks a field or encoding the setup needs.
    Record(&'static str),
    /// The verifier-side anchor could not be derived from the originals.
    Anchor(String),
    /// Existing preparation or native evaluation refused one authority.
    Refused(&'static str, ReplayError),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunId => f.write_str("RUN_ID must be 1-64 bytes of A-Z a-z 0-9 - _"),
            Self::Record(reason) => f.write_str(reason),
            Self::Anchor(reason) => write!(f, "anchor derivation rejected: {reason}"),
            Self::Refused(authority, error) => write!(
                f,
                "{authority}: {} ({:?}): {error}",
                error.stage(),
                error.disposition()
            ),
        }
    }
}

impl StdError for SetupError {}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id.len() <= MAX_RUN_ID_BYTES
        && run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Reproducible synthetic challenge; every part is length-prefixed.
fn challenge(run_id: &str, digests: &OriginalDigests, authority: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(NONCE_DOMAIN);
    for part in [
        run_id,
        digests.query_sha256.as_str(),
        digests.data_sha256.as_str(),
        authority,
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().into()
}

/// Reads cell identity and catalog; `replay::prepare` rechecks the whole record.
fn record_cell(record: &[u8], profile: &str) -> std::result::Result<(Cell, Vec<String>), SetupError> {
    let record: Value =
        serde_json::from_slice(record).map_err(|_| SetupError::Record("record.json is not JSON"))?;
    let text = |key: &str| record[key].as_str().map(str::to_owned);
    let (Some(seed), Some(category), Some(storage)) =
        (record["seed"].as_u64(), text("category"), text("storage"))
    else {
        return Err(SetupError::Record("record.json lacks seed, category or storage"));
    };
    let catalog: Vec<String> = serde_json::from_value(record["input"]["named_graph_catalog"].clone())
        .map_err(|_| SetupError::Record("record.json lacks its named-graph catalog"))?;
    let cell = Cell {
        profile: profile.to_owned(),
        category,
        seed,
        storage,
    };
    Ok((cell, catalog))
}

/// Derives both verifier manifests and checks each natively against `expected`.
///
/// The agreed anchor is computed from the original `data.ttl` and the record
/// catalog before any holder preparation. `expected` is only read.
fn setup(
    originals: Originals<'_>,
    profile: &str,
    run_id: &str,
    expected: &ExpectedManifest,
) -> std::result::Result<Manifests, SetupError> {
    if !valid_run_id(run_id) {
        return Err(SetupError::RunId);
    }
    let (cell, catalog) = record_cell(originals.record, profile)?;
    let digests = OriginalDigests {
        record_sha256: sha256(originals.record),
        query_sha256: sha256(originals.query),
        data_sha256: sha256(originals.data),
    };
    let query = std::str::from_utf8(originals.query)
        .map_err(|_| SetupError::Record("query.rq is not UTF-8"))?;
    let data = std::str::from_utf8(originals.data)
        .map_err(|_| SetupError::Record("data.ttl is not UTF-8"))?;
    let policy = v3::Policy::default();
    let source = replay::convert_turtle(data, &catalog, &policy)
        .map_err(|error| SetupError::Anchor(error.to_string()))?;
    let commitment = v3::dataset_commitment(&source.into_dataset(SYNTHETIC_SALT), &policy)
        .map_err(|error| SetupError::Anchor(error.to_string()))?;
    let manifest = |authority: DatasetAuthority, tag: &str| VerifierManifest {
        schema: replay::VERIFIER_SCHEMA.into(),
        cell: cell.clone(),
        originals: digests.clone(),
        request: v3::Request {
            version: v3::VERSION,
            contract: ProofContract::ExactDataset,
            dialect: v3::Dialect::SparqSparql11GraphResultsV3,
            query: query.to_owned(),
            authority,
            policy: policy.clone(),
            nonce: challenge(run_id, &digests, tag),
        },
    };
    let manifests = Manifests {
        holder: HolderManifest {
            schema: replay::HOLDER_SCHEMA.into(),
            synthetic_inputs: true,
            salt: HolderSalt::SyntheticFixed(SYNTHETIC_SALT),
        },
        holder_declared: manifest(DatasetAuthority::HolderDeclared, HOLDER_DECLARED),
        verifier_agreed: manifest(DatasetAuthority::VerifierAgreed { commitment }, VERIFIER_AGREED),
    };
    for (tag, verifier) in [
        (HOLDER_DECLARED, &manifests.holder_declared),
        (VERIFIER_AGREED, &manifests.verifier_agreed),
    ] {
        replay::prepare(originals, verifier, &manifests.holder)
            .and_then(|prepared| prepared.evaluate_against(expected))
            .map_err(|error| SetupError::Refused(tag, error))?;
    }
    Ok(manifests)
}

fn bounded_read(path: &Path, max: usize) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(format!("{} must be a regular file", path.display()).into());
    }
    let mut bytes = Vec::new();
    file.take((max + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(format!("{} exceeds its setup read bound", path.display()).into());
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

fn fresh_output(requested: &Path, replay: &Path) -> Result<PathBuf> {
    let name = requested
        .file_name()
        .ok_or("NEW_OUTPUT_DIR needs a final path component")?;
    let parent = match requested.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.canonicalize()?,
        _ => std::env::current_dir()?.canonicalize()?,
    };
    let output = parent.join(name);
    // Synthetic salt and witness-linked metadata never land in sources or originals.
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    if checkout
        .canonicalize()
        .is_ok_and(|checkout| output.starts_with(checkout))
    {
        return Err("NEW_OUTPUT_DIR must be outside the source checkout".into());
    }
    if output.starts_with(replay) {
        return Err("NEW_OUTPUT_DIR must be outside the retained replay directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&output)?;
    }
    #[cfg(not(unix))]
    fs::create_dir(&output)?;
    Ok(output)
}

fn run(args: &[PathBuf]) -> Result<PathBuf> {
    let [mode, replay, profile, expected_path, run_id, output] = args else {
        return Err(HELP.into());
    };
    if mode.as_os_str() != "synthetic" {
        return Err(HELP.into());
    }
    let profile = profile.to_str().ok_or("PROFILE must be UTF-8")?;
    let run_id = run_id.to_str().ok_or("RUN_ID must be UTF-8")?;
    let replay = replay.canonicalize()?;
    if !replay.is_dir() {
        return Err("REPLAY_DIR must be a directory".into());
    }
    let record = bounded_read(&replay.join("record.json"), MAX_RETAINED_BYTES)?;
    let query = bounded_read(&replay.join("query.rq"), MAX_RETAINED_BYTES)?;
    let data = bounded_read(&replay.join("data.ttl"), MAX_RETAINED_BYTES)?;
    let expected_bytes = bounded_read(expected_path, MAX_MANIFEST_BYTES)?;
    let expected: ExpectedManifest = serde_json::from_slice(&expected_bytes)?;
    let originals = Originals {
        record: &record,
        query: &query,
        data: &data,
    };
    let manifests = setup(originals, profile, run_id, &expected)?;
    let output = fresh_output(output, &replay)?;
    let files = [
        ("holder.json", serde_json::to_vec_pretty(&manifests.holder)?),
        ("holder-declared.json", serde_json::to_vec_pretty(&manifests.holder_declared)?),
        ("verifier-agreed.json", serde_json::to_vec_pretty(&manifests.verifier_agreed)?),
        ("expected.json", expected_bytes.clone()),
    ];
    let mut written = Map::new();
    for (name, bytes) in &files {
        write_new(&output.join(name), bytes)?;
        written.insert((*name).to_owned(), json!(sha256(bytes)));
    }
    let provenance = json!({
        "schema": SETUP_SCHEMA,
        "mode": "synthetic",
        "run_id": run_id,
        "run_id_role": "caller-chosen fresh experiment label; derived nonces are reproducible test challenges, never a production challenge source",
        "cell": manifests.holder_declared.cell,
        "replay_directory": replay.display().to_string(),
        "inputs": {
            "record_sha256": sha256(&record),
            "query_sha256": sha256(&query),
            "data_sha256": sha256(&data),
            "expected_manifest_sha256": sha256(&expected_bytes),
        },
        "outputs": written,
        "salt": "synthetic-fixed: published test salt, not production blinding",
        "nonce_derivation": "SHA-256 over the nonce domain, then u64-LE length and UTF-8 bytes of RUN_ID, query_sha256, data_sha256 and the authority tag",
        "native_checks": [
            "holder_declared: replay::prepare and evaluate_against matched the supplied expectation",
            "verifier_agreed: replay::prepare and evaluate_against matched the supplied expectation",
        ],
        "trust_boundary": {
            "host_only": "setup, Turtle conversion and native checks ran on this host; no guest executed any of them",
            "roles": "one process played verifier and holder for a synthetic experiment; the manifests are not independent of the holder",
            "verifier_agreed": "anchor computed here from original data.ttl and the record catalog with the published salt, before holder preparation; test-setup trust input, not source authentication",
            "holder_declared": "no anchor; computation over holder-chosen bytes only, no dataset authenticity or wallet completeness",
            "expected": "copied byte-for-byte from the caller's independently authored manifest; never derived or rewritten here",
            "not_generated": "no guest artifact, pin, tool identity or proof job; accept and assemble those independently",
        },
        "proof_count": 0,
        "verified_proof_count": 0,
        "assurance": "experimental and not externally audited",
    });
    write_new(&output.join("setup.json"), &serde_json::to_vec_pretty(&provenance)?)?;
    Ok(output)
}

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.len() == 1 && args[0] == Path::new("--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(output) => {
            println!("{}", output.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("engine replay setup: {error}");
            ExitCode::from(if error.downcast_ref::<SetupError>().is_some() { 1 } else { 2 })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_proved_evaluator_model::RowOrder;
    use sparq_proved_evaluator_model::replay::{ManifestError, Oracle, OracleKind, RetainedFile};

    const QUERY: &str = "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p ?o }";
    const DATA: &str = "@prefix ex: <http://ex/> .\nex:a ex:p ex:b , \"x\"@en .\nex:c ex:p 7 .\n";
    /// `DATA` with its last statement omitted.
    const OMITTED: &str = "@prefix ex: <http://ex/> .\nex:a ex:p ex:b , \"x\"@en .\n";

    struct Retained {
        record: Vec<u8>,
        query: Vec<u8>,
        data: Vec<u8>,
    }

    impl Retained {
        fn new(data: &str, edit: impl FnOnce(&mut Value)) -> Self {
            let mut record = json!({
                "schema": replay::RECORD_SCHEMA, "seed": 4695, "category": "bgp", "storage": "dense",
                "input": {"query": QUERY, "dataset": data, "format": "turtle", "named_graph_catalog": []},
                "comparison": {"status": "agreement"},
            });
            edit(&mut record);
            Self {
                record: serde_json::to_vec(&record).unwrap(),
                query: QUERY.as_bytes().to_vec(),
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

        fn digests(&self) -> OriginalDigests {
            OriginalDigests {
                record_sha256: sha256(&self.record),
                query_sha256: sha256(&self.query),
                data_sha256: sha256(&self.data),
            }
        }
    }

    /// Hand-derived from `QUERY` over `DATA` (or `OMITTED`); never evaluator output.
    fn expected(retained: &Retained, omitted: bool) -> ExpectedManifest {
        let row = |s: &str, o: &str| vec![Some(s.to_owned()), Some(o.to_owned())];
        let mut rows = vec![
            row("<http://ex/a>", "<http://ex/b>"),
            row("<http://ex/a>", "\"x\"@en"),
            row("<http://ex/c>", "\"7\"^^<http://www.w3.org/2001/XMLSchema#integer>"),
        ];
        if omitted {
            rows.pop();
        }
        ExpectedManifest {
            schema: replay::EXPECTED_SCHEMA.into(),
            query_sha256: sha256(&retained.query),
            data_sha256: sha256(&retained.data),
            oracle: Oracle {
                kind: OracleKind::ReviewedDerivation,
                source: "hand-derived from the synthetic Turtle and SPARQL 1.1 BGP semantics".into(),
                reviewer: "engine_replay_setup unit test".into(),
            },
            result: v3::CanonicalResult::Select {
                variables: vec!["s".into(), "o".into()],
                order: RowOrder::Bag,
                rows,
            },
        }
    }

    fn refused(error: ReplayError) -> SetupError {
        SetupError::Refused(HOLDER_DECLARED, error)
    }

    #[test]
    fn hand_derived_expectation_matches_under_both_authorities() {
        let cell = Retained::new(DATA, |_| {});
        let manifests = setup(cell.originals(), "shipped", "run-1", &expected(&cell, false)).unwrap();
        let (declared, agreed) = (&manifests.holder_declared, &manifests.verifier_agreed);
        assert_eq!(declared.request.authority, DatasetAuthority::HolderDeclared);
        assert!(matches!(agreed.request.authority, DatasetAuthority::VerifierAgreed { .. }));
        let identity = Cell {
            profile: "shipped".into(),
            category: "bgp".into(),
            seed: 4695,
            storage: "dense".into(),
        };
        assert_eq!((&declared.cell, &agreed.cell), (&identity, &identity));
        assert_eq!((&declared.originals, &agreed.originals), (&cell.digests(), &cell.digests()));
        assert_eq!(declared.request.query, QUERY);
        assert_eq!(agreed.request.policy, v3::Policy::default());
        assert!(manifests.holder.synthetic_inputs);
    }

    #[test]
    fn wrong_or_differently_bound_expectations_are_refused() {
        let cell = Retained::new(DATA, |_| {});
        let wrong = expected(&cell, true);
        let error = setup(cell.originals(), "shipped", "run-1", &wrong).unwrap_err();
        assert_eq!(error, refused(ReplayError::ExpectedMismatch));
        let mut other = expected(&cell, false);
        other.data_sha256 = sha256(OMITTED.as_bytes());
        let error = setup(cell.originals(), "shipped", "run-1", &other).unwrap_err();
        assert_eq!(error, refused(ReplayError::ExpectedBinding));
    }

    #[test]
    fn malformed_or_empty_run_ids_are_refused() {
        let cell = Retained::new(DATA, |_| {});
        let expected = expected(&cell, false);
        let long = "r".repeat(MAX_RUN_ID_BYTES + 1);
        for run_id in ["", " ", "run 1", "../run", "run/1", "run\n", "rün", long.as_str()] {
            let error = setup(cell.originals(), "shipped", run_id, &expected).unwrap_err();
            assert_eq!(error, SetupError::RunId, "{run_id:?}");
        }
        let longest = "r".repeat(MAX_RUN_ID_BYTES);
        setup(cell.originals(), "shipped", &longest, &expected).unwrap();
    }

    #[test]
    fn original_record_mismatches_are_refused() {
        let refusal = |cell: &Retained, profile: &str| {
            setup(cell.originals(), profile, "run-1", &expected(cell, false)).unwrap_err()
        };
        let data = Retained::new(DATA, |record| record["input"]["dataset"] = json!(OMITTED));
        assert_eq!(
            refusal(&data, "shipped"),
            refused(ReplayError::RecordFileMismatch(RetainedFile::Data))
        );
        let query = Retained::new(DATA, |record| record["input"]["query"] = json!("ASK {}"));
        assert_eq!(
            refusal(&query, "shipped"),
            refused(ReplayError::RecordFileMismatch(RetainedFile::Query))
        );
        let schema = Retained::new(DATA, |record| record["schema"] = json!("other"));
        assert_eq!(refusal(&schema, "shipped"), refused(ReplayError::RecordSchema));
        let seedless = Retained::new(DATA, |record| record["seed"] = Value::Null);
        assert!(matches!(refusal(&seedless, "shipped"), SetupError::Record(_)));
        let cell = Retained::new(DATA, |_| {});
        assert_eq!(
            refusal(&cell, "ship ped"),
            refused(ReplayError::Manifest(ManifestError::CellIdentity))
        );
    }

    #[test]
    fn challenges_are_nonzero_distinct_and_reproducible_per_run() {
        let cell = Retained::new(DATA, |_| {});
        let omitted = Retained::new(OMITTED, |_| {});
        let nonces = |cell: &Retained, omit: bool, run_id: &str| {
            let manifests = setup(cell.originals(), "shipped", run_id, &expected(cell, omit)).unwrap();
            [manifests.holder_declared.request.nonce, manifests.verifier_agreed.request.nonce]
        };
        let first = nonces(&cell, false, "run-a");
        assert_eq!(first, nonces(&cell, false, "run-a"));
        let all: Vec<[u8; 32]> = [first, nonces(&cell, false, "run-b"), nonces(&omitted, true, "run-a")]
            .into_iter()
            .flatten()
            .collect();
        for (index, nonce) in all.iter().enumerate() {
            assert_ne!(*nonce, [0; 32]);
            assert!(!all[index + 1..].contains(nonce));
        }
    }

    #[test]
    fn omission_under_the_agreed_anchor_stays_rejected() {
        let cell = Retained::new(DATA, |_| {});
        let expected = expected(&cell, false);
        let manifests = setup(cell.originals(), "shipped", "run-1", &expected).unwrap();
        // A holder substitutes consistent originals with one statement omitted.
        let omitted = Retained::new(OMITTED, |_| {});
        let mut agreed = manifests.verifier_agreed.clone();
        agreed.originals = omitted.digests();
        let error = replay::prepare(omitted.originals(), &agreed, &manifests.holder).unwrap_err();
        assert_eq!(error, ReplayError::AnchorMismatch);
        // Holder-declared computation over the omission is valid; only the oracle notices.
        let mut declared = manifests.holder_declared.clone();
        declared.originals = omitted.digests();
        let prepared = replay::prepare(omitted.originals(), &declared, &manifests.holder).unwrap();
        assert_eq!(prepared.evaluate_against(&expected).unwrap_err(), ReplayError::ExpectedBinding);
    }

    #[test]
    fn invocation_requires_the_synthetic_mode_word() {
        let paths = |items: &[&str]| items.iter().map(PathBuf::from).collect::<Vec<_>>();
        for rejected in [
            paths(&["r", "p", "e", "id", "o"]),
            paths(&["production", "r", "p", "e", "id", "o"]),
            paths(&["synthetic", "r", "p", "e", "id"]),
        ] {
            assert!(matches!(run(&rejected), Err(error) if error.to_string() == HELP));
        }
    }
}
