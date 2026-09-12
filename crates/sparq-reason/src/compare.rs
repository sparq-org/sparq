//! [FABLE-5] sq-pbz04.1.2 (epic sq-pbz04.1, substrate seam 3) — the reasoner-side adoption
//! of the SHARED SPARQL term total order: `sparq_substrate::compare::compare_terms` driven
//! by a [`CompareTerm`] implementation for the reasoner's own term representation, a
//! `sparq-core` dictionary [`Id`] resolved against its [`Dict`] ([`IdTerm`]).
//!
//! The SPARQL engine implements the same trait for its private `Value` enum and drives the
//! same `compare_terms` body for `ORDER BY` / `MIN`/`MAX`; this module gives the reasoner
//! the SAME total order over the `(Dict, [Id; 3])` closure representation it materialises,
//! so a consumer that orders ENTAILED solutions (a RIF `order`, an `ORDER BY` over a
//! materialised answer set, a deterministic export) sorts them IDENTICALLY to the engine —
//! one ordering algorithm across the engine AND the reasoners, the seam-3 goal of
//! `research/reasoner-federation-program.md`.
//!
//! # Ordering parity (the load-bearing contract)
//!
//! [`compare_ids`] / [`sort_ids`] order ids exactly as the engine's `ORDER BY` orders the
//! terms those ids denote: error/unbound < blank < IRI < literal < RDF 1.2 triple term,
//! then within each class the engine's arms — blanks/IRIs by string form; literals
//! KIND-FIRST (the sq-wjl8i total-order fix: a fixed `LiteralKind` rank between literal
//! kinds, then within the numeric kind the exact-rational order — NaN first, the
//! `exact_cmp` f64-collapse recheck exact for integers beyond 2^53 / high-precision
//! decimals / the mixed exact-vs-double tie — and within the other kinds strict
//! typed/temporal (`xsd:dateTime`/`xsd:date` by TIMELINE via the shared
//! `sparq_core::temporal::Timeline`, same-tag language strings and same-other-XSD
//! lexically), then the lexical fallback); triple terms component-wise
//! recursively. Every observation hook mirrors the engine's `Value` impl over the SAME
//! shared machinery (`Timeline`, the substrate `Num`/`Dec` tower, `parse_xsd_f64`), and the
//! full-multiset parity is pinned against a REAL engine `ORDER BY` over the same
//! materialised closure by `tests/compare_parity.rs`; the `exact_cmp` collapse-recheck
//! for distinct integers beyond 2^53 is pinned by the unit test
//! `exact_recheck_orders_big_integers_beyond_2_53`.
//!
//! # Zero-overhead + behaviour-neutrality
//!
//! The comparator is adopted MONOMORPHICALLY: [`IdTerm`] is a generic-type-parameter
//! consumer of `compare_terms` — no `Box<dyn>` / `&dyn` / vtable between the sort loop and
//! the term observations (`scripts/check-no-dyn-dispatch.py` guards this file). The module
//! is behind the NON-DEFAULT `substrate-compare` feature and is purely ADDITIVE: nothing in
//! the materialisers calls it, so which triples are entailed and the order they are emitted
//! in are byte-identical in both feature states.
//!
//! # Scope notes (honest boundaries)
//!
//! - Ids must come from the reasoner's id space: dictionary ids plus inline-integer ids
//!   (`is_inline`). The engine's local-vocab ids (computed BIND/aggregate values above the
//!   inline window) never appear in a materialised closure and are not meaningful here.
//! - A `NaN` double is totalised FIRST among numerics (sq-wjl8i) — no numeric pair
//!   reaches the `unwrap_or(Equal)` mapping any more; the mapping remains only for the
//!   no-string-form / non-decomposing-triple `None`s, matching the engine exactly.
//! - Two DISTINCT terms can compare `Equal` under SPARQL's lenient order (equal numeric
//!   values across datatypes, equal instants across timezones, same-lexical unknown-datatype
//!   literals). Their relative order after a stable sort is input-order on both sides —
//!   the same tie semantics the engine has, not a divergence introduced here.

use std::cmp::Ordering;

use sparq_core::dict::{is_inline, split_lang_dir, Dict, Id, TermParts, INLINE_BASE};
use sparq_core::temporal::Timeline;
use sparq_substrate::compare::{compare_terms, CompareTerm, LiteralKind, TermClass};
use sparq_substrate::numeric::{parse_xsd_f64, Num};
#[cfg(test)]
use sparq_substrate::numeric::Dec;

