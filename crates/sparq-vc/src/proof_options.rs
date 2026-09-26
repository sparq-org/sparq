//! Lexical validation of the typed Data Integrity proof options.
//!
//! [OPUS-5.5] zkp-14.3. [`check`] is the **single validation seam** every public
//! entry point ([`crate::sign`], [`crate::sign_graph`], [`crate::verify`],
//! [`crate::verify_graph`], [`crate::ProofConfig::validate`]) runs **before** any
//! graph materialization, RDFC-1.0 canonicalization, signing, or
//! `verificationMethod` resolution. Its result, [`CheckedProofConfig`], is the only
//! input the proof-configuration RDF builder accepts, so no unchecked option value
//! reaches the hashed RDF.
//!
//! # The validation contract
//!
//! - **`verificationMethod`** must be an absolute IRI (RFC 3987, parsed by
//!   `oxrdf::NamedNode::new`). It is hashed verbatim; no normalization.
//! - **`proofPurpose`** is either exactly one of the compact terms the Data
//!   Integrity `@context` scopes under `proofPurpose` —
//!   [`SUPPORTED_PURPOSE_TERMS`] — which map to `https://w3id.org/security#<term>`,
//!   or (when the value contains `:`) an absolute IRI hashed verbatim. Any other
//!   value (another bare term, wrong case, empty) is rejected: under the VC v2
//!   `@context` an unknown term would not expand into `sec:`, so guessing a
//!   `sec:` IRI could sign semantics the JSON-LD form does not carry. An absolute
//!   IRI is never appended to the `sec:` namespace. Compact IRIs are not expanded:
//!   `sec:assertionMethod` is read as an IRI with scheme `sec`, so pass the expanded
//!   `https://w3id.org/security#assertionMethod` instead (which hashes identically
//!   to the compact term `assertionMethod`).
//! - **`created`**, when present, must be in the XSD 1.1 `xsd:dateTime` lexical
//!   space ([XSD 1.1 §3.3.7]), as the cryptosuite's proof-configuration algorithm
//!   requires ([vc-di-eddsa §3.3.5]): year zero (a leap year) and negative years
//!   are allowed; the year has at least four digits and no leading zero beyond
//!   four; month/day must be a real calendar date; `24:00:00` is allowed only with
//!   an all-zero fraction; fractions may have any number of digits; the timezone
//!   is optional and at most `±14:00`. No whitespace collapsing is applied, since
//!   the exact literal is what gets signed — surrounding whitespace is rejected.
//!   The accepted literal is hashed exactly as supplied, never normalized.
//! - `domain` and `challenge` are arbitrary strings, hashed as plain literals.
//!
//! There is no narrower capacity limit: the `created` validator is one linear
//! pass over the bytes with constant memory and no arithmetic that can overflow,
//! so arbitrarily long years/fractions and non-ASCII input are handled without
//! panicking. The VC Data Integrity core spec additionally recommends a
//! `dateTimeStamp` (timezone required) for `created`; that stricter profile is
//! **not** enforced here.
//!
//! # What this is not
//!
//! This is **lexical** validation only. It does not decide whether the
//! `verificationMethod` is authorized by its controller for the purpose (issuer
//! key authorization), whether the purpose, `domain`, `challenge`, or `created`
//! match what a verifier expects (freshness, audience, replay window), or any
//! credential status. The signature-only API offers none of those checks; a
//! caller enforces them against [`crate::VerifiedProof::config`]. Unknown JSON-LD
//! proof options are not representable by the typed [`crate::ProofConfig`] at
//! all, so a caller mapping a JSON-LD `proof` node must itself reject options and
//! `@context` mappings this typed subset cannot carry.
//!
//! [XSD 1.1 §3.3.7]: https://www.w3.org/TR/xmlschema11-2/#dateTime
//! [vc-di-eddsa §3.3.5]: https://www.w3.org/TR/2025/REC-vc-di-eddsa-20250515/#proof-configuration-eddsa-rdfc-2022

use oxrdf::NamedNode;

use crate::suite::{ProofConfig, SEC};

/// Compact `proofPurpose` terms mapped into the `https://w3id.org/security#` namespace.
///
/// These are exactly the terms the Data Integrity `@context` defines in the
/// scoped context of `proofPurpose`. Adding a term here changes which inputs are
/// accepted, not how existing ones hash.
pub const SUPPORTED_PURPOSE_TERMS: [&str; 5] = [
    "assertionMethod",
    "authentication",
    "capabilityInvocation",
    "capabilityDelegation",
    "keyAgreement",
];

