//! Exact rationals over arbitrary-precision integers: the keys of every ordered value
//! space in the datatype map. No floating-point arithmetic happens here.

use crate::owl2dl::datatypes::Rational;
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

    /// The same value as a datatype-map rational.
    pub(crate) fn of(r: &Rational) -> Rat {
        Rat::new(r.num.into(), r.den.into())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_and_classifies_exactly() {
        let q = |n: i128, d: i128| Rat::of(&Rational::new(n, d).unwrap());
        assert_eq!(q(2, 4), q(1, 2));
        assert!(
            q(1, 3)
                < q(
                    333_333_333_333_333_333_333_333_333_333_334,
                    1_000_000_000_000_000_000_000_000_000_000_000
                )
        );
        assert!(!q(1, 3).is_decimal() && q(1, 8).is_decimal() && q(-7, 1).is_integer());
        assert_eq!(
            (q(-7, 2).floor(), q(-7, 2).ceil()),
            (BigInt::from(-4), BigInt::from(-3))
        );
        assert_eq!(
            q(i128::MAX, 1).add(&q(i128::MAX, 1)),
            Rat::int(BigInt::from(i128::MAX) * 2)
        );
    }
}
