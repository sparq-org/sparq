//! Query preparation and proof lifecycle over the shared native proving driver.
//! Credential authentication uses the shared Merkle suite and issuer membership.

use crate::{
    answer::Answer,
    capacity::Bounds,
    codegen,
    credential::{Credential, SuiteEncoder},
    interp::{self, EncodeTerm},
    lower::lower,
    plan::Plan,
    request::{self, Request},
    statement::{public_input_bytes, Layout, Statement},
    witness, Error,
};
use oxrdf::Term;
use sparq_query_protocol::{consume_challenge, ChallengeStore, Identifier, MethodDescriptor};
use sparq_zk_compose::driver::ProgramProver;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// A locally compiled shape and its exact verification key.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub plan: Plan,
    pub verification_key: Vec<u8>,
    pub descriptor: MethodDescriptor,
    /// Locally compiled artifact, for reproducibility checks and export.
    pub artifact: serde_json::Value,
}

/// A presentation carries RDF results and evidence, never a plan, program,
/// verification key, ABI, capacity or public-input blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presentation {
    pub request_digest: [u8; 32],
    pub input_commitment: [u8; 32],
    pub signature_mode: u8,
    pub result: Answer,
    pub proof: Vec<u8>,
}
impl Presentation {
    /// Canonical local envelope for this draft primitive. The public adapter
    /// additionally applies its complete transport envelope's resource bound.
    pub fn encoded(&self) -> Result<Vec<u8>, Error> {
        let result = match &self.result {
            Answer::Ask(value) => serde_json::json!({"boolean":value}),
            Answer::Select { variables, rows } => serde_json::json!({"variables":variables,
                "rows":rows.iter().map(|r|r.iter().map(|t|t.as_ref().map(ToString::to_string)).collect::<Vec<_>>()).collect::<Vec<_>>() }),
        };
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        serde_json::to_vec(&serde_json::json!({"requestDigest":URL_SAFE_NO_PAD.encode(self.request_digest),
            "inputCommitment":URL_SAFE_NO_PAD.encode(self.input_commitment),"signatureMode":self.signature_mode,
            "result":result,"proof":URL_SAFE_NO_PAD.encode(&self.proof)})).map_err(infrastructure)
    }
}

