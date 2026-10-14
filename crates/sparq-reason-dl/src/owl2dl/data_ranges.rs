//! Data-range reasoning over the OWL 2 datatype map: emptiness, membership, counting for
//! data cardinalities, and the tableau's per-node datatype check.
//!
//! A [`DataRangeExpr`] denotes a set of [`Value`]s. Every answer is a [`Tri`]: `Yes` and
//! `No` are exact, `Unknown` means the question is outside what is implemented (a facet
//! that is not in the datatype's facet space, an XSD pattern block escape, or a string
//! search whose automata outgrow a fixed budget). The pieces:
//!
//! - **Ordered families** (the reals, `xsd:float`, `xsd:double`, `xsd:boolean`, the two
//!   binary types by length, `xsd:dateTime`) are finite unions of intervals with exact
//!   rational ends, plus finitely many values added or removed. The reals keep integers,
//!   other decimals, other rationals and irrationals apart, so `owl:real ⊋ owl:rational ⊋
//!   xsd:decimal ⊋ xsd:integer` hold exactly. Floats are discrete, with `-0` and `+0`
//!   distinct values that facets treat as equal, and NaN admitted by no facet. A value
//!   with a time zone and one without compare only across the ±14:00 window, and the same
//!   instant under two offsets is equal (for facets) but not identical (for counting).
//! - **String-like families** (strings with and without a language tag, `xsd:anyURI`,
//!   `rdf:XMLLiteral`) are boolean formulas over automata for XSD patterns, lengths,
//!   basic language ranges, well-formed BCP 47 tags and enumerated values; emptiness and
//!   counting walk the product automaton.
//!
//! The data domain is the union of the datatype map's value spaces, so `Not` is taken
//! relative to it.

mod cuts;
mod rat;
mod region;
mod strings;
mod xsdre;

use super::datatypes::{
    in_value_space, integer_range, Builtin, Facet, Rational, RationalKey, Value,
};
use cuts::Cuts;
use rat::Rat;
use region::{f32_key, f64_key, int_range, Region, Sp};
use strings::{Atom, Space, StrKind, F};

/// A three-valued answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tri {
    Yes,
    No,
    Unknown,
}

impl From<bool> for Tri {
    fn from(b: bool) -> Tri {
        if b {
            Tri::Yes
        } else {
            Tri::No
        }
    }
}

impl From<Option<bool>> for Tri {
    fn from(b: Option<bool>) -> Tri {
        b.map_or(Tri::Unknown, Tri::from)
    }
}

/// A data range over the datatype map (OWL 2 Structural Specification §7).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DataRangeExpr {
    Datatype(Builtin),
    /// `DatatypeRestriction`: a datatype and its `(facet, value)` pairs.
    Restriction(Builtin, Vec<(Facet, Value)>),
    OneOf(Vec<Value>),
    And(Vec<DataRangeExpr>),
    Or(Vec<DataRangeExpr>),
    /// `DataComplementOf`, relative to the data domain.
    Not(Box<DataRangeExpr>),
}

/// Whether no value is in `r`.
#[must_use]
pub fn is_empty(r: &DataRangeExpr) -> Tri {
    match has_at_least(r, 1) {
        Tri::Yes => Tri::No,
        Tri::No => Tri::Yes,
        Tri::Unknown => Tri::Unknown,
    }
}

/// Whether `v` is in `r`.
#[must_use]
pub fn contains(r: &DataRangeExpr, v: &Value) -> Tri {
    match (region(r, 0), canonical(v)) {
        (Ok(r), Ok(v)) => Tri::from(r.contains(&v)),
        _ => Tri::Unknown,
    }
}

/// Whether `r` has at least `n` values.
#[must_use]
pub fn has_at_least(r: &DataRangeExpr, n: u64) -> Tri {
    region(r, 0).map_or(Tri::Unknown, |r| at_least(&r, n))
}

/// The tableau's datatype check for one node: whether there are `distinct` pairwise
/// different values, none of them in `values`, each in every range of `pos` and in no
/// range of `neg`.
#[must_use]
pub fn conjunction_satisfiable(
    pos: &[DataRangeExpr],
    neg: &[DataRangeExpr],
    distinct: usize,
    values: &[Value],
) -> Tri {
    if distinct == 0 {
        return Tri::Yes;
    }
    let build = || -> Result<Region, Unknown> {
        let mut acc = Region::full();
        for r in pos {
            acc = acc.and(&region(r, 0)?);
        }
        for r in neg {
            acc = acc.and(&region(r, 0)?.not());
        }
        Ok(acc.and(&one_of(values)?.not()))
    };
    build().map_or(Tri::Unknown, |r| at_least(&r, distinct as u64))
}

fn at_least(r: &Region, n: u64) -> Tri {
    if n == 0 {
        return Tri::Yes;
    }
    r.count(u128::from(n))
        .map_or(Tri::Unknown, |c| Tri::from(c >= u128::from(n)))
}

/// The marker for a question this module does not answer.
struct Unknown;

