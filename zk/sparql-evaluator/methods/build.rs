// SDK guest compilation with explicit source-path remapping.
// zkp-14.5: the optional V5 guest reuses this child build. The exact
// guest keeps its flags, target subdirectory, manifest, lock and constants in
// `methods.rs`, so the feature is intended not to change its bytes or image ID
// (pending a byte comparison against the previous build; not yet measured).
use std::{env, fs, path::PathBuf};

/// One independently locked guest workspace and the names generated for it.
struct Guest {
    /// Directory below `methods/` holding the guest's own manifest and lock.
    dir: &'static str,
    /// Package and binary name, target subdirectory and artifact file stem.
    package: &'static str,
    /// Prefix of the generated `_ELF` and `_ID` constants.
    constants: &'static str,
    /// Generated file in `OUT_DIR` that `src/lib.rs` includes.
    output: &'static str,
    /// Guest-crate features; a non-empty set builds into its own target subdirectory.
    features: &'static str,
}

/// The default exact V1-V3 guest; always built.
const EXACT: Guest = Guest {
    dir: "guest",
    package: "sparq-exact-guest",
    constants: "SPARQ_EXACT_GUEST",
    output: "methods.rs",
    features: "",
};

/// The separately pinned V5 guest; built only with the `authenticated-rdf` feature.
const AUTHRDF: Guest = Guest {
    dir: "guest-authrdf",
    package: "sparq-authrdf-guest",
    constants: "SPARQ_AUTHRDF_GUEST",
    output: "authrdf_methods.rs",
    features: "",
};

/// The V5 guest with per-phase cycle reporting, for measurement only; built only
/// with the `phase-cycles` feature. It is never an accepted production image.
const AUTHRDF_PHASES: Guest = Guest {
    dir: "guest-authrdf",
    package: "sparq-authrdf-guest",
    constants: "SPARQ_AUTHRDF_PHASES_GUEST",
    output: "authrdf_phases_methods.rs",
    features: "phase-cycles",
};

/// Repository-relative source inputs of the exact guest image.
///
/// The root manifest supplies workspace inheritance to the path crates. The
/// list names every path dependency (`sparq-canon` through the model's
/// `graph-results` feature), every patched SDK source and the guest workspace
/// with its lock. Registry dependencies are pinned by that lock.
const SOURCES: [&str; 12] = [
    "Cargo.toml",
    "crates/sparq-canon",
    "crates/sparq-core",
    "crates/sparq-engine",
    "crates/sparq-substrate",
    "vendor/spargebra",
    "vendor/zk-sdk/ark-crypto-primitives-0.5.0",
    "vendor/zk-sdk/ark-relations-0.5.1",
    "vendor/zk-sdk/risc0-zkos-v1compat-2.2.3",
    "vendor/zk-sdk/risc0-zkvm-3.0.6",
    "zk/sparql-evaluator/model",
    "zk/sparql-evaluator/methods/guest",
];

/// Additional source input of the V5 guest image: its own workspace and lock.
///
/// Every other V5 path dependency and patch is already in [`SOURCES`].
const AUTHRDF_SOURCES: [&str; 1] = ["zk/sparql-evaluator/methods/guest-authrdf"];

/// Paths shared by every guest build.
struct Build {
    methods: PathBuf,
    repo: PathBuf,
    out: PathBuf,
    cargo_home: PathBuf,
}

fn main() {
    assert!(
        env::var_os("RISC0_SKIP_BUILD").is_none(),
        "guest build cannot be skipped"
    );
    let methods = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .canonicalize()
        .expect("methods directory");
    let repo = methods
        .ancestors()
        .nth(3)
        .expect("repository root")
        .to_path_buf();
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").expect("Cargo home location")).join(".cargo")
        })
        .canonicalize()
        .expect("Cargo home directory");
    let build = Build {
        methods,
        repo,
        out,
        cargo_home,
    };
    build.guest(&EXACT);
    let mut sources = SOURCES.to_vec();
    // Cargo fingerprints enabled features itself; a feature change reruns this script.
    if env::var_os("CARGO_FEATURE_AUTHENTICATED_RDF").is_some() {
        build.guest(&AUTHRDF);
        sources.extend(AUTHRDF_SOURCES);
    }
    if env::var_os("CARGO_FEATURE_PHASE_CYCLES").is_some() {
        build.guest(&AUTHRDF_PHASES);
    }
    for source in sources {
        println!("cargo:rerun-if-changed={}", build.repo.join(source).display());
    }
    for key in ["RISC0_HOME", "RISC0_SKIP_BUILD", "CARGO_HOME"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
}

impl Guest {
    /// Target subdirectory and artifact stem: the package, plus any features.
    fn stem(&self) -> String {
        if self.features.is_empty() {
            self.package.to_owned()
        } else {
            format!("{}-{}", self.package, self.features)
        }
    }
}

impl Build {
    /// Compiles one guest with its own locked child Cargo and writes its constants.
    fn guest(&self, guest: &Guest) {
        let target = env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| self.out.join("guest-target"))
            .join(guest.stem());
        let flags = vec![
            format!("--remap-path-prefix={}=/sparq", self.repo.display()),
            format!("--remap-path-prefix={}=/cargo-home", self.cargo_home.display()),
            "-Z".into(),
            format!("remap-cwd-prefix=/sparq/{}", guest.dir),
            "-Z".into(),
            "location-detail=none".into(),
        ];
        let directory = self.methods.join(guest.dir);
        let mut command = risc0_build::cargo_command("build", &flags);
        // SDK3.0.6 removes CARGO_* and otherwise inherits the host Clippy driver.
        // Change only this child's environment; never process-global environment.
        command
            .env("CARGO_HOME", &self.cargo_home)
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .env_remove("RUSTC_WRAPPER")
            .current_dir(&directory)
            .args(["--release", "--manifest-path"])
            .arg(directory.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&target);
        if !guest.features.is_empty() {
            command.args(["--features", guest.features]);
        }
        if !command.get_args().any(|arg| arg == "--locked") {
            command.arg("--locked");
        }
        if let Some(jobs) = env::var_os("CARGO_BUILD_JOBS") {
            command.env("CARGO_BUILD_JOBS", jobs);
        }
        let status = command.status().expect("real guest compilation process");
        assert!(status.success(), "real guest compilation failed");
        let user = fs::read(
            target
                .join("riscv32im-risc0-zkvm-elf/release")
                .join(guest.package),
        )
        .expect("compiled guest ELF");
        let kernel = risc0_build::GuestOptions::default().kernel();
        let binary = risc0_binfmt::ProgramBinary::new(&user, &kernel).encode();
        let id = risc0_binfmt::compute_image_id(&binary).expect("guest image identity");
        let artifact = self.out.join(format!("{}.bin", guest.stem()));
        fs::write(&artifact, binary).expect("guest artifact output");
        // The generated text embeds the absolute `OUT_DIR` artifact path, so it is
        // not a build-independent invariant; the artifact bytes and image ID are.
        let constants = format!(
            "pub const {prefix}_ELF: &[u8] = include_bytes!({artifact:?});\n\
             pub const {prefix}_ID: [u32; 8] = {id:?};\n",
            prefix = guest.constants,
            id = id.as_words(),
        );
        fs::write(self.out.join(guest.output), constants).expect("guest constants output");
    }
}