/// The XSD namespace prefix — the engine's `lit_kind` "other XSD datatype" family test.
const XSD_PREFIX: &str = "http://www.w3.org/2001/XMLSchema#";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_FLOAT: &str = "http://www.w3.org/2001/XMLSchema#float";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";
const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const XSD_DATE_TIME_STAMP: &str = "http://www.w3.org/2001/XMLSchema#dateTimeStamp";

/// A reasoner term for the shared SPARQL total order: a dictionary [`Id`] paired with the
/// [`Dict`] that resolves it. Implements the substrate's [`CompareTerm`] so
/// `sparq_substrate::compare::compare_terms` — the engine's `ORDER BY` algorithm — can
/// drive it monomorphically (a generic type parameter, never a trait object).
#[derive(Clone, Copy)]
pub struct IdTerm<'a> {
    dict: &'a Dict,
    id: Id,
}

impl<'a> IdTerm<'a> {
    /// Wraps `id` for comparison against other ids of the SAME `dict`. `id` is a
    /// dictionary or inline-integer id (the reasoner's id space); `0` (`NO_ID`, never
    /// present in a materialised closure) classifies as error/unbound and sorts first,
    /// matching the engine's unbound rank.
    #[inline]
    pub fn new(dict: &'a Dict, id: Id) -> IdTerm<'a> {
        IdTerm { dict, id }
    }

    /// The wrapped dictionary id.
    #[inline]
    pub fn id(&self) -> Id {
        self.id
    }

    /// The typed numeric value for the EXACT tier — mirrors the engine's `as_numeric`
    /// (`Num::of_literal`) over the dict-stored literal parts: `None` for non-numerics,
    /// ill-formed numeric lexicals, and language-tagged literals.
    #[inline]
    fn as_num_typed(self) -> Option<Num> {
        if is_inline(self.id) {
            return Some(Num::Int(i64::from(self.id - INLINE_BASE)));
        }
        if self.id == 0 {
            return None;
        }
        match self.dict.term_parts(self.id) {
            TermParts::Lit { value, datatype, lang: None } => num_of_parts(value, datatype),
            _ => None,
        }
    }

    /// The literal-family classification the strict arm compares within — the engine's
    /// `lit_kind` over the dict-stored parts instead of an `oxrdf::Literal`.
    fn kind(self) -> Kind<'a> {
        if self.id == 0 {
            return Kind::NotLiteral;
        }
        if is_inline(self.id) {
            // An inline id IS a canonical non-negative xsd:integer value.
            return Kind::Num(Some(Num::Int(i64::from(self.id - INLINE_BASE))));
        }
        match self.dict.term_parts(self.id) {
            TermParts::Lit { value, datatype, lang } => {
                if let Some(slot) = lang {
                    // The stored slot may carry an RDF 1.2 base direction (`en--ltr`);
                    // the engine compares by the BCP47 tag only (case-insensitively),
                    // exactly as `oxrdf::Literal::language()` surfaces it.
                    let (tag, _dir) = split_lang_dir(slot);
                    return Kind::Lang(tag.to_ascii_lowercase(), value);
                }
                if is_numeric_dt(datatype) {
                    Kind::Num(num_of_parts(value, datatype))
                } else if datatype == XSD_STRING {
                    Kind::Str(value)
                } else if datatype == XSD_BOOLEAN {
                    Kind::Bool(parse_bool(value))
                } else if datatype == XSD_DATE_TIME || datatype == XSD_DATE_TIME_STAMP {
                    Kind::DateTime(Timeline::parse_datetime(value))
                } else if datatype == XSD_DATE {
                    Kind::Date(Timeline::parse_date(value))
                } else if datatype.starts_with(XSD_PREFIX) {
                    Kind::OtherXsd(datatype, value)
                } else {
                    Kind::Unknown
                }
            }
            _ => Kind::NotLiteral,
        }
    }
}

