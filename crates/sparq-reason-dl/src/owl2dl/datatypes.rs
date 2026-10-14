//! The OWL 2 datatype map (OWL 2 Structural Specification §4): which datatypes exist, their
//! lexical spaces, their value spaces, and which constraining facets apply to each.
//!
//! [`parse_value`] maps a literal to its data value (or reports it ill-typed), and
//! [`in_value_space`] decides membership of a value in a datatype's value space. Values are
//! exact: integers and decimals are rationals over `i128`, so `"1.0"^^xsd:decimal` and
//! `"1"^^xsd:integer` are the same value, as the specification requires. A literal whose value
//! does not fit (an integer beyond `i128`) is reported as [`ValueError::Unsupported`], never
//! approximated.

use std::cmp::Ordering;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";

/// A datatype of the OWL 2 datatype map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum Builtin {
    RdfsLiteral,
    // numbers
    OwlReal,
    OwlRational,
    Decimal,
    Integer,
    NonNegativeInteger,
    NonPositiveInteger,
    PositiveInteger,
    NegativeInteger,
    Long,
    Int,
    Short,
    Byte,
    UnsignedLong,
    UnsignedInt,
    UnsignedShort,
    UnsignedByte,
    Double,
    Float,
    // strings
    PlainLiteral,
    LangString,
    String,
    NormalizedString,
    Token,
    Language,
    Name,
    NcName,
    NmToken,
    // other
    Boolean,
    HexBinary,
    Base64Binary,
    AnyUri,
    DateTime,
    DateTimeStamp,
    XmlLiteral,
}

const BUILTINS: &[(&str, &str, Builtin)] = &[
    (RDFS, "Literal", Builtin::RdfsLiteral),
    (OWL, "real", Builtin::OwlReal),
    (OWL, "rational", Builtin::OwlRational),
    (XSD, "decimal", Builtin::Decimal),
    (XSD, "integer", Builtin::Integer),
    (XSD, "nonNegativeInteger", Builtin::NonNegativeInteger),
    (XSD, "nonPositiveInteger", Builtin::NonPositiveInteger),
    (XSD, "positiveInteger", Builtin::PositiveInteger),
    (XSD, "negativeInteger", Builtin::NegativeInteger),
    (XSD, "long", Builtin::Long),
    (XSD, "int", Builtin::Int),
    (XSD, "short", Builtin::Short),
    (XSD, "byte", Builtin::Byte),
    (XSD, "unsignedLong", Builtin::UnsignedLong),
    (XSD, "unsignedInt", Builtin::UnsignedInt),
    (XSD, "unsignedShort", Builtin::UnsignedShort),
    (XSD, "unsignedByte", Builtin::UnsignedByte),
    (XSD, "double", Builtin::Double),
    (XSD, "float", Builtin::Float),
    (RDF, "PlainLiteral", Builtin::PlainLiteral),
    (RDF, "langString", Builtin::LangString),
    (XSD, "string", Builtin::String),
    (XSD, "normalizedString", Builtin::NormalizedString),
    (XSD, "token", Builtin::Token),
    (XSD, "language", Builtin::Language),
    (XSD, "Name", Builtin::Name),
    (XSD, "NCName", Builtin::NcName),
    (XSD, "NMTOKEN", Builtin::NmToken),
    (XSD, "boolean", Builtin::Boolean),
    (XSD, "hexBinary", Builtin::HexBinary),
    (XSD, "base64Binary", Builtin::Base64Binary),
    (XSD, "anyURI", Builtin::AnyUri),
    (XSD, "dateTime", Builtin::DateTime),
    (XSD, "dateTimeStamp", Builtin::DateTimeStamp),
    (RDF, "XMLLiteral", Builtin::XmlLiteral),
];

/// The datatype-map datatype named by `iri`, if any.
#[must_use]
pub fn builtin(iri: &str) -> Option<Builtin> {
    BUILTINS
        .iter()
        .find(|(ns, local, _)| {
            iri.len() == ns.len() + local.len() && iri.starts_with(ns) && iri.ends_with(local)
        })
        .map(|&(_, _, b)| b)
}

/// The IRI of a datatype-map datatype.
#[must_use]
pub fn builtin_iri(b: Builtin) -> String {
    let (ns, local, _) = BUILTINS
        .iter()
        .find(|(_, _, x)| *x == b)
        .expect("every builtin is listed");
    format!("{ns}{local}")
}

/// `true` when `iri` names a datatype of the OWL 2 datatype map (including `rdfs:Literal`).
#[must_use]
pub fn is_datatype_map_iri(iri: &str) -> bool {
    builtin(iri).is_some()
}

/// A constraining facet (OWL 2 Structural Specification §4, Table 3 and the per-datatype
/// sections).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum Facet {
    MinInclusive,
    MaxInclusive,
    MinExclusive,
    MaxExclusive,
    Length,
    MinLength,
    MaxLength,
    Pattern,
    LangRange,
}

/// The facet named by `iri`, if any.
#[must_use]
pub fn facet(iri: &str) -> Option<Facet> {
    if iri == format!("{RDF}langRange") {
        return Some(Facet::LangRange);
    }
    let local = iri.strip_prefix(XSD)?;
    Some(match local {
        "minInclusive" => Facet::MinInclusive,
        "maxInclusive" => Facet::MaxInclusive,
        "minExclusive" => Facet::MinExclusive,
        "maxExclusive" => Facet::MaxExclusive,
        "length" => Facet::Length,
        "minLength" => Facet::MinLength,
        "maxLength" => Facet::MaxLength,
        "pattern" => Facet::Pattern,
        _ => return None,
    })
}

