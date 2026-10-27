// sq-1q9h: NON-circular soundness anchor for the SIGNED xsd:integer
// and xsd:decimal FILTER circuit members.
//!
//! The Noir-side tests (`compose_core/src/tests.nr`) compute `operand_enc` with
//! a test-side REPLICA of the token rebuild, so they alone can't prove the
//! circuit binds to what the REAL commitment pipeline produces. These tests
//! close that loop: `operand_enc` is computed by `sparq_zk::encode` (via the
//! `build::encode_signed_int_literal` / `encode_decimal_literal` host helpers),
//! and `nargo execute` is run on the compiled member with the witnessed
//! sign/digits. The witness generates ONLY IF the in-circuit token rebuild
//! byte-matches `sparq_zk::encode` — so a passing execute is direct evidence the
//! operand binding is sound (and a deliberately MISMATCHED enc must fail).
//!
//! Toolchain-gated: needs `nargo`, through the shared gate (a missing tool fails
//! the test unless the lane declares itself toolchain-free). Each rejection test
//! first proves the HONEST witness for the same anchor, and then requires the
//! refusal to name the operand-binding constraint, so an infrastructure failure
//! cannot pass as a rejection.

use sparq_zk_compose::build::{encode_decimal_literal, encode_signed_int_literal};
use sparq_zk_compose::manifest::FieldHex;
use std::path::{Path, PathBuf};

mod support;

/// `crates/sparq-zk-compose` -> `../../zk/compose`.
fn compose_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("zk").join("compose"))
        .expect("workspace layout")
}

/// Render a Prover.toml for `filter_signed_int_d{MD}` (param order matches
/// `main`: challenge, operand_enc, op, bound_neg, bound, expected, neg, digits).
fn signed_int_toml(
    operand_enc: &FieldHex,
    op: u32,
    bound_neg: bool,
    bound: u64,
    expected: bool,
    neg: bool,
    mag_digits: &[u8],
) -> String {
    let digits = mag_digits
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "challenge = \"0x2a\"\noperand_enc = \"{}\"\nop = \"{op}\"\nbound_neg = {bound_neg}\nbound = \"{bound}\"\nexpected = {expected}\nneg = {neg}\nmag_digits = [{digits}]\n",
        operand_enc.0
    )
}

/// Render a Prover.toml for `filter_decimal_i{ID}_f{FD}` (param order matches
/// `main`: challenge, operand_enc, op, bound_neg, bound_scaled, expected, neg,
/// int_digits, frac_digits).
#[allow(clippy::too_many_arguments)]
fn decimal_toml(
    operand_enc: &FieldHex,
    op: u32,
    bound_neg: bool,
    bound_scaled: u64,
    expected: bool,
    neg: bool,
    int_digits: &[u8],
    frac_digits: &[u8],
) -> String {
    let int_d = int_digits
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let frac_d = frac_digits
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "challenge = \"0x2a\"\noperand_enc = \"{}\"\nop = \"{op}\"\nbound_neg = {bound_neg}\nbound_scaled = \"{bound_scaled}\"\nexpected = {expected}\nneg = {neg}\nint_digits = [{int_d}]\nfrac_digits = [{frac_d}]\n",
        operand_enc.0
    )
}

/// What `nargo execute` made of one Prover.toml.
#[derive(Debug)]
enum Witness {
    /// Execute exited 0 and wrote the witness: the relation is satisfiable.
    Satisfied,
    /// Execute wrote no witness; carries its constraint diagnostic.
    Unsatisfied(String),
}

/// The constraint the operand binding asserts (`filter_signed.nr`). A refusal is
/// evidence about the BINDING only when it names this; a spawn, parse or compile
/// failure would also leave no witness, and must not count as a rejection.
const BINDING_DIAGNOSTIC: &str = "operand encoding mismatch";

/// Run `nargo compile` (asserted to succeed) and then `nargo execute` on `toml`,
/// and report what execute did. `tag` isolates the Prover.toml + witness file so
/// concurrent tests don't race.
fn witness(gate: &support::LiveToolchain, pkg: &str, toml: &str, tag: &str) -> Witness {
    let dir = compose_dir();
    let prover_name = format!("Prover_{tag}");
    let witness_name = format!("{pkg}_w_{tag}");
    let toml_path = dir.join(pkg).join(format!("{prover_name}.toml"));
    std::fs::write(&toml_path, toml).expect("write prover toml");
    let witness_path = dir.join("target").join(format!("{witness_name}.gz"));
    let _ = std::fs::remove_file(&witness_path);
    // Compile is idempotent; required so target/<pkg>.json exists.
    let compile = support::live_tool(gate, "nargo")
        .arg("compile")
        .arg("--package")
        .arg(pkg)
        .current_dir(&dir)
        .output()
        .expect("nargo compile runs");
    assert!(
        compile.status.success(),
        "nargo compile --package {pkg} failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let execute = support::live_tool(gate, "nargo")
        .arg("execute")
        .arg(&witness_name)
        .arg("--package")
        .arg(pkg)
        .arg("--prover-name")
        .arg(&prover_name)
        .current_dir(&dir)
        .output()
        .expect("nargo execute runs");
    let produced = witness_path.exists();
    let _ = std::fs::remove_file(&witness_path);
    let _ = std::fs::remove_file(&toml_path);
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&execute.stdout),
        String::from_utf8_lossy(&execute.stderr)
    );
    if produced {
        assert!(
            execute.status.success(),
            "nargo wrote a witness but exited unsuccessfully: {diagnostic}"
        );
        Witness::Satisfied
    } else {
        // Pinned nargo can exit successfully on an unsatisfied relation. Require
        // its constraint diagnostic as well as the absence of a witness.
        assert!(
            diagnostic.contains(BINDING_DIAGNOSTIC),
            "nargo produced no witness without the expected binding diagnostic: {diagnostic}"
        );
        Witness::Unsatisfied(diagnostic)
    }
}

