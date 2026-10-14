//! The OWL 2 datatype map (OWL 2 Structural Specification §4) with exact value spaces.
//!
//! Answers, for literals and data ranges over the datatypes of the map:
//! - whether a literal's lexical form denotes a value of its datatype ([`in_value_space`]);
//! - whether a facet–value pair is in a datatype's facet space ([`facet_applies`]);
//! - whether a [`DataRange`] is empty, is a subset of another, or contains a literal.
//!
//! Every answer is a [`Tri`]. `Unknown` is returned for whatever is not implemented:
//! datatypes outside the map (including `rdf:XMLLiteral` lexical forms), pattern block
//! escapes, years beyond `i64`, and string checks whose automata outgrow a fixed budget.
//! A `True` or `False` answer is exact: numbers are compared as rationals with no
//! floating-point fallback, floats and doubles are discrete with `-0` and `+0` distinct
//! values, and string ranges are decided by automata.
//!
//! The data domain is the union of the map's value spaces, so `DataComplementOf` is taken
//! relative to it; a caller with custom datatypes expands their definitions first.

mod cuts;
mod rat;
mod region;
mod strings;
mod time;
mod xsdre;

use cuts::Cuts;
use num_bigint::BigInt;
use rat::Rat;
use region::{f32_key, f64_key, int_range, Region, Sp, Val};
use strings::{Atom, StrKind, F};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDFS_LITERAL: &str = "http://www.w3.org/2000/01/rdf-schema#Literal";

/// A three-valued answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    True,
    False,
    Unknown,
}

impl From<bool> for Tri {
    fn from(b: bool) -> Tri {
        if b {
            Tri::True
        } else {
            Tri::False
        }
    }
}

/// An RDF literal: a lexical form with a datatype IRI, or a language-tagged string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Literal {
    pub lexical: String,
    pub datatype: String,
    /// A language tag. When present the literal is the pair `<lexical, tag>` of
    /// `rdf:PlainLiteral`, whatever `datatype` says (normally `rdf:langString`).
    pub lang: Option<String>,
}

impl Literal {
    pub fn typed(lexical: impl Into<String>, datatype: impl Into<String>) -> Literal {
        Literal {
            lexical: lexical.into(),
            datatype: datatype.into(),
            lang: None,
        }
    }

    pub fn lang(lexical: impl Into<String>, tag: impl Into<String>) -> Literal {
        Literal {
            lexical: lexical.into(),
            datatype: format!("{RDF}langString"),
            lang: Some(tag.into()),
        }
    }
}

/// An OWL 2 data range (Structural Specification §7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataRange {
    /// A datatype, by IRI.
    Datatype(String),
    Intersection(Vec<DataRange>),
    Union(Vec<DataRange>),
    /// Complement relative to the data domain.
    Complement(Box<DataRange>),
    OneOf(Vec<Literal>),
    /// A datatype restricted by `(facet IRI, value)` pairs.
    Restriction(String, Vec<(String, Literal)>),
}

/// Whether `lit` denotes a value of its datatype.
pub fn in_value_space(lit: &Literal) -> Tri {
    match value(lit) {
        Ok(v) => Tri::from(v.is_some()),
        Err(Unknown) => Tri::Unknown,
    }
}

/// Whether `(facet, value)` is in the facet space of `datatype`.
pub fn facet_applies(datatype: &str, facet: &str, value: &Literal) -> Tri {
    let Some(dt) = Dt::from_iri(datatype) else {
        return Tri::Unknown;
    };
    match facet_region(dt, facet, value) {
        Ok(r) => Tri::from(r.is_some()),
        Err(Unknown) => Tri::Unknown,
    }
}

impl DataRange {
    /// Whether no data value is in the range.
    pub fn is_empty(&self) -> Tri {
        self.region().map_or(Tri::Unknown, |r| r.is_empty())
    }

    /// Whether every value of `self` is in `other`.
    pub fn is_subset_of(&self, other: &DataRange) -> Tri {
        match (self.region(), other.region()) {
            (Ok(a), Ok(b)) => a.and(&b.not()).is_empty(),
            _ => Tri::Unknown,
        }
    }

    /// Whether the value `lit` denotes is in the range. Unknown for a literal with no
    /// value (see [`in_value_space`]).
    pub fn contains(&self, lit: &Literal) -> Tri {
        match (self.region(), value(lit)) {
            (Ok(r), Ok(Some(v))) => r.contains(&v),
            _ => Tri::Unknown,
        }
    }

