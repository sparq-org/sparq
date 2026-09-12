//! The XSD numeric value tower — [`Num`], [`Dec`], and [`as_numeric`].
//!
//! This is the value-space machinery a D-entailment / RIF-builtin reasoner needs and that
//! the SPARQL engine uses for FILTER / BIND arithmetic (`research/shared-eval-substrate.md`
//! §2.1). It classifies a numeric literal into the XPath promotion tower — `xsd:integer` →
//! `xsd:decimal` → `xsd:float` → `xsd:double` — keeping the *exact* representation (`i64`
//! integers and a fixed-point [`Dec`]) so a high-precision decimal threshold is not silently
//! flattened to `f64` (the soundness note in research record §5).
//!
//! # Provenance (sq-ev41x, epic sq-qonbz)
//!
//! This module is the engine's id-level numeric value tower, **moved here verbatim** from
//! `sparq-engine::exec` (the private `Num` / `Dec` / `ArithOp` / `RoundMode` and their
//! helpers `split_decimal` / `parse_xsd_f64` / `parse_xsd_f32` / `apply_f64` /
//! `fmt_xsd_double`). The move is **behaviour-neutral**: the engine now calls
//! `sparq_substrate::numeric::{Num, Dec, as_numeric, …}` and the W3C SPARQL conformance
//! floor + the ORDER BY / FILTER / numeric tests are bit-identical to pre-move.
//!
//! The earlier scaffold (sq-fmprw, #1290) carried a *placeholder* `as_numeric` that used
//! `Dec::parse` / `v.parse::<f32/f64>()`; that PLACEHOLDER is now replaced by the engine's
//! EXACT lexical path — `Dec::parse_lexical` (scale-preserving) for `xsd:decimal` and
//! `parse_xsd_f32` / `parse_xsd_f64` (which handle the XSD `INF` / `-INF` / `NaN` /
//! exponent spellings) for `xsd:float` / `xsd:double` — so classification is bit-identical
//! to what the engine did before the move.
//!
//! # Zero-overhead intent
//!
//! Every item here is monomorphic over the concrete numeric tiers (`i64` / `i128` /
//! `f32` / `f64`); there is NO `Box<dyn>` / `&dyn` / vtable anywhere. The accessors and
//! arithmetic carry `#[inline]` so cross-crate inlining (with the workspace LTO profile)
//! keeps the engine's FILTER / BIND / ORDER BY hot loops identical to the pre-move codegen.

use std::cmp::Ordering;

/// An EXACT fixed-point decimal: `mant * 10^-scale`. Used to evaluate `+ - *` on
/// integer / `xsd:decimal` operands without f64 rounding (`0.1 + 0.2` is exactly `0.3`),
/// which the f64 arithmetic path gets wrong. Within `i128` range; overflow → `None` →
/// the caller falls back to f64. Division and `xsd:double`/`float` stay f64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dec {
    /// The scaled integer mantissa.
    pub mant: i128,
    /// The number of fractional decimal digits: the value is `mant * 10^-scale`.
    pub scale: u32,
}

impl Dec {
    /// Parses an integer / decimal lexical (`[+-]?digits(.digits)?`), `None` otherwise
    /// (including `i128` mantissa overflow). Normalises insignificant zeros away (see
    /// [`split_decimal`]); use [`Dec::parse_lexical`] when the written scale must survive.
    #[inline]
    pub fn parse(s: &str) -> Option<Dec> {
        let (neg, int, frac) = split_decimal(s)?;
        let scale = frac.len() as u32;
        let mut mag: i128 = 0;
        for &ch in int.as_bytes().iter().chain(frac.as_bytes()) {
            mag = mag.checked_mul(10)?.checked_add((ch - b'0') as i128)?;
        }
        Some(Dec { mant: if neg { -mag } else { mag }, scale })
    }

    /// Both mantissas scaled to the common (max) scale, or `None` on overflow.
    #[inline]
    fn align(self, o: Dec) -> Option<(i128, i128)> {
        let scale = self.scale.max(o.scale);
        let a = self.mant.checked_mul(10i128.checked_pow(scale - self.scale)?)?;
        let b = o.mant.checked_mul(10i128.checked_pow(scale - o.scale)?)?;
        Some((a, b))
    }

    /// EXACT addition, or `None` on overflow (the caller falls back to f64).
    #[inline]
    pub fn checked_add(self, o: Dec) -> Option<Dec> {
        let (a, b) = self.align(o)?;
        Some(Dec { mant: a.checked_add(b)?, scale: self.scale.max(o.scale) })
    }
    /// EXACT subtraction, or `None` on overflow (the caller falls back to f64).
    #[inline]
    pub fn checked_sub(self, o: Dec) -> Option<Dec> {
        let (a, b) = self.align(o)?;
        Some(Dec { mant: a.checked_sub(b)?, scale: self.scale.max(o.scale) })
    }
    /// EXACT multiplication, or `None` on overflow (the caller falls back to f64).
    #[inline]
    pub fn checked_mul(self, o: Dec) -> Option<Dec> {
        Some(Dec { mant: self.mant.checked_mul(o.mant)?, scale: self.scale.checked_add(o.scale)? })
    }
    /// EXACT ordering of two decimals, or `None` on a scale-alignment overflow.
    // Deliberately NOT `Ord::cmp`: this is the FALLIBLE total order (it returns
    // `Option<Ordering>` so a scale-alignment overflow can fall back to f64), so it cannot
    // implement the infallible `Ord`/`PartialOrd` trait method. Kept as `cmp` so the engine
    // call sites are byte-identical to pre-move (this is a verbatim code-move). [OPUS-4.8]
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn cmp(self, o: Dec) -> Option<Ordering> {
        let (a, b) = self.align(o)?;
        Some(a.cmp(&b))
    }

    /// Parses an integer / decimal lexical PRESERVING the written scale ("1.0" keeps
    /// scale 1), unlike [`Dec::parse`] which normalises trailing fraction zeros away.
    /// The scale is what XSD-canonical serialisation of decimal arithmetic preserves
    /// (`1.0 + 2` is `"3.0"`), so typed values must carry it.
    #[inline]
    pub fn parse_lexical(s: &str) -> Option<Dec> {
        let s = s.trim();
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (int, frac) = s.split_once('.').unwrap_or((s, ""));
        if (int.is_empty() && frac.is_empty()) || !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut mag: i128 = 0;
        for &ch in int.as_bytes().iter().chain(frac.as_bytes()) {
            mag = mag.checked_mul(10)?.checked_add((ch - b'0') as i128)?;
        }
        Some(Dec { mant: if neg { -mag } else { mag }, scale: frac.len() as u32 })
    }

    /// EXACT decimal division. The result's scale is the SMALLEST `s >= 1` at which the
    /// quotient terminates (`0 / 2 = "0.0"`, `11.1 / 5 = "2.22"`); a non-terminating
    /// quotient is rounded half-up at scale 18. `None` on overflow (caller falls back
    /// to double); the caller must reject a zero divisor first (type error).
    #[inline]
    pub fn checked_div(self, o: Dec) -> Option<Dec> {
        debug_assert!(o.mant != 0);
        let neg = (self.mant < 0) != (o.mant < 0);
        let n0 = self.mant.unsigned_abs();
        let d = o.mant.unsigned_abs();
        // mant(s) = n0 * 10^(s + o.scale - self.scale) / d
        let num_den = |s: u32| -> Option<(u128, u128)> {
            let e = s as i32 + o.scale as i32 - self.scale as i32;
            if e >= 0 {
                Some((n0.checked_mul(10u128.checked_pow(e as u32)?)?, d))
            } else {
                Some((n0, d.checked_mul(10u128.checked_pow((-e) as u32)?)?))
            }
        };
        const MAX_SCALE: u32 = 18;
        for s in 1..=MAX_SCALE {
            let (num, den) = num_den(s)?;
            if num % den == 0 {
                let mant = i128::try_from(num / den).ok()?;
                return Some(Dec { mant: if neg { -mant } else { mant }, scale: s });
            }
        }
        // Non-terminating: round half-up at the max scale.
        let (num, den) = num_den(MAX_SCALE)?;
        let q = num / den + u128::from(num % den * 2 >= den);
        let mant = i128::try_from(q).ok()?;
        Some(Dec { mant: if neg { -mant } else { mant }, scale: MAX_SCALE })
    }

    /// Rounds to an integer-valued decimal (scale 0), preserving the decimal TYPE
    /// (`CEIL("2.5"^^xsd:decimal)` is `"3"^^xsd:decimal`).
    #[inline]
    pub fn round_to_int(self, mode: RoundMode) -> Dec {
        if self.scale == 0 || self.mant == 0 {
            return Dec { mant: self.mant, scale: 0 };
        }
        // [OPUS-4.8] `10i128.pow(self.scale)` overflows (debug panic / release wrap) for any
        // valid decimal whose scale is >= 39 (10^39 > i128::MAX). When the power exceeds i128
        // the integer part is necessarily 0 (|mant| < i128::MAX < 10^scale), so |value| < 1 and
        // also < 0.5 (2*i128::MAX < 10^39 <= 10^scale), making the rounded result obvious from
        // the sign alone — derive it directly instead of constructing the overflowing power.
        let mant = match 10i128.checked_pow(self.scale) {
            Some(p) => {
                let q = self.mant.div_euclid(p);
                let r = self.mant.rem_euclid(p); // 0..p
                match mode {
                    RoundMode::Floor => q,
                    RoundMode::Ceil => q + i128::from(r > 0),
                    RoundMode::HalfUp => q + i128::from(r * 2 >= p),
                }
            }
            None => match mode {
                // |value| < 1: floor of a tiny positive is 0, of a tiny negative is -1.
                RoundMode::Floor => -i128::from(self.mant < 0),
                // ceil of a tiny positive is 1, of a tiny negative is 0.
                RoundMode::Ceil => i128::from(self.mant > 0),
                // |value| < 0.5 always at this scale, so half-up rounds to 0.
                RoundMode::HalfUp => 0,
            },
        };
        Dec { mant, scale: 0 }
    }

    /// The value as an `f64` (lossy for mantissas beyond `f64`'s exact integer range — the
    /// exact value lives in `mant`/`scale`). Used by the LENIENT order / arithmetic fallback.
    #[inline]
    pub fn f64(self) -> f64 {
        self.mant as f64 / 10f64.powi(self.scale as i32)
    }

    /// The value as an `f32`, rounded to single precision **exactly once** — the
    /// single-precision half of [`Dec::f64`], used by [`Num::f32`] for XPath `xs:float`
    /// promotion.
    ///
    /// Deliberately NOT `self.f64() as f32`: that rounds TWICE (once into the `f64`
    /// quotient, once in the narrowing) and two roundings are not the correctly-rounded
    /// single conversion. Witness (exact rational arithmetic):
    /// `Dec { mant: 46_116_862_933_052_948_481, scale: 1 }` — i.e. `4611686293305294848.1` —
    /// has correctly-rounded `f32` bits `0x5E80_0001`, but `f64()` lands exactly on the
    /// midpoint `0x5E80_0000`/`0x5E80_0001`, which ties-to-even narrows DOWN to
    /// `0x5E80_0000`. [OPUS-5] issue #3796
    ///
    /// - **scale 0** (the representation an `xsd:integer` beyond `i64` takes) is an exact
    ///   integer, and `i128 as f32` is a single correctly-rounded conversion.
    /// - **scale > 0**: [`Dec::lexical`] writes `mant * 10^-scale` EXACTLY, and Rust's
    ///   decimal → binary parser is correctly rounded, so the round-trip through that exact
    ///   lexical is one rounding of the true value. This path allocates, so it is kept off
    ///   the scale-0 fast path; float promotion of a scaled decimal is a cold seam.
    #[inline]
    pub fn f32(self) -> f32 {
        if self.scale == 0 {
            return self.mant as f32;
        }
        match self.lexical().parse::<f32>() {
            Ok(f) => f,
            // Unreachable: `Dec::lexical` always writes `[-]digits[.digits]`, which
            // `f32::from_str` accepts (overflow saturates to INF rather than failing).
            // Fall back to the f64 route rather than panic on an arithmetic path.
            Err(_) => self.f64() as f32,
        }
    }

    /// The plain (never exponent) decimal lexical at this value's scale: scale 0 prints
    /// as an integer ("3"); otherwise exactly `scale` fraction digits ("3.0", "0.05").
    pub fn lexical(self) -> String {
        let mag = self.mant.unsigned_abs().to_string();
        let s = self.scale as usize;
        let mut out = String::with_capacity(mag.len() + s + 2);
        if self.mant < 0 {
            out.push('-');
        }
        if s == 0 {
            out.push_str(&mag);
            return out;
        }
        if mag.len() > s {
            out.push_str(&mag[..mag.len() - s]);
        } else {
            out.push('0');
        }
        out.push('.');
        for _ in mag.len()..s {
            out.push('0');
        }
        if mag.len() > s {
            out.push_str(&mag[mag.len() - s..]);
        } else {
            out.push_str(&mag);
        }
        out
    }
}

/// Splits a decimal lexical into (negative, integer-digits, fraction-digits), normalised
/// (no leading zeros on the integer part, no trailing zeros on the fraction). `None` if
/// the lexical is not digits with at most one `.`. Shared with the engine's exact
/// decimal-string comparison and significant-digit count.
#[inline]
pub fn split_decimal(s: &str) -> Option<(bool, &str, &str)> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if (int.is_empty() && frac.is_empty()) || !int.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((neg, int.trim_start_matches('0'), frac.trim_end_matches('0')))
}

/// Exact comparison of two plain decimal lexicals (`[+-]?digits(.digits)?`) of ANY
/// length — pure string arithmetic, no `f64` and no `i128` bound. `None` if either
/// lexical is not a well-formed decimal. Signed zeros (`-0`, `-0.0`) compare equal to
/// zero. This is the string-exact tier [`Num::cmp_total`]'s f64-tie disambiguation
/// rides on. [FABLE-5] sq-wjl8i
pub fn cmp_plain_decimal(a: &str, b: &str) -> Option<Ordering> {
    let (na, ia, fa) = split_decimal(a)?;
    let (nb, ib, fb) = split_decimal(b)?;
    let a_zero = ia.is_empty() && fa.is_empty();
    let b_zero = ib.is_empty() && fb.is_empty();
    if a_zero && b_zero {
        return Some(Ordering::Equal);
    }
    // Magnitude: longer (normalised) integer part wins, then integer digits, then
    // fraction digits with implicit trailing-zero padding.
    let mag = ia.len().cmp(&ib.len()).then_with(|| ia.cmp(ib)).then_with(|| {
        let n = fa.len().max(fb.len());
        (0..n)
            .map(|i| {
                (
                    fa.as_bytes().get(i).copied().unwrap_or(b'0'),
                    fb.as_bytes().get(i).copied().unwrap_or(b'0'),
                )
            })
            .find_map(|(x, y)| if x != y { Some(x.cmp(&y)) } else { None })
            .unwrap_or(Ordering::Equal)
    });
    let neg_a = na && !a_zero;
    let neg_b = nb && !b_zero;
    Some(match (neg_a, neg_b) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => mag,
        (true, true) => mag.reverse(),
    })
}

