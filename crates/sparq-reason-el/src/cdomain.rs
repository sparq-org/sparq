// [FABLE-5] sq-pbz04.2.2 (epic sq-pbz04.2): CR7–CR9 — concrete domains (faceted datatype
// restrictions) on the SHARED numeric value tower (`sparq_substrate::numeric`).
//
// # What this module decides
//
// The core question is FACET SATISFIABILITY: given `owl:onDatatype` (a base datatype) plus
// an `owl:withRestrictions` facet list, is the restricted value space EMPTY (unsatisfiable),
// and how do two such value spaces relate (containment)? Both questions are decided EXACTLY
// on the substrate's exact numeric tier (`Dec` — i128 fixed-point; `i64` integers), never on
// a lossy `f64` — a wrong sat/unsat verdict would be an UNSOUND entailment, so anything not
// exactly decidable is DEFERRED (the enclosing axiom stays in `Report::skipped_axioms`).
//
// # The encoding (why classify.rs needs no changes)
//
// Every supported range is reduced to a canonical [`NormRange`] and minted as a FRESH
// internal concept (no dict id — it can never surface in the emitted lattice). The
// concrete-domain completion rules then become ORDINARY `Normal` axioms over those concepts:
//
//   CR7  (clash)        value-space(d) = ∅          ⇒  d ⊑ ⊥
//   CR8  (implication)  value-space(d1) ⊆ value-space(d2)  ⇒  d1 ⊑ d2
//   CR9  (point ranges) DataHasValue(p, v) / DataOneOf(v)  =  ∃p.{v} with {v} a point range
//
// (CR7–CR9 is this repo's numbering for the concrete-domain rules of the Baader–Brandt–Lutz
// EL++ calculus — `research/owl2-el-ql-reasoning-spike.md`; the paper phrases them as
// satisfiability/implication oracles over conjunctions of datatype constraints, which is
// exactly what [`finalize`] and [`contains`] compute for the supported fragment.) The
// existing saturation does the rest: an empty range reached through `C ⊑ ∃p.d` propagates ⊥
// to `C` via CR5, and a containment `d1 ⊑ d2` threads through data-property existentials via
// CR1/CR3/CR4 exactly like a class filler. Every emitted axiom is a TRUE statement about
// fixed sets of data values in every model, so composing them with CR1–CR6/CR10/CR11 stays
// sound (the concrete domain only further constrains the models).
//
// # Supported vs deferred (the honest boundary)
//
// SUPPORTED — verdicts are computed, `skipped_axioms` drops for exactly these:
//   * base datatypes: `xsd:decimal`, `xsd:integer` and the 12 derived integer types
//     (long/int/short/byte, the four unsigned forms, the four sign-constrained forms) with
//     their implicit min/max folded into the interval (so `xsd:byte` + `minInclusive 1000`
//     is correctly UNSAT);
//   * facets: `xsd:minInclusive` / `xsd:maxInclusive` / `xsd:minExclusive` /
//     `xsd:maxExclusive`, each valued by an exact-tier numeric literal (integer-family or
//     `xsd:decimal` lexical, validated against ITS OWN datatype's bounds);
//   * point forms: a LITERAL-valued `owl:hasValue` (DataHasValue) and a SINGLETON literal
//     `owl:oneOf` (DataOneOf) over the same exact tier.
//
// DEFERRED — no verdict is ever produced; the enclosing axiom keeps the pre-cdomain skip:
//   * any other facet (`xsd:pattern`, `length`/`minLength`/`maxLength`, `totalDigits`/
//     `fractionDigits`, …) — and an unknown facet defers the WHOLE range, because ignoring
//     a constraint would compute a SUPERSET and could fabricate a containment verdict;
//   * `xsd:float`/`xsd:double` bases or bound values (representation-boundary honesty: a
//     double `0.1` is NOT the decimal `0.1`, and float emptiness needs next-after
//     reasoning), plus every non-numeric datatype (strings, dateTime, …);
//   * `owl:onDataRange` (qualified-cardinality vocabulary — cardinality is outside EL
//     entirely) and `owl:datatypeComplementOf` (negation);
//   * an exact-tier comparison the i128 fixed-point cannot align (`Dec::cmp → None`);
//   * a candidate node carrying ANY other class-expression structure (see the strictness
//     guard in `extract::resolve_cdomain`).
//
// Known INCOMPLETENESS (sound — a missing derivation, never a wrong one): a decimal-sorted
// range is not derived ⊆ an integer-sorted one (only exact points collapse to the integer
// sort during normalization), and a plain (facet-free) datatype IRI in filler position keeps
// its historical opaque-named-class treatment rather than becoming an unbounded range.

use crate::normal::{Concept, Names, Normal, BOTTOM};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{is_inline, Dict, Id, TermParts, INLINE_BASE};
use sparq_substrate::numeric::{as_numeric, Dec, Num, RoundMode};
use std::cmp::Ordering;

/// A decimal-sorted bound during folding: `(value, exclusive)`, `None` = unbounded.
type DecBound = Option<(Dec, bool)>;

/// The value-space SORT of a supported base datatype. Integer sorts (the whole derived
/// ladder) share one canonical interval form; `xsd:decimal` keeps dense-order bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sort {
    /// `xsd:integer` and its derived types — a discrete order, so exclusive bounds
    /// TIGHTEN to inclusive integer bounds exactly.
    Integer,
    /// `xsd:decimal` — a DENSE order (between any two distinct decimals lies another),
    /// so exclusivity must be carried, and `(a, b)` with `a < b` is never empty.
    Decimal,
}

/// A CANONICAL faceted numeric range — the dedup key and the comparison form. Two ranges
/// with the same `NormRange` have the same value space and share one minted concept.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NormRange {
    /// A provably EMPTY value space (every empty range canonicalizes here, any sort) —
    /// the CR7 unsat verdict.
    Empty,
    /// The integers in `[lo, hi]` (inclusive; `None` = unbounded on that side). Non-empty
    /// by construction. An integral point (`{5}`, `{5.0}`) also canonicalizes here.
    Int { lo: Option<i128>, hi: Option<i128> },
    /// `xsd:decimal` values between the bounds; each bound is `(mant, scale, exclusive)`
    /// with the mantissa SCALE-MINIMIZED (`dec_norm`) so `1.50` and `1.5` key equal.
    /// Non-empty and non-integral-point by construction (see [`finalize`]).
    Dec {
        lo: Option<(i128, u32, bool)>,
        hi: Option<(i128, u32, bool)>,
    },
}