/// A proof option in a [`ProofConfig`] failed lexical validation.
///
/// Returned (wrapped in [`crate::VcError::InvalidProofOption`]) before any
/// canonicalization, signing, or DID resolution happens. `value` is the rejected
/// input as supplied; `reason` says which rule it broke. The [`Display`] form
/// names the option and reason but does not echo the (possibly huge) value.
///
/// [`Display`]: std::fmt::Display
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProofOptionError {
    /// `verificationMethod` is not an absolute IRI.
    VerificationMethod {
        /// The rejected value.
        value: String,
        /// Why it was rejected.
        reason: String,
    },
    /// `proofPurpose` is neither a supported compact term nor an absolute IRI.
    ProofPurpose {
        /// The rejected value.
        value: String,
        /// Why it was rejected.
        reason: String,
    },
    /// `created` is not an XSD 1.1 `xsd:dateTime` lexical form.
    Created {
        /// The rejected value.
        value: String,
        /// Why it was rejected.
        reason: String,
    },
}

impl std::fmt::Display for ProofOptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (option, reason) = match self {
            ProofOptionError::VerificationMethod { reason, .. } => ("verificationMethod", reason),
            ProofOptionError::ProofPurpose { reason, .. } => ("proofPurpose", reason),
            ProofOptionError::Created { reason, .. } => ("created", reason),
        };
        write!(f, "invalid proof option `{option}`: {reason}")
    }
}

impl std::error::Error for ProofOptionError {}

/// A [`ProofConfig`] whose options passed [`check`], with its IRIs pre-built.
///
/// The proof-configuration RDF builder only accepts this type, so every hashed
/// option went through the one validation seam.
pub(crate) struct CheckedProofConfig<'a> {
    /// The validated configuration; its string fields are hashed verbatim.
    pub(crate) config: &'a ProofConfig,
    /// `verificationMethod`, parsed as an absolute IRI.
    pub(crate) verification_method: NamedNode,
    /// `proofPurpose`, resolved to its absolute IRI.
    pub(crate) proof_purpose: NamedNode,
}

/// Validates every option of `config`, in field order, and pre-builds its IRIs.
pub(crate) fn check(config: &ProofConfig) -> Result<CheckedProofConfig<'_>, ProofOptionError> {
    let verification_method = NamedNode::new(config.verification_method.as_str()).map_err(|e| {
        ProofOptionError::VerificationMethod {
            value: config.verification_method.clone(),
            reason: format!("not an absolute IRI: {e}"),
        }
    })?;
    let proof_purpose = purpose_iri(&config.proof_purpose)?;
    if let Some(created) = &config.created {
        check_xsd11_date_time(created).map_err(|reason| ProofOptionError::Created {
            value: created.clone(),
            reason: reason.to_string(),
        })?;
    }
    Ok(CheckedProofConfig {
        config,
        verification_method,
        proof_purpose,
    })
}

/// Resolves a `proofPurpose` value to its IRI (see the module-level contract).
fn purpose_iri(value: &str) -> Result<NamedNode, ProofOptionError> {
    if SUPPORTED_PURPOSE_TERMS.contains(&value) {
        // A fixed namespace plus a fixed ASCII term: always a valid IRI.
        return Ok(NamedNode::new_unchecked(format!("{SEC}{value}")));
    }
    if value.contains(':') {
        return NamedNode::new(value).map_err(|e| ProofOptionError::ProofPurpose {
            value: value.to_string(),
            reason: format!("not an absolute IRI: {e}"),
        });
    }
    Err(ProofOptionError::ProofPurpose {
        value: value.to_string(),
        reason: format!(
            "unsupported compact term; expected one of {} or an absolute IRI",
            SUPPORTED_PURPOSE_TERMS.join(", ")
        ),
    })
}

// ---------------------------------------------------------------------------
// XSD 1.1 xsd:dateTime lexical validation
// ---------------------------------------------------------------------------