/// `w` must be a refusal that names the operand-binding constraint.
fn assert_binding_refused(w: Witness, what: &str) {
    match w {
        Witness::Unsatisfied(stderr) => assert!(
            stderr.contains(BINDING_DIAGNOSTIC),
            "{what}: nargo refused the witness, but not on the operand binding \
             (expected `{BINDING_DIAGNOSTIC}`):\n{stderr}"
        ),
        Witness::Satisfied => panic!("{what}: the relation was satisfied"),
    }
}

#[test]
fn signed_int_binding_matches_real_encode() {
    let gate = match support::gate("signed_int_binding_matches_real_encode", &["nargo"]) {
        Some(g) => g,
        None => return,
    };
    // value = -42 committed by the REAL pipeline; the circuit must re-derive its
    // encoding from the witnessed sign + magnitude digits ("42", neg=true).
    let enc = encode_signed_int_literal(-42);
    // -42 < +1 => lt true. Satisfiable iff the token rebuild byte-matches encode.
    let toml = signed_int_toml(&enc, 0, false, 1, true, true, b"42");
    let got = witness(&gate, "filter_signed_int_d2", &toml, "sq1q9h_neg42_ok");
    assert!(
        matches!(got, Witness::Satisfied),
        "signed-int -42 binding must satisfy the relation (token rebuild must \
         byte-match sparq_zk::encode): {got:?}"
    );
    support::complete(gate);
}

#[test]
fn signed_int_binding_rejects_sign_lie() {
    let gate = match support::gate("signed_int_binding_rejects_sign_lie", &["nargo"]) {
        Some(g) => g,
        None => return,
    };
    let enc_neg = encode_signed_int_literal(-42);
    // The honest witness for the SAME anchor, operator and bound proves first: -42 >
    // 0 is false, witnessed as neg=true. Without it, a refusal below could be any
    // failure of this package rather than the binding.
    let honest = signed_int_toml(&enc_neg, 2, false, 0, false, true, b"42");
    let got = witness(&gate, "filter_signed_int_d2", &honest, "sq1q9h_sign_honest");
    assert!(
        matches!(got, Witness::Satisfied),
        "the honest -42 witness must satisfy the relation: {got:?}"
    );
    // Anchor encoding is for -42, but the prover witnesses +42 (neg=false). The
    // operand binding (h2(LITERAL, blake3("42"^^int)) != enc_for_"-42") must make
    // this UNSATISFIABLE — the sign cannot be forged against a committed value.
    let lie = signed_int_toml(&enc_neg, 2, false, 0, true, false, b"42");
    assert_binding_refused(
        witness(&gate, "filter_signed_int_d2", &lie, "sq1q9h_signlie"),
        "claiming +42 against the -42 anchor encoding",
    );
    support::complete(gate);
}

#[test]
fn decimal_binding_matches_real_encode() {
    let gate = match support::gate("decimal_binding_matches_real_encode", &["nargo"]) {
        Some(g) => g,
        None => return,
    };
    // value = 123.45 committed by the REAL pipeline (ID=3, FD=2).
    let enc = encode_decimal_literal(false, 123, "45");
    // 123.45 > 123.40 (bound_scaled = 12340) => gt true.
    let toml = decimal_toml(&enc, 2, false, 12340, true, false, b"123", b"45");
    let got = witness(&gate, "filter_decimal_i3_f2", &toml, "sq1q9h_dec_ok");
    assert!(
        matches!(got, Witness::Satisfied),
        "decimal 123.45 binding must satisfy the relation (token rebuild must \
         byte-match sparq_zk::encode): {got:?}"
    );
    support::complete(gate);
}

#[test]
fn decimal_binding_rejects_fraction_lie() {
    let gate = match support::gate("decimal_binding_rejects_fraction_lie", &["nargo"]) {
        Some(g) => g,
        None => return,
    };
    let enc_45 = encode_decimal_literal(false, 123, "45");
    // The honest witness for the SAME anchor, operator and bound proves first.
    let honest = decimal_toml(&enc_45, 2, false, 0, true, false, b"123", b"45");
    let got = witness(&gate, "filter_decimal_i3_f2", &honest, "sq1q9h_dec_honest");
    assert!(
        matches!(got, Witness::Satisfied),
        "the honest 123.45 witness must satisfy the relation: {got:?}"
    );
    // Anchor encoding is for 123.45; prover witnesses 123.99 fraction digits. The
    // binding (different token bytes => different enc) must be UNSATISFIABLE.
    let lie = decimal_toml(&enc_45, 2, false, 0, true, false, b"123", b"99");
    assert_binding_refused(
        witness(&gate, "filter_decimal_i3_f2", &lie, "sq1q9h_dec_fraclie"),
        "claiming .99 against the .45 anchor encoding",
    );
    support::complete(gate);
}