/// The EXACT plain-decimal expansion of a finite `f64` (every finite `f64` is an exact
/// decimal rational; the longest needs 1074 fraction digits). `None` for `NaN` / `±INF`.
/// Cold-path helper for the f64-tie disambiguation in [`Num::cmp_total`] and the
/// engine's numeric sort cells. [FABLE-5] sq-wjl8i
pub fn f64_exact_decimal(f: f64) -> Option<String> {
    if !f.is_finite() {
        return None;
    }
    // With precision >= the maximum exact digit count, Rust's correctly-rounded
    // formatting IS the exact expansion.
    Some(format!("{:.1074}", f))
}

/// Exact order of an exact fixed-point decimal against a (non-NaN) `f64`'s exact
/// rational value: the decimal's lexical against the double's exact decimal expansion,
/// pure string arithmetic throughout. Deliberately NO `f64` fast path: `Dec::f64` is a
/// two-rounding image (mantissa conversion then a division), so a strict `f64` verdict
/// here is not boundary-trustworthy — and this helper is only reached on the cold
/// f64-tie path of the total order. [FABLE-5] sq-wjl8i
fn cmp_dec_f64(d: Dec, f: f64) -> Ordering {
    if f == f64::INFINITY {
        return Ordering::Less;
    }
    if f == f64::NEG_INFINITY {
        return Ordering::Greater;
    }
    f64_exact_decimal(f)
        .and_then(|exp| cmp_plain_decimal(&d.lexical(), &exp))
        .unwrap_or(Ordering::Equal)
}

/// The arithmetic operator for [`Num::binop`]: `+ - * /` under XPath operand promotion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArithOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/` (integer / integer is DECIMAL division per SPARQL).
    Div,
}

/// The rounding direction for [`Dec::round_to_int`] / [`Num::ceil`] / [`Num::floor`] /
/// [`Num::round`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoundMode {
    /// Round towards positive infinity.
    Ceil,
    /// Round towards negative infinity.
    Floor,
    /// Round half towards positive infinity (XPath `fn:round`).
    HalfUp,
}

/// A COMPUTED numeric value carrying its XSD type, implementing the SPARQL/XPath
/// operand-type-promotion tower: integer < decimal < float < double. Arithmetic
/// promotes both operands to the greater type; integer and decimal arithmetic is
/// EXACT (i64 / fixed-point [`Dec`]), falling back to double only on overflow.
/// Serialisation (see [`Num::lexical`]) is the XSD canonical form of the type.
#[derive(Clone, Copy, Debug)]
pub enum Num {
    /// `xsd:integer` (and its sub-types) within `i64`.
    Int(i64),
    /// `xsd:decimal`, or an `xsd:integer` beyond `i64`, as an exact fixed-point [`Dec`].
    Dec(Dec),
    /// `xsd:float` (single precision).
    Float(f32),
    /// `xsd:double`.
    Double(f64),
}

impl Num {
    /// Promotion rank in the XPath numeric tower (`Int` < `Dec` < `Float` < `Double`). A
    /// binary numeric op promotes both operands to the higher rank.
    #[inline]
    pub fn rank(self) -> u8 {
        match self {
            Num::Int(_) => 0,
            Num::Dec(_) => 1,
            Num::Float(_) => 2,
            Num::Double(_) => 3,
        }
    }

    /// The value as an `f64` (lossy for the exact tiers' values beyond `f64`'s exact
    /// range — the exact value lives in the `Int` / `Dec` payload).
    #[inline]
    pub fn f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Dec(d) => d.f64(),
            Num::Float(f) => f as f64,
            Num::Double(d) => d,
        }
    }

    /// The value as an `f32` — the XPath **`xs:float` promotion** of this value, rounded to
    /// single precision **exactly once**.
    ///
    /// This is the single shared promotion helper [`Num::binop`]'s float tier rides on (and
    /// the substrate `overhead` kernel's inline replica of it), so the two cannot drift.
    /// Deliberately NOT `self.f64() as f32`: routing an exact operand through `f64` first
    /// rounds TWICE, and double rounding is not the correctly-rounded single conversion
    /// XPath/XSD numeric promotion requires. Witness (verified by exact rational
    /// arithmetic): the `i64` value `4_611_686_293_305_294_849` has correctly-rounded `f32`
    /// bits `0x5E80_0001`, but `as f64 as f32` yields `0x5E80_0000` — one ULP low. So
    /// `SUM(?int, "0.0"^^xsd:float)` and `AVG` over a mixed integer/float column returned a
    /// wrong float for integer magnitudes above 2^53 before this. [OPUS-5] issue #3796
    ///
    /// Per tier: `Int` and `Dec` convert directly (see [`Dec::f32`]); `Float` is already the
    /// value; `Double` narrows once (`f64 as f32` is itself correctly rounded).
    #[inline]
    pub fn f32(self) -> f32 {
        match self {
            Num::Int(i) => i as f32,
            Num::Dec(d) => d.f32(),
            Num::Float(f) => f,
            Num::Double(d) => d as f32,
        }
    }

    /// As an exact [`Dec`] for the exact tiers (`Int` / `Dec`); `None` for `Float` /
    /// `Double` (which are not exactly representable as a fixed-point decimal).
    #[inline]
    pub fn to_dec(self) -> Option<Dec> {
        match self {
            Num::Int(i) => Some(Dec { mant: i as i128, scale: 0 }),
            Num::Dec(d) => Some(d),
            _ => None,
        }
    }

    /// The TOTAL order over numeric values used by the SPARQL `ORDER BY` extension
    /// (the `CompareTerm::exact_cmp` hook of `sparq_substrate::compare`) — [FABLE-5]
    /// sq-wjl8i. Unlike the relational comparison (XPath `op:numeric-*`, where a
    /// float/double operand PROMOTES the pair to a possibly-collapsing `f64` and `NaN`
    /// is incomparable), this is the **exact-rational** order over the values,
    /// totalised:
    ///
    /// - `NaN` sorts FIRST (before `-INF`), and `NaN == NaN` — the fixed position that
    ///   makes the order total (the choice XPath 3.1's `fn:sort` makes: "NaN least");
    /// - an exact-tier operand (`Int` / `Dec`) against a float/double compares against
    ///   the float/double's EXACT rational value (every finite `f64` is one), so the
    ///   2^53 collapse cannot make two distinct values tie with a third
    ///   (the machine-checked intransitivity witness of bead sq-wjl8i);
    /// - the order REFINES the promoted (`f64`) comparison: `f64` rounding is monotonic,
    ///   so every strict promoted verdict is preserved — only promoted TIES are refined.
    ///
    /// Honest boundary: two `Dec`s whose scale alignment overflows `i128` fall back to
    /// the `f64` image (as `Dec::cmp` documents); values whose lexicals exceed the
    /// `i128` tower never reach this type (`as_numeric` classifies them out).
    pub fn cmp_total(self, o: Num) -> Ordering {
        let (fa, fb) = (self.f64(), o.f64());
        match (fa.is_nan(), fb.is_nan()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {}
        }
        match (self.to_dec(), o.to_dec()) {
            (Some(a), Some(b)) => a
                .cmp(b)
                .unwrap_or_else(|| fa.partial_cmp(&fb).unwrap_or(Ordering::Equal)),
            (Some(a), None) => cmp_dec_f64(a, fb),
            (None, Some(b)) => cmp_dec_f64(b, fa).reverse(),
            // Both inexact: the value IS the f64 (an `f32` widens exactly); both are
            // non-NaN here, so `partial_cmp` always decides.
            (None, None) => fa.partial_cmp(&fb).unwrap_or(Ordering::Equal),
        }
    }

    /// The **relational** numeric comparison used by the SPARQL `<`/`>`/`=` operators
    /// and by `MIN`/`MAX` — XPath `op:numeric-less-than` / `op:numeric-equal` semantics.
    ///
    /// Unlike [`cmp_total`](Self::cmp_total), this comparison is **partial**: `NaN`
    /// produces `None` (a SPARQL type error), and `+0.0` equals `-0.0`. Returns `None` when
    /// either operand is `NaN`, matching the engine's `values_equal`/`value_compare_strict`
    /// path (SPARQL type error on NaN).
    ///
    /// # Which tier a mixed pair is compared in
    ///
    /// XPath promotes the operands of a comparison to the **LEAST common type** in the
    /// hierarchy `xs:integer -> xs:decimal -> xs:float -> xs:double` (F&O *Operator
    /// Mapping*: if one operand is `xs:double` the other becomes double; OTHERWISE if one
    /// is `xs:float` the other becomes **float**). So:
    ///
    /// - both `Int`/`Dec` — exact value via [`Dec::cmp`], no floating promotion at all;
    /// - either operand `Double` — promote the pair to `f64`;
    /// - otherwise an operand is `Float` — promote the pair to **`f32`**, through
    ///   [`Num::f32`](Self::f32) so the promotion is a SINGLE correctly-rounded conversion.
    ///
    /// [OPUS-5] This last case used to fall through to `f64` like everything else, and the
    /// doc here asserted that was correct. It is not: `xs:double` is only the common type
    /// when an operand really IS a double. The error is invisible below 2^53 and appears
    /// above it — `Num::Int(4611686293305294849)` versus the `f32` `0x5E80_0001` (the
    /// correctly-rounded promotion of that very integer, = 2^62 + 2^39) compared `Less`
    /// where XPath requires `Equal`. Pinned by
    /// `tests::cmp_relational_integer_and_decimal_vs_float_compare_in_the_float_tier`.
    ///
    /// Within the `f64` tier the 2^53 collapse for a large `Int` operand IS spec-correct
    /// (integer -> double is a single correctly-rounded conversion), so that is an accepted
    /// consequence of XPath, not a bug.
    ///
    /// # When to use `cmp_total` vs `cmp_relational`
    ///
    /// - **`ORDER BY`, `MIN`/`MAX` (for the sort tie), `sort_ids`** — use `cmp_total`:
    ///   NaN must be positioned somewhere to give a total order.
    /// - **`<`, `>`, `=`, `!=` FILTER expressions and value-space equality**
    ///   (D-entailment, RIF `pred:numeric-equal`) — use `cmp_relational`: NaN is an error.
    ///
    /// [OPUS-4.8] sq-v5evr — the value-space equality/relational-compare hoist.
    #[inline]
    pub fn cmp_relational(self, o: Num) -> Option<Ordering> {
        // Exact tier: both Int/Dec → exact Dec comparison (no f64 promotion).
        if let (Some(x), Some(y)) = (self.to_dec(), o.to_dec()) {
            if let Some(ord) = x.cmp(y) {
                return Some(ord);
            }
        }
        // FLOAT tier: no operand is a Double, but one is a Float, so XPath's least common
        // type is `xs:float` — promote BOTH through `Num::f32` (a single correctly-rounded
        // conversion) and decide there. Comparing this pair as `f64` is a different, wrong
        // answer above 2^53. [OPUS-5] issue #3796
        if self.rank().max(o.rank()) == 2 {
            return self.f32().partial_cmp(&o.f32());
        }
        // DOUBLE tier (or an exact pair whose `Dec::cmp` overflowed): f64 promotion.
        // NaN → None (SPARQL type error).
        self.f64().partial_cmp(&o.f64())
    }

    /// `op` under XPath operand promotion. `None` is a SPARQL type error (exact-type
    /// division by zero); exact-arithmetic overflow falls back to double, mirroring
    /// the engine's previous f64 behaviour.
    #[inline]
    pub fn binop(self, o: Num, op: ArithOp) -> Option<Num> {
        let rank = self.rank().max(o.rank());
        if rank == 3 {
            return Some(Num::Double(apply_f64(self.f64(), o.f64(), op)));
        }
        if rank == 2 {
            // Promote each operand with a SINGLE rounding (`Num::f32`, NOT `f64() as f32`)
            // and evaluate in `f32`, so no value on the `xsd:float` tier is rounded twice.
            // [OPUS-5] issue #3796
            return Some(Num::Float(apply_f32(self.f32(), o.f32(), op)));
        }
        // Exact tier: integer / decimal.
        let (a, b) = (self.to_dec()?, o.to_dec()?);
        if op == ArithOp::Div {
            // xsd:integer / xsd:integer is DECIMAL division per SPARQL; exact-type
            // division by zero is a type error (not INF/NaN).
            if b.mant == 0 {
                return None;
            }
            return match a.checked_div(b) {
                Some(d) => Some(Num::Dec(d)),
                None => Some(Num::Double(self.f64() / o.f64())),
            };
        }
        if rank == 0 {
            // integer op integer -> integer (i64; on overflow fall back to double).
            let (x, y) = (match self { Num::Int(i) => i, _ => unreachable!() }, match o { Num::Int(i) => i, _ => unreachable!() });
            let r = match op {
                ArithOp::Add => x.checked_add(y),
                ArithOp::Sub => x.checked_sub(y),
                ArithOp::Mul => x.checked_mul(y),
                ArithOp::Div => unreachable!(),
            };
            return Some(match r {
                Some(i) => Num::Int(i),
                None => Num::Double(apply_f64(x as f64, y as f64, op)),
            });
        }
        let r = match op {
            ArithOp::Add => a.checked_add(b),
            ArithOp::Sub => a.checked_sub(b),
            ArithOp::Mul => a.checked_mul(b),
            ArithOp::Div => unreachable!(),
        };
        Some(match r {
            Some(d) => Num::Dec(d),
            None => Num::Double(apply_f64(self.f64(), o.f64(), op)),
        })
    }

    /// Unary negation, preserving the datatype (overflow falls back to double).
    // Deliberately NOT `Neg::neg`: SPARQL unary minus promotes to `Double` on `i64`/`Dec`
    // overflow (it never panics/wraps), so the typed result differs from a plain `Neg`. Kept
    // as `neg` so the engine call site is byte-identical to pre-move (verbatim move). [OPUS-4.8]
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn neg(self) -> Num {
        match self {
            Num::Int(i) => i.checked_neg().map(Num::Int).unwrap_or(Num::Double(-(i as f64))),
            Num::Dec(d) => d.mant.checked_neg().map(|m| Num::Dec(Dec { mant: m, scale: d.scale })).unwrap_or(Num::Double(-d.f64())),
            Num::Float(f) => Num::Float(-f),
            Num::Double(d) => Num::Double(-d),
        }
    }

    /// Absolute value, preserving the datatype (overflow falls back to double).
    #[inline]
    pub fn abs(self) -> Num {
        match self {
            Num::Int(i) => i.checked_abs().map(Num::Int).unwrap_or(Num::Double((i as f64).abs())),
            Num::Dec(d) => d.mant.checked_abs().map(|m| Num::Dec(Dec { mant: m, scale: d.scale })).unwrap_or(Num::Double(d.f64().abs())),
            Num::Float(f) => Num::Float(f.abs()),
            Num::Double(d) => Num::Double(d.abs()),
        }
    }

    /// XPath `fn:ceiling`, preserving the datatype.
    #[inline]
    pub fn ceil(self) -> Num {
        match self {
            Num::Int(_) => self,
            Num::Dec(d) => Num::Dec(d.round_to_int(RoundMode::Ceil)),
            Num::Float(f) => Num::Float(f.ceil()),
            Num::Double(d) => Num::Double(d.ceil()),
        }
    }

    /// XPath `fn:floor`, preserving the datatype.
    #[inline]
    pub fn floor(self) -> Num {
        match self {
            Num::Int(_) => self,
            Num::Dec(d) => Num::Dec(d.round_to_int(RoundMode::Floor)),
            Num::Float(f) => Num::Float(f.floor()),
            Num::Double(d) => Num::Double(d.floor()),
        }
    }

    /// XPath fn:round — round half towards POSITIVE INFINITY (so round(-2.5) = -2),
    /// preserving the argument's datatype.
    ///
    /// The float tiers use `round_half_to_pos_inf`, NOT the naive `(x + 0.5).floor()`:
    /// the `x + 0.5` addition double-rounds, so a value just below half such as
    /// `0.49999999999999994` would wrongly round up to `1` instead of `0`. [OPUS-4.8]
    #[inline]
    pub fn round(self) -> Num {
        match self {
            Num::Int(_) => self,
            Num::Dec(d) => Num::Dec(d.round_to_int(RoundMode::HalfUp)),
            // f32 promotes to f64 losslessly and the integral result is exact in f32,
            // so the shared f64 helper yields the correct f32 round. [OPUS-4.8]
            Num::Float(f) => Num::Float(round_half_to_pos_inf(f as f64) as f32),
            Num::Double(d) => Num::Double(round_half_to_pos_inf(d)),
        }
    }

    /// The XSD datatype IRI of this value's tier.
    #[inline]
    pub fn datatype(self) -> oxrdf::NamedNodeRef<'static> {
        use oxrdf::vocab::xsd;
        match self {
            Num::Int(_) => xsd::INTEGER,
            Num::Dec(_) => xsd::DECIMAL,
            Num::Float(_) => xsd::FLOAT,
            Num::Double(_) => xsd::DOUBLE,
        }
    }

    /// XSD CANONICAL lexical form of the value: integers as plain digits; decimals
    /// preserving the arithmetic scale ("3.0" for 1.0+2, "3" for CEIL(2.5)); float /
    /// double in mantissa-E-exponent form with a mandatory fractional digit ("3.21E4",
    /// "2.0E-1") and NaN / INF / -INF spelled per XSD.
    pub fn lexical(self) -> String {
        match self {
            Num::Int(i) => i.to_string(),
            Num::Dec(d) => d.lexical(),
            // f32 must be formatted as f32 (shortest round-trip); via f64 it would
            // grow spurious digits ("2.0000000298023224E-1" for 0.2f32).
            Num::Float(f) => {
                if f.is_nan() {
                    "NaN".to_string()
                } else if f == f32::INFINITY {
                    "INF".to_string()
                } else if f == f32::NEG_INFINITY {
                    "-INF".to_string()
                } else if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{}", f as i64)
                } else {
                    let s = format!("{f:E}");
                    match s.split_once('E') {
                        Some((m, e)) if !m.contains('.') => format!("{m}.0E{e}"),
                        _ => s,
                    }
                }
            }
            Num::Double(d) => fmt_xsd_double(d),
        }
    }

    /// STRICT XSD-canonical lexical: a FINITE float/double always in mantissa-E-exponent
    /// form ("3.21E4", "1.0E2"), never plain. The specials keep their XSD spellings
    /// (`INF` / `-INF` / `NaN` — not scientific), delegating to [`Num::lexical`]. The W3C
    /// aggregate expected results use this scientific form for MIN/MAX/SUM, while arithmetic
    /// results use the plain-integral convention of [`Num::lexical`] — the suites were
    /// generated by different engines.
    pub fn canonical_lexical(self) -> String {
        match self {
            Num::Int(_) | Num::Dec(_) => self.lexical(),
            Num::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    self.lexical()
                } else {
                    let s = format!("{f:E}");
                    match s.split_once('E') {
                        Some((m, e)) if !m.contains('.') => format!("{m}.0E{e}"),
                        _ => s,
                    }
                }
            }
            Num::Double(d) => {
                if d.is_nan() || d.is_infinite() {
                    self.lexical()
                } else {
                    let s = format!("{d:E}");
                    match s.split_once('E') {
                        Some((m, e)) if !m.contains('.') => format!("{m}.0E{e}"),
                        _ => s,
                    }
                }
            }
        }
    }

    /// `true` if this is a floating tier holding NaN.
    #[inline]
    pub fn is_nan(self) -> bool {
        match self {
            Num::Float(f) => f.is_nan(),
            Num::Double(d) => d.is_nan(),
            _ => false,
        }
    }

    /// `true` if the value is exactly zero (any tier).
    #[inline]
    pub fn is_zero(self) -> bool {
        match self {
            Num::Int(i) => i == 0,
            Num::Dec(d) => d.mant == 0,
            Num::Float(f) => f == 0.0,
            Num::Double(d) => d == 0.0,
        }
    }

    /// The typed numeric value of a literal, or `None` if the literal is not a
    /// valid, representable numeric (an invalid operand is a SPARQL type error).
    /// Valid integer/decimal values beyond the finite mantissa capacity also return `None`.
    /// This is the EXACT engine lexical path: `parse_xsd_f32`/`parse_xsd_f64` (with the
    /// XSD `INF`/`-INF`/`NaN`/exponent spellings) for float/double, and the
    /// scale-preserving [`Dec::parse_lexical`] for decimal.
    #[inline]
    pub fn of_literal(l: &oxrdf::Literal) -> Option<Num> {
        if l.language().is_some() {
            return None;
        }
        Self::of_parts(l.value(), l.datatype().as_str())
    }

    /// Parses borrowed numeric literal parts using the shared datatype and capacity rules.
    ///
    /// [GPT-6] Equivalent to [`Self::of_literal`] for a literal without a language tag.
    /// Returns `None` for invalid lexicals, subtype facet violations, nonnumeric
    /// datatypes, or values beyond this arithmetic tower's finite representation.
    #[inline]
    pub fn of_parts(value: &str, datatype: &str) -> Option<Num> {
        use oxrdf::vocab::xsd;
        let v = value.trim_matches([' ', '\t', '\r', '\n']);
        if !sparq_core::numeric_literal_valid(v, datatype) {
            return None;
        }
        if sparq_core::is_integer_datatype(datatype) {
            if let Ok(i) = v.parse::<i64>() {
                return Some(Num::Int(i));
            }
            return match Dec::parse(v) {
                Some(d) if d.scale == 0 => Some(Num::Dec(d)),
                _ => None,
            };
        }
        if datatype == xsd::DECIMAL.as_str() {
            return Dec::parse_lexical(v).map(Num::Dec);
        }
        if datatype == xsd::FLOAT.as_str() {
            return parse_xsd_f32(v).map(Num::Float);
        }
        if datatype == xsd::DOUBLE.as_str() {
            return parse_xsd_f64(v).map(Num::Double);
        }
        None
    }
}