/// Checks `lex` against the XSD 1.1 `dateTimeLexicalRep` plus the day-of-month
/// constraint.
///
/// Grammar (XSD 1.1 Part 2, §3.3.7 and Appendix D.3):
///
/// ```text
/// yearFrag '-' monthFrag '-' dayFrag 'T'
///     ((hourFrag ':' minuteFrag ':' secondFrag) | endOfDayFrag) timezoneFrag?
/// yearFrag     ::= '-'? (([1-9] digit digit digit+) | ('0' digit digit digit))
/// secondFrag   ::= ([0-5] digit) ('.' digit+)?
/// endOfDayFrag ::= '24:00:00' ('.' '0'+)?
/// timezoneFrag ::= 'Z' | ('+' | '-') ((('0' digit | '1' [0-3]) ':' minuteFrag) | '14:00')
/// ```
///
/// Single pass, constant memory: the only arithmetic is the year reduced modulo
/// 400 (enough for the Gregorian leap rule, which is sign-symmetric) and two-digit
/// field values, none of which can overflow.
fn check_xsd11_date_time(lex: &str) -> Result<(), &'static str> {
    let b = lex.as_bytes();
    let mut i = 0;

    // yearFrag
    if b.first() == Some(&b'-') {
        i = 1;
    }
    let year_start = i;
    // |year| mod 400. Max intermediate value is 399 * 10 + 9, well within u16.
    let mut year_mod_400: u16 = 0;
    while let Some(d) = digit_at(b, i) {
        year_mod_400 = (year_mod_400 * 10 + u16::from(d)) % 400;
        i += 1;
    }
    let year_digits = i - year_start;
    if year_digits < 4 {
        return Err("the year must have at least four digits");
    }
    if year_digits > 4 && b.get(year_start) == Some(&b'0') {
        return Err("a year of more than four digits must not start with 0");
    }

    expect(b, &mut i, b'-', "expected `-` after the year")?;
    let month = two_digits(b, &mut i).ok_or("the month must be two digits")?;
    if !(1..=12).contains(&month) {
        return Err("the month must be 01-12");
    }
    expect(b, &mut i, b'-', "expected `-` after the month")?;
    let day = two_digits(b, &mut i).ok_or("the day must be two digits")?;
    if !(1..=days_in_month(year_mod_400, month)).contains(&day) {
        return Err("the day does not exist in that month and year");
    }
    expect(b, &mut i, b'T', "expected `T` between date and time")?;

    let hour = two_digits(b, &mut i).ok_or("the hour must be two digits")?;
    expect(b, &mut i, b':', "expected `:` after the hour")?;
    let minute = two_digits(b, &mut i).ok_or("the minute must be two digits")?;
    expect(b, &mut i, b':', "expected `:` after the minute")?;
    let second = two_digits(b, &mut i).ok_or("the second must be two digits")?;
    let mut fraction_is_zero = true;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let fraction_start = i;
        while let Some(d) = digit_at(b, i) {
            fraction_is_zero &= d == 0;
            i += 1;
        }
        if i == fraction_start {
            return Err("a `.` must be followed by at least one fraction digit");
        }
    }
    if hour == 24 {
        if minute != 0 || second != 0 || !fraction_is_zero {
            return Err("hour 24 is only allowed as 24:00:00 with a zero fraction");
        }
    } else if hour > 23 {
        return Err("the hour must be 00-23 (or 24:00:00)");
    }
    if minute > 59 {
        return Err("the minute must be 00-59");
    }
    if second > 59 {
        return Err("the seconds must be below 60");
    }

    // timezoneFrag?
    match b.get(i) {
        None => return Ok(()),
        Some(b'Z') => i += 1,
        Some(b'+' | b'-') => {
            i += 1;
            let tz_hour = two_digits(b, &mut i).ok_or("the timezone hour must be two digits")?;
            expect(b, &mut i, b':', "expected `:` in the timezone offset")?;
            let tz_minute =
                two_digits(b, &mut i).ok_or("the timezone minute must be two digits")?;
            let in_range = (tz_hour <= 13 && tz_minute <= 59) || (tz_hour == 14 && tz_minute == 0);
            if !in_range {
                return Err("the timezone offset must be within -14:00..+14:00");
            }
        }
        Some(_) => return Err("unexpected character after the seconds"),
    }
    if i == b.len() {
        Ok(())
    } else {
        Err("unexpected trailing characters after the timezone")
    }
}

/// The ASCII digit value at `b[i]`, if any.
fn digit_at(b: &[u8], i: usize) -> Option<u8> {
    b.get(i).filter(|c| c.is_ascii_digit()).map(|c| c - b'0')
}

/// Reads exactly two ASCII digits at `*i`, advancing past them.
fn two_digits(b: &[u8], i: &mut usize) -> Option<u8> {
    let tens = digit_at(b, *i)?;
    let ones = digit_at(b, *i + 1)?;
    *i += 2;
    Some(tens * 10 + ones)
}

