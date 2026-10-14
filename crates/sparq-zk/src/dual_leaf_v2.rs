//! `DualLeafV2` — the versioned dual-leaf scheme with injective, domain-framed
//! constant derivation and an out-of-band no-value sentinel.
//!
//! OPT-IN, behind the `dual-leaf` cargo feature (OFF by default), exactly like
//! [`crate::dual_leaf`]. The default `string-canonical` pipeline
//! (`encode.rs`/`commit.rs`) is byte-unchanged, and **every `DualLeafV1`
//! constant, vector and public function in [`crate::dual_leaf`] is unchanged** —
//! V2 is added alongside it and reuses V1's canonical-lexical parsers verbatim
//! (three of which widened from private to `pub(crate)` for that reuse, with no
//! behaviour change), so the two versions can never disagree about which lexical
//! forms are committable.
//!
//! # What V2 changes, and why (`research/zk-field-native-encoding.md` §14.1)
//!
//! V2 keeps the §3.1/§3.2 leaf SHAPES of V1. Only the slot-1 constants and the
//! slot-0 no-value sentinel are derived differently:
//!
//! ```text
//! lane constant (iri, params) = field_from_hash_bytes(
//!     blake3::derive_key(LANE_CONTEXT, u32_be(len iri) ‖ iri ‖ u32_be(len params) ‖ params))
//! degenerate datatype const   = blake3_field(datatype string)      // the plain fold
//! language constant           = field_from_hash_bytes(
//!     blake3::derive_key(LANG_CONTEXT, lowercased tag))
//! no-value sentinel           = field_from_hash_bytes(
//!     blake3::derive_key(VALUE_NONE_CONTEXT, …))                   // band-excluded, §2 below
//! ```
//!
//! with `params` `""` for integer and double, `"scale=<fd>"` for decimal,
//! `"epochscale=3"` for dateTime and date, and `"bit"` for boolean.
//!
//! ## 1. Separate BLAKE3 hash domains, for every byte string
//!
//! A value-lane constant is derived in BLAKE3's **key-derivation mode**, which is
//! a hash domain separate from BLAKE3's **plain mode**; the degenerate fold a
//! string-lane leaf carries stays in plain mode. So a lane constant differs from
//! the degenerate fold of **any** byte string — valid RFC 3987 IRI or not. That
//! keeps the argument independent of whether every path that builds terms
//! validates the datatype as an IRI: the encoder accepts arbitrary UTF-8
//! (including spaces and NUL bytes) in the datatype position.
//!
//! Inside the key-derivation domain, the `u32_be` **length framing** makes
//! `(iri, params)` injective for arbitrary byte strings: no concatenation of one
//! pair can be read as another. So two distinct lanes — a different datatype, or
//! the same datatype at a different scale — cannot share a constant.
//!
//! Both statements are properties under BLAKE3's mode domain separation and,
//! after the 248-bit [`field_from_hash_bytes`] reduction, under its
//! collision/preimage resistance. They are **not** unconditional facts, and they
//! are not audited (see *Honesty* below).
//!
//! What this separation does is keep the CONSTANTS apart. It is not what keeps a
//! string-lane leaf from opening on a value-lane member: `datatype_const` is an
//! UNPINNED public input of those members, so that job falls to the out-of-band
//! sentinel of §2 below (a degenerate handle is outside every member's
//! range/bit constraint) plus the relying party's own pinning of
//! `datatype_const` to the expected lane constant (the sq-j506 verifier wiring).
//! See [`lane_const`].
//!
//! ## 2. The out-of-band no-value sentinel (§14.1's reserved-tag requirement)
//!
//! §14.1 requires the reserved no-value tag to be a domain-separated
//! `blake3_field` constant **asserted outside the reachable value-handle band**
//!
//! ```text
//! [0, 2^64)  ∪  (p − 2^64, p)        (p = the BN254 scalar-field modulus)
//! ```
//!
//! because every shipped value-lane member range-decomposes its handle to `u64`
//! and the signed lanes fold the sign by field negation, so that band is exactly
//! the set of values a member can range-check. [`value_none`] derives the
//! sentinel under its own key-derivation context, re-deriving with a `u32`
//! counter until the result lies outside the band, and [`is_in_handle_band`] is
//! the predicate this module's tests assert it against. V1's `VALUE_NONE = 2`
//! lies inside the band, so V1 retains the in-band sentinel and V2 is what
//! discharges this §14.1 requirement.
//!
//! The §14.1 **`LANG_NONE` separation assumption is NOT discharged here**: the
//! no-language slot still folds the V1 [`crate::dual_leaf::LANG_NONE`] tag, whose
//! separation from a derived language constant remains probabilistic (a hash
//! reduced into the field can equal `1`). That assumption stays open exactly as
//! §14.3 records it.
//!
//! ## 3. The bit-domain boolean lane
//!
//! `xsd:boolean`'s V2 lane constant carries the `"bit"` params
//! ([`boolean_lane_const`]), and the dedicated `filter_value_dl_bool` Noir member
//! additionally constrains both the witnessed handle and the public bound to
//! `{0, 1}` in-circuit — so a handle of `2` denotes nothing and the member itself
//! refuses it. The boolean lane remains part of the documented UNROUTED seam for
//! whole-graph commitments (see [`is_hookable_datatype`]): V2 routes exactly the
//! datatypes V1 routes.
//!
//! # Honesty (load-bearing — unchanged from V1)
//!
//! V2 inherits the dual-leaf method's documented **INV-VL downgrade**:
//! value↔lexical agreement on the value-FILTER lane rests on TRUSTED-ISSUER
//! HONESTY, not machine enforcement (#769 accepted at research grade; gap CR-G8 /
//! sq-qhy4). The whole ZK estate is **NOT externally audited**. Nothing here is a
//! soundness or privacy guarantee.

use crate::canon::{self, CanonicalGraph};
use crate::dual_leaf::{
    canonical_decimal_scaled, canonical_nonneg_u64, parse_xsd_double_bits, DualCommitError,
    DualGraphCommitment, DualLeafComponents, DualLeafError, LANG_NONE, XSD_DECIMAL, XSD_DOUBLE,
    XSD_INTEGER,
};
use crate::dual_leaf_boolean::{canonical_boolean, XSD_BOOLEAN};
use crate::dual_leaf_datetime::{
    canonical_date_scaled, canonical_datetime_scaled, EPOCH_SCALE_FS, XSD_DATE, XSD_DATE_TIME,
};
use crate::encode::{TYPE_CODE_BLANK_NODE, TYPE_CODE_IRI, TYPE_CODE_LITERAL};
use crate::field::{field_from_hash_bytes, field_to_be_bytes_32, Fr};
use crate::poseidon2;
use oxrdf::{Literal, NamedOrBlankNode, Term, Triple};

/// The BLAKE3 key-derivation context of every V2 VALUE-LANE constant. Changing it
/// re-bases every V2 lane constant, which is exactly what a further version bump
/// would do.
pub const LANE_CONTEXT: &str = "sparq dual-leaf v2 lane constant";

/// The BLAKE3 key-derivation context of every V2 LANGUAGE constant.
pub const LANG_CONTEXT: &str = "sparq dual-leaf v2 language constant";

