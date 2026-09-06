// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
//! Prover.toml emission for the circuit family.
//!
//! Each circuit `main`'s public + private inputs are written in declaration
//! order. We render field elements as decimal-free `0x` hex (nargo's toml
//! reader accepts `0x`-prefixed Field literals) and arrays as inline tables.
//! Private witnesses (graph encodings, filter digits) are supplied by the
//! prover driver, never present in the manifest.

use crate::build::{FilterSignedWitness, JoinWitness};
use crate::manifest::{CircuitId, FieldHex, ProofInputs};

/// Error returned by [`prover_toml_for`] when a `Prover.toml` cannot be emitted
/// for the given inputs.
///
/// [OPUS-4.8] sq-fi03 / PR #178: `prover_toml_for` is a public fn, so a premature
/// call on a not-yet-wired arm must surface a recoverable error rather than panic
/// (a `unimplemented!` in a public fn is a downstream-crash footgun).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProverTomlError {
    /// A [`ProofInputs::JoinEq`] was passed without its private [`JoinWitness`]:
    /// the `join_eq` member's private inputs
    /// (`enc_a`/`counts_a`/`enc_b`/`counts_b`/`row_a`/`row_b`/`blinding`) live in
    /// the witness, not the manifest, so a join Prover.toml cannot be emitted
    /// without it. The caller obtains the witness from [`crate::build::build_join`]
    /// and threads it through `prover_toml_for`'s `join_witness` parameter.
    // [OPUS-4.8] sq-r2s8: the join proving path is now implemented; this error is
    // the "witness omitted for a JoinEq input" recoverable failure (no panic in a
    // public fn).
    JoinEqMissingWitness,
    /// A [`ProofInputs::FilterSignedInt`] or [`ProofInputs::FilterDecimal`] was
    /// passed without its private [`crate::build::FilterSignedWitness`]: the
    /// operand's SIGN flag and canonical digits live in the witness, not the
    /// manifest, so the Prover.toml cannot be emitted without it. The caller obtains
    /// the witness from [`crate::build::build_filter_signed_int`] /
    /// [`crate::build::build_filter_decimal`] and threads it through
    /// `prover_toml_for`'s `filter_signed_witness` parameter.
    // [OPUS-4.8] sq-7lrq: signed/decimal proving path; witness-omitted recoverable
    // failure (no panic in a public fn).
    FilterSignedMissingWitness,
    /// A [`ProofInputs::FilterValueDl`] (DUAL-LEAF value lane) was passed to the
    /// general `prover_toml_for`: its private witness is two FIELD elements
    /// (`value_hook` + `lexical_component`), a different shape from the digit-byte
    /// witnesses the general entry threads, so the value-lane Prover.toml is
    /// emitted by the DEDICATED [`filter_value_dl_prover_toml`] instead. This keeps
    /// the general entry's signature unchanged (the value lane is opt-in,
    /// `dual-leaf` feature). [OPUS-4.8] sq-xojl.
    ///
    /// [OPUS-4.8] sq-2ezsx: the same applies to the `xsd:double`
    /// ([`filter_value_dl_f64_prover_toml`]) and `xsd:decimal`
    /// ([`filter_value_dl_decimal_prover_toml`]) sibling members, and
    /// [OPUS-5] sq-wz99x: to the `xsd:dateTime` / `xsd:date` member
    /// ([`filter_value_dl_datetime_prover_toml`]).
    #[cfg(feature = "dual-leaf")]
    FilterValueDlUseDedicatedFn,
    /// A [`ProofInputs::PathReach`] (bounded-depth property path) was passed to the
    /// general `prover_toml_for`: its private witness is the chain-shaped
    /// [`crate::build::PathReachWitness`] (`path_len` + `nodes` + `counts` + `enc`),
    /// a different shape from the scalar/digit witnesses the general entry threads,
    /// so the path Prover.toml is emitted by the DEDICATED
    /// [`path_reach_prover_toml`] instead (obtain the witness from
    /// [`crate::build::build_path_reach`]). This keeps the general entry's
    /// signature unchanged (the path lane is opt-in, `extended-fragment` feature).
    // [OPUS-4.8] sq-3kd2g.6.
    #[cfg(feature = "extended-fragment")]
    PathReachUseDedicatedFn,
}

impl std::fmt::Display for ProverTomlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProverTomlError::JoinEqMissingWitness => write!(
                f,
                "join_eq Prover.toml generation requires the private JoinWitness \
                 (enc_a/counts_a/enc_b/counts_b/row_a/row_b/blinding) — obtain it \
                 from build_join and pass it via prover_toml_for's join_witness arg"
            ),
            ProverTomlError::FilterSignedMissingWitness => write!(
                f,
                "signed-int / decimal FILTER Prover.toml generation requires the \
                 private FilterSignedWitness (sign flag + canonical digits) — obtain \
                 it from build_filter_signed_int / build_filter_decimal and pass it \
                 via prover_toml_for's filter_signed_witness arg"
            ),
            #[cfg(feature = "dual-leaf")]
            ProverTomlError::FilterValueDlUseDedicatedFn => write!(
                f,
                "dual-leaf value-lane FILTER Prover.toml is emitted by the dedicated \
                 filter_value_dl_prover_toml(challenge, operand_enc, op, bound, \
                 datatype_const, expected, value_hook, lexical_component) — its private \
                 witness is two field elements, not digit bytes"
            ),
            #[cfg(feature = "extended-fragment")]
            ProverTomlError::PathReachUseDedicatedFn => write!(
                f,
                "bounded-depth path Prover.toml is emitted by the dedicated \
                 path_reach_prover_toml — its private witness is the chain-shaped \
                 PathReachWitness (path_len/nodes/counts/enc) from build_path_reach, \
                 not the scalar/digit witnesses the general entry threads"
            ),
        }
    }
}

/// Render the `Prover.toml` body for a DUAL-LEAF value-lane FILTER proof
/// (`filter_value_dl_int`, [OPUS-4.8] sq-xojl). Order MUST match
/// `zk/compose/filter_value_dl_int/src/main.nr`:
/// challenge, operand_enc, op, bound, datatype_const, expected (public), then
/// value_hook, lexical_component (private). The two private witnesses are FIELD
/// elements: `value_hook` is the numeric value handle, `lexical_component` is the
/// OFF-circuit blake3 lexical hash carried as a free witness (the member binds it
/// via the leaf, never hashes it — the gate win). DOCUMENTED RISK: this carries
/// the INV-VL downgrade (value↔lexical agreement is trusted-issuer-honesty; #769
/// accepted, CR-G8 / sq-qhy4); NOT externally audited.
#[cfg(feature = "dual-leaf")]
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_hook: &FieldHex,
    lexical_component: &FieldHex,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("operand_enc = \"{}\"\n", operand_enc.0));
    s.push_str(&format!("op = \"{}\"\n", op));
    s.push_str(&format!("bound = \"{}\"\n", bound));
    s.push_str(&format!("datatype_const = \"{}\"\n", datatype_const.0));
    s.push_str(&format!("expected = {}\n", expected));
    s.push_str(&format!("value_hook = \"{}\"\n", value_hook.0));
    s.push_str(&format!("lexical_component = \"{}\"\n", lexical_component.0));
    s
}