/// Free-function alias of [`Num::of_literal`]: the typed numeric value of a literal, or
/// `None` if it is not a well-formed numeric (an ill-formed numeric operand is a SPARQL
/// type error; a language-tagged literal is never numeric). This is the substrate's
/// public literal → numeric-tower classifier the engine and a value-space reasoner share.
#[inline]
pub fn as_numeric(l: &oxrdf::Literal) -> Option<Num> {
    Num::of_literal(l)
}

#[inline]
fn apply_f64(a: f64, b: f64, op: ArithOp) -> f64 {
    match op {
        ArithOp::Add => a + b,
        ArithOp::Sub => a - b,
        ArithOp::Mul => a * b,
        ArithOp::Div => a / b,
    }
}

/// The `xsd:float` tier's arithmetic, evaluated NATIVELY in `f32` — each IEEE-754 binary32
/// operation is correctly rounded once, so the result carries no double rounding.
///
/// The previous shape (`apply_f64(a as f64, b as f64, op) as f32`) rounded the operation a
/// second time. That particular double rounding is benign for +/-/* by Figueroa's theorem
/// (`f64`'s 53-bit significand exceeds `2*24 + 2`) and a 4M-pair × 4-op random scan over the
/// full `f32` bit space (including subnormals) found no divergence — but the native form is
/// single-rounded by construction rather than by an argument, and it leaves no
/// `as f64 ... as f32` shape in this module for the next reader to have to re-audit.
/// [OPUS-5] issue #3796
#[inline]
fn apply_f32(a: f32, b: f32, op: ArithOp) -> f32 {
    match op {
        ArithOp::Add => a + b,
        ArithOp::Sub => a - b,
        ArithOp::Mul => a * b,
        ArithOp::Div => a / b,
    }
}

/// Round `x` half towards POSITIVE INFINITY (XPath `fn:round`) WITHOUT the classic
/// double-rounding defect of `(x + 0.5).floor()`.
///
/// `x + 0.5` is a rounded addition, so for a value just below half — e.g.
/// `0.49999999999999994` (the f64 predecessor of `0.5`) — the sum rounds UP to `1.0`
/// and `.floor()` then yields `1`, when the mathematically-nearest integer is `0`.
///
/// The fractional part `x - x.floor()` is instead computed EXACTLY for every non-negative
/// `x` — trivially when `floor(x) == 0`, else by Sterbenz's lemma (`floor(x)/2 <= x <=
/// 2*floor(x)`, so the difference loses no bits) — which covers the boundary case above,
/// making the half-comparison exact there. (For negative `x` the subtraction is only
/// correctly rounded, not exact, but that never flips the half tie.) Ties
/// (`x - floor(x) == 0.5`) go to the larger integer, i.e. towards `+INF`, matching
/// `round(-2.5) = -2`. NaN / ±INF / ±0.0 pass through unchanged. [OPUS-4.8]
#[inline]
fn round_half_to_pos_inf(x: f64) -> f64 {
    let fl = x.floor();
    if x - fl >= 0.5 {
        fl + 1.0
    } else {
        fl
    }
}

/// Parse an xsd:float/xsd:double lexical: the XSD spellings of the specials, plus the
/// ordinary scientific notation Rust's parser shares with XSD. `None` = ill-formed.
///
/// [FABLE-5] (sq-9781x) Delegates to the lowest-tier `sparq_core::parse_xsd_f64` — the
/// SINGLE shared body of this parser — so the numeric-value CACHE (`Graph::numeric_value`,
/// built in `sparq-core`) and this evaluator seam can never diverge on which double/float
/// lexicals are numeric. `sparq-substrate` depends on `sparq-core`, so the shared body
/// lives there; this re-export keeps the substrate's public `parse_xsd_f64` API stable.
#[inline]
pub fn parse_xsd_f64(v: &str) -> Option<f64> {
    sparq_core::parse_xsd_f64(v)
}

/// Parse an xsd:float lexical: the [`parse_xsd_f64`] spellings, valued at SINGLE precision.
///
/// ACCEPTANCE (which lexicals are well-formed XSD `floatRep`) stays the single shared body
/// in `sparq_core::parse_xsd_f64` reached through [`parse_xsd_f64`] — re-deriving the
/// spelling rules here is exactly the drift sq-9781x removed, and the reasoner's
/// `dtype::d_value_key` parity test pins the agreement.
///
/// Only the VALUE is computed here, and it is computed by a DIRECT `&str -> f32` parse.
/// The previous `parse_xsd_f64(v).map(|d| d as f32)` rounded twice — once into `f64`, once
/// in the narrowing — so an `xsd:float` literal could be mis-ingested by one ULP before any
/// arithmetic ran. Witness (verified by exact rational arithmetic): the lexical
/// `"4611686293305294849"` has correctly-rounded `f32` bits `0x5E80_0001`, but parsing at
/// `f64` and narrowing yields `0x5E80_0000`. Rust's decimal → binary parser is correctly
/// rounded at both widths, so the direct parse is one rounding of the true value.
/// [OPUS-5] issue #3796
#[inline]
pub fn parse_xsd_f32(v: &str) -> Option<f32> {
    let wide = parse_xsd_f64(v)?;
    // The XSD specials have no decimal expansion to re-parse; take them from the shared
    // acceptance result directly (`f32::from_str` would also accept `inf`/`nan` spellings
    // XSD forbids, so it must never be the gate).
    if wide.is_nan() {
        return Some(f32::NAN);
    }
    if wide.is_infinite() {
        return Some(if wide > 0.0 { f32::INFINITY } else { f32::NEG_INFINITY });
    }
    v.parse::<f32>().ok()
}