/// Datatype family of a literal term — the engine's `LitKind` over borrowed dict parts.
/// Only same-family pairs decide strictly; everything else falls back exactly as the
/// engine's `value_compare_strict` does.
enum Kind<'a> {
    /// A numeric-datatype literal; `None` = ill-formed lexical.
    Num(Option<Num>),
    /// `xsd:string`.
    Str(&'a str),
    /// `xsd:boolean`; `None` = ill-formed lexical.
    Bool(Option<bool>),
    /// `xsd:dateTime` / `xsd:dateTimeStamp` on the timeline; `None` = ill-formed.
    DateTime(Option<Timeline>),
    /// `xsd:date` on the timeline (midnight); `None` = ill-formed.
    Date(Option<Timeline>),
    /// Another XSD datatype (time, gYear, duration, …): (datatype IRI, lexical).
    OtherXsd(&'a str, &'a str),
    /// Language-tagged: (lowercased BCP47 tag, lexical value).
    Lang(String, &'a str),
    /// A literal of a non-XSD (unknown) datatype: open-world, never strictly decidable.
    Unknown,
    /// Not a literal (IRI / blank / triple / the id-0 sentinel) — never reaches a
    /// deciding strict arm.
    NotLiteral,
}

/// The engine's `is_numeric_dt` over a datatype-IRI string: the integer family plus
/// decimal / double / float — the datatypes the LENIENT numeric arm coerces to `f64`.
#[inline]
fn is_numeric_dt(dt: &str) -> bool {
    sparq_core::is_integer_datatype(dt) || dt == XSD_DECIMAL || dt == XSD_DOUBLE || dt == XSD_FLOAT
}

/// The engine's boolean-literal lexical acceptance (`as_bool_val`): the XSD canonical and
/// numeric forms; anything else is an ill-formed boolean (never strictly decidable).
#[inline]
fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// [GPT-6] Borrowed literal parsing shares the substrate implementation and validity rules.
#[inline]
fn num_of_parts(value: &str, datatype: &str) -> Option<Num> {
    Num::of_parts(value, datatype)
}

/// The numeric relational comparison for `strict_cmp` — delegates to
/// `Num::cmp_relational` (the shared substrate function, sq-v5evr): exact via the
/// `Dec` fixed-point tower when both operands have an exact tier (int/decimal), else
/// by `f64` value (float/double). NaN → `None` (SPARQL type error). [OPUS-4.8] sq-v5evr
#[inline]
fn num_compare(a: Num, b: Num) -> Option<Ordering> {
    a.cmp_relational(b)
}

impl CompareTerm for IdTerm<'_> {
    #[inline]
    fn term_class(&self) -> TermClass {
        if self.id == 0 {
            return TermClass::ErrorOrUnbound;
        }
        if is_inline(self.id) {
            return TermClass::Literal; // an inline id is an xsd:integer literal
        }
        match self.dict.term_parts(self.id) {
            TermParts::Blank(_) => TermClass::Blank,
            TermParts::Iri { .. } => TermClass::Iri,
            TermParts::Lit { .. } => TermClass::Literal,
            TermParts::Triple(_) => TermClass::Triple,
        }
    }

    #[inline]
    fn value_str(&self) -> Option<String> {
        if self.id == 0 {
            return None;
        }
        if is_inline(self.id) {
            return Some((self.id - INLINE_BASE).to_string());
        }
        match self.dict.term_parts(self.id) {
            // The engine's `value_str`: full IRI string, literal lexical value, blank label.
            TermParts::Iri { prefix, suffix } => Some(format!("{}{}", prefix, suffix)),
            TermParts::Lit { value, .. } => Some(value.to_string()),
            TermParts::Blank(b) => Some(b.to_string()),
            TermParts::Triple(_) => None, // triple terms compare component-wise, never by string
        }
    }

    #[inline]
    fn as_f64(&self) -> Option<f64> {
        if is_inline(self.id) {
            return Some(f64::from(self.id - INLINE_BASE));
        }
        if self.id == 0 {
            return None;
        }
        match self.dict.term_parts(self.id) {
            // [FABLE-5] sq-74oy4 / sq-6b1lj: the engine's lenient `as_num` arm, now
            // DATATYPE-AWARE and TRIMMED to match `Num::of_literal`: accept iff the lexical is
            // well-formed FOR its datatype (`num_of_parts` — the borrowed-parts twin of
            // `of_literal`), imaged by `parse_xsd_f64` on the trimmed value. A padded
            // `" 1"^^xsd:integer` is value-1; a per-datatype-ill-formed `"1.5"^^xsd:integer`
            // is `None` (type error), mirroring the engine seam and the graph cache. The XSD
            // f64 spellings (INF/-INF/NaN, not inf/infinity) are enforced by `parse_xsd_f64`.
            TermParts::Lit { value, datatype, lang: None }
                if is_numeric_dt(datatype) && num_of_parts(value, datatype).is_some() =>
            {
                parse_xsd_f64(value.trim())
            }
            _ => None,
        }
    }

    #[inline]
    fn literal_kind(&self) -> LiteralKind {
        // [FABLE-5] sq-wjl8i / sq-74oy4: the kind-first rank — mirrors the engine's
        // `Value::literal_kind` exactly: Numeric tracks the `as_f64` membership, now
        // DATATYPE-AWARE (a lexical ill-formed FOR its datatype like "1.5"^^xsd:integer is
        // NOT numeric and sorts as Other/lexical, matching `of_literal`); ill-formed
        // numeric/temporal lexicals classify as Other (a kind mixing value-ordered and
        // lexical-fallback pairs is intransitive).
        if is_inline(self.id) {
            return LiteralKind::Numeric; // an inline id IS a well-formed xsd:integer
        }
        match self.kind() {
            Kind::Bool(_) => LiteralKind::Boolean,
            Kind::Num(_) if self.as_f64().is_some() => LiteralKind::Numeric,
            Kind::Str(_) => LiteralKind::String,
            Kind::Lang(..) => LiteralKind::Lang,
            Kind::DateTime(Some(_)) => LiteralKind::DateTime,
            Kind::Date(Some(_)) => LiteralKind::Date,
            // Ill-formed numerics/temporals, other-XSD, unknown datatypes (and the
            // unreachable non-literal case — `compare_terms` gates on the class).
            _ => LiteralKind::Other,
        }
    }

    #[inline]
    fn exact_cmp(&self, other: &Self) -> Option<Ordering> {
        // The f64-collapse recheck (called by `compare_terms` only on an f64 tie),
        // mirroring the engine's `Value::exact_cmp` → `Num::cmp_total` (sq-wjl8i): the
        // EXACT-RATIONAL total order — exact for int/decimal pairs AND for the mixed
        // exact/inexact pair (against the double's exact decimal expansion), NaN
        // totalised first — so a reasoner-side sort agrees with the engine's ORDER BY.
        match (self.as_num_typed(), other.as_num_typed()) {
            (Some(a), Some(b)) => Some(a.cmp_total(b)),
            _ => None,
        }
    }

    #[inline]
    fn strict_cmp(&self, other: &Self) -> Option<Ordering> {
        // The engine's `value_compare_strict`: only same-family pairs decide.
        use Kind::*;
        match (self.kind(), other.kind()) {
            (Num(Some(a)), Num(Some(b))) => num_compare(a, b),
            (Str(a), Str(b)) => Some(a.cmp(b)),
            (Bool(Some(a)), Bool(Some(b))) => Some(a.cmp(&b)),
            // [SONNET-4.6] sq-2k5py: the TOTAL timeline order, mirroring the engine's
            // `CompareTerm::strict_cmp` (which extends `value_compare_strict` the same way).
            // `Timeline::cmp_tl`'s indeterminate mixed-timezone window would drop the pair to
            // `compare_terms`' lexical fallback INSIDE the DateTime/Date kind, which is
            // intransitive; the reasoner has no relational operators, so the total order is
            // the only consumer here.
            (DateTime(Some(a)), DateTime(Some(b))) => Some(Timeline::cmp_tl_total(a, b)),
            (Date(Some(a)), Date(Some(b))) => Some(Timeline::cmp_tl_total(a, b)),
            // Same language tag (case-insensitive): compare values (the suites' lenient
            // extension the engine applies).
            (Lang(t1, v1), Lang(t2, v2)) if t1 == t2 => Some(v1.cmp(v2)),
            // Same other-XSD datatype: lexical order (correct for time, gYear, …).
            (OtherXsd(d1, l1), OtherXsd(d2, l2)) if d1 == d2 => Some(l1.cmp(l2)),
            _ => None,
        }
    }

    #[inline]
    fn triple_parts(&self) -> Option<[Self; 3]> {
        if self.id == 0 || is_inline(self.id) {
            return None;
        }
        match self.dict.term_parts(self.id) {
            // A dict triple term stores its component IDS — already this very term type,
            // so the generic recursion needs no reconstruction at all (the predicate id
            // resolves to an IRI, comparing by IRI string exactly like the engine).
            TermParts::Triple([s, p, o]) => {
                Some([IdTerm::new(self.dict, s), IdTerm::new(self.dict, p), IdTerm::new(self.dict, o)])
            }
            _ => None,
        }
    }
}

