//! The shared SPARQL **term total order** — the `ORDER BY` / `MIN`/`MAX`-fallback
//! lenient comparison and the recursive RDF-1.2 triple-term ordering.
//!
//! This is the SPARQL engine's `compare_values` total order, **moved here** from
//! `sparq-engine::exec`, generalised over a tiny [`CompareTerm`] trait rather than
//! the engine's private `Value` enum + `Term` / `Num` / `Timeline` subsystem. The
//! engine implements [`CompareTerm`] for its `Value` (the impl is a set of zero-cost
//! wrappers over its existing `value_str` / numeric / strict-typed-compare helpers)
//! and calls [`compare_terms`]; a reasoner that materialises ordered solutions (RIF
//! `order`, an EL/QL `ORDER BY` over an entailed answer set) can implement the SAME
//! trait for its own term representation and reuse this body. Neither consumer pays a
//! vtable (`research/shared-eval-substrate.md` §2.1, §2.3 — Phase 4 of epic sq-qonbz).
//!
//! # What lives here vs. stays engine-private
//!
//! The substrate holds the **ordering ALGORITHM**: the SPARQL class precedence
//! (error/unbound < blank < IRI < literal < triple-term), the **kind-first** literal
//! order (a fixed [`LiteralKind`] precedence between literal kinds — the sq-wjl8i
//! total-order fix — with the numeric / strict-typed / string-fallback arms WITHIN a
//! kind), and the recursive component-wise triple-term order. It deliberately does **not** hoist the
//! engine's `Value` enum, its `LitKind` literal-family classifier, or its
//! `value_compare_strict` typed/temporal comparison: those are reused by the engine's
//! relational `<` / `>` / `=` operators too (not only the ORDER BY total order), and
//! are tightly coupled to `oxrdf::Term`, so they stay engine-resident and are surfaced
//! to this algorithm through the trait's [`strict_cmp`](CompareTerm::strict_cmp) /
//! [`as_f64`](CompareTerm::as_f64) hooks. This is the same "generic seam" the `join`
//! kernels use for `JoinKeys` and `numeric` uses for `Num`: the shared *algorithm* is
//! generic over a small abstraction the consumer implements, monomorphised at the call
//! site.
//!
//! # Zero-overhead intent (the load-bearing contract)
//!
//! [`compare_terms`] is generic over `T: CompareTerm` — a **generic type parameter**,
//! NOT a trait object. There is NO `Box<dyn>` / `&dyn` / vtable between the algorithm
//! and the term observations it makes; the compiler emits one specialised, inlinable
//! body per call site. Every item carries `#[inline]`; the structural `no-dyn-dispatch`
//! gate (`scripts/check-no-dyn-dispatch.py`) lists this file in its hot-path set. The
//! original extraction (sq-vezew) was behaviour-identical to the engine's
//! `compare_values`; sq-wjl8i then DELIBERATELY changed the order itself (kind-first
//! literals, exact mixed-tier ties, NaN totalised) to fix three machine-checked
//! intransitivity witnesses — see [`compare_terms`] for what is spec-mandated vs. a
//! documented extension. The fast paths (class ranks, the numeric `partial_cmp`) are
//! unchanged; the exact/lexical work still happens only on cold tie/fallback branches.

use std::cmp::Ordering;

/// The SPARQL total-order **class** of a term: the top-level precedence buckets the
/// `ORDER BY` order ranks by, *before* any within-class value comparison.
///
/// SPARQL 1.1 orders `unbound < blank node < IRI < literal`; the SPARQL 1.2 total-order
/// extension places **triple terms after literals**. The discriminants are the exact
/// ranks the engine's `compare_values` used (`error`/`unbound` = 0, blank = 1, IRI = 2,
/// literal = 3, triple = 4), so cross-class ordering is byte-identical to pre-move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TermClass {
    /// An unbound variable or a SPARQL type error — sorts first.
    ErrorOrUnbound = 0,
    /// A blank node.
    Blank = 1,
    /// An IRI (named node).
    Iri = 2,
    /// Any literal (including a computed numeric / boolean).
    Literal = 3,
    /// An RDF-1.2 quoted triple term — sorts after literals.
    Triple = 4,
}

/// The within-[`Literal`](TermClass::Literal)-class **KIND rank** of the total order —
/// the fixed precedence [`compare_terms`] applies BETWEEN literal kinds, before any
/// within-kind value comparison. sq-wjl8i
///
/// SPARQL 1.1 §15.1 fixes the cross-CLASS order (unbound < blank < IRI < literal) and,
/// via the `<` operator, the within-kind orders (numerics by value, strings lexically,
/// booleans, dateTimes by timeline); it leaves CROSS-KIND literal order (a number
/// against a plain string, …) undefined. This rank is sparq's total-order EXTENSION for
/// that undefined region: cross-kind pairs order by kind rank ALWAYS — never by a value
/// or lexical coercion. (The previous lexical cross-kind fallback made the comparator
/// intransitive: lexically `"10" < "11" < "2"` while numerically `10 > 2` — the
/// machine-checked witness of bead sq-wjl8i.) The rank values are a documented
/// implementation choice, not a spec claim.
///
/// A term's kind must agree with the observations the within-kind arms use — see the
/// [`CompareTerm::literal_kind`] contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LiteralKind {
    /// A numeric literal (or computed numeric) with a lenient `f64` view — orders by
    /// exact numeric value, `NaN` first (see [`compare_terms`]).
    Numeric = 0,
    /// An `xsd:boolean` literal or computed boolean — `false < true`; ill-formed
    /// boolean lexicals order lexically (consistently: `"false" < "true"`).
    Boolean = 1,
    /// A well-formed `xsd:dateTime` / `xsd:dateTimeStamp` — orders by timeline.
    DateTime = 2,
    /// A well-formed `xsd:date` — orders by timeline (midnight).
    Date = 3,
    /// A plain / `xsd:string` literal — orders lexically.
    String = 4,
    /// A language-tagged string — orders by lexical value (same-tag pairs strictly,
    /// cross-tag pairs by the same lexical value via the string fallback).
    Lang = 5,
    /// Everything else — other XSD datatypes, unknown datatypes, and ILL-FORMED
    /// numeric/temporal lexicals (which must not sit in a value-ordered kind: mixing a
    /// value order with a lexical fallback inside one kind is exactly the
    /// intransitivity this rank exists to remove) — orders lexically.
    Other = 6,
}

/// The minimal observation surface [`compare_terms`] needs from a term to compute the
/// SPARQL total order, **without** the substrate knowing about the engine's `Value`
/// enum, `oxrdf::Term`, or its temporal subsystem.
///
/// The engine implements this for its `Value` as zero-cost wrappers over its existing
/// helpers (`value_str`, the `as_num` f64 coercion, `value_compare_strict`, and the
/// triple decomposition). A reasoner implements it for its own ordered-term type. The
/// trait is a **monomorphisation seam**, never used as `dyn CompareTerm` on a hot path.
pub trait CompareTerm: Sized {
    /// The term's top-level SPARQL ordering class (the cross-class precedence bucket).
    fn term_class(&self) -> TermClass;

    /// The literal's [`LiteralKind`] — the within-Literal-class precedence bucket of
    /// the kind-first total order. Only consulted when BOTH terms are
    /// [`TermClass::Literal`]; the value for non-literal terms is irrelevant (return
    /// [`LiteralKind::Other`]). sq-wjl8i
    ///
    /// # Contract (what keeps the total order lawful)
    ///
    /// - [`Numeric`](LiteralKind::Numeric) **iff** [`as_f64`](Self::as_f64) returns
    ///   `Some` — with one exception: a computed boolean whose lenient `f64` view is
    ///   0.0/1.0 classifies as [`Boolean`](LiteralKind::Boolean) (the numeric arm is
    ///   gated on the KIND, so booleans still order `false < true` via
    ///   [`strict_cmp`](Self::strict_cmp)).
    /// - An ILL-FORMED numeric / temporal lexical must classify as
    ///   [`Other`](LiteralKind::Other), not into its value-ordered kind: a kind that
    ///   mixes value-ordered pairs with lexical-fallback pairs is intransitive (the
    ///   sq-wjl8i witnesses).
    /// - Within each kind, the arms the algorithm applies ([`strict_cmp`](Self::strict_cmp)
    ///   where it decides, else the lexical [`value_str`](Self::value_str) fallback)
    ///   must agree wherever both decide a pair.
    fn literal_kind(&self) -> LiteralKind;

    /// The term's lexical string form for the deterministic within-class fallback
    /// (the literal string arm, and the blank-node / IRI comparison): the literal
    /// lexical value, the IRI string, or the blank-node label. `None` for an
    /// unbound / error term (which never reaches a within-class string compare).
    ///
    /// This is the engine's `value_str`: the same form ORDER BY used to break ties
    /// across mixed literal datatypes, so the total order stays deterministic.
    fn value_str(&self) -> Option<String>;

    /// The term's numeric value as `f64` for the **numeric-aware** literal arm — the
    /// lenient ORDER BY coercion that lets `"1"^^xsd:integer` and `"1.5"^^xsd:double`
    /// order by value across the numeric tower. `None` for a non-numeric literal.
    ///
    /// This mirrors the engine's `as_num`: numerics (and booleans, as 0.0/1.0) order
    /// by value, everything else falls through to [`strict_cmp`](Self::strict_cmp)
    /// then the string form.
    fn as_f64(&self) -> Option<f64>;

    /// An EXACT recheck of two **numeric** literals for the f64-collapse case. The lenient
    /// numeric arm coerces both operands to `f64` (see [`as_f64`](Self::as_f64)); f64
    /// rounding is MONOTONIC, so it can only COLLAPSE two distinct numeric values to Equal,
    /// never flip a `Less`/`Greater`. So [`compare_terms`] calls this **only** when the f64
    /// arm reports Equal, to recover the true order for values that share one f64 image.
    ///
    /// Returns `Some(ordering)` when the pair's EXACT-RATIONAL value order is
    /// computable — including the MIXED exact/inexact pair (an `xsd:integer` /
    /// `xsd:decimal` against an `xsd:float` / `xsd:double`, whose value is an exact
    /// rational too): distinct integers beyond 2^53, high-precision decimals sharing
    /// one f64, and an exact value against the float/double it collapses onto must ALL
    /// order exactly, or the refined tie relation is not an equivalence and the order
    /// is intransitive (witness 1 of bead sq-wjl8i — the previous "`None` on mixed
    /// pairs, engine falls back to the collapsed f64" behaviour). The substrate's
    /// `Num::cmp_total` (`numeric` feature) implements exactly this order. Returns
    /// `None` when the exact order cannot be computed (e.g. a lexical beyond the exact
    /// tower) — the collapsed f64 tie then stands, for EVERY member of that tie class.
    /// A purely symbolic consumer with no exact numeric tier returns `None`.
    ///
    /// The engine's relational `=`/`<` keep the XPath PROMOTED semantics (a
    /// float/double operand promotes the pair to f64, so `2^53+1 = 2^53E0` is true
    /// there); this total order deliberately REFINES those promoted ties — every strict
    /// promoted verdict is preserved (rounding is monotonic), only ties are split.
    /// sq-rikm7 / sq-wjl8i
    fn exact_cmp(&self, other: &Self) -> Option<Ordering>;