/// Everything `resolve` hands back to the extractor: the node → concept maps `decode`
/// consults, plus the CR7/CR8 axioms over the minted range concepts.
pub(crate) struct Resolved {
    /// Faceted-range / singleton-`DataOneOf` node → its range concept (an `Expr::Atom`).
    pub node_range: FxHashMap<Id, Concept>,
    /// Literal-`owl:hasValue` restriction node → its POINT-range concept (the caller
    /// wraps it as `∃p.{v}` using the node's `owl:onProperty`).
    pub node_exists: FxHashMap<Id, Concept>,
    /// [SONNET-4.6] sq-vkq9u (`abox` + `cdomain`): data-property-assertion LITERAL id → its
    /// POINT-range concept (the caller wraps it as `{a} ⊑ ∃q.{v}`). Only ever populated for the
    /// literals handed in as `abox_points`; a literal outside the exact numeric tier is ABSENT,
    /// so the caller keeps its fail-closed counted skip.
    pub lit_point: FxHashMap<Id, Concept>,
    /// `d ⊑ ⊥` (CR7) and `d1 ⊑ d2` (CR8) over the minted concepts.
    pub axioms: Vec<Normal>,
}

/// Resolves the pre-screened concrete-domain candidates into minted range concepts +
/// CR7/CR8 axioms. `ranges` = `(node, owl:onDatatype object, owl:withRestrictions list
/// head)`; `points` = singleton-`DataOneOf` `(node, literal)`; `exists_points` =
/// literal-`hasValue` `(restriction node, literal)`; [SONNET-4.6] sq-vkq9u `abox_points` =
/// the bare LITERALS of `DataPropertyAssertion`s to rescue as point ranges (empty unless the
/// caller asked for ABox internalization); `list` walks an RDF list. A candidate that is not
/// supported is simply ABSENT from the output maps (the caller's decode then records the
/// ordinary skip) — deferral, never a guessed verdict.
// The four candidate slices are DELIBERATELY separate parameters: each is pre-screened by its own
// arm of `extract::resolve_cdomain`'s strictness guard, and collapsing them would let a caller mix
// a `hasValue` node into the faceted-range set. Same rationale as the two `#[allow]`s in
// classify.rs. [SONNET-4.6] sq-vkq9u
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve(
    dict: &Dict,
    triples: &[[Id; 3]],
    names: &mut Names,
    ranges: &[(Id, Id, Id)],
    points: &[(Id, Id)],
    exists_points: &[(Id, Id)],
    abox_points: &[Id],
    list: impl Fn(Id) -> Vec<Id>,
) -> Resolved {
    let mut out = Resolved {
        node_range: FxHashMap::default(),
        node_exists: FxHashMap::default(),
        lit_point: FxHashMap::default(),
        axioms: Vec::new(),
    };
    if ranges.is_empty()
        && points.is_empty()
        && exists_points.is_empty()
        && abox_points.is_empty()
    {
        return out; // O(1) fast path: no concrete-domain structure in the TBox or ABox.
    }
    let facets = FacetVocab::intern(dict);
    // Walk each withRestrictions list once, then index every facet node's triples in ONE
    // scan (facet predicates are open vocabulary, so the main extraction pass cannot know
    // which subjects are facet nodes).
    let lists: Vec<(Id, Id, Vec<Id>)> = ranges.iter().map(|&(n, dt, h)| (n, dt, list(h))).collect();
    let mut facet_nodes: FxHashSet<Id> = FxHashSet::default();
    for (_, _, members) in &lists {
        facet_nodes.extend(members.iter().copied());
    }
    let mut facet_triples: FxHashMap<Id, Vec<(Id, Id)>> = FxHashMap::default();
    if !facet_nodes.is_empty() {
        for &[s, p, o] in triples {
            if facet_nodes.contains(&s) {
                facet_triples.entry(s).or_default().push((p, o));
            }
        }
    }
    let mut mint = Mint::default();
    'ranges: for (node, dt, members) in &lists {
        if members.is_empty() {
            continue; // empty / malformed / cyclic facet list → defer.
        }
        let Some((sort, ilo, ihi)) = datatype_iri(dict, *dt).as_deref().and_then(base_of) else {
            continue; // unsupported or non-IRI base datatype → defer.
        };
        // Collect the facet constraints, deferring the WHOLE range on any unknown facet,
        // non-exact value, or facet-less member node (a partial read would compute a
        // SUPERSET of the true value space and could fabricate a containment verdict).
        let mut constraints: Vec<(bool, bool, Dec)> = Vec::new();
        for &f in members {
            let pairs = match facet_triples.get(&f) {
                Some(p) if !p.is_empty() => p,
                _ => continue 'ranges,
            };
            for &(p, o) in pairs {
                let Some((lower, excl)) = facets.kind(p) else {
                    continue 'ranges;
                };
                let Some(v) = facet_value(dict, o) else {
                    continue 'ranges;
                };
                constraints.push((lower, excl, v));
            }
        }
        let Some(key) = finalize(sort, ilo, ihi, &constraints) else {
            continue 'ranges;
        };
        let c = mint.get(names, key);
        out.node_range.insert(*node, c);
    }
    for &(node, lit) in points {
        if let Some(key) = point_value(dict, lit) {
            let c = mint.get(names, key);
            out.node_range.insert(node, c);
        }
    }
    for &(node, lit) in exists_points {
        if let Some(key) = point_value(dict, lit) {
            let c = mint.get(names, key);
            out.node_exists.insert(node, c);
        }
    }
    // [SONNET-4.6] sq-vkq9u: the ABox `DataPropertyAssertion` points, minted LAST — so whenever
    // `abox_points` is empty every TBox-driven concept id (and therefore the whole
    // `Classifier::classify` surface) is exactly what it was before this parameter existed. A
    // literal whose canonical value equals an already-minted key SHARES that concept via the
    // `Mint` dedup, which is what makes `{5}` from `a q 5` and the TBox's `[5, 5]` /
    // `DataHasValue 5` / `DataOneOf 5` the SAME concept.
    let outers = mint.minted.len();
    for &lit in abox_points {
        if let Some(key) = point_value(dict, lit) {
            let c = mint.get(names, key);
            out.lit_point.insert(lit, c);
        }
    }
    out.axioms = emit(&mint.minted, outers);
    out
}

