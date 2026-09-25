//! [GPT-6] Finite binding-job adapter for the experimental native RDF relation.
//!
//! Test-only synthetic issuance; no production credential or trust discovery.
use ark_std::rand::{rngs::StdRng, SeedableRng};
use oxrdf::GraphName;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sparq_native_composition_spike::rdf::{
    binding_tests::prove_without_query_preimages, issue_rdf, prepare_public_bgp, prove_public_bgp,
    public_context, verify_public_bgp, AcceptedStatus, ConsumedNonces, Error, Issuer, Mapping,
    Request, RolePolicy, StatusReference, Support, TrustedIssuer,
};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::Path,
};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;
// The protocol bounds each independent JSON job before deserialization.
const MAX_JOB_BYTES: u64 = 1_048_576;
const ISSUER_ID: &str = "urn:sparq:proof-binding:synthetic-issuer";

fn string<'a>(value: &'a Value, field: &str) -> AppResult<&'a str> {
    value[field]
        .as_str()
        .ok_or_else(|| format!("missing string {field}").into())
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn nonce(bytes: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"sparq/native-rdf/binding-job/nonce/v1\0");
    digest.update(bytes);
    digest.finalize().into()
}

struct Execution {
    outcome: Value,
    artifacts: Vec<(&'static str, Vec<u8>)>,
}

impl Execution {
    fn new(job: &Value, bytes: &[u8]) -> Self {
        Self {
            outcome: json!({
                "schema":"sparq.proof-binding-outcome.v1", "job_id":job["id"],
                "case_sha256":job["case_sha256"], "backend":"native_rdf", "tier":job["tier"],
                "observed":"unsupported", "stage":"admission", "proof_count":0,
                "verified_count":0, "error_class":null, "artifacts":[], "controls":[],
                "implementation":{
                    "protocol":"sparq/native-rdf/public-bgp/v1",
                    "authentication":"fresh synthetic issuer key independently supplied to verifier",
                    "relation":"one nonempty DISTINCT BGP binding; no completeness assertion",
                    "input_sha256":hash(bytes), "build_attestation":null,
                    "nonce":"SHA-256 domain-separated complete input bytes; deterministic test identifier",
                    "disclosure":"public query, row, issuer, status and canonical signed-slot indices"
                }
            }),
            artifacts: Vec::new(),
        }
    }

    fn unsupported(mut self, reason: &str) -> Self {
        self.outcome["unsupported_reason"] = json!(reason);
        self.outcome["error_class"] = json!("ProfileExclusion");
        self
    }