    /// The **strict** value comparison for same-family typed literals the numeric arm
    /// cannot decide — principally `xsd:dateTime` / `xsd:date` by timeline, and the
    /// lenient same-tag / same-other-XSD lexical orders. `Some(ordering)` only when the
    /// operands are value-comparable; `None` when they are not (the algorithm then
    /// falls back to the string form, exactly as `compare_values` did).
    ///
    /// This is the engine's `value_compare_strict`; it stays engine-resident because it
    /// is also driven by the relational `<` / `>` operators and is coupled to `oxrdf`.
    ///
    /// # Contract: TOTALITY within the temporal kinds — sq-2k5py
    ///
    /// For a pair that BOTH classify as [`DateTime`](LiteralKind::DateTime) (or both as
    /// [`Date`](LiteralKind::Date)) this must return `Some`: the temporal kinds admit **no**
    /// string fallback. The relational operators' comparison is deliberately PARTIAL — a
    /// tz-less dateTime against a tz-carrying one inside XPath's ±14h window is
    /// indeterminate (a type error) — but routing those pairs to the lexical fallback mixes
    /// timeline-decided and lexical-decided pairs inside ONE kind, which is the same
    /// intransitivity shape the sq-wjl8i witnesses had: two zoned dateTimes can be the same
    /// instant (timeline-Equal) while a tz-less one sits lexically BETWEEN them. An
    /// implementation must extend the timeline order to a TOTAL one over that window —
    /// `sparq_core::temporal::Timeline::cmp_tl_total` (instant-assuming-UTC, then timezone
    /// PRESENCE) is the definition both the engine and the reasoner use, and its doc carries
    /// the witness. Extending the total order does NOT change relational semantics.
    ///
    /// The other kinds have no such obligation: the [`String`](LiteralKind::String) /
    /// [`Lang`](LiteralKind::Lang) / [`Other`](LiteralKind::Other) kinds are lexical
    /// EVERYWHERE (a `None` there falls back to the same byte order the arm would give), so
    /// no pair inside one of them mixes two different orders.
    fn strict_cmp(&self, other: &Self) -> Option<Ordering>;

    /// If the term is an RDF-1.2 quoted triple, its `(subject, predicate, object)`
    /// components **as the same term type**, so [`compare_terms`] can recurse through
    /// nesting and order triple terms component-wise under this very order. `None` for
    /// a non-triple term.
    ///
    /// The predicate is surfaced as a term whose [`term_class`](Self::term_class) is
    /// [`TermClass::Iri`], so the generic recursion compares it by IRI string — byte
    /// identical to the engine's pre-move `predicate.as_str().cmp(...)`.
    fn triple_parts(&self) -> Option<[Self; 3]>;
}

/// The SPARQL **total order** for `ORDER BY` (and the `MIN`/`MAX` fallback), generic
/// over any [`CompareTerm`].
///
/// SPARQL orders unbound/error < blank nodes < IRIs < literals < triple terms
/// (spec-fixed cross-class order; triple-terms-after-literals is the SPARQL 1.2
/// extension), then within each class: blanks / IRIs by their string form; literals
/// **kind-first** (see below); triple terms component-wise (subject, then predicate,
/// then object) recursively under this order.
///
/// # The kind-first literal order — sq-wjl8i
///
/// Within the literal class, terms first rank by [`LiteralKind`] (numeric < boolean <
/// dateTime < date < string < language-tagged < other); ONLY same-kind pairs compare by
/// value. Same-kind: the [`Numeric`](LiteralKind::Numeric) kind orders by exact numeric
/// value — the lenient `f64` fast path, with `NaN` totalised FIRST (before `-INF`,
/// `NaN == NaN` — the XPath 3.1 `fn:sort` choice) and an f64 TIE rechecked exactly via
/// [`exact_cmp`](CompareTerm::exact_cmp) (distinct integers beyond 2^53, high-precision
/// decimals, and the mixed exact/inexact pair all order by exact rational value);
/// every other kind orders by [`strict_cmp`](CompareTerm::strict_cmp) where it decides
/// (dateTime/date by timeline, booleans, same-tag / same-other-XSD lexically), else by
/// the lexical [`value_str`](CompareTerm::value_str) fallback.
///
/// That lexical fallback is only lawful for the kinds that are lexical EVERYWHERE. Inside
/// the [`DateTime`](LiteralKind::DateTime) / [`Date`](LiteralKind::Date) kinds it is
/// **not**: mixing timeline-decided and lexical-decided pairs in one kind is intransitive
/// exactly as the cross-kind fallback was, so
/// [`strict_cmp`](CompareTerm::strict_cmp) is contracted to be TOTAL there (its
/// "TOTALITY within the temporal kinds" section carries the witness). sq-2k5py
///
/// Where SPARQL 1.1 §15.1 / the `<` operator define an order (the cross-class ranks;
/// numeric, string, boolean, dateTime same-kind pairs) this order agrees with the spec;
/// the cross-KIND ranking and the NaN / exact-tie refinements are documented
/// extensions in the region the spec leaves undefined (see [`LiteralKind`]).
///
/// Returns `None` only when (a) a within-class string fallback is needed but a term has
/// no string form (an unbound/error reaching a literal compare), or (b) a
/// [`TermClass::Triple`]-classed term yields no [`triple_parts`](CompareTerm::triple_parts).
/// A `NaN` operand no longer yields `None` (the sq-ma9fb doc-drift, fixed with the
/// behaviour): callers mapping `None` to `Ordering::Equal` no longer make `NaN` "equal"
/// to every numeric.
#[inline]
pub fn compare_terms<T: CompareTerm>(x: &T, y: &T) -> Option<Ordering> {
    let (ca, cb) = (x.term_class(), y.term_class());
    if ca != cb {
        return Some(ca.cmp(&cb));
    }
    match ca {
        TermClass::ErrorOrUnbound => Some(Ordering::Equal),
        TermClass::Blank | TermClass::Iri => Some(x.value_str()?.cmp(&y.value_str()?)),
        // Triple terms order component-wise (subject, predicate, then object under
        // this same total order, recursing through nesting).
        TermClass::Triple => {
            let (Some(a), Some(b)) = (x.triple_parts(), y.triple_parts()) else {
                return None;
            };
            let [as_, ap, ao] = a;
            let [bs, bp, bo] = b;
            let s = compare_terms(&as_, &bs)?;
            if s != Ordering::Equal {
                return Some(s);
            }
            let p = compare_terms(&ap, &bp)?;
            if p != Ordering::Equal {
                return Some(p);
            }
            compare_terms(&ao, &bo)
        }
        TermClass::Literal => {
            // KIND-FIRST: cross-kind pairs rank by LiteralKind, NEVER by a value or
            // lexical coercion — a cross-kind lexical fallback is intransitive against
            // the within-kind value orders (witness 2 of sq-wjl8i: lexically
            // "10" < "11" < "2" while numerically 10 > 2). sq-wjl8i
            let (ka, kb) = (x.literal_kind(), y.literal_kind());
            if ka != kb {
                return Some(ka.cmp(&kb));
            }
            if ka == LiteralKind::Numeric {
                // The literal_kind contract guarantees both f64 views exist here; the
                // `if let` keeps a contract violation falling through defensively.
                if let (Some(a), Some(b)) = (x.as_f64(), y.as_f64()) {
                    return Some(match a.partial_cmp(&b) {
                        // f64-collapse EXACT recheck. f64 rounding is MONOTONIC: it can
                        // only COLLAPSE distinct values to Equal, never flip a strict
                        // verdict. So — and ONLY — on an f64 tie, recheck exactly: the
                        // exact-rational order splits collapsed integers beyond 2^53,
                        // high-precision decimals, AND the mixed exact/inexact pair
                        // (witness 1 of sq-wjl8i). An undecidable recheck keeps the tie.
                        // Perf-neutral: exact work happens only on a tie.
                        // sq-rikm7 / sq-wjl8i
                        Some(Ordering::Equal) => x.exact_cmp(y).unwrap_or(Ordering::Equal),
                        Some(o) => o,
                        // NaN (the only `partial_cmp` None): totalise with NaN FIRST —
                        // before -INF, equal to itself (witness 3 of sq-wjl8i; the
                        // XPath 3.1 `fn:sort` "NaN least" choice). Relational `<`/`=`
                        // keep their SPARQL type-error semantics — only this total
                        // order positions NaN.
                        None => match (a.is_nan(), b.is_nan()) {
                            (true, false) => Ordering::Less,
                            (false, true) => Ordering::Greater,
                            _ => Ordering::Equal,
                        },
                    });
                }
            }
            // Same non-numeric kind: strict value order where decidable (dateTime/date
            // by timeline, booleans, same-tag / same-other-XSD lexically), else the
            // deterministic lexical fallback. The fallback is reachable only for the
            // lexical-everywhere kinds: for the DateTime/Date kinds `strict_cmp` is
            // contracted TOTAL, because a kind mixing timeline-decided and lexical-decided
            // pairs is intransitive. sq-2k5py
            if let Some(o) = x.strict_cmp(y) {
                return Some(o);
            }
            Some(x.value_str()?.cmp(&y.value_str()?))
        }
    }
}

