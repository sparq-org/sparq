//! The XSD 1.1 §3.4 derived-integer bounds/sign facet table, shared by the RIF-Core
//! (`rif-core`) and D-entailment (`d-entail`) front-ends so the two can never drift apart
//! on a facet. Crate-internal and dependency-free: it holds only the table plus the
//! `i128` predicate; the RIF path derives its arbitrary-precision lexical predicate from
//! [`integer_facet_bounds`] itself (issue #5337).

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// The inclusive `(min, max)` VALUE-SPACE bounds of a **derived** XSD integer datatype,
/// with `None` on a side that is unbounded. The outer `None` means `datatype` carries no
/// bounding facet at all — `xsd:integer` itself (unbounded both ways), or an IRI that is
/// not an integer datatype.
///
/// XSD derives each of these from `xsd:integer` by a `minInclusive`/`maxInclusive` facet
/// pair, and a derived type's LEXICAL space is exactly the lexicals mapping into its
/// value space — so `"-1"^^xsd:positiveInteger` and `"128"^^xsd:byte` are not well-formed
/// literals of their declared datatype, however well-formed the digit string is.
pub(crate) fn integer_facet_bounds(datatype: &str) -> Option<(Option<i128>, Option<i128>)> {
    let local = datatype.strip_prefix(XSD)?;
    Some(match local {
        "long" => (Some(i64::MIN as i128), Some(i64::MAX as i128)),
        "int" => (Some(i32::MIN as i128), Some(i32::MAX as i128)),
        "short" => (Some(i16::MIN as i128), Some(i16::MAX as i128)),
        "byte" => (Some(i8::MIN as i128), Some(i8::MAX as i128)),
        "unsignedLong" => (Some(0), Some(u64::MAX as i128)),
        "unsignedInt" => (Some(0), Some(u32::MAX as i128)),
        "unsignedShort" => (Some(0), Some(u16::MAX as i128)),
        "unsignedByte" => (Some(0), Some(u8::MAX as i128)),
        "nonNegativeInteger" => (Some(0), None),
        "positiveInteger" => (Some(1), None),
        "nonPositiveInteger" => (None, Some(0)),
        "negativeInteger" => (None, Some(-1)),
        // `xsd:integer` is unfaceted; anything else is not a derived integer datatype.
        _ => return None,
    })
}

/// Whether the already-parsed value `v` lies inside the value space of `datatype`'s
/// bounds/sign facets. `true` for a datatype with no bounding facet (`xsd:integer`, or
/// any IRI outside the derived-integer table).
#[cfg(feature = "d-entail")]
pub(crate) fn integer_in_bounds(datatype: &str, v: i128) -> bool {
    match integer_facet_bounds(datatype) {
        None => true,
        Some((min, max)) => min.is_none_or(|lo| v >= lo) && max.is_none_or(|hi| v <= hi),
    }
}

#[cfg(all(test, feature = "d-entail"))]
mod tests {
    use super::*;

    fn xsd(local: &str) -> String {
        format!("{XSD}{local}")
    }

    #[test]
    fn i128_predicate_matches_the_xsd_ranges_at_every_edge() {
        let cases: &[(&str, i128, i128)] = &[
            ("long", i64::MIN as i128, i64::MAX as i128),
            ("int", -2_147_483_648, 2_147_483_647),
            ("short", -32_768, 32_767),
            ("byte", -128, 127),
            ("unsignedLong", 0, 18_446_744_073_709_551_615),
            ("unsignedInt", 0, 4_294_967_295),
            ("unsignedShort", 0, 65_535),
            ("unsignedByte", 0, 255),
        ];
        for &(local, lo, hi) in cases {
            let dt = xsd(local);
            assert!(integer_in_bounds(&dt, lo), "{local} min");
            assert!(integer_in_bounds(&dt, hi), "{local} max");
            assert!(!integer_in_bounds(&dt, lo - 1), "{local} below min");
            assert!(!integer_in_bounds(&dt, hi + 1), "{local} above max");
        }
        assert!(integer_in_bounds(&xsd("nonNegativeInteger"), 0));
        assert!(!integer_in_bounds(&xsd("nonNegativeInteger"), -1));
        assert!(integer_in_bounds(&xsd("positiveInteger"), 1));
        assert!(!integer_in_bounds(&xsd("positiveInteger"), 0));
        assert!(integer_in_bounds(&xsd("nonPositiveInteger"), 0));
        assert!(!integer_in_bounds(&xsd("nonPositiveInteger"), 1));
        assert!(integer_in_bounds(&xsd("negativeInteger"), -1));
        assert!(!integer_in_bounds(&xsd("negativeInteger"), 0));
        // Unfaceted / non-XSD datatypes are permissive.
        assert!(integer_in_bounds(&xsd("integer"), i128::MAX));
        assert!(integer_in_bounds("http://example.org/byte", 1_000));
    }
}