/// The BLAKE3 key-derivation context of the V2 reserved no-value sentinel.
pub const VALUE_NONE_CONTEXT: &str = "sparq dual-leaf v2 value-none";

/// The V2 lane params of the `xsd:integer` lane: none — the handle is the integer
/// value itself, with no scale or sub-domain to name.
pub const LANE_PARAMS_INTEGER: &str = "";

/// The V2 lane params of the `xsd:double` lane: none — the handle is the
/// canonical IEEE-754 bit pattern, with no scale to name.
pub const LANE_PARAMS_DOUBLE: &str = "";

/// The V2 lane params of the `xsd:boolean` lane: the `bit` sub-domain, whose
/// `{0, 1}` constraint the dedicated `filter_value_dl_bool` member enforces
/// in-circuit.
pub const LANE_PARAMS_BOOLEAN: &str = "bit";

fn blake3_field(bytes: &[u8]) -> Fr {
    field_from_hash_bytes(blake3::hash(bytes).as_bytes())
}

/// Length-frame a sequence of byte strings: `u32_be(len) ‖ bytes` per part.
///
/// # Panics
/// If a part is longer than `u32::MAX` bytes — the length prefix could not then
/// be exact and the injectivity argument would not hold. No RDF term reaches that
/// size.
fn framed(parts: &[&[u8]]) -> Vec<u8> {
    let mut buf = Vec::new();
    for part in parts {
        let len = u32::try_from(part.len()).expect("a framed part must fit a u32 length prefix");
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(part);
    }
    buf
}

/// Derive a V2 constant in BLAKE3's key-derivation mode under `context`, reduced
/// into the field by [`field_from_hash_bytes`].
fn derive(context: &str, key_material: &[u8]) -> Fr {
    field_from_hash_bytes(&blake3::derive_key(context, key_material))
}

/// The V2 `DATATYPE_CONST` of a VALUE LANE:
/// `derive_key(`[`LANE_CONTEXT`]`, u32_be(len iri) ‖ iri ‖ u32_be(len params) ‖ params)`.
///
/// `params` names the lane's sub-domain or scale (`""`, `"scale=2"`,
/// `"epochscale=3"`, `"bit"`) and is a separate length-framed part, so a lane
/// constant is injective in `(datatype, params)` jointly: a different datatype,
/// or the same datatype at a different scale, cannot share a constant.
///
/// Because this is the key-derivation mode and [`degenerate_datatype_const`] is
/// the plain mode, a lane constant also differs from the degenerate fold of any
/// byte string. Both are properties under BLAKE3's mode domain separation and,
/// after the 248-bit field reduction, its collision resistance.
///
/// # What the split does, and what actually keeps the lanes apart
///
/// This separates the CONSTANTS: a lane constant and the string lane's plain fold
/// are distinct field elements, so the two lanes are distinguishable. It does
/// NOT, on its own, stop a string-lane leaf opening on a value-lane member —
/// `datatype_const` is a PUBLIC INPUT those members take UNPINNED, so a prover may
/// present the degenerate fold as if it were the lane constant and the leaf
/// binding would still be satisfied.
///
/// What refuses such a leaf is slot 0: a degenerate `value_component` carries
/// [`value_none`], which lies OUTSIDE the reachable handle band, so every shipped
/// value-lane member's `u64` range decomposition — and `filter_value_dl_bool`'s
/// `{0, 1}` constraint — rejects the handle it would have to witness. On top of
/// that, a relying party must PIN `datatype_const` to the lane constant it
/// expects; that verifier-side binding is the sq-j506 wiring, not a property of
/// this constant.
pub fn lane_const(datatype: &str, params: &str) -> Fr {
    derive(
        LANE_CONTEXT,
        &framed(&[datatype.as_bytes(), params.as_bytes()]),
    )
}

/// The V2 `DATATYPE_CONST` of a DEGENERATE literal (`xsd:string`,
/// `rdf:langString`, every opaque datatype, and the unrouted lanes): the PLAIN
/// `blake3_field(datatype string)` — deliberately identical to
/// [`crate::dual_leaf::datatype_const`].
///
/// It does not need its own derivation: what the degenerate fold must be separate
/// from is the set of VALUE-LANE constants, and [`lane_const`] achieves that by
/// living in a different BLAKE3 mode. Keeping the plain fold here means V2 adds
/// exactly one derivation rule rather than two, and the degenerate
/// `value_component` still differs from V1's because slot 0 carries
/// [`value_none`] instead of V1's in-band tag.
pub fn degenerate_datatype_const(datatype: &str) -> Fr {
    crate::dual_leaf::datatype_const(datatype)
}

/// The V2 `xsd:integer` lane constant — [`lane_const`] with
/// [`LANE_PARAMS_INTEGER`].
pub fn integer_lane_const() -> Fr {
    lane_const(XSD_INTEGER, LANE_PARAMS_INTEGER)
}

/// The V2 `xsd:double` lane constant — [`lane_const`] with
/// [`LANE_PARAMS_DOUBLE`].
pub fn double_lane_const() -> Fr {
    lane_const(XSD_DOUBLE, LANE_PARAMS_DOUBLE)
}

/// The V2 `xsd:decimal` lane constant at a fixed fraction-digit count `fd` —
/// [`lane_const`] with `params = "scale=<fd>"`. The canonical-SCALE bind of
/// `crate::dual_leaf::decimal_datatype_const`, carried as a framed part instead
/// of an `@scale=` suffix on the datatype bytes.
pub fn decimal_lane_const(fd: u32) -> Fr {
    lane_const(XSD_DECIMAL, &format!("scale={}", fd))
}

/// The V2 `xsd:boolean` lane constant — [`lane_const`] with
/// [`LANE_PARAMS_BOOLEAN`], the bit sub-domain of the dedicated
/// `filter_value_dl_bool` member.
pub fn boolean_lane_const() -> Fr {
    lane_const(XSD_BOOLEAN, LANE_PARAMS_BOOLEAN)
}

/// The V2 `xsd:dateTime` lane constant — [`lane_const`] with
/// `params = "epochscale=3"` (the lane-fixed sub-second scale
/// `crate::dual_leaf_datetime::EPOCH_SCALE_FS`).
pub fn datetime_lane_const() -> Fr {
    lane_const(XSD_DATE_TIME, &format!("epochscale={}", EPOCH_SCALE_FS))
}

/// The V2 `xsd:date` lane constant — [`lane_const`] with
/// `params = "epochscale=3"`. A date's handle is numerically equal to the
/// dateTime handle of its starting instant, so this constant is the only thing
/// separating the two lanes: the two constants differ because BLAKE3 separates
/// them (see [`lane_const`]), and the two LEAVES differ under Poseidon2's
/// collision resistance. Neither is an audited soundness claim.
pub fn date_lane_const() -> Fr {
    lane_const(XSD_DATE, &format!("epochscale={}", EPOCH_SCALE_FS))
}

/// The V2 `LANG_CONST` of a language tag:
/// `derive_key(`[`LANG_CONTEXT`]`, ASCII-lowercased tag)`.
///
/// Its own key-derivation context puts a language constant in a hash domain
/// separate from both the lane constants and the plain degenerate fold, so no
/// language tag can produce a datatype or lane constant for any byte string.
/// The tag is lowercased because RDF language tags are case-insensitive
/// (`oxrdf::Literal::language` already yields a lowercased tag; the explicit
/// lowercasing makes the constant independent of that).
///
/// This does NOT discharge the §14.1 `LANG_NONE` assumption — see the module docs
/// §2.
pub fn lang_const(language: &str) -> Fr {
    derive(LANG_CONTEXT, language.to_ascii_lowercase().as_bytes())
}