/// `true` for an XSD facet IRI (`xsd:minInclusive`, … — also the facets OWL 2 does not use,
/// such as `xsd:totalDigits`, which are still not user properties).
#[must_use]
pub fn is_facet_iri(iri: &str) -> bool {
    facet(iri).is_some()
        || iri.strip_prefix(XSD).is_some_and(|l| {
            matches!(
                l,
                "totalDigits"
                    | "fractionDigits"
                    | "whiteSpace"
                    | "enumeration"
                    | "assertion"
                    | "explicitTimezone"
            )
        })
}

/// The value-space family of a datatype. Families are pairwise disjoint (OWL 2 §4: the
/// numeric, string, boolean, binary, IRI, time and XML value spaces share no values;
/// `xsd:double` and `xsd:float` are disjoint from the reals and from each other).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(missing_docs)]
pub enum Family {
    Real,
    Double,
    Float,
    /// Strings without a language tag.
    String,
    /// Strings with a language tag.
    LangString,
    Boolean,
    HexBinary,
    Base64Binary,
    AnyUri,
    DateTime,
    XmlLiteral,
}

impl Builtin {
    /// The families whose values this datatype's value space contains (one family for every
    /// datatype but `rdfs:Literal` and `rdf:PlainLiteral`).
    #[must_use]
    pub fn families(self) -> &'static [Family] {
        use Builtin::*;
        match self {
            RdfsLiteral => &[
                Family::Real,
                Family::Double,
                Family::Float,
                Family::String,
                Family::LangString,
                Family::Boolean,
                Family::HexBinary,
                Family::Base64Binary,
                Family::AnyUri,
                Family::DateTime,
                Family::XmlLiteral,
            ],
            OwlReal | OwlRational | Decimal | Integer | NonNegativeInteger | NonPositiveInteger
            | PositiveInteger | NegativeInteger | Long | Int | Short | Byte | UnsignedLong
            | UnsignedInt | UnsignedShort | UnsignedByte => &[Family::Real],
            Double => &[Family::Double],
            Float => &[Family::Float],
            PlainLiteral => &[Family::String, Family::LangString],
            LangString => &[Family::LangString],
            String | NormalizedString | Token | Language | Name | NcName | NmToken => {
                &[Family::String]
            }
            Boolean => &[Family::Boolean],
            HexBinary => &[Family::HexBinary],
            Base64Binary => &[Family::Base64Binary],
            AnyUri => &[Family::AnyUri],
            DateTime | DateTimeStamp => &[Family::DateTime],
            XmlLiteral => &[Family::XmlLiteral],
        }
    }

    /// The facets OWL 2 allows on this datatype.
    #[must_use]
    pub fn allowed_facets(self) -> &'static [Facet] {
        use Builtin::*;
        const ORDER: &[Facet] = &[
            Facet::MinInclusive,
            Facet::MaxInclusive,
            Facet::MinExclusive,
            Facet::MaxExclusive,
        ];
        const LENGTH: &[Facet] = &[
            Facet::Length,
            Facet::MinLength,
            Facet::MaxLength,
            Facet::Pattern,
        ];
        // OWL 2 §4.5: the binary types take the length facets only.
        const BINARY: &[Facet] = &[Facet::Length, Facet::MinLength, Facet::MaxLength];
        const PLAIN: &[Facet] = &[
            Facet::Length,
            Facet::MinLength,
            Facet::MaxLength,
            Facet::Pattern,
            Facet::LangRange,
        ];
        match self {
            OwlReal | OwlRational | Decimal | Integer | NonNegativeInteger | NonPositiveInteger
            | PositiveInteger | NegativeInteger | Long | Int | Short | Byte | UnsignedLong
            | UnsignedInt | UnsignedShort | UnsignedByte | Double | Float | DateTime
            | DateTimeStamp => ORDER,
            String | NormalizedString | Token | Language | Name | NcName | NmToken | AnyUri => {
                LENGTH
            }
            HexBinary | Base64Binary => BINARY,
            PlainLiteral => PLAIN,
            RdfsLiteral | LangString | Boolean | XmlLiteral => &[],
        }
    }
}

// -------------------------------------------------------------------------------------------
// exact rationals
// -------------------------------------------------------------------------------------------

/// An exact rational `num / den` with `den > 0`, `gcd(num, den) == 1` and
/// `num != i128::MIN` (so negation, floor, ceiling and comparison never overflow).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    /// Numerator.
    pub num: i128,
    /// Denominator (positive).
    pub den: i128,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

impl Rational {
    /// `num / den`, normalised; `None` when `den == 0` or either part is `i128::MIN` (outside
    /// the represented range).
    #[must_use]
    pub fn new(num: i128, den: i128) -> Option<Rational> {
        if den == 0 || num == i128::MIN || den == i128::MIN {
            return None;
        }
        let g = gcd(num, den).max(1);
        let (mut n, mut d) = (num / g, den / g);
        if d < 0 {
            n = n.checked_neg()?;
            d = d.checked_neg()?;
        }
        Some(Rational { num: n, den: d })
    }