// sq-sqtk2.4 / sq-wjl8i (epic sq-sqtk2, property B-1 of
// `research/mechanized-proof-program.md` §3.2/§5): Kani bounded-proof harnesses for the
// ORDER LAWS of [`compare_terms`] — the laws `ORDER BY`'s `sort_by` validity rests on (an
// inconsistent comparator makes Rust's sort panic or produce garbage). The sq-sqtk2.4 wave
// machine-checked three intransitivity WITNESSES across mixed literal kinds; sq-wjl8i then
// FIXED the order (kind-first literals, exact mixed-tier ties, NaN totalised first) and
// UPGRADED the law set below — the former documented-as-failing mixed-kind transitivity is
// now a PROVED harness, and the witness harnesses pin the FIXED behaviour as regressions.
//
// WHAT IS PROVED (tier: PROVED (bounded), per the design record's vocabulary) — over every
// value of the PER-HARNESS domain, each a stated sub-domain of the bounded model `M` below
// (the harness doc-comment is the authoritative domain statement):
//   • REFLEXIVITY (full domain, NaN INCLUDED):  compare_terms(x, x) == Some(Equal)
//   • ANTISYMMETRY-CONSISTENCY (full domain, NaN included):
//        compare_terms(x, y) == Some(o)  iff  compare_terms(y, x) == Some(o.reverse())
//        (equivalently: None in one direction iff None in the other)
//   • TRANSITIVITY on the defined domain — per literal kind AND ACROSS MIXED LITERAL
//     KINDS (`transitivity_mixed_literal_kinds_incl_nan`, NaN included — the law the
//     sq-sqtk2.4 witnesses proved FALSE before the sq-wjl8i fix): exact-integer literals
//     INCLUDING the 2^53 f64-collapse straddle, double literals, string literals,
//     strict/temporal literals, the full mixed literal-kind domain, a collapse-free
//     exact/inexact numeric mix, recursive triple terms, the non-literal scalar classes,
//     and a REDUCED int-with-non-literal composition (3 straddle ints × 2-string
//     blanks/IRIs × Err — a stated shrink, see its doc)
//   • WITHIN-CLASS TOTALITY (full literal domain, NaN INCLUDED): same-class pairs always
//     compare `Some`
//   • EXACT-ORDER AGREEMENT: for exact-tier integer pairs, compare_terms equals the exact
//     i128 value order even where the f64 images collapse (THE guarantee the `exact_cmp`
//     recheck tier exists to provide — delete the recheck in `compare_terms` and this goes
//     red on the 2^53/2^53+1 pair)
//
// TRACTABILITY DISCIPLINE (why the harnesses are shaped this way): a term whose enum
// DISCRIMINANT is symbolic (built as `let x = match kani::any() { .. => Int, .. => Dbl }`)
// forks CBMC symex at EVERY downstream `match self` in the comparator's trait methods —
// multiplicatively per call, so even a UNARY harness over a handful of kinds does not
// terminate practically. The `for_each_*` kind-dispatch combinators below invert that:
// the symbolic kind CHOICE happens once, in a match whose arms each pass `f` a term with
// a CONCRETE discriminant (only the value index stays symbolic), keeping every dispatch
// leaf linear while the selector still quantifies over all kinds. Additionally, every
// harness carries the SMALLEST `#[kani::unwind]` bound its loops/recursion need (the
// bound also caps the syntactic expansion of `compare_terms`' triple-term recursion,
// which inflates the formula when set high). Under-bounding is LOUD: Kani's unwinding
// assertion fails, so a too-small bound cannot silently weaken a proof.
//
// DOMAIN-COVERAGE SELF-CHECKS (the sq-og8u8 pattern): a re-scoped (shrunk) domain can
// silently drop the very inputs that make a harness adversarial, so each shrunk domain's
// interesting inputs are PINNED by a proof: `domain_exhibits_the_2p53_collapse` pins the
// collapse straddle used by the full `INTS` table, the reduced int-with-non-literal
// domain, and the triple-object domain;
// `domain_cf_numeric_is_collapse_free_with_signed_zero_pair` pins the collapse-free
// mixed range (exact images only) and its ±0.0 equal-but-lexically-distinct pair. If a
// future re-scope silences an interesting input, one of these goes red rather than the
// law harness silently proving less.
//
// HONESTY BOUNDARY (per the bead): this machine-checks the `compare_terms` ALGORITHM over
// the model observation surface `M` below. It is explicitly NOT a claim about the engine's
// `Value` impl of `CompareTerm` in `sparq-engine/src/exec.rs` (that instance stays covered
// by unit + W3C conformance tests; ledgered as the next wave in the design record §6). The
// model is, however, deliberately FAITHFUL to that impl's observation semantics — each trait
// method below documents the `exec.rs` behaviour it mirrors — because a convenient model
// would make these proofs vacuous.
//
// FORMER MACHINE-CHECKED FINDINGS, NOW FIXED (bead sq-wjl8i; the `witness_*` harnesses pin
// the FIXED behaviour and go red on a regression): the sq-sqtk2.4 wave proved the order
// laws did NOT extend across mixed literal KINDS —
//   1. the exact/inexact numeric-tier mix AT the 2^53 collapse boundary was intransitive
//      (the collapsed-f64 fallback tied a double with BOTH straddle integers) — fixed by
//      the exact-rational mixed-tier recheck (`witness_mixed_tier_collapse_now_exact`);
//   2. numeric-vs-plain-string pairs fell to the LEXICAL form, which disagrees with the
//      numeric order (`"10" < "11" < "2"` vs `10 > 2`) — fixed by the kind-first rank:
//      cross-kind pairs order by `LiteralKind`, never by a lexical coercion
//      (`witness_numeric_vs_string_now_ranked_by_kind`);
//   3. NaN made the comparator PARTIAL (`None`, mapped to `Equal` by callers) — fixed by
//      totalising NaN FIRST among numerics (`witness_nan_now_totalised_first`).
// All three were reachable through the engine's real `Value` impl; the engine-side fix
// (`Value::literal_kind`, `Num::cmp_total`, the `cmp_sort_num` sort-cell mirror) is pinned
// by the engine's unit tests and the sparq-reason engine-parity suite.
//
// RUN:  cargo kani -p sparq-substrate --features compare
// (nightly lane wiring is bead sq-sqtk2.5; under normal build/clippy/test this module is
// stripped — the `kani` cfg is registered in this crate's Cargo.toml `[lints.rust]`.)
#[cfg(kani)]
mod kani_proofs {
    use super::*;
    use std::cmp::Ordering;

    /// 2^53 — the f64 integer-exactness boundary: every integer of magnitude ≤ 2^53 has an
    /// exact f64 image; above it, adjacent integers start sharing one image ("collapse").
    const TWO53: i128 = 9_007_199_254_740_992;

    /// Adversarial exact-integer domain (the model's `xsd:integer` tier). Deliberately
    /// STRADDLES the collapse boundary in both signs: `±(2^53 + 1)` are the first
    /// non-representable integers (ties-to-even rounds them onto `±2^53`), while
    /// `2^53 - 1` and `2^53 + 2` are exactly representable neighbours. The small values
    /// `2` and `10` exist for the numeric-vs-lexical witness (`"10" < "11" < "2"`).
    const INTS: [i128; 10] = [
        -(TWO53 + 1), // collapses onto -(2^53)
        -TWO53,
        -2,
        0,
        2,
        10,
        TWO53 - 1, // exactly representable
        TWO53,     // the collapse image
        TWO53 + 1, // NOT representable: rounds onto 2^53 — the collapse
        TWO53 + 2, // exactly representable again
    ];
    // Index names for the values the harnesses pick out explicitly.
    const I_TWO: u8 = 4;
    const I_TEN: u8 = 5;
    const I_2P53_M1: u8 = 6;
    const I_2P53: u8 = 7;
    const I_2P53_P1: u8 = 8;
    const I_2P53_P2: u8 = 9;

    /// The f64 images of `INTS`, computed at COMPILE TIME by the same `as f64`
    /// (round-ties-to-even) conversion the engine's numeric tower applies — so the collapse
    /// is data the solver can see: `INT_F64S[I_2P53] == INT_F64S[I_2P53_P1]`.
    const INT_F64S: [f64; 10] = [
        INTS[0] as f64,
        INTS[1] as f64,
        INTS[2] as f64,
        INTS[3] as f64,
        INTS[4] as f64,
        INTS[5] as f64,
        INTS[6] as f64,
        INTS[7] as f64,
        INTS[8] as f64,
        INTS[9] as f64,
    ];

    /// The decimal lexical forms of `INTS`, index-parallel. COMPILE-TIME constants: the
    /// model must not run an int→string conversion loop under CBMC, and the values are
    /// load-bearing for the lexical-fallback arms (`"10" < "11" < "2"` lexically).
    const INT_STRS: [&str; 10] = [
        "-9007199254740993",
        "-9007199254740992",
        "-2",
        "0",
        "2",
        "10",
        "9007199254740991",
        "9007199254740992",
        "9007199254740993",
        "9007199254740994",
    ];

    /// Adversarial inexact (`xsd:double`) domain: `±0.0` (an equal-but-lexically-distinct
    /// pair), a fraction, and — crucially — `2^53` itself: the shared f64 image of the
    /// collapsed integers, which is exactly where the mixed-tier witness lives.
    const DBLS: [f64; 6] = [
        -9_007_199_254_740_992.0, // -(2^53)
        -0.0,
        0.0,
        0.5,
        9_007_199_254_740_992.0, // 2^53 — the collapse image
        9_007_199_254_740_994.0, // 2^53 + 2
    ];
    const D_2P53: u8 = 4;
    /// Lexical forms of `DBLS` (fixed representative renderings; the laws only need
    /// `value_str` to be a deterministic function of the model value).
    const DBL_STRS: [&str; 6] = [
        "-9.007199254740992E15",
        "-0.0E0",
        "0.0E0",
        "5.0E-1",
        "9.007199254740992E15",
        "9.007199254740994E15",
    ];

    /// TWICE each `DBLS` value, as an exact `i128` — the model's exact-rational tier for
    /// the MIXED exact/inexact comparison (every table double is half-integral, so 2× is
    /// exact; pinned by `domain_x2_doubles_are_exact`). The engine's real mixed-tier
    /// tie-break (`Num::cmp_total` → the exact decimal-string comparison) is modelled as
    /// the i128 order of the doubled values. sq-wjl8i
    const DBL_X2: [i128; 6] = [
        -18_014_398_509_481_984, // -(2^54)
        0,                       // -0.0 — exactly equal to +0.0
        0,
        1, // 0.5
        18_014_398_509_481_984, // 2^54
        18_014_398_509_481_988, // 2^54 + 4
    ];

    /// Blank labels / IRIs / plain-string literal values: tiny but adversarial — the empty
    /// string, a prefix pair (`"a"` < `"ab"`), and `"11"` (lexically ABOVE `"10"` and BELOW
    /// `"2"`: the digit-string inversion the numeric-vs-string witness rides on).
    const STRS: [&str; 4] = ["", "11", "a", "ab"];
    const S_11: u8 = 1;

    /// Strictly-comparable typed-literal tier (models the engine's dateTime/date-by-timeline
    /// arm): `Strict(i)` compares to `Strict(j)` by index. The lexical forms are chosen to
    /// AGREE with that order — the real engine's mixed-timezone dateTimes can order
    /// lexically ≠ timeline, but that pair never reaches the string fallback (same family ⇒
    /// `strict_cmp` decides), so agreement here is not a fidelity loss for these laws.
    const STRICT_STRS: [&str; 3] = ["2024-01-01", "2025-01-01", "2026-01-01"];

    /// The bounded model term. One variant per observation SHAPE the algorithm
    /// distinguishes; every [`TermClass`] is covered. Triple terms are depth-1 (scalar
    /// components) — a stated bound; the recursion body is the same for deeper nesting.
    /// NOT modelled (stated bounds): booleans (an inexact-shaped numeric at 0.0/1.0 —
    /// `Dbl` covers the shape), decimals (same exact tier as `Int` through `exact_cmp`),
    /// and `±INF` doubles (totally ordered f64s like every other non-NaN double).
    enum M {
        /// Unbound / SPARQL type error.
        Err,
        /// Blank node with label `STRS[i]`.
        Blank(u8),
        /// IRI `STRS[i]`.
        Iri(u8),
        /// Exact-tier numeric literal (`xsd:integer`) with value `INTS[i]`: `as_f64` is the
        /// COLLAPSING image, `exact_cmp` is exact — the engine's int/decimal tier.
        Int(u8),
        /// Inexact-tier numeric literal (`xsd:double`) with value `DBLS[i]`: its value IS
        /// its f64; no exact tier.
        Dbl(u8),
        /// The `xsd:double` NaN (reachable in the engine: `parse_xsd_f64` accepts `NaN`).
        Nan,
        /// Plain string literal `STRS[i]`: no numeric / strict tier — lexical fallback.
        Str(u8),
        /// Same-family strictly-comparable typed literal (dateTime-by-timeline model).
        Strict(u8),
        /// Depth-1 RDF-1.2 quoted triple: `(Iri STRS[s], Iri STRS[p], Int INTS[o])`.
        Trip(u8, u8, u8),
    }

    impl M {
        /// The model's `as_numeric` (mirrors `exec.rs`): `(exact_tier, exact_value, f64_image)`.
        /// `exact_value` is meaningful only when `exact_tier` — for the inexact tier the
        /// value IS the image (the engine's `Num::Double`, whose `to_dec()` is `None`).
        fn num(&self) -> Option<(bool, i128, f64)> {
            match self {
                M::Int(i) => Some((true, INTS[usize::from(*i)], INT_F64S[usize::from(*i)])),
                M::Dbl(i) => Some((false, 0, DBLS[usize::from(*i)])),
                M::Nan => Some((false, 0, f64::NAN)),
                _ => None,
            }
        }

        /// TWICE the term's exact rational value, as an exact `i128` — the model's
        /// exact-rational tier for `exact_cmp` (see `DBL_X2`). `None` for NaN and
        /// non-numerics. sq-wjl8i
        fn x2(&self) -> Option<i128> {
            match self {
                M::Int(i) => Some(2 * INTS[usize::from(*i)]),
                M::Dbl(i) => Some(DBL_X2[usize::from(*i)]),
                _ => None,
            }
        }
    }

