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
//! Toolchain-gated: needs `nargo`. Skips cleanly when absent (like the other
//! witness-gen tests in `e2e.rs`).

use sparq_zk_compose::build::{encode_decimal_literal, encode_signed_int_literal};
use sparq_zk_compose::manifest::FieldHex;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Returns Ok(true) if `nargo execute` produced a witness (relation satisfiable),
/// Ok(false) if it didn't (unsatisfiable / mismatched binding). `tag` isolates
/// the Prover.toml + witness file so concurrent tests don't race.
fn try_witness(pkg: &str, toml: &str, tag: &str) -> bool {
    let dir = compose_dir();
    let prover_name = format!("Prover_{tag}");
    let witness_name = format!("{pkg}_w_{tag}");
    let toml_path = dir.join(pkg).join(format!("{prover_name}.toml"));
    std::fs::write(&toml_path, toml).expect("write prover toml");
    let witness_path = dir.join("target").join(format!("{witness_name}.gz"));
    let _ = std::fs::remove_file(&witness_path);
    // Compile is idempotent; required so target/<pkg>.json exists.
    let _ = Command::new("nargo")
        .arg("compile")
        .arg("--package")
        .arg(pkg)
        .current_dir(&dir)
        .output()
        .expect("nargo compile runs");
    let _ = Command::new("nargo")
        .arg("execute")
        .arg(&witness_name)
        .arg("--package")
        .arg(pkg)
        .arg("--prover-name")
        .arg(&prover_name)
        .current_dir(&dir)
        .output()
        .expect("nargo execute runs");
    let ok = witness_path.exists();
    let _ = std::fs::remove_file(&witness_path);
    let _ = std::fs::remove_file(&toml_path);
    ok
}

#[test]
fn signed_int_binding_matches_real_encode() {
    if !support::require_toolchain("signed_int_binding_matches_real_encode", &["nargo"]) {
        return;
    }
    // value = -42 committed by the REAL pipeline; the circuit must re-derive its
    // encoding from the witnessed sign + magnitude digits ("42", neg=true).
    let enc = encode_signed_int_literal(-42);
    // -42 < +1 => lt true. Satisfiable iff the token rebuild byte-matches encode.
    let toml = signed_int_toml(&enc, 0, false, 1, true, true, b"42");
    assert!(
        try_witness("filter_signed_int_d2", &toml, "sq1q9h_neg42_ok"),
        "signed-int -42 binding must satisfy the relation (token rebuild must \
         byte-match sparq_zk::encode)"
    );
}

#[test]
fn signed_int_binding_rejects_sign_lie() {
    if !support::require_toolchain("signed_int_binding_rejects_sign_lie", &["nargo"]) {
        return;
    }
    // Anchor encoding is for -42, but the prover witnesses +42 (neg=false). The
    // operand binding (h2(LITERAL, blake3("42"^^int)) != enc_for_"-42") must make
    // this UNSATISFIABLE — the sign cannot be forged against a committed value.
    let enc_neg = encode_signed_int_literal(-42);
    let toml = signed_int_toml(&enc_neg, 2, false, 0, true, false, b"42");
    assert!(
        !try_witness("filter_signed_int_d2", &toml, "sq1q9h_signlie"),
        "claiming +42 against the -42 anchor encoding must be unsatisfiable"
    );
}

#[test]
fn decimal_binding_matches_real_encode() {
    if !support::require_toolchain("decimal_binding_matches_real_encode", &["nargo"]) {
        return;
    }
    // value = 123.45 committed by the REAL pipeline (ID=3, FD=2).
    let enc = encode_decimal_literal(false, 123, "45");
    // 123.45 > 123.40 (bound_scaled = 12340) => gt true.
    let toml = decimal_toml(&enc, 2, false, 12340, true, false, b"123", b"45");
    assert!(
        try_witness("filter_decimal_i3_f2", &toml, "sq1q9h_dec_ok"),
        "decimal 123.45 binding must satisfy the relation (token rebuild must \
         byte-match sparq_zk::encode)"
    );
}

#[test]
fn decimal_binding_rejects_fraction_lie() {
    if !support::require_toolchain("decimal_binding_rejects_fraction_lie", &["nargo"]) {
        return;
    }
    // Anchor encoding is for 123.45; prover witnesses 123.99 fraction digits. The
    // binding (different token bytes => different enc) must be UNSATISFIABLE.
    let enc_45 = encode_decimal_literal(false, 123, "45");
    let toml = decimal_toml(&enc_45, 2, false, 0, true, false, b"123", b"99");
    assert!(
        !try_witness("filter_decimal_i3_f2", &toml, "sq1q9h_dec_fraclie"),
        "claiming .99 against the .45 anchor encoding must be unsatisfiable"
    );
}