    /// The integer `n`; `None` for `i128::MIN` (outside the represented range).
    #[must_use]
    pub fn int(n: i128) -> Option<Rational> {
        (n != i128::MIN).then_some(Rational { num: n, den: 1 })
    }

    /// `true` when the value is an integer.
    #[must_use]
    pub fn is_integer(&self) -> bool {
        self.den == 1
    }

    /// `true` when the value has a finite decimal expansion (the denominator has no prime
    /// factors other than 2 and 5).
    #[must_use]
    pub fn is_decimal(&self) -> bool {
        let mut d = self.den;
        while d % 2 == 0 {
            d /= 2;
        }
        while d % 5 == 0 {
            d /= 5;
        }
        d == 1
    }

    /// The largest integer `≤ self`.
    #[must_use]
    pub fn floor(&self) -> i128 {
        self.num.div_euclid(self.den)
    }

    /// The smallest integer `≥ self`.
    #[must_use]
    pub fn ceil(&self) -> i128 {
        // `num != i128::MIN`, so `-num` and the negated quotient are in range.
        -(-self.num).div_euclid(self.den)
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Rational) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Rational) -> Ordering {
        // Compare a/b with c/d without overflow: compare floors, then remainders.
        let (fa, fb) = (self.floor(), other.floor());
        if fa != fb {
            return fa.cmp(&fb);
        }
        // Same integer part: compare the fractional parts ra/b and rc/d in [0, 1).
        let ra = self.num.rem_euclid(self.den);
        let rc = other.num.rem_euclid(other.den);
        frac_cmp(ra, self.den, rc, other.den)
    }
}

/// Exact comparison of `a/b` and `c/d` with `0 ≤ a < b` and `0 ≤ c < d`, by continued
/// fractions: no product is formed, so nothing overflows.
fn frac_cmp(mut a: i128, mut b: i128, mut c: i128, mut d: i128) -> Ordering {
    let mut flipped = false;
    loop {
        let out = match (a == 0, c == 0) {
            (true, true) => Some(Ordering::Equal),
            (true, false) => Some(Ordering::Less),
            (false, true) => Some(Ordering::Greater),
            (false, false) => None,
        };
        if let Some(o) = out {
            return if flipped { o.reverse() } else { o };
        }
        // a/b vs c/d  ⇔  reversed(b/a vs d/c); compare the integer parts, then recurse on the
        // fractional remainders.
        let (qa, qc) = (b / a, d / c);
        flipped = !flipped;
        if qa != qc {
            let o = qa.cmp(&qc);
            return if flipped { o.reverse() } else { o };
        }
        (a, b, c, d) = (b % a, a, d % c, c);
    }
}

// -------------------------------------------------------------------------------------------
// values
// -------------------------------------------------------------------------------------------

/// A time instant or local date-time: the seconds since 0001-01-01T00:00:00 of the local
/// (or UTC-normalised) time, exact to the parsed precision, and the timezone offset in minutes
/// when one was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DateTimeValue {
    /// Seconds (UTC-normalised when `tz` is present).
    pub seconds: RationalKey,
    /// `Some(offset)` for a timezoned value.
    pub tz: Option<i16>,
}

/// A totally ordered key wrapper for [`Rational`] (derives the ordering of the rational).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RationalKey(pub Rational);

impl PartialOrd for RationalKey {
    fn partial_cmp(&self, other: &RationalKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RationalKey {
    fn cmp(&self, other: &RationalKey) -> Ordering {
        self.0.cmp(&other.0)
    }
}

/// A data value of the OWL 2 datatype map. Values from different families are never equal.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Value {
    /// A real number (owl:real and every numeric XSD type but double/float), exact.
    Real(RationalKey),
    /// An `xsd:double`, by its bits (NaN canonicalised; +0 and -0 distinct).
    Double(u64),
    /// An `xsd:float`, by its bits (NaN canonicalised; +0 and -0 distinct).
    Float(u32),
    /// A string without a language tag.
    String(String),
    /// A string with a (lower-cased) language tag.
    LangString(String, String),
    /// A boolean.
    Boolean(bool),
    /// An `xsd:hexBinary` octet sequence.
    HexBinary(Vec<u8>),
    /// An `xsd:base64Binary` octet sequence.
    Base64Binary(Vec<u8>),
    /// An `xsd:anyURI`.
    AnyUri(String),
    /// An `xsd:dateTime` / `xsd:dateTimeStamp`.
    DateTime(DateTimeValue),
    /// An `rdf:XMLLiteral`, by its lexical form.
    XmlLiteral(String),
}

impl Value {
    /// The family the value belongs to.
    #[must_use]
    pub fn family(&self) -> Family {
        match self {
            Value::Real(_) => Family::Real,
            Value::Double(_) => Family::Double,
            Value::Float(_) => Family::Float,
            Value::String(_) => Family::String,
            Value::LangString(..) => Family::LangString,
            Value::Boolean(_) => Family::Boolean,
            Value::HexBinary(_) => Family::HexBinary,
            Value::Base64Binary(_) => Family::Base64Binary,
            Value::AnyUri(_) => Family::AnyUri,
            Value::DateTime(_) => Family::DateTime,
            Value::XmlLiteral(_) => Family::XmlLiteral,
        }
    }
}

/// Why a literal has no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueError {
    /// The lexical form is not in the datatype's lexical space (an ill-typed literal).
    IllTyped,
    /// The datatype is not in the OWL 2 datatype map (a user datatype, or one OWL 2 does not
    /// support such as `xsd:date`).
    UnknownDatatype,
    /// Valid, but outside what this implementation represents exactly.
    Unsupported,
}