    fn region(&self) -> Result<Region, Unknown> {
        Ok(match self {
            DataRange::Datatype(iri) => Dt::from_iri(iri).ok_or(Unknown)?.universe(),
            DataRange::Intersection(rs) => rs.iter().try_fold(Region::full(), |acc, r| {
                Ok::<_, Unknown>(acc.and(&r.region()?))
            })?,
            DataRange::Union(rs) => rs.iter().try_fold(Region::empty(), |acc, r| {
                Ok::<_, Unknown>(acc.or(&r.region()?))
            })?,
            DataRange::Complement(r) => r.region()?.not(),
            DataRange::OneOf(lits) => lits.iter().try_fold(Region::empty(), |acc, l| {
                let v = value(l)?.ok_or(Unknown)?;
                Ok::<_, Unknown>(acc.or(&Region::value(v)))
            })?,
            DataRange::Restriction(iri, facets) => {
                let dt = Dt::from_iri(iri).ok_or(Unknown)?;
                facets.iter().try_fold(dt.universe(), |acc, (f, v)| {
                    Ok::<_, Unknown>(acc.and(&facet_region(dt, f, v)?.ok_or(Unknown)?))
                })?
            }
        })
    }
}

/// The marker for an answer this module does not implement.
#[derive(Debug)]
struct Unknown;

/// The datatypes of the OWL 2 datatype map, plus `rdfs:Literal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dt {
    Literal,
    Real,
    Rational,
    Decimal,
    /// `xsd:integer` and its subtypes, by value bounds.
    Integer(Option<i128>, Option<i128>),
    Float,
    Double,
    PlainLiteral,
    Str(StrKind),
    Boolean,
    HexBinary,
    Base64Binary,
    DateTime,
    DateTimeStamp,
    XmlLiteral,
}

impl Dt {
    fn from_iri(iri: &str) -> Option<Dt> {
        if iri == RDFS_LITERAL {
            return Some(Dt::Literal);
        }
        if let Some(l) = iri.strip_prefix(OWL) {
            return match l {
                "real" => Some(Dt::Real),
                "rational" => Some(Dt::Rational),
                _ => None,
            };
        }
        if let Some(l) = iri.strip_prefix(RDF) {
            return match l {
                "PlainLiteral" => Some(Dt::PlainLiteral),
                "XMLLiteral" => Some(Dt::XmlLiteral),
                _ => None,
            };
        }
        let int = |lo: Option<i128>, hi: Option<i128>| Some(Dt::Integer(lo, hi));
        match iri.strip_prefix(XSD)? {
            "decimal" => Some(Dt::Decimal),
            "integer" => int(None, None),
            "nonNegativeInteger" => int(Some(0), None),
            "positiveInteger" => int(Some(1), None),
            "nonPositiveInteger" => int(None, Some(0)),
            "negativeInteger" => int(None, Some(-1)),
            "long" => int(Some(i64::MIN.into()), Some(i64::MAX.into())),
            "int" => int(Some(i32::MIN.into()), Some(i32::MAX.into())),
            "short" => int(Some(i16::MIN.into()), Some(i16::MAX.into())),
            "byte" => int(Some(i8::MIN.into()), Some(i8::MAX.into())),
            "unsignedLong" => int(Some(0), Some(u64::MAX.into())),
            "unsignedInt" => int(Some(0), Some(u32::MAX.into())),
            "unsignedShort" => int(Some(0), Some(u16::MAX.into())),
            "unsignedByte" => int(Some(0), Some(u8::MAX.into())),
            "float" => Some(Dt::Float),
            "double" => Some(Dt::Double),
            "string" => Some(Dt::Str(StrKind::String)),
            "normalizedString" => Some(Dt::Str(StrKind::NormalizedString)),
            "token" => Some(Dt::Str(StrKind::Token)),
            "language" => Some(Dt::Str(StrKind::Language)),
            "Name" => Some(Dt::Str(StrKind::Name)),
            "NCName" => Some(Dt::Str(StrKind::NCName)),
            "NMTOKEN" => Some(Dt::Str(StrKind::NmToken)),
            "anyURI" => Some(Dt::Str(StrKind::AnyUri)),
            "boolean" => Some(Dt::Boolean),
            "hexBinary" => Some(Dt::HexBinary),
            "base64Binary" => Some(Dt::Base64Binary),
            "dateTime" => Some(Dt::DateTime),
            "dateTimeStamp" => Some(Dt::DateTimeStamp),
            _ => None,
        }
    }

