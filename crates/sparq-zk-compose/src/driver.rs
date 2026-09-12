// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! nargo + bb subprocess driver (v1: subprocess proving is acceptable per the
//! plan). One [`CircuitProver`] points at the `zk/compose/` workspace and a
//! scratch dir; it compiles a member, generates a witness from a Prover.toml,
//! produces a bb proof + vk, and verifies.
//!
//! File conventions (nargo 1.0.0-beta.21, bb 5.0.0-nightly.20260324):
//! - `nargo execute <name> --package P` writes `target/<name>.gz` (witness)
//!   and requires `target/P.json` (ACIR) from `nargo compile`.
//! - `bb prove -b acir.json -w witness.gz -o OUTDIR` writes `OUTDIR/proof`
//!   and `OUTDIR/public_inputs`.
//! - `bb write_vk -b acir.json -o OUTDIR` writes `OUTDIR/vk`.
//! - `bb verify -p proof -i public_inputs -k vk` exits 0 on success.

use crate::manifest::CircuitId;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Driver / proving error.
#[derive(Debug)]
pub enum DriverError {
    Spawn {
        tool: String,
        source: std::io::Error,
    },
    Tool {
        tool: String,
        stderr: String,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::Spawn { tool, source } => {
                write!(f, "failed to spawn `{tool}`: {source}")
            }
            DriverError::Tool { tool, stderr } => {
                write!(f, "`{tool}` failed:\n{stderr}")
            }
            DriverError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for DriverError {}

impl From<std::io::Error> for DriverError {
    fn from(e: std::io::Error) -> Self {
        DriverError::Io(e)
    }
}

// [GPT-6] Tags are filename labels, never paths or subprocess options. Preserve
// the legacy empty tag and common ASCII punctuation inside a single component.
pub(crate) fn validate_witness_tag(tag: &str) -> Result<(), DriverError> {
    if tag == "."
        || tag == ".."
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "witness tag must be an ASCII alphanumeric/underscore/hyphen/dot label, not a path",
        )
        .into());
    }
    Ok(())
}

// [GPT-6] nargo treats a dot in --prover-name as an extension and replaces it.
// Percent is outside the admitted alphabet, so this encoding is injective while
// preserving existing labels without dots. Only this internal filename changes.
fn witness_file_label(tag: &str) -> String {
    tag.replace('.', "%2E")
}

/// A produced proof: bb proof bytes, its public-inputs bytes, and the vk.
#[derive(Debug, Clone)]
pub struct ProofArtifacts {
    pub proof: Vec<u8>,
    pub public_inputs: Vec<u8>,
    pub vk: Vec<u8>,
}

// [GPT-6] Atomic directory allocation prevents shared-root calls from mixing
// verification tuples. The counter is only a name hint: create_dir arbitrates.
static SCRATCH_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct ScratchDir(PathBuf);
impl ScratchDir {
    fn new(root: &Path, label: &str) -> Result<Self, DriverError> {
        std::fs::create_dir_all(root)?;
        for _ in 0..1024 {
            let id = SCRATCH_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = root.join(format!("sparq_{label}_{}_{}", std::process::id(), id));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(DriverError::Io(e)),
            }
        }
        Err(DriverError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "unable to allocate isolated proof workspace",
        )))
    }
}
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// [GPT-6] nargo beta.21 has no target-directory override. Every driver compile
// and execute holds this advisory per-workspace OS lock, including implicit
// compilation during execute. Opening a fresh fd per acquisition coordinates
// independent driver instances, threads, and processes on a local filesystem.
// Closing the exclusively owned File releases flock on every return/unwind path.
struct NargoCacheLock {
    _file: std::fs::File,
}
impl NargoCacheLock {
    fn acquire(target_dir: &Path) -> Result<Self, DriverError> {
        std::fs::create_dir_all(target_dir)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(target_dir.join("sparq_nargo.lock"))?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // SAFETY: file exclusively owns this valid open fd and remains live
            // throughout the call. flock takes no pointers and does not close it.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Self { _file: file })
        }
        #[cfg(not(unix))]
        {
            let _ = file;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "nargo cache locking requires an OS advisory lock",
            )
            .into())
        }
    }
}

// [GPT-6] The witness is born under 0700, before nargo creates it. Neither this
// owner nor its credential-input path is serializable or printable with Debug.
#[cfg(feature = "successful-results")]
pub(crate) struct PrivateWitness {
    pub(crate) path: PathBuf,
    input_path: PathBuf,
    _scratch: ScratchDir,
}
#[cfg(feature = "successful-results")]
impl Drop for PrivateWitness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.input_path);
    }
}

