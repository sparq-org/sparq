//! Dual-leaf `xsd:dateTime` / `xsd:date` value-lane host encoding (sq-we9vs) —
//! the dateTime/date siblings of the integer/decimal/double encoders in
//! `dual_leaf.rs` and the boolean encoder in `dual_leaf_boolean.rs`.
//!
//! OPT-IN, behind the `dual-leaf` cargo feature (OFF by default). This module is
//! compiled out of a normal build: the default `string-canonical` commitment
//! pipeline is byte-unchanged.
//!
//! # The dateTime/date value handle (`research/zk-field-native-encoding.md` §13)
//!
//! `VALUE_HOOK` is a **signed scaled-epoch scalar**:
//!
//! ```text
//! VALUE_HOOK = sign(T) * |T|   where   T = milliseconds from 1970-01-01T00:00:00Z
//! ```
//!
//! on the XSD proleptic-Gregorian `timeOnTimeline` (NO leap seconds — XSD's
//! timeline has none), at the member-fixed sub-second scale `FS = 3`
//! ([`EPOCH_SCALE_FS`], milliseconds). Canonical lexicals with `1..=FS` fraction
//! digits are scaled UP to `FS` exactly — never rounded (rounding would break
//! injectivity AND desync the §6 co-binding). The magnitude lives in the same
//! `u64`-magnitude + sign domain the `filter_signed` / `filter_value_dl_decimal`
//! circuit machinery already compares; the sign is folded into the handle by
//! field negation (mirroring `encode_decimal`), and `-0` cannot arise (a zero
//! magnitude is non-negative by construction).
//!
//! `FS` is folded into the lane's `DATATYPE_CONST` exactly like the decimal
//! `@scale=` bind (B4): [`datetime_datatype_const`] =
//! `blake3("<xsd:dateTime IRI>@epochscale=3")`, so a hook at one scale can never
//! collide a hook at another. `xsd:date` is its OWN lane
//! ([`date_datatype_const`], `blake3("<xsd:date IRI>@epochscale=3")`) — a date
//! can never collide a dateTime; its hook is the scaled epoch of the date's
//! STARTING instant (midnight UTC — XSD orders dates by their starting moment).
//!
//! # The timezone rule — hookable domain = timezoned `Z` ONLY (§13.2)
//!
//! Accepted lexicals are strict XSD-canonical `Z`-timezoned forms ONLY.
//! Fail-closed REJECTED (a `DualLeafError`, never a silent desynced leaf, never
//! an implicit timezone):
//!
//! - **bare / un-timezoned** lexicals — XSD order between an un-timezoned and a
//!   timezoned value is PARTIAL (indeterminate inside the ±14:00 window); mapping
//!   both into one scalar domain would compare indeterminate pairs determinately,
//!   inconsistent with the engine's own residual partial order (sq-2k5py);
//! - **non-`Z` offsets** including `+00:00`/`-00:00` — the strict slice-1 mirror
//!   of the boolean lane's canonical-only rule; offset-normalisation is the
//!   documented §13.6 widening, NOT this module;
//! - **`24:00:00`** (two lexicals for one value) and the **leap second `60`**
//!   (not an XSD lexical);
//! - **non-canonical year zeros** (`-0000`, superfluous leading digits);
//! - fractions with more than `FS` digits or a trailing zero (canonical minimal
//!   form; never rounded);
//! - a scaled-epoch magnitude overflowing `u64` (far-proleptic years) — rejected,
//!   never wrapped (mirroring the integer lane).
//!
//! Within this domain the hook is **injective-on-value within the datatype** —
//! and in slice 1 injective on the TERM too (the `Z`-only canonical domain admits
//! one lexical per value), so no new row joins the §3.3 many-to-one hazard table
//! until the §13.6 offset widening lands.
//!
//! `lexical_component` stays EXACTLY the string-canonical
//! `h_s = blake3_field(literal.to_string())`, so identity ops
//! (`sameTerm`/`DISTINCT`/`join`) keep term identity unchanged.
//!
//! # NO production-soundness claim
//!
//! This inherits the documented INV-VL downgrade of the dual-leaf method (see
//! the `dual_leaf` module docs): value↔lexical agreement on the value-FILTER
//! lane rests on TRUSTED-ISSUER HONESTY, not machine enforcement; the §6
//! co-binding here binds honest sparq ingest only. The §13 rule set is itself
//! registered as an OPEN external-audit obligation (gap CR-G8 / sq-qhy4).
//! Nothing here is a soundness or privacy guarantee. Host half ONLY — the
//! `filter_value_dl_datetime` circuit member is the paired follow-on bead
//! (sq-wz99x); no circuit/verifier change lands here.

