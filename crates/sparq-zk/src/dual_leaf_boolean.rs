//! Dual-leaf `xsd:boolean` value-lane host encoding (sq-hh7a4) — the boolean
//! sibling of the integer/decimal/double encoders in `dual_leaf.rs`.
//!
//! OPT-IN, behind the `dual-leaf` cargo feature (OFF by default). This module is
//! compiled out of a normal build: the default `string-canonical` commitment
//! pipeline is byte-unchanged.
//!
//! # The boolean value handle (`research/zk-field-native-encoding.md` §3.3)
//!
//! `VALUE_HOOK = 0` (false) / `1` (true); `DATATYPE_CONST =
//! datatype_const(xsd:boolean IRI)`; `LANG_NONE` sentinel — the §3.3 boolean row.
//! The handle is injective on the two boolean VALUES but MANY-TO-ONE on the TERM
//! (reject-list (v)): the XSD-legal spellings `"true"` and `"1"` denote the same
//! value, so they would share the value handle, disambiguated ONLY by the
//! `lexical_component`. That is why `lexical_component` stays EXACTLY the
//! string-canonical `h_s = blake3_field(literal.to_string())` — identity ops
//! (`sameTerm`/`DISTINCT`/`join`) keep term identity unchanged.
//!
//! # Fail-closed same-leaf co-binding (§6)
//!
//! [`encode_boolean`] parses the lexical bytes ONCE and maps ONLY the CANONICAL
//! lexical forms `{"true", "false"}` to `{1, 0}`. The non-canonical XSD-legal
//! spellings `{"1", "0"}` (and any other lexical form, or any non-boolean
//! datatype) are REJECTED with a fail-closed `DualLeafError` — never a silent
//! desynced leaf — so sparq's OWN commitments cannot self-desync. This binds
//! honest sparq ingest, NOT a malicious external issuer.
//!
//! # NO production-soundness claim
//!
//! This inherits the documented INV-VL downgrade of the dual-leaf method (see
//! the `dual_leaf` module docs): value↔lexical agreement on the value-FILTER
//! lane rests on TRUSTED-ISSUER HONESTY, not machine enforcement. Open
//! external-audit obligation: gap CR-G8 / sq-qhy4. Nothing here is a soundness
//! or privacy guarantee. Host half ONLY — no circuit/verifier change.

use crate::dual_leaf::{datatype_const, DualLeafComponents, DualLeafError};
use crate::field::{field_from_hash_bytes, Fr};
use oxrdf::Literal;

/// The `xsd:boolean` datatype IRI (the boolean value-lane datatype class,
/// sq-hh7a4).
pub const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

fn blake3_field(bytes: &[u8]) -> Fr {
    field_from_hash_bytes(blake3::hash(bytes).as_bytes())
}

/// Map a CANONICAL `xsd:boolean` lexical form to its value handle: `"true"` →
/// `1`, `"false"` → `0`, everything else `None` (the §6 fail-closed predicate).
/// The XSD-legal but NON-canonical spellings `"1"` / `"0"` return `None`.
///
/// Crate-internal so the versioned sibling encoder shares exactly this
/// predicate instead of restating it.
pub(crate) fn canonical_boolean(lexical: &str) -> Option<u64> {
    match lexical {
        "true" => Some(1),
        "false" => Some(0),
        _ => None,
    }
}