fn collapse(s: &str) -> String {
    s.split([' ', '\t', '\n', '\r'])
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn replace_ws(s: &str) -> String {
    s.chars()
        .map(|c| {
            if matches!(c, '\t' | '\n' | '\r') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// Parses an XSD integer lexical form (after whitespace collapse) to `i128`.
#[must_use]
pub fn parse_integer(lex: &str) -> Option<i128> {
    let s = collapse(lex);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s.as_str()),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut v: i128 = 0;
    for b in digits.bytes() {
        v = v.checked_mul(10)?.checked_add(i128::from(b - b'0'))?;
    }
    Some(if neg { -v } else { v })
}

/// `Err(IllTyped)` when not an integer lexical form, `Err(Unsupported)` when out of `i128`.
fn integer_value(lex: &str) -> Result<i128, ValueError> {
    let s = collapse(lex);
    let body = s.strip_prefix(['+', '-']).unwrap_or(&s);
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ValueError::IllTyped);
    }
    parse_integer(&s).ok_or(ValueError::Unsupported)
}

fn decimal_value(lex: &str) -> Result<Rational, ValueError> {
    let s = collapse(lex);
    let (neg, body) = match s.as_bytes().first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s.as_str()),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, f),
        None => (body, ""),
    };
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(ValueError::IllTyped);
    }
    let frac = frac.trim_end_matches('0');
    let mut num: i128 = 0;
    for b in int.bytes().chain(frac.bytes()) {
        num = num
            .checked_mul(10)
            .and_then(|n| n.checked_add(i128::from(b - b'0')))
            .ok_or(ValueError::Unsupported)?;
    }
    let den = 10i128
        .checked_pow(u32::try_from(frac.len()).map_err(|_| ValueError::Unsupported)?)
        .ok_or(ValueError::Unsupported)?;
    Rational::new(if neg { -num } else { num }, den).ok_or(ValueError::Unsupported)
}