/// CR7's decision core — SATISFIABILITY (value-space non-emptiness) + the canonical form
/// of a faceted range. `ilo`/`ihi` are the base datatype's implicit integer bounds (only
/// for [`Sort::Integer`]); each constraint is `(is_lower, is_exclusive, value)`. Returns
/// the canonical [`NormRange`] (`NormRange::Empty` = PROVABLY UNSATISFIABLE), or `None`
/// when the exact tier cannot decide (i128 tighten/align overflow) — the caller defers,
/// producing NO verdict.
pub(crate) fn finalize(
    sort: Sort,
    ilo: Option<i128>,
    ihi: Option<i128>,
    constraints: &[(bool, bool, Dec)],
) -> Option<NormRange> {
    match sort {
        Sort::Integer => {
            let (mut lo, mut hi) = (ilo, ihi);
            for &(lower, excl, v) in constraints {
                if lower {
                    let b = int_lower(v, excl)?;
                    lo = Some(lo.map_or(b, |c| c.max(b)));
                } else {
                    let b = int_upper(v, excl)?;
                    hi = Some(hi.map_or(b, |c| c.min(b)));
                }
            }
            Some(match (lo, hi) {
                (Some(l), Some(h)) if l > h => NormRange::Empty,
                _ => NormRange::Int { lo, hi },
            })
        }
        Sort::Decimal => {
            let (mut lo, mut hi): (DecBound, DecBound) = (None, None);
            for &(lower, excl, v) in constraints {
                if lower {
                    lo = Some(match lo {
                        None => (v, excl),
                        Some(cur) => tighter(cur, (v, excl), true)?,
                    });
                } else {
                    hi = Some(match hi {
                        None => (v, excl),
                        Some(cur) => tighter(cur, (v, excl), false)?,
                    });
                }
            }
            finalize_dec(lo, hi)
        }
    }
}

/// The smallest integer satisfying the lower bound `v` (`>` if `excl`, else `>=`), or
/// `None` on i128 overflow (defer). Exact via the substrate's `Dec::round_to_int`.
fn int_lower(v: Dec, excl: bool) -> Option<i128> {
    if excl {
        v.round_to_int(RoundMode::Floor).mant.checked_add(1)
    } else {
        Some(v.round_to_int(RoundMode::Ceil).mant)
    }
}

/// The largest integer satisfying the upper bound `v` (`<` if `excl`, else `<=`), or
/// `None` on i128 overflow (defer).
fn int_upper(v: Dec, excl: bool) -> Option<i128> {
    if excl {
        v.round_to_int(RoundMode::Ceil).mant.checked_sub(1)
    } else {
        Some(v.round_to_int(RoundMode::Floor).mant)
    }
}

/// The TIGHTER of two same-side decimal bounds (`lower` picks the larger value, upper the
/// smaller; at an equal value EXCLUSIVE is tighter). `None` = the exact comparison is
/// undecidable (`Dec::cmp` scale-alignment overflow) → the caller defers the whole range.
fn tighter(cur: (Dec, bool), new: (Dec, bool), lower: bool) -> Option<(Dec, bool)> {
    Some(match cur.0.cmp(new.0)? {
        Ordering::Equal => (cur.0, cur.1 || new.1),
        Ordering::Less => {
            if lower {
                new
            } else {
                cur
            }
        }
        Ordering::Greater => {
            if lower {
                cur
            } else {
                new
            }
        }
    })
}

/// Canonicalizes decimal-sorted bounds: decides emptiness (dense order — `lo > hi`, or
/// `lo == hi` with either side exclusive), collapses an INTEGRAL inclusive point to the
/// `Int` form (`{5.0}` = `{5}` in the XSD value space), and scale-minimizes the bound
/// mantissas. `None` = undecidable comparison → defer.
fn finalize_dec(lo: DecBound, hi: DecBound) -> Option<NormRange> {
    if let (Some((l, le)), Some((h, he))) = (&lo, &hi) {
        match l.cmp(*h)? {
            Ordering::Greater => return Some(NormRange::Empty),
            Ordering::Equal => {
                if *le || *he {
                    return Some(NormRange::Empty);
                }
                let (m, s) = dec_norm(*l);
                return Some(if s == 0 {
                    NormRange::Int { lo: Some(m), hi: Some(m) }
                } else {
                    NormRange::Dec {
                        lo: Some((m, s, false)),
                        hi: Some((m, s, false)),
                    }
                });
            }
            Ordering::Less => {}
        }
    }
    let key = |b: Option<(Dec, bool)>| {
        b.map(|(v, e)| {
            let (m, s) = dec_norm(v);
            (m, s, e)
        })
    };
    Some(NormRange::Dec { lo: key(lo), hi: key(hi) })
}

/// Scale-minimal `(mantissa, scale)` of a `Dec` (strips trailing fraction zeros; zero is
/// `(0, 0)`) — the canonical form `NormRange::Dec` keys on.
fn dec_norm(d: Dec) -> (i128, u32) {
    let (mut m, mut s) = (d.mant, d.scale);
    if m == 0 {
        return (0, 0);
    }
    while s > 0 && m % 10 == 0 {
        m /= 10;
        s -= 1;
    }
    (m, s)
}