/// Compares two ids of `dict` under the engine's `ORDER BY` total order (a `NaN`
/// operand takes its fixed first-among-numerics position, sq-wjl8i). The rare
/// undecidable pairs (no string form / a non-decomposing triple) collapse to `Equal`,
/// exactly as the engine's sort maps its comparator's `None`.
#[inline]
pub fn compare_ids(dict: &Dict, a: Id, b: Id) -> Ordering {
    compare_terms(&IdTerm::new(dict, a), &IdTerm::new(dict, b)).unwrap_or(Ordering::Equal)
}

/// Sorts a slice of solution ids ascending under the engine's `ORDER BY` total order
/// ([`compare_ids`]), with a STABLE sort — ties (equal-comparing terms) keep their input
/// order, the same tie semantics as the engine's sort.
#[inline]
pub fn sort_ids(dict: &Dict, ids: &mut [Id]) {
    ids.sort_by(|&a, &b| compare_ids(dict, a, b));
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{Literal, NamedNode, Term, Triple};

    fn iri(d: &mut Dict, s: &str) -> Id {
        d.intern_iri(s)
    }
    fn lit(d: &mut Dict, v: &str, dt: &str) -> Id {
        d.intern_lit(v, dt, None)
    }

    /// Direct test of the [`IdTerm::new`] / [`IdTerm::id`] pair (coverage-floor rule).
    #[test]
    fn id_term_new_wraps_and_returns_its_id() {
        let mut d = Dict::new();
        let a = iri(&mut d, "http://ex/a");
        let t = IdTerm::new(&d, a);
        assert_eq!(t.id(), a);
        assert_eq!(t.term_class(), TermClass::Iri);
    }

    /// Every dict term kind lands in the engine's precedence class; id 0 (`NO_ID`) is the
    /// error/unbound rank; an inline-integer id is a literal.
    #[test]
    fn term_class_covers_every_dict_term_kind() {
        let mut d = Dict::new();
        let b = d.intern_blank("b0");
        let i = iri(&mut d, "http://ex/i");
        let l = lit(&mut d, "x", XSD_STRING);
        let lang = d.intern_lit("chat", "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString", Some("fr"));
        let inline = lit(&mut d, "7", "http://www.w3.org/2001/XMLSchema#integer");
        assert!(is_inline(inline), "canonical small xsd:integer must inline");
        let tt = d.intern(&Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked("http://ex/s"),
            NamedNode::new_unchecked("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("o")),
        ))));
        let class = |id| IdTerm::new(&d, id).term_class();
        assert_eq!(class(0), TermClass::ErrorOrUnbound);
        assert_eq!(class(b), TermClass::Blank);
        assert_eq!(class(i), TermClass::Iri);
        assert_eq!(class(l), TermClass::Literal);
        assert_eq!(class(lang), TermClass::Literal);
        assert_eq!(class(inline), TermClass::Literal);
        assert_eq!(class(tt), TermClass::Triple);
    }

    /// `value_str` matches the engine's forms: full IRI, literal lexical, blank label,
    /// inline decimal string; a triple term has no string form.
    #[test]
    fn value_str_matches_engine_string_forms() {
        let mut d = Dict::new();
        let i = iri(&mut d, "http://ex/ns#thing");
        let l = lit(&mut d, "bob", XSD_STRING);
        let b = d.intern_blank("alice");
        let inline = lit(&mut d, "42", "http://www.w3.org/2001/XMLSchema#integer");
        let tt = d.intern(&Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked("http://ex/s"),
            NamedNode::new_unchecked("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("o")),
        ))));
        let vs = |id| IdTerm::new(&d, id).value_str();
        assert_eq!(vs(i).as_deref(), Some("http://ex/ns#thing"));
        assert_eq!(vs(l).as_deref(), Some("bob"));
        assert_eq!(vs(b).as_deref(), Some("alice"));
        assert_eq!(vs(inline).as_deref(), Some("42"));
        assert_eq!(vs(tt), None);
        assert_eq!(vs(0), None);
    }

    /// Direct test of [`compare_ids`]: numeric literals order by VALUE (9 < 10 despite
    /// "10" < "9" lexically), across inline and stored ids and across the numeric tower.
    #[test]
    fn compare_ids_orders_numerics_by_value_not_lexical() {
        let mut d = Dict::new();
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let nine = lit(&mut d, "9", int); // inline
        let ten = lit(&mut d, "10", int); // inline
        let neg = lit(&mut d, "-3", int); // stored (negative never inlines)
        let dec = lit(&mut d, "9.5", XSD_DECIMAL);
        let dbl = lit(&mut d, "9.75E0", XSD_DOUBLE);
        assert!(is_inline(nine) && is_inline(ten) && !is_inline(neg));
        assert_eq!(compare_ids(&d, nine, ten), Ordering::Less);
        assert_eq!(compare_ids(&d, neg, nine), Ordering::Less);
        assert_eq!(compare_ids(&d, nine, dec), Ordering::Less);
        assert_eq!(compare_ids(&d, dec, dbl), Ordering::Less);
        assert_eq!(compare_ids(&d, dbl, ten), Ordering::Less);
        // NaN is totalised FIRST among numerics (sq-wjl8i): before every numeric,
        // equal to itself — no longer an undecidable pair collapsing to Equal.
        let nan = lit(&mut d, "NaN", XSD_DOUBLE);
        assert_eq!(compare_ids(&d, nan, nine), Ordering::Less);
        assert_eq!(compare_ids(&d, nine, nan), Ordering::Greater);
        assert_eq!(compare_ids(&d, nan, nan), Ordering::Equal);
        let ninf = lit(&mut d, "-INF", XSD_DOUBLE);
        assert_eq!(compare_ids(&d, nan, ninf), Ordering::Less);
    }

    /// [FABLE-5] sq-wjl8i: cross-KIND literal pairs rank by `LiteralKind` (numeric <
    /// boolean < dateTime < date < string < lang < other), never by the lexical form —
    /// the digit-inversion triple `10 / "11" / 2` that was intransitive under the
    /// lexical fallback is consistent under the rank. And the mixed exact-vs-double tie
    /// at the 2^53 collapse now orders exactly.
    #[test]
    fn kind_first_rank_and_exact_mixed_tier() {
        let mut d = Dict::new();
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let ten = lit(&mut d, "10", int);
        let two = lit(&mut d, "2", int);
        let s11 = lit(&mut d, "11", XSD_STRING);
        assert_eq!(compare_ids(&d, ten, s11), Ordering::Less, "Numeric < String by kind");
        assert_eq!(compare_ids(&d, s11, two), Ordering::Greater, "String > Numeric (was lexical Less)");
        assert_eq!(compare_ids(&d, ten, two), Ordering::Greater, "10 > 2 — consistent");
        // Mixed exact/inexact at the collapse: double(2^53) equals int 2^53 exactly,
        // and sits strictly below int 2^53+1 (was a three-way Equal tie).
        let big = lit(&mut d, "9007199254740992", int);
        let big1 = lit(&mut d, "9007199254740993", int);
        let dbl = lit(&mut d, "9007199254740992E0", XSD_DOUBLE);
        assert_eq!(compare_ids(&d, big, dbl), Ordering::Equal);
        assert_eq!(compare_ids(&d, dbl, big1), Ordering::Less);
        assert_eq!(compare_ids(&d, big1, dbl), Ordering::Greater);
    }

    /// The f64-collapse `exact_cmp` recheck: distinct integers beyond 2^53 share one f64
    /// yet still order by exact value (this FAILS if `exact_cmp` is mis-wired to `None`).
    #[test]
    fn exact_recheck_orders_big_integers_beyond_2_53() {
        let mut d = Dict::new();
        let int = "http://www.w3.org/2001/XMLSchema#integer";
        let a = lit(&mut d, "9007199254740992", int);
        let b = lit(&mut d, "9007199254740993", int);
        let (ta, tb) = (IdTerm::new(&d, a), IdTerm::new(&d, b));
        assert_eq!(ta.as_f64(), tb.as_f64(), "the f64 keys must collapse for this pin");
        assert_eq!(compare_ids(&d, a, b), Ordering::Less);
        assert_eq!(compare_ids(&d, b, a), Ordering::Greater);
        assert_eq!(compare_ids(&d, a, a), Ordering::Equal);
    }

    /// The strict arm decides dateTime by TIMELINE (cross-timezone), not lexically —
    /// the pair below flips if `strict_cmp` degrades to the string fallback. Booleans,
    /// same-tag language strings and same-other-XSD literals decide strictly too.
    #[test]
    fn strict_arm_timeline_bool_lang_and_other_xsd() {
        let mut d = Dict::new();
        // 14:00+01:00 == 13:00Z, which is BEFORE 13:30Z; lexically it sorts AFTER.
        let t1 = lit(&mut d, "2024-03-15T14:00:00+01:00", XSD_DATE_TIME);
        let t2 = lit(&mut d, "2024-03-15T13:30:00Z", XSD_DATE_TIME);
        assert_eq!(compare_ids(&d, t1, t2), Ordering::Less);
        assert_eq!(compare_ids(&d, t2, t1), Ordering::Greater);
        let d1 = lit(&mut d, "2024-03-14", XSD_DATE);
        let d2 = lit(&mut d, "2024-03-15", XSD_DATE);
        assert_eq!(compare_ids(&d, d1, d2), Ordering::Less);
        // Booleans: false < true; the numeric spelling "1" is the SAME value as "true"
        // (strict Equal — the engine's lenient acceptance), not a lexical compare.
        let f = lit(&mut d, "false", XSD_BOOLEAN);
        let t = lit(&mut d, "true", XSD_BOOLEAN);
        let one = lit(&mut d, "1", XSD_BOOLEAN);
        assert_eq!(compare_ids(&d, f, t), Ordering::Less);
        assert_eq!(compare_ids(&d, one, t), Ordering::Equal);
        // Language strings: the tag matches case-insensitively, then values compare.
        let en1 = d.intern_lit("apple", "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString", Some("en"));
        let en2 = d.intern_lit("banana", "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString", Some("EN"));
        assert_eq!(compare_ids(&d, en1, en2), Ordering::Less);
        // Same other-XSD datatype: lexical order.
        let y1 = lit(&mut d, "2020", "http://www.w3.org/2001/XMLSchema#gYear");
        let y2 = lit(&mut d, "2021", "http://www.w3.org/2001/XMLSchema#gYear");
        assert_eq!(compare_ids(&d, y1, y2), Ordering::Less);
        // Unknown datatypes never decide strictly -> lexical string fallback.
        let u1 = lit(&mut d, "alpha", "http://ex/custom");
        let u2 = lit(&mut d, "beta", "http://ex/custom");
        assert_eq!(compare_ids(&d, u1, u2), Ordering::Less);
    }

    /// Cross-class precedence over real ids: blank < IRI < literal < triple term.
    #[test]
    fn cross_class_precedence_over_ids() {
        let mut d = Dict::new();
        let b = d.intern_blank("zzz");
        let i = iri(&mut d, "http://ex/aaa");
        let l = lit(&mut d, "aaa", XSD_STRING);
        let tt = d.intern(&Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked("http://ex/aaa"),
            NamedNode::new_unchecked("http://ex/aaa"),
            Term::Literal(Literal::new_simple_literal("aaa")),
        ))));
        assert_eq!(compare_ids(&d, b, i), Ordering::Less);
        assert_eq!(compare_ids(&d, i, l), Ordering::Less);
        assert_eq!(compare_ids(&d, l, tt), Ordering::Less);
        assert_eq!(compare_ids(&d, 0, b), Ordering::Less); // unbound sorts first
    }

    /// RDF 1.2 triple terms order component-wise through the STRUCTURAL dict storage
    /// (component ids, recursing without reconstruction), including an inline-id object.
    #[test]
    fn triple_terms_order_componentwise_via_component_ids() {
        let mut d = Dict::new();
        let mk = |d: &mut Dict, o: &str| {
            d.intern(&Term::Triple(Box::new(Triple::new(
                NamedNode::new_unchecked("http://ex/s"),
                NamedNode::new_unchecked("http://ex/p"),
                Term::Literal(Literal::new_typed_literal(o, NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"))),
            ))))
        };
        let one = mk(&mut d, "1");
        let two = mk(&mut d, "2");
        assert_eq!(compare_ids(&d, one, two), Ordering::Less);
        assert_eq!(compare_ids(&d, two, one), Ordering::Greater);
        assert_eq!(compare_ids(&d, one, one), Ordering::Equal);
        // Predicate decides before object.
        let other_p = d.intern(&Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked("http://ex/s"),
            NamedNode::new_unchecked("http://ex/q"),
            Term::Literal(Literal::new_typed_literal("0", NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"))),
        ))));
        assert_eq!(compare_ids(&d, two, other_p), Ordering::Less); // p < q
    }

    /// Direct test of [`sort_ids`]: a mixed vector lands in the engine's ascending
    /// class-then-value order, and the sort is stable for equal-comparing ids.
    #[test]
    fn sort_ids_produces_engine_ascending_order() {
        let mut d = Dict::new();
        let b = d.intern_blank("n1");
        let i = iri(&mut d, "http://ex/x");
        let nine = lit(&mut d, "9", "http://www.w3.org/2001/XMLSchema#integer");
        let ten = lit(&mut d, "10", "http://www.w3.org/2001/XMLSchema#integer");
        let s = lit(&mut d, "zzz", XSD_STRING);
        let mut ids = vec![s, ten, i, nine, b];
        sort_ids(&d, &mut ids);
        assert_eq!(ids, vec![b, i, nine, ten, s]);
        // Stability: duplicate ids keep their (indistinguishable) placement; a re-sort
        // of an already-sorted slice is the identity.
        let snapshot = ids.clone();
        sort_ids(&d, &mut ids);
        assert_eq!(ids, snapshot);
    }

    /// ANTI-DRIFT pin: `num_of_parts` (the borrowed-parts mirror) agrees with the
    /// substrate's `Num::of_literal` over a lexical × datatype matrix, including the
    /// ill-formed and beyond-i64 corners. If the substrate dispatch ever changes, this
    /// fails loudly instead of the mirror silently diverging.
    #[test]
    fn num_of_parts_matches_substrate_of_literal() {
        let cases: &[(&str, &str)] = &[
            ("9", "http://www.w3.org/2001/XMLSchema#integer"),
            ("-3", "http://www.w3.org/2001/XMLSchema#integer"),
            (" 12 ", "http://www.w3.org/2001/XMLSchema#integer"), // trimmed
            ("99999999999999999999999999", "http://www.w3.org/2001/XMLSchema#integer"), // beyond i64
            ("1.5", "http://www.w3.org/2001/XMLSchema#integer"),  // ill-formed integer
            ("9.50", XSD_DECIMAL), // scale-preserving
            ("abc", XSD_DECIMAL),  // ill-formed
            ("1.5E0", XSD_DOUBLE),
            ("INF", XSD_DOUBLE),
            ("inf", XSD_DOUBLE), // rejected non-XSD spelling
            ("2.5", XSD_FLOAT),
            ("hello", XSD_STRING), // non-numeric datatype
            ("7", "http://www.w3.org/2001/XMLSchema#byte"), // derived integer type
            // [GPT-6] Facets and XML-only whitespace must also match the shared parser.
            ("1200", "http://www.w3.org/2001/XMLSchema#byte"),
            ("5.0", "http://www.w3.org/2001/XMLSchema#integer"),
            ("-1", "http://www.w3.org/2001/XMLSchema#unsignedLong"),
            ("\u{a0}5", "http://www.w3.org/2001/XMLSchema#integer"),
            ("\u{a0}5", XSD_DECIMAL),
            ("\u{a0}5", XSD_DOUBLE),
            ("é", "http://www.w3.org/2001/XMLSchema#integer"),
        ];
        for (v, dt) in cases {
            let l = Literal::new_typed_literal(*v, NamedNode::new_unchecked(*dt));
            assert_eq!(
                format!("{:?}", num_of_parts(v, dt)),
                format!("{:?}", Num::of_literal(&l)),
                "num_of_parts drifted from Num::of_literal for {:?}^^{:?}",
                v,
                dt
            );
        }
    }

    /// Direct pin of `num_compare` → `Num::cmp_relational` (sq-v5evr): verifies the
    /// delegation produces the expected relational semantics — exact for int/dec pairs,
    /// f64 promotion for mixed/inexact, and `None` for NaN (SPARQL type error).
    /// [OPUS-4.8] sq-v5evr
    #[test]
    fn num_compare_delegates_to_substrate_cmp_relational() {
        use std::cmp::Ordering::*;
        // Exact same-tier: integer vs integer
        assert_eq!(num_compare(Num::Int(1), Num::Int(2)), Some(Less));
        assert_eq!(num_compare(Num::Int(3), Num::Int(3)), Some(Equal));
        assert_eq!(num_compare(Num::Int(5), Num::Int(4)), Some(Greater));
        // Exact cross-tier: int vs decimal
        let dec = Num::Dec(Dec { mant: 10, scale: 1 }); // 1.0
        assert_eq!(num_compare(Num::Int(1), dec), Some(Equal));
        // Double: normal
        assert_eq!(num_compare(Num::Double(1.5), Num::Double(2.5)), Some(Less));
        // NaN → None (SPARQL type error — the relational semantics, not cmp_total)
        assert_eq!(num_compare(Num::Double(f64::NAN), Num::Int(0)), None);
        assert_eq!(num_compare(Num::Int(0), Num::Double(f64::NAN)), None);
    }
}
