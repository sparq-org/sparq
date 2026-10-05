//! Dual-leaf value-lane host encoding (sq-xojl) — the host mirror of the
//! `filter_value_dl_int` Noir circuit member.
//!
//! OPT-IN, behind the `dual-leaf` cargo feature (OFF by default). This module is
//! compiled out of a normal build: the default `string-canonical` commitment
//! pipeline (`encode.rs`/`commit.rs`) is byte-unchanged.
//!
//! # The dual-leaf literal shape (`research/zk-field-native-encoding.md` §3.1)
//!
//! ```text
//! Enc_literal = h3(value_component, lexical_component, TYPE_CODE_LITERAL)
//!   value_component   = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE)
//!   lexical_component = blake3_field(canonical N-Triples token)   // == today's
//!                                                                 // string-canonical h_s
//! ```
//!
//! - `VALUE_HOOK` is the numeric value handle. For `xsd:integer` it is the
//!   integer value itself as a field element (canonical by construction:
//!   `"05"` and `"5"` parse to the SAME hook — exactly the value-collapse the
//!   `lexical_component` disambiguates for identity ops).
//! - `DATATYPE_CONST = blake3_field(datatype IRI)` folds the datatype in so a
//!   cross-datatype value collision cannot occur.
//! - `LANG_NONE` is the reserved no-language sentinel (numeric datatypes have no
//!   language); it mirrors the Noir member's `LANG_NONE` global.
//! - `lexical_component` is exactly the string-canonical scheme's `h_s`
//!   (`blake3_field(literal.to_string())`), carried so identity ops
//!   (`sameTerm`/`DISTINCT`/`join`) keep term identity unchanged.
//!
//! This host encoder + the `filter_value_dl_int` circuit member recompute the
//! SAME leaf, so the verifier reconstructs the public `operand_enc` correctly
//! (`dual_leaf_value_components` is the cross-check seam).
//!
//! # INV-VL DOWNGRADE — DOCUMENTED RISK (load-bearing)
//!
//! The string-canonical pipeline enforces, in-circuit against an arbitrary
//! committer (including a malicious *trusted* issuer), that the compared value
//! equals `parse(committed lexical)` (the invariant INV-VL), because value and
//! binding derive from one witnessed digit array. The dual leaf witnesses
//! `VALUE_HOOK` and `lexical_component` INDEPENDENTLY, so it **REMOVES INV-VL**:
//! value↔lexical agreement on the value-FILTER lane moves from MACHINE-ENFORCED
//! to TRUSTED-ISSUER-HONESTY. A malicious *trusted* issuer can commit a leaf
//! whose `VALUE_HOOK` answers a value-FILTER as 18 while its `lexical_component`
//! answers `sameTerm`/`DISTINCT`/`join` as "5" — impossible in the
//! string-canonical pipeline. No *untrusted* party can exploit it (the issuer
//! signature chain is intact). The maintainer ACCEPTED this at research grade
//! (#769) and asked for it built WITH documentation; it is an open external-audit
//! obligation (gap CR-G8 / sq-qhy4). The honest host mitigation [`encode_literal`]
//! provides is **same-leaf co-binding**: it derives `VALUE_HOOK` and
//! `lexical_component` from the SAME parsed value and fails closed if the lexical
//! form does not canonically parse — so *sparq's own* commitments cannot
//! self-desync (`research/zk-field-native-encoding.md` §6). This binds honest
//! sparq ingest, NOT a malicious external issuer.
//!
//! The whole ZK estate is remediated + internally re-audited but **NOT externally
//! audited** (sq-qhy4, P0). Nothing here is a soundness or privacy guarantee.
//!
//! # The `DualLeafV1` whole-graph host commitment builder (sq-vvfte, §11 bead 2)
//!
//! [`encode_term_dual`] / [`encode_triple_dual`] / [`commit_triples_dual`] /
//! [`commit_graph_dual`] are the DualLeafV1 mirror of `encode.rs`/`commit.rs`:
//! the same RDFC10 canonicalization, the same canonical leaf ORDER, the same
//! flat per-graph Poseidon2 sponge — only the per-term leaf SHAPE differs
//! (`research/zk-field-native-encoding.md` §3.2/§3.5). The default
//! `string-canonical` pipeline is untouched and byte-identical; nothing in
//! `encode.rs`/`commit.rs` calls into this module.
//!
//! Leaf shapes (the §3.2 table):
//!
//! ```text
//! IRI          h3(NO_VALUE,        blake3(iri),                     TYPE_CODE_IRI)
//! blank node   h3(NO_VALUE,        h2(salt_G, blake3(label)),       TYPE_CODE_BLANK_NODE)
//! hookable     h3(value_component, blake3(N-Triples token),         TYPE_CODE_LITERAL)
//!   literal      value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE)
//! string /     h3(degenerate,      blake3(N-Triples token),         TYPE_CODE_LITERAL)
//!   langString / degenerate      = h3(VALUE_NONE, DATATYPE_CONST, LANG_CONST)
//!   opaque
//! ```
//!
//! The slot-1 lexical component is, for EVERY term class, byte-identical to the
//! string-canonical scheme's `h_s` — so a DualLeafV1 graph carries exactly the
//! same term identity as the same graph committed under `StringCanonicalV1`.
//!
//! **FAIL-CLOSED ingest (§6, load-bearing).** A literal whose datatype IS on a
//! hookable value lane but whose lexical form the lane encoder REJECTS is an
//! ERROR for the whole graph commitment ([`DualCommitError::Leaf`]) — it is
//! **NEVER** silently downgraded onto the string lane. A silent downgrade is
//! precisely the §6 value↔lexical desync this method's host mitigation exists to
//! prevent.

use crate::canon::{self, CanonError, CanonicalGraph};
use crate::encode::{TYPE_CODE_BLANK_NODE, TYPE_CODE_IRI, TYPE_CODE_LITERAL};
use crate::field::{field_from_hash_bytes, Fr};
use crate::poseidon2;
use oxrdf::{Literal, NamedOrBlankNode, Term, Triple};

/// The reserved "no language" sentinel for the `LANG_NONE` slot — mirrors the
/// Noir `filter_value::LANG_NONE` global. A real language tag would be
/// `blake3(lang)`; numeric datatypes have no language, so they fold this fixed
/// field tag, which is distinct from any plausible blake3 output (a small
/// reserved value) so a value component can never collide a language-tagged one.
pub const LANG_NONE: u64 = 1;

/// The `xsd:integer` datatype IRI (the integer value-lane datatype class,
/// sq-xojl).
pub const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

/// The `xsd:double` datatype IRI (the IEEE-754 value-lane class, sq-2ezsx).
pub const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";

/// The `xsd:decimal` datatype IRI (the fixed-scale value-lane class, sq-2ezsx).
pub const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";

/// The canonical IEEE-754 quiet-NaN bit pattern the double value lane folds ALL
/// NaN payloads to — mirrors the Noir `filter_value::F64_CANONICAL_NAN`. Every
/// NaN is one SPARQL-numeric unordered class, so the value handle must not
/// distinguish payloads.
pub const F64_CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;

fn blake3_field(bytes: &[u8]) -> Fr {
    field_from_hash_bytes(blake3::hash(bytes).as_bytes())
}

/// The `DATATYPE_CONST` for a datatype IRI: `blake3_field(IRI bytes)`. Folded
/// into `value_component` so a cross-datatype value collision (integer 5 vs the
/// bits for 5.0) is impossible.
pub fn datatype_const(datatype_iri: &str) -> Fr {
    blake3_field(datatype_iri.as_bytes())
}

/// The `DATATYPE_CONST` for an `xsd:decimal` value handle at a fixed fraction
/// SCALE: `blake3_field("<xsd:decimal IRI>@scale=<fd>")` — mirrors the Noir
/// `filter_value_dl_decimal` member's public `datatype_const` (which folds BOTH
/// the datatype AND the scale). This is the explicit canonical-SCALE bind (B4):
/// `"5.0"` (scale 1) and `"5.00"` (scale 2) have the SAME numeric value but are
/// DIFFERENT value handles because the scale is folded in, so a value at one scale
/// can never collide a value at another. `fd` is the fraction-digit count of the
/// canonical decimal lexical form.
pub fn decimal_datatype_const(fd: u32) -> Fr {
    blake3_field(format!("{}@scale={}", XSD_DECIMAL, fd).as_bytes())
}