use crate::dual_leaf::{DualLeafComponents, DualLeafError};
use crate::field::{field_from_hash_bytes, Fr};
use oxrdf::Literal;

/// The `xsd:dateTime` datatype IRI (the dateTime value-lane datatype class,
/// sq-we9vs).
pub const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

/// The `xsd:date` datatype IRI (the date value-lane datatype class, sq-we9vs).
pub const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";

/// The member-fixed sub-second scale `FS` (§13.1): the whole lane commits
/// scaled-epoch hooks at milliseconds. Fixed for the WHOLE lane (unlike the
/// decimal per-lexical `fd`) so every committed hook lives in ONE totally
/// ordered domain and a FILTER can compare operands of differing lexical
/// precision. Folded into the lane constants, so a higher-`FS` member is a
/// compatible future addition.
pub const EPOCH_SCALE_FS: u32 = 3;

const MS_PER_DAY: i128 = 86_400_000;

fn blake3_field(bytes: &[u8]) -> Fr {
    field_from_hash_bytes(blake3::hash(bytes).as_bytes())
}

/// The `xsd:dateTime` lane's `DATATYPE_CONST`:
/// `blake3_field("<xsd:dateTime IRI>@epochscale=3")` — the B4 scale bind
/// (§13.1), mirroring the Noir member's public `datatype_const`.
pub fn datetime_datatype_const() -> Fr {
    blake3_field(format!("{}@epochscale={}", XSD_DATE_TIME, EPOCH_SCALE_FS).as_bytes())
}

/// The `xsd:date` lane's OWN `DATATYPE_CONST`:
/// `blake3_field("<xsd:date IRI>@epochscale=3")` — a date can never collide a
/// dateTime (§13.3 cross-datatype separation).
pub fn date_datatype_const() -> Fr {
    blake3_field(format!("{}@epochscale={}", XSD_DATE, EPOCH_SCALE_FS).as_bytes())
}