    impl CompareTerm for M {
        fn term_class(&self) -> TermClass {
            match self {
                M::Err => TermClass::ErrorOrUnbound,
                M::Blank(_) => TermClass::Blank,
                M::Iri(_) => TermClass::Iri,
                M::Int(_) | M::Dbl(_) | M::Nan | M::Str(_) | M::Strict(_) => TermClass::Literal,
                M::Trip(..) => TermClass::Triple,
            }
        }
        fn literal_kind(&self) -> LiteralKind {
            // Engine (`Value::literal_kind`): the lenient numeric family (NaN included —
            // `parse_xsd_f64` accepts it) is Numeric; plain strings String; the strict
            // temporal family DateTime. Non-literals are never consulted.
            match self {
                M::Int(_) | M::Dbl(_) | M::Nan => LiteralKind::Numeric,
                M::Str(_) => LiteralKind::String,
                M::Strict(_) => LiteralKind::DateTime,
                _ => LiteralKind::Other,
            }
        }
        fn value_str(&self) -> Option<String> {
            // Engine (`value_str`): the term's lexical form — a deterministic function of
            // the term. Model: fixed compile-time tables (no conversion loops under CBMC).
            match self {
                M::Err => None,
                M::Blank(i) | M::Iri(i) | M::Str(i) => Some(STRS[usize::from(*i)].to_string()),
                M::Int(i) => Some(INT_STRS[usize::from(*i)].to_string()),
                M::Dbl(i) => Some(DBL_STRS[usize::from(*i)].to_string()),
                M::Nan => Some("NaN".to_string()),
                M::Strict(i) => Some(STRICT_STRS[usize::from(*i)].to_string()),
                // Never observed by `compare_terms` (the Triple arm recurses via
                // `triple_parts` before any string fallback).
                M::Trip(..) => None,
            }
        }
        fn as_f64(&self) -> Option<f64> {
            // Engine (`as_num`): the LENIENT f64 coercion — for an exact integer this is the
            // collapsing `as f64` image; that collapse is the entire point of the domain.
            self.num().map(|(_, _, f)| f)
        }
        fn exact_cmp(&self, other: &Self) -> Option<Ordering> {
            // Engine (`exec.rs` `Value::exact_cmp` → `Num::cmp_total`, the sq-wjl8i fix):
            // the exact-RATIONAL total order over numeric values — exact for int/decimal
            // pairs AND for the mixed exact/inexact pair (a finite double's value is an
            // exact rational; the engine compares against its exact decimal expansion),
            // with NaN totalised first. Model: every domain value doubled is an exact
            // i128 (`x2`, pinned by `domain_x2_doubles_are_exact`), so the exact-rational
            // order IS the i128 order of the doubled values. The NaN arm mirrors
            // `cmp_total` but is unreachable from `compare_terms` (NaN never produces the
            // f64 TIE that triggers the recheck).
            let (_, _, af) = self.num()?;
            let (_, _, bf) = other.num()?;
            match (af.is_nan(), bf.is_nan()) {
                (true, true) => return Some(Ordering::Equal),
                (true, false) => return Some(Ordering::Less),
                (false, true) => return Some(Ordering::Greater),
                (false, false) => {}
            }
            match (self.x2(), other.x2()) {
                (Some(a), Some(b)) => Some(a.cmp(&b)),
                _ => None,
            }
        }
        fn strict_cmp(&self, other: &Self) -> Option<Ordering> {
            // Engine (`value_compare_strict`): decides SAME-FAMILY pairs (dateTime/date by
            // timeline, same-tag/same-other-XSD lexically); every cross-family pair is
            // `None` ⇒ the caller falls back to the lexical string form. The model's only
            // strict family is `Strict`. (The engine also decides plain-string pairs here —
            // lexically, identical to the fallback the model routes them through.)
            //
            // sq-2k5py — HONESTY BOUNDARY: the `Strict` (DateTime-kind) arm is
            // TOTAL, which is now the documented `strict_cmp` CONTRACT rather than a
            // modelling convenience. These harnesses therefore prove transitivity for a
            // CONTRACT-CONFORMING implementation; they do NOT model the partial (relational)
            // temporal comparison, whose indeterminate mixed-timezone window fell through to
            // the lexical fallback and was intransitive — that witness is the
            // `witness4_datetime_kind_indeterminate_window_needs_a_total_strict_cmp` unit
            // test, and the real implementations satisfy the contract via
            // `sparq_core::temporal::Timeline::cmp_tl_total`.
            match (self, other) {
                (M::Strict(a), M::Strict(b)) => Some(a.cmp(b)),
                _ => None,
            }
        }
        fn triple_parts(&self) -> Option<[Self; 3]> {
            match self {
                M::Trip(s, p, o) => Some([M::Iri(*s), M::Iri(*p), M::Int(*o)]),
                _ => None,
            }
        }
    }

    /// A symbolic index below `n`.
    fn any_idx(n: u8) -> u8 {
        let i: u8 = kani::any();
        kani::assume(i < n);
        i
    }

    /// Any depth-1 triple. Objects range over `{2, 2^53, 2^53+1}` so the RECURSIVE numeric
    /// arm exercises the exact-tier recheck on the collapsed pair inside a triple.
    fn any_trip() -> M {
        let o: u8 = kani::any();
        kani::assume(o == I_TWO || o == I_2P53 || o == I_2P53_P1);
        M::Trip(any_idx(2), any_idx(2), o)
    }

    /// Strict-weak-order composition at one symbolic triple `(x, y, z)`. Asserting the
    /// `Less`/`Equal` compositions is COMPLETE: the `Greater` compositions are the same
    /// instantiations under an `(x, z)` swap, and the harness quantifies symbolically over
    /// every permutation of the domain (antisymmetry-consistency is proved separately).
    fn assert_transitive_at(x: &M, y: &M, z: &M) {
        let (Some(xy), Some(yz)) = (compare_terms(x, y), compare_terms(y, z)) else {
            return; // outside the defined domain (law scoped to Some legs)
        };
        let xz = compare_terms(x, z);
        match (xy, yz) {
            (Ordering::Less, Ordering::Less)
            | (Ordering::Less, Ordering::Equal)
            | (Ordering::Equal, Ordering::Less) => {
                assert!(xz == Some(Ordering::Less), "transitivity: x<=y<=z (strict leg) => x<z");
            }
            (Ordering::Equal, Ordering::Equal) => {
                assert!(xz == Some(Ordering::Equal), "transitivity: x~y~z => x~z");
            }
            _ => (),
        }
    }

    /// Domain self-check: the numeric domain genuinely exhibits the 2^53 collapse (distinct
    /// integers, one f64 image) and the double table contains the collapse image itself.
    /// If this fails, the domain is non-adversarial and the other harnesses are vacuous.
    #[kani::proof]
    fn domain_exhibits_the_2p53_collapse() {
        assert!(INTS[usize::from(I_2P53)] != INTS[usize::from(I_2P53_P1)]);
        // Deliberate exact f64 comparisons: the collapse IS bit-level image equality.
        assert!(INT_F64S[usize::from(I_2P53)] == INT_F64S[usize::from(I_2P53_P1)]);
        assert!(INT_F64S[usize::from(I_2P53_M1)] < INT_F64S[usize::from(I_2P53)]);
        assert!(INT_F64S[usize::from(I_2P53)] < INT_F64S[usize::from(I_2P53_P2)]);
        assert!(DBLS[usize::from(D_2P53)] == INT_F64S[usize::from(I_2P53)]);
        assert!(INTS[0] != INTS[1] && INT_F64S[0] == INT_F64S[1]); // negative-sign collapse
    }

    // ------------------------------------------------------------------------------
    // KIND-DISPATCH COMBINATORS. Each `for_each_*` symbolically SELECTS one literal kind
    // and calls `f` with a term whose enum DISCRIMINANT is concrete (only the value index
    // inside the kind stays symbolic). This is the load-bearing tractability device: a
    // term with a SYMBOLIC discriminant forks CBMC symex at every downstream match in the
    // comparator (multiplicatively, per trait-method call), which does not terminate
    // practically; a concrete-discriminant term keeps each dispatch leaf linear. Coverage
    // is unchanged — the symbolic selector still quantifies over every kind in the list.
    // ------------------------------------------------------------------------------

    /// One symbolic choice of NUMERIC literal kind — `Int` / `Dbl` over their full tables,
    /// plus NaN when `with_nan` — passed to `f` with a concrete discriminant.
    fn for_each_numeric(with_nan: bool, f: impl Fn(&M)) {
        match kani::any::<u8>() {
            0 => f(&M::Int(any_idx(INTS.len() as u8))),
            1 if with_nan => f(&M::Nan),
            _ => f(&M::Dbl(any_idx(DBLS.len() as u8))),
        }
    }

    /// One symbolic choice of NON-NUMERIC scalar kind — `Err`, `Blank`/`Iri`/`Str` over the
    /// full `STRS` table, `Strict` over the full `STRICT_STRS` table.
    fn for_each_nonnumeric_scalar(f: impl Fn(&M)) {
        match kani::any::<u8>() {
            0 => f(&M::Err),
            1 => f(&M::Blank(any_idx(STRS.len() as u8))),
            2 => f(&M::Iri(any_idx(STRS.len() as u8))),
            3 => f(&M::Str(any_idx(STRS.len() as u8))),
            _ => f(&M::Strict(any_idx(STRICT_STRS.len() as u8))),
        }
    }

    /// One symbolic choice of non-literal scalar class — `Err` / `Blank` / `Iri`.
    fn for_each_nonlit_scalar(f: impl Fn(&M)) {
        match kani::any::<u8>() {
            0 => f(&M::Err),
            1 => f(&M::Blank(any_idx(STRS.len() as u8))),
            _ => f(&M::Iri(any_idx(STRS.len() as u8))),
        }
    }

    /// One symbolic choice of literal kind over the FULL literal domain, NaN INCLUDED —
    /// `Int` / `Dbl` / `Nan` / `Str` / `Strict`. (Before the sq-wjl8i fix the literal
    /// laws only held NaN-free; the totalised order covers the full domain.)
    fn for_each_literal(f: impl Fn(&M)) {
        match kani::any::<u8>() {
            0 => f(&M::Int(any_idx(INTS.len() as u8))),
            1 => f(&M::Dbl(any_idx(DBLS.len() as u8))),
            2 => f(&M::Nan),
            3 => f(&M::Str(any_idx(STRS.len() as u8))),
            _ => f(&M::Strict(any_idx(STRICT_STRS.len() as u8))),
        }
    }

    /// One symbolic choice from the REDUCED int-with-non-literal composition domain:
    /// `Int` over the 3-value collapse straddle `{2, 2^53, 2^53+1}`, `Err`, and
    /// `Blank`/`Iri` over the first two `STRS` entries.
    fn for_each_int_or_nonlit_small(f: impl Fn(&M)) {
        match kani::any::<u8>() {
            0 => f(&M::Err),
            1 => f(&M::Blank(any_idx(2))),
            2 => f(&M::Iri(any_idx(2))),
            _ => {
                let o: u8 = kani::any();
                kani::assume(o == I_TWO || o == I_2P53 || o == I_2P53_P1);
                f(&M::Int(o))
            }
        }
    }