struct Compiled {
    _workspace: tempfile::TempDir,
    driver: ProgramProver,
    key: Vec<u8>,
    artifact: serde_json::Value,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct CacheKey {
    plan: [u8; 32],
    library: [u8; 32],
}

/// In-process key cache indexed by shape hash and invalidated when trusted
/// library source changes. Keys are derived locally, never loaded from a request.
pub struct Compiler {
    root: PathBuf,
    zk: PathBuf,
    crs_path: Option<PathBuf>,
    cache: Mutex<BTreeMap<CacheKey, Arc<Compiled>>>,
}

impl Compiler {
    /// Both directories are local configuration. `zk` is the repository's trusted
    /// library tree; `root` holds private, isolated generated workspaces.
    /// Choose `root` outside any existing Nargo workspace.
    pub fn new(root: impl Into<PathBuf>, zk: impl Into<PathBuf>) -> Result<Self, Error> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(infrastructure)?;
        Ok(Self {
            root,
            zk: zk.into().canonicalize().map_err(infrastructure)?,
            crs_path: None,
            cache: Mutex::new(BTreeMap::new()),
        })
    }
    /// Select bb's CRS cache without changing its toolchain or verifier target.
    pub fn with_crs_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.crs_path = Some(path.into());
        self
    }
    pub fn prepare(&self, query: &str, bounds: Bounds) -> Result<Prepared, Error> {
        let plan = lower(query, bounds)?;
        self.prepare_plan(plan)
    }
    /// Select heterogeneous quad buckets from verifier-declared limits.
    pub fn prepare_with_credential_quad_bounds(
        &self,
        query: &str,
        bounds: Bounds,
        quad_bounds: &[u32],
    ) -> Result<Prepared, Error> {
        self.prepare_plan(lower(query, bounds)?.with_credential_quad_bounds(quad_bounds)?)
    }
    fn prepare_plan(&self, plan: Plan) -> Result<Prepared, Error> {
        let compiled = self.compiled(&plan)?;
        Ok(Prepared {
            descriptor: request::descriptor(&plan, &compiled.key)?,
            plan,
            verification_key: compiled.key.clone(),
            artifact: compiled.artifact.clone(),
        })
    }
    fn compiled(&self, plan: &Plan) -> Result<Arc<Compiled>, Error> {
        let files = library_files(&self.zk)?;
        let mut fingerprint = blake3::Hasher::new();
        for (name, bytes) in &files {
            fingerprint.update(&(name.len() as u64).to_be_bytes());
            fingerprint.update(name.as_bytes());
            fingerprint.update(&(bytes.len() as u64).to_be_bytes());
            fingerprint.update(bytes);
        }
        let cache_key = CacheKey {
            plan: plan.hash(),
            library: *fingerprint.finalize().as_bytes(),
        };
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| Error::Infrastructure("key cache lock".into()))?;
        if let Some(compiled) = cache.get(&cache_key) {
            return Ok(Arc::clone(compiled));
        }
        let package = codegen::generate(plan)?;
        let workspace = tempfile::Builder::new()
            .prefix("noir_query_")
            .tempdir_in(&self.root)
            .map_err(infrastructure)?;
        for (name, bytes) in files {
            write(workspace.path().join("zk").join(name), bytes)?;
        }
        for (name, bytes) in package.files {
            write(workspace.path().join(name), bytes)?;
        }
        let mut driver =
            ProgramProver::new(workspace.path(), &package.name).map_err(infrastructure)?;
        if let Some(path) = &self.crs_path {
            fs::create_dir_all(path).map_err(infrastructure)?;
            driver = driver.with_crs_path(path);
        }
        let artifact = driver.compile().map_err(infrastructure)?;
        let artifact = serde_json::from_slice(&fs::read(artifact).map_err(infrastructure)?)
            .map_err(infrastructure)?;
        Layout::new(plan).audit_authenticated_artifact(&artifact)?;
        let key = driver
            .canonical_vk(workspace.path())
            .map_err(infrastructure)?;
        let compiled = Arc::new(Compiled {
            _workspace: workspace,
            driver,
            key,
            artifact,
        });
        cache.insert(cache_key, Arc::clone(&compiled));
        Ok(compiled)
    }

    /// Check the request's key before reading private credential cells.
    /// The caller supplies a private independently generated presentation salt.
    pub fn prove(
        &self,
        request: &Request,
        credentials: &[Credential],
        vocabulary: &[Term],
        _encoder: &impl EncodeTerm,
        salt: [u8; 32],
        now: impl Fn() -> u64,
    ) -> Result<Presentation, Error> {
        let plan = request.plan()?;
        let compiled = self.checked_key(request, &plan)?;
        let issuer_root = request.issuer_root()?;
        freshness(request, now())?;
        let encoder = &SuiteEncoder;
        let cells = credentials
            .iter()
            .enumerate()
            .map(|(i, c)| c.cells(i as u32))
            .collect::<Result<Vec<_>, _>>()?;
        let trace = interp::evaluate(&plan, &cells, encoder)?;
        let result = Answer::release(&trace.result, &plan, vocabulary, encoder)?;
        let input_commitment = crate::credential::dataset_commitment(credentials, salt);
        agreement(request, input_commitment)?;
        let statement = Statement {
            request_digest: request.digest(),
            input_commitment,
            signature_mode: 1,
            result: result.encode(encoder)?,
        };
        let public = public_input_bytes(&Layout::new(&plan).reconstruct_authenticated(
            &plan,
            &statement,
            issuer_root,
        )?);
        let input = witness::prover_toml_authenticated(
            &plan,
            credentials,
            &trace,
            request.digest(),
            salt,
            issuer_root,
        )?;
        let work = tempfile::Builder::new()
            .prefix("proof_")
            .tempdir_in(&self.root)
            .map_err(infrastructure)?;
        let artifacts = compiled
            .driver
            .prove(&input, work.path(), "query")
            .map_err(infrastructure)?;
        if artifacts.vk != compiled.key {
            return Err(Error::Invalid("prover key mismatch".into()));
        }
        if artifacts.public_inputs != public {
            return Err(Error::Invalid("prover public-input mismatch".into()));
        }
        let presentation = Presentation {
            request_digest: statement.request_digest,
            input_commitment,
            signature_mode: 1,
            result,
            proof: artifacts.proof,
        };
        resource_bounds(request, &presentation)?;
        freshness(request, now())?;
        Ok(presentation)
    }

    /// Rebuild the statement from the stored query and RDF answer. Consume the
    /// original protocol challenge only after proof, policy and freshness checks.
    pub fn verify(
        &self,
        request: &Request,
        presentation: &Presentation,
        _encoder: &impl EncodeTerm,
        expected_audience: &Identifier,
        store: &dyn ChallengeStore,
        now: impl Fn() -> u64,
    ) -> Result<Answer, Error> {
        let encoder = &SuiteEncoder;
        let issuer_root = request.issuer_root()?;
        if request.stored().audience() != expected_audience {
            return Err(Error::Invalid("request audience".into()));
        }
        freshness(request, now())?;
        if presentation.request_digest != request.digest() {
            return Err(Error::Invalid("request digest".into()));
        }
        agreement(request, presentation.input_commitment)?;
        resource_bounds(request, presentation)?;
        let plan = request.plan()?;
        let compiled = self.checked_key(request, &plan)?;
        let statement = Statement {
            request_digest: request.digest(),
            input_commitment: presentation.input_commitment,
            signature_mode: presentation.signature_mode,
            result: presentation.result.encode(encoder)?,
        };
        let public = public_input_bytes(&Layout::new(&plan).reconstruct_authenticated(
            &plan,
            &statement,
            issuer_root,
        )?);
        let work = tempfile::Builder::new()
            .prefix("verify_")
            .tempdir_in(&self.root)
            .map_err(infrastructure)?;
        if !compiled
            .driver
            .verify_with(&presentation.proof, &public, &compiled.key, work.path())
            .map_err(infrastructure)?
        {
            return Err(Error::Invalid("query proof".into()));
        }
        freshness(request, now())?;
        let answer = presentation.result.clone();
        consume_challenge(store, request.stored()).map_err(Error::Protocol)?;
        Ok(answer)
    }
    fn checked_key(&self, request: &Request, plan: &Plan) -> Result<Arc<Compiled>, Error> {
        let compiled = self.compiled(plan)?;
        if compiled.key != request.key() {
            return Err(Error::Invalid("verification key mismatch".into()));
        }
        Ok(compiled)
    }
}

