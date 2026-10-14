//! Finite unions of intervals over the rationals, kept as a sorted list of *cuts*.
//!
//! A cut sits just below or just above a rational: `Below(x) < x < Above(x)`. A set is
//! its membership at −∞ plus the cuts where membership flips, so union, intersection and
//! complement are one merge of two sorted lists, with open and closed ends exact.

use super::rat::Rat;
use num_bigint::BigInt;
use std::cmp::Ordering;

/// A position between rationals: just below `x` (`above == false`) or just above it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cut {
    pub(crate) x: Rat,
    pub(crate) above: bool,
}

impl Ord for Cut {
    fn cmp(&self, o: &Cut) -> Ordering {
        self.x.cmp(&o.x).then(self.above.cmp(&o.above))
    }
}

impl PartialOrd for Cut {
    fn partial_cmp(&self, o: &Cut) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// A finite union of intervals with rational (or infinite) ends.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct Cuts {
    /// Membership below every cut.
    pub(crate) from_neg_inf: bool,
    /// Strictly increasing flip points.
    pub(crate) cuts: Vec<Cut>,
}

/// One maximal interval of a [`Cuts`]: the points strictly between its two cuts
/// (`None` = unbounded on that side).
pub(crate) struct Segment<'a> {
    pub(crate) lo: Option<&'a Cut>,
    pub(crate) hi: Option<&'a Cut>,
}

impl Cuts {
    pub(crate) fn empty() -> Cuts {
        Cuts::default()
    }

    pub(crate) fn full() -> Cuts {
        Cuts {
            from_neg_inf: true,
            cuts: Vec::new(),
        }
    }

    /// `{x}`.
    pub(crate) fn point(x: Rat) -> Cuts {
        Cuts {
            from_neg_inf: false,
            cuts: vec![
                Cut {
                    x: x.clone(),
                    above: false,
                },
                Cut { x, above: true },
            ],
        }
    }

    /// `[x, ∞)` when `inclusive`, else `(x, ∞)`.
    pub(crate) fn at_least(x: Rat, inclusive: bool) -> Cuts {
        Cuts {
            from_neg_inf: false,
            cuts: vec![Cut {
                x,
                above: !inclusive,
            }],
        }
    }

    /// `(−∞, x]` when `inclusive`, else `(−∞, x)`.
    pub(crate) fn at_most(x: Rat, inclusive: bool) -> Cuts {
        Cuts {
            from_neg_inf: true,
            cuts: vec![Cut {
                x,
                above: inclusive,
            }],
        }
    }

    /// `[lo, hi]`.
    pub(crate) fn closed(lo: Rat, hi: Rat) -> Cuts {
        Cuts::at_least(lo, true).and(&Cuts::at_most(hi, true))
    }

    pub(crate) fn contains(&self, x: &Rat) -> bool {
        let below = self
            .cuts
            .partition_point(|c| c.x < *x || (c.x == *x && !c.above));
        self.from_neg_inf ^ (below % 2 == 1)
    }

    pub(crate) fn not(&self) -> Cuts {
        Cuts {
            from_neg_inf: !self.from_neg_inf,
            cuts: self.cuts.clone(),
        }
    }

    pub(crate) fn and(&self, o: &Cuts) -> Cuts {
        self.merge(o, |a, b| a && b)
    }

    pub(crate) fn or(&self, o: &Cuts) -> Cuts {
        self.merge(o, |a, b| a || b)
    }

    fn merge(&self, o: &Cuts, op: impl Fn(bool, bool) -> bool) -> Cuts {
        let (mut a, mut b) = (self.from_neg_inf, o.from_neg_inf);
        let start = op(a, b);
        let (mut cur, mut out) = (start, Vec::new());
        let (mut i, mut j) = (0, 0);
        while i < self.cuts.len() || j < o.cuts.len() {
            let next = match (self.cuts.get(i), o.cuts.get(j)) {
                (Some(x), Some(y)) => x.min(y),
                (Some(x), None) => x,
                (None, Some(y)) => y,
                (None, None) => unreachable!(),
            }
            .clone();
            if self.cuts.get(i) == Some(&next) {
                a = !a;
                i += 1;
            }
            if o.cuts.get(j) == Some(&next) {
                b = !b;
                j += 1;
            }
            if op(a, b) != cur {
                cur = !cur;
                out.push(next);
            }
        }
        Cuts {
            from_neg_inf: start,
            cuts: out,
        }
    }

    /// The maximal intervals of the set, in order.
    pub(crate) fn segments(&self) -> impl Iterator<Item = Segment<'_>> {
        let first = self.from_neg_inf.then_some(Segment {
            lo: None,
            hi: self.cuts.first(),
        });
        let skip = usize::from(self.from_neg_inf);
        let rest = self.cuts[skip.min(self.cuts.len())..]
            .chunks(2)
            .map(|c| Segment {
                lo: Some(&c[0]),
                hi: c.get(1),
            });
        first.into_iter().chain(rest)
    }
}

impl Segment<'_> {
    /// The single point of a degenerate interval `[x, x]`.
    pub(crate) fn point(&self) -> Option<&Rat> {
        match (self.lo, self.hi) {
            (Some(l), Some(h)) if !l.above && h.above && l.x == h.x => Some(&l.x),
            _ => None,
        }
    }

    /// The integers in the interval as `(first, last)`, `None` for an unbounded side;
    /// `Some((a, b))` with `a > b` when there are none.
    pub(crate) fn integers(&self) -> (Option<BigInt>, Option<BigInt>) {
        let lo = self
            .lo
            .map(|c| if c.above { c.x.floor() + 1 } else { c.x.ceil() });
        let hi = self
            .hi
            .map(|c| if c.above { c.x.floor() } else { c.x.ceil() - 1 });
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(n: i64) -> Rat {
        Rat::int(n)
    }

    #[test]
    fn algebra_keeps_open_and_closed_ends() {
        let a = Cuts::closed(q(0), q(10));
        let b = Cuts::at_least(q(10), false);
        assert!(a.and(&b).segments().next().is_none());
        let c = Cuts::at_least(q(10), true);
        let ac = a.and(&c);
        assert_eq!(ac.segments().next().unwrap().point(), Some(&q(10)));
        let u = a.or(&b);
        assert!(u.contains(&q(10)) && u.contains(&q(11)) && !u.contains(&q(-1)));
        assert_eq!(u.cuts.len(), 1, "[0,10] ∪ (10,∞) = [0,∞)");
        let n = a.not();
        assert!(
            n.contains(&q(-1)) && !n.contains(&q(0)) && !n.contains(&q(10)) && n.contains(&q(11))
        );
        assert_eq!(n.segments().count(), 2);
        assert_eq!(Cuts::full().not(), Cuts::empty());
    }

    #[test]
    fn integer_bounds_of_segments() {
        let s =
            Cuts::at_least(Rat::new(1.into(), 2.into()), false).and(&Cuts::at_most(q(3), false));
        let seg = s.segments().next().unwrap();
        assert_eq!(seg.integers(), (Some(1.into()), Some(2.into())));
        let open = Cuts::at_least(q(0), false).and(&Cuts::at_most(q(1), false));
        let (lo, hi) = open.segments().next().unwrap().integers();
        assert!(lo.unwrap() > hi.unwrap());
    }
}
