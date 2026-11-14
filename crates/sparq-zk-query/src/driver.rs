//! Query preparation and proof lifecycle over the shared native proving driver.
//! Credential authentication uses the shared Merkle suite and issuer membership.

use crate::{
    answer::Answer,
    capacity::{Bounds, OpeningProfile},
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
use sparq_query_protocol::{consume_challenge, ChallengeStore, Identifier};
use sparq_zk_compose::driver::ProgramProver;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub use crate::presentation::{Prepared, Presentation};

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
    /// Select lexical and metadata bounds from the verifier's request.
    pub fn prepare_with_opening_profile(
        &self,
        query: &str,
        bounds: Bounds,
        quad_bounds: Option<&[u32]>,
        profile: OpeningProfile,
    ) -> Result<Prepared, Error> {
        let mut plan = lower(query, bounds)?.with_opening_profile(profile);
        if let Some(quad_bounds) = quad_bounds {
            plan = plan.with_credential_quad_bounds(quad_bounds)?;
        }
        self.prepare_plan(plan)
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
        self.prove_with_envelope(
            request,
            credentials,
            vocabulary,
            salt,
            &witness::EnvelopeWitness::default(),
            now,
        )
    }

    /// Prove with private holder signatures and status paths for the stored policy.
    /// Public envelope values are reconstructed from `request`.
    pub fn prove_with_envelope(
        &self,
        request: &Request,
        credentials: &[Credential],
        vocabulary: &[Term],
        salt: [u8; 32],
        evidence: &witness::EnvelopeWitness,
        now: impl Fn() -> u64,
    ) -> Result<Presentation, Error> {
        let envelope = request.envelope_inputs()?;
        let plan = request.plan()?;
        let compiled = self.checked_key(request, &plan)?;
        freshness(request, now())?;
        let authorized;
        let credentials = if let Some(policy) = request.envelope_policy() {
            authorized = witness::authorize(credentials, policy, &envelope)?;
            authorized.as_slice()
        } else {
            credentials
        };
        for credential in credentials {
            if credential.canonical_issuer_root(envelope.issuer_depth)? != envelope.issuer_root {
                return Err(Error::Invalid("issuer membership root".into()));
            }
        }
        let encoder = &SuiteEncoder;
        let cells = credentials
            .iter()
            .enumerate()
            .map(|(i, c)| c.cells_with_profile(i as u32, plan.opening_profile()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut issuers = Vec::with_capacity(credentials.len());
        for credential in credentials {
            let issuer = Term::NamedNode(credential.trust_member()?.issuer);
            issuers.push(encoder.encode(&issuer)?);
        }
        let vocabulary = release_vocabulary(credentials, vocabulary)?;
        let dataset = crate::credential::assemble_dataset(&cells, &issuers, request.digest())?;
        let trace = interp::evaluate_dataset(&plan, &dataset, encoder)?;
        let result = Answer::release(&trace.result, &plan, &vocabulary, encoder)?;
        let input_commitment = crate::credential::dataset_commitment(credentials, salt)?;
        agreement(request, input_commitment)?;
        let statement = Statement {
            request_digest: request.digest(),
            input_commitment,
            signature_mode: 1,
            result: result.encode(encoder)?,
        };
        let public = public_input_bytes(
            &Layout::new(&plan)
                .reconstruct_authenticated_with_envelope(&plan, &statement, &envelope)?,
        );
        let input = witness::render_canonical(
            &plan,
            credentials,
            &trace,
            request.digest(),
            salt,
            witness::CanonicalInput {
                public: &envelope,
                evidence,
                revealed: None,
            },
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
        let envelope = request.envelope_inputs()?;
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
        let public = public_input_bytes(
            &Layout::new(&plan)
                .reconstruct_authenticated_with_envelope(&plan, &statement, &envelope)?,
        );
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

fn release_vocabulary(credentials: &[Credential], vocabulary: &[Term]) -> Result<Vec<Term>, Error> {
    let mut terms = vocabulary.to_vec();
    for credential in credentials {
        for quad in credential.quads()? {
            terms.extend([quad.subject.into(), quad.predicate.into(), quad.object]);
        }
    }
    terms.push(oxrdf::NamedNode::new_unchecked(crate::credential::VERIFIED_ISSUER).into());
    Ok(terms)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn released_vocabulary_covers_signed_metadata_and_the_issuer_assertion() {
        use crate::envelope_policy::{PolicyInput, TrustMember};
        use oxrdf::NamedNode;
        let key = sparq_zk::sig::SecretKey::from_seed(19);
        let source = "<urn:credential> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://www.w3.org/2018/credentials#VerifiableCredential> .\n<urn:credential> <https://www.w3.org/2018/credentials#issuer> <urn:issuer> .\n<urn:s> <urn:p> <urn:o> .\n";
        let config = "_:p <https://w3id.org/security#cryptosuite> \"schnorr-poseidon2-merkle-2026\" .\n_:p <https://w3id.org/security#verificationMethod> <urn:method> .\n";
        let data = [Credential {
            credential: sparq_merkle_suite::poseidon::issue(
                source,
                config,
                &key,
                sparq_merkle_suite::Fr::from(101u64),
            )
            .unwrap(),
            issuer: key.public_key(),
            issuer_index: 0,
            issuer_siblings: vec![],
        }];
        let policy = PolicyInput {
            trust_members: vec![TrustMember {
                issuer: NamedNode::new_unchecked("urn:issuer"),
                verification_method: NamedNode::new_unchecked("urn:method"),
                key: key.public_key(),
            }],
            ..Default::default()
        };
        let plan = lower(
            "SELECT ?p WHERE {?s ?p ?o}",
            Bounds {
                credentials: 1,
                quads_per_credential: 4,
                rows_per_table: 4,
            },
        )
        .unwrap();
        let dataset =
            crate::credential::assemble_issuer_dataset(&data, &policy.trust_members, [7; 32])
                .unwrap();
        let trace = interp::evaluate_dataset(&plan, &dataset, &SuiteEncoder).unwrap();
        let terms = release_vocabulary(&data, &[]).unwrap();
        let answer = Answer::release(&trace.result, &plan, &terms, &SuiteEncoder).unwrap();
        assert_eq!(answer.encode(&SuiteEncoder).unwrap(), trace.result);
    }
}