/// Whether `outer`'s value space PROVABLY contains `inner`'s (the CR8 implication test).
/// `false` means "no verdict" — possibly a genuine non-containment, possibly the
/// documented sound incompleteness (a decimal-sorted range inside an integer-sorted one,
/// or an undecidable exact comparison) — and simply derives nothing.
pub(crate) fn contains(outer: &NormRange, inner: &NormRange) -> bool {
    let dec = |m: i128, s: u32| Dec { mant: m, scale: s };
    match (inner, outer) {
        (NormRange::Empty, _) => true,
        (_, NormRange::Empty) => false,
        (NormRange::Int { lo: l1, hi: h1 }, NormRange::Int { lo: l2, hi: h2 }) => {
            let lo_ok = match l2 {
                None => true,
                Some(l2) => matches!(l1, Some(l1) if l1 >= l2),
            };
            let hi_ok = match h2 {
                None => true,
                Some(h2) => matches!(h1, Some(h1) if h1 <= h2),
            };
            lo_ok && hi_ok
        }
        // A non-empty integer interval inside decimal bounds: integers ARE decimal values,
        // and it suffices that the extreme integers satisfy the outer bounds.
        (NormRange::Int { lo: l1, hi: h1 }, NormRange::Dec { lo: l2, hi: h2 }) => {
            let lo_ok = match l2 {
                None => true,
                Some((m, s, excl)) => match l1 {
                    None => false,
                    Some(l1) => match dec(*l1, 0).cmp(dec(*m, *s)) {
                        Some(Ordering::Greater) => true,
                        Some(Ordering::Equal) => !excl,
                        _ => false,
                    },
                },
            };
            let hi_ok = match h2 {
                None => true,
                Some((m, s, excl)) => match h1 {
                    None => false,
                    Some(h1) => match dec(*h1, 0).cmp(dec(*m, *s)) {
                        Some(Ordering::Less) => true,
                        Some(Ordering::Equal) => !excl,
                        _ => false,
                    },
                },
            };
            lo_ok && hi_ok
        }
        (NormRange::Dec { lo: l1, hi: h1 }, NormRange::Dec { lo: l2, hi: h2 }) => {
            // Dense order: interval containment is necessary AND sufficient here.
            let lo_ok = match (l1, l2) {
                (_, None) => true,
                (None, Some(_)) => false,
                (Some((m1, s1, e1)), Some((m2, s2, e2))) => {
                    match dec(*m1, *s1).cmp(dec(*m2, *s2)) {
                        Some(Ordering::Greater) => true,
                        Some(Ordering::Equal) => *e1 || !*e2,
                        _ => false,
                    }
                }
            };
            let hi_ok = match (h1, h2) {
                (_, None) => true,
                (None, Some(_)) => false,
                (Some((m1, s1, e1)), Some((m2, s2, e2))) => {
                    match dec(*m1, *s1).cmp(dec(*m2, *s2)) {
                        Some(Ordering::Less) => true,
                        Some(Ordering::Equal) => *e1 || !*e2,
                        _ => false,
                    }
                }
            };
            lo_ok && hi_ok
        }
        // Documented incompleteness: a decimal-sorted range may contain non-integers, and
        // the integral-point case was already collapsed to `Int` by normalization.
        (NormRange::Dec { .. }, NormRange::Int { .. }) => false,
    }
}

/// CR7/CR8 as ordinary `Normal` axioms over the minted range concepts: `d ⊑ ⊥` for the
/// EMPTY range (CR5 then propagates the clash to any class with an `∃p.d` obligation) and
/// `d1 ⊑ d2` for every PROVEN containment (CR1/CR3/CR4 then thread them through
/// data-property existentials exactly like class fillers).
///
/// `outers` is the length of the `minted` PREFIX whose ranges may act as the CONTAINING side;
/// entries at or beyond it are [SONNET-4.6] sq-vkq9u's ABox `DataPropertyAssertion` POINTS, and
/// restricting the inner loop to the prefix keeps the cost O(k·outers) instead of O(k²) — an
/// ABox may assert hundreds of thousands of distinct values, where a full square would dominate
/// the whole classification. The restriction is output-EQUIVALENT, not an approximation: a point
/// `{v}` can only contain a NON-EMPTY range that is `{v}` itself (an inner `[l, h] ⊆ {v}` forces
/// `l = h = v`, and `finalize`/`point_value` canonicalize that to the SAME `NormRange`, which
/// `Mint` dedups to the SAME concept — so the `i != j` guard already excluded it), and an EMPTY
/// inner range never reaches the inner loop at all (it is `⊑ ⊥` and `continue`s above).
/// `emit(minted, minted.len())` is therefore the unrestricted form, and the unit test
/// `abox_points_never_act_as_containing_ranges` asserts the two agree on a mixed set.
fn emit(minted: &[(Concept, NormRange)], outers: usize) -> Vec<Normal> {
    let mut out = Vec::new();
    for (i, (ci, ki)) in minted.iter().enumerate() {
        if *ki == NormRange::Empty {
            out.push(Normal::Sub(*ci, BOTTOM));
            continue;
        }
        for (j, (cj, kj)) in minted[..outers].iter().enumerate() {
            if i != j && contains(kj, ki) {
                out.push(Normal::Sub(*ci, *cj));
            }
        }
    }
    out
}

/// Dedup registry: one fresh concept per canonical [`NormRange`], in deterministic mint
/// order. Minted concepts carry NO dict id, so they can never surface in the lattice.
#[derive(Default)]
struct Mint {
    by_key: FxHashMap<NormRange, Concept>,
    minted: Vec<(Concept, NormRange)>,
}

impl Mint {
    fn get(&mut self, names: &mut Names, key: NormRange) -> Concept {
        if let Some(&c) = self.by_key.get(&key) {
            return c;
        }
        let c = names.fresh();
        self.by_key.insert(key.clone(), c);
        self.minted.push((c, key));
        c
    }
}

/// The dict ids of the four supported facet predicates (absent terms get `NO_ID`, which
/// no real predicate carries, so they simply never match).
struct FacetVocab {
    min_inc: Id,
    max_inc: Id,
    min_exc: Id,
    max_exc: Id,
}

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

impl FacetVocab {
    fn intern(dict: &Dict) -> FacetVocab {
        let look = |local: &str| {
            dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                format!("{}{}", XSD, local),
            )))
        };
        FacetVocab {
            min_inc: look("minInclusive"),
            max_inc: look("maxInclusive"),
            min_exc: look("minExclusive"),
            max_exc: look("maxExclusive"),
        }
    }

    /// `(is_lower, is_exclusive)` of a facet predicate, `None` for ANY other predicate —
    /// which defers the whole range (never "ignore the facet").
    fn kind(&self, p: Id) -> Option<(bool, bool)> {
        if p == self.min_inc {
            Some((true, false))
        } else if p == self.min_exc {
            Some((true, true))
        } else if p == self.max_inc {
            Some((false, false))
        } else if p == self.max_exc {
            Some((false, true))
        } else {
            None
        }
    }
}

