//! Exact rationals over arbitrary-precision integers: the keys of every ordered value
//! space in the datatype map. No floating-point arithmetic happens here.

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, Zero};
use std::cmp::Ordering;

/// A rational `n / d` in lowest terms with `d > 0`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Rat {
    n: BigInt,
    d: BigInt,
}

impl Rat {
    pub(crate) fn new(n: BigInt, d: BigInt) -> Rat {
        debug_assert!(!d.is_zero());
        let g = n.gcd(&d);
        let (mut n, mut d) = (n / &g, d / &g);
        if d.is_negative() {
            n = -n;
            d = -d;
        }
        Rat { n, d }
    }

    pub(crate) fn int(n: impl Into<BigInt>) -> Rat {
        Rat {
            n: n.into(),
            d: BigInt::one(),
        }
    }

    pub(crate) fn is_integer(&self) -> bool {
        self.d.is_one()
    }

    /// True iff the value has a finite decimal expansion (the denominator has no prime
    /// factors other than 2 and 5).
    pub(crate) fn is_decimal(&self) -> bool {
        let mut d = self.d.clone();
        for p in [2u32, 5] {
            let p = BigInt::from(p);
            while (&d % &p).is_zero() {
                d /= &p;
            }
        }
        d.is_one()
    }

    pub(crate) fn floor(&self) -> BigInt {
        self.n.div_floor(&self.d)
    }

    pub(crate) fn ceil(&self) -> BigInt {
        -((-&self.n).div_floor(&self.d))
    }

    pub(crate) fn add(&self, o: &Rat) -> Rat {
        Rat::new(&self.n * &o.d + &o.n * &self.d, &self.d * &o.d)
    }

    pub(crate) fn is_negative(&self) -> bool {
        self.n.is_negative()
    }

    /// Parses an `xsd:decimal` lexical form: `(\+|-)?([0-9]+(\.[0-9]*)?|\.[0-9]+)`.
    pub(crate) fn parse_decimal(s: &str) -> Option<Rat> {
        let (neg, body) = sign(s);
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if int.is_empty() && frac.is_empty() || !all_digits(int) || !all_digits(frac) {
            return None;
        }
        let digits = BigInt::parse_bytes(format!("0{int}{frac}").as_bytes(), 10)?;
        let scale = BigInt::from(10u32).pow(u32::try_from(frac.len()).ok()?);
        Some(Rat::new(if neg { -digits } else { digits }, scale))
    }

    /// Parses an `xsd:integer` lexical form: `(\+|-)?[0-9]+`.
    pub(crate) fn parse_integer(s: &str) -> Option<Rat> {
        let (neg, body) = sign(s);
        if body.is_empty() || !all_digits(body) {
            return None;
        }
        let n = BigInt::parse_bytes(body.as_bytes(), 10)?;
        Some(Rat::int(if neg { -n } else { n }))
    }

    /// Parses an `owl:rational` lexical form: an `xsd:integer` numerator, `/`, and a
    /// positive denominator written without a sign (OWL 2 Structural Specification §4.1).
    pub(crate) fn parse_rational(s: &str) -> Option<Rat> {
        let (num, den) = s.split_once('/')?;
        let n = Rat::parse_integer(num)?;
        if den.is_empty() || !all_digits(den) {
            return None;
        }
        let d = BigInt::parse_bytes(den.as_bytes(), 10)?;
        (!d.is_zero()).then(|| Rat::new(n.n, d))
    }
}

impl Ord for Rat {
    fn cmp(&self, o: &Rat) -> Ordering {
        (&self.n * &o.d).cmp(&(&o.n * &self.d))
    }
}

impl PartialOrd for Rat {
    fn partial_cmp(&self, o: &Rat) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn sign(s: &str) -> (bool, &str) {
    match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    }
}

fn all_digits(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_exactly() {
        let r = |s| Rat::parse_rational(s).unwrap();
        let d = |s| Rat::parse_decimal(s).unwrap();
        assert_eq!(r("2/4"), r("1/2"));
        assert_eq!(d("0.50"), r("1/2"));
        assert_eq!(d("-.5"), r("-1/2"));
        assert!(r("1/3") < d("0.33333333333333333333333333333333333334"));
        assert!(r("1/3") > d("0.33333333333333333333333333333333333333"));
        for bad in ["", ".", "+", "1.2.3", "1e3", " 1"] {
            assert!(Rat::parse_decimal(bad).is_none(), "{bad}");
        }
        for bad in ["1/0", "1/+2", "1/-2", "/2", "1/", "1.0/2"] {
            assert!(Rat::parse_rational(bad).is_none(), "{bad}");
        }
        assert!(!r("1/3").is_decimal() && r("1/8").is_decimal() && r("-7/1").is_integer());
        assert_eq!(
            (r("-7/2").floor(), r("-7/2").ceil()),
            (BigInt::from(-4), BigInt::from(-3))
        );
    }
}