    // REFLEXIVITY over the FULL domain, NaN INCLUDED: `compare_terms(x, x) == Some(Equal)`.
    // (Before the sq-wjl8i fix NaN made the comparator partial; the totalised order is
    // reflexive everywhere.) Reflexivity is UNARY and per-value, so splitting the domain
    // by literal kind proves the SAME law over the SAME union domain: the three harnesses
    // below jointly cover every model term.
    fn assert_reflexive_at(x: &M) {
        assert!(compare_terms(x, x) == Some(Ordering::Equal), "reflexivity");
    }

    /// REFLEXIVITY: numeric literals — `Int` over the full `INTS` table (collapse straddle
    /// included), `Dbl` over the full `DBLS` table, and `NaN` (NaN == NaN in this total
    /// order — the sq-wjl8i totalisation).
    #[kani::proof]
    #[kani::unwind(3)]
    fn reflexivity_numeric_literals_incl_nan() {
        for_each_numeric(true, |x| assert_reflexive_at(x));
    }

    /// REFLEXIVITY: the non-numeric scalar kinds — `Err`, `Blank`/`Iri`/`Str` over the full
    /// `STRS` table, `Strict` over the full `STRICT_STRS` table.
    #[kani::proof]
    #[kani::unwind(4)]
    fn reflexivity_nonnumeric_scalars() {
        for_each_nonnumeric_scalar(|x| assert_reflexive_at(x));
    }

    /// REFLEXIVITY: depth-1 triple terms (objects straddle the 2^53 collapse, so the
    /// recursive exact-recheck arm is exercised on the x == x diagonal too).
    #[kani::proof]
    #[kani::unwind(4)]
    fn reflexivity_triple_terms() {
        assert_reflexive_at(&any_trip());
    }

    // ANTISYMMETRY-CONSISTENCY over the FULL domain (NaN included): the two directions are
    // exact mirrors — `Some(o)` iff `Some(o.reverse())`, and `None` iff `None`. The
    // assertion checks BOTH directions of one symbolic pair, so covering every unordered
    // kind-GROUP pair covers every ordered pair of the full domain: with groups
    // N = {Int, Dbl, Nan}, S = {Err, Blank, Iri, Str, Strict}, T = {Trip}, the five
    // harnesses below cover N×N, N×S, S×S, T×(N∪S), T×T = (N∪S∪T)².
    fn assert_antisymmetric_at(x: &M, y: &M) {
        match compare_terms(x, y) {
            Some(o) => {
                assert!(compare_terms(y, x) == Some(o.reverse()), "antisymmetry: mirror");
            }
            None => assert!(compare_terms(y, x).is_none(), "antisymmetry: None mirror"),
        }
    }

    /// ANTISYMMETRY: numeric × numeric, NaN INCLUDED (the `None`-mirror leg is live here:
    /// NaN against any numeric is `None` in BOTH directions).
    #[kani::proof]
    #[kani::unwind(3)]
    fn antisymmetry_numeric_pairs_incl_nan() {
        for_each_numeric(true, |x| for_each_numeric(true, |y| assert_antisymmetric_at(x, y)));
    }

    /// ANTISYMMETRY: numeric (NaN included) × non-numeric scalar — the mixed-kind literal
    /// pairs ride the lexical fallback (including the long `INT_STRS`/`DBL_STRS` forms
    /// against `STRICT_STRS`, hence the larger unwind); cross-class pairs ride the rank.
    #[kani::proof]
    #[kani::unwind(12)]
    fn antisymmetry_numeric_vs_nonnumeric_incl_nan() {
        for_each_numeric(true, |x| {
            for_each_nonnumeric_scalar(|y| assert_antisymmetric_at(x, y));
        });
    }

    /// ANTISYMMETRY: non-numeric scalar × non-numeric scalar (string arms + class ranks).
    #[kani::proof]
    #[kani::unwind(4)]
    fn antisymmetry_nonnumeric_scalar_pairs() {
        for_each_nonnumeric_scalar(|x| {
            for_each_nonnumeric_scalar(|y| assert_antisymmetric_at(x, y));
        });
    }

    /// ANTISYMMETRY: triple term × any non-triple (always a cross-class pair — the class
    /// rank decides; NaN included on the scalar side).
    #[kani::proof]
    #[kani::unwind(4)]
    fn antisymmetry_triple_vs_scalar() {
        let t = any_trip();
        if kani::any() {
            for_each_numeric(true, |y| assert_antisymmetric_at(&t, y));
        } else {
            for_each_nonnumeric_scalar(|y| assert_antisymmetric_at(&t, y));
        }
    }

    /// ANTISYMMETRY: triple term × triple term (the component-wise recursion mirrored).
    #[kani::proof]
    #[kani::unwind(4)]
    fn antisymmetry_triple_pairs() {
        assert_antisymmetric_at(&any_trip(), &any_trip());
    }

    // WITHIN-CLASS TOTALITY over the FULL domain, NaN INCLUDED: every same-class pair
    // compares `Some` — mixed literal kinds rank by `LiteralKind`, same-kind pairs by
    // their within-kind order, NaN by its fixed first-among-numerics position (all
    // sq-wjl8i). Split: the Literal class (the only class with multiple kinds) and the
    // singleton-kind classes.

    /// WITHIN-CLASS TOTALITY: the Literal class over the FULL literal domain (NaN
    /// included) — every literal-kind pair is `Some`. The larger unwind covers the
    /// longest within-kind lexical compares.
    #[kani::proof]
    #[kani::unwind(12)]
    fn within_class_totality_literals_incl_nan() {
        for_each_literal(|x| {
            for_each_literal(|y| {
                assert!(compare_terms(x, y).is_some(), "within-class totality: literals");
            });
        });
    }

    /// WITHIN-CLASS TOTALITY: the singleton-kind classes — `Err`/`Blank`/`Iri`/`Trip`.
    /// For these classes same-class MEANS same-kind, so the four same-kind pairs below are
    /// exactly the same-class pairs the law quantifies over.
    #[kani::proof]
    #[kani::unwind(4)]
    fn within_class_totality_nonliterals() {
        let tot = |x: &M, y: &M| {
            assert!(compare_terms(x, y).is_some(), "within-class totality: non-literals");
        };
        match kani::any::<u8>() {
            0 => tot(&M::Err, &M::Err),
            1 => tot(
                &M::Blank(any_idx(STRS.len() as u8)),
                &M::Blank(any_idx(STRS.len() as u8)),
            ),
            2 => tot(
                &M::Iri(any_idx(STRS.len() as u8)),
                &M::Iri(any_idx(STRS.len() as u8)),
            ),
            _ => tot(&any_trip(), &any_trip()),
        }
    }

    /// TRANSITIVITY: exact-integer literals over the FULL `INTS` table, INCLUDING the 2^53
    /// collapse straddle. The collapsed pair `{2^53, 2^53+1}` is in-domain, so this proves
    /// the `exact_cmp` recheck keeps the exact tier transitive ACROSS the boundary.
    /// Domain: Int-ONLY (a fixed kind pattern) — the composition with the non-literal
    /// scalar classes lives in `transitivity_nonliteral_scalars` /
    /// `transitivity_int_with_nonliteral_scalars` (see the tractability note above).
    #[kani::proof]
    #[kani::unwind(3)]
    fn transitivity_exact_int_literals_incl_2p53_collapse() {
        let term = || M::Int(any_idx(INTS.len() as u8));
        assert_transitive_at(&term(), &term(), &term());
    }

    /// TRANSITIVITY: the non-literal scalar classes (`Err` / `Blank` / `Iri` over the full
    /// `STRS` table) — cross-class precedence plus the within-class string arms.
    #[kani::proof]
    #[kani::unwind(4)]
    fn transitivity_nonliteral_scalars() {
        for_each_nonlit_scalar(|x| {
            for_each_nonlit_scalar(|y| {
                for_each_nonlit_scalar(|z| assert_transitive_at(x, y, z));
            });
        });
    }

    /// TRANSITIVITY: exact-integer literals COMPOSED with the non-literal scalar classes —
    /// the cross-class legs of the law. REDUCED domain (a stated shrink, for symex
    /// tractability): Int over the 3-value collapse straddle `{2, 2^53, 2^53+1}` (so the
    /// exact-recheck arm is still exercised next to cross-class legs), Blank/Iri over the
    /// first two `STRS` entries, and `Err`. The FULL-domain within-class laws are the two
    /// harnesses above; cross-class order itself is decided purely by the `TermClass` rank,
    /// which does not depend on the per-kind value domains.
    #[kani::proof]
    #[kani::unwind(4)]
    fn transitivity_int_with_nonliteral_scalars() {
        for_each_int_or_nonlit_small(|x| {
            for_each_int_or_nonlit_small(|y| {
                for_each_int_or_nonlit_small(|z| assert_transitive_at(x, y, z));
            });
        });
    }

    /// TRANSITIVITY: double literals over the `DBLS` table, which is NaN-free because `NaN`
    /// is the separate `M::Nan` variant — NOT because NaN is undecided: since the sq-wjl8i
    /// totalisation NaN takes a fixed first-among-numerics position, and transitivity WITH
    /// NaN is proved by `transitivity_mixed_literal_kinds_incl_nan`. Includes the `±0.0`
    /// equal-but-lexically-distinct pair: a mutation that let numeric ties fall through to
    /// the string form would go red here.
    #[kani::proof]
    #[kani::unwind(4)]
    fn transitivity_double_literals() {
        let term = || M::Dbl(any_idx(DBLS.len() as u8));
        assert_transitive_at(&term(), &term(), &term());
    }

    /// One symbolic choice from the collapse-free MIXED numeric domain: `Int` over
    /// `{-2, 0, 2, 10}` (indices 2..=I_TEN) or `Dbl` over `{-0.0, 0.0, 0.5}` (indices
    /// 1..=3) — every value exactly representable in f64.
    fn for_each_cf_numeric(f: impl Fn(&M)) {
        if kani::any() {
            let i: u8 = kani::any();
            kani::assume(i >= 2 && i <= I_TEN);
            f(&M::Int(i));
        } else {
            let i: u8 = kani::any();
            kani::assume(i >= 1 && i <= 3);
            f(&M::Dbl(i));
        }
    }

    /// TRANSITIVITY: a MIXED exact/inexact numeric domain restricted BELOW the collapse
    /// boundary (see `for_each_cf_numeric` for the exact values). Lawful here — which
    /// sharpens the mixed-tier witness: the law breaks specifically AT the collapse, not
    /// on every exact/inexact mix.
    #[kani::proof]
    #[kani::unwind(3)]
    fn transitivity_mixed_numeric_collapse_free_range() {
        for_each_cf_numeric(|x| {
            for_each_cf_numeric(|y| {
                for_each_cf_numeric(|z| assert_transitive_at(x, y, z));
            });
        });
    }

    /// Domain self-check for the collapse-free mixed-numeric domain (companion to
    /// `domain_exhibits_the_2p53_collapse`, which pins the straddle domains): every `Int`
    /// the `for_each_cf_numeric` combinator can emit has an EXACT f64 image (the range is
    /// genuinely collapse-free — an index drift onto the straddle would surface here), and
    /// the `Dbl` table's `±0.0` entries (indices 1/2, inside the combinator's `1..=3`
    /// range) are the equal-but-lexically-distinct pair the double-transitivity harness
    /// relies on to catch a ties-fall-to-string-form mutation. If this fails, a re-scope
    /// has silenced the domain's interesting inputs and the harnesses above it are weaker
    /// than documented. sq-sqtk2.4
    #[kani::proof]
    #[kani::unwind(3)]
    fn domain_cf_numeric_is_collapse_free_with_signed_zero_pair() {
        for_each_cf_numeric(|x| match x {
            M::Int(i) => {
                // Exact image: the f64 roundtrips to the same integer (deliberate exact
                // comparison — exactness IS the property).
                assert!(
                    INT_F64S[usize::from(*i)] as i128 == INTS[usize::from(*i)],
                    "cf domain: Int range must be collapse-free"
                );
            }
            M::Dbl(i) => assert!(usize::from(*i) < DBLS.len(), "cf domain: Dbl index in-table"),
            _ => unreachable!("for_each_cf_numeric emits only Int/Dbl"),
        });
        // The signed-zero pair: numerically equal, lexically distinct.
        assert!(DBLS[1] == DBLS[2], "signed zeros compare numerically equal");
        assert!(DBL_STRS[1] != DBL_STRS[2], "signed zeros are lexically distinct");
    }

