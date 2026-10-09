//! Compile the research predicate with its pinned compiler and scalar field.
// [GPT-6]
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=circuits/eligibility.circom");
    let version = Command::new("circom")
        .arg("--version")
        .output()
        .expect("install circom 2.2.2 to build the native composition experiment");
    assert!(version.status.success(), "circom --version failed");
    assert!(
        String::from_utf8_lossy(&version.stdout).trim() == "circom compiler 2.2.2",
        "this experiment pins circom 2.2.2"
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"));
    let output = Command::new("circom")
        .args([
            "circuits/eligibility.circom",
            "--r1cs",
            "--wasm",
            "--sym",
            "--O0",
            "--prime",
            "bls12381",
            "-o",
        ])
        .arg(&out_dir)
        .output()
        .expect("execute circom");
    assert!(
        output.status.success(),
        "circom failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Prevent a compiler/order change from silently linking the wrong inputs.
    let symbols = fs::read_to_string(out_dir.join("eligibility.sym")).expect("read symbols");
    for (name, wire) in [("threshold", "1"), ("income", "2"), ("rent", "3")] {
        let expected = format!("main.{name}");
        assert!(
            symbols.lines().any(|line| {
                let columns: Vec<_> = line.split(',').collect();
                columns.len() == 4 && columns[1] == wire && columns[3] == expected
            }),
            "unexpected witness order for {name}"
        );
    }
}
