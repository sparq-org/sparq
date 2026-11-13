//! JSON process interface to the existing authenticated-RDF relation.
//! Research grade; the issue command is a test issuer only.
mod nonces {
    include!("authrdf_cli/nonces.rs");
}

use ed25519_dalek::{Signer, SigningKey};
use model::{
    AuthorizedKey, Policy, PrivateCredentials, Request, SignatureMode, SignedCredential, Witness,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sparq_proved_evaluator::authenticated_rdf as host;
use sparq_proved_evaluator::{AcceptedGuest, ArtifactPin, Error, Presentation};
use sparq_proved_evaluator_model::{DatasetAuthority, authenticated_rdf as model};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

type Result<T> = std::result::Result<T, Error>;

fn hex<const N: usize>(text: &str) -> Result<[u8; N]> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error("hex must be lowercase and have the exact length"));
    }
    let mut bytes = [0; N];
    for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let digit = |b: u8| {
            if b.is_ascii_digit() {
                b - b'0'
            } else {
                b - b'a' + 10
            }
        };
        *byte = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(bytes)
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Args(BTreeMap<String, Vec<String>>);
impl Args {
    fn parse(command: &str, values: impl Iterator<Item = String>) -> Result<Self> {
        let allowed: &[&str] = match command {
            "issue" => &[
                "--seed-hex",
                "--issuer",
                "--verification-method",
                "--document",
                "--created",
                "--out",
            ],
            "request" => &[
                "--query",
                "--keys",
                "--authority",
                "--nonce-hex",
                "--signature-mode",
                "--out",
            ],
            "prove" => &[
                "--request",
                "--credentials",
                "--salt-hex",
                "--guest",
                "--r0vm",
                "--out",
            ],
            "verify" => &[
                "--presentation",
                "--request",
                "--guest",
                "--nonce-store",
                "--signatures",
            ],
            _ => return Err(Error("expected issue, request, prove or verify")),
        };
        let mut values = values.peekable();
        let mut options = BTreeMap::new();
        while let Some(flag) = values.next() {
            if !allowed.contains(&flag.as_str()) || options.contains_key(&flag) {
                return Err(Error("unknown or duplicate option"));
            }
            let mut arguments = Vec::new();
            while values.peek().is_some_and(|v| !v.starts_with("--")) {
                arguments.push(values.next().expect("peeked argument"));
                if flag != "--credentials" {
                    break;
                }
            }
            if arguments.is_empty() {
                return Err(Error("missing option value"));
            }
            options.insert(flag, arguments);
        }
        Ok(Self(options))
    }
    fn one(&self, flag: &str) -> Result<&str> {
        self.optional(flag).ok_or(Error("missing required option"))
    }
    fn optional(&self, flag: &str) -> Option<&str> {
        self.0.get(flag).map(|values| values[0].as_str())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Key {
    issuer: String,
    verification_method: String,
    public_key_hex: String,
}
impl Key {
    fn authorized(&self) -> Result<AuthorizedKey> {
        Ok(AuthorizedKey {
            issuer: self.issuer.clone(),
            verification_method: self.verification_method.clone(),
            public_key: hex(&self.public_key_hex)?,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credential {
    document: String,
    proof_config: String,
    signature_hex: String,
    public_key_hex: String,
    issuer: String,
    verification_method: String,
}
impl Credential {
    fn key(&self) -> Result<AuthorizedKey> {
        Key {
            issuer: self.issuer.clone(),
            verification_method: self.verification_method.clone(),
            public_key_hex: self.public_key_hex.clone(),
        }
        .authorized()
    }
    fn signed(&self) -> Result<SignedCredential> {
        Ok(SignedCredential {
            document: self.document.clone(),
            proof_config: self.proof_config.clone(),
            signature: hex::<64>(&self.signature_hex)?.to_vec(),
        })
    }
}

fn read_text(path: impl AsRef<Path>) -> Result<String> {
    fs::read_to_string(path).map_err(|_| Error("input file read failed"))
}
fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    serde_json::from_str(&read_text(path)?).map_err(|_| Error("input JSON rejected"))
}
fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(Error("output already exists or cannot be inspected")),
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<usize> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error("output JSON encoding failed"))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| Error("output creation refused"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| Error("output write failed"))?;
    Ok(bytes.len())
}
fn validate(request: &Request) -> Result<()> {
    model::validate_request(request).map_err(|e| Error(e.0))?;
    host::check_query_profile(request)?;
    if request.policy.cryptosuite != model::Cryptosuite::EddsaRdfc2022 {
        return Err(Error("CLI supports eddsa-rdfc-2022 only"));
    }
    Ok(())
}
fn guest(directory: &str) -> Result<AcceptedGuest> {
    let directory = Path::new(directory);
    let pin: ArtifactPin = read_json(directory.join("pin.json"))?;
    let artifact = fs::read(directory.join("guest.bin")).map_err(|_| Error("guest read failed"))?;
    AcceptedGuest::from_artifact(artifact, &pin)
}

fn issue(args: &Args) -> Result<Value> {
    let seed = hex::<32>(args.one("--seed-hex")?)?;
    let issuer = args.one("--issuer")?.to_owned();
    let verification_method = args.one("--verification-method")?.to_owned();
    let document_path = args.one("--document")?;
    let out = Path::new(args.one("--out")?);
    absent(out)?;
    let key = SigningKey::from_bytes(&seed);
    let authorized = AuthorizedKey {
        issuer: issuer.clone(),
        verification_method: verification_method.clone(),
        public_key: key.verifying_key().to_bytes(),
    };
    let policy = Policy::new(vec![authorized]);
    // Revealed evaluation supplies the model's own canonical signing message.
    // This local challenge and salt are used only to validate test issuance.
    let request = Request {
        version: model::VERSION,
        query: "ASK {}".into(),
        authority: DatasetAuthority::HolderDeclared,
        policy: policy.clone().with_signature_mode(SignatureMode::Revealed),
        nonce: [1; 32],
    };
    validate(&request)?;
    let mut proof_config = format!(
        concat!(
            "_:proof <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://w3id.org/security#DataIntegrityProof> .\n",
            "_:proof <https://w3id.org/security#cryptosuite> \"eddsa-rdfc-2022\"^^<https://w3id.org/security#cryptosuiteString> .\n",
            "_:proof <https://w3id.org/security#proofPurpose> <https://w3id.org/security#assertionMethod> .\n",
            "_:proof <https://w3id.org/security#verificationMethod> <{}> .\n"
        ),
        verification_method
    );
    if let Some(created) = args.optional("--created") {
        let lexical =
            serde_json::to_string(created).map_err(|_| Error("created encoding rejected"))?;
        proof_config.push_str(&format!("_:proof <http://purl.org/dc/terms/created> {lexical}^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n"));
    }
    let document = read_text(document_path)?;
    let mut dataset = PrivateCredentials {
        credentials: vec![SignedCredential {
            document: document.clone(),
            proof_config: proof_config.clone(),
            signature: Vec::new(),
        }],
        salt: [1; 32],
    };
    let journal = model::evaluate(&Witness {
        request,
        dataset: dataset.clone(),
    })
    .map_err(|e| Error(e.0))?;
    let signature = key.sign(&journal.signed_messages[0].message).to_bytes();
    dataset.credentials[0].signature = signature.to_vec();
    model::dataset_commitment(&dataset, &policy).map_err(|e| Error(e.0))?;
    write_json(
        out,
        &Credential {
            document,
            proof_config,
            signature_hex: hex_string(&signature),
            public_key_hex: hex_string(&key.verifying_key().to_bytes()),
            issuer,
            verification_method,
        },
    )?;
    Ok(json!({}))
}

fn request(args: &Args) -> Result<Value> {
    let nonce = hex::<32>(args.one("--nonce-hex")?)?;
    let authority = match args.one("--authority")? {
        "holder-declared" => DatasetAuthority::HolderDeclared,
        value => {
            let anchor = value
                .strip_prefix("verifier-agreed:")
                .ok_or(Error("verifier-agreed requires an anchor"))?;
            DatasetAuthority::VerifierAgreed {
                commitment: hex(anchor)?,
            }
        }
    };
    let mode = match args.optional("--signature-mode").unwrap_or("hidden") {
        "hidden" => SignatureMode::Hidden,
        "revealed" => SignatureMode::Revealed,
        _ => return Err(Error("signature mode must be hidden or revealed")),
    };
    let query_path = args.one("--query")?;
    let keys_path = args.one("--keys")?;
    let out = Path::new(args.one("--out")?);
    absent(out)?;
    let keys: Vec<Key> = read_json(keys_path)?;
    let authorization = keys
        .iter()
        .map(Key::authorized)
        .collect::<Result<Vec<_>>>()?;
    let request = Request {
        version: model::VERSION,
        query: read_text(query_path)?,
        authority,
        policy: Policy::new(authorization).with_signature_mode(mode),
        nonce,
    };
    validate(&request)?;
    let digest = model::request_digest(&request).map_err(|e| Error(e.0))?;
    write_json(out, &request)?;
    Ok(json!({"request_digest": hex_string(&digest)}))
}

fn prove(args: &Args) -> Result<Value> {
    let salt = hex::<32>(args.one("--salt-hex")?)?;
    let request_path = args.one("--request")?;
    let paths = args
        .0
        .get("--credentials")
        .ok_or(Error("missing credentials"))?;
    let guest_path = args.one("--guest")?;
    let r0vm = Path::new(args.one("--r0vm")?);
    let out = Path::new(args.one("--out")?);
    absent(out)?;
    let request: Request = read_json(request_path)?;
    validate(&request)?;
    let credentials = paths
        .iter()
        .map(read_json)
        .collect::<Result<Vec<Credential>>>()?;
    // Decode every hex field before authenticating any document.
    let keys = credentials
        .iter()
        .map(Credential::key)
        .collect::<Result<Vec<_>>>()?;
    let signed = credentials
        .iter()
        .map(Credential::signed)
        .collect::<Result<Vec<_>>>()?;
    if keys
        .iter()
        .any(|key| !request.policy.authorization.contains(key))
    {
        return Err(Error("credential key is not authorized by request"));
    }
    let mut dataset = PrivateCredentials {
        credentials: signed,
        salt,
    };
    let hidden_policy = request
        .policy
        .clone()
        .with_signature_mode(SignatureMode::Hidden);
    model::dataset_commitment(&dataset, &hidden_policy).map_err(|e| Error(e.0))?;
    if request.policy.signature_mode == SignatureMode::Revealed {
        for credential in &mut dataset.credentials {
            credential.signature.clear();
        }
    }
    let witness = Witness { request, dataset };
    // Native evaluation also checks anchors and gives no proof or acceptance shortcut.
    model::evaluate(&witness).map_err(|e| Error(e.0))?;
    let guest = guest(guest_path)?;
    let start = Instant::now();
    let presentation = host::prove_with_artifact(&witness, r0vm, &guest)?;
    let prove_ms = start.elapsed().as_millis();
    let journal: model::Journal = presentation
        .receipt
        .journal
        .decode()
        .map_err(|_| Error("journal decoding rejected"))?;
    let presentation_bytes = write_json(out, &presentation)?;
    Ok(
        json!({"result": journal.result, "presentation_bytes": presentation_bytes, "prove_ms": prove_ms}),
    )
}

fn verify(args: &Args) -> Result<Value> {
    let presentation_path = args.one("--presentation")?;
    let request_path = args.one("--request")?;
    let guest_path = args.one("--guest")?;
    let nonce_path = PathBuf::from(args.one("--nonce-store")?);
    let request: Request = read_json(request_path)?;
    validate(&request)?;
    let signatures = match (request.policy.signature_mode, args.optional("--signatures")) {
        (SignatureMode::Revealed, Some(path)) => {
            let strings: Vec<String> = read_json(path)?;
            strings
                .iter()
                .map(|s| hex::<64>(s).map(|b| b.to_vec()))
                .collect::<Result<Vec<_>>>()?
        }
        (SignatureMode::Hidden, None) => Vec::new(),
        _ => return Err(Error("signatures are required only in revealed mode")),
    };
    let presentation: Presentation = read_json(presentation_path)?;
    let guest = guest(guest_path)?;
    let mut nonces = nonces::FileNonces(nonce_path);
    let start = Instant::now();
    let journal = if request.policy.signature_mode == SignatureMode::Revealed {
        host::verify_revealed_with_artifact(
            &presentation,
            &request,
            &signatures,
            &mut nonces,
            &guest,
        )?
    } else {
        host::verify_with_artifact(&presentation, &request, &mut nonces, &guest)?
    };
    Ok(
        json!({"verified": true, "result": journal.result, "request_digest": hex_string(&journal.request_digest), "dataset_commitment": hex_string(&journal.dataset_commitment), "provenance": journal.provenance, "verify_ms": start.elapsed().as_millis()}),
    )
}

fn main() -> std::process::ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments
        .next()
        .and_then(|arg| arg.into_string().ok())
        .unwrap_or_default();
    let result = arguments
        .map(|arg| {
            arg.into_string()
                .map_err(|_| Error("non-Unicode argument rejected"))
        })
        .collect::<Result<Vec<_>>>()
        .and_then(|values| Args::parse(&command, values.into_iter()))
        .and_then(|args| match command.as_str() {
            "issue" => issue(&args),
            "request" => request(&args),
            "prove" => prove(&args),
            "verify" => verify(&args),
            _ => unreachable!(),
        });
    match result {
        Ok(value) => {
            println!("{value}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            if command == "verify" {
                println!("{}", json!({"verified": false, "reason": error.0}));
            }
            std::process::ExitCode::FAILURE
        }
    }
}