/// Encodes an `xsd:dateTime` literal under the dual-leaf method (§13), with
/// fail-closed same-leaf co-binding (§6): the value handle is the signed
/// scaled-epoch (milliseconds, [`EPOCH_SCALE_FS`]) of the SAME strict
/// XSD-canonical `Z`-timezoned lexical form the `lexical_component` hashes; any
/// bare / offset / non-canonical / overflowing form is REJECTED (so sparq's own
/// ingest cannot self-desync — see the module docs for the exact §13.4
/// predicate). Returns the three components; `.leaf()` is the committed `Enc`.
pub fn encode_datetime(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_DATE_TIME {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    // Same-leaf co-binding: one strict canonical parse of the same bytes h_s
    // hashes; anything outside the §13.4 domain is fail-closed.
    let (neg, mag) = canonical_datetime_scaled(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    let mag_fr = Fr::from(mag);
    Ok(DualLeafComponents {
        value_hook: if neg { -mag_fr } else { mag_fr },
        datatype_const: datetime_datatype_const(),
        // EXACTLY the string-canonical h_s over the canonical N-Triples token,
        // so a dual-leaf graph's identity ops read the same lexical identity.
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Encodes an `xsd:date` literal under the dual-leaf method (§13.3): same rule,
/// own lane — the value handle is the scaled epoch of the date's STARTING
/// instant (midnight UTC) under the [`date_datatype_const`] constant. Slice-1
/// domain = `YYYY-MM-DDZ` canonical lexicals only; bare dates are rejected
/// fail-closed for exactly the §13.2 indeterminacy reason.
pub fn encode_date(literal: &Literal) -> Result<DualLeafComponents, DualLeafError> {
    if literal.datatype().as_str() != XSD_DATE {
        return Err(DualLeafError::NotValueLane(literal.to_string()));
    }
    let (neg, mag) = canonical_date_scaled(literal.value())
        .ok_or_else(|| DualLeafError::NonCanonicalValue(literal.to_string()))?;
    let mag_fr = Fr::from(mag);
    Ok(DualLeafComponents {
        value_hook: if neg { -mag_fr } else { mag_fr },
        datatype_const: date_datatype_const(),
        lexical_component: blake3_field(literal.to_string().as_bytes()),
    })
}

/// Parse a strict XSD-canonical `Z`-timezoned `xsd:dateTime` lexical form to its
/// signed scaled-epoch `(neg, |T| in ms)`. Returns `None` for anything outside
/// the §13.4 fail-closed domain.
pub(crate) fn canonical_datetime_scaled(lexical: &str) -> Option<(bool, u64)> {
    let body = lexical.strip_suffix('Z')?;
    let (date_part, time_part) = body.split_once('T')?;
    let days = parse_canonical_date_days(date_part)?;
    let (sod_ms, rest) = parse_canonical_time_ms(time_part)?;
    if !rest.is_empty() {
        return None;
    }
    signed_mag(days * MS_PER_DAY + sod_ms)
}

/// Parse a strict XSD-canonical `YYYY-MM-DDZ` `xsd:date` lexical form to the
/// signed scaled-epoch `(neg, |T| in ms)` of its starting instant (midnight
/// UTC). Returns `None` for anything outside the §13.4 fail-closed domain.
pub(crate) fn canonical_date_scaled(lexical: &str) -> Option<(bool, u64)> {
    let body = lexical.strip_suffix('Z')?;
    let days = parse_canonical_date_days(body)?;
    signed_mag(days * MS_PER_DAY)
}

/// Fold a signed epoch-milliseconds value into the `filter_signed`
/// `(neg, u64 magnitude)` domain; a magnitude overflowing `u64` is `None` —
/// rejected, never wrapped (the far-proleptic overflow rule). `-0` cannot
/// arise: a zero magnitude reports non-negative.
fn signed_mag(total_ms: i128) -> Option<(bool, u64)> {
    let mag = u64::try_from(total_ms.unsigned_abs()).ok()?;
    Some((total_ms < 0, mag))
}

/// Parse the canonical `[-]YYYY(Y*)-MM-DD` date fields to signed days from the
/// 1970-01-01 epoch (proleptic Gregorian, days-from-civil — pure integer, no
/// float). Canonicality: 4+ year digits with no superfluous leading zero beyond
/// four digits, `-` permitted but `-0000` rejected, no `+`; month `01..=12`;
/// day valid for month + leap rule.
fn parse_canonical_date_days(s: &str) -> Option<i128> {
    // Canonical lexicals are ASCII; rejecting non-ASCII up front keeps every
    // later byte-index on a char boundary (fail-closed never panics).
    if !s.is_ascii() {
        return None;
    }
    let (neg_year, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    // Exactly year-'-'-MM-'-'-DD from the right: the year is everything before
    // the two fixed-width trailing components.
    let (year_str, md) = s.split_at(s.len().checked_sub(6)?);
    if !md.starts_with('-') {
        return None;
    }
    if year_str.len() < 4 || !year_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // No superfluous leading zeros beyond four digits ("02020" is non-canonical).
    if year_str.len() > 4 && year_str.starts_with('0') {
        return None;
    }
    let year: i128 = year_str.parse::<i64>().ok()?.into();
    // "-0000" is a non-canonical year zero.
    if neg_year && year == 0 {
        return None;
    }
    let year = if neg_year { -year } else { year };
    let month = fixed_two_digits(&md[1..3])?;
    if md.as_bytes()[3] != b'-' {
        return None;
    }
    let day = fixed_two_digits(&md[4..6])?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil(year, month as i128, day as i128))
}

/// Parse the canonical `hh:mm:ss(.f{1..=FS})?` time fields to (milliseconds
/// into the day, unconsumed rest). Hour `<= 23` (no `24:00:00`), minute/second
/// `<= 59` (no leap second — not an XSD lexical); fraction `1..=FS` digits with
/// no trailing zero (canonical minimal form), scaled UP to `FS` exactly — never
/// rounded.
fn parse_canonical_time_ms(s: &str) -> Option<(i128, &str)> {
    if !s.is_ascii() {
        return None;
    }
    let b = s.as_bytes();
    if b.len() < 8 || b[2] != b':' || b[5] != b':' {
        return None;
    }
    let hour = fixed_two_digits(&s[0..2])?;
    let minute = fixed_two_digits(&s[3..5])?;
    let second = fixed_two_digits(&s[6..8])?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let rest = &s[8..];
    let (frac_ms, rest) = match rest.strip_prefix('.') {
        None => (0u32, rest),
        Some(frac) => {
            // The fraction is 1..=FS ASCII digits ending in a non-zero digit.
            let digits = frac;
            if digits.is_empty()
                || digits.len() > EPOCH_SCALE_FS as usize
                || !digits.bytes().all(|b| b.is_ascii_digit())
                || digits.ends_with('0')
            {
                return None;
            }
            let val: u32 = digits.parse().ok()?;
            (val * 10u32.pow(EPOCH_SCALE_FS - digits.len() as u32), "")
        }
    };
    let sod_ms =
        i128::from(hour) * 3_600_000 + i128::from(minute) * 60_000 + i128::from(second) * 1_000;
    Some((sod_ms + i128::from(frac_ms), rest))
}

/// Exactly two ASCII digits -> value (fixed-width canonical component).
fn fixed_two_digits(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    if b.len() != 2 || !b[0].is_ascii_digit() || !b[1].is_ascii_digit() {
        return None;
    }
    Some(u32::from(b[0] - b'0') * 10 + u32::from(b[1] - b'0'))
}

/// Proleptic-Gregorian leap-year rule on the astronomical year number (year
/// `0000` IS a leap year); exact for negative years via euclidean remainders.
fn is_leap_year(year: i128) -> bool {
    year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
}

fn days_in_month(year: i128, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Signed days from 1970-01-01 for a proleptic-Gregorian civil date
/// (Howard Hinnant's `days_from_civil`, in `i128` so far-proleptic years cannot
/// overflow the intermediate arithmetic — the u64 magnitude check happens on
/// the scaled epoch).
fn days_from_civil(year: i128, month: i128, day: i128) -> i128 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400; // [0, 399]
    let mp = (month + 9) % 12; // Mar=0 .. Feb=11
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dual_leaf::LANG_NONE;
    use crate::encode::TYPE_CODE_LITERAL;
    use crate::poseidon2;
    use oxrdf::NamedNode;

    fn dt_lit(v: &str) -> Literal {
        Literal::new_typed_literal(v, NamedNode::new(XSD_DATE_TIME).unwrap())
    }

    fn date_lit(v: &str) -> Literal {
        Literal::new_typed_literal(v, NamedNode::new(XSD_DATE).unwrap())
    }

    #[test]
    fn z_canonical_datetime_round_trip_hook_is_scaled_epoch() {
        // The epoch itself.
        let epoch = encode_datetime(&dt_lit("1970-01-01T00:00:00Z")).unwrap();
        assert_eq!(epoch.value_hook, Fr::from(0u64));
        assert_eq!(epoch.datatype_const, datetime_datatype_const());
        // A known positive epoch: 2001-09-09T01:46:40Z = 1_000_000_000 s.
        let giga = encode_datetime(&dt_lit("2001-09-09T01:46:40Z")).unwrap();
        assert_eq!(giga.value_hook, Fr::from(1_000_000_000_000u64));
        // Sub-second precision scales to FS=3 exactly: +123 ms.
        let frac = encode_datetime(&dt_lit("2001-09-09T01:46:40.123Z")).unwrap();
        assert_eq!(frac.value_hook, Fr::from(1_000_000_000_123u64));
        // A PRE-1970 lexical: one half-second before the epoch is -500 ms —
        // the sign folds into the handle by field negation (no -0 domain).
        let pre = encode_datetime(&dt_lit("1969-12-31T23:59:59.5Z")).unwrap();
        assert_eq!(pre.value_hook, -Fr::from(500u64));
        // The leaf is the exact circuit construction
        // h3(h3(hook, dt, LANG_NONE), lexical, TYPE_CODE_LITERAL).
        let vc = poseidon2::hash(&[giga.value_hook, giga.datatype_const, Fr::from(LANG_NONE)]);
        let leaf = poseidon2::hash(&[vc, giga.lexical_component, Fr::from(TYPE_CODE_LITERAL)]);
        assert_eq!(giga.leaf(), leaf);
    }

    #[test]
    fn date_hook_is_midnight_utc_start_instant_own_datatype_const() {
        // XSD orders dates by their starting instant: the hook is midnight UTC.
        let d = encode_date(&date_lit("1970-01-02Z")).unwrap();
        assert_eq!(d.value_hook, Fr::from(86_400_000u64));
        // Pre-epoch date: 1969-12-31 starts one full day before the epoch.
        let pre = encode_date(&date_lit("1969-12-31Z")).unwrap();
        assert_eq!(pre.value_hook, -Fr::from(86_400_000u64));
        // The date lane has its OWN constant: a date can never collide the
        // dateTime commitment of its own starting instant.
        assert_eq!(d.datatype_const, date_datatype_const());
        assert_ne!(date_datatype_const(), datetime_datatype_const());
        let same_instant = encode_datetime(&dt_lit("1970-01-02T00:00:00Z")).unwrap();
        assert_eq!(d.value_hook, same_instant.value_hook);
        assert_ne!(d.value_component(), same_instant.value_component());
        assert_ne!(d.leaf(), same_instant.leaf());
    }

    #[test]
    fn bare_untimezoned_datetime_and_date_are_fail_closed() {
        // §13.2(1): un-timezoned values are order-INDETERMINATE against
        // timezoned ones — never hooked, never given an implicit timezone.
        assert!(matches!(
            encode_datetime(&dt_lit("2020-01-01T12:00:00")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        assert!(matches!(
            encode_date(&date_lit("2020-01-01")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
    }

    #[test]
    fn non_z_offset_and_plus00_are_fail_closed() {
        // §13.2(2): slice 1 is canonical-Z only; +00:00/-00:00 spell the same
        // value as Z but are NON-canonical — rejected, not normalised (the
        // offset-normalisation widening is §13.6, not this slice).
        for lex in [
            "2020-01-01T12:00:00+01:00",
            "2020-01-01T12:00:00-05:00",
            "2020-01-01T12:00:00+00:00",
            "2020-01-01T12:00:00-00:00",
        ] {
            assert!(matches!(
                encode_datetime(&dt_lit(lex)),
                Err(DualLeafError::NonCanonicalValue(_))
            ));
        }
        for lex in ["2020-01-01+00:00", "2020-01-01-05:00"] {
            assert!(matches!(
                encode_date(&date_lit(lex)),
                Err(DualLeafError::NonCanonicalValue(_))
            ));
        }
    }

    #[test]
    fn hour_24_and_leap_second_are_fail_closed() {
        // 24:00:00 is XSD-legal but excluded from the canonical form (two
        // lexicals for one value); second=60 is not an XSD lexical at all.
        assert!(matches!(
            encode_datetime(&dt_lit("2020-01-01T24:00:00Z")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        assert!(matches!(
            encode_datetime(&dt_lit("2016-12-31T23:59:60Z")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
    }

    #[test]
    fn fraction_over_fs_or_trailing_zero_is_fail_closed() {
        // More than FS=3 fraction digits would need ROUNDING — which breaks
        // injectivity and desyncs the §6 co-binding — so it is rejected.
        assert!(matches!(
            encode_datetime(&dt_lit("2020-01-01T12:00:00.1234Z")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        // Trailing zeros are non-canonical (".500" spells ".5"); ".0" spells
        // an absent fraction; the empty fraction "." is not a lexical.
        for lex in [
            "2020-01-01T12:00:00.500Z",
            "2020-01-01T12:00:00.0Z",
            "2020-01-01T12:00:00.Z",
        ] {
            assert!(matches!(
                encode_datetime(&dt_lit(lex)),
                Err(DualLeafError::NonCanonicalValue(_))
            ));
        }
    }

    #[test]
    fn far_proleptic_overflow_is_fail_closed_not_wrapped() {
        // A year whose scaled epoch overflows the u64 magnitude domain is
        // rejected — never wrapped (mirroring the integer lane). u64::MAX ms
        // ~ year 584_556_019; twelve-digit years are far beyond it.
        assert!(matches!(
            encode_datetime(&dt_lit("999999999999-01-01T00:00:00Z")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        assert!(matches!(
            encode_datetime(&dt_lit("-999999999999-01-01T00:00:00Z")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
        assert!(matches!(
            encode_date(&date_lit("999999999999-01-01Z")),
            Err(DualLeafError::NonCanonicalValue(_))
        ));
    }

    #[test]
    fn cross_precision_values_share_one_totally_ordered_domain() {
        // THE member-fixed-FS point (§13.1): lexicals of DIFFERING fraction
        // precision land in ONE scaled domain under ONE constant, so a single
        // signed comparison orders them: T12:00:00Z < T12:00:00.5Z by 500 ms.
        let (neg_a, mag_a) = canonical_datetime_scaled("2020-06-01T12:00:00Z").unwrap();
        let (neg_b, mag_b) = canonical_datetime_scaled("2020-06-01T12:00:00.5Z").unwrap();
        assert!(!neg_a && !neg_b);
        assert_eq!(mag_b, mag_a + 500);
        assert!(mag_a < mag_b);
        // Same lane constant — the member compares them directly.
        let a = encode_datetime(&dt_lit("2020-06-01T12:00:00Z")).unwrap();
        let b = encode_datetime(&dt_lit("2020-06-01T12:00:00.5Z")).unwrap();
        assert_eq!(a.datatype_const, b.datatype_const);
        assert_ne!(a.value_hook, b.value_hook);
    }

    #[test]
    fn lexical_component_equals_string_canonical_h_s() {
        // The dual leaf's lexical_component MUST be byte-identical to the
        // string-canonical scheme's h_s, so identity ops are unchanged.
        let dt = dt_lit("2001-09-09T01:46:40.123Z");
        let c = encode_datetime(&dt).unwrap();
        assert_eq!(c.lexical_component, blake3_field(dt.to_string().as_bytes()));
        let d = date_lit("2001-09-09Z");
        let cd = encode_date(&d).unwrap();
        assert_eq!(cd.lexical_component, blake3_field(d.to_string().as_bytes()));
    }

    #[test]
    fn calendar_validity_and_year_canonicality_are_fail_closed() {
        // Structural validity: day must exist in the proleptic-Gregorian month.
        assert!(encode_datetime(&dt_lit("2023-02-29T00:00:00Z")).is_err());
        assert!(encode_date(&date_lit("2023-02-29Z")).is_err());
        assert!(encode_date(&date_lit("2020-13-01Z")).is_err());
        assert!(encode_date(&date_lit("2020-00-10Z")).is_err());
        assert!(encode_date(&date_lit("2020-04-31Z")).is_err());
        // Leap rules: 2024 and year 0000 (astronomical, 1 BCE) ARE leap years;
        // 1900 (divisible by 100, not 400) is NOT.
        assert!(encode_date(&date_lit("2024-02-29Z")).is_ok());
        assert!(encode_date(&date_lit("0000-02-29Z")).is_ok());
        assert!(encode_date(&date_lit("1900-02-29Z")).is_err());
        // Year canonicality: no superfluous leading zeros beyond four digits,
        // no "+", no "-0000", no three-digit years.
        assert!(encode_date(&date_lit("02020-01-01Z")).is_err());
        assert!(encode_date(&date_lit("+2020-01-01Z")).is_err());
        assert!(encode_date(&date_lit("-0000-01-01Z")).is_err());
        assert!(encode_date(&date_lit("999-01-01Z")).is_err());
        // A canonical negative (proleptic BCE) year is accepted and lands
        // before the epoch.
        let bce = encode_date(&date_lit("-0001-01-01Z")).unwrap();
        assert_ne!(bce.value_hook, Fr::from(0u64));
        let (neg, _) = canonical_date_scaled("-0001-01-01Z").unwrap();
        assert!(neg);
    }

    #[test]
    fn non_value_lane_datatype_is_rejected() {
        let plain = Literal::new_simple_literal("2020-01-01T00:00:00Z");
        assert!(matches!(
            encode_datetime(&plain),
            Err(DualLeafError::NotValueLane(_))
        ));
        // Cross-lane misuse is NotValueLane, not a parse error.
        assert!(matches!(
            encode_datetime(&date_lit("2020-01-01Z")),
            Err(DualLeafError::NotValueLane(_))
        ));
        assert!(matches!(
            encode_date(&dt_lit("2020-01-01T00:00:00Z")),
            Err(DualLeafError::NotValueLane(_))
        ));
    }
}