/// Data ranges nested deeper than this answer Unknown, bounding the recursion.
const MAX_DEPTH: usize = 64;

/// `v` in the canonical form the parser produces (one NaN per type, lowercase language
/// tags), or Unknown for a value no literal has (a zero or non-reduced denominator, an
/// offset beyond ±14:00).
fn canonical(v: &Value) -> Result<Value, Unknown> {
    Ok(match v {
        Value::Real(r) if !reduced(r) => return Err(Unknown),
        // Seconds are decimal (XML Schema 1.1 Part 2 §D.2.1).
        Value::DateTime(d)
            if !reduced(&d.seconds)
                || !d.seconds.0.is_decimal()
                || d.tz.is_some_and(|t| !(-840..=840).contains(&t)) =>
        {
            return Err(Unknown);
        }
        Value::Float(b) if f32::from_bits(*b).is_nan() => Value::Float(f32::NAN.to_bits()),
        Value::Double(b) if f64::from_bits(*b).is_nan() => Value::Double(f64::NAN.to_bits()),
        Value::LangString(s, t) => Value::LangString(s.clone(), t.to_ascii_lowercase()),
        v => v.clone(),
    })
}

/// Whether `r` is in the normal form `Rational::new` gives, as parsed values are.
fn reduced(r: &RationalKey) -> bool {
    Rational::new(r.0.num, r.0.den) == Some(r.0)
}

fn one_of(vs: &[Value]) -> Result<Region, Unknown> {
    vs.iter().try_fold(Region::empty(), |a, v| {
        Ok(a.or(&Region::value(&canonical(v)?)))
    })
}

fn region(r: &DataRangeExpr, depth: usize) -> Result<Region, Unknown> {
    if depth > MAX_DEPTH {
        return Err(Unknown);
    }
    let sub = |r| region(r, depth + 1);
    Ok(match r {
        DataRangeExpr::Datatype(b) => universe(*b),
        DataRangeExpr::Restriction(b, facets) => {
            let mut acc = universe(*b);
            for (f, v) in facets {
                acc = acc.and(&facet_region(*b, *f, &canonical(v)?)?);
            }
            acc
        }
        DataRangeExpr::OneOf(vs) => one_of(vs)?,
        DataRangeExpr::And(rs) => {
            let mut acc = Region::full();
            for r in rs {
                acc = acc.and(&sub(r)?);
            }
            acc
        }
        DataRangeExpr::Or(rs) => {
            let mut acc = Region::empty();
            for r in rs {
                acc = acc.or(&sub(r)?);
            }
            acc
        }
        DataRangeExpr::Not(r) => sub(r)?.not(),
    })
}

fn universe(b: Builtin) -> Region {
    use Builtin::*;
    let reals = |layers: usize, ints: Cuts| {
        let sorts = (0..4).map(|s| match s {
            0 => ints.clone(),
            s if s < layers => Cuts::full(),
            _ => Cuts::empty(),
        });
        Region::ordered(Sp::Real, sorts.collect())
    };
    let text = |f| Region::strings(Space::Text, f);
    match b {
        RdfsLiteral => Region::full(),
        OwlReal => reals(4, Cuts::full()),
        OwlRational => reals(3, Cuts::full()),
        Decimal => reals(2, Cuts::full()),
        Double => Region::ordered(Sp::Double, vec![Cuts::full(); 2]),
        Float => Region::ordered(Sp::Float, vec![Cuts::full(); 2]),
        PlainLiteral => text(F::atom(Atom::Untagged).or(F::atom(Atom::PlainTag))),
        LangString => text(F::atom(Atom::Lang("*".into()))),
        String => text(F::atom(Atom::Untagged)),
        NormalizedString => text(F::atom(Atom::Plain(StrKind::NormalizedString))),
        Token => text(F::atom(Atom::Plain(StrKind::Token))),
        Language => text(F::atom(Atom::Plain(StrKind::Language))),
        Name => text(F::atom(Atom::Plain(StrKind::Name))),
        NcName => text(F::atom(Atom::Plain(StrKind::NcName))),
        NmToken => text(F::atom(Atom::Plain(StrKind::NmToken))),
        Boolean => Region::ordered(Sp::Bool, vec![Cuts::full()]),
        HexBinary => Region::ordered(Sp::Hex, vec![Cuts::full()]),
        Base64Binary => Region::ordered(Sp::B64, vec![Cuts::full()]),
        AnyUri => Region::strings(Space::Uri, F::Const(true)),
        DateTime => Region::ordered(Sp::Time, vec![Cuts::full(); 2]),
        DateTimeStamp => Region::ordered(Sp::Time, vec![Cuts::full(), Cuts::empty()]),
        XmlLiteral => Region::strings(Space::Xml, F::Const(true)),
        _ => {
            let (lo, hi) = integer_range(b).expect("every other builtin is an integer type");
            reals(1, int_range(lo, hi))
        }
    }
}

