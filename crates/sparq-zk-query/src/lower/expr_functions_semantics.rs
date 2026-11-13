//! Independent value semantics for the term, temporal and numeric function group.
use super::expr::{ExprOp, LaneType, constant_type};
use super::expr_semantics::{self as base, Error, Number, Result, XSD};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use oxrdf::{Literal, Term};
const SCALE: i128 = 1_000_000_000_000_000_000;
fn simple(term: &Term) -> std::result::Result<&str, Error> {
    match term {
        Term::Literal(l)
            if l.datatype().as_str() == format!("{XSD}string") && l.language().is_none() =>
        {
            Ok(l.value())
        }
        _ => Err(Error::Sparql),
    }
}
fn is_numeric(term: &Term) -> bool {
    let Term::Literal(l) = term else {
        return false;
    };
    let s = l.value().trim_matches([' ', '\t', '\r', '\n']);
    let ty = constant_type(term);
    if matches!(ty, LaneType::Float | LaneType::Double) {
        return base::number(term).is_ok();
    }
    if !matches!(ty, LaneType::Integer | LaneType::Decimal) {
        return false;
    }
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    if unsigned.is_empty() {
        return false;
    }
    if ty == LaneType::Integer {
        if !unsigned.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        let Some(n) = BigInt::parse_bytes(s.trim_start_matches('+').as_bytes(), 10) else {
            return false;
        };
        return base::integer_facets(
            &(n * SCALE),
            l.datatype().as_str().strip_prefix(XSD).unwrap_or(""),
        );
    }
    unsigned.bytes().any(|b| b.is_ascii_digit())
        && unsigned.bytes().filter(|&b| b == b'.').count() <= 1
        && unsigned.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}