/// Consumes the byte `want` at `*i`, or fails with `reason`.
fn expect(b: &[u8], i: &mut usize, want: u8, reason: &'static str) -> Result<(), &'static str> {
    if b.get(*i) == Some(&want) {
        *i += 1;
        Ok(())
    } else {
        Err(reason)
    }
}

/// XSD 1.1 `daysInMonth`, given `|year| mod 400` and a month in 1..=12.
///
/// Divisibility by 4, 100, and 400 is the same for `y` and `-y`, and 4 and 100
/// both divide 400, so the residue decides the leap rule for any year, including
/// year zero (a leap year) and negative years.
fn days_in_month(year_mod_400: u16, month: u8) -> u8 {
    match month {
        2 => {
            let leap = year_mod_400 == 0
                || (year_mod_400.is_multiple_of(4) && !year_mod_400.is_multiple_of(100));
            if leap { 29 } else { 28 }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ok(lex: &str) {
        assert_eq!(check_xsd11_date_time(lex), Ok(()), "should accept {lex:?}");
    }

    fn bad(lex: &str) {
        assert!(check_xsd11_date_time(lex).is_err(), "should reject {lex:?}");
    }

    #[test]
    fn accepts_xsd11_boundaries() {
        // The published W3C vc-di-eddsa proof-configuration value.
        ok("2023-02-24T23:36:38Z");
        // Year zero exists in XSD 1.1 and is a leap year; `-0000` is also lexical.
        ok("0000-01-01T00:00:00Z");
        ok("0000-02-29T12:00:00");
        ok("-0000-06-15T00:00:00Z");
        // Negative (BCE) years, with the sign-symmetric leap rule.
        ok("-0001-12-31T23:59:59Z");
        ok("-0004-02-29T00:00:00+14:00");
        ok("-0400-02-29T00:00:00Z");
        ok("-12345-01-01T00:00:00Z");
        // Gregorian leap rule.
        ok("2000-02-29T00:00:00Z");
        ok("2024-02-29T00:00:00-14:00");
        ok("2023-04-30T00:00:00Z");
        ok("2023-12-31T00:00:00Z");
        // Years beyond four digits, and beyond any machine integer.
        ok("12345-01-01T00:00:00Z");
        ok("1234567890123456789012345678902000-02-29T00:00:00Z");
        // End-of-day midnight, with and without a zero fraction.
        ok("2023-12-31T24:00:00Z");
        ok("2023-02-28T24:00:00.000");
        // Arbitrary fraction digits.
        ok("2023-01-01T00:00:00.5Z");
        ok("2023-01-01T00:00:00.123456789012345678901234567890Z");
        // Optional timezone, full offset range.
        ok("2023-01-01T00:00:00");
        ok("2023-01-01T00:00:00+13:59");
        ok("2023-01-01T00:00:00-00:00");
        ok("2023-06-30T23:59:59.5+05:30");
    }

    #[test]
    fn rejects_xsd11_violations() {
        for lex in [
            "",
            "T",
            // Calendar.
            "2023-02-29T00:00:00Z",
            "1900-02-29T00:00:00Z",
            "-0001-02-29T00:00:00Z",
            "-0100-02-29T00:00:00Z",
            "1234567890123456789012345678901900-02-29T00:00:00Z",
            "2023-04-31T00:00:00Z",
            "2023-13-01T00:00:00Z",
            "2023-00-10T00:00:00Z",
            "2023-01-00T00:00:00Z",
            "2023-01-32T00:00:00Z",
            // Year form.
            "023-01-01T00:00:00Z",
            "02023-01-01T00:00:00Z",
            "-02023-01-01T00:00:00Z",
            "+2023-01-01T00:00:00Z",
            "--2023-01-01T00:00:00Z",
            "-T",
            // Field widths and separators.
            "2023-1-01T00:00:00Z",
            "2023-01-01T0:00:00Z",
            "2023-01-01t00:00:00Z",
            "2023-01-01 00:00:00Z",
            "2023-01-01",
            "2023-01-01T00:00Z",
            // Time ranges.
            "2023-01-01T24:00:01Z",
            "2023-01-01T24:01:00Z",
            "2023-01-01T24:00:00.1Z",
            "2023-01-01T25:00:00Z",
            "2023-01-01T00:60:00Z",
            "2023-01-01T00:00:60Z",
            "2023-01-01T00:00:00.Z",
            // Timezone.
            "2023-01-01T00:00:00+14:01",
            "2023-01-01T00:00:00+15:00",
            "2023-01-01T00:00:00+1400",
            "2023-01-01T00:00:00+05",
            "2023-01-01T00:00:00+05:60",
            "2023-01-01T00:00:00z",
            "2023-01-01T00:00:00ZZ",
            "2023-01-01T00:00:00Z+00:00",
            // No whitespace collapsing, since the literal is signed verbatim.
            " 2023-01-01T00:00:00Z",
            "2023-01-01T00:00:00Z ",
            // Non-ASCII digits and bytes.
            "\u{ff12}\u{ff10}\u{ff12}\u{ff13}-01-01T00:00:00Z",
            "2023-01-01T00:00:00\u{2212}05:00",
            "2023-01-01T00:00:00Z\u{0}",
        ] {
            bad(lex);
        }
    }

    /// Million-digit years and fractions stay in one linear pass with no overflow.
    #[test]
    fn huge_inputs_are_handled_without_overflow() {
        let huge_zeros = "0".repeat(1_000_000);
        // 10^1_000_000 is divisible by 400, so its February has 29 days.
        ok(&format!("1{huge_zeros}-02-29T00:00:00Z"));
        bad(&format!("1{huge_zeros}1-02-29T00:00:00Z"));
        bad(&format!("0{huge_zeros}-01-01T00:00:00Z"));
        ok(&format!("2023-12-31T24:00:00.{huge_zeros}Z"));
        bad(&format!("2023-12-31T24:00:00.{huge_zeros}1Z"));
        bad(&"\u{1f600}".repeat(250_000));
    }

    #[test]
    fn purpose_terms_map_into_sec_and_absolute_iris_are_verbatim() {
        for term in SUPPORTED_PURPOSE_TERMS {
            assert_eq!(purpose_iri(term).unwrap().as_str(), format!("{SEC}{term}"));
        }
        let absolute = "https://example.test/purposes#audit";
        assert_eq!(purpose_iri(absolute).unwrap().as_str(), absolute);
        assert_eq!(
            purpose_iri("https://w3id.org/security#assertionMethod").unwrap(),
            purpose_iri("assertionMethod").unwrap()
        );
        for rejected in [
            "",
            "assertionmethod",
            "AssertionMethod",
            "foo",
            "assertionMethod ",
        ] {
            assert!(
                matches!(
                    purpose_iri(rejected),
                    Err(ProofOptionError::ProofPurpose { .. })
                ),
                "{rejected:?}"
            );
        }
        assert!(matches!(
            purpose_iri("https://exa mple.test/p"),
            Err(ProofOptionError::ProofPurpose { .. })
        ));
    }

    /// Decimal digits `0..=9` rendered as an ASCII string.
    fn digits(ds: Vec<u8>) -> String {
        ds.into_iter().map(|d| char::from(b'0' + d)).collect()
    }

    /// A strategy for valid XSD 1.1 dateTime lexical forms.
    fn valid_date_time() -> impl Strategy<Value = String> {
        let year = (
            any::<bool>(),
            prop_oneof![
                (0u16..=9999).prop_map(|y| format!("{y:04}")),
                (1u8..=9, prop::collection::vec(0u8..=9, 4..=40))
                    .prop_map(|(first, rest)| format!("{first}{}", digits(rest))),
            ],
        )
            .prop_map(|(neg, y)| if neg { format!("-{y}") } else { y });
        let fraction = prop_oneof![
            Just(String::new()),
            prop::collection::vec(0u8..=9, 1..=40).prop_map(|ds| format!(".{}", digits(ds))),
        ];
        let tz = prop_oneof![
            Just(String::new()),
            Just("Z".to_string()),
            (any::<bool>(), 0u8..=13, 0u8..=59).prop_map(|(neg, h, m)| {
                format!("{}{h:02}:{m:02}", if neg { '-' } else { '+' })
            }),
            any::<bool>().prop_map(|neg| format!("{}14:00", if neg { '-' } else { '+' })),
        ];
        (
            year,
            1u8..=12,
            1u8..=28,
            0u8..=23,
            0u8..=59,
            0u8..=59,
            fraction,
            tz,
        )
            .prop_map(|(y, mo, d, h, mi, s, frac, tz)| {
                format!("{y}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}{frac}{tz}")
            })
    }

    proptest! {
        #[test]
        fn arbitrary_strings_never_panic(s in any::<String>()) {
            let _ = check_xsd11_date_time(&s);
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
            let s = String::from_utf8_lossy(&bytes);
            let _ = check_xsd11_date_time(&s);
        }

        #[test]
        fn generated_valid_values_are_accepted(lex in valid_date_time()) {
            prop_assert_eq!(check_xsd11_date_time(&lex), Ok(()));
        }

        #[test]
        fn one_byte_edits_of_valid_values_never_panic(
            lex in valid_date_time(),
            at in any::<prop::sample::Index>(),
            byte in any::<u8>(),
            delete in any::<bool>(),
        ) {
            let mut bytes = lex.into_bytes();
            let at = at.index(bytes.len() + 1);
            if delete && at < bytes.len() {
                bytes.remove(at);
            } else {
                bytes.insert(at, byte);
            }
            let _ = check_xsd11_date_time(&String::from_utf8_lossy(&bytes));
        }

        #[test]
        fn out_of_range_fields_are_rejected(
            field in 0usize..5,
            value in 0u8..=99,
        ) {
            let (mo, d, h, mi, s) = match field {
                0 => (value, 1, 0, 0, 0),
                1 => (4, value, 0, 0, 0),
                2 => (1, 1, value, 0, 0),
                3 => (1, 1, 0, value, 0),
                _ => (1, 1, 0, 0, value),
            };
            let expected_ok = match field {
                0 => (1..=12).contains(&value),
                1 => (1..=30).contains(&value),
                2 => value <= 24,
                _ => value <= 59,
            };
            let lex = format!("2023-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z");
            prop_assert_eq!(check_xsd11_date_time(&lex).is_ok(), expected_ok, "{}", lex);
        }
    }
}

/// The validation seam exercised through the public sign/verify entry points.
#[cfg(test)]
mod api_tests {
    use std::cell::Cell;

    use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
    use proptest::prelude::*;
    use sparq_core::Graph;

    use super::{ProofOptionError, SUPPORTED_PURPOSE_TERMS};
    use crate::did::{Did, DidError, DidKeyResolver, DidResolver};
    use crate::{
        ProofConfig, SigningKey, VcError, VerifiedProof, VerifyingKey, sign, sign_graph, verify,
        verify_graph,
    };

    /// XSD 1.1 `xsd:dateTime` values at the edges of the lexical space.
    const VALID_CREATED: &[&str] = &[
        "2023-02-24T23:36:38Z",
        "0000-01-01T00:00:00Z",
        "0000-02-29T12:00:00",
        "-0001-12-31T23:59:59Z",
        "-0004-02-29T00:00:00+14:00",
        "2000-02-29T00:00:00Z",
        "2024-02-29T00:00:00-14:00",
        "12345-01-01T00:00:00Z",
        "2023-12-31T24:00:00Z",
        "2023-12-31T24:00:00.000Z",
        "2023-01-01T00:00:00.123456789012345678901234567890Z",
        "2023-01-01T00:00:00",
        "2023-01-01T00:00:00-00:00",
    ];

    /// Values outside the XSD 1.1 `xsd:dateTime` lexical space.
    const INVALID_CREATED: &[&str] = &[
        "",
        "2023-02-29T00:00:00Z",
        "1900-02-29T00:00:00Z",
        "-0001-02-29T00:00:00Z",
        "2023-04-31T00:00:00Z",
        "2023-13-01T00:00:00Z",
        "023-01-01T00:00:00Z",
        "02023-01-01T00:00:00Z",
        "+2023-01-01T00:00:00Z",
        "2023-01-01",
        "2023-01-01T24:00:01Z",
        "2023-01-01T24:00:00.5Z",
        "2023-01-01T00:00:60Z",
        "2023-01-01T00:00:00.Z",
        "2023-01-01T00:00:00+14:01",
        "2023-01-01T00:00:00+1400",
        " 2023-01-01T00:00:00Z",
        "\u{ff12}\u{ff10}\u{ff12}\u{ff13}-01-01T00:00:00Z",
    ];

    fn key() -> SigningKey {
        SigningKey::from_seed(&[31_u8; 32])
    }

    fn vm_for(key: &SigningKey) -> String {
        let did = key.did_key();
        format!("{did}#{}", did.strip_prefix("did:key:").unwrap())
    }

    fn triples() -> Vec<Triple> {
        vec![Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("https://example.test/s")),
            NamedNode::new_unchecked("https://example.test/p"),
            Term::Literal(Literal::new_simple_literal("o")),
        )]
    }

    /// A document RDFC-1.0 rejects (an RDF 1.2 triple term), so reaching the
    /// canonicalizer is observable as `VcError::Canon`.
    fn uncanonicalizable() -> Vec<Triple> {
        let inner = triples().remove(0);
        vec![Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new_unchecked("https://example.test/r")),
            NamedNode::new_unchecked("https://example.test/about"),
            Term::Triple(Box::new(inner)),
        )]
    }

    /// A `did:key` resolver that counts how often it is consulted.
    #[derive(Default)]
    struct CountingResolver(Cell<usize>);

    impl DidResolver for CountingResolver {
        fn resolve(&self, did: &Did) -> Result<VerifyingKey, DidError> {
            self.0.set(self.0.get() + 1);
            DidKeyResolver.resolve(did)
        }
    }

    fn invalid_option<T: std::fmt::Debug>(result: Result<T, VcError>) -> ProofOptionError {
        match result {
            Err(VcError::InvalidProofOption(e)) => e,
            other => panic!("expected InvalidProofOption, got {other:?}"),
        }
    }

    fn verified(result: Result<VerifiedProof, VcError>) -> VerifiedProof {
        result.expect("proof verifies")
    }

    fn with_purpose(purpose: &str, base: &ProofConfig) -> ProofConfig {
        ProofConfig {
            proof_purpose: purpose.to_string(),
            ..base.clone()
        }
    }

    #[test]
    fn boundary_created_values_round_trip_verbatim() {
        let key = key();
        for &created in VALID_CREATED {
            let cfg = ProofConfig::new(vm_for(&key)).with_created(created);
            assert_eq!(cfg.validate(), Ok(()), "{created:?}");
            let proof = sign(&triples(), &key, &cfg).expect("valid created signs");
            assert_eq!(proof.config.created.as_deref(), Some(created));
            let v = verified(verify(&triples(), &proof, &DidKeyResolver));
            assert_eq!(v.config.created.as_deref(), Some(created));
        }
    }

    /// The literal is signed as written: equal-valued lexical forms are
    /// different signed statements.
    #[test]
    fn created_is_not_normalized() {
        let key = key();
        let signed = ProofConfig::new(vm_for(&key)).with_created("2023-12-31T24:00:00Z");
        let mut proof = sign(&triples(), &key, &signed).unwrap();
        for equal_value in [
            "2024-01-01T00:00:00Z",
            "2023-12-31T24:00:00.0Z",
            "2023-12-31T24:00:00+00:00",
        ] {
            proof.config.created = Some(equal_value.to_string());
            assert!(
                matches!(
                    verify(&triples(), &proof, &DidKeyResolver),
                    Err(VcError::SignatureInvalid)
                ),
                "{equal_value:?}"
            );
        }
    }

    #[test]
    fn invalid_created_is_rejected_by_every_entry_point() {
        let key = key();
        let graph = Graph::new();
        let good = sign(&triples(), &key, &ProofConfig::new(vm_for(&key))).unwrap();
        for &created in INVALID_CREATED {
            let cfg = ProofConfig::new(vm_for(&key)).with_created(created);
            let expected = cfg.validate().expect_err(created);
            assert!(
                matches!(&expected, ProofOptionError::Created { value, .. } if value == created),
                "{expected:?}"
            );
            assert_eq!(invalid_option(sign(&triples(), &key, &cfg)), expected);
            assert_eq!(invalid_option(sign_graph(&graph, &key, &cfg)), expected);

            let mut proof = good.clone();
            proof.config = cfg;
            let resolver = CountingResolver::default();
            assert_eq!(
                invalid_option(verify(&triples(), &proof, &resolver)),
                expected
            );
            assert_eq!(
                invalid_option(verify_graph(&graph, &proof, &resolver)),
                expected
            );
            assert_eq!(resolver.0.get(), 0, "resolver consulted for {created:?}");
        }
    }

    #[test]
    fn verification_method_must_be_an_absolute_iri() {
        let key = key();
        for vm in [
            "",
            "#key",
            "relative/key",
            "z6MkrJVnaZkeFzdQyMZu1cgjg7k1pZZ6pvBQ7XJPt4swbTQ2",
            "did key",
            "https://exa mple.test/k",
        ] {
            let cfg = ProofConfig::new(vm);
            let err = invalid_option(sign(&triples(), &key, &cfg));
            assert!(
                matches!(&err, ProofOptionError::VerificationMethod { value, .. } if value == vm),
                "{vm:?}: {err:?}"
            );
            assert_eq!(cfg.validate(), Err(err));
        }
    }

    /// Lexical validity is not resolution: a well-formed IRI that is not a
    /// `did:key` passes validation, then fails in the resolver.
    #[test]
    fn lexically_valid_method_still_goes_through_resolution() {
        let key = key();
        let cfg = ProofConfig::new("https://issuer.example/keys/1");
        assert_eq!(cfg.validate(), Ok(()));
        let proof = sign(&triples(), &key, &cfg).expect("well-formed options sign");
        assert!(matches!(
            verify(&triples(), &proof, &DidKeyResolver),
            Err(VcError::Did(DidError::Malformed(_)))
        ));
    }

    #[test]
    fn purpose_accepts_supported_terms_and_absolute_iris_only() {
        let key = key();
        let base = ProofConfig::new(vm_for(&key));
        for term in SUPPORTED_PURPOSE_TERMS {
            let proof = sign(&triples(), &key, &with_purpose(term, &base)).unwrap();
            verified(verify(&triples(), &proof, &DidKeyResolver));
        }

        let custom = with_purpose("https://example.test/purposes#audit", &base);
        let proof = sign(&triples(), &key, &custom).unwrap();
        verified(verify(&triples(), &proof, &DidKeyResolver));

        for purpose in [
            "",
            "foo",
            "assertionmethod",
            "AssertionMethod",
            "assertionMethod ",
            "https://exa mple.test/p",
        ] {
            let err = invalid_option(sign(&triples(), &key, &with_purpose(purpose, &base)));
            assert!(
                matches!(&err, ProofOptionError::ProofPurpose { value, .. } if value == purpose),
                "{purpose:?}: {err:?}"
            );
        }
    }

    /// The compact term and its expanded `sec:` IRI are the same RDF, so they
    /// hash (and deterministically sign) identically.
    #[test]
    fn compact_and_expanded_purpose_sign_identically() {
        let key = key();
        let compact = ProofConfig::new(vm_for(&key));
        let expanded = with_purpose("https://w3id.org/security#assertionMethod", &compact);
        let a = sign(&triples(), &key, &compact).unwrap();
        let b = sign(&triples(), &key, &expanded).unwrap();
        assert_eq!(a.proof_value, b.proof_value);
    }

    /// Invalid options are reported before the canonicalizer or resolver runs;
    /// valid ones reach them.
    #[test]
    fn invalid_options_fail_before_canonicalization_and_resolution() {
        let key = key();
        let doc = uncanonicalizable();
        let valid = ProofConfig::new(vm_for(&key));
        assert!(matches!(
            sign(&doc, &key, &valid),
            Err(VcError::Canon(sparq_canon::CanonError::TripleTerm))
        ));

        let bad_created = valid.clone().with_created("2023-02-30T00:00:00Z");
        invalid_option(sign(&doc, &key, &bad_created));
        invalid_option(sign(&doc, &key, &with_purpose("unsupported", &valid)));

        let mut proof = sign(&triples(), &key, &valid).unwrap();
        let resolver = CountingResolver::default();
        assert!(matches!(
            verify(&doc, &proof, &resolver),
            Err(VcError::Canon(sparq_canon::CanonError::TripleTerm))
        ));
        assert_eq!(resolver.0.get(), 1);

        proof.config = bad_created;
        invalid_option(verify(&doc, &proof, &resolver));
        assert_eq!(
            resolver.0.get(),
            1,
            "resolver consulted for an invalid config"
        );
    }

    #[test]
    fn error_display_names_the_option_without_echoing_the_value() {
        let cfg = ProofConfig::new("did:key:z6Mk").with_created("2023-02-30T00:00:00Z");
        let message = VcError::InvalidProofOption(cfg.validate().unwrap_err()).to_string();
        assert!(
            message.starts_with("invalid proof option `created`: "),
            "{message}"
        );
        assert!(!message.contains("2023-02-30"), "{message}");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Any `created` string either signs and verifies with the literal
        /// intact, or is rejected as an invalid `created` option.
        #[test]
        fn arbitrary_created_signs_verbatim_or_is_rejected(created in any::<String>()) {
            let key = key();
            let cfg = ProofConfig::new(vm_for(&key)).with_created(created.clone());
            match sign(&triples(), &key, &cfg) {
                Ok(proof) => {
                    prop_assert_eq!(proof.config.created.as_deref(), Some(created.as_str()));
                    prop_assert!(verify(&triples(), &proof, &DidKeyResolver).is_ok());
                }
                Err(VcError::InvalidProofOption(ProofOptionError::Created { value, .. })) => {
                    prop_assert_eq!(value, created);
                }
                Err(other) => prop_assert!(false, "unexpected error {:?}", other),
            }
        }
    }
}