/// The supported base datatypes: `(sort, implicit lo, implicit hi)`. The derived integer
/// types fold their XSD-defined bounds into the interval so their restrictions are decided
/// over the TRUE value space (`xsd:byte` + `minInclusive 1000` is genuinely empty).
/// `None` = unsupported base → defer.
pub(crate) fn base_of(iri: &str) -> Option<(Sort, Option<i128>, Option<i128>)> {
    let local = iri.strip_prefix(XSD)?;
    Some(match local {
        "decimal" => (Sort::Decimal, None, None),
        "integer" => (Sort::Integer, None, None),
        "long" => (Sort::Integer, Some(i128::from(i64::MIN)), Some(i128::from(i64::MAX))),
        "int" => (Sort::Integer, Some(i128::from(i32::MIN)), Some(i128::from(i32::MAX))),
        "short" => (Sort::Integer, Some(-32768), Some(32767)),
        "byte" => (Sort::Integer, Some(-128), Some(127)),
        "nonNegativeInteger" => (Sort::Integer, Some(0), None),
        "positiveInteger" => (Sort::Integer, Some(1), None),
        "nonPositiveInteger" => (Sort::Integer, None, Some(0)),
        "negativeInteger" => (Sort::Integer, None, Some(-1)),
        "unsignedLong" => (Sort::Integer, Some(0), Some(i128::from(u64::MAX))),
        "unsignedInt" => (Sort::Integer, Some(0), Some(i128::from(u32::MAX))),
        "unsignedShort" => (Sort::Integer, Some(0), Some(65535)),
        "unsignedByte" => (Sort::Integer, Some(0), Some(255)),
        _ => return None,
    })
}

/// The full IRI of a dict id, if it denotes an IRI (`None` for inline/literal/blank/triple
/// ids — `term_parts` must not be asked about inline ids).
fn datatype_iri(dict: &Dict, id: Id) -> Option<String> {
    if is_inline(id) {
        return None;
    }
    match dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => Some(format!("{}{}", prefix, suffix)),
        _ => None,
    }
}

/// The EXACT-tier numeric value of the literal behind `id`, or `None` (defer): handles the
/// dict's inline-integer ids directly; heap literals route through the substrate's
/// `as_numeric` (the engine's exact lexical path) and are additionally validated against
/// their OWN datatype's implicit bounds (`"300"^^xsd:byte` is ill-formed — no verdict may
/// be built on it). Float/double values are deferred (representation-boundary honesty).
fn facet_value(dict: &Dict, id: Id) -> Option<Dec> {
    if is_inline(id) {
        // An inline id IS a canonical non-negative xsd:integer value.
        return Some(Dec { mant: i128::from(id - INLINE_BASE), scale: 0 });
    }
    let TermParts::Lit { value, datatype, lang: None } = dict.term_parts(id) else {
        return None;
    };
    let lit = oxrdf::Literal::new_typed_literal(value, oxrdf::NamedNode::new_unchecked(datatype));
    let d = match as_numeric(&lit)? {
        Num::Int(i) => Dec { mant: i128::from(i), scale: 0 },
        Num::Dec(d) => d,
        Num::Float(_) | Num::Double(_) => return None,
    };
    if sparq_core::is_integer_datatype(datatype) {
        // `as_numeric` already guarantees an integral value here; verify the DERIVED
        // types' implicit bounds too.
        let (_, lo, hi) = base_of(datatype)?;
        if lo.is_some_and(|l| d.mant < l) || hi.is_some_and(|h| d.mant > h) {
            return None;
        }
    }
    Some(d)
}