/// The values a facet–value pair admits within `b`'s own family (the caller intersects
/// with `b`). A pair outside `b`'s facet space is Unknown: the profile check rejects it.
fn facet_region(b: Builtin, f: Facet, v: &Value) -> Result<Region, Unknown> {
    use Builtin as B;
    if !b.allowed_facets().contains(&f) {
        return Err(Unknown);
    }
    let (inclusive, upper) = match f {
        Facet::MinInclusive => (true, false),
        Facet::MinExclusive => (false, false),
        Facet::MaxInclusive => (true, true),
        Facet::MaxExclusive => (false, true),
        _ => (false, false),
    };
    let bound = |inclusive: bool, k: Rat| {
        if upper {
            Cuts::at_most(k, inclusive)
        } else {
            Cuts::at_least(k, inclusive)
        }
    };
    let string_space = if b == B::AnyUri {
        Space::Uri
    } else {
        Space::Text
    };
    Ok(match (f, v) {
        (Facet::Length | Facet::MinLength | Facet::MaxLength, Value::Real(n)) => {
            let n = n.0;
            if !n.is_integer() || n.num < 0 {
                return Err(Unknown);
            }
            let (lo, hi) = match f {
                Facet::Length => (Some(n.num), Some(n.num)),
                Facet::MinLength => (Some(n.num), None),
                _ => (None, Some(n.num)),
            };
            match b {
                B::HexBinary => Region::ordered(Sp::Hex, vec![int_range(lo, hi)]),
                B::Base64Binary => Region::ordered(Sp::B64, vec![int_range(lo, hi)]),
                _ => {
                    let lo = u64::try_from(lo.unwrap_or(0)).map_err(|_| Unknown)?;
                    let hi = hi.map(u64::try_from).transpose().map_err(|_| Unknown)?;
                    Region::strings(string_space, F::atom(Atom::Len(lo, hi)))
                }
            }
        }
        // OWL 2 §4.5 gives the binary types no pattern facet.
        (Facet::Pattern, Value::String(p)) if !matches!(b, B::HexBinary | B::Base64Binary) => {
            match strings::pattern_status(p) {
                Some(true) => Region::strings(string_space, F::atom(Atom::Pattern(p.clone()))),
                _ => return Err(Unknown),
            }
        }
        (Facet::LangRange, Value::String(r)) if strings::is_language_range(r) => {
            Region::strings(Space::Text, F::atom(Atom::Lang(r.to_ascii_lowercase())))
        }
        (
            Facet::LangRange | Facet::Pattern | Facet::Length | Facet::MinLength | Facet::MaxLength,
            _,
        ) => {
            return Err(Unknown);
        }
        // Order facets: the bound must be in `b`'s value space (XML Schema 1.1 Part 2
        // §4.3.7–§4.3.10; OWL 2 §4.1 for `owl:real` and `owl:rational`).
        _ if !in_value_space(v, b) => return Err(Unknown),
        (_, Value::Real(k)) if b.families() == [super::datatypes::Family::Real] => {
            Region::ordered(Sp::Real, vec![bound(inclusive, Rat::of(&k.0)); 4])
        }
        (_, Value::Float(bits)) if b == B::Float => Region::ordered(
            Sp::Float,
            vec![
                float_bound(f64::from(f32::from_bits(*bits)), inclusive, upper, |x| {
                    f32_key(x as f32)
                }),
                Cuts::empty(),
            ],
        ),
        (_, Value::Double(bits)) if b == B::Double => Region::ordered(
            Sp::Double,
            vec![
                float_bound(f64::from_bits(*bits), inclusive, upper, f64_key),
                Cuts::empty(),
            ],
        ),
        (_, Value::DateTime(d)) if matches!(b, B::DateTime | B::DateTimeStamp) => {
            // A value with a time zone and one without are ordered only across the ±14:00
            // window, and are never equal (XML Schema 1.1 Part 2 §3.3.7.3).
            const WINDOW: i64 = 14 * 3600;
            let at = Rat::of(&d.seconds.0);
            let across = bound(
                false,
                at.add(&Rat::int(if upper { -WINDOW } else { WINDOW })),
            );
            let own = bound(inclusive, at);
            Region::ordered(
                Sp::Time,
                if d.tz.is_some() {
                    vec![own, across]
                } else {
                    vec![across, own]
                },
            )
        }
        _ => return Err(Unknown),
    })
}

/// The ordered floating-point values a min/max facet admits. Facets compare by numeric
/// equality, so a bound of either zero admits or excludes both zeros, and a NaN bound
/// admits nothing.
fn float_bound(x: f64, inclusive: bool, upper: bool, key: impl Fn(f64) -> i128) -> Cuts {
    if x.is_nan() {
        return Cuts::empty();
    }
    let k = |y: f64| Rat::int(key(y));
    match (x == 0.0, upper) {
        (true, false) => Cuts::at_least(k(if inclusive { -0.0 } else { 0.0 }), inclusive),
        (true, true) => Cuts::at_most(k(if inclusive { 0.0 } else { -0.0 }), inclusive),
        (false, false) => Cuts::at_least(k(x), inclusive),
        (false, true) => Cuts::at_most(k(x), inclusive),
    }
}

#[cfg(test)]
mod tests;
