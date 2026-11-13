//! Independent host mirror for differential tests. It is not a proving API.
use std::collections::BTreeMap;

use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use oxrdf::{Literal, NamedNode, Term};

use super::expr::{Expr, ExprOp, LaneType, constant_type};

pub(super) const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const SCALE: i128 = 1_000_000_000_000_000_000;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Error {
    Sparql,
    Capacity,
}
pub(super) type Result = std::result::Result<Term, Error>;
#[derive(Clone, Debug)]
pub(super) enum Number {
    Exact(BigInt, LaneType),
    Float(f32),
    Double(f64),
}
fn xml_trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r'])
}
fn datatype(lit: &Literal) -> String {
    lit.datatype()
        .as_str()
        .strip_prefix(XSD)
        .unwrap_or("")
        .to_owned()
}
fn boolean(lit: &Literal) -> std::result::Result<bool, Error> {
    match xml_trim(lit.value()) {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(Error::Sparql),
    }
}
fn decimal(s: &str, integer: bool) -> std::result::Result<BigInt, Error> {
    let s = xml_trim(s);
    let (negative, s) = if let Some(s) = s.strip_prefix('-') {
        (true, s)
    } else {
        (false, s.strip_prefix('+').unwrap_or(s))
    };
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if (integer && s.contains('.'))
        || (whole.is_empty() && frac.is_empty())
        || !whole
            .bytes()
            .chain(frac.bytes())
            .all(|c| c.is_ascii_digit())
    {
        return Err(Error::Sparql);
    }
    let frac = frac.trim_end_matches('0');
    if frac.len() > 18 {
        return Err(Error::Capacity);
    }
    let digits = format!("{whole}{frac}{}", "0".repeat(18 - frac.len()));
    let mag = BigInt::parse_bytes(digits.as_bytes(), 10).ok_or(Error::Sparql)?;
    let value = if negative { -mag } else { mag };
    if value.abs() >= (BigInt::from(1) << 112) {
        return Err(Error::Capacity);
    }
    Ok(value)
}
fn integer_facets(value: &BigInt, dt: &str) -> bool {
    let n = value / SCALE;
    match dt {
        "byte" => n >= (-128).into() && n <= 127.into(),
        "short" => n >= (-32768).into() && n <= 32767.into(),
        "int" => n >= i32::MIN.into() && n <= i32::MAX.into(),
        "long" => n >= i64::MIN.into() && n <= i64::MAX.into(),
        "unsignedByte" => !n.is_negative() && n <= 255.into(),
        "unsignedShort" => !n.is_negative() && n <= 65535.into(),
        "unsignedInt" => !n.is_negative() && n <= u32::MAX.into(),
        "unsignedLong" => !n.is_negative() && n <= u64::MAX.into(),
        "positiveInteger" => n > 0.into(),
        "negativeInteger" => n < 0.into(),
        "nonNegativeInteger" => n >= 0.into(),
        "nonPositiveInteger" => n <= 0.into(),
        _ => true,
    }
}
fn float_lexical(s: &str) -> bool {
    if matches!(s, "INF" | "-INF" | "NaN") {
        return true;
    }
    // Rust's parser handles the decimal/exponent grammar; exclude its extra
    // non-XSD infinity spellings and require at least one decimal digit.
    s.bytes()
        .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
        && s.bytes().any(|b| b.is_ascii_digit())
}
pub(super) fn number(term: &Term) -> std::result::Result<Number, Error> {
    let Term::Literal(lit) = term else {
        return Err(Error::Sparql);
    };
    let ty = constant_type(term);
    let dt = datatype(lit);
    let s = xml_trim(lit.value());
    match ty {
        LaneType::Integer | LaneType::Decimal => {
            let value = decimal(s, ty == LaneType::Integer)?;
            if ty == LaneType::Integer && !integer_facets(&value, &dt) {
                return Err(Error::Sparql);
            }
            Ok(Number::Exact(value, ty))
        }
        LaneType::Float | LaneType::Double => {
            if !float_lexical(s) {
                return Err(Error::Sparql);
            }
            let s = match s {
                "INF" => "inf",
                "-INF" => "-inf",
                s => s,
            };
            if ty == LaneType::Float {
                Ok(Number::Float(s.parse().map_err(|_| Error::Sparql)?))
            } else {
                Ok(Number::Double(s.parse().map_err(|_| Error::Sparql)?))
            }
        }
        _ => Err(Error::Sparql),
    }
}
fn exact_text(value: &BigInt, decimal_type: bool) -> String {
    let negative = value.is_negative();
    let mag = value.abs();
    let whole = &mag / SCALE;
    let fraction = (&mag % SCALE).to_u64().expect("scale remainder");
    let sign = if negative { "-" } else { "" };
    if !decimal_type {
        format!("{sign}{whole}")
    } else if fraction == 0 {
        format!("{sign}{whole}.0")
    } else {
        format!(
            "{sign}{whole}.{}",
            format!("{fraction:018}").trim_end_matches('0')
        )
    }
}
pub(super) fn typed(value: impl Into<String>, dt: &str) -> Term {
    Literal::new_typed_literal(value, NamedNode::new(format!("{XSD}{dt}")).unwrap()).into()
}
pub(super) fn bool_term(value: bool) -> Term {
    typed(if value { "true" } else { "false" }, "boolean")
}
fn number_term(n: Number) -> Term {
    match n {
        Number::Exact(n, ty) => typed(
            exact_text(&n, ty == LaneType::Decimal),
            if ty == LaneType::Decimal {
                "decimal"
            } else {
                "integer"
            },
        ),
        Number::Float(n) => typed(
            if n.is_nan() {
                "NaN".into()
            } else if n == f32::INFINITY {
                "INF".into()
            } else if n == f32::NEG_INFINITY {
                "-INF".into()
            } else {
                format!("{n:e}")
            },
            "float",
        ),
        Number::Double(n) => typed(
            if n.is_nan() {
                "NaN".into()
            } else if n == f64::INFINITY {
                "INF".into()
            } else if n == f64::NEG_INFINITY {
                "-INF".into()
            } else {
                format!("{n:e}")
            },
            "double",
        ),
    }
}
impl Number {
    fn float(&self) -> f32 {
        match self {
            Self::Float(v) => *v,
            Self::Double(_) => unreachable!("no narrowing promotion"),
            Self::Exact(n, _) => exact_text(n, true).parse().unwrap(),
        }
    }
    fn double(&self) -> f64 {
        match self {
            Self::Float(v) => f64::from(*v),
            Self::Double(v) => *v,
            Self::Exact(n, _) => exact_text(n, true).parse().unwrap(),
        }
    }
}
pub(super) fn ebv(term: &Term) -> std::result::Result<bool, Error> {
    match constant_type(term) {
        LaneType::Boolean => {
            let Term::Literal(lit) = term else {
                unreachable!()
            };
            Ok(boolean(lit).unwrap_or(false))
        }
        LaneType::String => {
            let Term::Literal(lit) = term else {
                unreachable!()
            };
            Ok(!lit.value().is_empty())
        }
        LaneType::Integer | LaneType::Decimal | LaneType::Float | LaneType::Double => {
            match number(term) {
                Ok(Number::Exact(n, _)) => Ok(!n.is_zero()),
                Ok(Number::Float(n)) => Ok(n != 0. && !n.is_nan()),
                Ok(Number::Double(n)) => Ok(n != 0. && !n.is_nan()),
                Err(Error::Sparql) => Ok(false),
                Err(e) => Err(e),
            }
        }
        _ => Err(Error::Sparql),
    }
}
fn relation<T: PartialEq + PartialOrd>(a: T, b: T, op: &ExprOp) -> bool {
    match op {
        ExprOp::Eq => a == b,
        ExprOp::Ne => a != b,
        ExprOp::Lt => a < b,
        ExprOp::Le => a <= b,
        ExprOp::Gt => a > b,
        ExprOp::Ge => a >= b,
        _ => unreachable!(),
    }
}
fn is_numeric(ty: LaneType) -> bool {
    matches!(
        ty,
        LaneType::Integer | LaneType::Decimal | LaneType::Float | LaneType::Double
    )
}
fn date_value(term: &Term) -> std::result::Result<(u32, u32, u32), Error> {
    let Term::Literal(literal) = term else {
        return Err(Error::Sparql);
    };
    let s = literal.value().as_bytes();
    if s.len() != 10 {
        return Err(Error::Capacity);
    }
    if s[4] != b'-'
        || s[7] != b'-'
        || !s
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return Err(Error::Sparql);
    }
    let parse = |a, b| {
        literal.value()[a..b]
            .parse::<u32>()
            .map_err(|_| Error::Sparql)
    };
    let (y, m, d) = (parse(0, 4)?, parse(5, 7)?, parse(8, 10)?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    if y == 0 || d == 0 || d > days {
        return Err(Error::Sparql);
    }
    Ok((y, m, d))
}