    /// TRANSITIVITY: plain-string literals (lexical order; includes the empty string and a
    /// prefix pair).
    #[kani::proof]
    #[kani::unwind(4)]
    fn transitivity_string_literals() {
        let term = || M::Str(any_idx(STRS.len() as u8));
        assert_transitive_at(&term(), &term(), &term());
    }

    /// TRANSITIVITY: strict/temporal literals (the dateTime-by-timeline model arm).
    #[kani::proof]
    #[kani::unwind(4)]
    fn transitivity_strict_typed_literals() {
        let term = || M::Strict(any_idx(STRICT_STRS.len() as u8));
        assert_transitive_at(&term(), &term(), &term());
    }

    /// TRANSITIVITY across MIXED LITERAL KINDS, NaN INCLUDED — the law the sq-sqtk2.4
    /// witnesses machine-checked as FALSE before the sq-wjl8i kind-first fix, now PROVED
    /// over the FULL literal domain: `Int` over the whole `INTS` table (2^53 collapse
    /// straddle and the `10`/`2` digit-inversion values included), `Dbl` over the whole
    /// `DBLS` table (the collapse image included), `NaN`, `Str` over the whole `STRS`
    /// table (`"11"`, the digit-string that lexically inverts against `10`/`2`,
    /// included), and `Strict`. Cross-kind legs are decided purely by the
    /// `LiteralKind` rank; within-kind legs by the per-kind orders (exact numeric with
    /// the mixed-tier recheck, NaN first, strict, lexical). Revert the kind-rank rule
    /// for any one cross-kind pair (fall back to the lexical form) and this goes red on
    /// the `10 / "11" / 2` cycle; revert the exact mixed-tier recheck and it goes red on
    /// the `2^53 / double(2^53) / 2^53+1` collapse triple. sq-wjl8i
    #[kani::proof]
    #[kani::unwind(12)]
    fn transitivity_mixed_literal_kinds_incl_nan() {
        for_each_literal(|x| {
            for_each_literal(|y| {
                for_each_literal(|z| assert_transitive_at(x, y, z));
            });
        });
    }

    /// TRANSITIVITY: depth-1 triple terms — the component-wise recursion, with objects
    /// ranging over the collapsed integer pair so the recursive numeric arm hits the
    /// exact-tier recheck INSIDE a triple.
    #[kani::proof]
    #[kani::unwind(4)]
    fn transitivity_triple_terms_recursive() {
        assert_transitive_at(&any_trip(), &any_trip(), &any_trip());
    }

    /// EXACT-ORDER AGREEMENT: for every exact-tier pair, `compare_terms` equals the exact
    /// i128 value order — INCLUDING the collapsed pairs, where the lenient f64 arm alone
    /// would report `Equal`. This is the load-bearing guarantee of the `exact_cmp` recheck:
    /// delete the recheck in `compare_terms` (or weaken the model's `exact_cmp` to `None` on
    /// exact pairs) and this harness goes red on `{2^53, 2^53+1}` — the non-vacuity anchor.
    #[kani::proof]
    #[kani::unwind(3)]
    fn exact_int_order_agreement_incl_2p53_collapse() {
        let i = any_idx(INTS.len() as u8);
        let j = any_idx(INTS.len() as u8);
        let verdict = compare_terms(&M::Int(i), &M::Int(j));
        assert!(
            verdict == Some(INTS[usize::from(i)].cmp(&INTS[usize::from(j)])),
            "exact tier orders by exact value, collapse notwithstanding"
        );
    }

    /// Domain self-check (the sq-og8u8 pattern): every `DBL_X2` entry is EXACTLY twice
    /// its `DBLS` value — the fidelity condition under which the model's i128
    /// `exact_cmp` is the exact-rational order the engine's `Num::cmp_total` computes.
    /// The signed-zero pair collapsing to one x2 value (0) is deliberate: `-0.0` and
    /// `0.0` are exactly equal rationals. sq-wjl8i
    #[kani::proof]
    #[kani::unwind(8)]
    fn domain_x2_doubles_are_exact() {
        let mut i = 0;
        while i < DBLS.len() {
            // Exact f64 comparison: fidelity IS bit-level agreement (×2 only bumps the
            // exponent, and every X2 value is within f64's exact-integer range).
            assert!(DBL_X2[i] as f64 == DBLS[i] * 2.0, "X2 must be exactly twice DBLS");
            i += 1;
        }
        assert!(DBL_X2[1] == 0 && DBL_X2[2] == 0, "signed zeros are exactly equal");
    }

    /// FIXED — former FINDING 1 (bead sq-wjl8i; this harness pins the FIX and goes red
    /// on a regression): the exact/inexact numeric-tier MIX at the 2^53 collapse
    /// boundary now orders EXACTLY (every finite double is an exact rational). The
    /// double equal to the shared f64 image ties ONLY with the integer it exactly
    /// equals, and orders strictly below the collapsed neighbour — the triple that was
    /// `Equal / Equal / Less` (intransitive) is now `Equal / Less / Less` (transitive).
    #[kani::proof]
    fn witness_mixed_tier_collapse_now_exact() {
        let x = M::Int(I_2P53);
        let y = M::Dbl(D_2P53);
        let z = M::Int(I_2P53_P1);
        assert!(compare_terms(&x, &y) == Some(Ordering::Equal)); // truly equal values
        assert!(compare_terms(&y, &z) == Some(Ordering::Less)); // was Equal — the bug
        assert!(compare_terms(&x, &z) == Some(Ordering::Less));
    }

    /// FIXED — former FINDING 2 (bead sq-wjl8i; pins the FIX): numeric vs plain-string
    /// pairs now rank by KIND (`Numeric < String`), never by the lexical form, so the
    /// `10 / "11" / 2` digit-inversion cycle (`Less / Less / Greater`) is gone: both
    /// integers sort below the string, and against each other numerically.
    #[kani::proof]
    #[kani::unwind(4)]
    fn witness_numeric_vs_string_now_ranked_by_kind() {
        let x = M::Int(I_TEN); // 10
        let y = M::Str(S_11); // "11"
        let z = M::Int(I_TWO); // 2
        assert!(compare_terms(&x, &y) == Some(Ordering::Less)); // kind rank, not "10"<"11"
        assert!(compare_terms(&y, &z) == Some(Ordering::Greater)); // was lexical "11"<"2"
        assert!(compare_terms(&x, &z) == Some(Ordering::Greater)); // 10 > 2 — consistent
    }