/// Fold an `xsd:double` IEEE-754 bit pattern to its SPARQL-numeric CANONICAL bit
/// pattern — the host mirror of the Noir `filter_value::canonical_f64_bits`: any
/// NaN -> [`F64_CANONICAL_NAN`], `-0.0` -> `+0.0`, everything else unchanged. This
/// is the explicit IEEE-bit bind (B4) the dual-leaf double class needs so the
/// value handle is single-valued per SPARQL-numeric value.
pub fn canonical_f64_bits(bits: u64) -> u64 {
    let exp_all_ones = bits & 0x7ff0_0000_0000_0000 == 0x7ff0_0000_0000_0000;
    let mantissa_nonzero = bits & 0x000f_ffff_ffff_ffff != 0;
    if exp_all_ones && mantissa_nonzero {
        // NaN (exponent all ones, non-zero mantissa) -> the canonical qNaN.
        F64_CANONICAL_NAN
    } else if bits == 0x8000_0000_0000_0000 {
        // -0.0 -> +0.0 (numerically equal).
        0
    } else {
        bits
    }
}

/// The dual-leaf failure for a literal that cannot be encoded on the requested
/// value lane.
#[derive(Debug, PartialEq, Eq)]
pub enum DualLeafError {
    /// The literal's datatype is not the one the called encoder handles (e.g.
    /// passing an `xsd:double` to [`encode_literal`], which is `xsd:integer`-only).
    NotValueLane(String),
    /// The lexical form does not canonically parse to the value lane's value
    /// handle — fail closed (same-leaf co-binding, §6). This is the host
    /// mitigation that keeps sparq's own commitments INV-VL-consistent; it does
    /// NOT bind a malicious external issuer.
    NonCanonicalValue(String),
}

impl std::fmt::Display for DualLeafError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DualLeafError::NotValueLane(t) => {
                write!(f, "not the expected value-lane datatype: {}", t)
            }
            DualLeafError::NonCanonicalValue(t) => {
                write!(f, "non-canonical value-lane literal (fail-closed co-binding): {}", t)
            }
        }
    }
}

impl std::error::Error for DualLeafError {}

/// The three field components of a dual-leaf literal value, exposed so the
/// verifier / cross-tests can reconstruct the public `operand_enc` exactly as
/// the circuit does. The leaf is
/// `h3(value_component, lexical_component, TYPE_CODE_LITERAL)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DualLeafComponents {
    /// `VALUE_HOOK`: the numeric value handle as a field element (the integer
    /// value for `xsd:integer`).
    pub value_hook: Fr,
    /// `DATATYPE_CONST = blake3_field(datatype IRI)`.
    pub datatype_const: Fr,
    /// `lexical_component = blake3_field(canonical N-Triples token)` — exactly the
    /// string-canonical scheme's `h_s`.
    pub lexical_component: Fr,
}

impl DualLeafComponents {
    /// `value_component = h3(VALUE_HOOK, DATATYPE_CONST, LANG_NONE)`.
    pub fn value_component(&self) -> Fr {
        poseidon2::hash(&[self.value_hook, self.datatype_const, Fr::from(LANG_NONE)])
    }

    /// The full dual leaf `Enc = h3(value_component, lexical_component, TYPE_CODE_LITERAL)`
    /// — the value committed as the literal's `operand_enc`, recomputed exactly
    /// as `filter_value_dl_int` does in-circuit.
    pub fn leaf(&self) -> Fr {
        poseidon2::hash(&[
            self.value_component(),
            self.lexical_component,
            Fr::from(TYPE_CODE_LITERAL),
        ])
    }
}