fn float_lexical_ok(s: &str) -> bool {
    if matches!(s, "INF" | "+INF" | "-INF" | "NaN") {
        return true;
    }
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let m = mantissa.strip_prefix(['+', '-']).unwrap_or(mantissa);
    let (int, frac) = m.split_once('.').unwrap_or((m, ""));
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return false;
    }
    match exp {
        None => true,
        Some(e) => {
            let e = e.strip_prefix(['+', '-']).unwrap_or(e);
            !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

fn double_value(lex: &str) -> Result<f64, ValueError> {
    ieee_value::<f64>(lex, f64::INFINITY, f64::NAN)
}

/// Rounds the literal once, to the target precision (going through `f64` first can round a
/// `float` twice).
fn float_value(lex: &str) -> Result<f32, ValueError> {
    ieee_value::<f32>(lex, f32::INFINITY, f32::NAN)
}

fn ieee_value<F>(lex: &str, inf: F, nan: F) -> Result<F, ValueError>
where
    F: std::str::FromStr + std::ops::Neg<Output = F>,
{
    let s = collapse(lex);
    if !float_lexical_ok(&s) {
        return Err(ValueError::IllTyped);
    }
    Ok(match s.as_str() {
        "INF" | "+INF" => inf,
        "-INF" => -inf,
        "NaN" => nan,
        _ => s.parse::<F>().map_err(|_| ValueError::IllTyped)?,
    })
}

/// XML 1.0 (Fifth Edition) `NameStartChar`.
fn is_name_start(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// XML 1.0 (Fifth Edition) `NameChar`.
fn is_name_char(c: char) -> bool {
    is_name_start(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// XML 1.0 (Fifth Edition) `Char`.
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

fn is_language_tag(s: &str) -> bool {
    // [a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*
    let mut parts = s.split('-');
    let Some(first) = parts.next() else {
        return false;
    };
    if first.is_empty() || first.len() > 8 || !first.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    parts.all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// A language tag of a plain literal, lower-cased. A tag outside `[a-zA-Z]{1,8}(-[a-zA-Z0-9]{1,8})*`
/// is ill-typed; one inside it that is not a well-formed BCP 47 `langtag` or `privateuse` tag
/// (RFC 5646 §2.1, e.g. a grandfathered tag) is reported as unsupported, so a check that
/// depends on it answers Unknown.
fn lang_tag(tag: &str) -> Result<String, ValueError> {
    if !is_language_tag(tag) {
        return Err(ValueError::IllTyped);
    }
    if !is_bcp47(tag) {
        return Err(ValueError::Unsupported);
    }
    Ok(tag.to_ascii_lowercase())
}

/// RFC 5646 §2.1 well-formedness for `langtag` and `privateuse` (grandfathered tags excluded).
fn is_bcp47(tag: &str) -> bool {
    let alpha = |s: &str, lo: usize, hi: usize| {
        (lo..=hi).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphabetic())
    };
    let alnum = |s: &str, lo: usize, hi: usize| {
        (lo..=hi).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
    };
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let parts: Vec<&str> = tag.split('-').collect();
    let private = |rest: &[&str]| !rest.is_empty() && rest.iter().all(|p| alnum(p, 1, 8));
    if parts[0].eq_ignore_ascii_case("x") {
        return private(&parts[1..]);
    }
    let mut i = 0;
    // language = 2*3ALPHA ["-" extlang] / 4ALPHA / 5*8ALPHA
    let lang = parts[0];
    if !alpha(lang, 2, 8) {
        return false;
    }
    i += 1;
    if lang.len() <= 3 {
        // extlang = 3ALPHA *2("-" 3ALPHA)
        let mut n = 0;
        while n < 3 && i < parts.len() && alpha(parts[i], 3, 3) {
            i += 1;
            n += 1;
        }
    }
    if i < parts.len() && alpha(parts[i], 4, 4) {
        i += 1; // script
    }
    if i < parts.len() && (alpha(parts[i], 2, 2) || digits(parts[i], 3)) {
        i += 1; // region
    }
    // variant = 5*8alphanum / (DIGIT 3alphanum)
    while i < parts.len()
        && (alnum(parts[i], 5, 8)
            || (parts[i].len() == 4
                && parts[i].as_bytes()[0].is_ascii_digit()
                && alnum(parts[i], 4, 4)))
    {
        i += 1;
    }
    // extension = singleton 1*("-" (2*8alphanum)), singleton = alphanum except x
    let mut singletons: Vec<u8> = Vec::new();
    while i < parts.len() && parts[i].len() == 1 && !parts[i].eq_ignore_ascii_case("x") {
        let c = parts[i].as_bytes()[0].to_ascii_lowercase();
        if !c.is_ascii_alphanumeric() || singletons.contains(&c) {
            return false;
        }
        singletons.push(c);
        i += 1;
        let start = i;
        while i < parts.len() && alnum(parts[i], 2, 8) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if i < parts.len() && parts[i].eq_ignore_ascii_case("x") {
        return private(&parts[i + 1..]);
    }
    i == parts.len()
}

fn hex_value(lex: &str) -> Result<Vec<u8>, ValueError> {
    sparq_core::hex::decode(&collapse(lex)).ok_or(ValueError::IllTyped)
}

fn base64_value(lex: &str) -> Result<Vec<u8>, ValueError> {
    // The whiteSpace facet is `collapse`: only XML whitespace may appear.
    let s: String = lex
        .chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r'))
        .collect();
    if !s.len().is_multiple_of(4) {
        return Err(ValueError::IllTyped);
    }
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (ci, chunk) in bytes.chunks(4).enumerate() {
        let last = ci == bytes.len() / 4 - 1;
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return Err(ValueError::IllTyped);
        }
        let mut n: u32 = 0;
        let mut lastv = 0;
        for &c in &chunk[..4 - pad] {
            lastv = val(c).ok_or(ValueError::IllTyped)?;
            n = (n << 6) | lastv;
        }
        // XSD 1.1 `B64final`: the bits a padded final quad does not use must be zero
        // (`B04` after `==`, `B16` after `=`).
        if (pad == 2 && lastv & 0b1111 != 0) || (pad == 1 && lastv & 0b11 != 0) {
            return Err(ValueError::IllTyped);
        }
        n <<= 6 * pad as u32;
        let b = n.to_be_bytes();
        out.extend_from_slice(&b[1..4 - pad]);
    }
    Ok(out)
}

fn two_digits(s: &str) -> Option<i64> {
    if s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn datetime_value(lex: &str, need_tz: bool) -> Result<DateTimeValue, ValueError> {
    let s = collapse(lex);
    let (date, time) = s.split_once('T').ok_or(ValueError::IllTyped)?;
    // date: -?YYYY+-MM-DD
    let (neg, date) = match date.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, date),
    };
    let mut dp = date.splitn(3, '-');
    let (ys, ms, ds) = (dp.next(), dp.next(), dp.next());
    let (Some(ys), Some(ms), Some(ds)) = (ys, ms, ds) else {
        return Err(ValueError::IllTyped);
    };
    if ys.len() < 4
        || !ys.bytes().all(|b| b.is_ascii_digit())
        || (ys.len() > 4 && ys.starts_with('0'))
    {
        return Err(ValueError::IllTyped);
    }
    // A year beyond i64 is not represented, but the rest of the form is still validated
    // first (with a stand-in year of the same leap-ness: 10000 is a multiple of 400), so an
    // ill-typed literal is never reported as merely unsupported.
    let (mut year, too_big) = match ys.parse::<i64>() {
        Ok(y) => (y, false),
        Err(_) => (
            10_000 + ys[ys.len() - 4..].parse::<i64>().unwrap_or(0),
            true,
        ),
    };
    if neg {
        year = -year;
    }
    let month = two_digits(ms).ok_or(ValueError::IllTyped)?;
    let day = two_digits(ds).ok_or(ValueError::IllTyped)?;
    let mdays = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month) || day < 1 || day > mdays[(month - 1) as usize] {
        return Err(ValueError::IllTyped);
    }
    // time: hh:mm:ss(.s+)?(Z|(+|-)hh:mm)?
    let (clock, tz) = if let Some(c) = time.strip_suffix('Z') {
        (c, Some(0i16))
    } else if let Some(i) = time.rfind(['+', '-']).filter(|&i| i >= 8) {
        let off = &time[i + 1..];
        let (h, m) = off.split_once(':').ok_or(ValueError::IllTyped)?;
        let (h, m) = (
            two_digits(h).ok_or(ValueError::IllTyped)?,
            two_digits(m).ok_or(ValueError::IllTyped)?,
        );
        if h > 14 || m > 59 || (h == 14 && m != 0) {
            return Err(ValueError::IllTyped);
        }
        let mut minutes = (h * 60 + m) as i16;
        if &time[i..=i] == "-" {
            minutes = -minutes;
        }
        (&time[..i], Some(minutes))
    } else {
        (time, None)
    };
    if need_tz && tz.is_none() {
        return Err(ValueError::IllTyped);
    }
    let mut cp = clock.splitn(3, ':');
    let (Some(hs), Some(mis), Some(ss)) = (cp.next(), cp.next(), cp.next()) else {
        return Err(ValueError::IllTyped);
    };
    let hour = two_digits(hs).ok_or(ValueError::IllTyped)?;
    let minute = two_digits(mis).ok_or(ValueError::IllTyped)?;
    let (sec_int, sec_frac) = ss.split_once('.').unwrap_or((ss, ""));
    let sec = two_digits(sec_int).ok_or(ValueError::IllTyped)?;
    if !sec_frac.bytes().all(|b| b.is_ascii_digit()) || (ss.contains('.') && sec_frac.is_empty()) {
        return Err(ValueError::IllTyped);
    }
    let frac_zero = sec_frac.bytes().all(|b| b == b'0');
    if minute > 59
        || sec > 59
        || (hour > 23 && !(hour == 24 && minute == 0 && sec == 0 && frac_zero))
    {
        return Err(ValueError::IllTyped);
    }
    if too_big {
        return Err(ValueError::Unsupported);
    }
    // Validated above, so the civil-date precondition holds; i128 keeps every i64 year exact.
    let days = sparq_core::civil::days_from_civil(year, month as u32, day as u32);
    let whole = days * 86_400 + i128::from(hour * 3600 + minute * 60 + sec)
        - i128::from(tz.unwrap_or(0)) * 60;
    let frac = decimal_value(&format!(
        "0.{}",
        if sec_frac.is_empty() { "0" } else { sec_frac }
    ))
    .map_err(|_| ValueError::Unsupported)?;
    let seconds = Rational::new(
        whole
            .checked_mul(frac.den)
            .and_then(|w| w.checked_add(frac.num))
            .ok_or(ValueError::Unsupported)?,
        frac.den,
    )
    .ok_or(ValueError::Unsupported)?;
    Ok(DateTimeValue {
        seconds: RationalKey(seconds),
        tz,
    })
}

fn integer_bounds(b: Builtin) -> (Option<i128>, Option<i128>) {
    use Builtin::*;
    match b {
        NonNegativeInteger => (Some(0), None),
        NonPositiveInteger => (None, Some(0)),
        PositiveInteger => (Some(1), None),
        NegativeInteger => (None, Some(-1)),
        Long => (Some(i128::from(i64::MIN)), Some(i128::from(i64::MAX))),
        Int => (Some(i128::from(i32::MIN)), Some(i128::from(i32::MAX))),
        Short => (Some(i128::from(i16::MIN)), Some(i128::from(i16::MAX))),
        Byte => (Some(i128::from(i8::MIN)), Some(i128::from(i8::MAX))),
        UnsignedLong => (Some(0), Some(i128::from(u64::MAX))),
        UnsignedInt => (Some(0), Some(i128::from(u32::MAX))),
        UnsignedShort => (Some(0), Some(i128::from(u16::MAX))),
        UnsignedByte => (Some(0), Some(i128::from(u8::MAX))),
        _ => (None, None),
    }
}

fn is_integer_type(b: Builtin) -> bool {
    use Builtin::*;
    matches!(
        b,
        Integer
            | NonNegativeInteger
            | NonPositiveInteger
            | PositiveInteger
            | NegativeInteger
            | Long
            | Int
            | Short
            | Byte
            | UnsignedLong
            | UnsignedInt
            | UnsignedShort
            | UnsignedByte
    )
}

/// The integer bounds `(min, max)` of an integer datatype's value space, `None` for
/// non-integer datatypes. Unbounded sides are `None` inside the pair.
#[must_use]
pub fn integer_range(b: Builtin) -> Option<(Option<i128>, Option<i128>)> {
    is_integer_type(b).then(|| integer_bounds(b))
}

/// The data value of a literal with lexical form `lex`, datatype IRI `datatype` and language
/// tag `lang`.
///
/// # Errors
/// [`ValueError::IllTyped`] for a lexical form outside the datatype's lexical space,
/// [`ValueError::UnknownDatatype`] for a datatype outside the OWL 2 datatype map, and
/// [`ValueError::Unsupported`] for a value this implementation cannot represent exactly.
pub fn parse_value(lex: &str, datatype: &str, lang: Option<&str>) -> Result<Value, ValueError> {
    use Builtin::*;
    let b = builtin(datatype).ok_or(ValueError::UnknownDatatype)?;
    if lang.is_some() && b != LangString {
        return Err(ValueError::IllTyped);
    }
    // Every lexical space of the datatype map is a set of XML 1.0 `Char` strings.
    if !lex.chars().all(is_xml_char) {
        return Err(ValueError::IllTyped);
    }
    Ok(match b {
        // rdfs:Literal and owl:real have no lexical space of their own (OWL 2 §4.1).
        RdfsLiteral | OwlReal => return Err(ValueError::IllTyped),
        OwlRational => {
            // OWL 2 §4.1: `numerator '/' denominator`, the denominator an unsigned positive
            // integer, with no whitespace inside the collapsed form.
            let s = collapse(lex);
            let (n, d) = s.split_once('/').ok_or(ValueError::IllTyped)?;
            if s.contains(' ') || d.is_empty() || !d.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ValueError::IllTyped);
            }
            let n = integer_value(n)?;
            let d = integer_value(d)?;
            if d <= 0 {
                return Err(ValueError::IllTyped);
            }
            Value::Real(RationalKey(
                Rational::new(n, d).ok_or(ValueError::Unsupported)?,
            ))
        }
        Decimal => Value::Real(RationalKey(decimal_value(lex)?)),
        _ if is_integer_type(b) => {
            let n = integer_value(lex)?;
            let (lo, hi) = integer_bounds(b);
            if lo.is_some_and(|lo| n < lo) || hi.is_some_and(|hi| n > hi) {
                return Err(ValueError::IllTyped);
            }
            Value::Real(RationalKey(
                Rational::int(n).ok_or(ValueError::Unsupported)?,
            ))
        }
        Double => {
            let d = double_value(lex)?;
            Value::Double(if d.is_nan() {
                f64::NAN.to_bits()
            } else {
                d.to_bits()
            })
        }
        Float => {
            let d = float_value(lex)?;
            Value::Float(if d.is_nan() {
                f32::NAN.to_bits()
            } else {
                d.to_bits()
            })
        }
        PlainLiteral => {
            let (text, tag) = lex.rsplit_once('@').ok_or(ValueError::IllTyped)?;
            if tag.is_empty() {
                Value::String(text.to_string())
            } else {
                Value::LangString(text.to_string(), lang_tag(tag)?)
            }
        }
        LangString => match lang {
            Some(l) => Value::LangString(lex.to_string(), lang_tag(l)?),
            None => return Err(ValueError::IllTyped),
        },
        String => Value::String(lex.to_string()),
        NormalizedString => Value::String(replace_ws(lex)),
        Token => Value::String(collapse(lex)),
        Language => {
            let s = collapse(lex);
            if !is_language_tag(&s) {
                return Err(ValueError::IllTyped);
            }
            Value::String(s)
        }
        Name | NcName | NmToken => {
            let s = collapse(lex);
            let ok = match b {
                NmToken => !s.is_empty() && s.chars().all(is_name_char),
                _ => {
                    let mut cs = s.chars();
                    cs.next().is_some_and(is_name_start)
                        && cs.all(is_name_char)
                        && (b != NcName || !s.contains(':'))
                }
            };
            if !ok {
                return Err(ValueError::IllTyped);
            }
            Value::String(s)
        }
        Boolean => match collapse(lex).as_str() {
            "true" | "1" => Value::Boolean(true),
            "false" | "0" => Value::Boolean(false),
            _ => return Err(ValueError::IllTyped),
        },
        HexBinary => Value::HexBinary(hex_value(lex)?),
        Base64Binary => Value::Base64Binary(base64_value(lex)?),
        AnyUri => Value::AnyUri(collapse(lex)),
        DateTime => Value::DateTime(datetime_value(lex, false)?),
        DateTimeStamp => Value::DateTime(datetime_value(lex, true)?),
        XmlLiteral => Value::XmlLiteral(lex.to_string()),
        _ => unreachable!("every builtin is handled above"),
    })
}

/// `true` when `v` is in the value space of `b`.
#[must_use]
pub fn in_value_space(v: &Value, b: Builtin) -> bool {
    use Builtin::*;
    if !b.families().contains(&v.family()) {
        return false;
    }
    match (b, v) {
        (OwlReal | OwlRational, Value::Real(_)) => true,
        (Decimal, Value::Real(r)) => r.0.is_decimal(),
        (_, Value::Real(r)) if is_integer_type(b) => {
            let (lo, hi) = integer_bounds(b);
            r.0.is_integer()
                && lo.is_none_or(|lo| r.0.num >= lo)
                && hi.is_none_or(|hi| r.0.num <= hi)
        }
        (NormalizedString, Value::String(s)) => !s.contains(['\t', '\n', '\r']),
        (Token, Value::String(s)) => collapse(s) == *s,
        (Language, Value::String(s)) => is_language_tag(s),
        (NmToken, Value::String(s)) => !s.is_empty() && s.chars().all(is_name_char),
        (Name | NcName, Value::String(s)) => {
            let mut cs = s.chars();
            cs.next().is_some_and(is_name_start)
                && cs.all(is_name_char)
                && (b != NcName || !s.contains(':'))
        }
        (DateTimeStamp, Value::DateTime(d)) => d.tz.is_some(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(lex: &str, local: &str) -> Result<Value, ValueError> {
        parse_value(lex, &format!("{XSD}{local}"), None)
    }

    #[test]
    fn numbers_compare_by_value_across_types() {
        assert_eq!(val("1.0", "decimal"), val("1", "integer"));
        assert_eq!(val("01", "integer"), val("1", "nonNegativeInteger"));
        assert_ne!(val("1", "integer"), val("1", "double"));
        assert_eq!(
            val("sixteen", "nonNegativeInteger"),
            Err(ValueError::IllTyped)
        );
        assert_eq!(val("-1", "nonNegativeInteger"), Err(ValueError::IllTyped));
        assert_eq!(val("300", "byte"), Err(ValueError::IllTyped));
        assert!(val(" 42 ", "int").is_ok());
        assert_eq!(val("1/3", "decimal"), Err(ValueError::IllTyped));
        assert!(parse_value("1/3", &format!("{OWL}rational"), None).is_ok());
        assert_eq!(
            parse_value("1", &format!("{OWL}real"), None),
            Err(ValueError::IllTyped)
        );
    }

    #[test]
    fn rationals_order_and_classify() {
        let third = Rational::new(1, 3).unwrap();
        let half = Rational::new(2, 4).unwrap();
        assert!(third < half);
        assert!(half.is_decimal());
        assert!(!third.is_decimal());
        assert_eq!(Rational::new(-7, 2).unwrap().floor(), -4);
        assert_eq!(Rational::new(-7, 2).unwrap().ceil(), -3);
        assert_eq!(Rational::new(6, -3), Rational::int(-2));
        // i128::MIN is outside the represented range; everything near it stays exact.
        assert_eq!(Rational::new(i128::MIN, 1), None);
        assert_eq!(Rational::new(1, i128::MIN), None);
        assert_eq!(Rational::int(i128::MIN), None);
        let lo = Rational::new(i128::MIN + 1, 2).unwrap();
        assert_eq!(lo.ceil(), (i128::MIN + 2) / 2);
        assert_eq!(lo.floor(), i128::MIN / 2);
        let dec = |s: &str| val(s, "decimal").unwrap();
        assert!(
            dec("-17014118346046923173168730371588410572.7")
                < dec("-17014118346046923173168730371588410572.5")
        );
    }

    #[test]
    fn strings_booleans_binaries() {
        assert_eq!(val("  a  b ", "token"), Ok(Value::String("a b".into())));
        assert_eq!(val("1", "boolean"), val("true", "boolean"));
        assert_eq!(val("yes", "boolean"), Err(ValueError::IllTyped));
        assert_eq!(
            val("0FB7", "hexBinary"),
            Ok(Value::HexBinary(vec![0x0f, 0xb7]))
        );
        assert_eq!(val("0FB", "hexBinary"), Err(ValueError::IllTyped));
        assert_eq!(
            val("aGk=", "base64Binary"),
            Ok(Value::Base64Binary(b"hi".to_vec()))
        );
        assert_eq!(val("en-GB", "language"), Ok(Value::String("en-GB".into())));
        assert_eq!(val("1abc", "NCName"), Err(ValueError::IllTyped));
        assert_eq!(
            parse_value("hi", &format!("{RDF}langString"), Some("EN")),
            Ok(Value::LangString("hi".into(), "en".into()))
        );
        assert_eq!(
            parse_value("hi", &format!("{XSD}string"), Some("en")),
            Err(ValueError::IllTyped)
        );
        assert_eq!(
            parse_value("x", "http://www.w3.org/2001/XMLSchema#date", None),
            Err(ValueError::UnknownDatatype)
        );
    }

    #[test]
    fn datetimes_normalise_timezones() {
        let a = val("2024-01-01T12:00:00Z", "dateTime").unwrap();
        let b = val("2024-01-01T13:00:00+01:00", "dateTime").unwrap();
        let (Value::DateTime(a), Value::DateTime(b)) = (a, b) else {
            panic!()
        };
        assert_eq!(a.seconds, b.seconds);
        assert!(val("2024-02-30T00:00:00", "dateTime").is_err());
        assert!(val("2024-02-29T00:00:00", "dateTime").is_ok());
        assert!(val("2024-01-01T00:00:00", "dateTimeStamp").is_err());
        assert!(val("2024-01-01T24:00:00", "dateTime").is_ok());
    }

    #[test]
    fn value_space_membership() {
        let one = val("1", "integer").unwrap();
        let half = val("0.5", "decimal").unwrap();
        assert!(in_value_space(&one, Builtin::PositiveInteger));
        assert!(!in_value_space(&half, Builtin::Integer));
        assert!(in_value_space(&half, Builtin::OwlReal));
        assert!(!in_value_space(&one, Builtin::Double));
        assert!(!in_value_space(
            &Value::String("a".into()),
            Builtin::LangString
        ));
        assert!(in_value_space(
            &Value::String("a".into()),
            Builtin::PlainLiteral
        ));
        assert!(in_value_space(&Value::String("a b".into()), Builtin::Token));
        assert!(!in_value_space(&Value::String(" a".into()), Builtin::Token));
        assert!(in_value_space(&one, Builtin::RdfsLiteral));
        assert_eq!(
            builtin(&builtin_iri(Builtin::NmToken)),
            Some(Builtin::NmToken)
        );
    }
}