/// Render the `Prover.toml` body for a DUAL-LEAF `xsd:boolean` value-lane FILTER
/// proof (sq-5xdlk). It targets the SAME compiled member as the integer lane —
/// `filter_value_dl_int` — because there is NO boolean Noir relation: the boolean
/// hooks `{0 = false, 1 = true}` are inside that member's `u64` domain and the
/// datatype lane is selected purely by the PUBLIC `datatype_const`.
///
/// This renderer therefore differs from [`filter_value_dl_prover_toml`] only in
/// that it PINS `datatype_const` to [`crate::manifest::boolean_datatype_const`]`()`
/// (the caller cannot pass the integer constant by mistake) and takes `bound` as a
/// `bool`, mapped to the hook `{0, 1}`. The emitted field order is byte-identical
/// to the integer lane's, as it must be — it is the same `main`.
///
/// DOCUMENTED RISK: inherits the value lane's INV-VL downgrade (#769 accepted,
/// CR-G8 / sq-qhy4). NOT externally audited; no soundness / privacy claim.
// [OPUS-5] sq-5xdlk: boolean value-lane Prover.toml. Opt-in, NOT-yet-sound.
#[cfg(feature = "dual-leaf")]
pub fn filter_value_dl_boolean_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound: bool,
    expected: bool,
    value_hook: &FieldHex,
    lexical_component: &FieldHex,
) -> String {
    filter_value_dl_prover_toml(
        challenge,
        operand_enc,
        op,
        u64::from(bound),
        &crate::manifest::boolean_datatype_const(),
        expected,
        value_hook,
        lexical_component,
    )
}

/// Render the `Prover.toml` body for a DUAL-LEAF `xsd:double` value-lane FILTER
/// proof (`filter_value_dl_f64`, [OPUS-4.8] sq-2ezsx). Order MUST match
/// `zk/compose/filter_value_dl_f64/src/main.nr`: challenge, operand_enc, op,
/// b_bits, datatype_const, expected (public), then value_hook, lexical_component
/// (private). `value_hook` is the IEEE-754 double bit pattern (as a field);
/// `b_bits` is the FILTER's constant double's bit pattern. The member
/// canonicalises the IEEE bits IN-CIRCUIT (B4), so a `value_hook` for `-0.0` or a
/// NaN binds the same leaf as its canonical form. DOCUMENTED RISK: INV-VL
/// downgrade (CR-G8 / sq-qhy4); NOT externally audited.
#[cfg(feature = "dual-leaf")]
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_f64_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    b_bits: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_hook: &FieldHex,
    lexical_component: &FieldHex,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("operand_enc = \"{}\"\n", operand_enc.0));
    s.push_str(&format!("op = \"{}\"\n", op));
    s.push_str(&format!("b_bits = \"{}\"\n", b_bits));
    s.push_str(&format!("datatype_const = \"{}\"\n", datatype_const.0));
    s.push_str(&format!("expected = {}\n", expected));
    s.push_str(&format!("value_hook = \"{}\"\n", value_hook.0));
    s.push_str(&format!("lexical_component = \"{}\"\n", lexical_component.0));
    s
}

/// Render the `Prover.toml` body for a DUAL-LEAF `xsd:decimal` value-lane FILTER
/// proof (`filter_value_dl_decimal`, [OPUS-4.8] sq-2ezsx). Order MUST match
/// `zk/compose/filter_value_dl_decimal/src/main.nr`: challenge, operand_enc, op,
/// bound_neg, bound_scaled, datatype_const, expected (public), then value_neg,
/// value_hook_scaled, lexical_component (private). The value handle is the SIGNED
/// scaled magnitude at the canonical scale (sign in `value_neg`, magnitude in
/// `value_hook_scaled`); the scale is folded into `datatype_const` (the B4 bind).
/// DOCUMENTED RISK: INV-VL downgrade (CR-G8 / sq-qhy4); NOT externally audited.
#[cfg(feature = "dual-leaf")]
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_decimal_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound_neg: bool,
    bound_scaled: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_neg: bool,
    value_hook_scaled: &FieldHex,
    lexical_component: &FieldHex,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("operand_enc = \"{}\"\n", operand_enc.0));
    s.push_str(&format!("op = \"{}\"\n", op));
    s.push_str(&format!("bound_neg = {}\n", bound_neg));
    s.push_str(&format!("bound_scaled = \"{}\"\n", bound_scaled));
    s.push_str(&format!("datatype_const = \"{}\"\n", datatype_const.0));
    s.push_str(&format!("expected = {}\n", expected));
    s.push_str(&format!("value_neg = {}\n", value_neg));
    s.push_str(&format!("value_hook_scaled = \"{}\"\n", value_hook_scaled.0));
    s.push_str(&format!("lexical_component = \"{}\"\n", lexical_component.0));
    s
}

/// Render the `Prover.toml` body for a DUAL-LEAF `xsd:dateTime` / `xsd:date`
/// value-lane FILTER proof (`filter_value_dl_datetime`, [OPUS-5] sq-wz99x). Order
/// MUST match `zk/compose/filter_value_dl_datetime/src/main.nr`: challenge,
/// operand_enc, op, bound_neg, bound_scaled_epoch, datatype_const, expected
/// (public), then value_neg, value_hook_scaled, lexical_component (private).
///
/// The value handle is the SIGNED SCALED EPOCH (milliseconds on the XSD
/// `timeOnTimeline`; sign in `value_neg`, magnitude in `value_hook_scaled`); the
/// lane AND the scale are folded into `datatype_const` — pass
/// [`crate::manifest::datetime_datatype_const`]`()` for `xsd:dateTime` or
/// [`crate::manifest::date_datatype_const`]`()` for `xsd:date`. ONE member serves
/// both lanes, so this ONE renderer does too. DOCUMENTED RISK: INV-VL downgrade +
/// the open §13 audit obligation (CR-G8 / sq-qhy4); NOT externally audited.
#[cfg(feature = "dual-leaf")]
#[allow(clippy::too_many_arguments)]
pub fn filter_value_dl_datetime_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound_neg: bool,
    bound_scaled_epoch: u64,
    datatype_const: &FieldHex,
    expected: bool,
    value_neg: bool,
    value_hook_scaled: &FieldHex,
    lexical_component: &FieldHex,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("operand_enc = \"{}\"\n", operand_enc.0));
    s.push_str(&format!("op = \"{}\"\n", op));
    s.push_str(&format!("bound_neg = {}\n", bound_neg));
    s.push_str(&format!("bound_scaled_epoch = \"{}\"\n", bound_scaled_epoch));
    s.push_str(&format!("datatype_const = \"{}\"\n", datatype_const.0));
    s.push_str(&format!("expected = {}\n", expected));
    s.push_str(&format!("value_neg = {}\n", value_neg));
    s.push_str(&format!("value_hook_scaled = \"{}\"\n", value_hook_scaled.0));
    s.push_str(&format!("lexical_component = \"{}\"\n", lexical_component.0));
    s
}

impl std::error::Error for ProverTomlError {}