/// Encodes an `xsd:boolean` literal under the dual-leaf method, with fail-closed
/// same-leaf co-binding (§6): the lexical bytes are parsed once, ONLY the
/// canonical forms `"true"` / `"false"` map to `VALUE_HOOK` `1` / `0`, and the
/// non-canonical XSD-legal spellings `"1"` / `"0"` (or anything else) are
/// REJECTED so sparq's own commitments cannot self-desync. Returns the three
/// components; `.leaf()` is the committed `Enc`.
///
/// `lexical_component` is EXACTLY the string-canonical `h_s`
/// (`blake3_field(literal.to_string())`), so identity ops keep term identity —
/// load-bearing because the boolean value handle is many-to-one on the term
/// (`"true"` and the rejected-at-ingest `"1"` would share the handle,
/// disambiguated only by the lexical lane).
pub fn encode_boolean(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_BOOLEAN {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    // Same-leaf co-binding: parse the lexical bytes once; ONLY the canonical
    // spellings are accepted (the §6 fail-closed predicate). "1"/"0" are
    // XSD-legal but NON-canonical — rejected, never silently canonicalised.
    let value: u64 = canonical_boolean(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    Ok(DualLeafComponents {
        value_hook: Fr::from(value),
        datatype_const: datatype_const(XSD_BOOLEAN),
        // EXACTLY the string-canonical h_s over the canonical N-Triples token,
        // so a dual-leaf graph's identity ops read the same lexical identity.
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dual_leaf::LANG_NONE;
    use crate::encode::TYPE_CODE_LITERAL;
    use crate::poseidon2;
    use oxrdf::NamedNode;

    fn bool_lit(v: &str) -> Literal {
        Literal::new_typed_literal(v, NamedNode::new(XSD_BOOLEAN).unwrap())
    }

    #[test]
    fn encode_boolean_true_and_false_round_trip() {
        let t = encode_boolean(&bool_lit("true")).unwrap();
        let f = encode_boolean(&bool_lit("false")).unwrap();
        assert_eq!(t.datatype_const, datatype_const(XSD_BOOLEAN));
        assert_eq!(f.datatype_const, datatype_const(XSD_BOOLEAN));
        // The leaves are well-defined, stable, and distinct.
        assert_eq!(t.leaf(), t.leaf());
        assert_eq!(f.leaf(), f.leaf());
        assert_ne!(t.leaf(), f.leaf());
        // The leaf is exactly h3(h3(hook, dt, LANG_NONE), lexical, TYPE_CODE_LITERAL)
        // — the same construction as the other dual-leaf value lanes.
        let vc = poseidon2::hash(&[t.value_hook, t.datatype_const, Fr::from(LANG_NONE)]);
        let leaf = poseidon2::hash(&[vc, t.lexical_component, Fr::from(TYPE_CODE_LITERAL)]);
        assert_eq!(t.leaf(), leaf);
    }

    #[test]
    fn boolean_value_hook_is_0_and_1() {
        // §3.3 boolean row: VALUE_HOOK = 0 (false) / 1 (true).
        assert_eq!(
            encode_boolean(&bool_lit("true")).unwrap().value_hook,
            Fr::from(1u64)
        );
        assert_eq!(
            encode_boolean(&bool_lit("false")).unwrap().value_hook,
            Fr::from(0u64)
        );
    }

    #[test]
    fn non_canonical_1_and_0_are_fail_closed() {
        // "1"/"0" are XSD-legal xsd:boolean spellings but NON-canonical: the §6
        // co-binding rejects them fail-closed (never a silent desynced leaf).
        assert!(matches!(
            encode_boolean(&bool_lit("1")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        assert!(matches!(
            encode_boolean(&bool_lit("0")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        // Case / whitespace variants are equally non-canonical.
        assert!(matches!(
            encode_boolean(&bool_lit("True")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        assert!(matches!(
            encode_boolean(&bool_lit(" true")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
    }

    #[test]
    fn non_boolean_datatype_is_rejected() {
        let plain = Literal::new_simple_literal("true");
        assert!(matches!(
            encode_boolean(&plain),
            Err(DualLeafError::NotValueLane(_))
        ));
        let int = Literal::new_typed_literal(
            "1",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        );
        assert!(matches!(
            encode_boolean(&int),
            Err(DualLeafError::NotValueLane(_))
        ));
    }

    #[test]
    fn lexical_component_equals_string_canonical_h_s() {
        // The dual leaf's lexical_component MUST be byte-identical to the
        // string-canonical scheme's h_s, so identity ops are unchanged.
        for lex in ["true", "false"] {
            let lit = bool_lit(lex);
            let c = encode_boolean(&lit).unwrap();
            assert_eq!(
                c.lexical_component,
                blake3_field(lit.to_string().as_bytes())
            );
        }
    }

    #[test]
    fn true_and_canonical_1_would_share_value_handle_but_differ_in_lexical_component() {
        // Reject-list (v): the boolean value handle is MANY-TO-ONE on the term.
        // encode_boolean REJECTS "1" (fail-closed), so build the hypothetical
        // components an issuer honest-about-the-VALUE would commit for `"1"` —
        // same VALUE_HOOK = 1, same datatype const — and check that ONLY the
        // lexical_component (hence the leaf) separates it from "true".
        let t = encode_boolean(&bool_lit("true")).unwrap();
        let one = DualLeafComponents {
            value_hook: Fr::from(1u64),
            datatype_const: datatype_const(XSD_BOOLEAN),
            lexical_component: blake3_field(bool_lit("1").to_string().as_bytes()),
        };
        // Shared value handle: identical value_component.
        assert_eq!(t.value_hook, one.value_hook);
        assert_eq!(t.value_component(), one.value_component());
        // Disambiguated ONLY by the lexical lane: distinct h_s, distinct leaf.
        assert_ne!(t.lexical_component, one.lexical_component);
        assert_ne!(t.leaf(), one.leaf());
    }
}