/// Float/double serialisation: an INTEGRAL value prints as a plain integer ("6",
/// "1050" — matching the dominant convention across the W3C expected results, which
/// mix plain and scientific forms); anything else uses the XSD canonical
/// mantissa-E-exponent form with a mandatory fractional digit ("2.0E-1", "1.5E1").
pub fn fmt_xsd_double(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v == f64::INFINITY {
        return "INF".to_string();
    }
    if v == f64::NEG_INFINITY {
        return "-INF".to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:E}"); // shortest round-trip mantissa, e.g. "2E-1"
    match s.split_once('E') {
        Some((m, e)) if !m.contains('.') => format!("{m}.0E{e}"),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::vocab::xsd;
    use oxrdf::Literal;

    fn typed(value: &str, dt: oxrdf::NamedNodeRef<'_>) -> Literal {
        Literal::new_typed_literal(value, dt)
    }

    #[test]
    fn integer_literal_classifies_to_int() {
        let n = as_numeric(&typed("42", xsd::INTEGER)).expect("42 is a well-formed integer");
        assert!(matches!(n, Num::Int(42)));
        assert_eq!(n.rank(), 0);
        assert_eq!(n.f64(), 42.0);
    }

    #[test]
    fn big_integer_beyond_i64_classifies_to_exact_dec() {
        // 2^63 overflows i64 but fits an i128 mantissa with scale 0 — kept EXACT, not f64.
        let big = "9223372036854775808"; // i64::MAX + 1
        let n = as_numeric(&typed(big, xsd::INTEGER)).expect("a large integer is exact in Dec");
        assert_eq!(n.to_dec(), Some(Dec { mant: 9_223_372_036_854_775_808i128, scale: 0 }));
        assert_eq!(n.rank(), 1);
    }

    #[test]
    fn fractional_integer_lexical_is_ill_formed() {
        // "1.5"^^xsd:integer is NOT a well-formed integer -> type error -> None.
        assert!(as_numeric(&typed("1.5", xsd::INTEGER)).is_none());
    }

    #[test]
    fn decimal_keeps_exact_fixed_point_and_written_scale() {
        // 0.10 as an EXACT decimal — parse_lexical PRESERVES the written scale (2), unlike
        // Dec::parse which would normalise the trailing zero away. This is the engine's
        // real path (the scaffold placeholder used Dec::parse and would have lost the scale).
        let n = as_numeric(&typed("0.10", xsd::DECIMAL)).expect("0.10 is a well-formed decimal");
        assert_eq!(n.to_dec(), Some(Dec { mant: 10, scale: 2 }));
        assert_eq!(n.rank(), 1);
        // And it round-trips to its written lexical.
        assert_eq!(n.lexical(), "0.10");
    }

    #[test]
    fn float_double_xsd_specials_parse() {
        // The XSD special spellings (INF / -INF / NaN) — the scaffold placeholder used
        // a bare v.parse::<f64>() which would REJECT "INF" and accept Rust's "inf".
        assert!(matches!(as_numeric(&typed("INF", xsd::DOUBLE)), Some(Num::Double(d)) if d == f64::INFINITY));
        assert!(matches!(as_numeric(&typed("-INF", xsd::DOUBLE)), Some(Num::Double(d)) if d == f64::NEG_INFINITY));
        assert!(matches!(as_numeric(&typed("NaN", xsd::DOUBLE)), Some(Num::Double(d)) if d.is_nan()));
        assert!(matches!(as_numeric(&typed("INF", xsd::FLOAT)), Some(Num::Float(f)) if f == f32::INFINITY));
        // Rust-only spellings XSD forbids are rejected.
        assert!(as_numeric(&typed("inf", xsd::DOUBLE)).is_none());
        assert!(as_numeric(&typed("infinity", xsd::DOUBLE)).is_none());
        assert!(as_numeric(&typed("nan", xsd::DOUBLE)).is_none());
    }

    #[test]
    fn float_double_exponent_forms_parse() {
        assert!(matches!(as_numeric(&typed("1.5E2", xsd::DOUBLE)), Some(Num::Double(d)) if d == 150.0));
        assert!(matches!(as_numeric(&typed("1.5", xsd::FLOAT)), Some(Num::Float(f)) if f == 1.5));
    }

    #[test]
    fn language_tagged_and_non_numeric_are_not_numeric() {
        assert!(as_numeric(&Literal::new_language_tagged_literal("12", "en").unwrap()).is_none());
        assert!(as_numeric(&typed("hello", xsd::STRING)).is_none());
        assert!(as_numeric(&typed("not-a-number", xsd::DECIMAL)).is_none());
    }

    #[test]
    fn dec_parse_rejects_malformed() {
        assert!(Dec::parse("").is_none());
        assert!(Dec::parse("+").is_none());
        assert!(Dec::parse(".").is_none());
        assert!(Dec::parse("1.2.3").is_none());
        assert!(Dec::parse("1e5").is_none()); // exponential is not a decimal lexical
        assert_eq!(Dec::parse("0.25"), Some(Dec { mant: 25, scale: 2 }));
    }

    #[test]
    fn exact_decimal_arithmetic_is_not_f64_rounded() {
        // 0.1 + 0.2 is EXACTLY 0.3 (the f64 path gets 0.30000000000000004).
        let a = as_numeric(&typed("0.1", xsd::DECIMAL)).unwrap();
        let b = as_numeric(&typed("0.2", xsd::DECIMAL)).unwrap();
        let s = a.binop(b, ArithOp::Add).unwrap();
        assert_eq!(s.lexical(), "0.3");
    }

    #[test]
    fn integer_division_is_decimal_and_zero_divisor_is_type_error() {
        let one = Num::Int(1);
        let three = Num::Int(3);
        // 1/3 -> decimal (rounded at scale 18), datatype xsd:decimal.
        let q = one.binop(three, ArithOp::Div).unwrap();
        assert_eq!(q.datatype(), xsd::DECIMAL);
        // x / 0 is a type error, not INF.
        assert!(one.binop(Num::Int(0), ArithOp::Div).is_none());
    }

    #[test]
    fn round_ceil_floor_preserve_datatype() {
        let d = as_numeric(&typed("2.5", xsd::DECIMAL)).unwrap();
        assert_eq!(d.ceil().lexical(), "3");
        assert_eq!(d.floor().lexical(), "2");
        assert_eq!(d.round().lexical(), "3");
        let neg = as_numeric(&typed("-2.5", xsd::DECIMAL)).unwrap();
        // fn:round is half-up towards +INF: round(-2.5) = -2.
        assert_eq!(neg.round().lexical(), "-2");
    }

    #[test]
    fn round_float_tier_no_double_rounding() {
        // Regression for sq-l11x2: the naive `(x + 0.5).floor()` double-rounds because
        // the `x + 0.5` addition is itself rounded, so a value just below one-half wrongly
        // rounds UP. `0.49999999999999994` (the f64 predecessor of 0.5) must round to 0. [OPUS-4.8]
        let just_below_half = 0.49999999999999994_f64;
        assert!(just_below_half < 0.5);
        // Pin the exact defect the OLD formula exhibited so any regression is loud:
        assert_eq!((just_below_half + 0.5).floor(), 1.0, "the naive formula rounds up");
        // The corrected helper and the xsd:double path both round DOWN to 0.
        assert_eq!(round_half_to_pos_inf(just_below_half), 0.0);
        let rd = Num::Double(just_below_half).round();
        assert_eq!(rd.f64(), 0.0);
        assert_eq!(rd.datatype(), xsd::DOUBLE);

        // The same double-rounding defect at the xsd:float tier: predecessor of 0.5_f32.
        let jbh_f32 = f32::from_bits(0x3EFF_FFFF);
        assert!(jbh_f32 < 0.5);
        let rf = Num::Float(jbh_f32).round();
        assert_eq!(rf.f64(), 0.0);
        assert_eq!(rf.datatype(), xsd::FLOAT);
    }

    #[test]
    fn round_float_tier_half_up_towards_pos_inf() {
        // fn:round is round-half-towards-+INF at the float tiers, without double rounding.
        assert_eq!(round_half_to_pos_inf(0.5), 1.0);
        assert_eq!(round_half_to_pos_inf(1.5), 2.0);
        assert_eq!(round_half_to_pos_inf(2.5), 3.0);
        assert_eq!(round_half_to_pos_inf(-0.5), 0.0); // towards +INF, not -1
        assert_eq!(round_half_to_pos_inf(-1.5), -1.0);
        assert_eq!(round_half_to_pos_inf(-2.5), -2.0);
        assert_eq!(round_half_to_pos_inf(2.4), 2.0);
        assert_eq!(round_half_to_pos_inf(2.6), 3.0);
        assert_eq!(round_half_to_pos_inf(-2.6), -3.0);
        // And through the datatype-preserving Num::round entry points.
        assert_eq!(Num::Double(-2.5).round().f64(), -2.0);
        assert_eq!(Num::Double(2.5).round().f64(), 3.0);
        assert_eq!(Num::Float(-2.5).round().f64(), -2.0);
        assert_eq!(Num::Float(2.5).round().f64(), 3.0);
    }

    #[test]
    fn round_float_tier_specials_pass_through() {
        // NaN / ±INF: the fractional part `x - x.floor()` is itself NaN (e.g. INF - INF),
        // so the `>= 0.5` branch is not taken and round() returns floor(x) — leaving the
        // rounding RESULT as NaN / ±INF unchanged.
        assert!(round_half_to_pos_inf(f64::NAN).is_nan());
        assert!(Num::Double(f64::NAN).round().f64().is_nan());
        assert_eq!(Num::Double(f64::INFINITY).round().f64(), f64::INFINITY);
        assert_eq!(Num::Double(f64::NEG_INFINITY).round().f64(), f64::NEG_INFINITY);
        assert!(Num::Float(f32::NAN).round().f64().is_nan());
    }

    #[test]
    fn ceil_floor_float_tier_are_single_rounded() {
        // ceil/floor already use the single correctly-rounded f64/f32 ops (no double
        // rounding); pin the boundary value alongside round to keep the trio in scope. [OPUS-4.8]
        let jbh = 0.49999999999999994_f64;
        assert_eq!(Num::Double(jbh).ceil().f64(), 1.0);
        assert_eq!(Num::Double(jbh).floor().f64(), 0.0);
        let jbh_f32 = f32::from_bits(0x3EFF_FFFF);
        assert_eq!(Num::Float(jbh_f32).ceil().f64(), 1.0);
        assert_eq!(Num::Float(jbh_f32).floor().f64(), 0.0);
    }

    #[test]
    fn split_decimal_normalises_zeros() {
        assert_eq!(split_decimal("007.50"), Some((false, "7", "5")));
        assert_eq!(split_decimal("-0.0"), Some((true, "", "")));
        assert!(split_decimal("1e5").is_none());
    }

    #[test]
    fn fmt_xsd_double_integral_and_scientific() {
        assert_eq!(fmt_xsd_double(6.0), "6");
        assert_eq!(fmt_xsd_double(0.2), "2.0E-1");
        assert_eq!(fmt_xsd_double(f64::INFINITY), "INF");
        assert_eq!(fmt_xsd_double(f64::NAN), "NaN");
    }

    #[test]
    fn canonical_lexical_is_always_scientific_for_floats() {
        let two = Num::Double(2.0);
        assert_eq!(two.canonical_lexical(), "2.0E0");
        // plain lexical keeps the integral-plain convention.
        assert_eq!(two.lexical(), "2");
    }

    #[test]
    fn parse_xsd_f64_f32_accept_xsd_specials_reject_rust_only_spellings() {
        // [OPUS-4.8] sq-rkzhr — direct coverage of the shared parser that BOTH the exact
        // `as_numeric` path and (after sq-rkzhr) the lenient engine `as_num`/`as_f64`
        // compare seam route through: the XSD spellings parse; the Rust-`FromStr`-only
        // spellings XSD forbids do not, so the two comparison paths can never disagree on
        // which double/float lexicals are numeric.
        assert_eq!(parse_xsd_f64("INF"), Some(f64::INFINITY));
        assert_eq!(parse_xsd_f64("+INF"), Some(f64::INFINITY));
        assert_eq!(parse_xsd_f64("-INF"), Some(f64::NEG_INFINITY));
        assert!(matches!(parse_xsd_f64("NaN"), Some(d) if d.is_nan()));
        assert_eq!(parse_xsd_f64("1.5E2"), Some(150.0));
        assert_eq!(parse_xsd_f64("6"), Some(6.0));
        for bad in ["inf", "+inf", "infinity", "-infinity", "nan", "Infinity", "NAN", ""] {
            // Positional arg (not inline `{bad}`) to dodge the CodeQL
            // `rust/unused-variable` false positive. [OPUS-4.8]
            assert_eq!(parse_xsd_f64(bad), None, "parse_xsd_f64 must reject {:?}", bad);
        }
        // f32 narrows the same acceptance set.
        assert_eq!(parse_xsd_f32("INF"), Some(f32::INFINITY));
        assert_eq!(parse_xsd_f32("-INF"), Some(f32::NEG_INFINITY));
        assert!(parse_xsd_f32("nan").is_none());
    }

    #[test]
    fn double_float_lexical_two_convention_per_surface_policy() {
        // [OPUS-4.8] sq-rkzhr — LOCK the deliberate two-convention split so a future
        // refactor cannot silently collapse it. `lexical` (STR / serialize / arithmetic
        // result construction) keeps the PLAIN-integral form the W3C SPARQL expected-result
        // files use for computed double/float arithmetic ("6", not "6.0E0"); while
        // `canonical_lexical` (the SUM/MIN/MAX aggregate-term surface) is the XSD-mandatory
        // scientific form. A global flip of `lexical` to scientific was MEASURED to REGRESS
        // 5 W3C eval tests (expr-ops +/-/*/unary-minus, AVG DISTINCT), so this split is
        // intentional and conformance-load-bearing, not an oversight.
        let dbl = Num::Double(6.0);
        assert_eq!(dbl.lexical(), "6"); // plain-integral (W3C-suite convention)
        assert_eq!(dbl.canonical_lexical(), "6.0E0"); // XSD-canonical scientific
        let flt = Num::Float(6.0);
        assert_eq!(flt.lexical(), "6");
        assert_eq!(flt.canonical_lexical(), "6.0E0");
        // Non-integral values are scientific under BOTH surfaces (no divergence there).
        assert_eq!(Num::Double(0.2).lexical(), "2.0E-1");
        assert_eq!(Num::Double(0.2).canonical_lexical(), "2.0E-1");
        // The XSD specials spell identically under both surfaces.
        assert_eq!(Num::Double(f64::INFINITY).lexical(), "INF");
        assert_eq!(Num::Double(f64::INFINITY).canonical_lexical(), "INF");
        assert_eq!(Num::Double(f64::NEG_INFINITY).lexical(), "-INF");
        assert_eq!(Num::Double(f64::NEG_INFINITY).canonical_lexical(), "-INF");
        assert_eq!(Num::Double(f64::NAN).lexical(), "NaN");
        assert_eq!(Num::Double(f64::NAN).canonical_lexical(), "NaN");
    }

    // [OPUS-4.8] sq-qcnn.12 — DIRECT unit tests over the numeric value tower's arithmetic /
    // rounding / accessor / lexical surface. Each asserts an EXACT value (not merely
    // "no panic") so a mutation of the comparison / arithmetic / branch logic goes red:
    // this is the mutation-killing coverage the epic's floor-raise gates.

    #[test]
    fn dec_parse_lexical_edge_cases() {
        // Empty / plus-only / dot-only lexicals are NOT a decimal -> None.
        assert!(Dec::parse_lexical("").is_none());
        assert!(Dec::parse_lexical("+").is_none());
        assert!(Dec::parse_lexical(".").is_none());
        assert!(Dec::parse_lexical("abc").is_none());
        // Leading-sign variants; parse_lexical PRESERVES the written scale (unlike Dec::parse,
        // which normalises trailing fraction zeros away).
        assert_eq!(Dec::parse_lexical("+5"), Some(Dec { mant: 5, scale: 0 }));
        assert_eq!(Dec::parse_lexical("-3.0"), Some(Dec { mant: -30, scale: 1 }));
        assert_eq!(Dec::parse_lexical("1.00"), Some(Dec { mant: 100, scale: 2 }));
        // Negative flag with a zero mantissa stays a non-negative zero mantissa.
        assert_eq!(Dec::parse_lexical("-0.0"), Some(Dec { mant: 0, scale: 1 }));
        // A bare "." after a sign is still ill-formed.
        assert!(Dec::parse_lexical("-.").is_none());
    }

    #[test]
    fn dec_checked_add_sub_mul_and_align_overflow() {
        // align() scales both operands to the common (max) scale before combining.
        let a = Dec { mant: 1, scale: 0 }; // 1
        let b = Dec { mant: 2, scale: 1 }; // 0.2
        assert_eq!(a.checked_add(b), Some(Dec { mant: 12, scale: 1 })); // 1.2
        let c = Dec { mant: 12, scale: 1 }; // 1.2
        let d = Dec { mant: 2, scale: 1 }; // 0.2
        assert_eq!(c.checked_sub(d), Some(Dec { mant: 10, scale: 1 })); // 1.0
        // checked_mul ADDS the scales and multiplies the mantissas exactly.
        assert_eq!(c.checked_mul(Dec { mant: 2, scale: 0 }), Some(Dec { mant: 24, scale: 1 })); // 2.4
        // Mantissa-add overflow -> None (the caller falls back to f64).
        let big = Dec { mant: i128::MAX, scale: 0 };
        assert_eq!(big.checked_add(Dec { mant: 1, scale: 0 }), None);
        assert_eq!(big.checked_mul(Dec { mant: 2, scale: 0 }), None);
        // A scale-ALIGNMENT overflow (multiplying i128::MAX by 10 to line up scales) -> None.
        assert_eq!(big.checked_add(Dec { mant: 1, scale: 1 }), None);
        assert_eq!(big.checked_sub(Dec { mant: 1, scale: 1 }), None);
    }

    #[test]
    fn dec_checked_div_terminating_nonterminating_and_negative_exponent() {
        // Terminating at scale 2: 1/4 = 0.25.
        assert_eq!(
            Dec { mant: 1, scale: 0 }.checked_div(Dec { mant: 4, scale: 0 }),
            Some(Dec { mant: 25, scale: 2 })
        );
        // Non-terminating: 1/3 rounds half-up at the max scale 18.
        let third = Dec { mant: 1, scale: 0 }.checked_div(Dec { mant: 3, scale: 0 }).unwrap();
        assert_eq!(third.scale, 18);
        assert_eq!(third.mant, 333_333_333_333_333_333i128);
        // The e < 0 branch of num_den: dividend scale exceeds the trial scale, so the
        // DIVISOR is scaled up. 0.005 / 1 terminates back at scale 3 as 0.005.
        assert_eq!(
            Dec { mant: 5, scale: 3 }.checked_div(Dec { mant: 1, scale: 0 }),
            Some(Dec { mant: 5, scale: 3 })
        );
        // Sign: negative / positive is negative; the terminating scale is 1.
        assert_eq!(
            Dec { mant: -1, scale: 0 }.checked_div(Dec { mant: 2, scale: 0 }),
            Some(Dec { mant: -5, scale: 1 })
        ); // -0.5
        // 0 / 5 terminates at scale 1 as 0.0.
        assert_eq!(
            Dec { mant: 0, scale: 0 }.checked_div(Dec { mant: 5, scale: 0 }),
            Some(Dec { mant: 0, scale: 1 })
        );
    }

    #[test]
    fn dec_round_to_int_early_return_and_overflow_scale() {
        // Early return: scale 0 is already integer-valued; a zero mantissa is too.
        assert_eq!(Dec { mant: 5, scale: 0 }.round_to_int(RoundMode::Floor), Dec { mant: 5, scale: 0 });
        assert_eq!(Dec { mant: 0, scale: 3 }.round_to_int(RoundMode::Ceil), Dec { mant: 0, scale: 0 });
        // Ordinary scale: 2.5 floors to 2, ceils to 3, half-up to 3.
        let two_five = Dec { mant: 25, scale: 1 };
        assert_eq!(two_five.round_to_int(RoundMode::Floor), Dec { mant: 2, scale: 0 });
        assert_eq!(two_five.round_to_int(RoundMode::Ceil), Dec { mant: 3, scale: 0 });
        assert_eq!(two_five.round_to_int(RoundMode::HalfUp), Dec { mant: 3, scale: 0 });
        // Scale >= 39: 10^scale exceeds i128, so |value| < 1 and the result is derived from
        // the sign alone WITHOUT constructing the overflowing power.
        let tiny_pos = Dec { mant: 1, scale: 40 };
        assert_eq!(tiny_pos.round_to_int(RoundMode::Floor), Dec { mant: 0, scale: 0 });
        assert_eq!(tiny_pos.round_to_int(RoundMode::Ceil), Dec { mant: 1, scale: 0 });
        assert_eq!(tiny_pos.round_to_int(RoundMode::HalfUp), Dec { mant: 0, scale: 0 });
        let tiny_neg = Dec { mant: -1, scale: 40 };
        assert_eq!(tiny_neg.round_to_int(RoundMode::Floor), Dec { mant: -1, scale: 0 });
        assert_eq!(tiny_neg.round_to_int(RoundMode::Ceil), Dec { mant: 0, scale: 0 });
        assert_eq!(tiny_neg.round_to_int(RoundMode::HalfUp), Dec { mant: 0, scale: 0 });
    }

    #[test]
    fn dec_cmp_total_order_across_scales() {
        let one = Dec { mant: 10, scale: 1 }; // 1.0
        let two = Dec { mant: 20, scale: 1 }; // 2.0
        let one_padded = Dec { mant: 100, scale: 2 }; // 1.00
        assert_eq!(one.cmp(two), Some(Ordering::Less));
        assert_eq!(two.cmp(one), Some(Ordering::Greater));
        assert_eq!(one.cmp(one_padded), Some(Ordering::Equal)); // 1.0 == 1.00
    }

    #[test]
    fn dec_f64_projection() {
        assert!((Dec { mant: 15, scale: 1 }.f64() - 1.5).abs() < 1e-12);
        assert!((Dec { mant: -25, scale: 2 }.f64() - (-0.25)).abs() < 1e-12);
        assert_eq!(Dec { mant: 7, scale: 0 }.f64(), 7.0);
    }

    #[test]
    fn dec_lexical_fraction_and_integer_part() {
        // scale 0 -> plain integer.
        assert_eq!(Dec { mant: 7, scale: 0 }.lexical(), "7");
        assert_eq!(Dec { mant: -7, scale: 0 }.lexical(), "-7");
        // |mant| shorter than the scale -> leading "0." then zero-padding.
        assert_eq!(Dec { mant: 5, scale: 2 }.lexical(), "0.05");
        assert_eq!(Dec { mant: -5, scale: 2 }.lexical(), "-0.05");
        assert_eq!(Dec { mant: 0, scale: 1 }.lexical(), "0.0");
        // |mant| longer than the scale -> a non-zero integer part with a decimal point.
        assert_eq!(Dec { mant: 15, scale: 1 }.lexical(), "1.5");
        assert_eq!(Dec { mant: 1234, scale: 2 }.lexical(), "12.34");
        assert_eq!(Dec { mant: -1234, scale: 2 }.lexical(), "-12.34");
    }

    #[test]
    fn num_rank_orders_the_promotion_tower() {
        assert_eq!(Num::Int(0).rank(), 0);
        assert_eq!(Num::Dec(Dec { mant: 0, scale: 0 }).rank(), 1);
        assert_eq!(Num::Float(0.0).rank(), 2);
        assert_eq!(Num::Double(0.0).rank(), 3);
    }

    #[test]
    fn num_f64_all_variants() {
        assert_eq!(Num::Int(-3).f64(), -3.0);
        assert!((Num::Dec(Dec { mant: 15, scale: 1 }).f64() - 1.5).abs() < 1e-12);
        assert_eq!(Num::Float(2.5).f64(), 2.5f64);
        assert_eq!(Num::Double(3.25).f64(), 3.25);
    }

    #[test]
    fn num_to_dec_exact_tiers_only() {
        assert_eq!(Num::Int(7).to_dec(), Some(Dec { mant: 7, scale: 0 }));
        let d = Dec { mant: 42, scale: 1 };
        assert_eq!(Num::Dec(d).to_dec(), Some(d));
        assert!(Num::Float(1.5).to_dec().is_none());
        assert!(Num::Double(1.5).to_dec().is_none());
    }

    #[test]
    fn num_binop_int_tier_exact_and_overflow_to_double() {
        // integer op integer stays integer and is exact.
        assert!(matches!(Num::Int(2).binop(Num::Int(3), ArithOp::Add), Some(Num::Int(5))));
        assert!(matches!(Num::Int(10).binop(Num::Int(4), ArithOp::Sub), Some(Num::Int(6))));
        assert!(matches!(Num::Int(6).binop(Num::Int(7), ArithOp::Mul), Some(Num::Int(42))));
        // i64::MAX + 1 overflows the exact tier -> falls back to Double.
        assert!(matches!(Num::Int(i64::MAX).binop(Num::Int(1), ArithOp::Add), Some(Num::Double(_))));
        // i64::MAX * 2 likewise overflows -> Double.
        assert!(matches!(Num::Int(i64::MAX).binop(Num::Int(2), ArithOp::Mul), Some(Num::Double(_))));
    }

    #[test]
    fn num_binop_dec_tier_exact_and_overflow_to_double() {
        // decimal op decimal stays exact decimal (no f64 rounding).
        let a = Num::Dec(Dec { mant: 12, scale: 1 }); // 1.2
        let b = Num::Dec(Dec { mant: 3, scale: 1 }); // 0.3
        assert!(matches!(a.binop(b, ArithOp::Add), Some(Num::Dec(d)) if d == Dec { mant: 15, scale: 1 }));
        assert!(matches!(a.binop(b, ArithOp::Sub), Some(Num::Dec(d)) if d == Dec { mant: 9, scale: 1 }));
        // i128::MAX * 2 overflows the exact decimal multiply -> falls back to Double.
        let big = Num::Dec(Dec { mant: i128::MAX, scale: 0 });
        let two = Num::Dec(Dec { mant: 2, scale: 0 });
        assert!(matches!(big.binop(two, ArithOp::Mul), Some(Num::Double(_))));
    }

    #[test]
    fn num_binop_div_is_decimal_and_zero_divisor_is_error() {
        // 1/4 divides exactly to the terminating decimal 0.25.
        let q = Num::Int(1).binop(Num::Int(4), ArithOp::Div).unwrap();
        assert!(matches!(q, Num::Dec(d) if d == Dec { mant: 25, scale: 2 }));
        assert_eq!(q.datatype(), xsd::DECIMAL);
        // x / 0 in the exact tier is a SPARQL type error (None), never INF.
        assert!(Num::Int(1).binop(Num::Int(0), ArithOp::Div).is_none());
        assert!(Num::Dec(Dec { mant: 5, scale: 1 }).binop(Num::Dec(Dec { mant: 0, scale: 0 }), ArithOp::Div).is_none());
    }

    #[test]
    fn num_binop_float_tier_stays_float() {
        let a = Num::Float(2.0);
        let b = Num::Float(3.0);
        assert!(matches!(a.binop(b, ArithOp::Add), Some(Num::Float(f)) if (f - 5.0f32).abs() < 1e-6));
        assert!(matches!(a.binop(b, ArithOp::Sub), Some(Num::Float(f)) if (f - (-1.0f32)).abs() < 1e-6));
        assert!(matches!(a.binop(b, ArithOp::Mul), Some(Num::Float(f)) if (f - 6.0f32).abs() < 1e-6));
        assert!(matches!(a.binop(b, ArithOp::Div), Some(Num::Float(f)) if (f - (2.0f32 / 3.0f32)).abs() < 1e-6));
        // A mixed float/int operation promotes to the float tier.
        assert!(matches!(Num::Int(4).binop(Num::Float(2.0), ArithOp::Div), Some(Num::Float(f)) if (f - 2.0f32).abs() < 1e-6));
    }

    // -----------------------------------------------------------------------
    // xsd:float promotion / parse must round to single precision EXACTLY ONCE.
    // [OPUS-5] issue #3796
    //
    // Every fixture below was constructed and cross-checked with EXACT RATIONAL
    // arithmetic, not with floats: `H = (2m+1) * 2^38` is an f32 MIDPOINT that is
    // exactly representable in f64, so an integer one unit away from `H` rounds to
    // `H` in f64 and then ties-to-even in the narrowing — landing on the wrong
    // side. `..._DOUBLE_ROUNDED` is what `as f64 as f32` produces (the pre-fix
    // behaviour); `..._CORRECT` is the correctly-rounded single conversion.
    //
    // The assertions are on `f32::to_bits()` deliberately: the two candidates are
    // ADJACENT floats and format identically at ordinary precision, which is why
    // the existing small-value coverage could not see this.
    // -----------------------------------------------------------------------

    /// `H + 1` for `H = (2^24+1) * 2^38`: true value is ABOVE the midpoint, so correct
    /// rounding goes UP, but the f64 detour ties-to-even DOWN.
    const F32_UP_I64: i64 = 4_611_686_293_305_294_849;
    const F32_UP_DOUBLE_ROUNDED: u32 = 0x5E80_0000;
    const F32_UP_CORRECT: u32 = 0x5E80_0001;
    /// `H - 1` for `H = (2^24+3) * 2^38`: true value is BELOW the midpoint, so correct
    /// rounding goes DOWN, but the f64 detour ties-to-even UP. (Both error directions
    /// are covered so a fix that merely biases one way cannot pass.)
    const F32_DOWN_I64: i64 = 4_611_686_843_061_108_735;
    const F32_DOWN_DOUBLE_ROUNDED: u32 = 0x5E80_0002;
    const F32_DOWN_CORRECT: u32 = 0x5E80_0001;

    #[test]
    fn num_f32_promotion_of_i64_is_single_rounded_bit_exact() {
        // DIRECT unit test of the public `Num::f32` promotion helper.
        assert_eq!(Num::Int(F32_UP_I64).f32().to_bits(), F32_UP_CORRECT);
        assert_eq!(Num::Int(F32_DOWN_I64).f32().to_bits(), F32_DOWN_CORRECT);
        // ... and it is NOT the double-rounded value the f64 detour produces.
        assert_eq!((F32_UP_I64 as f64 as f32).to_bits(), F32_UP_DOUBLE_ROUNDED);
        assert_ne!(Num::Int(F32_UP_I64).f32().to_bits(), F32_UP_DOUBLE_ROUNDED);
        assert_eq!((F32_DOWN_I64 as f64 as f32).to_bits(), F32_DOWN_DOUBLE_ROUNDED);
        assert_ne!(Num::Int(F32_DOWN_I64).f32().to_bits(), F32_DOWN_DOUBLE_ROUNDED);
        // The other tiers of the helper: Float is the identity, Double narrows once,
        // and small exactly-representable values are unaffected.
        assert_eq!(Num::Float(1.5f32).f32().to_bits(), 1.5f32.to_bits());
        assert_eq!(Num::Double(0.1f64).f32().to_bits(), (0.1f64 as f32).to_bits());
        assert_eq!(Num::Int(42).f32().to_bits(), 42.0f32.to_bits());
    }

    #[test]
    fn num_f32_promotion_of_over_i64_integer_is_single_rounded_bit_exact() {
        // An xsd:integer beyond i64 is carried as a scale-0 `Dec`; the same midpoint
        // construction at 2^70 (so it cannot fit i64) witnesses the same defect.
        let up = Dec { mant: 1_180_591_691_086_155_481_089, scale: 0 };
        let down = Dec { mant: 1_180_591_831_823_643_836_415, scale: 0 };
        assert_eq!(Num::Dec(up).f32().to_bits(), 0x6280_0001);
        assert_eq!(Num::Dec(down).f32().to_bits(), 0x6280_0001);
        assert_eq!((up.f64() as f32).to_bits(), 0x6280_0000);
        assert_eq!((down.f64() as f32).to_bits(), 0x6280_0002);
    }

    #[test]
    fn dec_f32_is_single_rounded_bit_exact_at_both_scales() {
        // DIRECT unit test of the public `Dec::f32`.
        // scale 0 (exact integer) — the i128 -> f32 conversion.
        assert_eq!(Dec { mant: 1_180_591_691_086_155_481_089, scale: 0 }.f32().to_bits(), 0x6280_0001);
        // scale > 0 — `4611686293305294848.1` and `4611686843061108735.9`, each one tenth
        // off an f32 midpoint, so the exact value is unambiguously on one side.
        let a = Dec { mant: 46_116_862_933_052_948_481, scale: 1 };
        let b = Dec { mant: 46_116_868_430_611_087_359, scale: 1 };
        assert_eq!(a.lexical(), "4611686293305294848.1");
        assert_eq!(b.lexical(), "4611686843061108735.9");
        assert_eq!(a.f32().to_bits(), F32_UP_CORRECT);
        assert_eq!(b.f32().to_bits(), F32_DOWN_CORRECT);
        assert_eq!((a.f64() as f32).to_bits(), F32_UP_DOUBLE_ROUNDED);
        assert_eq!((b.f64() as f32).to_bits(), F32_DOWN_DOUBLE_ROUNDED);
        // Ordinary small decimals are untouched by the change.
        assert_eq!(Dec { mant: 15, scale: 1 }.f32().to_bits(), 1.5f32.to_bits());
        assert_eq!(Dec { mant: -5, scale: 0 }.f32().to_bits(), (-5.0f32).to_bits());
    }

    #[test]
    fn num_binop_float_promotion_of_integer_is_bit_exact() {
        // The reported impact: `SUM(?int, "0.0"^^xsd:float)` / `AVG` over a mixed
        // integer/float column. `Int + Float(0.0)` promotes to the float tier.
        let zero = Num::Float(0.0);
        for (n, want) in [(F32_UP_I64, F32_UP_CORRECT), (F32_DOWN_I64, F32_DOWN_CORRECT)] {
            let got = match Num::Int(n).binop(zero, ArithOp::Add) {
                Some(Num::Float(f)) => f,
                other => panic!("int + float must stay on the float tier, got {:?}", other),
            };
            assert_eq!(got.to_bits(), want, "SUM(?int, 0.0f) for n={}", n);
            // The float operand may also come first; promotion is symmetric.
            let got_rev = match zero.binop(Num::Int(n), ArithOp::Add) {
                Some(Num::Float(f)) => f,
                other => panic!("float + int must stay on the float tier, got {:?}", other),
            };
            assert_eq!(got_rev.to_bits(), want);
        }
        // Multiplication by 1.0f and subtraction of 0.0f carry the value through too.
        assert_eq!(
            match Num::Int(F32_UP_I64).binop(Num::Float(1.0), ArithOp::Mul) {
                Some(Num::Float(f)) => f.to_bits(),
                other => panic!("unexpected {:?}", other),
            },
            F32_UP_CORRECT
        );
        assert_eq!(
            match Num::Int(F32_DOWN_I64).binop(Num::Float(0.0), ArithOp::Sub) {
                Some(Num::Float(f)) => f.to_bits(),
                other => panic!("unexpected {:?}", other),
            },
            F32_DOWN_CORRECT
        );
    }

    #[test]
    fn parse_xsd_f32_is_single_rounded_bit_exact() {
        // The PARSE path — a literal is mis-ingested before any arithmetic runs.
        // Plain and scientific spellings of the SAME exact value must both land on the
        // correctly-rounded f32.
        for lex in ["4611686293305294849", "4.611686293305294849E18"] {
            assert_eq!(
                parse_xsd_f32(lex).expect("well-formed xsd:float lexical").to_bits(),
                F32_UP_CORRECT,
                "parse_xsd_f32({:?})",
                lex
            );
            assert_eq!(
                (parse_xsd_f64(lex).expect("well-formed") as f32).to_bits(),
                F32_UP_DOUBLE_ROUNDED,
                "the f64-then-narrow route is the WRONG value for {:?}",
                lex
            );
        }
        for lex in ["4611686843061108735", "4.611686843061108735E18"] {
            assert_eq!(
                parse_xsd_f32(lex).expect("well-formed xsd:float lexical").to_bits(),
                F32_DOWN_CORRECT,
                "parse_xsd_f32({:?})",
                lex
            );
            assert_eq!(
                (parse_xsd_f64(lex).expect("well-formed") as f32).to_bits(),
                F32_DOWN_DOUBLE_ROUNDED,
                "the f64-then-narrow route is the WRONG value for {:?}",
                lex
            );
        }
        // ACCEPTANCE is unchanged: still the shared sparq-core spelling rules, so the
        // XSD specials parse and the Rust-only spellings stay rejected.
        assert_eq!(parse_xsd_f32("NaN").map(f32::is_nan), Some(true));
        assert_eq!(parse_xsd_f32("INF"), Some(f32::INFINITY));
        assert_eq!(parse_xsd_f32("+INF"), Some(f32::INFINITY));
        assert_eq!(parse_xsd_f32("-INF"), Some(f32::NEG_INFINITY));
        assert_eq!(parse_xsd_f32("-0.0").map(f32::to_bits), Some((-0.0f32).to_bits()));
        for bad in ["inf", "Infinity", "nan", "NAN", "0x1p3", "1.0f", "", "abc"] {
            assert_eq!(parse_xsd_f32(bad), None, "must reject {:?}", bad);
        }
        // Out-of-f32-range magnitudes saturate to the XSD specials, as before.
        assert_eq!(parse_xsd_f32("1E40"), Some(f32::INFINITY));
        assert_eq!(parse_xsd_f32("-1E40"), Some(f32::NEG_INFINITY));
    }

    #[test]
    fn of_literal_xsd_float_literal_is_single_rounded_bit_exact() {
        // End-to-end through the classifier the engine actually calls.
        let up = as_numeric(&typed("4611686293305294849", xsd::FLOAT))
            .expect("well-formed xsd:float literal");
        assert!(matches!(up, Num::Float(f) if f.to_bits() == F32_UP_CORRECT), "got {:?}", up);
        let down = as_numeric(&typed("4.611686843061108735E18", xsd::FLOAT))
            .expect("well-formed xsd:float literal");
        assert!(matches!(down, Num::Float(f) if f.to_bits() == F32_DOWN_CORRECT), "got {:?}", down);
        // The xsd:integer literal of the same digits promotes to the same f32.
        let as_int = as_numeric(&typed("4611686293305294849", xsd::INTEGER)).expect("integer");
        assert_eq!(as_int.f32().to_bits(), F32_UP_CORRECT);
    }

    #[test]
    fn num_binop_double_tier_stays_double() {
        let a = Num::Double(10.0);
        let b = Num::Double(4.0);
        assert!(matches!(a.binop(b, ArithOp::Add), Some(Num::Double(d)) if (d - 14.0).abs() < 1e-12));
        assert!(matches!(a.binop(b, ArithOp::Sub), Some(Num::Double(d)) if (d - 6.0).abs() < 1e-12));
        assert!(matches!(a.binop(b, ArithOp::Mul), Some(Num::Double(d)) if (d - 40.0).abs() < 1e-12));
        assert!(matches!(a.binop(b, ArithOp::Div), Some(Num::Double(d)) if (d - 2.5).abs() < 1e-12));
        // The double tier dominates a decimal operand.
        assert!(matches!(Num::Dec(Dec { mant: 5, scale: 0 }).binop(Num::Double(2.0), ArithOp::Add), Some(Num::Double(d)) if (d - 7.0).abs() < 1e-12));
    }

    #[test]
    fn num_neg_all_variants_and_overflow_to_double() {
        assert!(matches!(Num::Int(5).neg(), Num::Int(-5)));
        assert!(matches!(Num::Dec(Dec { mant: 15, scale: 1 }).neg(), Num::Dec(d) if d == Dec { mant: -15, scale: 1 }));
        assert!(matches!(Num::Float(3.0).neg(), Num::Float(f) if f == -3.0));
        assert!(matches!(Num::Double(2.5).neg(), Num::Double(d) if d == -2.5));
        // i64::MIN has no i64 negation -> promotes to Double (never panics/wraps).
        assert!(matches!(Num::Int(i64::MIN).neg(), Num::Double(d) if d == (i64::MIN as f64).abs()));
        // i128::MIN mantissa negation overflows -> Double as well.
        assert!(matches!(Num::Dec(Dec { mant: i128::MIN, scale: 0 }).neg(), Num::Double(d) if d > 0.0));
    }

    #[test]
    fn num_abs_all_variants_and_overflow_to_double() {
        assert!(matches!(Num::Int(-5).abs(), Num::Int(5)));
        assert!(matches!(Num::Dec(Dec { mant: -15, scale: 1 }).abs(), Num::Dec(d) if d == Dec { mant: 15, scale: 1 }));
        assert!(matches!(Num::Float(-3.0).abs(), Num::Float(f) if f == 3.0));
        assert!(matches!(Num::Double(-2.5).abs(), Num::Double(d) if d == 2.5));
        // i64::MIN has no i64 absolute value -> Double.
        assert!(matches!(Num::Int(i64::MIN).abs(), Num::Double(d) if d > 0.0));
        // i128::MIN likewise.
        assert!(matches!(Num::Dec(Dec { mant: i128::MIN, scale: 0 }).abs(), Num::Double(d) if d > 0.0));
    }

    #[test]
    fn num_ceil_floor_round_int_tier_is_identity() {
        // For an integer the three roundings are all the identity.
        assert!(matches!(Num::Int(5).ceil(), Num::Int(5)));
        assert!(matches!(Num::Int(5).floor(), Num::Int(5)));
        assert!(matches!(Num::Int(5).round(), Num::Int(5)));
    }

    #[test]
    fn num_ceil_floor_round_dec_tier_preserves_decimal() {
        let d = Num::Dec(Dec { mant: 25, scale: 1 }); // 2.5
        assert!(matches!(d.ceil(), Num::Dec(x) if x == Dec { mant: 3, scale: 0 }));
        assert!(matches!(d.floor(), Num::Dec(x) if x == Dec { mant: 2, scale: 0 }));
        assert!(matches!(d.round(), Num::Dec(x) if x == Dec { mant: 3, scale: 0 }));
    }

    #[test]
    fn num_datatype_all_variants() {
        assert_eq!(Num::Int(0).datatype(), xsd::INTEGER);
        assert_eq!(Num::Dec(Dec { mant: 0, scale: 0 }).datatype(), xsd::DECIMAL);
        assert_eq!(Num::Float(0.0).datatype(), xsd::FLOAT);
        assert_eq!(Num::Double(0.0).datatype(), xsd::DOUBLE);
    }

    #[test]
    fn num_lexical_float_tier_integral_scientific_and_specials() {
        // An integral float prints as a plain integer.
        assert_eq!(Num::Float(6.0).lexical(), "6");
        assert_eq!(Num::Float(-6.0).lexical(), "-6");
        // A non-integral float uses the XSD mantissa-E-exponent form with a fraction digit.
        assert_eq!(Num::Float(0.2).lexical(), "2.0E-1");
        assert_eq!(Num::Float(1.5).lexical(), "1.5E0");
        // The specials keep their XSD spellings.
        assert_eq!(Num::Float(f32::NAN).lexical(), "NaN");
        assert_eq!(Num::Float(f32::INFINITY).lexical(), "INF");
        assert_eq!(Num::Float(f32::NEG_INFINITY).lexical(), "-INF");
    }

    #[test]
    fn num_canonical_lexical_float_and_specials() {
        // A finite float is always scientific under canonical_lexical.
        assert_eq!(Num::Float(2.0).canonical_lexical(), "2.0E0");
        assert_eq!(Num::Float(1.5).canonical_lexical(), "1.5E0");
        // The specials delegate to lexical (their XSD spelling, not scientific).
        assert_eq!(Num::Float(f32::NAN).canonical_lexical(), "NaN");
        assert_eq!(Num::Float(f32::INFINITY).canonical_lexical(), "INF");
        assert_eq!(Num::Float(f32::NEG_INFINITY).canonical_lexical(), "-INF");
        // Double specials also delegate to lexical.
        assert_eq!(Num::Double(f64::INFINITY).canonical_lexical(), "INF");
        assert_eq!(Num::Double(f64::NEG_INFINITY).canonical_lexical(), "-INF");
        assert_eq!(Num::Double(f64::NAN).canonical_lexical(), "NaN");
        // A finite double is scientific.
        assert_eq!(Num::Double(1.5).canonical_lexical(), "1.5E0");
    }

    #[test]
    fn num_is_nan_all_variants() {
        assert!(!Num::Int(0).is_nan());
        assert!(!Num::Dec(Dec { mant: 0, scale: 0 }).is_nan());
        assert!(Num::Float(f32::NAN).is_nan());
        assert!(!Num::Float(1.0).is_nan());
        assert!(Num::Double(f64::NAN).is_nan());
        assert!(!Num::Double(1.0).is_nan());
    }

    #[test]
    fn num_is_zero_all_variants() {
        assert!(Num::Int(0).is_zero());
        assert!(!Num::Int(1).is_zero());
        assert!(Num::Dec(Dec { mant: 0, scale: 2 }).is_zero());
        assert!(!Num::Dec(Dec { mant: 1, scale: 2 }).is_zero());
        assert!(Num::Float(0.0).is_zero());
        assert!(!Num::Float(1.0).is_zero());
        assert!(Num::Double(0.0).is_zero());
        assert!(!Num::Double(1.0).is_zero());
    }

    #[test]
    fn fmt_xsd_double_negative_infinity_and_more() {
        assert_eq!(fmt_xsd_double(f64::NEG_INFINITY), "-INF");
        assert_eq!(fmt_xsd_double(-6.0), "-6");
        assert_eq!(fmt_xsd_double(1.5), "1.5E0");
    }

    // --- Num::cmp_total — the ORDER BY total order over numeric values (sq-wjl8i) ---

    const TWO53: i64 = 9_007_199_254_740_992;

    /// NaN takes a FIXED position: before -INF, equal to itself — the totalisation.
    #[test]
    fn cmp_total_nan_sorts_first_and_equals_itself() {
        use Ordering::*;
        let nan = Num::Double(f64::NAN);
        assert_eq!(nan.cmp_total(Num::Double(f64::NEG_INFINITY)), Less);
        assert_eq!(nan.cmp_total(Num::Int(0)), Less);
        assert_eq!(Num::Int(0).cmp_total(nan), Greater);
        assert_eq!(nan.cmp_total(Num::Float(f32::NAN)), Equal);
        assert_eq!(nan.cmp_total(nan), Equal);
    }

    /// The mixed exact/inexact tier at the 2^53 collapse: the double equal to the shared
    /// f64 image is EQUAL to the integer it exactly is, and strictly BELOW the collapsed
    /// neighbour 2^53+1 — the intransitivity witness of bead sq-wjl8i, now decided
    /// exactly instead of collapsing to a three-way tie.
    #[test]
    fn cmp_total_mixed_tier_is_exact_at_the_2p53_collapse() {
        use Ordering::*;
        let dbl = Num::Double(TWO53 as f64);
        let int_lo = Num::Int(TWO53);
        let int_hi = Num::Dec(Dec { mant: TWO53 as i128 + 1, scale: 0 });
        assert_eq!(int_lo.cmp_total(dbl), Equal, "2^53 IS the double exactly");
        assert_eq!(dbl.cmp_total(int_hi), Less, "the collapsed neighbour still orders");
        assert_eq!(int_hi.cmp_total(dbl), Greater);
        assert_eq!(int_lo.cmp_total(int_hi), Less);
    }

    /// A decimal against the double it rounds to is NOT equal: `0.1` (exact) is below
    /// the double `0.1` (exactly 0.1000000000000000055511151231257827…). And ±INF /
    /// exact pairs order by sign of the infinity.
    #[test]
    fn cmp_total_decimal_vs_double_refines_the_f64_tie() {
        use Ordering::*;
        let dec = Num::Dec(Dec { mant: 1, scale: 1 });
        let dbl = Num::Double(0.1);
        assert_eq!(dec.cmp_total(dbl), Less);
        assert_eq!(dbl.cmp_total(dec), Greater);
        assert_eq!(Num::Int(1).cmp_total(Num::Double(f64::INFINITY)), Less);
        assert_eq!(Num::Int(1).cmp_total(Num::Double(f64::NEG_INFINITY)), Greater);
        // Both-inexact: the value IS the f64; a float widens exactly.
        assert_eq!(Num::Float(0.5).cmp_total(Num::Double(0.5)), Equal);
        assert_eq!(Num::Double(-0.0).cmp_total(Num::Double(0.0)), Equal);
    }

    /// Direct pin of `cmp_plain_decimal`: arbitrary-length string-exact decimals,
    /// signed-zero equality, malformed inputs.
    #[test]
    fn cmp_plain_decimal_is_exact_and_total_on_decimals() {
        use Ordering::*;
        assert_eq!(cmp_plain_decimal("9007199254740992", "9007199254740993"), Some(Less));
        assert_eq!(cmp_plain_decimal("0.123456789012345678", "0.123456789012345679"), Some(Less));
        assert_eq!(cmp_plain_decimal("1.50", "1.5"), Some(Equal));
        assert_eq!(cmp_plain_decimal("-0.0", "0"), Some(Equal));
        assert_eq!(cmp_plain_decimal("-2", "-10"), Some(Greater));
        assert_eq!(cmp_plain_decimal("10", "9"), Some(Greater));
        assert_eq!(cmp_plain_decimal("abc", "1"), None);
    }

    /// Direct pin of `f64_exact_decimal`: exact expansions (not shortest round-trips),
    /// and `None` on the non-finite values.
    #[test]
    fn f64_exact_decimal_is_the_exact_expansion() {
        let exp = f64_exact_decimal(0.1).expect("finite");
        assert!(exp.starts_with("0.1000000000000000055511151231257827"), "got {}", exp);
        assert_eq!(
            cmp_plain_decimal(&f64_exact_decimal(TWO53 as f64).unwrap(), "9007199254740992"),
            Some(Ordering::Equal)
        );
        assert_eq!(f64_exact_decimal(f64::NAN), None);
        assert_eq!(f64_exact_decimal(f64::INFINITY), None);
        assert_eq!(f64_exact_decimal(f64::NEG_INFINITY), None);
    }

    // --- Num::cmp_relational — the SPARQL relational (</>/ =) numeric comparison ---
    // [OPUS-4.8] sq-v5evr: the value-space equality/relational-compare hoist.

    /// Basic ordering: int < dec < float < double ordering by value, and the identity
    /// `cmp_relational(x, x) == Some(Equal)` for the non-NaN variants.
    #[test]
    fn cmp_relational_basic_ordering() {
        use Ordering::*;
        // Exact tier: int vs dec
        assert_eq!(Num::Int(1).cmp_relational(Num::Int(2)), Some(Less));
        assert_eq!(Num::Int(2).cmp_relational(Num::Int(1)), Some(Greater));
        assert_eq!(Num::Int(1).cmp_relational(Num::Int(1)), Some(Equal));
        let dec1 = Num::Dec(Dec { mant: 10, scale: 1 }); // 1.0
        let dec2 = Num::Dec(Dec { mant: 15, scale: 1 }); // 1.5
        assert_eq!(dec1.cmp_relational(dec2), Some(Less));
        assert_eq!(Num::Int(1).cmp_relational(dec1), Some(Equal));
        // Float / double
        assert_eq!(Num::Double(1.0).cmp_relational(Num::Double(2.0)), Some(Less));
        assert_eq!(Num::Float(3.0).cmp_relational(Num::Float(3.0)), Some(Equal));
    }

    /// NaN operands produce `None` (SPARQL type error) — unlike `cmp_total` which
    /// totalises NaN first. This pin distinguishes `cmp_relational` from `cmp_total`.
    #[test]
    fn cmp_relational_nan_is_type_error() {
        let nan = Num::Double(f64::NAN);
        let fnan = Num::Float(f32::NAN);
        assert_eq!(nan.cmp_relational(Num::Int(0)), None, "NaN vs int is type error");
        assert_eq!(Num::Int(0).cmp_relational(nan), None, "int vs NaN is type error");
        assert_eq!(nan.cmp_relational(nan), None, "NaN vs NaN is type error");
        assert_eq!(fnan.cmp_relational(Num::Double(1.0)), None, "float-NaN is type error");
    }

    /// The XPath relational compare uses f64 promotion for mixed/inexact pairs — it does
    /// NOT refine the f64 collapse at 2^53 (unlike `cmp_total`). Two integers that
    /// collapse to the same f64 compare Equal under relational semantics (XPath spec).
    #[test]
    fn cmp_relational_mixed_tier_uses_f64_promotion() {
        use Ordering::*;
        // 2^53 and 2^53+1 collapse to the same f64 when compared as int vs double.
        let int_lo = Num::Int(TWO53);
        let dbl = Num::Double(TWO53 as f64);
        // Int vs Int: exact dec path — distinct
        let int_hi = Num::Dec(Dec { mant: TWO53 as i128 + 1, scale: 0 });
        assert_eq!(int_lo.cmp_relational(int_hi), Some(Less));
        // Int vs Double: f64 promotion — the double IS 2^53 exactly, so Equal
        assert_eq!(int_lo.cmp_relational(dbl), Some(Equal));
        // ±0.0 are equal (f64 comparison)
        assert_eq!(Num::Double(-0.0).cmp_relational(Num::Double(0.0)), Some(Equal));
    }

    /// XPath promotes a mixed numeric pair to the LEAST common type, NOT always to
    /// `xs:double`. The hierarchy is `xs:integer -> xs:decimal -> xs:float -> xs:double`
    /// (F&O "Operator Mapping": if one operand is `xs:double` the other becomes double;
    /// OTHERWISE if one is `xs:float` the other becomes FLOAT). So integer/decimal versus
    /// `xs:float` must be decided in the FLOAT tier — comparing it as `f64` is wrong, and
    /// wrong in a way that is invisible below 2^53.
    ///
    /// `cmp_relational` used to fall through to `f64` unconditionally for every mixed pair.
    /// The sibling test above only ever exercises integer-versus-DOUBLE, where `f64` is
    /// genuinely the right tier, so it could not see this.
    ///
    /// Fixture: the `f32` nearest `4611686293305294849` is exactly `0x5E80_0001`
    /// (= 2^62 + 2^39), so under correct float-tier promotion the two are EQUAL, while the
    /// f64 route makes the integer strictly Less. [OPUS-5] issue #3796
    #[test]
    fn cmp_relational_integer_and_decimal_vs_float_compare_in_the_float_tier() {
        use Ordering::*;
        const N: i64 = 4_611_686_293_305_294_849;
        let n = Num::Int(N);
        let f = Num::Float(f32::from_bits(0x5E80_0001));
        assert_eq!(n.f32().to_bits(), 0x5E80_0001, "promotion fixture drifted");

        // THE case: least common type is xs:float, so these are equal.
        assert_eq!(n.cmp_relational(f), Some(Equal), "integer vs float must promote to f32");
        assert_eq!(f.cmp_relational(n), Some(Equal), "and must be symmetric");

        // Pin the wrong answer the f64 route produces, so the fixture cannot silently
        // stop witnessing the defect.
        assert_eq!(n.f64().partial_cmp(&f.f64()), Some(Less), "f64 route is the bug");

        // Ordering against the ADJACENT floats must still be strict, so an
        // everything-is-Equal implementation cannot pass.
        assert_eq!(n.cmp_relational(Num::Float(f32::from_bits(0x5E80_0000))), Some(Greater));
        assert_eq!(n.cmp_relational(Num::Float(f32::from_bits(0x5E80_0002))), Some(Less));

        // xsd:decimal versus xsd:float takes the same tier (decimal -> float).
        let d = Num::Dec(Dec { mant: 46_116_862_933_052_948_490, scale: 1 });
        assert_eq!(d.cmp_relational(f), Some(Equal), "decimal vs float must promote to f32");

        // A DOUBLE operand genuinely does promote the pair to f64 — unchanged behaviour.
        let dbl = Num::Double(f32::from_bits(0x5E80_0001) as f64);
        assert_eq!(n.cmp_relational(dbl), Some(Less), "double operand still uses the f64 tier");

        // Float-tier NaN is still a type error, and same-tier float ordering is intact.
        assert_eq!(n.cmp_relational(Num::Float(f32::NAN)), None);
        assert_eq!(Num::Float(1.5).cmp_relational(Num::Int(2)), Some(Less));
        assert_eq!(Num::Float(-0.0).cmp_relational(Num::Int(0)), Some(Equal));
    }

    // [SONNET-4.6] sq-qcnn.40 — targeted tests killing the surviving mutants identified
    // from nightly run 28776460517. Each asserts an EXACT value so a mutation of the
    // comparison / arithmetic / sign logic goes red.

    /// `Dec::parse` must preserve the negative sign of the input lexical.
    /// Kills: `59:35 delete - in Dec::parse` (removes the `-mag` negation, making
    /// negative parses return a positive mantissa).
    #[test]
    fn dec_parse_negative_mantissa_sign() {
        assert_eq!(Dec::parse("-3"), Some(Dec { mant: -3, scale: 0 }));
        assert_eq!(Dec::parse("-1.5"), Some(Dec { mant: -15, scale: 1 }));
        assert_eq!(Dec::parse("-0.07"), Some(Dec { mant: -7, scale: 2 }));
    }

    /// `Dec::checked_div` with two negative operands must produce a positive result.
    /// Kills: `129:46 replace < with == in Dec::checked_div` (changes `o.mant < 0`
    /// to `o.mant == 0`; since the divisor is asserted non-zero, this makes the sign
    /// depend only on the dividend, giving a wrong negative result for neg/neg).
    #[test]
    fn dec_checked_div_neg_divided_by_neg_is_positive() {
        // −6 / −3 = 2.0 (positive, scale 1).
        let r = Dec { mant: -6, scale: 0 }.checked_div(Dec { mant: -3, scale: 0 });
        assert_eq!(r, Some(Dec { mant: 20, scale: 1 }));
        // −4 / −2 = 2.0.
        let r2 = Dec { mant: -4, scale: 0 }.checked_div(Dec { mant: -2, scale: 0 });
        assert_eq!(r2, Some(Dec { mant: 20, scale: 1 }));
    }

    /// When the divisor has a non-zero scale the exponent `e = s + o.scale - self.scale`
    /// uses `+` for `o.scale`; mutating that `+` to `-` gives the wrong exponent.
    /// Kills: `134:30 replace + with - in Dec::checked_div`.
    #[test]
    fn dec_checked_div_nonzero_divisor_scale() {
        // 10 / 0.5 = 20.0: o.scale=1, so e = s + 1 - 0 = s+1. At s=1: e=2, num=1000, den=5.
        // Under mutation (−): e = s − 1 − 0 = s−1. At s=1: e=0, num=10, den=5 → 2.0. Wrong.
        assert_eq!(
            Dec { mant: 10, scale: 0 }.checked_div(Dec { mant: 5, scale: 1 }),
            Some(Dec { mant: 200, scale: 1 })
        );
        // 3 / 0.3 = 10.0: o.scale=1. At s=1: e=2, num=300, den=3 → 100 → scale=1 → 10.0.
        assert_eq!(
            Dec { mant: 3, scale: 0 }.checked_div(Dec { mant: 3, scale: 1 }),
            Some(Dec { mant: 100, scale: 1 })
        );
    }

    /// Non-terminating division (1/3) rounds half-up: the quotient at max scale 18 is
    /// 0.333...3 with the last digit 3 (rounds DOWN from 0.333...33…, rem=1, 1*2<3).
    /// Non-terminating 2/3 rounds UP (rem=2, 2*2=4 ≥ 3), giving 0.666...7.
    /// Kills: `151:27 replace + with - in Dec::checked_div` (rounds DOWN instead of UP)
    ///        `151:50 replace * with / in Dec::checked_div` (rem/2 never ≥ den — no round).
    #[test]
    fn dec_checked_div_nonterminating_rounds_half_up() {
        // 2/3: non-terminating at every scale. Round half-up at scale 18.
        // rem = (2 * 10^18) % 3 = 2.  2*2=4 ≥ 3 → round UP.
        let two_thirds = Dec { mant: 2, scale: 0 }.checked_div(Dec { mant: 3, scale: 0 }).unwrap();
        assert_eq!(two_thirds.scale, 18);
        assert_eq!(two_thirds.mant, 666_666_666_666_666_667i128);
        // 1/3: rem = (1 * 10^18) % 3 = 1.  1*2=2 < 3 → round DOWN (no increment).
        let one_third = Dec { mant: 1, scale: 0 }.checked_div(Dec { mant: 3, scale: 0 }).unwrap();
        assert_eq!(one_third.scale, 18);
        assert_eq!(one_third.mant, 333_333_333_333_333_333i128);
    }

    /// The non-terminating path preserves the negative sign of the result.
    /// Kills: `153:35 delete - in Dec::checked_div` (removes the `-mant` negation in the
    /// non-terminating branch, returning a positive mantissa for a negative quotient).
    #[test]
    fn dec_checked_div_nonterminating_negative_result() {
        // −2/3 is non-terminating; result must have a negative mantissa.
        let r = Dec { mant: -2, scale: 0 }.checked_div(Dec { mant: 3, scale: 0 }).unwrap();
        assert_eq!(r.scale, 18);
        assert_eq!(r.mant, -666_666_666_666_666_667i128);
    }

    /// `Dec::round_to_int` Ceil of an exact decimal (no remainder) must NOT add 1.
    /// The condition `r > 0` must be STRICT; `>= 0` would always round up since rem_euclid ≥ 0.
    /// Kills: `174:57 replace > with >= in Dec::round_to_int`.
    #[test]
    fn dec_round_to_int_ceil_of_exact_decimal_is_identity() {
        // 3.0 is already an integer — ceil must stay 3, not become 4.
        assert_eq!(
            Dec { mant: 30, scale: 1 }.round_to_int(RoundMode::Ceil),
            Dec { mant: 3, scale: 0 }
        );
        // 7.0 with a different scale.
        assert_eq!(
            Dec { mant: 700, scale: 2 }.round_to_int(RoundMode::Ceil),
            Dec { mant: 7, scale: 0 }
        );
        // Negative exact decimal: ceil(-3.0) = -3, not -2.
        assert_eq!(
            Dec { mant: -30, scale: 1 }.round_to_int(RoundMode::Ceil),
            Dec { mant: -3, scale: 0 }
        );
    }

    /// `cmp_plain_decimal` must treat a negative non-zero value as Less than its
    /// positive counterpart (same magnitude). Tests the sign detection path.
    /// Kills:
    ///   `254:32 replace && with || in cmp_plain_decimal` — `||` makes any number whose
    ///     fraction part is empty (all integers) appear to be "zero", stripping the sign.
    ///   `273:23 delete ! in cmp_plain_decimal` — removes the `!` from `na && !a_zero`,
    ///     meaning neg_a is only true when the value IS zero (inverting the sign logic).
    #[test]
    fn cmp_plain_decimal_negative_vs_positive_same_magnitude() {
        use Ordering::*;
        assert_eq!(cmp_plain_decimal("-2", "2"), Some(Less));
        assert_eq!(cmp_plain_decimal("2", "-2"), Some(Greater));
        // Fraction form: -0.5 < 0.5.
        assert_eq!(cmp_plain_decimal("-0.5", "0.5"), Some(Less));
    }

    /// Zero compared to a positive value must be Less, not Equal.
    /// Kills: `256:15 replace && with || in cmp_plain_decimal` (the `a_zero || b_zero`
    /// mutation returns Equal as soon as EITHER operand is zero, even when the other is not).
    #[test]
    fn cmp_plain_decimal_zero_vs_nonzero() {
        use Ordering::*;
        assert_eq!(cmp_plain_decimal("0", "5"), Some(Less));
        assert_eq!(cmp_plain_decimal("5", "0"), Some(Greater));
        assert_eq!(cmp_plain_decimal("-0", "3"), Some(Less));
    }

    /// When `Dec::checked_div` overflows the exact `i128` path it falls back to
    /// `self.f64() / o.f64()` (division), not multiplication or remainder.
    /// Kills: `486:53 replace / with * in Num::binop`
    ///        `486:53 replace / with % in Num::binop`.
    #[test]
    fn num_binop_div_overflow_fallback_is_double_division() {
        // i128::MAX / 0.1 overflows the exact Dec path (numerator would be ~1.7e40,
        // exceeding u128::MAX), so falls back to Double.
        // Expected: ~1.7e39. Mutation *: ~1.7e37. Mutation %: tiny value near 0.
        let big = Num::Dec(Dec { mant: i128::MAX, scale: 0 });
        let small = Num::Dec(Dec { mant: 1, scale: 1 }); // 0.1
        let result = big.binop(small, ArithOp::Div).expect("never a type error here");
        assert!(
            matches!(result, Num::Double(d) if d > 1e38),
            "overflow fallback must use f64 DIVISION (got {:?})",
            result
        );
    }

    /// `fmt_xsd_double` uses `< 1e15` (strict) for the integer-plain path; the
    /// boundary value 1e15 itself must render as scientific notation, not as a plain integer.
    /// Kills: `794:36 replace < with <= in fmt_xsd_double`.
    #[test]
    fn fmt_xsd_double_boundary_at_1e15_is_scientific() {
        // 1e15.abs() < 1e15 is false → scientific "1.0E15".
        // Under mutation (<= 1e15) it would be true → integer "1000000000000000".
        assert_eq!(fmt_xsd_double(1e15), "1.0E15");
        // 9.99e14 is strictly below 1e15 → still integer form.
        assert_eq!(fmt_xsd_double(999_000_000_000_000.0_f64), "999000000000000");
    }

    /// `Num::lexical` for `Float` uses `f.abs() < 1e15` (strict); the nearest f32 to
    /// 1e15 (`999_999_986_991_104.0_f32`) sits exactly AT the boundary — it must use
    /// scientific notation, not integer form.
    /// Kills: `610:55 replace < with <= in Num::lexical`.
    #[test]
    fn num_lexical_float_at_1e15_f32_boundary_is_scientific() {
        // 1e15_f32 rounds to 999_999_986_991_104.0.  Its abs() is NOT strictly less
        // than itself, so the original `< 1e15_f32` is false → scientific.
        // Under mutation (`<=`), the condition is true → integer form (no 'E').
        let at_boundary: f32 = 1e15_f32; // = 999_999_986_991_104.0
        assert!(
            Num::Float(at_boundary).lexical().contains('E'),
            "float at 1e15_f32 boundary must render scientific; got {:?}",
            Num::Float(at_boundary).lexical()
        );
    }

    /// [FABLE-5] (sq-9781x / sq-74oy4 / sq-6b1lj) TRUE cross-seam differential: the sparq-core
    /// numeric-value CACHE acceptance (`sparq_core::numeric_cache_value` — the EXACT acceptance
    /// `numerics_of`/`numeric_of`/`dictspill` compute, NOT a re-implementation, so this is not
    /// circular) vs the datatype-AWARE evaluator `as_numeric` (`Num::of_literal`, which decides
    /// `values_equal`). As of sq-6b1lj the cache is DATATYPE-AWARE, so the two seams AGREE for
    /// every case: a lexical ill-formed FOR its datatype (`"1.5"^^xsd:integer`,
    /// `"1E2"^^xsd:decimal`, an i128-overflow decimal) MISSES the cache exactly as `of_literal`
    /// type-errors it. This test now pins that agreement: any NEW divergence — the cache
    /// admitting a lexical `of_literal` rejects, or rejecting one it accepts — fails here.
    #[test]
    fn cache_f64_seam_vs_as_numeric_differential() {
        // (lexical, datatype). Post sq-6b1lj the cache acceptance ⟺ `Num::of_literal` acceptance
        // (modulo the genuine-NaN-double sentinel, which both treat as a cache miss), so EVERY
        // case must AGREE — the divergence surface is closed.
        let cases: &[(&str, oxrdf::NamedNodeRef<'_>)] = &[
            // ---- both accept (well-formed for datatype), same f64 image ----
            ("42", xsd::INTEGER),
            (" 7 ", xsd::DECIMAL),      // whitespace collapse — both trim
            ("+3", xsd::INTEGER),
            (" 1 ", xsd::INTEGER),      // padded integer: both value-1 (sq-74oy4)
            ("1.5", xsd::DECIMAL),
            ("1.5E2", xsd::DOUBLE),
            ("INF", xsd::DOUBLE),
            ("3.0", xsd::FLOAT),
            // [GPT-6] Both reject decimal notation for an integer datatype.
            ("5.", xsd::INTEGER),
            ("5.0", xsd::INTEGER),
            ("5.00", xsd::INTEGER),
            ("-0", xsd::INTEGER),        // signed zero
            (".5", xsd::DECIMAL),        // empty integer part
            ("5.5", xsd::DECIMAL),       // ordinary decimal
            ("007.50", xsd::DECIMAL),    // leading/trailing zeros
            // ---- both reject (XSD f64 spellings) ----
            ("inf", xsd::DOUBLE),       // Rust-only spelling: both reject
            ("nan", xsd::DOUBLE),
            ("abc", xsd::INTEGER),
            ("", xsd::INTEGER),
            // ---- both reject (per-datatype ill-formed) — the sq-6b1lj cases, now CLOSED ----
            ("1.5", xsd::INTEGER),       // fraction on an integer (scale 1)
            (".5", xsd::INTEGER),        // no integer part, scale 1 -> not an integer
            ("1E2", xsd::DECIMAL),       // exponent on a decimal
            ("1E2", xsd::INTEGER),       // exponent on an integer
            // 40-digit decimal: beyond i128 -> of_literal None AND cache miss now.
            ("9999999999999999999999999999999999999999.5", xsd::DECIMAL),
            // 40-digit integer: beyond i128 -> of_literal None AND cache miss now.
            ("9999999999999999999999999999999999999999", xsd::INTEGER),
            // 39-digit integer that FITS i128 (i128::MAX ~1.7e38): both ACCEPT (large int).
            ("123456789012345678901234567890123456789", xsd::INTEGER),
        ];
        for (lex, dt) in cases {
            // CACHE seam: the ACTUAL cache acceptance (public wrapper over the fn `numerics_of`/
            // `numeric_of`/`dictspill` call). NaN-double folds to `None` (cache-miss sentinel).
            let cache_hit = sparq_core::numeric_cache_value(lex, dt.as_str()).is_some();
            // EVALUATOR seam: datatype-aware acceptance (NaN-double excluded symmetrically —
            // it is "accepted" as a term but is the cache's not-cached sentinel).
            let eval_accepts = matches!(as_numeric(&typed(lex, *dt)), Some(n) if !n.f64().is_nan());
            assert_eq!(
                cache_hit, eval_accepts,
                "cache seam vs as_numeric MUST agree for {:?}^^{:?}: cache_hit={} eval_accepts={}. \
                 The datatype-aware cache (sq-6b1lj) means cache acceptance ⟺ of_literal acceptance; \
                 a mismatch is a regression of that invariant.",
                lex, dt.as_str(), cache_hit, eval_accepts
            );
            // Where BOTH accept, the f64 images MUST be VALUE-equal (the cache stores that f64).
            // Compared by `==` not bits: `"-0"^^xsd:integer` caches `parse_xsd_f64("-0") = -0.0`
            // while `of_literal` yields `Int(0).f64() = +0.0` — the SAME value (`-0.0 == 0.0`),
            // and every consumer canonicalises signed zero (the `JKey::Num` path folds `-0.0`
            // into `+0.0`) or compares by value. That signed-zero bit-difference is the only
            // permitted one; any real value drift still fails here.
            if cache_hit && eval_accepts {
                let cache_f64 = sparq_core::numeric_cache_value(lex, dt.as_str()).unwrap();
                let eval_f64 = as_numeric(&typed(lex, *dt)).unwrap().f64();
                assert!(
                    cache_f64 == eval_f64 || (cache_f64 == 0.0 && eval_f64 == 0.0),
                    "f64 image mismatch for {:?}^^{:?}: cache={} eval={}",
                    lex, dt.as_str(), cache_f64, eval_f64
                );
            }
        }
    }
}