/// Whether `f` lies in the REACHABLE value-handle band
/// `[0, 2^64) ∪ (p − 2^64, p)` — the set of field elements a shipped value-lane
/// member can range-check (`assert_max_bit_size::<64>()`, with the signed lanes
/// folding the sign by field negation).
///
/// A reserved tag inside this band is a value some member could also range-check
/// as a real handle; [`value_none`] is derived to fall OUTSIDE it, and a test
/// asserts that.
pub fn is_in_handle_band(f: &Fr) -> bool {
    fn fits_u64(f: &Fr) -> bool {
        // A u64 occupies the low 8 big-endian bytes of the 32-byte word.
        field_to_be_bytes_32(f)[..24].iter().all(|b| *b == 0)
    }
    // `fits_u64(f)` covers `[0, 2^64)`; `fits_u64(-f)` covers `(p − 2^64, p)`
    // (and, for `f == 0`, is the same case as the first).
    fits_u64(f) || fits_u64(&-*f)
}

/// The V2 reserved **no-value** sentinel:
/// `derive_key(`[`VALUE_NONE_CONTEXT`]`, "")`, re-derived with a `u32` counter
/// part until the result lies OUTSIDE the reachable handle band
/// ([`is_in_handle_band`]).
///
/// This is what discharges the `research/zk-field-native-encoding.md` §14.1
/// reserved-tag requirement: the tag is a domain-separated constant whose band
/// exclusion is a CHECKABLE invariant (asserted by this module's tests) rather
/// than an argument about where a hash output lands. V1's
/// `crate::dual_leaf::VALUE_NONE` retains the in-band value.
///
/// A re-derivation is expected never to be needed (the band is a vanishing
/// fraction of the field); the counter exists so the construction is TOTAL rather
/// than resting on that expectation.
pub fn value_none() -> Fr {
    let first = derive(VALUE_NONE_CONTEXT, &[]);
    if !is_in_handle_band(&first) {
        return first;
    }
    for counter in 1u32..=u32::MAX {
        let candidate = derive(VALUE_NONE_CONTEXT, &counter.to_be_bytes());
        if !is_in_handle_band(&candidate) {
            return candidate;
        }
    }
    unreachable!("no band-excluded value-none candidate in 2^32 re-derivations")
}

/// The V2 flat slot-0 sentinel for IRI and blank-node leaves — the same reserved
/// tag as [`value_none`], used unfolded, exactly as V1's
/// `crate::dual_leaf::NO_VALUE` mirrors its `VALUE_NONE`. Non-literals have
/// neither a datatype nor a language to fold, and `TYPE_CODE` in slot 2 separates
/// the term classes regardless.
pub fn no_value() -> Fr {
    value_none()
}

/// Whether a datatype is on a HOOKABLE value lane for V2 WHOLE-GRAPH
/// commitments — i.e. whether [`encode_literal`] routes it to a value-lane
/// encoder (and so applies the §6 fail-closed rule) rather than to the degenerate
/// lane.
///
/// **V2 routes exactly what V1 routes** (`xsd:integer`, `xsd:decimal`,
/// `xsd:double`). The `xsd:boolean` and `xsd:dateTime`/`xsd:date` lanes remain
/// the documented UNROUTED seam, because routing them narrows what a committed
/// graph may CONTAIN (an XSD-legal but non-canonical `"1"^^xsd:boolean` becomes
/// uncommittable) and re-bases those leaves — a separate decision from the
/// versioned constant derivation. Their standalone encoders ([`encode_boolean`] /
/// [`encode_datetime`] / [`encode_date`]) exist and feed the value-lane FILTER
/// members; they just do not participate in whole-graph leaf routing.
///
/// Under V2 the unrouted seam needs no value-dependent argument: a degenerate
/// fold and a lane constant are in separate BLAKE3 hash domains for EVERY
/// datatype string, so an unrouted lane's degenerate leaf does not open on its
/// lane constant whatever hook a member would accept.
pub fn is_hookable_datatype(datatype: &str) -> bool {
    matches!(datatype, XSD_INTEGER | XSD_DECIMAL | XSD_DOUBLE)
}