    /// FIXED — former FINDING 3 (beads sq-wjl8i + the sq-ma9fb doc drift; pins the FIX):
    /// NaN no longer makes the comparator partial — it is totalised FIRST among
    /// numerics (before `-INF`) and equal to itself, so callers mapping `None` to
    /// `Equal` can no longer make NaN "equal" to every numeric. Against a plain string
    /// NaN now ranks by kind (`Numeric < String`), not by its `"NaN"` lexical form.
    #[kani::proof]
    #[kani::unwind(4)]
    fn witness_nan_now_totalised_first() {
        assert!(compare_terms(&M::Nan, &M::Nan) == Some(Ordering::Equal));
        assert!(compare_terms(&M::Nan, &M::Dbl(2)) == Some(Ordering::Less)); // vs 0.0
        assert!(compare_terms(&M::Dbl(2), &M::Nan) == Some(Ordering::Greater));
        assert!(compare_terms(&M::Nan, &M::Int(I_2P53)) == Some(Ordering::Less));
        assert!(compare_terms(&M::Nan, &M::Dbl(0)) == Some(Ordering::Less)); // vs -(2^53)
        assert!(compare_terms(&M::Nan, &M::Str(2)) == Some(Ordering::Less)); // kind rank
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    /// A minimal test-only term, exercising every arm of [`compare_terms`] without
    /// pulling the engine's `Value` (the engine's own conformance + ORDER BY suites
    /// exercise the real `Value` impl; this proves the algorithm in isolation, one
    /// direct test per public item per the coverage-ratchet rule).
    #[derive(Clone, Debug)]
    enum T {
        ErrorOrUnbound,
        Blank(String),
        Iri(String),
        /// A numeric literal (orders by f64 value; no exact tier — models `xsd:double`).
        NumLit(f64),
        /// A numeric literal carrying BOTH its lenient `f` (the possibly-collapsing f64)
        /// AND its EXACT value as a scaled integer `mant * 10^-scale`, so the f64-collapse
        /// recheck arm is exercised: two of these can share an `f` yet differ exactly —
        /// modelling an `xsd:integer` beyond 2^53 (`scale` 0) and a high-precision
        /// `xsd:decimal` (`scale > 0`). sq-rikm7
        ExactNum { f: f64, mant: i128, scale: u32 },
        /// A plain-string literal (orders by string form; `strict_cmp` decides equal
        /// strings, else `None` → string fallback).
        StrLit(String),
        /// A literal that is comparable only via `strict_cmp` returning the given
        /// ordering versus another `Strict` (models the dateTime/timeline arm).
        Strict(i64),
        /// A triple term `(s, p, o)`.
        Triple(Box<T>, Box<T>, Box<T>),
    }

    /// Test-model mirror of the engine's exact mixed-tier tie-break (`Num::cmp_total` →
    /// the exact decimal-string comparison): `mant × 10^-scale` against the f64's exact
    /// decimal expansion, pure string arithmetic. sq-wjl8i
    fn cmp_exact_vs_f64(mant: i128, scale: u32, f: f64) -> Ordering {
        if f.is_nan() {
            return Ordering::Greater; // NaN sorts first: every exact value is above it
        }
        if f == f64::INFINITY {
            return Ordering::Less;
        }
        if f == f64::NEG_INFINITY {
            return Ordering::Greater;
        }
        cmp_plain_dec(&dec_str(mant, scale), &format!("{:.1074}", f))
    }

    /// `mant × 10^-scale` as a plain decimal string (test-local `Dec::lexical` mirror).
    fn dec_str(mant: i128, scale: u32) -> String {
        let mag = mant.unsigned_abs().to_string();
        let s = scale as usize;
        let sign = if mant < 0 { "-" } else { "" };
        if s == 0 {
            return format!("{}{}", sign, mag);
        }
        if mag.len() > s {
            format!("{}{}.{}", sign, &mag[..mag.len() - s], &mag[mag.len() - s..])
        } else {
            format!("{}0.{}{}", sign, "0".repeat(s - mag.len()), mag)
        }
    }

    /// Exact comparison of two well-formed plain decimal strings (test-local mirror of
    /// `numeric::cmp_plain_decimal`; panics on malformed input — test-only).
    fn cmp_plain_dec(a: &str, b: &str) -> Ordering {
        let split = |s: &str| -> (bool, String, String) {
            let (neg, s) = match s.strip_prefix('-') {
                Some(r) => (true, r),
                None => (false, s),
            };
            let (int, frac) = s.split_once('.').unwrap_or((s, ""));
            (neg, int.trim_start_matches('0').to_string(), frac.trim_end_matches('0').to_string())
        };
        let (na, ia, fa) = split(a);
        let (nb, ib, fb) = split(b);
        let a_zero = ia.is_empty() && fa.is_empty();
        let b_zero = ib.is_empty() && fb.is_empty();
        if a_zero && b_zero {
            return Ordering::Equal;
        }
        let mag = ia.len().cmp(&ib.len()).then_with(|| ia.cmp(&ib)).then_with(|| {
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
        match (na && !a_zero, nb && !b_zero) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => mag,
            (true, true) => mag.reverse(),
        }
    }

    impl CompareTerm for T {
        fn term_class(&self) -> TermClass {
            match self {
                T::ErrorOrUnbound => TermClass::ErrorOrUnbound,
                T::Blank(_) => TermClass::Blank,
                T::Iri(_) => TermClass::Iri,
                T::NumLit(_) | T::ExactNum { .. } | T::StrLit(_) | T::Strict(_) => TermClass::Literal,
                T::Triple(..) => TermClass::Triple,
            }
        }
        fn literal_kind(&self) -> LiteralKind {
            match self {
                T::NumLit(_) | T::ExactNum { .. } => LiteralKind::Numeric,
                T::StrLit(_) => LiteralKind::String,
                T::Strict(_) => LiteralKind::DateTime,
                _ => LiteralKind::Other, // non-literals: never consulted
            }
        }
        fn value_str(&self) -> Option<String> {
            match self {
                T::ErrorOrUnbound => None,
                T::Blank(s) | T::Iri(s) | T::StrLit(s) => Some(s.clone()),
                T::NumLit(n) => Some(n.to_string()),
                T::ExactNum { f, .. } => Some(f.to_string()),
                T::Strict(n) => Some(n.to_string()),
                T::Triple(..) => None,
            }
        }
        fn as_f64(&self) -> Option<f64> {
            match self {
                T::NumLit(n) => Some(*n),
                T::ExactNum { f, .. } => Some(*f),
                _ => None,
            }
        }
        fn exact_cmp(&self, other: &Self) -> Option<Ordering> {
            match (self, other) {
                // Align both scaled integers to the common (max) scale, then compare the
                // mantissas exactly (mirrors the substrate `Dec::cmp` the engine's numeric
                // tower uses). sq-rikm7
                (T::ExactNum { mant: am, scale: asc, .. }, T::ExactNum { mant: bm, scale: bsc, .. }) => {
                    let scale = (*asc).max(*bsc);
                    let a = am.checked_mul(10i128.checked_pow(scale - asc)?)?;
                    let b = bm.checked_mul(10i128.checked_pow(scale - bsc)?)?;
                    Some(a.cmp(&b))
                }
                // MIXED exact/inexact pair: the exact-rational order against the f64's
                // exact decimal expansion — the engine's `Num::cmp_total` mixed arm
                // (witness 1 of sq-wjl8i: leaving this `None` keeps the collapsed f64
                // tie, which is intransitive against the exact int/int order).
                (T::ExactNum { mant, scale, .. }, T::NumLit(f)) => Some(cmp_exact_vs_f64(*mant, *scale, *f)),
                (T::NumLit(f), T::ExactNum { mant, scale, .. }) => {
                    Some(cmp_exact_vs_f64(*mant, *scale, *f).reverse())
                }
                // Two plain f64 numerics: the value IS the f64 — an f64 tie is a true tie.
                _ => None,
            }
        }
        fn strict_cmp(&self, other: &Self) -> Option<Ordering> {
            match (self, other) {
                (T::Strict(a), T::Strict(b)) => Some(a.cmp(b)),
                _ => None,
            }
        }
        fn triple_parts(&self) -> Option<[Self; 3]> {
            match self {
                T::Triple(s, p, o) => Some([(**s).clone(), (**p).clone(), (**o).clone()]),
                _ => None,
            }
        }
    }

    #[test]
    fn term_class_discriminants_match_sparql_precedence() {
        // The exact ranks compare_values used: error/unbound < blank < IRI < literal < triple.
        assert!(TermClass::ErrorOrUnbound < TermClass::Blank);
        assert!(TermClass::Blank < TermClass::Iri);
        assert!(TermClass::Iri < TermClass::Literal);
        assert!(TermClass::Literal < TermClass::Triple);
        assert_eq!(TermClass::ErrorOrUnbound as u8, 0);
        assert_eq!(TermClass::Triple as u8, 4);
    }

    #[test]
    fn cross_class_precedence() {
        // unbound < blank < IRI < literal < triple, regardless of within-class value.
        let unbound = T::ErrorOrUnbound;
        let blank = T::Blank("z".into());
        let iri = T::Iri("a".into());
        let lit = T::NumLit(-1.0);
        let triple = T::Triple(Box::new(T::Iri("a".into())), Box::new(T::Iri("a".into())), Box::new(T::Iri("a".into())));
        assert_eq!(compare_terms(&unbound, &blank), Some(Ordering::Less));
        assert_eq!(compare_terms(&blank, &iri), Some(Ordering::Less));
        assert_eq!(compare_terms(&iri, &lit), Some(Ordering::Less));
        assert_eq!(compare_terms(&lit, &triple), Some(Ordering::Less));
        // and the reverse direction.
        assert_eq!(compare_terms(&triple, &unbound), Some(Ordering::Greater));
    }

    #[test]
    fn within_class_error_is_equal() {
        assert_eq!(compare_terms(&T::ErrorOrUnbound, &T::ErrorOrUnbound), Some(Ordering::Equal));
    }

    #[test]
    fn blanks_and_iris_order_by_string() {
        assert_eq!(compare_terms(&T::Blank("a".into()), &T::Blank("b".into())), Some(Ordering::Less));
        assert_eq!(compare_terms(&T::Iri("http://b".into()), &T::Iri("http://a".into())), Some(Ordering::Greater));
        assert_eq!(compare_terms(&T::Iri("x".into()), &T::Iri("x".into())), Some(Ordering::Equal));
    }

    #[test]
    fn numeric_literals_order_by_value_not_lexical() {
        // 9 < 10 by value even though "10" < "9" lexically — the numeric arm wins.
        assert_eq!(compare_terms(&T::NumLit(9.0), &T::NumLit(10.0)), Some(Ordering::Less));
        assert_eq!(compare_terms(&T::NumLit(2.5), &T::NumLit(2.5)), Some(Ordering::Equal));
    }

    #[test]
    fn f64_collapse_recheck_orders_big_integers_beyond_2_53() {
        // 2^53 and 2^53 + 1 share ONE f64 (2^53 + 1 is not representable) — the lenient
        // numeric arm alone would report them Equal. The exact recheck (`exact_cmp`) keeps
        // them ordered by value, so ORDER BY / MIN / MAX agree with relational =/<. This
        // assertion FAILS if the recheck in `compare_terms` is reverted (it would be Equal).
        let two53 = 9_007_199_254_740_992.0_f64;
        let a = T::ExactNum { f: two53, mant: 9_007_199_254_740_992, scale: 0 };
        let b = T::ExactNum { f: two53, mant: 9_007_199_254_740_993, scale: 0 };
        // The f64 keys are byte-equal: the collapse the recheck must repair.
        assert_eq!(a.as_f64(), b.as_f64());
        assert_eq!(compare_terms(&a, &b), Some(Ordering::Less));
        assert_eq!(compare_terms(&b, &a), Some(Ordering::Greater));
        // Genuinely equal exact values stay Equal (no spurious inequality introduced).
        let a2 = T::ExactNum { f: two53, mant: 9_007_199_254_740_992, scale: 0 };
        assert_eq!(compare_terms(&a, &a2), Some(Ordering::Equal));
    }

    #[test]
    fn f64_collapse_recheck_orders_high_precision_decimals() {
        // 0.123456789012345678 and 0.123456789012345679 differ only in the 18th fraction
        // digit and round to the SAME f64 (its shortest form is 0.12345678901234568); the
        // `f` field is that shared collapsed f64. The exact recheck orders them by value.
        let f = 0.12345678901234568_f64;
        let a = T::ExactNum { f, mant: 123_456_789_012_345_678, scale: 18 };
        let b = T::ExactNum { f, mant: 123_456_789_012_345_679, scale: 18 };
        assert_eq!(a.as_f64(), b.as_f64()); // f64 COLLAPSES them
        assert_eq!(compare_terms(&a, &b), Some(Ordering::Less));
        assert_eq!(compare_terms(&b, &a), Some(Ordering::Greater));
        // Different scales still align exactly: 0.10 (mant 10, scale 2) == 0.1 (mant 1, scale 1).
        let ten_hundredths = T::ExactNum { f: 0.1, mant: 10, scale: 2 };
        let one_tenth = T::ExactNum { f: 0.1, mant: 1, scale: 1 };
        assert_eq!(compare_terms(&ten_hundredths, &one_tenth), Some(Ordering::Equal));
        // The MIXED exact/inexact pair also decides exactly (sq-wjl8i): the double f is
        // exactly 0.12345678901234567736…, so BOTH high-precision decimals (…678, …679)
        // sit strictly above the double they collapse onto — mirrored in both directions.
        assert_eq!(a.exact_cmp(&T::NumLit(f)), Some(Ordering::Greater));
        assert_eq!(T::NumLit(f).exact_cmp(&a), Some(Ordering::Less));
        assert_eq!(compare_terms(&a, &T::NumLit(f)), Some(Ordering::Greater));
        assert_eq!(compare_terms(&T::NumLit(f), &b), Some(Ordering::Less));
        // An exact value that IS its double stays a true tie.
        let half = T::ExactNum { f: 0.5, mant: 5, scale: 1 };
        assert_eq!(compare_terms(&half, &T::NumLit(0.5)), Some(Ordering::Equal));
    }

    #[test]
    fn strict_arm_decides_when_numeric_arm_cannot() {
        // Two Strict literals: as_f64 is None, strict_cmp decides by the timeline-like order.
        assert_eq!(compare_terms(&T::Strict(100), &T::Strict(200)), Some(Ordering::Less));
        assert_eq!(compare_terms(&T::Strict(5), &T::Strict(5)), Some(Ordering::Equal));
    }

    #[test]
    fn string_fallback_when_no_numeric_and_no_strict() {
        // Two plain string literals (same kind): strict_cmp None → lexical string order.
        assert_eq!(compare_terms(&T::StrLit("apple".into()), &T::StrLit("banana".into())), Some(Ordering::Less));
    }

    #[test]
    fn cross_kind_literals_rank_by_kind_never_lexically() {
        // sq-wjl8i: a numeric vs a string literal ranks by LiteralKind
        // (Numeric < String) — NEVER by the lexical form. "0" < "1" lexically would
        // put the string first; the kind rank keeps every numeric below every string.
        assert_eq!(compare_terms(&T::NumLit(1.0), &T::StrLit("0".into())), Some(Ordering::Less));
        assert_eq!(compare_terms(&T::StrLit("0".into()), &T::NumLit(1.0)), Some(Ordering::Greater));
        // Numeric < DateTime-kind (strict) < String, per the documented rank.
        assert_eq!(compare_terms(&T::NumLit(1.0), &T::Strict(0)), Some(Ordering::Less));
        assert_eq!(compare_terms(&T::Strict(0), &T::StrLit("".into())), Some(Ordering::Less));
        // The enum rank order itself (the documented extension).
        assert!(LiteralKind::Numeric < LiteralKind::Boolean);
        assert!(LiteralKind::Boolean < LiteralKind::DateTime);
        assert!(LiteralKind::DateTime < LiteralKind::Date);
        assert!(LiteralKind::Date < LiteralKind::String);
        assert!(LiteralKind::String < LiteralKind::Lang);
        assert!(LiteralKind::Lang < LiteralKind::Other);
    }

    #[test]
    fn triple_terms_order_componentwise_recursively() {
        let mk = |s: &str, p: &str, o: f64| {
            T::Triple(Box::new(T::Iri(s.into())), Box::new(T::Iri(p.into())), Box::new(T::NumLit(o)))
        };
        // Equal subject+predicate, object 1 < 2 by numeric value.
        assert_eq!(compare_terms(&mk("s", "p", 1.0), &mk("s", "p", 2.0)), Some(Ordering::Less));
        // Predicate decides before object: p1 < p2 even though o is greater.
        assert_eq!(compare_terms(&mk("s", "p1", 99.0), &mk("s", "p2", 1.0)), Some(Ordering::Less));
        // Subject decides first.
        assert_eq!(compare_terms(&mk("s1", "p", 1.0), &mk("s2", "p", 1.0)), Some(Ordering::Less));
        // Nested triple in subject position recurses.
        let nested = |inner_o: f64| {
            T::Triple(
                Box::new(T::Triple(Box::new(T::Iri("a".into())), Box::new(T::Iri("b".into())), Box::new(T::NumLit(inner_o)))),
                Box::new(T::Iri("p".into())),
                Box::new(T::Iri("o".into())),
            )
        };
        assert_eq!(compare_terms(&nested(1.0), &nested(2.0)), Some(Ordering::Less));
        assert_eq!(compare_terms(&nested(2.0), &nested(2.0)), Some(Ordering::Equal));
    }

    // --- FIXED BUG WITNESSES (sq-wjl8i; formerly `#[ignore]`d red pins of the broken
    //     behaviour, now ACTIVE regression cases of the FIX) ---
    //
    // These three tests carry the machine-checked counterexamples from the Kani
    // `kani_proofs` module's `witness_*` harnesses. Before the sq-wjl8i fix they were
    // `#[ignore]`d and asserted the ORDER LAWS that failed; the comparator is now a total
    // order, so they run in every `cargo test` and pin the fixed verdicts (the exact
    // triples that used to be intransitive). /

    /// FIXED witness 1 — the mixed exact/inexact numeric pair at the 2^53 collapse
    /// boundary now orders EXACTLY (every finite double is an exact rational): the
    /// double ties only with the integer it exactly equals and sits strictly below the
    /// collapsed neighbour, so the triple `(2^53, double(2^53), 2^53+1)` — formerly
    /// `Equal / Equal / Less`, an intransitive equality — is now `Equal / Less / Less`.
    /// `ORDER BY` / `MIN` / `MAX` over a column mixing integers and doubles near 2^53
    /// is permutation-stable again. sq-wjl8i
    #[test]
    fn witness1_mixed_exact_inexact_at_2p53_now_transitive() {
        let two53: f64 = 9_007_199_254_740_992.0;
        // x = xsd:integer 2^53 (exact tier)
        let x = T::ExactNum { f: two53, mant: 9_007_199_254_740_992_i128, scale: 0 };
        // y = xsd:double 2^53 (inexact tier — its value IS the f64)
        let y = T::NumLit(two53);
        // z = xsd:integer 2^53+1 (exact tier; shares its f64 image with x and y)
        let z = T::ExactNum { f: two53, mant: 9_007_199_254_740_993_i128, scale: 0 };
        assert_eq!(compare_terms(&x, &y), Some(Ordering::Equal), "x IS y exactly");
        assert_eq!(compare_terms(&y, &z), Some(Ordering::Less), "the collapse no longer ties (was Equal)");
        assert_eq!(compare_terms(&x, &z), Some(Ordering::Less), "transitive: x = y < z => x < z");
        assert_eq!(compare_terms(&z, &y), Some(Ordering::Greater), "mirror");
    }

    /// FIXED witness 2 — numeric vs plain-string pairs now rank by KIND
    /// (`Numeric < String`), never by the lexical form, so the digit-string inversion
    /// `"10" < "11" < "2"` can no longer contradict the numeric `10 > 2`: the triple
    /// `(10, "11", 2)` — formerly `Less / Less / Greater`, an intransitive strict
    /// order — is now consistent (`2 < 10 < "11"`). Any ORDER BY column mixing numerics
    /// and plain strings sorts consistently again. sq-wjl8i
    #[test]
    fn witness2_numeric_vs_string_now_ranked_by_kind() {
        let x = T::ExactNum { f: 10.0, mant: 10, scale: 0 };
        let z = T::ExactNum { f: 2.0, mant: 2, scale: 0 };
        let y = T::StrLit("11".into());
        assert_eq!(compare_terms(&x, &y), Some(Ordering::Less), "Numeric < String by kind rank");
        assert_eq!(compare_terms(&y, &z), Some(Ordering::Greater), "String > Numeric (was lexical \"11\" < \"2\")");
        assert_eq!(compare_terms(&x, &z), Some(Ordering::Greater), "10 > 2 numerically — consistent");
    }

    /// FIXED witness 3 — NaN no longer makes the comparator partial: it is totalised
    /// FIRST among numerics (before `-INF`) and equal to itself, so a caller mapping
    /// `None` to `Equal` can no longer place NaN "equal" to every numeric. Within-class
    /// totality holds NaN included. sq-wjl8i (and the sq-ma9fb doc drift:
    /// `None` is no longer returned for NaN).
    #[test]
    fn witness3_nan_now_totalised_first_among_numerics() {
        let nan = T::NumLit(f64::NAN);
        let zero = T::NumLit(0.0);
        let neg_inf = T::NumLit(f64::NEG_INFINITY);
        assert_eq!(compare_terms(&nan, &zero), Some(Ordering::Less), "NaN sorts first");
        assert_eq!(compare_terms(&zero, &nan), Some(Ordering::Greater), "mirror");
        assert_eq!(compare_terms(&nan, &neg_inf), Some(Ordering::Less), "NaN before -INF");
        assert_eq!(compare_terms(&nan, &nan), Some(Ordering::Equal), "NaN ties with itself");
        // And against another kind, NaN ranks as a numeric (kind rank, not lexical "NaN").
        assert_eq!(compare_terms(&nan, &T::StrLit("A".into())), Some(Ordering::Less));
    }

    /// An `xsd:dateTime`-kind literal model for the sq-2k5py contract: the parsed instant
    /// (assuming UTC), the timezone-PRESENCE bit, and the lexical form — plus a `total`
    /// switch selecting which comparison [`CompareTerm::strict_cmp`] surfaces. `false`
    /// reproduces the OLD relational (PARTIAL) comparison, whose indeterminate mixed-presence
    /// window returned `None` and so dropped the pair to the lexical fallback; `true` is the
    /// contracted TOTAL order (`sparq_core::temporal::Timeline::cmp_tl_total`: instant, then
    /// presence). sq-2k5py
    #[derive(Clone, Debug)]
    struct TzLit {
        instant: i64,
        has_tz: bool,
        lex: &'static str,
        total: bool,
    }

    impl CompareTerm for TzLit {
        fn term_class(&self) -> TermClass {
            TermClass::Literal
        }
        fn literal_kind(&self) -> LiteralKind {
            LiteralKind::DateTime
        }
        fn value_str(&self) -> Option<String> {
            Some(self.lex.to_string())
        }
        fn as_f64(&self) -> Option<f64> {
            None // a temporal is not numeric
        }
        fn exact_cmp(&self, _other: &Self) -> Option<Ordering> {
            None // no numeric tier
        }
        fn strict_cmp(&self, other: &Self) -> Option<Ordering> {
            // XPath: same (or no) timezone compares directly, and MIXED presence is
            // decidable only outside the ±14h window.
            let decidable = self.has_tz == other.has_tz || (self.instant - other.instant).abs() > 14 * 3600;
            if decidable {
                return Some(self.instant.cmp(&other.instant));
            }
            // The indeterminate window: `None` (the relational type error) unless the
            // implementation honours the totality contract.
            self.total.then(|| self.instant.cmp(&other.instant).then(self.has_tz.cmp(&other.has_tz)))
        }
        fn triple_parts(&self) -> Option<[Self; 3]> {
            None
        }
    }

    /// FIXED witness 4 — the DateTime-kind RESIDUAL of the sq-wjl8i kind-first fix: a
    /// `strict_cmp` that leaves XPath's indeterminate mixed-timezone window `None` sends
    /// those pairs — and ONLY those — to the lexical fallback, so one literal kind mixes
    /// timeline-decided and lexical-decided pairs. Two zoned dateTimes in OPPOSITE offsets are
    /// the SAME instant (timeline-Equal) while the tz-less `13:00:00` sits lexically
    /// BETWEEN them: `x ~ y` yet `x < z` and `z < y`, an inconsistent comparator — the same intransitivity SHAPE as the sq-wjl8i witnesses. Honouring the
    /// totality contract (instant, then timezone presence) removes it. sq-2k5py
    #[test]
    fn witness4_datetime_kind_indeterminate_window_needs_a_total_strict_cmp() {
        // 13:00 UTC as "12:00:00-01:00" (zoned), as "14:00:00+01:00" (zoned), and the
        // tz-less "13:00:00" — one instant, three lexicals.
        let at = |lex: &'static str, has_tz: bool, total: bool| TzLit { instant: 13 * 3600, has_tz, lex, total };
        let mk = |total: bool| {
            (
                at("2024-03-15T12:00:00-01:00", true, total),
                at("2024-03-15T14:00:00+01:00", true, total),
                at("2024-03-15T13:00:00", false, total),
            )
        };

        // PARTIAL `strict_cmp` (the pre-fix behaviour): x ~ y, but z falls lexically
        // BETWEEN them — `sort_by` is fed an inconsistent comparator.
        let (x, y, z) = mk(false);
        assert_eq!(x.strict_cmp(&z), None, "the pair really is XPath-indeterminate");
        assert_eq!(compare_terms(&x, &y), Some(Ordering::Equal), "same instant, both zoned");
        assert_eq!(compare_terms(&x, &z), Some(Ordering::Less), "lexical fallback: \"…T12:00:00-01:00\" < \"…T13:00:00\"");
        assert_eq!(compare_terms(&z, &y), Some(Ordering::Less), "lexical fallback: \"…T13:00:00\" < \"…T14:00:00+01:00\"");
        // `x ~ y` and `z < y` force `z < x` if the comparator is consistent. It is not:
        assert_ne!(compare_terms(&z, &x), Some(Ordering::Less), "INTRANSITIVE: z < y and y ~ x, yet z > x");

        // TOTAL `strict_cmp` (the contract): the equal-instant class is ordered by
        // timezone presence, so the tz-less value sits strictly below BOTH zoned ones.
        let (x, y, z) = mk(true);
        assert_eq!(compare_terms(&x, &y), Some(Ordering::Equal), "same instant, both zoned — still Equal");
        assert_eq!(compare_terms(&z, &x), Some(Ordering::Less), "tz-less before zoned");
        assert_eq!(compare_terms(&z, &y), Some(Ordering::Less), "…consistently, for both members of the class");
        assert_eq!(compare_terms(&x, &z), Some(Ordering::Greater), "antisymmetric");
        // The window is the ONLY thing that changed: a decidable pair still orders by
        // instant, tz-less or not (here 20h apart, outside the ±14h window).
        let far = TzLit { instant: 13 * 3600 - 20 * 3600, has_tz: false, lex: "2024-03-14T17:00:00", total: true };
        assert_eq!(compare_terms(&far, &x), Some(Ordering::Less));
    }
}
