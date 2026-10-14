//! Sets of data values, closed under union, intersection and complement, with exact
//! counting.
//!
//! The value-space families of the datatype map are pairwise disjoint, so a set is one
//! component per family. An ordered family is split into *sorts* that share one ordering
//! key: the reals into integers, the other decimals, the other rationals and the
//! irrationals; a floating-point family into its ordered values and NaN; time instants
//! into those with and those without a time zone. Such a component is a [`Cuts`] per sort
//! plus finitely many values added outside them or removed inside them, which is exactly
//! the shape the data-range constructors produce. String-like families are formulas over
//! automata ([`strings`]).

use super::cuts::{Cuts, Segment};
use super::rat::Rat;
use super::strings::{self, Space, F};
use crate::owl2dl::datatypes::Value;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::collections::BTreeSet;

/// The ordered families.
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

/// Offsets a value with a time zone can carry: -14:00 to +14:00, in minutes.
const OFFSETS: u128 = 2 * 14 * 60 + 1;

impl Sp {
    pub(crate) fn sorts(self) -> usize {
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

    /// Whether a sort's keys are integers, each a separate group of values.
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

    fn index(self) -> usize {
        SPACES.iter().position(|&s| s == self).expect("listed")
    }
}

/// The ordered family, sort and ordering key of a value; `None` for string-like values.
fn key(v: &Value) -> Option<(Sp, usize, Rat)> {
    Some(match v {
        Value::Real(r) => {
            let r = Rat::of(&r.0);
            (Sp::Real, layer(&r), r)
        }
        Value::Float(b) if f32::from_bits(*b).is_nan() => (Sp::Float, 1, Rat::int(0)),
        Value::Float(b) => (Sp::Float, 0, Rat::int(f32_key(f32::from_bits(*b)))),
        Value::Double(b) if f64::from_bits(*b).is_nan() => (Sp::Double, 1, Rat::int(0)),
        Value::Double(b) => (Sp::Double, 0, Rat::int(f64_key(f64::from_bits(*b)))),
        Value::Boolean(b) => (Sp::Bool, 0, Rat::int(u8::from(*b))),
        Value::HexBinary(v) => (Sp::Hex, 0, Rat::int(v.len())),
        Value::Base64Binary(v) => (Sp::B64, 0, Rat::int(v.len())),
        Value::DateTime(d) => (Sp::Time, usize::from(d.tz.is_none()), Rat::of(&d.seconds.0)),
        Value::String(_) | Value::LangString(..) | Value::AnyUri(_) | Value::XmlLiteral(_) => {
            return None
        }
    })
}

/// The string-like space and encoding of a value.
fn text(v: &Value) -> Option<(Space, Vec<u8>)> {
    Some(match v {
        Value::String(s) => (Space::Text, strings::encode(s, None)),
        Value::LangString(s, t) => (Space::Text, strings::encode(s, Some(t))),
        Value::AnyUri(s) => (Space::Uri, strings::encode(s, None)),
        Value::XmlLiteral(s) => (Space::Xml, strings::encode(s, None)),
        _ => return None,
    })
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

/// The position of a non-NaN float in IEEE total order, `-0` just below `+0`.
pub(crate) fn f32_key(f: f32) -> i128 {
    let b = f.to_bits();
    if b >> 31 == 1 {
        -i128::from(b & 0x7FFF_FFFF) - 1
    } else {
        i128::from(b)
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

/// One ordered family's part of a set.
#[derive(Clone, Debug, PartialEq)]
struct Comp {
    sorts: Vec<Cuts>,
    /// Values in the set outside `sorts`.
    plus: BTreeSet<Value>,
    /// Values outside the set inside `sorts`.
    minus: BTreeSet<Value>,
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
            ..Comp::empty(sp)
        }
    }

    fn in_sorts(sorts: &[Cuts], v: &Value) -> bool {
        let (_, s, k) = key(v).expect("ordered value");
        sorts[s].contains(&k)
    }

    fn contains(&self, v: &Value) -> bool {
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
        let (mut plus, mut minus) = (BTreeSet::new(), BTreeSet::new());
        for v in self
            .plus
            .iter()
            .chain(&o.plus)
            .chain(&self.minus)
            .chain(&o.minus)
        {
            match (
                op(self.contains(v), o.contains(v)),
                Comp::in_sorts(&sorts, v),
            ) {
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

    /// How many values the component holds, saturating (`u128::MAX` = infinitely many).
    fn count(&self, sp: Sp) -> u128 {
        let mut total = self.plus.len() as u128;
        for (s, cuts) in self.sorts.iter().enumerate() {
            for seg in cuts.segments() {
                total = total.saturating_add(segment_count(sp, s, &seg));
            }
        }
        if total == u128::MAX {
            total
        } else {
            total - self.minus.len() as u128
        }
    }
}

fn segment_count(sp: Sp, s: usize, seg: &Segment<'_>) -> u128 {
    if !sp.discrete(s) {
        // A dense interval with an interior holds infinitely many values.
        return seg.point().map_or(u128::MAX, |p| sp.fiber(s, p));
    }
    let (Some(lo), Some(hi)) = seg.integers() else {
        return u128::MAX;
    };
    if lo > hi {
        return 0;
    }
    if matches!(sp, Sp::Hex | Sp::B64) {
        // Octet sequences of each length: the sum saturates within a few terms.
        let mut n = 0u128;
        let mut k = lo;
        while k <= hi && n != u128::MAX {
            n = n.saturating_add(sp.fiber(s, &Rat::int(k.clone())));
            k += 1;
        }
        return n;
    }
    (hi - lo + BigInt::from(1)).to_u128().unwrap_or(u128::MAX)
}

/// A set of data values over the whole datatype map.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Region {
    ord: Vec<Comp>,
    text: F,
    uri: F,
    xml: F,
}

impl Region {
    pub(crate) fn empty() -> Region {
        let f = || F::Const(false);
        Region {
            ord: SPACES.iter().map(|&s| Comp::empty(s)).collect(),
            text: f(),
            uri: f(),
            xml: f(),
        }
    }

    pub(crate) fn full() -> Region {
        let t = || F::Const(true);
        Region {
            ord: SPACES.iter().map(|&s| Comp::full(s)).collect(),
            text: t(),
            uri: t(),
            xml: t(),
        }
    }

    /// Values of one ordered family, restricted per sort by `sorts`.
    pub(crate) fn ordered(sp: Sp, sorts: Vec<Cuts>) -> Region {
        let mut r = Region::empty();
        r.ord[sp.index()].sorts = sorts
            .into_iter()
            .enumerate()
            .map(|(s, c)| sp.universe(s).and(&c))
            .collect();
        r
    }

    /// Values of one string-like family satisfying `f`.
    pub(crate) fn strings(space: Space, f: F) -> Region {
        let mut r = Region::empty();
        match space {
            Space::Text => r.text = f,
            Space::Uri => r.uri = f,
            Space::Xml => r.xml = f,
        }
        r
    }

    /// `{v}`.
    pub(crate) fn value(v: &Value) -> Region {
        if let Some((space, enc)) = text(v) {
            return Region::strings(space, F::atom(strings::Atom::Is(enc)));
        }
        let mut r = Region::empty();
        let (sp, ..) = key(v).expect("ordered value");
        r.ord[sp.index()].plus.insert(v.clone());
        r
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
            xml: self.xml.clone().and(o.xml.clone()),
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
            xml: self.xml.clone().or(o.xml.clone()),
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
            xml: self.xml.clone().not(),
        }
    }

    /// How many values the set holds, exact below `need` and saturating at or above it
    /// (`u128::MAX` also stands for infinitely many). `None` = a string walk outgrew its
    /// budget before the answer was settled.
    pub(crate) fn count(&self, need: u128) -> Option<u128> {
        let mut total = 0u128;
        for (c, sp) in self.ord.iter().zip(SPACES) {
            total = total.saturating_add(c.count(sp));
        }
        let mut unknown = false;
        for (f, space) in [
            (&self.text, Space::Text),
            (&self.uri, Space::Uri),
            (&self.xml, Space::Xml),
        ] {
            if total >= need {
                return Some(total);
            }
            match strings::count(f, space, need - total) {
                Some(n) => total = total.saturating_add(n),
                None => unknown = true,
            }
        }
        if total >= need || !unknown {
            Some(total)
        } else {
            None
        }
    }

    /// Whether `v` is in the set; `None` when an automaton could not be built.
    pub(crate) fn contains(&self, v: &Value) -> Option<bool> {
        match text(v) {
            // A tag that is not well-formed BCP 47 puts the value outside the data domain.
            Some((Space::Text, enc)) if !strings::in_space(Space::Text, &enc)? => Some(false),
            Some((Space::Text, enc)) => strings::holds(&self.text, &enc),
            Some((Space::Uri, enc)) => strings::holds(&self.uri, &enc),
            Some((Space::Xml, enc)) => strings::holds(&self.xml, &enc),
            None => {
                let (sp, ..) = key(v).expect("ordered value");
                Some(self.ord[sp.index()].contains(v))
            }
        }
    }
}

/// The integers `lo..=hi` (`None` = unbounded on that side).
pub(crate) fn int_range(lo: Option<i128>, hi: Option<i128>) -> Cuts {
    let lo = lo.map_or_else(Cuts::full, |l| Cuts::at_least(Rat::int(l), true));
    let hi = hi.map_or_else(Cuts::full, |h| Cuts::at_most(Rat::int(h), true));
    lo.and(&hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owl2dl::datatypes::{Rational, RationalKey};

    fn real(n: i128) -> Value {
        Value::Real(RationalKey(Rational::int(n)))
    }

    fn ints(lo: i128, hi: i128) -> Region {
        Region::ordered(
            Sp::Real,
            vec![
                int_range(Some(lo), Some(hi)),
                Cuts::empty(),
                Cuts::empty(),
                Cuts::empty(),
            ],
        )
    }

    #[test]
    fn integer_ranges_count_their_points() {
        let three = Region::value(&real(0))
            .or(&Region::value(&real(1)))
            .or(&Region::value(&real(2)));
        assert_eq!(ints(0, 2).and(&three.not()).count(1), Some(0));
        assert_eq!(ints(0, 2).count(u128::MAX), Some(3));
        assert_eq!(
            ints(0, 2)
                .and(&Region::value(&real(1)).not())
                .count(u128::MAX),
            Some(2)
        );
        assert_eq!(three.and(&ints(0, 2).not()).count(1), Some(0));
        assert_eq!(
            ints(0, 2).or(&Region::value(&real(5))).count(u128::MAX),
            Some(4)
        );
    }

    #[test]
    fn float_zeros_are_distinct_values_and_nan_is_its_own_sort() {
        let zero = Region::value(&Value::Float(0.0f32.to_bits()));
        let neg_zero = Region::value(&Value::Float((-0.0f32).to_bits()));
        let both = Region::ordered(
            Sp::Float,
            vec![Cuts::closed(Rat::int(-1), Rat::int(0)), Cuts::empty()],
        );
        assert_eq!(both.count(u128::MAX), Some(2));
        assert_eq!(both.and(&zero.or(&neg_zero).not()).count(1), Some(0));
        let floats = Region::ordered(Sp::Float, vec![Cuts::full(), Cuts::full()]);
        // Every bit pattern but the NaNs, plus the one NaN value.
        assert_eq!(
            floats.count(u128::MAX),
            Some((1u128 << 32) - (2 * ((1 << 23) - 1)) + 1)
        );
    }

    #[test]
    fn binary_lengths_count_octet_sequences() {
        let up_to_one = Region::ordered(Sp::Hex, vec![int_range(Some(0), Some(1))]);
        assert_eq!(up_to_one.count(u128::MAX), Some(257));
    }

    #[test]
    fn complement_is_relative_to_the_whole_map() {
        let r = Region::value(&real(1));
        assert_eq!(r.not().not(), r);
        assert_eq!(Region::full().not().count(1), Some(0));
        assert_eq!(r.or(&r.not()).not().count(1), Some(0));
    }
}