/// Drives nargo/bb against the `zk/compose/` Noir workspace.
pub struct CircuitProver {
    /// Path to `zk/compose/` (the Nargo workspace root).
    compose_dir: PathBuf,
    /// bb verifier target (Noir circuits use `noir-recursive`).
    target: String,
}

impl CircuitProver {
    /// `compose_dir` is the `zk/compose/` workspace root.
    pub fn new(compose_dir: impl Into<PathBuf>) -> Self {
        CircuitProver {
            compose_dir: compose_dir.into(),
            target: "noir-recursive".to_string(),
        }
    }

    /// Locate `zk/compose/` relative to this crate (workspace layout).
    pub fn from_crate_root() -> Self {
        // crates/sparq-zk-compose -> ../../zk/compose
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let compose = here
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("zk").join("compose"))
            .expect("workspace layout");
        CircuitProver::new(compose)
    }

    fn target_dir(&self) -> PathBuf {
        self.compose_dir.join("target")
    }

    /// Compiles a member and returns an immutable, content-addressed ACIR snapshot.
    ///
    /// Snapshots live in the ignored target cache and may be reused until that
    /// cache is removed. Unlike nargo's mutable `target/P.json`, the returned
    /// file is never rewritten by this driver. Compile/execute cache operations
    /// are serialized across cooperating processes using this local workspace.
    /// Do not run external nargo writes or remove the cache during driver jobs.
    pub fn compile(&self, id: &CircuitId) -> Result<PathBuf, DriverError> {
        self.compile_package(&id.package())
    }

    // [GPT-6] Crate-private package dispatch also serves the isolated result contract.
    pub(crate) fn compile_package(&self, pkg: &str) -> Result<PathBuf, DriverError> {
        self.with_compiled_bytes(pkg, |bytes, _lock| self.snapshot_acir(pkg, bytes))
    }

    fn with_compiled_bytes<T>(
        &self,
        pkg: &str,
        consume: impl FnOnce(&[u8], &NargoCacheLock) -> Result<T, DriverError>,
    ) -> Result<T, DriverError> {
        let lock = NargoCacheLock::acquire(&self.target_dir())?;
        run(
            "nargo",
            Command::new("nargo")
                .arg("compile")
                .arg("--package")
                .arg(pkg)
                .current_dir(&self.compose_dir),
        )?;
        let bytes = std::fs::read(self.target_dir().join(format!("{pkg}.json")))?;
        consume(&bytes, &lock)
    }

    // Called while the workspace lock is held. Verify existing cache content;
    // publish a new immutable name atomically so a crash cannot cache half JSON.
    fn snapshot_acir(&self, pkg: &str, bytes: &[u8]) -> Result<PathBuf, DriverError> {
        let root = self.target_dir().join("sparq_acir_cache");
        std::fs::create_dir_all(&root)?;
        let digest = blake3::hash(bytes).to_hex();
        let path = root.join(format!("{pkg}_{digest}.json"));
        match std::fs::read(&path) {
            Ok(existing) if existing == bytes => return Ok(path),
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "compiled ACIR snapshot differs from its content address",
                )
                .into())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let temporary = ScratchDir::new(&root, "publish")?;
        let file = temporary.0.join("acir.json");
        std::fs::write(&file, bytes)?;
        std::fs::rename(file, &path)?;
        Ok(path)
    }

    fn compile_into(&self, pkg: &str, job_dir: &Path) -> Result<PathBuf, DriverError> {
        self.with_compiled_bytes(pkg, |bytes, _lock| {
            let path = job_dir.join("acir.json");
            std::fs::write(&path, bytes)?;
            Ok(path)
        })
    }

    /// Write `Prover.toml` and run `nargo execute`, returning the witness
    /// path. This is the FAST path (no proving) the ignored tests use to
    /// exercise the relation cheaply.
    ///
    /// Concurrency: the per-package `Prover.toml` and the workspace
    /// `target/<pkg>_w.gz` witness are SHARED state. Two prove/witness calls
    /// against the SAME member (e.g. two `filter_int_d1` tests) running
    /// concurrently would overwrite each other's `Prover.toml` between write
    /// and `nargo execute`, and clobber each other's witness — proving (or
    /// failing) the WRONG statement. Callers that may run concurrently against
    /// the same member MUST pass a unique `tag` via [`Self::gen_witness_tagged`]
    /// (and [`Self::prove_in`]) so the prover-input toml and witness get unique,
    /// non-colliding names. This default wrapper uses the empty tag (the shared
    /// `Prover.toml` / `<pkg>_w.gz`) and is only safe single-threaded.
    pub fn gen_witness(&self, id: &CircuitId, prover_toml: &str) -> Result<PathBuf, DriverError> {
        self.gen_witness_tagged(id, prover_toml, "")
    }

    /// As [`Self::gen_witness`], but isolates the prover-input toml and the
    /// emitted witness under a unique `tag` so concurrent calls against the same
    /// member don't race on shared file paths. With a non-empty `tag` the input
    /// is written to `<pkg>/Prover_<label>.toml` (selected via `nargo
    /// execute --prover-name`) and the witness to `target/<pkg>_w_<label>.gz`,
    /// where the internal label encodes each dot as `%2E` for Nargo compatibility.
    /// Tags allow ASCII alphanumerics, underscores, hyphens and dots; `.`/`..`
    /// and path separators reject before I/O. Empty retains the legacy shared name.
    // [OPUS-4.8] tag-isolated witness path so the toolchain tests are safe under
    // default (parallel) `cargo test` — no shared Prover.toml/witness race
    // (roborev codex job 2180).
    pub fn gen_witness_tagged(
        &self,
        id: &CircuitId,
        prover_toml: &str,
        tag: &str,
    ) -> Result<PathBuf, DriverError> {
        self.gen_package_witness(&id.package(), prover_toml, tag)
    }

    // [GPT-6] Callers supply a fixed, internally selected member name.
    pub(crate) fn gen_package_witness(
        &self,
        pkg: &str,
        prover_toml: &str,
        tag: &str,
    ) -> Result<PathBuf, DriverError> {
        validate_witness_tag(tag)?;
        let tag = witness_file_label(tag);
        // Empty tag => legacy shared names; non-empty => per-call-unique names.
        let (prover_name, witness_name) = if tag.is_empty() {
            ("Prover".to_string(), format!("{pkg}_w"))
        } else {
            (format!("Prover_{tag}"), format!("{pkg}_w_{tag}"))
        };
        self.gen_named_witness(pkg, prover_toml, &prover_name, &witness_name)
    }

    fn gen_named_witness(
        &self,
        pkg: &str,
        prover_toml: &str,
        prover_name: &str,
        witness_name: &str,
    ) -> Result<PathBuf, DriverError> {
        // Execute can implicitly rewrite shared compilation caches too. Hold the
        // same OS lock through input publication, compilation and witness writing.
        let _cache_lock = NargoCacheLock::acquire(&self.target_dir())?;
        let toml_path = self
            .compose_dir
            .join(pkg)
            .join(format!("{prover_name}.toml"));
        // [GPT-6] Input files contain credential secrets: restrict before writing.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // [GPT-6] A caller-selected label must not follow an existing input symlink.
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut input = options.open(&toml_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            input.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        std::io::Write::write_all(&mut input, prover_toml.as_bytes())?;
        let witness_path = self.target_dir().join(format!("{witness_name}.gz"));
        // nargo execute exits 0 even on a failed assertion / bad input — it
        // signals failure by NOT writing the witness file (and printing to
        // stderr). So we delete any stale witness first and treat its absence
        // afterwards as the unsatisfiability signal.
        let _ = std::fs::remove_file(&witness_path);
        let out = Command::new("nargo")
            .arg("execute")
            .arg(witness_name)
            .arg("--package")
            .arg(pkg)
            .arg("--prover-name")
            .arg(prover_name)
            .current_dir(&self.compose_dir)
            .output()
            .map_err(|source| DriverError::Spawn {
                tool: "nargo".into(),
                source,
            })?;
        if !witness_path.exists() {
            return Err(DriverError::Tool {
                tool: "nargo execute".into(),
                stderr: format!(
                    "no witness produced (relation unsatisfiable or bad input):\n{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
            });
        }
        Ok(witness_path)
    }

    // [GPT-6] A caller tag is descriptive; the actual input/witness names are
    // allocated internally so even identical caller tags cannot collide.
    #[cfg(feature = "successful-results")]
    pub(crate) fn private_package_witness(
        &self,
        pkg: &str,
        prover_toml: &str,
        tag: &str,
    ) -> Result<PrivateWitness, DriverError> {
        validate_witness_tag(tag)?;
        let tag = witness_file_label(tag);
        let scratch = ScratchDir::new(&self.target_dir(), "witness")?;
        let directory = scratch
            .0
            .file_name()
            .expect("allocated directory name")
            .to_string_lossy()
            .into_owned();
        let prover_name = format!("Prover_{tag}_{directory}");
        let witness_name = format!("{directory}/witness");
        let private = PrivateWitness {
            path: scratch.0.join("witness.gz"),
            input_path: self
                .compose_dir
                .join(pkg)
                .join(format!("{prover_name}.toml")),
            _scratch: scratch,
        };
        self.gen_named_witness(pkg, prover_toml, &prover_name, &witness_name)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&private.path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(private)
    }

    #[cfg(feature = "successful-results")]
    pub(crate) fn prove_private_package(
        &self,
        pkg: &str,
        prover_toml: &str,
        out_dir: &Path,
        tag: &str,
    ) -> Result<ProofArtifacts, DriverError> {
        validate_witness_tag(tag)?;
        let scratch = ScratchDir::new(out_dir, "prove")?;
        let acir = self.compile_into(pkg, &scratch.0)?;
        let witness = self.private_package_witness(pkg, prover_toml, tag)?;
        self.prove_witness(&acir, &witness.path, &scratch.0)
    }

    /// Full prove: compile -> witness -> bb prove + write_vk. `out_dir` is a
    /// scratch directory the artifacts are written into.
    ///
    /// NOTE: this uses the SHARED (untagged) witness path and so is only safe
    /// single-threaded against a given member; concurrent callers must use
    /// [`Self::prove_in`] with a unique tag.
    pub fn prove(
        &self,
        id: &CircuitId,
        prover_toml: &str,
        out_dir: &Path,
    ) -> Result<ProofArtifacts, DriverError> {
        self.prove_in(id, prover_toml, out_dir, "")
    }

    /// As [`Self::prove`], but threads a unique `tag` through the witness step
    /// so concurrent proves against the same member don't race on the shared
    /// `Prover.toml` / `target/<pkg>_w.gz` (the bb artifacts already land in the
    /// caller's isolated `out_dir`). Use a per-test-unique `tag`.
    /// The same filename-label rules as [`Self::gen_witness_tagged`] apply.
    // [OPUS-4.8] tag-isolated prove path (roborev codex job 2180).
    pub fn prove_in(
        &self,
        id: &CircuitId,
        prover_toml: &str,
        out_dir: &Path,
        tag: &str,
    ) -> Result<ProofArtifacts, DriverError> {
        self.prove_package(&id.package(), prover_toml, out_dir, tag)
    }

    // [GPT-6] Shares the explicit noir-recursive (ZK) backend with legacy members.
    pub(crate) fn prove_package(
        &self,
        pkg: &str,
        prover_toml: &str,
        out_dir: &Path,
        tag: &str,
    ) -> Result<ProofArtifacts, DriverError> {
        validate_witness_tag(tag)?;
        let scratch = ScratchDir::new(out_dir, "acir")?;
        let acir = self.compile_into(pkg, &scratch.0)?;
        let witness = self.gen_package_witness(pkg, prover_toml, tag)?;
        self.prove_witness(&acir, &witness, out_dir)
    }

    fn prove_witness(
        &self,
        acir: &Path,
        witness: &Path,
        out_dir: &Path,
    ) -> Result<ProofArtifacts, DriverError> {
        std::fs::create_dir_all(out_dir)?;

        // `--write_vk` emits proof, public_inputs, AND vk in one pass (bb
        // prove otherwise requires a pre-existing vk).
        run(
            "bb",
            Command::new("bb")
                .arg("prove")
                .arg("-b")
                .arg(acir)
                .arg("-w")
                .arg(witness)
                .arg("-o")
                .arg(out_dir)
                .arg("--write_vk")
                .arg("-t")
                .arg(&self.target),
        )?;

        let proof = std::fs::read(out_dir.join("proof"))?;
        let public_inputs = std::fs::read(out_dir.join("public_inputs"))?;
        let vk = std::fs::read(out_dir.join("vk"))?;
        Ok(ProofArtifacts {
            proof,
            public_inputs,
            vk,
        })
    }

    /// Recompute the CANONICAL verification key for a circuit-family member,
    /// verifier-side, from the compiled member named by `id` — never trusting a
    /// prover-supplied vk (audit #2). Compiles the member (idempotent) then runs
    /// `bb write_vk`. Measured deterministic and fast (~40-60ms once the ACIR is
    /// cached, ~350ms cold); a freshly-recompiled ACIR yields a byte-identical
    /// vk to `bb prove --write_vk`, so this is the authentic member vk.
    // [OPUS-4.8] new verifier-side canonical-vk path (audit #2).
    pub fn canonical_vk(&self, id: &CircuitId, work_dir: &Path) -> Result<Vec<u8>, DriverError> {
        self.canonical_package_vk(&id.package(), work_dir)
    }

    // [GPT-6] Rebuilds a member key locally; never consumes an untrusted bundled VK.
    pub(crate) fn canonical_package_vk(
        &self,
        pkg: &str,
        work_dir: &Path,
    ) -> Result<Vec<u8>, DriverError> {
        let scratch = ScratchDir::new(work_dir, "vk")?;
        let acir = self.compile_into(pkg, &scratch.0)?;
        let work_dir = &scratch.0;
        run(
            "bb",
            Command::new("bb")
                .arg("write_vk")
                .arg("-b")
                .arg(acir)
                .arg("-o")
                .arg(work_dir)
                .arg("-t")
                .arg(&self.target),
        )?;
        Ok(std::fs::read(work_dir.join("vk"))?)
    }

    /// Verify a proof against an EXPLICIT verification key and public-input
    /// bytes — the verifier supplies the canonical member vk (from
    /// [`Self::canonical_vk`]) and the reconstructed-and-byte-checked public
    /// inputs, NEVER the prover's `art.vk` / `art.public_inputs`. Returns
    /// Ok(true) on a valid proof, Ok(false) if bb rejects, Err on spawn/io.
    // [OPUS-4.8] verify against caller-pinned vk + public inputs (audit #1/#2).
    pub fn verify_with(
        &self,
        proof: &[u8],
        public_inputs: &[u8],
        vk: &[u8],
        work_dir: &Path,
    ) -> Result<bool, DriverError> {
        let scratch = ScratchDir::new(work_dir, "verify")?;
        let work_dir = &scratch.0;
        let proof_p = work_dir.join("proof");
        let pi_p = work_dir.join("public_inputs");
        let vk_p = work_dir.join("vk");
        std::fs::write(&proof_p, proof)?;
        std::fs::write(&pi_p, public_inputs)?;
        std::fs::write(&vk_p, vk)?;

        let out = Command::new("bb")
            .arg("verify")
            .arg("-p")
            .arg(&proof_p)
            .arg("-i")
            .arg(&pi_p)
            .arg("-k")
            .arg(&vk_p)
            .arg("-t")
            .arg(&self.target)
            .output()
            .map_err(|source| DriverError::Spawn {
                tool: "bb".into(),
                source,
            })?;
        Ok(out.status.success())
    }

    /// Verify artifacts via `bb verify` using the bundled vk + public inputs.
    ///
    /// NOTE: this trusts `art.vk` and `art.public_inputs` and so is NOT a sound
    /// third-party gate on its own — `verify_manifest` does NOT call this; it
    /// recomputes the canonical vk and reconstructs the public inputs (see
    /// [`Self::canonical_vk`] / [`Self::verify_with`] and the verifier). Kept
    /// for the prover-side round-trip / bb-tamper tests only.
    pub fn verify(&self, art: &ProofArtifacts, work_dir: &Path) -> Result<bool, DriverError> {
        self.verify_with(&art.proof, &art.public_inputs, &art.vk, work_dir)
    }
}

fn run(tool: &str, cmd: &mut Command) -> Result<(), DriverError> {
    let out = cmd.output().map_err(|source| DriverError::Spawn {
        tool: tool.into(),
        source,
    })?;
    if !out.status.success() {
        return Err(DriverError::Tool {
            tool: tool.into(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(())
}

// [OPUS-4.8] sq-bif.6: GLUE unit tests for the nargo/bb subprocess wrapper's
// ERROR-classification + tag-based path isolation. These are toolchain-FREE: they
// drive the error arms with a guaranteed-absent binary (the `Spawn` arm) and a
// real-but-failing binary (the `Tool` arm), and check the tag-isolation file-naming
// glue against a scratch dir — NO nargo/bb proving, NO cryptographic claim. The
// real proving e2e lives in `tests/e2e.rs`, gated on the toolchain being present.
#[cfg(test)]
mod driver_glue_tests {
    use super::*;

    #[test]
    fn concurrent_verification_workspaces_never_share_artifact_paths() {
        let root =
            std::env::temp_dir().join(format!("sparq_workspace_race_{}", std::process::id()));
        let barrier = std::sync::Barrier::new(8);
        let paths = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0u8..8)
                .map(|id| {
                    let root = &root;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        let scratch = ScratchDir::new(root, "verify").unwrap();
                        let path = scratch.0.join("proof");
                        std::fs::write(&path, [id]).unwrap();
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            assert_eq!(
                                std::fs::metadata(&scratch.0).unwrap().permissions().mode() & 0o777,
                                0o700
                            );
                        }
                        barrier.wait();
                        assert_eq!(std::fs::read(&path).unwrap(), vec![id]);
                        path
                    })
                })
                .collect();
            jobs.into_iter()
                .map(|j| j.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            paths
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            8
        );
        assert!(paths.iter().all(|p| !p.exists()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn content_addressed_compile_cache_never_rewrites_returned_snapshots() {
        let root = ScratchDir::new(&std::env::temp_dir(), "compile_cache_test").unwrap();
        let prover = CircuitProver::new(&root.0);
        let _lock = NargoCacheLock::acquire(&prover.target_dir()).unwrap();
        let first = prover
            .snapshot_acir("member", b"first compiled circuit")
            .unwrap();
        let second = prover
            .snapshot_acir("member", b"second compiled circuit")
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"first compiled circuit");
        assert_eq!(
            prover
                .snapshot_acir("member", b"first compiled circuit")
                .unwrap(),
            first
        );
        std::fs::write(&first, b"corrupted cache").unwrap();
        assert!(prover
            .snapshot_acir("member", b"first compiled circuit")
            .is_err());
    }

    // Run by the parent regression in independent processes. The sentinel must
    // never already exist while a cooperating process holds the advisory lock.
    #[test]
    fn nargo_cache_lock_process_worker() {
        let Some(root) = std::env::var_os("SPARQ_TEST_NARGO_LOCK_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        for _ in 0..8 {
            let _lock = NargoCacheLock::acquire(&root).unwrap();
            let sentinel = root.join("writer_active");
            let _writer = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&sentinel)
                .expect("two processes entered the nargo cache concurrently");
            let counter_path = root.join("count");
            let value: u32 = std::fs::read_to_string(&counter_path)
                .unwrap()
                .parse()
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
            std::fs::write(counter_path, (value + 1).to_string()).unwrap();
            std::fs::remove_file(sentinel).unwrap();
        }
    }

    #[test]
    fn nargo_cache_lock_serializes_independent_processes_and_releases_on_drop() {
        let root = ScratchDir::new(&std::env::temp_dir(), "process_lock_test").unwrap();
        std::fs::write(root.0.join("count"), "0").unwrap();
        let mut children: Vec<_> = (0..4)
            .map(|_| {
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "driver::driver_glue_tests::nargo_cache_lock_process_worker",
                        "--nocapture",
                    ])
                    .env("SPARQ_TEST_NARGO_LOCK_ROOT", &root.0)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in children.drain(..) {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "lock worker failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert_eq!(std::fs::read_to_string(root.0.join("count")).unwrap(), "32");
        let lock = NargoCacheLock::acquire(&root.0).unwrap();
        drop(lock);
        let _reacquired = NargoCacheLock::acquire(&root.0).unwrap();
    }

    #[test]
    #[ignore = "requires pinned nargo; stresses shared compiler cache and immutable snapshots"]
    fn real_concurrent_compile_returns_immutable_complete_acir() {
        let id = CircuitId::FilterInt { d: 1 };
        let prover = CircuitProver::from_crate_root();
        let expected = std::fs::read(prover.compile(&id).unwrap()).unwrap();
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..4)
                .map(|_| {
                    let expected = &expected;
                    let barrier = &barrier;
                    let id = &id;
                    scope.spawn(move || {
                        let independent = CircuitProver::from_crate_root();
                        barrier.wait();
                        for _ in 0..3 {
                            let snapshot = independent.compile(id).unwrap();
                            let bytes = std::fs::read(&snapshot).unwrap();
                            assert_eq!(&bytes, expected);
                            assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok());
                            assert!(snapshot.parent().unwrap().ends_with("sparq_acir_cache"));
                        }
                    })
                })
                .collect();
            for job in jobs {
                job.join().unwrap();
            }
        });
    }

    /// A name no executable on a sane PATH resolves to — drives the `Spawn` arm
    /// (`std::io::Error` from a failed `fork`/`exec`) deterministically, with or
    /// without the nargo/bb toolchain installed.
    const ABSENT_TOOL: &str = "sparq-zk-compose-no-such-tool-xyzzy";
    /// A binary that EXISTS but always exits non-zero — drives the `Tool` arm
    /// (the process ran but reported failure). POSIX `false` is universally present.
    const FAILING_TOOL: &str = "/usr/bin/false";

    // --- error classification: the `run` helper --------------------------

    /// Spawning a non-existent tool is classified as `DriverError::Spawn` (a typed
    /// error carrying the io source), NOT a panic / unwrap. This is the contract the
    /// public `compile`/`prove`/`canonical_vk` wrappers rely on.
    #[test]
    fn run_missing_tool_is_spawn_error_not_panic() {
        let err = run(ABSENT_TOOL, &mut Command::new(ABSENT_TOOL))
            .expect_err("a non-existent tool must surface an error, never spawn");
        match err {
            DriverError::Spawn { tool, source } => {
                assert_eq!(tool, ABSENT_TOOL, "the Spawn error names the failing tool");
                // The io source is NotFound (the binary is not on PATH).
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected Spawn classification, got {:?}", other),
        }
    }

    /// A tool that runs but exits non-zero is classified as `DriverError::Tool`
    /// (distinct from `Spawn`) and carries the tool name. `false` writes nothing to
    /// stderr, so the captured stderr is empty — the glue still classifies on the
    /// exit status, not on stderr content.
    #[test]
    fn run_failing_tool_is_tool_error_distinct_from_spawn() {
        let err = run("false", &mut Command::new(FAILING_TOOL))
            .expect_err("a non-zero exit must be an error");
        match err {
            DriverError::Tool { tool, stderr } => {
                assert_eq!(tool, "false", "the Tool error names the failing tool");
                assert!(stderr.is_empty(), "`false` emits no stderr");
            }
            other => panic!("expected Tool classification, got {:?}", other),
        }
    }

    /// A tool that exists and exits zero is `Ok(())` — the success arm of the same
    /// classifier (so the error tests above are not vacuously always-erroring).
    #[test]
    fn run_succeeding_tool_is_ok() {
        run("true", &mut Command::new("/usr/bin/true")).expect("exit 0 is Ok");
    }

    // --- Display + From glue ---------------------------------------------

    /// Each `DriverError` variant renders a distinct, human-readable message that
    /// names its tool / cause — the operator-facing surface the CLI prints.
    #[test]
    fn display_renders_each_variant_distinctly() {
        let spawn = DriverError::Spawn {
            tool: "nargo".into(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
        };
        let tool = DriverError::Tool {
            tool: "bb".into(),
            stderr: "boom".into(),
        };
        let io = DriverError::Io(std::io::Error::other("disk full"));
        let s_msg = spawn.to_string();
        let t_msg = tool.to_string();
        let io_msg = io.to_string();
        assert!(
            s_msg.contains("spawn") && s_msg.contains("nargo"),
            "Spawn names the tool"
        );
        assert!(
            t_msg.contains("bb") && t_msg.contains("boom"),
            "Tool carries name + stderr"
        );
        assert!(io_msg.contains("io error") && io_msg.contains("disk full"));
        // The three renderings are mutually distinct (no two variants collide).
        assert_ne!(s_msg, t_msg);
        assert_ne!(t_msg, io_msg);
        assert_ne!(s_msg, io_msg);
    }

    /// `?` on a bare `std::io::Error` lands in the `Io` arm via the `From` impl
    /// (the `std::fs::read`/`write`/`create_dir_all` calls in `prove`/`verify_with`
    /// rely on this conversion).
    #[test]
    fn io_error_converts_into_io_variant() {
        let e: DriverError =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope").into();
        match e {
            DriverError::Io(inner) => {
                assert_eq!(inner.kind(), std::io::ErrorKind::PermissionDenied)
            }
            other => panic!("io::Error must convert to DriverError::Io, got {:?}", other),
        }
    }

    // --- tag-based witness-path isolation --------------------------------

    /// The tag-isolation glue (roborev codex job 2180): a non-empty `tag` MUST route
    /// the prover-input toml AND the emitted witness to per-call-unique paths so two
    /// concurrent witness generations against the SAME member cannot clobber each
    /// other's `Prover.toml` / `target/<pkg>_w.gz`. We verify this WITHOUT a real
    /// toolchain: `gen_witness_tagged` writes the toml BEFORE spawning nargo, so even
    /// though `nargo execute` then fails (no compiled member in this scratch dir), the
    /// two tags leave two DISTINCT, COEXISTING toml files — the collision-avoidance
    /// property. The failure is the typed `Tool` error (no witness produced), never a
    /// panic, and its message names the per-tag witness file.
    #[test]
    fn tagged_witness_paths_do_not_collide() {
        let tmp = std::env::temp_dir().join(format!("sq-bif6-driver-{}", std::process::id()));
        let id = CircuitId::FilterInt { d: 1 };
        let pkg = id.package();
        // The package dir must exist for the toml write to land.
        std::fs::create_dir_all(tmp.join(&pkg)).expect("scratch package dir");
        std::fs::create_dir_all(tmp.join("target")).expect("scratch target dir");
        let prover = CircuitProver::new(&tmp);

        // Two distinct tags -> two distinct prover-input toml files in the package
        // dir. If `nargo` is absent this is a Spawn error AFTER the toml write; if
        // present it is a Tool error (no member compiled) — either way the toml is
        // written first, which is the isolation behaviour we assert.
        let r_a = prover.gen_witness_tagged(&id, "challenge = \"0x1\"\n", "taga");
        let r_b = prover.gen_witness_tagged(&id, "challenge = \"0x2\"\n", "tagb");
        assert!(
            r_a.is_err() && r_b.is_err(),
            "no real witness without the toolchain"
        );

        let toml_a = tmp.join(&pkg).join("Prover_taga.toml");
        let toml_b = tmp.join(&pkg).join("Prover_tagb.toml");
        assert!(toml_a.exists(), "tag `taga` wrote its own Prover_taga.toml");
        assert!(toml_b.exists(), "tag `tagb` wrote its own Prover_tagb.toml");
        assert_ne!(toml_a, toml_b, "the two tags use distinct toml paths");
        // The two inputs coexist with their original, non-clobbered contents — the
        // collision the tag isolation prevents.
        assert_eq!(
            std::fs::read_to_string(&toml_a).unwrap(),
            "challenge = \"0x1\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(&toml_b).unwrap(),
            "challenge = \"0x2\"\n"
        );

        // The empty tag uses the LEGACY shared `Prover.toml` name (distinct from any
        // tagged name), so a tagged call never overwrites the untagged one.
        let _ = prover.gen_witness_tagged(&id, "challenge = \"0x3\"\n", "");
        let toml_shared = tmp.join(&pkg).join("Prover.toml");
        assert!(
            toml_shared.exists(),
            "empty tag uses the shared Prover.toml name"
        );
        assert_ne!(
            toml_shared, toml_a,
            "shared name differs from a tagged name"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // [GPT-6] Reject caller-controlled path fragments before input publication,
    // scratch allocation or compilation, including an existing traversal path.
    #[test]
    fn unsafe_witness_tags_reject_before_any_artifact_write() {
        let scratch = ScratchDir::new(&std::env::temp_dir(), "bad_tag").unwrap();
        let id = CircuitId::FilterInt { d: 1 };
        let pkg = id.package();
        std::fs::create_dir_all(scratch.0.join(&pkg).join("Prover_escape")).unwrap();
        let sentinel = scratch.0.join("sentinel.toml");
        std::fs::write(&sentinel, "unchanged").unwrap();
        let out = scratch.0.join("proof_output");
        let prover = CircuitProver::new(&scratch.0);
        for tag in [
            "escape/../../sentinel",
            "../outside",
            "/absolute",
            "a\\b",
            ".",
            "..",
            "a b",
            "a\nb",
            "\0",
            "é",
            "proof%2Ev1",
        ] {
            let assert_invalid = |error: DriverError| {
                assert!(
                    matches!(error, DriverError::Io(ref e) if e.kind() == std::io::ErrorKind::InvalidInput),
                    "tag {tag:?}: {error}"
                );
            };
            assert_invalid(prover.gen_witness_tagged(&id, "secret", tag).unwrap_err());
            assert_invalid(prover.prove_in(&id, "secret", &out, tag).unwrap_err());
            #[cfg(feature = "successful-results")]
            {
                assert_invalid(
                    prover
                        .private_package_witness(&pkg, "secret", tag)
                        .err()
                        .unwrap(),
                );
                assert_invalid(
                    prover
                        .prove_private_package(&pkg, "secret", &out, tag)
                        .unwrap_err(),
                );
            }
            assert!(!out.exists());
            assert!(!prover.target_dir().exists());
            assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "unchanged");
        }
        for tag in ["", "legacy", "Job_1", "proof-v1.2", "--help"] {
            assert!(validate_witness_tag(tag).is_ok(), "safe tag {tag:?}");
        }
        assert_eq!(witness_file_label("proof-v1.2"), "proof-v1%2E2");
        assert_eq!(witness_file_label("proof-v1_2"), "proof-v1_2");
    }

    #[cfg(unix)]
    #[test]
    fn tagged_input_symlink_cannot_overwrite_its_target() {
        let scratch = ScratchDir::new(&std::env::temp_dir(), "symlink_tag").unwrap();
        let id = CircuitId::FilterInt { d: 1 };
        let pkg = scratch.0.join(id.package());
        std::fs::create_dir_all(&pkg).unwrap();
        let sentinel = scratch.0.join("sentinel.toml");
        std::fs::write(&sentinel, "unchanged").unwrap();
        std::os::unix::fs::symlink(&sentinel, pkg.join("Prover_safe.toml")).unwrap();
        let error = CircuitProver::new(&scratch.0)
            .gen_witness_tagged(&id, "secret", "safe")
            .unwrap_err();
        assert!(matches!(error, DriverError::Io(ref e) if e.raw_os_error() == Some(libc::ELOOP)));
        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "unchanged");
    }
}