    fn rejection(mut self, stage: &str, error: Error) -> Self {
        self.outcome["observed"] = json!("rejected");
        self.outcome["stage"] = json!(stage);
        self.outcome["error_class"] = json!(format!("{error:?}"));
        self
    }
}

// Require the independently supplied dataset representations to describe the
// same default graph; never silently discard named graphs or extra source triples.
fn graph_set(source: &str) -> AppResult<BTreeSet<String>> {
    let quads = sparq_canon::parse_nquads(source).map_err(|_| Error::RdfProfile)?;
    if quads
        .iter()
        .any(|q| q.graph_name != GraphName::DefaultGraph)
    {
        return Err(Error::RdfProfile.into());
    }
    Ok(quads.into_iter().map(|q| q.to_string()).collect())
}

fn document(job: &Value) -> AppResult<String> {
    let triples = job["triples"].as_array().ok_or("missing triple array")?;
    let mut source = String::new();
    for triple in triples {
        let fields = triple.as_array().ok_or("triple is not an array")?;
        if fields.len() != 3 {
            return Err("triple width differs from three".into());
        }
        let terms = fields
            .iter()
            .map(|t| t.as_str().ok_or("non-string triple term"))
            .collect::<Result<Vec<_>, _>>()?;
        let line = format!("{} {} {} .\n", terms[0], terms[1], terms[2]);
        let quads = sparq_canon::parse_nquads(&line).map_err(|_| Error::RdfProfile)?;
        let Some(quad) = quads.first().filter(|_| quads.len() == 1) else {
            return Err(Error::RdfProfile.into());
        };
        // Compare N-Triples spellings, not NamedNode's raw-IRI string equality.
        let canonical = [
            quad.subject.to_string(),
            quad.predicate.to_string(),
            quad.object.to_string(),
        ];
        if quad.graph_name != GraphName::DefaultGraph
            || canonical
                .iter()
                .map(String::as_str)
                .ne(terms.iter().copied())
        {
            return Err(Error::RdfProfile.into());
        }
        source.push_str(&line);
    }
    if let Some(dataset) = job.get("dataset") {
        let names = dataset["named_graphs"]
            .as_array()
            .ok_or("missing graph catalog")?;
        if !names.is_empty() {
            return Err(Error::RdfProfile.into());
        }
        let expected = graph_set(&source)?;
        for field in ["ntriples", "nquads"] {
            if graph_set(string(dataset, field)?)? != expected {
                return Err("dataset source differs from declared triple array".into());
            }
        }
    }
    Ok(source)
}

fn public_artifact(
    request: &Request,
    support: &Support,
    issuer_bytes: &[u8],
) -> AppResult<Vec<u8>> {
    let context = public_context(request, support)?;
    Ok(serde_json::to_vec_pretty(&json!({
        "context":serde_json::from_slice::<Value>(&context)?, "nonce":request.nonce,
        "issuer_key":issuer_bytes, "support":support.rows.iter().map(|r|r.iter().map(|s|[s.role,s.triple]).collect::<Vec<_>>()).collect::<Vec<_>>()
    }))?)
}

fn execute(bytes: &[u8]) -> AppResult<Execution> {
    let job: Value = serde_json::from_slice(bytes)?;
    if job["schema"] != "sparq.proof-binding-job.v1" || job["backend"] != "native_rdf" {
        return Err("wrong job schema or backend".into());
    }
    for field in ["id", "case_sha256"] {
        let id = string(&job, field)?;
        if id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!("invalid {field}").into());
        }
    }
    if string(&job, "case_id")?.is_empty()
        || job["nonce_id"].as_u64().is_none()
        || job["expected_accept"].as_bool().is_none()
    {
        return Err("missing case identity, nonce or expectation".into());
    }
    // The expectation is metadata for the independent controller, never an
    // input to acceptance, support selection or error classification.
    let mut run = Execution::new(&job, bytes);
    if !matches!(job["tier"].as_str(), Some("native" | "real")) || job["operation"] != "binding" {
        return Ok(
            run.unsupported("native RDF adapter admits binding jobs in native/real tiers only")
        );
    }
    if job.get("authority").is_some()
        || job
            .get("policy_overrides")
            .is_some_and(|v| v.as_object().is_none_or(|m| !m.is_empty()))
    {
        return Ok(run.unsupported(
            "native signed-support contract has no dataset-authority or policy overrides",
        ));
    }
    let query = string(&job, "query")?.to_owned();
    let variables = job["variables"].as_array().ok_or("missing variables")?;
    let rows = job["rows"].as_array().ok_or("missing rows")?;
    if rows.len() != 1 {
        return Err("binding job requires exactly one row".into());
    }
    let cells = rows[0].as_array().ok_or("row is not an array")?;
    if cells.len() != variables.len() {
        return Err("row width differs from variables".into());
    }
    let mut mapping = Mapping::new();
    for (var, cell) in variables.iter().zip(cells) {
        let var = var.as_str().ok_or("non-string variable")?;
        if cell.is_null() {
            return Ok(run.unsupported("every BGP variable must be publicly bound"));
        }
        if mapping
            .insert(
                var.to_owned(),
                cell.as_str().ok_or("non-string cell")?.to_owned(),
            )
            .is_some()
        {
            return Err("duplicate variable".into());
        }
    }
    let source = match document(&job) {
        Ok(source) => source,
        Err(error) if error.downcast_ref::<Error>() == Some(&Error::RdfProfile) => {
            return Ok(run
                .unsupported("default-graph canonical RDF terms required; named graphs excluded"));
        }
        Err(error) => return Err(error),
    };
    let mut rng = StdRng::from_entropy();
    let issuer = Issuer::generate(&mut rng, ISSUER_ID)?;
    let status = StatusReference {
        list: "urn:sparq:proof-binding:synthetic-status".into(),
        epoch: 1,
        index: 0,
    };
    let credential = match issue_rdf(&mut rng, &issuer, &source, &status) {
        Ok(credential) => credential,
        Err(Error::Capacity) => {
            run = run.rejection("admission", Error::Capacity);
            run.outcome["unsupported_reason"] = json!("existing bounded nonempty signed-graph profile; admission-only refusal, no attempted proof");
            return Ok(run);
        }
        Err(Error::RdfProfile) => {
            return Ok(
                run.unsupported("native issuer excludes blank nodes and RDF1.2-only term forms")
            )
        }
        Err(error) => return Err(error.into()),
    };
    let issuer_bytes = issuer.public().to_bytes()?;
    let request = Request {
        query,
        rows: vec![mapping],
        nonce: nonce(bytes),
        roles: vec![RolePolicy {
            issuer: TrustedIssuer::from_bytes(ISSUER_ID, &issuer_bytes)?,
            status: AcceptedStatus {
                reference: status,
                bits: vec![0],
            },
        }],
    };
    let support = match prepare_public_bgp(&request, std::slice::from_ref(&credential)) {
        Ok(support) => support,
        Err(Error::QueryProfile) => {
            return Ok(run.unsupported(
                "SELECT DISTINCT nonempty BGP with every variable projected is required",
            ))
        }
        Err(Error::Support) if job["tier"] == "real" => {
            // This deliberately bypasses honest preparation. The helper builds
            // and independently verifies a genuine weaker BBS+ statement using
            // this same required query/row context and nonce.
            let weaker = prove_without_query_preimages(&mut rng, &request, &credential)?;
            let mut consumed = ConsumedNonces::default();
            if verify_public_bgp(&mut rng, &request, &weaker, &mut consumed)
                != Err(Error::Verification)
            {
                return Err(
                    "weaker proof did not fail the required verifier through Verification".into(),
                );
            }
            run.artifacts.push((
                "public.json",
                public_artifact(&request, &weaker.support, &issuer_bytes)?,
            ));
            run.artifacts.push(("weaker-proof.bin", weaker.proof));
            run = run.rejection("proof_verifier", Error::Verification);
            run.outcome["proof_count"] = json!(1);
            // Only verification of the required RDF claim counts here.
            run.outcome["verified_count"] = json!(0);
            run.outcome["controls"] = json!([
                {"name":"honest-preparation","observed":"rejected","stage":"support","error_class":"Support"},
                {"name":"same-context-weaker-statement","observed":"accepted","stage":"proof_verifier","verified_count":1,"omitted":"all requested RDF preimages"},
                {"name":"required-statement","observed":"rejected","stage":"proof_verifier","error_class":"Verification"}
            ]);
            return Ok(run);
        }
        Err(error @ (Error::Support | Error::RdfProfile | Error::Capacity)) => {
            return Ok(run.rejection("support", error))
        }
        Err(error) => return Err(error.into()),
    };
    run.artifacts.push((
        "public.json",
        public_artifact(&request, &support, &issuer_bytes)?,
    ));
    if job["tier"] == "native" {
        run.outcome["observed"] = json!("accepted");
        run.outcome["stage"] = json!("native");
        return Ok(run);
    }
    let presentation = prove_public_bgp(&mut rng, &request, &[credential])?;
    if presentation.support != support {
        return Err("native/proving support preparation diverged".into());
    }
    let mut consumed = ConsumedNonces::default();
    verify_public_bgp(&mut rng, &request, &presentation, &mut consumed)?;
    if verify_public_bgp(&mut rng, &request, &presentation, &mut consumed) != Err(Error::Replay) {
        return Err("real replay control did not reject through Replay".into());
    }
    run.artifacts.push(("proof.bin", presentation.proof));
    run.outcome["observed"] = json!("accepted");
    run.outcome["stage"] = json!("proof_verifier");
    run.outcome["proof_count"] = json!(1);
    run.outcome["verified_count"] = json!(1);
    run.outcome["controls"] = json!([{"name":"same-verifier-replay","observed":"rejected","error_class":"Replay","stage":"proof_verifier"}]);
    Ok(run)
}

fn write_new(path: &Path, bytes: &[u8]) -> AppResult<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

fn main() -> AppResult<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: native-bindings INPUT_JSON NEW_OUTPUT_DIRECTORY".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&args[0])?
        .take(MAX_JOB_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JOB_BYTES {
        return Err("job byte capacity exceeded".into());
    }
    let mut run = execute(&bytes)?;
    let output = Path::new(&args[1]);
    fs::create_dir(output)?;
    let mut inventory = Vec::new();
    for (name, bytes) in run.artifacts {
        write_new(&output.join(name), &bytes)?;
        inventory.push(json!({"path":name,"sha256":hash(&bytes)}));
    }
    run.outcome["artifacts"] = json!(inventory);
    write_new(
        &output.join("outcome.json"),
        &serde_json::to_vec_pretty(&run.outcome)?,
    )
}

#[cfg(test)]
#[path = "native-bindings/tests.rs"]
mod tests;
