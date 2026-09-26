// [OPUS-5.5] zkp-14.5: retained V5 receipt evidence, bound to the approved V5 pin.
//! Evidence for genuine V5 receipts, kept separate from `support/evidence.rs`.
//!
//! Every record names the exact typed request, the independently approved V5
//! [`ArtifactPin`] from the job (never an embedded default), SHA-256 digests and
//! the complete receipt. Before writing a record, this module re-verifies the
//! receipt against that pin, requiring Succinct and dev mode disabled. Every
//! file is created with `create_new`, owner-only on Unix, and never replaced or
//! removed.

use risc0_zkvm::{InnerReceipt, VerifierContext};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sparq_proved_evaluator::{ArtifactPin, Presentation};
use sparq_proved_evaluator_model::authenticated_rdf::Request;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const RECORD_SCHEMA: &str = "sparq.authrdf-receipt-evidence.v1";
/// Package of the guest every record is pinned to, so records name their image.
pub const GUEST_PACKAGE: &str = "sparq-authrdf-guest";

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn pretty(value: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec_pretty(value).expect("typed public JSON")
}

/// Reads one bounded explicit input: absolute, no `.`/`..`, a regular file, not a symlink.
pub fn read_input(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let shown = path.display();
    let dotted = path
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::CurDir));
    if !path.is_absolute() || dotted {
        return Err(format!("{shown}: inputs must be absolute paths without . or .."));
    }
    let kind = fs::symlink_metadata(path)
        .map_err(|error| format!("{shown}: {error}"))?
        .file_type();
    if !kind.is_file() {
        return Err(format!("{shown}: inputs must be regular files, not symlinks or directories"));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("{shown}: {error}"))?;
    if bytes.len() > limit {
        return Err(format!("{shown} exceeds its read bound of {limit} bytes"));
    }
    Ok(bytes)
}

/// Creates one new directory, owner-only on Unix; an existing path is an error.
fn create_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Writes one new file, owner-only on Unix; never replaces existing evidence.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

/// A new evidence directory outside the checkout, tied to one approved V5 pin.
pub struct Evidence {
    root: PathBuf,
    pin: ArtifactPin,
}

impl Evidence {
    /// Creates the absent directory below a canonicalized parent outside the checkout.
    pub fn create(requested: &Path, pin: &ArtifactPin) -> Result<Self, String> {
        if !requested.is_absolute() {
            return Err("new_output_directory must be absolute".into());
        }
        let (Some(parent), Some(name)) = (requested.parent(), requested.file_name()) else {
            return Err("new_output_directory needs a parent and a final component".into());
        };
        let parent = parent
            .canonicalize()
            .map_err(|error| format!("output parent: {error}"))?;
        let root = parent.join(name);
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .map_err(|error| format!("source checkout: {error}"))?;
        if root.starts_with(&checkout) {
            return Err("new_output_directory must be outside the source checkout".into());
        }
        create_dir(&root).map_err(|error| format!("{}: {error}", root.display()))?;
        Ok(Self {
            root,
            pin: pin.clone(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Writes one new top-level file, returning its size and SHA-256.
    pub fn write(&self, name: &str, bytes: &[u8]) -> Value {
        write_new(&self.root.join(name), bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        json!({ "bytes": bytes.len(), "sha256": sha256(bytes) })
    }

    /// Creates a case directory and writes its exact request before proving starts.
    pub fn start(&self, case: &str, request: &Request) {
        let dir = self.root.join(case);
        create_dir(&dir).unwrap_or_else(|error| panic!("{case}: {error}"));
        write_new(&dir.join("request.json"), &pretty(request)).expect("request evidence");
        let marker = json!({ "case": case, "progress": "proving", "claims": "none" });
        write_new(&dir.join("started.json"), &pretty(&marker)).expect("progress marker");
    }

    /// Retains the raw presentation right after proving, before any verification.
    pub fn retain(&self, case: &str, presentation: &Presentation) {
        let bytes = serde_json::to_vec(presentation).expect("presentation JSON");
        write_new(&self.root.join(case).join("presentation.json"), &bytes)
            .expect("presentation evidence");
    }

    /// Re-verifies against the approved V5 pin, then writes the complete case record.
    ///
    /// Returns the record without the receipt body, for the run summary.
    pub fn record(
        &self,
        case: &str,
        request: &Request,
        presentation: &Presentation,
        details: Value,
    ) -> Value {
        let receipt = &presentation.receipt;
        assert!(
            matches!(receipt.inner, InnerReceipt::Succinct(_)),
            "{case}: a Succinct receipt is required"
        );
        receipt
            .verify_with_context(
                &VerifierContext::default().with_dev_mode(false),
                self.pin.image_id,
            )
            .unwrap_or_else(|error| panic!("{case}: receipt does not verify with the V5 pin: {error}"));
        let receipt_bytes = serde_json::to_vec(receipt).expect("receipt JSON");
        let request_bytes = serde_json::to_vec(request).expect("request JSON");
        let summary = json!({
            "schema": RECORD_SCHEMA,
            "case": case,
            "fixture": "published W3C vc-di-eddsa eddsa-rdfc-2022 vector; public data only",
            "guest": GUEST_PACKAGE,
            "pin": self.pin,
            "artifact_sha256": hex(&self.pin.sha256),
            "request_sha256": sha256(&request_bytes),
            "receipt_sha256": sha256(&receipt_bytes),
            "journal_sha256": sha256(&receipt.journal.bytes),
            "details": details,
        });
        let mut record = summary.clone();
        record["request"] = json!(request);
        record["receipt"] = json!(receipt);
        write_new(&self.root.join(case).join("record.json"), &pretty(&record))
            .expect("case record");
        summary
    }
}