fn round_positive64(n: f64) -> f64 {
    if !n.is_finite() || n == 0.0 {
        return n;
    }
    let t = n.trunc();
    let r = n - t;
    let v = if r >= 0.5 {
        t + 1.0
    } else if r < -0.5 {
        t - 1.0
    } else {
        t
    };
    if v == 0.0 && n.is_sign_negative() {
        -0.0
    } else {
        v
    }
}
fn round_positive32(n: f32) -> f32 {
    if !n.is_finite() || n == 0.0 {
        return n;
    }
    let t = n.trunc();
    let r = n - t;
    let v = if r >= 0.5 {
        t + 1.0
    } else if r < -0.5 {
        t - 1.0
    } else {
        t
    };
    if v == 0.0 && n.is_sign_negative() {
        -0.0
    } else {
        v
    }
}
fn numeric(term: &Term, op: &ExprOp) -> Result {
    let value = match base::number(term)? {
        Number::Exact(n, ty) => {
            let v = if *op == ExprOp::Abs {
                n.abs()
            } else if ty == LaneType::Integer {
                n
            } else {
                let negative = n.is_negative();
                let m = n.abs();
                let q = &m / SCALE;
                let r = (&m % SCALE).to_i128().unwrap();
                let inc = match op {
                    ExprOp::Round => r * 2 > SCALE || (r * 2 == SCALE && !negative),
                    ExprOp::Ceil => !negative && r != 0,
                    _ => negative && r != 0,
                };
                let v = (q + BigInt::from(u8::from(inc))) * SCALE;
                if negative { -v } else { v }
            };
            if v.abs() >= (BigInt::from(1) << 112) {
                return Err(Error::Capacity);
            }
            Number::Exact(v, ty)
        }
        Number::Float(n) => Number::Float(match op {
            ExprOp::Abs => n.abs(),
            ExprOp::Round => round_positive32(n),
            ExprOp::Ceil => n.ceil(),
            _ => n.floor(),
        }),
        Number::Double(n) => Number::Double(match op {
            ExprOp::Abs => n.abs(),
            ExprOp::Round => round_positive64(n),
            ExprOp::Ceil => n.ceil(),
            _ => n.floor(),
        }),
    };
    Ok(base::number_term(value))
}
#[derive(Clone, Debug)]
pub(super) struct Date {
    pub(super) year: u32,
    pub(super) month: u32,
    pub(super) day: u32,
    pub(super) hour: u32,
    pub(super) minute: u32,
    pub(super) second: u32,
    pub(super) fraction: BigInt,
    pub(super) offset: Option<i32>,
    pub(super) zone: String,
    pub(super) is_date: bool,
}
fn days(year: u32, month: u32) -> u32 {
    match month {
        2 => {
            if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    }
}
pub(super) fn date(term: &Term) -> std::result::Result<Date, Error> {
    let Term::Literal(l) = term else {
        return Err(Error::Sparql);
    };
    let is_date = l.datatype().as_str() == format!("{XSD}date");
    if !is_date && l.datatype().as_str() != format!("{XSD}dateTime") {
        return Err(Error::Sparql);
    }
    let text = l.value().trim_matches([' ', '\t', '\r', '\n']);
    let negative = text.starts_with('-');
    let text = text.strip_prefix('-').unwrap_or(text);
    let (y, rest) = text.split_once('-').ok_or(Error::Sparql)?;
    if y.len() < 4
        || !y.bytes().all(|b| b.is_ascii_digit())
        || y.bytes().all(|b| b == b'0')
        || (y.len() > 4 && y.starts_with('0'))
    {
        return Err(Error::Sparql);
    }
    let mut modulo = 0_u32;
    for b in y.bytes() {
        modulo = (modulo * 10 + u32::from(b - b'0')) % 400;
    }
    let parse = |s: &str| -> std::result::Result<u32, Error> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            Err(Error::Sparql)
        } else {
            s.parse().map_err(|_| Error::Sparql)
        }
    };
    let pair = |a, b| parse(rest.get(a..b).ok_or(Error::Sparql)?);
    let month = pair(0, 2)?;
    let day = pair(3, 5)?;
    let leap_year = if negative {
        (401 - modulo) % 400
    } else {
        modulo
    };
    if rest.as_bytes().get(2) != Some(&b'-') || day == 0 || day > days(leap_year, month) {
        return Err(Error::Sparql);
    }
    let mut suffix = rest.get(5..).ok_or(Error::Sparql)?;
    let (mut hour, minute, second) = (0, 0, 0);
    let mut fraction = BigInt::zero();
    let mut precision = false;
    let (minute, second) = if is_date {
        (minute, second)
    } else {
        if !suffix.starts_with('T')
            || suffix.as_bytes().get(3) != Some(&b':')
            || suffix.as_bytes().get(6) != Some(&b':')
        {
            return Err(Error::Sparql);
        }
        hour = parse(suffix.get(1..3).ok_or(Error::Sparql)?)?;
        let minute = parse(suffix.get(4..6).ok_or(Error::Sparql)?)?;
        let second = parse(suffix.get(7..9).ok_or(Error::Sparql)?)?;
        suffix = suffix.get(9..).ok_or(Error::Sparql)?;
        if let Some(t) = suffix.strip_prefix('.') {
            let n = t.bytes().take_while(u8::is_ascii_digit).count();
            if n == 0 {
                return Err(Error::Sparql);
            }
            let kept = &t[..n.min(18)];
            precision = t.as_bytes()[n.min(18)..n].iter().any(|&b| b != b'0');
            fraction = BigInt::parse_bytes(
                format!("{kept}{}", "0".repeat(18 - kept.len())).as_bytes(),
                10,
            )
            .unwrap();
            suffix = &t[n..];
        }
        if minute > 59
            || second > 59
            || hour > 24
            || (hour == 24 && (minute != 0 || second != 0 || !fraction.is_zero() || precision))
        {
            return Err(Error::Sparql);
        }
        (minute, second)
    };
    let offset = if suffix.is_empty() {
        None
    } else if suffix == "Z" {
        Some(0)
    } else {
        let b = suffix.as_bytes();
        if b.len() != 6 || !matches!(b[0], b'+' | b'-') || b[3] != b':' {
            return Err(Error::Sparql);
        }
        let h = parse(&suffix[1..3])?;
        let m = parse(&suffix[4..6])?;
        if m > 59 || h * 60 + m > 840 {
            return Err(Error::Sparql);
        }
        Some((h * 60 + m) as i32 * if b[0] == b'-' { -1 } else { 1 })
    };
    if negative || y.len() != 4 || precision {
        return Err(Error::Capacity);
    }
    let mut year = y.parse::<u32>().unwrap();
    let mut month = month;
    let mut day = day;
    if hour == 24 {
        hour = 0;
        day += 1;
        if day > days(year, month) {
            day = 1;
            month += 1;
            if month == 13 {
                month = 1;
                year += 1;
            }
        }
    }
    if year > 9999 {
        return Err(Error::Capacity);
    }
    Ok(Date {
        year,
        month,
        day,
        hour,
        minute,
        second,
        fraction,
        offset,
        zone: suffix.to_owned(),
        is_date,
    })
}
fn seconds(d: &Date) -> i64 {
    let y = i64::from(d.year) - i64::from(d.month <= 2);
    let era = y / 400;
    let yoe = y - era * 400;
    let m = i64::from(d.month);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let day =
        era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + i64::from(d.day) - 1;
    day * 86400 + i64::from(d.hour * 3600 + d.minute * 60 + d.second)
        - i64::from(d.offset.unwrap_or(0)) * 60
}
pub(super) fn compare_dates(a: &Term, b: &Term, op: &ExprOp) -> std::result::Result<bool, Error> {
    let a = date(a)?;
    let b = date(b)?;
    if a.is_date != b.is_date {
        return Err(Error::Sparql);
    }
    if a.offset.is_some() != b.offset.is_some() {
        return Err(Error::Capacity);
    }
    let order = (seconds(&a), &a.fraction).cmp(&(seconds(&b), &b.fraction));
    Ok(match op {
        ExprOp::Eq => order.is_eq(),
        ExprOp::Ne => !order.is_eq(),
        ExprOp::Lt => order.is_lt(),
        ExprOp::Le => !order.is_gt(),
        ExprOp::Gt => order.is_gt(),
        _ => !order.is_lt(),
    })
}
fn temporal(term: &Term, op: &ExprOp) -> Result {
    let d = date(term)?;
    if d.is_date {
        return Err(Error::Sparql);
    }
    if *op == ExprOp::Tz {
        return Ok(Literal::new_simple_literal(d.zone).into());
    }
    if *op == ExprOp::Timezone {
        let offset = d.offset.ok_or(Error::Sparql)?;
        let n = offset.unsigned_abs();
        let h = n / 60;
        let m = n % 60;
        let body = if n == 0 {
            "PT0S".to_owned()
        } else {
            format!(
                "PT{}{}",
                if h == 0 {
                    String::new()
                } else {
                    format!("{h}H")
                },
                if m == 0 {
                    String::new()
                } else {
                    format!("{m}M")
                }
            )
        };
        return Ok(base::typed(
            format!("{}{body}", if offset < 0 { "-" } else { "" }),
            "dayTimeDuration",
        ));
    }
    let n = match op {
        ExprOp::Year => d.year,
        ExprOp::Month => d.month,
        ExprOp::Day => d.day,
        ExprOp::Hours => d.hour,
        ExprOp::Minutes => d.minute,
        _ => d.second,
    };
    Ok(base::number_term(Number::Exact(
        BigInt::from(n) * SCALE
            + if *op == ExprOp::Seconds {
                d.fraction
            } else {
                BigInt::zero()
            },
        if *op == ExprOp::Seconds {
            LaneType::Decimal
        } else {
            LaneType::Integer
        },
    )))
}
fn resolve(source: &Term, base: &Term) -> Result {
    if matches!(source, Term::NamedNode(_)) {
        return Ok(source.clone());
    }
    let text = simple(source)?;
    let Term::NamedNode(base) = base else {
        return Err(Error::Sparql);
    };
    if text
        .bytes()
        .any(|b| b <= 32 || b == 127 || b"<>\"{}|\\^`".contains(&b))
    {
        return Err(Error::Sparql);
    }
    if !text.is_ascii() || text.contains(['[', ']']) {
        return Err(Error::Capacity);
    }
    let query = format!(
        "BASE <{base}> SELECT * WHERE {{ <{text}> <urn:p> <urn:o> }}",
        base = base.as_str()
    );
    let spargebra::Query::Select { pattern, .. } = spargebra::SparqlParser::new()
        .parse_query(&query)
        .map_err(|_| Error::Sparql)?
    else {
        unreachable!();
    };
    let spargebra::algebra::GraphPattern::Project { inner, .. } = pattern else {
        return Err(Error::Sparql);
    };
    let spargebra::algebra::GraphPattern::Bgp { patterns } = *inner else {
        return Err(Error::Sparql);
    };
    let Some(spargebra::term::TriplePattern {
        subject: spargebra::term::TermPattern::NamedNode(n),
        ..
    }) = patterns.first()
    else {
        return Err(Error::Sparql);
    };
    Ok(n.clone().into())
}
pub(super) fn function(op: &ExprOp, args: &[std::result::Result<Term, Error>]) -> Result {
    let a = args[0].as_ref().map_err(|e| *e)?;
    match op {
        ExprOp::IsIri => Ok(base::bool_term(matches!(a, Term::NamedNode(_)))),
        ExprOp::IsBlank => Ok(base::bool_term(matches!(a, Term::BlankNode(_)))),
        ExprOp::IsLiteral => Ok(base::bool_term(matches!(a, Term::Literal(_)))),
        ExprOp::IsNumeric => Ok(base::bool_term(is_numeric(a))),
        ExprOp::Str => match a {
            Term::NamedNode(n) => Ok(Literal::new_simple_literal(n.as_str()).into()),
            Term::Literal(l) => Ok(Literal::new_simple_literal(l.value()).into()),
            _ => Err(Error::Sparql),
        },
        ExprOp::Lang => match a {
            Term::Literal(l) => Ok(Literal::new_simple_literal(l.language().unwrap_or("")).into()),
            _ => Err(Error::Sparql),
        },
        ExprOp::Datatype => match a {
            Term::Literal(l) => Ok(l.datatype().into_owned().into()),
            _ => Err(Error::Sparql),
        },
        ExprOp::LangMatches => {
            let tag = simple(a)?;
            let range = simple(args[1].as_ref().map_err(|e| *e)?)?;
            let matched = if range == "*" {
                !tag.is_empty()
            } else {
                !range.is_empty()
                    && tag
                        .get(..range.len())
                        .is_some_and(|p| p.eq_ignore_ascii_case(range))
                    && (tag.len() == range.len() || tag.as_bytes().get(range.len()) == Some(&b'-'))
            };
            Ok(base::bool_term(matched))
        }
        ExprOp::Iri => resolve(a, args[1].as_ref().map_err(|e| *e)?),
        ExprOp::StrDt => {
            let lexical = simple(a)?;
            let Term::NamedNode(dt) = args[1].as_ref().map_err(|e| *e)? else {
                return Err(Error::Sparql);
            };
            Ok(Literal::new_typed_literal(lexical, dt.clone()).into())
        }
        ExprOp::StrLang => {
            let lexical = simple(a)?;
            let tag = simple(args[1].as_ref().map_err(|e| *e)?)?;
            Ok(Literal::new_language_tagged_literal(lexical, tag)
                .map_err(|_| Error::Sparql)?
                .into())
        }
        ExprOp::Abs | ExprOp::Round | ExprOp::Ceil | ExprOp::Floor => numeric(a, op),
        ExprOp::Year
        | ExprOp::Month
        | ExprOp::Day
        | ExprOp::Hours
        | ExprOp::Minutes
        | ExprOp::Seconds
        | ExprOp::Timezone
        | ExprOp::Tz => temporal(a, op),
        _ => Err(Error::Sparql),
    }
}
