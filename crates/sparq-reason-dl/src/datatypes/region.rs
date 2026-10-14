//! Sets of data values, closed under union, intersection and complement, with exact
//! emptiness.
//!
//! The value spaces of the datatype map are pairwise disjoint (OWL 2 Structural
//! Specification §4), so a set is one component per space. An ordered space is split
//! into *sorts* that share one ordering key: the real line into integers, the other
//! decimals, the other rationals and the irrationals; a floating-point space into its
//! ordered values and NaN; time instants into those with and without a time zone. A
//! component is a [`Cuts`] per sort plus finitely many values added outside them or
//! removed inside them, which is exactly the shape the data-range constructors produce.

use super::cuts::{Cuts, Segment};
use super::rat::Rat;
use super::strings::{self, F};
use super::Tri;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::collections::BTreeSet;

/// A data value. Identity, not equality: `-0.0` and `+0.0` differ, as do the same
/// instant written with two offsets.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Val {
    Real(Rat),
    Float(u32),
    Double(u64),
    Bool(bool),
    Hex(Vec<u8>),
    B64(Vec<u8>),
    /// `at` is the instant for a value with an offset, else the local time read as UTC.
    Time {
        at: Rat,
        offset: Option<i16>,
    },
    Str(String),
    Lang(String, String),
    Uri(String),
}

/// The ordered spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sp {
    Real,
    Float,
    Double,
    Bool,
    Hex,
    B64,
    Time,
}

const SPACES: [Sp; 7] = [
    Sp::Real,
    Sp::Float,
    Sp::Double,
    Sp::Bool,
    Sp::Hex,
    Sp::B64,
    Sp::Time,
];

/// Offsets a value with a time zone can carry: -14:00 to +14:00 in minutes.
const OFFSETS: u128 = 2 * 14 * 60 + 1;

impl Sp {
    fn sorts(self) -> usize {
        match self {
            Sp::Real => 4,
            Sp::Float | Sp::Double | Sp::Time => 2,
            Sp::Bool | Sp::Hex | Sp::B64 => 1,
        }
    }

    pub(crate) fn universe(self, sort: usize) -> Cuts {
        match (self, sort) {
            (Sp::Float, 0) => Cuts::closed(
                Rat::int(f32_key(f32::NEG_INFINITY)),
                Rat::int(f32_key(f32::INFINITY)),
            ),
            (Sp::Double, 0) => Cuts::closed(
                Rat::int(f64_key(f64::NEG_INFINITY)),
                Rat::int(f64_key(f64::INFINITY)),
            ),
            (Sp::Float | Sp::Double, _) => Cuts::point(Rat::int(0)),
            (Sp::Bool, _) => Cuts::closed(Rat::int(0), Rat::int(1)),
            (Sp::Hex | Sp::B64, _) => Cuts::at_least(Rat::int(0), true),
            _ => Cuts::full(),
        }
    }

    /// Whether a sort's keys are integers (each key a separate group of values).
    fn discrete(self, sort: usize) -> bool {
        !matches!((self, sort), (Sp::Real, 1..) | (Sp::Time, _))
    }

    /// How many values share ordering key `k` in `sort`, saturating.
    fn fiber(self, sort: usize, k: &Rat) -> u128 {
        match (self, sort) {
            (Sp::Real, 0) => u128::from(k.is_integer()),
            (Sp::Real, s) => u128::from(layer(k) == s),
            (Sp::Hex | Sp::B64, _) => k
                .floor()
                .to_u32()
                .map_or(u128::MAX, |n| 256u128.checked_pow(n).unwrap_or(u128::MAX)),
            (Sp::Time, 0) if k.is_decimal() => OFFSETS,
            (Sp::Time, _) => u128::from(k.is_decimal()),
            _ => 1,
        }
    }

    fn of(v: &Val) -> Option<Sp> {
        Some(match v {
            Val::Real(_) => Sp::Real,
            Val::Float(_) => Sp::Float,
            Val::Double(_) => Sp::Double,
            Val::Bool(_) => Sp::Bool,
            Val::Hex(_) => Sp::Hex,
            Val::B64(_) => Sp::B64,
            Val::Time { .. } => Sp::Time,
            _ => return None,
        })
    }
}