    fn universe(self) -> Region {
        let reals = |layers: usize, ints: Cuts| {
            let sorts = (0..4).map(|s| {
                if s == 0 {
                    ints.clone()
                } else if s < layers {
                    Cuts::full()
                } else {
                    Cuts::empty()
                }
            });
            Region::ordered(Sp::Real, sorts.collect())
        };
        match self {
            Dt::Literal => Region::full(),
            Dt::Real => reals(4, Cuts::full()),
            Dt::Rational => reals(3, Cuts::full()),
            Dt::Decimal => reals(2, Cuts::full()),
            Dt::Integer(lo, hi) => reals(1, int_range(lo.map(BigInt::from), hi.map(BigInt::from))),
            Dt::Float => Region::ordered(Sp::Float, vec![Cuts::full(), Cuts::full()]),
            Dt::Double => Region::ordered(Sp::Double, vec![Cuts::full(), Cuts::full()]),
            Dt::PlainLiteral => Region::text(F::Const(true)),
            Dt::Str(StrKind::AnyUri) => Region::uri(F::Const(true)),
            Dt::Str(k) => Region::text(F::atom(Atom::Plain(k))),
            Dt::Boolean => Region::ordered(Sp::Bool, vec![Cuts::full()]),
            Dt::HexBinary => Region::ordered(Sp::Hex, vec![Cuts::full()]),
            Dt::Base64Binary => Region::ordered(Sp::B64, vec![Cuts::full()]),
            Dt::DateTime => Region::ordered(Sp::Time, vec![Cuts::full(), Cuts::full()]),
            Dt::DateTimeStamp => Region::ordered(Sp::Time, vec![Cuts::full(), Cuts::empty()]),
            Dt::XmlLiteral => Region::xml(),
        }
    }
}

/// The value `lit` denotes: `Ok(None)` when its lexical form is not in the lexical
/// space of its datatype.
fn value(lit: &Literal) -> Result<Option<Val>, Unknown> {
    if let Some(tag) = &lit.lang {
        return Ok(lang_pair(&lit.lexical, tag));
    }
    let s = lit.lexical.as_str();
    let dt = Dt::from_iri(&lit.datatype).ok_or(Unknown)?;
    let real = |r: Option<Rat>| r.map(Val::Real);
    Ok(match dt {
        Dt::Literal | Dt::Real => None,
        Dt::Rational => real(Rat::parse_rational(s)),
        Dt::Decimal => real(Rat::parse_decimal(s)),
        Dt::Integer(..) => Rat::parse_integer(s)
            .filter(|r| dt.universe().contains(&Val::Real(r.clone())) == Tri::True)
            .map(Val::Real),
        Dt::Float => sparq_core::parse_xsd_f64(s).map(|d| {
            let f = if d.is_nan() || d.is_infinite() {
                d as f32
            } else {
                s.parse::<f32>().unwrap_or(d as f32)
            };
            Val::Float(if f.is_nan() {
                f32::NAN.to_bits()
            } else {
                f.to_bits()
            })
        }),
        Dt::Double => sparq_core::parse_xsd_f64(s).map(|d| {
            Val::Double(if d.is_nan() {
                f64::NAN.to_bits()
            } else {
                d.to_bits()
            })
        }),
        Dt::PlainLiteral => match s.rsplit_once('@') {
            Some((text, "")) => text_value(text, StrKind::String)?.map(Val::Str),
            Some((text, tag)) => lang_pair(text, tag),
            None => None,
        },
        Dt::Str(StrKind::AnyUri) => text_value(s, StrKind::AnyUri)?.map(Val::Uri),
        Dt::Str(k) => text_value(s, k)?.map(Val::Str),
        Dt::Boolean => match s {
            "true" | "1" => Some(Val::Bool(true)),
            "false" | "0" => Some(Val::Bool(false)),
            _ => None,
        },
        Dt::HexBinary => sparq_core::hex::decode(s).map(Val::Hex),
        Dt::Base64Binary => base64(s).map(Val::B64),
        Dt::DateTime | Dt::DateTimeStamp => match time::parse(s).map_err(|()| Unknown)? {
            Some(t) if t.offset.is_some() || dt == Dt::DateTime => Some(Val::Time {
                at: t.instant(),
                offset: t.offset,
            }),
            _ => None,
        },
        Dt::XmlLiteral => return Err(Unknown),
    })
}

