//! [GPT-6] Default host gate for measured compatibility evidence, without nargo or bb.

use std::path::Path;
use std::process::Command;

#[test]
fn result_compatibility_evidence_and_corruption_controls() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg(root.join("bench/zk-compose/scripts/verify_result_evidence.py"))
        .arg("--self-test")
        .current_dir(&root)
        .output()
        .expect("the repository's host evidence checks require Python 3");
    assert!(
        output.status.success(),
        "compatibility evidence failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