/// The POINT range `{v}` of a literal id (`DataHasValue` / singleton `DataOneOf`), or
/// `None` (defer). An integral value canonicalizes to the `Int` form so `{5}`, `{5.0}`
/// and the faceted `[5, 5]` all share one concept.
fn point_value(dict: &Dict, id: Id) -> Option<NormRange> {
    let d = facet_value(dict, id)?;
    let (m, s) = dec_norm(d);
    Some(if s == 0 {
        NormRange::Int { lo: Some(m), hi: Some(m) }
    } else {
        NormRange::Dec {
            lo: Some((m, s, false)),
            hi: Some((m, s, false)),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    fn d(mant: i128, scale: u32) -> Dec {
        Dec { mant, scale }
    }

    // [FABLE-5] sq-pbz04.2.2 — one DIRECT unit test per fn, each asserting an EXACT
    // verdict so a mutated comparison / rounding / branch goes red.

    #[test]
    fn base_of_covers_the_ladder_and_defers_the_rest() {
        assert_eq!(base_of("http://www.w3.org/2001/XMLSchema#integer"), Some((Sort::Integer, None, None)));
        assert_eq!(base_of("http://www.w3.org/2001/XMLSchema#decimal"), Some((Sort::Decimal, None, None)));
        assert_eq!(
            base_of("http://www.w3.org/2001/XMLSchema#byte"),
            Some((Sort::Integer, Some(-128), Some(127)))
        );
        assert_eq!(
            base_of("http://www.w3.org/2001/XMLSchema#unsignedLong"),
            Some((Sort::Integer, Some(0), Some(i128::from(u64::MAX))))
        );
        assert_eq!(
            base_of("http://www.w3.org/2001/XMLSchema#negativeInteger"),
            Some((Sort::Integer, None, Some(-1)))
        );
        // Deferred bases: float/double/string (and anything non-XSD).
        assert_eq!(base_of("http://www.w3.org/2001/XMLSchema#double"), None);
        assert_eq!(base_of("http://www.w3.org/2001/XMLSchema#float"), None);
        assert_eq!(base_of("http://www.w3.org/2001/XMLSchema#string"), None);
        assert_eq!(base_of("http://ex/notxsd"), None);
    }

    #[test]
    fn int_lower_upper_tighten_exactly() {
        // Inclusive bounds on non-integers round INWARD; exclusive integer bounds step.
        assert_eq!(int_lower(d(55, 1), false), Some(6)); // >= 5.5 → 6
        assert_eq!(int_lower(d(55, 1), true), Some(6)); // >  5.5 → 6
        assert_eq!(int_lower(d(5, 0), false), Some(5)); // >= 5   → 5
        assert_eq!(int_lower(d(5, 0), true), Some(6)); // >  5   → 6
        assert_eq!(int_upper(d(55, 1), false), Some(5)); // <= 5.5 → 5
        assert_eq!(int_upper(d(55, 1), true), Some(5)); // <  5.5 → 5
        assert_eq!(int_upper(d(5, 0), false), Some(5)); // <= 5   → 5
        assert_eq!(int_upper(d(5, 0), true), Some(4)); // <  5   → 4
        // Negative values: > -5.5 → -5, < -5.5 → -6.
        assert_eq!(int_lower(d(-55, 1), true), Some(-5));
        assert_eq!(int_upper(d(-55, 1), true), Some(-6));
        // i128 overflow at the extremes → None (defer, no verdict).
        assert_eq!(int_lower(d(i128::MAX, 0), true), None);
        assert_eq!(int_upper(d(i128::MIN, 0), true), None);
    }

    #[test]
    fn tighter_picks_the_stricter_bound() {
        // Lower side: the LARGER value wins; ties promote exclusivity.
        assert_eq!(tighter((d(3, 0), false), (d(5, 0), true), true), Some((d(5, 0), true)));
        assert_eq!(tighter((d(5, 0), true), (d(3, 0), false), true), Some((d(5, 0), true)));
        assert_eq!(tighter((d(5, 0), false), (d(50, 1), true), true), Some((d(5, 0), true)));
        // Upper side: the SMALLER value wins.
        assert_eq!(tighter((d(3, 0), false), (d(5, 0), true), false), Some((d(3, 0), false)));
    }

    #[test]
    fn finalize_integer_detects_unsat_and_sat() {
        // minInclusive 18 / maxInclusive 10 → EMPTY (the bead's acceptance range).
        assert_eq!(
            finalize(Sort::Integer, None, None, &[(true, false, d(18, 0)), (false, false, d(10, 0))]),
            Some(NormRange::Empty)
        );
        // minInclusive 10 / maxInclusive 18 → the non-empty [10, 18].
        assert_eq!(
            finalize(Sort::Integer, None, None, &[(true, false, d(10, 0)), (false, false, d(18, 0))]),
            Some(NormRange::Int { lo: Some(10), hi: Some(18) })
        );
        // DISCRETE tightening: (5, 6) holds NO integer → EMPTY; (5, 7) holds {6}.
        assert_eq!(
            finalize(Sort::Integer, None, None, &[(true, true, d(5, 0)), (false, true, d(6, 0))]),
            Some(NormRange::Empty)
        );
        assert_eq!(
            finalize(Sort::Integer, None, None, &[(true, true, d(5, 0)), (false, true, d(7, 0))]),
            Some(NormRange::Int { lo: Some(6), hi: Some(6) })
        );
        // Implicit derived-type bounds participate: byte + minInclusive 1000 → EMPTY.
        assert_eq!(
            finalize(Sort::Integer, Some(-128), Some(127), &[(true, false, d(1000, 0))]),
            Some(NormRange::Empty)
        );
        // Repeated same-side facets conjoin (the tightest wins).
        assert_eq!(
            finalize(Sort::Integer, None, None, &[(true, false, d(3, 0)), (true, false, d(7, 0))]),
            Some(NormRange::Int { lo: Some(7), hi: None })
        );
    }

    #[test]
    fn finalize_decimal_dense_order_differs_from_integer() {
        // (5.0, 6.0) over xsd:decimal is NON-empty (5.5 lies inside) — the dense-order
        // contrast with the integer case above.
        assert_eq!(
            finalize(Sort::Decimal, None, None, &[(true, true, d(50, 1)), (false, true, d(60, 1))]),
            Some(NormRange::Dec { lo: Some((5, 0, true)), hi: Some((6, 0, true)) })
        );
        // [5.0, 5.0] is the integral point → canonical Int{5,5}; making a side exclusive
        // empties it.
        assert_eq!(
            finalize(Sort::Decimal, None, None, &[(true, false, d(50, 1)), (false, false, d(5, 0))]),
            Some(NormRange::Int { lo: Some(5), hi: Some(5) })
        );
        assert_eq!(
            finalize(Sort::Decimal, None, None, &[(true, true, d(50, 1)), (false, false, d(5, 0))]),
            Some(NormRange::Empty)
        );
        // A NON-integral point stays decimal-sorted.
        assert_eq!(
            finalize(Sort::Decimal, None, None, &[(true, false, d(25, 1)), (false, false, d(25, 1))]),
            Some(NormRange::Dec { lo: Some((25, 1, false)), hi: Some((25, 1, false)) })
        );
        // lo > hi → EMPTY.
        assert_eq!(
            finalize(Sort::Decimal, None, None, &[(true, false, d(6, 0)), (false, false, d(5, 0))]),
            Some(NormRange::Empty)
        );
    }

    #[test]
    fn finalize_dec_defers_on_unalignable_scales() {
        // i128 scale alignment of (i128::MAX, scale 0) vs (1, scale 1) overflows →
        // finalize_dec must return None (defer), NEVER a guessed verdict.
        assert_eq!(
            finalize(
                Sort::Decimal,
                None,
                None,
                &[(true, false, d(i128::MAX, 0)), (false, false, d(1, 1))]
            ),
            None
        );
    }

    #[test]
    fn dec_norm_strips_trailing_zeros_only() {
        assert_eq!(dec_norm(d(1500, 2)), (15, 0)); // 15.00 → 15
        assert_eq!(dec_norm(d(150, 1)), (15, 0)); // 15.0 → 15
        assert_eq!(dec_norm(d(15, 1)), (15, 1)); // 1.5 stays
        assert_eq!(dec_norm(d(0, 5)), (0, 0));
        assert_eq!(dec_norm(d(-50, 1)), (-5, 0));
    }

    #[test]
    fn contains_int_int_exact_interval_logic() {
        let r = |lo, hi| NormRange::Int { lo, hi };
        assert!(contains(&r(Some(0), Some(20)), &r(Some(5), Some(10))));
        assert!(!contains(&r(Some(5), Some(10)), &r(Some(0), Some(20)))); // flipped → red
        assert!(contains(&r(None, None), &r(Some(5), Some(10))));
        assert!(contains(&r(None, Some(10)), &r(Some(5), Some(10)))); // UPPER boundary equal
        assert!(contains(&r(Some(5), Some(20)), &r(Some(5), Some(10)))); // LOWER boundary equal
        assert!(!contains(&r(Some(6), Some(10)), &r(Some(5), Some(10))));
        assert!(!contains(&r(Some(0), Some(9)), &r(Some(5), Some(10)))); // upper violates
        assert!(!contains(&r(Some(0), Some(20)), &r(None, Some(10)))); // unbounded inner
    }

    #[test]
    fn contains_int_in_dec_and_never_dec_in_int() {
        let int = NormRange::Int { lo: Some(5), hi: Some(10) };
        let dec_wide = NormRange::Dec { lo: Some((0, 0, false)), hi: Some((100, 0, false)) };
        assert!(contains(&dec_wide, &int)); // integers are decimal values
        let dec_excl5 = NormRange::Dec { lo: Some((5, 0, true)), hi: Some((100, 0, false)) };
        assert!(!contains(&dec_excl5, &int)); // 5 is excluded by the outer bound
        // Boundary-EQUAL inclusive endpoints on BOTH sides admit the integer interval;
        // an exclusive UPPER endpoint at 10 rejects it.
        let dec_tight = NormRange::Dec { lo: Some((5, 0, false)), hi: Some((10, 0, false)) };
        assert!(contains(&dec_tight, &int));
        let dec_excl10 = NormRange::Dec { lo: Some((5, 0, false)), hi: Some((10, 0, true)) };
        assert!(!contains(&dec_excl10, &int));
        // Documented sound incompleteness: decimal-sorted inner never derives ⊆ Int.
        assert!(!contains(&int, &dec_wide));
        assert!(!contains(&NormRange::Int { lo: None, hi: None }, &dec_wide));
    }

    #[test]
    fn contains_dec_dec_exclusivity_boundaries() {
        let rd = |lo, hi| NormRange::Dec { lo, hi };
        // (1, 2) ⊆ [1, 2] but NOT the reverse.
        let open = rd(Some((1, 0, true)), Some((2, 0, true)));
        let closed = rd(Some((1, 0, false)), Some((2, 0, false)));
        assert!(contains(&closed, &open));
        assert!(!contains(&open, &closed));
        // Unbounded outer side accepts anything on that side.
        assert!(contains(&rd(None, Some((2, 0, false))), &open));
        assert!(!contains(&rd(Some((15, 1, false)), None), &open)); // 1.5 > lower 1
    }

    #[test]
    fn contains_empty_edges() {
        let some = NormRange::Int { lo: Some(0), hi: Some(1) };
        assert!(contains(&some, &NormRange::Empty)); // ∅ ⊆ anything
        assert!(!contains(&NormRange::Empty, &some)); // nothing non-empty ⊆ ∅
    }

    #[test]
    fn emit_bottoms_empties_and_orders_containments() {
        let e = NormRange::Empty;
        let narrow = NormRange::Int { lo: Some(5), hi: Some(10) };
        let wide = NormRange::Int { lo: Some(0), hi: Some(20) };
        let minted = vec![(7u32, e), (8u32, narrow), (9u32, wide)];
        let ax = emit(&minted, minted.len());
        assert!(ax.contains(&Normal::Sub(7, BOTTOM)), "empty range ⊑ ⊥ (CR7)");
        assert!(ax.contains(&Normal::Sub(8, 9)), "narrow ⊑ wide (CR8)");
        assert!(!ax.contains(&Normal::Sub(9, 8)), "wide ⊑ narrow must NOT be emitted");
        assert!(!ax.contains(&Normal::Sub(8, BOTTOM)), "a satisfiable range is not ⊥");
        assert_eq!(ax.len(), 2);
    }

    #[test]
    fn abox_points_never_act_as_containing_ranges() {
        // [SONNET-4.6] sq-vkq9u: the equivalence `emit`'s `outers` cutoff relies on — restricting
        // the CONTAINING side to the TBox prefix must not lose a single axiom. Mixed set: an empty
        // range, a wide range, and two ABox points (one INSIDE the wide range, one outside, plus a
        // non-integral one). The restricted and unrestricted forms must agree exactly.
        let minted = vec![
            (7u32, NormRange::Empty),
            (8u32, NormRange::Int { lo: Some(0), hi: Some(20) }),
            // the ABox-point suffix:
            (9u32, NormRange::Int { lo: Some(5), hi: Some(5) }),
            (10u32, NormRange::Int { lo: Some(99), hi: Some(99) }),
            (11u32, NormRange::Dec { lo: Some((55, 1, false)), hi: Some((55, 1, false)) }),
        ];
        let restricted = emit(&minted, 2);
        let full = emit(&minted, minted.len());
        assert_eq!(restricted, full, "the `outers` cutoff must be output-equivalent");
        assert!(restricted.contains(&Normal::Sub(9, 8)), "{{5}} ⊑ [0, 20] (CR8)");
        assert!(!restricted.contains(&Normal::Sub(10, 8)), "99 ∉ [0, 20]");
        // A point is never a proper OUTER: no axiom names 9/10/11 on the right.
        assert!(
            !restricted.iter().any(|a| matches!(a, Normal::Sub(_, sup) if *sup >= 9)),
            "a point can only contain itself, which `Mint` dedups away"
        );
    }

    #[test]
    fn facet_value_reads_inline_heap_and_validates_datatypes() {
        // Small integers inline in the dict; big/negative/decimal ones are heap literals.
        let ttl = r#"
            @prefix : <http://ex/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            :f :small 18 .
            :f :neg -7 .
            :f :dec 2.50 .
            :f :bad "300"^^xsd:byte .
            :f :dbl "5.5"^^xsd:double .
            :f :str "18" .
            :f :iri :notALiteral .
        "#;
        let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
        let obj = |pred: &str| {
            let p = dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                format!("http://ex/{}", pred),
            )));
            triples.iter().find(|t| t[1] == p).map(|t| t[2]).expect("triple")
        };
        assert_eq!(facet_value(&dict, obj("small")), Some(d(18, 0)));
        assert_eq!(facet_value(&dict, obj("neg")), Some(d(-7, 0)));
        assert_eq!(facet_value(&dict, obj("dec")), Some(d(250, 2))); // written scale kept
        // Ill-formed for its own datatype / float tier / non-numeric / non-literal → defer.
        assert_eq!(facet_value(&dict, obj("bad")), None);
        assert_eq!(facet_value(&dict, obj("dbl")), None);
        assert_eq!(facet_value(&dict, obj("str")), None);
        assert_eq!(facet_value(&dict, obj("iri")), None);
    }

    #[test]
    fn point_value_collapses_integral_decimals() {
        let ttl = r#"
            @prefix : <http://ex/> .
            :f :int 5 .
            :f :intdec 5.0 .
            :f :frac 5.5 .
        "#;
        let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
        let obj = |pred: &str| {
            let p = dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                format!("http://ex/{}", pred),
            )));
            triples.iter().find(|t| t[1] == p).map(|t| t[2]).expect("triple")
        };
        let five = NormRange::Int { lo: Some(5), hi: Some(5) };
        assert_eq!(point_value(&dict, obj("int")), Some(five.clone()));
        assert_eq!(point_value(&dict, obj("intdec")), Some(five)); // {5.0} = {5}
        assert_eq!(
            point_value(&dict, obj("frac")),
            Some(NormRange::Dec { lo: Some((55, 1, false)), hi: Some((55, 1, false)) })
        );
    }

    #[test]
    fn datatype_iri_only_for_iris() {
        let ttl = r#"@prefix : <http://ex/> . :a :p :b . :a :q "lit" ."#;
        let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
        assert_eq!(datatype_iri(&dict, triples[0][2]).as_deref(), Some("http://ex/b"));
        let lit = triples.iter().find(|t| t[2] != triples[0][2]).unwrap()[2];
        assert_eq!(datatype_iri(&dict, lit), None);
    }

    #[test]
    fn resolve_dedups_equal_ranges_and_bottoms_the_empty_one() {
        // Two structurally different but VALUE-EQUAL ranges ([5,10] via integer facets and
        // via decimal-lexical facets) must mint ONE concept; the empty range gets ⊑ ⊥.
        let ttl = r#"
            @prefix : <http://ex/> .
            @prefix owl: <http://www.w3.org/2002/07/owl#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            :n1 owl:onDatatype xsd:integer ;
                owl:withRestrictions ( [ xsd:minInclusive 5 ] [ xsd:maxInclusive 10 ] ) .
            :n2 owl:onDatatype xsd:integer ;
                owl:withRestrictions ( [ xsd:minInclusive 5.0 ] [ xsd:maxInclusive 10.0 ] ) .
            :n3 owl:onDatatype xsd:integer ;
                owl:withRestrictions ( [ xsd:minInclusive 18 ] [ xsd:maxInclusive 10 ] ) .
            :n4 owl:onDatatype xsd:string ;
                owl:withRestrictions ( [ xsd:minInclusive 5 ] ) .
        "#;
        let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
        let node = |frag: &str| {
            dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                format!("http://ex/{}", frag),
            )))
        };
        let owl = |l: &str| {
            dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                format!("http://www.w3.org/2002/07/owl#{}", l),
            )))
        };
        let (p_dt, p_wr) = (owl("onDatatype"), owl("withRestrictions"));
        let (rdf_first, rdf_rest, rdf_nil) = (
            dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#first".to_string(),
            ))),
            dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest".to_string(),
            ))),
            dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil".to_string(),
            ))),
        );
        let mut first = FxHashMap::default();
        let mut rest = FxHashMap::default();
        let mut dt_of = FxHashMap::default();
        let mut wr_of = FxHashMap::default();
        for &[s, p, o] in &triples {
            if p == rdf_first {
                first.insert(s, o);
            } else if p == rdf_rest {
                rest.insert(s, o);
            } else if p == p_dt {
                dt_of.insert(s, o);
            } else if p == p_wr {
                wr_of.insert(s, o);
            }
        }
        let walk = |head: Id| {
            let mut out = Vec::new();
            let mut cur = head;
            while cur != rdf_nil {
                let Some(&m) = first.get(&cur) else { break };
                out.push(m);
                let Some(&n) = rest.get(&cur) else { break };
                cur = n;
            }
            out
        };
        let mut ranges: Vec<(Id, Id, Id)> = ["n1", "n2", "n3", "n4"]
            .iter()
            .map(|f| {
                let n = node(f);
                (n, dt_of[&n], wr_of[&n])
            })
            .collect();
        ranges.sort_unstable();
        let mut names = Names::new();
        let out = resolve(&dict, &triples, &mut names, &ranges, &[], &[], &[], walk);
        // n1 and n2 dedup to ONE concept; n3 is empty; n4 (string base) is deferred.
        assert_eq!(out.node_range.len(), 3, "n4 must be ABSENT (deferred)");
        assert_eq!(out.node_range[&node("n1")], out.node_range[&node("n2")]);
        let empty_c = out.node_range[&node("n3")];
        assert_ne!(out.node_range[&node("n1")], empty_c);
        assert!(out.axioms.contains(&Normal::Sub(empty_c, BOTTOM)), "CR7: empty range ⊑ ⊥");
        assert!(
            !out.axioms.contains(&Normal::Sub(out.node_range[&node("n1")], BOTTOM)),
            "a satisfiable range must NOT be ⊑ ⊥ (a flipped verdict here is unsound)"
        );
    }

    #[test]
    fn resolve_mints_abox_points_and_defers_out_of_tier_literals() {
        // [SONNET-4.6] sq-vkq9u: the `abox_points` half of `resolve` — the literals of
        // `DataPropertyAssertion`s. Value-EQUAL literals (`5` and `5.0`) must share ONE concept;
        // anything outside the exact numeric tier (a string, an xsd:double, a literal ill-formed
        // for its own datatype) must be ABSENT so the caller keeps its fail-closed skip. With no
        // TBox range in play there is nothing to contain a point, so NO axiom is emitted.
        let ttl = r#"
            @prefix : <http://ex/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            :a :int 5 .
            :a :dec 5.0 .
            :a :other 6 .
            :a :str "five" .
            :a :dbl "5.0"^^xsd:double .
            :a :bad "300"^^xsd:byte .
        "#;
        let (dict, triples) = Graph::parse_to_triples(ttl, "turtle").expect("parse");
        let obj = |pred: &str| {
            let p = dict.lookup(&oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                format!("http://ex/{}", pred),
            )));
            triples.iter().find(|t| t[1] == p).map(|t| t[2]).expect("triple")
        };
        let lits: Vec<Id> =
            ["int", "dec", "other", "str", "dbl", "bad"].iter().map(|&p| obj(p)).collect();
        let mut names = Names::new();
        let out = resolve(&dict, &triples, &mut names, &[], &[], &[], &lits, |_| Vec::new());
        assert_eq!(out.lit_point.len(), 3, "only 5, 5.0 and 6 are in the exact tier");
        assert_eq!(
            out.lit_point[&obj("int")],
            out.lit_point[&obj("dec")],
            "{{5}} and {{5.0}} are the SAME value space, so ONE concept"
        );
        assert_ne!(out.lit_point[&obj("int")], out.lit_point[&obj("other")]);
        for deferred in ["str", "dbl", "bad"] {
            assert!(
                !out.lit_point.contains_key(&obj(deferred)),
                "{deferred} is outside the exact tier — no point may be minted for it"
            );
        }
        assert!(out.axioms.is_empty(), "distinct points relate to nothing (no TBox range here)");
        assert!(out.node_range.is_empty() && out.node_exists.is_empty());
    }
}