fn freshness(request: &Request, now: u64) -> Result<(), Error> {
    if now < request.stored().not_before() || now >= request.stored().not_after() {
        return Err(Error::Invalid("request validity window".into()));
    }
    Ok(())
}
fn agreement(request: &Request, commitment: [u8; 32]) -> Result<(), Error> {
    if request
        .stored()
        .requirements()
        .anchor()
        .is_some_and(|anchor| anchor.as_bytes() != &commitment)
    {
        return Err(Error::Invalid("dataset commitment agreement".into()));
    }
    Ok(())
}
fn resource_bounds(request: &Request, presentation: &Presentation) -> Result<(), Error> {
    let bounds = request.stored().requirements().resources();
    if let Answer::Select { rows, .. } = &presentation.result {
        if rows.len() > bounds.released_rows() as usize {
            return Err(Error::Capacity("released result".into()));
        }
    }
    if presentation.encoded()?.len() > bounds.presentation_bytes() as usize {
        return Err(Error::Capacity("presentation bytes".into()));
    }
    Ok(())
}
fn infrastructure(error: impl std::fmt::Display) -> Error {
    Error::Infrastructure(error.to_string())
}
fn write(path: PathBuf, bytes: impl AsRef<[u8]>) -> Result<(), Error> {
    fs::create_dir_all(path.parent().unwrap()).map_err(infrastructure)?;
    fs::write(path, bytes).map_err(infrastructure)
}
fn library_files(zk: &Path) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), Error> {
        for entry in fs::read_dir(path).map_err(infrastructure)? {
            let entry = entry.map_err(infrastructure)?;
            let path = entry.path();
            let ty = entry.file_type().map_err(infrastructure)?;
            if ty.is_dir() && entry.file_name() != "target" {
                visit(root, &path, out)?;
            } else if ty.is_file()
                && (path.extension().is_some_and(|x| x == "nr")
                    || path.file_name().is_some_and(|x| x == "Nargo.toml"))
            {
                let name = path
                    .strip_prefix(root)
                    .map_err(infrastructure)?
                    .to_str()
                    .ok_or_else(|| Error::Invalid("library path".into()))?
                    .to_owned();
                out.insert(name, fs::read(path).map_err(infrastructure)?);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    for dir in ["noir-query/core", "compose/compose_core", "dual-leaf-v2"] {
        visit(zk, &zk.join(dir), &mut out)?;
    }
    Ok(out)
}