/// Encodes an `xsd:integer` literal under V2 — V1's fail-closed canonical-lexical
/// predicate (`crate::dual_leaf::encode_literal`'s, shared verbatim) with the V2
/// [`integer_lane_const`].
pub fn encode_integer(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_INTEGER {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    let value = canonical_nonneg_u64(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: Fr::from(value),
        datatype_const: integer_lane_const(),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Encodes an `xsd:double` literal under V2 — V1's canonical-lexical predicate
/// and canonical IEEE-bit fold, with the V2 [`double_lane_const`].
pub fn encode_double(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_DOUBLE {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    let bits = parse_xsd_double_bits(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: Fr::from(crate::dual_leaf::canonical_f64_bits(bits)),
        datatype_const: double_lane_const(),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Encodes an `xsd:decimal` literal under V2 — V1's canonical-lexical predicate
/// and signed scaled magnitude, with the V2 [`decimal_lane_const`] at the lexical
/// form's fraction-digit count.
pub fn encode_decimal(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_DECIMAL {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    let (neg, scaled, fd) = canonical_decimal_scaled(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: signed_hook(neg, scaled),
        datatype_const: decimal_lane_const(fd),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Encodes an `xsd:boolean` literal under V2 — V1's canonical-lexical predicate
/// (only `"true"`/`"false"`; the XSD-legal `"1"`/`"0"` are fail-closed rejected)
/// with the V2 bit-domain [`boolean_lane_const`]. Pairs with the dedicated
/// `filter_value_dl_bool` member, which additionally constrains the handle and
/// the bound to `{0, 1}` in-circuit.
pub fn encode_boolean(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_BOOLEAN {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    let value = canonical_boolean(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: Fr::from(value),
        datatype_const: boolean_lane_const(),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Encodes an `xsd:dateTime` literal under V2 — V1's §13.4 fail-closed
/// `Z`-timezoned canonical predicate and signed scaled epoch, with the V2
/// [`datetime_lane_const`].
pub fn encode_datetime(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_DATE_TIME {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    let (neg, scaled) = canonical_datetime_scaled(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: signed_hook(neg, scaled),
        datatype_const: datetime_lane_const(),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Encodes an `xsd:date` literal under V2 — V1's §13.3/§13.4 predicate (the
/// starting instant's scaled epoch) with the V2 [`date_lane_const`].
pub fn encode_date(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_DATE {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    let (neg, scaled) = canonical_date_scaled(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: signed_hook(neg, scaled),
        datatype_const: date_lane_const(),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Fold a `(neg, magnitude)` pair into one field handle by field negation — the
/// shape the signed value-lane members recompute. `(true, 0)` is never produced:
/// the canonical predicates reject a negative zero.
fn signed_hook(neg: bool, magnitude: u64) -> Fr {
    let mag = Fr::from(magnitude);
    if neg {
        -mag
    } else {
        mag
    }
}

/// The V2 DEGENERATE, datatype-folded `value_component`:
/// `h3(value_none(), degenerate_datatype_const(datatype), LANG_CONST)`, with
/// `LANG_CONST` the V2 [`lang_const`] of the language tag for a language-tagged
/// literal and the V1 `LANG_NONE` sentinel otherwise.
pub fn degenerate_value_component(datatype: &str, language: Option<&str>) -> Fr {
    let lang = match language {
        Some(l) => lang_const(l),
        None => Fr::from(LANG_NONE),
    };
    poseidon2::hash(&[value_none(), degenerate_datatype_const(datatype), lang])
}

/// Encodes a literal to its V2 leaf, routing by datatype exactly as V1 does
/// ([`is_hookable_datatype`]).
///
/// **Fail-closed (§6):** a hookable-datatyped literal whose lexical form the lane
/// encoder rejects returns that encoder's [`DualLeafError`]; it is NEVER
/// downgraded onto the degenerate lane.
pub fn encode_literal(literal: &Literal) -> Result<Fr, DualLeafError> {
    let datatype = literal.datatype();
    let components = match datatype.as_str() {
        XSD_INTEGER => Some(encode_integer(literal)?),
        XSD_DECIMAL => Some(encode_decimal(literal)?),
        XSD_DOUBLE => Some(encode_double(literal)?),
        // The `xsd:boolean` / `xsd:dateTime` / `xsd:date` lanes are the
        // documented unrouted seam — see `is_hookable_datatype`. Adding an arm
        // here MUST also extend that predicate (a test pins the two together).
        _ => None,
    };
    Ok(match components {
        Some(c) => c.leaf(),
        None => poseidon2::hash(&[
            degenerate_value_component(datatype.as_str(), literal.language()),
            // EXACTLY the string-canonical `h_s` over the canonical N-Triples
            // token, byte-identical to `encode.rs`'s and to V1's.
            blake3_field(literal.to_string().as_bytes()),
            Fr::from(TYPE_CODE_LITERAL),
        ]),
    })
}

/// Encodes a term to its V2 leaf under a graph salt — the V2 mirror of
/// `crate::dual_leaf::encode_term_dual`, with the same §3.2 tuple shapes and the
/// same salt-scoped blank-node inner (the Q6 cross-graph correlation guard).
///
/// `Ok(None)` for an RDF 1.2 triple term (outside the committed data model, so
/// callers fail closed); `Err` for a hookable literal the value lane rejects.
pub fn encode_term(term: &Term, salt: &Fr) -> Result<Option<Fr>, DualLeafError> {
    Ok(match term {
        Term::NamedNode(n) => Some(poseidon2::hash(&[
            no_value(),
            blake3_field(n.as_str().as_bytes()),
            Fr::from(TYPE_CODE_IRI),
        ])),
        Term::BlankNode(b) => {
            let inner = poseidon2::hash(&[*salt, blake3_field(b.as_str().as_bytes())]);
            Some(poseidon2::hash(&[
                no_value(),
                inner,
                Fr::from(TYPE_CODE_BLANK_NODE),
            ]))
        }
        Term::Literal(l) => Some(encode_literal(l)?),
        Term::Triple(_) => None,
    })
}

/// Encodes a triple to its V2 commitment leaf `h3(Enc(s), Enc(p), Enc(o))` — the
/// same outer shape and argument order as `crate::encode::encode_triple` and the
/// V1 dual-leaf builder.
pub fn encode_triple(triple: &Triple, salt: &Fr) -> Result<Option<Fr>, DualLeafError> {
    let s = match &triple.subject {
        NamedOrBlankNode::NamedNode(n) => encode_term(&Term::NamedNode(n.clone()), salt)?,
        NamedOrBlankNode::BlankNode(b) => encode_term(&Term::BlankNode(b.clone()), salt)?,
    };
    let p = encode_term(&Term::NamedNode(triple.predicate.clone()), salt)?;
    let o = encode_term(&triple.object, salt)?;
    Ok(match (s, p, o) {
        (Some(s), Some(p), Some(o)) => Some(poseidon2::hash(&[s, p, o])),
        _ => None,
    })
}

/// Canonicalizes and commits one named graph's content under `salt` using the V2
/// leaf shape — the V2 mirror of `crate::dual_leaf::commit_triples_dual`. The
/// ordering contract is `commit.rs`'s, unchanged: RDFC10-canonicalize, one leaf
/// per canonical triple in canonical N-Quads order, then ONE flat Poseidon2
/// sponge over the leaf sequence.
pub fn commit_triples(
    triples: &[Triple],
    salt: Fr,
) -> Result<DualGraphCommitment, DualCommitError> {
    commit_canonical(canon::canonicalize_triples(triples)?, salt)
}

/// Commits the content of a `sparq_core::Graph` under `salt` using the V2 leaf
/// shape — the V2 mirror of `crate::dual_leaf::commit_graph_dual`.
pub fn commit_graph(
    g: &sparq_core::Graph,
    salt: Fr,
) -> Result<DualGraphCommitment, DualCommitError> {
    commit_canonical(canon::canonicalize_graph_content(g)?, salt)
}

fn commit_canonical(
    canonical: CanonicalGraph,
    salt: Fr,
) -> Result<DualGraphCommitment, DualCommitError> {
    let mut leaves = Vec::with_capacity(canonical.triples.len());
    for t in &canonical.triples {
        // `?` on the value-lane rejection is the §6 FAIL-CLOSED ingest rule: the
        // whole commitment fails, the offending literal is never downgraded.
        let leaf = encode_triple(t, &salt)?
            .ok_or_else(|| DualCommitError::UncommittableTerm(t.to_string()))?;
        leaves.push(leaf);
    }
    let commitment = poseidon2::hash(&leaves);
    Ok(DualGraphCommitment { canonical, leaves, commitment, salt })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dual_leaf;
    use crate::encode::salt_from_bytes;
    use oxrdf::{BlankNode, NamedNode};
    use std::collections::HashSet;

    const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
    const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

    fn typed(lexical: &str, datatype: &str) -> Literal {
        Literal::new_typed_literal(lexical, NamedNode::new_unchecked(datatype))
    }

    /// Datatype strings the disjointness properties are swept over. Deliberately
    /// NOT all valid IRIs: the encoder accepts arbitrary UTF-8 here. The list
    /// covers suffixes such as `@scale=`, `@epochscale=` and `%20`, strings with
    /// spaces, `=` and embedded NUL bytes, the derivation CONTEXT strings
    /// themselves, and strings that spell out a framed part.
    fn datatype_corpus() -> Vec<String> {
        let mut v: Vec<String> = vec![
            XSD_INTEGER.to_string(),
            XSD_DECIMAL.to_string(),
            XSD_DOUBLE.to_string(),
            XSD_BOOLEAN.to_string(),
            XSD_DATE_TIME.to_string(),
            XSD_DATE.to_string(),
            XSD_STRING.to_string(),
            RDF_LANG_STRING.to_string(),
            "http://ex/dt".to_string(),
            String::new(),
            " ".to_string(),
            "=".to_string(),
            "bit".to_string(),
            "scale=2".to_string(),
            "epochscale=3".to_string(),
            "a datatype with spaces".to_string(),
            "\0".to_string(),
            "pre\0post".to_string(),
            // The derivation contexts themselves, and concatenations with them.
            LANE_CONTEXT.to_string(),
            LANG_CONTEXT.to_string(),
            VALUE_NONE_CONTEXT.to_string(),
            format!("{}{}", LANE_CONTEXT, XSD_BOOLEAN),
            format!("{}{}", XSD_BOOLEAN, LANE_CONTEXT),
            // A string that spells out a framed (iri, params) part sequence.
            String::from_utf8_lossy(&framed(&[XSD_BOOLEAN.as_bytes(), b"bit"])).into_owned(),
            "日本語".to_string(),
        ];
        // The suffix shapes a bare-bytes derivation would have folded.
        for base in [XSD_DECIMAL, XSD_DATE_TIME, XSD_DATE, XSD_BOOLEAN] {
            for suffix in [
                "@scale=0",
                "@scale=2",
                "@epochscale=3",
                "@domain=bit",
                " domain=bit",
                "%20",
                "@",
                "=",
                "\0bit",
            ] {
                v.push(format!("{}{}", base, suffix));
            }
        }
        v
    }

    fn lane_params_corpus() -> Vec<String> {
        let mut v = vec![
            LANE_PARAMS_INTEGER.to_string(),
            LANE_PARAMS_BOOLEAN.to_string(),
            "epochscale=3".to_string(),
            " ".to_string(),
            "\0".to_string(),
            LANE_CONTEXT.to_string(),
        ];
        for fd in [0u32, 1, 2, 9] {
            v.push(format!("scale={}", fd));
        }
        v
    }

    /// The NAMED lane constants — the ones a member actually carries as a public
    /// input today.
    fn named_lane_constants() -> Vec<Fr> {
        let mut v = vec![
            integer_lane_const(),
            double_lane_const(),
            boolean_lane_const(),
            datetime_lane_const(),
            date_lane_const(),
        ];
        for fd in 0u32..=9 {
            v.push(decimal_lane_const(fd));
        }
        v
    }

    /// Every lane constant reachable from the corpus — the named ones plus
    /// `lane_const(d, p)` over the whole `(datatype, params)` sweep.
    fn all_lane_constants() -> Vec<Fr> {
        let mut v = named_lane_constants();
        for d in datatype_corpus() {
            for p in lane_params_corpus() {
                v.push(lane_const(&d, &p));
            }
        }
        v
    }

    // ---- §3.1 the two separation properties ---------------------------------

    #[test]
    fn lane_constant_is_the_documented_derivation() {
        // Pin the exact derivation: key-derivation mode under LANE_CONTEXT over
        // u32_be(len iri) ‖ iri ‖ u32_be(len params) ‖ params. A change to the
        // layout or the context re-bases every V2 lane constant, so this is the
        // byte contract rather than a restatement.
        let mut material = Vec::new();
        material.extend_from_slice(&(XSD_BOOLEAN.len() as u32).to_be_bytes());
        material.extend_from_slice(XSD_BOOLEAN.as_bytes());
        material.extend_from_slice(&3u32.to_be_bytes());
        material.extend_from_slice(b"bit");
        assert_eq!(
            boolean_lane_const(),
            field_from_hash_bytes(&blake3::derive_key(LANE_CONTEXT, &material))
        );
        // And the degenerate fold is the PLAIN mode over the bare bytes — V1's.
        assert_eq!(
            degenerate_datatype_const(XSD_BOOLEAN),
            blake3_field(XSD_BOOLEAN.as_bytes())
        );
        assert_eq!(
            degenerate_datatype_const(XSD_BOOLEAN),
            dual_leaf::datatype_const(XSD_BOOLEAN)
        );
    }

    #[test]
    fn lane_and_degenerate_domains_are_disjoint_for_every_byte_string() {
        // THE HEADLINE PROPERTY. For EVERY datatype string in the corpus — valid
        // IRI or not, including suffixed, spaced, NUL-bearing, context-equal and
        // framed-part-shaped strings — no degenerate fold equals ANY lane
        // constant, over every lane params value. The separation is the BLAKE3
        // mode (key-derivation vs plain), so it does not depend on what bytes a
        // datatype string carries.
        let datatypes = datatype_corpus();
        let lanes: HashSet<Fr> = all_lane_constants().into_iter().collect();
        let named = named_lane_constants();
        assert!(!datatypes.is_empty() && !lanes.is_empty());
        for d in &datatypes {
            // Direct string-API call, exactly as a caller with arbitrary bytes
            // would make it.
            let fold = degenerate_datatype_const(d);
            assert!(
                !lanes.contains(&fold),
                "degenerate fold of {:?} collided a lane constant",
                d
            );
            // And the whole degenerate component: it must not equal the value
            // component a member would recompute on a NAMED lane for a reachable
            // hook — including the boolean hook `2` an unconstrained integer
            // member would have accepted.
            for lang in [None, Some("en")] {
                let component = degenerate_value_component(d, lang);
                for l in &named {
                    for hook in [0u64, 1, 2, u64::MAX] {
                        assert_ne!(
                            component,
                            poseidon2::hash(&[Fr::from(hook), *l, Fr::from(LANG_NONE)]),
                            "{:?}: degenerate component opened on a lane component",
                            d
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_natively_loaded_non_iri_datatype_is_separated_too() {
        // The real ingest path, not a synthetic string: sparq-core's native
        // N-Triples loader accepts an escaped space in the datatype position and
        // stores it unvalidated, so the committer sees "http://ex/dt x". Its
        // degenerate fold must still differ from every lane constant. (Whether the
        // commit path would then reject that graph is NOT the property here.)
        let g = sparq_core::Graph::load_str(
            "<http://ex/s> <http://ex/p> \"v\"^^<http://ex/dt\\u0020x> .\n",
            "ntriples",
        )
        .unwrap();
        let triples = canon::graph_triples(&g).unwrap();
        let literal = triples
            .iter()
            .find_map(|t| match &t.object {
                Term::Literal(l) => Some(l.clone()),
                _ => None,
            })
            .expect("the loaded graph has a literal object");
        let datatype = literal.datatype().as_str().to_string();
        assert_eq!(datatype, "http://ex/dt x");
        assert!(
            NamedNode::new(&datatype).is_err(),
            "the loaded datatype must genuinely be an invalid IRI for this test to bite"
        );
        let lanes: HashSet<Fr> = all_lane_constants().into_iter().collect();
        assert!(!lanes.contains(&degenerate_datatype_const(&datatype)));
        // The literal encodes on the degenerate lane, as a non-hookable datatype.
        assert!(!is_hookable_datatype(&datatype));
        assert_eq!(
            encode_literal(&literal).unwrap(),
            poseidon2::hash(&[
                degenerate_value_component(&datatype, None),
                blake3_field(literal.to_string().as_bytes()),
                Fr::from(TYPE_CODE_LITERAL),
            ])
        );
    }

    #[test]
    fn lane_constants_are_injective_in_their_framed_parts() {
        // Inside the key-derivation domain, the length framing makes
        // (datatype, params) injective over arbitrary byte strings.
        let datatypes = datatype_corpus();
        let params = lane_params_corpus();
        let mut seen = HashSet::new();
        for d in &datatypes {
            for p in &params {
                assert!(
                    seen.insert(lane_const(d, p)),
                    "two distinct (datatype, params) pairs share a lane constant: {:?} {:?}",
                    d,
                    p
                );
            }
        }
        // The split point between the two parts is unambiguous: a datatype ending
        // in what a params string begins with does NOT alias.
        assert_ne!(lane_const("ab", "c"), lane_const("a", "bc"));
        assert_ne!(lane_const("a\0b", ""), lane_const("a", "b"));
        assert_ne!(lane_const("", "ab"), lane_const("ab", ""));
    }

    #[test]
    fn language_constants_live_in_their_own_domain() {
        // A language constant is derived under its own key-derivation context, so
        // it is separate from both the lane constants and the plain degenerate
        // fold — and it is case-insensitive on the tag (RDF language tags are).
        let datatypes = datatype_corpus();
        let lanes: HashSet<Fr> = all_lane_constants().into_iter().collect();
        for tag in ["en", "EN", "en-GB", "", " ", "\0", "bit", LANG_CONTEXT] {
            let l = lang_const(tag);
            assert!(!lanes.contains(&l), "lang const of {tag:?} collided a lane constant");
            for d in &datatypes {
                assert_ne!(l, degenerate_datatype_const(d));
            }
        }
        assert_eq!(lang_const("en"), lang_const("EN"));
        assert_eq!(lang_const("en-GB"), lang_const("en-gb"));
        assert_ne!(lang_const("en"), lang_const("fr"));
        // And it is NOT the V1 plain-fold language constant.
        assert_ne!(lang_const("en"), dual_leaf::lang_const("en"));
    }

    #[test]
    fn every_named_lane_constant_differs_from_its_v1_sibling() {
        assert_eq!(integer_lane_const(), lane_const(XSD_INTEGER, ""));
        assert_eq!(double_lane_const(), lane_const(XSD_DOUBLE, ""));
        assert_eq!(decimal_lane_const(2), lane_const(XSD_DECIMAL, "scale=2"));
        assert_eq!(boolean_lane_const(), lane_const(XSD_BOOLEAN, "bit"));
        assert_eq!(datetime_lane_const(), lane_const(XSD_DATE_TIME, "epochscale=3"));
        assert_eq!(date_lane_const(), lane_const(XSD_DATE, "epochscale=3"));

        assert_ne!(integer_lane_const(), dual_leaf::datatype_const(XSD_INTEGER));
        assert_ne!(double_lane_const(), dual_leaf::datatype_const(XSD_DOUBLE));
        assert_ne!(decimal_lane_const(2), dual_leaf::decimal_datatype_const(2));
        assert_ne!(
            datetime_lane_const(),
            crate::dual_leaf_datetime::datetime_datatype_const()
        );
        assert_ne!(
            date_lane_const(),
            crate::dual_leaf_datetime::date_datatype_const()
        );
        // Lanes stay distinct from each other, and scales from each other.
        assert_ne!(datetime_lane_const(), date_lane_const());
        assert_ne!(decimal_lane_const(1), decimal_lane_const(2));
        assert_ne!(integer_lane_const(), double_lane_const());
    }

    // ---- §3.3 the out-of-band sentinel --------------------------------------

    #[test]
    fn value_none_lies_outside_the_handle_band() {
        // THE §14.1 REQUIREMENT: the sentinel must fall in neither `[0, 2^64)`
        // nor `(p − 2^64, p)`.
        let s = value_none();
        assert!(
            !is_in_handle_band(&s),
            "the V2 no-value sentinel must lie outside the reachable handle band"
        );
        assert_eq!(no_value(), s, "the flat non-literal sentinel is the same tag");
        assert_eq!(value_none(), s, "the sentinel is deterministic");
    }

    #[test]
    fn value_none_is_pinned_to_its_derivation_and_value() {
        // Pin the VALUE, not just the property: the sentinel is the bare
        // `VALUE_NONE_CONTEXT` derivation (no counter re-derivation was needed),
        // so a change to the derivation or the context is caught here.
        let s = value_none();
        assert_eq!(s, derive(VALUE_NONE_CONTEXT, &[]));
        assert_eq!(
            crate::field::field_to_hex(&s),
            "0x00fe2f1f6b7aca564b1ccd5dc072a739e14179b3abee2e971bd03acb54de1016"
        );
        // It is NOT V1's in-band tag, and V1's genuinely is in band.
        assert_ne!(s, Fr::from(dual_leaf::VALUE_NONE));
        assert!(is_in_handle_band(&Fr::from(dual_leaf::VALUE_NONE)));
    }

    #[test]
    fn handle_band_predicate_covers_both_arms() {
        // Direct test of the predicate: small non-negative values and small field
        // negations are IN the band; a mid-field value is not.
        assert!(is_in_handle_band(&Fr::from(0u64)));
        assert!(is_in_handle_band(&Fr::from(1u64)));
        assert!(is_in_handle_band(&Fr::from(u64::MAX)));
        assert!(is_in_handle_band(&-Fr::from(1u64)));
        assert!(is_in_handle_band(&-Fr::from(u64::MAX)));
        // 2^64 itself is the first value above the non-negative arm.
        let two_64 = Fr::from(u64::MAX) + Fr::from(1u64);
        assert!(!is_in_handle_band(&two_64));
        assert!(!is_in_handle_band(&-two_64));
    }

    // ---- the lanes ----------------------------------------------------------

    #[test]
    fn value_lanes_carry_the_v2_constants_and_v1_handles() {
        // Each lane encoder reuses V1's value handle and canonical-lexical
        // predicate but folds the V2 lane constant, so the leaf differs from V1's
        // while the handle does not.
        for (lit, hook, konst) in [
            (typed("18", XSD_INTEGER), Fr::from(18u64), integer_lane_const()),
            (
                typed("2.5E0", XSD_DOUBLE),
                Fr::from(2.5f64.to_bits()),
                double_lane_const(),
            ),
            (
                typed("123.45", XSD_DECIMAL),
                Fr::from(12345u64),
                decimal_lane_const(2),
            ),
            (typed("true", XSD_BOOLEAN), Fr::from(1u64), boolean_lane_const()),
        ] {
            let v2 = match lit.datatype().as_str() {
                XSD_INTEGER => encode_integer(&lit),
                XSD_DOUBLE => encode_double(&lit),
                XSD_DECIMAL => encode_decimal(&lit),
                XSD_BOOLEAN => encode_boolean(&lit),
                other => panic!("unexpected datatype {other}"),
            }
            .unwrap();
            assert_eq!(v2.value_hook, hook, "handle drifted for {lit}");
            assert_eq!(v2.datatype_const, konst, "lane const drifted for {lit}");
            assert_eq!(
                v2.lexical_component,
                blake3_field(lit.to_string().as_bytes()),
                "lexical component must stay the string-canonical h_s for {lit}"
            );
            // The leaf is the same h3∘h3 construction the members recompute.
            assert_eq!(
                v2.leaf(),
                poseidon2::hash(&[
                    poseidon2::hash(&[v2.value_hook, v2.datatype_const, Fr::from(LANG_NONE)]),
                    v2.lexical_component,
                    Fr::from(TYPE_CODE_LITERAL),
                ])
            );
        }
        // Negative decimals fold the sign by field negation, as V1 does.
        assert_eq!(
            encode_decimal(&typed("-2.50", XSD_DECIMAL)).unwrap().value_hook,
            -Fr::from(250u64)
        );
        // And the B4 scale bind: same value, different scale, different handle
        // AND different lane constant.
        let a = encode_decimal(&typed("5.0", XSD_DECIMAL)).unwrap();
        let b = encode_decimal(&typed("5.00", XSD_DECIMAL)).unwrap();
        assert_ne!(a.datatype_const, b.datatype_const);
        assert_ne!(a.value_component(), b.value_component());
    }

    #[test]
    fn double_lane_keeps_the_in_circuit_canonical_bit_fold() {
        // -0.0 and +0.0 share the canonical handle; the lexical lane separates the
        // terms. Every NaN payload folds to the canonical quiet NaN.
        let neg = encode_double(&typed("-0.0E0", XSD_DOUBLE)).unwrap();
        let pos = encode_double(&typed("0.0E0", XSD_DOUBLE)).unwrap();
        assert_eq!(neg.value_hook, Fr::from(0u64));
        assert_eq!(neg.value_component(), pos.value_component());
        assert_ne!(neg.leaf(), pos.leaf());
        assert_eq!(
            encode_double(&typed("NaN", XSD_DOUBLE)).unwrap().value_hook,
            Fr::from(dual_leaf::F64_CANONICAL_NAN)
        );
    }

    #[test]
    fn datetime_and_date_lanes_carry_the_v2_constants() {
        let dt = typed("1970-01-02T00:00:00Z", XSD_DATE_TIME);
        let d = typed("1970-01-02Z", XSD_DATE);
        let edt = encode_datetime(&dt).unwrap();
        let ed = encode_date(&d).unwrap();
        // Same instant -> numerically equal handles; only the lane const differs.
        assert_eq!(edt.value_hook, Fr::from(86_400_000u64));
        assert_eq!(ed.value_hook, edt.value_hook);
        assert_eq!(edt.datatype_const, datetime_lane_const());
        assert_eq!(ed.datatype_const, date_lane_const());
        assert_ne!(edt.value_component(), ed.value_component());
        // Pre-epoch instants fold the sign by field negation.
        assert_eq!(
            encode_datetime(&typed("1969-12-31T23:59:59Z", XSD_DATE_TIME))
                .unwrap()
                .value_hook,
            -Fr::from(1000u64)
        );
    }

    #[test]
    fn lanes_reject_the_wrong_datatype_and_non_canonical_lexicals() {
        // The §6 fail-closed predicates are V1's, shared verbatim.
        assert!(matches!(
            encode_integer(&typed("1", XSD_DOUBLE)),
            Err(DualLeafError::NotValueLane(_))
        ));
        for lex in ["05", "-5", "99999999999999999999999999", "!!"] {
            assert!(
                matches!(
                    encode_integer(&typed(lex, XSD_INTEGER)),
                    Err(DualLeafError::NonCanonicalValue(_))
                ),
                "integer lane accepted the non-canonical {lex:?}"
            );
        }
        for lex in ["1e3", "inf", "+INF", "1.0E+3"] {
            assert!(
                matches!(
                    encode_double(&typed(lex, XSD_DOUBLE)),
                    Err(DualLeafError::NonCanonicalValue(_))
                ),
                "double lane accepted the non-canonical {lex:?}"
            );
        }
        for lex in ["5", "05.0", "-0.0"] {
            assert!(
                matches!(
                    encode_decimal(&typed(lex, XSD_DECIMAL)),
                    Err(DualLeafError::NonCanonicalValue(_))
                ),
                "decimal lane accepted the non-canonical {lex:?}"
            );
        }
        // The boolean lane rejects the XSD-legal but non-canonical spellings.
        for lex in ["1", "0", "True", " true"] {
            assert!(
                matches!(
                    encode_boolean(&typed(lex, XSD_BOOLEAN)),
                    Err(DualLeafError::NonCanonicalValue(_))
                ),
                "boolean lane accepted the non-canonical {lex:?}"
            );
        }
        assert!(matches!(
            encode_boolean(&typed("true", XSD_INTEGER)),
            Err(DualLeafError::NotValueLane(_))
        ));
        // A non-Z dateTime is outside the hookable domain.
        assert!(matches!(
            encode_datetime(&typed("1970-01-02T00:00:00+01:00", XSD_DATE_TIME)),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        assert!(matches!(
            encode_date(&typed("1970-01-02", XSD_DATE)),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
    }

    #[test]
    fn boolean_hook_is_exactly_zero_or_one() {
        // The bit domain the dedicated member constrains in-circuit: the host
        // encoder can only ever produce `{0, 1}`.
        assert_eq!(
            encode_boolean(&typed("false", XSD_BOOLEAN)).unwrap().value_hook,
            Fr::from(0u64)
        );
        assert_eq!(
            encode_boolean(&typed("true", XSD_BOOLEAN)).unwrap().value_hook,
            Fr::from(1u64)
        );
    }

    // ---- routing + the degenerate lane --------------------------------------

    #[test]
    fn routing_matches_the_hookable_predicate() {
        // `is_hookable_datatype` must agree with what `encode_literal` routes: a
        // hookable datatype fails closed on a bad lexical, a non-hookable one
        // takes the degenerate lane. Includes non-IRI datatype strings.
        for iri in [XSD_INTEGER, XSD_DECIMAL, XSD_DOUBLE] {
            assert!(is_hookable_datatype(iri));
            assert!(
                encode_literal(&typed("!!", iri)).is_err(),
                "hookable {iri} must fail closed on a bad lexical"
            );
        }
        for iri in [
            XSD_STRING,
            RDF_LANG_STRING,
            XSD_BOOLEAN,
            XSD_DATE_TIME,
            XSD_DATE,
            "http://ex/dt",
            "http://ex/dt x",
            "pre\0post",
            "",
        ] {
            assert!(!is_hookable_datatype(iri));
            let lit = typed("!!", iri);
            assert_eq!(
                encode_literal(&lit).unwrap(),
                poseidon2::hash(&[
                    degenerate_value_component(iri, lit.language()),
                    blake3_field(lit.to_string().as_bytes()),
                    Fr::from(TYPE_CODE_LITERAL),
                ]),
                "{iri:?} must take the degenerate lane"
            );
        }
    }

    #[test]
    fn degenerate_lane_folds_the_datatype_and_the_language() {
        let plain = Literal::new_simple_literal("v");
        assert_eq!(plain.datatype().as_str(), XSD_STRING);
        assert_eq!(
            encode_literal(&plain).unwrap(),
            poseidon2::hash(&[
                poseidon2::hash(&[
                    value_none(),
                    degenerate_datatype_const(XSD_STRING),
                    Fr::from(LANG_NONE),
                ]),
                blake3_field(plain.to_string().as_bytes()),
                Fr::from(TYPE_CODE_LITERAL),
            ]),
        );
        let en = Literal::new_language_tagged_literal("v", "en").unwrap();
        assert_eq!(
            encode_literal(&en).unwrap(),
            poseidon2::hash(&[
                poseidon2::hash(&[
                    value_none(),
                    degenerate_datatype_const(RDF_LANG_STRING),
                    lang_const("en"),
                ]),
                blake3_field(en.to_string().as_bytes()),
                Fr::from(TYPE_CODE_LITERAL),
            ]),
        );
        // A different language is a different value component (LANG_CONST is
        // load-bearing, not decorative).
        assert_ne!(
            degenerate_value_component(RDF_LANG_STRING, Some("en")),
            degenerate_value_component(RDF_LANG_STRING, Some("fr")),
        );
        assert_ne!(
            degenerate_value_component(RDF_LANG_STRING, Some("en")),
            degenerate_value_component(RDF_LANG_STRING, None),
        );
        // The §3.2 distinctness the degenerate lane must preserve.
        let leaves = [
            encode_literal(&plain).unwrap(),
            encode_literal(&typed("v", "http://ex/dt")).unwrap(),
            encode_literal(&en).unwrap(),
            encode_literal(&Literal::new_simple_literal("1")).unwrap(),
            encode_literal(&Literal::new_language_tagged_literal("1", "en").unwrap()).unwrap(),
            encode_literal(&typed("1", XSD_INTEGER)).unwrap(),
        ];
        for (i, a) in leaves.iter().enumerate() {
            for b in &leaves[i + 1..] {
                assert_ne!(a, b, "V2 literal leaves must be pairwise distinct");
            }
        }
    }

    // ---- whole-graph commitments --------------------------------------------

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
    fn commit_mirrors_the_ordering_contract_and_is_deterministic() {
        let salt = salt_from_bytes(&[7u8; 32]);
        let g = graph_triples();
        let v2 = commit_triples(&g, salt).unwrap();
        let string_canonical = crate::commit::commit_triples(&g, salt).unwrap();

        assert_eq!(v2.canonical.lines, string_canonical.canonical.lines);
        assert_eq!(v2.leaves.len(), string_canonical.leaves.len());
        assert_eq!(v2.salt, salt);

        let recomputed: Vec<Fr> = v2
            .canonical
            .triples
            .iter()
            .map(|t| encode_triple(t, &salt).unwrap().unwrap())
            .collect();
        assert_eq!(v2.leaves, recomputed, "leaves must be encode_triple in canonical order");
        assert_eq!(v2.commitment, poseidon2::hash(&recomputed));

        // Input order does not matter; the salt separates graphs.
        let mut reversed = g.clone();
        reversed.reverse();
        assert_eq!(commit_triples(&reversed, salt).unwrap().commitment, v2.commitment);
        assert_ne!(
            commit_triples(&g, salt_from_bytes(&[8u8; 32])).unwrap().commitment,
            v2.commitment
        );
    }

    #[test]
    fn v2_commitment_differs_from_v1_and_from_string_canonical() {
        // The leaf re-base: V2 is a DIFFERENT commitment over the same graph, so
        // a verifier MUST dispatch on the recorded scheme.
        let salt = salt_from_bytes(&[7u8; 32]);
        let g = graph_triples();
        let v2 = commit_triples(&g, salt).unwrap();
        let v1 = dual_leaf::commit_triples_dual(&g, salt).unwrap();
        let sc = crate::commit::commit_triples(&g, salt).unwrap();
        assert_ne!(v2.commitment, v1.commitment);
        assert_ne!(v2.commitment, sc.commitment);
        for ((a, b), c) in v2.leaves.iter().zip(&v1.leaves).zip(&sc.leaves) {
            assert_ne!(a, b, "every V2 leaf must differ from its V1 sibling");
            assert_ne!(a, c);
        }
        // Including the hookable lanes (routed in both versions) and the
        // language-tagged degenerate lane.
        let int = typed("18", XSD_INTEGER);
        assert_ne!(
            encode_literal(&int).unwrap(),
            dual_leaf::encode_literal_dual(&int).unwrap()
        );
        let en = Literal::new_language_tagged_literal("v", "en").unwrap();
        assert_ne!(
            encode_literal(&en).unwrap(),
            dual_leaf::encode_literal_dual(&en).unwrap()
        );
    }

    #[test]
    fn commit_graph_matches_commit_triples() {
        use sparq_core::Graph;
        let salt = salt_from_bytes(&[5u8; 32]);
        let g = Graph::load_str(
            "<http://ex/s> <http://ex/p> \"o\" .\n<http://ex/s> <http://ex/q> <http://ex/o2> .",
            "turtle",
        )
        .unwrap();
        let from_store = commit_graph(&g, salt).unwrap();
        let from_triples = commit_triples(&canon::graph_triples(&g).unwrap(), salt).unwrap();
        assert_eq!(from_store.commitment, from_triples.commitment);
        assert_eq!(from_store.leaves, from_triples.leaves);
    }

    #[test]
    fn ingest_is_fail_closed_on_a_rejected_hookable_lexical() {
        // THE §6 RULE: a hookable literal the lane rejects fails the WHOLE
        // commitment, never a silent degenerate-lane downgrade.
        let salt = salt_from_bytes(&[7u8; 32]);
        let triples = vec![Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            Term::Literal(typed("05", XSD_INTEGER)),
        )];
        assert!(matches!(
            commit_triples(&triples, salt).unwrap_err(),
            DualCommitError::Leaf(DualLeafError::NonCanonicalValue(_))
        ));
        // The canonical sibling commits fine, so the failure is about the lexical.
        let ok = vec![Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            Term::Literal(typed("5", XSD_INTEGER)),
        )];
        assert!(commit_triples(&ok, salt).is_ok());
    }

    #[test]
    fn non_literal_leaves_pin_the_flat_sentinel_shape() {
        let salt = salt_from_bytes(&[7u8; 32]);
        let iri = Term::NamedNode(NamedNode::new("http://ex/a").unwrap());
        assert_eq!(
            encode_term(&iri, &salt).unwrap(),
            Some(poseidon2::hash(&[
                no_value(),
                blake3_field(b"http://ex/a"),
                Fr::from(TYPE_CODE_IRI),
            ]))
        );
        // Salt-independent, like the string-canonical IRI leaf.
        assert_eq!(
            encode_term(&iri, &salt).unwrap(),
            encode_term(&iri, &salt_from_bytes(&[9u8; 32])).unwrap()
        );
        let b = Term::BlankNode(BlankNode::new("c14n0").unwrap());
        let inner = poseidon2::hash(&[salt, blake3_field(b"c14n0")]);
        assert_eq!(
            encode_term(&b, &salt).unwrap(),
            Some(poseidon2::hash(&[
                no_value(),
                inner,
                Fr::from(TYPE_CODE_BLANK_NODE),
            ]))
        );
        // The Q6 salt separation holds.
        assert_ne!(
            encode_term(&b, &salt).unwrap(),
            encode_term(&b, &salt_from_bytes(&[8u8; 32])).unwrap()
        );
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
        assert_eq!(encode_term(&quoted, &salt).unwrap(), None);
        let outer = Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            quoted,
        );
        assert_eq!(encode_triple(&outer, &salt).unwrap(), None);
        assert!(encode_triple(&inner, &salt).unwrap().is_some());
    }

    #[test]
    fn encode_triple_is_position_sensitive() {
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
            encode_triple(&t, &salt).unwrap(),
            encode_triple(&swapped, &salt).unwrap()
        );
    }
}