fn compare(a: &Term, b: &Term, op: &ExprOp) -> std::result::Result<bool, Error> {
    let at = constant_type(a);
    let bt = constant_type(b);
    if at == LaneType::Date && bt == LaneType::Date {
        return Ok(relation(date_value(a)?, date_value(b)?, op));
    }
    if is_numeric(at) && is_numeric(bt) {
        let x = number(a)?;
        let y = number(b)?;
        return Ok(
            if matches!(x, Number::Double(_)) || matches!(y, Number::Double(_)) {
                relation(x.double(), y.double(), op)
            } else if matches!(x, Number::Float(_)) || matches!(y, Number::Float(_)) {
                relation(x.float(), y.float(), op)
            } else {
                let (Number::Exact(x, _), Number::Exact(y, _)) = (x, y) else {
                    unreachable!()
                };
                relation(x, y, op)
            },
        );
    }
    if let (Term::Literal(a), Term::Literal(b)) = (a, b) {
        if at == LaneType::Boolean && bt == LaneType::Boolean {
            return Ok(relation(boolean(a)?, boolean(b)?, op));
        }
        if at == LaneType::String && bt == LaneType::String {
            if matches!(op, ExprOp::Eq | ExprOp::Ne) {
                let same = a.language() == b.language() && a.value() == b.value();
                return Ok(if *op == ExprOp::Eq { same } else { !same });
            }
            if a.language().is_some() || b.language().is_some() {
                return Err(Error::Sparql);
            }
            return Ok(relation(a.value(), b.value(), op));
        }
        if matches!(op, ExprOp::Eq | ExprOp::Ne) && a == b {
            return Ok(*op == ExprOp::Eq);
        }
        return Err(Error::Sparql);
    }
    if matches!(op, ExprOp::Eq | ExprOp::Ne) {
        Ok(if *op == ExprOp::Eq { a == b } else { a != b })
    } else {
        Err(Error::Sparql)
    }
}
fn arithmetic(a: &Term, b: &Term, op: &ExprOp) -> Result {
    let x = number(a)?;
    let y = number(b)?;
    let n = if matches!(x, Number::Double(_)) || matches!(y, Number::Double(_)) {
        let x = x.double();
        let y = y.double();
        Number::Double(match op {
            ExprOp::Add => x + y,
            ExprOp::Sub => x - y,
            ExprOp::Mul => x * y,
            _ => x / y,
        })
    } else if matches!(x, Number::Float(_)) || matches!(y, Number::Float(_)) {
        let x = x.float();
        let y = y.float();
        Number::Float(match op {
            ExprOp::Add => x + y,
            ExprOp::Sub => x - y,
            ExprOp::Mul => x * y,
            _ => x / y,
        })
    } else {
        let (Number::Exact(x, xt), Number::Exact(y, yt)) = (x, y) else {
            unreachable!()
        };
        let ty = super::expr::promote(xt, yt, *op == ExprOp::Div);
        let z = match op {
            ExprOp::Add => x + y,
            ExprOp::Sub => x - y,
            ExprOp::Mul => {
                let n = x * y;
                if &n % SCALE != BigInt::zero() {
                    return Err(Error::Capacity);
                }
                n / SCALE
            }
            _ => {
                if y.is_zero() {
                    return Err(Error::Sparql);
                }
                let n = x * SCALE;
                if &n % &y != BigInt::zero() {
                    return Err(Error::Capacity);
                }
                n / y
            }
        };
        if z.abs() >= (BigInt::from(1) << 112) {
            return Err(Error::Capacity);
        }
        Number::Exact(z, ty)
    };
    Ok(number_term(n))
}
fn logical(
    a: std::result::Result<bool, Error>,
    b: std::result::Result<bool, Error>,
    and: bool,
) -> std::result::Result<bool, Error> {
    if a == Err(Error::Capacity) || b == Err(Error::Capacity) {
        return Err(Error::Capacity);
    }
    if a == Ok(!and) || b == Ok(!and) {
        return Ok(!and);
    }
    Ok(if and { a? && b? } else { a? || b? })
}
pub(super) fn eval(expr: &Expr, vars: &BTreeMap<u32, Term>) -> Result {
    match expr {
        Expr::BooleanInput { slot } => {
            let Some(Term::Literal(value)) = vars.get(slot) else {
                return Err(Error::Sparql);
            };
            if value.datatype().as_str() != format!("{XSD}boolean") {
                return Err(Error::Sparql);
            }
            boolean(value).map(bool_term)
        }
        Expr::Constant(t) => Ok(t.clone()),
        Expr::Variable { slot, .. } => vars.get(slot).cloned().ok_or(Error::Sparql),
        Expr::Bound { slot } => Ok(bool_term(vars.contains_key(slot))),
        Expr::Call { op, args } => {
            let arg = |i: usize| eval(&args[i], vars);
            let arg_ebv = |i| arg(i).and_then(|v| ebv(&v));
            match op {
                ExprOp::And | ExprOp::Or => {
                    logical(arg_ebv(0), arg_ebv(1), *op == ExprOp::And).map(bool_term)
                }
                ExprOp::Not => arg_ebv(0).map(|v| bool_term(!v)),
                ExprOp::If => {
                    if arg_ebv(0)? {
                        arg(1)
                    } else {
                        arg(2)
                    }
                }
                ExprOp::Coalesce => {
                    for i in 0..args.len() {
                        match arg(i) {
                            Err(Error::Sparql) => {}
                            result => return result,
                        }
                    }
                    Err(Error::Sparql)
                }
                ExprOp::SameTerm => Ok(bool_term(arg(0)? == arg(1)?)),
                ExprOp::In | ExprOp::NotIn => {
                    let mut result = Ok(false);
                    for i in 1..args.len() {
                        let item =
                            arg(0).and_then(|a| arg(i).and_then(|b| compare(&a, &b, &ExprOp::Eq)));
                        result = logical(result, item, false);
                    }
                    result.map(|v| bool_term(if *op == ExprOp::NotIn { !v } else { v }))
                }
                ExprOp::Eq | ExprOp::Ne | ExprOp::Lt | ExprOp::Le | ExprOp::Gt | ExprOp::Ge => {
                    compare(&arg(0)?, &arg(1)?, op).map(bool_term)
                }
                ExprOp::Add | ExprOp::Sub | ExprOp::Mul | ExprOp::Div => {
                    arithmetic(&arg(0)?, &arg(1)?, op)
                }
                ExprOp::UnaryPlus => {
                    let t = arg(0)?;
                    number(&t)?;
                    Ok(t)
                }
                ExprOp::UnaryMinus => {
                    let n = match number(&arg(0)?)? {
                        Number::Exact(n, ty) => Number::Exact(-n, ty),
                        Number::Float(n) => Number::Float(-n),
                        Number::Double(n) => Number::Double(-n),
                    };
                    Ok(number_term(n))
                }
                ExprOp::Unsupported(_) => Err(Error::Sparql),
            }
        }
    }
}
/// Compare full datatype and value, while tolerating permitted numeric lexical spellings.
pub(super) fn normalized(term: &Term) -> String {
    match number(term) {
        Ok(Number::Exact(n, ty)) => format!("{ty:?}:{}", exact_text(&n, true)),
        Ok(Number::Float(n)) => format!(
            "Float:{:08x}",
            if n.is_nan() {
                f32::NAN.to_bits()
            } else {
                n.to_bits()
            }
        ),
        Ok(Number::Double(n)) => format!(
            "Double:{:016x}",
            if n.is_nan() {
                f64::NAN.to_bits()
            } else {
                n.to_bits()
            }
        ),
        _ => term.to_string(),
    }
}