fn text_value(s: &str, kind: StrKind) -> Result<Option<String>, Unknown> {
    match strings::is_kind(kind, s) {
        Tri::True => Ok(Some(s.to_string())),
        Tri::False => Ok(None),
        Tri::Unknown => Err(Unknown),
    }
}

/// `<text, lowercase tag>`, when `tag` is a language tag and `text` a string.
fn lang_pair(text: &str, tag: &str) -> Option<Val> {
    let ok = strings::is_kind(StrKind::Language, tag) == Tri::True
        && strings::is_kind(StrKind::String, text) == Tri::True;
    ok.then(|| Val::Lang(text.to_string(), tag.to_ascii_lowercase()))
}

/// Decodes an `xsd:base64Binary` lexical (XSD 1.1 Part 2 §3.3.16): single spaces may
/// separate characters, and the bits a final quantum leaves unused must be zero (so
/// each value has exactly one lexical form up to spacing).
fn base64(s: &str) -> Option<Vec<u8>> {
    if s.starts_with(' ') || s.ends_with(' ') || s.contains("  ") {
        return None;
    }
    let chars: Vec<u8> = s.bytes().filter(|&b| b != b' ').collect();
    if !chars.len().is_multiple_of(4) {
        return None;
    }
    let pad = chars.iter().rev().take_while(|&&b| b == b'=').count();
    if pad > 2 || chars[..chars.len() - pad].contains(&b'=') {
        return None;
    }
    let digit = |b: u8| -> Option<u32> {
        Some(u32::from(match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    };
    let mut out = Vec::with_capacity(chars.len() / 4 * 3);
    for quad in chars.chunks(4) {
        let n = 4 - quad.iter().filter(|&&b| b == b'=').count();
        let mut acc = 0u32;
        for &b in &quad[..n] {
            acc = acc << 6 | digit(b)?;
        }
        let bits = n as u32 * 6;
        let bytes = bits / 8;
        // The leftover low bits of a padded quantum must be zero.
        if acc & ((1 << (bits % 8)) - 1) != 0 {
            return None;
        }
        let acc = acc >> (bits % 8);
        out.extend((0..bytes).rev().map(|i| (acc >> (8 * i)) as u8));
    }
    Some(out)
}

/// The constraining facets the datatype map uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Facet {
    Min(bool),
    Max(bool),
    Length,
    MinLength,
    MaxLength,
    Pattern,
    LangRange,
}

impl Facet {
    fn from_iri(iri: &str) -> Option<Facet> {
        if iri == format!("{RDF}langRange") {
            return Some(Facet::LangRange);
        }
        Some(match iri.strip_prefix(XSD)? {
            "minInclusive" => Facet::Min(true),
            "minExclusive" => Facet::Min(false),
            "maxInclusive" => Facet::Max(true),
            "maxExclusive" => Facet::Max(false),
            "length" => Facet::Length,
            "minLength" => Facet::MinLength,
            "maxLength" => Facet::MaxLength,
            "pattern" => Facet::Pattern,
            _ => return None,
        })
    }
}

/// The values a facet–value pair admits, in every space (the caller intersects with the
/// datatype); `Ok(None)` when the pair is not in the datatype's facet space.
fn facet_region(dt: Dt, facet_iri: &str, v: &Literal) -> Result<Option<Region>, Unknown> {
    let Some(facet) = Facet::from_iri(facet_iri) else {
        return Ok(None);
    };
    let Some(val) = value(v)? else {
        return Ok(None);
    };
    let bound = |inclusive: bool, upper: bool, k: Rat| {
        if upper {
            Cuts::at_most(k, inclusive)
        } else {
            Cuts::at_least(k, inclusive)
        }
    };
    // Only the datatype's own space matters: the caller intersects with its universe.
    let ordered = |sp: Sp, sorts: Vec<Cuts>| Some(Region::ordered(sp, sorts));
    let (inclusive, upper) = match facet {
        Facet::Min(i) => (i, false),
        Facet::Max(i) => (i, true),
        _ => (false, false),
    };
    let order = matches!(facet, Facet::Min(_) | Facet::Max(_));
    Ok(match (dt, facet, val) {
        (Dt::Real | Dt::Rational | Dt::Decimal | Dt::Integer(..), _, Val::Real(k)) if order => {
            ordered(Sp::Real, vec![bound(inclusive, upper, k); 4])
        }
        (Dt::Float, _, Val::Float(b)) if order => ordered(
            Sp::Float,
            vec![
                float_bound(f64::from(f32::from_bits(b)), inclusive, upper, |f| {
                    i128::from(f32_key(f as f32))
                }),
                Cuts::empty(),
            ],
        ),
        (Dt::Double, _, Val::Double(b)) if order => ordered(
            Sp::Double,
            vec![
                float_bound(f64::from_bits(b), inclusive, upper, f64_key),
                Cuts::empty(),
            ],
        ),
        (Dt::DateTime | Dt::DateTimeStamp, _, Val::Time { at, offset }) if order => {
            // A value with an offset and one without compare only across a ±14:00 gap,
            // and are never equal (XSD 1.1 Part 2 §3.3.7.3).
            let gap = |d: i64| at.add(&Rat::int(if upper { -d } else { d }));
            let shift = time::MAX_OFFSET_SECS;
            let sorts = match offset {
                Some(_) => vec![
                    bound(inclusive, upper, at.clone()),
                    bound(false, upper, gap(shift)),
                ],
                None => vec![
                    bound(false, upper, gap(shift)),
                    bound(inclusive, upper, at.clone()),
                ],
            };
            ordered(Sp::Time, sorts)
        }
        (
            Dt::PlainLiteral | Dt::Str(_) | Dt::HexBinary | Dt::Base64Binary,
            Facet::Length | Facet::MinLength | Facet::MaxLength,
            Val::Real(n),
        ) if n.is_integer() && !n.is_negative() => {
            let n = n.floor();
            let (lo, hi) = match facet {
                Facet::Length => (Some(n.clone()), Some(n)),
                Facet::MinLength => (Some(n), None),
                _ => (None, Some(n)),
            };
            match dt {
                Dt::HexBinary => ordered(Sp::Hex, vec![int_range(lo, hi)]),
                Dt::Base64Binary => ordered(Sp::B64, vec![int_range(lo, hi)]),
                _ => {
                    let lo = lo
                        .map_or(Some(0), |l| u64::try_from(l).ok())
                        .ok_or(Unknown)?;
                    let hi = hi.map(u64::try_from).transpose().map_err(|_| Unknown)?;
                    Some(strings_facet(dt, Atom::Len(lo, hi)))
                }
            }
        }
        (Dt::PlainLiteral | Dt::Str(_), Facet::Pattern, Val::Str(p)) => {
            match strings::pattern_status(&p) {
                Tri::True => Some(strings_facet(dt, Atom::Pattern(p))),
                Tri::False => None,
                Tri::Unknown => return Err(Unknown),
            }
        }
        (Dt::PlainLiteral, Facet::LangRange, Val::Str(r)) => {
            let basic = r == "*" || strings::is_kind(StrKind::Language, &r) == Tri::True;
            basic.then(|| strings_facet(dt, Atom::Lang(r.to_ascii_lowercase())))
        }
        _ => None,
    })
}

/// A string-space facet in the datatype's own space.
fn strings_facet(dt: Dt, a: Atom) -> Region {
    if dt == Dt::Str(StrKind::AnyUri) {
        Region::uri(F::atom(a))
    } else {
        Region::text(F::atom(a))
    }
}

/// The ordered floating-point values a min/max facet admits. Facets compare by
/// numeric equality, so a bound of either zero admits or excludes both zeros, and a NaN
/// bound admits nothing.
fn float_bound(f: f64, inclusive: bool, upper: bool, key: impl Fn(f64) -> i128) -> Cuts {
    let k = |x: f64| Rat::int(key(x));
    if f.is_nan() {
        return Cuts::empty();
    }
    match (f == 0.0, upper) {
        (true, false) => Cuts::at_least(k(if inclusive { -0.0 } else { 0.0 }), inclusive),
        (true, true) => Cuts::at_most(k(if inclusive { 0.0 } else { -0.0 }), inclusive),
        (false, false) => Cuts::at_least(k(f), inclusive),
        (false, true) => Cuts::at_most(k(f), inclusive),
    }
}

#[cfg(test)]
mod tests;