/// Render the `Prover.toml` body for a scan proof.
///
/// Order MUST match `scan_k{k}_n{n}_r{r}/src/main.nr`:
/// challenge, commitments, pattern_is_const, pattern_const_enc, rows,
/// row_count, attribution, counts, enc.
#[allow(clippy::too_many_arguments)]
pub fn scan_prover_toml(
    challenge: &FieldHex,
    commitments: &[FieldHex],
    pattern_is_const: &[bool; 3],
    pattern_const_enc: &[FieldHex; 3],
    rows: &[[FieldHex; 3]],
    row_count: u32,
    attribution: &[bool],
    counts: &[u32],
    enc: &[Vec<[FieldHex; 3]>],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("commitments = {}\n", hex_array(commitments)));
    s.push_str(&format!(
        "pattern_is_const = [{}]\n",
        pattern_is_const
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s.push_str(&format!(
        "pattern_const_enc = {}\n",
        hex_array(pattern_const_enc)
    ));
    s.push_str(&format!("rows = {}\n", rows_array(rows)));
    s.push_str(&format!("row_count = \"{row_count}\"\n"));
    // attribution: [bool; K] (audit #8) -- declared right after row_count.
    s.push_str(&format!(
        "attribution = [{}]\n",
        attribution
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s.push_str(&format!(
        "counts = [{}]\n",
        counts
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    // enc: [[[Field;3];N];K]
    s.push_str("enc = [");
    for (gi, graph) in enc.iter().enumerate() {
        if gi > 0 {
            s.push_str(", ");
        }
        s.push_str(&rows_array(graph));
    }
    s.push_str("]\n");
    s
}

/// Render the `Prover.toml` body for a filter_int proof. Order MUST match
/// `filter_int_d{d}/src/main.nr`: challenge, operand_enc, op, bound, expected,
/// digits.
pub fn filter_int_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound: u64,
    expected: bool,
    digits: &[u8],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("operand_enc = \"{}\"\n", operand_enc.0));
    s.push_str(&format!("op = \"{op}\"\n"));
    s.push_str(&format!("bound = \"{bound}\"\n"));
    s.push_str(&format!("expected = {expected}\n"));
    s.push_str(&format!(
        "digits = [{}]\n",
        digits
            .iter()
            .map(|d| format!("\"{d}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s
}

/// Render the `Prover.toml` body for a MANIFEST-COMPOSABLE filter_f64 proof
/// ([OPUS-4.8] sq-q7e / sq-tat). Order MUST match `filter_f64_d{d}/src/main.nr`:
/// challenge, operand_enc, op, b_bits, expected, digits.
pub fn filter_f64_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    b_bits: u64,
    expected: bool,
    digits: &[u8],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("operand_enc = \"{}\"\n", operand_enc.0));
    s.push_str(&format!("op = \"{op}\"\n"));
    s.push_str(&format!("b_bits = \"{b_bits}\"\n"));
    s.push_str(&format!("expected = {expected}\n"));
    s.push_str(&format!(
        "digits = [{}]\n",
        digits
            .iter()
            .map(|d| format!("\"{d}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s
}

/// Render a `[ "d0", "d1", … ]` inline array of decimal digit bytes (each byte
/// rendered as its ASCII codepoint string, matching the circuit's `[u8; N]`).
fn digits_array(digits: &[u8]) -> String {
    format!(
        "[{}]",
        digits
            .iter()
            .map(|d| format!("\"{d}\""))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Render the `Prover.toml` body for a MANIFEST-COMPOSABLE `filter_signed_int`
/// proof ([OPUS-4.8] sq-7lrq). Order MUST match
/// `filter_signed_int_d{md}/src/main.nr`: PUBLIC `challenge, operand_enc, op,
/// bound_neg, bound, expected` then PRIVATE `neg, mag_digits`. `neg` is the hidden
/// operand's sign flag; `mag_digits` are its canonical MAGNITUDE digits (length
/// MD).
#[allow(clippy::too_many_arguments)]
pub fn filter_signed_int_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound_neg: bool,
    bound: u64,
    expected: bool,
    neg: bool,
    mag_digits: &[u8],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("operand_enc = \"{}\"\n", operand_enc.0));
    s.push_str(&format!("op = \"{op}\"\n"));
    s.push_str(&format!("bound_neg = {bound_neg}\n"));
    s.push_str(&format!("bound = \"{bound}\"\n"));
    s.push_str(&format!("expected = {expected}\n"));
    s.push_str(&format!("neg = {neg}\n"));
    s.push_str(&format!("mag_digits = {}\n", digits_array(mag_digits)));
    s
}

/// Render the `Prover.toml` body for a MANIFEST-COMPOSABLE `filter_decimal` proof
/// ([OPUS-4.8] sq-7lrq). Order MUST match `filter_decimal_i{id}_f{fd}/src/main.nr`:
/// PUBLIC `challenge, operand_enc, op, bound_neg, bound_scaled, expected` then
/// PRIVATE `neg, int_digits, frac_digits`. `neg` is the hidden operand's sign flag;
/// `int_digits` (length ID) / `frac_digits` (length FD) are its canonical
/// integer-part / fraction digits.
#[allow(clippy::too_many_arguments)]
pub fn filter_decimal_prover_toml(
    challenge: &FieldHex,
    operand_enc: &FieldHex,
    op: u32,
    bound_neg: bool,
    bound_scaled: u64,
    expected: bool,
    neg: bool,
    int_digits: &[u8],
    frac_digits: &[u8],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("operand_enc = \"{}\"\n", operand_enc.0));
    s.push_str(&format!("op = \"{op}\"\n"));
    s.push_str(&format!("bound_neg = {bound_neg}\n"));
    s.push_str(&format!("bound_scaled = \"{bound_scaled}\"\n"));
    s.push_str(&format!("expected = {expected}\n"));
    s.push_str(&format!("neg = {neg}\n"));
    s.push_str(&format!("int_digits = {}\n", digits_array(int_digits)));
    s.push_str(&format!("frac_digits = {}\n", digits_array(frac_digits)));
    s
}

/// Render the `Prover.toml` body for a hidden cross-credential `join_eq` proof
/// ([OPUS-4.8] sq-r2s8). Order MUST match `join_eq_na{n_a}_nb{n_b}/src/main.nr`:
/// PUBLIC `challenge, commit_a, commit_b, join_commitment, slot_a, slot_b` then
/// PRIVATE `enc_a, counts_a, enc_b, counts_b, row_a, row_b, blinding`. `enc_a`/
/// `enc_b` are padded to `n_a`/`n_b` slots by the caller (mirrors the scan arm).
#[allow(clippy::too_many_arguments)]
pub fn join_prover_toml(
    challenge: &FieldHex,
    commit_a: &FieldHex,
    commit_b: &FieldHex,
    join_commitment: &FieldHex,
    slot_a: u32,
    slot_b: u32,
    enc_a: &[[FieldHex; 3]],
    counts_a: u32,
    enc_b: &[[FieldHex; 3]],
    counts_b: u32,
    row_a: &[FieldHex; 3],
    row_b: &[FieldHex; 3],
    blinding: &FieldHex,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("commit_a = \"{}\"\n", commit_a.0));
    s.push_str(&format!("commit_b = \"{}\"\n", commit_b.0));
    s.push_str(&format!("join_commitment = \"{}\"\n", join_commitment.0));
    s.push_str(&format!("slot_a = \"{slot_a}\"\n"));
    s.push_str(&format!("slot_b = \"{slot_b}\"\n"));
    s.push_str(&format!("enc_a = {}\n", rows_array(enc_a)));
    s.push_str(&format!("counts_a = \"{counts_a}\"\n"));
    s.push_str(&format!("enc_b = {}\n", rows_array(enc_b)));
    s.push_str(&format!("counts_b = \"{counts_b}\"\n"));
    s.push_str(&format!(
        "row_a = [\"{}\", \"{}\", \"{}\"]\n",
        row_a[0].0, row_a[1].0, row_a[2].0
    ));
    s.push_str(&format!(
        "row_b = [\"{}\", \"{}\", \"{}\"]\n",
        row_b[0].0, row_b[1].0, row_b[2].0
    ));
    s.push_str(&format!("blinding = \"{}\"\n", blinding.0));
    s
}

/// Render the `Prover.toml` body for a bounded-depth `path_reach` proof
/// (sq-3kd2g.6). Order MUST match `path_reach_d{d}_k{k}_n{n}/src/main.nr`:
/// PUBLIC `challenge, commitments, pred_enc, src_enc, dst_enc, allow_zero,
/// depth_bound, attribution` then PRIVATE `path_len, nodes, counts, enc`. `nodes`
/// is padded to `d` and each graph's `enc` to `n` by the caller (mirrors the scan
/// arm's padding of `enc` to `n`).
// [OPUS-4.8] sq-3kd2g.6.
#[cfg(feature = "extended-fragment")]
#[allow(clippy::too_many_arguments)]
pub fn path_reach_prover_toml(
    challenge: &FieldHex,
    commitments: &[FieldHex],
    pred_enc: &FieldHex,
    src_enc: &FieldHex,
    dst_enc: &FieldHex,
    allow_zero: bool,
    depth_bound: u32,
    attribution: &[bool],
    path_len: u32,
    nodes: &[FieldHex],
    counts: &[u32],
    enc: &[Vec<[FieldHex; 3]>],
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", challenge.0));
    s.push_str(&format!("commitments = {}\n", hex_array(commitments)));
    s.push_str(&format!("pred_enc = \"{}\"\n", pred_enc.0));
    s.push_str(&format!("src_enc = \"{}\"\n", src_enc.0));
    s.push_str(&format!("dst_enc = \"{}\"\n", dst_enc.0));
    s.push_str(&format!("allow_zero = {allow_zero}\n"));
    s.push_str(&format!("depth_bound = \"{depth_bound}\"\n"));
    s.push_str(&format!(
        "attribution = [{}]\n",
        attribution
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s.push_str(&format!("path_len = \"{path_len}\"\n"));
    s.push_str(&format!("nodes = {}\n", hex_array(nodes)));
    s.push_str(&format!(
        "counts = [{}]\n",
        counts
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    // enc: [[[Field;3];N];K]
    s.push_str("enc = [");
    for (gi, graph) in enc.iter().enumerate() {
        if gi > 0 {
            s.push_str(", ");
        }
        s.push_str(&rows_array(graph));
    }
    s.push_str("]\n");
    s
}

fn hex_array(items: &[FieldHex]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|h| format!("\"{}\"", h.0))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn rows_array(rows: &[[FieldHex; 3]]) -> String {
    format!(
        "[{}]",
        rows.iter()
            .map(|r| format!(
                "[\"{}\", \"{}\", \"{}\"]",
                r[0].0, r[1].0, r[2].0
            ))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Pad a hex-encoding list to `len` with zero field elements (the circuit's
/// inactive padding slots).
pub fn pad_hex(mut v: Vec<FieldHex>, len: usize) -> Vec<FieldHex> {
    while v.len() < len {
        v.push(FieldHex("0x0".to_string()));
    }
    v
}

/// Pad a list of rows to `len` with zero rows.
pub fn pad_rows(mut v: Vec<[FieldHex; 3]>, len: usize) -> Vec<[FieldHex; 3]> {
    let zero = || FieldHex("0x0".to_string());
    while v.len() < len {
        v.push([zero(), zero(), zero()]);
    }
    v
}

/// Sanity helper: digit bytes of a non-negative integer's canonical decimal.
pub fn canonical_digits(value: u64) -> Vec<u8> {
    let s = value.to_string();
    s.bytes().collect()
}

/// Render the witness-bearing `Prover.toml` for any `ProofInputs`, given the
/// private witnesses the manifest does not carry. Returns the package id too.
///
/// For [`ProofInputs::JoinEq`] the private [`JoinWitness`] MUST be supplied via
/// `join_witness` (obtained from [`crate::build::build_join`]); omitting it returns
/// [`ProverTomlError::JoinEqMissingWitness`] — a recoverable error, never a panic
/// (a public fn must not crash a downstream caller). The `join_witness` argument is
/// ignored for every non-join input, exactly as `scan_*` is ignored for filters.
/// [OPUS-4.8] sq-r2s8: the join_eq proving path is now implemented.
///
/// For [`ProofInputs::FilterSignedInt`] / [`ProofInputs::FilterDecimal`] the private
/// [`FilterSignedWitness`] (operand sign + canonical digits) MUST be supplied via
/// `filter_signed_witness`; omitting it returns
/// [`ProverTomlError::FilterSignedMissingWitness`] (recoverable, never a panic — the
/// `filter_digits` arg does NOT carry the sign these members need). The
/// `filter_signed_witness` argument is ignored for every other input.
/// [OPUS-4.8] sq-7lrq: the signed-int / decimal proving path is now implemented.
#[allow(clippy::too_many_arguments)]
pub fn prover_toml_for(
    inputs: &ProofInputs,
    challenge: &FieldHex,
    // scan witnesses (ignored for filter): per-graph active triple-counts and
    // per-graph per-slot encodings.
    scan_counts: &[u32],
    scan_enc: &[Vec<[FieldHex; 3]>],
    // filter witness (ignored for scan): canonical decimal digits (filter_int /
    // filter_f64). Signed-int / decimal carry their digits in `filter_signed_witness`.
    filter_digits: &[u8],
    // join witness (ignored for scan/filter): the join_eq member's private inputs
    // (enc_a/counts_a/enc_b/counts_b/row_a/row_b/blinding). [OPUS-4.8] sq-r2s8.
    join_witness: Option<&JoinWitness>,
    // signed-int / decimal witness (ignored for every other input): the operand's
    // PRIVATE sign flag + canonical digits. [OPUS-4.8] sq-7lrq.
    filter_signed_witness: Option<&FilterSignedWitness>,
) -> Result<(CircuitId, String), ProverTomlError> {
    let out = match inputs {
        ProofInputs::Scan {
            id,
            commitments,
            pattern_is_const,
            pattern_const_enc,
            rows,
            row_count,
            attribution,
        } => {
            let CircuitId::Scan { n, r, .. } = id else {
                unreachable!("scan inputs carry a scan id")
            };
            let rows = pad_rows(rows.clone(), *r as usize);
            // Pad each graph's enc to N slots.
            let enc: Vec<Vec<[FieldHex; 3]>> = scan_enc
                .iter()
                .map(|g| pad_rows(g.clone(), *n as usize))
                .collect();
            let toml = scan_prover_toml(
                challenge,
                commitments,
                pattern_is_const,
                pattern_const_enc,
                &rows,
                *row_count,
                attribution,
                scan_counts,
                &enc,
            );
            (id.clone(), toml)
        }
        ProofInputs::FilterInt {
            id,
            operand_enc,
            op,
            bound,
            expected,
        } => {
            let toml = filter_int_prover_toml(
                challenge,
                operand_enc,
                op.code(),
                *bound,
                *expected,
                filter_digits,
            );
            (id.clone(), toml)
        }
        // [OPUS-4.8] sq-q7e + sq-tat: composable xsd:double FILTER. `filter_digits`
        // carries the integer-valued double's canonical decimal digits (same role
        // as the filter_int digit witness); `b_bits` is the constant double's IEEE
        // bit pattern.
        ProofInputs::FilterF64 {
            id,
            operand_enc,
            op,
            b_bits,
            expected,
        } => {
            let toml = filter_f64_prover_toml(
                challenge,
                operand_enc,
                op.code(),
                *b_bits,
                *expected,
                filter_digits,
            );
            (id.clone(), toml)
        }
        // [OPUS-4.8] sq-7lrq: composable SIGNED xsd:integer FILTER. The operand's
        // PRIVATE sign flag + magnitude digits come from `filter_signed_witness`
        // (built by `build_filter_signed_int`); omitting it is a recoverable `Err`
        // (no panic in a public fn). `frac_digits` is empty for signed-int.
        ProofInputs::FilterSignedInt {
            id,
            operand_enc,
            op,
            bound_neg,
            bound,
            expected,
        } => {
            let w = filter_signed_witness.ok_or(ProverTomlError::FilterSignedMissingWitness)?;
            let toml = filter_signed_int_prover_toml(
                challenge,
                operand_enc,
                op.code(),
                *bound_neg,
                *bound,
                *expected,
                w.neg,
                &w.int_digits,
            );
            (id.clone(), toml)
        }
        // [OPUS-4.8] sq-7lrq: composable xsd:decimal FILTER. The operand's PRIVATE
        // sign flag + integer-part / fraction digits come from `filter_signed_witness`
        // (built by `build_filter_decimal`); `bound_scaled` is the host-prescaled
        // constant magnitude. Omitting the witness is a recoverable `Err`.
        ProofInputs::FilterDecimal {
            id,
            operand_enc,
            op,
            bound_neg,
            bound_scaled,
            expected,
        } => {
            let w = filter_signed_witness.ok_or(ProverTomlError::FilterSignedMissingWitness)?;
            let toml = filter_decimal_prover_toml(
                challenge,
                operand_enc,
                op.code(),
                *bound_neg,
                *bound_scaled,
                *expected,
                w.neg,
                &w.int_digits,
                &w.frac_digits,
            );
            (id.clone(), toml)
        }
        // [OPUS-4.8] sq-xojl: DUAL-LEAF value-lane FILTER. Its private witness is two
        // FIELD elements (value_hook + lexical_component), a different shape from the
        // digit-byte witnesses this general entry threads, so the value-lane
        // Prover.toml is emitted by the dedicated `filter_value_dl_prover_toml` (which
        // keeps this signature unchanged for the opt-in `dual-leaf` feature). Surface
        // a recoverable error here, never a panic.
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDl { .. } => {
            return Err(ProverTomlError::FilterValueDlUseDedicatedFn);
        }
        // [OPUS-4.8] sq-2ezsx: the double + decimal value-lane siblings also carry
        // FIELD-element private witnesses, so they use the dedicated fns
        // (`filter_value_dl_f64_prover_toml` / `filter_value_dl_decimal_prover_toml`).
        // [OPUS-5] sq-wz99x: same for the dateTime/date member
        // (`filter_value_dl_datetime_prover_toml`).
        #[cfg(feature = "dual-leaf")]
        ProofInputs::FilterValueDlF64 { .. }
        | ProofInputs::FilterValueDlDecimal { .. }
        | ProofInputs::FilterValueDlDateTime { .. } => {
            return Err(ProverTomlError::FilterValueDlUseDedicatedFn);
        }
        // [OPUS-4.8] sq-bwwl / sq-r2s8 (step 4 proving path): hidden cross-credential
        // JOIN. The public inputs (commit_a/commit_b/join_commitment/slot_a/slot_b)
        // come from `inputs`; the PRIVATE witnesses
        // (enc_a/counts_a/enc_b/counts_b/row_a/row_b/blinding) come from the
        // `join_witness` the caller built with `build_join`. Omitting it is a
        // recoverable `Err` (no panic in a public fn). `enc_a`/`enc_b` are padded to
        // the member's `n_a`/`n_b` buckets, exactly as the scan arm pads `enc`.
        ProofInputs::JoinEq {
            id,
            commit_a,
            commit_b,
            join_commitment,
            slot_a,
            slot_b,
        } => {
            let CircuitId::JoinEq { n_a, n_b } = id else {
                unreachable!("join_eq inputs carry a join_eq id")
            };
            let w = join_witness.ok_or(ProverTomlError::JoinEqMissingWitness)?;
            let enc_a = pad_rows(w.enc_a.clone(), *n_a as usize);
            let enc_b = pad_rows(w.enc_b.clone(), *n_b as usize);
            let toml = join_prover_toml(
                challenge,
                commit_a,
                commit_b,
                join_commitment,
                *slot_a,
                *slot_b,
                &enc_a,
                w.counts_a,
                &enc_b,
                w.counts_b,
                &w.row_a,
                &w.row_b,
                &w.blinding,
            );
            (id.clone(), toml)
        }
        // [OPUS-4.8] sq-3kd2g.6: bounded-depth path. Its private witness is the
        // chain-shaped `PathReachWitness` (path_len/nodes/counts/enc), a different
        // shape from the scalar/digit witnesses this general entry threads, so the
        // path Prover.toml is emitted by the dedicated `path_reach_prover_toml`
        // (fed the `build_path_reach` witness). Surface a recoverable error here,
        // never a panic — mirrors the dual-leaf value-lane arm.
        #[cfg(feature = "extended-fragment")]
        ProofInputs::PathReach { .. } => {
            return Err(ProverTomlError::PathReachUseDedicatedFn);
        }
    };
    Ok(out)
}

// [OPUS-4.8] sq-bif.6: GLUE unit tests for the witness `Prover.toml` SERIALIZATION
// — the `FieldHex` hex round-trip, the array-vs-scalar rendering shape, the
// declaration-order field layout each `*_prover_toml` emits, and the recoverable
// missing-witness error arms of the public `prover_toml_for`. These cover the
// NON-cryptographic serialization plumbing ONLY; no soundness / privacy property is
// asserted (the circuit family is NOT-yet-sound, sq-qhy4). The real prove/verify
// e2e lives in `tests/e2e.rs`, gated on the nargo/bb toolchain.
#[cfg(test)]
mod toml_glue_tests {
    use super::*;
    use crate::build::FilterSignedWitness;
    use crate::manifest::FilterOp;
    use sparq_zk::field::Fr;

    fn fh(s: &str) -> FieldHex {
        FieldHex(s.to_string())
    }

    // --- FieldHex hex round-trip + rejection -----------------------------

    /// `FieldHex` round-trips through the field: `from_field(to_field) == self`
    /// (canonicalised), and a value re-rendered is byte-stable. This is the
    /// representation every `*_prover_toml` writes verbatim, so its round-trip is
    /// load-bearing for the witness contract.
    #[test]
    fn field_hex_round_trips_through_the_field() {
        let f = Fr::from(0x1a2bu64);
        let hex = FieldHex::from_field(&f);
        // The canonical rendering is 0x-prefixed 64-nibble hex.
        assert!(hex.0.starts_with("0x") && hex.0.len() == 66, "canonical 0x + 64 nibbles");
        // Parse back to the same field element.
        assert_eq!(hex.to_field(), Some(f), "to_field inverts from_field");
        // Re-rendering the parsed field is byte-identical (idempotent canonical form).
        assert_eq!(FieldHex::from_field(&hex.to_field().unwrap()), hex);
        // A short, non-canonical literal parses to the SAME field element (the
        // circuit reduces 0x-hex), so the toml may carry either form.
        assert_eq!(fh("0x1a2b").to_field(), Some(f));
    }

    /// Malformed hex returns `None` from `to_field` — the parse never panics, so a
    /// hand-edited / corrupt manifest surfaces a recoverable error.
    #[test]
    fn field_hex_rejects_malformed_input() {
        assert_eq!(fh("0xzz").to_field(), None, "non-hex digits rejected");
        assert_eq!(fh("").to_field(), None, "empty rejected");
        assert_eq!(fh(&format!("0x{}", "f".repeat(65))).to_field(), None, "over-long rejected");
    }

    // --- scalar vs array rendering shape ---------------------------------

    /// `filter_int_prover_toml` renders SCALARS as quoted Field literals and the
    /// digit witness as an inline ARRAY of quoted bytes, in the declaration order
    /// the `filter_int_d{d}` member's `main` expects.
    #[test]
    fn filter_int_toml_scalar_and_array_shape_and_order() {
        let toml = filter_int_prover_toml(
            &fh("0x1"),       // challenge
            &fh("0xabc"),     // operand_enc
            FilterOp::Gt.code(),
            42,               // bound
            true,             // expected
            b"123",
        );
        // Scalars: quoted Field literals (nargo's toml reader accepts 0x-prefixed).
        assert!(toml.contains("challenge = \"0x1\"\n"));
        assert!(toml.contains("operand_enc = \"0xabc\"\n"));
        assert!(toml.contains("op = \"2\"\n"), "FilterOp::Gt.code() == 2, rendered as a string");
        assert!(toml.contains("bound = \"42\"\n"), "u64 bound is a quoted Field");
        // expected is a BARE bool (not quoted) — the circuit's `pub bool`.
        assert!(toml.contains("expected = true\n"));
        // digits: an inline array of quoted ASCII codepoints, one per decimal digit.
        assert!(toml.contains("digits = [\"49\", \"50\", \"51\"]\n"), "b'1'=49, b'2'=50, b'3'=51");
        // Declaration order: challenge < operand_enc < op < bound < expected < digits.
        let order: Vec<&str> = ["challenge", "operand_enc", "op", "bound", "expected", "digits"]
            .iter()
            .map(|k| {
                let pat = format!("{} =", k);
                assert!(toml.contains(&pat), "field `{}` present", k);
                *k
            })
            .collect();
        let positions: Vec<usize> = order
            .iter()
            .map(|k| toml.find(&format!("{} =", k)).unwrap())
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "fields in declaration order");
    }

    /// `scan_prover_toml` renders the nested `enc` as `[[[..];3];N];K]` and emits
    /// the bool vectors (`pattern_is_const`, `attribution`) as BARE-bool arrays —
    /// the array-shape contract the scan member's `main` consumes.
    #[test]
    fn scan_toml_nested_array_and_bool_array_shape() {
        let commitments = [fh("0xc0"), fh("0xc1")];
        let pattern_is_const = [true, false, true];
        let pattern_const_enc = [fh("0xs"), fh("0x0"), fh("0xo")];
        let rows = [[fh("0x1"), fh("0x2"), fh("0x3")]];
        let attribution = [true, false];
        let counts = [2u32, 1u32];
        let g0 = vec![[fh("0xa"), fh("0xb"), fh("0xc")], [fh("0xd"), fh("0xe"), fh("0xf")]];
        let g1 = vec![[fh("0x4"), fh("0x5"), fh("0x6")]];
        let enc = [g0, g1];
        let toml = scan_prover_toml(
            &fh("0x9"),
            &commitments,
            &pattern_is_const,
            &pattern_const_enc,
            &rows,
            1,
            &attribution,
            &counts,
            &enc,
        );
        // commitments: flat array of quoted Fields.
        assert!(toml.contains("commitments = [\"0xc0\", \"0xc1\"]\n"));
        // pattern_is_const + attribution: BARE bools (not quoted).
        assert!(toml.contains("pattern_is_const = [true, false, true]\n"));
        assert!(toml.contains("attribution = [true, false]\n"));
        // row_count + counts: quoted Field-ish.
        assert!(toml.contains("row_count = \"1\"\n"));
        assert!(toml.contains("counts = [\"2\", \"1\"]\n"));
        // rows: [[Field;3]] — one inner triple here.
        assert!(toml.contains("rows = [[\"0x1\", \"0x2\", \"0x3\"]]\n"));
        // enc: [[[Field;3];N];K] — two graphs, the first with two triples.
        assert!(
            toml.contains(
                "enc = [[[\"0xa\", \"0xb\", \"0xc\"], [\"0xd\", \"0xe\", \"0xf\"]], \
                 [[\"0x4\", \"0x5\", \"0x6\"]]]\n"
            ),
            "nested K-by-N-by-3 array; toml was:\n{}",
            toml
        );
    }

    /// `filter_signed_int_prover_toml` and `filter_decimal_prover_toml` render the
    /// PRIVATE `neg` flag as a bare bool and the magnitude / int / frac digit arrays
    /// in the member's declaration order (the sq-7lrq members).
    #[test]
    fn signed_and_decimal_toml_shape_and_order() {
        let signed = filter_signed_int_prover_toml(
            &fh("0x1"),
            &fh("0xop"),
            FilterOp::Lt.code(),
            true,     // bound_neg
            7,        // bound magnitude
            false,    // expected
            true,     // neg (operand)
            b"42",
        );
        assert!(signed.contains("bound_neg = true\n"), "bound sign is a bare bool");
        assert!(signed.contains("bound = \"7\"\n"));
        assert!(signed.contains("neg = true\n"), "operand sign is a bare bool");
        assert!(signed.contains("mag_digits = [\"52\", \"50\"]\n"), "b'4'=52, b'2'=50");

        let decimal = filter_decimal_prover_toml(
            &fh("0x1"),
            &fh("0xop"),
            FilterOp::Ge.code(),
            false,    // bound_neg
            12345,    // bound_scaled
            true,     // expected
            false,    // neg
            b"123",
            b"45",
        );
        assert!(decimal.contains("bound_scaled = \"12345\"\n"));
        assert!(decimal.contains("int_digits = [\"49\", \"50\", \"51\"]\n"));
        assert!(decimal.contains("frac_digits = [\"52\", \"53\"]\n"));
    }

    // --- pad / canonical-digits helpers ----------------------------------

    /// `pad_hex` / `pad_rows` extend to the member's bucket length with ZERO field
    /// elements (the circuit's inactive padding slots) and never truncate.
    #[test]
    fn pad_helpers_extend_with_zero_and_never_truncate() {
        let padded = pad_hex(vec![fh("0x1")], 3);
        assert_eq!(padded.len(), 3);
        assert_eq!(padded[0], fh("0x1"));
        assert_eq!(padded[1], FieldHex("0x0".to_string()), "pad slot is the zero field element");
        // Already-long input is left intact (no truncation).
        let same = pad_hex(vec![fh("0xa"), fh("0xb")], 1);
        assert_eq!(same.len(), 2, "pad_hex never shortens");

        let rows = pad_rows(vec![[fh("0x1"), fh("0x2"), fh("0x3")]], 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], [FieldHex("0x0".into()), FieldHex("0x0".into()), FieldHex("0x0".into())]);
    }

    /// `canonical_digits` is the ASCII-decimal byte witness the digit arrays carry.
    #[test]
    fn canonical_digits_are_ascii_decimal_bytes() {
        assert_eq!(canonical_digits(0), b"0");
        assert_eq!(canonical_digits(1234), b"1234");
        assert_eq!(canonical_digits(u64::MAX), u64::MAX.to_string().as_bytes());
    }

    // --- prover_toml_for: dispatch + recoverable missing-witness errors --

    /// `prover_toml_for` on a `FilterInt` input returns the member id + a toml that
    /// matches the direct `filter_int_prover_toml` rendering — the public dispatcher
    /// agrees with the per-member renderer.
    #[test]
    fn prover_toml_for_filter_int_dispatches_to_member_renderer() {
        let inputs = ProofInputs::FilterInt {
            id: CircuitId::FilterInt { d: 2 },
            operand_enc: fh("0xab"),
            op: FilterOp::Eq,
            bound: 50,
            expected: true,
        };
        let (id, toml) =
            prover_toml_for(&inputs, &fh("0x1"), &[], &[], b"50", None, None)
                .expect("filter_int needs no extra witness");
        assert_eq!(id, CircuitId::FilterInt { d: 2 });
        let direct = filter_int_prover_toml(&fh("0x1"), &fh("0xab"), FilterOp::Eq.code(), 50, true, b"50");
        assert_eq!(toml, direct, "dispatcher output matches the member renderer");
    }

    /// A `JoinEq` input WITHOUT its private `JoinWitness` returns the recoverable
    /// `JoinEqMissingWitness` error — NOT a panic in this public fn (the witness
    /// holds the join's private inputs the manifest never carries).
    #[test]
    fn prover_toml_for_join_eq_without_witness_is_recoverable_error() {
        let inputs = ProofInputs::JoinEq {
            id: CircuitId::JoinEq { n_a: 16, n_b: 16 },
            commit_a: fh("0x0a"),
            commit_b: fh("0x0b"),
            join_commitment: fh("0x0c"),
            slot_a: 0,
            slot_b: 2,
        };
        let err = prover_toml_for(&inputs, &fh("0x1"), &[], &[], &[], None, None)
            .expect_err("join_eq without its witness must be an Err, never a panic");
        assert_eq!(err, ProverTomlError::JoinEqMissingWitness);
        // And the message points the caller at build_join / the join_witness arg.
        assert!(err.to_string().contains("build_join"), "the error message is actionable");
    }

    /// A `FilterSignedInt` / `FilterDecimal` input WITHOUT its `FilterSignedWitness`
    /// returns the recoverable `FilterSignedMissingWitness` error (the sign flag +
    /// canonical digits live in the witness, not the manifest).
    #[test]
    fn prover_toml_for_signed_without_witness_is_recoverable_error() {
        let inputs = ProofInputs::FilterSignedInt {
            id: CircuitId::FilterSignedInt { md: 2 },
            operand_enc: fh("0xop"),
            op: FilterOp::Lt,
            bound_neg: false,
            bound: 9,
            expected: true,
        };
        let err = prover_toml_for(&inputs, &fh("0x1"), &[], &[], &[], None, None)
            .expect_err("signed-int without its witness must be an Err");
        assert_eq!(err, ProverTomlError::FilterSignedMissingWitness);

        // Supplying the witness renders successfully and carries the operand sign.
        let w = FilterSignedWitness { neg: true, int_digits: vec![b'4', b'2'], frac_digits: vec![] };
        let (id, toml) = prover_toml_for(&inputs, &fh("0x1"), &[], &[], &[], None, Some(&w))
            .expect("with witness it renders");
        assert_eq!(id, CircuitId::FilterSignedInt { md: 2 });
        assert!(toml.contains("neg = true\n") && toml.contains("mag_digits = [\"52\", \"50\"]\n"));
    }

    /// The two `ProverTomlError` variants render distinct, actionable messages and
    /// compare by value (the public error is `PartialEq`).
    #[test]
    fn prover_toml_error_variants_are_distinct() {
        let a = ProverTomlError::JoinEqMissingWitness;
        let b = ProverTomlError::FilterSignedMissingWitness;
        assert_ne!(a, b);
        assert_ne!(a.to_string(), b.to_string());
        assert!(a.to_string().contains("join_eq"));
        assert!(b.to_string().contains("FilterSignedWitness"));
    }

    // [OPUS-4.8] sq-2ezsx: the double + decimal dedicated Prover.toml renderers
    // emit the public + private fields in the EXACT `main` declaration order of
    // their member, with the FIELD-element private witnesses. These cover the
    // NON-cryptographic serialization plumbing only (NOT-yet-sound, sq-qhy4).

    #[cfg(feature = "dual-leaf")]
    #[test]
    fn filter_value_dl_f64_toml_shape_and_order() {
        let toml = filter_value_dl_f64_prover_toml(
            &fh("0x01"), // challenge
            &fh("0x02"), // operand_enc
            3,           // op (ge)
            0x4008000000000000, // b_bits (3.0)
            &fh("0x03"), // datatype_const
            true,        // expected
            &fh("0x04"), // value_hook (IEEE bits)
            &fh("0x05"), // lexical_component
        );
        let lines: Vec<&str> = toml.lines().collect();
        // Declaration order: challenge, operand_enc, op, b_bits, datatype_const,
        // expected, then the two private field witnesses.
        assert!(lines[0].starts_with("challenge = "));
        assert!(lines[1].starts_with("operand_enc = "));
        assert!(lines[2].starts_with("op = "));
        assert!(lines[3].starts_with("b_bits = "));
        assert!(lines[4].starts_with("datatype_const = "));
        assert!(lines[5] == "expected = true");
        assert!(lines[6].starts_with("value_hook = "));
        assert!(lines[7].starts_with("lexical_component = "));
    }

    #[cfg(feature = "dual-leaf")]
    #[test]
    fn filter_value_dl_decimal_toml_shape_and_order() {
        let toml = filter_value_dl_decimal_prover_toml(
            &fh("0x01"), // challenge
            &fh("0x02"), // operand_enc
            3,           // op (ge)
            false,       // bound_neg
            10000,       // bound_scaled (100.00 at fd=2)
            &fh("0x03"), // datatype_const
            true,        // expected
            true,        // value_neg
            &fh("0x04"), // value_hook_scaled
            &fh("0x05"), // lexical_component
        );
        let lines: Vec<&str> = toml.lines().collect();
        // Declaration order: challenge, operand_enc, op, bound_neg, bound_scaled,
        // datatype_const, expected, then the three private witnesses.
        assert!(lines[0].starts_with("challenge = "));
        assert!(lines[1].starts_with("operand_enc = "));
        assert!(lines[2].starts_with("op = "));
        assert!(lines[3] == "bound_neg = false");
        assert!(lines[4].starts_with("bound_scaled = "));
        assert!(lines[5].starts_with("datatype_const = "));
        assert!(lines[6] == "expected = true");
        assert!(lines[7] == "value_neg = true");
        assert!(lines[8].starts_with("value_hook_scaled = "));
        assert!(lines[9].starts_with("lexical_component = "));
    }

    /// [OPUS-5] sq-wz99x: the dateTime/date renderer emits the member `main`'s
    /// declaration order — the decimal layout with `bound_scaled` renamed
    /// `bound_scaled_epoch`. A reorder silently desyncs the proof from the
    /// verifier's public-input reconstruction, so pin it here too.
    #[cfg(feature = "dual-leaf")]
    #[test]
    fn filter_value_dl_datetime_toml_shape_and_order() {
        let toml = filter_value_dl_datetime_prover_toml(
            &fh("0x01"),  // challenge
            &fh("0x02"),  // operand_enc
            0,            // op (lt)
            false,        // bound_neg
            86_400_000,   // bound_scaled_epoch (1970-01-02T00:00:00Z)
            &fh("0x03"),  // datatype_const (the LANE constant)
            true,         // expected
            true,         // value_neg (a pre-epoch instant)
            &fh("0x04"),  // value_hook_scaled
            &fh("0x05"),  // lexical_component
        );
        let lines: Vec<&str> = toml.lines().collect();
        assert!(lines[0].starts_with("challenge = "));
        assert!(lines[1].starts_with("operand_enc = "));
        assert!(lines[2].starts_with("op = "));
        assert!(lines[3] == "bound_neg = false");
        assert!(lines[4].starts_with("bound_scaled_epoch = "));
        assert!(lines[5].starts_with("datatype_const = "));
        assert!(lines[6] == "expected = true");
        assert!(lines[7] == "value_neg = true");
        assert!(lines[8].starts_with("value_hook_scaled = "));
        assert!(lines[9].starts_with("lexical_component = "));
        assert_eq!(lines.len(), 10);
    }

    /// The double + decimal value-lane inputs route to the dedicated-fn error in
    /// the general `prover_toml_for` (their FIELD-element witnesses do not fit the
    /// general entry's digit-byte threading), never a panic.
    #[cfg(feature = "dual-leaf")]
    #[test]
    fn prover_toml_for_dual_leaf_value_classes_is_recoverable_error() {
        for inputs in [
            ProofInputs::FilterValueDlF64 {
                id: CircuitId::FilterValueDlF64,
                operand_enc: fh("0x02"),
                op: FilterOp::Ge,
                b_bits: 0x4008000000000000,
                datatype_const: fh("0x03"),
                expected: true,
            },
            ProofInputs::FilterValueDlDecimal {
                id: CircuitId::FilterValueDlDecimal,
                operand_enc: fh("0x02"),
                op: FilterOp::Ge,
                bound_neg: false,
                bound_scaled: 10000,
                datatype_const: fh("0x03"),
                expected: true,
            },
            // [OPUS-5] sq-wz99x: the dateTime/date member routes the same way.
            ProofInputs::FilterValueDlDateTime {
                id: CircuitId::FilterValueDlDateTime,
                operand_enc: fh("0x02"),
                op: FilterOp::Ge,
                bound_neg: false,
                bound_scaled_epoch: 86_400_000,
                datatype_const: fh("0x03"),
                expected: true,
            },
        ] {
            let r = prover_toml_for(&inputs, &fh("0x01"), &[], &[], &[], None, None);
            assert_eq!(r, Err(ProverTomlError::FilterValueDlUseDedicatedFn));
        }
    }

    // [OPUS-4.8] sq-3kd2g.6: the bounded-depth path Prover.toml renderer emits the
    // public + private fields in the EXACT `main` declaration order, and the
    // general `prover_toml_for` routes a PathReach input to the dedicated-fn error.
    #[cfg(feature = "extended-fragment")]
    #[test]
    fn path_reach_toml_shape_and_order() {
        let toml = path_reach_prover_toml(
            &fh("0x01"),                     // challenge
            &[fh("0x0a"), fh("0x0b")],       // commitments (k=2)
            &fh("0x11"),                     // pred_enc
            &fh("0x22"),                     // src_enc
            &fh("0x33"),                     // dst_enc
            true,                            // allow_zero
            4,                               // depth_bound
            &[true, false],                  // attribution (k=2)
            2,                               // path_len
            &[fh("0x44"), fh("0x55"), fh("0x55"), fh("0x55")], // nodes (padded to d=4)
            &[2, 0],                         // counts (k=2)
            &[vec![[fh("0x1"), fh("0x2"), fh("0x3")]], vec![]], // enc[k][*][3]
        );
        let lines: Vec<&str> = toml.lines().collect();
        assert!(lines[0].starts_with("challenge = "));
        assert!(lines[1].starts_with("commitments = ["));
        assert!(lines[2].starts_with("pred_enc = "));
        assert!(lines[3].starts_with("src_enc = "));
        assert!(lines[4].starts_with("dst_enc = "));
        assert_eq!(lines[5], "allow_zero = true");
        assert_eq!(lines[6], "depth_bound = \"4\"");
        assert_eq!(lines[7], "attribution = [true, false]");
        assert_eq!(lines[8], "path_len = \"2\"");
        assert!(lines[9].starts_with("nodes = ["));
        assert!(lines[10].starts_with("counts = ["));
        assert!(lines[11].starts_with("enc = ["));
    }

    #[cfg(feature = "extended-fragment")]
    #[test]
    fn prover_toml_for_path_reach_is_recoverable_error() {
        // A PathReach input routes to the dedicated-fn error in the general entry
        // (its chain-shaped witness does not fit the scalar/digit threading).
        let inputs = ProofInputs::PathReach {
            id: CircuitId::PathReach { d: 4, k: 1, n: 16 },
            commitments: vec![fh("0x0a")],
            pred_enc: fh("0x11"),
            src_enc: fh("0x22"),
            dst_enc: fh("0x33"),
            allow_zero: false,
            depth_bound: 4,
            attribution: vec![true],
        };
        let r = prover_toml_for(&inputs, &fh("0x01"), &[], &[], &[], None, None);
        assert_eq!(r, Err(ProverTomlError::PathReachUseDedicatedFn));
        assert!(r.unwrap_err().to_string().contains("path_reach_prover_toml"));
    }
}