/// Encodes an `xsd:integer` literal under the dual-leaf method, with fail-closed
/// same-leaf co-binding (§6): `VALUE_HOOK` and `lexical_component` are derived
/// from the SAME canonical value, and a lexical form that does not canonically
/// parse to a non-negative `u64` integer is REJECTED (so sparq's own ingest
/// cannot self-desync). Returns the three components; `.leaf()` is the committed
/// `Enc`.
///
/// Honest scope: non-negative `xsd:integer`, magnitude `< 2^64` — the same
/// canonical-non-negative fragment the `filter_value_dl_int` member proves.
/// Negative integers / decimal / double are a documented follow-up.
pub fn encode_literal(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    let dt = literal.datatype();
    if dt.as_str() != XSD_INTEGER {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    // Same-leaf co-binding: parse the lexical value once. Fail closed on a
    // non-canonical / out-of-range / signed lexical form — this is the host
    // mitigation that keeps sparq's own commitments INV-VL-consistent.
    let lex = literal.value();
    let value = canonical_nonneg_u64(lex)
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: Fr::from(value),
        datatype_const: datatype_const(XSD_INTEGER),
        // lexical_component is EXACTLY the string-canonical h_s over the canonical
        // N-Triples token (the same bytes `encode::encode_term` hashes), so a
        // dual-leaf graph's identity ops read the same lexical identity.
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Parse a canonical non-negative `xsd:integer` lexical form to a `u64`.
/// Canonical = ASCII digits only, no sign, no leading zero (except the single
/// digit "0"), value `< 2^64`. Returns `None` for any non-canonical / signed /
/// overflowing form (the §6 fail-closed predicate).
fn canonical_nonneg_u64(lexical: &str) -> Option<u64> {
    if lexical.is_empty() || !lexical.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Reject non-canonical leading zero ("05"), but accept the lone "0".
    if lexical.len() > 1 && lexical.starts_with('0') {
        return None;
    }
    lexical.parse::<u64>().ok()
}

/// Encodes an `xsd:double` literal under the dual-leaf method (sq-2ezsx), with
/// fail-closed same-leaf co-binding (§6): the value handle is the SPARQL-numeric
/// CANONICAL IEEE-754 bit pattern of the SAME lexical form the `lexical_component`
/// hashes, and a lexical form outside the canonical `xsd:double` lexical space
/// is REJECTED (so sparq's own ingest cannot self-desync). Returns the three
/// components; `.leaf()` is the committed `Enc`.
///
/// The value handle is [`canonical_f64_bits`]`(parsed.to_bits())` — exactly the
/// `filter_value_dl_f64` member's in-circuit canonical bind: `-0.0`/`+0.0` and all
/// NaN payloads collapse to ONE handle, the many-to-one-on-the-term property the
/// `lexical_component` disambiguates for identity ops.
///
/// Honest scope: canonical scientific notation (one non-zero digit before the
/// point, no redundant trailing fractional zero, canonical integer exponent),
/// the two signed zero spellings, and `INF`/`-INF`/`NaN`. The general fractional
/// /scientific in-circuit decimal→IEEE parse the blake3 lane defers is SIDESTEPPED
/// here: the value handle is the IEEE bits, committed off-circuit. This ZK path
/// remains research-grade and awaits the external audit tracked by sq-qhy4.
pub fn encode_double(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_DOUBLE {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    // [GPT-5.6] Same-leaf co-binding: validate the canonical lexical form before
    // asking Rust for its value. Rust deliberately accepts a wider language.
    let bits = parse_xsd_double_bits(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: Fr::from(canonical_f64_bits(bits)),
        datatype_const: datatype_const(XSD_DOUBLE),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Encodes an `xsd:decimal` literal under the dual-leaf method (sq-2ezsx), with
/// fail-closed same-leaf co-binding (§6): the value handle is the SIGNED SCALED
/// MAGNITUDE `sign * round(|v| * 10^fd)` of the SAME lexical form the
/// `lexical_component` hashes, where `fd` is the fraction-digit count of that
/// lexical form; a non-canonical form is REJECTED. Returns the three components;
/// `.leaf()` is the committed `Enc`.
///
/// The B4 canonical-SCALE bind: `fd` is folded into `datatype_const`
/// ([`decimal_datatype_const`]), so `"5.0"` (fd=1) and `"5.00"` (fd=2) — the SAME
/// value, distinct lexical forms — get DIFFERENT handles (the scale is part of the
/// handle); within one scale the handle is canonical, and the `lexical_component`
/// disambiguates for identity ops. The sign is folded into the value handle (a
/// negative is the field negation), so a sign flip changes the leaf.
///
/// Honest scope: a canonical `xsd:decimal` lexical form `[-]?<int>.<frac>` whose
/// scaled magnitude `round(|v| * 10^fd)` fits `u64` (`fd >= 1`; the member's
/// fixed-point domain). An integer-only decimal (`"5"`, no point) or `>u64`
/// scaled magnitude is rejected fail-closed.
pub fn encode_decimal(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_DECIMAL {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    let (neg, scaled, fd) = canonical_decimal_scaled(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    let mag = Fr::from(scaled);
    let signed = if neg { -mag } else { mag };
    Ok(DualLeafComponents {
        value_hook: signed,
        datatype_const: decimal_datatype_const(fd),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Parse a canonical `xsd:double` lexical form to its IEEE-754 `u64` bit pattern.
/// Returns `None` for any non-canonical or unrepresentable form (fail-closed).
fn parse_xsd_double_bits(lexical: &str) -> Option<u64> {
    let v = match lexical {
        "INF" => f64::INFINITY,
        "-INF" => f64::NEG_INFINITY,
        "NaN" => f64::NAN,
        finite => {
            let unsigned = finite.strip_prefix('-').unwrap_or(finite);
            let (mantissa, exponent) = unsigned.split_once('E')?;
            // A second `E`, lowercase `e`, or a leading `+` fails these checks.
            let (integer, fraction) = mantissa.split_once('.')?;
            if integer.len() != 1
                || fraction.is_empty()
                || !integer.bytes().all(|b| b.is_ascii_digit())
                || !fraction.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            let zero = integer == "0" && fraction.bytes().all(|b| b == b'0');
            if zero {
                // The canonical signed-zero spellings are `0.0E0` and `-0.0E0`.
                if fraction != "0" || exponent != "0" {
                    return None;
                }
            } else if integer == "0" || (fraction.len() > 1 && fraction.ends_with('0')) {
                return None;
            }
            let exponent_digits = exponent.strip_prefix('-').unwrap_or(exponent);
            if exponent_digits.is_empty()
                || !exponent_digits.bytes().all(|b| b.is_ascii_digit())
                || (exponent_digits.len() > 1 && exponent_digits.starts_with('0'))
                || exponent == "-0"
            {
                return None;
            }
            let parsed = finite.parse::<f64>().ok()?;
            // Overflow/underflow must use their own canonical value spelling.
            if !parsed.is_finite() || (parsed == 0.0 && !zero) {
                return None;
            }
            parsed
        }
    };
    Some(v.to_bits())
}

/// Parse a canonical `xsd:decimal` lexical form `[-]?<int>.<frac>` to `(neg,
/// round(|v| * 10^fd), fd)`. Canonical = optional leading `-`, integer part with
/// no leading zero (except the lone `0`), a `.`, and `fd >= 1` fraction digits;
/// `-0...` is rejected (no `-0`); the scaled magnitude must fit `u64`. Returns
/// `None` for any non-canonical / integer-only / overflowing form (the §6
/// fail-closed predicate, mirroring the `filter_signed::filter_decimal_check`
/// canonical-form discipline).
fn canonical_decimal_scaled(lexical: &str) -> Option<(bool, u64, u32)> {
    let (neg, rest) = match lexical.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, lexical),
    };
    let (int_part, frac_part) = rest.split_once('.')?;
    // Integer + fraction parts: ASCII digits, fraction non-empty.
    if int_part.is_empty() || frac_part.is_empty() {
        return None;
    }
    if !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    // Canonical integer part: no leading zero unless it is the lone "0".
    if int_part.len() > 1 && int_part.starts_with('0') {
        return None;
    }
    let fd = frac_part.len() as u32;
    let int_val: u64 = int_part.parse().ok()?;
    let frac_val: u64 = frac_part.parse().ok()?;
    let scale = 10u64.checked_pow(fd)?;
    let scaled = int_val.checked_mul(scale)?.checked_add(frac_val)?;
    // No "-0.0...0": a negative sign on a zero scaled magnitude is not canonical.
    if neg && scaled == 0 {
        return None;
    }
    Some((neg, scaled, fd))
}

// ---------------------------------------------------------------------------
// sq-vvfte — the DualLeafV1 WHOLE-GRAPH host commitment builder (§11 bead 2).
// Host slice only: NO circuit / verifier change (the scan.nr + join.nr leaf
// recompute, `reconstruct_public_inputs`, and the cross-vectors are §11 bead 3).
// ---------------------------------------------------------------------------

/// The reserved **no-value** field tag (`VALUE_NONE`, §3.2). It occupies slot 0
/// of the DEGENERATE `value_component` a term with no numeric handle folds:
/// `h3(VALUE_NONE, DATATYPE_CONST, LANG_CONST)`.
///
/// **Distinctness does NOT rest on this tag's numeric value** (Q1, resolved
/// proceed-and-document under epic-owner sq-1s2.1). A `VALUE_HOOK` is an
/// arbitrary field element — for `xsd:integer` it is the integer itself — so no
/// small reserved tag can be globally disjoint from every real hook. What keeps
/// a degenerate `value_component` from ever colliding a real one is the
/// **datatype-folded tuple plus the routing discipline**:
///
/// 1. every datatype on a hookable value lane in this build is routed to its
///    lane encoder by [`encode_literal_dual`], so a hookable `DATATYPE_CONST`
///    **never** carries a degenerate `value_component`; and
/// 2. no real `VALUE_HOOK` is ever emitted under a NON-hookable
///    `DATATYPE_CONST` (`xsd:string`, `rdf:langString`, an opaque datatype),
///    so a degenerate const never carries a real hook.
///
/// The two sets are therefore separated by slot 1 (`DATATYPE_CONST`), not by
/// slot 0. The tag is nonetheless chosen distinct from the small hooks the
/// not-yet-routed lanes use (see [`is_hookable_datatype`]'s seam note), and
/// distinct from a real `VALUE_HOOK = 0`, so `literal_shapes_are_distinguished`
/// (`encode.rs`) stays true without leaning on the lexical lane alone.
pub const VALUE_NONE: u64 = 2;

/// The §3.2 table's **flat slot-0 `NO_VALUE` sentinel** for IRI and blank-node
/// leaves: `h3(NO_VALUE, lexical, TYPE_CODE)`. Non-literals have neither a
/// datatype nor a language to fold, so the table gives them the flat sentinel
/// rather than the datatype-folded degenerate tuple — this is the same reserved
/// no-value tag ([`VALUE_NONE`]), used unfolded. IRI / blank-node / literal
/// leaves are separated by the `TYPE_CODE` in slot 2 regardless.
pub const NO_VALUE: u64 = VALUE_NONE;

/// The `LANG_CONST` for a language tag: `blake3_field(lang)` — the `rdf:langString`
/// slot-2 constant of the degenerate `value_component` (§3.3). A literal with no
/// language folds the reserved [`LANG_NONE`] sentinel instead, so a language-tagged
/// literal can never share a `value_component` with an un-tagged one.
pub fn lang_const(language: &str) -> Fr {
    blake3_field(language.as_bytes())
}

/// Whether a datatype IRI is on a HOOKABLE value lane **in this build** — i.e.
/// whether [`encode_literal_dual`] routes it to a value-lane encoder (and so
/// applies the §6 fail-closed rule) rather than to the degenerate string lane.
///
/// SEAM (sq-vvfte): the `xsd:boolean` lane (`dual_leaf_boolean`) and the
/// `xsd:dateTime`/`xsd:date` lanes (`dual_leaf_datetime`) are deliberately NOT
/// joined here — joining them is a follow-on, because it narrows what a
/// DualLeafV1 graph may CONTAIN (e.g. the XSD-legal but non-canonical
/// `"1"^^xsd:boolean` becomes uncommittable) and re-bases those leaves. Until
/// they join, those literals take the degenerate string lane, and the §3.2
/// no-collision property still holds by construction:
/// - `xsd:boolean`'s real hooks are `{0, 1}` and [`VALUE_NONE`] is neither;
/// - the `xsd:dateTime`/`xsd:date` lanes fold their epoch SCALE into their
///   `DATATYPE_CONST` (`blake3("<iri>@epochscale=3")`), which is not the bare
///   `blake3(iri)` the degenerate folds — so the consts are disjoint outright.
pub fn is_hookable_datatype(datatype_iri: &str) -> bool {
    matches!(datatype_iri, XSD_INTEGER | XSD_DECIMAL | XSD_DOUBLE)
}

/// The DEGENERATE, datatype-folded `value_component` of §3.2:
/// `h3(VALUE_NONE, blake3(datatype IRI), LANG_CONST)`, where `LANG_CONST` is
/// [`lang_const`] of the language tag for `rdf:langString` and the reserved
/// [`LANG_NONE`] sentinel otherwise. Used for `xsd:string`, `rdf:langString` and
/// every opaque (non-hookable) datatype — the terms that have no numeric handle
/// and whose `lexical_component` is the sole binding.
pub fn degenerate_value_component(datatype_iri: &str, language: Option<&str>) -> Fr {
    let lang = match language {
        Some(l) => lang_const(l),
        None => Fr::from(LANG_NONE),
    };
    poseidon2::hash(&[Fr::from(VALUE_NONE), datatype_const(datatype_iri), lang])
}

/// Encodes a literal to its DualLeafV1 leaf, routing by datatype (§3.2/§3.3).
///
/// - a HOOKABLE datatype ([`is_hookable_datatype`]) goes to its value-lane
///   encoder and the leaf is that encoder's [`DualLeafComponents::leaf`];
/// - everything else (`xsd:string`, `rdf:langString`, opaque datatypes) takes
///   the degenerate string lane
///   `h3(degenerate_value_component, h_s, TYPE_CODE_LITERAL)`.
///
/// **Fail-closed (§6):** a hookable-datatyped literal whose lexical form the lane
/// encoder rejects returns the encoder's [`DualLeafError`]. It is NEVER
/// downgraded onto the string lane — that silent downgrade is the §6 desync.
pub fn encode_literal_dual(literal: &Literal) -> Result<Fr, DualLeafError> {
    let datatype = literal.datatype();
    let components = match datatype.as_str() {
        XSD_INTEGER => Some(encode_literal(literal)?),
        XSD_DECIMAL => Some(encode_decimal(literal)?),
        XSD_DOUBLE => Some(encode_double(literal)?),
        // SEAM (sq-vvfte): the `xsd:boolean` (`dual_leaf_boolean`) and
        // `xsd:dateTime`/`xsd:date` (`dual_leaf_datetime`) lanes join here as a
        // follow-on — see `is_hookable_datatype` for why they are not routed yet
        // and why the §3.2 no-collision property holds meanwhile. Adding an arm
        // here MUST also extend `is_hookable_datatype` (a test pins the two
        // together).
        _ => None,
    };
    Ok(match components {
        Some(c) => c.leaf(),
        None => poseidon2::hash(&[
            degenerate_value_component(datatype.as_str(), literal.language()),
            // The lexical slot is EXACTLY the string-canonical `h_s` over the
            // canonical N-Triples token — byte-identical to `encode.rs`'s.
            blake3_field(literal.to_string().as_bytes()),
            Fr::from(TYPE_CODE_LITERAL),
        ]),
    })
}

/// Encodes a term to its DualLeafV1 leaf under a graph salt — the DualLeafV1
/// mirror of [`crate::encode::encode_term`] (§3.2 table). Blank-node labels are
/// expected to be the RDFC10 canonical labels (encode after canonicalization),
/// and the salt-scoped inner `h2(salt_G, blake3(label))` is retained verbatim
/// from the string-canonical scheme (Q6 cross-graph correlation guard).
///
/// Returns `Ok(None)` for RDF 1.2 triple terms — outside the committed data
/// model, exactly as [`crate::encode::encode_term`] does, so callers fail
/// closed. Returns `Err` for a hookable literal the value lane rejects (§6).
pub fn encode_term_dual(term: &Term, salt: &Fr) -> Result<Option<Fr>, DualLeafError> {
    Ok(match term {
        Term::NamedNode(n) => Some(poseidon2::hash(&[
            Fr::from(NO_VALUE),
            blake3_field(n.as_str().as_bytes()),
            Fr::from(TYPE_CODE_IRI),
        ])),
        Term::BlankNode(b) => {
            let inner = poseidon2::hash(&[*salt, blake3_field(b.as_str().as_bytes())]);
            Some(poseidon2::hash(&[
                Fr::from(NO_VALUE),
                inner,
                Fr::from(TYPE_CODE_BLANK_NODE),
            ]))
        }
        Term::Literal(l) => Some(encode_literal_dual(l)?),
        Term::Triple(_) => None,
    })
}

/// Encodes a triple to its DualLeafV1 commitment leaf:
/// `h3(Enc(s), Enc(p), Enc(o))` — the same outer shape (and the same argument
/// order) as [`crate::encode::encode_triple`], over DualLeafV1 term leaves.
///
/// `Ok(None)` if any position is an RDF 1.2 triple term (fail closed); `Err` if
/// a hookable literal fails its lane's §6 canonical-lexical predicate.
pub fn encode_triple_dual(triple: &Triple, salt: &Fr) -> Result<Option<Fr>, DualLeafError> {
    let s = match &triple.subject {
        NamedOrBlankNode::NamedNode(n) => encode_term_dual(&Term::NamedNode(n.clone()), salt)?,
        NamedOrBlankNode::BlankNode(b) => encode_term_dual(&Term::BlankNode(b.clone()), salt)?,
    };
    let p = encode_term_dual(&Term::NamedNode(triple.predicate.clone()), salt)?;
    let o = encode_term_dual(&triple.object, salt)?;
    Ok(match (s, p, o) {
        (Some(s), Some(p), Some(o)) => Some(poseidon2::hash(&[s, p, o])),
        _ => None,
    })
}

/// A DualLeafV1 whole-graph commitment failure. Mirrors `commit::CommitError`
/// and adds the [`Leaf`](Self::Leaf) arm that carries the §6 fail-closed
/// value-lane rejection.
#[derive(Debug)]
pub enum DualCommitError {
    /// RDFC10 canonicalization failed.
    Canon(CanonError),
    /// A canonical triple contained a term outside the committed data model
    /// (an RDF 1.2 triple term).
    UncommittableTerm(String),
    /// A hookable-datatyped literal failed its value lane's §6 canonical-lexical
    /// predicate. **Fail-closed for the WHOLE graph** — never a silent
    /// string-lane downgrade of the offending literal.
    Leaf(DualLeafError),
}

impl std::fmt::Display for DualCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DualCommitError::Canon(e) => write!(f, "{e}"),
            DualCommitError::UncommittableTerm(t) => write!(f, "uncommittable term: {t}"),
            DualCommitError::Leaf(e) => write!(f, "dual-leaf value lane rejected a literal: {e}"),
        }
    }
}

impl std::error::Error for DualCommitError {}

impl From<CanonError> for DualCommitError {
    fn from(e: CanonError) -> Self {
        DualCommitError::Canon(e)
    }
}

impl From<DualLeafError> for DualCommitError {
    fn from(e: DualLeafError) -> Self {
        DualCommitError::Leaf(e)
    }
}

/// A graph committed under `DualLeafV1`: canonical form, ordered leaves, and the
/// commitment. Field-for-field the shape of `commit::GraphCommitment`, and the
/// leaf ORDER is identical (`canonical.lines` order) — only the per-term leaf
/// encoding differs.
#[derive(Debug, Clone)]
pub struct DualGraphCommitment {
    /// The RDFC10 canonical form (leaf order = `canonical.lines` order).
    pub canonical: CanonicalGraph,
    /// Per-triple DualLeafV1 leaf hashes, in canonical order.
    pub leaves: Vec<Fr>,
    /// `C(G)`: Poseidon2 sponge over `leaves`.
    pub commitment: Fr,
    /// The per-graph salt the leaves were encoded under (`zk:rdfc10Salt`).
    pub salt: Fr,
}

/// Canonicalizes and commits one named graph's content under `salt`, using the
/// `DualLeafV1` leaf shape — the DualLeafV1 mirror of `commit::commit_triples`.
///
/// The ordering contract is `commit.rs`'s, unchanged: RDFC10-canonicalize, then
/// one leaf per canonical triple in canonical N-Quads order (index = leaf
/// index), then ONE flat Poseidon2 sponge over the leaf sequence.
pub fn commit_triples_dual(
    triples: &[Triple],
    salt: Fr,
) -> Result<DualGraphCommitment, DualCommitError> {
    commit_canonical_dual(canon::canonicalize_triples(triples)?, salt)
}

/// Commits the content of a `sparq_core::Graph` under `salt` using the
/// `DualLeafV1` leaf shape — the DualLeafV1 mirror of
/// `commit::commit_graph_content`.
pub fn commit_graph_dual(
    g: &sparq_core::Graph,
    salt: Fr,
) -> Result<DualGraphCommitment, DualCommitError> {
    commit_canonical_dual(canon::canonicalize_graph_content(g)?, salt)
}

fn commit_canonical_dual(
    canonical: CanonicalGraph,
    salt: Fr,
) -> Result<DualGraphCommitment, DualCommitError> {
    let mut leaves = Vec::with_capacity(canonical.triples.len());
    for t in &canonical.triples {
        // `?` on the value-lane rejection is the §6 FAIL-CLOSED ingest rule: the
        // whole commitment fails, the offending literal is never downgraded.
        let leaf = encode_triple_dual(t, &salt)?
            .ok_or_else(|| DualCommitError::UncommittableTerm(t.to_string()))?;
        leaves.push(leaf);
    }
    let commitment = poseidon2::hash(&leaves);
    Ok(DualGraphCommitment { canonical, leaves, commitment, salt })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;

    fn int_lit(v: &str) -> Literal {
        Literal::new_typed_literal(v, NamedNode::new(XSD_INTEGER).unwrap())
    }

    #[test]
    fn integer_round_trips_to_a_dual_leaf() {
        let c = encode_literal(&int_lit("18")).unwrap();
        assert_eq!(c.value_hook, Fr::from(18u64));
        // The leaf is well-defined and stable.
        assert_eq!(c.leaf(), c.leaf());
        // value_component folds the datatype so 18-as-integer != a bare 18 hash.
        assert_ne!(c.value_component(), Fr::from(18u64));
    }

    #[test]
    fn lexical_component_equals_string_canonical_h_s() {
        // The dual leaf's lexical_component MUST be byte-identical to the
        // string-canonical scheme's h_s, so identity ops are unchanged.
        let lit = int_lit("18");
        let c = encode_literal(&lit).unwrap();
        let string_canonical_hs = blake3_field(lit.to_string().as_bytes());
        assert_eq!(c.lexical_component, string_canonical_hs);
    }

    #[test]
    fn host_leaf_matches_the_circuit_construction() {
        // The leaf the host commits MUST equal h3(h3(hook, dt, LANG_NONE),
        // lexical, TYPE_CODE_LITERAL) — exactly the filter_value_dl_int member's
        // construction. This is the load-bearing host<->circuit cross-check.
        let c = encode_literal(&int_lit("42")).unwrap();
        let vc = poseidon2::hash(&[c.value_hook, c.datatype_const, Fr::from(LANG_NONE)]);
        let leaf = poseidon2::hash(&[vc, c.lexical_component, Fr::from(TYPE_CODE_LITERAL)]);
        assert_eq!(c.leaf(), leaf);
    }

    #[test]
    fn value_collapse_05_and_5_is_intended_for_the_value_hook() {
        // "05" is NON-canonical and fail-closed-rejected at ingest (the §6
        // co-binding), so honest sparq never commits it. Canonical "5" parses.
        assert!(matches!(
            encode_literal(&int_lit("05")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        let c5 = encode_literal(&int_lit("5")).unwrap();
        assert_eq!(c5.value_hook, Fr::from(5u64));
    }

    #[test]
    fn non_value_lane_literal_is_rejected() {
        let plain = Literal::new_simple_literal("hello");
        assert!(matches!(
            encode_literal(&plain),
            Err(DualLeafError::NotValueLane(_))
        ));
        let dbl = Literal::new_typed_literal(
            "1.5",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#double").unwrap(),
        );
        assert!(matches!(encode_literal(&dbl), Err(DualLeafError::NotValueLane(_))));
    }

    #[test]
    fn signed_and_overflow_are_fail_closed() {
        // The §6 co-binding rejects signed forms (this slice is non-negative
        // only) and out-of-u64-range forms — fail closed, never silent.
        assert!(matches!(
            encode_literal(&int_lit("-5")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        let huge = "99999999999999999999999999"; // > u64::MAX
        assert!(matches!(
            encode_literal(&int_lit(huge)),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
    }

    #[test]
    fn zero_is_canonical() {
        let c = encode_literal(&int_lit("0")).unwrap();
        assert_eq!(c.value_hook, Fr::from(0u64));
    }

    #[test]
    fn distinct_values_give_distinct_leaves() {
        let a = encode_literal(&int_lit("17")).unwrap().leaf();
        let b = encode_literal(&int_lit("18")).unwrap().leaf();
        assert_ne!(a, b);
    }

    // ---- xsd:double class (sq-2ezsx) ----

    fn dbl_lit(v: &str) -> Literal {
        Literal::new_typed_literal(v, NamedNode::new(XSD_DOUBLE).unwrap())
    }

    #[test]
    fn double_round_trips_to_a_dual_leaf() {
        let c = encode_double(&dbl_lit("2.5E0")).unwrap();
        assert_eq!(c.value_hook, Fr::from(2.5f64.to_bits()));
        assert_eq!(c.datatype_const, datatype_const(XSD_DOUBLE));
        assert_eq!(c.leaf(), c.leaf());
    }

    #[test]
    fn double_host_leaf_matches_the_circuit_construction() {
        // h3(h3(canon_bits, dt, LANG_NONE), lexical, TYPE_CODE_LITERAL) — exactly
        // the filter_value_dl_f64 member's construction.
        let lit = dbl_lit("1.0E1");
        let c = encode_double(&lit).unwrap();
        let vc = poseidon2::hash(&[c.value_hook, c.datatype_const, Fr::from(LANG_NONE)]);
        let leaf = poseidon2::hash(&[vc, c.lexical_component, Fr::from(TYPE_CODE_LITERAL)]);
        assert_eq!(c.leaf(), leaf);
    }

    #[test]
    fn double_neg_zero_collapses_to_pos_zero_value_handle() {
        // THE B4 INVARIANT (double): -0.0 and +0.0 are numerically equal, so the
        // CANONICAL value handle is identical (both fold to +0.0 bits). The lexical
        // forms differ, so the leaves still differ (identity ops disambiguated).
        let neg = encode_double(&dbl_lit("-0.0E0")).unwrap();
        let pos = encode_double(&dbl_lit("0.0E0")).unwrap();
        assert_eq!(neg.value_hook, Fr::from(0u64));
        assert_eq!(pos.value_hook, Fr::from(0u64));
        assert_eq!(neg.value_component(), pos.value_component());
        // The lexical_component differs ("-0.0E0" vs "0.0E0"), so leaves differ.
        assert_ne!(neg.lexical_component, pos.lexical_component);
        assert_ne!(neg.leaf(), pos.leaf());
    }

    #[test]
    fn double_nan_payloads_share_the_value_handle() {
        // THE B4 INVARIANT (double): every NaN is one SPARQL-numeric class, so the
        // canonical handle is the same regardless of payload. The host canonicalises
        // the bits before the value handle.
        let c = encode_double(&dbl_lit("NaN")).unwrap();
        assert_eq!(c.value_hook, Fr::from(F64_CANONICAL_NAN));
    }

    #[test]
    fn double_canonical_bits_helper() {
        assert_eq!(canonical_f64_bits(0x8000_0000_0000_0000), 0); // -0.0 -> +0.0
        assert_eq!(canonical_f64_bits(0x7ff0_0000_0000_0001), F64_CANONICAL_NAN); // sNaN
        assert_eq!(canonical_f64_bits(0x7ff8_0000_0000_0000), F64_CANONICAL_NAN); // qNaN
        // +inf and a finite value are unchanged.
        assert_eq!(canonical_f64_bits(0x7ff0_0000_0000_0000), 0x7ff0_0000_0000_0000);
        assert_eq!(canonical_f64_bits(2.5f64.to_bits()), 2.5f64.to_bits());
    }

    #[test]
    fn double_rejects_non_double_and_bad_lexical() {
        assert!(matches!(
            encode_double(&int_lit("5")),
            Err(DualLeafError::NotValueLane(_))
        ));
        assert!(matches!(
            encode_double(&dbl_lit("not-a-number")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
    }

    #[test]
    fn double_rejects_non_canonical_lexicals() {
        // [GPT-5.6] Audit regression: Rust's permissive `f64` parser accepts all
        // of these, but none is in the canonical xsd:double lexical space.
        for lexical in [
            "inf", "Infinity", "+INF", "1.", "+.5", "01.5", "1e3", "1.0E+3",
            "1.0E03", "0.5E0", "1.00E0", "1.0E9999", "1.0E-9999",
        ] {
            assert!(
                matches!(
                    encode_double(&dbl_lit(lexical)),
                    Err(DualLeafError::NonCanonicalValue(_))
                ),
                "non-canonical xsd:double lexical was accepted: {lexical}"
            );
        }
    }

    #[test]
    fn double_accepts_canonical_lexicals() {
        for lexical in [
            "INF", "-INF", "NaN", "0.0E0", "-0.0E0", "1.0E0", "-2.5E3", "5.0E-1",
        ] {
            assert!(
                encode_double(&dbl_lit(lexical)).is_ok(),
                "canonical xsd:double lexical was rejected: {lexical}"
            );
        }
    }

    // ---- xsd:decimal class (sq-2ezsx) ----

    fn dec_lit(v: &str) -> Literal {
        Literal::new_typed_literal(v, NamedNode::new(XSD_DECIMAL).unwrap())
    }

    #[test]
    fn decimal_round_trips_to_a_dual_leaf() {
        // "123.45" at fd=2 -> scaled 12345, non-negative.
        let c = encode_decimal(&dec_lit("123.45")).unwrap();
        assert_eq!(c.value_hook, Fr::from(12345u64));
        assert_eq!(c.datatype_const, decimal_datatype_const(2));
        assert_eq!(c.leaf(), c.leaf());
    }

    #[test]
    fn decimal_host_leaf_matches_the_circuit_construction() {
        let c = encode_decimal(&dec_lit("2.50")).unwrap();
        // Mirror the circuit: signed_value = scaled (non-negative).
        let vc = poseidon2::hash(&[c.value_hook, c.datatype_const, Fr::from(LANG_NONE)]);
        let leaf = poseidon2::hash(&[vc, c.lexical_component, Fr::from(TYPE_CODE_LITERAL)]);
        assert_eq!(c.leaf(), leaf);
    }

    #[test]
    fn decimal_negative_folds_sign_into_value_handle() {
        // -2.50 (scaled 250, neg) — the value handle is the field negation of 250.
        let c = encode_decimal(&dec_lit("-2.50")).unwrap();
        assert_eq!(c.value_hook, -Fr::from(250u64));
        // The negative and positive of the same magnitude give DIFFERENT handles.
        let pos = encode_decimal(&dec_lit("2.50")).unwrap();
        assert_ne!(c.value_hook, pos.value_hook);
        assert_ne!(c.leaf(), pos.leaf());
    }

    #[test]
    fn decimal_scale_is_folded_so_5_0_and_5_00_differ() {
        // THE B4 CANONICAL-SCALE INVARIANT: "5.0" (fd=1) and "5.00" (fd=2) are the
        // SAME numeric value but DIFFERENT value handles (the scale is folded into
        // datatype_const), so a value at one scale cannot collide a value at another.
        let a = encode_decimal(&dec_lit("5.0")).unwrap();
        let b = encode_decimal(&dec_lit("5.00")).unwrap();
        assert_eq!(a.value_hook, Fr::from(50u64)); // 5.0 * 10^1
        assert_eq!(b.value_hook, Fr::from(500u64)); // 5.00 * 10^2
        assert_ne!(a.datatype_const, b.datatype_const);
        assert_ne!(a.value_component(), b.value_component());
        assert_ne!(a.leaf(), b.leaf());
    }

    #[test]
    fn decimal_rejects_non_canonical_forms() {
        assert!(matches!(
            encode_decimal(&int_lit("5")),
            Err(DualLeafError::NotValueLane(_))
        ));
        // Integer-only decimal (no point) is rejected (fixed-point member needs fd>=1).
        assert!(matches!(
            encode_decimal(&dec_lit("5")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        // Leading zero on the integer part is non-canonical.
        assert!(matches!(
            encode_decimal(&dec_lit("05.0")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        // "-0.0" is rejected (no negative zero).
        assert!(matches!(
            encode_decimal(&dec_lit("-0.0")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        // Lone "0.0" is canonical (non-negative zero).
        assert_eq!(encode_decimal(&dec_lit("0.0")).unwrap().value_hook, Fr::from(0u64));
    }

    // ---- sq-vvfte: the DualLeafV1 whole-graph host commitment builder ----

    use crate::encode::salt_from_bytes;
    use oxrdf::BlankNode;

    const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
    const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

    fn graph_triples() -> Vec<Triple> {
        let b = BlankNode::new("x").unwrap();
        vec![
            Triple::new(
                NamedOrBlankNode::BlankNode(b.clone()),
                NamedNode::new("http://ex/p").unwrap(),
                Term::Literal(Literal::new_simple_literal("v")),
            ),
            Triple::new(
                NamedOrBlankNode::BlankNode(b),
                NamedNode::new("http://ex/q").unwrap(),
                Term::NamedNode(NamedNode::new("http://ex/o").unwrap()),
            ),
        ]
    }

    #[test]
    fn dual_iri_leaf_pins_the_flat_no_value_slot0_shape() {
        // §3.2 table: Enc(IRI) = h3(NO_VALUE, blake3(iri), TYPE_CODE_IRI) — the
        // value-first / type-last tuple, NOT the string-canonical h2 pair.
        let salt = salt_from_bytes(&[7u8; 32]);
        let iri = Term::NamedNode(NamedNode::new("http://ex/a").unwrap());
        let expected = poseidon2::hash(&[
            Fr::from(NO_VALUE),
            blake3_field(b"http://ex/a"),
            Fr::from(TYPE_CODE_IRI),
        ]);
        assert_eq!(encode_term_dual(&iri, &salt).unwrap(), Some(expected));
        // Salt-independent, exactly like the string-canonical IRI leaf.
        let other = salt_from_bytes(&[9u8; 32]);
        assert_eq!(
            encode_term_dual(&iri, &salt).unwrap(),
            encode_term_dual(&iri, &other).unwrap()
        );
    }

    #[test]
    fn dual_bnode_leaf_retains_the_q6_salt_scoped_inner() {
        // §3.2 table: Enc(bnode) = h3(NO_VALUE, h2(salt_G, blake3(label)),
        // TYPE_CODE_BLANK_NODE). The salt-scoped inner is retained VERBATIM from
        // the string-canonical scheme (Q6 cross-graph correlation guard).
        let salt = salt_from_bytes(&[7u8; 32]);
        let b = Term::BlankNode(BlankNode::new("c14n0").unwrap());
        let inner = poseidon2::hash(&[salt, blake3_field(b"c14n0")]);
        let expected =
            poseidon2::hash(&[Fr::from(NO_VALUE), inner, Fr::from(TYPE_CODE_BLANK_NODE)]);
        assert_eq!(encode_term_dual(&b, &salt).unwrap(), Some(expected));
        // Different graphs (different salts) -> different leaves (the Q6 property).
        let other = salt_from_bytes(&[8u8; 32]);
        assert_ne!(
            encode_term_dual(&b, &salt).unwrap(),
            encode_term_dual(&b, &other).unwrap()
        );
    }

    #[test]
    fn lexical_slot_is_byte_identical_to_the_string_canonical_h_s_per_term_class() {
        // THE INVARIANT: for EVERY term class the dual leaf's slot-1 lexical
        // component is exactly the `h_s` the string-canonical encoder hashes, so
        // a DualLeafV1 graph carries the same term identity. Each case binds ONE
        // `hs` value into BOTH compositions, so a divergence in either is caught.
        let salt = salt_from_bytes(&[3u8; 32]);

        let iri = NamedNode::new("http://ex/a").unwrap();
        let hs = blake3_field(iri.as_str().as_bytes());
        assert_eq!(
            crate::encode::encode_term(&Term::NamedNode(iri.clone()), &salt).unwrap(),
            poseidon2::hash(&[Fr::from(TYPE_CODE_IRI), hs]),
        );
        assert_eq!(
            encode_term_dual(&Term::NamedNode(iri), &salt).unwrap(),
            Some(poseidon2::hash(&[Fr::from(NO_VALUE), hs, Fr::from(TYPE_CODE_IRI)])),
        );

        let b = BlankNode::new("c14n0").unwrap();
        let hs = poseidon2::hash(&[salt, blake3_field(b.as_str().as_bytes())]);
        assert_eq!(
            crate::encode::encode_term(&Term::BlankNode(b.clone()), &salt).unwrap(),
            poseidon2::hash(&[Fr::from(TYPE_CODE_BLANK_NODE), hs]),
        );
        assert_eq!(
            encode_term_dual(&Term::BlankNode(b), &salt).unwrap(),
            Some(poseidon2::hash(&[Fr::from(NO_VALUE), hs, Fr::from(TYPE_CODE_BLANK_NODE)])),
        );

        // Both literal lanes: the degenerate string lane AND a hookable lane.
        for lit in [Literal::new_simple_literal("v"), int_lit("18")] {
            let hs = blake3_field(lit.to_string().as_bytes());
            assert_eq!(
                crate::encode::encode_term(&Term::Literal(lit.clone()), &salt).unwrap(),
                poseidon2::hash(&[Fr::from(TYPE_CODE_LITERAL), hs]),
                "string-canonical h_s drifted for {lit}"
            );
            let value_component = if is_hookable_datatype(lit.datatype().as_str()) {
                encode_literal(&lit).unwrap().value_component()
            } else {
                degenerate_value_component(lit.datatype().as_str(), lit.language())
            };
            assert_eq!(
                encode_term_dual(&Term::Literal(lit.clone()), &salt).unwrap(),
                Some(poseidon2::hash(&[value_component, hs, Fr::from(TYPE_CODE_LITERAL)])),
                "dual lexical slot drifted for {lit}"
            );
        }
    }

    #[test]
    fn degenerate_lane_folds_the_datatype_and_the_language() {
        // §3.2: the string/opaque lane's value_component is the DATATYPE-FOLDED
        // degenerate h3(VALUE_NONE, DATATYPE_CONST, LANG_CONST) — Q1 resolved.
        let plain = Literal::new_simple_literal("v"); // datatype xsd:string
        assert_eq!(plain.datatype().as_str(), XSD_STRING);
        assert_eq!(
            encode_literal_dual(&plain).unwrap(),
            poseidon2::hash(&[
                poseidon2::hash(&[
                    Fr::from(VALUE_NONE),
                    datatype_const(XSD_STRING),
                    Fr::from(LANG_NONE),
                ]),
                blake3_field(plain.to_string().as_bytes()),
                Fr::from(TYPE_CODE_LITERAL),
            ]),
        );

        // rdf:langString folds blake3(lang) into the LANG_CONST slot.
        let en = Literal::new_language_tagged_literal("v", "en").unwrap();
        assert_eq!(en.datatype().as_str(), RDF_LANG_STRING);
        assert_eq!(
            encode_literal_dual(&en).unwrap(),
            poseidon2::hash(&[
                poseidon2::hash(&[
                    Fr::from(VALUE_NONE),
                    datatype_const(RDF_LANG_STRING),
                    lang_const("en"),
                ]),
                blake3_field(en.to_string().as_bytes()),
                Fr::from(TYPE_CODE_LITERAL),
            ]),
        );

        // The §3.2 distinctness the degeneracy must preserve (the encode.rs
        // `literal_shapes_are_distinguished` property, on the dual lane): a plain
        // string, an opaque-datatyped literal, a langString, and a hookable
        // integer over the SAME lexical are pairwise distinct.
        let opaque = Literal::new_typed_literal("v", NamedNode::new("http://ex/dt").unwrap());
        let one_plain = Literal::new_simple_literal("1");
        let one_lang = Literal::new_language_tagged_literal("1", "en").unwrap();
        let leaves = [
            encode_literal_dual(&plain).unwrap(),
            encode_literal_dual(&opaque).unwrap(),
            encode_literal_dual(&en).unwrap(),
            encode_literal_dual(&one_plain).unwrap(),
            encode_literal_dual(&one_lang).unwrap(),
            encode_literal_dual(&int_lit("1")).unwrap(),
        ];
        for (i, a) in leaves.iter().enumerate() {
            for b in &leaves[i + 1..] {
                assert_ne!(a, b, "dual literal leaves must be pairwise distinct");
            }
        }
        // A different language is a different value_component (LANG_CONST is
        // load-bearing, not decorative).
        assert_ne!(
            degenerate_value_component(RDF_LANG_STRING, Some("en")),
            degenerate_value_component(RDF_LANG_STRING, Some("fr")),
        );
        assert_ne!(
            degenerate_value_component(RDF_LANG_STRING, Some("en")),
            degenerate_value_component(RDF_LANG_STRING, None),
        );
    }

    #[test]
    fn hookable_literals_route_to_their_value_lane_encoder() {
        // The routing contract: a hookable datatype's leaf IS its lane encoder's
        // `.leaf()` (the value_component carries a REAL hook), never the
        // degenerate string-lane leaf.
        for (lit, components) in [
            (int_lit("18"), encode_literal(&int_lit("18")).unwrap()),
            (dec_lit("2.50"), encode_decimal(&dec_lit("2.50")).unwrap()),
            (dbl_lit("2.5E0"), encode_double(&dbl_lit("2.5E0")).unwrap()),
        ] {
            assert!(is_hookable_datatype(lit.datatype().as_str()));
            assert_eq!(encode_literal_dual(&lit).unwrap(), components.leaf());
            // NOT the degenerate lane.
            let degenerate = poseidon2::hash(&[
                degenerate_value_component(lit.datatype().as_str(), lit.language()),
                blake3_field(lit.to_string().as_bytes()),
                Fr::from(TYPE_CODE_LITERAL),
            ]);
            assert_ne!(encode_literal_dual(&lit).unwrap(), degenerate);
        }
    }

    #[test]
    fn hookable_datatype_predicate_agrees_with_the_routing_match() {
        // Pins `is_hookable_datatype` to the arms `encode_literal_dual` actually
        // routes: a datatype the predicate calls hookable MUST reject a lexical
        // its lane rejects (fail-closed), and a datatype it calls non-hookable
        // MUST take the degenerate lane. Adding a lane arm without updating the
        // predicate turns this red.
        for iri in [XSD_INTEGER, XSD_DECIMAL, XSD_DOUBLE] {
            assert!(is_hookable_datatype(iri));
            let bogus = Literal::new_typed_literal("!!", NamedNode::new(iri).unwrap());
            assert!(
                encode_literal_dual(&bogus).is_err(),
                "hookable {iri} must fail closed on a bad lexical"
            );
        }
        for iri in [XSD_STRING, RDF_LANG_STRING, "http://ex/dt"] {
            assert!(!is_hookable_datatype(iri));
            let lit = Literal::new_typed_literal("!!", NamedNode::new(iri).unwrap());
            assert_eq!(
                encode_literal_dual(&lit).unwrap(),
                poseidon2::hash(&[
                    degenerate_value_component(iri, lit.language()),
                    blake3_field(lit.to_string().as_bytes()),
                    Fr::from(TYPE_CODE_LITERAL),
                ]),
            );
        }
    }

    #[test]
    fn unrouted_lane_hooks_cannot_collide_the_degenerate_component() {
        // The §3.2 no-collision obligation for the lanes NOT yet routed (the
        // documented seam). `xsd:boolean` shares the bare `blake3(IRI)`
        // DATATYPE_CONST with its degenerate form, so distinctness there rests on
        // VALUE_NONE differing from both boolean hooks {0, 1}. The dateTime/date
        // lanes fold their epoch scale into the const, so the consts are disjoint
        // outright and the hook value is irrelevant.
        assert_ne!(VALUE_NONE, 0);
        assert_ne!(VALUE_NONE, 1);
        let boolean_iri = crate::dual_leaf_boolean::XSD_BOOLEAN;
        assert!(!is_hookable_datatype(boolean_iri));
        for lexical in ["true", "false"] {
            let lit = Literal::new_typed_literal(lexical, NamedNode::new(boolean_iri).unwrap());
            let real = crate::dual_leaf_boolean::encode_boolean(&lit).unwrap();
            assert_eq!(real.datatype_const, datatype_const(boolean_iri));
            assert_ne!(real.value_component(), degenerate_value_component(boolean_iri, None));
        }
        for (iri, real_const) in [
            (
                crate::dual_leaf_datetime::XSD_DATE_TIME,
                crate::dual_leaf_datetime::datetime_datatype_const(),
            ),
            (
                crate::dual_leaf_datetime::XSD_DATE,
                crate::dual_leaf_datetime::date_datatype_const(),
            ),
        ] {
            assert!(!is_hookable_datatype(iri));
            assert_ne!(real_const, datatype_const(iri), "{iri} const must be scale-folded");
        }
    }

    #[test]
    fn rdf12_triple_terms_are_fail_closed_none() {
        let salt = salt_from_bytes(&[1u8; 32]);
        let inner = Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            Term::NamedNode(NamedNode::new("http://ex/o").unwrap()),
        );
        let quoted = Term::Triple(Box::new(inner.clone()));
        assert_eq!(encode_term_dual(&quoted, &salt).unwrap(), None);
        // A triple whose object is a quoted triple has no leaf either, and the
        // whole-graph builder turns that into UncommittableTerm.
        let outer = Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            quoted,
        );
        assert_eq!(encode_triple_dual(&outer, &salt).unwrap(), None);
        assert_eq!(
            encode_triple_dual(&inner, &salt).unwrap(),
            Some(poseidon2::hash(&[
                encode_term_dual(&Term::NamedNode(NamedNode::new("http://ex/s").unwrap()), &salt)
                    .unwrap()
                    .unwrap(),
                encode_term_dual(&Term::NamedNode(NamedNode::new("http://ex/p").unwrap()), &salt)
                    .unwrap()
                    .unwrap(),
                encode_term_dual(&Term::NamedNode(NamedNode::new("http://ex/o").unwrap()), &salt)
                    .unwrap()
                    .unwrap(),
            ])),
        );
    }

    #[test]
    fn encode_triple_dual_is_position_sensitive() {
        let salt = salt_from_bytes(&[9u8; 32]);
        let s = NamedNode::new("http://ex/s").unwrap();
        let p = NamedNode::new("http://ex/p").unwrap();
        let o = NamedNode::new("http://ex/o").unwrap();
        let t = Triple::new(
            NamedOrBlankNode::NamedNode(s.clone()),
            p.clone(),
            Term::NamedNode(o.clone()),
        );
        let swapped = Triple::new(NamedOrBlankNode::NamedNode(s), o, Term::NamedNode(p));
        assert_ne!(
            encode_triple_dual(&t, &salt).unwrap(),
            encode_triple_dual(&swapped, &salt).unwrap()
        );
    }

    #[test]
    fn ingest_is_fail_closed_on_a_rejected_hookable_lexical() {
        // THE LOAD-BEARING §6 RULE: a hookable-datatyped literal whose lexical the
        // lane rejects ("05"^^xsd:integer is non-canonical) is an ERROR for the
        // WHOLE dual-leaf commitment — NEVER a silent string-lane downgrade.
        let salt = salt_from_bytes(&[7u8; 32]);
        let bad = int_lit("05");
        assert!(matches!(
            encode_literal_dual(&bad),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        let triples = vec![Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            Term::Literal(bad.clone()),
        )];
        let err = commit_triples_dual(&triples, salt).unwrap_err();
        assert!(
            matches!(err, DualCommitError::Leaf(DualLeafError::NonCanonicalValue(_))),
            "expected a fail-closed value-lane rejection, got {err}"
        );
        // And the downgrade the rule forbids is genuinely a DIFFERENT leaf — the
        // rejection is not a no-op dressed up as a guard.
        let downgraded = poseidon2::hash(&[
            degenerate_value_component(XSD_INTEGER, None),
            blake3_field(bad.to_string().as_bytes()),
            Fr::from(TYPE_CODE_LITERAL),
        ]);
        let canonical = encode_literal_dual(&int_lit("5")).unwrap();
        assert_ne!(downgraded, canonical);
        // The canonical sibling commits fine, so the failure is about the lexical,
        // not about the datatype being uncommittable.
        let ok = vec![Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            Term::Literal(int_lit("5")),
        )];
        assert!(commit_triples_dual(&ok, salt).is_ok());
    }

    #[test]
    fn commit_dual_mirrors_commit_rs_ordering_and_the_flat_sponge() {
        // The ordering contract is `commit.rs`'s, unchanged: same RDFC10 canonical
        // form, same leaf INDEX per canonical triple, one flat Poseidon2 sponge
        // over the leaf sequence. Only the per-term leaf SHAPE differs.
        let salt = salt_from_bytes(&[7u8; 32]);
        let g = graph_triples();
        let dual = commit_triples_dual(&g, salt).unwrap();
        let string_canonical = crate::commit::commit_triples(&g, salt).unwrap();

        assert_eq!(dual.canonical.lines, string_canonical.canonical.lines);
        assert_eq!(dual.canonical.triples, string_canonical.canonical.triples);
        assert_eq!(dual.leaves.len(), string_canonical.leaves.len());
        assert_eq!(dual.salt, salt);

        let recomputed: Vec<Fr> = dual
            .canonical
            .triples
            .iter()
            .map(|t| encode_triple_dual(t, &salt).unwrap().unwrap())
            .collect();
        assert_eq!(dual.leaves, recomputed, "leaves must be encode_triple_dual in canonical order");
        assert_eq!(dual.commitment, poseidon2::hash(&recomputed));

        // The two methods are genuinely different commitments over the same graph
        // (the leaf re-base of §3.5) — so a verifier MUST dispatch on zk:scheme.
        assert_ne!(dual.commitment, string_canonical.commitment);
        for (d, s) in dual.leaves.iter().zip(&string_canonical.leaves) {
            assert_ne!(d, s);
        }

        // Input order does not matter (RDFC10 fixes the order), and the salt
        // separates graphs exactly as in the string-canonical pipeline.
        let mut reversed = g.clone();
        reversed.reverse();
        assert_eq!(commit_triples_dual(&reversed, salt).unwrap().commitment, dual.commitment);
        assert_ne!(
            commit_triples_dual(&g, salt_from_bytes(&[8u8; 32])).unwrap().commitment,
            dual.commitment
        );
    }

    #[test]
    fn commit_graph_dual_matches_commit_triples_dual() {
        use sparq_core::Graph;
        let salt = salt_from_bytes(&[5u8; 32]);
        let g = Graph::load_str(
            "<http://ex/s> <http://ex/p> \"o\" .\n<http://ex/s> <http://ex/q> <http://ex/o2> .",
            "turtle",
        )
        .unwrap();
        let from_store = commit_graph_dual(&g, salt).unwrap();
        let from_triples =
            commit_triples_dual(&crate::canon::graph_triples(&g).unwrap(), salt).unwrap();
        assert_eq!(from_store.commitment, from_triples.commitment);
        assert_eq!(from_store.leaves, from_triples.leaves);
        assert_eq!(from_store.canonical.lines, from_triples.canonical.lines);
    }

    #[test]
    fn dual_commit_error_display_is_exact() {
        assert_eq!(
            DualCommitError::UncommittableTerm("http://ex/quoted".to_string()).to_string(),
            "uncommittable term: http://ex/quoted"
        );
        let leaf = DualLeafError::NonCanonicalValue("\"05\"".to_string());
        let leaf_msg = leaf.to_string();
        let wrapped: DualCommitError = leaf.into();
        assert_eq!(
            wrapped.to_string(),
            format!("dual-leaf value lane rejected a literal: {leaf_msg}")
        );
        let canon_err = CanonError::Bridge("boom".to_string());
        let canon_msg = canon_err.to_string();
        let wrapped: DualCommitError = canon_err.into();
        assert!(matches!(wrapped, DualCommitError::Canon(_)));
        assert_eq!(wrapped.to_string(), canon_msg);
    }
}