/// The sort and ordering key of a value of an ordered space.
fn key(v: &Val) -> (usize, Rat) {
    match v {
        Val::Real(r) => (layer(r), r.clone()),
        Val::Float(b) if f32::from_bits(*b).is_nan() => (1, Rat::int(0)),
        Val::Float(b) => (0, Rat::int(f32_key(f32::from_bits(*b)))),
        Val::Double(b) if f64::from_bits(*b).is_nan() => (1, Rat::int(0)),
        Val::Double(b) => (0, Rat::int(f64_key(f64::from_bits(*b)))),
        Val::Bool(b) => (0, Rat::int(u8::from(*b))),
        Val::Hex(v) | Val::B64(v) => (0, Rat::int(v.len())),
        Val::Time { at, offset } => (usize::from(offset.is_none()), at.clone()),
        Val::Str(_) | Val::Lang(..) | Val::Uri(_) => {
            unreachable!("string values have no ordering key")
        }
    }
}

/// 0 for integers, 1 for other decimals, 2 for other rationals.
pub(crate) fn layer(r: &Rat) -> usize {
    if r.is_integer() {
        0
    } else if r.is_decimal() {
        1
    } else {
        2
    }
}

/// The position of a non-NaN float in IEEE total order, with `-0` just below `+0`.
pub(crate) fn f32_key(f: f32) -> i64 {
    let b = f.to_bits();
    if b >> 31 == 1 {
        -i64::from(b & 0x7FFF_FFFF) - 1
    } else {
        i64::from(b)
    }
}

/// [`f32_key`] for doubles.
pub(crate) fn f64_key(f: f64) -> i128 {
    let b = f.to_bits();
    if b >> 63 == 1 {
        -i128::from(b & 0x7FFF_FFFF_FFFF_FFFF) - 1
    } else {
        i128::from(b)
    }
}

/// One ordered space's part of a set.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Comp {
    pub(crate) sorts: Vec<Cuts>,
    /// Values in the set outside `sorts`.
    plus: BTreeSet<Val>,
    /// Values outside the set inside `sorts`.
    minus: BTreeSet<Val>,
}

impl Comp {
    fn empty(sp: Sp) -> Comp {
        Comp {
            sorts: vec![Cuts::empty(); sp.sorts()],
            plus: BTreeSet::new(),
            minus: BTreeSet::new(),
        }
    }

    fn full(sp: Sp) -> Comp {
        Comp {
            sorts: (0..sp.sorts()).map(|s| sp.universe(s)).collect(),
            plus: BTreeSet::new(),
            minus: BTreeSet::new(),
        }
    }

    fn in_sorts(sorts: &[Cuts], v: &Val) -> bool {
        let (s, k) = key(v);
        sorts[s].contains(&k)
    }

    fn contains(&self, v: &Val) -> bool {
        self.plus.contains(v) || (Comp::in_sorts(&self.sorts, v) && !self.minus.contains(v))
    }

    fn combine(
        &self,
        o: &Comp,
        op: fn(bool, bool) -> bool,
        cuts: fn(&Cuts, &Cuts) -> Cuts,
    ) -> Comp {
        let sorts: Vec<Cuts> = self
            .sorts
            .iter()
            .zip(&o.sorts)
            .map(|(a, b)| cuts(a, b))
            .collect();
        let points = self
            .plus
            .iter()
            .chain(&o.plus)
            .chain(&self.minus)
            .chain(&o.minus);
        let (mut plus, mut minus) = (BTreeSet::new(), BTreeSet::new());
        for v in points {
            let inside = op(self.contains(v), o.contains(v));
            match (inside, Comp::in_sorts(&sorts, v)) {
                (true, false) => plus.insert(v.clone()),
                (false, true) => minus.insert(v.clone()),
                _ => false,
            };
        }
        Comp { sorts, plus, minus }
    }

    fn not(&self, sp: Sp) -> Comp {
        let sorts = self
            .sorts
            .iter()
            .enumerate()
            .map(|(s, c)| sp.universe(s).and(&c.not()))
            .collect();
        Comp {
            sorts,
            plus: self.minus.clone(),
            minus: self.plus.clone(),
        }
    }

