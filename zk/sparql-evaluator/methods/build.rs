// [GPT-6] SDK guest compilation with explicit source-path remapping.
use std::{env, fs, path::PathBuf};

fn main() {
    assert!(
        env::var_os("RISC0_SKIP_BUILD").is_none(),
        "guest build cannot be skipped"
    );
    let methods = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .canonicalize()
        .expect("methods directory");
    let repo = methods.ancestors().nth(3).expect("repository root");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").expect("Cargo home location")).join(".cargo")
        })
        .canonicalize()
        .expect("Cargo home directory");
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| out.join("guest-target"))
        .join("sparq-exact-guest");
    let flags = vec![
        format!("--remap-path-prefix={}=/sparq", repo.display()),
        format!("--remap-path-prefix={}=/cargo-home", cargo_home.display()),
        "-Z".into(),
        "remap-cwd-prefix=/sparq/guest".into(),
        "-Z".into(),
        "location-detail=none".into(),
    ];
    let mut command = risc0_build::cargo_command("build", &flags);
    // SDK3.0.6 removes CARGO_* and otherwise inherits the host Clippy driver.
    // Change only this child's environment; never process-global environment.
    command
        .env("CARGO_HOME", &cargo_home)
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_WRAPPER")
        .current_dir(methods.join("guest"))
        .args(["--release", "--manifest-path"])
        .arg(methods.join("guest/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target);
    if !command.get_args().any(|arg| arg == "--locked") {
        command.arg("--locked");
    }
    if let Some(jobs) = env::var_os("CARGO_BUILD_JOBS") {
        command.env("CARGO_BUILD_JOBS", jobs);
    }
    let status = command.status().expect("real guest compilation process");
    assert!(status.success(), "real guest compilation failed");
    let user = fs::read(target.join("riscv32im-risc0-zkvm-elf/release/sparq-exact-guest"))
        .expect("compiled guest ELF");
    let kernel = risc0_build::GuestOptions::default().kernel();
    let binary = risc0_binfmt::ProgramBinary::new(&user, &kernel).encode();
    let id = risc0_binfmt::compute_image_id(&binary).expect("guest image identity");
    let artifact = out.join("sparq-exact-guest.bin");
    fs::write(&artifact, binary).expect("guest artifact output");
    fs::write(
        out.join("methods.rs"),
        format!(
            "pub const SPARQ_EXACT_GUEST_ELF: &[u8] = include_bytes!({:?});\n\
         pub const SPARQ_EXACT_GUEST_ID: [u32; 8] = {:?};\n",
            artifact,
            id.as_words()
        ),
    )
    .expect("guest constants output");
    for source in [
        "Cargo.toml",
        "crates/sparq-core",
        "crates/sparq-engine",
        "crates/sparq-substrate",
        "vendor/spargebra",
        "zk/sparql-evaluator/model",
        "zk/sparql-evaluator/methods/guest",
    ] {
        println!("cargo:rerun-if-changed={}", repo.join(source).display());
    }
    for key in ["RISC0_HOME", "RISC0_SKIP_BUILD", "CARGO_HOME"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
}
