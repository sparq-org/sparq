// Context preparation is an explicit step before cargo/vercel build, never a
// network fetch hidden in a build script. Embed the original cache bytes.
fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let files = [
        (
            "https://www.w3.org/ns/credentials/v2",
            "credentials-v2.jsonld",
        ),
        (
            "https://www.w3.org/2018/credentials/v1",
            "credentials-v1.jsonld",
        ),
        (
            "https://www.w3.org/ns/credentials/examples/v2",
            "examples-v2.jsonld",
        ),
        (
            "https://w3id.org/security/data-integrity/v2",
            "data-integrity-v2.jsonld",
        ),
        (
            "https://w3id.org/security/data-integrity/v1",
            "data-integrity-v1.jsonld",
        ),
        (
            "https://w3id.org/security/multikey/v1",
            "multikey-v1.jsonld",
        ),
        ("https://www.w3.org/ns/did/v1", "did-v1.jsonld"),
    ];
    let mut generated =
        String::from("fn embedded_contexts() -> std::collections::BTreeMap<String, String> { [\n");
    for (url, file) in files {
        let path = manifest.join("contexts").join(file);
        assert!(
            path.is_file(),
            "run python3 deploy/vc-api/prepare-contexts.py before building"
        );
        println!("cargo:rerun-if-changed={}", path.display());
        generated.push_str(&format!(
            "({url:?}.into(), include_str!({:?}).into()),\n",
            path
        ));
    }
    generated.push_str("].into_iter().collect() }\n");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("contexts.rs"), generated).unwrap();
}