    fn is_empty(&self, sp: Sp) -> bool {
        if !self.plus.is_empty() {
            return false;
        }
        for (s, cuts) in self.sorts.iter().enumerate() {
            for seg in cuts.segments() {
                let removed: Vec<Rat> = self
                    .minus
                    .iter()
                    .map(key)
                    .filter(|(vs, k)| *vs == s && within(&seg, k))
                    .map(|(_, k)| k)
                    .collect();
                if !sp.discrete(s) {
                    match seg.point() {
                        // A dense interval with an interior holds infinitely many values.
                        None => return false,
                        Some(p) if sp.fiber(s, p) > removed.len() as u128 => return false,
                        Some(_) => {}
                    }
                    continue;
                }
                let (Some(lo), Some(hi)) = seg.integers() else {
                    return false;
                };
                if lo > hi {
                    continue;
                }
                // More keys than removed values leaves some key untouched.
                if &hi - &lo + 1 > BigInt::from(removed.len()) {
                    return false;
                }
                let mut k = lo;
                while k <= hi {
                    let kr = Rat::int(k.clone());
                    let gone = removed.iter().filter(|r| **r == kr).count() as u128;
                    if sp.fiber(s, &kr) > gone {
                        return false;
                    }
                    k += 1;
                }
            }
        }
        true
    }
}

/// Whether key `k` lies strictly between the segment's cuts.
fn within(seg: &Segment<'_>, k: &Rat) -> bool {
    let after_lo = seg.lo.is_none_or(|c| c.x < *k || (c.x == *k && !c.above));
    let before_hi = seg.hi.is_none_or(|c| c.x > *k || (c.x == *k && c.above));
    after_lo && before_hi
}

/// A set of data values over the whole datatype map.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Region {
    ord: Vec<Comp>,
    /// Plain strings and language-tagged pairs (`rdf:PlainLiteral`).
    text: F,
    /// `xsd:anyURI` values.
    uri: F,
    /// The whole `rdf:XMLLiteral` space (its values are never enumerated).
    xml: bool,
}

impl Region {
    pub(crate) fn empty() -> Region {
        Region {
            ord: SPACES.iter().map(|&s| Comp::empty(s)).collect(),
            text: F::Const(false),
            uri: F::Const(false),
            xml: false,
        }
    }

    pub(crate) fn full() -> Region {
        Region {
            ord: SPACES.iter().map(|&s| Comp::full(s)).collect(),
            text: F::Const(true),
            uri: F::Const(true),
            xml: true,
        }
    }

    /// The whole of one ordered space, restricted per sort by `sorts`.
    pub(crate) fn ordered(sp: Sp, sorts: Vec<Cuts>) -> Region {
        let mut r = Region::empty();
        let i = SPACES.iter().position(|&s| s == sp).expect("listed");
        r.ord[i].sorts = sorts
            .into_iter()
            .enumerate()
            .map(|(s, c)| sp.universe(s).and(&c))
            .collect();
        r
    }

    pub(crate) fn text(f: F) -> Region {
        Region {
            text: f,
            ..Region::empty()
        }
    }

    pub(crate) fn uri(f: F) -> Region {
        Region {
            uri: f,
            ..Region::empty()
        }
    }

    pub(crate) fn xml() -> Region {
        Region {
            xml: true,
            ..Region::empty()
        }
    }

    /// `{v}`.
    pub(crate) fn value(v: Val) -> Region {
        match &v {
            Val::Str(s) => Region::text(F::atom(strings::Atom::Is(strings::encode(s, None)))),
            Val::Lang(s, l) => {
                Region::text(F::atom(strings::Atom::Is(strings::encode(s, Some(l)))))
            }
            Val::Uri(s) => Region::uri(F::atom(strings::Atom::Is(strings::encode(s, None)))),
            _ => {
                let mut r = Region::empty();
                let sp = Sp::of(&v).expect("ordered");
                let i = SPACES.iter().position(|&s| s == sp).expect("listed");
                r.ord[i].plus.insert(v);
                r
            }
        }
    }

    pub(crate) fn and(&self, o: &Region) -> Region {
        Region {
            ord: self
                .ord
                .iter()
                .zip(&o.ord)
                .map(|(a, b)| a.combine(b, |x, y| x && y, Cuts::and))
                .collect(),
            text: self.text.clone().and(o.text.clone()),
            uri: self.uri.clone().and(o.uri.clone()),
            xml: self.xml && o.xml,
        }
    }

