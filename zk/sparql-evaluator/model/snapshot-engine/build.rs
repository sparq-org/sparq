use std::{env, fs, path::Path};

#[path = "src/patch.rs"]
mod patch;

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("engine output directory");
    for entry in fs::read_dir(source).expect("engine sources") {
        let entry = entry.expect("engine source entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("engine source type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("engine output source");
        }
    }
}

fn main() {
    let manifest = env::var_os("CARGO_MANIFEST_DIR").expect("adapter directory");
    let source = Path::new(&manifest).join("../../../../crates/sparq-engine/src");
    let output = env::var_os("OUT_DIR").expect("adapter output directory");
    let output = Path::new(&output).join("engine");
    if output.exists() {
        fs::remove_dir_all(&output).expect("remove previous generated engine");
    }
    copy_tree(&source, &output);
    let lib = output.join("lib.rs");
    let text = fs::read_to_string(&lib).expect("engine library source");
    let body = text.strip_prefix(
        "#![doc = include_str!(\"../README.md\")]\n#![warn(clippy::undocumented_unsafe_blocks)]\n"
    ).expect("review changed engine crate attributes");
    fs::write(lib, body).expect("engine library output");
    let compiled = output.join("exec/compiled.rs");
    let text = fs::read_to_string(&compiled).expect("numeric consumers");
    fs::write(
        compiled,
        patch::v1_snapshot_numeric_bound(text, include_str!("v1-numeric-bound.patch")),
    )
    .expect("V1 numeric consumers output");
    // Keep the engine's complete unit suite, with its wider-comparison test
    // asserting this adapter's typed capacity cause and the same positive controls.
    let tests = output.join("exec/exact_decimal_tests.rs");
    let text = fs::read_to_string(&tests).expect("numeric consumer tests");
    fs::write(tests, patch::v1_snapshot_numeric_bound(text, include_str!("v1-numeric-tests.patch")))
        .expect("V1 numeric tests output");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=v1-numeric-bound.patch");
    println!("cargo:rerun-if-changed=v1-numeric-tests.patch");
}