    pub(crate) fn or(&self, o: &Region) -> Region {
        Region {
            ord: self
                .ord
                .iter()
                .zip(&o.ord)
                .map(|(a, b)| a.combine(b, |x, y| x || y, Cuts::or))
                .collect(),
            text: self.text.clone().or(o.text.clone()),
            uri: self.uri.clone().or(o.uri.clone()),
            xml: self.xml || o.xml,
        }
    }

    pub(crate) fn not(&self) -> Region {
        Region {
            ord: self
                .ord
                .iter()
                .zip(SPACES)
                .map(|(c, sp)| c.not(sp))
                .collect(),
            text: self.text.clone().not(),
            uri: self.uri.clone().not(),
            xml: !self.xml,
        }
    }

    pub(crate) fn is_empty(&self) -> Tri {
        if self.xml || self.ord.iter().zip(SPACES).any(|(c, sp)| !c.is_empty(sp)) {
            return Tri::False;
        }
        match (
            strings::is_empty(&self.text, true),
            strings::is_empty(&self.uri, false),
        ) {
            (Tri::False, _) | (_, Tri::False) => Tri::False,
            (Tri::True, Tri::True) => Tri::True,
            _ => Tri::Unknown,
        }
    }

    pub(crate) fn contains(&self, v: &Val) -> Tri {
        match v {
            Val::Str(s) => strings::holds(&self.text, &strings::encode(s, None)),
            Val::Lang(s, l) => strings::holds(&self.text, &strings::encode(s, Some(l))),
            Val::Uri(s) => strings::holds(&self.uri, &strings::encode(s, None)),
            _ => {
                let sp = Sp::of(v).expect("ordered");
                let i = SPACES.iter().position(|&s| s == sp).expect("listed");
                Tri::from(self.ord[i].contains(v))
            }
        }
    }
}

/// `Rat` keys of the integers `lo..=hi` as cuts.
pub(crate) fn int_range(lo: Option<BigInt>, hi: Option<BigInt>) -> Cuts {
    let lo = lo.map_or_else(Cuts::full, |l| Cuts::at_least(Rat::int(l), true));
    let hi = hi.map_or_else(Cuts::full, |h| Cuts::at_most(Rat::int(h), true));
    lo.and(&hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_traits::Zero;

    fn real(n: i64) -> Val {
        Val::Real(Rat::int(n))
    }

    #[test]
    fn integer_ranges_count_their_points() {
        let ints = Region::ordered(
            Sp::Real,
            vec![
                int_range(Some(BigInt::zero()), Some(BigInt::from(2))),
                Cuts::empty(),
                Cuts::empty(),
                Cuts::empty(),
            ],
        );
        let three = Region::value(real(0))
            .or(&Region::value(real(1)))
            .or(&Region::value(real(2)));
        assert_eq!(ints.and(&three.not()).is_empty(), Tri::True);
        assert_eq!(
            ints.and(&Region::value(real(1)).not()).is_empty(),
            Tri::False
        );
        assert_eq!(three.and(&ints.not()).is_empty(), Tri::True);
    }

    #[test]
    fn float_zeros_are_distinct_values_and_nan_is_its_own_sort() {
        let zero = Region::value(Val::Float(0.0f32.to_bits()));
        let neg_zero = Region::value(Val::Float((-0.0f32).to_bits()));
        let both = Region::ordered(
            Sp::Float,
            vec![Cuts::closed(Rat::int(-1), Rat::int(0)), Cuts::empty()],
        );
        assert_eq!(both.and(&zero.not()).is_empty(), Tri::False);
        assert_eq!(both.and(&zero.or(&neg_zero).not()).is_empty(), Tri::True);
        let nan = Region::value(Val::Float(f32::NAN.to_bits()));
        assert_eq!(Region::full().and(&nan).is_empty(), Tri::False);
        assert_eq!(nan.contains(&Val::Float(f32::NAN.to_bits())), Tri::True);
    }

    #[test]
    fn complement_is_relative_to_the_whole_map() {
        let r = Region::value(real(1));
        assert_eq!(r.not().not(), r);
        assert_eq!(Region::full().not().is_empty(), Tri::True);
        assert_eq!(r.or(&r.not()).not().is_empty(), Tri::True);
    }
}
